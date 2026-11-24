"""The status bar's task line (discovery/tasks-build.md item 2.3;
discovery/tasks.md §2 item 23): with ``tasks = on`` the editor's footer
spells out the caret line's task, "Doing · due Mon 23 Nov, in 4 days · ⏫
High · @alice.chen", the overdue part in the theme's danger colour. It
follows the caret, each edit and the day. With tasks off the footer is what
it was.

``tasks.get`` reports the footer's task line as ``footer``: its text,
whether it is overdue, and its pieces with the look each is drawn in;
``null`` off a task line. It is the line the footer shows when it has room:
how much of it shows at a narrow width is for the scenes to look at
(``capture_scenes.py --only tasks``). Each case writes its own ``tasks`` line and the
engine ``WRENTEST_ENGINE`` names, whatever the lane's ``WRENTEST_TASKS``
says.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from tools.wrentest.test_editor import engine  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

# The design page's day, a Thursday; every task test pins it.
TODAY = "2026-11-19"
NOTE = "Tasks/Footer/Status bar.md"
LINES = [
    "# Status bar",
    "",
    "- [/] Draft the cutover runbook @alice.chen ⏫ 📅 2026-11-23",
    "- [ ] Send the deck @sam @priya 🔼 📅 2026-11-16",
    "Not a task line.",
    "- [x] Review the packet @bob 📅 2026-11-12 ✅ 2026-11-16",
    "- [-] Book the room ❌ 2026-11-18",
    "- [?] Ask legal 📅 2026-02-30",
    "> 1. [ ] Call the bank 📅 2026-11-20",
]
ORIGINAL = "\n".join(LINES)
EXAMPLE = "Doing · due Mon 23 Nov, in 4 days · ⏫ High · @alice.chen"


@contextmanager
def footer_app(tmp_path: Path, engine: str, tasks: str) -> Iterator[WrenApp]:  # noqa: F811
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    settings = f"editor-engine = {engine}\ntasks = {tasks}\n"
    with WrenApp(workspace=workspace, evaluation=False, settings=settings, today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-footer-{tasks}-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert app.client.call("tasks.get")["tasks"] is (tasks == "on")
        note = next(row["id"] for row in app.client.call("session.get")["notes"]
                    if row["path"] == NOTE)
        app.client.call("core.apply", {"op": "open_note", "note": note})
        wait_for(lambda: app.client.call("editor.get")["text"] == ORIGINAL,
                 description="the note opens")
        app.client.call("editor.focus")
        yield app


def footer(app: WrenApp) -> dict | None:
    return app.client.call("tasks.get")["footer"]


def text_at(app: WrenApp, line: int) -> str | None:
    """The footer's task line with the caret on *line*."""
    app.client.call("editor.cursor", {"line": line, "column": 0, "selection": None})
    state = app.client.call("tasks.get")
    assert state["line"]["number"] == line, state
    return state["footer"] and state["footer"]["text"]


def test_tasks_get_reports_each_lines_footer(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with footer_app(tmp_path, engine, "on") as app:
        app.client.call("editor.cursor", {"line": 2, "column": 7, "selection": None})
        assert footer(app) == {
            "text": EXAMPLE,
            "overdue": False,
            "pieces": [
                {"text": "Doing", "look": "status"},
                {"text": "due Mon 23 Nov, in 4 days", "look": "plain"},
                {"text": "⏫ High", "look": "plain"},
                {"text": "@alice.chen", "look": "owners"},
            ],
        }
        app.client.call("editor.cursor", {"line": 3, "column": 0, "selection": None})
        assert footer(app) == {
            "text": "Open · due Mon 16 Nov, 3 days overdue · 🔼 Medium · @sam @priya",
            "overdue": True,
            "pieces": [
                {"text": "Open", "look": "status"},
                {"text": "due Mon 16 Nov, 3 days overdue", "look": "overdue"},
                {"text": "🔼 Medium", "look": "plain"},
                {"text": "@sam @priya", "look": "owners"},
            ],
        }
        assert text_at(app, 5) == "Done · due Thu 12 Nov · @bob · done Mon 16 Nov"
        assert text_at(app, 6) == "Cancelled · yours · cancelled Wed 18 Nov"
        app.client.call("editor.cursor", {"line": 7, "column": 0, "selection": None})
        assert footer(app) == {
            "text": "Open [?] · due 2026-02-30 is not a real date · yours",
            "overdue": False,
            "pieces": [
                {"text": "Open [?]", "look": "status"},
                {"text": "due 2026-02-30 is not a real date", "look": "invalid"},
                {"text": "yours", "look": "plain"},
            ],
        }
        assert text_at(app, 8) == "Open · due Fri 20 Nov, tomorrow · yours"
        assert app.client.call("editor.get")["text"] == ORIGINAL


def test_the_footer_follows_the_caret(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with footer_app(tmp_path, engine, "on") as app:
        call = app.client.call
        call("editor.cursor", {"line": 0, "column": 0, "selection": None})
        seen = []
        for _ in range(len(LINES) - 1):
            seen.append(footer(app) and footer(app)["text"])
            position = call("editor.key", {"key": "down", "mods": []})["cursor"]
            assert position["line"] == len(seen), position
        seen.append(footer(app) and footer(app)["text"])
        assert seen == [
            None,
            None,
            EXAMPLE,
            "Open · due Mon 16 Nov, 3 days overdue · 🔼 Medium · @sam @priya",
            None,
            "Done · due Thu 12 Nov · @bob · done Mon 16 Nov",
            "Cancelled · yours · cancelled Wed 18 Nov",
            "Open [?] · due 2026-02-30 is not a real date · yours",
            "Open · due Fri 20 Nov, tomorrow · yours",
        ]
        # Anywhere on the line, the line's own task.
        line = LINES[2].encode("utf-8")
        for column in (0, 3, line.index(b"@"), len(line)):
            call("editor.cursor", {"line": 2, "column": column, "selection": None})
            assert footer(app)["text"] == EXAMPLE, column


def test_the_footer_follows_an_edit(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with footer_app(tmp_path, engine, "on") as app:
        call = app.client.call
        end = len(LINES[2].encode("utf-8"))
        call("editor.cursor", {"line": 2, "column": end, "selection": None})
        call("editor.insert", {"text": " ✅ 2026-11-19"})
        assert footer(app)["text"] == EXAMPLE + " · done Thu 19 Nov"
        # Typing x in the box by hand: the status changes, and a finished
        # task's due date has no relative part.
        call("editor.cursor", {"line": 2, "column": 3, "selection": {"line": 2, "column": 4}})
        call("editor.insert", {"text": "x"})
        assert footer(app)["text"] == \
            "Done · due Mon 23 Nov · ⏫ High · @alice.chen · done Thu 19 Nov"
        # Undone, it is the example again.
        for _ in range(2):
            call("editor.undo")
        assert call("editor.get")["text"] == ORIGINAL
        call("editor.cursor", {"line": 2, "column": 0, "selection": None})
        assert footer(app)["text"] == EXAMPLE
        # A line that stops being a task leaves the footer.
        call("editor.cursor", {"line": 2, "column": 0, "selection": {"line": 2, "column": 6}})
        call("editor.action", {"kind": "delete"})
        assert call("editor.get")["text"].split("\n")[2] == LINES[2][6:]
        assert footer(app) is None


def test_the_relative_part_follows_the_day(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with footer_app(tmp_path, engine, "on") as app:
        call = app.client.call
        call("editor.cursor", {"line": 2, "column": 0, "selection": None})
        for day, due, overdue in [
            ("2026-11-22", "due Mon 23 Nov, tomorrow", False),
            ("2026-11-23", "due Mon 23 Nov, today", False),
            ("2026-11-24", "due Mon 23 Nov, 1 day overdue", True),
            ("2026-12-01", "due Mon 23 Nov, 8 days overdue", True),
            ("2026-11-02", "due Mon 23 Nov, in 3 weeks", False),
            (TODAY, "due Mon 23 Nov, in 4 days", False),
        ]:
            state = call("tasks.set_today", {"date": day})
            assert state["today"] == day
            assert state["footer"]["pieces"][1] == \
                {"text": due, "look": "overdue" if overdue else "plain"}, day
            assert state["footer"]["overdue"] is overdue, day
        assert call("editor.get")["text"] == ORIGINAL


def test_with_tasks_off_the_footer_has_no_task_line(tmp_path: Path,
                                                    engine: str) -> None:  # noqa: F811
    with footer_app(tmp_path, engine, "off") as app:
        for line in range(len(LINES)):
            app.client.call("editor.cursor", {"line": line, "column": 0, "selection": None})
            assert app.client.call("tasks.get") == {"tasks": False}
        app.client.call("editor.cursor", {"line": 2, "column": 0, "selection": None})
        app.client.call("app.screenshot", {"path": str(
            artifact_directory(f"tasks-footer-off-{engine}") / "frame.png")})
