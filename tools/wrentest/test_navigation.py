"""Real-window navigation acceptance through the production IPC gateways.

Every vault and global configuration is disposable. These are not native-input
checks: row clicks, shortcuts, drags, and actual window resizing have separate
coverage. The two legacy core adapters used below have no navigation RPC yet.
"""

from __future__ import annotations

from contextlib import contextmanager
from datetime import date
import os
from pathlib import Path
import re
import sys
import stat
import tomllib

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import capture
from tools.wrentest.test_setup import BACKGROUNDS, artifact_directory, editor_column
from tools.wrentest.ui import WrenApp, wait_for


COMMANDS = {
    "note.new", "note.new_dated", "note.rename", "note.delete", "note.reveal",
    "note.move", "group.new", "section.new", "group.rename", "group.delete",
    "group.reveal", "group.move", "section.rename", "section.delete", "section.reveal",
    "navigation.jump", "navigation.search", "navigation.find", "navigation.outline",
    "navigation.back",
    "navigation.forward", "navigation.column_left", "navigation.column_right", "view.focus",
    "view.reset_widths", "notes.sort_modified", "notes.sort_title", "theme.choose",
    "font.choose", "font.increase", "font.decrease", "font.reset",
    "workspace.switch", "workspace.add", "workspace.remove",
    "section.reorder", "group.reorder", "note.open_conflict",
    "settings.open", "settings.reload", "app.quit",
    "markdown.toggle_task", "markdown.bold", "markdown.italic", "markdown.code",
    "edit.move_line_up", "edit.move_line_down",
    "editor.toggle_markdown_assist", "editor.toggle_engine",
}
# Listed only with the `boards` setting on, and never enabled on a Markdown note.
BOARD_COMMANDS = {"note.export_svg"} if os.environ.get("WRENTEST_BOARDS") == "on" else set()
DEFAULT_WIDTHS = {"sections": 176, "groups": 192, "notes": 256}
FIRST = "Atlas/Research/Note 1.md"
SECOND = "Atlas/Research/Note 2.md"
TENTH = "Atlas/Research/Note 10.md"


@pytest.fixture
def vault_factory(tmp_path: Path):
    def make(name: str = "field-notes") -> Path:
        root = tmp_path / name
        for relative in (
            "Atlas/Research", "Atlas/Review", "Atlas/Empty group", "Atlas/Workbench",
            "Boreal/Research", "Boreal/Workbench", "Empty section",
        ):
            (root / relative).mkdir(parents=True, exist_ok=True)
        contents = {
            FIRST: f"first plain baseline; vault-token-{name}\nsecond line editor state\n",
            SECOND: (
                "# Observations\n\n前İSTANBUL café ΟΣ\nemoji 👩🏽‍💻 e\u0301\n"
                + "界" * 65 + "needle amber" + "é" * 65 + "\nneedle amber again\n"
            ),
            TENTH: "amber-before\n",
            "Atlas/Research/Needle amber.md": "# Titled checklist\n\nA title-only match lives here.\n",
            "Atlas/Review/Peer check.md": "needle amber from the review desk\n",
            "Atlas/Workbench/Loose 3.md": "needle amber from the workbench desk\n",
            "Boreal/Research/Remote 4.md": "needle amber from the northern desk\n",
            "Boreal/Workbench/Loose 5.md": "northern baseline\n",
        }
        for index, (relative, text) in enumerate(contents.items()):
            path = root / relative
            path.write_text(text, encoding="utf-8")
            stamp = 1_700_000_000 + index
            os.utime(path, ns=(stamp * 1_000_000_000, stamp * 1_000_000_000))
        (root / "Atlas/Research/.private").write_bytes(b"invented private attachment\x00")
        return root
    return make


@pytest.fixture
def vault(vault_factory) -> Path:
    return vault_factory()


@pytest.fixture
def periodic_vault(vault: Path):
    writer = vault / ".periodic-writer"
    os.link(vault / TENTH, writer)
    return vault, writer


def session(app: WrenApp):
    return app.client.call("session.get")


def navigation(app: WrenApp):
    return app.client.call("navigation.get")


def core(app: WrenApp, operation: str, **params):
    return app.client.call("core.apply", {"op": operation, **params})


def item(app: WrenApp, collection: str, path: str):
    matches = [row for row in session(app)[collection] if row["path"] == path]
    assert len(matches) == 1, (collection, path, matches)
    return matches[0]


def gateway(app: WrenApp, operation: str, *, disposition: str = "completed", **params):
    result = app.client.call(operation, params)
    assert result["disposition"] == disposition, result
    assert type(result["correlation"]) is int and result["correlation"] > 0
    assert result["navigation"]["processed"] == {
        "correlation": result["correlation"],
        "disposition": disposition,
        "warning": result["warning"],
    }
    if disposition in {"blocked", "committed-warning"}:
        assert isinstance(result["warning"], str) and result["warning"]
    else:
        assert result["warning"] is None
    return result


def command(app: WrenApp, command_id: str, **params):
    return gateway(app, "command.run", id=command_id, **params)


def palette(app: WrenApp, mode: str, query: str = ""):
    gateway(app, "palette.open", mode=mode)
    return gateway(app, "palette.query", query=query)["navigation"]["modal"]


def activate(app: WrenApp, key: dict, *, disposition: str = "completed"):
    modal = navigation(app)["modal"]
    assert modal is not None
    indices = [index for index, row in enumerate(modal["results"]) if row["key"] == key]
    assert len(indices) == 1, (key, modal)
    return gateway(app, "palette.activate", index=indices[0], disposition=disposition)


def choose_theme(app: WrenApp, theme: str):
    """Pick a built-in theme through the palette, as a user would."""
    palette(app, "themes")
    return activate(app, {"theme": theme if theme.startswith("wren-") else f"wren-{theme}"})


def jump(app: WrenApp, path: str, *, disposition: str = "completed"):
    note = item(app, "notes", path)
    palette(app, "jump", note["title"])
    activate(app, {"note": note["id"]}, disposition=disposition)
    snapshot = session(app)
    assert snapshot["document"]["path"] == path
    assert snapshot["selected_note"] == note["id"]
    return note["id"]


def settle_workspace(app: WrenApp, response: dict, root: Path, *, disposition="completed"):
    correlation = response["correlation"]

    def finished():
        snapshot = session(app)
        processed = snapshot["navigation"]["processed"]
        return snapshot if (not snapshot["loading"] and processed is not None
                            and processed["correlation"] == correlation
                            and processed["disposition"] != "async-pending") else None

    snapshot = wait_for(finished, timeout=5, interval=0.025,
                        description="correlated workspace completion")
    assert snapshot["navigation"]["processed"]["disposition"] == disposition, snapshot
    assert Path(snapshot["root"]) == root.resolve()
    return snapshot


def add_workspace(app: WrenApp, root: Path, name: str | None = None):
    params = {"path": str(root)}
    if name is not None:
        params["name"] = name
    response = gateway(app, "workspace.add", disposition="async-pending", **params)
    return settle_workspace(app, response, root)


def switch_workspace(app: WrenApp, root: Path):
    palette(app, "workspaces")
    response = activate(app, {"workspace": str(root.resolve())}, disposition="async-pending")
    return settle_workspace(app, response, root)


def append(app: WrenApp, text: str):
    app.client.call("editor.action", {"kind": "move", "motion": "end"})
    return app.client.call("editor.insert", {"text": text})["text"]


def saved(app: WrenApp, path: Path, expected: str):
    def observed():
        document = session(app)["document"]
        return (path.is_file() and path.read_bytes() == expected.encode("utf-8")
                and document is not None and document["text"] == expected
                and document["dirty"] is False)
    wait_for(observed, timeout=5, interval=0.025, description="saved UTF-8 bytes and clean draft")


def byte_slice(text: str, span: dict) -> str:
    encoded = text.encode("utf-8")
    assert 0 <= span["start"] < span["end"] <= len(encoded), (text, span)
    encoded[:span["start"]].decode("utf-8", errors="strict")
    encoded[span["end"]:].decode("utf-8", errors="strict")
    return encoded[span["start"]:span["end"]].decode("utf-8", errors="strict")


def byte_position(text: str, offset: int):
    prefix = text.encode("utf-8")[:offset].decode("utf-8", errors="strict")
    return {"line": prefix.count("\n"), "column": len(prefix.rsplit("\n", 1)[-1].encode("utf-8"))}


def editor_identity(snapshot):
    return {key: snapshot[key] for key in (
        "text", "cursor", "selection", "selected_text", "revision", "can_undo", "can_redo",
    )}


def selection_identity(snapshot):
    return {key: snapshot[key] for key in ("selected_section", "selected_group", "selected_note")}


def tree_bytes(path: Path):
    if path.is_file():
        return {"": path.read_bytes()}
    return {str(child.relative_to(path)): child.read_bytes()
            for child in path.rglob("*") if child.is_file()}


@contextmanager
def unwritable(directory: Path):
    assert os.geteuid() != 0, "Permission acceptance requires an unprivileged test process"
    mode = stat.S_IMODE(directory.stat().st_mode)
    try:
        directory.chmod(mode & ~0o222)
        yield
    finally:
        directory.chmod(mode)


def test_full_command_catalog_and_context_availability(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("navigation-catalog")) as app:
        jump(app, FIRST)
        rows = palette(app, "commands")["results"]
        by_id = {row["key"]["command"]: row for row in rows}
        assert set(by_id) == COMMANDS | BOARD_COMMANDS
        assert len(rows) == len(COMMANDS | BOARD_COMMANDS)
        assert {row["group"] for row in rows} == {"This note", "Markdown", "Go", "View", "Workspace",
                                                   "App"}
        assert all(row["title"] for row in rows)
        assert {key: row["state"] for key, row in by_id.items() if row["state"]} == {
            "editor.toggle_markdown_assist": "on", "editor.toggle_engine": "classic"}
        for key in COMMANDS - {"navigation.back", "navigation.forward", "note.open_conflict"}:
            assert by_id[key]["enabled"] is True, key
        assert by_id["navigation.forward"]["enabled"] is False
        assert by_id["note.open_conflict"]["enabled"] is False
        for key in BOARD_COMMANDS:
            assert by_id[key]["enabled"] is False, key
        before = session(app)["document"]
        command(app, "navigation.forward", disposition="blocked")
        assert session(app)["document"] == before
        assert navigation(app)["modal"]["mode"] == "commands"
        gateway(app, "palette.close")
        empty = item(app, "sections", "Empty section")["id"]
        core(app, "select_parent", parent={"section": empty})
        rows = palette(app, "commands")["results"]
        by_id = {row["key"]["command"]: row for row in rows}
        for key in ("note.rename", "note.delete", "note.reveal", "note.move", "navigation.find",
                    "navigation.outline", "group.rename", "group.delete", "group.reveal", "group.move",
                    "note.new", "note.new_dated", "notes.sort_title",
                    "markdown.toggle_task", "markdown.bold", "edit.move_line_up"):
            assert by_id[key]["enabled"] is False, key
        for key in ("group.new", "section.rename"):
            assert by_id[key]["enabled"] is True, key
        activate(app, {"command": "note.rename"}, disposition="blocked")
        assert navigation(app)["dialog"] is None
        assert session(app)["document"] is None


def test_outline_jumps_within_the_open_note_and_closes_when_it_changes(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("navigation-outline")) as app:
        call = app.client.call
        jump(app, SECOND)
        call("editor.cursor", {"line": 3, "column": 2, "selection": None})
        before = call("editor.get")
        modal = palette(app, "outline")
        assert [row["title"] for row in modal["results"]] == ["Observations"]
        assert modal["results"][0]["current"] is True
        gateway(app, "palette.close")
        assert call("editor.get")["cursor"] == {"line": 3, "column": 2}

        palette(app, "outline")
        activate(app, {"heading": {"revision": before["revision"], "line": 0}})
        after = call("editor.get")
        assert after["cursor"] == {"line": 0, "column": 0}
        assert (after["text"], after["revision"]) == (before["text"], before["revision"])
        assert navigation(app)["column"] == "editor"
        assert navigation(app)["modal"] is None

        # Another note opening underneath closes the outline.
        palette(app, "outline")
        core(app, "open_note", note=item(app, "notes", FIRST)["id"])
        assert session(app)["document"]["path"] == FIRST
        assert navigation(app)["modal"] is None


@pytest.mark.parametrize("command_id,query,name,path,collection", [
    ("section.new", "nw sct", "Idea: field", "Idea- field", "sections"),
    ("group.new", "nw grp", "Notebook/Trials", "Atlas/Notebook-Trials", "groups"),
    ("note.new", "nw nt", "Draft: bridge.md", "Atlas/Research/Draft- bridge.md", "notes"),
])
def test_fuzzy_commands_create_real_items_through_dialogs(
    vault: Path, command_id: str, query: str, name: str, path: str, collection: str,
) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory(f"palette-{command_id}")) as app:
        jump(app, FIRST)
        assert palette(app, "commands", "srt ttl")["results"][0]["key"] == {"command": "notes.sort_title"}
        activate(app, {"command": "notes.sort_title"})
        assert navigation(app)["sort"] == "title"
        rows = palette(app, "commands", query)["results"]
        assert {"command": command_id} in [row["key"] for row in rows]
        count = len(session(app)[collection])
        activate(app, {"command": command_id})
        assert navigation(app)["modal"] is None
        assert navigation(app)["dialog"]["kind"].startswith("Create")
        command(app, "note.new_dated", disposition="blocked")
        gateway(app, "palette.open", mode="commands", disposition="blocked")
        app.client.call("dialog.confirm_trash")
        assert len(session(app)[collection]) == count
        app.client.call("dialog.name", {"name": name})
        app.client.call("dialog.submit")
        assert navigation(app)["dialog"] is None
        assert len(session(app)[collection]) == count + 1
        created = item(app, collection, path)
        assert (vault / path).exists()
        if collection == "groups":
            assert session(app)["selected_group"] == created["id"]
            assert session(app)["document"] is None
        if collection == "notes":
            assert session(app)["selected_note"] == created["id"]
            assert (vault / path).read_bytes() == b""
            expected = append(app, "Created by the actual naming dialog: café 🐦\n")
            saved(app, vault / path, expected)
        app.client.call("dialog.submit")
        assert len(session(app)[collection]) == count + 1, "A dismissed dialog replayed creation"


def test_dated_creation_and_shortcut_metadata_handle_filename_collisions(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("dated-command")) as app:
        jump(app, FIRST)
        rows = palette(app, "commands")["results"]
        shortcuts = {row["key"]["command"]: row["shortcut"] for row in rows}
        if sys.platform == "darwin":
            assert shortcuts["note.new"] == "⌘N"
            assert shortcuts["note.new_dated"] == "⇧⌘N"
        else:
            assert shortcuts["note.new"] == "Alt+N"
            assert shortcuts["note.new_dated"] == "Alt+Shift+N"
        nonempty = [value for value in shortcuts.values() if value]
        assert len(nonempty) == len(set(nonempty))
        activate(app, {"command": "note.new_dated"})
        first_path = session(app)["document"]["path"]
        stem = Path(first_path).stem
        assert re.fullmatch(r"\d{4}-\d{2}-\d{2}", stem)
        assert date.fromisoformat(stem) == date.today()
        assert first_path == f"Atlas/Research/{stem}.md"
        command(app, "note.new_dated")
        assert session(app)["document"]["path"] == f"Atlas/Research/{stem} 2.md"
        command(app, "note.new")
        assert navigation(app)["dialog"]["name"] == stem
        app.client.call("dialog.submit")
        assert session(app)["document"]["path"] == f"Atlas/Research/{stem} 3.md"
        for suffix in ("", " 2", " 3"):
            assert (vault / f"Atlas/Research/{stem}{suffix}.md").read_bytes() == b""


@pytest.mark.parametrize("kind,path", [
    ("note", FIRST), ("group", "Atlas/Research"), ("section", "Atlas"),
])
def test_trash_requires_explicit_confirmation_and_only_uses_private_quarantine(
    vault: Path, kind: str, path: str,
) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory(f"confirmed-trash-{kind}")) as app:
        note = jump(app, FIRST)
        expected = append(app, "published before confirmed trash\n")
        core(app, "flush")
        assert (vault / FIRST).read_bytes() == expected.encode()
        original = tree_bytes(vault / path)
        quarantine = Path(session(app)["trash_path"])
        assert quarantine == app.sandbox / "trash"
        assert quarantine.is_dir() and not quarantine.is_symlink()
        assert stat.S_IMODE(quarantine.stat().st_mode) == 0o700
        assert list(quarantine.iterdir()) == []
        palette(app, "commands", f"Delete {kind}")
        activate(app, {"command": f"{kind}.delete"})
        dialog = navigation(app)["dialog"]
        assert dialog["kind"].startswith("Delete(")
        app.client.call("dialog.submit")
        assert navigation(app)["dialog"] == dialog
        assert tree_bytes(vault / path) == original
        assert list(quarantine.iterdir()) == []
        app.client.call("dialog.close")
        assert (vault / path).exists()
        command(app, f"{kind}.delete")
        app.client.call("dialog.confirm_trash")
        assert navigation(app)["dialog"] is None
        assert not (vault / path).exists()
        assert session(app)["document"] is None
        assert note not in [entry["id"] for entry in session(app)["notes"]]
        moved = list(quarantine.iterdir())
        assert len(moved) == 1
        assert tree_bytes(moved[0]) == original
        assert (vault / "Boreal/Workbench/Loose 5.md").read_bytes() == b"northern baseline\n"
        palette(app, "search", "vault-token-")
        assert navigation(app)["modal"]["results"] == []


def test_independent_name_filters_render_only_matching_rows_and_keep_hidden_document_open(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("filter-scope")) as app:
        note = jump(app, FIRST)
        before = app.client.call("editor.get")
        atlas = item(app, "sections", "Atlas")["id"]
        boreal = item(app, "sections", "Boreal")["id"]
        review = item(app, "groups", "Atlas/Review")["id"]
        gateway(app, "filter.set", pane="sections", query="brl")
        gateway(app, "filter.set", pane="groups", query="rvw")
        gateway(app, "filter.set", pane="notes", query="needle amber")
        snapshot = navigation(app)
        assert snapshot["visible"]["sections"] == [boreal]
        assert snapshot["visible"]["groups"] == [review]
        assert set(snapshot["visible"]["notes"]) == {
            item(app, "notes", SECOND)["id"], item(app, "notes", "Atlas/Research/Needle amber.md")["id"],
        }
        assert note not in snapshot["visible"]["notes"]
        assert session(app)["selected_note"] == note
        assert editor_identity(app.client.call("editor.get")) == editor_identity(before)
        assert snapshot["filters"] == {"sections": "brl", "groups": "rvw", "notes": "needle amber"}
        bounds = wait_for(lambda: app.client.call("navigation.bounds")["rows"], timeout=5)
        keys = [row["key"] for row in bounds]
        assert {"section": boreal} in keys and {"group": review} in keys
        assert {"section": atlas} not in keys
        gateway(app, "filter.set", pane="sections", query="")
        assert set(navigation(app)["visible"]["sections"]) == {
            entry["id"] for entry in session(app)["sections"]
        }
        assert session(app)["document"]["note"] == note


@pytest.mark.parametrize("collection,parent,expected_paths", [
    ("groups", "Atlas/Research", {SECOND, "Atlas/Research/Needle amber.md"}),
    ("groups", "Atlas/Workbench", {"Atlas/Workbench/Loose 3.md"}),
    ("groups", "Atlas/Empty group", set()),
    ("groups", "Boreal/Research", {"Boreal/Research/Remote 4.md"}),
])
def test_notes_filter_matches_titles_or_content_in_exact_parent_only(
    vault: Path, collection: str, parent: str, expected_paths: set[str],
) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("parent-note-filter")) as app:
        parent_id = item(app, collection, parent)["id"]
        core(app, "select_parent", parent={"group" if collection == "groups" else "section": parent_id})
        before = session(app)
        gateway(app, "filter.set", pane="notes", query="needle amber")
        expected_ids = {item(app, "notes", path)["id"] for path in expected_paths}
        assert set(navigation(app)["visible"]["notes"]) == expected_ids
        assert selection_identity(session(app)) == selection_identity(before)
        assert session(app)["document"] == before["document"]
        gateway(app, "filter.set", pane="notes", query="Atlas/Research")
        assert navigation(app)["visible"]["notes"] == [], "Paths must not leak into note matching"
        assert navigation(app)["filters"]["sections"] == ""
        assert navigation(app)["filters"]["groups"] == ""
        gateway(app, "filter.set", pane="notes", query="")
        parent_key = {"group" if collection == "groups" else "section": parent_id}
        assert set(navigation(app)["visible"]["notes"]) == {
            row["id"] for row in session(app)["notes"] if row["parent"] == parent_key
        }


@pytest.mark.parametrize("query,matched", [
    ("i\u0307s", "İS"), ("👩🏽‍💻", "👩🏽‍💻"), ("E\u0301", "e\u0301"), ("needle amber", "needle amber"),
])
def test_global_search_has_safe_utf8_snippets_first_hit_cursor_and_independent_find(
    vault: Path, query: str, matched: str,
) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("unicode-search")) as app:
        jump(app, FIRST)
        app.client.call("editor.find", {"query": "baseline"})
        target = item(app, "notes", SECOND)["id"]
        original = (vault / SECOND).read_text(encoding="utf-8")
        command(app, "navigation.search")
        modal = gateway(app, "palette.query", query=query)["navigation"]["modal"]
        hit = next(row for row in modal["results"] if row["note"] == target)
        assert hit["hit"] == "content" and hit["enabled"] is True
        assert hit["index_revision"] == navigation(app)["index_revision"]
        assert byte_slice(hit["snippet"], hit["highlight"]) == matched
        assert byte_slice(original, hit["content_match"]) == matched
        start = original.encode().index(matched.encode())
        assert hit["content_match"] == {"start": start, "end": start + len(matched.encode())}
        if query == "needle amber":
            assert hit["snippet"].startswith("…") and hit["snippet"].endswith("…")
            assert modal["results"][0]["path"] == "Atlas/Research/Needle amber.md"
            assert modal["results"][0]["hit"] == "title"
            assert modal["results"][0]["content_match"] is None
            assert modal["results"][0]["highlight"] is None
        activate(app, {"note": target})
        editor = app.client.call("editor.get")
        assert editor["text"] == original
        assert editor["selected_text"] == matched
        assert editor["selection"] == byte_position(original, start)
        assert editor["cursor"] == byte_position(original, start + len(matched.encode()))
        assert editor["find_open"] is True and editor["find_query"] == "baseline"
        assert navigation(app)["modal"] is None
        assert session(app)["document"]["note"] == target


def test_jump_is_fuzzy_title_only_and_branching_history_preserves_mru_on_restart(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("jump-history")) as app:
        first = jump(app, FIRST)
        assert palette(app, "jump", "vault-token-")["results"] == []
        assert palette(app, "jump", "amber-before")["results"] == []
        gateway(app, "palette.close")
        modal = palette(app, "jump", "nt 2")
        second = item(app, "notes", SECOND)["id"]
        assert {"note": second} in [row["key"] for row in modal["results"]]
        assert all(row["hit"] == "title" and row["content_match"] is None for row in modal["results"])
        activate(app, {"note": second})
        command(app, "navigation.back")
        assert session(app)["document"]["note"] == first
        assert navigation(app)["can_forward"] is True
        third_path = "Atlas/Workbench/Loose 3.md"
        jump(app, third_path)
        branched = navigation(app)
        assert branched["history"][-2:] == [FIRST, third_path]
        assert SECOND not in branched["history"]
        assert branched["history_position"] == len(branched["history"]) - 1
        assert branched["mru"][-3:] == [SECOND, FIRST, third_path]
        assert branched["can_forward"] is False
        assert [row["path"] for row in palette(app, "jump")["results"][:3]] == [third_path, FIRST, SECOND]
        gateway(app, "palette.close")
        jump(app, third_path)
        assert navigation(app)["history"] == branched["history"], "Opening the current note is not a visit"
        app.restart(hard=False)
        assert session(app)["document"]["path"] == third_path
        after = navigation(app)
        for field in ("history", "history_position", "mru", "can_back", "can_forward"):
            assert after[field] == branched[field]
        assert [row["path"] for row in palette(app, "jump")["results"][:3]] == [third_path, FIRST, SECOND]
        assert navigation(app)["modal"]["results"][0]["current"] is True
        assert session(app)["selected_note"] is not None
        assert session(app)["selected_note"] == item(app, "notes", third_path)["id"]


def test_palette_selection_clamps_and_refresh_preserves_stable_key(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("palette-refresh")) as app:
        jump(app, FIRST)
        command(app, "notes.sort_title")
        palette(app, "jump", "Note")
        modal = gateway(app, "palette.step", delta=100_000)["navigation"]["modal"]
        assert modal["active_index"] == len(modal["results"]) - 1
        assert gateway(app, "palette.step", delta=-100_000)["navigation"]["modal"]["active_index"] == 0
        target = item(app, "notes", TENTH)["id"]
        rows = navigation(app)["modal"]["results"]
        index = next(index for index, row in enumerate(rows) if row["note"] == target)
        gateway(app, "palette.step", delta=index)
        before = navigation(app)
        assert before["modal"]["active_key"] == {"note": target}
        added = vault / "Atlas/Research/Note 0.md"
        added.write_text("Freshly scanned research sample\n", encoding="utf-8")

        def refreshed():
            current = navigation(app)
            return current if (current["index_revision"] > before["index_revision"]
                               and any(row["path"].endswith("/Note 0.md")
                                       for row in current["modal"]["results"])) else None

        after = wait_for(refreshed, timeout=5, interval=0.025, description="open Jump refresh")
        assert after["modal"]["active_key"] == {"note": target}
        assert after["modal"]["active_index"] != before["modal"]["active_index"]
        (vault / TENTH).unlink()
        wait_for(lambda: all(row["note"] != target for row in navigation(app)["modal"]["results"]),
                 timeout=5, description="removed active palette result disappears")
        modal = navigation(app)["modal"]
        assert modal["active_index"] == 0 and modal["active_key"] == modal["results"][0]["key"]
        modal = gateway(app, "palette.query", query="no-such-galaxy")["navigation"]["modal"]
        assert modal["results"] == [] and modal["active_index"] is None and modal["active_key"] is None
        gateway(app, "palette.step", delta=1)
        gateway(app, "palette.activate", disposition="blocked")
        assert session(app)["document"]["path"] == FIRST
        assert navigation(app)["modal"]["query"] == "no-such-galaxy"
        assert gateway(app, "palette.query", query="Note")["navigation"]["modal"]["active_index"] == 0


def test_index_tracks_own_create_rename_note_move_group_move_and_confirmed_delete(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("index-own-operations")) as app:
        jump(app, FIRST)
        initial = navigation(app)
        command(app, "note.new")
        app.client.call("dialog.name", {"name": "Ephemeral index"})
        app.client.call("dialog.submit")
        created = session(app)["selected_note"]
        expected = append(app, "lifecycle-cipher café\n")
        core(app, "flush")
        assert navigation(app)["index_count"] == initial["index_count"] + 1
        assert [row["path"] for row in palette(app, "search", "lifecycle-cipher")["results"]] == [
            "Atlas/Research/Ephemeral index.md",
        ]
        gateway(app, "palette.close")
        command(app, "note.rename")
        app.client.call("dialog.name", {"name": "Updated index"})
        app.client.call("dialog.submit")
        assert not (vault / "Atlas/Research/Ephemeral index.md").exists()
        assert palette(app, "jump", "Ephemeral")["results"] == []
        gateway(app, "palette.close")
        command(app, "note.move")
        modal = navigation(app)["modal"]
        current_parent = item(app, "groups", "Atlas/Research")["id"]
        assert {"destination": {"group": current_parent}} not in [row["key"] for row in modal["results"]]
        destination = item(app, "groups", "Atlas/Review")["id"]
        activate(app, {"destination": {"group": destination}})
        assert session(app)["document"]["path"] == "Atlas/Review/Updated index.md"
        command(app, "group.move")
        modal = navigation(app)["modal"]
        assert all(set(row["key"]["destination"]) == {"section"} for row in modal["results"])
        boreal = item(app, "sections", "Boreal")["id"]
        activate(app, {"destination": {"section": boreal}})
        final_path = "Boreal/Review/Updated index.md"
        assert session(app)["document"]["note"] == created
        assert session(app)["document"]["path"] == final_path
        assert (vault / final_path).read_bytes() == expected.encode()
        assert (vault / "Boreal/Review/Peer check.md").is_file()
        assert not (vault / "Atlas/Review").exists()
        hit = palette(app, "search", "lifecycle-cipher")["results"]
        assert len(hit) == 1 and hit[0]["path"] == final_path and hit[0]["note"] == created
        gateway(app, "palette.close")
        command(app, "note.delete")
        app.client.call("dialog.confirm_trash")
        assert not (vault / final_path).exists()
        assert navigation(app)["index_count"] == initial["index_count"]
        assert navigation(app)["index_revision"] > initial["index_revision"]
        assert palette(app, "search", "lifecycle-cipher")["results"] == []
        quarantined = list((app.sandbox / "trash").iterdir())
        assert len(quarantined) == 1 and quarantined[0].read_bytes() == expected.encode()


def test_open_search_refreshes_unopened_same_size_restored_mtime_and_external_membership(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("index-external-refresh")) as app:
        first = jump(app, FIRST)
        note = item(app, "notes", TENTH)["id"]
        modal = palette(app, "search", "amber-before")
        assert [row["note"] for row in modal["results"]] == [note]
        before = navigation(app)
        path = vault / TENTH
        metadata = path.stat()
        replacement = b"cobalt-after\n"
        assert len(replacement) == metadata.st_size
        path.write_bytes(replacement)
        os.utime(path, ns=(metadata.st_atime_ns, metadata.st_mtime_ns))
        assert path.stat().st_mtime_ns == metadata.st_mtime_ns
        wait_for(lambda: navigation(app)["modal"]["results"] == [], timeout=5, interval=0.025,
                 description="watcher/periodic byte refresh removes stale unopened content")
        assert navigation(app)["index_revision"] > before["index_revision"]
        modal = gateway(app, "palette.query", query="cobalt-after")["navigation"]["modal"]
        assert [row["note"] for row in modal["results"]] == [note]
        assert byte_slice(modal["results"][0]["snippet"], modal["results"][0]["highlight"]) == "cobalt-after"
        external = vault / "Atlas/Review/External copy.md"
        external.write_bytes(replacement)
        wait_for(lambda: len(navigation(app)["modal"]["results"]) == 2, timeout=5,
                 description="new external note enters the already-open search")
        assert navigation(app)["index_count"] == before["index_count"] + 1
        external.unlink()
        wait_for(lambda: len(navigation(app)["modal"]["results"]) == 1, timeout=5,
                 description="deleted external note leaves the already-open search")
        assert navigation(app)["index_count"] == before["index_count"]
        assert session(app)["document"]["note"] == first
        assert session(app)["document"]["text"] == (vault / FIRST).read_text(encoding="utf-8")


def test_periodic_refresh_finds_bytes_written_through_watcher_ignored_hidden_link(periodic_vault) -> None:
    vault, writer = periodic_vault
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("index-periodic-refresh")) as app:
        jump(app, FIRST)
        target = item(app, "notes", TENTH)["id"]
        assert [row["note"] for row in palette(app, "search", "amber-before")["results"]] == [target]
        metadata = writer.stat()
        assert metadata.st_ino == (vault / TENTH).stat().st_ino
        revision = navigation(app)["index_revision"]
        # watch::needs_scan ignores dot-component names. The visible note shares
        # bytes with this hidden directory entry, so periodic reconciliation is needed.
        writer.write_bytes(b"cobalt-after\n")
        os.utime(writer, ns=(metadata.st_atime_ns, metadata.st_mtime_ns))
        assert (vault / TENTH).stat().st_size == metadata.st_size
        assert (vault / TENTH).stat().st_mtime_ns == metadata.st_mtime_ns
        wait_for(lambda: navigation(app)["modal"]["results"] == [], timeout=5, interval=0.025,
                 description="periodic search refresh after a hidden-link write")
        assert navigation(app)["index_revision"] > revision
        rows = gateway(app, "palette.query", query="cobalt-after")["navigation"]["modal"]["results"]
        assert len(rows) == 1 and rows[0]["note"] == target and rows[0]["path"] == TENTH
        assert byte_slice(rows[0]["snippet"], rows[0]["highlight"]) == "cobalt-after"
        assert session(app)["document"]["path"] == FIRST


def test_search_activation_recomputes_stale_content_offset_against_loaded_bytes(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("search-stale-offset")) as app:
        jump(app, FIRST)
        app.client.call("editor.find", {"query": "baseline"})
        target = item(app, "notes", SECOND)["id"]
        modal = palette(app, "search", "needle amber")
        index, old_hit = next((index, row) for index, row in enumerate(modal["results"])
                              if row["note"] == target)
        gateway(app, "palette.step", delta=index)
        assert navigation(app)["modal"]["active_key"] == {"note": target}
        replacement = "needle amber now starts here\n\nMoved earlier than the cached offset: café 🐦\n"
        assert old_hit["content_match"]["start"] > len(replacement.encode())
        (vault / SECOND).write_text(replacement, encoding="utf-8")
        gateway(app, "palette.activate")
        editor = app.client.call("editor.get")
        assert session(app)["document"]["note"] == target
        assert editor["text"] == replacement
        assert editor["selected_text"] == "needle amber"
        assert editor["selection"] == {"line": 0, "column": 0}
        assert editor["cursor"] == {"line": 0, "column": len(b"needle amber")}
        assert editor["find_query"] == "baseline" and editor["find_open"] is True
        assert navigation(app)["modal"] is None


def test_dirty_draft_search_and_filters_are_query_only_without_flushing(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("draft-search-no-flush")) as app:
        note = jump(app, FIRST)
        disk = (vault / FIRST).read_bytes()
        expected = append(app, "unsaved-azimuth-cipher 👩🏽‍💻\n")
        app.client.call("test.fail_next", {"fault": "write"})
        with pytest.raises(RemoteError, match="injected storage failure"):
            core(app, "flush")
        with unwritable((vault / FIRST).parent):
            before = app.client.call("editor.get")
            modal = palette(app, "search", "unsaved-azimuth-cipher")
            assert [row["note"] for row in modal["results"]] == [note]
            assert modal["results"][0]["hit"] == "content"
            assert byte_slice(modal["results"][0]["snippet"], modal["results"][0]["highlight"]) == "unsaved-azimuth-cipher"
            gateway(app, "palette.close")
            gateway(app, "filter.set", pane="notes", query="unsaved-azimuth-cipher")
            assert navigation(app)["visible"]["notes"] == [note]
            assert (vault / FIRST).read_bytes() == disk
            assert session(app)["document"]["dirty"] is True
            assert session(app)["document"]["text"] == expected
            assert editor_identity(app.client.call("editor.get")) == editor_identity(before)
        core(app, "flush")
        saved(app, vault / FIRST, expected)


@pytest.mark.parametrize("transition", ["jump", "search", "scoped_note", "parent", "back", "forward", "workspace"])
def test_failed_write_blocks_each_navigation_gateway_and_explicit_retry_recovers(
    vault: Path, vault_factory, transition: str,
) -> None:
    other = vault_factory("other-vault")
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory(f"navigation-barrier-{transition}")) as app:
        if transition == "workspace":
            add_workspace(app, other, "Northern laboratory")
            switch_workspace(app, vault)
        first = jump(app, FIRST)
        second = item(app, "notes", SECOND)["id"]
        empty = item(app, "sections", "Empty section")["id"]
        if transition == "back":
            jump(app, SECOND)
        elif transition == "forward":
            jump(app, SECOND)
            command(app, "navigation.back")
        before = session(app)
        source = vault / before["document"]["path"]
        original = source.read_bytes()
        # Opening and querying a palette can outlast autosave's idle delay, and
        # autosave must not write the draft before the fault is armed.
        app.client.call("test.timer.hold", {"held": True})
        expected = append(app, f"retained draft for {transition}\n")
        app.client.call("editor.cursor", {"line": 0, "column": 10,
                                         "selection": {"line": 0, "column": 6}})
        if transition in {"jump", "search", "workspace"}:
            mode = "workspaces" if transition == "workspace" else transition
            query = {"jump": "Note 2", "search": "needle amber", "workspace": "Northern laboratory"}[transition]
            palette(app, mode, query)
            key = {"workspace": str(other.resolve())} if transition == "workspace" else {"note": second}
            modal = navigation(app)["modal"]
            activation = next(index for index, row in enumerate(modal["results"]) if row["key"] == key)
            request = ("palette.activate", {"index": activation})
        elif transition in {"back", "forward"}:
            request = ("command.run", {"id": f"navigation.{transition}"})
        elif transition == "scoped_note":
            gateway(app, "filter.set", pane="notes", query="needle amber")
            assert second in navigation(app)["visible"]["notes"]
            request = ("core.apply", {"op": "open_note", "note": second})
        else:
            request = ("core.apply", {"op": "select_parent", "parent": {"section": empty}})
        editor_before = app.client.call("editor.get")
        nav_before = navigation(app)
        app.client.call("test.fail_next", {"fault": "write"})
        if request[0] == "core.apply":
            # There is no scoped-row or parent navigation RPC in this protocol.
            with pytest.raises(RemoteError, match="injected storage failure"):
                app.client.call(*request)
            blocked = navigation(app)["processed"]
            assert blocked["disposition"] == "blocked"
            assert "injected storage failure" in blocked["warning"]
        else:
            response = gateway(app, request[0], disposition="blocked", **request[1])
            blocked = response["navigation"]["processed"]
            assert "injected storage failure" in blocked["warning"]
        assert blocked["correlation"] > nav_before["processed"]["correlation"]
        app.client.call("test.timer.hold", {"held": False})
        # Freeze only subsequent timer retries, after the injected write was reached.
        with unwritable(source.parent):
            after = session(app)
            assert after["root"] == before["root"]
            assert selection_identity(after) == selection_identity(before)
            assert after["document"]["note"] == before["document"]["note"]
            assert after["document"]["text"] == expected and after["document"]["dirty"] is True
            assert "injected storage failure" in after["document"]["error"]
            assert source.read_bytes() == original
            assert editor_identity(app.client.call("editor.get")) == editor_identity(editor_before)
            for field in ("history", "history_position", "mru", "filters", "active_registration", "generation"):
                assert after["navigation"][field] == nav_before[field]
            if nav_before["modal"] is not None:
                for field in ("mode", "query", "active_key", "active_index", "results"):
                    assert after["navigation"]["modal"][field] == nav_before["modal"][field]
                assert after["navigation"]["modal"]["error"] == blocked["warning"]
            else:
                assert after["navigation"]["modal"] is None
        core(app, "flush")
        assert source.read_bytes() == expected.encode()
        assert session(app)["document"]["dirty"] is False
        if request[0] == "core.apply":
            app.client.call(*request)
            assert navigation(app)["processed"]["disposition"] == "completed"
        else:
            response = gateway(app, request[0], disposition="async-pending" if transition == "workspace" else "completed", **request[1])
            if transition == "workspace":
                settle_workspace(app, response, other)
        if transition == "parent":
            assert session(app)["selected_section"] == empty and session(app)["document"] is None
        elif transition == "workspace":
            assert Path(session(app)["root"]) == other.resolve()
        else:
            target = first if transition == "back" else second
            assert session(app)["selected_note"] == target
        assert navigation(app)["modal"] is None


def test_focus_filter_sort_and_width_changes_retain_selection_undo_and_redo(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("editor-view-changes")) as app:
        note = jump(app, FIRST)
        original = app.client.call("editor.get")["text"]
        one = append(app, "edit one\n")
        two = append(app, "edit two\n")
        assert app.client.call("editor.undo")["text"] == one
        app.client.call("editor.cursor", {"line": 0, "column": 11, "selection": {"line": 0, "column": 6}})
        before = app.client.call("editor.get")
        assert before["can_undo"] is True and before["can_redo"] is True
        requests = [
            ("view.focus", {"enabled": True}),
            ("filter.set", {"pane": "sections", "query": "Boreal"}),
            ("filter.set", {"pane": "groups", "query": "Review"}),
            ("filter.set", {"pane": "notes", "query": "no-such-galaxy"}),
            ("view.sort", {"mode": "title"}),
            ("view.width", {"pane": "sections", "width": 333}),
            ("view.width", {"pane": "notes", "width": 444}),
            ("view.focus", {"enabled": False}),
            ("palette.open", {"mode": "themes"}),
            ("palette.activate", {"key": {"theme": "wren-light"}}),
            ("palette.open", {"mode": "themes"}),
            ("palette.activate", {"key": {"theme": "nord"}}),
        ]
        for operation, params in requests:
            gateway(app, operation, **params)
            assert editor_identity(app.client.call("editor.get")) == editor_identity(before), (operation, params)
            assert session(app)["document"]["note"] == note
        current = navigation(app)
        assert current["focus"] is False
        assert current["visible"]["notes"] == [] and current["sort"] == "title"
        assert current["preferred_widths"]["notes"] == 444
        assert app.client.call("editor.undo")["text"] == original
        assert app.client.call("editor.redo")["text"] == one
        assert app.client.call("editor.redo")["text"] == two
        saved(app, vault / FIRST, two)


def test_width_preferences_are_clamped_reset_and_persisted(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("width-preferences")) as app:
        jump(app, FIRST)
        expected = {"sections": 140, "groups": 400, "notes": 399}
        for pane, width in (("sections", 0), ("groups", 999), ("notes", 399)):
            gateway(app, "view.width", pane=pane, width=width)
        current = navigation(app)
        assert current["preferred_widths"] == expected
        for pane, width in current["effective_widths"].items():
            assert 120 <= width <= expected[pane]
        budget = min(sum(expected.values()), max(current["viewport"]["width"] - 320, 120 * len(expected)))
        assert sum(current["effective_widths"].values()) == pytest.approx(budget, abs=0.1)
        metadata = vault / ".wren/workspace.toml"
        assert tomllib.loads(metadata.read_text(encoding="utf-8"))["widths"] == {"columns": expected}
        app.restart(hard=False)
        assert navigation(app)["preferred_widths"] == expected
        command(app, "view.reset_widths")
        assert navigation(app)["preferred_widths"] == DEFAULT_WIDTHS
        app.restart(hard=False)
        assert navigation(app)["preferred_widths"] == DEFAULT_WIDTHS
        assert tomllib.loads(metadata.read_text(encoding="utf-8"))["widths"] == {"columns": DEFAULT_WIDTHS}


def test_natural_title_sort_is_per_parent_and_persists(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("per-parent-sort")) as app:
        jump(app, FIRST)
        research = item(app, "groups", "Atlas/Research")["id"]
        command(app, "notes.sort_title")
        notes = {row["id"]: row["title"] for row in session(app)["notes"]}
        assert [notes[note] for note in navigation(app)["visible"]["notes"]] == [
            "Needle amber", "Note 1", "Note 2", "Note 10",
        ]
        review = item(app, "groups", "Atlas/Review")["id"]
        core(app, "select_parent", parent={"group": review})
        assert navigation(app)["sort"] == "modified"
        core(app, "select_parent", parent={"group": research})
        assert navigation(app)["sort"] == "title"
        app.restart(hard=False)
        assert navigation(app)["sort"] == "title"
        command(app, "notes.sort_modified")
        notes = {row["id"]: row["title"] for row in session(app)["notes"]}
        assert [notes[note] for note in navigation(app)["visible"]["notes"]] == [
            "Needle amber", "Note 10", "Note 2", "Note 1",
        ]


def test_normal_onboarding_named_canonical_workspaces_and_last_active_restore_without_flags(vault_factory) -> None:
    first = vault_factory("southern-vault")
    second = vault_factory("northern-vault")
    alias = first / "Atlas/.."
    with WrenApp(evaluation=False, artifacts_dir=artifact_directory("registry-normal-startup")) as app:
        initial = session(app)
        assert initial["root"] is None and initial["document"] is None and initial["loading"] is False
        assert initial["sections"] == [] and initial["groups"] == [] and initial["notes"] == []
        assert app.client.call("editor.get")["text"] == ""
        assert navigation(app)["registrations"] == [] and navigation(app)["active_registration"] is None
        rows = palette(app, "commands")["results"]
        enabled = {row["key"]["command"] for row in rows if row["enabled"]}
        assert enabled == {
            "workspace.add", "workspace.switch", "theme.choose",
            "font.choose", "font.increase", "font.decrease", "font.reset",
            "settings.open", "settings.reload", "app.quit",
            "editor.toggle_markdown_assist", "editor.toggle_engine",
        }
        gateway(app, "palette.close")
        command(app, "note.new", disposition="blocked")
        add_workspace(app, alias, "Southern laboratory")
        jump(app, SECOND)
        gateway(app, "view.width", pane="sections", width=311)
        command(app, "notes.sort_title")
        first_state = navigation(app)
        add_workspace(app, second, "Northern laboratory")
        assert navigation(app)["preferred_widths"] == DEFAULT_WIDTHS
        assert navigation(app)["sort"] == "modified"
        jump(app, "Boreal/Workbench/Loose 5.md")
        gateway(app, "view.width", pane="groups", width=355)
        assert navigation(app)["registrations"] == [
            {"name": "Southern laboratory", "path": str(first.resolve())},
            {"name": "Northern laboratory", "path": str(second.resolve())},
        ]
        switch_workspace(app, first)
        assert session(app)["document"]["path"] == SECOND
        restored = navigation(app)
        for field in ("preferred_widths", "sort", "history", "history_position", "mru"):
            assert restored[field] == first_state[field]
        add_workspace(app, alias, "Duplicate name must not replace the original")
        assert len(navigation(app)["registrations"]) == 2
        assert navigation(app)["registrations"][0]["name"] == "Southern laboratory"
        rows = palette(app, "search", "vault-token-southern-vault")["results"]
        assert len(rows) == 1 and rows[0]["path"] == FIRST
        assert gateway(app, "palette.query", query="vault-token-northern-vault")["navigation"]["modal"]["results"] == []
        gateway(app, "palette.close")
        switch_workspace(app, second)
        assert session(app)["document"]["path"] == "Boreal/Workbench/Loose 5.md"
        assert navigation(app)["preferred_widths"]["groups"] == 355
        assert navigation(app)["active_registration"] == str(second.resolve())
        assert palette(app, "search", "vault-token-southern-vault")["results"] == []
        rows = gateway(app, "palette.query", query="vault-token-northern-vault")["navigation"]["modal"]["results"]
        assert len(rows) == 1 and rows[0]["path"] == FIRST
        gateway(app, "palette.close")
        app.restart(hard=False)
        assert Path(session(app)["root"]) == second.resolve()
        assert session(app)["document"]["path"] == "Boreal/Workbench/Loose 5.md"
        assert navigation(app)["preferred_widths"]["groups"] == 355
        assert len(navigation(app)["registrations"]) == 2
        app.restart(hard=True)
        assert Path(session(app)["root"]) == second.resolve()
        assert navigation(app)["active_registration"] == str(second.resolve())


def test_removing_active_registration_keeps_live_draft_session_and_every_file(vault: Path) -> None:
    with WrenApp(evaluation=False, artifacts_dir=artifact_directory("remove-active-registration")) as app:
        add_workspace(app, vault, "Disposable registration")
        note = jump(app, FIRST)
        original = tree_bytes(vault)
        generation = session(app)["generation"]
        with unwritable((vault / FIRST).parent):
            expected = append(app, "draft retained after removing only the registration\n")
            before = app.client.call("editor.get")
            gateway(app, "workspace.remove", path=str(vault.resolve()))
            current = session(app)
            assert current["generation"] == generation and Path(current["root"]) == vault.resolve()
            assert current["document"]["note"] == note and current["document"]["text"] == expected
            assert current["document"]["dirty"] is True
            assert editor_identity(app.client.call("editor.get")) == editor_identity(before)
            assert tree_bytes(vault) == original
            assert navigation(app)["registrations"] == [] and navigation(app)["active_registration"] is None
            assert palette(app, "workspaces")["results"][0]["key"] == {"command": "workspace.add"}
            gateway(app, "palette.close")
        core(app, "flush")
        saved(app, vault / FIRST, expected)
        assert (vault / "Atlas/Research/.private").read_bytes() == b"invented private attachment\x00"
        app.restart(hard=False)
        assert session(app)["root"] is None, "Removing last-active metadata must not delete or restore the open vault"
        assert (vault / FIRST).read_bytes() == expected.encode()


def test_folder_completions_are_tagged_cancel_safe_and_failed_or_current_open_preserves_state(vault: Path, vault_factory, tmp_path: Path) -> None:
    other = vault_factory("picker-vault")
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("tagged-folder-picker")) as app:
        jump(app, FIRST)
        original = (vault / FIRST).read_bytes()
        with unwritable((vault / FIRST).parent):
            expected = append(app, "picker cancellation retains this draft\n")
            editor_before = app.client.call("editor.get")
            before = session(app)
            response = gateway(app, "test.folder.begin", disposition="async-pending")
            tag = response["navigation"]["folder_request"]
            assert type(tag) is int
            gateway(app, "test.folder.begin", disposition="blocked")
            gateway(app, "test.folder.complete", tag=tag + 1, path=str(other), disposition="blocked")
            assert navigation(app)["folder_request"] == tag
            assert session(app)["document"]["text"] == expected
            gateway(app, "test.folder.complete", tag=tag, path=None)
            assert navigation(app)["folder_request"] is None
            assert selection_identity(session(app)) == selection_identity(before)
            assert editor_identity(app.client.call("editor.get")) == editor_identity(editor_before)
            assert session(app)["document"]["dirty"] is True
            assert (vault / FIRST).read_bytes() == original
            gateway(app, "test.folder.complete", tag=tag, path=str(other), disposition="blocked")
        core(app, "flush")
        saved(app, vault / FIRST, expected)
        gateway(app, "view.width", pane="notes", width=301)
        gateway(app, "filter.set", pane="notes", query="baseline")
        before = session(app)
        editor_before = app.client.call("editor.get")
        same = gateway(app, "workspace.add", path=str(vault / "Atlas/.."), disposition="async-pending")
        settle_workspace(app, same, vault)
        assert session(app)["generation"] == before["generation"]
        assert selection_identity(session(app)) == selection_identity(before)
        assert editor_identity(app.client.call("editor.get")) == editor_identity(editor_before)
        assert navigation(app)["preferred_widths"]["notes"] == 301 and navigation(app)["filters"]["notes"] == "baseline"
        response = gateway(app, "workspace.add", path=str(tmp_path / "does-not-exist"), disposition="async-pending")
        failed = settle_workspace(app, response, vault, disposition="blocked")
        assert failed["navigation"]["processed"]["warning"]
        assert failed["generation"] == before["generation"]
        assert selection_identity(failed) == selection_identity(before)
        assert editor_identity(app.client.call("editor.get")) == editor_identity(editor_before)
        assert not (tmp_path / "does-not-exist").exists()
        tag = gateway(app, "test.folder.begin", disposition="async-pending")["navigation"]["folder_request"]
        response = gateway(app, "test.folder.complete", tag=tag, path=str(other), disposition="async-pending")
        settle_workspace(app, response, other)
        assert navigation(app)["folder_request"] is None


@pytest.mark.parametrize("optional_file", ["global_config", "workspace_metadata"])
def test_corrupt_optional_configuration_is_preserved_while_notes_open_and_save(vault: Path, optional_file: str) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory(f"corrupt-{optional_file}")) as app:
        jump(app, FIRST)
        if optional_file == "global_config":
            path = Path(navigation(app)["config_directory"]) / "config.toml"
            assert path.is_relative_to(app.sandbox) and path.is_file()
        else:
            path = vault / ".wren/workspace.toml"
        corrupt = b"schema_version = [invented broken optional TOML\n"
        path.write_bytes(corrupt)
        app.restart(hard=True)
        assert path.read_bytes() == corrupt
        if optional_file == "global_config":
            assert navigation(app)["config_writable"] is False
            assert "config.toml" in navigation(app)["registry_warning"]
            assert navigation(app)["registrations"] == []
        else:
            assert "workspace.toml" in session(app)["banner"]
        current_path = session(app)["document"]["path"]
        destination = SECOND if current_path != SECOND else FIRST
        jump(app, destination, disposition="committed-warning" if optional_file == "workspace_metadata" else "completed")
        expected = append(app, "Markdown remains writable despite broken optional metadata: café\n")
        saved(app, vault / destination, expected)
        assert path.read_bytes() == corrupt
        if optional_file == "global_config":
            gateway(app, "workspace.remove", path=str(vault), disposition="blocked")
        else:
            command(app, "notes.sort_title", disposition="committed-warning")
            assert navigation(app)["sort"] == "title"
        assert path.read_bytes() == corrupt
        app.restart(hard=True)
        assert path.read_bytes() == corrupt and (vault / destination).read_bytes() == expected.encode()


def test_malformed_parameters_are_remote_errors_but_context_rejections_are_processed(vault: Path) -> None:
    with WrenApp(workspace=vault, artifacts_dir=artifact_directory("navigation-protocol-errors")) as app:
        jump(app, FIRST)
        for operation, params, message in (
            ("command.run", {"id": "save"}, "Unknown command ID"),
            ("palette.step", {"delta": "down"}, "delta must be i32"),
            ("view.width", {"pane": "outline", "width": 200}, "Unknown pane"),
            ("view.width", {"pane": "notes", "width": -1}, "width must be finite"),
            ("view.focus", {"enabled": "yes"}, "enabled must be boolean"),
        ):
            with pytest.raises(RemoteError, match=message):
                app.client.call(operation, params)
            assert app.client.call("identify")["pid"] == app.pid
        gateway(app, "filter.set", pane="editor", query="not a browser", disposition="blocked")
        gateway(app, "palette.activate", disposition="blocked")
        assert session(app)["document"]["path"] == FIRST


def color_count(image, viewport: dict, rectangle: tuple[float, float, float, float], color):
    sx, sy = image.width / viewport["width"], image.height / viewport["height"]
    left, top, right, bottom = rectangle
    xs = range(max(0, int(left * sx)), min(image.width, int(right * sx)))
    ys = range(max(0, int(top * sy)), min(image.height, int(bottom * sy)))
    return sum(all(abs(actual - expected) <= 3 for actual, expected in zip(image.pixel(x, y)[:3], color))
               for y in ys for x in xs)


# The note row is selected while the editor has the keyboard, so it shows the
# quieter inactive fill; the palette's active result shows the full selection.
@pytest.mark.parametrize("theme,inactive,selected,text,body,raised,accent", [
    ("dark", (0x20, 0x24, 0x2C), (0x2A, 0x31, 0x40), (0xE6, 0xE8, 0xEC), (0xD5, 0xD9, 0xE0),
     (0x1C, 0x1F, 0x26), (0xF0, 0xB4, 0x5A)),
    ("light", (0xE4, 0xE8, 0xEE), (0xD2, 0xDD, 0xEC), (0x20, 0x24, 0x2D), (0x31, 0x38, 0x44),
     (0xFF, 0xFF, 0xFF), (0xA1, 0x48, 0x08)),
])
def test_palette_search_columns_and_focus_render_real_selection_and_glyphs(
    vault: Path, theme: str, inactive: tuple, selected: tuple, text: tuple, body: tuple,
    raised: tuple, accent: tuple,
) -> None:
    artifacts = artifact_directory(f"navigation-render-{theme}")
    with WrenApp(workspace=vault, artifacts_dir=artifacts) as app:
        note = jump(app, FIRST)
        choose_theme(app, theme)
        assert session(app)["document"]["note"] == note

        def selected_row():
            rows = app.client.call("navigation.bounds")["rows"]
            return next((row["visible"] for row in rows if row["key"] == {"note": note}
                         and row["visible"]["height"] >= 40), None)

        bounds = wait_for(selected_row, timeout=5, description="actual selected note row geometry")
        image = capture(app, artifacts / "columns.png")
        viewport = navigation(app)["viewport"]
        rect = (bounds["x"] + 4, bounds["y"] + 4,
                bounds["x"] + bounds["width"] - 4, bounds["y"] + bounds["height"] - 4)
        assert color_count(image, viewport, rect, inactive) > 250
        assert color_count(image, viewport, rect, text) > 15, "The selected note title did not paint glyphs"
        for mode, query in (("commands", "New note"), ("search", "needle amber")):
            modal = palette(app, mode, query)
            assert modal["results"] and modal["active_index"] == 0
            if mode == "search":
                index = next(index for index, row in enumerate(modal["results"])
                             if row["path"] == "Atlas/Workbench/Loose 3.md")
                modal = gateway(app, "palette.step", delta=index)["navigation"]["modal"]
                assert modal["results"][modal["active_index"]]["hit"] == "content"

            def visible_palette_row():
                geometry = app.client.call("palette.bounds")
                return geometry if geometry is not None and geometry["fully_visible"] else None

            geometry = wait_for(visible_palette_row, timeout=5, description="actual visible palette selection")
            image = capture(app, artifacts / f"{mode}.png")
            current = navigation(app)
            assert current["modal"]["query"] == query
            viewport = current["viewport"]
            row = geometry["row"]
            y = row["y"] - geometry["translation_y"]
            rect = (row["x"] + 4, y + 4, row["x"] + row["width"] - 4, y + row["height"] - 4)
            assert color_count(image, viewport, rect, selected) > 400
            assert color_count(image, viewport, rect, text) > 30, "Selected palette result labels are blank"
            # palette_view centers one 680px panel, with a 32px horizontal margin.
            width = min(680, max(viewport["width"] - 32, 120))
            left = (viewport["width"] - width) / 2
            top = min(80, max(viewport["height"] * 0.16, 8))
            assert color_count(image, viewport, (left + 8, top + 8, left + width - 8, top + 45), raised) > 100
            if mode == "search":
                assert modal["results"][modal["active_index"]]["highlight"] is not None
                assert color_count(image, viewport, rect, accent) > 10, "Selected search-hit highlight glyphs are missing"
            gateway(app, "palette.close")
        gateway(app, "view.focus", enabled=True)
        assert navigation(app)["focus"] is True
        assert app.client.call("navigation.bounds")["rows"] == []
        image = capture(app, artifacts / "focus.png")
        viewport = navigation(app)["viewport"]
        assert image.pixel(image.width - 5, image.height // 2)[:3] == BACKGROUNDS[theme]
        left = editor_column(navigation(app))[0]
        assert color_count(image, viewport, (left + 16, 115, left + 500, 220), body) > 70
        assert session(app)["document"]["text"] == (vault / FIRST).read_text(encoding="utf-8")
        gateway(app, "view.focus", enabled=False)
        assert wait_for(selected_row, timeout=5)
