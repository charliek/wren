"""Keep the release workflow, its Makefile targets, scripts and packaging in agreement.

These read the files as text, like test_ci_contract.py (no YAML dependency in
the locked test environment), and pin the release-workflows invariants: the
draft-until-complete job graph, the ci-success gate, all-or-nothing macOS
notarization, the Sparkle feed and key every Wren.app carries, and a .deb
whose Depends cover every library the binary loads.
"""

import os
from pathlib import Path
import re
import unittest

from tools import package
from tools.wrentest_unit.test_ci_contract import job_blocks

ROOT = Path(__file__).resolve().parents[2]
CI = ROOT / ".github/workflows/ci.yml"
RELEASE = ROOT / ".github/workflows/release.yml"
SANITY = ROOT / ".github/workflows/sanity-check-app.yml"
APPLE_SECRETS = {"MACOS_CERTIFICATE_P12_BASE64", "MACOS_CERTIFICATE_PASSWORD", "WREN_DEVELOPER_ID_IDENTITY",
                 "APPLE_ID", "APPLE_TEAM_ID", "APPLE_APP_SPECIFIC_PASSWORD"}
GATED_IDENTITY = "${{ env.CAN_NOTARIZE == 'true' && secrets.WREN_DEVELOPER_ID_IDENTITY || '' }}"
# The sonames the release binary dlopens (winit, wgpu, xkbcommon-dl), mapped to
# the Ubuntu 24.04 package that ships each.
DLOPENED = {"libxkbcommon.so.0": "libxkbcommon0", "libxkbcommon-x11.so.0": "libxkbcommon-x11-0",
            "libwayland-client.so.0": "libwayland-client0", "libwayland-egl.so.1": "libwayland-egl1",
            "libX11.so.6": "libx11-6", "libX11-xcb.so.1": "libx11-xcb1", "libXcursor.so.1": "libxcursor1",
            "libXi.so.6": "libxi6", "libEGL.so.1": "libegl1"}


def needs(block: str) -> set[str]:
    match = re.search(r"^    needs: (?:\[([^\]]+)\]|([\w-]+))$", block, re.MULTILINE)
    if match is None:
        return set()
    return {name.strip() for name in (match[1] or match[2]).split(",")}


def steps(block: str) -> list[dict[str, object]]:
    """Each step's name, `if:` and env keys, from the job's six-space step items."""
    found = []
    for chunk in re.split(r"(?m)^      - ", block.split("\n    steps:\n", 1)[1])[1:]:
        name = re.search(r"(?m)^(?:        )?name: (.+)$", chunk)
        condition = re.search(r"(?m)^        if: (.+)$", chunk)
        env = re.search(r"(?m)^        env:\n((?:          [^\n]*\n)+)", chunk)
        found.append({
            "name": name[1] if name else "",
            "if": condition[1] if condition else None,
            "env": set(re.findall(r"(?m)^          ([A-Z0-9_]+):", env[1])) if env else set(),
        })
    return found


def make_target(makefile: str, target: str) -> str:
    match = re.search(rf"(?m)^{re.escape(target)}:[^\n]*\n((?:\t[^\n]*\n)+)", makefile)
    assert match is not None, target
    return match[1]


def yaml_list(source: str, key: str) -> list[str]:
    block = re.search(rf"(?m)^{key}:\n((?:(?:  - [^\n]*|  #[^\n]*)\n)+)", source)
    assert block is not None, key
    return re.findall(r"(?m)^  - (.+)$", block[1])


class ReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.source = RELEASE.read_text()
        self.jobs = job_blocks(self.source)
        self.makefile = (ROOT / "Makefile").read_text()

    def test_tag_trigger_and_draft_until_complete_job_graph(self):
        self.assertIn("on:\n  push:\n    tags:\n      - 'v*'\n", self.source)
        self.assertIn("\npermissions:\n  contents: read\n", self.source)
        self.assertEqual({name: needs(block) for name, block in self.jobs.items()}, {
            "version-check": set(), "ci-gate": set(),
            "create-release": {"version-check", "ci-gate"},
            "linux": {"create-release"}, "mac": {"create-release"},
            "publish-release": {"linux", "mac"},
            "dispatch-apt-charliek": {"publish-release"}, "appcast": {"publish-release"},
        })
        # No job may run past a failed dependency: that keeps a failed build's
        # draft unpublished and the apt dispatch from firing early.
        for name, block in self.jobs.items():
            self.assertNotRegex(block, r"(?m)^    if:", name)
            self.assertNotIn("continue-on-error:", block, name)
        self.assertIn("--draft --title", self.jobs["create-release"])
        self.assertIn("--draft=false", self.jobs["publish-release"])
        self.assertIn('| python3 tools/release_assets.py check "${tag}"', self.jobs["publish-release"])

    def test_ci_gate_requires_the_ci_success_aggregate_on_the_tagged_commit(self):
        gate = self.jobs["ci-gate"]
        self.assertIn("commits/${GITHUB_SHA}/check-runs?check_name=ci-success", gate)
        self.assertIn('[ "${conclusion}" = "success" ] && {', gate)
        self.assertIn("checks: read", gate)
        self.assertIn("ci-success", job_blocks(CI.read_text()))

    def test_version_check_compares_the_full_tag_to_the_workspace_version(self):
        check = self.jobs["version-check"]
        self.assertIn('["workspace"]["package"]["version"]', check)
        self.assertIn('if [ "${tag#v}" != "${cargo_ver}" ]; then', check)

    def test_build_jobs_use_make_targets_and_scripts_that_exist(self):
        linux, mac = self.jobs["linux"], self.jobs["mac"]
        self.assertIn('run: make deb ARCH="${{ matrix.arch }}" RELEASE_VERSION="${GITHUB_REF_NAME#v}"', linux)
        self.assertIn('scripts/linux/verify-deb.sh "${deb}" "${GITHUB_REF_NAME#v}" --clean-install', linux)
        self.assertIn('run: make app RELEASE_VERSION="${GITHUB_REF_NAME#v}"', mac)
        self.assertIn('run: make dmg RELEASE_VERSION="${GITHUB_REF_NAME#v}"', mac)
        self.assertEqual(set(re.findall(r"- \{ runner: ([\w.-]+), +arch: (\w+),", linux)),
                         {("ubuntu-24.04", "amd64"), ("ubuntu-24.04-arm", "arm64")})
        self.assertRegex(mac, r"(?m)^    runs-on: macos-\d+")
        self.assertIn('lipo "${APP}/Contents/MacOS/wren" -verify_arch arm64', mac)
        for target in ("release-binary", "deb", "app", "dmg"):
            self.assertRegex(self.makefile, rf"(?m)^{target}:")
        for path in set(re.findall(r"\b(?:tools|scripts|packaging)/[\w./-]+", self.source + self.makefile)):
            path = path.rstrip(".")
            self.assertTrue((ROOT / path).exists(), path)
            if path.endswith(".sh"):
                self.assertTrue(os.access(ROOT / path, os.X_OK), path)

    def test_release_targets_build_from_package_py_with_the_release_version(self):
        deb, app = make_target(self.makefile, "deb"), make_target(self.makefile, "app")
        self.assertIn('--system-layout --release-version "$(RELEASE_VERSION)" --output target/deb-root/usr', deb)
        self.assertIn('WREN_DEB_VERSION="$(RELEASE_VERSION)" WREN_DEB_ARCH="$(ARCH)"', deb)
        self.assertIn("--config packaging/nfpm.yaml", deb)
        self.assertIn('--release-version "$(RELEASE_VERSION)" --output target/dist/macos/Wren.app', app)
        self.assertIn("scripts/macos/sign-app.sh target/dist/macos/Wren.app", app)
        self.assertIn('"target/dist/Wren-$(RELEASE_VERSION).dmg"', make_target(self.makefile, "dmg"))
        for block in (deb, app):
            self.assertIn("$(MAKE) release-binary", block)
        self.assertIn("cargo build --locked --release -p wren-iced", make_target(self.makefile, "release-binary"))

    def test_mac_signing_and_notarization_are_all_or_nothing(self):
        mac = self.jobs["mac"]
        gate = re.search(r"(?m)^      CAN_NOTARIZE: \$\{\{ (.+) \}\}$", mac)
        self.assertIsNotNone(gate)
        self.assertEqual(set(re.findall(r"secrets\.([A-Z0-9_]+) != ''", gate[1])), APPLE_SECRETS)
        self.assertEqual(gate[1].count("&&"), len(APPLE_SECRETS) - 1)
        self.assertNotIn("||", gate[1])
        self.assertLessEqual(set(re.findall(r"secrets\.([A-Z0-9_]+)", mac)), APPLE_SECRETS | {"SPARKLE_ED_PRIVATE_KEY"})
        # The identity reaches a step only through the gate, so any missing
        # secret falls back to an ad-hoc signature and the first-launch note.
        self.assertEqual(re.findall(r"WREN_DEVELOPER_ID_IDENTITY: (.+)", mac), [GATED_IDENTITY] * 2)
        mac_steps = steps(mac)
        for step in mac_steps:
            apple = step["env"] & (APPLE_SECRETS - {"WREN_DEVELOPER_ID_IDENTITY"})
            if apple or str(step["name"]).startswith(("Notarize", "Import", "Assert the app carries")):
                self.assertEqual(step["if"], "env.CAN_NOTARIZE == 'true'", step["name"])
        names = [step["name"] for step in mac_steps]
        order = ["Import Developer ID certificate", "Build and sign Wren.app",
                 "Notarize + staple the app (skipped without credentials)", "Make DMG",
                 "Notarize + staple the DMG (skipped without credentials)", "Upload DMG to the Release"]
        positions = [names.index(name) for name in order]
        self.assertEqual(positions, sorted(positions))

    def test_bundle_identity_is_the_one_package_py_writes(self):
        self.assertEqual(package.BUNDLE_ID, "ai.stridelabs.Wren")
        self.assertIn(f"check CFBundleIdentifier {package.BUNDLE_ID}", self.jobs["mac"])
        entitlements = (ROOT / "packaging/macos/Wren.entitlements").read_text()
        self.assertNotIn("com.apple.security.automation.apple-events", entitlements)
        self.assertIn("--options runtime --timestamp", (ROOT / "scripts/macos/sign-app.sh").read_text())

    def test_sparkle_feed_key_and_signature_flow(self):
        mac, app = self.jobs["mac"], make_target(self.makefile, "app")
        feed = re.search(r"(?m)^SPARKLE_FEED_URL \?= (\S+)$", self.makefile)[1]
        self.assertEqual(feed, "https://raw.githubusercontent.com/charliek/wren/main/packaging/macos/appcast.xml")
        self.assertIn(f"check SUFeedURL {feed}", mac)
        self.assertIn("check SUPublicEDKey \"$(tr -d '[:space:]' < packaging/macos/sparkle-ed-public-key.txt)\"", mac)
        self.assertIn("SPARKLE_PUBLIC_KEY ?= packaging/macos/sparkle-ed-public-key.txt", self.makefile)
        self.assertNotEqual((ROOT / "packaging/macos/sparkle-ed-public-key.txt").read_text(),
                            (ROOT / "tools/wrentest/fixtures/sparkle/TEST-ONLY-public-ed-key.txt").read_text())
        # make app: fetch, bundle with the feed, embed the framework, then sign.
        order = ["third_party/sparkle/fetch.sh", "--sparkle-feed-url", "scripts/macos/embed-sparkle.sh",
                 "scripts/macos/sign-app.sh"]
        self.assertEqual([app.index(part) for part in order], sorted(app.index(part) for part in order))
        names = [step["name"] for step in steps(mac)]
        order = ["Stage Sparkle", "Verify the Sparkle key pair", "Build and sign Wren.app",
                 "Notarize + staple the DMG (skipped without credentials)", "Upload DMG to the Release",
                 "Sign the DMG for Sparkle", "Hand the signature to the appcast job"]
        positions = [names.index(name) for name in order]
        self.assertEqual(positions, sorted(positions))
        self.assertIn("name: sparkle-signature", mac)

        appcast = self.jobs["appcast"]
        self.assertIn("name: sparkle-signature", appcast)
        self.assertIn("ref: main", appcast)
        self.assertIn("*-*)", appcast)  # prereleases never enter the appcast
        for step in steps(appcast)[1:]:
            self.assertEqual(step["if"], "steps.guard.outputs.skip == 'false'", step["name"])
        self.assertIn("client-id: ${{ secrets.RELEASE_BOT_CLIENT_ID }}", appcast)
        self.assertIn("permission-contents: write", appcast)
        self.assertIn("python3 tools/update_appcast.py --tag", appcast)
        self.assertIn('[ "${staged}" = "${appcast}" ]', appcast)
        self.assertIn("appcast=packaging/macos/appcast.xml", appcast)
        self.assertLess(appcast.index("Verify the DMG download resolves"), appcast.index("git push"))

    def test_apt_dispatch_uses_a_receiver_scoped_release_bot_token(self):
        dispatch = self.jobs["dispatch-apt-charliek"]
        self.assertIn("client-id: ${{ secrets.RELEASE_BOT_CLIENT_ID }}", dispatch)
        self.assertIn("private-key: ${{ secrets.RELEASE_BOT_APP_KEY }}", dispatch)
        self.assertIn("repositories: apt-charliek", dispatch)
        self.assertIn('-F "client_payload[package]=wren"', dispatch)
        self.assertIn("repos/charliek/apt-charliek/dispatches", dispatch)
        self.assertIn("*-*)", dispatch)  # prereleases never reach the apt repo

    def test_nfpm_packages_the_tree_package_py_stages_with_complete_depends(self):
        nfpm = (ROOT / "packaging/nfpm.yaml").read_text()
        for line in ("name: wren", "arch: ${WREN_DEB_ARCH}", "version: ${WREN_DEB_VERSION}", "version_schema: semver"):
            self.assertIn(f"\n{line}\n", nfpm)
        self.assertIn("contents:\n"
                      "  # The whole staged /usr tree. Directory entries (/usr, /usr/share, ...) carry\n"
                      "  # the stage's 0755 modes, as any dpkg-built package's do.\n"
                      "  - src: ./target/deb-root/usr\n    dst: /usr\n    type: tree\n", nfpm)
        # Every library the binary dlopens (and scripts/linux/verify-deb.sh
        # resolves in a clean container) must come from a declared Depends.
        verify = (ROOT / "scripts/linux/verify-deb.sh").read_text()
        self.assertEqual(set(re.search(r'(?m)^  sonames="([^"]+)"$', verify)[1].split()), set(DLOPENED))
        depends = yaml_list(nfpm, "depends")
        self.assertIn("libc6 (>= 2.35)", depends)
        self.assertEqual({entry.split(" ", 1)[0] for entry in depends} - {"libc6", "libgcc-s1"},
                         set(DLOPENED.values()))
        self.assertEqual(set(yaml_list(nfpm, "recommends")), {"libvulkan1", "mesa-vulkan-drivers", "xdg-desktop-portal"})


class SanityCheckWorkflowTests(unittest.TestCase):
    def test_manual_only_and_reaches_self_and_apt_charliek(self):
        source = SANITY.read_text()
        self.assertIn("\non:\n  workflow_dispatch: {}\n", source)
        self.assertIn("\npermissions: {}", source)
        tokens = re.findall(r"(?m)^      - id: ([\w-]+)\n        uses: actions/create-github-app-token@v3\n", source)
        self.assertEqual(tokens, ["self-token", "apt-token"])
        self.assertEqual(source.count("repositories:"), 1)
        self.assertIn("owner:        charliek\n          repositories: apt-charliek\n", source)
        self.assertIn('gh api "/repos/charliek/apt-charliek" --jq', source)


class ReleaseFilesTests(unittest.TestCase):
    def test_update_version_script_follows_the_convention_contract(self):
        script = ROOT / "scripts/release/update-version.sh"
        self.assertTrue(os.access(script, os.X_OK))
        source = script.read_text()
        self.assertIn('"${cargo[@]}" update --workspace --offline', source)
        self.assertIn(r"/^\[workspace\.package\]/,/^\[/", source)
        # Every crate's lock entry is checked, including ones added later.
        self.assertIn("for manifest in crates/*/Cargo.toml; do", source)
        self.assertNotRegex(source, r"(?m)^[^#]*\bgit (add|commit)\b")

    def test_changelog_has_an_unreleased_section_and_releasing_documents_the_secrets(self):
        self.assertIn("\n## [Unreleased]\n", (ROOT / "CHANGELOG.md").read_text())
        releasing = (ROOT / "RELEASING.md").read_text()
        for secret in ("RELEASE_BOT_CLIENT_ID", "RELEASE_BOT_APP_KEY", "SPARKLE_ED_PRIVATE_KEY", *APPLE_SECRETS):
            self.assertIn(f"`{secret}`", releasing)


if __name__ == "__main__":
    unittest.main()
