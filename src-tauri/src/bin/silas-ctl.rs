//! `silas-ctl press|release|toggle|cancel`: what compositor keybindings run. Same as
//! `silas press`, without loading the GUI libraries first. Only std: it includes the engine's
//! `ctl.rs` and nothing else.

#[cfg(unix)]
#[path = "../../../core/voice/src/ctl.rs"]
mod ctl;

#[cfg(not(unix))]
fn main() {
    eprintln!("silas-ctl is for Linux and macOS");
    std::process::exit(2);
}

#[cfg(unix)]
fn main() {
    let code = ctl::cli().unwrap_or_else(|| {
        eprintln!("usage: silas-ctl {}", ctl::COMMANDS.join("|"));
        2
    });
    std::process::exit(code);
}
