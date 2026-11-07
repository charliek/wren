"""Add a release to Wren's Sparkle appcast.

    python3 tools/update_appcast.py --tag v0.1.0 --sign-file sign.txt

`release.yml`'s `appcast` job runs this after the release is public, then the
release bot commits the result to main. The appcast lists every stable
release; Sparkle offers the newest one whose `sparkle:version` is above the
installed app's CFBundleVersion.

`--sign-file` holds the line Sparkle's `sign_update` printed for the DMG:

    sparkle:edSignature="<base64>" length="<bytes>"

Prerelease tags are refused. A prerelease bundle records only its X.Y.Z as
CFBundleVersion (tools/package.py), so it would compare equal to its final
release, and stable users must never be offered one anyway.

An existing item for the same version is replaced, keeping its pubDate, so
rerunning a release job leaves the file unchanged. Standard library only, like
release_assets.py.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
from pathlib import Path
import re
import sys
import xml.etree.ElementTree as ET

try:
    import release_assets
except ModuleNotFoundError:  # imported as tools.update_appcast (the unit tests)
    from tools import release_assets

SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
# RFC 822, the format Sparkle reads in <pubDate>.
PUBDATE_FORMAT = "%a, %d %b %Y %H:%M:%S %z"
DEFAULT_APPCAST = Path(__file__).resolve().parents[1] / "packaging/macos/appcast.xml"
SIGNATURE = re.compile(r'sparkle:edSignature="([A-Za-z0-9+/]+={0,2})"')
LENGTH = re.compile(r'length="([1-9][0-9]*)"')


def sparkle(name: str) -> str:
    return f"{{{SPARKLE_NS}}}{name}"


def parse_signature(text: str) -> tuple[str, str]:
    """The (edSignature, length) pair from sign_update's output."""
    signature, length = SIGNATURE.search(text), LENGTH.search(text)
    if signature is None or length is None:
        raise ValueError("sign_update output has no sparkle:edSignature and length")
    return signature.group(1), length.group(1)


def update(appcast: str, tag: str, signature: str, length: str, repo: str, now: datetime) -> str:
    """Return `appcast` with the release for `tag` added or replaced."""
    version = release_assets.version(tag)
    if "-" in version:
        raise ValueError(f"{tag} is a prerelease; only stable releases go in the appcast")
    ET.register_namespace("sparkle", SPARKLE_NS)
    root = ET.fromstring(appcast)
    channel = root.find("channel")
    if channel is None:
        raise ValueError("the appcast has no <channel>")

    pubdate = now.strftime(PUBDATE_FORMAT)
    for item in channel.findall("item"):
        if item.findtext(sparkle("version")) == version:
            pubdate = item.findtext("pubDate") or pubdate
            channel.remove(item)

    item = ET.SubElement(channel, "item")
    ET.SubElement(item, "title").text = f"Wren {version}"
    ET.SubElement(item, "pubDate").text = pubdate
    ET.SubElement(item, sparkle("version")).text = version
    ET.SubElement(item, sparkle("shortVersionString")).text = version
    enclosure = ET.SubElement(item, "enclosure")
    enclosure.set("url", f"https://github.com/{repo}/releases/download/{tag}/{release_assets.dmg_name(tag)}")
    enclosure.set("type", "application/octet-stream")
    enclosure.set(sparkle("edSignature"), signature)
    enclosure.set("length", length)

    ET.indent(root, space="  ")
    return "<?xml version='1.0' encoding='utf-8'?>\n" + ET.tostring(root, encoding="unicode") + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tag", required=True, help="the release tag, vX.Y.Z")
    parser.add_argument("--sign-file", type=Path, required=True, help="sign_update's output for the DMG")
    parser.add_argument("--appcast", type=Path, default=DEFAULT_APPCAST)
    parser.add_argument("--repo", default="charliek/wren", help="owner/name the DMG is released from")
    args = parser.parse_args(argv)
    try:
        signature, length = parse_signature(args.sign_file.read_text(encoding="utf-8"))
        updated = update(args.appcast.read_text(encoding="utf-8"), args.tag, signature, length, args.repo,
                         datetime.now(timezone.utc))
    except (OSError, ValueError, ET.ParseError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    args.appcast.write_text(updated, encoding="utf-8")
    print(f"appcast: {args.tag} ({length} bytes) -> {args.appcast}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
