"""Board export as PNG, with the `boards` setting on (discovery/boards.md
§3.8, §5.19, §6.8 Phase 3 item 5, §7.4 EX1).

`board.export` writes what Export board as PNG… writes once its save dialog
has closed: the board's SVG export, as drawn when the command ran, rendered
by resvg at twice its size on the theme's canvas colour.
"""

from __future__ import annotations

import math
import re
import time
import xml.etree.ElementTree as ElementTree
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, GROUP, NOTES, board, board_app, copy_samples, fitted, near, open_note, screenshot,
    stamp,
)
from tools.wrentest.client import RemoteError
from tools.wrentest.png import PNG, read_png
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp

# A pan or zoom step draws no label on the software renderer until the
# view has been still for 150 ms.
REST = 0.4
SCALE = 2
SWATCHES = f"{GROUP}/Swatches.wren-board"
SHAPES = ("rect", "rounded", "pill", "ellipse", "diamond", "parallelogram", "cylinder", "document")
COLORS = ("neutral", "red", "orange", "yellow", "green", "cyan", "purple", "blue")


def swatches() -> str:
    """One unlabelled shape of each kind in a row, each in its own colour,
    tint and solid in turn, so the middle of each is its fill."""
    parts = ["schema = 1\n"]
    for index, (shape, color) in enumerate(zip(SHAPES, COLORS)):
        fill = "solid" if index % 2 else "tint"
        parts.append(
            f'\n[node.s{index}]\nkind = "shape"\nat = [{index * 160}, 0]\nshape = "{shape}"\n'
            f'size = [120, 80]\nstyle.color = "{color}"\nstyle.fill = "{fill}"\nz = "a{index}"\n'
        )
    return "".join(parts)


@pytest.fixture
def samples(tmp_path: Path) -> Path:
    """A copy of the sample workspace with the swatch board: opening one
    writes into it."""
    work = copy_samples(tmp_path / "work")
    (work / SWATCHES).write_text(swatches(), encoding="utf-8")
    return work


def command(app: WrenApp, command_id: str) -> dict:
    return app.client.call("command.run", {"id": command_id})


def export_rows(app: WrenApp) -> list[dict]:
    """The PNG export command's rows in the Commands palette."""
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": "Export board"})["navigation"]["modal"]
    app.client.call("palette.close")
    return [row for row in modal["results"] if row["title"] == "Export board as PNG…"]


def view_box(svg: Path) -> tuple[float, float, float, float]:
    """The view box of an SVG export: the scene's bounds and its padding."""
    root = ElementTree.parse(svg).getroot()
    x, y, width, height = (float(part) for part in root.attrib["viewBox"].split())
    assert (float(root.attrib["width"]), float(root.attrib["height"])) == (width, height)
    return x, y, width, height


def export(app: WrenApp, out: Path) -> tuple[PNG, tuple[float, float, float, float]]:
    """The open board exported as PNG to ``out``, decoded, with the view
    box of its SVG export, which the image is twice the size of."""
    reply = app.client.call("board.export", {"format": "png", "path": str(out)})
    assert reply["bytes"] == out.stat().st_size
    svg = out.with_suffix(".svg")
    app.client.call("board.export", {"format": "svg", "path": str(svg)})
    box = view_box(svg)
    image = read_png(out.read_bytes())
    assert (image.width, image.height) == (
        math.ceil(box[2] * SCALE), math.ceil(box[3] * SCALE)
    ), (image.width, image.height, box)
    return image, box


def inked(image: PNG, box: tuple[float, float, float, float], node: dict) -> int:
    """How many pixels of the middle of a node's box, where its label is
    centred, are not the colour most of that area is."""
    x, y, width, height = box
    (left, top), (w, h) = node["at"], node["size"]
    columns = range(round((left + w / 4 - x) * image.width / width),
                    round((left + 3 * w / 4 - x) * image.width / width))
    rows = range(round((top + h / 3 - y) * image.height / height),
                 round((top + 2 * h / 3 - y) * image.height / height))
    pixels = [image.pixel(px, py) for py in rows for px in columns]
    common = max(set(pixels), key=pixels.count)
    return sum(1 for pixel in pixels if not near(pixel, common, 24))


@pytest.mark.parametrize("theme", ["dark", "light"])
def test_a_png_export_is_the_board_at_twice_its_size_in_its_colours(
    samples: Path, tmp_path: Path, theme: str,
) -> None:
    """§6.8 Phase 3 item 5: the PNG decodes, its size is the scene's bounds
    and padding times the scale, and each shape's middle is its fill colour
    within 8 per channel, in the PNG and in a window capture of the board."""
    artifacts = artifact_directory(f"board-export-png-{theme}")
    swatch = samples / SWATCHES
    before = stamp(swatch)
    with WrenApp(workspace=samples, settings="boards = on\n", artifacts_dir=artifacts) as app:
        app.client.call("theme.set", {"id": theme})
        open_note(app, SWATCHES)
        placed = fitted(app)
        nodes = {node["id"]: node for node in board(app, elements=True)["nodes"]}
        out = artifacts / "swatches.png"
        image, (x, y, width, height) = export(app, out)
        # The scene is the row of shapes, 40 units of padding around it.
        assert width == pytest.approx(7 * 160 + 120 + 80, abs=2), width
        assert height == pytest.approx(80 + 80, abs=2), height

        time.sleep(REST)
        window = screenshot(app, artifacts / "window.png")
        scale = placed["scale_factor"]
        canvas = None
        for node in placed["nodes"]:
            fill = node["fill"]
            assert fill is not None, node
            (left, top), (w, h) = nodes[node["id"]]["at"], nodes[node["id"]]["size"]
            middle = (
                round((left + w / 2 - x) * image.width / width),
                round((top + h / 2 - y) * image.height / height),
            )
            assert near(image.pixel(*middle), fill, 8), (node["id"], image.pixel(*middle), fill)
            assert image.pixel(*middle)[3] == 255, node["id"]
            # Unlabelled, so nothing is inked where a label would be.
            assert inked(image, (x, y, width, height), nodes[node["id"]]) < 5, node["id"]
            rect = node["bounds"]
            shown = (
                round((rect["x"] + rect["width"] / 2) * scale),
                round((rect["y"] + rect["height"] / 2) * scale),
            )
            assert near(window.pixel(*shown), fill, 8), (node["id"], window.pixel(*shown), fill)
            # Above the row is the canvas, in the export and on screen.
            canvas = window.pixel(shown[0], round((rect["y"] - 20) * scale))
            assert near(image.pixel(middle[0], 20), canvas, 8), (image.pixel(middle[0], 20), canvas)
        assert canvas is not None
        assert image.pixel(0, 0)[3] == 255 and near(image.pixel(0, 0), canvas, 8)
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert stamp(swatch) == before


def test_a_png_export_of_the_flow_draws_its_labels(samples: Path, tmp_path: Path) -> None:
    """The sample flowchart exports whole, labels inked, and an export
    leaves the board untouched; inside the workspace a PNG export is a
    .png file, never a note."""
    flow = samples / FLOW
    before = stamp(flow)
    with board_app(samples, "board-export-flow") as app:
        open_note(app, NOTES)
        assert [row["enabled"] for row in export_rows(app)] == [False]
        assert command(app, "note.export_png")["disposition"] == "blocked"
        with pytest.raises(RemoteError, match="board"):
            app.client.call("board.export", {"format": "png", "path": str(tmp_path / "x.png")})

        open_note(app, FLOW)
        fitted(app)
        assert [row["enabled"] for row in export_rows(app)] == [True]
        image, box = export(app, tmp_path / "flow.png")
        assert image.pixel(0, 0)[3] == 255
        nodes = board(app, elements=True)["nodes"]
        labelled = [node for node in nodes if node.get("text", "").strip()]
        assert len(labelled) == 15
        for node in labelled:
            assert inked(image, box, node) >= 30, node["text"]

        for target in (flow, samples / NOTES):
            with pytest.raises(RemoteError, match="Did not export the board over .*: it is a note"):
                app.client.call("board.export", {"format": "png", "path": str(target)})
        inside = samples / GROUP / "Flow.svg"
        with pytest.raises(RemoteError, match=r"inside the workspace an export is an \.png file"):
            app.client.call("board.export", {"format": "png", "path": str(inside)})
        assert not inside.exists()
        with pytest.raises(RemoteError, match="format"):
            app.client.call("board.export", {"format": "gif", "path": str(tmp_path / "x.gif")})
        assert board(app)["surface"] == "board"
        assert app.client.call("session.get")["document"]["dirty"] is False
    assert stamp(flow) == before


def test_labels_xml_cannot_carry_and_other_scripts_export(samples: Path, tmp_path: Path) -> None:
    """Markup, right-to-left and CJK labels, and a control character, which
    the SVG strips, render; a system without a CJK face draws Inter's
    missing-glyph box, so only the Latin label's ink is asserted."""
    labels = ['a < b & "c" ]]>', "שלום עולם", "日本語のラベル", "tab\\u0001here"]
    parts = ["schema = 1\n"]
    for index, label in enumerate(labels):
        parts.append(
            f'\n[node.l{index}]\ntext = "{label.replace(chr(34), chr(92) + chr(34))}"\n'
            f'kind = "shape"\nat = [0, {index * 100}]\nsize = [160, 60]\nz = "a{index}"\n'
        )
    (samples / GROUP / "Scripts.wren-board").write_text("".join(parts), encoding="utf-8")
    with board_app(samples, "board-export-scripts") as app:
        open_note(app, f"{GROUP}/Scripts.wren-board")
        state = board(app, elements=True)
        assert state["surface"] == "board", state["error"]
        image, (x, y, width, height) = export(app, tmp_path / "scripts.png")
        svg = (tmp_path / "scripts.svg").read_text(encoding="utf-8")
        assert "&lt; b &amp; &quot;c&quot; ]]&gt;" in svg
        assert not re.search("[\x00-\x08\x0b\x0c\x0e-\x1f]", svg)
        nodes = {node["id"]: node for node in state["nodes"]}
        assert inked(image, (x, y, width, height), nodes["l0"]) >= 30


def test_with_the_setting_off_there_is_no_png_export(samples: Path) -> None:
    with board_app(samples, "board-export-png-off", setting="off") as app:
        open_note(app, NOTES)
        assert export_rows(app) == []
        assert command(app, "note.export_png")["disposition"] == "blocked"
