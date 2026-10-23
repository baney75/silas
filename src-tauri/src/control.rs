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
    if let Err(e) = private_dir(&path) {
        log::warn!("no control socket at {}: {e}", path.display());
        return;
    }
    // Only one Silas gets this far (single-instance), so a file here is a dead one's.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("no control socket at {}: {e}", path.display());
            return;
        }
    };
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    // Commands are stamped as they arrive and acted on one at a time, like keys from the
    // listener: a release must not overtake the press that is still opening the microphone,
    // and a quick tap must still read as a tap when the microphone was slow to open.
    let (tx, rx) = std::sync::mpsc::channel::<(String, Instant)>();
    std::thread::spawn(move || {
        for (cmd, at) in rx {
            let out = match cmd.as_str() {
                "press" => core.machine.lock().external(true, at),
                "release" => core.machine.lock().external(false, at),
                "toggle" => core.machine.lock().toggle_from_ui(),
                _ => core.machine.lock().cancel(),
            };
            if out != Out::Nothing {
                core.act(out);
            }
        }
    });
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Some(cmd) = read(&stream) {
                        let _ = tx.send((cmd, Instant::now()));
                    }
                }
                Err(e) => {
                    // Out of file descriptors, say: don't spin.
                    log::warn!("control socket: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
}

/// The socket's directory must be ours and closed to others. The runtime dir already is; the
/// temp-dir fallback is created that way, and refused if someone else made it first.
fn private_dir(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent"))?;
    if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
        return Ok(());
    }
    let _ = std::fs::DirBuilder::new().mode(0o700).create(dir);
    let meta = std::fs::symlink_metadata(dir)?;
    let mine = std::fs::metadata("/proc/self").map(|m| m.uid()).ok();
    if !meta.is_dir() || mine.is_some_and(|uid| meta.uid() != uid) || meta.permissions().mode() & 0o077 != 0 {
        return Err(std::io::Error::other("the socket directory isn't private"));
    }
    Ok(())
}

/// One command line, answered at once so the keybinding never waits on the microphone.
fn read(stream: &UnixStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let cmd = line.trim().to_string();
    let known = crate::ctl::COMMANDS.contains(&cmd.as_str());
    let _ = writeln!(&*stream, "{}", if known { "ok" } else { "unknown command" });
    known.then_some(cmd)
}
