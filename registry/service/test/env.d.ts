import type { D1Migration } from "cloudflare:test";

export interface ParityCase {
  name: string;
  body: string;
  /** What ingest.py's validate() made of the body: the cleaned report, or the reason and Stale. */
  ok: boolean;
  report?: unknown;
  error?: string;
  stale?: boolean;
}

declare global {
  namespace Cloudflare {
    interface GlobalProps {
      mainModule: typeof import("../src/index");
    }
    interface Env {
      DB: D1Database;
      RATE_KEY?: string;
      TEST_MIGRATIONS: D1Migration[];
      /** registry/ingest.py as text, for the bars test. */
      INGEST_PY: string;
      /** Outcomes from registry/ingest.py for the shared vectors and mutations of base. */
      PARITY: { vectors: Record<"good" | "bad" | "stale", ParityCase[]>; mutations: ParityCase[] };
    }
  }
}
