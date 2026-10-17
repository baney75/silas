# Silas: agent notes

Tauri 2 app. Rust core in `src-tauri/src`, vanilla TypeScript UI in `src/`, contract in `docs/ipc.md`.

## Done means

```sh
bun run build && (cd src-tauri && cargo test --lib && cargo clippy --lib -- -D warnings)
```

iPad app (SwiftUI, FluidAudio) in `ios/`; see `ios/README.md`. Its check is
`(cd ios/SilasCore && swift test)`.

## Rules

- Never paste. Text goes out as keystrokes (`typer.rs`); the clipboard is never touched.
- Never auto-unload the model. Reload latency is the bug Silas exists to avoid.
- Fuzzy matching must not touch common English words (`dictionary.rs` guards with
  `data/common-en.txt`). Add a test to `common_words_are_never_bent_into_jargon` for any
  new matching rule.
- Stable releases are macOS only and come from `scripts/release-stable.sh` on Donovan's Mac
  (Apple Development signing keeps macOS permissions across updates). CI builds nightly only.
- Two updater keys. Nightly: `~/.tauri/silas.key`, also the `TAURI_SIGNING_PRIVATE_KEY` repo
  secret. Stable: `~/.tauri/silas-stable.key`, which never leaves the Mac. `updates.rs` pins each
  channel to its own public key. Never print, copy or commit either private key.
- Never call `set_status` (it waits on the main thread for the tray) while holding a lock, and
  never write `if let Some(x) = self.m.lock().take()`: the guard lives through the body.
- `[profile.release.build-override]` must stay: thin LTO otherwise breaks proc-macros.
- Benchmark engines with `cargo run --release --features bench --bin silas-bench -- parakeet <dir> <wav>...`.
