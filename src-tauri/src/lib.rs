pub mod commands;
pub mod host;
pub mod theme;
pub mod tray;
pub mod updates;

// The engine lives in core/voice (polycarp-voice). Re-exported so `crate::core::Shared`,
// `silas_lib::models` and the other old paths keep working.
pub use polycarp_voice::{audio, benchmark, core, dictionary, focus, keys, models, permissions, store, typer};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,ort=warn,transcribe_rs=warn"),
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
        .setup(|app| {
            let handle = app.handle().clone();
            let core = Core::new(std::sync::Arc::new(host::TauriHost::new(handle.clone())));
            app.manage(core.clone());

            // First run: honor the launch-at-login default.
            if !store::exists("settings.json") {
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
            theme::watch(handle.clone());

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
            commands::get_theme,
            commands::wait_device_pick,
            commands::check_update,
            commands::install_update,
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
