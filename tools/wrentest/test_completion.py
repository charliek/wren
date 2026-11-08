"""Acceptance for review fixes and direct desktop actions, on disposable data."""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.wrentest import png
from tools.wrentest import test_gestures as gestures
from tools.wrentest import test_navigation as nav
from tools.wrentest.test_gestures import native_workspace
from tools.wrentest.test_navigation import vault, vault_factory
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for
from tools.wrentest.x11 import read_pixels


@pytest.mark.parametrize("mode", ["commands", "jump", "search", "workspaces"])
@pytest.mark.parametrize("query,reason", [("é" * 2049, "4,096"), ("a " * 129, "128")], ids=["byte-budget", "word-budget"])
def test_oversized_palette_queries_are_visible_rejections_without_state_loss(vault, mode, query, reason):
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("query-budget")) as app:
        nav.jump(app, nav.FIRST)
        before = nav.editor_identity(app.client.call("editor.get"))
        nav.palette(app, mode, "note")
        result = nav.gateway(app, "palette.query", query=query, disposition="blocked")
        assert reason in result["warning"]
        assert result["navigation"]["modal"]["query"] == "note"
        assert reason in result["navigation"]["modal"]["error"]
        assert nav.editor_identity(app.client.call("editor.get")) == before
        current = nav.gateway(app, "palette.query", query="")
        assert current["navigation"]["modal"]["error"] is None


@pytest.mark.parametrize("pane", ["sections", "groups", "notes"])
def test_oversized_pane_filters_leave_previous_projection_and_document_intact(vault, pane):
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("filter-budget")) as app:
        nav.jump(app, nav.FIRST)
        nav.gateway(app, "filter.set", pane=pane, query="a")
        before = nav.navigation(app)
        document = nav.session(app)["document"]
        result = nav.gateway(app, "filter.set", pane=pane, query="x" * 4097, disposition="blocked")
        assert "4,096" in result["warning"]
        assert result["navigation"]["filters"] == before["filters"]
        assert result["navigation"]["visible"] == before["visible"]
        assert nav.session(app)["document"] == document


def test_registered_picker_rejects_replaced_ancestors_before_a_save_barrier(vault_factory, tmp_path):
    original = vault_factory("original/work")
    foreign = vault_factory("foreign/work")
    with WrenApp(evaluation=False, artifacts_dir=artifact_directory("registry-replacement")) as app:
        response = nav.gateway(app, "workspace.add", path=str(original), disposition="async-pending")
        nav.settle_workspace(app, response, original)
        response = nav.gateway(app, "workspace.add", path=str(foreign), disposition="async-pending")
        nav.settle_workspace(app, response, foreign)
        nav.jump(app, nav.FIRST)
        before = nav.navigation(app)
        baseline = (foreign / nav.FIRST).read_bytes()
        original.parent.rename(tmp_path / "original-retained")
        original.parent.symlink_to(foreign.parent, target_is_directory=True)
        with nav.unwritable((foreign / nav.FIRST).parent):
            app.client.call("editor.action", {"kind": "move", "motion": "end"})
            expected = app.client.call("editor.insert", {"text": " retained unsaved draft"})["text"]
            nav.palette(app, "workspaces")
            result = nav.activate(app, {"workspace": str(original)}, disposition="blocked")
            assert "symlink" in result["warning"] or "unavailable" in result["warning"]
            after = nav.navigation(app)
            assert after["generation"] == before["generation"]
            assert after["active_registration"] == str(foreign)
            assert after["registrations"] == before["registrations"]
            assert after["modal"]["mode"] == "workspaces"
            assert nav.session(app)["document"]["text"] == expected
            assert nav.session(app)["document"]["dirty"] is True
            assert (foreign / nav.FIRST).read_bytes() == baseline
        nav.gateway(app, "palette.close")
        nav.core(app, "flush")
        assert (foreign / nav.FIRST).read_text() == expected
        assert (tmp_path / "original-retained/work" / nav.FIRST).read_bytes() != expected.encode()


def test_normal_startup_does_not_follow_a_redirected_registered_root(vault_factory, tmp_path):
    original = vault_factory("original/work")
    foreign = vault_factory("foreign/work")
    with WrenApp(evaluation=False, artifacts_dir=artifact_directory("startup-replacement")) as app:
        response = nav.gateway(app, "workspace.add", path=str(original), disposition="async-pending")
        nav.settle_workspace(app, response, original)
        config = Path(nav.navigation(app)["config_directory"]) / "config.toml"
        before = config.read_bytes()
        original.parent.rename(tmp_path / "original-retained")
        original.parent.symlink_to(foreign.parent, target_is_directory=True)
        app.restart(hard=True)
        current = nav.navigation(app)
        assert current["root"] is None
        assert current["registrations"][0]["path"] == str(original)
        assert config.read_bytes() == before
        assert nav.session(app)["notes"] == []
        assert "unavailable" in (current["registry_warning"] or nav.session(app)["banner"])
        response = nav.gateway(app, "workspace.add", path=str(foreign), disposition="async-pending")
        nav.settle_workspace(app, response, foreign, disposition="committed-warning")
        assert "unavailable" in nav.navigation(app)["processed"]["warning"]
        assert nav.session(app)["document"] is not None


def test_passive_result_removal_reveals_the_new_active_palette_row(vault):
    for index in range(100):
        (vault / f"Atlas/Research/Scrollable {index:03}.md").write_text(f"Body {index}\n")
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("passive-palette-reveal")) as app:
        modal = nav.palette(app, "jump", "Scrollable")
        assert len(modal["results"]) == 100
        current = nav.gateway(app, "palette.step", delta=90)["navigation"]["modal"]
        assert current["active_index"] == 90
        wait_for(lambda: app.client.call("palette.bounds")["fully_visible"], description="scrolled active result")
        (vault / current["results"][90]["path"]).unlink()

        def refreshed():
            current = nav.navigation(app)["modal"]
            return current if len(current["results"]) == 99 and current["active_index"] == 0 else None
        current = wait_for(refreshed, timeout=5, description="external removal resets selection")
        geometry = wait_for(lambda: (bounds if (bounds := app.client.call("palette.bounds"))
                                    and bounds["fully_visible"] else None),
                            timeout=5, description="passively refreshed active row is actually visible")
        assert geometry["row"]["y"] - geometry["translation_y"] == pytest.approx(geometry["viewport"]["y"])
        nav.gateway(app, "palette.activate")
        assert nav.session(app)["document"]["path"] == current["results"][0]["path"]


@pytest.mark.parametrize("kind", ["section", "group"])
@pytest.mark.parametrize("metadata_failure", [False, True])
def test_palette_reordering_uses_full_siblings_and_persists_after_optional_warning(vault, kind, metadata_failure):
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("palette-reorder")) as app:
        nav.jump(app, nav.FIRST)
        original = gestures.files(vault)
        document = nav.session(app)["document"]
        if kind == "section":
            source = nav.item(app, "sections", "Empty section")["id"]
            before = nav.item(app, "sections", "Boreal")["id"]
            nav.gateway(app, "filter.set", pane="sections", query="brl")
            expected = ["Atlas", "Empty section", "Boreal"]
        else:
            source = nav.item(app, "groups", "Atlas/Review")["id"]
            before = nav.item(app, "groups", "Atlas/Research")["id"]
            nav.gateway(app, "filter.set", pane="groups", query="rsrch")
            expected = ["Atlas/Empty group", "Atlas/Review", "Atlas/Research", "Atlas/Workbench"]
        modal = nav.gateway(app, "palette.open", mode="reorder", target={kind: source})["navigation"]["modal"]
        assert modal["source"]["item"] == {kind: source}
        index = next(index for index, row in enumerate(modal["results"])
                     if (row["key"]["reorder"]["before"] or {}).get("item") == {kind: before})
        if metadata_failure:
            with nav.unwritable(vault / ".wren"):
                nav.gateway(app, "palette.activate", index=index, disposition="committed-warning")
                assert nav.navigation(app)["modal"] is None
            nav.gateway(app, "view.width", pane="sections", width=200)
        else:
            nav.gateway(app, "palette.activate", index=index)
        assert nav.navigation(app)["modal"] is None
        assert nav.session(app)["document"] == document
        collection = "sections" if kind == "section" else "groups"
        actual = [item["path"] for item in nav.session(app)[collection]
                  if kind == "section" or item["path"].startswith("Atlas/")]
        assert actual == expected
        assert gestures.files(vault) == original
        app.restart(hard=False)
        actual = [item["path"] for item in nav.session(app)[collection]
                  if kind == "section" or item["path"].startswith("Atlas/")]
        assert actual == expected
        assert gestures.files(vault) == original


@pytest.mark.parametrize("kind", ["section", "group"])
def test_empty_tree_reconciliation_refreshes_reorder_choices_without_resetting_query(vault, kind):
    parent = vault / "Atlas" if kind == "group" else vault
    for name in ("Empty A", "Empty B", "Empty C"):
        (parent / name).mkdir()
    collection = f"{kind}s"
    prefix = "Atlas/" if kind == "group" else ""
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("empty-tree-reorder")) as app:
        source = nav.item(app, collection, f"{prefix}Empty C")["id"]
        anchor = nav.item(app, collection, f"{prefix}Empty B")["id"]
        nav.gateway(app, "palette.open", mode="reorder", target={kind: source})
        modal = nav.gateway(app, "palette.query", query="  Before  ")["navigation"]["modal"]
        index = next(index for index, row in enumerate(modal["results"])
                     if (row["key"]["reorder"]["before"] or {}).get("item") == {kind: anchor})
        modal = nav.gateway(app, "palette.step", delta=index)["navigation"]["modal"]
        selected = modal["active_key"]
        (parent / "Empty B").rmdir()
        (parent / "Empty D").mkdir()
        wait_for(lambda: any(item["path"] == f"{prefix}Empty D" for item in nav.session(app)[collection]),
                 timeout=5, description="empty-container filesystem reconciliation")
        modal = nav.navigation(app)["modal"]
        assert modal["query"] == "  Before  "
        assert modal["active_key"] == selected
        assert modal["results"][modal["active_index"]]["enabled"] is False
        assert any(row["title"] == "Before Empty D" and row["enabled"] for row in modal["results"])
        nav.gateway(app, "palette.activate", disposition="blocked")
        assert nav.navigation(app)["modal"]["active_key"] == selected


def test_conflict_copy_command_opens_preserved_draft_through_the_existing_barrier(vault):
    from tools.wrentest.client import RemoteError
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("open-conflict-copy")) as app:
        nav.jump(app, nav.FIRST)
        path = vault / nav.FIRST
        with nav.unwritable(path.parent):
            app.client.call("editor.action", {"kind": "move", "motion": "end"})
            local = app.client.call("editor.insert", {"text": " local recoverable draft"})["text"]
            external = "external replacement bytes\n"
            path.write_text(external)
        try:
            nav.core(app, "flush")
        except RemoteError as error:
            assert "External changes were preserved" in str(error)
        link = wait_for(lambda: nav.navigation(app)["conflict_copy"], timeout=5, description="published conflict-copy action")
        assert link["enabled"] is True
        assert path.read_text() == external
        assert (vault / link["path"]).read_text() == local
        nav.command(app, "note.open_conflict")
        assert nav.session(app)["document"]["path"] == link["path"]
        assert nav.session(app)["document"]["text"] == local
        assert app.client.call("editor.get")["text"] == local
        assert path.read_text() == external


@pytest.mark.parametrize("theme,body", [("dark", (213, 217, 224)), ("light", (49, 56, 68))])
def test_screenshot_draws_the_current_editor_after_layout(vault, theme, body):
    artifacts = artifact_directory(f"capture-after-layout-{theme}")
    with WrenApp(workspace=vault, artifacts_dir=artifacts) as app:
        nav.jump(app, nav.FIRST)
        nav.choose_theme(app, theme)
        before = nav.editor_identity(app.client.call("editor.get"))
        for index, (sections, focused) in enumerate(((200, False), (320, False), (320, True), (200, False))):
            nav.gateway(app, "view.width", pane="sections", width=sections)
            nav.gateway(app, "view.focus", enabled=focused)
            path = artifacts / f"current-frame-{index}.png"
            result = app.client.call("test.capture.after_layout", {"path": str(path)})
            image = png.read_png(path.read_bytes())
            current = nav.navigation(app)
            viewport = current["viewport"]
            assert (image.width, image.height) == (result["width"], result["height"])
            left = ((viewport["width"] - 760) / 2 if focused else sum(current["effective_widths"].values()))
            assert nav.color_count(image, viewport, (left + 16, 115, left + 500, 220), body) > 70, "The rebuilt editor's glyphs were not drawn before capture"
            assert nav.editor_identity(app.client.call("editor.get")) == before


@pytest.mark.native_x11
@pytest.mark.parametrize("widget", ["hover", "focus"])
@pytest.mark.parametrize("theme,raised,accent", [
    ("dark", (28, 31, 38), (240, 180, 90)),
    ("light", (255, 255, 255), (154, 88, 0)),
])
def test_screenshot_matches_native_hover_and_focused_input_styles(native_workspace, theme, raised, accent, widget):
    app, root, window = native_workspace
    artifacts = artifact_directory(f"capture-widget-status-{theme}")
    nav.choose_theme(app, theme)
    assert nav.navigation(app)["viewport"] == {"width": 1280, "height": 800}
    before = gestures.files(root)
    if widget == "hover":
        gestures.move(window, (1252, 24))
        native = wait_for(lambda: (image if (image := read_pixels(app.pid, window, 1236, 8, 32, 32))
                                  and image.pixel(10, 4) == raised else None),
                          description="native enabled toolbar button hover styling")
        path = artifacts / "hovered-button.png"
        app.client.call("test.capture.after_layout", {"path": str(path)})
        image = png.read_png(path.read_bytes())
        assert image.pixel(1246, 12)[:3] == native.pixel(10, 4), "Capture lost the native button's hover status"
        assert gestures.files(root) == before
        return

    nav.command(app, "note.rename")
    gestures.type_text(window, " focused capture")
    wait_for(lambda: " focused capture" in nav.navigation(app)["dialog"]["name"],
             description="native typing confirms the rename input owns focus")
    rectangle = (380, 330, 520, 125)

    def focused_border():
        image = read_pixels(app.pid, window, *rectangle)
        count = sum(max(abs(a - b) for a, b in zip(image.pixel(x, y), accent)) <= 3
                    for y in range(image.height) for x in range(image.width))
        # Horizontal edges are antialiased; both straight vertical sides still
        # contribute at least forty unblended accent pixels, excluding the caret.
        return count if count > 40 else None

    native_accent = wait_for(focused_border, timeout=4, description="native focused input accent border")
    path = artifacts / "focused-input.png"
    app.client.call("test.capture.after_layout", {"path": str(path)})
    image = png.read_png(path.read_bytes())
    captured_accent = nav.color_count(image, nav.navigation(app)["viewport"], (380, 330, 900, 455), accent)
    assert captured_accent == native_accent, f"Capture lost the focused input border: native={native_accent}, captured={captured_accent}"
    assert gestures.files(root) == before
    app.client.call("dialog.close")


@pytest.mark.native_x11
@pytest.mark.parametrize("query,command", [
    ("Sort notes: title", "notes.sort_title"),
    ("Reset column widths", "view.reset_widths"),
])
def test_native_palette_preference_actions_restore_editor_typing_focus(native_workspace, query, command):
    app, root, window = native_workspace
    original = gestures.session(app)["document"]["text"]
    gestures.keys(window, "alt+shift+p")
    wait_for(lambda: gestures.navigation(app)["modal"] is not None)
    gestures.type_text(window, query)
    wait_for(lambda: gestures.navigation(app)["modal"]["query"] == query)
    assert gestures.navigation(app)["modal"]["active_key"] == {"command": command}
    gestures.keys(window, "Return")
    wait_for(lambda: gestures.navigation(app)["modal"] is None)
    gestures.keys(window, "ctrl+End")
    suffix = " native focus restored"
    gestures.type_text(window, suffix)
    wait_for(lambda: app.client.call("editor.get")["text"] == original + suffix,
             description="typing reaches editor after palette action")
    wait_for(lambda: (root / gestures.ORIGIN).read_text() == original + suffix,
             description="refocused native edit is autosaved")


@pytest.mark.native_x11
def test_native_move_picker_restores_typing_focus_on_the_moved_document(native_workspace):
    app, root, window = native_workspace
    original = gestures.session(app)["document"]["text"]
    note = gestures.session(app)["document"]["note"]
    group = gestures.item(app, "groups", gestures.DESTINATION)["id"]
    nav.gateway(app, "palette.open", mode="move_note", note=note)
    current = nav.gateway(app, "palette.query", query="C Visible")["navigation"]["modal"]
    index = next(index for index, row in enumerate(current["results"])
                 if row["key"] == {"destination": {"group": group}})
    nav.gateway(app, "palette.step", delta=index)
    gestures.keys(window, "Return")
    destination = root / gestures.DESTINATION / "Origin.md"
    wait_for(lambda: gestures.navigation(app)["modal"] is None and destination.is_file(),
             description="native Enter commits the move picker")
    assert not (root / gestures.ORIGIN).exists()
    assert gestures.session(app)["document"]["note"] == note
    gestures.keys(window, "ctrl+End")
    suffix = " typed after move"
    gestures.type_text(window, suffix)
    wait_for(lambda: app.client.call("editor.get")["text"] == original + suffix,
             description="native typing reaches moved editor")
    wait_for(lambda: destination.read_text() == original + suffix,
             description="moved document autosaves the refocused native edit")


@pytest.mark.native_x11
def test_native_double_click_opens_and_alt_r_renames_without_replacing_the_document(native_workspace):
    app, root, window = native_workspace
    source = gestures.item(app, "notes", gestures.ORIGIN)
    original = (root / gestures.ORIGIN).read_bytes()
    point = gestures.point_in(gestures.visible_row(app, {"note": source["id"]})["visible"])
    gestures.move(window, point)
    gestures.xdo("click", "--repeat", "2", "--delay", "80", "1")
    wait_for(lambda: gestures.navigation(app)["column"] == "editor",
             description="native double-click moves into the editor")
    assert gestures.navigation(app)["dialog"] is None
    gestures.keys(window, "alt+r")
    wait_for(lambda: gestures.navigation(app)["dialog"] is not None, description="Alt+R naming dialog")
    gestures.keys(window, "ctrl+a")
    gestures.type_text(window, "Renamed by shortcut")
    gestures.keys(window, "Return")
    destination = root / gestures.SOURCE / "Renamed by shortcut.md"
    wait_for(destination.is_file, description="native Alt+R rename commits")
    assert not (root / gestures.ORIGIN).exists()
    assert destination.read_bytes() == original
    assert gestures.session(app)["document"]["note"] == source["id"]
    assert gestures.session(app)["document"]["path"] == str(destination.relative_to(root))


@pytest.mark.native_x11
def test_native_row_context_actions_use_the_clicked_item_not_the_open_note(native_workspace):
    app, root, window = native_workspace
    source = gestures.item(app, "notes", gestures.PICKER_A)
    original = (root / gestures.PICKER_A).read_bytes()
    active = gestures.session(app)["document"]
    point = gestures.point_in(gestures.visible_row(app, {"note": source["id"]})["visible"])
    gestures.move(window, point)
    gestures.xdo("click", "3")
    wait_for(lambda: gestures.navigation(app)["modal"] is not None, description="native row context actions")
    modal = gestures.navigation(app)["modal"]
    assert modal["source"]["item"] == {"note": source["id"]}
    index = next(index for index, row in enumerate(modal["results"])
                 if row["key"].get("item_action", {}).get("command") == "note.rename")
    assert modal["results"][index]["key"]["item_action"]["target"]["item"] == {"note": source["id"]}
    nav.gateway(app, "palette.activate", index=index)
    wait_for(lambda: gestures.navigation(app)["dialog"] is not None)
    app.client.call("dialog.name", {"name": "Context target"})
    app.client.call("dialog.submit")
    destination = root / gestures.SOURCE / "Context target.md"
    wait_for(destination.is_file, description="context rename affects the clicked note")
    assert not (root / gestures.PICKER_A).exists()
    assert destination.read_bytes() == original
    assert (root / gestures.ORIGIN).read_text() == active["text"]
    assert gestures.session(app)["document"] == active
