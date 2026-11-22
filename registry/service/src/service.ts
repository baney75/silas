// Everything the Worker does. src/index.ts exports only the handler: workerd rejects a main
// module with other named exports.

import { Reject, type Report, SUITE_ID, median, pick, readReport, validate } from "./validate";

export interface Env {
  DB: D1Database;
  /** Worker secret for the rate-limit HMAC. Without it POST fails closed with 503. */
  RATE_KEY?: string;
}

export const MAX_BODY = 20480;
export const CLIENT_POSTS_PER_DAY = 20;
export const ACCEPTED_PER_DAY = 5000;
export const MIN_AGREE = 3;
export const EXPORT_PAGE = 5000;

const JSON_TYPE = "application/json; charset=utf-8";

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": JSON_TYPE, "X-Content-Type-Options": "nosniff", ...headers },
  });
}

export function utcDay(t: Date): string {
  return t.toISOString().slice(0, 10);
}

function secondsToMidnight(t: Date): number {
  const next = Date.UTC(t.getUTCFullYear(), t.getUTCMonth(), t.getUTCDate() + 1);
  return Math.max(1, Math.ceil((next - t.getTime()) / 1000));
}

async function hmac(key: Uint8Array, message: string): Promise<Uint8Array> {
  const k = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return new Uint8Array(await crypto.subtle.sign("HMAC", k, new TextEncoder().encode(message)));
}

/** hex HMAC-SHA256(dayKey, ip), dayKey = HMAC-SHA256(RATE_KEY, day). Never reversible to the IP. */
export async function clientId(rateKey: string, day: string, ip: string): Promise<string> {
  const dayKey = await hmac(new TextEncoder().encode(rateKey), day);
  const mac = await hmac(dayKey, ip);
  return Array.from(mac, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** The body, or null once it passes `cap` bytes (reading stops there). */
async function readCapped(body: ReadableStream<Uint8Array> | null, cap: number): Promise<Uint8Array | null> {
  if (body === null) return new Uint8Array(0);
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > cap) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.byteLength;
  }
  return out;
}

export interface ModelSummary {
  reports: number;
  speed: number;
  wer: number;
  wait: number;
}

export interface DeviceSummary {
  reports: number;
  pick: string | null;
  agree: number;
  models: Record<string, ModelSummary>;
}

/**
 * Medians per model across a device's reports, the pick over them, and how many reports agree.
 * The caller passes one report per sender per day, so agree counts independent senders.
 */
export function summarize(reports: Report[]): DeviceSummary {
  const by = new Map<string, { speed: number[]; wer: number[]; wait: number[] }>();
  for (const rep of reports) {
    for (const r of rep.results) {
      let m = by.get(r.model);
      if (!m) by.set(r.model, (m = { speed: [], wer: [], wait: [] }));
      m.speed.push(r.speed);
      m.wer.push(r.wer);
      m.wait.push(r.wait_seconds);
    }
  }
  const models: Record<string, ModelSummary> = {};
  const meds = [];
  for (const model of [...by.keys()].sort()) {
    const v = by.get(model)!;
    const m = { reports: v.speed.length, speed: median(v.speed), wer: median(v.wer), wait: median(v.wait) };
    models[model] = m;
    meds.push({ model, speed: m.speed, wer: m.wer });
  }
  const best = pick(meds);
  const agree = best === null ? 0 : reports.filter((rep) => pick(rep.results) === best).length;
  return { reports: reports.length, pick: best, agree, models };
}

/**
 * Bump the day's accepted count and store the report, as one transaction. The count only moves
 * while it is under the cap, and the insert happens only if the count moved, so concurrent posts
 * can't push a day past ACCEPTED_PER_DAY. A sender's earlier reports for the same device that day
 * stop counting: one sender is one voice in the summary. Returns the new report id, or null at
 * the cap.
 */
export async function store(db: D1Database, day: string, client: string, report: Report): Promise<number | null> {
  const [, inserted] = await db.batch<{ id: number }>([
    db
      .prepare(
        "INSERT INTO daily (day, accepted) VALUES (?1, 1) " +
          "ON CONFLICT (day) DO UPDATE SET accepted = accepted + 1 WHERE accepted < ?2",
      )
      .bind(day, ACCEPTED_PER_DAY),
    db
      .prepare(
        "INSERT INTO reports (day, suite, device_key, report, client) SELECT ?1, ?2, ?3, ?4, ?5 WHERE changes() = 1 RETURNING id",
      )
      .bind(day, report.suite, report.device.key, JSON.stringify(report), client),
    db
      .prepare(
        "UPDATE reports SET counted = 0 WHERE day = ?1 AND client = ?2 AND suite = ?3 AND device_key = ?4 AND counted = 1 " +
          "AND id < (SELECT MAX(id) FROM reports WHERE day = ?1 AND client = ?2 AND suite = ?3 AND device_key = ?4)",
      )
      .bind(day, client, report.suite, report.device.key),
  ]);
  return inserted.results.length > 0 ? inserted.results[0].id : null;
}

/**
 * Recompute one device's aggregate from its counted reports on the current set: one per sender
 * per day. Each stored report is validated again, as ingest.py's stored() does, so one naming a
 * model Silas has since dropped is skipped. The write only lands if no report for the key arrived
 * after the read; that later post recomputes with it included, so the table never goes back to an
 * older aggregate.
 */
export async function recompute(db: D1Database, key: string): Promise<DeviceSummary> {
  const rows = await db
    .prepare("SELECT counted, report FROM reports WHERE suite = ?1 AND device_key = ?2 ORDER BY id")
    .bind(SUITE_ID, key)
    .all<{ counted: number; report: string }>();
  const reports: Report[] = [];
  for (const row of rows.results) {
    if (row.counted !== 1) continue;
    try {
      reports.push(validate(readReport(row.report)));
    } catch (e) {
      if (!(e instanceof Reject)) throw e;
    }
  }
  const summary = summarize(reports);
  await db
    .prepare(
      "INSERT INTO devices (suite, device_key, summary) SELECT ?1, ?2, ?3 " +
        "WHERE (SELECT COUNT(*) FROM reports WHERE suite = ?1 AND device_key = ?2) = ?4 " +
        "ON CONFLICT (suite, device_key) DO UPDATE SET summary = excluded.summary",
    )
    .bind(SUITE_ID, key, JSON.stringify(summary), rows.results.length)
    .run();
  return summary;
}

/**
 * Delete clients rows and clear report senders from before `yesterday`, so no network HMAC
 * lives past 48 hours. Runs on every POST and daily on the cron trigger, so a quiet day still
 * forgets.
 */
function prune(db: D1Database, yesterday: string): D1PreparedStatement[] {
  return [
    db.prepare("DELETE FROM clients WHERE day < ?1").bind(yesterday),
    db.prepare("UPDATE reports SET client = NULL WHERE day < ?1 AND client IS NOT NULL").bind(yesterday),
  ];
}

function yesterdayOf(t: Date): string {
  return utcDay(new Date(t.getTime() - 86_400_000));
}

async function submit(request: Request, env: Env): Promise<Response> {
  if (!env.RATE_KEY) return json({ error: "The registry isn't taking reports right now." }, 503);
  const now = new Date();
  const day = utcDay(now);
  const retry = { "Retry-After": String(secondsToMidnight(now)) };

  // 1. Per-client limit. Every POST counts, accepted or not.
  const client = await clientId(env.RATE_KEY, day, request.headers.get("CF-Connecting-IP") ?? "unknown");
  const [, , counted] = await env.DB.batch<{ posts: number }>([
    ...prune(env.DB, yesterdayOf(now)),
    env.DB
      .prepare(
        "INSERT INTO clients (day, client, posts) VALUES (?1, ?2, 1) " +
          "ON CONFLICT (day, client) DO UPDATE SET posts = posts + 1 RETURNING posts",
      )
      .bind(day, client),
  ]);
  if (counted.results[0].posts > CLIENT_POSTS_PER_DAY) {
    return json({ error: "Too many reports from this network today. Try again tomorrow." }, 429, retry);
  }

  // 2. Body cap, by the declared length and again while reading.
  const tooLarge = () => json({ error: "The report is too long." }, 413);
  const declared = Number(request.headers.get("Content-Length"));
  if (declared > MAX_BODY) return tooLarge();
  const bytes = await readCapped(request.body, MAX_BODY);
  if (bytes === null) return tooLarge();

  // 3. Global cap.
  const today = await env.DB.prepare("SELECT accepted FROM daily WHERE day = ?1").bind(day).first<number>("accepted");
  const full = () => json({ error: "The registry has taken all the reports it can today. Try again tomorrow." }, 429, retry);
  if (today !== null && today >= ACCEPTED_PER_DAY) return full();

  // 4. Parse and validate, with ingest.py's reasons.
  let text: string;
  try {
    // A byte-order mark stays in, so it fails like Python's json.loads.
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    return json({ error: "The report isn't valid JSON. Please submit it from Silas without editing it.", stale: false }, 422);
  }
  let report: Report;
  try {
    report = validate(readReport(text));
  } catch (e) {
    if (e instanceof Reject) return json({ error: e.message, stale: e.stale }, 422);
    // As in ingest.py: anything unexpected is still a rejection with a reason.
    return json({ error: "The report couldn't be read. Please submit it from Silas without editing it.", stale: false }, 422);
  }

  // 5. Store, count, and recompute this device's aggregate.
  const id = await store(env.DB, day, client, report);
  if (id === null) return full();
  const key = report.device.key;
  const summary = await recompute(env.DB, key);

  // 6. Only validated values go back.
  return json({ ok: true, key, reports: summary.reports, pick: summary.pick, agree: summary.agree }, 201);
}

const PUBLIC_GET = { "Access-Control-Allow-Origin": "*" };

async function devices(url: URL, env: Env): Promise<Response> {
  const key = url.searchParams.get("key");
  const rows =
    key === null
      ? await env.DB.prepare("SELECT device_key, summary FROM devices WHERE suite = ?1 ORDER BY device_key")
          .bind(SUITE_ID)
          .all<{ device_key: string; summary: string }>()
      : await env.DB.prepare("SELECT device_key, summary FROM devices WHERE suite = ?1 AND device_key = ?2")
          .bind(SUITE_ID, key)
          .all<{ device_key: string; summary: string }>();
  // Each summary is JSON this Worker wrote, so it goes out as stored.
  const entries = rows.results.map((r) => `${JSON.stringify(r.device_key)}:${r.summary}`);
  const head = JSON.stringify({ schema: 1, suite: SUITE_ID, min_agree: MIN_AGREE }).slice(0, -1);
  return new Response(`${head},"devices":{${entries.join(",")}}}`, {
    headers: {
      "Content-Type": JSON_TYPE,
      "Cache-Control": "public, max-age=300",
      "X-Content-Type-Options": "nosniff",
      ...PUBLIC_GET,
    },
  });
}

async function exportReports(url: URL, env: Env): Promise<Response> {
  const raw = url.searchParams.get("after") ?? "0";
  if (!/^[0-9]{1,15}$/.test(raw)) return json({ error: "after must be a report id." }, 400, PUBLIC_GET);
  const rows = await env.DB.prepare("SELECT id, day, report FROM reports WHERE id > ?1 ORDER BY id LIMIT ?2")
    .bind(Number(raw), EXPORT_PAGE + 1)
    .all<{ id: number; day: string; report: string }>();
  const page = rows.results.slice(0, EXPORT_PAGE);
  const lines = page.map((r) => JSON.stringify({ id: r.id, day: r.day, ...JSON.parse(r.report) }) + "\n");
  const headers: Record<string, string> = {
    "Content-Type": "application/x-ndjson; charset=utf-8",
    "X-Content-Type-Options": "nosniff",
    "Access-Control-Expose-Headers": "X-Next-After",
    ...PUBLIC_GET,
  };
  if (rows.results.length > EXPORT_PAGE) headers["X-Next-After"] = String(page[page.length - 1].id);
  return new Response(lines.join(""), { headers });
}

const ROUTES: Record<string, { method: string; handle: (req: Request, url: URL, env: Env) => Promise<Response> }> = {
  "/v1/reports": { method: "POST", handle: (req, _url, env) => submit(req, env) },
  "/v1/devices": { method: "GET", handle: (_req, url, env) => devices(url, env) },
  "/v1/reports.jsonl": { method: "GET", handle: (_req, url, env) => exportReports(url, env) },
};

export const handler = {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const route = Object.hasOwn(ROUTES, url.pathname) ? ROUTES[url.pathname] : undefined;
    if (!route) return json({ error: "Not found." }, 404);
    if (request.method !== route.method) return json({ error: "Method not allowed." }, 405, { Allow: route.method });
    try {
      return await route.handle(request, url, env);
    } catch {
      // Nothing about the request is logged, not even the error.
      return json({ error: "Something went wrong. Try again later." }, 500);
    }
  },

  async scheduled(controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    ctx.waitUntil(env.DB.batch(prune(env.DB, yesterdayOf(new Date(controller.scheduledTime)))));
  },
} satisfies ExportedHandler<Env>;
