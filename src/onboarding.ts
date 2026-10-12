import { api, type PermissionState } from "./api";
import { modelCard } from "./models";
import { downloadModel, hotkeyLabel, refreshState, selectModel, statusText, store, subscribe, updateSettings } from "./store";
import { clear, h, icon, keycap, logo } from "./ui";

type Step = "welcome" | "permissions" | "model" | "try";

export function mountOnboarding(root: HTMLElement, onDone: () => void): () => void {
  const platform = store.state.platform;
  const steps: Step[] = platform === "windows" ? ["welcome", "model", "try"] : ["welcome", "permissions", "model", "try"];
  let index = 0;
  let poll = 0;
  let unsub: (() => void) | null = null;
  let autoSelected = false;

  const card = h("div", { class: "onb-card" });
  const dots = h("ol", { class: "onb-dots", "aria-label": "Setup progress" });
  const bg = h("div", { class: "onb-bg", role: "status", "aria-live": "polite" });
  root.append(h("div", { class: "onb", "data-tauri-drag-region": true }, card, h("div", { class: "onb-foot" }, dots, bg)));

  // Start the model download at once so it is done by the time permissions are.
  const recommended = () => store.models.find((x) => x.recommended) ?? store.models[0];
  async function prepareModel() {
    const m = recommended();
    if (!m) return;
    if (!m.installed && !store.progress[m.id]) await downloadModel(m.id);
    const fresh = store.models.find((x) => x.id === m.id);
    if (fresh?.installed && !store.models.some((x) => x.active)) {
      autoSelected = true;
      await selectModel(m.id);
    }
  }
  function drawBg() {
    const m = recommended();
    if (!m) return clear(bg);
    const p = store.progress[m.id];
    const pct = p && p.total > 0 ? Math.round((p.downloaded / p.total) * 100) : null;
    let text = "";
    if (p && p.stage === "downloading") text = pct === null ? "Getting the voice model…" : `Getting the voice model · ${pct}%`;
    else if (p && (p.stage === "unpacking" || p.stage === "loading")) text = "Setting up the voice model…";
    else if (m.active || (m.installed && store.state.status === "ready")) text = "Voice model ready";
    else if (p?.stage === "error") text = "The voice model didn't download. You can retry in the next steps.";
    bg.className = `onb-bg${m.active ? " ok" : ""}`;
    clear(bg, text && steps[index] !== "model" ? h("span", null, m.active ? icon("check", 14) : h("span", { class: "spinner sm" }), text) : null);
  }
  const unsubBg = subscribe(["models", "progress", "state"], drawBg);

  function stopLive() {
    window.clearInterval(poll);
    poll = 0;
    unsub?.();
    unsub = null;
  }

  function go(i: number) {
    stopLive();
    index = i;
    draw();
    requestAnimationFrame(() => (card.querySelector("[data-autofocus]") as HTMLElement | null)?.focus());
  }
  const next = () => go(index + 1);

  function nav(primary: HTMLElement, opts: { back?: boolean } = { back: true }): HTMLElement {
    return h(
      "div",
      { class: "onb-nav" },
      opts.back && index > 0 ? h("button", { class: "btn btn-ghost", type: "button", onclick: () => go(index - 1) }, "Back") : h("span"),
      primary,
    );
  }

  function welcome(): HTMLElement[] {
    return [
      h("div", { class: "onb-mark" }, logo(72)),
      h("h1", { class: "onb-title display" }, "Silas"),
      h("p", { class: "onb-tagline" }, "Speak. Silas writes."),
      h(
        "p",
        { class: "onb-body" },
        "Hold a key, say what you mean, and let go. Silas types it into whatever app you're using. It runs entirely on this computer and learns the words you use.",
      ),
      nav(h("button", { class: "btn btn-primary btn-lg", type: "button", "data-autofocus": true, onclick: next }, "Get started", icon("arrow", 16)), { back: false }),
    ];
  }

  function permRow(title: string, why: string, state: PermissionState, kind: "microphone" | "accessibility" | "input"): HTMLElement {
    const granted = state === "granted";
    return h(
      "div",
      { class: `perm${granted ? " ok" : ""}` },
      h("span", { class: "perm-icon" }, icon(kind === "microphone" ? "mic" : "keyboard", 20)),
      h("div", { class: "perm-text" }, h("strong", null, title), h("p", null, why)),
      granted
        ? h("span", { class: "state-pill active" }, icon("check", 14), "Allowed")
        : h("button", { class: "btn btn-ghost btn-sm", type: "button", onclick: () => void api.openPermission(kind) }, "Open Settings"),
    );
  }

  function permissions(): HTMLElement[] {
    const p = store.state.permissions;
    if (!poll) {
      poll = window.setInterval(() => void refreshState(), 1500);
      unsub = subscribe(["state"], draw);
    }
    if (platform === "linux") {
      return [
        h("h1", { class: "onb-title" }, "Let Silas hear your hotkey"),
        h(
          "p",
          { class: "onb-body" },
          "On Linux, Silas reads the hotkey straight from your keyboard. The installer adds a rule that lets your user read input devices. If the hotkey doesn't respond, log out and back in once.",
        ),
        h("div", { class: "perm-list" }, permRow("Input access", "So Silas can see when you hold the hotkey and type for you.", p.accessibility, "input")),
        nav(h("button", { class: "btn btn-primary btn-lg", type: "button", "data-autofocus": true, onclick: next }, p.accessibility === "granted" ? "Continue" : "Continue anyway")),
      ];
    }
    const all = p.microphone === "granted" && p.accessibility === "granted";
    return [
      h("h1", { class: "onb-title" }, "Two quick permissions"),
      h("p", { class: "onb-body" }, "macOS asks you to allow these once. Turn on Silas in each pane, then come back here."),
      h(
        "div",
        { class: "perm-list" },
        permRow("Microphone", "So Silas can hear you. Audio stays on this computer.", p.microphone, "microphone"),
        permRow("Accessibility", "So Silas can notice your hotkey and type into other apps.", p.accessibility, "accessibility"),
      ),
      nav(
        h(
          "button",
          { class: `btn ${all ? "btn-primary" : "btn-ghost"} btn-lg`, type: "button", "data-autofocus": true, onclick: next },
          all ? "Continue" : "Skip for now",
        ),
      ),
    ];
  }

  function model(): HTMLElement[] {
    if (!unsub) unsub = subscribe(["models", "progress", "state"], draw);
    const m = store.models.find((x) => x.recommended) ?? store.models[0];
    const ready = !!m && m.installed;
    if (m && ready && !autoSelected && !store.models.some((x) => x.active)) {
      // Downloaded but not loaded yet: load it so the next step works.
      autoSelected = true;
      void selectModel(m.id);
    }
    return [
      h("h1", { class: "onb-title" }, ready ? "Your voice model is ready" : "Getting your voice model"),
      h("p", { class: "onb-body" }, "Silas turns speech into text with a model that lives on this computer. It downloads once, then works offline."),
      m ? h("div", { class: "onb-model" }, modelCard(m, { compact: true })) : h("p", null, "Loading models…"),
      h("p", { class: "onb-note" }, "You can switch models any time in Model."),
      nav(
        h(
          "button",
          { class: `btn ${ready ? "btn-primary" : "btn-ghost"} btn-lg`, type: "button", "data-autofocus": ready ? true : null, onclick: next },
          ready ? "Continue" : "Skip for now",
        ),
      ),
    ];
  }

  function tryIt(): HTMLElement[] {
    if (!unsub) unsub = subscribe(["state"], updateLive);
    const live = h("div", { class: "live", role: "status", "aria-live": "polite" });
    const area = h("textarea", {
      class: "try-area",
      rows: 4,
      placeholder: "Your words will appear here.",
      "aria-label": "Practice area",
      "data-autofocus": true,
    });
    function updateLive() {
      const s = store.state.status;
      live.className = `live live-${s}`;
      clear(live, h("span", { class: "dot" }), statusText[s]);
    }
    updateLive();
    return [
      h("h1", { class: "onb-title" }, "Try it"),
      h("p", { class: "onb-try-line" }, "Hold ", keycap(hotkeyLabel(), "lg"), " and say something"),
      h("p", { class: "onb-body center" }, "Let go when you're done. Silas types into whatever field has focus, like this one."),
      area,
      live,
      h("p", { class: "onb-note" }, store.settings.tapToToggle ? "Tip: tap the key once for hands-free. Tap again to stop." : ""),
      nav(
        h(
          "button",
          {
            class: "btn btn-primary btn-lg",
            type: "button",
            onclick: async () => {
              await updateSettings({ onboarded: true });
              onDone();
            },
          },
          "Start using Silas",
        ),
      ),
    ];
  }

  function draw() {
    const step = steps[index];
    // Redrawing the practice step would wipe what the user dictated.
    if (step === "try" && card.querySelector(".try-area")) return;
    card.setAttribute("data-step", step);
    clear(card, ...(step === "welcome" ? welcome() : step === "permissions" ? permissions() : step === "model" ? model() : tryIt()));
    drawBg();
    clear(
      dots,
      ...steps.map((s, i) =>
        h("li", { class: i === index ? "on" : i < index ? "done" : null, "aria-current": i === index ? "step" : null }, h("span", { class: "sr-only" }, `Step ${i + 1} of ${steps.length}: ${s}`)),
      ),
    );
  }

  draw();
  void prepareModel();
  requestAnimationFrame(() => (card.querySelector("[data-autofocus]") as HTMLElement | null)?.focus());
  return () => {
    stopLive();
    unsubBg();
  };
}
