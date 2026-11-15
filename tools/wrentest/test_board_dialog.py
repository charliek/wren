"""The New note dialog's kind (discovery/boards.md §5.4, §6.8 Phase 2
item 9).

Tab and Shift+Tab, which no widget in the dialog takes, step the kind; the
harness sends them as the keyboard subscription does (`dialog.tab`).
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest.boards import GROUP, NOTES, board, board_app, copy_samples, open_note
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.ui import WrenApp


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def dialog(app: WrenApp) -> dict | None:
    return app.client.call("navigation.get")["dialog"]


def kind(app: WrenApp) -> str:
    """The kind the dialog would create: `Markdown` or `Board`."""
    shown = dialog(app)
    assert shown is not None and shown["kind"].startswith("CreateNote("), shown
    return shown["kind"].rsplit(", ", 1)[1].rstrip(")")


def create(app: WrenApp, name: str, tabs: list[bool]) -> None:
    app.client.call("command.run", {"id": "note.new"})
    app.client.call("dialog.name", {"name": name})
    for reverse in tabs:
        app.client.call("dialog.tab", {"reverse": reverse})
    app.client.call("dialog.submit")


def test_enter_makes_markdown_and_tab_enter_a_board(samples: Path) -> None:
    group = samples / GROUP
    with board_app(samples, "board-dialog-kinds") as app:
        open_note(app, NOTES)
        app.client.call("command.run", {"id": "note.new"})
        assert kind(app) == "Markdown"
        app.client.call("dialog.tab", {"reverse": False})
        assert kind(app) == "Board"
        # Shift+Tab goes back, and both wrap.
        app.client.call("dialog.tab", {"reverse": True})
        assert kind(app) == "Markdown"
        app.client.call("dialog.tab", {"reverse": True})
        assert kind(app) == "Board"
        app.client.call("dialog.close")

        create(app, "Plain", [])
        assert (group / "Plain.md").is_file()
        create(app, "Flowchart", [False])
        assert (group / "Flowchart.wren-board").read_text(encoding="utf-8") == "schema = 1\n"
        # The new board is open, empty, focused, at 100 % and Idle.
        state = board(app)
        assert state["surface"] == "board" and state["counts"] == {"nodes": 0, "edges": 0}
        wait_focus(app, ["note-editor"])
        state = board(app)
        assert state["state"] == "idle" and state["viewport"]["zoom"] == 1.0, state
        # The kind is never remembered; a taken name is numbered.
        create(app, "Flowchart", [False])
        assert (group / "Flowchart 2.wren-board").is_file()
        create(app, "Again", [])
        assert (group / "Again.md").is_file()


def test_new_board_preselects_board_and_rename_has_no_kind(samples: Path) -> None:
    with board_app(samples, "board-dialog-new-board") as app:
        open_note(app, NOTES)
        app.client.call("command.run", {"id": "note.new_board"})
        assert kind(app) == "Board"
        app.client.call("dialog.close")
        app.client.call("command.run", {"id": "note.rename"})
        shown = dialog(app)
        assert shown is not None and not shown["kind"].startswith("CreateNote"), shown
        before = shown["kind"]
        app.client.call("dialog.tab", {"reverse": False})
        assert dialog(app)["kind"] == before
        app.client.call("dialog.close")


def test_with_the_setting_off_tab_changes_nothing(samples: Path) -> None:
    group = samples / GROUP
    with board_app(samples, "board-dialog-off", setting="off") as app:
        open_note(app, NOTES)
        create(app, "Still markdown", [False, False, True])
        assert (group / "Still markdown.md").is_file()
        assert not list(group.glob("Still markdown*.wren-board"))
        rows = app.client.call("navigation.get")
        assert dialog(app) is None and rows is not None
