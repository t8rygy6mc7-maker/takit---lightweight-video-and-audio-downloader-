//! Takit — a lightweight, cross-platform video & audio downloader.
//!
//! The Rust side owns the external tools (yt-dlp/ffmpeg), the download queue,
//! settings persistence, and the tray. The webview frontend is a thin UI that
//! talks to the commands defined here over Tauri IPC.

mod bins;
mod download;
mod settings;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use download::{DownloadRequest, JobsMap};
use settings::Settings;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Semaphore;

/// Shared application state, accessible from every command.
struct AppState {
    settings: Mutex<Settings>,
    sem: Arc<Semaphore>,
    jobs: JobsMap,
    /// Whether a system tray was successfully created (false on minimal Linux).
    has_tray: bool,
}

// ---------------------------------------------------------------------------
// Settings commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<AppState>,
    mut settings: Settings,
) -> Result<(), String> {
    settings.normalize(&app);
    settings::save(&app, &settings)?;
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

// ---------------------------------------------------------------------------
// File-system helpers
// ---------------------------------------------------------------------------

#[tauri::command]
fn pick_download_folder(app: AppHandle) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    rx.recv()
        .ok()
        .flatten()
        .and_then(|fp| fp.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    reveal_in_dir(&app, &path)
}

/// Reveal a file in the OS file manager (used by "open when done" and the UI).
pub(crate) fn reveal_in_dir(app: &AppHandle, path: &str) -> Result<(), String> {
    if Path::new(path).exists() {
        app.opener()
            .reveal_item_in_dir(path)
            .map_err(|e| e.to_string())
    } else {
        // Fall back to opening the containing folder.
        let parent = Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        app.opener()
            .open_path(parent, None::<&str>)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn open_downloads_folder(app: AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    let dir = state.settings.lock().unwrap().download_dir.clone();
    std::fs::create_dir_all(&dir).ok();
    app.opener()
        .open_path(dir, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn read_clipboard(app: AppHandle) -> Option<String> {
    app.clipboard().read_text().ok()
}

// ---------------------------------------------------------------------------
// Tool management
// ---------------------------------------------------------------------------

#[tauri::command]
fn check_binaries(app: AppHandle, state: tauri::State<AppState>) -> bins::BinStatus {
    let settings = state.settings.lock().unwrap().clone();
    bins::status(&app, &settings)
}

#[tauri::command]
async fn ensure_binaries(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    force: bool,
) -> Result<bins::BinStatus, String> {
    let settings = state.settings.lock().unwrap().clone();
    bins::ensure(app, settings, force).await
}

#[tauri::command]
async fn update_ytdlp(app: AppHandle) -> Result<String, String> {
    bins::update_ytdlp(app).await
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

#[tauri::command]
async fn probe_url(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<download::ProbeInfo, String> {
    let settings = state.settings.lock().unwrap().clone();
    let ytdlp = bins::resolve_ytdlp(&app, &settings)
        .map(|(p, _)| p)
        .ok_or("yt-dlp is not installed yet. Open Setup first.")?;
    download::probe(ytdlp, url).await
}

#[tauri::command]
fn start_download(
    app: AppHandle,
    state: tauri::State<AppState>,
    req: DownloadRequest,
) -> Result<String, String> {
    let settings = state.settings.lock().unwrap().clone();

    let ytdlp = bins::resolve_ytdlp(&app, &settings)
        .map(|(p, _)| p)
        .ok_or("yt-dlp is not installed yet. Click \"Set up\" first.")?;
    let ffmpeg_dir = bins::ffmpeg_dir(&app, &settings)
        .ok_or("ffmpeg is not installed yet. Click \"Set up\" first.")?;

    download::start(
        app.clone(),
        state.sem.clone(),
        state.jobs.clone(),
        ytdlp,
        ffmpeg_dir,
        settings,
        req,
    )
}

#[tauri::command]
fn cancel_download(state: tauri::State<AppState>, id: String) -> Result<(), String> {
    download::cancel(&state.jobs, &id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------

fn focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItemBuilder::with_id("show", "Show Takit").build(app)?;
    let folder = MenuItemBuilder::with_id("folder", "Open downloads folder").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Takit").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&show)
        .item(&folder)
        .separator()
        .item(&quit)
        .build()?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))
        .ok()
        .or_else(|| app.default_window_icon().cloned())
        .expect("a tray icon");

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Takit")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => focus_main(app),
            "folder" => {
                if let Some(state) = app.try_state::<AppState>() {
                    let dir = state.settings.lock().unwrap().download_dir.clone();
                    std::fs::create_dir_all(&dir).ok();
                    let _ = app.opener().open_path(dir, None::<&str>);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                focus_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// App entrypoint
// ---------------------------------------------------------------------------

pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_main(app);
        }));
    }

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = settings::load(&handle);
            let permits = settings.concurrent.clamp(1, 8);
            // The tray is best-effort: on minimal Linux setups the indicator
            // library may be missing. If it fails we keep running without it
            // (and won't hide-to-tray, so the window stays reachable).
            let has_tray = match build_tray(app) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Takit: system tray unavailable: {e}");
                    false
                }
            };
            app.manage(AppState {
                settings: Mutex::new(settings),
                sem: Arc::new(Semaphore::new(permits)),
                jobs: Arc::new(Mutex::new(HashMap::new())),
                has_tray,
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let hide_to_tray = window
                    .try_state::<AppState>()
                    .map(|s| s.has_tray && s.settings.lock().unwrap().close_to_tray)
                    .unwrap_or(false);
                if hide_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            pick_download_folder,
            open_path,
            reveal_path,
            open_downloads_folder,
            read_clipboard,
            check_binaries,
            ensure_binaries,
            update_ytdlp,
            probe_url,
            start_download,
            cancel_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Takit");
}
