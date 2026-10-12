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

fn chunks(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(size.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

pub fn type_text(text: &str, pace: Pace, shift_enter: bool) -> anyhow::Result<()> {
    let mut k = Keyboard::new()?;
    for piece in pieces(text) {
        match piece {
            Piece::Text(t) => {
                for c in chunks(t, pace.chunk) {
                    k.text(&c)?;
                    sleep(pace.gap);
                }
            }
            Piece::Newline => {
                k.enter(shift_enter)?;
                sleep(pace.gap * 4);
            }
            Piece::Tab => {
                k.text("\t")?;
                sleep(pace.gap);
            }
        }
    }
    Ok(())
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
    wayland_tool: Option<&'static str>,
}

#[cfg(not(target_os = "macos"))]
impl Keyboard {
    fn new() -> anyhow::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            // Native Wayland windows ignore X11 input, so prefer a Wayland typing tool.
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                for tool in ["wtype", "ydotool"] {
                    if which(tool) {
                        return Ok(Self {
                            enigo: None,
                            wayland_tool: Some(tool),
                        });
                    }
                }
            }
        }
        let enigo = enigo::Enigo::new(&enigo::Settings::default())
            .map_err(|e| anyhow::anyhow!("could not open the keyboard: {e}"))?;
        Ok(Self {
            enigo: Some(enigo),
            #[cfg(target_os = "linux")]
            wayland_tool: None,
        })
    }

    fn text(&mut self, t: &str) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(tool) = self.wayland_tool {
            let status = match tool {
                "wtype" => std::process::Command::new("wtype")
                    .arg("--")
                    .arg(t)
                    .status()?,
                _ => std::process::Command::new("ydotool")
                    .args(["type", "--key-delay", "1", "--"])
                    .arg(t)
                    .status()?,
            };
            anyhow::ensure!(status.success(), "{tool} failed to type");
            return Ok(());
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
        if let Some(tool) = self.wayland_tool {
            let mut cmd = std::process::Command::new(tool);
            match (tool, shift) {
                ("wtype", true) => cmd.args(["-M", "shift", "-k", "Return", "-m", "shift"]),
                ("wtype", false) => cmd.args(["-k", "Return"]),
                // ydotool keycodes: 42 = Left Shift, 28 = Enter.
                (_, true) => cmd.args(["key", "42:1", "28:1", "28:0", "42:0"]),
                (_, false) => cmd.args(["key", "28:1", "28:0"]),
            };
            cmd.status()?;
            return Ok(());
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
}
