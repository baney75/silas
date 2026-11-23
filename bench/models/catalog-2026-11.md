# Which models Silas should offer (2026-11)

Studied 2026-11-22 on chapel1 (Intel Core i5-10210U, 4 cores, 3 of them used, no GPU), the reference laptop of
[the accent study](accent-2026-11.md), and on parish1 (Intel Core i9-14900HX, 8 threads, no GPU). Numbers:
[catalog-2026-11.json](catalog-2026-11.json) and [catalog-2026-11-parish1.json](catalog-2026-11-parish1.json), made
by `summarize.py` from [catalog-2026-11.candidates.json](catalog-2026-11.candidates.json),
[catalog-2026-11-parish1.candidates.json](catalog-2026-11-parish1.candidates.json) and the per-clip rows (gzipped)
in `catalog-2026-11/` and `catalog-2026-11-parish1/`.

The candidates were the speech models handy-computer publishes as GGUF on Hugging Face, all of which
transcribe.cpp (Silas's engine since the accent study) loads: 18 besides the default, from NVIDIA, OpenAI,
Moonshine AI, Qwen, IBM, Mistral, Cohere and Moondream.

## Verdict

**The default stays Parakeet v3 (Q4_K_M GGUF).** No candidate wins across categories. Parakeet v3 has the fewest
errors with accents by a wide margin (12.0% against 14.3% for the next best), ties for the fewest overall (4.4%),
and is among the fastest (1.05 s median from key up to text on the reference laptop). Two English models beat it on code
and dictation clips (Parakeet v2 and Canary-Qwen), and both lose on accents. Nothing here argues for a switch.

| Use | Pick | Why |
| --- | --- | --- |
| Default | **Parakeet v3** (485 MB) | Best with accents, tied best overall, 25 languages, about 1 s wait |
| Smallest and fastest | **Moonshine Tiny** (35 MB) | 0.5 s wait, 194 MB of memory, 8 CPU seconds per audio minute; more than twice the errors (10.7%) |
| Small and fast, still accurate | **Canary 180M Flash** (218 MB) | 5.5% overall and 5.3% on dictation at half Parakeet's size, 0.9 s wait, 720 MB of memory |
| Most accurate (English) | **Canary-Qwen 2.5B** (1.7 GB) | Fewest errors with code (13.8%) and dictation (5.3%), no command errors, tied best overall; waits 4.1 s on the laptop |
| Most languages | **Whisper Large v3 Turbo** (536 MB) | 99 languages and 5.5% in English, but a 13 s wait on the laptop: for a fast computer or files |
| Languages Parakeet lacks, at dictation speed | **Qwen3-ASR 0.6B** (590 MB) | 30 languages plus 22 Chinese dialects, 15.3% with accents (second only to Parakeet v3 among small models), 2.0 s wait |
| English only | **Parakeet v2** (475 MB) | Ties v3 overall (4.4%), best fast model on code (14.7%) and better on dictation (6.1%); worse with accents (16.5%) |
| Streaming (live text) | **None added** | Silas already shows text while you speak by transcribing each pause-cut piece, with any model. The streaming models run offline in transcribe.cpp and scored worse than their offline peers (Nemotron Streaming 7.9%, Moonshine Streaming Small 7.3% and Medium 6.3% overall, against 4.4% to 5.5% for the kept offline models other than Moonshine Tiny; Voxtral Realtime was accurate but waited 18 s) |

Kept from before: **Parakeet v3 (ONNX)** for x86 processors below the GGUF engine's floor (AVX2, FMA, F16C), where
Settings hides every GGUF model, and **Cohere Transcribe**
(int4 ONNX, 2 GB), whose 14 languages and code accuracy (13.8%) no GGUF build matched (below).

That is 9 models. The registry's device report holds 10 results, so one slot is left; the case for filling it is
under "Dropped".

## Dropped

| Candidate | Why |
| --- | --- |
| Parakeet Unified English 0.6B (Hex's default) | Parakeet v2 is as accurate overall and better on accent (16.5 against 18.6), code (14.7 against 23.2) and punctuation (F1 0.763 against 0.650). Its NVIDIA Open Model License allows commercial use but ends it for anyone who removes a safety guardrail built into the model without an equivalent one, a condition the CC-BY models don't carry |
| Nemotron 3.5 ASR Streaming 0.6B | The weakest NVIDIA model here: 7.9% overall, 23.0% accent, 30.1% code |
| Canary 1B v2 | Parakeet v3's 25 languages with more errors in every category but command and longform (5.4% overall, 19.6% accent) and half again the wait |
| Whisper small, Whisper base | Small: Canary 180M has fewer errors overall (5.5% against 7.5%) and waits a sixth as long (0.9 s against 5.4 s). Base: Moonshine Tiny is as accurate (10.7% against 10.4%) at less than half the size and wait |
| Moonshine Streaming Small, Medium | Worse than Canary 180M at a similar size (7.3% and 6.3% against 5.5%), English only |
| Qwen3-ASR 1.7B | Better than the 0.6B (4.6% overall, 14.3% accent, 20.7% code) but twice the wait (3.9 s), memory (1.9 GB) and download (1.3 GB). It is the best case for the tenth slot: a more accurate choice in the 30 languages for a fast computer. Left out so the list stays short and the report keeps a slot free; add it if people ask |
| Voxtral Mini 4B Realtime (Mistral) | Run on the first 61 clips only (the 60 clean clips and one noise clip; the rest would have taken over an hour), rows in `catalog-2026-11/voxtral-4b-realtime-q4km.partial.jsonl.gz`. On those clips it was as accurate as Parakeet v3 (2.0% WER each), but its median wait was 17.8 s against Parakeet v3's 1.0 s and Whisper Turbo's 13.1 s, it peaked at 3.1 GB of memory, used 423 CPU seconds per audio minute (Parakeet v3: 20) and is a 2.8 GB download. Too slow for dictation on a laptop CPU whatever the remaining clips show |
| Granite Speech 5.0 470M (IBM) | Writes no punctuation at all (F1 0.000) and has the most dictation errors (14.7%) |
| Cohere Transcribe 2B, Q4_K_M GGUF | Would replace the int4 ONNX build if it matched it, and it doesn't on code (19.8% against 13.8%) or dictation (9.2% against 8.4%). It does use far less memory (2.6 GB against 6.0 GB on parish1) and waits less, so it is worth a second look on a later engine |
| Parakeet Ultra (Moondream) | Moondream is not one of the labs this catalog accepts (NVIDIA, OpenAI, Mistral, IBM, Cohere, Qwen, Moonshine AI); not run |

## Results on all 250 clips, reference laptop (chapel1)

WER in percent on the verbatim reference (fillers kept, `score.py`'s default), on the model's own text before Silas
formats it. Command clips are scored as words. Wait is `t_final_ms`, from key up to final text, simulated by
`--fast` from real model time (the accent study checked `--fast` against real time for transcribe.cpp models). CPU
is process CPU seconds per minute of audio over the 3 cores. Size is the download.

| Candidate | All | clean | noise | accent | dictation | code | command | longform | Fmt err | Punct F1 | Wait p50 | Wait p95 | Peak MB | CPU s/min | Size MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Parakeet TDT 0.6B v3, Q4_K_M (Silas's default) | 4.4 | 1.9 | 2.3 | 12.0 | 7.3 | 20.7 | 1.9 | 2.5 | 13.6 | 0.754 | 1049 | 1682 | 1060 | 20.1 | 485 |
| Parakeet TDT 0.6B v2, Q4_K_M | 4.4 | 1.5 | 2.6 | 16.5 | 6.1 | 14.7 | 1.9 | 2.1 | 13.9 | 0.763 | 1024 | 1640 | 1020 | 19.4 | 475 |
| Parakeet Unified English 0.6B, Q4_K_M | 4.7 | 1.7 | 2.2 | 18.6 | 5.6 | 23.2 | 1.9 | 1.8 | 15.7 | 0.650 | 1142 | 1850 | 1004 | 21.8 | 477 |
| Nemotron 3.5 ASR Streaming 0.6B, Q4_K_M | 7.9 | 3.5 | 6.6 | 23.0 | 7.8 | 30.1 | 1.9 | 4.5 | 20.1 | 0.588 | 1319 | 2525 | 1082 | 29.1 | 496 |
| Canary 1B v2, Q4_K_M | 5.4 | 2.2 | 2.7 | 19.6 | 8.6 | 21.9 | 0.0 | 2.4 | 13.8 | 0.758 | 1553 | 2477 | 1565 | 30.8 | 735 |
| Canary 180M Flash, Q8_0 | 5.5 | 2.1 | 3.2 | 22.4 | 5.3 | 26.7 | 5.7 | 1.8 | 15.3 | 0.747 | 888 | 1689 | 720 | 14.9 | 218 |
| Canary-Qwen 2.5B, Q4_K_M | 4.4 | 2.0 | 3.3 | 16.0 | 5.3 | 13.8 | 0.0 | 2.0 | 15.3 | 0.677 | 4128 | 7044 | 2748 | 92.8 | 1738 |
| Whisper large-v3-turbo, Q4_K_M | 5.5 | 2.5 | 3.0 | 19.9 | 9.7 | 23.8 | 3.8 | 2.1 | 19.3 | 0.544 | 13148 | 21665 | 722 | 248.3 | 536 |
| Whisper small, Q8_0 | 7.5 | 3.4 | 5.6 | 24.6 | 9.7 | 26.0 | 1.9 | 3.8 | 16.4 | 0.707 | 5391 | 17099 | 493 | 103.1 | 270 |
| Whisper base, Q8_0 | 10.4 | 5.7 | 10.7 | 28.2 | 9.4 | 31.7 | 3.8 | 6.2 | 19.8 | 0.671 | 1418 | 2009 | 236 | 26.4 | 85 |
| Moonshine Tiny, Q8_0 | 10.7 | 5.0 | 13.4 | 30.6 | 7.0 | 34.2 | 11.3 | 5.5 | 21.8 | 0.669 | 520 | 1166 | 194 | 8.4 | 35 |
| Moonshine Streaming Small, Q8_0 | 7.3 | 2.2 | 7.2 | 24.3 | 6.6 | 26.7 | 1.9 | 3.5 | 20.3 | 0.618 | 750 | 1515 | 603 | 14.5 | 199 |
| Moonshine Streaming Medium, Q8_0 | 6.3 | 1.7 | 5.4 | 22.2 | 5.5 | 28.5 | 1.9 | 2.8 | 19.5 | 0.627 | 1765 | 6790 | 906 | 38.5 | 296 |
| Qwen3-ASR 0.6B, Q4_K_M | 5.1 | 2.2 | 3.1 | 15.3 | 5.8 | 23.2 | 0.0 | 2.9 | 15.8 | 0.708 | 2004 | 3641 | 1218 | 46.5 | 590 |
| Qwen3-ASR 1.7B, Q4_K_M | 4.6 | 1.8 | 2.6 | 14.3 | 6.1 | 20.7 | 0.0 | 2.5 | 15.1 | 0.722 | 3875 | 6542 | 1919 | 86.0 | 1320 |
| Granite Speech 5.0 470M TurboCTC, Q4_K_M | 7.6 | 2.2 | 2.3 | 25.3 | 14.7 | 36.0 | 1.9 | 4.0 | 29.6 | 0.000 | 1518 | 3229 | 478 | 22.4 | 279 |

The default's row matches [the accent study](accent-2026-11.md) (4.4% overall, 12.0% accent, 1063 ms wait there).
Its ONNX build is in that study, on the same machine, set and path: 5.8% overall, 19.8% accent. Cohere Transcribe's int4 ONNX build measured 4.6% overall, 20.2% accent, 13.8% code, a 4534 ms
median wait, 2821 MB peak and 92.3 CPU s per audio minute there; its GGUF build ran on parish1 only (below).

Model load and machine load per run (chapel1 is shared; `run.sh` holds while the 1-minute load is 8 or more,
which bounds the start of each piece, not the whole run):

| Candidate | Load ms | 1-min load, median | 1-min load, max |
| --- | ---: | ---: | ---: |
| Parakeet TDT 0.6B v3, Q4_K_M (Silas's default) | 571 | 3.2 | 3.9 |
| Parakeet TDT 0.6B v2, Q4_K_M | 377 | 3.0 | 3.4 |
| Parakeet Unified English 0.6B, Q4_K_M | 595 | 4.4 | 6.3 |
| Nemotron 3.5 ASR Streaming 0.6B, Q4_K_M | 455 | 5.0 | 11.8 |
| Canary 1B v2, Q4_K_M | 709 | 3.1 | 3.7 |
| Canary 180M Flash, Q8_0 | 139 | 5.6 | 7.0 |
| Canary-Qwen 2.5B, Q4_K_M | 777 | 3.1 | 5.2 |
| Whisper large-v3-turbo, Q4_K_M | 49 | 3.1 | 5.9 |
| Whisper small, Q8_0 | 49 | 8.9 | 28.5 |
| Whisper base, Q8_0 | 76 | 5.8 | 7.0 |
| Moonshine Tiny, Q8_0 | 62 | 6.5 | 6.9 |
| Moonshine Streaming Small, Q8_0 | 45 | 3.7 | 4.8 |
| Moonshine Streaming Medium, Q8_0 | 42 | 8.5 | 14.2 |
| Qwen3-ASR 0.6B, Q4_K_M | 177 | 3.2 | 14.0 |
| Qwen3-ASR 1.7B, Q4_K_M | 132 | 3.1 | 3.4 |
| Granite Speech 5.0 470M TurboCTC, Q4_K_M | 129 | 15.0 | 20.3 |

Load is the time `silas-bench` took to load the model before the first clip (one run each, not a cold-start
measurement). Runs
with a median 1-minute load above 4 (three of 4 cores busy is 3) shared the CPU with other work, and their wait and
CPU numbers are high by an unknown amount: Granite, Whisper small, Moonshine Streaming Medium, Moonshine Tiny,
Whisper base, Canary 180M, Nemotron and Parakeet Unified. None of those is close enough to a kept model for that to change a verdict, and two of
them (Moonshine Tiny, Canary 180M) were kept despite it.

The 95% intervals (clip bootstrap, in the JSON) overall: Parakeet v3 3.5 to 5.5, Canary-Qwen 3.5 to 5.7, Qwen3 1.7B
3.7 to 5.8, Parakeet Unified 3.7 to 6.2; on accent: Parakeet v3 9.6 to 15.1, Qwen3 1.7B 11.7 to 17.9, Qwen3 0.6B
12.6 to 18.5, Canary-Qwen 13.2 to 19.2. The top four overall are not separable on this set. On accent, Parakeet v3's interval overlaps those of
the two Qwen3 models, Canary-Qwen and Parakeet v2 (13.7 to 19.6), and clears every other model's.

## The same runs on parish1

Accuracy nearly matches: every model's overall WER is within 0.12 points of chapel1's, and every category within
0.7 (Whisper small on accent). The gap is the replay, not the models: before #74, `--fast` cut pieces by the
machine's real compute time, so the two machines cut a few clips differently. The text quotes chapel1. parish1 is a shared desktop (median 1-minute load 9 to 49 over these runs, other builds
running; `MAXLOAD=20`), so its timings rank the models only loosely; on it Qwen3 0.6B waited longer than Qwen3 1.7B. It adds
the two Cohere builds, which were too large for chapel1's free memory alongside its other work.

| Candidate | All | clean | noise | accent | dictation | code | command | longform | Fmt err | Punct F1 | Wait p50 | Wait p95 | Peak MB | CPU s/min | Size MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Parakeet TDT 0.6B v3, Q4_K_M (Silas's default) | 4.4 | 1.9 | 2.3 | 12.0 | 7.3 | 20.7 | 1.9 | 2.5 | 13.5 | 0.755 | 479 | 831 | 997 | 19.4 | 485 |
| Parakeet TDT 0.6B v2, Q4_K_M | 4.4 | 1.5 | 2.6 | 16.5 | 6.1 | 14.7 | 1.9 | 2.1 | 13.9 | 0.766 | 644 | 1180 | 919 | 27.3 | 475 |
| Parakeet Unified English 0.6B, Q4_K_M | 4.8 | 1.7 | 2.2 | 18.6 | 5.6 | 23.2 | 1.9 | 1.9 | 15.7 | 0.651 | 757 | 4057 | 916 | 42.0 | 477 |
| Nemotron 3.5 ASR Streaming 0.6B, Q4_K_M | 7.8 | 3.5 | 6.6 | 23.0 | 7.8 | 30.1 | 1.9 | 4.3 | 20.0 | 0.593 | 675 | 1112 | 970 | 32.6 | 496 |
| Canary 1B v2, Q4_K_M | 5.4 | 2.2 | 2.7 | 19.6 | 8.6 | 21.9 | 0.0 | 2.4 | 13.7 | 0.760 | 2838 | 10930 | 1361 | 146.6 | 735 |
| Canary 180M Flash, Q8_0 | 5.4 | 2.1 | 3.2 | 22.4 | 5.3 | 26.7 | 5.7 | 1.8 | 15.2 | 0.748 | 1813 | 3811 | 644 | 94.9 | 218 |
| Canary-Qwen 2.5B, Q4_K_M | 4.4 | 2.0 | 3.2 | 16.3 | 5.3 | 13.8 | 0.0 | 2.0 | 15.3 | 0.678 | 2274 | 4340 | 2534 | 151.8 | 1738 |
| Whisper small, Q8_0 | 7.4 | 3.2 | 5.7 | 23.9 | 9.7 | 26.0 | 1.9 | 3.8 | 16.3 | 0.704 | 2627 | 9013 | 438 | 146.0 | 270 |
| Whisper base, Q8_0 | 10.3 | 5.8 | 10.6 | 28.0 | 8.9 | 31.7 | 3.8 | 6.2 | 19.8 | 0.669 | 1192 | 2568 | 217 | 69.4 | 85 |
| Moonshine Tiny, Q8_0 | 10.7 | 5.0 | 13.3 | 30.6 | 7.0 | 34.2 | 11.3 | 5.6 | 21.9 | 0.668 | 388 | 904 | 193 | 11.4 | 35 |
| Moonshine Streaming Small, Q8_0 | 7.3 | 2.2 | 7.2 | 24.3 | 6.6 | 26.7 | 1.9 | 3.5 | 20.2 | 0.619 | 2162 | 4233 | 582 | 106.5 | 199 |
| Moonshine Streaming Medium, Q8_0 | 6.3 | 1.7 | 5.4 | 22.2 | 5.5 | 28.5 | 1.9 | 2.8 | 19.7 | 0.621 | 3141 | 7927 | 841 | 154.8 | 296 |
| Qwen3-ASR 0.6B, Q4_K_M | 5.1 | 2.0 | 3.4 | 15.2 | 5.8 | 23.2 | 0.0 | 2.9 | 16.0 | 0.699 | 4529 | 14334 | 1170 | 207.3 | 590 |
| Qwen3-ASR 1.7B, Q4_K_M | 4.6 | 1.8 | 2.6 | 14.4 | 6.2 | 20.7 | 0.0 | 2.4 | 15.0 | 0.723 | 2233 | 5837 | 1890 | 137.4 | 1320 |
| Granite Speech 5.0 470M TurboCTC, Q4_K_M | 7.6 | 2.2 | 2.3 | 25.3 | 14.7 | 36.0 | 1.9 | 4.0 | 29.6 | 0.000 | 537 | 805 | 495 | 20.5 | 279 |
| Cohere Transcribe 2B, Q4_K_M GGUF | 4.8 | 1.0 | 1.2 | 20.1 | 9.2 | 19.8 | 0.0 | 2.0 | 15.9 | 0.630 | 1891 | 5210 | 2638 | 96.1 | 1558 |
| Cohere Transcribe 2B, int4 ONNX (Silas's catalog today) | 4.5 | 1.0 | 1.3 | 20.2 | 8.4 | 13.8 | 0.0 | 1.8 | 16.4 | 0.619 | 2950 | 6519 | 6035 | 113.9 | 2000 |

Whisper Large v3 Turbo's parish1 run was stopped at 231 of 250 clips and is left out; Voxtral and Parakeet v3 ONNX
were not run there.

## Languages

All of these are scored on English. For the five models that take other languages, a German and a Mandarin
sentence from Tatoeba (CC BY 2.0 FR) went through Silas's engine with the language set, with "Detect", and with
English:

| Model | German, set | German, Detect | Mandarin, set | Mandarin, Detect |
| --- | --- | --- | --- | --- |
| Parakeet v3 | correct | correct | not a language it has | not a language it has |
| Canary 180M Flash | correct | translated to English | not a language it has | not a language it has |
| Qwen3-ASR 0.6B | correct | correct | correct | correct |
| Whisper Large v3 Turbo | correct | correct | correct | correct |
| Cohere Transcribe | correct | correct | correct | half English |

Parakeet v3 ignores the language it's given and always writes what it hears. Set to English, Whisper translated
both sentences into English, Canary the German, Qwen3 the Mandarin (it kept the German); Cohere kept the German. So
the language setting matters for every model but Parakeet v3. Canary and Cohere can't detect a language, so with
"Detect" Silas gives them English; both taglines now tell people to choose their language in Settings. Settings
now lists 27 languages and Detect, where it listed 8; a language not listed still works with Detect on Qwen3 or
Whisper.

Canary 180M writes German, French and Spanish with a space before each comma and period ("nur , wenn du
mitkommst ."). Silas now removes that space outside English, except before French `?`, `!`, `;` and `:`, which
French spaces on purpose (`attach_punctuation` in `core/voice/src/models.rs`, with a test). English output is
untouched: of the 250 clips' raw text across every model here, one Moonshine Tiny code clip had the pattern.

## Trust

Every kept model comes from a known lab under a license that allows commercial use. Each new one is a single GGUF
file (tensors and metadata, no pickled code) pinned to one 40-character revision and checked against the Hub's Git
LFS SHA-256 before Silas loads it; each was downloaded and matched on 2026-11-22. The two ONNX entries were already
pinned the same way. A unit test (`every_catalog_entry_is_pinned_to_a_revision_and_hashes`)
fails if an entry loses its revision or hash; another (`every_catalog_entry_meets_the_trust_rules`) checks the lab,
license and license link, and `every_gguf_model_is_the_build_the_study_scored` ties each entry to its row in the
candidates file.

| Model | Lab | Base model | License | Repo, revision |
| --- | --- | --- | --- | --- |
| Parakeet TDT 0.6B v3, Q4_K_M (Silas's default) | NVIDIA | [nvidia/parakeet-tdt-0.6b-v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) | [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/) | [handy-computer/parakeet-tdt-0.6b-v3-gguf](https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf) `d8d8c32cb201` |
| Parakeet TDT 0.6B v2, Q4_K_M | NVIDIA | [nvidia/parakeet-tdt-0.6b-v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) | [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/) | [handy-computer/parakeet-tdt-0.6b-v2-gguf](https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v2-gguf) `58239cf6d0ec` |
| Canary 180M Flash, Q8_0 | NVIDIA | [nvidia/canary-180m-flash](https://huggingface.co/nvidia/canary-180m-flash) | [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/) | [handy-computer/canary-180m-flash-gguf](https://huggingface.co/handy-computer/canary-180m-flash-gguf) `8b01fc77ac16` |
| Moonshine Tiny, Q8_0 | Moonshine AI | [UsefulSensors/moonshine-tiny](https://huggingface.co/UsefulSensors/moonshine-tiny) | [MIT](https://opensource.org/license/mit) | [handy-computer/moonshine-tiny-gguf](https://huggingface.co/handy-computer/moonshine-tiny-gguf) `c71a93b8f1cb` |
| Qwen3-ASR 0.6B, Q4_K_M | Qwen (Alibaba) | [Qwen/Qwen3-ASR-0.6B](https://huggingface.co/Qwen/Qwen3-ASR-0.6B) | [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) | [handy-computer/Qwen3-ASR-0.6B-gguf](https://huggingface.co/handy-computer/Qwen3-ASR-0.6B-gguf) `7e7c69a577fa` |
| Whisper large-v3-turbo, Q4_K_M | OpenAI | [openai/whisper-large-v3-turbo](https://huggingface.co/openai/whisper-large-v3-turbo) | [MIT](https://opensource.org/license/mit) | [handy-computer/whisper-large-v3-turbo-gguf](https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf) `18ada752005f` |
| Canary-Qwen 2.5B, Q4_K_M | NVIDIA | [nvidia/canary-qwen-2.5b](https://huggingface.co/nvidia/canary-qwen-2.5b) | [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/) | [handy-computer/canary-qwen-2.5b-gguf](https://huggingface.co/handy-computer/canary-qwen-2.5b-gguf) `a4480da236b7` |
| Parakeet v3, int8 ONNX | NVIDIA | [nvidia/parakeet-tdt-0.6b-v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) | [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/) | Silas's own release `models-v1`, SHA-256 pinned |
| Cohere Transcribe 2B, int4 ONNX | Cohere | [CohereLabs/cohere-transcribe-03-2026](https://huggingface.co/CohereLabs/cohere-transcribe-03-2026) | [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) | [cstr/cohere-transcribe-onnx-int4](https://huggingface.co/cstr/cohere-transcribe-onnx-int4) `2c870f89692c` |

Each GGUF repo's license matches its base model's on the Hub (Cohere Transcribe's base repo is gated; its Hub
metadata says Apache-2.0, as does the int4 ONNX conversion Silas uses, `cstr/cohere-transcribe-onnx-int4`). Whisper base and small are Apache-2.0 on the Hub, Whisper Large
v3 Turbo MIT. The NVIDIA Open Model License (Parakeet Unified) and OpenMDW-1.1 (Nemotron) both allow commercial
use; neither model was kept, on measurements.

## Method

- Every model goes through Silas's dictation path (`core/voice/src/replay.rs`): pieces cut at pauses while the clip
  plays, the 200 ms tail, the rest at key up, the default dictionary. `silas-bench run --fast`, one model at a time.
- The runs used the dictation path as of 0302bdc. Since then #69 skips the tail when the speaker has already stopped
  and transcribes at pauses before key up, and #74 makes `--fast` cut pieces the same way on any machine. Both
  shorten every model's wait and change a few clips' text; every model here ran on the same older path, so the
  comparison stands, but the waits above are higher than today's app gives.
- chapel1: `--threads 3` under `nice -n 10 taskset -c 1,2,3` (`run.sh`'s defaults), as in the accent study.
  parish1: `--threads 8` on one thread of each performance core (`CPUS=0,2,4,6,8,10,12,14 THREADS=8 MAXLOAD=20`),
  the app's own thread count. No GPU on either: parish1's GPU was in use.
- Models above 1 GB ran one at a time after a free-memory check; the largest (Canary-Qwen, Qwen3 1.7B, Voxtral) ran
  on chapel1 as well as or instead of parish1.
- English is passed to models that take a language. Nothing is warmed beyond one second of silence.
- Every GGUF was checked against its LFS SHA-256 and kept in its own folder; nothing downloaded was run from inside
  its folder.
- The engine test `every_catalog_model_transcribes_a_clip` loads each kept model through Silas's own `Engine` and
  transcribes a bench clip; it skips unless `SILAS_TEST_MODELS` names a data folder, and it ran on parish1 with all 9
models in one, each giving the clip's text.

## Reproduce

See [README.md](README.md). `run.sh` takes `RUN`, `CPUS`, `THREADS` and `MAXLOAD`; the candidates files list every
model, file, revision and hash, and `summarize.py` writes the JSON.
