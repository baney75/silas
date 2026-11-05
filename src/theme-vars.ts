// Theme variables, with no window or document access so `bun test` can run it. `theme.ts` applies them.

import type { Theme, ThemeColors } from "./api";

const px = (n: number) => `${round(n)}px`;
const ms = (n: number) => `${round(n)}ms`;
const round = (n: number) => Math.round(n * 100) / 100;

/**
 * Silas draws radii at sizes the theme has no name for (2, 4, 5, 7, 8, 9, 11, 12, 20, 22 px).
 * Read them off the theme's three radii: a line through (6, small), (10, medium), (16, large),
 * and beyond 16 in proportion to the large one.
 */
export function radiusAt(size: number, small: number, medium: number, large: number): number {
  if (size <= 6) return (small * size) / 6;
  if (size <= 10) return small + ((medium - small) * (size - 6)) / 4;
  if (size <= 16) return medium + ((large - medium) * (size - 10)) / 6;
  return (large * size) / 16;
}

const RADII = [1, 2, 4, 5, 7, 8, 9, 11, 12, 20, 22];

/**
 * Every variable for one mode. `mixOk` says whether the webview has `color-mix`. Without it a
 * mixed colour becomes its plain first colour, never an invalid value.
 */
export function themeVars(t: Theme, isDark: boolean, mixOk = true): Record<string, string> {
  const mix = (a: string, pct: number, b: string) => (mixOk ? `color-mix(in srgb, ${a} ${pct}%, ${b})` : a);
  const alpha = (c: string, pct: number) => mix(c, pct, "transparent");
  const c: ThemeColors = isDark ? t.dark : t.light;
  const d: ThemeColors = t.dark; // the toast and the stopped-key shadows are always the dark palette
  const s = t.shape;
  const v: Record<string, string> = {
    // Main window
    "--bg": c.background,
    "--bg-side": c.sunken,
    "--surface": c.surface,
    "--surface-2": c.sunken,
    "--line": c.border,
    "--line-strong": mix(c.border, 65, c.foreground),
    "--text": c.foreground,
    "--text-2": mix(c.muted, 60, c.foreground),
    "--text-3": c.muted,
    "--gold": c.accent,
    "--gold-ink": mix(c.accent, 70, c.foreground), // text on tinted chips: a little toward foreground keeps 4.5
    "--gold-soft": alpha(c.accent, 12),
    "--gold-line": alpha(c.accent, 45),
    "--blue": c.working,
    "--blue-soft": alpha(c.working, 8),
    "--green": c.done,
    "--green-soft": alpha(c.done, 9),
    "--red": c.urgent,
    "--red-soft": alpha(c.urgent, 8),
    "--primary-bg": c.accent,
    "--primary-fg": c.on_accent,
    "--primary-hover": mix(c.accent, 90, c.foreground),
    "--focus": c.accent,
    "--key-face": c.surface,
    "--key-edge": c.border,
    "--key-hi": isDark ? "rgba(255, 255, 255, 0.06)" : "rgba(255, 255, 255, 0.5)",
    "--r-sm": px(s.radius_small),
    "--r": px(s.radius),
    "--r-lg": px(s.radius_large),
    "--r-pill": s.radius_large === 0 ? "0px" : "999px",
    "--serif": t.type.display,
    "--sans": t.type.ui,
    "--ease": t.motion.easing,
    "--thumb": c.surface,
    "--thumb-shadow": `0 1px 2px ${mix(c.foreground, 25, "transparent")}`,
    "--seg-shadow": `0 1px 2px ${mix(c.foreground, 15, "transparent")}`,
    // Toast: always the dark palette, as today.
    "--toast-bg": d.surface,
    "--toast-fg": d.foreground,
    "--toast-border": d.border,
    "--toast-border-w": isDark ? "1px" : "0px",
    "--toast-ok": d.done,
    "--toast-learned": d.accent,
    "--toast-error": d.urgent,
    // Overlay
    "--gold-hi": mix(c.accent, 70, c.foreground),
    "--learned-arrow": mix(c.accent, 70, c.foreground),
    "--rec": c.urgent,
    "--glass": alpha(c.surface, 90),
    "--muted": c.muted,
    "--rec-glow-0": alpha(c.urgent, 55),
    "--rec-glow-1": alpha(c.urgent, 50),
    "--rec-glow-clear": alpha(c.urgent, 0),
    "--ink-track": alpha(c.accent, 16),
    "--pill-hi": isDark ? "rgba(255, 255, 255, 0.06)" : "rgba(255, 255, 255, 0.5)",
    "--blur": px(s.blur),
  };
  // Shadows: `shape.shadow` is an opacity. Dark surfaces need more of it to show.
  const o = Math.min(1, s.shadow * (isDark ? 2.5 : 1));
  const ink = (a: number) => `rgba(0, 0, 0, ${round(Math.min(1, a))})`;
  v["--shadow"] = `0 1px 0 ${ink(o * 0.25)}, 0 1px 3px ${ink(o * 0.5)}`;
  v["--shadow-lg"] = `0 12px 32px ${ink(o)}, 0 2px 6px ${ink(o * 0.5)}`;
  v["--pill-shadow"] = `0 4px 14px ${ink(o * 2)}`;
  for (const r of RADII) {
    v[`--r-${r}`] = px(radiusAt(r, s.radius_small, s.radius, s.radius_large));
  }
  const m = t.motion;
  v["--motion-fast"] = ms(m.reduced ? 0 : m.fast);
  v["--motion-normal"] = ms(m.reduced ? 0 : m.normal);
  v["--motion-slow"] = ms(m.reduced ? 0 : m.slow);
  return v;
}


/** The part of CSSStyleDeclaration the applier uses. */
export interface StyleLike {
  setProperty(name: string, value: string): void;
  removeProperty(name: string): unknown;
}

/** Sets a theme's variables on `style` and removes every one it set when given null. */
export function createApplier(style: StyleLike) {
  let applied = new Set<string>();
  return {
    apply(t: Theme | null, isDark: boolean, mixOk = true): void {
      for (const k of applied) style.removeProperty(k);
      applied = new Set();
      if (!t) return;
      for (const [k, val] of Object.entries(themeVars(t, isDark, mixOk))) {
        style.setProperty(k, val);
        applied.add(k);
      }
    },
  };
}
