# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/). Releases are
cut with `/release-workflows:release vX.Y.Z`: it turns the section below into a
`## vX.Y.Z — YYYY-MM-DD` entry, commits, bumps `[workspace.package].version` in
`Cargo.toml` with `scripts/release/update-version.sh`, tags `vX.Y.Z`, and
pushes. The tag triggers `.github/workflows/release.yml`, which builds the
macOS DMG and the Linux `.deb`s, publishes the GitHub Release with this
version's section as its notes, and publishes to the apt repo. See
[RELEASING.md](RELEASING.md).

## [Unreleased]

### Added

- **Tasks** (`tasks = on` in `config.conf`, off by default; restart Wren to
  apply). A task line's box takes a status: `[ ]` open, `[/]` doing,
  `[x]` or `[X]` done, `[-]` cancelled, and any other character reads as
  open.
  After the description go a due date (`📅 2026-12-01`), a priority
  (`🔺 ⏫ 🔼 🔽 ⏬`), `@owner`s, and the done and cancelled dates (`✅`,
  `❌`), in Obsidian Tasks' syntax, with a few documented differences, and tokens
  Wren does not write are kept as they are. See
  [Tasks](https://charliek.github.io/wren/reference/tasks/). With tasks off, nothing changes.
- **Task colours:** due dates, overdue dates (bold, in the danger colour),
  owners, and done and cancelled tasks, dimmed and struck, in every theme
  and both editor engines. Spell check skips the tokens.
- **A status-bar line for the caret's task**, for example "Doing · due Mon
  23 Nov, in 4 days · ⏫ High · @alice.chen".
- **Toggle a task writes the date:** ⌘↵ / Ctrl+Enter adds `✅ <today>`
  when a task becomes done and removes it when it reopens. Typing `x` in
  the box yourself writes nothing.
- **Task commands in the palette:** **Cycle task status**, **Set task
  priority** at each level, **Raise** and **Lower task priority**,
  **Remove task priority**, **Clear task due date**, **Set task status…**,
  **Set task priority…**, **Set task due date…** and **Create or edit
  task…**. ⌘D / Ctrl+D opens the calendar and ⇧⌘↵ / Ctrl+Shift+Enter opens
  the form.
- **Right-click menus on task lines** (advanced engine): the box opens the
  status menu, a priority glyph the priority menu, a due date the
  calendar, and the rest of the line the task menu. Each takes single
  keys. A click on a box cycles open, doing, done.
- **A calendar for the due date** (advanced engine): a month of days with
  a field that takes `fri`, `next mon`, `+2w` or an ISO date, quick picks
  and **Clear**.
- **Suggestions while typing a task** (advanced engine, `tasks-suggest`):
  two letters at the start of a word offer a due date or a priority, and `@`
  alone offers an owner already used in the workspace. Tab picks; Enter never rewrites a word you meant.
- **The task form:** description, status, due date, priority and owners, and
  the Markdown it will write. It saves only what you changed, as one undo
  step, and makes a task of a blank line, an empty list item, a list item
  or a line of text. It refuses to save a due field that is not a date.
- **`tasks-suggest`** turns the suggestions off.

### Changed

- The spelling menu is now one of several menus built on a shared anchored
  popover. Its keys and look are the same.

### Fixed

- In a spelling menu, ↵ and a click on a row that arrive together now act
  in that order, and keys typed while an input method is composing are no
  longer taken by an open menu.

## v0.2.1 — 2026-11-19

### Added

- **A tool dock on boards:** a bar at the bottom of the canvas with a
  button for each tool (Select, Hand, the shapes, Text, Connector) and
  **?**. A click does what the tool's key does. Pressing a tool's key or
  button again locks it, so you can place one shape after another; Esc, V,
  or another tool unlocks it. Hold Space to see each button's key.
- **A style bar on boards:** while something is selected, a bar at the top
  of the canvas holds what styles it: the shape menu, the eight colour
  swatches, the fills, the text sizes, solid or dashed, a connector's
  route and arrowheads, and a menu with Duplicate, Bring to front, Send to
  back, Delete, and the actions palette. Resting the pointer on a control
  previews it on the board.
- **Text size on boards:** a shape's label and free text can be 12, 14,
  20, or 28, from the style bar's **Aa** menu or the palette. A shape
  grows to hold a larger label. The size is one line in the file
  (`style.text-size`), written only when it is not 14, and it goes into
  the SVG, PNG, and draw.io exports. Wren 0.2.0 keeps the line and draws the
  label at 14.
- **The colour you pick stays** for the shapes you add next, until you
  pick another. With a shape or text tool armed and nothing selected, the
  swatches (and the digits 0 to 7) set it.
- **A hint line** in a board's footer names the keys that fit what is
  selected or armed, and **?** opens a sheet of every board key, with a
  filter. `board-hints = off` hides the hint line.
- **Undo, redo, and zoom buttons** in a board's footer; a click on the
  zoom figure fits the board.
- **The pointer shows what a press would do** on a board: a crosshair
  with a shape or connector tool, an open hand with the hand tool or
  Space held, a closed hand during a pan, and resize arrows over a handle.
- **Pinch to zoom a board** on a macOS trackpad, about the pointer. Linux
  has no pinch yet: the windowing library reports none there.
- `board-toolbar` and `board-key-badges` in `config.conf`:
  `board-toolbar = off` removes the dock, the style bar, and the footer's
  buttons; `board-key-badges` is `hold` (the default), `on`, or `off`.

### Changed

- **The hand tool stays armed** after a pan, until you pick another tool.
  It used to go back to Select after one drag.
- **Keyboard zoom steps are half as large** on a board: ⌘= and ⌘- (Alt on
  Linux) and the footer's − and + now also stop at 15, 35, 60, 85, 125,
  175, 250, and 350 %.

### Fixed

- **Lucidchart imports a board's draw.io export.** It refused the file
  because each element was written on one line; draw.io itself read it
  either way.

## v0.2.0 — 2026-11-17

### Added

- **Boards**, a second kind of note: a canvas of shapes and connectors for
  flowcharts and small diagrams, saved as `Name.wren-board` beside the
  Markdown notes of a group. The file is plain text with one field per
  line, so a change to a board is a small, readable diff in git; opening a
  board never rewrites it. Boards save, reload, and keep conflict copies as
  notes do, and their labels are searchable. See
  [Boards](https://charliek.github.io/wren/reference/boards/) and the
  [board file format](https://charliek.github.io/wren/reference/board-format/).
- **Creating one:** the New note dialog (⌘N, Alt+N on Linux) has Markdown
  and Board pills, switched with Tab, and the palette has **New board…**.
- **Building with the mouse:** eight shapes (rectangle, rounded rectangle,
  pill, ellipse, diamond, parallelogram, cylinder, document) and free text,
  connect handles on hover, attached connectors with straight or
  right-angled routes that avoid other shapes, arrowheads, dashes and a
  label, a 20-unit grid, marquee selection, and resize handles.
- **Building with the keyboard:** tool letters (V, H, R, O, D, U, Y, P, T,
  C), Tab to add the next connected shape, ⌘+arrow (Ctrl+arrow on Linux)
  to connect in a direction, the arrows to walk from shape to shape,
  ⌥+arrow (Alt+arrow) to move, ⌥⇧+arrow (Alt+Shift+arrow) to resize, Enter
  to edit a label, and C with a typed search to connect to a distant
  shape. Duplicate (⌘D / Ctrl+D), copy, cut, and paste between boards, and
  undo and redo.
- **Colours and arranging:** the digits 0 to 7 set one of eight colours
  (neutral, red, orange, yellow, green, cyan, purple, blue) that every theme
  supplies and that stay distinguishable under common colour blindness;
  fills none, tint, and solid; ] and [ bring to front and send to back, ⇧]
  and ⇧[ one step; align and distribute from the palette or the macOS
  Board menu.
- **The actions palette:** / or Shift+F10, or a right-click (Ctrl-click on
  macOS), lists what applies to the selection and previews each style
  before you choose it.
- **Jump to node:** on a board, Jump to heading (⇧⌘O, Alt+Shift+O) and Find
  in note (⌘F, Alt+F or Ctrl+F) list its labelled shapes and connectors; a
  workspace search hit on a board selects and centres the match.
- **Zoom and pan:** the wheel pans and ⌘+wheel (Ctrl+wheel) zooms, the
  text-size keys (⌘= ⌘- ⌘0, Alt on Linux) zoom in steps, and ⇧1 and ⇧2 fit
  the board or the selection.
- **Export** as SVG, PNG, or a draw.io file from the palette and the macOS
  Board menu, and with **`wren export`** on the command line, with no
  window: `wren export "Flow.wren-board" -o flow.png --theme wren-dark`.
- **A Board menu** on macOS, between View and Window. Go's **Jump to
  Heading…** is now **Jump in Note…**.
- **`boards`** in `config.conf`, on by default; `boards = off` hides boards
  and leaves their files alone.
- **Spell check** on the `advanced` editor engine: misspelled words get a
  red zigzag, and a right-click or **Suggest spelling** (⌘., Alt+. on
  Linux) opens a menu of up to five corrections. It works offline and the
  same on both platforms, with a bundled US English dictionary (SCOWL
  `en_US`) plus your own `words.txt` beside `config.conf`, which **Open
  spelling word list** creates and opens. Code, URLs, link destinations,
  front matter and HTML are never checked. `spell-check` in `config.conf`
  is on by default, and **Toggle spell check** in the palette flips it. See
  [Spell check](https://charliek.github.io/wren/reference/spell-check/).
- **Toggle collapsed columns** (⇧⌘Y, Alt+Shift+Y on Linux) folds Sections
  and Groups down to the Notes column, as a double-click on a note does,
  without moving the open note, the caret or the keyboard; press it again
  to bring them back. It is in the palette and the macOS View menu, beside
  a new **Toggle Focus Mode** row.
- **Line cut, copy and paste:** with nothing selected, ⌘X and ⌘C (Ctrl on
  Linux) take the caret's whole line, as in VS Code. Cut is one undo step,
  and pasting a line copied that way puts it above the caret's line.

### Changed

- **Tab and Shift+Tab move between the columns:** Sections, Groups, Notes,
  then the editor. In the editor, on a board, in the New note dialog and in
  a filter box, Tab keeps its own job.
- **A new section is selected,** as new groups and notes already were, and
  a new section or group opens its column and takes the keyboard.

## v0.1.1 — 2026-11-10

### Changed

- **The editor's text starts at the left of its pane** and wraps at
  `editor-wrap-column` characters (default `100`), measured in the font's
  `0`, so the same text fits at any size; `none` wraps at the pane's edge.
  Focus mode centres the text, with the line numbers in its left margin.
  `editor-wrap-column` replaces `editor-max-width`, which was in pixels.
  Without line numbers the text sits a little further from the pane's edge.
  An `editor-max-width` line left in `config.conf` now reports an unknown
  setting; delete it.

## v0.1.0 — 2026-11-08

### Added

- **Markdown assist.** Enter continues lists, numbered lists (renumbering
  the items after), tasks, and quotes, and on an empty item moves it out a
  level or ends the list. Shift+Enter breaks a line inside an item. Tab and
  Shift+Tab nest and un-nest list items, Backspace removes markup before
  characters, and Home stops after the markup first. The `markdown-assist`
  setting (default `on`) and the palette's **Toggle Markdown assist** turn
  it off; Tab still inserts spaces to the next multiple of 4 when it is.
- **Markdown commands:** Toggle task (⌘↵), Bold (⌘B), Italic (⌘I), Inline
  code (⌘E), and Move line up / down (⌥↑ / ⌥↓), also in the palette under
  **Markdown**. On Linux the first four take Ctrl (Ctrl+Enter, Ctrl+B,
  Ctrl+I, Ctrl+E).
- **Jump to heading** (⇧⌘O, Alt+Shift+O on Linux) lists the open note's
  headings. Moving the highlight shows each one; ↵ keeps it and Esc puts
  the cursor and scroll back.
- **Word deletes and note motions.** ⌥⌫ / ⌥⌦ (Ctrl or Alt with Backspace
  / Delete on Linux) delete a word, ⌘⌫ deletes to the start of the line,
  and ⌘↑ / ⌘↓ (Ctrl+Home / Ctrl+End) go to the start or end of the note.
  Backspace and Delete remove a whole character, emoji and accents
  included.
- **An advanced editor engine**, `editor-engine = advanced` or the
  palette's **Switch editor engine**, still being evaluated (`classic` stays
  the default). It sets H1–H3 headings larger, wraps a list item's or a
  quote's long lines under its text, keeps the exact scroll through undo,
  and places outline jumps precisely.
- **`editor-line-height`** (default `1.7`) sets the row height as a
  multiple of the text size.
- **`editor-max-width`** (default `760`) caps the editor column's width and
  centres it in a wider pane; `none` lets it fill the pane.

### Changed

- **Search previews** show a note's first paragraph line outside front
  matter, headings, and code, as plain words: list, task, and quote
  markers, emphasis and code marks, and link destinations are dropped.
  Search snippets start and end on whole words and separate blocks with
  " · ".
- **Retuned themes.** All eleven built-in themes keep their upstream hues
  and are tuned against one set of tested rules for contrast, surface
  steps, and distinct colours. In the editor, H1 and H2 headings, links,
  inline code, and quotes each have their own colour, fenced code blocks
  sit on a band, done tasks and struck words are quieter but still
  readable, and the selection has its own fill. Markdown markers recede to
  the faintest colour. Inline code no longer shares the "Saved" green.
- **The focused column stands out.** Columns without keyboard focus show
  their selected row in a quieter fill, without the accent edge.
- **Note rows** show a note's age at the end of its title row, leaving the
  second row to the preview.
- **Change theme…** shows a small preview of each theme. The palettes
  share one 680 px width (Jump to heading keeps its narrower one), and the
  search list fits its results.
- **Narrower default columns** (176, 192, and 256 px), so the editor gets
  more room. **Reset column widths** returns to them.
- **Dialogs** dim the window more in dark themes and less in light ones.

### Fixed

- **Bold and italic across several words.** `**Alice Chen**` was bold
  only on its last word and `*Bob*` was not italic, because each word took
  the font of the marker in front of it. Markers now share their span's
  font.
- **⌃A, ⌃E, ⌃B, and ⌃F move the cursor on macOS** (start and end of the
  line, back and forward a character), as in other Mac text views.
- **Line breaks keep the note's line endings.** Enter, paste, and input
  method commits in a CRLF (or CR) note insert that ending instead of LF.
- **Undo and redo keep the view.** They change only the text that differs,
  so the editor no longer reloads the note and jumps to the cursor's edge of
  the window.

## v0.0.3 — 2026-11-08

### Added

- **Keyboard navigation between columns.** One of Sections, Groups, Notes,
  or the editor has the keyboard, shown by an accent header and an accent
  border on its selected row. ↑ / ↓ select rows live, ↵ or → moves right, ←
  moves left, and Esc returns to the editor. ⇧⌘[ / ⇧⌘] (Alt+Shift+[ /
  Alt+Shift+] on Linux) move a column left or right from anywhere, including
  the editor. Arrow keys typed in the editor or a filter box keep their usual
  jobs.
- **Folding columns.** Moving right by keyboard or double-click folds
  Sections (and then Groups) into a `‹ Section › Group` path at the top of
  the leftmost open column, giving the editor their width. Click a name or
  `‹` to unfold; moving left only unfolds. Notes never folds.
- **Shortcuts follow the active column.** ⌘P jumps among sections or
  groups when one of those columns is active (⌘P again widens to every
  note), ⌘N creates a section or group there, ⌘F filters that column, and
  ⌘R renames its selection; Alt on Linux. The title-bar Jump button follows
  too. Palette commands are unchanged.
- **⌘R (Alt+R on Linux) renames** the open note, or the selected section or
  group in their columns.
- **Four more dark themes:** Tokyo Night (`tokyo-night`), Dracula
  (`dracula`), Catppuccin Mocha (`catppuccin-mocha`) and Catppuccin Macchiato
  (`catppuccin-macchiato`), each meeting the same 4.5:1 text contrast as the
  other built-ins.
- **Text size shortcuts:** ⌘+ / ⌘- / ⌘0 (Alt on Linux) make the note text
  bigger, smaller, or back to the default, one point at a time, and save the
  size to `config.conf` as `editor-font-size`. ⌘= works too, without Shift.
  The line-number gutter widens with the text.
- **Editor font picker:** **Change editor font…** in the command palette
  lists the installed monospace fonts, bundled JetBrains Mono first, and
  saves the choice as `editor-font-family`. That setting accepts any
  installed family, including proportional ones, if you set it by hand.
  Bitmap-only fonts, such as macOS's GB18030 Bitmap, are left out.
- **Live preview in the theme and font pickers.** Moving the highlight shows
  that theme or font right away. Enter keeps it; Esc puts the saved one back.

### Changed

- **Double-clicking a row selects it and moves one column right**, the same
  as ↵. It used to rename the row; rename is now ⌘R / Alt+R or the palette.
- **A single click on a note keeps the keyboard in the Notes column**, so ↑ /
  ↓ keep moving through notes. Click into the editor, or press ↵, to type.
- **Focus mode toggles with ⌘Y on macOS and Alt+Y on Linux**, replacing ⌘\ /
  Alt+\. A `keybind` line in `config.conf` can still bind it elsewhere.
- **Notes live only in groups.** A section holds groups, and the "(No group)"
  row is gone. `.md` files placed directly in a section folder are ignored
  with a warning; move them into a group folder to see them. In an empty
  workspace the editor area walks you through New section…, New group…, and
  New note…, and selecting a section opens its first group.
- **Documentation moved to a site** at https://charliek.github.io/wren/,
  replacing `SPEC.md`, `PLAN.md`, and `MOCKUP.md`.

### Removed

- **The Outline layout**, with its title-bar toggle and the
  `view.columns` / `view.outline` commands (⌥⌘1 / ⌥⌘2). The Columns
  layout is the only one, and keyboard drill-down navigation is replacing
  Outline's space savings. Saved column widths carry over; an old
  `layout = 'outline'` line in `.wren/workspace.toml` is left alone and
  ignored, and a `keybind` line naming a removed command now warns.

### Fixed

- **Fonts with malformed tables no longer crash Wren.** Wren now builds with a
  patched copy of the swash font library carrying Roost's safety fixes. One
  macOS font, GB18030 Bitmap, aborted a debug build when previewed in the
  font picker.
- **Only one filter field holds focus at a time.** Clicking a filter left any
  filter to its right focused too, with a focus ring and caret in each, because
  the click never reached them.

## v0.0.2 — 2026-11-07

### Changed

- **The macOS DMG is signed with a Developer ID and notarized.** Wren now opens
  with a normal Gatekeeper check instead of needing the
  `xattr -dr com.apple.quarantine` bypass. Wren 0.0.1 picks this release up
  through "Wren → Check for Updates…".

### Fixed

- The appcast unit tests and `tools/check_updater.py` no longer treat the
  committed update feed as empty, which broke CI once 0.0.1 was published to
  it.

## v0.0.1 — 2026-11-07

The first public build of Wren: a desktop app for structured Markdown notes,
for macOS (Apple silicon) and Linux (amd64, arm64).

### Added

- **Plain Markdown workspaces.** A workspace is a directory of `.md` files you
  own; Wren keeps no database, so git, `grep` and other editors keep working
  on the same folder. Several workspaces can be registered and switched.
- **A fixed, shallow hierarchy:** Workspace → Section → Group → Note, with
  notes allowed directly in a section. Create, rename, move and delete notes,
  groups and sections from the UI; deleted items go to the OS trash.
- **One open note at a time, no tabs.** Back/forward history and a quick
  switcher (⌘P / Alt+P) replace tab sprawl; fuzzy title search and indexed
  full-text search (⇧⌘F / Alt+Shift+F) cover the whole workspace.
- **Always-on autosave.** There is no Save command; writes are durable, and
  changes made on disk by other tools are picked up live.
- **A Markdown editor** with a line-number gutter, current-line highlight,
  accent caret, and struck-through completed tasks.
- **Two layouts** (columns, or a section/group outline) with resizable panes,
  plus a command palette (⌘K / Alt+K) for every action.
- **Settings and keybindings** in one user-owned `config.conf` (editor font
  size, line numbers, theme and more; see [docs/configuration.md](docs/configuration.md)
  and [docs/keybindings.md](docs/keybindings.md)), with several built-in
  themes.
- **Native integration:** the owl app icon on both platforms (the macOS 26
  glass icon where Xcode 26 builds it), a native macOS menu bar whose Quit
  saves first, and a Linux desktop entry and hicolor icons.
- **Release packaging:** a DMG for macOS and `.deb` packages for Ubuntu 24.04+
  (also published to the apt repo at `apt.stridelabs.ai`).
- **Automatic updates on macOS** through Sparkle: "Wren → Check for Updates…"
  plus Sparkle's scheduled checks, offering each stable release as a signed
  update. On Linux, updates come through apt.
