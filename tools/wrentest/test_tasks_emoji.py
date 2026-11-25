"""The task glyphs in the real app (discovery/tasks-build.md item 0.3, the
emoji spike): ``📅 ✅ ❌ 🔺 ⏫ 🔼 🔽 ⏬``, each bare and with U+FE0F, typed,
pasted and already in the fixture's wrapped task lines, with ``tasks = on``
on the engines ``WRENTEST_ENGINE`` names. Arrow keys step over a glyph in
one press and Shift selects exactly it; one Backspace takes all of it, and
the note's bytes on disk say so.

Whether a glyph is drawn depends on the fonts installed, so nothing here
looks at pixels: ``capture_scenes.py --only tasks`` shoots them.
"""

from __future__ import annotations

import shutil
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from tools.wrentest.test_editor import engine  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.test_tasks_baseline import saved
from tools.wrentest.ui import WrenApp, wait_for

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "tasks" / "workspace"
NOTE = "Tasks/Emoji/Emoji spike.md"
ORIGINAL = (FIXTURE / NOTE).read_text(encoding="utf-8")
LINES = ORIGINAL.split("\n")
TODAY = "2026-11-19"

GLYPHS = ["\U0001f4c5", "✅", "❌", "\U0001f53a", "⏫", "\U0001f53c",
          "\U0001f53d", "⏬"]
VS16 = "️"
CLUSTERS = [form for glyph in GLYPHS for form in (glyph, glyph + VS16)]


def width(text: str) -> int:
    """A column as the editor counts it: UTF-8 bytes."""
    return len(text.encode("utf-8"))


@contextmanager
def emoji_app(tmp_path: Path, engine: str) -> Iterator[tuple[WrenApp, Path]]:  # noqa: F811
    workspace = tmp_path / "workspace"
    shutil.copytree(FIXTURE, workspace)
    with WrenApp(workspace=workspace, evaluation=False,
                 settings=f"editor-engine = {engine}\ntasks = on\n", today=TODAY,
                 artifacts_dir=artifact_directory(f"tasks-emoji-{engine}")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert app.client.call("tasks.get")["tasks"] is True
        note = next(row["id"] for row in app.client.call("session.get")["notes"]
                    if row["path"] == NOTE)
        app.client.call("core.apply", {"op": "open_note", "note": note})
        wait_for(lambda: app.client.call("editor.get")["text"] == ORIGINAL,
                 description="the note opens")
        app.client.call("editor.focus")
        yield app, workspace / NOTE


def key(app: WrenApp, name: str, *mods: str) -> dict:
    return app.client.call("editor.key", {"key": name, "mods": list(mods)})


def at(line: int, column: int) -> dict:
    return {"line": line, "column": column}


def test_the_fixtures_wrapped_glyphs_step_select_and_backspace_as_one(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with emoji_app(tmp_path, engine) as (app, path):
        call = app.client.call
        for cluster in CLUSTERS:
            # Its line's last glyph, past the wrap; a Due line for 📅 and so on.
            line = next(i for i, text in enumerate(LINES)
                        if text.startswith("- [") and f" {cluster} on the first row" in text)
            text = LINES[line]
            start = width(text[:text.rfind(cluster)])
            end = start + width(cluster)
            case = f"{cluster!r} on line {line}"

            call("editor.cursor", {"line": line, "column": start, "selection": None})
            assert key(app, "right")["cursor"] == at(line, end), case
            assert key(app, "left")["cursor"] == at(line, start), case
            selected = key(app, "right", "shift")
            assert selected["selected_text"] == cluster, case
            assert (selected["cursor"], selected["selection"]) == (at(line, end),
                                                                   at(line, start)), case

            call("editor.cursor", {"line": line, "column": end, "selection": None})
            removed = key(app, "backspace")
            expected = "\n".join(LINES[:line] + [text[:text.rfind(cluster)]
                                                 + text[text.rfind(cluster) + len(cluster):]]
                                 + LINES[line + 1:])
            assert removed["text"] == expected, case
            assert removed["cursor"] == at(line, start), case
            saved(app, path, expected)
            # The next case's bytes on disk show the undo was saved too.
            assert call("editor.undo")["text"] == ORIGINAL, case
        saved(app, path, ORIGINAL)


def test_typed_and_pasted_glyphs_step_select_and_backspace_as_one(
        tmp_path: Path, engine: str) -> None:  # noqa: F811
    with emoji_app(tmp_path, engine) as (app, path):
        call = app.client.call
        last = len(LINES) - 1
        assert LINES[last] == "", "the note ends with a line break"
        prefix = "- [ ] Typed "
        for cluster in CLUSTERS:
            for how in ("paste", "type"):
                case = f"{how} {cluster!r}"
                call("editor.cursor", {"line": last, "column": 0, "selection": None})
                call("editor.insert", {"text": prefix})
                if how == "paste":
                    result = call("editor.insert", {"text": cluster})
                else:
                    # One key per code point: the selector arrives on its own.
                    for point in cluster:
                        result = key(app, point)
                line = prefix + cluster
                assert result["text"] == ORIGINAL + line, case
                start, end = width(prefix), width(line)
                assert result["cursor"] == at(last, end), case

                assert key(app, "left")["cursor"] == at(last, start), case
                assert key(app, "right")["cursor"] == at(last, end), case
                assert key(app, "left", "shift")["selected_text"] == cluster, case

                call("editor.cursor", {"line": last, "column": end, "selection": None})
                removed = key(app, "backspace")
                assert removed["text"] == ORIGINAL + prefix, case
                assert removed["cursor"] == at(last, start), case
                saved(app, path, ORIGINAL + prefix)

                # Put the note back for the next case.
                call("editor.cursor", {"line": last, "column": start,
                                       "selection": at(last, 0)})
                assert key(app, "backspace")["text"] == ORIGINAL, case
        saved(app, path, ORIGINAL)
