import { expect, test } from "bun:test";
import { encode } from "uqr";
import type { SyncStatus } from "../src/api";
import { syncLine } from "../src/sync";

const base: SyncStatus = {
  enabled: true,
  paired: true,
  deviceId: "d1",
  name: "Desk",
  platform: "linux",
  port: 47823,
  peers: [],
  folder: null,
  devices: [],
  last: null,
};

test("the status line says who synced and what came in", () => {
  const at = Date.now();
  expect(syncLine({ ...base, last: { with: "Phone", at, added: 3, changed: 0, removed: 0 } })).toBe("Synced with Phone just now (3 new words)");
  expect(syncLine({ ...base, last: { with: "Phone", at, added: 1, changed: 2, removed: 1 } })).toBe("Synced with Phone just now (1 new word, 2 changed, 1 removed)");
  expect(syncLine({ ...base, last: { with: "Phone", at, added: 0, changed: 0, removed: 0 } })).toBe("Synced with Phone just now");
  expect(syncLine(base)).toBe("On. No other devices yet.");
  expect(syncLine({ ...base, enabled: false, paired: false })).toContain("stays on this computer");
});

test("the pairing URL fits a QR code a phone can scan", () => {
  // docs/sync.md: silas://pair?k= and 52 base32 letters.
  const qr = encode(`silas://pair?k=${"A".repeat(52)}`, { ecc: "M", border: 2 });
  expect(qr.size).toBeGreaterThan(20);
  expect(qr.size).toBeLessThan(50);
  expect(qr.data.length).toBe(qr.size);
});

test("Settings fetches the pairing code only from a click", () => {
  // The code must not be read when Settings opens: the only call sits in the Pair button's handler.
  const src = require("node:fs").readFileSync(require("node:path").resolve(import.meta.dir, "../src/sync.ts"), "utf8") as string;
  const at = src.indexOf("api.syncPairingCode(");
  expect(at).toBeGreaterThan(0);
  expect(src.indexOf("api.syncPairingCode(", at + 1)).toBe(-1);
  expect(src.slice(src.lastIndexOf("onclick:", at), at)).toContain("async () =>");
});
