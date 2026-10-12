# Silas brand

**The faithful scribe.** Tagline: *Speak. Silas writes.*

## Story

Silas, also called Silvanus, traveled with Paul and wrote Peter's first letter for him:
"By Silvanus, a faithful brother... I have written briefly" (1 Peter 5:12).
The app does the same job: you speak, and Silas writes it down faithfully, in your words and spelled your way.

## Mark

`icon.svg` is a gold pen nib on a deep-ink rounded square. The nib's slit rises into a
five-bar sound wave, so the pen and the voice are one shape. The light left half and
darker right half give it a struck-metal look without gradients.

| File | Use |
| --- | --- |
| `icon.svg` / `icon-1024.png` | App icon source. Regenerate with `bunx tauri icon brand/icon-1024.png`. |
| `wordmark.svg` | Nib plus "Silas" (Charter, outlined). Ink text, so use it on parchment or light surfaces. |
| `tray.svg`, `tray-recording.svg` | Menu-bar glyphs. Black on transparent; macOS renders them as template images. Export to `src-tauri/icons/tray*.png` at 22 and 44 px. |

Keep clear space around the mark equal to the nib's collar height. Don't recolor the nib,
add effects, or set the wordmark in another face.

## Color

| Token | Hex | Role |
| --- | --- | --- |
| Ink | `#14161A` | Dark background, light-mode text |
| Parchment | `#F5F0E6` | Light background, dark-mode text (`#EDE6D8` on dark) |
| Scribe gold | `#C8A15A` | The one accent: active states, recording, the nib |
| Gold, deep | `#8A6A2E` | Gold text on parchment (passes AA) |
| Quill blue | `#2F4E73` / `#8FB0D6` | Secondary, used sparingly for info and links |
| Nib light / dark | `#D9B676` / `#B98F4B` | The mark's two faces |

## Type

- Wordmark and headings: a serif. The wordmark is Charter outlined to paths. In the app,
  headings use the system serif stack `ui-serif, "New York", "Iowan Old Style", Charter, Georgia, serif`.
  No web fonts are loaded at runtime.
- Body and UI: the system UI stack (`system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif`).

## Voice

Plain, warm and direct. Say what Silas did ("Learned: Paris one → Parish1"), not how clever it is.
The scripture line appears once, in About. Don't use it anywhere else.
