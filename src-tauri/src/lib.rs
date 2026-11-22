pub mod commands;
pub mod host;
#[cfg(test)]
mod settings_bench;
pub mod tray;
pub mod updates;

// The engine lives in core/voice (polycarp-voice). Re-exported so `crate::core::Shared`,
// `silas_lib::models` and the other old paths keep working.
pub use polycarp_voice::{
    audio, benchmark, core, dictionary, file, focus, keys, models, nowplaying, permissions, store, typer,
};

use tauri::{AppHandle, Manager};

use crate::core::Core;

pub fn show_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// On Windows the release app has no console of its own: borrow the one `silas transcribe` was
/// typed into so its output shows. Elsewhere this does nothing.
pub fn attach_console() {
    #[cfg(windows)]
    // SAFETY: a plain Win32 call with a constant; it fails harmlessly when there is no parent console.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

/// Files dropped on the main window are transcribed one after another into History.
fn on_drop(app: &AppHandle, event: &tauri::DragDropEvent) {
    use tauri::{DragDropEvent, Emitter};
    let active = matches!(event, DragDropEvent::Enter { .. } | DragDropEvent::Over { .. });
    if matches!(event, DragDropEvent::Over { .. }) {
        return;
    }
    let _ = app.emit_to("main", "file-drag", serde_json::json!({ "active": active }));
    if let DragDropEvent::Drop { paths, .. } = event {
        let core = app.state::<core::Shared>().inner().clone();
        let paths = paths.clone();
        std::thread::spawn(move || {
            for path in paths {
                // Each outcome, refusals included, reaches the UI as a `file-progress` event.
                if let Err(e) = core.transcribe_file(&path) {
                    log::info!("file not transcribed: {e:#}");
                }
            }
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,ort=warn,transcribe_rs=warn,transcribe_cpp=warn"),
    )
    .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let core = Core::new(std::sync::Arc::new(host::TauriHost::new(handle.clone())));
            app.manage(core.clone());

            // First run: honor the launch-at-login default.
            if core.first_run {
                use tauri_plugin_autostart::ManagerExt;
                if core.settings.lock().launch_at_login {
                    let _ = handle.autolaunch().enable();
                }
                core.save_settings();
            }

            tray::build(&handle)?;
            core.load_model();
            core.start_device_pick();
            polycarp_voice::start_listener(core.clone());
            updates::schedule(handle.clone());

            let background = std::env::args().any(|a| a == "--background");
            let onboarded = core.settings.lock().onboarded;
            if !background || !onboarded {
                show_main(&handle);
                // Reopened by Fix Permission: the stale entry is gone, so ask once here.
                if std::env::args().any(|a| a == permissions::FIX_RELAUNCH_ARG) {
                    permissions::after_fix_relaunch();
                }
            } else {
                #[cfg(target_os = "macos")]
                let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::DragDrop(drop) = event {
                if window.label() == "main" {
                    on_drop(window.app_handle(), drop);
                }
            }
            // Closing the window keeps Silas running in the menu bar / tray.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                    #[cfg(target_os = "macos")]
                    let _ = window
                        .app_handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::get_settings,
            commands::set_settings,
            commands::list_models,
            commands::download_model,
            commands::select_model,
            commands::delete_model,
            commands::get_history,
            commands::correct_history,
            commands::delete_history,
            commands::clear_history,
            commands::get_dictionary,
            commands::add_term,
            commands::remove_term,
            commands::add_correction,
            commands::remove_correction,
            commands::accept_pending,
            commands::dismiss_pending,
            commands::import_dictionary,
            commands::export_dictionary,
            commands::save_dictionary,
            commands::get_stats,
            commands::toggle_recording,
            commands::capture_hotkey,
            commands::open_permission,
            commands::reset_accessibility,
            commands::fix_accessibility,
            commands::list_microphones,
            commands::show_main,
            commands::get_benchmark,
            commands::run_benchmark,
            commands::share_benchmark,
            commands::get_device,
            commands::wait_device_pick,
            commands::check_update,
            commands::install_update,
            commands::transcribe_file,
            commands::cancel_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Silas")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app);
            }
            let _ = (app, event);
        });
}

#[cfg(test)]
mod engine_patch {
    /// The app links the vendored transcribe-rs with the TDT decoder fix, not the crates.io release
    /// (which has no TdtCursor): a [patch] in core/voice does not reach this workspace on its own.
    #[test]
    fn the_onnx_fallback_has_the_tdt_decoder_fix() {
        let mut cursor = transcribe_rs::onnx::parakeet::TdtCursor::default();
        // A word that spans 3 frames: token 0 scored best, duration 3 scored best (blank is 1).
        assert_eq!(cursor.step(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0], 2, 1), Some(0));
        assert_eq!(cursor.t, 3);
    }
}

