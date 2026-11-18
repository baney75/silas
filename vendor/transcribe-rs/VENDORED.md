# transcribe-rs 0.3.12, vendored with a TDT decoder fix

Silas's `parakeet-v3-onnx` fallback runs Parakeet TDT through this crate. The crates.io release reads
only the vocabulary half of the TDT joint's output and steps one frame at a time, so on a stutter it
types the same word again and again (accent WER 26.0% instead of 19.8% on the bench set,
`bench/models/accent-2026-11.md`). This copy honors the predicted duration, as NeMo's greedy decoder does.

Both `core/voice/Cargo.toml` and `src-tauri/Cargo.toml` point `[patch.crates-io]` here, and tests in
both crates name `TdtCursor`, which the crates.io release does not have, so dropping either patch fails
the build.

Made from the crates.io 0.3.12 tarball (git abc6f58 of cjpais/transcribe-rs):

1. Copied `Cargo.toml`, `build.rs`, `LICENSE` and `src/`. Left out `samples/`, `tests/` and `examples/`
   (4.8 MB of audio), and removed their targets from `Cargo.toml`.
2. Applied `bench/models/transcribe-rs-tdt-durations.patch`.
3. Moved the greedy step into `TdtCursor` in `src/onnx/parakeet/mod.rs`, with unit tests, so the fix
   can be tested without a model.
4. Replaced em and en dashes in comments and docs with hyphens (Silas's text rule), so a diff against
   upstream shows them: `grep -rlP '[\x{2013}\x{2014}]' src | xargs sed -i 's/\xe2\x80\x94/-/g; s/\xe2\x80\x93/-/g'`.

Drop this folder and both `[patch.crates-io]` entries once a transcribe-rs release has the fix.
