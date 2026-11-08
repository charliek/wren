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

The `make smoke-*` targets pass the variable through, so
`WRENTEST_ENGINE=advanced make smoke-x11` runs the whole suite with the editor
cases on the advanced engine. CI sets `WRENTEST_ENGINE=classic,advanced` on all four Linux
smoke lanes.

## Looking at real screens

Build with `--features smoke-test`, then run `tools/capture_scenes.py`
(under `tools/run_display.py x11 -- …` on Linux). It writes one PNG per scene
to `target/scenes/<theme>/`, or with `--engine advanced` to
`target/scenes/advanced/<theme>/`. The `markdown` scene shows a note with
every construct the editor highlights and a selection across blank lines;
check it on both engines in both themes after changing the highlighter or
the editor's drawing.

## macOS

Rsync the tree to a Mac (excluding `target/`, `.venv/`, and `.claude/`) and
build there. `capture_scenes.py` works over ssh against a real Metal window
when a user is logged in at the console. Synthetic keystrokes through
`osascript` need an Accessibility grant that ssh sessions don't have.

For updater changes: `cargo build --features smoke-test`,
`third_party/sparkle/fetch.sh`, then `python3 tools/check_updater.py`.
