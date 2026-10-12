//! What the OS must allow: the microphone, plus reading the hotkey and typing
//! (Accessibility on macOS, input-device access on Linux, nothing on Windows).

use serde::Serialize;

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Grant {
    Granted,
    Denied,
    Unknown,
}

#[derive(Serialize, Clone, Debug)]
pub struct Permissions {
    pub microphone: Grant,
    pub accessibility: Grant,
}

pub fn check() -> Permissions {
    Permissions {
        microphone: microphone(),
        accessibility: accessibility(),
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
    if handy_keys::check_accessibility() {
        Grant::Granted
    } else {
        Grant::Denied
    }
}

#[cfg(target_os = "linux")]
fn accessibility() -> Grant {
    // The hotkey listener reads /dev/input; the udev rule shipped in the packages grants it.
    let readable = std::fs::read_dir("/dev/input")
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
                .any(|e| std::fs::File::open(e.path()).is_ok())
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

pub fn open(kind: &str) {
    #[cfg(target_os = "macos")]
    {
        let pane = match kind {
            "microphone" => {
                // Asking for the mic the first time shows the system prompt; afterwards, the pane.
                if microphone() == Grant::Unknown {
                    std::thread::spawn(|| {
                        if let Ok(r) = crate::audio::Recording::start(None, |_| {}) {
                            std::thread::sleep(std::time::Duration::from_millis(300));
                            let _ = r.finish();
                        }
                    });
                    return;
                }
                "Privacy_Microphone"
            }
            "input" => "Privacy_ListenEvent",
            _ => {
                // Shows the system prompt with a button straight to the right pane.
                prompt_accessibility();
                "Privacy_Accessibility"
            }
        };
        let _ = std::process::Command::new("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .spawn();
    }
    #[cfg(target_os = "windows")]
    if kind == "microphone" {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "ms-settings:privacy-microphone"])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    let _ = kind;
}

/// macOS keeps an Accessibility entry tied to the build it saw first. After a re-sign the
/// switch can look on while the grant no longer applies. Clearing Silas's own entry and
/// asking again fixes it.
pub fn reset_accessibility() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("tccutil")
            .args(["reset", "Accessibility", "com.baney.silas"])
            .status();
        prompt_accessibility();
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
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
