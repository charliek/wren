"""The macOS menu bar's Board menu (discovery/boards.md §5.19), through the
table the native menu is built from.

The menu itself exists only on macOS, but its rows, the shortcut each would
show and whether each is enabled come from code every platform runs:
``menu.get`` reports them, and ``menu.run`` sends a row's key back as
choosing the item does. On a Mac ``native`` also reports what AppKit holds.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, NOTES, board, board_app, click, copy_samples, fitted, open_note,
)
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.ui import WrenApp, wait_for

START = "ahj860xh"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def menus(app: WrenApp) -> dict[str, list[dict]]:
    reply = app.client.call("menu.get")
    return {menu["title"]: menu["items"] for menu in reply["menus"]}


def commands(items: list[dict]) -> list[dict]:
    """The command items, a submenu's included, in order."""
    found = []
    for item in items:
        if "key" in item:
            found.append(item)
        elif "items" in item:
            found.extend(commands(item["items"]))
    return found


def enabled(app: WrenApp) -> list[str]:
    reply = app.client.call("menu.get")
    if reply["native"] is not None:
        board_menu = next(menu for menu in reply["menus"] if menu["kind"] == "board")
        assert reply["native"] == [{"key": item["key"], "enabled": item["enabled"]}
                                   for item in commands(board_menu["items"])]
    return [item["key"] for item in commands(menus(app)["Board"]) if item["enabled"]]


def test_the_board_menu_follows_the_open_note_and_runs_through_the_keys_path(samples: Path) -> None:
    with board_app(samples, "board-menu") as app:
        bar = menus(app)
        assert list(bar) == ["Wren", "Go", "View", "Board", "Window"]
        outline = next(item for item in commands(bar["Go"]) if item["key"] == "navigation.outline")
        assert outline["title"] == "Jump in Note…"
        align = next(item for item in bar["Board"] if item.get("submenu") == "Align")
        assert [item["key"] for item in align["items"] if "key" in item] == [
            f"board.align.{to}" for to in ("left", "center", "right", "top", "middle", "bottom")]
        # A Board item shows only a Command-key trigger; Linux binds none.
        assert all(item["trigger"] is None or "super" in item["trigger"]
                   for item in commands(bar["Board"]))

        open_note(app, NOTES)
        wait_for(lambda: enabled(app) == ["note.new_board"], timeout=10,
                 description="only New Board on a Markdown note")

        open_note(app, FLOW)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        fitted(app)
        shown = enabled(app)
        assert {"note.new_board", "note.export_svg", "note.export_png",
                "board.select.connected"} <= set(shown), shown
        assert "board.duplicate" not in shown and "board.align.left" not in shown

        # Go > Jump in Note… on a board opens the jump to a node.
        app.client.call("menu.run", {"key": "navigation.outline"})
        assert app.client.call("navigation.get")["modal"]["mode"] == "board_nodes"
        # The palette has the keyboard: a menu item does nothing meanwhile.
        app.client.call("menu.run", {"key": "board.select.connected"})
        assert board(app)["selection"] == []
        app.client.call("palette.close")

        click(app, {"node": START})
        assert board(app)["selection"] == [START]
        assert "board.duplicate" in enabled(app)
        before = len(board(app, elements=True)["nodes"])
        app.client.call("menu.run", {"key": "board.duplicate"})
        state = board(app, elements=True)
        assert len(state["nodes"]) == before + 1
        assert app.client.call("session.get")["document"]["dirty"] is True


def test_with_the_setting_off_no_menu_reaches_a_board_command(samples: Path) -> None:
    with board_app(samples, "board-menu-off", setting="off") as app:
        bar = menus(app)
        assert list(bar) == ["Wren", "Go", "View", "Window"]
        keys = [item["key"] for items in bar.values() for item in commands(items)]
        assert not [key for key in keys
                    if key.startswith("board.") or key in ("note.new_board", "note.export_svg",
                                                           "note.export_png")]
        assert app.client.call("menu.get")["native"] in (None, [])
