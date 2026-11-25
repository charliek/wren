"""Task suggestions (discovery/tasks-build.md item 5.1; discovery/tasks.md §6
item 2, §3.1 Q8, §3.2 Q12): typing on a task line offers "📅 Due date…",
which opens the calendar, a date word or a priority written at once, and
after an ``@`` the owners already used in the workspace. Tab chooses; Enter
chooses only what cannot rewrite ordinary writing ("Due date…" for ``du``,
which writes nothing until a date is picked) or a row picked with Up or
Down, and otherwise continues the list as it always has. Escape hides them for the
line. They are the advanced engine's, with ``tasks`` and ``tasks-suggest``
on: on the classic engine, with ``tasks = off`` or with
``tasks-suggest = off``, nothing is offered, Tab indents and Enter
continues the list.

``editor.key`` presses keys one at a time; the ``native_x11`` cases type
with xdotool at full speed, so a word and its Enter arrive together. Each
case writes its own settings and the engine ``WRENTEST_ENGINE`` names, pins
today to Thursday 19 November 2026, and asserts the note's bytes on disk.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
from pathlib import Path
import time

import pytest

from tools.wrentest.test_editor import engine, focus_native, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_baseline import saved
from tools.wrentest.ui import WrenApp, wait_for

TODAY = "2026-11-19"
NOTE = "Tasks/Suggest/Suggest.md"
MEETING = "Tasks/Suggest/Meeting.md"
LINES = [
    "# Suggest",
    "",
    "- [ ] Meet sam",
    "",
    "Plain text.",
]
ORIGINAL = "\n".join(LINES)
TASK = 2
MEETING_TEXT = "\n".join([
    "# Meeting",
    "",
    "- [ ] Send the notes @priya.k",
    "- [ ] Book the room @Sam",
    "- [x] Order lunch @Sam",
    "Mail @prose about it.",
    "",
])


@contextmanager
def suggest_app(tmp_path: Path, engine: str,  # noqa: F811
                settings: str = "tasks = on\n") -> Iterator[tuple[WrenApp, Path]]:
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    (workspace / MEETING).write_text(MEETING_TEXT, encoding="utf-8")
    name = "on" if "tasks = on" in settings and "tasks-suggest = off" not in settings else "off"
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\nspell-check = off\n{settings}", today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-suggest-{name}-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert app.client.call("session.get")["banner"] is None
        note = next(row["id"] for row in app.client.call("session.get")["notes"]
                    if row["path"] == NOTE)
        app.client.call("core.apply", {"op": "open_note", "note": note})
        wait_for(lambda: app.client.call("editor.get")["text"] == ORIGINAL,
                 description="the note opens")
        app.client.call("editor.focus")
        end_of_task(app)
        yield app, path


def end_of_task(app: WrenApp) -> None:
    app.client.call("editor.cursor", {"line": TASK, "column": len(LINES[TASK].encode()),
                                      "selection": None})


def key(app: WrenApp, name: str) -> dict:
    app.client.call("editor.key", {"key": name})
    return app.client.call("editor.get")


def type_keys(app: WrenApp, text: str) -> dict:
    for character in text:
        app.client.call("editor.key", {"key": "space" if character == " " else character})
    return app.client.call("editor.get")


def labels(editor: dict) -> list[str]:
    popover = editor["popover"]
    if popover is None:
        return []
    assert popover["mode"] == "suggest", popover
    return [row["label"] for row in popover["rows"]]


def with_task(line: str, *after: str) -> str:
    return "\n".join(LINES[:TASK] + [line, *after] + LINES[TASK + 1:])


def owners_ready(app: WrenApp) -> None:
    wait_for(lambda: app.client.call("tasks.owners")["current"],
             description="the owners are read from the workspace")


def test_typing_offers_dates_and_priorities_and_tab_writes_one(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with suggest_app(tmp_path, engine) as (app, path):
        editor = type_keys(app, " to")
        if engine == "classic":
            # No popover on classic: Tab indents, as before.
            assert editor["popover"] is None
            editor = key(app, "tab")
            assert editor["text"] == with_task("- [ ] Meet sam to")
            return
        assert labels(editor) == ["Today", "Tomorrow"]
        assert editor["popover"]["text"] == "to"
        assert [row["glyph"] for row in editor["popover"]["rows"]] == ["📅", "📅"]
        editor = key(app, "tab")
        expected = with_task("- [ ] Meet sam 📅 2026-11-19")
        assert editor["text"] == expected
        assert editor["popover"] is None
        assert editor["cursor"] == {"line": TASK, "column": len(expected.split("\n")[TASK].encode())}
        saved(app, path, expected)
        # One undo step brings the word back.
        app.client.call("editor.undo")
        assert app.client.call("editor.get")["text"] == with_task("- [ ] Meet sam to")
        app.client.call("editor.redo")
        # A priority, picked with Down and Enter; only what the line lacks.
        editor = type_keys(app, " hi")
        assert labels(editor) == ["High priority", "Highest priority"]
        assert editor["popover"]["takes_enter"] is False
        editor = key(app, "down")
        assert editor["popover"]["takes_enter"] is True
        editor = key(app, "enter")
        expected = with_task("- [ ] Meet sam 🔺 📅 2026-11-19")
        assert editor["text"] == expected
        assert labels(type_keys(app, " to")) == []
        saved(app, path, with_task("- [ ] Meet sam 🔺 📅 2026-11-19 to"))


@pytest.mark.parametrize("word", ["to", "hi", "lo", "me", "fr", "sa", "@sa"])
def test_enter_after_an_ordinary_word_continues_the_list(
        tmp_path: Path, engine: str, word: str) -> None:  # noqa: F811
    """The Enter table: the word stays, and Enter makes the next task."""
    with suggest_app(tmp_path, engine) as (app, path):
        if engine == "advanced" and word.startswith("@"):
            owners_ready(app)
        editor = type_keys(app, f" {word}")
        assert (editor["popover"] is not None) is (engine == "advanced"), word
        if engine == "advanced":
            # Drawn as tentative, with the hint saying how to choose.
            assert editor["popover"]["footer"] == "Tab picks · ↑↓ ↵"
            assert editor["popover"]["takes_enter"] is False
        editor = key(app, "enter")
        expected = with_task(f"- [ ] Meet sam {word}", "- [ ] ")
        assert editor["text"] == expected
        assert editor["popover"] is None
        assert editor["cursor"] == {"line": TASK + 1, "column": 6}
        saved(app, path, expected)


def test_du_and_enter_open_the_calendar_which_writes_in_the_words_place(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with suggest_app(tmp_path, engine) as (app, path):
        call = app.client.call
        editor = type_keys(app, " du")
        if engine == "classic":
            assert editor["popover"] is None
            key(app, "enter")
            assert call("tasks.calendar") == {"open": False}
            return
        assert labels(editor) == ["Due date…"]
        key(app, "enter")
        state = call("tasks.calendar")
        assert state["open"] is True
        assert state["line"] == TASK
        # The word stays while it is open, and when it is cancelled.
        assert call("editor.get")["text"] == with_task("- [ ] Meet sam du")
        call("editor.key", {"key": "escape"})
        assert call("tasks.calendar") == {"open": False}
        assert call("editor.get")["text"] == with_task("- [ ] Meet sam du")
        # Again, and a date typed: the word goes with it, in one step.
        key(app, "backspace")
        type_keys(app, "u")
        key(app, "enter")
        assert call("tasks.calendar")["open"] is True
        type_keys(app, "fri")
        call("editor.key", {"key": "enter"})
        expected = with_task("- [ ] Meet sam 📅 2026-11-20")
        wait_for(lambda: call("editor.get")["text"] == expected, description="the date is written")
        assert call("tasks.calendar") == {"open": False}
        saved(app, path, expected)
        call("editor.undo")
        assert call("editor.get")["text"] == with_task("- [ ] Meet sam du")


def test_owners_come_from_the_workspace_and_complete_the_name(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with suggest_app(tmp_path, engine) as (app, path):
        call = app.client.call
        if engine == "classic":
            assert labels(type_keys(app, " @sa")) == []
            return
        owners_ready(app)
        found = call("tasks.owners")["owners"]
        # `@Sam` twice in one note is one owner of one note; prose is none.
        assert sorted((owner["name"], owner["notes"]) for owner in found) == [
            ("Sam", 1), ("priya.k", 1)]
        editor = type_keys(app, " @")
        assert sorted(labels(editor)) == ["@Sam", "@priya.k"]
        editor = type_keys(app, "pr")
        assert labels(editor) == ["@priya.k"]
        editor = key(app, "tab")
        expected = with_task("- [ ] Meet sam @priya.k")
        assert editor["text"] == expected
        saved(app, path, expected)
        # A name written here is offered on the next task, once read.
        key(app, "enter")
        type_keys(app, "Call @zo")
        type_keys(app, "e")
        assert labels(app.client.call("editor.get")) == []
        key(app, "enter")
        wait_for(lambda: any(owner["name"] == "zoe" for owner in call("tasks.owners")["owners"]),
                 description="the new owner is read")
        owners_ready(app)
        editor = type_keys(app, "Ask @z")
        assert labels(editor) == ["@zoe"]


def test_escape_hides_them_for_the_rest_of_the_line(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with suggest_app(tmp_path, engine) as (app, path):
        if engine == "classic":
            pytest.skip("suggestions are the advanced engine's")
        assert labels(type_keys(app, " to")) == ["Today", "Tomorrow"]
        editor = key(app, "escape")
        assert editor["popover"] is None
        assert labels(type_keys(app, " hi")) == []
        # The next line offers them again.
        key(app, "enter")
        assert labels(type_keys(app, "Call lo")) == ["Low priority", "Lowest priority"]


@pytest.mark.parametrize(("word", "move"), [("to", "up"), ("fr", "down"), ("@sa", "up")])
def test_a_move_that_moves_nothing_leaves_enter_to_the_list(
        tmp_path: Path, engine: str, word: str, move: str) -> None:  # noqa: F811
    """The Enter table's edges: Up on the first row, or Down on the only
    one, picks nothing, and Enter still continues the list."""
    if engine == "classic":
        pytest.skip("suggestions are the advanced engine's")
    with suggest_app(tmp_path, engine) as (app, path):
        if word.startswith("@"):
            owners_ready(app)
        editor = type_keys(app, f" {word}")
        assert editor["popover"] is not None, word
        editor = key(app, move)
        assert editor["popover"]["takes_enter"] is False, editor["popover"]
        editor = key(app, "enter")
        expected = with_task(f"- [ ] Meet sam {word}", "- [ ] ")
        assert editor["text"] == expected
        saved(app, path, expected)


@pytest.mark.parametrize("settings", ["tasks = on\ntasks-suggest = off\n", "tasks = off\n"])
def test_with_the_setting_or_tasks_off_nothing_is_offered(
        tmp_path: Path, engine: str, settings: str) -> None:  # noqa: F811
    with suggest_app(tmp_path, engine, settings) as (app, path):
        assert labels(type_keys(app, " du")) == []
        editor = key(app, "enter")
        assert editor["text"] == with_task("- [ ] Meet sam du", "- [ ] ")
        if "tasks = on" in settings:
            assert app.client.call("tasks.calendar") == {"open": False}
        editor = type_keys(app, "Call hi")
        assert labels(editor) == []
        # Tab indents the new item under the one before.
        editor = key(app, "tab")
        expected = with_task("- [ ] Meet sam du", "  - [ ] Call hi")
        assert editor["text"] == expected
        saved(app, path, expected)


@pytest.mark.native_x11
def test_native_du_and_enter_at_full_speed_open_the_calendar(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with suggest_app(tmp_path, engine) as (app, path):
        call = app.client.call
        window = focus_native(app)
        end_of_task(app)
        # `du` and Return in one go: the suggestions they open in that
        # batch take the Return, and what follows is dropped, never typed.
        # On the advanced engine, Escape in that batch too: typed before the
        # calendar could be seen, it closes nothing.
        escape = [] if engine == "classic" else ["Escape"]
        xdo("key", "--window", window, "--clearmodifiers", "--delay", "0",
            "space", "d", "u", "Return", *escape, "x", "y")
        if engine == "classic":
            expected = with_task("- [ ] Meet sam du", "- [ ] xy")
            wait_for(lambda: call("editor.get")["text"] == expected,
                     description="classic types it all")
            assert call("tasks.calendar") == {"open": False}
            return
        wait_for(lambda: call("tasks.calendar")["open"], description="du⏎ opens the calendar")
        wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                 description="the field takes the keyboard")
        time.sleep(0.3)
        assert call("tasks.calendar")["open"] is True
        assert call("focus.get")["named"] == ["calendar-field"]
        assert call("editor.get")["text"] == with_task("- [ ] Meet sam du")
        assert call("tasks.calendar")["typed"] == ""
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "fri")
        wait_for(lambda: call("tasks.calendar")["typed"] == "fri", description="typing reaches the field")
        xdo("key", "--window", window, "--clearmodifiers", "Return")
        expected = with_task("- [ ] Meet sam 📅 2026-11-20")
        wait_for(lambda: call("editor.get")["text"] == expected, description="Enter writes the date")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard back")
        saved(app, path, expected)


@pytest.mark.native_x11
def test_native_to_and_enter_at_full_speed_continue_the_list(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with suggest_app(tmp_path, engine) as (app, path):
        call = app.client.call
        window = focus_native(app)
        end_of_task(app)
        xdo("key", "--window", window, "--clearmodifiers", "--delay", "0",
            "space", "t", "o", "Return", "n", "e", "x", "t")
        expected = with_task("- [ ] Meet sam to", "- [ ] next")
        wait_for(lambda: call("editor.get")["text"] == expected,
                 description="Enter continues the list and the typing follows")
        assert call("tasks.calendar") == {"open": False}
        saved(app, path, expected)
