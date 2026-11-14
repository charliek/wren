"""Colour, fill and outline on a board (discovery/boards.md §5.16 rule 1,
§5.1 rule 7, §7.4 item ST5).

Each case flushes, keeps the bytes, acts, flushes again and compares lines,
as `test_board_edit.py` does: a style command changes one line per element
it touches, and undo gives the bytes back. The digit keys go in through
`board.key`, and once under X11 as real digit presses of the lane's US
layout; the AZERTY fallback of §5.1 rule 7 is held by the key-policy unit
tests in `wren-iced`, since Xvfb keeps its US keymap.
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, board, board_app, bounds, click, copy_samples, fitted, flushed, key, line_diff, near,
    open_note, screenshot,
)
from tools.wrentest.test_board_view import REST, fill_point, wait_focus
from tools.wrentest.test_editor import xdo
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

# Checkout flow's elements: two shapes and the connector between them.
RECEIVE, PACK, INTO_RECEIVE = "12axfsny", "3qrg4kv1", "4x2tte9d"
# Wren Dark's red hue (crates/wren-core/src/theme.rs), a solid fill as is.
WREN_DARK_RED = [0xFD, 0x94, 0x94]


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def ready(app: WrenApp, path: str = FLOW) -> dict:
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    return fitted(app)


def placed_node(app: WrenApp, id_: str) -> tuple[dict, dict]:
    placed = bounds(app)
    return next(node for node in placed["nodes"] if node["id"] == id_), placed


def drawn_fill(app: WrenApp, artifacts: Path, name: str, id_: str) -> tuple[list[int], tuple]:
    """The fill `board.bounds` gives the node and the pixel a screenshot
    has inside it, clear of labels and connectors."""
    time.sleep(REST)
    node, placed = placed_node(app, id_)
    point = fill_point(node, placed)
    assert point is not None, node
    image = screenshot(app, artifacts / f"{name}.png")
    scale = placed["scale_factor"]
    return node["fill"], image.pixel(round(point[0] * scale), round(point[1] * scale))


def test_a_colour_key_is_one_line_drawn_in_its_fill_and_undo_gives_the_bytes_back(
        samples: Path) -> None:
    flow = samples / FLOW
    artifacts = artifact_directory("board-style-colour")
    with board_app(samples, "board-style-colour") as app:
        app.client.call("theme.set", {"id": "wren-dark"})
        ready(app)
        original = flow.read_bytes()
        neutral, _ = drawn_fill(app, artifacts, "neutral", RECEIVE)

        # Nothing selected: the key is taken and changes nothing.
        revision = board(app)["revision"]
        state = key(app, "1")
        assert state["handled"] and state["revision"] == revision, state

        click(app, {"node": RECEIVE})
        state = key(app, "1")
        assert state["handled"] and state["revision"] == revision + 1, state
        flushed(app)
        red = flow.read_bytes()
        assert line_diff(original, red) == ['+ style.color = "red"']
        fill, pixel = drawn_fill(app, artifacts, "red-tint", RECEIVE)
        assert fill != neutral and fill[0] > fill[1] and fill[0] > fill[2], fill
        assert near(pixel, fill, 8), (pixel, fill)

        # Red again sends no draft.
        assert key(app, "1")["revision"] == revision + 1

        # Solid fills with the theme's hue itself.
        app.client.call("command.run", {"id": "board.fill.solid"})
        flushed(app)
        assert line_diff(red, flow.read_bytes()) == ['+ style.fill = "solid"']
        fill, pixel = drawn_fill(app, artifacts, "red-solid", RECEIVE)
        assert fill == WREN_DARK_RED, fill
        assert near(pixel, fill, 8), (pixel, fill)

        # Undo, one step a command, gives the opening bytes back.
        key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == red
        state = key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == original
        assert state["pristine"] and not state["can_undo"], state
        assert not app.client.call("session.get")["document"]["dirty"]


def test_each_style_command_on_a_shape_and_a_connector_is_one_line_each(samples: Path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-style-commands") as app:
        ready(app)
        original = flow.read_bytes()
        click(app, {"node": RECEIVE})
        click(app, {"edge": INTO_RECEIVE}, mods=("shift",))
        assert set(board(app)["selection"]) == {RECEIVE, INTO_RECEIVE}
        for action, lines in (
            (("key", "7"), ['+ style.color = "blue"', '+ style.color = "blue"']),
            # A connector has no fill.
            (("command", "board.fill.none"), ['+ style.fill = "none"']),
            (("command", "board.stroke.dashed"),
             ['+ style.stroke = "dashed"', '+ style.stroke = "dashed"']),
            (("key", "0"), ['- style.color = "blue"', '- style.color = "blue"']),
        ):
            flushed(app)
            before = flow.read_bytes()
            if action[0] == "key":
                assert key(app, action[1])["handled"]
            else:
                app.client.call("command.run", {"id": action[1]})
            flushed(app)
            assert line_diff(before, flow.read_bytes()) == lines, action
        while board(app)["can_undo"]:
            key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == original


@pytest.mark.native_x11
def test_native_digit_keys_set_colours_and_a_shifted_digit_does_not(samples: Path) -> None:
    """The digit row pressed for real: 1 and 7 set red and blue, each one
    line; Shift+1 is zoom to fit, which writes nothing. The lane's layout
    is US; the AZERTY fallback of §5.1 rule 7 is the key policy's own test
    (`the_digit_row_falls_back_to_the_physical_key_when_the_character_is_unbound`),
    since Xvfb keeps its keymap whatever `setxkbmap` asks."""
    flow = samples / FLOW
    with board_app(samples, "board-style-digits") as app:
        ready(app)
        windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
        assert len(windows) == 1, windows
        xdo("windowfocus", "--sync", windows[0])
        wait_focus(app, ["note-editor"])
        original = flow.read_bytes()
        click(app, {"node": RECEIVE})
        for keysym, color in (("1", "red"), ("7", "blue")):
            revision = board(app)["revision"]
            xdo("key", keysym)
            wait_for(lambda: board(app)["revision"] == revision + 1, timeout=5,
                     interval=0.025, description=f"{keysym} set a colour")
            flushed(app)
            assert line_diff(original, flow.read_bytes()) == [f'+ style.color = "{color}"']
        written = flow.read_bytes()
        viewport = board(app)["viewport"]
        app.client.call("board.pointer", {"kind": "wheel", "at": {"node": RECEIVE},
                                          "delta": {"y": -2}, "mods": ["mod"]})
        zoomed = wait_for(lambda: board(app)["viewport"] != viewport and board(app)["viewport"],
                          timeout=5, interval=0.025, description="zoomed in")
        xdo("key", "shift+1")
        wait_for(lambda: board(app)["viewport"] != zoomed, timeout=5, interval=0.025,
                 description="shift+1 fitted the board")
        flushed(app)
        assert flow.read_bytes() == written
        assert board(app)["state"] == "idle"
