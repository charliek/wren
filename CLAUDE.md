# Wren — Project Conventions

Wren is a structured Markdown notes app for macOS and Linux: plain `.md`
files in a fixed *Workspace → Section → Group → Note* hierarchy, one open
document at a time, and autosave. `docs/` (published with Zensical to
https://charliek.github.io/wren/) describes current behavior for users and
contributors. `discovery/` holds designs that are planned or in progress;
when one lands, move what users need into `docs/` and trim the note.

## Layout

- `crates/wren-board/`: boards (`Name.wren-board`), the second note kind,
  being built on the `boards` branch to `discovery/boards.md` (§3.1 is the
  module plan, §4 the file format's only contract). Toolkit-neutral, with
  no iced or Wren dependency (only `toml` and `unicode-segmentation`).
  `lib.rs` holds the model (`Board`, `Node`, `Edge`; unknown content kept
  as `toml` values) and the theme's `Palette`; `scene.rs` holds the display
  list, built from a board, a `Palette` and a text `Measure`, that both the
  canvas and the SVG writer draw, so the two cannot disagree.
  `FixedMeasure` is the font-free measurer for tests. Full-count property
  tests and timings are ignored tests named `fuzz_*` and `perf_*`, run by
  `make fuzz-board` and `make perf-board`.
- `crates/wren-canvas/`: the board widget, kept free of Wren types as
  `wren-editor` is. It must not depend on `wren-core` or know app messages;
  the host passes in a `Palette`, a style, the font and a key policy. It
  paints a `wren_board::scene` through iced's geometry API as a custom
  advanced widget (not `canvas::Program`), and its interaction machine is a
  pure function. Test-only accessors sit behind its `test-support`
  feature, which `wren-iced/smoke-test` enables.
- `crates/wren-core/`: toolkit-neutral core, with no iced dependency.
  `ops.rs` holds the ordered `Session`, the one owner of every mutation.
  The rest: `scan.rs`/`reconcile.rs` (disk → model, with reconciliation off
  the UI thread), `fs_store.rs` (descriptor-relative, no-follow storage),
  `autosave.rs`, `index.rs` (search), `config.rs` (the app-managed workspace
  registry, `config.toml`), `settings.rs` (the user-owned `config.conf`),
  `keybind.rs` (triggers and platform default bindings), and `theme.rs`
  (built-in palettes).
- `crates/wren-editor/`: the Markdown editor widget, kept free of Wren
  types so it can be extracted later. It must not depend on `wren-core` or
  know app messages; the host passes in a `Style`, a `Palette`, and the font.
  `lib.rs` holds `EditorView` and `Message`; `widget.rs` is a fork of iced's
  text editor with the gutter, current line, and caret; `backend/` holds
  the two text engines behind `Content` (`iced.rs`, iced's own editor, and
  `owned.rs`, an owned cosmic-text editor drawn with `fill_raw`), and the
  editor tests run on both; plus `edit.rs` (range edits and the line
  endings their breaks take), `plain.rs` (the keys' plain plans while
  Markdown assist is off), `highlighter.rs` (spans and per-line
  decorations from `wren_markdown::block`), `history.rs` (snapshot
  undo), `find.rs`, and `keys.rs` (the editor's own shortcuts). Test-only
  accessors sit behind its `test-support` feature, which
  `wren-iced/smoke-test` enables.
- `crates/wren-iced/`: the desktop app (binary `wren`).
  - `app.rs` holds `App`, `Message`, `update`, `view`, and `subscription`;
    its submodules live in `app/`: `commands.rs` (the command table and
    gateway), `palette.rs`, `preferences.rs` (settings, keymap, and theme),
    `workspace.rs` (loading and core events), `workspace_view.rs`, and
    `navigation.rs`.
  - `theme.rs` maps a `wren_core` theme to iced's and to the editor's
    `Style` and `Palette`.
  - Widgets: `panes.rs` (resize), `drag*.rs` (drag and drop),
    `ellipsis.rs`, and `keys.rs`.
  - `macos_menu.rs` is the native menu bar.
- `crates/wren-markdown/`: Markdown structure and editing rules, with no
  iced or Wren dependency (only `unicode-segmentation`), so it can be
  extracted later. `block.rs` is the line classifier and container scanner,
  `outline.rs` the heading outline, and `edit.rs` plus `edit/` plan each
  editing `Intent` as an `EditPlan` of replacements. `block` is Wren's one
  classifier: the editor's highlighter and the search index's previews use
  it.
- `crates/wren-sparkle/`: the macOS Sparkle updater bridge (`dlopen` plus
  Objective-C calls), the only crate allowed `unsafe`. It is empty on Linux.
- `vendor/`: patched copies of crates.io crates (`cosmic-text`, `iced_winit`,
  `swash`), wired in by the root `[patch.crates-io]`; `make check-patches`
  asserts they are the ones built. `cosmic-text` and `swash` each have a
  `WREN.md` describing the change; `wren-editor`'s hanging indent needs the
  cosmic-text one.
- `docs/`: the documentation site (`zensical.toml` at the root). Keep it
  in step with behavior changes: keybindings, settings, and on-disk rules.
- `discovery/`: working design notes, deliberately outside `docs/` because
  everything under `docs/` is published.
- `tools/`: the Python functional harness (`wrentest/`, which drives a real
  window over a private test socket), its unit tests (`wrentest_unit/`),
  packaging (`package.py`), `capture_scenes.py` for visual review,
  `check_updater.py` (the Sparkle check on a Mac), and `update_appcast.py`
  (the release's appcast entry).

## Architecture rules

- Every mutation goes through `Session` ops. Clicks, drags, palette
  commands, shortcuts, menu items, and tests all reach the same command
  path (`App::dispatch(Command::Run(id))`). The UI never keeps its own copy
  of the truth.
- Keybindings come from `wren_core::keybind` plus the user's `keybind`
  lines. Command keys (`note.new`, `palette.commands`, …) are the stable
  names for config, the palette, menus, and the test protocol. To add a
  command, add a `CommandId` and a `definitions!` row, and handle it in
  `run_command`. A row's scope is `App`, or `Editor` for a command whose key
  only the focused editor runs (through the `KeyPolicy` built from the
  keymap); `App::key_input` ignores Editor keys.
- macOS uses Command as the app modifier; Linux uses Alt, leaving Ctrl to
  text editing. `keys::alt_filter` drops the text of Alt presses on Linux
  so shortcuts never type their letter.
- Autosave is a primary reliability requirement. Every note or workspace
  transition, and quit, flushes first, and a failed flush blocks the
  transition and keeps the draft.

## Style

- Flat modules and concrete types. No `Manager`/`Service`/`Helper` names.
- Errors are returned, not logged and swallowed.
- Unit tests live inline in `#[cfg(test)] mod tests`; filesystem
  integration tests go in `crates/wren-core/tests/`.
- Comments only for a non-obvious *why*. No committed `TODO`s.
- `unsafe_code` is forbidden workspace-wide; macOS integration uses safe
  crates (`muda`). The one exception is `crates/wren-sparkle`, because
  Sparkle has no safe binding. Keep its Objective-C surface minimal, and give
  every `unsafe` block a `SAFETY:` comment (clippy enforces it).

## Verifying changes

```sh
make check                       # fmt, clippy (normal + smoke-test), all Rust tests, Python units
ICED_BACKEND=tiny-skia make smoke-x11   # real-window functional suite under Xvfb (~2.5 min)
make smoke-wayland               # same suite on headless Weston, minus X11-native cases
uv run --locked --group docs zensical build --strict  # docs site; run for changes under docs/
```

- **Never run a normal `cargo build` while a smoke suite is running.** It
  replaces `target/debug/wren` (the smoke-test build), and every later
  launch in that run fails with "unsupported or duplicate option".
- To look at the real screens, build with `--features smoke-test`, then run
  `tools/capture_scenes.py` (under `tools/run_display.py x11 -- …` on
  Linux). It writes one PNG per scene to `target/scenes/<theme>/`
  (`--engine advanced` writes to `target/scenes/advanced/<theme>/`).
- macOS over ssh: rsync the tree (excluding `target/`, `.venv/`,
  `.claude/`) to a Mac and build there. `capture_scenes.py` works over ssh
  against a real Metal window when a user is logged in at the console.
  Synthetic keystrokes through `osascript` need an Accessibility grant
  that ssh sessions don't have.
- Updater changes: on a Mac, `cargo build --features smoke-test`,
  `third_party/sparkle/fetch.sh`, then `python3 tools/check_updater.py`.

## Branches and commits

Work lands directly on `main` in small verified commits, and each commit
must build and pass the gates above. CI's aggregate `ci-success` check
covers the Rust jobs (Linux and macOS), the harness units, and all four
Linux display/renderer smoke lanes.
