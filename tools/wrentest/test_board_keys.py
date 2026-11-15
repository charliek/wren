"""Keys on a board (discovery/boards.md §5.1, §5.9, §5.14, §5.15, §6.8
Phase 2 item 7).

`board.key` takes the path a real key takes: the board's key policy from
the keymap, then the app's keyboard path for whatever the board leaves.
These cases run on Linux, where Alt is the app's modifier and Ctrl the
document's (`mod`).
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, NOTES, board, board_app, click, copy_samples, fitted, key, open_note, pointer, press,
    stamp, type_text,
)
from tools.wrentest.test_board_view import column, focused, wait_focus
from tools.wrentest.ui import WrenApp

# The app's modifier on Linux.
APP = "alt"
START, RECEIVE, PACK = "ahj860xh", "12axfsny", "3qrg4kv1"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def ready(app: WrenApp, path: str = FLOW) -> dict:
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    return fitted(app)


def test_a_letter_in_the_palette_or_a_dialog_arms_nothing(samples: Path) -> None:
    with board_app(samples, "board-keys-overlay") as app:
        ready(app)
        app.client.call("palette.open", {"mode": "commands"})
        wait_focus(app, ["palette-query"])
        state = key(app, "r")
        assert not state["handled"] and state["tool"] == "select", state
        app.client.call("palette.query", {"query": "r"})
        app.client.call("palette.close")
        wait_focus(app, ["note-editor"])
        assert board(app)["tool"] == "select"

        app.client.call("command.run", {"id": "note.new"})
        assert app.client.call("navigation.get")["dialog"] is not None
        assert not key(app, "d")["handled"]
        app.client.call("dialog.close")
        assert board(app)["tool"] == "select"
        # And with the board focused again, the letter arms the tool.
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert key(app, "d")["tool"] == "diamond"


def test_in_a_label_a_tool_letter_is_text(samples: Path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-keys-label") as app:
        ready(app)
        click(app, {"node": PACK})
        assert key(app, "enter")["state"] == "editing"
        for letter in "rodut":
            state = key(app, letter)
            assert state["handled"] and state["tool"] == "select", state
        # Shift types a capital, Space a space, Backspace erases, and the
        # app's own shortcuts still reach the app.
        key(app, "c", "shift")
        key(app, "space")
        key(app, "x")
        state = key(app, "backspace")
        assert state["editing"]["text"] == "rodutC "
        assert key(app, "enter", "mod")["state"] == "idle"
        app.client.call("core.apply", {"op": "flush"})
        assert b'text = "rodutC' in flow.read_bytes()


def test_in_keyboard_connecting_a_tool_letter_is_query_text(samples: Path) -> None:
    """§5.1 rule 3: in keyboard Connecting a tool letter types into the
    query, and only Enter, Escape, the arrows and Backspace act."""
    with board_app(samples, "board-keys-query") as app:
        ready(app)
        click(app, {"node": PACK})
        state = key(app, "c")
        assert state["state"] == "connecting" and state["tool"] == "select", state
        assert state["connecting"] == {"target": None, "side": None, "query": ""}, state
        for letter in "che":
            state = key(app, letter)
            assert state["handled"] and state["tool"] == "select", state
        check = next(n["id"] for n in board(app, elements=True)["nodes"]
                     if n["text"] == "Check stock")
        assert state["connecting"] == {"target": check, "side": None, "query": "che"}, state
        # Tool letters, Shift and Space are text too.
        for name, mods in (("d", ()), ("r", ("shift",)), ("space", ())):
            state = key(app, name, *mods)
            assert state["handled"] and state["tool"] == "select", state
        assert state["connecting"] == {"target": None, "side": None, "query": "chedR "}, state
        for _ in range(3):
            state = key(app, "backspace")
        assert state["connecting"]["target"] == check, state
        assert state["connecting"]["query"] == "che", state
        assert state["counts"]["edges"] == 17
        state = key(app, "enter")
        assert state["state"] == "idle" and state["connecting"] is None, state
        assert state["counts"]["edges"] == 18
        made = state["selection"][0]
        made_edge = next(e for e in board(app, elements=True)["edges"] if e["id"] == made)
        floating = {"side": None, "anchor": None}
        assert made_edge["from"] == {"node": PACK, **floating}, made_edge
        assert made_edge["to"] == {"node": check, **floating}, made_edge


def test_a_held_tab_or_delete_acts_once(samples: Path) -> None:
    with board_app(samples, "board-keys-repeat") as app:
        ready(app)
        click(app, {"node": PACK})
        state = key(app, "tab")
        assert state["state"] == "editing" and state["counts"]["nodes"] == 16, state
        for _ in range(3):
            state = key(app, "tab", repeat=True)
        assert state["counts"]["nodes"] == 16 and state["state"] == "editing", state
        key(app, "escape")

        click(app, {"node": START})
        assert key(app, "delete")["counts"]["nodes"] == 15
        click(app, {"node": RECEIVE})
        state = key(app, "delete", repeat=True)
        assert state["counts"]["nodes"] == 15 and state["selection"] == [RECEIVE], state
        # A held arrow repeats: it walks.
        assert key(app, "down", repeat=True)["selection"] != [RECEIVE]


def test_escape_descends_one_rung_a_press_and_never_leaves_the_board(samples: Path) -> None:
    """§5.14: a gesture, a label, an armed tool, a selection, nothing."""
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-keys-escape") as app:
        ready(app)
        click(app, {"node": PACK})
        key(app, "r")
        press(app, {"canvas": [600, 0]})
        assert pointer(app, "move", {"canvas": [700, 100]})["state"] == "creating"
        rungs = []
        for _ in range(4):
            state = key(app, "escape")
            rungs.append((state["handled"], state["state"], state["tool"], state["selection"]))
            assert focused(app) == ["note-editor"] and column(app) == "editor"
        assert rungs == [
            (True, "armed", "rectangle", [PACK]),
            (True, "idle", "select", [PACK]),
            (True, "idle", "select", []),
            (False, "idle", "select", []),
        ], rungs
        # Releasing the button the gesture had changes nothing now.
        assert pointer(app, "release")["counts"]["nodes"] == 15

        # In a label Escape commits, and the next press clears the
        # selection, the cursor staying.
        click(app, {"node": PACK})
        key(app, "enter")
        state = key(app, "escape")
        assert state["state"] == "idle" and state["selection"] == [PACK], state
        state = key(app, "escape")
        assert state["selection"] == [] and state["cursor"] == PACK, state
        assert focused(app) == ["note-editor"] and column(app) == "editor"
        assert not app.client.call("session.get")["document"]["dirty"]
    assert stamp(flow) == before


def test_each_app_shortcut_on_a_focused_board_acts_once(samples: Path) -> None:
    with board_app(samples, "board-keys-app") as app:
        ready(app)
        app.client.call("command.run", {"id": "font.reset"})
        # The zoom steps one stop, not two.
        state = key(app, "=", APP)
        assert not state["handled"] and state["viewport"]["zoom"] == 1.5, state
        state = key(app, "-", APP)
        assert state["viewport"]["zoom"] == 1.0, state
        # The New note dialog opens once, over the board, which settles.
        key(app, "n", APP)
        dialog = app.client.call("navigation.get")["dialog"]
        assert dialog is not None and dialog["kind"].startswith("CreateNote"), dialog
        app.client.call("dialog.close")
        # The column keys leave the board and come back.
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        key(app, "[", APP, "shift")
        assert column(app) == "notes"
        wait_focus(app, [])
        assert board(app)["state"] == "unfocused"


def test_a_board_command_key_is_inert_in_a_markdown_note(samples: Path) -> None:
    """Alt+Up still moves a line in the editor; the board's bare keys and
    Ctrl chords never reach a Markdown note's board path."""
    with board_app(samples, "board-keys-markdown") as app:
        open_note(app, NOTES)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        text = app.client.call("editor.get")["text"]
        lines = text.split("\n")
        assert len(lines) >= 2
        app.client.call("editor.cursor", {"line": 1, "column": 0})
        app.client.call("shortcut", {"id": "edit.move_line_up"})
        moved = app.client.call("editor.get")["text"].split("\n")
        assert moved[0] == lines[1] and moved[1] == lines[0], (lines, moved)
        with pytest.raises(Exception, match="board"):
            key(app, "r")
        with pytest.raises(Exception, match="board"):
            type_text(app, "x")
