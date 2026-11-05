import { describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { checkCss, declarations } from "../scripts/check-theme.mjs";

const script = resolve(import.meta.dir, "../scripts/check-theme.mjs");
const bad = (css: string) => checkCss("t.css", css);

describe("check:theme fails on", () => {
  const cases: [string, string][] = [
    ["hex", ".a { background: #fff; }"],
    ["rgb()", ".a { color: rgb(1, 2, 3); }"],
    ["literal border-radius", ".a { border-radius: 8px; }"],
    ["literal corner radius", ".a { border-top-left-radius: 0.5rem; }"],
    ["literal radius next to a variable", ".a { border-radius: var(--r) 4px 0 0; }"],
    ["literal font-family", '.a { font-family: "Inter", sans-serif; }'],
    ["literal transition duration", ".a { transition: opacity 0.2s ease; }"],
    ["literal transition-duration", ".a { transition-duration: 150ms; }"],
    ["named colour", ".a { color: white; }"],
    ["named colour in a border", ".a { border: 1px solid red; }"],
    ["named colour in a shadow", ".a { box-shadow: 0 0 4px Black; }"],
    ["a colour in a variable outside :root", ".a { --bg: #123456; }"],
    ["a named colour in a variable outside :root", ".a { --bg: white; }"],
    ["a colour in a variable in a media rule", "@media (min-width: 1px) { .a { --x: rgb(0 0 0); } }"],
    ["a colour after a string with a paren", '.a::before { content: ")"; } .b { color: #fff; }'],
    ["a colour after a string with a brace and a semicolon", '.a::before { content: "};{"; } .b { color: red; }'],
    ["a colour after a comment marker in a string", '.a::before { content: "/*"; } .b { color: #fff; }'],
  ];
  for (const [name, css] of cases) {
    test(name, () => expect(bad(css).length).toBeGreaterThan(0));
  }
});

describe("check:theme accepts", () => {
  const cases: [string, string][] = [
    ["defaults in :root", ":root { --bg: #fff; --c: white; --d: rgba(0,0,0,.5); }"],
    ["defaults in :root inside a media rule", "@media (prefers-color-scheme: dark) { :root { --bg: #000; } }"],
    ["variables everywhere else", ".a { color: var(--text); border-radius: var(--r-8); font-family: var(--sans); transition: opacity var(--motion-fast) var(--ease); }"],
    ["a mix of variables", ".a { background: color-mix(in srgb, var(--gold) 12%, transparent); }"],
    ["a variable assigned from another in a rule", ".a { --x: var(--gold); }"],
    ["a circle, a zero radius and a zero or 1ms transition", ".a { border-radius: 50%; } .b { border-radius: 0; transition-duration: 1ms; } .c { transition: none; }"],
    ["transparent and currentColor", ".a { color: currentColor; background: transparent; border-color: inherit; }"],
    ["a font-face family", '@font-face { font-family: "X"; src: url(x.woff2); }'],
    ["words that look like colours in strings", '.a::before { content: "white red #fff"; }'],
    ["a looping animation time", ".a { animation: spin 0.8s linear infinite; }"],
  ];
  for (const [name, css] of cases) {
    test(name, () => expect(bad(css)).toEqual([]));
  }
});

describe("declarations", () => {
  test("a paren, brace or semicolon inside a string does not end the statement", () => {
    const d = declarations('.a::before { content: ")"; color: red; } .b { content: "{;"; margin: 0; }');
    expect(d.map((x: { prop: string }) => x.prop)).toEqual(["content", "color", "content", "margin"]);
    expect(d[1].selector).toBe(".a::before");
    expect(d[3].selector).toBe(".b");
  });
  test("lines are kept", () => {
    expect(declarations("\n\n.a {\n  color: red;\n}")[0].line).toBe(4);
  });
});

describe("check:theme as a program", () => {
  test("runs when started through a symlink", () => {
    const dir = mkdtempSync(join(tmpdir(), "silas-link-"));
    try {
      const link = join(dir, "check.mjs");
      symlinkSync(script, link);
      const out = execFileSync("node", [link]).toString();
      expect(out).toContain("check:theme ok");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
  test("passes on the real sources", () => {
    expect(execFileSync("node", [script]).toString()).toContain("check:theme ok");
  });
});
