//! `silas-ctl press|release|toggle|cancel`: what compositor keybindings run. Same as
//! `silas press`, without loading the GUI libraries first.

#[cfg(unix)]
#[path = "../ctl.rs"]
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
