// Polycarp theme for Silas. Used by the main window and the overlay.
// The core checks the file (docs/ipc.md, "Theme") and sends a Theme or null. `applyTheme` turns it
// into CSS variables on <html>. null removes every variable it set, so the built-in look returns.

import { api, on, type Theme } from "./api";
import { createApplier } from "./theme-vars";
import { readCached, startTheme, writeCached } from "./theme-boot";

export { radiusAt, themeVars } from "./theme-vars";

const root = document.documentElement;
const dark = window.matchMedia("(prefers-color-scheme: dark)");

let theme: Theme | null = null;
const applier = createApplier(root.style);
const mixOk = typeof CSS !== "undefined" && CSS.supports("color", "color-mix(in srgb, red 50%, blue)");
let reduced = false;

/** True when the theme asks for no motion. The overlay's JS reads this next to the system setting. */
export function themeReducedMotion(): boolean {
  return reduced;
}

function activeIsDark(t: Theme): boolean {
  return t.mode === "dark" || (t.mode === "auto" && dark.matches);
}

function clearAll() {
  applier.apply(null, false);
  root.style.removeProperty("color-scheme");
  delete root.dataset.reducedMotion;
  reduced = false;
}

/** Set the theme's variables, or with null remove every one this module set. */
export function applyTheme(next: Theme | null): void {
  theme = next;
  clearAll();
  if (!next) return;
  const isDark = activeIsDark(next);
  applier.apply(next, isDark, mixOk);
  root.style.setProperty("color-scheme", isDark ? "dark" : "light");
  reduced = next.motion.reduced;
  if (reduced) root.dataset.reducedMotion = "true";
}

// Paint with the last theme the core sent, now, before the core answers. The core's answer replaces it.
try {
  applyTheme(readCached(localStorage));
} catch {
  applyTheme(null);
}

/** Load the theme, then follow the core's `theme_changed` and the system light/dark setting. */
export async function initTheme(): Promise<void> {
  dark.addEventListener("change", () => {
    if (theme && theme.mode === "auto") applyTheme(theme);
  });
  await startTheme({
    listen: (cb) => on("theme_changed", cb),
    snapshot: () => api.getTheme(),
    apply: (t) => {
      try {
        applyTheme(t);
      } catch {
        applyTheme(null);
      }
      writeCached(localStorage, t);
    },
  });
}
