//! The command end of the control socket (see `control.rs`). Only std, so the `silas-ctl`
//! binary skips loading the app's GUI libraries and a key press costs about a millisecond
//! instead of 30.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const COMMANDS: &[&str] = &["press", "release", "toggle", "cancel"];

/// In the runtime dir, which only the user can open. Without one, a name with the uid in the
/// temp dir; the app makes the socket owner-only either way.
pub fn socket() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("silas.sock"),
        None => std::env::temp_dir().join(format!("silas-{}.sock", uid())),
    }
}

fn uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(std::env::var_os("HOME").unwrap_or_default()).map(|m| m.uid()).unwrap_or(0)
}

/// Handles `silas <command>`. Some(exit code) when the first argument was a command.
pub fn cli() -> Option<i32> {
    let cmd = std::env::args().nth(1)?;
    if !COMMANDS.contains(&cmd.as_str()) {
        return None;
    }
    Some(match send(&cmd) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("silas {cmd}: {e}");
            1
        }
    })
}

fn send(cmd: &str) -> std::io::Result<()> {
    let mut s = UnixStream::connect(socket())
        .map_err(|_| std::io::Error::other("Silas isn't running. Open it from the app launcher."))?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    writeln!(s, "{cmd}")?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    if reply.trim() != "ok" {
        return Err(std::io::Error::other(reply.trim().to_string()));
    }
    Ok(())
}
