# Silas IPC contract

The Rust core (`src-tauri/src`) and the web UI (`src/`) talk only through these Tauri
commands and events. Names are snake_case on the Rust side; the UI calls them with
`invoke("name", { camelCaseArgs })`.

## Types (JSON)

```ts
type Status = "loading" | "needs_model" | "ready" | "recording" | "transcribing" | "typing" | "error";

interface AppState {
  status: Status;
  detail: string | null;          // error text or loading step
  platform: "macos" | "windows" | "linux";
  version: string;
  model: string | null;           // active model id, null if none installed
  hotkey: string;                 // e.g. "CmdRight"
  hotkeyLabel: string;            // e.g. "Right ⌘"
  retypeHotkeyLabel: string | null; // label of Settings.retypeHotkey, null when unset
  app: AppInfo | null;            // the app the last dictation went to; null before the first one,
                                  // or where the system doesn't name apps (Linux outside Hyprland)
  permissions: Permissions;
}

// How Silas shapes text for an app (core/voice/src/style.rs):
//   prose    as spoken (the default for apps not in the table)
//   chat     prose; new lines go out as Shift+Return whatever newlineAsShiftEnter says
//   code     spoken code on (core/voice/src/spoken.rs): "dash dash", "dot", "open paren",
//            "camel case user name"; the trailing period is dropped when the text reads as code.
//            Words that are also English (underscore, equals, pipe, dot, tilde, all caps) turn
//            into code only next to other code; "new line" breaks only when said on its own
//   terminal spoken code on, no capital at the start, never a trailing period, and spoken code
//            never types a line break (it would be Return)
type AppStyle = "prose" | "chat" | "code" | "terminal";

interface AppInfo {
  id: string;                     // macOS bundle id, Windows exe name ("code.exe"), Hyprland window class
  name: string;                   // from the built-in table (style.rs KNOWN), else the id
  style: AppStyle;                // what it gets now: Settings.appStyles, else `builtin`
  builtin: AppStyle;              // Silas's own style for it: the table, else "prose"
}

interface Snippet {
  trigger: string;                // what you say; matched without case or punctuation
  text: string;                   // typed exactly as saved, new lines included
  added: string;                  // RFC 3339
}

interface Permissions {
  microphone: "granted" | "denied" | "unknown";
  accessibility: "granted" | "denied" | "unknown"; // macOS: typing + key listening; Linux: input device access
}

interface Settings {
  hotkey: string;                 // handy-keys string: "CmdRight", "CtrlRight", "AltRight", "Fn", "F13", ...
  tapToToggle: boolean;           // quick tap starts hands-free; tap again to stop
  model: string;                  // model id. Read-only through set_settings: use select_model
  modelSource: "default" | "registry" | "community" | "benchmark" | "user"; // who chose `model`. "user" is never overridden. Read-only through set_settings
  language: string;               // "en" etc. (Cohere needs it; Parakeet auto-detects)
  removeFillers: boolean;
  trailingSpace: boolean;
  typingSpeed: "steady" | "fast" | "instant"; // chars/sec 250 | 700 | 2000
  newlineAsShiftEnter: boolean;   // so chat boxes do not send early
  retypeHotkey: string | null;    // handy-keys string for "type the last dictation again"; null = unset.
                                  // set_settings rejects the dictation hotkey itself or a string that isn't a key
  appStyles: Record<string, AppStyle>; // app id -> style the user chose; ids match without case
  learnFromEdits: boolean;
  sounds: boolean;
  pauseMedia: boolean;            // pause playing music and video while dictating (default on); see below
  showOverlay: boolean;
  launchAtLogin: boolean;
  microphone: string | null;      // device name, null = system default
  onboarded: boolean;
}

interface ModelEntry {
  id: string;                     // "parakeet-v3" | "parakeet-v3-onnx" | "cohere-int4"
  name: string;                   // "Parakeet v3"
  tier: "fast" | "balanced" | "best";
  tagline: string;
  sizeMb: number;
  languages: string;              // "25 European languages"
  installed: boolean;
  active: boolean;
  recommended: boolean;           // this computer's own pick, else the registry's, else the static flag; never a community pick
  deviceResult: ModelResult | null; // this computer's measurement (benchmark.json)
  bestForDevice: boolean;         // this computer's own benchmark's pick, else the registry's, else a settled community pick
}

// Benchmark types keep snake_case: the same JSON is the shared device report.
interface Device {
  key: string;                    // "macos/apple-m5/10": slug(os)/slug(cpu brand)/logical cores
  os: "macos" | "windows" | "linux" | "ios" | "ipados";
  os_version: string;
  cpu: string;
  cores: number;
  memory_gb: number;
  hw_model: string | null;        // macOS only, e.g. "Mac17,2"
}

interface ModelResult {
  model: string;
  speed: number;                  // seconds of audio per second of compute; the bar is 10
  wer: number;                    // word error rate; the bar is 0.08
  audio_seconds: number;
  compute_seconds: number;
  clips: number;
  wait_seconds: number;           // median compute for the 5 to 10 s clips: the wait after you stop talking
  clip_seconds?: number[];        // compute per clip; kept in benchmark.json, left out of a shared report
}

interface CommunityDevice {       // GET /v1/devices?key= on the report service, one device key
  reports: number;
  pick: string | null;            // the pick rule over the per-model medians
  agree: number;                  // reports whose own pick is `pick`
  models: Record<string, { reports: number; speed: number; wer: number; wait: number }>; // medians
}

interface ShareReply { key: string; reports: number; pick: string | null; agree: number }

interface Report {
  schema: 1;
  silas: string;                  // app version
  suite: string;                  // which speech set produced the numbers, registry/suite.json `id` ("silas-bench-mixed-14clips-v2")
  device: Device;
  results: ModelResult[];
  pick: string | null;            // model id; rule in src-tauri/src/benchmark.rs (mirrored in registry/ingest.py)
  ran_at: string;                 // RFC 3339
}

interface HistoryItem {
  id: string;                     // ms timestamp string
  ts: string;                     // RFC 3339
  text: string;                   // what was typed
  raw: string;                    // engine output before the dictionary
  durationS: number;              // audio length
  latencyMs: number;              // release → text ready
  words: number;
  corrected: string | null;       // text after the user taught a fix
  file?: string;                  // file name when it came from transcribe_file (then nothing was typed,
                                  // latencyMs is the whole transcription and Stats leave it out)
}

interface FileProgress {
  name: string;                   // the file's name
  stage: "decoding" | "transcribing" | "waiting" | "done" | "error" | "cancelled" | "refused";
  doneS: number;                  // seconds of audio transcribed so far
  totalS: number | null;          // the file's length when the container says it
  message?: string;               // error, cancelled and refused: why
  item?: HistoryItem;             // done: the new History entry
}

interface Correction { from: string; to: string; count: number; source: "user" | "learned" | "import"; added: string }
interface Pending    { from: string; to: string; count: number; lastSeen: string }
interface Term       { text: string; source: "user" | "learned" | "import"; added: string; uses: number }

interface Dictionary {
  terms: Term[];
  corrections: Correction[];
  pending: Pending[];             // seen fixes not yet trusted (needs 2 sightings)
  fillers: string[];              // words removed when removeFillers is on
  snippets: Snippet[];            // one per trigger match form; sync key "s:" + norm(trigger) (docs/sync.md)
}

interface Stats {
  wordsTotal: number; wordsToday: number; dictations: number;
  avgWpm: number;                 // speaking pace
  minutesSaved: number;           // vs typing at 40 wpm
  learned: number;                // corrections learned automatically
}

interface LearnResult { learned: Correction[]; pending: Pending[]; terms: string[] }
```

## Commands

| Command | Args | Returns |
| --- | --- | --- |
| `get_state` | | `AppState` |
| `get_settings` | | `Settings` |
| `set_settings` | `{ settings: Settings }` | `Settings` (applies hotkey, autostart). Keeps the core's `model` and `modelSource`, so an older copy of the settings can't undo a benchmark's switch |
| `list_models` | | `ModelEntry[]` |
| `download_model` | `{ id }` | `null` once done; progress arrives as events |
| `select_model` | `{ id, auto? }` | `null` (loads it; status events follow; sets `modelSource` to `"user"` unless `auto`, which setup uses for its own choice) |
| `wait_device_pick` | | `null` once this run has asked the device registry (at most 6 s); setup awaits it before choosing a model |
| `delete_model` | `{ id }` | `null` |
| `get_history` | `{ limit }` | `HistoryItem[]` newest first |
| `correct_history` | `{ id, text }` | `LearnResult` (teach Silas from a fixed transcript) |
| `delete_history` | `{ id }` | `null` |
| `clear_history` | | `null` |
| `get_dictionary` | | `Dictionary` |
| `add_term` / `remove_term` | `{ text }` | `Dictionary` |
| `add_correction` | `{ from, to }` | `Dictionary` |
| `remove_correction` | `{ from }` | `Dictionary` |
| `add_snippet` | `{ trigger, text }` | `Dictionary`. Replaces the text of a snippet with the same trigger. Errors when either is empty |
| `remove_snippet` | `{ trigger }` | `Dictionary` |
| `accept_pending` / `dismiss_pending` | `{ from }` | `Dictionary` |
| `import_dictionary` | `{ json }` | `Dictionary` (accepts Silas export or a TypeWhisper export) |
| `export_dictionary` | | `string` (pretty JSON) |
| `get_stats` | | `Stats` |
| `toggle_recording` | | `null` (same as tapping the hotkey) |
| `type_again` | | `null` once the retype has started. Types the newest History item (its taught fix, if any) into the field with the keyboard, with a dictation's guards: never into a password field or a window that took the keyboard, and Escape stops it. The tray item and `retypeHotkey` do the same. Errors with the reason when paused, busy, or History is empty. Windows: errors, and the tray item is disabled, because Silas can't yet remember and restore the window the user was typing in and the command and the menu leave Silas's own window in front; only `retypeHotkey` types there |
| `capture_hotkey` | | `string` next key/modifier pressed, e.g. `"CmdRight"` (10 s timeout) |
| `open_permission` | `{ kind: "microphone" \| "accessibility" \| "input" }` | `null`. Reads first; shows the system prompt at most once per run and never when already granted, otherwise opens the OS settings pane |
| `reset_accessibility` | | `null`. macOS onboarding: when not trusted, clears Silas's Accessibility entry, asks at most once per run, opens Settings |
| `fix_accessibility` | | `null`. macOS Fix Permission: when not trusted, clears Silas's Accessibility entry and reopens Silas, which asks once and opens Settings. No-op when trusted or off macOS |
| `list_microphones` | | `string[]`. Runs off the main thread (Linux opens every ALSA device to list them); the UI reads it at launch and on focus, never when Settings opens |
| `show_main` | | `null` |
| `get_benchmark` | | `Report \| null` (saved `benchmark.json`; null when it was made on another computer or an older `suite`, which counts as no benchmark) |
| `run_benchmark` | `{ all }` | `null` when the run ends. Measures the active model, and with `all` every other installed one. Errors at once if a benchmark is already running or no model is loaded. Results of models not run this time are kept in the report |
| `share_preview` | | `string \| null`: the exact JSON a share posts (`Report` without `clip_seconds`), or null before any benchmark on the current `suite` |
| `share_benchmark` | `{ payload }` | `ShareReply`. Posts `payload` to the report service (10 s timeout) and refreshes the community copy. Errors with a sentence if `payload` is not the current `share_preview` (so what the user saw is what is sent), on 413 or 429, on a rejected report, or when the service is out of reach |
| `share_benchmark_github` | | `null` opens a prefilled GitHub issue (`report_url`) in the default browser; the user reviews and submits it |
| `get_community` | | `CommunityDevice \| null`: shared results for this device (3 s timeout, then the cached `community.json`; null offline with no copy) |
| `get_device` | | `Device` |
| `transcribe_file` | `{ path?: string }` | `HistoryItem \| null`. Transcribes an audio or video file into History; nothing is typed and the clipboard is untouched. With no `path` it opens the system picker and returns `null` if the person cancels. Errors (and emits `file-progress` `refused`) while a dictation is under way, before a model is loaded, or while another file runs. Progress and the result arrive as `file-progress` events too |
| `cancel_file` | | `boolean` whether a file was being transcribed; it stops after the current piece |

## Events (`listen(name, cb)`)

| Event | Payload |
| --- | --- |
| `status` | `{ status: Status, detail: string \| null }` |
| `level` | `number` 0..1 microphone level, ~30 Hz while recording (overlay) |
| `transcript` | `HistoryItem` just typed |
| `learned` | `LearnResult` when Silas learned from an edit |
| `app` | `AppInfo` of the app a dictation or retype is going to, sent as it is typed |
| `notice` | `{ kind: "error" \| "busy" \| "stopped", message }` why a press did nothing or a dictation stopped. The overlay shows it for 4 s (`NOTICE_FOR`) even when its setting is off; the main window shows it as a toast |
| `benchmark-progress` | `{ model, clip, clips }` clips finished so far for `model` (0 when it starts) |
| `benchmark-done` | `Report` just saved |
| `benchmark-error` | `{ message }` |
| `file-progress` | `FileProgress`. `decoding` first, then `transcribing` after each piece, `waiting` while a dictation has the model, and one of `done`, `error`, `cancelled`. `refused` alone means it never started (also sent for dropped files, which have no caller) |
| `file-drag` | `{ active }` files are being dragged over the main window (`true`), or left or were dropped (`false`). Dropped files are transcribed one after another, as `transcribe_file` |
| `model-progress` | `{ id, downloaded, total, stage: "downloading" \| "unpacking" \| "loading" \| "done" \| "error", message? }` |

## Control socket (Linux, macOS)

`silas-ctl <command>` (or `silas <command>`, which loads the GUI libraries first and takes
about 30 ms instead of 2) writes one line to `$XDG_RUNTIME_DIR/silas.sock` and reads `ok`.
Without `XDG_RUNTIME_DIR` the socket is in an owner-only `silas-<uid>` folder in the temp dir.
Compositor keybindings use it, since on Wayland the compositor owns the keyboard. The engine
(`core/voice/src/control.rs`) handles commands one at a time, in the order they arrived.
`silas-ctl` ships in the Arch package; elsewhere use `silas <command>`.

| Command | Same as |
| --- | --- |
| `press` | the hotkey going down (a short press-and-release starts hands-free) |
| `release` | the hotkey coming up |
| `toggle` | the record button in the window |
| `cancel` | Esc while listening |

Exit code 1 means Silas isn't running. This socket is to fold into the `silas.voice` engine
socket (baney75/local-studio#6).

## Choosing a model for this computer

At startup, in the background, the core first reads this computer's own `benchmark.json`; when
its pick passes both bars, that settles it and nothing is looked up. Otherwise the core fetches
`registry/devices.json` from GitHub (5 s timeout,
cached as `registry.json`) and looks up the device key. On a miss it asks the report service
(`GET /v1/devices?key=`, 3 s timeout, cached as `community.json`). Trust order:

1. This computer's own `benchmark.json` for the current `suite`.
2. The registry: maintainer-approved.
3. The community: used only when at least 3 independent senders agree (`agree >= 3`; the service
   counts one report per network per day), the pick
   recomputed over the medians matches the service's, the model is known, and its medians pass both
   bars (`modelSource = "community"`). Fewer than 3 are shown on the Model page as information only.

Offline, the cached copies answer; with no copy, Silas measures as before.

- A hit while `modelSource` is `"default"` sets `model` and `modelSource` (`"registry"` or `"community"`), with no benchmark.
  It switches only if that model is installed, or, for a registry hit, if no model is installed yet. A
  community pick never chooses what first-run setup downloads: `recommended` comes only from this
  computer's report or the registry.
- A miss in both with no `benchmark.json`: once a model is loaded, the core runs the bundled set
  (`core/voice/bench/`, 14 clips, 84 s: clean, noisy, accented, dictation, command and code speech) clip by clip, waiting while a dictation is happening. An automatic run
  measures the active model and then every other installed model (nothing is downloaded). It saves the
  `Report` and, unless `modelSource` is `"user"`, switches to the pick only if it is installed and
  passes both bars (`modelSource = "benchmark"`); a pick that misses a bar is shown, never switched to.
  Other models load only after 30 s without dictation. Earlier results are kept for models a run
  skipped only if they came from the same Silas version and `suite`.
- `select_model` sets `modelSource = "user"` (unless `auto`). Nothing automatic changes the model after that.
- Settings saved before `modelSource` existed migrate on load: a non-default model, or a second installed model after onboarding, becomes `"user"`.
- One benchmark runs at a time. While `status` is `recording`, `transcribing`, `typing` or `loading`
  the benchmark waits (300 ms polls); it holds the engine for one clip at a time.

`silas-bench suite [model-id...]` runs the same set on the installed models from the command line and
prints the `Report` and its share link.

## Pausing media while dictating

With `pauseMedia` on, the key press pauses every player that is playing (Linux: MPRIS over D-Bus;
macOS: the system Now Playing app; Windows: the system media sessions), off the key thread. When the
dictation ends or is cancelled, Silas plays again only the players it paused, and only those still
paused: a player the person paused or changed meanwhile is left alone. After a finished dictation the
music waits for the end-of-dictation tail, so a dictation right after it doesn't make it blip.
Code: `core/voice/src/nowplaying.rs`.

## Command line

`silas transcribe <file> [--raw] [--json] [--model <id>] [--data-dir <dir>]` prints what was said
in the file and exits (0 on success, 1 on failure, 2 for a usage error). It runs without the window
or the tray and loads the model once. `--json` prints
`{ file, model, text, raw, audioSeconds, pieces, timings: { loadMs, decodeMs, transcribeMs, totalMs } }`.
Formats: WAV, MP3, M4A (AAC, ALAC), FLAC, OGG (Vorbis, Opus), Opus, AIFF, CAF, and the sound of
MP4, MOV, MKV and WebM. Long audio is cut at pauses into pieces of 6 to 20 s, as a long dictation is.
