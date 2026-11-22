//! Type the last dictation again: a second hotkey, the tray menu and the `type_again` command.
//! The text goes out through the normal typer with the same guards as a dictation: never into a
//! password field, never into a window that took the keyboard, and Escape stops it.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{delivery, Core, Delivery, Notice, Shared, Status};
use crate::focus::{Restore, Target};
use crate::keys::Trigger;
use crate::store::Settings;
use crate::style::Style;
use crate::typer;

/// Where a "type it again" came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Via {
    /// The retype hotkey.
    Key,
    /// The tray menu item, which waits for the menu to close and hand the keyboard back.
    /// Not on Windows: there the menu leaves Silas's own window in front (see `allowed`).
    Menu,
    /// The `type_again` command; the caller shows why it didn't happen.
    Command,
}

/// How long the tray menu takes to close and give the keyboard back to the app under it.
const MENU_CLOSE: Duration = Duration::from_millis(300);

const BUSY: &str = "Silas is busy with a dictation. Try again when it's done.";

/// Said when the retype key and a dictation key are the same key.
const SAME_KEY: &str = "The dictation key and the Type it again key must be different keys.";

/// The retype key from settings. None when it is unset. An error when it can't be a key, or
/// when it is a key that starts dictation: the hotkey or its fallback (Right \u{2318} while the
/// microphone key hasn't been seen).
pub(super) fn trigger(settings: &Settings) -> Result<Option<Trigger>, String> {
    let Some(key) = settings.retype_hotkey.as_deref().map(str::trim).filter(|k| !k.is_empty()) else {
        return Ok(None);
    };
    let again = Trigger::parse(key).ok_or_else(|| format!("{key} is not a key Silas can use"))?;
    let dictation = [Some(settings.hotkey.as_str()), settings.hotkey_fallback.as_deref()];
    if dictation.into_iter().flatten().any(|k| Trigger::parse(k) == Some(again)) {
        return Err(SAME_KEY.into());
    }
    Ok(Some(again))
}

/// Windows has no way yet to find the window the user was typing in (`focus::Target` is None
/// there), and the tray menu and the Silas window both leave Silas's own window in front. So
/// there only the retype key, pressed while the user's app has the keyboard, may type.
fn allowed(via: Via, windows: bool) -> Result<(), String> {
    if windows && via != Via::Key {
        return Err("On Windows, use your Type it again key. The menu can't type into your app yet.".into());
    }
    Ok(())
}

/// Said when a password field has the keyboard as a retype starts.
pub fn secure_again_message() -> String {
    "A password field has the keyboard, so Silas didn't type your last dictation.".into()
}

impl Core {
    /// The hook in `handle_key`: a clean press and release of the retype key types the last
    /// dictation again.
    pub(super) fn again_key(self: &Shared, ev: &handy_keys::KeyEvent) {
        let fire = self.machine.lock().again.on_event(ev);
        if fire {
            let _ = self.type_again(Via::Key);
        }
    }

    /// Type the newest History item again (its taught fix, if it has one) into the field with
    /// the keyboard. Returns at once; the typing runs on its own thread. An error says why
    /// nothing will be typed; from a key or the menu it also shows as a notice.
    pub fn type_again(self: &Shared, via: Via) -> Result<(), String> {
        let result = allowed(via, cfg!(target_os = "windows")).and_then(|()| self.claim_again());
        match result {
            Ok(text) => {
                let me = self.clone();
                std::thread::spawn(move || {
                    if via == Via::Menu {
                        std::thread::sleep(MENU_CLOSE);
                    }
                    let guard = me.clone();
                    guard.survive(false, move || me.retype(&text));
                });
                Ok(())
            }
            Err(why) => {
                if via != Via::Command {
                    self.refuse(Notice::Busy, &why);
                }
                Err(why)
            }
        }
    }

    /// The text to retype, with the typing flag taken so nothing else types meanwhile.
    fn claim_again(&self) -> Result<String, String> {
        if !self.is_enabled() {
            return Err("Silas is paused. Turn it back on to type your last dictation again.".into());
        }
        let busy = matches!(
            self.status().0,
            Status::Loading | Status::Recording | Status::Transcribing | Status::Typing
        );
        if busy || self.machine.lock().is_active() {
            return Err(BUSY.into());
        }
        let text = {
            let history = self.history.lock();
            let item = history.first().ok_or("Nothing to type yet. Dictate something first.")?;
            item.corrected.clone().unwrap_or_else(|| item.text.clone())
        };
        if text.trim().is_empty() {
            return Err("Your last dictation was empty.".into());
        }
        if self.typing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_err() {
            return Err(BUSY.into());
        }
        Ok(text)
    }

    /// Runs on its own thread with `typing` already set.
    fn retype(self: &Shared, text: &str) {
        let before = self.status();
        let target = Target::capture();
        let restored = target.as_ref().map_or(Restore::Elsewhere, Target::restore);
        match delivery(crate::focus::secure_input(), restored) {
            Delivery::Type => {}
            Delivery::Drop => {
                self.typing.store(false, Ordering::Relaxed);
                self.fail(secure_again_message());
                return;
            }
            Delivery::HistoryOnly => {
                self.typing.store(false, Ordering::Relaxed);
                self.fail("Another window has the keyboard, so Silas didn't type your last dictation.".into());
                return;
            }
        }
        let s = self.settings.lock().clone();
        let style = self.note_app(target.as_ref()).map_or(Style::Prose, |a| a.style);
        let mut text = text.to_string();
        if s.trailing_space && !text.ends_with(char::is_whitespace) {
            text.push(' ');
        }
        let still_here = |after_focus_key: bool| {
            !self.stop_typing.load(Ordering::Relaxed)
                && target.as_ref().is_none_or(|t| t.still_here(after_focus_key))
        };
        self.set_status(Status::Typing, None);
        self.typed_over.store(false, Ordering::Relaxed);
        self.stop_typing.store(false, Ordering::Relaxed);
        let typed = typer::type_text(
            &text,
            typer::Pace::from_setting(&s.typing_speed),
            style.shift_enter(s.newline_as_shift_enter),
            &still_here,
        );
        // Let the last synthetic events drain before reading the keyboard again.
        std::thread::sleep(Duration::from_millis(60));
        self.typing.store(false, Ordering::Relaxed);
        let stopped = self.stop_typing.swap(false, Ordering::Relaxed);
        *self.last_activity.lock() = std::time::Instant::now();
        match typed {
            Err(e) => self.fail(e.to_string()),
            Ok(_) if stopped => {
                self.notice(Notice::Stopped, "Stopped typing.");
                self.restore_status(before);
            }
            Ok(false) => self.fail("Another window took the keyboard, so Silas stopped typing.".into()),
            Ok(true) => self.restore_status(before),
        }
    }

    /// Back to the exact status and detail Silas had before the retype: an Error stays that
    /// Error. The key-access error goes through Ready, which `set_status` turns back into it
    /// while the listener is still down. A dictation Error gets its recovery again, since one
    /// that came due during the retype saw Typing and did nothing.
    fn restore_status(self: &Shared, before: (Status, Option<String>)) {
        if before.0 == Status::Error && before.1.as_deref() == Some(self.key_message().as_str()) {
            self.set_status(Status::Ready, None);
            return;
        }
        let error = before.0 == Status::Error;
        self.set_status(before.0, before.1);
        if error && self.active_model().is_some() {
            self.recover_soon();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NullHost;
    use crate::store::HistoryItem;
    use std::sync::Arc;

    fn item(text: &str, corrected: Option<&str>) -> HistoryItem {
        HistoryItem {
            id: "1".into(),
            ts: String::new(),
            text: text.into(),
            raw: text.into(),
            duration_s: 1.0,
            latency_ms: 1,
            words: 1,
            corrected: corrected.map(Into::into),
        }
    }

    #[test]
    fn the_retype_key_must_be_a_key_and_not_the_dictation_key() {
        let mut s = Settings::default();
        assert_eq!(trigger(&s), Ok(None));
        s.retype_hotkey = Some("  ".into());
        assert_eq!(trigger(&s), Ok(None));
        s.retype_hotkey = Some("F14".into());
        assert_eq!(trigger(&s), Ok(Some(Trigger::Key(handy_keys::Key::F14))));
        s.retype_hotkey = Some(s.hotkey.clone());
        assert_eq!(trigger(&s).unwrap_err(), SAME_KEY);
        s.retype_hotkey = Some("NotAKey".into());
        assert!(trigger(&s).is_err());
    }

    #[test]
    fn the_retype_key_cannot_be_the_second_dictation_key() {
        let mut s = Settings {
            hotkey: "F13".into(),
            hotkey_fallback: Some("CmdRight".into()),
            retype_hotkey: Some("CmdRight".into()),
            ..Settings::default()
        };
        assert_eq!(trigger(&s).unwrap_err(), SAME_KEY, "Right Cmd still starts dictation");
        s.hotkey_fallback = None;
        assert_eq!(trigger(&s), Ok(Trigger::parse("CmdRight")));
    }

    #[test]
    fn windows_types_again_only_from_the_retype_key() {
        assert_eq!(allowed(Via::Key, true), Ok(()));
        assert!(allowed(Via::Menu, true).unwrap_err().contains("Windows"));
        assert!(allowed(Via::Command, true).unwrap_err().contains("Windows"));
        for via in [Via::Key, Via::Menu, Via::Command] {
            assert_eq!(allowed(via, false), Ok(()));
        }
    }

    #[test]
    fn a_retype_puts_back_the_exact_status_it_found() {
        let _turn = crate::store::TEST_DIR.lock();
        let dir = std::env::temp_dir().join(format!("silas-again-status-{}", std::process::id()));
        crate::store::set_data_dir(Some(dir.clone()));
        let core = Core::new(Arc::new(NullHost));
        for before in [
            (Status::Error, Some("Could not load the model: no file".to_string())),
            (Status::NeedsModel, None),
        ] {
            core.set_status(before.0, before.1.clone());
            core.set_status(Status::Typing, None);
            core.restore_status(before.clone());
            assert_eq!(core.status(), before);
        }
        crate::store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retyping_takes_the_newest_item_and_its_fix_and_refuses_when_busy() {
        let _turn = crate::store::TEST_DIR.lock();
        let dir = std::env::temp_dir().join(format!("silas-again-{}", std::process::id()));
        crate::store::set_data_dir(Some(dir.clone()));
        let core = Core::new(Arc::new(NullHost));
        // A new Core is Loading its model, which counts as busy.
        assert_eq!(core.claim_again().unwrap_err(), BUSY);
        core.set_status(Status::Ready, None);
        let first = core.claim_again().unwrap_err();
        assert!(first.contains("Nothing to type yet"), "{first}");
        core.history.lock().insert(0, item("first try", None));
        core.history.lock().insert(0, item("send it to Parish one", Some("send it to Parish1")));
        assert_eq!(core.claim_again().as_deref(), Ok("send it to Parish1"));
        // The typing flag is held until the retype finishes: a second one is refused.
        assert_eq!(core.claim_again().unwrap_err(), BUSY);
        core.typing.store(false, Ordering::Relaxed);
        core.set_status(Status::Transcribing, None);
        assert_eq!(core.claim_again().unwrap_err(), BUSY);
        core.set_status(Status::Ready, None);
        core.set_enabled(false);
        assert!(core.claim_again().unwrap_err().contains("paused"));
        crate::store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
