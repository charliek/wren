"""Finding things on a board (discovery/boards.md §5.18): the jump palette
that Find, Outline and `board.jump` open on a board, and where a workspace
search hit on a board lands.

Neither changes the board: the selection and the view are never written to
the file, so every case also checks the bytes, the mtime and the draft.
`mod` is the command key of the lane's `linux-modifier` (⌘ on macOS),
which app and board commands share.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, NOTES, board, board_app, bounds, click, copy_samples, fitted, key, note_id, open_note,
    stamp,
)
from tools.wrentest.test_board_view import column, wait_focus
from tools.wrentest.ui import WrenApp, wait_for

START, PACK = "ahj860xh", "3qrg4kv1"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def ready(app: WrenApp, path: str = FLOW) -> dict:
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    return fitted(app)


def modal(app: WrenApp) -> dict | None:
    return app.client.call("navigation.get")["modal"]


def node_named(app: WrenApp, text: str) -> str:
    return next(n["id"] for n in board(app, elements=True)["nodes"] if n["text"] == text)


def off_centre(app: WrenApp, node: str) -> tuple[float, float]:
    """How far the node's middle is drawn from the middle of the board
    widget, in logical pixels."""
    placed = bounds(app)
    box = next(n["bounds"] for n in placed["nodes"] if n["id"] == node)
    root = placed["root"]
    return (box["x"] + box["width"] / 2 - (root["x"] + root["width"] / 2),
            box["y"] + box["height"] / 2 - (root["y"] + root["height"] / 2))


def assert_centred(app: WrenApp, node: str) -> None:
    dx, dy = off_centre(app, node)
    assert abs(dx) < 1 and abs(dy) < 1, (dx, dy)


def assert_untouched(app: WrenApp, path: Path, before: tuple[bytes, int]) -> None:
    assert stamp(path) == before
    assert app.client.call("session.get")["document"]["dirty"] is False


def test_the_jump_palette_shows_each_node_and_closing_puts_the_board_back(samples: Path) -> None:
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-find-jump") as app:
        ready(app)
        click(app, {"node": START})
        state = board(app)
        origin, revision = state["viewport"], state["revision"]
        assert state["selection"] == [START]

        # Ctrl+F on a focused board is the app's Find, which opens the jump
        # palette there; it opens on the cursor and shows nothing yet.
        key(app, "f", "mod")
        palette = modal(app)
        assert palette["mode"] == "board_nodes", palette
        wait_focus(app, ["palette-query"])
        rows = palette["results"]
        assert [row["title"] for row in rows[:3]] == ["Start", "Receive order", "Valid?"]
        assert rows[0]["tag"] == "pill" and rows[0]["current"], rows[0]
        assert rows[2]["tag"] == "diamond"
        # Fifteen nodes and the six branch connectors, Yes or No.
        assert len(rows) == 21
        assert sorted(row["title"] for row in rows if row["tag"] == "connector") == \
            ["No", "No", "No", "Yes", "Yes", "Yes"]
        assert palette["active_key"] == {"element": {"node": START}}
        assert board(app)["viewport"] == origin

        # A query highlights its best row, which is selected and centred
        # behind the palette at the zoom the board had.
        app.client.call("palette.query", {"query": "payment"})
        shown = node_named(app, modal(app)["results"][0]["title"])
        state = board(app)
        assert state["selection"] == [shown] and state["cursor"] == shown
        assert state["viewport"]["zoom"] == origin["zoom"]
        assert_centred(app, shown)
        app.client.call("palette.step", {"delta": 1})
        second = node_named(app, modal(app)["results"][1]["title"])
        assert board(app)["selection"] == [second]
        assert_centred(app, second)

        app.client.call("palette.close")
        state = board(app)
        assert state["selection"] == [START] and state["cursor"] == START
        assert state["viewport"] == origin
        assert state["revision"] == revision and state["pristine"]

        # Escape does the same, and the Outline key opens it too.
        key(app, "o", "mod", "shift")
        assert modal(app)["mode"] == "board_nodes"
        app.client.call("palette.query", {"query": "ship"})
        assert board(app)["selection"] == [node_named(app, "Ship")]
        app.client.call("key.nav", {"key": "escape"})
        assert modal(app) is None
        assert board(app)["selection"] == [START]
        assert board(app)["viewport"] == origin
        assert_untouched(app, flow, before)


def test_enter_keeps_the_node_and_gives_the_board_the_keyboard(samples: Path) -> None:
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-find-enter") as app:
        ready(app)
        app.client.call("command.run", {"id": "board.jump"})
        app.client.call("palette.query", {"query": "pack"})
        app.client.call("palette.activate")
        assert modal(app) is None
        wait_focus(app, ["note-editor"])
        assert column(app) == "editor"
        state = board(app)
        assert state["selection"] == [PACK] and state["state"] == "idle", state
        assert_centred(app, PACK)
        # The board has the keyboard: an arrow walks from the node chosen.
        state = key(app, "down")
        assert state["handled"] and state["selection"] != [PACK], state
        assert_untouched(app, flow, before)


def test_a_note_change_closes_the_jump_palette(samples: Path) -> None:
    with board_app(samples, "board-find-switch") as app:
        ready(app)
        app.client.call("command.run", {"id": "board.jump"})
        app.client.call("palette.step", {"delta": 3})
        assert modal(app)["mode"] == "board_nodes"
        open_note(app, NOTES)
        wait_for(lambda: modal(app) is None, timeout=5, description="the palette closed")
        assert board(app)["surface"] == "text"


def test_a_search_hit_lands_on_the_first_node_holding_the_query(samples: Path) -> None:
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-find-search") as app:
        open_note(app, NOTES)
        app.client.call("palette.open", {"mode": "search"})
        app.client.call("palette.query", {"query": "PAYMENT"})
        note = note_id(app, FLOW)
        hit = next(row for row in modal(app)["results"] if row["note"] == note)
        assert hit["hit"] == "content", hit
        app.client.call("palette.activate", {"key": hit["key"]})
        wait_for(lambda: board(app)["surface"] == "board", timeout=10,
                 description="the board open")
        # "Payment ok?" and "Retry payment" share a row; the left one is
        # first in reading order. It is centred once the widget has its
        # size, after the fit a board opened for the first time waits for.
        payment = node_named(app, "Payment ok?")
        assert board(app)["selection"] == [payment]
        assert board(app)["cursor"] == payment

        def centred() -> bool:
            dx, dy = off_centre(app, payment)
            return abs(dx) < 1 and abs(dy) < 1

        wait_for(centred, timeout=10, interval=0.05, description="the hit centred")
        assert_untouched(app, flow, before)
