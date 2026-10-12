import { api, errorText, type HistoryItem, type LearnResult } from "./api";
import { hotkeyLabel, notify, refreshDict, refreshStats, store, subscribe } from "./store";
import { append, clear, fixChip, h, icon, keycap, seconds, timeAgo, toast } from "./ui";

// What Silas learned from each teach, kept so it survives re-renders.
const lessons = new Map<string, LearnResult>();

export function mountHistory(root: HTMLElement): () => void {
  let editing: string | null = null;
  let confirmClear = false;

  const head = h("div", { class: "page-head" });
  const list = h("ul", { class: "entries", "aria-label": "Dictation history" });
  root.append(head, list);

  function drawHead() {
    const n = store.history.length;
    clear(
      head,
      h("div", null, h("h1", null, "History"), h("p", { class: "lede" }, n ? `${n} dictation${n === 1 ? "" : "s"}, newest first. Teach a fix and Silas remembers it.` : "Your dictations show up here.")),
      n
        ? h(
            "button",
            {
              class: `btn ${confirmClear ? "btn-danger" : "btn-ghost"}`,
              type: "button",
              onclick: async () => {
                if (!confirmClear) {
                  confirmClear = true;
                  drawHead();
                  window.setTimeout(() => {
                    confirmClear = false;
                    drawHead();
                  }, 3000);
                  return;
                }
                confirmClear = false;
                await api.clearHistory();
                store.history = [];
                notify("history");
              },
            },
            confirmClear ? "Click again to clear" : "Clear all",
          )
        : null,
    );
  }

  function entry(it: HistoryItem): HTMLLIElement {
    const li = h("li", { class: "entry", "data-id": it.id });
    const text = it.corrected ?? it.text;
    const isEditing = editing === it.id;

    const meta = h(
      "div",
      { class: "entry-meta" },
      h("time", { datetime: it.ts, title: new Date(it.ts).toLocaleString() }, timeAgo(it.ts)),
      h("span", { title: "Audio length" }, seconds(it.durationS)),
      h("span", { title: "Time from release to text" }, `${it.latencyMs} ms`),
      it.corrected ? h("span", { class: "badge badge-gold" }, "Fixed") : null,
    );

    if (isEditing) {
      const area = h("textarea", { class: "entry-edit", rows: 1, "aria-label": "Corrected text", spellcheck: "false" });
      area.value = text;
      const grow = () => {
        area.style.height = "auto";
        area.style.height = `${area.scrollHeight}px`;
      };
      area.addEventListener("input", grow);
      const save = async () => {
        const next = area.value.trim();
        if (!next || next === text) return cancel();
        saveBtn.disabled = true;
        try {
          const res = await api.correctHistory(it.id, next);
          lessons.set(it.id, res);
          it.corrected = next;
          editing = null;
          draw();
          void refreshDict();
          void refreshStats();
          if (res.learned.length) toast(`Learned: ${res.learned.map((c) => `${c.from} → ${c.to}`).join(", ")}`, "learned");
        } catch (e) {
          saveBtn.disabled = false;
          toast(`Couldn't save the fix. ${errorText(e)}`, "error");
        }
      };
      const cancel = () => {
        editing = null;
        draw();
      };
      area.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
          e.preventDefault();
          void save();
        } else if (e.key === "Escape") {
          e.preventDefault();
          cancel();
        }
      });
      const saveBtn = h("button", { class: "btn btn-primary btn-sm", type: "button", onclick: () => void save() }, icon("check", 16), "Save fix");
      li.classList.add("editing");
      li.append(
        area,
        h(
          "div",
          { class: "edit-bar" },
          h("span", { class: "hint" }, "Change the words Silas got wrong.", h("span", { class: "keys" }, keycap("Enter", "sm"), " to save, ", keycap("Esc", "sm"), " to cancel")),
          h("div", { class: "edit-actions" }, h("button", { class: "btn btn-ghost btn-sm", type: "button", onclick: cancel }, "Cancel"), saveBtn),
        ),
      );
      requestAnimationFrame(() => {
        grow();
        area.focus();
        area.setSelectionRange(area.value.length, area.value.length);
      });
      return li;
    }

    const lesson = lessons.get(it.id);
    append(li, [
      h("p", { class: "entry-text" }, text),
      h(
        "div",
        { class: "entry-foot" },
        meta,
        h(
          "div",
          { class: "entry-actions" },
          h(
            "button",
            {
              class: "btn btn-ghost btn-sm teach",
              type: "button",
              onclick: () => {
                editing = it.id;
                draw();
              },
            },
            icon("nib", 16),
            "Teach a fix",
          ),
          h("button", { class: "icon-btn", type: "button", title: "Copy", "aria-label": "Copy text", onclick: () => void copy(text) }, icon("copy", 16)),
          h(
            "button",
            {
              class: "icon-btn",
              type: "button",
              title: "Delete",
              "aria-label": "Delete dictation",
              onclick: async () => {
                li.classList.add("leaving");
                await api.deleteHistory(it.id);
                store.history = store.history.filter((x) => x.id !== it.id);
                lessons.delete(it.id);
                notify("history");
              },
            },
            icon("trash", 16),
          ),
        ),
      ),
      lesson ? lessonRow(lesson) : null,
    ]);
    return li;
  }

  function draw() {
    drawHead();
    if (!store.history.length) {
      clear(
        list,
        h(
          "li",
          { class: "empty" },
          h("p", null, "No dictations yet. Click into any text field, hold ", keycap(hotkeyLabel(), "sm"), " and speak."),
        ),
      );
      return;
    }
    clear(list, ...store.history.map(entry));
  }

  draw();
  // A new transcript should not throw away an edit in progress.
  const unsub = subscribe(["history"], () => {
    if (editing && store.history.some((x) => x.id === editing)) {
      const first = list.firstElementChild;
      const fresh = store.history[0];
      if (first && fresh && first.getAttribute("data-id") !== fresh.id && !list.querySelector(`[data-id="${CSS.escape(fresh.id)}"]`)) {
        list.prepend(entry(fresh));
      }
      drawHead();
      return;
    }
    editing = null;
    draw();
  });
  return unsub;
}

function lessonRow(res: LearnResult): HTMLElement {
  if (!res.learned.length && !res.pending.length && !res.terms.length) {
    return h("div", { class: "lesson" }, icon("check", 16), h("span", null, "Saved. Nothing new to learn from that one."));
  }
  return h(
    "div",
    { class: "lesson" },
    icon("spark", 16),
    h("span", { class: "lesson-label" }, "Silas learned"),
    ...res.learned.map((c) => fixChip(c.from, c.to)),
    ...res.pending.map((p) => fixChip(p.from, p.to, "pending")),
    res.pending.length ? h("span", { class: "hint" }, "Fix it once more and it's automatic.") : null,
    ...res.terms.filter((t) => !res.learned.some((c) => c.to === t)).map((t) => h("span", { class: "term-chip" }, t)),
  );
}

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = h("textarea", { style: "position:fixed;opacity:0" });
    ta.value = text;
    document.body.append(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
  toast("Copied");
}
