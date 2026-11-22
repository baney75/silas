import { api, errorText, type AppStyle, type Settings, type TypingSpeed } from "./api";
import { assignStyle, assignments, STYLES, styleFor, styleHelp, unassign } from "./appstyles";
import { checkForUpdate, hotkeyLabel, installUpdate, store, subscribe, updateSettings } from "./store";
import type { UpdateChannel } from "./api";
import { forgetPairingCode, syncGroup } from "./sync";
import { clear, h, icon, keycap, logo, segmented, switchEl, toast } from "./ui";

export const languages: Array<[string, string]> = [
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["nl", "Dutch"],
  ["pl", "Polish"],
];

type BoolKey = { [K in keyof Settings]: Settings[K] extends boolean ? K : never }[keyof Settings];

function row(label: string, desc: string | null, control: HTMLElement, id?: string): HTMLElement {
  return h(
    "div",
    { class: "set-row" },
    h("div", { class: "set-text" }, h("label", { class: "set-label", for: id ?? null }, label), desc ? h("p", { class: "set-desc" }, desc) : null),
    h("div", { class: "set-control" }, control),
  );
}

// What the controls on screen show. A store update that matches it needs no redraw,
// which keeps switch animations and focus intact.
let shown: Settings | null = null;

function change(patch: Partial<Settings>) {
  shown = { ...(shown ?? store.settings), ...patch };
  void updateSettings(patch);
}

// A style change redraws the App styles rows (the description, the list of assigned apps), so it
// skips `change`, which marks the page as already showing it.
function restyle(appStyles: Settings["appStyles"]) {
  void updateSettings({ appStyles });
}

function toggle(key: BoolKey, label: string, desc: string | null): HTMLElement {
  return row(label, desc, switchEl(store.settings[key], label, (v) => change({ [key]: v } as Partial<Settings>)));
}

// Which key is being picked: the dictation key or the one that types the last dictation again.
type KeySetting = "hotkey" | "retypeHotkey";
let capturing: KeySetting | null = null;

// An app id typed into "Add an app" but not added yet. Kept across redraws.
let appDraft = "";
let appDraftStyle: AppStyle = "terminal";

/** What the page shows besides the settings themselves; a change to it redraws. */
function drawnKey(): string {
  return JSON.stringify([hotkeyLabel(), store.state.retypeHotkeyLabel, store.state.app]);
}

export function mountSettings(root: HTMLElement): () => void {
  const body = h("div", { class: "settings" });
  let drawnLabel = "";
  root.append(h("div", { class: "page-head" }, h("div", null, h("h1", null, "Settings"), h("p", { class: "lede" }, "Changes save as you make them."))), body);

  /** Pick a key by pressing it. One capture call serves both keys. */
  async function pickKey(which: KeySetting) {
    capturing = which;
    draw();
    try {
      const key = await api.captureHotkey();
      capturing = null;
      await updateSettings({ [which]: key });
      if (which === "hotkey") toast(`Hotkey set to ${hotkeyLabel()}`);
      else if (store.settings.retypeHotkey === key) toast(`${store.state.retypeHotkeyLabel ?? key} types your last dictation again`);
    } catch (e) {
      capturing = null;
      // Escape ends the pick quietly; anything else says why.
      if (errorText(e) !== "cancelled") toast(errorText(e), "error");
    }
    draw();
  }

  function keyControl(which: KeySetting): HTMLElement {
    const label = which === "hotkey" ? hotkeyLabel() : store.state.retypeHotkeyLabel;
    const waiting = capturing === which;
    const btn = h(
      "button",
      { class: "btn btn-ghost btn-sm", type: "button", disabled: capturing !== null, onclick: () => void pickKey(which) },
      waiting ? "Waiting…" : label ? "Change" : "Set",
    );
    const clearBtn =
      which === "retypeHotkey" && label && !waiting
        ? h("button", { class: "btn btn-ghost btn-sm", type: "button", disabled: capturing !== null, onclick: () => change({ retypeHotkey: null }) }, "Clear")
        : null;
    return h(
      "div",
      { class: "hotkey" },
      waiting
        ? h("span", { class: "keycap keycap-md capturing", "aria-live": "polite" }, "Press a key…")
        : label
          ? keycap(label, "md")
          : h("span", { class: "set-desc" }, "Not set"),
      btn,
      clearBtn,
    );
  }

  function styleSelect(value: AppStyle, label: string, onChange: (v: AppStyle) => void): HTMLSelectElement {
    const sel = h("select", { class: "select select-sm", "aria-label": label }) as HTMLSelectElement;
    for (const st of STYLES) {
      const o = h("option", { value: st.value }, st.label);
      o.selected = st.value === value;
      sel.append(o);
    }
    sel.addEventListener("change", () => onChange(sel.value as AppStyle));
    return sel;
  }

  function appStyles(): HTMLElement[] {
    const s = store.settings;
    const app = store.state.app;
    const rows: HTMLElement[] = [];
    // Silas's own style for `id`, known only for the app it last saw.
    const builtinFor = (id: string) => (app && app.id.toLowerCase() === id.trim().toLowerCase() ? app.builtin : undefined);
    if (app) {
      const style = styleFor(s.appStyles, app);
      const named = app.name !== app.id ? `${app.name} (${app.id})` : app.id;
      rows.push(
        row(
          "Last app",
          `${named}. ${styleHelp(style)}`,
          segmented<AppStyle>(
            STYLES.map(({ value, label }) => ({ value, label })),
            style,
            `Style for ${app.name}`,
            (v) => restyle(assignStyle(s.appStyles, app.id, v, app.builtin)),
          ),
        ),
      );
    } else {
      rows.push(row("Last app", "Dictate into an app and it shows up here with the style Silas uses for it.", h("span")));
    }
    for (const [id, style] of assignments(s.appStyles)) {
      const name = app && app.id.toLowerCase() === id.toLowerCase() ? app.name : id;
      rows.push(
        row(
          name,
          `You chose ${STYLES.find((x) => x.value === style)?.label ?? style}.`,
          h(
            "div",
            { class: "hotkey" },
            styleSelect(style, `Style for ${name}`, (v) => restyle(assignStyle(s.appStyles, id, v, builtinFor(id)))),
            h(
              "button",
              { class: "icon-btn", type: "button", "aria-label": `Forget the style for ${name}`, title: "Use Silas's own style", onclick: () => restyle(unassign(s.appStyles, id)) },
              icon("x", 16),
            ),
          ),
        ),
      );
    }
    const input = h("input", {
      class: "input app-id",
      type: "text",
      placeholder: "App id",
      "aria-label": "App id",
      autocomplete: "off",
      spellcheck: "false",
      value: appDraft,
    }) as HTMLInputElement;
    const add = () => {
      const id = input.value.trim();
      if (!id) return input.focus();
      appDraft = "";
      restyle(assignStyle(s.appStyles, id, appDraftStyle, builtinFor(id)));
    };
    input.addEventListener("input", () => (appDraft = input.value));
    input.addEventListener("keydown", (e) => e.key === "Enter" && add());
    rows.push(
      row(
        "Add an app",
        "Its bundle id on a Mac (com.example.App), its program on Windows (app.exe), or its window class on Linux.",
        h(
          "div",
          { class: "add-row" },
          input,
          styleSelect(appDraftStyle, "Style for the new app", (v) => (appDraftStyle = v)),
          h("button", { class: "btn btn-ghost btn-sm", type: "button", onclick: add }, "Add"),
        ),
      ),
    );
    return rows;
  }

  function micSelect(): HTMLElement {
    const sel = h("select", { class: "select", id: "set-mic" });
    const opts: Array<[string, string]> = [["", "System default"], ...store.microphones.map((m): [string, string] => [m, m])];
    const current = store.settings.microphone;
    if (current && !store.microphones.includes(current)) opts.push([current, `${current} (not connected)`]);
    for (const [value, label] of opts) {
      const o = h("option", { value }, label);
      o.selected = (current ?? "") === value;
      sel.append(o);
    }
    sel.addEventListener("change", () => change({ microphone: sel.value || null }));
    return sel;
  }

  function langSelect(): HTMLElement {
    const sel = h("select", { class: "select", id: "set-lang" });
    for (const [code, name] of languages) {
      const o = h("option", { value: code }, name);
      o.selected = store.settings.language === code;
      sel.append(o);
    }
    sel.addEventListener("change", () => change({ language: sel.value }));
    return sel;
  }

  function group(title: string, ...rows: HTMLElement[]): HTMLElement {
    return h("section", { class: "set-group" }, h("h2", null, title), h("div", { class: "set-card" }, ...rows));
  }

  function draw() {
    const s = store.settings;
    shown = { ...s };
    drawnLabel = drawnKey();
    const isMac = store.state.platform === "macos";
    clear(
      body,
      group(
        "Dictation",
        row("Hotkey", "Hold it to talk. Pick a key you don't use for anything else.", keyControl("hotkey")),
        row("Type it again", "Press it to type your last dictation again where you are. Also in the tray menu.", keyControl("retypeHotkey")),
        toggle("tapToToggle", "Tap for hands-free", "A quick tap starts listening. Tap again to stop."),
        row(
          "Typing speed",
          "Steady is safest in slow apps. Instant suits fast editors.",
          segmented<TypingSpeed>(
            [
              { value: "steady", label: "Steady" },
              { value: "fast", label: "Fast" },
              { value: "instant", label: "Instant" },
            ],
            s.typingSpeed,
            "Typing speed",
            (v) => change({ typingSpeed: v }),
          ),
        ),
        toggle("newlineAsShiftEnter", "Shift+Enter for new lines", "So chat boxes don't send your message early."),
        toggle("trailingSpace", "Add a space after", "Ready for the next sentence."),
      ),
      group("App styles", ...appStyles()),
      group(
        "Words",
        toggle("removeFillers", "Remove filler words", `Drops ${store.dict.fillers.slice(0, 3).join(", ") || "um, uh"} and the like.`),
        toggle("learnFromEdits", "Learn from my edits", "When you fix a word right after Silas types it, Silas remembers."),
      ),
      group(
        "Audio",
        row("Microphone", null, micSelect(), "set-mic"),
        row("Language", "Parakeet detects the language on its own. Cohere uses this one.", langSelect(), "set-lang"),
        toggle("sounds", "Sounds", "A soft click when Silas starts and stops listening."),
        toggle("pauseMedia", "Pause media while dictating", "Music and videos that were playing pause while you speak, then play again."),
        toggle("showOverlay", "Show the recording pill", "A small floating pill while Silas listens."),
      ),
      syncSection(),
      group("System", toggle("launchAtLogin", isMac ? "Open at login" : "Start with your computer", isMac ? "Silas waits quietly in the menu bar." : "Silas waits quietly in the system tray.")),
      group(
        "Updates",
        row(
          "Channel",
          channelNote(s.updateChannel, isMac),
          segmented<UpdateChannel>(
            [
              { value: "stable", label: "Stable" },
              { value: "nightly", label: "Nightly" },
              { value: "dev", label: "Dev" },
            ],
            s.updateChannel,
            "Update channel",
            (v) => {
              change({ updateChannel: v });
              store.update = null;
              store.updateCheck = "idle";
              draw();
            },
          ),
        ),
        toggle("autoUpdate", "Update automatically", "Silas downloads updates quietly and installs them when you restart it."),
        updateRow(),
      ),
      about(),
    );
  }

  function syncSection(): HTMLElement {
    return syncGroup(() => body.querySelector(".sync-group")?.replaceWith(syncSection()));
  }

  draw();
  // A sync event redraws only the Sync group, and not while someone is typing in it.
  const unsubSync = subscribe(["sync"], () => {
    const old = body.querySelector(".sync-group");
    if (old && !old.contains(document.activeElement)) old.replaceWith(syncSection());
  });
  // The list is read at launch and on focus (`refreshMicrophones`); a newer one replaces only this control.
  const unsubMics = subscribe(["microphones"], () => {
    const old = body.querySelector("#set-mic");
    if (old && document.activeElement !== old) old.replaceWith(micSelect());
  });
  const unsubUpdate = subscribe(["update"], () => {
    const old = body.querySelector(".update-row");
    if (old) old.replaceWith(updateRow());
  });
  const unsub = subscribe(["settings", "state"], () => {
    if (capturing) return;
    if (JSON.stringify(shown) === JSON.stringify(store.settings) && drawnLabel === drawnKey()) return;
    // Keep focus where it was so keyboard users don't lose their place.
    const active = document.activeElement as HTMLElement | null;
    const rows = [...body.querySelectorAll("button, select, input")];
    const index = active ? rows.indexOf(active) : -1;
    draw();
    if (index >= 0) ([...body.querySelectorAll("button, select, input")][index] as HTMLElement | undefined)?.focus();
  });
  return () => {
    unsub();
    unsubUpdate();
    unsubMics();
    unsubSync();
    forgetPairingCode();
  };
}

function channelNote(channel: UpdateChannel, isMac: boolean): string {
  if (channel === "dev") return "A build of every change as it lands, for testing. Expect breakage.";
  if (channel === "nightly") return "The newest build every night. Expect rough edges.";
  // updates.rs follows nightly here until a stable release carries Windows and Linux.
  return isMac ? "Tested releases. Recommended." : "Tested releases. Recommended. Until the first one for your system ships, you get nightly builds.";
}

function updateRow(): HTMLElement {
  const u = store.update;
  const p = store.updateProgress;
  const version = store.state.version;
  let line: HTMLElement;
  let action: HTMLElement;
  if (p) {
    const pct = p.total ? Math.round((p.downloaded / p.total) * 100) : null;
    line = h("p", { class: "set-desc" }, pct === null ? "Downloading…" : `Downloading… ${pct}%. Silas restarts when it's done.`);
    action = h("span", { class: "spinner", "aria-hidden": "true" });
  } else if (u) {
    line = h(
      "div",
      { class: "update-found" },
      h("p", { class: "set-desc strong" }, `Silas ${u.version} is ready`, h("span", { class: "muted" }, ` · you have ${u.currentVersion}`)),
      u.notes ? h("p", { class: "set-desc" }, u.notes) : null,
    );
    action = h("button", { class: "btn btn-primary btn-sm", type: "button", onclick: () => void installUpdate() }, "Install and restart");
  } else {
    const checking = store.updateCheck === "checking";
    line = h(
      "p",
      { class: "set-desc" },
      store.updateCheck === "none" ? `You're up to date · ${version}` : store.updateCheck === "error" ? "The check didn't go through. Try again in a moment." : `Version ${version}`,
    );
    action = h(
      "button",
      { class: "btn btn-ghost btn-sm", type: "button", disabled: checking, onclick: () => void checkForUpdate() },
      checking ? "Checking…" : "Check for updates",
    );
  }
  return h(
    "div",
    { class: "set-row update-row" },
    h("div", { class: "set-text" }, h("span", { class: "set-label" }, "Version"), line),
    h("div", { class: "set-control" }, action),
  );
}

function about(): HTMLElement {
  return h(
    "section",
    { class: "about" },
    logo(40),
    h(
      "div",
      null,
      h("div", { class: "about-name" }, "Silas ", h("span", { class: "about-version" }, `Version ${store.state.version}`)),
      h("p", { class: "about-line" }, "Speak. Silas writes."),
      h("p", { class: "about-private" }, "Private: audio never leaves this computer."),
      h(
        "blockquote",
        { class: "scripture" },
        "“By Silvanus, a faithful brother… I have written briefly.”",
        h("cite", null, "1 Peter 5:12"),
      ),
    ),
  );
}
