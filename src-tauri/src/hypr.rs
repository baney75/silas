//! Hyprland's IPC socket: which window has the keyboard, putting it back, and the layout.
//! Wayland gives apps none of this, so on other compositors these all answer None.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

fn socket() -> Option<PathBuf> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime).join("hypr").join(sig).join(".socket.sock"))
}

/// Send one request and read the whole reply. A round trip takes about 0.1 ms.
fn ask(request: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket()?).ok()?;
    // Hyprland answers from its main loop; a stalled compositor must not stall typing.
    s.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    s.write_all(request.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

fn json(request: &str) -> Option<serde_json::Value> {
    serde_json::from_str(&ask(&format!("j/{request}"))?).ok()
}

pub fn running() -> bool {
    socket().is_some_and(|p| p.exists())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub address: String,
    pub pid: i64,
    /// Top-left corner and size in the global layout.
    pub at: (i64, i64),
    pub size: (i64, i64),
}

impl Window {
    /// The address as a number, for code that only compares ids.
    pub fn id(&self) -> Option<u64> {
        u64::from_str_radix(self.address.trim_start_matches("0x"), 16).ok()
    }

    pub fn contains(&self, (x, y): (i64, i64)) -> bool {
        x >= self.at.0 && y >= self.at.1 && x < self.at.0 + self.size.0 && y < self.at.1 + self.size.1
    }
}

fn pair(v: &serde_json::Value, key: &str) -> (i64, i64) {
    let a = v.get(key).and_then(|a| a.as_array());
    let n = |i: usize| a.and_then(|a| a.get(i)).and_then(|n| n.as_i64()).unwrap_or(0);
    (n(0), n(1))
}

/// The window with the keyboard. None when nothing has focus (an empty workspace).
pub fn active() -> Option<Window> {
    let v = json("activewindow")?;
    let address = v.get("address")?.as_str()?.to_string();
    let pid = v.get("pid").and_then(|p| p.as_i64()).unwrap_or(-1);
    (!address.is_empty()).then(|| Window { address, pid, at: pair(&v, "at"), size: pair(&v, "size") })
}

/// The pointer, in the same coordinates as window positions.
pub fn cursor() -> Option<(i64, i64)> {
    let v = json("cursorpos")?;
    Some((v.get("x")?.as_i64()?, v.get("y")?.as_i64()?))
}

/// The window still exists (it may be on another workspace).
pub fn exists(address: &str) -> bool {
    json("clients")
        .and_then(|v| v.as_array().cloned())
        .is_some_and(|clients| clients.iter().any(|c| c.get("address").and_then(|a| a.as_str()) == Some(address)))
}

/// Give the keyboard back to a window. Hyprland 0.53+ takes Lua dispatches; older ones the
/// classic dispatcher.
pub fn focus(address: &str) -> bool {
    let lua = format!("dispatch hl.dsp.focus({{ window = \"address:{address}\" }})");
    if ask(&lua).is_some_and(|r| r.trim() == "ok") {
        return true;
    }
    ask(&format!("dispatch focuswindow address:{address}")).is_some_and(|r| r.trim() == "ok")
}

/// The main keyboard's active layout is plain US, or None when Hyprland can't say.
pub fn us_layout() -> Option<bool> {
    let v = json("devices")?;
    let main = v.get("keyboards")?.as_array()?.iter().find(|k| k.get("main").and_then(|m| m.as_bool()) == Some(true))?;
    let i = main.get("active_layout_index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
    let field = |key: &str| main.get(key).and_then(|f| f.as_str()).unwrap_or("").to_string();
    Some(is_plain_us(&field("layout"), &field("variant"), i))
}

/// "us" with no variant at the active index: dvorak, colemak and intl map keys differently.
fn is_plain_us(layouts: &str, variants: &str, active: usize) -> bool {
    let layout = layouts.split(',').nth(active).map(str::trim);
    let variant = variants.split(',').nth(active).map(str::trim).unwrap_or("");
    layout == Some("us") && variant.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_us_counts() {
        assert!(is_plain_us("us", "", 0));
        assert!(is_plain_us("us,de", ",", 0));
        assert!(!is_plain_us("us,de", "", 1));
        assert!(!is_plain_us("us", "dvorak", 0));
        assert!(!is_plain_us("us,us", ",colemak", 1));
        assert!(!is_plain_us("gb", "", 0));
    }

    #[test]
    fn windows_contain_their_own_area() {
        let w = Window { address: "0x1".into(), pid: 1, at: (10, 20), size: (100, 50) };
        assert!(w.contains((10, 20)) && w.contains((109, 69)));
        assert!(!w.contains((110, 20)) && !w.contains((9, 30)));
    }

    /// Reads only. `cargo test --lib hypr -- --ignored` inside a Hyprland session.
    #[test]
    #[ignore]
    fn reads_focus_cheaply() {
        let w = active().expect("a focused window");
        assert!(w.id().is_some() && exists(&w.address));
        let t = std::time::Instant::now();
        for _ in 0..50 {
            assert_eq!(active().as_ref(), Some(&w));
        }
        let each = t.elapsed() / 50;
        eprintln!("active(): {each:?} per call");
        assert!(each < Duration::from_millis(2), "{each:?}");
        assert!(us_layout().is_some());
    }
}
