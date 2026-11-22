import { api, errorText, type CommunityDevice, type ModelEntry, type ModelProgress, type ModelResult, type Report, type Tier } from "./api";
import { closeShare, downloadModel, notify, openShare, refreshModels, runBenchmark, selectModel, sendShare, shareOnGithub, store, subscribe } from "./store";
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
// Shared results choose a model only when this many reports agree (benchmark::MIN_AGREE).
const MIN_AGREE = 3;

const speedText = (r: ModelResult) =>
  `${Math.round(r.speed)}× real time · a 10\u00a0s dictation in ${(10 / Math.max(r.speed, 0.01)).toFixed(1)}\u00a0s`;
const accuracyText = (r: ModelResult) => `${(100 * Math.max(0, 1 - r.wer)).toFixed(1)}% of words right`;
const waitText = (seconds: number) => `text ${seconds.toFixed(1)}\u00a0s after you stop talking`;
const modelName = (id: string) => store.models.find((m) => m.id === id)?.name ?? id;

function mark(ok: boolean, good: string, bad: string): HTMLElement {
  return h("span", { class: `mark ${ok ? "mark-ok" : "mark-miss"}` }, icon(ok ? "check" : "x", 14), ok ? good : bad);
}

function resultRow(r: ModelResult, isPick: boolean): HTMLElement {
  const name = store.models.find((m) => m.id === r.model)?.name ?? r.model;
  return h(
    "li",
    { class: `bench-row${isPick ? " is-pick" : ""}` },
    h("div", { class: "bench-name" }, h("span", { class: "bench-model" }, name), isPick ? h("span", { class: "badge badge-gold" }, "Best here") : null),
    h("div", { class: "bench-marks" }, mark(r.speed >= MIN_SPEED, "Fast enough", "Too slow"), mark(r.wer <= MAX_WER, "Accurate enough", "Too many misses")),
    h(
      "div",
      { class: "bench-nums" },
      h("span", null, speedText(r)),
      h("span", null, accuracyText(r)),
      r.wait_seconds > 0 ? h("span", null, waitText(r.wait_seconds)) : null,
    ),
  );
}

/** The share sheet: exactly what Send posts, then one tap. */
function shareSheet(): HTMLElement {
  const sheet = store.share!;
  let sent: Report | null = null;
  try {
    sent = JSON.parse(sheet.payload) as Report;
  } catch {
    // The raw JSON below still shows what would be sent.
  }
  const facts: Array<HTMLElement | null> = [];
  if (sent) {
    const d = sent.device;
    facts.push(
      h("dt", null, "Device type"),
      h("dd", null, h("code", null, d.key)),
      h("dt", null, "Hardware"),
      h("dd", null, `${d.cpu}, ${d.cores} cores, ${d.memory_gb}\u00a0GB${d.hw_model ? `, ${d.hw_model}` : ""}`),
      h("dt", null, "OS"),
      h("dd", null, d.os_version),
      h("dt", null, "Silas"),
      h("dd", null, sent.silas),
      ...sent.results.flatMap((r) => [
        h("dt", null, modelName(r.model)),
        h("dd", null, `${r.speed.toFixed(1)}× real time, ${(100 * r.wer).toFixed(1)}% word errors, ${r.wait_seconds.toFixed(2)}\u00a0s wait`),
      ]),
    );
  }
  const reply = sheet.reply;
  const done = reply
    ? h(
        "p",
        { class: "bench-status share-done", role: "status" },
        icon("check", 16),
        reply.agree >= MIN_AGREE && reply.pick
          ? `Sent. ${reply.reports} reports for this device; ${reply.agree} agree on ${modelName(reply.pick)}.`
          : `Sent. ${reply.reports} ${reply.reports === 1 ? "report" : "reports"} for this device so far.`,
      )
    : null;
  return h(
    "div",
    { class: "share-sheet", role: "region", "aria-labelledby": "share-title" },
    h("h3", { id: "share-title", class: "share-title" }, "Share these results"),
    h(
      "p",
      { class: "bench-meta" },
      "This goes to Silas's open report service so people with the same device get the right model. No account, no audio, no text. The service keeps these numbers and the day, never your address.",
    ),
    sent ? h("dl", { class: "share-facts" }, ...facts) : null,
    h("details", { class: "share-raw" }, h("summary", null, "Exact data sent"), h("pre", null, sheet.payload)),
    done,
    sheet.error ? h("p", { class: "error-text", role: "alert" }, sheet.error) : null,
    h(
      "div",
      { class: "card-actions bench-actions" },
      reply
        ? h("button", { class: "btn btn-ghost", type: "button", onclick: () => closeShare() }, "Done")
        : h(
            "button",
            { class: "btn btn-primary", type: "button", disabled: sheet.sending, onclick: () => void sendShare() },
            sheet.sending ? h("span", { class: "spinner sm" }) : icon("upload", 16),
            sheet.sending ? "Sending…" : "Send",
          ),
      reply ? null : h("button", { class: "btn btn-ghost", type: "button", disabled: sheet.sending, onclick: () => closeShare() }, "Cancel"),
      h("button", { class: "link", type: "button", onclick: () => void shareOnGithub() }, "Share on GitHub instead"),
    ),
  );
}

/** Whether shared results are strong enough to choose a model: the same test as benchmark.rs. */
function communityPick(c: CommunityDevice): string | null {
  if (!c.pick || c.agree < MIN_AGREE) return null;
  const m = c.models[c.pick];
  return m && m.speed >= MIN_SPEED && m.wer <= MAX_WER ? c.pick : null;
}

/** One line on what others with this device shared. Fewer than three that agree is information only. */
function communityLine(): HTMLElement | null {
  const c = store.community;
  if (!c || c.reports === 0) return null;
  const pick = communityPick(c);
  const people = `${c.reports} shared ${c.reports === 1 ? "result" : "results"} from this device type`;
  const text = pick
    ? `${people}: ${c.agree} agree on ${modelName(pick)}.`
    : `${people} so far. Silas waits for ${MIN_AGREE} that agree before it uses them.`;
  return h("p", { class: "bench-meta community-line" }, icon("users", 14), text);
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
      h("button", { class: "btn btn-primary", type: "button", onclick: () => void openShare() }, icon("upload", 16), "Share results"),
    ];
    body = h(
      "div",
      { class: "bench-result" },
      h("ul", { class: "bench-list" }, ...report.results.map((r) => resultRow(r, r.model === report.pick && r.speed >= MIN_SPEED && r.wer <= MAX_WER))),
      store.share ? shareSheet() : h("div", { class: "card-actions bench-actions" }, ...buttons),
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
    communityLine(),
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
