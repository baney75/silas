# Silas: agent notes

Tauri 2 app. The voice engine (audio, keys, typing, dictionary, permissions, models) is the
`polycarp-voice` crate in `core/voice`, with no UI toolkit; the Tauri host is `src-tauri/src`;
vanilla TypeScript UI in `src/`; contract in `docs/ipc.md`. Polycarp's apps share the same engine,
so keep `core/voice` free of Tauri.

## Done means

```sh
bun install --frozen-lockfile && bun run build && bun run check:theme && bun test && (cd core/voice && cargo test) && (cd src-tauri && cargo test --lib && cargo clippy --lib -- -D warnings)
```

## Rules

- Never paste. Text goes out as keystrokes (`core/voice/src/typer.rs`); the clipboard is never touched.
- Never auto-unload the model. Reload latency is the bug Silas exists to avoid.
- Fuzzy matching must not touch common English words (`core/voice/src/dictionary.rs` guards with
  `core/voice/data/common-en.txt`). Add a test to `common_words_are_never_bent_into_jargon` for any
  new matching rule.
- Channels are stable, nightly and dev (`docs/channels.md`). `promote.yml` checks a promotion and
  builds the Windows and Linux drafts; `scripts/release-stable.sh` on Donovan's Mac moves `stable`,
  tags `vX.Y.Z`, builds and signs the Mac app (Developer ID Application, else Apple Development; a stable
  designated requirement keeps macOS permissions across updates), signs every stable update and
  publishes. No workflow starts on a pushed tag, and none may name a stable signing secret
  (`scripts/check-channels.sh`). See `docs/signing.md`.
- Reading a permission never asks for it. Requests go through `PermissionGate` in
  `core/voice/src/permissions.rs`: preflight first, never ask when granted, each prompt at most once per run.
- Two updater keys. Nightly and dev: `~/.tauri/silas.key`, also the `TAURI_SIGNING_PRIVATE_KEY` repo
  secret. Stable: `~/.tauri/silas-stable.key`, which never leaves the Mac. `updates.rs` pins each
  channel to its own public key. Never print, copy or commit either private key.
- Never call `set_status` (it waits on the main thread for the tray) while holding a lock, and
  never write `if let Some(x) = self.m.lock().take()`: the guard lives through the body.
- `[profile.release.build-override]` must stay: thin LTO otherwise breaks proc-macros.
- Benchmark Silas with `silas-bench run` in `core/voice` (`bench/HARNESS.md`); `src-tauri`'s
  `silas-bench parakeet <dir> <wav>...` runs only the ONNX fallback.
- Parakeet v3, the default model, runs on transcribe.cpp (the `transcribe-cpp` crate), built from source
  with CMake. `.cargo/config.toml` turns off `GGML_NATIVE` so a release built on a CI runner runs on other
  CPUs, and `models::runs_here` keeps CPUs below that floor on `parakeet-v3-onnx`; keep both. The ONNX
  models run on the vendored transcribe-rs (`vendor/transcribe-rs/VENDORED.md`), patched in both
  `core/voice` and `src-tauri`.
- Colours, radii, fonts and durations come from the theme (`src/theme-vars.ts`, applied by `src/theme.ts`). Write `var(--x)` in rule bodies; defaults live in the `:root` blocks. `bun run check:theme` fails on a hex, `rgb()` or named colour, a literal `border-radius` (other than `50%`), `font-family` or `transition` time (other than 0 and 1ms), and a colour given to a variable outside `:root`. The only exceptions are the files in `ALLOW` in `scripts/check-theme.mjs`, each with a reason.
- Theme logic that `bun test` runs (`theme-vars.ts`, `theme-boot.ts`) must not touch `window` or `document`. `bun test` and `cargo test --lib` load the built-in themes from `tests/fixtures/themes`, the files Polycarp's `theme set` writes; refresh them with `node scripts/theme-fixtures.mjs --refresh` (needs a `polycarp` command on PATH or `POLYCARP_BIN`) when Polycarp's theme format changes.
