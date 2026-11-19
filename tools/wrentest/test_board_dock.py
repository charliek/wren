"""The board's tool dock (discovery/board-controls.md §3.1, §3.3, D3):
`board.dock` reports each button with the command it runs, where it is, and
whether it is lit, locked and showing its key. `board.dock` with `click`
sends what a click on a button sends; the `native_x11` cases click the real
buttons with xdotool, at scale factor 1 (the X11 lanes' Xvfb).
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, FUTURE, board, board_app, click, copy_samples, fitted, key, open_note, screenshot,
    type_text,
)
from tools.wrentest.client import RemoteError
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.test_editor import xdo
from tools.wrentest.test_gestures import move, point_in
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

PACK = "3qrg4kv1"

WIDE = ["board-dock-select", "board-dock-hand", "board-dock-rectangle", "board-dock-ellipse",
        "board-dock-diamond", "board-dock-pill", "board-dock-cylinder",
        "board-dock-parallelogram", "board-dock-more", "board-dock-text", "board-dock-connector",
        "board-dock-keys"]


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def dock(app: WrenApp, click_id: str | None = None) -> dict:
    params = {"click": click_id} if click_id else {}
    return app.client.call("board.dock", params)


def button(state: dict, button_id: str) -> dict:
    return next(each for each in state["buttons"] if each["id"] == button_id)


def laid_out(app: WrenApp, check=lambda state: True) -> dict:
    """The dock once the frame that shows its state has placed it."""
    def ready():
        state = dock(app)
        placed = state["shown"] and state["rect"] and all(each["rect"] for each in state["buttons"])
        return state if placed and check(state) else None

    return wait_for(ready, timeout=5, interval=0.025, description="the dock laid out")


def focused_board(app: WrenApp, path: str = FLOW) -> dict:
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    return fitted(app)


def inside(inner: dict, outer: dict) -> bool:
    return (outer["x"] <= inner["x"] and inner["x"] + inner["width"] <= outer["x"] + outer["width"]
            and outer["y"] <= inner["y"]
            and inner["y"] + inner["height"] <= outer["y"] + outer["height"])


def test_the_dock_lists_every_tool_and_lights_the_armed_one(samples: Path) -> None:
    with board_app(samples, "board-dock") as app:
        placed = focused_board(app)
        state = laid_out(app)
        assert [each["id"] for each in state["buttons"]] == WIDE
        assert state["folded"] is False and state["menu"] is None
        assert button(state, "board-dock-rectangle")["command"] == "board.tool.rectangle"
        assert button(state, "board-dock-keys")["command"] == "board.keys"
        assert button(state, "board-dock-more")["command"] is None
        # Each tooltip names the tool and its key from the keymap.
        assert button(state, "board-dock-rectangle")["tip"] == "Rectangle   R"
        assert button(state, "board-dock-connector")["key"] == "C"
        assert button(state, "board-dock-more")["key"] is None
        # Badges wait for Space.
        assert all(each["badge"] is None for each in state["buttons"])
        assert [each["id"] for each in state["buttons"] if each["lit"]] == ["board-dock-select"]

        # The bar floats at the bottom centre of the canvas, inside it.
        root, bar = placed["root"], state["rect"]
        assert inside(bar, root)
        assert abs((bar["x"] + bar["width"] / 2) - (root["x"] + root["width"] / 2)) <= 1
        assert root["y"] + root["height"] - (bar["y"] + bar["height"]) == pytest.approx(12, abs=1)
        assert all(inside(each["rect"], bar) for each in state["buttons"])
        # A fitted board ends above it.
        nodes = [node["bounds"] for node in placed["nodes"]]
        assert max(node["y"] + node["height"] for node in nodes) < bar["y"]

        # A key lights its tool's button; armed again it is locked.
        key(app, "d")
        state = laid_out(app, lambda state: button(state, "board-dock-diamond")["lit"])
        assert [each["id"] for each in state["buttons"] if each["lit"]] == ["board-dock-diamond"]
        assert button(state, "board-dock-diamond")["locked"] is False
        key(app, "d")
        state = laid_out(app, lambda state: button(state, "board-dock-diamond")["locked"])
        assert board(app)["locked"] is True
        key(app, "escape")
        state = laid_out(app, lambda state: button(state, "board-dock-select")["lit"])
        assert not any(each["locked"] for each in state["buttons"])
        assert app.client.call("session.get")["document"]["dirty"] is False


def test_a_click_runs_the_command_its_key_does(samples: Path) -> None:
    with board_app(samples, "board-dock-click") as app:
        focused_board(app)
        before = board(app)["counts"]["nodes"]
        dock(app, "board-dock-rectangle")
        state = board(app)
        assert (state["tool"], state["locked"]) == ("rectangle", False)
        # The board keeps the keyboard.
        wait_focus(app, ["note-editor"])
        dock(app, "board-dock-rectangle")
        assert board(app)["locked"] is True
        # Locked, the tool makes two in a row.
        for at in ([-400, 100], [-400, 300]):
            click(app, {"canvas": at})
            key(app, "escape")
            assert board(app)["tool"] == "rectangle"
        assert board(app)["counts"]["nodes"] == before + 2
        key(app, "escape")
        assert (board(app)["tool"], board(app)["locked"]) == ("select", False)

        # The hand stays armed after its drag.
        dock(app, "board-dock-hand")
        app.client.call("board.pointer", {"kind": "press", "at": {"canvas": [-300, 200]},
                                          "button": "left", "mods": []})
        app.client.call("board.pointer", {"kind": "move", "at": {"canvas": [-200, 260]},
                                          "button": "left", "mods": []})
        app.client.call("board.pointer", {"kind": "release", "button": "left", "mods": []})
        assert board(app)["tool"] == "hand"

        # The "more shapes" menu holds the two shapes with no key.
        state = dock(app, "board-dock-more")
        assert state["menu"]["kind"] == "more"
        assert [row["command"] for row in state["menu"]["rows"]] == [
            "board.tool.rounded", "board.tool.document"]
        assert [row["key"] for row in state["menu"]["rows"]] == [None, None]
        state = dock(app, "board-dock-menu-document")
        assert state["menu"] is None
        assert board(app)["tool"] == "document"
        assert button(state, "board-dock-more")["lit"] is True
        # Its button again closes its menu, and a press on the canvas does.
        assert dock(app, "board-dock-more")["menu"] is not None
        assert dock(app, "board-dock-more")["menu"] is None
        dock(app, "board-dock-more")
        key(app, "escape")
        assert dock(app)["menu"] is None
        with pytest.raises(RemoteError, match="no board-dock-menu-rounded"):
            dock(app, "board-dock-menu-rounded")

        # The last button opens the keys sheet.
        dock(app, "board-dock-keys")
        assert board(app)["keys_sheet"] is not None
        app.client.call("palette.close")


def test_a_click_commits_an_open_label_first(samples: Path) -> None:
    with board_app(samples, "board-dock-label") as app:
        focused_board(app)
        click(app, {"node": PACK})
        key(app, "enter")
        type_text(app, "Packed")
        assert board(app)["state"] == "editing"
        dock(app, "board-dock-ellipse")
        state = board(app, elements=True)
        assert state["editing"] is None and state["tool"] == "ellipse"
        assert next(node for node in state["nodes"] if node["id"] == PACK)["text"] == "Packed"


def test_badges_show_while_space_is_held(samples: Path) -> None:
    with board_app(samples, "board-dock-badges") as app:
        focused_board(app)
        key(app, "space")
        state = dock(app)
        assert button(state, "board-dock-rectangle")["badge"] == "R"
        assert button(state, "board-dock-keys")["badge"] == "?"
        assert button(state, "board-dock-more")["badge"] is None
        key(app, "space", release=True)
        assert all(each["badge"] is None for each in dock(app)["buttons"])


@pytest.mark.parametrize("badges, held, shown, hinted", [
    ("on", False, True, False),
    ("off", True, False, False),
])
def test_the_badges_setting(samples: Path, badges: str, held: bool, shown: bool,
                            hinted: bool) -> None:
    with WrenApp(workspace=samples, settings=f"boards = on\nboard-key-badges = {badges}\n",
                 artifacts_dir=artifact_directory(f"board-dock-badges-{badges}")) as app:
        focused_board(app)
        if held:
            key(app, "space")
        state = dock(app)
        assert (button(state, "board-dock-text")["badge"] == "T") is shown
        words = [part["words"] for part in board(app)["hint"]]
        assert ("show keys" in words) is hinted


def test_a_narrow_canvas_folds_the_shapes_into_one_button(samples: Path) -> None:
    with board_app(samples, "board-dock-fold") as app:
        focused_board(app)
        app.client.call("view.width", {"pane": "notes", "width": 480})
        app.client.call("column.open", {"column": "sections"})
        state = laid_out(app, lambda state: state["folded"])
        assert fitted(app)["root"]["width"] < 560
        assert [each["id"] for each in state["buttons"]] == [
            "board-dock-select", "board-dock-hand", "board-dock-shapes", "board-dock-text",
            "board-dock-connector", "board-dock-keys"]
        assert inside(state["rect"], fitted(app)["root"])
        state = dock(app, "board-dock-shapes")
        assert state["menu"]["kind"] == "shapes"
        assert [row["key"] for row in state["menu"]["rows"]] == [
            "R", "O", "D", "U", "Y", "P", None, None]
        state = laid_out(app, lambda state: state["menu"] and state["menu"]["rect"])
        assert inside(state["menu"]["rect"], fitted(app)["root"])
        state = dock(app, "board-dock-menu-cylinder")
        assert board(app)["tool"] == "cylinder"
        assert state["menu"] is None
        assert button(state, "board-dock-shapes")["lit"] is True


def test_a_read_only_board_docks_only_what_it_takes(samples: Path) -> None:
    with board_app(samples, "board-dock-read-only") as app:
        focused_board(app, FUTURE)
        state = laid_out(app)
        assert board(app)["read_only"] is True
        assert [each["id"] for each in state["buttons"]] == [
            "board-dock-select", "board-dock-hand", "board-dock-keys"]
        dock(app, "board-dock-hand")
        assert board(app)["tool"] == "hand"


def test_the_setting_off_leaves_the_canvas_as_it_was(samples: Path) -> None:
    with WrenApp(workspace=samples, settings="boards = on\nboard-toolbar = off\n",
                 artifacts_dir=artifact_directory("board-dock-off")) as app:
        focused_board(app)
        assert dock(app)["shown"] is False
        with pytest.raises(RemoteError, match="no dock"):
            dock(app, "board-dock-select")
        # No dock, so Space shows no keys.
        assert "show keys" not in [part["words"] for part in board(app)["hint"]]


def test_with_boards_off_there_is_no_dock(samples: Path) -> None:
    with board_app(samples, "board-dock-boards-off", setting="off") as app:
        with pytest.raises(RemoteError, match="not shown as a board"):
            dock(app)


# Real buttons, real keys.

def native(app: WrenApp) -> tuple[str, dict]:
    from tools.wrentest.test_board_native import native_board

    window, placed = native_board(app)
    move(window, point_in(placed["root"]))
    xdo("click", "2")
    wait_focus(app, ["note-editor"])
    return window, placed


def press_on(window: str, rect: dict) -> None:
    move(window, point_in(rect))
    xdo("click", "1")


@pytest.mark.native_x11
def test_native_a_shape_button_then_the_canvas_places_that_shape(samples) -> None:
    with board_app(samples, "board-dock-native-place") as app:
        window, placed = native(app)
        root = placed["root"]
        state = laid_out(app)
        before = board(app)["counts"]["nodes"]
        press_on(window, button(state, "board-dock-ellipse")["rect"])
        wait_for(lambda: board(app)["tool"] == "ellipse", timeout=3, interval=0.025,
                 description="the click armed the ellipse")
        wait_focus(app, ["note-editor"])
        # Again: locked.
        press_on(window, button(state, "board-dock-ellipse")["rect"])
        wait_for(lambda: board(app)["locked"], timeout=3, interval=0.025,
                 description="the second click locked it")
        # Two placements on empty canvas, left of the flow, with no re-arming.
        for down in (0.25, 0.55):
            move(window, (round(root["x"] + 90), round(root["y"] + root["height"] * down)))
            xdo("click", "1")
            wait_for(lambda: board(app)["state"] == "editing", timeout=3, interval=0.025,
                     description="the new shape's label open")
            assert board(app)["tool"] == "ellipse"
        xdo("key", "Escape")
        elements = board(app, elements=True)
        assert elements["counts"]["nodes"] == before + 2
        made = [node for node in elements["nodes"] if node["shape"] == "ellipse"]
        assert len(made) >= 2
        xdo("key", "Escape")
        wait_for(lambda: board(app)["tool"] == "select", timeout=3, interval=0.025,
                 description="Escape put the tool away")


@pytest.mark.native_x11
def test_native_a_press_on_the_dock_reaches_no_canvas_beneath_it(samples) -> None:
    with board_app(samples, "board-dock-native-press") as app:
        window, placed = native(app)
        state = laid_out(app)
        hand = button(state, "board-dock-hand")["rect"]
        shape = button(state, "board-dock-rectangle")["rect"]
        # The rule between the hand and the rectangle is the bar's own.
        rule = (round((hand["x"] + hand["width"] + shape["x"]) / 2),
                round(hand["y"] + hand["height"] / 2))
        move(window, rule)
        xdo("mousedown", "1")
        try:
            move(window, (rule[0] - 80, rule[1] - 160))
            assert board(app)["state"] == "idle"
        finally:
            xdo("mouseup", "1")
        assert board(app)["selection"] == []
        wait_focus(app, ["note-editor"])
        # With a shape tool armed, a click on another button arms that one
        # and makes nothing beneath it.
        before = board(app)["counts"]["nodes"]
        xdo("key", "r")
        wait_for(lambda: board(app)["tool"] == "rectangle", timeout=3, interval=0.025,
                 description="R armed the rectangle")
        press_on(window, button(state, "board-dock-text")["rect"])
        wait_for(lambda: board(app)["tool"] == "text", timeout=3, interval=0.025,
                 description="the click armed the text tool")
        assert board(app)["counts"]["nodes"] == before
        wait_focus(app, ["note-editor"])
        assert app.client.call("session.get")["document"]["dirty"] is False


@pytest.mark.native_x11
def test_native_space_shows_the_keys_and_the_lit_button_is_drawn_lit(samples, tmp_path) -> None:
    def region(image, rect: dict, from_right: int, from_bottom: int, size: int) -> list:
        x0 = round(rect["x"] + rect["width"]) - from_right
        y0 = round(rect["y"] + rect["height"]) - from_bottom
        return [image.pixel(x, y)[:3] for x in range(x0, x0 + size) for y in range(y0, y0 + size)]

    with board_app(samples, "board-dock-native-space") as app:
        window, placed = native(app)
        state = laid_out(app)
        rect = button(state, "board-dock-rectangle")["rect"]
        lit = button(state, "board-dock-select")["rect"]
        plain = screenshot(app, tmp_path / "plain.png")
        # The lit button's ground is not the bar's.
        corner = (round(lit["x"]) + 4, round(lit["y"]) + 4)
        bare = (round(rect["x"]) + 4, round(rect["y"]) + 4)
        assert plain.pixel(*corner)[:3] != plain.pixel(*bare)[:3]

        xdo("keydown", "space")
        try:
            wait_for(lambda: button(dock(app), "board-dock-rectangle")["badge"] == "R",
                     timeout=3, interval=0.025, description="the badges shown")
            held = screenshot(app, tmp_path / "held.png")
        finally:
            xdo("keyup", "space")
        wait_for(lambda: button(dock(app), "board-dock-rectangle")["badge"] is None,
                 timeout=3, interval=0.025, description="the badges hidden")
        # The badge's corner of the button changed while Space was held.
        assert region(held, rect, 9, 11, 7) != region(plain, rect, 9, 11, 7)
        after = screenshot(app, tmp_path / "after.png")
        assert region(after, rect, 9, 11, 7) == region(plain, rect, 9, 11, 7)
        assert app.client.call("session.get")["document"]["dirty"] is False

