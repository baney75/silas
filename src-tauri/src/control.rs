//! `silas press`, `silas release`, `silas toggle` and `silas cancel`: drive a running Silas from
//! a compositor keybinding. On Wayland the compositor owns the keyboard, so a Hyprland bind is
//! the dependable hotkey: it swallows the chord, works with keys Silas can't read (the Copilot
//! key sends Super + Shift + F23), and shows up in the desktop's own keybinding list.
//!
//! This is the app's end of the socket; `ctl.rs` is the command's end.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

use crate::core::Shared;
use crate::ctl::socket;
use crate::keys::Out;

/// Listen for commands for as long as Silas runs.
pub fn serve(core: Shared) {
    let path = socket();
    // Only one Silas gets this far (single-instance), so a file here is a dead one's.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("no control socket at {}: {e}", path.display());
            return;
        }
    };
    // One command at a time, like keys from the listener: a release must not overtake the
    // press that is still opening the microphone.
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(&core, stream);
        }
    });
}

fn handle(core: &Shared, stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    let mut reader = BufReader::new(&stream);
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let now = Instant::now();
    let out = match line.trim() {
        "press" => Some(core.machine.lock().external(true, now)),
        "release" => Some(core.machine.lock().external(false, now)),
        "toggle" => Some(core.machine.lock().toggle_from_ui()),
        "cancel" => Some(core.machine.lock().cancel()),
        _ => None,
    };
    let reply = if out.is_some() { "ok" } else { "unknown command" };
    let _ = writeln!(&stream, "{reply}");
    // Answer first: the binding shouldn't wait on the microphone opening.
    if let Some(out) = out.filter(|o| *o != Out::Nothing) {
        core.act(out);
    }
}
