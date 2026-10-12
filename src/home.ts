import { hotkeyLabel, installUpdate, statusText, store, subscribe } from "./store";
import { clear, h, icon, keycap, num, timeAgo } from "./ui";
import type { Navigate } from "./main";

// Hidden until the next launch once the user says "later".
let dismissed = false;

export function mountHome(root: HTMLElement, go: Navigate): () => void {
  const updateBar = h("div");
  const hero = h("section", { class: "hero" });
  const banner = h("div");
  const stats = h("section", { class: "stats", "aria-label": "Your numbers" });
  const recent = h("section", { class: "recent" });
  root.append(updateBar, hero, banner, stats, recent);

  function drawUpdate() {
    const u = store.update;
    const p = store.updateProgress;
    if (!u || dismissed) return clear(updateBar);
    const pct = p && p.total ? Math.round((p.downloaded / p.total) * 100) : null;
    clear(
      updateBar,
      h(
        "div",
        { class: "update-bar", role: "status" },
        h("span", { class: "update-dot", "aria-hidden": "true" }),
        h("span", { class: "update-text" }, h("strong", null, `Silas ${u.version}`), p ? (pct === null ? " is downloading…" : ` is downloading… ${pct}%`) : " is ready."),
        p
          ? null
          : h(
              "span",
              { class: "update-actions" },
              h("button", { class: "link", type: "button", onclick: () => go("settings") }, "What's new"),
              h("button", { class: "btn btn-primary btn-sm", type: "button", onclick: () => void installUpdate() }, "Install and restart"),
              h(
                "button",
                {
                  class: "icon-btn",
                  type: "button",
                  "aria-label": "Hide until next launch",
                  title: "Later",
                  onclick: () => {
                    dismissed = true;
                    drawUpdate();
                  },
                },
                icon("x", 14),
              ),
            ),
      ),
    );
  }

  function drawHero() {
    const s = store.state;
    const busy = s.status === "recording" || s.status === "transcribing" || s.status === "typing";
    clear(
      hero,
      h(
        "div",
        { class: "hero-line" },
        h("span", null, "Hold"),
        keycap(hotkeyLabel(), "lg"),
        h("span", null, "and speak"),
      ),
      h(
        "p",
        { class: "hero-sub" },
        "Release to type",
        store.settings.tapToToggle ? h("span", { class: "sep", "aria-hidden": "true" }, "·") : null,
        store.settings.tapToToggle ? "Tap for hands-free" : null,
      ),
      h(
        "div",
        { class: `live live-${s.status}${busy ? " busy" : ""}`, role: "status", "aria-live": "polite" },
        h("span", { class: "dot" }),
        statusText[s.status],
        s.status === "error" && s.detail ? h("span", { class: "live-detail" }, s.detail) : null,
      ),
    );
    clear(
      banner,
      s.status === "needs_model"
        ? h(
            "div",
            { class: "banner" },
            icon("model", 20),
            h("div", null, h("strong", null, "Silas needs a speech model."), " It downloads once and runs on this computer."),
            h("button", { class: "btn btn-primary", type: "button", onclick: () => go("model") }, "Choose a model"),
          )
        : null,
    );
  }

  function drawStats() {
    const st = store.stats;
    const card = (label: string, value: string, hint: string) =>
      h("div", { class: "stat" }, h("div", { class: "stat-value" }, value), h("div", { class: "stat-label" }, label), h("div", { class: "stat-hint" }, hint));
    clear(
      stats,
      card("Words today", st ? num(st.wordsToday) : "–", st ? `${num(st.avgWpm)} wpm average pace` : ""),
      card("Total words", st ? num(st.wordsTotal) : "–", st ? `${num(st.dictations)} dictations` : ""),
      card("Minutes saved", st ? num(st.minutesSaved) : "–", "Compared with typing at 40 wpm"),
      card("Words learned", st ? num(st.learned) : "–", "From your fixes"),
    );
  }

  function drawRecent() {
    const items = store.history.slice(0, 3);
    clear(
      recent,
      h(
        "div",
        { class: "section-head" },
        h("h2", null, "Recent"),
        store.history.length > 3
          ? h("button", { class: "link", type: "button", onclick: () => go("history") }, "See all", icon("arrow", 14))
          : null,
      ),
      items.length
        ? h(
            "ul",
            { class: "recent-list" },
            ...items.map((it) =>
              h(
                "li",
                { class: "recent-item" },
                h("p", null, it.corrected ?? it.text),
                h("time", { datetime: it.ts }, timeAgo(it.ts)),
              ),
            ),
          )
        : h(
            "div",
            { class: "empty" },
            h("p", null, "Nothing yet. Click into any text field, hold ", keycap(hotkeyLabel(), "sm"), " and say a sentence."),
          ),
    );
  }

  drawUpdate();
  drawHero();
  drawStats();
  drawRecent();
  const unsubs = [
    subscribe(["state", "settings"], drawHero),
    subscribe(["stats"], drawStats),
    subscribe(["history", "state"], drawRecent),
    subscribe(["update"], drawUpdate),
  ];
  return () => unsubs.forEach((u) => u());
}
