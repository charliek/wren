"""Arranging a board: stacking, align and distribute (discovery/boards.md
§5.16 rules 2 and 3, §7.4 items AR1 to AR3), and the actions palette
(§5.17, item AR4).

Each case flushes, keeps the bytes, acts, flushes again and reads the line
diff: a restack changes only the moved elements' `z` lines, and align and
distribute only `at` lines. The keys go in through `board.key`, the
widget's own seam; align and distribute have no default key and run as the
palette runs them, through `command.run`. The actions palette opens with
`/` and with a right-click, through `board.pointer` and, in the X11 lane,
through xdotool.
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, OVERLAP, board, board_app, bounds, click, copy_samples, fitted, flushed, inside, key,
    line_diff, near, open_note, pointer, screenshot, type_text, wheel,
)
from tools.wrentest.test_board_native import native_board
from tools.wrentest.test_board_view import REST, wait_focus
from tools.wrentest.test_editor import xdo
from tools.wrentest.test_gestures import move, point_in
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

# Checkout flow's elements.
START, RECEIVE, PACK, BACKORDER = "ahj860xh", "12axfsny", "3qrg4kv1", "7kfsgr04"
# The connector from Start to Receive order.
START_RECEIVE = "4x2tte9d"


# A point of Overlap's back shape that no other shape covers.
BACK = {"canvas": [20, 10]}


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def ready(app: WrenApp, path: str) -> dict:
    """Open a board, give it the keyboard and wait for its fit."""
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    return fitted(app)


def command(app: WrenApp, id_: str) -> dict:
    return app.client.call("command.run", {"id": id_})


def dirty(app: WrenApp) -> bool:
    return app.client.call("session.get")["document"]["dirty"]


def test_the_stacking_keys_rewrite_only_the_moved_z_line(samples: Path) -> None:
    """`]`, `[`, Shift+`]` and Shift+`[` on Overlap: the shape at the back
    comes to the front, where it covers the middle one, and each step is
    one `z` line; at the end of the order a key does nothing, and undo gives
    the bytes back."""
    overlap = samples / OVERLAP
    artifacts = artifact_directory("board-arrange-stacking")
    with board_app(samples, "board-arrange-stacking") as app:
        placed = ready(app, OVERLAP)
        original = overlap.read_bytes()
        nodes = {node["id"]: node for node in placed["nodes"]}
        back, middle = nodes["back"], nodes["middle"]
        # A point inside both, clear of every label and outline.
        box = middle["bounds"]
        candidates = (
            (box["x"] + dx, box["y"] + dy) for dy in range(4, 60, 2) for dx in range(4, 200, 2)
        )
        point = next(
            point for point in candidates
            if inside(point, back["bounds"], -4)
            and not inside(point, nodes["front"]["bounds"], 4)
            and not any(inside(point, node["label"], 4) for node in placed["nodes"])
        )

        def colour(name: str) -> tuple[int, ...]:
            time.sleep(REST)
            image = screenshot(app, artifacts / f"{name}.png")
            scale = bounds(app)["scale_factor"]
            return image.pixel(round(point[0] * scale), round(point[1] * scale))

        assert near(colour("before"), middle["fill"], 8)
        click(app, BACK)
        state = key(app, "]")
        assert state["handled"] and state["selection"] == ["back"], state
        flushed(app)
        assert line_diff(original, overlap.read_bytes()) == ['- z = "V01"', '+ z = "V04"']
        key(app, "escape")
        assert near(colour("front"), back["fill"], 8)

        # On top already: `]` and Shift+`]` change nothing and add no step.
        click(app, BACK)
        assert key(app, "]")["revision"] == 1
        assert key(app, "]", "shift")["revision"] == 1
        # One step back passes `front`; to the back again takes the key
        # below `middle`, which is the one it had.
        key(app, "[", "shift")
        flushed(app)
        assert line_diff(original, overlap.read_bytes()) == ['- z = "V01"', '+ z = "V02V"']
        state = key(app, "[")
        flushed(app)
        assert overlap.read_bytes() == original
        assert state["revision"] == 3 and state["pristine"] and not dirty(app), state
        for _ in range(3):
            key(app, "z", "mod")
        flushed(app)
        assert overlap.read_bytes() == original and not dirty(app)


def test_align_and_distribute_change_at_lines_once(samples: Path) -> None:
    """Align on two nodes and distribute on three move `at` lines and no
    connector line; a second run changes nothing; undo gives the bytes
    back. With too few nodes selected they are blocked."""
    flow = samples / FLOW
    with board_app(samples, "board-arrange-align") as app:
        ready(app, FLOW)
        original = flow.read_bytes()
        click(app, {"node": START})
        assert command(app, "board.align.left")["disposition"] == "blocked"
        # Start is at 0 and Backorder at 200, both 120 wide.
        click(app, {"node": BACKORDER}, mods=("shift",))
        assert command(app, "board.distribute.vertical")["disposition"] == "blocked"
        assert command(app, "board.align.right")["disposition"] == "completed"
        flushed(app)
        assert line_diff(original, flow.read_bytes()) == ["- at = [0, 0]", "+ at = [200, 0]"]
        revision = board(app)["revision"]
        assert command(app, "board.align.right")["disposition"] == "completed"
        assert board(app)["revision"] == revision
        key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == original and not dirty(app)

        # Start ends at 60 and Pack begins at 840: Receive order goes
        # half way, at 420, and they stay.
        for node in (START, RECEIVE, PACK):
            click(app, {"node": node}, mods=("shift",) if node != START else ())
        assert command(app, "board.distribute.vertical")["disposition"] == "completed"
        flushed(app)
        assert line_diff(original, flow.read_bytes()) == ["- at = [0, 120]", "+ at = [0, 420]"]
        revision = board(app)["revision"]
        command(app, "board.distribute.vertical")
        assert board(app)["revision"] == revision
        key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == original and not dirty(app)


def test_a_bracket_typed_in_a_label_is_text(samples: Path) -> None:
    """`]` and `[` in an open label type themselves and restack nothing."""
    flow = samples / FLOW
    with board_app(samples, "board-arrange-label") as app:
        ready(app, FLOW)
        original = flow.read_bytes()
        click(app, {"node": PACK})
        assert key(app, "enter")["state"] == "editing"
        type_text(app, "x")
        for bracket in ("]", "["):
            assert key(app, bracket)["state"] == "editing"
        state = key(app, "]", "shift")
        assert state["editing"]["text"] == "x][]", state
        key(app, "enter", "mod")
        flushed(app)
        diff = line_diff(original, flow.read_bytes())
        assert diff and all(line[2:].startswith("text = ") for line in diff), diff


def modal(app: WrenApp) -> dict | None:
    return app.client.call("navigation.get")["modal"]


def actions_open(app: WrenApp) -> bool:
    shown = modal(app)
    return shown is not None and shown["mode"] == "board_actions"


def open_actions(app: WrenApp) -> dict:
    """`/` on the focused board: the actions palette for the selection."""
    assert key(app, "/")["handled"]
    assert actions_open(app)
    return modal(app)


def close_actions(app: WrenApp) -> None:
    """Escape, as the palette's input passes it on, and the keyboard back
    on the board."""
    app.client.call("key.nav", {"key": "escape"})
    assert modal(app) is None
    wait_focus(app, ["note-editor"])


def listing(shown: dict) -> list[tuple[str, str]]:
    return [(row["group"], row["key"]["command"]) for row in shown["results"]]


def groups(shown: dict) -> list[str]:
    seen: list[str] = []
    for group, _ in listing(shown):
        if not seen or seen[-1] != group:
            seen.append(group)
    return seen


def current(shown: dict) -> list[str]:
    return [row["key"]["command"] for row in shown["results"] if row["current"]]


def test_the_actions_palette_lists_the_commands_the_selection_takes(samples: Path) -> None:
    """§5.17 rule 3 for nothing selected, one node, one connector and a
    mixed selection: only the commands the selection takes, grouped, with
    the selection's own values current and the first of them highlighted.
    Listing writes nothing."""
    flow = samples / FLOW
    with board_app(samples, "board-actions-listing") as app:
        ready(app, FLOW)
        original = flow.read_bytes()

        shown = open_actions(app)
        assert listing(shown) == [
            ("Board", "board.paste"), ("Board", "board.select.all"),
            ("Board", "board.zoom.fit"), ("Board", "note.export_svg"),
        ]
        close_actions(app)

        click(app, {"node": START})
        shown = open_actions(app)
        assert groups(shown) == ["Shape", "Colour", "Fill", "Text size", "Stroke", "Arrange",
                                 "Edit"]
        assert current(shown) == ["board.shape.pill", "board.color.neutral", "board.fill.tint",
                                  "board.text_size.14"]
        assert shown["active_key"] == {"command": "board.shape.pill"}
        rows = {row["key"]["command"]: row for row in shown["results"]}
        assert rows["board.color.red"]["shortcut"] == "1"
        assert [command for group, command in listing(shown) if group == "Edit"] == [
            "board.label.edit", "board.duplicate", "board.copy", "board.cut", "board.delete",
        ]
        close_actions(app)

        click(app, {"edge": START_RECEIVE})
        shown = open_actions(app)
        assert groups(shown) == ["Colour", "Stroke", "Connector", "Arrange", "Edit"]
        assert current(shown) == ["board.color.neutral"]
        close_actions(app)

        for node in (START, RECEIVE, PACK):
            click(app, {"node": node}, mods=("shift",))
        shown = open_actions(app)
        assert groups(shown) == [
            "Shape", "Colour", "Fill", "Text size", "Stroke", "Connector", "Arrange", "Align",
            "Distribute", "Edit",
        ]
        # Start is a pill and the others rectangles: no shape is current.
        assert current(shown) == ["board.color.neutral", "board.fill.tint", "board.text_size.14"]
        assert shown["active_key"] == {"command": "board.color.neutral"}
        # `/` and `]` typed into the palette are its query.
        typed = app.client.call("palette.query", {"query": "/]"})["navigation"]["modal"]
        assert typed["query"] == "/]" and typed["mode"] == "board_actions"
        close_actions(app)
        flushed(app)
        assert flow.read_bytes() == original and not dirty(app)
        assert board(app)["revision"] == 0


def test_a_right_click_previews_and_only_enter_writes(samples: Path) -> None:
    """§5.17 rules 2 and 4 through `board.pointer`: a right-click selects
    what it is on and opens the palette; a highlighted colour is drawn on
    the board and never written; Escape takes it away; Enter writes the one
    changed line, which undo takes back."""
    flow = samples / FLOW
    artifacts = artifact_directory("board-actions-preview")
    with board_app(samples, "board-actions-preview") as app:
        ready(app, FLOW)
        original = flow.read_bytes()
        # Pack alone, at 100 % in the middle of the board, clear of the
        # palette, which sits over the browser columns.
        click(app, {"node": PACK})
        assert app.client.call("command.run", {"id": "board.zoom.selection"})["disposition"] \
            == "completed"
        click(app, {"node": START})
        placed = {node["id"]: node for node in bounds(app)["nodes"]}
        box = placed[PACK]["bounds"]
        point = (box["x"] + 12, box["y"] + box["height"] / 2)

        def pixel(name: str) -> tuple[int, ...]:
            time.sleep(REST)
            image = screenshot(app, artifacts / f"{name}.png")
            scale = bounds(app)["scale_factor"]
            return image.pixel(round(point[0] * scale), round(point[1] * scale))

        state = pointer(app, "click", {"node": PACK}, button="right")
        assert state["selection"] == [PACK] and actions_open(app), state
        assert board(app)["patch"] is None
        opened = pixel("opened")

        shown = app.client.call("palette.query", {"query": "colour: red"})["navigation"]["modal"]
        assert shown["active_key"] == {"command": "board.color.red"}
        assert board(app)["patch"] == "board.color.red"
        previewed = pixel("previewed")
        assert not near(previewed, opened, 8), (previewed, opened)
        flushed(app)
        assert flow.read_bytes() == original and not dirty(app)

        # The current colour shows the board as it is.
        app.client.call("palette.query", {"query": "colour: neutral"})
        assert board(app)["patch"] is None
        assert near(pixel("current"), opened, 2)

        app.client.call("palette.query", {"query": "colour: red"})
        close_actions(app)
        state = board(app)
        assert state["patch"] is None and state["revision"] == 0 and not state["can_undo"]
        flushed(app)
        assert flow.read_bytes() == original and not dirty(app)

        # On a selected element the selection stays; Enter applies.
        pointer(app, "click", {"node": PACK}, button="right")
        app.client.call("palette.query", {"query": "colour: red"})
        app.client.call("key.nav", {"key": "enter"})
        assert modal(app) is None
        wait_focus(app, ["note-editor"])
        state = board(app)
        assert state["revision"] == 1 and state["patch"] is None, state
        flushed(app)
        assert line_diff(original, flow.read_bytes()) == ['+ style.color = "red"']
        key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == original and not dirty(app)

        # On empty canvas the selection clears, and the palette offers what
        # needs none.
        state = pointer(app, "click", {"canvas": [-2000, -2000]}, button="right")
        assert state["selection"] == [] and actions_open(app), state
        assert listing(modal(app))[0] == ("Board", "board.paste")
        close_actions(app)


@pytest.mark.native_x11
def test_native_right_click_opens_the_actions_on_the_element_under_the_pointer(
    samples: Path,
) -> None:
    """A real right-click (§5.17 rule 2) selects the element under the
    pointer and opens the palette with the keyboard in its input, where `/`
    and `]` type; after a pan it still finds the element under it."""
    flow = samples / FLOW
    with board_app(samples, "board-actions-native") as app:
        window, placed = native_board(app, FLOW)
        wait_focus(app, ["note-editor"])
        original = flow.read_bytes()

        def right_click(node: str) -> None:
            where = {each["id"]: each for each in bounds(app)["nodes"]}[node]
            move(window, point_in(where["visible"]))
            xdo("click", "3")
            wait_for(lambda: actions_open(app), timeout=5, interval=0.025,
                     description="the actions palette opened")
            assert board(app)["selection"] == [node]

        right_click(PACK)
        wait_focus(app, ["palette-query"])
        xdo("type", "/]")
        wait_for(lambda: modal(app)["query"] == "/]", timeout=5, interval=0.025,
                 description="the palette took the text")
        close_actions(app)

        # After a pan and a zoom the element under the pointer is the one
        # drawn there.
        wheel(app, {"node": RECEIVE}, -3)
        wheel(app, {"node": RECEIVE}, 2, mods=("mod",))
        time.sleep(REST)
        right_click(RECEIVE)
        close_actions(app)
        flushed(app)
        assert flow.read_bytes() == original and board(app)["revision"] == 0
