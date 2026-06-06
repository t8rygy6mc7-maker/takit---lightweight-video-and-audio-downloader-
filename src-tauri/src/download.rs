//! The download engine: turns a request into a yt-dlp invocation, parses its
//! progress output, streams updates to the UI, and supports cancellation and a
//! simple concurrency limit.

use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{oneshot, Semaphore};

/// Cancel handles for in-flight jobs, keyed by job id.
pub type JobsMap = Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>;

/// What the frontend sends to start a download.
#[derive(Clone, Debug, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub mode: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub audio_format: String,
}

/// A single download's state, mirrored in the UI.
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub url: String,
    pub mode: String,
    pub format: String,
    pub title: String,
    pub status: String,
    pub percent: f64,
    pub speed: String,
    pub eta: String,
    pub size: String,
    pub filepath: String,
    pub error: String,
}

/// yt-dlp prints one of these lines per progress tick (see `--progress-template`).
const PROGRESS_TEMPLATE: &str = "download:[PROG]%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s";

fn gen_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{t}-{n}")
}

fn emit_job(app: &AppHandle, job: &Job) {
    let _ = app.emit("job://update", job.clone());
}

/// Build the yt-dlp argument list and a short human label for the UI.
fn build_args(
    req: &DownloadRequest,
    settings: &Settings,
    ffmpeg_dir: &str,
    download_dir: &str,
) -> (Vec<String>, String) {
    let mut args: Vec<String> = vec![
        "--newline".into(),
        "--no-playlist".into(),
        "--ignore-config".into(),
        "--no-mtime".into(),
        "--progress-template".into(),
        PROGRESS_TEMPLATE.into(),
        "--ffmpeg-location".into(),
        ffmpeg_dir.into(),
        "-P".into(),
        download_dir.into(),
        "-o".into(),
        "%(title).200B [%(id)s].%(ext)s".into(),
        "--windows-filenames".into(),
    ];

    if settings.embed_metadata {
        args.push("--embed-metadata".into());
    }

    let label = if req.mode == "audio" {
        let fmt = if req.audio_format.trim().is_empty() {
            settings.audio_format.clone()
        } else {
            req.audio_format.clone()
        };
        args.push("-f".into());
        args.push("ba/b".into());
        args.push("-x".into());
        args.push("--audio-format".into());
        args.push(fmt.clone());
        args.push("--audio-quality".into());
        args.push("0".into());
        if settings.embed_thumbnail && matches!(fmt.as_str(), "mp3" | "m4a" | "flac") {
            args.push("--embed-thumbnail".into());
        }
        format!("Audio • {}", fmt.to_uppercase())
    } else {
        let q = if req.quality.trim().is_empty() {
            settings.video_quality.clone()
        } else {
            req.quality.clone()
        };
        let fmt = if q == "best" {
            "bv*+ba/b".to_string()
        } else {
            format!("bv*[height<={q}]+ba/b[height<={q}]/b[height<={q}]/b")
        };
        args.push("-f".into());
        args.push(fmt);
        args.push("--merge-output-format".into());
        args.push("mp4".into());
        if q == "best" {
            "Video • best".to_string()
        } else {
            format!("Video • {q}p")
        }
    };

    args.push(req.url.clone());
    (args, label)
}

/// Validate the request, register the job, and spawn it. Returns the job id.
#[allow(clippy::too_many_arguments)]
pub fn start(
    app: AppHandle,
    sem: Arc<Semaphore>,
    jobs: JobsMap,
    ytdlp: PathBuf,
    ffmpeg_dir: PathBuf,
    settings: Settings,
    req: DownloadRequest,
) -> Result<String, String> {
    let url = req.url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Please paste a valid http(s) link.".to_string());
    }

    let download_dir = settings.download_dir.clone();
    std::fs::create_dir_all(&download_dir)
        .map_err(|e| format!("Cannot create download folder: {e}"))?;

    let req = DownloadRequest {
        url: url.clone(),
        ..req
    };
    let (args, label) = build_args(
        &req,
        &settings,
        &ffmpeg_dir.to_string_lossy(),
        &download_dir,
    );

    let id = gen_id();
    let job = Job {
        id: id.clone(),
        url,
        mode: req.mode.clone(),
        format: label,
        title: String::new(),
        status: "queued".into(),
        percent: 0.0,
        speed: String::new(),
        eta: String::new(),
        size: String::new(),
        filepath: String::new(),
        error: String::new(),
    };

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    jobs.lock().unwrap().insert(id.clone(), cancel_tx);
    emit_job(&app, &job);

    let open_when_done = settings.open_when_done;
    tauri::async_runtime::spawn(execute(
        app,
        sem,
        jobs,
        id.clone(),
        job,
        args,
        ytdlp,
        open_when_done,
        cancel_rx,
    ));

    Ok(id)
}

/// Cancel a queued or running job.
pub fn cancel(jobs: &JobsMap, id: &str) {
    if let Some(tx) = jobs.lock().unwrap().remove(id) {
        let _ = tx.send(());
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    app: AppHandle,
    sem: Arc<Semaphore>,
    jobs: JobsMap,
    id: String,
    mut job: Job,
    args: Vec<String>,
    ytdlp: PathBuf,
    open_when_done: bool,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    // Wait for a concurrency slot, but stay cancellable while queued.
    let permit = tokio::select! {
        slot = sem.acquire_owned() => slot.ok(),
        _ = &mut cancel_rx => None,
    };
    let _permit = match permit {
        Some(p) => p,
        None => {
            job.status = "canceled".into();
            emit_job(&app, &job);
            jobs.lock().unwrap().remove(&id);
            return;
        }
    };

    job.status = "downloading".into();
    emit_job(&app, &job);

    let mut cmd = tokio::process::Command::new(&ytdlp);
    cmd.args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    // tokio's Command exposes creation_flags as an inherent method on Windows.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            job.status = "error".into();
            job.error = format!("Could not start yt-dlp: {e}");
            emit_job(&app, &job);
            jobs.lock().unwrap().remove(&id);
            return;
        }
    };

    // Drain stderr in the background so we can report a useful error message.
    let err_buf = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let eb = err_buf.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut guard = eb.lock().unwrap();
                if guard.len() < 8000 {
                    guard.push_str(&line);
                    guard.push('\n');
                }
            }
        });
    }

    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();

    let mut canceled = false;
    let mut last_emit = Instant::now();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        let changed = handle_line(&mut job, &line);
                        if changed && last_emit.elapsed() >= Duration::from_millis(180) {
                            emit_job(&app, &job);
                            last_emit = Instant::now();
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            _ = &mut cancel_rx => {
                canceled = true;
                let _ = child.start_kill();
                break;
            }
        }
    }

    let exit_ok = child
        .wait()
        .await
        .ok()
        .map(|s| s.success())
        .unwrap_or(false);

    if canceled {
        job.status = "canceled".into();
        job.speed.clear();
        job.eta.clear();
    } else if exit_ok {
        job.status = "done".into();
        job.percent = 100.0;
        job.speed.clear();
        job.eta.clear();
        if job.title.is_empty() {
            job.title = file_stem(&job.filepath);
        }
        if open_when_done && !job.filepath.is_empty() {
            let _ = crate::reveal_in_dir(&app, &job.filepath);
        }
    } else {
        job.status = "error".into();
        let tail = err_buf.lock().unwrap().clone();
        job.error = last_error(&tail);
    }

    emit_job(&app, &job);
    jobs.lock().unwrap().remove(&id);
}

/// Update `job` from a single line of yt-dlp output. Returns whether the UI
/// should be refreshed.
fn handle_line(job: &mut Job, line: &str) -> bool {
    if let Some(rest) = line.strip_prefix("[PROG]") {
        let parts: Vec<&str> = rest.split('|').collect();
        let downloaded = parse_num(parts.first());
        let total = parse_num(parts.get(1));
        let estimate = parse_num(parts.get(2));
        let speed = parse_num(parts.get(3));
        let eta = parse_num(parts.get(4));

        let denom = total.or(estimate);
        if let (Some(d), Some(t)) = (downloaded, denom) {
            if t > 0.0 {
                job.percent = (d / t * 100.0).clamp(0.0, 100.0);
            }
        }
        if let Some(t) = denom {
            job.size = human_size(t);
        }
        job.speed = speed
            .map(|s| format!("{}/s", human_size(s)))
            .unwrap_or_default();
        job.eta = eta.map(human_eta).unwrap_or_default();
        if job.status != "processing" {
            job.status = "downloading".into();
        }
        return true;
    }

    // Post-processing phases (merging, audio extraction, metadata, etc.).
    if line.contains("[Merger]")
        || line.contains("[ExtractAudio]")
        || line.contains("[VideoConvertor]")
        || line.contains("[VideoRemuxer]")
        || line.contains("[Metadata]")
        || line.contains("[ThumbnailsConvertor]")
        || line.contains("[EmbedThumbnail]")
        || line.contains("[Fixup")
    {
        job.status = "processing".into();
        job.percent = 100.0;
        job.speed.clear();
        job.eta.clear();
        if let Some(idx) = line.find("Merging formats into \"") {
            let rest = &line[idx + "Merging formats into \"".len()..];
            if let Some(end) = rest.rfind('"') {
                job.filepath = rest[..end].to_string();
            }
        }
        return true;
    }

    if let Some(idx) = line.find("Destination:") {
        let path = line[idx + "Destination:".len()..].trim().to_string();
        if !path.is_empty() {
            job.filepath = path;
            if job.title.is_empty() {
                job.title = file_stem(&job.filepath);
            }
            return true;
        }
    }

    false
}

/// Probe a URL for title/thumbnail/quality info without downloading.
#[derive(Serialize)]
pub struct ProbeInfo {
    pub title: String,
    pub thumbnail: String,
    pub duration: f64,
    pub uploader: String,
    pub heights: Vec<u64>,
}

pub async fn probe(ytdlp: PathBuf, url: String) -> Result<ProbeInfo, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Please paste a valid http(s) link.".to_string());
    }
    let mut cmd = tokio::process::Command::new(&ytdlp);
    cmd.args([
        "--ignore-config",
        "--no-playlist",
        "--no-warnings",
        "--dump-single-json",
        &url,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .stdin(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);

    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let out = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| "Timed out reading the link.".to_string())?
        .map_err(|e| e.to_string())?;

    if !out.status.success() {
        let tail = String::from_utf8_lossy(&out.stderr);
        return Err(last_error(&tail));
    }

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let mut heights: Vec<u64> = v["formats"]
        .as_array()
        .map(|fs| {
            fs.iter()
                .filter_map(|f| f["height"].as_u64())
                .filter(|h| *h > 0)
                .collect()
        })
        .unwrap_or_default();
    heights.sort_unstable();
    heights.dedup();
    heights.reverse();

    Ok(ProbeInfo {
        title: v["title"].as_str().unwrap_or_default().to_string(),
        thumbnail: v["thumbnail"].as_str().unwrap_or_default().to_string(),
        duration: v["duration"].as_f64().unwrap_or(0.0),
        uploader: v["uploader"]
            .as_str()
            .or_else(|| v["channel"].as_str())
            .unwrap_or_default()
            .to_string(),
        heights,
    })
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn parse_num(field: Option<&&str>) -> Option<f64> {
    let s = field?.trim();
    if s.is_empty() || s == "NA" || s == "None" || s == "N/A" {
        return None;
    }
    s.parse::<f64>().ok()
}

fn human_size(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn human_eta(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn last_error(tail: &str) -> String {
    if let Some(line) = tail.lines().rev().find(|l| l.contains("ERROR")) {
        return line.trim().trim_start_matches("ERROR:").trim().to_string();
    }
    tail.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| "Download failed.".to_string())
}
