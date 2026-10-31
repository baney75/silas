//! Settings, history and dictionary on disk, as plain JSON in the app data folder.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("Silas");
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

pub fn default_hotkey() -> &'static str {
    // Right ⌘ on a Mac. Elsewhere the right Windows/Super key opens the Start menu or
    // launcher on release, so Right Ctrl is the safer hold key.
    if cfg!(target_os = "macos") {
        "CmdRight"
    } else {
        "CtrlRight"
    }
}

pub fn hotkey_label(hotkey: &str) -> String {
    let mac = cfg!(target_os = "macos");
    match hotkey {
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
