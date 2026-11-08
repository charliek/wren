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


def test_word_deletes_take_one_group_and_undo_restores_it(keys_app) -> None:
    cases = [
        ("delete_word_back", "foo!!", 5, "foo", 3),
        ("delete_word_back", "**bold**", 8, "**bold", 6),
        ("delete_word_back", "- [ ] ", 6, "- [ ", 4),
        ("delete_word_back", "ok👍👍", len("ok👍👍".encode()), "ok", 2),
        ("delete_word_back", "日本語のテキスト", len("日本語のテキスト".encode()), "", 0),
        ("delete_word_forward", "!!foo", 0, "foo", 0),
    ]
    with keys_app("keys-word-delete") as app:
        editor = Editor(app)
        for intent, before, column, after, at in cases:
            editor.load(before, 0, column)
            state = editor.press(intent)
            assert (state["text"], cursor(state)) == (after, (0, at)), (intent, before)
            state = editor.call("editor.undo")
            assert (state["text"], cursor(state)) == (before, (0, column)), (intent, before)


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
        assert editor.press("shift_tab")["text"] == "- a\n"
        assert editor.press("tab")["text"] == "- a\n    "
        assert editor.press("backspace")["text"] == "- a\n   "
        editor.load("- [ ] task", 0, 10)
        assert cursor(editor.press("home")) == (0, 0)
        # Tab over several lines indents them; it never replaces them.
        editor.load("one\ntwo\nthree", 2, 5)
        editor.call("editor.cursor", {"line": 2, "column": 5,
                                      "selection": {"line": 0, "column": 0}})
        state = editor.press("tab")
        assert state["text"] == "    one\n    two\n    three"
        assert state["selected_text"] == state["text"]
        assert editor.press("shift_tab")["text"] == "one\ntwo\nthree"


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

        call("editor.set", {"text": "foo **bold**!!"})
        key("ctrl+End", "ctrl+BackSpace")
        native_text(app, "foo **bold", "ctrl+BackSpace takes one run of punctuation")
        key("alt+BackSpace")
        native_text(app, "foo **", "alt+BackSpace takes one word")
        key("ctrl+z")
        native_text(app, "foo **bold", "ctrl+z restores the word")


@pytest.mark.native_x11
def test_native_keys_with_assist_off(keys_app) -> None:
    """Without assist, Tab indents selected lines, and the Markdown commands'
    keys do what they did before there were commands."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("keys-native-plain", settings="markdown-assist = off\n") as app:
        call = app.client.call
        call("editor.set", {"text": "one\ntwo\nthree"})
        window = focus_native(app)

        def key(*names: str) -> None:
            xdo("key", "--window", window, "--clearmodifiers", *names)

        key("ctrl+Home", "shift+Down", "shift+Down", "shift+End", "Tab")
        native_text(app, "    one\n    two\n    three", "Tab indents the selected lines")
        key("shift+Tab")
        native_text(app, "one\ntwo\nthree", "Shift+Tab outdents them")
        call("editor.set", {"text": "ab\ncd"})
        call("editor.cursor", {"line": 1, "column": 2, "selection": None})
        key("alt+Up")
        wait_for(lambda: call("editor.get")["cursor"] == {"line": 0, "column": 2},
                 description="alt+Up moves the cursor up")
        assert call("editor.get")["text"] == "ab\ncd"
        key("ctrl+Return")
        native_text(app, "ab\n\ncd", "ctrl+Return breaks the line")


def run(app: WrenApp, command: str) -> str:
    """Run *command* as the palette does, returning its disposition."""
    return app.client.call("command.run", {"id": command})["disposition"]


def test_markdown_commands_act_at_the_cursor_from_the_palette(keys_app) -> None:
    with keys_app("keys-commands") as app:
        editor = Editor(app)
        get = lambda: editor.call("editor.get")  # noqa: E731

        editor.load("- [ ] a", 0, 7)
        assert run(app, "markdown.toggle_task") == "completed"
        assert (get()["text"], cursor(get())) == ("- [x] a", (0, 7))
        run(app, "markdown.toggle_task")
        assert get()["text"] == "- [ ] a"
        editor.load("plain", 0, 2)
        run(app, "markdown.toggle_task")
        assert (get()["text"], cursor(get())) == ("- [ ] plain", (0, 8))

        for command, wrapped in [
            ("markdown.bold", "**hello** world"),
            ("markdown.italic", "*hello* world"),
            ("markdown.code", "`hello` world"),
        ]:
            editor.load("hello world", 0, 2)
            run(app, command)
            assert get()["text"] == wrapped, command
            # Again on the wrapped word unwraps it, as one more undo step.
            run(app, command)
            assert get()["text"] == "hello world", command
            assert editor.call("editor.undo")["text"] == wrapped, command

        editor.load("1. a\n2. b", 1, 4)
        run(app, "edit.move_line_up")
        assert (get()["text"], cursor(get())) == ("1. b\n2. a", (0, 4))
        run(app, "edit.move_line_down")
        assert (get()["text"], cursor(get())) == ("1. a\n2. b", (1, 4))
        # At the top, moving up does nothing.
        editor.load("a\nb", 0, 1)
        run(app, "edit.move_line_up")
        assert get()["text"] == "a\nb"


def test_markdown_commands_are_unavailable_without_assist(keys_app) -> None:
    with keys_app("keys-commands-off", settings="markdown-assist = off\n") as app:
        editor = Editor(app)
        editor.load("- [ ] a\nb", 0, 7)
        for command in ["markdown.toggle_task", "markdown.bold", "edit.move_line_down"]:
            assert run(app, command) == "blocked", command
        assert editor.call("editor.get")["text"] == "- [ ] a\nb"


def test_settings_commands_switch_the_engine_and_markdown_assist(keys_app, engine) -> None:  # noqa: F811
    other = "advanced" if engine == "classic" else "classic"
    with keys_app("keys-settings-commands") as app:
        editor = Editor(app)
        get = lambda: editor.call("editor.get")  # noqa: E731
        settings = app.settings_path
        editor.load("- a\n- b", 1, 3)

        assert run(app, "editor.toggle_engine") == "completed"
        state = get()
        assert state["engine"] == other
        assert (state["text"], cursor(state)) == ("- a\n- b", (1, 3))
        assert f"editor-engine = {other}\n" in settings.read_text()

        assert run(app, "editor.toggle_markdown_assist") == "completed"
        assert "markdown-assist = off\n" in settings.read_text()
        assert editor.press("enter")["text"] == "- a\n- b\n"
        assert run(app, "markdown.bold") == "blocked"

        assert run(app, "editor.toggle_markdown_assist") == "completed"
        assert "markdown-assist = on\n" in settings.read_text()
        editor.load("- a", 0, 3)
        assert editor.press("enter")["text"] == "- a\n- "

        assert run(app, "editor.toggle_engine") == "completed"
        state = get()
        assert (state["engine"], state["text"], cursor(state)) == (engine, "- a\n- ", (1, 2))
        assert f"editor-engine = {engine}\n" in settings.read_text()


def test_markdown_shortcuts_act_only_in_the_focused_editor(keys_app) -> None:
    with keys_app("keys-shortcuts") as app:
        call = app.client.call
        editor = Editor(app)
        editor.load("hello world", 0, 2)
        call("editor.focus")

        def shortcut(command: str) -> None:
            call("shortcut", {"id": command})

        shortcut("markdown.bold")
        assert call("editor.get")["text"] == "**hello** world"
        shortcut("markdown.toggle_task")
        assert call("editor.get")["text"] == "- [ ] **hello** world"
        # With the find bar holding the keyboard, the keys do nothing.
        shortcut("navigation.find")
        wait_for(lambda: call("focus.get")["named"] == ["find-query"],
                 description="find bar focus")
        for command in ["markdown.bold", "markdown.toggle_task", "edit.move_line_up"]:
            shortcut(command)
        assert call("editor.get")["text"] == "- [ ] **hello** world"


def test_markdown_shortcuts_skip_a_browser_column(tmp_path: Path, keys_app) -> None:
    note = workspace(tmp_path / "notes", b"1. a\n2. b")
    with keys_app("keys-shortcuts-column", workspace=tmp_path / "notes") as app:
        call = app.client.call
        open_note(app)
        assert call("column.open", {"column": "notes"})["disposition"] == "completed"
        wait_for(lambda: call("focus.get")["named"] == [], description="the editor lets go")
        call("shortcut", {"id": "edit.move_line_up"})
        state = call("editor.get")
        assert state["text"] == "1. a\n2. b"
        assert call("navigation.get")["column"] == "notes"
        assert not call("session.get")["document"]["dirty"]
        assert note.read_bytes() == b"1. a\n2. b"


@pytest.mark.native_x11
def test_native_markdown_command_keys(keys_app) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("keys-native-commands") as app:
        call = app.client.call
        call("editor.set", {"text": "# Title\nhello world"})
        window = focus_native(app)

        def key(*names: str) -> None:
            xdo("key", "--window", window, "--clearmodifiers", *names)

        def typed(text: str) -> None:
            xdo("type", "--window", window, "--clearmodifiers", "--delay", "12", text)

        key("ctrl+End", "ctrl+Return")
        native_text(app, "# Title\n- [ ] hello world", "ctrl+Return makes a task")
        key("ctrl+Return")
        native_text(app, "# Title\n- [x] hello world", "ctrl+Return checks it")
        key("ctrl+b")
        native_text(app, "# Title\n- [x] hello **world**", "ctrl+b bolds the word")
        key("ctrl+i")
        native_text(app, "# Title\n- [x] hello ***world***", "ctrl+i adds italics")
        key("alt+Up")
        native_text(app, "- [x] hello ***world***\n# Title", "alt+Up moves the line")
        key("alt+Down")
        native_text(app, "# Title\n- [x] hello ***world***", "alt+Down moves it back")
        key("ctrl+z")
        native_text(app, "- [x] hello ***world***\n# Title", "one undo per command")

        # In the find bar the keys neither format the note nor type.
        key("ctrl+f")
        wait_for(lambda: call("editor.get")["find_open"], description="ctrl+f opens find")
        typed("wor")
        key("ctrl+b", "ctrl+Return", "alt+Up")
        typed("l")
        wait_for(lambda: call("editor.get")["find_query"] == "worl", description="find typing")
        assert call("editor.get")["text"] == "- [x] hello ***world***\n# Title"


@pytest.mark.native_x11
def test_native_alt_up_in_the_notes_column_leaves_the_text(tmp_path: Path, keys_app) -> None:
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    note = workspace(tmp_path / "notes", b"1. a\n2. b")
    with keys_app("keys-native-column", workspace=tmp_path / "notes") as app:
        call = app.client.call
        open_note(app)
        window = focus_native(app)
        call("column.open", {"column": "notes"})

        def key(*names: str) -> None:
            xdo("key", "--window", window, "--clearmodifiers", *names)

        key("alt+Up", "alt+Down")
        # Column right is an app shortcut: once it lands, so have the keys
        # before it.
        key("alt+shift+bracketright")
        wait_for(lambda: call("navigation.get")["column"] == "editor",
                 description="back in the editor")
        assert call("editor.get")["text"] == "1. a\n2. b"
        assert not call("session.get")["document"]["dirty"]
        # In the editor the same key moves the line.
        key("alt+Up")
        native_text(app, "1. b\n2. a", "alt+Up in the editor")
        saved(app, note, b"1. b\n2. a")
