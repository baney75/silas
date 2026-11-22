// Settings → Sync (docs/sync.md section 6). Draws from `store.sync`, which is read at launch and
// kept current by `sync` events, so opening Settings asks the core for nothing. The pairing code
// is fetched only when the person asks to pair, and forgotten when the section is redrawn away.

import { encode } from "uqr";
import { api, errorText, type PairingCode, type SyncStatus } from "./api";
import { setSync, store } from "./store";
import { h, icon, switchEl, timeAgo, toast } from "./ui";

const platformName: Record<string, string> = { macos: "Mac", windows: "Windows", linux: "Linux", ios: "iPhone or iPad" };

/** The code on screen, if the person asked for it. */
let shownCode: PairingCode | null = null;
/** Which destructive button is waiting for its second click. */
let confirming: "leave" | "newkey" | null = null;
let confirmTimer = 0;

/** "Synced with Donovan's iPhone just now (3 new words)". */
export function syncLine(s: SyncStatus): string {
  if (!s.enabled) return s.paired ? "Off. Your other devices keep their copy." : "Off. Your dictionary stays on this computer.";
  const last = s.last;
  if (!last) return s.devices.length ? `On. Last synced with ${s.devices[0].name} ${timeAgo(iso(s.devices[0].lastSynced))}.` : "On. No other devices yet.";
  const parts: string[] = [];
  if (last.added) parts.push(`${last.added} new word${last.added === 1 ? "" : "s"}`);
  if (last.changed) parts.push(`${last.changed} changed`);
  if (last.removed) parts.push(`${last.removed} removed`);
  return `Synced with ${last.with} ${timeAgo(iso(last.at))}${parts.length ? ` (${parts.join(", ")})` : ""}`;
}

const iso = (ms: number) => new Date(ms).toISOString();

/** A QR code as an SVG of `currentColor` squares; the theme colours it. */
export function qrSvg(text: string): SVGSVGElement {
  const qr = encode(text, { ecc: "M", border: 2 });
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", `0 0 ${qr.size} ${qr.size}`);
  svg.setAttribute("class", "qr");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "Pairing QR code");
  svg.setAttribute("shape-rendering", "crispEdges");
  let d = "";
  qr.data.forEach((row, y) =>
    row.forEach((on, x) => {
      if (on) d += `M${x} ${y}h1v1h-1z`;
    }),
  );
  const path = document.createElementNS(ns, "path");
  path.setAttribute("d", d);
  path.setAttribute("fill", "currentColor");
  svg.append(path);
  return svg;
}

async function act(p: Promise<SyncStatus>, done?: string): Promise<boolean> {
  try {
    setSync(await p);
    if (done) toast(done);
    return true;
  } catch (e) {
    toast(errorText(e), "error", 6000);
    return false;
  }
}

function row(label: string, desc: string | Node | null, control: HTMLElement | null, extra?: HTMLElement | null): HTMLElement {
  return h(
    "div",
    { class: "set-row sync-row" },
    h("div", { class: "set-text" }, h("span", { class: "set-label" }, label), desc ? h("p", { class: "set-desc" }, desc) : null, extra ?? null),
    control ? h("div", { class: "set-control" }, control) : null,
  );
}

/** A text field with a button; Enter submits too. */
function inputWith(placeholder: string, label: string, button: string, submit: (v: string) => Promise<boolean>, value = ""): HTMLElement {
  const input = h("input", { class: "input", type: "text", placeholder, "aria-label": label, autocomplete: "off", spellcheck: "false", value });
  const go = async () => {
    const v = input.value.trim();
    if (!v && !value) return;
    if (await submit(v)) input.value = "";
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") void go();
  });
  return h("div", { class: "sync-field" }, input, h("button", { class: "btn btn-ghost btn-sm", type: "button", onclick: () => void go() }, button));
}

function twoStep(which: "leave" | "newkey", label: string, again: string, run: () => void, redraw: () => void): HTMLElement {
  const waiting = confirming === which;
  return h(
    "button",
    {
      class: `btn btn-sm ${waiting ? "btn-danger" : "btn-ghost"}`,
      type: "button",
      onclick: () => {
        window.clearTimeout(confirmTimer);
        if (waiting) {
          confirming = null;
          run();
        } else {
          confirming = which;
          confirmTimer = window.setTimeout(() => {
            confirming = null;
            redraw();
          }, 4000);
        }
        redraw();
      },
    },
    waiting ? again : label,
  );
}

/** The Sync group for Settings. `redraw` rebuilds just this group. */
export function syncGroup(redraw: () => void): HTMLElement {
  const s = store.sync;
  const card = h("div", { class: "set-card" });
  const group = h("section", { class: "set-group sync-group" }, h("h2", null, "Sync"), card);
  if (!s) {
    card.append(row("Sync", "Sync isn't available in this build.", null));
    return group;
  }

  card.append(
    row(
      "Sync my dictionary",
      h("span", { class: "sync-line", "aria-live": "polite" }, syncLine(s)),
      switchEl(s.enabled, "Sync my dictionary", (on) => {
        shownCode = null;
        void act(api.setSyncEnabled(on)).then(redraw);
      }),
    ),
  );
  card.append(
    row(
      "What syncs",
      "Words, corrections, snippets and filler words, encrypted with a key only your devices hold. History, audio and settings stay here.",
      null,
    ),
  );

  // Pair: the code and QR, only on request.
  const pairButton = h(
    "button",
    {
      class: "btn btn-ghost btn-sm",
      type: "button",
      onclick: async () => {
        if (shownCode) {
          shownCode = null;
          return redraw();
        }
        try {
          shownCode = await api.syncPairingCode();
          setSync(await api.getSync());
        } catch (e) {
          toast(errorText(e), "error", 6000);
        }
        redraw();
      },
    },
    shownCode ? "Hide code" : "Pair a device",
  );
  card.append(
    row(
      "Pair a device",
      shownCode ? "On the other device, scan this or type the code into Join with a code." : "Shows a code for your other device to scan or type.",
      pairButton,
      shownCode
        ? h(
            "div",
            { class: "pair-panel" },
            h("div", { class: "qr-box" }, qrSvg(shownCode.url)),
            h(
              "div",
              { class: "pair-text" },
              h("code", { class: "pair-code", "aria-label": "Pairing code" }, shownCode.code),
              h(
                "div",
                { class: "pair-actions" },
                h(
                  "button",
                  {
                    class: "btn btn-ghost btn-sm",
                    type: "button",
                    onclick: async () => {
                      try {
                        await navigator.clipboard.writeText(shownCode?.code ?? "");
                        toast("Code copied");
                      } catch {
                        toast("Couldn't copy. Select the code and copy it.", "error");
                      }
                    },
                  },
                  icon("copy", 14),
                  "Copy",
                ),
                twoStep(
                  "newkey",
                  "New sync key",
                  "Every device must pair again",
                  () => {
                    shownCode = null;
                    void act(api.newSyncKey(), "New key made. Pair your other devices again.").then(redraw);
                  },
                  redraw,
                ),
              ),
              h("p", { class: "set-desc" }, "Anyone with this code can read and change your dictionary. Share it only with your own devices."),
            ),
          )
        : null,
    ),
  );

  card.append(
    row(
      "Join with a code",
      "Paste the code from your other device. This device joins its group.",
      null,
      inputWith("AAAA-BBBB-…", "Pairing code", "Join", (code) => act(api.joinSync(code), "Joined. Syncing now.")),
    ),
  );

  if (s.paired || s.enabled) {
    card.append(
      row(
        "This device",
        `Other devices see it by this name${s.port ? ` · port ${s.port}` : ""}.`,
        null,
        inputWith(s.name, "Device name", "Rename", (name) => act(api.setSyncName(name), "Renamed")),
      ),
    );

    card.append(
      row(
        "Devices",
        s.devices.length ? null : "None yet. Devices on the same network find each other.",
        h("button", { class: "btn btn-ghost btn-sm", type: "button", disabled: !s.enabled, onclick: () => void api.syncNow().then(() => toast("Syncing")) }, "Sync now"),
        s.devices.length
          ? h(
              "ul",
              { class: "sync-list" },
              ...s.devices.map((d) =>
                h(
                  "li",
                  null,
                  h("span", { class: "sync-name" }, d.name || "Unnamed"),
                  h("span", { class: "muted" }, ` · ${platformName[d.platform] ?? d.platform} · ${timeAgo(iso(d.lastSynced))}${d.via === "folder" ? " · folder" : ""}`),
                ),
              ),
            )
          : null,
      ),
    );

    card.append(
      row(
        "Peers by address",
        "For networks where devices can't see each other, like a tailnet. Type a name or address, with :port if it isn't 47823.",
        null,
        h(
          "div",
          null,
          s.peers.length
            ? h(
                "ul",
                { class: "sync-list" },
                ...s.peers.map((p) =>
                  h(
                    "li",
                    null,
                    h("span", { class: "sync-name" }, p),
                    h("button", { class: "chip-x", type: "button", "aria-label": `Remove ${p}`, onclick: () => void act(api.removeSyncPeer(p)) }, icon("x", 14)),
                  ),
                ),
              )
            : null,
          inputWith("rectory1:47823", "Peer address", "Add", (addr) => act(api.addSyncPeer(addr))),
        ),
      ),
    );

    card.append(
      row(
        "Sync folder",
        s.folder ? `Silas keeps an encrypted copy in ${s.folder}.` : "For devices that are never on the same network: a folder you already sync (iCloud Drive, Syncthing, Dropbox).",
        s.folder ? h("button", { class: "btn btn-ghost btn-sm", type: "button", onclick: () => void act(api.setSyncFolder(null), "Sync folder removed") }, "Stop using") : null,
        s.folder ? null : inputWith("~/Sync", "Sync folder path", "Use", (f) => act(api.setSyncFolder(f), "Sync folder set")),
      ),
    );

    card.append(
      row(
        "Leave sync",
        "Deletes the sync key from this device. Your dictionary stays as it is.",
        twoStep(
          "leave",
          "Leave sync",
          "Click again to leave",
          () => {
            shownCode = null;
            void act(api.leaveSync(), "Left sync").then(redraw);
          },
          redraw,
        ),
      ),
    );
  }
  return group;
}

/** Drop the code from memory when Settings closes. */
export function forgetPairingCode() {
  shownCode = null;
  confirming = null;
  window.clearTimeout(confirmTimer);
}
