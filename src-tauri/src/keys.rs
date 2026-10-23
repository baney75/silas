//! The hold-to-talk key and the edit watcher.
//!
//! `Machine` turns raw key events into start / finish / cancel decisions.
//! `EditWatch` follows the keys the user presses right after Silas types, keeping a
//! shadow copy of the text, so a backspace-and-retype fix can be learned.

use std::time::{Duration, Instant};

use handy_keys::{Key, KeyEvent, Modifiers};

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
        /// Pressed through `silas press` (a compositor keybinding), not a key Silas reads.
        external: bool,
    },
    /// The key became part of a shortcut (Right ⌘ + C); wait for its release.
    Shortcut,
    Toggled,
    /// Pressed again to end hands-free mode; swallow the release.
    Ending,
}

pub struct Machine {
    pub trigger: Trigger,
    pub tap_to_toggle: bool,
    state: State,
}

impl Machine {
    pub fn new(trigger: Trigger, tap_to_toggle: bool) -> Self {
        Self {
            trigger,
            tap_to_toggle,
            state: State::Idle,
        }
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

    /// The hotkey went down or up where Silas can't see it: a compositor keybinding running
    /// `silas press` and `silas release`. It acts like the configured key, but other keys never
    /// turn it into a shortcut, since the compositor already took the whole chord.
    pub fn external(&mut self, down: bool, now: Instant) -> Out {
        self.on_trigger(down, now, true)
    }

    /// `silas cancel`: drop the dictation in progress.
    pub fn cancel(&mut self) -> Out {
        let active = self.is_active();
        self.state = State::Idle;
        if active { Out::Cancel } else { Out::Nothing }
    }

    fn on_trigger(&mut self, down: bool, now: Instant, external: bool) -> Out {
        match (self.state, down) {
            (State::Idle, true) => {
                self.state = State::Held { since: now, external };
                Out::Start
            }
            (State::Held { .. }, true) => Out::Nothing, // key repeat
            (State::Held { since, .. }, false) => {
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
            (State::Shortcut, false) => {
                self.state = State::Idle;
                Out::Nothing
            }
            (State::Toggled, true) => {
                self.state = State::Ending;
                Out::Finish
            }
            (State::Ending, false) => {
                self.state = State::Idle;
                Out::Nothing
            }
            _ => Out::Nothing,
        }
    }

    pub fn on_event(&mut self, ev: &KeyEvent, now: Instant) -> Out {
        if let Some(down) = self.trigger.matches(ev) {
            // A hold from a compositor binding ends with its own release, not this key's.
            if matches!(self.state, State::Held { external: true, .. }) {
                return Out::Nothing;
            }
            return self.on_trigger(down, now, false);
        }
        let is_mouse = matches!(
            ev.key,
            Some(Key::MouseLeft | Key::MouseRight | Key::MouseMiddle | Key::MouseX1 | Key::MouseX2)
        );
        let escape = ev.key == Some(Key::Escape) && ev.is_key_down;
        match self.state {
            State::Held { .. } if escape => {
                self.state = State::Shortcut;
                Out::Cancel
            }
            State::Held { external: false, .. }
                if ev.is_key_down
                    && !is_mouse
                    && (ev.key.is_some() || ev.changed_modifier.is_some()) =>
            {
                self.state = State::Shortcut;
                Out::Cancel
            }
            State::Toggled if escape => {
                self.state = State::Idle;
                Out::Cancel
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
        let mut w = EditWatch::new("ssh into Paris one ", t, None);
        for _ in 0..10 {
            assert!(matches!(
                w.on_event(&key(Key::Delete, Modifiers::empty()), t),
                Watch::Continue
            ));
        }
        for (k, s) in [
            (Key::P, true),
            (Key::A, false),
            (Key::R, false),
            (Key::I, false),
            (Key::S, false),
            (Key::H, false),
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
                assert_eq!(a, "ssh into Paris one");
                assert_eq!(b, "ssh into Parish1");
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

    #[test]
    fn a_compositor_binding_holds_and_taps_like_the_key() {
        let t0 = Instant::now();
        let mut m = Machine::new(Trigger::parse("CtrlRight").unwrap(), true);
        // Super + Shift + F23 (the Copilot key) reaches Silas's listener too; it must not
        // count as a shortcut while the binding holds the dictation.
        assert_eq!(m.external(true, t0), Out::Start);
        assert_eq!(m.on_event(&modifier(Modifiers::SHIFT_LEFT, true), t0), Out::Nothing);
        assert_eq!(m.on_event(&key(Key::F23, Modifiers::SHIFT_LEFT), t0), Out::Nothing);
        assert!(m.is_active());
        assert_eq!(m.external(true, t0), Out::Nothing); // repeat
        assert_eq!(m.external(false, t0 + Duration::from_secs(2)), Out::Finish);
        assert!(!m.is_active());

        // A tap starts hands-free; the next press ends it and its release is swallowed.
        assert_eq!(m.external(true, t0), Out::Start);
        assert_eq!(m.external(false, t0 + Duration::from_millis(100)), Out::Nothing);
        assert!(m.is_active());
        assert_eq!(m.external(true, t0 + Duration::from_secs(4)), Out::Finish);
        assert_eq!(m.external(false, t0 + Duration::from_secs(4)), Out::Nothing);
        assert!(!m.is_active());

        // Escape still cancels, and the release that follows does nothing.
        assert_eq!(m.external(true, t0), Out::Start);
        assert_eq!(m.on_event(&key(Key::Escape, Modifiers::empty()), t0), Out::Cancel);
        assert_eq!(m.external(false, t0 + Duration::from_secs(1)), Out::Nothing);
        assert!(!m.is_active());

        assert_eq!(m.external(true, t0), Out::Start);
        assert_eq!(m.cancel(), Out::Cancel);
        assert_eq!(m.cancel(), Out::Nothing);

        // The configured key going up and down during a binding's hold changes nothing.
        assert_eq!(m.external(true, t0), Out::Start);
        assert_eq!(m.on_event(&modifier(Modifiers::CTRL_RIGHT, true), t0), Out::Nothing);
        assert_eq!(m.on_event(&modifier(Modifiers::CTRL_RIGHT, false), t0 + Duration::from_secs(1)), Out::Nothing);
        assert!(m.is_active());
        assert_eq!(m.external(false, t0 + Duration::from_secs(2)), Out::Finish);
    }
}
