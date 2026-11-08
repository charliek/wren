"""Capture the main Wren screens for visual review against the mockup.

Builds the mockup's sample workspace in a private directory, launches the
smoke-test build under the test harness, and writes one PNG per scene:

    cargo build --locked -p wren-iced --features smoke-test
    python3 tools/capture_scenes.py --theme wren-dark --out target/scenes

`--engine advanced` runs the editor on the owned engine; the PNGs then go
to `<out>/advanced/<theme>/` instead of `<out>/<theme>/`.

On Linux run it inside `tools/run_display.py x11 -- ...` for a private
display. On macOS it uses the logged-in window server directly.
"""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

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

# A section with no groups yet, for the empty-state scenes.
EMPTY_SECTION = "Archive"


def build_workspace(root: Path) -> None:
    (root / EMPTY_SECTION).mkdir(parents=True)
    for relative, text in SAMPLE.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    # Stagger modification times so "modified" order matches the mockup.
    now = time.time()
    for offset, relative in enumerate(SAMPLE):
        mtime = now - offset * 3600
        os.utime(root / relative, (mtime, mtime))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--theme", default="dark")
    parser.add_argument("--out", type=Path, default=Path("target/scenes"))
    parser.add_argument("--engine", choices=("classic", "advanced"), default="classic",
                        help="the editor-engine setting")
    args = parser.parse_args()
    out = args.out if args.engine == "classic" else args.out / args.engine
    out = (out / args.theme).resolve()
    settings = f"editor-engine = {args.engine}\n"
    out.mkdir(parents=True, exist_ok=True)

    def shot(client, name: str) -> None:
        path = out / f"{name}.png"
        path.unlink(missing_ok=True)
        time.sleep(0.3)
        client.call("app.screenshot", {"path": str(path)})
        print(path)

    with tempfile.TemporaryDirectory() as temp:
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
            anchor, position = MARKDOWN_SELECTION
            client.call("editor.cursor", {**position, "selection": anchor})
            client.call("editor.focus")
            shot_here("markdown")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
