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
    FLOW, NOTES, board, board_app, bounds, fitted, open_note, stamp,
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


# Phase 2: real buttons and keys on the board (§6.1, §6.8 Phase 2 items 6
# and 7). Each case proves the translation; what the board then does is
# covered without a display.

PACK, RECEIVE, START = "3qrg4kv1", "12axfsny", "ahj860xh"


def screen_of(placed: dict, node: str) -> tuple[int, int]:
    """A node's middle in the window, in whole logical pixels."""
    rect = next(n["bounds"] for n in placed["nodes"] if n["id"] == node)
    return point_in(rect)


def native_drag(window: str, start: tuple[int, int], end: tuple[int, int]) -> None:
    move(window, start)
    xdo("mousedown", "1")
    try:
        for step in (1, 2, 3):
            move(window, (start[0] + (end[0] - start[0]) * step // 3,
                          start[1] + (end[1] - start[1]) * step // 3))
    finally:
        xdo("mouseup", "1")


def at_of(app, node: str) -> list[int]:
    return next(n for n in board(app, elements=True)["nodes"] if n["id"] == node)["at"]


def settled(app, check, description: str):
    return wait_for(lambda: (lambda state: state if check(state) else None)(board(app, elements=True)),
                    timeout=5, interval=0.025, description=description)


@pytest.mark.native_x11
def test_native_drag_marquee_and_resize(samples) -> None:
    with board_app(samples, "board-native-edit") as app:
        window, placed = native_board(app)
        zoom = placed["viewport"]["zoom"]
        # A drag moves the shape by whole cells.
        start = screen_of(placed, PACK)
        native_drag(window, start, (start[0] + round(60 * zoom), start[1]))
        settled(app, lambda state: state["state"] == "idle" and at_of(app, PACK) == [60, 840],
                "the shape dragged")
        # A marquee from empty canvas selects what it encloses.
        start_box = next(n["bounds"] for n in placed["nodes"] if n["id"] == START)
        receive = next(n["bounds"] for n in placed["nodes"] if n["id"] == RECEIVE)
        corner = (round(start_box["x"] - 12), round(start_box["y"] - 12))
        far = (round(receive["x"] + receive["width"] + 12), round(receive["y"] + receive["height"] + 12))
        native_drag(window, corner, far)
        state = settled(app, lambda state: state["state"] == "idle" and len(state["selection"]) == 3,
                        "the marquee's selection")
        assert set(state["selection"]) >= {START, RECEIVE}, state
        # A click selects one node, and its south-east handle resizes it.
        move(window, screen_of(bounds(app), PACK))
        xdo("click", "1")
        settled(app, lambda state: state["selection"] == [PACK], "the click's selection")
        handle = next(h["bounds"] for h in bounds(app)["handles"] if h["handle"] == "se")
        grip = (round(handle["x"] + handle["width"] / 2), round(handle["y"] + handle["height"] / 2))
        native_drag(window, grip, (grip[0] + round(40 * zoom), grip[1] + round(20 * zoom)))
        settled(app, lambda state: next(n for n in state["nodes"] if n["id"] == PACK)["size"] == [160, 80],
                "the resize")


@pytest.mark.native_x11
def test_native_double_click_type_and_escape(samples) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-native-label") as app:
        window, placed = native_board(app)
        move(window, screen_of(placed, PACK))
        xdo("click", "--repeat", "2", "--delay", "80", "1")
        settled(app, lambda state: state["state"] == "editing", "the label open")
        xdo("type", "--delay", "20", "Boxed")
        xdo("key", "Escape")
        state = settled(app, lambda state: state["state"] == "idle", "the label committed")
        assert next(n for n in state["nodes"] if n["id"] == PACK)["text"] == "Boxed"
        assert column(app) == "editor"
        app.client.call("core.apply", {"op": "flush"})
        assert b'text = "Boxed"' in flow.read_bytes()


@pytest.mark.native_x11
def test_native_board_keys(samples) -> None:
    """The real keys r, Tab, Ctrl+Right, Alt+Right, BackSpace, Ctrl+Z,
    Ctrl+C and Ctrl+V on a focused board."""
    with board_app(samples, "board-native-keys") as app:
        window, placed = native_board(app)
        move(window, screen_of(placed, RECEIVE))
        xdo("click", "1")
        settled(app, lambda state: state["selection"] == [RECEIVE], "the click's selection")

        xdo("key", "r")
        settled(app, lambda state: state["tool"] == "rectangle", "r armed the tool")
        xdo("key", "Escape")
        settled(app, lambda state: state["tool"] == "select", "Escape put it away")

        xdo("key", "alt+Right")
        settled(app, lambda state: at_of(app, RECEIVE) == [20, 120], "Alt+Right moved it")
        xdo("key", "ctrl+z")
        settled(app, lambda state: at_of(app, RECEIVE) == [0, 120], "Ctrl+Z undid it")

        xdo("key", "Tab")
        state = settled(app, lambda state: state["state"] == "editing", "Tab made the next shape")
        assert state["counts"] == {"nodes": 16, "edges": 18}, state
        xdo("key", "Escape")
        settled(app, lambda state: state["state"] == "idle", "Escape committed it")
        # The slot right of Start is free, so Ctrl+Right makes a shape there.
        move(window, screen_of(bounds(app), START))
        xdo("click", "1")
        settled(app, lambda state: state["selection"] == [START], "Start selected")
        xdo("key", "ctrl+Right")
        state = settled(app, lambda state: state["state"] == "editing", "Ctrl+Right made a shape")
        assert state["counts"] == {"nodes": 17, "edges": 19}, state
        made = state["selection"]
        xdo("key", "Escape")
        settled(app, lambda state: state["state"] == "idle" and state["selection"] == made,
                "Escape committed it")
        xdo("key", "BackSpace")
        settled(app, lambda state: state["counts"] == {"nodes": 16, "edges": 18}, "BackSpace deleted it")

        move(window, screen_of(bounds(app), START))
        xdo("click", "1")
        settled(app, lambda state: state["selection"] == [START], "Start selected")
        xdo("key", "ctrl+c")
        wait_for(lambda: app.client.call("clipboard.read").startswith("# wren-board fragment 1"),
                 timeout=5, interval=0.05, description="Ctrl+C copied")
        xdo("key", "ctrl+v")
        settled(app, lambda state: state["counts"]["nodes"] == 17, "Ctrl+V pasted")
        assert column(app) == "editor"


@pytest.mark.native_x11
def test_native_a_letter_in_the_palette_arms_nothing(samples) -> None:
    with board_app(samples, "board-native-palette") as app:
        window, placed = native_board(app)
        move(window, point_in(placed["root"]))
        xdo("click", "1")
        wait_focus(app, ["note-editor"])
        xdo("key", "alt+shift+p")
        wait_focus(app, ["palette-query"])
        xdo("type", "--delay", "20", "rdo")
        wait_for(lambda: app.client.call("navigation.get")["modal"]["query"] == "rdo", timeout=3,
                 interval=0.025, description="the letters typed into the palette")
        assert board(app)["tool"] == "select"
        xdo("key", "Escape")
        wait_focus(app, ["note-editor"])
        assert board(app)["tool"] == "select"


REJECT = "pa6md7qp"


@pytest.mark.native_x11
def test_native_drag_from_a_connect_handle_onto_another_shape(samples) -> None:
    """The real pointer rests on a node until its connect handles show,
    then drags from one onto another node (§5.5 rule 8, §5.12 rule 1)."""
    with board_app(samples, "board-native-connect") as app:
        window, placed = native_board(app)
        # A click gives the board the keyboard, without which no handle
        # shows; then the pointer comes onto Receive order and rests.
        move(window, screen_of(placed, START))
        xdo("click", "1")
        settled(app, lambda state: state["selection"] == [START], "Start selected")
        move(window, screen_of(placed, RECEIVE))
        handles = wait_for(
            lambda: [h for h in bounds(app)["handles"]
                     if h["id"] == RECEIVE and h["handle"].startswith("connect_")],
            timeout=3, interval=0.025, description="the connect handles shown")
        assert len(handles) == 4, handles
        handle = next(h["bounds"] for h in handles if h["handle"] == "connect_right")
        grip = (round(handle["x"] + handle["width"] / 2), round(handle["y"] + handle["height"] / 2))
        before = {edge["id"] for edge in board(app, elements=True)["edges"]}
        native_drag(window, grip, screen_of(placed, REJECT))
        state = settled(app, lambda state: state["state"] == "idle" and state["counts"]["edges"] == 18,
                        "the connector drawn")
        (made,) = [edge for edge in state["edges"] if edge["id"] not in before]
        # Released in the middle of the node, the end floats.
        assert (made["from"]["node"], made["to"]["node"], made["to"]["side"]) == (RECEIVE, REJECT, None)
        assert state["counts"]["nodes"] == 15
        assert column(app) == "editor"


@pytest.mark.native_x11
def test_native_keys_build_the_first_four_nodes_of_the_acceptance_script(samples) -> None:
    """§5.20's first line and the start of its second with real keys, on
    a new board: `U Tab` Start, `Tab` Receive order, `Esc D Tab` Valid?,
    `Esc ⌘→` Reject order (Ctrl on Linux)."""
    with board_app(samples, "board-native-script") as app:
        open_note(app, NOTES)
        app.client.call("command.run", {"id": "note.new_board"})
        app.client.call("dialog.name", {"name": "Typed"})
        app.client.call("dialog.submit")
        wait_for(lambda: board(app)["surface"] == "board", timeout=10, interval=0.05,
                 description="the new board open")
        windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
        assert len(windows) == 1, windows
        xdo("windowfocus", "--sync", windows[0])
        wait_focus(app, ["note-editor"])

        def typed(text: str) -> None:
            xdo("type", "--delay", "20", text)
            settled(app, lambda state: state["editing"] and state["editing"]["text"] == text,
                    f"{text!r} typed")

        xdo("key", "--delay", "0", "u", "Tab")
        typed("Start")
        xdo("key", "Tab")
        typed("Receive order")
        xdo("key", "--delay", "0", "Escape", "d", "Tab")
        typed("Valid?")
        xdo("key", "--delay", "0", "Escape", "ctrl+Right")
        typed("Reject order")
        xdo("key", "Escape")
        state = settled(app, lambda state: state["state"] == "idle", "the last label committed")

        assert state["counts"] == {"nodes": 4, "edges": 3}, state
        nodes = {node["text"]: node for node in state["nodes"]}
        start = nodes["Start"]["at"]
        cells = {text: (node["shape"], [(node["at"][0] - start[0]) // 20, (node["at"][1] - start[1]) // 20])
                 for text, node in nodes.items()}
        # The keyboard model's layout (golden/accept/checkout-flow.toml).
        assert cells == {"Start": ("pill", [0, 0]), "Receive order": ("rect", [0, 6]),
                         "Valid?": ("diamond", [0, 12]), "Reject order": ("rect", [10, 12])}
        ids = {node["id"]: text for text, node in nodes.items()}
        links = {(ids[edge["from"]["node"]], ids[edge["to"]["node"]]) for edge in state["edges"]}
        assert links == {("Start", "Receive order"), ("Receive order", "Valid?"),
                         ("Valid?", "Reject order")}
        assert state["selection"] == [nodes["Reject order"]["id"]]


@pytest.mark.native_x11
def test_native_a_refused_paste_leaves_the_board_the_keyboard(samples) -> None:
    """§5.15 rule 4: the banner a refused paste raises does not take the
    keyboard, so the next real key still reaches the board."""
    with board_app(samples, "board-native-notice") as app:
        window, placed = native_board(app)
        app.client.call("test.clipboard", {"own": True})
        app.client.call("clipboard.write", {"text": "# wren-board fragment 1\nschema = 2\n"})
        move(window, screen_of(placed, RECEIVE))
        xdo("click", "1")
        settled(app, lambda state: state["selection"] == [RECEIVE], "the click's selection")
        xdo("key", "ctrl+v")
        wait_for(lambda: app.client.call("session.get")["banner"], timeout=5, interval=0.025,
                 description="the refused paste's notice")
        xdo("key", "alt+Right")
        settled(app, lambda state: at_of(app, RECEIVE) == [20, 120], "Alt+Right moved it")
        wait_focus(app, ["note-editor"])
