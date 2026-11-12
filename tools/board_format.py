#!/usr/bin/env python3
"""A second implementation of the schema-1 board format (discovery/boards.md section 4).

The board fixtures and the golden outputs of crates/wren-board/golden/format/ are written
with it, so the Rust writer must reproduce an independent implementation byte for byte. It
also holds the z-key functions and the self-checks behind the claims in that section.

Where Python's tomllib reads differently from the toml crate, a fixture avoids the case:
tomllib rejects a BOM and TOML 1.1 syntax, turns CRLF inside a multi-line string into LF,
and reads an offset of +00:00 as Z.

  python3 tools/board_format.py check FILE    exit 0 if FILE is canonical, 2 if valid but not
  python3 tools/board_format.py fmt FILE      print the canonical form
  python3 tools/board_format.py selftest crates/wren-board/golden/format/password-reset.out.wren-board
                                              run the property checks (z keys, strings, example)
  python3 tools/board_format.py fuzzfile DIR  write strings.toml and strings.hex for a Rust check
"""
import copy
import datetime
import math
import random
import re
import sys
import tomllib

ALPHABET = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"   # ASCII order
BARE = re.compile(r"^[A-Za-z0-9_-]+$")
ID = re.compile(r"^[A-Za-z0-9_-]{1,32}$")
ZKEY = re.compile(r"^[0-9A-Za-z]{1,64}$")
HEX = re.compile(r"^#[0-9A-Fa-f]{6}$")
NODE_FIRST = ["text", "kind"]
EDGE_FIRST = ["kind", "from", "to"]
Z_MAX = 64
# Known optional fields omitted when equal to their default (section 4.11 rule 9).
NODE_DEFAULTS = {"text": "", "shape": "rect"}
EDGE_DEFAULTS = {"heads": ["none", "arrow"], "labels": [], "route": "elbow"}
NODE_STYLE_DEFAULTS = {"color": "neutral", "fill": "tint", "stroke": "solid"}
EDGE_STYLE_DEFAULTS = {"color": "neutral", "stroke": "solid"}   # a connector has no fill
LABEL_AT = 500


# ------------------------------------------------------------------ canonical writer
def key(k):
    return k if BARE.match(k) else string(k)


def string(s):
    out = []
    for c in s:
        o = ord(c)
        if c == "\\":
            out.append("\\\\")
        elif c == '"':
            out.append('\\"')
        elif c == "\n":
            out.append("\\n")
        elif c == "\t":
            out.append("\\t")
        elif c == "\r":
            out.append("\\r")
        elif o < 0x20 or o == 0x7F:
            out.append("\\u%04X" % o)
        else:
            out.append(c)
    return '"' + "".join(out) + '"'


def number(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if math.isnan(v):
        return "nan"
    if math.isinf(v):
        return "inf" if v > 0 else "-inf"
    r = repr(v)                             # then match Rust's {:?}: 1e21 and 1.5e-7, not 1e+21 and 1.5e-07
    if "e" in r:
        mant, exp = r.split("e")
        r = "%se%d" % (mant[:-2] if mant.endswith(".0") else mant, int(exp))
    return r if any(c in r for c in ".e") else r + ".0"


def inline(v):
    if isinstance(v, str):
        return string(v)
    if isinstance(v, (bool, int, float)):
        return number(v)
    if isinstance(v, list):
        return "[" + ", ".join(inline(x) for x in v) + "]"
    if isinstance(v, dict):
        if not v:
            return "{}"
        return "{ " + ", ".join("%s = %s" % (key(k), inline(v[k])) for k in sorted(v, key=bytes_of)) + " }"
    return moment(v)


def moment(v):
    """A TOML date or time as the toml crate's Display writes it."""
    def clock(t):
        s = "%02d:%02d:%02d" % (t.hour, t.minute, t.second)
        return s + ("." + ("%06d" % t.microsecond).rstrip("0") if t.microsecond else "")
    if isinstance(v, datetime.datetime):
        s = "%04d-%02d-%02dT%s" % (v.year, v.month, v.day, clock(v))
        offset = v.utcoffset()
        if offset is None:
            return s
        minutes = int(offset.total_seconds()) // 60
        if minutes == 0:
            return s + "Z"
        sign = "-" if minutes < 0 else "+"
        return s + "%s%02d:%02d" % (sign, abs(minutes) // 60, abs(minutes) % 60)
    if isinstance(v, datetime.date):
        return "%04d-%02d-%02d" % (v.year, v.month, v.day)
    if isinstance(v, datetime.time):
        return clock(v)
    raise TypeError("not a TOML value: %r" % (v,))


def bytes_of(s):
    return s.encode("utf-8")


def field(out, path, v, inline_table=False):
    name = ".".join(key(p) for p in path)
    if isinstance(v, dict) and v and not inline_table:
        for k in sorted(v, key=bytes_of):   # component-wise: sub-keys sorted inside their table
            field(out, path + [k], v[k])
    elif isinstance(v, list) and any(isinstance(x, dict) for x in v):
        out.append("%s = [" % name)
        out.extend("  %s," % inline(x) for x in v)
        out.append("]")
    else:
        out.append("%s = %s" % (name, inline(v)))


def same(a, b):
    """Equal and of the same types, so 1 is not 1.0 and true is not 1."""
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    return type(a) is type(b) and a == b


def canonical(rec, defaults, style_defaults):
    """A record with its known defaults dropped and a literal colour in lower case."""
    rec = copy.deepcopy(rec)
    for k, v in defaults.items():
        if k in rec and same(rec[k], v):
            del rec[k]
    style = rec.get("style")
    if isinstance(style, dict):
        for k, v in style_defaults.items():
            if k in style and same(style[k], v):
                del style[k]
        if isinstance(style.get("color"), str) and HEX.match(style["color"]):
            style["color"] = style["color"].lower()
        if not style:
            del rec["style"]
    for label in rec.get("labels", []) if isinstance(rec.get("labels"), list) else []:
        if isinstance(label, dict) and same(label.get("at"), LABEL_AT):
            del label["at"]
    return rec


def dump(m):
    out = ["schema = %d" % m["schema"]]
    if m.get("requires"):
        out.append("requires = %s" % inline(sorted(m["requires"], key=bytes_of)))
    for k in sorted((k for k in m if k not in ("schema", "requires", "nodes", "edges")), key=bytes_of):
        field(out, [k], m[k])
    for table, first, coll, defaults, style_defaults in (
            ("node", NODE_FIRST, m["nodes"], NODE_DEFAULTS, NODE_STYLE_DEFAULTS),
            ("edge", EDGE_FIRST, m["edges"], EDGE_DEFAULTS, EDGE_STYLE_DEFAULTS)):
        for i in sorted(coll, key=bytes_of):
            rec = canonical(coll[i], defaults, style_defaults)
            out += ["", "[%s.%s]" % (table, i)]
            order = [k for k in first if k in rec] + sorted((k for k in rec if k not in first), key=bytes_of)
            for k in order:
                field(out, [k], rec[k], inline_table=(table == "edge" and k in ("from", "to")))
    return "\n".join(out) + "\n"


def load(text):
    d = tomllib.loads(text)
    if not d:
        return {"schema": 1, "nodes": {}, "edges": {}}
    if not isinstance(d.get("schema"), int) or isinstance(d.get("schema"), bool) or d["schema"] < 1:
        raise ValueError("schema must be an integer of 1 or more")
    m = {"schema": d.pop("schema")}
    nodes, edges = d.pop("node", {}), d.pop("edge", {})
    for coll in (nodes, edges):
        if not isinstance(coll, dict) or not all(isinstance(r, dict) for r in coll.values()):
            raise ValueError("node and edge must be tables of tables")
        for i in coll:
            if not ID.match(i):
                raise ValueError("bad id %r" % i)
    m.update(d)
    m["nodes"], m["edges"] = nodes, edges
    for rec in nodes.values():
        if isinstance(rec.get("text"), str):
            rec["text"] = label_text(rec["text"])
    for rec in edges.values():
        for label in rec.get("labels", []) if isinstance(rec.get("labels"), list) else []:
            if isinstance(label, dict) and isinstance(label.get("text"), str):
                label["text"] = label_text(label["text"])
    return m


def label_text(s):
    """Label line breaks are LF: a CRLF or a lone CR becomes LF (section 4.9)."""
    return s.replace("\r\n", "\n").replace("\r", "\n")


# ------------------------------------------------------------------ z keys
def enc3(n):
    return ALPHABET[n // 3844] + ALPHABET[n // 62 % 62] + ALPHABET[n % 62]


def dec3(s):
    p = (s[:3] + "000")[:3]
    return ALPHABET.index(p[0]) * 3844 + ALPHABET.index(p[1]) * 62 + ALPHABET.index(p[2])


def bisect(lo, hi):
    """A key strictly between lo ("" = none below) and hi (None = none above), or None if no key
    exists there. Never ends in '0'."""
    if hi is not None and (lo >= hi or (hi.startswith(lo) and set(hi[len(lo):]) <= {"0"})):
        return None
    out, i = "", 0
    while True:
        a = ALPHABET.index(lo[i]) if i < len(lo) else 0
        b = ALPHABET.index(hi[i]) if hi is not None else 62
        if a == b:
            out += ALPHABET[a]
        elif b - a >= 2:
            return out + ALPHABET[(a + b) // 2]
        elif hi is not None and i + 1 < len(hi):
            return out + ALPHABET[b]
        else:
            out += ALPHABET[a]
            hi = None
        i += 1


def key_between(lo, hi):
    """lo, hi: neighbouring keys or None. Returns a key, or None when the caller must renumber."""
    if lo is None and hi is None:
        k = "V"
    elif hi is None:
        n = dec3(lo) + 1
        n += n % 62 == 0
        k = enc3(n) if n < 62 ** 3 else bisect(lo, None)
    elif lo is None:
        n = dec3(hi) - 1
        n -= n % 62 == 0
        k = enc3(n) if n >= 1 else bisect("", hi)
    else:
        k = bisect(lo, hi)
    return k if k is not None and len(k) <= Z_MAX else None


def renumber(count):
    return [enc3(119165 + 2 * i) for i in range(count)]


# ------------------------------------------------------------------ self-checks
def rand_key(rng, zeros=True):
    k = "".join(rng.choice(ALPHABET if rng.random() < 0.7 else "0zV1") for _ in range(rng.randint(1, 6)))
    return k if zeros else (k.rstrip("0") or "V")


def selftest(example):
    rng = random.Random(1)
    # 1. key_between: strictly between bytewise, in the alphabet, never ends in '0'; None only when
    #    no key exists (hi is lo followed by zeros) or the cap is hit.
    none = 0
    for _ in range(400_000):
        a, b = rand_key(rng), rand_key(rng)
        if a == b:
            assert key_between(a, b) is None
            continue
        lo, hi = min(a, b), max(a, b)
        k = key_between(lo, hi)
        impossible = hi.startswith(lo) and set(hi[len(lo):]) <= {"0"}
        if impossible:
            assert k is None, (lo, hi, k)
            none += 1
        else:
            assert k is not None and lo < k < hi and ZKEY.match(k) and not k.endswith("0"), (lo, hi, k)
        up, down = key_between(lo, None), key_between(None, hi)
        assert up is not None and up > lo and not up.endswith("0"), (lo, up)
        if down is None:
            assert set(hi) <= {"0"}, hi          # nothing sorts below an all-zero key
        else:
            assert down < hi and not down.endswith("0") and ZKEY.match(down), (hi, down)
    print("key_between: 400,000 random pairs hold; %d had no key (hi = lo + zeros)" % none)
    # 2. growth: appending and prepending stay at three characters
    k, longest = key_between(None, None), 1
    for _ in range(100_000):
        nk = key_between(k, None)
        assert nk > k
        k, longest = nk, max(longest, len(nk))
    print("100,000 appends: longest key %d chars, last %r" % (longest, k))
    k, longest = "V", 1
    for _ in range(100_000):
        nk = key_between(None, k)
        assert nk < k
        k, longest = nk, max(longest, len(nk))
    print("100,000 prepends: longest key %d chars, last %r" % (longest, k))
    # 3. worst case: always insert directly above the same lower neighbour
    lo, hi, n = "V01", "V03", 0
    while True:
        k = key_between(lo, hi)
        if k is None:
            break
        assert lo < k < hi
        hi, n = k, n + 1
    print("same-gap inserts before the 64-character cap forces a renumber: %d" % n)
    r = renumber(50_000)
    assert r == sorted(r) and len(set(r)) == len(r) and not any(k.endswith("0") for k in r) and r[0] == "V01"
    print("renumber(50,000): sorted, unique, none ends in 0, %r .. %r" % (r[0], r[-1]))
    # 4. strings and floats survive a stock TOML 1.0 parser
    pool = ['"', "\\", "\n", "\t", "\r", "\x00", "\x1f", "\x7f", "[", "]", "#", "=", "'", " ", "a", "é", "→", "🚀", " ", "<<<<<<<"]
    for _ in range(50_000):
        s = "".join(rng.choice(pool) for _ in range(rng.randint(0, 12)))
        line = "x = %s\n" % string(s)
        assert "\n" not in line[:-1] and tomllib.loads(line)["x"] == s, repr(s)
    for f in [0.0, -0.0, 1.0, 0.25, 0.1, 1e15, 1e16, 1e21, 1e-5, 1.5e-7, 1.7976931348623157e308, 5e-324]:
        got = tomllib.loads("x = %s\n" % number(f))["x"]
        assert isinstance(got, float) and repr(got) == repr(f), (f, number(f))
    assert [number(x) for x in (1e21, 1e16, 1.5e-7, 1.0, -0.0)] == ["1e21", "1e16", "1.5e-7", "1.0", "-0.0"]
    print("50,000 random strings and 12 floats round-trip through tomllib on one line each")
    # 5. component-wise key order differs from joined-string order; the writer uses the former
    m = {"schema": 1, "nodes": {"a": {"kind": "shape", "style-x": 1, "style": {"color": "red"}}}, "edges": {}}
    assert dump(m).index("style.color") < dump(m).index("style-x") and dump(load(dump(m))) == dump(m)
    # 6. the example file
    text = open(example, encoding="utf-8", newline="").read()
    m = load(text)
    assert dump(m) == text, "example is not canonical"
    ids = set(m["nodes"])
    for e in m["edges"].values():
        for end in ("from", "to"):
            assert ("node" in e[end]) != ("at" in e[end]) and e[end].get("node", next(iter(ids))) in ids
    defaults = {"shape": "rect", "route": "elbow", "heads": ["none", "arrow"], "text": "", "labels": []}
    for rec in list(m["nodes"].values()) + list(m["edges"].values()):
        assert all(rec.get(k, object()) != v for k, v in defaults.items()), rec
        assert rec.get("style", {}).get("color") != "neutral" and rec.get("style", {}).get("fill") != "tint"
        assert rec.get("style", {}).get("stroke") != "solid" and ZKEY.match(rec["z"])
    assert not any(line != line.rstrip() or "\t" in line for line in text.split("\n"))
    print("example: %d nodes, %d edges, %d lines, %d bytes; parses with tomllib; canonical; no default written"
          % (len(m["nodes"]), len(m["edges"]), text.count("\n"), len(text.encode())))
    # 7. a non-canonical spelling of the same board loads to the same model
    loose = text.replace("\n", "\r\n").replace("schema = 1", "# hand-edited\nschema = 1", 1)
    assert load(loose.replace("\r\n", "\n")) == m and dump(load(loose.replace("\r\n", "\n"))) == text
    print("CRLF + comment variant loads to the same model and writes the canonical bytes")


def fuzzfile(d):
    rng = random.Random(7)
    pool = ['"', "\\", "\n", "\t", "\r", "\x00", "\x1f", "\x7f", "[", "#", "'", " ", "a", "é", "→", "🚀", " ", "﻿"]
    toml, hexes = [], []
    for i in range(20_000):
        s = "".join(rng.choice(pool) for _ in range(rng.randint(0, 12)))
        toml.append("k%d = %s" % (i, string(s)))
        hexes.append(s.encode("utf-8").hex())
    open(d + "/strings.toml", "w", encoding="utf-8").write("\n".join(toml) + "\n")
    open(d + "/strings.hex", "w").write("\n".join(hexes) + "\n")


def main(argv):
    if argv[1] == "check":
        text = open(argv[2], encoding="utf-8", newline="").read()
        same = dump(load(text)) == text
        print("canonical" if same else "valid, not canonical")
        return 0 if same else 2
    if argv[1] == "fmt":
        sys.stdout.write(dump(load(open(argv[2], encoding="utf-8", newline="").read())))
        return 0
    if argv[1] == "selftest":
        selftest(argv[2])
        return 0
    if argv[1] == "fuzzfile":
        fuzzfile(argv[2])
        return 0
    return 64


if __name__ == "__main__":
    sys.exit(main(sys.argv))
