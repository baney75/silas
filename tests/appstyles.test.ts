import { expect, test } from "bun:test";
import type { AppInfo } from "../src/api";
import { assignStyle, assignments, findSnippet, snippetPreview, styleFor, STYLES, triggerKey, unassign } from "../src/appstyles";

const ghostty: AppInfo = { id: "com.mitchellh.ghostty", name: "Ghostty", style: "terminal", builtin: "terminal" };
const notes: AppInfo = { id: "com.apple.Notes", name: "com.apple.Notes", style: "prose", builtin: "prose" };

test("every style has a label and a line of help", () => {
  expect(STYLES.map((s) => s.value).sort()).toEqual(["chat", "code", "prose", "terminal"]);
  for (const s of STYLES) expect(s.help.length).toBeGreaterThan(10);
});

test("an assignment wins over Silas's own style, matched without case", () => {
  expect(styleFor({}, ghostty)).toBe("terminal");
  expect(styleFor({ "COM.MITCHELLH.GHOSTTY": "prose" }, ghostty)).toBe("prose");
  // A value the UI doesn't know falls back to the built-in style, as the core does.
  expect(styleFor({ "com.mitchellh.ghostty": "bogus" as never }, ghostty)).toBe("terminal");
});

test("choosing the built-in style removes the assignment", () => {
  let map = assignStyle({}, notes.id, "chat", notes.builtin);
  expect(map).toEqual({ "com.apple.Notes": "chat" });
  map = assignStyle(map, "COM.APPLE.NOTES", "code", notes.builtin);
  expect(map).toEqual({ "COM.APPLE.NOTES": "code" });
  map = assignStyle(map, notes.id, "prose", notes.builtin);
  expect(map).toEqual({});
  // With no built-in known (an app typed in by hand), any choice is kept.
  expect(assignStyle({}, " slack.exe ", "prose")).toEqual({ "slack.exe": "prose" });
  expect(assignStyle({ a: "code" }, "  ", "chat")).toEqual({ a: "code" });
});

test("assignments list sorted and remove without case", () => {
  const map = { zed: "code", "Alacritty": "terminal", "com.apple.Notes": "chat" } as const;
  expect(assignments(map).map(([id]) => id)).toEqual(["Alacritty", "com.apple.Notes", "zed"]);
  expect(unassign(map, "alacritty")).toEqual({ zed: "code", "com.apple.Notes": "chat" });
});

test("trigger keys match the core's match_key", () => {
  // Same cases as core/voice/src/snippets.rs tests.
  expect(triggerKey("  My  Address! ")).toBe("my address");
  expect(triggerKey("Don't")).toBe("dont");
  expect(triggerKey("e-mail")).toBe("e mail");
  expect(triggerKey("Sign-off!")).toBe("sign off");
  expect(triggerKey("...")).toBe("");
});

test("a snippet is found by its trigger in any spelling", () => {
  const list = [{ trigger: "my address", text: "12 Benson Rd", added: "" }];
  expect(findSnippet(list, "My Address.")?.text).toBe("12 Benson Rd");
  expect(findSnippet(list, "my email")).toBeUndefined();
  expect(findSnippet(list, "!!")).toBeUndefined();
});

test("previews show the first line and mark the rest", () => {
  expect(snippetPreview("Thanks,\nDonovan")).toBe("Thanks, ↵ …");
  expect(snippetPreview("short")).toBe("short");
  expect(snippetPreview("x".repeat(100), 10)).toBe(`${"x".repeat(9)}…`);
});
