// Typed wrapper around the Silas IPC contract (docs/ipc.md).
// Outside Tauri (plain `vite` in a browser) every call goes to an in-memory mock,
// so the UI can be previewed and designed without the Rust core.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { mockInvoke, mockListen } from "./mock";

export type Status = "loading" | "needs_model" | "ready" | "recording" | "transcribing" | "typing" | "error";
export type Platform = "macos" | "windows" | "linux";
export type PermissionState = "granted" | "denied" | "unknown";

export interface Permissions {
  microphone: PermissionState;
  accessibility: PermissionState;
}

export interface AppState {
  status: Status;
  detail: string | null;
  platform: Platform;
  version: string;
  model: string | null;
  hotkey: string;
  hotkeyLabel: string;
  /** The key that types the last dictation again, or null when none is set. */
  retypeHotkeyLabel: string | null;
  /** The app the last dictation went to, or null before the first one (or where the system doesn't name apps). */
  app: AppInfo | null;
  permissions: Permissions;
}

/** How Silas shapes text for an app. See docs/ipc.md "App styles". */
export type AppStyle = "prose" | "terminal" | "code" | "chat";

export interface AppInfo {
  /** macOS bundle id, Windows exe name, or Hyprland window class. */
  id: string;
  name: string;
  /** The style the app gets now: the user's choice, else `builtin`. */
  style: AppStyle;
  /** The style Silas gives the app on its own. */
  builtin: AppStyle;
}

export interface Snippet {
  trigger: string;
  text: string;
  added: string;
}

export type TypingSpeed = "steady" | "fast" | "instant";

export interface Settings {
  hotkey: string;
  tapToToggle: boolean;
  model: string;
  /** Who chose the model: the app's default, the device registry, shared community results, a benchmark, or the user. */
  modelSource: ModelSource;
  language: string;
  removeFillers: boolean;
  trailingSpace: boolean;
  typingSpeed: TypingSpeed;
  newlineAsShiftEnter: boolean;
  /** A second key that types the last dictation again. null when unset. */
  retypeHotkey: string | null;
  /** App id to style, for apps the user assigned by hand. */
  appStyles: Record<string, AppStyle>;
  learnFromEdits: boolean;
  sounds: boolean;
  /** Pause music and videos while dictating, and play them again after. */
  pauseMedia: boolean;
  showOverlay: boolean;
  launchAtLogin: boolean;
  microphone: string | null;
  onboarded: boolean;
  updateChannel: UpdateChannel;
  autoUpdate: boolean;
}

export type ModelSource = "default" | "registry" | "community" | "benchmark" | "user";

export type UpdateChannel = "stable" | "nightly" | "dev";

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
  channel: string;
}

export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

export type Tier = "fast" | "balanced" | "best";

export interface ModelEntry {
  id: string;
  name: string;
  tier: Tier;
  tagline: string;
  sizeMb: number;
  languages: string;
  installed: boolean;
  active: boolean;
  recommended: boolean;
  /** This computer's measurement of the model, once it has been run. */
  deviceResult: ModelResult | null;
  /** The registry's or this computer's own benchmark's pick. */
  bestForDevice: boolean;
}

// The benchmark types keep the core's snake_case: the same JSON goes into the shared report.

export interface Device {
  key: string;
  os: string;
  os_version: string;
  cpu: string;
  cores: number;
  memory_gb: number;
  hw_model: string | null;
}

export interface ModelResult {
  model: string;
  /** Seconds of audio per second of compute. */
  speed: number;
  /** Word error rate, 0..1. */
  wer: number;
  audio_seconds: number;
  compute_seconds: number;
  clips: number;
  /** Median compute seconds for a 5 to 10 s clip: the wait after you stop talking. */
  wait_seconds: number;
  /** Compute seconds per clip, kept on this computer and left out of a shared report. */
  clip_seconds?: number[];
}

/** Medians over the shared reports for one model on one device key. */
export interface CommunityModel {
  reports: number;
  speed: number;
  wer: number;
  wait: number;
}

/** What people with the same device key shared. Only `agree >= 3` can choose a model. */
export interface CommunityDevice {
  reports: number;
  pick: string | null;
  agree: number;
  models: Record<string, CommunityModel>;
}

/** The report service's answer to a shared report. */
export interface ShareReply {
  key: string;
  reports: number;
  pick: string | null;
  agree: number;
}

export interface Report {
  schema: number;
  silas: string;
  suite: string;
  device: Device;
  results: ModelResult[];
  pick: string | null;
  ran_at: string;
}

/** `clip` clips of `clips` are finished for `model`. */
export interface BenchmarkProgress {
  model: string;
  clip: number;
  clips: number;
}

export interface HistoryItem {
  id: string;
  ts: string;
  text: string;
  raw: string;
  durationS: number;
  latencyMs: number;
  words: number;
  corrected: string | null;
  /** The file's name when the text came from "Transcribe a file", not a dictation. */
  file?: string;
}

/** Where a file transcription is. `refused` means it never started (and `message` says why). */
export type FileStage = "decoding" | "transcribing" | "waiting" | "done" | "error" | "cancelled" | "refused";

/** A file transcription's progress: `doneS` of `totalS` seconds of audio (`null` when unknown). */
export interface FileProgress {
  name: string;
  stage: FileStage;
  doneS: number;
  totalS: number | null;
  message?: string;
  /** With `done`: the new History entry. */
  item?: HistoryItem;
}

export type Source = "user" | "learned" | "import";

export interface Correction {
  from: string;
  to: string;
  count: number;
  source: Source;
  added: string;
}

export interface Pending {
  from: string;
  to: string;
  count: number;
  lastSeen: string;
}

export interface Term {
  text: string;
  source: Source;
  added: string;
  uses: number;
}

export interface Dictionary {
  terms: Term[];
  corrections: Correction[];
  pending: Pending[];
  fillers: string[];
  snippets: Snippet[];
}

export interface Stats {
  wordsTotal: number;
  wordsToday: number;
  dictations: number;
  avgWpm: number;
  minutesSaved: number;
  learned: number;
}

export interface LearnResult {
  learned: Correction[];
  pending: Pending[];
  terms: string[];
}

export type ProgressStage = "downloading" | "unpacking" | "loading" | "done" | "error";

export interface ModelProgress {
  id: string;
  downloaded: number;
  total: number;
  stage: ProgressStage;
  message?: string;
}

export interface StatusEvent {
  status: Status;
  detail: string | null;
}

/** A short line on why a press did nothing or a dictation stopped. The overlay shows it. */
export interface Notice {
  kind: "error" | "busy" | "stopped";
  message: string;
}

export interface EventMap {
  status: StatusEvent;
  level: number;
  transcript: HistoryItem;
  learned: LearnResult;
  notice: Notice;
  app: AppInfo;
  "model-progress": ModelProgress;
  "benchmark-progress": BenchmarkProgress;
  "benchmark-done": Report;
  "benchmark-error": { message: string };
  "update-available": UpdateInfo;
  "update-progress": UpdateProgress;
  "file-progress": FileProgress;
  /** Files are being dragged over the main window (`active`), or left or were dropped. */
  "file-drag": { active: boolean };
}

export type PermissionKind = "microphone" | "accessibility" | "input";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return inTauri ? invoke<T>(cmd, args) : mockInvoke<T>(cmd, args);
}

/** Subscribe to a core event. Resolves to an unsubscribe function. */
export function on<K extends keyof EventMap>(event: K, cb: (payload: EventMap[K]) => void): Promise<() => void> {
  if (inTauri) return listen<EventMap[K]>(event, (e) => cb(e.payload));
  return mockListen(event, cb as (p: unknown) => void);
}

export const api = {
  getState: () => call<AppState>("get_state"),
  getSettings: () => call<Settings>("get_settings"),
  setSettings: (settings: Settings) => call<Settings>("set_settings", { settings }),

  listModels: () => call<ModelEntry[]>("list_models"),
  downloadModel: (id: string) => call<null>("download_model", { id }),
  selectModel: (id: string, auto = false) => call<null>("select_model", { id, auto }),
  waitDevicePick: () => call<null>("wait_device_pick"),
  deleteModel: (id: string) => call<null>("delete_model", { id }),

  getHistory: (limit: number) => call<HistoryItem[]>("get_history", { limit }),
  correctHistory: (id: string, text: string) => call<LearnResult>("correct_history", { id, text }),
  deleteHistory: (id: string) => call<null>("delete_history", { id }),
  clearHistory: () => call<null>("clear_history"),

  getDictionary: () => call<Dictionary>("get_dictionary"),
  addTerm: (text: string) => call<Dictionary>("add_term", { text }),
  removeTerm: (text: string) => call<Dictionary>("remove_term", { text }),
  addCorrection: (from: string, to: string) => call<Dictionary>("add_correction", { from, to }),
  removeCorrection: (from: string) => call<Dictionary>("remove_correction", { from }),
  addSnippet: (trigger: string, text: string) => call<Dictionary>("add_snippet", { trigger, text }),
  removeSnippet: (trigger: string) => call<Dictionary>("remove_snippet", { trigger }),
  acceptPending: (from: string) => call<Dictionary>("accept_pending", { from }),
  dismissPending: (from: string) => call<Dictionary>("dismiss_pending", { from }),
  importDictionary: (json: string) => call<Dictionary>("import_dictionary", { json }),
  exportDictionary: () => call<string>("export_dictionary"),
  /** Writes the dictionary to the Downloads folder and returns the file path. */
  saveDictionary: () => call<string>("save_dictionary"),

  getStats: () => call<Stats>("get_stats"),
  toggleRecording: () => call<null>("toggle_recording"),
  /** Types the newest History item again into the field with the keyboard. Rejects with the reason when it can't. */
  typeAgain: () => call<null>("type_again"),
  captureHotkey: () => call<string>("capture_hotkey"),
  openPermission: (kind: PermissionKind) => call<null>("open_permission", { kind }),
  /** macOS: clear Silas's Accessibility entry and ask again (fixes a stale grant). */
  resetAccessibility: () => call<null>("reset_accessibility"),
  /** macOS Fix Permission: clear the stale Accessibility entry, then reopen Silas to ask once. */
  fixAccessibility: () => call<null>("fix_accessibility"),
  listMicrophones: () => call<string[]>("list_microphones"),
  showMain: () => call<null>("show_main"),
  getBenchmark: () => call<Report | null>("get_benchmark"),
  /** Resolves when the run ends; progress and the result also arrive as events. */
  runBenchmark: (all: boolean) => call<null>("run_benchmark", { all }),
  /** The exact JSON a share sends, or null before any benchmark. */
  sharePreview: () => call<string | null>("share_preview"),
  /** Sends `payload` (from sharePreview) to the report service. No account, no audio, no text. */
  shareBenchmark: (payload: string) => call<ShareReply>("share_benchmark", { payload }),
  /** Opens a prefilled GitHub issue in the browser; the user reviews and submits it. */
  shareBenchmarkGithub: () => call<null>("share_benchmark_github"),
  /** Shared results for this device, fetched with a short timeout or from the last good copy. */
  getCommunity: () => call<CommunityDevice | null>("get_community"),
  getDevice: () => call<Device>("get_device"),
  checkUpdate: () => call<UpdateInfo | null>("check_update"),
  installUpdate: () => call<null>("install_update"),
  /** Transcribes a file into History. With no path it opens the picker; `null` if that was cancelled. */
  transcribeFile: (path?: string) => call<HistoryItem | null>("transcribe_file", { path: path ?? null }),
  cancelFile: () => call<boolean>("cancel_file"),
};

/** Error text from a rejected invoke (Tauri rejects with a string). */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong.";
}
