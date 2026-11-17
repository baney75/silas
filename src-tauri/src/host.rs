//! The Tauri side of the engine's [`Host`]: events go to the webviews, the status goes to the
//! tray and the overlay window.

use polycarp_voice::core::{overlay_status, Host, Shared, Status};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Host for TauriHost {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.app.emit(event, payload);
    }

    fn level(&self, level: f32) {
        let _ = self.app.emit_to("overlay", "level", level);
    }

    fn status_changed(&self, status: Status, show_overlay: bool) {
        crate::tray::refresh(&self.app, status);
        self.overlay(status, show_overlay);
    }

    fn show_main(&self) {
        crate::show_main(&self.app);
    }

    fn listener_returned(&self) {
        // The user just flipped the switch in System Settings; bring them back to Silas.
        if let Some(w) = self.app.get_webview_window("main") {
            if w.is_visible().unwrap_or(false) {
                let _ = w.set_focus();
            }
        }
    }

    fn is_nightly(&self) -> bool {
        self.app.package_info().version.pre.as_str().starts_with("nightly")
    }
}

impl TauriHost {
    fn overlay(&self, s: Status, enabled: bool) {
        let Some(w) = self.app.get_webview_window("overlay") else {
            return;
        };
        let show = enabled && overlay_status(s);
        if show {
            if let Ok(Some(monitor)) = w.primary_monitor().or_else(|_| w.current_monitor()) {
                let size = monitor.size();
                let pos = monitor.position();
                let scale = monitor.scale_factor();
                let (ww, wh) = (260.0 * scale, 64.0 * scale);
                let x = pos.x as f64 + (size.width as f64 - ww) / 2.0;
                let y = pos.y as f64 + size.height as f64 - wh - 72.0 * scale;
                let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
            }
        }
        place_overlay(&self.app, w, enabled);
    }
}

/// Shows or hides the overlay on the main thread, deciding there from the current status, not
/// the one this call was made for: two status changes racing from worker threads can queue
/// their calls in either order, and the window must end in the state the last status asks for.
///
/// On macOS, showing puts it on the Space in front, full-screen apps included. Tao sets "all
/// Spaces" once at launch, but a hidden window misses Spaces made after that (a window going
/// full screen) and comes up on its old Space, covered. Clearing and setting the behavior again
/// before each show puts it back on every Space; full-screen auxiliary lets it sit over a
/// full-screen app. `orderFrontRegardless` orders it in without making it key or activating
/// Silas, which runs in the background.
fn place_overlay(app: &AppHandle, w: tauri::WebviewWindow, enabled: bool) {
    let app = app.clone();
    let _ = w.clone().run_on_main_thread(move || {
        // Only the status lock, held for a copy: no thread holds it while waiting on this one.
        let show = enabled
            && app
                .try_state::<Shared>()
                .is_some_and(|c| overlay_status(c.status().0));
        #[cfg(target_os = "macos")]
        order_overlay(&w, show);
        #[cfg(not(target_os = "macos"))]
        let _ = if show { w.show() } else { w.hide() };
    });
}

/// Runs on the main thread. The window is looked up here, so it can't be gone by the time
/// it's used.
#[cfg(target_os = "macos")]
fn order_overlay(w: &tauri::WebviewWindow, show: bool) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    // NSWindowCollectionBehavior: CanJoinAllSpaces | Stationary | IgnoresCycle |
    // FullScreenAuxiliary.
    const BEHAVIOR: usize = 1 | 1 << 4 | 1 << 6 | 1 << 8;
    let Ok(ns) = w.ns_window() else { return };
    let Some(win) = (unsafe { (ns as *mut AnyObject).as_ref() }) else {
        return;
    };
    unsafe {
        if !show {
            let _: () = msg_send![win, orderOut: std::ptr::null::<AnyObject>()];
            return;
        }
        // Already up (Recording, then Writing): leave its Spaces alone so it can't blink.
        let visible: bool = msg_send![win, isVisible];
        if !visible {
            let _: () = msg_send![win, setCollectionBehavior: 0usize];
            let _: () = msg_send![win, setCollectionBehavior: BEHAVIOR];
        }
        let _: () = msg_send![win, orderFrontRegardless];
    }
}

