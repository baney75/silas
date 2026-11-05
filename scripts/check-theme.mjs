#!/usr/bin/env node
// Fails when a hard-coded colour sits in product UI instead of coming from the theme.
//
// CSS: in a declaration, these are errors: a colour (hex, rgb(), hsl() and friends, or a name such as
// white); a literal border-radius, font-family or transition/animation duration; and a colour given to
// a custom property (--name: ...) in any rule except :root. Looping animations (spin, pulse) are not
// theme motion and keep their own times. The :root blocks hold the defaults that
// equal today's look; everything else must read them with var().
// TS and HTML: the same colours in a string are an error unless the file is allowlisted below.
//
// Run: bun run check:theme

import { readdirSync, readFileSync, realpathSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..");

// The written allowlist. Path (from voice/) -> why colours are allowed there.
export const ALLOW = {
  "src/ui.ts": "The app logo is a drawing with its own fixed colours, not UI chrome.",
  "src/theme-vars.ts": "Builds the theme's variables. Shadow inks are black or white by design.",
  "src/mock.ts": "Browser preview only: loads a theme file and fakes data. Never reaches the app.",
};

const COLOR = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|oklch|oklab|lab|lch)\(/;

// CSS named colors. `transparent`, `currentcolor` and the CSS-wide keywords are fine anywhere.
const NAMED = new Set(
  (
    "aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown " +
    "burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan " +
    "darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid " +
    "darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet " +
    "deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro " +
    "ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki " +
    "lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow " +
    "lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray " +
    "lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine " +
    "mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise " +
    "mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab " +
    "orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru " +
    "pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown " +
    "seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan " +
    "teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen"
  ).split(" "),
);

// Properties whose value can carry a color.
const COLOR_PROPS = /color|^background|^border|^outline|^fill$|^stroke$|shadow|^text-decoration|^column-rule|^filter$/;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

/** Blank comments, keeping line breaks and leaving strings alone (`"/*"` is text, not a comment). */
function blankComments(css) {
  let out = "";
  for (let i = 0; i < css.length; i++) {
    const ch = css[i];
    if (ch === '"' || ch === "'") {
      let j = i + 1;
      while (j < css.length && css[j] !== ch && css[j] !== "\n") j += css[j] === "\\" ? 2 : 1;
      out += css.slice(i, j + 1);
      i = j;
    } else if (ch === "/" && css[i + 1] === "*") {
      const end = css.indexOf("*/", i + 2);
      const stop = end < 0 ? css.length : end + 2;
      out += css.slice(i, stop).replace(/[^\n]/g, " ");
      i = stop - 1;
    } else out += ch;
  }
  return out;
}

/**
 * Declarations in a stylesheet, as { line, prop, value, selector }. `selector` is the innermost
 * block's prelude ("" at top level). Comments are blanked and lines kept. Quotes hide `() {} ;`.
 */
export function declarations(css) {
  const text = blankComments(css);
  const found = [];
  const stack = [];
  let line = 1;
  let start = 0; // where the current statement began
  let startLine = 1;
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === '"' || ch === "'") {
      i++;
      while (i < text.length && text[i] !== ch && text[i] !== "\n") i += text[i] === "\\" ? 2 : 1;
      continue;
    }
    if (ch === "\n") line++;
    if (ch === "(") depth++;
    else if (ch === ")") depth = Math.max(0, depth - 1);
    else if (depth === 0 && (ch === "{" || ch === "}" || ch === ";")) {
      const stmt = text.slice(start, i);
      if (ch === "{") stack.push(stmt.trim().replace(/\s+/g, " "));
      else {
        const m = /^\s*([-\w]+)\s*:([\s\S]*)$/.exec(stmt);
        if (m) {
          const lead = stmt.match(/^\s*/)[0].match(/\n/g)?.length ?? 0;
          found.push({ line: startLine + lead, prop: m[1], value: m[2], selector: stack[stack.length - 1] ?? "" });
        }
        if (ch === "}") stack.pop();
      }
      start = i + 1;
      startLine = line;
    }
  }
  return found;
}

/** A value with its strings and `var(...)` calls removed, so only literals are left. */
const noStrings = (value) => value.replace(/"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'/g, " ");

function literals(value) {
  let v = noStrings(value);
  for (;;) {
    const i = v.indexOf("var(");
    if (i < 0) return v;
    let d = 0;
    let j = i + 3;
    for (; j < v.length; j++) {
      if (v[j] === "(") d++;
      else if (v[j] === ")" && --d === 0) break;
    }
    v = v.slice(0, i) + " " + v.slice(j + 1);
  }
}

const hasNamedColor = (value) => (literals(value).toLowerCase().match(/[a-z-]+/g) ?? []).some((w) => NAMED.has(w));
const isRoot = (selector) => selector.split(",").every((p) => p.trim() === ":root");

/** Problems in one stylesheet's text. `where` names it in messages. */
export function checkCss(where, css) {
  const problems = [];
  for (const { line, prop, value, selector } of declarations(css)) {
    const at = `${where}:${line}: ${prop}:${value.trim()}`;
    const lit = literals(value);
    if (prop.startsWith("--")) {
      // A default colour belongs in :root; anywhere else it would shadow the theme.
      if (!isRoot(selector) && (COLOR.test(lit) || hasNamedColor(value))) {
        problems.push(`${at}  (colour assigned to a variable outside :root)`);
      }
      continue;
    }
    if (COLOR.test(noStrings(value))) problems.push(at);
    else if (COLOR_PROPS.test(prop) && hasNamedColor(value)) problems.push(`${at}  (named colour)`);
    if (/^(?:border(?:-\w+)*-radius)$/.test(prop) && /[1-9]|\.\d*[1-9]/.test(lit) && lit.trim() !== "50%") {
      problems.push(`${at}  (literal radius, use a --r variable; only 50% is allowed, for circles)`);
    }
    if (prop === "font-family" && !selector.startsWith("@font-face") && !/^\s*(?:inherit|initial|unset|revert)\s*$/.test(lit) && !/var\(/.test(value)) {
      problems.push(`${at}  (literal font-family, use var(--sans), var(--serif) or a theme font)`);
    }
    if (/^transition(?:-duration|-delay)?$/.test(prop) && /(?:^|[^\w.-])(?:\d*\.\d+|(?!1ms\b)[1-9]\d*)m?s\b/.test(lit)) {
      problems.push(`${at}  (literal transition time, use var(--motion-fast|normal|slow); 0 and 1ms are allowed)`);
    }
  }
  return problems;
}

/** Problems in a source file's text; `rel` is its path from voice/. */
export function checkSource(relPath, text) {
  const rel = relPath.replaceAll("\\", "/"); // Windows paths use backslashes; ALLOW keys use /
  if (rel.endsWith(".css")) return checkCss(rel, text);
  if (ALLOW[rel]) return [];
  const problems = [];
  text.split("\n").forEach((l, i) => {
    if (COLOR.test(l)) problems.push(`${rel}:${i + 1}: ${l.trim()}`);
  });
  return problems;
}

export function check(files) {
  return files.flatMap((file) => checkSource(relative(root, file), readFileSync(file, "utf8")));
}

/** True when this file is the one node was started with, even through a symlink. */
export function isMain(argv1, metaUrl) {
  try {
    return !!argv1 && realpathSync(argv1) === realpathSync(fileURLToPath(metaUrl));
  } catch {
    return false;
  }
}

if (isMain(process.argv[1], import.meta.url)) {
  const files = [
    ...walk(join(root, "src")).filter((f) => /\.(css|ts)$/.test(f)),
    join(root, "index.html"),
    join(root, "overlay.html"),
  ];
  const problems = check(files);
  if (problems.length) {
    console.error("Hard-coded theme values (use a theme variable, or add the file to ALLOW with a reason):");
    for (const p of problems) console.error("  " + p);
    process.exit(1);
  }
  console.log(`check:theme ok: ${files.length} files, ${Object.keys(ALLOW).length} allowlisted`);
}
