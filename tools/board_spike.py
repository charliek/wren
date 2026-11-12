"""Look at and time the board viewer in a real window.

Drives ``wren --board-spike N`` over the test socket and prints one JSON
document per check. Run it under a display, once per renderer, scale factor
and build::

    cargo build --locked --release -p wren-iced --features smoke-test --target-dir target/perf
    WREN_TEST_BIN=$PWD/target/perf/release/wren ICED_BACKEND=tiny-skia \\
        python3 tools/run_display.py x11 -- \\
        uv run --locked --group test python tools/board_spike.py perf --shapes 1000

``WINIT_X11_SCALE_FACTOR=2`` gives scale factor 2 (set ``WREN_XVFB_SCREEN=2880x1800``
so the window fits the screen), and ``WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1
ICED_BACKEND=wgpu ICED_PRESENT_MODE=immediate`` selects software Vulkan.

Checks: ``perf`` (scene, draw and frame times, and what an idle board
builds), ``layers`` (a label under a later shape does not show through it),
``zoom`` (label placement at six zoom levels) and ``measure`` (measured
against drawn widths).
"""

from __future__ import annotations

import argparse
import json
import math
import statistics
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools.wrentest.png import PNG, read_png  # noqa: E402
from tools.wrentest.ui import WrenApp  # noqa: E402

SETTLE = 0.4
ZOOMS = (0.25, 0.5, 1.0, 1.5, 2.0, 4.0)


def differs(a: tuple[int, ...], b: tuple[int, ...], tolerance: int) -> bool:
    return any(abs(x - y) > tolerance for x, y in zip(a[:3], b[:3]))


def percentile(values: list[int], share: float) -> int:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(share * (len(ordered) - 1)))]


def summary(values: list[int]) -> dict[str, int] | None:
    if not values:
        return None
    return {
        "n": len(values),
        "median": round(statistics.median(values)),
        "p95": percentile(values, 0.95),
        "max": max(values),
    }


class Spike:
    def __init__(self, app: WrenApp, out: Path) -> None:
        self.app = app
        self.call = app.client.call
        self.out = out
        focus = self.call("focus.get")
        self.bounds = focus["bounds"]["note-editor"]
        self.focused = focus["named"]
        # Every check but perf looks at labels, whatever the renderer's rule.
        self.set(labels={"min_px": 0.0, "moving": True})

    def set(self, **params):
        return self.call("spike.board", params, timeout=120)

    def rest(self) -> None:
        """Wait until the view has stopped moving and been redrawn at rest."""
        time.sleep(SETTLE)
        self.call("identify")
        time.sleep(0.05)

    def shot(self, name: str) -> tuple[PNG, float, Path]:
        path = self.out / f"{name}.png"
        # The app refuses to replace a capture.
        path.unlink(missing_ok=True)
        self.call("app.screenshot", {"path": str(path)}, timeout=120)
        image = read_png(path.read_bytes())
        window = self.bounds["x"] * 2 + self.bounds["width"]
        return image, image.width / window, path

    def to_pixels(self, state, scale: float, x: float, y: float) -> tuple[float, float]:
        viewport = state["viewport"]
        return (
            (self.bounds["x"] + (x - viewport["x"]) * viewport["zoom"]) * scale,
            (self.bounds["y"] + (y - viewport["y"]) * viewport["zoom"]) * scale,
        )


def fit(spike: Spike, state) -> dict[str, float]:
    """The viewport that shows the whole scene with a 40 px margin, at 100 % at most."""
    scene = state["scene_bounds"]
    width, height = spike.bounds["width"], spike.bounds["height"]
    zoom = max(0.1, min(1.0, (width - 80) / scene["width"], (height - 80) / scene["height"]))
    return {
        "x": scene["x"] - (width / zoom - scene["width"]) / 2,
        "y": scene["y"] - (height / zoom - scene["height"]) / 2,
        "zoom": zoom,
    }


def run_perf(spike: Spike, args) -> dict:
    result: dict = {"shapes": args.shapes}
    first = spike.call("perf.board", {})
    result["open_micros"] = first["open_micros"]
    result["scene_micros"] = first["scene_micros"]
    labels = {"min_px": 0.0, "moving": True} if args.all_labels else None
    state = spike.set(labels=labels)
    result["labels"] = state["labels"] or "the renderer's rule"
    result["items"] = state["items"]
    result["layers"] = state["layers"]
    whole = fit(spike, state)
    result["fit_zoom"] = round(whole["zoom"], 3)
    # Each run starts either with the whole board in view, where nothing is
    # culled, or at 100 %, where most of a large board is.
    starts = {
        "fit": whole,
        "30": {"x": 300.0, "y": 150.0, "zoom": 0.3},
        "100": {"x": 300.0, "y": 150.0, "zoom": 1.0},
    }
    spike.set(viewport=starts["100"])
    spike.rest()
    spike.call("perf.board", {"clear": True})
    spike.call("perf.view", {"clear": True})
    before = spike.set()
    for _ in range(30):
        spike.call("identify")
    idle = spike.call("perf.board", {"clear": True})
    views = spike.call("perf.view", {"clear": True})["view_micros"]
    after = spike.set()
    result["idle"] = {
        "view_micros": summary(views),
        "draw_micros": summary(idle["draw_micros"]),
        "scene_builds": after["scenes"] - before["scenes"],
        "geometry_builds": after["builds"] - before["builds"],
    }
    result["runs"] = {}
    for name, start, run in (
        ("pan at fit", "fit", {"pan": {"steps": 60, "dx": 8, "dy": 0}}),
        ("pan at 30", "30", {"pan": {"steps": 60, "dx": 8, "dy": 0}}),
        ("pan at 100", "100", {"pan": {"steps": 60, "dx": 8, "dy": 0}}),
        ("zoom from fit", "fit", {"zoom": {"steps": 30, "factor": 1.05}}),
        ("zoom from 100", "100", {"zoom": {"steps": 30, "factor": 1.05}}),
    ):
        spike.set(viewport=starts[start])
        spike.rest()
        spike.call("perf.board", {"clear": True})
        before = spike.set()
        rings = spike.call("perf.board", run, timeout=600)
        steps = next(iter(run.values()))["steps"]
        moving = rings["draw_micros"][-steps:]
        after = spike.set()
        spike.rest()
        at_rest = spike.call("perf.board", {"clear": True})["draw_micros"]
        rested = spike.set()
        result["runs"][name] = {
            "frame_micros": summary(rings["frame_micros"]),
            "draw_micros": summary(moving),
            "scene_builds": rested["scenes"] - before["scenes"],
            "builds": after["builds"] - before["builds"],
            "items_painted_per_step": round((after["painted"] - before["painted"]) / steps),
            "rest_draw_micros": max(at_rest) if at_rest else None,
            "rest_builds": rested["builds"] - after["builds"],
        }
    result["pass"] = (
        result["idle"]["scene_builds"] == 0
        and result["idle"]["geometry_builds"] == 0
        and all(run["scene_builds"] == 0 for run in result["runs"].values())
    )
    return result


def region_ink(image: PNG, box: tuple[float, float, float, float], fill, tolerance: int) -> int:
    """How many pixels strictly inside ``box`` are not ``fill``."""
    left, top, right, bottom = box
    return sum(
        differs(image.pixel(x, y), fill, tolerance)
        for y in range(int(top) + 1, int(bottom))
        for x in range(int(left) + 1, int(right))
    )


def label_box(node, line_height: float) -> tuple[float, float, float, float]:
    """The box around a node's label lines: left, top, right, bottom."""
    lines = node["lines"]
    return (
        min(line["x"] for line in lines),
        lines[0]["y"],
        max(line["x"] + line["width"] for line in lines),
        lines[-1]["y"] + line_height,
    )


def covered_region(spike: Spike, shapes: int):
    """Where a generated solid shape lies over its left neighbour's label,
    clear of its own label: the region in board units, and a point of the
    shape's fill with nothing over it."""
    columns = max(1, math.ceil(math.sqrt(shapes) * 1.25))
    cover = next(index for index in range(9, shapes, 10) if index % columns)
    state = spike.set(nodes=[f"n{cover - 1}", f"n{cover}"])
    under, over = state["nodes"]
    height = state["line_height"]
    shape = over["bounds"]
    hidden = label_box(under, height)
    own = label_box(over, height)
    left = max(hidden[0], shape["x"] + 2)
    top = max(hidden[1], shape["y"] + 2)
    right = min(hidden[2], shape["x"] + shape["width"] - 2, own[0] - 2)
    bottom = min(hidden[3], shape["y"] + shape["height"] - 2)
    if right - left < 8 or bottom - top < 8:
        raise RuntimeError(f"no covered label to look at: {under} under {over}")
    fill = (shape["x"] + 14, shape["y"] + shape["height"] - 5)
    return (left, top, right, bottom), fill


def run_layers(spike: Spike, args) -> dict:
    result: dict = {"cases": []}
    state = spike.set()
    result["layers"] = state["layers"]
    result["overflow"] = state["overflow"]
    region, fill_at = covered_region(spike, args.shapes)
    for zoom in (1.0, 2.0):
        for cap, name in ((4, "split"), (1, "one-layer")):
            state = spike.set(
                cap=cap,
                viewport={"x": region[0] - 300 / zoom, "y": region[1] - 200 / zoom, "zoom": zoom},
            )
            spike.rest()
            image, scale, path = spike.shot(f"layers-{name}-zoom{zoom:g}")
            left, top = spike.to_pixels(state, scale, region[0], region[1])
            right, bottom = spike.to_pixels(state, scale, region[2], region[3])
            fx, fy = spike.to_pixels(state, scale, *fill_at)
            fill = image.pixel(round(fx), round(fy))
            result["cases"].append({
                "zoom": zoom, "plan": name, "layers": state["layers"], "overflow": state["overflow"],
                "box": [round(left), round(top), round(right), round(bottom)],
                "fill": list(fill[:3]),
                "show_through_pixels": region_ink(image, (left, top, right, bottom), fill, 8),
                "screenshot": str(path),
            })
    spike.set(cap=4)
    split_ok = all(c["show_through_pixels"] == 0 for c in result["cases"] if c["plan"] == "split")
    control = all(c["show_through_pixels"] > 0 for c in result["cases"] if c["plan"] == "one-layer")
    result["pass"] = split_ok and control and result["overflow"] == 0
    return result


def ink_bounds(image: PNG, box, fill, margin=(0, 0, 0, 0), tolerance: int = 24):
    """The bounds of the pixels that are not ``fill`` in ``box`` widened by
    ``margin`` (left, top, right, bottom), in pixels."""
    left, top, right, bottom = box
    columns = range(max(0, round(left) - margin[0]), min(image.width, round(right) + margin[2]))
    rows = range(max(0, round(top) - margin[1]), min(image.height, round(bottom) + margin[3]))
    xs: list[int] = []
    ys: list[int] = []
    for y in rows:
        for x in columns:
            if differs(image.pixel(x, y), fill, tolerance):
                xs.append(x)
                ys.append(y)
    if not xs:
        return None
    return min(xs), min(ys), max(xs) + 1, max(ys) + 1


def run_zoom(spike: Spike, args) -> dict:
    result: dict = {"cases": []}
    # Two plain labels and one with CJK and an emoji, none under a covering shape.
    chosen = ["n1", "n2", "n4"]
    for zoom in ZOOMS:
        worst = {"left": 0.0, "right": 0.0, "top": 0.0, "bottom": 0.0}
        slack = {"left": 0.0, "right": 0.0}
        lines, missing, shots = 0, [], []
        for chosen_node in chosen:
            bounds = spike.set(nodes=[chosen_node])["nodes"][0]["bounds"]
            state = spike.set(
                nodes=[chosen_node],
                viewport={"x": bounds["x"] - 20 / zoom, "y": bounds["y"] - 20 / zoom, "zoom": zoom},
            )
            spike.rest()
            image, scale, path = spike.shot(f"zoom{zoom:g}-{chosen_node}")
            shots.append(str(path))
            unit = zoom * scale
            margin = max(1, int(3 * unit))
            line_height = state["line_height"]
            node = state["nodes"][0]
            fx, fy = spike.to_pixels(state, scale, bounds["x"] + 12, bounds["y"] + bounds["height"] / 2)
            fill = image.pixel(round(fx), round(fy))
            boxes = []
            for line in node["lines"]:
                left, top = spike.to_pixels(state, scale, line["x"], line["y"])
                right, bottom = spike.to_pixels(
                    state, scale, line["x"] + line["width"], line["y"] + line_height
                )
                boxes.append((left, top, right, bottom))
                # Sideways from this line's own rows only: the next
                # line's ink starts a few pixels below.
                ink = ink_bounds(image, (left, top, right, bottom), fill, (margin, 0, margin, 0))
                if ink is None:
                    missing.append(line["text"])
                    continue
                lines += 1
                worst["left"] = max(worst["left"], (left - ink[0]) / unit)
                worst["right"] = max(worst["right"], (ink[2] - right) / unit)
                slack["left"] = max(slack["left"], (ink[0] - left) / unit)
                slack["right"] = max(slack["right"], (right - ink[2]) / unit)
            # Up and down from the label as a whole.
            label = (min(b[0] for b in boxes), boxes[0][1], max(b[2] for b in boxes), boxes[-1][3])
            ink = ink_bounds(image, label, fill, (0, margin, 0, margin))
            if ink is not None:
                worst["top"] = max(worst["top"], (label[1] - ink[1]) / unit)
                worst["bottom"] = max(worst["bottom"], (ink[3] - label[3]) / unit)
        # Ink may leave its line's box by at most one board unit, and one
        # device pixel for the edge the raster rounds to.
        limit = 1.0 + 1.0 / unit
        result["cases"].append({
            "zoom": zoom, "lines": lines, "missing": missing,
            "ink_outside_box_units": {k: round(v, 2) for k, v in worst.items()},
            "ink_inside_box_units": {k: round(v, 2) for k, v in slack.items()},
            "limit_units": round(limit, 2),
            "pass": lines == 6 and all(v <= limit for v in worst.values()),
            "screenshots": shots,
        })
    result["pass"] = all(case.get("pass") for case in result["cases"])
    return result


def label_board(parts: list[str]) -> tuple[str, list[str]]:
    """A board of free text, one line per node, each line written between
    two bars so the ink shows its advance, under a first row of the two
    bars alone. JSON's string escapes are TOML's."""
    blocks = ["schema = 1\n"]
    ids = []
    for row, part in enumerate(["", *parts]):
        ids.append(f"l{row}")
        blocks.append(
            f"\n[node.l{row}]\ntext = {json.dumps(f'|{part}|', ensure_ascii=False)}\n"
            f'kind = "text"\nat = [20, {20 + row * 24}]\nsize = [4000, 20]\nz = "{row:04}a"\n'
        )
    return "".join(blocks), ids


def run_measure(spike: Spike, args) -> dict:
    labels = json.loads(Path(args.labels).read_text(encoding="utf-8"))
    parts = [part for label in labels for part in label.split("\n")]
    board, ids = label_board(parts)
    state = spike.set(scene={"text": board}, viewport={"x": 0.0, "y": 0.0, "zoom": 1.0}, nodes=ids)
    spike.rest()
    image, scale, path = spike.shot("measure-labels")
    canvas = tuple(state["palette"]["canvas"])
    line_height = state["line_height"]
    rows = []
    for node in state["nodes"]:
        line = node["lines"][0]
        left, top = spike.to_pixels(state, scale, line["x"], line["y"])
        right, bottom = spike.to_pixels(state, scale, line["x"] + line["width"], line["y"] + line_height)
        # A bar's ink may sit past the measured box, so look well beyond it,
        # but only in this row's own pixels.
        reach = round(40 * scale)
        ink = ink_bounds(image, (left, top + 2 * scale, right, bottom - 2 * scale), canvas, (reach, 0, reach, 0))
        rows.append({"text": line["text"], "measured": line["width"],
                     "ink": None if ink is None else (ink[2] - ink[0]) / scale,
                     "ink_right_past_box": None if ink is None else (ink[2] - right) / scale})
    bars = rows[0]
    cases = []
    for row in rows[1:]:
        measured = row["measured"] - bars["measured"]
        drawn = None if row["ink"] is None or bars["ink"] is None else row["ink"] - bars["ink"]
        cases.append({
            "label": row["text"][1:-1], "measured": round(measured, 2),
            "drawn": None if drawn is None else round(drawn, 2),
            "difference": None if drawn is None else round(drawn - measured, 2),
        })
    worst = max((abs(case["difference"]) for case in cases if case["difference"] is not None), default=None)
    return {
        "labels": len(cases), "bars": bars, "cases": cases, "worst_difference": worst,
        "screenshot": str(path),
        "pass": len(cases) == len(parts) and worst is not None and worst <= 1.0
        and all(case["drawn"] is not None for case in cases),
    }


def show(name: str, value, depth: int = 0) -> None:
    """Print a result with one line per record, so a run reads as a table."""
    def scalar(entry) -> bool:
        return not isinstance(entry, (dict, list)) or (
            isinstance(entry, list) and all(not isinstance(item, (dict, list)) for item in entry)
        )

    if scalar(value) or (isinstance(value, dict) and all(scalar(entry) for entry in value.values())):
        print(f"{'  ' * depth}{name}: {json.dumps(value, ensure_ascii=False)}")
        return
    print(f"{'  ' * depth}{name}:")
    for key, entry in value.items() if isinstance(value, dict) else enumerate(value):
        show(str(key), entry, depth + 1)


CHECKS = {"perf": run_perf, "layers": run_layers, "zoom": run_zoom, "measure": run_measure}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("check", choices=sorted(CHECKS))
    parser.add_argument("--shapes", type=int, default=1000)
    parser.add_argument("--all-labels", action="store_true",
                        help="perf: draw every label on every frame, instead of leaving the "
                             "label rule to the widget, which picks it by renderer")
    parser.add_argument("--labels", help="measure: a JSON file holding a list of labels")
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "board-spike")
    args = parser.parse_args()
    if args.check == "measure" and not args.labels:
        parser.error("measure needs --labels")
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    with WrenApp(extra_args=("--board-spike", str(args.shapes)), artifacts_dir=args.out / "logs",
                 readiness_timeout=120) as app:
        spike = Spike(app, args.out)
        result = CHECKS[args.check](spike, args)
    record = args.out / f"{args.check}.json"
    record.write_text(json.dumps(result, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    show(args.check, result)
    print(f"written to {record}")
    return 0 if result.get("pass", True) else 1


if __name__ == "__main__":
    raise SystemExit(main())
