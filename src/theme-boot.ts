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
  try {
    await io.listen((t) => {
      eventSeen = true;
      io.apply(t);
    });
  } catch {
    // No live updates, but the snapshot below still applies and the app starts.
  }
  let first: Theme | null = null;
  try {
    first = await io.snapshot();
  } catch {
    first = null;
  }
  if (!eventSeen) io.apply(first);
}
