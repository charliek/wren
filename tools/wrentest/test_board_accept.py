"""The keyboard-only flowchart (discovery/boards.md §5.20, §6.8 Phase 2
item 1): the acceptance script, through `board.key` and `board.type`, on a
new empty board, in every lane.

Ids are random, so the checks are on structure: the layout is the
keyboard model's, which `crates/wren-board/golden/accept/checkout-flow.toml`
holds with the script itself, and the file is compared with itself across
undo, redo and a reopen, never with a built file.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest

from tools import board_format
from tools.wrentest.boards import (
    ACCEPTANCE, GROUP, NOTES, ROOT, board, board_app, copy_samples, flushed, key, open_note,
    script_steps, stamp, type_text,
)
from tools.wrentest.test_board_view import wait_focus
from tools.wrentest.ui import WrenApp, wait_for

GOLDEN = ROOT / "crates" / "wren-board" / "golden" / "accept" / "checkout-flow.toml"
EMPTY = b"schema = 1\n"


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    return copy_samples(tmp_path / "work")


def golden() -> dict:
    return tomllib.loads(GOLDEN.read_text(encoding="utf-8"))


def golden_steps(layout: dict) -> list[tuple[str, ...]]:
    """The golden file's script as `script_steps` writes steps."""
    steps: list[tuple[str, ...]] = []
    for step in layout["script"]:
        if step.startswith("type:"):
            steps.append(("type", step.removeprefix("type:")))
        else:
            *mods, name = step.split("+")
            steps.append(("key", name, *mods))
    return steps


def new_board(app: WrenApp, name: str) -> None:
    """A new empty board from `note.new_board`, open and focused."""
    open_note(app, NOTES)
    app.client.call("command.run", {"id": "note.new_board"})
    app.client.call("dialog.name", {"name": name})
    app.client.call("dialog.submit")
    wait_for(lambda: board(app)["surface"] == "board" and board(app)["counts"]["nodes"] == 0,
             timeout=10, interval=0.05, description="the new board open")
    wait_focus(app, ["note-editor"])


def saved(app: WrenApp) -> None:
    """The autosave has run on its own: the document is clean."""
    def clean():
        session = app.client.call("session.get")
        return not session["pending_durability"] and not session["document"]["dirty"]

    wait_for(clean, timeout=10, interval=0.05, description="the autosave")


def test_the_scripts_notation_is_the_golden_script() -> None:
    assert script_steps(ACCEPTANCE) == golden_steps(golden())


def test_the_keyboard_builds_the_flowchart(samples: Path) -> None:
    layout = golden()
    path = samples / GROUP / "Built.wren-board"
    with board_app(samples, "board-accept") as app:
        new_board(app, "Built")
        assert path.read_bytes() == EMPTY

        # Every key reaches the board; text typed in a label and in the
        # connect query is counted apart.
        count = {"label": 0, "query": 0, "chords": 0}
        state = board(app)
        for step in script_steps(ACCEPTANCE):
            if step[0] == "key":
                count["chords"] += 1
                state = key(app, step[1], *step[2:])
                assert state["handled"], (step, state)
            else:
                assert state["state"] in ("editing", "connecting"), (step, state)
                count["label" if state["state"] == "editing" else "query"] += len(step[1])
                state = type_text(app, step[1])
        # 232 key presses: 153 label characters, 9 query letters, 70 chords.
        assert count == {"label": 153, "query": 9, "chords": 70}

        built = board(app, elements=True)
        assert built["counts"] == {"nodes": 15, "edges": 17}
        nodes = {node["id"]: node for node in built["nodes"]}
        edges = {edge["id"]: edge for edge in built["edges"]}
        names = {id_: node["text"] for id_, node in nodes.items()}

        # Each node 120 by 60, on the grid, where the model put it
        # relative to Start.
        start = next(node["at"] for node in nodes.values() if node["text"] == "Start")
        placed = {}
        for node in nodes.values():
            assert node["size"] == [120, 60], node
            assert node["kind"] == "shape" and node["parent"] is None, node
            assert all(value % 20 == 0 for value in node["at"] + node["size"]), node
            placed[node["text"]] = (node["shape"], [(node["at"][0] - start[0]) // 20,
                                                    (node["at"][1] - start[1]) // 20])
        assert placed == {node["label"]: (node["shape"], node["at"]) for node in layout["node"]}
        shapes = [node["shape"] for node in nodes.values()]
        assert (shapes.count("pill"), shapes.count("diamond"), shapes.count("rect")) == (2, 3, 10)

        # No two node bounds intersect.
        boxes = [(node["text"], node["at"], node["size"]) for node in nodes.values()]
        for a, (ax, ay), (aw, ah) in boxes:
            for b, (bx, by), (bw, bh) in boxes:
                assert a == b or ax + aw <= bx or bx + bw <= ax or ay + ah <= by or by + bh <= ay, (a, b)

        # Every end attached to a node that exists, with no side; the six
        # branches labelled Yes or No.
        links = set()
        for edge in edges.values():
            for end in (edge["from"], edge["to"]):
                assert end.get("node") in nodes and end["side"] is None, edge
            label = edge["labels"][0]["text"] if edge["labels"] else ""
            links.add((names[edge["from"]["node"]], names[edge["to"]["node"]], label))
        assert links == {(e["from"], e["to"], e.get("label", "")) for e in layout["edge"]}
        branches = [label for _, _, label in links if label]
        assert len(branches) == 6 and set(branches) == {"Yes", "No"}

        # Both loops are cycles.
        onward: dict[str, set[str]] = {}
        for source, target, _ in links:
            onward.setdefault(source, set()).add(target)

        def reaches(source: str, target: str) -> bool:
            seen, stack = set(), [source]
            while stack:
                for nxt in onward.get(stack.pop(), ()):
                    if nxt == target:
                        return True
                    if nxt not in seen:
                        seen.add(nxt)
                        stack.append(nxt)
            return False

        for a, b in (("Check stock", "Notify customer"), ("Charge card", "Retry payment")):
            assert reaches(a, b) and reaches(b, a), (a, b)

        # Idle, with the last connector, Reject order to End, selected.
        assert built["state"] == "idle"
        (last,) = built["selection"]
        assert (names[edges[last]["from"]["node"]], names[edges[last]["to"]["node"]]) == (
            "Reject order", "End")

        # After the autosave delay the file parses, is canonical, and holds
        # the board the app shows.
        saved(app)
        written = path.read_bytes()
        text = written.decode("utf-8")
        assert board_format.dump(board_format.load(text)) == text
        model = tomllib.loads(text)
        assert set(model["node"]) == set(nodes) and set(model["edge"]) == set(edges)
        for id_, node in model["node"].items():
            shown = nodes[id_]
            assert (node.get("text", ""), node["at"], node["size"], node.get("shape", "rect")) == (
                shown["text"], shown["at"], shown["size"], shown["shape"]), id_
        for id_, edge in model["edge"].items():
            shown = edges[id_]
            assert (edge["from"]["node"], edge["to"]["node"]) == (
                shown["from"]["node"], shown["to"]["node"]), id_
            assert [label["text"] for label in edge.get("labels", [])] == [
                label["text"] for label in shown["labels"]], id_

        # Undoing every unit gives back the empty board, and redoing them
        # the same bytes: fifteen creations, each with its label, six
        # connector labels and three connections made by query.
        undos = 0
        while board(app)["can_undo"]:
            key(app, "z", "mod")
            undos += 1
        assert undos == 24
        saved(app)
        assert path.read_bytes() == EMPTY
        assert board(app)["pristine"]
        for _ in range(undos):
            key(app, "z", "mod", "shift")
        assert not board(app)["can_redo"]
        saved(app)
        assert path.read_bytes() == written

        # Reopening the board leaves the file as it is.
        flushed(app)
        before = stamp(path)
        open_note(app, NOTES)
        wait_for(lambda: board(app)["surface"] != "board", timeout=10, interval=0.05,
                 description="the note open")
        open_note(app, f"{GROUP}/Built.wren-board")
        # Back on a board opened in this run, its viewport is restored, not
        # fitted.
        wait_for(lambda: board(app)["surface"] == "board", timeout=10, interval=0.05,
                 description="the board open again")
        assert board(app)["counts"] == {"nodes": 15, "edges": 17}
        flushed(app)
        assert stamp(path) == before
        assert app.client.call("session.get")["document"]["dirty"] is False
