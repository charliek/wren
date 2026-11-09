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
  matter, headings, and code, with list, task, and quote markers dropped.

### Fixed

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
