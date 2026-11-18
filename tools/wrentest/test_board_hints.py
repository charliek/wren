"""The keys a board shows (discovery/board-controls.md §3.4, §3.5): the hint
line in the footer, which `board.get` reports as `hint`, and the keys sheet,
reported as `keys_sheet` while it is open.

Keys are the live keymap's labels, so these cases expect Linux's or
macOS's spelling of Mod as the platform has it.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, FUTURE, board, board_app, click, copy_samples, fitted, key, open_note,
)
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

PACK = "3qrg4kv1"
MOD = "⌘" if sys.platform == "darwin" else "Ctrl+"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def line(state: dict) -> str:
    """The hint as text: each part's keys, then its words."""
    return " · ".join(" ".join([*part["keys"], part["words"]]) for part in state["hint"])


def focused_board(app: WrenApp, path: str = FLOW) -> None:
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    fitted(app)


def sheet(app: WrenApp) -> dict | None:
    return board(app)["keys_sheet"]


def test_the_hint_follows_the_selection_the_tool_and_the_label(samples: Path) -> None:
    with board_app(samples, "board-hints") as app:
        focused_board(app)
        assert line(board(app)) == "Tab add a shape · R O D U Y P pick a shape · ? all keys"

        click(app, {"node": PACK})
        assert line(board(app)) == (f"Tab next shape · {MOD}Arrow connect · ↵ label · "
                                    "0–7 colour · " + ("⇧D" if MOD == "⌘" else "Shift+D")
                                    + " reshape · / actions")
        state = key(app, "d")
        assert state["tool"] == "diamond"
        assert line(state) == ("click to place · drag to size · Tab add it after the selection"
                               " · esc cancel")
        key(app, "escape")
        state = key(app, "enter")
        assert state["state"] == "editing"
        assert line(state) == f"{MOD}↵ done · Tab done and add the next shape · esc done"
        key(app, "escape")

        # Another column takes the keyboard: no hint.
        app.client.call("column.open", {"column": "notes"})
        wait_for(lambda: board(app)["hint"] == [], timeout=3, interval=0.025,
                 description="the hint gone with the keyboard")
        assert app.client.call("session.get")["document"]["dirty"] is False


def test_a_read_only_board_hints_only_what_it_takes(samples: Path) -> None:
    with board_app(samples, "board-hints-read-only") as app:
        focused_board(app, FUTURE)
        state = board(app)
        assert state["read_only"] is True
        assert line(state) == "? all keys"


def test_the_setting_off_leaves_the_footer_as_it_was(samples: Path) -> None:
    with WrenApp(workspace=samples, settings="boards = on\nboard-hints = off\n",
                 artifacts_dir=artifact_directory("board-hints-off")) as app:
        focused_board(app)
        click(app, {"node": PACK})
        assert board(app)["hint"] == []
        # The sheet is a command of its own, and still opens.
        app.client.call("command.run", {"id": "board.keys"})
        assert sheet(app) is not None


def test_the_keys_sheet_opens_with_question_mark_dims_and_closes(samples: Path) -> None:
    with board_app(samples, "board-keys-sheet") as app:
        focused_board(app)
        # `?` as a US layout types it (Shift with /), and as a layout that
        # types it with another key does, which the board asks as Shift+/.
        for typed in ("/", "?"):
            assert key(app, typed, "shift")["handled"] is True
            wait_for(lambda: sheet(app) is not None, timeout=3, interval=0.025,
                     description=f"the sheet open from {typed}")
            modal = app.client.call("navigation.get")["modal"]
            assert modal["mode"] == "board_keys"
            rows = {row["words"]: row for row in sheet(app)["rows"]}
            assert rows["Board keys"]["keys"] == ["?"]
            assert rows["Delete selection"]["dimmed"] is True
            assert rows["Select all"]["dimmed"] is False
            assert rows["Pan"]["keys"] == ["Scroll", "Space+drag", "Middle-drag"]
            groups = list(dict.fromkeys(row["group"] for row in sheet(app)["rows"]))
            assert groups == ["Tools", "Build", "Select", "Edit", "Colour", "Arrange", "View",
                              "Menus"]
            # `?` in the filter closes it; the board has the keyboard again.
            app.client.call("palette.query", {"query": "?"})
            assert sheet(app) is None
            wait_focus(app, ["note-editor"])

        # With a shape selected, what it takes is lit; a filter narrows.
        click(app, {"node": PACK})
        app.client.call("command.run", {"id": "board.keys"})
        rows = {row["words"]: row for row in sheet(app)["rows"]}
        assert rows["Delete selection"]["dimmed"] is False
        assert rows["Colour: red"]["dimmed"] is False
        app.client.call("palette.query", {"query": "colour"})
        assert {row["group"] for row in sheet(app)["rows"]} == {"Colour"}
        app.client.call("key.nav", {"key": "escape"})
        assert sheet(app) is None
        wait_focus(app, ["note-editor"])
        assert board(app)["selection"] == [PACK]

        # Enter closes it too; in a label `?` is text.
        app.client.call("command.run", {"id": "board.keys"})
        app.client.call("key.nav", {"key": "enter"})
        assert sheet(app) is None
        key(app, "enter")
        state = key(app, "?", "shift")
        assert state["state"] == "editing" and state["editing"]["text"] == "?"
        assert sheet(app) is None
        key(app, "escape")


def test_the_sheet_opens_on_a_read_only_board(samples: Path) -> None:
    with board_app(samples, "board-keys-read-only") as app:
        focused_board(app, FUTURE)
        app.client.call("command.run", {"id": "board.keys"})
        rows = {row["words"]: row for row in sheet(app)["rows"]}
        assert rows["Rectangle tool"]["dimmed"] is True
        assert rows["Hand tool"]["dimmed"] is False
        assert rows["Board keys"]["dimmed"] is False


def test_with_boards_off_there_is_no_sheet_command(samples: Path) -> None:
    with board_app(samples, "board-keys-off", setting="off") as app:
        app.client.call("palette.open", {"mode": "commands"})
        results = app.client.call("navigation.get")["modal"]["results"]
        assert results
        assert not [row for row in results if row["key"].get("command", "").startswith("board.")]
