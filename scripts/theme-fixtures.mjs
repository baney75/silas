#!/usr/bin/env node
// Theme files for tests and the browser preview. Silas reads the theme file Polycarp writes
// (`<config>/current/theme.json`); the built-in ones are kept in tests/fixtures/themes as
// <theme>-<mode>.json so Silas builds and tests without Polycarp.
//
// Preview: node scripts/theme-fixtures.mjs <out-dir> [theme...]   (default: still-water blocks calm)
//          copies the light and dark files of each theme into <out-dir>.
//          Then: SILAS_THEME_DIR=<out-dir> bun run dev, and open /?themeFile=/__theme/still-water-dark.json
//
// Refresh: node scripts/theme-fixtures.mjs --refresh
//          rewrites tests/fixtures/themes from an installed `polycarp` command (or POLYCARP_BIN),
//          run in a temporary config folder. Never touches ~/.config.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const fixtures = fileURLToPath(new URL("../tests/fixtures/themes", import.meta.url));
const [first, ...rest] = process.argv.slice(2);

if (first === "--refresh") {
  const bin = process.env.POLYCARP_BIN || "polycarp";
  const config = mkdtempSync(join(tmpdir(), "silas-theme-"));
  const env = { ...process.env, POLYCARP_CONFIG_DIR: config, XDG_CONFIG_HOME: "" };
  const run = (...args) => execFileSync(bin, args, { env }).toString();
  const list = JSON.parse(run("theme", "list", "--json")).data.themes;
  const names = list.filter((t) => t.source === "built-in").map((t) => t.name);
  mkdirSync(fixtures, { recursive: true });
  for (const f of readdirSync(fixtures)) if (f.endsWith(".json")) rmSync(join(fixtures, f));
  for (const theme of names) {
    for (const mode of ["auto", "light", "dark"]) {
      run("theme", "set", theme, "--mode", mode);
      copyFileSync(join(config, "current", "theme.json"), join(fixtures, `${theme}-${mode}.json`));
      console.log(`wrote ${theme}-${mode}.json`);
    }
  }
  rmSync(config, { recursive: true, force: true });
} else if (first && !first.startsWith("-")) {
  const themes = rest.length ? rest : ["still-water", "blocks", "calm"];
  mkdirSync(first, { recursive: true });
  for (const theme of themes) {
    for (const mode of ["light", "dark"]) {
      copyFileSync(join(fixtures, `${theme}-${mode}.json`), join(first, `${theme}-${mode}.json`));
      console.log(`wrote ${theme}-${mode}.json`);
    }
  }
} else {
  console.error("usage: theme-fixtures.mjs <out-dir> [theme...] | --refresh");
  process.exit(2);
}
