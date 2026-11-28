"""The task view's layout switch (discovery/tasks-build.md item 12.1;
discovery/tasks.md §2 item 36, §11): List or Board, kept for each scope in
``.wren/tasks.toml`` beside its filters and written only for the board.

The List | Board switch in the view's header, ``B`` while the list or the
board has the keyboard, and the App command ``tasks.toggle_board`` (Toggle
task board, in the palette's View group and the macOS View menu) each flip
it. A switch is no stop in Back and Forward. On the board the status chips,
Show all and Group by are hidden; its four columns are drawn empty until
item 12.2 fills them.

``tasks.view`` with ``"list": true`` reports ``layout`` (``"list"`` or
``"board"``) and ``board`` (its ``columns``, ``null`` on the list);
``{"click": {"layout": …}}`` clicks a segment of the switch.

The board is drawn by the view, not the editor, so every case runs on the
text engines ``WRENTEST_ENGINE`` names, and the tasks-off case on classic
too; each writes its own ``tasks`` line whatever the lane's
``WRENTEST_TASKS`` says, asserts the settings the app runs with, and pins
the design page's day.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import engine, selected_engines, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_gestures import move
from tools.wrentest.test_navigation import activate, command, core, gateway, item, navigation, \
    palette, session
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_view_list import ROADMAP, TODAY, click, press, show, state, \
    tasks_toml, workspace
from tools.wrentest.ui import WrenApp, wait_for

SETTINGS = "tasks = {tasks}\nboards = on\ntasks-me = sam, sam.lee\neditor-engine = {engine}\n"
COLUMNS = [{"status": "open", "label": "Open"}, {"status": "doing", "label": "Doing"},
           {"status": "done", "label": "Done"}, {"status": "cancelled", "label": "Cancelled"}]
LIST_ONLY = {"chip:open", "chip:doing", "chip:done", "chip:cancelled", "show_all", "group"}
BOARD_FILE = 'schema_version = 1\n\n[views."."]\nlayout = "board"\n'


@contextmanager
def board_app(tmp_path: Path, case: str, engine: str, tasks: str = "on") -> Iterator[WrenApp]:
    """The M11 fixture on `engine`, with `tasks` as given, the settings
    asserted as the app runs with them."""
    with WrenApp(workspace=workspace(tmp_path), evaluation=False,
                 settings=SETTINGS.format(tasks=tasks, engine=engine), today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-board-{case}-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        if tasks == "on":
            got = app.client.call("tasks.get")
            assert got["tasks"] is True and got["today"] == TODAY, got
        else:
            assert app.client.call("tasks.get") == {"tasks": False}
        yield app


def state_file(root: Path) -> bytes | None:
    path = root / ".wren" / "workspace.toml"
    return path.read_bytes() if path.exists() else None


def open_note(app: WrenApp, path: str) -> None:
    core(app, "open_note", note=item(app, "notes", path)["id"])
    assert session(app)["document"]["path"] == path


def drawn(app: WrenApp, *present: str) -> set[str]:
    """The controls drawn, once a frame has drawn every one of *present*."""
    return wait_for(lambda: (lambda names: names if set(present) <= names else None)(
        set(state(app)["bounds"])), timeout=5, description=f"{present} drawn")


def view_menu_item(app: WrenApp) -> dict:
    """Toggle Task Board's item in the View menu, as the menu shows it."""
    menus = app.client.call("menu.get")["menus"]
    items = next(menu["items"] for menu in menus if menu["title"] == "View")
    return next(entry for entry in items if entry.get("key") == "tasks.toggle_board")


def steps(app: WrenApp) -> tuple[bool, bool]:
    nav = navigation(app)
    return nav["can_back"], nav["can_forward"]


def test_the_switch_shows_the_board_and_only_the_board_is_written(
        tmp_path: Path, engine: str) -> None:
    with board_app(tmp_path, "switch", engine) as app:
        root = workspace(tmp_path)
        listing = show(app)
        assert (listing["layout"], listing["board"]) == ("list", None)
        drawn(app, "layout", *(LIST_ONLY - {"show_all"}))
        assert tasks_toml(root) is None

        listing = click(app, layout="board")
        assert listing["layout"] == "board"
        assert listing["board"] == {"columns": COLUMNS}
        assert listing["focus"] == "list", "a click leaves the keyboard with the body"
        assert listing["hint"] == "", "no list key is named on the board"
        assert tasks_toml(root) == BOARD_FILE
        # Only what the board draws is drawn: no chip, Show all or Group by.
        names = drawn(app, "scope", "layout", "owner", "due", "priority", "hide_old")
        assert not LIST_ONLY & names, names
        # Its filters stay, and still write as a filter change does.
        listing = click(app, control="hide_old")
        assert listing["filters"]["hide_old"] is False and listing["layout"] == "board"
        assert tasks_toml(root) == (
            'schema_version = 1\n\n[views."."]\nhide_old = false\nlayout = "board"\n')
        listing = click(app, control="hide_old")
        assert tasks_toml(root) == BOARD_FILE

        # The pressed segment changes nothing; the other one goes back.
        assert click(app, layout="board")["layout"] == "board"
        listing = click(app, layout="list")
        assert listing["layout"] == "list" and listing["board"] is None
        assert listing["hint"].startswith("Space toggle")
        assert tasks_toml(root) == "schema_version = 1\n", "the scope's layout is gone"
        assert listing["shown"] == 12 and listing["chips"][0]["count"] == 7


def test_b_and_the_switch_by_keys_flip_it_and_history_never_moves(
        tmp_path: Path, engine: str) -> None:
    with board_app(tmp_path, "keys", engine) as app:
        root = workspace(tmp_path)
        open_note(app, ROADMAP)
        listing = show(app)
        before = steps(app)
        assert before == (True, False)
        written = state_file(root)
        chosen = [row["description"] for row in listing["rows"] if row["selected"]]

        listing = press(app, "b")
        assert listing["layout"] == "board"
        assert steps(app) == before
        listing = press(app, "B", "shift")
        assert listing["layout"] == "list"
        assert [row["description"] for row in listing["rows"] if row["selected"]] == chosen
        assert steps(app) == before
        # Tab: the scope chip, then the switch, where Enter and Space flip it.
        assert press(app, "tab")["focus"] == "scope"
        listing = press(app, "tab")
        assert listing["focus"] == "layout"
        listing = press(app, "enter")
        assert (listing["layout"], listing["focus"]) == ("board", "layout")
        # On the board Tab walks no chip and no Group by.
        walked = [press(app, "tab")["focus"] for _ in range(5)]
        assert walked == ["owner", "due", "priority", "hide_old", "list"]
        # B on a control is the control's: nothing.
        assert press(app, "tab")["focus"] == "scope"
        assert press(app, "b")["layout"] == "board"
        assert press(app, "escape")["focus"] == "list"
        listing = press(app, "b")
        assert listing["layout"] == "list"
        assert steps(app) == before
        assert tasks_toml(root) == "schema_version = 1\n"
        assert state_file(root) == written, "the state file holds no layout"

        # Back leaves the view; Forward shows it in the layout it was left in.
        press(app, "b")
        command(app, "navigation.back")
        assert navigation(app)["surface"] == "text"
        assert session(app)["document"]["path"] == ROADMAP
        command(app, "navigation.forward")
        assert navigation(app)["surface"] == "tasks"
        assert state(app)["layout"] == "board"
        assert steps(app) == before


def test_the_palette_command_flips_it_only_while_a_view_shows(
        tmp_path: Path, engine: str) -> None:
    with board_app(tmp_path, "command", engine) as app:
        root = workspace(tmp_path)
        open_note(app, ROADMAP)
        rows = palette(app, "commands", "task board")["results"]
        row = next(row for row in rows if row["key"] == {"command": "tasks.toggle_board"})
        assert (row["title"], row["group"], row["enabled"]) == ("Toggle task board", "View", False)
        assert row["shortcut"] in (None, ""), row
        gateway(app, "palette.close")
        command(app, "tasks.toggle_board", disposition="blocked")
        assert tasks_toml(root) is None
        assert view_menu_item(app)["enabled"] is False, "greyed with no view"

        show(app)
        before = steps(app)
        rows = palette(app, "commands", "task board")["results"]
        row = next(row for row in rows if row["key"] == {"command": "tasks.toggle_board"})
        assert row["enabled"] is True
        activate(app, {"command": "tasks.toggle_board"})
        assert navigation(app)["modal"] is None
        view = app.client.call("tasks.view", {"list": True})
        assert view["keys"] and view["list"]["layout"] == "board"
        assert tasks_toml(root) == BOARD_FILE
        command(app, "tasks.toggle_board")
        assert state(app)["layout"] == "list"
        assert steps(app) == before
        # The macOS View menu lists it after Toggle task counts, enabled
        # while the view shows.
        menus = app.client.call("menu.get")["menus"]
        view_menu = next(menu for menu in menus if menu["title"] == "View")
        keys = [entry.get("key") for entry in view_menu["items"]]
        at = keys.index("view.toggle_task_counts")
        assert keys[at + 1] == "tasks.toggle_board", keys
        assert view_menu_item(app)["enabled"] is True
        # Chosen from the menu while a filter's menu is open over the view,
        # it switches and closes that menu; Group by, hidden, takes no click.
        assert click(app, control="group")["menu"]["for"] == "group"
        app.client.call("menu.run", {"key": "tasks.toggle_board"})
        listing = state(app)
        assert listing["layout"] == "board" and listing["menu"] is None, listing["menu"]
        listing = click(app, control="group")
        assert listing["menu"] is None and listing["filters"]["group"] == "note"
        command(app, "tasks.toggle_board")
        open_note(app, ROADMAP)
        assert view_menu_item(app)["enabled"] is False


def test_each_scope_keeps_its_layout_across_a_restart(tmp_path: Path, engine: str) -> None:
    with board_app(tmp_path, "restart", engine) as app:
        root = workspace(tmp_path)
        show(app)
        click(app, layout="board")
        listing = show(app, "group", "Work/Planning")
        assert listing["layout"] == "list"
        assert tasks_toml(root) == BOARD_FILE

        app.restart(hard=False)
        assert show(app)["layout"] == "board"
        assert show(app, "group", "Work/Planning")["layout"] == "list"
        assert show(app, "section", "Work")["layout"] == "list"
        assert tasks_toml(root) == BOARD_FILE


@pytest.mark.parametrize("off_engine", sorted({"classic", *selected_engines()}))
def test_with_tasks_off_there_is_no_command_key_or_file(tmp_path: Path, off_engine: str) -> None:
    root = workspace(tmp_path)
    with board_app(tmp_path, "off", off_engine, tasks="off") as app:
        rows = palette(app, "commands", "")["results"]
        assert "tasks.toggle_board" not in {row["key"].get("command") for row in rows}
        gateway(app, "palette.close")
        command(app, "tasks.toggle_board", disposition="blocked")
        with pytest.raises(RemoteError):
            app.client.call("shortcut", {"id": "tasks.toggle_board"})
        with pytest.raises(RemoteError, match="Tasks are off"):
            app.client.call("tasks.view", {"click": {"layout": "board"}})
        menus = app.client.call("menu.get")["menus"]
        assert not [entry for menu in menus for entry in menu["items"]
                    if entry.get("key") == "tasks.toggle_board"]
        assert not (root / ".wren" / "tasks.toml").exists()


@pytest.mark.native_x11
def test_native_b_and_a_click_on_each_segment(tmp_path: Path, engine: str) -> None:
    with board_app(tmp_path, "native", engine) as app:
        windows = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()
        assert len(windows) == 1, windows
        window = windows[0]
        xdo("windowfocus", "--sync", window)
        show(app)
        xdo("key", "b")
        wait_for(lambda: state(app)["layout"] == "board", timeout=5,
                 description="B shows the board")
        xdo("key", "shift+b")
        wait_for(lambda: state(app)["layout"] == "list", timeout=5,
                 description="Shift+B shows the list")

        def segment(fraction: float) -> tuple[int, int]:
            drawn(app, "layout")
            at = state(app)["bounds"]["layout"]
            return round(at["x"] + at["width"] * fraction), round(at["y"] + at["height"] / 2)

        move(window, segment(0.8))
        xdo("click", "1")
        wait_for(lambda: state(app)["layout"] == "board", timeout=5,
                 description="a click on Board shows the board")
        move(window, segment(0.2))
        xdo("click", "1")
        wait_for(lambda: state(app)["layout"] == "list", timeout=5,
                 description="a click on List shows the list")
        assert tasks_toml(workspace(tmp_path)) == "schema_version = 1\n"
