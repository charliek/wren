"""The board viewer in the app, with the `boards` setting on
(discovery/boards.md §3.3, §5.13, §5.15, §6.8 Phase 1 items 2 to 4, 7, 8
and 10).

A board is shown by the board widget, mounted under the editor's own focus
id `note-editor`, so the editor column, its focus and the column moves work
on a board as they do on a Markdown note.
"""

from __future__ import annotations

import math
import time
import xml.etree.ElementTree as ElementTree
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    CONFLICT, FLOW, FUTURE, NOTES, SHAPES, board, board_app, bounds, conflict_board, copy_samples,
    distance_to_polyline, fitted, inside, midpoint, near, open_note, screenshot, stamp, wheel,
)
from tools.wrentest.client import RemoteError
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

# A pan or zoom step draws no label on the software renderer until the
# view has been still for 150 ms.
REST = 0.4


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    """A copy of the sample workspace: opening one writes into it."""
    return copy_samples(tmp_path / "work")


def column(app: WrenApp) -> str:
    return app.client.call("navigation.get")["column"]


def nav(app: WrenApp, key: str) -> dict:
    return app.client.call("key.nav", {"key": key})


def command(app: WrenApp, command_id: str) -> dict:
    return app.client.call("command.run", {"id": command_id})


def focused(app: WrenApp) -> list[str]:
    return app.client.call("focus.get")["named"]


def wait_focus(app: WrenApp, names: list[str]) -> None:
    wait_for(lambda: focused(app) == names, timeout=3, interval=0.025,
             description=f"focused widgets {names}")


def assert_board_shown(app: WrenApp) -> None:
    """The editor is not the surface: it refuses an edit naming the board."""
    with pytest.raises(RemoteError, match="board"):
        app.client.call("editor.insert", {"text": "bypass"})


def test_a_board_is_the_editor_column_under_note_editor(samples: Path) -> None:
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-focus") as app:
        note = open_note(app, FLOW)
        assert_board_shown(app)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert column(app) == "editor"
        assert board(app)["state"] == "idle"
        canvas = app.client.call("focus.get")["bounds"]["note-editor"]
        assert canvas["width"] > 200 and canvas["height"] > 200, canvas

        # The column moves leave the board and give it the keyboard back.
        assert command(app, "navigation.column_left")["navigation"]["column"] == "notes"
        wait_focus(app, [])
        assert board(app)["state"] == "unfocused"
        assert command(app, "navigation.column_right")["navigation"]["column"] == "editor"
        wait_focus(app, ["note-editor"])

        # An overlay takes the keyboard and Escape returns it to the board.
        app.client.call("palette.open", {"mode": "commands"})
        wait_focus(app, ["palette-query"])
        state = nav(app, "escape")
        assert state["modal"] is None and state["column"] == "editor"
        wait_focus(app, ["note-editor"])
        # With nothing open, Escape keeps the board focused.
        assert nav(app, "escape")["column"] == "editor"
        wait_focus(app, ["note-editor"])

        # Into the board from the notes list, by keyboard.
        app.client.call("column.open", {"column": "notes"})
        wait_focus(app, [])
        state = nav(app, "enter")
        assert state["column"] == "editor" and state["selected_note"] == note
        wait_focus(app, ["note-editor"])

        # A Markdown note mounts the editor under the same id, and a board
        # mounts the canvas again.
        open_note(app, NOTES)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert app.client.call("editor.get")["text"].startswith("#")
        assert board(app)["surface"] == "text"
        open_note(app, FLOW)
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert_board_shown(app)
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert stamp(flow) == before


def test_a_board_opens_fitted_with_every_node_in_view(samples: Path) -> None:
    """§6.8 Phase 1 item 2."""
    with board_app(samples, "board-opens") as app:
        rows = [row for row in app.client.call("session.get")["notes"] if row["path"] == FLOW]
        assert [row["title"] for row in rows] == ["Checkout flow"]
        open_note(app, FLOW)
        placed = fitted(app)
        state = board(app, elements=True)
        assert state["surface"] == "board" and state["enabled"] is True
        assert state["counts"] == {"nodes": 15, "edges": 17}
        assert len(state["nodes"]) == 15 and len(state["edges"]) == 17
        assert {node["text"] for node in state["nodes"]} >= {"Receive order", "Payment ok?", "End"}
        assert state["issues"] == [] and state["read_only"] is False and state["error"] is None
        assert 0.1 <= state["viewport"]["zoom"] <= 1.0
        assert len(placed["nodes"]) == 15 and len(placed["edges"]) == 17
        for node in placed["nodes"]:
            visible = node["visible"]
            assert visible and visible["width"] > 0 and visible["height"] > 0, node
            # The fit leaves 40 px around the content.
            root = placed["root"]
            assert visible["x"] >= root["x"] + 39 and visible["y"] >= root["y"] + 39, node


def fill_point(node: dict, placed: dict) -> tuple[int, int] | None:
    """A pixel at least 4 px inside a filled rectangle, clear of every
    label, other node and connector."""
    rect = node["bounds"]
    margin = 6
    candidates = [
        (rect["x"] + margin, rect["y"] + rect["height"] / 2),
        (rect["x"] + rect["width"] - margin, rect["y"] + rect["height"] / 2),
        (rect["x"] + rect["width"] / 2, rect["y"] + margin),
        (rect["x"] + rect["width"] / 2, rect["y"] + rect["height"] - margin),
        (rect["x"] + margin, rect["y"] + margin),
    ]
    for point in candidates:
        clear = (
            not any(inside(point, other["label"], 2) for other in placed["nodes"])
            and not any(inside(point, edge["label"], 2) for edge in placed["edges"])
            and not any(inside(point, other["bounds"], 2)
                        for other in placed["nodes"] if other["id"] != node["id"])
            and all(distance_to_polyline(point, edge["points"]) > 4 for edge in placed["edges"])
        )
        if clear:
            return round(point[0]), round(point[1])
    return None


def stroke_point(edge: dict, placed: dict) -> tuple[int, int] | None:
    """The middle of a connector, when it is in view and clear of every
    node, label and other connector."""
    point = midpoint(edge["points"])
    root = placed["root"]
    clear = (
        inside(point, root, -8)
        and not any(inside(point, node["bounds"], 4) for node in placed["nodes"])
        and not any(inside(point, node["label"], 4) for node in placed["nodes"])
        and not any(inside(point, other["label"], 4) for other in placed["edges"])
        and all(distance_to_polyline(point, other["points"]) > 6
                for other in placed["edges"] if other["id"] != edge["id"])
    )
    # A segment's own antialiasing is centred on it: snap across it.
    return (round(point[0]), round(point[1])) if clear else None


@pytest.mark.parametrize("theme", ["dark", "light"])
def test_the_board_is_drawn_in_its_colours(samples: Path, theme: str) -> None:
    """§6.8 Phase 1 item 3, on whichever renderer the lane runs."""
    artifacts = artifact_directory(f"board-pixels-{theme}")
    with WrenApp(workspace=samples, settings="boards = on\n", artifacts_dir=artifacts) as app:
        app.client.call("theme.set", {"id": theme})
        open_note(app, FLOW)
        placed = fitted(app)
        time.sleep(REST)
        image = screenshot(app, artifacts / "fitted.png")
        scale = placed["scale_factor"]

        def pixel(point: tuple[float, float]) -> tuple[int, ...]:
            return image.pixel(round(point[0] * scale), round(point[1] * scale))

        filled = 0
        for node in placed["nodes"]:
            if node["fill"] is None:
                continue
            point = fill_point(node, placed)
            if point is None:
                continue
            assert near(pixel(point), node["fill"], 8), (node["id"], point, pixel(point), node["fill"])
            filled += 1
        assert filled >= 5, filled

        # Labels: each label box holds glyph pixels, unlike its fill.
        inked = 0
        for node in placed["nodes"]:
            label, fill = node["label"], node["fill"]
            if label is None or fill is None:
                continue
            x0, y0 = label["x"] * scale, label["y"] * scale
            x1, y1 = x0 + label["width"] * scale, y0 + label["height"] * scale
            glyphs = sum(
                1
                for y in range(int(y0), int(y1))
                for x in range(int(x0), int(x1))
                if not near(image.pixel(x, y), fill, 24)
            )
            assert glyphs >= 10, (node["id"], glyphs)
            inked += 1
        assert inked >= 10, inked

        # A connector is 1.5 board units wide: at 200 % its middle pixel is
        # covered whole.
        app.client.call("command.run", {"id": "font.reset"})
        app.client.call("command.run", {"id": "font.increase"})
        app.client.call("command.run", {"id": "font.increase"})
        placed = bounds(app)
        assert placed["viewport"]["zoom"] == pytest.approx(2.0)
        time.sleep(REST)
        image = screenshot(app, artifacts / "zoomed.png")
        stroked = 0
        for edge in placed["edges"]:
            point = stroke_point(edge, placed)
            if point is None or edge["stroke"] is None:
                continue
            assert near(pixel(point), edge["stroke"], 24), (edge["id"], point, pixel(point), edge["stroke"])
            stroked += 1
        assert stroked >= 1, "no connector's middle was in view and clear"


RIGHT_TO_LEFT = '''schema = 1

[node.arabic]
text = "العربية"
kind = "shape"
at = [0, 0]
size = [160, 60]
z = "a"

[node.hebrew]
text = "שלום עולם"
kind = "shape"
at = [200, 0]
size = [160, 60]
z = "b"

[node.mixed]
text = "Send שלום עולם to العربية and back again"
kind = "shape"
at = [0, 100]
size = [160, 100]
z = "c"

[node.feed]
text = "abc\\nשלום"
kind = "shape"
at = [200, 100]
size = [160, 60]
z = "d"

[node.mark]
text = "\\u200Fmark first"
kind = "shape"
at = [0, 240]
size = [160, 60]
z = "e"

[node.free]
text = "טקסט חופשי"
kind = "text"
at = [200, 240]
size = [160, 40]
z = "f"

[edge.e]
kind = "edge"
from = { node = "arabic", side = "right" }
to = { node = "hebrew", side = "left" }
route = "straight"
labels = [
  { text = "כן" },
]
z = "g"
'''


def ink(image, rect: dict, scale: float) -> tuple[int, float, float]:
    """How many pixels of a label box are not its most common colour, and
    the first and last column holding one, in logical pixels from its left."""
    x0, y0 = int(rect["x"] * scale), int(rect["y"] * scale)
    x1, y1 = int((rect["x"] + rect["width"]) * scale), int((rect["y"] + rect["height"]) * scale)
    pixels = {(x, y): image.pixel(x, y)[:3] for y in range(y0, y1) for x in range(x0, x1)}
    values = list(pixels.values())
    ground = max(set(values), key=values.count)
    columns = [x for (x, _), pixel in pixels.items() if not near(pixel, ground, 24)]
    if not columns:
        return 0, math.inf, -math.inf
    return len(columns), min(columns) / scale - rect["x"], (max(columns) + 1) / scale - rect["x"]


def test_right_to_left_labels_are_drawn(samples: Path) -> None:
    """A right-to-left line is drawn in its box, not past an unbounded one."""
    path = "Boards/Samples/Right to left.wren-board"
    (samples / path).write_text(RIGHT_TO_LEFT, encoding="utf-8")
    artifacts = artifact_directory("board-right-to-left")
    with board_app(samples, "board-right-to-left") as app:
        open_note(app, path)
        fitted(app)
        for step in ("fitted", "font.reset", "font.increase", "font.increase"):
            if step != "fitted":
                command(app, step)
            time.sleep(REST)
            placed = bounds(app)
            image = screenshot(app, artifacts / f"{step}-{placed['viewport']['zoom']:.2f}.png")
            scale = placed["scale_factor"]
            labels = [(node["id"], node["label"]) for node in placed["nodes"]]
            labels += [(edge["id"], edge["label"]) for edge in placed["edges"]]
            for name, label in labels:
                assert label is not None, (step, name)
                if not inside((label["x"], label["y"]), placed["root"]) or not inside(
                        (label["x"] + label["width"], label["y"] + label["height"]), placed["root"]):
                    continue
                count, left, right = ink(image, label, scale)
                # The ink spans the box the line was measured into: a line
                # placed past it is cut off on one side.
                assert count >= 10, (step, name, label)
                assert left <= 3 and right >= label["width"] - 3, (step, name, label, left, right)
        assert app.client.call("session.get")["document"]["dirty"] is False


FAR_ARROW = '''schema = 1

[node.a]
text = "near"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "a"

[node.b]
text = "far"
kind = "shape"
at = [-8000000, 8000000]
size = [120, 60]
z = "b"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
z = "c"

[edge.free]
kind = "edge"
from = { node = "a", side = "right" }
to = { at = [16777216, -16777216] }
route = "straight"
heads = ["arrow", "arrow"]
z = "d"
'''


def test_a_connector_millions_of_units_long_is_drawn_at_every_zoom(samples: Path) -> None:
    """Looking at one end of a connector whose other end, and arrowhead, lie
    millions of units away, at each zoom stop: the software renderer is
    never handed a path millions of pixels across."""
    path = "Boards/Samples/Far arrow.wren-board"
    (samples / path).write_text(FAR_ARROW, encoding="utf-8")
    artifacts = artifact_directory("board-far-arrow")
    with board_app(samples, "board-far-arrow") as app:
        open_note(app, path)
        # Too wide to fit: the view stops at the smallest zoom.
        wait_for(lambda: board(app)["viewport"] is not None, timeout=10, interval=0.05,
                 description="the board shown")
        command(app, "font.reset")
        placed = bounds(app)
        near_node = next(node for node in placed["nodes"] if node["id"] == "a")["bounds"]
        root = placed["root"]
        centre = [root["x"] + root["width"] / 2, root["y"] + root["height"] / 2]
        # Pan the near node to the middle of the widget.
        wheel(app, {"screen": centre},
              y=centre[1] - near_node["y"] - near_node["height"] / 2,
              x=centre[0] - near_node["x"] - near_node["width"] / 2, unit="pixels")
        near_node = next(node for node in bounds(app)["nodes"] if node["id"] == "a")
        assert inside(centre, near_node["visible"]), near_node
        zooms = []
        for index, step in enumerate(["font.increase"] * 4 + ["font.decrease"] * 8):
            command(app, step)
            time.sleep(0.2)
            zoom = board(app)["viewport"]["zoom"]
            zooms.append(round(zoom, 2))
            screenshot(app, artifacts / f"{index:02}-zoom-{zoom:.2f}.png")
            placed = bounds(app)
            near_node = next(node for node in placed["nodes"] if node["id"] == "a")
            assert near_node["visible"], (zoom, near_node)
        assert zooms == [1.5, 2.0, 3.0, 4.0, 3.0, 2.0, 1.5, 1.0, 0.75, 0.5, 0.25, 0.1]
        assert app.client.call("session.get")["document"]["dirty"] is False


def moved(before: dict, after: dict) -> set[tuple[float, float]]:
    """How far each node's box and label moved, rounded to 0.01 px."""
    deltas = set()
    for old, new in zip(before["nodes"], after["nodes"]):
        assert old["id"] == new["id"]
        for key in ("bounds", "label"):
            if old[key] is not None:
                deltas.add((round(new[key]["x"] - old[key]["x"], 2), round(new[key]["y"] - old[key]["y"], 2)))
    return deltas


def test_the_wheel_pans_and_mod_wheel_zooms_about_the_pointer(samples: Path) -> None:
    """§6.8 Phase 1 item 4, through `board.pointer`; the native X11 case
    turns the real wheel."""
    flow = samples / FLOW
    before_bytes = stamp(flow)
    with board_app(samples, "board-wheel") as app:
        open_note(app, FLOW)
        start = fitted(app)
        zoom = start["viewport"]["zoom"]
        root = start["root"]
        pointer = (root["x"] + root["width"] / 2, root["y"] + root["height"] / 2)

        # One line down pans by 40 logical px: every rectangle moves alike.
        wheel(app, {"screen": list(pointer)}, -1)
        after = bounds(app)
        assert moved(start, after) == {(0.0, -40.0)}
        time.sleep(0.2)  # a new burst
        wheel(app, {"screen": list(pointer)}, -1, mods=("shift",))
        assert moved(after, bounds(app)) == {(-40.0, 0.0)}
        time.sleep(0.2)

        # Mod+wheel zooms 1.1 a line about the pointer.
        placed = bounds(app)
        view = placed["viewport"]
        under = (view["x"] + (pointer[0] - root["x"]) / view["zoom"],
                 view["y"] + (pointer[1] - root["y"]) / view["zoom"])
        state = wheel(app, {"screen": list(pointer)}, 2, mods=("mod",))
        view = state["viewport"]
        assert view["zoom"] == pytest.approx(zoom * 1.1 ** 2, rel=1e-4)
        back = (root["x"] + (under[0] - view["x"]) * view["zoom"],
                root["y"] + (under[1] - view["y"]) * view["zoom"])
        assert abs(back[0] - pointer[0]) <= 1 and abs(back[1] - pointer[1]) <= 1, (back, pointer)

        # The text-size commands step the zoom and write no setting.
        config = app.settings_path
        settings = config.read_text(encoding="utf-8")
        font_size = app.client.call("editor.get")["font_size"]
        command(app, "font.reset")
        assert board(app)["viewport"]["zoom"] == pytest.approx(1.0)
        command(app, "font.increase")
        assert board(app)["viewport"]["zoom"] == pytest.approx(1.5)
        command(app, "font.decrease")
        assert board(app)["viewport"]["zoom"] == pytest.approx(1.0)
        assert config.read_text(encoding="utf-8") == settings
        assert app.client.call("editor.get")["font_size"] == font_size

        # A middle drag pans by the pointer's motion and gives the board
        # the editor column.
        app.client.call("column.open", {"column": "notes"})
        start = bounds(app)
        app.client.call("board.pointer", {"kind": "press", "button": "middle",
                                          "at": {"screen": list(pointer)}})
        assert board(app)["state"] == "panning"
        app.client.call("board.pointer", {"kind": "move",
                                          "at": {"screen": [pointer[0] + 30, pointer[1] - 20]}})
        app.client.call("board.pointer", {"kind": "release", "button": "middle"})
        assert moved(start, bounds(app)) == {(30.0, -20.0)}
        assert column(app) == "editor"
        wait_focus(app, ["note-editor"])
        assert board(app)["state"] == "idle"
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert stamp(flow) == before_bytes


def test_board_requests_refuse_a_surface_that_is_not_a_board(samples: Path) -> None:
    with board_app(samples, "board-requests") as app:
        open_note(app, NOTES)
        assert board(app)["surface"] == "text"
        for op, params in [
            ("board.bounds", {}),
            ("board.pointer", {"kind": "wheel", "at": {"canvas": [0, 0]}, "delta": {"y": 1}}),
            ("board.key", {"key": "r"}),
            ("board.type", {"text": "x"}),
            ("board.apply", {"edits": []}),
        ]:
            with pytest.raises(RemoteError, match="board"):
                app.client.call(op, params)
        open_note(app, FLOW)
        fitted(app)
        for params in [
            {"kind": "click", "button": "left", "at": {"node": "n1"}},
            {"kind": "press", "button": "fourth", "at": {"canvas": [0, 0]}},
            {"kind": "wheel", "at": {"edge": "e1"}, "delta": {"y": 1}},
        ]:
            with pytest.raises(RemoteError):
                app.client.call("board.pointer", params)


def test_a_raw_board_is_edited_in_the_focused_editor_and_flips_back(samples: Path) -> None:
    """§6.8 Phase 1 item 7."""
    raw = samples / CONFLICT
    text = conflict_board()
    raw.write_text(text, encoding="utf-8")
    with board_app(samples, "board-raw") as app:
        open_note(app, CONFLICT)
        state = board(app)
        assert state["surface"] == "raw" and state["counts"] is None
        assert state["error"]["line"] == 1 and state["error"]["message"], state
        app.client.call("editor.focus")
        wait_focus(app, ["note-editor"])
        assert app.client.call("editor.get")["text"] == text
        # The raw text is the editor's: it takes an edit, and is saved as typed.
        app.client.call("editor.insert", {"text": "# typed\n"})
        app.client.call("core.apply", {"op": "flush"})
        assert raw.read_text(encoding="utf-8") == "# typed\n" + text
        assert board(app)["surface"] == "raw", "typing never flips it"
        wait_focus(app, ["note-editor"])

        # Fixed on disk and reconciled, it is a board again.
        raw.write_text('schema = 1\n\n[node.a]\ntext = "Fixed"\nkind = "shape"\n'
                       'at = [0, 0]\nsize = [120, 60]\nz = "a"\n', encoding="utf-8")

        def flipped():
            app.client.call("core.apply", {"op": "reconcile"})
            return board(app)["surface"] == "board"

        wait_for(flipped, timeout=10, interval=0.2, description="the fixed board shown as a board")
        assert board(app)["counts"] == {"nodes": 1, "edges": 0}

        open_note(app, FUTURE)
        assert board(app)["read_only"] is True
        open_note(app, SHAPES)
        issues = board(app)["issues"]
        assert {"kind": "dangling_edge"} in [{"kind": issue["kind"]} for issue in issues], issues


def test_searching_a_label_finds_the_board_and_its_syntax_does_not(samples: Path) -> None:
    """§6.8 Phase 1 item 10."""
    with board_app(samples, "board-search") as app:
        app.client.call("palette.open", {"mode": "search"})
        modal = app.client.call("palette.query", {"query": "Payment"})["navigation"]["modal"]
        rows = [row for row in modal["results"] if row["path"] == FLOW]
        assert len(rows) == 1, modal["results"]
        assert rows[0]["hit"] == "content" and "Payment" in rows[0]["snippet"], rows[0]
        modal = app.client.call("palette.query", {"query": "schema"})["navigation"]["modal"]
        assert [row["path"] for row in modal["results"] if row["path"].endswith(".wren-board")] == []
        app.client.call("palette.close")


ESCAPES = '''schema = 1

[node.odd]
text = "a < b & \\"c\\""
kind = "shape"
at = [0, 0]
size = [160, 60]
z = "a"
'''


def export_rows(app: WrenApp) -> list[dict]:
    """The export command's rows in the Commands palette."""
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": "Export board"})["navigation"]["modal"]
    app.client.call("palette.close")
    return [row for row in modal["results"] if row["title"] == "Export board as SVG…"]


def test_export_svg_writes_the_shown_board_as_xml(samples: Path, tmp_path: Path) -> None:
    """§6.8 Phase 1 item 8, through what the command writes once its save
    dialog has closed."""
    (samples / "Boards/Samples/Escapes.wren-board").write_text(ESCAPES, encoding="utf-8")
    flow = samples / FLOW
    before = stamp(flow)
    text = "{http://www.w3.org/2000/svg}text"
    with board_app(samples, "board-export") as app:
        open_note(app, NOTES)
        assert [row["enabled"] for row in export_rows(app)] == [False]
        assert command(app, "note.export_svg")["disposition"] == "blocked"
        with pytest.raises(RemoteError, match="board"):
            app.client.call("board.export", {"format": "svg", "path": str(tmp_path / "x.svg")})

        open_note(app, FLOW)
        assert [row["enabled"] for row in export_rows(app)] == [True]
        out = tmp_path / "flow.svg"
        reply = app.client.call("board.export", {"format": "svg", "path": str(out)})
        assert reply["bytes"] == out.stat().st_size
        root = ElementTree.parse(out).getroot()
        labels = [node.text or "" for node in root.iter(text)]
        # Fifteen shapes, some of them two lines, and the connector labels.
        assert len(labels) > 15 and "Receive order" in labels, labels
        assert root[0].tag == "{http://www.w3.org/2000/svg}rect", "the canvas behind it all"

        open_note(app, "Boards/Samples/Escapes.wren-board")
        out = tmp_path / "escapes.svg"
        app.client.call("board.export", {"format": "svg", "path": str(out)})
        assert [node.text for node in ElementTree.parse(out).getroot().iter(text)] == [
            'a < b & "c"'
        ]
        assert '>a &lt; b &amp; &quot;c&quot;</text>' in out.read_text(encoding="utf-8")

        with pytest.raises(RemoteError, match="Could not export"):
            app.client.call("board.export", {
                "format": "svg", "path": str(tmp_path / "missing" / "x.svg"),
            })

        # Never over a note: the open board's own file, or a Markdown note.
        open_note(app, FLOW)
        notes = samples / NOTES
        kept = stamp(notes)
        for target in (flow, notes):
            with pytest.raises(RemoteError, match="Did not export the board over .*: it is a note"):
                app.client.call("board.export", {"format": "svg", "path": str(target)})
        assert stamp(notes) == kept
        assert board(app)["surface"] == "board"
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert stamp(flow) == before


def test_with_the_setting_off_there_is_no_export_command(samples: Path) -> None:
    with board_app(samples, "board-export-off", setting="off") as app:
        open_note(app, NOTES)
        assert export_rows(app) == []
        assert command(app, "note.export_svg")["disposition"] == "blocked"
