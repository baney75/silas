// Pure helpers for app styles and snippets. No DOM and no IPC, so `bun test` covers them
// (tests/appstyles.test.ts). The core decides the real style (core/voice/src/style.rs); these
// mirror its rules so Settings shows the right choice before the next dictation.

import type { AppInfo, AppStyle, Snippet } from "./api";

export const STYLES: Array<{ value: AppStyle; label: string; help: string }> = [
  { value: "prose", label: "Prose", help: "Sentences as you say them." },
  { value: "chat", label: "Chat", help: "Prose, and new lines use Shift+Return so the message doesn't send early." },
  { value: "code", label: "Code", help: "Say symbols: “dash dash”, “dot”, “open paren”, “camel case user name”." },
  { value: "terminal", label: "Terminal", help: "Spoken code, no capital at the start and no period at the end." },
];

export function styleHelp(style: AppStyle): string {
  return STYLES.find((s) => s.value === style)?.help ?? "";
}

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** The style `app` gets with the user's assignments: theirs first, else Silas's own. */
export function styleFor(assigned: Record<string, AppStyle>, app: AppInfo): AppStyle {
  const hit = Object.entries(assigned).find(([id]) => same(id, app.id));
  return hit && STYLES.some((s) => s.value === hit[1]) ? hit[1] : app.builtin;
}

/**
 * Assign `style` to the app `id`. App ids match without case, so an old spelling of the same id
 * is replaced. Choosing the style Silas gives the app anyway (`builtin`) removes the assignment.
 */
export function assignStyle(assigned: Record<string, AppStyle>, id: string, style: AppStyle, builtin?: AppStyle): Record<string, AppStyle> {
  const key = id.trim();
  const out: Record<string, AppStyle> = {};
  for (const [k, v] of Object.entries(assigned)) if (!same(k, key)) out[k] = v;
  if (key && style !== builtin) out[key] = style;
  return out;
}

/** Drop the assignment for `id`. */
export function unassign(assigned: Record<string, AppStyle>, id: string): Record<string, AppStyle> {
  return Object.fromEntries(Object.entries(assigned).filter(([k]) => !same(k, id)));
}

/** The user's assignments, sorted by app id. */
export function assignments(assigned: Record<string, AppStyle>): Array<[string, AppStyle]> {
  return Object.entries(assigned).sort(([a], [b]) => a.toLowerCase().localeCompare(b.toLowerCase()));
}

/**
 * The form snippet triggers are compared in, the same as `match_key` in core/voice/src/snippets.rs:
 * apostrophes dropped, anything not a letter or digit is a space, one space between words, lower case.
 */
export function triggerKey(s: string): string {
  return s
    .replace(/['’]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim()
    .toLowerCase();
}

/** The saved snippet with the same trigger, if any. */
export function findSnippet(snippets: Snippet[], trigger: string): Snippet | undefined {
  const key = triggerKey(trigger);
  return key ? snippets.find((s) => triggerKey(s.trigger) === key) : undefined;
}

/** One line for a list: the first line of the text, with a mark when there is more. */
export function snippetPreview(text: string, max = 80): string {
  const lines = text.trim().split(/\r?\n/);
  let first = lines[0];
  const more = lines.length > 1;
  if (first.length > max) first = `${first.slice(0, max - 1).trimEnd()}…`;
  return more ? `${first} ↵ …` : first;
}
