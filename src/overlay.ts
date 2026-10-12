// The floating recording pill. Driven only by events: status, level and learned.
import "./overlay.css";
import { inTauri, on, type Status } from "./api";
import { startOverlayDemo } from "./mock";

type Face = "idle" | "recording" | "writing" | "typing" | "learned";

const pill = document.getElementById("pill")!;
const barsEl = document.getElementById("bars")!;
const elapsedEl = document.getElementById("elapsed")!;
const learnedEl = document.getElementById("learned")!;

const BARS = 16;
const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
const bars: HTMLElement[] = [];
for (let i = 0; i < BARS; i++) {
  const b = document.createElement("span");
  barsEl.append(b);
  bars.push(b);
}

// Rolling history of levels (newest on the right) and the eased value drawn per bar.
const targets = new Array<number>(BARS).fill(0);
const shown = new Array<number>(BARS).fill(0);
let latest = 0;
let lastPush = 0;
let raf = 0;
let startedAt = 0;
let learnedTimer = 0;
let face: Face = "idle";

function setFace(next: Face) {
  face = next;
  pill.dataset.state = next;
  if (next === "recording") {
    startedAt = performance.now();
    targets.fill(0);
    shown.fill(0);
    elapsedEl.textContent = "0:00";
    loop();
  }
}

function shape(level: number): number {
  // Speech levels sit low; lift quiet sounds so the bars feel alive.
  return Math.min(1, Math.pow(Math.max(0, level), 0.6) * 1.15);
}

function loop() {
  cancelAnimationFrame(raf);
  const tick = (t: number) => {
    if (face !== "recording") return;
    // Shift the history about 22 times a second so the wave flows right to left.
    if (t - lastPush > 45) {
      targets.shift();
      targets.push(shape(latest));
      lastPush = t;
    }
    const ease = reduced.matches ? 1 : 0.28;
    for (let i = 0; i < BARS; i++) {
      // Taper the edges so the waveform reads as a soft lozenge.
      const edge = Math.sin(((i + 0.5) / BARS) * Math.PI) * 0.55 + 0.45;
      shown[i] += (targets[i] * edge - shown[i]) * ease;
      bars[i].style.transform = `scaleY(${(0.14 + shown[i] * 0.86).toFixed(3)})`;
    }
    const s = Math.floor((t - startedAt) / 1000);
    const text = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
    if (elapsedEl.textContent !== text) elapsedEl.textContent = text;
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
}

function onStatus(status: Status) {
  if (status === "recording") {
    if (face !== "recording") setFace("recording");
  } else if (status === "transcribing") setFace("writing");
  else if (status === "typing") setFace("typing");
  else if (face !== "learned") setFace("idle");
}

void on("status", ({ status }) => onStatus(status));
void on("level", (level) => {
  latest = level;
});
void on("learned", (res) => {
  const c = res.learned[0];
  if (!c) return;
  learnedEl.textContent = "";
  const from = document.createElement("span");
  from.className = "from";
  from.textContent = c.from;
  const arrow = document.createElement("span");
  arrow.className = "arrow";
  arrow.textContent = "→";
  const to = document.createElement("span");
  to.className = "to";
  to.textContent = c.to;
  learnedEl.append(from, arrow, to);
  setFace("learned");
  window.clearTimeout(learnedTimer);
  learnedTimer = window.setTimeout(() => setFace("idle"), 2200);
});

if (!inTauri) {
  document.documentElement.dataset.preview = "true";
  startOverlayDemo();
}
