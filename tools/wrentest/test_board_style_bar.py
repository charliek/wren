"""The board's style bar and the footer's controls (discovery/board-controls.md
§3.2, §3.3, §3.6 and D4): `board.style_bar` reports each control with the
command it runs, whether it is current and where it is, and with `hover` or
`click` sends what the pointer there sends; `board.footer` does the same for
the footer's undo, redo and zoom. The `native_x11` cases move and click the
real pointer with xdotool, at scale factor 1 (the X11 lanes' Xvfb).
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, FUTURE, board, board_app, click, copy_samples, fitted, flushed, key, near,
    screenshot, stamp,
)
from tools.wrentest.client import RemoteError
from tools.wrentest.test_board_dock import focused_board, inside, native, press_on
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.test_editor import xdo
from tools.wrentest.test_gestures import move, point_in
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

START, PACK, SHIP, EDGE = "ahj860xh", "3qrg4kv1", "1wg3aw6z", "zpsr8rp7"

SWATCHES = ["board-style-neutral", "board-style-red", "board-style-orange",
            "board-style-yellow", "board-style-green", "board-style-cyan",
            "board-style-purple", "board-style-blue"]


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def style_bar(app: WrenApp, **params) -> dict:
    return app.client.call("board.style_bar", params)


def footer(app: WrenApp, click_id: str | None = None) -> dict:
    return app.client.call("board.footer", {"click": click_id} if click_id else {})


def control(state: dict, control_id: str) -> dict:
    return next(each for each in state["controls"] if each["id"] == control_id)


def footer_control(state: dict, control_id: str) -> dict:
    return next(each for each in state["controls"] if each["id"] == control_id)


def laid_out(app: WrenApp, check=lambda state: True) -> dict:
    """The bar once the frame that shows its state has placed it."""
    def ready():
        state = style_bar(app)
        placed = (state["shown"] and state["rect"]
                  and all(each["rect"] for each in state["controls"])
                  and (state["menu"] is None or state["menu"]["rect"]))
        return state if placed and check(state) else None

    return wait_for(ready, timeout=5, interval=0.025, description="the style bar laid out")


def footer_laid_out(app: WrenApp, check=lambda state: True) -> dict:
    def ready():
        state = footer(app)
        placed = state["controls"] and all(each["rect"] for each in state["controls"])
        return state if placed and check(state) else None

    return wait_for(ready, timeout=5, interval=0.025, description="the footer laid out")


def node(app: WrenApp, node_id: str) -> dict:
    return next(each for each in board(app, elements=True)["nodes"] if each["id"] == node_id)


def colour(app: WrenApp, node_id: str) -> str:
    return node(app, node_id)["style"]["color"]


def select(app: WrenApp, *ids: str) -> None:
    for index, id_ in enumerate(ids):
        at = {"edge": id_} if id_ == EDGE else {"node": id_}
        click(app, at, mods=() if index == 0 else ("shift",))


def test_the_bar_shows_what_applies_to_the_selection(samples: Path) -> None:
    with board_app(samples, "board-style-bar") as app:
        placed = focused_board(app)
        assert style_bar(app)["shown"] is False
        select(app, START)
        state = laid_out(app)
        ids = [each["id"] for each in state["controls"]]
        assert ids == (["board-style-shape"] + SWATCHES
                       + ["board-style-fill-none", "board-style-fill-tint",
                          "board-style-fill-solid", "board-style-text-size",
                          "board-style-dashed", "board-style-more"])
        assert state["new_shapes"] is False and state["folded"] is False
        assert [each["id"] for each in state["controls"] if each["current"]] == [
            "board-style-shape", "board-style-neutral", "board-style-fill-tint"]
        assert control(state, "board-style-shape")["shape"] == "pill"
        assert control(state, "board-style-red")["command"] == "board.color.red"
        assert control(state, "board-style-red")["tip"] == "Red   1"
        assert control(state, "board-style-more")["command"] is None
        assert all(each["badge"] is None for each in state["controls"])

        # Fixed at the top centre of the canvas, inside it.
        root, rect = placed["root"], state["rect"]
        assert inside(rect, root)
        assert abs((rect["x"] + rect["width"] / 2) - (root["x"] + root["width"] / 2)) <= 1
        assert rect["y"] - root["y"] == pytest.approx(12, abs=1)
        assert all(inside(each["rect"], rect) for each in state["controls"])
        # A fitted board starts below it: the selected shape at the top is
        # not under the bar.
        top = min(each["bounds"]["y"] for each in placed["nodes"])
        assert top > rect["y"] + rect["height"]

        # A connector: its colour, route, heads and outline; no shape.
        select(app, EDGE)
        state = laid_out(app, lambda state: "board-style-route" in
                         [each["id"] for each in state["controls"]])
        ids = [each["id"] for each in state["controls"]]
        assert ids == SWATCHES + ["board-style-route", "board-style-start", "board-style-end",
                                  "board-style-reverse", "board-style-dashed", "board-style-more"]
        # A shape and a connector: both. With every control of both the bar
        # is wider than this canvas, so the styles are in the last menu.
        select(app, PACK, EDGE)
        state = laid_out(app, lambda state: "board-style-shape" in
                         [each["id"] for each in state["controls"]])
        assert state["folded"] is True
        style_bar(app, click="board-style-more")
        state = laid_out(app, lambda state: state["menu"] is not None)
        commands = [row["command"] for row in state["menu"]["rows"]]
        assert {"board.edge.routing", "board.fill.solid", "board.text_size.20"} <= set(commands)
        style_bar(app, click="board-style-more")
        laid_out(app, lambda state: state["menu"] is None)
        # Nothing selected: no bar.
        key(app, "escape")
        assert style_bar(app)["shown"] is False
        assert board(app)["pristine"] is True


def test_a_click_runs_its_command_as_one_undo_step_and_the_colour_sticks(samples: Path) -> None:
    with board_app(samples, "board-style-click") as app:
        focused_board(app)
        select(app, PACK)
        assert board(app)["next_color"] == "neutral"
        state = style_bar(app, click="board-style-red")
        assert colour(app, PACK) == "red"
        assert control(state, "board-style-red")["current"] is True
        assert board(app)["next_color"] == "red"
        wait_focus(app, ["note-editor"])
        key(app, "z", "mod")
        assert colour(app, PACK) == "neutral"
        assert board(app)["pristine"] is True
        # The fill, a reshape from the shape menu, and the last menu.
        style_bar(app, click="board-style-fill-solid")
        assert node(app, PACK)["style"]["fill"] == "solid"
        state = style_bar(app, click="board-style-shape")
        assert state["menu"]["kind"] == "shapes"
        assert [row["command"] for row in state["menu"]["rows"]][:2] == [
            "board.shape.rectangle", "board.shape.ellipse"]
        assert [row["current"] for row in state["menu"]["rows"]][0] is True
        state = style_bar(app, click="board-style-menu-ellipse")
        assert state["menu"] is None
        assert node(app, PACK)["shape"] == "ellipse"
        state = style_bar(app, click="board-style-more")
        assert [row["command"] for row in state["menu"]["rows"]] == [
            "board.duplicate", "board.arrange.front", "board.arrange.back", "board.delete",
            "board.actions"]
        style_bar(app, click="board-style-menu-actions")
        assert app.client.call("navigation.get")["modal"]["mode"] == "board_actions"
        app.client.call("palette.close")
        for _ in range(2):
            key(app, "z", "mod")
        assert board(app)["pristine"] is True
        with pytest.raises(RemoteError, match="no board-style-menu-rounded"):
            style_bar(app, click="board-style-menu-rounded")

        # A connector's colour is its own: the next shape's stays red.
        select(app, PACK)
        style_bar(app, click="board-style-red")
        select(app, EDGE)
        style_bar(app, click="board-style-blue")
        assert board(app)["next_color"] == "red"


def test_with_nothing_selected_a_shape_tool_shows_the_colour_new_shapes_take(
        samples: Path) -> None:
    with board_app(samples, "board-style-new-shapes") as app:
        focused_board(app)
        key(app, "r")
        state = laid_out(app)
        assert state["new_shapes"] is True
        assert [each["id"] for each in state["controls"]] == SWATCHES
        assert [each["id"] for each in state["controls"] if each["current"]] == [
            "board-style-neutral"]
        # A colour picked changes nothing on the board, and the tool stays.
        style_bar(app, click="board-style-green")
        state = board(app)
        assert (state["next_color"], state["tool"], state["pristine"]) == ("green", "rectangle",
                                                                            True)
        assert app.client.call("session.get")["document"]["dirty"] is False
        # The digit key does the same with nothing selected.
        key(app, "6")
        assert board(app)["next_color"] == "purple"
        assert board(app)["pristine"] is True
        # The shape placed takes it, and so does Tab's next one.
        before = {each["id"] for each in board(app, elements=True)["nodes"]}
        click(app, {"canvas": [-400, 100]})
        key(app, "escape")
        made = [each for each in board(app, elements=True)["nodes"] if each["id"] not in before]
        assert [each["style"]["color"] for each in made] == ["purple"]
        key(app, "tab")
        key(app, "escape")
        made = [each for each in board(app, elements=True)["nodes"] if each["id"] not in before]
        assert sorted(each["style"]["color"] for each in made) == ["purple", "purple"]
        # The select tool: no bar.
        key(app, "escape")
        key(app, "escape")
        assert style_bar(app)["shown"] is False


def test_hovering_a_control_previews_it_and_writes_nothing(samples: Path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-style-hover") as app:
        focused_board(app)
        flushed(app)
        before = stamp(flow)
        select(app, PACK)
        state = style_bar(app, hover="board-style-purple")
        assert state["hover"] == "board.color.purple"
        assert board(app)["patch"] == "board.color.purple"
        assert board(app)["pristine"] is True
        state = style_bar(app, hover=None)
        assert state["hover"] is None and board(app)["patch"] is None
        # A fill and a shape row preview; the current colour does not.
        style_bar(app, hover="board-style-fill-solid")
        assert board(app)["patch"] == "board.fill.solid"
        style_bar(app, hover="board-style-neutral")
        assert board(app)["patch"] is None
        style_bar(app, click="board-style-shape")
        style_bar(app, hover="board-style-menu-diamond")
        assert board(app)["patch"] == "board.shape.diamond"
        style_bar(app, hover=None)
        with pytest.raises(RemoteError, match="previews nothing"):
            style_bar(app, hover="board-style-dashed")
        flushed(app)
        assert stamp(flow) == before
        assert board(app)["pristine"] is True


def test_badges_show_each_swatch_s_digit_while_space_is_held(samples: Path) -> None:
    with board_app(samples, "board-style-badges") as app:
        focused_board(app)
        select(app, PACK)
        key(app, "space")
        state = style_bar(app)
        assert [control(state, each)["badge"] for each in SWATCHES] == [
            str(digit) for digit in range(8)]
        assert control(state, "board-style-dashed")["badge"] is None
        key(app, "space", release=True)
        assert all(each["badge"] is None for each in style_bar(app)["controls"])


def test_a_narrow_canvas_folds_the_styles_into_the_last_menu(samples: Path) -> None:
    with board_app(samples, "board-style-fold") as app:
        focused_board(app)
        app.client.call("view.width", {"pane": "notes", "width": 480})
        app.client.call("column.open", {"column": "sections"})
        app.client.call("editor.focus")
        select(app, PACK, EDGE)
        state = laid_out(app, lambda state: state["folded"])
        assert [each["id"] for each in state["controls"]] == (
            ["board-style-shape"] + SWATCHES + ["board-style-more"])
        root = fitted(app)["root"]
        assert inside(state["rect"], root)
        state = style_bar(app, click="board-style-more")
        state = laid_out(app, lambda state: state["menu"] is not None)
        commands = [row["command"] for row in state["menu"]["rows"]]
        assert commands[:3] == ["board.fill.none", "board.fill.tint", "board.fill.solid"]
        assert "board.edge.routing" in commands and commands[-1] == "board.actions"
        assert inside(state["menu"]["rect"], root)
        style_bar(app, click="board-style-menu-fill-solid")
        assert node(app, PACK)["style"]["fill"] == "solid"


def test_a_read_only_board_has_no_bar_and_no_undo(samples: Path) -> None:
    with board_app(samples, "board-style-read-only") as app:
        focused_board(app, FUTURE)
        assert board(app)["read_only"] is True
        app.client.call("command.run", {"id": "board.select.all"})
        assert style_bar(app)["shown"] is False
        state = footer_laid_out(app)
        assert [each["id"] for each in state["controls"]] == [
            "board-footer-zoom-out", "board-footer-zoom", "board-footer-zoom-in"]


def test_the_setting_off_shows_no_bar_and_the_plain_zoom(samples: Path) -> None:
    with WrenApp(workspace=samples, settings="boards = on\nboard-toolbar = off\n",
                 artifacts_dir=artifact_directory("board-style-off")) as app:
        focused_board(app)
        select(app, PACK)
        assert style_bar(app)["shown"] is False
        with pytest.raises(RemoteError, match="no style bar"):
            style_bar(app, click="board-style-red")
        assert footer(app)["controls"] == []
        # The key still sets the colour new shapes take.
        key(app, "escape")
        key(app, "3")
        assert board(app)["next_color"] == "yellow"


def test_the_footer_undoes_redoes_and_zooms(samples: Path) -> None:
    with board_app(samples, "board-footer") as app:
        focused_board(app)
        state = footer_laid_out(app)
        assert [each["id"] for each in state["controls"]] == [
            "board-footer-undo", "board-footer-redo", "board-footer-zoom-out",
            "board-footer-zoom", "board-footer-zoom-in"]
        assert [each["enabled"] for each in state["controls"]] == [False, False, True, True,
                                                                    True]
        assert footer_control(state, "board-footer-undo")["tip"] == "Undo   Ctrl+Z"
        assert footer_control(state, "board-footer-zoom")["command"] == "board.zoom.fit"
        assert footer_control(state, "board-footer-zoom-in")["command"] == "font.increase"
        fitted_zoom = board(app)["viewport"]["zoom"]
        assert state["zoom"] == f"{round(fitted_zoom * 100)}%"
        # Greyed, a click does nothing.
        footer(app, "board-footer-undo")
        assert board(app)["pristine"] is True

        select(app, PACK)
        key(app, "1")
        state = footer(app)
        assert footer_control(state, "board-footer-undo")["enabled"] is True
        footer(app, "board-footer-undo")
        assert colour(app, PACK) == "neutral"
        assert footer_control(footer(app), "board-footer-redo")["enabled"] is True
        footer(app, "board-footer-redo")
        assert colour(app, PACK) == "red"
        wait_focus(app, ["note-editor"])

        footer(app, "board-footer-zoom-in")
        assert board(app)["viewport"]["zoom"] > fitted_zoom
        footer(app, "board-footer-zoom")
        assert board(app)["viewport"]["zoom"] == pytest.approx(fitted_zoom)
        footer(app, "board-footer-zoom-out")
        assert board(app)["viewport"]["zoom"] < fitted_zoom
        wait_focus(app, ["note-editor"])


def test_with_boards_off_there_is_no_bar(samples: Path) -> None:
    with board_app(samples, "board-style-boards-off", setting="off") as app:
        with pytest.raises(RemoteError, match="not shown as a board"):
            style_bar(app)


# Real pointer input.

def pixel_at(image, x: float, y: float) -> tuple[int, ...]:
    return image.pixel(round(x), round(y))[:3]


@pytest.mark.native_x11
def test_native_a_swatch_click_recolours_the_selection_as_one_undo_step(samples,
                                                                         tmp_path) -> None:
    with board_app(samples, "board-style-native-click") as app:
        window, placed = native(app)
        nodes = {each["id"]: each["bounds"] for each in placed["nodes"]}
        press_on(window, nodes[PACK])
        wait_for(lambda: board(app)["selection"] == [PACK], timeout=3, interval=0.025,
                 description="the click selected Pack")
        state = laid_out(app)
        press_on(window, control(state, "board-style-green")["rect"])
        wait_for(lambda: colour(app, PACK) == "green", timeout=3, interval=0.025,
                 description="the swatch recoloured Pack")
        # The board kept the keyboard and the selection.
        wait_focus(app, ["note-editor"])
        assert board(app)["selection"] == [PACK]
        assert board(app)["next_color"] == "green"
        # The ring moved to it, drawn in the accent.
        move(window, point_in(placed["root"], vertical=0.95))
        state = laid_out(app, lambda state: control(state, "board-style-green")["current"])
        image = screenshot(app, tmp_path / "ringed.png")
        for swatch_id, ringed in (("board-style-green", True), ("board-style-red", False)):
            rect = control(state, swatch_id)["rect"]
            # The top of the ring, half a ring's width inside its edge.
            centre_x = rect["x"] + rect["width"] / 2
            ring_y = rect["y"] + rect["height"] / 2 - 10.5
            assert near(pixel_at(image, centre_x, ring_y), state["ring"], 40) is ringed, swatch_id
        # One undo step takes it back.
        xdo("key", "ctrl+z")
        wait_for(lambda: colour(app, PACK) == "neutral", timeout=3, interval=0.025,
                 description="one undo took the colour back")
        assert board(app)["pristine"] is True


@pytest.mark.native_x11
def test_native_hovering_a_swatch_previews_it_and_writes_nothing(samples, tmp_path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-style-native-hover") as app:
        window, placed = native(app)
        nodes = {each["id"]: each["bounds"] for each in placed["nodes"]}
        press_on(window, nodes[PACK])
        wait_for(lambda: board(app)["selection"] == [PACK], timeout=3, interval=0.025,
                 description="the click selected Pack")
        flushed(app)
        before = stamp(flow)
        state = laid_out(app)
        # Inside Pack, clear of its label: its fill.
        inner = (nodes[PACK]["x"] + 4, nodes[PACK]["y"] + 4)
        plain = screenshot(app, tmp_path / "plain.png")
        move(window, point_in(control(state, "board-style-purple")["rect"]))
        wait_for(lambda: board(app)["patch"] == "board.color.purple", timeout=3,
                 interval=0.025, description="the hover previewed purple")
        shown = screenshot(app, tmp_path / "shown.png")
        assert pixel_at(shown, *inner) != pixel_at(plain, *inner)
        assert board(app)["pristine"] is True
        # Off the swatch, onto the bar's own ground, the preview goes.
        rect = state["rect"]
        move(window, (round(rect["x"] + 3), round(rect["y"] + rect["height"] / 2)))
        wait_for(lambda: board(app)["patch"] is None, timeout=3, interval=0.025,
                 description="leaving dropped the preview")
        back = screenshot(app, tmp_path / "back.png")
        assert pixel_at(back, *inner) == pixel_at(plain, *inner)
        flushed(app)
        assert stamp(flow) == before
        # Clicked, it is what the preview showed.
        press_on(window, control(state, "board-style-purple")["rect"])
        wait_for(lambda: colour(app, PACK) == "purple", timeout=3, interval=0.025,
                 description="the click applied purple")
        move(window, point_in(placed["root"], vertical=0.95))
        applied = screenshot(app, tmp_path / "applied.png")
        assert near(pixel_at(applied, *inner), pixel_at(shown, *inner), 2)


@pytest.mark.native_x11
def test_native_the_shape_menu_reshapes_by_real_clicks(samples) -> None:
    with board_app(samples, "board-style-native-shape") as app:
        window, placed = native(app)
        nodes = {each["id"]: each["bounds"] for each in placed["nodes"]}
        press_on(window, nodes[SHIP])
        wait_for(lambda: board(app)["selection"] == [SHIP], timeout=3, interval=0.025,
                 description="the click selected Ship")
        state = laid_out(app)
        press_on(window, control(state, "board-style-shape")["rect"])
        state = laid_out(app, lambda state: state["menu"] is not None)
        row = next(each for each in state["menu"]["rows"]
                   if each["id"] == "board-style-menu-cylinder")
        press_on(window, row["rect"])
        wait_for(lambda: node(app, SHIP)["shape"] == "cylinder", timeout=3, interval=0.025,
                 description="the row reshaped Ship")
        assert style_bar(app)["menu"] is None
        wait_focus(app, ["note-editor"])
        assert board(app)["can_undo"] is True


@pytest.mark.native_x11
def test_native_a_press_on_the_bar_starts_no_marquee(samples) -> None:
    with board_app(samples, "board-style-native-press") as app:
        window, placed = native(app)
        nodes = {each["id"]: each["bounds"] for each in placed["nodes"]}
        press_on(window, nodes[PACK])
        wait_for(lambda: board(app)["selection"] == [PACK], timeout=3, interval=0.025,
                 description="the click selected Pack")
        state = laid_out(app)
        # Between the shape button and the first swatch: the bar's own rule.
        shape = control(state, "board-style-shape")["rect"]
        swatch = control(state, "board-style-neutral")["rect"]
        rule = (round((shape["x"] + shape["width"] + swatch["x"]) / 2),
                round(shape["y"] + shape["height"] / 2))
        move(window, rule)
        xdo("mousedown", "1")
        try:
            move(window, (rule[0] + 60, rule[1] + 220))
            assert board(app)["state"] == "idle"
        finally:
            xdo("mouseup", "1")
        assert board(app)["selection"] == [PACK]
        wait_focus(app, ["note-editor"])
        assert app.client.call("session.get")["document"]["dirty"] is False


@pytest.mark.native_x11
def test_native_a_click_on_a_greyed_footer_control_leaves_the_board_as_it_was(samples) -> None:
    """A greyed undo takes its press: one that fell through to the board
    would be a click off it, which takes the keyboard from it and puts the
    armed tool away."""
    with board_app(samples, "board-footer-greyed") as app:
        window, placed = native(app)
        nodes = {each["id"]: each["bounds"] for each in fitted(app)["nodes"]}
        press_on(window, nodes[PACK])
        wait_for(lambda: board(app)["selection"] == [PACK], timeout=3, interval=0.025,
                 description="the click selected Pack")
        xdo("key", "d")
        wait_for(lambda: board(app)["tool"] == "diamond", timeout=3, interval=0.025,
                 description="D armed the diamond")
        state = footer_laid_out(app)
        for control in ("board-footer-undo", "board-footer-redo"):
            assert footer_control(state, control)["enabled"] is False
            press_on(window, footer_control(state, control)["rect"])
        # Whatever the clicks did has been done once a key gets through.
        xdo("key", "1")
        wait_for(lambda: colour(app, PACK) == "red", timeout=3, interval=0.025,
                 description="the key still reached the board")
        assert board(app)["tool"] == "diamond"
        assert board(app)["state"] == "armed"
        wait_focus(app, ["note-editor"])


@pytest.mark.native_x11
def test_native_the_footer_s_zoom_and_undo_work_by_real_clicks(samples) -> None:
    with board_app(samples, "board-footer-native") as app:
        window, placed = native(app)
        fitted_zoom = board(app)["viewport"]["zoom"]
        state = footer_laid_out(app)
        press_on(window, footer_control(state, "board-footer-zoom-in")["rect"])
        wait_for(lambda: board(app)["viewport"]["zoom"] > fitted_zoom, timeout=3,
                 interval=0.025, description="zoom in")
        # The board keeps the keyboard: a key still reaches it.
        wait_focus(app, ["note-editor"])
        press_on(window, footer_control(state, "board-footer-zoom")["rect"])
        wait_for(lambda: board(app)["viewport"]["zoom"] == pytest.approx(fitted_zoom),
                 timeout=3, interval=0.025, description="zoom to fit")
        press_on(window, footer_control(state, "board-footer-zoom-out")["rect"])
        wait_for(lambda: board(app)["viewport"]["zoom"] < fitted_zoom, timeout=3,
                 interval=0.025, description="zoom out")
        press_on(window, footer_control(state, "board-footer-zoom")["rect"])
        wait_for(lambda: board(app)["viewport"]["zoom"] == pytest.approx(fitted_zoom),
                 timeout=3, interval=0.025, description="zoom to fit again")

        # A shape armed stays armed through a footer click.
        nodes = {each["id"]: each["bounds"] for each in fitted(app)["nodes"]}
        press_on(window, nodes[PACK])
        wait_for(lambda: board(app)["selection"] == [PACK], timeout=3, interval=0.025,
                 description="the click selected Pack")
        xdo("key", "1")
        wait_for(lambda: colour(app, PACK) == "red", timeout=3, interval=0.025,
                 description="the key coloured Pack")
        xdo("key", "d")
        wait_for(lambda: board(app)["tool"] == "diamond", timeout=3, interval=0.025,
                 description="D armed the diamond")
        state = footer_laid_out(app, lambda state: footer_control(
            state, "board-footer-undo")["enabled"])
        press_on(window, footer_control(state, "board-footer-undo")["rect"])
        wait_for(lambda: colour(app, PACK) == "neutral", timeout=3, interval=0.025,
                 description="the undo button took the colour back")
        assert board(app)["tool"] == "diamond"
        wait_focus(app, ["note-editor"])
        # The key goes to the board, which still has it.
        xdo("key", "Escape")
        wait_for(lambda: board(app)["tool"] == "select", timeout=3, interval=0.025,
                 description="Escape reached the board")


@pytest.mark.native_x11
def test_native_a_click_in_a_browser_column_takes_the_keyboard_from_the_board(samples) -> None:
    """The bars float over the board in layers, and a press a column's filter
    takes still reaches the board beneath them, which lets the keyboard go."""
    with board_app(samples, "board-style-native-blur") as app:
        window, placed = native(app)
        nodes = {each["id"]: each["bounds"] for each in placed["nodes"]}
        press_on(window, nodes[PACK])
        laid_out(app)
        wait_focus(app, ["note-editor"])
        field = app.client.call("focus.get")["bounds"]["filter-notes"]
        press_on(window, field)
        wait_focus(app, ["filter-notes"])
        assert board(app)["state"] == "unfocused"
