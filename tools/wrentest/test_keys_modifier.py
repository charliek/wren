"""Linux's command key, Mod, in both its modes (discovery/keybindings.md).

``linux-modifier = ctrl`` (the default) makes Ctrl the command key, and
``alt`` makes it Alt, the way ⌘ works on the Mac. Every case writes the
mode it is about into ``config.conf`` itself, so it means the same thing
whatever ``WRENTEST_LINUX_MODIFIER`` the lane pins, and runs once in each
mode, on the engines ``WRENTEST_ENGINE`` names:

- the palette, bold and task keys, undo and redo on Mod+Z and Mod+Shift+Z,
  and no redo on Ctrl+Y;
- the labels the palette and the menus show for the keys;
- the palette command that switches the mode, after which every key and
  label has moved, with no restart;
- in Alt mode, Alt+A, Alt+C and Alt+V in the editor, the palette's input
  and the calendar's field; in Ctrl mode, none of them;
- and the same chords pressed through X11 (``native_x11``).

macOS has no such setting, so the file is Linux's alone.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
import re
import time

import pytest

from tools.wrentest.boards import FLOW, NOTES, board, click, copy_samples, open_note
from tools.wrentest.boards import key as board_key
from tools.wrentest.test_editor import engine, focus_native, xdo  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp, wait_for

pytestmark = pytest.mark.skipif(os.uname().sysname == "Darwin",
                                reason="linux-modifier is Linux's alone")

MODES = ("ctrl", "alt")
SWITCH = "keys.toggle_linux_modifier"


@pytest.fixture(params=MODES)
def mode(request: pytest.FixtureRequest) -> str:
    return request.param


def other(mode: str) -> str:
    """The modifier that is not the command key."""
    return "alt" if mode == "ctrl" else "ctrl"


def label(mode: str) -> str:
    """How a label writes the command key."""
    return "Ctrl" if mode == "ctrl" else "Alt"


@contextmanager
def keys_app(name: str, engine: str, mode: str,  # noqa: F811
             settings: str = "", **kwargs) -> Iterator[WrenApp]:
    with WrenApp(settings=f"editor-engine = {engine}\nlinux-modifier = {mode}\n{settings}",
                 artifacts_dir=artifact_directory(f"keys-modifier-{name}-{mode}-{engine}"),
                 **kwargs) as app:
        assert app.client.call("editor.get")["engine"] == engine
        yield app


def press(app: WrenApp, key: str, *mods: str) -> dict:
    """A press the focused editor routes, and the app after it."""
    return app.client.call("editor.key", {"key": key, "mods": list(mods)})


def text(app: WrenApp) -> str:
    return app.client.call("editor.get")["text"]


def modal(app: WrenApp) -> dict | None:
    return app.client.call("navigation.get")["modal"]


def command_rows(app: WrenApp) -> dict[str, dict]:
    """The command palette's rows by command key; the palette is left shut."""
    app.client.call("palette.open", {"mode": "commands"})
    rows = app.client.call("palette.query", {"query": ""})["navigation"]["modal"]["results"]
    app.client.call("palette.close")
    return {row["key"]["command"]: row for row in rows if "command" in row["key"]}


def menu_trigger(app: WrenApp, key: str) -> str | None:
    """The trigger the menus show for the command *key*."""
    def walk(items: list[dict]) -> Iterator[dict]:
        for item in items:
            if "key" in item:
                yield item
            yield from walk(item.get("items", []))

    for menu in app.client.call("menu.get")["menus"]:
        for item in walk(menu["items"]):
            if item["key"] == key:
                return item["trigger"]
    raise AssertionError(f"no menu item runs {key}")


def task_stamp(app: WrenApp) -> str:
    """What checking a task writes after it: the done date, with tasks on."""
    state = app.client.call("tasks.get")
    return f" ✅ {state['today']}" if state["tasks"] else ""


def assert_mode(app: WrenApp, mode: str) -> None:
    """Every key and label follows *mode*, and the other modifier's chords
    are no commands."""
    call = app.client.call
    rows = command_rows(app)
    assert rows[SWITCH]["title"] == ("Use Alt for commands" if mode == "ctrl"
                                     else "Use Ctrl for commands")
    assert rows[SWITCH]["shortcut"] == ""
    assert rows["navigation.jump"]["shortcut"] == f"{label(mode)}+P"
    assert rows["navigation.search"]["shortcut"] == f"{label(mode)}+Shift+F"
    assert rows["markdown.bold"]["shortcut"] == f"{label(mode)}+B"
    assert rows["markdown.toggle_task"]["shortcut"] == f"{label(mode)}+↵"
    assert menu_trigger(app, "navigation.jump") == f"{mode}+p"

    call("editor.set", {"text": "hello world"})
    call("editor.cursor", {"line": 0, "column": 2, "selection": None})
    call("editor.focus")
    # The other modifier's bold neither formats nor types.
    press(app, "b", other(mode))
    assert text(app) == "hello world"
    press(app, "b", mode)
    assert text(app) == "**hello** world"
    press(app, "enter", mode)
    assert text(app) == "- [ ] **hello** world"
    press(app, "enter", mode)
    assert text(app) == f"- [x] **hello** world{task_stamp(app)}"
    # Undo and redo are Mod+Z and Mod+Shift+Z; Ctrl+Y redoes nothing.
    press(app, "z", mode)
    assert text(app) == "- [ ] **hello** world"
    press(app, "z", mode)
    assert text(app) == "**hello** world"
    press(app, "y", "ctrl")
    assert text(app) == "**hello** world"
    assert call("editor.get")["can_redo"] is True
    press(app, "z", mode, "shift")
    assert text(app) == "- [ ] **hello** world"
    if mode == "alt":
        # The desktop's Ctrl+Z and Ctrl+Shift+Z keep working.
        press(app, "z", "ctrl")
        assert text(app) == "**hello** world"
        press(app, "z", "ctrl", "shift")
        assert text(app) == "- [ ] **hello** world"
    else:
        # Alt is no command key: Alt+Z undoes nothing.
        press(app, "z", "alt")
        assert text(app) == "- [ ] **hello** world"
    # The palette opens on Mod+Shift+P, and not on the other modifier's.
    press(app, "P", other(mode), "shift")
    assert modal(app) is None
    press(app, "P", mode, "shift")
    assert modal(app)["mode"] == "commands"
    call("palette.close")


def test_each_mode_moves_the_command_keys_and_their_labels(engine, mode) -> None:  # noqa: F811
    with keys_app("keys", engine, mode) as app:
        assert_mode(app, mode)


def test_the_palette_command_switches_the_mode_at_once_and_saves_it(engine, mode) -> None:  # noqa: F811
    with keys_app("switch", engine, mode, "# kept\n") as app:
        call = app.client.call
        target = other(mode)
        title = f"Use {label(target)} for commands"
        # From the palette, as a user runs it.
        call("palette.open", {"mode": "commands"})
        rows = call("palette.query", {"query": title})["navigation"]["modal"]["results"]
        index = next(at for at, row in enumerate(rows) if row["key"] == {"command": SWITCH})
        assert rows[index]["enabled"] is True
        result = call("palette.activate", {"index": index})
        assert result["disposition"] == "completed", result
        assert modal(app) is None
        settings = app.settings_path.read_text(encoding="utf-8")
        assert re.findall(r"(?m)^linux-modifier = (\w+)$", settings) == [target], settings
        assert "# kept\n" in settings
        assert call("session.get")["banner"] is None
        assert_mode(app, target)
        # And back, by its key in the command protocol.
        assert call("command.run", {"id": SWITCH})["disposition"] == "completed"
        settings = app.settings_path.read_text(encoding="utf-8")
        assert re.findall(r"(?m)^linux-modifier = (\w+)$", settings) == [mode], settings
        assert_mode(app, mode)
        # A restart reads the mode the switch left.
        app.restart(hard=False)
        assert_mode(app, mode)


# ── Native X11 ────────────────────────────────────────────────────────────────


def native(window: str, *keys: str) -> None:
    xdo("key", "--window", window, "--clearmodifiers", *keys)


def typed(window: str, value: str) -> None:
    xdo("type", "--window", window, "--clearmodifiers", "--delay", "12", value)


def settle() -> None:
    """Long enough for a chord that does nothing to have done it."""
    time.sleep(0.3)


@pytest.mark.native_x11
def test_native_chords_follow_the_mode(engine, mode) -> None:  # noqa: F811
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("native", engine, mode) as app:
        call = app.client.call
        call("editor.set", {"text": ""})
        window = focus_native(app)
        typed(window, "hello world")
        wait_for(lambda: text(app) == "hello world", description="native typing")
        # The other modifier's bold neither formats nor types its letter.
        native(window, f"{other(mode)}+b")
        settle()
        assert text(app) == "hello world"
        native(window, f"{mode}+b")
        wait_for(lambda: text(app) == "hello **world**", description="Mod+B bolds")
        native(window, f"{mode}+Return")
        wait_for(lambda: text(app) == "- [ ] hello **world**", description="Mod+Return tasks")
        native(window, f"{mode}+z")
        wait_for(lambda: text(app) == "hello **world**", description="Mod+Z undoes")
        native(window, "ctrl+y")
        settle()
        assert text(app) == "hello **world**"
        native(window, f"{mode}+shift+z")
        wait_for(lambda: text(app) == "- [ ] hello **world**", description="Mod+Shift+Z redoes")
        # Select all and copy: Mod's, and in Alt mode Ctrl's too; Alt's in
        # Ctrl mode do nothing. Two lines, so that a copy with nothing
        # selected (the caret's line) is no select all, and a sentinel on
        # the clipboard before each copy.
        two = "first line\nsecond line"
        call("editor.set", {"text": two})
        call("editor.cursor", {"line": 1, "column": 3, "selection": None})
        window = focus_native(app)
        selected = lambda: call("editor.get")["selected_text"]  # noqa: E731
        call("clipboard.write", {"text": "sentinel"})
        native(window, "alt+a")
        if mode == "alt":
            wait_for(lambda: selected() == two, description="Alt+A selects the note")
        else:
            settle()
            assert selected() in (None, "")
        call("clipboard.write", {"text": "sentinel"})
        native(window, "alt+c")
        if mode == "alt":
            wait_for(lambda: call("clipboard.read") == two, description="Alt+C copies it")
        else:
            settle()
            assert call("clipboard.read") == "sentinel"
        call("editor.cursor", {"line": 1, "column": 3, "selection": None})
        native(window, "ctrl+a")
        wait_for(lambda: selected() == two, description="Ctrl+A selects the note")
        call("clipboard.write", {"text": "sentinel"})
        native(window, "ctrl+c")
        wait_for(lambda: call("clipboard.read") == two, description="Ctrl+C copies it")
        # The palette opens on Mod+Shift+P, and not on the other's.
        call("editor.focus")
        native(window, f"{other(mode)}+shift+p")
        settle()
        assert modal(app) is None
        native(window, f"{mode}+shift+p")
        wait_for(lambda: (modal(app) or {}).get("mode") == "commands",
                 description="Mod+Shift+P opens the palette")
        native(window, "Escape")
        wait_for(lambda: modal(app) is None, description="Escape closes it")
        assert text(app) == two


@pytest.mark.native_x11
def test_native_alt_mode_edits_every_text_field_with_alt(engine, mode) -> None:  # noqa: F811
    """Alt+A, Alt+C and Alt+V in the editor, the palette's input and the
    calendar's field (advanced only), in Alt mode; in Ctrl mode none."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    alt = mode == "alt"
    with keys_app("fields", engine, mode, "tasks = on\n") as app:
        call = app.client.call
        call("editor.set", {"text": ""})
        window = focus_native(app)

        # The editor.
        typed(window, "note")
        wait_for(lambda: text(app) == "note", description="typing")
        call("clipboard.write", {"text": "clip"})
        native(window, "End", "alt+v")
        if alt:
            wait_for(lambda: text(app) == "noteclip", description="Alt+V pastes in the editor")
            native(window, "alt+a", "alt+c")
            wait_for(lambda: call("clipboard.read") == "noteclip",
                     description="Alt+A, Alt+C copy the note")
        else:
            settle()
            assert text(app) == "note"

        # The palette's input.
        call("clipboard.write", {"text": "clip"})
        native(window, f"{mode}+shift+p")
        wait_for(lambda: call("focus.get")["named"] == ["palette-query"],
                 description="the palette's input has the keyboard")
        typed(window, "abc")
        wait_for(lambda: modal(app)["query"] == "abc", description="typing in the palette")
        native(window, "alt+a", "alt+c", "End", "alt+v")
        if alt:
            wait_for(lambda: modal(app)["query"] == "abcabc",
                     description="Alt+A, Alt+C, Alt+V in the palette")
            assert call("clipboard.read") == "abc"
        else:
            settle()
            assert modal(app)["query"] == "abc"
            assert call("clipboard.read") == "clip"
        # Ctrl is the input's own in both modes.
        native(window, "ctrl+a", "ctrl+c")
        wait_for(lambda: call("clipboard.read") == modal(app)["query"],
                 description="Ctrl+A, Ctrl+C in the palette")
        native(window, "Escape")
        wait_for(lambda: modal(app) is None, description="Escape closes the palette")

        # The calendar's field, at a task's box, on the advanced engine.
        call("editor.set", {"text": "- [ ] task"})
        call("editor.cursor", {"line": 0, "column": 10, "selection": None})
        window = focus_native(app)
        if engine == "advanced":
            native(window, f"{mode}+d")
            wait_for(lambda: call("focus.get")["named"] == ["calendar-field"],
                     description="the calendar's field has the keyboard")
            typed(window, "fri")
            wait_for(lambda: call("tasks.calendar")["typed"] == "fri",
                     description="typing in the field")
            call("clipboard.write", {"text": "clip"})
            native(window, "alt+a", "alt+c", "End", "alt+v")
            if alt:
                wait_for(lambda: call("tasks.calendar")["typed"] == "frifri",
                         description="Alt+A, Alt+C, Alt+V in the field")
                assert call("clipboard.read") == "fri"
            else:
                settle()
                assert call("tasks.calendar")["typed"] == "fri"
                assert call("clipboard.read") == "clip"
            native(window, "Escape")
            wait_for(lambda: call("tasks.calendar") == {"open": False},
                     description="Escape closes the calendar")
            assert text(app) == "- [ ] task"
            window = focus_native(app)

        # The task form's description, on either engine, with tasks on.
        if not call("tasks.get")["tasks"]:
            return
        native(window, f"{mode}+shift+Return")
        wait_for(lambda: call("focus.get")["named"] == ["task-form-description"],
                 description="the form's description has the keyboard")
        assert call("tasks.form")["description"] == "task"
        call("clipboard.write", {"text": "clip"})
        native(window, "alt+a", "alt+c", "End", "alt+v")
        if alt:
            wait_for(lambda: call("tasks.form")["description"] == "tasktask",
                     description="Alt+A, Alt+C, Alt+V in the form")
            assert call("clipboard.read") == "task"
        else:
            settle()
            assert call("tasks.form")["description"] == "task"
            assert call("clipboard.read") == "clip"
        native(window, "Escape")
        wait_for(lambda: call("tasks.form") == {"open": False},
                 description="Escape cancels the form")
        assert text(app) == "- [ ] task"


@pytest.mark.native_x11
def test_native_the_switch_moves_the_keys_without_a_restart(engine, mode) -> None:  # noqa: F811
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("native-switch", engine, mode) as app:
        call = app.client.call
        target = other(mode)
        call("editor.set", {"text": "word"})
        call("editor.cursor", {"line": 0, "column": 4, "selection": None})
        window = focus_native(app)
        native(window, f"{mode}+b")
        wait_for(lambda: text(app) == "**word**", description="Mod+B bolds")
        native(window, f"{mode}+z")
        wait_for(lambda: text(app) == "word", description="Mod+Z undoes")
        assert call("command.run", {"id": SWITCH})["disposition"] == "completed"
        assert f"linux-modifier = {target}" in app.settings_path.read_text(encoding="utf-8")
        window = focus_native(app)
        # The old command key is a command no more, and types nothing.
        native(window, f"{mode}+b")
        settle()
        assert text(app) == "word"
        native(window, f"{target}+b")
        wait_for(lambda: text(app) == "**word**", description="the new Mod+B bolds")
        native(window, f"{target}+z")
        wait_for(lambda: text(app) == "word", description="the new Mod+Z undoes")
        native(window, f"{target}+shift+p")
        wait_for(lambda: (modal(app) or {}).get("mode") == "commands",
                 description="the new Mod+Shift+P opens the palette")
        native(window, "Escape")
        wait_for(lambda: modal(app) is None, description="Escape closes it")


@pytest.mark.native_x11
def test_native_an_unbound_clipboard_chord_does_nothing(engine, mode) -> None:  # noqa: F811
    """``keybind = ctrl+v = unbind``: Ctrl+V pastes neither in the editor
    nor in a text field, though the toolkit's widgets would on their own."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("unbound", engine, mode, "keybind = ctrl+v = unbind\n") as app:
        call = app.client.call
        assert call("session.get")["banner"] is None
        call("editor.set", {"text": "note"})
        call("editor.cursor", {"line": 0, "column": 4, "selection": None})
        window = focus_native(app)
        call("clipboard.write", {"text": "clip"})
        native(window, "ctrl+v")
        settle()
        assert text(app) == "note"
        native(window, f"{mode}+shift+p")
        wait_for(lambda: call("focus.get")["named"] == ["palette-query"],
                 description="the palette's input has the keyboard")
        typed(window, "abc")
        wait_for(lambda: modal(app)["query"] == "abc", description="typing in the palette")
        native(window, "ctrl+v")
        settle()
        assert modal(app)["query"] == "abc"
        # Its neighbours are the field's still.
        native(window, "ctrl+a", "ctrl+c")
        wait_for(lambda: call("clipboard.read") == "abc", description="Ctrl+A, Ctrl+C")
        native(window, "Escape")
        wait_for(lambda: modal(app) is None, description="Escape closes the palette")
        assert text(app) == "note"


@pytest.mark.native_x11
def test_native_a_clipboard_chord_given_to_a_command_does_only_it(engine, mode) -> None:  # noqa: F811
    """``keybind = ctrl+v = palette.commands``: in the editor and in the
    find bar's field Ctrl+V opens the palette and pastes nothing."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("rebound", engine, mode, "keybind = ctrl+v = palette.commands\n") as app:
        call = app.client.call
        assert call("session.get")["banner"] is None
        call("editor.set", {"text": "note"})
        call("editor.cursor", {"line": 0, "column": 4, "selection": None})
        window = focus_native(app)
        call("clipboard.write", {"text": "clip"})
        native(window, "ctrl+v")
        wait_for(lambda: (modal(app) or {}).get("mode") == "commands",
                 description="Ctrl+V opens the palette from the editor")
        assert (text(app), modal(app)["query"]) == ("note", "")
        native(window, "Escape")
        wait_for(lambda: modal(app) is None, description="Escape closes it")
        call("editor.focus")
        native(window, f"{mode}+f")
        wait_for(lambda: call("focus.get")["named"] == ["find-query"],
                 description="Mod+F opens Find")
        typed(window, "no")
        wait_for(lambda: call("editor.get")["find_query"] == "no", description="typing in Find")
        native(window, "ctrl+v")
        wait_for(lambda: (modal(app) or {}).get("mode") == "commands",
                 description="Ctrl+V opens the palette from the find bar")
        state = call("editor.get")
        assert (state["find_query"], state["text"], modal(app)["query"]) == ("no", "note", "")
        native(window, "Escape")
        wait_for(lambda: modal(app) is None, description="Escape closes it")


@pytest.mark.native_x11
def test_native_a_clipboard_chord_given_to_a_markdown_command_without_assist(
        engine, mode) -> None:  # noqa: F811
    """``keybind = ctrl+x = markdown.bold`` with Markdown assist off: Ctrl+X
    neither bolds nor cuts; the selection and the clipboard stay."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    settings = "markdown-assist = off\nkeybind = ctrl+x = markdown.bold\n"
    with keys_app("bold-cut", engine, mode, settings) as app:
        call = app.client.call
        two = "first line\nsecond line"
        call("editor.set", {"text": two})
        window = focus_native(app)
        native(window, "ctrl+a")
        wait_for(lambda: call("editor.get")["selected_text"] == two,
                 description="Ctrl+A selects the note")
        call("clipboard.write", {"text": "sentinel"})
        native(window, "ctrl+x")
        settle()
        state = call("editor.get")
        assert (state["text"], state["selected_text"]) == (two, two)
        assert call("clipboard.read") == "sentinel"


@pytest.mark.native_x11
def test_native_a_field_opened_with_mod_held_pastes(engine, mode) -> None:  # noqa: F811
    """Hold Mod, press F to open Find, then V without letting go: the new
    field never heard Mod go down, and pastes all the same."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    with keys_app("held", engine, mode) as app:
        call = app.client.call
        call("editor.set", {"text": "note"})
        window = focus_native(app)
        call("clipboard.write", {"text": "clip"})
        xdo("keydown", "--window", window, mode)
        try:
            xdo("key", "--window", window, "f")
            wait_for(lambda: call("focus.get")["named"] == ["find-query"],
                     description="Mod+F opens Find")
            xdo("key", "--window", window, "v")
            wait_for(lambda: call("editor.get")["find_query"] == "clip",
                     description="Mod+V pastes in the new field")
        finally:
            xdo("keyup", "--window", window, mode)
        assert text(app) == "note"


def reloaded(app: WrenApp, mode: str) -> str:
    """Switch the command key from *mode* by editing ``config.conf``, which
    the window reads again when it gets the keyboard back; returns the new
    mode."""
    target = other(mode)
    path = app.settings_path
    text = path.read_text(encoding="utf-8")
    path.write_text(text.replace(f"linux-modifier = {mode}", f"linux-modifier = {target}"),
                    encoding="utf-8")
    app.client.call("window.focus")
    return target


def switched(app: WrenApp, mode: str) -> str:
    """Switch the command key from *mode* while something holds the
    keyboard, as a reload of ``config.conf`` would; returns the new mode."""
    target = other(mode)
    assert app.client.call("command.run", {"id": SWITCH})["disposition"] == "completed"
    assert f"linux-modifier = {target}" in app.settings_path.read_text(encoding="utf-8")
    return target


@pytest.mark.native_x11
def test_native_the_switch_keeps_an_open_label_dialog_and_calendar(
        tmp_path, engine, mode) -> None:  # noqa: F811
    """A switch while a board label, a rename dialog or the calendar has the
    keyboard keeps its draft and its focus, and the next chord there is the
    new mode's. The label and the calendar switch by the command, the dialog
    by ``config.conf`` read again on focus."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    samples = copy_samples(tmp_path / "work")
    with keys_app("switch-open", engine, mode, "boards = on\ntasks = on\n",
                  workspace=samples, evaluation=False) as app:
        call = app.client.call
        focus = lambda: call("focus.get")["named"]  # noqa: E731
        # A board label.
        open_note(app, FLOW)
        window = focus_native(app)
        wait_for(lambda: focus() == ["note-editor"], description="the board has the keyboard")
        click(app, {"node": "3qrg4kv1"})
        assert board_key(app, "enter")["state"] == "editing"
        native(window, "ctrl+a" if mode == "ctrl" else "alt+a")
        typed(window, "draft")
        wait_for(lambda: (board(app).get("editing") or {}).get("text") == "draft",
                 description="typing in the label")
        mode = switched(app, mode)
        state = board(app)
        assert (state["state"], state["editing"]["text"]) == ("editing", "draft")
        assert focus() == ["note-editor"]
        native(window, f"{mode}+a")
        typed(window, "x")
        wait_for(lambda: (board(app).get("editing") or {}).get("text") == "x",
                 description="the new Mod+A selects the label")
        native(window, "Escape")
        wait_for(lambda: board(app)["state"] == "idle", description="Escape ends the label")

        # A rename dialog.
        open_note(app, NOTES)
        window = focus_native(app)
        assert call("command.run", {"id": "note.rename"})["disposition"] == "completed"
        wait_for(lambda: focus() == ["item-name"], description="the dialog has the keyboard")
        native(window, "ctrl+a")
        typed(window, "draft")
        wait_for(lambda: call("navigation.get")["dialog"]["name"] == "draft",
                 description="typing in the dialog")
        # The palette is shut while a dialog is open: the file switches.
        mode = reloaded(app, mode)
        assert call("navigation.get")["dialog"]["name"] == "draft"
        assert focus() == ["item-name"]
        native(window, f"{mode}+a")
        typed(window, "z")
        wait_for(lambda: call("navigation.get")["dialog"]["name"] == "z",
                 description="the new Mod+A selects the name")
        native(window, "Escape")
        wait_for(lambda: call("navigation.get")["dialog"] is None,
                 description="Escape closes the dialog")

        # The calendar, on the advanced engine.
        if engine != "advanced":
            return
        lines = text(app).split("\n")
        end = {"line": len(lines) - 1, "column": len(lines[-1].encode()), "selection": None}
        call("editor.cursor", end)
        call("editor.insert", {"text": "\n- [ ] task"})
        task = text(app)
        call("editor.cursor", {"line": len(lines), "column": 10, "selection": None})
        window = focus_native(app)
        native(window, f"{mode}+d")
        wait_for(lambda: focus() == ["calendar-field"], description="the calendar opens")
        typed(window, "fri")
        wait_for(lambda: call("tasks.calendar")["typed"] == "fri", description="typing in it")
        mode = switched(app, mode)
        assert call("tasks.calendar")["typed"] == "fri"
        assert focus() == ["calendar-field"]
        native(window, f"{mode}+a")
        typed(window, "mon")
        wait_for(lambda: call("tasks.calendar")["typed"] == "mon",
                 description="the new Mod+A selects the field")
        native(window, "Escape")
        wait_for(lambda: call("tasks.calendar") == {"open": False},
                 description="Escape closes the calendar")
        assert text(app) == task


@pytest.mark.native_x11
def test_native_the_keys_sheet_s_filter_and_its_own_key(tmp_path, engine, mode) -> None:  # noqa: F811
    """In the keys sheet's filter `what?` pasted filters, `?` pressed closes
    the sheet, and with ``keybind = mod+x = board.keys`` Mod+X closes it and
    cuts nothing."""
    assert not os.environ.get("WAYLAND_DISPLAY"), "Run native_x11 only in the X11 lane"
    samples = copy_samples(tmp_path / "work")
    with keys_app("sheet", engine, mode, "boards = on\nkeybind = mod+x = board.keys\n",
                  workspace=samples, evaluation=False) as app:
        call = app.client.call
        sheet = lambda: board(app)["keys_sheet"]  # noqa: E731
        open_note(app, FLOW)
        window = focus_native(app)
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the board has the keyboard")
        native(window, "shift+slash")
        wait_for(lambda: sheet() is not None, description="? opens the sheet")
        wait_for(lambda: call("focus.get")["named"] == ["palette-query"],
                 description="its filter has the keyboard")
        call("clipboard.write", {"text": "what?"})
        native(window, "ctrl+v")
        wait_for(lambda: (sheet() or {}).get("query") == "what?",
                 description="what? pasted filters")
        native(window, "shift+slash")
        wait_for(lambda: sheet() is None, description="? pressed closes the sheet")
        wait_for(lambda: call("focus.get")["named"] == ["note-editor"],
                 description="the board has the keyboard again")
        native(window, f"{mode}+x")
        wait_for(lambda: sheet() is not None, description="Mod+X opens the sheet")
        wait_for(lambda: call("focus.get")["named"] == ["palette-query"],
                 description="its filter has the keyboard")
        typed(window, "abc")
        wait_for(lambda: sheet()["query"] == "abc", description="typing in the filter")
        native(window, "ctrl+a")
        call("clipboard.write", {"text": "sentinel"})
        native(window, f"{mode}+x")
        wait_for(lambda: sheet() is None, description="Mod+X closes the sheet")
        assert call("clipboard.read") == "sentinel"
        assert call("session.get")["document"]["dirty"] is False
