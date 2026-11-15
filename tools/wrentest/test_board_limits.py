"""The limits for untrusted board files in the app, with the `boards` setting
on (discovery/boards.md §3.8, §4.12, §6.8 Phase 3 item 7, §7.4 H1).

A board over 8 MiB or 50,000 elements opens as raw text and is searched as
written; one at the element cap opens as a board. A value past one of the
limits the scene draws within is listed as an issue, drawn clamped or cut,
and never written. An edit that would take a board past a cap is refused.
"""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from tools.wrentest.boards import board, board_app, click, grid_board, grid_elements, stamp
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import wait_for

GROUP = "Boards/Limits"
LARGE = f"{GROUP}/Large.wren-board"
CROWDED = f"{GROUP}/Crowded.wren-board"
FAR = f"{GROUP}/Far.wren-board"
CAP = f"{GROUP}/At the cap.wren-board"
FULL = f"{GROUP}/Full.wren-board"
NOTE = f"{GROUP}/Note.md"

MAX_BOARD_BYTES = 8 * 1024 * 1024
MAX_ELEMENTS = 50_000
COORDINATE_LIMIT = 1 << 24
MAX_LABEL_LINES = 1024
# Opening a board at the element cap, in a release build (§7.4 H1): over
# this, the cap is lowered.
OPEN_AT_THE_CAP_MS = 3000
RELEASE = os.environ.get("WRENTEST_PERF", "debug") == "release"


def large_board() -> str:
    """One byte over the size a board may have, in comment lines: the raw
    text editor shapes each line whole, so no line is longer than a
    sentence."""
    head = 'schema = 1\n\n[node.a]\ntext = "Large"\nkind = "shape"\nat = [0, 0]\n' \
           'size = [120, 60]\nz = "V"\n'
    line = "# " + "x" * 77 + "\n"
    rest = MAX_BOARD_BYTES + 1 - len(head)
    return head + line * (rest // len(line)) + "#" * (rest % len(line))


def node(id_: str, fields: str, text: str = "") -> str:
    label = f'text = "{text}"\n' if text else ""
    return f'\n[node.{id_}]\n{label}kind = "shape"\n{fields}'


def crowded_board(count: int) -> str:
    """``count`` small shapes in a row, unlabelled."""
    return "schema = 1\n" + "".join(
        node(f"n{i:05}", f'at = [{i * 4}, 0]\nsize = [2, 2]\nz = "V"\n') for i in range(count))


def far_board() -> str:
    """A value past each limit the scene draws within, and a chain one level
    deeper than a board draws inside its parents. Canonical."""
    parts = ["schema = 1\n"]
    parts.append(node("chain01", 'at = [0, 400]\nsize = [40, 40]\nz = "V"\n'))
    for level in range(2, 34):
        parts.append(node(f"chain{level:02}", f'at = [1, 1]\nparent = "chain{level - 1:02}"\n'
                                              f'size = [40, 40]\nz = "V"\n'))
    parts.append(node("far", f'at = [{COORDINATE_LIMIT}, 0]\nsize = [120, 60]\nz = "V"\n',
                      "Far away"))
    parts.append(node("flat", 'at = [200, 0]\nsize = [120, 0]\nz = "V"\n', "Flat"))
    lines = "\\n".join(["line"] * (MAX_LABEL_LINES + 1))
    parts.append(node("tall", 'at = [400, 0]\nsize = [120, 60]\nz = "V"\n', lines))
    parts.append(node("ok", 'at = [0, 0]\nsize = [120, 60]\nz = "V"\n', "Within"))
    labels = "".join(f'  {{ text = "l{i}" }},\n' for i in range(17))
    parts.append(
        '\n[edge.anchored]\nkind = "edge"\nfrom = { anchor = [2000, 500], node = "ok" }\n'
        'to = { node = "flat" }\nlabels = [\n  { at = 5000, text = "past the end" },\n]\n'
        'z = "W"\n')
    parts.append(
        f'\n[edge.many]\nkind = "edge"\nfrom = {{ node = "ok" }}\nto = {{ node = "tall" }}\n'
        f'labels = [\n{labels}]\nz = "{"W" * 65}"\n')
    return "".join(parts)


def cap_board() -> str:
    """The grid of the performance cases, topped up with unlabelled shapes
    below it to exactly the element cap."""
    shapes = 1
    while grid_elements(shapes + 1) <= MAX_ELEMENTS:
        shapes += 1
    text = grid_board(shapes)
    extra = MAX_ELEMENTS - grid_elements(shapes)
    return text + "".join(node(f"x{i:05}", f'at = [{i * 4}, -400]\nsize = [2, 2]\nz = "0"\n')
                          for i in range(extra))


def full_board() -> str:
    """Exactly 8 MiB, canonical: two labelled shapes, and unknown content
    that fills the rest on one line the raw text editor never shows."""
    head = ('schema = 1\npad = ""\n\n[node.a]\ntext = "First"\nkind = "shape"\nat = [0, 0]\n'
            'size = [120, 60]\nz = "V"\n\n[node.b]\ntext = "Second"\nkind = "shape"\n'
            'at = [200, 0]\nsize = [120, 60]\nz = "W"\n')
    return head.replace('pad = ""', f'pad = "{"x" * (MAX_BOARD_BYTES - len(head))}"', 1)


@pytest.fixture
def workspace(tmp_path: Path) -> Path:
    group = tmp_path / "work" / GROUP
    group.mkdir(parents=True)
    for path, text in ((LARGE, large_board()), (CROWDED, crowded_board(MAX_ELEMENTS + 1)),
                       (FAR, far_board()), (CAP, cap_board()), (FULL, full_board())):
        (tmp_path / "work" / path).write_text(text, encoding="utf-8")
    (tmp_path / "work" / NOTE).write_text("# Note\n", encoding="utf-8")
    return tmp_path / "work"


def notes(app) -> dict[str, int]:
    return {row["path"]: row["id"] for row in app.client.call("session.get")["notes"]}


def open_row(app, note: int) -> None:
    """Open a note as a click on its row does: `core.apply` would answer
    with the Session's text, over the harness's 1 MiB frame here."""
    app.client.call("row.click", {"row": {"note": note}})


def test_a_board_over_a_cap_opens_as_text_and_is_never_written(workspace: Path) -> None:
    """§6.8 Phase 3 item 7: an oversized board opens as raw text, with the
    reason at line 1, is searched as written, and keeps its bytes."""
    before = {path: stamp(workspace / path) for path in (LARGE, CROWDED)}
    with board_app(workspace, "board-limits-raw") as app:
        rows = notes(app)
        for path, reason in ((LARGE, "8,388,609 bytes, over the 8,388,608 (8 MiB)"),
                             (CROWDED, "50,001 nodes and connectors, over the 50,000")):
            open_row(app, rows[NOTE])
            open_row(app, rows[path])
            state = wait_for(lambda: (lambda s: s if s["surface"] == "raw" else None)(board(app)),
                             timeout=30, interval=0.1, description=f"{path} opened as text")
            assert state["counts"] is None, state
            assert state["error"]["line"] == 1, state
            assert reason in state["error"]["message"], state
        # Searched as written: its syntax is found, as no board's is.
        app.client.call("palette.open", {"mode": "search"})
        modal = app.client.call("palette.query", {"query": "schema"})["navigation"]["modal"]
        found = sorted(row["path"] for row in modal["results"]
                       if row["path"].endswith(".wren-board"))
        assert found == sorted([LARGE, CROWDED]), found
        app.client.call("palette.close")
    for path, was in before.items():
        assert stamp(workspace / path) == was, f"{path} was written"


def test_values_past_a_limit_are_flagged_and_never_written(workspace: Path) -> None:
    """§6.8 Phase 3 item 7: out-of-range coordinates and over-deep parents
    are flagged, as is every other value past a limit, and the board is
    left as it was."""
    before = stamp(workspace / FAR)
    with board_app(workspace, "board-limits-flagged") as app:
        open_row(app, notes(app)[FAR])
        state = wait_for(lambda: (lambda s: s if s["surface"] == "board" else None)(board(app)),
                         timeout=10, interval=0.05, description="the board shown")
        issues = [(issue["id"], issue["kind"], issue["limit"]) for issue in state["issues"]]
        assert issues == [
            ("chain33", "too_deep", None),
            ("far", "out_of_range", "coordinates"),
            ("flat", "out_of_range", "size"),
            ("tall", "out_of_range", "label_lines"),
            ("anchored", "out_of_range", "anchor"),
            ("anchored", "out_of_range", "label_at"),
            ("many", "out_of_range", "labels"),
            ("many", "out_of_range", "z_length"),
        ], issues
        assert app.client.call("session.get")["document"]["dirty"] is False
        app.client.call("window.blur")
    assert stamp(workspace / FAR) == before


def test_a_board_at_the_element_cap_opens_as_a_board(workspace: Path) -> None:
    """At 50,000 elements a board is drawn; in a release build it opens
    within 3 s, or the cap is lowered (§7.4 H1)."""
    text = (workspace / CAP).read_text(encoding="utf-8")
    assert text.count("\n[node.") + text.count("\n[edge.") == MAX_ELEMENTS
    measured: dict = {"mode": "release" if RELEASE else "debug",
                      "renderer": os.environ.get("ICED_BACKEND", ""),
                      "bytes": len(text.encode("utf-8")), "load_average": os.getloadavg()}
    with board_app(workspace, "board-limits-cap") as app:
        rows = notes(app)
        samples = []
        for _ in range(3 if RELEASE else 1):
            open_row(app, rows[NOTE])
            open_row(app, rows[CAP])
            rings = wait_for(
                lambda: (lambda r: r if r["open_micros"] is not None else None)(
                    app.client.call("perf.board", {}, timeout=600)),
                timeout=300, interval=0.1, description="the board at the cap opened")
            samples.append(rings["open_micros"])
            state = board(app)
            assert state["surface"] == "board", state
            assert state["counts"]["nodes"] + state["counts"]["edges"] == MAX_ELEMENTS, state
        measured["open_micros"] = samples
    artifacts = artifact_directory("board-limits-cap-measured")
    (artifacts / "measurements.json").write_text(json.dumps(measured, indent=2) + "\n",
                                                 encoding="utf-8")
    if RELEASE:
        median = sorted(samples)[len(samples) // 2] / 1000
        assert median <= OPEN_AT_THE_CAP_MS, measured


def test_an_edit_past_the_byte_cap_is_refused_and_the_board_still_opens(workspace: Path) -> None:
    """The phase-end review's blocker: on a board at the byte cap, a recolour,
    a duplicate, Tab and a paste are refused with a banner, nothing is
    written, and after a restart the board still opens as a board."""
    before = stamp(workspace / FULL)
    assert len(before[0]) == MAX_BOARD_BYTES
    with board_app(workspace, "board-limits-full") as app:
        rows = notes(app)
        open_row(app, rows[FULL])
        wait_for(lambda: board(app)["surface"] == "board", timeout=30, interval=0.1,
                 description="the board at the cap shown")
        app.client.call("test.clipboard", {"own": True})
        app.client.call("editor.focus")
        for action in ("board.color.red", "board.duplicate", "board.connect.next", "board.copy",
                       "board.paste"):
            click(app, {"node": "a"})
            app.client.call("command.run", {"id": action})
            state = board(app)
            assert state["counts"] == {"nodes": 2, "edges": 0}, (action, state)
            assert state["can_undo"] is False, (action, state)
            if action != "board.copy":
                banner = wait_for(lambda: board(app)["banner"], timeout=5,
                                  interval=0.02, description=f"the banner after {action}")
                assert "Cannot make this change: the board would be" in banner, (action, banner)
        # Leaving the board flushes whatever it drafted.
        open_row(app, rows[NOTE])
    assert stamp(workspace / FULL) == before
    with board_app(workspace, "board-limits-full-again") as app:
        open_row(app, notes(app)[FULL])
        state = wait_for(lambda: (lambda s: s if s["surface"] in ("board", "raw") else None)(
            board(app)), timeout=30, interval=0.1, description="the board reopened")
        assert state["surface"] == "board", state
