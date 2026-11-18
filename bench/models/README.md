# Model studies

Candidate speech models for Silas, scored on the bench set (`../README.md`) through Silas's own dictation path, so
every model is timed and scored the way the app would run it. Each study has a results page (`<run>.md`), its
numbers (`<run>.json`), the candidates file it was made from (`<run>.candidates.json`) and the per-clip rows
(`<run>/<id>.jsonl.gz`; `run.sh` writes `.jsonl`, and `gzip -n` it before committing so the rows stay small and out
of review diffs). Models and audio are not in git.

| Study | Question |
| --- | --- |
| [accent-2026-11](accent-2026-11.md) | Which free CPU model should be Silas's default, now that Hex's Parakeet Unified beats Parakeet v3 on accents |

## Run a study

```sh
# silas-bench (needs CMake and a C++ compiler: the app's engine crate builds transcribe.cpp from source)
(cd core/voice && cargo build --release -j3 --features bench --bin silas-bench)
bench/engines/build-whisper-cpp.sh                       # only for whisper:<name> candidates

# one model per argument: an app model id, gguf:<file>, or whisper:<ggml name>
bench/models/run.sh parakeet-v3-int8-onnx=parakeet-v3-onnx \
  parakeet-unified-en-q8=gguf:$HOME/.cache/silas-bench/models/gguf/parakeet-unified-en-0.6b-Q8_0/parakeet-unified-en-0.6b-Q8_0.gguf \
  whisper-small.en=whisper:small.en

python3 -I bench/models/summarize.py bench/models/accent-2026-11.candidates.json --out bench/models/accent-2026-11.json
```

`run.sh` runs `silas-bench run --fast --threads 3` on all 250 clips under `nice -n 10 taskset -c 1,2,3` (`CPUS`
changes the cores, `RUN` the output folder, `BIN` the silas-bench binary), one model at a time, and holds while the
1-minute load is 8 or more.

Since the study, `parakeet-v3` is the Q4_K_M GGUF on transcribe.cpp and the ONNX build is `parakeet-v3-onnx`. The
study's ONNX rows came from the stock transcribe-rs 0.3.12 (`parakeet-v3-int8-onnx`) and from a copy patched with
[transcribe-rs-tdt-durations.patch](transcribe-rs-tdt-durations.patch) (`parakeet-v3-int8-onnx-tdtfix`). The app
now carries that fix in `vendor/transcribe-rs`, so today's `parakeet-v3-onnx` is the `-tdtfix` row.

To score a change to an engine crate without touching the app's build, change a copy of the crate, build against it
into a separate target with `--config patch` (it wins over the `[patch.crates-io]` entry in `core/voice/Cargo.toml`),
and pass that binary as `BIN`:

```sh
cp -r vendor/transcribe-rs /tmp/transcribe-rs-try    # then edit /tmp/transcribe-rs-try
(cd core/voice && CARGO_TARGET_DIR=/tmp/target-try cargo build --release -j3 --features bench --bin silas-bench \
  --config 'patch.crates-io.transcribe-rs.path="/tmp/transcribe-rs-try"')
BIN=/tmp/target-try/release/silas-bench bench/models/run.sh parakeet-v3-onnx-try=parakeet-v3-onnx
```

`silas-bench run --model` takes, besides the app's own model ids:

- `gguf:<file.gguf>`: any transcribe.cpp model (Parakeet, Nemotron, Canary, Qwen3-ASR, Granite Speech, Whisper and
  more), loaded in process through the `transcribe-cpp` crate on its CPU backend with `--threads` threads (default
  the app's count, one per physical core up to 8; `run.sh` passes 3). The app runs Parakeet v3 on the same crate. English is passed as the
  language to models that take one.
- `whisper-server:<url>`: posts each piece to a running whisper.cpp server, as `bench/engines/whisper_cpp.py`
  does with whole clips. `--usage-pid <pid>` adds that server's CPU time and peak memory to each row.
- `parakeet-onnx:<dir>`: a full-precision Parakeet ONNX export (`encoder-model.onnx`, `decoder_joint-model.onnx`,
  `nemo128.onnx`, `vocab.txt`) through the app's own transcribe-rs engine, which the app runs at int8 as `parakeet-v3-onnx`.

Downloads go in their own folders under `~/.cache/silas-bench/models/gguf/<name>/`; check each against the Hub's
LFS sha256 (the candidates file records them) before a run.
