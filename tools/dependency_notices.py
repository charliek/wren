"""Inventory Cargo dependency declarations and copy bounded local notice texts.

Collection is deliberately broader than a native executable's link graph: all
non-workspace packages from unfiltered locked Cargo metadata are included,
including build/dev dependencies and packages for other targets. Text presence
is reported, not interpreted as proof of license compliance or completeness.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
MAX_TEXT_BYTES = 1024 * 1024
MAX_PACKAGE_BYTES = 4 * MAX_TEXT_BYTES
MAX_FILES = 128
MAX_DIRECTORY_ENTRIES = 512
MAX_DIRECTORY_DEPTH = 2
NOTICE_NAME = re.compile(r"^(?:licen[cs]e|copying|notice|copyright|unlicense|authors)(?:$|[._-])", re.I)
NOTICE_DIRECTORIES = {"license", "licenses", "licence", "licences", "legal", "notice", "notices"}
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_NONBLOCK
READ_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK


class NoticeError(ValueError):
    pass


@dataclass(frozen=True)
class LicenseText:
    path: str
    text: str


@dataclass(frozen=True)
class PackageNotice:
    name: str
    version: str
    spdx: str | None
    texts: tuple[LicenseText, ...]
    issues: tuple[str, ...]

    @property
    def label(self) -> str:
        return f"{self.name}@{self.version}"

    @property
    def needs_review(self) -> bool:
        return not self.texts or not self.spdx or bool(self.issues)


@dataclass(frozen=True)
class NoticeReport:
    packages: tuple[PackageNotice, ...]
    workspace_count: int

    @property
    def text_count(self) -> int:
        return sum(len(package.texts) for package in self.packages)

    @property
    def unresolved(self) -> tuple[str, ...]:
        return tuple(package.label for package in self.packages if package.needs_review)

    def summary(self) -> str:
        return (f"Cargo dependency notices: {len(self.packages)} packages, {self.text_count} local texts, "
                f"{len(self.unresolved)} packages requiring review; {self.workspace_count} workspace members excluded.")

    def render(self) -> bytes:
        missing = sum(not package.texts for package in self.packages)
        lines = [
            "Wren — Cargo dependency notices",
            "================================",
            "",
            "Scope: every non-workspace package in cargo metadata --locked --format-version 1",
            "(unfiltered, offline). This transitive inventory includes build/dev dependencies",
            "and target-specific packages; it is not a claim that every package is linked",
            "into this particular executable. Native/system libraries are not inventoried.",
            f"Workspace members excluded: {self.workspace_count}. No license is chosen or granted for Wren.",
            "",
            "SPDX expressions below are package declarations, not substituted license texts.",
            "Texts are copied only from local package sources. No license text is downloaded,",
            "no standard-license fallback is supplied, and no copyright is invented.",
            "Source URLs, package IDs, and absolute/cache paths from Cargo metadata are omitted.",
            "",
            "Discovery: common LICENSE/LICENCE, COPYING, NOTICE, COPYRIGHT, UNLICENSE and",
            "AUTHORS files at the package root, declared license_file inside that root, and",
            "files in license(s)/licence(s)/legal/notice(s) directories (at most two directory",
            "levels below the package root). Other source/vendor directories are not scanned.",
            f"Limits: {MAX_TEXT_BYTES} bytes/file, {MAX_PACKAGE_BYTES} bytes/package, {MAX_FILES} files/package,",
            f"and {MAX_DIRECTORY_ENTRIES} entries/directory. Only regular, nonempty UTF-8 text is copied;",
            "symlinks, outside paths, special files, binary/undecodable and oversized files are rejected.",
            "",
            self.summary(),
            f"Packages with no collected text: {missing}.",
            "Text presence does NOT establish that an SPDX expression, all copyright holders,",
            "vendored components, or redistribution obligations are fully covered. Missing",
            "texts and collection issues below require review before redistribution; even a",
            "package without a reported issue is not a legal-completeness determination.",
            "",
            "Packages requiring review:",
            *(f"  {label}" for label in self.unresolved),
            *([] if self.unresolved else ["  None flagged by this bounded collector (not a legal assurance)."]),
            "",
        ]
        for package in self.packages:
            lines.extend([
                "=" * 72,
                f"Package: {package.name}",
                f"Version: {package.version}",
                f"License (SPDX, as declared): {package.spdx or 'NOT DECLARED'}",
                (f"Text coverage: {len(package.texts)} local file(s) collected; completeness not established."
                 if package.texts else "Text coverage: MISSING — no local license/notice text collected."),
            ])
            if not package.texts and package.spdx:
                lines.append("An SPDX declaration alone does not supply the license text or copyright notice.")
            for issue in package.issues:
                lines.append(f"Collection issue: {issue}")
            for text in package.texts:
                label = json.dumps(text.path, ensure_ascii=True)
                lines.extend(["", f"--- Local file: {label} ---", text.text, f"--- End local file: {label} ---"])
            lines.append("")
        return ("\n".join(lines) + "\n").encode("utf-8")


def load_metadata(root: Path) -> dict:
    """Never forward Cargo output: errors can contain credential-bearing URLs."""
    try:
        result = subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1", "--offline"],
            cwd=root, capture_output=True, check=False, timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired):
        raise NoticeError("could not run locked offline Cargo metadata; Cargo output suppressed") from None
    if result.returncode:
        raise NoticeError(
            f"locked offline Cargo metadata failed (exit {result.returncode}); "
            "ensure dependencies are cached with cargo fetch --locked; Cargo output suppressed"
        )
    try:
        metadata = json.loads(result.stdout)
    except (ValueError, UnicodeDecodeError):
        raise NoticeError("Cargo metadata was not valid JSON; Cargo output suppressed") from None
    if not isinstance(metadata, dict):
        raise NoticeError("Cargo metadata must be an object; Cargo output suppressed")
    return metadata


@contextmanager
def directory_fd(path: Path):
    """Anchor every component, including ancestors, without following symlinks."""
    if not path.is_absolute() or ".." in path.parts:
        raise NoticeError("package/directory root must be absolute without parent traversal")
    fd = os.open(path.anchor, DIRECTORY_FLAGS)
    try:
        for component in path.parts[1:]:
            next_fd = os.open(component, DIRECTORY_FLAGS, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        yield fd
    finally:
        os.close(fd)


@contextmanager
def relative_directory_fd(root_fd: int, parts: tuple[str, ...]):
    fd = os.dup(root_fd)
    try:
        for component in parts:
            next_fd = os.open(component, DIRECTORY_FLAGS, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        yield fd
    finally:
        os.close(fd)


def relative_label(parts: tuple[str, ...]) -> str:
    # JSON quoting escapes control characters in local names without including
    # the private absolute root or the raw declared license_file value.
    return json.dumps("/".join(parts) or ".", ensure_ascii=True)


def declared_file(value: object, root: Path) -> tuple[str, ...]:
    if (not isinstance(value, str) or not value or "\\" in value or "\x00" in value
            or "://" in value or re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", value.lstrip("./"))):
        raise NoticeError("declared license_file rejected: invalid path")
    path = Path(value)
    if ".." in path.parts:
        raise NoticeError("declared license_file rejected: parent traversal")
    if path.is_absolute():
        try:
            path = path.relative_to(root)
        except ValueError:
            raise NoticeError("declared license_file rejected: outside package root") from None
    if not path.parts:
        raise NoticeError("declared license_file rejected: not a file path")
    return path.parts


def discover_files(root_fd: int) -> tuple[set[tuple[str, ...]], list[str]]:
    files: set[tuple[str, ...]] = set()
    issues: list[str] = []

    def scan(parts: tuple[str, ...]) -> None:
        try:
            with relative_directory_fd(root_fd, parts) as fd:
                with os.scandir(fd) as entries:
                    names = []
                    for entry in entries:
                        names.append(entry.name)
                        if len(names) > MAX_DIRECTORY_ENTRIES:
                            issues.append(f"directory entry limit exceeded: {relative_label(parts)}")
                            return
                for name in sorted(names):
                    if len(files) > MAX_FILES:
                        return
                    relative = (*parts, name)
                    mode = os.stat(name, dir_fd=fd, follow_symlinks=False).st_mode
                    if not parts:
                        if name.lower() in NOTICE_DIRECTORIES:
                            if stat.S_ISDIR(mode):
                                scan(relative)
                            else:
                                files.add(relative)
                        elif NOTICE_NAME.match(name):
                            if stat.S_ISDIR(mode):
                                issues.append(f"notice candidate is a directory: {relative_label(relative)}")
                            else:
                                files.add(relative)
                    elif stat.S_ISDIR(mode):
                        if len(parts) < MAX_DIRECTORY_DEPTH:
                            scan(relative)
                        else:
                            issues.append(f"notice directory depth limit exceeded: {relative_label(relative)}")
                    else:
                        files.add(relative)
        except OSError:
            issues.append(f"cannot scan notice directory safely: {relative_label(parts)}")

    scan(())
    return files, issues


def read_text(root_fd: int, parts: tuple[str, ...]) -> str:
    try:
        with relative_directory_fd(root_fd, parts[:-1]) as directory:
            fd = os.open(parts[-1], READ_FLAGS, dir_fd=directory)
        try:
            before = os.fstat(fd)
            if not stat.S_ISREG(before.st_mode):
                raise NoticeError("not a regular file")
            if before.st_size > MAX_TEXT_BYTES:
                raise NoticeError("file size limit exceeded")
            with os.fdopen(fd, "rb", closefd=False) as source:
                data = source.read(MAX_TEXT_BYTES + 1)
            after = os.fstat(fd)
            if len(data) > MAX_TEXT_BYTES:
                raise NoticeError("file size limit exceeded")
            if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
                after.st_size, after.st_mtime_ns, after.st_ctime_ns
            ):
                raise NoticeError("file changed during collection")
        finally:
            os.close(fd)
    except OSError:
        raise NoticeError("file missing, unreadable, or unsafe (symlinks are not followed)") from None
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise NoticeError("not UTF-8 text") from None
    if not text.strip("\ufeff \t\r\n\f"):
        raise NoticeError("empty text")
    if any(ord(character) < 32 and character not in "\n\r\t\f" for character in text):
        raise NoticeError("binary/control characters in text")
    return text


def package_notice(package: dict) -> PackageNotice:
    name, version = package.get("name"), package.get("version")
    if (not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", name)
            or not isinstance(version, str) or not re.fullmatch(r"[0-9][A-Za-z0-9.+-]{0,127}", version)):
        raise NoticeError("invalid Cargo package name/version; raw metadata omitted")
    issues: list[str] = []
    spdx = package.get("license")
    if spdx is not None and (not isinstance(spdx, str) or not re.fullmatch(r"[A-Za-z0-9.+() /-]{1,512}", spdx)):
        spdx = None
        issues.append("invalid SPDX declaration omitted")
    if not spdx:
        issues.append("no SPDX license expression declared; no permissions inferred")
    manifest = package.get("manifest_path")
    if not isinstance(manifest, str) or "\x00" in manifest:
        raise NoticeError("invalid Cargo manifest path; raw metadata omitted")
    path = Path(manifest)
    if path.name != "Cargo.toml" or not path.is_absolute() or ".." in path.parts:
        raise NoticeError("invalid Cargo manifest path; raw metadata omitted")
    root = path.parent
    texts = []
    try:
        with directory_fd(root) as fd:
            if not stat.S_ISREG(os.stat("Cargo.toml", dir_fd=fd, follow_symlinks=False).st_mode):
                raise NoticeError("package manifest is not a regular file")
            files, discovery_issues = discover_files(fd)
            issues.extend(discovery_issues)
            if package.get("license_file") is not None:
                try:
                    files.add(declared_file(package["license_file"], root))
                except NoticeError as error:
                    issues.append(str(error))
            if len(files) > MAX_FILES:
                issues.append("notice file count limit exceeded; no texts copied")
            else:
                total = 0
                for parts in sorted(files):
                    try:
                        text = read_text(fd, parts)
                        total += len(text.encode("utf-8"))
                        if total > MAX_PACKAGE_BYTES:
                            issues.append("package text size limit exceeded; remaining texts not copied")
                            break
                        texts.append(LicenseText("/".join(parts), text))
                    except NoticeError as error:
                        issues.append(f"{relative_label(parts)}: {error}")
    except (OSError, NoticeError):
        issues.append("local package root/manifest unavailable or unsafe; no outside paths or symlinks followed")
    return PackageNotice(name, version, spdx, tuple(texts), tuple(sorted(set(issues))))


def notices_from_metadata(metadata: dict) -> NoticeReport:
    """Read only metadata-selected package roots; never run Cargo or use source URLs."""
    packages, members = metadata.get("packages"), metadata.get("workspace_members")
    if (not isinstance(packages, list) or not isinstance(members, list)
            or not all(isinstance(member, str) for member in members)
            or not all(isinstance(package, dict) and isinstance(package.get("id"), str) for package in packages)):
        raise NoticeError("Cargo metadata packages/workspace_members are invalid; raw metadata omitted")
    ids = [package["id"] for package in packages]
    if len(set(ids)) != len(ids) or len(set(members)) != len(members) or not set(members).issubset(ids):
        raise NoticeError("Cargo metadata package/workspace identities are inconsistent; raw metadata omitted")
    own = set(members)
    notices = [package_notice(package) for package in packages if package["id"] not in own]
    # Package IDs can contain private source URLs. Content-based tie breaking
    # handles equal name/version packages without emitting or sorting by IDs.
    notices.sort(key=lambda notice: (notice.name, notice.version, notice.spdx or "",
                                    tuple((text.path, text.text) for text in notice.texts), notice.issues))
    return NoticeReport(tuple(notices), len(members))


def collect_notices(root: Path = ROOT) -> NoticeReport:
    return notices_from_metadata(load_metadata(root))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT, help="Cargo workspace root (dependencies must already be cached)")
    parser.add_argument("--output", type=Path, required=True, help="new UTF-8 report; parent directory must already exist")
    args = parser.parse_args(argv)
    try:
        report = collect_notices(args.root)
        rendered = report.render()
        output = args.output.absolute()
        if ".." in args.output.parts:
            raise NoticeError("output parent traversal is not allowed")
        with directory_fd(output.parent) as directory:
            fd = os.open(output.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644, dir_fd=directory)
            with os.fdopen(fd, "wb") as target:
                target.write(rendered)
    except (OSError, ValueError):
        print("Dependency notice collection failed; existing outputs were not replaced. Raw paths/Cargo output suppressed.", file=sys.stderr)
        return 1
    print(report.summary())
    print("Packages requiring review: " + (", ".join(report.unresolved) or "none flagged (not a legal assurance)"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
