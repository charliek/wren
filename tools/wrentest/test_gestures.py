"""Native X11 keyboard, divider, and file-drag acceptance on owned workspaces.

RPCs arrange fixtures and inspect state/real widget geometry. Every gesture under
assertion is xdotool input, never a drag, resize, or palette-activation adapter.
Run through tools/run_display.py x11, not on an existing user desktop.
"""

from __future__ import annotations

from contextlib import contextmanager
import os
from pathlib import Path
import stat
import time
import tomllib

import pytest

from tools.wrentest.test_editor import focus_native, xdo
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for


SECTIONS = ("A Visible", "B Hidden", "C Visible", "D Hidden")
GROUPS = tuple(f"{SECTIONS[0]}/{name}" for name in SECTIONS)
SOURCE = GROUPS[0]
DESTINATION = GROUPS[2]
REMOTE = f"{SECTIONS[1]}/Remote"
ORIGIN = f"{SOURCE}/Origin.md"
PICKER_A = f"{SOURCE}/Picker A.md"
PICKER_B = f"{SOURCE}/Picker B.md"
DEFAULT_WIDTHS = {
    "columns": {"sections": 200, "groups": 220, "notes": 280},
    "outline": {"sections": 260, "notes": 280},
}
WIDTH_LIMITS = {
    "columns": {"sections": (140, 360), "groups": (150, 400), "notes": (200, 480)},
    "outline": {"sections": (180, 420), "notes": (200, 480)},
}


def session(app: WrenApp):
    return app.client.call("session.get")


def navigation(app: WrenApp):
    return app.client.call("navigation.get")


def core(app: WrenApp, operation: str, **params):
    return app.client.call("core.apply", {"op": operation, **params})


def fixture_gateway(app: WrenApp, operation: str, **params):
    result = app.client.call(operation, params)
    assert result["disposition"] == "completed", result
    assert result["warning"] is None, result
    return result["navigation"]


def item(app: WrenApp, collection: str, path: str):
    matches = [entry for entry in session(app)[collection] if entry["path"] == path]
    assert len(matches) == 1, (collection, path, matches)
    return matches[0]


def editor_identity(app: WrenApp):
    snapshot = app.client.call("editor.get")
    return {key: snapshot[key] for key in (
        "text", "cursor", "selection", "selected_text", "revision", "can_undo", "can_redo",
    )}


def selection(snapshot):
    return {key: snapshot[key] for key in ("selected_section", "selected_group", "selected_note")}


def files(root: Path):
    return {path.relative_to(root).as_posix(): path.read_bytes()
            for path in root.rglob("*")
            if path.is_file() and path.relative_to(root).parts[0] != ".wren"}


def metadata(root: Path):
    return tomllib.loads((root / ".wren/workspace.toml").read_text(encoding="utf-8"))


def keys(window: str, *names: str) -> None:
    xdo("key", "--window", window, "--clearmodifiers", *names)


def settle_focus() -> None:
    # iced moves Tab focus with a widget operation that runs after the current
    # event batch; keys typed in the same batch would reach the old widget.
    time.sleep(0.3)


def type_text(window: str, text: str) -> None:
    xdo("type", "--window", window, "--clearmodifiers", "--delay", "5", text)


def move(window: str, point: tuple[int, int]) -> None:
    # --sync waits for a new motion event, which never arrives at the same point.
    # Observe the server position instead; gesture outcomes are polled separately.
    geometry = dict(line.split("=", 1) for line in xdo("getwindowgeometry", "--shell", window).splitlines())
    expected = (int(geometry["X"]) + point[0], int(geometry["Y"]) + point[1])
    xdo("mousemove", "--window", window, str(point[0]), str(point[1]))

    def positioned():
        position = dict(line.split("=", 1) for line in xdo("getmouselocation", "--shell").splitlines())
        return (int(position["X"]), int(position["Y"])) == expected
    wait_for(positioned, timeout=3, interval=0.025, description="native pointer position")


def bounds(app: WrenApp):
    def measured():
        snapshot = app.client.call("navigation.bounds")
        root = snapshot["root"]
        return snapshot if root["width"] > 0 and root["height"] > 0 and snapshot["rows"] else None
    return wait_for(measured, timeout=5, interval=0.025, description="real browser geometry")


def visible_row(app: WrenApp, key: dict):
    def measured():
        matches = [row for row in bounds(app)["rows"] if row["key"] == key]
        assert len(matches) <= 1, (key, matches)
        if not matches:
            return None
        visible = matches[0]["visible"]
        return matches[0] if visible["width"] > 8 and visible["height"] > 8 else None
    return wait_for(measured, timeout=5, interval=0.025, description=f"visible row {key}")


def point_in(rectangle: dict, *, vertical: float = 0.5) -> tuple[int, int]:
    point = (round(rectangle["x"] + rectangle["width"] / 2),
             round(rectangle["y"] + rectangle["height"] * vertical))
    assert rectangle["x"] <= point[0] < rectangle["x"] + rectangle["width"]
    assert rectangle["y"] <= point[1] < rectangle["y"] + rectangle["height"]
    return point


@contextmanager
def held_button(window: str, point: tuple[int, int]):
    move(window, point)
    try:
        xdo("mousedown", "1")
        yield
    finally:
        xdo("mouseup", "1")


@contextmanager
def row_drag(app: WrenApp, window: str, key: dict):
    with held_button(window, point_in(visible_row(app, key)["visible"])):
        wait_for(lambda: bounds(app)["pressed"], timeout=3, interval=0.025,
                 description="native row press is owned by the drag root")
        yield


def preview(app: WrenApp, source: dict, target: dict | None, *, insertion: bool = False):
    def observed():
        current = navigation(app)["gesture_preview"]
        return current if (current is not None and current["source"] == source
                           and current["target"] == target) else None
    current = wait_for(observed, timeout=3, interval=0.025,
                       description=f"native drag preview {source} to {target}")
    assert (current["insertion"] is not None) == insertion, current
    return current


def idle(app: WrenApp) -> None:
    def observed():
        current = navigation(app)
        return (current["gesture_preview"] is None and current["spring_section"] is None
                and not bounds(app)["pressed"])
    wait_for(observed, timeout=3, interval=0.025, description="native drag and release settled")


def seam(app: WrenApp, pane: str) -> tuple[int, int]:
    current = navigation(app)
    assert current["focus"] is False
    panes = list(DEFAULT_WIDTHS[current["layout"]])
    assert pane in panes
    assert set(current["effective_widths"]) == set(panes)
    root = bounds(app)["root"]
    x = root["x"] + sum(current["effective_widths"][key]
                        for key in panes[:panes.index(pane) + 1])
    point = (round(x), round(root["y"] + root["height"] / 2))
    assert root["x"] < point[0] < root["x"] + root["width"]
    return point


def wait_width(app: WrenApp, layout: str, pane: str, width: float):
    def observed():
        current = navigation(app)
        return current if current["preferred_widths"][layout][pane] == pytest.approx(width) else None
    return wait_for(observed, timeout=3, interval=0.025,
                    description=f"native {layout}/{pane} width {width}")


def stored_width(root: Path, layout: str, pane: str, width: float) -> None:
    wait_for(lambda: metadata(root).get("widths", {}).get(layout, {}).get(pane) == width,
             timeout=3, interval=0.025, description=f"persisted {layout}/{pane} width {width}")


def resize(app: WrenApp, root: Path, window: str, pane: str, delta: int):
    before = navigation(app)
    layout = before["layout"]
    expected = {key: widths.copy() for key, widths in before["preferred_widths"].items()}
    minimum, maximum = WIDTH_LIMITS[layout][pane]
    expected[layout][pane] = min(maximum, max(minimum, expected[layout][pane] + delta))
    old_disk = (root / ".wren/workspace.toml").read_bytes()
    start = seam(app, pane)
    with held_button(window, start):
        move(window, (start[0] + delta, start[1]))
        during = wait_width(app, layout, pane, expected[layout][pane])
        assert during["preferred_widths"] == expected
        assert during["processed"] == before["processed"], "Live resize must not dispatch persistence"
        assert (root / ".wren/workspace.toml").read_bytes() == old_disk
        assert_browser_geometry(app, during)
    stored_width(root, layout, pane, expected[layout][pane])
    after = navigation(app)
    assert after["preferred_widths"] == expected
    assert after["processed"]["disposition"] == "completed", after["processed"]
    assert after["processed"]["correlation"] > before["processed"]["correlation"]
    idle(app)
    return after


def window_size(app: WrenApp, window: str, width: int, height: int):
    xdo("windowsize", window, str(width), str(height))

    def observed():
        current = navigation(app)
        measured = bounds(app)["root"]
        return current if (current["viewport"] == {"width": width, "height": height}
                           and measured["width"] == pytest.approx(width)) else None
    return wait_for(observed, timeout=5, interval=0.025, description=f"native window size {width}x{height}")


def assert_browser_geometry(app: WrenApp, current: dict) -> None:
    def measured_panes():
        measured = bounds(app)
        root = measured["root"]
        left = root["x"]
        kinds = (({"section"}, {"group"}, {"note"})
                 if current["layout"] == "columns" else ({"section", "group"}, {"note"}))
        for pane, row_kinds in zip(DEFAULT_WIDTHS[current["layout"]], kinds):
            right = left + current["effective_widths"][pane]
            rows = [row for row in measured["rows"] if set(row["key"]) <= row_kinds]
            if not rows:
                return None
            for row in rows:
                visible = row["visible"]
                if not (left - 0.5 <= visible["x"] < right
                        and visible["x"] + visible["width"] <= right + 0.5
                        and abs(visible["x"] + visible["width"] / 2 - (left + right) / 2) <= 1
                        and visible["width"] > 80
                        and root["y"] <= visible["y"] < root["y"] + root["height"]):
                    return None
            left = right
        return measured if root["x"] + root["width"] - left >= 320 - 0.5 else None
    wait_for(measured_panes, timeout=5, interval=0.025,
             description="actual row geometry follows pane widths and retains the editor minimum")


@contextmanager
def unwritable(directory: Path):
    assert os.geteuid() != 0, "Dirty-draft isolation requires an unprivileged test process"
    mode = stat.S_IMODE(directory.stat().st_mode)
    try:
        directory.chmod(mode & ~0o222)
        yield
    finally:
        directory.chmod(mode)


@pytest.fixture
def native_workspace(tmp_path: Path, request):
    assert os.environ.get("DISPLAY"), "native_x11 requires an X11 display"
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    assert Path(os.environ.get("XDG_RUNTIME_DIR", "")).name.startswith("wren-display-"), (
        "Run with tools/run_display.py x11; these gestures must not target a user desktop"
    )
    root = tmp_path / "notes"
    for relative in (*SECTIONS, *GROUPS, REMOTE):
        (root / relative).mkdir(parents=True, exist_ok=True)
    contents = {
        ORIGIN: "source paragraph\nsecond line for preserved selection\n",
        PICKER_A: "first picker document\n",
        PICKER_B: "second picker document\n",
        f"{DESTINATION}/A resident.md": "destination resident must not open on drop\n",
        f"{REMOTE}/Northern.md": "remote group document\n",
    }
    for relative, text in contents.items():
        path = root / relative
        path.write_text(text, encoding="utf-8")
        os.utime(path, ns=(1_700_000_000_000_000_000, 1_700_000_000_000_000_000))
    (root / SOURCE / ".private").write_bytes(b"owned ignored attachment\x00")
    with WrenApp(workspace=root, artifacts_dir=artifact_directory(request.node.name)) as app:
        note = item(app, "notes", ORIGIN)["id"]
        core(app, "open_note", note=note)
        assert session(app)["document"]["path"] == ORIGIN
        assert navigation(app)["preferred_widths"] == DEFAULT_WIDTHS
        wait_for(lambda: (root / ".wren/workspace.toml").is_file(), timeout=3,
                 description="fixture selection metadata")
        window = focus_native(app)
        assert int(xdo("getwindowpid", window)) == app.pid
        window_size(app, window, 1280, 800)
        bounds(app)
        sandbox = app.sandbox
        try:
            yield app, root, window
        finally:
            quarantine = Path(session(app)["trash_path"])
            assert quarantine == app.sandbox / "trash"
            assert list(quarantine.iterdir()) == [], "Gesture tests must never send anything to trash"
    assert not sandbox.exists(), "The owned app sandbox was not removed after verified process teardown"


@pytest.mark.native_x11
def test_native_command_palette_focus_typing_arrows_and_enter(native_workspace) -> None:
    app, root, window = native_workspace
    before = editor_identity(app)
    document = session(app)["document"]
    original_files = files(root)
    keys(window, "alt+shift+p")
    wait_for(lambda: navigation(app)["modal"] is not None, description="Alt+Shift+P opens commands")
    assert navigation(app)["modal"]["mode"] == "commands"
    type_text(window, "lyt")
    wait_for(lambda: navigation(app)["modal"]["query"] == "lyt", description="native command query focus")
    modal = navigation(app)["modal"]
    assert [row["key"] for row in modal["results"]] == [
        {"command": "view.columns"}, {"command": "view.outline"},
    ]
    assert modal["active_index"] == 0
    keys(window, "Down")
    wait_for(lambda: navigation(app)["modal"]["active_key"] == {"command": "view.outline"},
             description="native Down selects Outline")
    keys(window, "Up")
    wait_for(lambda: navigation(app)["modal"]["active_key"] == {"command": "view.columns"},
             description="native Up returns to Columns")
    keys(window, "Down", "Return")
    wait_for(lambda: navigation(app)["layout"] == "outline" and navigation(app)["modal"] is None,
             description="native Enter runs the highlighted layout command")
    assert metadata(root)["layout"] == "outline"
    assert editor_identity(app) == before
    assert session(app)["document"] == document
    assert files(root) == original_files


@pytest.mark.native_x11
def test_native_ctrl_p_jump_types_moves_and_opens_the_highlighted_note(native_workspace) -> None:
    app, root, window = native_workspace
    original_files = files(root)
    before = editor_identity(app)
    first = item(app, "notes", PICKER_A)["id"]
    second = item(app, "notes", PICKER_B)["id"]
    keys(window, "alt+p")
    wait_for(lambda: navigation(app)["modal"] is not None, description="Alt+P opens Jump")
    assert navigation(app)["modal"]["mode"] == "jump"
    type_text(window, "Picker")
    wait_for(lambda: navigation(app)["modal"]["query"] == "Picker", description="native Jump query focus")
    modal = navigation(app)["modal"]
    assert [row["key"] for row in modal["results"]] == [{"note": first}, {"note": second}]
    assert modal["active_key"] == {"note": first}
    assert editor_identity(app) == before, "Typing into Jump leaked into the open document"
    keys(window, "Down")
    wait_for(lambda: navigation(app)["modal"]["active_key"] == {"note": second},
             description="native Jump Down selection")
    keys(window, "Return")
    wait_for(lambda: session(app)["document"]["path"] == PICKER_B and navigation(app)["modal"] is None,
             description="native Jump Enter opens the highlighted note, not the first result")
    assert session(app)["selected_note"] == second
    assert app.client.call("editor.get")["text"] == original_files[PICKER_B].decode("utf-8")
    assert session(app)["document"]["dirty"] is False
    assert files(root) == original_files


@pytest.mark.native_x11
def test_native_escape_and_scrim_dismiss_palettes_without_typing_or_click_through(native_workspace) -> None:
    app, root, window = native_workspace
    keys(window, "ctrl+a")
    wait_for(lambda: app.client.call("editor.get")["selected_text"] == session(app)["document"]["text"],
             description="native editor selection before modal isolation")
    before_editor = editor_identity(app)
    before_session = session(app)
    original_files = files(root)
    keys(window, "alt+shift+p")
    wait_for(lambda: navigation(app)["modal"] is not None, description="commands overlay opens")
    type_text(window, "unmatched-gesture-query")
    wait_for(lambda: navigation(app)["modal"]["query"] == "unmatched-gesture-query",
             description="commands input consumes native typing")
    assert navigation(app)["modal"]["results"] == []
    assert editor_identity(app) == before_editor
    keys(window, "Escape")
    wait_for(lambda: navigation(app)["modal"] is None, description="native Escape dismisses palette")
    assert editor_identity(app) == before_editor
    assert session(app)["document"] == before_session["document"]
    keys(window, "alt+p")
    wait_for(lambda: navigation(app)["modal"] is not None, description="Jump overlay reopens")
    type_text(window, "Picker")
    wait_for(lambda: navigation(app)["modal"]["query"] == "Picker", description="Jump input consumes typing")
    other_section = item(app, "sections", SECTIONS[1])["id"]
    at = point_in(visible_row(app, {"section": other_section})["visible"])
    palette_bounds = wait_for(lambda: app.client.call("palette.bounds"), description="actual palette result bounds")
    assert at[0] < palette_bounds["viewport"]["x"], "The dismissal click must land on the scrim"
    move(window, at)
    xdo("click", "1")
    wait_for(lambda: navigation(app)["modal"] is None, description="native scrim press dismisses Jump")
    idle(app)
    assert selection(session(app)) == selection(before_session)
    assert session(app)["document"] == before_session["document"]
    assert editor_identity(app) == before_editor
    assert files(root) == original_files


@pytest.mark.native_x11
def test_native_dividers_update_live_clamp_and_persist_independently_on_restart(native_workspace) -> None:
    app, root, window = native_workspace
    before_editor = editor_identity(app)
    before_document = session(app)["document"]
    maximum = resize(app, root, window, "sections", 500)
    assert maximum["preferred_widths"]["columns"]["sections"] == 360
    columns = resize(app, root, window, "sections", -240)
    assert columns["preferred_widths"]["columns"]["sections"] == 140
    assert columns["preferred_widths"]["outline"] == DEFAULT_WIDTHS["outline"]
    keys(window, "ctrl+alt+2")
    wait_for(lambda: navigation(app)["layout"] == "outline", description="native Outline shortcut")
    outline = resize(app, root, window, "notes", 64)
    assert outline["preferred_widths"]["outline"]["notes"] == 344
    assert outline["preferred_widths"]["columns"] == columns["preferred_widths"]["columns"]
    assert editor_identity(app) == before_editor
    assert session(app)["document"] == before_document
    expected = outline["preferred_widths"]
    app.restart(hard=False)
    assert navigation(app)["layout"] == "outline"
    assert navigation(app)["preferred_widths"] == expected
    table = metadata(root)["widths"]
    assert table["columns"]["sections"] == 140
    assert table["outline"]["notes"] == 344
    assert session(app)["document"]["path"] == ORIGIN
    assert session(app)["document"]["text"] == before_document["text"]


@pytest.mark.native_x11
def test_native_divider_escape_restores_width_without_touching_dirty_text_or_selection(native_workspace) -> None:
    app, root, window = native_workspace
    original = session(app)["document"]["text"]
    expected_text = original + "unsaved divider draft"
    with unwritable(root / SOURCE):
        keys(window, "ctrl+End")
        type_text(window, "unsaved divider draft")
        wait_for(lambda: app.client.call("editor.get")["text"] == expected_text,
                 description="native dirty editor fixture")
        keys(window, "ctrl+a")
        wait_for(lambda: app.client.call("editor.get")["selected_text"] == expected_text,
                 description="native dirty selection before seam press")
        before_editor = editor_identity(app)
        before = navigation(app)
        old_disk = (root / ".wren/workspace.toml").read_bytes()
        start = seam(app, "notes")
        with held_button(window, start):
            move(window, (start[0] + 80, start[1]))
            wait_width(app, "columns", "notes", 360)
            keys(window, "Escape")
            wait_width(app, "columns", "notes", 280)
            assert editor_identity(app) == before_editor
        idle(app)
        assert navigation(app)["preferred_widths"] == before["preferred_widths"]
        assert (root / ".wren/workspace.toml").read_bytes() == old_disk
        assert editor_identity(app) == before_editor
        document = session(app)["document"]
        assert document["path"] == ORIGIN and document["text"] == expected_text
        assert document["dirty"] is True
        assert (root / ORIGIN).read_text(encoding="utf-8") == original
    core(app, "flush")
    assert (root / ORIGIN).read_text(encoding="utf-8") == expected_text


@pytest.mark.native_x11
def test_native_double_click_resets_only_the_target_divider(native_workspace) -> None:
    app, root, window = native_workspace
    before_editor = editor_identity(app)
    resize(app, root, window, "sections", 32)
    resized = resize(app, root, window, "groups", 64)
    assert resized["preferred_widths"]["columns"]["groups"] == 284
    expected = {layout: widths.copy() for layout, widths in resized["preferred_widths"].items()}
    expected["columns"]["groups"] = 220
    move(window, seam(app, "groups"))
    xdo("click", "--repeat", "2", "--delay", "80", "1")
    wait_width(app, "columns", "groups", 220)
    stored_width(root, "columns", "groups", 220)
    idle(app)
    assert navigation(app)["preferred_widths"] == expected
    assert metadata(root)["widths"]["columns"]["sections"] == 232
    assert editor_identity(app) == before_editor
    assert session(app)["document"]["dirty"] is False


@pytest.mark.native_x11
def test_native_focused_divider_steps_sixteen_pixels_and_tab_traverses_real_widgets(native_workspace) -> None:
    app, root, window = native_workspace
    before_editor = editor_identity(app)
    move(window, seam(app, "sections"))
    xdo("click", "1")
    stored_width(root, "columns", "sections", 200)
    for key, expected in (("Right", 216), ("Right", 232), ("Left", 216)):
        keys(window, key)
        wait_width(app, "columns", "sections", expected)
        stored_width(root, "columns", "sections", expected)
    keys(window, "Tab")
    settle_focus()
    type_text(window, "Visible")
    wait_for(lambda: navigation(app)["filters"]["groups"] == "Visible",
             description="Tab moves from the Sections grip to the real Groups input")
    assert navigation(app)["preferred_widths"]["columns"]["sections"] == 216
    move(window, seam(app, "sections"))
    xdo("click", "1")
    keys(window, "Right")
    wait_width(app, "columns", "sections", 232)
    stored_width(root, "columns", "sections", 232)
    keys(window, "shift+Tab")
    settle_focus()
    type_text(window, "Visible")
    wait_for(lambda: navigation(app)["filters"]["sections"] == "Visible",
             description="Shift+Tab moves from the grip to the real Sections input")
    after = navigation(app)
    assert after["filters"]["groups"] == after["filters"]["sections"] == "Visible"
    assert after["preferred_widths"]["columns"] == {"sections": 232, "groups": 220, "notes": 280}
    assert after["preferred_widths"]["outline"] == DEFAULT_WIDTHS["outline"]
    assert editor_identity(app) == before_editor
    assert session(app)["document"]["dirty"] is False


@pytest.mark.native_x11
def test_native_narrow_window_compresses_actual_panes_and_wide_restores_preferred_widths(native_workspace) -> None:
    app, root, window = native_workspace
    before_editor = editor_identity(app)
    for pane, delta in (("sections", 80), ("groups", 80), ("notes", 60)):
        resize(app, root, window, pane, delta)
    preferred = navigation(app)["preferred_widths"]
    assert preferred["columns"] == {"sections": 280, "groups": 300, "notes": 340}
    old_disk = (root / ".wren/workspace.toml").read_bytes()
    narrow = window_size(app, window, 760, 700)
    assert narrow["preferred_widths"] == preferred
    factor = (760 - 320) / sum(preferred["columns"].values())
    for pane, width in narrow["effective_widths"].items():
        assert 120 <= width < preferred["columns"][pane]
        assert width == pytest.approx(preferred["columns"][pane] * factor, abs=0.1)
    assert_browser_geometry(app, narrow)
    assert (root / ".wren/workspace.toml").read_bytes() == old_disk
    compressed = resize(app, root, window, "sections", 32)
    assert compressed["preferred_widths"]["columns"]["sections"] == 312, (
        "A compressed seam must anchor its drag to the preferred, not effective, width"
    )
    assert_browser_geometry(app, compressed)
    old_disk = (root / ".wren/workspace.toml").read_bytes()
    wide = window_size(app, window, 1280, 800)
    assert wide["preferred_widths"] == compressed["preferred_widths"]
    assert wide["effective_widths"] == wide["preferred_widths"]["columns"]
    assert_browser_geometry(app, wide)
    assert (root / ".wren/workspace.toml").read_bytes() == old_disk
    assert editor_identity(app) == before_editor


@pytest.mark.native_x11
def test_native_note_drop_moves_one_file_and_never_activates_the_destination_row(native_workspace) -> None:
    app, root, window = native_workspace
    note = item(app, "notes", ORIGIN)["id"]
    group = item(app, "groups", DESTINATION)["id"]
    keys(window, "ctrl+a")
    wait_for(lambda: app.client.call("editor.get")["selected_text"] == session(app)["document"]["text"],
             description="native selection retained across note move")
    before_editor = editor_identity(app)
    before = session(app)
    original_files = files(root)
    inode = (root / ORIGIN).stat().st_ino
    source, target = {"note": note}, {"group": group}
    with row_drag(app, window, source):
        move(window, point_in(visible_row(app, target)["visible"]))
        preview(app, source, target)
        assert session(app)["document"] == before["document"], "Hover must not activate the destination"
        assert files(root) == original_files
    destination = f"{DESTINATION}/Origin.md"
    wait_for(lambda: session(app)["document"]["path"] == destination,
             description="native drop moves the active note")
    idle(app)
    expected_files = original_files.copy()
    expected_files[destination] = expected_files.pop(ORIGIN)
    assert files(root) == expected_files, "A drop must produce exactly one source/destination path delta"
    assert (root / destination).stat().st_ino == inode
    after = session(app)
    assert after["document"]["note"] == note and after["selected_note"] == note
    assert after["selected_group"] == group
    assert after["document"]["dirty"] is False
    assert len(after["notes"]) == len(before["notes"])
    assert {entry["id"] for entry in after["notes"]} == {entry["id"] for entry in before["notes"]}
    assert editor_identity(app) == before_editor
    for field in ("history", "mru"):
        assert after["navigation"][field] == [destination if path == ORIGIN else path
                                               for path in before["navigation"][field]]
    assert after["navigation"]["processed"]["disposition"] == "completed"
    app.restart(hard=False)
    assert session(app)["document"]["path"] == destination
    assert files(root) == expected_files


@pytest.mark.native_x11
def test_native_invalid_note_drop_and_escape_cancel_preserve_disk_document_and_selection(native_workspace) -> None:
    app, root, window = native_workspace
    note = item(app, "notes", ORIGIN)["id"]
    other_note = item(app, "notes", PICKER_B)["id"]
    group = item(app, "groups", DESTINATION)["id"]
    keys(window, "ctrl+a")
    wait_for(lambda: app.client.call("editor.get")["selected_text"] == session(app)["document"]["text"],
             description="native editor selection before invalid/cancelled drops")
    before_editor = editor_identity(app)
    before = session(app)
    original_files = files(root)
    old_metadata = (root / ".wren/workspace.toml").read_bytes()
    for target, cancel in (({"note": other_note}, False), ({"group": group}, True)):
        with row_drag(app, window, {"note": note}):
            move(window, point_in(visible_row(app, target)["visible"]))
            preview(app, {"note": note}, target if cancel else None)
            if cancel:
                keys(window, "Escape")
                wait_for(lambda: navigation(app)["gesture_preview"] is None,
                         description="native Escape cancels a valid file drag before release")
        idle(app)
        after = session(app)
        assert selection(after) == selection(before), "A cancelled release clicked through to a row"
        assert after["document"] == before["document"]
        for field in ("history", "history_position", "mru", "processed"):
            assert after["navigation"][field] == before["navigation"][field]
        assert editor_identity(app) == before_editor
        assert files(root) == original_files
        assert (root / ".wren/workspace.toml").read_bytes() == old_metadata


@pytest.mark.native_x11
def test_native_spring_hover_is_transient_and_note_drop_survives_hidden_source(native_workspace) -> None:
    app, root, window = native_workspace
    note = item(app, "notes", ORIGIN)["id"]
    source_group = item(app, "groups", SOURCE)["id"]
    other_section = item(app, "sections", SECTIONS[1])["id"]
    destination_group = item(app, "groups", REMOTE)["id"]
    before_editor = editor_identity(app)
    before = session(app)
    original_files = files(root)
    old_metadata = (root / ".wren/workspace.toml").read_bytes()
    source = {"note": note}
    with row_drag(app, window, source):
        hover_point = point_in(visible_row(app, {"section": other_section})["visible"])
        hover_started = time.monotonic()
        move(window, hover_point)
        preview(app, source, None)

        def sprung_without_navigation():
            current = session(app)
            assert selection(current) == selection(before)
            assert current["document"] == before["document"]
            assert editor_identity(app) == before_editor
            for field in ("history", "history_position", "mru", "processed", "generation"):
                assert current["navigation"][field] == before["navigation"][field]
            assert (root / ".wren/workspace.toml").read_bytes() == old_metadata
            return current if current["navigation"]["spring_section"] == other_section else None

        wait_for(sprung_without_navigation, timeout=5, interval=0.025,
                 description="600ms native hover springs the other section without selecting a note")
        assert time.monotonic() - hover_started >= 0.5, "Spring-open fired without the hover dwell"
        destination = visible_row(app, {"group": destination_group})
        measured = bounds(app)
        assert measured["pressed"] is True
        assert source not in [row["key"] for row in measured["rows"]]
        assert {"group": source_group} not in [row["key"] for row in measured["rows"]]
        assert files(root) == original_files
        move(window, point_in(destination["visible"]))
        preview(app, source, {"group": destination_group})
        assert session(app)["document"] == before["document"]
    destination_path = f"{REMOTE}/Origin.md"
    wait_for(lambda: session(app)["document"]["path"] == destination_path,
             description="drop commits after spring-open hid the source scroll subtree")
    idle(app)
    expected_files = original_files.copy()
    expected_files[destination_path] = expected_files.pop(ORIGIN)
    assert files(root) == expected_files
    assert session(app)["selected_note"] == note
    assert session(app)["selected_section"] == other_section
    assert session(app)["selected_group"] == destination_group
    assert editor_identity(app) == before_editor


@pytest.mark.native_x11
def test_native_filtered_section_and_group_reorders_keep_full_siblings_and_persist(native_workspace) -> None:
    app, root, window = native_workspace
    before_editor = editor_identity(app)
    before_document = session(app)["document"]
    original_files = files(root)
    expected_sections = [SECTIONS[1], SECTIONS[2], SECTIONS[0], SECTIONS[3]]
    expected_groups = [GROUPS[1], GROUPS[2], GROUPS[0], GROUPS[3]]
    for kind, collection, paths, expected, pane in (
        ("section", "sections", SECTIONS, expected_sections, "sections"),
        ("group", "groups", GROUPS, expected_groups, "groups"),
    ):
        fixture_gateway(app, "filter.set", pane=pane, query="Visible")
        source = {kind: item(app, collection, paths[0])["id"]}
        target = {kind: item(app, collection, paths[2])["id"]}
        hidden_ids = {item(app, collection, paths[index])["id"] for index in (1, 3)}
        assert not hidden_ids.intersection(navigation(app)["visible"][collection])
        with row_drag(app, window, source):
            move(window, point_in(visible_row(app, target)["visible"], vertical=0.8))
            current = preview(app, source, target, insertion=True)
            assert current["insertion"]["height"] > 0
            assert files(root) == original_files
        idle(app)
        siblings = [row["path"] for row in session(app)[collection]
                    if kind == "section" or Path(row["path"]).parent.as_posix() == SECTIONS[0]]
        assert siblings == expected, "The hidden successor must stay after the insertion, not before it"
        order = metadata(root)["order"]
        assert [path for path in order if path in paths] == expected
        assert session(app)["document"] == before_document
        assert editor_identity(app) == before_editor
        fixture_gateway(app, "filter.set", pane=pane, query="")
        by_id = {row["id"]: row["path"] for row in session(app)[collection]}

        def rendered_order():
            rendered = sorted((row for row in bounds(app)["rows"] if kind in row["key"]),
                              key=lambda row: row["visible"]["y"])
            return [by_id[row["key"][kind]] for row in rendered]

        wait_for(lambda: rendered_order() == expected, timeout=5, interval=0.025,
                 description=f"actual unfiltered {collection} rows retain full sibling order")
    assert files(root) == original_files
    app.restart(hard=False)
    after = session(app)
    assert [row["path"] for row in after["sections"]] == expected_sections
    assert [row["path"] for row in after["groups"]
            if Path(row["path"]).parent.as_posix() == SECTIONS[0]] == expected_groups
    assert after["document"]["path"] == ORIGIN
    assert after["document"]["text"] == before_document["text"]
    assert files(root) == original_files


@pytest.mark.native_x11
def test_native_clicks_leave_exactly_one_text_widget_focused(native_workspace) -> None:
    app, root, window = native_workspace
    before_document = session(app)["document"]
    original_files = files(root)

    def focus():
        return app.client.call("focus.get")

    def click(name: str) -> None:
        move(window, point_in(focus()["bounds"][name]))
        xdo("click", "1")
        try:
            wait_for(lambda: (lambda state: state if state["named"] == [name] and state["unnamed"] == 0 else None)(focus()),
                     timeout=3, description=f"{name} is the only focused widget")
        except TimeoutError:
            state = focus()
            raise AssertionError(f"after clicking {name}: focused {state['named']}, {state['unnamed']} unnamed") from None

    # Right to left: a press captured by an earlier pane's input used to stop
    # at that pane, leaving every input to its right focused as well.
    for layout, names in (
        ("columns", ("note-editor", "filter-notes", "filter-groups", "filter-sections")),
        ("outline", ("note-editor", "filter-notes", "filter-outline")),
    ):
        fixture_gateway(app, "view.layout", layout=layout)
        for name in names:
            click(name)
    type_text(window, "zz")
    wait_for(lambda: navigation(app)["filters"]["outline"] == "zz", timeout=3,
             description="typing reaches the focused filter")
    assert navigation(app)["filters"]["notes"] == ""
    assert session(app)["document"] == before_document
    assert files(root) == original_files
