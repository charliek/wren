# Architecture

Wren is a Rust workspace on iced 0.14, with all logic in a toolkit-neutral
core and a thin desktop crate on top.

| Crate | Role |
|---|---|
| `crates/wren-core` | The model, scanning, storage, autosave, search index, settings, and keybindings. No iced dependency. |
| `crates/wren-markdown` | Markdown structure with no UI dependency: the one block classifier (`block`), the heading outline, and the edit planner, which turns an editing intent (Enter, Tab, Backspace, Home, …) at a selection into replacements. The search index's note previews and the editor's highlighting both classify lines with `block`. The app's outline palette lists its headings. |
| `crates/wren-editor` | The Markdown editor widget: a fork of iced's text editor with the gutter, current line, caret, highlighting, undo, and find. Undo, redo, and multi-range edits (`EditorView::apply`) replace only the text that changes, and inserted line breaks take the note's line ending. It runs on iced's own text engine or on a cosmic-text editor it owns (the `editor-engine` setting). Depends only on iced, `wren-markdown`, and `unicode-segmentation`; the app passes in colours and the font. |
| `crates/wren-iced` | The desktop app (binary `wren`): views, widgets, the command table, gestures, and the macOS menu bar. |
| `crates/wren-sparkle` | The macOS Sparkle updater bridge. The only crate allowed `unsafe`; empty on Linux. |
| `tools/` | The Python functional harness, packaging, scene capture, and release helpers. |

`CLAUDE.md` at the repository root has the file-by-file layout and the
contributor conventions.

## One ordered owner of every mutation

Every change to notes, folders, or workspace metadata is an `Op` applied by
the core's `Session` (`crates/wren-core/src/ops.rs`). Clicks, drags, palette
commands, shortcuts, menu items, and the test protocol all reach the same
path, `App::dispatch(Command::Run(id))`, which turns into `Session` ops. The
UI renders from the `Session`'s model and events and never keeps its own copy
of the truth.

The UI gateway reports what happened to each requested action: blocked,
completed, committed with a warning, or pending. Dialogs and pickers close
only when their effect committed, and a committed action is never replayed.

IDs (`SectionId`, `GroupId`, `NoteId`) are stable for a session and survive
moves and renames. The UI holds IDs, not paths, and resolves them again when
an action runs.

## Commands and keybindings

A command has a `CommandId`, a stable key (`note.new`, `navigation.jump`), a
palette label, a group, and a scope. To add one, add a `CommandId`, a
`definitions!` row in `app/commands.rs`, and a case in `run_command`. Default
shortcuts live in `wren_core::keybind::default_bindings`; users override them
with `keybind` lines. macOS uses Command as the app modifier and Linux uses
Alt, leaving Ctrl to text editing. `keys::alt_filter` drops the text of Alt
presses on Linux so shortcuts never type their letter.

The scope says who runs a command from its key. An **App** command runs
wherever the keyboard is. An **Editor** command (`markdown.*`, `edit.*`)
runs only in the focused editor: `App::key_input` ignores its key, and the
editor widget turns it into an intent instead. The editor learns which key
is which from a `wren_editor::KeyPolicy` that the app builds from the
keymap, reading keys through the same `keys::shortcut` as its own
shortcuts. The widget asks the policy before handling any key: an App
command's key is ignored, so a shortcut never also edits the text, and an
Editor command's key becomes its `Message::Intent`. From the palette, an
Editor command always runs, through `App::edit` at the editor's cursor.

## Editing keys

The editor widget turns its editing keys (Enter, Tab, Backspace, Delete,
the word deletes, Home, …) into `wren_editor::Message::Intent`. It reads
them from iced's own key bindings, so the platform mappings (⌘← and ⌃A as
Home on macOS) carry over. Like every other edit, an intent reaches
`App::edit`, which hands it to `EditorView::intent`. That plans it with
`wren_markdown::plan` over the editor's lines in place, without copying the
text, and applies the plan as one undo unit, writing each inserted line
break in the line ending of the line it splits. After any change,
`App::edit` sends the draft to the `Session` (so autosave sees it) and bumps
the edit revision; the footer's word count follows on the next tick, at most
250 ms later.

## Editor layout

The editor's metrics are the text size and a row height, the text size times
`editor-line-height` rounded to whole pixels; the caret and gutter scale from
the text size. The column of gutter and text grows to `editor-max-width` and
is centred past it: the widget fills its pane and pads the column in, so
clicks and the wheel beside it still reach the editor. Focus mode uses the
same column, at 760 px when the setting is `none`.

The highlighter gives every span a `Scale` as well as a colour and a font:
the text size and the row height, as multiples of the body's. H1 is 1.35 em,
H2 1.18 em, and H3 1.05 em; their `#` markers keep the body size in the
heading's row. Only the advanced engine applies it, as cosmic-text span
metrics (`Attrs::metrics`), because iced's highlighter `Format` carries only
a colour and a font; classic draws every row at the body size. In
cosmic-text a visual row is as tall as the tallest span metrics on it, and
spans without metrics do not count, so a heading's markers carry its row
height. Everything the widget draws comes from the layout runs, each with
its own top and height: the current-line and code bands, selection
rectangles and selected line breaks, strikes, the caret (as much taller as
its row), and the IME rectangle. A line number is set at the body size so
its baseline meets its row's. Clicks hit-test through cosmic-text against
the same runs, and the owned engine counts PageUp and PageDown by row
heights rather than body rows.

Row heights exist only once a line is highlighted, and highlighting runs
after the engine's update has already scrolled. So the owned
engine highlights a page past both the view and the cursor, which lets a
page motion or a short scroll measure real heights. When highlighting
resizes rows, it places the view again: at a pending `scroll_to_line`
target, then as far as needed to show a cursor that moved since the last
draw. A jump to the end of the note, a far Find match, and an outline jump
forward all land where they would on a fully highlighted note.

## Hanging indent and the vendored cosmic-text

On the `advanced` engine, the wrapped rows of a list item or a block quote
start under the line's content column: after `- `, `10. `, or `> `, and for
a task under its box, not after it. cosmic-text 0.15 has one wrap width per
line, so Wren vendors it under `vendor/cosmic-text` with a small patch (see
its `WREN.md`): each `BufferLine` takes a hanging indent as a byte offset,
measured from the line's shaped glyphs, and its continuation rows wrap that
much narrower and start that far in. Hit-testing and drawing follow the glyph
positions; the engine's caret and selection read them too, and its ↑, ↓,
PageUp, and PageDown aim at the caret's x rather than cosmic-text's glyph
index (a page still counts its rows by their heights). The offset is the
end of the line's container prefix, which the highlighter's block scanner
records with each line it styles, so an edit that changes a marker re-hangs
the lines from there down. The widget highlights while laying out, not only
before drawing, because a key or click can arrive before the next frame and
moves over the layout it finds. The `classic` engine's buffer belongs to
iced, so it wraps every row at the left edge.

`[patch.crates-io]` stops applying silently once a dependency needs a newer
version, so `make check` (`check-patches`) fails unless the vendored `swash`
and `cosmic-text` are the ones in the build.

## Storage safety

- Storage walks validated workspace-relative paths through no-follow
  directory descriptors, so a symlink swapped in mid-operation cannot
  redirect a write.
- Saves write an exclusive sibling temporary, fsync it, compare the target
  against the exact bytes last loaded, rename over it, and sync the parent.
- Creates and moves publish without replacing anything, adding collision
  suffixes instead.
- Delete goes to the OS trash and never falls back to unlinking.
- An advisory lock keeps two cooperating Wren processes from writing the
  same workspace.

## Autosave

`autosave.rs` is a pure state machine with injected time: a 750 ms idle
debounce plus a 750 ms oldest-dirty deadline, with revision-aware
completion so a late save acknowledgment cannot mark newer edits clean.
Every note or workspace transition, and quit, runs a flush barrier first; a
failed flush blocks the transition and keeps the draft.

## Threading

iced's `update` and `view` run on the main thread, and the `Session` is
synchronous there. Workspace loading and reconciliation run off it: a
reconcile job scans through a duplicated root descriptor on a worker thread
and returns only changed notes, which the owner merges in order. Results from
an older job, another workspace, or a tree changed in the meantime are
dropped and rescanned.

## Search

An in-memory index of every note (path, title, mtime, content) is built when
a workspace opens and kept current from the `Session`'s own writes and
reconciliation. Fuzzy title matching uses `nucleo-matcher`; content search is
a case-insensitive substring scan. On 5,000 notes / 25 MB a query takes a few
milliseconds (`make perf-search`). A note's preview is its first paragraph
line outside front matter and headings, with list, task, and quote markers
dropped.
