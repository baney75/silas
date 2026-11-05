#!/usr/bin/env node
// Fails when a hard-coded colour sits in product UI instead of coming from the theme.
//
// CSS: a hex, rgb(), rgba(), hsl() or hsla() value in a declaration is an error unless the
// declaration defines a custom property (--name: ...). The :root blocks hold the defaults that
// equal today's look; everything else must read them with var().
// TS and HTML: the same colours in a string are an error unless the file is allowlisted below.
//
// Run: bun run check:theme

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..");

// The written allowlist. Path (from voice/) -> why colours are allowed there.
export const ALLOW = {
  "src/ui.ts": "The app logo is a drawing with its own fixed colours, not UI chrome.",
  "src/theme.ts": "Builds the theme's variables. Shadow inks are black or white by design.",
  "src/mock.ts": "Browser preview only: loads a theme file and fakes data. Never reaches the app.",
};

const COLOR = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|oklch|oklab|lab|lch)\(/;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

/** Declarations in a stylesheet, as [line, property, value]. Comments are blanked, lines kept. */
export function declarations(css) {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, " "));
  const found = [];
  let line = 1;
  let start = 0; // where the current statement began
  let startLine = 1;
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === "\n") line++;
    if (ch === "(") depth++;
    else if (ch === ")") depth--;
    else if (depth === 0 && (ch === "{" || ch === "}" || ch === ";")) {
      const stmt = text.slice(start, i);
      if (ch !== "{") {
        const m = /^\s*([-\w]+)\s*:([\s\S]*)$/.exec(stmt);
        if (m) found.push([startLine + (stmt.match(/^\s*/)[0].match(/\n/g)?.length ?? 0), m[1], m[2]]);
      }
      start = i + 1;
      startLine = line;
    }
  }
  return found;
}

export function check(files) {
  const problems = [];
  for (const file of files) {
    const rel = relative(root, file);
    const text = readFileSync(file, "utf8");
    if (file.endsWith(".css")) {
      for (const [line, prop, value] of declarations(text)) {
        if (prop.startsWith("--")) continue;
        if (COLOR.test(value)) problems.push(`${rel}:${line}: ${prop}:${value.trim()}`);
      }
    } else if (!ALLOW[rel]) {
      text.split("\n").forEach((l, i) => {
        if (COLOR.test(l)) problems.push(`${rel}:${i + 1}: ${l.trim()}`);
      });
    }
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = [
    ...walk(join(root, "src")).filter((f) => /\.(css|ts)$/.test(f)),
    join(root, "index.html"),
    join(root, "overlay.html"),
  ];
  const problems = check(files);
  if (problems.length) {
    console.error("Hard-coded colours (use a theme variable, or add the file to ALLOW with a reason):");
    for (const p of problems) console.error("  " + p);
    process.exit(1);
  }
  console.log(`check:theme ok: ${files.length} files, ${Object.keys(ALLOW).length} allowlisted`);
}
