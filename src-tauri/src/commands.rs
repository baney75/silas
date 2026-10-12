//! Tauri commands: the UI's whole API. See docs/ipc.md.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::core::{Shared, Status};
use crate::dictionary::{Dictionary, LearnResult};
use crate::keys::Trigger;
use crate::models::{self, ModelEntry};
use crate::store::{self, HistoryItem, Settings, Stats};
use crate::{audio, permissions, updates};

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
    let old = std::mem::replace(&mut *core.settings.lock(), settings.clone());
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
    if old.model != settings.model {
        core.inner().select_model(&settings.model);
    } else if old.hotkey != settings.hotkey && core.status().0 == Status::Ready {
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

#[tauri::command]
pub fn select_model(core: State<Shared>, id: String) -> R<()> {
    if !models::installed(&id) {
        return Err("Download that model first".into());
    }
    core.inner().select_model(&id);
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
pub fn list_microphones() -> Vec<String> {
    audio::microphones()
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    crate::show_main(&app);
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> R<Option<updates::UpdateInfo>> {
    updates::check(&app, false).await.map_err(err)
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> R<()> {
    updates::install(&app).await.map_err(err)
}
