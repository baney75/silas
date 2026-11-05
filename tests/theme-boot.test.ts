import { describe, expect, test } from "bun:test";
import { CACHE_KEY, readCached, startTheme, writeCached, type KeyValue } from "../src/theme-boot";
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

function memory(): KeyValue & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return { data, getItem: (k) => data.get(k) ?? null, setItem: (k, v) => void data.set(k, v), removeItem: (k) => void data.delete(k) };
}

describe("cache for the first paint", () => {
  test("round trip, and null clears it", () => {
    const m = memory();
    expect(readCached(m)).toBeNull();
    writeCached(m, t("a"));
    expect(readCached(m)?.name).toBe("a");
    writeCached(m, null);
    expect(m.data.has(CACHE_KEY)).toBe(false);
    expect(readCached(m)).toBeNull();
  });

  test("junk in the cache is ignored", () => {
    const m = memory();
    for (const junk of ["", "{", "null", "[]", "5", '{"light":{}}']) {
      m.data.set(CACHE_KEY, junk);
      expect(readCached(m)).toBeNull();
    }
  });

  test("a storage that throws does not break startup", () => {
    const bad: KeyValue = {
      getItem: () => { throw new Error("off"); },
      setItem: () => { throw new Error("full"); },
      removeItem: () => { throw new Error("off"); },
    };
    expect(readCached(bad)).toBeNull();
    expect(() => writeCached(bad, t("a"))).not.toThrow();
  });
});
