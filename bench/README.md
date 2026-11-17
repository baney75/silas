# Bench set v1

A fixed speech test set that Silas and rival dictation apps are scored on: 250 clips,
62.2 minutes of 16 kHz mono 16-bit PCM WAV, each with a verbatim and a formatted
reference. Audio is not in git. It lives in the `bench-set-v1` release of this repo
and is checked file by file against `bench/checksums.sha256`.

```sh
python3 -I bench/tools/fetch.py              # download and verify into ~/.cache/silas-bench/bench-set-v1
python3 -I bench/tools/check.py              # counts, minutes, format and duration of every clip
python3 -I bench/tools/fetch.py --rebuild    # regenerate everything from the original sources instead
```

Both take `--dest DIR`. `fetch.py` and `check.py` need only Python 3 (3.14, or any
Python 3 with the `zstd` command on PATH). `--rebuild` needs `uv` and downloads about
2 GB of source data into `~/.cache/silas-bench/`.

## Contents

| Category | Clips | Minutes | What it tests |
| --- | ---: | ---: | --- |
| clean | 60 | 7.8 | LibriSpeech test-clean read speech, 30 speakers, one short (3 to 7 s) and one medium (7 to 15 s) clip each |
| noise | 60 | 10.8 | 30 of the clean clips mixed with real cafe, street, keyboard and fan noise at 10 dB and 0 dB SNR |
| accent | 50 | 7.0 | Conversational English from EdAcc, 10 accents, 5 clips and 2 speakers each |
| dictation | 30 | 4.2 | Emails, messages and notes as people speak them to a computer |
| code | 25 | 2.2 | Code and terminal dictation |
| command | 15 | 0.5 | Editing commands a dictation app should act on |
| longform | 10 | 29.7 | 2 to 5 minute passages: 5 LibriSpeech chapters, 5 synthesized documents |
| total | 250 | 62.2 | |

### Where the audio came from

| Category | Audio | Made on |
| --- | --- | --- |
| clean | real recordings: LibriSpeech test-clean (LibriVox readers) | copied unchanged on chapel1 |
| noise | real recordings: LibriSpeech test-clean mixed with real DEMAND and FSD50K noise | mixed on chapel1 |
| accent | real recordings: EdAcc conversations | resampled on chapel1 |
| dictation | TTS: Kokoro-82M, 8 voices | chapel1 (CPU); dictation-0006 and 0021 re-rendered on abbey1 (CPU) |
| code | TTS: Kokoro-82M, 8 voices | chapel1 (CPU); code-0005, 0009 and 0020 re-rendered on abbey1 (CPU) |
| command | TTS: Kokoro-82M, 8 voices | chapel1 (CPU); command-0001 and 0009 re-rendered on abbey1 (CPU) |
| longform-0001 to 0005 | real recordings: LibriSpeech test-other | joined on chapel1 |
| longform-0006 to 0010 | TTS: Kokoro-82M | rendered on chapel1 (CPU) |

No audio in v1 came from Parish1 or any GPU. The first build ran on chapel1. When chapel1
got overloaded, the remaining heavy steps moved to abbey1: the ASR proxy check, the 7 clips
re-rendered after it, and the release tarball, built from a copy of the chapel1 audio that
matched `checksums.sha256` byte for byte. Each synthesized clip's `tts` field names its
engine, voice and host, for example `kokoro-82m af_heart chapel1`.

`bench/subsets/ratchet.txt` lists 40 clips from every category, 5.6 minutes in all,
for a fast regression check.

### Accents

| Code | EdAcc label | Code | EdAcc label |
| --- | --- | --- | --- |
| `ng` | Nigerian English | `ke` | Kenyan English |
| `in` | Indian English | `jm` | Jamaican English |
| `scot` | Scottish English | `zh` | Chinese (Mandarin L1) |
| `ie` | Irish English | `es` | Spanish L1 |
| `uk` | Southern British English | `vi` | Vietnamese L1 |

Synthesized clips use `us` or `uk` by voice. LibriSpeech clips have `accent: null`:
LibriVox readers are mostly American but the corpus does not label accent.

## Manifest

`bench/manifest.jsonl` has one JSON object per clip with exactly these keys:

| Key | Meaning |
| --- | --- |
| `id` | `<category>-NNNN` |
| `category` | `clean`, `noise`, `accent`, `dictation`, `code`, `command` or `longform` |
| `audio` | WAV path relative to the set root |
| `seconds` | duration, 3 decimals |
| `ref_verbatim` | the spoken words: lowercase, no punctuation, numbers as words, fillers kept |
| `ref_formatted` | what an excellent dictation app should type (rules below); for `command`, a JSON action list as a string |
| `speaker` | `librispeech-<reader id>`, the EdAcc speaker id, or `kokoro-<voice>` |
| `accent` | code from the table above, or `null` |
| `snr_db` | `10` or `0` for noise clips, else `null` |
| `noise` | `cafe`, `street`, `keyboard` or `fan` for noise clips, else `null` |
| `source` | where the audio came from, precise enough to find it again |
| `license` | license of that clip |
| `tts` | `<engine> <voice> <host>` for synthesized clips (speed is always 1.0), else `null` |
| `notes` | free text, or `null` |

### ref_verbatim

Lowercase words separated by single spaces. Apostrophes inside words stay (`don't`,
`'cause`); hyphens split words; accents fold (`café` becomes `cafe`). Fillers and
disfluencies are kept exactly as spoken (`um`, `uh`, `mm hmm`, cut-off words in EdAcc).
LibriSpeech and EdAcc use the official transcripts; synthesized clips use the text the
voice was given.

### ref_formatted

Written by hand, not from a model transcript. The rules, so a scorer can be fair:

- Sentence case and normal punctuation. Line breaks appear only where the speaker said
  "new line" or "new paragraph" (`\n` and `\n\n`); unspoken paragraphs are not guessed.
- Spoken punctuation becomes the mark: "comma", "period", "question mark", "exclamation
  point", "colon".
- Fillers are removed (`um`, `uh`, `mm hmm`, `uh huh`), as are stutters (immediate exact
  repeats) and cut-off words. Explicit self-corrections are applied: "Tuesday, no,
  Wednesday" becomes "Wednesday", likewise "actually make that", "sorry, I mean".
  Unmarked restarts in conversational EdAcc speech are kept, and so is the speaker's own
  grammar.
- Numbers: digits for money (`$47.50`, `$2,400`), percentages (`12%`), times (`3:30`,
  `10 AM`, `4 PM`), dates (`March 4`, `the 22nd`), years, versions (`3.2`), addresses,
  phone numbers (`555-867-5309`), measurements (`3.5 pounds`, `7 feet`), config and code
  values, and any number 10 or above. Zero to nine stay words in running prose ("two
  weeks", "three books").
- Email addresses, URLs, file names and paths are written as text (`jane.porter@example.com`,
  `silas.app/download`, `config.toml`).
- Code follows the spoken formatter words ("camel case user name" is `userName`, "snake
  case", "pascal case", "kebab case", "all caps") and common spacing (`a += b`,
  `[1, 2, 3]`). Library and tool names use their own spelling: TypeScript, Kubernetes,
  PostgreSQL, Tauri, ONNX, kubectl, nginx.
- LibriSpeech clips start from the LibriSpeech-PC punctuated text with quotation marks
  removed (a dictation app cannot hear them), dashes turned into commas, and "mister"
  written as "Mr.". Book spelling stays (`practise`, `thro`).

### Command actions

`ref_formatted` for `command` clips is a JSON list, applied in order:

| Action | Fields | Meaning |
| --- | --- | --- |
| `insert` | `text` | type this text |
| `newline` | | line break |
| `new_paragraph` | | blank line |
| `delete_last` | | delete the last dictated phrase ("delete that", "scratch that") |
| `delete` | `unit`, `direction`, `count` | delete words before the cursor |
| `select` | `unit`, `direction`, `count` | select words before the cursor |
| `select_all` | | select everything in the field |
| `capitalize` | `target: "last"` | capitalize the last phrase or the selection |
| `uppercase` | `target: "last"` | all caps for the last phrase or the selection |
| `undo` | | undo the last edit |
| `move` | `to: "line_end"` | move the cursor |
| `key` | `key` | press a key ("press enter" submits; it is not a typed line break) |

Two command clips mix dictation and commands (`command-0014`, `command-0015`); their
`notes` give the final text.

## How each category was made

All selection is seeded (`20261117`) and frozen in `bench/sources/selection.json`;
`bench/tools/select_sources.py` made it once and the build never reruns it.

- **clean**: 30 of the 40 test-clean readers, one short and one medium utterance each,
  only from utterances where LibriSpeech-PC's punctuated text has the official words.
- **noise**: every second clean clip (30 clips, one per reader) mixed at 10 dB and 0 dB.
  Noise types rotate cafe, street, keyboard, fan. SNR is whole-clip speech power over
  noise power for the exact noise segment used; the offset into the noise file is fixed
  in `selection.json`; short noise clips loop; a mix that would clip is scaled down as a
  whole, which keeps the SNR. Cafe is DEMAND PCAFETER, street is DEMAND STRAFFIC (each from
  channel 1 or 9), keyboard and fan are CC0 FSD50K recordings (see LICENSES.md).
- **accent**: EdAcc test split, segments of 12 to 35 words with no `<laugh>`,
  `<overlap>` or other tags, 3 to 20 seconds long, from the two speakers per accent with
  the most such segments. These are Zoom conversations, so expect real disfluency and
  some room and line noise.
- **dictation, code, command**: written in `bench/sources/synth.toml`, voiced with
  Kokoro-82M (kokoro-onnx 0.4.9, CPU, 3 threads) in 8 voices (`af_heart`, `af_bella`,
  `am_michael`, `am_fenrir` US; `bf_emma`, `bf_isabella`, `bm_george`, `bm_fable` UK),
  one sentence at a time with 0.25 to 0.5 s pauses, 0.25 s lead and 0.35 s tail,
  normalized to -22 dBFS RMS. Half the dictation clips use spoken punctuation; the other
  half do not. A small `[respell]` table fixes TTS pronunciation ("A M" is voiced "ay em",
  "onnx" is voiced "onyx") without changing the reference, and an item's optional `speak`
  text replaces `say` for the TTS only, when a clip needs different wording to be voiced
  right.
- **longform**: `longform-0001` to `0005` join 16 to 34 consecutive LibriSpeech test-other
  utterances from one chapter with 0.35 s gaps (about 2.2 to 4.8 minutes; consecutive
  utterances still skip some book text between them). `longform-0006` to `0010` are a
  long email, meeting notes with spoken punctuation, a short essay, a status report and a
  personal letter, voiced like the dictation clips.

## Deviations from the plan

- No Southern US or Australian accent: EdAcc's test split has neither, and VCTK (which has
  Australian readers) was not added to keep one conversational source. Ten other accents
  are covered.
- The longest LibriSpeech passage is 4.8 minutes, not 6: no test-other chapter has a
  longer unbroken run of utterances with punctuated text.
- One test-other chapter (5764-299665, a polemical essay against religion) was skipped for
  longform; any other chapter tests the model as well.
- All synthesized speech is Kokoro on chapel1. Parish1's Qwen3-TTS was not used: its GPU
  was busy (`parish-media status` reported `not_ready`) and the host was short of memory.
  No second TTS engine was added; voice variety comes from 8 Kokoro voices.

## Adding Qwen3-TTS clips later

Qwen3-TTS runs on Parish1 through `parish-media voice`. Before every session, check all
three, and do not start if any fails:

- `ssh parish1 free -m` shows at least 6 GB (6144 MiB) in the `available` column.
- `ssh parish1 uptime` shows a 1-minute load under 8.
- `ssh parish1 ~/.local/bin/parish-media status` shows no `not_ready` field.

Never stop other GPU work or other processes to free it.

1. Add items to `bench/sources/synth.toml` under new ids (keep existing ids stable; new
   clips make a new set version, `bench-set-v2`, with its own tag).
2. Voice each `say` text, one call per item (1500 characters at most), for example:
   `ssh parish1 '~/.local/bin/parish-media voice -- --mode custom --speaker Ryan --text "..." --output /home/baney75/.local/share/parish-media/outputs/dictation-0031.wav'`.
   Use `--speaker Aiden` for a second voice, or `--mode design` with an `--instruct`
   accent description for more variety.
3. Copy the WAVs back with `scp`, resample to 16 kHz mono 16-bit with `sources.to_16k`
   and `sources.to_int16`, and add a build step that reads them from
   `~/.cache/silas-bench/src/qwen3/` the way `build_synth` writes Kokoro output. Set
   `tts` to `qwen3-tts Ryan parish1` (engine, voice, host) and `speaker` to `qwen3-<voice>`.
4. Run `asr_check.py --all` on the new clips and fix any over 15% WER, then rebuild,
   pack and publish as in the next section.

## Building and publishing

```sh
OMP_NUM_THREADS=3 nice uv run bench/tools/build.py --pack     # audio, manifest, checksums, tarball
python3 -I bench/tools/make_subsets.py                         # ratchet subset
python3 -I bench/tools/check.py
OMP_NUM_THREADS=3 nice uv run bench/tools/asr_check.py         # proxy listen check, 15 random synthesized clips
gh release create bench-set-v1 --latest=false ~/.cache/silas-bench/bench-set-v1.tar.zst
```

`--latest=false` matters: stable Silas updates (`src-tauri/src/updates.rs`) and
`install.sh` read `releases/latest/download/...`, so a bench release that took "Latest"
would break every stable install's updater. Keep it a normal release (not a pre-release)
and check afterward that `gh api repos/baney75/silas/releases/latest --jq .tag_name`
still prints the newest app version.

Then record the asset URL, size and sha256 in `bench/release.json`. `build.py --only synth`
or `--only real` rebuilds half the set and keeps the other manifest rows; `--ids a,b` re-renders only
those synthesized clips. `build.py
--pack-only` repacks the audio already in the cache after a manifest edit.

Rebuilt LibriSpeech, noise and EdAcc clips match the release bit for bit. Kokoro output
depends on the CPU and onnxruntime build, so rebuilt synthesized clips can differ in bytes
while keeping the same words and durations; `fetch.py --rebuild` reports those as a note,
not a failure.

## Tools

| Script | Runs with | Does |
| --- | --- | --- |
| `tools/fetch.py` | `python3 -I` | download the release, verify sha256 per file; `--rebuild` regenerates |
| `tools/check.py` | `python3 -I` | counts, minutes, keys, WAV format, durations, subsets |
| `tools/build.py` | `uv run` | build audio, manifest, checksums, tarball from `sources/` |
| `tools/select_sources.py` | `uv run` | the one-off seeded selection behind `sources/selection.json` |
| `tools/sources.py` | imported | source URLs, downloads, safe extraction, WAV IO |
| `tools/make_subsets.py` | `python3 -I` | write `subsets/ratchet.txt` |
| `tools/asr_check.py` | `uv run` | faster-whisper small.en WER against `ref_verbatim` |

Downloaded archives are treated as untrusted data: tar members go through the `data`
filter, zip members are checked for path escapes, and nothing inside them is executed.
