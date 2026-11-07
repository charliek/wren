"""Assemble an uninstalled Wren bundle from an explicitly supplied executable.

The CLI requires the matching host and an ELF (Linux) or Mach-O (macOS) header.
assemble_bundle also supports cross-platform layout tests with fixture bytes;
it does not establish that a supplied binary can run. No build, installation,
signing, download, or application-configuration steps are performed. The CLI
collects dependency texts using locked, offline Cargo metadata before assembly;
run cargo fetch --locked separately if the locked sources are not cached.
Existing outputs are never replaced; choose a new --output for another bundle.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import ctypes
from dataclasses import dataclass
from functools import lru_cache
import math
import os
from pathlib import Path
import plistlib
import re
import secrets
import shutil
import stat
import struct
import sys
import tomllib
import xml.etree.ElementTree as ET
import zlib

if __package__:
    from .dependency_notices import collect_notices, directory_fd
else:
    from dependency_notices import collect_notices, directory_fd

DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_NONBLOCK

ROOT = Path(__file__).resolve().parents[1]
BUNDLE_ID = "io.github.charliek.wren"
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


def cargo_version(root: Path) -> str:
    app = tomllib.loads(read_source(root / "crates/wren-iced/Cargo.toml").decode("utf-8"))
    version = app["package"]["version"]
    if isinstance(version, dict) and version.get("workspace") is True:
        workspace = tomllib.loads(read_source(root / "Cargo.toml").decode("utf-8"))
        version = workspace["workspace"]["package"]["version"]
    # Both CFBundleVersion and CFBundleShortVersionString accept this release
    # version form. Do not silently invent a numeric version for a prerelease.
    if not isinstance(version, str) or not re.fullmatch(
        r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version
    ):
        raise PackageError("the app Cargo version must be a three-component numeric release")
    return version


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


def desktop_entry(template: bytes, destination: Path) -> bytes:
    source = template.decode("utf-8")
    if source.count("\nExec=wren\n") != 1 or source.count("\nIcon=wren\n") != 1:
        raise PackageError("desktop asset must contain exactly one Exec=wren and Icon=wren")
    source = source.replace("\nExec=wren\n", f"\nExec={desktop_exec(destination / 'bin/wren')}\n")
    icon = destination / "share/icons/hicolor/scalable/apps/wren.svg"
    return source.replace("\nIcon=wren\n", f"\nIcon={desktop_string(str(icon))}\n").encode("utf-8")


def path_polygon(data: str) -> list[tuple[float, float]]:
    tokens = re.findall(r"[MLCQZ]|-?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)", data)
    if re.sub(r"[MLCQZ]|-?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)|[\s,]+", "", data):
        raise PackageError("icon path uses an unsupported SVG command")
    points = []
    index = 0
    while index < len(tokens):
        command = tokens[index]
        count = {"M": 2, "L": 2, "C": 6, "Q": 4, "Z": 0}.get(command)
        if count is None or index + count >= len(tokens):
            raise PackageError("invalid icon path")
        values = [float(value) for value in tokens[index + 1:index + count + 1]]
        index += count + 1
        if command in ("M", "L"):
            if command == "M" and points:
                raise PackageError("icon paths must have a single closed subpath")
            points.append((values[0], values[1]))
        elif command in ("C", "Q"):
            if not points:
                raise PackageError("icon curve must follow M")
            x, y = points[-1]
            for step in range(1, 25):
                t = step / 24
                u = 1 - t
                if command == "C":
                    points.append((u**3*x + 3*u*u*t*values[0] + 3*u*t*t*values[2] + t**3*values[4],
                                   u**3*y + 3*u*u*t*values[1] + 3*u*t*t*values[3] + t**3*values[5]))
                else:
                    points.append((u*u*x + 2*u*t*values[0] + t*t*values[2],
                                   u*u*y + 2*u*t*values[1] + t*t*values[3]))
        elif command == "Z" and index != len(tokens):
            raise PackageError("icon path must end at its closing command")
    if len(points) < 3 or not tokens or tokens[-1] != "Z":
        raise PackageError("icon path must be a closed polygon")
    return points


def icon_shapes(svg: bytes) -> list[tuple[list[tuple[float, float]], bytes]]:
    # This is deliberately a renderer for our flat, solid-fill icon, not a
    # general SVG engine. Fail instead of silently dropping unsupported art.
    root = ET.fromstring(svg)
    ns = "{http://www.w3.org/2000/svg}"
    if root.tag != ns + "svg" or root.attrib.get("viewBox") != "0 0 256 256":
        raise PackageError("icon must be a 256-unit SVG")
    shapes = []
    for shape in root:
        tag = shape.tag.removeprefix(ns)
        if tag in ("title", "desc"):
            continue
        allowed = {
            "path": {"d", "fill"},
            "circle": {"cx", "cy", "r", "fill"},
            "rect": {"x", "y", "width", "height", "rx", "fill"},
        }
        if tag not in allowed or set(shape.attrib) - allowed[tag] or len(shape):
            raise PackageError(f"unsupported icon shape or attributes: {tag}")
        fill = shape.attrib.get("fill", "")
        if not re.fullmatch(r"#[0-9a-fA-F]{6}", fill):
            raise PackageError("icon shapes must use solid RGB fills")
        rgba = bytes.fromhex(fill[1:]) + b"\xff"
        if tag == "path":
            points = path_polygon(shape.attrib["d"])
        elif tag == "circle":
            x, y, radius = (float(shape.attrib[key]) for key in ("cx", "cy", "r"))
            points = [(x + radius*math.cos(i*math.tau/96),
                       y + radius*math.sin(i*math.tau/96)) for i in range(96)]
        elif tag == "rect":
            x, y, width, height = (float(shape.attrib[key]) for key in ("x", "y", "width", "height"))
            radius = float(shape.attrib.get("rx", "0"))
            points = []
            for cx, cy, start in ((x+width-radius, y+radius, -math.pi/2),
                                  (x+width-radius, y+height-radius, 0),
                                  (x+radius, y+height-radius, math.pi/2),
                                  (x+radius, y+radius, math.pi)):
                points.extend((cx + radius*math.cos(start+i*math.pi/48),
                               cy + radius*math.sin(start+i*math.pi/48)) for i in range(25))
        else:
            raise PackageError(f"unsupported icon shape: {tag}")
        shapes.append((points, rgba))
    if not shapes:
        raise PackageError("icon is empty")
    return shapes


def png_chunk(kind: bytes, data: bytes) -> bytes:
    return (struct.pack(">I", len(data)) + kind + data
            + struct.pack(">I", zlib.crc32(kind + data)))


@lru_cache(maxsize=8)
def icon_png(svg: bytes, size: int) -> bytes:
    if size not in (128, 256, 512):
        raise PackageError("supported icon sizes are 128, 256, and 512")
    scale = size * 2 / 256
    side = size * 2
    pixels = bytearray(side * side * 4)
    for polygon, rgba in icon_shapes(svg):
        points = [(x*scale, y*scale) for x, y in polygon]
        edges = list(zip(points, points[1:] + points[:1]))
        for y in range(max(0, math.ceil(min(y for _, y in points) - 0.5)),
                       min(side, math.ceil(max(y for _, y in points) - 0.5))):
            scan = y + 0.5
            xs = sorted(x1 + (scan-y1)*(x2-x1)/(y2-y1)
                        for (x1, y1), (x2, y2) in edges
                        if min(y1, y2) <= scan < max(y1, y2))
            for left, right in zip(xs[::2], xs[1::2]):
                begin = max(0, math.ceil(left - 0.5))
                end = min(side, math.ceil(right - 0.5))
                if begin < end:
                    offset = (y*side + begin)*4
                    pixels[offset:offset + (end-begin)*4] = rgba*(end-begin)
    scanlines = bytearray()
    for y in range(size):
        scanlines.append(0)
        for x in range(size):
            offset = (y*2*side + x*2)*4
            samples = (offset, offset+4, offset+side*4, offset+side*4+4)
            coverage = sum(pixels[sample+3] for sample in samples)
            # Unpremultiplied PNG colors must retain their brightness along
            # a partially transparent edge, rather than acquire a dark halo.
            for c in range(3):
                scanlines.append(sum(pixels[sample+c] for sample in samples)*255 // coverage
                                 if coverage else 0)
            scanlines.append((coverage + 2) // 4)
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", header)
            + png_chunk(b"IDAT", zlib.compress(scanlines, 9)) + png_chunk(b"IEND", b""))


def icon_icns(svg: bytes) -> bytes:
    chunks = []
    for kind, size in ((b"ic07", 128), (b"ic08", 256), (b"ic09", 512)):
        png = icon_png(svg, size)
        chunks.append(kind + struct.pack(">I", len(png) + 8) + png)
    data = b"".join(chunks)
    return b"icns" + struct.pack(">I", len(data) + 8) + data


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
                    dependency_notices: bytes, root: Path = ROOT, native: bool = False) -> Path:
    """Assemble supplied notices/layout without Cargo or executing the input binary."""
    if platform not in ("linux", "macos"):
        raise PackageError(f"unsupported platform: {platform}")
    root = checked_path(root)
    binary = checked_path(binary)
    destination = checked_path(destination, missing=True)
    if os.path.lexists(destination):
        raise PackageError(f"output already exists; choose a new --output: {destination}")
    if platform == "macos" and destination.name != "Wren.app":
        raise PackageError("the macOS output must be named Wren.app")
    if platform == "linux" and "=" in str(destination):
        raise PackageError("Linux output paths must not contain '='; desktop Exec executable paths forbid it")
    if not isinstance(dependency_notices, bytes) or not dependency_notices.strip():
        raise PackageError("nonempty Cargo dependency notices must be supplied before assembly")
    try:
        dependency_notices.decode("utf-8")
    except UnicodeDecodeError:
        raise PackageError("Cargo dependency notices must be UTF-8 text") from None
    version = cargo_version(root)
    with (
        directory_fd(Path(destination.anchor)) as anchor,
        make_directory_fd(anchor, destination.parent.relative_to(destination.anchor)) as parent,
        staging_directory(parent, destination) as stage,
    ):
        svg = read_source(root / "assets/wren.svg")
        icon_shapes(svg)
        if platform == "linux":
            copy_executable(binary, Path("bin/wren"), platform, native, dir_fd=stage.fd)
            write_file(Path("share/icons/hicolor/scalable/apps/wren.svg"), svg, dir_fd=stage.fd)
            template = read_source(root / "assets/wren.desktop")
            write_file(Path("share/applications/wren.desktop"), desktop_entry(template, destination), dir_fd=stage.fd)
            notices = Path("share/doc/wren")
            write_file(notices / "VERSION", (version + "\n").encode("utf-8"), dir_fd=stage.fd)
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
                "CFBundleShortVersionString": version,
                "CFBundleVersion": version,
                "LSApplicationCategoryType": "public.app-category.productivity",
                "NSHighResolutionCapable": True,
                "NSPrincipalClass": "NSApplication",
                "NSSupportsAutomaticGraphicsSwitching": True,
            }
            write_file(Path("Contents/Info.plist"), plistlib.dumps(info), dir_fd=stage.fd)
            write_file(Path("Contents/PkgInfo"), b"APPL????", dir_fd=stage.fd)
            write_file(resources / "AppIcon.icns", icon_icns(svg), dir_fd=stage.fd)
            write_file(resources / "wren.svg", svg, dir_fd=stage.fd)
            notices = resources
        for notice in FONT_NOTICES:
            write_file(notices / notice, read_source(root / notice), dir_fd=stage.fd)
        write_file(notices / "DEPENDENCY_NOTICES.txt", dependency_notices, dir_fd=stage.fd)
        write_file(notices / "THIRD_PARTY_NOTICES.txt", NOTICES.encode("utf-8"), dir_fd=stage.fd)
        publish_bundle(stage, destination)
        return destination


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
    args = parser.parse_args(argv)
    try:
        host = native_platform()
        platform = args.platform or host
        if platform != host:
            raise PackageError(f"cannot package {platform} on a {host} host; use a native build host")
        version = cargo_version(ROOT)
        output = args.output or ROOT / "target/package" / f"wren-{version}-{platform}"
        if platform == "macos" and args.output is None:
            output /= "Wren.app"
        report = collect_notices(ROOT)
        result = assemble_bundle(args.binary, output, platform, dependency_notices=report.render(), root=ROOT, native=True)
    except (OSError, ValueError, KeyError, ET.ParseError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        return 1
    print(f"Assembled uninstalled {platform} bundle: {result}")
    print(report.summary())
    if report.unresolved:
        print("Warning: dependency text coverage requires review; see bundled DEPENDENCY_NOTICES.txt before redistribution.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
