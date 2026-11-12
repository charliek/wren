"""Run the board spike's measurements against a real window.

Drives ``wren --board-spike N`` over the test socket (and xdotool, for the
key and wheel checks) and prints one JSON document per check. Run it under
a display, once per renderer, scale factor and build::

    cargo build --locked --release -p wren-iced --features smoke-test --target-dir target/perf
    WREN_TEST_BIN=$PWD/target/perf/release/wren ICED_BACKEND=tiny-skia \\
        python3 tools/run_display.py x11 -- \\
        uv run --locked --group test python tools/board_spike.py perf --shapes 1000

``WINIT_X11_SCALE_FACTOR=2`` gives scale factor 2 (set ``WREN_XVFB_SCREEN=2880x1800``
so the window fits the screen), and ``WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1
ICED_BACKEND=wgpu ICED_PRESENT_MODE=immediate`` selects software Vulkan.

Checks: ``perf`` (draw and frame times per painting strategy), ``layers``
(label occlusion, and what a layer costs), ``zoom`` (label placement and
sharpness at six zoom levels), ``measure`` (measured against drawn widths),
``keys``, ``wheel`` and ``label`` (focus, scroll units and in-canvas label
editing; these three need X11).
"""

from __future__ import annotations

import argparse
import json
import statistics
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools.wrentest.png import PNG, read_png  # noqa: E402
from tools.wrentest.ui import WrenApp, wait_for  # noqa: E402

STRATEGIES = ("transform", "rebuild", "settle")
SETTLE = 0.4
ZOOMS = (0.25, 0.5, 1.0, 1.5, 2.0, 4.0)


def write_png(path: Path, width: int, height: int, rgba: bytes) -> None:
    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    rows = b"".join(
        b"\x00" + rgba[row * width * 4:(row + 1) * width * 4] for row in range(height)
    )
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 6))
        + chunk(b"IEND", b"")
    )


def crop(image: PNG, left: int, top: int, width: int, height: int) -> bytes:
    return b"".join(
        bytes(channel for x in range(left, left + width) for channel in image.pixel(x, y))
        for y in range(top, top + height)
    )


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
        self.set(labels={"min_px": 0.0, "moving_lines": None})

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
    result: dict = {"shapes": args.shapes, "strategies": {}}
    first = spike.call("perf.board", {})
    result["open_micros"] = first["open_micros"]
    result["scene_micros"] = first["scene_micros"]
    state = spike.set()
    result["items"] = state["items"]
    result["layers"] = state["layers"]
    if args.layers:
        result["forced_layers"] = spike.set(layers=args.layers)["layers"]
    whole = fit(spike, state)
    result["fit_zoom"] = round(whole["zoom"], 3)
    # Each run starts either with the whole board in view, where nothing is
    # culled, or at 100 %, where most of a large board is.
    starts = {
        "fit": whole,
        "30": {"x": 300.0, "y": 150.0, "zoom": 0.3},
        "100": {"x": 300.0, "y": 150.0, "zoom": 1.0},
    }
    labels = None if args.renderer_labels else {"min_px": args.min_label_px, "moving_lines": args.moving_lines}
    result["labels"] = spike.set(labels=labels)["labels"] or "the renderer's rule"
    for strategy in args.strategy:
        spike.set(strategy=strategy, viewport=starts["100"])
        spike.rest()
        spike.call("perf.board", {"clear": True})
        spike.call("perf.view", {"clear": True})
        builds = spike.set()["builds"]
        for _ in range(30):
            spike.call("identify")
        idle = spike.call("perf.board", {"clear": True})
        views = spike.call("perf.view", {"clear": True})["view_micros"]
        idle_builds = spike.set()["builds"] - builds
        runs = {}
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
            runs[name] = {
                "frame_micros": summary(rings["frame_micros"]),
                "draw_micros": summary(moving),
                "builds": after["builds"] - before["builds"],
                "items_painted_per_step": round((after["painted"] - before["painted"]) / steps),
                "rest_draw_micros": max(at_rest) if at_rest else None,
                "rest_builds": rested["builds"] - after["builds"],
            }
        result["strategies"][strategy] = {
            "idle_view_micros": summary(views),
            "idle_draw_micros": summary(idle["draw_micros"]),
            "idle_builds": idle_builds,
            **runs,
        }
    return result


def region_ink(image: PNG, box: tuple[float, float, float, float], fill, tolerance: int) -> int:
    """How many pixels strictly inside ``box`` are not ``fill``."""
    left, top, right, bottom = box
    return sum(
        differs(image.pixel(x, y), fill, tolerance)
        for y in range(int(top) + 1, int(bottom))
        for x in range(int(left) + 1, int(right))
    )


def run_layers(spike: Spike, args) -> dict:
    result: dict = {"cases": [], "cost": {}}
    state = spike.set()
    overlap = state["overlaps"][0]
    region = overlap["region"]
    result["layers"] = state["layers"]
    result["overflow"] = state["overflow"]
    for zoom in (1.0, 2.0):
        for strategy in ("transform", "rebuild"):
            for forced, name in ((None, "split"), (1, "one-layer")):
                state = spike.set(
                    strategy=strategy,
                    layers=forced,
                    viewport={"x": region["x"] - 300 / zoom, "y": region["y"] - 200 / zoom, "zoom": zoom},
                )
                spike.rest()
                image, scale, path = spike.shot(f"s2-{name}-{strategy}-zoom{zoom:g}")
                left, top = spike.to_pixels(state, scale, region["x"], region["y"])
                right, bottom = spike.to_pixels(
                    state, scale, region["x"] + region["width"], region["y"] + region["height"]
                )
                ink = region_ink(image, (left, top, right, bottom), tuple(overlap["fill"]), 8)
                result["cases"].append({
                    "zoom": zoom, "strategy": strategy, "plan": name,
                    "layers": state["layers"],
                    "box": [round(left), round(top), round(right), round(bottom)],
                    "show_through_pixels": ink,
                    "screenshot": str(path),
                })
    spike.set(layers=None)
    for strategy in ("transform", "rebuild"):
        costs = {}
        for forced in (1, 2, 3, 4, 8):
            spike.set(strategy=strategy, layers=forced, viewport={"x": 300.0, "y": 150.0, "zoom": 1.0})
            spike.rest()
            spike.call("perf.board", {"clear": True})
            rings = spike.call("perf.board", {"pan": {"steps": 40, "dx": 8, "dy": 0}}, timeout=600)
            costs[forced] = {
                "frame_micros": summary(rings["frame_micros"]),
                "draw_micros": summary(rings["draw_micros"][-40:]),
            }
        result["cost"][strategy] = costs
    spike.set(layers=None)
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
    result: dict = {"cases": [], "crops": []}
    # Two plain labels and one with CJK and an emoji, none under a covering shape.
    chosen = [1, 2, 4]
    crops: dict[str, tuple[int, int, bytes]] = {}
    for zoom in ZOOMS:
        for strategy in ("transform", "rebuild"):
            worst = {"left": 0.0, "right": 0.0, "top": 0.0, "bottom": 0.0}
            slack = {"left": 0.0, "right": 0.0}
            lines, missing, shots = 0, [], []
            for chosen_node in chosen:
                bounds = spike.set(nodes=[chosen_node])["nodes"][0]["bounds"]
                state = spike.set(
                    strategy=strategy, nodes=[chosen_node],
                    viewport={"x": bounds["x"] - 20 / zoom, "y": bounds["y"] - 20 / zoom, "zoom": zoom},
                )
                spike.rest()
                image, scale, path = spike.shot(f"s3-{strategy}-zoom{zoom:g}-node{chosen_node}")
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
                if zoom == 2.0 and chosen_node == chosen[0]:
                    left, top = spike.to_pixels(state, scale, bounds["x"], bounds["y"])
                    width = round(bounds["width"] * unit)
                    height = round(bounds["height"] * unit)
                    crops[strategy] = (width, height, crop(image, round(left), round(top), width, height))
            # Ink may leave its line's box by at most one board unit, and one
            # device pixel for the edge the raster rounds to.
            limit = 1.0 + 1.0 / unit
            result["cases"].append({
                "zoom": zoom, "strategy": strategy, "lines": lines, "missing": missing,
                "ink_outside_box_units": {k: round(v, 2) for k, v in worst.items()},
                "ink_inside_box_units": {k: round(v, 2) for k, v in slack.items()},
                "limit_units": round(limit, 2),
                "pass": lines == 6 and all(v <= limit for v in worst.values()),
                "screenshots": shots,
            })
    if len(crops) == 2:
        (width, height, scaled), (_, _, native) = crops["transform"], crops["rebuild"]
        gap = bytes([255, 0, 255, 255]) * 8
        rows = b"".join(
            scaled[row * width * 4:(row + 1) * width * 4] + gap + native[row * width * 4:(row + 1) * width * 4]
            for row in range(height)
        )
        path = spike.out / "s3-zoom2-transform-beside-rebuild.png"
        write_png(path, width * 2 + 8, height, rows)
        different = sum(scaled[i:i + 3] != native[i:i + 3] for i in range(0, len(scaled), 4))
        largest = max((abs(a - b) for a, b in zip(scaled, native)), default=0)
        result["crops"].append({
            "path": str(path), "left": "transform (14 px scaled by 2)",
            "right": "rebuild (laid out at 28 px)",
            "pixels": width * height, "pixels_that_differ": different, "largest_channel_difference": largest,
        })
    result["pass"] = all(case.get("pass") for case in result["cases"])
    return result


def run_measure(spike: Spike, args) -> dict:
    labels = json.loads(Path(args.labels).read_text(encoding="utf-8"))
    parts = [part for label in labels for part in label.split("\n")]
    state = spike.set(scene={"labels": parts}, strategy="rebuild", viewport={"x": 0.0, "y": 0.0, "zoom": 1.0})
    spike.rest()
    image, scale, path = spike.shot("s4-labels")
    canvas = tuple(state["palette"]["canvas"])
    line_height = state["line_height"]
    rows = []
    for line in state["lines"]:
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
        "pass": worst is not None and worst <= 1.0 and all(case["drawn"] is not None for case in cases),
    }


def xdo(*args: str) -> str:
    done = subprocess.run(["xdotool", *args], check=True, capture_output=True, text=True, timeout=10)
    return done.stdout.strip()


def native_window(spike: Spike) -> str:
    windows = xdo("search", "--onlyvisible", "--pid", str(spike.app.pid)).splitlines()
    if len(windows) != 1:
        raise RuntimeError(f"expected one window, found {windows}")
    xdo("windowfocus", "--sync", windows[0])
    spike.call("editor.focus")
    return windows[0]


def pointer_over(spike: Spike, window: str, x: float = 400, y: float = 300) -> None:
    xdo("mousemove", "--window", window, "--sync",
        str(round(spike.bounds["x"] + x)), str(round(spike.bounds["y"] + y)))


def palette_open(spike: Spike) -> bool:
    return bool(spike.call("navigation.get").get("modal"))


def run_keys(spike: Spike, args) -> dict:
    window = native_window(spike)
    result: dict = {"focused_at_open": spike.focused}
    result["focus"] = spike.call("focus.get")["named"]
    before = spike.set()["arrows"]
    for key in ("Up", "Down", "Left", "Right"):
        xdo("key", "--window", window, "--clearmodifiers", key)
    wait_for(lambda: spike.set()["arrows"] == before + 4, description="four arrows captured")
    result["arrows_captured"] = spike.set()["arrows"] - before
    # A bound App shortcut still runs with the board focused.
    xdo("key", "--window", window, "--clearmodifiers", "alt+k")
    wait_for(lambda: palette_open(spike), description="the palette shortcut ran")
    result["shortcut_opened_palette"] = True
    result["focus_with_palette"] = spike.call("focus.get")["named"]
    counted = spike.set()["arrows"]
    xdo("key", "--window", window, "--clearmodifiers", "Down")
    time.sleep(0.3)
    result["arrows_while_unfocused"] = spike.set()["arrows"] - counted
    xdo("key", "--window", window, "--clearmodifiers", "Escape")
    wait_for(lambda: not palette_open(spike), description="Escape closed the palette")
    wait_for(lambda: spike.call("focus.get")["named"] == ["note-editor"], description="focus returned")
    result["focus_after_palette"] = spike.call("focus.get")["named"]
    # Escape with nothing open leaves the board focused.
    xdo("key", "--window", window, "--clearmodifiers", "Escape")
    time.sleep(0.3)
    result["focus_after_escape"] = spike.call("focus.get")["named"]
    xdo("key", "--window", window, "--clearmodifiers", "Up")
    wait_for(lambda: spike.set()["arrows"] == counted + 1, description="arrows captured again")
    result["pass"] = (
        result["focus"] == ["note-editor"]
        and result["arrows_captured"] == 4
        and result["arrows_while_unfocused"] == 0
        and result["focus_after_palette"] == ["note-editor"]
        and result["focus_after_escape"] == ["note-editor"]
    )
    return result


def run_wheel(spike: Spike, args) -> dict:
    window = native_window(spike)
    pointer_over(spike, window)
    result: dict = {"steps": []}
    spike.set(viewport={"x": 0.0, "y": 0.0, "zoom": 1.0})

    def step(name: str, action, expect, settle: float = 0.3) -> None:
        before = spike.set()
        action()
        time.sleep(settle)
        after = spike.set()
        deltas = after["wheel"][len(before["wheel"]):]
        start, end = before["viewport"], after["viewport"]
        x = sum(delta["x"] for delta in deltas)
        y = sum(delta["y"] for delta in deltas)
        if expect == "pan":
            want = {"x": start["x"] - x * 40 / start["zoom"], "y": start["y"] - y * 40 / start["zoom"],
                    "zoom": start["zoom"]}
        elif expect == "sideways":
            want = {"x": start["x"] - y * 40 / start["zoom"], "y": start["y"], "zoom": start["zoom"]}
        elif expect == "zoom":
            zoom = start["zoom"]
            for delta in deltas:
                zoom *= 1.1 ** delta["y"]
            # About the pointer: the board point under it stays put.
            px = round(spike.bounds["x"] + 400) - spike.bounds["x"]
            py = round(spike.bounds["y"] + 300) - spike.bounds["y"]
            want = {"x": start["x"] + px / start["zoom"] - px / zoom,
                    "y": start["y"] + py / start["zoom"] - py / zoom, "zoom": zoom}
        else:
            want = {"x": start["x"] - expect[0] / start["zoom"], "y": start["y"] - expect[1] / start["zoom"],
                    "zoom": start["zoom"]}
        ok = all(abs(end[key] - want[key]) < 0.01 for key in want) and (bool(deltas) or expect not in ("pan", "sideways", "zoom"))
        result["steps"].append({"input": name, "deltas": deltas, "from": start, "to": end, "as_expected": ok})

    step("button 5 (wheel down)", lambda: xdo("click", "5"), "pan")
    step("button 4 (wheel up)", lambda: xdo("click", "4"), "pan")
    step("button 7 (wheel right)", lambda: xdo("click", "7"), "pan")
    step("button 6 (wheel left)", lambda: xdo("click", "6"), "pan")
    step("shift + button 5", lambda: (xdo("keydown", "shift"),
                                      xdo("click", "5"),
                                      xdo("keyup", "shift")), "sideways")
    step("ctrl + button 4 (zoom in)", lambda: (xdo("keydown", "ctrl"),
                                               xdo("click", "4"),
                                               xdo("keyup", "ctrl")), "zoom")
    step("ctrl + button 5 (zoom out)", lambda: (xdo("keydown", "ctrl"),
                                                xdo("click", "5"),
                                                xdo("keyup", "ctrl")), "zoom")
    step("three fast button 5, ctrl pressed after the first (burst latch)", lambda: (
        xdo("click", "5"),
        xdo("keydown", "ctrl"),
        xdo("click", "--repeat", "2", "--delay", "20", "5"),
        xdo("keyup", "ctrl"),
    ), "pan")
    step("middle-button drag by (60, 30)", lambda: (
        xdo("mousedown", "2"),
        xdo("mousemove_relative", "--sync", "60", "30"),
        xdo("mouseup", "2"),
    ), (60, 30))
    result["pass"] = all(step["as_expected"] for step in result["steps"])
    return result


def run_label(spike: Spike, args) -> dict:
    window = native_window(spike)
    result: dict = {"zooms": []}

    def expect_ime(state) -> dict:
        caret, viewport, ime = state["caret"], state["viewport"], state["ime"]
        if caret is None or ime is None:
            return {"zoom": viewport["zoom"], "ime": ime, "tracks": False}
        want = {
            "x": spike.bounds["x"] + (caret["x"] - viewport["x"]) * viewport["zoom"],
            "y": spike.bounds["y"] + (caret["y"] - viewport["y"]) * viewport["zoom"],
            "height": caret["height"] * viewport["zoom"],
        }
        tracks = all(abs(ime["cursor"][key] - want[key]) <= 0.5 for key in want)
        return {"zoom": viewport["zoom"], "ime_cursor": ime["cursor"], "expected": want,
                "preedit_text_size": ime["text_size"], "tracks": tracks}

    spike.set(strategy="settle", viewport={"x": -60.0, "y": -40.0, "zoom": 0.5})
    xdo("key", "--window", window, "--clearmodifiers", "Return")
    wait_for(lambda: spike.set()["label"] is not None, description="Enter opened the label")
    opened = spike.set()["label"]
    xdo("type", "--window", window, "--clearmodifiers", "--delay", "20", " ok")
    wait_for(lambda: spike.set()["label"]["text"] == opened["text"] + " ok", description="typed text")
    xdo("key", "--window", window, "--clearmodifiers", "BackSpace")
    xdo("key", "--window", window, "--clearmodifiers", "Return")
    xdo("type", "--window", window, "--clearmodifiers", "--delay", "20", "x")
    typed = wait_for(
        lambda: (s := spike.set())["label"]["text"] == opened["text"] + " o\nx" and s,
        description="backspace, a line break and a letter",
    )
    result["typed"] = typed["label"]["text"]
    for zoom in (0.5, 2.0):
        spike.set(viewport={"x": -60.0, "y": -40.0, "zoom": zoom})
        composing = spike.set(label={"preedit": "にほんご"})
        spike.rest()
        image, scale, path = spike.shot(f"s8-preedit-zoom{zoom:g}")
        state = spike.set()
        entry = expect_ime(state)
        entry["preedit"] = state["label"]["preedit"]
        entry["text_while_composing"] = composing["label"]["text"]
        entry["screenshot"] = str(path)
        # Keys belong to the composition: Escape must not close the label.
        xdo("key", "--window", window, "--clearmodifiers", "Escape")
        time.sleep(0.3)
        still = spike.set()
        entry["escape_while_composing"] = {
            "label_open": still["label"] is not None,
            "preedit": still["label"]["preedit"] if still["label"] else None,
            "focus": spike.call("focus.get")["named"],
        }
        committed = spike.set(label={"commit": "日本語"})
        entry["committed"] = committed["label"]["text"]
        spike.rest()
        entry["caret_tracks_after_commit"] = expect_ime(spike.set())["tracks"]
        result["zooms"].append(entry)
    image, scale, path = spike.shot("s8-caret-zoom2")
    result["caret_screenshot"] = str(path)
    final = spike.set()["label"]["text"]
    xdo("key", "--window", window, "--clearmodifiers", "Escape")
    wait_for(lambda: spike.set()["label"] is None, description="Escape committed the label")
    after = spike.set(nodes=[typed["label"]["node"]])
    result["escape"] = {
        "label_open": after["label"] is not None,
        "focus": spike.call("focus.get")["named"],
        "text_kept": "\n".join(line["text"] for line in after["nodes"][0]["lines"]) == final,
        "ime_after_close": after["ime"],
    }
    result["final_text"] = final
    result["pass"] = (
        final == opened["text"] + " o\nx日本語日本語"
        and all(z["tracks"] and z["caret_tracks_after_commit"] and z["preedit"] == "にほんご"
                and z["escape_while_composing"]["label_open"]
                and z["escape_while_composing"]["preedit"] == "にほんご"
                and z["escape_while_composing"]["focus"] == ["note-editor"]
                and z["text_while_composing"] == typed["label"]["text"] + ("" if z["zoom"] == 0.5 else "日本語")
                for z in result["zooms"])
        and result["escape"] == {"label_open": False, "focus": ["note-editor"], "text_kept": True,
                                 "ime_after_close": None}
    )
    return result


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


CHECKS = {
    "perf": run_perf, "layers": run_layers, "zoom": run_zoom, "measure": run_measure,
    "keys": run_keys, "wheel": run_wheel, "label": run_label,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("check", choices=sorted(CHECKS))
    parser.add_argument("--shapes", type=int, default=1000)
    parser.add_argument("--strategy", action="append", choices=STRATEGIES)
    parser.add_argument("--layers", type=int, help="perf: paint in this many forced layers")
    parser.add_argument("--min-label-px", type=float, default=0.0,
                        help="perf: do not draw labels smaller than this on screen (rebuilt geometry only)")
    parser.add_argument("--renderer-labels", action="store_true",
                        help="perf: leave the label rule to the widget, which picks it by renderer")
    parser.add_argument("--moving-lines", type=int,
                        help="perf: while the view moves, draw labels only if at most this many "
                             "lines are in view (rebuilt geometry only)")
    parser.add_argument("--labels", help="measure: a JSON file holding a list of labels")
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "board-spike")
    args = parser.parse_args()
    args.strategy = args.strategy or list(STRATEGIES)
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
