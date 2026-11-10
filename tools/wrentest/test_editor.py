"""Real editor regression tests; native X11 cases exercise OS input separately.

Every case runs on the text engines that ``WRENTEST_ENGINE`` names, a
comma-separated list of ``classic`` and ``advanced`` (default ``classic``).
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

import pytest

from tools.wrentest import png
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

ENGINES = ("classic", "advanced")


def selected_engines() -> list[str]:
    names = [name.strip() for name in os.environ.get("WRENTEST_ENGINE", "classic").split(",")]
    unknown = [name for name in names if name not in ENGINES]
    if unknown:
        raise pytest.UsageError(f"WRENTEST_ENGINE must name {' or '.join(ENGINES)}, got {unknown}")
    return list(dict.fromkeys(names))


@pytest.fixture(params=selected_engines())
def engine(request: pytest.FixtureRequest) -> str:
    return request.param


@pytest.fixture
def editor_app(engine: str):
    """Launch WrenApps on the engine under test; each must report it."""

    @contextmanager
    def launch(**kwargs) -> Iterator[WrenApp]:
        with WrenApp(settings=f"editor-engine = {engine}\n", **kwargs) as app:
            assert app.client.call("editor.get")["engine"] == engine
            yield app

    return launch


def test_unicode_selection_history_and_find(editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-history")) as app:
        call = app.client.call
        original = "# Café 🐦\n\nOne café, two CAFÉ.\n"
        call("editor.set", {"text": original})
        call("editor.action", {"kind": "move", "motion": "end"})
        edited = call("editor.insert", {"text": "日本語 e\u0301 🐦"})
        assert edited["text"] == original + "日本語 e\u0301 🐦"
        assert edited["can_undo"] is True
        assert call("editor.undo")["text"] == original
        assert call("editor.redo")["text"] == edited["text"]
        found = call("editor.find", {"query": "café"})
        assert found["find_count"] == 3
        assert found["selected_text"].lower() == "café"
        assert call("editor.find", {"direction": "previous"})["find_current"] == 2
        call("editor.action", {"kind": "select_all"})
        assert call("editor.get")["selected_text"] == edited["text"]


@pytest.mark.parametrize("ending", ["\n", "\r\n", "\r", "\n\r"])
def test_find_and_edit_with_supported_line_endings(ending, editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-line-endings")) as app:
        call = app.client.call
        original = ending.join(("aa", "bb", "cc"))
        call("editor.set", {"text": original})
        found = call("editor.find", {"query": "bb"})
        assert found["selected_text"] == "bb"
        assert found["cursor"]["line"] == 1
        assert call("editor.insert", {"text": "🐦"})["text"] == ending.join(("aa", "🐦", "cc"))
        assert call("editor.undo")["text"] == original
        assert call("editor.get")["selected_text"] == "bb"


@pytest.mark.parametrize("ending", ["\n", "\r\n", "\r", "\n\r"])
def test_enter_and_paste_keep_the_notes_line_endings(ending, editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-enter-endings")) as app:
        call = app.client.call
        original = ending.join(("- a", "- b"))
        call("editor.set", {"text": original})
        call("editor.cursor", {"line": 0, "column": 3, "selection": None})
        assert call("editor.action", {"kind": "enter"})["text"] == ending.join(("- a", "", "- b"))
        pasted = call("editor.insert", {"text": "x\ny\r\nz"})["text"]
        assert pasted == ending.join(("- a", "x", "y", "z", "- b"))
        call("editor.action", {"kind": "move", "motion": "end"})
        assert call("editor.action", {"kind": "enter"})["text"] == pasted + ending
        call("editor.undo")
        call("editor.undo")
        assert call("editor.undo")["text"] == original


def test_theme_changes_preserve_editor_state_and_noop_edits(editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-themes")) as app:
        call = app.client.call
        call("editor.set", {"text": "first\nemoji 🐦 and café\nlast"})
        call("editor.cursor", {"line": 1, "column": 10,
                               "selection": {"line": 1, "column": 6}})
        before = call("editor.get")
        for theme in ("light", "dark"):
            after = call("theme.set", {"id": theme})
            for field in ("text", "cursor", "selection", "selected_text", "revision"):
                assert after[field] == before[field]
            assert after["theme"] == f"wren-{theme}"
        call("editor.set", {"text": "abc"})
        before = call("editor.action", {"kind": "move", "motion": "start"})
        after = call("editor.action", {"kind": "backspace"})
        assert after["revision"] == before["revision"]
        assert after["can_undo"] is False
        call("editor.insert", {"text": "Z"})
        call("editor.undo")
        call("editor.action", {"kind": "backspace"})
        assert call("editor.redo")["text"] == "Zabc"
        call("editor.set", {"text": "hello world"})
        call("editor.action", {"kind": "select_all"})
        moved = call("editor.cursor", {"line": 0, "column": 5, "selection": None})
        assert moved["selected_text"] is None
        assert call("editor.insert", {"text": "X"})["text"] == "helloX world"


def capture(app: WrenApp, path: Path) -> png.PNG:
    result = app.client.call("app.screenshot", {"path": str(path)})
    image = png.read_png(path.read_bytes())
    assert (image.width, image.height) == (result["width"], result["height"])
    return image


@pytest.mark.parametrize("theme,heading,code", [
    ("dark", (0xF0, 0xB4, 0x5A), (0x7F, 0xD1, 0xA0)),
    ("light", (0xA1, 0x48, 0x08), (0x19, 0x73, 0x42)),
])
def test_editor_highlighting_renders_in_both_themes(theme, heading, code, editor_app) -> None:
    artifacts = artifact_directory(f"editor-highlight-{theme}")
    with editor_app(artifacts_dir=artifacts) as app:
        app.client.call("theme.set", {"id": theme})
        app.client.call("editor.set", {"text": "# Heading\nplain text\n`inline code`\n**bold** and *italic*"})
        image = capture(app, artifacts / "highlighting.png")
        # The editor fills the window; its text starts after the 60 px gutter.
        colors = [image.pixel(x, y)[:3] for y in range(100, 215) for x in range(60, 340)]
        for expected in (heading, code):
            assert sum(all(abs(a - b) <= 3 for a, b in zip(pixel, expected))
                       for pixel in colors) >= 15, f"Missing rendered syntax color {expected}"


@pytest.mark.parametrize("theme,code_block,link", [
    ("dark", (0x18, 0x1B, 0x21), (0x7F, 0xB0, 0xE6)),
    ("light", (0xEF, 0xF1, 0xF5), (0x1F, 0x5F, 0xBF)),
])
def test_code_blocks_get_a_band_and_links_their_color(theme, code_block, link, editor_app) -> None:
    artifacts = artifact_directory(f"editor-blocks-{theme}")
    with editor_app(artifacts_dir=artifacts) as app:
        call = app.client.call
        call("theme.set", {"id": theme})

        def count(image: png.PNG, color: tuple, step: int = 2) -> int:
            return sum(all(abs(a - b) <= 2 for a, b in zip(image.pixel(x, y)[:3], color))
                       for y in range(0, image.height, step) for x in range(0, image.width, step))

        # The link color is its own token (not the caret's accent), so a few
        # dozen glyph pixels prove it; the classic engine antialiases more.
        call("editor.set", {"text": "plain\n\nSee [documentation for every link](https://x.io) now"})
        call("editor.cursor", {"line": 0, "column": 0, "selection": None})
        plain = capture(app, artifacts / "no-code.png")
        assert count(plain, link, step=1) >= 60, "Link text did not use the link color"

        for fence in ("```", "~~~"):
            call("editor.set", {"text": f"plain\n{fence}sh\nrun it\nand more\n{fence}\nafter"})
            call("editor.cursor", {"line": 0, "column": 0, "selection": None})
            banded = capture(app, artifacts / f"code-{fence[0]}.png")
            # Four rows (both fences and two body lines) of a wide band,
            # sampled every other pixel; other surfaces may sit within the
            # tolerance.
            added = count(banded, code_block) - count(plain, code_block)
            assert added >= 4000, f"The {fence} block has no background band ({added})"


def test_soft_wrap_renders_multiple_rows(editor_app) -> None:
    artifacts = artifact_directory("editor-wrap")
    with editor_app(artifacts_dir=artifacts) as app:
        app.client.call("theme.set", {"id": "dark"})
        state = app.client.call("editor.set", {"text": "wrapped words " * 80})
        assert state["line_count"] == 1
        image = capture(app, artifacts / "wrapped.png")
        # The one logical line must paint text on several visual rows, well
        # below the title/warning and above the footer. Do not count the caret.
        background = image.pixel(image.width - 4, image.height // 2)
        painted_rows = sum(
            any(image.pixel(x, y) != background for x in range(24, 420))
            for y in range(150, min(400, image.height - 60))
        )
        assert painted_rows >= 35, f"Expected multiple wrapped text rows, got {painted_rows}"
        assert state["soft_wrap"] is True


def test_wrapped_list_rows_hang_on_the_advanced_engine(engine, editor_app) -> None:
    """The advanced engine starts an item's wrapped rows under its text, and
    Down keeps the caret's x, so it lands the marker's width further left in
    the row's text."""
    if engine != "advanced":
        pytest.skip("only the advanced engine hangs wrapped rows")
    with editor_app(artifacts_dir=artifact_directory("editor-hanging")) as app:
        call = app.client.call
        call("editor.set", {"text": "- " + "wrapped words " * 40})

        def down_from(column: int) -> int:
            call("editor.cursor", {"line": 0, "column": column, "selection": None})
            call("editor.action", {"kind": "move", "motion": "down"})
            cursor = call("editor.get")["cursor"]
            assert cursor["line"] == 0
            return cursor["column"]

        second_row = down_from(0)
        assert 2 < second_row < 200
        assert down_from(5) == second_row + 5 - len("- ")


def test_scroll_survives_theme_roundtrip(editor_app) -> None:
    artifacts = artifact_directory("editor-scroll")
    with editor_app(artifacts_dir=artifacts) as app:
        call = app.client.call
        call("theme.set", {"id": "dark"})
        call("editor.set", {"text": "\n".join(f"line {n:04d}: scrolling evidence" for n in range(300))})
        initial = capture(app, artifacts / "initial.png")
        call("editor.action", {"kind": "scroll", "lines": 60})
        before = capture(app, artifacts / "scrolled.png")
        call("theme.set", {"id": "light"})
        capture(app, artifacts / "light.png")
        call("theme.set", {"id": "dark"})
        after = capture(app, artifacts / "dark-again.png")
        region = [(x, y) for y in range(110, before.height - 60)
                  for x in range(20, 410)]
        assert sum(initial.pixel(x, y) != before.pixel(x, y) for x, y in region) > 100
        assert sum(before.pixel(x, y) != after.pixel(x, y) for x, y in region) < 30


def test_five_thousand_line_edit_measurements(editor_app) -> None:
    artifacts = artifact_directory("editor-5000")
    with editor_app(artifacts_dir=artifacts, extra_args=("--editor-spike",)) as app:
        call = app.client.call
        initial = call("editor.get")
        assert initial["line_count"] == 5000
        assert "café 🐦" in initial["text"]
        call("editor.action", {"kind": "move", "motion": "end"})
        roundtrips = []
        for _ in range(16):
            started = time.perf_counter()
            state = call("editor.insert", {"text": "x"})
            roundtrips.append((time.perf_counter() - started) * 1000)
        assert state["text"] == initial["text"] + "x" * 16
        assert call("editor.undo")["text"] == initial["text"] + "x" * 15
        assert call("editor.redo")["text"] == state["text"]
        # Smart Enter and Backspace in a 5,000-item list: the planner reads
        # the list around the cursor, and its cost must stay near a plain edit.
        smart: dict[str, list[int]] = {"enter": [], "backspace": []}
        for _ in range(6):
            for intent in ("enter", "backspace"):
                before = len(call("editor.get")["edit_micros"])
                timings = call("editor.command", {"intent": intent})["edit_micros"]
                assert len(timings) == before + 1, intent
                smart[intent].append(timings[-1])
        capture(app, artifacts / "large-editor.png")
        measurements = {
            "build": "debug", "lines": 5000, "bytes": len(initial["text"].encode()),
            "samples": len(roundtrips), "rpc_roundtrip_ms_p50": statistics.median(roundtrips),
            "rpc_roundtrip_ms_max": max(roundtrips),
            "edit_handler_us_p50": statistics.median(state["edit_micros"]),
            "edit_handler_us_max": max(state["edit_micros"]),
            "smart_enter_us_p50": statistics.median(smart["enter"]),
            "smart_backspace_us_p50": statistics.median(smart["backspace"]),
            "renderer": os.environ.get("ICED_BACKEND", "default"),
            "display": "wayland" if os.environ.get("WAYLAND_DISPLAY") else "x11",
        }
        (artifacts / "measurements.json").write_text(json.dumps(measurements, indent=2) + "\n")
        print(json.dumps(measurements, sort_keys=True))
        # A generous regression ceiling for shared software-rendered CI; the
        # spike report records actual values, not this timeout as typing latency.
        assert max(roundtrips) < 2000
        # Measured at 4-11 ms each in a debug build, most of it one scan of
        # the list; a key whose cost grew faster than the note would show here.
        assert statistics.median(smart["enter"]) < 100_000
        assert statistics.median(smart["backspace"]) < 100_000


def xdo(*args: str) -> str:
    result = subprocess.run(["xdotool", *args], check=True, capture_output=True,
                            text=True, timeout=10)
    return result.stdout.strip()


def focus_native(app: WrenApp) -> str:
    windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
    assert len(windows) == 1, windows
    window = windows[0]
    xdo("windowfocus", "--sync", window)
    app.client.call("editor.focus")
    return window


@pytest.mark.native_x11
def test_native_typing_shortcuts_selection_and_clipboard(editor_app) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with editor_app(artifacts_dir=artifact_directory("editor-native")) as app:
        call = app.client.call
        call("editor.set", {"text": ""})
        window = focus_native(app)
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "12", "native typing")
        wait_for(lambda: call("editor.get")["text"] == "native typing", description="native typing")
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+a", "ctrl+c")
        wait_for(lambda: call("clipboard.read") == "native typing", description="native copy")
        call("clipboard.write", {"text": "日本語 café 🐦 e\u0301"})
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+v")
        wait_for(lambda: call("editor.get")["text"] == "日本語 café 🐦 e\u0301", description="native paste")
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+z")
        wait_for(lambda: call("editor.get")["text"] == "native typing", description="native undo")
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+shift+z")
        wait_for(lambda: call("editor.get")["text"] == "日本語 café 🐦 e\u0301", description="native redo")
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+f")
        wait_for(lambda: call("editor.get")["find_open"], description="native find shortcut")
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "caf")
        wait_for(lambda: call("editor.get")["find_count"] == 1, description="native find input")
        assert call("editor.get")["selected_text"] == "caf"
        xdo("key", "--window", window, "--clearmodifiers", "Escape")
        wait_for(lambda: not call("editor.get")["find_open"], description="native find close")


@pytest.mark.native_x11
def test_native_return_keeps_a_crlf_note_crlf(editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-native-crlf")) as app:
        call = app.client.call
        call("editor.set", {"text": "one\r\ntwo"})
        window = focus_native(app)
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+End", "Return")
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "12", "three")
        wait_for(lambda: call("editor.get")["text"] == "one\r\ntwo\r\nthree",
                 description="native Return in a CRLF note")


@pytest.mark.native_x11
def test_native_engine_switch_on_focus_keeps_keyboard_focus(engine, editor_app) -> None:
    """A focus reload that moves the editor to the other engine leaves the
    keyboard where it was: in the find field, or in the editor."""
    other = next(name for name in ENGINES if name != engine)
    with editor_app(artifacts_dir=artifact_directory("editor-engine-focus")) as app:
        call = app.client.call
        config = app.settings_path
        call("editor.set", {"text": "café"})
        window = focus_native(app)

        def switch_on_refocus(to: str) -> None:
            xdo("windowfocus", "--sync", "0")
            config.write_text(f"editor-engine = {to}\n")
            xdo("windowfocus", "--sync", window)
            wait_for(lambda: call("editor.get")["engine"] == to, description=f"switch to {to}")

        xdo("key", "--window", window, "--clearmodifiers", "ctrl+f")
        wait_for(lambda: call("editor.get")["find_open"], description="native find shortcut")
        switch_on_refocus(other)
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "caf")
        wait_for(lambda: call("editor.get")["find_query"] == "caf", description="find keeps focus")
        assert call("editor.get")["text"] == "café"

        xdo("key", "--window", window, "--clearmodifiers", "Escape")
        wait_for(lambda: not call("editor.get")["find_open"], description="native find close")
        call("editor.focus")
        call("editor.action", {"kind": "move", "motion": "end"})
        switch_on_refocus(engine)
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "!")
        wait_for(lambda: call("editor.get")["text"] == "café!", description="editor keeps focus")


@pytest.mark.native_x11
@pytest.mark.parametrize("clicks,selected,edited", [(2, "hello", "X world"), (3, "hello world", "X")])
def test_native_word_and_line_selection_survive_undo(clicks, selected, edited, editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-native-selection")) as app:
        call = app.client.call
        call("editor.set", {"text": "hello world"})
        window = focus_native(app)
        xdo("mousemove", "--sync", "--window", window, "40", "115")
        xdo("click", "--repeat", str(clicks), "--delay", "80", "1")
        wait_for(lambda: call("editor.get")["selected_text"] == selected,
                 description="native word/line selection")
        xdo("key", "--window", window, "--clearmodifiers", "BackSpace")
        wait_for(lambda: call("editor.get")["text"] != "hello world", description="native selected deletion")
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+z")
        wait_for(lambda: call("editor.get")["text"] == "hello world", description="native deletion undo")
        assert call("editor.get")["selected_text"] == selected
        xdo("type", "--window", window, "--clearmodifiers", "X")
        wait_for(lambda: call("editor.get")["text"] == edited, description="replace restored selection")
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+z")
        wait_for(lambda: call("editor.get")["text"] == "hello world", description="native replacement undo")
        assert call("editor.get")["selected_text"] == selected
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+shift+z")
        wait_for(lambda: call("editor.get")["text"] == edited, description="native replacement redo")


@pytest.mark.native_x11
def test_native_large_document_typing(editor_app) -> None:
    artifacts = artifact_directory("editor-native-5000")
    with editor_app(artifacts_dir=artifacts, extra_args=("--editor-spike",)) as app:
        call = app.client.call
        initial = call("editor.get")["text"]
        window = focus_native(app)
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+End")
        started = time.perf_counter()
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "native input")
        wait_for(lambda: call("editor.get")["text"] == initial + "native input",
                 description="native 5000-line typing")
        elapsed_ms = (time.perf_counter() - started) * 1000
        capture(app, artifacts / "native-large.png")
        (artifacts / "native-measurements.json").write_text(
            json.dumps({"build": "debug", "lines": 5000, "characters": 12,
                        "injected_delay_ms": 15, "injection_to_observed_state_ms": elapsed_ms}, indent=2) + "\n"
        )
        assert elapsed_ms < 5000


@pytest.mark.native_x11
def test_native_text_size_shortcuts_resize_and_save(editor_app) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with editor_app(artifacts_dir=artifact_directory("editor-text-size")) as app:
        call = app.client.call
        config = app.settings_path
        call("editor.set", {"text": ""})
        window = focus_native(app)
        assert call("editor.get")["font_size"] == 14
        # `alt+plus` is Shift+= to the X server; `alt+equal` is unshifted.
        for key, size in (("alt+equal", 15), ("alt+plus", 16), ("alt+minus", 15)):
            xdo("key", "--window", window, "--clearmodifiers", key)
            wait_for(lambda size=size: call("editor.get")["font_size"] == size,
                     description=f"{key} → {size}")
        assert "editor-font-size = 15\n" in config.read_text()
        xdo("key", "--window", window, "--clearmodifiers", "alt+0")
        wait_for(lambda: call("editor.get")["font_size"] == 14, description="reset")
        assert "editor-font-size = 14\n" in config.read_text()
        # The shortcuts never type into the note.
        assert call("editor.get")["text"] == ""


def outline_note() -> tuple[str, dict[str, int]]:
    """A long note with two heading levels, and each heading's line."""
    lines = ["# Handbook", ""]
    headings = {"Handbook": 0}
    for part in range(1, 9):
        headings[f"Part {part}"] = len(lines)
        lines += [f"## Part {part}", ""]
        lines += [f"Body line {n} of part {part}." for n in range(20)]
        lines.append("")
        headings[f"Details {part}"] = len(lines)
        lines += [f"### Details {part}", ""]
        lines += [f"Detail {n} of part {part}." for n in range(5)]
        lines.append("")
    return "\n".join(lines), headings


def presented(app: WrenApp, path: Path) -> dict:
    """The editor once a frame has been drawn, so the scroll is settled."""
    capture(app, path)
    return app.client.call("editor.get")


def test_outline_previews_headings_restores_on_escape_and_commits_on_enter(
        engine, editor_app, tmp_path: Path) -> None:
    text, headings = outline_note()
    with editor_app(artifacts_dir=artifact_directory("editor-outline")) as app:
        call = app.client.call
        call("editor.set", {"text": text})
        line = headings["Part 3"] + 6
        call("editor.cursor", {"line": line, "column": 9, "selection": {"line": line, "column": 5}})
        before = presented(app, tmp_path / "before.png")
        assert before["selected_text"] == "line"
        assert before["first_visible_line"] > 0

        modal = call("palette.open", {"mode": "outline"})["navigation"]["modal"]
        assert modal["mode"] == "outline"
        rows = modal["results"]
        assert [row["title"] for row in rows[:3]] == ["Handbook", "Part 1", "Details 1"]
        assert [row["path"] for row in rows[:3]] == ["", "Handbook", "Handbook › Part 1"]
        assert rows[2]["key"] == {"heading": {"revision": before["revision"],
                                              "line": headings["Details 1"]}}
        active = rows[modal["active_index"]]
        assert active["title"] == "Part 3" and active["current"]
        # Opening moves nothing.
        opened = call("editor.get")
        assert (opened["cursor"], opened["selection"]) == (before["cursor"], before["selection"])

        call("palette.query", {"query": "part 7"})
        shown = presented(app, tmp_path / "part-7.png")
        assert shown["cursor"] == {"line": headings["Part 7"], "column": 0}
        assert shown["selection"] is None
        top = shown["first_visible_line"]
        assert top <= headings["Part 7"] < top + 12, top
        if engine == "advanced":
            # Near the top, with a few lines above it for context.
            assert top < headings["Part 7"], top
        call("palette.query", {"query": ""})
        assert call("navigation.get")["modal"]["results"][modal["active_index"]]["title"] == "Part 3"
        assert call("editor.get")["cursor"]["line"] == headings["Part 3"]
        call("palette.step", {"delta": 1})
        assert call("editor.get")["cursor"]["line"] == headings["Details 3"]

        call("palette.close")
        restored = presented(app, tmp_path / "restored.png")
        for field in ("cursor", "selection", "selected_text", "text", "revision"):
            assert restored[field] == before[field], field
        if engine == "advanced":
            assert restored["first_visible_line"] == before["first_visible_line"]

        call("palette.open", {"mode": "outline"})
        call("palette.query", {"query": "details 5"})
        chosen = call("palette.activate", {})
        assert chosen["disposition"] == "completed"
        assert chosen["navigation"]["modal"] is None
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard")
        committed = presented(app, tmp_path / "committed.png")
        assert committed["cursor"] == {"line": headings["Details 5"], "column": 0}
        assert committed["selection"] is None
        top = committed["first_visible_line"]
        assert top <= headings["Details 5"] < top + 12, top


def test_outline_leaves_a_view_away_from_the_cursor_in_place(
        engine, editor_app, tmp_path: Path) -> None:
    text, _ = outline_note()
    with editor_app(artifacts_dir=artifact_directory("editor-outline-away")) as app:
        call = app.client.call
        call("editor.set", {"text": text})
        caret = {"line": 2, "column": 0}
        selection = {"line": 5, "column": 4, "selection": {"line": 5, "column": 0}}
        for name, origin in (("caret", caret), ("selection", selection)):
            call("editor.cursor", origin)
            presented(app, tmp_path / f"{name}-origin.png")
            call("editor.action", {"kind": "scroll", "lines": 100})
            before = presented(app, tmp_path / f"{name}-away.png")
            assert before["first_visible_line"] >= 90, before["first_visible_line"]

            # Opening and closing with nothing previewed moves nothing.
            call("palette.open", {"mode": "outline"})
            opened = presented(app, tmp_path / f"{name}-opened.png")
            call("palette.close")
            closed = presented(app, tmp_path / f"{name}-closed.png")
            for state in (opened, closed):
                for field in ("first_visible_line", "cursor", "selection", "selected_text"):
                    assert state[field] == before[field], (name, field)

            # After a preview, advanced puts the view back exactly; classic
            # brings the cursor, above the view, onto the top row.
            call("palette.open", {"mode": "outline"})
            call("palette.step", {"delta": 1})
            previewed = presented(app, tmp_path / f"{name}-previewed.png")
            assert previewed["first_visible_line"] != before["first_visible_line"]
            call("palette.close")
            restored = presented(app, tmp_path / f"{name}-restored.png")
            for field in ("cursor", "selection", "selected_text"):
                assert restored[field] == before[field], (name, field)
            if engine == "advanced":
                assert restored["first_visible_line"] == before["first_visible_line"], name


def test_outline_of_a_note_without_headings_is_empty(editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-outline-empty")) as app:
        call = app.client.call
        call("editor.set", {"text": "plain\n```\n# in code\n```\n"})
        modal = call("palette.open", {"mode": "outline"})["navigation"]["modal"]
        assert modal["results"] == [] and modal["active_index"] is None
        assert call("palette.activate", {})["disposition"] == "blocked"
        call("palette.close")
        assert call("editor.get")["cursor"] == {"line": 0, "column": 0}


@pytest.mark.native_x11
def test_native_outline_shortcut_previews_restores_and_jumps(editor_app) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    text, headings = outline_note()
    with editor_app(artifacts_dir=artifact_directory("editor-native-outline")) as app:
        call = app.client.call
        call("editor.set", {"text": text})
        call("editor.cursor", {"line": 10, "column": 2, "selection": None})
        window = focus_native(app)

        def outline_open() -> bool:
            modal = call("navigation.get")["modal"]
            return modal is not None and modal["mode"] == "outline"

        xdo("key", "--window", window, "--clearmodifiers", "alt+shift+o")
        wait_for(outline_open, description="native outline shortcut")
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "part 6")
        wait_for(lambda: call("editor.get")["cursor"]["line"] == headings["Part 6"],
                 description="the outline previews the match")
        xdo("key", "--window", window, "--clearmodifiers", "Escape")
        wait_for(lambda: call("editor.get")["cursor"] == {"line": 10, "column": 2},
                 description="Escape restores the cursor")
        assert call("navigation.get")["modal"] is None

        xdo("key", "--window", window, "--clearmodifiers", "alt+shift+o")
        wait_for(outline_open, description="native outline shortcut again")
        xdo("key", "--window", window, "--clearmodifiers", "Down")
        wait_for(lambda: call("editor.get")["cursor"]["line"] == headings["Details 1"],
                 description="Down previews the next heading")
        xdo("key", "--window", window, "--clearmodifiers", "Return")
        wait_for(lambda: call("navigation.get")["modal"] is None, description="Return commits")
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "X")
        wait_for(lambda: call("editor.get")["text"].split("\n")[headings["Details 1"]]
                 == "X### Details 1", description="the editor has the keyboard")


def test_theme_and_font_pickers_preview_live_and_restore(engine, editor_app) -> None:
    with editor_app(artifacts_dir=artifact_directory("editor-pickers")) as app:
        call = app.client.call
        config = app.settings_path
        shown = call("editor.get")["theme"]

        modal = call("palette.open", {"mode": "themes"})["navigation"]["modal"]
        assert modal["active_key"] == {"theme": "system"}
        call("palette.query", {"query": "nord"})
        assert call("editor.get")["theme"] == "nord"
        call("palette.close")
        assert call("editor.get")["theme"] == shown
        # Previews never write the settings file.
        assert config.read_text() == f"editor-engine = {engine}\n"

        modal = call("palette.open", {"mode": "fonts"})["navigation"]["modal"]
        assert modal["active_key"] == {"font": "JetBrains Mono"}
        assert modal["results"][0]["path"] == "Bundled"
        assert call("editor.get")["font_family"] == "JetBrains Mono"
        others = [row["key"]["font"] for row in modal["results"] if not row["current"]]
        if others:
            index = next(i for i, row in enumerate(modal["results"])
                         if row["key"] == {"font": others[0]})
            active = modal["active_index"]
            call("palette.step", {"delta": index - active})
            assert call("editor.get")["font_family"] == others[0]
            call("palette.activate", {})
            assert call("editor.get")["font_family"] == others[0]
            assert f'editor-font-family = "{others[0]}"\n' in config.read_text()


def test_every_listed_font_previews_without_crashing(tmp_path: Path, editor_app) -> None:
    """Each installed font the picker lists is laid out and drawn. A screenshot
    waits for a presented frame, so every font's glyphs are rasterized."""
    with editor_app(artifacts_dir=artifact_directory("editor-every-font")) as app:
        call = app.client.call
        text = "# Heading\n\nSome **bold** and *italic* text, `code`, 12345.\n- [ ] a task\n"
        call("editor.set", {"text": text})
        modal = call("palette.open", {"mode": "fonts"})["navigation"]["modal"]
        rows = modal["results"]
        call("palette.step", {"delta": -modal["active_index"]})
        for index, row in enumerate(rows):
            if index:
                call("palette.step", {"delta": 1})
            assert call("editor.get")["font_family"] == row["key"]["font"]
            capture(app, tmp_path / f"font-{index}.png")
        call("palette.close")
        assert call("editor.get")["text"] == text


def test_font_changes_with_the_cursor_off_screen_keep_the_editor_alive(editor_app) -> None:
    """A font change reshapes only the visible lines; the caret's line must
    still be laid out when it is off-screen."""
    with editor_app(artifacts_dir=artifact_directory("editor-font-offscreen")) as app:
        call = app.client.call
        text = "\n".join(f"Line {number} of a long note" for number in range(1, 200))
        call("editor.set", {"text": text})

        def switch_font() -> None:
            call("palette.open", {"mode": "fonts"})
            rows = call("navigation.get")["modal"]["results"]
            call("palette.step", {"delta": 1 if len(rows) > 1 else 0})
            call("palette.close")
            assert call("editor.get")["font_family"] == "JetBrains Mono"

        call("editor.action", {"kind": "move", "motion": "end"})
        for _ in range(14):
            call("command.run", {"id": "font.increase"})
        switch_font()
        call("editor.action", {"kind": "move", "motion": "start"})
        call("editor.action", {"kind": "scroll", "lines": 400})
        switch_font()
        assert call("editor.get")["text"] == text
