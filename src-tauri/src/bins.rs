//! Locates and, when needed, downloads the two external tools Takit relies on:
//! `yt-dlp` (the downloader) and `ffmpeg` (audio extraction / stream merging).
//!
//! Resolution order for each tool: an explicit user override, then a copy Takit
//! manages in its app-data folder, then anything already on the system `PATH`.
//! Keeping the heavyweight binaries out of the installer is what lets the app
//! ship as a tiny download, and it means yt-dlp stays current.

use crate::settings::Settings;
use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

/// Snapshot of which tools are available, reported to the UI.
#[derive(Clone, Debug, Serialize)]
pub struct BinStatus {
    pub ytdlp_ready: bool,
    pub ffmpeg_ready: bool,
    pub ytdlp_version: String,
    pub ytdlp_source: String,
    pub ffmpeg_source: String,
}

/// Progress event streamed to the frontend during first-run setup.
#[derive(Clone, Serialize)]
struct SetupProgress {
    component: String,
    phase: String,
    message: String,
    downloaded: u64,
    total: u64,
}

#[derive(Clone, Copy)]
enum ArchiveKind {
    Zip,
    TarXz,
}

struct FfSource {
    url: &'static str,
    kind: ArchiveKind,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Folder where Takit keeps the tools it downloads itself.
pub fn bin_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("bin");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Platform-correct executable filename (adds `.exe` on Windows).
pub fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perm = meta.permissions();
        perm.set_mode(0o755);
        let _ = std::fs::set_permissions(path, perm);
    }
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// Look for an executable on the `PATH`.
fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if cfg!(windows) {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Resolve the yt-dlp binary and where it came from.
pub fn resolve_ytdlp(app: &AppHandle, settings: &Settings) -> Option<(PathBuf, &'static str)> {
    let custom = settings.ytdlp_path.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some((p, "custom"));
        }
    }
    if let Ok(dir) = bin_dir(app) {
        let p = dir.join(exe_name("yt-dlp"));
        if p.is_file() {
            return Some((p, "managed"));
        }
    }
    which("yt-dlp").map(|p| (p, "system"))
}

/// Resolve the ffmpeg binary and where it came from.
pub fn resolve_ffmpeg(app: &AppHandle, settings: &Settings) -> Option<(PathBuf, &'static str)> {
    let custom = settings.ffmpeg_path.trim();
    if !custom.is_empty() {
        let raw = PathBuf::from(custom);
        if raw.is_file() {
            return Some((raw, "custom"));
        }
        if raw.is_dir() {
            let p = raw.join(exe_name("ffmpeg"));
            if p.is_file() {
                return Some((p, "custom"));
            }
        }
    }
    if let Ok(dir) = bin_dir(app) {
        let p = dir.join(exe_name("ffmpeg"));
        if p.is_file() {
            return Some((p, "managed"));
        }
    }
    which("ffmpeg").map(|p| (p, "system"))
}

/// Directory that contains ffmpeg, suitable for yt-dlp's `--ffmpeg-location`.
pub fn ffmpeg_dir(app: &AppHandle, settings: &Settings) -> Option<PathBuf> {
    resolve_ffmpeg(app, settings).and_then(|(p, _)| p.parent().map(Path::to_path_buf))
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Run a tool quickly and capture stdout, without flashing a console on Windows.
fn run_capture(path: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(path);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// Report which tools are present and ready to use.
pub fn status(app: &AppHandle, settings: &Settings) -> BinStatus {
    let (ytdlp_version, ytdlp_source) = match resolve_ytdlp(app, settings) {
        Some((p, src)) => (run_capture(&p, &["--version"]).unwrap_or_default(), src),
        None => (String::new(), "none"),
    };
    let (ffmpeg_ready, ffmpeg_source) = match resolve_ffmpeg(app, settings) {
        Some((p, src)) => (run_capture(&p, &["-version"]).is_some(), src),
        None => (false, "none"),
    };
    BinStatus {
        ytdlp_ready: !ytdlp_version.is_empty(),
        ffmpeg_ready,
        ytdlp_version,
        ytdlp_source: ytdlp_source.to_string(),
        ffmpeg_source: if ffmpeg_ready {
            ffmpeg_source.to_string()
        } else {
            "none".to_string()
        },
    }
}

// ---------------------------------------------------------------------------
// Download sources (selected at compile time for this OS/arch)
// ---------------------------------------------------------------------------

fn ytdlp_url() -> &'static str {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_arm64.exe"
        } else {
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
        }
    } else if cfg!(target_os = "macos") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64"
    } else {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
    }
}

fn ffmpeg_sources() -> Vec<FfSource> {
    if cfg!(target_os = "windows") {
        vec![FfSource {
            url: "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
            kind: ArchiveKind::Zip,
        }]
    } else if cfg!(target_os = "macos") {
        vec![
            FfSource {
                url: "https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip",
                kind: ArchiveKind::Zip,
            },
            FfSource {
                url: "https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip",
                kind: ArchiveKind::Zip,
            },
        ]
    } else if cfg!(target_arch = "aarch64") {
        vec![FfSource {
            url: "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz",
            kind: ArchiveKind::TarXz,
        }]
    } else {
        vec![FfSource {
            url: "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz",
            kind: ArchiveKind::TarXz,
        }]
    }
}

// ---------------------------------------------------------------------------
// Install / update
// ---------------------------------------------------------------------------

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("Takit/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

fn emit_setup(
    app: &AppHandle,
    component: &str,
    phase: &str,
    message: &str,
    downloaded: u64,
    total: u64,
) {
    let _ = app.emit(
        "setup://progress",
        SetupProgress {
            component: component.to_string(),
            phase: phase.to_string(),
            message: message.to_string(),
            downloaded,
            total,
        },
    );
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    app: &AppHandle,
    component: &str,
) -> Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        if last.elapsed() >= Duration::from_millis(150) {
            emit_setup(
                app,
                component,
                "download",
                "Downloading…",
                downloaded,
                total,
            );
            last = Instant::now();
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    emit_setup(
        app,
        component,
        "download",
        "Downloaded",
        downloaded,
        total.max(downloaded),
    );
    Ok(())
}

/// Ensure both tools are present, downloading whatever is missing.
/// When `force` is true, managed copies are (re)downloaded even if found.
pub async fn ensure(app: AppHandle, settings: Settings, force: bool) -> Result<BinStatus, String> {
    let dir = bin_dir(&app)?;
    let client = build_client()?;

    if force || resolve_ytdlp(&app, &settings).is_none() {
        emit_setup(&app, "yt-dlp", "download", "Downloading yt-dlp…", 0, 0);
        let dest = dir.join(exe_name("yt-dlp"));
        let tmp = dir.join("yt-dlp.part");
        download_file(&client, ytdlp_url(), &tmp, &app, "yt-dlp").await?;
        make_executable(&tmp);
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        make_executable(&dest);
    }

    if force || resolve_ffmpeg(&app, &settings).is_none() {
        for source in ffmpeg_sources() {
            emit_setup(&app, "ffmpeg", "download", "Downloading ffmpeg…", 0, 0);
            let archive = dir.join("ffmpeg-archive.part");
            download_file(&client, source.url, &archive, &app, "ffmpeg").await?;
            emit_setup(&app, "ffmpeg", "extract", "Extracting ffmpeg…", 0, 0);
            let dest = dir.clone();
            let kind = source.kind;
            let archive_path = archive.clone();
            tauri::async_runtime::spawn_blocking(move || {
                extract_archive(&archive_path, kind, &dest)
            })
            .await
            .map_err(|e| e.to_string())??;
            let _ = std::fs::remove_file(&archive);
        }
    }

    emit_setup(&app, "all", "done", "Ready", 0, 0);
    Ok(status(&app, &settings))
}

/// Update yt-dlp: self-update a managed copy, or download a fresh managed copy.
pub async fn update_ytdlp(app: AppHandle) -> Result<String, String> {
    let dir = bin_dir(&app)?;
    let managed = dir.join(exe_name("yt-dlp"));
    if managed.is_file() {
        emit_setup(&app, "yt-dlp", "download", "Updating yt-dlp…", 0, 0);
        let p = managed.clone();
        tauri::async_runtime::spawn_blocking(move || run_capture(&p, &["-U"]))
            .await
            .map_err(|e| e.to_string())?;
    } else {
        emit_setup(&app, "yt-dlp", "download", "Downloading yt-dlp…", 0, 0);
        let client = build_client()?;
        let tmp = dir.join("yt-dlp.part");
        download_file(&client, ytdlp_url(), &tmp, &app, "yt-dlp").await?;
        make_executable(&tmp);
        std::fs::rename(&tmp, &managed).map_err(|e| e.to_string())?;
        make_executable(&managed);
    }
    emit_setup(&app, "all", "done", "Ready", 0, 0);
    Ok(run_capture(&managed, &["--version"]).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Archive extraction (runs on a blocking thread)
// ---------------------------------------------------------------------------

/// If an archive entry is `ffmpeg`/`ffprobe` (any nesting, optional `.exe`),
/// return the bare filename it should be written out as.
fn wanted_binary(entry_name: &str) -> Option<String> {
    let base = entry_name.rsplit(['/', '\\']).next().unwrap_or(entry_name);
    if base.is_empty() {
        return None;
    }
    let lower = base.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    if stem == "ffmpeg" || stem == "ffprobe" {
        Some(base.to_string())
    } else {
        None
    }
}

fn extract_archive(path: &Path, kind: ArchiveKind, dest: &Path) -> Result<(), String> {
    match kind {
        ArchiveKind::Zip => extract_zip(path, dest),
        ArchiveKind::TarXz => extract_tar_xz(path, dest),
    }
}

fn extract_zip(path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut found = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        if let Some(out_name) = wanted_binary(&name) {
            let out_path = dest.join(&out_name);
            let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            drop(out);
            make_executable(&out_path);
            found = true;
        }
    }
    if found {
        Ok(())
    } else {
        Err("no ffmpeg binary found in archive".to_string())
    }
}

fn extract_tar_xz(path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut reader = std::io::BufReader::new(file);
    let mut decompressed: Vec<u8> = Vec::new();
    lzma_rs::xz_decompress(&mut reader, &mut decompressed)
        .map_err(|e| format!("xz error: {e:?}"))?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(decompressed));
    let mut found = false;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry.path().map_err(|e| e.to_string())?.to_path_buf();
        let name = entry_path.to_string_lossy().to_string();
        if let Some(out_name) = wanted_binary(&name) {
            let out_path = dest.join(&out_name);
            let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            drop(out);
            make_executable(&out_path);
            found = true;
        }
    }
    if found {
        Ok(())
    } else {
        Err("no ffmpeg binary found in archive".to_string())
    }
}
