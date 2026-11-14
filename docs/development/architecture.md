# Architecture

Wren is a Rust workspace on iced 0.14, with all logic in a toolkit-neutral
core and a thin desktop crate on top.

| Crate | Role |
|---|---|
| `crates/wren-core` | The model, scanning, storage, autosave, search index, settings (including where `words.txt` lives), and keybindings. No iced dependency. |
| `crates/wren-markdown` | Markdown structure with no UI dependency: the one block classifier (`block`), the heading outline, and the edit planner, which turns an editing intent (Enter, Tab, Backspace, Home, …) at a selection into replacements. The search index's note previews and the editor's highlighting both classify lines with `block`. The app's outline palette lists its headings. |
| `crates/wren-editor` | The Markdown editor widget: a fork of iced's text editor with the gutter, current line, caret, highlighting, undo, find, and spelling marks and suggestions through a `Proofreader` the host passes in. Undo, redo, and multi-range edits (`EditorView::apply`) replace only the text that changes, and inserted line breaks take the note's line ending. It runs on iced's own text engine or on a cosmic-text editor it owns (the `editor-engine` setting). Depends only on iced (a minimal feature set of its own), `wren-markdown`, and `unicode-segmentation`; the app passes in colours and the font. It is not yet extractable: the advanced engine's hanging indent needs Wren's patched cosmic-text (`vendor/cosmic-text`), the crate has no publish metadata, and its glyph metrics assume JetBrains Mono. |
| `crates/wren-spell` | Spell checking with no UI or Wren dependency (only `spellbook` and `unicode-segmentation`): which words of a line are checked, and whether a dictionary or the user's word list accepts them. Bundles SCOWL's Hunspell `en_US` dictionary. |
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
wherever the keyboard is. An **Editor** command (`markdown.*`, `edit.*`,
`editor.spell_suggest`) runs only in the focused editor: `App::key_input`
ignores its key, and the editor widget handles it instead. Each Editor
command has an editor route (`CommandId::editor_route`, a
`wren_editor::KeyRoute`): an editing intent for the Markdown commands, or
`KeyRoute::Suggest` for the spelling menu. The editor learns which key
is which from a `wren_editor::KeyPolicy` that the app builds from the
keymap, reading keys through the same `keys::shortcut` as its own
shortcuts. The widget asks the policy before handling any key: an App
command's key is ignored, so a shortcut never also edits the text, and an
Editor command's key becomes its route's message (`Message::Intent`, or
`Message::Spell` to open the menu). From the palette, an
enabled Editor command runs wherever the keyboard is, through `App::edit`
at the editor's cursor; the Markdown commands are disabled while
`markdown-assist` is off, and Suggest spelling while spell check cannot
run.

## Editing keys

The editor widget turns its editing keys (Enter, Tab, Backspace, Delete,
the word deletes, Home, …) into `wren_editor::Message::Intent`. It reads
them from iced's own key bindings, so its platform mappings (⌘← as Home
on macOS) carry over. macOS's ⌃A, ⌃E, ⌃B, and ⌃F motions are mapped in
`wren_editor::keys`, because iced 0.14 drops them. Like every other edit, an intent reaches
`App::edit`, which hands it to `EditorView::intent`. That plans it with
`wren_markdown::plan` over the editor's lines in place, without copying the
text, and applies the plan as one undo unit, writing each inserted line
break in the line ending of the line it splits (or a neighbouring one,
where a lone CR and LF would otherwise meet and read back as one break).
After any change,
`App::edit` sends the draft to the `Session` (so autosave sees it) and bumps
the edit revision; the footer's word count follows on the next tick, at most
250 ms later.

## Editor layout

The editor's metrics are the text size and a row height, the text size times
`editor-line-height` rounded to whole pixels; the caret and gutter scale from
the text size. The host passes a `wren_editor::Measure`: a wrap column in
characters and whether to centre. The widget measures the font's `0` once
per font and size, and lays the text out as padding inside the pane it
fills, so clicks and the wheel beside the text still reach the editor.
Left-placed text starts after the gutter and wraps at the column or 24 px
short of the edge; centred text keeps equal margins of at least the gutter,
with the gutter in the left one. `App::editor_measure` is the one place that
decides: focus mode centres, at 100 characters when `editor-wrap-column` is
`none`.

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
changes a line's style or hanging indent, which can change its row heights
or its wrapping, it places the view again: at a pending `scroll_to_line`
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

## Spell check

Spell check is split across three crates so that neither the word rules
nor the editor knows the other ([behaviour](../reference/spell-check.md)).

- **`wren-spell` judges words.** A `Speller` asks its `WordSource`s in
  order: `Hunspell` (`spellbook` over a `.aff` and `.dic` pair; `en_us()`
  is the bundled dictionary in `crates/wren-spell/dict/`, whose source and
  hashes are in `dict/SOURCE.md`) and the user's `WordList`.
  `Speller::marks` takes one line and the byte ranges of it that are prose,
  splits them into words, skips the ones the rules never check (addresses,
  paths, identifiers, capitals, digits, non-Latin scripts), and returns the
  byte ranges of the rest that no source accepts. `Speller::suggestions`
  asks the sources for corrections, best first, without duplicates. A
  second language, a system checker, or a grammar engine would be another
  `WordSource`.
- **The editor finds prose and draws.** The highlighter's per-line
  `LineDecor` carries `prose`, the line's checkable byte ranges, from the
  same inline scan that styles it: the scan marks code spans, comments,
  link destinations, URLs, and emphasis delimiters as excluded, and also
  runs over heading and done-task text for this alone. The editor defines
  the `Proofreader` trait and is handed one with
  `EditorView::set_proofreader`; it never depends on `wren-spell`. Marks
  are computed while a row is drawn, from the line, its prose ranges, and
  the proofreader, and nothing is cached between frames, so an edit or a
  new word list is right on the next frame. The zigzag is one 1 px square
  per pixel of the word in a 4 px period, after the selection and before
  the text. A frame's squares are drawn as a single SVG picture, kept
  while they stay the same: tiny-skia repaints the screen once for each
  changed primitive it cannot merge, so a quad per square cost seconds a
  frame with a screen of marks. The suggestions menu is the
  widget's `overlay()`, placed at layout time from the word's glyph
  extents. Opening it (`EditorView::spell`, with a `SpellEvent`) runs a
  fresh highlighter over the lines down to the target to find its prose,
  then asks the proofreader for suggestions on the UI thread. Accepting is
  an ordinary range edit through `EditorView::apply`, so undo, find, the
  outline, and autosave see it as typing. Only the owned engine marks or
  opens anything; with `BackendKind::Iced` the proofreader is never asked.
- **The app wires them.** `app/spelling.rs` loads the dictionary and the
  first read of `words.txt` on a thread of its own at launch, then re-reads
  `words.txt` beside `config.conf` whenever the settings are reloaded,
  rebuilding the word list only when the file's text changed. Its warnings
  join the settings warnings in the banner. The app adapts a `Speller` over
  both sources to `Proofreader` and hands it to the editor only while
  `spell-check` is on and a note is open for editing. Escape, which always
  reaches `App::key_input`, closes an open menu before it does anything
  else, and the app also closes the menu when the palette or a dialog
  opens, the settings or the theme change, or the window loses focus.

## Themes

A theme is a `wren_core::theme::Colors` value: plain RGB tokens with no
toolkit types. `crates/wren-iced/src/theme.rs` turns it into iced's palette
(`from_definition`) and into the editor's `Style` and `Palette`
(`editor_colors`), mapping each token one to one, and the app chrome reads
the tokens directly.

| Tokens | Used for |
|---|---|
| `editor`, `pane`, `notes`, `titlebar`, `input`, `control`, `raised` | Surfaces, from the editor out to dialogs. |
| `text`, `body`, `muted`, `faint` | The text ramp: titles, note text, secondary labels, and decorative markers. |
| `accent` | The caret, the focused column's selected row edge, and selected controls' text. |
| `divider`, `control_border`, `dialog_border`, `selected_border` | Lines: between panes, around controls and dialogs, and around selected controls and active scrollbars. |
| `selected`, `selected_inactive` | List rows selected in the focused column, and in the others. |
| `current_line`, `text_selection`, `code_block` | Editor bands: the cursor's line, the selection, and fenced code. |
| `heading`, `heading2` | H1 and H2 text. H3 to H6 use `text`. |
| `link`, `code`, `quote`, `completed` | Link text, inline code, quote text, and done tasks and struck words. |
| `success`, `warning`, `danger` | Status: saved, warnings, and errors. `danger` also draws the spell-check zigzag. |

Markdown markers (`#`, `**`, `>`, link destinations, strike lines) use
`faint`, so the words stay the loudest thing on a line. The tests in
`theme.rs` hold every built-in theme to the same rules:

- Reading text reaches 4.5:1 on its surfaces, `faint` 3:1, and the ramp
  steps down from `text` to `faint`.
- `quote` and `completed` are no louder than `body`.
- Inside a text selection, body text stays at 4.5:1, syntax colours at
  3.5:1, and `faint` at 3:1.
- Surfaces step apart by measured CIE L*, enough to see but not to shout:
  panes from the editor, the current line, the code band, and the
  selection from both, and focused selected rows from unfocused ones.
- Colours that sit next to each other stay apart in hue: `heading2` from
  `link` and `text`, `code` from `success`, and the status colours from
  each other and from `accent`.

To add a theme, add a `Theme` to `BUILT_INS` and run the tests; they name
each rule a palette breaks.

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
dropped and rescanned. The spelling dictionary loads on a thread of its own
at launch; until it arrives the editor has no proofreader. Spelling
suggestions are computed on the main thread when the menu opens.

## Search

An in-memory index of every note (path, title, mtime, content) is built when
a workspace opens and kept current from the `Session`'s own writes and
reconciliation. Fuzzy title matching uses `nucleo-matcher`; content search is
a case-insensitive substring scan. On 5,000 notes / 25 MB a query takes a few
milliseconds (`make perf-search`). A note's preview is its first paragraph
line outside front matter and headings, with list, task, and quote markers
dropped.
