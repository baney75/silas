import { api, errorText, type ModelEntry, type ModelProgress, type ModelResult, type Tier } from "./api";
import { downloadModel, notify, refreshModels, runBenchmark, selectModel, shareBenchmark, store, subscribe } from "./store";
import { clear, h, icon, megabytes, progressBar, toast } from "./ui";

export const tierLabel: Record<Tier, string> = { fast: "Fast", balanced: "Balanced", best: "Best" };

const stageText: Record<ModelProgress["stage"], string> = {
  downloading: "Downloading",
  unpacking: "Unpacking",
  loading: "Loading",
  done: "Ready",
  error: "Failed",
};

export function progressView(p: ModelProgress): HTMLElement {
  const fraction = p.stage === "downloading" && p.total > 0 ? p.downloaded / p.total : p.stage === "done" ? 1 : null;
  const detail =
    p.stage === "downloading" && p.total > 0
      ? `${megabytes(p.downloaded / 1048576)} of ${megabytes(p.total / 1048576)}`
      : p.message ?? "";
  return h(
    "div",
    { class: "dl" },
    progressBar(fraction, `${stageText[p.stage]} model`),
    h(
      "div",
      { class: "dl-meta" },
      h("span", null, stageText[p.stage], fraction !== null && p.stage !== "done" ? ` ${Math.round(fraction * 100)}%` : "…"),
      h("span", null, detail),
    ),
  );
}

let confirmRemove: string | null = null;

export function modelCard(m: ModelEntry, opts: { compact?: boolean } = {}): HTMLElement {
  const p = store.progress[m.id];
  const busy = !!p && p.stage !== "done" && p.stage !== "error";
  const loading = m.active && store.state.status === "loading";

  let actions: HTMLElement;
  if (busy) {
    actions = progressView(p);
  } else if (!m.installed) {
    actions = h(
      "div",
      { class: "card-actions" },
      h("button", { class: "btn btn-primary", type: "button", onclick: () => void downloadModel(m.id) }, icon("download", 16), `Download · ${megabytes(m.sizeMb)}`),
    );
  } else if (m.active) {
    actions = h(
      "div",
      { class: "card-actions" },
      h("span", { class: "state-pill active" }, h("span", { class: "dot" }), loading ? "Loading…" : "In use"),
    );
  } else {
    actions = h(
      "div",
      { class: "card-actions" },
      h("button", { class: "btn btn-primary", type: "button", onclick: () => void selectModel(m.id) }, "Use this model"),
      opts.compact
        ? null
        : h(
            "button",
            {
              class: `btn ${confirmRemove === m.id ? "btn-danger" : "btn-ghost"}`,
              type: "button",
              onclick: async () => {
                if (confirmRemove !== m.id) {
                  confirmRemove = m.id;
                  notify("models");
                  window.setTimeout(() => {
                    if (confirmRemove === m.id) {
                      confirmRemove = null;
                      notify("models");
                    }
                  }, 3000);
                  return;
                }
                confirmRemove = null;
                try {
                  await api.deleteModel(m.id);
                  toast(`Removed ${m.name}`);
                } catch (e) {
                  toast(errorText(e), "error");
                }
                void refreshModels();
              },
            },
            confirmRemove === m.id ? `Remove ${megabytes(m.sizeMb)}?` : "Remove",
          ),
    );
  }

  return h(
    "article",
    { class: `model-card${m.active ? " is-active" : ""}${m.recommended ? " is-recommended" : ""}` },
    h(
      "div",
      { class: "model-top" },
      h("span", { class: `tier tier-${m.tier}` }, tierLabel[m.tier]),
      m.bestForDevice ? h("span", { class: "badge badge-gold" }, "Best for this computer") : m.recommended ? h("span", { class: "badge badge-gold" }, "Recommended") : null,
      m.installed && !m.active ? h("span", { class: "badge" }, "Installed") : null,
    ),
    h("h3", { class: "model-name" }, m.name),
    h("p", { class: "model-tagline" }, m.tagline),
    h("dl", { class: "model-facts" }, h("dt", null, "Size"), h("dd", null, megabytes(m.sizeMb)), h("dt", null, "Languages"), h("dd", null, m.languages)),
    actions,
    p?.stage === "error" ? h("p", { class: "error-text" }, p.message ?? "Download failed. Try again.") : null,
  );
}

// The bars a model has to clear on this computer. Keep in step with benchmark.rs.
const MIN_SPEED = 10;
const MAX_WER = 0.08;

const speedText = (r: ModelResult) =>
  `${Math.round(r.speed)}× real time · a 10 s dictation in ${(10 / Math.max(r.speed, 0.01)).toFixed(1)} s`;
const accuracyText = (r: ModelResult) => `${(100 * (1 - r.wer)).toFixed(1)}% of words right`;

function mark(ok: boolean, good: string, bad: string): HTMLElement {
  return h("span", { class: `mark ${ok ? "mark-ok" : "mark-miss"}` }, icon(ok ? "check" : "x", 14), ok ? good : bad);
}

function resultRow(r: ModelResult, isPick: boolean): HTMLElement {
  const name = store.models.find((m) => m.id === r.model)?.name ?? r.model;
  return h(
    "li",
    { class: `bench-row${isPick ? " is-pick" : ""}` },
    h("div", { class: "bench-name" }, name, isPick ? h("span", { class: "badge badge-gold" }, "Best here") : null),
    h("div", { class: "bench-num" }, speedText(r)),
    h("div", { class: "bench-num" }, accuracyText(r)),
    h("div", { class: "bench-marks" }, mark(r.speed >= MIN_SPEED, "Fast enough", "Too slow"), mark(r.wer <= MAX_WER, "Accurate enough", "Too many misses")),
  );
}

function thisComputer(): HTMLElement {
  const d = store.device;
  const run = store.bench;
  const report = store.benchmark;
  const running = !!run?.running;
  const installed = store.models.filter((m) => m.installed).length;
  const noModel = installed === 0 || store.state.model === null;

  let body: HTMLElement;
  if (running) {
    const p = run!.progress;
    const name = p ? store.models.find((m) => m.id === p.model)?.name ?? p.model : null;
    body = h(
      "div",
      { class: "bench-running" },
      h("p", { class: "bench-status" }, h("span", { class: "spinner sm" }), "Checking which model suits this computer…"),
      progressBar(p && p.clips > 0 ? p.clip / p.clips : null, "Benchmark progress"),
      h("p", { class: "bench-meta" }, p ? `${name} · clip ${Math.min(p.clip + 1, p.clips)} of ${p.clips}` : "Getting ready…"),
    );
  } else if (report && report.results.length) {
    const buttons = [
      h("button", { class: "btn btn-ghost", type: "button", disabled: noModel, onclick: () => void runBenchmark(false) }, "Run again"),
      installed > 1
        ? h("button", { class: "btn btn-ghost", type: "button", onclick: () => void runBenchmark(true) }, "Include all installed models")
        : null,
      h("button", { class: "btn btn-primary", type: "button", onclick: () => void shareBenchmark() }, icon("upload", 16), "Share results"),
    ];
    body = h(
      "div",
      null,
      h("ul", { class: "bench-list" }, ...report.results.map((r) => resultRow(r, r.model === report.pick && r.speed >= MIN_SPEED && r.wer <= MAX_WER))),
      h("div", { class: "card-actions bench-actions" }, ...buttons),
      h(
        "p",
        { class: "bench-meta" },
        "Opens a GitHub issue with your device type, OS, Silas version and these numbers. No audio or text leaves this computer.",
      ),
    );
  } else {
    body = h(
      "div",
      { class: "bench-empty" },
      h("p", { class: "bench-status" }, "Not measured yet"),
      h(
        "div",
        { class: "card-actions" },
        h("button", { class: "btn btn-primary", type: "button", disabled: noModel, onclick: () => void runBenchmark(false) }, "Run benchmark"),
      ),
    );
  }

  return h(
    "section",
    { class: "device-panel", "aria-labelledby": "device-title" },
    h("h2", { id: "device-title", class: "device-title" }, "This computer"),
    d ? h("p", { class: "device-line" }, `${d.cpu} · ${d.os_version}`) : null,
    body,
    run?.error && !running ? h("p", { class: "error-text" }, `The benchmark didn't finish. ${run.error}`) : null,
  );
}

export function mountModels(root: HTMLElement): () => void {
  const device = h("div");
  const grid = h("div", { class: "model-grid" });
  root.append(
    h(
      "div",
      { class: "page-head" },
      h(
        "div",
        null,
        h("h1", null, "Model"),
        h("p", { class: "lede" }, "Every model runs on this computer. Pick the one that fits your machine and the way you talk."),
      ),
    ),
    device,
    grid,
    h("p", { class: "footnote" }, icon("lock", 14), "Audio never leaves this computer. Models download once, then work offline."),
  );
  const draw = () => clear(grid, ...store.models.map((m) => modelCard(m)));
  const drawDevice = () => clear(device, thisComputer());
  draw();
  drawDevice();
  const stopModels = subscribe(["models", "progress", "state"], () => {
    draw();
    drawDevice();
  });
  const stopBench = subscribe(["benchmark"], drawDevice);
  return () => {
    stopModels();
    stopBench();
  };
}
