"""Editing a board and what it saves (discovery/boards.md §3.4, §5.5 to
§5.12, §6.8 Phase 2 items 2, 4, 5, 6 and 8).

Every row of item 2 is: flush, keep the bytes, act, flush, keep the bytes,
then a line diff, because a small edit making a small diff is the product's
promise. Input goes in through `board.key`, `board.type` and
`board.pointer`, the widget's own seam; `board.apply` only arranges a board
before what a case is about.
"""

from __future__ import annotations

import time
import tomllib
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, HAND_WRITTEN, GROUP, blocks, board, board_app, bounds, canvas_point, click, copy_samples,
    drag, fitted, flushed, key, line_diff, open_note, pointer, press, stamp, type_text,
)
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.ui import WrenApp, wait_for

# Checkout flow's elements.
START, RECEIVE, PACK, VALID = "ahj860xh", "12axfsny", "3qrg4kv1", "z9hqp66f"
# The connector from Start to Receive order, and Receive order's other one.
INTO_RECEIVE, FROM_RECEIVE = "4x2tte9d", "080b71vw"
HEADER = "# wren-board fragment 1"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def ready(app: WrenApp, path: str = FLOW) -> dict:
    """Open a board, give it the keyboard and wait for its fit."""
    open_note(app, path)
    app.client.call("editor.focus")
    wait_focus(app, ["note-editor"])
    return fitted(app)


def dirty(app: WrenApp) -> bool:
    return app.client.call("session.get")["document"]["dirty"]


def node(app: WrenApp, id_: str) -> dict:
    return next(n for n in board(app, elements=True)["nodes"] if n["id"] == id_)


def reconciled(app: WrenApp, done) -> None:
    """Ask the Session to look at the disk until ``done()`` holds: the scan
    runs off the window's thread."""
    def check():
        app.client.call("core.apply", {"op": "reconcile"})
        return done()

    wait_for(check, timeout=10, interval=0.1, description="the change on disk taken")


def test_looking_around_writes_nothing(samples: Path) -> None:
    """Item 2, first row: select, walk, pan, zoom, arm and disarm a tool,
    open and close a label unchanged."""
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-edit-look") as app:
        ready(app)
        assert click(app, {"node": RECEIVE})["selection"] == [RECEIVE]
        for arrow in ("down", "down", "up", "right", "left"):
            assert key(app, arrow)["handled"]
        app.client.call("board.pointer", {"kind": "wheel", "at": {"node": RECEIVE},
                                          "delta": {"y": -1}})
        app.client.call("command.run", {"id": "font.increase"})
        app.client.call("command.run", {"id": "board.zoom.fit"})
        assert key(app, "r")["state"] == "armed"
        assert key(app, "escape")["tool"] == "select"
        assert key(app, "enter")["state"] == "editing"
        state = key(app, "escape")
        assert state["state"] == "idle" and state["editing"] is None
        assert state["pristine"] and not state["can_undo"] and state["revision"] == 0, state
        flushed(app)
        assert not dirty(app)
    assert stamp(flow) == before


def test_a_move_is_one_line_and_its_inverse_or_undo_none(samples: Path) -> None:
    """Item 2: a key move and a pointer drag each change one `at` line and
    no connector line; the inverse move and undo give the bytes back."""
    flow = samples / FLOW
    with board_app(samples, "board-edit-move") as app:
        ready(app)
        original = flow.read_bytes()
        click(app, {"node": RECEIVE})
        key(app, "right", "alt")
        flushed(app)
        assert line_diff(original, flow.read_bytes()) == ["- at = [0, 120]", "+ at = [20, 120]"]
        key(app, "left", "alt")
        flushed(app)
        assert flow.read_bytes() == original

        state = drag(app, {"node": PACK}, (40, 0))
        assert state["state"] == "idle" and state["selection"] == [PACK], state
        flushed(app)
        assert line_diff(original, flow.read_bytes()) == ["- at = [0, 840]", "+ at = [40, 840]"]
        state = key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == original
        assert state["pristine"] and not dirty(app)


def test_a_hand_written_board_comes_back_byte_for_byte_after_undo(samples: Path) -> None:
    """Item 2: move then undo on `Hand written`, waiting past the
    debounce, saves the original non-canonical file."""
    path = samples / HAND_WRITTEN
    original = path.read_bytes()
    with board_app(samples, "board-edit-hand") as app:
        ready(app, HAND_WRITTEN)
        click(app, {"node": "first"})
        key(app, "down", "alt")
        flushed(app)
        canonical = path.read_bytes()
        assert canonical != original and b"#" not in canonical
        key(app, "z", "mod")
        # Autosave, not a flush, writes it back.
        wait_for(lambda: path.read_bytes() == original and not dirty(app), timeout=10,
                 interval=0.1, description="the hand-written bytes saved back")


def test_a_label_edit_is_its_text_line(samples: Path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-edit-label") as app:
        ready(app)
        original = flow.read_bytes()
        click(app, {"node": PACK})
        assert key(app, "enter")["editing"]["text"] == "Pack"
        # The text opened selected, so typing replaces it.
        assert type_text(app, "Pack it")["editing"]["text"] == "Pack it"
        key(app, "escape")
        flushed(app)
        assert line_diff(original, flow.read_bytes()) == ['- text = "Pack"', '+ text = "Pack it"']
        # A label that grows its node writes its size too.
        click(app, {"node": PACK})
        key(app, "enter")
        type_text(app, "Pack\nthe\nwhole\norder")
        key(app, "escape")
        flushed(app)
        changed = line_diff(original, flow.read_bytes())
        assert changed[0] == '- text = "Pack"', changed
        assert changed[-2:] == ["- size = [120, 60]", "+ size = [120, 100]"], changed
        assert all(line.startswith("+ ") for line in changed[1:-2]), changed
        assert node(app, PACK)["text"] == "Pack\nthe\nwhole\norder"


def test_a_new_node_or_connector_is_one_block(samples: Path) -> None:
    """Item 2: add a node; add a connector."""
    flow = samples / FLOW
    with board_app(samples, "board-edit-add") as app:
        ready(app)
        original = flow.read_bytes()
        assert key(app, "r")["tool"] == "rectangle"
        state = click(app, {"canvas": [600, 30]})
        assert state["state"] == "editing" and len(state["selection"]) == 1, state
        made = state["selection"][0]
        type_text(app, "Gift wrap")
        key(app, "escape")
        flushed(app)
        after = flow.read_bytes()
        assert set(blocks(after)) - set(blocks(original)) == {f"[node.{made}]"}
        assert set(blocks(original)) <= set(blocks(after))
        assert all(line.startswith("+") for line in line_diff(original, after))
        assert node(app, made)["at"] == [540, 0] and node(app, made)["size"] == [120, 60]

        # A node in the slot right of Receive order, then Ctrl+Right
        # connects to it and makes no node.
        app.client.call("board.apply", {"edits": [
            {"op": "add_node", "id": "aside", "at": [200, 120], "size": [120, 60], "text": "Aside"},
        ]})
        flushed(app)
        before = flow.read_bytes()
        click(app, {"node": RECEIVE})
        state = key(app, "right", "mod")
        assert state["state"] == "idle" and state["counts"]["nodes"] == 17, state
        flushed(app)
        after = flow.read_bytes()
        new = set(blocks(after)) - set(blocks(before))
        assert len(new) == 1 and next(iter(new)).startswith("[edge."), new
        assert all(line.startswith("+") for line in line_diff(before, after))


def test_deleting_a_node_takes_its_two_connectors_and_nothing_else(samples: Path) -> None:
    flow = samples / FLOW
    with board_app(samples, "board-edit-delete") as app:
        ready(app)
        original = flow.read_bytes()
        click(app, {"node": RECEIVE})
        state = key(app, "backspace")
        assert state["selection"] == [] and state["counts"] == {"nodes": 14, "edges": 15}, state
        flushed(app)
        after = flow.read_bytes()
        assert set(blocks(original)) - set(blocks(after)) == {
            f"[node.{RECEIVE}]", f"[edge.{INTO_RECEIVE}]", f"[edge.{FROM_RECEIVE}]"}
        assert set(blocks(after)) <= set(blocks(original))
        assert all(line.startswith("-") for line in line_diff(original, after))
        # One undo brings all three back.
        key(app, "z", "mod")
        flushed(app)
        assert flow.read_bytes() == original


def test_copy_and_paste_make_new_ids_one_cell_on_and_survive_a_restart(samples: Path) -> None:
    """Items 2 and 8: a two-node, one-connector fragment, pasted twice."""
    flow = samples / FLOW
    with board_app(samples, "board-edit-paste") as app:
        ready(app)
        # Headless Weston gives the app no clipboard: what it writes there
        # reads back as nothing. There the app keeps one of its own, so the
        # copy and paste path is tested and only the system clipboard is not.
        app.client.call("clipboard.write", {"text": "probe"})
        try:
            wait_for(lambda: app.client.call("clipboard.read") == "probe", timeout=2,
                     interval=0.05, description="the clipboard")
        except TimeoutError:
            app.client.call("test.clipboard", {"own": True})
        original = flow.read_bytes()
        click(app, {"node": START})
        assert click(app, {"node": RECEIVE}, mods=("shift",))["selection"] == sorted([START, RECEIVE])
        assert key(app, "c", "mod")["handled"]
        copied = wait_for(lambda: (lambda text: text if text.startswith(HEADER) else None)(
            app.client.call("clipboard.read")), timeout=5, description="the copy on the clipboard")
        fragment = tomllib.loads(copied.split("\n", 1)[1])
        assert sorted(fragment["node"]) == sorted([START, RECEIVE])
        assert list(fragment["edge"]) == [INTO_RECEIVE]

        for paste in (1, 2):
            key(app, "v", "mod")
            state = wait_for(lambda: (lambda s: s if s["counts"]["nodes"] == 15 + 2 * paste else None)(
                board(app, elements=True)), timeout=5, description=f"paste {paste}")
            selection = state["selection"]
            assert len(selection) == 3 and not set(selection) & {START, RECEIVE, INTO_RECEIVE}
            pasted = [n for n in state["nodes"] if n["id"] in selection]
            # One cell further each time the same text is pasted.
            assert sorted(n["at"] for n in pasted) == [[20 * paste, 20 * paste],
                                                      [20 * paste, 120 + 20 * paste]], pasted
            flushed(app)
        after = flow.read_bytes()
        new = set(blocks(after)) - set(blocks(original))
        assert len(new) == 6 and sum(header.startswith("[edge.") for header in new) == 2, new
        assert all(line.startswith("+") for line in line_diff(original, after))
        tomllib.loads(after.decode("utf-8"))

        app.restart()
        ready(app)
        assert board(app)["counts"] == {"nodes": 19, "edges": 19}
    assert flow.read_bytes() == after


def test_a_change_on_disk_reloads_keeps_the_view_and_ends_what_was_in_flight(samples: Path) -> None:
    """Item 4: clean, dirty and in the middle of a drag."""
    flow = samples / FLOW
    added = '\n[node.extra]\ntext = "Extra"\nkind = "shape"\nat = [400, 0]\nsize = [120, 60]\nz = "zz"\n'
    with board_app(samples, "board-edit-external") as app:
        ready(app)
        # Clean: the selection by id and the viewport are kept, and the
        # move made before is no longer undoable.
        click(app, {"node": RECEIVE})
        key(app, "right", "alt")
        flushed(app)
        assert board(app)["can_undo"]
        viewport = board(app)["viewport"]
        flow.write_bytes(flow.read_bytes() + added.encode())
        reconciled(app, lambda: board(app)["counts"]["nodes"] == 16)
        state = board(app)
        assert state["selection"] == [RECEIVE] and state["viewport"] == viewport, state
        assert not state["can_undo"] and state["pristine"]
        assert not dirty(app)

        # Dirty: the draft goes to a conflict copy named for a board.
        app.client.call("test.timer.hold", {"held": True})
        key(app, "right", "alt")
        assert dirty(app)
        mine = app.client.call("session.get")["document"]["text"]
        theirs = flow.read_text(encoding="utf-8").replace('"Extra"', '"Theirs"')
        flow.write_text(theirs, encoding="utf-8")
        reconciled(app, lambda: not dirty(app))
        app.client.call("test.timer.hold", {"held": False})
        copies = sorted((samples / GROUP).glob("Checkout flow (conflict*).wren-board"))
        assert len(copies) == 1, list((samples / GROUP).iterdir())
        assert copies[0].read_text(encoding="utf-8") == mine
        assert flow.read_text(encoding="utf-8") == theirs
        assert next(n for n in board(app, elements=True)["nodes"] if n["id"] == "extra")["text"] == "Theirs"

        # In the middle of a drag: the gesture ends, nothing it did is
        # kept, and the release afterwards does nothing.
        press(app, {"node": PACK})
        x, y = canvas_point(app, {"node": PACK})
        assert pointer(app, "move", {"canvas": [x + 60, y]})["state"] == "moving"
        rewritten = theirs.replace('"Theirs"', '"Again"')
        flow.write_text(rewritten, encoding="utf-8")
        reconciled(app, lambda: board(app)["state"] == "idle")
        state = pointer(app, "release")
        assert state["state"] == "idle" and node(app, PACK)["at"] == [0, 840], state
        flushed(app)
        assert flow.read_text(encoding="utf-8") == rewritten
        assert not dirty(app)


def test_each_gesture_is_one_step_and_as_many_undos_give_the_bytes_back(samples: Path) -> None:
    """Item 5."""
    flow = samples / FLOW
    with board_app(samples, "board-edit-undo") as app:
        ready(app)
        original = flow.read_bytes()
        gestures = [
            lambda: (click(app, {"node": RECEIVE}), key(app, "right", "alt")),
            lambda: drag(app, {"node": PACK}, (60, 20)),
            lambda: (click(app, {"node": VALID}), key(app, "enter"), type_text(app, "Valid order?"),
                     key(app, "escape")),
            lambda: (click(app, {"node": START}), key(app, "d", "mod")),
            lambda: (click(app, {"node": RECEIVE}), key(app, "delete")),
            lambda: (key(app, "o"), click(app, {"canvas": [600, 300]}), type_text(app, "Round"),
                     key(app, "escape")),
        ]
        saved = [original]
        for gesture in gestures:
            gesture()
            flushed(app)
            saved.append(flow.read_bytes())
        assert len(set(saved)) == len(saved), "every gesture changed the file"
        for back in reversed(saved[:-1]):
            key(app, "z", "mod")
            flushed(app)
            assert flow.read_bytes() == back
        state = board(app)
        assert state["pristine"] and not state["can_undo"] and state["can_redo"]
        # Redo all of it, and the last save comes back.
        for _ in gestures:
            key(app, "z", "mod", "shift")
        flushed(app)
        assert flow.read_bytes() == saved[-1]


def test_the_mouse_creates_selects_moves_resizes_edits_and_deletes(samples: Path) -> None:
    """Item 6, through `board.pointer`; the X11 lane repeats the gestures
    with the real pointer in `test_board_native.py`."""
    flow = samples / FLOW
    with board_app(samples, "board-edit-mouse") as app:
        ready(app)
        # An armed tool and a click: a default node centred on the point,
        # snapped, and its label open.
        key(app, "d")
        state = click(app, {"canvas": [612, 37]})
        made = state["selection"][0]
        assert state["state"] == "editing"
        assert node(app, made)["at"] == [560, 0] and node(app, made)["shape"] == "diamond"
        key(app, "escape")
        # An armed tool and a drag: from corner to corner, snapped.
        key(app, "r")
        press(app, {"canvas": [600, 200]})
        pointer(app, "move", {"canvas": [650, 230]})
        state = pointer(app, "move", {"canvas": [702, 279]})
        assert state["state"] == "creating"
        state = pointer(app, "release")
        drawn = state["selection"][0]
        assert node(app, drawn)["at"] == [600, 200] and node(app, drawn)["size"] == [100, 80]
        key(app, "escape")
        # A double-click on empty canvas: a rectangle there.
        state = click(app, {"canvas": [800, 30]}, double=True)
        assert state["state"] == "editing" and node(app, state["selection"][0])["at"] == [740, 0]
        key(app, "escape")
        key(app, "escape")

        # A marquee around Start and Receive order selects them and the
        # connector between them.
        state = drag(app, {"canvas": [-30, -30]}, (180, 240))
        assert state["selection"] == sorted([START, RECEIVE, INTO_RECEIVE]), state
        # A move with snap: the pressed node's corner lands on the grid.
        state = drag(app, {"node": START}, (33, 9))
        assert node(app, START)["at"] == [40, 0] and node(app, RECEIVE)["at"] == [40, 120]
        # A resize by the south-east handle of the one selected node.
        click(app, {"node": PACK})
        assert [h["handle"] for h in bounds(app)["handles"]] == ["nw", "n", "ne", "e", "se", "s", "sw", "w"]
        drag(app, {"node": PACK, "handle": "se"}, (41, 18))
        assert node(app, PACK)["size"] == [160, 80] and node(app, PACK)["at"] == [0, 840]
        # A double-click on a node edits its label.
        state = click(app, {"node": PACK}, double=True)
        assert state["editing"]["id"] == PACK
        type_text(app, "Box")
        key(app, "escape")
        assert node(app, PACK)["text"] == "Box"
        # Delete.
        click(app, {"node": PACK})
        assert key(app, "delete")["counts"]["nodes"] == 17
        flushed(app)
        tomllib.loads(flow.read_text(encoding="utf-8"))


def test_reloading_saved_bytes_and_a_no_op_writes_nothing(samples: Path) -> None:
    """Item 2, last row, and a reload of reformatted but equal content."""
    flow = samples / FLOW
    with board_app(samples, "board-edit-reload") as app:
        ready(app)
        click(app, {"node": RECEIVE})
        key(app, "right", "alt")
        flushed(app)
        saved = flow.read_text(encoding="utf-8")
        # The same board, reformatted by another program.
        reformatted = "# reformatted elsewhere\n" + saved.replace(" = ", "=")
        flow.write_text(reformatted, encoding="utf-8")
        reconciled(app, lambda: board(app)["revision"] >= 2 and not dirty(app))
        before = stamp(flow)
        click(app, {"node": PACK})
        key(app, "up")
        key(app, "enter")
        key(app, "escape")
        key(app, "right", "alt")
        key(app, "left", "alt")
        time.sleep(1.0)
        flushed(app)
        assert stamp(flow) == before and not dirty(app)


def test_a_label_being_typed_is_found_by_search(samples: Path) -> None:
    """§3.4 rule 7: the index takes the open board's labels once its draft
    rests, and an open search refreshes by itself."""
    with board_app(samples, "board-edit-search") as app:
        ready(app)
        click(app, {"node": PACK})
        key(app, "enter")
        type_text(app, "Quokka")
        app.client.call("palette.open", {"mode": "search"})

        def found():
            modal = app.client.call("palette.query", {"query": "quokka"})["navigation"]["modal"]
            return [row for row in modal["results"] if row["path"] == FLOW]

        rows = wait_for(found, timeout=5, interval=0.1, description="the typed label found")
        assert "Quokka" in rows[0]["snippet"], rows
        app.client.call("palette.close")
