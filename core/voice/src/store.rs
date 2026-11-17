//! Settings, history and dictionary on disk, as plain JSON in the app data folder.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

static DATA_DIR: parking_lot::RwLock<Option<PathBuf>> = parking_lot::RwLock::new(None);

/// Keep all data under `dir` instead of the default per-user folder. Call before `Core::new`.
/// Tests and the native Mac app use this; the Tauri app keeps the default.
pub fn set_data_dir(dir: Option<PathBuf>) {
    *DATA_DIR.write() = dir;
}

/// The data folder is one global, so tests that set it take turns. Otherwise one test clearing it
/// could send another's writes into the real settings folder.
#[cfg(test)]
pub(crate) static TEST_DIR: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub fn data_dir() -> PathBuf {
    let dir = match DATA_DIR.read().clone() {
        Some(d) => d,
        None => dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Silas"),
    };
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn models_dir() -> PathBuf {
    let dir = data_dir().join("models");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    let path = data_dir().join(name);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| match serde_json::from_str(&s) {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!("{name} is unreadable ({e}); keeping a copy and starting fresh");
                let _ = fs::copy(&path, path.with_extension("broken.json"));
                None
            }
        })
        .unwrap_or_default()
}

/// Like `load`, for a struct whose fields each have a default. One field with a bad value (a word
/// where a switch goes, a `null`) falls back to its default and every other field is kept, so a
/// hand edit or an older build's file never sends someone back through setup. A copy of the file
/// as it was is kept beside it, as `load` does.
pub fn load_fields<T: DeserializeOwned + Serialize + Default>(name: &str) -> T {
    let path = data_dir().join(name);
    let Ok(text) = fs::read_to_string(&path) else {
        return T::default();
    };
    let Ok(serde_json::Value::Object(saved)) = serde_json::from_str::<serde_json::Value>(&text) else {
        return load(name);
    };
    let Ok(serde_json::Value::Object(mut kept)) = serde_json::to_value(T::default()) else {
        return load(name);
    };
    let mut dropped = Vec::new();
    for (key, value) in saved {
        let mut trial = kept.clone();
        trial.insert(key.clone(), value);
        if serde_json::from_value::<T>(serde_json::Value::Object(trial.clone())).is_ok() {
            kept = trial;
        } else {
            dropped.push(key);
        }
    }
    if !dropped.is_empty() {
        log::warn!("{name}: {} had a bad value; using the default for it and keeping a copy", dropped.join(", "));
        let _ = fs::copy(&path, path.with_extension("broken.json"));
    }
    serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default()
}

/// Write via a temp file and rename, so a crash never leaves half a file.
pub fn save<T: Serialize>(name: &str, value: &T) {
    let path = data_dir().join(name);
    let tmp = path.with_extension("tmp");
    match serde_json::to_vec_pretty(value) {
        Ok(bytes) => {
            if fs::write(&tmp, bytes).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
        Err(e) => log::error!("could not save {name}: {e}"),
    }
}

/// The file's text, or "" when it can't be read.
pub fn read_raw(name: &str) -> String {
    fs::read_to_string(data_dir().join(name)).unwrap_or_default()
}

pub fn exists(name: &str) -> bool {
    Path::new(&data_dir().join(name)).exists()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hotkey: String,
    /// Which default `hotkey` was last settled against. See `migrate_hotkey`.
    pub hotkey_version: u32,
    /// A second key that also works until the microphone key has been seen once. No code says whether a keyboard has
    /// that key (a Mac mini with a third-party keyboard doesn't), so nobody is left unable to dictate.
    pub hotkey_fallback: Option<String>,
    pub tap_to_toggle: bool,
    pub model: String,
    /// Who chose `model`: default | registry | benchmark | user. A user choice is never overridden.
    pub model_source: String,
    pub language: String,
    pub remove_fillers: bool,
    pub trailing_space: bool,
    pub typing_speed: String,
    pub newline_as_shift_enter: bool,
    pub learn_from_edits: bool,
    pub sounds: bool,
    pub show_overlay: bool,
    pub launch_at_login: bool,
    pub microphone: Option<String>,
    pub onboarded: bool,
    pub update_channel: String,
    pub auto_update: bool,
}

/// Settings saved before per-device picks have no `modelSource`. A model other than the
/// default was the user's own choice then, so mark it so no benchmark or registry moves it.
/// So was keeping a second model installed after setup: someone who downloaded more than one
/// model chose among them.
pub fn migrate_model_source(settings: &mut Settings, raw: &str, installed_models: usize) {
    let has_source = serde_json::from_str::<serde_json::Value>(raw)
        .map(|v| v.get("modelSource").is_some())
        .unwrap_or(true);
    if !has_source {
        let chosen = settings.model != Settings::default().model || (settings.onboarded && installed_models > 1);
        settings.model_source = if chosen { "user" } else { "default" }.into();
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey().into(),
            hotkey_version: HOTKEY_VERSION,
            hotkey_fallback: default_fallback().map(String::from),
            tap_to_toggle: true,
            model: "parakeet-v3".into(),
            model_source: "default".into(),
            language: "en".into(),
            remove_fillers: true,
            trailing_space: true,
            typing_speed: "fast".into(),
            newline_as_shift_enter: true,
            learn_from_edits: true,
            sounds: true,
            show_overlay: true,
            launch_at_login: true,
            microphone: None,
            onboarded: false,
            update_channel: if cfg!(target_os = "macos") {
                "stable"
            } else {
                "nightly"
            }
            .into(),
            auto_update: true,
        }
    }
}

impl Settings {
    /// Only macOS has a tested stable channel; everyone else follows nightly.
    pub fn effective_channel(&self) -> &str {
        if cfg!(target_os = "macos") && self.update_channel == "stable" {
            "stable"
        } else {
            "nightly"
        }
    }
}

/// Bumped when the default key changes, so a saved old default can move once and a key picked
/// afterwards stays picked.
pub const HOTKEY_VERSION: u32 = 2;

/// Settings saved before version 2 hold Right ⌘ whether or not anyone chose it, since it was the
/// default on a Mac and the key was never recorded as a choice. Where the microphone key is now the
/// default they move to it once (Donovan, 2026-11-06), with Right ⌘ still working until the new key
/// is pressed. Any other key was picked by hand and stays, and so does Right Super on Windows and
/// Linux, where it was never the default.
pub fn migrate_hotkey(settings: &mut Settings, raw: &str) {
    let has_version = serde_json::from_str::<serde_json::Value>(raw)
        .map(|v| v.get("hotkeyVersion").is_some())
        .unwrap_or(true);
    if !has_version {
        let moves = settings.hotkey == "CmdRight" && default_hotkey() == "Dictation";
        settings.hotkey_fallback = moves.then(|| "CmdRight".to_string());
        if moves {
            settings.hotkey = default_hotkey().into();
        }
        settings.hotkey_version = HOTKEY_VERSION;
    }
}

/// What keeps working beside the default key on a new install.
pub fn default_fallback() -> Option<&'static str> {
    (default_hotkey() == "Dictation").then_some("CmdRight")
}

pub fn default_hotkey() -> &'static str {
    // The microphone key on an Apple silicon Mac: the top row's key with the microphone on it (keycode 0xB0, not the F5 function), or Globe if it is set to
    // start dictation. Intel Macs mostly lack it, so they keep Right ⌘. Elsewhere the right
    // Windows/Super key opens the Start menu or launcher on release, so Right Ctrl is the safer
    // hold key.
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "Dictation"
    } else if cfg!(target_os = "macos") {
        "CmdRight"
    } else {
        "CtrlRight"
    }
}

pub fn hotkey_label(hotkey: &str) -> String {
    let mac = cfg!(target_os = "macos");
    match hotkey {
        "Dictation" => "the microphone key".into(),
        "CmdRight" if mac => "Right ⌘".into(),
        "CmdRight" => "Right Super".into(),
        "CmdLeft" if mac => "Left ⌘".into(),
        "CtrlRight" => "Right Ctrl".into(),
        "CtrlLeft" => "Left Ctrl".into(),
        "OptRight" if mac => "Right ⌥".into(),
        "OptRight" => "Right Alt".into(),
        "OptLeft" if mac => "Left ⌥".into(),
        "ShiftRight" => "Right Shift".into(),
        "Fn" => "fn".into(),
        other => other.into(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub ts: String,
    pub text: String,
    pub raw: String,
    pub duration_s: f32,
    pub latency_ms: u64,
    pub words: usize,
    #[serde(default)]
    pub corrected: Option<String>,
}

pub const HISTORY_KEEP: usize = 1000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub words_total: u64,
    pub words_today: u64,
    pub dictations: u64,
    pub avg_wpm: f32,
    pub minutes_saved: f32,
    pub learned: u64,
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    #[test]
    fn a_saved_old_default_moves_only_where_the_microphone_key_is_the_default() {
        let raw = r#"{"hotkey":"CmdRight","onboarded":true}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_hotkey(&mut s, raw);
        assert_eq!(s.hotkey_version, HOTKEY_VERSION);
        if default_hotkey() == "Dictation" {
            assert_eq!(s.hotkey, "Dictation");
            assert_eq!(s.hotkey_fallback.as_deref(), Some("CmdRight"), "nobody loses dictation");
        } else {
            assert_eq!(s.hotkey, "CmdRight", "a key picked by hand here, since it was never the default");
            assert_eq!(s.hotkey_fallback, None);
        }
    }

    #[test]
    fn a_key_picked_by_hand_stays_with_no_second_key() {
        let raw = r#"{"hotkey":"F13","onboarded":true}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_hotkey(&mut s, raw);
        assert_eq!(s.hotkey, "F13");
        assert_eq!(s.hotkey_fallback, None, "Right ⌘ must not start working for someone who chose F13");
        assert_eq!(s.hotkey_version, HOTKEY_VERSION);
    }

    #[test]
    fn right_command_picked_after_the_move_stays() {
        let raw = r#"{"hotkey":"CmdRight","hotkeyVersion":2,"hotkeyFallback":null}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_hotkey(&mut s, raw);
        assert_eq!(s.hotkey, "CmdRight");
    }

    #[test]
    fn one_bad_value_keeps_every_other_setting() {
        let dir = std::env::temp_dir().join(format!("pc-fields-{}", std::process::id()));
        let _store = TEST_DIR.lock();
        set_data_dir(Some(dir.clone()));
        fs::write(
            data_dir().join("settings.json"),
            r#"{"onboarded":true,"hotkey":"F13","sounds":"yes","microphone":7,"language":null,"model":"cohere-int4"}"#,
        )
        .unwrap();
        let s: Settings = load_fields("settings.json");
        assert!(s.onboarded, "a bad switch must not send anyone back through setup");
        assert_eq!((s.hotkey.as_str(), s.model.as_str()), ("F13", "cohere-int4"));
        let d = Settings::default();
        assert_eq!((s.sounds, &s.microphone, &s.language), (d.sounds, &d.microphone, &d.language));
        assert!(exists("settings.broken.json"), "the file as it was is kept");
        fs::write(data_dir().join("settings.json"), "not json").unwrap();
        assert_eq!(load_fields::<Settings>("settings.json"), d, "a file that isn't JSON starts fresh, as before");
        set_data_dir(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_microphone_key_parses_and_has_a_label() {
        assert!(crate::keys::Trigger::parse("Dictation").is_some());
        assert_eq!(hotkey_label("Dictation"), "the microphone key");
    }

    #[test]
    fn old_settings_with_a_chosen_model_become_user() {
        let raw = r#"{"model":"cohere-int4","onboarded":true}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_model_source(&mut s, raw, 1);
        assert_eq!(s.model_source, "user");
    }

    #[test]
    fn old_settings_on_the_default_stay_default() {
        let raw = r#"{"model":"parakeet-v3","onboarded":true}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_model_source(&mut s, raw, 1);
        assert_eq!(s.model_source, "default");
    }

    #[test]
    fn old_settings_with_two_models_installed_become_user() {
        let raw = r#"{"model":"parakeet-v3","onboarded":true}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_model_source(&mut s, raw, 2);
        assert_eq!(s.model_source, "user");
        // Not yet onboarded: nothing was chosen.
        let raw = r#"{"model":"parakeet-v3","onboarded":false}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_model_source(&mut s, raw, 2);
        assert_eq!(s.model_source, "default");
    }

    #[test]
    fn new_settings_keep_their_source() {
        let raw = r#"{"model":"cohere-int4","modelSource":"benchmark"}"#;
        let mut s: Settings = serde_json::from_str(raw).unwrap();
        migrate_model_source(&mut s, raw, 2);
        assert_eq!(s.model_source, "benchmark");
    }
}
