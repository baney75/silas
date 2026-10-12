import { api, errorText, type Dictionary, type Source } from "./api";
import { setDict, store, subscribe } from "./store";
import { clear, fixChip, h, icon, num, timeAgo, toast } from "./ui";

type Tab = "words" | "corrections" | "learning";
let tab: Tab = "words";

const sourceLabel: Record<Source, string> = { learned: "learned", user: "you", import: "imported" };

async function run(p: Promise<Dictionary>, done?: string) {
  try {
    setDict(await p);
    if (done) toast(done);
  } catch (e) {
    toast(errorText(e), "error");
  }
}

export function mountDictionary(root: HTMLElement): () => void {
  const fileInput = h("input", { type: "file", accept: "application/json,.json", hidden: true });
  fileInput.addEventListener("change", async () => {
    const file = fileInput.files?.[0];
    fileInput.value = "";
    if (!file) return;
    const before = store.dict.terms.length + store.dict.corrections.length;
    try {
      const d = await api.importDictionary(await file.text());
      setDict(d);
      const added = d.terms.length + d.corrections.length - before;
      toast(added > 0 ? `Imported ${num(added)} entr${added === 1 ? "y" : "ies"}` : "Imported. Everything was already here.");
    } catch (e) {
      toast(`Couldn't import that file. ${errorText(e)}`, "error", 6000);
    }
  });

  const head = h(
    "div",
    { class: "page-head" },
    h(
      "div",
      null,
      h("h1", null, "Dictionary"),
      h("p", { class: "lede" }, "Fix a word right after Silas types it (backspace and retype) and Silas learns it. Fix it twice and it's automatic."),
    ),
    h(
      "div",
      { class: "head-actions" },
      fileInput,
      h("button", { class: "btn btn-ghost", type: "button", onclick: () => fileInput.click() }, icon("upload", 16), "Import"),
      h("button", { class: "btn btn-ghost", type: "button", onclick: () => void exportDict() }, icon("download", 16), "Export"),
    ),
  );
  const tabs = h("div", { class: "tabs", role: "tablist", "aria-label": "Dictionary sections" });
  const panel = h("div", { class: "tab-panel", role: "tabpanel" });
  root.append(head, tabs, panel);

  const tabDefs: Array<{ id: Tab; label: string; count: () => number }> = [
    { id: "words", label: "Words", count: () => store.dict.terms.length },
    { id: "corrections", label: "Corrections", count: () => store.dict.corrections.length },
    { id: "learning", label: "Learning", count: () => store.dict.pending.length },
  ];

  function drawTabs() {
    const buttons = tabDefs.map((t) =>
      h(
        "button",
        {
          type: "button",
          role: "tab",
          id: `tab-${t.id}`,
          "aria-selected": String(tab === t.id),
          tabindex: tab === t.id ? 0 : -1,
          class: t.id === "learning" && t.count() ? "has-news" : null,
          onclick: () => {
            tab = t.id;
            draw();
          },
        },
        t.label,
        h("span", { class: "count" }, String(t.count())),
      ),
    );
    buttons.forEach((b, i) =>
      b.addEventListener("keydown", (e) => {
        const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (!step) return;
        tab = tabDefs[(i + step + tabDefs.length) % tabDefs.length].id;
        draw();
        (tabs.querySelector('[aria-selected="true"]') as HTMLElement | null)?.focus();
      }),
    );
    clear(tabs, ...buttons);
    panel.setAttribute("aria-labelledby", `tab-${tab}`);
  }

  function words() {
    const input = h("input", {
      class: "input",
      type: "text",
      placeholder: "Add a word or name, then press Enter",
      "aria-label": "New word",
      autocomplete: "off",
      spellcheck: "false",
    });
    input.addEventListener("keydown", async (e) => {
      if (e.key !== "Enter") return;
      const text = input.value.trim();
      if (!text) return;
      if (store.dict.terms.some((t) => t.text.toLowerCase() === text.toLowerCase())) {
        toast(`${text} is already in your dictionary.`);
        return;
      }
      await run(api.addTerm(text));
      requestAnimationFrame(() => (panel.querySelector("input") as HTMLInputElement | null)?.focus());
    });
    const terms = store.dict.terms;
    return [
      h("p", { class: "panel-note" }, "Names, brands and jargon Silas should always spell your way."),
      h("div", { class: "add-row" }, input),
      terms.length
        ? h(
            "ul",
            { class: "term-list" },
            ...terms.map((t) =>
              h(
                "li",
                { class: `term term-${t.source}`, title: `${t.uses ? `Used ${t.uses}×. ` : ""}Added ${timeAgo(t.added)} by ${sourceLabel[t.source]}` },
                h("span", null, t.text),
                t.source === "learned" ? h("span", { class: "term-src" }, "learned") : null,
                h(
                  "button",
                  { class: "chip-x", type: "button", "aria-label": `Remove ${t.text}`, onclick: () => void run(api.removeTerm(t.text)) },
                  icon("x", 14),
                ),
              ),
            ),
          )
        : h("div", { class: "empty" }, h("p", null, "No words yet. Add the names you say often: people, places, products.")),
    ];
  }

  function corrections() {
    const from = h("input", { class: "input", type: "text", placeholder: "When Silas hears…", "aria-label": "Heard", autocomplete: "off", spellcheck: "false" });
    const to = h("input", { class: "input", type: "text", placeholder: "Write this", "aria-label": "Write", autocomplete: "off", spellcheck: "false" });
    const add = async () => {
      const f = from.value.trim();
      const t = to.value.trim();
      if (!f) return from.focus();
      if (!t) return to.focus();
      await run(api.addCorrection(f, t), `Added: ${f} → ${t}`);
      requestAnimationFrame(() => (panel.querySelector("input") as HTMLInputElement | null)?.focus());
    };
    from.addEventListener("keydown", (e) => e.key === "Enter" && (to.value.trim() ? void add() : to.focus()));
    to.addEventListener("keydown", (e) => e.key === "Enter" && void add());
    const list = store.dict.corrections;
    return [
      h("p", { class: "panel-note" }, "Silas swaps what it hears for what you mean, every time."),
      h(
        "div",
        { class: "add-row pair" },
        from,
        h("span", { class: "pair-arrow", "aria-hidden": "true" }, "→"),
        to,
        h("button", { class: "btn btn-primary", type: "button", onclick: () => void add() }, "Add"),
      ),
      list.length
        ? h(
            "ul",
            { class: "rows" },
            ...list.map((c) =>
              h(
                "li",
                { class: "row" },
                fixChip(c.from, c.to),
                h("span", { class: "row-meta" }, c.count ? `${num(c.count)}×` : "new"),
                h("span", { class: `badge badge-${c.source}` }, sourceLabel[c.source]),
                h(
                  "button",
                  { class: "icon-btn", type: "button", "aria-label": `Remove ${c.from}`, title: "Remove", onclick: () => void run(api.removeCorrection(c.from)) },
                  icon("x", 16),
                ),
              ),
            ),
          )
        : h("div", { class: "empty" }, h("p", null, "No corrections yet. Fix a word after Silas types it, or add one above.")),
    ];
  }

  function learning() {
    const list = store.dict.pending;
    return [
      h("p", { class: "panel-note" }, "Fixes Silas has seen once. Accept to trust them now, or wait for a second fix."),
      list.length
        ? h(
            "ul",
            { class: "rows" },
            ...list.map((p) =>
              h(
                "li",
                { class: "row" },
                fixChip(p.from, p.to, "pending"),
                h("span", { class: "row-meta" }, `Seen ${p.count === 1 ? "once" : `${p.count}×`} · ${timeAgo(p.lastSeen)}`),
                h(
                  "div",
                  { class: "row-actions" },
                  h("button", { class: "btn btn-ghost btn-sm", type: "button", onclick: () => void run(api.dismissPending(p.from)) }, "Dismiss"),
                  h(
                    "button",
                    { class: "btn btn-primary btn-sm", type: "button", onclick: () => void run(api.acceptPending(p.from), `Learned: ${p.from} → ${p.to}`) },
                    icon("check", 16),
                    "Accept",
                  ),
                ),
              ),
            ),
          )
        : h("div", { class: "empty" }, h("p", null, "Nothing waiting. When you fix a word right after Silas types it, it shows up here.")),
    ];
  }

  function draw() {
    drawTabs();
    clear(panel, ...(tab === "words" ? words() : tab === "corrections" ? corrections() : learning()));
  }

  draw();
  return subscribe(["dict"], () => {
    const focused = document.activeElement;
    const hadFocus = focused instanceof HTMLInputElement && panel.contains(focused);
    draw();
    if (hadFocus) (panel.querySelector("input") as HTMLInputElement | null)?.focus();
  });
}

async function exportDict() {
  try {
    const path = await api.saveDictionary();
    toast(`Saved to ${path}`);
  } catch (e) {
    toast(`Couldn't export. ${errorText(e)}`, "error");
  }
}
