# Testing

## Gates

```bash
make check                              # fmt, clippy (normal, smoke-test, wren-editor alone), Rust tests, Python units
ICED_BACKEND=tiny-skia make smoke-x11   # real-window functional suite under Xvfb
make smoke-wayland                      # same suite on headless Weston, minus X11-native cases
uv run --locked --group docs zensical build --strict # the docs site, for changes under docs/
make fuzz-board                         # board format property tests at full count, in release
make perf-board                         # board parse, write, scene and edit timings, in release
```

Run `make fuzz-board` and `make perf-board` after changing `wren-board`.
`make perf-board` asserts release budgets, so run it on a quiet machine: a
loaded one misses them. `tools/board_spike.py` looks at and times the board
viewer in a real window by hand, through `wren --board-spike N` and the
`spike.board` request; its docstring has the commands. `make check` and `cargo build` without
`--features smoke-test` replace the smoke binary too, so never run them
during a smoke suite.

Every commit on `main` builds and passes these. CI's aggregate `ci-success`
check covers the Rust jobs (Linux and macOS), the harness units, and four
Linux smoke lanes: X11 and Wayland, each with wgpu and tiny-skia.

One headless Weston serves a whole Wayland run. Weston 13 has no
server-side decorations, so winit draws the title bar itself, and Weston's
pixman renderer can crash (SIGSEGV) compositing it after it is redrawn soon
after launch, as a theme switch from light to dark does. So under that
Weston the app runs without decorations, as it does under Xvfb:
`tools/run_display.py` sets `WRENTEST_DECORATIONS=off`, and the harness
passes `WREN_TEST_DECORATIONS=off` (the `smoke-test` build only) to each
launch. Set `WRENTEST_DECORATIONS=on` to keep them.

If Weston still dies, it logs nothing. `tools/run_display.py` then prints
`run_display: Weston exited with signal …` with the old and new logs and
starts it again on the same socket, up to three times, so the crash fails
the case it interrupts and not every case after it. An app launched
meanwhile waits until Weston accepts connections again. If it cannot be
started again, the run stops and fails.

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
| Release workloads | `make perf-search`: 5,000 notes / 25 MB, ignored in normal runs. `make fuzz-board` and `make perf-board`: `wren-board`'s ignored `fuzz_*` and `perf_*` tests. |
| Board golden files | `crates/wren-board/golden/`: `format/` (the bytes of schema 1, pinned by `lock.rs`), `scene/`, `svg/`, `drawio/`, and `accept/` (the keyboard script's layout). `WREN_BLESS=1 cargo test -p wren-board` rewrites them; a rewrite of `format/` fails `lock.rs`, which pins the length and digest of every file there, since schema 1 never changes. |
| `wren export` | `crates/wren-iced/tests/export_cli.rs` runs the built binary with no display (`DISPLAY`, `WAYLAND_DISPLAY`, and `WREN_TEST_MODE` removed): every option, every exit status, nothing written on failure, and the bytes of `tests/golden/checkout-flow.svg`, `checkout-flow.drawio`, and `coverage.drawio`. |

The functional harness launches its own `wren` built with
`--features smoke-test`, in temporary workspace and config directories. It
never attaches to or kills a normal instance. Cases marked `native_x11` use
real OS keyboard and pointer input and run only in the X11 lanes.

`WrenApp(settings="…")` writes that text to the sandbox's `config.conf`
before the first launch, so a case can start Wren with any setting. Unless
the text sets `spell-check` itself, the harness appends the lane's
`spell-check` line (see [Spell check](#spell-check) below), and without
settings the file holds only that line. Restarts keep whatever the app has
written there since, and `app.settings_path` names the file.
`WrenApp(words="…")` writes `words.txt` beside it the same way, and
`app.words_path` names that file.

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

Every other case runs on one engine, the first that `WRENTEST_ENGINE`
names (`classic` when unset): the harness writes that `editor-engine` line
into the `config.conf` of every `WrenApp` whose settings do not name
`editor-engine`, as it does for `spell-check`, since the build defaults to
`advanced`. The `make smoke-*` targets pass the variable through, so
`WRENTEST_ENGINE=advanced make smoke-x11` runs the whole suite on the
advanced engine. CI sets `WRENTEST_ENGINE=classic,advanced` on all four
Linux smoke lanes, which runs the editor cases on both engines and the rest
on `classic`.

### Command key

On Linux the build's default command key is Ctrl, but the harness pins
`linux-modifier = alt` in every case's `config.conf`, so the cases written
with Alt shortcuts mean the same thing whatever the default is. A case that
names `linux-modifier` itself keeps its own value. `WRENTEST_LINUX_MODIFIER`
sets the pin for a run:

```sh
WRENTEST_LINUX_MODIFIER=ctrl ICED_BACKEND=tiny-skia make smoke-x11
```

It takes `ctrl` or `alt` (`alt` when unset), and anything else fails the
run. macOS ignores the setting. CI runs one lane (X11, wgpu) with it `ctrl`,
the shipped default, and the others with `alt`.

Cases name keys with the logical modifiers instead of ⌘, Ctrl or Alt.
`editor.key` and `board.key` take `mod` (the command key: ⌘ on macOS, Ctrl or
Alt on Linux as the lane says) and `opt` (the board's second key: ⌥, or the
other of Ctrl and Alt) in `mods`, next to `shift`, `ctrl`, `alt` and `logo`.
For native X11 keys and for labels, `tools/wrentest/ui.py` has three helpers:

| Helper | Gives |
|---|---|
| `native_mod(key)` | An `xdotool` chord of `key` on the lane's command key, such as `alt+Return`. |
| `native_opt(key)` | The same on the board's second key: Alt with Ctrl as the command key, Ctrl with Alt. |
| `mod_label()` | How the app writes the command key in a label: `⌘` on macOS, `Ctrl+` or `Alt+` on Linux. |

### Spell check

`WRENTEST_SPELL` sets the `spell-check` setting the harness writes for
every case that does not name it: `off` (the default when unset, so sample
notes full of names keep their look) or `on`. Any other value fails the
case. A case that needs no settings file at all, as a fresh install has
none, passes `WrenApp(lane_settings=False)`, which adds none of the lane's
lines (this one, `boards`, `tasks`, `editor-engine` or `linux-modifier`);
it then has the build's defaults, tasks on and the advanced engine among
them. Spell check runs only on the advanced engine,
so `on` matters only to the cases `WRENTEST_ENGINE` runs there:

```bash
WRENTEST_SPELL=on ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11
```

`tools/wrentest/test_spell.py` sets `spell-check` itself. It runs the
acceptance note `tools/wrentest/fixtures/spell.md` with `catppuccin` in the
word list, and checks its marks, the menu, replacing a word, the toggle,
the word list, the classic engine, and a board shown as raw text, which is
never checked. Its `native_x11` cases right-click
and press the real suggest key. These requests drive it:

| Request | Parameters | Returns |
|---|---|---|
| `editor.spell` | none | `enabled` (the setting on the advanced engine), `ready` (the dictionary has loaded), `marks` as the last frame drew them (none while a board is shown, since the editor draws nothing then), and `menu`, or `null` when it is closed. |
| `editor.spell_event` | `event`: `open_at_caret`, `move`, `hover`, `accept`, or `close`; `arg`: a row for `hover` and `accept`, a step for `move` | The `editor.get` snapshot, after the event went through the app as the widget would send it. |
| `editor.key` | `key`: a name (`up`, `down`, `left`, `right`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `home`, `end`, `shift`, `control`, `alt`, `super`, `caps_lock`) or one character; `mods`: a list of `shift`, `ctrl`, `alt`, `logo`, `mod`, `opt` | The `editor.get` snapshot, after the editor's own key routing and then the app's key handling, as a real press in the focused editor. |
| `editor.right_click` | `line`, `column`: the byte offset in the line of the character to click on, or the line's length for the end of its text | The `editor.get` snapshot, after a right-click on that character went through the app as the editor would send it: on a misspelled word, the menu opens, and with a menu open the click only closes it. A click nothing answers changes nothing. |
| `editor.click` | `line`, `column`, as for `editor.right_click` | The `editor.get` snapshot, after a left click, pressed and let go on that character, went through the app: on a task's box (tasks on, the advanced engine) it cycles the task's status, and anywhere else it places the caret. |
| `editor.bounds` | `line`, `start`, `end`: a byte range of the line | Where the range is drawn, `x`, `y`, `width` and `height` in window coordinates, for a native click on it; an error while it is not drawn. |

Each mark has its `line`, its `start` and `end` byte offsets in the line,
its `word`, whether it is `shown` (the caret at its end hides it), and the
`rects` of its zigzag in window coordinates. The menu has the `word`, its
`line`, `start`, and `end`, the `items`, the `selected` row, and its
`bounds` in window coordinates once a frame has drawn it, with `anchor`,
the rectangle of the word's glyphs on their row that the menu hangs under,
as the editor reports it to the app. A frame must be
drawn before marks appear, so poll `editor.spell` with `wait_for`. The
`shortcut` request runs `editor.spell_suggest` as its key does, and
`command.run` runs it as the palette does.

### Boards

Boards are behind the `boards` setting, which is on by default. The board
cases live in `tools/wrentest/test_board_*.py` and set the setting
themselves; `tools/wrentest/boards.py` holds their helpers. Each copies the
sample workspace, `tools/wrentest/fixtures/boards/workspace/`, before opening
it, because opening a workspace writes `.wren/workspace.toml` into it. A board
with git conflict markers is written at run time by `conflict_board()` and is
never committed, and large boards come from `grid_board(n)`.

| File | What it covers |
|---|---|
| `test_board_view.py` | Focus and the editor column, the opening fit, pixel checks of fills, connectors and labels, right-to-left labels, a connector millions of units long at every zoom, wheel and drag panning, zoom, the raw-text fallback, search, and SVG export. |
| `test_board_gate.py` | The setting off, the setting on with no settings file, a board never written by looking at it, the hidden editor never writing a board, and the state file's board paths. |
| `test_board_native.py` | Real wheel, Ctrl+wheel and middle-button input under X11 (`native_x11`). |
| `test_board_dialog.py` | The New note dialog's kind pills. |
| `test_board_edit.py` | Creating, moving, resizing, labels, connectors, delete, duplicate, the clipboard and undo, each checked as a line diff of the saved file. |
| `test_board_keys.py` | The board's key policy, the Escape ladder, keys in a label and focus. |
| `test_board_safety.py` | A label in the draft from its first keystroke, the settle before every transition, the single writer, and nothing written by a sequence that changes nothing. |
| `test_board_accept.py` | The keyboard-only flowchart of 232 presses, checked against the layout in `golden/accept/`. |
| `test_board_style.py` | Colour, fill and outline commands, one line per element, a fill pixel, and the digit keys under X11. |
| `test_board_arrange.py` | Stacking, align and distribute as line diffs, and the actions palette: its rows, its preview never reaching the file, and right-clicks. |
| `test_board_find.py` | The jump palette and where a search hit lands, with the file's bytes and mtime unchanged. |
| `test_board_export.py` | PNG and draw.io exports, `wren export` with no display against the app's bytes, and a window that keeps answering while a large PNG renders. |
| `test_board_limits.py` | The 8 MiB and 50,000-element caps opening as raw text, and values past each drawing limit listed as issues and never written. |
| `test_board_perf.py` | Timings against debug ceilings, written to `measurements.json` beside the case's artifacts, and an idle second with no parse, scene build or geometry build. |
| `test_board_menu.py` | The macOS menu bar's rows, shortcuts and enabling through `menu.get` and `menu.run`, on every platform. |

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
| `board.get` `{"elements": bool}` | The open note's surface (`board`, `raw`, `text` or none), whether the setting is on, and for a board its counts, issues (each with its `kind`, its element's `id` and, for `out_of_range`, the `limit` passed: `coordinates`, `size`, `anchor`, `label_at`, `label_bytes`, `labels`, `z_length`, `text_size` or `label_lines`; otherwise `null`), viewport, `fit_pending` (a fit still waiting for the widget's size, while the viewport is the board's origin at 100 %), whether it is read-only, the machine's state (`unfocused`, `idle`, `armed`, `pressed`, `moving`, `marquee`, `resizing`, `creating`, `connecting`, `editing`, `panning`), the armed tool and whether it is `locked`, `next_color` (the colour token new shapes take), the `pointer` the board shows for its state (`arrow`, `grab`, `grabbing`, `crosshair`, `text`, `copy` or `resize`; the I-beam over an open label is the widget's and is not reported), the selection and cursor, the open label's text and caret (with `ime`, the caret the input method is given, in the canvas, as `x`, `y` and `height`, and the `size` its composition is shown at), `connecting` (while a connector end is dragged, the node a release would attach it to, the side it would pin it to and an empty `query`; in keyboard Connecting, the node Enter would connect to, a `side` of `null` and the query typed so far; otherwise `null`), whether there is anything to undo or redo, whether the board still writes as it was opened (`pristine`) and `revision`, the footer's `hint` (each part's `keys` and `words`, empty while the board is unfocused or with `board-hints = off`), `keys_sheet` (while the keys sheet is open, its `query` and `rows`, each with its `group`, `keys`, `words`, and whether it is `dimmed`; otherwise `null`), the count of its drafts and reloads, and `export`: `waiting` while an export's save dialog is open, `writing` while its file is rendered and written, otherwise `null`; and the window's `banner`, as `session.get` gives it, for a board whose text is too large for `session.get`'s frame. With `elements` it lists every node and connector by their file fields. For raw text, `error` names the line, column and message. |
| `board.bounds` | Where the widget and each node, label and connector are drawn in the window, in logical pixels, with each element's fill and stroke colour and the window's scale factor; the handles shown (`nw` to `w` on the one selected node, `connect_top` to `connect_left` on the node the pointer has rested on, `from` and `to` on the one selected connector, and `side_top` to `side_left` on the node a dragged connector end is over) and the open label's box. |
| `board.pointer` | A wheel step (`{"kind": "wheel", "at": …, "delta": {"x", "y", "unit"}, "mods": ["mod", "shift"]}`), a trackpad pinch (`{"kind": "pinch", "at": …, "delta": 0.25}`, the zoom multiplied by 1 + `delta`) or a button's `press`, `move`, `release`, `click` or `double_click` (`"button": "left"`, `"middle"` or `"right"`, `"mods": ["shift", "duplicate"]`), through the path the widget's own input takes. A press on a board without the keyboard first gives it the keyboard, as a real press does, and then presses. `at` is `{"node": id}`, `{"node": id, "handle": "se"}` (a resize handle `nw` to `w`, a connect handle `connect_top` to `connect_left`, or a side handle `side_top` to `side_left`, the middle of that side on the outline), `{"edge": id}` (the middle of its line), `{"edge": id, "handle": "from"}` (or `"to"`, one of its ends), `{"canvas": [x, y]}` in board units, or `{"screen": [x, y]}` in the window. A `move` with no button down is the pointer passing over the board: it reaches only a board that has the keyboard, and the connect handles of the node it rests on come up 120 ms later. |
| `board.pinch` `{"delta": 0.25}` | Records a trackpad pinch for the window as the vendored `iced_winit` records winit's `PinchGesture` on macOS (`vendor/iced_winit/WREN.md`), on any platform. The window's next frame gives it to the board if the real pointer is over the board then, and drops it if not, so a test places the pointer first (the native X11 case moves it with `xdotool`). Answers `{"recorded": delta}` at once; the zoom changes with that frame. |
| `board.key` `{"key": "tab", "mods": ["mod", "shift"], "repeat": false}` | A key pressed on the focused board, through the board's key policy and then the app's keyboard path, as a real key goes; `mod` is the lane's command key (⌘ on macOS, Ctrl or Alt on Linux) and `opt` the board's second key. A letter with `shift` types its capital and is named, as a real press names it, by the letter without its modifiers. Answers the snapshot with `handled`, whether the board took the key; with a palette or dialog open, or another widget focused, nothing happens. `{"key": "space", "release": true}` lets go of a held Space, as the widget hears its release; Space alone is released. |
| `board.dock` `{"click": id}` | The board's tool dock: whether it is `shown`, whether its shapes are `folded` into one button, the bar's `rect`, its `buttons` in order (each with its `id`, such as `board-dock-rectangle`, `board-dock-more`, `board-dock-shapes` or `board-dock-keys`; the `command` it runs, or `null` for a menu button; whether it is `lit` and `locked`; its `key` from the keymap; the `badge` it shows now, or `null`; its `tip`; and its `rect`) and the open `menu` (its `kind`, `more` or `shapes`, its `rect`, and its `rows`, each with its `id`, `command`, `lit`, `locked`, `key` and `rect`), or `null`. Each `rect` is where the last frame drew it, in logical pixels, or `null` before one has. With `click`, a button's or a row's id, the message a click there sends goes first; an id the dock does not show is an error. |
| `board.style_bar` `{"hover": id, "click": id}` | The board's style bar: whether it is `shown`, whether it is the `new_shapes` row, whether its styles are `folded` into the last menu, the bar's `rect`, its `controls` in order (each with its `id`, such as `board-style-shape`, `board-style-red`, `board-style-fill-solid`, `board-style-text-size`, `board-style-route` or `board-style-more`; the `command` it runs, or `null` for a menu button; whether it is `current`; the `shape` the shape button draws, or `null` for several; its `key`, the `badge` it shows now, its `tip`, a swatch's `color` as `[r, g, b]`, and its `rect`), the open `menu` (its `kind`, `shapes`, `text_size` or `more`, its `rect`, and its `rows`, each with its `id`, `command`, `name`, `current`, `key` and `rect`), the command the pointer previews as `hover`, and the accent the current swatch is ringed in as `ring`. With `hover`, a control's or a row's id that previews, the pointer comes onto it first, and with `null` it leaves; with `click`, the message a click there sends goes next. An id the bar does not show is an error, and so is a `hover` on one that previews nothing. |
| `board.footer` `{"click": id}` | The controls in the footer under the board, `board-footer-undo`, `board-footer-redo`, `board-footer-zoom-out`, `board-footer-zoom` and `board-footer-zoom-in` (none with `board-toolbar = off`, and no undo or redo on a read-only board), each with the `command` it runs, whether it is `enabled`, its `key`, `tip` and `rect`; and the `zoom` the footer shows. With `click`, a control's id, the message a click there sends goes first; a greyed control sends only what closes the bars' menus. |
| `board.type` `{"text": "…"}` | Text typed on the focused board: the open label's, else the connect query's. |
| `board.apply` `{"edits": [...]}` | Edits with explicit ids (`add_node`, `move`, `set`, `connect`, `delete`), applied as one undo step and one draft, for arranging a board before what a test looks at. |
| `board.seed` `{"seed": 7}` | The open board draws the ids of the elements it makes from this seed from now on, so a test or a scene that builds a board makes the same one each run. Ids decide which of two connectors that would share a line moves aside. |
| `board.export` `{"format": "svg", "path": "/abs/out.svg"}` | Writes what Export board as SVG (`"png"`: as PNG; `"drawio"`: as draw.io) writes, without the save dialog, on the same worker thread, and answers once the file is written. It refuses the paths the command refuses (a note's, and inside the workspace any but a file of the format's extension, `.svg`, `.png` or `.drawio`), and any export while another is under way. |
| `spike.board` | Only under `wren --board-spike N`: shows a generated grid or given board text, and sets the label rule, the layer cap and the viewport, for `tools/board_spike.py`. |
| `perf.board` | The board's scene, draw and frame timings, how long it took to open, how long each draft took (the canonical write, and with the Session's op), and how many times the view has parsed, built its scene and built geometry, optionally after a pan or zoom run of one step a redraw. |

Every board request but `board.get` fails when the open note is not shown
as a board.

`menu.get` reports the macOS menu bar as its table builds it with the
setting the app runs with: each menu's `title`, `kind` (`plain`, `board` or
`window`) and `items`. A command item has its `key`, `title`, the `trigger`
a Mac would show for this keymap and whether it is `enabled`; AppKit's own
rows are `{"native": name}`, separators `{"separator": true}` and submenus
`{"submenu": title, "items": [...]}`. The reply's top-level `native` lists
each Board item's key and whether AppKit holds it enabled on macOS, and is
`null` elsewhere. `menu.run` `{"key": "board.duplicate"}` sends an item's
key back as choosing it does. Both work on every platform, so the Board
menu's enabling and the path an item takes are tested on Linux too.

`navigation.get` reports `index_unextracted`: how many boards the search
index still holds by their title alone, waiting for their labels to be read
off the UI thread after a workspace opens. A search from the palette reads
them first, so the count only shows how far the background read has got.

`test.clipboard` `{"own": true}` gives the app a clipboard of its own, which
`clipboard.write`, `clipboard.read` and the board's copy, cut and paste then
use. Headless Weston gives the app no clipboard, so the paste case turns it
on there and still tests the board's copy and paste; `{"own": false}` goes
back to the system clipboard.

### Tasks

Every task case writes its own `tasks` and `editor-engine` lines, takes the
engine `WRENTEST_ENGINE` names, and pins today to Thursday 19 November
2026 with `WrenApp(today="2026-11-19")`, which sets `WREN_TEST_TODAY` on
every launch (a `YYYY-MM-DD` date; the `smoke-test` build only). Dates in
a case are written relative to that day. The cases are
`tools/wrentest/test_tasks_*.py`, and `tools/wrentest/fixtures/tasks/` is
their sample workspace. A feature that only the advanced engine has is
checked absent on classic, and with tasks off.

| File | What it covers |
|---|---|
| `test_tasks_gate.py` | Tasks off: no task command or key, every `tasks.*` request refused, or answering `{"tasks": false}` for the plain `tasks.get`, `tasks.view`, `tasks.entry` and the view's `write`, `hide` and `undo`, and the pinned date reaching the app with tasks on. |
| `test_tasks_baseline.py` | The baseline note with tasks off: Enter, Backspace, toggle, highlighting, previews and spelling as before the feature. `make check` pins the same note in `crates/wren-iced/golden/tasks-baseline.txt`. |
| `test_tasks_emoji.py` | The task glyphs: caret steps, one-press Backspace, selection and hit-testing. |
| `test_tasks_status.py` | The statuses of the box, and the toggle's stamps. |
| `test_tasks_colours.py` | Which role each token is drawn in, and the day change. |
| `test_tasks_footer.py` | The status-bar line's full text and pieces; `capture_scenes.py --only tasks` draws its narrow, cutoff and wide layouts. |
| `test_tasks_commands.py` | The Tasks palette group, its keys, and the Editor-scope routing. |
| `test_tasks_menus.py` | The status, priority and task menus, the box click, and right-clicks on each token, including native X11 input. |
| `test_tasks_calendar.py` | The calendar. |
| `test_tasks_suggest.py` | Suggestions, including a native X11 case that types `du` and Enter at xdotool speed. |
| `test_tasks_form.py` | The task form. |
| `test_tasks_write.py` | Writing a task from outside the editor: `tasks.view`'s `write`, `hide` and `undo` on an open and a closed note, an out-of-date row, and the front matter cases. |
| `test_tasks_view.py` | The view's surface: opening it, Back and Forward through it, the note under it untouched, and the state file unchanged. |
| `test_tasks_entry.py` | The ways in: the header counts (on and off), Show tasks from each column, the editor and a board, a row's actions, the hides and their marks, and the message with Undo. |
| `test_tasks_view_list.py` | The list: filters, grouping, order, the footer, the keys, the menus, and the filters remembered in `.wren/tasks.toml` across a restart. Its workspace is `fixtures/tasks/list/`. |
| `test_tasks_view_write.py` | Writes from a row: keys, clicks and menus, the message and its Undo, and rows held until `r`. |

`tasks.get` reports the task state, and with tasks off it, like plain
`tasks.view`, plain `tasks.entry` and the view's `write`, `hide` and `undo`,
answers `{"tasks": false}`. With them on it gives `today`, the date the
app last `seen` and how many times the day has changed (`day_changes`), the
`engine`, the caret's `line` (its number, its text and its task: the box's
byte range, symbol, status name and type, or `null`), its `tokens` (each
with its kind, text and byte range, the palette role it is drawn in and
whether it is bold), and the status bar's `footer` (its text, whether it is
overdue and its pieces). `tasks.set_today` `{"date": "YYYY-MM-DD"}` moves
the clock as midnight would, so the colours, the footer and the calendar
refresh. `tasks.owners` returns the owners suggestions would offer, each
with its note count, and whether the list is current for the workspace.
Any other `tasks.*` request is refused with tasks off.

`editor.open_menu`, `editor.click`, `editor.bounds` and `editor.right_click`
drive the popovers and clicks; `editor.cursor` places the caret.
`editor.command` `{"intent": "toggle_task"}` runs the toggle as the keys do:
with tasks on it writes the done date when it completes a task and removes
the done or cancelled date when it reopens one, and with tasks off it is the
plain toggle and writes none. The toggle never cancels a task: the cancelled
date is written by a change of status to Cancelled (`task.set_status`, the
status menu, the form). The `native_x11` cases use real clicks
and chords.

`WRENTEST_TASKS=off` or `on` writes that `tasks` line into every
`config.conf` whose settings do not name `tasks`. Unset or empty means
`off`, as `WRENTEST_SPELL` does, because the build defaults to on: a lane
that does not ask for tasks runs with them off. CI runs one lane (X11,
tiny-skia) with it `on`; the others pass it empty, so they run with tasks
off. `make
smoke-tasks` runs `tools/wrentest/test_tasks_*.py` on both engines, once with
tasks `off` and once `on`, for `DISPLAY_BACKEND` (default `x11`) and the
current `ICED_BACKEND`, and fails if either run ran no task test (none collected, or all skipped).

The task line commands (`task.cycle_status`, `task.priority.high` and the
rest) run through `command.run` as the palette runs them, at the caret, and
through `shortcut` once a `keybind` line gives one a key. `editor.get`
reports the editor's open popover, whichever it is, as `popover`: its
`mode` (`menu` or `suggest`), whether it is the `spelling` menu, the
`line`, `start`, `end` and `text` it hangs under, its `rows` (each with its
`id`, `label`, the `command` key the id names or `null`, its `glyph`, the
`key` named beside it, and whether it is `checked` and `enabled`), the
`selected` row, the `provider` that opened it for a right-click, its
`instance` number, and its `bounds` and `anchor` in window coordinates once
a frame has drawn it; `null` with none open. `editor.open_menu` `{"rows":
[{"command": "task.priority.high", "label": "High", "key": "h", "shift":
false}]}` opens a menu at the caret whose rows run those commands, on the
advanced engine only, so `editor.key` can press a row's key, Up, Down,
Enter or Escape into a menu of the app's as into the spelling menu.

The calendar (`task.set_due`, `tools/wrentest/test_tasks_calendar.py`)
opens through `shortcut` or `command.run`, and from a right-click on a due
date. While it is open, `editor.key` presses a key in its field instead of
the editor: the calendar's keys (also `page_up` and `page_down`), a
character typed or Backspace at the end of its text, and Escape and
shortcuts heard by the app as a real press is. `tasks.calendar` returns
`{"open": false}`, or its `line`, the field's `typed` text, the
`selected`, `today` and `current` dates, the `month` shown, the `caption`
under the field and whether it is `invalid`, whether Clear is shown
(`clear`), its `anchor` (`line` and byte `range`) and the card's `bounds`
in window coordinates. `tasks.calendar_type` `{"text": …}` sets the
field's text as a paste or an input method's commit does, and
`tasks.calendar_click` clicks `{"day": "YYYY-MM-DD"}`, a quick pick
`{"chip": "Fri"}`, or `{"target": …}`, one of `previous`, `next`, `clear`,
`inside` (the card off its controls) and `outside`. `focus.get` names the
field `calendar-field`. With tasks off each of them is refused.

The task form (`task.edit`, `tools/wrentest/test_tasks_form.py`) opens
through `shortcut` or `command.run`, on either engine. While it is open,
`editor.key` presses a key in it: the form's own keys (Tab, the arrows,
Enter, Escape, and on the status and priority rows their characters), a
character typed or Backspace at the end of the focused field's text, and
Escape and shortcuts heard by the app as a real press is; with the
calendar open over it, the calendar's field takes the key. `tasks.form`
returns `{"open": false}`, or its `kind` (`edit`, `new` or `make`),
`title`, `line`, the slot with the keyboard (`focus`), the fields
(`description`, `status`, `due`, `priority`, `owners`, and `owner_text`,
what the owners field holds), the due field's `due_caption` and
`due_invalid`, `due_value` (what the due field names), the owners `suggestions` and the chosen `suggestion`, the
`writes` line, the last save's `error`, the `calendar` over it (`typed`,
`selected`, `month`, `caption` and `bounds`) and the due field's
`due_bounds`. `tasks.form_type` `{"field": "description" | "due" |
"owners", "text": …}` sets a field's text, and `tasks.form_click` clicks
`{"status": name}`, `{"priority": name}` (`"None"` for none),
`{"suggestion": name}`, `{"remove": name}` (an owner's remove button),
`{"target": …}` (`save`, `cancel`, `outside` or `calendar`), or on the
calendar over the form `{"day": …}`, `{"chip": …}` or `{"calendar": …}`
(`previous`, `next`, `clear`, `inside` or `outside`). `focus.get` names
the fields `task-form-description`, `task-form-due` and
`task-form-owners`.

The task summary view (`tools/wrentest/test_tasks_view*.py`) is driven by
`tasks.view`. With no parameters it reports `{"tasks", "open", "scope",
"path", "label", "keys"}`: whether the view is open, its scope, and whether
it has the keyboard (`{"tasks": false}` with tasks off). `{"open": {"scope":
"workspace" | "section" | "group" | "note", "path": …}}` opens a view as
any entry point does, and answers with the action's `disposition` and
`warning`. `{"key": {"key": …, "mods": […]}}` presses a key while the view
has the keyboard, as its key wrapper and then the app hear it, and
`{"click": …}` clicks a control, or with `{"click": {"cell": {"row": 0,
"cell": "box" | "priority" | "due"}}}` a cell of a row, `{"click": {"right":
0}}` right-clicks a row, `{"click": {"menu": "Done"}}` presses a row of the
menu over the view (rows count from zero), and with
`{"list": true}` the answer adds the list's state: its filters, chips,
groups, rows (with whether each is held in place), counts, focus, open menu,
and where the controls were drawn. `{"write": {"path": "Work/A.md", "line":
0, "change": "toggle"}}` (`line` counts from zero; `expect` optionally names
the line a stale row holds), `{"hide": {"path", "hidden"}}` and `{"undo":
true}` call the functions the keys and menus call, and answer with the
`write` or `switch` made, `refused` (`{"message", "refresh"}` when nothing
was written, `null` for a write that changed nothing), the index's `note`,
`can_undo`, and the action's `disposition` and `warning`. Under a view every
`board.*` request but `board.get` is refused, and so is `editor.focus`, and
a key is refused while the view has no keyboard. `navigation.get` reports
the pane as `surface` (`text`, `board`, `raw` or `tasks`), and its
`can_back` and `can_forward` count views.

`tasks.entry` reports the ways in: with no parameters `{"tasks", "counts",
"headers", "marks", "scope", "notice"}`, whether the headers count, each
column's count (its `scope`, `path`, `open_and_doing`, `overdue`, `pressed`
and `label`), the paths of the notes and groups that carry a mark, the scope
`tasks.show` would open now, and the message showing with its `text`,
`code` and whether it has `undo`. `{"click": column}` clicks a column's
count, `{"undo": true}` presses the message's Undo and `{"dismiss": true}`
dismisses it. With tasks off every action is refused.

`make perf-tasks` times the task index on `make perf-search`'s workload, in
release: the full build, one note's update, a keystroke and the three header
counts.

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

`--only tasks` (task lines, colours, the line commands, the status bar, the menus, the calendar, the suggestions, the form and the ways into the task view) opens the tasks fixture workspace with `tasks = on` and today
pinned to 19 November 2026, on the advanced engine unless `--engine` says
otherwise, and shoots task lines, colours, the line commands, the status bar
and the menus into `target/scenes/tasks/<engine>-<renderer>-<display>-<fonts>/<theme>/`;
`--fonts` names the emoji font setup in that path. Check them in three
themes on both engines after changing task colours or the status bar.

`--only task-list` opens `fixtures/tasks/list/` (two groups, with
`tasks-me = sam, sam.lee`) the same way and shoots the summary view of the
workspace's tasks: all of them, narrowed by a status chip, grouped by due
date and by owner, an empty list with its hint, and the scope menu; then a
row's menus, the calendar and the form over it, the message with Undo, and
a row held in place. The PNGs go where `--only tasks` writes them.

`--only spell`, which needs `--engine advanced`, turns spell check on and
shoots the acceptance note: `spell-marks` (its marks, with the caret
hiding one), `spell-menu` (the menu under a word), `spell-menu-empty`
("No suggestions"), and `spell-menu-above` (the menu above a word on the
last visible row). Every other mode keeps the lane's `WRENTEST_SPELL`
setting. Check them in every theme after changing the zigzag or the menu:

```bash
ICED_BACKEND=tiny-skia python3 tools/run_display.py x11 -- \
  python3 tools/capture_scenes.py --only spell --engine advanced \
  --label tiny-skia --theme wren-dark
```

`--only boards` opens the board samples with `boards = on` and writes the
board scenes: `board-flow` (fitted), `board-zoom-in` (200 %),
`board-zoom-out` (25 %), `board-shapes`, `board-overlap`, `board-raw` (a
board with conflict markers, shown as text under its banner),
`board-notes-list` (the kind icon), `board-search` (a label hit),
`board-wide` (columns folded) and `board-new-dialog` (the kind pills), with
the exports `board-flow.svg` and `board-flow-export.png` beside them; then
what editing draws over a board: `board-selected` (one node and its
handles), `board-cursor` (the cursor ring with nothing selected),
`board-multiple` (two selected, the ring on the second), `board-marquee` (a
marquee being dragged), `board-armed` (the mode chip),
`board-connect-handles` (the hover handles), `board-connect-target` (a
connector dragged onto a node, its side handles shown), `board-connect-new`
(the same drag over empty canvas), `board-connect-end` (an end dragged off
its node), `board-connecting` (the keyboard connect's target and query),
`board-editing` (a label open, with its caret), `board-edges` (heads,
dashes, labels, both routes), `board-empty-hint` (a new board) and
`board-built` (the flowchart the keyboard script builds); then
`board-stacking`, `board-actions` (the actions palette with a preview),
`board-colors` (every colour at every fill), `board-jump` (the jump
palette) and `board-limits` (values past each limit, flagged); and the
text sizes: `board-text-sizes` (each size on shapes and free text, one past
the range flagged), `board-text-sizes-quarter` and `board-text-sizes-far`
(at 25 % and 10 %, where small labels are left out),
`board-text-size-menu` (the **Aa** menu), `board-text-size-hover` (a size
previewed) and `board-text-size-editing` (a label edited at 28). Shoot them
on both renderers in both themes, and in all eleven themes
(`--theme <id>`) after changing board colours:

```bash
for r in tiny-skia wgpu; do for t in dark light; do
  WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1 ICED_BACKEND=$r python3 tools/run_display.py x11 -- \
    python3 tools/capture_scenes.py --only boards --theme $t --label $r
done; done
```

`--window 1024x640` (X11 only) resizes the window before the board scenes,
to see them at the size of a smaller screen; give it a `--label` too.

To try boards by hand without touching your own settings, copy the sample
workspace and give Wren a throwaway config directory, in a target directory
of its own so a running suite keeps its binary:

```bash
rm -rf /tmp/wren-try && mkdir -p /tmp/wren-try/config/wren
cp -r tools/wrentest/fixtures/boards/workspace /tmp/wren-try/work
export CARGO_TARGET_DIR=$PWD/target/try XDG_CONFIG_HOME=/tmp/wren-try/config
make run ARGS="--workspace /tmp/wren-try/work"
cargo run --locked -p wren-iced -- export "/tmp/wren-try/work/Boards/Samples/Checkout flow.wren-board" -o /tmp/wren-try/flow.png
```

## macOS

Rsync the tree to a Mac (excluding `target/`, `.venv/`, and `.claude/`) and
build there. `capture_scenes.py` works over ssh against a real Metal window
when a user is logged in at the console. Synthetic keystrokes through
`osascript` need an Accessibility grant that ssh sessions don't have.

So over ssh a Mac can build, run the Rust tests, capture scenes (the
`board.*` requests work there, so they include interactive states), and run
`wren export`, but it cannot send real keys or pointer input, and the
functional suite runs on Linux only. The macOS-only code (`macos_menu.rs`,
the menu's key equivalents) is type-checked from Linux with
`cargo clippy --locked --target aarch64-apple-darwin -p wren-iced --all-targets --features smoke-test -- -D warnings`
(the target must be installed), and otherwise runs only on a Mac or in CI.
What still needs hands at a Mac console: the trackpad (two-finger pan, ⌘ +
scroll zoom, momentum), ⌘ keys and the Board menu on a board, the input
method in a label, and right-click and Ctrl-click opening the actions
palette.

For updater changes: `cargo build --features smoke-test`,
`third_party/sparkle/fetch.sh`, then `python3 tools/check_updater.py`.
