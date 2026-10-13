import { api, type PermissionState } from "./api";
import { modelCard } from "./models";
import { downloadModel, hotkeyLabel, refreshState, selectModel, statusText, store, subscribe, updateSettings } from "./store";
import { clear, h, icon, keycap, logo } from "./ui";

type Step = "welcome" | "permissions" | "model" | "try";

/** Mac and Linux need the hotkey listener; without access Silas can't hear its key. */
export function needsAccess(): boolean {
  const s = store.state;
  // Windows needs nothing; on Linux a missing udev grant is shown as a Home banner rather
  // than reopening setup on every launch.
  if (s.platform !== "macos") return false;
  return s.permissions.accessibility !== "granted" || (s.platform === "macos" && s.permissions.microphone === "denied");
}

export function mountOnboarding(root: HTMLElement, onDone: () => void, opts: { start?: Step } = {}): () => void {
  const platform = store.state.platform;
  // The model downloads in the background from the first screen, so it needs no step of its own.
  const steps: Step[] = opts.start === "permissions" ? ["permissions"] : platform === "windows" ? ["welcome", "try"] : ["welcome", "permissions", "try"];
  const repair = opts.start === "permissions";
  let index = 0;
  let asked: Record<string, number> = {};
  let advancing = 0;
  let lastPermKey = "";
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
    window.clearTimeout(advancing);
    advancing = 0;
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

  function permRow(
    title: string,
    why: string,
    state: PermissionState,
    kind: "microphone" | "accessibility" | "input",
    enabled: boolean,
  ): HTMLElement {
    const granted = state === "granted";
    // On Linux there is nothing to click: the installer's udev rule grants keyboard access.
    const actionable = kind !== "input";
    // A denied permission isn't pending: show the button that opens its Settings pane.
    const waiting = actionable && !granted && state !== "denied" && asked[kind] !== undefined;
    const slow = waiting && Date.now() - asked[kind] > 9000;
    const hint =
      kind === "accessibility"
        ? "In the list that opened, turn on Silas. If macOS asks, enter your password. Silas comes back here by itself."
        : "Click Allow in the macOS prompt. If no prompt appears, turn on Silas in the Microphone list that opened.";
    const label = state === "denied" && kind === "microphone" ? "Open Settings" : "Allow";
    return h(
      "div",
      { class: `perm${granted ? " ok" : ""}${enabled || granted ? "" : " later"}` },
      h("span", { class: "perm-icon" }, icon(kind === "microphone" ? "mic" : "keyboard", 20)),
      h(
        "div",
        { class: "perm-text" },
        h("strong", null, title),
        h("p", null, waiting ? hint : why),
        slow && kind === "accessibility"
          ? h(
              "p",
              { class: "perm-fix" },
              "Already switched on? macOS sometimes keeps an old entry. ",
              h(
                "button",
                {
                  class: "linklike",
                  type: "button",
                  onclick: async () => {
                    asked[kind] = Date.now();
                    await api.resetAccessibility();
                    draw();
                  },
                },
                "Reset it and ask again",
              ),
            )
          : null,
      ),
      granted
        ? h("span", { class: "state-pill active" }, icon("check", 14), "Allowed")
        : !actionable
          ? h("span", { class: "state-pill" }, "Not yet")
          : waiting
          ? h("span", { class: "state-pill" }, h("span", { class: "spinner sm" }), "Waiting")
          : h(
              "button",
              {
                class: `btn ${enabled ? "btn-primary" : "btn-ghost"} btn-sm`,
                type: "button",
                disabled: enabled ? null : true,
                "data-autofocus": enabled ? true : null,
                onclick: async () => {
                  asked[kind] = Date.now();
                  lastPermKey = permKey();
                  draw();
                  await api.openPermission(kind);
                },
              },
              label,
            ),
    );
  }

  function permKey(): string {
    const p = store.state.permissions;
    const slow = Object.entries(asked).map(([k, t]) => `${k}:${Date.now() - t > 9000}`);
    return [p.microphone, p.accessibility, ...slow].join("|");
  }

  function permissions(): HTMLElement[] {
    const p = store.state.permissions;
    if (!poll) {
      // Redraw only when something visible changed, so clicks and focus survive the poll.
      const redraw = () => {
        const key = permKey();
        if (key === lastPermKey) return;
        lastPermKey = key;
        const hadFocus = card.contains(document.activeElement);
        draw();
        if (hadFocus) (card.querySelector("[data-autofocus]") as HTMLElement | null)?.focus();
      };
      poll = window.setInterval(() => {
        void refreshState();
        redraw(); // the "already switched on?" hint appears with time
      }, 1000);
      unsub = subscribe(["state"], redraw);
      lastPermKey = permKey();
    }
    const leave = () => (index + 1 < steps.length ? next() : finish());
    if (platform === "linux") {
      const ok = p.accessibility === "granted";
      if (ok && !advancing) advancing = window.setTimeout(leave, 700);
      return [
        h("h1", { class: "onb-title" }, ok ? "Silas can hear your hotkey" : "Let Silas hear your hotkey"),
        h(
          "p",
          { class: "onb-body" },
          ok
            ? "All set."
            : "On Linux, Silas reads the hotkey straight from your keyboard. The installer adds a rule for that. If this stays unchecked, log out and back in once.",
        ),
        h("div", { class: "perm-list" }, permRow("Keyboard access", "So Silas can see when you hold the hotkey.", p.accessibility, "input", true)),
        nav(h("button", { class: "btn btn-ghost btn-lg", type: "button", onclick: leave }, ok ? "Continue" : "Continue anyway")),
      ];
    }
    const micOk = p.microphone === "granted";
    const all = micOk && p.accessibility === "granted";
    if (all && !advancing) advancing = window.setTimeout(leave, 800);
    return [
      h("h1", { class: "onb-title" }, all ? "You're all set" : repair ? "Silas needs access again" : "Two quick permissions"),
      h(
        "p",
        { class: "onb-body" },
        all
          ? "Silas can hear you and type for you."
          : repair
            ? "macOS turned off a permission Silas needs, usually after an update. Allow it once more."
            : "Allow each one when asked. Silas moves on by itself when both are on.",
      ),
      h(
        "div",
        { class: "perm-list" },
        permRow("Microphone", "So Silas can hear you. Audio never leaves this computer.", p.microphone, "microphone", true),
        permRow(
          "Accessibility",
          "So Silas can notice your hotkey and type into other apps.",
          p.accessibility,
          "accessibility",
          micOk || p.microphone === "denied" || asked.microphone !== undefined,
        ),
      ),
      all
        ? h("div", { class: "onb-nav" }, h("span"), h("span", { class: "state-pill active" }, icon("check", 14), "Continuing…"))
        : nav(h("button", { class: "btn btn-ghost", type: "button", onclick: leave }, "Skip for now"), { back: !repair }),
    ];
  }

  async function finish() {
    await updateSettings({ onboarded: true });
    onDone();
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
    if (!unsub) unsub = subscribe(["state", "progress", "models"], updateLive);
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
      const m = recommended();
      const p = m ? store.progress[m.id] : undefined;
      const pct = p && p.total > 0 ? Math.round((p.downloaded / p.total) * 100) : null;
      const text =
        s === "needs_model" || (s === "loading" && p)
          ? pct === null
            ? "Getting the voice model…"
            : `Getting the voice model · ${pct}%. You can try it as soon as this finishes.`
          : statusText[s];
      clear(live, h("span", { class: "dot" }), text);
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
            onclick: () => void finish(),
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
  // Repair mode is only about permissions; never start a download the user didn't ask for.
  if (!repair) void prepareModel();
  requestAnimationFrame(() => (card.querySelector("[data-autofocus]") as HTMLElement | null)?.focus());
  return () => {
    window.clearTimeout(advancing);
    stopLive();
    unsubBg();
  };
}
