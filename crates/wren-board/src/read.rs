//! The loader: a board file's text into a [`Board`].
//!
//! The text is parsed as any valid TOML, as written, with no normalising
//! first, so key and block order, comments, CRLF, a BOM, sub-table layouts
//! and every string form are all accepted. Known fields are then typed and
//! everything else is kept as TOML values. A wrong type in a known field,
//! a record without its geometry or endpoints, and a bad id are errors;
//! anything that can still be drawn where it is (a dangling reference, a
//! missing `kind` or `z`, an out-of-range number) loads, and `check`
//! reports it.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;

use toml::de::{DeTable, DeValue};
use toml::{Table, Value};

use crate::{
    Board, Color, Edge, EdgeStyle, End, Head, Id, Label, Named, Node, NodeStyle, Route, Shape,
};

/// Why a file cannot be read as a board. Such a board opens as raw text
/// with this message and its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The line of the offending key or value, from 1.
    pub line: usize,
    /// The column on that line, from 1, in characters.
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Reads a board file. Never panics, whatever the input.
pub fn parse(source: &str) -> Result<Board, ParseError> {
    let table = source.parse::<Table>().map_err(|error| {
        let (line, column) = error
            .span()
            .map_or((1, 1), |span| position(source, span.start));
        ParseError {
            line,
            column,
            message: error.message().trim_end().to_owned(),
        }
    })?;
    board(table).map_err(|fault| fault.locate(source))
}

/// One step from the document root to a value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Key(String),
    Item(usize),
}

/// A model error, before it has a position. The path is collected on the
/// way out, so reading a valid file builds none.
#[derive(Debug)]
struct Fault {
    path: Vec<Step>,
    detail: String,
}

impl Fault {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            detail: detail.into(),
        }
    }

    fn wrong(expected: &str, found: &Value) -> Self {
        let found = found.type_str();
        let article = if found.starts_with(['a', 'i']) {
            "an"
        } else {
            "a"
        };
        Self::new(format!("must be {expected}, not {article} {found}"))
    }

    fn count(expected: &str, items: usize) -> Self {
        let noun = if items == 1 { "item" } else { "items" };
        Self::new(format!("must be {expected}, not {items} {noun}"))
    }

    fn under(mut self, step: Step) -> Self {
        self.path.insert(0, step);
        self
    }

    fn under_key(self, key: &str) -> Self {
        self.under(Step::Key(key.to_owned()))
    }

    /// The error with the line and column of the deepest part of the
    /// path the document has. The parse that keeps spans runs only here,
    /// on the error path.
    fn locate(self, source: &str) -> ParseError {
        let span = DeTable::parse(source)
            .ok()
            .and_then(|root| find(root.get_ref(), &self.path));
        let (line, column) = span.map_or((1, 1), |span| position(source, span.start));
        let message = if self.path.is_empty() {
            self.detail
        } else {
            format!("`{}` {}", dotted(&self.path), self.detail)
        };
        ParseError {
            line,
            column,
            message,
        }
    }
}

/// The span of the last step of `path` found in the document: a key's
/// span for a key, an item's span for an array item.
fn find(root: &DeTable<'_>, path: &[Step]) -> Option<Range<usize>> {
    let mut table = Some(root);
    let mut array = None;
    let mut span = None;
    for step in path {
        let found = match step {
            Step::Key(key) => table.and_then(|table| {
                table
                    .iter()
                    .find(|(name, _)| name.get_ref().as_ref() == key.as_str())
                    .map(|(name, value)| (name.span(), value.get_ref()))
            }),
            Step::Item(index) => array.and_then(|items: &[toml::Spanned<DeValue<'_>>]| {
                items.get(*index).map(|item| (item.span(), item.get_ref()))
            }),
        };
        let Some((found_span, value)) = found else {
            break;
        };
        span = Some(found_span);
        (table, array) = match value {
            DeValue::Table(inner) => (Some(inner), None),
            DeValue::Array(items) => (None, Some(&items[..])),
            _ => (None, None),
        };
    }
    span
}

/// A path as TOML would spell it, with an array item as `[index]`.
fn dotted(path: &[Step]) -> String {
    let mut out = String::new();
    for step in path {
        match step {
            Step::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                if !key.is_empty()
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    out.push_str(key);
                } else {
                    out.push_str(&format!("{key:?}"));
                }
            }
            Step::Item(index) => out.push_str(&format!("[{index}]")),
        }
    }
    out
}

/// The line and column, both from 1, of a byte offset. A leading BOM is
/// not a column.
fn position(source: &str, offset: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    let before = bytes.get(..offset).unwrap_or(bytes);
    let line = 1 + before.iter().filter(|&&b| b == b'\n').count();
    let start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let mut on_line = before.get(start..).unwrap_or_default();
    if start == 0 {
        on_line = on_line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(on_line);
    }
    let column = 1 + on_line.iter().filter(|&&b| b & 0xC0 != 0x80).count();
    (line, column)
}

fn board(mut top: Table) -> Result<Board, Fault> {
    if top.is_empty() {
        return Ok(Board::default());
    }
    let Some(schema) = top.remove("schema") else {
        return Err(Fault::new(
            "the file has keys but no `schema`; a board starts with `schema = 1`",
        ));
    };
    let schema = int(schema).map_err(|fault| fault.under_key("schema"))?;
    if schema < 1 {
        return Err(Fault::new("must be 1 or more").under_key("schema"));
    }
    let requires = take(&mut top, "requires", strings)?.unwrap_or_default();
    let nodes = take(&mut top, "node", |value| records(value, node))?.unwrap_or_default();
    let edges = take(&mut top, "edge", |value| records(value, edge))?.unwrap_or_default();
    Ok(Board {
        schema,
        requires,
        nodes,
        edges,
        extra: top,
    })
}

/// `node` or `edge`: a table of id to record.
fn records<T>(
    value: Value,
    record: fn(Table) -> Result<T, Fault>,
) -> Result<BTreeMap<Id, T>, Fault> {
    let mut out = BTreeMap::new();
    for (key, value) in table(value)? {
        let Some(id) = Id::new(&key) else {
            return Err(
                Fault::new("is not a valid id: use 1 to 32 of A-Z, a-z, 0-9, `_` and `-`")
                    .under(Step::Key(key)),
            );
        };
        let read = table(value).and_then(record);
        out.insert(id, read.map_err(|fault| fault.under(Step::Key(key)))?);
    }
    Ok(out)
}

fn node(mut fields: Table) -> Result<Node, Fault> {
    let text = take(&mut fields, "text", string)?.map_or_else(String::new, label_text);
    let kind = take(&mut fields, "kind", string)?.map(Named::read);
    let at = take(&mut fields, "at", pair)?;
    let parent = take(&mut fields, "parent", string)?;
    let shape = take(&mut fields, "shape", string)?.map_or(Named::Known(Shape::Rect), Named::read);
    let size = take(&mut fields, "size", pair)?;
    let style = take(&mut fields, "style", node_style)?.unwrap_or_default();
    let z = take(&mut fields, "z", string)?;
    Ok(Node {
        text,
        kind,
        at: at.ok_or_else(|| Fault::new("has no `at`"))?,
        parent,
        shape,
        size: size.ok_or_else(|| Fault::new("has no `size`"))?,
        style,
        z,
        extra: fields,
    })
}

fn node_style(value: Value) -> Result<NodeStyle, Fault> {
    let mut fields = table(value)?;
    let defaults = NodeStyle::default();
    Ok(NodeStyle {
        color: take(&mut fields, "color", string)?.map_or(defaults.color, Color::read),
        fill: take(&mut fields, "fill", string)?.map_or(defaults.fill, Named::read),
        stroke: take(&mut fields, "stroke", string)?.map_or(defaults.stroke, Named::read),
        extra: fields,
    })
}

fn edge(mut fields: Table) -> Result<Edge, Fault> {
    let kind = take(&mut fields, "kind", string)?.map(Named::read);
    let from = take(&mut fields, "from", end)?;
    let to = take(&mut fields, "to", end)?;
    let heads = take(&mut fields, "heads", heads)?.unwrap_or(Edge::DEFAULT_HEADS);
    let labels = take(&mut fields, "labels", labels)?.unwrap_or_default();
    let route = take(&mut fields, "route", string)?.map_or(Named::Known(Route::Elbow), Named::read);
    let style = take(&mut fields, "style", edge_style)?.unwrap_or_default();
    let z = take(&mut fields, "z", string)?;
    Ok(Edge {
        kind,
        from: from.ok_or_else(|| Fault::new("has no `from`"))?,
        to: to.ok_or_else(|| Fault::new("has no `to`"))?,
        heads,
        labels,
        route,
        style,
        z,
        extra: fields,
    })
}

/// An endpoint holds exactly one of `node` and `at`. In a free end every
/// other key is unknown content, `side` and `anchor` included.
fn end(value: Value) -> Result<End, Fault> {
    let mut fields = table(value)?;
    match (fields.contains_key("node"), fields.contains_key("at")) {
        (true, false) => {
            let node = take(&mut fields, "node", string)?.unwrap_or_default();
            let side = take(&mut fields, "side", string)?.map(Named::read);
            let anchor = take(&mut fields, "anchor", pair)?;
            Ok(End::Node {
                node,
                side,
                anchor,
                extra: fields,
            })
        }
        (false, true) => {
            let at = take(&mut fields, "at", pair)?.unwrap_or_default();
            Ok(End::Free { at, extra: fields })
        }
        _ => Err(Fault::new("must have exactly one of `node` and `at`")),
    }
}

fn heads(value: Value) -> Result<[Named<Head>; 2], Fault> {
    const EXPECTED: &str = "an array of two strings";
    let Value::Array(items) = value else {
        return Err(Fault::wrong(EXPECTED, &value));
    };
    let [from, to] =
        <[Value; 2]>::try_from(items).map_err(|items| Fault::count(EXPECTED, items.len()))?;
    let from = string(from).map_err(|fault| fault.under(Step::Item(0)))?;
    let to = string(to).map_err(|fault| fault.under(Step::Item(1)))?;
    Ok([Named::read(from), Named::read(to)])
}

fn labels(value: Value) -> Result<Vec<Label>, Fault> {
    let Value::Array(items) = value else {
        return Err(Fault::wrong("an array of tables", &value));
    };
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| label(item).map_err(|fault| fault.under(Step::Item(index))))
        .collect()
}

fn label(value: Value) -> Result<Label, Fault> {
    let mut fields = table(value)?;
    let text = take(&mut fields, "text", string)?.ok_or_else(|| Fault::new("has no `text`"))?;
    let at = take(&mut fields, "at", int)?.unwrap_or(Label::DEFAULT_AT);
    Ok(Label {
        text: label_text(text),
        at,
        extra: fields,
    })
}

fn edge_style(value: Value) -> Result<EdgeStyle, Fault> {
    let mut fields = table(value)?;
    let defaults = EdgeStyle::default();
    Ok(EdgeStyle {
        color: take(&mut fields, "color", string)?.map_or(defaults.color, Color::read),
        stroke: take(&mut fields, "stroke", string)?.map_or(defaults.stroke, Named::read),
        extra: fields,
    })
}

/// Removes `key` and reads it, so whatever is left is unknown content.
fn take<T>(
    fields: &mut Table,
    key: &str,
    read: impl FnOnce(Value) -> Result<T, Fault>,
) -> Result<Option<T>, Fault> {
    fields
        .remove(key)
        .map(|value| read(value).map_err(|fault| fault.under_key(key)))
        .transpose()
}

fn table(value: Value) -> Result<Table, Fault> {
    match value {
        Value::Table(table) => Ok(table),
        other => Err(Fault::wrong("a table", &other)),
    }
}

fn string(value: Value) -> Result<String, Fault> {
    match value {
        Value::String(text) => Ok(text),
        other => Err(Fault::wrong("a string", &other)),
    }
}

fn strings(value: Value) -> Result<Vec<String>, Fault> {
    const EXPECTED: &str = "an array of strings";
    let Value::Array(items) = value else {
        return Err(Fault::wrong(EXPECTED, &value));
    };
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| match item {
            Value::String(text) => Ok(text),
            other => Err(Fault::wrong("a string", &other).under(Step::Item(index))),
        })
        .collect()
}

/// Every integer in a known field fits `i32`.
fn int(value: Value) -> Result<i32, Fault> {
    match value {
        Value::Integer(n) => i32::try_from(n).map_err(|_| {
            Fault::new(format!(
                "is out of range: {n} is not between {} and {}",
                i32::MIN,
                i32::MAX
            ))
        }),
        other => Err(Fault::wrong("an integer", &other)),
    }
}

/// `[x, y]` or `[w, h]`: exactly two integers.
fn pair(value: Value) -> Result<[i32; 2], Fault> {
    const EXPECTED: &str = "an array of two integers";
    let Value::Array(items) = value else {
        return Err(Fault::wrong(EXPECTED, &value));
    };
    let [x, y] =
        <[Value; 2]>::try_from(items).map_err(|items| Fault::count(EXPECTED, items.len()))?;
    Ok([
        int(x).map_err(|fault| fault.under(Step::Item(0)))?,
        int(y).map_err(|fault| fault.under(Step::Item(1)))?,
    ])
}

/// Label line breaks are LF: a CRLF or a lone CR becomes one.
fn label_text(text: String) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, Rng};
    use crate::{Dash, EdgeKind, Extra, Fill, Head, NodeKind, Side, Token, Vocabulary};

    #[track_caller]
    fn ok(source: &str) -> Board {
        parse(source).unwrap_or_else(|error| panic!("{error}\n{source}"))
    }

    #[track_caller]
    fn err(source: &str) -> ParseError {
        match parse(source) {
            Ok(board) => panic!("parsed, expected an error:\n{source}\n{board:?}"),
            Err(error) => error,
        }
    }

    fn keys(extra: &Extra) -> Vec<&str> {
        extra.keys().map(String::as_str).collect()
    }

    #[test]
    fn every_committed_board_parses() {
        for (name, text) in fixtures::all() {
            if let Err(error) = parse(text) {
                panic!("{name}: {error}");
            }
        }
    }

    // Writing arrives with `write`; until then the two must read as the
    // same model. `requires` is compared sorted, as it is written.
    #[test]
    fn non_canonical_inputs_read_as_their_canonical_files() {
        for (name, input, output) in fixtures::PAIRS {
            let mut read = ok(input);
            read.requires.sort();
            assert_eq!(read, ok(output), "{name}");
        }
    }

    #[test]
    fn a_file_with_no_keys_is_an_empty_board() {
        for source in [
            "",
            "\n",
            " \t\r\n",
            "# a comment\n\n# and another",
            "\u{feff}",
            crate::EMPTY,
        ] {
            assert_eq!(ok(source), Board::default(), "{source:?}");
        }
    }

    #[test]
    fn known_fields_are_typed() {
        let board = ok(fixtures::CANONICAL[1].1);
        assert_eq!(fixtures::CANONICAL[1].0, "fields.out");

        let base = &board.nodes["base"];
        assert_eq!(base.text, "");
        assert_eq!(base.shape, Named::Known(Shape::Rect));
        assert_eq!(base.style, NodeStyle::default());
        assert_eq!(base.parent, None);

        let full = &board.nodes["full"];
        assert_eq!(full.text, "Every field");
        assert_eq!(full.kind, Some(Named::Known(NodeKind::Shape)));
        assert_eq!(full.at, [20, -40]);
        assert_eq!(full.parent.as_deref(), Some("base"));
        assert_eq!(full.shape, Named::Known(Shape::Diamond));
        assert_eq!(full.size, [140, 80]);
        assert_eq!(full.style.color, Color::Token(Token::Red));
        assert_eq!(full.style.fill, Named::Known(Fill::Solid));
        assert_eq!(full.style.stroke, Named::Known(Dash::Dashed));
        assert_eq!(full.z.as_deref(), Some("V02"));
        assert_eq!(keys(&full.extra), ["ext"]);

        let words = &board.nodes["words"];
        assert_eq!(words.kind, Some(Named::Known(NodeKind::Text)));
        assert_eq!(words.style.color, Color::Hex([0x3a, 0x7b, 0xd5]));
        assert_eq!(words.style.fill, Named::Known(Fill::None));

        let edge = &board.edges["full"];
        assert_eq!(edge.kind, Some(Named::Known(EdgeKind::Edge)));
        assert_eq!(
            edge.from,
            End::Node {
                node: "base".into(),
                side: Some(Named::Known(Side::Right)),
                anchor: None,
                extra: Extra::new(),
            }
        );
        assert_eq!(
            edge.to,
            End::Node {
                node: "words".into(),
                side: None,
                anchor: Some([0, 500]),
                extra: Extra::new(),
            }
        );
        assert_eq!(
            edge.heads,
            [Named::Known(Head::Arrow), Named::Known(Head::None)]
        );
        let labels: Vec<(&str, i32)> = edge
            .labels
            .iter()
            .map(|l| (l.text.as_str(), l.at))
            .collect();
        assert_eq!(labels, [("first", 250), ("second", Label::DEFAULT_AT)]);
        assert_eq!(edge.route, Named::Known(Route::Straight));
        assert_eq!(edge.style.color, Color::Token(Token::Blue));
        assert_eq!(edge.style.stroke, Named::Known(Dash::Dashed));
        assert_eq!(edge.z.as_deref(), Some("V04"));
        assert_eq!(keys(&edge.extra), ["ext"]);
    }

    #[test]
    fn unknown_content_and_values_are_kept() {
        let board = ok(fixtures::CANONICAL[4].1);
        assert_eq!(fixtures::CANONICAL[4].0, "unknown.out");
        assert_eq!(keys(&board.extra), ["layout", "meta", "zz_last"]);

        let a = &board.nodes["a"];
        assert_eq!(a.shape, Named::Other("hexagon".into()));
        assert_eq!(a.style.color, Color::Unknown("teal".into()));
        assert_eq!(a.style.fill, Named::Other("hatched".into()));
        assert_eq!(a.style.stroke, Named::Other("dotted".into()));
        assert_eq!(keys(&a.style.extra), ["glow"]);
        assert_eq!(keys(&a.extra), ["ext", "extra_field", "zz"]);
        assert_eq!(board.nodes["b"].kind, Some(Named::Other("sticky".into())));

        let e = &board.edges["e"];
        assert_eq!(e.kind, Some(Named::Other("link".into())));
        let End::Node { side, extra, .. } = &e.from else {
            panic!("{:?}", e.from);
        };
        assert_eq!(side, &Some(Named::Other("middle".into())));
        assert_eq!(keys(extra), ["port"]);
        assert_eq!(
            e.heads,
            [
                Named::Other("diamond".into()),
                Named::Other("circle".into())
            ]
        );
        assert_eq!(keys(&e.labels[0].extra), ["font"]);
        assert_eq!(e.route, Named::Other("curved".into()));
        // A connector has no fill, so its `fill` is unknown content.
        assert_eq!(keys(&e.style.extra), ["fill"]);
        assert_eq!(keys(&e.extra), ["weight"]);
        assert_eq!(e.style.stroke.as_str(), "wavy");

        let future = ok(fixtures::CANONICAL[8].1);
        assert_eq!(future.requires, ["frames"]);
        let card = &future.nodes["card"];
        assert_eq!(card.kind, Some(Named::Other("note".into())));
        assert_eq!(card.parent.as_deref(), Some("frame"));
        assert_eq!(keys(&card.extra), ["file", "markup", "rot", "subpath"]);
        let End::Node { side, anchor, .. } = &future.edges["e"].to else {
            panic!("{:?}", future.edges["e"].to);
        };
        assert_eq!(
            (side, anchor),
            (&Some(Named::Other("centre".into())), &Some([500, 0]))
        );
    }

    #[test]
    fn colours_are_tokens_hex_in_either_case_or_kept() {
        for token in Token::ALL {
            assert_eq!(Color::read(token.name().into()), Color::Token(*token));
        }
        for (text, rgb) in [
            ("#3a7bd5", [0x3a, 0x7b, 0xd5]),
            ("#3A7BD5", [0x3a, 0x7b, 0xd5]),
            ("#FF00Aa", [255, 0, 0xaa]),
        ] {
            assert_eq!(Color::read(text.into()), Color::Hex(rgb), "{text}");
        }
        for text in [
            "",
            "Red",
            "#ABC",
            "#GGGGGG",
            "#3a7bd5 ",
            "3a7bd5",
            "#3a7bd5ff",
            "#ｆｆｆｆｆｆ",
        ] {
            assert_eq!(
                Color::read(text.into()),
                Color::Unknown(text.into()),
                "{text:?}"
            );
        }
    }

    #[test]
    fn named_values_keep_their_spelling() {
        for shape in Shape::ALL {
            assert_eq!(
                Named::<Shape>::read(shape.name().into()),
                Named::Known(*shape)
            );
            assert_eq!(Named::Known(*shape).as_str(), shape.name());
        }
        let other = Named::<Shape>::read("Rect".into());
        assert_eq!(other, Named::Other("Rect".into()));
        assert_eq!((other.as_str(), other.known()), ("Rect", None));
    }

    #[test]
    fn label_line_breaks_become_line_feeds_and_nothing_else_changes() {
        let board = ok(concat!(
            "schema = 1\n",
            "[node.a]\n",
            "text = \"a\\r\\nb\\rc\\n\\r\\n \"\n",
            "kind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\n",
            "ext.raw = \"x\\r\\ny\\r\"\n",
            "[edge.e]\n",
            "kind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"a\" }\n",
            "labels = [{ text = \"1\\r2\\r\\n3\", note = \"k\\r\" }]\n",
        ));
        let a = &board.nodes["a"];
        assert_eq!(a.text, "a\nb\nc\n\n ");
        assert_eq!(a.extra["ext"]["raw"].as_str(), Some("x\r\ny\r"));
        let label = &board.edges["e"].labels[0];
        assert_eq!(label.text, "1\n2\n3");
        assert_eq!(label.extra["note"].as_str(), Some("k\r"));
    }

    /// Content `check` flags, not errors: each of these loads.
    #[test]
    fn flagged_and_unusual_content_loads() {
        let node = "kind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\n";
        let one = |fields: &str| ok(&format!("schema = 1\n[node.a]\n{fields}"));

        let bare = one("at = [0, 0]\nsize = [1, 1]\n");
        assert_eq!((&bare.nodes["a"].kind, &bare.nodes["a"].z), (&None, &None));
        let z = one(&format!("{node}z = \"not a key!\"\n"));
        assert_eq!(z.nodes["a"].z.as_deref(), Some("not a key!"));
        let long = "V".repeat(65);
        let z = one(&format!("{node}z = \"{long}\"\n"));
        assert_eq!(z.nodes["a"].z.as_deref(), Some(long.as_str()));
        let size = one("kind = \"shape\"\nat = [0, 0]\nsize = [-1, 0]\n");
        assert_eq!(size.nodes["a"].size, [-1, 0]);
        let at = one("kind = \"shape\"\nat = [2147483647, -2147483648]\nsize = [1, 1]\n");
        assert_eq!(at.nodes["a"].at, [i32::MAX, i32::MIN]);
        for parent in ["nowhere", "a", "not an id!"] {
            let board = one(&format!("{node}parent = \"{parent}\"\n"));
            assert_eq!(board.nodes["a"].parent.as_deref(), Some(parent));
        }

        // A node and an edge sharing an id; an end naming no node; an
        // anchor out of range.
        let shared = one(&format!(
            "{node}[edge.a]\nkind = \"edge\"\nfrom = {{ node = \"x\" }}\n\
             to = {{ node = \"a\", anchor = [-5, 5000] }}\n"
        ));
        assert!(shared.nodes.contains_key("a") && shared.edges.contains_key("a"));

        // In a free end, `side` and `anchor` are unknown content.
        let free = ok("schema = 1\n[edge.e]\nfrom = { at = [0, 0] }\n\
                       to = { at = [1, 1], side = 7, anchor = \"x\" }\n");
        let End::Free { at, extra } = &free.edges["e"].to else {
            panic!("{:?}", free.edges["e"].to);
        };
        assert_eq!((at, keys(extra)), (&[1, 1], vec!["anchor", "side"]));
        assert_eq!(free.edges["e"].kind, None);

        let labels = "{ text = \"x\", at = 2000 }, ".repeat(17);
        let many = ok(&format!(
            "schema = 1\n[edge.e]\nkind = \"edge\"\nfrom = {{ at = [0, 0] }}\n\
             to = {{ at = [1, 1] }}\nlabels = [{labels}]\n"
        ));
        assert_eq!(many.edges["e"].labels.len(), 17);
        assert_eq!(many.edges["e"].labels[0].at, 2000);

        let future = ok("schema = 2\nrequires = [\"frames\", \"Not A Feature!\"]\n");
        assert_eq!((future.schema, future.requires.len()), (2, 2));

        // Floats, `nan` and dates are fine outside known fields, and the
        // reserved ring-1 fields are unknown content of any type.
        let loose = one(&format!(
            "{node}ext.t = [nan, inf, -inf, 1979-05-27T07:32:00Z]\nrot = 1.5\nlocked = \"yes\"\n"
        ));
        assert_eq!(keys(&loose.nodes["a"].extra), ["ext", "locked", "rot"]);
        let top = ok("schema = 1\nlayout = 3\next = \"anything\"\nnode = {}\nedge = {}\n");
        assert_eq!(keys(&top.extra), ["ext", "layout"]);
        assert!(top.nodes.is_empty() && top.edges.is_empty());

        let longest = format!("schema = 1\n[node.{}]\n{node}", "a".repeat(Id::MAX_LEN));
        assert_eq!(ok(&longest).nodes.len(), 1);
    }

    /// A one-record board whose last line is `field = value`, and that
    /// line's number. `record` is `node` or `edge`.
    fn with_field(record: &str, field: &str, value: &str) -> (String, usize) {
        let defaults: &[(&str, &str)] = if record == "node" {
            &[("kind", "\"shape\""), ("at", "[0, 0]"), ("size", "[1, 1]")]
        } else {
            &[
                ("kind", "\"edge\""),
                ("from", "{ node = \"a\" }"),
                ("to", "{ node = \"b\" }"),
            ]
        };
        let mut lines = vec!["schema = 1".to_owned(), format!("[{record}.r]")];
        for (key, default) in defaults {
            if *key != field {
                lines.push(format!("{key} = {default}"));
            }
        }
        lines.push(format!("{field} = {value}"));
        (lines.join("\n") + "\n", lines.len())
    }

    #[test]
    fn a_wrong_type_in_a_known_field_names_the_field_and_its_line() {
        let cases = [
            (
                "node",
                "text",
                "1",
                "`node.r.text` must be a string, not an integer",
            ),
            (
                "node",
                "kind",
                "true",
                "`node.r.kind` must be a string, not a boolean",
            ),
            (
                "node",
                "at",
                "[0]",
                "`node.r.at` must be an array of two integers, not 1 item",
            ),
            (
                "node",
                "at",
                "[]",
                "`node.r.at` must be an array of two integers, not 0 items",
            ),
            (
                "node",
                "at",
                "\"0, 0\"",
                "`node.r.at` must be an array of two integers, not a string",
            ),
            (
                "node",
                "at",
                "[0, 0.5]",
                "`node.r.at[1]` must be an integer, not a float",
            ),
            (
                "node",
                "at",
                "[0, nan]",
                "`node.r.at[1]` must be an integer, not a float",
            ),
            (
                "node",
                "at",
                "[inf, 0]",
                "`node.r.at[0]` must be an integer, not a float",
            ),
            (
                "node",
                "at",
                "[0, 2147483648]",
                "`node.r.at[1]` is out of range: 2147483648",
            ),
            (
                "node",
                "at",
                "[-2147483649, 0]",
                "`node.r.at[0]` is out of range",
            ),
            (
                "node",
                "at",
                "[0, 1979-05-27]",
                "`node.r.at[1]` must be an integer, not a datetime",
            ),
            ("node", "parent", "1", "`node.r.parent` must be a string"),
            (
                "node",
                "shape",
                "[\"rect\"]",
                "`node.r.shape` must be a string, not an array",
            ),
            (
                "node",
                "size",
                "[1, 2, 3]",
                "`node.r.size` must be an array of two integers, not 3 items",
            ),
            (
                "node",
                "size",
                "{ w = 1, h = 1 }",
                "`node.r.size` must be an array of two integers, not a table",
            ),
            (
                "node",
                "style",
                "\"red\"",
                "`node.r.style` must be a table, not a string",
            ),
            (
                "node",
                "style.color",
                "1",
                "`node.r.style.color` must be a string",
            ),
            (
                "node",
                "style.fill",
                "[]",
                "`node.r.style.fill` must be a string",
            ),
            (
                "node",
                "style.stroke",
                "{}",
                "`node.r.style.stroke` must be a string",
            ),
            ("node", "z", "1", "`node.r.z` must be a string"),
            ("edge", "kind", "1", "`edge.r.kind` must be a string"),
            (
                "edge",
                "from",
                "\"a\"",
                "`edge.r.from` must be a table, not a string",
            ),
            (
                "edge",
                "from",
                "{}",
                "`edge.r.from` must have exactly one of `node` and `at`",
            ),
            (
                "edge",
                "from",
                "{ node = \"a\", at = [0, 0] }",
                "`edge.r.from` must have exactly one of `node` and `at`",
            ),
            (
                "edge",
                "from",
                "{ side = \"left\" }",
                "`edge.r.from` must have exactly one of",
            ),
            (
                "edge",
                "from",
                "{ node = 1 }",
                "`edge.r.from.node` must be a string",
            ),
            (
                "edge",
                "from",
                "{ node = \"a\", side = 1 }",
                "`edge.r.from.side` must be a string",
            ),
            (
                "edge",
                "from",
                "{ node = \"a\", anchor = [1] }",
                "`edge.r.from.anchor` must be an array of two integers, not 1 item",
            ),
            (
                "edge",
                "to",
                "{ at = [0, \"x\"] }",
                "`edge.r.to.at[1]` must be an integer, not a string",
            ),
            (
                "edge",
                "heads",
                "\"arrow\"",
                "`edge.r.heads` must be an array of two strings, not a string",
            ),
            (
                "edge",
                "heads",
                "[\"arrow\"]",
                "`edge.r.heads` must be an array of two strings, not 1 item",
            ),
            (
                "edge",
                "heads",
                "[\"none\", 1]",
                "`edge.r.heads[1]` must be a string, not an integer",
            ),
            (
                "edge",
                "labels",
                "{ text = \"x\" }",
                "`edge.r.labels` must be an array of tables, not a table",
            ),
            (
                "edge",
                "labels",
                "[\"x\"]",
                "`edge.r.labels[0]` must be a table, not a string",
            ),
            (
                "edge",
                "labels",
                "[{ text = \"x\" }, { at = 1 }]",
                "`edge.r.labels[1]` has no `text`",
            ),
            (
                "edge",
                "labels",
                "[{ text = 1 }]",
                "`edge.r.labels[0].text` must be a string",
            ),
            (
                "edge",
                "labels",
                "[{ text = \"x\", at = 1.5 }]",
                "`edge.r.labels[0].at` must be an integer, not a float",
            ),
            (
                "edge",
                "labels",
                "[{ text = \"x\", at = 4294967296 }]",
                "`edge.r.labels[0].at` is out of range",
            ),
            ("edge", "route", "1", "`edge.r.route` must be a string"),
            (
                "edge",
                "style",
                "1",
                "`edge.r.style` must be a table, not an integer",
            ),
            (
                "edge",
                "style.color",
                "1",
                "`edge.r.style.color` must be a string",
            ),
            (
                "edge",
                "style.stroke",
                "1",
                "`edge.r.style.stroke` must be a string",
            ),
            ("edge", "z", "1", "`edge.r.z` must be a string"),
        ];
        for (record, field, value, fragment) in cases {
            let (source, line) = with_field(record, field, value);
            let error = err(&source);
            assert!(
                error.message.contains(fragment),
                "{fragment:?} not in {error}\n{source}"
            );
            assert_eq!(error.line, line, "{error}\n{source}");
        }
    }

    #[test]
    fn model_errors_carry_a_line_and_column() {
        let cases = [
            ("[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\n", "the file has keys but no `schema`", 1, 1),
            ("# no schema\nlayout = 1\n", "no `schema`", 1, 1),
            ("schema = \"1\"\n", "`schema` must be an integer, not a string", 1, 1),
            ("# c\n\nschema = 0\n", "`schema` must be 1 or more", 3, 1),
            ("schema = -1\n", "`schema` must be 1 or more", 1, 1),
            ("schema = 1.0\n", "`schema` must be an integer, not a float", 1, 1),
            ("schema = 4294967297\n", "`schema` is out of range", 1, 1),
            ("schema = 1\nrequires = \"frames\"\n", "`requires` must be an array of strings, not a string", 2, 1),
            ("schema = 1\nrequires = [\"a\", 1]\n", "`requires[1]` must be a string, not an integer", 2, 18),
            ("schema = 1\nnode = 3\n", "`node` must be a table, not an integer", 2, 1),
            ("schema = 1\n[[node]]\nkind = \"shape\"\n", "`node` must be a table, not an array", 2, 3),
            ("schema = 1\nedge = []\n", "`edge` must be a table, not an array", 2, 1),
            ("schema = 1\nnode.a = 1\n", "`node.a` must be a table, not an integer", 2, 6),
            ("schema = 1\n\n[node.\"a b\"]\n", "`node.\"a b\"` is not a valid id", 3, 7),
            ("schema = 1\n[node.\"\"]\n", "`node.\"\"` is not a valid id", 2, 7),
            ("schema = 1\n[edge.\"é\"]\n", "`edge.\"é\"` is not a valid id", 2, 7),
            ("schema = 1\n[node.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]\n", "is not a valid id", 2, 7),
            ("schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\n", "`node.a` has no `size`", 3, 7),
            ("schema = 1\n[node.a]\nkind = \"shape\"\nsize = [1, 1]\n", "`node.a` has no `at`", 2, 7),
            // A field after a sub-table header lands in the sub-table.
            ("schema = 1\n[node.a]\nkind = \"shape\"\nsize = [1, 1]\n[node.a.style]\ncolor = \"red\"\nat = [0, 0]\n", "`node.a` has no `at`", 2, 7),
            ("schema = 1\n[edge.e]\nkind = \"edge\"\nto = { node = \"a\" }\n", "`edge.e` has no `from`", 2, 7),
            ("schema = 1\n[edge.e]\nkind = \"edge\"\nfrom = { node = \"a\" }\n", "`edge.e` has no `to`", 2, 7),
            ("schema = 1\n[edge.e]\nfrom.node = \"a\"\nto.node = \"b\"\nto.at = [1, 2]\n", "`edge.e.to` must have exactly one of", 4, 1),
            (
                "schema = 1\n[edge.e]\nfrom = { at = [0, 0] }\nto = { at = [0, 0] }\nlabels = [\n  { text = \"a\" },\n  { at = 3 },\n]\n",
                "`edge.e.labels[1]` has no `text`",
                7,
                3,
            ),
            (
                "schema = 1\n[edge.e]\nfrom = { at = [0, 0] }\nto = { at = [0, 0] }\n[[edge.e.labels]]\ntext = \"a\"\n[[edge.e.labels]]\ntext = 2\n",
                "`edge.e.labels[1].text` must be a string",
                8,
                1,
            ),
            ("\u{feff}schema = 0\n", "`schema` must be 1 or more", 1, 1),
            ("schema = 1\r\n\r\n[node.a]\r\nkind = 1\r\n", "`node.a.kind` must be a string", 4, 1),
            ("schema = 1\n[node.a]\ntext = \"é漢🚀\"; kind = 2\n", "", 3, 0),
        ];
        for (source, fragment, line, column) in cases {
            let error = err(source);
            assert!(
                error.message.contains(fragment),
                "{fragment:?} not in {error}\n{source}"
            );
            assert_eq!(error.line, line, "{error}\n{source}");
            if column > 0 {
                assert_eq!(error.column, column, "{error}\n{source}");
            }
        }
    }

    #[test]
    fn toml_errors_carry_the_parser_position() {
        let cases = [
            ("schema = 1\n[node.a\n", 2),
            ("schema = 1\nschema = 1\n", 2),
            ("schema = 1\n[node.a]\nkind = \"shape\"\n[node.a]\n", 4),
            ("schema = 1\n[node.a]\nat = [0, 0]\nat = [1, 1]\n", 4),
            ("schema = 1\nx = 99999999999999999999\n", 2),
            ("schema = 1\r\nx = 1\r\ny = \r\n", 3),
            ("\u{feff}schema = = 1\n", 1),
            (
                "schema = 1\n<<<<<<< HEAD\nx = 1\n=======\nx = 2\n>>>>>>> theirs\n",
                2,
            ),
            ("schema = 1\n[node.a]\ntext = \"unterminated\n", 3),
            ("schema = 1\nx = \"\u{0}\"\n", 2),
        ];
        for (source, line) in cases {
            let error = err(source);
            assert!(!error.message.is_empty(), "{source}");
            assert_eq!(error.line, line, "{error}\n{source:?}");
            assert!(error.column >= 1, "{error}");
        }
        let error = err("\u{feff}schema = = 1\n");
        assert_eq!(error.column, 10, "{error}");
    }

    #[test]
    fn nesting_deeper_than_80_is_an_error() {
        let nested = |depth: usize| {
            format!(
                "schema = 1\nx = {}{}\n",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        assert!(parse(&nested(80)).is_ok());
        let error = err(&nested(81));
        assert_eq!(error.line, 2, "{error}");
        let tables = format!(
            "schema = 1\nx = {}1{}\n",
            "{ a = ".repeat(81),
            " }".repeat(81)
        );
        assert!(parse(&tables).is_err());
    }

    /// A wrong-typed value in each known field of a valid model, built as
    /// a TOML table rather than text: the model is refused, at that field.
    #[test]
    fn every_known_field_refuses_every_wrong_type() {
        let base: Table = concat!(
            "schema = 1\n",
            "requires = [\"x\"]\n",
            "[node.a]\n",
            "text = \"A\"\nkind = \"shape\"\nat = [0, 0]\nparent = \"b\"\nshape = \"rect\"\nsize = [1, 1]\n",
            "style = { color = \"red\", fill = \"tint\", stroke = \"solid\" }\nz = \"V\"\n",
            "[edge.e]\n",
            "kind = \"edge\"\nfrom = { node = \"a\", side = \"top\", anchor = [0, 0] }\nto = { at = [0, 0] }\n",
            "heads = [\"none\", \"arrow\"]\nlabels = [{ text = \"l\", at = 1 }]\nroute = \"elbow\"\n",
            "style = { color = \"red\", stroke = \"solid\" }\nz = \"W\"\n",
        )
        .parse()
        .unwrap();
        assert!(board(base.clone()).is_ok());

        let date = "d = 1979-05-27".parse::<Table>().unwrap()["d"].clone();
        let s = || Value::String("s".into());
        let i = |n: i64| Value::Integer(n);
        let f = |x: f64| Value::Float(x);
        let a = |items: Vec<Value>| Value::Array(items);
        let t = |pairs: &[(&str, Value)]| {
            Value::Table(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .collect(),
            )
        };
        let common = [
            s(),
            i(1),
            f(1.5),
            f(f64::NAN),
            Value::Boolean(true),
            date.clone(),
            a(vec![]),
            t(&[]),
        ];
        let without = |kind: &str| -> Vec<Value> {
            common
                .iter()
                .filter(|v| v.type_str() != kind)
                .cloned()
                .collect()
        };
        let mut string = without("string");
        string.push(a(vec![s()]));
        let mut int = without("integer");
        int.extend([
            i(i64::from(i32::MAX) + 1),
            i(i64::from(i32::MIN) - 1),
            i(i64::MAX),
            i(i64::MIN),
        ]);
        let mut pair = without("array");
        pair.extend([
            i(1),
            a(vec![i(1)]),
            a(vec![i(1), i(2), i(3)]),
            a(vec![i(1), f(1.0)]),
            a(vec![f(f64::INFINITY), i(1)]),
            a(vec![i(1), s()]),
            a(vec![i(1), i(1 << 40)]),
            a(vec![i(1), date.clone()]),
            a(vec![a(vec![i(1), i(2)]), i(1)]),
        ]);
        let mut table = without("table");
        table.push(a(vec![t(&[("k", s())])]));
        let mut strings = without("array");
        strings.extend([
            a(vec![i(1)]),
            a(vec![s(), Value::Boolean(false)]),
            a(vec![t(&[])]),
        ]);
        let mut heads = without("array");
        heads.extend([
            a(vec![s()]),
            a(vec![s(), s(), s()]),
            a(vec![s(), i(1)]),
            a(vec![date.clone(), s()]),
        ]);
        let mut labels = without("array");
        labels.extend([
            a(vec![s()]),
            a(vec![t(&[("at", i(1))])]),
            a(vec![t(&[("text", i(1))])]),
            a(vec![t(&[("text", s()), ("at", f(1.0))])]),
            a(vec![t(&[("text", s())]), t(&[("text", s()), ("at", s())])]),
        ]);
        let mut end = without("table");
        end.extend([
            t(&[]),
            t(&[("node", s()), ("at", a(vec![i(0), i(0)]))]),
            t(&[("side", s())]),
        ]);
        let mut schema = int.clone();
        schema.extend([i(0), i(-1)]);

        use Step::{Item, Key};
        let k =
            |names: &[&str]| -> Vec<Step> { names.iter().map(|n| Key((*n).to_owned())).collect() };
        let mut label = k(&["edge", "e", "labels"]);
        label.push(Item(0));
        let fields: Vec<(Vec<Step>, &Vec<Value>)> = vec![
            (k(&["schema"]), &schema),
            (k(&["requires"]), &strings),
            (k(&["node"]), &table),
            (k(&["edge"]), &table),
            (k(&["node", "a"]), &table),
            (k(&["edge", "e"]), &table),
            (k(&["node", "a", "text"]), &string),
            (k(&["node", "a", "kind"]), &string),
            (k(&["node", "a", "at"]), &pair),
            (k(&["node", "a", "parent"]), &string),
            (k(&["node", "a", "shape"]), &string),
            (k(&["node", "a", "size"]), &pair),
            (k(&["node", "a", "style"]), &table),
            (k(&["node", "a", "style", "color"]), &string),
            (k(&["node", "a", "style", "fill"]), &string),
            (k(&["node", "a", "style", "stroke"]), &string),
            (k(&["node", "a", "z"]), &string),
            (k(&["edge", "e", "kind"]), &string),
            (k(&["edge", "e", "from"]), &end),
            (k(&["edge", "e", "to"]), &end),
            (k(&["edge", "e", "from", "node"]), &string),
            (k(&["edge", "e", "from", "side"]), &string),
            (k(&["edge", "e", "from", "anchor"]), &pair),
            (k(&["edge", "e", "to", "at"]), &pair),
            (k(&["edge", "e", "heads"]), &heads),
            (k(&["edge", "e", "labels"]), &labels),
            ([label.clone(), k(&["text"])].concat(), &string),
            ([label.clone(), k(&["at"])].concat(), &int),
            (k(&["edge", "e", "route"]), &string),
            (k(&["edge", "e", "style"]), &table),
            (k(&["edge", "e", "style", "color"]), &string),
            (k(&["edge", "e", "style", "stroke"]), &string),
            (k(&["edge", "e", "z"]), &string),
        ];
        for (path, values) in fields {
            for value in values {
                let mut top = base.clone();
                set(&mut top, &path, value.clone());
                let fault = board(top).expect_err(&format!("{} = {value:?}", dotted(&path)));
                assert!(
                    fault.path.starts_with(&path),
                    "{} = {value:?} faulted at {}",
                    dotted(&path),
                    dotted(&fault.path)
                );
            }
        }
    }

    fn set(table: &mut Table, path: &[Step], value: Value) {
        let mut slot: Option<&mut Value> = None;
        let mut here = Some(table);
        let (last, steps) = path.split_last().unwrap();
        for step in steps {
            let next = match (step, here.take(), slot.take()) {
                (Step::Key(key), Some(table), _) => table.get_mut(key.as_str()).unwrap(),
                (Step::Key(key), None, Some(Value::Table(table))) => {
                    table.get_mut(key.as_str()).unwrap()
                }
                (Step::Item(index), None, Some(Value::Array(items))) => &mut items[*index],
                other => panic!("no path at {step:?}: {other:?}"),
            };
            slot = Some(next);
        }
        let Step::Key(key) = last else {
            panic!("path ends in an item");
        };
        let table = match (here, slot) {
            (Some(table), _) => table,
            (None, Some(Value::Table(table))) => table,
            other => panic!("no table for {key}: {other:?}"),
        };
        table.insert(key.clone(), value);
    }

    /// One to four byte and line mutations of a committed board.
    fn mutate(rng: &mut Rng, files: &[&str]) -> String {
        const BYTES: &[u8] = b"[]{}\"'=.,#\n\r\\ -+0129aez_\x00\x7f\xc3\xa9\xff";
        let mut bytes = files[rng.below(files.len())].as_bytes().to_vec();
        for _ in 0..=rng.below(4) {
            let mut lines: Vec<Vec<u8>> =
                bytes.split(|&b| b == b'\n').map(<[u8]>::to_vec).collect();
            match rng.below(7) {
                0 if !bytes.is_empty() => {
                    let at = rng.below(bytes.len());
                    bytes[at] = BYTES[rng.below(BYTES.len())];
                    continue;
                }
                1 => {
                    let at = rng.below(bytes.len() + 1);
                    bytes.insert(at, BYTES[rng.below(BYTES.len())]);
                    continue;
                }
                2 if !bytes.is_empty() => {
                    let at = rng.below(bytes.len());
                    let end = (at + 1 + rng.below(8)).min(bytes.len());
                    bytes.drain(at..end);
                    continue;
                }
                3 => {
                    let at = rng.below(lines.len());
                    lines.remove(at);
                }
                4 => {
                    let at = rng.below(lines.len());
                    let line = lines[at].clone();
                    lines.insert(rng.below(lines.len() + 1), line);
                }
                5 => {
                    let (a, b) = (rng.below(lines.len()), rng.below(lines.len()));
                    lines.swap(a, b);
                }
                _ => {
                    let other = files[rng.below(files.len())];
                    let donor: Vec<&str> = other.lines().collect();
                    if !donor.is_empty() {
                        let line = donor[rng.below(donor.len())].as_bytes().to_vec();
                        lines.insert(rng.below(lines.len() + 1), line);
                    }
                }
            }
            bytes = lines.join(&b'\n');
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn survives_mutations(count: usize, seed: u64) {
        let files: Vec<&str> = fixtures::all().map(|(_, text)| text).collect();
        let mut rng = Rng(seed);
        let (mut loaded, mut refused) = (0, 0);
        for _ in 0..count {
            let source = mutate(&mut rng, &files);
            match parse(&source) {
                Ok(board) => {
                    loaded += 1;
                    let issues = crate::check(&board);
                    assert!(issues.iter().all(|issue| {
                        board.nodes.contains_key(issue.id()) || board.edges.contains_key(issue.id())
                    }));
                    let _ = (crate::access(&board), crate::placements(&board));
                    let palette = crate::style::tests::LIGHT;
                    let scene = crate::scene::build(&board, &palette, &crate::scene::FixedMeasure);
                    crate::svg::write(&scene, &crate::svg::Options::default());
                    let written = crate::write(&board);
                    let again = parse(&written).unwrap_or_else(|e| panic!("{e}\n{written}"));
                    assert_eq!(crate::write(&again), written, "{source}");
                }
                Err(error) => {
                    refused += 1;
                    let lines = source.split('\n').count();
                    assert!((1..=lines).contains(&error.line), "{error}\n{source}");
                    assert!(error.column >= 1, "{error}\n{source}");
                    assert!(!error.message.is_empty(), "{source}");
                }
            }
        }
        // Both outcomes are exercised, or the mutations are too weak or
        // too strong to test anything.
        assert!(
            loaded > count / 20 && refused > count / 20,
            "{loaded} loaded, {refused} refused"
        );
    }

    #[test]
    fn parse_survives_mutations() {
        survives_mutations(2_000, 1);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_parse_survives_mutations() {
        survives_mutations(100_000, 2);
    }
}
