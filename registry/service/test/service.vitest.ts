import { createExecutionContext, createScheduledController, waitOnExecutionContext } from "cloudflare:test";
import { env, exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { ACCEPTED_PER_DAY, CLIENT_POSTS_PER_DAY, EXPORT_PAGE, MAX_BODY, MIN_AGREE, handler, store, utcDay } from "../src/service";
import { MAX_SPEED, MAX_WER, MIN_SPEED, SUITE_ID, TIE_WER, validate } from "../src/validate";
import * as main from "../src/index";
import { ORIGIN, call, post } from "./helpers";
import { BASE } from "./vectors";

const SECONDS = BASE.results[0].audio_seconds;

function result(model: string, speed: number, wer: number, wait: number) {
  return { model, speed, wer, clips: BASE.results[0].clips, audio_seconds: SECONDS, compute_seconds: SECONDS / speed, wait_seconds: wait };
}

function report(results: ReturnType<typeof result>[], device: Partial<typeof BASE.device> = {}) {
  return JSON.stringify({ ...BASE, device: { ...BASE.device, ...device }, results });
}

const today = () => utcDay(new Date());
// workerd warns on .text() for a non-text type; ndjson is UTF-8 text.
const ndjson = async (res: Response) => new TextDecoder().decode(await res.arrayBuffer());
const daysAgo = (n: number) => utcDay(new Date(Date.now() - n * 86_400_000));

async function dumpAllTables(): Promise<string> {
  const tables = await env.DB.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND substr(name, 1, 4) != '_cf_'").all<{ name: string }>();
  const out: Record<string, unknown> = {};
  for (const { name } of tables.results) {
    out[name] = (await env.DB.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}"`).all()).results;
  }
  return JSON.stringify(out);
}

describe("bars", () => {
  it("match registry/ingest.py", () => {
    const py = env.INGEST_PY;
    const bar = (name: string) => {
      const m = py.match(new RegExp(`^${name} = ([0-9._]+)$`, "m"));
      if (!m) throw new Error(`${name} not found in ingest.py`);
      return Number(m[1].replaceAll("_", ""));
    };
    expect(MIN_SPEED).toBe(bar("MIN_SPEED"));
    expect(MAX_WER).toBe(bar("MAX_WER"));
    expect(TIE_WER).toBe(bar("TIE_WER"));
    expect(MAX_SPEED).toBe(bar("MAX_SPEED"));
    expect([MIN_SPEED, MAX_WER, TIE_WER]).toEqual([10.0, 0.08, 0.005]);
  });
});

describe("routing", () => {
  it("404 for unknown paths, 405 for wrong methods", async () => {
    const missing = await call("/nope");
    expect(missing.status).toBe(404);
    expect(await missing.json()).toEqual({ error: "Not found." });
    const proto = await call("/__proto__");
    expect(proto.status).toBe(404);
    const wrong = await call("/v1/reports");
    expect(wrong.status).toBe(405);
    expect(wrong.headers.get("Allow")).toBe("POST");
    expect((await call("/v1/devices", { method: "POST", body: "{}" })).status).toBe(405);
    expect((await call("/v1/reports.jsonl", { method: "DELETE" })).status).toBe(405);
  });

  it("the main module exports only the handler, or workerd won't start it", () => {
    expect(Object.keys(main)).toEqual(["default"]);
  });

  it("serves requests through the Worker's own entrypoint", async () => {
    const res = await exports.default.fetch(
      new Request(`${ORIGIN}/v1/reports`, { method: "POST", body: report([result("parakeet-v3", 20.5, 0.061, 0.38)]), headers: { "CF-Connecting-IP": "198.51.100.9" } }),
    );
    expect(res.status).toBe(201);
  });
});

describe("POST /v1/reports", () => {
  it("201 with only validated values", async () => {
    const res = await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]));
    expect(res.status).toBe(201);
    expect(await res.json()).toEqual({ ok: true, key: "macos/apple-m5/10", reports: 1, pick: "parakeet-v3", agree: 1 });
    const row = await env.DB.prepare("SELECT day, suite, device_key FROM reports").first();
    expect(row).toEqual({ day: today(), suite: SUITE_ID, device_key: "macos/apple-m5/10" });
    expect(await env.DB.prepare("SELECT accepted FROM daily WHERE day = ?").bind(today()).first("accepted")).toBe(1);
  });

  it("503 without RATE_KEY, and nothing is stored", async () => {
    const res = await call(
      "/v1/reports",
      { method: "POST", body: report([result("parakeet-v3", 20.5, 0.061, 0.38)]) },
      { DB: env.DB },
    );
    expect(res.status).toBe(503);
    expect(await dumpAllTables()).not.toContain("apple-m5");
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM clients").first("n")).toBe(0);
  });

  it("413 over 20 KB, by Content-Length and by the stream", async () => {
    // Exactly the cap is fine.
    const exact = report([result("parakeet-v3", 20.5, 0.061, 0.38)]);
    const padded = exact.padEnd(MAX_BODY, " ");
    expect(new TextEncoder().encode(padded).byteLength).toBe(MAX_BODY);
    expect((await post(padded, "198.51.100.2")).status).toBe(201);

    expect((await post(padded + " ", "198.51.100.3")).status).toBe(413);

    const declared = await call("/v1/reports", {
      method: "POST",
      body: exact,
      headers: { "CF-Connecting-IP": "198.51.100.4", "Content-Length": String(MAX_BODY + 1) },
    });
    expect(declared.status).toBe(413);

    // A stream with no length that runs past the cap: reading stops, nothing is stored.
    let pulled = 0;
    const stream = new ReadableStream<Uint8Array>({
      pull(controller) {
        pulled++;
        if (pulled > 100) controller.close();
        else controller.enqueue(new Uint8Array(4096).fill(32));
      },
    });
    const streamed = await post(stream, "198.51.100.5");
    expect(streamed.status).toBe(413);
    expect(pulled).toBeLessThan(100);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM reports").first("n")).toBe(1);
  });

  it("429 after 20 POSTs from one client while another still gets through", async () => {
    for (let i = 0; i < CLIENT_POSTS_PER_DAY; i++) {
      // Rejected posts count too.
      const res = await post(i % 2 ? "not json" : report([result("parakeet-v3", 20.5, 0.061, 0.38)]), "203.0.113.10");
      expect(res.status).not.toBe(429);
    }
    const blocked = await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]), "203.0.113.10");
    expect(blocked.status).toBe(429);
    const retry = Number(blocked.headers.get("Retry-After"));
    expect(retry).toBeGreaterThan(0);
    expect(retry).toBeLessThanOrEqual(86_400);
    expect((await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]), "203.0.113.11")).status).toBe(201);
    // No CF-Connecting-IP: all such posts share the "unknown" client.
    const anon = await call("/v1/reports", { method: "POST", body: report([result("parakeet-v3", 20.5, 0.061, 0.38)]) });
    expect(anon.status).toBe(201);
  });

  it("429 with Retry-After once the day's accepted reports reach 5,000", async () => {
    await env.DB.prepare("INSERT INTO daily (day, accepted) VALUES (?, ?)").bind(today(), ACCEPTED_PER_DAY - 1).run();
    expect((await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]), "203.0.113.20")).status).toBe(201);
    expect(await env.DB.prepare("SELECT accepted FROM daily WHERE day = ?").bind(today()).first("accepted")).toBe(ACCEPTED_PER_DAY);
    const res = await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]), "203.0.113.21");
    expect(res.status).toBe(429);
    const retry = Number(res.headers.get("Retry-After"));
    const now = new Date();
    const midnight = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1);
    expect(Math.abs(retry - (midnight - now.getTime()) / 1000)).toBeLessThan(5);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM reports").first("n")).toBe(1);
  });

  it("the store transaction never takes a report past the cap, even if the check raced", async () => {
    await env.DB.prepare("INSERT INTO daily (day, accepted) VALUES (?, ?)").bind(today(), ACCEPTED_PER_DAY).run();
    const rep = validate(JSON.parse(report([result("parakeet-v3", 20.5, 0.061, 0.38)])));
    expect(await store(env.DB, today(), "c", rep)).toBeNull();
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM reports").first("n")).toBe(0);
    expect(await env.DB.prepare("SELECT accepted FROM daily").first("accepted")).toBe(ACCEPTED_PER_DAY);
    // A new day starts its own count.
    expect(await store(env.DB, daysAgo(-1), "c", rep)).toBeTypeOf("number");
  });

  it("deletes client rows older than yesterday", async () => {
    const seed = env.DB.prepare("INSERT INTO clients (day, client, posts) VALUES (?, ?, 3)");
    await env.DB.batch([seed.bind(daysAgo(2), "old"), seed.bind(daysAgo(30), "older"), seed.bind(daysAgo(1), "recent")]);
    await post("{}", "203.0.113.30");
    const rows = await env.DB.prepare("SELECT day, client FROM clients ORDER BY day").all<{ day: string; client: string }>();
    expect(rows.results.map((r) => r.client).filter((c) => c.length < 64)).toEqual(["recent"]);
    expect(rows.results.every((r) => r.day >= daysAgo(1))).toBe(true);
  });

  it("stores no IP address, user agent or header anywhere", async () => {
    const ip = "203.0.113.77";
    const ipv6 = "2001:db8::7a:77";
    const ua = "SilasProbe/9.9 (unique-ua-marker)";
    const headers = { "User-Agent": ua, "X-Forwarded-For": ip, Referer: "https://referer-marker.example/" };
    await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]), ip, headers);
    await post("not json", ip, headers);
    await post(report([result("parakeet-v3", 21, 0.06, 0.38)], { os_version: "macOS 27.3" }), ipv6, headers);
    const dump = await dumpAllTables();
    expect(dump).toContain("apple-m5"); // the dump does see the stored reports
    for (const needle of [ip, "203.0.113", ipv6, "2001:db8", ua, "unique-ua-marker", "SilasProbe", "referer-marker"]) {
      expect(dump).not.toContain(needle);
    }
  });
});

describe("scheduled", () => {
  it("deletes client rows and clears report senders older than yesterday with no POST", async () => {
    // Review first-F3 on #64: rows were deleted only inside a POST, so a quiet registry kept them
    // past the 48 hours the README promises.
    const seed = env.DB.prepare("INSERT INTO clients (day, client, posts) VALUES (?, ?, 3)");
    await env.DB.batch([seed.bind(daysAgo(2), "old"), seed.bind(daysAgo(30), "older"), seed.bind(daysAgo(1), "recent"), seed.bind(today(), "now")]);
    const rep = validate(JSON.parse(report([result("parakeet-v3", 20.5, 0.061, 0.38)])));
    for (const [day, client] of [[daysAgo(2), "old"], [daysAgo(1), "recent"], [today(), "now"]]) {
      expect(await store(env.DB, day, client, rep)).toBeTypeOf("number");
    }
    const ctx = createExecutionContext();
    await handler.scheduled(createScheduledController({ scheduledTime: Date.now(), cron: "5 0 * * *" }), env, ctx);
    await waitOnExecutionContext(ctx);
    const clients = await env.DB.prepare("SELECT client FROM clients ORDER BY day").all<{ client: string }>();
    expect(clients.results.map((r) => r.client)).toEqual(["recent", "now"]);
    const senders = await env.DB.prepare("SELECT client FROM reports ORDER BY id").all<{ client: string | null }>();
    expect(senders.results.map((r) => r.client)).toEqual([null, "recent", "now"]);
  });
});

describe("GET /v1/devices", () => {
  it("medians, pick and agree over one key's reports, ?key= filtering, and a key with one report", async () => {
    const posts = [
      // Picks parakeet-v3 on its own.
      report([result("parakeet-v3", 20, 0.06, 0.3), result("cohere-int4", 4, 0.05, 1.5)]),
      report([result("parakeet-v3", 30, 0.07, 0.2)]),
      // Nothing passes here; cohere-int4 is the only accurate one, so this report picks it.
      report([result("parakeet-v3", 25, 0.09, 0.4), result("cohere-int4", 6, 0.04, 1.0)]),
    ];
    let last: unknown;
    for (const [i, body] of posts.entries()) {
      const res = await post(body, `198.51.100.${40 + i}`);
      expect(res.status).toBe(201);
      last = await res.json();
    }
    expect(last).toEqual({ ok: true, key: "macos/apple-m5/10", reports: 3, pick: "parakeet-v3", agree: 2 });
    const other = report([result("parakeet-v3-onnx", 12, 0.07, 0.5)], { os: "linux", cpu: "AMD Ryzen 7 5800X 8-Core Processor", key: "linux/amd-ryzen-7-5800x-8-core-processor/16", cores: 16, os_version: "Ubuntu 24.04", hw_model: undefined });
    expect((await post(other, "198.51.100.50")).status).toBe(201);

    const res = await call("/v1/devices");
    expect(res.status).toBe(200);
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=300");
    expect(res.headers.get("Access-Control-Allow-Origin")).toBe("*");
    expect(res.headers.get("Content-Type")).toContain("application/json");
    const mac = {
      reports: 3,
      pick: "parakeet-v3",
      agree: 2,
      models: {
        "cohere-int4": { reports: 2, speed: (4 + 6) / 2, wer: (0.05 + 0.04) / 2, wait: (1.5 + 1.0) / 2 },
        "parakeet-v3": { reports: 3, speed: 25, wer: 0.07, wait: 0.3 },
      },
    };
    const linux = { reports: 1, pick: "parakeet-v3-onnx", agree: 1, models: { "parakeet-v3-onnx": { reports: 1, speed: 12, wer: 0.07, wait: 0.5 } } };
    expect(await res.json()).toEqual({
      schema: 1,
      suite: SUITE_ID,
      min_agree: 3,
      devices: { "macos/apple-m5/10": mac, "linux/amd-ryzen-7-5800x-8-core-processor/16": linux },
    });

    const one = await call(`/v1/devices?key=${encodeURIComponent("linux/amd-ryzen-7-5800x-8-core-processor/16")}`);
    expect(await one.json()).toEqual({ schema: 1, suite: SUITE_ID, min_agree: 3, devices: { "linux/amd-ryzen-7-5800x-8-core-processor/16": linux } });
    const none = await call("/v1/devices?key=windows/nothing/4");
    expect(await none.json()).toEqual({ schema: 1, suite: SUITE_ID, min_agree: 3, devices: {} });
  });

  it("counts only reports on the current benchmark set", async () => {
    await env.DB.prepare("INSERT INTO devices (suite, device_key, summary) VALUES (?, ?, ?)")
      .bind("librispeech-test-clean-9clips-v1", "macos/apple-m5/10", JSON.stringify({ reports: 9, pick: "parakeet-v3", agree: 9, models: {} }))
      .run();
    await env.DB.prepare("INSERT INTO reports (day, suite, device_key, report) VALUES (?, ?, ?, ?)")
      .bind(today(), "librispeech-test-clean-9clips-v1", "macos/apple-m5/10", JSON.stringify({ ...BASE, suite: "librispeech-test-clean-9clips-v1" }))
      .run();
    expect(await (await call("/v1/devices")).json()).toMatchObject({ devices: {} });
    const res = await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]));
    expect(await res.json()).toMatchObject({ reports: 1 });
    expect(await (await call("/v1/devices")).json()).toMatchObject({ devices: { "macos/apple-m5/10": { reports: 1 } } });
  });

  it("one sender's many reports count once toward agree", async () => {
    // Review first-F1 on #64: agree counted reports, so one network posting MIN_AGREE fabricated
    // reports settled a device on its own.
    const body = report([result("cohere-int4", 12, 0.03, 0.5)]);
    for (let i = 0; i < MIN_AGREE + 2; i++) {
      const res = await post(body, "203.0.113.60");
      expect(res.status).toBe(201);
      expect(await res.json()).toEqual({ ok: true, key: "macos/apple-m5/10", reports: 1, pick: "cohere-int4", agree: 1 });
    }
    const devices = async () => ((await (await call("/v1/devices")).json()) as { devices: Record<string, { reports: number; agree: number }> }).devices;
    expect((await devices())["macos/apple-m5/10"]).toMatchObject({ reports: 1, agree: 1 });
    // Every report stays in the open data.
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM reports").first("n")).toBe(MIN_AGREE + 2);
    // A second network is a second sender.
    expect(await (await post(body, "203.0.113.61")).json()).toMatchObject({ reports: 2, agree: 2 });
    // The same network on another day is another sender too: its HMAC rotates.
    await env.DB.prepare("INSERT INTO reports (day, suite, device_key, report) VALUES (?, ?, ?, ?)")
      .bind(daysAgo(3), SUITE_ID, "macos/apple-m5/10", body)
      .run();
    expect(await (await post(body, "203.0.113.61")).json()).toMatchObject({ reports: 3, agree: 3 });
  });

  it("forgets which network sent a report once its day is past", async () => {
    await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]), "203.0.113.62");
    await env.DB.prepare("UPDATE reports SET day = ?").bind(daysAgo(2)).run();
    await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]), "203.0.113.63");
    const rows = await env.DB.prepare("SELECT day, client FROM reports ORDER BY id").all<{ day: string; client: string | null }>();
    expect(rows.results.map((r) => [r.day, r.client === null])).toEqual([[daysAgo(2), true], [today(), false]]);
  });

  it("skips a stored report whose model Silas no longer offers, as ingest.py's stored() does", async () => {
    const retired = { ...JSON.parse(report([result("parakeet-v3", 20.5, 0.061, 0.38)])), results: [result("retired-model", 50, 0.01, 0.1)] };
    await env.DB.prepare("INSERT INTO reports (day, suite, device_key, report) VALUES (?, ?, ?, ?)")
      .bind(today(), SUITE_ID, "macos/apple-m5/10", JSON.stringify(retired))
      .run();
    const res = await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]));
    expect(await res.json()).toEqual({ ok: true, key: "macos/apple-m5/10", reports: 1, pick: "parakeet-v3", agree: 1 });
  });
});

describe("GET /v1/reports.jsonl", () => {
  it("exports every stored report as NDJSON, paged by id", async () => {
    expect((await post(report([result("parakeet-v3", 20.5, 0.061, 0.38)]))).status).toBe(201);
    const first = await call("/v1/reports.jsonl");
    expect(first.headers.get("Content-Type")).toContain("application/x-ndjson");
    expect(first.headers.get("Access-Control-Allow-Origin")).toBe("*");
    expect(first.headers.get("X-Next-After")).toBeNull();
    const lines = (await ndjson(first)).trimEnd().split("\n");
    expect(lines).toHaveLength(1);
    const row = JSON.parse(lines[0]);
    expect(Object.keys(row).slice(0, 3)).toEqual(["id", "day", "schema"]);
    expect(row).toEqual({ id: 1, day: today(), ...validate(JSON.parse(report([result("parakeet-v3", 20.5, 0.061, 0.38)]))) });

    // Fill past one page straight in SQL.
    const extra = EXPORT_PAGE + 1;
    await env.DB.prepare(
      "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1) " +
        "INSERT INTO reports (day, suite, device_key, report) SELECT ?2, ?3, 'k', '{\"schema\":1}' FROM n",
    )
      .bind(extra, today(), SUITE_ID)
      .run();
    const page1 = await call("/v1/reports.jsonl");
    const ids1 = (await ndjson(page1)).trimEnd().split("\n").map((l) => JSON.parse(l).id as number);
    expect(ids1).toHaveLength(EXPORT_PAGE);
    expect(ids1[0]).toBe(1);
    expect(page1.headers.get("X-Next-After")).toBe(String(ids1.at(-1)));
    const page2 = await call(`/v1/reports.jsonl?after=${page1.headers.get("X-Next-After")}`);
    const ids2 = (await ndjson(page2)).trimEnd().split("\n").map((l) => JSON.parse(l).id as number);
    expect(ids2).toEqual([EXPORT_PAGE + 1, EXPORT_PAGE + 2]);
    expect(page2.headers.get("X-Next-After")).toBeNull();
    const empty = await call(`/v1/reports.jsonl?after=${EXPORT_PAGE + 2}`);
    expect(await ndjson(empty)).toBe("");
    expect((await call("/v1/reports.jsonl?after=-1")).status).toBe(400);
    expect((await call("/v1/reports.jsonl?after=abc")).status).toBe(400);
  });
});
