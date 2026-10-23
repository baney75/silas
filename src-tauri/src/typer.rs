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
    // One keymap for the whole text when it fits; otherwise each chunk brings its own.
    let _ = k.prepare(text);
    // A Tab or Return may move focus to the next field on purpose.
    let mut after_focus_key = false;
    for piece in pieces(text) {
        if !still_here(after_focus_key) {
            return Ok(false);
        }
        match piece {
            Piece::Text(t) => {
                for c in chunks(t, pace.chunk) {
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

    /// Sends the keymap for the whole text up front, so the target app sees it change once.
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
                let status = std::process::Command::new("ydotool")
                    .args(["type", "--key-delay", "1", "--"])
                    .arg(t)
                    .status()?;
                anyhow::ensure!(status.success(), "ydotool failed to type");
                return Ok(());
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
                std::process::Command::new("ydotool").arg("key").args(keys).status()?;
                return Ok(());
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
        let text = "Man needs no excuse. Parish1 runs Hyprland: naïve café, “quotes” — 100% ok? <a href='x'>{y}</a> ~`|\\ ";
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
