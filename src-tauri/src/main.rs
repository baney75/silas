// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `silas transcribe <file>`: the command line, with no window and no tray.
    if args.first().map(String::as_str) == Some("transcribe") {
        silas_lib::attach_console();
        std::process::exit(silas_lib::file::cli(&args[1..]));
    }
    silas_lib::run()
}
