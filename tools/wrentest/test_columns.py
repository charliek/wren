"""Keyboard drill-down: the active column, arrows, Enter, Escape, column moves,
and click semantics (discovery/keyboard-drill-down.md).

The gateway cases drive the production key and row-click handlers over the
test socket (`key.nav` sends a navigation key no widget captured; `row.click`
sends the drag layer's own click events). The native_x11 cases press real keys
and click real rows, which also proves that keys a text widget captured stay
with that widget.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest import test_gestures as native
from tools.wrentest.test_editor import capture
from tools.wrentest.test_navigation import (
    FIRST, append, choose_theme, color_count, command, item, jump, navigation, session,
    vault, vault_factory,  # noqa: F401  (fixtures)
)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

native_workspace = native.native_workspace

ACCENT = {"dark": (0xF0, 0xB4, 0x5A), "light": (0x9A, 0x58, 0x00)}


def nav(app: WrenApp, key: str) -> dict:
    return app.client.call("key.nav", {"key": key})


def click(app: WrenApp, row: dict, *, double: bool = False) -> dict:
    return app.client.call("row.click", {"row": row, "double": double})


def column(app: WrenApp) -> str:
    return navigation(app)["column"]


def focused(app: WrenApp) -> list[str]:
    return app.client.call("focus.get")["named"]


def wait_focus(app: WrenApp, names: list[str]) -> None:
    wait_for(lambda: focused(app) == names, timeout=3, interval=0.025,
             description=f"focused widgets {names}")


def document_path(app: WrenApp) -> str | None:
    document = session(app)["document"]
    return document and document["path"]


def test_keyboard_walks_columns_with_live_selection_and_flushes(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("columns-walk")) as app:
        jump(app, FIRST)
        assert column(app) == "editor"
        expected = append(app, "drill-down edit\n")
        for name in ("notes", "groups", "sections"):
            command(app, "navigation.column_left")
            assert column(app) == name
        command(app, "navigation.column_left", disposition="blocked")
        assert column(app) == "sections"
        wait_focus(app, [])

        # ↓ selects Boreal live: its first group and note open, and the
        # outgoing draft is saved first.
        state = nav(app, "down")
        assert state["column"] == "sections"
        assert state["selected_section"] == item(app, "sections", "Boreal")["id"]
        assert document_path(app).startswith("Boreal/")
        assert (vault / FIRST).read_text(encoding="utf-8") == expected, "the switch flushed first"
        nav(app, "up")
        assert navigation(app)["selected_section"] == item(app, "sections", "Atlas")["id"]

        assert nav(app, "enter")["column"] == "groups"
        research = item(app, "groups", "Atlas/Research")["id"]
        while navigation(app)["selected_group"] != research:
            before = navigation(app)["selected_group"]
            assert nav(app, "down")["selected_group"] != before, "↓ stopped before Research"
        assert document_path(app).startswith("Atlas/Research/")

        assert nav(app, "right")["column"] == "notes"
        opened = document_path(app)
        nav(app, "down")
        assert column(app) == "notes"
        assert document_path(app) != opened and document_path(app).startswith("Atlas/Research/")
        wait_focus(app, [])
        assert nav(app, "left")["column"] == "groups"
        assert nav(app, "right")["column"] == "notes"

        assert nav(app, "enter")["column"] == "editor"
        wait_focus(app, ["note-editor"])
        command(app, "navigation.column_right", disposition="blocked")


def test_escape_returns_to_the_editor_and_overlays_keep_the_column(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("columns-escape")) as app:
        note = jump(app, FIRST)
        command(app, "navigation.column_left")
        command(app, "navigation.column_left")
        app.client.call("palette.open", {"mode": "commands"})
        nav(app, "down")
        assert column(app) == "groups", "arrows move the palette, not the column"
        nav(app, "escape")
        assert navigation(app)["modal"] is None and column(app) == "groups"
        state = nav(app, "escape")
        assert state["column"] == "editor" and state["selected_note"] == note
        wait_focus(app, ["note-editor"])
        for key in ("down", "up", "left", "right", "enter"):
            assert nav(app, key)["selected_note"] == note, key
        assert column(app) == "editor"


def test_click_selects_like_an_arrow_and_double_click_moves_right(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("columns-clicks")) as app:
        jump(app, FIRST)
        boreal = item(app, "sections", "Boreal")["id"]
        state = click(app, {"section": boreal})
        assert state["column"] == "sections" and state["selected_section"] == boreal
        assert state["dialog"] is None

        workbench = item(app, "groups", "Boreal/Workbench")["id"]
        state = click(app, {"group": workbench}, double=True)
        assert state["selected_group"] == workbench and state["column"] == "notes"
        assert state["dialog"] is None, "double-click moves right; rename is Cmd/Alt+R"

        loose = item(app, "notes", "Boreal/Workbench/Loose 5.md")["id"]
        state = click(app, {"note": loose})
        assert state["column"] == "notes" and document_path(app) == "Boreal/Workbench/Loose 5.md"
        wait_focus(app, [])
        state = click(app, {"note": loose}, double=True)
        assert state["column"] == "editor"
        wait_focus(app, ["note-editor"])


def test_moving_right_needs_a_selection(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("columns-empty")) as app:
        jump(app, FIRST)
        empty = item(app, "groups", "Atlas/Empty group")["id"]
        click(app, {"group": empty})
        assert navigation(app)["selected_note"] is None
        command(app, "navigation.column_right")
        assert column(app) == "notes"
        command(app, "navigation.column_right", disposition="blocked")
        assert nav(app, "enter")["column"] == "notes"
        assert nav(app, "down")["selected_note"] is None


def test_jump_and_new_note_land_in_the_editor(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("columns-landing")) as app:
        jump(app, FIRST)
        command(app, "navigation.column_left")
        jump(app, "Boreal/Workbench/Loose 5.md")
        assert column(app) == "editor"
        command(app, "navigation.column_left")
        command(app, "note.new_dated")
        assert column(app) == "editor"
        wait_focus(app, ["note-editor"])


@pytest.mark.parametrize("theme", ["dark", "light"])
def test_active_column_header_is_accented_in_each_state(vault: Path, theme: str) -> None:
    artifacts = artifact_directory(f"columns-states-{theme}")
    with WrenApp(workspace=vault, artifacts_dir=artifacts) as app:
        jump(app, FIRST)
        choose_theme(app, theme)
        for _ in range(3):
            command(app, "navigation.column_left")
        panes = ("sections", "groups", "notes")
        for active in (*panes, "editor"):
            current = navigation(app)
            assert current["column"] == active
            image = capture(app, artifacts / f"active-{active}.png")
            viewport = current["viewport"]
            left = 0.0
            shown = [pane for pane in panes if current["effective_widths"][pane] > 0]
            assert shown == list(panes[current["fold"]:])
            for pane in panes:
                width = current["effective_widths"][pane]
                # The header band: below the 48 px title bar (and the 41 px
                # breadcrumb, on the leftmost pane while any are folded).
                top = 52 + (41 if current["fold"] and pane == shown[0] else 0)
                accent = color_count(image, viewport, (left + 4, top, left + width - 40, top + 40),
                                     ACCENT[theme])
                if pane == active:
                    assert accent > 20, (active, pane, accent)
                else:
                    assert accent == 0, (active, pane, accent)
                left += width
            if active != "editor":
                command(app, "navigation.column_right")


def rendered_kinds(app: WrenApp) -> set[str]:
    rows = app.client.call("navigation.bounds")["rows"]
    return {kind for row in rows for kind in row["key"]}


def test_folding_follows_keyboard_moves_and_breadcrumbs_unfold(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("columns-fold")) as app:
        jump(app, FIRST)
        for _ in range(3):
            command(app, "navigation.column_left")
        state = navigation(app)
        assert (state["column"], state["fold"]) == ("sections", 0), "moving left never folds"
        widths = state["preferred_widths"]

        state = nav(app, "enter")
        assert (state["column"], state["fold"]) == ("groups", 1)
        assert state["effective_widths"]["sections"] == 0
        wait_for(lambda: rendered_kinds(app) == {"group", "note"}, timeout=5,
                 description="Sections folds out of the rendered browser")
        state = nav(app, "enter")
        assert (state["column"], state["fold"]) == ("notes", 2)
        assert state["effective_widths"] == {"sections": 0, "groups": 0, "notes": widths["notes"]}
        wait_for(lambda: rendered_kinds(app) == {"note"}, timeout=5, description="only Notes remains")
        assert nav(app, "enter")["fold"] == 2, "Notes never folds"
        assert navigation(app)["preferred_widths"] == widths, "folding never changes widths"

        # Esc from a browser column folds; Esc in the editor never does.
        state = app.client.call("column.open", {"column": "groups"})
        assert (state["navigation"]["column"], state["navigation"]["fold"]) == ("groups", 1)
        assert nav(app, "escape")["fold"] == 2
        state = app.client.call("column.open", {"column": "sections"})["navigation"]
        assert (state["column"], state["fold"]) == ("sections", 0)
        wait_for(lambda: rendered_kinds(app) == {"section", "group", "note"}, timeout=5,
                 description="every column is back")
        command(app, "navigation.column_right")
        command(app, "navigation.column_left")
        assert navigation(app)["fold"] == 0
        nav(app, "escape")
        assert nav(app, "escape")["fold"] == 2

        # Clicks never move the layout; a restart starts unfolded.
        note = item(app, "notes", "Atlas/Research/Note 2.md")["id"]
        assert click(app, {"note": note})["fold"] == 2
        app.restart(hard=False)
        assert (navigation(app)["column"], navigation(app)["fold"]) == ("editor", 0)


@pytest.mark.native_x11
def test_native_keys_walk_columns_and_editor_keeps_its_arrows(native_workspace) -> None:
    app, root, window = native_workspace
    original_files = native.files(root)
    origin = native.item(app, "notes", native.ORIGIN)["id"]
    assert navigation(app)["selected_note"] == origin
    wait_focus(app, ["note-editor"])
    native.keys(window, "Down", "Up", "Return", "Left")
    native.settle_focus()
    # The editor captured those keys: no column move, no selection change.
    assert navigation(app)["selected_note"] == origin and column(app) == "editor"
    assert "\n" in app.client.call("editor.get")["text"]

    native.keys(window, "alt+shift+bracketleft")
    wait_for(lambda: column(app) == "notes", description="Alt+Shift+[ moves to Notes")
    wait_focus(app, [])
    native.keys(window, "Down")
    wait_for(lambda: navigation(app)["selected_note"] != origin,
             description="native Down selects the next note")
    assert column(app) == "notes"
    native.keys(window, "Left")
    wait_for(lambda: column(app) == "groups", description="native Left moves to Groups")
    native.keys(window, "Return")
    wait_for(lambda: column(app) == "notes", description="native Enter moves right")
    native.keys(window, "alt+shift+bracketright")
    wait_for(lambda: column(app) == "editor", description="Alt+Shift+] moves into the editor")
    wait_focus(app, ["note-editor"])
    native.type_text(window, "typed after drill")
    wait_for(lambda: "typed after drill" in app.client.call("editor.get")["text"],
             description="typing reaches the editor after moving right")
    native.keys(window, "alt+shift+bracketleft", "Escape")
    wait_for(lambda: column(app) == "editor", description="Escape returns to the editor")
    assert {path for path in native.files(root)} == set(original_files)


@pytest.mark.native_x11
def test_native_double_click_moves_right_and_alt_r_renames(native_workspace) -> None:
    app, root, window = native_workspace
    remote = native.item(app, "sections", native.SECTIONS[1])["id"]
    native.move(window, native.point_in(native.visible_row(app, {"section": remote})["visible"]))
    native.xdo("click", "--repeat", "2", "--delay", "80", "1")
    wait_for(lambda: navigation(app)["selected_section"] == remote and column(app) == "groups",
             description="native double-click selects the section and moves right")
    assert navigation(app)["dialog"] is None
    native.keys(window, "alt+r")
    wait_for(lambda: navigation(app)["dialog"] is not None, description="Alt+R opens rename")
    assert navigation(app)["dialog"]["kind"].startswith("Rename")


@pytest.mark.native_x11
def test_native_breadcrumb_back_button_unfolds_one_column_at_a_time(native_workspace) -> None:
    app, root, window = native_workspace
    native.keys(window, "alt+shift+bracketleft", "Escape")
    wait_for(lambda: (navigation(app)["column"], navigation(app)["fold"]) == ("editor", 2),
             description="Esc from Notes folds Sections and Groups")
    area = native.bounds(app)["root"]
    back = (round(area["x"] + 8 + 13), round(area["y"] + 20))
    for expected in (("groups", 1), ("sections", 0)):
        native.move(window, back)
        native.xdo("click", "1")
        wait_for(lambda: (navigation(app)["column"], navigation(app)["fold"]) == expected,
                 description=f"breadcrumb back opens {expected[0]}")
    assert navigation(app)["dialog"] is None and navigation(app)["modal"] is None
