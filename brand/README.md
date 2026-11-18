# Silas brand

**The faithful scribe.** Tagline: *Speak. Silas writes.*

## Story

Silas, also called Silvanus, traveled with Paul and wrote Peter's first letter for him:
"By Silvanus, a faithful brother... I have written briefly" (1 Peter 5:12).
The app does the same job: you speak, and Silas writes it down faithfully, in your words and spelled your way.

## Mark

`icon.svg` is a gold pen nib on a diamond-black rounded square with a thin gold rim. The nib's
slit rises into a five-bar sound wave, so the pen and the voice are one shape. The nib is cut in
four flat facets, lit from the upper left, so it reads as struck gold without gradients or glow.

| File | Use |
| --- | --- |
| `icon.svg` / `icon-1024.png` | App icon source. Regenerate with `bunx tauri icon brand/icon-1024.png` (delete the android and ios folders it adds). |
| `wordmark.svg` | Faceted nib plus "Silas" (Charter, outlined) in parchment. Set it on diamond black. |
| `tray.svg`, `tray-recording.svg` | Menu-bar glyphs. Black on transparent; macOS renders them as template images. Export to `src-tauri/icons/tray*.png` at 22 and 44 px. |

Keep clear space around the mark equal to the nib's collar height. Don't recolor the nib,
add effects, or set the wordmark in another face.

## Color

Diamond black and gold. Silas is dark only: it does not change with the system light or dark setting.

| Token | Hex | Role |
| --- | --- | --- |
| Diamond black | `#0A0A0B` | Background. The sidebar is `#050506`, cards `#101012` |
| Parchment | `#F3EEE4` | Text. Secondary `#BDB6A8`, muted `#8E877A` |
| Gold | `#D6AE45` | The one accent: primary buttons, active states, the nib |
| Gold ink | `#E8C66C` | Gold text and links on black |
| Gold high / deep | `#F2D58A` / `#A7832B` | Focus ring and keycap edge / dark edges |
| Gold hairline | `#D6AE45` at 30% | The hero card's border and the sidebar edge |
| Facets | `#F6DE9C` `#DDB451` `#C49A36` `#8F6C22` | The nib's four faces, light to dark |
| Green / red | `#86BF93` / `#EC7D65` | Saved and error states only |

Cards carry one facet: a 1 px light line along the top edge (`--facet`). No gradients, glass or glow.

## Type

- Wordmark and headings: a serif. The wordmark is Charter outlined to paths. In the app,
  headings use the system serif stack `ui-serif, "New York", "Iowan Old Style", Charter, Georgia, serif`.
  No web fonts are loaded at runtime.
- Body and UI: the system UI stack (`system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif`).

## Voice

Plain, warm and direct. Say what Silas did ("Learned: Argon one → Argon1"), not how clever it is.
The scripture line appears once, in About. Don't use it anywhere else.
