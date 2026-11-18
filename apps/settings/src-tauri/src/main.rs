//! Docky settings: a separate process that exists only while its window is
//! open. It edits the configuration file and tells the dock to reload it.
//!
//! With `--update` it opens no window: the dock starts it that way once a
//! day, while the user is away, to install a newer version if there is one.
//! It exits as soon as it knows there is none.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env, io, path::PathBuf, process::Command, sync::Mutex, time::Duration};

use docky_config::Config;
use docky_shell::{autostart, dock_link, launcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindowBuilder};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The dock owns the pinned and recent apps; this app never overwrites them
/// with a stale copy.
fn merged_with_disk(config: Config) -> Config {
    match docky_config::load(&docky_config::default_path()) {
        Ok(Some(disk)) => Config {
            pinned: disk.pinned,
            recent: disk.recent,
            taskbar_pins: disk.taskbar_pins,
            ..config
        },
        _ => config,
    }
}

/// The dock is installed next to this app.
fn dock_exe() -> io::Result<PathBuf> {
    Ok(env::current_exe()?.with_file_name("docky.exe"))
}

#[tauri::command]
fn get_config() -> Config {
    docky_config::load(&docky_config::default_path())
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The defaults live in `docky-config`; the window asks for them to reset.
#[tauri::command]
fn default_config() -> Config {
    Config::default()
}

#[tauri::command]
fn set_config(config: Config) -> Result<Config, String> {
    let config = merged_with_disk(config.sanitized());
    docky_config::save(&docky_config::default_path(), &config)
        .map_err(|error| error.to_string())?;
    dock_link::notify_config_changed();
    Ok(config)
}

#[tauri::command]
fn get_autostart() -> bool {
    autostart::is_enabled()
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    let dock = dock_exe().map_err(|error| error.to_string())?;
    let target = enabled.then(|| dock.to_string_lossy().into_owned());
    autostart::set(target.as_deref()).map_err(|error| error.to_string())
}

/// The update found by the last check, kept until the user installs it.
#[derive(Default)]
struct PendingUpdate(Mutex<Option<Update>>);

/// Asks GitHub Releases for a newer version; returns it, if any. Only runs
/// while Settings is open: the dock itself never touches the network.
#[tauri::command]
async fn check_update(
    app: tauri::AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<Option<UpdateInfo>, String> {
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    let info = update.as_ref().map(|update| UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone().filter(|notes| is_release_page(notes)),
    });
    *pending.0.lock().map_err(|error| error.to_string())? = update;
    Ok(info)
}

/// What the window shows about an available update.
#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    /// The release page, when the manifest points at one.
    notes: Option<String>,
}

/// Download progress, sent to the window as `update-progress`.
#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

const RELEASES: &str = "https://github.com/latiefahmad/Docky/releases/";

/// The update manifest is not signed (only the installer is), so the only
/// link it may open is a release page of this repository.
fn is_release_page(url: &str) -> bool {
    url.starts_with(RELEASES) && !url.contains(char::is_whitespace)
}

#[tauri::command]
fn open_release_notes(url: String) -> Result<(), String> {
    if !is_release_page(&url) {
        return Err("enlace no permitido".into());
    }
    launcher::open(&url).map_err(|error| error.to_string())
}

/// Downloads the signed installer and runs it; the installer stops the dock,
/// replaces both apps and starts the dock again. This process exits.
#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<(), String> {
    // Cloned, not taken: if the download fails the user can try again.
    let update = pending
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or("there is no pending update")?;
    let mut downloaded = 0u64;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                let _ = app.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|error| error.to_string())
}

/// The argument the dock starts this app with for a background update.
const UPDATE_ARG: &str = "--update";

/// Checks for a newer version and installs it without asking, as the user
/// chose in Settings; the installer stops and restarts the dock. Exits when
/// there is nothing to do or anything fails: the next try is tomorrow.
async fn update_in_background(app: AppHandle) {
    let config = get_config();
    let allowed = config.check_updates && config.auto_update;
    let found = match app.updater_builder().timeout(Duration::from_secs(60)) {
        // The installer starts the dock again; this app need not come back.
        builder if allowed => match builder.restart_after_install(false).build() {
            Ok(updater) => updater.check().await.ok().flatten(),
            Err(_) => None,
        },
        _ => None,
    };
    // Opening Settings meanwhile lands in this process (single instance):
    // then it stays as the user's Settings, which shows the update itself,
    // and installing must not close it under them.
    let window_opened = || app.get_webview_window("main").is_some();
    if let Some(update) = found.filter(|_| !window_opened())
        && let Err(error) = update.download_and_install(|_, _| {}, || {}).await
    {
        eprintln!("docky-settings: update: {error}");
    }
    if !window_opened() {
        app.exit(0);
    }
}

/// Opens the Settings window, or brings it forward if it is open.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let Some(config) = app.config().app.windows.first() else {
        return;
    };
    if let Err(error) =
        WebviewWindowBuilder::from_config(app, config).and_then(|window| window.build())
    {
        eprintln!("docky-settings: {error}");
    }
}

fn main() {
    docky_config::migrate_legacy_folder();
    let background = env::args().any(|arg| arg == UPDATE_ARG);
    // The Start menu entry opens this app: it also brings back a dock the user quit.
    if !background
        && !dock_link::is_running()
        && let Err(error) = dock_exe().and_then(|dock| Command::new(dock).spawn())
    {
        eprintln!("docky-settings: could not open the dock: {error}");
    }
    let result = tauri::Builder::default()
        // First, so a second launch exits before doing anything else.
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            // The open window checks for updates itself; a background update
            // started meanwhile has nothing to do.
            if !args.iter().any(|arg| arg == UPDATE_ARG) {
                show_window(app);
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(PendingUpdate::default())
        .setup(move |app| {
            if background {
                tauri::async_runtime::spawn(update_in_background(app.handle().clone()));
            } else {
                show_window(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            default_config,
            set_config,
            get_autostart,
            set_autostart,
            check_update,
            install_update,
            open_release_notes
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("docky-settings: {error}");
    }
}
