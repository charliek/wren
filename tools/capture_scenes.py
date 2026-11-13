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
wider than its column. `--only boards` opens the board sample workspace
(`tools/wrentest/fixtures/boards/`) with `boards = on` and shoots the board
scenes, writing `board-flow.svg` beside them; the last of them select, drag
a marquee, arm a tool, open a label and make an empty board, through the
harness's `board.*` requests.

On Linux run it inside `tools/run_display.py x11 -- ...` for a private
display. On macOS it uses the logged-in window server directly.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
import time
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools.wrentest.boards import SAMPLES as BOARD_SAMPLES  # noqa: E402
from tools.wrentest.boards import conflict_board  # noqa: E402
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


def board_scenes(client, shot_here, out: Path) -> None:
    """discovery/boards.md §6.3, Phase 1."""

    def open_board(name: str) -> None:
        open_note(client, f"{name}.wren-board")
        client.call("editor.focus")

        def fitted():
            state = client.call("board.get")
            if state["surface"] != "board":
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
    # The kind icon, with the notes list holding the keyboard.
    client.call("column.open", {"column": "notes"})
    shot_here("board-notes-list")
    client.call("column.open", {"column": "editor"})
    client.call("editor.focus")
    client.call("command.run", {"id": "font.reset"})
    for _ in range(2):
        client.call("command.run", {"id": "font.increase"})
    shot_here("board-zoom-in")
    client.call("command.run", {"id": "font.reset"})
    for _ in range(3):
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
    client.call("command.run", {"id": "font.decrease"})
    client.call("command.run", {"id": "font.decrease"})
    shot_here("board-wide")
    # The New note dialog with its kind pills, opened on Board.
    client.call("command.run", {"id": "note.new_board"})
    client.call("dialog.name", {"name": "Release plan"})
    shot_here("board-new-dialog")
    client.call("dialog.close")
    board_edit_scenes(client, shot_here, open_board)
    client.call("column.open", {"column": "sections"})


def board_edit_scenes(client, shot_here, open_board) -> None:
    """discovery/boards.md §6.3, Phase 2: what editing draws over a board.
    These change the board, so they come last."""
    pack, receive, reject, into_receive = "3qrg4kv1", "12axfsny", "pa6md7qp", "4x2tte9d"

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

    open_board("Checkout flow")
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
    # A new board: empty, with the hint.
    client.call("command.run", {"id": "note.new_board"})
    client.call("dialog.name", {"name": "Empty board"})
    client.call("dialog.submit")
    focused()
    shot_here("board-empty-hint")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--theme", default="dark")
    parser.add_argument("--out", type=Path, default=Path("target/scenes"))
    parser.add_argument("--engine", choices=("classic", "advanced"), default="classic",
                        help="the editor-engine setting")
    parser.add_argument("--setting", action="append", default=[], metavar="KEY=VALUE",
                        help="another config.conf line; repeatable")
    parser.add_argument("--label", help="a directory level for these settings")
    parser.add_argument("--only", choices=("markdown", "boards"), help="shoot only these scenes")
    args = parser.parse_args()
    out = args.out if args.engine == "classic" else args.out / args.engine
    if args.label:
        out = out / args.label
    out = (out / args.theme).resolve()
    settings = f"editor-engine = {args.engine}\n"
    for setting in args.setting:
        key, separator, value = setting.partition("=")
        if not separator:
            parser.error(f"--setting {setting!r} is not KEY=VALUE")
        settings += f"{key.strip()} = {value.strip()}\n"
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
                app.client.call("theme.set", {"id": args.theme})
                wait_for(lambda: app.client.call("session.get")["notes"], timeout=10)
                banner = app.client.call("session.get")["banner"]
                assert banner is None, banner

                def shot_board(name: str) -> None:
                    shot(app.client, name)

                board_scenes(app.client, shot_board, out)
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
