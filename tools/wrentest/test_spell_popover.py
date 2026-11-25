"""The suggestions menu as the editor's anchored popover, in a real window.

The editor tells the app where a popover's anchor is in the window, which
only a laid out window can show: the menu must hang under the rectangle
reported for its word.
"""

from __future__ import annotations

from pathlib import Path

from tools.wrentest.test_spell import (
    MARKS, caret, drawn_menu, editor, key, lines, mark, menu, spell_app, suggest, wait_marks,
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
            # A frame draws the new menu after it opens; until then the
            # bounds reported may still be the last menu's.
            state = wait_for(
                lambda: (s := drawn_menu(app)) and s["word"] == word
                and s["bounds"]["y"] > s["anchor"]["y"] and s,
                description=f"the menu for {word!r} is drawn")
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


def test_a_right_click_asks_the_providers_and_the_suggestions_menu_takes_a_word(
        tmp_path: Path) -> None:
    """A right-click goes to the editor's menu providers in order. The
    suggestions menu is the one an editor has: it takes a misspelled word,
    and a click anywhere else is no one's."""
    with spell_app(tmp_path, "spell-popover-right-click") as (app, _root):
        wait_marks(app, MARKS, "the acceptance note's nine marks")
        line, word = 13, "mispelled"
        start = mark(app, line, word)["start"]
        before = editor(app)
        app.client.call("editor.right_click", {"line": line, "column": start + 2})
        state = drawn_menu(app)
        assert (state["line"], state["word"]) == (line, word), state
        # It hangs where the pointer's own right-click opens it, and the
        # click moves neither the caret nor the text.
        anchor, bounds = state["anchor"], state["bounds"]
        assert bounds["x"] == round(anchor["x"]), (anchor, bounds)
        assert bounds["y"] == round(anchor["y"] + anchor["height"] + MENU_GAP), (anchor, bounds)
        after = editor(app)
        assert (after["text"], after["cursor"]) == (before["text"], before["cursor"])
        # With the menu open, a right-click in the text only closes it: on
        # a word spelled right, and on the misspelled word itself.
        text = lines(app)[line]
        for column in (text.index("done"), start + 2):
            app.client.call("editor.right_click", {"line": line, "column": column})
            assert menu(app) is None, column
            app.client.call("editor.right_click", {"line": line, "column": start + 2})
            assert drawn_menu(app)["word"] == word, column
        key(app, "escape")
        wait_for(lambda: menu(app) is None, description="escape closes the menu")
        # A word spelled right, the task's box, and the end of the line.
        for column in (text.index("done"), text.index("["), len(text)):
            app.client.call("editor.right_click", {"line": line, "column": column})
            assert menu(app) is None, column
        assert editor(app)["text"] == before["text"]


def test_a_right_click_opens_no_menu_on_the_classic_engine(tmp_path: Path) -> None:
    with spell_app(tmp_path, "spell-popover-right-click-classic",
                   settings="editor-engine = classic\nspell-check = on\n") as (app, _root):
        assert editor(app)["engine"] == "classic"
        text = lines(app)[13]
        app.client.call("editor.right_click", {"line": 13, "column": text.index("mispelled") + 2})
        assert menu(app) is None
