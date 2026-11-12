#!/usr/bin/env python3
"""Writes the board fixtures with the second implementation in board_format.py.

The sample workspace (discovery/boards.md section 6.1) serves the harness, the scenes and hand
trials; the golden files (section 4.16) are the bytes the Rust writer must reproduce. Each
`<name>.in.wren-board` must write to exactly `<name>.out.wren-board`; a `.out` with no `.in`
must write to itself.

  python3 tools/board_fixtures.py write    rewrite every fixture
  python3 tools/board_fixtures.py check    exit 1 if a committed fixture differs from this script
"""
from pathlib import Path
import random
import sys

if __package__:
    from . import board_format as bf
else:
    import board_format as bf

ROOT = Path(__file__).resolve().parents[1]
SAMPLES = ROOT / "tools/wrentest/fixtures/boards/workspace/Boards/Samples"
GOLDEN = ROOT / "crates/wren-board/golden/format"
EXAMPLE = GOLDEN / "password-reset.out.wren-board"
CROCKFORD = "0123456789abcdefghjkmnpqrstvwxyz"
W, H = 120, 60


def board(nodes, edges=None, **top):
    return bf.dump({"schema": 1, **top, "nodes": nodes, "edges": edges or {}})


def stack(keys):
    """The next z key in creation order, on top of the last (section 4.7)."""
    keys.append(bf.key_between(keys[-1] if keys else None, None))
    return keys[-1]


# ------------------------------------------------------------------ the sample workspace
def checkout_flow():
    """The 15-node flowchart that the acceptance script (section 5.20) builds by keyboard.

    Positions are the keyboard model's output in cells, times 20, with Start at the origin.
    Elements are stacked in the order the script creates them, and ids are random, as the
    editor draws them.
    """
    rng = random.Random(515)
    used = set()

    def new_id():
        while True:
            i = "".join(rng.choice(CROCKFORD) for _ in range(8))
            if i not in used:
                used.add(i)
                return i

    layout = {
        "Start": ("pill", 0, 0),
        "Receive order": ("rect", 0, 6),
        "Valid?": ("diamond", 0, 12),
        "Reject order": ("rect", 10, 12),
        "Check stock": ("rect", 0, 18),
        "In stock?": ("diamond", 0, 24),
        "Backorder": ("rect", 10, 24),
        "Notify customer": ("rect", 10, 30),
        "Charge card": ("rect", 0, 30),
        "Payment ok?": ("diamond", 0, 36),
        "Retry payment": ("rect", 10, 36),
        "Pack": ("rect", 0, 42),
        "Ship": ("rect", 0, 48),
        "Send receipt": ("rect", 0, 54),
        "End": ("pill", 0, 60),
    }
    # (node created, or None) then (connector from, to, label), in the script's order
    script = [
        ("Start", None),
        ("Receive order", ("Start", "Receive order", "")),
        ("Valid?", ("Receive order", "Valid?", "")),
        ("Reject order", ("Valid?", "Reject order", "No")),
        ("Check stock", ("Valid?", "Check stock", "Yes")),
        ("In stock?", ("Check stock", "In stock?", "")),
        ("Backorder", ("In stock?", "Backorder", "No")),
        ("Notify customer", ("Backorder", "Notify customer", "")),
        (None, ("Notify customer", "Check stock", "")),
        ("Charge card", ("In stock?", "Charge card", "Yes")),
        ("Payment ok?", ("Charge card", "Payment ok?", "")),
        ("Retry payment", ("Payment ok?", "Retry payment", "No")),
        (None, ("Retry payment", "Charge card", "")),
        ("Pack", ("Payment ok?", "Pack", "Yes")),
        ("Ship", ("Pack", "Ship", "")),
        ("Send receipt", ("Ship", "Send receipt", "")),
        ("End", ("Send receipt", "End", "")),
        (None, ("Reject order", "End", "")),
    ]
    keys, ids, nodes, edges = [], {}, {}, {}
    for label, edge in script:
        if label is not None:
            shape, x, y = layout[label]
            ids[label] = new_id()
            nodes[ids[label]] = dict(text=label, kind="shape", at=[x * 20, y * 20], shape=shape,
                                     size=[W, H], z=stack(keys))
        if edge is not None:
            a, b, text = edge
            rec = {"kind": "edge", "from": {"node": ids[a]}, "to": {"node": ids[b]}, "z": stack(keys)}
            if text:
                rec["labels"] = [{"text": text}]
            edges[new_id()] = rec
    assert len(nodes) == 15 and len(edges) == 17
    return board(nodes, edges)


def shapes():
    """Every shape, free text, an unknown kind holding a child, an unknown shape, and a
    connector whose end names no node."""
    keys = []

    def shape(text, shape, x, y, w=W, h=H, **kw):
        return dict(text=text, kind="shape", at=[x, y], shape=shape, size=[w, h], z=stack(keys), **kw)

    nodes = {
        "rect": shape("Rectangle\nמלבן", "rect", 0, 0),
        "rounded": shape("Rounded", "rounded", 160, 0, style={"color": "blue"}),
        "pill": shape("Pill", "pill", 320, 0, style={"color": "green"}),
        "ellipse": shape("Ellipse", "ellipse", 480, 0, style={"color": "purple"}),
        "diamond": shape("Diamond", "diamond", 0, 120, 140, 80, style={"color": "yellow"}),
        "parallelogram": shape("Parallelogram", "parallelogram", 180, 120, 160, 60,
                               style={"color": "orange"}),
        "cylinder": shape("Cylinder", "cylinder", 380, 120, 120, 80, style={"color": "cyan"}),
        "document": shape("Document", "document", 540, 120, 120, 80,
                          style={"color": "red", "fill": "solid"}),
        "text": dict(text="Free text has no outline\nand sits top left.", kind="text", at=[0, 260],
                     size=[240, 40], z=stack(keys)),
        "hexagon": shape("Unknown shape", "hexagon", 280, 260, 160, 60),
        "frame": dict(text="Frame, a kind this build does not know", kind="frame", at=[0, 360],
                      size=[400, 180], z=stack(keys)),
        "child": dict(text="Child of the frame", kind="shape", at=[40, 80], parent="frame",
                      size=[160, 60], z=stack(keys)),
    }
    edges = {
        "rect-rounded": {"kind": "edge", "from": {"node": "rect"}, "to": {"node": "rounded"},
                         "route": "straight", "z": stack(keys)},
        "pill-ellipse": {"kind": "edge", "from": {"node": "pill"}, "to": {"node": "ellipse"},
                         "style": {"stroke": "dashed"}, "z": stack(keys)},
        "child-hexagon": {"kind": "edge", "from": {"node": "child"}, "to": {"node": "hexagon"},
                          "labels": [{"text": "elbow"}], "z": stack(keys)},
        "dangling": {"kind": "edge", "from": {"node": "document"}, "to": {"node": "gone"},
                     "z": stack(keys)},
    }
    return board(nodes, edges)


def overlap():
    """Three solid shapes, each covering part of the label of the one below."""
    def shape(text, x, y, color, z):
        return dict(text=text, kind="shape", at=[x, y], size=[240, 80],
                    style={"color": color, "fill": "solid"}, z=z)

    # The ids sort in another order than the stacking keys, so drawing must follow z.
    return board({
        "back": shape("The back shape's label is covered", 0, 0, "blue", "V01"),
        "middle": shape("The middle shape's label is covered", 80, 40, "green", "V02"),
        "front": shape("The front shape is drawn last", 160, 80, "red", "V03"),
    })


def future():
    """A board from a newer build: a feature this build lacks makes it read-only."""
    return board({
        "plan": dict(text="Release plan", kind="frame", at=[0, 0], size=[440, 200], z="V01"),
        "design": dict(text="Design", kind="shape", at=[40, 80], parent="plan", size=[W, H],
                       locked=True, z="V02"),
        "build": dict(text="Build", kind="shape", at=[280, 80], parent="plan", size=[W, H],
                      markup="md", rot=0, z="V03"),
        "notes": dict(text="Notes", kind="note", at=[520, 40], size=[200, 120], file="Notes.md",
                      subpath="#Boards", link="https://example.com/release", z="V04"),
    }, {
        "design-build": {"kind": "edge", "from": {"node": "design"}, "to": {"node": "build"},
                         "points": [[200, 140], [260, 140]], "z": "V05"},
    }, requires=["frames"], layout={"direction": "right"})


HAND_WRITTEN = '''\
# A board written by hand, the way a person or a script might write one.
# Wren keeps these bytes until the first edit, which rewrites the file in
# canonical form.

schema=1

[edge.flow]
kind = "edge"
to   = { node = "second" }
from = { node = "first" }
labels = [ { text = "then" } ]
z = "c"

[node.second]
kind="shape"
text = "Second step"
at = [ 200, 0 ]
size = [ 120, 60 ]
shape = 'rounded'
z = "b"

[node.first]
text = """First step"""
kind = "shape"
size = [120,60]
at = [0,0]
z = "a"

    [node.first.style]
    color = "green"   # one of the eight colour tokens

[node.caption]
kind = "text"
at = [0, 100]
size = [320, 20]
text = "Comments and spacing are not kept."
z = "d"
'''

NOTES = '''\
# Notes

The boards beside this note are fixtures for Wren's board tests and scenes.
Tests copy this folder before opening it, so nothing here changes.
'''


def samples():
    return {
        "Checkout flow.wren-board": checkout_flow(),
        "Shapes.wren-board": shapes(),
        "Overlap.wren-board": overlap(),
        "Future.wren-board": future(),
        "Hand written.wren-board": HAND_WRITTEN,
        "Notes.md": NOTES,
    }


# ------------------------------------------------------------------ golden files
def fields():
    """Every node and edge field at a value other than its default."""
    return board({
        "base": dict(kind="shape", at=[0, 0], size=[W, H], z="V01"),
        "full": dict(text="Every field", kind="shape", at=[20, -40], parent="base", shape="diamond",
                     size=[140, 80], style={"color": "red", "fill": "solid", "stroke": "dashed"},
                     z="V02", ext={"tool": {"note": "kept"}}),
        "words": dict(text="Free text", kind="text", at=[200, 0], size=[W, 20],
                      style={"color": "#3a7bd5", "fill": "none"}, z="V03"),
    }, {
        "full": {"kind": "edge", "from": {"node": "base", "side": "right"},
                 "to": {"node": "words", "anchor": [0, 500]}, "heads": ["arrow", "none"],
                 "labels": [{"text": "first", "at": 250}, {"text": "second"}], "route": "straight",
                 "style": {"color": "blue", "stroke": "dashed"}, "z": "V04",
                 "ext": {"tool": {"weight": 2}}},
    })


def shapes_and_ends():
    """The eight shapes and every endpoint form."""
    names = ["rect", "rounded", "pill", "ellipse", "diamond", "parallelogram", "cylinder", "document"]
    nodes = {n: dict(text=n.capitalize(), kind="shape", at=[(i % 4) * 160, (i // 4) * 120], shape=n,
                     size=[W, H], z="V%02d" % (i + 1)) for i, n in enumerate(names)}
    ends = [
        ({"node": "rect"}, {"node": "rounded"}),
        ({"node": "rounded", "side": "top"}, {"node": "pill", "side": "bottom"}),
        ({"node": "pill", "side": "right"}, {"node": "ellipse", "side": "left"}),
        ({"node": "ellipse", "anchor": [500, 1000]}, {"node": "document", "anchor": [0, 0]}),
        ({"node": "diamond", "anchor": [1000, 500], "side": "left"}, {"at": [-80, 300]}),
        ({"at": [0, 300]}, {"node": "cylinder"}),
        ({"at": [-40, -40]}, {"at": [-40, 400]}),
    ]
    edges = {"e%d" % i: {"kind": "edge", "from": a, "to": b, "z": "V%02d" % (10 + i)}
             for i, (a, b) in enumerate(ends)}
    return board(nodes, edges)


def escapes():
    """Every escape the writer uses, and characters written raw (section 4.9). A label
    holds no CR, which the reader turns into LF; unknown content keeps it."""
    controls = "".join(chr(c) for c in range(0x20) if c != 0x0D) + "\x7f"
    return board({
        "escapes": dict(text='back\\slash "quoted"\ttab\nnew line', kind="shape", at=[0, 0],
                        size=[W, H], z="V01"),
        "controls": dict(text=controls, kind="shape", at=[160, 0], size=[W, H], z="V02"),
        "raw": dict(text="é, e\u0301, 漢字, عربى, 🚀, \u200b and \u2028 stay raw", kind="shape",
                    at=[320, 0], size=[W, H], z="V03", ext={"raw": {"cr": "a\rb", "crlf": "a\r\nb"}}),
    }, {
        "label": {"kind": "edge", "from": {"node": "escapes"}, "to": {"node": "controls"},
                  "labels": [{"text": "two\nlines \\ \"q\""}], "z": "V04"},
    })


def unknown():
    """Unknown kinds, values, fields, ext and top-level keys, all kept (section 4.10)."""
    return board({
        "a": dict(text="Unknown values", kind="shape", at=[0, 0], shape="hexagon", size=[W, H],
                  style={"color": "teal", "fill": "hatched", "stroke": "dotted", "glow": True},
                  z="V01", ext={"drawio": {"style": "rounded=1;whiteSpace=wrap;"}},
                  extra_field="kept", zz=[1, 2]),
        "b": dict(text="Unknown kind", kind="sticky", at=[200, 0], size=[W, H], z="V02",
                  colour="yellow"),
    }, {
        "e": {"kind": "link", "from": {"node": "a", "side": "middle", "port": "p1"},
              "to": {"node": "b"}, "heads": ["diamond", "circle"],
              "labels": [{"text": "styled", "font": "mono"}], "route": "curved",
              "style": {"color": "magenta", "stroke": "wavy", "fill": "tint"}, "z": "V03",
              "weight": 3},
    }, layout={"direction": "down"}, meta={"author": "a script", "version": 2}, zz_last=True)


def quoted_keys():
    """Keys that are not bare are written as basic strings, wherever they are."""
    return board({
        "a": dict(text="Quoted keys", kind="shape", at=[0, 0], size=[W, H], z="V01",
                  ext={"my tool": {"a.b": 1, "": 2, "café": 3, 'say "hi"': 4}}),
    }, {
        "e": {"kind": "edge", "from": {"node": "a", "my port": "west"}, "to": {"at": [200, 0]},
              "z": "V02"},
    }, **{"top level": 1, "x.y": {"z": True}})


def values():
    """Floats, nan, dates, times and nested tables and arrays in unknown content."""
    import datetime as dt
    floats = [1.0, 0.25, -0.0, 0.1, 1e15, 1e16, 1e21, 1e-5, 1.5e-7, 1.7976931348623157e308, 5e-324,
              float("nan"), float("inf"), float("-inf")]
    minus8 = dt.timezone(dt.timedelta(hours=-8))
    return board({
        "a": dict(text="Values", kind="shape", at=[0, 0], size=[W, H], z="V01", ext={"values": {
            "floats": floats,
            "one": 1.0,
            "ints": [0, -5, 9223372036854775807, -9223372036854775808],
            "bools": [True, False],
            "offset": dt.datetime(1979, 5, 27, 7, 32, tzinfo=minus8),
            "utc": dt.datetime(2026, 11, 12, 10, 0, tzinfo=dt.timezone.utc),
            "local": dt.datetime(1979, 5, 27, 7, 32, 0, 500000),
            "date": dt.date(1979, 5, 27),
            "time": dt.time(7, 32, 0, 250000),
            "nested": {"deeper": {"deepest": "x"}, "empty": {}},
            "arrays": [[1, 2], [3], [], ["a", 0.5, {"k": "v"}]],
            "tables": [{"name": "first", "at": [1, 2]}, {}],
        }}),
    })


def requires():
    return board({"a": dict(text="Needs features", kind="shape", at=[0, 0], size=[W, H], z="V01")},
                 requires=["frames", "zz-feature"])


def future_keys():
    """A file from a newer build: the ring-1 fields of section 4.5, an unknown kind, unknown
    enum values, and unknown keys before, between and after the known ones at every level."""
    return board({
        "frame": dict(text="A frame", kind="frame", at=[0, 0], size=[400, 240], z="V01",
                      locked=True, link="https://example.com", a_first=1, mid=2, zz_last=3,
                      ext={"tool": {"k": "v"}}),
        "card": dict(text="A note card", kind="note", at=[40, 60], parent="frame", size=[160, 80],
                     file="Notes.md", subpath="#Heading", markup="md", rot=15, z="V02"),
        "image": dict(kind="image", at=[220, 60], parent="frame", size=[140, 120],
                      src="assets/0123abcd.png", z="V03"),
        "shape": dict(text="Known kind", kind="shape", at=[0, 300], shape="star", size=[W, H],
                      style={"a": 1, "color": "red", "dash": 2, "fill": "gradient", "q": 3,
                             "stroke": "dotted", "zz": 4},
                      aa=0, q=1, zz=2, z="V04"),
    }, {
        "e": {"kind": "edge", "from": {"node": "shape", "a": 1, "port": "x", "zz": 2},
              "to": {"node": "card", "anchor": [500, 0], "side": "centre", "b": 3},
              "heads": ["arrow", "open"], "labels": [{"a": 0, "at": 100, "font": "x", "text": "t", "zz": 1}],
              "route": "curved", "points": [[100, 280], [120, 200]], "locked": False,
              "aa": 1, "m": 2, "zz": 3, "style": {"a": 1, "color": "blue", "s": 2, "stroke": "dashed",
                                                  "zz": 3}, "z": "V05"},
    }, requires=["frames"], aaa=1, layout={"direction": "right"}, zzz={"k": "v"})


# Non-canonical inputs (section 4.16 test 2). Each is (input, the same board in TOML 1.0
# without a BOM when tomllib cannot read the input, or None).
CANONICAL_ISH = '''\
schema = 1

[node.a]
text = "First"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "V01"

[node.b]
text = "Second"
kind = "shape"
at = [0, 120]
size = [120, 60]
z = "V02"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
z = "V03"
'''

INPUTS = {
    "empty": ("", None),
    "blank": ("\n   \n\t\n", None),
    "comments-only": ("# nothing but a comment\n\n# and another\n", None),
    "crlf": (CANONICAL_ISH.replace('text = "First"', 'text = """one\ntwo"""').replace("\n", "\r\n"),
             None),
    "bom": ("\ufeff" + CANONICAL_ISH, CANONICAL_ISH),
    "comments": ('''\
# A comment before everything
schema = 1 # trailing comment

# a block comment
[node.a] # after a header
text = "First" # after a value
kind = "shape"
at = [0, 0]   # spaces, then a comment
size = [
  120, # inside an array
  60,
]
z = "V01"
''', None),
    "shuffled": ('''\
requires = ["zeta", "alpha"]
zz_top = "unknown, top level"
schema = 1
a_top = "unknown, top level"
[edge.e]
z = "V03"
to = { node = "a" }
kind = "edge"
from = { node = "b" }
[node.b]
z = "V02"
size = [120, 60]
at = [0, 120]
kind = "shape"
text = "B"
[node.a]
size = [120, 60]
kind = "shape"
text = "A"
at = [0, 0]
z = "V01"
style.fill = "solid"
style.color = "red"
''', None),
    "subtables": ('''\
schema = 1

[node.a]
text = "A"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "V01"
style = { color = "blue" }

[node.a.ext.tool]
k = "v"

[node.b]
text = "B"
kind = "shape"
at = [0, 120]
size = [120, 60]
z = "V02"

[node.b.style]
color = "green"
fill = "none"

[edge.e]
kind = "edge"
z = "V03"

[edge.e.from]
node = "a"
side = "bottom"

[edge.e.to]
node = "b"

[[edge.e.labels]]
text = "first"
at = 250

[[edge.e.labels]]
text = "second"
''', None),
    "strings": ('''\
schema = 1

[node.a]
text = """
Multi-line \\
basic, with a line \\
   continuation"""
kind = 'shape'
at = [0, 0]
size = [120, 60]
z = 'V01'
ext.paths.windows = 'C:\\Users\\wren'
ext.paths.raw = \'\'\'
two
lines\'\'\'

[node.b]
text = "Escapes: \\u00E9 \\U0001F680, CRLF\\r\\nand a lone CR\\rbecome line feeds"
kind = "shape"
at = [0, 120]
size = [120, 60]
z = "V02"
ext.kept.cr = "a\\rb"
''', None),
    "toml11": ('''\
schema = 1

[node.a]
text = "Escape \\e[0m and \\x41\\xe9"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "V01"

[node.b]
text = "B"
kind = "shape"
at = [0, 120]
size = [120, 60]
z = "V02"

[edge.e]
kind = "edge"
from = {
  node = "a",
  side = "bottom",
}
to = { node = "b", }
z = "V03"
''', '''\
schema = 1

[node.a]
text = "Escape \\u001B[0m and A\\u00E9"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "V01"

[node.b]
text = "B"
kind = "shape"
at = [0, 120]
size = [120, 60]
z = "V02"

[edge.e]
kind = "edge"
from = { node = "a", side = "bottom" }
to = { node = "b" }
z = "V03"
'''),
    "defaults": ('''\
schema = 1
requires = []

[node.a]
text = ""
kind = "shape"
at = [0, 0]
shape = "rect"
size = [120, 60]
style.color = "neutral"
style.fill = "tint"
style.stroke = "solid"
z = "V01"

[node.b]
text = "B"
kind = "shape"
at = [0, 120]
size = [120, 60]
style = {}
z = "V02"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
heads = ["none", "arrow"]
labels = [
  { at = 500, text = "middle" },
]
route = "elbow"
style.color = "neutral"
style.stroke = "solid"
z = "V03"

[edge.f]
kind = "edge"
from = { node = "b" }
to = { node = "a" }
labels = []
z = "V04"
''', None),
    "colour-case": ('''\
schema = 1

[node.a]
text = "Upper-case hex"
kind = "shape"
at = [0, 0]
size = [120, 60]
style.color = "#3A7BD5"
z = "V01"

[node.b]
text = "Not a colour"
kind = "shape"
at = [0, 120]
size = [120, 60]
style.color = "#ABC"
z = "V02"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
style.color = "#FF00Aa"
z = "V03"
''', None),
}


def golden():
    files = {
        "fields.out.wren-board": fields(),
        "shapes-and-ends.out.wren-board": shapes_and_ends(),
        "escapes.out.wren-board": escapes(),
        "unknown.out.wren-board": unknown(),
        "quoted-keys.out.wren-board": quoted_keys(),
        "values.out.wren-board": values(),
        "requires.out.wren-board": requires(),
        "future.out.wren-board": future_keys(),
    }
    for name, (source, readable) in INPUTS.items():
        files[name + ".in.wren-board"] = source
        files[name + ".out.wren-board"] = bf.dump(bf.load(source if readable is None else readable))
    return files


def fixtures():
    out = {SAMPLES / name: text for name, text in samples().items()}
    out.update({GOLDEN / name: text for name, text in golden().items()})
    return out


def stale(files):
    """The fixtures whose committed bytes differ from `files`."""
    return [p for p, text in files.items() if not p.exists() or p.read_bytes() != text.encode()]


def committed(files):
    """`files` plus the example, which was written once from a model and is checked, not
    rewritten."""
    return {**files, EXAMPLE: EXAMPLE.read_bytes().decode()}


def problems(files):
    """Fixtures that break their own rules: an output that does not write to itself, or an
    input that is already canonical. A board that does not load raises."""
    found = []
    for path, text in files.items():
        if path.name.endswith(".out.wren-board") or (path.parent == SAMPLES and path.suffix == ".wren-board"
                                                       and path.name != "Hand written.wren-board"):
            if bf.dump(bf.load(text)) != text:
                found.append("%s is not canonical" % path.name)
        elif path.name.endswith(".wren-board"):
            readable = INPUTS.get(path.name.removesuffix(".in.wren-board"), (None, None))[1]
            if bf.dump(bf.load(readable or text)) == text:
                found.append("%s is already canonical" % path.name)
    return found


def main(argv):
    wanted = fixtures()
    differ = stale(wanted)
    broken = problems(committed(wanted))
    for problem in broken:
        print(problem)
    if argv[1:] == ["write"]:
        for path in differ:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(wanted[path].encode())
        print("wrote %d of %d fixtures" % (len(differ), len(wanted)))
        return 1 if broken else 0
    if argv[1:] == ["check"]:
        for path in differ:
            print("differs: %s" % path.relative_to(ROOT))
        return 1 if differ or broken else 0
    print(__doc__)
    return 64


if __name__ == "__main__":
    sys.exit(main(sys.argv))
