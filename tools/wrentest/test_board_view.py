"""The board viewer in the app, with the `boards` setting on
(discovery/boards.md §3.3, §5.15, §6.8 Phase 1).

A board is shown by the board widget, mounted under the editor's own focus
id `note-editor`, so the editor column, its focus and the column moves work
on a board as they do on a Markdown note.
"""

from __future__ import annotations

import shutil
import xml.etree.ElementTree as ElementTree
from pathlib import Path

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

ROOT = Path(__file__).resolve().parents[2]
SAMPLES = ROOT / "tools" / "wrentest" / "fixtures" / "boards" / "workspace"
FLOW = "Boards/Samples/Checkout flow.wren-board"
NOTES = "Boards/Samples/Notes.md"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    """A copy of the sample workspace: opening one writes into it."""
    work = tmp_path / "work"
    shutil.copytree(SAMPLES, work)
    return work


def board_app(workspace: Path, case: str) -> WrenApp:
    return WrenApp(workspace=workspace, settings="boards = on\n",
                   artifacts_dir=artifact_directory(case))


def open_note(app: WrenApp, path: str) -> int:
    note = next(row for row in app.client.call("session.get")["notes"] if row["path"] == path)
    app.client.call("core.apply", {"op": "open_note", "note": note["id"]})
    return note["id"]


def column(app: WrenApp) -> str:
    return app.client.call("navigation.get")["column"]


def nav(app: WrenApp, key: str) -> dict:
    return app.client.call("key.nav", {"key": key})


def command(app: WrenApp, command_id: str) -> dict:
    return app.client.call("command.run", {"id": command_id})


def focused(app: WrenApp) -> list[str]:
    return app.client.call("focus.get")["named"]


def wait_focus(app: WrenApp, names: list[str]) -> None:
    wait_for(lambda: focused(app) == names, timeout=3, interval=0.025,
             description=f"focused widgets {names}")


def assert_board_shown(app: WrenApp) -> None:
    """The editor is not the surface: it refuses an edit naming the board."""
    with pytest.raises(RemoteError, match="board"):
        app.client.call("editor.insert", {"text": "bypass"})


def test_a_board_is_the_editor_column_under_note_editor(samples: Path) -> None:
    flow = samples / FLOW
    before = (flow.read_bytes(), flow.stat().st_mtime_ns)
    with board_app(samples, "board-focus") as app:
        board = open_note(app, FLOW)
        assert_board_shown(app)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert column(app) == "editor"
        canvas = app.client.call("focus.get")["bounds"]["note-editor"]
        assert canvas["width"] > 200 and canvas["height"] > 200, canvas

        # The column moves leave the board and give it the keyboard back.
        assert command(app, "navigation.column_left")["navigation"]["column"] == "notes"
        wait_focus(app, [])
        assert command(app, "navigation.column_right")["navigation"]["column"] == "editor"
        wait_focus(app, ["note-editor"])

        # An overlay takes the keyboard and Escape returns it to the board.
        app.client.call("palette.open", {"mode": "commands"})
        wait_focus(app, ["palette-query"])
        state = nav(app, "escape")
        assert state["modal"] is None and state["column"] == "editor"
        wait_focus(app, ["note-editor"])
        # With nothing open, Escape keeps the board focused.
        assert nav(app, "escape")["column"] == "editor"
        wait_focus(app, ["note-editor"])

        # Into the board from the notes list, by keyboard.
        app.client.call("column.open", {"column": "notes"})
        wait_focus(app, [])
        state = nav(app, "enter")
        assert state["column"] == "editor" and state["selected_note"] == board
        wait_focus(app, ["note-editor"])

        # A Markdown note mounts the editor under the same id, and a board
        # mounts the canvas again.
        open_note(app, NOTES)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert app.client.call("editor.get")["text"].startswith("#")
        open_note(app, FLOW)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert_board_shown(app)
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert (flow.read_bytes(), flow.stat().st_mtime_ns) == before


def test_a_raw_board_is_edited_in_the_focused_editor(samples: Path) -> None:
    raw = samples / "Boards/Samples/Conflict.wren-board"
    raw.write_text("<<<<<<< ours\nschema = 1\n=======\nschema = 1\n>>>>>>> theirs\n",
                   encoding="utf-8")
    with board_app(samples, "board-raw-focus") as app:
        open_note(app, "Boards/Samples/Conflict.wren-board")
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert app.client.call("editor.get")["text"].startswith("<<<<<<< ours")
        # The raw text is the editor's: it takes an edit.
        app.client.call("editor.insert", {"text": "# typed\n"})
        assert "# typed" in app.client.call("editor.get")["text"]
        wait_focus(app, ["note-editor"])


ESCAPES = '''schema = 1

[node.odd]
text = "a < b & \\"c\\""
kind = "shape"
at = [0, 0]
size = [160, 60]
z = "a"
'''


def export_rows(app: WrenApp) -> list[dict]:
    """The export command's rows in the Commands palette."""
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": "Export board"})["navigation"]["modal"]
    app.client.call("palette.close")
    return [row for row in modal["results"] if row["title"] == "Export board as SVG…"]


def test_export_svg_writes_the_shown_board_as_xml(samples: Path, tmp_path: Path) -> None:
    """§6.8 Phase 1 item 8, through what the command writes once its save
    dialog has closed."""
    (samples / "Boards/Samples/Escapes.wren-board").write_text(ESCAPES, encoding="utf-8")
    flow = samples / FLOW
    before = (flow.read_bytes(), flow.stat().st_mtime_ns)
    text = "{http://www.w3.org/2000/svg}text"
    with board_app(samples, "board-export") as app:
        open_note(app, NOTES)
        assert [row["enabled"] for row in export_rows(app)] == [False]
        assert command(app, "note.export_svg")["disposition"] == "blocked"
        with pytest.raises(RemoteError, match="board"):
            app.client.call("board.export", {"format": "svg", "path": str(tmp_path / "x.svg")})

        open_note(app, FLOW)
        assert [row["enabled"] for row in export_rows(app)] == [True]
        out = tmp_path / "flow.svg"
        reply = app.client.call("board.export", {"format": "svg", "path": str(out)})
        assert reply["bytes"] == out.stat().st_size
        root = ElementTree.parse(out).getroot()
        labels = [node.text or "" for node in root.iter(text)]
        # Fifteen shapes, some of them two lines, and the connector labels.
        assert len(labels) > 15 and "Receive order" in labels, labels
        assert root[0].tag == "{http://www.w3.org/2000/svg}rect", "the canvas behind it all"

        open_note(app, "Boards/Samples/Escapes.wren-board")
        out = tmp_path / "escapes.svg"
        app.client.call("board.export", {"format": "svg", "path": str(out)})
        assert [node.text for node in ElementTree.parse(out).getroot().iter(text)] == [
            'a < b & "c"'
        ]
        assert '>a &lt; b &amp; &quot;c&quot;</text>' in out.read_text(encoding="utf-8")

        with pytest.raises(RemoteError, match="Could not export"):
            app.client.call("board.export", {
                "format": "svg", "path": str(tmp_path / "missing" / "x.svg"),
            })
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert (flow.read_bytes(), flow.stat().st_mtime_ns) == before


def test_with_the_setting_off_there_is_no_export_command(samples: Path) -> None:
    with WrenApp(workspace=samples, settings="boards = off\n",
                 artifacts_dir=artifact_directory("board-export-off")) as app:
        open_note(app, NOTES)
        assert export_rows(app) == []
        assert command(app, "note.export_svg")["disposition"] == "blocked"
