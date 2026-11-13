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

from tools.wrentest.boards import (ROUTED, board, board_app, drag, grid_board, grid_elements,
                                   routed_board)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import wait_for

MODE = os.environ.get("WRENTEST_PERF", "debug")
if MODE not in ("debug", "release"):
    raise pytest.UsageError(f"WRENTEST_PERF must be debug or release, got {MODE!r}")
RELEASE = MODE == "release"
RENDERER = os.environ.get("ICED_BACKEND", "")

# Milliseconds. Release: the spike's budgets as §8.5 records them, the edit
# budget replaced by the canvas chain's measured 12 ms with the whole board
# in view, and on wgpu by 16 ms: a quarter over the worst median measured on
# lavapipe on a quiet machine, whose threads share the processor with the
# scene build, so that a doubled scene build still fails (§6.7). Debug:
# §6.7's ceilings.
EDIT = (16 if RENDERER == "wgpu" else 12) if RELEASE else 100
FRAME = 33 if RELEASE else 400
DRAW = 8
OPEN_1K = 150 if RELEASE else 5000
OPEN_10K = 1000 if RELEASE else 5000
VIEW = 1 if RELEASE else 5
# One label keystroke's draft at 1,000 nodes, the canonical write and the
# Session's op with its index parse (§3.4 rule 7): release only.
KEYSTROKE = 8

# Whole-scene changes left out before the timed ones, and those timed.
WARM = 3
EDITS = 7

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
    for index, kind in enumerate(ROUTED):
        (group / f"Routed {index}.wren-board").write_text(routed_board(kind, SHAPES),
                                                          encoding="utf-8")
    (group / "Note.md").write_text("# Note\n", encoding="utf-8")
    return tmp_path / "work"


def open_row(app, note: int) -> None:
    """Open a note as a click on its row does. `core.apply` would answer
    with the Session's text, over the harness's 1 MiB frame for the largest
    board."""
    app.client.call("row.click", {"row": {"note": note}})


def rings(app, **params) -> dict:
    return app.client.call("perf.board", params, timeout=600)


def painted(app, built: int) -> dict:
    """The rings once a draw has built the geometry of a changed scene. A
    request is answered between draws, never during one, so that draw's
    own time is in the ring by then. Polled without a pause, which also
    keeps the app's core busy."""
    return wait_for(lambda: (lambda r: r if r["builds"] > built else None)(rings(app)),
                    timeout=10, interval=0.001, description="the changed scene drawn")


def opened(app) -> dict:
    return wait_for(lambda: (lambda r: r if r["open_micros"] is not None else None)(rings(app)),
                    timeout=60, interval=0.02, description="the board's opening timed")


def test_board_timings_stay_under_their_ceilings(workspace: Path) -> None:
    artifacts = artifact_directory(f"board-perf-{MODE}")
    # The release budgets hold on a quiet machine only; the load tells a run
    # made beside other work from a regression.
    measured: dict = {"mode": MODE, "renderer": RENDERER, "shapes": SHAPES,
                      "load_average": os.getloadavg()}
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

            # Idle (exit criterion 9): over a second, forty ticks, nothing is
            # parsed, no scene and no geometry is built, and the view is cheap.
            time.sleep(0.3)
            before = rings(app, clear=True)
            app.client.call("perf.view", {"clear": True})
            time.sleep(1.0)
            for _ in range(30):
                app.client.call("identify")
            idle = rings(app)
            views = app.client.call("perf.view", {"clear": True})["view_micros"]
            counts = {key: idle[key] - before[key] for key in ("parses", "scenes", "builds")}
            measured["idle"] = {"scene_micros": idle["scene_micros"], "view_micros": views,
                                "counts": counts}
            assert idle["scene_micros"] == [], "an idle board built its scene"
            assert counts == {"parses": 0, "scenes": 0, "builds": 0}, counts
            assert len(views) >= 10 and median_ms(views) <= VIEW, views

            # One change of the whole scene, a theme switch, and the draw that
            # paints it, timed back to back: the first few changes are left
            # out, so each counted one runs on a core already busy. Taken
            # after an idle pause, the same build measured twice as long on
            # a core that had dropped to its idle clock (§6.7).
            scenes, draws, warm = [], [], []
            for change in range(WARM + EDITS):
                before = rings(app, clear=True)
                app.client.call("theme.set", {"id": ("light", "dark")[change % 2]})
                after = painted(app, before["builds"])
                assert len(after["scene_micros"]) == 1, after["scene_micros"]
                sample = (after["scene_micros"][0], max(after["draw_micros"]))
                if change < WARM:
                    warm.append(sum(sample))
                else:
                    scenes.append(sample[0])
                    draws.append(sample[1])
            edits = [scene + draw for scene, draw in zip(scenes, draws)]
            measured.update(edit_micros=edits, edit_scene_micros=scenes,
                            edit_draw_micros=draws, edit_warm_up_micros=warm)
            assert median_ms(edits) <= EDIT, measured

            # Pan and zoom, one step a redraw.
            for name, run in (("pan", {"pan": {"steps": 60, "dx": 8, "dy": 0}}),
                              ("zoom", {"zoom": {"steps": 30, "factor": 1.05}})):
                app.client.call("command.run", {"id": "font.reset"})
                time.sleep(0.3)
                start = rings(app, clear=True)
                result = rings(app, **run)
                steps = next(iter(run.values()))["steps"]
                measured[name] = {"frame_micros": result["frame_micros"],
                                  "draw_micros": result["draw_micros"][-steps:]}
                assert result["scene_micros"] == [], f"a {name} built the scene"
                # The geometry counter the idle check reads is live: moving
                # the view rebuilds geometry.
                assert result["builds"] > start["builds"], (start, result)
                if RELEASE and RENDERER == "wgpu":
                    assert median_ms(measured[name]["draw_micros"]) <= DRAW, measured[name]
                else:
                    assert median_ms(result["frame_micros"]) <= FRAME, measured[name]
            assert app.client.call("navigation.get")["selected_note"] == notes[f"{GROUP}/Grid.wren-board"]

            # One label keystroke at 1,000 nodes (§3.4 rule 7): each typed
            # character is one draft, the canonical write of the board and
            # the Session's op, which re-indexes the document. The scene
            # the change rebuilds is recorded beside it.
            app.client.call("command.run", {"id": "font.reset"})
            app.client.call("board.pointer", {"kind": "click", "button": "left", "at": {"node": "n0"}})
            wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                     timeout=5, interval=0.02, description="the board focused")
            assert app.client.call("board.key", {"key": "enter"})["state"] == "editing"
            rings(app, clear=True)
            typed = "keystroke timing"
            for character in typed:
                assert app.client.call("board.type", {"text": character})["handled"]
            keys = rings(app)
            measured["keystroke"] = {"draft_micros": keys["draft_micros"],
                                     "write_micros": keys["write_micros"],
                                     "scene_micros": keys["scene_micros"]}
            assert len(keys["draft_micros"]) == len(typed), measured["keystroke"]
            app.client.call("board.key", {"key": "escape"})
            if RELEASE:
                assert median_ms(keys["draft_micros"]) <= KEYSTROKE, measured["keystroke"]

            # On the boards whose every connector spends its whole routing
            # budget, a drag's preview, rebuilt once a pointer move, and a
            # label keystroke each build the whole scene (§7.3 attack 11).
            # Recorded, not asserted: the frame ceilings above are the
            # grid's, and these miss them (§8.5, boards-p2-wire).
            measured["routed"] = {}
            for index, kind in enumerate(ROUTED):
                open_row(app, notes[f"{GROUP}/Routed {index}.wren-board"])
                opened(app)
                rings(app, clear=True)
                # Far enough at the fitted zoom that each move is past the
                # 4 px a drag starts at.
                drag(app, {"node": "n1"}, (800, 400))
                dragged = rings(app)
                wait_for(lambda: app.client.call("focus.get")["named"] == ["note-editor"],
                         timeout=5, interval=0.02, description="the board focused")
                assert app.client.call("board.key", {"key": "enter"})["state"] == "editing"
                rings(app, clear=True)
                for character in "keys":
                    assert app.client.call("board.type", {"text": character})["handled"]
                typed = rings(app)
                app.client.call("board.key", {"key": "escape"})
                measured["routed"][kind] = {
                    "drag_scene_micros": dragged["scene_micros"],
                    "keystroke_scene_micros": typed["scene_micros"],
                    "keystroke_draft_micros": typed["draft_micros"]}
                # Four preview steps and the release's own build.
                assert len(dragged["scene_micros"]) == 5, measured["routed"][kind]
                assert len(typed["draft_micros"]) == 4, measured["routed"][kind]
    finally:
        (artifacts / "measurements.json").write_text(json.dumps(measured, indent=2), encoding="utf-8")
