"""Writes from the task view (discovery/tasks-build.md item 10.4; discovery/tasks.md
§2 items 15, 20, 22, 26 and 28, §9, §10).

Keys, clicks and menus on a row change its task in its note: Space and the
``markdown.toggle_task`` chord toggle done, a click on the box cycles the
status, S, P and D open the status menu, the priority menu and the calendar,
+ and - raise and lower the priority, E opens the task form, and a
right-click opens the row's task menu. Each write shows a message naming
what changed and the note's line, with Undo; the ``edit.undo`` chord undoes
it while the view has the keyboard; a row written stays where it stood
until R.

The workspace is written by the test: ``Work/Tasks/A.md``, open under the
view, and ``Work/Tasks/B.md``, closed. Every case writes its own ``tasks =
on`` and the advanced engine, whatever the lane's ``WRENTEST_TASKS`` and
``WRENTEST_ENGINE`` say, and pins the design page's day.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest

from tools.wrentest import test_gestures as native
from tools.wrentest.test_editor import focus_native, xdo
from tools.wrentest.test_navigation import core, item, session
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, native_mod, wait_for

# The design page's day, a Thursday; every task test pins it.
TODAY = "2026-11-19"
ON = "editor-engine = advanced\ntasks = on\nboards = on\n"
GROUP = "Work/Tasks"
A = f"{GROUP}/A.md"
B = f"{GROUP}/B.md"
A_TEXT = "# A\n\n- [ ] Mail the deck 📅 2026-12-01\n- [ ] Book the room\n"
B_TEXT = "# B\n\n- [ ] Order lunch\n"
LUNCH = "- [ ] Order lunch"
CHANGED = "This task changed; the list was refreshed"


@contextmanager
def view_app(tmp_path: Path, case: str,
             focused: bool = False) -> Iterator[tuple[WrenApp, Path]]:
    root = tmp_path / "workspace"
    for relative, text in {A: A_TEXT, B: B_TEXT}.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    with WrenApp(workspace=root, evaluation=False, settings=ON, today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-view-write-{case}")) as app:
        assert app.client.call("editor.get")["engine"] == "advanced"
        core(app, "open_note", note=item(app, "notes", A)["id"])
        assert session(app)["document"]["path"] == A
        if focused:
            # The window has the keyboard, given while the editor shows.
            focus_native(app)
        answer = app.client.call("tasks.view", {"open": {"scope": "group", "path": GROUP}})
        assert answer["disposition"] == "completed" and answer["keys"], answer
        yield app, root


def state(app: WrenApp) -> dict:
    return app.client.call("tasks.view", {"list": True})["list"]


def press(app: WrenApp, key: str, *mods: str) -> dict:
    return app.client.call("tasks.view", {"key": {"key": key, "mods": list(mods)},
                                          "list": True})["list"]


def click(app: WrenApp, **target) -> dict:
    return app.client.call("tasks.view", {"click": target, "list": True})["list"]


def row_of(listing: dict, description: str) -> int:
    for index, row in enumerate(listing["rows"]):
        if row["description"] == description:
            return index
    raise AssertionError(f"no row {description!r}: {listing['rows']}")


def select(app: WrenApp, description: str) -> int:
    index = row_of(state(app), description)
    listing = click(app, row=index)
    assert listing["rows"][index]["selected"], listing["rows"]
    return index


def descriptions(listing: dict) -> list[str]:
    return [row["description"] for row in listing["rows"]]


def notice(app: WrenApp) -> dict | None:
    return app.client.call("tasks.entry")["notice"]


def message(app: WrenApp) -> str | None:
    shown = notice(app)
    return shown and shown["text"]


def disk(root: Path, relative: str) -> str:
    return (root / relative).read_text(encoding="utf-8")


def draft(app: WrenApp) -> str:
    return session(app)["document"]["text"]


def test_each_key_writes_with_a_message_and_undoes(tmp_path: Path) -> None:
    with view_app(tmp_path, "keys") as (app, root):
        select(app, "Order lunch")
        cases = [
            (("space",), "- [x] Order lunch ✅ 2026-11-19", "Done"),
            (("enter", "mod"), "- [x] Order lunch ✅ 2026-11-19", "Done"),
            (("+",), "- [ ] Order lunch 🔼", "Priority raised"),
            (("-",), "- [ ] Order lunch 🔽", "Priority lowered"),
        ]
        for keys, line, label in cases:
            press(app, *keys)
            assert disk(root, B) == B_TEXT.replace(LUNCH, line), keys
            shown = notice(app)
            assert shown["text"] == f"{label} · wrote line 3 of {B}", keys
            assert shown["undo"] is True, shown
            # The message's Undo, then the key again and the undo chord:
            # each says what it undid, and where, with no Undo of its own.
            undone = f"Undone: {label} · wrote line 3 of {B}"
            app.client.call("tasks.entry", {"undo": True})
            assert disk(root, B) == B_TEXT, keys
            shown = notice(app)
            assert (shown["text"], shown["undo"]) == (undone, False), shown
            press(app, *keys)
            assert disk(root, B) == B_TEXT.replace(LUNCH, line), keys
            press(app, "z", "mod")
            assert disk(root, B) == B_TEXT, keys
            assert message(app) == undone, keys
        press(app, "z", "mod")
        assert message(app) == "Nothing to undo here."
        # The open note is written through its editor: its draft.
        select(app, "Book the room")
        press(app, "space")
        assert draft(app) == A_TEXT.replace("- [ ] Book the room",
                                            "- [x] Book the room ✅ 2026-11-19")
        assert message(app) == f"Done · wrote line 4 of {A}"
        press(app, "z", "mod")
        assert draft(app) == A_TEXT


def test_a_click_on_the_box_cycles_and_rows_stay_put_until_r(tmp_path: Path) -> None:
    with view_app(tmp_path, "cycle") as (app, root):
        # Only open work: a row cycled to doing, then done, stays where it
        # stood until R. Another filter, as R does, lets it go.
        listing = click(app, control="chip:open")
        order = descriptions(listing)
        lunch = row_of(listing, "Order lunch")
        for line, label in [("- [/] Order lunch", "Marked doing"),
                            ("- [x] Order lunch ✅ 2026-11-19", "Marked done")]:
            listing = click(app, cell={"row": lunch, "cell": "box"})
            assert disk(root, B) == B_TEXT.replace(LUNCH, line)
            assert message(app).startswith(label)
            assert descriptions(listing) == order
            assert listing["rows"][lunch]["held"] is True
        listing = press(app, "r")
        assert "Order lunch" not in descriptions(listing)
        assert not any(row["held"] for row in listing["rows"])


def test_menus_by_click_and_by_key(tmp_path: Path) -> None:
    with view_app(tmp_path, "menus") as (app, root):
        lunch = select(app, "Order lunch")
        # A click on the priority: its menu, under the cell; a row clicked.
        listing = click(app, cell={"row": lunch, "cell": "priority"})
        assert listing["menu"]["for"] == "row_priority", listing["menu"]
        click(app, menu="High")
        assert disk(root, B) == B_TEXT.replace(LUNCH, "- [ ] Order lunch ⏫")
        assert message(app).startswith("High priority")
        # S, and the menu's own key.
        listing = press(app, "s")
        assert listing["menu"]["for"] == "row_status"
        assert [row["key"] for row in listing["menu"]["rows"]] == ["Space", "/", "X", "-"]
        listing = press(app, "/")
        assert listing["menu"] is None
        assert disk(root, B) == B_TEXT.replace(LUNCH, "- [/] Order lunch ⏫")
        # A right-click: the task menu, with the view's two rows.
        listing = click(app, right=lunch)
        menu = listing["menu"]
        assert menu["for"] == "row_task"
        assert [row["label"] for row in menu["rows"]][-2:] == [
            "Open the note at this line", "Hide tasks in B"]
        press(app, "x")
        assert disk(root, B) == B_TEXT.replace(LUNCH, "- [x] Order lunch ⏫ ✅ 2026-11-19")
        # Escape closes a menu and writes nothing.
        listing = press(app, "p")
        assert listing["menu"]["for"] == "row_priority"
        listing = press(app, "escape")
        assert listing["menu"] is None
        # Hide tasks in B, from the task menu, with Undo.
        click(app, right=lunch)
        listing = press(app, "h")
        assert disk(root, B).startswith("---\ntasks: off\n---\n")
        assert "Order lunch" not in descriptions(listing)
        assert message(app).startswith("Tasks in B hidden")
        app.client.call("tasks.entry", {"undo": True})
        assert "Order lunch" in descriptions(state(app))


def test_the_calendar_and_the_form_over_a_row(tmp_path: Path) -> None:
    with view_app(tmp_path, "dialogs") as (app, root):
        call = app.client.call
        select(app, "Order lunch")
        press(app, "d")
        calendar = call("tasks.calendar")
        assert calendar["open"] is True and calendar["selected"] == TODAY
        call("tasks.calendar_type", {"text": "fri"})
        call("tasks.calendar_click", {"target": "enter"})
        assert call("tasks.calendar")["open"] is False
        assert call("tasks.view")["keys"] is True
        assert disk(root, B) == B_TEXT.replace(LUNCH, "- [ ] Order lunch 📅 2026-11-20")
        assert message(app) == f"Due Fri 20 Nov · wrote line 3 of {B}"
        # The form: a description changed and saved.
        press(app, "e")
        assert call("tasks.form")["open"] is True
        call("tasks.form_type", {"field": "description", "text": "Order tea"})
        call("tasks.form_click", {"target": "save"})
        assert call("tasks.form")["open"] is False
        assert disk(root, B) == B_TEXT.replace(LUNCH, "- [ ] Order tea 📅 2026-11-20")
        assert message(app) == f"Saved · wrote line 3 of {B}"


def test_a_stale_row_is_refused_and_the_list_refreshed(tmp_path: Path) -> None:
    with view_app(tmp_path, "stale") as (app, root):
        select(app, "Order lunch")
        theirs = B_TEXT.replace("Order lunch", "Order dinner")
        (root / B).write_text(theirs, encoding="utf-8")
        listing = press(app, "space")
        assert disk(root, B) == theirs
        assert message(app) == CHANGED
        listing = wait_for(lambda: (lambda shown: shown if "Order dinner" in descriptions(shown)
                                    else None)(state(app)),
                           timeout=5, description="the list shows the note as it is")
        assert "Order lunch" not in descriptions(listing)


@pytest.mark.native_x11
def test_native_a_click_on_the_box_and_a_right_click_with_its_key(tmp_path: Path) -> None:
    with view_app(tmp_path, "native", focused=True) as (app, root):
        windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
        assert len(windows) == 1, windows
        window = windows[0]

        def drawn(description: str, cell: str) -> dict:
            def found():
                rows = state(app)["rows"]
                row = rows[row_of({"rows": rows}, description)]
                return row["cells"][cell] if cell != "row" else row["bounds"]
            return wait_for(found, timeout=5, description=f"the {cell} of {description} drawn")

        native.move(window, native.point_in(drawn("Order lunch", "box")))
        xdo("click", "1")
        wait_for(lambda: disk(root, B) == B_TEXT.replace(LUNCH, "- [/] Order lunch"),
                 timeout=5, description="a click on the box cycles it")
        native.move(window, native.point_in(drawn("Order lunch", "row"), vertical=0.3))
        xdo("click", "3")
        wait_for(lambda: (state(app)["menu"] or {}).get("for") == "row_task", timeout=5,
                 description="a right-click opens the task menu")
        native.keys(window, "x")
        wait_for(lambda: disk(root, B)
                 == B_TEXT.replace(LUNCH, "- [x] Order lunch ✅ 2026-11-19"),
                 timeout=5, description="the menu's key writes the row")
        assert state(app)["menu"] is None
        # The undo chord, while the view has the keyboard.
        native.keys(window, native_mod("z"))
        wait_for(lambda: disk(root, B) == B_TEXT.replace(LUNCH, "- [/] Order lunch"),
                 timeout=5, description="the undo chord undoes the write")


def describe(app: WrenApp) -> dict | None:
    return state(app)["describe"]


def test_the_description_edited_in_place(tmp_path: Path) -> None:
    with view_app(tmp_path, "describe") as (app, root):
        call = app.client.call
        lunch = select(app, "Order lunch")
        # F2: the description as written, in a field over the row.
        listing = press(app, "f2")
        assert listing["describe"]["row"] == lunch, listing["describe"]
        assert listing["describe"]["text"] == "Order lunch"
        # Tab keeps the keyboard in the field; typed keys are the field's.
        for key in ("tab", "space", "t", "e", "a"):
            press(app, key)
        assert describe(app)["text"] == "Order lunch tea"
        listing = press(app, "enter")
        assert listing["describe"] is None
        assert disk(root, B) == B_TEXT.replace("Order lunch", "Order lunch tea")
        shown = notice(app)
        assert shown["text"] == f"Description changed · wrote line 3 of {B}"
        assert shown["undo"] is True
        call("tasks.entry", {"undo": True})
        assert disk(root, B) == B_TEXT
        # The undo chord in the field undoes no task write.
        press(app, "space")
        assert "[x]" in disk(root, B)
        press(app, "f2")
        press(app, "z", "mod")
        assert describe(app) is not None
        assert "[x]" in disk(root, B)
        press(app, "escape")
        assert describe(app) is None
        press(app, "z", "mod")
        assert disk(root, B) == B_TEXT
        # A double-click on the open note's row: Escape writes nothing, and
        # the list keeps the keyboard.
        room = row_of(state(app), "Book the room")
        listing = click(app, describe=room)
        assert listing["describe"]["text"] == "Book the room"
        assert listing["rows"][room]["selected"]
        call("tasks.view", {"type": "Book the big room"})
        listing = press(app, "escape")
        assert listing["describe"] is None
        assert call("tasks.view")["keys"] is True
        assert draft(app) == A_TEXT
        # An empty description is refused and the field stays.
        click(app, describe=room)
        call("tasks.view", {"type": "  "})
        press(app, "enter")
        assert message(app) == "A task needs a description; nothing was written"
        assert describe(app) is not None
        call("tasks.view", {"type": "Book the big room"})
        press(app, "enter")
        assert draft(app) == A_TEXT.replace("Book the room", "Book the big room")
        assert message(app) == f"Description changed · wrote line 4 of {A}"
        # A click elsewhere ends the edit, writing nothing.
        click(app, describe=row_of(state(app), "Order lunch"))
        call("tasks.view", {"type": "Order cake"})
        listing = click(app, control="chip:open")
        assert listing["describe"] is None
        assert disk(root, B) == B_TEXT


def test_a_stale_description_edit_is_refused(tmp_path: Path) -> None:
    with view_app(tmp_path, "describe-stale") as (app, root):
        select(app, "Order lunch")
        press(app, "f2")
        theirs = B_TEXT.replace("Order lunch", "Order dinner")
        (root / B).write_text(theirs, encoding="utf-8")
        app.client.call("tasks.view", {"type": "Order tea"})
        press(app, "enter")
        # Either Enter finds the line changed and refuses, or the watcher's
        # rescan came first and closed the field, its task gone: nothing is
        # written either way.
        assert disk(root, B) == theirs
        assert message(app) == CHANGED or describe(app) is None
        wait_for(lambda: "Order dinner" in descriptions(state(app)), timeout=5,
                 description="the list shows the note as it is")


@pytest.mark.native_x11
def test_native_typing_in_the_description_field(tmp_path: Path) -> None:
    with view_app(tmp_path, "describe-native", focused=True) as (app, root):
        windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
        assert len(windows) == 1, windows
        window = windows[0]

        def drawn(description: str) -> dict:
            def found():
                rows = state(app)["rows"]
                return rows[row_of({"rows": rows}, description)]["cells"]["description"]
            return wait_for(found, timeout=5, description=f"the description of {description}")

        # F2 on the selected row, then typing, Enter: the closed note.
        select(app, "Order lunch")
        native.keys(window, "F2")
        wait_for(lambda: describe(app), timeout=5, description="F2 opens the field")
        native.settle_focus()
        native.type_text(window, " now")
        wait_for(lambda: (describe(app) or {}).get("text") == "Order lunch now", timeout=5,
                 description="typing reaches the field")
        native.keys(window, "Return")
        wait_for(lambda: disk(root, B) == B_TEXT.replace("Order lunch", "Order lunch now"),
                 timeout=5, description="Enter writes the description")
        assert describe(app) is None
        # A double-click on a description opens it there; Escape writes
        # nothing, and the list has the keyboard again: Space toggles.
        native.move(window, native.point_in(drawn("Book the room")))
        xdo("click", "--repeat", "2", "--delay", "60", "1")
        wait_for(lambda: (describe(app) or {}).get("text") == "Book the room", timeout=5,
                 description="a double-click opens the field")
        native.settle_focus()
        native.type_text(window, "xyz")
        wait_for(lambda: (describe(app) or {}).get("text") == "Book the roomxyz", timeout=5,
                 description="typing reaches the field")
        native.keys(window, "Escape")
        wait_for(lambda: describe(app) is None, timeout=5, description="Escape closes it")
        assert draft(app) == A_TEXT
        native.keys(window, "space")
        wait_for(lambda: "- [x] Book the room" in draft(app), timeout=5,
                 description="the list has the keyboard again")
