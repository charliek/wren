"""The calendar (discovery/tasks-build.md item 4.1; discovery/tasks.md §6,
"Calendar popover"): ``task.set_due`` (⌘D / Ctrl+D) opens a calendar at the
caret's task, on its due date or today, whose field takes ``fri``, ``+2w``
or an ISO date; Enter writes or replaces the line's ``📅`` token as one undo
step, Clear removes it, and Escape, a click outside or the field losing the
keyboard close it writing nothing. It is advanced-only: on classic the
command is greyed and its key does nothing, and with ``tasks = off`` it does
not exist.

While it is open, ``editor.key`` presses keys in its field. Each case writes
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

from tools.wrentest import test_gestures as gestures
from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import engine, focus_native, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_baseline import saved
from tools.wrentest.ui import WrenApp, wait_for

TODAY = "2026-11-19"
NOTE = "Tasks/Calendar/Calendar.md"
LINES = [
    "# Calendar",
    "",
    "- [ ] Dated alpha @sam 📅 2026-11-23",
    "- [ ] Undated bravo",
    "- [ ] Alias charlie 🗓 2026-12-02 ^abc",
    "",
    "Plain delta.",
]
ORIGINAL = "\n".join(LINES)
MAC = os.uname().sysname == "Darwin"


def width(text: str) -> int:
    """A column as the editor counts it: UTF-8 bytes."""
    return len(text.encode("utf-8"))


def line_of(start: str) -> int:
    return next(index for index, text in enumerate(LINES) if text.startswith(start))


def replaced(line: int, new: str) -> str:
    return "\n".join(LINES[:line] + [new] + LINES[line + 1:])


@contextmanager
def calendar_app(tmp_path: Path, engine: str, tasks: str = "on",  # noqa: F811
                 ) -> Iterator[tuple[WrenApp, Path]]:
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\ntasks = {tasks}\n", today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-calendar-{tasks}-{engine}")) as app:
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


def calendar(app: WrenApp) -> dict:
    return app.client.call("tasks.calendar")


def set_due(app: WrenApp) -> dict:
    """``task.set_due``'s key, as the focused editor routes it."""
    app.client.call("shortcut", {"id": "task.set_due"})
    return calendar(app)


def type_keys(app: WrenApp, text: str) -> None:
    for character in text:
        app.client.call("editor.key", {"key": character if character != " " else "space"})


def tasks_rows(app: WrenApp) -> dict[str, bool]:
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": ""})["navigation"]["modal"]
    app.client.call("palette.close")
    return {row["key"]["command"]: row["enabled"] for row in modal["results"]
            if row["group"] == "Tasks"}


def test_typing_fri_and_enter_writes_friday(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with calendar_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Undated")
        cursor = caret_at(app, line)
        if engine == "classic":
            # No popover on classic: the command is greyed and its key
            # opens nothing and changes nothing.
            assert tasks_rows(app)["task.set_due"] is False
            assert set_due(app) == {"open": False}
            assert call("command.run", {"id": "task.set_due"})["disposition"] == "blocked"
            assert call("editor.get")["text"] == ORIGINAL
            return
        assert tasks_rows(app)["task.set_due"] is True
        state = set_due(app)
        assert state["open"] is True
        assert (state["line"], state["selected"], state["typed"]) == (line, TODAY, "")
        assert state["clear"] is False
        assert state["caption"] == "Thursday 19 November 2026 · today"
        # The field has the keyboard, once the toolkit has moved it.
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="the field takes the keyboard")
        type_keys(app, "fri")
        state = calendar(app)
        assert (state["typed"], state["selected"], state["invalid"]) == ("fri", "2026-11-20", False)
        assert state["caption"] == "Friday 20 November 2026 · tomorrow"
        call("editor.key", {"key": "enter"})
        assert calendar(app) == {"open": False}
        expected = replaced(line, "- [ ] Undated bravo 📅 2026-11-20")
        editor = call("editor.get")
        assert editor["text"] == expected
        assert editor["cursor"] == cursor
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard back")
        saved(app, path, expected)
        call("editor.undo")
        editor = call("editor.get")
        assert editor["text"] == ORIGINAL and editor["can_undo"] is False, "one undo step"
        saved(app, path, ORIGINAL)


def test_plus_two_weeks_replaces_the_date_and_keys_move_the_day(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    if engine == "classic":
        pytest.skip("the calendar is advanced-only (the classic case is in the first test)")
    with calendar_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Dated")
        caret_at(app, line, width("- [ ] Dated"))
        state = set_due(app)
        assert (state["selected"], state["current"], state["clear"]) == ("2026-11-23", "2026-11-23", True)
        assert state["anchor"] == {"line": line, "range": [width("- [ ] Dated alpha @sam "),
                                                          width(LINES[line])]}
        type_keys(app, "+2w")
        assert calendar(app)["selected"] == "2026-12-03"
        call("editor.key", {"key": "enter"})
        expected = replaced(line, "- [ ] Dated alpha @sam 📅 2026-12-03")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)

        # Arrows: a week, a day while the field is empty, a month.
        set_due(app)
        for key, selected in [("down", "2026-12-10"), ("right", "2026-12-11"),
                              ("up", "2026-12-04"), ("left", "2026-12-03"),
                              ("page_down", "2027-01-03"), ("page_up", "2026-12-03")]:
            call("editor.key", {"key": key})
            assert calendar(app)["selected"] == selected, key
        # With text in the field, Left and Right are its own.
        type_keys(app, "tom")
        call("editor.key", {"key": "left"})
        assert calendar(app)["typed"] == "tom"
        type_keys(app, "orrow")
        assert calendar(app)["selected"] == "2026-11-20"
        call("editor.key", {"key": "down"})
        state = calendar(app)
        assert (state["typed"], state["selected"]) == ("", "2026-11-27")
        call("editor.key", {"key": "enter"})
        expected = replaced(line, "- [ ] Dated alpha @sam 📅 2026-11-27")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)

        # An alias keeps its glyph and its block id.
        alias = line_of("- [ ] Alias")
        caret_at(app, alias, 2)
        set_due(app)
        call("tasks.calendar_click", {"chip": "Fri"})
        assert call("editor.get")["text"].split("\n")[alias] == "- [ ] Alias charlie 🗓 2026-11-20 ^abc"


def test_an_invalid_word_writes_nothing_and_escape_cancels(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    if engine == "classic":
        pytest.skip("the calendar is advanced-only (the classic case is in the first test)")
    with calendar_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Dated")
        call("editor.cursor", {"line": line, "column": 9, "selection": {"line": line, "column": 6}})
        before = call("editor.get")["cursor"]
        set_due(app)
        for word, caption in [("blorp", "Not a date. Try fri, +3d or 2026-12-01."),
                              ("2026-02-30", "2026-02-30 is not a real date")]:
            call("tasks.calendar_type", {"text": word})
            state = calendar(app)
            assert (state["invalid"], state["caption"]) == (True, caption), word
            # The selection stays where it was.
            assert state["selected"] == "2026-11-23", word
            call("editor.key", {"key": "enter"})
            assert calendar(app)["open"] is True, word
            assert call("editor.get")["text"] == ORIGINAL, word
        call("editor.key", {"key": "escape"})
        assert calendar(app) == {"open": False}
        editor = call("editor.get")
        assert editor["text"] == ORIGINAL and editor["can_undo"] is False
        assert editor["cursor"] == before
        assert editor["selected_text"] == "Dat"
        # Escape was the calendar's alone: nothing else closed.
        assert editor["popover"] is None
        saved(app, path, ORIGINAL)

        # A click outside the card, and the window losing the keyboard,
        # close it too, writing nothing.
        for close in [lambda: call("tasks.calendar_click", {"target": "outside"}),
                      lambda: call("window.blur")]:
            set_due(app)
            wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                     description="the field takes the keyboard")
            type_keys(app, "fri")
            close()
            wait_for(lambda: calendar(app) == {"open": False}, description="it closes")
            assert call("editor.get")["text"] == ORIGINAL


def test_clear_removes_the_due_date(tmp_path: Path, engine: str) -> None:  # noqa: F811
    if engine == "classic":
        pytest.skip("the calendar is advanced-only (the classic case is in the first test)")
    with calendar_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Dated")
        caret_at(app, line)
        set_due(app)
        call("tasks.calendar_click", {"target": "clear"})
        expected = replaced(line, "- [ ] Dated alpha @sam")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)
        call("editor.undo")
        assert call("editor.get")["text"] == ORIGINAL
        # A task with no due date shows no Clear.
        caret_at(app, line_of("- [ ] Undated"))
        set_due(app)
        with pytest.raises(RemoteError, match="Clear is not shown"):
            call("tasks.calendar_click", {"target": "clear"})
        # A click on a day writes it.
        call("tasks.calendar_click", {"day": "2026-11-30"})
        assert call("editor.get")["text"] == replaced(line_of("- [ ] Undated"),
                                                     "- [ ] Undated bravo 📅 2026-11-30")


def test_a_right_click_on_a_due_date_opens_it(tmp_path: Path, engine: str) -> None:  # noqa: F811
    if engine == "classic":
        pytest.skip("the calendar is advanced-only (the classic case is in the first test)")
    with calendar_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Dated")
        cursor = caret_at(app, line_of("- [ ] Undated"), 3)
        column = width("- [ ] Dated alpha @sam 📅 2026-")
        call("editor.right_click", {"line": line, "column": column})
        state = calendar(app)
        assert state["open"] is True
        assert (state["line"], state["selected"]) == (line, "2026-11-23")
        assert state["anchor"]["range"] == [width("- [ ] Dated alpha @sam "), width(LINES[line])]
        assert call("editor.get")["popover"] is None
        type_keys(app, "next fri")
        call("editor.key", {"key": "enter"})
        expected = replaced(line, "- [ ] Dated alpha @sam 📅 2026-11-27")
        editor = call("editor.get")
        assert editor["text"] == expected
        assert editor["cursor"] == cursor
        saved(app, path, expected)


def test_off_a_task_line_the_key_claims_nothing(tmp_path: Path, engine: str) -> None:  # noqa: F811
    if engine == "classic":
        pytest.skip("the calendar is advanced-only (the classic case is in the first test)")
    with calendar_app(tmp_path, engine) as (app, path):
        call = app.client.call
        plain = line_of("Plain delta.")
        caret_at(app, plain, 0)
        call("editor.key", {"key": "d", "mods": ["logo"] if MAC else ["ctrl"]})
        assert calendar(app) == {"open": False}
        call("editor.key", {"key": "x"})
        expected = replaced(plain, "xPlain delta.")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)


def test_with_tasks_off_there_is_no_calendar(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with calendar_app(tmp_path, engine, "off") as (app, path):
        call = app.client.call
        caret_at(app, line_of("- [ ] Undated"))
        assert "task.set_due" not in tasks_rows(app)
        with pytest.raises(RemoteError, match="unavailable"):
            call("shortcut", {"id": "task.set_due"})
        assert call("command.run", {"id": "task.set_due"})["disposition"] == "blocked"
        # Ctrl+D (⌘D) is unbound, and does what it did before tasks.
        call("editor.key", {"key": "d", "mods": ["logo"] if MAC else ["ctrl"]})
        assert call("editor.get")["text"] == ORIGINAL
        with pytest.raises(RemoteError, match="Tasks are off"):
            call("tasks.calendar_type", {"text": "fri"})
        saved(app, path, ORIGINAL)


@pytest.mark.native_x11
def test_native_ctrl_d_opens_it_and_typing_reaches_the_field(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with calendar_app(tmp_path, engine) as (app, path):
        call = app.client.call
        line = line_of("- [ ] Undated")
        cursor = caret_at(app, line)
        window = focus_native(app)
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+d")
        if engine == "classic":
            time.sleep(0.5)
            assert calendar(app) == {"open": False}
            assert call("editor.get")["text"] == ORIGINAL
            return
        wait_for(lambda: calendar(app)["open"], description="Ctrl+D opens the calendar")
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="the field takes the keyboard")
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "fri")
        wait_for(lambda: calendar(app)["typed"] == "fri", description="typing reaches the field")
        assert calendar(app)["selected"] == "2026-11-20"
        assert call("editor.get")["text"] == ORIGINAL
        xdo("key", "--window", window, "--clearmodifiers", "Down")
        wait_for(lambda: calendar(app)["selected"] == "2026-11-27", description="Down moves a week")
        xdo("key", "--window", window, "--clearmodifiers", "Return")
        expected = replaced(line, "- [ ] Undated bravo 📅 2026-11-27")
        wait_for(lambda: call("editor.get")["text"] == expected, description="Enter writes")
        assert calendar(app) == {"open": False}
        assert call("editor.get")["cursor"] == cursor
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard back")
        saved(app, path, expected)

        # Escape closes it and nothing else.
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+d")
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="the field takes the keyboard again")
        xdo("key", "--window", window, "--clearmodifiers", "Escape")
        wait_for(lambda: calendar(app) == {"open": False}, description="Escape closes it")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard back")
        assert call("editor.get")["text"] == expected

        # The wheel over the card closes it too, writing nothing.
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+d")
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="the field takes the keyboard again")
        card = calendar(app)["bounds"]
        gestures.move(window, (int(card["x"] + card["width"] / 2),
                               int(card["y"] + card["height"] / 2)))
        xdo("click", "5")
        wait_for(lambda: calendar(app) == {"open": False}, description="a scroll closes it")
        assert call("editor.get")["text"] == expected
        call("editor.focus")

        # Typed straight after Ctrl+D, in one go: whatever comes before the
        # field has the keyboard is dropped, never typed into the note.
        xdo("key", "--window", window, "--clearmodifiers", "--delay", "0",
            "ctrl+d", "t", "o", "d", "a", "y")
        wait_for(lambda: calendar(app)["open"], description="Ctrl+D opens the calendar")
        time.sleep(0.3)
        assert call("editor.get")["text"] == expected
        typed = calendar(app)["typed"]
        assert "today".endswith(typed), typed
        xdo("key", "--window", window, "--clearmodifiers", "Escape")
        wait_for(lambda: calendar(app) == {"open": False}, description="Escape closes it")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)
