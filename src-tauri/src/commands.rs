//! Tauri commands: the UI's whole API. See docs/ipc.md.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::core::{Scope, Shared, Status};
use crate::dictionary::{Dictionary, LearnResult};
use crate::keys::Trigger;
use crate::models::{self, ModelEntry};
use crate::store::{self, HistoryItem, Settings, Stats};
use crate::{audio, benchmark, permissions, updates};

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    status: Status,
    detail: Option<String>,
    platform: &'static str,
    version: String,
    model: Option<String>,
    hotkey: String,
    hotkey_label: String,
    permissions: permissions::Permissions,
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

#[tauri::command]
pub fn get_state(core: State<Shared>, app: AppHandle) -> AppState {
    let (status, detail) = core.status();
    let hotkey = core.settings.lock().hotkey.clone();
    AppState {
        status,
        detail,
        platform: platform(),
        version: app.package_info().version.to_string(),
        model: core.active_model(),
        hotkey_label: store::hotkey_label(&hotkey),
        hotkey,
        permissions: permissions::check(),
    }
}

#[tauri::command]
pub fn get_settings(core: State<Shared>) -> Settings {
    core.settings.lock().clone()
}

#[tauri::command]
pub fn set_settings(core: State<Shared>, app: AppHandle, settings: Settings) -> R<Settings> {
    let mut settings = settings;
    let trigger = Trigger::parse(&settings.hotkey)
        .ok_or_else(|| format!("{} is not a key Silas can use", settings.hotkey))?;
    if !cfg!(target_os = "macos") {
        settings.update_channel = "nightly".into();
    }
    // The model and who chose it belong to the core: a benchmark can switch models while the
    // UI holds an older copy of the settings, and saving that copy must not undo the switch.
    // The UI changes models with select_model.
    let old = {
        let mut cur = core.settings.lock();
        settings.model = cur.model.clone();
        settings.model_source = cur.model_source.clone();
        std::mem::replace(&mut *cur, settings.clone())
    };
    {
        let mut m = core.machine.lock();
        m.trigger = trigger;
        m.tap_to_toggle = settings.tap_to_toggle;
    }
    if old.launch_at_login != settings.launch_at_login {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        let _ = if settings.launch_at_login {
            al.enable()
        } else {
            al.disable()
        };
    }
    core.save_settings();
    if old.hotkey != settings.hotkey && core.status().0 == Status::Ready {
        core.set_status(Status::Ready, None); // refresh the tray label
    }
    Ok(settings)
}

#[tauri::command]
pub fn list_models(core: State<Shared>) -> Vec<ModelEntry> {
    let active = core.settings.lock().model.clone();
    models::list(Some(&active))
}

#[tauri::command]
pub async fn download_model(core: State<'_, Shared>, id: String) -> R<()> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.download_model(&id))
        .await
        .map_err(err)?
        .map_err(err)
}

/// `auto` is setup choosing for the user (the device pick still applies); otherwise the user chose.
#[tauri::command]
pub fn select_model(core: State<Shared>, id: String, auto: Option<bool>) -> R<()> {
    if !models::installed(&id) {
        return Err("Download that model first".into());
    }
    core.inner().select_model(&id, auto.unwrap_or(false));
    Ok(())
}

#[tauri::command]
pub fn delete_model(core: State<Shared>, id: String) -> R<()> {
    if core.settings.lock().model == id && core.active_model().is_some() {
        return Err("Switch to another model before removing this one".into());
    }
    models::remove(&id).map_err(err)
}

#[tauri::command]
pub fn get_history(core: State<Shared>, limit: Option<usize>) -> Vec<HistoryItem> {
    let h = core.history.lock();
    h.iter().take(limit.unwrap_or(200)).cloned().collect()
}

#[tauri::command]
pub fn correct_history(core: State<Shared>, id: String, text: String) -> R<LearnResult> {
    let typed = core
        .history
        .lock()
        .iter()
        .find(|i| i.id == id)
        .map(|i| i.corrected.clone().unwrap_or_else(|| i.text.clone()))
        .ok_or("That dictation is gone")?;
    Ok(core.learn_from_edit(&typed, text.trim(), true, Some(&id)))
}

#[tauri::command]
pub fn delete_history(core: State<Shared>, id: String) {
    let mut h = core.history.lock();
    h.retain(|i| i.id != id);
    store::save("history.json", &*h);
}

#[tauri::command]
pub fn clear_history(core: State<Shared>) {
    let mut h = core.history.lock();
    h.clear();
    store::save("history.json", &*h);
}

fn with_dict(core: &State<Shared>, f: impl FnOnce(&mut Dictionary)) -> Dictionary {
    let mut d = core.dict.lock();
    f(&mut d);
    store::save("dictionary.json", &*d);
    d.clone()
}

#[tauri::command]
pub fn get_dictionary(core: State<Shared>) -> Dictionary {
    core.dict.lock().clone()
}

#[tauri::command]
pub fn add_term(core: State<Shared>, text: String) -> Dictionary {
    with_dict(&core, |d| {
        d.add_term(&text, "user");
    })
}

#[tauri::command]
pub fn remove_term(core: State<Shared>, text: String) -> Dictionary {
    with_dict(&core, |d| d.remove_term(&text))
}

#[tauri::command]
pub fn add_correction(core: State<Shared>, from: String, to: String) -> Dictionary {
    with_dict(&core, |d| {
        d.add_correction(&from, &to, "user");
    })
}

#[tauri::command]
pub fn remove_correction(core: State<Shared>, from: String) -> Dictionary {
    with_dict(&core, |d| d.remove_correction(&from))
}

#[tauri::command]
pub fn accept_pending(core: State<Shared>, from: String) -> Dictionary {
    with_dict(&core, |d| d.accept_pending(&from))
}

#[tauri::command]
pub fn dismiss_pending(core: State<Shared>, from: String) -> Dictionary {
    with_dict(&core, |d| d.dismiss_pending(&from))
}

#[tauri::command]
pub fn import_dictionary(core: State<Shared>, json: String) -> R<Dictionary> {
    let mut d = core.dict.lock();
    d.import(&json)
        .map_err(|e| format!("That file isn't a dictionary Silas can read: {e}"))?;
    store::save("dictionary.json", &*d);
    Ok(d.clone())
}

#[tauri::command]
pub fn export_dictionary(core: State<Shared>) -> R<String> {
    serde_json::to_string_pretty(&*core.dict.lock()).map_err(err)
}

#[tauri::command]
pub fn save_dictionary(core: State<Shared>) -> R<String> {
    let dir = dirs::download_dir().unwrap_or_else(store::data_dir);
    let path = dir.join(format!("silas-dictionary-{}.json", chrono::Local::now().format("%Y-%m-%d")));
    let json = serde_json::to_string_pretty(&*core.dict.lock()).map_err(err)?;
    std::fs::write(&path, json).map_err(err)?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn get_stats(core: State<Shared>) -> Stats {
    core.stats()
}

#[tauri::command]
pub fn toggle_recording(core: State<Shared>) {
    let out = core.machine.lock().toggle_from_ui();
    core.inner().act(out);
}

#[tauri::command]
pub async fn capture_hotkey(core: State<'_, Shared>) -> R<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    *core.capture.lock() = Some(tx);
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let got = rx.recv_timeout(Duration::from_secs(10));
        core.capture.lock().take();
        got.map_err(|_| "No key pressed".to_string())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn open_permission(kind: String) {
    permissions::open(&kind);
}

#[tauri::command]
pub fn reset_accessibility() {
    permissions::reset_accessibility();
}

#[tauri::command]
pub fn list_microphones() -> Vec<String> {
    audio::microphones()
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    crate::show_main(&app);
}

#[tauri::command]
pub fn get_benchmark() -> Option<benchmark::Report> {
    benchmark::load_report()
}

/// Returns when the run ends; progress and the result also arrive as events.
#[tauri::command]
pub async fn run_benchmark(core: State<'_, Shared>, all: bool) -> R<()> {
    let core = core.inner().clone();
    // Refuse here, before the thread starts, so the caller hears about it.
    let guard = core.begin_benchmark().map_err(err)?;
    if core.active_model().is_none() {
        return Err("No model is loaded yet".into());
    }
    tauri::async_runtime::spawn_blocking(move || core.run_benchmark(if all { Scope::All } else { Scope::Active }, guard).map(|_| ()))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub fn share_benchmark() -> R<()> {
    let report = benchmark::load_report().ok_or("Run the benchmark first")?;
    benchmark::open_url(&benchmark::report_url(&report)).map_err(err)
}

/// Resolves once this run has asked the device registry (at most 6 s), for first-run setup.
#[tauri::command]
pub async fn wait_device_pick() -> R<()> {
    tauri::async_runtime::spawn_blocking(|| benchmark::wait_for_registry(std::time::Duration::from_secs(6)))
        .await
        .map_err(err)
}

#[tauri::command]
pub fn get_device() -> benchmark::Device {
    benchmark::device().clone()
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> R<Option<updates::UpdateInfo>> {
    updates::check(&app, false).await.map_err(err)
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> R<()> {
    updates::install(&app).await.map_err(err)
}
