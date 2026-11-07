"""Assemble an uninstalled Wren bundle from an explicitly supplied executable.

The CLI requires the matching host and an ELF (Linux) or Mach-O (macOS) header.
assemble_bundle also supports cross-platform layout tests with fixture bytes;
it does not establish that a supplied binary can run. No build, installation,
signing, download, or application-configuration steps are performed, except
that a native macOS run compiles the committed Icon Composer icon with actool
when full Xcode provides it. Icons are committed outputs of `make icons`. The CLI
collects dependency texts using locked, offline Cargo metadata before assembly;
run cargo fetch --locked separately if the locked sources are not cached.
Existing outputs are never replaced; choose a new --output for another bundle.

--system-layout stages the same Linux tree for a system package that installs
it under /usr (the release .deb): the desktop entry then keeps Exec=wren and
Icon=wren, which PATH and the hicolor icon theme resolve once installed.
--release-version names the release tag's version (X.Y.Z or X.Y.Z-pre, no v)
and fails unless it is exactly the Cargo version, so an artifact can never be
built for a tag the source tree does not carry.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import ctypes
from dataclasses import dataclass
import os
from pathlib import Path
import plistlib
import re
import secrets
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import tomllib
import xml.etree.ElementTree as ET

if __package__:
    from .dependency_notices import collect_notices, directory_fd
else:
    from dependency_notices import collect_notices, directory_fd

DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_NONBLOCK

ROOT = Path(__file__).resolve().parents[1]
# Matches the sibling app's `ai.stridelabs.Roost`. Changing it orphans every
# macOS preference, TCC grant, and notarization record keyed on the old id.
BUNDLE_ID = "ai.stridelabs.Wren"
# Deleting a note moves it to the Trash through Finder (the trash crate's
# default macOS method runs osascript), which macOS gates behind an Automation
# prompt that shows this text. packaging/macos/Wren.entitlements grants the
# matching hardened-runtime Apple Events entitlement for Developer ID builds.
APPLE_EVENTS_USAGE = "Wren asks Finder to move notes you delete to the Trash."
# Every icon file is derived from ICON_SVG by `make icons` (tools/generate_icons.py)
# and committed, so packaging needs no rendering toolchain.
ICON_SVG = Path("assets/icon/wren-icon.svg")
ICON_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
ICON_ICNS = Path("assets/icon/AppIcon.icns")
ICON_CATALOG = Path("assets/icon/AppIcon.icon")
# Rust's own default for aarch64-apple-darwin; actool needs some target.
MACOS_ICON_DEPLOYMENT_TARGET = "11.0"
FONT_NOTICES = (
    Path("third_party/inter/LICENSE.txt"),
    Path("third_party/jetbrains-mono/OFL.txt"),
)
NOTICES = (
    "Wren embeds Inter and JetBrains Mono fonts in its executable.\n"
    "Their copyright and SIL Open Font License notices accompany this bundle:\n"
    "  third_party/inter/LICENSE.txt\n"
    "  third_party/jetbrains-mono/OFL.txt\n"
    "\nCargo dependency declarations, local license/copyright/NOTICE texts, and\n"
    "explicit text-coverage gaps are in DEPENDENCY_NOTICES.txt. That inventory\n"
    "also includes build/dev and other-target packages, not just linked code.\n"
    "Text collection is not a legal-completeness assurance. No Wren project\n"
    "license is chosen or granted by these third-party notices.\n"
)


class PackageError(ValueError):
    pass


def checked_path(path: Path, *, missing: bool = False) -> Path:
    path = Path(path)
    if ".." in path.parts:
        raise PackageError(f"parent traversal is not allowed: {path}")
    path = path.absolute()
    for component in reversed((path, *path.parents)):
        try:
            mode = component.lstat().st_mode
        except FileNotFoundError:
            if missing:
                continue
            raise
        if stat.S_ISLNK(mode):
            raise PackageError(f"symlinks are not allowed; use a canonical path: {component}")
        if component != path and not stat.S_ISDIR(mode):
            raise PackageError(f"path parent is not a directory: {component}")
    return path


def open_source(path: Path):
    path = checked_path(path)
    # Pin every ancestor as well as the leaf. A renamed pinned parent still
    # supplies its original file, never a file behind a replacement symlink.
    with directory_fd(path.parent) as parent:
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise PackageError(f"input is not a regular file: {path}")
        return os.fdopen(fd, "rb")
    except BaseException:
        os.close(fd)
        raise


def read_source(path: Path) -> bytes:
    with open_source(path) as source:
        return source.read()


VERSION_PATTERN = (r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
                   r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?")


def cargo_version(root: Path) -> str:
    """The app's Cargo version: X.Y.Z, or X.Y.Z-pre for a prerelease tag."""
    app = tomllib.loads(read_source(root / "crates/wren-iced/Cargo.toml").decode("utf-8"))
    version = app["package"]["version"]
    if isinstance(version, dict) and version.get("workspace") is True:
        workspace = tomllib.loads(read_source(root / "Cargo.toml").decode("utf-8"))
        version = workspace["workspace"]["package"]["version"]
    # No build metadata: release tags never carry it, and it has no Debian or
    # bundle-version spelling.
    if not isinstance(version, str) or not re.fullmatch(VERSION_PATTERN, version):
        raise PackageError("the app Cargo version must be a numeric X.Y.Z release, optionally -prerelease")
    return version


def bundle_version(version: str) -> str:
    """CFBundleVersion/CFBundleShortVersionString form: numeric X.Y.Z only.

    A prerelease records its base version, as its tag's X.Y.Z; the full
    version stays in the DMG name and the Release.
    """
    return version.split("-", 1)[0]


def release_version(cargo: str, requested: str | None) -> str:
    """Return the version an artifact records, refusing a tag the tree does not carry."""
    if requested is not None and requested != cargo:
        raise PackageError(f"release version {requested!r} does not match the Cargo version {cargo}; "
                           "run scripts/release/update-version.sh first")
    return cargo


@contextmanager
def make_directory_fd(root_fd: int, path: Path, *, mode: int = 0o755):
    if path.is_absolute() or ".." in path.parts:
        raise PackageError(f"expected a relative path without parent traversal: {path}")
    fd = os.dup(root_fd)
    try:
        for component in path.parts:
            created = False
            try:
                os.mkdir(component, mode, dir_fd=fd)
                created = True
            except FileExistsError:
                pass
            next_fd = os.open(component, DIRECTORY_FLAGS, dir_fd=fd)
            os.close(fd)
            fd = next_fd
            if created:
                os.fchmod(fd, mode)
        yield fd
    finally:
        os.close(fd)


@contextmanager
def output_file(path: Path, *, dir_fd: int, mode: int):
    with make_directory_fd(dir_fd, path.parent) as parent:
        fd = os.open(path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                     0o600, dir_fd=parent)
    try:
        with os.fdopen(fd, "wb", closefd=False) as target:
            yield target
            target.flush()
            os.fchmod(fd, mode)
    finally:
        os.close(fd)


def write_file(path: Path, contents: bytes, *, dir_fd: int, mode: int = 0o644) -> None:
    with output_file(path, dir_fd=dir_fd, mode=mode) as target:
        target.write(contents)


def check_native_format(header: bytes, platform: str) -> None:
    if platform == "linux":
        valid = (
            len(header) >= 20 and header[:4] == b"\x7fELF"
            and header[4] in (1, 2) and header[5] in (1, 2) and header[6] == 1
            and int.from_bytes(header[16:18], "little" if header[5] == 1 else "big") in (2, 3)
        )
    else:
        thin = {b"\xfe\xed\xfa\xce": "big", b"\xce\xfa\xed\xfe": "little",
                b"\xfe\xed\xfa\xcf": "big", b"\xcf\xfa\xed\xfe": "little"}
        fat = {b"\xca\xfe\xba\xbe": "big", b"\xbe\xba\xfe\xca": "little",
               b"\xca\xfe\xba\xbf": "big", b"\xbf\xba\xfe\xca": "little"}
        magic = header[:4]
        valid = (
            magic in thin and len(header) >= 28
            and int.from_bytes(header[12:16], thin[magic]) == 2
        ) or (
            magic in fat and len(header) >= 8
            and 0 < int.from_bytes(header[4:8], fat[magic]) <= 64
        )
    if not valid:
        expected = "ELF executable" if platform == "linux" else "Mach-O executable"
        raise PackageError(f"expected a native {expected}; input format does not match {platform}")


def copy_executable(binary: Path, target: Path, platform: str, native: bool, *, dir_fd: int) -> None:
    with open_source(binary) as source:
        before = os.fstat(source.fileno())
        if not before.st_mode & 0o111:
            raise PackageError(f"input does not have executable permissions: {binary}")
        if native:
            check_native_format(source.read(32), platform)
            source.seek(0)
        # Never carry setuid/setgid, ACLs, or source write permissions over.
        with output_file(target, dir_fd=dir_fd, mode=0o755) as output:
            shutil.copyfileobj(source, output)
        after = os.fstat(source.fileno())
        if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_size, after.st_mtime_ns, after.st_ctime_ns
        ):
            raise PackageError(f"input changed while it was being copied: {binary}")


def desktop_string(value: str) -> str:
    return (value.replace("\\", "\\\\").replace("\n", "\\n")
            .replace("\r", "\\r").replace("\t", "\\t"))


def desktop_exec(binary: Path) -> str:
    # Desktop-entry string escaping precedes Exec argument unquoting, and %%
    # is a literal percent, not a desktop-launcher field-code substitution.
    argument = "".join("\\" + c if c in '\\"`$' else c for c in str(binary))
    return desktop_string('"' + argument.replace("%", "%%") + '"')


def installable_desktop_entry(template: bytes) -> str:
    source = template.decode("utf-8")
    if source.count("\nExec=wren\n") != 1 or source.count("\nIcon=wren\n") != 1:
        raise PackageError("desktop asset must contain exactly one Exec=wren and Icon=wren")
    return source


def desktop_entry(template: bytes, destination: Path) -> bytes:
    source = installable_desktop_entry(template)
    source = source.replace("\nExec=wren\n", f"\nExec={desktop_exec(destination / 'bin/wren')}\n")
    icon = destination / "share/icons/hicolor/scalable/apps/wren.svg"
    return source.replace("\nIcon=wren\n", f"\nIcon={desktop_string(str(icon))}\n").encode("utf-8")


def check_icon_svg(svg: bytes) -> None:
    root = ET.fromstring(svg)
    if root.tag != "{http://www.w3.org/2000/svg}svg" or root.attrib.get("viewBox") != "0 0 1024 1024":
        raise PackageError("the icon source must be a 1024-unit SVG")


def check_icon_png(data: bytes, size: int) -> None:
    if (len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR"
            or struct.unpack_from(">II", data, 16) != (size, size)):
        raise PackageError(f"the hicolor {size}x{size} icon is not a {size}x{size} PNG; run make icons")


def check_icns(data: bytes) -> None:
    valid = len(data) > 8 and data[:4] == b"icns" and struct.unpack_from(">I", data, 4)[0] == len(data)
    offset = 8
    while valid and offset < len(data):
        length = struct.unpack_from(">I", data, offset + 4)[0] if offset + 8 <= len(data) else 0
        valid = 8 < length <= len(data) - offset
        offset += length
    if not valid:
        raise PackageError("AppIcon.icns is not a complete ICNS file; run make icons")


def icon_assets(root: Path) -> tuple[bytes, dict[int, bytes], bytes]:
    """Read and validate the committed icon assets that make icons derives from the SVG."""
    svg = read_source(root / ICON_SVG)
    check_icon_svg(svg)
    pngs = {}
    for size in ICON_SIZES:
        pngs[size] = read_source(root / hicolor_icon(size))
        check_icon_png(pngs[size], size)
    icns = read_source(root / ICON_ICNS)
    check_icns(icns)
    return svg, pngs, icns


def compile_icon_catalog(catalog: Path) -> bytes | None:
    """Compile the Icon Composer source into Assets.car, or return None.

    macOS 26 fills its glass Dock tile edge to edge only for a compiled .icon
    catalog; a loose .icns is inset as a legacy icon. actool comes with full
    Xcode 26 or later, not the Command Line Tools, so a missing or failing
    actool falls back to the flat AppIcon.icns that every bundle carries.
    """
    if shutil.which("xcrun") is None or not checked_path(catalog, missing=True).is_dir():
        return None
    target = os.environ.get("MACOSX_DEPLOYMENT_TARGET") or MACOS_ICON_DEPLOYMENT_TARGET
    with tempfile.TemporaryDirectory(prefix="wren-actool-") as temp:
        car = Path(temp) / "Assets.car"
        command = ["xcrun", "actool", str(catalog), "--compile", temp, "--platform", "macosx",
                   "--minimum-deployment-target", target, "--app-icon", "AppIcon",
                   "--output-partial-info-plist", str(Path(temp) / "partial.plist"),
                   "--errors", "--warnings"]
        try:
            compiled = subprocess.run(command, capture_output=True, timeout=300, check=False).returncode == 0
        except (OSError, subprocess.TimeoutExpired):
            compiled = False
        if not compiled or not car.is_file():
            print("Warning: actool could not compile the Icon Composer icon (full Xcode 26+ is required); "
                  "the bundle uses the flat AppIcon.icns, which macOS 26 shows inset.", file=sys.stderr)
            return None
        return car.read_bytes()


@dataclass
class BundleStage:
    name: str
    parent_fd: int
    container_fd: int
    fd: int
    published: bool = False


def same_directory_entry(parent_fd: int, name: str, fd: int) -> bool:
    try:
        entry = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    except FileNotFoundError:
        return False
    return os.path.samestat(entry, os.fstat(fd))


def clear_directory(fd: int) -> None:
    # The root is an owned descriptor, not the possibly replaced staging alias.
    for name in os.listdir(fd):
        entry = os.stat(name, dir_fd=fd, follow_symlinks=False)
        if stat.S_ISDIR(entry.st_mode):
            child = os.open(name, DIRECTORY_FLAGS, dir_fd=fd)
            try:
                if not os.path.samestat(entry, os.fstat(child)):
                    raise PackageError("owned staging directory retained: a child directory changed")
                clear_directory(child)
                if not same_directory_entry(fd, name, child):
                    raise PackageError("owned staging directory retained: a child directory was replaced")
                os.rmdir(name, dir_fd=fd)
            finally:
                os.close(child)
        else:
            os.unlink(name, dir_fd=fd)


@contextmanager
def staging_directory(parent_fd: int, destination: Path):
    for _ in range(100):
        name = f".{destination.name}.staging-{secrets.token_hex(16)}"
        try:
            os.mkdir(name, 0o700, dir_fd=parent_fd)
            break
        except FileExistsError:
            pass
    else:
        raise PackageError("could not create a unique private staging directory")
    try:
        container = os.open(name, DIRECTORY_FLAGS, dir_fd=parent_fd)
    except OSError as error:
        raise PackageError(f"owned staging directory retained; could not pin it safely: {name}") from error
    fd = None
    stage = None
    try:
        if not same_directory_entry(parent_fd, name, container):
            raise PackageError("owned staging directory retained: staging pathname changed during creation")
        # The extra private parent keeps the rename source unreachable through
        # a substituted alias in a shared-writable output parent.
        os.mkdir("bundle", 0o700, dir_fd=container)
        fd = os.open("bundle", DIRECTORY_FLAGS, dir_fd=container)
        stage = BundleStage(name, parent_fd, container, fd)
        yield stage
    finally:
        try:
            if fd is not None and (stage is None or not stage.published):
                os.fchmod(fd, 0o700)
                clear_directory(fd)
                if not same_directory_entry(container, "bundle", fd):
                    raise PackageError("owned staging directory retained: bundle pathname was replaced")
                os.rmdir("bundle", dir_fd=container)
            if not same_directory_entry(parent_fd, name, container):
                raise PackageError(
                    f"owned staging directory retained after cleanup; refusing a renamed or replaced pathname: {name}"
                )
            os.rmdir(name, dir_fd=parent_fd)
        except OSError as error:
            raise PackageError(f"owned staging directory retained; cleanup could not finish safely: {name}") from error
        finally:
            if fd is not None:
                os.close(fd)
            os.close(container)


def check_output_parent(parent_fd: int, destination: Path) -> None:
    try:
        with directory_fd(destination.parent) as current:
            if os.path.samestat(os.fstat(current), os.fstat(parent_fd)):
                return
    except OSError:
        pass
    raise PackageError(f"output parent was renamed or replaced; refusing its replacement: {destination.parent}")


def publish_bundle(stage: BundleStage, destination: Path) -> None:
    check_output_parent(stage.parent_fd, destination)
    if not same_directory_entry(stage.parent_fd, stage.name, stage.container_fd):
        raise PackageError("staging pathname was renamed or replaced; refusing to publish its replacement")
    if not same_directory_entry(stage.container_fd, "bundle", stage.fd):
        raise PackageError("bundle pathname was replaced; refusing to publish its replacement")
    # Both endpoints are relative to pinned parents. In particular, the source
    # lives inside the 0700 container, not at its exposed, renameable alias.
    libc = ctypes.CDLL(None, use_errno=True)
    if sys.platform == "linux" and hasattr(libc, "renameat2"):
        rename = libc.renameat2
        flag = 1  # RENAME_NOREPLACE
    elif sys.platform == "darwin" and hasattr(libc, "renameatx_np"):
        rename = libc.renameatx_np
        flag = 4  # RENAME_EXCL
    else:
        raise PackageError("host does not provide a supported no-replace directory rename")
    rename.argtypes = (ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint)
    rename.restype = ctypes.c_int
    os.fchmod(stage.fd, 0o755)
    if rename(stage.container_fd, b"bundle", stage.parent_fd, os.fsencode(destination.name), flag) != 0:
        error = ctypes.get_errno()
        os.fchmod(stage.fd, 0o700)
        raise OSError(error, os.strerror(error), str(destination))
    stage.published = True
    # A replacement after validation cannot redirect the native rename. Report
    # it instead of returning a path that now names an unrelated parent.
    check_output_parent(stage.parent_fd, destination)


def assemble_bundle(binary: Path, destination: Path, platform: str, *,
                    dependency_notices: bytes, root: Path = ROOT, native: bool = False,
                    system_layout: bool = False, version: str | None = None) -> Path:
    """Assemble supplied notices/layout without Cargo or executing the input binary.

    system_layout stages a Linux tree to be installed under /usr by a system
    package; version is a release tag's version, checked against Cargo's.
    """
    if platform not in ("linux", "macos"):
        raise PackageError(f"unsupported platform: {platform}")
    if system_layout and platform != "linux":
        raise PackageError("the system layout is Linux-only")
    root = checked_path(root)
    binary = checked_path(binary)
    destination = checked_path(destination, missing=True)
    if os.path.lexists(destination):
        raise PackageError(f"output already exists; choose a new --output: {destination}")
    if platform == "macos" and destination.name != "Wren.app":
        raise PackageError("the macOS output must be named Wren.app")
    if platform == "linux" and not system_layout and "=" in str(destination):
        raise PackageError("Linux output paths must not contain '='; desktop Exec executable paths forbid it")
    if not isinstance(dependency_notices, bytes) or not dependency_notices.strip():
        raise PackageError("nonempty Cargo dependency notices must be supplied before assembly")
    try:
        dependency_notices.decode("utf-8")
    except UnicodeDecodeError:
        raise PackageError("Cargo dependency notices must be UTF-8 text") from None
    release = release_version(cargo_version(root), version)
    with (
        directory_fd(Path(destination.anchor)) as anchor,
        make_directory_fd(anchor, destination.parent.relative_to(destination.anchor)) as parent,
        staging_directory(parent, destination) as stage,
    ):
        svg, pngs, icns = icon_assets(root)
        if platform == "linux":
            copy_executable(binary, Path("bin/wren"), platform, native, dir_fd=stage.fd)
            icons = Path("share/icons/hicolor")
            write_file(icons / "scalable/apps/wren.svg", svg, dir_fd=stage.fd)
            for size, png in pngs.items():
                write_file(icons / f"{size}x{size}/apps/wren.png", png, dir_fd=stage.fd)
            template = read_source(root / "assets/wren.desktop")
            entry = (installable_desktop_entry(template).encode("utf-8") if system_layout
                     else desktop_entry(template, destination))
            write_file(Path("share/applications/wren.desktop"), entry, dir_fd=stage.fd)
            notices = Path("share/doc/wren")
            write_file(notices / "VERSION", (release + "\n").encode("utf-8"), dir_fd=stage.fd)
        else:
            copy_executable(binary, Path("Contents/MacOS/wren"), platform, native, dir_fd=stage.fd)
            resources = Path("Contents/Resources")
            info = {
                "CFBundleDevelopmentRegion": "en",
                "CFBundleDisplayName": "Wren",
                "CFBundleExecutable": "wren",
                "CFBundleIconFile": "AppIcon.icns",
                "CFBundleIdentifier": BUNDLE_ID,
                "CFBundleInfoDictionaryVersion": "6.0",
                "CFBundleName": "Wren",
                "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": bundle_version(release),
                "CFBundleVersion": bundle_version(release),
                "LSApplicationCategoryType": "public.app-category.productivity",
                "NSAppleEventsUsageDescription": APPLE_EVENTS_USAGE,
                "NSHighResolutionCapable": True,
                "NSPrincipalClass": "NSApplication",
                "NSSupportsAutomaticGraphicsSwitching": True,
            }
            # Only a native build compiles the macOS 26 icon; fixtures and
            # Linux hosts ship the flat ICNS alone.
            catalog = compile_icon_catalog(root / ICON_CATALOG) if native else None
            if catalog is not None:
                info["CFBundleIconName"] = "AppIcon"
                write_file(resources / "Assets.car", catalog, dir_fd=stage.fd)
            write_file(Path("Contents/Info.plist"), plistlib.dumps(info), dir_fd=stage.fd)
            write_file(Path("Contents/PkgInfo"), b"APPL????", dir_fd=stage.fd)
            write_file(resources / "AppIcon.icns", icns, dir_fd=stage.fd)
            notices = resources
        for notice in FONT_NOTICES:
            write_file(notices / notice, read_source(root / notice), dir_fd=stage.fd)
        write_file(notices / "DEPENDENCY_NOTICES.txt", dependency_notices, dir_fd=stage.fd)
        write_file(notices / "THIRD_PARTY_NOTICES.txt", NOTICES.encode("utf-8"), dir_fd=stage.fd)
        publish_bundle(stage, destination)
        return destination


def hicolor_icon(size: int) -> Path:
    return Path(f"assets/icon/hicolor/{size}x{size}/apps/wren.png")


def native_platform() -> str:
    if sys.platform == "linux":
        return "linux"
    if sys.platform == "darwin":
        return "macos"
    raise PackageError(f"unsupported packaging host: {sys.platform}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="explicit, already built native wren executable")
    parser.add_argument("--platform", choices=("linux", "macos"), help="defaults to the native host; cross-packaging is rejected")
    parser.add_argument("--output", type=Path, help="new bundle path; existing paths are never replaced")
    parser.add_argument("--release-version", help="release tag version (X.Y.Z or X.Y.Z-pre); must equal the Cargo version")
    parser.add_argument("--system-layout", action="store_true",
                        help="Linux only: stage a tree a system package installs under /usr")
    args = parser.parse_args(argv)
    try:
        host = native_platform()
        platform = args.platform or host
        if platform != host:
            raise PackageError(f"cannot package {platform} on a {host} host; use a native build host")
        if args.system_layout and platform != "linux":
            raise PackageError("the system layout is Linux-only")
        version = release_version(cargo_version(ROOT), args.release_version)
        suffix = "-system" if args.system_layout else ""
        output = args.output or ROOT / "target/package" / f"wren-{version}-{platform}{suffix}"
        if platform == "macos" and args.output is None:
            output /= "Wren.app"
        report = collect_notices(ROOT)
        result = assemble_bundle(args.binary, output, platform, dependency_notices=report.render(), root=ROOT,
                                 native=True, system_layout=args.system_layout, version=args.release_version)
    except (OSError, ValueError, KeyError, ET.ParseError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        return 1
    kind = "system-layout linux tree" if args.system_layout else f"uninstalled {platform} bundle"
    print(f"Assembled {kind}: {result}")
    print(report.summary())
    if report.unresolved:
        print("Warning: dependency text coverage requires review; see bundled DEPENDENCY_NOTICES.txt before redistribution.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
