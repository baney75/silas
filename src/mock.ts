// In-browser stand-in for the Rust core. Used only when the page runs outside Tauri,
// so designers can preview every screen with `bunx vite`. Query flags:
//   ?onboarding        start in onboarding
//   ?platform=windows  pretend to be another OS (macos | windows | linux)
//   ?empty             no history, no dictionary
//   ?update            an update is waiting
//   ?unmeasured        this computer has not been benchmarked yet
//   ?benchmarking      a benchmark is running when the page opens

import type {
  AppState,
  Correction,
  Dictionary,
  HistoryItem,
  Device,
  LearnResult,
  ModelEntry,
  ModelResult,
  Pending,
  Platform,
  Report,
  Settings,
  Stats,
  Status,
} from "./api";

type Listener = (payload: unknown) => void;
const listeners = new Map<string, Set<Listener>>();

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((cb) => cb(payload));
}

export function mockListen(event: string, cb: Listener): Promise<() => void> {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(cb);
  return Promise.resolve(() => listeners.get(event)?.delete(cb));
}

const params = new URLSearchParams(typeof location !== "undefined" ? location.search : "");
const empty = params.has("empty");
const platform: Platform =
  (params.get("platform") as Platform | null) ??
  (/Mac/.test(typeof navigator !== "undefined" ? navigator.platform : "") ? "macos" : "windows");

const now = Date.now();
const iso = (msAgo: number) => new Date(now - msAgo).toISOString();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

const hotkeyLabels: Record<string, string> = {
  CmdRight: "Right ⌘",
  CmdLeft: "Left ⌘",
  CtrlRight: "Right Ctrl",
  CtrlLeft: "Left Ctrl",
  AltRight: platform === "macos" ? "Right ⌥" : "Right Alt",
  AltLeft: platform === "macos" ? "Left ⌥" : "Left Alt",
  ShiftRight: "Right ⇧",
  Fn: "Fn",
};

function labelFor(key: string): string {
  return hotkeyLabels[key] ?? key;
}

const settings: Settings = {
  hotkey: platform === "macos" ? "CmdRight" : "CtrlRight",
  tapToToggle: true,
  model: "parakeet-v3",
  modelSource: "default",
  language: "en",
  removeFillers: true,
  trailingSpace: true,
  typingSpeed: "fast",
  newlineAsShiftEnter: true,
  learnFromEdits: true,
  sounds: true,
  showOverlay: true,
  launchAtLogin: false,
  microphone: null,
  onboarded: !params.has("onboarding"),
  updateChannel: platform === "macos" ? "stable" : "nightly",
  autoUpdate: true,
};

const onboarding = params.has("onboarding");

const state: AppState = {
  status: onboarding ? "needs_model" : "ready",
  detail: null,
  platform,
  version: "0.1.0",
  model: onboarding ? null : "parakeet-v3",
  hotkey: settings.hotkey,
  hotkeyLabel: labelFor(settings.hotkey),
  permissions: {
    microphone: onboarding ? "unknown" : "granted",
    accessibility: onboarding ? "denied" : "granted",
  },
};

const models: ModelEntry[] = [
  {
    id: "parakeet-v3",
    name: "Parakeet v3",
    tier: "fast",
    tagline: "Fast and accurate. The right choice for most people.",
    sizeMb: 478,
    languages: "25 European languages",
    installed: !onboarding,
    active: !onboarding,
    recommended: true,
    deviceResult: null,
    bestForDevice: false,
  },
  {
    id: "cohere-int4",
    name: "Cohere Transcribe",
    tier: "best",
    tagline: "The most accurate, for long dictation and hard names. Slower.",
    sizeMb: 2048,
    languages: "English plus 13 languages",
    installed: false,
    active: false,
    recommended: false,
    deviceResult: null,
    bestForDevice: false,
  },
];

// ---- Benchmark ----

const device: Device = {
  key: `${platform}/apple-m5/10`,
  os: platform,
  os_version: platform === "macos" ? "macOS 27.2" : platform === "windows" ? "Windows 11 (26200)" : "Ubuntu 26.04",
  cpu: "Apple M5",
  cores: 10,
  memory_gb: 32,
  hw_model: platform === "macos" ? "Mac17,2" : null,
};

/** What each mock model scores, so every state of the page can be previewed. */
const mockScores: Record<string, ModelResult> = {
  "parakeet-v3": { model: "parakeet-v3", speed: 23.4, wer: 0.019, audio_seconds: 62.2, compute_seconds: 2.66, clips: 9 },
  "cohere-int4": { model: "cohere-int4", speed: 4.1, wer: 0.012, audio_seconds: 62.2, compute_seconds: 15.2, clips: 9 },
};

const SUITE = "librispeech-test-clean-9clips-v1";
const BAR_SPEED = 10;
const BAR_WER = 0.08;

/** The same rule as benchmark::pick in the core. */
function pickModel(results: ModelResult[]): string | null {
  const passing = results.filter((r) => r.speed >= BAR_SPEED && r.wer <= BAR_WER);
  const fastest = (rs: ModelResult[]) => rs.reduce((a, b) => (b.speed > a.speed ? b : a)).model;
  if (passing.length) {
    const best = Math.min(...passing.map((r) => r.wer));
    return fastest(passing.filter((r) => r.wer - best <= 0.005));
  }
  const accurate = results.filter((r) => r.wer <= BAR_WER);
  if (accurate.length) return fastest(accurate);
  return results.length ? fastest(results) : null;
}

let report: Report | null = null;
let benchmarking = false;

function applyReport(next: Report) {
  report = next;
  for (const m of models) {
    m.deviceResult = next.results.find((r) => r.model === m.id) ?? null;
    // Only a pick that clears both bars is "best for this computer".
    const best = next.results.find((r) => r.model === next.pick);
    const passes = !!best && best.speed >= BAR_SPEED && best.wer <= BAR_WER;
    m.bestForDevice = passes && next.pick === m.id;
    m.recommended = passes ? next.pick === m.id : m.id === "parakeet-v3";
  }
}

async function benchmark(all: boolean) {
  if (benchmarking) throw "A benchmark is already running";
  benchmarking = true;
  try {
    const ids = [state.model ?? "parakeet-v3", ...(all ? models.filter((m) => m.installed && m.id !== state.model).map((m) => m.id) : [])];
    const fresh: ModelResult[] = [];
    for (const id of ids) {
      for (let clip = 0; clip <= 9; clip++) {
        emit("benchmark-progress", { model: id, clip, clips: 9 });
        await delay(id === "cohere-int4" ? 420 : 180);
      }
      fresh.push({ ...mockScores[id], speed: mockScores[id].speed * (0.95 + Math.random() * 0.1) });
    }
    const kept = (report?.results ?? []).filter((r) => !fresh.some((f) => f.model === r.model));
    const results = [...kept, ...fresh].sort((a, b) => models.findIndex((m) => m.id === a.model) - models.findIndex((m) => m.id === b.model));
    const next: Report = { schema: 1, silas: state.version, suite: SUITE, device, results, pick: pickModel(results), ran_at: new Date().toISOString() };
    applyReport(next);
    if (next.pick && next.pick !== settings.model && settings.modelSource !== "user" && models.find((m) => m.id === next.pick)?.installed) {
      settings.model = next.pick;
      settings.modelSource = "benchmark";
      state.model = next.pick;
      models.forEach((m) => (m.active = m.id === next.pick));
    }
    emit("benchmark-done", next);
  } finally {
    benchmarking = false;
  }
}

if (!onboarding && !params.has("unmeasured")) {
  report = { schema: 1, silas: "0.1.0", suite: SUITE, device, results: [mockScores["parakeet-v3"]], pick: "parakeet-v3", ran_at: iso(2 * HOUR) };
  applyReport(report);
}
if (params.has("benchmarking")) {
  window.setTimeout(() => void benchmark(false).catch(() => {}), 600);
}

const sample: Array<[string, number, number]> = [
  ["Can you check whether Parish1 is on AC power before we start the render tonight?", 4.8, 310],
  ["Remind me to send the deck to Claire on Thursday and ask about the venue.", 4.1, 280],
  ["The coordinator on Chapel1 should retry the job twice and then page me.", 4.6, 295],
  ["Let's move the standup to nine thirty so the West Coast folks can join.", 3.9, 260],
  ["Draft a short thank-you note to the Hendersons for dinner on Sunday.", 3.6, 240],
  ["Silas, new paragraph. The second option costs less but takes a week longer.", 5.2, 330],
  ["Order more coffee filters and the good olive oil.", 2.4, 190],
];

let history: HistoryItem[] = empty
  ? []
  : sample.map(([text, durationS, latencyMs], i) => {
      const ago = [2 * MIN, 18 * MIN, 1.5 * HOUR, 4 * HOUR, DAY + 2 * HOUR, 2 * DAY, 5 * DAY][i];
      const ts = now - ago;
      return {
        id: String(ts),
        ts: new Date(ts).toISOString(),
        text,
        raw: text.replace("Parish1", "Paris one").replace("Chapel1", "chapel one"),
        durationS,
        latencyMs,
        words: text.split(/\s+/).length,
        corrected: null,
      };
    });

const dict: Dictionary = empty
  ? { terms: [], corrections: [], pending: [], fillers: ["um", "uh", "you know"] }
  : {
      terms: [
        { text: "Parish1", source: "learned", added: iso(3 * DAY), uses: 14 },
        { text: "Chapel1", source: "user", added: iso(9 * DAY), uses: 22 },
        { text: "Tauri", source: "user", added: iso(6 * DAY), uses: 5 },
        { text: "Silvanus", source: "user", added: iso(2 * DAY), uses: 1 },
        { text: "BarnLabs", source: "import", added: iso(12 * DAY), uses: 9 },
        { text: "Kubernetes", source: "import", added: iso(12 * DAY), uses: 3 },
      ],
      corrections: [
        { from: "Paris one", to: "Parish1", count: 14, source: "learned", added: iso(3 * DAY) },
        { from: "chapel one", to: "Chapel1", count: 22, source: "user", added: iso(9 * DAY) },
        { from: "barn labs", to: "BarnLabs", count: 9, source: "import", added: iso(12 * DAY) },
        { from: "towery", to: "Tauri", count: 3, source: "learned", added: iso(DAY) },
      ],
      pending: [
        { from: "sea law", to: "Silas", count: 1, lastSeen: iso(40 * MIN) },
        { from: "Hendersen", to: "Henderson", count: 1, lastSeen: iso(DAY) },
      ],
      fillers: ["um", "uh", "you know"],
    };

function stats(): Stats {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const wordsTotal = 18_420 + history.reduce((n, h) => n + h.words, 0);
  const wordsToday = history.filter((h) => new Date(h.ts) >= today).reduce((n, h) => n + h.words, 0);
  return {
    wordsTotal,
    wordsToday: empty ? 0 : wordsToday + 412,
    dictations: 640 + history.length,
    avgWpm: 148,
    minutesSaved: Math.round(wordsTotal / 40 - wordsTotal / 148),
    learned: dict.corrections.filter((c) => c.source === "learned").length,
  };
}

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

function setStatus(status: Status, detail: string | null = null) {
  state.status = status;
  state.detail = detail;
  emit("status", { status, detail });
}

let recording: { timer: number; started: number } | null = null;
const phrases = [
  "Push the fix to Parish1 and let me know when the build is green.",
  "Tell Mark I'll call him back after lunch.",
  "The meeting notes are in the shared folder under October.",
];

function startRecording() {
  setStatus("recording");
  let t = 0;
  const timer = window.setInterval(() => {
    t += 1;
    const speech = 0.35 + 0.35 * Math.sin(t / 3) + Math.random() * 0.3;
    emit("level", Math.max(0, Math.min(1, speech * (t % 40 < 32 ? 1 : 0.15))));
  }, 33);
  recording = { timer, started: Date.now() };
}

async function stopRecording() {
  if (!recording) return;
  window.clearInterval(recording.timer);
  const durationS = (Date.now() - recording.started) / 1000;
  recording = null;
  setStatus("transcribing");
  await delay(700);
  const text = phrases[history.length % phrases.length];
  const ts = Date.now();
  const item: HistoryItem = {
    id: String(ts),
    ts: new Date(ts).toISOString(),
    text,
    raw: text,
    durationS: Math.round(durationS * 10) / 10,
    latencyMs: 240 + Math.round(Math.random() * 120),
    words: text.split(/\s+/).length,
    corrected: null,
  };
  history.unshift(item);
  setStatus("typing");
  emit("transcript", item);
  await delay(500);
  setStatus("ready");
}

/** Mock of the core's diff: the first differing word span between old and new. */
function diffSpan(before: string, after: string): { from: string; to: string } | null {
  const a = before.split(/\s+/);
  const b = after.split(/\s+/);
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  let j = 0;
  while (j < a.length - i && j < b.length - i && a[a.length - 1 - j] === b[b.length - 1 - j]) j++;
  const from = a.slice(i, a.length - j).join(" ").replace(/[.,!?;:]+$/, "");
  const to = b.slice(i, b.length - j).join(" ").replace(/[.,!?;:]+$/, "");
  if (!from || !to || from === to) return null;
  return { from, to };
}

function learn(from: string, to: string): LearnResult {
  const result: LearnResult = { learned: [], pending: [], terms: [] };
  const existing = dict.pending.find((p) => p.from.toLowerCase() === from.toLowerCase());
  const correction: Correction = { from, to, count: 1, source: "user", added: new Date().toISOString() };
  // Teaching from History is explicit, so the core trusts it at once.
  dict.pending = dict.pending.filter((p) => p !== existing);
  dict.corrections = [correction, ...dict.corrections.filter((c) => c.from !== from)];
  result.learned.push(correction);
  if (/[A-Z0-9]/.test(to) && !dict.terms.some((t) => t.text === to)) {
    dict.terms.unshift({ text: to, source: "learned", added: new Date().toISOString(), uses: 0 });
    result.terms.push(to);
  }
  return result;
}

const keyMap: Record<string, string> = {
  MetaRight: "CmdRight",
  MetaLeft: "CmdLeft",
  ControlRight: "CtrlRight",
  ControlLeft: "CtrlLeft",
  AltRight: "AltRight",
  AltLeft: "AltLeft",
  ShiftRight: "ShiftRight",
  ShiftLeft: "ShiftLeft",
};

function captureKey(): Promise<string> {
  return new Promise((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      window.removeEventListener("keydown", onKey, true);
      reject("No key pressed within 10 seconds.");
    }, 10_000);
    function onKey(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();
      window.clearTimeout(timeout);
      window.removeEventListener("keydown", onKey, true);
      resolve(keyMap[e.code] ?? e.code.replace(/^Key/, ""));
    }
    window.addEventListener("keydown", onKey, true);
  });
}

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

export async function mockInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  await delay(60);
  const out = (v: unknown) => clone(v) as T;
  switch (cmd) {
    case "get_state":
      return out(state);
    case "get_settings":
      return out(settings);
    case "set_settings": {
      // Like the core: the model and who chose it are not changed through here.
      const next = { ...(args.settings as Settings), model: settings.model, modelSource: settings.modelSource };
      Object.assign(settings, next);
      state.hotkey = settings.hotkey;
      state.hotkeyLabel = labelFor(settings.hotkey);
      return out(settings);
    }
    case "list_models":
      return out(models);
    case "download_model": {
      const m = models.find((x) => x.id === args.id)!;
      const total = m.sizeMb * 1024 * 1024;
      for (let i = 1; i <= 40; i++) {
        await delay(70);
        emit("model-progress", { id: m.id, downloaded: (total * i) / 40, total, stage: "downloading" });
      }
      emit("model-progress", { id: m.id, downloaded: total, total, stage: "unpacking" });
      await delay(500);
      m.installed = true;
      emit("model-progress", { id: m.id, downloaded: total, total, stage: "done" });
      return null as T;
    }
    case "get_benchmark":
      return out(report);
    case "wait_device_pick":
      await delay(300);
      return null as T;
    case "get_device":
      return out(device);
    case "run_benchmark":
      await benchmark(Boolean(args.all));
      return null as T;
    case "share_benchmark":
      if (!report) throw "Run the benchmark first";
      window.open("https://github.com/baney75/silas/issues/new?template=device-report.yml", "_blank", "noopener");
      return null as T;
    case "select_model": {
      // Like the core: setup's automatic choice never overrides the user's.
      if (args.auto && settings.modelSource === "user") return null as T;
      setStatus("loading", "Loading model");
      await delay(900);
      models.forEach((m) => (m.active = m.id === args.id));
      settings.model = String(args.id);
      if (!args.auto) settings.modelSource = "user";
      state.model = String(args.id);
      setStatus("ready");
      return null as T;
    }
    case "delete_model": {
      const m = models.find((x) => x.id === args.id)!;
      m.installed = false;
      m.active = false;
      return null as T;
    }
    case "get_history":
      return out(history.slice(0, Number(args.limit) || 100));
    case "correct_history": {
      const item = history.find((h) => h.id === args.id)!;
      const text = String(args.text);
      const span = diffSpan(item.corrected ?? item.text, text);
      item.corrected = text;
      return out(span ? learn(span.from, span.to) : { learned: [], pending: [], terms: [] });
    }
    case "delete_history":
      history = history.filter((h) => h.id !== args.id);
      return null as T;
    case "clear_history":
      history = [];
      return null as T;
    case "get_dictionary":
      return out(dict);
    case "add_term":
      dict.terms.unshift({ text: String(args.text), source: "user", added: new Date().toISOString(), uses: 0 });
      return out(dict);
    case "remove_term":
      dict.terms = dict.terms.filter((t) => t.text !== args.text);
      return out(dict);
    case "add_correction":
      dict.corrections.unshift({
        from: String(args.from),
        to: String(args.to),
        count: 0,
        source: "user",
        added: new Date().toISOString(),
      });
      return out(dict);
    case "remove_correction":
      dict.corrections = dict.corrections.filter((c) => c.from !== args.from);
      return out(dict);
    case "accept_pending": {
      const p = dict.pending.find((x) => x.from === args.from) as Pending;
      dict.pending = dict.pending.filter((x) => x !== p);
      dict.corrections.unshift({ from: p.from, to: p.to, count: p.count, source: "learned", added: new Date().toISOString() });
      return out(dict);
    }
    case "dismiss_pending":
      dict.pending = dict.pending.filter((x) => x.from !== args.from);
      return out(dict);
    case "import_dictionary": {
      const parsed = JSON.parse(String(args.json)) as Partial<Dictionary>;
      for (const t of parsed.terms ?? []) if (!dict.terms.some((x) => x.text === t.text)) dict.terms.push({ ...t, source: "import" });
      for (const c of parsed.corrections ?? [])
        if (!dict.corrections.some((x) => x.from === c.from)) dict.corrections.push({ ...c, source: "import" });
      return out(dict);
    }
    case "save_dictionary":
      return "~/Downloads/silas-dictionary.json" as T;
    case "export_dictionary":
      return JSON.stringify({ app: "silas", version: 1, ...dict }, null, 2) as T;
    case "get_stats":
      return out(stats());
    case "toggle_recording":
      if (recording) void stopRecording();
      else startRecording();
      return null as T;
    case "capture_hotkey":
      return (await captureKey()) as T;
    case "open_permission": {
      const kind = args.kind as "microphone" | "accessibility";
      window.setTimeout(() => (state.permissions[kind] = "granted"), 2500);
      return null as T;
    }
    case "reset_accessibility":
      window.setTimeout(() => (state.permissions.accessibility = "granted"), 2500);
      return null as T;
    case "list_microphones":
      return out(["MacBook Pro Microphone", "Shure MV7+", "AirPods Pro"]);
    case "show_main":
      return null as T;
    case "check_update":
      await delay(700);
      return out(
        params.has("update")
          ? { version: "0.1.2", currentVersion: state.version, notes: "Faster first dictation after wake. Fixes a stuck key on some keyboards.", date: iso(DAY), channel: settings.updateChannel }
          : null,
      );
    case "install_update": {
      const total = 18 * 1024 * 1024;
      for (let i = 1; i <= 30; i++) {
        await delay(60);
        emit("update-progress", { downloaded: (total * i) / 30, total });
      }
      return null as T;
    }
    default:
      throw `Unknown command: ${cmd}`;
  }
}

/** Drives the overlay preview in a browser: recording, writing, learned, on a loop. */
if (params.has("update")) {
  window.setTimeout(
    () => emit("update-available", { version: "0.1.2", currentVersion: "0.1.0", notes: "Faster first dictation after wake.", date: iso(DAY), channel: settings.updateChannel }),
    1200,
  );
}

export function startOverlayDemo() {
  const loop = async () => {
    startRecording();
    await delay(4200);
    if (recording) window.clearInterval(recording.timer);
    recording = null;
    setStatus("transcribing");
    await delay(1800);
    setStatus("typing");
    await delay(700);
    emit("learned", {
      learned: [{ from: "Paris one", to: "Parish1", count: 2, source: "learned", added: new Date().toISOString() }],
      pending: [],
      terms: [],
    });
    setStatus("ready");
    await delay(2600);
    void loop();
  };
  void loop();
}
