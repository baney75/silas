# Silas vs OpenCode's voice

Studied 2026-11-18; Hex's timing and the word errors redone 2026-11-22. Every source claim links a file and line at a pinned commit:

- [anomalyco/opencode@1862f2ce](https://github.com/anomalyco/opencode/tree/1862f2ce5c4ac5ec9b47b4aea85a5412f8e23a19) (`sst/opencode` redirects here)
- [anomalyco/hex@536cffef](https://github.com/anomalyco/hex/tree/536cffef8d5e508b92e50a31a562edd43803ab22) (Hex 3.4.0, Kit Langton's app, now under the OpenCode team)
- [kitlangton/Hex@3e300721](https://github.com/kitlangton/Hex/tree/3e300721b8784ea19ede0359d8fbcca94cfb6a04) (the old Swift Hex)

The numbers are in [opencode.json](opencode.json). `bench/rivals.py` checks them and the bench ratchet CI job runs it.

## Verdict

OpenCode's own voice is not dictation. It is a paid cloud voice call that steers OpenCode sessions, and it can't run free. So the bench scores Hex, the OpenCode team's free local dictation app, on the same 40 clips and the same machine as Silas.

Against Hex, Silas wins 6 of the 7 metrics scored and loses 1:

- Silas wins: word accuracy, by one word in 925 (5.19% against 5.30%, a bootstrap tie), punctuation, formatted error, setup steps, memory and CPU.
- Hex wins: typing, 62.5 ms against 110.6 ms from the final text to the text in the field. Hex's figure is modeled from its paste code and now counts the 60 ms it sleeps before Ctrl+V; the 2026-11-18 model said 2.0 ms.
- Not scored, with reasons: time to first word and both waits after key up. Hex is now timed poll free through its own local API, the way its app finishes a dictation, but chapel1 stayed busy all day and `rivals.py` refuses times from a busy machine. Paired clip by clip on the busy machine, Silas finished first on all 40 clips. On the quiet 2026-11-18 run, Hex's polled times put a ceiling on its wait, and that ceiling (815.9 ms) is under Silas's median (867.1 ms), so on a quiet machine Hex leads the median wait by at least 51 ms. The p95 and time to first word can't be called from a ceiling.

Since 2026-11-18 Silas runs the same engine family as Hex: Parakeet TDT 0.6B v3, Q4_K_M GGUF, on transcribe.cpp ([accent study](../models/accent-2026-11.md)). That closed the accent gap: 10.7% WER against Hex's 14.8%, where the int8 ONNX build scored 29.5%. It cost code dictation: 29.8% WER against Hex's 33.3% one-shot and 26.3% through its app path, where the ONNX build scored 21.1%.

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
- On CPU it is batch, so a long dictation waits for the whole clip after key up. On the 122 s longform clip Hex took 18.4 s after key up on a quiet machine (polled, so at most that); Silas took 0.45 s. On a busy one, 47.6 s against 3.4 s.
- Paste goes through the clipboard with a fixed wait and no acknowledgment ([paste.rs:41-43](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L41-L43)). A terminal needs the Terminal paste setting unless it takes Ctrl+V ([docs/linux.md:527-529](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L527-L529)).
- Code dictation: 33.3% WER one-shot and 26.3% through its app path, against Silas's 29.8% (the ONNX build Silas used before scored 22.8% in real time).
- Its two install docs give different script URLs ([README.md:83](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L83), [docs/linux.md:220](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/docs/linux.md#L220)).
- It takes no pull requests ([README.md:107](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/README.md#L107)).

## Timing Hex

What Hex's app does after key up on a CPU, at the pinned commit:

- **Nothing reaches the model while the key is held.** Parakeet streams only on a GPU unless `HEX_STREAMING=1` is set ([gguf_session.rs:259-273](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/transcription/gguf_session.rs#L259-L273)), so on a CPU no live session opens.
- **At key up it keeps recording a 200 ms tail** ([dictation.rs:111-113](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/dictation.rs#L111-L113)), longer only while speech carries on past the release, and waits for that audio ([release_tail.rs:84-92](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/dictation/release_tail.rs#L84-L92)). Silas waits the same 200 ms.
- **Then it transcribes the whole clip once** ([engine.rs:720-735](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/transcription/engine.rs#L720-L735)), padded to at least 1.5 s with 400 ms of trailing silence for Unified English ([transcription.rs:170-176](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/transcription.rs#L170-L176)).

`hex_engine.py` replays exactly that. Each clip plays in real time with the key held, then the adapter waits the 200 ms tail and sends the clip plus that tail of quiet room to the warm service's own local API, `POST /transcriptions`, in one request. The wait after key up is from key up to the answer. The request crosses localhost as a WAV, a few milliseconds the app doesn't spend, so the figure is Hex's wait plus that.

**Why `hex transcribe` came in 612 ms steps.** While a transcription runs, the service's handler waits up to 100 ms for the result ([transcriptions.rs:83-99](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/local_api/transcriptions.rs#L83-L99), [local_api.rs:52](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/local_api.rs#L52)), then checks whether the client went away by peeking its socket ([local_api.rs:1103-1118](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/local_api.rs#L1103-L1118)). The socket has a 500 ms read timeout ([local_api.rs:49](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/local_api.rs#L49), [local_api.rs:568](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/local_api.rs#L568)), and a client that keeps its socket open, as `hex transcribe` does, sends nothing more, so every peek waits the full 500 ms. A result that lands during a peek waits for it to end, so answers come in steps of about 600 ms; a 122 s file goes in 30 s pieces ([transcribe.rs:6-9](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/control/cli/transcribe.rs#L6-L9)), each rounded up. The adapter half-closes its socket once the WAV is sent. A peek then sees end of file at once, which the handler treats as a client still listening ([local_api.rs:1106-1108](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/local_api.rs#L1106-L1108)), so the answer comes when the model is done. `--cli` keeps the old measurement; its rows are `hex-cli.jsonl.gz`.

`/streams` is not the app's path: it cuts the audio into windows at pauses and transcribes each as it closes, so its finish does less work than the app's key up. The app's own `/dictations` records the microphone itself and takes no audio.

### On a busy machine

chapel1 is shared: fleet CI and a 20-model catalog run that skips the bench lock kept load1 near 10 on its 8 threads on 2026-11-22. Silas and Hex were timed clip by clip there anyway, back to back under one lock in alternating order, so each pair saw the same load (load1 median 9.5 for Silas's clips, 10.5 for Hex's, whose own threads add to it). Rows in [under-load/](opencode-chapel1/under-load/):

| | Silas | Hex 3.4.0 |
|---|---|---|
| Wait after key up, median | 1214 ms | 3565 ms |
| Wait after key up, p95 | 2574 ms | 6462 ms |
| Time to first word, median | 7112 ms | 9795 ms |
| Longform (122 s) wait | 3.4 s | 47.6 s |
| CPU s per audio minute | 41.6 | 151.7 |

Silas finished first on all 40 clips, at a median 2.7 times sooner. On a quiet machine the gap is far smaller (see Results): load costs Hex much more, for two reasons. On a CPU Hex leaves transcribe.cpp at its default of up to 8 spin-waiting worker threads, which its own source says "stall it under load" on GPU machines, where it cuts them to one ([gguf_session.rs:90-101](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/transcription/gguf_session.rs#L90-L101)); Silas runs 4 threads on this 4-core CPU. And all of Hex's decoding happens after key up, inside the wait, while Silas decodes a long clip piece by piece as you speak. These rows are not the recorded figures; a busy machine would set the check's bar where a quiet run could never miss it.

## Results, chapel1 (Intel i5-10210U, no GPU used), ratchet subset (40 clips)

| Metric | Silas | Hex 3.4.0 | Winner | Gate |
|---|---|---|---|---|
| Time to first word, median (key down) | 6476.3 ms | at most 6571.0 ms (quiet); 9795.1 ms busy, against Silas's 7111.7 | not scored: busy machine; the quiet ceiling can't call it | latency |
| Wait after key up, median | 867.1 ms | at most 815.9 ms (quiet); 3565.3 ms busy, against Silas's 1214.3 | not scored: busy machine; **Hex** on a quiet machine by at least 51 ms | latency |
| Wait after key up, p95 | 1145.8 ms | at most 1428.7 ms (quiet); 6461.5 ms busy, against Silas's 2574.3 | not scored: busy machine; the quiet ceiling can't call it | latency |
| WER, pooled | 5.19% | 5.30% | **Silas** by one word (a bootstrap tie) | accuracy |
| Punctuation F1 | 0.752 | 0.694 | **Silas** by 0.057 | accuracy |
| Formatted error | 17.2% | 18.0% | **Silas** by 0.8 pt | accuracy |
| Final text to text in the field, median | 110.6 ms | 62.5 ms (modeled; was 2.0) | **Hex** by 48 ms | typing |
| Setup steps on Linux | 1 | 2 (OpenCode voice: 6) | **Silas** by 1 | setup |
| Peak memory | 1048 MB | 2162 MB | **Silas** by 1114 MB | latency |
| CPU s per audio minute | 20.2 | 54.9 | **Silas**, 2.7x less | latency |

Silas's times, memory and CPU are from its quiet 2026-11-18 run; Hex's memory and CPU from its quiet 2026-11-18 service run. Accuracy is from 2026-11-22. Before the engine switch (int8 ONNX, recorded earlier on 2026-11-18): WER 8.2%, formatted error 19.8%, punctuation F1 0.725, peak memory 1213 MB, CPU 24.3 s per audio minute.

How each was measured:

- **Silas's accuracy** comes from its `--fast` run of today's main, the run CI repeats word for word. Its times come from a run paced in real time.
- **Hex** ran its pinned 3.4.0 release binary three ways: through its local API as its app finishes a dictation (above, "Timing Hex"), through `hex transcribe` against a warm `hex service` (2026-11-18), and one-shot with no service. Each accuracy metric keeps Hex's better run (`--rival` more than once). The text differs a little between them: the API run gets the app's 200 ms tail, and one-shot keeps a final period on `clean-0011` that the service drops.
- **Hex's quiet ceiling**: each clip's polled `hex transcribe` time is later than the model's answer, so the 200 ms tail plus that time is the most Hex's app could take on that clip, give or take decoding the tail itself: the CLI sent no tail, and 200 ms of audio costs Hex at most about 24 ms at its quiet speed (most clips answered within one 612 ms step, a real-time factor of 0.12 or less). The 51 ms margin on the median wait covers that; the other two ceilings decide nothing.
- **Hex picked its CPU engine** on chapel1 ("GPU failed or ran slower"). So it is batch, and its time to first word is the clip length plus its wait.
- **Silas's first piece** arrives while you speak. Its median first word is still close to Hex's batch result because the median clip is about 6 s long.
- **Typing** runs from the final text to the text in the focused field. Silas's figure is measured by `xvfb-run -a -s "-screen 0 1280x1024x24 -noreset" silas-bench type`, its typer at the default "fast" pace on X11. Focus goes back with no fixed wait on X11, and the 60 ms Silas waits after the last key ([core.rs:1393](../../core/voice/src/core.rs#L1393)) comes after the text is in, so it is not counted. Hex's figure is modeled from its X11 paste code, since the paste can't run without Hex's desktop app ([paste.rs at the pinned commit](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs)):

  | Step before the text is in | ms | Source |
  |---|---|---|
  | Save the old clipboard (instant when it is empty or holds Hex's last paste) | 0 | [paste.rs:615-631](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L615-L631) |
  | Wait for held modifier keys (none after key up) | 0 | [paste.rs:772](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L772) |
  | Take the X11 clipboard | 0 | [paste.rs:782-784](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L782-L784) |
  | Sleep before the chord | 60 | [paste.rs:785](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L785) |
  | Ctrl+V: four checked XTest events, at Silas's measured 803 keys a second | 2.5 | [paste.rs:786-817](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L786-L817) |
  | **Total** | **62.5** | |

  After the text is in, Hex waits a 100 ms settle ([paste.rs:644-646](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L644-L646)) and puts the old clipboard back 500 ms later on its own thread ([paste.rs:676-678](https://github.com/anomalyco/hex/blob/536cffef8d5e508b92e50a31a562edd43803ab22/src/linux/paste.rs#L676-L678)). Neither delays the text, so neither is counted, as Silas's 60 ms is not. The 2026-11-18 figure was 2.0 ms: one Ctrl+V timed as one key, which left out the 60 ms sleep before it. Key up to text in the field is the wait after key up plus this row. Silas typed 4852 characters in 7.2 s, about 680 a second (it was 43.9 s, about 108 a second, before the typer wrote to XTest itself); the 122 s longform clip took 2.9 s to type (it was 24.6 s). Keys with no wait go at about 803 a second: the 40 clips have 87 doubled keys, and the typer waits 11 ms before the second press of each so Qt (10 ms) and SDL (2 ms) don't read it as autorepeat, about 1 s in all. Without `-noreset`, Xvfb resets each time the typer's connection closes and the next row waits for it, which adds about 36 ms a row.

### WER by category (lower is better)

| Category | Silas fast | Hex through its API (app path) | Hex one-shot |
|---|---|---|---|
| accent | 10.7% | 15.6% | 14.8% |
| clean | 2.2% | 1.1% | 1.1% |
| code | 29.8% | 26.3% | 33.3% |
| dictation | 6.5% | 7.3% | 3.2% |
| longform | 0.3% | 0.8% | 0.8% |
| noise | 4.8% | 3.4% | 2.7% |

Paired bootstrap, Hex against Silas's `--fast` run ([report.json](opencode-chapel1/report.json)):

- Overall is a tie against both Hex runs.
- Every category is a tie, including accent, where Silas is about 4 to 5 pts ahead on 7 clips, except one: Hex's one-shot run is better on dictation, by 3.2 pts.

Where Silas still loses: dictation against Hex's one-shot run (6.5% against 3.2%, three of the words spoken punctuation), noise (4.8% against 2.7%) and clean (2.2% against 1.1%) lean to Hex, typing is a paste against keystrokes, and on a quiet machine the median wait after key up trails by at least 51 ms (not scored).

### Where Silas's extra word errors come from (2026-11-22)

On 2026-11-18 Hex's better run made 49 word errors in 925 words and Silas 50. Today's build of main makes 48: it writes "rosebush" where the 2026-11-18 build wrote "rose bush" on longform-0010, and two `--fast` runs today agree word for word, so that is the code, not chance. This branch changes no Silas code. Clip by clip against Hex's one-shot run, grouped by cause:

| Cause | Silas's extra errors | Hex's extra errors | Whose |
|---|---|---|---|
| The model wrote a mark where the speaker said "period", "comma" or "colon" (dictation-0008, -0013, -0014) | 3 | 0 | the model |
| Other model words (`uh` as "I", "a m" as "im", Thorleif as "Tor Leaf", "dash dash" as one "dash", accent-0030, noise-0002, -0007) | 8 | 0 | the model |
| Words Hex's model drops or mishears that Silas's keeps (accent-0005, -0014, -0019, -0022, code-0004, "rose bush" in longform-0010) | 0 | 12 | the model |
| Filler removal, dictionary bending | 0 | 0 | not in WER: it is scored on the text before the dictionary |
| Number and casing formatting | 0 | 0 | `20th` and `twentieth` both score as written; case is folded |
| Joins between pieces | 0 | n/a | dictation-0013 and longform-0010 are the only cut clips; whole, they make 2 and 3 errors, cut 2 and 1 |

So Silas now leads by one word, a bootstrap tie, and every error it makes that Hex doesn't is its model's. Three are marks for spoken punctuation. That is what a dictation user wants, and formatted error and punctuation F1 credit it; the verbatim reference counts each as a missing word. Scored with a typed mark allowed to stand for its spoken name (a side calculation, both engines alike, not the scored metric), Silas makes 43 errors and Hex's better run 46.

All 250 clips, the same day and CPU, Hex through its app path (rows in [all-250/](opencode-chapel1/all-250/)):

| | WER | clean | noise | accent | dictation | code | longform | Formatted error | Punctuation F1 |
|---|---|---|---|---|---|---|---|---|---|
| Silas | **4.39%** | 1.9% | 2.3% | **12.0%** | 7.3% | **20.7%** | 2.5% | **13.6%** | **0.753** |
| Hex 3.4.0 | 4.98% | **1.4%** | 2.4% | 19.0% | **6.7%** | 23.5% | **2.1%** | 15.8% | 0.630 |

Silas leads overall by 0.6 pt. Hex leads clean, dictation and longform by under a point each, the categories where its model writes spoken punctuation as words.

Silas's own pipeline costs words in one place: a short clip cut once. On the 41 of 250 clips Silas cuts into pieces, cutting makes 187 errors where the same build with cutting turned off makes 207 (`silas-no-pieces-fast.jsonl.gz`, a diagnostic build, not the app). The long clips gain; three short accented clips cut once lose 11 words at the cut (accent-0001, -0037, -0043). The cut rules live in `core/voice/src/pieces.rs`, which is being changed for speed in another branch, so this branch leaves them alone. No dictionary rule changed, since no error came from the dictionary.

### What it would take to win

- **Typing:** Silas never pastes (AGENTS.md: text goes out as keystrokes and the clipboard is never touched), so typing stays key by key. It went from 349.9 ms to 110.6 ms a clip on X11 (104.2 ms on the ONNX text, which was shorter). On Wayland it types through `wtype`, which waits 2 ms after every press and every release and tops out near 210 characters a second; a Wayland virtual keyboard of Silas's own would lift that.
- **Dictation and noise WER:** Hex's Parakeet Unified English build still leads on these, a few points each, and its model writes spoken punctuation as words, which the verbatim reference wants. Accent no longer loses since the GGUF switch. Both are the model; the default stays ([accent study](../models/accent-2026-11.md)).
- **Code WER:** the ONNX build scored 21.1%; the GGUF build gave up 8.8 points there for the accent gain. Silas's custom words are the lever.
- **Wait after key up:** on a quiet machine Hex leads the median by at least 51 ms, since a short clip is one fast batch decode. Silas leads far on long dictation and on a busy machine, where Hex's 8 spinning threads stall. To settle the quiet numbers, rerun the pairs (below) when chapel1 is quiet and record them.

## The checks

`bench/rivals.py` recomputes Silas's numbers and fails when Silas's margin over Hex is worse than recorded by more than the metric's slack: a lead that shrinks or a loss that grows.

| Gate | What it covers | Where it runs |
|---|---|---|
| accuracy | WER (slack 1 pt), punctuation F1 (0.02), formatted error (1 pt) | CI, every PR that touches `core/voice` or `bench`, on the ratchet run's own rows |
| setup | The README still says the one-line install and First run has 3 items | CI |
| latency | Time to first word, both waits, memory, CPU | Only on the recorded CPU; skipped elsewhere, since shared runners are noisy. `--record` refuses times when Silas's real-time rows show a median load average over 2 |
| typing | `silas-bench type` | Only on the recorded CPU |

The committed rows are gzipped (`gzip -n`); the scripts read `.jsonl.gz` as is. To rerun on chapel1, set `S=~/.cache/silas-bench/bench-set-v1` and `D=bench/rivals/opencode-chapel1`, then:

```sh
silas-bench run --manifest bench/manifest.jsonl --set $S --subset bench/subsets/ratchet.txt --model parakeet-v3 --fast --out $D/silas-fast.jsonl
mkdir -p /tmp/pair && for id in $(grep -v '^#' bench/subsets/ratchet.txt); do
  echo $id > /tmp/pair/$id.txt
  flock /tmp/silas-bench.lock sh -c "silas-bench run --manifest bench/manifest.jsonl --set $S --subset /tmp/pair/$id.txt --model parakeet-v3 --out /tmp/pair/silas-$id.jsonl && uv run bench/engines/hex_engine.py --manifest bench/manifest.jsonl --set $S --subset /tmp/pair/$id.txt --model parakeet_unified_en --out /tmp/pair/hex-$id.jsonl"
done
cat /tmp/pair/silas-*.jsonl > $D/silas-real-time.jsonl; cat /tmp/pair/hex-*.jsonl > $D/hex-api.jsonl
xvfb-run -a -s "-screen 0 1280x1024x24 -noreset" silas-bench type --results $D/silas-real-time.jsonl --out $D/silas-typing.jsonl
uv run bench/engines/hex_engine.py --manifest bench/manifest.jsonl --set $S --subset bench/subsets/ratchet.txt --model parakeet_unified_en --one-shot --out $D/hex-one-shot.jsonl
python3 -I bench/rivals.py opencode --silas $D/silas-fast.jsonl --silas $D/silas-real-time.jsonl --typing $D/silas-typing.jsonl
gzip -n -f $D/*.jsonl
```

The loop times Silas and Hex clip by clip, back to back under one lock, so a busy machine slows both alike; on a quiet machine two whole runs give the same numbers. Add `--rival $D/hex-api.jsonl --rival $D/hex-one-shot.jsonl --record` to the last command to rewrite the file. It refuses when Silas fell behind, unless `--allow-worse` says the loss is known. `hex_engine.py --cli` repeats the old polled `hex transcribe` timing (`hex-cli.jsonl.gz`), which the check reads as unmeasured.

`hex_engine.py` hash-checks the release binary and runs Hex with HOME and the XDG folders inside `~/.cache/silas-bench/hex-home`. It never touches your own Hex settings, and it needs no account.
