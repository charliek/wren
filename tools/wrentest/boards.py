"""Helpers for the board cases (discovery/boards.md §6.1, §6.2).

The sample workspace is copied before a case opens it, because opening a
workspace writes ``.wren/workspace.toml`` into it. A board with git conflict
markers is written at run time (:func:`conflict_board`), never committed, and
large boards come from :func:`grid_board`. It imports no pytest, so
``tools/capture_scenes.py`` can use it where pytest is not installed.
"""

from __future__ import annotations

import difflib
import math
import re
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


ROUTED = ("long random connectors", "a fan-in", "stacked shapes to one far shape")


def routed_board(kind: str, shapes: int) -> str:
    """One of the boards ``make perf-board`` times on which every connector
    spends its whole routing budget (``perf_parse_write_and_scene`` in
    ``crates/wren-board/src/lib.rs``), written as that test writes it, with
    the same seeded targets: ``kind`` is one of :data:`ROUTED`."""
    state = 3
    mask = (1 << 64) - 1

    def below(n: int) -> int:
        nonlocal state
        state = (state + 0x9E3779B97F4A7C15) & mask
        z = state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & mask
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & mask
        return (z ^ (z >> 31)) % max(n, 1)

    targets = [below(5100) for _ in range(5100)]
    grid = (lambda i: (i % 40 * 200, i // 40 * 100))
    if kind == ROUTED[0]:
        count, at, size = shapes, grid, (160, 64)
        edges = [(i, targets[i] % shapes) for i in range(shapes)]
    elif kind == ROUTED[1]:
        count, at, size = shapes, grid, (160, 64)
        edges = [(i, 0) for i in range(1, shapes)]
    elif kind == ROUTED[2]:
        count, size = shapes + 1, (100, 60)
        at = (lambda i: (5000, 5000) if i == shapes else (i % 7, i % 5))
        edges = [(i, shapes) for i in range(shapes)]
    else:
        raise ValueError(kind)
    names = ("rect", "rounded", "pill", "ellipse", "diamond", "parallelogram", "cylinder",
             "document")
    parts = ["schema = 1\n"]
    for i in range(count):
        x, y = at(i)
        parts.append(f'\n[node.n{i}]\ntext = "Step {i}\\nchecks the order"\nkind = "shape"\n'
                     f'at = [{x}, {y}]\nshape = "{names[i % len(names)]}"\n'
                     f'size = [{size[0]}, {size[1]}]\nz = "V{i}"\n')
    for k, (start, end) in enumerate(edges):
        parts.append(f'\n[edge.e{k}]\nkind = "edge"\nfrom = {{ node = "n{start}" }}\n'
                     f'to = {{ node = "n{end}" }}\nlabels = [\n  {{ text = "next" }},\n]\n'
                     f'z = "W{k}"\n')
    return "".join(parts)


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


def key(app: WrenApp, name: str, *mods: str, repeat: bool = False) -> dict:
    """A key pressed on the focused board (§6.2 ``board.key``): ``mod`` is
    ⌘ on macOS and Ctrl elsewhere."""
    return app.client.call("board.key", {"key": name, "mods": list(mods), "repeat": repeat})


def type_text(app: WrenApp, text: str) -> dict:
    """Text typed on the focused board: the open label's, or the connect
    query's."""
    return app.client.call("board.type", {"text": text})


def pointer(app: WrenApp, kind: str, at: dict | None = None, *, button: str = "left",
            mods: tuple[str, ...] = ()) -> dict:
    params: dict = {"kind": kind, "button": button, "mods": list(mods)}
    if at is not None:
        params["at"] = at
    return app.client.call("board.pointer", params)


def press(app: WrenApp, at: dict, *, button: str = "left", mods: tuple[str, ...] = ()) -> dict:
    return pointer(app, "press", at, button=button, mods=mods)


def click(app: WrenApp, at: dict, *, mods: tuple[str, ...] = (), double: bool = False) -> dict:
    return pointer(app, "double_click" if double else "click", at, mods=mods)


def hover(app: WrenApp, node: str) -> list[dict]:
    """Rest the pointer on a node until it shows its four connect handles
    (§5.5 rule 8), which the widget's own redraw brings up once the
    machine's 120 ms have passed. Returns them."""
    pointer(app, "move", {"node": node})

    def shown():
        handles = [h for h in bounds(app)["handles"]
                   if h["id"] == node and h["handle"].startswith("connect_")]
        return handles if len(handles) == 4 else None

    return wait_for(shown, timeout=5, interval=0.02, description="the connect handles shown")


def edge(app: WrenApp, id_: str) -> dict:
    """A connector's schema-1 fields."""
    return next(e for e in board(app, elements=True)["edges"] if e["id"] == id_)


def canvas_point(app: WrenApp, at: dict) -> tuple[float, float]:
    """Where ``at`` is in board units, as ``board.pointer`` reads it: a
    node's middle, one of the handles the board shows, a board point or a
    window point."""
    placed = bounds(app)
    view, root = placed["viewport"], placed["root"]

    def from_screen(x: float, y: float) -> tuple[float, float]:
        return view["x"] + (x - root["x"]) / view["zoom"], view["y"] + (y - root["y"]) / view["zoom"]

    if "canvas" in at:
        return tuple(at["canvas"])  # type: ignore[return-value]
    if "screen" in at:
        return from_screen(*at["screen"])
    if "handle" in at:
        rect = next(h["bounds"] for h in placed["handles"]
                    if h["id"] == at["node"] and h["handle"] == at["handle"])
    else:
        rect = next(n["bounds"] for n in placed["nodes"] if n["id"] == at["node"])
    return from_screen(rect["x"] + rect["width"] / 2, rect["y"] + rect["height"] / 2)


def drag(app: WrenApp, start: dict, by: tuple[float, float], *, button: str = "left",
         mods: tuple[str, ...] = (), steps: int = 4) -> dict:
    """Press at ``start``, move ``by`` board units in ``steps`` moves and
    release: one gesture, as a hand makes it."""
    x, y = canvas_point(app, start)
    press(app, start, button=button, mods=mods)
    for step in range(1, steps + 1):
        pointer(app, "move", {"canvas": [x + by[0] * step / steps, y + by[1] * step / steps]},
                button=button, mods=mods)
    return pointer(app, "release", button=button, mods=mods)


# The keys of the acceptance script's notation (§5.20), and the modifiers.
SCRIPT_KEYS = {"Tab": "tab", "Esc": "escape", "↵": "enter", "←": "left", "→": "right",
               "↑": "up", "↓": "down", "⌫": "backspace"}
SCRIPT_MODS = {"⌘": "mod", "⇧": "shift", "⌥": "alt"}
NUMBERS = {"two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8,
           "nine": 9, "ten": 10}


# The acceptance script of §5.20, as the plan writes it.
ACCEPTANCE = """
`U Tab` Start, `Tab` Receive order, `Esc D Tab` Valid?
`Esc ⌘→` Reject order, `Esc ⇧⌘← ↵` No `Esc ←`
`Tab` Check stock, `Esc ⇧⌘↑ ↵` Yes, `Esc D Tab` In stock?
`Esc ⌘→` Backorder, `Esc ⇧⌘← ↵` No, `Tab` Notify customer, `Esc C` che `↵`, `← ↓`
`Tab` Charge card, `Esc ⇧⌘↑ ↵` Yes, `Esc D Tab` Payment ok?
`Esc ⌘→` Retry payment, `Esc ⇧⌘← ↵` No, `Esc → C` cha `↵`, `← ↓`
`Tab` Pack, `Esc ⇧⌘↑ ↵` Yes, `Tab` Ship, `Tab` Send receipt, `Esc U Tab` End
`Esc`, `↑` eight times, `→`, `C` end `↵`
"""


def script_steps(text: str) -> list[tuple[str, ...]]:
    """The steps of a script written as §5.20 writes one: key presses in
    backticks, separated by spaces (``⇧⌘←``, ``Esc``, ``D``: a capital is
    its key, unshifted), and the text between them typed. ``N times`` after
    a group of keys presses it N times in all. Each step is ``("key", name,
    *mods)`` or ``("type", text)``."""
    steps: list[tuple[str, ...]] = []
    last: list[tuple[str, ...]] = []
    for index, part in enumerate(text.replace("\n", " ").split("`")):
        if index % 2:
            last = []
            for token in part.split():
                mods: list[str] = []
                while token and token[0] in SCRIPT_MODS and len(token) > 1:
                    mods.append(SCRIPT_MODS[token[0]])
                    token = token[1:]
                name = SCRIPT_KEYS.get(token, token.lower() if len(token) == 1 else None)
                if name is None:
                    raise ValueError(f"no key {token!r} in the script")
                last.append(("key", name, *mods))
            steps.extend(last)
            continue
        words = part.strip().strip(",").strip()
        repeat = re.fullmatch(r"(\w+) times", words)
        if repeat:
            count = NUMBERS.get(repeat.group(1)) or int(repeat.group(1))
            steps.extend(last * (count - 1))
        elif words:
            steps.append(("type", words))
    return steps


def script(app: WrenApp, text: str) -> dict:
    """Replay a script written as in §5.20 through ``board.key`` and
    ``board.type``; every key must reach the board. Returns the last
    snapshot."""
    state: dict = {}
    for step in script_steps(text):
        if step[0] == "key":
            state = key(app, step[1], *step[2:])
            assert state["handled"], (step, state)
        else:
            state = type_text(app, step[1])
    return state


def line_diff(before: bytes | str, after: bytes | str) -> list[str]:
    """The lines a save removed (``-``) and added (``+``), in order: what
    the product promises stays small for a small edit."""
    def lines(text: bytes | str) -> list[str]:
        return (text.decode("utf-8") if isinstance(text, bytes) else text).splitlines()

    return [line for line in difflib.ndiff(lines(before), lines(after))
            if line.startswith(("- ", "+ "))]


def blocks(text: bytes | str) -> list[str]:
    """The ``[node.…]`` and ``[edge.…]`` headers of a board file, in order."""
    text = text.decode("utf-8") if isinstance(text, bytes) else text
    return re.findall(r"^\[(?:node|edge)\.[^\]]+\]$", text, flags=re.MULTILINE)


def flushed(app: WrenApp) -> None:
    """Whatever the open note's draft holds is on disk."""
    app.client.call("core.apply", {"op": "flush"})
    wait_for(lambda: not app.client.call("session.get")["pending_durability"],
             timeout=10, interval=0.02, description="the flush")
