//! Types on Wayland through the virtual-keyboard protocol (Hyprland, Sway and other wlroots
//! compositors). One connection and one keymap per dictation: running wtype once per key cost
//! about 9 ms a key, so a paragraph took seconds instead of a fraction of one.
//!
//! The keymap holds the US keys at their real keycodes (so `event.code` in a browser still
//! says "KeyA" for an a) plus a spare keycode for every other character in the text.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::Instant;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// evdev codes for Return and Tab.
const RETURN: u32 = 28;
const TAB: u32 = 15;
/// Shift's bit in every xkb keymap.
const SHIFT: u32 = 1;
/// Keycodes nothing on a normal keyboard sends meaning into text: 195-199 are unassigned and
/// 200-247 are media keys, which Silas's keymap turns into plain characters.
const SPARE: std::ops::RangeInclusive<u32> = 195..=247;

/// The US layout: each row's keys from their first evdev code, plain and with Shift.
const US_ROWS: &[(u32, &str, &str)] = &[
    (2, "1234567890-=", "!@#$%^&*()_+"),
    (16, "qwertyuiop[]", "QWERTYUIOP{}"),
    (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
    (43, "\\zxcvbnm,./", "|ZXCVBNM<>?"),
];
const SPACE: u32 = 57;

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpVirtualKeyboardManagerV1, _: <ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwpVirtualKeyboardV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpVirtualKeyboardV1, _: <ZwpVirtualKeyboardV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

/// Where a character lives in Silas's keymap: an evdev code, and whether it needs Shift.
type Slot = (u32, bool);

fn us_slot(c: char) -> Option<Slot> {
    if c == ' ' {
        return Some((SPACE, false));
    }
    for (first, plain, shifted) in US_ROWS {
        if let Some(i) = plain.chars().position(|p| p == c) {
            return Some((first + i as u32, false));
        }
        if let Some(i) = shifted.chars().position(|p| p == c) {
            return Some((first + i as u32, true));
        }
    }
    None
}

/// The xkb keysym that types `c`: Latin-1 has its own, everything else uses the Unicode range.
fn keysym(c: char) -> u32 {
    let u = c as u32;
    if (0x20..=0x7e).contains(&u) || (0xa0..=0xff).contains(&u) {
        u
    } else {
        0x0100_0000 + u
    }
}

/// The keymap text: every US key, plus one spare keycode per other character.
fn keymap(spares: &HashMap<char, u32>) -> String {
    let mut codes = String::new();
    let mut symbols = String::new();
    let mut key = |code: u32, syms: &str| {
        let xkb = code + 8;
        codes.push_str(&format!("<K{xkb}> = {xkb};\n"));
        symbols.push_str(&format!("key <K{xkb}> {{ [ {syms} ] }};\n"));
    };
    for (first, plain, shifted) in US_ROWS {
        for (i, (p, s)) in plain.chars().zip(shifted.chars()).enumerate() {
            key(first + i as u32, &format!("{:#x}, {:#x}", keysym(p), keysym(s)));
        }
    }
    key(SPACE, "space");
    key(RETURN, "Return");
    key(TAB, "Tab");
    let mut extra: Vec<_> = spares.iter().collect();
    extra.sort_by_key(|(_, code)| **code);
    for (c, code) in extra {
        key(*code, &format!("{:#x}", keysym(*c)));
    }
    format!(
        "xkb_keymap {{\nxkb_keycodes \"silas\" {{\nminimum = 8;\nmaximum = 255;\n{codes}}};\n\
         xkb_types \"silas\" {{ include \"complete\" }};\n\
         xkb_compatibility \"silas\" {{ include \"complete\" }};\n\
         xkb_symbols \"silas\" {{\n{symbols}}};\n}};\n"
    )
}

/// Give every character of `text` that has no key a spare one. When the spares run out, keep
/// only the ones `text` needs. True when the map changed.
fn assign(spares: &mut HashMap<char, u32>, text: &str) -> anyhow::Result<bool> {
    let missing: Vec<char> = {
        let mut m: Vec<char> = text
            .chars()
            .filter(|c| !matches!(c, '\n' | '\t') && us_slot(*c).is_none() && !spares.contains_key(c))
            .collect();
        m.sort_unstable();
        m.dedup();
        m
    };
    if missing.is_empty() {
        return Ok(false);
    }
    let free = |spares: &HashMap<char, u32>| SPARE.clone().filter(|code| !spares.values().any(|v| v == code)).count();
    if free(spares) < missing.len() {
        spares.retain(|c, _| text.contains(*c));
        anyhow::ensure!(free(spares) >= missing.len(), "too many different characters to type at once");
    }
    for c in missing {
        let code = SPARE.clone().find(|code| !spares.values().any(|v| v == code)).expect("counted above");
        spares.insert(c, code);
    }
    Ok(true)
}

pub struct VirtualKeyboard {
    conn: Connection,
    queue: EventQueue<State>,
    kb: ZwpVirtualKeyboardV1,
    spares: HashMap<char, u32>,
    /// The keymap the compositor has, if any.
    uploaded: bool,
    shift: bool,
    born: Instant,
}

impl VirtualKeyboard {
    /// None when there is no Wayland session or the compositor doesn't offer virtual keyboards
    /// (GNOME and KDE don't).
    pub fn new() -> Option<Self> {
        let conn = Connection::connect_to_env().ok()?;
        let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).ok()?;
        let manager: ZwpVirtualKeyboardManagerV1 = globals.bind(&qh, 1..=1, ()).ok()?;
        let kb = manager.create_virtual_keyboard(&seat, &qh, ());
        Some(Self {
            conn,
            queue,
            kb,
            spares: HashMap::new(),
            uploaded: false,
            shift: false,
            born: Instant::now(),
        })
    }

    /// Make sure every character of `text` has a key, uploading a new keymap only when one
    /// is missing. Call it with the whole dictation first so the keymap is usually sent once;
    /// each chunk calls it again, which remaps when the text has more unusual characters than
    /// spare keys.
    pub fn prepare(&mut self, text: &str) -> anyhow::Result<()> {
        if assign(&mut self.spares, text)? || !self.uploaded {
            self.upload()?;
        }
        Ok(())
    }

    fn upload(&mut self) -> anyhow::Result<()> {
        let mut map = keymap(&self.spares).into_bytes();
        map.push(0);
        let fd = unsafe { libc::memfd_create(c"silas-keymap".as_ptr(), libc::MFD_CLOEXEC) };
        anyhow::ensure!(fd >= 0, "could not make the keymap");
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        std::fs::File::from(fd.try_clone()?).write_all(&map)?;
        self.kb.keymap(1, fd.as_fd(), map.len() as u32); // 1 = XKB_V1
        // The compositor must have the keymap before the first key that needs it.
        self.queue.roundtrip(&mut State)?;
        self.uploaded = true;
        Ok(())
    }

    fn slot(&self, c: char) -> Option<Slot> {
        match c {
            '\n' => Some((RETURN, false)),
            '\t' => Some((TAB, false)),
            _ => us_slot(c).or_else(|| self.spares.get(&c).map(|code| (*code, false))),
        }
    }

    fn set_shift(&mut self, on: bool) {
        if self.shift != on {
            self.kb.modifiers(if on { SHIFT } else { 0 }, 0, 0, 0);
            self.shift = on;
        }
    }

    fn tap(&mut self, code: u32) {
        let t = self.born.elapsed().as_millis() as u32;
        self.kb.key(t, code, 1);
        self.kb.key(t, code, 0);
    }

    pub fn text(&mut self, text: &str) -> anyhow::Result<()> {
        self.prepare(text)?;
        for c in text.chars() {
            let (code, shift) = self.slot(c).ok_or_else(|| anyhow::anyhow!("no key for {c:?}"))?;
            self.set_shift(shift);
            self.tap(code);
        }
        self.set_shift(false);
        self.conn.flush()?;
        Ok(())
    }

    pub fn enter(&mut self, shift: bool) -> anyhow::Result<()> {
        self.prepare("")?;
        self.set_shift(shift);
        self.tap(RETURN);
        self.set_shift(false);
        self.conn.flush()?;
        Ok(())
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        // Let the compositor take every key before the keyboard goes away.
        let _ = self.queue.roundtrip(&mut State);
        self.kb.destroy();
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_keys_sit_at_their_real_codes() {
        assert_eq!(us_slot('a'), Some((30, false)));
        assert_eq!(us_slot('A'), Some((30, true)));
        assert_eq!(us_slot('1'), Some((2, false)));
        assert_eq!(us_slot('?'), Some((53, true)));
        assert_eq!(us_slot('"'), Some((40, true)));
        assert_eq!(us_slot('|'), Some((43, true)));
        assert_eq!(us_slot(' '), Some((SPACE, false)));
        assert_eq!(us_slot('é'), None);
    }

    #[test]
    fn more_unusual_characters_than_spare_keys_still_type() {
        // 80 different Greek and Cyrillic letters: more than the 53 spare keys.
        let text: String = ('Α'..='ω').chain('А'..='я').take(80).collect();
        let mut spares = HashMap::new();
        assert!(assign(&mut spares, &text).is_err(), "one keymap can't hold them all");
        // Typed in chunks, each chunk gets its own keys and keeps every code unique.
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(7) {
            let chunk: String = chunk.iter().collect();
            assign(&mut spares, &chunk).unwrap();
            for c in chunk.chars() {
                assert!(spares.contains_key(&c));
            }
            let mut codes: Vec<_> = spares.values().collect();
            codes.sort();
            codes.dedup();
            assert_eq!(codes.len(), spares.len());
            assert!(spares.values().all(|c| SPARE.contains(c)));
        }
        // Plain text needs no spare keys and no new keymap.
        assert!(!assign(&mut spares, "Hello, world!").unwrap());
    }

    #[test]
    fn keymap_lists_every_key_once() {
        let mut spares = HashMap::new();
        spares.insert('é', 195);
        spares.insert('—', 196);
        let map = keymap(&spares);
        assert!(map.contains("<K38> = 38;")); // a
        assert!(map.contains("key <K38> { [ 0x61, 0x41 ] };"));
        assert!(map.contains("key <K203> { [ 0xe9 ] };")); // é is Latin-1
        assert!(map.contains("key <K204> { [ 0x1002014 ] };")); // an em dash is not
        let codes = map.lines().filter(|l| l.starts_with("<K")).count();
        assert_eq!(codes, 47 + 3 + 2);
    }
}
