//! Menu-bar / system-tray icon.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::core::Status;

const ICON: &[u8] = include_bytes!("../icons/tray@2x.png");
const ICON_LIVE: &[u8] = include_bytes!("../icons/tray-recording@2x.png");

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Silas", true, None::<&str>)?;
    let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Check for Updates…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Silas", true, Some("CmdOrCtrl+Q"))?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &sep,
            &open,
            &update,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON)?)
        .icon_as_template(true)
        .tooltip("Silas")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => crate::show_main(app),
            "update" => {
                crate::show_main(app);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::updates::check(&app, true).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                if !cfg!(target_os = "macos") {
                    crate::show_main(tray.app_handle());
                }
            }
        })
        .build(app)?;
    app.manage(StatusItem(status));
    Ok(())
}

struct StatusItem(MenuItem<tauri::Wry>);

pub fn refresh(app: &AppHandle, s: Status) {
    let label = match s {
        Status::Loading => "Loading model…".to_string(),
        Status::NeedsModel => "Choose a speech model".to_string(),
        Status::Ready => {
            let key = app
                .try_state::<crate::core::Shared>()
                .map(|c| crate::store::hotkey_label(&c.settings.lock().hotkey))
                .unwrap_or_default();
            format!("Ready · hold {key} to speak")
        }
        Status::Recording => "Listening…".to_string(),
        Status::Transcribing | Status::Typing => "Writing…".to_string(),
        Status::Error => "Needs attention".to_string(),
    };
    if let Some(item) = app.try_state::<StatusItem>() {
        let _ = item.0.set_text(label);
    }
    if let Some(tray) = app.tray_by_id("main") {
        let live = matches!(s, Status::Recording);
        if let Ok(img) = Image::from_bytes(if live { ICON_LIVE } else { ICON }) {
            let _ = tray.set_icon(Some(img));
            let _ = tray.set_icon_as_template(true);
        }
    }
}
