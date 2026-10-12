import { api, errorText, type Settings, type TypingSpeed } from "./api";
import { checkForUpdate, hotkeyLabel, installUpdate, store, subscribe, updateSettings } from "./store";
import type { UpdateChannel } from "./api";
import { clear, h, keycap, logo, segmented, switchEl, toast } from "./ui";

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

function toggle(key: BoolKey, label: string, desc: string | null): HTMLElement {
  return row(label, desc, switchEl(store.settings[key], label, (v) => change({ [key]: v } as Partial<Settings>)));
}

let capturing = false;

export function mountSettings(root: HTMLElement): () => void {
  const body = h("div", { class: "settings" });
  let drawnLabel = "";
  root.append(h("div", { class: "page-head" }, h("div", null, h("h1", null, "Settings"), h("p", { class: "lede" }, "Changes save as you make them."))), body);

  function hotkeyControl(): HTMLElement {
    const btn = h(
      "button",
      {
        class: "btn btn-ghost btn-sm",
        type: "button",
        disabled: capturing,
        onclick: async () => {
          capturing = true;
          draw();
          try {
            const key = await api.captureHotkey();
            capturing = false;
            await updateSettings({ hotkey: key });
            toast(`Hotkey set to ${hotkeyLabel()}`);
          } catch (e) {
            capturing = false;
            toast(errorText(e), "error");
          }
          draw();
        },
      },
      capturing ? "Waiting…" : "Change",
    );
    return h(
      "div",
      { class: "hotkey" },
      capturing ? h("span", { class: "keycap keycap-md capturing", "aria-live": "polite" }, "Press a key…") : keycap(hotkeyLabel(), "md"),
      btn,
    );
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
    drawnLabel = hotkeyLabel();
    const isMac = store.state.platform === "macos";
    clear(
      body,
      group(
        "Dictation",
        row("Hotkey", "Hold it to talk. Pick a key you don't use for anything else.", hotkeyControl()),
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
        toggle("showOverlay", "Show the recording pill", "A small floating pill while Silas listens."),
      ),
      group("System", toggle("launchAtLogin", isMac ? "Open at login" : "Start with your computer", isMac ? "Silas waits quietly in the menu bar." : "Silas waits quietly in the system tray.")),
      group(
        "Updates",
        isMac
          ? row(
              "Channel",
              s.updateChannel === "stable" ? "Tested releases. Recommended." : "The newest build every night. Expect rough edges.",
              segmented<UpdateChannel>(
                [
                  { value: "stable", label: "Stable" },
                  { value: "nightly", label: "Nightly" },
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
            )
          : row("Channel", "Silas for Windows and Linux ships as nightly builds for now.", h("span", { class: "badge" }, "Nightly builds")),
        toggle("autoUpdate", "Update automatically", "Silas downloads updates quietly and installs them when you restart it."),
        updateRow(),
      ),
      about(),
    );
  }

  draw();
  if (!store.microphones.length) {
    api
      .listMicrophones()
      .then((m) => {
        store.microphones = m;
        if (!capturing) draw();
      })
      .catch(() => undefined);
  }
  const unsubUpdate = subscribe(["update"], () => {
    const old = body.querySelector(".update-row");
    if (old) old.replaceWith(updateRow());
  });
  const unsub = subscribe(["settings", "state"], () => {
    if (capturing) return;
    if (JSON.stringify(shown) === JSON.stringify(store.settings) && drawnLabel === hotkeyLabel()) return;
    // Keep focus where it was so keyboard users don't lose their place.
    const active = document.activeElement as HTMLElement | null;
    const rows = [...body.querySelectorAll("button, select")];
    const index = active ? rows.indexOf(active) : -1;
    draw();
    if (index >= 0) ([...body.querySelectorAll("button, select")][index] as HTMLElement | undefined)?.focus();
  });
  return () => {
    unsub();
    unsubUpdate();
  };
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
