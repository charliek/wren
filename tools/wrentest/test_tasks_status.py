"""Status in the box (discovery/tasks-build.md item 1.1; discovery/tasks.md
§5.1): with ``tasks = on`` a box holds any one character but ``]``, read
through the status table, and Enter, Backspace and ``markdown.toggle_task``
work from the box's byte range, whatever the character's width. With
``tasks = off`` only ``[ ]``, ``[x]`` and ``[X]`` are boxes, and ``[/]`` is
the item's text.

Each case writes its own ``tasks`` line and the engine ``WRENTEST_ENGINE``
names, whatever the lane's ``WRENTEST_TASKS`` says, and asserts the note's
bytes on disk.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from tools.wrentest.test_editor import engine  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_baseline import saved
from tools.wrentest.ui import WrenApp, wait_for

TODAY = "2026-11-19"
NOTE = "Tasks/Status/Statuses.md"

# (line text, its task as ``tasks.get`` reports it with tasks on: the box's
# byte range, its symbol, the status name and the status type)
STATUSES = [
    ("# Statuses", None),
    ("", None),
    ("- [ ] Open alpha", ((2, 5), " ", "Open", "open")),
    ("- [/] Doing bravo", ((2, 5), "/", "Doing", "doing")),
    ("- [x] Done charlie", ((2, 5), "x", "Done", "done")),
    ("- [X] Done delta", ((2, 5), "X", "Done", "done")),
    ("- [-] Cancelled echo", ((2, 5), "-", "Cancelled", "cancelled")),
    ("- [a] Unknown foxtrot", ((2, 5), "a", "Unknown", "open")),
    ("- [1] x", ((2, 5), "1", "Unknown", "open")),
    ("- [[] Bracket golf", ((2, 5), "[", "Unknown", "open")),
    ("- [é] Accent hotel", ((2, 6), "é", "Unknown", "open")),
    ("- [界] Wide india", ((2, 7), "界", "Unknown", "open")),
    ("- [✓] Check juliet", ((2, 7), "✓", "Unknown", "open")),
    ("> 1. [\U0001f600]\tQuoted kilo", ((5, 11), "\U0001f600", "Unknown", "open")),
    # Not boxes: two scalar values, a `]`, no space after, no list marker.
    ("- [✔\ufe0f] Two lima", None),
    ("- []] Bracket mike", None),
    ("- [a]: http://u", None),
    ("- [/]november", None),
    # An empty task, last in the list: the item after an empty one is in a
    # loose list to Enter, which is not what these cases are about.
    ("- [/]", ((2, 5), "/", "Doing", "doing")),
    ("[/] Bare oscar", None),
    ("plain papa", None),
]
LINES = [text for text, _ in STATUSES]
ORIGINAL = "\n".join(LINES)


def width(text: str) -> int:
    """A column as the editor counts it: UTF-8 bytes."""
    return len(text.encode("utf-8"))


def line_of(start: str) -> int:
    return next(index for index, text in enumerate(LINES) if text.startswith(start))


def replaced(line: int, *new: str) -> str:
    """The note with line *line* replaced by the lines *new*."""
    return "\n".join(LINES[:line] + list(new) + LINES[line + 1:])


@contextmanager
def status_app(tmp_path: Path, engine: str, tasks: str) -> Iterator[tuple[WrenApp, Path]]:  # noqa: F811
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\ntasks = {tasks}\n", today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-status-{tasks}-{engine}")) as app:
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


def act(app: WrenApp, line: int, column: int, action: str) -> str:
    """Put the caret at *line*:*column*, press the key or run the toggle,
    and return the editor's text."""
    call = app.client.call
    call("editor.cursor", {"line": line, "column": column, "selection": None})
    if action == "toggle":
        result = call("command.run", {"id": "markdown.toggle_task"})
        assert result["disposition"] == "completed", result
    else:
        call("editor.key", {"key": action, "mods": []})
    return call("editor.get")["text"]


def check(app: WrenApp, path: Path, cases: list[tuple[int, int, str, str]]) -> None:
    """Each case's bytes in the editor and on disk, and one undo back."""
    for line, column, action, expected in cases:
        case = f"{action} at {line}:{column} ({LINES[line]!r})"
        assert act(app, line, column, action) == expected, case
        saved(app, path, expected)
        if expected != ORIGINAL:
            app.client.call("editor.undo")
            assert app.client.call("editor.get")["text"] == ORIGINAL, case
            saved(app, path, ORIGINAL)


def test_the_caret_line_reports_its_box_symbol_and_status(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with status_app(tmp_path, engine, "on") as (app, _):
        call = app.client.call
        for number, (text, task) in enumerate(STATUSES):
            call("editor.cursor", {"line": number, "column": 0, "selection": None})
            expected = None if task is None else {
                "box": list(task[0]), "symbol": task[1], "status": task[2], "type": task[3]}
            assert call("tasks.get")["line"] == {
                "number": number, "text": text, "task": expected}, text
            if task is not None:
                start, end = task[0]
                assert text.encode("utf-8")[start:end].decode("utf-8") == f"[{task[1]}]"


def test_enter_backspace_and_toggle_work_from_the_box_range(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    doing, cancelled, wide = line_of("- [/] Doing"), line_of("- [-]"), line_of("- [界]")
    quoted, empty = line_of("> 1. ["), LINES.index("- [/]")
    end = {line: width(LINES[line]) for line in range(len(LINES))}
    with status_app(tmp_path, engine, "on") as (app, path):
        check(app, path, [
            # Enter at the end continues with an open box and the task's
            # own spacing.
            (doing, end[doing], "enter", replaced(doing, LINES[doing], "- [ ] ")),
            (cancelled, end[cancelled], "enter",
             replaced(cancelled, LINES[cancelled], "- [ ] ")),
            (wide, end[wide], "enter", replaced(wide, LINES[wide], "- [ ] ")),
            (quoted, end[quoted], "enter", replaced(quoted, LINES[quoted], "> 2. [ ]\t")),
            # Enter at the task's text puts an open task above it.
            (doing, 6, "enter", replaced(doing, "- [ ]", LINES[doing])),
            (wide, width("- [界] "), "enter", replaced(wide, "- [ ]", LINES[wide])),
            # Backspace at the text takes the whole box, however wide.
            (doing, 6, "backspace", replaced(doing, "- Doing bravo")),
            (cancelled, 6, "backspace", replaced(cancelled, "- Cancelled echo")),
            (wide, width("- [界] "), "backspace", replaced(wide, "- Wide india")),
            (quoted, width("> 1. [\U0001f600]\t"), "backspace",
             replaced(quoted, "> 1. Quoted kilo")),
            (empty, end[empty], "backspace", replaced(empty, "- ")),
            # Inside the box it takes one character.
            (wide, width("- [界"), "backspace", replaced(wide, "- [] Wide india")),
            # The toggle: unfinished becomes done, finished becomes open,
            # and all of the symbol's bytes are replaced.
            (line_of("- [ ] Open"), 0, "toggle", replaced(2, "- [x] Open alpha")),
            (doing, end[doing], "toggle", replaced(doing, "- [x] Doing bravo")),
            (line_of("- [x]"), 0, "toggle", replaced(4, "- [ ] Done charlie")),
            (line_of("- [X]"), 0, "toggle", replaced(5, "- [ ] Done delta")),
            (cancelled, end[cancelled], "toggle", replaced(cancelled, "- [ ] Cancelled echo")),
            (line_of("- [a] Unknown"), 0, "toggle", replaced(7, "- [x] Unknown foxtrot")),
            (line_of("- [1]"), 0, "toggle", replaced(8, "- [x] x")),
            (line_of("- [[]"), 0, "toggle", replaced(9, "- [x] Bracket golf")),
            (line_of("- [é]"), 0, "toggle", replaced(10, "- [x] Accent hotel")),
            (wide, end[wide], "toggle", replaced(wide, "- [x] Wide india")),
            (quoted, end[quoted], "toggle", replaced(quoted, "> 1. [x]\tQuoted kilo")),
            # Not boxes: Enter continues a plain item, and the toggle gives
            # the item a box of its own.
            (line_of("- [✔"), end[line_of("- [✔")], "enter",
             replaced(line_of("- [✔"), LINES[line_of("- [✔")], "- ")),
            (line_of("- []]"), 0, "toggle", replaced(line_of("- []]"), "- [ ] []] Bracket mike")),
            (line_of("- [a]:"), end[line_of("- [a]:")], "enter",
             replaced(line_of("- [a]:"), "- [a]: http://u", "- ")),
            (line_of("- [a]:"), 0, "toggle", replaced(line_of("- [a]:"), "- [ ] [a]: http://u")),
        ])
        # The caret keeps its place in the text when the symbol shrinks.
        assert act(app, wide, end[wide], "toggle") == replaced(wide, "- [x] Wide india")
        assert app.client.call("editor.get")["cursor"] == {
            "line": wide, "column": width("- [x] Wide india")}
        assert app.client.call("tasks.get")["line"]["task"] == {
            "box": [2, 5], "symbol": "x", "status": "Done", "type": "done"}
        app.client.call("editor.undo")
        saved(app, path, ORIGINAL)


def test_the_toggle_over_a_selection_follows_the_statuses_in_it(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with status_app(tmp_path, engine, "on") as (app, path):
        call = app.client.call

        def toggled(first: int, last: int) -> str:
            call("editor.cursor", {"line": last, "column": width(LINES[last]),
                                   "selection": {"line": first, "column": 0}})
            result = call("command.run", {"id": "markdown.toggle_task"})
            assert result["disposition"] == "completed", result
            return call("editor.get")["text"]

        # Open through wide: an unfinished task among them, so every task
        # becomes done, the cancelled one included; a done one keeps its X.
        first, last = line_of("- [ ] Open"), line_of("- [✓]")
        done = "\n".join(
            LINES[:first]
            + ["- [x] Open alpha", "- [x] Doing bravo", "- [x] Done charlie", "- [X] Done delta",
               "- [x] Cancelled echo", "- [x] Unknown foxtrot", "- [x] x", "- [x] Bracket golf",
               "- [x] Accent hotel", "- [x] Wide india", "- [x] Check juliet"]
            + LINES[last + 1:])
        assert toggled(first, last) == done
        saved(app, path, done)
        call("editor.undo")
        assert call("editor.get")["text"] == ORIGINAL
        # Done and cancelled only: every task becomes open.
        first, last = line_of("- [x]"), line_of("- [-]")
        opened = "\n".join(
            LINES[:first] + ["- [ ] Done charlie", "- [ ] Done delta", "- [ ] Cancelled echo"]
            + LINES[last + 1:])
        assert toggled(first, last) == opened
        saved(app, path, opened)
        call("editor.undo")
        saved(app, path, ORIGINAL)


def test_a_search_snippet_drops_a_status_box_only_with_tasks_on(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    for tasks, snippet in [("on", "Open alpha · Doing bravo · Done charlie"),
                           ("off", "Open alpha · [/] Doing bravo · Done charlie")]:
        with status_app(tmp_path / tasks, engine, tasks) as (app, _):
            call = app.client.call
            call("palette.open", {"mode": "search"})
            rows = call("palette.query", {"query": "bravo"})["navigation"]["modal"]["results"]
            call("palette.close")
            assert rows and rows[0]["hit"] == "content", rows
            assert snippet in rows[0]["snippet"], rows[0]["snippet"]


def test_with_tasks_off_a_status_box_is_plain_text(tmp_path: Path, engine: str) -> None:  # noqa: F811
    doing, cancelled, wide = line_of("- [/] Doing"), line_of("- [-]"), line_of("- [界]")
    end = {line: width(LINES[line]) for line in range(len(LINES))}
    with status_app(tmp_path, engine, "off") as (app, path):
        assert app.client.call("tasks.get") == {"tasks": False}
        check(app, path, [
            # No box: Enter continues the plain item, Backspace takes one
            # character, and the toggle gives the item a box of its own.
            (doing, end[doing], "enter", replaced(doing, LINES[doing], "- ")),
            (cancelled, end[cancelled], "enter", replaced(cancelled, LINES[cancelled], "- ")),
            (wide, end[wide], "enter", replaced(wide, LINES[wide], "- ")),
            (doing, 6, "backspace", replaced(doing, "- [/]Doing bravo")),
            (wide, width("- [界] "), "backspace", replaced(wide, "- [界]Wide india")),
            (doing, end[doing], "toggle", replaced(doing, "- [ ] [/] Doing bravo")),
            (cancelled, end[cancelled], "toggle",
             replaced(cancelled, "- [ ] [-] Cancelled echo")),
            (wide, end[wide], "toggle", replaced(wide, "- [ ] [界] Wide india")),
            # The three plain boxes are boxes still.
            (line_of("- [ ] Open"), 0, "toggle", replaced(2, "- [x] Open alpha")),
            (line_of("- [X]"), 0, "toggle", replaced(5, "- [ ] Done delta")),
            (line_of("- [x]"), 6, "backspace", replaced(4, "- Done charlie")),
            (line_of("- [x]"), end[4], "enter", replaced(4, LINES[4], "- [ ] ")),
        ])
