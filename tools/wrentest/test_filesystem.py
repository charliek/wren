"""Saved-note acceptance through the real editor, application timer, and core ops."""

from __future__ import annotations

import json
import os
from pathlib import Path
import statistics
import time

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import capture, focus_native, xdo
from tools.wrentest.test_setup import BACKGROUNDS, artifact_directory
from tools.wrentest.ui import WrenApp, wait_for
from tools.wrentest.x11 import request_close


def workspace(path: Path) -> Path:
    (path / "S1" / "G").mkdir(parents=True)
    (path / "S2").mkdir()
    (path / "S1" / "G" / "a.md").write_text("seed\n", encoding="utf-8")
    (path / "S2" / "b.md").write_text("other\n", encoding="utf-8")
    return path


def state(app: WrenApp):
    return app.client.call("session.get")


def op(app: WrenApp, operation: str, **params):
    return app.client.call("core.apply", {"op": operation, **params})


def item(snapshot, collection: str, path: str):
    return next(entry for entry in snapshot[collection] if entry["path"] == path)


def select_a(app: WrenApp):
    snapshot = state(app)
    note = item(snapshot, "notes", "S1/G/a.md")["id"]
    op(app, "open_note", note=note)
    app.client.call("editor.action", {"kind": "move", "motion": "end"})
    return note


def append(app: WrenApp, text: str):
    return app.client.call("editor.insert", {"text": text})["text"]


def saved(app: WrenApp, path: Path, expected: str):
    return wait_for(
        lambda: path.is_file() and path.read_text(encoding="utf-8") == expected
        and not state(app)["document"]["dirty"],
        timeout=5, interval=0.025, description="autosaved bytes and clean document",
    )


@pytest.mark.parametrize("theme", ["dark", "light"])
def test_saved_workspace_renders_both_themes(tmp_path: Path, theme: str) -> None:
    root = workspace(tmp_path / "notes")
    (root / "S1/G/a.md").write_text("# Visible workspace heading\n\n" + "visible editor words " * 6)
    artifacts = artifact_directory(f"workspace-{theme}")
    with WrenApp(workspace=root, artifacts_dir=artifacts) as app:
        select_a(app)
        app.client.call("theme.set", {"id": theme})
        image = capture(app, artifacts / "workspace.png")
        background = BACKGROUNDS[theme]
        assert image.pixel(image.width - 6, image.height // 2)[:3] == background
        painted = sum(any(abs(a - b) > 20 for a, b in zip(image.pixel(x, y)[:3], background))
                      for y in range(105, 300) for x in range(660, image.width - 30))
        assert painted > 150, "The workspace editor glyphs were not captured"
        assert state(app)["document"]["text"].startswith("# Visible workspace heading")


def test_timer_autosave_survives_clean_and_hard_restart(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    artifacts = artifact_directory("autosave-restart")
    with WrenApp(workspace=root, artifacts_dir=artifacts) as app:
        select_a(app)
        expected = append(app, "日本語 café 🐦 e\u0301")
        saved(app, root / "S1/G/a.md", expected)
        app.restart(hard=False)
        assert state(app)["document"]["text"] == expected
        app.restart(hard=True)
        assert state(app)["document"]["text"] == expected
        assert app.client.call("editor.get")["text"] == expected
        assert not state(app)["document"]["dirty"]
        capture(app, artifacts / "saved-workspace.png")


def test_five_thousand_line_note_saves_and_reopens(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    path = root / "S1/G/a.md"
    original = "\n".join(f"{n:04d}: Markdown **words** and `code` — café 日本語 " + "text " * 4
                         for n in range(5000))
    path.write_text(original, encoding="utf-8")
    artifacts = artifact_directory("autosave-5000")
    with WrenApp(workspace=root, artifacts_dir=artifacts) as app:
        select_a(app)
        started = time.perf_counter()
        expected = append(app, "saved by the real timer")
        saved(app, path, expected)
        timer_ms = (time.perf_counter() - started) * 1000
        samples = [state(app)["last_save_micros"]]
        for _ in range(8):
            expected = append(app, "x")
            samples.append(op(app, "flush")["last_save_micros"])
            assert path.read_text(encoding="utf-8") == expected
        assert min(samples) > 0
        app.restart(hard=True)
        assert state(app)["document"]["text"] == expected
        measurements = {
            "build": "debug", "lines": 5000, "initial_bytes": len(original.encode()),
            "samples": len(samples), "timer_to_observed_clean_ms": timer_ms,
            "core_save_us_p50": statistics.median(samples), "core_save_us_max": max(samples),
            "renderer": os.environ.get("ICED_BACKEND", "default"),
            "display": "wayland" if os.environ.get("WAYLAND_DISPLAY") else "x11",
        }
        (artifacts / "measurements.json").write_text(json.dumps(measurements, indent=2) + "\n")
        print(json.dumps(measurements, sort_keys=True))


def test_kill_one_second_after_typing_without_waiting_for_saved_ack(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("autosave-timed-kill")) as app:
        select_a(app)
        started = time.monotonic()
        expected = append(app, "survives the timed process kill")
        # This wait is intentional: kill at a fixed deadline, independently of
        # any disk check or saved acknowledgment. It is not a readiness sleep.
        time.sleep(max(0, started + 1.1 - time.monotonic()))
        app.restart(hard=True)
        assert state(app)["document"]["text"] == expected
        assert (root / "S1/G/a.md").read_text() == expected


def test_continuous_typing_persists_before_the_idle_debounce_can_fire(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("autosave-continuous")) as app:
        select_a(app)
        expected = "seed\n"
        versions = set()
        edit_times = []
        for index in range(28):
            edit_times.append(time.monotonic())
            expected = append(app, f"{index},")
            disk = (root / "S1/G/a.md").read_text()
            assert expected.startswith(disk), "Disk must contain a complete published revision"
            if disk != "seed\n":
                versions.add(disk)
            time.sleep(0.08)
        assert max(b - a for a, b in zip(edit_times, edit_times[1:])) < 0.75
        assert len(versions) >= 2, "Continuous typing never crossed multiple persistence deadlines"
        saved(app, root / "S1/G/a.md", expected)
        app.restart(hard=True)
        assert state(app)["document"]["text"] == expected


@pytest.mark.parametrize("transition", [
    "switch_note", "rename_note", "rename_group", "rename_section", "move_note",
    "move_group", "delete_note", "delete_group", "delete_section", "workspace", "quit",
])
def test_failed_save_keeps_draft_and_blocks_transition(tmp_path: Path, transition: str) -> None:
    root = workspace(tmp_path / "notes")
    other = workspace(tmp_path / "other")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory(f"save-barrier-{transition}")) as app:
        note = select_a(app)
        snapshot = state(app)
        section = item(snapshot, "sections", "S1")["id"]
        group = item(snapshot, "groups", "S1/G")["id"]
        destination = item(snapshot, "sections", "S2")["id"]
        other_note = item(snapshot, "notes", "S2/b.md")["id"]
        expected = append(app, "unsaved draft")
        app.client.call("test.fail_next", {"fault": "write"})
        with pytest.raises(RemoteError):
            if transition == "switch_note":
                op(app, "open_note", note=other_note)
            elif transition.startswith("rename_"):
                kind = transition.removeprefix("rename_")
                target = {kind: {"note": note, "group": group, "section": section}[kind]}
                op(app, "rename", target=target, new_name="Renamed")
            elif transition == "move_note":
                op(app, "move_note", note=note, to={"section": destination})
            elif transition == "move_group":
                op(app, "move_group", group=group, to=destination)
            elif transition.startswith("delete_"):
                kind = transition.removeprefix("delete_")
                op(app, "delete", target={kind: {"note": note, "group": group, "section": section}[kind]})
            elif transition == "workspace":
                app.client.call("workspace.open", {"path": str(other)})
            else:
                app.client.call("app.quit")
        after = state(app)
        assert Path(after["root"]) == root.resolve()
        assert after["document"]["note"] == note
        assert after["document"]["text"] == expected
        assert after["document"]["dirty"]
        assert after["document"]["error"]
        assert (root / "S1/G/a.md").read_text() == "seed\n"
        assert app.client.call("identify")["pid"] == app.pid
        op(app, "flush")
        assert (root / "S1/G/a.md").read_text() == expected


def test_focus_loss_flushes_without_waiting_for_the_timer(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("autosave-focus")) as app:
        select_a(app)
        expected = append(app, "flush on focus loss")
        app.client.call("window.blur")
        assert (root / "S1/G/a.md").read_text() == expected
        assert not state(app)["document"]["dirty"]


@pytest.mark.native_x11
def test_native_focus_loss_flushes_real_editor_typing(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("autosave-native-focus")) as app:
        select_a(app)
        window = focus_native(app)
        started = time.monotonic()
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "1", "native-save")
        wait_for(lambda: app.client.call("editor.get")["text"] == "seed\nnative-save", timeout=2)
        assert state(app)["document"]["dirty"]
        xdo("windowfocus", "--sync", "0")
        wait_for(lambda: (root / "S1/G/a.md").read_text() == "seed\nnative-save", timeout=2)
        assert time.monotonic() - started < 0.75, "The timer could have saved instead of native focus loss"


@pytest.mark.native_x11
@pytest.mark.parametrize("fail_first", [False, True])
def test_real_window_close_flushes_or_retains_failed_draft(tmp_path: Path, fail_first: bool) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("native-window-close")) as app:
        select_a(app)
        window = focus_native(app)
        expected = append(app, "flush on actual WM_DELETE_WINDOW")
        if fail_first:
            app.client.call("test.fail_next", {"fault": "write"})
        request_close(app.pid, window)
        if fail_first:
            wait_for(lambda: state(app)["document"]["error"], description="failed native close flush")
            assert app.returncode is None
            assert state(app)["document"]["text"] == expected
            assert state(app)["document"]["dirty"]
            assert (root / "S1/G/a.md").read_text() == "seed\n"
            op(app, "flush")
            request_close(app.pid, window)
        wait_for(lambda: app.returncode == 0, description="owned app exits after native close")
        assert (root / "S1/G/a.md").read_text() == expected


@pytest.mark.native_x11
def test_native_committed_creation_is_not_replayed_after_metadata_error(tmp_path: Path) -> None:
    assert os.geteuid() != 0, "Metadata permission acceptance requires an unprivileged user"
    root = workspace(tmp_path / "notes")
    artifacts = artifact_directory("native-create-metadata-error")
    with WrenApp(workspace=root, artifacts_dir=artifacts) as app:
        select_a(app)
        window = focus_native(app)
        metadata = root / ".wren"
        try:
            metadata.chmod(0o500)
            xdo("mousemove", "--sync", "--window", window, "80", "103")
            xdo("click", "1")
            xdo("type", "--window", window, "--clearmodifiers", "--delay", "8", "Created")
            xdo("key", "--window", window, "--clearmodifiers", "Return")
            wait_for(lambda: any(entry["path"] == "Created" for entry in state(app)["sections"]),
                     description="committed native creation")
            assert (root / "Created").is_dir()
            assert "workspace.toml" in state(app)["banner"]
            xdo("key", "--window", window, "--clearmodifiers", "Return", "ctrl+f")
            wait_for(lambda: app.client.call("editor.get")["find_open"],
                     description="creation modal dismissed despite metadata error")
            assert not (root / "Created 2").exists()
            assert len(state(app)["sections"]) == 3
            assert "workspace.toml" in state(app)["banner"]
            capture(app, artifacts / "committed-error-banner.png")
        finally:
            metadata.chmod(0o700)


def test_external_clean_reload_keeps_identity_and_clamps_cursor(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    path = root / "S1/G/a.md"
    path.write_text("first\nlong second line", encoding="utf-8")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("external-reload")) as app:
        note = select_a(app)
        app.client.call("editor.cursor", {"line": 1, "column": 12})
        before = path.stat()
        replacement = "longer first\nnew short"
        assert len(replacement) == len(path.read_text())
        path.write_text(replacement)
        os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
        wait_for(lambda: state(app)["document"]["text"] == replacement, timeout=5,
                 description="external byte revision reload")
        assert state(app)["selected_note"] == note
        editor = app.client.call("editor.get")
        assert editor["text"] == replacement
        assert editor["cursor"]["line"] == 1
        assert editor["cursor"]["column"] == len("new short")
        assert not state(app)["document"]["dirty"]


def test_dirty_external_edit_preserves_both_versions(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("external-conflict")) as app:
        select_a(app)
        local = append(app, "local draft 🐦")
        assert state(app)["document"]["dirty"]
        external = "external version 日本語\n"
        (root / "S1/G/a.md").write_text(external, encoding="utf-8")
        wait_for(lambda: state(app)["document"]["text"] == external, timeout=5,
                 description="conflict preservation and disk-version reload")
        copies = list((root / "S1/G").glob("*conflict*.md"))
        assert len(copies) == 1
        assert copies[0].read_text(encoding="utf-8") == local
        assert (root / "S1/G/a.md").read_text(encoding="utf-8") == external
        assert not state(app)["document"]["dirty"]
        assert state(app)["banner"]


def test_real_permission_failure_retains_conflicting_draft_and_blocks_quit(tmp_path: Path) -> None:
    assert os.geteuid() != 0, "Permission-denial acceptance must run as an unprivileged user"
    root = workspace(tmp_path / "notes")
    parent = root / "S1/G"
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("conflict-permission")) as app:
        select_a(app)
        local = append(app, "local draft")
        try:
            parent.chmod(0o500)
            (parent / "a.md").write_text("external bytes\n")
            wait_for(lambda: state(app)["document"]["error"], timeout=5,
                     description="failed conflict-copy persistence")
            with pytest.raises(RemoteError):
                app.client.call("app.quit")
            assert state(app)["document"]["dirty"]
            assert state(app)["document"]["text"] == local
            assert (parent / "a.md").read_text() == "external bytes\n"
            assert not list(parent.glob("*conflict*.md"))
        finally:
            parent.chmod(0o700)
        with pytest.raises(RemoteError):
            op(app, "flush")
        op(app, "flush")
        assert list(parent.glob("*conflict*.md"))[0].read_text() == local
        assert (parent / "a.md").read_text() == "external bytes\n"


def test_note_and_ancestor_moves_keep_editor_history_and_save_at_new_path(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("editor-moves")) as app:
        note = select_a(app)
        snapshot = state(app)
        section = item(snapshot, "sections", "S1")["id"]
        destination = item(snapshot, "sections", "S2")["id"]
        group = item(snapshot, "groups", "S1/G")["id"]
        original = append(app, "before moving")
        app.client.call("editor.cursor", {"line": 1, "column": 3})
        before = app.client.call("editor.get")
        op(app, "rename", target={"section": section}, new_name="Renamed")
        op(app, "move_group", group=group, to=destination)
        after = app.client.call("editor.get")
        assert after["text"] == original
        assert after["cursor"] == before["cursor"]
        assert after["can_undo"] == before["can_undo"] is True
        assert state(app)["selected_note"] == note
        assert state(app)["document"]["path"] == "S2/G/a.md"
        expected = append(app, " new location ")
        saved(app, root / "S2/G/a.md", expected)
        assert not (root / "S1/G/a.md").exists()
        assert not (root / "Renamed/G/a.md").exists()


def test_create_sanitize_and_delete_use_owned_quarantine(tmp_path: Path) -> None:
    root = tmp_path / "empty"
    root.mkdir()
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("create-trash")) as app:
        snapshot = op(app, "create_section", name="Work: Notes")
        section = snapshot["sections"][0]["id"]
        snapshot = op(app, "create_group", section=section, name="Team/One")
        group = snapshot["groups"][0]["id"]
        snapshot = op(app, "create_note", parent={"group": group}, title="My: note.md")
        note = item(snapshot, "notes", "Work- Notes/Team-One/My- note.md")["id"]
        op(app, "open_note", note=note)
        expected = append(app, "saved before delete")
        parent = root / "Work- Notes/Team-One"
        (parent / ".private").write_bytes(b"ignored file")
        op(app, "delete", target={"group": group})
        assert not parent.exists()
        assert state(app)["document"] is None
        trash = app.sandbox / "trash"
        assert any(path.read_text() == expected for path in trash.rglob("*.md"))
        assert any(path.read_bytes() == b"ignored file" for path in trash.rglob(".private"))
        assert root.exists()


def test_workspace_switch_flushes_and_failed_switch_preserves_the_old_session(tmp_path: Path) -> None:
    first = workspace(tmp_path / "first")
    second = workspace(tmp_path / "second")
    with WrenApp(workspace=first, artifacts_dir=artifact_directory("workspace-switch")) as app:
        select_a(app)
        expected = append(app, "flush before switch")
        generation = state(app)["generation"]
        switched = app.client.call("workspace.open", {"path": str(second)})
        assert Path(switched["root"]) == second.resolve()
        assert switched["generation"] > generation
        assert (first / "S1/G/a.md").read_text() == expected
        assert not app.client.call("editor.get")["can_undo"]
        with pytest.raises(RemoteError):
            app.client.call("workspace.open", {"path": str(tmp_path / "does-not-exist")})
        assert Path(state(app)["root"]) == second.resolve()
        restored = app.client.call("workspace.open", {"path": str(first)})
        assert Path(restored["root"]) == first.resolve()
        assert restored["document"]["text"] == expected


def test_corrupt_metadata_never_blocks_markdown_saves_or_gets_overwritten(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    (root / ".wren").mkdir()
    metadata = root / ".wren/workspace.toml"
    original = b"layout = [malformed user metadata\n"
    metadata.write_bytes(original)
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("corrupt-metadata")) as app:
        select_a(app)
        expected = append(app, "saved despite optional metadata error")
        saved(app, root / "S1/G/a.md", expected)
        assert metadata.read_bytes() == original
        op(app, "create_section", name="Still writable")
        assert metadata.read_bytes() == original
        app.restart(hard=True)
        assert (root / "S1/G/a.md").read_text() == expected
        assert metadata.read_bytes() == original


def test_editor_fixture_hook_cannot_bypass_a_saved_document(tmp_path: Path) -> None:
    root = workspace(tmp_path / "notes")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory("editor-fixture-guard")) as app:
        select_a(app)
        with pytest.raises(RemoteError):
            app.client.call("editor.set", {"text": "unsafe bypass"})
        assert state(app)["document"]["text"] == "seed\n"
        assert (root / "S1/G/a.md").read_text() == "seed\n"
