# Silas vs OpenCode's voice

Studied 2026-11-18. Every source claim links a file and line at a pinned commit:

- [anomalyco/opencode@1862f2ce](https://github.com/anomalyco/opencode/tree/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19) (`sst/opencode` redirects here)
- [anomalyco/hex@536cffef](https://github.com/anomalyco/hex/tree/536cffef8d5e508b92e50a31a562edd43803ab22) (Hex 3.4.0, Kit Langton's app, now under the OpenCode team)
- [kitlangton/Hex@3e300721](https://github.com/kitlangton/Hex/tree/3e300721b8784ea19ede0359d8fbcca94cfb6a04) (the old Swift Hex)

The numbers are in [opencode.json](opencode.json). `bench/rivals.py` checks them and the bench ratchet CI job runs it.

## Verdict

OpenCode's own voice is not dictation. It is a paid cloud voice call that steers OpenCode sessions, and it can't run free. So the bench scores Hex, the OpenCode team's free local dictation app, on the same 40 clips and the same machine as Silas.

Against Hex, Silas wins 5 of the 7 metrics scored and loses 2:

- Silas wins: punctuation, formatted error, setup steps, memory and CPU.
- Hex wins: word accuracy, by 0.1 point (5.4% against 5.3%), which the paired bootstrap calls a tie, and typing time.

Since 2026-11-18 Silas runs the same engine family as Hex: Parakeet TDT 0.6B v3, Q4_K_M GGUF, on transcribe.cpp ([accent study](../models/accent-2026-11.md)). That closed the accent gap: 10.7% WER against Hex's 14.8%, where the int8 ONNX build scored 29.5%. It cost code dictation: 29.8% WER, level with Hex's service run, where the ONNX build scored 21.1%.
- Not scored: time to first word and both waits after key up. Hex's times through `hex transcribe` come in whole steps of about 612 ms whatever the clip length (a 3.1 s clip and a 4.4 s clip both took 615 ms, a 122 s clip about 30 steps), so they measure the CLI's polling, not Hex's decoding. They stay unmeasured until Hex is timed through its local API.

The typing loss was the largest: Silas typed its text key by key at about 108 characters a second, while Hex pastes it in one step. Its typer now sends X11 keys straight through XTest at about 680 a second, so the median clip takes 110.6 ms to type instead of 349.9 ms; Hex still wins it, since a paste is one key.

## What OpenCode's voice is

| | OpenCode voice call | Hex (scored in its place) |
|---|---|---|
| What it does | A spoken conversation with an agent that lists, reads, creates and messages OpenCode sessions ([spec.ts:5-14](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/spec.ts#L5-L14), [spec.ts:42-106](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/spec.ts#L42-L106)) | Hold a key, speak, let go, and it pastes into the app you are in ([README.md:3-10](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L3-L10)) |
| Engine | `gpt-live-1` over WebRTC, voice `marin`, delegating tool calls to `gpt-6.1-sol` at low reasoning effort ([transport.ts:252-273](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/transport.ts#L252-L273)). The server also lists `gpt-realtime-2.1`, `grok-voice-3.0` and `gemini-3.8-live`, all cloud ([realtime.ts:34-49](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/ai/src/experimental/realtime.ts#L34-L49)) | Parakeet Unified English 0.6B, Q8_0 GGUF, 731 MB, on transcribe.cpp ([artifacts.rs:26-34](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/transcription/artifacts.rs#L26-L34), [models.rs:411-421](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/transcription/models.rs#L411-L421)). It is the default on an English system ([models.rs:1324-1345](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/transcription/models.rs#L1324-L1345)) |
| Local or cloud | Cloud. The call fails unless `gpt-live-1` is offered over WebRTC ([transport.ts:237-250](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/transport.ts#L237-L250)) | Local after the first model download ([README.md:10](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L10)) |
| Price | Paid. It needs an OpenAI API key or an OpenCode Console connection, and a ChatGPT sign-in is refused ([voice.ts:68-76](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/core/src/voice.ts#L68-L76), [voice.ts:155-164](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/core/src/voice.ts#L155-L164)). Hosted use is "billed to the workspace" ([voice.ts:136-137](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/core/src/voice.ts#L136-L137)). The source names no rate | Free, MIT ([README.md:110](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L110)); the model is a free Hugging Face download |
| Where it runs | The desktop app only, and not in the prod channel ([composer.tsx:55-56](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/composer.tsx#L55-L56)). It sits behind the `voice.calls` experiment, off by default ([composer.tsx:31-38](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/composer.tsx#L31-L38)). The TUI has no voice input | Mac, Windows and Linux ([README.md:3](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L3)) |
| Latency design | Streaming: a live duplex audio call ([transport.ts:35-37](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/transport.ts#L35-L37)) | Streaming on a GPU: "it writes while you talk" ([README.md:40-42](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L40-L42)). A CPU engine shows no words while you speak ([docs/linux.md:37](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L37)), so on a laptop without a working GPU it is batch after key up |
| Punctuation and formatting | Whatever the agent writes into a session. Your words are never shown as typed | The model's own punctuation and case. The Linux default mode adds no rewrite, only your word replacements ([processor.rs:518-531](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/dictation/processor.rs#L518-L531)). An OpenCode rewrite step is optional ([README.md:19-21](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L19-L21)) |
| How text reaches the app | It doesn't. The agent's tools send messages to OpenCode sessions only ([spec.ts:42-106](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/spec.ts#L42-L106)) | Clipboard then Ctrl+V, by `wtype` on wlroots, the RemoteDesktop portal on KDE and GNOME, and an X11 paster on X11 ([docs/linux.md:19-21](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L19-L21), [paste.rs:403-404](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L403-L404)). It waits 100 ms after the chord ([paste.rs:32](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L32), [paste.rs:644-646](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L644-L646)) and puts the old clipboard back after 500 ms ([paste.rs:41-43](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L41-L43)). Live typing through an input method is a Lab, off by default ([docs/linux.md:25-28](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L25-L28), [live_typing.rs:1-4](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/live_typing.rs#L1-L4)) |

Does OpenCode use Hex? No. OpenCode has no Hex dependency. It runs the other way: Hex can call a running OpenCode service to rewrite a dictation, and it never starts one itself ([opencode-compatibility.md:26-28](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/opencode-compatibility.md#L26-L28)). The old Swift Hex now only says the app moved to anomalyco/hex ([README.md:1-4](https://github.com/kitlangton/Hex/blob/3e300721b8784ea19ede0359d8fbcca94cfb6a04/README.md#L1-L4)).

## Setup steps on Linux, counted

- **Silas: 1.** Paste the installer line into a terminal; it installs Silas and opens it, and the model downloads by itself (README.md, "Install").
- **Hex: 2.**
  1. Run the install script ([docs/linux.md:220](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L220)).
  2. Open Hex from the launcher, since the script doesn't; its first run downloads the model ([docs/linux.md:223-225](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L223-L225)).

  On Wayland without the GlobalShortcuts portal (Sway, for one), binding the hotkey in the desktop's own config is a third step ([docs/linux.md:530-536](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L530-L536)).
- **OpenCode voice: 6, plus a paid account.**
  1. Install a desktop build that isn't prod.
  2. Turn on the Voice calls experiment in the developer Knobs panel.
  3. Give the server an OpenAI key or an OpenCode Console sign-in.
  4. Open a session.
  5. Click Call ([composer.tsx:67-79](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/composer.tsx#L67-L79)).
  6. Allow the microphone ([transport.ts:281-286](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/transport.ts#L281-L286)).

  The sources for each step are in `setup.opencode_voice_steps`.

The check reads Silas's count from README.md. It fails if the quoted install line is gone or the First run list gains an item.

## Where each fails

OpenCode voice:

- It can't type into any other app, so it can't replace dictation at all.
- It won't start without a paid key, and a ChatGPT plan doesn't count.
- It is hidden in prod builds and off by default.
- Linux never asks for the microphone ([window.ts:23-25](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/desktop/src/main/ipc-handlers/window.ts#L23-L25)), so a blocked mic shows up only as a failed call step ([transport.ts:286](https://github.com/anomalyco/opencode/blob/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19/packages/app/src/voice/transport.ts#L286)).
- It is useless offline.

Hex:

- It needs a Vulkan driver even for CPU inference ([docs/linux.md:11](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L11)) and quits on a CPU without AVX2 ([docs/linux.md:15](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L15)).
- On CPU it is batch, so a long dictation waits for the whole clip after key up. On the 122 s longform clip Hex took 18.4 s after key up; Silas took 0.45 s.
- Paste goes through the clipboard with a fixed wait and no acknowledgment ([paste.rs:41-43](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L41-L43)). A terminal needs the Terminal paste setting unless it takes Ctrl+V ([docs/linux.md:527-529](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L527-L529)).
- Code dictation: 29.8% WER, level with Silas now (the ONNX build Silas used before scored 22.8% in real time).
- Its two install docs give different script URLs ([README.md:83](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L83), [docs/linux.md:220](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L220)).
- It takes no pull requests ([README.md:107](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L107)).

## Results, chapel1 (Intel i5-10210U, no GPU used), ratchet subset (40 clips)

| Metric | Silas | Hex 3.4.0 | Winner | Gate |
|---|---|---|---|---|
| Time to first word, median (key down) | 6476.3 ms | 6371.0 ms | not scored (Hex by 105 ms) | latency |
| Wait after key up, median | 867.1 ms | 615.9 ms | not scored (Hex by 251 ms) | latency |
| Wait after key up, p95 | 1145.8 ms | 1228.7 ms | not scored (Silas by 83 ms) | latency |
| WER, pooled | 5.4% | 5.3% | **Hex** by 0.1 pt (a bootstrap tie) | accuracy |
| Punctuation F1 | 0.752 | 0.694 | **Silas** by 0.057 | accuracy |
| Formatted error | 17.5% | 18.0% | **Silas** by 0.6 pt | accuracy |
| Typing time into the app, median | 110.6 ms | 2.0 ms (modeled) | **Hex** by 109 ms | typing |
| Setup steps on Linux | 1 | 2 (OpenCode voice: 6) | **Silas** by 1 | setup |
| Peak memory | 1048 MB | 2162 MB | **Silas** by 1114 MB | latency |
| CPU s per audio minute | 20.2 | 54.9 | **Silas**, 2.7x less | latency |

Before the engine switch (int8 ONNX, recorded earlier on 2026-11-18): WER 8.2%, formatted error 19.8%, punctuation F1 0.725, peak memory 1213 MB, CPU 24.3 s per audio minute.

How each was measured:

- **Silas's accuracy** comes from its `--fast` run, the run CI repeats word for word. Its times come from a run paced in real time.
- **Hex** ran its pinned 3.4.0 release binary through `hex transcribe` against a warm `hex service`, and again one-shot with no service. Each metric keeps Hex's better run (`--rival` twice). The two differ in text: the service drops a final period on `clean-0011` that one-shot keeps.
- **Hex picked its CPU engine** on chapel1 ("GPU failed or ran slower"). So its file path is batch, and its time to first word is the clip length plus its wait.
- **Silas's first piece** arrives while you speak. Its median first word is still later than Hex's batch result because the median clip is about 6 s long.
- **Typing:** Silas's figure is measured by `xvfb-run -a -s "-screen 0 1280x1024x24 -noreset" silas-bench type`, its typer at the default "fast" pace on X11. Hex's figure is modeled, not measured: one Ctrl+V after a clipboard write, timed as one key event by the old typer. Hex's 100 ms settle runs after the paste, so it is not counted. Silas typed 4852 characters in 7.2 s, about 680 a second (it was 43.9 s, about 108 a second, before the typer wrote to XTest itself); the 122 s longform clip took 2.9 s to type (it was 24.6 s). Keys with no wait go at about 803 a second: the 40 clips have 87 doubled keys, and the typer waits 11 ms before the second press of each so Qt (10 ms) and SDL (2 ms) don't read it as autorepeat, about 1 s in all. Without `-noreset`, Xvfb resets each time the typer's connection closes and the next row waits for it, which adds about 36 ms a row.

### WER by category (lower is better)

| Category | Silas fast | Silas real time | Hex service | Hex one-shot |
|---|---|---|---|---|
| accent | 10.7% | 10.7% | 14.8% | 14.8% |
| clean | 2.2% | 2.2% | 1.1% | 1.1% |
| code | 29.8% | 29.8% | 29.8% | 33.3% |
| dictation | 6.5% | 7.3% | 4.8% | 3.2% |
| longform | 0.8% | 0.3% | 0.8% | 0.8% |
| noise | 4.8% | 4.8% | 3.4% | 2.7% |

Paired bootstrap, Hex against Silas real time ([report.json](opencode-chapel1/report.json)):

- Overall is a tie (Hex's service run 0.1 pt better, its one-shot run level).
- Every category is a tie against Hex's service run, including accent, where Silas is 4.1 pts ahead on 7 clips.
- Hex's one-shot run is better on dictation, by 4.0 pts.

Where Silas still loses: dictation (7.3% against 3.2% one-shot), noise (4.8% against 2.7%) and clean (2.2% against 1.1%) lean to Hex, typing is a paste against keystrokes, and the median wait after key up is 251 ms behind (not scored). Code is level at 29.8%, which is worse than Silas's old engine.

### What it would take to win

- **Typing:** Silas never pastes (AGENTS.md: text goes out as keystrokes and the clipboard is never touched), so typing stays key by key. It went from 349.9 ms to 110.6 ms a clip on X11 (104.2 ms on the ONNX text, which was shorter). On Wayland it types through `wtype`, which waits 2 ms after every press and every release and tops out near 210 characters a second; a Wayland virtual keyboard of Silas's own would lift that.
- **Dictation and noise WER:** Hex's Parakeet Unified English build still leads on these, a few points each. Accent no longer loses since the GGUF switch.
- **Code WER:** the ONNX build scored 21.1%; the GGUF build gave up 8.8 points there for the accent gain. Silas's custom words are the lever.
- **Wait after key up:** about 225 ms behind at the median, but ahead at p95, and far ahead on long dictation.

## The checks

`bench/rivals.py` recomputes Silas's numbers and fails when Silas's margin over Hex is worse than recorded by more than the metric's slack: a lead that shrinks or a loss that grows.

| Gate | What it covers | Where it runs |
|---|---|---|
| accuracy | WER (slack 1 pt), punctuation F1 (0.02), formatted error (1 pt) | CI, every PR that touches `core/voice` or `bench`, on the ratchet run's own rows |
| setup | The README still says the one-line install and First run has 3 items | CI |
| latency | Time to first word, both waits, memory, CPU | Only on the recorded CPU; skipped elsewhere, since shared runners are noisy |
| typing | `silas-bench type` | Only on the recorded CPU |

The committed rows are gzipped (`gzip -n`); the scripts read `.jsonl.gz` as is. To rerun on chapel1, set `S=~/.cache/silas-bench/bench-set-v1` and `D=bench/rivals/opencode-chapel1`, then:

```sh
silas-bench run --manifest bench/manifest.jsonl --set $S --subset bench/subsets/ratchet.txt --model parakeet-v3 --fast --out $D/silas-fast.jsonl
silas-bench run --manifest bench/manifest.jsonl --set $S --subset bench/subsets/ratchet.txt --model parakeet-v3 --out $D/silas-real-time.jsonl
xvfb-run -a -s "-screen 0 1280x1024x24 -noreset" silas-bench type --results $D/silas-real-time.jsonl --out $D/silas-typing.jsonl
uv run bench/engines/hex_engine.py --manifest bench/manifest.jsonl --set $S --subset bench/subsets/ratchet.txt --model parakeet_unified_en --out $D/hex-service.jsonl
uv run bench/engines/hex_engine.py --manifest bench/manifest.jsonl --set $S --subset bench/subsets/ratchet.txt --model parakeet_unified_en --one-shot --out $D/hex-one-shot.jsonl
python3 -I bench/rivals.py opencode --silas $D/silas-fast.jsonl --silas $D/silas-real-time.jsonl --typing $D/silas-typing.jsonl
gzip -n -f $D/*.jsonl
```

Add `--rival $D/hex-service.jsonl --rival $D/hex-one-shot.jsonl --record` to the last command to rewrite the file. It refuses when Silas fell behind, unless `--allow-worse` says the loss is known.

`hex_engine.py` hash-checks the release binary and runs Hex with HOME and the XDG folders inside `~/.cache/silas-bench/hex-home`. It never touches your own Hex settings, and it needs no account.
