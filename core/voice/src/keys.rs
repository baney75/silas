//! The hold-to-talk key and the edit watcher.
//!
//! `Machine` turns raw key events into start / finish / cancel decisions.
//! `EditWatch` follows the keys the user presses right after Silas types, keeping a
//! shadow copy of the text, so a backspace-and-retype fix can be learned.

use std::time::{Duration, Instant};

use handy_keys::{Hotkey, Key, KeyEvent, Modifiers};

/// Holding shorter than this is a tap.
pub const TAP: Duration = Duration::from_millis(280);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Trigger {
    Modifier(Modifiers),
    Key(Key),
}

impl Trigger {
    pub fn parse(s: &str) -> Option<Self> {
        if let Ok(m) = s.parse::<Modifiers>() {
            if !m.is_empty() {
                return Some(Trigger::Modifier(m));
            }
        }
        s.parse::<Key>().ok().map(Trigger::Key)
    }

    /// Some(true) = pressed, Some(false) = released, None = not this key.
    fn matches(&self, ev: &KeyEvent) -> Option<bool> {
        match self {
            Trigger::Modifier(m) => match ev.changed_modifier {
                Some(c) if c.intersects(*m) && ev.key.is_none() => Some(ev.is_key_down),
                _ => None,
            },
            Trigger::Key(k) => (ev.key == Some(*k)).then_some(ev.is_key_down),
        }
    }
}

/// Describe a key event the way settings store hotkeys ("CmdRight", "F13").
pub fn describe(ev: &KeyEvent) -> Option<String> {
    if !ev.is_key_down {
        return None;
    }
    if let (None, Some(m)) = (ev.key, ev.changed_modifier) {
        return Some(m.to_string());
    }
    ev.key
        .filter(|k| !matches!(k, Key::MouseLeft | Key::MouseRight))
        .map(|k| k.to_string())
}

/// What a key pressed while picking the dictation key does.
#[derive(Debug, Clone, PartialEq)]
pub enum Picked {
    /// This key, by its settings name.
    Key(String),
    /// Escape: stop picking and keep the key there was.
    Cancelled,
    /// A key that types text or edits it can't start dictation; why, in a sentence.
    Refused(String),
}

impl Picked {
    /// The shape `Core::capture` carries.
    pub fn into_result(self) -> Result<String, String> {
        match self {
            Picked::Key(k) => Ok(k),
            Picked::Cancelled => Err(crate::core::CANCELLED.to_string()),
            Picked::Refused(why) => Err(why),
        }
    }
}

/// Decide a key pressed while picking the dictation key. None: not a press to act on (a release,
/// a mouse button). Letters, digits, Space, Return, Tab and Delete are refused: holding one to
/// dictate would also type it, and every app needs them. So are punctuation keys, for the same
/// reason.
pub fn pick(ev: &KeyEvent) -> Option<Picked> {
    if ev.is_key_down && ev.key == Some(Key::Escape) {
        return Some(Picked::Cancelled);
    }
    let name = describe(ev)?;
    let typing = ev.key.and_then(|k| match k {
        Key::Space => Some("Space".to_string()),
        Key::Return | Key::KeypadEnter => Some("Return".to_string()),
        Key::Tab => Some("Tab".to_string()),
        Key::Delete | Key::ForwardDelete => Some("Delete".to_string()),
        // Letters, digits and punctuation: whatever the edit watcher reads as a character.
        _ => char_for(k, false).map(|c| format!("The {} key", c.to_ascii_uppercase())),
    });
    Some(match typing {
        Some(what) => Picked::Refused(format!(
            "{what} types text, so it can't be the dictation key. Try Right \u{2318}, Right \u{2325} or F13."
        )),
        None => Picked::Key(name),
    })
}

/// The combinations to swallow so the system does not act on the same press. Only the microphone
/// key: pressed for Polycarp, it must not also start Apple's dictation. Every other trigger keeps
/// reaching apps, so Right ⌘ + C still copies. Hardware presses carry fn in their flags; a Globe
/// key or a remapped keyboard may not. The listener matches modifiers exactly, and a press with
/// Shift or ⌘ held still starts Silas, so every combination is listed.
pub fn blocked_hotkeys(trigger: &Trigger) -> Vec<Hotkey> {
    if *trigger != Trigger::Key(Key::Dictation) {
        return Vec::new();
    }
    let groups = [Modifiers::CMD, Modifiers::SHIFT, Modifiers::CTRL, Modifiers::OPT];
    let mut out = Vec::new();
    for mask in 0..(1u8 << groups.len()) {
        let mut held = Modifiers::empty();
        for (i, group) in groups.iter().enumerate() {
            if mask & (1 << i) != 0 {
                held |= *group;
            }
        }
        for mods in [held, held | Modifiers::FN] {
            out.extend(Hotkey::new(mods, Key::Dictation));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Out {
    Nothing,
    Start,
    Finish,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    Held {
        since: Instant,
    },
    /// The key became part of a shortcut (Right ⌘ + C); wait for its release.
    Shortcut,
    Toggled,
    /// Pressed again to end hands-free mode; swallow the release.
    Ending,
}

pub struct Machine {
    pub trigger: Trigger,
    /// A second key that does the same, until the microphone key is first seen (`Settings::hotkey_fallback`).
    pub fallback: Option<Trigger>,
    pub tap_to_toggle: bool,
    state: State,
}

impl Machine {
    pub fn new(trigger: Trigger, tap_to_toggle: bool) -> Self {
        Self {
            trigger,
            fallback: None,
            tap_to_toggle,
            state: State::Idle,
        }
    }

    /// A press (not a release, not a repeat check) of the dictation key or its second key.
    pub fn is_trigger_press(&self, ev: &KeyEvent) -> bool {
        self.trigger.matches(ev).or_else(|| self.fallback.and_then(|f| f.matches(ev))) == Some(true)
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, State::Held { .. } | State::Toggled)
    }

    /// Started from the UI button: behave like a tap.
    pub fn toggle_from_ui(&mut self) -> Out {
        match self.state {
            State::Idle => {
                self.state = State::Toggled;
                Out::Start
            }
            State::Toggled | State::Held { .. } => {
                self.state = State::Idle;
                Out::Finish
            }
            _ => Out::Nothing,
        }
    }

    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    pub fn on_event(&mut self, ev: &KeyEvent, now: Instant) -> Out {
        let mine = self.trigger.matches(ev).or_else(|| self.fallback.and_then(|f| f.matches(ev)));
        let is_mouse = matches!(
            ev.key,
            Some(Key::MouseLeft | Key::MouseRight | Key::MouseMiddle | Key::MouseX1 | Key::MouseX2)
        );
        let escape = ev.key == Some(Key::Escape) && ev.is_key_down;
        match (self.state, mine) {
            (State::Idle, Some(true)) => {
                self.state = State::Held { since: now };
                Out::Start
            }
            (State::Held { .. }, Some(true)) => Out::Nothing, // key repeat
            (State::Held { since }, Some(false)) => {
                if now.duration_since(since) < TAP {
                    if self.tap_to_toggle {
                        self.state = State::Toggled;
                        Out::Nothing
                    } else {
                        self.state = State::Idle;
                        Out::Cancel
                    }
                } else {
                    self.state = State::Idle;
                    Out::Finish
                }
            }
            (State::Held { .. }, None) if escape => {
                self.state = State::Shortcut;
                Out::Cancel
            }
            (State::Held { .. }, None)
                if ev.is_key_down
                    && !is_mouse
                    && (ev.key.is_some() || ev.changed_modifier.is_some()) =>
            {
                self.state = State::Shortcut;
                Out::Cancel
            }
            (State::Shortcut, Some(false)) => {
                self.state = State::Idle;
                Out::Nothing
            }
            (State::Toggled, Some(true)) => {
                self.state = State::Ending;
                Out::Finish
            }
            (State::Toggled, None) if escape => {
                self.state = State::Idle;
                Out::Cancel
            }
            (State::Ending, Some(false)) => {
                self.state = State::Idle;
                Out::Nothing
            }
            _ => Out::Nothing,
        }
    }
}

// ------------------------------------------------------------------ edits

/// Stop watching after this long without a key.
pub const WATCH_IDLE: Duration = Duration::from_secs(12);

pub struct EditWatch {
    pub original: String,
    /// The app that had focus when Silas typed (see `focus::focused`).
    pub focus: Option<u64>,
    buffer: String,
    last: Instant,
    born: Instant,
}

/// Fixes happen right away; anything later is new writing, not a correction.
pub const WATCH_MAX: Duration = Duration::from_secs(60);

pub enum Watch {
    Continue,
    /// Finished. Holds (typed, edited) when the user changed something we could follow.
    Done(Option<(String, String)>),
}

impl EditWatch {
    pub fn new(typed: &str, now: Instant, focus: Option<u64>) -> Self {
        Self {
            original: typed.to_string(),
            focus,
            buffer: typed.to_string(),
            last: now,
            born: now,
        }
    }

    fn done(&self) -> Watch {
        let (a, b) = (self.original.trim_end(), self.buffer.trim_end());
        if a != b && !b.is_empty() {
            Watch::Done(Some((a.to_string(), b.to_string())))
        } else {
            Watch::Done(None)
        }
    }

    pub fn idle(&self, now: Instant) -> Option<Watch> {
        (now.duration_since(self.last) > WATCH_IDLE || now.duration_since(self.born) > WATCH_MAX).then(|| self.done())
    }

    pub fn on_event(&mut self, ev: &KeyEvent, now: Instant) -> Watch {
        if !ev.is_key_down {
            return Watch::Continue;
        }
        let Some(key) = ev.key else {
            return Watch::Continue; // a modifier alone
        };
        if now.duration_since(self.born) > WATCH_MAX {
            return self.done();
        }
        self.last = now;
        let m = ev.modifiers;
        let mac = cfg!(target_os = "macos");
        let word_mod = if mac { Modifiers::OPT } else { Modifiers::CTRL };
        let shortcut_mod = Modifiers::CMD | Modifiers::CTRL;

        if key == Key::Delete {
            if m.intersects(word_mod) {
                let trimmed = self.buffer.trim_end().len();
                self.buffer.truncate(trimmed);
                while let Some(c) = self.buffer.chars().last() {
                    if c.is_whitespace() {
                        break;
                    }
                    self.buffer.pop();
                }
            } else if m.intersects(shortcut_mod) {
                return Watch::Done(None); // delete to line start: can't follow
            } else {
                self.buffer.pop();
            }
            if self.buffer.is_empty() {
                return Watch::Done(None); // the whole dictation is gone
            }
            return Watch::Continue;
        }
        if m.intersects(shortcut_mod) || (mac && m.intersects(Modifiers::OPT)) {
            return self.done(); // a shortcut, or a special character we can't predict
        }
        match char_for(key, m.intersects(Modifiers::SHIFT)) {
            Some(c) => {
                self.buffer.push(c);
                Watch::Continue
            }
            None => self.done(), // Return, arrows, clicks, Escape, Tab: the edit is over
        }
    }
}

/// US layout. Other layouts still learn from fixes made in Silas's History.
fn char_for(key: Key, shift: bool) -> Option<char> {
    use Key::*;
    let letter = |c: char| Some(if shift { c.to_ascii_uppercase() } else { c });
    let pair = |plain: char, shifted: char| Some(if shift { shifted } else { plain });
    match key {
        A => letter('a'),
        B => letter('b'),
        C => letter('c'),
        D => letter('d'),
        E => letter('e'),
        F => letter('f'),
        G => letter('g'),
        H => letter('h'),
        I => letter('i'),
        J => letter('j'),
        K => letter('k'),
        L => letter('l'),
        M => letter('m'),
        N => letter('n'),
        O => letter('o'),
        P => letter('p'),
        Q => letter('q'),
        R => letter('r'),
        S => letter('s'),
        T => letter('t'),
        U => letter('u'),
        V => letter('v'),
        W => letter('w'),
        X => letter('x'),
        Y => letter('y'),
        Z => letter('z'),
        Num0 => pair('0', ')'),
        Num1 => pair('1', '!'),
        Num2 => pair('2', '@'),
        Num3 => pair('3', '#'),
        Num4 => pair('4', '$'),
        Num5 => pair('5', '%'),
        Num6 => pair('6', '^'),
        Num7 => pair('7', '&'),
        Num8 => pair('8', '*'),
        Num9 => pair('9', '('),
        Space => Some(' '),
        Minus => pair('-', '_'),
        Equal => pair('=', '+'),
        LeftBracket => pair('[', '{'),
        RightBracket => pair(']', '}'),
        Backslash => pair('\\', '|'),
        Semicolon => pair(';', ':'),
        Quote => pair('\'', '"'),
        Comma => pair(',', '<'),
        Period => pair('.', '>'),
        Slash => pair('/', '?'),
        Grave => pair('`', '~'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modifier(m: Modifiers, down: bool) -> KeyEvent {
        KeyEvent {
            modifiers: if down { m } else { Modifiers::empty() },
            key: None,
            is_key_down: down,
            changed_modifier: Some(m),
        }
    }
    fn key(k: Key, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            modifiers: mods,
            key: Some(k),
            is_key_down: true,
            changed_modifier: None,
        }
    }

    #[test]
    fn the_microphone_key_holds_and_taps_like_any_other_key() {
        let t0 = Instant::now();
        let mut m = Machine::new(Trigger::parse("Dictation").unwrap(), true);
        // Hardware presses carry fn in their flags.
        let mut press = key(Key::Dictation, Modifiers::FN);
        press.is_key_down = true;
        let mut release = press;
        release.is_key_down = false;
        assert_eq!(m.on_event(&press, t0), Out::Start);
        assert_eq!(m.on_event(&press, t0 + Duration::from_millis(50)), Out::Nothing, "key repeat");
        assert_eq!(m.on_event(&release, t0 + Duration::from_secs(2)), Out::Finish);
        // A tap starts hands-free, and the next press finishes it.
        assert_eq!(m.on_event(&press, t0 + Duration::from_secs(3)), Out::Start);
        m.on_event(&release, t0 + Duration::from_millis(3100));
        assert!(m.is_active());
        assert_eq!(m.on_event(&press, t0 + Duration::from_secs(9)), Out::Finish);
        // Holding that press sends repeats (seen on a real Mac, 2026-11-06); they start nothing.
        assert_eq!(m.on_event(&press, t0 + Duration::from_millis(9100)), Out::Nothing);
        m.on_event(&release, t0 + Duration::from_millis(9200));
        assert!(!m.is_active());
    }

    #[test]
    fn the_second_key_works_beside_the_microphone_key() {
        let t0 = Instant::now();
        let mut m = Machine::new(Trigger::parse("Dictation").unwrap(), true);
        assert_eq!(m.on_event(&modifier(Modifiers::CMD_RIGHT, true), t0), Out::Nothing);
        m.fallback = Trigger::parse("CmdRight");
        assert_eq!(m.on_event(&modifier(Modifiers::CMD_RIGHT, true), t0), Out::Start);
        assert_eq!(m.on_event(&modifier(Modifiers::CMD_RIGHT, false), t0 + Duration::from_secs(2)), Out::Finish);
    }

    #[test]
    fn only_the_microphone_key_is_ever_swallowed() {
        let blocked = blocked_hotkeys(&Trigger::parse("Dictation").unwrap());
        assert!(blocked.iter().any(|h| h.modifiers == Modifiers::FN && h.key == Some(Key::Dictation)));
        assert!(blocked.iter().any(|h| h.modifiers.is_empty() && h.key == Some(Key::Dictation)));
        // A press with Shift held starts Silas too, so Apple's dictation must not start beside it.
        for held in [Modifiers::SHIFT_LEFT | Modifiers::FN, Modifiers::CMD_RIGHT, Modifiers::OPT_LEFT | Modifiers::CTRL_RIGHT | Modifiers::FN] {
            assert!(blocked.iter().any(|h| h.modifiers.matches(held)), "{held:?}");
        }
        for other in ["CmdRight", "OptRight", "Fn", "F13"] {
            assert!(blocked_hotkeys(&Trigger::parse(other).unwrap()).is_empty(), "{other}");
        }
    }

    #[test]
    fn picking_a_key_refuses_typing_keys_and_escape_cancels() {
        assert_eq!(pick(&key(Key::Escape, Modifiers::empty())), Some(Picked::Cancelled));
        for (k, what) in [
            (Key::A, "The A key"),
            (Key::Z, "The Z key"),
            (Key::Num1, "The 1 key"),
            (Key::Space, "Space"),
            (Key::Return, "Return"),
            (Key::Tab, "Tab"),
            (Key::Delete, "Delete"),
            (Key::Comma, "The , key"),
        ] {
            match pick(&key(k, Modifiers::empty())) {
                Some(Picked::Refused(m)) => assert!(m.starts_with(what) && m.contains("can't be the dictation key"), "{m}"),
                other => panic!("{k:?}: {other:?}"),
            }
        }
        assert_eq!(pick(&modifier(Modifiers::CMD_RIGHT, true)), Some(Picked::Key("CmdRight".into())));
        assert_eq!(pick(&key(Key::F13, Modifiers::empty())), Some(Picked::Key("F13".into())));
        assert_eq!(pick(&key(Key::Dictation, Modifiers::FN)), Some(Picked::Key("Dictation".into())));
        // A release or a mouse button waits for a real press.
        assert_eq!(pick(&modifier(Modifiers::CMD_RIGHT, false)), None);
        assert_eq!(pick(&key(Key::MouseLeft, Modifiers::empty())), None);
    }

    #[test]
    fn escape_ends_hands_free_and_a_held_shortcut_but_never_starts_anything() {
        let t0 = Instant::now();
        let mut m = Machine::new(Trigger::parse("CmdRight").unwrap(), true);
        let esc = key(Key::Escape, Modifiers::empty());
        assert_eq!(m.on_event(&esc, t0), Out::Nothing, "idle: Escape is the app's");
        // Held: Escape cancels, and the release that follows does nothing.
        m.on_event(&modifier(Modifiers::CMD_RIGHT, true), t0);
        assert_eq!(m.on_event(&esc, t0 + Duration::from_millis(500)), Out::Cancel);
        assert_eq!(m.on_event(&modifier(Modifiers::CMD_RIGHT, false), t0 + Duration::from_secs(1)), Out::Nothing);
        assert!(!m.is_active());
        // Hands-free: Escape cancels it.
        m.on_event(&modifier(Modifiers::CMD_RIGHT, true), t0);
        m.on_event(&modifier(Modifiers::CMD_RIGHT, false), t0 + Duration::from_millis(100));
        assert!(m.is_active());
        assert_eq!(m.on_event(&esc, t0 + Duration::from_secs(3)), Out::Cancel);
        assert!(!m.is_active());
        // Escape's release changes nothing.
        let mut up = esc;
        up.is_key_down = false;
        assert_eq!(m.on_event(&up, t0 + Duration::from_secs(4)), Out::Nothing);
    }

    #[test]
    fn is_trigger_press_sees_only_the_press() {
        let mut m = Machine::new(Trigger::parse("F13").unwrap(), true);
        assert!(m.is_trigger_press(&key(Key::F13, Modifiers::empty())));
        let mut up = key(Key::F13, Modifiers::empty());
        up.is_key_down = false;
        assert!(!m.is_trigger_press(&up));
        assert!(!m.is_trigger_press(&key(Key::A, Modifiers::empty())));
        m.fallback = Trigger::parse("CmdRight");
        assert!(m.is_trigger_press(&modifier(Modifiers::CMD_RIGHT, true)));
    }

    #[test]
    fn hold_speaks_and_tap_toggles() {
        let t0 = Instant::now();
        let mut m = Machine::new(Trigger::parse("CmdRight").unwrap(), true);
        assert_eq!(
            m.on_event(&modifier(Modifiers::CMD_RIGHT, true), t0),
            Out::Start
        );
        assert_eq!(
            m.on_event(
                &modifier(Modifiers::CMD_RIGHT, false),
                t0 + Duration::from_secs(2)
            ),
            Out::Finish
        );

        assert_eq!(
            m.on_event(&modifier(Modifiers::CMD_RIGHT, true), t0),
            Out::Start
        );
        assert_eq!(
            m.on_event(
                &modifier(Modifiers::CMD_RIGHT, false),
                t0 + Duration::from_millis(100)
            ),
            Out::Nothing
        );
        assert!(m.is_active());
        assert_eq!(
            m.on_event(
                &modifier(Modifiers::CMD_RIGHT, true),
                t0 + Duration::from_secs(5)
            ),
            Out::Finish
        );
        assert_eq!(
            m.on_event(
                &modifier(Modifiers::CMD_RIGHT, false),
                t0 + Duration::from_secs(5)
            ),
            Out::Nothing
        );
        assert!(!m.is_active());
    }

    #[test]
    fn shortcuts_and_other_side_do_not_dictate() {
        let t0 = Instant::now();
        let mut m = Machine::new(Trigger::parse("CmdRight").unwrap(), true);
        assert_eq!(
            m.on_event(&modifier(Modifiers::CMD_LEFT, true), t0),
            Out::Nothing
        );
        assert_eq!(
            m.on_event(&modifier(Modifiers::CMD_RIGHT, true), t0),
            Out::Start
        );
        assert_eq!(
            m.on_event(&key(Key::C, Modifiers::CMD_RIGHT), t0),
            Out::Cancel
        );
        assert_eq!(
            m.on_event(
                &modifier(Modifiers::CMD_RIGHT, false),
                t0 + Duration::from_secs(1)
            ),
            Out::Nothing
        );
        assert!(!m.is_active());
    }

    #[test]
    fn watch_follows_backspace_fixes() {
        let t = Instant::now();
        let mut w = EditWatch::new("ssh into Argon one ", t, None);
        for _ in 0..10 {
            assert!(matches!(
                w.on_event(&key(Key::Delete, Modifiers::empty()), t),
                Watch::Continue
            ));
        }
        for (k, s) in [
            (Key::A, true),
            (Key::R, false),
            (Key::G, false),
            (Key::O, false),
            (Key::N, false),
            (Key::Num1, false),
        ] {
            w.on_event(
                &key(
                    k,
                    if s {
                        Modifiers::SHIFT_LEFT
                    } else {
                        Modifiers::empty()
                    },
                ),
                t,
            );
        }
        match w.on_event(&key(Key::Return, Modifiers::empty()), t) {
            Watch::Done(Some((a, b))) => {
                assert_eq!(a, "ssh into Argon one");
                assert_eq!(b, "ssh into Argon1");
            }
            _ => panic!("expected a finished edit"),
        }
    }

    #[test]
    fn watch_word_delete() {
        let t = Instant::now();
        let mut w = EditWatch::new("hello clot code", t, None);
        let word = if cfg!(target_os = "macos") {
            Modifiers::OPT_LEFT
        } else {
            Modifiers::CTRL_LEFT
        };
        w.on_event(&key(Key::Delete, word), t);
        w.on_event(&key(Key::Delete, word), t);
        for k in [Key::C, Key::L, Key::A, Key::U, Key::D, Key::E] {
            w.on_event(&key(k, Modifiers::empty()), t);
        }
        match w.idle(t + Duration::from_secs(20)) {
            Some(Watch::Done(Some((_, b)))) => assert_eq!(b, "hello claude"),
            _ => panic!("expected a finished edit"),
        }
    }
}
