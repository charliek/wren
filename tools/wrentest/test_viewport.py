"""Actual X11 client pixels, not IPC screenshots, prove viewport retention.

The protocol exposes text/cursor/history but no absolute editor scroll offset or
Content allocation identity. Native crop equality supplies the missing visible
viewport evidence. These checks require the private Xvfb lane and are deselected
on Wayland; they do not replace the native input/selection/resize gesture tests.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

import pytest

from tools.wrentest.test_editor import focus_native, xdo
from tools.wrentest.test_setup import BACKGROUNDS, artifact_directory
from tools.wrentest.ui import WrenApp, wait_for
from tools.wrentest.x11 import RGBImage, read_pixels


LINE_HEIGHT = 24
TEXT_LEFT = 8 + 52
SCROLL_LINES = 40
CROP_WIDTH = 416
CROP_HEIGHT = 10 * LINE_HEIGHT
CAPTURE_TIMEOUT = 8
MAX_CAPTURES = 40
STABLE_CAPTURES = 3
BODY_COLORS = {"dark": (0xd5, 0xd9, 0xe0), "light": (0x31, 0x38, 0x44)}
NOTE = "Viewport/Proofs/Proof.md"
IDENTITY_FIELDS = (
    "text", "cursor", "selection", "selected_text", "revision", "line_count", "words",
    "can_undo", "can_redo", "find_open", "find_query", "find_count", "find_current",
    "soft_wrap", "theme",
)


def editor_identity(app: WrenApp):
    snapshot = app.client.call("editor.get", timeout=3)
    return {field: snapshot[field] for field in IDENTITY_FIELDS}


def gateway(app: WrenApp, operation: str, **params):
    result = app.client.call(operation, params, timeout=3)
    assert result["disposition"] == "completed", result
    assert result["warning"] is None, result
    return result["navigation"]


def resize_window(app: WrenApp, window: str, width: int) -> None:
    xdo("windowsize", window, str(width), "800")

    def measured():
        geometry = dict(line.split("=", 1) for line in
                        xdo("getwindowgeometry", "--shell", window).splitlines())
        viewport = app.client.call("navigation.get", timeout=3)["viewport"]
        return (int(geometry["WIDTH"]) == width and int(geometry["HEIGHT"]) == 800
                and viewport == {"width": width, "height": 800})
    wait_for(measured, timeout=5, interval=0.025, description=f"owned X11 client size {width}x800")


def body_rectangle(app: WrenApp) -> tuple[int, int, int, int]:
    current = app.client.call("navigation.get", timeout=3)
    root = app.client.call("navigation.bounds", timeout=3)["root"]
    assert root["width"] == current["viewport"]["width"], (root, current["viewport"])
    assert root["height"] > 600 and root["y"] == 48, root
    if current["focus"]:
        editor_width = min(root["width"], 760)
        left = root["x"] + (root["width"] - editor_width) / 2
    else:
        browser_width = sum(current["effective_widths"].values())
        editor_width = root["width"] - browser_width
        left = root["x"] + browser_width
    assert editor_width >= 580, "Every fixture line must remain unwrapped"
    # The saved editor header is 49px plus its bottom rule. With no banner or
    # find bar, the body starts at client y=98, followed by the editor's 24px
    # top padding. Text starts after the 8px row inset and 52px line-number
    # gutter; skip 4px more to exclude the initial (0,0) 2px caret too.
    x, y = left + TEXT_LEFT + 4, root["y"] + 50 + 24
    assert x == int(x) and y == 122, (x, y)
    assert TEXT_LEFT + 4 + CROP_WIDTH < editor_width - 8
    assert y + CROP_HEIGHT < root["y"] + root["height"] - 50
    return int(x), int(y), CROP_WIDTH, CROP_HEIGHT


def glyph_statistics(image: RGBImage, theme: str) -> dict:
    background, body = BACKGROUNDS[theme], BODY_COLORS[theme]
    background_pixels = body_pixels = 0
    glyph_rows = [0] * (image.height // LINE_HEIGHT)
    for y in range(image.height):
        for x in range(image.width):
            pixel = image.pixel(x, y)
            background_distance = max(abs(a - b) for a, b in zip(pixel, background))
            body_distance = max(abs(a - b) for a, b in zip(pixel, body))
            background_pixels += background_distance <= 3
            body_pixels += body_distance <= 4
            glyph_rows[y // LINE_HEIGHT] += body_distance < background_distance
    return {
        "background_pixels": background_pixels,
        "body_pixels": body_pixels,
        "glyph_pixels_per_line": glyph_rows,
        "sha256": hashlib.sha256(image.data).hexdigest(),
    }


def has_body_glyphs(image: RGBImage, statistics: dict) -> bool:
    return (statistics["background_pixels"] >= image.width * image.height * 0.70
            and statistics["body_pixels"] >= 200
            and all(count >= 60 for count in statistics["glyph_pixels_per_line"]))


def save_native(path: Path, image: RGBImage) -> None:
    path.write_bytes(f"P6\n{image.width} {image.height}\n255\n".encode("ascii") + image.data)


def native_crop(app: WrenApp, window: str, theme: str, artifacts: Path, stage: str,
                *, expected: bytes | None = None, different_from: bytes | None = None,
                require_body: bool = True) -> tuple[RGBImage, dict]:
    # The IPC screenshot requests a real window frame. Its file is deliberately
    # never decoded or used as evidence: only XGetImage pixels can satisfy this
    # poll, including when the offscreen screenshot's editor primitives are blank.
    app.client.call("app.screenshot", {"path": str(artifacts / f"{stage}-frame.png")}, timeout=10)
    rectangle = body_rectangle(app)
    previous = None
    consecutive = attempts = 0
    latest = None
    statistics = {}

    def presented():
        nonlocal previous, consecutive, attempts, latest, statistics
        assert app.returncode is None, "The owned GUI exited during pixel readback"
        attempts += 1
        assert attempts <= MAX_CAPTURES, f"Exceeded {MAX_CAPTURES} native captures at {stage}: {statistics}"
        latest = read_pixels(app.pid, window, *rectangle)
        statistics = glyph_statistics(latest, theme)
        meaningful = not require_body or has_body_glyphs(latest, statistics)
        consecutive = consecutive + 1 if meaningful and latest.data == previous else int(meaningful)
        previous = latest.data
        if (consecutive >= STABLE_CAPTURES
                and (expected is None or latest.data == expected)
                and (different_from is None or latest.data != different_from)):
            return latest
        return None

    path = artifacts / f"{stage}-native.ppm"
    try:
        image = wait_for(presented, timeout=CAPTURE_TIMEOUT, interval=0.05,
                         description=f"{stage}: stable, nonblank actual X11 editor pixels")
    except (TimeoutError, AssertionError) as error:
        if latest is not None:
            save_native(path, latest)
        raise AssertionError(
            f"Actual X11 client pixels were blank, unexpected, or unstable at {stage}; "
            f"rectangle={rectangle}, captures={attempts}, statistics={statistics}, "
            f"expected_sha256={hashlib.sha256(expected).hexdigest() if expected is not None else None}, "
            f"native_artifact={path}"
        ) from error
    save_native(path, image)
    return image, {"stage": stage, "rectangle": rectangle, "captures": attempts, **statistics}


@pytest.fixture
def viewport_workspace(tmp_path: Path, request):
    assert os.environ.get("DISPLAY"), "native_x11 requires an X11 display"
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    assert Path(os.environ.get("XDG_RUNTIME_DIR", "")).name.startswith("wren-display-"), (
        "Run with tools/run_display.py x11; readback must never target a user desktop"
    )
    root = tmp_path / "notes"
    path = root / NOTE
    path.parent.mkdir(parents=True)
    lines = [f"row {index:04d} / {index * 7919 % 0xffffff:06x} / "
             + ("W" if index % 2 else "i") * 8 for index in range(180)]
    assert len(set(lines)) == len(lines) and max(map(len, lines)) <= 32
    original = "\n".join(lines) + "\n"
    assert len(original.encode("utf-8")) < 16 * 1024, "Keep protocol fixtures far below the 1MiB frame cap"
    path.write_text(original, encoding="utf-8")
    artifacts = artifact_directory(request.node.name)
    with WrenApp(workspace=root, artifacts_dir=artifacts) as app:
        snapshot = app.client.call("session.get")
        notes = [entry for entry in snapshot["notes"] if entry["path"] == NOTE]
        assert len(notes) == 1, notes
        app.client.call("core.apply", {"op": "open_note", "note": notes[0]["id"]})
        window = focus_native(app)
        assert int(xdo("getwindowpid", window)) == app.pid
        assert int(xdo("getwindowfocus"), 0) == int(window, 0)
        xdo("mousemove", "--window", window, "10", "10")
        resize_window(app, window, 1280)
        sandbox = app.sandbox
        try:
            yield app, path, original, window, artifacts
        finally:
            quarantine = Path(app.client.call("session.get")["trash_path"])
            assert quarantine == app.sandbox / "trash"
            assert list(quarantine.iterdir()) == [], "Viewport tests must never send anything to trash"
    assert not sandbox.exists(), "The owned GUI sandbox survived verified process teardown"


@pytest.mark.native_x11
@pytest.mark.parametrize("theme", ["dark", "light"])
def test_actual_client_pixels_retain_scrolled_viewport_across_chrome(viewport_workspace, theme: str) -> None:
    app, path, original, window, artifacts = viewport_workspace
    call = app.client.call
    call("theme.set", {"id": theme})
    gateway(app, "view.focus", enabled=False)
    # Both history directions are populated; their concrete contents are replayed
    # after the viewport assertions, rather than trusting only can_undo/can_redo.
    call("editor.action", {"kind": "move", "motion": "end"})
    one = original + "history one\n"
    two = one + "history two\n"
    assert call("editor.insert", {"text": "history one\n"})["text"] == one
    call("editor.action", {"kind": "move", "motion": "end"})
    assert call("editor.insert", {"text": "history two\n"})["text"] == two
    assert call("editor.undo")["text"] == one
    call("core.apply", {"op": "flush"})
    call("editor.cursor", {"line": 0, "column": 0, "selection": None})
    # The footer's word count follows an edit on a later tick.
    wait_for(lambda: call("editor.get")["words"] == len(one.split()),
             description="settled word count")
    before_editor = editor_identity(app)
    before_session = call("session.get")
    assert before_editor["can_undo"] is True and before_editor["can_redo"] is True
    assert before_editor["cursor"] == {"line": 0, "column": 0}
    assert before_editor["selection"] is None and before_editor["selected_text"] is None
    assert before_session["document"]["dirty"] is False
    assert before_session["document"]["error"] is None and before_session["banner"] is None
    assert before_editor["find_open"] is False
    assert path.read_text(encoding="utf-8") == one

    reports = []
    initial, report = native_crop(app, window, theme, artifacts, "columns-top")
    reports.append(report)
    call("editor.action", {"kind": "scroll", "lines": SCROLL_LINES})
    assert editor_identity(app) == before_editor
    reference, report = native_crop(app, window, theme, artifacts, "columns-scrolled",
                                    different_from=initial.data)
    reports.append(report)
    # The distinct glyph-bearing first ten lines must really change on scroll.
    # A content-only snapshot, blank crop, or unchanged viewport cannot pass.
    assert sum(a != b for a, b in zip(initial.data, reference.data)) > 500

    def unchanged(stage: str):
        assert editor_identity(app) == before_editor, stage
        current = call("session.get")
        assert current["document"] == before_session["document"], stage
        for field in ("root", "generation", "selected_section", "selected_group", "selected_note"):
            assert current[field] == before_session[field], (stage, field)
        assert current["banner"] is None, current["banner"]
        assert path.read_text(encoding="utf-8") == one, stage

    for stage, operation, params in (
        ("narrower-sections", "view.width", {"pane": "sections", "width": 140}),
        ("focus", "view.focus", {"enabled": True}),
        ("unfocus", "view.focus", {"enabled": False}),
    ):
        gateway(app, operation, **params)
        unchanged(stage)
        image, report = native_crop(app, window, theme, artifacts, stage, expected=reference.data)
        assert image.data == reference.data, stage
        reports.append(report)
        unchanged(stage)
    assert reports[2]["rectangle"][0] != reports[1]["rectangle"][0], "Narrowing a pane did not translate the editor"
    assert reports[3]["rectangle"][0] != reports[2]["rectangle"][0], "Focus did not center the editor"

    gateway(app, "palette.open", mode="commands")
    assert call("navigation.get")["modal"]["mode"] == "commands"
    unchanged("commands-open")
    _overlay, report = native_crop(app, window, theme, artifacts, "commands-open",
                                   different_from=reference.data, require_body=False)
    reports.append(report)
    gateway(app, "palette.close")
    assert call("navigation.get")["modal"] is None
    image, report = native_crop(app, window, theme, artifacts, "commands-closed", expected=reference.data)
    assert image.data == reference.data
    reports.append(report)
    unchanged("commands-closed")

    resize_window(app, window, 1440)
    image, report = native_crop(app, window, theme, artifacts, "wider", expected=reference.data)
    assert image.data == reference.data
    reports.append(report)
    unchanged("wider")
    assert call("editor.undo")["text"] == original
    assert call("editor.redo")["text"] == one
    assert call("editor.redo")["text"] == two
    call("core.apply", {"op": "flush"})
    assert path.read_text(encoding="utf-8") == two
    (artifacts / "native-viewport.json").write_text(json.dumps(reports, indent=2) + "\n", encoding="utf-8")
