"""The task-line baseline through the real app (discovery/tasks-build.md item
0.1): Enter, Backspace and ``markdown.toggle_task`` on the fixture's task
lines, with ``tasks = off``, on the engines ``WRENTEST_ENGINE`` names,
asserting the note's bytes on disk.

The expected bytes are what Wren did before any task code existed, the same
behaviour ``crates/wren-iced/golden/tasks-baseline.txt`` pins in Rust. They
must stay as they are, with ``WRENTEST_TASKS=off``, to the end of Run B: do
not change them to make a later item pass.
"""

from __future__ import annotations

import shutil
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from tools.wrentest.test_editor import engine  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "tasks" / "workspace"
NOTE = "Tasks/Baseline/Task lines.md"
ORIGINAL = (FIXTURE / NOTE).read_text(encoding="utf-8")
LINES = ORIGINAL.split("\n")


def replaced(line: int, *new: str) -> str:
    """The note with line *line* replaced by the lines *new*."""
    return "\n".join(LINES[:line] + list(new) + LINES[line + 1:])


# (line, column, action, the note's bytes after it)
CASES = [
    # Enter at the line's end continues the item with an empty box, or with
    # none where today's classifier reads no box.
    (2, 21, "enter", replaced(2, LINES[2], "- [ ] ")),
    (3, 21, "enter", replaced(3, LINES[3], "- [ ] ")),
    (5, 22, "enter", replaced(5, LINES[5], "- ")),
    (7, 25, "enter", replaced(7, LINES[7], "- ")),
    (8, 24, "enter", replaced(8, LINES[8], "> - [ ] ")),
    (9, 26, "enter", replaced(9, LINES[9], "2. [ ] ")),
    (10, 20, "enter", replaced(10, LINES[10], "- [ ]\t")),
    (18, 40, "enter", replaced(18, LINES[18], "- [ ] ")),
    (20, 5, "enter", replaced(20, "", "")),
    # Backspace at the text removes the box, or the marker where there is
    # no box.
    (2, 6, "backspace", replaced(2, "- Open alpha task")),
    (4, 6, "backspace", replaced(4, "- Done charlie task")),
    (6, 2, "backspace", replaced(6, "  [-] Cancelled echo task")),
    (8, 8, "backspace", replaced(8, "> - Quoted golf task")),
    (9, 7, "backspace", replaced(9, "1. Numbered hotel task")),
    (10, 6, "backspace", replaced(10, "- Tab india task")),
    # The toggle flips a space, x or X; any other box gains one in front,
    # and no date is written or taken away.
    (2, 21, "toggle", replaced(2, "- [x] Open alpha task")),
    (4, 23, "toggle", replaced(4, "- [ ] Done charlie task")),
    (5, 22, "toggle", replaced(5, "- [ ] [/] Doing delta task")),
    (6, 25, "toggle", replaced(6, "- [ ] [-] Cancelled echo task")),
    (7, 25, "toggle", replaced(7, "- [ ] [a] Unknown foxtrot box")),
    (8, 24, "toggle", replaced(8, "> - [x] Quoted golf task")),
    (9, 26, "toggle", replaced(9, "1. [x] Numbered hotel task")),
    (10, 20, "toggle", replaced(10, "- [x]\tTab india task")),
    (14, 35, "toggle", replaced(14, "- [x] Due mike task 📅 2026-11-20")),
    (17, 53, "toggle", replaced(17, "- [x] Every papa token ⏫ @bob 📅 2026-11-18 #home")),
    (18, 40, "toggle", replaced(18, "- [ ] Stamped quebec task ✅ 2026-11-19")),
    (23, 24, "toggle", ORIGINAL),
]


# The words the drawn rows mark with spell check on: the bundled dictionary's
# misses in the descriptions, and none of the `@`, `#`, date or emoji tokens.
DRAWN_MARKS = [
    (6, "Cancelled"), (10, "india"), (11, "juliet"), (13, "lima"), (15, "november"),
    (16, "oscar"), (18, "quebec"), (19, "recieve"),
]


@contextmanager
def baseline_app(tmp_path: Path, engine: str,  # noqa: F811
                 spell: str = "off") -> Iterator[tuple[WrenApp, Path]]:
    workspace = tmp_path / "workspace"
    shutil.copytree(FIXTURE, workspace)
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\ntasks = off\nspell-check = {spell}\n",
                 today="2026-11-19",
                 artifacts_dir=artifact_directory(f"tasks-baseline-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert app.client.call("tasks.get") == {"tasks": False}
        assert app.client.call("session.get")["banner"] is None
        note = next(row["id"] for row in app.client.call("session.get")["notes"]
                    if row["path"] == NOTE)
        app.client.call("core.apply", {"op": "open_note", "note": note})
        wait_for(lambda: app.client.call("editor.get")["text"] == ORIGINAL,
                 description="the note opens")
        app.client.call("editor.focus")
        yield app, workspace / NOTE


def saved(app: WrenApp, path: Path, text: str) -> None:
    """Wait for autosave to put *text* on disk."""
    def written() -> bool:
        document = app.client.call("session.get")["document"]
        return (document is not None and not document["dirty"]
                and path.read_bytes() == text.encode("utf-8"))
    wait_for(written, timeout=10, description="the note is saved")


def test_task_lines_edit_as_before_with_tasks_off(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with baseline_app(tmp_path, engine) as (app, path):
        call = app.client.call
        for line, column, action, expected in CASES:
            case = f"{action} at {line}:{column} ({LINES[line]!r})"
            call("editor.cursor", {"line": line, "column": column, "selection": None})
            if action == "toggle":
                result = call("command.run", {"id": "markdown.toggle_task"})
                assert result["disposition"] == "completed", (case, result)
            else:
                call("editor.key", {"key": action, "mods": []})
            assert call("editor.get")["text"] == expected, case
            saved(app, path, expected)
            if expected != ORIGINAL:
                # One undo takes it back, on disk too.
                call("editor.undo")
                assert call("editor.get")["text"] == ORIGINAL, case
                saved(app, path, ORIGINAL)


def test_drawn_spelling_marks_skip_task_tokens_with_tasks_off(tmp_path: Path,
                                                             engine: str) -> None:  # noqa: F811
    """The advanced engine's drawn marks; classic checks nothing."""
    with baseline_app(tmp_path, engine, spell="on") as (app, _):
        call = app.client.call
        wait_for(lambda: call("editor.spell")["ready"], timeout=10,
                 description="the dictionary loads")
        call("editor.cursor", {"line": 0, "column": 0, "selection": None})
        # A screenshot waits for a frame drawn after every change so far.
        call("app.screenshot", {"path": str(artifact_directory(f"tasks-baseline-spell-{engine}")
                                            / "frame.png")})
        state = call("editor.spell")
        drawn = sorted((mark["line"], mark["word"]) for mark in state["marks"])
        if engine == "advanced":
            assert state["enabled"] is True
            assert drawn == DRAWN_MARKS
            assert all(mark["shown"] for mark in state["marks"])
        else:
            assert state["enabled"] is False
            assert drawn == []
