# Silas: agent notes

Tauri 2 app. Rust core in `src-tauri/src`, vanilla TypeScript UI in `src/`, contract in `docs/ipc.md`.

## Done means

```sh
bun run build && bun run check:theme && bun test && (cd src-tauri && cargo test --lib && cargo clippy --lib -- -D warnings)
```

## Rules

- Never paste. Text goes out as keystrokes (`typer.rs`); the clipboard is never touched.
- Never auto-unload the model. Reload latency is the bug Silas exists to avoid.
- Fuzzy matching must not touch common English words (`dictionary.rs` guards with
  `data/common-en.txt`). Add a test to `common_words_are_never_bent_into_jargon` for any
  new matching rule.
- Stable releases are macOS only and come from `scripts/release-stable.sh` on Donovan's Mac
  (Developer ID Application, else Apple Development; a stable designated requirement keeps macOS
  permissions across updates). CI builds nightly only, ad-hoc unless the signing secrets exist.
  See `docs/signing.md`.
- Reading a permission never asks for it. Requests go through `PermissionGate` in
  `permissions.rs`: preflight first, never ask when granted, each prompt at most once per run.
- Two updater keys. Nightly: `~/.tauri/silas.key`, also the `TAURI_SIGNING_PRIVATE_KEY` repo
  secret. Stable: `~/.tauri/silas-stable.key`, which never leaves the Mac. `updates.rs` pins each
  channel to its own public key. Never print, copy or commit either private key.
- Never call `set_status` (it waits on the main thread for the tray) while holding a lock, and
  never write `if let Some(x) = self.m.lock().take()`: the guard lives through the body.
- `[profile.release.build-override]` must stay: thin LTO otherwise breaks proc-macros.
- Benchmark engines with `cargo run --release --features bench --bin silas-bench -- parakeet <dir> <wav>...`.
- Colours, radii, fonts and durations come from the theme (`src/theme-vars.ts`, applied by `src/theme.ts`). Write `var(--x)` in rule bodies; defaults live in the `:root` blocks. `bun run check:theme` fails on a hex, `rgb()` or named colour, a literal `border-radius` (other than `50%`), `font-family` or `transition` time (other than 0 and 1ms), and a colour given to a variable outside `:root`. The only exceptions are the files in `ALLOW` in `scripts/check-theme.mjs`, each with a reason.
- Theme logic that `bun test` runs (`theme-vars.ts`, `theme-boot.ts`) must not touch `window` or `document`. `bun test` and `cargo test --lib` both run the real `polycarp theme` command, so `cargo` must be on PATH.
