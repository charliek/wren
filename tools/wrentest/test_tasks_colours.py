"""Task colours (discovery/tasks-build.md item 2.2; discovery/tasks.md §3.2
Q9, §5.6, §5.7 and §6): with ``tasks = on`` the editor draws a due date in
its date colour, overdue (the theme's danger, bold) once today has passed it
on an unfinished task, and an owner in its own colour; a finished task's
stamps and dates recede; a priority glyph has no colour of its own. Spell
check reads none of a task's tokens.

``tasks.get`` reports the caret line's tokens as the highlighter draws them,
by palette role. Each case writes its own ``tasks`` line and the engine
``WRENTEST_ENGINE`` names, whatever the lane's ``WRENTEST_TASKS`` says.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from tools.wrentest.test_editor import engine  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

TODAY = "2026-11-19"
NOTE = "Tasks/Colours/Due dates.md"
LINES = [
    "# Due dates",
    "",
    "- [ ] Send the deck @alice.chen ⏫ 📅 2026-11-20",
    "- [ ] Call the bank 📅 2026-11-19",
    "- [/] Review the packet @sam 📅 2026-11-17",
    "- [x] Shipped the notes 📅 2026-11-01 ✅ 2026-11-18",
    "- [-] Book the room @sam ❌ 2026-11-16",
    "- [ ] Pay the rent 📅 2026-02-30",
    "- [ ] Plan the trpi 🆔 qzxv 🔽",
    "> - [ ] File the claim @sam ⏫",
]
ORIGINAL = "\n".join(LINES)
FUTURE, TODAYS, LATE, DONE, CANCELLED, INVALID, SPELLING, QUOTED = range(2, 10)


def token(kind: str, text: str, colour: str | None, bold: bool = False) -> tuple:
    return (kind, text, colour, bold)


def tokens(app: WrenApp, line: int) -> list[tuple]:
    """Line *line*'s task tokens as ``tasks.get`` reports them."""
    app.client.call("editor.cursor", {"line": line, "column": 0, "selection": None})
    state = app.client.call("tasks.get")
    assert state["line"]["number"] == line, state
    found = []
    for entry in state["tokens"]:
        start, end = entry["start"], entry["end"]
        assert LINES[line].encode("utf-8")[start:end].decode("utf-8") == entry["text"], entry
        found.append((entry["kind"], entry["text"], entry["colour"], entry["bold"]))
    return found


@contextmanager
def colours_app(tmp_path: Path, engine: str, tasks: str,  # noqa: F811
                spell: str = "off") -> Iterator[WrenApp]:
    workspace = tmp_path / "workspace"
    path = workspace / NOTE
    path.parent.mkdir(parents=True)
    path.write_text(ORIGINAL, encoding="utf-8")
    settings = f"editor-engine = {engine}\ntasks = {tasks}\nspell-check = {spell}\n"
    with WrenApp(workspace=workspace, evaluation=False, settings=settings, today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-colours-{tasks}-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert app.client.call("tasks.get")["tasks"] is (tasks == "on")
        note = next(row["id"] for row in app.client.call("session.get")["notes"]
                    if row["path"] == NOTE)
        app.client.call("core.apply", {"op": "open_note", "note": note})
        wait_for(lambda: app.client.call("editor.get")["text"] == ORIGINAL,
                 description="the note opens")
        yield app


def test_each_token_is_drawn_in_its_colour(tmp_path: Path, engine: str) -> None:  # noqa: F811
    with colours_app(tmp_path, engine, "on") as app:
        assert tokens(app, FUTURE) == [
            token("owner", "@alice.chen", "owner"),
            token("priority", "⏫", None),
            token("due", "📅 2026-11-20", "date"),
        ]
        # Due today is not late.
        assert tokens(app, TODAYS) == [token("due", "📅 2026-11-19", "date")]
        assert tokens(app, LATE) == [
            token("owner", "@sam", "owner"),
            token("due", "📅 2026-11-17", "overdue", bold=True),
        ]
        # A finished task's tokens recede, a past due date too, and its
        # owner dims with its description.
        assert tokens(app, DONE) == [
            token("due", "📅 2026-11-01", "faint"),
            token("done", "✅ 2026-11-18", "faint"),
        ]
        assert tokens(app, CANCELLED) == [
            token("owner", "@sam", "completed"),
            token("cancelled", "❌ 2026-11-16", "faint"),
        ]
        # No such day: the problem colour, without the weight of late work.
        assert tokens(app, INVALID) == [token("due", "📅 2026-02-30", "overdue")]
        assert tokens(app, SPELLING) == [
            token("id", "🆔 qzxv", "faint"),
            token("priority", "🔽", None),
        ]
        # In a quote a priority glyph is quoted text. Wren Light draws a
        # quote and a done task's text in one colour, and the role is named
        # still.
        app.client.call("theme.set", {"id": "wren-light"})
        assert tokens(app, QUOTED) == [
            token("owner", "@sam", "owner"),
            token("priority", "⏫", "quote"),
        ]


def test_moving_today_turns_the_same_due_date_overdue(tmp_path: Path,
                                                      engine: str) -> None:  # noqa: F811
    with colours_app(tmp_path, engine, "on") as app:
        call = app.client.call
        due = ("due", "📅 2026-11-20")
        assert tokens(app, FUTURE)[-1] == (*due, "date", False)
        call("tasks.set_today", {"date": "2026-11-20"})
        assert tokens(app, FUTURE)[-1] == (*due, "date", False)
        assert tokens(app, TODAYS) == [token("due", "📅 2026-11-19", "overdue", bold=True)]
        state = call("tasks.set_today", {"date": "2026-11-21"})
        assert state["day_changes"] == 2
        assert tokens(app, FUTURE)[-1] == (*due, "overdue", True)
        # A finished task is never late.
        assert tokens(app, DONE)[0] == token("due", "📅 2026-11-01", "faint")
        # And back: a day moved back is a change too.
        call("tasks.set_today", {"date": TODAY})
        assert tokens(app, FUTURE)[-1] == (*due, "date", False)
        assert call("editor.get")["text"] == ORIGINAL
        call("app.screenshot", {"path": str(artifact_directory(f"tasks-colours-day-{engine}")
                                            / "frame.png")})


def test_with_tasks_off_a_task_line_has_no_tokens(tmp_path: Path,
                                                  engine: str) -> None:  # noqa: F811
    with colours_app(tmp_path, engine, "off") as app:
        assert app.client.call("tasks.get") == {"tasks": False}


def drawn_marks(app: WrenApp, name: str) -> tuple[bool, list[tuple[int, str]]]:
    """Whether spell check is on, and the marks a frame drawn after every
    change so far shows."""
    call = app.client.call
    wait_for(lambda: call("editor.spell")["ready"], timeout=10,
             description="the dictionary loads")
    call("editor.cursor", {"line": 0, "column": 0, "selection": None})
    call("app.screenshot", {"path": str(artifact_directory(name) / "frame.png")})
    state = call("editor.spell")
    return state["enabled"], sorted((mark["line"], mark["word"]) for mark in state["marks"])


def test_spell_check_skips_owners_and_task_tokens(tmp_path: Path,
                                                  engine: str) -> None:  # noqa: F811
    """The advanced engine's drawn marks: a misspelled description word is
    marked, an owner, a date and an id are not; with tasks off the id is
    prose again and is marked. Classic checks nothing."""
    with colours_app(tmp_path, engine, "on", spell="on") as app:
        enabled, marks = drawn_marks(app, f"tasks-colours-spell-on-{engine}")
        assert enabled is (engine == "advanced")
        if engine == "advanced":
            assert marks == [(SPELLING, "trpi")]
        else:
            assert marks == []
    with colours_app(tmp_path / "off", engine, "off", spell="on") as app:
        enabled, marks = drawn_marks(app, f"tasks-colours-spell-off-{engine}")
        if engine == "advanced":
            assert marks == [(SPELLING, "qzxv"), (SPELLING, "trpi")]
        else:
            assert marks == []
