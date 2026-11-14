"""Arranging a board: stacking, align and distribute (discovery/boards.md
§5.16 rules 2 and 3, §7.4 items AR1 to AR3).

Each case flushes, keeps the bytes, acts, flushes again and reads the line
diff: a restack changes only the moved elements' `z` lines, and align and
distribute only `at` lines. The keys go in through `board.key`, the
widget's own seam; align and distribute have no default key and run as the
palette runs them, through `command.run`.
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, OVERLAP, board, board_app, bounds, click, copy_samples, fitted, flushed, inside, key,
    line_diff, near, open_note, screenshot, type_text,
)
from tools.wrentest.test_board_view import REST, wait_focus
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp

# Checkout flow's elements.
START, RECEIVE, PACK, BACKORDER = "ahj860xh", "12axfsny", "3qrg4kv1", "7kfsgr04"


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
