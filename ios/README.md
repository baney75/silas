# Silas for iPhone and iPad

One universal SwiftUI app with the same job as the desktop Silas: hold the button, speak, let go,
and Silas writes what you said, spelled your way. The Silas Keyboard brings it into any app.

- **Speech:** Parakeet TDT v3 on the device through [FluidAudio](https://github.com/FluidInference/FluidAudio)
  0.9.1 (CoreML). The model (about 480 MB) downloads once from Hugging Face on first launch.
  Audio never leaves the device and there are no analytics.
- **Button:** hold to speak, release to write. A tap (under 0.28 s, like the desktop key)
  starts hands-free; tap again to write. The × cancels. Recording stops at 5 minutes.
- **Live text:** while you speak, Silas re-reads what it has heard every second or so with the
  same loaded model and shows it.
- **Cleanup and dictionary:** the desktop's rules, ported to Swift in `SilasCore`: fillers and
  stutters, your words (close-sounding words snap to your spelling, by Double Metaphone and
  edit distance), corrections with the common-word guard, and learning from your edits. Edit a
  dictation and Silas learns the fix, as the desktop does from its history. The tests reuse the
  desktop's own cases from `core/voice/src/dictionary.rs`.
- **Sync:** your words, corrections and fillers sync with your other Silas devices per
  [`docs/sync.md`](../docs/sync.md): on the local network while the app is open, through peers
  by address (a tailnet name), or through a folder you pick (iCloud Drive works). Pair by
  scanning the desktop's QR code or typing the code. The key stays in the Keychain.
- **Copy, Share, Save to Obsidian, History:** Copy touches the pasteboard only when you tap it.
  Save to Obsidian opens `obsidian://new?vault=BaneyVault&file=0_Inbox/Dictation <date>`.
  History is in `Application Support/Silas/history.json`; tap one to reopen it.
- **Shortcuts and the Action button:** "Dictate with Silas" opens Silas already listening.
  "Last Silas dictation" returns the last text to any shortcut.

## The Silas Keyboard

iOS keyboards can't use the microphone, so the keyboard asks the app to listen:

1. In any app, switch to the Silas keyboard and tap the mic. Silas opens
   (`silas://dictate?from=keyboard`) and starts listening.
2. Speak, then go back to your app (the "◀" in the corner) and tap the mic again to stop, or
   tap the big button in Silas. The text is typed where you were, with `textDocumentProxy`.
3. For five minutes after, the microphone stays armed (the background audio mode, so iOS
   shows the microphone indicator) and the mic key starts and stops without leaving your app.

The app and keyboard talk through the App Group `group.com.baney.silas`
(`SilasCore/Sources/SilasCore/Handoff.swift`) and Darwin notifications. The keyboard needs
**Allow Full Access** to read that shared folder; it sends nothing anywhere. Other keys: globe
(next keyboard), delete, space, return, and "type last again".

Turn it on: Settings › General › Keyboard › Keyboards › Add New Keyboard › Silas, then tap
Silas and turn on Allow Full Access.

## What ships when

Checked on 2026-11-22: Xcode made free Personal Team profiles for both `com.baney.silas.ios` and
`com.baney.silas.ios.keyboard`, each with `group.com.baney.silas`, and a `generic/platform=iOS`
Release build of the app with the keyboard signed with them.

| What | Free Apple account (Personal Team), today | Paid Apple Developer Program ($99 a year) |
| --- | --- | --- |
| The app: dictation, live text, dictionary, history, Copy, Share, Obsidian | Yes | Yes |
| Silas Keyboard (App Group, Full Access, background mic session) | Yes | Yes |
| Shortcuts, Siri, the Action button | Yes | Yes |
| Sync: local network, peers by address, a synced folder (iCloud Drive through the picker) | Yes | Yes |
| How long an install lasts | 7 days; run `install.sh` again (history, dictionary and model are kept) | A year |
| Devices | Up to 3 per device type, plugged into the Mac | Up to 100, over the air |
| TestFlight, App Store, installing on someone else's phone | No | Yes |

Nothing in Silas needs a paid-only capability; the program buys longer signatures and distribution.
If the App Group is ever missing (a build signed without it), the app runs without the keyboard
handoff and Settings says so.

## Layout

| Path | What |
| --- | --- |
| `project.yml` | XcodeGen spec. `Silas.xcodeproj` is generated and not committed. |
| `Silas/` | The app: `SpeechEngine` (FluidAudio), `Recorder`, `DictationModel` (+`Keyboard`), `SyncController`, views, `Intents`. |
| `SilasKeyboard/` | The keyboard extension. |
| `SilasCore/` | Swift package: cleanup, dictionary, Double Metaphone, sync, keyboard handoff, history, hold/tap. The root `Package.swift` points at it so `swift test` runs from the repo root. |
| `SilasUITests/` | XCUITests: launch, model ready plus a bundled clip, the keyboard typing handed-off text. |
| `Config/` | Info.plists and entitlements. |
| `scripts/install.sh` | Build, install and launch on a USB-connected device (free signing works). |
| `scripts/screenshots.sh` | Run the UI tests and export their screenshots to `docs/`. |

## Check

```sh
swift test                                   # at the repo root: SilasCore, sync vectors included
cd ios && xcodegen generate && xcodebuild -project Silas.xcodeproj -scheme Silas \
  -destination "platform=iOS Simulator,name=iPhone 17" -derivedDataPath build test
```

The first UI test run on a fresh simulator downloads the speech model (about 480 MB).

## Install on a device

Needs Xcode signed in to the Apple ID behind team `54H7JX2PD4` (Xcode › Settings › Accounts).
With the iPhone or iPad on USB, in Terminal on the Mac (codesign can't reach the keychain over ssh):

```sh
ios/scripts/install.sh <udid>      # or SILAS_DEVICE=<udid>; list with: xcrun devicectl list devices
```

Then turn on the keyboard (above). Debug builds accept `-selftest <file.wav>`: once the model is
loaded they transcribe that file and log `SILAS-SELFTEST`.
