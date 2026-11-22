// The shared vectors (registry/vectors.json) and ingest.py's own answers for them, through the
// whole POST handler, plus a wider set of mutations straight through the ported validator.

import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { Reject, readReport, validate } from "../src/validate";
import { post } from "./helpers";
import type { ParityCase } from "./env";
import { BASE, VECTORS, apply } from "./vectors";

const PARITY = env.PARITY;

/** What the ported validator makes of a body, in the shape parity.py reports. */
function outcome(body: string): Omit<ParityCase, "name" | "body"> {
  try {
    return { ok: true, report: validate(readReport(body)) };
  } catch (e) {
    if (!(e instanceof Reject)) throw e;
    return { ok: false, error: e.message, stale: e.stale };
  }
}

let ip = 0;
const nextIp = () => `192.0.2.${++ip % 250}`;

describe("shared vectors over HTTP", () => {
  it("vectors.json has cases of every kind", () => {
    expect(VECTORS.good.length).toBeGreaterThan(0);
    expect(VECTORS.bad.length).toBeGreaterThan(0);
    expect(VECTORS.stale.length).toBeGreaterThan(0);
  });

  for (const c of VECTORS.good) {
    it(`good: ${c.name}`, async () => {
      const res = await post(apply(c), nextIp());
      expect(res.status, await res.clone().text()).toBe(201);
      const out = (await res.json()) as { ok: boolean; key: string; reports: number };
      expect(out.ok).toBe(true);
      expect(out.reports).toBe(1);
      const row = await env.DB.prepare("SELECT report FROM reports").first<string>("report");
      const stored = JSON.parse(row!);
      expect(stored).not.toHaveProperty("pick");
      expect(stored).not.toHaveProperty("ran_at");
      expect(stored).not.toHaveProperty("extra");
      expect(out.key).toBe(stored.device.key);
    });
  }

  for (const kind of ["bad", "stale"] as const) {
    for (const c of VECTORS[kind]) {
      it(`${kind}: ${c.name}`, async () => {
        const res = await post(apply(c), nextIp());
        expect(res.status).toBe(422);
        const out = (await res.json()) as { error: string; stale: boolean };
        expect(out.stale).toBe(kind === "stale");
        expect(typeof out.error).toBe("string");
        expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM reports").first("n")).toBe(0);
      });
    }
  }
});

describe("parity with registry/ingest.py", () => {
  for (const kind of ["good", "bad", "stale"] as const) {
    it(`apply() builds the same ${kind} bodies as test_ingest.py`, () => {
      const ours = VECTORS[kind].map((c) => apply(c));
      const theirs = PARITY.vectors[kind].map((p) => p.body);
      // Python writes 10.0 where JSON.stringify writes 10; the parsed values must still match.
      ours.forEach((body, i) => {
        if (body === theirs[i]) return;
        expect(outcome(body), VECTORS[kind][i].name).toEqual(outcome(theirs[i]));
      });
    });

    it(`same decision, reason and cleaned report for every ${kind} vector`, async () => {
      for (const p of PARITY.vectors[kind]) {
        const { name: _n, body: _b, ...want } = p;
        expect(outcome(apply(VECTORS[kind].find((c) => c.name === p.name)!)), p.name).toEqual(want);
        expect(outcome(p.body), p.name).toEqual(want);
      }
    });
  }

  it(`same answer as ingest.py for ${PARITY.mutations.length} mutations of the base report`, () => {
    expect(PARITY.mutations.length).toBeGreaterThan(500);
    for (const p of PARITY.mutations) {
      const { name: _n, body, ...want } = p;
      expect(outcome(body), p.name).toEqual(want);
    }
  });

  it("Python's whole-number floats reach the handler as written and are stored as integers", async () => {
    const body = JSON.stringify(BASE, null, 2).replace('"cores": 10', '"cores": 10.0').replace('"clips": 14', '"clips": 14.0');
    expect(body).toContain('"cores": 10.0');
    const res = await post(body, nextIp());
    expect(res.status).toBe(201);
    const stored = JSON.parse((await env.DB.prepare("SELECT report FROM reports").first<string>("report"))!);
    expect(stored.device.cores).toBe(10);
    expect(stored.results[0].clips).toBe(14);
  });
});
