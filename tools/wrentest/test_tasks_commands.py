"""The task line commands (discovery/tasks-build.md item 3.3; discovery/tasks.md
§5.1, §5.3, §5.7 and §6): ``task.cycle_status``, ``task.priority.*`` (one
per level, then raise and lower), ``task.clear_due`` and
``markdown.toggle_task`` with its done and cancelled stamps. Each rewrites
the caret's task line, or for the toggle every task a selection covers, as
one undo step, on both engines. With ``tasks = off`` none is listed or runs,
and the toggle writes no stamp.

Each case writes its own ``tasks`` line and the engine ``WRENTEST_ENGINE``
names, whatever the lane's ``WRENTEST_TASKS`` says, pins today to Thursday
19 November 2026, and asserts the note's bytes on disk.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
from pathlib import Path

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import engine, focus_native, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_baseline import saved
from tools.wrentest.ui import WrenApp, wait_for

TODAY = "2026-11-19"
NOTE = "Tasks/Commands/Commands.md"
LINES = [
    "# Commands",
    "",
    "- [ ] Open alpha @sam 📅 2026-11-23",
    "- [/] Doing bravo ⏫",
    "- [x] Done charlie ✅ 2026-11-18",
    "- [-] Cancelled delta ❌ 2026-11-17",
    "- [?] Unknown echo",
    "- [ ] Selector foxtrot ⏫️ 📅 2026-12-01",
    "- [ ] Alias golf 🗓 2026-12-02 ^abc",
    "",
    "Plain hotel.",
]
ORIGINAL = "\n".join(LINES)

# The Tasks group's rows, in the palette's order (design Q7: High, Medium
# and Low first), with the status and priority menus (item 3.4).
TASK_COMMANDS = [
    "task.set_status",
    "task.cycle_status",
    "task.set_priority",
    "task.priority.high",
    "task.priority.medium",
    "task.priority.low",
    "task.priority.highest",
    "task.priority.lowest",
    "task.priority.none",
    "task.priority.raise",
    "task.priority.lower",
    "task.set_due",
    "task.clear_due",
]
# The commands that open a menu (item 3.4) or the calendar (item 4.1),
# which the classic engine has not.
MENU_COMMANDS = {"task.set_status", "task.set_priority", "task.set_due"}


def width(text: str) -> int:
    """A column as the editor counts it: UTF-8 bytes."""
    return len(text.encode("utf-8"))


def line_of(start: str) -> int:
    return next(index for index, text in enumerate(LINES) if text.startswith(start))


def replaced(line: int, new: str) -> str:
    return "\n".join(LINES[:line] + [new] + LINES[line + 1:])


@contextmanager
def commands_app(tmp_path: Path, engine: str, tasks: str,  # noqa: F811
                 settings: str = "") -> Iterator[tuple[WrenApp, Path]]:
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\ntasks = {tasks}\n{settings}", today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-commands-{tasks}-{engine}")) as app:
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


def run(app: WrenApp, command: str) -> dict:
    return app.client.call("command.run", {"id": command})


def at_end(app: WrenApp, line: int) -> None:
    app.client.call("editor.cursor", {"line": line, "column": width(LINES[line]),
                                      "selection": None})


def undo_to_original(app: WrenApp, path: Path) -> None:
    app.client.call("editor.undo")
    state = app.client.call("editor.get")
    assert state["text"] == ORIGINAL, "one undo step"
    assert state["can_undo"] is False, "one undo step and no more"
    saved(app, path, ORIGINAL)


def tasks_rows(app: WrenApp, query: str = "") -> list[tuple[str, bool]]:
    """The palette's Tasks group: each row's command and whether it runs."""
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": query})["navigation"]["modal"]
    app.client.call("palette.close")
    return [(row["key"]["command"], row["enabled"]) for row in modal["results"]
            if row["group"] == "Tasks"]


# (line's start, command, the line after it)
CASES = [
    ("- [ ] Open", "task.cycle_status", "- [/] Open alpha @sam 📅 2026-11-23"),
    ("- [/] Doing", "task.cycle_status", "- [x] Doing bravo ⏫ ✅ 2026-11-19"),
    ("- [x] Done", "task.cycle_status", "- [ ] Done charlie"),
    ("- [-] Cancelled", "task.cycle_status", "- [ ] Cancelled delta"),
    ("- [?] Unknown", "task.cycle_status", "- [/] Unknown echo"),
    ("- [ ] Open", "task.priority.high", "- [ ] Open alpha @sam ⏫ 📅 2026-11-23"),
    ("- [/] Doing", "task.priority.medium", "- [/] Doing bravo 🔼"),
    ("- [x] Done", "task.priority.low", "- [x] Done charlie 🔽 ✅ 2026-11-18"),
    ("- [?] Unknown", "task.priority.highest", "- [?] Unknown echo 🔺"),
    ("- [-] Cancelled", "task.priority.lowest", "- [-] Cancelled delta ⏬ ❌ 2026-11-17"),
    ("- [/] Doing", "task.priority.none", "- [/] Doing bravo"),
    ("- [ ] Selector", "task.priority.low", "- [ ] Selector foxtrot 🔽 📅 2026-12-01"),
    ("- [/] Doing", "task.priority.raise", "- [/] Doing bravo 🔺"),
    ("- [/] Doing", "task.priority.lower", "- [/] Doing bravo 🔼"),
    ("- [ ] Selector", "task.priority.raise", "- [ ] Selector foxtrot 🔺 📅 2026-12-01"),
    ("- [ ] Open", "task.priority.lower", "- [ ] Open alpha @sam 🔽 📅 2026-11-23"),
    ("- [ ] Open", "task.clear_due", "- [ ] Open alpha @sam"),
    ("- [ ] Selector", "task.clear_due", "- [ ] Selector foxtrot ⏫️"),
    ("- [ ] Alias", "task.clear_due", "- [ ] Alias golf ^abc"),
]


def test_each_line_command_rewrites_the_caret_line(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with commands_app(tmp_path, engine, "on") as (app, path):
        call = app.client.call
        for start, command, after in CASES:
            line = line_of(start)
            case = f"{command} on {LINES[line]!r}"
            at_end(app, line)
            result = run(app, command)
            assert result["disposition"] == "completed", (case, result)
            expected = replaced(line, after)
            state = call("editor.get")
            assert state["text"] == expected, case
            # The caret stays on its line, before what was written at the end.
            assert state["cursor"]["line"] == line, case
            assert state["cursor"]["column"] <= width(after), case
            saved(app, path, expected)
            undo_to_original(app, path)

        # In the description, the caret keeps its character.
        doing = line_of("- [/] Doing")
        call("editor.cursor", {"line": doing, "column": width("- [/] Do"), "selection": None})
        run(app, "task.cycle_status")
        assert call("editor.get")["cursor"] == {"line": doing, "column": width("- [x] Do")}
        undo_to_original(app, path)

        # Nothing to change: the command completes and writes nothing.
        done = line_of("- [x] Done")
        at_end(app, done)
        assert run(app, "task.clear_due")["disposition"] == "completed"
        state = call("editor.get")
        assert state["text"] == ORIGINAL and state["can_undo"] is False


def test_off_a_task_line_the_commands_are_listed_greyed_and_do_nothing(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with commands_app(tmp_path, engine, "on") as (app, path):
        call = app.client.call
        call("editor.cursor", {"line": line_of("- [ ] Open"), "column": 0, "selection": None})
        assert tasks_rows(app) == [(command, engine == "advanced" or command not in MENU_COMMANDS)
                                   for command in TASK_COMMANDS]
        for line in [line_of("# Commands"), 1, line_of("Plain hotel.")]:
            call("editor.cursor", {"line": line, "column": 0, "selection": None})
            assert tasks_rows(app) == [(command, False) for command in TASK_COMMANDS], line
            for command in TASK_COMMANDS:
                assert run(app, command)["disposition"] == "blocked", (command, line)
            assert call("editor.get")["text"] == ORIGINAL
        saved(app, path, ORIGINAL)


def test_the_toggle_stamps_from_the_palette_and_its_key(tmp_path: Path, engine: str) -> None:  # noqa: F811
    toggle = ["logo"] if os.uname().sysname == "Darwin" else ["ctrl"]
    with commands_app(tmp_path, engine, "on") as (app, path):
        call = app.client.call
        for start, after in [
            ("- [ ] Open", "- [x] Open alpha @sam 📅 2026-11-23 ✅ 2026-11-19"),
            ("- [/] Doing", "- [x] Doing bravo ⏫ ✅ 2026-11-19"),
            ("- [x] Done", "- [ ] Done charlie"),
            ("- [-] Cancelled", "- [ ] Cancelled delta"),
            ("- [?] Unknown", "- [x] Unknown echo ✅ 2026-11-19"),
        ]:
            line = line_of(start)
            for how in ["palette", "key"]:
                at_end(app, line)
                if how == "palette":
                    assert run(app, "markdown.toggle_task")["disposition"] == "completed"
                else:
                    call("editor.key", {"key": "enter", "mods": toggle})
                expected = replaced(line, after)
                assert call("editor.get")["text"] == expected, (how, start)
                saved(app, path, expected)
                undo_to_original(app, path)

        # Over a selection: an unfinished task makes every task done, the
        # done one keeping its stamp, as one undo step.
        first, last = line_of("- [ ] Open"), line_of("- [?] Unknown")
        call("editor.cursor", {"line": last, "column": width(LINES[last]),
                               "selection": {"line": first, "column": 0}})
        assert run(app, "markdown.toggle_task")["disposition"] == "completed"
        expected = "\n".join(
            LINES[:first]
            + ["- [x] Open alpha @sam 📅 2026-11-23 ✅ 2026-11-19",
               "- [x] Doing bravo ⏫ ✅ 2026-11-19",
               "- [x] Done charlie ✅ 2026-11-18",
               "- [x] Cancelled delta ✅ 2026-11-19",
               "- [x] Unknown echo ✅ 2026-11-19"]
            + LINES[last + 1:])
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)
        undo_to_original(app, path)


def test_set_today_dates_the_stamps(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with commands_app(tmp_path, engine, "on") as (app, path):
        call = app.client.call
        call("tasks.set_today", {"date": "2026-11-20"})
        line = line_of("- [/] Doing")
        at_end(app, line)
        run(app, "task.cycle_status")
        expected = replaced(line, "- [x] Doing bravo ⏫ ✅ 2026-11-20")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)
        # A stamp there is re-dated: done → open → done on a later day.
        run(app, "markdown.toggle_task")
        call("tasks.set_today", {"date": "2026-11-21"})
        run(app, "markdown.toggle_task")
        expected = replaced(line, "- [x] Doing bravo ⏫ ✅ 2026-11-21")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)


def test_a_bound_task_command_runs_from_its_key(tmp_path: Path, engine: str) -> None:  # noqa: F811
    mods = ["logo", "shift"] if os.uname().sysname == "Darwin" else ["alt", "shift"]
    trigger = "super+shift+u" if os.uname().sysname == "Darwin" else "alt+shift+u"
    settings = f"keybind = {trigger} = task.priority.raise\n"
    with commands_app(tmp_path, engine, "on", settings) as (app, path):
        call = app.client.call
        assert call("session.get")["banner"] is None
        line = line_of("- [ ] Open")
        at_end(app, line)
        call("shortcut", {"id": "task.priority.raise"})
        expected = replaced(line, "- [ ] Open alpha @sam 🔼 📅 2026-11-23")
        assert call("editor.get")["text"] == expected
        call("editor.key", {"key": "u", "mods": mods})
        expected = replaced(line, "- [ ] Open alpha @sam ⏫ 📅 2026-11-23")
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)


def test_a_menu_of_command_rows_takes_its_keys(tmp_path: Path, engine: str) -> None:  # noqa: F811
    rows = {"rows": [
        {"command": "task.priority.high", "label": "High", "key": "h"},
        {"command": "task.priority.medium", "label": "Medium", "key": "m"},
        {"command": "task.priority.highest", "label": "Highest", "key": "h", "shift": True},
    ]}
    with commands_app(tmp_path, engine, "on") as (app, path):
        call = app.client.call
        line = line_of("- [ ] Open")
        at_end(app, line)
        if engine == "classic":
            # The classic engine draws no popover of the app's.
            with pytest.raises(RemoteError, match="no menu"):
                call("editor.open_menu", rows)
            return
        popover = call("editor.open_menu", rows)["popover"]
        assert popover["mode"] == "menu" and popover["spelling"] is False
        assert [row["command"] for row in popover["rows"]] == [
            "task.priority.high", "task.priority.medium", "task.priority.highest"]
        state = call("editor.key", {"key": "h", "mods": ["shift"]})
        assert state["popover"] is None
        expected = replaced(line, "- [ ] Open alpha @sam 🔺 📅 2026-11-23")
        assert state["text"] == expected
        saved(app, path, expected)
        # Down and Enter choose a row; Escape closes the menu and changes
        # nothing; another key closes it and types.
        call("editor.open_menu", rows)
        call("editor.key", {"key": "down"})
        assert call("editor.get")["popover"]["selected"] == 1
        state = call("editor.key", {"key": "enter"})
        expected = replaced(line, "- [ ] Open alpha @sam 🔼 📅 2026-11-23")
        assert state["text"] == expected and state["popover"] is None
        call("editor.open_menu", rows)
        state = call("editor.key", {"key": "escape"})
        assert state["text"] == expected and state["popover"] is None
        saved(app, path, expected)


def test_with_tasks_off_no_task_command_runs_and_the_toggle_writes_no_stamp(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    toggle = ["logo"] if os.uname().sysname == "Darwin" else ["ctrl"]
    with commands_app(tmp_path, engine, "off") as (app, path):
        call = app.client.call
        line = line_of("- [ ] Open")
        at_end(app, line)
        assert tasks_rows(app) == []
        assert tasks_rows(app, "task") == []
        for command in TASK_COMMANDS:
            assert run(app, command)["disposition"] == "blocked", command
        assert call("editor.get")["text"] == ORIGINAL
        expected = replaced(line, "- [x] Open alpha @sam 📅 2026-11-23")
        assert run(app, "markdown.toggle_task")["disposition"] == "completed"
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)
        call("editor.undo")
        at_end(app, line)
        call("editor.key", {"key": "enter", "mods": toggle})
        assert call("editor.get")["text"] == expected
        saved(app, path, expected)


@pytest.mark.native_x11
def test_native_ctrl_enter_toggles_with_a_stamp(tmp_path: Path, engine: str) -> None:  # noqa: F811
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with commands_app(tmp_path, engine, "on") as (app, path):
        call = app.client.call
        line = line_of("- [/] Doing")
        at_end(app, line)
        window = focus_native(app)
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+Return")
        expected = replaced(line, "- [x] Doing bravo ⏫ ✅ 2026-11-19")
        wait_for(lambda: call("editor.get")["text"] == expected,
                 description="Ctrl+Enter checks the task with a stamp")
        saved(app, path, expected)
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+Return")
        wait_for(lambda: call("editor.get")["text"] == replaced(line, "- [ ] Doing bravo ⏫"),
                 description="Ctrl+Enter reopens it without the stamp")
