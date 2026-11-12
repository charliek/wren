# Testing

## Gates

```bash
make check                              # fmt, clippy (normal, smoke-test, wren-editor alone), Rust tests, Python units
ICED_BACKEND=tiny-skia make smoke-x11   # real-window functional suite under Xvfb
make smoke-wayland                      # same suite on headless Weston, minus X11-native cases
uv run --locked --group docs zensical build --strict # the docs site, for changes under docs/
```

Every commit on `main` builds and passes these. CI's aggregate `ci-success`
check covers the Rust jobs (Linux and macOS), the harness units, and four
Linux smoke lanes: X11 and Wayland, each with wgpu and tiny-skia.

!!! warning
    Never run a normal `cargo build` while a smoke suite is running. It
    replaces `target/debug/wren`, the smoke-test build, and every later launch
    in that run fails with "unsupported or duplicate option".

## Where tests live

| Kind | Location |
|---|---|
| Unit tests | Inline `#[cfg(test)] mod tests` next to the code. |
| Filesystem integration | `crates/wren-core/tests/`. |
| Functional (real window) | `tools/wrentest/`, driven over a private test socket. |
| Harness unit tests | `tools/wrentest_unit/`. |
| Release workloads | `make perf-search`: 5,000 notes / 25 MB, ignored in normal runs. |

The functional harness launches its own `wren` built with
`--features smoke-test`, in temporary workspace and config directories. It
never attaches to or kills a normal instance. Cases marked `native_x11` use
real OS keyboard and pointer input and run only in the X11 lanes.

`WrenApp(settings="…")` writes that text to the sandbox's `config.conf`
before the first launch, so a case can start Wren with any setting. Restarts
keep whatever the app has written there since, and `app.settings_path` names
the file.

### Editor engines

The editor cases in `tools/wrentest/test_editor.py` and
`tools/wrentest/test_markdown_keys.py` run on each text engine
that `WRENTEST_ENGINE` names: `classic` (the default), `advanced`, or both,
comma-separated. Each case starts Wren with `editor-engine` set to its engine,
checks that `editor.get` reports that engine, and gets the engine in its test
id:

```bash
cargo build --features smoke-test
ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced \
  uv run --locked --group test python tools/run_display.py x11 -- \
  uv run --locked --group test pytest tools/wrentest/test_editor.py
```

`test_markdown_keys.py` drives the Markdown-aware keys with the
`editor.command` request, `{"intent": "enter"}` and the like (`shift_enter`,
`tab`, `shift_tab`, `backspace`, `delete`, `delete_word_back`,
`delete_word_forward`, `delete_to_line_start`, `home`, `select_home`, …). It
reaches the app's edit path the same way the editor widget's keys do and
returns the editor snapshot; its `native_x11` case presses the real keys.
The Markdown commands run two ways: `command.run` (`{"id": "markdown.bold"}`)
runs one as the palette does, at the editor's cursor whatever has the
keyboard, and `shortcut` runs one as its key does, routing the bound key
through the editor's key policy and acting only while the editor is
focused. The settings commands `editor.toggle_engine` and
`editor.toggle_markdown_assist` run through `command.run` too, and each
palette row reports the value such a command flips as its `state`.

`perf.view` returns how long each recent `App::view` took to build
(`view_micros`, the last 256, in microseconds; `{"clear": true}` empties the
list after reading it). `test_markdown_keys.py` opens a group of three
5,000-line notes and holds the median view build under 5 ms at idle and
after smart keys, and each smart key's handler under 400 ms, both generous
ceilings for a debug build on CI. Work done on every frame, such as
re-reading every note's preview, shows up there first.

The `make smoke-*` targets pass the variable through, so
`WRENTEST_ENGINE=advanced make smoke-x11` runs the whole suite with the editor
cases on the advanced engine. CI sets `WRENTEST_ENGINE=classic,advanced` on all four Linux
smoke lanes.

### Boards

Boards are behind the `boards` setting, which is off by default. The board
cases live in `tools/wrentest/test_board_*.py` and set the setting
themselves; `tools/wrentest/boards.py` holds their helpers. Each copies the
sample workspace, `tools/wrentest/fixtures/boards/workspace/`, before opening
it, because opening a workspace writes `.wren/workspace.toml` into it. A board
with git conflict markers is written at run time by `conflict_board()` and is
never committed, and large boards come from `grid_board(n)`.

| File | What it covers |
|---|---|
| `test_board_view.py` | Focus and the editor column, the opening fit, pixel checks of fills, connectors and labels, right-to-left labels, a connector millions of units long at every zoom, wheel and drag panning, zoom, the raw-text fallback, search, and SVG export. |
| `test_board_gate.py` | The setting off, a board never written by looking at it, the hidden editor never writing a board, and the state file's board paths. |
| `test_board_native.py` | Real wheel, Ctrl+wheel and middle-button input under X11 (`native_x11`). |
| `test_board_perf.py` | Timings against debug ceilings, written to `measurements.json` beside the case's artifacts, and an idle second with no parse, scene build or geometry build. |

Run them alone with a path glob (`-k board` would also pick every keyboard
and clipboard case):

```bash
make build-smoke
ICED_BACKEND=tiny-skia make smoke-x11 SMOKE_TESTS='tools/wrentest/test_board*.py'
make smoke-wayland SMOKE_TESTS='tools/wrentest/test_board*.py'
```

`WRENTEST_BOARDS=off` or `on` writes that `boards` line into the
`config.conf` of every `WrenApp` whose settings do not name `boards`
already, so the whole suite can run with the setting on. Unset, nothing is
added and the build's default holds.

`WRENTEST_PERF=release` makes `test_board_perf.py` assert the release
budgets instead of the debug ceilings. Build the release binary in its own
target directory so the smoke build is not replaced:

```bash
cargo build --locked --release -p wren-iced --features smoke-test --target-dir target/perf
WRENTEST_PERF=release WREN_TEST_BIN=$PWD/target/perf/release/wren ICED_BACKEND=tiny-skia \
  python3 tools/run_display.py x11 -- \
  uv run --locked --group test pytest tools/wrentest/test_board_perf.py -s
```

`test_board_gate.py` has one case that opens a workspace with a released
Wren and with this build in turn. It runs only when `WRENTEST_RELEASED_BIN`
names a v0.1.1 build made with `--features smoke-test`.

The board requests:

| Request | What it does |
|---|---|
| `board.get` `{"elements": bool}` | The open note's surface (`board`, `raw`, `text` or none), whether the setting is on, and for a board its counts, issues, viewport, whether it is read-only and whether the widget is panning, idle or unfocused. With `elements` it lists every node and connector by their file fields. For raw text, `error` names the line, column and message. |
| `board.bounds` | Where the widget and each node, label and connector are drawn in the window, in logical pixels, with each element's fill and stroke colour and the window's scale factor. |
| `board.pointer` | A wheel step (`{"kind": "wheel", "at": …, "delta": {"x", "y", "unit"}, "mods": ["mod", "shift"]}`) or one step of a middle-button pan (`press`, `move`, `release` with `"button": "middle"`), through the path the widget's own input takes. `at` is `{"node": id}`, `{"canvas": [x, y]}` in board units, or `{"screen": [x, y]}` in the window. |
| `board.export` `{"format": "svg", "path": "/abs/out.svg"}` | Writes what Export board as SVG writes, without the save dialog, and refuses the paths it refuses: a note's, and any but an `.svg` inside the workspace. |
| `perf.board` | The board's scene, draw and frame timings, how long it took to open, and how many times the view has parsed, built its scene and built geometry, optionally after a pan or zoom run of one step a redraw. |

Every board request but `board.get` fails when the open note is not shown
as a board.

## Looking at real screens

Build with `--features smoke-test`, then run `tools/capture_scenes.py`
(under `tools/run_display.py x11 -- …` on Linux). It writes one PNG per scene
to `target/scenes/<theme>/`, or with `--engine advanced` to
`target/scenes/advanced/<theme>/`. The `markdown` scene shows a note with
every construct the editor highlights and a selection across blank lines;
the `headings` scenes show every heading level, one wrapping, with the caret
and then a selection on it; and each has a `-wide` twin with the browser
columns folded, where the editor's column is centred. The `hanging-indent`
scene shows long wrapped list items, nested and ordered items, tasks,
quotes, and a heading inside a list item, with a selection across wrapped
rows; only the advanced engine hangs them. Check them on both engines in
both themes after changing the highlighter or the editor's drawing.
`--setting key=value` (repeatable) starts Wren with more settings and
`--label` files the PNGs under a directory of their own; `--only markdown`
shoots just these editor scenes:

```bash
ICED_BACKEND=tiny-skia python3 tools/run_display.py x11 -- \
  python3 tools/capture_scenes.py --engine advanced --theme light \
  --setting editor-line-height=2 --label tall --only markdown
```

`--only boards` opens the board samples with `boards = on` and writes the
board scenes: `board-flow` (fitted), `board-zoom-in` (200 %),
`board-zoom-out` (25 %), `board-shapes`, `board-overlap`, `board-raw` (a
board with conflict markers, shown as text under its banner),
`board-notes-list` (the kind icon), `board-search` (a label hit) and
`board-wide` (columns folded), with `board-flow.svg` beside them. Shoot
them on both renderers in both themes:

```bash
for r in tiny-skia wgpu; do for t in dark light; do
  WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1 ICED_BACKEND=$r python3 tools/run_display.py x11 -- \
    python3 tools/capture_scenes.py --only boards --theme $t --label $r
done; done
```

## macOS

Rsync the tree to a Mac (excluding `target/`, `.venv/`, and `.claude/`) and
build there. `capture_scenes.py` works over ssh against a real Metal window
when a user is logged in at the console. Synthetic keystrokes through
`osascript` need an Accessibility grant that ssh sessions don't have.

For updater changes: `cargo build --features smoke-test`,
`third_party/sparkle/fetch.sh`, then `python3 tools/check_updater.py`.
