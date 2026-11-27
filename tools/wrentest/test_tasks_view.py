"""The task view's surface (discovery/tasks-build.md item 10.1;
discovery/tasks.md §9): a view in the editor pane that is not a document.

``tasks.view`` opens the view of a scope and reports what the pane shows.
The note under the view stays the Session's document, the editor under it
takes nothing, and Back and Forward pass through the view as through a
note, by the app's own commands and their keys. Nothing of a view is
written: the state file is what the note visits alone write, and a restart
reopens the last note.

Every case here writes its own ``tasks`` line and the advanced engine,
whatever the lane's ``WRENTEST_TASKS`` and ``WRENTEST_ENGINE`` say. Chords
are pressed with the harness's ``mod`` token, or natively on the lane's
command key.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import shutil

import pytest

from tools.wrentest import test_gestures as native
from tools.wrentest.client import RemoteError
from tools.wrentest.test_navigation import command, core, item, navigation, session
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, native_mod, wait_for

# The design page's day, a Thursday; every task test pins it.
TODAY = "2026-11-19"
ON = "editor-engine = advanced\ntasks = on\nboards = on\n"
OFF = "editor-engine = advanced\ntasks = off\nboards = on\n"
GROUP = "Work/Tasks"
LIST = f"{GROUP}/List.md"
NOTES = f"{GROUP}/Notes.md"
FLOW = f"{GROUP}/Flow.wren-board"
THIRD = "Home/Chores/Third.md"
SPARE = "Work/Spare"
FILES = {
    LIST: "# List\n\n- [ ] Mail the deck 📅 2026-11-23\n- [x] Book the room\n",
    NOTES: "# Notes\n\nplain words\n",
    FLOW: "schema = 1\n",
    THIRD: "third\n",
    f"{SPARE}/Extra.md": "- [ ] spare\n",
}
CLOSED = {"tasks": True, "open": False, "scope": None, "path": None, "label": None,
          "keys": False}


def write_workspace(root: Path) -> Path:
    for relative, text in FILES.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    return root


@contextmanager
def view_app(tmp_path: Path, case: str, settings: str = ON) -> Iterator[WrenApp]:
    root = write_workspace(tmp_path / "workspace")
    with WrenApp(workspace=root, evaluation=False, settings=settings, today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-view-{case}")) as app:
        assert app.client.call("editor.get")["engine"] == "advanced"
        assert app.client.call("tasks.get")["tasks"] is ("tasks = on" in settings)
        open_note(app, LIST)
        yield app


def open_note(app: WrenApp, path: str) -> int:
    note = item(app, "notes", path)["id"]
    core(app, "open_note", note=note)
    assert document(app) == path
    return note


def document(app: WrenApp) -> str | None:
    current = session(app)["document"]
    return current and current["path"]


def view(app: WrenApp) -> dict:
    return app.client.call("tasks.view")


def show(app: WrenApp, scope: str, path: str | None = None, *,
         disposition: str = "completed") -> dict:
    target = {"scope": scope} if path is None else {"scope": scope, "path": path}
    answer = app.client.call("tasks.view", {"open": target})
    assert answer["disposition"] == disposition, answer
    return answer


def press(app: WrenApp, key: str, *mods: str) -> dict:
    """A key pressed while the view has the keyboard."""
    return app.client.call("tasks.view", {"key": {"key": key, "mods": list(mods)}})


def shown(app: WrenApp, scope: str, path: str | None, label: str, under: str | None) -> None:
    """The view of *scope* is in the pane, over the note *under*."""
    assert view(app) == {"tasks": True, "open": True, "scope": scope, "path": path,
                         "label": label, "keys": True}
    assert navigation(app)["surface"] == "tasks"
    assert document(app) == under


def state_file(root: Path) -> bytes:
    return (root / ".wren" / "workspace.toml").read_bytes()


def test_the_view_opens_over_the_note_and_the_editor_under_it_takes_nothing(
        tmp_path: Path) -> None:
    with view_app(tmp_path, "opens") as app:
        root = tmp_path / "workspace"
        assert view(app) == CLOSED
        assert navigation(app)["surface"] == "text"
        app.client.call("editor.cursor", {"line": 2, "column": 6, "selection": None})
        before = app.client.call("editor.get")
        saved = state_file(root)

        answer = show(app, "group", GROUP)
        assert answer["warning"] is None
        shown(app, "group", GROUP, "Tasks", LIST)
        assert navigation(app)["column"] == "editor"
        # Neither the editor nor a board is mounted, so no widget has the
        # keyboard and the app's own key handling hears every press.
        wait_for(lambda: app.client.call("focus.get")["named"] == [],
                 description="no widget has the keyboard under the view")
        assert "note-editor" not in app.client.call("focus.get")["bounds"]
        assert state_file(root) == saved

        # Every way the harness writes through the editor is refused, and
        # the editor's and the task commands are unavailable.
        for request, params in [
            ("editor.insert", {"text": "bypass"}),
            ("editor.key", {"key": "x", "mods": []}),
            ("editor.command", {"intent": "bold"}),
            ("editor.undo", {}),
            ("editor.click", {"line": 2, "column": 3}),
            ("editor.focus", {}),
        ]:
            with pytest.raises(RemoteError, match="task view"):
                app.client.call(request, params)
        for id_ in ("markdown.toggle_task", "markdown.bold", "task.set_due", "task.edit",
                    "task.cycle_status", "navigation.find", "navigation.outline"):
            result = app.client.call("command.run", {"id": id_})
            assert result["disposition"] == "blocked", (id_, result)
        # Their keys do nothing: only a focused editor runs them.
        for id_ in ("markdown.bold", "task.set_due", "task.edit"):
            app.client.call("shortcut", {"id": id_})
        press(app, "d", "mod")
        press(app, "enter", "mod", "shift")
        press(app, "z", "mod")
        after = app.client.call("editor.get")
        for key in ("text", "cursor", "revision", "can_undo", "can_redo"):
            assert after[key] == before[key], key
        assert after["popover"] is None
        assert app.client.call("tasks.form") == {"open": False}
        assert session(app)["document"]["dirty"] is False
        assert (root / LIST).read_text(encoding="utf-8") == FILES[LIST]

        # Escape and the arrows leave the view in place (Enter opens the
        # selected task's note: test_tasks_view_list.py).
        for key in ("escape", "up", "down", "left", "right"):
            press(app, key)
            shown(app, "group", GROUP, "Tasks", LIST)
        assert navigation(app)["column"] == "editor"

        # Under a palette or a dialog the keyboard is theirs: the view has
        # no keys, and a key sent to it is refused and closes nothing.
        escape = {"key": {"key": "escape", "mods": []}}
        app.client.call("palette.open", {"mode": "commands"})
        assert view(app) == {"tasks": True, "open": True, "scope": "group", "path": GROUP,
                             "label": "Tasks", "keys": False}
        with pytest.raises(RemoteError, match="does not have the keyboard"):
            app.client.call("tasks.view", escape)
        assert navigation(app)["modal"]["mode"] == "commands"
        app.client.call("palette.close")
        command(app, "section.new")
        assert view(app)["keys"] is False
        with pytest.raises(RemoteError, match="does not have the keyboard"):
            app.client.call("tasks.view", escape)
        assert navigation(app)["dialog"] is not None
        app.client.call("dialog.close")
        shown(app, "group", GROUP, "Tasks", LIST)

        # A scope that is not there, or of another kind, is refused.
        for scope, path in [("group", "Work/Missing"), ("note", GROUP), ("section", GROUP)]:
            assert show(app, scope, path, disposition="blocked")["warning"]
            shown(app, "group", GROUP, "Tasks", LIST)
        for bad in [{"scope": "folder", "path": GROUP}, {"scope": "group"},
                    {"scope": "group", "path": "../Work"}]:
            with pytest.raises(RemoteError):
                app.client.call("tasks.view", {"open": bad})


def test_back_and_forward_pass_through_the_view_by_command_and_by_key(tmp_path: Path) -> None:
    with view_app(tmp_path, "history") as app:
        root = tmp_path / "workspace"
        # The notes visited so far: the one the launch opened, if it was
        # another, and the list.
        base = navigation(app)["history"]
        assert base[-1] == LIST and len(base) in (1, 2), base
        nav = navigation(app)
        assert (nav["can_back"], nav["can_forward"]) == (len(base) > 1, False)

        # Note, view, board: the notes' own history, as it is written, has
        # the notes and nothing of the view.
        show(app, "group", GROUP)
        nav = navigation(app)
        assert (nav["can_back"], nav["can_forward"]) == (True, False)
        open_note(app, FLOW)
        assert navigation(app)["surface"] == "board"
        assert navigation(app)["history"] == base + [FLOW]
        written = state_file(root)

        # Back by the command: the view, over the note it was opened from.
        command(app, "navigation.back")
        shown(app, "group", GROUP, "Tasks", LIST)
        # Back by its key, pressed in the view: the note.
        press(app, "[", "mod")
        assert view(app) == CLOSED
        assert navigation(app)["surface"] == "text"
        assert document(app) == LIST
        wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                 description="the note takes the keyboard back")
        assert navigation(app)["can_back"] is (len(base) > 1)
        assert navigation(app)["can_forward"] is True

        # Forward by the shortcut the editor passes on, then by the key in
        # the view, and no further.
        app.client.call("editor.key", {"key": "]", "mods": ["mod"]})
        shown(app, "group", GROUP, "Tasks", LIST)
        press(app, "]", "mod")
        assert navigation(app)["surface"] == "board"
        assert document(app) == FLOW
        assert navigation(app)["can_forward"] is False
        command(app, "navigation.forward", disposition="blocked")
        assert document(app) == FLOW
        # The steps moved the notes' position and wrote nothing else.
        assert navigation(app)["history"] == base + [FLOW]
        assert navigation(app)["history_position"] == len(base)
        assert state_file(root) == written

        # A view over the board puts the viewer away whole; Back mounts it
        # again as it was, with the keyboard.
        board = app.client.call("board.get")
        show(app, "note", LIST)
        shown(app, "note", LIST, "List", FLOW)
        app.client.call("shortcut", {"id": "board.select.all"})
        # The board under the view takes no input and no edit, and can
        # still be looked at.
        for request, params in [
            ("board.apply", {"edits": [{"op": "add_node", "id": "n1", "shape": "rect",
                                        "at": [0, 0], "size": [120, 60], "text": "One"}]}),
            ("board.pointer", {"kind": "press", "at": {"x": 10, "y": 10}, "button": "left"}),
            ("board.key", {"key": "delete", "mods": []}),
            ("board.type", {"text": "typed"}),
            ("board.export", {"format": "svg", "path": str(tmp_path / "covered.svg")}),
        ]:
            with pytest.raises(RemoteError, match="not shown as a board"):
                app.client.call(request, params)
        assert not (tmp_path / "covered.svg").exists()
        assert app.client.call("board.get")["surface"] == "board"
        wait_for(lambda: app.client.call("focus.get")["named"] == [],
                 description="no widget has the keyboard under the view")
        assert app.client.call("command.run", {"id": "board.select.all"})["disposition"] \
            == "blocked"
        command(app, "navigation.back")
        assert navigation(app)["surface"] == "board" and view(app) == CLOSED
        wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                 description="the board takes the keyboard back")
        after = app.client.call("board.get")
        for key in ("surface", "note", "viewport", "state"):
            assert after[key] == board[key], key
        assert session(app)["document"]["dirty"] is False
        assert state_file(root) == written

        # A second view from the first, a wider scope: each is a stop.
        command(app, "navigation.back")
        show(app, "section", "Work")
        shown(app, "section", "Work", "Work", LIST)
        show(app, "workspace")
        shown(app, "workspace", None, "workspace", LIST)
        command(app, "navigation.back")
        shown(app, "section", "Work", "Work", LIST)
        command(app, "navigation.back")
        shown(app, "group", GROUP, "Tasks", LIST)

        # A note opened from the view takes the pane, the views ahead go,
        # and Back returns to the view with its scope.
        open_note(app, THIRD)
        assert view(app) == CLOSED
        assert navigation(app)["history"] == base + [THIRD]
        command(app, "navigation.back")
        shown(app, "group", GROUP, "Tasks", LIST)
        command(app, "navigation.forward")
        assert document(app) == THIRD and view(app) == CLOSED
        # The paths name the `Tasks` group; nothing names a view.
        for word in (b"tasks", b"view", b"scope"):
            assert word not in state_file(root)


def test_a_draft_that_cannot_be_saved_keeps_the_view_shut(tmp_path: Path) -> None:
    with view_app(tmp_path, "dirty") as app:
        root = tmp_path / "workspace"
        # Autosave must not write the draft before the fault is armed.
        app.client.call("test.timer.hold", {"held": True})
        app.client.call("editor.insert", {"text": "unsaved "})
        draft = app.client.call("editor.get")["text"]
        assert session(app)["document"]["dirty"] is True
        app.client.call("test.fail_next", {"fault": "write"})
        answer = show(app, "group", GROUP, disposition="blocked")
        assert "injected storage failure" in answer["warning"]
        assert answer["open"] is False
        assert navigation(app)["surface"] == "text"
        assert app.client.call("editor.get")["text"] == draft
        assert session(app)["document"]["dirty"] is True
        assert (root / LIST).read_text(encoding="utf-8") == FILES[LIST]

        # The next try saves the draft and opens the view.
        show(app, "group", GROUP)
        shown(app, "group", GROUP, "Tasks", LIST)
        assert session(app)["document"]["dirty"] is False
        assert (root / LIST).read_text(encoding="utf-8") == draft


def test_what_is_deleted_outside_wren_under_a_view_leaves_history_whole(tmp_path: Path) -> None:
    with view_app(tmp_path, "outside") as app:
        root = tmp_path / "workspace"

        # The scoped group goes while a palette is open over its view: the
        # view leaves the pane, and the search field keeps the keyboard.
        show(app, "group", SPARE)
        shown(app, "group", SPARE, "Spare", LIST)
        app.client.call("palette.open", {"mode": "search"})
        wait_for(lambda: app.client.call("focus.get")["named"] == ["palette-query"],
                 description="the search field has the keyboard")
        shutil.rmtree(root / SPARE)
        core(app, "reconcile")
        assert view(app) == CLOSED
        native.settle_focus()
        assert app.client.call("focus.get")["named"] == ["palette-query"]
        assert navigation(app)["modal"]["mode"] == "search"
        app.client.call("palette.close")
        assert navigation(app)["can_forward"] is False

        # The note under a view goes: the view stays, over no note, and Back
        # from it opens the note of the visit before, which is still in
        # history.
        open_note(app, NOTES)
        show(app, "workspace")
        (root / NOTES).unlink()
        core(app, "reconcile")
        shown(app, "workspace", None, "workspace", None)
        assert navigation(app)["history"][-1] == LIST
        command(app, "navigation.back")
        assert view(app) == CLOSED
        assert document(app) == LIST
        assert app.client.call("editor.get")["text"] == FILES[LIST]
        wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                 description="the note takes the keyboard")
        command(app, "navigation.forward")
        shown(app, "workspace", None, "workspace", LIST)


def test_a_restart_reopens_the_last_note_and_tasks_off_has_no_view(tmp_path: Path) -> None:
    with view_app(tmp_path, "restart") as app:
        root = tmp_path / "workspace"
        open_note(app, NOTES)
        history = navigation(app)["history"]
        assert history[-2:] == [LIST, NOTES], history
        show(app, "group", GROUP)
        show(app, "workspace")
        written = state_file(root)

        app.restart(hard=False)
        assert state_file(root) == written
        assert view(app) == CLOSED
        assert navigation(app)["surface"] == "text"
        assert document(app) == NOTES
        assert app.client.call("editor.get")["text"] == FILES[NOTES]
        nav = navigation(app)
        assert nav["history"] == history
        assert (nav["can_back"], nav["can_forward"]) == (True, False)

        # With a view shown again, tasks are turned off and Wren restarted.
        show(app, "note", LIST)
        shown(app, "note", LIST, "List", NOTES)
        app.settings_path.write_text(OFF, encoding="utf-8")
        app.restart(hard=False)
        assert app.client.call("tasks.get") == {"tasks": False}
        assert view(app) == {"tasks": False}
        assert navigation(app)["surface"] == "text"
        assert document(app) == NOTES
        assert state_file(root) == written
        for params in [{"open": {"scope": "workspace"}}, {"open": {"scope": "group", "path": GROUP}},
                       {"key": {"key": "escape", "mods": []}}]:
            with pytest.raises(RemoteError, match="Tasks are off"):
                app.client.call("tasks.view", params)
        assert view(app) == {"tasks": False}
        # Back and Forward are the notes' alone.
        command(app, "navigation.back")
        assert document(app) == LIST
        command(app, "navigation.forward")
        assert document(app) == NOTES
        command(app, "navigation.forward", disposition="blocked")


def test_with_tasks_off_from_the_start_the_view_is_refused(tmp_path: Path) -> None:
    with view_app(tmp_path, "off", settings=OFF) as app:
        root = tmp_path / "workspace"
        written = state_file(root)
        assert view(app) == {"tasks": False}
        with pytest.raises(RemoteError, match="Tasks are off"):
            app.client.call("tasks.view", {"open": {"scope": "group", "path": GROUP}})
        assert navigation(app)["surface"] == "text"
        assert document(app) == LIST
        assert state_file(root) == written


@pytest.mark.native_x11
def test_native_keys_in_the_view_reach_the_app_and_never_the_note(tmp_path: Path) -> None:
    with view_app(tmp_path, "native") as app:
        root = tmp_path / "workspace"
        window = native.focus_native(app)
        open_note(app, NOTES)
        open_note(app, LIST)
        app.client.call("editor.focus")
        wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard")
        before = app.client.call("editor.get")
        show(app, "group", GROUP)
        wait_for(lambda: app.client.call("focus.get")["named"] == [],
                 description="no widget has the keyboard under the view")

        # Typing, editing keys, the editor's shortcuts and Escape: none
        # reaches the note, and the view stays. The letters are none of the
        # list's own keys: Return would open the selected task's note
        # (test_tasks_view_list.py), and Space, Mod+Return and the single
        # keys S, P, D, E, +, - and R act on the selected row
        # (test_tasks_view_write.py).
        native.type_text(window, "zoxy")
        native.keys(window, "BackSpace", "Delete", "Escape",
                    native_mod("b"), native_mod("d"),
                    native_mod("shift+Return"), native_mod("z"), native_mod("v"))
        native.settle_focus()
        shown(app, "group", GROUP, "Tasks", LIST)
        after = app.client.call("editor.get")
        for key in ("text", "cursor", "revision", "can_undo", "can_redo"):
            assert after[key] == before[key], key
        assert app.client.call("tasks.form") == {"open": False}
        assert app.client.call("tasks.calendar") == {"open": False}
        assert session(app)["document"]["dirty"] is False
        assert (root / LIST).read_text(encoding="utf-8") == FILES[LIST]

        # The app's own keys work: Back to the note, Forward to the view,
        # Back twice to the note before.
        native.keys(window, native_mod("bracketleft"))
        wait_for(lambda: view(app) == CLOSED, description="Mod+[ leaves the view")
        assert document(app) == LIST
        wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                 description="the note takes the keyboard back")
        native.keys(window, native_mod("bracketright"))
        wait_for(lambda: view(app)["open"], description="Mod+] returns to the view")
        native.keys(window, native_mod("bracketleft"))
        wait_for(lambda: view(app) == CLOSED, description="Mod+[ leaves the view")
        native.keys(window, native_mod("bracketleft"))
        wait_for(lambda: document(app) == NOTES, description="Mod+[ steps to the note before")
        # Typing reaches the note again.
        native.type_text(window, "x")
        wait_for(lambda: app.client.call("editor.get")["text"].startswith("x"),
                 description="typing reaches the editor after the view")
