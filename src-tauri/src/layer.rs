//! The recording overlay on Wayland. An ordinary window can't place itself there, and a tiling
//! compositor like Hyprland tiles it and hands it the keyboard, so the dictation would be typed
//! into the overlay. A layer-shell surface floats over every window, sits where it is anchored
//! and never takes the keyboard.
//!
//! gtk-layer-shell is loaded at run time so Silas still starts where it isn't installed. There,
//! a tiling compositor gets no overlay (it would take the keyboard); GNOME and KDE get the plain
//! window they always had.

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicU8, Ordering};

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

const UNSET: u8 = 0;
const LAYER: u8 = 1;
const HIDDEN: u8 = 2;
const PLAIN: u8 = 3;
static MODE: AtomicU8 = AtomicU8::new(UNSET);

// gtk-layer-shell enums.
const LAYER_OVERLAY: c_int = 3;
const EDGE_BOTTOM: c_int = 3;
const KEYBOARD_NONE: c_int = 0;

/// Distance from the bottom of the screen, as on macOS.
pub const MARGIN: c_int = 72;

/// How the overlay can be shown on this desktop.
pub enum Overlay {
    /// A layer surface: show and hide it, never move it.
    Layer,
    /// A tiling Wayland compositor without layer-shell: showing a window would steal the
    /// keyboard.
    Hidden,
    /// X11, macOS, Windows: a normal always-on-top window that Silas positions.
    Plain,
}

pub fn mode() -> Overlay {
    match MODE.load(Ordering::Relaxed) {
        LAYER => Overlay::Layer,
        HIDDEN => Overlay::Hidden,
        _ => Overlay::Plain,
    }
}

/// Turn the overlay window into a layer surface. Must run on the main thread before the
/// window is first shown.
pub fn setup(window: &tauri::WebviewWindow) {
    let wayland = gtk::gdk::Display::default().is_some_and(|d| d.backend().is_wayland());
    if !wayland {
        MODE.store(PLAIN, Ordering::Relaxed);
        return;
    }
    let ok = match window.gtk_window() {
        Ok(w) => unsafe { init(w.upcast_ref::<gtk::Window>()) },
        Err(_) => false,
    };
    let tiling = crate::hypr::running() || std::env::var_os("SWAYSOCK").is_some();
    let mode = match (ok, tiling) {
        (true, _) => LAYER,
        (false, true) => HIDDEN,
        (false, false) => PLAIN,
    };
    log::info!("overlay: {}", ["", "layer surface", "hidden (no gtk-layer-shell)", "window"][mode as usize]);
    MODE.store(mode, Ordering::Relaxed);
}

unsafe fn init(window: &gtk::Window) -> bool {
    if window.is_realized() {
        log::warn!("overlay window was realized before layer-shell setup");
        return false;
    }
    let Ok(lib) = libloading::Library::new("libgtk-layer-shell.so.0") else {
        return false;
    };
    type Supported = unsafe extern "C" fn() -> c_int;
    type Init = unsafe extern "C" fn(*mut c_void);
    type SetInt = unsafe extern "C" fn(*mut c_void, c_int);
    type SetEdge = unsafe extern "C" fn(*mut c_void, c_int, c_int);
    type SetName = unsafe extern "C" fn(*mut c_void, *const std::ffi::c_char);
    let (Ok(supported), Ok(init), Ok(layer), Ok(anchor), Ok(margin), Ok(keyboard), Ok(namespace)) = (
        lib.get::<Supported>(b"gtk_layer_is_supported\0"),
        lib.get::<Init>(b"gtk_layer_init_for_window\0"),
        lib.get::<SetInt>(b"gtk_layer_set_layer\0"),
        lib.get::<SetEdge>(b"gtk_layer_set_anchor\0"),
        lib.get::<SetEdge>(b"gtk_layer_set_margin\0"),
        lib.get::<SetInt>(b"gtk_layer_set_keyboard_mode\0"),
        lib.get::<SetName>(b"gtk_layer_set_namespace\0"),
    ) else {
        return false;
    };
    if supported() == 0 {
        return false;
    }
    let w: *mut gtk::ffi::GtkWindow = window.to_glib_none().0;
    let w = w as *mut c_void;
    init(w);
    namespace(w, c"silas-overlay".as_ptr());
    layer(w, LAYER_OVERLAY);
    // Anchored to the bottom edge only: centered across the monitor with the keyboard.
    anchor(w, EDGE_BOTTOM, 1);
    margin(w, EDGE_BOTTOM, MARGIN);
    keyboard(w, KEYBOARD_NONE);
    // The functions live as long as the process.
    std::mem::forget(lib);
    true
}
