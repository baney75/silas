//! The command end of the control socket (see `control.rs`). Only std, so the `silas-ctl`
//! binary skips loading the app's GUI libraries and a key press costs about a millisecond
//! instead of 30.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const COMMANDS: &[&str] = &["press", "release", "toggle", "cancel"];

pub fn socket() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("silas.sock")
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
