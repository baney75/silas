//! Types text as real keystrokes, never through the clipboard, so apps show no paste
//! prompt and the clipboard stays untouched.

use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct Pace {
    /// Characters sent per key event.
    pub chunk: usize,
    /// Time from one event's start to the next's start; the event's own cost is spent inside it.
    pub gap: Duration,
}

impl Pace {
    pub fn from_setting(speed: &str) -> Self {
        match speed {
            // ~250 chars/s: for apps that drop fast input.
            "steady" => Pace {
                chunk: 1,
                gap: Duration::from_micros(4000),
            },
            // ~2000 chars/s: a long paragraph lands in well under a second.
            "instant" => Pace {
                chunk: 6,
                gap: Duration::from_micros(3000),
            },
            // ~700 chars/s.
            _ => Pace {
                chunk: 1,
                gap: Duration::from_micros(1400),
            },
        }
    }
}

enum Piece<'a> {
    Text(&'a str),
    Newline,
    Tab,
}

fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = vec![];
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if c == '\n' || c == '\t' {
            if i > start {
                out.push(Piece::Text(&text[start..i]));
            }
            out.push(if c == '\n' {
                Piece::Newline
            } else {
                Piece::Tab
            });
            start = i + c.len_utf8();
        }
    }
    if start < text.len() {
        out.push(Piece::Text(&text[start..]));
    }
    out
}

/// Splits text into key events of about `size` characters. A space never travels alone: it
/// rides with the character before it ("no " is one event). macOS turns two quick space events
/// into ". " (the double-space period shortcut), and an app that lags behind fast input checks
/// that shortcut against stale text, so "needs no excuse" landed as "needs. noexcuse".
fn chunks(text: &str, size: usize) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    let mut n = 0;
    for c in text.chars() {
        if c.is_whitespace() || n < size.max(1) {
            cur.push(c);
            n += usize::from(!c.is_whitespace());
            continue;
        }
        out.push(std::mem::take(&mut cur));
        cur.push(c);
        n = 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// How many characters (spaces not counted) a tool that types whole strings gets per process.
/// Small enough that focus and Escape are checked every few words, large enough that a sentence
/// is a handful of processes.
const WHOLE_BATCH: usize = 24;

/// The key events for one run of text. A tool that types a whole string per call (wtype,
/// ydotool) gets batches of about `WHOLE_BATCH` characters: starting a process per character made
/// a sentence take seconds, and one process for the whole run kept typing a long paragraph into
/// whatever window took focus, with no check until it finished. It paces itself, and the
/// double-space shortcut `chunks` guards against is macOS's.
fn events(text: &str, size: usize, whole: bool) -> Vec<String> {
    if whole {
        chunks(text, WHOLE_BATCH)
    } else {
        chunks(text, size)
    }
}

/// What the typing loop needs from a platform's keyboard.
trait Keys {
    /// The tool types a whole string per call, so it gets batches rather than key events.
    fn whole_runs(&self) -> bool {
        false
    }
    fn text(&mut self, t: &str) -> anyhow::Result<()>;
    fn enter(&mut self, shift: bool) -> anyhow::Result<()>;
    /// Waits until the last event has left, so a run counts as typed only once it has.
    fn finish(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Waits until `due`, then sets the next event `gap` after this one starts. The gap runs from one
/// event's start to the next, so the time an event takes (a focus check, a round trip, starting a
/// process) is spent inside it instead of added to it, and events are still never closer than it.
fn pace(due: &mut Instant, gap: Duration) {
    let now = Instant::now();
    if now < *due {
        sleep(*due - now);
    }
    *due = Instant::now() + gap;
}

/// Types `text`, asking `still_here` before every event whether the keyboard is still on the
/// target (a check costs well under a tenth of a millisecond). Returns false if it stopped
/// because focus moved.
pub fn type_text(
    text: &str,
    pace: Pace,
    shift_enter: bool,
    still_here: &dyn Fn(bool) -> bool,
) -> anyhow::Result<bool> {
    type_with(&mut Keyboard::new()?, text, pace, shift_enter, still_here)
}

fn type_with(
    k: &mut impl Keys,
    text: &str,
    p: Pace,
    shift_enter: bool,
    still_here: &dyn Fn(bool) -> bool,
) -> anyhow::Result<bool> {
    let mut due = Instant::now();
    // A Tab or Return may move focus to the next field on purpose.
    let mut after_focus_key = false;
    for piece in pieces(text) {
        match piece {
            Piece::Text(t) => {
                for c in events(t, p.chunk, k.whole_runs()) {
                    pace(&mut due, p.gap);
                    if !still_here(after_focus_key) {
                        return Ok(false);
                    }
                    k.text(&c)?;
                }
            }
            Piece::Newline => {
                pace(&mut due, p.gap * 4);
                if !still_here(after_focus_key) {
                    return Ok(false);
                }
                after_focus_key = true;
                k.enter(shift_enter)?;
            }
            Piece::Tab => {
                pace(&mut due, p.gap);
                if !still_here(after_focus_key) {
                    return Ok(false);
                }
                after_focus_key = true;
                k.text("\t")?;
            }
        }
    }
    k.finish()?;
    Ok(true)
}

// ------------------------------------------------------------------ macOS

#[cfg(target_os = "macos")]
struct Keyboard {
    source: core_graphics::event_source::CGEventSource,
}

#[cfg(target_os = "macos")]
impl Keyboard {
    fn new() -> anyhow::Result<Self> {
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| anyhow::anyhow!("Silas needs Accessibility access to type"))?;
        Ok(Self { source })
    }

    fn post(
        &self,
        keycode: u16,
        text: Option<&str>,
        flags: core_graphics::event::CGEventFlags,
    ) -> anyhow::Result<()> {
        use core_graphics::event::{CGEvent, CGEventTapLocation};
        for down in [true, false] {
            let ev = CGEvent::new_keyboard_event(self.source.clone(), keycode, down)
                .map_err(|_| anyhow::anyhow!("could not create a key event"))?;
            // Clear modifiers so a key the user is still holding cannot turn text into shortcuts.
            ev.set_flags(flags);
            if let Some(t) = text {
                ev.set_string(t);
            }
            ev.post(CGEventTapLocation::HID);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
impl Keys for Keyboard {
    fn text(&mut self, t: &str) -> anyhow::Result<()> {
        self.post(
            0,
            Some(t),
            core_graphics::event::CGEventFlags::CGEventFlagNull,
        )
    }

    fn enter(&mut self, shift: bool) -> anyhow::Result<()> {
        use core_graphics::event::CGEventFlags;
        let flags = if shift {
            CGEventFlags::CGEventFlagShift
        } else {
            CGEventFlags::CGEventFlagNull
        };
        self.post(36, None, flags)
    }
}

// ------------------------------------------------------------------ Linux

#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
enum Keyboard {
    /// wtype or ydotool, for native Wayland windows, which ignore X11 input.
    Tool(&'static str),
    X11(Box<x11::Keys>),
}

#[cfg(target_os = "linux")]
impl Keyboard {
    fn new() -> anyhow::Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            for tool in ["wtype", "ydotool"] {
                if which(tool) {
                    return Ok(Self::Tool(tool));
                }
            }
        }
        x11::Keys::new()
            .map(|k| Self::X11(Box::new(k)))
            .map_err(|e| anyhow::anyhow!("could not open the keyboard: {e}"))
    }
}

#[cfg(target_os = "linux")]
impl Keys for Keyboard {
    fn whole_runs(&self) -> bool {
        matches!(self, Self::Tool(_))
    }

    fn text(&mut self, t: &str) -> anyhow::Result<()> {
        match self {
            Self::Tool(tool) => {
                let mut cmd = std::process::Command::new(*tool);
                match *tool {
                    "wtype" => cmd.arg("--").arg(t),
                    _ => cmd.args(ydotool_type(ydotool_holds())).arg("--").arg(t),
                };
                run(tool, &mut cmd)
            }
            Self::X11(x) => x.text(t),
        }
    }

    fn enter(&mut self, shift: bool) -> anyhow::Result<()> {
        match self {
            Self::Tool(tool) => {
                let mut cmd = std::process::Command::new(*tool);
                match (*tool, shift) {
                    ("wtype", true) => cmd.args(["-M", "shift", "-k", "Return", "-m", "shift"]),
                    ("wtype", false) => cmd.args(["-k", "Return"]),
                    // ydotool keycodes: 42 = Left Shift, 28 = Enter.
                    (_, true) => cmd.args(["key", "42:1", "28:1", "28:0", "42:0"]),
                    (_, false) => cmd.args(["key", "28:1", "28:0"]),
                };
                run(tool, &mut cmd)
            }
            Self::X11(x) => x.enter(shift),
        }
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Tool(_) => Ok(()),
            Self::X11(x) => x.finish(),
        }
    }
}

/// `ydotool type`'s options. ydotool 1.0.4 holds each key down 20 ms unless told otherwise, which
/// kept typing under 50 characters a second; `--key-hold 1` is the 1 ms hold that 1.0.0 and 1.0.1
/// take from `--key-delay 1`. Those two refuse `--key-hold`, so it goes only where `--help` lists it.
#[cfg(target_os = "linux")]
fn ydotool_type(hold: bool) -> Vec<&'static str> {
    let mut args = vec!["type", "--key-delay", "1"];
    if hold {
        args.extend(["--key-hold", "1"]);
    }
    args
}

/// Whether this ydotool takes `--key-hold`, asked once. A failed answer (no daemon yet) is asked
/// again next time.
#[cfg(target_os = "linux")]
fn ydotool_holds() -> bool {
    static HOLDS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if let Some(&h) = HOLDS.get() {
        return h;
    }
    match std::process::Command::new("ydotool").args(["type", "--help"]).output() {
        Ok(o) if o.status.success() => *HOLDS.get_or_init(|| {
            lists_key_hold(&o.stdout) || lists_key_hold(&o.stderr)
        }),
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn lists_key_hold(help: &[u8]) -> bool {
    String::from_utf8_lossy(help).contains("--key-hold")
}

// --------------------------------------------------------- Windows and the rest

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
struct Keyboard {
    enigo: enigo::Enigo,
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl Keyboard {
    fn new() -> anyhow::Result<Self> {
        let enigo = enigo::Enigo::new(&enigo::Settings::default())
            .map_err(|e| anyhow::anyhow!("could not open the keyboard: {e}"))?;
        Ok(Self { enigo })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl Keys for Keyboard {
    fn text(&mut self, t: &str) -> anyhow::Result<()> {
        use enigo::Keyboard as _;
        self.enigo.text(t).map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn enter(&mut self, shift: bool) -> anyhow::Result<()> {
        use enigo::{Direction, Key, Keyboard as _};
        let e = &mut self.enigo;
        let err = |e: enigo::InputError| anyhow::anyhow!("{e}");
        if shift {
            e.key(Key::Shift, Direction::Press).map_err(err)?;
        }
        let r = e.key(Key::Return, Direction::Click).map_err(err);
        if shift {
            e.key(Key::Shift, Direction::Release).map_err(err)?;
        }
        r
    }
}

/// Runs a typing tool; a failed exit is an error, so the text goes to History instead of being
/// reported as typed.
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn run(tool: &str, cmd: &mut std::process::Command) -> anyhow::Result<()> {
    let status = cmd.status().map_err(|e| anyhow::anyhow!("{tool} couldn't start: {e}"))?;
    anyhow::ensure!(status.success(), "{tool} failed to type ({status})");
    Ok(())
}

#[cfg(target_os = "linux")]
fn which(tool: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_newlines_and_tabs() {
        let p = pieces("a\nb\tc");
        assert_eq!(p.len(), 5);
        assert!(matches!(p[1], Piece::Newline));
        assert!(matches!(p[3], Piece::Tab));
        assert_eq!(chunks("héllo", 2), vec!["hé", "ll", "o"]);
    }

    #[test]
    fn a_tool_that_types_whole_strings_gets_one_call_per_run() {
        let text = "Man needs no excuse.";
        assert_eq!(events(text, 1, true), vec![text]);
        // Key by key elsewhere, as before.
        assert_eq!(events(text, 1, false), chunks(text, 1));
        assert!(events(text, 1, false).len() > 15);
    }

    #[test]
    fn a_long_run_reaches_a_whole_string_tool_in_batches_so_focus_is_checked_between_them() {
        // Review sol-F1: one ydotool process per paragraph kept typing after focus moved.
        let text = "Man needs no excuse to strive for excellence, and a long dictated paragraph \
                    must stop when the person changes windows or presses Escape.";
        let batches = events(text, 1, true);
        assert!(batches.len() > 3, "{batches:?}");
        assert_eq!(batches.concat(), text);
        for b in &batches {
            assert!(b.chars().filter(|c| !c.is_whitespace()).count() <= WHOLE_BATCH, "{b:?}");
        }
    }

    /// Records what it was asked to type, taking `cost` for each run of text.
    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
        starts: Vec<Instant>,
        cost: Duration,
        /// How long the `cost` sleeps really took (a loaded CI runner oversleeps a lot).
        spent: Duration,
        finished: bool,
    }

    impl Keys for Fake {
        fn text(&mut self, t: &str) -> anyhow::Result<()> {
            self.starts.push(Instant::now());
            self.log.push(t.into());
            let t0 = Instant::now();
            sleep(self.cost);
            self.spent += t0.elapsed();
            Ok(())
        }
        fn enter(&mut self, shift: bool) -> anyhow::Result<()> {
            self.log.push(if shift { "Shift+Return" } else { "Return" }.into());
            Ok(())
        }
        fn finish(&mut self) -> anyhow::Result<()> {
            self.finished = true;
            Ok(())
        }
    }

    const NO_GAP: Pace = Pace { chunk: 1, gap: Duration::ZERO };

    #[test]
    fn keys_go_out_in_order_and_a_whole_run_is_finished() {
        let mut k = Fake::default();
        assert!(type_with(&mut k, "Ab\ncd\te", NO_GAP, true, &|_| true).unwrap());
        assert_eq!(k.log, ["A", "b", "Shift+Return", "c", "d", "\t", "e"]);
        assert!(k.finished);
    }

    #[test]
    fn typing_stops_at_the_first_event_after_focus_moves() {
        let mut k = Fake::default();
        let asked = std::cell::RefCell::new(vec![]);
        let still_here = |after_focus_key: bool| {
            asked.borrow_mut().push(after_focus_key);
            asked.borrow().len() < 5
        };
        assert!(!type_with(&mut k, "ab\ncdef", NO_GAP, false, &still_here).unwrap());
        assert_eq!(k.log, ["a", "b", "Return", "c"], "nothing after the check that said no");
        assert!(!k.finished);
        // After a Return only the window is checked, since the Return may move focus on purpose.
        assert_eq!(*asked.borrow(), [false, false, false, true, true]);
    }

    #[test]
    fn the_gap_runs_from_one_event_to_the_next_instead_of_after_each() {
        // Each event takes at least the gap. Sleeping a gap after each event, as the typer did,
        // adds 10 more gaps on top of the events' own time; counting the event inside the gap adds
        // none. The bound is relative to what the events really took, since a loaded macOS runner
        // stretched a 10 ms sleep to about 31 ms (2026-11-18) and a fixed bound failed there.
        let gap = Duration::from_millis(10);
        let mut k = Fake { cost: gap, ..Fake::default() };
        let t = Instant::now();
        assert!(type_with(&mut k, "abcdefghij", Pace { chunk: 1, gap }, false, &|_| true).unwrap());
        let took = t.elapsed();
        assert!(took < k.spent + gap * 5, "{took:?} for events that took {:?}", k.spent);
        // Never closer together than the gap.
        assert!(k.starts.windows(2).all(|w| w[1] - w[0] >= gap), "{:?}", k.starts);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ydotool_gets_a_short_hold_only_when_it_takes_one() {
        let v104 = b"  -d, --key-delay=N          Delay N milliseconds between keys (the delay between every key down/up pair) (default: 20)\n  -H, --key-hold=N           Hold each key for N milliseconds (the delay between key down and up) (default: 20)\n";
        let v101 = b"  -d, --key-delay=N          Delay N milliseconds between key events (up/down each)\n  -D, --next-delay=N         Delay N milliseconds between strings\n";
        assert!(lists_key_hold(v104));
        assert!(!lists_key_hold(v101));
        assert_eq!(ydotool_type(true), ["type", "--key-delay", "1", "--key-hold", "1"]);
        assert_eq!(ydotool_type(false), ["type", "--key-delay", "1"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_fails_is_an_error() {
        assert!(run("true", &mut std::process::Command::new("true")).is_ok());
        let e = run("false", &mut std::process::Command::new("false")).unwrap_err().to_string();
        assert!(e.contains("false failed to type"), "{e}");
        assert!(run("nope", &mut std::process::Command::new("/no/such/tool")).is_err());
    }

    #[test]
    fn spaces_never_travel_alone() {
        assert_eq!(
            chunks("needs no excuse ", 1),
            vec!["n", "e", "e", "d", "s ", "n", "o ", "e", "x", "c", "u", "s", "e "]
        );
        assert_eq!(chunks(" a  b", 1), vec![" a  ", "b"]);
        assert_eq!(chunks("Man needs no", 6), vec!["Man nee", "ds no"]);
        for size in [1, 6] {
            let text = "Man needs no excuse to strive for excellence. ";
            let c = chunks(text, size);
            assert_eq!(c.concat(), text);
            assert!(c.iter().all(|e| !e.starts_with(' ') && e.chars().count() <= 20));
        }
    }
}
