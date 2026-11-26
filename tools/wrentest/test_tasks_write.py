"""Writing a task from outside the editor (discovery/tasks-build.md item
9.1; discovery/tasks.md §10): one writer per note.

``tasks.view`` changes a task with ``write``, hides a note's tasks with
``hide`` and takes the last write back with ``undo``, each through the
function the view's own keys and menus call. The open note is changed
through its editor, so it keeps its undo history, its caret and its draft;
a note that is not open is written by the Session, after the open note's
draft is saved. A row that is out of date is refused, and nothing is
written.

Every case here writes its own ``tasks`` line and the advanced engine,
whatever the lane's ``WRENTEST_TASKS`` and ``WRENTEST_ENGINE`` say.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_navigation import command, core, item, navigation, session
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

# The design page's day, a Thursday; every task test pins it.
TODAY = "2026-11-19"
ON = "editor-engine = advanced\ntasks = on\nboards = on\n"
OFF = "editor-engine = advanced\ntasks = off\nboards = on\n"
GROUP = "Work/Tasks"
LIST = f"{GROUP}/List.md"
OTHER = f"{GROUP}/Other.md"
MAIL = "- [ ] Mail the deck 📅 2026-11-23"
MAILED = "- [x] Mail the deck 📅 2026-11-23 ✅ 2026-11-19"
LUNCH = "- [ ] Order lunch"
FILES = {
    LIST: f"# List\n\n{MAIL}\n- [ ] Book the room\n",
    OTHER: f"# Other\n\n{LUNCH}\n",
}
CHANGED = "This task changed; the list was refreshed"


@contextmanager
def write_app(tmp_path: Path, case: str, settings: str = ON) -> Iterator[WrenApp]:
    root = tmp_path / "workspace"
    for relative, text in FILES.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    with WrenApp(workspace=root, evaluation=False, settings=settings, today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-write-{case}")) as app:
        assert app.client.call("editor.get")["engine"] == "advanced"
        assert app.client.call("tasks.get")["tasks"] is ("tasks = on" in settings)
        core(app, "open_note", note=item(app, "notes", LIST)["id"])
        assert session(app)["document"]["path"] == LIST
        yield app


def show(app: WrenApp) -> None:
    answer = app.client.call("tasks.view", {"open": {"scope": "group", "path": GROUP}})
    assert answer["disposition"] == "completed", answer
    assert navigation(app)["surface"] == "tasks"


def write(app: WrenApp, path: str, line: int, change, **more) -> dict:
    row = {"path": path, "line": line, "change": change, **more}
    return app.client.call("tasks.view", {"write": row})


def undo(app: WrenApp) -> dict:
    return app.client.call("tasks.view", {"undo": True})


def on_disk(tmp_path: Path, relative: str) -> str:
    return (tmp_path / "workspace" / relative).read_bytes().decode("utf-8")


def files(tmp_path: Path) -> list[str]:
    return sorted(path.name for path in (tmp_path / "workspace" / GROUP).iterdir())


def test_the_open_note_is_changed_through_its_editor_and_keeps_its_history(
        tmp_path: Path) -> None:
    with write_app(tmp_path, "open") as app:
        # Typed in the note, with the caret left in the task's description.
        app.client.call("editor.cursor", {"line": 3, "column": 19, "selection": None})
        app.client.call("editor.insert", {"text": " at noon"})
        typed = FILES[LIST].replace("Book the room", "Book the room at noon")
        assert app.client.call("editor.get")["text"] == typed
        caret = {"line": 2, "column": 10, "selection": None}
        app.client.call("editor.cursor", caret)
        show(app)
        assert on_disk(tmp_path, LIST) == typed

        answer = write(app, LIST, 2, "toggle")
        assert answer["refused"] is None and answer["disposition"] == "completed"
        assert answer["write"] == {"path": LIST, "line": 2, "before": MAIL, "after": MAILED,
                                   "changed": True, "open": True, "warning": None}
        # The task index has the line as written: the row is found again.
        assert answer["note"]["tasks"][0] == {"line": 2, "source": MAILED}
        assert answer["can_undo"] is True
        toggled = typed.replace(MAIL, MAILED)
        editor = app.client.call("editor.get")
        assert editor["text"] == toggled
        assert editor["cursor"]["line"] == 2 and editor["cursor"]["column"] == 10
        assert editor["can_undo"] is True
        assert session(app)["document"]["text"] == toggled
        # The view still has the pane, and autosave saves the draft.
        assert navigation(app)["surface"] == "tasks"
        wait_for(lambda: on_disk(tmp_path, LIST) == toggled,
                 description="autosave saves the change made from the view")

        # A second change to the same row, then both taken back from the
        # view, newest first, through the editor.
        answer = write(app, LIST, 2, {"priority": "high"})
        assert answer["write"]["before"] == MAILED
        raised = answer["write"]["after"]
        assert "⏫" in raised
        assert undo(app)["write"] == {"path": LIST, "line": 2, "before": raised,
                                      "after": MAILED, "changed": True, "open": True,
                                      "warning": None}
        assert app.client.call("editor.get")["text"] == toggled

        # Back in the note: its text, caret and undo history. The view's
        # three changes are three steps, the typing one more.
        command(app, "navigation.back")
        assert navigation(app)["surface"] == "text"
        editor = app.client.call("editor.get")
        assert editor["text"] == toggled
        assert editor["cursor"]["line"] == 2 and editor["cursor"]["column"] == 10
        assert app.client.call("editor.undo")["text"] == typed.replace(MAIL, raised)
        assert app.client.call("editor.undo")["text"] == toggled
        assert app.client.call("editor.undo")["text"] == typed
        assert app.client.call("editor.undo")["text"] == FILES[LIST]
        assert app.client.call("editor.get")["can_undo"] is False
        wait_for(lambda: on_disk(tmp_path, LIST) == FILES[LIST],
                 description="autosave saves the note as undone")


def test_a_closed_note_is_written_after_the_open_notes_draft_and_undone(
        tmp_path: Path) -> None:
    with write_app(tmp_path, "closed") as app:
        show(app)
        # Autosave is held, so only the settle before the write can have
        # saved the open note's draft.
        app.client.call("test.timer.hold", {"held": True})
        write(app, LIST, 2, "toggle")
        toggled = FILES[LIST].replace(MAIL, MAILED)
        assert session(app)["document"]["dirty"] is True
        assert on_disk(tmp_path, LIST) == FILES[LIST]

        answer = write(app, OTHER, 2, "cycle")
        assert answer["write"] == {"path": OTHER, "line": 2, "before": LUNCH,
                                   "after": "- [/] Order lunch", "changed": True,
                                   "open": False, "warning": None}
        assert answer["note"] == {"path": OTHER, "hidden": False,
                                  "tasks": [{"line": 2, "source": "- [/] Order lunch"}]}
        assert on_disk(tmp_path, OTHER) == FILES[OTHER].replace("[ ]", "[/]")
        assert on_disk(tmp_path, LIST) == toggled
        assert session(app)["document"]["dirty"] is False
        assert session(app)["document"]["path"] == LIST
        assert app.client.call("editor.get")["text"] == toggled

        # A change that changes nothing writes nothing and leaves no undo.
        modified = (tmp_path / "workspace" / OTHER).stat().st_mtime_ns
        kept = write(app, OTHER, 2, {"due": None})
        assert kept["write"]["changed"] is False and kept["refused"] is None
        assert (tmp_path / "workspace" / OTHER).stat().st_mtime_ns == modified

        # Undo takes the closed note's change back, then the open note's.
        answer = undo(app)
        assert answer["write"]["path"] == OTHER and answer["write"]["after"] == LUNCH
        assert on_disk(tmp_path, OTHER) == FILES[OTHER]
        answer = undo(app)
        assert answer["write"]["path"] == LIST and answer["write"]["open"] is True
        assert app.client.call("editor.get")["text"] == FILES[LIST]
        assert answer["can_undo"] is False
        answer = undo(app)
        assert answer["refused"] == {"message": "Nothing to undo", "refresh": False}
        assert files(tmp_path) == ["List.md", "Other.md"]


def test_a_row_out_of_date_and_an_undo_after_an_outside_edit_are_refused(
        tmp_path: Path) -> None:
    with write_app(tmp_path, "refused") as app:
        show(app)
        stale = {"message": CHANGED, "refresh": True}
        # A row that holds a line the note no longer has, in the open
        # note and in a closed one.
        for path in (LIST, OTHER):
            answer = write(app, path, 2, "toggle", expect="- [ ] as it was")
            assert answer["refused"] == stale, answer
            assert answer["write"] is None and answer["disposition"] == "blocked"
            assert answer["can_undo"] is False
        editor = app.client.call("editor.get")
        assert editor["text"] == FILES[LIST] and editor["can_undo"] is False
        assert session(app)["document"]["dirty"] is False
        for path, text in FILES.items():
            assert on_disk(tmp_path, path) == text

        # Another program moves the task down: it is found at its new
        # place, by the line the row holds.
        moved = FILES[OTHER].replace(LUNCH, f"new\nlines\n{LUNCH}")
        (tmp_path / "workspace" / OTHER).write_text(moved, encoding="utf-8")
        answer = write(app, OTHER, 2, "toggle", expect=LUNCH)
        done = "- [x] Order lunch ✅ 2026-11-19"
        assert answer["write"]["line"] == 4 and answer["write"]["after"] == done
        assert on_disk(tmp_path, OTHER) == moved.replace(LUNCH, done)

        # It edits the written line before the undo: refused, with its
        # bytes left alone, no conflict copy, and the list refreshed.
        theirs = on_disk(tmp_path, OTHER).replace("Order lunch", "Order lunches")
        (tmp_path / "workspace" / OTHER).write_text(theirs, encoding="utf-8")
        answer = undo(app)
        assert answer["refused"] == stale and answer["can_undo"] is False
        assert on_disk(tmp_path, OTHER) == theirs
        assert files(tmp_path) == ["List.md", "Other.md"]
        fresh = write(app, OTHER, 4, {"due": None})
        assert fresh["refused"] is None
        assert fresh["note"]["tasks"] == [
            {"line": 4, "source": "- [x] Order lunches ✅ 2026-11-19"}]


def test_a_draft_that_cannot_be_saved_blocks_the_write_to_another_note(
        tmp_path: Path) -> None:
    with write_app(tmp_path, "blocked") as app:
        show(app)
        app.client.call("test.timer.hold", {"held": True})
        write(app, LIST, 2, "toggle")
        toggled = FILES[LIST].replace(MAIL, MAILED)
        app.client.call("test.fail_next", {"fault": "write"})
        answer = write(app, OTHER, 2, "toggle")
        assert answer["write"] is None and answer["disposition"] == "blocked"
        assert answer["refused"]["refresh"] is False
        assert "injected storage failure" in answer["refused"]["message"]
        assert on_disk(tmp_path, OTHER) == FILES[OTHER]
        assert on_disk(tmp_path, LIST) == FILES[LIST]
        assert session(app)["document"]["dirty"] is True
        assert session(app)["document"]["text"] == toggled
        assert navigation(app)["surface"] == "tasks"
        # The next try saves the draft, then writes.
        answer = write(app, OTHER, 2, "toggle")
        assert answer["write"]["changed"] is True
        assert on_disk(tmp_path, LIST) == toggled
        assert session(app)["document"]["dirty"] is False


def test_hiding_a_notes_tasks_writes_its_front_matter_and_undo_takes_it_out(
        tmp_path: Path) -> None:
    with write_app(tmp_path, "hide") as app:
        block = "---\ntasks: off\n---\n"
        app.client.call("editor.cursor", {"line": 2, "column": 10, "selection": None})
        show(app)
        # A closed note, by the Session.
        answer = app.client.call("tasks.view", {"hide": {"path": OTHER, "hidden": True}})
        assert answer["switch"] == {"path": OTHER, "before": "", "after": block,
                                    "changed": True, "open": False, "warning": None}
        assert answer["note"] == {"path": OTHER, "hidden": True, "tasks": []}
        assert on_disk(tmp_path, OTHER) == block + FILES[OTHER]
        # The open note, in its editor: the caret moves down with its text.
        answer = app.client.call("tasks.view", {"hide": {"path": LIST, "hidden": True}})
        assert answer["switch"]["open"] is True and answer["switch"]["after"] == block
        assert answer["note"] == {"path": LIST, "hidden": True, "tasks": []}
        editor = app.client.call("editor.get")
        assert editor["text"] == block + FILES[LIST]
        assert editor["cursor"]["line"] == 5 and editor["cursor"]["column"] == 10
        # Hidden already: nothing more to write.
        again = app.client.call("tasks.view", {"hide": {"path": LIST, "hidden": True}})
        assert again["switch"]["changed"] is False

        # Showing a note takes the line out, and the block with it.
        answer = app.client.call("tasks.view", {"hide": {"path": OTHER, "hidden": False}})
        assert answer["switch"]["after"] == "" and answer["note"]["hidden"] is False
        assert on_disk(tmp_path, OTHER) == FILES[OTHER]
        # Undo puts it back, then takes the open note's out, then the
        # first one's.
        assert undo(app)["switch"]["after"] == block
        assert on_disk(tmp_path, OTHER) == block + FILES[OTHER]
        answer = undo(app)
        assert answer["switch"]["open"] is True and answer["note"]["hidden"] is False
        editor = app.client.call("editor.get")
        assert editor["text"] == FILES[LIST]
        assert editor["cursor"]["line"] == 2 and editor["cursor"]["column"] == 10
        answer = undo(app)
        assert answer["note"] == {"path": OTHER, "hidden": False,
                                  "tasks": [{"line": 2, "source": LUNCH}]}
        assert on_disk(tmp_path, OTHER) == FILES[OTHER]
        assert answer["can_undo"] is False


def test_with_tasks_off_no_write_is_reachable(tmp_path: Path) -> None:
    with write_app(tmp_path, "off", OFF) as app:
        for request in ({"write": {"path": LIST, "line": 2, "change": "toggle"}},
                        {"write": {"path": OTHER, "line": 2, "change": "toggle",
                                   "expect": LUNCH}},
                        {"hide": {"path": OTHER, "hidden": True}},
                        {"undo": True}):
            with pytest.raises(RemoteError, match="Tasks are off"):
                app.client.call("tasks.view", request)
        assert app.client.call("tasks.view") == {"tasks": False}
        editor = app.client.call("editor.get")
        assert editor["text"] == FILES[LIST] and editor["can_undo"] is False
        for path, text in FILES.items():
            assert on_disk(tmp_path, path) == text
        assert not (tmp_path / "workspace" / ".wren" / "tasks.toml").exists()
