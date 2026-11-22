// The request body a shared vector describes: the same rules as apply() in
// registry/test_ingest.py. registry/vectors.json is imported, never copied.

import vectors from "../../vectors.json";

export interface Case {
  name: string;
  set?: Record<string, unknown>;
  remove?: string[];
  replace?: [string, string];
  body?: string;
}

export const BASE = vectors.base;
export const VECTORS = vectors as unknown as { base: typeof BASE } & Record<"good" | "bad" | "stale", Case[]>;

type Node = Record<string, unknown> | unknown[];

function parent(root: unknown, parts: string[]): Node {
  let node = root as Node;
  for (const k of parts) node = (Array.isArray(node) ? node[Number(k)] : node[k]) as Node;
  return node;
}

export function apply(c: Case): string {
  if (c.body !== undefined) return c.body;
  const rep = structuredClone(BASE) as unknown;
  for (const [path, value] of Object.entries(c.set ?? {})) {
    const parts = path.split(".");
    const last = parts.pop()!;
    const node = parent(rep, parts);
    // Python's node[i:i + 1] = [value]: replaces index i, or appends when i is the length.
    if (Array.isArray(node)) node.splice(Number(last), 1, value);
    else node[last] = value;
  }
  for (const path of c.remove ?? []) {
    const parts = path.split(".");
    const last = parts.pop()!;
    const node = parent(rep, parts);
    if (Array.isArray(node)) node.splice(Number(last), 1);
    else delete node[last];
  }
  let text = JSON.stringify(rep, null, 2);
  if (c.replace) {
    const [from, to] = c.replace;
    const at = text.indexOf(from);
    if (at < 0) throw new Error(`replace target missing in ${c.name}`);
    text = text.slice(0, at) + to + text.slice(at + from.length);
  }
  return text;
}
