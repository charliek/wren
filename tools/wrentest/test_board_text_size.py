"""Text size on boards (discovery/board-controls.md §3.9 and D10): the style
bar's "Aa" menu and the actions palette set a node's `style.text-size` to
12, 14, 20 or 28; a hover previews it and writes nothing; a click is one
undo step that adds one line to the file, or a second for a shape that grew
to hold its label. The sample board "Text sizes" draws every preset and a
size past the range, clamped and listed as an issue. The `native_x11` cases
click the real pointer through the menu, at scale factor 1.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, GROUP, board, board_app, bounds, copy_samples, flushed, key, line_diff, screenshot,
    stamp,
)
from tools.wrentest.test_board_arrange import close_actions, current, listing, open_actions
from tools.wrentest.test_board_dock import focused_board, native, press_on
from tools.wrentest.test_board_style_bar import (
    EDGE, PACK, SHIP, laid_out, node, select, style_bar,
)
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.test_editor import xdo
from tools.wrentest.test_gestures import move, point_in
from tools.wrentest.ui import WrenApp, wait_for

SIZES = f"{GROUP}/Text sizes.wren-board"
ROWS = ["board-style-menu-text-size-12", "board-style-menu-text-size-14",
        "board-style-menu-text-size-20", "board-style-menu-text-size-28"]


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def text_size(app: WrenApp, node_id: str) -> int:
    return node(app, node_id)["style"]["text_size"]


def labels(app: WrenApp) -> dict[str, dict]:
    return {each["id"]: each["label"] for each in bounds(app)["nodes"]}


def test_the_aa_menu_previews_then_sets_a_size_as_one_line_and_one_undo(samples: Path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-text-size") as app:
        focused_board(app)
        flushed(app)
        before = stamp(flow)
        select(app, PACK)
        state = laid_out(app)
        ids = [each["id"] for each in state["controls"]]
        assert ids.index("board-style-text-size") == ids.index("board-style-fill-solid") + 1
        aa = next(each for each in state["controls"] if each["id"] == "board-style-text-size")
        assert (aa["command"], aa["tip"]) == (None, "Text size")

        state = style_bar(app, click="board-style-text-size")
        assert state["menu"]["kind"] == "text_size"
        rows = state["menu"]["rows"]
        assert [row["id"] for row in rows] == ROWS
        assert [row["command"] for row in rows] == [
            "board.text_size.12", "board.text_size.14", "board.text_size.20",
            "board.text_size.28"]
        assert [row["current"] for row in rows] == [False, True, False, False]
        assert [row["key"] for row in rows] == [None] * 4

        # A hover previews the size and writes nothing.
        label = labels(app)[PACK]
        style_bar(app, hover="board-style-menu-text-size-28")
        assert board(app)["patch"] == "board.text_size.28"
        assert board(app)["pristine"] is True
        assert labels(app)[PACK] == label
        style_bar(app, hover=None)
        assert board(app)["patch"] is None
        flushed(app)
        assert stamp(flow) == before

        # A click: one line in the file, one undo step, the menu closed and
        # the board still has the keyboard.
        state = style_bar(app, click="board-style-menu-text-size-28")
        assert state["menu"] is None
        assert text_size(app, PACK) == 28
        wait_focus(app, ["note-editor"])
        grown = labels(app)[PACK]
        assert grown["height"] > label["height"] * 1.5
        assert grown["width"] > label["width"] * 1.5
        flushed(app)
        assert line_diff(before[0], flow.read_bytes()) == ["+ style.text-size = 28"]
        state = style_bar(app, click="board-style-text-size")
        assert [row["current"] for row in state["menu"]["rows"]] == [False, False, False, True]
        style_bar(app, click="board-style-text-size")
        key(app, "z", "mod")
        assert text_size(app, PACK) == 14
        assert board(app)["pristine"] is True
        flushed(app)
        assert flow.read_bytes() == before[0]


def test_a_longer_label_grows_its_shape_in_whole_cells_and_a_connector_takes_none(
        samples: Path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-text-size-grow") as app:
        focused_board(app)
        flushed(app)
        before = flow.read_bytes()
        # "Notify customer" does not fit its box at 28: it grows.
        notify = next(each["id"] for each in board(app, elements=True)["nodes"]
                      if each["text"] == "Notify customer")
        size = node(app, notify)["size"]
        select(app, notify)
        style_bar(app, click="board-style-text-size")
        style_bar(app, click="board-style-menu-text-size-28")
        grown = node(app, notify)["size"]
        assert grown != size and all(side % 20 == 0 for side in grown)
        assert grown[0] >= size[0] and grown[1] >= size[1]
        flushed(app)
        changed = line_diff(before, flow.read_bytes())
        assert "+ style.text-size = 28" in changed
        assert [line for line in changed if "size = [" in line] == [
            f"- size = [{size[0]}, {size[1]}]", f"+ size = [{grown[0]}, {grown[1]}]"]
        # Smaller again, it keeps the room it grew to.
        style_bar(app, click="board-style-text-size")
        style_bar(app, click="board-style-menu-text-size-12")
        assert node(app, notify)["size"] == grown
        # A connector alone has no Aa.
        select(app, EDGE)
        state = laid_out(app, lambda state: "board-style-route" in
                         [each["id"] for each in state["controls"]])
        assert "board-style-text-size" not in [each["id"] for each in state["controls"]]


def test_the_actions_palette_offers_the_sizes_with_the_current_one(samples: Path) -> None:
    with board_app(samples, "board-text-size-palette") as app:
        focused_board(app)
        select(app, SHIP)
        app.client.call("command.run", {"id": "board.text_size.20"})
        assert text_size(app, SHIP) == 20
        shown = open_actions(app)
        assert [command for group, command in listing(shown) if group == "Text size"] == [
            "board.text_size.12", "board.text_size.14", "board.text_size.20",
            "board.text_size.28"]
        assert "board.text_size.20" in current(shown)
        rows = {row["key"]["command"]: row for row in shown["results"]}
        assert rows["board.text_size.28"]["shortcut"] == ""
        close_actions(app)
        app.client.call("command.run", {"id": "board.text_size.12"})
        assert text_size(app, SHIP) == 12


def test_a_narrow_canvas_folds_the_sizes_into_the_last_menu(samples: Path) -> None:
    with board_app(samples, "board-text-size-fold") as app:
        focused_board(app)
        app.client.call("view.width", {"pane": "notes", "width": 480})
        app.client.call("column.open", {"column": "sections"})
        app.client.call("editor.focus")
        select(app, PACK, EDGE)
        state = laid_out(app, lambda state: state["folded"])
        assert "board-style-text-size" not in [each["id"] for each in state["controls"]]
        style_bar(app, click="board-style-more")
        state = laid_out(app, lambda state: state["menu"] is not None)
        commands = [row["command"] for row in state["menu"]["rows"]]
        assert commands[3:7] == ["board.text_size.12", "board.text_size.14",
                                 "board.text_size.20", "board.text_size.28"]
        style_bar(app, click="board-style-menu-text-size-20")
        assert text_size(app, PACK) == 20


def test_the_sample_board_draws_each_size_and_flags_one_past_the_range(samples: Path) -> None:
    path = samples / SIZES
    before = stamp(path)
    with board_app(samples, "board-text-size-sample") as app:
        focused_board(app, SIZES)
        state = board(app)
        assert state["issues"] == [{"kind": "out_of_range", "id": "huge", "limit": "text_size"}]
        zoom = state["viewport"]["zoom"]
        drawn = labels(app)
        # One line each: its height is the line height at its size.
        for name, height in (("t12", 16), ("t14", 20), ("t20", 20 * 20 / 14), ("t28", 40)):
            assert drawn[name]["height"] == pytest.approx(height * zoom, abs=0.5), name
        heights = [drawn[name]["height"] for name in ("s12", "s14", "s20", "s28")]
        assert heights == sorted(heights) and len(set(heights)) == 4
        # Opening it never writes it.
        flushed(app)
        assert stamp(path) == before


def test_a_label_is_edited_at_its_size(samples: Path) -> None:
    with board_app(samples, "board-text-size-label") as app:
        focused_board(app, SIZES)
        app.client.call("command.run", {"id": "board.select.none"})
        select(app, "t28")
        key(app, "enter")
        editing = wait_for(lambda: board(app)["editing"], timeout=3, interval=0.025,
                           description="the label opened")
        zoom = board(app)["viewport"]["zoom"]
        assert editing["id"] == "t28"
        assert editing["ime"]["height"] == pytest.approx(40 * zoom, abs=0.01)
        assert editing["ime"]["size"] == pytest.approx(28 * zoom, abs=0.01)
        key(app, "escape")


# Real pointer input.

def ink_rows(image, rect: dict) -> int:
    """How many pixel rows inside `rect`, away from its outline, hold ink:
    a pixel unlike the fill at the rect's inner corner."""
    left, top = round(rect["x"] + 6), round(rect["y"] + 6)
    right, bottom = round(rect["x"] + rect["width"] - 6), round(rect["y"] + rect["height"] - 6)
    fill = image.pixel(left, top)[:3]
    rows = 0
    for y in range(top, bottom):
        if any(max(abs(a - b) for a, b in zip(image.pixel(x, y)[:3], fill)) > 60
               for x in range(left, right)):
            rows += 1
    return rows


@pytest.mark.native_x11
def test_native_the_aa_menu_sets_a_size_by_real_clicks(samples, tmp_path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-text-size-native") as app:
        window, placed = native(app)
        flushed(app)
        before = flow.read_bytes()
        nodes = {each["id"]: each["bounds"] for each in placed["nodes"]}
        press_on(window, nodes[PACK])
        wait_for(lambda: board(app)["selection"] == [PACK], timeout=3, interval=0.025,
                 description="the click selected Pack")
        move(window, point_in(placed["root"], vertical=0.95))
        plain = ink_rows(screenshot(app, tmp_path / "plain.png"), nodes[PACK])
        state = laid_out(app)
        aa = next(each for each in state["controls"] if each["id"] == "board-style-text-size")
        press_on(window, aa["rect"])
        state = laid_out(app, lambda state: state["menu"] is not None)
        assert state["menu"]["kind"] == "text_size"
        row = next(each for each in state["menu"]["rows"]
                   if each["id"] == "board-style-menu-text-size-28")
        # The pointer on the row previews it; the board is not written.
        move(window, point_in(row["rect"]))
        wait_for(lambda: board(app)["patch"] == "board.text_size.28", timeout=3,
                 interval=0.025, description="the row previewed 28")
        assert board(app)["pristine"] is True
        xdo("click", "1")
        wait_for(lambda: text_size(app, PACK) == 28, timeout=3, interval=0.025,
                 description="the row set 28")
        assert style_bar(app)["menu"] is None
        # The board kept the keyboard and the selection.
        wait_focus(app, ["note-editor"])
        assert board(app)["selection"] == [PACK]
        move(window, point_in(placed["root"], vertical=0.95))
        wait_for(lambda: board(app)["patch"] is None, timeout=3, interval=0.025,
                 description="no preview left")
        big = ink_rows(screenshot(app, tmp_path / "big.png"), nodes[PACK])
        assert plain > 0 and big > plain * 1.6, (plain, big)
        flushed(app)
        assert line_diff(before, flow.read_bytes()) == ["+ style.text-size = 28"]
        # One undo step takes it back.
        xdo("key", "ctrl+z")
        wait_for(lambda: text_size(app, PACK) == 14, timeout=3, interval=0.025,
                 description="one undo took the size back")
        assert board(app)["pristine"] is True


@pytest.mark.native_x11
def test_native_a_press_on_the_open_menu_s_ground_starts_no_marquee(samples) -> None:
    """The menu's own ground takes a press: one that fell through would be
    a press on the board, which would start a marquee and drop the
    selection."""
    with board_app(samples, "board-text-size-native-ground") as app:
        window, placed = native(app)
        nodes = {each["id"]: each["bounds"] for each in placed["nodes"]}
        press_on(window, nodes[PACK])
        wait_for(lambda: board(app)["selection"] == [PACK], timeout=3, interval=0.025,
                 description="the click selected Pack")
        state = laid_out(app)
        aa = next(each for each in state["controls"] if each["id"] == "board-style-text-size")
        press_on(window, aa["rect"])
        state = laid_out(app, lambda state: state["menu"] is not None)
        menu = state["menu"]["rect"]
        # The menu's padding, right of the rows.
        ground = (round(menu["x"] + menu["width"] - 2), round(menu["y"] + menu["height"] / 2))
        move(window, ground)
        xdo("mousedown", "1")
        try:
            move(window, (ground[0] + 40, ground[1] + 160))
            assert board(app)["state"] == "idle"
        finally:
            xdo("mouseup", "1")
        assert board(app)["selection"] == [PACK]
        assert text_size(app, PACK) == 14
        wait_focus(app, ["note-editor"])
        assert app.client.call("session.get")["document"]["dirty"] is False
