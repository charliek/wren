"""Capture the main Wren screens for visual review against the mockup.

Builds the mockup's sample workspace in `target/scene-workspace/`, launches
the smoke-test build under the test harness, and writes one PNG per scene:

    cargo build --locked -p wren-iced --features smoke-test
    python3 tools/capture_scenes.py --theme wren-dark --out target/scenes

`--engine advanced` runs the editor on the owned engine; the PNGs then go
to `<out>/advanced/<theme>/` instead of `<out>/<theme>/`. Each `--setting
key=value` adds a `config.conf` line (`--setting editor-line-height=2`), and
`--label` names a further directory level for them. `--only markdown` shoots
just the editor scenes: the Markdown tour, the headings note, the long list
items, and all three again with the browser columns folded so the editor is
wider than its column. `--only spell` (with `--engine advanced`) turns spell
check on, seeds `words.txt` and shoots the spell-check acceptance note: its
marks with one hidden at the caret, the suggestions menu under a word, the
menu with no suggestions, and the menu above a word on the last visible row.
`--only tasks` opens the task fixture workspace
(`tools/wrentest/fixtures/tasks/`) on the advanced engine, unless `--engine` or
`--setting editor-engine=…` says otherwise, with `tasks = on` and today pinned
to 19 November 2026, shoots its task lines, the task colours (with a caret,
a selection, and two days later), the emoji spike's note (with a caret
and a selection on its glyphs), the line commands, the status bar and, on
the advanced engine, the task, status, priority and spelling menus, then
the task view and its ways in (the header counts, a row's task actions,
the hidden marks and the message with Undo), and
writes to
`<out>/tasks/<engine>-<renderer>-<display>-<fonts>/<theme>/`, the setup a task
scene depends on; `--fonts` names the font setup instead of the check for Noto
Color Emoji.
`--only task-list` opens the summary view's fixture
(`tools/wrentest/fixtures/tasks/list/`, two groups, with `tasks-me = sam,
sam.lee`) on the advanced engine with `tasks = on` and the same pinned day,
and shoots the view of the workspace's tasks: all of them, narrowed by a
status chip, grouped by due date and by owner, an empty list with its hint,
and the scope menu open; the PNGs go where `--only tasks` writes them.
`--only boards` opens the board sample workspace
(`tools/wrentest/fixtures/boards/`) with `boards = on` and shoots the board
scenes, writing the exports `board-flow.svg` and `board-flow-export.png` beside
them; the last of them select, drag a marquee, arm a tool, open a label and
make an empty board, through the harness's `board.*` requests.

On Linux run it inside `tools/run_display.py x11 -- ...` for a private
display. On macOS it uses the logged-in window server directly.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools.wrentest.boards import SAMPLES as BOARD_SAMPLES  # noqa: E402
from tools.wrentest.boards import ACCEPTANCE, conflict_board, script_steps  # noqa: E402
from tools.wrentest.ui import WrenApp, wait_for  # noqa: E402

# Every construct the highlighter draws, for the `markdown` scene.
MARKDOWN = """---
title: Markdown tour
tags: [demo]
---
# Markdown tour
## Lists and tasks
- Plan the **API migration** with *Bob*
  - Nested item with `inline code`
    1. Ordered child
- [x] Shipped the [design notes](https://example.com/notes)
  - [ ] Open follow-up
  - [x] Done nested ~~draft~~
> Quoted text with a [link](https://wren.dev)
> > Nested quote

Selection crosses

blank lines and ~~struck~~ text.

**[Bold](https://x.io/a_b)** and https://x.io/Foo_(bar).
```sh
cargo test
```
~~~
tilde fence
~~~
    indented code
See https://example.com for more.
"""
# The selection runs from "crosses" (line 15) into line 17, leaving prose
# between the cursor's line and the first fence so the code band shows apart.
MARKDOWN_SELECTION = ({"line": 15, "column": 10}, {"line": 17, "column": 5})

# Every heading level, one long enough to wrap, and markup around them.
HEADINGS = """# A first-level heading that runs long enough to wrap onto a second row
Body text under it, with `code` and a [link](https://wren.dev).
## Second level ##
- A list item between headings
### Third level
#### Fourth level
##### Fifth
###### Sixth
#
> # Quoted heading
Last line of prose.
"""
# The cursor on the wrapped heading's second row, then a selection from the
# middle of the heading into the body.
HEADINGS_CURSOR = {"line": 0, "column": 60}
HEADINGS_SELECTION = ({"line": 0, "column": 40}, {"line": 2, "column": 6})

# Wrapped list items, tasks, headings in items, and quotes, for the
# `hanging-indent` scene. The blank lines keep each block from running on
# lazily into the last.
LONG_ITEMS = """# Long items

- A bullet item long enough to wrap onto a second row and then a third, so each continuation row lines up under the text and not under the dash
  - A nested item that also runs long enough to wrap, hanging under its own content column one level in
    1. An ordered child whose wrapped rows sit under the text after its number, not under the digit
- [ ] An open task whose wrapped rows hang under its box, at the content column, where a plain item's text starts
- [x] A finished task that wraps as well, struck through on every row it covers

10. A two-digit number moves the content column right, and the continuation rows move with it

- # A first-level heading inside a list item, long enough to wrap under its text
- ## A second-level heading in the next item hangs its taller rows the same way

> A quoted paragraph long enough to wrap: its continuation rows start after the quote marker and its space, not at the edge
> - A list inside a quote hangs under the item's text, past both the quote marker and the dash

Plain prose has no container, so when it wraps its rows start at the left edge of the editor as before.
"""
# The selection runs from the first item's second row into the nested item.
LONG_ITEMS_SELECTION = ({"line": 2, "column": 90}, {"line": 3, "column": 40})


SAMPLE = {
    "1-1s/Alice Chen/2026-11-04.md": (
        "# 1:1 — Alice Chen · Nov 4\n\n## Agenda\n"
        "- API migration: cutover plan and **rollback** window\n"
        "- [ ] Review promo packet draft\n- [x] Share Q4 roadmap\n\n"
        "## Notes\nAlice wants to own the read-path cutover.\n"
        "She asked about the promo timeline; follow up next week.\n"
    ),
    "1-1s/Alice Chen/2026-10-28.md": "# 1:1 — Alice Chen · Oct 28\n\n- Hiring loop feedback\n",
    "1-1s/Alice Chen/2026-10-21.md": "# 1:1 — Alice Chen · Oct 21\n\n- Promo criteria walkthrough\n",
    "1-1s/Bob Ruiz/2026-11-03.md": "# 1:1 — Bob Ruiz\n\n- On-call load\n",
    "1-1s/Priya Nair/2026-11-02.md": "# 1:1 — Priya Nair\n\n- Design review\n",
    "1-1s/Team/Team sync.md": "# Team sync\n",
    "Projects/API Migration/Design notes.md": "# Design notes\n\nRead path first.\n",
    "Projects/API Migration/Cutover plan.md": "# Cutover plan\n",
    "Projects/Planning/Quarterly planning.md": "# Quarterly planning\n",
    "Inbox/Triage/scratch.md": "scratch\n",
    "Inbox/Triage/Markdown tour.md": MARKDOWN,
    "Inbox/Triage/Headings.md": HEADINGS,
    "Reference/Promotions/Promo rubric.md": "# Promo rubric\n\nScope, impact, and influence.\n",
}


def runbook() -> str:
    """A note long enough that the outline scrolls the editor."""
    parts = [
        ("Preparation", ["Freeze window", "Stakeholders"]),
        ("Cutover", ["Read path", "Write path", "Verification"]),
        ("Rollback", ["Triggers", "Steps"]),
        ("Follow-up", []),
    ]
    lines = ["# Cutover runbook", "", "The API migration, step by step.", ""]
    for part, children in parts:
        lines += [f"## {part}", ""]
        lines += [f"- {part} item {n}: check the dashboards" for n in range(1, 7)]
        lines.append("")
        for child in children:
            lines += [f"### {child}", ""]
            lines += [f"{child} note {n}." for n in range(1, 6)]
            lines.append("")
    return "\n".join(lines)


SAMPLE["Projects/API Migration/Cutover runbook.md"] = runbook()
SAMPLE["Inbox/Triage/Long items.md"] = LONG_ITEMS

# A section with no groups yet, for the empty-state scenes.
EMPTY_SECTION = "Archive"


@contextmanager
def scene_workspaces() -> Iterator[Path]:
    """An emptied `target/scene-workspace/`. The header draws the workspace
    path, so a fixed one lets two captures of one build match byte for byte."""
    root = ROOT / "target" / "scene-workspace"
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    yield root


def build_workspace(root: Path) -> None:
    (root / EMPTY_SECTION).mkdir(parents=True)
    for relative, text in SAMPLE.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    # Stagger modification times so "modified" order matches the mockup. Each
    # sits half an hour from an hour boundary, so its "h ago" label holds for
    # the whole capture.
    now = time.time()
    for offset, relative in enumerate(SAMPLE):
        mtime = now - (offset + 1.5) * 3600
        os.utime(root / relative, (mtime, mtime))


def open_note(client, name: str) -> None:
    note = next(row for row in client.call("session.get")["notes"]
                if row["path"].endswith(name))
    client.call("core.apply", {"op": "open_note", "note": note["id"]})


def editor_scenes(client, shot_here, suffix: str = "") -> None:
    """The Markdown tour, the headings note, and the long list items, selected,
    with the editor focused."""
    open_note(client, "Markdown tour.md")
    anchor, position = MARKDOWN_SELECTION
    client.call("editor.cursor", {**position, "selection": anchor})
    client.call("editor.focus")
    shot_here(f"markdown{suffix}")
    open_note(client, "Headings.md")
    client.call("editor.cursor", {**HEADINGS_CURSOR, "selection": None})
    client.call("editor.focus")
    shot_here(f"headings{suffix}")
    anchor, position = HEADINGS_SELECTION
    client.call("editor.cursor", {**position, "selection": anchor})
    shot_here(f"headings-selected{suffix}")
    # Wrapped rows hang under their item's content (advanced engine).
    open_note(client, "Long items.md")
    anchor, position = LONG_ITEMS_SELECTION
    client.call("editor.cursor", {**position, "selection": anchor})
    client.call("editor.focus")
    shot_here(f"hanging-indent{suffix}")


def folded_editor_scenes(client, shot_here) -> None:
    """The editor scenes with Sections and Groups folded away."""
    for _ in range(3):
        client.call("command.run", {"id": "navigation.column_left"})
    for _ in range(3):
        client.call("command.run", {"id": "navigation.column_right"})
    editor_scenes(client, shot_here, "-wide")
    client.call("column.open", {"column": "sections"})


SPELL_NOTE = Path(__file__).resolve().parent / "wrentest" / "fixtures" / "spell.md"
SPELL_WORDS = "catppuccin\n"
# The acceptance note's nine marks with `catppuccin` in the word list.
SPELL_MARK_COUNT = 9
# A word no dictionary suggests anything for, for the empty menu.
NO_SUGGESTIONS = "xqzvkpw"
# Enough correctly spelled rows to push the last one to the bottom of the view.
FILLER = "".join(f"\n- Row {n} of a list long enough to fill the view" for n in range(1, 61))


def spell_scenes(client, shot_here) -> None:
    """The acceptance note's marks and the suggestions menu in three places."""
    wait_for(lambda: client.call("editor.spell")["ready"], timeout=10,
             description="the dictionary loads")
    open_note(client, "Spell chek plan.md")
    text = client.call("editor.get")["text"]
    lines = text.split("\n")
    client.call("editor.focus")

    def caret(line: int, column: int) -> None:
        client.call("editor.cursor", {"line": line, "column": column, "selection": None})

    def column_after(line: int, word: str) -> int:
        return lines[line].index(word) + len(word)

    def marks() -> list[dict]:
        return client.call("editor.spell")["marks"]

    def open_menu(word: str, *, empty: bool = False) -> dict:
        client.call("shortcut", {"id": "editor.spell_suggest"})
        menu = wait_for(lambda: (state := client.call("editor.spell")["menu"])
                        and state["bounds"] and state, timeout=10,
                        description=f"the menu under {word!r} is drawn")
        assert menu["word"] == word and bool(menu["items"]) != empty, menu
        return menu

    def close_menu() -> None:
        client.call("editor.key", {"key": "escape", "mods": []})
        assert client.call("editor.spell")["menu"] is None

    # The caret at the end of `apear` hides its mark (rule 4.3.3).
    caret(5, column_after(5, "apear"))
    wait_for(lambda: len(marks()) == SPELL_MARK_COUNT
             and [m["shown"] for m in marks() if m["word"] == "apear"] == [False],
             timeout=10, description="nine marks, the one at the caret hidden")
    shot_here("spell-marks")

    caret(6, column_after(6, "replacments") - 2)
    open_menu("replacments")
    shot_here("spell-menu")
    close_menu()

    # A word with no suggestions, typed before the quote's full stop.
    quote = next(index for index, line in enumerate(lines) if "misteak." in line)
    caret(quote, column_after(quote, "misteak"))
    client.call("editor.insert", {"text": f" {NO_SUGGESTIONS}"})
    open_menu(NO_SUGGESTIONS, empty=True)
    shot_here("spell-menu-empty")
    close_menu()
    client.call("editor.undo")
    assert client.call("editor.get")["text"] == text

    # Rows of filler after the note leave its last row, and the typo at the
    # caret, at the bottom of the view, so the menu opens above the word.
    caret(len(lines) - 1, len(lines[-1]))
    client.call("editor.insert", {"text": FILLER + "\n\nThe last row ends in a typo: wrogn"})
    # Inside the word, so its mark shows under the tint.
    client.call("editor.action", {"kind": "move", "motion": "left"})
    client.call("editor.action", {"kind": "move", "motion": "left"})
    word = wait_for(lambda: next((m for m in marks() if m["word"] == "wrogn" and m["shown"]),
                                 None), timeout=10, description="the typo's mark")
    menu = open_menu("wrogn")
    assert menu["bounds"]["y"] + menu["bounds"]["height"] <= word["rects"][0]["y"], (menu, word)
    shot_here("spell-menu-above")
    close_menu()


TASKS_FIXTURE = Path(__file__).resolve().parent / "wrentest" / "fixtures" / "tasks" / "workspace"
# The summary view's fixture (item 10.3): two groups, `tasks-me` names, a
# task with two owners, finished work with and without stamps.
TASK_LIST_FIXTURE = TASKS_FIXTURE.parent / "list"
TASK_LIST_SETTINGS = "tasks-me = sam, sam.lee\n"
# Every task scene pins the design page's day, a Thursday.
TASKS_TODAY = "2026-11-19"


def display_server() -> str:
    """The display server the app will open its window on. winit picks
    Wayland whenever `WAYLAND_DISPLAY` is set and not empty, X11 otherwise
    (Wren forces neither); `WAYLAND_SOCKET` never reaches the app, since the
    harness passes only `WAYLAND_DISPLAY` and `DISPLAY` through."""
    if sys.platform == "darwin":
        return "macos"
    return "wayland" if os.environ.get("WAYLAND_DISPLAY") else "x11"


def last_value(settings: str, key: str) -> str | None:
    """The value the last `key` line of `settings` gives, as Wren reads it."""
    value = None
    for line in settings.splitlines():
        name, separator, rest = line.partition("=")
        if separator and not line.lstrip().startswith("#") and name.strip() == key:
            value = rest.strip().strip('"')
    return value


def tasks_out(out: Path, engine: str, fonts: str | None) -> Path:
    """`<out>/tasks/<engine>-<renderer>-<display>-<fonts>`: a task scene
    depends on the renderer, the display server and whether an emoji font
    is installed, so each setup keeps its own PNGs."""
    renderer = os.environ.get("ICED_BACKEND", "default")
    display = display_server()
    if fonts is None:
        if sys.platform == "darwin":
            fonts = "system"
        else:
            try:
                families = subprocess.run(["fc-list", ":", "family"], capture_output=True,
                                          text=True, check=True).stdout
            except (OSError, subprocess.CalledProcessError):
                families = ""
            fonts = "noto-emoji" if "Noto Color Emoji" in families else "no-emoji"
    return out / "tasks" / f"{engine}-{renderer}-{display}-{fonts}"


def task_scenes(client, shot_here, engine: str, resize) -> None:
    """The note of task lines, with the pinned date in force, on `engine`.
    `resize` sizes the window (``WIDTHxHEIGHT``); X11 only."""
    state = client.call("tasks.get")
    assert state["tasks"] is True and state["today"] == TASKS_TODAY, state
    assert state["engine"] == engine, (state["engine"], engine)
    open_note(client, "Task lines.md")
    client.call("editor.cursor", {"line": 2, "column": 21, "selection": None})
    client.call("editor.focus")
    shot_here("task-lines")
    colour_scenes(client, shot_here)
    emoji_scenes(client, shot_here)
    command_scenes(client, shot_here)
    footer_scenes(client, shot_here, resize)
    form_scenes(client, shot_here, resize)
    if engine == "advanced":
        menu_scenes(client, shot_here, resize)
        calendar_scenes(client, shot_here, resize)
        suggestion_scenes(client, shot_here)
    view_scenes(client, shot_here)
    entry_scenes(client, shot_here)


def entry_scenes(client, shot_here) -> None:
    """The ways into the task view (discovery/tasks-build.md item 10.2),
    from the Emoji group, whose tasks are all due later while the rest of
    the workspace has overdue ones: the header counts (the Notes one not
    red, the others red) and then hidden; a group's actions with the task
    rows; the Footer group hidden, with its mark and the message with Undo;
    and the open note's tasks hidden, with its "tasks off" mark. Each is
    undone, so the workspace is left as it was."""
    open_note(client, "Emoji spike.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Emoji"),
             timeout=10, description="the emoji note opens")
    client.call("editor.focus")
    headers = client.call("tasks.entry")["headers"]
    assert headers["notes"]["overdue"] == 0 and headers["sections"]["overdue"] > 0, headers
    shot_here("entry-counts")
    client.call("command.run", {"id": "view.toggle_task_counts"})
    client.call("tasks.entry", {"dismiss": True})
    assert client.call("tasks.entry")["counts"] is False
    shot_here("entry-counts-off")
    client.call("command.run", {"id": "view.toggle_task_counts"})
    client.call("tasks.entry", {"dismiss": True})
    groups = {row["path"]: row["id"] for row in client.call("session.get")["groups"]}
    client.call("palette.open", {"mode": "item_actions",
                                 "target": {"group": groups["Tasks/Footer"]}})
    shot_here("entry-group-actions")
    rows = client.call("navigation.get")["modal"]["results"]
    hide = next(index for index, row in enumerate(rows)
                if row["key"]["item_action"]["command"] == "group.hide_tasks")
    client.call("palette.activate", {"index": hide})
    state = client.call("tasks.entry")
    assert state["marks"]["groups"] == ["Tasks/Footer"] and state["notice"]["undo"], state
    shot_here("entry-group-hidden")
    client.call("tasks.entry", {"undo": True})
    notes = {row["path"]: row["id"] for row in client.call("session.get")["notes"]}
    emoji = "Tasks/Emoji/Emoji spike.md"
    client.call("palette.open", {"mode": "item_actions", "target": {"note": notes[emoji]}})
    rows = client.call("navigation.get")["modal"]["results"]
    hide = next(index for index, row in enumerate(rows)
                if row["key"]["item_action"]["command"] == "note.hide_tasks")
    client.call("palette.activate", {"index": hide})
    state = client.call("tasks.entry")
    assert state["marks"]["notes"] == [emoji] and state["notice"]["undo"], state
    shot_here("entry-note-hidden")
    client.call("tasks.entry", {"undo": True})
    assert client.call("tasks.entry")["marks"] == {"groups": [], "notes": []}


def view_scenes(client, shot_here) -> None:
    """The task view's surface (discovery/tasks-build.md item 10.1): the
    view of a group in the editor pane, over the note it was opened from,
    with Back lit in its header; then the view of the whole workspace,
    opened from the first. Back twice returns to the note."""
    open_note(client, "Task lines.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Task lines"),
             timeout=10, description="the task lines note opens")
    state = client.call("tasks.view", {"open": {"scope": "group", "path": "Tasks/Baseline"}})
    assert state["open"] and (state["scope"], state["label"]) == ("group", "Baseline"), state
    assert state["keys"] and client.call("navigation.get")["can_back"], state
    shot_here("task-view")
    state = client.call("tasks.view", {"open": {"scope": "workspace"}})
    assert state["open"] and state["scope"] == "workspace", state
    shot_here("task-view-workspace")
    for _ in range(2):
        client.call("command.run", {"id": "navigation.back"})
    assert not client.call("tasks.view")["open"]
    assert client.call("navigation.get")["surface"] == "text"


def task_list_scenes(client, shot_here) -> None:
    """The summary view's list (discovery/tasks-build.md item 10.3), of
    the whole workspace: every task, then narrowed by the Doing chip,
    grouped by due date, grouped by owner (Me first, a task with two owners
    under each), an empty list with the hint that says how to widen it, and
    the scope menu ("Show tasks in") open under its chip. Each filter is
    put back after its scene, so `.wren/tasks.toml` ends with no view."""
    def view(**params):
        return client.call("tasks.view", {**params, "list": True})

    def click(**target):
        return view(click=target)["list"]

    state = view(open={"scope": "workspace"})
    assert state["open"] and state["keys"], state
    assert len(state["list"]["rows"]) == 12, state["list"]
    shot_here("task-list")
    click(control="chip:doing")
    assert [row["status"] for row in view()["list"]["rows"]] == ["doing"]
    shot_here("task-list-narrowed")
    click(control="chip:doing")
    for group, row in (("due", "Due date"), ("owner", "Owner")):
        click(control="group")
        assert view()["list"]["menu"]["for"] == "group"
        click(menu=row)
        assert view()["list"]["filters"]["group"] == group
        shot_here(f"task-list-by-{group}")
    click(control="group")
    click(menu="Note")
    click(control="priority")
    click(menu="Lowest")
    click(control="chip:done")
    state = view()["list"]
    assert not state["rows"] and state["empty"], state
    shot_here("task-list-empty")
    click(control="chip:done")
    click(control="priority")
    click(menu="Any priority")
    click(control="scope")
    menu = view()["list"]["menu"]
    assert menu["for"] == "scope" and menu["title"] == "Show tasks in", menu
    shot_here("task-list-scope-menu")
    view(key={"key": "escape", "mods": []})
    assert view()["list"]["menu"] is None
    assert view()["list"]["filters"] == {"statuses": [], "owner": "anyone", "group": "note",
                                          "hide_old": True, "due": None, "priority": None}


def suggestion_scenes(client, shot_here) -> None:
    """Task suggestions (discovery/tasks-build.md item 5.1), typed at the
    end of a task line: `to` offers the dates, `hi` the priorities, and `@`
    the owners of the workspace's task lines, each drawn tentative (Enter
    would continue the list), and the dates again once Down has picked
    Tomorrow, which Enter would then write. Each word is taken back with
    Backspace, which leaves the note as it was."""
    open_note(client, "Task lines.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Task lines"),
             timeout=10, description="the task lines note opens")
    original = client.call("editor.get")["text"]
    line = 2
    text = original.split("\n")[line]
    assert text == "- [ ] Open alpha task", text
    client.call("editor.cursor", {"line": line, "column": len(text.encode()), "selection": None})
    client.call("editor.focus")
    wait_for(lambda: client.call("tasks.owners")["current"], timeout=10,
             description="the owners are read")

    def offered(word: str, labels: list[str], name: str, confirmed: str | None = None) -> None:
        for character in f" {word}":
            client.call("editor.key", {"key": "space" if character == " " else character})
        popover = client.call("editor.get")["popover"]
        assert popover is not None and popover["mode"] == "suggest", popover
        shown = [row["label"] for row in popover["rows"]]
        assert shown[:len(labels)] == labels, shown
        assert popover["takes_enter"] is False and popover["footer"], popover
        shot_here(name)
        if confirmed:
            client.call("editor.key", {"key": "down"})
            popover = client.call("editor.get")["popover"]
            assert popover["takes_enter"] is True and popover["selected"] == 1, popover
            shot_here(confirmed)
        for _ in f" {word}":
            client.call("editor.key", {"key": "backspace"})
        assert client.call("editor.get")["text"] == original

    offered("to", ["Today", "Tomorrow"], "task-suggest-dates", "task-suggest-dates-confirmed")
    offered("hi", ["High priority", "Highest priority"], "task-suggest-priority")
    offered("@", ["@alice.chen", "@bob"], "task-suggest-owners")


def form_scenes(client, shot_here, resize) -> None:
    """The task form (discovery/tasks-build.md item 6.1), on either engine:
    a new task on a blank line with its description, due date and priority
    filled in; a full task (doing, an owner, a priority, a due date) being
    edited, its "Writes" line showing the change; and the calendar open
    under the form's due field. Each is cancelled with Escape, which writes
    nothing."""
    if display_server() == "x11":
        resize("1280x800")
    open_note(client, "Status bar.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Status bar"),
             timeout=10, description="the status bar note opens")
    original = client.call("editor.get")["text"]
    lines = original.split("\n")

    def opened(line: int, description: str) -> dict:
        client.call("editor.cursor", {"line": line, "column": 0, "selection": None})
        client.call("editor.focus")
        client.call("shortcut", {"id": "task.edit"})
        wait_for(lambda: client.call("focus.get")["named"] == ["task-form-description"],
                 timeout=10, description=f"the form's description has the keyboard ({description})")
        return client.call("tasks.form")

    def close() -> None:
        while client.call("tasks.form")["open"]:
            client.call("editor.key", {"key": "escape"})
        assert client.call("editor.get")["text"] == original

    blank = lines.index("", 3)
    state = opened(blank, "new")
    assert (state["kind"], state["title"]) == ("new", "New task"), state
    for field, text in [("description", "Call the bank about the card"), ("due", "fri")]:
        client.call("tasks.form_type", {"field": field, "text": text})
    for _ in range(3):
        client.call("editor.key", {"key": "tab"})
    client.call("editor.key", {"key": "h"})
    state = client.call("tasks.form")
    assert state["writes"] == "- [ ] Call the bank about the card ⏫ 📅 2026-11-20", state
    assert state["focus"] == "priority", state
    shot_here("task-form-new")
    close()

    full = 2
    assert lines[full].startswith("- [/] Draft the cutover runbook"), lines[full]
    state = opened(full, "edit")
    assert (state["kind"], state["owners"]) == ("edit", ["alice.chen"]), state
    client.call("tasks.form_click", {"status": "Done"})
    client.call("tasks.form_type", {"field": "due", "text": "+1w"})
    client.call("tasks.form_type", {"field": "owners", "text": "sam"})
    client.call("tasks.form_click", {"target": "calendar"})
    client.call("editor.key", {"key": "escape"})
    for _ in range(2):
        client.call("editor.key", {"key": "tab"})
    state = client.call("tasks.form")
    assert state["focus"] == "owners", state
    shot_here("task-form-edit")
    client.call("tasks.form_type", {"field": "owners", "text": ""})
    client.call("tasks.form_click", {"target": "calendar"})
    wait_for(lambda: client.call("focus.get")["named"] == ["calendar-field"],
             timeout=10, description="the calendar over the form has the keyboard")
    assert client.call("tasks.form")["calendar"]["selected"] == "2026-11-26"
    shot_here("task-form-calendar")
    close()


def calendar_scenes(client, shot_here, resize) -> None:
    """The calendar (discovery/tasks-build.md item 4.1): opened by
    `task.set_due` on a task with no due date, so on today; with `fri`
    typed, the selection moved to Friday; with a word that names no date;
    opened by a right-click on a due date, which it opens on, with Clear;
    and on X11, in macOS's default 1024×640 window, flipped above a dated
    task near the bottom, clear of the footer. Each is closed with Escape,
    which writes nothing."""
    if display_server() == "x11":
        resize("1280x800")
    open_note(client, "Task lines.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Task lines"),
             timeout=10, description="the task lines note opens")
    original = client.call("editor.get")["text"]
    lines = original.split("\n")

    def opened(description: str) -> dict:
        wait_for(lambda: client.call("focus.get")["named"] == ["calendar-field"],
                 timeout=10, description=f"the calendar's field has the keyboard ({description})")
        return client.call("tasks.calendar")

    def close() -> None:
        client.call("editor.key", {"key": "escape"})
        assert client.call("tasks.calendar") == {"open": False}
        assert client.call("editor.get")["text"] == original

    line = 2
    assert lines[line] == "- [ ] Open alpha task", lines[line]
    client.call("editor.cursor", {"line": line, "column": len(lines[line].encode()),
                                  "selection": None})
    client.call("editor.focus")
    client.call("shortcut", {"id": "task.set_due"})
    state = opened("empty")
    assert (state["selected"], state["typed"], state["invalid"]) == (TASKS_TODAY, "", False), state
    shot_here("task-calendar-empty")
    for character in "fri":
        client.call("editor.key", {"key": character})
    state = client.call("tasks.calendar")
    assert (state["selected"], state["invalid"]) == ("2026-11-20", False), state
    shot_here("task-calendar-typed")
    client.call("tasks.calendar_type", {"text": "blorp"})
    assert client.call("tasks.calendar")["invalid"] is True
    shot_here("task-calendar-invalid")
    close()

    due = 14
    assert lines[due] == "- [ ] Due mike task 📅 2026-11-20", lines[due]
    client.call("editor.cursor", {"line": 1, "column": 0, "selection": None})
    client.call("editor.focus")
    client.call("editor.right_click",
                {"line": due, "column": len(lines[due][:lines[due].index("2026")].encode())})
    state = opened("from the date")
    assert (state["line"], state["selected"], state["clear"]) == (due, "2026-11-20", True), state
    shot_here("task-calendar-date")
    close()

    if display_server() == "x11":
        resize("1024x640")
        low = 17
        assert lines[low].startswith("- [ ] Every papa token"), lines[low]
        client.call("editor.cursor", {"line": low, "column": 6, "selection": None})
        client.call("editor.focus")
        client.call("shortcut", {"id": "task.set_due"})
        state = opened("flipped")
        anchor = client.call("editor.bounds", {
            "line": low, "start": state["anchor"]["range"][0],
            "end": state["anchor"]["range"][1]})
        card = state["bounds"]
        assert card["y"] + card["height"] <= anchor["y"], ("flipped above", card, anchor)
        assert card["y"] >= 0 and card["y"] + card["height"] <= 640 - 30, card
        shot_here("task-calendar-flipped")
        close()
        resize("1280x800")


def menu_scenes(client, shot_here, resize) -> None:
    """The task menus (discovery/tasks-build.md item 3.4), each with the
    task's status or priority marked: the task menu for a right-click on a
    doing task's description, the status menu on its box, and the priority
    menu on its glyph; then a misspelled word on a task line, whose
    right-click opens spelling's menu first. The window is first put back
    to its own size, which the footer scenes changed on X11, and spell
    check is turned on for the last scene alone."""
    if display_server() == "x11":
        resize("1280x800")
    open_note(client, "Status bar.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Status bar"),
             timeout=10, description="the status bar note opens")
    line = 2
    text = client.call("editor.get")["text"].split("\n")[line]
    assert text == "- [/] Draft the cutover runbook @alice.chen ⏫ 📅 2026-11-23", text
    client.call("editor.cursor", {"line": 1, "column": 0, "selection": None})
    client.call("editor.focus")
    # Spelling may be on already (`--setting spell-check=on`): off for the
    # task menus, which a misspelled `runbook` would open spelling's menu
    # over, and as it was once the scenes are done.
    spelling_was_on = client.call("editor.spell")["enabled"]
    if spelling_was_on:
        client.call("command.run", {"id": "editor.toggle_spell_check"})

    def menu_at(token: str, rows: list[str], marked: str) -> None:
        column = len(text[:text.index(token)].encode())
        client.call("editor.right_click", {"line": line, "column": column})
        popover = wait_for(lambda: (state := client.call("editor.get")["popover"])
                           and state["bounds"] and state,
                           timeout=10, description=f"the menu under {token!r} is drawn")
        assert [row["label"] for row in popover["rows"]] == rows, popover
        assert [row["label"] for row in popover["rows"] if row["checked"]] == [marked], popover

    status = ["Open", "Doing", "Done", "Cancelled"]
    menu_at("runbook", status + ["Change due date…", "Priority…", "Edit task…"], "Doing")
    shot_here("task-menu")
    client.call("editor.key", {"key": "escape"})
    menu_at("[", status, "Doing")
    shot_here("task-menu-status")
    client.call("editor.key", {"key": "escape"})
    menu_at("⏫", ["Highest", "High", "Medium", "Low", "Lowest", "None"], "High")
    shot_here("task-menu-priority")
    client.call("editor.key", {"key": "escape"})

    client.call("command.run", {"id": "editor.toggle_spell_check"})
    wait_for(lambda: client.call("editor.spell")["ready"], timeout=20,
             description="the dictionary loads")
    original = client.call("editor.get")["text"]
    bank = 7
    assert original.split("\n")[bank] == "- [ ] Call the bank 📅 2026-11-20"
    client.call("editor.cursor", {"line": bank, "column": len("- [ ] Call the bank"),
                                  "selection": None})
    client.call("editor.insert", {"text": " acount"})
    client.call("editor.cursor", {"line": 1, "column": 0, "selection": None})
    wait_for(lambda: any(mark["word"] == "acount" for mark in client.call("editor.spell")["marks"]),
             timeout=10, description="the misspelling is marked")
    client.call("editor.right_click", {"line": bank, "column": len("- [ ] Call the bank a")})
    wait_for(lambda: (menu := client.call("editor.spell")["menu"]) and menu["bounds"],
             timeout=10, description="spelling's menu is drawn")
    assert client.call("editor.get")["popover"]["spelling"] is True
    shot_here("task-menu-spelling")
    client.call("editor.spell_event", {"event": "close"})
    client.call("editor.undo")
    assert client.call("editor.get")["text"] == original
    if not spelling_was_on:
        client.call("command.run", {"id": "editor.toggle_spell_check"})


def command_scenes(client, shot_here) -> None:
    """The task line commands (discovery/tasks-build.md item 3.3): the
    palette filtered to them with the caret on a task line, then an open
    task before and after `task.cycle_status` (doing), and after the toggle
    (done, with today's stamp). Both edits are undone after, so the scenes
    that follow see the note as it was."""
    open_note(client, "Status bar.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Status bar"),
             timeout=10, description="the status bar note opens")
    line = 3
    original = client.call("editor.get")["text"]
    before = original.split("\n")[line]
    assert before == "- [ ] Send the deck to finance @sam @priya 🔼 📅 2026-11-16", before
    client.call("editor.cursor", {"line": line, "column": len("- [ ] Send the deck".encode()),
                                  "selection": None})
    client.call("editor.focus")
    client.call("palette.open", {"mode": "commands"})
    rows = client.call("palette.query", {"query": "task"})["navigation"]["modal"]["results"]
    # The line commands: Show tasks (item 10.2) is the app's.
    tasks = [row for row in rows
             if row["group"] == "Tasks" and row["key"]["command"] != "tasks.show"]
    # The status and priority menus (item 3.4) and the calendar (item 4.1)
    # are the advanced engine's.
    menus = {"task.set_status", "task.set_priority", "task.set_due"}
    assert len(tasks) == 14 and all(row["enabled"] or row["key"]["command"] in menus
                                    for row in tasks), tasks
    shot_here("task-commands-palette")
    client.call("palette.close")
    client.call("editor.focus")
    shot_here("task-commands-before")
    client.call("command.run", {"id": "task.cycle_status"})
    after = client.call("editor.get")["text"].split("\n")[line]
    assert after == "- [/] Send the deck to finance @sam @priya 🔼 📅 2026-11-16", after
    shot_here("task-commands-cycled")
    client.call("command.run", {"id": "markdown.toggle_task"})
    after = client.call("editor.get")["text"].split("\n")[line]
    assert after == f"- [x] Send the deck to finance @sam @priya 🔼 📅 2026-11-16 ✅ {TASKS_TODAY}", after
    shot_here("task-commands-toggled")
    client.call("editor.undo")
    client.call("editor.undo")
    assert client.call("editor.get")["text"] == original


# The windows the tight, cutoff and narrow footer scenes size to; at these
# widths the browser columns take 624 px, so the editor's footer is the rest.
# In the tight one (516 px of footer) the task line has room for its first
# pieces only; the cutoff one's footer (446 px) is just wider than the 440 px
# below which the footer keeps only the position and the save status, as the
# narrow one's does.
TIGHT_WINDOW = "1140x560"
CUTOFF_WINDOW = "1070x560"
NARROW_WINDOW = "620x560"


def footer_scenes(client, shot_here, resize) -> None:
    """The status bar's task line (discovery/tasks-build.md item 2.3): the
    design's example on a doing task, an overdue task, and then, on X11, a
    window so narrow that the line gives way to the save status. The window
    is resized last, so the scenes before keep the platform's size. The
    tight window and the one just above the cutoff in between show the
    pieces that do not fit dropping whole, from the end."""
    open_note(client, "Status bar.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Status bar"),
             timeout=10, description="the status bar note opens")

    def footer_at(line: int) -> dict | None:
        client.call("editor.cursor", {"line": line, "column": 0, "selection": None})
        return client.call("tasks.get")["footer"]

    client.call("editor.focus")
    footer = footer_at(2)
    assert footer["text"] == "Doing · due Mon 23 Nov, in 4 days · ⏫ High · @alice.chen", footer
    shot_here("task-footer")
    footer = footer_at(3)
    assert footer["overdue"] and footer["text"] == \
        "Open · due Mon 16 Nov, 3 days overdue · 🔼 Medium · @sam @priya", footer
    shot_here("task-footer-overdue")
    # Focus mode folds the browser columns away, so the footer is wide on
    # any platform, macOS's small default window included.
    client.call("command.run", {"id": "view.focus"})
    footer_at(2)
    shot_here("task-footer-wide")
    client.call("command.run", {"id": "view.focus"})
    if display_server() != "x11":
        print(f"task-footer-narrow: skipped, resizing needs X11 ({display_server()})")
        return
    resize(TIGHT_WINDOW)
    footer_at(2)
    shot_here("task-footer-tight")
    resize(CUTOFF_WINDOW)
    shot_here("task-footer-cutoff")
    resize(NARROW_WINDOW)
    shot_here("task-footer-narrow")


def colour_scenes(client, shot_here) -> None:
    """Task colours (discovery/tasks-build.md item 2.2): due dates ahead,
    today, past and on no real day, owners, every priority, and a done and a
    cancelled task, with the caret on an overdue line; the same lines
    selected; then two days later, when the dates passed turn overdue."""
    open_note(client, "Due dates.md")
    wait_for(lambda: client.call("editor.get")["text"].startswith("# Due dates"),
             timeout=10, description="the due dates note opens")
    client.call("editor.cursor", {"line": 4, "column": 0, "selection": None})
    client.call("editor.focus")
    shot_here("task-colours")
    client.call("editor.cursor", {"line": 13, "column": 0,
                                  "selection": {"line": 2, "column": 0}})
    shot_here("task-colours-selection")
    client.call("editor.cursor", {"line": 4, "column": 0, "selection": None})
    client.call("tasks.set_today", {"date": "2026-11-21"})
    shot_here("task-colours-later")
    client.call("tasks.set_today", {"date": TASKS_TODAY})


def emoji_scenes(client, shot_here) -> None:
    """The emoji spike's note (discovery/tasks-build.md item 0.3): every task
    glyph bare and with U+FE0F in wrapped task lines, then the caret one
    Right past a glyph on a wrapped row, and a Shift+Right selection of one
    glyph. Each step is checked before its shot, with `assert` or without."""

    def expect(actual, wanted, what: str) -> None:
        if actual != wanted:
            raise RuntimeError(f"{what}: got {actual!r}, wanted {wanted!r}")

    open_note(client, "Emoji spike.md")
    lines = wait_for(lambda: (text := client.call("editor.get")["text"]).startswith("# Emoji")
                     and text, timeout=10, description="the emoji note opens").split("\n")

    def cluster_at(prefix: str, cluster: str, last: bool) -> tuple[int, int, int]:
        """The line starting with `prefix`, and the byte range of the first
        or last `cluster` in it."""
        line = next(i for i, text in enumerate(lines) if text.startswith(prefix))
        text = lines[line]
        found = text.rfind(cluster) if last else text.find(cluster)
        start = len(text[:found].encode("utf-8"))
        return line, start, start + len(cluster.encode("utf-8"))

    def at(line: int, column: int) -> dict:
        return {"line": line, "column": column}

    def step(key: str, mods: list[str]) -> dict:
        return client.call("editor.key", {"key": key, "mods": mods})

    client.call("editor.cursor", {"line": 0, "column": 0, "selection": None})
    client.call("editor.focus")
    shot_here("tasks-emoji")

    # The last line's last glyph, so the view scrolls to the lines the first
    # scene leaves below it. The window's size is the platform's, so check
    # that the glyph is on a wrapped row: Home there stops at the row's
    # start, past the line's first glyph, and not at the task's text.
    lowest = "\u23ec\ufe0f"
    line, start, end = cluster_at("- [ ] Lowest VS16", lowest, last=True)
    _, _, first_end = cluster_at("- [ ] Lowest VS16", lowest, last=False)
    client.call("editor.cursor", {"line": line, "column": start, "selection": None})
    expect(step("right", [])["cursor"], at(line, end), "one Right over the last U+23EC U+FE0F")
    row = step("home", [])["cursor"]
    if row["line"] != line or not first_end < row["column"] <= start:
        raise RuntimeError(f"the last U+23EC U+FE0F is not on a wrapped row: Home went "
                           f"to {row!r}, wanted line {line}, a column in "
                           f"{first_end + 1}..={start}")
    expect(client.call("editor.cursor", {"line": line, "column": end, "selection": None})
           ["cursor"], at(line, end), "the caret back after the last U+23EC U+FE0F")
    shot_here("tasks-emoji-caret")

    # The line has two; Shift+Right from the first selects the first.
    highest = "\U0001f53a\ufe0f"
    line, start, end = cluster_at("- [ ] Highest VS16", highest, last=False)
    client.call("editor.cursor", {"line": line, "column": start, "selection": None})
    selected = step("right", ["shift"])
    expect(selected["selected_text"], highest, "Shift+Right over the first U+1F53A U+FE0F")
    expect((selected["selection"], selected["cursor"]), (at(line, start), at(line, end)),
           "the selection's anchor and caret")
    shot_here("tasks-emoji-selection")


BOARD_GROUP = "Boards/Samples"


def build_board_workspace(root: Path) -> None:
    """The board samples, a board with conflict markers, and fixed
    modification times, so the notes list reads the same each capture."""
    shutil.copytree(BOARD_SAMPLES, root)
    (root / BOARD_GROUP / "Conflict.wren-board").write_text(conflict_board(), encoding="utf-8")
    now = time.time()
    names = sorted(path.name for path in (root / BOARD_GROUP).iterdir())
    for offset, name in enumerate(names):
        mtime = now - (offset + 1.5) * 3600
        os.utime(root / BOARD_GROUP / name, (mtime, mtime))


def board_scenes(client, shot_here, out: Path, workspace: Path,
                 settings: tuple[Path, str] | None = None) -> None:
    """discovery/boards.md §6.3, Phase 1. `settings` is the app's
    `config.conf` and the text it was started with, for the scenes that
    change a setting and put it back."""

    def open_board(name: str, fit: bool = False) -> None:
        open_note(client, f"{name}.wren-board")
        client.call("editor.focus")
        if fit:
            # A board that is already open keeps the view it had, which on a
            # small window need not show every node.
            client.call("command.run", {"id": "board.zoom.fit"})

        def fitted():
            state = client.call("board.get")
            # A small board shows whole from its origin before its first fit.
            if state["surface"] != "board" or state["fit_pending"]:
                return False
            placed = client.call("board.bounds")
            return all(node["visible"] == node["bounds"] for node in placed["nodes"])

        wait_for(fitted, timeout=10, description=f"{name} fitted")

    open_board("Checkout flow")
    shot_here("board-flow")
    svg = out / "board-flow.svg"
    svg.unlink(missing_ok=True)
    client.call("board.export", {"format": "svg", "path": str(svg)})
    print(svg)
    # Beside the window's own `board-flow.png`, which it must not replace.
    png = out / "board-flow-export.png"
    png.unlink(missing_ok=True)
    client.call("board.export", {"format": "png", "path": str(png)})
    print(png)
    # The kind icon, with the notes list holding the keyboard.
    client.call("column.open", {"column": "notes"})
    shot_here("board-notes-list")
    client.call("column.open", {"column": "editor"})
    client.call("editor.focus")
    client.call("command.run", {"id": "font.reset"})
    for _ in range(4):
        client.call("command.run", {"id": "font.increase"})
    shot_here("board-zoom-in")
    client.call("command.run", {"id": "font.reset"})
    for _ in range(6):
        client.call("command.run", {"id": "font.decrease"})
    shot_here("board-zoom-out")
    open_board("Shapes")
    shot_here("board-shapes")
    open_board("Overlap")
    shot_here("board-overlap")
    open_note(client, "Conflict.wren-board")
    client.call("editor.focus")
    shot_here("board-raw")
    client.call("palette.open", {"mode": "search"})
    client.call("palette.query", {"query": "payment"})
    shot_here("board-search")
    client.call("palette.close")
    # The flow again, with the browser columns folded away.
    open_note(client, "Checkout flow.wren-board")
    for _ in range(3):
        client.call("command.run", {"id": "navigation.column_left"})
    for _ in range(3):
        client.call("command.run", {"id": "navigation.column_right"})
    client.call("command.run", {"id": "font.reset"})
    for _ in range(4):
        client.call("command.run", {"id": "font.decrease"})
    shot_here("board-wide")
    # The New note dialog with its kind pills, opened on Board.
    client.call("command.run", {"id": "note.new_board"})
    client.call("dialog.name", {"name": "Release plan"})
    shot_here("board-new-dialog")
    client.call("dialog.close")
    board_hint_scenes(client, shot_here, open_board)
    board_dock_scenes(client, shot_here, open_board, settings)
    board_style_bar_scenes(client, shot_here, open_board)
    board_text_size_scenes(client, shot_here, open_board)
    board_edit_scenes(client, shot_here, open_board)
    board_arrange_scenes(client, shot_here, open_board)
    board_style_scenes(client, shot_here)
    # Jump to node (§5.18): the palette beside the flow, fitted, and the
    # highlighted node selected and centred behind it. Last, so the view it
    # leaves moves no other scene.
    open_board("Checkout flow", fit=True)
    client.call("command.run", {"id": "board.jump"})
    client.call("palette.query", {"query": "pay"})
    shot_here("board-jump")
    client.call("palette.close")
    board_limits_scene(client, shot_here, workspace)
    client.call("column.open", {"column": "sections"})


# Values past the limits the scene draws within (§3.8, §4.12): a shape of
# no height, an anchor and a label position outside 0 to 1000, seventeen
# labels and a 65-character z on a connector, and a chain one level deeper
# than a board draws inside its parents. Each is drawn clamped, with an
# issue marker at its top-left corner.
LIMITS_BOARD = "".join(
    ["schema = 1\n",
     '\n[node.base]\ntext = "Base"\nkind = "shape"\nat = [0, 0]\nsize = [160, 80]\nz = "V"\n',
     '\n[node.flat]\ntext = "No height"\nkind = "shape"\nat = [320, 0]\nsize = [160, 0]\n'
     'z = "V"\n',
     '\n[node.right]\ntext = "Labels"\nkind = "shape"\nat = [320, 200]\nsize = [160, 80]\n'
     'z = "V"\n',
     '\n[node.c01]\ntext = "Chain"\nkind = "shape"\nat = [0, 360]\nsize = [200, 120]\n'
     'z = "V"\n']
    + [f'\n[node.c{level:02}]\nkind = "shape"\nat = [3, 3]\nparent = "c{level - 1:02}"\n'
       f'size = [100, 60]\nz = "V"\n' for level in range(2, 34)]
    + ['\n[edge.anchored]\nkind = "edge"\nfrom = { anchor = [2000, 500], node = "base" }\n'
       'to = { node = "flat" }\nlabels = [\n  { at = 5000, text = "past the end" },\n]\n'
       'z = "W"\n',
       '\n[edge.many]\nkind = "edge"\nfrom = { node = "base" }\nto = { node = "right" }\n'
       'labels = [\n' + "".join(f'  {{ at = {i * 60}, text = "{i}" }},\n' for i in range(17))
       + f']\nz = "{"W" * 65}"\n'])


def board_limits_scene(client, shot_here, workspace: Path) -> None:
    """A board whose values pass the limits, added last so that no other
    scene's notes list holds it."""
    path = workspace / BOARD_GROUP / "Limits.wren-board"
    path.write_text(LIMITS_BOARD, encoding="utf-8")

    def listed() -> bool:
        client.call("core.apply", {"op": "reconcile"})
        return any(row["path"] == f"{BOARD_GROUP}/Limits.wren-board"
                   for row in client.call("session.get")["notes"])

    wait_for(listed, timeout=10, interval=0.2, description="the limits board listed")
    # Not `open_board`: its fit check reads stored sizes, and a shape of no
    # height never shows a whole box.
    open_note(client, "Limits.wren-board")
    client.call("editor.focus")
    wait_for(lambda: client.call("board.get")["surface"] == "board", timeout=10,
             description="the limits board shown")
    client.call("command.run", {"id": "board.zoom.fit"})
    shot_here("board-limits")


def board_hint_scenes(client, shot_here, open_board) -> None:
    """discovery/board-controls.md §3.4 and §3.5: the footer's hint line
    with nothing selected, one shape and a tool armed, and in a narrow
    editor column; the keys sheet with one shape selected. Nothing here
    changes the board."""
    pack = "3qrg4kv1"

    def key(name: str, *mods: str) -> dict:
        return client.call("board.key", {"key": name, "mods": list(mods)})

    def hinted(first: str) -> None:
        wait_for(lambda: [part["words"] for part in client.call("board.get")["hint"]][:1]
                 == [first], timeout=5, description=f"the hint starting {first!r}")

    open_board("Checkout flow", fit=True)
    wait_for(lambda: client.call("focus.get")["named"] == ["note-editor"], timeout=5,
             description="the board focused")
    key("escape")
    hinted("add a shape")
    shot_here("board-hint-idle")
    # The sheet with nothing selected dims what needs a selection.
    client.call("command.run", {"id": "board.keys"})
    wait_for(lambda: client.call("board.get")["keys_sheet"] is not None, timeout=5,
             description="the keys sheet open")
    shot_here("board-keys-sheet")
    client.call("palette.query", {"query": "colour"})
    shot_here("board-keys-sheet-filtered")
    client.call("palette.close")
    wait_for(lambda: client.call("focus.get")["named"] == ["note-editor"], timeout=5,
             description="the board focused again")
    client.call("board.pointer", {"kind": "click", "at": {"node": pack}, "button": "left",
                                  "mods": []})
    hinted("next shape")
    shot_here("board-hint-shape")
    key("escape")
    key("r")
    hinted("click to place")
    shot_here("board-hint-armed")
    key("escape")
    # A narrow editor column: the hint keeps the parts that fit whole and
    # never pushes the zoom or the save status out.
    client.call("view.width", {"pane": "notes", "width": 720})
    client.call("editor.focus")
    client.call("board.pointer", {"kind": "click", "at": {"node": pack}, "button": "left",
                                  "mods": []})
    hinted("next shape")
    shot_here("board-hint-narrow")
    client.call("command.run", {"id": "view.reset_widths"})
    key("escape")


def board_dock_scenes(client, shot_here, open_board,
                      settings: tuple[Path, str] | None) -> None:
    """discovery/board-controls.md §3.1 and §3.3: the tool dock idle, with
    a shape armed and locked, with its keys shown while Space is held,
    with the "more shapes" menu open, folded on a narrow canvas with its
    menu, on a read-only board, and with `board-toolbar = off`. Nothing
    here changes the board."""

    def key(name: str, *mods: str, release: bool = False) -> dict:
        return client.call("board.key", {"key": name, "mods": list(mods), "release": release})

    def dock() -> dict:
        return client.call("board.dock")

    def laid_out(check) -> None:
        # The places are the last frame's, so wait for the one that shows it.
        wait_for(lambda: check(dock()), timeout=5, description="the dock laid out")

    def lit(button_id: str):
        return lambda state: any(button["id"] == button_id and button["lit"]
                                 and button["rect"] for button in state["buttons"])

    open_board("Checkout flow", fit=True)
    wait_for(lambda: client.call("focus.get")["named"] == ["note-editor"], timeout=5,
             description="the board focused")
    key("escape")
    laid_out(lambda state: state["shown"] and not state["folded"] and state["rect"])
    shot_here("board-dock-idle")
    key("d")
    laid_out(lit("board-dock-diamond"))
    shot_here("board-dock-armed")
    key("d")
    laid_out(lambda state: any(button["locked"] for button in state["buttons"]))
    shot_here("board-dock-locked")
    key("escape")
    key("space")
    laid_out(lambda state: any(button["badge"] for button in state["buttons"]))
    shot_here("board-dock-badges")
    key("space", release=True)
    client.call("board.dock", {"click": "board-dock-more"})
    laid_out(lambda state: state["menu"] and state["menu"]["rect"])
    shot_here("board-dock-more")
    client.call("board.dock", {"click": "board-dock-menu-document"})
    laid_out(lit("board-dock-more"))
    shot_here("board-dock-more-armed")
    key("escape")

    # A narrow canvas folds the six shapes into one button: every browser
    # column open, the notes at their widest. The board, left fitted,
    # fits again at the narrower width.
    key("r")
    key("escape")
    client.call("view.width", {"pane": "notes", "width": 480})
    client.call("column.open", {"column": "sections"})
    laid_out(lambda state: state["folded"] and state["rect"])
    shot_here("board-dock-fold")
    client.call("board.dock", {"click": "board-dock-shapes"})
    laid_out(lambda state: state["menu"] and state["menu"]["rect"])
    shot_here("board-dock-fold-menu")
    client.call("board.dock", {"click": "board-dock-shapes"})
    client.call("command.run", {"id": "view.reset_widths"})
    # A board command from a browser column folds the browser as it was.
    client.call("command.run", {"id": "board.tool.select"})

    open_board("Future", fit=True)
    laid_out(lambda state: state["shown"] and state["rect"])
    shot_here("board-dock-read-only")

    if settings is not None:
        path, text = settings
        path.write_text(text + "board-toolbar = off\n", encoding="utf-8")
        client.call("command.run", {"id": "settings.reload"})
        open_board("Checkout flow", fit=True)
        wait_for(lambda: not dock()["shown"], timeout=5, description="the dock off")
        key("d")
        shot_here("board-dock-off")
        key("escape")
        path.write_text(text, encoding="utf-8")
        client.call("command.run", {"id": "settings.reload"})
        wait_for(lambda: dock()["shown"], timeout=5, description="the dock back")


def board_style_bar_scenes(client, shot_here, open_board) -> None:
    """discovery/board-controls.md §3.2, §3.3 and §3.6: the style bar for
    one shape, for shapes of two colours, for a connector and for a shape
    with a connector; its shape menu and its last menu open; a swatch's
    preview under the pointer; its keys shown; the New shapes row with a
    colour picked; folded on a narrow canvas; and the footer's controls
    with something to undo and with nothing. The colours it sets are
    undone, so the board ends as it began."""

    start, pack, ship, edge = "ahj860xh", "3qrg4kv1", "1wg3aw6z", "zpsr8rp7"

    def key(name: str, *mods: str, release: bool = False) -> dict:
        return client.call("board.key", {"key": name, "mods": list(mods), "release": release})

    def bar(**params) -> dict:
        return client.call("board.style_bar", params)

    def laid_out(check=lambda state: True) -> None:
        # The places are the last frame's, so wait for the one that shows it.
        def ready() -> bool:
            state = bar()
            return bool(state["shown"] and state["rect"]
                        and all(control["rect"] for control in state["controls"])
                        and (state["menu"] is None or state["menu"]["rect"])
                        and check(state))

        wait_for(ready, timeout=5, description="the style bar laid out")

    def pick(*ids: str) -> None:
        for index, id_ in enumerate(ids):
            at = {"edge": id_} if id_ == edge else {"node": id_}
            client.call("board.pointer", {"kind": "click", "at": at, "button": "left",
                                          "mods": [] if index == 0 else ["shift"]})

    def menu(kind: str):
        return lambda state: state["menu"] is not None and state["menu"]["kind"] == kind

    open_board("Checkout flow", fit=True)
    wait_for(lambda: client.call("focus.get")["named"] == ["note-editor"], timeout=5,
             description="the board focused")
    key("escape")
    shot_here("board-footer")
    pick(start)
    laid_out()
    shot_here("board-style-shape")
    bar(click="board-style-shape")
    laid_out(menu("shapes"))
    shot_here("board-style-shape-menu")
    bar(click="board-style-shape")
    bar(click="board-style-more")
    laid_out(menu("more"))
    shot_here("board-style-more")
    bar(click="board-style-more")
    # Close enough to see the colour the pointer previews.
    client.call("command.run", {"id": "board.zoom.selection"})
    bar(hover="board-style-purple")
    wait_for(lambda: client.call("board.get")["patch"] == "board.color.purple", timeout=5,
             description="the preview shown")
    shot_here("board-style-hover")
    bar(hover=None)
    client.call("command.run", {"id": "board.zoom.fit"})
    key("space")
    laid_out(lambda state: any(control["badge"] for control in state["controls"]))
    shot_here("board-style-badges")
    key("space", release=True)

    # Two shapes of two colours: nothing ringed. Then the footer with
    # something to undo.
    pick(pack)
    key("1")
    pick(ship)
    key("5")
    pick(pack, ship)
    laid_out()
    shot_here("board-style-colours")
    shot_here("board-footer-undo")
    pick(edge)
    laid_out()
    shot_here("board-style-connector")
    pick(pack, edge)
    laid_out()
    shot_here("board-style-mixed")
    key("z", "mod")
    key("z", "mod")

    # Nothing selected and a shape tool armed: the colour new shapes take.
    key("escape")
    key("4")
    key("r")
    laid_out(lambda state: state["new_shapes"])
    shot_here("board-style-new-shapes")
    key("escape")
    key("0")

    # A narrow canvas folds the fills, the outline and the connector's
    # controls into the last menu: 432 pixels, what a 1280-wide window
    # leaves beside the sections and a 480-wide notes column. The notes
    # column gives up what a narrower window lacks (macOS fits the window
    # to a 1024x768 screen at 1024x640), or the canvas would be too narrow
    # for even the folded bar, and none would show.
    before = client.call("board.bounds")["root"]
    window = client.call("navigation.get")["viewport"]["width"]
    client.call("view.width", {"pane": "notes", "width": 480 - (1280 - window)})
    client.call("column.open", {"column": "sections"})
    client.call("editor.focus")
    wait_for(lambda: client.call("board.bounds")["root"]["width"] == 432, timeout=5,
             description="a canvas 432 wide")
    pick(pack, edge)
    laid_out(lambda state: state["folded"])
    shot_here("board-style-fold")
    bar(click="board-style-more")
    laid_out(menu("more"))
    shot_here("board-style-fold-menu")
    bar(click="board-style-more")
    client.call("command.run", {"id": "view.reset_widths"})
    # The browser columns fold back as they were, or the scenes after these
    # get the narrow canvas too.
    client.call("command.run", {"id": "view.collapse_columns"})
    wait_for(lambda: client.call("board.bounds")["root"] == before, timeout=5,
             description="the canvas as it was")
    client.call("command.run", {"id": "board.select.none"})


def board_text_size_scenes(client, shot_here, open_board) -> None:
    """discovery/board-controls.md §3.9: the four sizes on shapes and on
    free text, with one past the range drawn at 96 and flagged; the same
    board far out, where tiny-skia leaves out labels under 4 pixels; the
    "Aa" menu open on a shape; a size previewed under the pointer; and a
    label edited at 28. The board ends as it began."""

    def key(name: str, *mods: str) -> dict:
        return client.call("board.key", {"key": name, "mods": list(mods)})

    def bar(**params) -> dict:
        return client.call("board.style_bar", params)

    def laid_out(check=lambda state: True) -> None:
        def ready() -> bool:
            state = bar()
            return bool(state["shown"] and state["rect"]
                        and all(control["rect"] for control in state["controls"])
                        and (state["menu"] is None or state["menu"]["rect"])
                        and check(state))

        wait_for(ready, timeout=5, description="the style bar laid out")

    def pick(node: str) -> None:
        client.call("board.pointer", {"kind": "click", "at": {"node": node}, "button": "left",
                                      "mods": []})

    open_board("Text sizes", fit=True)
    wait_for(lambda: client.call("focus.get")["named"] == ["note-editor"], timeout=5,
             description="the board focused")
    key("escape")
    shot_here("board-text-sizes")
    # Further out, a label under 4 pixels on screen is left out: at 25 %
    # the sizes 12 and 14, and at 10 % all but the one drawn at 96.
    # Stepped down from 100 % to each, whatever the fit and the stops are.
    client.call("command.run", {"id": "font.reset"})
    for name, zoom in (("board-text-sizes-quarter", 0.25), ("board-text-sizes-far", 0.1)):
        while client.call("board.get", {})["viewport"]["zoom"] > zoom + 0.001:
            client.call("command.run", {"id": "font.decrease"})
        shot_here(name)
    client.call("command.run", {"id": "board.zoom.fit"})
    pick("s20")
    laid_out()
    bar(click="board-style-text-size")
    laid_out(lambda state: state["menu"] is not None)
    shot_here("board-text-size-menu")
    bar(hover="board-style-menu-text-size-28")
    wait_for(lambda: client.call("board.get")["patch"] == "board.text_size.28", timeout=5,
             description="the preview shown")
    shot_here("board-text-size-hover")
    bar(hover=None)
    bar(click="board-style-text-size")
    # A label edited at 28, its caret at the end.
    pick("s28")
    key("enter")
    wait_for(lambda: client.call("board.get")["editing"] is not None, timeout=5,
             description="the label open")
    key("end")
    shot_here("board-text-size-editing")
    key("escape")
    key("escape")
    assert client.call("board.get")["pristine"], "the text size scenes changed the board"


def board_edit_scenes(client, shot_here, open_board) -> None:
    """discovery/boards.md §6.3, Phase 2: what editing draws over a board.
    These change the board, so they come last."""
    pack, receive, reject, into_receive = "3qrg4kv1", "12axfsny", "pa6md7qp", "4x2tte9d"
    start, notify, charge = "ahj860xh", "c51ksbz1", "8wcf01q2"
    into_check, reject_no = "8va037pc", "4xtwed1c"

    def key(name: str, *mods: str) -> dict:
        return client.call("board.key", {"key": name, "mods": list(mods)})

    def pointer(kind: str, at: dict | None = None, mods: tuple[str, ...] = ()) -> dict:
        params = {"kind": kind, "button": "left", "mods": list(mods)}
        if at is not None:
            params["at"] = at
        return client.call("board.pointer", params)

    def focused():
        wait_for(lambda: client.call("focus.get")["named"] == ["note-editor"], timeout=5,
                 description="the board focused")

    open_board("Checkout flow", fit=True)
    focused()
    client.call("command.run", {"id": "board.zoom.fit"})
    # One node selected: its outline and eight handles.
    pointer("click", {"node": pack})
    focused()
    shot_here("board-selected")
    # The cursor ring: the selection cleared, the cursor kept, and then
    # two selected with the cursor on the second.
    key("escape")
    shot_here("board-cursor")
    pointer("click", {"node": receive})
    pointer("click", {"node": pack}, ("shift",))
    shot_here("board-multiple")
    key("escape")
    # A marquee being dragged from empty canvas.
    placed = client.call("board.bounds")
    box = next(node["bounds"] for node in placed["nodes"] if node["id"] == receive)
    view, root = placed["viewport"], placed["root"]
    corner = [view["x"] + (box["x"] - 30 - root["x"]) / view["zoom"],
              view["y"] + (box["y"] - 30 - root["y"]) / view["zoom"]]
    pointer("press", {"canvas": corner})
    pointer("move", {"canvas": [corner[0] + 120, corner[1] + 200]})
    pointer("move", {"canvas": [corner[0] + 260, corner[1] + 330]})
    shot_here("board-marquee")
    pointer("release")
    key("escape")
    # An armed tool's mode chip.
    key("d")
    shot_here("board-armed")
    key("escape")
    # The connect handles of the node the pointer has rested on.
    pointer("move", {"node": receive})
    wait_for(lambda: len(client.call("board.bounds")["handles"]) == 4, timeout=5,
             description="the connect handles shown")
    shot_here("board-connect-handles")
    # A connector dragged from one: over a node, which is outlined and
    # shows its side handles, the one in reach taken; and over empty
    # canvas, where a release would make a node. Both routes run clear of
    # every other connector: where a new one would share a line, which of
    # the two moves aside follows its id, which is drawn at random.
    pointer("press", {"node": receive, "handle": "connect_right"})
    pointer("move", {"node": reject})
    pointer("move", {"node": reject, "handle": "side_top"})
    shot_here("board-connect-target")
    pointer("move", {"canvas": [420, 300]})
    shot_here("board-connect-new")
    key("escape")
    # An end of a connector dragged off its node, where a release would
    # leave it as it was.
    pointer("click", {"edge": into_receive})
    pointer("press", {"edge": into_receive, "handle": "to"})
    pointer("move", {"canvas": [420, 300]})
    shot_here("board-connect-end")
    key("escape")
    key("escape")
    # Keyboard Connecting from Pack: the query in the mode chip and the
    # node it found outlined.
    pointer("click", {"node": pack})
    key("c")
    client.call("board.type", {"text": "che"})
    shot_here("board-connecting")
    key("escape")
    key("escape")
    # A label open, its text replaced and the caret after it.
    pointer("double_click", {"node": pack})
    client.call("board.type", {"text": "Pack the order"})
    shot_here("board-editing")
    key("escape")
    # Connectors styled by the style commands, beside the labelled and
    # elbow ones the flow has: a straight diagonal with a head at each end,
    # a dashed branch, and one with no head.
    for connector, commands in (
        (into_check, ("board.edge.routing", "board.edge.start")),
        (reject_no, ("board.stroke.dashed",)),
        (into_receive, ("board.edge.end",)),
    ):
        pointer("click", {"edge": connector})
        for command in commands:
            client.call("command.run", {"id": command})
    pointer("click", {"node": start})
    for node in (reject, notify, charge):
        pointer("click", {"node": node}, ("shift",))
    client.call("command.run", {"id": "board.zoom.selection"})
    # Away and back: the view is kept, the selection and its cursor are not.
    open_note(client, "Notes.md")
    open_note(client, "Checkout flow.wren-board")
    client.call("editor.focus")
    wait_for(lambda: client.call("board.get")["surface"] == "board", timeout=10,
             description="the flow open again")
    shot_here("board-edges")
    # A new board: empty, with the hint.
    client.call("command.run", {"id": "note.new_board"})
    client.call("dialog.name", {"name": "Empty board"})
    client.call("dialog.submit")
    focused()
    shot_here("board-empty-hint")
    # The flowchart the acceptance script builds on it, fitted, the last
    # connector still selected as the script leaves it. Its ids are drawn
    # from a seed: where two connectors would share a line, which moves
    # aside follows their ids.
    client.call("board.seed", {"seed": 20})
    for step in script_steps(ACCEPTANCE):
        if step[0] == "key":
            key(step[1], *step[2:])
        else:
            client.call("board.type", {"text": step[1]})
    client.call("command.run", {"id": "board.zoom.fit"})
    shot_here("board-built")


def board_arrange_scenes(client, shot_here, open_board) -> None:
    """discovery/boards.md §6.3, Phase 3: the stacking order changed. The
    shape at the back of Overlap, picked where no other covers it, is
    brought to the front, over the other two, and the selection cleared."""
    open_board("Overlap", fit=True)
    client.call("board.pointer", {"kind": "click", "button": "left", "mods": [],
                                  "at": {"canvas": [20, 10]}})
    client.call("board.key", {"key": "]", "mods": []})
    client.call("board.key", {"key": "escape", "mods": []})
    shot_here("board-stacking")

    # §5.17: a right-click on Pack opens the actions palette on its shape,
    # the current value; the highlight moved to blue previews it on the
    # board, which nothing has written.
    open_board("Checkout flow", fit=True)
    client.call("board.pointer", {"kind": "click", "button": "right", "mods": [],
                                  "at": {"node": "3qrg4kv1"}})
    modal = client.call("navigation.get")["modal"]
    keys = [row["key"]["command"] for row in modal["results"]]
    client.call("palette.step",
                {"delta": keys.index("board.color.blue") - modal["active_index"]})
    shot_here("board-actions")
    client.call("palette.close")


# The colour tokens in the order of their keys, 0 to 7.
BOARD_COLORS = ("neutral", "red", "orange", "yellow", "green", "cyan", "purple", "blue")


def board_style_scenes(client, shot_here) -> None:
    """discovery/boards.md §6.3, Phase 3 styling: every colour at every
    fill level, set through the digit keys and the fill commands on a new
    board. A column per colour: a shape with each fill, then a text node,
    and a labelled connector from the unfilled shape to it."""
    client.call("command.run", {"id": "note.new_board"})
    client.call("dialog.name", {"name": "Colours"})
    client.call("dialog.submit")
    wait_for(lambda: client.call("focus.get")["named"] == ["note-editor"], timeout=5,
             description="the board focused")
    fills = ("tint", "solid", "none")
    edits = []
    for column, color in enumerate(BOARD_COLORS):
        x = column * 140
        for row, fill in enumerate(fills):
            edits.append({"op": "add_node", "id": f"{fill}-{color}", "kind": "shape",
                          "shape": "rounded", "at": [x, row * 100], "size": [120, 60],
                          "text": f"{color.capitalize()}\n{fill}"})
        edits.append({"op": "add_node", "id": f"text-{color}", "kind": "text",
                      "at": [x, 420], "size": [120, 20], "text": f"{color.capitalize()} text"})
        edits.append({"op": "connect", "id": f"edge-{color}", "from": {"node": f"none-{color}"},
                      "to": {"node": f"text-{color}"}})
    client.call("board.apply", {"edits": edits})
    client.call("board.apply", {"edits": [
        {"op": "set", "id": f"edge-{color}", "field": "text", "value": "yes"}
        for color in BOARD_COLORS]})

    def pick(ids: list[str]) -> None:
        def at(id_: str) -> dict:
            return {"edge": id_} if id_.startswith("edge-") else {"node": id_}

        for index, id_ in enumerate(ids):
            client.call("board.pointer", {"kind": "click", "at": at(id_), "button": "left",
                                          "mods": [] if index == 0 else ["shift"]})

    client.call("command.run", {"id": "board.zoom.fit"})
    for digit, color in enumerate(BOARD_COLORS):
        pick([f"{fill}-{color}" for fill in fills] + [f"text-{color}", f"edge-{color}"])
        client.call("board.key", {"key": str(digit), "mods": []})
    for fill in fills[1:]:
        pick([f"{fill}-{color}" for color in BOARD_COLORS])
        client.call("command.run", {"id": f"board.fill.{fill}"})
    client.call("command.run", {"id": "board.select.none"})
    client.call("command.run", {"id": "board.zoom.fit"})
    # An unfocused board draws no cursor ring (§5.3 rule 7).
    client.call("column.open", {"column": "notes"})
    shot_here("board-colors")


def window_size(app: WrenApp, size: str) -> None:
    """Resize the app's window to ``WIDTHxHEIGHT`` logical pixels with
    xdotool, so X11 only, and wait until the app lays itself out at it."""
    width, height = (int(part) for part in size.split("x"))

    def xdo(*args: str) -> str:
        return subprocess.run(["xdotool", *args], check=True, capture_output=True, text=True,
                              timeout=10).stdout.strip()

    window = xdo("search", "--onlyvisible", "--pid", str(app.pid)).splitlines()[0]
    xdo("windowsize", window, str(width), str(height))
    wait_for(lambda: app.client.call("navigation.get")["viewport"]
             == {"width": width, "height": height},
             timeout=5, description=f"the window at {size}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--theme", default="dark")
    parser.add_argument("--out", type=Path, default=Path("target/scenes"))
    parser.add_argument("--engine", choices=("classic", "advanced"),
                        help="the editor-engine setting: classic, or advanced for --only tasks")
    parser.add_argument("--setting", action="append", default=[], metavar="KEY=VALUE",
                        help="another config.conf line; repeatable")
    parser.add_argument("--label", help="a directory level for these settings")
    parser.add_argument("--only", choices=("markdown", "spell", "boards", "tasks", "task-list"),
                        help="shoot only these scenes")
    parser.add_argument("--fonts", metavar="LABEL",
                        help="with --only tasks, name the font setup in the output path "
                             "(default: whether Noto Color Emoji is installed)")
    parser.add_argument("--window", metavar="WIDTHxHEIGHT",
                        help="with --only boards, resize the window first (X11 only); "
                             "pair it with --label")
    args = parser.parse_args()
    if args.engine is None:
        args.engine = "advanced" if args.only in ("tasks", "task-list") else "classic"
    if args.fonts and args.only not in ("tasks", "task-list"):
        parser.error("--fonts goes with --only tasks")
    if args.only == "spell" and args.engine != "advanced":
        parser.error("--only spell needs --engine advanced: spell check runs on that engine only")
    if args.window and (args.only != "boards" or sys.platform == "darwin"
                        or "x" not in args.window):
        parser.error("--window takes WIDTHxHEIGHT, with --only boards, on X11")
    settings = f"editor-engine = {args.engine}\n"
    for setting in args.setting:
        key, separator, value = setting.partition("=")
        if not separator:
            parser.error(f"--setting {setting!r} is not KEY=VALUE")
        settings += f"{key.strip()} = {value.strip()}\n"
    # A `--setting editor-engine=…` comes later in the file, so it wins.
    engine = last_value(settings, "editor-engine")
    if args.only in ("tasks", "task-list"):
        out = tasks_out(args.out, engine, args.fonts)
    else:
        out = args.out if engine == "classic" else args.out / engine
    if args.label:
        out = out / args.label
    out = (out / args.theme).resolve()
    out.mkdir(parents=True, exist_ok=True)

    def settled(client) -> bool:
        session = client.call("session.get")
        document = session["document"]
        return not session["pending_durability"] and not (document and document["dirty"])

    def shot(client, name: str) -> None:
        # Autosave and the caret blink run on wall-clock time; each scene
        # waits for the save and shoots with the caret freshly shown.
        path = out / f"{name}.png"
        path.unlink(missing_ok=True)
        time.sleep(0.3)
        wait_for(lambda: settled(client), timeout=10, description="autosave")
        client.call("test.caret.restart")
        client.call("app.screenshot", {"path": str(path)})
        print(path)

    with scene_workspaces() as temp:
        if args.only == "boards":
            workspace = Path(temp).resolve() / "boards"
            build_board_workspace(workspace)
            with WrenApp(workspace=workspace, evaluation=False,
                         settings=settings + "boards = on\n") as app:
                if args.window:
                    window_size(app, args.window)
                app.client.call("theme.set", {"id": args.theme})
                wait_for(lambda: app.client.call("session.get")["notes"], timeout=10)
                banner = app.client.call("session.get")["banner"]
                assert banner is None, banner

                def shot_board(name: str) -> None:
                    shot(app.client, name)

                board_scenes(app.client, shot_board, out, workspace,
                             (app.settings_path, settings + "boards = on\n"))
            return 0
        if args.only == "tasks":
            workspace = Path(temp).resolve() / "tasks"
            shutil.copytree(TASKS_FIXTURE, workspace)
            # Half an hour from an hour boundary, so "Edited" reads the same
            # through the capture.
            mtime = time.time() - 1.5 * 3600
            for path in workspace.rglob("*.md"):
                os.utime(path, (mtime, mtime))
            with WrenApp(workspace=workspace, evaluation=False,
                         settings=settings + "tasks = on\n", today=TASKS_TODAY) as app:
                app.client.call("theme.set", {"id": args.theme})
                wait_for(lambda: app.client.call("session.get")["notes"], timeout=10)
                banner = app.client.call("session.get")["banner"]
                assert banner is None, banner

                def shot_tasks(name: str) -> None:
                    shot(app.client, name)

                task_scenes(app.client, shot_tasks, engine,
                            lambda size: window_size(app, size))
            return 0
        if args.only == "task-list":
            workspace = Path(temp).resolve() / "list"
            shutil.copytree(TASK_LIST_FIXTURE, workspace)
            with WrenApp(workspace=workspace, evaluation=False,
                         settings=settings + "tasks = on\n" + TASK_LIST_SETTINGS,
                         today=TASKS_TODAY) as app:
                app.client.call("theme.set", {"id": args.theme})
                wait_for(lambda: app.client.call("session.get")["notes"], timeout=10)
                banner = app.client.call("session.get")["banner"]
                assert banner is None, banner

                def shot_list(name: str) -> None:
                    shot(app.client, name)

                task_list_scenes(app.client, shot_list)
            return 0
        if args.only == "spell":
            workspace = Path(temp).resolve() / "work"
            (workspace / "Notes/Inbox").mkdir(parents=True)
            shutil.copyfile(SPELL_NOTE, workspace / "Notes/Inbox/Spell chek plan.md")
            with WrenApp(workspace=workspace, evaluation=False,
                         settings=settings + "spell-check = on\n", words=SPELL_WORDS) as app:
                app.client.call("theme.set", {"id": args.theme})
                wait_for(lambda: app.client.call("session.get")["notes"], timeout=10)
                banner = app.client.call("session.get")["banner"]
                assert banner is None, banner

                def shot_spell(name: str) -> None:
                    shot(app.client, name)

                spell_scenes(app.client, shot_spell)
            return 0
        if args.only == "markdown":
            workspace = Path(temp).resolve() / "work"
            build_workspace(workspace)
            with WrenApp(workspace=workspace, evaluation=False, settings=settings) as app:
                app.client.call("theme.set", {"id": args.theme})
                wait_for(lambda: app.client.call("session.get")["notes"], timeout=10)
                banner = app.client.call("session.get")["banner"]
                assert banner is None, banner

                def shot_markdown(name: str) -> None:
                    shot(app.client, name)

                editor_scenes(app.client, shot_markdown)
                folded_editor_scenes(app.client, shot_markdown)
            return 0

        blank = Path(temp).resolve() / "blank"
        blank.mkdir()
        with WrenApp(workspace=blank, evaluation=False, settings=settings) as app:
            app.client.call("theme.set", {"id": args.theme})
            wait_for(lambda: app.client.call("session.get")["root"], timeout=10)
            shot(app.client, "empty-workspace")

        workspace = Path(temp).resolve() / "work"
        build_workspace(workspace)
        with WrenApp(workspace=workspace, evaluation=False, settings=settings) as app:
            client = app.client
            client.call("theme.set", {"id": args.theme})
            wait_for(lambda: client.call("session.get")["notes"], timeout=10)

            def shot_here(name: str) -> None:
                shot(client, name)

            note = next(
                row for row in client.call("session.get")["notes"]
                if row["path"].endswith("2026-11-04.md")
            )
            client.call("core.apply", {"op": "open_note", "note": note["id"]})
            time.sleep(0.3)
            shot_here("columns")
            # Keyboard drill-down: walk left to Sections, then fold rightward.
            for _ in range(3):
                client.call("command.run", {"id": "navigation.column_left"})
            shot_here("column-sections")
            for name in ("column-groups-folded", "column-notes-folded", "column-editor-folded"):
                client.call("command.run", {"id": "navigation.column_right"})
                shot_here(name)
            client.call("column.open", {"column": "groups"})
            client.call("shortcut", {"id": "navigation.jump"})
            shot_here("palette-jump-group")
            client.call("palette.close")
            # Unfold, then return to the editor through Jump, which keeps the fold.
            client.call("column.open", {"column": "sections"})
            client.call("palette.open", {"mode": "jump"})
            client.call("palette.query", {"query": "2026-11-04"})
            client.call("palette.activate", {})
            for mode, query in [
                ("commands", ""),
                ("jump", "al"),
                ("search", "promo"),
                ("workspaces", ""),
                ("themes", ""),
                ("fonts", ""),
            ]:
                client.call("palette.open", {"mode": mode})
                if query:
                    client.call("palette.query", {"query": query})
                shot_here(f"palette-{mode}")
                client.call("palette.close")
            # The editor's commands, highlighted down in their own group.
            client.call("palette.open", {"mode": "commands"})
            client.call("palette.step", {"delta": 12})
            shot_here("palette-markdown")
            client.call("palette.close")
            # The settings commands, each showing the value it flips.
            for name, query in [("palette-assist", "assist"), ("palette-engine", "engine")]:
                client.call("palette.open", {"mode": "commands"})
                client.call("palette.query", {"query": query})
                shot_here(name)
                client.call("palette.close")
            runbook_note = next(
                row for row in client.call("session.get")["notes"]
                if row["path"].endswith("Cutover runbook.md")
            )
            client.call("core.apply", {"op": "open_note", "note": runbook_note["id"]})
            client.call("palette.open", {"mode": "outline"})
            # Previewing a heading far down scrolls the editor under the palette.
            client.call("palette.step", {"delta": 7})
            shot_here("palette-outline")
            client.call("palette.query", {"query": "path"})
            shot_here("palette-outline-filtered")
            client.call("palette.close")
            # Without the browser columns it docks at the right instead.
            client.call("view.focus", {"enabled": True})
            client.call("palette.open", {"mode": "outline"})
            client.call("palette.step", {"delta": 8})
            shot_here("palette-outline-focus")
            client.call("palette.close")
            client.call("view.focus", {"enabled": False})
            client.call("core.apply", {"op": "open_note", "note": note["id"]})
            for _ in range(4):
                client.call("command.run", {"id": "font.increase"})
            shot_here("text-larger")
            client.call("command.run", {"id": "font.reset"})
            client.call("view.focus", {"enabled": True})
            shot_here("focus")
            client.call("view.focus", {"enabled": False})
            # Smart keys: continue the task list, nest an item, and continue
            # the nested list.
            client.call("editor.cursor", {"line": 5, "column": len("- [x] Share Q4 roadmap"),
                                          "selection": None})
            for step in ("enter", "Draft the cutover note", "enter", "tab",
                         "Ask Bob about on-call", "enter", "Loop in Priya", "enter", "enter"):
                if step in ("enter", "tab"):
                    client.call("editor.command", {"intent": step})
                else:
                    client.call("editor.insert", {"text": step})
            shot_here("editor-smart-keys")

            # Markdown commands at the cursor: check a task, format three
            # words, and move a line up.
            def at_word(word: str) -> None:
                lines = client.call("editor.get")["text"].split("\n")
                line = next(index for index, text in enumerate(lines) if word in text)
                client.call("editor.cursor", {
                    "line": line, "column": len(lines[line].split(word)[0].encode()) + 1,
                    "selection": None,
                })

            for word, command in [
                ("Review promo", "markdown.toggle_task"),
                ("cutover.", "markdown.bold"),
                ("promo timeline", "markdown.italic"),
                ("timeline", "markdown.code"),
                ("She asked", "edit.move_line_up"),
            ]:
                at_word(word)
                client.call("command.run", {"id": command})
            shot_here("editor-markdown-commands")
            archive = next(
                row for row in client.call("session.get")["sections"]
                if row["path"] == EMPTY_SECTION
            )
            client.call("core.apply", {"op": "select_section", "section": archive["id"]})
            shot_here("empty-section")
            client.call("core.apply", {
                "op": "create_group", "section": archive["id"], "name": "2026",
            })
            shot_here("empty-group")
            client.call("palette.open", {"mode": "jump"})
            client.call("palette.query", {"query": "Markdown tour"})
            client.call("palette.activate", {})
            editor_scenes(client, shot_here)
            folded_editor_scenes(client, shot_here)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
