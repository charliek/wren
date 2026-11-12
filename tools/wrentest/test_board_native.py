"""Real pointer input on a board under X11 (discovery/boards.md §5.13,
§5.15, §6.8 Phase 1 item 4).

These cases prove only the translation from the toolkit's events to the
board: what the board then does is covered without a display. Targets come
from `board.bounds`, at scale factor 1 (the X11 lanes' Xvfb).
"""

from __future__ import annotations

import math
import os
import time

import pytest

from tools.wrentest.boards import (
    FLOW, board, board_app, bounds, fitted, open_note, stamp,
)
from tools.wrentest.test_board_view import column, moved, samples, wait_focus  # noqa: F401  (fixture)
from tools.wrentest.test_editor import xdo
from tools.wrentest.test_gestures import move, point_in
from tools.wrentest.ui import wait_for

# Past the wheel's 150 ms burst, so the next click starts its own.
BURST = 0.3


def native_board(app, path: str = FLOW) -> tuple[str, dict]:
    assert os.environ.get("DISPLAY") and not os.environ.get("WAYLAND_DISPLAY"), (
        "Run native_x11 only in the X11 lane"
    )
    open_note(app, path)
    windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
    assert len(windows) == 1, windows
    xdo("windowfocus", "--sync", windows[0])
    placed = fitted(app)
    assert placed["scale_factor"] == 1, "the native cases place the pointer in logical pixels"
    return windows[0], placed


@pytest.mark.native_x11
def test_native_press_on_a_board_takes_the_column_and_the_wheel_does_not(samples) -> None:
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-native-column") as app:
        window, placed = native_board(app)
        app.client.call("column.open", {"column": "notes"})
        wait_focus(app, [])
        move(window, point_in(placed["root"]))

        # The wheel reaches the board under the pointer and leaves the
        # keyboard where it was.
        for button in ("5", "4"):
            xdo("click", button)
        time.sleep(BURST)
        assert column(app) == "notes"
        wait_focus(app, [])

        # A middle-button click is the board taking the keyboard.
        xdo("click", "2")
        wait_for(lambda: column(app) == "editor", timeout=3, interval=0.025,
                 description="the press took the editor column")
        wait_focus(app, ["note-editor"])
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert stamp(flow) == before


@pytest.mark.native_x11
def test_native_wheel_pans_every_rectangle_alike(samples) -> None:
    with board_app(samples, "board-native-wheel") as app:
        window, start = native_board(app)
        move(window, point_in(start["root"]))
        lines = {}
        for button, axis in (("5", 1), ("4", 1), ("6", 0), ("7", 0)):
            before = bounds(app)
            xdo("click", button)
            time.sleep(BURST)
            after = bounds(app)
            deltas = moved(before, after)
            assert len(deltas) == 1, (button, deltas)
            (delta,) = deltas
            # Whole wheel lines of 40 px along the wheel's axis only.
            assert delta[1 - axis] == 0, (button, delta)
            lines[button] = delta[axis] / 40
            assert lines[button] != 0 and lines[button] == round(lines[button]), (button, delta)
            assert after["viewport"]["zoom"] == before["viewport"]["zoom"]
        # Down shows what is further down, so the board moves up; left and
        # right go opposite ways.
        assert lines["5"] < 0 < lines["4"], lines
        assert lines["6"] * lines["7"] < 0, lines


@pytest.mark.native_x11
def test_native_ctrl_wheel_zooms_about_the_pointer(samples) -> None:
    with board_app(samples, "board-native-zoom") as app:
        window, placed = native_board(app)
        root = placed["root"]
        pointer = (round(root["x"] + root["width"] * 0.3), round(root["y"] + root["height"] * 0.6))
        move(window, pointer)
        for button, larger in (("4", True), ("5", False)):
            view = bounds(app)["viewport"]
            under = (view["x"] + (pointer[0] - root["x"]) / view["zoom"],
                     view["y"] + (pointer[1] - root["y"]) / view["zoom"])
            xdo("keydown", "ctrl")
            try:
                xdo("click", button)
            finally:
                xdo("keyup", "ctrl")
            time.sleep(BURST)
            after = bounds(app)["viewport"]
            steps = math.log(after["zoom"] / view["zoom"]) / math.log(1.1)
            assert abs(steps - round(steps)) < 1e-3 and (round(steps) > 0) == larger, (button, steps)
            back = (root["x"] + (under[0] - after["x"]) * after["zoom"],
                    root["y"] + (under[1] - after["y"]) * after["zoom"])
            assert abs(back[0] - pointer[0]) <= 1 and abs(back[1] - pointer[1]) <= 1, (back, pointer)


@pytest.mark.native_x11
def test_native_middle_drag_pans_by_the_pointers_motion(samples) -> None:
    with board_app(samples, "board-native-drag") as app:
        window, placed = native_board(app)
        start = point_in(placed["root"])
        move(window, start)
        before = bounds(app)
        xdo("mousedown", "2")
        try:
            wait_for(lambda: board(app)["state"] == "panning", timeout=3, interval=0.025,
                     description="the middle button started a pan")
            move(window, (start[0] + 60, start[1] - 30))
        finally:
            xdo("mouseup", "2")
        wait_for(lambda: board(app)["state"] != "panning", timeout=3, interval=0.025,
                 description="the release ended the pan")
        assert moved(before, bounds(app)) == {(60.0, -30.0)}
        assert column(app) == "editor"
