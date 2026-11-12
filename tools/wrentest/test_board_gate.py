"""What the `boards` setting gates, and what a board must never write
(discovery/boards.md §3.3 to §3.5, §6.8 Phase 1 items 1, 5, 6 and 9).

With the setting off a workspace with boards behaves as if they were not
there. With it on, a board is never written by opening or looking at it,
the hidden editor behind it never writes it, and the state file keeps the
keys an older Wren reads free of board paths.
"""

from __future__ import annotations

import os
import time
import tomllib
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, HAND_WRITTEN, NOTES, board, board_app, fitted, note_id, open_note, stamp, wheel,
)
from tools.wrentest.test_board_view import samples  # noqa: F401  (fixture)
from tools.wrentest.client import RemoteError
from tools.wrentest.ui import WrenApp, wait_for

# Past the 750 ms autosave debounce.
SETTLE = 1.0

BOARD = """schema = 1

[node.a]
text = "Gate"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "a"
"""


@pytest.fixture
def small(tmp_path: Path) -> Path:
    """`Flow.wren-board` beside `Note.md`."""
    group = tmp_path / "work" / "Plans" / "Q1"
    group.mkdir(parents=True)
    (group / "Flow.wren-board").write_text(BOARD, encoding="utf-8")
    (group / "Note.md").write_text("# Note\n\nwords\n", encoding="utf-8")
    return tmp_path / "work"


def palette_titles(app: WrenApp, query: str) -> list[str]:
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": query})["navigation"]["modal"]
    app.client.call("palette.close")
    return [row["title"] for row in modal["results"]]


def test_with_the_setting_off_a_board_is_not_there(small: Path) -> None:
    """§6.8 Phase 1 item 1."""
    flow = small / "Plans/Q1/Flow.wren-board"
    before = stamp(flow)
    with board_app(small, "board-gate-off", setting="off") as app:
        session = app.client.call("session.get")
        assert [row["path"] for row in session["notes"]] == ["Plans/Q1/Note.md"]
        assert session["banner"] is None
        assert board(app)["enabled"] is False
        titles = palette_titles(app, "board")
        assert not any("board" in title.lower() for title in titles), titles
        app.client.call("palette.open", {"mode": "search"})
        modal = app.client.call("palette.query", {"query": "Gate"})["navigation"]["modal"]
        app.client.call("palette.close")
        assert modal["results"] == []
        assert app.client.call("command.run", {"id": "note.export_svg"})["disposition"] == "blocked"
        open_note(app, "Plans/Q1/Note.md")
        assert board(app)["surface"] == "text"
    assert stamp(flow) == before
    state = (small / ".wren/workspace.toml").read_text(encoding="utf-8")
    assert "wren-board" not in state and "[navigation]" not in state


def test_the_setting_on_lists_the_board(small: Path) -> None:
    with board_app(small, "board-gate-on") as app:
        notes = {row["path"]: row["title"] for row in app.client.call("session.get")["notes"]}
        assert notes == {"Plans/Q1/Flow.wren-board": "Flow", "Plans/Q1/Note.md": "Note"}


@pytest.mark.parametrize("path", [FLOW, HAND_WRITTEN])
def test_looking_at_a_board_never_writes_it(samples: Path, tmp_path: Path, path: str) -> None:
    """§6.8 Phase 1 item 5: open, pan, zoom, theme switch, export, note
    switch, window blur and quit."""
    file = samples / path
    before = stamp(file)
    with board_app(samples, "board-never-writes") as app:
        open_note(app, path)
        placed = fitted(app)
        root = placed["root"]
        middle = [root["x"] + root["width"] / 2, root["y"] + root["height"] / 2]
        wheel(app, {"screen": middle}, -2)
        time.sleep(0.2)
        wheel(app, {"screen": middle}, 1, mods=("mod",))
        app.client.call("board.pointer", {"kind": "press", "button": "middle", "at": {"screen": middle}})
        app.client.call("board.pointer", {"kind": "move", "at": {"screen": [middle[0] + 25, middle[1]]}})
        app.client.call("board.pointer", {"kind": "release", "button": "middle"})
        app.client.call("command.run", {"id": "font.increase"})
        app.client.call("theme.set", {"id": "light"})
        app.client.call("theme.set", {"id": "dark"})
        app.client.call("board.export", {"format": "svg", "path": str(tmp_path / "out.svg")})
        assert app.client.call("session.get")["document"]["dirty"] is False
        open_note(app, NOTES)
        open_note(app, path)
        app.client.call("window.blur")
        time.sleep(SETTLE)
        assert app.client.call("session.get")["document"]["dirty"] is False
        assert stamp(file) == before
    assert stamp(file) == before


def test_the_hidden_editor_never_writes_a_board(samples: Path) -> None:
    """§6.8 Phase 1 item 6."""
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-single-writer") as app:
        open_note(app, FLOW)
        text = app.client.call("session.get")["document"]
        for op, params in [
            ("editor.insert", {"text": "bypass"}),
            ("editor.command", {"intent": "bold"}),
        ]:
            with pytest.raises(RemoteError, match="board"):
                app.client.call(op, params)
        assert app.client.call("command.run", {"id": "markdown.bold"})["disposition"] == "blocked"
        engine = app.client.call("editor.get")["engine"]
        assert app.client.call("command.run", {"id": "editor.toggle_engine"})["disposition"] == "completed"
        assert app.client.call("editor.get")["engine"] != engine
        assert app.client.call("editor.get")["text"] == ""
        time.sleep(SETTLE)
        after = app.client.call("session.get")["document"]
        assert after["dirty"] is False and after == text
        assert board(app)["surface"] == "board"
        assert stamp(flow) == before
    assert stamp(flow) == before


def state_file(workspace: Path) -> dict:
    return tomllib.loads((workspace / ".wren/workspace.toml").read_text(encoding="utf-8"))


def assert_v1_keys_are_markdown(state: dict) -> None:
    """The keys v0.1.1 validates name Markdown notes only."""
    for key in ("history", "recent"):
        for entry in state.get(key, []):
            assert entry.endswith(".md"), (key, state)
    if "last_open_note" in state:
        assert state["last_open_note"].endswith(".md"), state


def test_board_paths_stay_out_of_the_keys_an_older_wren_reads(samples: Path) -> None:
    """§6.8 Phase 1 item 9; the frozen v0.1.1 validator runs over the same
    sequences in `wren-core`'s `released_state` tests."""
    with board_app(samples, "board-state-file") as app:
        open_note(app, FLOW)
        open_note(app, NOTES)
        app.client.call("core.apply", {"op": "back"})
        assert board(app)["surface"] == "board"
        app.client.call("core.apply", {"op": "forward"})
        assert board(app)["surface"] == "text"
        app.client.call("core.apply", {"op": "back"})
        assert board(app)["surface"] == "board"
        app.client.call("core.apply", {"op": "flush"})
        state = state_file(samples)
        assert_v1_keys_are_markdown(state)
        assert FLOW in state["navigation"]["history"], state
        app.restart(hard=False)
        reopened = board(app)
        assert reopened["surface"] == "board", reopened
        assert reopened["note"] == note_id(app, FLOW)
    state = state_file(samples)
    assert_v1_keys_are_markdown(state)
    assert state["selection"] == FLOW


RELEASED = os.environ.get("WRENTEST_RELEASED_BIN")


@pytest.mark.skipif(not RELEASED, reason="WRENTEST_RELEASED_BIN names no v0.1.1 smoke-test build")
def test_a_released_wren_and_this_one_share_a_workspace(samples: Path, monkeypatch) -> None:
    """§6.8 Phase 1 item 9, live: v0.1.1 and this build open the same
    workspace in turn and each keeps the other's order, sort and widths."""
    assert RELEASED and Path(RELEASED).is_absolute(), RELEASED
    current = os.environ.get("WREN_TEST_BIN")

    def launch(binary: str | None, case: str) -> WrenApp:
        if binary:
            monkeypatch.setenv("WREN_TEST_BIN", binary)
        else:
            monkeypatch.delenv("WREN_TEST_BIN", raising=False)
        return board_app(samples, case)

    def kept(state: dict) -> dict:
        return {key: state.get(key) for key in ("order", "sort", "widths")}

    with launch(current, "board-mixed-new") as app:
        open_note(app, FLOW)
        open_note(app, NOTES)
        open_note(app, FLOW)
        app.client.call("view.sort", {"mode": "title"})
        app.client.call("view.width", {"pane": "notes", "width": 300})
    ours = kept(state_file(samples))
    assert ours["widths"], ours
    with launch(RELEASED, "board-mixed-released") as app:
        session = app.client.call("session.get")
        # It takes the state file without a warning, and lists the Markdown note only.
        assert session["banner"] is None, session["banner"]
        assert [row["path"] for row in session["notes"]] == [NOTES]
        app.client.call("view.width", {"pane": "groups", "width": 260})
    theirs = kept(state_file(samples))
    assert theirs["sort"] == ours["sort"] and theirs["order"] == ours["order"], (ours, theirs)
    assert theirs["widths"]["columns"]["notes"] == 300, (ours, theirs)
    assert theirs["widths"]["columns"]["groups"] == 260, theirs
    assert_v1_keys_are_markdown(state_file(samples))
    with launch(current, "board-mixed-again") as app:
        wait_for(lambda: app.client.call("session.get")["notes"], timeout=10)
        assert kept(state_file(samples)) == theirs
        # The released build kept the board's navigation for this one.
        assert board(app)["surface"] == "board"
    assert kept(state_file(samples)) == theirs
