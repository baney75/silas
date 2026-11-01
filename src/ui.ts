// Small DOM helpers, icons and shared widgets. No framework: views rebuild their own
// subtree from data, and user text always goes through textContent.

type Child = Node | string | number | null | undefined | false;
type Attrs = Record<string, string | number | boolean | EventListener | null | undefined>;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs | null = null,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (attrs) {
    for (const [key, value] of Object.entries(attrs)) {
      if (value === null || value === undefined || value === false) continue;
      if (key.startsWith("on") && typeof value === "function") {
        el.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
      } else if (key === "class") {
        el.className = String(value);
      } else if (key === "html") {
        el.innerHTML = String(value); // only for constant icon markup
      } else if (value === true) {
        el.setAttribute(key, "");
      } else {
        el.setAttribute(key, String(value));
      }
    }
  }
  append(el, children);
  return el;
}

export function append(el: Element, children: Child[]) {
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

export function clear(el: Element, ...children: Child[]) {
  el.replaceChildren();
  append(el, children);
}

const paths: Record<string, string> = {
  home: '<path d="M4 10.5 12 4l8 6.5V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1z"/>',
  history: '<circle cx="12" cy="12" r="8"/><path d="M12 7.5V12l3 2"/>',
  book: '<path d="M5 4.5h9.5A3.5 3.5 0 0 1 18 8v11.5H8.5A3.5 3.5 0 0 1 5 16z"/><path d="M5 16a3 3 0 0 1 3-3h10"/>',
  model: '<rect x="5" y="5" width="14" height="14" rx="3"/><path d="M9.5 9.5h5v5h-5zM9 2.5V5M15 2.5V5M9 19v2.5M15 19v2.5M2.5 9H5M2.5 15H5M19 9h2.5M19 15h2.5"/>',
  settings: '<path d="M4 7h10M18 7h2M4 17h4M12 17h8"/><circle cx="16" cy="7" r="2"/><circle cx="10" cy="17" r="2"/>',
  copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2"/><path d="M15.5 8.5V6a1.5 1.5 0 0 0-1.5-1.5H6A1.5 1.5 0 0 0 4.5 6v8A1.5 1.5 0 0 0 6 15.5h2.5"/>',
  trash: '<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5h9l1-12.5"/>',
  nib: '<path d="M12 21c-.6-2-5-5.6-5.3-9.6-.1-1.7.4-3 1.2-3.9V4.5h8.2v3c.8.9 1.3 2.2 1.2 3.9-.3 4-4.7 7.6-5.3 9.6z"/><path d="M12 11v10"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  x: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
  download: '<path d="M12 4.5v11M7.5 11 12 15.5l4.5-4.5M5 19.5h14"/>',
  upload: '<path d="M12 15.5v-11M7.5 9 12 4.5 16.5 9M5 19.5h14"/>',
  mic: '<rect x="9" y="3.5" width="6" height="11" rx="3"/><path d="M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v2.5"/>',
  shield: '<path d="M12 3.5 19 6v5.5c0 4.2-2.9 7.6-7 9-4.1-1.4-7-4.8-7-9V6z"/><path d="m9 12 2.2 2.2L15.5 10"/>',
  keyboard: '<rect x="3" y="6" width="18" height="12" rx="2.5"/><path d="M7 10h.01M11 10h.01M15 10h.01M7 14h10"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  spark: '<path d="M12 4v4M12 16v4M4 12h4M16 12h4M6.5 6.5l2.5 2.5M15 15l2.5 2.5M6.5 17.5 9 15M15 9l2.5-2.5"/>',
  lock: '<rect x="5" y="10.5" width="14" height="10" rx="2"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/>',
};

export function icon(name: keyof typeof paths | string, size = 18): SVGSVGElement {
  const wrap = document.createElement("span");
  wrap.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] ?? ""}</svg>`;
  return wrap.firstElementChild as SVGSVGElement;
}

/** The brand nib mark, drawn in the two gold faces. */
export function logo(size = 28): SVGSVGElement {
  const wrap = document.createElement("span");
  wrap.innerHTML = `<svg width="${size}" height="${size}" viewBox="330 190 364 700" aria-hidden="true" class="logo-mark">
    <defs><mask id="nibcut-${size}" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">
      <rect width="1024" height="1024" fill="#fff"/>
      <g stroke="#000" stroke-linecap="round" stroke-width="30">
        <path d="M512 330V900M452 388v160M572 388v160M396 436v64M628 436v64"/>
      </g>
      <path d="M396 300h232" stroke="#000" stroke-width="16"/>
    </mask></defs>
    <g mask="url(#nibcut-${size})">
      <path fill="#B98F4B" d="M512 872c-20-72-152-232-162-402-4-70 18-130 46-166v-52q0-16 16-16h200q16 0 16 16v52c28 36 50 96 46 166-10 170-142 330-162 402z"/>
      <path fill="#D9B676" d="M512 872c-20-72-152-232-162-402-4-70 18-130 46-166v-52q0-16 16-16h100z"/>
    </g>
  </svg>`;
  return wrap.firstElementChild as SVGSVGElement;
}

export function keycap(label: string, size: "sm" | "md" | "lg" = "md"): HTMLElement {
  return h("kbd", { class: `keycap keycap-${size}` }, label);
}

// ---- Toasts ----

let toastHost: HTMLElement | null = null;

export function toast(message: string | Node, kind: "info" | "learned" | "error" = "info", ms = 3600) {
  if (!toastHost) {
    toastHost = h("div", { class: "toasts", role: "status", "aria-live": "polite" });
    document.body.append(toastHost);
  }
  const el = h(
    "div",
    { class: `toast toast-${kind}` },
    kind === "learned" ? icon("spark", 16) : kind === "error" ? icon("x", 16) : icon("check", 16),
    h("span", null, message),
  );
  toastHost.append(el);
  requestAnimationFrame(() => el.classList.add("in"));
  window.setTimeout(() => {
    el.classList.remove("in");
    el.addEventListener("transitionend", () => el.remove(), { once: true });
    window.setTimeout(() => el.remove(), 400);
  }, ms);
}

/** "Argon one → Argon1" rendered as a chip. */
export function fixChip(from: string, to: string, kind: "learned" | "pending" = "learned"): HTMLElement {
  return h(
    "span",
    { class: `fix-chip fix-${kind}` },
    h("span", { class: "fix-from" }, from),
    h("span", { class: "fix-arrow", "aria-label": "becomes" }, "→"),
    h("span", { class: "fix-to" }, to),
  );
}

// ---- Formatting ----

export function timeAgo(ts: string): string {
  const s = Math.max(0, (Date.now() - new Date(ts).getTime()) / 1000);
  if (s < 45) return "just now";
  if (s < 90) return "1 min ago";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 5400) return "1 hr ago";
  if (s < 86_400) return `${Math.round(s / 3600)} hr ago`;
  if (s < 172_800) return "yesterday";
  if (s < 604_800) return `${Math.round(s / 86_400)} days ago`;
  return new Date(ts).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function num(n: number): string {
  return Math.round(n).toLocaleString();
}

export function seconds(s: number): string {
  if (s < 60) return `${s.toFixed(1)} s`;
  return `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;
}

export function megabytes(mb: number): string {
  return mb >= 1000 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}

export function switchEl(checked: boolean, label: string, onChange: (v: boolean) => void): HTMLButtonElement {
  const btn = h("button", {
    class: "switch",
    type: "button",
    role: "switch",
    "aria-checked": String(checked),
    "aria-label": label,
  });
  btn.append(h("span", { class: "switch-thumb" }));
  btn.addEventListener("click", () => {
    const next = btn.getAttribute("aria-checked") !== "true";
    btn.setAttribute("aria-checked", String(next));
    onChange(next);
  });
  return btn;
}

export function segmented<T extends string>(
  options: Array<{ value: T; label: string }>,
  value: T,
  label: string,
  onChange: (v: T) => void,
): HTMLElement {
  const group = h("div", { class: "segmented", role: "radiogroup", "aria-label": label });
  const buttons = options.map((o) => {
    const b = h("button", { type: "button", role: "radio", "aria-checked": String(o.value === value) }, o.label);
    b.addEventListener("click", () => {
      buttons.forEach((x) => x.setAttribute("aria-checked", String(x === b)));
      onChange(o.value);
    });
    b.addEventListener("keydown", (e) => {
      const i = buttons.indexOf(b);
      const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      e.preventDefault();
      const next = buttons[(i + step + buttons.length) % buttons.length];
      next.focus();
      next.click();
    });
    return b;
  });
  append(group, buttons);
  return group;
}

export function progressBar(fraction: number | null, label: string): HTMLElement {
  const bar = h("div", {
    class: `progress${fraction === null ? " indeterminate" : ""}`,
    role: "progressbar",
    "aria-label": label,
    "aria-valuemin": 0,
    "aria-valuemax": 100,
    "aria-valuenow": fraction === null ? null : Math.round(fraction * 100),
  });
  const fill = h("div", { class: "progress-fill" });
  if (fraction !== null) fill.style.transform = `scaleX(${Math.max(0, Math.min(1, fraction))})`;
  bar.append(fill);
  return bar;
}
