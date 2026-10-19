//! Which app has the keyboard, where a dictation should land, and whether the layout is one
//! the edit watcher can read.
//!
//! The edit watcher replays keys against its copy of the typed text. That only works while
//! the same app keeps focus (handy-keys reports no plain mouse clicks) and the layout maps
//! keys the way `keys::char_for` expects.

/// An id for the focused app or window, or None where the OS can't tell us (Linux).
#[cfg(target_os = "macos")]
pub fn focused() -> Option<u64> {
    if let Some(pid) = mac::front_pid() {
        return Some(pid as u64);
    }
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

/// The text field that had the keyboard when the hotkey went down. A window that opens
/// mid-dictation steals focus without the user asking, so Silas puts the keyboard back here
/// before it types, unless the user clicked into another text field to choose a new place.
/// Only a click counts as choosing; switching apps from the keyboard does not.
pub struct Target {
    #[cfg(target_os = "macos")]
    mac: mac::Target,
}

impl Target {
    #[cfg(target_os = "macos")]
    pub fn capture() -> Option<Self> {
        mac::Target::capture().map(|mac| Self { mac })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn capture() -> Option<Self> {
        None
    }

    /// Puts the keyboard back on the captured field if something else took it and the user
    /// didn't click into another field.
    pub fn restore(&self) -> Restore {
        #[cfg(target_os = "macos")]
        return self.mac.restore();
        #[cfg(not(target_os = "macos"))]
        Restore::Elsewhere
    }

    /// False once something else has clearly taken the keyboard from the captured window.
    /// An app that can't answer counts as still here, so a slow reply never cuts typing short.
    pub fn still_here(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.mac.still_here();
        #[cfg(not(target_os = "macos"))]
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Restore {
    /// Focus never left the captured window.
    InPlace,
    /// Focus was taken and Silas put it back.
    Restored,
    /// The user clicked into another field, or Silas can't tell: type where focus is now.
    Elsewhere,
    /// Focus was taken and Silas couldn't put it back; typing now would hit the wrong window.
    Failed,
}

#[cfg(target_os = "macos")]
mod mac {
    //! macOS ignores activation requests from background apps, Accessibility's AXFrontmost
    //! included. Window managers move focus through WindowServer (SkyLight) instead: make the
    //! process frontmost, then tell it which exact window is key. Silas does the same.

    use super::Restore;
    use core_foundation::base::{CFEqual, CFType, CFTypeRef, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    type AXUIElementRef = *const c_void;
    type Psn = [u8; 8];

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(el: AXUIElementRef, attr: CFStringRef, out: *mut CFTypeRef) -> i32;
        fn AXUIElementSetAttributeValue(el: AXUIElementRef, attr: CFStringRef, value: CFTypeRef) -> i32;
        fn AXUIElementPerformAction(el: AXUIElementRef, action: CFStringRef) -> i32;
        fn _AXUIElementGetWindow(el: AXUIElementRef, wid: *mut u32) -> i32;
        fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, seconds: f32) -> i32;
        fn AXUIElementGetTypeID() -> core_foundation::base::CFTypeID;
        fn AXUIElementCopyElementAtPosition(app: AXUIElementRef, x: f32, y: f32, out: *mut AXUIElementRef) -> i32;
        fn GetProcessForPID(pid: i32, psn: *mut Psn) -> i32;
        fn GetProcessPID(psn: *const Psn, pid: *mut i32) -> i32;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceCounterForEventType(state: i32, event_type: u32) -> u32;
    }

    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `SLPSSetFrontProcessWithOptions(psn, window id, options)`
    type SetFront = unsafe extern "C" fn(*const Psn, u32, u32) -> i32;
    /// `SLPSPostEventRecordTo(psn, 248-byte record)`
    type PostRecord = unsafe extern "C" fn(*const Psn, *const u8) -> i32;
    /// `_SLPSGetFrontProcess(psn)`
    type GetFront = unsafe extern "C" fn(*mut Psn) -> i32;

    struct SkyLight {
        set_front: SetFront,
        post_record: PostRecord,
        get_front: GetFront,
    }

    fn skylight() -> Option<&'static SkyLight> {
        static SL: OnceLock<Option<SkyLight>> = OnceLock::new();
        SL.get_or_init(|| unsafe {
            const RTLD_LAZY: c_int = 1;
            const RTLD_GLOBAL: c_int = 8;
            let h = dlopen(c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(), RTLD_LAZY | RTLD_GLOBAL);
            if h.is_null() {
                return None;
            }
            let sym = |name: &std::ffi::CStr| Some(dlsym(h, name.as_ptr())).filter(|p| !p.is_null());
            Some(SkyLight {
                set_front: std::mem::transmute::<*mut c_void, SetFront>(sym(c"SLPSSetFrontProcessWithOptions")?),
                post_record: std::mem::transmute::<*mut c_void, PostRecord>(sym(c"SLPSPostEventRecordTo")?),
                get_front: std::mem::transmute::<*mut c_void, GetFront>(sym(c"_SLPSGetFrontProcess")?),
            })
        })
        .as_ref()
    }

    /// The process WindowServer has in front. NSWorkspace lags behind SkyLight changes.
    pub(super) fn front_pid() -> Option<i32> {
        let sl = skylight()?;
        let mut psn: Psn = [0; 8];
        let mut pid = 0;
        unsafe { ((sl.get_front)(&mut psn) == 0 && GetProcessPID(&psn, &mut pid) == 0).then_some(pid) }
    }

    /// Mouse-down presses since boot, any button. A change means the user clicked.
    fn clicks() -> u32 {
        const HID_SYSTEM_STATE: i32 = 1;
        // kCGEventLeftMouseDown, kCGEventRightMouseDown, kCGEventOtherMouseDown.
        [1u32, 3, 25]
            .iter()
            .map(|&t| unsafe { CGEventSourceCounterForEventType(HID_SYSTEM_STATE, t) })
            .fold(0u32, u32::wrapping_add)
    }

    /// Takes ownership of an element. A hung app must not stall a dictation, so every call
    /// into it gives up after a quarter second instead of the default six.
    fn wrap(el: AXUIElementRef) -> Option<CFType> {
        if el.is_null() {
            return None;
        }
        let el = unsafe { CFType::wrap_under_create_rule(el) };
        if el.type_of() == unsafe { AXUIElementGetTypeID() } {
            unsafe { AXUIElementSetMessagingTimeout(el.as_CFTypeRef(), 0.25) };
        }
        Some(el)
    }

    const AX_INVALID_ELEMENT: i32 = -25202;

    fn attr_err(el: &CFType, name: &str) -> Result<CFType, i32> {
        let mut out: CFTypeRef = std::ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(el.as_CFTypeRef(), CFString::new(name).as_concrete_TypeRef(), &mut out)
        };
        if err != 0 {
            return Err(err);
        }
        wrap(out).ok_or(err)
    }

    fn attr(el: &CFType, name: &str) -> Option<CFType> {
        attr_err(el, name).ok()
    }

    fn set_true(el: &CFType, name: &str) {
        unsafe {
            AXUIElementSetAttributeValue(
                el.as_CFTypeRef(),
                CFString::new(name).as_concrete_TypeRef(),
                CFBoolean::true_value().as_CFTypeRef(),
            );
        }
    }

    fn window_id(window: &CFType) -> Option<u32> {
        let mut wid = 0;
        (unsafe { _AXUIElementGetWindow(window.as_CFTypeRef(), &mut wid) } == 0 && wid != 0).then_some(wid)
    }

    fn app(pid: i32) -> Option<CFType> {
        wrap(unsafe { AXUIElementCreateApplication(pid) })
    }

    fn same(a: &CFType, b: &CFType) -> bool {
        unsafe { CFEqual(a.as_CFTypeRef(), b.as_CFTypeRef()) != 0 }
    }

    /// A text field (sliders and pop-up menus also take focus on click, but aren't).
    fn editable(el: &CFType) -> bool {
        let role = attr(el, "AXRole").and_then(|r| r.downcast::<CFString>()).map(|r| r.to_string());
        matches!(role.as_deref(), Some("AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField"))
    }

    /// The element under the mouse, in the given app.
    fn under_mouse(pid: i32) -> Option<CFType> {
        use core_graphics::event::CGEvent;
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
        let at = CGEvent::new(CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()?).ok()?.location();
        let mut out: AXUIElementRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyElementAtPosition(app(pid)?.as_CFTypeRef(), at.x as f32, at.y as f32, &mut out) };
        if err == 0 { wrap(out) } else { None }
    }

    /// `el` is `field` or sits inside it (a browser reports the text run under the mouse).
    fn within(el: CFType, field: &CFType) -> bool {
        let mut cur = Some(el);
        for _ in 0..8 {
            match cur {
                Some(c) if same(&c, field) => return true,
                Some(c) => cur = attr(&c, "AXParent"),
                None => return false,
            }
        }
        false
    }

    fn focused_window_id(pid: i32) -> Option<u32> {
        window_id(&attr(&app(pid)?, "AXFocusedWindow")?)
    }

    /// Tell the app which window is key: the paired focus records window managers send.
    fn make_key(sl: &SkyLight, psn: &Psn, wid: u32) {
        for kind in [0x01u8, 0x02] {
            let mut record = [0u8; 0xF8];
            record[0x04] = 0xF8;
            record[0x08] = kind;
            record[0x3A] = 0x10;
            record[0x3C..0x40].copy_from_slice(&wid.to_le_bytes());
            record[0x20..0x30].fill(0xFF);
            unsafe { (sl.post_record)(psn, record.as_ptr()) };
        }
    }

    pub struct Target {
        pid: i32,
        window: CFType,
        wid: u32,
        /// The text field; browsers and Electron apps may not report one.
        element: Option<CFType>,
        clicks: u32,
    }

    // AXUIElement is a CoreFoundation type that may be used from any thread.
    unsafe impl Send for Target {}

    impl Target {
        pub fn capture() -> Option<Self> {
            // Without SkyLight Silas can't move focus back, so it doesn't try: it types where
            // focus is, as it did before.
            skylight()?;
            let pid = super::focused()? as i32;
            let app = app(pid)?;
            let window = attr(&app, "AXFocusedWindow")?;
            Some(Self {
                pid,
                wid: window_id(&window)?,
                window,
                element: attr(&app, "AXFocusedUIElement"),
                clicks: clicks(),
            })
        }

        /// Keystrokes would land in the captured window.
        pub(super) fn in_place(&self) -> bool {
            front_pid() == Some(self.pid) && focused_window_id(self.pid) == Some(self.wid)
        }

        pub fn still_here(&self) -> bool {
            let front_ok = front_pid().is_none_or(|p| p == self.pid);
            front_ok && focused_window_id(self.pid).is_none_or(|w| w == self.wid)
        }

        /// The user clicked into a different text field since key-down: that's where they
        /// want the text. A click that dismissed a pop-up, dragged it or moved the caret is not,
        /// and neither is a field that was already focused when they clicked somewhere else.
        fn user_chose_elsewhere(&self) -> bool {
            if clicks() == self.clicks {
                return false;
            }
            let Some(pid) = front_pid() else { return false };
            let Some(now) = app(pid).and_then(|a| attr(&a, "AXFocusedUIElement")) else {
                return false;
            };
            let original = pid == self.pid && self.element.as_ref().is_some_and(|e| same(e, &now));
            !original && editable(&now) && under_mouse(pid).is_some_and(|el| within(el, &now))
        }

        /// Ask the app to focus the captured field if something else in it has focus.
        fn refocus_field(&self) {
            if let Some(e) = &self.element {
                let focused = app(self.pid).and_then(|a| attr(&a, "AXFocusedUIElement"));
                if focused.is_none_or(|f| !same(&f, e)) {
                    set_true(e, "AXFocused");
                }
            }
        }

        pub fn restore(&self) -> Restore {
            if self.user_chose_elsewhere() {
                return Restore::Elsewhere;
            }
            if self.in_place() {
                // Same window. Don't re-activate it (that can clear a browser's own focus),
                // but a sheet or dialog inside it may have taken the field.
                self.refocus_field();
                return Restore::InPlace;
            }
            if let Err(AX_INVALID_ELEMENT) = attr_err(&self.window, "AXRole") {
                // The window is gone. Another window of the same app is a fine home for the
                // text; anything else would be a stranger's window.
                log::info!("the window the dictation started in is gone");
                return if front_pid() == Some(self.pid) { Restore::Elsewhere } else { Restore::Failed };
            }
            let Some(sl) = skylight() else { return Restore::Failed };
            let mut psn: Psn = [0; 8];
            if unsafe { GetProcessForPID(self.pid, &mut psn) } != 0 {
                return Restore::Failed;
            }
            for _ in 0..2 {
                // kCPSUserGenerated lets the app install its key window before it handles keys.
                unsafe { (sl.set_front)(&psn, self.wid, 0x200) };
                make_key(sl, &psn, self.wid);
                unsafe {
                    AXUIElementPerformAction(self.window.as_CFTypeRef(), CFString::new("AXRaise").as_concrete_TypeRef());
                }
                let deadline = Instant::now() + Duration::from_millis(400);
                while !self.in_place() && Instant::now() < deadline {
                    if self.user_chose_elsewhere() {
                        return Restore::Elsewhere;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                // A window still opening can take focus right back; wait until it stays put.
                if self.settled() {
                    break;
                }
            }
            if !self.in_place() {
                log::warn!("could not put the keyboard back where the dictation started");
                return Restore::Failed;
            }
            // The app restores its own last field on activation; nudge it in case it didn't.
            self.refocus_field();
            log::info!("put the keyboard back where the dictation started");
            Restore::Restored
        }

        /// In place now and still in place 150 ms later.
        fn settled(&self) -> bool {
            let until = Instant::now() + Duration::from_millis(150);
            while Instant::now() < until {
                if !self.in_place() {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            self.in_place()
        }
    }

    #[cfg(test)]
    pub fn front() -> Option<i32> {
        front_pid()
    }

    #[cfg(test)]
    pub fn steal(pid: i32) -> bool {
        let (Some(sl), Some(wid)) = (skylight(), focused_window_id(pid)) else { return false };
        let mut psn: Psn = [0; 8];
        unsafe { GetProcessForPID(pid, &mut psn) };
        unsafe { (sl.set_front)(&psn, wid, 0x200) };
        make_key(sl, &psn, wid);
        true
    }
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// The typing guard asks this every 16 key events; it must stay well under their pace.
    /// Reads focus only. `cargo test --lib focus -- --ignored`
    #[test]
    #[ignore]
    fn checking_focus_is_cheap() {
        let target = Target::capture().expect("a focused window and Accessibility access");
        let t = Instant::now();
        for _ in 0..50 {
            assert!(target.still_here());
        }
        let each = t.elapsed() / 50;
        eprintln!("still_here: {each:?} per call");
        assert!(each < Duration::from_millis(5), "{each:?}");
    }

    /// Live check for the window-steals-focus bug. Needs a Mac session and Accessibility
    /// access for the terminal: `cargo test --lib focus -- --ignored`. It opens Calculator
    /// for a moment, then quits it.
    #[test]
    #[ignore]
    fn puts_the_keyboard_back_after_a_window_steals_it() {
        struct Quit;
        impl Drop for Quit {
            fn drop(&mut self) {
                let _ = Command::new("osascript").args(["-e", "quit app \"Calculator\""]).status();
            }
        }
        let target = Target::capture().expect("a focused window and Accessibility access");
        let before = mac::front();
        let _quit = Quit;
        Command::new("open").args(["-g", "-a", "Calculator"]).status().unwrap();
        let mut calc = None;
        for _ in 0..100 {
            calc = String::from_utf8(Command::new("pgrep").args(["-x", "Calculator"]).output().unwrap().stdout)
                .ok()
                .and_then(|p| p.trim().parse::<i32>().ok());
            if calc.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let calc = calc.expect("Calculator started");
        // Steal focus the way a new window does: Calculator comes to the front.
        let deadline = Instant::now() + Duration::from_secs(5);
        while mac::front() != Some(calc) && Instant::now() < deadline {
            mac::steal(calc);
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(mac::front(), Some(calc), "Calculator never took focus");
        assert_eq!(target.restore(), Restore::Restored);
        assert_eq!(mac::front(), before);
        std::thread::sleep(Duration::from_millis(300));
        assert!(target.still_here(), "focus left again after the restore");
        assert!(target.mac.in_place());
    }
}
