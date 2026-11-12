"""Helpers for the board cases (discovery/boards.md §6.1, §6.2).

The sample workspace is copied before a case opens it, because opening a
workspace writes ``.wren/workspace.toml`` into it. A board with git conflict
markers is written at run time (:func:`conflict_board`), never committed, and
large boards come from :func:`grid_board`. It imports no pytest, so
``tools/capture_scenes.py`` can use it where pytest is not installed.
"""

from __future__ import annotations

import math
import shutil
from pathlib import Path

from tools.wrentest.png import PNG, read_png
from tools.wrentest.ui import WrenApp, wait_for

ROOT = Path(__file__).resolve().parents[2]
SAMPLES = ROOT / "tools" / "wrentest" / "fixtures" / "boards" / "workspace"
GROUP = "Boards/Samples"
FLOW = f"{GROUP}/Checkout flow.wren-board"
SHAPES = f"{GROUP}/Shapes.wren-board"
OVERLAP = f"{GROUP}/Overlap.wren-board"
FUTURE = f"{GROUP}/Future.wren-board"
HAND_WRITTEN = f"{GROUP}/Hand written.wren-board"
NOTES = f"{GROUP}/Notes.md"
CONFLICT = f"{GROUP}/Conflict.wren-board"


def copy_samples(destination: Path) -> Path:
    """A copy of the sample workspace at ``destination``."""
    shutil.copytree(SAMPLES, destination)
    return destination


def conflict_board() -> str:
    """A board left with merge conflict markers, which is not TOML: line 1
    is where it stops being one."""
    ours = 'schema = 1\n\n[node.a]\ntext = "Ours"\nkind = "shape"\nat = [0, 0]\nsize = [120, 60]\nz = "a"\n'
    theirs = ours.replace("Ours", "Theirs")
    return f"<<<<<<< ours\n{ours}=======\n{theirs}>>>>>>> theirs\n"


def grid_board(shapes: int) -> str:
    """A board of ``shapes`` rounded rectangles of 160 by 64 in a grid, each
    with a two-line label and an elbow connector to its right and lower
    neighbour, as the spike's generator (``app/board_spike.rs``) lays out.
    Every tenth label has CJK text and an emoji, and one shape in ten is solid
    and sits over its left neighbour's label."""
    colors = ("red", "orange", "yellow", "green", "cyan", "purple", "blue")
    columns = max(1, math.ceil(math.sqrt(shapes) * 1.25))
    parts = ["schema = 1\n"]
    for index in range(shapes):
        covers = index % 10 == 9 and index % columns != 0
        x, y = (index % columns) * 220, (index // columns) * 120
        if covers:
            x, y = x - 120, y + 32
        label = (f"検証 {index} 🐦\\n二行目のラベル" if index % 10 == 4
                 else f"Step {index}: validate\\nrequest and reply")
        parts.append(
            f'\n[node.n{index}]\ntext = "{label}"\nkind = "shape"\nat = [{x}, {y}]\n'
            f'shape = "rounded"\nsize = [160, 64]\nstyle.color = "{colors[index % 7]}"\n'
            f'style.fill = "{"solid" if covers else "tint"}"\nz = "{index:06}a"\n')

        def connect(name: str, to: int, sides: tuple[str, str], z: str) -> None:
            parts.append(
                f'\n[edge.{name}{index}]\nkind = "edge"\n'
                f'from = {{ node = "n{index}", side = "{sides[0]}" }}\n'
                f'to = {{ node = "n{to}", side = "{sides[1]}" }}\nz = "{index:06}{z}"\n')

        if index + 1 < shapes and (index + 1) % columns != 0:
            connect("r", index + 1, ("right", "left"), "b")
        if index + columns < shapes:
            connect("d", index + columns, ("bottom", "top"), "c")
    return "".join(parts)


def grid_elements(shapes: int) -> int:
    """How many elements :func:`grid_board` writes: shapes and connectors."""
    columns = max(1, math.ceil(math.sqrt(shapes) * 1.25))
    right = sum(1 for index in range(shapes) if index + 1 < shapes and (index + 1) % columns != 0)
    down = max(0, shapes - columns)
    return shapes + right + down


def board_app(workspace: Path, case: str, setting: str = "on", **kwargs) -> WrenApp:
    from tools.wrentest.test_setup import artifact_directory

    return WrenApp(workspace=workspace, settings=f"boards = {setting}\n",
                   artifacts_dir=artifact_directory(case), **kwargs)


def note_id(app: WrenApp, path: str) -> int:
    return next(row for row in app.client.call("session.get")["notes"]
                if row["path"] == path)["id"]


def open_note(app: WrenApp, path: str) -> int:
    note = note_id(app, path)
    app.client.call("core.apply", {"op": "open_note", "note": note})
    return note


def board(app: WrenApp, elements: bool = False) -> dict:
    return app.client.call("board.get", {"elements": elements})


def bounds(app: WrenApp) -> dict:
    return app.client.call("board.bounds")


def fitted(app: WrenApp) -> dict:
    """The open board's bounds once its first fit has its widget's size."""
    def ready():
        state = board(app)
        if state["surface"] != "board" or state["viewport"] is None:
            return None
        placed = bounds(app)
        # Fitted boards sit inside the widget: a waiting fit is still at
        # the board's origin, full size.
        return placed if all(node["visible"] == node["bounds"] for node in placed["nodes"]) else None

    return wait_for(ready, timeout=10, interval=0.05, description="the board fitted")


def stamp(path: Path) -> tuple[bytes, int]:
    """What a file holds and when it was last written."""
    return path.read_bytes(), path.stat().st_mtime_ns


def wheel(app: WrenApp, at: dict, y: float, *, x: float = 0.0, mods: tuple[str, ...] = (),
          unit: str = "lines") -> dict:
    return app.client.call("board.pointer", {
        "kind": "wheel", "at": at, "mods": list(mods),
        "delta": {"x": x, "y": y, "unit": unit},
    })


def screenshot(app: WrenApp, path: Path) -> PNG:
    result = app.client.call("app.screenshot", {"path": str(path)})
    image = read_png(path.read_bytes())
    assert (image.width, image.height) == (result["width"], result["height"])
    return image


def near(pixel: tuple[int, ...], color: list[int] | tuple[int, ...], tolerance: int) -> bool:
    return all(abs(a - b) <= tolerance for a, b in zip(pixel[:3], color[:3]))


def inside(point: tuple[float, float], rect: dict | None, margin: float = 0.0) -> bool:
    if rect is None:
        return False
    x, y = point
    return (rect["x"] - margin <= x <= rect["x"] + rect["width"] + margin
            and rect["y"] - margin <= y <= rect["y"] + rect["height"] + margin)


def distance_to_polyline(point: tuple[float, float], points: list[list[float]]) -> float:
    best = math.inf
    for (ax, ay), (bx, by) in zip(points, points[1:]):
        dx, dy = bx - ax, by - ay
        length = dx * dx + dy * dy
        t = 0.0 if length == 0 else max(0.0, min(1.0, ((point[0] - ax) * dx + (point[1] - ay) * dy) / length))
        best = min(best, math.hypot(point[0] - (ax + t * dx), point[1] - (ay + t * dy)))
    return best


def midpoint(points: list[list[float]]) -> tuple[float, float]:
    """The point halfway along a polyline."""
    lengths = [math.hypot(bx - ax, by - ay) for (ax, ay), (bx, by) in zip(points, points[1:])]
    remaining = sum(lengths) / 2
    for ((ax, ay), (bx, by)), length in zip(zip(points, points[1:]), lengths):
        if remaining <= length and length > 0:
            t = remaining / length
            return ax + t * (bx - ax), ay + t * (by - ay)
        remaining -= length
    return tuple(points[-1])  # type: ignore[return-value]
