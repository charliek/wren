"""Capture the main Wren screens for visual review against the mockup.

Builds the mockup's sample workspace in a private directory, launches the
smoke-test build under the test harness, and writes one PNG per scene:

    cargo build --locked -p wren-iced --features smoke-test
    python3 tools/capture_scenes.py --theme wren-dark --out target/scenes

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
    "1-1s/Team sync.md": "# Team sync\n",
    "Projects/API Migration/Design notes.md": "# Design notes\n\nRead path first.\n",
    "Projects/API Migration/Cutover plan.md": "# Cutover plan\n",
    "Projects/Quarterly planning.md": "# Quarterly planning\n",
    "Inbox/scratch.md": "scratch\n",
    "Reference/Promo rubric.md": "# Promo rubric\n\nScope, impact, and influence.\n",
}


def build_workspace(root: Path) -> None:
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
    args = parser.parse_args()
    out = (args.out / args.theme).resolve()
    out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory() as temp:
        workspace = Path(temp).resolve() / "work"
        build_workspace(workspace)
        with WrenApp(workspace=workspace, evaluation=False) as app:
            client = app.client
            client.call("theme.set", {"id": args.theme})
            wait_for(lambda: client.call("session.get")["notes"], timeout=10)

            def shot(name: str) -> None:
                path = out / f"{name}.png"
                path.unlink(missing_ok=True)
                time.sleep(0.3)
                client.call("app.screenshot", {"path": str(path)})
                print(path)

            note = next(
                row for row in client.call("session.get")["notes"]
                if row["path"].endswith("2026-11-04.md")
            )
            client.call("core.apply", {"op": "open_note", "note": note["id"]})
            time.sleep(0.3)
            shot("columns")
            client.call("view.layout", {"layout": "outline"})
            shot("outline")
            client.call("view.layout", {"layout": "columns"})
            for mode, query in [
                ("commands", ""),
                ("jump", "al"),
                ("search", "promo"),
                ("workspaces", ""),
                ("themes", ""),
            ]:
                client.call("palette.open", {"mode": mode})
                if query:
                    client.call("palette.query", {"query": query})
                shot(f"palette-{mode}")
                client.call("palette.close")
            client.call("view.focus", {"enabled": True})
            shot("focus")
            client.call("view.focus", {"enabled": False})
            wait_for(lambda: True, timeout=1)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
