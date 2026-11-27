"""The task view's list (discovery/tasks-build.md item 10.3; discovery/tasks.md
§9): the tasks of a scope from the task index, filtered, grouped and sorted,
read-only, keyboard-first, with the filters remembered per scope in
``.wren/tasks.toml``.

The workspace is the M11 fixture (``fixtures/tasks/list/``): two groups,
owners that ``tasks-me`` names (``sam``, ``Sam.Lee``), a task with two
owners, finished work with and without a stamp, stamps 14 and 15 days before
the pinned day, a due date that is no real day, and a note whose front matter
hides its tasks.

``tasks.view`` with ``"list": true`` adds the list's state: its filters,
chips, groups and rows (with their flags), counts, focus, menu and where its
controls were drawn. ``{"key": …}`` presses a key as the view's key wrapper
and then the app hear it, and ``{"click": …}`` clicks a control, a row, a
row's source, or a row of the menu over the view.

Every case writes its own ``tasks = on`` and the advanced engine, whatever
the lane's ``WRENTEST_TASKS`` and ``WRENTEST_ENGINE`` say.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
import shutil
import tomllib

import pytest

from tools.wrentest.test_editor import xdo
from tools.wrentest.test_gestures import move, point_in
from tools.wrentest.test_navigation import session
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "tasks" / "list"
# The design page's day, a Thursday; every task test pins it.
TODAY = "2026-11-19"
ON = "editor-engine = advanced\ntasks = on\nboards = on\ntasks-me = sam, sam.lee\n"
ROADMAP = "Work/Planning/Roadmap.md"
STANDUP = "Work/Meetings/Standup.md"

STANDUP_ROWS = [
    "Order badges",
    "Ping finance @SAM",
    "Follow up with legal @priya",
    "Close the sprint",
    "Cancel the vendor call",
]
ROADMAP_ROWS = [
    "Draft the Q1 plan",
    "Book the offsite @priya @Sam.Lee",
    "Review the budget @sam",
    "Fix the date",
    "Write the retro",
    "Send the minutes",
    "Tidy the wiki",
]
DEFAULTS = {"statuses": [], "owner": "anyone", "group": "note", "hide_old": True,
            "due": None, "priority": None}


def workspace(tmp_path: Path) -> Path:
    root = tmp_path / "workspace"
    if not root.exists():
        shutil.copytree(FIXTURE, root)
    return root


@contextmanager
def list_app(tmp_path: Path, case: str) -> Iterator[WrenApp]:
    with WrenApp(workspace=workspace(tmp_path), evaluation=False, settings=ON, today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-list-{case}")) as app:
        assert app.client.call("editor.get")["engine"] == "advanced"
        assert app.client.call("tasks.get")["today"] == TODAY
        yield app


def state(app: WrenApp) -> dict:
    return app.client.call("tasks.view", {"list": True})["list"]


def show(app: WrenApp, scope: str = "workspace", path: str | None = None) -> dict:
    target = {"scope": scope} if path is None else {"scope": scope, "path": path}
    answer = app.client.call("tasks.view", {"open": target, "list": True})
    assert answer["disposition"] == "completed", answer
    assert answer["keys"], answer
    return answer["list"]


def press(app: WrenApp, key: str, *mods: str) -> dict:
    return app.client.call("tasks.view", {"key": {"key": key, "mods": list(mods)},
                                          "list": True})["list"]


def click(app: WrenApp, **target) -> dict:
    return app.client.call("tasks.view", {"click": target, "list": True})["list"]


def choose(app: WrenApp, control: str, row: str) -> dict:
    """Open the menu of *control* by a click and click its row *row*."""
    menu = click(app, control=control)["menu"]
    assert menu is not None and menu["for"] == control, menu
    return click(app, menu=row)


def descriptions(listing: dict) -> list[str]:
    return [row["description"] for row in listing["rows"]]


def selected(listing: dict) -> str:
    rows = [row["description"] for row in listing["rows"] if row["selected"]]
    assert len(rows) == 1, rows
    return rows[0]


def tasks_toml(root: Path) -> str | None:
    path = root / ".wren" / "tasks.toml"
    return path.read_text(encoding="utf-8") if path.exists() else None


def test_counts_order_and_strikethrough(tmp_path: Path) -> None:
    with list_app(tmp_path, "order") as app:
        listing = show(app)
        assert listing["filters"] == DEFAULTS
        assert [(group["label"], group["count"]) for group in listing["groups"]] == [
            ("Standup", 5), ("Roadmap", 7)]
        assert descriptions(listing) == STANDUP_ROWS + ROADMAP_ROWS
        assert listing["shown"] == 12 and (listing["open"], listing["overdue"]) == (8, 1)
        assert listing["left_out"] == "Left out: 1 note (2 tasks, tasks: off)"
        assert listing["hint"] == "Space toggle · S D P · E edit · ↵ open · R refresh"
        assert [(chip["status"], chip["count"], chip["pressed"]) for chip in listing["chips"]] == [
            ("open", 7, False), ("doing", 1, False), ("done", 3, False), ("cancelled", 1, False)]
        rows = {row["description"]: row for row in listing["rows"]}
        # Finished work is struck through and dimmed, below the open tasks,
        # with its stamp in place of the due date; work with no stamp stays.
        for name in ("Close the sprint", "Cancel the vendor call", "Send the minutes",
                     "Tidy the wiki"):
            assert rows[name]["struck"] and rows[name]["dimmed"], rows[name]
        for name in ("Order badges", "Review the budget @sam", "Draft the Q1 plan"):
            assert not rows[name]["struck"], rows[name]
        assert rows["Review the budget @sam"]["status"] == "doing"
        assert (rows["Send the minutes"]["due"], rows["Send the minutes"]["due_look"]) == (
            "✓ Thu 5 Nov", "finished")
        assert rows["Cancel the vendor call"]["due"] == "✕ Thu 12 Nov"
        assert rows["Tidy the wiki"]["due"] is None
        assert rows["Draft the Q1 plan"]["overdue"] and rows["Draft the Q1 plan"]["due_look"] == "overdue"
        assert (rows["Fix the date"]["due"], rows["Fix the date"]["due_look"]) == (
            "2026-02-30 ⚠", "invalid")
        assert rows["Draft the Q1 plan"]["priority"] == "High"
        # Grouped by note, the source is the nearest heading.
        assert rows["Write the retro"]["source"] == "Later"
        assert rows["Order badges"]["source"] == "Standup"
        assert "Archive old docs" not in rows, "stamped 15 days ago"
        assert selected(listing) == "Order badges"

        # Without hiding old work, the 15-day-old stamp shows too.
        listing = click(app, control="hide_old")
        assert listing["filters"]["hide_old"] is False and listing["shown"] == 13
        assert "Archive old docs" in descriptions(listing)
        listing = click(app, control="hide_old")
        assert listing["shown"] == 12


def test_chips_narrow_and_show_all_brings_every_status_back(tmp_path: Path) -> None:
    with list_app(tmp_path, "chips") as app:
        show(app)
        listing = click(app, control="chip:open")
        assert {row["status"] for row in listing["rows"]} == {"open"}
        assert listing["shown"] == 7
        assert listing["chips"][0]["pressed"] and not listing["chips"][1]["pressed"]
        listing = click(app, control="chip:cancelled")
        assert {row["status"] for row in listing["rows"]} == {"open", "cancelled"}
        # The counts are what every filter but the chips keeps.
        assert [chip["count"] for chip in listing["chips"]] == [7, 1, 3, 1]
        assert listing["focus"] == "list", "a click leaves the keyboard with the list"
        listing = click(app, control="show_all")
        assert listing["filters"]["statuses"] == [] and listing["shown"] == 12
        # A filter that keeps nothing says how to widen it.
        listing = choose(app, "priority", "Lowest")
        listing = click(app, control="chip:done")
        assert listing["rows"] == [] and listing["groups"] == []
        assert listing["empty"] == ("No tasks match. Press Show all, set Priority to Any, "
                                    "or widen the scope.")


def test_owner_me_reads_tasks_me_and_a_person_narrows(tmp_path: Path) -> None:
    with list_app(tmp_path, "owner") as app:
        show(app)
        menu = click(app, control="owner")["menu"]
        assert [row["label"] for row in menu["rows"]] == [
            "Anyone", "Me", "@priya", "@SAM", "@Sam.Lee"]
        assert menu["rows"][0]["checked"] and menu["rows"][1]["key"] == "M"
        listing = click(app, menu="Me")
        assert listing["filters"]["owner"] == "me" and listing["shown"] == 11
        rows = descriptions(listing)
        # No owner, @sam, @SAM and @Sam.Lee are all Me; @priya alone is not.
        for name in ("Order badges", "Ping finance @SAM", "Review the budget @sam",
                     "Book the offsite @priya @Sam.Lee"):
            assert name in rows
        assert "Follow up with legal @priya" not in rows
        listing = choose(app, "owner", "@priya")
        assert listing["filters"]["owner"] == "@priya"
        assert descriptions(listing) == ["Follow up with legal @priya",
                                         "Book the offsite @priya @Sam.Lee"]
        listing = choose(app, "owner", "Anyone")
        assert listing["shown"] == 12


def test_group_by_each(tmp_path: Path) -> None:
    with list_app(tmp_path, "group") as app:
        show(app)
        listing = choose(app, "group", "Due date")
        assert [group["label"] for group in listing["groups"]] == [
            "Overdue", "Earlier", "Today", "This week", "Next week", "Later", "No date"]
        rows = {row["description"]: row for row in listing["rows"]}
        # Out of note grouping the source names the group and the note.
        assert rows["Write the retro"]["source"] == "Planning › Roadmap · Later"
        listing = choose(app, "group", "Priority")
        assert [group["label"] for group in listing["groups"]] == [
            "\U0001f53a Highest", "⏫ High", "\U0001f53c Medium", "No priority",
            "\U0001f53d Low", "⏬ Lowest"]
        listing = choose(app, "group", "Owner")
        groups = [(group["label"], group["count"]) for group in listing["groups"]]
        assert groups == [("Me", 11), ("@priya", 2)]
        # The task with two owners is under each, and counted once.
        offsite = [row for row in listing["rows"]
                   if row["description"] == "Book the offsite @priya @Sam.Lee"]
        assert [row["group"] for row in offsite] == [0, 1]
        assert listing["shown"] == 12 and len(listing["rows"]) == 13
        listing = choose(app, "group", "Note")
        assert listing["filters"] == DEFAULTS


def test_filters_are_remembered_per_scope_after_a_restart(tmp_path: Path) -> None:
    root = workspace(tmp_path)
    with list_app(tmp_path, "persist") as app:
        show(app)
        show(app, "group", "Work/Planning")
        assert tasks_toml(root) is None, "opening a view writes nothing"
        show(app)
        click(app, control="chip:doing")
        choose(app, "group", "Due date")
        choose(app, "due", "Today")
        assert tomllib.loads(tasks_toml(root)) == {
            "schema_version": 1,
            "views": {".": {"statuses": ["doing"], "group": "due", "due": "today"}}}
    with list_app(tmp_path, "persist-again") as app:
        listing = show(app)
        assert listing["filters"] == {**DEFAULTS, "statuses": ["doing"], "group": "due",
                                      "due": "Today"}
        assert descriptions(listing) == ["Review the budget @sam"]
        # Another scope keeps its own, the defaults.
        listing = show(app, "group", "Work/Planning")
        assert listing["filters"] == DEFAULTS and listing["shown"] == 7
        assert tasks_toml(root).count("[views.") == 1


def test_keys_move_the_selection_and_enter_opens_the_note_at_the_line(tmp_path: Path) -> None:
    with list_app(tmp_path, "keys") as app:
        show(app)
        assert selected(press(app, "j")) == "Ping finance @SAM"
        assert selected(press(app, "down")) == "Follow up with legal @priya"
        assert selected(press(app, "k")) == "Ping finance @SAM"
        assert selected(press(app, "end")) == "Tidy the wiki"
        assert selected(press(app, "home")) == "Order badges"
        assert selected(press(app, "page_down")) != "Order badges"
        assert selected(press(app, "page_up")) == "Order badges"
        # Escape does nothing in the list: the view stays, its filters too.
        listing = press(app, "escape")
        assert app.client.call("tasks.view")["open"] and listing["filters"] == DEFAULTS
        listing = press(app, "j")
        listing = press(app, "j")
        row = next(row for row in listing["rows"] if row["selected"])
        assert row["description"] == "Follow up with legal @priya"
        press(app, "enter")
        assert not app.client.call("tasks.view")["open"]
        assert session(app)["document"]["path"] == STANDUP
        editor = app.client.call("editor.get")
        assert editor["cursor"]["line"] == row["line"]
        assert editor["text"].splitlines()[row["line"]].startswith(
            "- [ ] Follow up with legal @priya")
        wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                 description="the editor has the keyboard")
        assert session(app)["document"]["dirty"] is False

        # A click on a row's source opens it too.
        listing = show(app)
        entry = next(index for index, row in enumerate(listing["rows"])
                     if row["description"] == "Write the retro")
        click(app, source=entry)
        assert session(app)["document"]["path"] == ROADMAP
        line = listing["rows"][entry]["line"]
        assert app.client.call("editor.get")["cursor"]["line"] == line


def test_the_scope_menu_by_keys_and_by_click(tmp_path: Path) -> None:
    with list_app(tmp_path, "scope") as app:
        show(app)
        listing = press(app, "tab")
        assert listing["focus"] == "scope"
        menu = press(app, "enter")["menu"]
        assert menu["for"] == "scope" and menu["title"] == "Show tasks in"
        assert [row["key"] for row in menu["rows"]] == ["W", "S", "G", "N"]
        assert menu["rows"][0]["checked"] and menu["rows"][1]["label"] == "Work (section)"
        # Escape closes it without a choice, and the chip keeps the keys.
        listing = press(app, "escape")
        assert listing["menu"] is None and listing["focus"] == "scope"
        assert app.client.call("tasks.view")["scope"] == "workspace"
        # Down opens it too, and its key chooses a row.
        assert press(app, "down")["menu"]["for"] == "scope"
        press(app, "s")
        view = app.client.call("tasks.view")
        assert (view["scope"], view["path"]) == ("section", "Work")
        # By click: the chip, then a row.
        click(app, control="scope")
        listing = click(app, menu="workspace")
        assert app.client.call("tasks.view")["scope"] == "workspace"
        assert listing["menu"] is None
        # A press outside the menu closes it and chooses nothing.
        click(app, control="scope")
        listing = click(app, outside=True)
        assert listing["menu"] is None
        assert app.client.call("tasks.view")["scope"] == "workspace"


@pytest.mark.native_x11
def test_native_a_click_on_a_chip_and_on_a_rows_source(tmp_path: Path) -> None:
    with list_app(tmp_path, "native") as app:
        windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
        assert len(windows) == 1, windows
        window = windows[0]
        xdo("windowfocus", "--sync", window)
        show(app)

        def drawn(name: str) -> dict:
            return wait_for(lambda: state(app)["bounds"].get(name), timeout=5,
                            description=f"the {name} control drawn")

        move(window, point_in(drawn("chip:doing")))
        xdo("click", "1")
        wait_for(lambda: descriptions(state(app)) == ["Review the budget @sam"], timeout=5,
                 description="the Doing chip narrows the list")
        move(window, point_in(drawn("chip:doing")))
        xdo("click", "1")
        wait_for(lambda: state(app)["shown"] == 12, timeout=5,
                 description="the chip again shows every status")

        def source() -> dict | None:
            rows = state(app)["rows"]
            row = next(row for row in rows if row["description"] == "Draft the Q1 plan")
            return row if row["source_bounds"] else None
        row = wait_for(source, timeout=5, description="the row's source drawn")
        move(window, point_in(row["source_bounds"]))
        xdo("click", "1")
        wait_for(lambda: not app.client.call("tasks.view")["open"], timeout=5,
                 description="the click opens the note")
        assert session(app)["document"]["path"] == ROADMAP
        assert app.client.call("editor.get")["cursor"]["line"] == row["line"]
