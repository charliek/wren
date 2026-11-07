"""Release asset names, and the check that gates publishing a draft release.

    python3 tools/release_assets.py deb-name v0.1.0 amd64     # wren_0.1.0_amd64.deb
    python3 tools/release_assets.py dmg-name v0.1.0           # Wren-0.1.0.dmg
    gh release view v0.1.0 --json assets \\
        -q '.assets[] | [.name, (.size|tostring)] | @tsv' \\
      | python3 tools/release_assets.py check v0.1.0

`release.yml` names its uploads with the first two and runs `check` in
`publish-release`, the one irreversible step: the draft holds exactly one
amd64 .deb, one arm64 .deb and one DMG, each named for the tag and at least
MIN_ARTIFACT_BYTES (a truncated upload lands above zero, so "non-empty" would
wave through the corruption worth catching), and nothing else. apt-charliek
globs `wren_*.deb`, so a stale .deb left on a reused draft would ship.

nfpm's semver schema splits a version at its FIRST hyphen and rejoins with
`~` (0.1.0-rc-2 -> 0.1.0~rc-2), so a prerelease .deb sorts before its final
release. GitHub then rewrites every character outside `[A-Za-z0-9._-]` of an
uploaded file name to `.`, so that `~` is stored as `.`; expected names are
compared after the same rewrite. Standard library only: `publish-release`
runs it with the runner's python3, without the project environment.
"""

from __future__ import annotations

import argparse
from collections.abc import Iterable
import re
import sys

PACKAGE = "wren"
DEB_ARCHES = ("amd64", "arm64")
MIN_ARTIFACT_BYTES = 1024 * 1024
TAG = re.compile(r"v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
                 r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?")


def version(tag: str) -> str:
    if not TAG.fullmatch(tag):
        raise ValueError(f"not a release tag (vX.Y.Z or vX.Y.Z-pre): {tag!r}")
    return tag[1:]


def deb_version(tag: str) -> str:
    return version(tag).replace("-", "~", 1)


def deb_name(tag: str, arch: str) -> str:
    if arch not in DEB_ARCHES:
        raise ValueError(f"unsupported Debian architecture {arch!r}; expected one of {', '.join(DEB_ARCHES)}")
    return f"{PACKAGE}_{deb_version(tag)}_{arch}.deb"


def dmg_name(tag: str) -> str:
    return f"Wren-{version(tag)}.dmg"


def stored_name(name: str) -> str:
    """GitHub's rewrite of an uploaded asset's file name."""
    return re.sub(r"[^A-Za-z0-9._-]", ".", name)


def expected_assets(tag: str) -> set[str]:
    return {stored_name(name) for name in (*(deb_name(tag, arch) for arch in DEB_ARCHES), dmg_name(tag))}


def parse_assets(lines: Iterable[str]) -> list[tuple[str, int]]:
    assets = []
    for number, line in enumerate(lines, 1):
        line = line.rstrip("\n")
        if not line.strip():
            continue
        name, separator, size = line.partition("\t")
        if not separator or not size.isdigit():
            raise ValueError(f"line {number} is not `name<TAB>size`: {line!r}")
        assets.append((name, int(size)))
    return assets


def problems(tag: str, assets: list[tuple[str, int]]) -> list[str]:
    expected = expected_assets(tag)
    found: dict[str, list[int]] = {}
    for name, size in assets:
        found.setdefault(name, []).append(size)
    out = []
    for name in sorted(expected):
        sizes = found.get(name, [])
        if len(sizes) != 1:
            out.append(f"expected exactly one asset named {name} on {tag}, found {len(sizes)}.")
        elif sizes[0] < MIN_ARTIFACT_BYTES:
            out.append(f"asset {name} on {tag} is only {sizes[0]} bytes (expected at least "
                       f"{MIN_ARTIFACT_BYTES}); it looks truncated.")
    for name in sorted(set(found) - expected):
        out.append(f"unexpected asset {name} on {tag} (a stale upload from an earlier attempt?).")
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    deb = commands.add_parser("deb-name", help="print the .deb file name nfpm writes for a tag")
    deb.add_argument("tag")
    deb.add_argument("arch", choices=DEB_ARCHES)
    commands.add_parser("dmg-name", help="print the DMG file name for a tag").add_argument("tag")
    commands.add_parser("check", help="check a draft's `name<TAB>size` asset lines on stdin").add_argument("tag")
    args = parser.parse_args(argv)
    try:
        if args.command == "deb-name":
            print(deb_name(args.tag, args.arch))
            return 0
        if args.command == "dmg-name":
            print(dmg_name(args.tag))
            return 0
        found = problems(args.tag, parse_assets(sys.stdin))
    except ValueError as error:
        print(f"::error::{error}")
        return 1
    for problem in found:
        print(f"::error::{problem}")
    if found:
        return 1
    print(f"{args.tag}: all {len(expected_assets(args.tag))} expected assets present and sized.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
