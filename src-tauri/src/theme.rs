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

/// A font stack goes straight into CSS, so allow only names and commas, with quoted names.
fn font(v: &Value, key: &str) -> Result<String, String> {
    let s = v.get(key).and_then(Value::as_str).ok_or_else(|| format!("{key} is missing"))?;
    let bad = s.is_empty()
        || s.len() > 300
        || s.chars().any(|c| c.is_control() || ";{}()<>\\@!".contains(c))
        || s.contains("/*");
    if bad {
        Err(format!("{key} is not a font list"))
    } else {
        Ok(s.to_string())
    }
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
    Ok(Theme {
        name: root
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| n.len() <= 48)
            .unwrap_or("")
            .to_string(),
        mode,
        light: colors(voice, "light")?,
        dark: colors(voice, "dark")?,
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

/// Read one file. Missing is quiet; a file that is there but wrong is logged.
pub fn load(path: &Path) -> Option<Theme> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return None,
    };
    if !meta.is_file() || meta.len() > MAX_BYTES {
        log::warn!("theme file ignored: {} is not a file under {MAX_BYTES} bytes", path.display());
        return None;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("theme file unreadable: {e}");
            return None;
        }
    };
    match parse(&text) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("theme file ignored: {e}");
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

/// Check the file once a second and tell every window when it changes. One stat a second.
/// Holds no lock while it emits.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Stamp = theme_path().as_deref().and_then(stamp);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let Some(path) = theme_path() else { continue };
            let now = stamp(&path);
            if changed(&last, &now) {
                last = now;
                let payload = load(&path);
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
    fn uppercase_colors_are_lowercased() {
        let mut v = good();
        v["surfaces"]["voice"]["light"]["accent"] = "#ABCDEF".into();
        assert_eq!(parse_value(&v).unwrap().light.accent, "#abcdef");
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
}
