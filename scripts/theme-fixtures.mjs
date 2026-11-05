#!/usr/bin/env node
// Writes the theme files the browser preview loads: <dir>/<theme>-<mode>.json for each theme and
// mode, made by the real `polycarp theme set` in a temporary config folder. Never touches ~/.config.
//
// Run: node scripts/theme-fixtures.mjs <out-dir> [theme...]     (default: still-water blocks calm)
// Then: SILAS_THEME_DIR=<out-dir> bun run dev, and open /?themeFile=/__theme/still-water-dark.json
// Set POLYCARP_BIN to use a built `polycarp` instead of `cargo run`.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const [out, ...names] = process.argv.slice(2);
if (!out) {
  console.error("usage: theme-fixtures.mjs <out-dir> [theme...]");
  process.exit(2);
}
const themes = names.length ? names : ["still-water", "blocks", "calm"];
const cli = resolve(fileURLToPath(new URL("../../cli/Cargo.toml", import.meta.url)));
const bin = process.env.POLYCARP_BIN;
mkdirSync(out, { recursive: true });
const config = mkdtempSync(join(tmpdir(), "silas-theme-"));
const env = { ...process.env, POLYCARP_CONFIG_DIR: config };

for (const theme of themes) {
  for (const mode of ["light", "dark"]) {
    const args = ["theme", "set", theme, "--mode", mode];
    if (bin) execFileSync(bin, args, { env, stdio: "ignore" });
    else execFileSync("cargo", ["run", "-q", "--manifest-path", cli, "--", ...args], { env, stdio: "ignore" });
    copyFileSync(join(config, "current", "theme.json"), join(out, `${theme}-${mode}.json`));
    console.log(`wrote ${theme}-${mode}.json`);
  }
}
