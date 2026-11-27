"""The ways into the task view, and out of it (discovery/tasks-build.md item
10.2; discovery/tasks.md §2 items 21, 24 and 28, §7, §9).

``tasks.entry`` reports each column's header count (its scope, the open and
doing tasks, the overdue ones, and whether it is pressed), the rows that
carry a mark, the scope ``tasks.show`` would open, and the message with
Undo; it clicks a count, the message's Undo, or the message. The rest goes
through the paths a user takes: ``tasks.show`` by its ``mod`` chord in the
editor and on a board, by the shortcut path from each column, and from the
palette; ``view.toggle_task_counts`` by its ``mod`` chord in the editor and
on a board; a row's actions from the item-actions palette a right-click
opens; ``view.toggle_task_counts`` from the palette.

Every case writes its own ``tasks`` line and the advanced engine, whatever
the lane's ``WRENTEST_TASKS`` and ``WRENTEST_ENGINE`` say, and pins the
design page's day.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest

from tools.wrentest import test_gestures as native
from tools.wrentest.client import RemoteError
from tools.wrentest.test_navigation import activate, core, gateway, item, navigation, palette, \
    session
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
EXTRA = "Work/Spare/Extra.md"
THIRD = "Home/Chores/Third.md"
FILES = {
    # Two open or doing, one overdue on the pinned day.
    LIST: "# List\n\n- [ ] Mail the deck 📅 2026-11-16\n- [/] Book the room\n- [x] Booked\n",
    # Due tomorrow.
    NOTES: "# Notes\n\n- [ ] Order lunch 📅 2026-11-20\n",
    FLOW: "schema = 1\n",
    EXTRA: "- [ ] spare\n",
    THIRD: "- [ ] third\n",
}
HIDDEN_NOTE = "Tasks in Notes hidden · wrote tasks: off to its front matter"
HIDDEN_GROUP = "Work/Tasks hidden from task summaries · saved in .wren/tasks.toml"


def write_workspace(root: Path) -> Path:
    for relative, text in FILES.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    return root


@contextmanager
def entry_app(tmp_path: Path, case: str, settings: str = ON) -> Iterator[WrenApp]:
    root = write_workspace(tmp_path / "workspace")
    with WrenApp(workspace=root, evaluation=False, settings=settings, today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-entry-{case}")) as app:
        assert app.client.call("editor.get")["engine"] == "advanced"
        assert app.client.call("tasks.get")["tasks"] is ("tasks = on" in settings)
        open_note(app, LIST)
        yield app


def open_note(app: WrenApp, path: str) -> int:
    note = item(app, "notes", path)["id"]
    core(app, "open_note", note=note)
    assert session(app)["document"]["path"] == path
    return note


def entry(app: WrenApp, **action) -> dict:
    return app.client.call("tasks.entry", action)


def view(app: WrenApp) -> dict:
    return app.client.call("tasks.view")


def shown_scope(app: WrenApp) -> tuple[str | None, str | None]:
    state = view(app)
    return state["scope"], state["path"]


def count(scope: str, path: str | None, open_and_doing: int, overdue: int, label: str,
          pressed: bool = False) -> dict:
    return {"scope": scope, "path": path, "open_and_doing": open_and_doing,
            "overdue": overdue, "pressed": pressed, "label": label}


def leave_views(app: WrenApp) -> None:
    while view(app)["open"]:
        app.client.call("command.run", {"id": "navigation.back"})


def item_actions(app: WrenApp, collection: str, path: str) -> list[dict]:
    """The item-actions palette a right-click on the row opens: its rows."""
    target = {collection[:-1]: item(app, collection, path)["id"]}
    gateway(app, "palette.open", mode="item_actions", target=target)
    return navigation(app)["modal"]["results"]


def run_item_action(app: WrenApp, collection: str, path: str, command: str,
                    disposition: str = "completed") -> dict:
    rows = item_actions(app, collection, path)
    index = next(index for index, row in enumerate(rows)
                 if row["key"]["item_action"]["command"] == command)
    return gateway(app, "palette.activate", index=index, disposition=disposition)


def titles(rows: list[dict]) -> list[str]:
    return [row["title"] for row in rows]


def test_header_counts_count_their_scope_and_open_its_view(tmp_path: Path) -> None:
    with entry_app(tmp_path, "counts") as app:
        state = entry(app)
        assert state["counts"] is True
        assert state["headers"] == {
            "sections": count("workspace", None, 5, 1, "workspace"),
            "groups": count("section", "Work", 4, 1, "Work"),
            "notes": count("group", GROUP, 3, 1, "Tasks"),
        }
        assert state["notice"] is None
        # A click on each count opens its scope's view, and that count is
        # then pressed.
        for column, scope in [("sections", ("workspace", None)), ("groups", ("section", "Work")),
                              ("notes", ("group", GROUP))]:
            state = entry(app, click=column)
            assert state["disposition"] == "completed", state
            assert shown_scope(app) == scope
            assert view(app)["keys"] is True
            pressed = {name: header["pressed"] for name, header in state["headers"].items()}
            assert pressed == {name: name == column for name in pressed}, pressed
        # The day moves: lunch, due on the 20th, is overdue on the 21st.
        app.client.call("tasks.set_today", {"date": "2026-11-21"})
        assert entry(app)["headers"]["notes"]["overdue"] == 2
        app.client.call("tasks.set_today", {"date": "2026-11-15"})
        assert entry(app)["headers"]["notes"]["overdue"] == 0
        app.client.call("tasks.set_today", {"date": TODAY})
        # Another section's columns count that section and its group.
        leave_views(app)
        open_note(app, THIRD)
        headers = entry(app)["headers"]
        assert headers["groups"] == count("section", "Home", 1, 0, "Home")
        assert headers["notes"] == count("group", "Home/Chores", 1, 0, "Chores")


def test_counts_hide_and_show_from_the_palette_with_undo(tmp_path: Path) -> None:
    with entry_app(tmp_path, "toggle") as app:
        rows = palette(app, "commands", "task counts")["results"]
        row = next(row for row in rows if row["key"] == {"command": "view.toggle_task_counts"})
        assert (row["title"], row["group"], row["state"]) == ("Toggle task counts", "View", "on")
        activate(app, {"command": "view.toggle_task_counts"})
        assert navigation(app)["modal"] is None
        state = entry(app)
        assert state["counts"] is False
        assert state["headers"] == {"sections": None, "groups": None, "notes": None}
        assert state["notice"]["text"] == (
            "Task counts hidden · saved task-counts = off to config.conf")
        assert state["notice"]["code"] == ["task-counts = off"] and state["notice"]["undo"]
        assert app.settings_path.read_text(encoding="utf-8").endswith("task-counts = off\n")
        # Undo shows them again and saves that, and leaves no message.
        state = entry(app, undo=True)
        assert state["disposition"] == "completed", state
        assert state["counts"] is True and state["headers"]["notes"]["open_and_doing"] == 3
        assert state["notice"] is None
        assert app.settings_path.read_text(encoding="utf-8").endswith("task-counts = on\n")
        # The command again, then the message dismissed: the counts stay off.
        gateway(app, "command.run", id="view.toggle_task_counts")
        assert entry(app, dismiss=True)["notice"] is None
        assert entry(app)["counts"] is False
        with pytest.raises(RemoteError):
            entry(app, click="notes")


def test_counts_hide_and_show_by_their_chord_in_the_editor_and_on_a_board(
        tmp_path: Path) -> None:
    """Mod+Shift+H (discovery/tasks-build.md §10, 2026-11-27) runs Toggle
    task counts: from the editor, which it types nothing into, and from a
    board, which it leaves as it was. The palette shows the key."""
    with entry_app(tmp_path, "chord") as app:
        call = app.client.call
        rows = palette(app, "commands", "task counts")["results"]
        row = next(row for row in rows if row["key"] == {"command": "view.toggle_task_counts"})
        assert row["shortcut"].endswith("+Shift+H") or row["shortcut"] == "⇧⌘H", row
        gateway(app, "palette.close")
        call("editor.focus")
        text = call("editor.get")["text"]
        call("editor.key", {"key": "h", "mods": ["mod", "shift"]})
        state = entry(app)
        assert state["counts"] is False
        assert state["headers"] == {"sections": None, "groups": None, "notes": None}
        assert state["notice"]["text"] == (
            "Task counts hidden · saved task-counts = off to config.conf")
        assert call("editor.get")["text"] == text
        assert app.settings_path.read_text(encoding="utf-8").endswith("task-counts = off\n")
        # On a board the chord shows them again, and the board is untouched.
        open_note(app, FLOW)
        call("editor.focus")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the board has the keyboard")
        def board_state() -> tuple:
            board = call("board.get")
            return tuple(board[key] for key in ("revision", "pristine", "selection", "tool",
                                                "can_undo"))
        before = board_state()
        call("board.key", {"key": "h", "mods": ["mod", "shift"]})
        assert entry(app)["counts"] is True
        assert board_state() == before
        assert app.settings_path.read_text(encoding="utf-8").endswith("task-counts = on\n")
        assert (tmp_path / "workspace" / FLOW).read_text(encoding="utf-8") == FILES[FLOW]


def test_show_tasks_opens_where_the_keyboard_is(tmp_path: Path) -> None:
    with entry_app(tmp_path, "show") as app:
        call = app.client.call
        # From the editor, by its chord: the note's group. The note is
        # untouched.
        call("editor.focus")
        text = call("editor.get")["text"]
        assert entry(app)["scope"] == {"scope": "group", "path": GROUP}
        call("editor.key", {"key": "t", "mods": ["mod", "shift"]})
        assert shown_scope(app) == ("group", GROUP)
        assert call("editor.get")["text"] == text
        # From the view itself: the view, which is no new stop.
        back = navigation(app)["can_back"]
        call("shortcut", {"id": "tasks.show"})
        assert shown_scope(app) == ("group", GROUP) and navigation(app)["can_back"] == back
        leave_views(app)
        # From each column, as its shortcut runs: the selected section,
        # the selected group, the group listed.
        for column, scope in [("sections", ("section", "Work")), ("groups", ("group", GROUP)),
                              ("notes", ("group", GROUP))]:
            gateway(app, "column.open", column=column)
            assert entry(app)["scope"] == {"scope": scope[0], "path": scope[1]}, column
            call("shortcut", {"id": "tasks.show"})
            assert shown_scope(app) == scope, column
            assert navigation(app)["column"] == "editor"
            leave_views(app)
        # From the palette, with its key shown: the editor's scope.
        rows = palette(app, "commands", "Show tasks")["results"]
        row = next(row for row in rows if row["key"] == {"command": "tasks.show"})
        assert (row["title"], row["group"], row["enabled"]) == ("Show tasks", "Tasks", True)
        assert row["shortcut"].endswith("+Shift+T") or row["shortcut"] == "⇧⌘T", row
        activate(app, {"command": "tasks.show"})
        assert shown_scope(app) == ("group", GROUP)
        leave_views(app)
        # From a board, by its chord: the board's group.
        open_note(app, FLOW)
        call("editor.focus")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the board has the keyboard")
        call("board.key", {"key": "t", "mods": ["mod", "shift"]})
        assert shown_scope(app) == ("group", GROUP)
        # The Go menu holds it, with its key.
        menus = {menu["title"]: menu["items"] for menu in call("menu.get")["menus"]}
        go = [entry["key"] for entry in menus["Go"] if "key" in entry]
        assert go[go.index("navigation.outline") + 1] == "tasks.show"
        assert [entry["key"] for entry in menus["View"] if "key" in entry][-1] == \
            "view.toggle_task_counts"


def test_a_groups_actions_show_and_hide_its_tasks_with_undo(tmp_path: Path) -> None:
    with entry_app(tmp_path, "group") as app:
        root = tmp_path / "workspace"
        rows = item_actions(app, "groups", GROUP)
        assert titles(rows)[-2:] == ["Show tasks in this group", "Hide tasks from this group"]
        assert all(row["path"] == "Tasks" and row["enabled"] for row in rows[-2:])
        gateway(app, "palette.close")
        run_item_action(app, "groups", GROUP, "tasks.show")
        assert shown_scope(app) == ("group", GROUP)
        leave_views(app)

        run_item_action(app, "groups", GROUP, "group.hide_tasks")
        assert navigation(app)["modal"] is None
        state = entry(app)
        assert state["marks"] == {"groups": [GROUP], "notes": []}
        assert state["notice"]["text"] == HIDDEN_GROUP and state["notice"]["undo"]
        assert state["headers"]["notes"]["open_and_doing"] == 0
        assert state["headers"]["sections"]["open_and_doing"] == 2
        assert GROUP in (root / ".wren" / "tasks.toml").read_text(encoding="utf-8")
        assert titles(item_actions(app, "groups", GROUP))[-1] == "Show this group’s tasks again"
        gateway(app, "palette.close")
        # Undo brings it back.
        state = entry(app, undo=True)
        assert state["disposition"] == "completed", state
        assert state["marks"]["groups"] == [] and state["notice"] is None
        assert state["headers"]["notes"]["open_and_doing"] == 3
        assert GROUP not in (root / ".wren" / "tasks.toml").read_text(encoding="utf-8")
        # Hidden again, then shown again by the row's own action.
        run_item_action(app, "groups", GROUP, "group.hide_tasks")
        run_item_action(app, "groups", GROUP, "group.hide_tasks")
        state = entry(app)
        assert state["marks"]["groups"] == []
        assert state["notice"]["text"] == (
            "Work/Tasks back in task summaries · saved in .wren/tasks.toml")


def test_a_notes_actions_hide_its_tasks_in_its_front_matter_with_undo(tmp_path: Path) -> None:
    with entry_app(tmp_path, "note") as app:
        root = tmp_path / "workspace"
        # A note that is not open: written by the Session.
        assert titles(item_actions(app, "notes", NOTES))[-2:] == [
            "Show tasks in this note", "Hide tasks in this note"]
        gateway(app, "palette.close")
        run_item_action(app, "notes", NOTES, "note.hide_tasks")
        assert (root / NOTES).read_text(encoding="utf-8") == f"---\ntasks: off\n---\n{FILES[NOTES]}"
        state = entry(app)
        assert state["marks"] == {"groups": [], "notes": [NOTES]}
        assert state["notice"]["text"] == HIDDEN_NOTE
        assert state["notice"]["code"] == ["tasks: off"]
        assert state["headers"]["notes"]["open_and_doing"] == 2
        assert titles(item_actions(app, "notes", NOTES))[-1] == "Show tasks in this note again"
        gateway(app, "palette.close")
        state = entry(app, undo=True)
        assert state["disposition"] == "completed", state
        assert (root / NOTES).read_text(encoding="utf-8") == FILES[NOTES]
        assert state["marks"]["notes"] == []
        assert state["notice"]["text"] == (
            f"Undone: Tasks hidden · wrote the front matter of {NOTES}")
        assert state["notice"]["undo"] is False

        # The open note: through its editor, as a draft.
        run_item_action(app, "notes", LIST, "note.hide_tasks")
        assert app.client.call("editor.get")["text"] == f"---\ntasks: off\n---\n{FILES[LIST]}"
        assert session(app)["document"]["dirty"] is True
        state = entry(app)
        assert state["marks"]["notes"] == [LIST]
        assert state["notice"]["text"] == (
            "Tasks in List hidden · wrote tasks: off to its front matter")
        entry(app, undo=True)
        assert app.client.call("editor.get")["text"] == FILES[LIST]
        assert entry(app)["marks"]["notes"] == []

        # A board has no tasks to hide.
        rows = item_actions(app, "notes", FLOW)
        assert titles(rows)[-1] == "Show tasks in this note"
        assert all(row["key"]["item_action"]["command"] != "note.hide_tasks" for row in rows)
        gateway(app, "palette.close")
        # The message shows the last write and undoes that one alone; the
        # one it replaced is gone with its Undo.
        run_item_action(app, "notes", NOTES, "note.hide_tasks")
        run_item_action(app, "notes", EXTRA, "note.hide_tasks")
        assert sorted(entry(app)["marks"]["notes"]) == sorted([NOTES, EXTRA])
        entry(app, undo=True)
        assert entry(app)["marks"]["notes"] == [NOTES]
        assert (root / EXTRA).read_text(encoding="utf-8") == FILES[EXTRA]
        # The undo's own message undoes nothing.
        assert entry(app, undo=True)["disposition"] == "blocked"
        assert entry(app)["marks"]["notes"] == [NOTES]


def test_with_tasks_off_there_is_no_way_in(tmp_path: Path) -> None:
    with entry_app(tmp_path, "off", OFF) as app:
        call = app.client.call
        root = tmp_path / "workspace"
        assert entry(app) == {"tasks": False}
        for action in ({"click": "notes"}, {"undo": True}, {"dismiss": True}):
            with pytest.raises(RemoteError):
                entry(app, **action)
        rows = palette(app, "commands")["results"]
        keys = {row["key"]["command"] for row in rows}
        assert not keys & {"tasks.show", "view.toggle_task_counts", "group.hide_tasks",
                           "note.hide_tasks"}
        gateway(app, "palette.close")
        for collection, path in [("sections", "Work"), ("groups", GROUP), ("notes", NOTES)]:
            assert not [title for title in titles(item_actions(app, collection, path))
                        if "tasks" in title.lower()], path
            gateway(app, "palette.close")
        for command in ("tasks.show", "view.toggle_task_counts", "group.hide_tasks",
                        "note.hide_tasks"):
            gateway(app, "command.run", id=command, disposition="blocked")
            with pytest.raises(RemoteError):
                call("shortcut", {"id": command})
        # The chord is no command, and types nothing into the note.
        call("editor.focus")
        text = call("editor.get")["text"]
        call("editor.key", {"key": "t", "mods": ["mod", "shift"]})
        call("editor.key", {"key": "h", "mods": ["mod", "shift"]})
        assert call("editor.get")["text"] == text
        assert view(app) == {"tasks": False}
        assert entry(app) == {"tasks": False}
        menus = call("menu.get")["menus"]
        assert not [entry for menu in menus for entry in menu["items"]
                    if entry.get("key") in ("tasks.show", "view.toggle_task_counts")]
        assert not (root / ".wren" / "tasks.toml").exists()
        assert (root / NOTES).read_text(encoding="utf-8") == FILES[NOTES]


@pytest.mark.native_x11
def test_native_show_tasks_chord_opens_the_notes_group(tmp_path: Path) -> None:
    with entry_app(tmp_path, "native") as app:
        call = app.client.call
        window = native.focus_native(app)
        call("editor.focus")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard")
        text = call("editor.get")["text"]
        native.keys(window, native_mod("shift+t"))
        wait_for(lambda: view(app)["open"], description="Mod+Shift+T opens the task view")
        assert shown_scope(app) == ("group", GROUP)
        native.settle_focus()
        assert call("editor.get")["text"] == text
        assert entry(app)["headers"]["notes"]["pressed"] is True
        # Back leaves it; the chord from the Sections column opens the
        # section.
        native.keys(window, native_mod("bracketleft"))
        wait_for(lambda: not view(app)["open"], description="Mod+[ leaves the view")
        gateway(app, "column.open", column="sections")
        native.keys(window, native_mod("shift+t"))
        wait_for(lambda: view(app)["open"], description="Mod+Shift+T from Sections")
        assert shown_scope(app) == ("section", "Work")
        assert call("editor.get")["text"] == text
        # Mod+Shift+H hides the counts and shows them again.
        native.keys(window, native_mod("shift+h"))
        wait_for(lambda: entry(app)["counts"] is False, description="Mod+Shift+H hides the counts")
        native.settle_focus()
        native.keys(window, native_mod("shift+h"))
        wait_for(lambda: entry(app)["counts"] is True, description="Mod+Shift+H shows them")
        assert call("editor.get")["text"] == text
