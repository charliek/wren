"""The appcast update the release's appcast job runs."""

from contextlib import redirect_stderr
from datetime import datetime, timezone
import io
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

from tools import update_appcast

ROOT = Path(__file__).resolve().parents[2]
NS = {"sparkle": update_appcast.SPARKLE_NS}
SIGN = 'sparkle:edSignature="c2lnbmF0dXJl+/A=" length="7340032"\n'
EMPTY = (ROOT / "packaging/macos/appcast.xml").read_text()
NOW = datetime(2026, 11, 7, 12, 0, tzinfo=timezone.utc)


def items(appcast: str) -> list[ET.Element]:
    return ET.fromstring(appcast).findall("channel/item")


class UpdateAppcastTests(unittest.TestCase):
    def test_committed_appcast_is_an_empty_feed(self):
        self.assertEqual(items(EMPTY), [])

    def test_adds_a_signed_item_for_the_release_dmg(self):
        signature, length = update_appcast.parse_signature(SIGN)
        updated = update_appcast.update(EMPTY, "v0.1.0", signature, length, "charliek/wren", NOW)
        (item,) = items(updated)
        self.assertEqual(item.findtext("sparkle:version", namespaces=NS), "0.1.0")
        self.assertEqual(item.findtext("sparkle:shortVersionString", namespaces=NS), "0.1.0")
        self.assertEqual(item.findtext("pubDate"), "Sat, 07 Nov 2026 12:00:00 +0000")
        enclosure = item.find("enclosure")
        self.assertEqual(enclosure.get("url"),
                         "https://github.com/charliek/wren/releases/download/v0.1.0/Wren-0.1.0.dmg")
        self.assertEqual(enclosure.get(f"{{{update_appcast.SPARKLE_NS}}}edSignature"), "c2lnbmF0dXJl+/A=")
        self.assertEqual(enclosure.get("length"), "7340032")
        self.assertTrue(updated.startswith("<?xml version='1.0' encoding='utf-8'?>\n<rss"))
        self.assertIn('xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"', updated)

    def test_rerun_replaces_the_version_and_keeps_its_pubdate(self):
        first = update_appcast.update(EMPTY, "v0.1.0", "AAAA", "10", "charliek/wren", NOW)
        second = update_appcast.update(first, "v0.2.0", "BBBB", "20", "charliek/wren", NOW)
        later = datetime(2026, 12, 1, tzinfo=timezone.utc)
        rerun = update_appcast.update(second, "v0.1.0", "AAAA", "10", "charliek/wren", later)
        versions = [item.findtext("sparkle:version", namespaces=NS) for item in items(rerun)]
        self.assertEqual(sorted(versions), ["0.1.0", "0.2.0"])
        self.assertEqual(update_appcast.update(rerun, "v0.1.0", "AAAA", "10", "charliek/wren", later), rerun)
        self.assertNotIn("Tue, 01 Dec 2026", rerun)

    def test_prereleases_and_bad_input_are_refused(self):
        with self.assertRaisesRegex(ValueError, "prerelease"):
            update_appcast.update(EMPTY, "v0.1.0-rc1", "AAAA", "10", "charliek/wren", NOW)
        with self.assertRaisesRegex(ValueError, "not a release tag"):
            update_appcast.update(EMPTY, "0.1.0", "AAAA", "10", "charliek/wren", NOW)
        with self.assertRaisesRegex(ValueError, "no <channel>"):
            update_appcast.update("<rss/>", "v0.1.0", "AAAA", "10", "charliek/wren", NOW)
        for text in ("", 'length="10"', 'sparkle:edSignature="AAAA"', 'sparkle:edSignature="AAAA" length="0"'):
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, "sign_update output"):
                update_appcast.parse_signature(text)

    def test_cli_writes_the_file_and_fails_cleanly(self):
        with tempfile.TemporaryDirectory() as temp:
            appcast, sign = Path(temp) / "appcast.xml", Path(temp) / "sign.txt"
            appcast.write_text(EMPTY)
            sign.write_text(SIGN)
            self.assertEqual(update_appcast.main(["--tag", "v1.0.0", "--sign-file", str(sign),
                                                  "--appcast", str(appcast)]), 0)
            self.assertEqual(len(items(appcast.read_text())), 1)
            before = appcast.read_text()
            stderr = io.StringIO()
            with redirect_stderr(stderr):
                self.assertEqual(update_appcast.main(["--tag", "v1.0.1-rc1", "--sign-file", str(sign),
                                                      "--appcast", str(appcast)]), 1)
            self.assertIn("prerelease", stderr.getvalue())
            self.assertEqual(appcast.read_text(), before)


if __name__ == "__main__":
    unittest.main()
