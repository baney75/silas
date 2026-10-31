// App-wide data and a tiny publish/subscribe. Views read from `store` and
// subscribe to the topics they draw from.

import {
  api,
  errorText,
  type AppState,
  type BenchmarkProgress,
  type Device,
  type Dictionary,
  type HistoryItem,
  type ModelEntry,
  type ModelProgress,
  type Report,
  type Settings,
  type Stats,
  type Status,
  type UpdateInfo,
  type UpdateProgress,
} from "./api";
import { toast } from "./ui";

export type Topic = "state" | "settings" | "models" | "history" | "dict" | "stats" | "progress" | "update" | "benchmark";

export const store = {
  state: null as unknown as AppState,
  settings: null as unknown as Settings,
  models: [] as ModelEntry[],
  history: [] as HistoryItem[],
  dict: { terms: [], corrections: [], pending: [], fillers: [] } as Dictionary,
  stats: null as Stats | null,
  progress: {} as Record<string, ModelProgress>,
  microphones: [] as string[],
  device: null as Device | null,
  /** The last saved benchmark of this computer. */
  benchmark: null as Report | null,
  /** The benchmark in flight (or its last error); `null` when none has run since launch. */
  bench: null as BenchRun | null,
  update: null as UpdateInfo | null,
  /** Result of the last manual check: "none" means up to date. */
  updateCheck: "idle" as "idle" | "checking" | "none" | "found" | "error",
  updateProgress: null as UpdateProgress | null,
};

export interface BenchRun {
  running: boolean;
  progress: BenchmarkProgress | null;
  error: string | null;
}

const subs = new Map<Topic, Set<() => void>>();

export function subscribe(topics: Topic[], fn: () => void): () => void {
  for (const t of topics) {
    if (!subs.has(t)) subs.set(t, new Set());
    subs.get(t)!.add(fn);
  }
  return () => topics.forEach((t) => subs.get(t)?.delete(fn));
}

export function notify(topic: Topic) {
  // Copy first: a subscriber may unsubscribe while we iterate.
  [...(subs.get(topic) ?? [])].forEach((fn) => fn());
}

export const HISTORY_LIMIT = 200;

export async function refreshState() {
  store.state = await api.getState();
  notify("state");
}
export async function refreshModels() {
  store.models = await api.listModels();
  notify("models");
}
export async function refreshHistory() {
  store.history = await api.getHistory(HISTORY_LIMIT);
  notify("history");
}
export async function refreshDict() {
  store.dict = await api.getDictionary();
  notify("dict");
}
export async function refreshStats() {
  store.stats = await api.getStats();
  notify("stats");
}

export async function refreshBenchmark() {
  store.benchmark = await api.getBenchmark();
  notify("benchmark");
}

export function setDict(d: Dictionary) {
  store.dict = d;
  notify("dict");
}

/** Apply a settings change at once, then persist it. Reverts and explains on failure. */
export async function updateSettings(patch: Partial<Settings>) {
  const before = store.settings;
  store.settings = { ...before, ...patch };
  notify("settings");
  try {
    store.settings = await api.setSettings(store.settings);
    notify("settings");
    if ("hotkey" in patch || "model" in patch) await refreshState();
  } catch (e) {
    store.settings = before;
    notify("settings");
    toast(`Couldn't save that setting. ${errorText(e)}`, "error");
  }
}

export const statusText: Record<Status, string> = {
  loading: "Loading model…",
  needs_model: "No model yet",
  ready: "Ready",
  recording: "Listening",
  transcribing: "Writing…",
  typing: "Typing…",
  error: "Something went wrong",
};

export function hotkeyLabel(): string {
  return store.state?.hotkeyLabel ?? "Right ⌘";
}

/** Download a model; progress arrives through the model-progress event. */
export async function downloadModel(id: string) {
  store.progress[id] = { id, downloaded: 0, total: 0, stage: "downloading" };
  notify("progress");
  try {
    await api.downloadModel(id);
  } catch (e) {
    delete store.progress[id];
    notify("progress");
    toast(`Download failed. ${errorText(e)}`, "error", 6000);
  }
  await refreshModels();
}

export async function selectModel(id: string, auto = false) {
  try {
    await api.selectModel(id, auto);
    store.settings = { ...store.settings, model: id };
    notify("settings");
  } catch (e) {
    toast(`Couldn't load that model. ${errorText(e)}`, "error", 6000);
  }
  await Promise.all([refreshModels(), refreshState()]);
}

/** Measure the active model, or every installed one. Progress arrives as events. */
export async function runBenchmark(all: boolean) {
  store.bench = { running: true, progress: null, error: null };
  notify("benchmark");
  try {
    await api.runBenchmark(all);
  } catch (e) {
    store.bench = { running: false, progress: null, error: errorText(e) };
    notify("benchmark");
    return;
  }
  // The benchmark-done event has normally landed already; this covers one that didn't.
  await Promise.all([refreshBenchmark(), refreshModels()]);
  store.bench = { running: false, progress: null, error: null };
  notify("benchmark");
}

export async function shareBenchmark() {
  try {
    await api.shareBenchmark();
  } catch (e) {
    toast(`Couldn't open the report. ${errorText(e)}`, "error", 6000);
  }
}

export async function checkForUpdate() {
  store.updateCheck = "checking";
  notify("update");
  try {
    store.update = await api.checkUpdate();
    store.updateCheck = store.update ? "found" : "none";
  } catch (e) {
    store.updateCheck = "error";
    toast(`Couldn't check for updates. ${errorText(e)}`, "error");
  }
  notify("update");
}

/** Downloads, installs and relaunches. Progress arrives through update-progress. */
export async function installUpdate() {
  store.updateProgress = { downloaded: 0, total: null };
  notify("update");
  try {
    await api.installUpdate();
  } catch (e) {
    store.updateProgress = null;
    notify("update");
    toast(`The update didn't install. ${errorText(e)}`, "error", 6000);
  }
}
