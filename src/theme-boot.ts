// Theme startup, with no window or document access so `bun test` can run it.

import type { Theme } from "./api";

export interface BootIO {
  /** Register for theme_changed. Resolves once the listener is live. */
  listen(cb: (t: Theme | null) => void): Promise<unknown>;
  /** Ask the core for the current theme. */
  snapshot(): Promise<Theme | null>;
  apply(t: Theme | null): void;
}

/**
 * Listen first, then ask for the snapshot. An event that arrives while the snapshot is in flight
 * is newer than it, so the snapshot is dropped. After the snapshot, events apply as they come.
 */
export async function startTheme(io: BootIO): Promise<void> {
  let eventSeen = false;
  await io.listen((t) => {
    eventSeen = true;
    io.apply(t);
  });
  let first: Theme | null = null;
  try {
    first = await io.snapshot();
  } catch {
    first = null;
  }
  if (!eventSeen) io.apply(first);
}

/** The part of localStorage the cache uses. */
export interface KeyValue {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem(k: string): void;
}

export const CACHE_KEY = "silas.theme";

/**
 * The last theme the core sent, kept so the next start can paint with it before the core answers.
 * A dark theme on a light system then does not flash. The core's answer replaces it.
 */
export function readCached(store: KeyValue): Theme | null {
  try {
    const raw = store.getItem(CACHE_KEY);
    if (!raw) return null;
    const t = JSON.parse(raw);
    return t && typeof t === "object" && t.light && t.dark && t.shape && t.type && t.motion ? (t as Theme) : null;
  } catch {
    return null;
  }
}

export function writeCached(store: KeyValue, t: Theme | null): void {
  try {
    if (t) store.setItem(CACHE_KEY, JSON.stringify(t));
    else store.removeItem(CACHE_KEY);
  } catch {
    /* storage may be full or off: the cache is optional */
  }
}
