# Speech bench

One command scores any engine on the Silas speech test set, and a ratchet fails when Silas gets worse.

`core/voice/src/benchmark.rs` is the in-app suite that picks a model for each computer. This folder is the
outside bench: a bigger test set, the dictation path rather than the bare model, rival engines, and a report.

## Run it

```sh
# Silas, through the dictation path, in real time (key down at the start of each clip, key up at its end)
(cd src-tauri && cargo build --release --features bench --bin silas-bench)
src-tauri/target/release/silas-bench run --manifest bench/fixtures/manifest.jsonl --model parakeet-v3 --out bench/out/silas.jsonl
# add --fast to skip the real-time pacing: model time stays real, the waiting is simulated

# rivals
bench/engines/build-whisper-cpp.sh
uv run bench/engines/whisper_cpp.py --manifest bench/fixtures/manifest.jsonl --model small.en --out bench/out/whisper-small.en.jsonl
uv run bench/engines/moonshine.py   --manifest bench/fixtures/manifest.jsonl --model base --out bench/out/moonshine-base.jsonl
uv run bench/engines/vosk_engine.py --manifest bench/fixtures/manifest.jsonl --model small-en-us-0.15 --out bench/out/vosk.jsonl

# report: bench/out/report.json and bench/out/report.html
python3 -I bench/score.py --manifest bench/fixtures/manifest.jsonl bench/out/*.jsonl

# ratchet
python3 -I bench/ratchet.py            # fails on a regression
python3 -I bench/ratchet.py --update   # records the numbers, only if none got worse
```

Every runner takes `--set <dir>` (audio paths are relative to it; default the manifest's folder) and
`--subset <ids.txt>` (one clip id per line). Engines, models and downloads live in `~/.cache/silas-bench/`;
Silas's model goes in `~/.cache/silas-bench/silas-data` (`$SILAS_BENCH_DATA`), downloaded and hash-checked by
the app's own `models::download` when missing.

## Test set format

`manifest.jsonl`, one clip per line, 16 kHz mono 16-bit WAV:

```json
{"id": "clean-0001", "category": "clean|noise|accent|dictation|code|command|longform", "audio": "clean/clean-0001.wav",
 "seconds": 7.42, "ref_verbatim": "...", "ref_formatted": "...", "speaker": "...", "accent": null, "snr_db": null,
 "noise": null, "source": "...", "license": "...", "tts": null, "notes": null}
```

`ref_verbatim` is the spoken words in lowercase; `ref_formatted` is what an excellent app should type. For
`command` clips `ref_formatted` is a JSON-encoded action list.

`bench/fixtures/` is a 6-clip stand-in until the full set lands: four LibriSpeech clips bundled in
`core/voice/bench`, one with white noise at 10 dB SNR, and a 34-second clip of four joined by pauses so pieces
get cut while the speaker is talking. `python3 -I bench/fixtures/make_fixture.py` rebuilds it.

## What the Silas runner measures

`silas-bench run` replays each clip through the code a dictation runs (`core/voice/src/replay.rs`): the clip
is fed into the same `Recording` buffer a microphone fills, 10 ms at a time in real time; the `Pieces` worker
cuts and transcribes at pauses while the clip plays; release waits the same 200 ms tail, transcribes the rest
and joins it (`pieces::transcript`, shared with `core.rs`), then the dictionary runs (fillers removed, as the
app's default settings do; `--dict` loads a saved dictionary, `--keep-fillers` turns removal off). Typing is
not included.

One row per clip in `results.jsonl`:

| field | meaning |
| --- | --- |
| `hypothesis` | what the app would type |
| `hypothesis_raw` | the model's text before the dictionary |
| `t_first_ms` | key down to the first text available (the first piece, or the final text) |
| `t_final_ms` | key up to the final text: the wait users feel |
| `audio_seconds`, `compute_seconds`, `rtf` | time inside the model, and its share of the audio |
| `peak_rss_mb` | the process's peak resident memory so far |
| `cpu_seconds` | CPU time the clip took, all threads |
| `energy_j` | package energy from RAPL, null where it needs root |
| `load_ms` | model load, once per run, kept out of every other number |
| `pieces`, `paced` | pieces cut before release; false under `--fast` |

The rival adapters write the same rows. whisper.cpp and Moonshine are batch engines: they see the whole clip
after key up, so `t_first_ms = t_final_ms` = decode time. Vosk streams, so its adapter feeds 100 ms blocks on a
simulated clock the way nerd-dictation does and reports when the text was ready.

## Scoring

- **WER** on `ref_verbatim`, pooled over clips. The default normalization is `benchmark.rs`'s `normalize`
  (lowercase, hyphens to spaces, letters, digits and apostrophes kept). `--whisper-norm` adds Whisper-style
  English normalization on both sides: spelled numbers to digits, British spellings, fillers, `$` and `%`.
- **Formatted error** on `ref_formatted`: token errors with case and punctuation counted.
- **Command accuracy**: the hypothesis must parse as JSON and equal the action list. Engines with no
  command mode score 0, and the report says so.
- Median and p95 of `t_first_ms` and `t_final_ms`, median RTF, peak memory, CPU seconds per audio minute
  (the battery proxy), energy per audio minute when RAPL is readable.
- Bootstrap 95% intervals for WER per category, and a paired bootstrap of each engine against Silas. A
  difference whose interval includes zero is reported as a tie.

## Ratchet

`bench/ratchet.json` holds Silas's numbers on the ratchet clips (the fixture until `bench/subsets/ratchet.txt`
exists): WER per category, formatted error, median `t_final_ms`, peak memory. `bench/ratchet.py` fails when a
category's WER or the formatted error rises more than 0.5 points, median `t_final_ms` more than 15%, or peak
memory more than 10%. Latency is compared only between runs with the same timing mode. `--update` writes new
numbers only when every one is equal or better. CI (`.github/workflows/bench-ratchet.yml`) runs it with
`--fast --gate accuracy` on PRs touching `core/voice/**` or `bench/**`; run the full gate locally.
