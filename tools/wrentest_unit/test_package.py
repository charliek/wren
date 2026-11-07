"""Bundle-layout and safety tests, not evidence of native application execution."""

from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET
import zlib

from tools import dependency_notices, package

ROOT = Path(__file__).resolve().parents[2]
ICON_ASSETS = (package.ICON_SVG, package.ICON_ICNS, *map(package.hicolor_icon, package.ICON_SIZES))
# PNG-payload ICNS element types written by tools/generate_icons.py.
ICNS_SIZES = {b"icp4": 16, b"icp5": 32, b"ic11": 32, b"ic12": 64, b"ic07": 128,
              b"ic13": 256, b"ic08": 256, b"ic14": 512, b"ic09": 512, b"ic10": 1024}


def snapshot(root: Path) -> dict[str, tuple[bytes, int, int]]:
    return {str(path.relative_to(root)): (path.read_bytes(), stat.S_IMODE(path.stat().st_mode),
                                        path.stat().st_mtime_ns)
            for path in root.rglob("*") if path.is_file()}


def native_fixture(platform: str) -> bytes:
    # These headers exercise format rejection only; neither fixture is runnable.
    if platform == "linux":
        return b"\x7fELF\x02\x01\x01" + bytes(9) + b"\x03\x00" + bytes(46)
    return b"\xcf\xfa\xed\xfe" + bytes(8) + b"\x02\x00\x00\x00" + bytes(16)


def png_header(data: bytes) -> tuple[int, int, int, int]:
    assert data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR"
    width, height, depth, color = struct.unpack_from(">IIBB", data, 16)
    return width, height, depth, color


def decode_rgba(data: bytes) -> tuple[int, bytes]:
    """Decode a non-interlaced RGBA8 PNG with any row filters (the icon outputs)."""
    width, height, depth, color = png_header(data)
    assert (depth, color, data[28]) == (8, 6, 0)
    offset, compressed = 8, b""
    while offset < len(data):
        length, kind = struct.unpack_from(">I4s", data, offset)
        if kind == b"IDAT":
            compressed += data[offset + 8:offset + 8 + length]
        offset += length + 12
    raw, stride, rows, previous = zlib.decompress(compressed), width * 4, bytearray(), bytearray(width * 4)
    for y in range(height):
        kind, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            left = line[i - 4] if i >= 4 else 0
            up, corner = previous[i], previous[i - 4] if i >= 4 else 0
            if kind == 4:
                estimate = left + up - corner
                pa, pb, pc = abs(estimate - left), abs(estimate - up), abs(estimate - corner)
                predictor = left if pa <= pb and pa <= pc else up if pb <= pc else corner
            else:
                predictor = (0, left, up, (left + up) // 2)[kind]
            line[i] = (line[i] + predictor) & 0xFF
        rows += line
        previous = line
    return width, bytes(rows)


def icns_images(data: bytes) -> dict[bytes, bytes]:
    assert data[:4] == b"icns" and struct.unpack_from(">I", data, 4)[0] == len(data)
    offset, images = 8, {}
    while offset < len(data):
        kind, length = struct.unpack_from(">4sI", data, offset)
        assert 8 < length <= len(data) - offset
        images[kind] = data[offset + 8:offset + length]
        offset += length
    return images


def desktop_values(path: Path) -> dict[str, str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    assert lines[0] == "[Desktop Entry]"
    return dict(line.split("=", 1) for line in lines[1:] if line)


def desktop_unescape(value: str) -> str:
    escapes = {"s": " ", "n": "\n", "t": "\t", "r": "\r", "\\": "\\"}
    return re.sub(r"\\([sntr\\])", lambda match: escapes[match[1]], value)


def desktop_exec_argument(value: str) -> str:
    # Undo the desktop string layer, then the single quoted Exec argument.
    value = desktop_unescape(value)
    assert value.startswith('"') and value.endswith('"')
    value = re.sub(r'\\(["`$\\])', lambda match: match[1], value[1:-1])
    assert not re.search(r"(?<!%)%(?!%)", value)
    return value.replace("%%", "%")


class PackageTests(unittest.TestCase):
    def setUp(self):
        # /var and /tmp can be symlink aliases on macOS; fixtures deliberately
        # use their canonical path, as the no-symlink packaging API requires.
        self.temp = tempfile.TemporaryDirectory(prefix="wren-package-", dir=Path(tempfile.gettempdir()).resolve())
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.source = self.base / "source with spaces"
        self.source.mkdir()
        for relative in (*ICON_ASSETS, "assets/wren.desktop", *package.FONT_NOTICES):
            target = self.source / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        (self.source / "Cargo.toml").write_text('[workspace.package]\nversion = "0.0.1"\n')
        app = self.source / "crates/wren-iced/Cargo.toml"
        app.parent.mkdir(parents=True)
        app.write_text('[package]\nname = "wren-iced"\nversion.workspace = true\n')
        self.binary = self.source / "build output/wren"
        self.binary.parent.mkdir()
        self.binary.write_bytes(b"fixture only; this is deliberately not a runnable Wren binary\n")
        self.binary.chmod(0o700)
        self.original = snapshot(self.source)
        crate = self.base / "fake private registry/fixture-dependency"
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text('[package]\nname = "fixture-dependency"\nversion = "1.2.3"\n')
        (crate / "LICENSE").write_text("Fixture copyright and license text for bundle-layout tests only.\n")
        self.metadata = {
            "workspace_members": ["workspace-wren"],
            "packages": [
                {"id": "workspace-wren", "name": "wren-iced", "version": "0.0.1", "license": None},
                {"id": "private-registry-fixture", "name": "fixture-dependency", "version": "1.2.3",
                 "license": "MIT", "manifest_path": str(crate / "Cargo.toml")},
            ],
        }
        self.report = dependency_notices.notices_from_metadata(self.metadata)
        self.dependency_text = self.report.render()
        collector = patch.object(package, "collect_notices", return_value=self.report)
        collector.start()
        self.addCleanup(collector.stop)
        fake_bin = self.base / "fake tools"
        fake_bin.mkdir()
        cargo = fake_bin / "cargo"
        cargo.write_text(f"#!{sys.executable}\nimport sys\n"
                         "assert sys.argv[1:] == ['metadata', '--locked', '--format-version', '1', '--offline']\n"
                         f"sys.stdout.write({json.dumps(self.metadata)!r})\n")
        cargo.chmod(0o700)
        self.fake_env = {**os.environ, "PATH": str(fake_bin) + os.pathsep + os.environ.get("PATH", "")}

    def assemble(self, platform="linux", output=None, **kwargs):
        if output is None:
            output = self.base / "bundle with spaces" / ("Wren.app" if platform == "macos" else "wren-linux")
        kwargs.setdefault("dependency_notices", self.dependency_text)
        return package.assemble_bundle(self.binary, output, platform, root=self.source, **kwargs)

    def assert_source_unchanged(self):
        self.assertEqual(snapshot(self.source), self.original)

    def assert_permissions(self, output: Path, binary: Path):
        for path in (output, *output.rglob("*")):
            self.assertFalse(path.is_symlink(), path)
            expected = 0o755 if path.is_dir() or path == binary else 0o644
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), expected, path)

    def assert_notices(self, directory: Path):
        for notice in package.FONT_NOTICES:
            self.assertEqual((directory / notice).read_bytes(), (ROOT / notice).read_bytes())
        text = (directory / "THIRD_PARTY_NOTICES.txt").read_text()
        for name in ("Inter", "JetBrains Mono", "SIL Open Font License", *map(str, package.FONT_NOTICES)):
            self.assertIn(name, text)
        self.assertIn("DEPENDENCY_NOTICES.txt", text)
        self.assertEqual((directory / "DEPENDENCY_NOTICES.txt").read_bytes(), self.dependency_text)
        self.assertIn("fixture-dependency", self.dependency_text.decode("utf-8"))
        self.assertNotIn(str(self.base), self.dependency_text.decode("utf-8"))
        self.assertFalse(list(directory.rglob("*.ttf")))

    def test_linux_layout_desktop_icon_permissions_and_notices(self):
        output = self.assemble()
        binary = output / "bin/wren"
        self.assertEqual(binary.read_bytes(), self.binary.read_bytes())
        values = desktop_values(output / "share/applications/wren.desktop")
        self.assertEqual(values["Type"], "Application")
        self.assertEqual(values["Version"], "1.0")
        self.assertEqual(values["Name"], "Wren")
        self.assertEqual(values["GenericName"], "Structured Notes")
        self.assertEqual(values["Terminal"], "false")
        self.assertEqual(values["StartupNotify"], "false")
        self.assertEqual(values["Categories"], "Office;TextEditor;")
        self.assertEqual(desktop_exec_argument(values["Exec"]), str(binary))
        icon = output / "share/icons/hicolor/scalable/apps/wren.svg"
        self.assertEqual(desktop_unescape(values["Icon"]), str(icon))
        self.assertEqual(values["StartupWMClass"], "wren")
        self.assertEqual(icon.read_bytes(), (ROOT / package.ICON_SVG).read_bytes())
        self.assertEqual(ET.fromstring(icon.read_bytes()).attrib["viewBox"], "0 0 1024 1024")
        themed = sorted(path.relative_to(output / "share/icons/hicolor").as_posix()
                        for path in (output / "share/icons/hicolor").rglob("*") if path.is_file())
        self.assertEqual(themed, sorted(["scalable/apps/wren.svg",
                                         *(f"{size}x{size}/apps/wren.png" for size in package.ICON_SIZES)]))
        for size in package.ICON_SIZES:
            png = (output / f"share/icons/hicolor/{size}x{size}/apps/wren.png").read_bytes()
            self.assertEqual(png, (ROOT / package.hicolor_icon(size)).read_bytes())
        self.assert_notices(output / "share/doc/wren")
        self.assertEqual((output / "share/doc/wren/VERSION").read_text(), "0.0.1\n")
        self.assert_permissions(output, binary)
        self.assert_source_unchanged()

    def test_macos_layout_plist_icon_permissions_and_notices(self):
        output = self.assemble("macos")
        binary = output / "Contents/MacOS/wren"
        self.assertEqual(binary.read_bytes(), self.binary.read_bytes())
        info = plistlib.loads((output / "Contents/Info.plist").read_bytes())
        expected = {
            "CFBundleExecutable": "wren", "CFBundleName": "Wren", "CFBundleDisplayName": "Wren",
            "CFBundleIdentifier": "ai.stridelabs.Wren", "CFBundlePackageType": "APPL",
            "CFBundleVersion": "0.0.1", "CFBundleShortVersionString": "0.0.1",
            "CFBundleIconFile": "AppIcon.icns", "CFBundleInfoDictionaryVersion": "6.0",
            "NSHighResolutionCapable": True, "NSPrincipalClass": "NSApplication",
        }
        self.assertEqual({key: info[key] for key in expected}, expected)
        self.assertRegex(info["CFBundleIdentifier"], r"^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$")
        self.assertIn("Finder", info["NSAppleEventsUsageDescription"])
        for forbidden in ("SUFeedURL", "SUPublicEDKey", "NSCameraUsageDescription", "LSMinimumSystemVersion"):
            self.assertNotIn(forbidden, info)
        self.assertEqual((output / "Contents/PkgInfo").read_bytes(), b"APPL????")
        resources = output / "Contents/Resources"
        data = (resources / info["CFBundleIconFile"]).read_bytes()
        self.assertEqual(data, (ROOT / package.ICON_ICNS).read_bytes())
        self.assertEqual({kind: png_header(png)[:2] for kind, png in icns_images(data).items()},
                         {kind: (size, size) for kind, size in ICNS_SIZES.items()})
        # A fixture (non-native) bundle never runs actool: the flat ICNS alone.
        self.assertNotIn("CFBundleIconName", info)
        self.assertEqual(sorted(path.name for path in resources.iterdir()),
                         sorted(["AppIcon.icns", "DEPENDENCY_NOTICES.txt", "THIRD_PARTY_NOTICES.txt", "third_party"]))
        self.assert_notices(resources)
        self.assert_permissions(output, binary)
        self.assert_source_unchanged()

    def test_system_layout_keeps_installable_desktop_names_for_a_usr_install(self):
        self.set_cargo_version("0.0.1-rc.1")
        output = self.assemble(output=self.base / "deb root=with equals" / "usr", system_layout=True,
                               version="0.0.1-rc.1")
        binary = output / "bin/wren"
        self.assertEqual(binary.read_bytes(), self.binary.read_bytes())
        # Installed under /usr, PATH and the icon theme resolve the asset's names.
        self.assertEqual((output / "share/applications/wren.desktop").read_bytes(),
                         (ROOT / "assets/wren.desktop").read_bytes())
        themed = sorted(path.relative_to(output / "share/icons/hicolor").as_posix()
                        for path in (output / "share/icons/hicolor").rglob("*") if path.is_file())
        self.assertEqual(themed, sorted(["scalable/apps/wren.svg",
                                         *(f"{size}x{size}/apps/wren.png" for size in package.ICON_SIZES)]))
        self.assert_notices(output / "share/doc/wren")
        self.assertEqual((output / "share/doc/wren/VERSION").read_text(), "0.0.1-rc.1\n")
        self.assert_permissions(output, binary)
        self.assert_source_unchanged()

    def test_system_layout_is_linux_only(self):
        with self.assertRaisesRegex(package.PackageError, "Linux-only"):
            self.assemble("macos", system_layout=True)
        self.assertFalse((self.base / "bundle with spaces").exists())
        stderr = io.StringIO()
        with patch.object(package, "native_platform", return_value="macos"), redirect_stderr(stderr):
            self.assertEqual(package.main(["--binary", str(self.binary), "--system-layout",
                                           "--output", str(self.base / "not-created")]), 1)
        self.assertIn("Linux-only", stderr.getvalue())
        self.assertFalse((self.base / "not-created").exists())

    def set_cargo_version(self, version):
        (self.source / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{version}"\n')
        self.original = snapshot(self.source)

    def test_release_version_must_be_exactly_the_cargo_version(self):
        self.assertEqual(package.release_version("1.2.3", None), "1.2.3")
        self.assertEqual(package.release_version("1.2.3-rc1", "1.2.3-rc1"), "1.2.3-rc1")
        for rejected in ("v1.2.3", "1.2.4", "1.2.3-rc1", "1.2.3 ", ""):
            with self.subTest(rejected=rejected):
                with self.assertRaisesRegex(package.PackageError, "does not match the Cargo version"):
                    package.release_version("1.2.3", rejected)
        with self.assertRaisesRegex(package.PackageError, "does not match the Cargo version"):
            self.assemble(version="0.0.2")
        self.assertFalse((self.base / "bundle with spaces").exists())

    def test_prerelease_records_its_full_version_but_a_numeric_bundle_version(self):
        self.set_cargo_version("0.0.1-rc1")
        output = self.assemble("macos", version="0.0.1-rc1")
        info = plistlib.loads((output / "Contents/Info.plist").read_bytes())
        self.assertEqual((info["CFBundleShortVersionString"], info["CFBundleVersion"]), ("0.0.1", "0.0.1"))
        linux = self.assemble(output=self.base / "linux-rc")
        self.assertEqual((linux / "share/doc/wren/VERSION").read_text(), "0.0.1-rc1\n")

    def test_cli_system_layout_and_release_version(self):
        self.binary.write_bytes(native_fixture("linux"))
        self.set_cargo_version("0.0.1-rc1")
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(package, "ROOT", self.source), patch.object(package, "native_platform", return_value="linux"), \
                redirect_stdout(stdout), redirect_stderr(stderr):
            self.assertEqual(package.main(["--binary", str(self.binary), "--system-layout",
                                           "--release-version", "0.0.1-rc1"]), 0, stderr.getvalue())
            output = self.source / "target/package/wren-0.0.1-rc1-linux-system"
            self.assertEqual((output / "share/doc/wren/VERSION").read_text(), "0.0.1-rc1\n")
            self.assertEqual(package.main(["--binary", str(self.binary), "--release-version", "9.9.9",
                                           "--output", str(self.base / "mismatch")]), 1)
        self.assertIn("system-layout linux tree", stdout.getvalue())
        self.assertIn("does not match the Cargo version", stderr.getvalue())
        self.assertFalse((self.base / "mismatch").exists())

    def test_desktop_paths_escape_spaces_quotes_backslashes_and_field_codes(self):
        output = self.base / 'bundle "quoted" $HOME `literal` 50% \\ path' / "wren-linux"
        output = self.assemble(output=output)
        values = desktop_values(output / "share/applications/wren.desktop")
        self.assertEqual(desktop_exec_argument(values["Exec"]), str(output / "bin/wren"))
        self.assertEqual(desktop_unescape(values["Icon"]), str(output / "share/icons/hicolor/scalable/apps/wren.svg"))
        self.assertNotIn("%F", values["Exec"])
        self.assert_source_unchanged()

    def test_linux_output_with_equals_is_rejected_before_staging_or_publication(self):
        output = self.base / "unsupported=desktop path" / "wren-linux"
        with patch.object(package, "publish_bundle") as publish:
            with self.assertRaisesRegex(package.PackageError, "="):
                self.assemble(output=output)
        publish.assert_not_called()
        self.assertFalse(output.parent.exists())
        self.assert_source_unchanged()

    def test_asset_desktop_entry_uses_standard_installable_names(self):
        values = desktop_values(ROOT / "assets/wren.desktop")
        self.assertEqual(values["Exec"], "wren")
        self.assertEqual(values["Icon"], "wren")
        self.assertEqual(values["StartupWMClass"], "wren")
        self.assertNotIn("Path", values)
        self.assertNotIn("URL", values)

    def test_linux_window_application_id_matches_the_desktop_entry_basename(self):
        # Wayland pairs a window with its launcher entry and icon only by app_id.
        main = (ROOT / "crates/wren-iced/src/main.rs").read_text(encoding="utf-8")
        app_id = re.findall(r'^const APP_ID: &str = "([^"]+)";$', main, re.MULTILINE)
        self.assertEqual(app_id, ["wren"])
        self.assertIn("application_id: APP_ID.to_owned()", main)
        self.assertEqual(desktop_values(ROOT / "assets/wren.desktop")["StartupWMClass"], app_id[0])
        self.assertIn('Path("share/applications/wren.desktop")', (ROOT / "tools/package.py").read_text())

    def test_committed_icons_are_current_renders_of_the_source_svg(self):
        roles = ("background", "owl-body", "eyes", "eye-glints")
        svg = ET.fromstring((ROOT / package.ICON_SVG).read_bytes())
        fills = {element.attrib["id"]: element.attrib["fill"].lower()
                 for element in svg.iter() if element.attrib.get("id") in roles}
        self.assertEqual(set(fills), set(roles))
        for fill in fills.values():
            self.assertRegex(fill, r"^#[0-9a-f]{6}$")
        background = tuple(bytes.fromhex(fills["background"][1:]))
        hicolor = ROOT / "assets/icon/hicolor"
        self.assertEqual(sorted(path.relative_to(ROOT).as_posix() for path in hicolor.rglob("*") if path.is_file()),
                         sorted(package.hicolor_icon(size).as_posix() for size in package.ICON_SIZES))
        for size in package.ICON_SIZES:
            self.assertEqual(png_header((ROOT / package.hicolor_icon(size)).read_bytes()), (size, size, 8, 6))
        width, rgba = decode_rgba((ROOT / package.hicolor_icon(256)).read_bytes())

        def pixel(x, y):
            return tuple(rgba[(y * width + x) * 4:(y * width + x) * 4 + 4])

        self.assertEqual(pixel(0, 0)[3], 0)
        # Top-center of the full-bleed tile is background; the owl sits lower.
        self.assertEqual(pixel(128, 8), (*background, 255), "edited the SVG? run make icons")
        width, rgba = decode_rgba(icns_images((ROOT / package.ICON_ICNS).read_bytes())[b"ic08"])
        self.assertEqual(pixel(128, 8)[3], 0, "the macOS tile is inset on Apple's icon grid")
        self.assertEqual(pixel(128, 40), (*background, 255), "edited the SVG? run make icons")
        catalog = json.loads((ROOT / package.ICON_CATALOG / "icon.json").read_text())
        channels = catalog["fill"]["solid"].removeprefix("srgb:").split(",")
        self.assertEqual(tuple(round(float(value) * 255) for value in channels[:3]), background)
        layers = [layer["image-name"] for group in catalog["groups"] for layer in group["layers"]]
        self.assertEqual(layers, ["owl.png"])
        owl = (ROOT / package.ICON_CATALOG / "Assets/owl.png").read_bytes()
        self.assertEqual(png_header(owl), (1024, 1024, 8, 6))
        width, rgba = decode_rgba(owl)
        self.assertEqual(pixel(512, 40)[3], 0, "the Icon Composer owl layer has no baked tile")

    def test_cargo_version_is_inherited_or_taken_from_app_manifest(self):
        self.assertEqual(package.cargo_version(self.source), "0.0.1")
        app = self.source / "crates/wren-iced/Cargo.toml"
        app.write_text('[package]\nversion = "2.3.4"\n')
        output = self.assemble("macos")
        self.assertEqual(plistlib.loads((output / "Contents/Info.plist").read_bytes())["CFBundleVersion"], "2.3.4")

    def test_invalid_versions_are_rejected(self):
        for version in ("../escape", "1.2.3-", "1.2.3-beta..1", "1.2.3+build", "01.2.3", "1.2", "", True):
            with self.subTest(version=version):
                value = 'true' if version is True else '"' + version + '"'
                (self.source / "crates/wren-iced/Cargo.toml").write_text(f"[package]\nversion = {value}\n")
                with self.assertRaisesRegex(package.PackageError, "numeric X.Y.Z release"):
                    self.assemble()
                self.assertFalse((self.base / "bundle with spaces").exists())

    def test_symlink_executable_and_symlink_ancestors_are_rejected(self):
        for name, link_target, suffix in (("binary-link", self.binary, ""),
                                          ("directory-link", self.binary.parent, "wren")):
            with self.subTest(name=name):
                link = self.base / name
                link.symlink_to(link_target)
                with self.assertRaisesRegex(package.PackageError, "symlinks"):
                    package.assemble_bundle(link / suffix, self.base / (name + "-output"), "linux",
                                            dependency_notices=self.dependency_text, root=self.source)
                self.assertFalse((self.base / (name + "-output")).exists())
        self.assert_source_unchanged()

    def test_source_ancestor_replaced_after_checked_path_cannot_redirect_reads(self):
        real_checked = package.checked_path
        for index, source in enumerate((self.binary, self.source / package.FONT_NOTICES[0])):
            with self.subTest(source=source):
                foreign = self.base / f"foreign-source-{index}"
                foreign.mkdir()
                (foreign / source.name).write_bytes(b"FOREIGN CONTENT MUST NOT BE READ")
                moved = self.base / f"moved-source-{index}"
                replaced = False

                def replace_after_check(path, **kwargs):
                    nonlocal replaced
                    checked = real_checked(path, **kwargs)
                    if checked == source and not replaced:
                        source.parent.rename(moved)
                        source.parent.symlink_to(foreign, target_is_directory=True)
                        replaced = True
                    return checked

                try:
                    with patch.object(package, "checked_path", side_effect=replace_after_check):
                        with self.assertRaises((OSError, package.PackageError)):
                            package.read_source(source)
                    self.assertTrue(replaced)
                    self.assertEqual((foreign / source.name).read_bytes(), b"FOREIGN CONTENT MUST NOT BE READ")
                finally:
                    if replaced:
                        source.parent.unlink()
                        moved.rename(source.parent)
        self.assert_source_unchanged()

    def test_source_parent_renamed_after_pinning_reads_original_not_replacement(self):
        real_open = os.open
        for index, source in enumerate((self.binary, self.source / package.FONT_NOTICES[0])):
            with self.subTest(source=source):
                expected = source.read_bytes()
                foreign = self.base / f"foreign-pinned-source-{index}"
                foreign.mkdir()
                (foreign / source.name).write_bytes(b"FOREIGN CONTENT MUST NOT BE READ")
                moved = self.base / f"moved-pinned-source-{index}"
                replaced = False

                def replace_before_leaf(path, flags, *args, **kwargs):
                    nonlocal replaced
                    if path == source.name and kwargs.get("dir_fd") is not None and not replaced:
                        source.parent.rename(moved)
                        source.parent.symlink_to(foreign, target_is_directory=True)
                        replaced = True
                    return real_open(path, flags, *args, **kwargs)

                try:
                    with patch.object(package.os, "open", side_effect=replace_before_leaf):
                        self.assertEqual(package.read_source(source), expected)
                    self.assertTrue(replaced, "source leaf must be opened relative to its pinned parent")
                finally:
                    if replaced:
                        source.parent.unlink()
                        moved.rename(source.parent)
        self.assert_source_unchanged()

    def test_symlink_notice_is_not_followed_and_stage_is_cleaned(self):
        notice = self.source / package.FONT_NOTICES[0]
        notice.unlink()
        notice.symlink_to(ROOT / package.FONT_NOTICES[0])
        parent = self.base / "bundle with spaces"
        with self.assertRaisesRegex(package.PackageError, "symlinks"):
            self.assemble()
        self.assertEqual(list(parent.iterdir()), [])

    def test_symlink_output_or_output_ancestor_is_rejected(self):
        external = self.base / "external"
        external.mkdir()
        sentinel = external / "unchanged"
        sentinel.write_bytes(b"must stay")
        link = self.base / "link"
        link.symlink_to(external)
        for output in (link, link / "bundle"):
            with self.subTest(output=output):
                with self.assertRaisesRegex(package.PackageError, "symlinks"):
                    self.assemble(output=output)
        self.assertEqual(sentinel.read_bytes(), b"must stay")
        self.assertEqual(list(external.iterdir()), [sentinel])
        self.assert_source_unchanged()

    def test_broken_symlink_output_is_rejected(self):
        output = self.base / "broken"
        output.symlink_to(self.base / "missing")
        with self.assertRaisesRegex(package.PackageError, "symlinks"):
            self.assemble(output=output)
        self.assertTrue(output.is_symlink())
        self.assertFalse((self.base / "missing").exists())

    def test_existing_file_empty_directory_and_existing_bundle_are_preserved(self):
        for kind in ("file", "empty-directory", "bundle"):
            with self.subTest(kind=kind):
                output = self.base / kind
                if kind == "file":
                    output.write_bytes(b"existing artifact")
                elif kind == "empty-directory":
                    output.mkdir()
                else:
                    self.assemble(output=output)
                before = output.read_bytes() if output.is_file() else snapshot(output)
                with self.assertRaisesRegex(package.PackageError, "already exists"):
                    self.assemble(output=output)
                self.assertEqual(output.read_bytes() if output.is_file() else snapshot(output), before)
        self.assert_source_unchanged()

    def test_output_collision_at_publication_cannot_replace_even_an_empty_directory(self):
        real_publish = package.publish_bundle
        for contents in (False, True):
            with self.subTest(contents=contents):
                output = self.base / ("nonempty" if contents else "empty")
                def collide(stage, destination):
                    destination.mkdir()
                    if contents:
                        (destination / "unrelated").write_bytes(b"preserve me")
                    real_publish(stage, destination)
                with patch.object(package, "publish_bundle", side_effect=collide):
                    with self.assertRaises(OSError):
                        self.assemble(output=output)
                self.assertTrue(output.is_dir())
                if contents:
                    self.assertEqual((output / "unrelated").read_bytes(), b"preserve me")
                else:
                    self.assertEqual(list(output.iterdir()), [])
                self.assertEqual(list(self.base.glob(f".{output.name}.staging-*")), [])
        self.assert_source_unchanged()

    def replace_staging_alias(self, output, moved):
        stages = list(output.parent.glob(f".{output.name}.staging-*"))
        self.assertEqual(len(stages), 1)
        alias = stages[0]
        alias.rename(moved)
        alias.mkdir()
        (alias / "foreign directory").mkdir()
        (alias / "foreign directory/sentinel").write_bytes(b"foreign artifact must stay")
        return alias, snapshot(alias)

    def test_stage_alias_replacement_on_failed_publication_preserves_foreign_directory(self):
        output = self.base / "destination/wren-linux"
        moved = self.base / "moved owned stage"
        captured = {}

        def replace_and_fail(stage, destination):
            captured["alias"], captured["before"] = self.replace_staging_alias(destination, moved)
            raise OSError("injected publication failure after stage replacement")

        with patch.object(package, "publish_bundle", side_effect=replace_and_fail):
            with self.assertRaises((OSError, package.PackageError)) as error:
                self.assemble(output=output)
        alias = captured["alias"]
        self.assertTrue(alias.is_dir(), "cleanup must not remove a foreign stage replacement")
        self.assertEqual(snapshot(alias), captured["before"])
        self.assertEqual(list(moved.iterdir()), [])
        self.assertEqual(stat.S_IMODE(moved.stat().st_mode), 0o700)
        self.assertIn("retained", str(error.exception))
        self.assertFalse(output.exists())
        self.assert_source_unchanged()

    def test_stage_alias_replacement_cannot_be_published_as_the_bundle(self):
        output = self.base / "destination/wren-linux"
        moved = self.base / "moved owned stage"
        real_publish = package.publish_bundle
        captured = {}

        def replace_before_publish(stage, destination):
            captured["alias"], captured["before"] = self.replace_staging_alias(destination, moved)
            real_publish(stage, destination)

        with patch.object(package, "publish_bundle", side_effect=replace_before_publish):
            with self.assertRaises(package.PackageError):
                self.assemble(output=output)
        self.assertFalse(output.exists())
        self.assertEqual(snapshot(captured["alias"]), captured["before"])
        self.assertEqual(list(moved.iterdir()), [])
        self.assertEqual(stat.S_IMODE(moved.stat().st_mode), 0o700)
        self.assert_source_unchanged()

    def test_stage_writes_after_alias_replacement_never_touch_foreign_directory(self):
        output = self.base / "destination/wren-linux"
        moved = self.base / "moved owned stage"
        real_write = package.write_file
        captured = {}

        def replace_before_write(path, contents, **kwargs):
            captured["alias"], captured["before"] = self.replace_staging_alias(output, moved)
            real_write(path, contents, **kwargs)
            raise OSError("injected write failure after stage replacement")

        with patch.object(package, "write_file", side_effect=replace_before_write):
            with self.assertRaises((OSError, package.PackageError)):
                self.assemble(output=output)
        self.assertTrue(captured["alias"].is_dir())
        self.assertEqual(snapshot(captured["alias"]), captured["before"])
        self.assertEqual(list(moved.iterdir()), [])
        self.assertFalse(output.exists())
        self.assert_source_unchanged()

    def test_parent_replacement_cannot_redirect_publication_or_cleanup(self):
        parent = self.base / "destination"
        parent.mkdir()
        (parent / "original sentinel").write_bytes(b"keep original parent")
        output = parent / "wren-linux"
        moved = self.base / "moved parent"
        real_publish = package.publish_bundle
        captured = {}

        def replace_parent(stage, destination):
            alias = next(parent.glob(f".{output.name}.staging-*"))
            parent.rename(moved)
            parent.mkdir()
            foreign_stage = parent / alias.name
            foreign_stage.mkdir()
            (foreign_stage / "foreign sentinel").write_bytes(b"do not publish or delete")
            captured["before"] = snapshot(parent)
            real_publish(stage, destination)

        with patch.object(package, "publish_bundle", side_effect=replace_parent):
            with self.assertRaises(package.PackageError):
                self.assemble(output=output)
        self.assertEqual(snapshot(parent), captured["before"])
        self.assertFalse(output.exists())
        self.assertFalse((moved / output.name).exists())
        self.assertEqual(list(moved.iterdir()), [moved / "original sentinel"])
        self.assertEqual((moved / "original sentinel").read_bytes(), b"keep original parent")
        self.assert_source_unchanged()

    def test_failure_after_copy_cleans_only_owned_staging_and_keeps_sources(self):
        parent = self.base / "destination"
        parent.mkdir()
        unrelated = parent / ".Wren.app.staging-not-owned"
        unrelated.mkdir()
        (unrelated / "artifact").write_bytes(b"keep")
        output = parent / "Wren.app"
        real_write = package.write_file
        def fail_after_binary(path, data, **kwargs):
            binary = os.stat(path.parent / "MacOS/wren", dir_fd=kwargs["dir_fd"], follow_symlinks=False)
            self.assertTrue(stat.S_ISREG(binary.st_mode))
            raise OSError("injected write failure")
        with patch.object(package, "write_file", side_effect=fail_after_binary):
            with self.assertRaisesRegex(OSError, "injected write failure"):
                self.assemble("macos", output=output)
        self.assertEqual(list(parent.iterdir()), [unrelated])
        self.assertEqual((unrelated / "artifact").read_bytes(), b"keep")
        self.assert_source_unchanged()
        # A later attempt can succeed without deleting any arbitrary directory.
        self.assertIs(package.write_file, real_write)
        self.assemble("macos", output=output)
        self.assertTrue((unrelated / "artifact").is_file())

    def test_missing_notice_cleans_partial_bundle_and_preserves_binary(self):
        (self.source / package.FONT_NOTICES[1]).unlink()
        before = snapshot(self.source)
        with self.assertRaises(FileNotFoundError):
            self.assemble()
        self.assertEqual(list((self.base / "bundle with spaces").iterdir()), [])
        self.assertEqual(snapshot(self.source), before)

    def test_input_is_not_executed_and_unsafe_permission_bits_are_not_propagated(self):
        self.binary.chmod(0o6755)
        before = snapshot(self.source)
        output = self.assemble()
        self.assertEqual(stat.S_IMODE((output / "bin/wren").stat().st_mode), 0o755)
        self.assertEqual(snapshot(self.source), before)

    def test_non_executable_directory_and_fifo_inputs_are_rejected_without_hanging(self):
        regular = self.base / "not-executable"
        regular.write_bytes(b"data")
        regular.chmod(0o644)
        fifo = self.base / "fifo"
        os.mkfifo(fifo, 0o700)
        for binary in (regular, self.binary.parent, fifo):
            with self.subTest(binary=binary):
                with self.assertRaisesRegex(package.PackageError, "executable permissions|regular file"):
                    package.assemble_bundle(binary, self.base / "rejected", "linux",
                                            dependency_notices=self.dependency_text, root=self.source)
                self.assertFalse((self.base / "rejected").exists())
                self.assertEqual(list(self.base.glob(".rejected.staging-*")), [])

    def test_missing_input_and_parent_traversal_are_rejected(self):
        with self.assertRaises(FileNotFoundError):
            package.assemble_bundle(self.base / "missing", self.base / "output", "linux",
                                    dependency_notices=self.dependency_text, root=self.source)
        with self.assertRaisesRegex(package.PackageError, "parent traversal"):
            self.assemble(output=self.base / "foo/../output")
        self.assertFalse((self.base / "foo").exists())
        self.assert_source_unchanged()

    def test_macos_name_and_unsupported_platform_are_rejected_before_staging(self):
        for platform, name in (("macos", "Other.app"), ("windows", "wren")):
            with self.subTest(platform=platform):
                with self.assertRaises(package.PackageError):
                    self.assemble(platform, output=self.base / "new-parent" / name)
                self.assertFalse((self.base / "new-parent").exists())

    def test_asset_corruption_fails_closed_with_cleanup(self):
        wrong_size = (ROOT / package.hicolor_icon(32)).read_bytes()
        for asset, bad in ((package.ICON_SVG, b"not XML"),
                           (package.ICON_SVG, b'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"/>'),
                           (package.hicolor_icon(48), wrong_size),
                           (package.hicolor_icon(64), b"not a PNG"),
                           (package.ICON_ICNS, b"icns\x00\x00\x00\x10truncated"),
                           (Path("assets/wren.desktop"), b"[Desktop Entry]\nExec=wrong\nIcon=wren\n")):
            with self.subTest(asset=asset, bad=bad[:24]):
                path = self.source / asset
                previous = path.read_bytes()
                path.write_bytes(bad)
                with self.assertRaises((package.PackageError, ET.ParseError)):
                    self.assemble()
                self.assertEqual(list((self.base / "bundle with spaces").iterdir()), [])
                path.write_bytes(previous)

    def test_native_macos_bundle_adds_the_compiled_icon_catalog_or_falls_back(self):
        self.binary.write_bytes(native_fixture("macos"))
        for compiled in (b"fixture Assets.car", None):
            with self.subTest(compiled=compiled):
                output = self.base / f"native-{compiled is not None}" / "Wren.app"
                with patch.object(package, "compile_icon_catalog", return_value=compiled) as compile_icon:
                    self.assemble("macos", output=output, native=True)
                compile_icon.assert_called_once_with(self.source / package.ICON_CATALOG)
                info = plistlib.loads((output / "Contents/Info.plist").read_bytes())
                resources = output / "Contents/Resources"
                self.assertEqual(info["CFBundleIconFile"], "AppIcon.icns")
                self.assertTrue((resources / "AppIcon.icns").is_file())
                if compiled is None:
                    self.assertNotIn("CFBundleIconName", info)
                    self.assertFalse((resources / "Assets.car").exists())
                else:
                    self.assertEqual(info["CFBundleIconName"], "AppIcon")
                    self.assertEqual((resources / "Assets.car").read_bytes(), compiled)
                self.assert_permissions(output, output / "Contents/MacOS/wren")

    def test_icon_catalog_compilation_is_skipped_without_xcrun(self):
        with patch.object(package.shutil, "which", return_value=None), \
                patch.object(package.subprocess, "run") as run:
            self.assertIsNone(package.compile_icon_catalog(ROOT / package.ICON_CATALOG))
        run.assert_not_called()

    def test_icon_catalog_compilation_failure_falls_back_with_a_warning(self):
        failed = subprocess.CompletedProcess([], 1, b"", b"unsupported input")
        with patch.object(package.shutil, "which", return_value="/usr/bin/xcrun"), \
                patch.object(package.subprocess, "run", return_value=failed) as run, \
                redirect_stderr(io.StringIO()) as stderr:
            self.assertIsNone(package.compile_icon_catalog(ROOT / package.ICON_CATALOG))
        command = run.call_args.args[0]
        self.assertEqual(command[:3], ["xcrun", "actool", str(ROOT / package.ICON_CATALOG)])
        self.assertIn("--app-icon", command)
        self.assertIn("flat AppIcon.icns", stderr.getvalue())

    def test_staging_container_and_bundle_remain_private_until_publication(self):
        real_publish = package.publish_bundle
        seen = []

        def check_private(stage, destination):
            self.assertEqual(stat.S_IMODE(os.fstat(stage.container_fd).st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(os.fstat(stage.fd).st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((destination.parent / stage.name).stat().st_mode), 0o700)
            self.assertFalse(stage.published)
            seen.append(stage.name)
            real_publish(stage, destination)
            self.assertTrue(stage.published)

        with patch.object(package, "publish_bundle", side_effect=check_private):
            output = self.assemble()
        self.assertEqual(len(seen), 1)
        self.assertFalse((output.parent / seen[0]).exists())
        self.assert_permissions(output, output / "bin/wren")
        self.assert_source_unchanged()

    def test_native_rename_races_use_pinned_source_and_destination_directories(self):
        libc = package.ctypes.CDLL(None, use_errno=True)
        symbol = "renameat2" if sys.platform == "linux" else "renameatx_np"
        real_rename = getattr(libc, symbol)
        real_rename.argtypes = (package.ctypes.c_int, package.ctypes.c_char_p, package.ctypes.c_int,
                               package.ctypes.c_char_p, package.ctypes.c_uint)
        real_rename.restype = package.ctypes.c_int
        for replacement in ("parent", "stage"):
            with self.subTest(replacement=replacement):
                parent = self.base / f"native-{replacement}-race"
                output = parent / "wren-linux"
                moved = self.base / f"native-{replacement}-moved"
                captured = {}

                def rename_after_replacement(source_fd, source_name, target_fd, target_name, flags):
                    self.assertGreaterEqual(source_fd, 0)
                    self.assertGreaterEqual(target_fd, 0)
                    self.assertNotEqual(source_fd, target_fd)
                    self.assertEqual(source_name, b"bundle")
                    self.assertEqual(target_name, os.fsencode(output.name))
                    self.assertEqual(flags, 1 if sys.platform == "linux" else 4)
                    if replacement == "parent":
                        parent.rename(moved)
                        parent.mkdir()
                        (parent / "foreign sentinel").write_bytes(b"preserve replacement parent")
                        captured["foreign"] = parent
                    else:
                        captured["foreign"], _ = self.replace_staging_alias(output, moved)
                    captured["before"] = snapshot(captured["foreign"])
                    return real_rename(source_fd, source_name, target_fd, target_name, flags)

                class NativeLibrary:
                    pass

                wrapped_libc = NativeLibrary()
                setattr(wrapped_libc, symbol, rename_after_replacement)
                with patch.object(package.ctypes, "CDLL", return_value=wrapped_libc):
                    with self.assertRaises(package.PackageError):
                        self.assemble(output=output)
                published = moved / output.name if replacement == "parent" else output
                self.assertEqual((published / "bin/wren").read_bytes(), self.binary.read_bytes())
                self.assert_permissions(published, published / "bin/wren")
                self.assertEqual(snapshot(captured["foreign"]), captured["before"])
                if replacement == "parent":
                    self.assertEqual(list(moved.iterdir()), [published])
                    self.assertFalse(output.exists())
                else:
                    self.assertEqual(list(moved.iterdir()), [])
                    self.assertEqual(stat.S_IMODE(moved.stat().st_mode), 0o700)
        self.assert_source_unchanged()

    def test_missing_exclusive_rename_api_fails_closed_without_an_ordinary_rename(self):
        with patch.object(package.ctypes, "CDLL", return_value=object()):
            with patch.object(package.os, "rename") as rename, patch.object(package.os, "replace") as replace:
                with self.assertRaisesRegex(package.PackageError, "no-replace directory rename"):
                    self.assemble()
        rename.assert_not_called()
        replace.assert_not_called()
        self.assertEqual(list((self.base / "bundle with spaces").iterdir()), [])
        self.assert_source_unchanged()

    def test_copy_detects_source_changes_and_cleans_owned_stage(self):
        binary = self.base / "mutable executable"
        binary.write_bytes(self.binary.read_bytes())
        binary.chmod(0o700)
        output = self.base / "changed-input-output"
        real_copy = package.shutil.copyfileobj

        def change_after_copy(source, target):
            real_copy(source, target)
            with binary.open("ab") as changed:
                changed.write(b"changed during copy")

        with patch.object(package.shutil, "copyfileobj", side_effect=change_after_copy):
            with self.assertRaisesRegex(package.PackageError, "changed while it was being copied"):
                package.assemble_bundle(binary, output, "linux", root=self.source,
                                        dependency_notices=self.dependency_text)
        self.assertFalse(output.exists())
        self.assertEqual(list(self.base.glob(f".{output.name}.staging-*")), [])
        self.assertEqual(binary.read_bytes(), self.binary.read_bytes() + b"changed during copy")
        self.assert_source_unchanged()

    def test_permissions_are_reasonable_even_under_restrictive_umask(self):
        previous = os.umask(0o077)
        try:
            output = self.assemble()
        finally:
            os.umask(previous)
        self.assert_permissions(output, output / "bin/wren")

    def test_publication_failure_preserves_source_and_removes_owned_stage(self):
        with patch.object(package, "publish_bundle", side_effect=OSError("injected publish failure")):
            with self.assertRaisesRegex(OSError, "injected publish failure"):
                self.assemble()
        self.assertEqual(list((self.base / "bundle with spaces").iterdir()), [])
        self.assert_source_unchanged()

    def test_native_format_rejection_cleans_staging(self):
        for platform in ("linux", "macos"):
            with self.subTest(platform=platform):
                self.binary.write_bytes(native_fixture("macos" if platform == "linux" else "linux"))
                before = snapshot(self.source)
                with self.assertRaisesRegex(package.PackageError, "input format does not match"):
                    self.assemble(platform, native=True)
                self.assertEqual(list((self.base / "bundle with spaces").iterdir()), [])
                self.assertEqual(snapshot(self.source), before)

    def test_native_format_gate_accepts_exec_headers_but_rejects_scripts_and_macho_libraries(self):
        for platform in ("linux", "macos"):
            with self.subTest(platform=platform):
                package.check_native_format(native_fixture(platform), platform)
                for bad in (b"", b"#!/bin/sh\nexit 0\n", bytes(32)):
                    with self.assertRaises(package.PackageError):
                        package.check_native_format(bad, platform)
        macho_library = bytearray(native_fixture("macos"))
        macho_library[12:16] = (6).to_bytes(4, "little")
        with self.assertRaises(package.PackageError):
            package.check_native_format(bytes(macho_library), "macos")

    def test_cli_rejects_cross_platform_even_before_reading_input(self):
        for host, requested in (("linux", "macos"), ("macos", "linux")):
            with self.subTest(host=host):
                stderr = io.StringIO()
                with patch.object(package, "native_platform", return_value=host), redirect_stderr(stderr):
                    code = package.main(["--binary", str(self.base / "missing"), "--platform", requested,
                                         "--output", str(self.base / "not-created")])
                self.assertEqual(code, 1)
                self.assertIn("use a native build host", stderr.getvalue())
                self.assertFalse((self.base / "not-created").exists())

    def test_default_cli_output_is_under_target_and_existing_output_is_preserved(self):
        host = package.native_platform()
        self.binary.write_bytes(native_fixture(host))
        before = snapshot(self.source)
        stderr = io.StringIO()
        stdout = io.StringIO()
        with patch.object(package, "ROOT", self.source), redirect_stderr(stderr), redirect_stdout(stdout):
            self.assertEqual(package.main(["--binary", str(self.binary)]), 0, stderr.getvalue())
            output = self.source / f"target/package/wren-0.0.1-{host}"
            if host == "macos":
                output /= "Wren.app"
            self.assertTrue(output.is_dir())
            artifact = snapshot(output)
            self.assertEqual(package.main(["--binary", str(self.binary)]), 1)
            self.assertEqual(snapshot(output), artifact)
        self.assertIn("Assembled uninstalled", stdout.getvalue())
        self.assertIn("already exists", stderr.getvalue())
        self.assertEqual({name: state for name, state in snapshot(self.source).items()
                          if not name.startswith("target/")}, before)
        self.assertIn("/target/", (ROOT / ".gitignore").read_text().splitlines())

    def test_cli_subprocess_requires_an_explicit_binary_and_assembles_without_executing_it(self):
        missing = subprocess.run([sys.executable, str(ROOT / "tools/package.py")],
                                 text=True, capture_output=True, check=False)
        self.assertEqual(missing.returncode, 2)
        self.assertIn("--binary", missing.stderr)
        host = package.native_platform()
        self.binary.write_bytes(native_fixture(host))
        output = self.base / ("Wren.app" if host == "macos" else "cli bundle")
        result = subprocess.run([sys.executable, str(ROOT / "tools/package.py"),
                                 "--binary", str(self.binary), "--output", str(output)],
                                 text=True, capture_output=True, check=False, env=self.fake_env)
        self.assertEqual(result.returncode, 0, result.stderr)
        packaged = output / ("Contents/MacOS/wren" if host == "macos" else "bin/wren")
        self.assertEqual(packaged.read_bytes(), native_fixture(host))
        self.assertIn("Assembled uninstalled", result.stdout)
        self.assertIn("1 packages, 1 local texts", result.stdout)
        self.assert_notices(output / ("Contents/Resources" if host == "macos" else "share/doc/wren"))

    def test_layout_assembly_never_collects_metadata_and_rejects_empty_or_invalid_notices(self):
        with patch.object(package, "collect_notices", side_effect=AssertionError("must not run Cargo")):
            self.assemble()
        for text in (b"", b" \n", b"\xff", "not bytes"):
            with self.subTest(text=text):
                output = self.base / "new-parent/rejected"
                with self.assertRaisesRegex(package.PackageError, "dependency notices"):
                    self.assemble(output=output, dependency_notices=text)
                self.assertFalse(output.parent.exists())
        self.assert_source_unchanged()

    def test_cli_collector_failure_does_not_publish_or_touch_existing_artifacts(self):
        parent = self.base / "existing parent"
        parent.mkdir()
        existing = parent / "old-bundle"
        existing.mkdir()
        (existing / "sentinel").write_bytes(b"keep existing")
        unrelated = parent / ".new-bundle.staging-not-owned"
        unrelated.mkdir()
        (unrelated / "sentinel").write_bytes(b"keep staging")
        before = snapshot(parent)
        stderr = io.StringIO()
        with patch.object(package, "ROOT", self.source), redirect_stderr(stderr):
            with patch.object(package, "collect_notices", side_effect=dependency_notices.NoticeError("metadata failed")):
                for output in (parent / "new-bundle", existing):
                    self.assertEqual(package.main(["--binary", str(self.binary), "--output", str(output)]), 1)
        self.assertEqual(snapshot(parent), before)
        self.assertEqual(set(parent.iterdir()), {existing, unrelated})
        self.assertIn("metadata failed", stderr.getvalue())
        self.assert_source_unchanged()

    def test_dependency_notice_write_failure_cleans_owned_stage_but_not_other_artifacts(self):
        parent = self.base / "notice-write-test"
        parent.mkdir()
        unrelated = parent / ".wren-linux.staging-unrelated"
        unrelated.mkdir()
        (unrelated / "keep").write_bytes(b"untouched")
        real_write = package.write_file

        def fail_notice(path, contents, **kwargs):
            if path.name == "DEPENDENCY_NOTICES.txt":
                raise OSError("injected dependency notice write failure")
            real_write(path, contents, **kwargs)

        with patch.object(package, "write_file", side_effect=fail_notice):
            with self.assertRaisesRegex(OSError, "dependency notice write failure"):
                self.assemble(output=parent / "wren-linux")
        self.assertEqual(list(parent.iterdir()), [unrelated])
        self.assertEqual((unrelated / "keep").read_bytes(), b"untouched")
        self.assert_source_unchanged()

    def test_cli_collects_current_root_notices_and_warns_about_coverage_gaps(self):
        report = dependency_notices.NoticeReport((dependency_notices.PackageNotice(
            "spdx-only", "2.0.0", "MIT", (), ()),), 1)
        self.binary.write_bytes(native_fixture(package.native_platform()))
        output = self.base / ("Wren.app" if package.native_platform() == "macos" else "cli-collected")
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(package, "ROOT", self.source), redirect_stdout(stdout), redirect_stderr(stderr):
            with patch.object(package, "collect_notices", return_value=report) as collector:
                self.assertEqual(package.main(["--binary", str(self.binary), "--output", str(output)]), 0)
        collector.assert_called_once_with(self.source)
        notices = output / ("Contents/Resources" if package.native_platform() == "macos" else "share/doc/wren")
        self.assertEqual((notices / "DEPENDENCY_NOTICES.txt").read_bytes(), report.render())
        self.assertIn("1 packages, 0 local texts", stdout.getvalue())
        self.assertIn("coverage requires review", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
