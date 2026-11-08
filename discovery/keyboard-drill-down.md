# Keyboard drill-down navigation

Status: **implemented** (2026-11-08). User-facing behavior is documented in
`docs/getting-started/keybindings.md` ("Moving between columns"). This note
keeps the design decisions and the places where the build refined them.
Design canvas: https://claude.ai/artifact/HyKW5Smk3fsy1SwhfpdPV4 (private to
the owner).

## Problem

The Columns layout is good for drilling from wide categories down to a note,
but it costs a lot of width, so focus mode got heavy use, and moving between
columns needed the mouse.

## Decisions

1. **One active column** (`app/columns.rs`, `App.column`): Sections, Groups,
   Notes, or the editor. Its header turns accent with a dot; its selected row
   gets an accent border.
2. **Arrows select live**, through the same select commands as a click, so
   autosave flushes first. In Notes the keyboard stays in Notes.
3. **Right:** ↵, →, ⇧⌘] / Alt+Shift+], or a double-click. Needs a selection.
   **Left:** ←, ⇧⌘[ / Alt+Shift+[.
4. **A single click only selects** and activates its column.
5. **Folding** (`App.fold`, 0–2): moving right folds the columns left of the
   active one. Notes never folds; focus mode hides it.
6. **Breadcrumb**, not a rail: `‹ Section › Group` at the top of the leftmost
   open pane. A rail needs vertical text, which iced can only draw as glyph
   paths in a canvas; it may come back later as an option.
7. **Esc** from a browser column returns to the editor.
8. **Scoped shortcuts**: ⌘P, ⌘N, ⌘F, ⌘R act on the active column. Palette rows
   never do.
9. **Outline layout removed.**
10. **Rename is ⌘R / Alt+R** (matching Roost), since double-click moves right.

## Refinements made while building

- Moving left, and opening a column from the breadcrumb, only ever unfold;
  folding happens only when moving right (or Esc from a column, a jump pick,
  a double-click). Esc in the editor, which also closes find, never folds.
- The fold is not persisted: a restart or a workspace switch starts with
  every column open and the editor active.
- Arrows and ↵ reach the columns only when no widget captured them, so the
  editor, filter boxes, and resize grips keep their keys. An Esc that a text
  field captured (`Nav::FieldEscape`) leaves a filter box but keeps the
  column; it still closes the palette, a dialog, or the find bar.
- Typing in a column's filter makes that column active.
- Selecting a section opens its first group (the existing core rule), not the
  group last visited in it.
- Shifted punctuation is matched by its physical key, so ⇧[ is
  `shift+bracketleft` rather than `{`.
- ⌘⇧[ from the editor in focus mode leaves focus mode.

## Possible follow-ups

- A collapsed side rail as an alternative to the breadcrumb.
- Remember the fold across restarts, if it turns out people want that.
- Drag and drop while columns are folded: folded columns are not drop
  targets, so moving a note to another group needs Move to… or unfolding.
