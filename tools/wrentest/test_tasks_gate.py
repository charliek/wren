"""What the ``tasks`` setting gates, and the one clock (discovery/tasks-build.md
§1 and item 0.1).

With the setting off ``tasks.get`` says only that, and every other task
request is refused. With it on, the date ``WREN_TEST_TODAY`` pins at launch
is today, and ``tasks.set_today`` moves it through the day-change path the
timer takes. Each case writes its own ``tasks`` line and engine, whatever
the lane's ``WRENTEST_TASKS`` says.
"""

from __future__ import annotations

import pytest

from tools.wrentest.client import RemoteError
from tools.wrentest.test_editor import engine  # noqa: F401  (fixture)
from tools.wrentest.test_setup import artifact_directory
from tools.wrentest.ui import WrenApp

# The design page's day, a Thursday; every task test pins it.
TODAY = "2026-11-19"


def tasks(app: WrenApp) -> dict:
    return app.client.call("tasks.get")


def palette_keys(app: WrenApp) -> list[str]:
    app.client.call("palette.open", {"mode": "commands"})
    modal = app.client.call("palette.query", {"query": ""})["navigation"]["modal"]
    app.client.call("palette.close")
    return [row["key"]["command"] for row in modal["results"] if "command" in row["key"]]


def test_with_tasks_off_the_requests_say_so_and_refuse(engine: str) -> None:  # noqa: F811
    with WrenApp(settings=f"editor-engine = {engine}\ntasks = off\n", today=TODAY,
                 artifacts_dir=artifact_directory("tasks-gate-off")) as app:
        assert app.client.call("editor.get")["engine"] == engine
        assert tasks(app) == {"tasks": False}
        with pytest.raises(RemoteError, match="Tasks are off"):
            app.client.call("tasks.set_today", {"date": "2026-11-20"})
        assert tasks(app) == {"tasks": False}
        assert palette_keys(app), "the palette lists commands"
        assert not any(key.startswith("task.") or key.startswith("tasks.")
                       for key in palette_keys(app))


def test_the_launch_date_is_today_and_set_today_moves_it_once(engine: str) -> None:  # noqa: F811
    with WrenApp(settings=f"editor-engine = {engine}\ntasks = on\n", today=TODAY,
                 artifacts_dir=artifact_directory("tasks-gate-on")) as app:
        state = tasks(app)
        assert state["tasks"] is True
        assert state["engine"] == engine
        assert (state["today"], state["seen"], state["day_changes"]) == (TODAY, TODAY, 0)

        state = app.client.call("tasks.set_today", {"date": "2026-11-20"})
        assert (state["today"], state["seen"], state["day_changes"]) == \
            ("2026-11-20", "2026-11-20", 1)
        # The same day again is no change; going back a day is one.
        assert app.client.call("tasks.set_today", {"date": "2026-11-20"})["day_changes"] == 1
        state = app.client.call("tasks.set_today", {"date": TODAY})
        assert (state["today"], state["day_changes"]) == (TODAY, 2)
        assert tasks(app)["day_changes"] == 2

        for bad in ("2026-11-31", "tomorrow", "19/11/2026"):
            with pytest.raises(RemoteError, match="YYYY-MM-DD"):
                app.client.call("tasks.set_today", {"date": bad})
        assert tasks(app)["today"] == TODAY


def test_the_caret_line_reads_as_the_classifier_reads_it(engine: str) -> None:  # noqa: F811
    with WrenApp(settings=f"editor-engine = {engine}\ntasks = on\n", today=TODAY,
                 artifacts_dir=artifact_directory("tasks-gate-line")) as app:
        app.client.call("editor.set", {"text": "- [ ] open\n- [x] done\n- [/] doing\nplain"})

        def box(symbol: str, status: str, kind: str) -> dict:
            return {"box": [2, 5], "symbol": symbol, "status": status, "type": kind}

        for line, text, task in [
            (0, "- [ ] open", box(" ", "Open", "open")),
            (1, "- [x] done", box("x", "Done", "done")),
            (2, "- [/] doing", box("/", "Doing", "doing")),
            (3, "plain", None),
        ]:
            app.client.call("editor.cursor", {"line": line, "column": 0, "selection": None})
            assert tasks(app)["line"] == {"number": line, "text": text, "task": task}


def test_with_no_note_open_there_is_no_caret_line(tmp_path) -> None:
    workspace = tmp_path / "workspace"
    (workspace / "Plans" / "Q1").mkdir(parents=True)
    with WrenApp(workspace=workspace, evaluation=False, settings="tasks = on\n", today=TODAY,
                 artifacts_dir=artifact_directory("tasks-gate-no-note")) as app:
        assert app.client.call("session.get")["document"] is None
        state = tasks(app)
        assert state["tasks"] is True and state["today"] == TODAY
        assert state["line"] is None


def test_without_a_pin_today_is_a_real_date() -> None:
    with WrenApp(settings="tasks = on\n",
                 artifacts_dir=artifact_directory("tasks-gate-clock")) as app:
        state = tasks(app)
        assert len(state["today"]) == 10 and state["today"][4] == "-"
        assert state["day_changes"] == 0
