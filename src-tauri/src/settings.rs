use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// User-facing preferences, persisted as JSON in the app config directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folder downloads are saved into.
    pub download_dir: String,
    /// "video" or "audio" — which mode the UI defaults to.
    pub default_mode: String,
    /// Max video height: "best", "2160", "1440", "1080", "720", "480", "360".
    pub video_quality: String,
    /// Audio container/codec: "mp3", "m4a", "opus", "wav", "flac".
    pub audio_format: String,
    /// How many downloads may run at once (1-8). Applied on app start.
    pub concurrent: usize,
    /// Write title/artist/etc. metadata into the file.
    pub embed_metadata: bool,
    /// Embed the thumbnail as cover art (audio only).
    pub embed_thumbnail: bool,
    /// Reveal the file in the file manager when a download finishes.
    pub open_when_done: bool,
    /// Hide to the tray instead of quitting when the window is closed.
    pub close_to_tray: bool,
    /// Optional explicit path to a yt-dlp binary (overrides auto-detection).
    pub ytdlp_path: String,
    /// Optional explicit path to an ffmpeg binary or its containing folder.
    pub ffmpeg_path: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            default_mode: "video".into(),
            video_quality: "1080".into(),
            audio_format: "mp3".into(),
            concurrent: 3,
            embed_metadata: true,
            embed_thumbnail: true,
            open_when_done: false,
            close_to_tray: true,
            ytdlp_path: String::new(),
            ffmpeg_path: String::new(),
        }
    }
}

impl Settings {
    /// Clamp values that came from disk/IPC into safe ranges.
    pub fn normalize(&mut self, app: &AppHandle) {
        if self.download_dir.trim().is_empty() {
            self.download_dir = default_download_dir(app).to_string_lossy().to_string();
        }
        if self.default_mode != "audio" {
            self.default_mode = "video".into();
        }
        self.concurrent = self.concurrent.clamp(1, 8);
    }
}

fn default_download_dir(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("Takit")
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("settings.json"))
}

/// Load settings from disk, falling back to sensible defaults.
pub fn load(app: &AppHandle) -> Settings {
    let mut settings = config_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
        .unwrap_or_default();
    settings.normalize(app);
    settings
}

/// Persist settings to disk.
pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = config_path(app).ok_or("could not resolve config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}
