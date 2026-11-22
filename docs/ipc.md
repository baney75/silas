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

// Sync (docs/sync.md). Never carries the key or the pairing code, except PairingCode.
interface SyncStatus {
  enabled: boolean;
  paired: boolean;                // a sync key is on this device
  deviceId: string;               // 32 hex, made once, kept in sync.json
  name: string;                   // this device's name as the others see it
  platform: Platform;
  port: number | null;            // the listener's port while sync runs (47823, or any free port if that is taken)
  peers: string[];                // "host:port" for networks where multicast doesn't reach
  folder: string | null;          // the synced folder, if one is set
  devices: SyncDevice[];          // devices seen with this key, newest first
  last: LastSync | null;          // the last sync this run, even one that changed nothing
}
interface SyncDevice { id: string; name: string; platform: string; lastSynced: number; via: "network" | "folder" } // lastSynced: Unix ms
interface LastSync   { with: string; at: number; added: number; changed: number; removed: number }           // at: Unix ms
interface PairingCode { code: string; url: string } // "ABCD-EFGH-...", "silas://pair?k=ABCDEFGH..."
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
| `get_sync` | | `SyncStatus`. Read once at launch; `sync` events keep it current, so Settings asks for nothing |
| `set_sync_enabled` | `{ on }` | `SyncStatus`. On makes a key the first time and starts the listener, discovery and the folder watcher; off stops them |
| `sync_pairing_code` | | `PairingCode`. Only when the person presses Pair; makes a key and turns sync on the first time. The code is never logged or sent in an event |
| `join_sync` | `{ code }` | `SyncStatus`. Takes the code typed in any case, with or without dashes or spaces, or a `silas://pair?k=` URL. A bad code errors with "That code isn't right. Check it and try again." Replaces any key this device had |
| `new_sync_key` | | `SyncStatus`. A fresh key; every other device must pair again |
| `leave_sync` | | `SyncStatus`. Deletes the key from the keychain (and the Linux fallback file), forgets seen devices, turns sync off. The dictionary stays |
| `set_sync_name` | `{ name }` | `SyncStatus` (trimmed, at most 63 bytes, control characters dropped) |
| `add_sync_peer` / `remove_sync_peer` | `{ addr }` | `SyncStatus`. `host`, `host:port`, `ip` or `[ipv6]:port`; the default port is 47823. Adding one syncs with it at once |
| `set_sync_folder` | `{ folder: string \| null }` | `SyncStatus`. `~` expands; it must be an existing folder. Silas writes `Silas Sync/<deviceId>.silas` inside it. `null` stops using it |
| `sync_now` | | `null`. Syncs with every peer, discovered device and the folder now, without waiting for the 10-minute resync |

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
| `model-progress` | `{ id, downloaded, total, stage: "downloading" \| "unpacking" \| "loading" \| "done" \| "error", message? }` |
| `sync` | `SyncStatus` whenever it changes (a sync, a device seen, a setting) |
| `dictionary` | `Dictionary` after a sync merged a change from another device. The running app already uses it; the UI replaces its copy |

## Sync files

Sync keeps `sync.json` next to `dictionary.json`: the device id, the clock, the last-writer-wins
entries (tombstones included), the on/off switch, the name, peers, folder and seen devices. It never
holds the key. The key lives in the OS keychain (service `Silas Sync`), or on Linux without a
Secret Service in `sync-key`, mode 0600, in the data folder. Every write to `dictionary.json` (an
edit, an import, learning) tells the sync engine, which sends the change after a 2 s pause.

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
