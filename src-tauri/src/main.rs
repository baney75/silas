// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `silas press` and friends talk to the running app and exit without starting another.
    #[cfg(unix)]
    if let Some(code) = silas_lib::ctl::cli() {
        std::process::exit(code);
    }
    silas_lib::run()
}
