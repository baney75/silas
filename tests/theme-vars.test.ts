import { beforeAll, describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createApplier, radiusAt, themeVars } from "../src/theme-vars";
import type { Theme } from "../src/api";

// Every variable the CSS may read from the theme. A rename or a dropped one fails here.
const EXPECTED = [
  "--bg", "--bg-side", "--surface", "--surface-2", "--line", "--line-strong", "--text", "--text-2", "--text-3",
  "--gold", "--gold-ink", "--gold-soft", "--gold-line", "--blue", "--blue-soft", "--green", "--green-soft",
  "--red", "--red-soft", "--primary-bg", "--primary-fg", "--primary-hover", "--focus", "--key-face", "--key-edge",
  "--key-hi", "--r-sm", "--r", "--r-lg", "--r-pill", "--serif", "--sans", "--ease", "--thumb", "--thumb-shadow",
  "--seg-shadow", "--toast-bg", "--toast-fg", "--toast-border", "--toast-border-w", "--toast-ok", "--toast-learned",
  "--toast-error", "--gold-hi", "--learned-arrow", "--rec", "--glass", "--muted", "--rec-glow-0", "--rec-glow-1",
  "--rec-glow-clear", "--ink-track", "--pill-hi", "--blur", "--shadow", "--shadow-lg", "--pill-shadow",
  ...[1, 2, 4, 5, 7, 8, 9, 11, 12, 20, 22].map((r) => `--r-${r}`),
  "--motion-fast", "--motion-normal", "--motion-slow",
];

// The built-in themes, written by the real `polycarp theme set` into a throwaway folder.
const themes: Record<string, Theme> = {};
let dir = "";
beforeAll(() => {
  dir = mkdtempSync(join(tmpdir(), "silas-vars-"));
  const manifest = resolve(import.meta.dir, "../../cli/Cargo.toml");
  const env = { ...process.env, POLYCARP_CONFIG_DIR: dir, XDG_CONFIG_HOME: "" };
  const run = (...a: string[]) =>
    process.env.POLYCARP_BIN
      ? execFileSync(process.env.POLYCARP_BIN, a, { env }).toString()
      : execFileSync("cargo", ["run", "-q", "--manifest-path", manifest, "--", ...a], { env }).toString();
  const list = JSON.parse(run("theme", "list", "--json")).data.themes as { name: string; source: string }[];
  for (const { name } of list.filter((t) => t.source === "built-in")) {
    run("theme", "set", name);
    const f = JSON.parse(readFileSync(join(dir, "current", "theme.json"), "utf8"));
    const v = f.surfaces.voice;
    themes[name] = { name: f.name, mode: f.mode, light: v.light, dark: v.dark, shape: v.shape, type: v.type, motion: v.motion };
  }
}, 120_000);

describe("themeVars", () => {
  test("there are built-in themes to test", () => {
    expect(Object.keys(themes).length).toBeGreaterThanOrEqual(6);
  });

  test("every built-in theme gives every expected variable in both modes", () => {
    for (const [name, t] of Object.entries(themes)) {
      for (const isDark of [false, true]) {
        const got = themeVars(t, isDark);
        expect(Object.keys(got).sort(), `${name} dark=${isDark}`).toEqual([...EXPECTED].sort());
        for (const [k, v] of Object.entries(got)) {
          expect(v, `${name} ${k}`).not.toMatch(/undefined|NaN|\[object/);
          expect(v.trim(), `${name} ${k}`).not.toBe("");
        }
      }
    }
  });

  test("the two modes differ where they should", () => {
    const t = Object.values(themes)[0];
    expect(themeVars(t, false)["--bg"]).toBe(t.light.background);
    expect(themeVars(t, true)["--bg"]).toBe(t.dark.background);
    expect(themeVars(t, true)["--toast-border-w"]).toBe("1px");
    expect(themeVars(t, false)["--toast-border-w"]).toBe("0px");
  });

  test("null removes every variable that was set", () => {
    const style = new Map<string, string>();
    const applier = createApplier({
      setProperty: (k, v) => void style.set(k, v),
      removeProperty: (k) => void style.delete(k),
    });
    style.set("color", "red"); // not the applier's: must survive
    for (const t of Object.values(themes)) {
      applier.apply(t, true);
      expect(style.size).toBe(EXPECTED.length + 1);
      applier.apply(t, false); // switching mode leaves no stale variable
      expect(style.size).toBe(EXPECTED.length + 1);
      applier.apply(null, false);
      expect([...style.keys()]).toEqual(["color"]);
    }
  });

  test("without color-mix no value is invalid: mixes become the plain colour", () => {
    for (const t of Object.values(themes)) {
      const c = t.light;
      const got = themeVars(t, false, false);
      expect(Object.keys(got).sort()).toEqual([...EXPECTED].sort());
      for (const [k, v] of Object.entries(got)) expect(v, k).not.toContain("color-mix");
      expect(got["--gold-soft"]).toBe(c.accent);
      expect(got["--line-strong"]).toBe(c.border);
      expect(got["--glass"]).toBe(c.surface);
      expect(themeVars(t, false, true)["--gold-soft"]).toContain("color-mix");
    }
  });
});

describe("radiusAt", () => {
  const [s, m, l] = [4, 10, 20];
  test("passes through the three named radii", () => {
    expect(radiusAt(6, s, m, l)).toBe(s);
    expect(radiusAt(10, s, m, l)).toBe(m);
    expect(radiusAt(16, s, m, l)).toBe(l);
  });
  test("is proportional below 6 and above 16, and a line between", () => {
    expect(radiusAt(0, s, m, l)).toBe(0);
    expect(radiusAt(3, s, m, l)).toBe(2);
    expect(radiusAt(8, s, m, l)).toBe(7);
    expect(radiusAt(13, s, m, l)).toBe(15);
    expect(radiusAt(32, s, m, l)).toBe(40);
  });
  test("zero radii stay square everywhere", () => {
    for (const r of [1, 2, 6, 9, 12, 22]) expect(radiusAt(r, 0, 0, 0)).toBe(0);
  });
});

process.on("exit", () => dir && rmSync(dir, { recursive: true, force: true }));
