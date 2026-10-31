import "./styles.css";
import { api, errorText, inTauri, on } from "./api";
import { mountDictionary } from "./dictionary";
import { mountHistory } from "./history";
import { mountHome } from "./home";
import { mountModels } from "./models";
import { mountOnboarding, needsAccess } from "./onboarding";
import { mountSettings } from "./settings";
import {
  HISTORY_LIMIT,
  notify,
  refreshDict,
  refreshModels,
  refreshState,
  refreshStats,
  statusText,
  store,
  subscribe,
} from "./store";
import { clear, h, icon, logo, toast } from "./ui";

export type Route = "home" | "history" | "dictionary" | "model" | "settings";
export type Navigate = (r: Route) => void;

const routes: Array<{ id: Route; label: string; icon: string; mount: (root: HTMLElement, go: Navigate) => () => void }> = [
  { id: "home", label: "Home", icon: "home", mount: mountHome },
  { id: "history", label: "History", icon: "history", mount: mountHistory },
  { id: "dictionary", label: "Dictionary", icon: "book", mount: mountDictionary },
  { id: "model", label: "Model", icon: "model", mount: mountModels },
  { id: "settings", label: "Settings", icon: "settings", mount: mountSettings },
];

const app = document.getElementById("app")!;
let current: Route = "home";
let unmount: (() => void) | null = null;

async function boot() {
  try {
    const [state, settings, models, history, dict, stats, device, benchmark] = await Promise.all([
      api.getState(),
      api.getSettings(),
      api.listModels(),
      api.getHistory(HISTORY_LIMIT),
      api.getDictionary(),
      api.getStats(),
      api.getDevice(),
      api.getBenchmark(),
    ]);
    Object.assign(store, { state, settings, models, history, dict, stats, device, benchmark });
  } catch (e) {
    clear(app, h("div", { class: "fatal" }, logo(48), h("h1", null, "Silas couldn't start"), h("p", null, errorText(e))));
    return;
  }
  document.documentElement.dataset.platform = store.state.platform;
  if (!inTauri) document.documentElement.dataset.preview = "true";
  wireEvents();
  if (!store.settings.onboarded) onboarding();
  else if (needsAccess()) onboarding("permissions");
  else shell();
}

function wireEvents() {
  void on("status", ({ status, detail }) => {
    const was = store.state.status;
    store.state = { ...store.state, status, detail };
    notify("state");
    if (status === "ready" && was === "loading") void refreshModels();
  });
  void on("transcript", (item) => {
    store.history = [item, ...store.history.filter((x) => x.id !== item.id)].slice(0, HISTORY_LIMIT);
    notify("history");
    void refreshStats();
  });
  void on("learned", (res) => {
    for (const c of res.learned) toast(`Learned: ${c.from} → ${c.to}`, "learned", 4200);
    void refreshDict();
    void refreshStats();
  });
  void on("model-progress", (p) => {
    store.progress[p.id] = p;
    notify("progress");
    if (p.stage === "done" || p.stage === "error") {
      if (p.stage === "error") toast(p.message ?? "The model download failed.", "error", 6000);
      window.setTimeout(() => {
        if (store.progress[p.id]?.stage === p.stage) delete store.progress[p.id];
        notify("progress");
      }, p.stage === "done" ? 600 : 8000);
      void refreshModels();
      void refreshState();
    }
  });
  void on("benchmark-progress", (progress) => {
    store.bench = { running: true, progress, error: null };
    notify("benchmark");
  });
  void on("benchmark-done", (report) => {
    store.benchmark = report;
    store.bench = { running: false, progress: null, error: null };
    notify("benchmark");
    // The pick may have changed the recommended model or the one in use.
    void refreshModels();
    void refreshState();
    void api.getSettings().then((next) => {
      store.settings = next;
      notify("settings");
    });
  });
  void on("benchmark-error", ({ message }) => {
    store.bench = { running: false, progress: null, error: message };
    notify("benchmark");
  });
  void on("update-available", (info) => {
    store.update = info;
    store.updateCheck = "found";
    notify("update");
  });
  void on("update-progress", (p) => {
    store.updateProgress = p;
    notify("update");
  });
  // Pick up permission or model changes made while the window was hidden.
  window.addEventListener("focus", () => void refreshState());
}

function onboarding(start?: "permissions") {
  unmount?.();
  const root = h("div", { class: "onb-root" });
  clear(app, root);
  unmount = mountOnboarding(
    root,
    () => {
      unmount?.();
      unmount = null;
      shell();
    },
    { start },
  );
}

function shell() {
  const nav = h("nav", { class: "nav", "aria-label": "Sections" });
  const status = h("div", { class: "side-status", role: "status", "aria-live": "polite" });
  const view = h("div", { class: "view", tabindex: -1 });

  const sidebar = h(
    "aside",
    { class: "sidebar" },
    h("div", { class: "side-drag", "data-tauri-drag-region": true }),
    h("div", { class: "brand", "data-tauri-drag-region": true }, logo(26), h("span", { class: "wordmark" }, "Silas")),
    nav,
    status,
  );
  const main = h("main", { class: "main" }, h("div", { class: "topbar", "data-tauri-drag-region": true }), view);
  clear(app, h("div", { class: "app" }, sidebar, main));

  function drawNav() {
    const pending = store.dict.pending.length;
    clear(
      nav,
      ...routes.map((r) =>
        h(
          "button",
          {
            type: "button",
            class: `nav-item${r.id === current ? " active" : ""}`,
            "aria-current": r.id === current ? "page" : null,
            onclick: () => go(r.id),
          },
          icon(r.icon, 18),
          h("span", null, r.label),
          r.id === "dictionary" && pending ? h("span", { class: "nav-badge", title: `${pending} waiting` }, String(pending)) : null,
          r.id === "model" && store.state.status === "needs_model" ? h("span", { class: "nav-dot", title: "No model installed" }) : null,
        ),
      ),
    );
  }

  function drawStatus() {
    const s = store.state;
    status.className = `side-status live-${s.status}`;
    status.title = s.detail ?? "";
    clear(status, h("span", { class: "dot" }), h("span", null, statusText[s.status]));
  }

  function go(r: Route) {
    current = r;
    unmount?.();
    view.replaceChildren();
    view.scrollTop = 0;
    const page = h("div", { class: `page page-${r}` });
    view.append(page);
    unmount = routes.find((x) => x.id === r)!.mount(page, go);
    drawNav();
  }

  drawNav();
  drawStatus();
  subscribe(["dict", "state"], drawNav);
  subscribe(["state"], drawStatus);

  // Cmd/Ctrl+1..5 jumps between sections.
  window.addEventListener("keydown", (e) => {
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
    const i = Number(e.key) - 1;
    if (i >= 0 && i < routes.length) {
      e.preventDefault();
      go(routes[i].id);
    }
  });

  go(store.state.status === "needs_model" ? "model" : current);
}

void boot();
