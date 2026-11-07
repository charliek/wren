"""The release asset names and the draft check publish-release runs."""

from contextlib import redirect_stdout
import io
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

from tools import release_assets

ROOT = Path(__file__).resolve().parents[2]
MIB = release_assets.MIN_ARTIFACT_BYTES


def draft(tag: str, size: int = 8 * MIB) -> list[tuple[str, int]]:
    return [(name, size) for name in sorted(release_assets.expected_assets(tag))]


class NameTests(unittest.TestCase):
    def test_stable_names(self):
        self.assertEqual(release_assets.deb_name("v0.1.0", "amd64"), "wren_0.1.0_amd64.deb")
        self.assertEqual(release_assets.deb_name("v0.1.0", "arm64"), "wren_0.1.0_arm64.deb")
        self.assertEqual(release_assets.dmg_name("v0.1.0"), "Wren-0.1.0.dmg")

    def test_prerelease_deb_replaces_only_the_first_hyphen_like_nfpm(self):
        self.assertEqual(release_assets.deb_name("v1.2.3-rc-2", "amd64"), "wren_1.2.3~rc-2_amd64.deb")
        self.assertEqual(release_assets.dmg_name("v1.2.3-rc-2"), "Wren-1.2.3-rc-2.dmg")
        # GitHub stores the `~` as `.`.
        self.assertIn("wren_1.2.3.rc-2_amd64.deb", release_assets.expected_assets("v1.2.3-rc-2"))

    def test_invalid_tags_and_arches_are_rejected(self):
        for tag in ("0.1.0", "v0.1", "v01.0.0", "v0.1.0-", "v0.1.0+meta", "v0.1.0-rc..1", "refs/tags/v0.1.0"):
            with self.subTest(tag=tag):
                with self.assertRaises(ValueError):
                    release_assets.dmg_name(tag)
        with self.assertRaisesRegex(ValueError, "architecture"):
            release_assets.deb_name("v0.1.0", "i386")

    def test_expected_set_is_both_debs_and_the_dmg(self):
        self.assertEqual(release_assets.expected_assets("v0.1.0"),
                         {"wren_0.1.0_amd64.deb", "wren_0.1.0_arm64.deb", "Wren-0.1.0.dmg"})


class CheckTests(unittest.TestCase):
    def test_complete_draft_passes(self):
        self.assertEqual(release_assets.problems("v0.1.0", draft("v0.1.0")), [])
        self.assertEqual(release_assets.problems("v0.1.0-rc1", draft("v0.1.0-rc1")), [])

    def test_missing_duplicate_truncated_and_stale_assets_fail(self):
        assets = draft("v0.1.0")
        missing = [asset for asset in assets if not asset[0].endswith(".dmg")]
        self.assertRegex(release_assets.problems("v0.1.0", missing)[0], "exactly one asset named Wren-0.1.0.dmg.*found 0")
        self.assertRegex(release_assets.problems("v0.1.0", assets + assets[:1])[0], "found 2")
        truncated = [(name, MIB - 1 if name.endswith(".dmg") else size) for name, size in assets]
        self.assertRegex(release_assets.problems("v0.1.0", truncated)[0], "looks truncated")
        stale = assets + [("wren_0.0.9_amd64.deb", 8 * MIB)]
        self.assertEqual(release_assets.problems("v0.1.0", stale),
                         ["unexpected asset wren_0.0.9_amd64.deb on v0.1.0 (a stale upload from an earlier attempt?)."])

    def test_unsanitized_prerelease_name_is_not_what_github_stores(self):
        assets = [("wren_0.1.0~rc1_amd64.deb" if name.endswith("amd64.deb") else name, size)
                  for name, size in draft("v0.1.0-rc1")]
        self.assertEqual(len(release_assets.problems("v0.1.0-rc1", assets)), 2)

    def test_parse_rejects_malformed_lines_and_skips_blank_ones(self):
        self.assertEqual(release_assets.parse_assets(["a.deb\t12\n", "\n"]), [("a.deb", 12)])
        for line in ("a.deb 12", "a.deb\t", "a.deb\t-1", "a.deb\t1.5"):
            with self.subTest(line=line):
                with self.assertRaisesRegex(ValueError, "name<TAB>size"):
                    release_assets.parse_assets([line])

    def test_cli(self):
        script = str(ROOT / "tools/release_assets.py")
        for args, out in ((["deb-name", "v0.2.0-rc1", "arm64"], "wren_0.2.0~rc1_arm64.deb\n"),
                          (["dmg-name", "v0.2.0"], "Wren-0.2.0.dmg\n")):
            result = subprocess.run([sys.executable, script, *args], capture_output=True, text=True, check=False)
            self.assertEqual((result.returncode, result.stdout), (0, out), result.stderr)
        good = "".join(f"{name}\t{size}\n" for name, size in draft("v0.2.0"))
        result = subprocess.run([sys.executable, script, "check", "v0.2.0"], input=good,
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("all 3 expected assets", result.stdout)
        result = subprocess.run([sys.executable, script, "check", "v0.2.0"], input=good + "extra.txt\t9\n",
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("::error::unexpected asset extra.txt", result.stdout)

    def test_cli_reports_bad_input_as_an_annotation(self):
        stdout = io.StringIO()
        with patch.object(sys, "stdin", io.StringIO("broken line\n")), redirect_stdout(stdout):
            self.assertEqual(release_assets.main(["check", "v0.2.0"]), 1)
        self.assertIn("::error::line 1", stdout.getvalue())
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            self.assertEqual(release_assets.main(["dmg-name", "0.2.0"]), 1)
        self.assertIn("::error::not a release tag", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
