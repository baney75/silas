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
  permissions: Permissions;
}

interface Permissions {
  microphone: "granted" | "denied" | "unknown";
  accessibility: "granted" | "denied" | "unknown"; // macOS: typing + key listening; Linux: input device access
}

interface Settings {
  hotkey: string;                 // handy-keys string: "CmdRight", "CtrlRight", "AltRight", "Fn", "F13", ...
  tapToToggle: boolean;           // quick tap starts hands-free; tap again to stop
  model: string;                  // model id. Read-only through set_settings: use select_model
  modelSource: "default" | "registry" | "benchmark" | "user"; // who chose `model`. "user" is never overridden. Read-only through set_settings
  language: string;               // "en" etc. (Cohere needs it; Parakeet auto-detects)
  removeFillers: boolean;
  trailingSpace: boolean;
  typingSpeed: "steady" | "fast" | "instant"; // chars/sec 250 | 700 | 2000
  newlineAsShiftEnter: boolean;   // so chat boxes do not send early
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
  recommended: boolean;           // best_for_device when this computer has a pick, else the static flag
  deviceResult: ModelResult | null; // this computer's measurement (benchmark.json)
  bestForDevice: boolean;         // the registry's or this computer's own benchmark's pick
}

// Benchmark types keep snake_case: the same JSON is the shared device report.
interface Device {
  key: string;                    // "macos/apple-m5/10": slug(os)/slug(cpu brand)/logical cores
  os: "macos" | "windows" | "linux";
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
}

interface Report {
  schema: 1;
  silas: string;                  // app version
  suite: string;                  // which speech set produced the numbers, "librispeech-test-clean-9clips-v1"
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
| `accept_pending` / `dismiss_pending` | `{ from }` | `Dictionary` |
| `import_dictionary` | `{ json }` | `Dictionary` (accepts Silas export or a TypeWhisper export) |
| `export_dictionary` | | `string` (pretty JSON) |
| `get_stats` | | `Stats` |
| `toggle_recording` | | `null` (same as tapping the hotkey) |
| `capture_hotkey` | | `string` next key/modifier pressed, e.g. `"CmdRight"` (10 s timeout) |
| `open_permission` | `{ kind: "microphone" \| "accessibility" \| "input" }` | `null`. Reads first; shows the system prompt at most once per run and never when already granted, otherwise opens the OS settings pane |
| `reset_accessibility` | | `null`. macOS onboarding: when not trusted, clears Silas's Accessibility entry, asks at most once per run, opens Settings |
| `fix_accessibility` | | `null`. macOS Fix Permission: when not trusted, clears Silas's Accessibility entry and reopens Silas, which asks once and opens Settings. No-op when trusted or off macOS |
| `list_microphones` | | `string[]`. Runs off the main thread (Linux opens every ALSA device to list them); the UI reads it at launch and on focus, never when Settings opens |
| `show_main` | | `null` |
| `get_benchmark` | | `Report \| null` (saved `benchmark.json`) |
| `run_benchmark` | `{ all }` | `null` when the run ends. Measures the active model, and with `all` every other installed one. Errors at once if a benchmark is already running or no model is loaded. Results of models not run this time are kept in the report |
| `share_benchmark` | | `null` opens a prefilled GitHub issue (`report_url`) in the default browser; the user reviews and submits it |
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
| `notice` | `{ kind: "error" \| "busy" \| "stopped", message }` why a press did nothing or a dictation stopped. The overlay shows it for 4 s (`NOTICE_FOR`) even when its setting is off; the main window shows it as a toast |
| `benchmark-progress` | `{ model, clip, clips }` clips finished so far for `model` (0 when it starts) |
| `benchmark-done` | `Report` just saved |
| `benchmark-error` | `{ message }` |
| `file-progress` | `FileProgress`. `decoding` first, then `transcribing` after each piece, `waiting` while a dictation has the model, and one of `done`, `error`, `cancelled`. `refused` alone means it never started (also sent for dropped files, which have no caller) |
| `file-drag` | `{ active }` files are being dragged over the main window (`true`), or left or were dropped (`false`). Dropped files are transcribed one after another, as `transcribe_file` |
| `model-progress` | `{ id, downloaded, total, stage: "downloading" \| "unpacking" \| "loading" \| "done" \| "error", message? }` |

## Choosing a model for this computer

At startup, in the background, the core fetches `registry/devices.json` from GitHub (5 s timeout,
cached as `registry.json`) and looks up the device key.

- A hit while `modelSource` is `"default"` sets `model` and `modelSource = "registry"`, with no benchmark.
  It switches only if that model is installed, or if no model is installed yet.
- A miss with no `benchmark.json`: once a model is loaded, the core runs the bundled LibriSpeech set
  (`core/voice/bench/`) clip by clip, waiting while a dictation is happening. An automatic run
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
