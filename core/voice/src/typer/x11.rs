//! X11 keystrokes through XTest, written straight to the server.
//!
//! enigo typed about 108 characters a second here, not the pace's 700. For every key it listed the
//! input devices and waited on a round trip, and it had the server hold each press and release at
//! least 1 ms; once a capital or symbol came back, every key after it waited 12 ms until the next
//! one. This writes a run's events at once and waits on the server only when typing ends.

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{self, ConnectionExt as _, ModMask};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const RETURN: u32 = 0xff0d;
const TAB: u32 = 0xff09;

/// How long the server holds a press of the key just released, in ms. Qt and SDL read a release
/// followed by a press of the same key with the same timestamp (SDL: under 2 ms apart) as key
/// autorepeat, and a game can drop the repeat, so the "ee" in "needs" gets 3 ms between its two
/// presses. Other keys go at once.
const REPEAT_HOLD_MS: u32 = 3;

/// How long the server holds the first press of a keycode just bound to a new keysym, in ms. An
/// app reads the key in order after the map change, but an input method server that is sent the
/// raw key looks it up on its own time; enigo gave it the same 12 ms.
const REBIND_HOLD_MS: u32 = 12;

/// The X keysym for a character: Latin-1 keysyms are the code point, the rest are 0x0100_0000 plus it.
pub(super) fn keysym(c: char) -> u32 {
    match c {
        '\n' => RETURN,
        '\t' => TAB,
        ' '..='~' | '\u{a0}'..='\u{ff}' => c as u32,
        _ => 0x0100_0000 | c as u32,
    }
}

/// The server's keyboard map: `per` keysyms for each keycode from `min`.
pub(super) struct Map {
    pub min: u8,
    pub per: usize,
    pub syms: Vec<u32>,
}

impl Map {
    /// The keycode that types `sym`, and whether it needs Shift. A keysym in the first column needs
    /// no modifier; one in the second needs Shift, which only holds while Caps Lock is off (Shift
    /// would undo Caps Lock on a letter). Anything else gets a spare keycode of its own.
    pub fn find(&self, sym: u32, shift_ok: bool) -> Option<(u8, bool)> {
        let cols: &[usize] = if shift_ok && self.per > 1 { &[0, 1] } else { &[0] };
        cols.iter().find_map(|&col| {
            self.syms
                .chunks(self.per)
                .position(|keys| keys[col] == sym)
                .map(|i| (self.min + i as u8, col == 1))
        })
    }

    /// Keycodes with no keysym at all, free to bind for this run.
    pub fn spare(&self) -> Vec<u8> {
        self.syms
            .chunks(self.per)
            .enumerate()
            .filter(|(_, keys)| keys.iter().all(|&s| s == 0))
            .map(|(i, _)| self.min + i as u8)
            .collect()
    }
}

/// Keycodes free to bind, and the keysyms bound to them so far, least recently used first.
#[derive(Default)]
pub(super) struct Spares {
    pub free: Vec<u8>,
    pub bound: Vec<(u32, u8)>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Slot {
    /// Already bound to the keysym.
    Bound(u8),
    /// Bind it: it was free.
    Free(u8),
    /// Bind it: it held the keysym used longest ago, since none was free.
    Reused(u8),
}

impl Spares {
    pub fn slot(&mut self, sym: u32) -> Option<Slot> {
        if let Some(i) = self.bound.iter().position(|&(s, _)| s == sym) {
            let b = self.bound.remove(i);
            self.bound.push(b);
            return Some(Slot::Bound(b.1));
        }
        let slot = match self.free.pop() {
            Some(k) => Slot::Free(k),
            None if !self.bound.is_empty() => Slot::Reused(self.bound.remove(0).1),
            None => return None,
        };
        let (Slot::Free(k) | Slot::Reused(k) | Slot::Bound(k)) = slot;
        self.bound.push((sym, k));
        Some(slot)
    }
}

/// One XTest event: a keycode, down or up, and how long the server holds it first.
#[derive(Debug, PartialEq)]
pub(super) struct Event {
    pub keycode: u8,
    pub down: bool,
    pub hold_ms: u32,
}

/// The events that type one key, given the key typed before it. Shift wraps a key that needs it.
pub(super) fn strokes(keycode: u8, shift: Option<u8>, last: Option<u8>) -> Vec<Event> {
    let mut out = vec![];
    let press = |keycode: u8, out: &mut Vec<Event>| {
        let hold_ms = if out.last().map_or(last, |e: &Event| Some(e.keycode)) == Some(keycode) {
            REPEAT_HOLD_MS
        } else {
            0
        };
        out.push(Event { keycode, down: true, hold_ms });
    };
    if let Some(s) = shift {
        press(s, &mut out);
    }
    press(keycode, &mut out);
    out.push(Event { keycode, down: false, hold_ms: 0 });
    if let Some(s) = shift {
        out.push(Event { keycode: s, down: false, hold_ms: 0 });
    }
    out
}

pub(super) struct Keys {
    conn: RustConnection,
    root: xproto::Window,
    map: Map,
    shift: Option<u8>,
    /// Caps Lock was on when typing began, so Shift can't be used to reach a second column.
    caps: bool,
    /// Keycodes this run binds, put back to nothing when it ends.
    spares: Spares,
    /// The next press is of a keycode just bound to a new keysym.
    rebound: bool,
    last: Option<u8>,
}

impl Keys {
    pub fn new() -> anyhow::Result<Self> {
        let (conn, screen) = x11rb::connect(None)?;
        anyhow::ensure!(
            conn.extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)?.is_some(),
            "the X server has no XTEST"
        );
        let setup = conn.setup();
        let root = setup.roots[screen].root;
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let map = Map {
            min,
            per: usize::from(reply.keysyms_per_keycode.max(1)),
            syms: reply.keysyms,
        };
        // The modifier map's first row is Shift.
        let mods = conn.get_modifier_mapping()?.reply()?;
        let shift = mods.keycodes[..mods.keycodes.len() / 8].iter().copied().find(|&k| k != 0);
        let caps = conn.query_pointer(root)?.reply()?.mask.contains(ModMask::LOCK);
        let spares = Spares { free: map.spare(), bound: vec![] };
        Ok(Self { conn, root, map, shift, caps, spares, rebound: false, last: None })
    }

    /// The keycode for `sym` and the Shift key it needs, binding a spare keycode if the map has none.
    fn keycode(&mut self, sym: u32) -> anyhow::Result<(u8, Option<u8>)> {
        let shift_ok = self.shift.is_some() && !self.caps;
        if let Some((k, shifted)) = self.map.find(sym, shift_ok) {
            return Ok((k, if shifted { self.shift } else { None }));
        }
        let k = match self.spares.slot(sym) {
            Some(Slot::Bound(k)) => return Ok((k, None)),
            Some(Slot::Free(k)) => k,
            Some(Slot::Reused(k)) => {
                // Every key sent with the old keysym reaches its app before the map changes.
                self.conn.sync()?;
                self.rebound = true;
                k
            }
            None => anyhow::bail!("the X server has no free keycode to type {sym:#x}"),
        };
        // Both columns, or X fills the second with the keysym's other case.
        self.conn.change_keyboard_mapping(1, k, 2, &[sym, sym])?.check()?;
        Ok((k, None))
    }

    fn send(&mut self, keycode: u8, shift: Option<u8>) -> anyhow::Result<()> {
        for mut e in strokes(keycode, shift, self.last) {
            if e.down && e.keycode == keycode && std::mem::take(&mut self.rebound) {
                e.hold_ms = e.hold_ms.max(REBIND_HOLD_MS);
            }
            let kind = if e.down { xproto::KEY_PRESS_EVENT } else { xproto::KEY_RELEASE_EVENT };
            self.conn.xtest_fake_input(kind, e.keycode, e.hold_ms, self.root, 0, 0, 0)?;
            self.last = Some(e.keycode);
        }
        Ok(())
    }

    pub fn text(&mut self, t: &str) -> anyhow::Result<()> {
        for c in t.chars() {
            let (k, shift) = self.keycode(keysym(c))?;
            self.send(k, shift)?;
        }
        self.conn.flush()?;
        Ok(())
    }

    pub fn enter(&mut self, shift: bool) -> anyhow::Result<()> {
        let (k, _) = self.keycode(RETURN)?;
        let shift = if shift {
            Some(self.shift.ok_or_else(|| anyhow::anyhow!("no Shift key for Shift+Return"))?)
        } else {
            None
        };
        self.send(k, shift)?;
        self.conn.flush()?;
        Ok(())
    }

    /// Waits until the server has taken every event, so a run counts as typed only once it has.
    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.conn.sync()?;
        Ok(())
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        // The keys are processed before the map changes back, so they keep their keysyms.
        let _ = self.conn.sync();
        for &(_, k) in &self.spares.bound {
            let _ = self.conn.change_keyboard_mapping(1, k, 2, &[0, 0]);
        }
        let _ = self.conn.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A US layout's first keys: a, s, Shift_L, 1 and an empty keycode, two columns each.
    fn us() -> Map {
        Map {
            min: 38,
            per: 2,
            syms: vec![0x61, 0x41, 0x73, 0x53, 0xffe1, 0, 0x31, 0x21, 0, 0],
        }
    }

    #[test]
    fn keysyms_are_latin1_or_unicode() {
        assert_eq!(keysym('a'), 0x61);
        assert_eq!(keysym('A'), 0x41);
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('€'), 0x0100_20ac);
        assert_eq!(keysym('\u{2014}'), 0x0100_2014);
        assert_eq!(keysym('\n'), RETURN);
        assert_eq!(keysym('\t'), TAB);
    }

    #[test]
    fn capitals_and_shifted_symbols_use_shift_unless_caps_lock_is_on() {
        let m = us();
        assert_eq!(m.find(0x61, true), Some((38, false)));
        assert_eq!(m.find(0x41, true), Some((38, true)), "A is Shift+a");
        assert_eq!(m.find(0x21, true), Some((41, true)), "! is Shift+1");
        // Under Caps Lock, Shift+a is "a": a capital needs a keycode of its own.
        assert_eq!(m.find(0x41, false), None);
        assert_eq!(m.find(0xe9, true), None, "é is not on this layout");
        assert_eq!(m.spare(), vec![42]);
    }

    #[test]
    fn with_no_free_keycode_the_one_used_longest_ago_is_bound_again() {
        let mut s = Spares { free: vec![90, 91], bound: vec![] };
        assert_eq!(s.slot(0x438), Some(Slot::Free(91)));
        assert_eq!(s.slot(0x439), Some(Slot::Free(90)));
        assert_eq!(s.slot(0x438), Some(Slot::Bound(91)), "used again, so now the newest");
        assert_eq!(s.slot(0x43a), Some(Slot::Reused(90)), "0x439 was used longest ago");
        assert_eq!(s.bound, vec![(0x438, 91), (0x43a, 90)]);
        assert_eq!(Spares::default().slot(0x438), None);
    }

    #[test]
    fn a_doubled_letter_gets_the_server_to_wait_before_its_second_press() {
        // "ee": the second press would carry the release's timestamp and read as autorepeat.
        let second = strokes(26, None, Some(26));
        assert_eq!(second[0], Event { keycode: 26, down: true, hold_ms: REPEAT_HOLD_MS });
        // A different key goes at once.
        assert_eq!(strokes(27, None, Some(26))[0].hold_ms, 0);
        assert_eq!(strokes(27, None, None)[0].hold_ms, 0);
    }

    #[test]
    fn shift_wraps_the_key_and_is_released_after_it() {
        let s = strokes(38, Some(50), Some(40));
        let order: Vec<(u8, bool)> = s.iter().map(|e| (e.keycode, e.down)).collect();
        assert_eq!(order, vec![(50, true), (38, true), (38, false), (50, false)]);
        assert!(s.iter().all(|e| e.hold_ms == 0));
        // "AA": Shift comes back right after its release, so it waits too.
        assert_eq!(strokes(38, Some(50), Some(50))[0].hold_ms, REPEAT_HOLD_MS);
    }
}
