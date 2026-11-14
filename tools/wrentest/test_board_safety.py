"""What a board never loses and never writes by mistake (discovery/boards.md
§3.4, §5.9 rule 6, §6.8 Phase 2 item 3, and §7.3's attacks 1, 6 and 7).

A label is part of the draft from its first keystroke, the board settles
before anything that is not passive, only the board writes a board, and a
sequence that leaves nothing changed writes nothing.
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, GROUP, HAND_WRITTEN, NOTES, blocks, board, board_app, click, copy_samples, fitted, flushed,
    key, note_id, open_note, stamp, type_text,
)
from tools.wrentest.client import RemoteError
from tools.wrentest.test_board_view import focused, wait_focus
from tools.wrentest.ui import WrenApp, wait_for

PACK = "3qrg4kv1"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def ready(app: WrenApp, path: str = FLOW) -> dict:
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    return fitted(app)


def label_typed(app: WrenApp, text: str) -> None:
    """Open Pack's label and type into it, leaving it open."""
    click(app, {"node": PACK})
    key(app, "enter")
    assert type_text(app, text)["editing"]["text"] == text


@pytest.mark.parametrize("leave", ["note", "blur", "quit"])
def test_a_label_being_typed_is_on_disk_before_the_board_is_left(samples: Path, leave: str) -> None:
    """Item 3: type into a label, then at once open another note, blur the
    window or quit."""
    flow = samples / FLOW
    with board_app(samples, f"board-safety-{leave}") as app:
        ready(app)
        # Autosave must not be what saves it.
        app.client.call("test.timer.hold", {"held": True})
        label_typed(app, "Packed fast")
        if leave == "note":
            open_note(app, NOTES)
            assert board(app)["surface"] == "text"
        elif leave == "blur":
            app.client.call("window.blur")
            state = board(app)
            assert state["editing"] is None and state["state"] in ("idle", "unfocused"), state
        else:
            app.quit()
        if leave != "quit":
            wait_for(lambda: not app.client.call("session.get")["pending_durability"], timeout=10,
                     interval=0.02, description="the flush")
            assert not app.client.call("session.get")["document"]["dirty"]
    assert b'text = "Packed fast"' in flow.read_bytes()


def test_a_failed_write_blocks_the_switch_and_keeps_the_label(samples: Path) -> None:
    """Item 3: with a write fault the note switch is blocked, the label
    committed and the draft kept; the next flush saves it."""
    flow = samples / FLOW
    with board_app(samples, "board-safety-fault") as app:
        ready(app)
        app.client.call("test.timer.hold", {"held": True})
        label_typed(app, "Kept")
        original = flow.read_bytes()
        app.client.call("test.fail_next", {"fault": "write"})
        with pytest.raises(RemoteError, match="injected storage failure"):
            app.client.call("core.apply", {"op": "open_note", "note": note_id(app, NOTES)})
        state = board(app)
        assert state["surface"] == "board" and state["editing"] is None, state
        document = app.client.call("session.get")["document"]
        assert document["dirty"] and 'text = "Kept"' in document["text"]
        assert flow.read_bytes() == original
        app.client.call("test.timer.hold", {"held": False})
        flushed(app)
        assert b'text = "Kept"' in flow.read_bytes()


def test_only_the_board_writes_a_board(samples: Path) -> None:
    """Attack 7: the editor's requests and commands, with a board open and
    edited, and a board command on a Markdown note."""
    flow = samples / FLOW
    with board_app(samples, "board-safety-writer") as app:
        ready(app)
        click(app, {"node": PACK})
        key(app, "right", "alt")
        flushed(app)
        edited = flow.read_bytes()
        for op, params in [
            ("editor.insert", {"text": "bypass"}),
            ("editor.command", {"intent": "bold"}),
        ]:
            with pytest.raises(RemoteError):
                app.client.call(op, params)
        app.client.call("command.run", {"id": "markdown.bold"})
        app.client.call("command.run", {"id": "editor.toggle_engine"})
        app.client.call("command.run", {"id": "editor.toggle_engine"})
        time.sleep(1.0)
        flushed(app)
        assert flow.read_bytes() == edited
        assert board(app)["surface"] == "board"

        # A board command with a Markdown note open changes nothing.
        notes = samples / NOTES
        before = stamp(notes)
        open_note(app, NOTES)
        for command in ("board.delete", "board.select.all", "board.paste", "board.undo"):
            app.client.call("command.run", {"id": command})
        time.sleep(1.0)
        flushed(app)
        assert stamp(notes) == before
        assert flow.read_bytes() == edited


def test_a_sequence_that_changes_nothing_writes_nothing(samples: Path) -> None:
    """Attack 6, on the hand-written board, whose bytes are not canonical:
    any write would show."""
    path = samples / HAND_WRITTEN
    before = stamp(path)
    with board_app(samples, "board-safety-pristine") as app:
        ready(app, HAND_WRITTEN)
        click(app, {"node": "first"})
        for arrow in ("right", "left", "down"):
            key(app, arrow)
        key(app, "enter")
        key(app, "escape")
        key(app, "right", "alt")
        key(app, "left", "alt")
        key(app, "down", "alt")
        key(app, "z", "mod")
        key(app, "z", "mod", "shift")
        state = key(app, "z", "mod")
        assert state["pristine"], state
        # A label changed and changed back.
        click(app, {"node": "second"})
        key(app, "enter")
        type_text(app, "Other")
        key(app, "z", "mod")
        state = key(app, "escape")
        assert state["pristine"], state
        # Past the autosave debounce.
        time.sleep(1.2)
        flushed(app)
        assert not app.client.call("session.get")["document"]["dirty"]
    assert stamp(path) == before


# A flagged z key in the root scope, so a new element on top renumbers the
# scope (§4.7), and a label stored with trailing blank lines.
TAKEN_BACK = (
    'schema = 1\n\n# Written by hand.\n[node.a]\nkind = "shape"\nat = [0, 0]\n'
    'size = [120, 60]\nz = "a-b"\n\n[node.b]\ntext = "B\\n\\n"\nkind = "shape"\n'
    'at = [200, 0]\nsize = [120, 60]\nz = "V"\n'
)


def test_an_edit_that_comes_to_nothing_writes_nothing(samples: Path) -> None:
    """§5.9 rules 3, 4 and 8: free text made and left empty, and a label
    put back by its own undo, leave the file's bytes and time as they were."""
    path = samples / GROUP / "Taken back.wren-board"
    path.write_text(TAKEN_BACK, encoding="utf-8")
    before = stamp(path)
    with board_app(samples, "board-safety-taken-back") as app:
        ready(app, f"{GROUP}/Taken back.wren-board")
        key(app, "t")
        click(app, {"canvas": [160, 200]})
        assert board(app)["counts"]["nodes"] == 3
        state = key(app, "escape")
        assert state["counts"]["nodes"] == 2, state
        assert state["pristine"] and not state["can_undo"], state
        click(app, {"node": "b"})
        key(app, "enter")
        type_text(app, "C")
        assert not board(app)["pristine"]
        key(app, "z", "mod")
        state = key(app, "escape")
        assert state["pristine"] and not state["can_undo"], state
        # Past the autosave debounce.
        time.sleep(1.2)
        flushed(app)
        assert not app.client.call("session.get")["document"]["dirty"]
    assert stamp(path) == before


@pytest.mark.parametrize("notice", ["blocked paste", "label full"])
def test_a_notice_leaves_the_keyboard_where_it_was(samples: Path, notice: str) -> None:
    """§5.15 rule 4: a banner is not among what may take the keyboard, so
    the board keeps it, and an open label stays open, after a notice."""
    with board_app(samples, f"board-safety-notice-{notice.replace(' ', '-')}") as app:
        ready(app)
        app.client.call("test.clipboard", {"own": True})
        if notice == "blocked paste":
            click(app, {"node": PACK})
            app.client.call("clipboard.write", {"text": "# wren-board fragment 1\nschema = 2\n"})
        else:
            label_typed(app, "abc")
            app.client.call("clipboard.write", {"text": "y" * 20_000})
        key(app, "v", "mod")
        wait_for(lambda: app.client.call("session.get")["banner"], timeout=5, interval=0.02,
                 description="the notice")
        # Let the frame that shows the banner be drawn.
        time.sleep(0.3)
        assert focused(app) == ["note-editor"]
        if notice == "blocked paste":
            before = board(app, elements=True)
            state = key(app, "right", "alt")
            assert state["handled"], state
            after = board(app, elements=True)
            assert before["nodes"] != after["nodes"]
        else:
            state = type_text(app, "def")
            assert state["editing"]["text"] == "abcdef", state


# A node and a connector named alike (§4.12), the node with a child.
SHARED = (
    'schema = 1\n\n[node.a]\nkind = "shape"\nat = [0, 0]\nsize = [120, 60]\nz = "V"\n\n'
    '[node.kid]\nkind = "shape"\nat = [0, 0]\nparent = "shared"\nsize = [40, 20]\nz = "V"\n\n'
    '[node.shared]\ntext = "Node shared"\nkind = "shape"\nat = [0, 300]\nsize = [120, 60]\n'
    'z = "W"\n\n[edge.shared]\nkind = "edge"\nfrom = { node = "a" }\nto = { at = [400, 30] }\n'
    'labels = [{ text = "Edge shared" }]\nz = "X"\n'
)


def test_a_connector_named_as_a_node_is_acted_on_alone(samples: Path) -> None:
    """§5.10 rule 1: delete removes what is selected, so a connector that
    shares a node's id goes alone, and Enter and a key move are its own."""
    path = samples / GROUP / "Shared.wren-board"
    path.write_text(SHARED, encoding="utf-8")
    with board_app(samples, "board-safety-shared-id") as app:
        ready(app, f"{GROUP}/Shared.wren-board")
        click(app, {"edge": "shared"})
        assert key(app, "enter")["editing"]["text"] == "Edge shared"
        key(app, "escape")
        key(app, "right", "alt")
        node = next(n for n in board(app, elements=True)["nodes"] if n["id"] == "shared")
        assert node["at"] == [0, 300]
        state = key(app, "delete")
        assert state["counts"] == {"nodes": 3, "edges": 0}, state
        flushed(app)
    assert blocks(path.read_bytes()) == ["[node.a]", "[node.kid]", "[node.shared]"]
