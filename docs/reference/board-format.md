# Board file format

This page is the specification of a board file, schema 1. Schema 1 is
locked: nothing described here changes while a file says `schema = 1`, and
anything a later version adds follows the rules in
[Compatibility](#compatibility). It is written for people who generate,
convert, or review boards by hand or with a tool. What a board looks like
and how it is edited is in [Boards](boards.md).

## File

- A board is a file named `Name.wren-board` inside a group. The kind comes
  from the extension, matched without regard to case, and the title is the
  file name without it; nothing inside the file names it.
- The file is UTF-8 with no byte order mark, LF line endings, and exactly
  one LF at the end. Wren writes TOML 1.0 syntax only, in the
  [canonical form](#canonical-form), and reads any valid TOML of the same
  shape ([Reading](#reading)).
- Top-level keys: `schema` (integer, required, `1`), `requires` (array of
  strings, optional, see [Unknown content](#unknown-content-and-requires)),
  `node` (a table of id to [node record](#node-records)), and `edge` (a
  table of id to [edge record](#edge-records)). Any other top-level key is
  unknown content and is kept. `layout` and `ext` are reserved names.
- A file with no keys at all (empty, blank, or only comments) is an empty
  schema-1 board. A new board is the line `schema = 1` and a line feed.
- Comments are accepted. Like key order and spacing, they are lost the
  first time Wren saves the board after an edit. An annotation that must
  survive goes in an `ext.*` field.

## Ids

- An id matches `[A-Za-z0-9_-]{1,32}`. It is case-sensitive and compared
  byte by byte. It is the last part of the block header, always written as
  a bare key.
- Node ids are unique among nodes, and edge ids among edges (TOML itself
  refuses a duplicate header). A node and an edge that share an id are
  [flagged](#flagged-issues). `parent`, `from.node`, and `to.node` always
  name nodes.
- Wren generates ids of 8 characters from
  `0123456789abcdefghjkmnpqrstvwxyz`, chosen so that no node, connector, or
  name a dangling `parent` or connector end uses already has it.
  Hand-chosen ids such as `start` are equally valid.

## Node records

A node is a block `[node.<id>]`.

| Field | Type | Presence | Default | Values and meaning |
|---|---|---|---|---|
| `text` | string | optional | `""` | The label: plain text, line breaks as LF (see [Strings](#strings-labels-and-numbers)). |
| `kind` | string | required | | `shape`, or `text` (free text, no outline). Reserved: `frame`, `note`, `image`. Any other value is kept and drawn as a labelled placeholder in the node's box. |
| `at` | `[int, int]` | required | | `[x, y]` of the top-left corner ([Coordinates](#coordinates-and-units)). |
| `parent` | string | optional | none | The id of the containing node; `at` is then relative to it. |
| `shape` | string | optional | `rect` | For `kind = "shape"`: `rect`, `rounded`, `pill`, `ellipse`, `diamond`, `parallelogram`, `cylinder`, `document`. Any other value draws as `rect` and is kept. |
| `size` | `[int, int]` | required | | `[w, h]`, each at least 1. It is authoritative and never measured again on load. For `text` it is the box the text wraps in. |
| `style.color` | string | optional | `neutral` | A colour token or `#rrggbb` ([Colour and fill](#colour-and-fill)). |
| `style.fill` | string | optional | `tint` | `none`, `tint`, `solid`. Not used by `text`. |
| `style.stroke` | string | optional | `solid` | The outline: `solid` or `dashed`. |
| `z` | string | required | | The stacking key ([Stacking order](#stacking-order)). |
| `ext.<tool>.*` | table | optional | | Foreign data, namespaced by tool. Any TOML value. Never interpreted. |

Wren honours `parent` but never creates it. Of the required fields, a
missing `at` or `size` is an [error](#errors), and a missing `kind` or `z`
is [flagged](#flagged-issues).

## Edge records

A connector is a block `[edge.<id>]`.

| Field | Type | Presence | Default | Values and meaning |
|---|---|---|---|---|
| `kind` | string | required | | `edge`. Any other value is kept and drawn as a plain connector. |
| `from`, `to` | table | required | | An endpoint, one of the four forms below. |
| `heads` | `[string, string]` | optional | `["none", "arrow"]` | The marker at the `from` end, then at the `to` end: `none` or `arrow`. Any other value draws as `arrow`. |
| `labels` | array of tables | optional | `[]` | Each item is `{ text = string }` with an optional `at`, an integer from 0 to 1000 (default 500): the position along the route from the `from` end, in thousandths of its length. Wren edits the first label; all are drawn. |
| `route` | string | optional | `elbow` | `straight` or `elbow` (right-angled). Reserved: `curved`. Any other value draws as `straight`. The path is computed from the board and never stored. |
| `style.color` | string | optional | `neutral` | As for nodes; colours the line, the heads, and the labels. |
| `style.stroke` | string | optional | `solid` | `solid` or `dashed`. |
| `z` | string | required | | The stacking key. |
| `ext.<tool>.*` | table | optional | | As for nodes. |

An endpoint holds exactly one of `node` and `at`:

| Form | Meaning |
|---|---|
| `{ node = "id" }` | Attached to the node; the route picks the side. |
| `{ node = "id", side = "right" }` | Attached at the middle of `top`, `right`, `bottom`, or `left`. An unknown side reads as none. |
| `{ anchor = [fx, fy], node = "id" }` | Attached at a point of the node's box, in thousandths from its top-left corner, each from 0 to 1000: `[500, 0]` is the top centre. `anchor` wins over `side`. |
| `{ at = [x, y] }` | A free end at absolute board coordinates. In a free end, `side` and `anchor` are unknown content. |

Wren's editor sets `side` only when a connector end is dropped on a side
handle, never sets `anchor`, and never makes a free end; all three are kept
when a file has them. A missing `from` or `to` is an error.

## Reserved names

These are kept as unknown content and given no meaning in schema 1, except
`parent`, which is honoured. Each is a new key or a new value of an
existing string field, so a later version can add it without changing the
format. Their types are fixed now.

| Name | On | Type | Meaning |
|---|---|---|---|
| `kind = "frame"` | node | | A container; children point at it with `parent`; `text` is its title. |
| `kind = "note"`, `file`, `subpath` | node | strings | A note card. `file` is relative to the board's folder; `subpath` is `#Heading`. |
| `kind = "image"`, `src` | node | string | An image; `src` is relative to the board's folder (under `assets/`). |
| `locked` | node, edge | boolean | Default `false`. No effect in schema 1. |
| `link` | node | string | A URL, or a path relative to the board's folder. |
| `rot` | node | integer | Degrees clockwise about the centre. Default 0. |
| `markup` | node | string | The label's format (`"md"`). Absent means plain text. |
| `points` | edge | array of `[x, y]` | Hand-placed waypoints, absolute. |
| `layout` | top level | table | `direction`, for layout and import. |

Wren reads no `file`, `src`, or `link`: it opens no file and fetches
nothing a board names.

## Coordinates and units

- The unit is one logical pixel at 100 % zoom; x grows to the right and y
  downward. The editor snaps to a 20-unit grid, which is never stored, and
  values off the grid are valid.
- `at` is the top-left corner of the node's box, relative to the parent's
  top-left corner when `parent` is set, otherwise absolute. Free ends and
  `points` are absolute.
- Fractions are integers in thousandths, from 0 to 1000: `anchor` and a
  label's `at`. Every number in a known field is therefore an integer;
  floats appear only in unknown content.
- An integer in a known field must fit a signed 32-bit integer, or the file
  is an [error](#errors). A resolved coordinate beyond ±16,777,216 (2^24,
  the range a 32-bit float holds exactly) is [flagged](#flagged-issues) and
  drawn clamped.

## Stacking order

- `z` is 1 to 64 characters of `[0-9A-Za-z]`. Elements that share a
  parent are drawn in ascending byte order of `z` (digits, then upper case,
  then lower case), then of id, a node before an edge. Root nodes and all
  edges form one scope. A node's children are drawn directly above it, in
  their own order. A missing `z` sorts as the empty string.
- Keys are opaque: a hand-written `a1` is as good as a generated one. Wren
  never generates a key ending in `0`, because no key sorts between `X`
  and `X0`.
- A command that moves elements in the stacking order rewrites only the
  moved elements' `z` lines, except when no key fits between their new
  neighbours, the key would pass 64 characters, or a neighbour's key is
  not valid. Then it renumbers the whole scope, in drawing order, as
  `V01`, `V03`, … in the same undo step, and only then places the moved
  elements. A missing `z` counts as the empty key, so nothing goes below
  it without a renumber.

How Wren picks a key between two neighbours, either of which may be
absent, reading each character as a digit from 0 to 61 in that order:

1. No neighbours: `V`.
2. Nothing above: the first three characters of the lower key, padded on
   the right with `0`, as a base-62 number, plus 1, and 1 more if the
   result is a multiple of 62, written as three characters. When that
   would pass `zzz`, it bisects.
3. Nothing below: the same from the upper key, minus 1, and minus 1 again
   if the result is a multiple of 62. When that is below 1, it bisects.
4. Both present: bisect. At each position take the lower key's digit `a`
   (0 past its end) and the upper key's `b`. Equal: append it and go on. At
   least 2 apart: append their mean, rounded down, and stop. One apart and
   the upper key has more characters: append `b` and stop. One apart
   otherwise: append `a` and go on as if there were no upper key. When the
   upper key is the lower key followed only by `0`s, nothing fits.

The keys Wren picks are not part of the format: a new element, a key, or
a renumbered scope may come out differently in another version.

## Colour and fill

- `style.color` is a token: `neutral`, `red`, `orange`, `yellow`, `green`,
  `cyan`, `purple`, or `blue`. Each theme supplies the seven hues, so the
  file never changes with the theme. The six hues after `neutral` are JSON
  Canvas's presets 1 to 6, in order.
- A literal `#rrggbb` is also valid. It is read without regard to case,
  written in lower case, and drawn unchanged in every theme. Wren's
  commands write tokens only. Any other string is kept and drawn as
  `neutral`.
- `style.fill`: `none` draws the outline only, `tint` mixes the colour into
  the canvas, and `solid` fills with it. Any other value is kept and drawn
  as `tint`. The label's colour is chosen for contrast when the board is
  drawn and never stored.

## Strings, labels, and numbers

**Every string is written as a single-line basic string, and a line break
in a label is `\n`.** Every field is then one physical line, so changing
one field changes one line, labels included.

- Escapes: `\\`, `\"`, `\n`, `\t`, `\r`, and `\uXXXX` (four upper-case hex
  digits) for every other character from U+0000 to U+001F and for U+007F.
  Everything else is written as it is, with no Unicode normalisation.
  Wren reads every TOML string form (`"""…"""`, `'…'`, `'''…'''`).
- Label text (a node's `text`, a label item's `text`) is plain text, never
  Markdown. On load, a CRLF or a lone CR inside it becomes LF; nothing is
  trimmed. Every other string, unknown content included, is written as it
  was read. Other control characters are kept; exports leave out the ones
  XML cannot carry.
- Integers are decimal, with `-` as the only sign, no leading zeros, and
  no separators. Booleans are `true` and `false`.
- Floats, in unknown content only, are written as Rust's `{:?}` formats an
  `f64` (`1.0`, `0.25`, `-0.0`, `1e21`), and as `nan`, `inf`, and `-inf`.
  The type read is the type written: `1` stays an integer and `1.0` a
  float. Dates and times are written as TOML 1.0 requires; a time read
  without seconds (TOML 1.1's `07:32`) is written with `:00`.

## Unknown content and `requires`

- Kept through load and save, with value and type: unknown top-level keys;
  unknown keys in a node, an edge, `style`, an endpoint, or a label item;
  everything under `ext`; and unknown values of the string fields (`kind`,
  `shape`, colour, fill, stroke, `route`, `side`, the heads). They are
  drawn as the tables above say, and are never an error and never flagged.
- An element of unknown kind keeps all its fields. Its envelope must still
  be well-typed: every node has a box (`at`, `size`) and every edge two
  endpoints, whatever the kind. A known field with the wrong type is an
  error, not unknown content. TOML has no null, so a tool writing `ext`
  leaves nulls out.
- `requires` lists feature names (`[a-z0-9-]{1,32}`), written sorted, not
  deduplicated, and left out when empty. Schema 1 defines none. An entry
  outside that pattern is not an error: it is a feature this version does
  not know.
- A board that requires a feature Wren does not know opens **read-only**:
  it draws, pans, zooms, selects, copies, jumps, and exports, in the app
  and with `wren export`, and is never written. So is a board whose
  `schema` is not 1 but which still loads under these rules. When both
  apply, the reason given is the schema.
- A later feature adds its name to `requires` while a board uses it only
  if a version without the feature could write a wrong file or draw a
  misleading picture. `rot` would; `frame` would not, because `parent` is
  already honoured.

## Canonical form

Wren writes a board in one canonical form. Two versions of Wren write the
same bytes for the same board.

1. UTF-8, no byte order mark, LF, no tabs, no trailing spaces, no
   comments, exactly one LF at the end.
2. Line 1 is `schema = 1`. Then `requires = ["a", "b"]` if it is not
   empty. Then the unknown top-level keys in byte order, by rules 5 to 8.
3. Then one block per node, then one per edge, each group in byte order
   of id. A block is one empty line, the header `[node.<id>]` or
   `[edge.<id>]`, then its fields. There are no other empty lines.
4. Field order in a node: `text`, `kind`, then every other key in byte
   order. In an edge: `kind`, `from`, `to`, then every other key in byte
   order. The order never depends on whether a version knows a key.
5. One field per line, `key = value`. A key is bare if it matches
   `[A-Za-z0-9_-]+`, and a basic string otherwise.
6. A table is written as dotted keys, one leaf per line
   (`style.color = "blue"`), sub-keys in byte order inside their own
   table, recursively. Order is by key, not by the joined text:
   `style.color` comes before a sibling key `style-x`. Three cases are
   one-line inline tables, `{ k = v, k = v }` with keys in byte order: an
   edge's `from` and `to`, a table inside an array, and an empty table
   (`{}`).
7. An array is one line, `[a, b]`, nested arrays included. A field whose
   array holds a table is written one item per line: `key = [`, each item
   indented two spaces with a trailing comma, then `]`. Inside an inline
   table or another array everything stays on one line.
8. Scalars as in [Strings, labels, and numbers](#strings-labels-and-numbers).
9. A known optional field equal to its default is left out: empty `text`,
   `shape = "rect"`, `style.color = "neutral"`, `style.fill = "tint"`,
   `style.stroke = "solid"`, `route = "elbow"`,
   `heads = ["none", "arrow"]`, empty `labels`, a label's `at = 500`, and
   empty `requires`. A missing `kind` or `z` stays missing.
10. The output depends on the board alone, not on the order or layout of
    the file it was read from, the platform, or the locale. Writing a
    board that was read from Wren's own output gives the same bytes.

Opening a board never writes it. A board that is not in canonical form
(hand-written, or merged by git) keeps its bytes until its first edit, and
that edit's save writes the whole file in canonical form once. Undoing
every edit writes the original bytes back. Whether a board is unchanged is
decided by comparing canonical forms, never by comparing a file with its
canonical form, so two versions of Wren never rewrite each other's files.

## Reading

Wren reads the file as TOML without changing it first. That accepts any
order of keys and blocks, comments, CRLF line endings, a byte order mark,
sub-table and array-of-tables layouts, every string form, and the TOML 1.1
syntax the parser allows. A clean hand merge in git can leave blocks out
of order; that is valid input. Known fields are typed and everything else
is kept as it was read.

### Errors

A board with an error opens as text, under a banner naming the line and
column (counted in characters, a byte order mark excluded) and the
problem. The line is the key's for a field, the item's for an array item,
the id's in its header for a record missing `at`, `size`, `from`, or `to`,
and line 1 for a missing `schema`. These are errors:

- Text that is not valid TOML, including a duplicate key or header and
  nesting deeper than 80. A key or header of more than 80 dotted parts.
- Unknown content that the canonical form would write past that limit: a
  dotted key of more than 80 parts, or more than 80 arrays and inline
  tables open at once.
- `schema` missing from a file that has other keys, not an integer, or
  below 1. `requires` not an array of strings.
- `node` or `edge` that is not a table, or an entry in either that is not
  a table. An id that does not match [the id pattern](#ids).
- A known field of the wrong type: a float, `nan`, `inf`, or a string where
  an integer is required; an integer outside the signed 32-bit range; a
  pair (`at`, `size`, `anchor`, an endpoint's `at`, `heads`) without
  exactly two items; a label item without `text`. (`nan` and `inf` are
  legal in unknown content.)
- A node without `at` or `size`. An edge without `from` or `to`, or with an
  endpoint holding both or neither of `node` and `at`.
- A file over 8 MiB (8,388,608 bytes), or with more than 50,000 nodes and
  edges together. Both are reported at line 1, column 1.

A missing `at` or `size` is an error because such a node has no geometry,
and inventing one would be a repair nobody asked for. One trap for hand
and generated files: a field written after a `[node.a.style]` header lands
inside `style`. If the stray field is `at` or `size`, the node fails as
missing it, with the line; any other field is kept as an unknown style
key.

### Flagged issues

A flagged board opens and can be edited. The issue is counted in the
footer, marked on the element, and printed by `wren export`; the content
is kept byte for byte through a save, and nothing is repaired unless you
edit that element.

| Issue | Drawn as |
|---|---|
| An endpoint names no node (one issue per connector, naming which ends dangle). | A short stub from the other end; nothing if both ends dangle. |
| `parent` names no node. | A root node, its stored `at` taken as absolute. |
| A parent cycle, or a chain deeper than 32 levels (a root is level 1). | Every node in a cycle, and every node past level 32, as a root node with its stored `at` as absolute; the first 32 levels as written. A node hanging from a cycle without being in it is drawn inside its laid-out parent. |
| A node and an edge share an id (one issue on each). | Both, normally. Each is selected and edited as its own element. |
| A missing `kind`. | A node as a placeholder; an edge as a plain connector. |
| A missing `z`, an empty one, or one with a character outside `[0-9A-Za-z]`. | In byte order of what is there. |
| A value out of range (the table below). | Clamped or cut, as the table says. |

### Limits

| Limit | Value | A value past it |
|---|---|---|
| Coordinates | A node's resolved corner from −16,777,216 to 16,777,215, and its far edge at most 16,777,216; a free end within ±16,777,216 | Drawn clamped. |
| Size | Each side at least 1 | Drawn 1 long. |
| `anchor`, a label's `at` | 0 to 1000 | Drawn clamped. |
| One label, or a placeholder's caption | 16 KiB | Only the first 16 KiB is drawn, cut at a character boundary. |
| Labels on one edge | 16 | Only the first 16 are drawn. |
| `z` | 64 characters | Stacked by its first 64. |
| Lines of one label (a node label, a connector label, or a caption), after wrapping | 1,024, blank lines included | Only the first 1,024 are drawn. |
| Label lines on a whole board | 262,144, blank lines included, in drawing order | A label reaching past them is cut to what is left; one with none left draws nothing. |

Each is flagged once per element and limit. The stored values are never
changed.

## Compatibility

Schema 1 is frozen: the bytes of a file, the id and key alphabets, every
key name with its type, default, and presence, the endpoint forms, the
reserved names with their types, whole units and thousandths, the colour
vocabulary and the fill and stroke values, single-line strings and their
escapes, what is kept as unknown content, how `requires` and `schema`
make a board read-only, every rule of the canonical form, and what the
reader accepts, refuses, and flags.

- **Allowed without a change of schema:** new optional keys, new kinds,
  shapes, and other values of the existing string fields, new top-level
  keys, and anything under `ext.*`. Older versions keep all of these as
  unknown content.
- **Needs a `requires` entry:** anything an older version would draw
  misleadingly, such as `rot`.
- **Needs `schema = 2`:** any change to the bytes written for an existing
  board, and any file the reader accepts now and would then refuse or read
  differently.

How the editor makes new content (where a shape goes, generated ids and
keys, default sizes) and how a board is drawn (routes, theme colours) are
not part of the format, since neither is stored.

## Clipboard fragments

Copying from a board puts text on the clipboard: the line
`# wren-board fragment 1`, then a board file in canonical form holding only
the copied elements. A node whose parent was not copied is written at the
top level with its absolute `at`. Paste reads it with the same rules and
limits as a file, refuses a fragment with a newer `schema` or an unknown
`requires`, and gives every element a new id.

## Example

A password reset flow with every endpoint form (147 lines):

```toml
schema = 1

[node.av4c4ntg]
text = "Show \"Check your email\""
kind = "shape"
at = [140, 540]
shape = "parallelogram"
size = [200, 60]
z = "V05"

[node.bydezvv1]
text = "Look up account"
kind = "shape"
at = [160, 120]
size = [160, 60]
z = "V02"

[node.ctrj33th]
text = "Send reset link\n(expires in 30 min)"
kind = "shape"
at = [160, 400]
shape = "rounded"
size = [160, 80]
style.color = "purple"
z = "V04"

[node.ddrf7fvp]
text = "Password reset — draft 2"
kind = "text"
at = [100, -60]
size = [320, 20]
z = "V"

[node.eje807fy]
text = "Done"
kind = "shape"
at = [180, 660]
shape = "ellipse"
size = [120, 60]
style.color = "green"
style.fill = "solid"
z = "V06"

[node.k5f7n16m]
text = "Account found?"
kind = "shape"
at = [160, 240]
shape = "diamond"
size = [160, 100]
style.color = "yellow"
z = "V03"

[node.start]
text = "Reset requested"
kind = "shape"
at = [180, 0]
shape = "pill"
size = [120, 60]
style.color = "green"
z = "V01"

[node.t8y8fyb9]
text = "Audit log"
kind = "shape"
at = [420, 400]
shape = "document"
size = [140, 80]
style.fill = "none"
style.stroke = "dashed"
z = "V04V"

[node.xy2v9h64]
text = "Users DB"
kind = "shape"
at = [420, 100]
shape = "cylinder"
size = [120, 100]
style.color = "blue"
z = "V07"

[edge.35y91dgz]
kind = "edge"
from = { node = "k5f7n16m", side = "bottom" }
to = { node = "ctrj33th" }
labels = [
  { text = "yes" },
]
z = "V0B"

[edge.6491fnkw]
kind = "edge"
from = { node = "start" }
to = { node = "bydezvv1" }
z = "V08"

[edge.85ntwknw]
kind = "edge"
from = { node = "bydezvv1" }
to = { node = "k5f7n16m" }
z = "V0A"

[edge.a27zqcxy]
kind = "edge"
from = { anchor = [1000, 500], node = "ctrj33th" }
to = { node = "t8y8fyb9", side = "left" }
z = "V0D"

[edge.ga3nphf6]
kind = "edge"
from = { node = "av4c4ntg" }
to = { node = "eje807fy" }
z = "V0F"

[edge.jppm92yk]
kind = "edge"
from = { node = "k5f7n16m", side = "left" }
to = { node = "av4c4ntg", side = "left" }
labels = [
  { at = 250, text = "no" },
]
style.color = "red"
z = "V0C"

[edge.p9t2zee2]
kind = "edge"
from = { node = "bydezvv1", side = "right" }
to = { node = "xy2v9h64", side = "left" }
heads = ["arrow", "arrow"]
route = "straight"
style.stroke = "dashed"
z = "V09"

[edge.tvnnkqhm]
kind = "edge"
from = { node = "ctrj33th" }
to = { node = "av4c4ntg" }
z = "V0E"

[edge.tyw6cetx]
kind = "edge"
from = { node = "t8y8fyb9", side = "right" }
to = { at = [660, 440] }
labels = [
  { text = "to SIEM" },
]
route = "straight"
z = "V0G"
```
