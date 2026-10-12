import { api, errorText, type ModelEntry, type ModelProgress, type Tier } from "./api";
import { downloadModel, notify, refreshModels, selectModel, store, subscribe } from "./store";
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
      m.recommended ? h("span", { class: "badge badge-gold" }, "Recommended") : null,
      m.installed && !m.active ? h("span", { class: "badge" }, "Installed") : null,
    ),
    h("h3", { class: "model-name" }, m.name),
    h("p", { class: "model-tagline" }, m.tagline),
    h("dl", { class: "model-facts" }, h("dt", null, "Size"), h("dd", null, megabytes(m.sizeMb)), h("dt", null, "Languages"), h("dd", null, m.languages)),
    actions,
    p?.stage === "error" ? h("p", { class: "error-text" }, p.message ?? "Download failed. Try again.") : null,
  );
}

export function mountModels(root: HTMLElement): () => void {
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
    grid,
    h("p", { class: "footnote" }, icon("lock", 14), "Audio never leaves this computer. Models download once, then work offline."),
  );
  const draw = () => clear(grid, ...store.models.map((m) => modelCard(m)));
  draw();
  return subscribe(["models", "progress", "state"], draw);
}
