//! The Polycarp voice engine: record, transcribe, type, learn. No UI toolkit.
//!
//! Interfaces implement [`core::Host`] to receive events and levels. The Tauri app
//! (Windows, Linux, Android) and the native Mac app share this crate.

pub mod audio;
pub mod benchmark;
pub mod core;
pub mod dictionary;
pub mod focus;
pub mod keys;
pub mod models;
pub mod permissions;
pub mod store;
pub mod typer;

use std::time::Duration;

use crate::core::Shared;

/// Read every key on its own thread and hand events to the core.
pub fn start_listener(core: Shared) {
    std::thread::spawn(move || {
        let mut warned = false;
        loop {
            match handy_keys::KeyboardListener::new() {
                Ok(listener) => {
                    log::info!("keyboard listener running");
                    warned = false;
                    core.set_listener(true);
                    loop {
                        match listener.recv_timeout(Duration::from_millis(500)) {
                            Ok(ev) => core.on_key(&ev),
                            Err(handy_keys::Error::Timeout) => core.tick(),
                            Err(e) => {
                                log::warn!("keyboard listener stopped: {e}");
                                break;
                            }
                        }
                    }
                    core.set_listener(false);
                }
                Err(e) => {
                    if !warned {
                        log::warn!("keyboard listener unavailable (permission?): {e}");
                        warned = true;
                    }
                    core.set_listener(false);
                    // Retry until the user grants access; no restart needed.
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });
}
