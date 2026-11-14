"""Board export as PNG and as draw.io, with the `boards` setting on, and
`wren export` (discovery/boards.md §3.8, §5.19, §6.8 Phase 3 items 5, 6
and 9, §7.4 EX1, EX2 and the draw.io paragraph).

`board.export` writes what Export board as PNG… writes once its save dialog
has closed: the board's SVG export, as drawn when the command ran, rendered
by resvg at twice its size on the theme's canvas colour; and what Export
board as draw.io… writes: the board as one draw.io page in Wren Light's
colours. `wren export` writes the same files from the command line with no
display.
"""

from __future__ import annotations

import math
import re
import subprocess
import threading
import time
import xml.etree.ElementTree as ElementTree
from pathlib import Path

import pytest

from tools.wrentest.boards import (
    FLOW, GROUP, NOTES, board, board_app, conflict_board, copy_samples, fitted, grid_board, near,
    open_note, screenshot, stamp,
)
from tools.wrentest.client import Client, RemoteError
from tools.wrentest.png import PNG, read_png
from tools.wrentest.test_setup import artifact_directory, binary_path, isolated_environment
from tools.wrentest.ui import WrenApp, wait_for

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


def export_rows(app: WrenApp, title: str = "Export board as PNG…") -> list[dict]:
    """An export command's rows in the Commands palette, the PNG export's
    unless another title is given."""
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": "Export board"})["navigation"]["modal"]
    app.client.call("palette.close")
    return [row for row in modal["results"] if row["title"] == title]


DRAWIO = "Export board as draw.io…"


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
        assert export_rows(app, DRAWIO) == []
        assert command(app, "note.export_drawio")["disposition"] == "blocked"


@pytest.mark.parametrize("theme", ["dark", "light"])
def test_a_drawio_export_is_the_board_as_one_draw_io_page(
    samples: Path, tmp_path: Path, theme: str,
) -> None:
    """§7.4's draw.io paragraph and §6.8 Phase 3 item 9: the flow exports as
    one uncompressed draw.io page, well-formed XML, one ``<object>`` for
    each node and connector carrying its Wren id, built-in shapes only, in
    Wren Light's colours whatever the app's theme: the bytes ``wren export
    --format drawio`` writes. The board is untouched; inside the workspace
    the export is only a .drawio file."""
    flow = samples / FLOW
    before = stamp(flow)
    out = tmp_path / "flow.drawio"
    with board_app(samples, f"board-export-drawio-{theme}") as app:
        app.client.call("theme.set", {"id": theme})
        open_note(app, NOTES)
        assert [row["enabled"] for row in export_rows(app, DRAWIO)] == [False]
        open_note(app, FLOW)
        fitted(app)
        assert [row["enabled"] for row in export_rows(app, DRAWIO)] == [True]
        reply = app.client.call("board.export", {"format": "drawio", "path": str(out)})
        assert reply["bytes"] == out.stat().st_size
        state = board(app, elements=True)
        inside = samples / GROUP / "Flow.svg"
        with pytest.raises(RemoteError, match=r"inside the workspace an export is an \.drawio file"):
            app.client.call("board.export", {"format": "drawio", "path": str(inside)})
        assert not inside.exists()
        assert app.client.call("session.get")["document"]["dirty"] is False

    root = ElementTree.parse(out).getroot()
    assert root.tag == "mxfile"
    (diagram,) = root.findall("diagram")
    assert diagram.get("name") == "Checkout flow"
    cells = diagram.find("mxGraphModel/root")
    assert [cell.get("id") for cell in cells[:2]] == ["0", "1"]
    objects = cells.findall("object")
    ids = [node["id"] for node in state["nodes"]] + [edge["id"] for edge in state["edges"]]
    assert sorted(item.get("wrenId") for item in objects) == sorted(ids)
    assert len(objects) == len(cells) - 2
    for item in objects:
        style = item.find("mxCell").get("style")
        assert "mxgraph." not in style and "wren" not in style, style
    labels = {item.get("label") for item in objects}
    assert {"Start", "Check stock", "Payment ok?", "Yes", "No"} <= labels
    result = wren_export(tmp_path / "home", flow, "-o", tmp_path / "cli.drawio")
    assert result.returncode == 0, result.stderr
    assert (tmp_path / "cli.drawio").read_bytes() == out.read_bytes()
    assert stamp(flow) == before


def test_the_window_answers_while_a_large_png_export_renders(tmp_path: Path) -> None:
    """A PNG export renders and writes on a thread of its own: while a
    board of about 1,450 elements (54 megapixels) is written, the window
    answers requests at once, the board is still shown, and a second export
    is refused until the first has finished; the export's own reply comes
    when the file is written."""
    work = tmp_path / "work"
    (work / GROUP).mkdir(parents=True)
    (work / GROUP / "Large.wren-board").write_text(grid_board(500), encoding="utf-8")
    out = tmp_path / "large.png"
    with board_app(work, "board-export-worker") as app:
        open_note(app, f"{GROUP}/Large.wren-board")
        wait_for(lambda: board(app)["surface"] == "board", timeout=30, interval=0.05,
                 description="the board shown")
        assert board(app)["export"] is None
        finished: dict = {}

        def export_large() -> None:
            finished["started"] = started = time.monotonic()
            try:
                finished["reply"] = app.client.call(
                    "board.export", {"format": "png", "path": str(out)}, timeout=120)
            except Exception as error:  # noqa: BLE001 - reported below
                finished["error"] = error
            finished["seconds"] = time.monotonic() - started

        other = Client(app.client.socket_path)
        other.connect()
        worker = threading.Thread(target=export_large)
        try:
            worker.start()
            wait_for(lambda: (other.call("board.get")["export"] == "writing"
                              or not worker.is_alive()),
                     timeout=30, interval=0.002, description="the export being written")
            seen = time.monotonic()
            answers = []
            while worker.is_alive() and len(answers) < 20:
                started = time.monotonic()
                state = other.call("board.get")
                answers.append((time.monotonic() - started, state["export"], state["surface"]))
                if len(answers) == 1:
                    with pytest.raises(RemoteError, match="is still being written"):
                        other.call("board.export",
                                   {"format": "svg", "path": str(tmp_path / "second.svg")})
                    refused = other.call("command.run", {"id": "note.export_png"})
                    assert refused["disposition"] == "blocked", refused
        finally:
            worker.join(timeout=120)
            other.close()
        assert "error" not in finished, finished
        assert answers and answers[0][1] == "writing", (answers, finished)
        print(f"export {finished['seconds']:.3f} s; answers while writing: "
              f"{[round(answer[0], 4) for answer in answers if answer[1] == 'writing']}")
        writing = [answer for answer in answers if answer[1] == "writing"]
        assert all(surface == "board" for _, _, surface in writing), answers
        # The export was seen under way early in its render, and each answer
        # took a fraction of it: on the UI thread the render held every
        # request for all of it (§8.5, boards-p3-harden).
        assert seen - finished["started"] < finished["seconds"] / 2, (seen, finished)
        assert max(seconds for seconds, _, _ in writing) < finished["seconds"] / 4, (
            answers, finished)
        assert finished["reply"]["bytes"] == out.stat().st_size
        header = out.read_bytes()[:24]
        # Too large for the harness's decoder: the signature and the size.
        assert header[:8] == b"\x89PNG\r\n\x1a\n" and header[12:16] == b"IHDR", header
        assert int.from_bytes(header[16:20], "big") > 10_000, header
        assert not (tmp_path / "second.svg").exists()
        assert board(app)["export"] is None
        assert [row["enabled"] for row in export_rows(app)] == [True]


def wren_export(home: Path, *args: str | Path) -> subprocess.CompletedProcess[bytes]:
    """``wren export`` with these arguments, run with no display, no test
    mode and an empty configuration under ``home``."""
    home.mkdir()
    env = isolated_environment(home, test_mode=False)
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    return subprocess.run(
        [str(binary_path()), "export", *map(str, args)],
        env=env, capture_output=True, timeout=60, check=False,
    )


@pytest.mark.parametrize("theme", ["dark", "light"])
def test_wren_export_writes_what_the_app_exports(
    samples: Path, tmp_path: Path, theme: str,
) -> None:
    """§6.8 Phase 3 item 6: with no display, ``wren export`` exits 0 and
    writes the bytes ``board.export`` writes for the same board in the same
    theme, SVG and PNG; at another scale its PNG is the scene's bounds times
    that scale and each shape's middle is the fill the app drew (item 5)."""
    artifacts = artifact_directory(f"board-export-cli-{theme}")
    flow, swatch = samples / FLOW, samples / SWATCHES
    before = stamp(flow), stamp(swatch)
    app_out = tmp_path / "app"
    app_out.mkdir()
    with board_app(samples, f"board-export-cli-app-{theme}") as app:
        app.client.call("theme.set", {"id": theme})
        open_note(app, FLOW)
        fitted(app)
        for extension in ("svg", "png"):
            app.client.call(
                "board.export", {"format": extension, "path": str(app_out / f"flow.{extension}")},
            )
        open_note(app, SWATCHES)
        placed = fitted(app)
        nodes = {node["id"]: node for node in board(app, elements=True)["nodes"]}
        app.client.call("board.export", {"format": "png", "path": str(app_out / "swatches.png")})
        app.client.call("board.export", {"format": "svg", "path": str(app_out / "swatches.svg")})
        assert app.client.call("session.get")["document"]["dirty"] is False

    for extension in ("svg", "png"):
        out = artifacts / f"flow-cli.{extension}"
        result = wren_export(tmp_path / f"home-{extension}", flow, "-o", out, "--theme", theme)
        assert result.returncode == 0, result.stderr
        assert out.read_bytes() == (app_out / f"flow.{extension}").read_bytes(), extension

    x, y, width, height = view_box(app_out / "swatches.svg")
    shown = read_png((app_out / "swatches.png").read_bytes())
    for scale in (1, 3):
        out = artifacts / f"swatches-x{scale}.png"
        result = wren_export(
            tmp_path / f"home-x{scale}", swatch, "-o", out, "--theme", theme, "--scale", str(scale),
        )
        assert result.returncode == 0, result.stderr
        image = read_png(out.read_bytes())
        assert (image.width, image.height) == (math.ceil(width * scale), math.ceil(height * scale))
        for node in placed["nodes"]:
            (left, top), (w, h) = nodes[node["id"]]["at"], nodes[node["id"]]["size"]
            middle = (left + w / 2 - x, top + h / 2 - y)
            fill = shown.pixel(
                round(middle[0] * shown.width / width), round(middle[1] * shown.height / height),
            )
            assert near(fill, node["fill"], 8), (node["id"], fill, node["fill"])
            pixel = image.pixel(
                round(middle[0] * image.width / width), round(middle[1] * image.height / height),
            )
            assert near(pixel, fill, 8), (node["id"], scale, pixel, fill)
    assert (stamp(flow), stamp(swatch)) == before


def test_wren_export_of_a_missing_or_broken_board_writes_nothing(
    samples: Path, tmp_path: Path,
) -> None:
    """§6.8 Phase 3 item 6: a missing input and one that does not parse exit
    non-zero with a message and write nothing."""
    broken = samples / GROUP / "Broken.wren-board"
    broken.write_text(conflict_board(), encoding="utf-8")
    out = tmp_path / "out"
    out.mkdir()
    missing = wren_export(tmp_path / "home-missing", samples / "Missing.wren-board", "-o", out / "a.svg")
    assert missing.returncode == 1
    assert b"Could not read" in missing.stderr, missing.stderr
    unparsed = wren_export(tmp_path / "home-broken", broken, "-o", out / "b.png")
    assert unparsed.returncode == 3
    assert b"is not a board Wren can draw: line 1" in unparsed.stderr, unparsed.stderr
    beside = wren_export(tmp_path / "home-beside", broken)
    assert beside.returncode == 3
    assert list(out.iterdir()) == []
    assert not (samples / GROUP / "Broken.svg").exists()
