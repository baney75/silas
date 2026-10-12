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
  permissions: Permissions;
}

export type TypingSpeed = "steady" | "fast" | "instant";

export interface Settings {
  hotkey: string;
  tapToToggle: boolean;
  model: string;
  language: string;
  removeFillers: boolean;
  trailingSpace: boolean;
  typingSpeed: TypingSpeed;
  newlineAsShiftEnter: boolean;
  learnFromEdits: boolean;
  sounds: boolean;
  showOverlay: boolean;
  launchAtLogin: boolean;
  microphone: string | null;
  onboarded: boolean;
  updateChannel: UpdateChannel;
  autoUpdate: boolean;
}

export type UpdateChannel = "stable" | "nightly";

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

export interface EventMap {
  status: StatusEvent;
  level: number;
  transcript: HistoryItem;
  learned: LearnResult;
  "model-progress": ModelProgress;
  "update-available": UpdateInfo;
  "update-progress": UpdateProgress;
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
  selectModel: (id: string) => call<null>("select_model", { id }),
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
  acceptPending: (from: string) => call<Dictionary>("accept_pending", { from }),
  dismissPending: (from: string) => call<Dictionary>("dismiss_pending", { from }),
  importDictionary: (json: string) => call<Dictionary>("import_dictionary", { json }),
  exportDictionary: () => call<string>("export_dictionary"),
  /** Writes the dictionary to the Downloads folder and returns the file path. */
  saveDictionary: () => call<string>("save_dictionary"),

  getStats: () => call<Stats>("get_stats"),
  toggleRecording: () => call<null>("toggle_recording"),
  captureHotkey: () => call<string>("capture_hotkey"),
  openPermission: (kind: PermissionKind) => call<null>("open_permission", { kind }),
  listMicrophones: () => call<string[]>("list_microphones"),
  showMain: () => call<null>("show_main"),
  checkUpdate: () => call<UpdateInfo | null>("check_update"),
  installUpdate: () => call<null>("install_update"),
};

/** Error text from a rejected invoke (Tauri rejects with a string). */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong.";
}
