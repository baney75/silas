# Silas: agent notes

Tauri 2 app. Rust core in `src-tauri/src`, vanilla TypeScript UI in `src/`, contract in `docs/ipc.md`.

## Done means

```sh
bun run build && (cd src-tauri && cargo test --lib && cargo clippy --lib -- -D warnings)
```

## Rules

- Never paste. Text goes out as keystrokes (`typer.rs`); the clipboard is never touched.
- Never auto-unload the model. Reload latency is the bug Silas exists to avoid.
- Fuzzy matching must not touch common English words (`dictionary.rs` guards with
  `data/common-en.txt`). Add a test to `common_words_are_never_bent_into_jargon` for any
  new matching rule.
- Stable releases are macOS only and come from `scripts/release-stable.sh` on Donovan's Mac
  (Apple Development signing keeps macOS permissions across updates). CI builds nightly only.
- The updater key lives in `~/.tauri/silas.key` and the `TAURI_SIGNING_PRIVATE_KEY` repo
  secret. Never print it or commit it.
- `[profile.release.build-override]` must stay: thin LTO otherwise breaks proc-macros.
- Benchmark engines with `cargo run --release --features bench --bin silas-bench -- parakeet <dir> <wav>...`.
