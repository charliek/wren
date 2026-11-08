"""Markdown-aware editing keys through the real editor.

``editor.command`` sends an editing intent the way the editor widget does
for its keys, through the app's one edit path; the ``native_x11`` cases press
the keys themselves. Every case runs on the engines ``WRENTEST_ENGINE`` names.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
from pathlib import Path

import pytest

from tools.wrentest.test_editor import engine, focus_native, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for


@pytest.fixture
def keys_app(engine: str):  # noqa: F811
    """Launch WrenApps on the engine under test, with extra settings."""

    @contextmanager
    def launch(name: str, settings: str = "", **kwargs) -> Iterator[WrenApp]:
        with WrenApp(settings=f"editor-engine = {engine}\n{settings}",
                     artifacts_dir=artifact_directory(name), **kwargs) as app:
            assert app.client.call("editor.get")["engine"] == engine
            yield app

    return launch


class Editor:
    """The in-memory editor of one app, driven by intents."""

    def __init__(self, app: WrenApp) -> None:
        self.call = app.client.call

    def load(self, text: str, line: int, column: int) -> None:
        self.call("editor.set", {"text": text})
        self.call("editor.cursor", {"line": line, "column": column, "selection": None})

    def press(self, intent: str) -> dict:
        return self.call("editor.command", {"intent": intent})

    def type(self, text: str) -> dict:
        return self.call("editor.insert", {"text": text})


def cursor(state: dict) -> tuple[int, int]:
    return state["cursor"]["line"], state["cursor"]["column"]


def test_enter_continues_lists_quotes_and_tasks_then_exits(keys_app) -> None:
    with keys_app("keys-enter") as app:
        editor = Editor(app)
        editor.load("- a", 0, 3)
        state = editor.press("enter")
        assert (state["text"], cursor(state)) == ("- a\n- ", (1, 2))
        assert editor.type("b")["text"] == "- a\n- b"
        assert editor.press("enter")["text"] == "- a\n- b\n- "
        # Enter on the empty item ends the list, leaving a blank line.
        state = editor.press("enter")
        assert (state["text"], cursor(state)) == ("- a\n- b\n\n", (3, 0))

        for before, after in [
            ("> quote", "> quote\n> "),
            ("- [x] done", "- [x] done\n- [ ] "),
            ("9. nine", "9. nine\n10. "),
            ("  - sub", "  - sub\n  - "),
            ("plain", "plain\n"),
        ]:
            editor.load(before, 0, len(before))
            assert editor.press("enter")["text"] == after, before

        # Shift+Enter stays inside the item, under its text.
        editor.load("- [ ] a", 0, 7)
        state = editor.press("shift_enter")
        assert (state["text"], cursor(state)) == ("- [ ] a\n  ", (1, 2))

        # One undo takes back one smart Enter.
        editor.load("1. a\n2. b", 0, 4)
        assert editor.press("enter")["text"] == "1. a\n2. \n3. b"
        assert editor.call("editor.undo")["text"] == "1. a\n2. b"


def test_tab_and_shift_tab_nest_items_and_renumber(keys_app) -> None:
    with keys_app("keys-tab") as app:
        editor = Editor(app)
        editor.load("1. a\n2. b\n3. c", 1, 4)
        state = editor.press("tab")
        assert (state["text"], cursor(state)) == ("1. a\n   1. b\n2. c", (1, 7))
        assert editor.press("shift_tab")["text"] == "1. a\n2. b\n3. c"

        editor.load("- a\n- b\n  - child", 1, 3)
        assert editor.press("tab")["text"] == "- a\n  - b\n    - child"

        # No previous sibling: Tab is taken and changes nothing.
        editor.load("- a", 0, 3)
        state = editor.press("tab")
        assert state["text"] == "- a"
        assert state["can_undo"] is False

        # A plain line gets spaces to the next multiple of 4.
        editor.load("ab", 0, 2)
        assert editor.press("tab")["text"] == "ab  "


def test_backspace_removes_markup_then_whole_characters(keys_app) -> None:
    with keys_app("keys-backspace") as app:
        editor = Editor(app)
        editor.load("- [ ] ", 0, 6)
        assert editor.press("backspace")["text"] == "- "
        assert editor.press("backspace")["text"] == ""

        editor.load("> > ", 0, 4)
        assert editor.press("backspace")["text"] == "> "

        editor.load("- a\n- b", 1, 2)
        state = editor.press("backspace")
        assert (state["text"], cursor(state)) == ("- a\n  b", (1, 2))

        flag = "\U0001F1EF\U0001F1F5"
        editor.load(f"é{flag}", 0, len(f"é{flag}".encode()))
        assert editor.press("backspace")["text"] == "é"
        assert editor.press("backspace")["text"] == ""

        editor.load("foo bar  ", 0, 9)
        assert editor.press("delete_word_back")["text"] == "foo "
        editor.load("- [ ] foo", 0, 9)
        assert editor.press("delete_to_line_start")["text"] == "- [ ] "
        assert editor.press("delete_to_line_start")["text"] == ""


def test_home_toggles_between_the_text_and_column_0(keys_app) -> None:
    with keys_app("keys-home") as app:
        editor = Editor(app)
        editor.load("- [ ] task", 0, 10)
        assert cursor(editor.press("home")) == (0, 6)
        assert cursor(editor.press("home")) == (0, 0)
        assert cursor(editor.press("home")) == (0, 6)
        editor.load("- [ ] task", 0, 10)
        state = editor.press("select_home")
        assert cursor(state) == (0, 6)
        assert state["selection"] == {"line": 0, "column": 10}
        assert state["selected_text"] == "task"
        assert state["can_undo"] is False


def test_with_assist_off_the_keys_edit_plain_text(keys_app) -> None:
    with keys_app("keys-plain", settings="markdown-assist = off\n") as app:
        editor = Editor(app)
        assert editor.call("editor.get")["markdown_assist"] is False
        editor.load("- a", 0, 3)
        assert editor.press("enter")["text"] == "- a\n"
        assert editor.press("tab")["text"] == "- a\n    "
        assert editor.press("shift_tab")["text"] == "- a\n    "
        assert editor.press("backspace")["text"] == "- a\n   "
        editor.load("- [ ] task", 0, 10)
        assert cursor(editor.press("home")) == (0, 0)


def test_the_word_count_catches_up_after_smart_edits(keys_app) -> None:
    with keys_app("keys-words") as app:
        editor = Editor(app)
        editor.load("- one", 0, 5)
        editor.press("enter")
        editor.type("two three")
        wait_for(lambda: editor.call("editor.get")["words"] == 5, timeout=3,
                 description="word count after smart Enter")


def workspace(root: Path, contents: bytes) -> Path:
    (root / "S" / "G").mkdir(parents=True)
    (root / "S" / "G" / "a.md").write_bytes(contents)
    return root / "S" / "G" / "a.md"


def open_note(app: WrenApp) -> None:
    snapshot = app.client.call("session.get")
    note = next(entry["id"] for entry in snapshot["notes"] if entry["path"] == "S/G/a.md")
    app.client.call("core.apply", {"op": "open_note", "note": note})
    app.client.call("editor.action", {"kind": "move", "motion": "end"})


def saved(app: WrenApp, path: Path, expected: bytes) -> None:
    wait_for(lambda: path.read_bytes() == expected
             and not app.client.call("session.get")["document"]["dirty"],
             timeout=5, interval=0.025, description="autosaved bytes and a clean document")


def test_a_smart_edit_alone_reaches_the_file(tmp_path: Path, keys_app) -> None:
    note = workspace(tmp_path / "notes", b"- [x] first")
    with keys_app("keys-autosave", workspace=tmp_path / "notes") as app:
        open_note(app)
        assert Editor(app).press("enter")["text"] == "- [x] first\n- [ ] "
        assert app.client.call("session.get")["document"]["dirty"]
        saved(app, note, b"- [x] first\n- [ ] ")


def test_a_crlf_note_stays_crlf_through_smart_enter_and_autosave(tmp_path: Path, keys_app) -> None:
    note = workspace(tmp_path / "notes", b"- one\r\n- two")
    with keys_app("keys-crlf", workspace=tmp_path / "notes") as app:
        open_note(app)
        editor = Editor(app)
        assert editor.press("enter")["text"] == "- one\r\n- two\r\n- "
        editor.type("three")
        editor.press("enter")
        assert editor.press("enter")["text"] == "- one\r\n- two\r\n- three\r\n\r\n"
        saved(app, note, b"- one\r\n- two\r\n- three\r\n\r\n")


def native_text(app: WrenApp, expected: str, description: str) -> None:
    wait_for(lambda: app.client.call("editor.get")["text"] == expected, description=description)


@pytest.mark.native_x11
def test_native_smart_keys(keys_app) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("keys-native") as app:
        call = app.client.call
        call("editor.set", {"text": "- a"})
        window = focus_native(app)

        def key(*names: str) -> None:
            xdo("key", "--window", window, "--clearmodifiers", *names)

        def typed(text: str) -> None:
            xdo("type", "--window", window, "--clearmodifiers", "--delay", "12", text)

        key("ctrl+End", "Return")
        native_text(app, "- a\n- ", "Return continues the list")
        typed("b")
        native_text(app, "- a\n- b", "typing after Return")
        key("Tab")
        native_text(app, "- a\n  - b", "Tab nests the item")
        # Tab stayed in the editor: the keyboard still types there.
        typed("c")
        native_text(app, "- a\n  - bc", "typing after Tab")
        key("shift+Tab")
        native_text(app, "- a\n- bc", "Shift+Tab outdents the item")
        key("shift+Return")
        native_text(app, "- a\n- bc\n  ", "Shift+Return stays in the item")
        key("BackSpace")
        native_text(app, "- a\n- bc\n", "BackSpace goes back an indent stop")
        key("BackSpace")
        native_text(app, "- a\n- bc", "BackSpace joins the lines")
        def at(column: int, description: str) -> None:
            wait_for(lambda: call("editor.get")["cursor"] == {"line": 1, "column": column},
                     description=description)

        key("Home")
        at(2, "Home stops after the marker")
        typed("X")
        native_text(app, "- a\n- Xbc", "typing at the text start")
        key("Home")
        at(2, "Home goes back to the text start")
        key("Home")
        at(0, "Home again goes to column 0")
        key("Home")
        at(2, "and back to the text start")
        key("shift+End", "BackSpace")
        native_text(app, "- a\n- ", "BackSpace deletes a selection")
        key("BackSpace")
        native_text(app, "- a\n  ", "BackSpace turns the marker into indentation")
