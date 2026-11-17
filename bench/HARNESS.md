# Speech bench

One command scores any engine on the Silas speech test set, and a ratchet fails when Silas gets worse.

`core/voice/src/benchmark.rs` is the in-app suite that picks a model for each computer. This folder is the
outside bench: a bigger test set, the dictation path rather than the bare model, rival engines, and a report.

## Run it

```sh
python3 -I bench/tools/fetch.py        # the audio, into ~/.cache/silas-bench/bench-set-v1 (README.md)
SET=~/.cache/silas-bench/bench-set-v1
SUB="--subset bench/subsets/ratchet.txt"  # the 40 ratchet clips; leave it off for all 250

# Silas, through the dictation path, in real time (key down at the start of each clip, key up at its end)
(cd core/voice && cargo build --release --features bench --bin silas-bench)
core/voice/target/release/silas-bench run --manifest bench/manifest.jsonl --set $SET $SUB --model parakeet-v3 --out bench/out/silas.jsonl
# add --fast to skip the real-time pacing: model time stays real, the waiting is simulated

# rivals
bench/engines/build-whisper-cpp.sh
uv run bench/engines/whisper_cpp.py --manifest bench/manifest.jsonl --set $SET $SUB --model small.en --out bench/out/whisper-small.en.jsonl
uv run bench/engines/moonshine.py   --manifest bench/manifest.jsonl --set $SET $SUB --model base --out bench/out/moonshine-base.jsonl
uv run bench/engines/vosk_engine.py --manifest bench/manifest.jsonl --set $SET $SUB --model small-en-us-0.15 --out bench/out/vosk.jsonl

# report: bench/out/report.json and bench/out/report.html
python3 -I bench/score.py --manifest bench/manifest.jsonl bench/out/*.jsonl

# ratchet
python3 -I bench/ratchet.py            # fails on a regression
python3 -I bench/ratchet.py --update   # records the numbers, only if none got worse
```

The runner lives in `core/voice` (no Tauri, so it builds where the app cannot). Every runner takes `--set <dir>`
(audio paths are relative to it) and `--subset <ids.txt>` (one clip id per line). Engines, models and downloads
live in `~/.cache/silas-bench/`; Silas's model goes in `~/.cache/silas-bench/silas-data` (`$SILAS_BENCH_DATA`),
downloaded and hash-checked by the app's own `models::download` when missing. The test set itself, its format
and how it was built are in `README.md`.

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
| `load1` | the machine's 1-minute load average when the clip started; latency means little without it |

The rival adapters write the same rows. whisper.cpp and Moonshine are batch engines: they see the whole clip
after key up, so `t_first_ms = t_final_ms` = decode time. Vosk streams, so its adapter feeds 100 ms blocks on a
simulated clock the way nerd-dictation does and reports when the text was ready.

## Scoring

- **WER** on `ref_verbatim`, pooled over clips, scored on the model's own text (`hypothesis_raw`, before the
  dictionary drops the fillers `ref_verbatim` keeps). The default normalization, applied to the reference and
  every engine alike:
  1. Written symbols become the words spoken for them, since `ref_verbatim` spells out what was said: `$389`
     is "389 dollars", `3.2` "3 point 2", `12%` "12 percent", `silas.app/download` "silas dot app slash
     download", `a.m.` "am".
  2. `benchmark.rs`'s `normalize`: lowercase, accents folded, hyphens to spaces, letters, digits and
     apostrophes kept. Quote marks come off the ends of words.
  3. README.md's numbers rule: 10 and above in digits, zero to nine in words. LibriSpeech books spell "a girl
     of sixteen"; an app that types "a girl of 16" is not wrong. Spoken pairs count: "one twenty five" is 125,
     "two forty" is 240, "nineteen ninety" is 1990.

  `--whisper-norm` uses Whisper-style English normalization instead: every number in digits, British
  spellings, fillers dropped, `$` and `%`.
- **Formatted error** on `ref_formatted`: token errors with case, punctuation and line breaks counted, numbers
  under the same rule ("sixteen," and "16," match; "twenty-five" and "25" match).
- **Command accuracy**: the hypothesis must parse as JSON and equal the action list. An engine that
  produces no action lists (every engine today, Silas included) scores 0, and the report says so.
- Median and p95 of `t_first_ms` and `t_final_ms`, median RTF, peak memory, CPU seconds per audio minute
  (the battery proxy), energy per audio minute when RAPL is readable.
- Bootstrap 95% intervals for WER per category, and a paired bootstrap of each engine against Silas. A
  difference whose interval includes zero is reported as a tie; a category with fewer than 5 clips gets no verdict.

## Ratchet

`bench/ratchet.json` holds Silas's numbers on the 40 clips in `bench/subsets/ratchet.txt`: WER per category,
formatted error, median `t_final_ms`, peak memory. `bench/ratchet.py` fails when a category's WER or the
formatted error rises more than 0.5 points, median `t_final_ms` more than 15%, or peak memory more than 10%.
Latency is compared only between runs with the same timing mode. `--update` writes new numbers only when every
one is equal or better. CI (`.github/workflows/bench-ratchet.yml`) runs it with `--gate accuracy` on PRs
touching `core/voice/**` or `bench/**`; run the full gate locally.

The ratchet runs `silas-bench run --fast`, because it repeats word for word and real time does not. Parakeet's
text shifts a little with how much silence trails the clip. In real time that tail is the 200 ms the app waits
plus a few milliseconds of thread timing, and on abbey1 six of the 40 clips came out a word different between a
real-time run and a `--fast` run; two `--fast` runs matched on all 40. `--real-time` runs it paced anyway.

One limit remains: under `--fast` the pieces worker still waits on the model's real speed, so on a much slower or
faster machine a long clip can be cut at different pauses and its text can change by a word or two. Only the one
longform clip in the ratchet subset is long enough to be cut.
