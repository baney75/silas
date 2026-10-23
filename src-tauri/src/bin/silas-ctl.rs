//! `silas-ctl press|release|toggle|cancel`: what compositor keybindings run. Same as
//! `silas press`, without loading the GUI libraries first.

#[path = "../ctl.rs"]
mod ctl;

fn main() {
    let code = ctl::cli().unwrap_or_else(|| {
        eprintln!("usage: silas-ctl {}", ctl::COMMANDS.join("|"));
        2
    });
    std::process::exit(code);
}
