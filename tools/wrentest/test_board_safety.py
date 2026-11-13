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
    FLOW, HAND_WRITTEN, NOTES, board, board_app, click, copy_samples, fitted, flushed, key,
    note_id, open_note, stamp, type_text,
)
from tools.wrentest.client import RemoteError
from tools.wrentest.test_board_view import wait_focus
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
