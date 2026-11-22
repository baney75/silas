# Silas device registry service

An anonymous report service for Silas's benchmark, as a Cloudflare Worker with a D1 database.
Silas's "Share results" posts the benchmark report it built; the service checks it exactly as
`registry/ingest.py` does, stores the validated fields, and keeps a per-device summary that Silas
reads to suggest a model for hardware like yours.

The app talks to `https://silas-registry.magnus-b37.workers.dev`.

The rules come from files in `registry/`, imported at build time and never copied:
`suite.json` (the benchmark set), `models.json` (the models Silas offers) and, for the tests,
`vectors.json` (good, bad and stale reports every validator must agree on). The bars
(`MIN_SPEED` 10.0, `MAX_WER` 0.08, `TIE_WER` 0.005) live in `src/validate.ts`; a test fails if
they drift from `ingest.py`.

## Endpoints

### `POST /v1/reports`

The body is the report JSON Silas builds. Checks run in this order:

1. 20 POSTs per client per UTC day, counted whether accepted or not: `429` after that.
2. Body at most 20 KB (20480 bytes): `413`.
3. At most 5,000 accepted reports per UTC day across everyone: `429` with `Retry-After` set to
   the seconds until the next UTC midnight.
4. The report must pass `ingest.py`'s rules: `422 {"error": "<reason>", "stale": false}`, or
   `"stale": true` when it was made on an older benchmark set or names a model Silas no longer
   offers.
5. Accepted: `201 {"ok": true, "key": "<device key>", "reports": N, "pick": "<model>", "agree": K}`
   for that device after this report.

```sh
curl -X POST https://silas-registry.magnus-b37.workers.dev/v1/reports \
  -H 'Content-Type: application/json' --data-binary @report.json
```

If the Worker has no `RATE_KEY` secret, POST answers `503` and stores nothing.

### `GET /v1/devices`

Per-device medians on the current benchmark set, read from a table updated on each accepted
report. `?key=<device key>` returns just that device (or an empty `devices`).

```sh
curl 'https://silas-registry.magnus-b37.workers.dev/v1/devices?key=macos/apple-m5/10'
```

```json
{"schema":1,"suite":"silas-bench-mixed-14clips-v2","min_agree":3,"devices":{"macos/apple-m5/10":
  {"reports":3,"pick":"parakeet-v3","agree":2,
   "models":{"parakeet-v3":{"reports":3,"speed":25,"wer":0.07,"wait":0.3}}}}}
```

Only one report per sender per device per UTC day counts, the latest; a sender is the daily
HMAC of the network address described under Privacy. So `reports` and `agree` count independent
senders, and one network posting the same device many times is one voice. `speed`, `wer` and
`wait` are medians across those reports (the mean of the middle two when the count is even, like
Python's `statistics.median`). `pick` is Silas's pick rule over those medians; `agree` counts the
senders whose own report picks the same model. Silas
treats a device as settled once `agree` reaches `min_agree`. Cached for 5 minutes; CORS open.

### `GET /v1/reports.jsonl`

Every stored report as open data, one JSON object per line:
`{"id": 1, "day": "2026-11-22", "schema": 1, "suite": ..., "silas": ..., "device": {...}, "results": [...]}`.
Up to 5,000 lines per page; when more remain the response has an `X-Next-After` header, and
`?after=<id>` fetches the next page.

```sh
curl -D - 'https://silas-registry.magnus-b37.workers.dev/v1/reports.jsonl?after=0'
```

The export is dedicated to the public domain under
[CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).

Any other path is `404`; a known path with the wrong method is `405`.

## Privacy

- Stored: the validated report fields (Silas version, OS, OS version, CPU name, core count,
  memory, hardware model when Silas sends one, and the per-model results) and the UTC day it
  arrived. Anything else in the body is dropped before storage.
- Never stored or logged: IP addresses, user agents or any other header. The Worker logs nothing
  about requests.
- The rate-limit table holds only a daily-rotating HMAC of the IP: `HMAC-SHA256(dayKey, ip)`
  with `dayKey = HMAC-SHA256(RATE_KEY, "YYYY-MM-DD")`. It can't be turned back into an IP or
  matched across days, and rows are deleted once they are older than yesterday (on each POST
  and by a daily cron trigger at 00:05 UTC), so none lives past 48 hours. Each stored report carries the same HMAC, so a sender counts once a day, and
  it is cleared on the same schedule; only a flag saying whether the report still counts stays.

## Limits

| Limit | Value |
| --- | --- |
| Body size | 20 KB (20480 bytes) |
| POSTs per client | 20 per UTC day |
| Accepted reports, everyone | 5,000 per UTC day |

All on the Workers and D1 free plans: no Rate Limiting binding, no paid features, no routes. The
one cron trigger fits the free plan's allowance.

## Deploy (maintainer)

1. `cd registry/service && bun install`
2. `bunx wrangler d1 create silas-registry`, then paste the `database_id` it prints into
   `wrangler.jsonc` in place of the zero placeholder.
3. `bunx wrangler d1 migrations apply silas-registry --remote`
4. `openssl rand -hex 32 | bunx wrangler secret put RATE_KEY`
5. `bunx wrangler deploy`
6. Read it back: `curl https://silas-registry.magnus-b37.workers.dev/v1/devices`

## Develop

```sh
bun install --frozen-lockfile && bun run test   # tests in the Workers runtime, with D1
bun run typecheck
```

The tests also need `python3`: `vitest.config.ts` runs `test/parity.py`, which asks
`registry/ingest.py` for its answer to every shared vector and several hundred mutations of the
base report, and the Worker must give the same decision, reason and cleaned report for each.
Test files are named `*.vitest.ts` so the repo root's `bun test` leaves them alone.

To run it locally: `bunx wrangler d1 migrations apply silas-registry --local`, put
`RATE_KEY=<anything>` in `.dev.vars` (ignored by git), then `bun run dev`.

## Where it differs from `ingest.py`

- The body is raw JSON, not a GitHub issue, so there is no code-fence or brace extraction, and
  the size cap is 20480 bytes rather than 20,000 characters.
- Python 3.12's `json` gives up on nesting deeper than about 1000 levels; this service (and
  Python 3.14) reads any nesting that fits in 20 KB. Silas never nests that deep.
