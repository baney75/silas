import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import * as boot from "../src/theme-boot";
import { startTheme } from "../src/theme-boot";
import type { Theme } from "../src/api";

const t = (name: string) => ({ name, light: {}, dark: {}, shape: {}, type: {}, motion: {} }) as unknown as Theme;

/** A promise the test settles by hand. */
function gate<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((a, b) => ((resolve = a), (reject = b)));
  return { promise, resolve, reject };
}

describe("startTheme", () => {
  test("registers the listener before it asks for the snapshot", async () => {
    const order: string[] = [];
    const listening = gate<void>();
    const run = startTheme({
      listen: () => (order.push("listen"), listening.promise),
      snapshot: async () => (order.push("snapshot"), t("snap")),
      apply: () => {},
    });
    await Promise.resolve();
    expect(order).toEqual(["listen"]); // snapshot waits for the listener to be live
    listening.resolve();
    await run;
    expect(order).toEqual(["listen", "snapshot"]);
  });

  test("an event during the snapshot request wins over the snapshot", async () => {
    const applied: (string | null)[] = [];
    const snap = gate<Theme | null>();
    let fire!: (x: Theme | null) => void;
    const run = startTheme({
      listen: async (cb) => void (fire = cb),
      snapshot: () => snap.promise,
      apply: (x) => applied.push(x ? x.name : null),
    });
    await Promise.resolve();
    await Promise.resolve();
    fire(t("event")); // the file changed while the snapshot was in flight
    snap.resolve(t("old snapshot"));
    await run;
    expect(applied).toEqual(["event"]);
  });

  test("the snapshot applies when no event came, and later events still apply", async () => {
    const applied: (string | null)[] = [];
    let fire!: (x: Theme | null) => void;
    await startTheme({
      listen: async (cb) => void (fire = cb),
      snapshot: async () => t("snap"),
      apply: (x) => applied.push(x ? x.name : null),
    });
    fire(null);
    expect(applied).toEqual(["snap", null]);
  });

  test("a failed snapshot means the built-in look, unless an event already spoke", async () => {
    const applied: (string | null)[] = [];
    await startTheme({ listen: async () => {}, snapshot: async () => Promise.reject(new Error("x")), apply: (x) => applied.push(x?.name ?? null) });
    expect(applied).toEqual([null]);
  });
});

describe("first paint", () => {
  test("no cached theme is read or kept, so nothing stale paints when the file is gone", () => {
    expect(Object.keys(boot).sort()).toEqual(["startTheme"]);
    const src = readFileSync(resolve(import.meta.dir, "../src/theme.ts"), "utf8");
    expect(src).not.toMatch(/localStorage|readCached|writeCached/);
  });

  test("applyTheme is not called before the core has answered", () => {
    const src = readFileSync(resolve(import.meta.dir, "../src/theme.ts"), "utf8");
    // Every call sits inside a function body (indented), none at module level.
    expect(src).not.toMatch(/^applyTheme\(/m);
    expect(src).not.toMatch(/^try \{/m);
  });
});
