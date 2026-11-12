"""The board's timings against §6.7's ceilings (discovery/boards.md §6.7).

Every lane asserts the debug ceilings, which catch only order-of-magnitude
regressions, and writes what it measured to ``measurements.json`` beside
the case's other artifacts. ``WRENTEST_PERF=release``, with
``WREN_TEST_BIN`` naming a release build with ``--features smoke-test``,
asserts the release budgets instead.
"""

from __future__ import annotations

import json
import os
import statistics
import time
from pathlib import Path

import pytest

from tools.wrentest.boards import board, board_app, grid_board, grid_elements
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import wait_for

MODE = os.environ.get("WRENTEST_PERF", "debug")
if MODE not in ("debug", "release"):
    raise pytest.UsageError(f"WRENTEST_PERF must be debug or release, got {MODE!r}")
RELEASE = MODE == "release"
RENDERER = os.environ.get("ICED_BACKEND", "")

# Milliseconds. Release: the spike's budgets as §8.5 records them (the edit
# budget replaced by the canvas chain's measured 12 ms with the whole board
# in view). Debug: §6.7's ceilings.
EDIT = 12 if RELEASE else 100
FRAME = 33 if RELEASE else 400
DRAW = 8
OPEN_1K = 150 if RELEASE else 5000
OPEN_10K = 1000 if RELEASE else 5000
VIEW = 1 if RELEASE else 5

GROUP = "Perf/Boards"
SHAPES = 1000
# The spike's generated boards of about 1,000 and 10,000 elements.
OPENS = {"1k": 347, "10k": 3380}


def median_ms(values: list[int]) -> float:
    return statistics.median(values) / 1000


@pytest.fixture
def workspace(tmp_path: Path) -> Path:
    group = tmp_path / "work" / GROUP
    group.mkdir(parents=True)
    (group / "Grid.wren-board").write_text(grid_board(SHAPES), encoding="utf-8")
    for name, shapes in OPENS.items():
        (group / f"Open {name}.wren-board").write_text(grid_board(shapes), encoding="utf-8")
    (group / "Note.md").write_text("# Note\n", encoding="utf-8")
    return tmp_path / "work"


def open_row(app, note: int) -> None:
    """Open a note as a click on its row does. `core.apply` would answer
    with the Session's text, over the harness's 1 MiB frame for the largest
    board."""
    app.client.call("row.click", {"row": {"note": note}})


def rings(app, **params) -> dict:
    return app.client.call("perf.board", params, timeout=600)


def opened(app) -> dict:
    return wait_for(lambda: (lambda r: r if r["open_micros"] is not None else None)(rings(app)),
                    timeout=60, interval=0.02, description="the board's opening timed")


def test_board_timings_stay_under_their_ceilings(workspace: Path) -> None:
    artifacts = artifact_directory(f"board-perf-{MODE}")
    measured: dict = {"mode": MODE, "renderer": RENDERER, "shapes": SHAPES}
    try:
        with board_app(workspace, f"board-perf-{MODE}-app") as app:
            # Opening, from the note operation to the first redraw after the
            # board's first painted draw.
            notes = {row["path"]: row["id"] for row in app.client.call("session.get")["notes"]}
            for name, shapes in OPENS.items():
                samples = []
                for _ in range(3):
                    open_row(app, notes[f"{GROUP}/Note.md"])
                    open_row(app, notes[f"{GROUP}/Open {name}.wren-board"])
                    assert board(app)["surface"] == "board"
                    samples.append(opened(app)["open_micros"])
                state = board(app)
                assert state["counts"]["nodes"] + state["counts"]["edges"] == grid_elements(shapes)
                measured[f"open_{name}_micros"] = samples
            assert median_ms(measured["open_1k_micros"]) <= OPEN_1K, measured
            assert median_ms(measured["open_10k_micros"]) <= OPEN_10K, measured

            open_row(app, notes[f"{GROUP}/Grid.wren-board"])
            opened(app)
            state = board(app)
            assert state["counts"]["nodes"] == SHAPES

            # Idle: no scene is built and the view is cheap.
            time.sleep(0.3)
            rings(app, clear=True)
            app.client.call("perf.view", {"clear": True})
            time.sleep(1.0)
            for _ in range(30):
                app.client.call("identify")
            idle = rings(app)
            views = app.client.call("perf.view", {"clear": True})["view_micros"]
            measured["idle"] = {"scene_micros": idle["scene_micros"], "view_micros": views}
            assert idle["scene_micros"] == [], "an idle board built its scene"
            assert len(views) >= 10 and median_ms(views) <= VIEW, views

            # One change of the whole scene, a theme switch, and the draw that
            # paints it.
            edits = []
            for theme in ("light", "dark", "light", "dark", "light"):
                rings(app, clear=True)
                app.client.call("theme.set", {"id": theme})
                time.sleep(0.4)
                after = rings(app)
                assert len(after["scene_micros"]) == 1, after["scene_micros"]
                edits.append(after["scene_micros"][0] + max(after["draw_micros"]))
            measured["edit_micros"] = edits
            assert median_ms(edits) <= EDIT, edits

            # Pan and zoom, one step a redraw.
            for name, run in (("pan", {"pan": {"steps": 60, "dx": 8, "dy": 0}}),
                              ("zoom", {"zoom": {"steps": 30, "factor": 1.05}})):
                app.client.call("command.run", {"id": "font.reset"})
                time.sleep(0.3)
                rings(app, clear=True)
                result = rings(app, **run)
                steps = next(iter(run.values()))["steps"]
                measured[name] = {"frame_micros": result["frame_micros"],
                                  "draw_micros": result["draw_micros"][-steps:]}
                assert result["scene_micros"] == [], f"a {name} built the scene"
                if RELEASE and RENDERER == "wgpu":
                    assert median_ms(measured[name]["draw_micros"]) <= DRAW, measured[name]
                else:
                    assert median_ms(result["frame_micros"]) <= FRAME, measured[name]
            assert app.client.call("navigation.get")["selected_note"] == notes[f"{GROUP}/Grid.wren-board"]
    finally:
        (artifacts / "measurements.json").write_text(json.dumps(measured, indent=2), encoding="utf-8")
