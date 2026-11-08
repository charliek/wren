# Architecture

Wren is a Rust workspace on iced 0.14, with all logic in a toolkit-neutral
core and a thin desktop crate on top.

| Crate | Role |
|---|---|
| `crates/wren-core` | The model, scanning, storage, autosave, search index, settings, and keybindings. No iced dependency. |
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
palette label, and a group. To add one, add a `CommandId`, a `definitions!`
row in `app/commands.rs`, and a case in `run_command`. Default shortcuts live
in `wren_core::keybind::default_bindings`; users override them with
`keybind` lines. macOS uses Command as the app modifier and Linux uses Alt,
leaving Ctrl to text editing. `keys::alt_filter` drops the text of Alt
presses on Linux so shortcuts never type their letter.

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
milliseconds (`make perf-search`).
