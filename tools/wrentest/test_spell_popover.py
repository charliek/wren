"""The suggestions menu as the editor's anchored popover, in a real window.

The editor tells the app where a popover's anchor is in the window, which
only a laid out window can show: the menu must hang under the rectangle
reported for its word.
"""

from __future__ import annotations

from pathlib import Path

from tools.wrentest.test_spell import (
    MARKS, caret, drawn_menu, key, mark, menu, spell_app, suggest, wait_marks,
)
from tools.wrentest.ui import wait_for

# The menu hangs 4 px under its anchor's row (rule 4.4.3).
MENU_GAP = 4


def test_the_menu_hangs_under_the_anchor_the_app_is_told(tmp_path: Path) -> None:
    with spell_app(tmp_path, "spell-popover-anchor") as (app, _root):
        wait_marks(app, MARKS, "the acceptance note's nine marks")
        for line, word in [(3, "chek"), (13, "mispelled")]:
            caret(app, line, mark(app, line, word)["start"] + 1)
            app.client.call("editor.focus")
            suggest(app)
            state = drawn_menu(app)
            assert state["word"] == word, state
            anchor, bounds = state["anchor"], state["bounds"]
            # The anchor is the word's glyphs on their row: as wide as the
            # word's zigzag, which lies inside the row.
            zigzag = mark(app, line, word)["rects"][0]
            assert abs(anchor["x"] - zigzag["x"]) <= 1.5, (anchor, zigzag)
            assert abs(anchor["width"] - zigzag["width"]) <= 2.5, (anchor, zigzag)
            assert anchor["y"] <= zigzag["y"] < anchor["y"] + anchor["height"], (anchor, zigzag)
            # And the menu hangs under it, at its left edge.
            assert bounds["x"] == round(anchor["x"]), (anchor, bounds)
            assert bounds["y"] == round(anchor["y"] + anchor["height"] + MENU_GAP), (anchor, bounds)
            key(app, "escape")
            wait_for(lambda: menu(app) is None, description="escape closes the menu")
