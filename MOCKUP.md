# Wren — UI Mockup Reference

This is the visual design for Wren. Read it with `SPEC.md`: **the spec decides
behavior, this file decides look and feel.** Where this file and the spec
disagree on behavior, follow the spec.

- **Live, clickable mockup:** https://claude.ai/artifact/SrtCo5tswrAedxr8Z59S1f
  (private to the owner. If you can't open it, use the source files below).
- **Source files:** `mockup/*.dc.html`. Each file is one screen. The markup
  inside `<x-dc>` is plain HTML with inline styles, so exact colors, sizes and
  spacing can be read straight from it. `{{name}}` holes are filled by the
  `class Component` script at the bottom of the file; that script is mockup
  logic only and is not meant as a model for the Rust code. The files need
  the design tool's runtime and **will not render on their own**. Read them as
  source, don't open them in a browser.
  `mockup/Main.dc.html` holds every screen. The other files show it in one
  fixed state (`layout=` / `scene=` attributes).

## Screens

| File | Shows |
|---|---|
| `Main.dc.html` | **Layout A — Columns.** Title bar, Sections, Groups, Notes, Editor. Interactive in the live mockup: selection, per-pane filters, layout toggle, pane resizing, palette, search. |
| `Outline.dc.html` | **Layout B — Outline.** Sections & groups as one two-level outline, then Notes, then Editor. |
| `Palette.dc.html` | Command palette (`⌘K`), grouped: This note · Go · View · Workspace. First row is highlighted. |
| `SwitchWorkspace.dc.html` | Palette in "Switch workspace" mode: an accent chip in the input, workspaces with their paths, "current" marker, "Add workspace…". |
| `Search.dc.html` | Global search (`⌘⇧F`) for "promo": each result shows a breadcrumb, the title, and a snippet with the match highlighted. |
| `Drag.dc.html` | Layout A mid-drag: a note dragged from *1:1s › Alice Chen* toward *Projects › API Migration*. |
| `OutlineDrag.dc.html` | The same drag in Layout B. |
| `Resize.dc.html` | Resizing the Groups pane: active handle plus the width chip. |
| `Focus.dc.html` | Focus mode: no browser panes, editor text centered. |
| `Map.dc.html` | Diagram of the hierarchy on disk and of the navigation between views. |

## Design tokens

Dark theme only for the MVP; a light theme comes later. All values are in
logical px.

**Color**

| Token | Value | Use |
|---|---|---|
| `bg.editor` | `#14161B` | editor background, app root |
| `bg.titlebar` | `#101216` | title bar |
| `bg.pane` | `#17191F` | Sections, Groups, Outline panes |
| `bg.pane.notes` | `#16181D` | Notes pane |
| `bg.input` | `#121419` | filter/search inputs |
| `bg.control` | `#181B21` | title-bar buttons, segmented control |
| `bg.raised` | `#1C1F26` | palette/search dialogs, toasts |
| `bg.selected` | `#2A3140` (border `#3A4456`) | selected row; highlighted palette row |
| `bg.currentLine` | `#191D25` | editor line with the cursor |
| `border.divider` | `#22262E` | pane dividers, header/footer rules |
| `border.control` | `#2A2F38` | inputs, buttons |
| `border.dialog` | `#333944` | dialogs |
| `text.primary` | `#E6E8EC` | |
| `text.body` | `#D5D9E0` | editor prose, button labels |
| `text.muted` | `#A3AAB5` | counts, labels, subtitles, hints |
| `text.faint` | `#8A919C` | line numbers, Markdown markers, placeholders |
| `accent` | `#F0B45A` (amber) | wordmark, H1, bullets, open checkboxes, cursor, focus ring, drop targets, resize handles |
| `success` | `#7FD1A0` | "● Saved" |
| `scrim` | `rgba(6,7,10,0.6)` | behind palette/search |

**Markdown highlighting (editor)**: `# ` and `## ` markers use `text.faint`.
H1 text is `accent` 700. H2 text is `text.primary` 700. `- ` and `- [ ] `
markers use `accent`. A done item `- [x] …` is `text.faint`, struck through.
`**` markers use `text.faint` and the bold text inside is `#FFFFFF` 700.

**Type**: the UI font is Inter (400/500/600/700) and the editor font is
JetBrains Mono. Both are already bundled in roost (`third_party/inter`,
`third_party/jetbrains-mono`).

| Role | Spec |
|---|---|
| UI base | Inter 13 px |
| Pane header label | Inter 11 px, 600, uppercase, letter-spacing 0.08em, `text.muted`. Groups and Notes headers name their parent: "GROUPS · 1:1S", "NOTES · ALICE CHEN" |
| Row subtitle / counts | Inter 11–12 px, `text.muted` |
| Palette input | Inter 15 px |
| Editor | JetBrains Mono 14 px / 24 px line height, line-number gutter 52 px, right-aligned |
| Title-bar wordmark | Inter 14 px 700, bird glyph in `accent` |

**Shape and size**

| Element | Value |
|---|---|
| Title bar | 48 px tall |
| Single-line row (section, group, outline) | min 36 px, padding 0 10, radius 6, 2 px gap between rows |
| Two-line note row | min 54 px, padding 8 10: title (500) + "date · first line" (12 px muted), both ellipsized |
| Pane filter input | 30 px tall, radius 6, search icon 13 px |
| Palette | 620 px wide, 96 px from the top, radius 12, rows min 44 px, shadow `0 24px 64px rgba(0,0,0,0.55)`, footer hints "↑↓ move · ↵ run · esc close" |
| Search dialog | 720 px wide, 72 px from the top, same style; result count right-aligned in the input row |
| `kbd` badge | Inter 11 px, padding 1 6, radius 4, border `#3A404B`, bg `#15171C` |
| Pane defaults / limits | see SPEC §1.4 table |

## Interaction details

- **Title bar** (left to right): wordmark, then the workspace button (folder
  icon, name, path in mono 11 px, chevron), which opens the palette in
  Switch-workspace mode. On the right: the layout toggle (Columns | Outline),
  "Jump to note ⌘P", "Search ⌘⇧F", "Commands ⌘K", and the focus-mode button.
- **Editor header:** breadcrumb (Section › Group › **Note**), then
  "Edited <time>", then a "Move to…" button that opens the palette's move
  picker.
- **Editor footer:** "Markdown · N lines · N words" on the left; "Ln, Col" and
  "● Saved" on the right.
- **Selection states:** a selected row uses `bg.selected`. In Outline, a
  section row is shown as selected only while it is collapsed; when it is
  open, its selected group carries the highlight.
- **Drag-and-drop**:
  - *Ghost:* a card under the cursor with the note title and
    "→ Section › Group", 1 px accent border, rotated −2°.
  - *Source row:* stays in place, 1 px dashed border, 50 % opacity, hint
    "moving…".
  - *Spring-loading section:* dashed accent border, 8 % accent tint, hint
    "opening…". After about 600 ms it opens.
  - *Valid drop target:* 18 % accent tint, solid accent border, hint
    "Drop here".
  - *Toast* at the bottom center: "Hover a section to open it, then drop on a
    group · esc cancel".
- **Resize handle:** a 7 px hit area over the 1 px divider. It turns accent
  (about 45 %) on hover and about 60 % while dragging. During a drag an accent
  chip shows "<Pane> · <width> px" at the top of the divider.
- **Empty states:** "No groups match.", "No notes here yet. Press ⌘N.",
  "No matches.", "Nothing matches “…”."
- **The palette** filters with fuzzy subsequence matching and hides headers
  that have no matching rows. The move picker lists every Section (root) and
  Section › Group except the note's current location.

## Sample content

The mockup uses an invented workspace (sections *1:1s, Projects, Inbox,
Reference*; people *Alice Chen, Bob Ruiz, Priya Nair*) plus a second workspace
called *personal*. That data is only there to fill the screens. Don't ship it.
A version of it would make a good test fixture workspace.
