"""Real pointer input on a board under X11 (discovery/boards.md §5.15).

These cases prove only the translation from the toolkit's events to the
board: what the board then does is covered without a display.
"""

from __future__ import annotations

import os
import time

import pytest

from tools.wrentest.test_board_view import (
    FLOW, board_app, column, open_note, samples, wait_focus,  # noqa: F401  (fixture)
)
from tools.wrentest.test_editor import xdo
from tools.wrentest.test_gestures import move, point_in
from tools.wrentest.ui import wait_for


@pytest.mark.native_x11
def test_native_press_on_a_board_takes_the_column_and_the_wheel_does_not(samples) -> None:
    assert os.environ.get("DISPLAY") and not os.environ.get("WAYLAND_DISPLAY"), (
        "Run native_x11 only in the X11 lane"
    )
    flow = samples / FLOW
    before = (flow.read_bytes(), flow.stat().st_mtime_ns)
    with board_app(samples, "board-native-column") as app:
        open_note(app, FLOW)
        windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
        assert len(windows) == 1, windows
        window = windows[0]
        xdo("windowfocus", "--sync", window)
        canvas = wait_for(
            lambda: app.client.call("focus.get")["bounds"].get("note-editor"),
            timeout=5, description="the board's bounds")
        app.client.call("column.open", {"column": "notes"})
        wait_focus(app, [])
        move(window, point_in(canvas))

        # The wheel reaches the board under the pointer and leaves the
        # keyboard where it was.
        for button in ("5", "4"):
            xdo("click", button)
        time.sleep(0.3)
        assert column(app) == "notes"
        wait_focus(app, [])

        # A middle-button click is the board taking the keyboard.
        xdo("click", "2")
        wait_for(lambda: column(app) == "editor", timeout=3, interval=0.025,
                 description="the press took the editor column")
        wait_focus(app, ["note-editor"])
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert (flow.read_bytes(), flow.stat().st_mtime_ns) == before
