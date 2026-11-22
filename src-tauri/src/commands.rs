//! Tauri commands: the UI's whole API. See docs/ipc.md.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::core::{Scope, Shared, Status};
use crate::dictionary::{Dictionary, LearnResult};
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
    /// The retype key's label, or None when no retype key is set.
    retype_hotkey_label: Option<String>,
    /// The app the last dictation went to and its style (`style.rs`).
    app: Option<crate::style::AppInfo>,
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
    let retype = core.settings.lock().retype_hotkey.clone();
    AppState {
        status,
        detail,
        platform: platform(),
        version: app.package_info().version.to_string(),
        model: core.active_model(),
        hotkey_label: store::hotkey_label(&hotkey),
        hotkey,
        retype_hotkey_label: retype.filter(|k| !k.trim().is_empty()).map(|k| store::hotkey_label(&k)),
        app: core.last_app(),
        permissions: permissions::check(),
    }
}

#[tauri::command]
pub fn get_settings(core: State<Shared>) -> Settings {
    core.settings.lock().clone()
}

#[tauri::command]
pub fn set_settings(core: State<Shared>, app: AppHandle, settings: Settings) -> R<Settings> {
    // The model, its source and the hotkey are handled by the core (see `Core::apply_settings`).
    let (settings, old) = core.inner().apply_settings(settings)?;
    if old.launch_at_login != settings.launch_at_login {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        let _ = if settings.launch_at_login {
            al.enable()
        } else {
            al.disable()
        };
    }
    Ok(settings)
}

#[tauri::command]
pub async fn list_models(core: State<'_, Shared>) -> R<Vec<ModelEntry>> {
    let active = core.settings.lock().model.clone();
    // Reads the benchmark and registry files, and the first call detects the device.
    tauri::async_runtime::spawn_blocking(move || models::list(Some(&active)))
        .await
        .map_err(err)
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
    let out = {
        let mut d = core.dict.lock();
        f(&mut d);
        store::save("dictionary.json", &*d);
        d.clone()
    };
    core.dictionary_changed();
    out
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
pub fn add_snippet(core: State<Shared>, trigger: String, text: String) -> R<Dictionary> {
    let mut added = false;
    let d = with_dict(&core, |d| added = d.add_snippet(&trigger, &text));
    if added {
        Ok(d)
    } else {
        Err("A snippet needs a phrase to say and the text to type.".into())
    }
}

#[tauri::command]
pub fn remove_snippet(core: State<Shared>, trigger: String) -> Dictionary {
    with_dict(&core, |d| d.remove_snippet(&trigger))
}

#[tauri::command]
pub fn import_dictionary(core: State<Shared>, json: String) -> R<Dictionary> {
    let out = {
        let mut d = core.dict.lock();
        d.import(&json)
            .map_err(|e| format!("That file isn't a dictionary Silas can read: {e}"))?;
        store::save("dictionary.json", &*d);
        d.clone()
    };
    core.dictionary_changed();
    Ok(out)
}

#[tauri::command]
pub fn export_dictionary(core: State<Shared>) -> R<String> {
    serde_json::to_string_pretty(&*core.dict.lock()).map_err(err)
}

#[tauri::command]
pub fn save_dictionary(core: State<Shared>) -> R<String> {
    let dir = dirs::download_dir().unwrap_or_else(store::data_dir);
    let path = dir.join(format!(
        "silas-dictionary-{}.json",
        chrono::Local::now().format("%Y-%m-%d")
    ));
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

/// Types the last dictation again. Async: it returns at once and the typing runs on the core's thread.
#[tauri::command]
pub async fn type_again(core: State<'_, Shared>) -> R<()> {
    core.inner().type_again(crate::core::Via::Command)
}

#[tauri::command]
pub async fn capture_hotkey(core: State<'_, Shared>) -> R<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    *core.capture.lock() = Some(tx);
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let got = rx.recv_timeout(Duration::from_secs(10));
        core.capture.lock().take();
        // Escape gives `cancelled`; a key that types text gives a sentence saying why.
        got.map_err(|_| "No key pressed".to_string())?
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

/// The home banner's Fix Permission (macOS). Clears Silas's stale Accessibility entry, then
/// reopens Silas so a new process asks once (`permissions::after_fix_relaunch`).
#[tauri::command]
pub fn fix_accessibility(app: AppHandle) {
    if !cfg!(target_os = "macos") || !permissions::fix_accessibility() {
        return;
    }
    let bundle = std::env::current_exe()
        .ok()
        .and_then(|exe| updates::bundle_of(&exe));
    let Some(bundle) = bundle else {
        // A dev build has no bundle to reopen; ask from this process instead.
        log::info!(target: "silas.permissions", "fix: no app bundle to reopen; asking from this process");
        permissions::after_fix_relaunch();
        return;
    };
    // Single-instance would hand a second copy back to this one, so the relaunch waits for
    // this process to exit, then opens the same bundle through Launch Services.
    let spawned = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec /usr/bin/open "$2" --args "$3""#,
            "sh",
        ])
        .arg(std::process::id().to_string())
        .arg(&bundle)
        .arg(permissions::FIX_RELAUNCH_ARG)
        .spawn();
    match spawned {
        Ok(_) => {
            log::info!(target: "silas.permissions", "fix: reopening {}", bundle.display());
            app.exit(0);
        }
        Err(e) => {
            log::warn!(target: "silas.permissions", "fix: could not reopen Silas ({e}); asking from this process");
            permissions::after_fix_relaunch();
        }
    }
}

/// Async, so it runs off the main thread: on Linux, listing ALSA devices opens each one and the
/// window would freeze until it finished.
#[tauri::command]
pub async fn list_microphones() -> R<Vec<String>> {
    tauri::async_runtime::spawn_blocking(audio::microphones)
        .await
        .map_err(err)
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    crate::show_main(&app);
}

#[tauri::command]
pub async fn get_benchmark() -> R<Option<benchmark::Report>> {
    tauri::async_runtime::spawn_blocking(benchmark::load_report)
        .await
        .map_err(err)
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
    tauri::async_runtime::spawn_blocking(move || {
        core.run_benchmark(if all { Scope::All } else { Scope::Active }, guard)
            .map(|_| ())
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn share_benchmark() -> R<()> {
    tauri::async_runtime::spawn_blocking(|| {
        let report = benchmark::load_report().ok_or("Run the benchmark first".to_string())?;
        benchmark::open_url(&benchmark::report_url(&report)).map_err(err)
    })
    .await
    .map_err(err)?
}

/// Resolves once this run has asked the device registry (at most 6 s), for first-run setup.
#[tauri::command]
pub async fn wait_device_pick() -> R<()> {
    tauri::async_runtime::spawn_blocking(|| {
        benchmark::wait_for_registry(std::time::Duration::from_secs(6))
    })
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn get_device() -> R<benchmark::Device> {
    tauri::async_runtime::spawn_blocking(|| benchmark::device().clone())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> R<Option<updates::UpdateInfo>> {
    updates::check(&app, false).await.map_err(err)
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> R<()> {
    updates::install(&app).await.map_err(err)
}

// ---------------------------------------------------------------- sync (docs/sync.md)

use polycarp_voice::sync_engine::{PairingCode, SyncEngine, SyncStatus};

fn sync_engine(core: &Shared) -> R<std::sync::Arc<SyncEngine>> {
    core.sync.get().cloned().ok_or_else(|| "Sync isn't running.".to_string())
}

/// Runs off the main thread: these may read the keychain or restart the listener.
async fn on_sync<T: Send + 'static>(
    core: State<'_, Shared>,
    f: impl FnOnce(&SyncEngine) -> anyhow::Result<T> + Send + 'static,
) -> R<T> {
    let engine = sync_engine(core.inner())?;
    tauri::async_runtime::spawn_blocking(move || f(&engine).map_err(|e| format!("{e:#}")))
        .await
        .map_err(err)?
}

#[tauri::command]
pub async fn get_sync(core: State<'_, Shared>) -> R<SyncStatus> {
    Ok(sync_engine(core.inner())?.status())
}

#[tauri::command]
pub async fn set_sync_enabled(core: State<'_, Shared>, on: bool) -> R<SyncStatus> {
    on_sync(core, move |e| e.set_enabled(on)).await
}

/// The pairing code and URL, only when the person asks to pair a device.
#[tauri::command]
pub async fn sync_pairing_code(core: State<'_, Shared>) -> R<PairingCode> {
    on_sync(core, |e| e.pairing_code()).await
}

#[tauri::command]
pub async fn join_sync(core: State<'_, Shared>, code: String) -> R<SyncStatus> {
    on_sync(core, move |e| e.join(&code)).await
}

#[tauri::command]
pub async fn new_sync_key(core: State<'_, Shared>) -> R<SyncStatus> {
    on_sync(core, |e| e.new_key()).await
}

#[tauri::command]
pub async fn leave_sync(core: State<'_, Shared>) -> R<SyncStatus> {
    on_sync(core, |e| e.leave()).await
}

#[tauri::command]
pub async fn set_sync_name(core: State<'_, Shared>, name: String) -> R<SyncStatus> {
    on_sync(core, move |e| e.set_name(&name)).await
}

#[tauri::command]
pub async fn add_sync_peer(core: State<'_, Shared>, addr: String) -> R<SyncStatus> {
    on_sync(core, move |e| e.add_peer(&addr)).await
}

#[tauri::command]
pub async fn remove_sync_peer(core: State<'_, Shared>, addr: String) -> R<SyncStatus> {
    on_sync(core, move |e| e.remove_peer(&addr)).await
}

#[tauri::command]
pub async fn set_sync_folder(core: State<'_, Shared>, folder: Option<String>) -> R<SyncStatus> {
    on_sync(core, move |e| e.set_folder(folder.as_deref())).await
}

#[tauri::command]
pub async fn sync_now(core: State<'_, Shared>) -> R<()> {
    sync_engine(core.inner())?.sync_now();
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Tauri runs a command that isn't `async` on the main thread, where the window draws and takes
    /// input, so the window freezes for as long as one takes. These stay there: each reads memory
    /// or writes one small file, and the settings writes must keep their order. Devices, network,
    /// processes and big files go in an `async` command. This list only shrinks.
    const ON_MAIN_THREAD: &[&str] = &[
        "accept_pending",
        "add_correction",
        "add_snippet",
        "add_term",
        "clear_history",
        "correct_history",
        "delete_history",
        "delete_model",
        "dismiss_pending",
        "export_dictionary",
        "fix_accessibility",
        "get_dictionary",
        "get_history",
        "get_settings",
        "get_state",
        "get_stats",
        "import_dictionary",
        "open_permission",
        "remove_correction",
        "remove_snippet",
        "remove_term",
        "reset_accessibility",
        "save_dictionary",
        "select_model",
        "set_settings",
        "show_main",
        "toggle_recording",
    ];

    #[test]
    fn only_the_listed_commands_run_on_the_main_thread() {
        let mut sync = Vec::new();
        let mut lines = include_str!("commands.rs").lines().map(str::trim);
        while let Some(line) = lines.next() {
            if line != "#[tauri::command]" {
                continue;
            }
            let Some(sig) = lines.find(|l| !l.starts_with("#[") && !l.starts_with("//")) else {
                break;
            };
            if let Some(rest) = sig.strip_prefix("pub fn ") {
                sync.push(rest.split('(').next().unwrap_or(rest));
            }
        }
        sync.sort_unstable();
        assert_eq!(sync, ON_MAIN_THREAD, "a new command on the main thread: make it async, or list it with a reason");
    }
}
