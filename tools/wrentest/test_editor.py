"""Real editor regression tests; native X11 cases exercise OS input separately."""

from __future__ import annotations

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


def test_unicode_selection_history_and_find() -> None:
    with WrenApp(artifacts_dir=artifact_directory("editor-history")) as app:
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
def test_find_and_edit_with_supported_line_endings(ending) -> None:
    with WrenApp(artifacts_dir=artifact_directory("editor-line-endings")) as app:
        call = app.client.call
        original = ending.join(("aa", "bb", "cc"))
        call("editor.set", {"text": original})
        found = call("editor.find", {"query": "bb"})
        assert found["selected_text"] == "bb"
        assert found["cursor"]["line"] == 1
        assert call("editor.insert", {"text": "🐦"})["text"] == ending.join(("aa", "🐦", "cc"))
        assert call("editor.undo")["text"] == original
        assert call("editor.get")["selected_text"] == "bb"


def test_theme_changes_preserve_editor_state_and_noop_edits() -> None:
    with WrenApp(artifacts_dir=artifact_directory("editor-themes")) as app:
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


@pytest.mark.parametrize("theme,accent,code", [
    ("dark", (0xF0, 0xB4, 0x5A), (0x7F, 0xD1, 0xA0)),
    ("light", (0x9A, 0x58, 0x00), (0x19, 0x73, 0x42)),
])
def test_editor_highlighting_renders_in_both_themes(theme, accent, code) -> None:
    artifacts = artifact_directory(f"editor-highlight-{theme}")
    with WrenApp(artifacts_dir=artifacts) as app:
        app.client.call("theme.set", {"id": theme})
        app.client.call("editor.set", {"text": "# Heading\nplain text\n`inline code`\n**bold** and *italic*"})
        image = capture(app, artifacts / "highlighting.png")
        colors = [image.pixel(x, y)[:3] for y in range(100, 215) for x in range(20, 300)]
        for expected in (accent, code):
            assert sum(all(abs(a - b) <= 3 for a, b in zip(pixel, expected))
                       for pixel in colors) >= 15, f"Missing rendered syntax color {expected}"


def test_soft_wrap_renders_multiple_rows() -> None:
    artifacts = artifact_directory("editor-wrap")
    with WrenApp(artifacts_dir=artifacts) as app:
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


def test_scroll_survives_theme_roundtrip() -> None:
    artifacts = artifact_directory("editor-scroll")
    with WrenApp(artifacts_dir=artifacts) as app:
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


def test_five_thousand_line_edit_measurements() -> None:
    artifacts = artifact_directory("editor-5000")
    with WrenApp(artifacts_dir=artifacts, extra_args=("--editor-spike",)) as app:
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
        capture(app, artifacts / "large-editor.png")
        measurements = {
            "build": "debug", "lines": 5000, "bytes": len(initial["text"].encode()),
            "samples": len(roundtrips), "rpc_roundtrip_ms_p50": statistics.median(roundtrips),
            "rpc_roundtrip_ms_max": max(roundtrips),
            "edit_handler_us_p50": statistics.median(state["edit_micros"]),
            "edit_handler_us_max": max(state["edit_micros"]),
            "renderer": os.environ.get("ICED_BACKEND", "default"),
            "display": "wayland" if os.environ.get("WAYLAND_DISPLAY") else "x11",
        }
        (artifacts / "measurements.json").write_text(json.dumps(measurements, indent=2) + "\n")
        print(json.dumps(measurements, sort_keys=True))
        # A generous regression ceiling for shared software-rendered CI; the
        # spike report records actual values, not this timeout as typing latency.
        assert max(roundtrips) < 2000


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
def test_native_typing_shortcuts_selection_and_clipboard() -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with WrenApp(artifacts_dir=artifact_directory("editor-native")) as app:
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
@pytest.mark.parametrize("clicks,selected,edited", [(2, "hello", "X world"), (3, "hello world", "X")])
def test_native_word_and_line_selection_survive_undo(clicks, selected, edited) -> None:
    with WrenApp(artifacts_dir=artifact_directory("editor-native-selection")) as app:
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
def test_native_large_document_typing() -> None:
    artifacts = artifact_directory("editor-native-5000")
    with WrenApp(artifacts_dir=artifacts, extra_args=("--editor-spike",)) as app:
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
