# Bench harness handoff (WIP, paused 2026-11-17)

Paused by the lead's order: chapel1 is overloaded. From here on, every cargo build, test, engine build and
bench or WER run happens on abbey1 over ssh with a login shell (`ssh abbey1 'bash -lc "..."'`). chapel1 is for
git, editing and short checks only. Check `uptime` on abbey1 before each heavy step and hold while the 1-minute
load is above 10.

## Done (prepared; checked where noted)

- `core/voice/src/pieces.rs`: the worker's decisions moved into `Cutter` (`look_from`, `piece`, `add`) and the
  release-time join into `pieces::transcript`. `Done` gained `first` (when the first non-empty piece was
  ready). `core.rs` `write()` now calls `transcript`; same behavior.
- `core/voice/src/audio.rs`: `Recording::replay(samples, rate)` feeds a clip into the same buffer a microphone
  fills, 10 ms at a time in real time, then silence (the tail) until stopped.
- `core/voice/src/replay.rs` (new): `live` (real threads, real time, same `Pieces`, `TAIL` and `transcript`) and
  `simulated` (same `Cutter` on a simulated clock for `--fast`), plus `usage()` (getrusage CPU and peak RSS,
  Linux) and `energy_joules()` (RAPL; root-only on chapel1, so null).
  Checked on chapel1: `cd core/voice && cargo test` passed 122 tests, 4 of them new replay tests (before the
  last small edit: "first" now ignores empty pieces; rerun).
- `src-tauri/src/bin/silas-bench.rs`: `run` subcommand (`--manifest --model --out [--set --subset --fast --dict
  --keep-fillers]`), one JSON row per clip. Release build succeeded on chapel1; not yet run on a clip.
- `bench/fixtures/`: 6-clip fixture (`make_fixture.py` rebuilds it): 4 clean LibriSpeech clips referenced in
  `core/voice/bench`, one at 10 dB white noise, one 34 s longform clip joined by pauses.
- `bench/engines/`: `_common.py`, `build-whisper-cpp.sh` (tag v1.9.5, cmake from uvx if missing),
  `whisper_cpp.py` (whisper-server, model loads once), `moonshine.py` (useful-moonshine-onnx 20251121),
  `vosk_engine.py` (streaming on a simulated clock, like nerd-dictation). uv environments resolved on chapel1;
  no adapter has run on a clip yet.
- `bench/score.py` (stdlib), `bench/ratchet.py`, `bench/test_ratchet.py`. Checked: `python3 -I
  bench/test_ratchet.py` ran 12 tests, OK.
- `.github/workflows/bench-ratchet.yml`: builds silas-bench on ubuntu-latest and runs
  `ratchet.py --fast --gate accuracy`. Must change with the move below.
- Downloads on chapel1 under `~/.cache/silas-bench/`: ggml base.en, small.en, large-v3-turbo-q5_0; Vosk
  small-en-us-0.15 zip; Parakeet v3 unpacked in `silas-data/models/parakeet-v3` (archive SHA-256 matched
  models.rs). whisper.cpp v1.9.5 source cloned, not built. None of this is on abbey1 yet.

## Next

1. Move the runner into `core/voice` as a `[[bin]] silas-bench` behind its existing `bench` feature (no Tauri;
   src-tauri can't build on abbey1, which lacks webkit2gtk-4.1). Keep `src-tauri/src/bin/silas-bench.rs` as a
   thin wrapper or drop its `run` code; `parakeet|cohere|suite` stay there. Add `"load1"` (1-minute load from
   /proc/loadavg) to every row, in the Rust runner and in `_common.Writer`.
2. Point `ratchet.py`'s default binary and the CI workflow at the core/voice build
   (`core/voice/target/release/silas-bench`, no apt webkit packages needed).
3. On abbey1: copy `~/.cache/silas-bench/{models,silas-data}` over (or let the runner download Parakeet), then
   run the commands below.
4. The full set has landed at `~/.cache/silas-bench/bench-set-v1/` on chapel1 (and is being synced to abbey1);
   run it too, and add `bench/subsets/ratchet.txt` once its owner settles the subset.
5. Record the first ratchet numbers, show pass and injected-worse fail, run the AGENTS.md Done-means command,
   open the PR "Bench harness: score Silas and rival engines, ratchet regressions". Do not merge.

## Commands I was about to run (now on abbey1, after step 1)

```sh
uptime
cd core/voice && CARGO_BUILD_JOBS=3 nice cargo test
CARGO_BUILD_JOBS=3 nice cargo build --release --features bench --bin silas-bench
target/release/silas-bench run --manifest ../../bench/fixtures/manifest.jsonl --model parakeet-v3 --out ../../bench/out/silas.jsonl
target/release/silas-bench run --manifest ../../bench/fixtures/manifest.jsonl --model parakeet-v3 --out ../../bench/out/silas-fast.jsonl --fast
JOBS=3 bench/engines/build-whisper-cpp.sh
for m in base.en small.en large-v3-turbo-q5_0; do uv run bench/engines/whisper_cpp.py --manifest bench/fixtures/manifest.jsonl --model $m --out bench/out/whisper-$m.jsonl; done
uv run bench/engines/moonshine.py --manifest bench/fixtures/manifest.jsonl --model base --out bench/out/moonshine-base.jsonl
uv run bench/engines/vosk_engine.py --manifest bench/fixtures/manifest.jsonl --model small-en-us-0.15 --out bench/out/vosk.jsonl
python3 -I bench/score.py --manifest bench/fixtures/manifest.jsonl bench/out/silas.jsonl bench/out/whisper-*.jsonl bench/out/moonshine-base.jsonl bench/out/vosk.jsonl
python3 -I bench/ratchet.py --results bench/out/silas.jsonl --update   # writes bench/ratchet.json (none yet)
python3 -I bench/ratchet.py --results bench/out/silas.jsonl            # expect exit 0
# inject a worse result (edit one hypothesis) and expect exit 1
```

Lesson from chapel1: a wait loop of `pgrep -f "cargo build ..."` matches its own command line and never ends.
