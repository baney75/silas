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
    Menu,
    /// The `type_again` command; the caller shows why it didn't happen.
    Command,
}

/// How long the tray menu takes to close and give the keyboard back to the app under it.
const MENU_CLOSE: Duration = Duration::from_millis(300);

const BUSY: &str = "Silas is busy with a dictation. Try again when it's done.";

/// The retype key from settings. None when it is unset. An error when it can't be a key, or
/// when it is the dictation key itself.
pub(super) fn trigger(settings: &Settings) -> Result<Option<Trigger>, String> {
    let Some(key) = settings.retype_hotkey.as_deref().map(str::trim).filter(|k| !k.is_empty()) else {
        return Ok(None);
    };
    if key == settings.hotkey {
        return Err("That key already starts dictation. Pick another key to type the last dictation again.".into());
    }
    Trigger::parse(key).map(Some).ok_or_else(|| format!("{key} is not a key Silas can use"))
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
        let result = self.claim_again();
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
        let before = self.status().0;
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
        // Back to where Silas was: Ready, or still asking for a model.
        let after = if before == Status::NeedsModel { Status::NeedsModel } else { Status::Ready };
        match typed {
            Err(e) => self.fail(e.to_string()),
            Ok(_) if stopped => {
                self.notice(Notice::Stopped, "Stopped typing.");
                self.set_status(after, None);
            }
            Ok(false) => self.fail("Another window took the keyboard, so Silas stopped typing.".into()),
            Ok(true) => self.set_status(after, None),
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
        assert!(trigger(&s).unwrap_err().contains("already starts dictation"));
        s.retype_hotkey = Some("NotAKey".into());
        assert!(trigger(&s).is_err());
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
