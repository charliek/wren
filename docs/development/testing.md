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

The `make smoke-*` targets pass the variable through, so
`WRENTEST_ENGINE=advanced make smoke-x11` runs the whole suite with the editor
cases on the advanced engine. CI sets `WRENTEST_ENGINE=classic,advanced` on all four Linux
smoke lanes.

### Spell check

`WRENTEST_SPELL` sets the `spell-check` setting the harness writes for
every case that does not name it: `off` (the default when unset, so sample
notes full of names keep their look) or `on`. Any other value fails the
case. Spell check runs only on the advanced engine, so `on` matters only
to the cases `WRENTEST_ENGINE` runs there:

```bash
WRENTEST_SPELL=on ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11
```

`tools/wrentest/test_spell.py` sets `spell-check` itself. It runs the
acceptance note `tools/wrentest/fixtures/spell.md` with `catppuccin` in the
word list, and checks its marks, the menu, replacing a word, the toggle,
the word list, and the classic engine. Its `native_x11` cases right-click
and press the real suggest key. These requests drive it:

| Request | Parameters | Returns |
|---|---|---|
| `editor.spell` | none | `enabled` (the setting on the advanced engine), `ready` (the dictionary has loaded), `marks` as the last frame drew them, and `menu`, or `null` when it is closed. |
| `editor.spell_event` | `event`: `open_at_caret`, `move`, `hover`, `accept`, or `close`; `arg`: a row for `hover` and `accept`, a step for `move` | The `editor.get` snapshot, after the event went through the app as the widget would send it. |
| `editor.key` | `key`: a name (`up`, `down`, `left`, `right`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `home`, `end`, `shift`, `control`, `alt`, `super`, `caps_lock`) or one character; `mods`: a list of `shift`, `ctrl`, `alt`, `logo` | The `editor.get` snapshot, after the editor's own key routing and then the app's key handling, as a real press in the focused editor. |

Each mark has its `line`, its `start` and `end` byte offsets in the line,
its `word`, whether it is `shown` (the caret at its end hides it), and the
`rects` of its zigzag in window coordinates. The menu has the `word`, its
`line`, `start`, and `end`, the `items`, the `selected` row, and its
`bounds` in window coordinates once a frame has drawn it. A frame must be
drawn before marks appear, so poll `editor.spell` with `wait_for`. The
`shortcut` request runs `editor.spell_suggest` as its key does, and
`command.run` runs it as the palette does.

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

## macOS

Rsync the tree to a Mac (excluding `target/`, `.venv/`, and `.claude/`) and
build there. `capture_scenes.py` works over ssh against a real Metal window
when a user is logged in at the console. Synthetic keystrokes through
`osascript` need an Accessibility grant that ssh sessions don't have.

For updater changes: `cargo build --features smoke-test`,
`third_party/sparkle/fetch.sh`, then `python3 tools/check_updater.py`.
