"""Spell check through the real app.

The acceptance script of ``discovery/spell-check.md`` §5.5 on the advanced
engine, over the committed acceptance note: steps 1 to 4 and 6 to 8 drive the
app through its requests (``editor.spell``, ``editor.spell_event``,
``editor.key``, ``shortcut``), so they run on Wayland too; the ``native_x11``
cases press real keys and click with the real pointer. Every case sets
``spell-check`` itself except the one that checks the lane's setting.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
import time
from pathlib import Path

import pytest

from tools.wrentest import test_gestures as gestures
from tools.wrentest.test_editor import focus_native, xdo
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, spell_setting, wait_for

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "spell.md"
NOTE = "Notes/Inbox/Spell chek plan.md"
OTHER = "Notes/Inbox/Other.md"
ADVANCED = "editor-engine = advanced\nspell-check = on\n"
WORDS = "catppuccin\n"
# The acceptance note's marks with `catppuccin` in the word list, in order.
MARKS = [
    (3, "chek"), (5, "apear"), (6, "replacments"), (8, "bundeled"), (9, "desing"),
    (10, "CommonMark"), (13, "mispelled"), (14, "knwon"), (16, "misteak"),
]
# A menu row is 30 px under 4 px of padding (rule 4.4.3).
MENU_PADDING = 4
MENU_ROW = 30


def original() -> str:
    return FIXTURE.read_text(encoding="utf-8")


def spell(app: WrenApp) -> dict:
    return app.client.call("editor.spell")


def marks(app: WrenApp) -> list[tuple[int, str]]:
    return [(mark["line"], mark["word"]) for mark in spell(app)["marks"]]


def wait_marks(app: WrenApp, expected: list[tuple[int, str]], description: str) -> None:
    wait_for(lambda: marks(app) == expected, description=description)


def menu(app: WrenApp) -> dict | None:
    return spell(app)["menu"]


def drawn_menu(app: WrenApp) -> dict:
    """The open menu once a frame has drawn it."""
    return wait_for(lambda: (state := menu(app)) and state["bounds"] and state,
                    description="the menu is drawn")


def mark(app: WrenApp, line: int, word: str) -> dict:
    found = [entry for entry in spell(app)["marks"]
             if entry["line"] == line and entry["word"] == word]
    assert len(found) == 1, (line, word, spell(app)["marks"])
    return found[0]


def editor(app: WrenApp) -> dict:
    return app.client.call("editor.get")


def lines(app: WrenApp) -> list[str]:
    return editor(app)["text"].split("\n")


def caret(app: WrenApp, line: int, column: int) -> None:
    app.client.call("editor.cursor", {"line": line, "column": column, "selection": None})


def key(app: WrenApp, name: str, *mods: str) -> dict:
    return app.client.call("editor.key", {"key": name, "mods": list(mods)})


def suggest(app: WrenApp) -> None:
    """The suggest key firing, as the focused editor's key policy routes it."""
    app.client.call("shortcut", {"id": "editor.spell_suggest"})


def command(app: WrenApp, command_id: str) -> dict:
    result = app.client.call("command.run", {"id": command_id})
    assert result["disposition"] == "completed", result
    return result


def focused(app: WrenApp) -> list[str]:
    return app.client.call("focus.get")["named"]


def open_note(app: WrenApp, path: str) -> int:
    note = next(entry["id"] for entry in app.client.call("session.get")["notes"]
                if entry["path"] == path)
    app.client.call("core.apply", {"op": "open_note", "note": note})
    wait_for(lambda: (app.client.call("session.get")["document"] or {}).get("note") == note,
             description=f"open {path}")
    return note


def drawn_frame(app: WrenApp, name: str) -> None:
    """Wait for a frame drawn after every change so far."""
    app.client.call("app.screenshot", {"path": str(artifact_directory(name) / "frame.png")})


@contextmanager
def spell_app(tmp_path: Path, name: str, *, settings: str = ADVANCED,
              words: str | None = WORDS) -> Iterator[tuple[WrenApp, Path]]:
    """The acceptance note and one other note, the first open, the
    dictionary loaded."""
    root = tmp_path / "workspace"
    (root / "Notes/Inbox").mkdir(parents=True)
    (root / NOTE).write_text(original(), encoding="utf-8")
    (root / OTHER).write_text("Another note.\n", encoding="utf-8")
    with WrenApp(workspace=root, settings=settings, words=words,
                 artifacts_dir=artifact_directory(name)) as app:
        wait_for(lambda: spell(app)["ready"], description="the dictionary loads")
        open_note(app, NOTE)
        assert editor(app)["text"] == original()
        yield app, root


def test_the_acceptance_note_is_the_plans() -> None:
    text = original()
    assert "Catppuccin’s" in text and "doesn’t" in text and "don't" in text
    lines = text.split("\n")
    for line, word in MARKS:
        assert word in lines[line], (line, word)


def test_acceptance_marks_menu_and_accept(tmp_path: Path) -> None:
    with spell_app(tmp_path, "spell-acceptance") as (app, root):
        # 1. Exactly the nine marks, at the words' own bytes.
        wait_marks(app, MARKS, "the acceptance note's nine marks")
        state = spell(app)
        assert state["enabled"] is True and state["menu"] is None
        source = original().split("\n")
        for entry in state["marks"]:
            assert source[entry["line"]].encode()[entry["start"]:entry["end"]].decode() == entry["word"]
            assert entry["shown"] is True and entry["rects"], entry

        # 2. A word typed at the end of a line is not marked until it ends.
        quote = 16
        caret(app, quote, len(source[quote]))
        app.client.call("editor.focus")
        app.client.call("editor.insert", {"text": " wrod"})
        wait_for(lambda: (quote, "wrod") in marks(app)
                 and mark(app, quote, "wrod")["shown"] is False,
                 description="the mark at the caret is hidden")
        app.client.call("editor.insert", {"text": " "})
        wait_for(lambda: mark(app, quote, "wrod")["shown"] is True,
                 description="a space shows the mark")
        for _ in " wrod ":
            app.client.call("editor.action", {"kind": "backspace"})
        assert editor(app)["text"] == original()
        wait_marks(app, MARKS, "the typed word is gone")

        # 3. The suggest key opens the menu on the word at the caret; Down,
        # Up and Escape change nothing and leave the editor the keyboard.
        heading = 3
        caret(app, heading, 9)
        app.client.call("editor.focus")
        wait_for(lambda: "note-editor" in focused(app), description="the editor is focused")
        before = editor(app)
        suggest(app)
        opened = drawn_menu(app)
        assert opened["word"] == "chek"
        assert opened["items"][0] == "cheek" and "check" in opened["items"], opened
        assert opened["selected"] == 0
        assert (opened["line"], opened["start"], opened["end"]) == (heading, 8, 12)
        word = mark(app, heading, "chek")["rects"][0]
        assert opened["bounds"]["y"] >= word["y"], (opened, word)
        assert editor(app)["cursor"] == before["cursor"] and editor(app)["selection"] is None
        key(app, "down")
        assert menu(app)["selected"] == 1
        key(app, "up")
        assert menu(app)["selected"] == 0
        key(app, "escape")
        assert menu(app) is None
        after = editor(app)
        assert after["text"] == original() and after["cursor"] == before["cursor"]
        assert "note-editor" in focused(app)

        # 4. Enter accepts the first row as one undoable edit, and it saves.
        suggest(app)
        assert menu(app)["items"][0] == "cheek"
        key(app, "enter")
        accepted = original().replace("# Spell chek plan", "# Spell cheek plan")
        state = editor(app)
        assert state["text"] == accepted
        assert state["cursor"] == {"line": heading, "column": 13}
        assert menu(app) is None
        wait_marks(app, MARKS[1:], "the accepted word's mark is gone")
        wait_for(lambda: (root / NOTE).read_text(encoding="utf-8") == accepted,
                 description="the accepted word autosaves")
        assert app.client.call("editor.undo")["text"] == original()
        wait_marks(app, MARKS, "undo brings the mark back")
        wait_for(lambda: (root / NOTE).read_text(encoding="utf-8") == original(),
                 description="the undo autosaves")


def test_acceptance_toggle_word_list_and_engine(tmp_path: Path) -> None:
    with spell_app(tmp_path, "spell-settings") as (app, _root):
        wait_marks(app, MARKS, "the acceptance note's nine marks")

        # 6. The toggle hides every mark, saves, and the key does nothing.
        command(app, "editor.toggle_spell_check")
        wait_marks(app, [], "no marks with spell check off")
        assert spell(app)["enabled"] is False
        assert "spell-check = off\n" in app.settings_path.read_text(encoding="utf-8")
        caret(app, 3, 9)
        app.client.call("editor.focus")
        wait_for(lambda: "note-editor" in focused(app), description="the editor is focused")
        suggest(app)
        assert menu(app) is None
        blocked = app.client.call("command.run", {"id": "editor.spell_suggest"})
        assert blocked["disposition"] == "blocked", blocked
        assert menu(app) is None
        command(app, "editor.toggle_spell_check")
        wait_marks(app, MARKS, "the marks return")
        assert "spell-check = on\n" in app.settings_path.read_text(encoding="utf-8")

        # 7. A word added to words.txt is no longer marked after a reload.
        with app.words_path.open("a", encoding="utf-8") as words:
            words.write("misteak\n")
        command(app, "settings.reload")
        wait_marks(app, MARKS[:-1], "the listed word's mark is gone")

        # 8. The classic engine shows no marks and opens no menu.
        command(app, "editor.toggle_engine")
        assert editor(app)["engine"] == "classic"
        wait_marks(app, [], "no marks on the classic engine")
        assert spell(app)["enabled"] is False
        caret(app, 5, 10)
        app.client.call("editor.spell_event", {"event": "open_at_caret"})
        assert menu(app) is None


def test_the_classic_engine_shows_nothing(tmp_path: Path) -> None:
    with spell_app(tmp_path, "spell-classic",
                   settings="editor-engine = classic\nspell-check = on\n") as (app, _root):
        assert editor(app)["engine"] == "classic"
        drawn_frame(app, "spell-classic-frame")
        state = spell(app)
        assert state == {"enabled": False, "ready": True, "marks": [], "menu": None}, state
        caret(app, 5, 10)
        app.client.call("editor.focus")
        wait_for(lambda: "note-editor" in focused(app), description="the editor is focused")
        suggest(app)
        app.client.call("editor.spell_event", {"event": "open_at_caret"})
        assert menu(app) is None


def test_the_lanes_setting_decides_when_the_test_names_none(tmp_path: Path) -> None:
    lane = spell_setting()
    with spell_app(tmp_path, "spell-lane", settings="editor-engine = advanced\n") as (app, _root):
        assert f"spell-check = {lane}\n" in app.settings_path.read_text(encoding="utf-8")
        assert spell(app)["enabled"] is (lane == "on")
        if lane == "on":
            wait_marks(app, MARKS, "the lane's marks")
        else:
            drawn_frame(app, "spell-lane-frame")
            assert marks(app) == []
            caret(app, 5, 10)
            app.client.call("editor.spell_event", {"event": "open_at_caret"})
            assert menu(app) is None


def test_a_screen_full_of_marks_draws_without_a_stall(tmp_path: Path) -> None:
    # Drawn as one 1 px quad per pixel, a screen of marks once took tiny-skia
    # 4 to 12 s a frame in this build, and the client gave up at 5 s.
    root = tmp_path / "workspace"
    (root / "Notes/Inbox").mkdir(parents=True)
    filler = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor"
    (root / "Notes/Inbox/Filler.md").write_text(
        "".join(f"- item {n} {filler}\n" for n in range(600)), encoding="utf-8")
    with WrenApp(workspace=root, settings=ADVANCED,
                 artifacts_dir=artifact_directory("spell-many-marks")) as app:
        wait_for(lambda: spell(app)["ready"], description="the dictionary loads")
        open_note(app, "Notes/Inbox/Filler.md")
        wait_for(lambda: sum(mark["shown"] for mark in spell(app)["marks"]) >= 100,
                 description="a screen of marks")
        frames = []
        for page in range(1, 6):
            start = time.monotonic()
            # Each page moves every mark, so the whole screen is drawn again.
            caret(app, page * 100, 0)
            drawn_frame(app, "spell-many-marks")
            frames.append(time.monotonic() - start)
        # A generous ceiling for a debug build on a shared machine; these
        # frames take tens of milliseconds there.
        assert max(frames) < 2.0, frames


def test_menu_keys_close_then_act_and_rows_follow_the_pointer_events(tmp_path: Path) -> None:
    with spell_app(tmp_path, "spell-menu-keys") as (app, _root):
        wait_marks(app, MARKS, "the acceptance note's nine marks")
        line = 9
        caret(app, line, 13)
        app.client.call("editor.focus")
        app.client.call("editor.spell_event", {"event": "open_at_caret"})
        # Drawn first, so the comparison below never sees `bounds` filled in.
        opened = drawn_menu(app)
        assert opened["word"] == "desing" and opened["selected"] == 0
        key(app, "shift")
        assert menu(app) == opened, "a modifier alone does nothing"
        app.client.call("editor.spell_event", {"event": "hover", "arg": 2})
        assert menu(app)["selected"] == 2
        app.client.call("editor.spell_event", {"event": "move", "arg": -1})
        assert menu(app)["selected"] == 1
        # Shift+Down is any other key: it closes the menu, then selects.
        state = key(app, "down", "shift")
        assert menu(app) is None
        assert state["selection"] is not None
        assert state["text"] == original()

        caret(app, line, 13)
        app.client.call("editor.spell_event", {"event": "open_at_caret"})
        app.client.call("editor.spell_event", {"event": "accept", "arg": 1})
        second = opened["items"][1]
        assert lines(app)[line] == original().split("\n")[line].replace("desing", second)
        assert menu(app) is None


def test_a_menu_opened_just_after_a_theme_or_text_size_change_stays_open(
        tmp_path: Path) -> None:
    changes = [
        lambda app: app.client.call("theme.set", {"id": "nord"}),
        lambda app: command(app, "font.increase"),
        lambda app: app.client.call("theme.set", {"id": "wren-dark"}),
    ]
    with spell_app(tmp_path, "spell-menu-after-change") as (app, _root):
        wait_marks(app, MARKS, "the acceptance note's nine marks")
        app.client.call("editor.focus")
        for i, change in enumerate(changes):
            change(app)
            caret(app, 9, 13)
            app.client.call("editor.spell_event", {"event": "open_at_caret"})
            opened = drawn_menu(app)
            # The editor's events after the change see a menu opened since.
            drawn_frame(app, "spell-menu-after-change")
            drawn_frame(app, "spell-menu-after-change")
            assert menu(app) == opened, f"change {i}"
            app.client.call("editor.spell_event", {"event": "close"})


# ── native X11 ────────────────────────────────────────────────────────────


@contextmanager
def native_spell_app(tmp_path: Path, name: str) -> Iterator[tuple[WrenApp, Path, str]]:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with spell_app(tmp_path, name) as (app, root):
        window = focus_native(app)
        gestures.window_size(app, window, 1280, 800)
        wait_marks(app, MARKS, "the acceptance note's nine marks")
        yield app, root, window


def word_point(app: WrenApp, line: int, word: str) -> tuple[int, int]:
    """A point on the word's glyphs: a few pixels above its zigzag."""
    rect = mark(app, line, word)["rects"][0]
    return round(rect["x"] + rect["width"] / 2), round(rect["y"] - 4)


def row_point(bounds: dict, row: int) -> tuple[int, int]:
    return (round(bounds["x"] + bounds["width"] / 2),
            round(bounds["y"] + MENU_PADDING + MENU_ROW * row + MENU_ROW / 2))


@pytest.mark.native_x11
def test_native_right_click_opens_the_menu_and_a_click_accepts(tmp_path: Path) -> None:
    with native_spell_app(tmp_path, "spell-native-click") as (app, root, window):
        # 5. A right-click on a marked word opens the menu under it.
        line = 5
        point = word_point(app, line, "apear")
        gestures.move(window, point)
        xdo("click", "3")
        opened = drawn_menu(app)
        assert opened["word"] == "apear" and opened["line"] == line
        assert opened["bounds"]["y"] > point[1]
        assert app.client.call("navigation.get")["column"] == "editor"
        # Moving over the second row selects it; a click accepts it.
        gestures.move(window, row_point(opened["bounds"], 1))
        wait_for(lambda: (menu(app) or {}).get("selected") == 1,
                 description="the pointer selects the second row")
        xdo("click", "1")
        second = opened["items"][1]
        wait_for(lambda: lines(app)[line] == original().split("\n")[line].replace("apear", second),
                 description="the click accepts the second row")
        assert menu(app) is None

        # With the menu open, a click on another note in the list opens it.
        point = word_point(app, 16, "misteak")
        gestures.move(window, point)
        xdo("click", "3")
        assert drawn_menu(app)["word"] == "misteak"
        other = gestures.item(app, "notes", OTHER)["id"]
        row = gestures.visible_row(app, {"note": other})
        gestures.move(window, gestures.point_in(row["visible"]))
        xdo("click", "1")
        wait_for(lambda: app.client.call("session.get")["document"]["note"] == other,
                 description="the click on the list switches notes")
        assert menu(app) is None
        wait_for(lambda: (root / NOTE).read_text(encoding="utf-8")
                 == original().replace("apear", second),
                 description="the accepted word was saved")


@pytest.mark.native_x11
def test_native_right_click_takes_the_keyboard_from_the_find_field(tmp_path: Path) -> None:
    with native_spell_app(tmp_path, "spell-native-find") as (app, _root, window):
        xdo("key", "--window", window, "--clearmodifiers", "ctrl+f")
        wait_for(lambda: "find-query" in focused(app), description="the find field is focused")
        gestures.move(window, word_point(app, 8, "bundeled"))
        xdo("click", "3")
        assert drawn_menu(app)["word"] == "bundeled"
        wait_for(lambda: "note-editor" in focused(app), description="the editor takes the keyboard")
        assert "find-query" not in focused(app)
        xdo("type", "--window", window, "--clearmodifiers", "--delay", "15", "z")
        wait_for(lambda: "z" in editor(app)["text"], description="the key reaches the text")
        state = editor(app)
        assert state["find_query"] == ""
        assert state["text"].replace("z", "", 1) == original()
        assert menu(app) is None


@pytest.mark.native_x11
def test_native_suggest_key_arrows_escape_and_return(tmp_path: Path) -> None:
    with native_spell_app(tmp_path, "spell-native-keys") as (app, _root, window):
        def keys(*names: str) -> None:
            xdo("key", "--window", window, "--clearmodifiers", *names)

        # Escape closes only the menu, though the find bar is open too.
        keys("ctrl+f")
        wait_for(lambda: editor(app)["find_open"], description="the find bar opens")
        line = 9
        caret(app, line, 13)
        app.client.call("editor.focus")
        wait_for(lambda: "note-editor" in focused(app), description="the editor is focused")
        keys("alt+period")
        opened = drawn_menu(app)
        assert opened["word"] == "desing" and opened["selected"] == 0
        keys("Down")
        wait_for(lambda: (menu(app) or {}).get("selected") == 1, description="Down selects")
        keys("Escape")
        wait_for(lambda: menu(app) is None, description="Escape closes the menu")
        assert editor(app)["find_open"] is True
        assert "note-editor" in focused(app)
        assert editor(app)["text"] == original()

        keys("alt+period")
        assert drawn_menu(app)["selected"] == 0
        keys("Down", "Return")
        second = opened["items"][1]
        wait_for(lambda: lines(app)[line] == original().split("\n")[line].replace("desing", second),
                 description="Down then Return accepts the second row")
        assert menu(app) is None
        assert editor(app)["cursor"] == {"line": line, "column": 11 + len(second)}
