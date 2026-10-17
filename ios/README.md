# Silas for iPad

A native SwiftUI app with the same job as the desktop Silas: hold the button, speak, let go,
and Silas writes what you said. iPadOS can't type into other apps, so the text lands in an
editable transcript you can copy, share, or save to Obsidian.

- **Speech:** Parakeet TDT v3 on the device through [FluidAudio](https://github.com/FluidInference/FluidAudio)
  0.9.1 (CoreML). The model (about 480 MB) downloads once from Hugging Face on first launch.
  Audio never leaves the iPad. There are no other network calls and no analytics.
- **Button:** hold to speak, release to write. A tap (under 0.28 s, like the desktop key)
  starts hands-free; tap again to write. The × cancels. Recording stops at 5 minutes.
- **Cleanup:** fillers and stutters are removed with a Swift port of the desktop's
  `drop_fillers` and `drop_stutters` (`SilasCore/Sources/SilasCore/Cleanup.swift`). The
  personal dictionary isn't ported yet.
- **Save to Obsidian:** opens
  `obsidian://new?vault=BaneyVault&file=0_Inbox/Dictation <yyyy-MM-dd HH.mm>&content=<text>`.
- **History:** every dictation is kept in `Application Support/Silas/history.json`. Tap one to
  reopen it; edits are saved; swipe to delete.

## Layout

| Path | What |
| --- | --- |
| `project.yml` | XcodeGen spec. `Silas.xcodeproj` is generated and not committed. |
| `Silas/` | The app: `SpeechEngine` (FluidAudio), `Recorder` (16 kHz mono capture), `DictationModel`, views. |
| `SilasCore/` | Pure Swift package: cleanup, the hold/tap state machine, the Obsidian URL. |
| `scripts/install.sh` | Generate, build, install and launch on a USB-connected iPad. |

## Check

```sh
(cd ios/SilasCore && swift test)
```

## Install on the iPad

Needs Xcode signed in to the Apple ID behind the free Personal Team (`54H7JX2PD4`):
Xcode › Settings › Accounts. Then, with the iPad on USB:

```sh
ios/scripts/install.sh            # vestry1; pass another UDID to target a different iPad
```

Free Personal Team profiles expire after 7 days. When Silas stops opening, run the same
command again to re-sign and reinstall; history and the downloaded model are kept.

Debug builds accept `-selftest <file.wav>` as a launch argument: once the model is loaded they
transcribe that file and log `SILAS-SELFTEST` with the raw and cleaned text.
