//! X11 keystrokes through XTest, written straight to the server.
//!
//! enigo typed about 108 characters a second here, not the pace's 700. For every key it listed the
//! input devices and waited on a round trip, and it had the server hold each press and release at
//! least 1 ms; once a capital or symbol came back, every key after it waited 12 ms until the next
//! one. This writes a run's events at once and waits on the server only when typing ends, after a
//! Return, and around a key that has to wait.

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _, ModMask};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const RETURN: u32 = 0xff0d;
const TAB: u32 = 0xff09;

/// How long the typer waits before a press of the key just released, in ms. Qt's xcb plugin reads a
/// press of the same keycode within 10 ms of its release as autorepeat, and SDL one within 2 ms; a
/// game can drop the repeat. 11 ms clears both, so the second "e" in "needs" is a new key. Other
/// keys go at once.
const REPEAT_HOLD_MS: u32 = 11;

/// How long a keycode's map change waits on an input method, in ms: before the first press of a
/// keycode just bound, and before a bound keycode is bound again or unbound. An app reads keys in
/// order with the map changes, but an input method server that is sent the raw key looks it up on
/// its own time; enigo gave it the same 12 ms.
const REBIND_HOLD_MS: u32 = 12;

/// The locked modifiers that change which column of a key types: Caps Lock, and Shift Lock
/// (`caps:shiftlock`). Typing lifts them for the run and locks them again when it ends.
pub(super) fn level_locks(locked: ModMask) -> ModMask {
    locked & (ModMask::SHIFT | ModMask::LOCK)
}

/// The X keysym for a character: Latin-1 keysyms are the code point, the rest are 0x0100_0000 plus it.
pub(super) fn keysym(c: char) -> u32 {
    match c {
        '\n' => RETURN,
        '\t' => TAB,
        ' '..='~' | '\u{a0}'..='\u{ff}' => c as u32,
        _ => 0x0100_0000 | c as u32,
    }
}

/// The server's keyboard map: `per` keysyms for each keycode from `min`. Under XKB a key's first
/// four keysyms are group 1 level 1 and 2, then group 2 level 1 and 2.
pub(super) struct Map {
    pub min: u8,
    pub per: usize,
    pub syms: Vec<u32>,
}

impl Map {
    /// The keycode that types `sym` in the active `group` (0 for the first layout), and whether it
    /// needs Shift. Level 1 needs no modifier and level 2 needs Shift, with no level lock held.
    /// Groups past the second, and keysyms not in the active group, get a spare keycode of their own.
    pub fn find(&self, sym: u32, group: u8, shift_ok: bool) -> Option<(u8, bool)> {
        let first = match group {
            0 | 1 => usize::from(group) * 2,
            _ => return None,
        };
        let cols = first..first + if shift_ok { 2 } else { 1 };
        cols.filter(|&col| col < self.per).find_map(|col| {
            self.syms
                .chunks(self.per)
                .position(|keys| keys[col] == sym)
                .map(|i| (self.min + i as u8, col == first + 1))
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

/// One XTest event: a keycode, down or up, and how long the typer waits before it.
#[derive(Debug, PartialEq)]
pub(super) struct Event {
    pub keycode: u8,
    pub down: bool,
    pub hold_ms: u32,
}

/// The events that type one key, given the key typed before it. Shift wraps a key that needs it.
/// What the typer does for a key: send an event, wait on the server to take everything sent, or sleep.
#[derive(Debug, PartialEq)]
pub(super) enum Step {
    Key(u8, bool),
    Sync,
    Sleep(u32),
}

/// The steps that send `events`. A wait is the typer's own, after the server has caught up, and
/// the server catches up again after the key: XTest's delay would hold the server while the typer
/// kept sending, so events would bunch up behind it and land after focus moved.
pub(super) fn steps(events: &[Event]) -> Vec<Step> {
    let mut out = vec![];
    for e in events {
        if e.hold_ms > 0 {
            out.extend([Step::Sync, Step::Sleep(e.hold_ms)]);
        }
        out.push(Step::Key(e.keycode, e.down));
    }
    if events.iter().any(|e| e.hold_ms > 0) {
        out.push(Step::Sync);
    }
    out
}

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
    /// The active XKB group: which layout's columns the keys type.
    group: u8,
    /// Level locks that were on when typing began, lifted until it ends.
    locks: ModMask,
    /// Keycodes this run binds, put back to nothing when it ends.
    spares: Spares,
    /// The next press is of a keycode just bound to a new keysym.
    rebound: bool,
    /// The last event waited inside itself.
    held: bool,
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
        // A server started without XKB (`-kb`) refuses its requests: one group, no locks.
        let xkb = conn.extension_information(xkb::X11_EXTENSION_NAME)?.is_some();
        let (group, locks) = if xkb && conn.xkb_use_extension(1, 0)?.reply()?.supported {
            let state = conn.xkb_get_state(xkb::ID::USE_CORE_KBD.into())?.reply()?;
            (u8::from(state.group), level_locks(state.locked_mods))
        } else {
            (0, ModMask::from(0u16))
        };
        let spares = Spares { free: map.spare(), bound: vec![] };
        let (rebound, held, last) = (false, false, None);
        let keys = Self { conn, root, map, shift, group, locks, spares, rebound, held, last };
        // Under Caps Lock a letter's bare key types its capital; with the lock lifted, every key
        // types the column it is found in. Drop locks it again.
        keys.lock(ModMask::from(0u16))?;
        Ok(keys)
    }

    /// Sets the lifted level locks to `to`: none to lift them, `self.locks` to put them back.
    fn lock(&self, to: ModMask) -> anyhow::Result<()> {
        if u16::from(self.locks) != 0 {
            let none = ModMask::from(0u16);
            let dev = xkb::ID::USE_CORE_KBD.into();
            self.conn
                .xkb_latch_lock_state(dev, self.locks, to, false, xkb::Group::M1, none, false, 0)?
                .check()?;
        }
        Ok(())
    }

    /// Waits until the server has taken every event, then gives an input method time to look up
    /// the keys it was sent before a spare keycode they used changes.
    fn settle(&self) -> anyhow::Result<()> {
        self.conn.sync()?;
        std::thread::sleep(std::time::Duration::from_millis(REBIND_HOLD_MS.into()));
        Ok(())
    }

    /// The keycode for `sym` and the Shift key it needs, binding a spare keycode if the map has none.
    fn keycode(&mut self, sym: u32) -> anyhow::Result<(u8, Option<u8>)> {
        if let Some((k, shifted)) = self.map.find(sym, self.group, self.shift.is_some()) {
            return Ok((k, if shifted { self.shift } else { None }));
        }
        let k = match self.spares.slot(sym) {
            Some(Slot::Bound(k)) => return Ok((k, None)),
            Some(Slot::Free(k)) => k,
            Some(Slot::Reused(k)) => {
                // Every key sent with the old keysym is looked up before the map changes.
                self.settle()?;
                k
            }
            None => anyhow::bail!("the X server has no free keycode to type {sym:#x}"),
        };
        self.rebound = true;
        // Both columns, or X fills the second with the keysym's other case.
        self.conn.change_keyboard_mapping(1, k, 2, &[sym, sym])?.check()?;
        Ok((k, None))
    }

    fn send(&mut self, keycode: u8, shift: Option<u8>) -> anyhow::Result<()> {
        let mut events = strokes(keycode, shift, self.last);
        for e in &mut events {
            if e.down && e.keycode == keycode && std::mem::take(&mut self.rebound) {
                e.hold_ms = e.hold_ms.max(REBIND_HOLD_MS);
            }
        }
        for step in steps(&events) {
            match step {
                Step::Key(k, down) => {
                    let kind = if down { xproto::KEY_PRESS_EVENT } else { xproto::KEY_RELEASE_EVENT };
                    self.conn.xtest_fake_input(kind, k, 0, self.root, 0, 0, 0)?;
                    self.last = Some(k);
                }
                Step::Sync => self.conn.sync()?,
                Step::Sleep(ms) => {
                    std::thread::sleep(std::time::Duration::from_millis(ms.into()));
                    self.held = true;
                }
            }
        }
        Ok(())
    }

    /// Whether an event since the last ask waited inside itself, so the next gap runs from its end.
    pub fn held(&mut self) -> bool {
        std::mem::take(&mut self.held)
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
        // A Return may move focus on purpose; the next focus check sees where it went.
        self.conn.sync()?;
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
        // The keys are looked up before the map changes back, so they keep their keysyms.
        if self.spares.bound.is_empty() {
            let _ = self.conn.sync();
        } else {
            let _ = self.settle();
        }
        for &(_, k) in &self.spares.bound {
            let _ = self.conn.change_keyboard_mapping(1, k, 2, &[0, 0]);
        }
        let _ = self.lock(self.locks);
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

    /// The keys and Shift that type `text` in `group`, or None for a spare keycode.
    fn keys(m: &Map, group: u8, text: &str) -> Vec<Option<(u8, bool)>> {
        text.chars().map(|c| m.find(keysym(c), group, true)).collect()
    }

    #[test]
    fn capitals_and_shifted_symbols_use_shift() {
        let m = us();
        assert_eq!(m.find(0x61, 0, true), Some((38, false)));
        assert_eq!(m.find(0x41, 0, true), Some((38, true)), "A is Shift+a");
        assert_eq!(m.find(0x21, 0, true), Some((41, true)), "! is Shift+1");
        // With no Shift key, a capital needs a keycode of its own.
        assert_eq!(m.find(0x41, 0, false), None);
        assert_eq!(m.find(0xe9, 0, true), None, "é is not on this layout");
        assert_eq!(m.spare(), vec![42]);
    }

    #[test]
    fn caps_lock_is_lifted_so_lowercase_letters_stay_lowercase() {
        // Under Caps Lock the bare "a" key types "A", so "as" would come out "AS". Caps Lock and
        // Shift Lock are lifted for the run; Num Lock (Mod2) is left alone.
        assert_eq!(level_locks(ModMask::LOCK | ModMask::M2), ModMask::LOCK);
        assert_eq!(level_locks(ModMask::SHIFT | ModMask::LOCK), ModMask::SHIFT | ModMask::LOCK);
        assert_eq!(level_locks(ModMask::M2), ModMask::from(0u16));
        // With no lock held, lowercase is bare and a capital is Shift.
        let m = us();
        assert_eq!(keys(&m, 0, "as"), vec![Some((38, false)), Some((39, false))]);
        assert_eq!(keys(&m, 0, "aS!"), vec![Some((38, false)), Some((39, true)), Some((41, true))]);
    }

    #[test]
    fn keys_come_from_the_active_layout_group() {
        // us,ru: a/A and ef/EF on one key, s/S and yeru/YERU on the next, and a space key with one
        // group, which X repeats into the second group's columns.
        let m = Map {
            min: 38,
            per: 4,
            syms: vec![0x61, 0x41, 0x6c6, 0x6e6, 0x73, 0x53, 0x6d9, 0x6f9, 0x20, 0, 0x20, 0],
        };
        assert_eq!(keys(&m, 0, "aS "), vec![Some((38, false)), Some((39, true)), Some((40, false))]);
        // With Russian active, the "a" key types "ф": "a" needs a spare keycode.
        assert_eq!(keys(&m, 1, "aS "), vec![None, None, Some((40, false))]);
        assert_eq!(m.find(0x6c6, 1, true), Some((38, false)), "ф is the bare key");
        assert_eq!(m.find(0x6f9, 1, true), Some((39, true)), "Ы is Shift");
        assert_eq!(m.find(0x6c6, 0, true), None, "ф is not in the US group");
        // A third or fourth group is not in the core map's first columns.
        assert_eq!(m.find(0x61, 2, true), None);
        // A map with one group has no second group's columns.
        assert_eq!(us().find(0x61, 1, true), None);
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
        // "ee": a press within 10 ms of the release reads as autorepeat in Qt (2 ms in SDL).
        let second = strokes(26, None, Some(26));
        assert_eq!((second[0].keycode, second[0].down), (26, true));
        assert!(second[0].hold_ms > 10, "held {} ms, inside Qt's window", second[0].hold_ms);
        // A different key goes at once.
        assert_eq!(strokes(27, None, Some(26))[0].hold_ms, 0);
        assert_eq!(strokes(27, None, None)[0].hold_ms, 0);
    }

    #[test]
    fn a_wait_starts_after_the_server_caught_up_and_the_server_catches_up_after_the_key() {
        let held = steps(&strokes(26, None, Some(26)));
        let want = [Step::Sync, Step::Sleep(REPEAT_HOLD_MS), Step::Key(26, true), Step::Key(26, false)];
        assert_eq!(held[..4], want);
        assert_eq!(held.last(), Some(&Step::Sync), "nothing is in flight at the next focus check");
        // A key with no wait is one write: no round trip.
        let plain = steps(&strokes(27, Some(50), Some(26)));
        assert!(plain.iter().all(|s| matches!(s, Step::Key(..))), "{plain:?}");
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
