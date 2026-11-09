"""Capture the same scenes in every built-in theme, for design review.

Builds a mock workspace whose notes use every Markdown construct the editor
colors (headings, lists, tasks, quotes, code, links, emphasis, rules), then
captures each scene once per theme:

    cargo build --locked -p wren-iced --features smoke-test
    python3 tools/run_display.py x11 -- python3 tools/capture_themes.py

Writes `<out>/<theme>/<scene>.png`. `--themes` limits the run to a
comma-separated list of ids.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from tools.wrentest.ui import WrenApp, wait_for  # noqa: E402

FEATURE_NOTE = "Projects/API Migration/Cutover plan.md"

SAMPLE = {
    FEATURE_NOTE: """# Cutover plan — read path

Owner: **Alice Chen** · Reviewers: *Bob*, *Priya* · Status: `in review`
Spec: [rollout doc](https://wiki.internal/rollout) · live at https://status.internal

## Goals

- Move all read traffic to the new gateway by **Nov 21**
- Keep p99 latency under `120 ms` during the shift
- [x] Shadow traffic at 5% for a week
- [x] Compare responses with the [diff dashboard](https://grafana.internal/d/api-diff)
- [ ] Ramp 5% → 25% → 100% with a ***hard*** rollback gate
- [ ] Write the incident playbook

> If error rate exceeds 0.5% for five minutes, roll back first and
> investigate second. Nobody gets paged for a rollback.

### Rollback

```sh
kubectl rollout undo deploy/gateway-read --to-revision=41
./scripts/flip-flag read_path_v2 off
```

---

1. Freeze schema changes on Nov 18
2. Ramp during the Tuesday window
3. Retire the legacy readers after two clean weeks

## Open questions

- Do batch exports ~~need~~ get their own pool?
- Who owns the cache warm-up job after cutover?
""",
    "Projects/API Migration/Design notes.md": "# Design notes\n\nRead path first, then writes.\n",
    "Projects/API Migration/Load test results.md": "# Load test results\n\n- 14k rps sustained\n",
    "Projects/API Migration/Vendor comparison.md": "# Vendor comparison\n",
    "Projects/Planning/Quarterly planning.md": "# Quarterly planning\n",
    "Projects/Planning/Headcount.md": "# Headcount\n",
    "Projects/Onboarding revamp/Week one checklist.md": "# Week one checklist\n",
    "1-1s/Alice Chen/2026-11-04.md": (
        "# 1:1 — Alice Chen · Nov 4\n\n## Agenda\n"
        "- API migration: cutover plan and **rollback** window\n"
        "- [ ] Review promo packet draft\n- [x] Share Q4 roadmap\n"
    ),
    "1-1s/Alice Chen/2026-10-28.md": "# 1:1 — Alice Chen · Oct 28\n",
    "1-1s/Bob Ruiz/2026-11-03.md": "# 1:1 — Bob Ruiz\n\n- On-call load\n",
    "1-1s/Priya Nair/2026-11-02.md": "# 1:1 — Priya Nair\n\n- Design review\n",
    "Inbox/Triage/scratch.md": "scratch\n",
    "Inbox/Triage/Ideas for offsite.md": "# Ideas for offsite\n",
    "Reference/Promotions/Promo rubric.md": "# Promo rubric\n\nScope, impact, and influence.\n",
    "Reference/Runbooks/Gateway on-call.md": "# Gateway on-call\n",
    "Reading/Books/Thinking in Systems.md": "# Thinking in Systems\n",
}


def build_workspace(root: Path) -> None:
    if root.exists():
        shutil.rmtree(root)
    for relative, text in SAMPLE.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    now = time.time()
    for offset, relative in enumerate(SAMPLE):
        mtime = now - offset * 3600
        os.utime(root / relative, (mtime, mtime))


def capture(theme: str, workspace: Path, out: Path, engine: str) -> None:
    out.mkdir(parents=True, exist_ok=True)
    settings = f"editor-engine = {engine}\n"
    with WrenApp(workspace=workspace, evaluation=False, settings=settings) as app:
        client = app.client

        def shot(name: str) -> None:
            path = out / f"{name}.png"
            path.unlink(missing_ok=True)
            time.sleep(0.35)
            client.call("app.screenshot", {"path": str(path)})
            print(path, flush=True)

        client.call("theme.set", {"id": theme})
        wait_for(lambda: client.call("session.get")["notes"], timeout=10)
        note = next(
            row for row in client.call("session.get")["notes"]
            if row["path"] == FEATURE_NOTE
        )
        client.call("core.apply", {"op": "open_note", "note": note["id"]})
        shot("editor")
        for _ in range(3):
            client.call("command.run", {"id": "navigation.column_left"})
        shot("sections-focus")
        client.call("column.open", {"column": "sections"})
        client.call("palette.open", {"mode": "jump"})
        client.call("palette.query", {"query": "cutover"})
        client.call("palette.activate", {})
        for mode, query in [("commands", ""), ("search", "rollback"), ("themes", "")]:
            client.call("palette.open", {"mode": mode})
            if query:
                client.call("palette.query", {"query": query})
            shot(f"palette-{mode}")
            client.call("palette.close")
        client.call("view.focus", {"enabled": True})
        shot("focus")
        # A text selection across a heading, a task list, and a done task.
        client.call("editor.focus")
        client.call("editor.cursor", {
            "line": 11, "column": 20, "selection": {"line": 5, "column": 0},
        })
        shot("focus-selection")
        client.call("view.focus", {"enabled": False})


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/theme-scenes"))
    parser.add_argument("--workspace", type=Path, default=Path("/tmp/wren-design-workspace"))
    parser.add_argument("--themes", default="")
    parser.add_argument("--engine", choices=["classic", "advanced"], default="classic")
    args = parser.parse_args()

    workspace = args.workspace.resolve()
    build_workspace(workspace)
    themes = [t for t in args.themes.split(",") if t] or theme_ids()
    for theme in themes:
        capture(theme, workspace, (args.out / theme).resolve(), args.engine)
    return 0


def theme_ids() -> list[str]:
    source = (Path(__file__).resolve().parents[1]
              / "crates/wren-core/src/theme.rs").read_text(encoding="utf-8")
    ids = []
    for line in source.splitlines():
        line = line.strip()
        if line.startswith('id: "'):
            ids.append(line.split('"')[1])
    return ids


if __name__ == "__main__":
    raise SystemExit(main())
