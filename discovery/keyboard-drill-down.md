# Keyboard drill-down navigation

Status: **designed, not started** (2026-11-07). Visual reference: the design
canvas at https://claude.ai/artifact/HyKW5Smk3fsy1SwhfpdPV4 (private to the
owner). It has an interactive prototype, a keymap card, and fixed states for
each depth.

## Problem

The Columns layout is good for drilling from wide categories down to a note,
but it costs a lot of width, so focus mode gets heavy use. Moving between
columns needs the mouse: arrow keys do nothing outside the palette, and most
shortcuts only act on notes.

## Decisions

1. **One active column.** Exactly one of Sections, Groups, Notes, or the
   editor has the keyboard. Its header label turns accent-colored with a
   small dot. Its selected row gets an accent border, and selected rows in
   other columns turn muted.
2. **Arrows select immediately.** ↑ / ↓ in Sections or Groups select the next
   item, so the column to the right shows its contents and its last-open note
   opens, exactly as a click does today. In Notes, ↑ / ↓ open each note in the
   editor while the keyboard stays in Notes.
3. **Moving right:** ↵, →, ⌘⇧] (Alt+Shift+] on Linux), or a double-click
   selects and moves one column right. From Notes it moves into the editor
   with the caret where it was. It does nothing when the column has no
   selection, for example an empty group.
4. **Moving left:** ← or ⌘⇧[ (Alt+Shift+[) moves one column left and opens it
   again. ← and → only change columns outside the editor; in the editor they
   move the caret, and ⌘⇧[ is the way out.
5. **A single click only selects**, the same as ↑ / ↓. It makes that column
   active but never folds or moves the layout under the pointer.
6. **Folding.** Moving right by keyboard, double-click, or a scoped jump
   folds every browser column to the left of the active one. Notes never
   folds; focus mode (⌘Y) stays the way to hide it. Active Groups folds
   Sections. Active Notes or the editor folds Sections and Groups.
7. **Breadcrumb for folded columns.** Folded columns disappear, and a 40 px
   path bar at the top of the leftmost open column shows `‹ Section ›
   Group`. Each crumb, and `‹`, opens that column again and makes it active.
   A collapsed side rail was considered and may come back later as an
   option, but not now: iced has no vertical text, so a rail would need a
   custom canvas widget.
8. **Esc in a browser column** returns to the editor without changing the
   selection, and folds as if you had moved right. In the editor, Esc keeps
   its current job (closing find and overlays).
9. **Scoped shortcuts** follow the active column:

   | Key | Sections | Groups | Notes | Editor |
   |---|---|---|---|---|
   | ⌘P | Jump to section | Jump to group (current section first) | Jump to note | Jump to note |
   | ⌘N | New section | New group | New note | New note |
   | ⌘F | Filter sections | Filter groups | Search titles and text | Find in note |
   | ⌘R | Rename section | Rename group | Rename note | Rename note |

   Pressing ⌘P again in a scoped jump widens it to every note, so the usual
   note jump is always one press away. Picking a section or group in the
   jump selects it and moves right, like ↵. Scoping applies to the
   shortcuts only; a palette row always does what its label says.
10. **Drop the Outline layout.** At Notes depth, Columns with the breadcrumb
    uses about 280 px of browser, against 540 px for Outline, so Outline's
    reason to exist goes away.
11. **Rename is ⌘R (Alt+R on Linux)**, matching tab rename in Roost, because
    double-click now moves right. The three Rename palette commands stay.

## What changes in the code

### 1. Remove Outline (do this first, on its own)

- `panes.rs`: remove `Layout::Outline`, the outline pane widths, and the
  outline arm of the width allocator. `Layout` may collapse to nothing if
  Columns is the only layout; keep `Widths` keyed by pane.
- `app/workspace_view.rs`: remove `outline_pane` and the title-bar
  Columns/Outline segmented control.
- `app/navigation.rs`: remove `Filters::outline`, the `expanded` set, and
  outline reveal logic.
- `app/commands.rs`: remove `CommandId::Columns`/`Outline`, the
  `view.columns`/`view.outline` rows, `Command::Layout`, and
  `Command::ToggleSection` if nothing else uses it.
- `wren_core::keybind`: drop the `view.columns`/`view.outline` defaults. A
  user `keybind` line naming them will then warn as an unknown action,
  which is the existing behavior for unknown actions.
- `wren_core::workspace_state` and `ops.rs`: stop writing `layout` and
  `[widths.outline]`; keep reading old files without warnings. Unknown keys
  already survive saves. Remove `Op::SetLayout`.
- Gestures and drag: remove outline drop targets and outline spring-open.
- Tests: outline cases in `app/navigation_tests.rs`, `crates/wren-core/tests/`,
  and `tools/wrentest/` (`test_navigation.py`, `test_gestures.py`,
  `test_completion.py`, `test_viewport.py`), the `view.layout` test op, and
  outline scenes in `tools/capture_scenes.py`.
- Docs: the two layout rows in `docs/getting-started/keybindings.md` and the
  action list there.

### 2. The active column and keyboard routing

- Add `enum Column { Sections, Groups, Notes, Editor }` and an
  `App.column` field. It is UI state, not persisted. On workspace load it
  starts at `Editor` when a note is open, otherwise at the first column that
  needs input.
- `keys.rs`: add `Nav::Left` and `Nav::Right`. The subscription in
  `App::subscription` currently ignores the event status. Pass it through
  and handle unmodified arrows/↵/Esc only when the event was **not
  captured**: the editor and text inputs capture their own keys, so typing
  is unaffected.
- When `column` leaves `Editor`, unfocus the editor with
  `iced::widget::operation::focusable::unfocus` (iced_core 0.14 has it), so
  arrows stop reaching it. Entering `Editor` focuses `note-editor` as
  today.
- Filter boxes: while a column's filter has focus, ↑ / ↓ / ↵ still drive
  that column's list (`text_input` does not capture ↑ / ↓; ↵ comes from
  `on_submit`). Typing filters as today.
- Arrows reuse the existing selection commands (`Command::SelectSection`,
  `Command::SelectParent`, `Command::OpenNote`). Each step flushes autosave
  first, like a click. Selecting a section already restores its last group
  and note through `Session::section_parent`.
- Clicking a row sets `column` to that row's column, and a note click
  leaves the keyboard in Notes (check where note selection focuses
  `note-editor` today and skip that for clicks). Clicking into the editor
  sets `Editor`.
- **Double-click on a row means rename today** (`drag.rs` /
  `drag_view.rs` own the double-click detection). It becomes "select and
  move right", and rename moves to ⌘R / Alt+R (decision 11). This is the
  one existing behavior that changes; call it out in the changelog.

### 3. Column commands

- Add `CommandId::ColumnLeft` / `ColumnRight` with keys
  `navigation.column_left` / `navigation.column_right`, palette labels
  "Column left" / "Column right" in the Go group, and defaults
  `{p}+shift+bracketleft` / `{p}+shift+bracketright`. `keys.rs` already maps
  physical keys, so Shift+[ resolves to `bracketleft`, not `{`.
- ↵ / → / Esc route through the same functions, so the palette, shortcuts,
  menus, and the test protocol share one path.
- Add both to the macOS menu (View or Go).

### 4. Folding and the breadcrumb

- Fold state is derived: `folded = min(column_index, 2)` after a keyboard,
  double-click, or jump move; single clicks leave it as it is. Store it
  as `App.fold: u8` so a click can change `column` without changing the
  fold.
- `workspace_view.rs`: skip folded columns and their resize handles. Render
  the path bar at the top of the leftmost open column (`‹` plus one crumb per
  folded column, ellipsized). The width allocator in `panes.rs` must treat
  hidden panes as zero width; stored widths are unchanged, so unfolding
  restores them.
- Focus mode is independent: it hides everything; turning it off restores
  the folded state as it was.

### 5. Scoped jump, new, and filter

- `app/palette.rs`: add Jump-to-section and Jump-to-group modes. They reuse
  the fuzzy name matcher from the column filters and show a scope chip in
  the input, as Switch workspace does. ⌘P inside a scoped mode switches to
  note jump.
- `run_shortcut`: when `Jump`, `NewNote`, `Find`, or `RenameNote` arrive
  from a key binding, check `self.column` and dispatch to the scoped
  variant (`section.rename` / `group.rename` for ⌘R). Palette rows are not
  scoped: "Rename note…" always renames the note. Keep the existing command
  keys; the palette rows for New section, New group, and the three Rename
  commands already exist.
- `wren_core::keybind`: add `{p}+r` → `note.rename` to the defaults (⌘R on
  macOS, Alt+R on Linux; neither is bound today). Add it to the macOS menu
  next to the other item actions.

### 6. Test protocol and scenes

- Report `column` and `fold` in the test state JSON
  (`app/test_requests.rs`), and add a `key.nav` request that injects
  Up/Down/Left/Right/Enter/Escape through `key_input`.
- Functional cases: arrows at each depth, ↵ and ⌘⇧] fold, ⌘⇧[ and crumb
  clicks unfold, an empty group blocks moving right, typing in the editor
  never moves a column, scoped ⌘P / ⌘N / ⌘F, and a native X11 case for real
  arrow and Shift+bracket input.
- `tools/capture_scenes.py`: scenes for Groups active, Notes active,
  editing with columns folded, and the scoped jump.

## Suggested commits

1. Remove the Outline layout.
2. Active column, arrows, ↵, Esc, and click/double-click semantics.
3. Column left/right commands and bindings.
4. Folding and the breadcrumb.
5. Scoped jump, new, and filter.
6. Docs: keybindings page, first-run page, changelog.

## Open questions

- Should the fold state survive a relaunch? The default is no: the app
  starts with every column open and the editor active.
- On narrow windows, should moving into the editor also fold Notes? The
  default is no, because focus mode covers it.
