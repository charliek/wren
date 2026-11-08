# Theme tokens and the editor

Status: in progress on `design/theme-polish` (2026-11-08). This note is the
contract between the theme work and the Markdown editor rebuild
(`markdown-editor` / `md-editor`), so that both can land without conflicts.

## Merge order

1. The editor rebuild lands first.
2. The theme commit lands next. It touches no editor files: `wren-core`'s
   `theme.rs` and `index.rs`, plus app chrome (`palette.rs`,
   `workspace_view.rs`, `chrome.rs`). A trial merge onto `md-editor` merged
   cleanly.
3. The editor-integration commit is then ported onto `wren-editor`: a few
   `Palette`/`Style` fields, the `editor_colors` mapping, and the highlighter
   and band changes below.

To keep this order cheap, the theme side stays out of `crates/wren-editor`,
and the editor side stays out of `crates/wren-core/src/theme.rs`.

## Tokens added to `wren_core::theme::Colors`

| Token | Meaning |
|---|---|
| `heading` | H1 text (bold). |
| `heading2` | H2 text (bold). H3–H6 stay `text`. |
| `link` | Link text and bare `http(s)://` URLs. Brackets and destinations are `faint`. |
| `code` | Inline code spans. It replaces the old use of `success`, which is now status only. |
| `code_block` | A background band behind fenced code blocks, fences included. |
| `quote` | Block-quote text, drawn italic. The `>` marker is `faint`. |
| `completed` | The text of done tasks and of `~~struck~~` words: readable de-emphasis. Markers and strike lines stay `faint`. |
| `text_selection` | The editor's selection fill, apart from list rows' `selected`. |
| `selected_inactive` | Selected list rows in columns without keyboard focus. App only. |

Every built-in theme defines all of them. The tests in `theme.rs` require:

- `heading`, `link`, `code`, and `quote` at 4.5:1 or better on `editor`;
- `code` at 4.5:1 or better on `code_block`;
- `code_block` at least 1 L* away from `editor`.

Round 2 adds rules for the new tokens. `quote` and `completed` must not
outshine `body`. Inside a `text_selection`, body text must stay at 4.5:1 or
better, syntax colors at 3.5:1, and `faint` at 3:1.

The interim editor mapping, until the theme lands, is:
- `heading2` ← `heading`
- `completed` ← `faint`
- `text_selection` ← `selected`

## What the editor side needs

On `md-editor`, `wren-iced/src/theme.rs::editor_colors` maps a theme onto
`wren_editor::Style` and `wren_editor::Palette`. The integration adds:

- `Palette`: `heading`, `heading2`, `link`, `code`, `quote`, and `completed`. `success` and `danger`
  can then leave the palette: code and strikethrough no longer use them.
- `Style`: `code_block`, the band color. `selection` maps from
  `text_selection`.

The highlighter's mapping (the `df7a6c1` commit on `design/theme-polish`):

- H1 is bold `heading`, H2 is bold `heading2`, and H3–H6 are bold `text`.
  `#` markers are `faint`.
- A done task (`- [x] text`) has a `faint` marker and `completed` text.
  `~~struck~~` has `faint` tildes and `completed` text.
- `[text](url)` and `[text][ref]`: the brackets and destination are `faint`,
  and the text is `link`. Bare `http(s)://` URLs are `link`, with trailing
  sentence punctuation excluded.
- `> quote`: the marker is `faint`, and the text is italic `quote`. Inline
  markup inside a quote keeps its own style.
- `` `code` `` is `code`. Fenced block bodies stay `muted` on the band.
- **Emphasis markers carry their span's font.** `**`, `*`, `***`, `__`, and
  `_` keep the `faint` color, but use the bold or italic face of the text
  they wrap. cosmic-text shapes each word in the font of its first glyph, so
  a regular-weight `**` turns `**Alice` regular: `**Alice Chen**` rendered
  only "Chen" bold, and `*Bob*` never rendered italic. The `md-editor`
  highlighter (`extract_emphasis`, `Highlight::colored(faint)` for the
  markers) still has this bug.

The widget draws the code band like the current-line band: rows whose logical
line is inside a fence (`code_block_lines`, which counts both fences) get a
`code_block` quad. It starts 8 px left of the text and runs to the right row
inset, beneath the current-line band.

## Asked for by reviewers, and owned by the editor

All four design reviewers asked for these. Only the new engine can deliver
them, so they belong to the rebuild, not the theme work:

- **A heading type scale** (for example H1 1.35em, H2 1.18em, H3 1.05em).
  Color alone gives weak hierarchy.
- **Hanging indent** for wrapped list items and quotes. Plan §130 already
  calls for a vendored cosmic-text patch.
- **A readable maximum width** outside focus mode, and an `editor-line-height`
  setting.
- **A brighter number for the current line** in the gutter.
