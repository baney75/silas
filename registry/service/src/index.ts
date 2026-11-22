// Silas's anonymous device registry: a Worker over D1.
//
//   POST /v1/reports        one benchmark report from Silas, validated as registry/ingest.py does
//   GET  /v1/devices        per-device medians and the pick, read from a precomputed table
//   GET  /v1/reports.jsonl  every stored report as open data (CC0), paged with ?after=<id>
//
// Privacy: nothing here stores or logs an IP address, a user agent or any other header. The
// rate-limit table holds only an HMAC of the IP under a key that changes every UTC day, and its
// rows are deleted once they are older than yesterday. Reports hold only validated fields.
//
// Only the handler is exported: workerd refuses a main module with any other named export.

import { handler } from "./service";

export default handler;
