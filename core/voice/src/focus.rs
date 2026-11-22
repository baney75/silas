//! Which app has the keyboard, where a dictation should land, and whether the layout is one
//! the edit watcher can read.
//!
//! The edit watcher replays keys against its copy of the typed text. That only works while
//! the same app keeps focus (handy-keys reports no plain mouse clicks) and the layout maps
//! keys the way `keys::char_for` expects.

/// An id for the focused app or window, or None where the OS can't tell us (Linux outside
/// Hyprland).
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

/// Hyprland says which window has the keyboard; other Linux desktops don't, so there this is
/// None and nothing guards the typing or the edit learning, as before.
#[cfg(target_os = "linux")]
pub fn focused() -> Option<u64> {
    hypr::active().map(|w| w.id)
}

/// True while a password field has the keyboard (macOS secure input). macOS then hides every
/// key from other apps, and anything typed would land in the password.
#[cfg(target_os = "macos")]
pub fn secure_input() -> bool {
    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn IsSecureEventInputEnabled() -> u8;
    }
    unsafe { IsSecureEventInputEnabled() != 0 }
}

/// Other systems don't say.
#[cfg(not(target_os = "macos"))]
pub fn secure_input() -> bool {
    false
}

/// The focused app's id, for its style (`style.rs`): the bundle id on macOS
/// (`com.mitchellh.ghostty`), the exe name on Windows (`windowsterminal.exe`), the window class on
/// Hyprland (`kitty`). None where the system doesn't say (Linux outside Hyprland).
#[cfg(target_os = "macos")]
pub fn focused_app() -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    let cls = AnyClass::get(c"NSWorkspace")?;
    unsafe {
        let ws: *mut AnyObject = msg_send![cls, sharedWorkspace];
        let app: *mut AnyObject = msg_send![ws.as_ref()?, frontmostApplication];
        mac_bundle_id(app.as_ref()?)
    }
}

/// The bundle id of the app with this process id.
#[cfg(target_os = "macos")]
fn mac_app_of(pid: i32) -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    let cls = AnyClass::get(c"NSRunningApplication")?;
    unsafe {
        let app: *mut AnyObject = msg_send![cls, runningApplicationWithProcessIdentifier: pid];
        mac_bundle_id(app.as_ref()?)
    }
}

/// # Safety
/// `app` must be an `NSRunningApplication`.
#[cfg(target_os = "macos")]
unsafe fn mac_bundle_id(app: &objc2::runtime::AnyObject) -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let id: *mut AnyObject = msg_send![app, bundleIdentifier];
    let utf8: *const std::ffi::c_char = msg_send![id.as_ref()?, UTF8String];
    if utf8.is_null() {
        return None;
    }
    Some(std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned())
}

#[cfg(target_os = "windows")]
pub fn focused_app() -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        exe_name(&String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[cfg(target_os = "linux")]
pub fn focused_app() -> Option<String> {
    hypr::active_class()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn focused_app() -> Option<String> {
    None
}

/// `C:\Program Files\Git\usr\bin\mintty.exe` → `mintty.exe`: the file name in lower case.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn exe_name(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?.trim();
    (!name.is_empty()).then(|| name.to_lowercase())
}

/// Hyprland's focused window, over the IPC socket `hyprctl` talks to (a read takes about 36 µs;
/// starting `hyprctl` took milliseconds). Built everywhere so the parsing is tested on every
/// platform; only Linux talks to Hyprland.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod hypr {
    pub const OVERLAY_TITLE: &str = "Silas Overlay";

    /// One window as Hyprland reports it.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Window {
        /// Address and class as one id (see `parse`).
        pub id: u64,
        /// What a focus dispatch needs to give it the keyboard back.
        pub address: u64,
        /// Silas's recording overlay, which can take the keyboard as it opens even though it
        /// asks not to. Its other windows count as any app's: a click there is a choice.
        pub overlay: bool,
    }

    /// The request socket: in the runtime dir since Hyprland 0.40, in /tmp before.
    #[cfg(target_os = "linux")]
    fn socket() -> Option<std::path::PathBuf> {
        let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from);
        runtime
            .into_iter()
            .chain([std::path::PathBuf::from("/tmp")])
            .map(|dir| dir.join("hypr").join(&sig).join(".socket.sock"))
            .find(|p| p.exists())
    }

    /// Running under Hyprland.
    #[cfg(target_os = "linux")]
    pub fn running() -> bool {
        socket().is_some()
    }

    /// Sends one request and reads the whole reply. None when Hyprland isn't there or didn't
    /// answer.
    #[cfg(target_os = "linux")]
    fn ask(request: &str) -> Option<String> {
        use std::io::{Read, Write};
        let mut s = std::os::unix::net::UnixStream::connect(socket()?).ok()?;
        // Hyprland answers from its main loop; a stalled compositor must not stall typing.
        let wait = Some(std::time::Duration::from_millis(300));
        s.set_read_timeout(wait).ok()?;
        s.set_write_timeout(wait).ok()?;
        s.write_all(request.as_bytes()).ok()?;
        let mut out = String::new();
        s.read_to_string(&mut out).ok()?;
        Some(out)
    }

    /// Some(None) when Hyprland answered that no window has the keyboard (an empty workspace);
    /// None when it didn't answer.
    #[cfg(target_os = "linux")]
    pub fn focused() -> Option<Option<Window>> {
        answer(&ask("j/activewindow")?)
    }

    #[cfg(target_os = "linux")]
    pub fn active() -> Option<Window> {
        focused().flatten()
    }

    /// The focused window and its class, from one question to Hyprland.
    #[cfg(target_os = "linux")]
    pub fn active_with_class() -> Option<(Window, Option<String>)> {
        let json = ask("j/activewindow")?;
        Some((answer(&json)??, class(&json)))
    }

    #[cfg(target_os = "linux")]
    pub fn active_class() -> Option<String> {
        class(&ask("j/activewindow")?)
    }

    /// The window's class (`kitty`, `com.mitchellh.ghostty`), which names the app.
    pub fn class(json: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let c = v.get("class")?.as_str()?.trim();
        (!c.is_empty()).then(|| c.to_string())
    }

    /// Gives the keyboard to the window at `address`. False when Hyprland refused.
    #[cfg(target_os = "linux")]
    pub fn focus(address: u64) -> bool {
        focus_requests(address).iter().any(|r| ask(r).is_some_and(|reply| reply.trim() == "ok"))
    }

    /// Hyprland 0.53 and later take Lua dispatches; older ones, and a Lua config that refuses
    /// the call, take the classic dispatcher.
    pub fn focus_requests(address: u64) -> [String; 2] {
        let w = window_arg(address);
        [format!("dispatch hl.dsp.focus({{ window = \"{w}\" }})"), format!("dispatch focuswindow {w}")]
    }

    /// How a dispatch names a window: the address as `activewindow` prints it.
    pub fn window_arg(address: u64) -> String {
        format!("address:0x{address:x}")
    }

    /// The main keyboard's active layout is plain US, or None when Hyprland can't say.
    #[cfg(target_os = "linux")]
    pub fn us_layout() -> Option<bool> {
        layout_is_us(&ask("j/devices")?)
    }

    pub fn layout_is_us(devices: &str) -> Option<bool> {
        let v: serde_json::Value = serde_json::from_str(devices).ok()?;
        let keyboards = v.get("keyboards")?.as_array()?;
        let main = keyboards.iter().find(|k| k.get("main").and_then(|m| m.as_bool()) == Some(true))?;
        let active = main.get("active_layout_index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        let field = |key: &str| main.get(key).and_then(|f| f.as_str()).unwrap_or("");
        Some(is_plain_us(field("layout"), field("variant"), active))
    }

    /// "us" with no variant at the active index: dvorak, colemak and intl map keys differently.
    pub fn is_plain_us(layouts: &str, variants: &str, active: usize) -> bool {
        let layout = layouts.split(',').nth(active).map(str::trim);
        let variant = variants.split(',').nth(active).map(str::trim).unwrap_or("");
        layout == Some("us") && variant.is_empty()
    }

    /// An `activewindow` reply: Some(None) for `{}`, None when it isn't JSON.
    pub fn answer(reply: &str) -> Option<Option<Window>> {
        let v: serde_json::Value = serde_json::from_str(reply).ok()?;
        Some(window(&v))
    }

    /// The window in a reply, or None when no window has the keyboard (`{}`) or the reply isn't
    /// JSON.
    #[cfg(test)]
    pub fn parse(json: &str) -> Option<Window> {
        answer(json).flatten()
    }

    /// The window's address and class as one id. An address alone can come back for a later
    /// window once the first one closes; the class makes that much less likely to pass. The
    /// overlay is known by its title (`tauri.conf.json`), which holds inside a Flatpak too, where
    /// pids don't match.
    fn window(v: &serde_json::Value) -> Option<Window> {
        let addr = v.get("address")?.as_str()?.trim();
        let address = u64::from_str_radix(addr.trim_start_matches("0x"), 16).ok().filter(|a| *a != 0)?;
        let class = v.get("class").and_then(|c| c.as_str()).unwrap_or("");
        // FNV-1a over the class, seeded with the address.
        let mut h = address ^ 0xcbf2_9ce4_8422_2325;
        for b in class.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let overlay = v.get("title").and_then(|t| t.as_str()) == Some(OVERLAY_TITLE);
        Some(Window { id: h, address, overlay })
    }
}

/// Running under Hyprland, which tiles plain windows and hands them the keyboard.
#[cfg(target_os = "linux")]
pub fn hyprland() -> bool {
    hypr::running()
}

/// Where a dictation's text goes: the text field that had the keyboard when the hotkey went
/// down, or the one the user clicked into since. A window that opens mid-dictation steals
/// focus without the user asking, so Silas puts the keyboard back here before it types and
/// stops typing if it leaves. Only a click on a text field counts as choosing; switching apps
/// from the keyboard does not.
pub struct Target {
    #[cfg(target_os = "macos")]
    mac: mac::Target,
    /// Hyprland: the window that had the keyboard.
    #[cfg(target_os = "linux")]
    window: hypr::Window,
    /// Its class, which names the app (`style.rs`).
    #[cfg(target_os = "linux")]
    class: Option<String>,
}

/// Where the Linux target stands, from the window that had the keyboard and the one that has it
/// now. None now: Hyprland didn't answer, so nothing is known and typing goes ahead. When
/// Silas's overlay took the keyboard, `refocus` hands it back to the target and reports
/// the window that has it after.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_restore(
    then: &hypr::Window,
    now: Option<hypr::Window>,
    refocus: impl FnOnce(u64) -> Option<hypr::Window>,
) -> Restore {
    match now {
        Some(w) if w.id == then.id => Restore::InPlace,
        // Silas's overlay took it; nobody chose that, so put the keyboard back.
        Some(w) if w.overlay && !then.overlay => match refocus(then.address) {
            Some(w) if w.id == then.id => Restore::Restored,
            _ => Restore::Failed,
        },
        // Hyprland could move focus back, but a click there is the user's choice; keep the
        // text in History rather than type into the wrong window.
        Some(_) => Restore::Failed,
        None => Restore::Elsewhere,
    }
}

/// `linux_restore` from Hyprland's whole answer, where Some(None) means no window has the
/// keyboard: the user went to an empty workspace. Nobody chose another window there, so the
/// keyboard goes back to the target rather than the text going nowhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_restore_from(
    then: &hypr::Window,
    now: Option<Option<hypr::Window>>,
    refocus: impl FnOnce(u64) -> Option<hypr::Window>,
) -> Restore {
    match now {
        Some(None) => match refocus(then.address) {
            Some(w) if w.id == then.id => Restore::Restored,
            _ => Restore::Failed,
        },
        now => linux_restore(then, now.flatten(), refocus),
    }
}

/// Hands the keyboard back to the window at `address` and says which window has it now.
#[cfg(target_os = "linux")]
fn hypr_refocus(address: u64) -> Option<hypr::Window> {
    if !hypr::focus(address) {
        return None;
    }
    hypr::active()
}

impl Target {
    #[cfg(target_os = "macos")]
    pub fn capture() -> Option<Self> {
        mac::Target::capture().map(|mac| Self { mac })
    }

    #[cfg(target_os = "linux")]
    pub fn capture() -> Option<Self> {
        hypr::active_with_class().map(|(window, class)| Self { window, class })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn capture() -> Option<Self> {
        None
    }

    /// The id of the app this target is in (see `focused_app`), as it was at key-down. None where
    /// only the app with the keyboard now can be asked.
    pub fn app(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        return mac_app_of(self.mac.pid());
        #[cfg(target_os = "linux")]
        return self.class.clone();
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        None
    }

    /// Puts the keyboard back on the captured field if something else took it and the user
    /// didn't click into another field.
    pub fn restore(&self) -> Restore {
        #[cfg(target_os = "macos")]
        return self.mac.restore();
        #[cfg(target_os = "linux")]
        return linux_restore_from(&self.window, hypr::focused(), hypr_refocus);
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        Restore::Elsewhere
    }

    /// While typing: the overlay taking the keyboard is handed back; a move to an empty
    /// workspace stops the typing.
    #[cfg(target_os = "linux")]
    fn linux_here(&self) -> bool {
        match hypr::focused() {
            Some(None) => false,
            now => linux_restore(&self.window, now.flatten(), hypr_refocus) != Restore::Failed,
        }
    }

    /// False once something else has clearly taken the keyboard from the captured window.
    /// An app that can't answer counts as still here, so a slow reply never cuts typing short.
    /// `after_focus_key`: Silas just typed a Tab or Return, which may move focus to another
    /// field on purpose, so from then on only the window is checked.
    pub fn still_here(&self, after_focus_key: bool) -> bool {
        #[cfg(target_os = "macos")]
        return self.mac.still_here(after_focus_key);
        // Window by window: a Tab or Return that moves to another field stays in the window.
        #[cfg(target_os = "linux")]
        {
            let _ = after_focus_key;
            self.linux_here()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = after_focus_key;
            true
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Restore {
    /// Focus never left the captured window.
    InPlace,
    /// Focus was taken and Silas put it back.
    Restored,
    /// The target window is gone and another window of the same app has the keyboard: type
    /// where focus is now.
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};
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
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementGetPid(el: AXUIElementRef, pid: *mut i32) -> i32;
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

    /// Left and right mouse-down presses since boot. A change means the user clicked. Other
    /// buttons are left out: one may be the dictation hotkey.
    fn clicks() -> u32 {
        const HID_SYSTEM_STATE: i32 = 1;
        // kCGEventLeftMouseDown, kCGEventRightMouseDown.
        [1u32, 3]
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

    fn mouse_location() -> Option<(f64, f64)> {
        use core_graphics::event::CGEvent;
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
        let at = CGEvent::new(CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()?).ok()?.location();
        Some((at.x, at.y))
    }

    /// The element at a screen point and the app it belongs to.
    fn element_at((x, y): (f64, f64)) -> Option<(i32, CFType)> {
        // Not through wrap(): a timeout set on the system-wide element applies process-wide.
        let raw = unsafe { AXUIElementCreateSystemWide() };
        if raw.is_null() {
            return None;
        }
        let system = unsafe { CFType::wrap_under_create_rule(raw) };
        let mut out: AXUIElementRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyElementAtPosition(system.as_CFTypeRef(), x as f32, y as f32, &mut out) };
        let el = if err == 0 { wrap(out)? } else { return None };
        let mut pid = 0;
        (unsafe { AXUIElementGetPid(el.as_CFTypeRef(), &mut pid) } == 0).then_some((pid, el))
    }

    /// The text field `el` is, or sits inside (a browser reports the text run under the
    /// mouse). Gives up at `deadline`, so a hung app can't hold up a dictation.
    fn text_field_of(el: CFType, deadline: Instant) -> Option<CFType> {
        let mut cur = Some(el);
        for _ in 0..8 {
            let c = cur?;
            if editable(&c) {
                return Some(c);
            }
            if Instant::now() > deadline {
                return None;
            }
            cur = attr(&c, "AXParent");
        }
        None
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

    fn focused_element(pid: i32) -> Option<CFType> {
        app(pid).and_then(|a| attr(&a, "AXFocusedUIElement"))
    }

    /// A place text can go: an app, one of its windows, and the text field in it.
    #[derive(Clone)]
    struct Spot {
        pid: i32,
        /// None when the app couldn't name its window at key-down (still building its
        /// Accessibility tree): Silas then only checks the app.
        window: Option<(CFType, u32)>,
        /// None when the app reported no text field (a browser still building its tree, or
        /// a button had focus), so Silas never focuses a container.
        element: Option<CFType>,
        /// Where the caret sat, put back after refocusing (AppKit selects a field's whole
        /// text when it takes focus, and typing would replace it).
        selection: Option<CFType>,
    }

    impl Spot {
        /// Where the keyboard is in `pid` right now.
        fn focused_in(pid: i32) -> Option<Self> {
            let app = app(pid)?;
            let window = attr(&app, "AXFocusedWindow").and_then(|w| Some((window_id(&w)?, w)));
            let element = attr(&app, "AXFocusedUIElement").filter(editable);
            Some(Self {
                pid,
                window: window.map(|(id, w)| (w, id)),
                selection: element.as_ref().and_then(|e| attr(e, "AXSelectedTextRange")),
                element,
            })
        }

        fn has(&self, pid: i32, el: &CFType) -> bool {
            pid == self.pid && self.element.as_ref().is_some_and(|e| same(e, el))
        }

        fn wid(&self) -> Option<u32> {
            self.window.as_ref().map(|(_, id)| *id)
        }
    }

    /// Shared between the target and its click watcher.
    struct Shared {
        /// Where the text goes: the field focused at key-down, or the one the user clicked
        /// into since.
        spot: parking_lot::Mutex<Spot>,
        stop: AtomicBool,
        /// Typing started with the target field focused, so a later change of field inside
        /// the same window (a sheet, a command palette) means the keyboard left.
        field_at_start: AtomicBool,
        /// Silas typed a Tab or Return, which can move focus to another field on purpose;
        /// from then on only the window is checked.
        field_checks_off: AtomicBool,
        /// A refocus Silas couldn't confirm (the app didn't answer): the first field the app
        /// names must be the target, or the keyboard is elsewhere.
        unconfirmed: AtomicBool,
        checks: std::sync::atomic::AtomicUsize,
    }

    // AXUIElement is a CoreFoundation type that may be used from any thread.
    unsafe impl Send for Shared {}
    unsafe impl Sync for Shared {}

    /// Notes where each click happened, the moment it happens, so judging a slow click
    /// never looks at where the mouse went afterwards.
    fn sample_clicks(shared: Arc<Shared>, mut last: u32, tx: std::sync::mpsc::Sender<(f64, f64)>) {
        // A dictation's recording stops at 5 minutes; transcription follows. Stop is the real
        // exit (restore or drop); this only bounds a thread nobody stopped.
        let give_up = Instant::now() + Duration::from_secs(900);
        while !shared.stop.load(Ordering::Relaxed) && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(10));
            let now = clicks();
            if now != last {
                last = now;
                if let Some(at) = mouse_location() {
                    if tx.send(at).is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// Judges each click by what was under the mouse when it happened. Only a press on a text
    /// field, after which that app takes the keyboard there, makes it the place for the text.
    /// A press on anything else (closing or dragging a pop-up, a label, a button) changes
    /// nothing, even if it moves focus as a side effect. Editors that draw their own text
    /// (VS Code, some terminals) don't count, so the text goes back where it started.
    fn judge_clicks(shared: Arc<Shared>, rx: std::sync::mpsc::Receiver<(f64, f64)>) {
        let mut next_caret = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(at) => judge(&shared, at),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if shared.stop.load(Ordering::Relaxed) {
                        continue; // drain: the sampler hangs up once it has stopped
                    }
                    // Keep the caret position fresh while the target field has the keyboard,
                    // so a refocus puts it back where the user left it.
                    if Instant::now() >= next_caret {
                        next_caret = Instant::now() + Duration::from_millis(250);
                        let mut spot = shared.spot.lock().clone();
                        if let Some(e) = spot.element.clone() {
                            if front_pid() == Some(spot.pid) && focused_element(spot.pid).is_some_and(|f| same(&f, &e)) {
                                spot.selection = attr(&e, "AXSelectedTextRange").or(spot.selection);
                                let mut cur = shared.spot.lock();
                                if cur.has(spot.pid, &e) {
                                    cur.selection = spot.selection;
                                }
                            }
                        }
                    }
                }
                // The sampler stopped and every click it saw has been judged.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn judge(shared: &Shared, at: (f64, f64)) {
        let deadline = Instant::now() + Duration::from_millis(500);
        let Some((pid, hit)) = element_at(at) else { return };
        let Some(field) = text_field_of(hit, deadline) else { return };
        if shared.spot.lock().has(pid, &field) {
            return; // a click inside the target only moves the caret, which is sampled
        }
        // The clicked app must take the keyboard in that field: clicking OK on a dialog that
        // then closes doesn't make a background app's field the target.
        loop {
            if front_pid() == Some(pid) {
                if let Some(spot) = Spot::focused_in(pid).filter(|s| s.has(pid, &field)) {
                    *shared.spot.lock() = spot;
                    return;
                }
            }
            if Instant::now() > deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub struct Target {
        shared: Arc<Shared>,
        /// Hangs up when the click judge has finished.
        judged: parking_lot::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    }

    unsafe impl Send for Target {}

    impl Drop for Target {
        fn drop(&mut self) {
            self.shared.stop.store(true, Ordering::Relaxed);
        }
    }

    impl Target {
        pub fn capture() -> Option<Self> {
            // Without SkyLight Silas can't move focus back, so it doesn't try: it types where
            // focus is, as it did before.
            skylight()?;
            let baseline = clicks();
            let pid = super::focused()? as i32;
            // Browsers can miss the first question while they build their tree; ask again.
            let mut spot = Spot::focused_in(pid);
            for _ in 0..2 {
                if spot.as_ref().is_some_and(|s| s.window.is_some()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
                spot = Spot::focused_in(pid).or(spot);
            }
            let spot = spot.unwrap_or(Spot { pid, window: None, element: None, selection: None });
            let shared = Arc::new(Shared {
                spot: parking_lot::Mutex::new(spot),
                stop: AtomicBool::new(false),
                field_at_start: AtomicBool::new(false),
                field_checks_off: AtomicBool::new(false),
                unconfirmed: AtomicBool::new(false),
                checks: std::sync::atomic::AtomicUsize::new(0),
            });
            let (tx, rx) = std::sync::mpsc::channel();
            let sampler = shared.clone();
            std::thread::spawn(move || sample_clicks(sampler, baseline, tx));
            let judge = shared.clone();
            let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
            std::thread::spawn(move || {
                judge_clicks(judge, rx);
                drop(done_tx);
            });
            Some(Self { shared, judged: parking_lot::Mutex::new(Some(done_rx)) })
        }

        fn spot(&self) -> Spot {
            self.shared.spot.lock().clone()
        }

        pub(super) fn pid(&self) -> i32 {
            self.shared.spot.lock().pid
        }

        /// Keystrokes would land in the target window, or the app is too slow to say
        /// otherwise. Only a clear sign (another app in front, another window focused) is No.
        pub(super) fn in_place(&self) -> bool {
            let spot = self.spot();
            front_pid().is_none_or(|p| p == spot.pid)
                && spot.wid().is_none_or(|want| focused_window_id(spot.pid).is_none_or(|w| w == want))
        }

        /// Asked before every key event: the app and window every time (about 25 µs), the
        /// field every fourth time.
        pub fn still_here(&self, after_focus_key: bool) -> bool {
            if after_focus_key {
                self.shared.field_checks_off.store(true, Ordering::Relaxed);
            }
            if !self.in_place() {
                return false;
            }
            if !self.shared.checks.fetch_add(1, Ordering::Relaxed).is_multiple_of(4)
                || self.shared.field_checks_off.load(Ordering::Relaxed)
            {
                return true;
            }
            let spot = self.spot();
            let Some(e) = &spot.element else { return true };
            match focused_element(spot.pid) {
                Some(f) if same(&f, e) => {
                    self.shared.field_at_start.store(true, Ordering::Relaxed);
                    self.shared.unconfirmed.store(false, Ordering::Relaxed);
                    true
                }
                Some(_) => {
                    !self.shared.field_at_start.load(Ordering::Relaxed)
                        && !self.shared.unconfirmed.load(Ordering::Relaxed)
                }
                None => true,
            }
        }

        /// Ask the app to focus the target field if something else in it has focus, put the
        /// caret back where it was, and confirm it took. False when another field kept the
        /// keyboard (a sheet or a web dialog over the field): typing would go there.
        fn refocus_field(&self, spot: &Spot) -> bool {
            let Some(e) = &spot.element else { return true };
            let on_target = |f: &Option<CFType>| f.as_ref().map(|f| same(f, e));
            if on_target(&focused_element(spot.pid)) == Some(true) {
                self.shared.field_at_start.store(true, Ordering::Relaxed);
                return true;
            }
            set_true(e, "AXFocused");
            if let Some(range) = &spot.selection {
                unsafe {
                    AXUIElementSetAttributeValue(
                        e.as_CFTypeRef(),
                        CFString::new("AXSelectedTextRange").as_concrete_TypeRef(),
                        range.as_CFTypeRef(),
                    );
                }
            }
            let deadline = Instant::now() + Duration::from_millis(200);
            loop {
                match on_target(&focused_element(spot.pid)) {
                    Some(true) => {
                        self.shared.field_at_start.store(true, Ordering::Relaxed);
                        return true;
                    }
                    // Another field answered; give the app until the deadline to move.
                    Some(false) if Instant::now() > deadline => return false,
                    // Too slow to answer: type, but the first field it names must be this one.
                    None if Instant::now() > deadline => {
                        self.shared.unconfirmed.store(true, Ordering::Relaxed);
                        return true;
                    }
                    _ => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        }

        pub fn restore(&self) -> Restore {
            // Let the judge finish clicks it has already seen, but not wait on a hung app.
            self.shared.stop.store(true, Ordering::Relaxed);
            if let Some(done) = self.judged.lock().take() {
                let _ = done.recv_timeout(Duration::from_millis(700));
            }
            let spot = self.spot();
            if self.in_place() {
                // Same window. Don't re-activate it (that can clear a browser's own focus),
                // but a sheet or dialog inside it may have taken the field.
                if !self.refocus_field(&spot) {
                    log::warn!("another field in the window kept the keyboard");
                    return Restore::Failed;
                }
                return Restore::InPlace;
            }
            let Some((window, wid)) = spot.window.clone() else {
                // Silas never learned the window, so it can't bring it back.
                return Restore::Failed;
            };
            if let Err(AX_INVALID_ELEMENT) = attr_err(&window, "AXRole") {
                // The window is gone. A text field in another window of the same app is a fine
                // home for the text, guarded like any target; anything else would be a stranger's.
                log::info!("the window the dictation was going to is gone");
                let here = front_pid()
                    .filter(|p| *p == spot.pid)
                    .and_then(Spot::focused_in)
                    .filter(|s| s.element.is_some() && s.window.is_some());
                return match here {
                    Some(s) => {
                        *self.shared.spot.lock() = s;
                        Restore::Elsewhere
                    }
                    None => Restore::Failed,
                };
            }
            let Some(sl) = skylight() else { return Restore::Failed };
            let mut psn: Psn = [0; 8];
            if unsafe { GetProcessForPID(spot.pid, &mut psn) } != 0 {
                return Restore::Failed;
            }
            for _ in 0..2 {
                // kCPSUserGenerated lets the app install its key window before it handles keys.
                unsafe { (sl.set_front)(&psn, wid, 0x200) };
                make_key(sl, &psn, wid);
                unsafe {
                    AXUIElementPerformAction(window.as_CFTypeRef(), CFString::new("AXRaise").as_concrete_TypeRef());
                }
                let deadline = Instant::now() + Duration::from_millis(400);
                while !self.in_place() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                // A window still opening can take focus right back; wait until it stays put.
                if self.settled() {
                    break;
                }
            }
            if !self.in_place() {
                log::warn!("could not put the keyboard back where the dictation was going");
                return Restore::Failed;
            }
            // The app restores its own last field on activation; nudge it in case it didn't.
            if !self.refocus_field(&spot) {
                log::warn!("another field in the window kept the keyboard");
                return Restore::Failed;
            }
            log::info!("put the keyboard back where the dictation was going");
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
    if let Some(us) = hypr::us_layout() {
        return us;
    }
    // X11 reports the layout; other Wayland compositors don't share it, so trust the locale.
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
            assert!(target.still_here(false));
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
        assert!(target.still_here(false), "focus left again after the restore");
        assert!(target.mac.in_place());
    }
}

#[cfg(test)]
mod hypr_tests {
    use super::*;

    /// `hyprctl activewindow -j` from Hyprland 0.56.2 on Omarchy, recorded 2026-11-10 (titles
    /// replaced).
    const RECORDED: &str = include_str!("testdata/hyprctl-activewindow.json");

    fn id(json: &str) -> Option<u64> {
        hypr::parse(json).map(|w| w.id)
    }

    fn window(id: u64, overlay: bool) -> hypr::Window {
        hypr::Window { id, address: id * 16, overlay }
    }

    #[test]
    fn hyprland_names_the_window_by_address_and_class() {
        let w = hypr::parse(RECORDED).expect("a window");
        assert_eq!(w.address, 0x55ef19943100);
        assert!(!w.overlay);
        let id = w.id;
        // Same address and class: the same id, whatever else changed.
        let moved = RECORDED.replace("\"floating\": true", "\"floating\": false");
        assert_eq!(self::id(&moved), Some(id));
        // Another window, or the same address reused by another app: another id.
        assert_ne!(self::id(&RECORDED.replace("0x55ef19943100", "0x55ef19943200")), Some(id));
        assert_ne!(self::id(&RECORDED.replace("\"class\": \"org.omarchy.screensaver\"", "\"class\": \"foot\"")), Some(id));
    }

    #[test]
    fn hyprland_names_the_app_by_its_window_class() {
        assert_eq!(hypr::class(RECORDED).as_deref(), Some("org.omarchy.screensaver"));
        let kitty = RECORDED.replace("\"class\": \"org.omarchy.screensaver\"", "\"class\": \"kitty\"");
        assert_eq!(hypr::class(&kitty).as_deref(), Some("kitty"));
        assert_eq!(crate::style::known("kitty").map(|a| a.style), Some(crate::style::Style::Terminal));
        assert_eq!(hypr::class("{}"), None);
        assert_eq!(hypr::class(r#"{"class": "  "}"#), None);
    }

    #[test]
    fn windows_apps_are_named_by_their_exe() {
        assert_eq!(exe_name(r"C:\Program Files\Git\usr\bin\MinTTY.exe").as_deref(), Some("mintty.exe"));
        assert_eq!(exe_name(r"C:\Users\me\AppData\Local\Programs\cursor\Cursor.exe").as_deref(), Some("cursor.exe"));
        assert_eq!(exe_name("pwsh.exe").as_deref(), Some("pwsh.exe"));
        assert_eq!(exe_name(r"C:\dir\"), None);
    }

    #[test]
    fn the_hand_back_names_the_window_as_hyprland_printed_it() {
        let w = hypr::parse(RECORDED).expect("a window");
        assert_eq!(hypr::window_arg(w.address), "address:0x55ef19943100");
        // Lua dispatch first (Hyprland 0.53+), then the classic dispatcher.
        assert_eq!(
            hypr::focus_requests(w.address),
            [
                r#"dispatch hl.dsp.focus({ window = "address:0x55ef19943100" })"#.to_string(),
                "dispatch focuswindow address:0x55ef19943100".to_string(),
            ]
        );
    }

    #[test]
    fn an_empty_workspace_is_an_answer_and_garbage_is_not() {
        assert_eq!(hypr::answer("{}"), Some(None));
        assert_eq!(hypr::answer(""), None);
        assert_eq!(hypr::answer("Invalid"), None);
        assert_eq!(hypr::answer(RECORDED).flatten(), hypr::parse(RECORDED));
    }

    /// Review r3 on #6: with no window focused, the text went nowhere.
    #[test]
    fn an_empty_workspace_brings_the_target_back() {
        let target = window(7, false);
        let got = linux_restore_from(&target, Some(None), |address| {
            assert_eq!(address, target.address);
            Some(target)
        });
        assert_eq!(got, Restore::Restored);
        assert_eq!(linux_restore_from(&target, Some(None), |_| None), Restore::Failed);
        // Otherwise the same as `linux_restore`.
        let never = |_| -> Option<hypr::Window> { panic!("only the overlay is moved off") };
        assert_eq!(linux_restore_from(&target, None, never), Restore::Elsewhere);
        assert_eq!(linux_restore_from(&target, Some(Some(window(8, false))), never), Restore::Failed);
        assert_eq!(linux_restore_from(&target, Some(Some(target)), never), Restore::InPlace);
    }

    /// Review r1 on #6: a US layout with a dvorak variant mapped keys differently.
    #[test]
    fn only_plain_us_counts() {
        assert!(hypr::is_plain_us("us", "", 0));
        assert!(hypr::is_plain_us("us,de", ",", 0));
        assert!(!hypr::is_plain_us("us,de", "", 1));
        assert!(!hypr::is_plain_us("us", "dvorak", 0));
        assert!(!hypr::is_plain_us("us,us", ",colemak", 1));
        assert!(!hypr::is_plain_us("gb", "", 0));
        let devices = r#"{"keyboards": [
            {"name": "virtual", "layout": "gb", "variant": "", "active_layout_index": 0, "main": false},
            {"name": "at-translated-set-2-keyboard", "layout": "us,de", "variant": ",", "active_layout_index": 1, "main": true}
        ]}"#;
        assert_eq!(hypr::layout_is_us(devices), Some(false));
        assert_eq!(hypr::layout_is_us(&devices.replace("\"active_layout_index\": 1", "\"active_layout_index\": 0")), Some(true));
        assert_eq!(hypr::layout_is_us("{}"), None);
    }

    /// Reads only. `cargo test --lib focus -- --ignored` inside a Hyprland session.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn reads_focus_cheaply() {
        let w = hypr::active().expect("a focused window");
        let t = std::time::Instant::now();
        for _ in 0..50 {
            assert_eq!(hypr::active().map(|a| a.id), Some(w.id));
        }
        let each = t.elapsed() / 50;
        eprintln!("active(): {each:?} per call");
        assert!(each < std::time::Duration::from_millis(2), "{each:?}");
        assert!(hypr::us_layout().is_some());
    }

    #[test]
    fn hyprland_knows_the_overlay_by_its_title() {
        let titled = |t: &str| RECORDED.replacen("\"title\": \"(scrubbed)\"", &format!("\"title\": \"{t}\""), 1);
        assert!(hypr::parse(&titled(hypr::OVERLAY_TITLE)).expect("a window").overlay);
        // Silas's main window is a window like any other.
        assert!(!hypr::parse(&titled("Silas")).expect("a window").overlay);
        assert!(!hypr::parse(RECORDED).expect("a window").overlay);
    }

    #[test]
    fn the_overlay_title_matches_the_window_config() {
        let conf = include_str!("../../../src-tauri/tauri.conf.json");
        assert!(conf.contains(&format!("\"title\": \"{}\"", hypr::OVERLAY_TITLE)));
    }

    #[test]
    fn no_window_or_no_answer_is_none() {
        assert_eq!(id("{}"), None);
        assert_eq!(id("Invalid"), None);
        assert_eq!(id(""), None);
        assert_eq!(id(r#"{"address": "0x0", "class": "foot"}"#), None);
        assert_eq!(id(r#"{"address": "", "class": "foot"}"#), None);
    }

    #[test]
    fn the_linux_target_types_only_into_the_window_it_started_in() {
        let never = |_| -> Option<hypr::Window> { panic!("only the overlay is moved off") };
        assert_eq!(linux_restore(&window(7, false), Some(window(7, false)), never), Restore::InPlace);
        assert_eq!(linux_restore(&window(7, false), Some(window(8, false)), never), Restore::Failed);
        // Hyprland didn't answer: as before, nothing stops the typing.
        assert_eq!(linux_restore(&window(7, false), None, never), Restore::Elsewhere);
    }

    /// Review second-F4 on #23: on Hyprland the overlay can take the keyboard as it opens.
    /// Before, every dictation then saw another window, went to History and typed nothing.
    #[test]
    fn the_overlay_taking_the_keyboard_hands_it_back() {
        let target = window(7, false);
        let mut asked = None;
        let got = linux_restore(&target, Some(window(9, true)), |address| {
            asked = Some(address);
            Some(target)
        });
        assert_eq!(got, Restore::Restored);
        assert_eq!(asked, Some(target.address));
        // Hyprland refused, or something else has it after: still not safe to type.
        assert_eq!(linux_restore(&target, Some(window(9, true)), |_| None), Restore::Failed);
        assert_eq!(linux_restore(&target, Some(window(9, true)), |_| Some(window(8, false))), Restore::Failed);
    }
}
