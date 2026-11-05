//! Polycarp theme for Silas. Reads `<config>/current/theme.json`, checks every value and hands
//! the UI only what passed. A missing or bad file means `None`, and the UI shows its own look.
//!
//! The file is written by `polycarp theme`. Only `surfaces.voice` and the top-level `mode`
//! are read here. See `docs/ipc.md` for the shape sent to the UI.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

/// Contrast the main text pairs need, as in the CLI (`TEXT_MIN`).
const MIN_CONTRAST: f64 = 4.5;

/// Biggest theme file read. A real one is about 3 KB.
const MAX_BYTES: u64 = 256 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Colors {
    pub accent: String,
    pub background: String,
    pub border: String,
    pub done: String,
    pub foreground: String,
    pub muted: String,
    pub on_accent: String,
    pub selection: String,
    pub strong: String,
    pub sunken: String,
    pub surface: String,
    pub urgent: String,
    pub working: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Shape {
    pub blur: f64,
    pub border_width: f64,
    pub radius_small: f64,
    pub radius: f64,
    pub radius_large: f64,
    pub shadow: f64,
    pub density: String,
    pub indicator: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Type {
    pub display: String,
    pub ui: String,
    pub mono: String,
    pub size: f64,
    pub weight: f64,
    pub line_height: f64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Motion {
    pub fast: f64,
    pub normal: f64,
    pub slow: f64,
    pub easing: String,
    pub scale_from: f64,
    pub reduced: bool,
}

/// What the UI gets: `mode` is `auto`, `light` or `dark`.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub mode: String,
    pub light: Colors,
    pub dark: Colors,
    pub shape: Shape,
    #[serde(rename = "type")]
    pub typ: Type,
    pub motion: Motion,
}

// ---------------------------------------------------------------------------------------------
// Where the file is
// ---------------------------------------------------------------------------------------------

/// The config folder, by the same rules as `polycarp theme`. `env` reads one variable; an empty
/// value counts as unset. A relative `POLYCARP_CONFIG_DIR` is refused (None), as the command does.
pub fn config_dir(
    env: &dyn Fn(&str) -> Option<String>,
    home: Option<PathBuf>,
    windows: bool,
) -> Option<PathBuf> {
    let get = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(d) = get("POLYCARP_CONFIG_DIR") {
        return d.is_absolute().then_some(d);
    }
    if windows {
        return get("APPDATA").map(|p| p.join("Polycarp"));
    }
    if let Some(x) = get("XDG_CONFIG_HOME").filter(|p| p.is_absolute()) {
        return Some(x.join("polycarp"));
    }
    home.map(|h| h.join(".config").join("polycarp"))
}

pub fn theme_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir);
    config_dir(&|k| std::env::var(k).ok(), home, cfg!(windows))
        .map(|d| d.join("current").join("theme.json"))
}

// ---------------------------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------------------------

fn color(v: &Value, path: &str) -> Result<String, String> {
    let s = v.get(path).and_then(Value::as_str).ok_or_else(|| format!("{path} is missing"))?;
    let b = s.as_bytes();
    if b.len() == 7 && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit) {
        Ok(s.to_ascii_lowercase())
    } else {
        Err(format!("{path} is not #rrggbb"))
    }
}

fn num(v: &Value, key: &str, lo: f64, hi: f64) -> Result<f64, String> {
    let n = v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("{key} is missing"))?;
    if n.is_finite() && (lo..=hi).contains(&n) {
        Ok(n)
    } else {
        Err(format!("{key} is {n}, needs {lo} to {hi}"))
    }
}

fn choice(v: &Value, key: &str, allowed: &[&str]) -> Result<String, String> {
    let s = v.get(key).and_then(Value::as_str).ok_or_else(|| format!("{key} is missing"))?;
    if allowed.contains(&s) {
        Ok(s.to_string())
    } else {
        Err(format!("{key} is {s:?}, needs one of {allowed:?}"))
    }
}

/// A font stack: names separated by commas. Each is plain (letters, digits, spaces, hyphens,
/// underscores) or in matching quotes with no quote, backslash, `<`, control character or `/*`
/// inside. Same rule as `check_font` in cli/src/theme.rs; the stack goes straight into CSS.
fn check_font(s: &str) -> Result<(), &'static str> {
    if s.trim().is_empty() {
        return Err("is empty");
    }
    let mut rest = s;
    loop {
        rest = rest.trim_start();
        let after = if let Some(q) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) {
            let body = &rest[1..];
            let end = body.find(q).ok_or("has a quote that is not closed")?;
            let inner = &body[..end];
            if inner.trim().is_empty() {
                return Err("has an empty quoted name");
            }
            if inner.contains("/*")
                || inner.chars().any(|c| c.is_control() || matches!(c, '\\' | '"' | '\'' | '<'))
            {
                return Err("has a quoted name with a character that is not allowed");
            }
            &body[end + 1..]
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let name = rest[..end].trim_end();
            if name.is_empty() {
                return Err("has a name missing");
            }
            if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')) {
                return Err("has a name with a character that is not allowed");
            }
            &rest[end..]
        };
        let after = after.trim_start();
        match after.strip_prefix(',') {
            Some(next) => rest = next,
            None if after.is_empty() => return Ok(()),
            None => return Err("has something after a quoted name that is not a comma"),
        }
    }
}

fn font(v: &Value, key: &str) -> Result<String, String> {
    let s = v.get(key).and_then(Value::as_str).ok_or_else(|| format!("{key} is missing"))?;
    if s.len() > 300 {
        return Err(format!("{key} is longer than 300 bytes"));
    }
    check_font(s).map_err(|why| format!("{key} {why}"))?;
    Ok(s.to_string())
}

fn easing(v: &Value) -> Result<String, String> {
    let s = v.get("easing").and_then(Value::as_str).ok_or("easing is missing")?;
    if ["ease", "ease-in", "ease-out", "ease-in-out", "linear"].contains(&s) {
        return Ok(s.to_string());
    }
    let inner = s
        .strip_prefix("cubic-bezier(")
        .and_then(|r| r.strip_suffix(')'))
        .ok_or("easing is not a known curve")?;
    let n: Vec<f64> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    let ok = n.len() == 4
        && inner.split(',').count() == 4
        && n.iter().all(|x| x.is_finite() && x.abs() <= 10.0)
        && (0.0..=1.0).contains(&n[0])
        && (0.0..=1.0).contains(&n[2]);
    if ok {
        Ok(s.to_string())
    } else {
        Err("easing is not a valid cubic-bezier".into())
    }
}

fn lin(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG contrast ratio of two `#rrggbb` colors that `color` already passed.
pub fn contrast(a: &str, b: &str) -> f64 {
    let lum = |h: &str| {
        let ch = |i: usize| lin(u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0));
        0.2126 * ch(1) + 0.7152 * ch(3) + 0.0722 * ch(5)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Contrast the large marks need on `sunken`, as in the CLI (`MARK_MIN`).
const MARK_MIN: f64 = 3.0;

/// Every pair `contrast_on` checks in cli/src/theme.rs (`pairs`), in the same order. A hand-edited
/// or corrupt file can break them, and the UI would be unreadable.
fn readable(mode: &str, c: &Colors) -> Result<(), String> {
    let get = |k: &str| -> &str {
        match k {
            "foreground" => &c.foreground,
            "strong" => &c.strong,
            "muted" => &c.muted,
            "background" => &c.background,
            "surface" => &c.surface,
            "sunken" => &c.sunken,
            "selection" => &c.selection,
            "urgent" => &c.urgent,
            "working" => &c.working,
            "done" => &c.done,
            "accent" => &c.accent,
            _ => &c.on_accent,
        }
    };
    let mut pairs: Vec<(&str, &str, f64)> = Vec::new();
    for fg in ["foreground", "strong", "muted"] {
        for bg in ["background", "surface", "sunken"] {
            pairs.push((fg, bg, MIN_CONTRAST));
        }
    }
    for fg in ["foreground", "strong"] {
        pairs.push((fg, "selection", MIN_CONTRAST));
    }
    for fg in ["urgent", "working", "done", "accent"] {
        for bg in ["background", "surface"] {
            pairs.push((fg, bg, MIN_CONTRAST));
        }
    }
    pairs.push(("on_accent", "accent", MIN_CONTRAST));
    for fg in ["urgent", "working", "done", "accent"] {
        pairs.push((fg, "sunken", MARK_MIN));
    }
    for (fa, fb, min) in pairs {
        let r = contrast(get(fa), get(fb));
        if r < min {
            return Err(format!("{mode}.{fa} on {mode}.{fb} is {r:.2}, needs {min:?}"));
        }
    }
    Ok(())
}

fn section<'a>(v: &'a Value, key: &str) -> Result<&'a Value, String> {
    v.get(key).filter(|x| x.is_object()).ok_or_else(|| format!("{key} is missing"))
}

fn colors(v: &Value, mode: &str) -> Result<Colors, String> {
    let c = section(v, mode)?;
    let g = |k: &str| color(c, k).map_err(|e| format!("{mode}.{e}"));
    Ok(Colors {
        accent: g("accent")?,
        background: g("background")?,
        border: g("border")?,
        done: g("done")?,
        foreground: g("foreground")?,
        muted: g("muted")?,
        on_accent: g("on_accent")?,
        selection: g("selection")?,
        strong: g("strong")?,
        sunken: g("sunken")?,
        surface: g("surface")?,
        urgent: g("urgent")?,
        working: g("working")?,
    })
}

/// Parse and check the text of `theme.json`. Any problem is one message.
pub fn parse(text: &str) -> Result<Theme, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let mode = choice(&root, "mode", &["auto", "light", "dark"])?;
    let voice = root
        .get("surfaces")
        .and_then(|s| s.get("voice"))
        .filter(|x| x.is_object())
        .ok_or("surfaces.voice is missing")?;
    let shape = section(voice, "shape")?;
    let ty = section(voice, "type")?;
    let mo = section(voice, "motion")?;
    let light = colors(voice, "light")?;
    let dark = colors(voice, "dark")?;
    readable("light", &light)?;
    readable("dark", &dark)?;
    Ok(Theme {
        name: root
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| n.len() <= 48)
            .unwrap_or("")
            .to_string(),
        mode,
        light,
        dark,
        shape: Shape {
            blur: num(shape, "blur", 0.0, 80.0)?,
            border_width: num(shape, "border_width", 0.0, 8.0)?,
            radius_small: num(shape, "radius_small", 0.0, 64.0)?,
            radius: num(shape, "radius", 0.0, 64.0)?,
            radius_large: num(shape, "radius_large", 0.0, 64.0)?,
            shadow: num(shape, "shadow", 0.0, 1.0)?,
            density: choice(shape, "density", &["compact", "regular", "spacious"])?,
            indicator: choice(shape, "indicator", &["dot", "bar", "number-block"])?,
        },
        typ: Type {
            display: font(ty, "display")?,
            ui: font(ty, "ui")?,
            mono: font(ty, "mono")?,
            size: num(ty, "size", 10.0, 24.0)?,
            weight: num(ty, "weight", 300.0, 800.0)?,
            line_height: num(ty, "line_height", 1.0, 2.0)?,
        },
        motion: Motion {
            fast: num(mo, "fast", 0.0, 1000.0)?,
            normal: num(mo, "normal", 0.0, 1000.0)?,
            slow: num(mo, "slow", 0.0, 1000.0)?,
            easing: easing(mo)?,
            scale_from: num(mo, "scale_from", 0.5, 1.0)?,
            reduced: mo.get("reduced").and_then(Value::as_bool).ok_or("reduced is missing")?,
        },
    })
}

/// Read at most `MAX_BYTES` from `r`. It reads one byte more to tell "exactly full" from "too big",
/// and refuses before the bytes are decoded.
fn read_capped<R: std::io::Read>(r: R) -> Result<String, String> {
    use std::io::Read;
    let mut buf = Vec::new();
    r.take(MAX_BYTES + 1).read_to_end(&mut buf).map_err(|e| format!("unreadable: {e}"))?;
    if buf.len() as u64 > MAX_BYTES {
        return Err(format!("over {MAX_BYTES} bytes"));
    }
    String::from_utf8(buf).map_err(|_| "not UTF-8".to_string())
}

/// `O_NONBLOCK` by platform, so no libc crate is needed: opening a FIFO that has no writer then
/// returns at once instead of waiting for one.
#[cfg(any(target_os = "linux", target_os = "android"))]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
const O_NONBLOCK: i32 = 0x4; // macOS and the BSDs

/// Open `path` for reading and return it only if the handle is a regular file. It never blocks:
/// a FIFO, even one swapped in after a check by path, is refused rather than waited on.
fn open_regular(path: &Path) -> Result<std::fs::File, String> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(O_NONBLOCK);
    }
    let file = opts.open(path).map_err(|e| format!("unreadable: {e}"))?;
    match file.metadata() {
        Ok(m) if m.is_file() && m.len() <= MAX_BYTES => Ok(file),
        Ok(_) => Err(format!("not a file under {MAX_BYTES} bytes")),
        Err(e) => Err(format!("unreadable: {e}")),
    }
}

/// Read one file. Missing is quiet; a file that is there but wrong is logged. A path that is not a
/// regular file (a FIFO, a folder) is refused before it is opened. The file is then opened without
/// blocking and checked through that handle, so it cannot be swapped between the check and the read.
pub fn load(path: &Path) -> Option<Theme> {
    let text = match std::fs::metadata(path) {
        Err(_) => return None,
        Ok(m) if !m.is_file() => Err("not a regular file".to_string()),
        Ok(_) => open_regular(path).and_then(read_capped),
    };
    match text.and_then(|t| parse(&t)) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("theme file ignored: {} {e}", path.display());
            None
        }
    }
}

pub fn current() -> Option<Theme> {
    theme_path().and_then(|p| load(&p))
}

// ---------------------------------------------------------------------------------------------
// Watching
// ---------------------------------------------------------------------------------------------

/// What one stat says about the file: modified time and size, or None when it is not there.
pub type Stamp = Option<(SystemTime, u64)>;

pub fn stamp(path: &Path) -> Stamp {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// True when the file appeared, disappeared or was rewritten since `last`.
pub fn changed(last: &Stamp, now: &Stamp) -> bool {
    last != now
}

/// One look at the file. Stat, read, stat again. `None` means nothing to report yet: either it is
/// the same as `last`, or it changed while being read, so `last` stays and the next tick tries
/// again. A change seen only half-way never sticks.
pub fn poll(
    last: &Stamp,
    mut stat: impl FnMut() -> Stamp,
    read: impl FnOnce() -> Option<Theme>,
) -> Option<(Stamp, Option<Theme>)> {
    let before = stat();
    if !changed(last, &before) {
        return None;
    }
    let theme = read();
    let after = stat();
    if changed(&before, &after) {
        return None;
    }
    Some((after, theme))
}

/// Check the file once a second and tell every window when it changes. One stat a second.
/// Runs on its own thread, so a stalled disk never reaches the window. Holds no lock while it emits.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Stamp = theme_path().as_deref().and_then(stamp);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let Some(path) = theme_path() else { continue };
            if let Some((now, payload)) = poll(&last, || stamp(&path), || load(&path)) {
                last = now;
                let _ = app.emit("theme_changed", payload);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of<'a>(m: &'a HashMap<&str, &str>) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| m.get(k).map(|v| v.to_string())
    }

    fn colors_json() -> Value {
        serde_json::json!({
            "accent": "#3f6b6a", "background": "#f1f5f5", "border": "#cbd8d9", "done": "#2f6b55",
            "foreground": "#233033", "muted": "#51636a", "on_accent": "#f1f5f5",
            "selection": "#d3e4e2", "strong": "#142022", "sunken": "#e3ebeb",
            "surface": "#fafdfd", "urgent": "#a33a2a", "working": "#80651f"
        })
    }

    fn good() -> Value {
        serde_json::json!({
            "name": "still-water", "mode": "auto",
            "surfaces": { "voice": {
                "light": colors_json(), "dark": colors_json(),
                "shape": { "blur": 0, "border_width": 1, "density": "regular", "indicator": "dot",
                           "radius": 10, "radius_large": 16, "radius_small": 6, "shadow": 0.12 },
                "type": { "display": "Georgia, serif", "ui": "system-ui, \"Segoe UI\", sans-serif",
                          "mono": "Menlo, monospace", "size": 16, "weight": 400, "line_height": 1.5 },
                "motion": { "easing": "cubic-bezier(0.2, 0, 0, 1)", "fast": 120, "normal": 200,
                            "slow": 320, "reduced": false, "scale_from": 0.98 }
            }}
        })
    }

    fn parse_value(v: &Value) -> Result<Theme, String> {
        parse(&v.to_string())
    }

    #[test]
    fn path_prefers_polycarp_config_dir() {
        let m = HashMap::from([("POLYCARP_CONFIG_DIR", "/tmp/pc"), ("XDG_CONFIG_HOME", "/x")]);
        assert_eq!(config_dir(&env_of(&m), Some("/h".into()), false), Some("/tmp/pc".into()));
    }

    #[test]
    fn relative_config_dir_is_refused() {
        let m = HashMap::from([("POLYCARP_CONFIG_DIR", "rel/dir"), ("XDG_CONFIG_HOME", "/x")]);
        assert_eq!(config_dir(&env_of(&m), Some("/h".into()), false), None);
    }

    #[test]
    fn path_uses_xdg_only_when_absolute() {
        let m = HashMap::from([("XDG_CONFIG_HOME", "/x")]);
        assert_eq!(config_dir(&env_of(&m), Some("/h".into()), false), Some("/x/polycarp".into()));
        let m = HashMap::from([("XDG_CONFIG_HOME", "rel")]);
        assert_eq!(
            config_dir(&env_of(&m), Some("/h".into()), false),
            Some("/h/.config/polycarp".into())
        );
    }

    #[test]
    fn empty_values_count_as_unset() {
        let m = HashMap::from([("POLYCARP_CONFIG_DIR", ""), ("XDG_CONFIG_HOME", "")]);
        assert_eq!(
            config_dir(&env_of(&m), Some("/h".into()), false),
            Some("/h/.config/polycarp".into())
        );
        assert_eq!(config_dir(&env_of(&m), None, false), None);
    }

    #[test]
    fn windows_uses_appdata_and_ignores_xdg() {
        let m = HashMap::from([("APPDATA", "/a"), ("XDG_CONFIG_HOME", "/x")]);
        assert_eq!(config_dir(&env_of(&m), Some("/h".into()), true), Some("/a/Polycarp".into()));
        let m = HashMap::new();
        assert_eq!(config_dir(&env_of(&m), Some("/h".into()), true), None);
    }

    #[test]
    fn good_file_parses() {
        let t = parse_value(&good()).unwrap();
        assert_eq!(t.mode, "auto");
        assert_eq!(t.light.accent, "#3f6b6a");
        assert_eq!(t.shape.radius_large, 16.0);
        assert!(!t.motion.reduced);
    }

    /// Hands out 0xff forever and counts what was taken.
    struct Endless(std::rc::Rc<std::cell::Cell<u64>>);
    impl std::io::Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            buf.fill(0xff);
            self.0.set(self.0.get() + buf.len() as u64);
            Ok(buf.len())
        }
    }

    #[test]
    fn oversized_content_is_refused_before_decoding() {
        let n = std::rc::Rc::new(std::cell::Cell::new(0));
        let e = read_capped(Endless(n.clone())).unwrap_err();
        assert!(e.starts_with("over "), "size comes first, not a UTF-8 error: {e}");
        assert!(n.get() <= MAX_BYTES + 8192, "read {} bytes", n.get());
        // Exactly full is allowed; one more is not.
        assert_eq!(read_capped(vec![b' '; MAX_BYTES as usize].as_slice()).unwrap().len() as u64, MAX_BYTES);
        assert!(read_capped(vec![b' '; MAX_BYTES as usize + 1].as_slice()).is_err());
        assert_eq!(read_capped(&[0xffu8][..]).unwrap_err(), "not UTF-8");
    }

    #[test]
    fn oversized_file_is_none() {
        let dir = std::env::temp_dir().join(format!("silas-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("theme.json");
        let good = good().to_string();
        let mut padded = good.clone().into_bytes();
        padded.resize(MAX_BYTES as usize + 1, b' ');
        std::fs::write(&f, &padded).unwrap();
        assert!(load(&f).is_none());
        padded.truncate(MAX_BYTES as usize);
        std::fs::write(&f, &padded).unwrap();
        assert!(load(&f).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn mkfifo(path: &Path) {
        let ok = std::process::Command::new("mkfifo").arg(path).status().expect("mkfifo").success();
        assert!(ok, "mkfifo");
    }

    /// Run `f` on a thread and fail, not hang, if it has not returned in 5 seconds.
    #[cfg(unix)]
    fn within_5s<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(5)).expect("blocked on a FIFO")
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_at_the_theme_path_is_refused_without_blocking() {
        let dir = std::env::temp_dir().join(format!("silas-fifo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("theme.json");
        mkfifo(&f);
        // The whole load, and the open alone (as if the FIFO appeared after the check by path).
        let p = f.clone();
        assert!(within_5s(move || load(&p)).is_none());
        let p = f.clone();
        assert!(within_5s(move || open_regular(&p)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_none() {
        assert!(load(Path::new("/nonexistent/polycarp/current/theme.json")).is_none());
    }

    #[test]
    fn corrupt_and_odd_files_are_none() {
        let dir = std::env::temp_dir().join(format!("silas-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("theme.json");
        for text in ["", "{", "[]", "null", "{\"mode\":\"auto\"}", "\u{0}\u{1}"] {
            std::fs::write(&f, text).unwrap();
            assert!(load(&f).is_none(), "{text:?}");
        }
        std::fs::write(&f, good().to_string()).unwrap();
        assert!(load(&f).is_some());
        // A folder where the file should be.
        assert!(load(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_colors_are_refused() {
        for bad in ["red", "#fff", "#12345g", "#1234567", "rgb(1,2,3)", "url(x)", "#ff0000; x", ""] {
            let mut v = good();
            v["surfaces"]["voice"]["dark"]["accent"] = bad.into();
            assert!(parse_value(&v).is_err(), "{bad:?}");
        }
        let mut v = good();
        v["surfaces"]["voice"]["light"]["border"] = 5.into();
        assert!(parse_value(&v).is_err());
        let mut v = good();
        v["surfaces"]["voice"]["light"]["#"] = "x".into(); // extra keys are ignored
        assert!(parse_value(&v).is_ok());
    }

    #[test]
    fn unreadable_pairs_are_refused_in_both_modes() {
        let pairs = [
            ("foreground", "background"),
            ("foreground", "surface"),
            ("muted", "background"),
            ("muted", "surface"),
            ("on_accent", "accent"),
        ];
        for mode in ["light", "dark"] {
            for (fg, bg) in pairs {
                let mut v = good();
                let c = &mut v["surfaces"]["voice"][mode];
                c[fg] = c[bg].clone(); // contrast 1
                // surface is close to background, so an earlier pair may be the one named.
                let e = parse_value(&v).unwrap_err();
                assert!(e.starts_with(&format!("{mode}.")) && e.contains("needs 4.5"), "{fg} {bg}: {e}");
            }
        }
        assert!((contrast("#000000", "#ffffff") - 21.0).abs() < 1e-9);
    }

    #[test]
    fn every_cli_pair_is_checked() {
        // The reviewer's case: five pairs pass (white on black), the rest are black on black.
        for mode in ["light", "dark"] {
            let mut v = good();
            let c = &mut v["surfaces"]["voice"][mode];
            for k in ["foreground", "muted", "sunken", "accent"] {
                c[k] = "#000000".into();
            }
            for k in ["background", "surface", "on_accent"] {
                c[k] = "#ffffff".into();
            }
            let e = parse_value(&v).unwrap_err();
            assert!(e.starts_with(&format!("{mode}.")), "{e}");
        }
        // Each pair alone, in both modes: set fg equal to bg.
        let all = [
            ("strong", "sunken"), ("muted", "sunken"), ("strong", "background"), ("foreground", "sunken"),
            ("foreground", "selection"), ("strong", "selection"), ("urgent", "background"),
            ("working", "surface"), ("done", "background"), ("accent", "surface"), ("urgent", "sunken"),
            ("working", "sunken"), ("done", "sunken"), ("accent", "sunken"),
        ];
        for mode in ["light", "dark"] {
            for (fg, bg) in all {
                let mut v = good();
                let c = &mut v["surfaces"]["voice"][mode];
                c[fg] = c[bg].clone();
                let e = parse_value(&v).unwrap_err();
                assert!(e.starts_with(&format!("{mode}.{fg} on ")), "{fg} {bg}: {e}");
            }
        }
    }

    #[test]
    fn corrupt_contrast_loads_as_none() {
        let dir = std::env::temp_dir().join(format!("silas-contrast-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("theme.json");
        let mut v = good();
        v["surfaces"]["voice"]["dark"]["foreground"] = v["surfaces"]["voice"]["dark"]["background"].clone();
        std::fs::write(&f, v.to_string()).unwrap();
        assert!(load(&f).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uppercase_colors_are_lowercased() {
        let mut v = good();
        v["surfaces"]["voice"]["light"]["accent"] = "#3F6B6A".into();
        assert_eq!(parse_value(&v).unwrap().light.accent, "#3f6b6a");
    }

    #[test]
    fn numbers_out_of_range_are_refused() {
        let cases: [(&str, &str, Value); 12] = [
            ("shape", "radius", 65.into()),
            ("shape", "radius_small", (-1).into()),
            ("shape", "shadow", 1.5.into()),
            ("shape", "blur", 81.into()),
            ("shape", "border_width", 9.into()),
            ("shape", "radius", "10".into()),
            ("type", "size", 9.into()),
            ("type", "weight", 900.into()),
            ("type", "line_height", 2.5.into()),
            ("motion", "fast", 1001.into()),
            ("motion", "scale_from", 0.4.into()),
            ("motion", "slow", Value::Null),
        ];
        for (sec, key, bad) in cases {
            let mut v = good();
            v["surfaces"]["voice"][sec][key] = bad.clone();
            assert!(parse_value(&v).is_err(), "{sec}.{key} = {bad}");
        }
    }

    #[test]
    fn choices_and_easing_are_checked() {
        let mut v = good();
        v["mode"] = "sepia".into();
        assert!(parse_value(&v).is_err());
        let mut v = good();
        v["surfaces"]["voice"]["shape"]["density"] = "tight".into();
        assert!(parse_value(&v).is_err());
        for bad in ["cubic-bezier(2, 0, 0, 1)", "cubic-bezier(0, 0, 0)", "steps(4)", "url(x)", "ease; x"] {
            let mut v = good();
            v["surfaces"]["voice"]["motion"]["easing"] = bad.into();
            assert!(parse_value(&v).is_err(), "{bad}");
        }
        let mut v = good();
        v["surfaces"]["voice"]["motion"]["easing"] = "ease-out".into();
        assert!(parse_value(&v).is_ok());
    }

    #[test]
    fn font_lists_cannot_carry_css() {
        for bad in ["a; color: red", "url(x)", "a } b", "a /* b", "", "a\nb", "a<b"] {
            let mut v = good();
            v["surfaces"]["voice"]["type"]["ui"] = bad.into();
            assert!(parse_value(&v).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn font_rule_matches_the_cli() {
        for ok in [
            "Georgia, serif",
            "system-ui, \"Segoe UI\", sans-serif",
            "'Times New Roman', serif",
            "\"a;b\", serif",
            "Arial",
        ] {
            assert!(check_font(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            "", " ", "a, ,b", "a,", ",a", "\"\"", "\" \"", "a;b", "url(x)", "a(b)", "a{b}", "a/b",
            "Arial, \"x/*y\"", "Arial /* c */", "\"a\\b\"", "\"a<b\"", "\"a'b\"", "'a\"b'", "\"a\nb\"",
            "a\tb", "a\u{7}", "\"a\" b", "\"open", "a\"b", "a<b", "a@b", "a!b",
        ] {
            assert!(check_font(bad).is_err(), "{bad:?}");
        }
        let mut v = good();
        v["surfaces"]["voice"]["type"]["mono"] = " ".into();
        assert!(parse_value(&v).unwrap_err().starts_with("mono "));
    }

    #[test]
    fn no_voice_surface_is_refused() {
        let mut v = good();
        v["surfaces"] = serde_json::json!({ "spaces": {} });
        assert!(parse_value(&v).is_err());
    }

    #[test]
    fn change_detection() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let t1 = t0 + Duration::from_secs(1);
        let a: Stamp = Some((t0, 10));
        assert!(!changed(&None, &None));
        assert!(!changed(&a, &a));
        assert!(changed(&None, &a), "appeared");
        assert!(changed(&a, &None), "disappeared");
        assert!(changed(&a, &Some((t1, 10))), "newer time");
        assert!(changed(&a, &Some((t0, 11))), "same time, new size");
    }

    #[test]
    fn a_change_seen_half_way_is_retried() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let (s1, s2) = (Some((t, 10)), Some((t + Duration::from_secs(1), 12)));
        let theme = parse_value(&good()).ok();
        // Changed while it was read: report nothing and keep `last`.
        let mut seq = vec![s2.clone(), s1.clone()].into_iter();
        assert!(poll(&None, || seq.next().unwrap(), || theme.clone()).is_none());
        // The retry finds it still, so it is reported.
        let mut seq = vec![s2.clone(), s2.clone()].into_iter();
        let (stamp, payload) = poll(&None, || seq.next().unwrap(), || theme.clone()).unwrap();
        assert_eq!(stamp, s2);
        assert!(payload.is_some());
        // Same as last: no read at all.
        assert!(poll(&s2, || s2.clone(), || panic!("must not read")).is_none());
        // Deleted: reported as None.
        let (stamp, payload) = poll(&s2, || None, || None).unwrap();
        assert_eq!(stamp, None);
        assert!(payload.is_none());
    }

    #[test]
    fn stamp_follows_the_file() {
        let dir = std::env::temp_dir().join(format!("silas-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("theme.json");
        let _ = std::fs::remove_file(&f);
        let none = stamp(&f);
        assert!(none.is_none());
        std::fs::write(&f, "a").unwrap();
        let one = stamp(&f);
        assert!(changed(&none, &one));
        std::fs::write(&f, "ab").unwrap();
        assert!(changed(&one, &stamp(&f)));
        std::fs::remove_file(&f).unwrap();
        assert!(changed(&stamp(&dir.join("x")), &one));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Run the real `polycarp theme` command in a throwaway config folder.
    fn polycarp(dir: &Path, args: &[&str]) -> String {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cli/Cargo.toml");
        let mut cmd = match std::env::var_os("POLYCARP_BIN") {
            Some(bin) => std::process::Command::new(bin),
            None => {
                let mut c = std::process::Command::new(
                    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()),
                );
                c.args(["run", "-q", "--manifest-path"]).arg(manifest).arg("--");
                c
            }
        };
        let out = cmd
            .args(args)
            .env("POLYCARP_CONFIG_DIR", dir)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("run polycarp");
        assert!(out.status.success(), "polycarp {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn every_built_in_theme_from_the_real_command_loads() {
        let dir = std::env::temp_dir().join(format!("silas-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let list: Value = serde_json::from_str(&polycarp(&dir, &["theme", "list", "--json"])).unwrap();
        let names: Vec<String> = list["data"]["themes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["source"] == "built-in")
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.len() >= 6, "expected the built-in themes, got {names:?}");
        for name in &names {
            for mode in ["auto", "light", "dark"] {
                polycarp(&dir, &["theme", "set", name, "--mode", mode]);
                let file = dir.join("current").join("theme.json");
                let text = std::fs::read_to_string(&file).unwrap();
                let t = parse(&text).unwrap_or_else(|e| panic!("{name} {mode}: {e}"));
                assert_eq!(t.mode, mode, "{name}");
                assert!(load(&file).is_some(), "{name} {mode}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
