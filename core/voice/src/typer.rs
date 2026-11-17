//! Types text as real keystrokes, never through the clipboard, so apps show no paste
//! prompt and the clipboard stays untouched.

use std::thread::sleep;
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct Pace {
    /// Characters sent per key event.
    pub chunk: usize,
    /// Pause after each event.
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

/// The key events for one run of text. A tool that types a whole string per call
/// (ydotool) gets batches of about `WHOLE_BATCH` characters: starting a process per character made
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

/// Types `text`, asking `still_here` before every event whether the keyboard is still on the
/// target (a check costs well under a tenth of a millisecond). Returns false if it stopped
/// because focus moved.
pub fn type_text(
    text: &str,
    pace: Pace,
    shift_enter: bool,
    still_here: &dyn Fn(bool) -> bool,
) -> anyhow::Result<bool> {
    let mut k = Keyboard::new()?;
    // One keymap for the whole text when it fits. When it doesn't (more unusual characters than
    // spare keys, review r2 on #6), each event brings its own, so this can't fail the dictation.
    let _ = k.prepare(text);
    // A Tab or Return may move focus to the next field on purpose.
    let mut after_focus_key = false;
    for piece in pieces(text) {
        if !still_here(after_focus_key) {
            return Ok(false);
        }
        match piece {
            Piece::Text(t) => {
                for c in events(t, pace.chunk, k.whole_runs()) {
                    if !still_here(after_focus_key) {
                        return Ok(false);
                    }
                    k.text(&c)?;
                    sleep(pace.gap);
                }
            }
            Piece::Newline => {
                after_focus_key = true;
                k.enter(shift_enter)?;
                sleep(pace.gap * 4);
            }
            Piece::Tab => {
                after_focus_key = true;
                k.text("\t")?;
                sleep(pace.gap);
            }
        }
    }
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

    fn whole_runs(&self) -> bool {
        false
    }

    fn prepare(&mut self, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

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

// --------------------------------------------------------- Windows / Linux

#[cfg(not(target_os = "macos"))]
struct Keyboard {
    enigo: Option<enigo::Enigo>,
    #[cfg(target_os = "linux")]
    wayland: Option<crate::wayland::VirtualKeyboard>,
    #[cfg(target_os = "linux")]
    ydotool: bool,
}

#[cfg(not(target_os = "macos"))]
impl Keyboard {
    fn new() -> anyhow::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            // Native Wayland windows ignore X11 input. Hyprland and other wlroots compositors
            // take a virtual keyboard; GNOME and KDE don't, so they need ydotool.
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                if let Some(vk) = crate::wayland::VirtualKeyboard::new() {
                    return Ok(Self { enigo: None, wayland: Some(vk), ydotool: false });
                }
                if which("ydotool") {
                    return Ok(Self { enigo: None, wayland: None, ydotool: true });
                }
            }
        }
        let enigo = enigo::Enigo::new(&enigo::Settings::default())
            .map_err(|e| anyhow::anyhow!("could not open the keyboard: {e}"))?;
        Ok(Self {
            enigo: Some(enigo),
            #[cfg(target_os = "linux")]
            wayland: None,
            #[cfg(target_os = "linux")]
            ydotool: false,
        })
    }

    /// ydotool types a whole run per process; the virtual keyboard and enigo go key event by
    /// key event.
    fn whole_runs(&self) -> bool {
        #[cfg(target_os = "linux")]
        return self.ydotool;
        #[cfg(not(target_os = "linux"))]
        false
    }

    /// Sends the virtual keyboard's keymap for the whole text up front, so the target app sees
    /// it change once.
    fn prepare(&mut self, text: &str) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(vk) = self.wayland.as_mut() {
            return vk.prepare(text);
        }
        let _ = text;
        Ok(())
    }

    fn text(&mut self, t: &str) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let Some(vk) = self.wayland.as_mut() {
                return vk.text(t);
            }
            if self.ydotool {
                let mut cmd = std::process::Command::new("ydotool");
                cmd.args(["type", "--key-delay", "1", "--"]).arg(t);
                return run("ydotool", &mut cmd);
            }
        }
        use enigo::Keyboard as _;
        self.enigo
            .as_mut()
            .unwrap()
            .text(t)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn enter(&mut self, shift: bool) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let Some(vk) = self.wayland.as_mut() {
                return vk.enter(shift);
            }
            if self.ydotool {
                // ydotool keycodes: 42 = Left Shift, 28 = Enter.
                let keys: &[&str] = if shift { &["42:1", "28:1", "28:0", "42:0"] } else { &["28:1", "28:0"] };
                let mut cmd = std::process::Command::new("ydotool");
                cmd.arg("key").args(keys);
                return run("ydotool", &mut cmd);
            }
        }
        use enigo::{Direction, Key, Keyboard as _};
        let e = self.enigo.as_mut().unwrap();
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

    /// Live check on a Wayland compositor. Run a terminal that writes what it gets to a file,
    /// headless so nothing lands on your screen:
    ///   WLR_BACKENDS=headless cage -- foot sh -c 'stty -icanon; cat > /tmp/typed.txt'
    /// then `SILAS_TYPED=/tmp/typed.txt WAYLAND_DISPLAY=wayland-N cargo test --lib typer -- --ignored`.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn types_every_character_on_wayland() {
        let path = std::env::var("SILAS_TYPED").expect("SILAS_TYPED names the terminal's file");
        let before = std::fs::read_to_string(&path).unwrap_or_default().len();
        let text = "Man needs no excuse. Parish1 runs Hyprland: naïve café, “quotes” \u{2014} 100% ok? <a href='x'>{y}</a> ~`|\\ ";
        let words = text.repeat(4);
        let t = std::time::Instant::now();
        assert!(type_text(&words, Pace::from_setting("normal"), false, &|_| true).unwrap());
        let took = t.elapsed();
        std::thread::sleep(Duration::from_millis(500));
        let got = std::fs::read_to_string(&path).unwrap();
        assert_eq!(&got[before..], words);
        let per_sec = words.chars().count() as f64 / took.as_secs_f64();
        eprintln!("{} chars in {took:?}: {per_sec:.0} chars/s", words.chars().count());
        assert!(per_sec > 400.0, "{per_sec:.0} chars/s");
    }
}
