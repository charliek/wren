"""The task form (discovery/tasks-build.md item 6.1; discovery/tasks.md §6,
item 3): ``task.edit`` (⇧⌘↵ / Ctrl+Shift+Enter) opens "Create or edit task"
at the caret's line, on either engine: the description, the status, the due
date with the calendar, the priority and the owners, and a "Writes" line
showing the Markdown that saving writes. Saving writes only the fields that
changed, as one undo step, and is refused while the line has changed since
the form opened. On a blank line it makes a task; on a line of text it makes
that text a task. Tab walks the fields, Enter saves, Escape cancels, and the
editor gets the keyboard back with its caret where it was.

While the form is open, ``editor.key`` presses keys in it. Each case writes
its own ``tasks`` line and the engine ``WRENTEST_ENGINE`` names, pins today
to Thursday 19 November 2026, and asserts the note's bytes on disk.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
from pathlib import Path
import time

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import engine, focus_native, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_baseline import saved
from tools.wrentest.ui import WrenApp, native_mod, wait_for

TODAY = "2026-11-19"
NOTE = "Tasks/Form/Form.md"
LINES = [
    "# Form",
    "",
    "- [/] Draft the runbook @alice ⏫ 📅 2026-11-23",
    "- [ ] Two priorities ⏫ 🔼",
    "- [ ] Owed by @sam",
    "",
    "Plain line",
    "",
]
ORIGINAL = "\n".join(LINES)


def width(text: str) -> int:
    """A column as the editor counts it: UTF-8 bytes."""
    return len(text.encode("utf-8"))


def line_of(start: str) -> int:
    return next(index for index, text in enumerate(LINES) if text.startswith(start))


def replaced(line: int, new: str) -> str:
    return "\n".join(LINES[:line] + [new] + LINES[line + 1:])


@contextmanager
def form_app(tmp_path: Path, engine: str, tasks: str = "on",  # noqa: F811
             ) -> Iterator[tuple[WrenApp, Path]]:
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\ntasks = {tasks}\n", today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-form-{tasks}-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert app.client.call("tasks.get")["tasks"] is (tasks == "on")
        assert app.client.call("session.get")["banner"] is None
        note = next(row["id"] for row in app.client.call("session.get")["notes"]
                    if row["path"] == NOTE)
        app.client.call("core.apply", {"op": "open_note", "note": note})
        wait_for(lambda: app.client.call("editor.get")["text"] == ORIGINAL,
                 description="the note opens")
        app.client.call("editor.focus")
        yield app, path


def caret_at(app: WrenApp, line: int, column: int | None = None) -> dict:
    column = width(LINES[line]) if column is None else column
    app.client.call("editor.cursor", {"line": line, "column": column, "selection": None})
    return {"line": line, "column": column}


def form(app: WrenApp) -> dict:
    return app.client.call("tasks.form")


def open_form(app: WrenApp) -> dict:
    """``task.edit``'s key, as the focused editor routes it."""
    app.client.call("shortcut", {"id": "task.edit"})
    wait_for(lambda: app.client.call("focus.get")["named"] == ["task-form-description"],
             description="the description takes the keyboard")
    return form(app)


def press(app: WrenApp, *keys: str) -> None:
    for key in keys:
        app.client.call("editor.key", {"key": key})


def type_keys(app: WrenApp, text: str) -> None:
    press(app, *[character if character != " " else "space" for character in text])


def editor_back(app: WrenApp, cursor: dict) -> None:
    """The form is closed and the editor has the keyboard, its caret where
    it was."""
    assert form(app) == {"open": False}
    wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
             description="the editor has the keyboard back")
    assert app.client.call("editor.get")["cursor"] == cursor


def undo_to_original(app: WrenApp, path: Path) -> None:
    app.client.call("editor.undo")
    state = app.client.call("editor.get")
    assert state["text"] == ORIGINAL, "one undo step"
    assert state["can_undo"] is False, "one undo step and no more"
    saved(app, path, ORIGINAL)


def test_a_blank_line_becomes_a_new_task(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with form_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("Plain") + 1
        caret_at(app, line, 0)
        state = open_form(app)
        assert (state["kind"], state["title"], state["line"]) == ("new", "New task", line)
        assert (state["description"], state["status"], state["due"]) == ("", "Open", "")
        assert state["writes"] == "- [ ] "
        type_keys(app, "Call the bank")
        press(app, "tab", "tab")
        assert form(app)["focus"] == "due"
        type_keys(app, "fri")
        state = form(app)
        assert state["due_caption"] == "Friday 20 November 2026 · tomorrow", state
        assert state["writes"] == "- [ ] Call the bank 📅 2026-11-20"
        assert call("editor.get")["text"] == ORIGINAL, "nothing is written before Save"
        press(app, "enter")
        written = "- [ ] Call the bank 📅 2026-11-20"
        expected = replaced(line, written)
        assert call("editor.get")["text"] == expected
        editor_back(app, {"line": line, "column": width(written)})
        saved(app, path, expected)
        undo_to_original(app, path)

        # A heading has no form.
        caret_at(app, line_of("# Form"), 2)
        assert call("command.run", {"id": "task.edit"})["disposition"] == "blocked"
        assert form(app) == {"open": False}

        # A line of text becomes a task with its text as the description.
        plain = line_of("Plain")
        caret_at(app, plain, 3)
        state = open_form(app)
        assert (state["kind"], state["title"], state["description"]) == (
            "make", "Make a task", "Plain line")
        press(app, "enter")
        expected = replaced(plain, "- [ ] Plain line")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)


def test_editing_a_task_writes_each_field_changed(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with form_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [/] Draft")
        cursor = caret_at(app, line, width("- [/] Draft"))
        state = open_form(app)
        assert (state["kind"], state["title"]) == ("edit", "Edit task")
        assert state == {**state, "description": "Draft the runbook @alice", "status": "Doing",
                         "due": "2026-11-23", "priority": "High", "owners": ["alice"],
                         "writes": LINES[line]}
        steps = [
            (lambda: call("tasks.form_type", {"field": "description",
                                              "text": "Draft the plan @alice"}),
             "- [/] Draft the plan @alice ⏫ 📅 2026-11-23"),
            (lambda: call("tasks.form_click", {"status": "Done"}),
             "- [x] Draft the plan @alice ⏫ 📅 2026-11-23 ✅ 2026-11-19"),
            (lambda: call("tasks.form_type", {"field": "due", "text": "+2w"}),
             "- [x] Draft the plan @alice ⏫ 📅 2026-12-03 ✅ 2026-11-19"),
            (lambda: call("tasks.form_click", {"priority": "Low"}),
             "- [x] Draft the plan @alice 🔽 📅 2026-12-03 ✅ 2026-11-19"),
            # The priority's row has the keyboard after its click: Tab
            # moves to the owners.
            (lambda: (press(app, "tab"), type_keys(app, "priya"), press(app, "enter")),
             "- [x] Draft the plan @alice @priya 🔽 📅 2026-12-03 ✅ 2026-11-19"),
            (lambda: call("tasks.form_click", {"remove": "alice"}),
             "- [x] Draft the plan @priya 🔽 📅 2026-12-03 ✅ 2026-11-19"),
        ]
        for step, writes in steps:
            step()
            assert form(app)["writes"] == writes
        assert form(app)["owners"] == ["priya"]
        assert call("editor.get")["text"] == ORIGINAL
        call("tasks.form_click", {"target": "save"})
        expected = replaced(line, steps[-1][1])
        assert call("editor.get")["text"] == expected
        # The caret was in the description: it ends the new one.
        editor_back(app, {"line": line, "column": width("- [x] Draft the plan")})
        saved(app, path, expected)
        undo_to_original(app, path)
        del cursor

        # A name typed in the owners field and not added is in the Writes
        # line, as typed, and Save writes it.
        owed = line_of("- [ ] Owed")
        caret_at(app, owed)
        open_form(app)
        call("tasks.form_type", {"field": "owners", "text": "Priya"})
        assert form(app)["writes"] == "- [ ] Owed by @sam @Priya"
        assert form(app)["owners"] == ["sam"]
        call("tasks.form_click", {"target": "save"})
        expected = replaced(owed, "- [ ] Owed by @sam @Priya")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)
        undo_to_original(app, path)

        # Only what changed is written: a line with two priorities keeps
        # them when only the status changes, and a form nobody changed
        # writes nothing.
        two = line_of("- [ ] Two")
        cursor = caret_at(app, two)
        open_form(app)
        press(app, "enter")
        editor_back(app, cursor)
        assert call("editor.get")["text"] == ORIGINAL
        assert call("editor.get")["can_undo"] is False
        open_form(app)
        call("tasks.form_click", {"status": "Doing"})
        press(app, "enter")
        expected = replaced(two, "- [/] Two priorities ⏫ 🔼")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)


def test_saving_is_refused_once_the_line_changed(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with form_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Owed")
        cursor = caret_at(app, line)
        open_form(app)
        call("tasks.form_click", {"status": "Done"})
        # The note changes on disk while the form is open, and is reloaded.
        changed = replaced(line, "- [ ] Owed by @sam and @priya")
        path.write_text(changed, encoding="utf-8")
        wait_for(lambda: call("editor.get")["text"] == changed, timeout=10,
                 description="the note is reloaded")
        state = call("tasks.form_click", {"target": "save"})
        assert state["open"] is True, "the form stays open"
        assert state["error"] == "The task changed since the form opened; nothing was written"
        assert call("editor.get")["text"] == changed
        press(app, "escape")
        editor_back(app, cursor)
        assert call("editor.get")["text"] == changed
        saved(app, path, changed)


def test_the_keyboard_alone_fills_and_saves_the_form(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with form_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Owed")
        cursor = caret_at(app, line, 3)
        open_form(app)
        order = []
        for _ in range(7):
            order.append(form(app)["focus"])
            press(app, "tab")
        assert order == ["description", "status", "due", "priority", "owners", "save", "cancel"]
        assert form(app)["focus"] == "description", "round the ends"
        press(app, "backspace", "backspace", "backspace", "backspace")
        type_keys(app, "the deck")
        press(app, "tab", "right")
        assert form(app)["status"] == "Doing"
        press(app, "tab")
        type_keys(app, "fri")
        press(app, "tab", "h", "tab")
        type_keys(app, "priya")
        press(app, "enter")
        assert form(app)["owners"] == ["priya"]
        assert form(app)["open"] is True, "Enter takes the name first"
        written = "- [/] Owed by the deck @priya ⏫ 📅 2026-11-20"
        assert form(app)["writes"] == written
        press(app, "enter")
        expected = replaced(line, written)
        assert call("editor.get")["text"] == expected
        editor_back(app, cursor)
        saved(app, path, expected)

        # Escape cancels, writing nothing, whatever was typed.
        open_form(app)
        type_keys(app, " more")
        press(app, "tab", "x")
        press(app, "escape")
        editor_back(app, cursor)
        assert call("editor.get")["text"] == expected

        # Down chooses an owner with nothing typed, and Enter adds it.
        open_form(app)
        press(app, "tab", "tab", "tab", "tab")
        assert form(app)["focus"] == "owners"
        suggestions = form(app)["suggestions"]
        assert suggestions, "the workspace's owners are offered"
        press(app, "down", "enter")
        state = form(app)
        assert state["open"] is True and suggestions[0] in state["owners"], state
        press(app, "escape")
        editor_back(app, cursor)
        assert call("editor.get")["text"] == expected

        # Shift+Tab to Cancel, and Enter there cancels.
        open_form(app)
        call("editor.key", {"key": "tab", "mods": ["shift"]})
        assert form(app)["focus"] == "cancel"
        call("tasks.form_click", {"status": "Done"})
        call("editor.key", {"key": "tab", "mods": ["shift"]})
        call("editor.key", {"key": "tab", "mods": ["shift"]})
        assert form(app)["focus"] == "cancel"
        press(app, "enter")
        editor_back(app, cursor)
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)


def test_the_calendar_in_the_form_gives_the_form_back_the_keyboard(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with form_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [/] Draft")
        cursor = caret_at(app, line)
        open_form(app)
        press(app, "tab", "tab", "down")
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="the calendar's field takes the keyboard")
        state = form(app)
        assert state["calendar"]["selected"] == "2026-11-23", state
        type_keys(app, "fri")
        assert form(app)["calendar"]["selected"] == "2026-11-20"
        press(app, "enter")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-due"],
                 description="the due field has the keyboard back")
        state = form(app)
        assert (state["calendar"], state["due"], state["focus"]) == (None, "2026-11-20", "due")
        assert call("editor.get")["text"] == ORIGINAL, "the calendar writes the field, not the note"

        # Escape closes the calendar alone, writing nothing.
        call("tasks.form_click", {"target": "calendar"})
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="the calendar's field takes the keyboard again")
        press(app, "down", "escape")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-due"],
                 description="the due field has the keyboard back")
        state = form(app)
        assert (state["open"], state["calendar"], state["due"]) == (True, None, "2026-11-20")

        # A click on a day, and Clear.
        call("tasks.form_click", {"target": "calendar"})
        call("tasks.form_click", {"day": "2026-11-25"})
        assert form(app)["due"] == "2026-11-25"
        call("tasks.form_click", {"target": "calendar"})
        call("tasks.form_click", {"calendar": "clear"})
        assert form(app)["due"] == ""
        press(app, "enter")
        written = "- [/] Draft the runbook @alice ⏫"
        expected = replaced(line, written)
        assert call("editor.get")["text"] == expected
        # The caret ended the line: it ends it still.
        editor_back(app, {**cursor, "column": width(written)})
        saved(app, path, expected)


def test_the_task_menus_edit_row_opens_the_form(tmp_path: Path, engine: str) -> None:  # noqa: F811
    if engine == "classic":
        pytest.skip("the task menus are the advanced engine's")
    with form_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Owed")
        cursor = caret_at(app, 0, 0)
        menu = call("editor.right_click", {"line": line, "column": width("- [ ] Ow")})["popover"]
        assert menu["rows"][-1]["label"] == "Edit task…" and menu["rows"][-1]["enabled"], menu
        press(app, "e")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-description"],
                 description="the form opens for the task")
        assert (form(app)["kind"], form(app)["line"]) == ("edit", line)
        press(app, "escape")
        editor_back(app, cursor)
        assert call("editor.get")["text"] == ORIGINAL
        saved(app, path, ORIGINAL)


def test_with_tasks_off_there_is_no_form(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with form_app(tmp_path, engine, tasks="off") as (app, path):
        call = app.client.call
        caret_at(app, line_of("- [/] Draft"))
        assert call("command.run", {"id": "task.edit"})["disposition"] == "blocked"
        mods = ["mod", "shift"]
        call("editor.key", {"key": "enter", "mods": mods})
        with pytest.raises(RemoteError, match="Tasks are off"):
            call("tasks.form")
        assert call("focus.get")["named"] == ["note-editor"]
        saved(app, path, call("editor.get")["text"])


@pytest.mark.native_x11
def test_native_ctrl_shift_enter_opens_it_and_the_keyboard_fills_it(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with form_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("Plain") + 1
        cursor = caret_at(app, line, 0)
        window = focus_native(app)

        def key(*keys: str) -> None:
            xdo("key", "--window", window, "--clearmodifiers", *keys)

        def typed(text: str) -> None:
            xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", text)

        key(native_mod("shift+Return"))
        wait_for(lambda: form(app)["open"], description="Ctrl+Shift+Enter opens the form")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-description"],
                 description="the description takes the keyboard")
        typed("Call the bank")
        wait_for(lambda: form(app)["description"] == "Call the bank",
                 description="typing reaches the description")
        key("Tab", "Right")
        wait_for(lambda: form(app)["status"] == "Doing", description="Right picks Doing")
        key("Tab")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-due"],
                 description="Tab reaches the due field")
        key("Down")
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="Down opens the calendar")
        typed("fri")
        wait_for(lambda: (form(app)["calendar"] or {}).get("typed") == "fri",
                 description="typing reaches the calendar")
        key("Return")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-due"],
                 description="the calendar gives the due field the keyboard back")
        assert form(app)["due"] == "2026-11-20"
        key("Tab", "h", "Tab")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-owners"],
                 description="Tab reaches the owners")
        typed("sam")
        wait_for(lambda: form(app)["owner_text"] == "sam", description="typing reaches the owners")
        key("Return")
        wait_for(lambda: form(app)["owners"] == ["sam"], description="Enter takes the name")
        assert call("editor.get")["text"] == ORIGINAL
        key("Return")
        written = "- [/] Call the bank @sam ⏫ 📅 2026-11-20"
        expected = replaced(line, written)
        wait_for(lambda: call("editor.get")["text"] == expected, description="Enter saves")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard back")
        assert call("editor.get")["cursor"] == {"line": line, "column": width(written)}
        saved(app, path, expected)
        del cursor

        # Escape cancels and nothing reaches the note.
        caret = caret_at(app, line_of("- [ ] Owed"))
        call("editor.focus")
        key(native_mod("shift+Return"))
        wait_for(lambda: call("focus.get")["named"] == ["task-form-description"],
                 description="the form opens again")
        typed(" now")
        key("Escape")
        wait_for(lambda: form(app) == {"open": False}, description="Escape closes it")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard back")
        assert call("editor.get")["cursor"] == caret
        assert call("editor.get")["text"] == expected

        # Typed straight after the key, in one go: whatever comes before the
        # form has the keyboard is dropped, never typed into the note.
        xdo("key", "--window", window, "--clearmodifiers", "--delay", "0",
            native_mod("shift+Return"), "z", "z", "z")
        wait_for(lambda: form(app)["open"], description="the form opens")
        time.sleep(0.3)
        assert call("editor.get")["text"] == expected
        assert "zzz".endswith(form(app)["description"].removeprefix("Owed by @sam"))
        key("Escape")
        wait_for(lambda: form(app) == {"open": False}, description="Escape closes it")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)
