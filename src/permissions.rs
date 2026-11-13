//! What the OS must allow: the microphone, plus reading the hotkey and typing
//! (Accessibility on macOS, input-device access on Linux, nothing on Windows).
//!
//! Reading a permission never asks for it. Asking goes through [`PermissionGate`], which
//! preflights first, never asks for something already granted, and shows each system prompt
//! at most once per process: a second click opens the Settings pane instead.
//!
//! Dictation never asks. A hotkey press records only when the microphone preflight says
//! granted ([`mic_ready_for_recording`]); an undetermined microphone is treated like a denied
//! one, so the first system prompt always comes from the onboarding Allow button, not from a
//! surprise mid-dictation.

use parking_lot::Mutex;
use serde::Serialize;

const LOG: &str = "silas.permissions";

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Grant {
    Granted,
    Denied,
    Unknown,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
pub struct Permissions {
    pub microphone: Grant,
    pub accessibility: Grant,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pane {
    Microphone,
    Accessibility,
    InputMonitoring,
}

/// The OS calls behind the gate. The `*_status` calls only read; the `request_*` calls may
/// show a system prompt.
pub trait PermissionHost {
    fn mic_status(&self) -> Grant;
    fn ax_status(&self) -> Grant;
    fn request_mic(&mut self);
    fn request_ax(&mut self);
    fn reset_ax(&mut self);
    fn open_pane(&mut self, pane: Pane);
}

pub struct PermissionGate<H> {
    host: H,
    asked_mic: bool,
    asked_ax: bool,
    last: Option<Permissions>,
}

impl<H: PermissionHost> PermissionGate<H> {
    pub const fn new(host: H) -> Self {
        Self {
            host,
            asked_mic: false,
            asked_ax: false,
            last: None,
        }
    }

    /// Preflight only. Launch, `get_state` and the onboarding poll all land here.
    pub fn check(&mut self) -> Permissions {
        let p = Permissions {
            microphone: self.host.mic_status(),
            accessibility: self.host.ax_status(),
        };
        // The UI polls every second; log only what changed.
        if self.last != Some(p) {
            log::info!(target: LOG, "preflight microphone={:?} accessibility={:?}", p.microphone, p.accessibility);
            self.last = Some(p);
        }
        p
    }

    pub fn open(&mut self, kind: &str) {
        match kind {
            "microphone" => {
                let status = self.host.mic_status();
                if status == Grant::Unknown && !self.asked_mic {
                    log::info!(target: LOG, "preflight microphone={status:?}; requesting once");
                    self.asked_mic = true;
                    self.host.request_mic();
                } else {
                    log::info!(target: LOG, "preflight microphone={status:?}; no request (asked={}), opening Settings", self.asked_mic);
                    self.host.open_pane(Pane::Microphone);
                }
            }
            "input" => {
                log::info!(target: LOG, "input monitoring: opening Settings, no request");
                self.host.open_pane(Pane::InputMonitoring);
            }
            _ => {
                let status = self.host.ax_status();
                if status != Grant::Granted && !self.asked_ax {
                    log::info!(target: LOG, "preflight accessibility={status:?}; requesting once");
                    self.asked_ax = true;
                    self.host.request_ax();
                } else {
                    log::info!(target: LOG, "preflight accessibility={status:?}; no request (asked={}), opening Settings", self.asked_ax);
                }
                self.host.open_pane(Pane::Accessibility);
            }
        }
    }

    /// The user's explicit "reset it and ask again". macOS can keep a stale entry whose
    /// switch looks on while the grant no longer applies; then the preflight says not
    /// trusted, and clearing Silas's own entry fixes it. A trusted process has nothing to
    /// fix, so it only gets the Settings pane. The system prompt still shows at most once
    /// per process: after it, a reset clears the entry and opens Settings without asking.
    pub fn reset_accessibility(&mut self) {
        let status = self.host.ax_status();
        if status == Grant::Granted {
            log::info!(target: LOG, "preflight accessibility=Granted; no reset, no request");
        } else {
            self.host.reset_ax();
            if self.asked_ax {
                log::info!(target: LOG, "preflight accessibility={status:?}; reset, no request (asked=true), opening Settings");
            } else {
                log::info!(target: LOG, "preflight accessibility={status:?}; reset, requesting once");
                self.asked_ax = true;
                self.host.request_ax();
            }
        }
        self.host.open_pane(Pane::Accessibility);
    }

    /// The home banner's Fix Permission, for a stale grant: Settings shows Silas on, but the
    /// entry holds an older build's code requirement and `AXIsProcessTrusted` says no. Turning
    /// the switch off and on keeps that requirement, so it never fixes this. Clearing Silas's
    /// own entry does, and a new process must then ask, so this returns true when the caller
    /// should relaunch Silas. A trusted process has nothing to fix and is left alone.
    pub fn fix_accessibility(&mut self) -> bool {
        let status = self.host.ax_status();
        if status == Grant::Granted {
            log::info!(target: LOG, "fix: preflight accessibility=Granted; no reset, no relaunch");
            return false;
        }
        self.host.reset_ax();
        log::info!(target: LOG, "fix: preflight accessibility={status:?}; reset, relaunching to ask once");
        true
    }

    /// The first launch after Fix Permission: ask once if still not trusted, then open the
    /// Accessibility pane so the new entry is right there.
    pub fn after_fix_relaunch(&mut self) {
        let status = self.host.ax_status();
        if status == Grant::Granted {
            log::info!(target: LOG, "after fix: preflight accessibility=Granted; no request");
            return;
        }
        if self.asked_ax {
            log::info!(target: LOG, "after fix: preflight accessibility={status:?}; no request (asked=true), opening Settings");
        } else {
            log::info!(target: LOG, "after fix: preflight accessibility={status:?}; requesting once, opening Settings");
            self.asked_ax = true;
            self.host.request_ax();
        }
        self.host.open_pane(Pane::Accessibility);
    }

    /// Preflight for a dictation recording. Never asks: only `Granted` may record.
    pub fn mic_ready_for_recording(&mut self) -> bool {
        let status = self.host.mic_status();
        if status != Grant::Granted {
            log::info!(target: LOG, "preflight microphone={status:?}; dictation not started, no request");
        }
        status == Grant::Granted
    }
}

static GATE: Mutex<PermissionGate<System>> = Mutex::new(PermissionGate::new(System));

pub fn check() -> Permissions {
    GATE.lock().check()
}

pub fn open(kind: &str) {
    GATE.lock().open(kind);
}

pub fn reset_accessibility() {
    GATE.lock().reset_accessibility();
}

/// Passed to the relaunched Silas after Fix Permission.
pub const FIX_RELAUNCH_ARG: &str = "--after-accessibility-fix";

pub fn fix_accessibility() -> bool {
    GATE.lock().fix_accessibility()
}

pub fn after_fix_relaunch() {
    GATE.lock().after_fix_relaunch();
}

pub fn mic_ready_for_recording() -> bool {
    GATE.lock().mic_ready_for_recording()
}

/// The real OS.
pub struct System;

impl PermissionHost for System {
    fn mic_status(&self) -> Grant {
        microphone()
    }

    fn ax_status(&self) -> Grant {
        accessibility()
    }

    fn request_mic(&mut self) {
        // A short recording is what makes macOS show its microphone prompt.
        #[cfg(target_os = "macos")]
        std::thread::spawn(|| {
            if let Ok(r) = crate::audio::Recording::start(None, |_| {}) {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let _ = r.finish();
            }
        });
    }

    fn request_ax(&mut self) {
        // Shows the system prompt with a button straight to the right pane.
        #[cfg(target_os = "macos")]
        prompt_accessibility();
    }

    fn reset_ax(&mut self) {
        #[cfg(target_os = "macos")]
        {
            let status = std::process::Command::new("tccutil")
                .args(["reset", "Accessibility", "com.baney.silas"])
                .status();
            log::info!(target: LOG, "tccutil reset Accessibility com.baney.silas: {status:?}");
        }
    }

    fn open_pane(&mut self, pane: Pane) {
        #[cfg(target_os = "macos")]
        {
            let anchor = match pane {
                Pane::Microphone => "Privacy_Microphone",
                Pane::Accessibility => "Privacy_Accessibility",
                Pane::InputMonitoring => "Privacy_ListenEvent",
            };
            let _ = std::process::Command::new("open")
                .arg(format!(
                    "x-apple.systempreferences:com.apple.preference.security?{anchor}"
                ))
                .spawn();
        }
        #[cfg(target_os = "windows")]
        if pane == Pane::Microphone {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "ms-settings:privacy-microphone"])
                .spawn();
        }
        #[cfg(target_os = "linux")]
        let _ = pane;
    }
}

#[cfg(target_os = "macos")]
fn microphone() -> Grant {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    use objc2_foundation::NSString;

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
        return Grant::Unknown;
    };
    let media = NSString::from_str("soun"); // AVMediaTypeAudio
    let status: isize = unsafe { msg_send![cls, authorizationStatusForMediaType: &*media] };
    match status {
        3 => Grant::Granted,
        1 | 2 => Grant::Denied,
        _ => Grant::Unknown, // not asked yet; the first recording asks
    }
}

#[cfg(not(target_os = "macos"))]
fn microphone() -> Grant {
    Grant::Granted
}

#[cfg(target_os = "macos")]
fn accessibility() -> Grant {
    // AXIsProcessTrusted: a read, never a prompt.
    if handy_keys::check_accessibility() {
        Grant::Granted
    } else {
        Grant::Denied
    }
}

#[cfg(target_os = "linux")]
fn accessibility() -> Grant {
    // The hotkey listener reads /dev/input; the udev rule shipped in the packages grants it.
    // access(2) asks the permission bits and ACLs without opening the device: opening an evdev
    // node takes 4 to 12 ms, and this runs on the main thread for every `get_state`.
    use std::os::unix::ffi::OsStrExt;
    let readable = std::fs::read_dir("/dev/input")
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.file_name().as_bytes().starts_with(b"event"))
                .any(|e| {
                    std::ffi::CString::new(e.path().as_os_str().as_bytes())
                        // SAFETY: a NUL-terminated path that lives for the call.
                        .is_ok_and(|p| unsafe { libc::access(p.as_ptr(), libc::R_OK) } == 0)
                })
        })
        .unwrap_or(false);
    if readable {
        Grant::Granted
    } else {
        Grant::Denied
    }
}

#[cfg(target_os = "windows")]
fn accessibility() -> Grant {
    Grant::Granted
}

#[cfg(target_os = "macos")]
fn prompt_accessibility() {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(
            options: core_foundation::dictionary::CFDictionaryRef,
        ) -> bool;
    }
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let opts =
        CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
    unsafe {
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check gives the answer an open would, without opening a device (see `accessibility`).
    #[cfg(target_os = "linux")]
    #[test]
    fn the_linux_key_check_matches_an_open_and_opens_nothing() {
        let opens = std::fs::read_dir("/dev/input")
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
                    .any(|e| std::fs::File::open(e.path()).is_ok())
            })
            .unwrap_or(false);
        assert_eq!(accessibility() == Grant::Granted, opens);
        let src = include_str!("permissions.rs");
        let body = &src[src.find("fn accessibility() -> Grant {\n    // The hotkey listener").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(!body.contains("File::open") && !body.contains("OpenOptions"), "the check must not open a device");
    }

    #[derive(Default)]
    struct Fake {
        mic: Option<Grant>,
        ax: Option<Grant>,
        mic_requests: usize,
        ax_requests: usize,
        resets: usize,
        panes: Vec<Pane>,
    }

    impl Fake {
        fn new(mic: Grant, ax: Grant) -> Self {
            Self {
                mic: Some(mic),
                ax: Some(ax),
                ..Default::default()
            }
        }
        fn requests(&self) -> usize {
            self.mic_requests + self.ax_requests
        }
    }

    impl PermissionHost for Fake {
        fn mic_status(&self) -> Grant {
            self.mic.unwrap()
        }
        fn ax_status(&self) -> Grant {
            self.ax.unwrap()
        }
        fn request_mic(&mut self) {
            self.mic_requests += 1;
        }
        fn request_ax(&mut self) {
            self.ax_requests += 1;
        }
        fn reset_ax(&mut self) {
            self.resets += 1;
        }
        fn open_pane(&mut self, pane: Pane) {
            self.panes.push(pane);
        }
    }

    #[test]
    fn granted_access_is_never_requested() {
        let mut g = PermissionGate::new(Fake::new(Grant::Granted, Grant::Granted));
        for _ in 0..3 {
            g.check();
            g.open("microphone");
            g.open("accessibility");
            g.reset_accessibility();
        }
        assert_eq!(g.host.requests(), 0);
        assert_eq!(g.host.resets, 0);
    }

    #[test]
    fn check_only_preflights() {
        let mut g = PermissionGate::new(Fake::new(Grant::Unknown, Grant::Denied));
        for _ in 0..10 {
            let p = g.check();
            assert_eq!(
                p,
                Permissions {
                    microphone: Grant::Unknown,
                    accessibility: Grant::Denied
                }
            );
        }
        assert_eq!(g.host.requests(), 0);
        assert!(g.host.panes.is_empty());
    }

    #[test]
    fn one_open_asks_exactly_once() {
        let mut g = PermissionGate::new(Fake::new(Grant::Unknown, Grant::Denied));
        g.open("accessibility");
        assert_eq!(g.host.ax_requests, 1);
        g.open("microphone");
        assert_eq!(g.host.mic_requests, 1);
        assert_eq!(g.host.requests(), 2);
    }

    #[test]
    fn a_second_open_while_denied_opens_settings_only() {
        let mut g = PermissionGate::new(Fake::new(Grant::Unknown, Grant::Denied));
        g.open("accessibility");
        g.open("microphone");
        g.host.panes.clear();
        g.open("accessibility");
        g.open("microphone");
        assert_eq!((g.host.ax_requests, g.host.mic_requests), (1, 1));
        assert_eq!(g.host.panes, vec![Pane::Accessibility, Pane::Microphone]);
    }

    #[test]
    fn a_denied_microphone_gets_settings_not_a_prompt() {
        let mut g = PermissionGate::new(Fake::new(Grant::Denied, Grant::Granted));
        g.open("microphone");
        assert_eq!(g.host.mic_requests, 0);
        assert_eq!(g.host.panes, vec![Pane::Microphone]);
    }

    #[test]
    fn input_monitoring_is_never_requested() {
        let mut g = PermissionGate::new(Fake::new(Grant::Unknown, Grant::Denied));
        g.open("input");
        assert_eq!(g.host.requests(), 0);
        assert_eq!(g.host.panes, vec![Pane::InputMonitoring]);
    }

    #[test]
    fn reset_after_open_clears_the_entry_without_a_second_prompt() {
        let mut g = PermissionGate::new(Fake::new(Grant::Granted, Grant::Denied));
        g.open("accessibility");
        assert_eq!(g.host.ax_requests, 1);
        g.reset_accessibility();
        assert_eq!((g.host.resets, g.host.ax_requests), (1, 1));
        g.reset_accessibility();
        g.open("accessibility");
        assert_eq!((g.host.resets, g.host.ax_requests), (2, 1));
        assert_eq!(g.host.panes, vec![Pane::Accessibility; 4]);
    }

    #[test]
    fn reset_first_asks_once_then_never_again() {
        let mut g = PermissionGate::new(Fake::new(Grant::Granted, Grant::Denied));
        g.reset_accessibility();
        assert_eq!((g.host.resets, g.host.ax_requests), (1, 1));
        g.reset_accessibility();
        assert_eq!((g.host.resets, g.host.ax_requests), (2, 1));
        g.open("accessibility");
        assert_eq!(g.host.ax_requests, 1);
    }

    #[test]
    fn fix_leaves_a_trusted_process_alone() {
        let mut g = PermissionGate::new(Fake::new(Grant::Granted, Grant::Granted));
        assert!(!g.fix_accessibility());
        g.after_fix_relaunch();
        assert_eq!((g.host.resets, g.host.requests()), (0, 0));
        assert!(g.host.panes.is_empty());
    }

    #[test]
    fn fix_resets_and_relaunches_without_asking_from_the_stale_process() {
        let mut g = PermissionGate::new(Fake::new(Grant::Granted, Grant::Denied));
        assert!(g.fix_accessibility());
        assert_eq!((g.host.resets, g.host.ax_requests), (1, 0));
        assert!(g.host.panes.is_empty());
    }

    #[test]
    fn after_fix_relaunch_asks_once_and_opens_settings() {
        let mut g = PermissionGate::new(Fake::new(Grant::Granted, Grant::Denied));
        g.after_fix_relaunch();
        assert_eq!((g.host.resets, g.host.ax_requests), (0, 1));
        assert_eq!(g.host.panes, vec![Pane::Accessibility]);
        // The one prompt is spent: neither a second call nor Allow asks again.
        g.after_fix_relaunch();
        g.open("accessibility");
        assert_eq!(g.host.ax_requests, 1);
        assert_eq!(g.host.panes, vec![Pane::Accessibility; 3]);
    }

    #[test]
    fn dictation_records_only_with_a_granted_microphone_and_never_asks() {
        for (mic, ready) in [
            (Grant::Granted, true),
            (Grant::Denied, false),
            (Grant::Unknown, false),
        ] {
            let mut g = PermissionGate::new(Fake::new(mic, Grant::Granted));
            for _ in 0..3 {
                assert_eq!(g.mic_ready_for_recording(), ready, "{mic:?}");
            }
            assert_eq!(g.host.requests(), 0);
            assert!(g.host.panes.is_empty());
            // Dictation doesn't spend the one prompt; onboarding's Allow still gets it.
            g.open("microphone");
            assert_eq!(g.host.mic_requests, usize::from(mic == Grant::Unknown));
        }
    }
}
