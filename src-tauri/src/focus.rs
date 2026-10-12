//! Which app has the keyboard, and whether its layout is one the edit watcher can read.
//!
//! The edit watcher replays keys against its copy of the typed text. That only works while
//! the same app keeps focus (handy-keys reports no plain mouse clicks) and the layout maps
//! keys the way `keys::char_for` expects.

/// An id for the focused app or window, or None where the OS can't tell us (Linux).
#[cfg(target_os = "macos")]
pub fn focused() -> Option<u64> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    let cls = AnyClass::get(c"NSWorkspace")?;
    unsafe {
        let ws: *mut AnyObject = msg_send![cls, sharedWorkspace];
        let app: *mut AnyObject = msg_send![ws.as_ref()?, frontmostApplication];
        let pid: i32 = msg_send![app.as_ref()?, processIdentifier];
        Some(pid as u64)
    }
}

#[cfg(target_os = "windows")]
pub fn focused() -> Option<u64> {
    let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    (!hwnd.is_null()).then_some(hwnd as u64)
}

#[cfg(target_os = "linux")]
pub fn focused() -> Option<u64> {
    None
}

/// True when the active keyboard layout is US English (or Apple's "ABC").
#[cfg(target_os = "macos")]
pub fn us_layout() -> bool {
    let out = std::process::Command::new("defaults")
        .args(["read", "com.apple.HIToolbox", "AppleSelectedInputSources"])
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.contains("\"KeyboardLayout Name\" = ABC;")
                || s.contains("\"KeyboardLayout Name\" = \"U.S.\";")
                || s.contains("\"KeyboardLayout Name\" = US;")
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "windows")]
pub fn us_layout() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    unsafe {
        let tid = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        (GetKeyboardLayout(tid) as usize & 0xffff) == 0x0409
    }
}

#[cfg(target_os = "linux")]
pub fn us_layout() -> bool {
    // X11 reports the layout; Wayland compositors don't share it, so trust the locale.
    if let Ok(o) = std::process::Command::new("setxkbmap")
        .arg("-query")
        .output()
    {
        let s = String::from_utf8_lossy(&o.stdout);
        if let Some(line) = s.lines().find(|l| l.starts_with("layout:")) {
            return line
                .split_whitespace()
                .nth(1)
                .is_some_and(|l| l == "us" || l.starts_with("us,"));
        }
    }
    std::env::var("LANG")
        .map(|l| l.starts_with("en_US"))
        .unwrap_or(false)
}
