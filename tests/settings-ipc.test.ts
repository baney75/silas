import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Opening Settings waits on nothing: what it shows is in the store already. The microphone list is
// read at launch and on focus (`refreshMicrophones` in store.ts), so the page asks the core only when
// you press Change for the hotkey. The page used to ask for the list on open, a call that ran on the
// main thread and froze the window while Linux opened every ALSA device.
test("opening Settings makes no IPC call", () => {
  const src = readFileSync(resolve(import.meta.dir, "../src/settings.ts"), "utf8");
  const calls = [...src.matchAll(/\bapi\s*\.\s*(\w+)\s*\(/g)].map((m) => m[1]);
  expect(calls).toEqual(["captureHotkey"]);
});
