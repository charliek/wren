"""Regenerate every Wren app-icon asset from assets/icon/wren-icon.svg.

The SVG is the single source: the owl mark shared with the sibling Roost app,
with each color set once, as the fill of the element named for its role
(background, owl-body, eyes, eye-glints). Run through `make icons`, which
supplies cairosvg and Pillow in a throwaway uv environment:

    make icons                                         # re-render after editing the SVG
    make icons ICON_COLORS="background=#123456 eyes=#FFAA00"   # recolor, then re-render

Derived outputs, all committed so that builds and CI never need this toolchain:

    assets/icon/hicolor/<N>x<N>/apps/wren.png  Linux hicolor sizes (16 to 512); the
                                               256 px image is also the runtime
                                               window icon embedded by wren-iced
    assets/icon/AppIcon.icns                   macOS flat icon on Apple's 824/1024
                                               grid (pre-Tahoe, or no actool)
    assets/icon/AppIcon.icon/                  macOS 26 Icon Composer source (solid
                                               background fill plus the owl layer);
                                               tools/package.py compiles it with
                                               actool when full Xcode is present

The Linux scalable icon is the source SVG itself, copied by tools/package.py.

Default palette ("lagoon owl"): Deep Lagoon #0E5159 tile, Parchment #FBF6EC
owl, Wren Amber #F0B45A eyes (the dark theme's accent), white #FFFFFF glints.
Teal and amber sit opposite each other on the color wheel, so the eyes carry
the app's accent at full strength against a tile that is distinct from both
Roost's violet and Wren's own near-black UI.
"""

from __future__ import annotations

import argparse
import io
import json
from pathlib import Path
import re
import shutil
import struct
import sys
import xml.etree.ElementTree as ET

import cairosvg
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "assets/icon"
SOURCE = ICON_DIR / "wren-icon.svg"
HICOLOR = ICON_DIR / "hicolor"
ICNS = ICON_DIR / "AppIcon.icns"
ICON_COMPOSER = ICON_DIR / "AppIcon.icon"

ROLES = ("background", "owl-body", "eyes", "eye-glints")
HICOLOR_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
# PNG-payload ICNS element types and their pixel sizes (@2x types included).
ICNS_TYPES = (
    (b"icp4", 16), (b"icp5", 32), (b"ic11", 32), (b"ic12", 64), (b"ic07", 128),
    (b"ic13", 256), (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024),
)
# Apple's macOS icon grid: the tile is 824 of 1024 units, centered, leaving room
# the Dock expects. Linux hicolor icons stay full-bleed, as Roost's do.
MAC_TILE = 824 / 1024
HEX = re.compile(r"#[0-9A-Fa-f]{6}")


def fill_pattern(role: str) -> re.Pattern[str]:
    return re.compile(r'(<[a-z]+\s+id="' + re.escape(role) + r'"[^>]*?\sfill=")(#[0-9A-Fa-f]{6})(")')


def palette(svg: str) -> dict[str, str]:
    ET.fromstring(svg)  # well-formed XML before anything is rendered
    colors = {}
    for role in ROLES:
        matches = fill_pattern(role).findall(svg)
        if len(matches) != 1:
            raise SystemExit(f"error: {SOURCE.name} needs exactly one id=\"{role}\" element with a #RRGGBB fill")
        colors[role] = matches[0][1]
    return colors


def recolor(svg: str, assignments: list[str]) -> str:
    for assignment in assignments:
        role, _, color = assignment.partition("=")
        if role not in ROLES or not HEX.fullmatch(color):
            raise SystemExit(f"error: expected ROLE=#RRGGBB with ROLE in {', '.join(ROLES)}; got {assignment!r}")
        svg = fill_pattern(role).sub(lambda match: match[1] + color.upper() + match[3], svg)
    return svg


def render(svg: str, size: int) -> Image.Image:
    png = cairosvg.svg2png(bytestring=svg.encode("utf-8"), output_width=size, output_height=size)
    return Image.open(io.BytesIO(png)).convert("RGBA")


def png_bytes(image: Image.Image) -> bytes:
    output = io.BytesIO()
    image.save(output, "PNG", optimize=True)
    return output.getvalue()


def mac_tile(svg: str, size: int) -> Image.Image:
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    tile = render(svg, max(1, round(size * MAC_TILE)))
    offset = (size - tile.width) // 2
    canvas.paste(tile, (offset, offset), tile)
    return canvas


def write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    print(f"  wrote {path.relative_to(ROOT)}")


def write_icns(svg: str) -> None:
    rendered: dict[int, bytes] = {}
    chunks = []
    for kind, size in ICNS_TYPES:
        if size not in rendered:
            rendered[size] = png_bytes(mac_tile(svg, size))
        chunks.append(kind + struct.pack(">I", len(rendered[size]) + 8) + rendered[size])
    data = b"".join(chunks)
    write(ICNS, b"icns" + struct.pack(">I", len(data) + 8) + data)


def write_icon_composer(svg: str, background: str) -> None:
    # Hand-authored like Roost's: a solid fill paints the system tile and the
    # owl rides on a transparent 1024 canvas, so macOS 26 fills its glass tile
    # edge to edge instead of insetting a legacy icon.
    if ICON_COMPOSER.exists():
        shutil.rmtree(ICON_COMPOSER)
    owl = fill_pattern("background").sub(lambda match: match[1] + "none" + match[3], svg)
    write(ICON_COMPOSER / "Assets/owl.png", png_bytes(render(owl, 1024)))
    red, green, blue = (int(background[i:i + 2], 16) / 255 for i in (1, 3, 5))
    icon = {
        "fill": {"solid": f"srgb:{red:.5f},{green:.5f},{blue:.5f},1.00000"},
        "groups": [{
            "layers": [{"image-name": "owl.png", "name": "owl"}],
            "shadow": {"kind": "neutral", "opacity": 0.5},
            "translucency": {"enabled": True, "value": 0.5},
        }],
        "supported-platforms": {"circles": ["watchOS"], "squares": "shared"},
    }
    write(ICON_COMPOSER / "icon.json", (json.dumps(icon, indent=2) + "\n").encode("utf-8"))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("colors", nargs="*", metavar="ROLE=#RRGGBB",
                        help=f"rewrite a fill in the source SVG first; ROLE is one of {', '.join(ROLES)}")
    parser.add_argument("--preview", type=Path, help="also write a 512 px and 32 px side-by-side preview PNG here")
    args = parser.parse_args(argv)

    source = SOURCE.read_text(encoding="utf-8")
    svg = recolor(source, args.colors)
    colors = palette(svg)
    if svg != source:
        write(SOURCE, svg.encode("utf-8"))
    print("Generating Wren icons: " + ", ".join(f"{role}={color}" for role, color in colors.items()))

    if HICOLOR.exists():
        shutil.rmtree(HICOLOR)
    for size in HICOLOR_SIZES:
        write(HICOLOR / f"{size}x{size}/apps/wren.png", png_bytes(render(svg, size)))
    write_icns(svg)
    write_icon_composer(svg, colors["background"])

    if args.preview:
        large, small = render(svg, 512), render(svg, 32)
        sheet = Image.new("RGBA", (512 + 32 * 3 + 64, 512 + 32), (128, 128, 128, 255))
        sheet.paste(large, (16, 16), large)
        sheet.paste(small, (512 + 32, 16), small)
        zoomed = small.resize((96, 96), Image.Resampling.NEAREST)
        sheet.paste(zoomed, (512 + 32, 64), zoomed)
        args.preview.parent.mkdir(parents=True, exist_ok=True)
        sheet.save(args.preview)
        print(f"  wrote preview {args.preview}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
