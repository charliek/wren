"""The task menus and the box click (discovery/tasks-build.md item 3.4;
discovery/tasks.md §2 items 20 and 22, §5.1 and §5.7): a right-click on a
task line opens the task menu, on its box the status menu and on its
priority glyph the priority menu, each with its single keys; a click on the
box cycles the status; ``task.set_status`` and ``task.set_priority`` open
the status and priority menus at the caret's task. All of it is the
advanced engine's, with ``tasks = on``: on the classic engine and with tasks
off a right-click and a click on a box do what they did before.

Each case writes its own ``tasks`` and ``spell-check`` lines and the engine
``WRENTEST_ENGINE`` names, pins today to Thursday 19 November 2026, and
asserts the note's bytes on disk. The ``native_x11`` cases click with the
real pointer.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
from pathlib import Path

import pytest

from tools.wrentest import test_gestures as gestures
from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import engine, focus_native, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_baseline import saved
from tools.wrentest.ui import WrenApp, wait_for

TODAY = "2026-11-19"
NOTE = "Tasks/Menus/Menus.md"
LINES = [
    "# Menus",
    "",
    "- [ ] Open alpha @sam 📅 2026-11-23",
    "- [/] Doing bravo ⏫",
    "- [x] Done charlie ✅ 2026-11-18",
    "- [-] Cancelled delta ❌ 2026-11-17",
    "- [?] Unknown echo",
    "- [ ] Selector foxtrot ⏫️ 📅 2026-12-01",
    "- [ ] Spelling with a mispelled golf",
    "",
    "Plain hotel.",
]
ORIGINAL = "\n".join(LINES)
STATUS = ["Open", "Doing", "Done", "Cancelled"]
PRIORITY = ["High", "Medium", "Low", "Highest", "Lowest", "None"]
TASK = STATUS + ["Change due date…", "Priority…", "Edit task…"]


def width(text: str) -> int:
    """A column as the editor counts it: UTF-8 bytes."""
    return len(text.encode("utf-8"))


def line_of(start: str) -> int:
    return next(index for index, text in enumerate(LINES) if text.startswith(start))


def column_of(line: int, token: str) -> int:
    return width(LINES[line][:LINES[line].index(token)])


def replaced(line: int, new: str) -> str:
    return "\n".join(LINES[:line] + [new] + LINES[line + 1:])


@contextmanager
def menus_app(tmp_path: Path, engine: str, tasks: str = "on",  # noqa: F811
              settings: str = "spell-check = off\n") -> Iterator[tuple[WrenApp, Path]]:
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\ntasks = {tasks}\n{settings}", today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-menus-{tasks}-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert app.client.call("tasks.get")["tasks"] is (tasks == "on")
        assert app.client.call("session.get")["banner"] is None
        note = next(row["id"] for row in app.client.call("session.get")["notes"]
                    if row["path"] == NOTE)
        app.client.call("core.apply", {"op": "open_note", "note": note})
        wait_for(lambda: app.client.call("editor.get")["text"] == ORIGINAL,
                 description="the note opens")
        app.client.call("editor.focus")
        app.client.call("editor.cursor", {"line": 1, "column": 0, "selection": None})
        yield app, path


def right_click(app: WrenApp, line: int, token: str) -> dict | None:
    """A right-click on the first character of `token` on `line`: the
    popover it leaves open."""
    state = app.client.call("editor.right_click", {"line": line, "column": column_of(line, token)})
    return state["popover"]


def labels(popover: dict | None) -> list[str]:
    assert popover is not None, "a menu is open"
    return [row["label"] for row in popover["rows"]]


def checked(popover: dict) -> list[str]:
    return [row["label"] for row in popover["rows"] if row["checked"]]


def key(app: WrenApp, name: str, mods: list[str] | None = None) -> dict:
    return app.client.call("editor.key", {"key": name, "mods": mods or []})


def undo_to_original(app: WrenApp, path: Path) -> None:
    app.client.call("editor.undo")
    state = app.client.call("editor.get")
    assert state["text"] == ORIGINAL, "one undo step"
    assert state["can_undo"] is False, "one undo step and no more"
    saved(app, path, ORIGINAL)


def test_each_menu_opens_on_its_token_with_its_rows_and_the_current_one_marked(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with menus_app(tmp_path, engine) as (app, path):
        doing = line_of("- [/] Doing")
        if engine == "classic":
            # The classic engine opens no popover of the app's.
            for token in ["[", "⏫", "bravo"]:
                assert right_click(app, doing, token) is None, token
            saved(app, path, ORIGINAL)
            return
        status = right_click(app, doing, "[")
        assert labels(status) == STATUS
        assert (status["line"], status["start"], status["end"]) == (doing, 2, 5)
        assert [row["key"] for row in status["rows"]] == ["Space", "/", "X", "-"]
        assert [row["glyph"] for row in status["rows"]] == ["[ ]", "[/]", "[x]", "[-]"]
        assert checked(status) == ["Doing"] and status["selected"] == 1
        key(app, "escape")

        priority = right_click(app, doing, "⏫")
        assert labels(priority) == PRIORITY
        assert [row["key"] for row in priority["rows"]] == ["H", "M", "L", "⇧H", "⇧L", "N"]
        assert checked(priority) == ["High"] and priority["selected"] == 0
        assert priority["text"] == "⏫"
        key(app, "escape")

        # The description and an owner open the task menu; Edit task is
        # shown and cannot be chosen yet. The due date opens the calendar
        # (item 4.1).
        open_line = line_of("- [ ] Open")
        for token in ["alpha", "@sam"]:
            task = right_click(app, open_line, token)
            assert labels(task) == TASK, token
            assert [row["enabled"] for row in task["rows"]] == [True] * 4 + [True, True, False]
            assert [row["key"] for row in task["rows"]] == ["Space", "/", "X", "-", "D", "P", "E"]
            assert checked(task) == ["Open"], token
            key(app, "escape")
        assert labels(right_click(app, doing, "bravo"))[4] == "Set due date…"
        key(app, "escape")
        for token in ["📅", "2026"]:
            assert right_click(app, open_line, token) is None, token
            assert app.client.call("tasks.calendar")["line"] == open_line, token
            key(app, "escape")
            assert app.client.call("tasks.calendar") == {"open": False}, token
        # A box of no status of its own marks none, and opens on Open.
        unknown = right_click(app, line_of("- [?] Unknown"), "?")
        assert checked(unknown) == [] and unknown["selected"] == 0
        key(app, "escape")
        # Off a task line, nothing opens.
        assert right_click(app, line_of("Plain hotel."), "hotel") is None
        assert right_click(app, line_of("# Menus"), "Menus") is None
        saved(app, path, ORIGINAL)


# (line's start, token right-clicked, key, mods, the line after)
CASES = [
    ("- [ ] Open", "[", "/", [], "- [/] Open alpha @sam 📅 2026-11-23"),
    ("- [ ] Open", "]", "x", [], "- [x] Open alpha @sam 📅 2026-11-23 ✅ 2026-11-19"),
    ("- [ ] Open", "[", "-", [], "- [-] Open alpha @sam 📅 2026-11-23 ❌ 2026-11-19"),
    ("- [/] Doing", "/", "space", [], "- [ ] Doing bravo ⏫"),
    ("- [x] Done", "x", "space", [], "- [ ] Done charlie"),
    ("- [x] Done", "x", "-", [], "- [-] Done charlie ❌ 2026-11-19"),
    ("- [-] Cancelled", "-]", "/", [], "- [/] Cancelled delta"),
    ("- [?] Unknown", "?", "space", [], "- [ ] Unknown echo"),
    ("- [/] Doing", "⏫", "m", [], "- [/] Doing bravo 🔼"),
    ("- [/] Doing", "⏫", "l", [], "- [/] Doing bravo 🔽"),
    ("- [/] Doing", "⏫", "h", ["shift"], "- [/] Doing bravo 🔺"),
    ("- [/] Doing", "⏫", "l", ["shift"], "- [/] Doing bravo ⏬"),
    ("- [/] Doing", "⏫", "n", [], "- [/] Doing bravo"),
    ("- [ ] Selector", "⏫", "m", [], "- [ ] Selector foxtrot 🔼 📅 2026-12-01"),
    ("- [ ] Open", "alpha", "x", [], "- [x] Open alpha @sam 📅 2026-11-23 ✅ 2026-11-19"),
    ("- [ ] Open", "@sam", "/", [], "- [/] Open alpha @sam 📅 2026-11-23"),
    ("- [x] Done", "✅", "space", [], "- [ ] Done charlie"),
]


def test_each_accelerator_writes_the_line_as_one_undo_step(tmp_path: Path) -> None:
    with menus_app(tmp_path, "advanced") as (app, path):
        for start, token, name, mods, after in CASES:
            line = line_of(start)
            assert right_click(app, line, token) is not None, (start, token)
            state = key(app, name, mods)
            assert state["popover"] is None, (start, token, name)
            expected = replaced(line, after)
            assert state["text"] == expected, (start, token, name, mods)
            assert state["cursor"]["line"] == 1, "the caret stays"
            saved(app, path, expected)
            undo_to_original(app, path)


def test_priority_opens_the_priority_menu_and_escape_writes_nothing(tmp_path: Path) -> None:
    with menus_app(tmp_path, "advanced") as (app, path):
        line = line_of("- [ ] Open")
        right_click(app, line, "alpha")
        # E names a row that cannot be chosen yet: nothing happens.
        for name in ["e"]:
            state = key(app, name)
            assert labels(state["popover"]) == TASK and state["text"] == ORIGINAL, name
        state = key(app, "p")
        assert labels(state["popover"]) == PRIORITY
        assert state["popover"]["line"] == line and checked(state["popover"]) == ["None"]
        state = key(app, "h")
        expected = replaced(line, "- [ ] Open alpha @sam ⏫ 📅 2026-11-23")
        assert state["text"] == expected
        saved(app, path, expected)
        undo_to_original(app, path)
        # Escape closes each menu and writes nothing; so does a press of
        # a key that is no row's, which then types.
        for token in ["[", "alpha", "📅"]:
            right_click(app, line, token)
            state = key(app, "escape")
            assert state["popover"] is None and state["text"] == ORIGINAL, token
        right_click(app, line_of("- [/] Doing"), "⏫")
        state = key(app, "escape")
        assert state["popover"] is None and state["text"] == ORIGINAL
        assert state["can_undo"] is False
        saved(app, path, ORIGINAL)


def test_a_misspelled_word_on_a_task_line_opens_spelling_first(tmp_path: Path) -> None:
    settings = "spell-check = on\n"
    with menus_app(tmp_path, "advanced", settings=settings) as (app, path):
        wait_for(lambda: app.client.call("editor.spell")["ready"], description="the dictionary loads")
        line = line_of("- [ ] Spelling")
        wait_for(lambda: any(mark["word"] == "mispelled"
                             for mark in app.client.call("editor.spell")["marks"]),
                 description="the misspelling is marked")
        popover = right_click(app, line, "mispelled")
        assert popover["spelling"] is True
        assert "misspelled" in labels(popover)
        key(app, "escape")
        app.client.call("editor.spell_event", {"event": "close"})
        task = right_click(app, line, "golf")
        assert task["spelling"] is False
        assert labels(task) == STATUS + ["Set due date…", "Priority…", "Edit task…"]
        state = key(app, "escape")
        assert state["text"] == ORIGINAL
        saved(app, path, ORIGINAL)


def test_set_status_and_set_priority_open_at_the_carets_task(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    trigger = "super+shift+s" if os.uname().sysname == "Darwin" else "alt+shift+s"
    mods = ["logo", "shift"] if os.uname().sysname == "Darwin" else ["alt", "shift"]
    settings = f"spell-check = off\nkeybind = {trigger} = task.set_status\n"
    with menus_app(tmp_path, engine, settings=settings) as (app, path):
        call = app.client.call
        assert call("session.get")["banner"] is None
        doing = line_of("- [/] Doing")
        call("editor.cursor", {"line": doing, "column": 8, "selection": None})
        if engine == "classic":
            for command in ["task.set_status", "task.set_priority"]:
                assert call("command.run", {"id": command})["disposition"] == "blocked"
                assert call("editor.get")["popover"] is None
            key(app, "s", mods)
            assert call("editor.get")["popover"] is None
            saved(app, path, ORIGINAL)
            return
        assert call("command.run", {"id": "task.set_status"})["disposition"] == "completed"
        popover = call("editor.get")["popover"]
        assert labels(popover) == STATUS and (popover["start"], popover["end"]) == (2, 5)
        state = key(app, "x")
        expected = replaced(doing, "- [x] Doing bravo ⏫ ✅ 2026-11-19")
        assert state["text"] == expected
        saved(app, path, expected)
        undo_to_original(app, path)
        assert call("command.run", {"id": "task.set_priority"})["disposition"] == "completed"
        popover = call("editor.get")["popover"]
        assert labels(popover) == PRIORITY and popover["text"] == "⏫"
        state = key(app, "m")
        assert state["text"] == replaced(doing, "- [/] Doing bravo 🔼")
        undo_to_original(app, path)
        # The bound key asks for the menu.
        key(app, "s", mods)
        assert labels(call("editor.get")["popover"]) == STATUS
        state = key(app, "space")
        assert state["text"] == replaced(doing, "- [ ] Doing bravo ⏫")
        undo_to_original(app, path)
        # Off a task line they are unavailable.
        call("editor.cursor", {"line": line_of("Plain hotel."), "column": 0, "selection": None})
        for command in ["task.set_status", "task.set_priority"]:
            assert call("command.run", {"id": command})["disposition"] == "blocked"
        assert call("editor.get")["popover"] is None
        saved(app, path, ORIGINAL)


def test_a_click_on_the_box_cycles_the_status(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with menus_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Open")
        if engine == "classic":
            state = call("editor.click", {"line": line, "column": 3})
            assert state["text"] == ORIGINAL and state["cursor"]["line"] == line
            saved(app, path, ORIGINAL)
            return
        for column, after in [
            (2, "- [/] Open alpha @sam 📅 2026-11-23"),
            (3, "- [x] Open alpha @sam 📅 2026-11-23 ✅ 2026-11-19"),
            (4, "- [ ] Open alpha @sam 📅 2026-11-23"),
        ]:
            state = call("editor.click", {"line": line, "column": column})
            assert state["text"] == replaced(line, after), column
            assert state["cursor"] == {"line": 1, "column": 0}, "the caret stays"
            saved(app, path, replaced(line, after))
        for _ in range(3):
            call("editor.undo")
        assert call("editor.get")["text"] == ORIGINAL and call("editor.get")["can_undo"] is False
        for start, after in [("- [-] Cancelled", "- [ ] Cancelled delta"),
                             ("- [?] Unknown", "- [/] Unknown echo")]:
            at = line_of(start)
            state = call("editor.click", {"line": at, "column": 3})
            assert state["text"] == replaced(at, after), start
            undo_to_original(app, path)
        # Off the box a click places the caret.
        state = call("editor.click", {"line": line, "column": column_of(line, "alpha")})
        assert state["text"] == ORIGINAL and state["cursor"]["line"] == line
        saved(app, path, ORIGINAL)


def test_with_tasks_off_there_are_no_menus_and_no_box_clicks(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with menus_app(tmp_path, engine, tasks="off") as (app, path):
        call = app.client.call
        line = line_of("- [ ] Open")
        for token in ["[", "alpha", "📅"]:
            assert right_click(app, line, token) is None, token
        state = call("editor.click", {"line": line, "column": 3})
        assert state["text"] == ORIGINAL and state["cursor"]["line"] == line
        for command in ["task.set_status", "task.set_priority"]:
            assert call("command.run", {"id": command})["disposition"] == "blocked"
        with pytest.raises(RemoteError):
            call("editor.open_menu", {"rows": [{"command": "task.set_status"}]})
        saved(app, path, ORIGINAL)


def drawn(app: WrenApp, line: int, token: str, length: int | None = None) -> tuple[int, int]:
    """The window point in the middle of `token` on `line`, as drawn."""
    start = column_of(line, token)
    end = start + (length if length is not None else width(token))
    rect = wait_for(lambda: _bounds(app, line, start, end), description=f"{token!r} is drawn")
    return round(rect["x"] + rect["width"] / 2), round(rect["y"] + rect["height"] / 2)


def _bounds(app: WrenApp, line: int, start: int, end: int) -> dict | None:
    try:
        return app.client.call("editor.bounds", {"line": line, "start": start, "end": end})
    except RemoteError:
        return None


@pytest.mark.native_x11
def test_native_click_on_the_box_cycles_and_a_drag_from_it_does_not(tmp_path: Path) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with menus_app(tmp_path, "advanced") as (app, path):
        call = app.client.call
        window = focus_native(app)
        line = line_of("- [ ] Open")
        box = drawn(app, line, "[ ]")
        gestures.move(window, box)
        xdo("click", "1")
        expected = replaced(line, "- [/] Open alpha @sam 📅 2026-11-23")
        wait_for(lambda: call("editor.get")["text"] == expected, description="the click cycles")
        assert call("editor.get")["cursor"] == {"line": 1, "column": 0}, "the caret stays"
        saved(app, path, expected)
        # A drag that starts on the box selects, and cycles nothing.
        xdo("mousedown", "1")
        gestures.move(window, drawn(app, line, "alpha"))
        xdo("mouseup", "1")
        wait_for(lambda: call("editor.get")["cursor"]["line"] == line,
                 description="the drag selects")
        assert call("editor.get")["text"] == expected
        # A press elsewhere first, so the next is no double click.
        gestures.move(window, drawn(app, line_of("Plain hotel."), "hotel"))
        xdo("click", "1")
        gestures.move(window, box)
        xdo("click", "1")
        expected = replaced(line, "- [x] Open alpha @sam 📅 2026-11-23 ✅ 2026-11-19")
        wait_for(lambda: call("editor.get")["text"] == expected, description="done, stamped")
        saved(app, path, expected)


@pytest.mark.native_x11
def test_native_right_click_on_each_token_opens_its_menu(tmp_path: Path) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with menus_app(tmp_path, "advanced") as (app, path):
        call = app.client.call
        window = focus_native(app)
        open_line, doing = line_of("- [ ] Open"), line_of("- [/] Doing")
        for line, token, length, rows in [
            (open_line, "[ ]", None, STATUS),
            (doing, "⏫", None, PRIORITY),
            (open_line, "@sam", None, TASK),
            (open_line, "alpha", None, TASK),
        ]:
            gestures.move(window, drawn(app, line, token, length))
            xdo("click", "3")
            popover = wait_for(lambda: call("editor.get")["popover"], description=f"{token}'s menu")
            assert labels(popover) == rows, token
            assert popover["line"] == line, token
            xdo("key", "--window", window, "Escape")
            wait_for(lambda: call("editor.get")["popover"] is None, description="Escape closes it")
        # The due date opens the calendar.
        gestures.move(window, drawn(app, open_line, "📅"))
        xdo("click", "3")
        wait_for(lambda: call("tasks.calendar")["open"], description="the calendar")
        assert call("editor.get")["popover"] is None
        xdo("key", "--window", window, "Escape")
        wait_for(lambda: call("tasks.calendar") == {"open": False}, description="Escape closes it")
        assert call("editor.get")["text"] == ORIGINAL
        # A real key chooses from the status menu.
        gestures.move(window, drawn(app, doing, "[/]"))
        xdo("click", "3")
        wait_for(lambda: call("editor.get")["popover"], description="the status menu")
        xdo("key", "--window", window, "x")
        expected = replaced(doing, "- [x] Doing bravo ⏫ ✅ 2026-11-19")
        wait_for(lambda: call("editor.get")["text"] == expected, description="x writes done")
        saved(app, path, expected)
