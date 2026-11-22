//! `silas-ctl press|release|toggle|cancel`: drive the running engine from a compositor
//! keybinding. On Wayland the compositor owns the keyboard, so a Hyprland bind is the dependable
//! hotkey: it swallows the chord, works with keys Silas can't read (the Copilot key sends
//! Super + Shift + F23), and shows up in the desktop's own keybinding list.
//!
//! This is the engine's end of the socket; `ctl.rs` is the command's end. The socket is meant to
//! fold into the `silas.voice` engine socket proposed in baney75/local-studio#6, which carries
//! the same press, release and cancel as `capture.start`, `capture.stop` and `capture.cancel`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

use crate::core::{Control, Shared};
use crate::ctl::socket;

/// Listen for commands for as long as the app runs. Call it once, from the single instance.
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
    let (tx, rx) = std::sync::mpsc::channel::<(Control, Instant)>();
    std::thread::spawn(move || {
        for (command, at) in rx {
            core.on_control(command, at);
        }
    });
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Some(command) = read(&stream) {
                        let _ = tx.send((command, Instant::now()));
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
    if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
        return Ok(());
    }
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent"))?;
    ensure_private(dir)
}

fn ensure_private(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let _ = std::fs::DirBuilder::new().mode(0o700).create(dir);
    let meta = std::fs::symlink_metadata(dir)?;
    let mine = std::fs::metadata("/proc/self").map(|m| m.uid()).ok();
    if !meta.is_dir() || mine.is_some_and(|uid| meta.uid() != uid) || meta.permissions().mode() & 0o077 != 0 {
        return Err(std::io::Error::other("the socket directory isn't private"));
    }
    Ok(())
}

pub fn parse(line: &str) -> Option<Control> {
    match line.trim() {
        "press" => Some(Control::Press),
        "release" => Some(Control::Release),
        "toggle" => Some(Control::Toggle),
        "cancel" => Some(Control::Cancel),
        _ => None,
    }
}

/// One command line, answered at once so the keybinding never waits on the microphone.
fn read(stream: &UnixStream) -> Option<Control> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let command = parse(&line);
    let _ = writeln!(&*stream, "{}", if command.is_some() { "ok" } else { "unknown command" });
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_silas_ctl_sends_is_understood() {
        for c in crate::ctl::COMMANDS {
            assert!(parse(&format!("{c}\n")).is_some(), "{c}");
        }
        assert_eq!(parse("press\n"), Some(Control::Press));
        assert_eq!(parse("explode"), None);
    }

    /// Review r1 on #6: the socket must not be open to other users.
    #[test]
    fn a_shared_socket_dir_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("silas-ctl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ensure_private(&dir).is_ok(), "created owner-only");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let open = ensure_private(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(open.is_err());
        // A file where the directory should be is refused too.
        std::fs::write(&dir, "").unwrap();
        let file = ensure_private(&dir);
        let _ = std::fs::remove_file(&dir);
        assert!(file.is_err());
    }
}
