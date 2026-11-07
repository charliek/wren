"""Check Wren's Sparkle updater end to end on a Mac.

    cargo build --features smoke-test
    third_party/sparkle/fetch.sh
    python3 tools/check_updater.py

Builds a throwaway Wren.app around the smoke-test binary with Sparkle
embedded, trusting the TEST-ONLY key in tools/wrentest/fixtures/sparkle and
polling a feed served from 127.0.0.1. Then, through the test socket, it checks
that the updater starts with "Check for Updates…" enabled, that a feed
offering a newer version is reported as an available update, and that a feed
offering only an older one is reported as up to date. The bare binary, with no
framework beside it, must report no updater and a disabled menu item. Downloading and installing an update is Sparkle's UI and is
left to a manual check.

macOS only, and it needs a logged-in console session (it opens a window).
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import functools
import http.server
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools import package, update_appcast  # noqa: E402
from tools.wrentest.ui import WrenApp, wait_for  # noqa: E402

FIXTURES = ROOT / "tools/wrentest/fixtures/sparkle"
SIGN_UPDATE = ROOT / "third_party/sparkle/out/bin/sign_update"
EMPTY_APPCAST = update_appcast.without_releases((ROOT / "packaging/macos/appcast.xml").read_text(encoding="utf-8"))


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):  # noqa: A002 - the base class's name
        pass


def publish(feed: Path, version: str) -> None:
    """Write an appcast offering `version`, signed with the TEST-ONLY key."""
    dmg = feed / f"Wren-{version}.dmg"
    dmg.write_bytes(os.urandom(4096))
    signed = subprocess.run(
        [str(SIGN_UPDATE), "--ed-key-file", str(FIXTURES / "TEST-ONLY-private-ed-key.txt"), str(dmg)],
        check=True, capture_output=True, text=True,
    ).stdout
    signature, length = update_appcast.parse_signature(signed)
    appcast = update_appcast.update(EMPTY_APPCAST, f"v{version}", signature, length, "charliek/wren",
                                    datetime.now(timezone.utc))
    (feed / "appcast.xml").write_text(appcast, encoding="utf-8")


def build_bundle(binary: Path, destination: Path, feed_url: str) -> Path:
    key = (FIXTURES / "TEST-ONLY-public-ed-key.txt").read_text(encoding="utf-8")
    app = package.assemble_bundle(binary, destination, "macos", root=ROOT,
                                  dependency_notices=b"check_updater fixture bundle\n",
                                  sparkle=package.sparkle_feed(feed_url, key))
    # Settled up front, so Sparkle never opens its "check automatically?"
    # prompt, which holds an update session open and blocks the checks below.
    subprocess.run(["/usr/libexec/PlistBuddy", "-c", "Add :SUEnableAutomaticChecks bool false",
                    str(app / "Contents/Info.plist")], check=True)
    env = {name: value for name, value in os.environ.items() if name != "WREN_DEVELOPER_ID_IDENTITY"}
    for script in ("embed-sparkle.sh", "sign-app.sh"):
        subprocess.run([str(ROOT / "scripts/macos" / script), str(app)], check=True, env=env,
                       stdout=subprocess.DEVNULL)
    return app


def check(client, expected: str, version: str | None) -> None:
    before = client.call("updater.status")["checks"]
    client.call("updater.check")
    status = wait_for(lambda: (lambda s: s if s["checks"] > before else None)(client.call("updater.status")),
                      timeout=60, description="the update check to finish")
    last = status["last"]
    if last["outcome"] != expected or (version is not None and last["version"] != version):
        raise SystemExit(f"FAIL: expected {expected} {version or ''}, got {last}")
    print(f"ok: {expected}{f' {version}' if version else ''} ({last['detail'] or 'no detail'})")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/wren",
                        help="a smoke-test build of wren (default: target/debug/wren)")
    args = parser.parse_args()
    if sys.platform != "darwin":
        print("check_updater.py runs on macOS only", file=sys.stderr)
        return 1
    if not SIGN_UPDATE.is_file():
        print("Sparkle is not staged; run third_party/sparkle/fetch.sh", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as temp:
        temp = Path(temp).resolve()
        feed = temp / "feed"
        feed.mkdir()
        handler = functools.partial(QuietHandler, directory=str(feed))
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            feed_url = f"http://127.0.0.1:{server.server_address[1]}/appcast.xml"
            app = build_bundle(args.binary.resolve(), temp / "bundle/Wren.app", feed_url)
            print(f"ok: built {app} polling {feed_url}")
            os.environ["WREN_TEST_BIN"] = str(app / "Contents/MacOS/wren")
            workspace = temp / "work"
            workspace.mkdir()
            with WrenApp(workspace=workspace, evaluation=False) as running:
                client = running.client
                status = client.call("updater.status")
                if not (status["started"] and status["can_check"] and status["menu_enabled"]):
                    raise SystemExit(f"FAIL: the updater did not start: {status}")
                print("ok: the updater started and Check for Updates… is enabled")
                publish(feed, "9999.0.0")
                check(client, "found", "9999.0.0")
                publish(feed, "0.0.0")
                check(client, "up_to_date", None)
        finally:
            server.shutdown()

        os.environ["WREN_TEST_BIN"] = str(args.binary.resolve())
        with WrenApp(workspace=workspace, evaluation=False) as running:
            status = running.client.call("updater.status")
            if status["started"] or status["menu_enabled"] is not False or not status["unavailable"]:
                raise SystemExit(f"FAIL: a bare binary should have no updater: {status}")
            print(f"ok: the bare binary has no updater ({status['unavailable']})")
    print("PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
