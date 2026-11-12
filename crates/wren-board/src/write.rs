//! The canonical writer (§4.11 of the board plan): a [`Board`] as the one
//! text every build writes for it.
//!
//! Every field is one physical line, strings included, so editing one field
//! of one element changes one line of the file. The order of blocks and of
//! keys is a plain bytewise sort that never depends on whether this build
//! knows a key, so builds of different ages write the same bytes for the
//! same model. Hand-written on purpose: the `toml` crate's serialiser
//! explodes arrays and writes sub-table headers.

use std::borrow::Cow;
use std::fmt;
use std::fmt::Write as _;

use toml::value::Datetime;
use toml::{Table, Value};

use crate::{Board, Color, Edge, End, Label, Node, Token, Vocabulary};

/// The keys a typed field of the model owns. An unknown-content key of the
/// same name can only come from a model built in code, never from a file;
/// the typed field wins, so the output always reads back.
const TOP: &[&str] = &["schema", "requires", "node", "edge"];
const NODE: &[&str] = &[
    "text", "kind", "at", "parent", "shape", "size", "style", "z",
];
const NODE_STYLE: &[&str] = &["color", "fill", "stroke"];
const EDGE: &[&str] = &[
    "kind", "from", "to", "heads", "labels", "route", "style", "z",
];
const EDGE_STYLE: &[&str] = &["color", "stroke"];
const NODE_END: &[&str] = &["node", "side", "anchor", "at"];
const FREE_END: &[&str] = &["node", "at"];
const LABEL: &[&str] = &["text", "at"];

/// Writes a board in canonical form. The output is a function of the model
/// alone, and `write(&parse(&write(b))?) == write(b)`.
pub fn write(board: &Board) -> String {
    let mut out = String::with_capacity(64 + 160 * (board.nodes.len() + board.edges.len()));
    put(&mut out, format_args!("schema = {}\n", board.schema));
    if !board.requires.is_empty() {
        let mut requires: Vec<&str> = board.requires.iter().map(String::as_str).collect();
        requires.sort_unstable();
        out.push_str("requires = [");
        for (i, name) in requires.into_iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            string(&mut out, name);
        }
        out.push_str("]\n");
    }
    for (key, field) in merged(Vec::new(), &board.extra, TOP) {
        line(&mut out, "", key, &field);
    }
    for (id, node) in &board.nodes {
        put(&mut out, format_args!("\n[node.{id}]\n"));
        node_fields(&mut out, node);
    }
    for (id, edge) in &board.edges {
        put(&mut out, format_args!("\n[edge.{id}]\n"));
        edge_fields(&mut out, edge);
    }
    out
}

/// A value on its way out: a typed field of the model, or unknown content
/// as read.
enum Field<'a> {
    Raw(&'a Value),
    Str(Cow<'a, str>),
    Int(i32),
    Ints([i32; 2]),
    Strs([&'a str; 2]),
    /// Keys in bytewise order. Written as dotted keys, one leaf per line;
    /// omitted when empty.
    Table(Vec<(&'a str, Field<'a>)>),
    /// Keys in bytewise order, always one inline table.
    Inline(Vec<(&'a str, Field<'a>)>),
    /// An array of tables: one item per line.
    Items(Vec<Field<'a>>),
}

fn node_fields(out: &mut String, node: &Node) {
    if !node.text.is_empty() {
        out.push_str("text = ");
        string(out, &node.text);
        out.push('\n');
    }
    if let Some(kind) = &node.kind {
        out.push_str("kind = ");
        string(out, kind.as_str());
        out.push('\n');
    }
    let mut known = vec![
        ("at", Field::Ints(node.at)),
        ("size", Field::Ints(node.size)),
    ];
    if let Some(parent) = &node.parent {
        known.push(("parent", Field::Str(Cow::Borrowed(parent))));
    }
    if node.shape.as_str() != "rect" {
        known.push(("shape", Field::Str(Cow::Borrowed(node.shape.as_str()))));
    }
    let style = &node.style;
    let mut style_known = Vec::new();
    let color = color(&style.color);
    if color != Token::Neutral.name() {
        style_known.push(("color", Field::Str(color)));
    }
    if style.fill.as_str() != "tint" {
        style_known.push(("fill", Field::Str(Cow::Borrowed(style.fill.as_str()))));
    }
    if style.stroke.as_str() != "solid" {
        style_known.push(("stroke", Field::Str(Cow::Borrowed(style.stroke.as_str()))));
    }
    known.push((
        "style",
        Field::Table(merged(style_known, &style.extra, NODE_STYLE)),
    ));
    if let Some(z) = &node.z {
        known.push(("z", Field::Str(Cow::Borrowed(z))));
    }
    for (key, field) in merged(known, &node.extra, NODE) {
        line(out, "", key, &field);
    }
}

fn edge_fields(out: &mut String, edge: &Edge) {
    if let Some(kind) = &edge.kind {
        out.push_str("kind = ");
        string(out, kind.as_str());
        out.push('\n');
    }
    line(out, "", "from", &end(&edge.from));
    line(out, "", "to", &end(&edge.to));
    let mut known = Vec::new();
    let heads = [edge.heads[0].as_str(), edge.heads[1].as_str()];
    if heads != ["none", "arrow"] {
        known.push(("heads", Field::Strs(heads)));
    }
    if !edge.labels.is_empty() {
        known.push((
            "labels",
            Field::Items(edge.labels.iter().map(label).collect()),
        ));
    }
    if edge.route.as_str() != "elbow" {
        known.push(("route", Field::Str(Cow::Borrowed(edge.route.as_str()))));
    }
    let style = &edge.style;
    let mut style_known = Vec::new();
    let color = color(&style.color);
    if color != Token::Neutral.name() {
        style_known.push(("color", Field::Str(color)));
    }
    if style.stroke.as_str() != "solid" {
        style_known.push(("stroke", Field::Str(Cow::Borrowed(style.stroke.as_str()))));
    }
    known.push((
        "style",
        Field::Table(merged(style_known, &style.extra, EDGE_STYLE)),
    ));
    if let Some(z) = &edge.z {
        known.push(("z", Field::Str(Cow::Borrowed(z))));
    }
    for (key, field) in merged(known, &edge.extra, EDGE) {
        line(out, "", key, &field);
    }
}

fn end(end: &End) -> Field<'_> {
    match end {
        End::Node {
            node,
            side,
            anchor,
            extra,
        } => {
            let mut known = vec![("node", Field::Str(Cow::Borrowed(node.as_str())))];
            if let Some(side) = side {
                known.push(("side", Field::Str(Cow::Borrowed(side.as_str()))));
            }
            if let Some(anchor) = anchor {
                known.push(("anchor", Field::Ints(*anchor)));
            }
            Field::Inline(merged(known, extra, NODE_END))
        }
        End::Free { at, extra } => {
            Field::Inline(merged(vec![("at", Field::Ints(*at))], extra, FREE_END))
        }
    }
}

fn label(label: &Label) -> Field<'_> {
    let mut known = vec![("text", Field::Str(Cow::Borrowed(label.text.as_str())))];
    if label.at != Label::DEFAULT_AT {
        known.push(("at", Field::Int(label.at)));
    }
    Field::Inline(merged(known, &label.extra, LABEL))
}

/// A colour as written: a token's name, a literal in lower case, or any
/// other string as it was read.
fn color(color: &Color) -> Cow<'_, str> {
    match color {
        Color::Token(token) => Cow::Borrowed(token.name()),
        Color::Hex([r, g, b]) => Cow::Owned(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Unknown(text) => match Color::read(text.clone()) {
            Color::Unknown(_) => Cow::Borrowed(text),
            read => Cow::Owned(self::color(&read).into_owned()),
        },
    }
}

/// The known fields and the unknown keys together, in bytewise key order.
fn merged<'a>(
    mut fields: Vec<(&'a str, Field<'a>)>,
    extra: &'a Table,
    reserved: &[&str],
) -> Vec<(&'a str, Field<'a>)> {
    fields.extend(
        extra
            .iter()
            .filter(|(key, _)| !reserved.contains(&key.as_str()))
            .map(|(key, value)| (key.as_str(), Field::Raw(value))),
    );
    fields.sort_unstable_by_key(|&(key, _)| key);
    fields
}

/// A table's entries in bytewise key order, whatever order the map keeps.
fn sorted(table: &Table) -> Vec<(&String, &Value)> {
    let mut entries: Vec<(&String, &Value)> = table.iter().collect();
    entries.sort_unstable_by_key(|&(key, _)| key);
    entries
}

/// One field of a block, or several lines for a table written as dotted
/// keys or an array of tables. `prefix` is the dotted path so far, ending
/// in `.` when not empty.
fn line(out: &mut String, prefix: &str, name: &str, field: &Field<'_>) {
    match field {
        Field::Table(fields) => {
            let prefix = dotted(prefix, name);
            for (key, field) in fields {
                line(out, &prefix, key, field);
            }
        }
        Field::Raw(Value::Table(table)) if !table.is_empty() => {
            let prefix = dotted(prefix, name);
            for (key, value) in sorted(table) {
                line(out, &prefix, key, &Field::Raw(value));
            }
        }
        Field::Raw(Value::Array(items)) if items.iter().any(Value::is_table) => {
            start(out, prefix, name);
            out.push_str("[\n");
            for item in items {
                out.push_str("  ");
                inline(out, item);
                out.push_str(",\n");
            }
            out.push_str("]\n");
        }
        Field::Items(items) => {
            start(out, prefix, name);
            out.push_str("[\n");
            for item in items {
                out.push_str("  ");
                inline_field(out, item);
                out.push_str(",\n");
            }
            out.push_str("]\n");
        }
        other => {
            start(out, prefix, name);
            inline_field(out, other);
            out.push('\n');
        }
    }
}

fn dotted(prefix: &str, name: &str) -> String {
    let mut out = String::from(prefix);
    key(&mut out, name);
    out.push('.');
    out
}

fn start(out: &mut String, prefix: &str, name: &str) {
    out.push_str(prefix);
    key(out, name);
    out.push_str(" = ");
}

fn inline_field(out: &mut String, field: &Field<'_>) {
    match field {
        Field::Raw(value) => inline(out, value),
        Field::Str(text) => string(out, text),
        Field::Int(n) => put(out, format_args!("{n}")),
        Field::Ints([a, b]) => put(out, format_args!("[{a}, {b}]")),
        Field::Strs([a, b]) => {
            out.push('[');
            string(out, a);
            out.push_str(", ");
            string(out, b);
            out.push(']');
        }
        Field::Table(fields) | Field::Inline(fields) => {
            if fields.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{ ");
            for (i, (name, field)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                key(out, name);
                out.push_str(" = ");
                inline_field(out, field);
            }
            out.push_str(" }");
        }
        Field::Items(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                inline_field(out, item);
            }
            out.push(']');
        }
    }
}

/// Any value on one line, tables as inline tables.
fn inline(out: &mut String, value: &Value) {
    match value {
        Value::String(text) => string(out, text),
        Value::Integer(n) => put(out, format_args!("{n}")),
        Value::Float(x) => float(out, *x),
        Value::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Datetime(moment) => datetime(out, moment),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                inline(out, item);
            }
            out.push(']');
        }
        Value::Table(table) => {
            if table.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{ ");
            for (i, (name, value)) in sorted(table).into_iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                key(out, name);
                out.push_str(" = ");
                inline(out, value);
            }
            out.push_str(" }");
        }
    }
}

/// A float as Rust's `{:?}` writes it, which TOML reads back to the same
/// bits; `{}` would write `1.0` as `1`, which reads back as an integer.
fn float(out: &mut String, x: f64) {
    if x.is_nan() {
        out.push_str("nan");
    } else if x.is_infinite() {
        out.push_str(if x > 0.0 { "inf" } else { "-inf" });
    } else {
        put(out, format_args!("{x:?}"));
    }
}

/// A date or time as the `toml` crate displays it, except that a time
/// always has seconds: TOML 1.1 may omit them and TOML 1.0 may not.
fn datetime(out: &mut String, moment: &Datetime) {
    let mut moment = *moment;
    if let Some(time) = &mut moment.time {
        time.second.get_or_insert(0);
    }
    put(out, format_args!("{moment}"));
}

/// A key, bare when it can be and a basic string otherwise.
fn key(out: &mut String, name: &str) {
    let bare = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare {
        out.push_str(name);
    } else {
        string(out, name);
    }
}

/// A single-line basic string: a line break is `\n`, and every other
/// control character is escaped too, so no line of the file holds one.
fn string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{0}'..='\u{1f}' | '\u{7f}' => put(out, format_args!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Formats into the output; formatting into a `String` cannot fail.
fn put(out: &mut String, args: fmt::Arguments<'_>) {
    let _ = out.write_fmt(args);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, Rng};
    use crate::{
        parse, Dash, EdgeKind, EdgeStyle, Extra, Fill, Head, Id, Named, NodeKind, NodeStyle, Route,
        Shape, Side,
    };

    #[track_caller]
    fn read(text: &str) -> Board {
        parse(text).unwrap_or_else(|error| panic!("{error}\n{text}"))
    }

    /// Test 1: each canonical file is its own canonical form.
    #[test]
    fn canonical_files_write_to_themselves() {
        for (name, text) in fixtures::CANONICAL {
            fixtures::assert_golden(name, text, &write(&read(text)));
        }
    }

    /// Test 2: each non-canonical input writes to exactly its output.
    #[test]
    fn non_canonical_inputs_write_to_their_outputs() {
        for (name, input, output) in fixtures::PAIRS {
            let out = format!("{name}.out");
            fixtures::assert_golden(&out, output, &write(&read(input)));
        }
        let (name, text) = fixtures::HAND_WRITTEN;
        let canonical = write(&read(text));
        assert_ne!(canonical, text, "{name}");
        assert_eq!(write(&read(&canonical)), canonical, "{name}");
    }

    #[test]
    fn the_empty_board_is_the_template() {
        assert_eq!(write(&Board::default()), crate::EMPTY);
        assert_eq!(write(&read("")), crate::EMPTY);
    }

    /// TOML 1.1 lets a time leave out its seconds; TOML 1.0 does not, so
    /// they are written.
    #[test]
    fn a_time_without_seconds_gains_them() {
        let board = read("schema = 1\nt = 07:32\nd = 1979-05-27T07:32Z\nl = 1979-05-27 07:32\n");
        assert_eq!(
            write(&board),
            "schema = 1\nd = 1979-05-27T07:32:00Z\nl = 1979-05-27T07:32:00\nt = 07:32:00\n"
        );
    }

    #[test]
    fn literal_colours_are_written_lower_case() {
        let plain = "schema = 1\n\n[node.a]\nat = [0, 0]\nsize = [1, 1]\n";
        let mut board = read(plain);
        let node = board.nodes.get_mut("a").unwrap();
        node.style.color = Color::Unknown("#ABCDEF".into());
        assert!(write(&board).ends_with("style.color = \"#abcdef\"\n"));
        let node = board.nodes.get_mut("a").unwrap();
        node.style.color = Color::Unknown("neutral".into());
        node.shape = Named::Other("rect".into());
        assert_eq!(write(&board), plain);
    }

    /// A key of unknown content that a typed field owns can only come from
    /// a model built in code; the typed field wins.
    #[test]
    fn a_typed_field_owns_its_key() {
        let mut board = read(fixtures::CANONICAL[0].1);
        let node = board.nodes.values_mut().next().unwrap();
        node.extra.insert("at".into(), Value::Integer(1));
        node.extra.insert("text".into(), Value::Integer(1));
        node.style.extra.insert("fill".into(), Value::Integer(1));
        let edge = board.edges.values_mut().next().unwrap();
        if let End::Node { extra, .. } = &mut edge.from {
            extra.insert("at".into(), Value::Integer(1));
        }
        board.extra.insert("node".into(), Value::Integer(1));
        assert_eq!(write(&board), fixtures::CANONICAL[0].1);
    }

    /// The lines of a file that hold a field: not headers, empty lines, or
    /// the items of a multi-line array.
    fn blocks(text: &str) -> Vec<(String, Vec<Vec<String>>)> {
        let mut blocks = vec![(String::new(), Vec::new())];
        for line in text.lines() {
            if line.starts_with('[') {
                blocks.push((line.to_owned(), Vec::new()));
            } else if !line.is_empty() && !line.starts_with("  ") && line != "]" {
                if let Some(block) = blocks.last_mut() {
                    block.1.push(key_path(line));
                }
            }
        }
        blocks
    }

    /// The dotted key at the start of a canonical line, each part decoded.
    fn key_path(line: &str) -> Vec<String> {
        let mut path = Vec::new();
        let mut chars = line.chars().peekable();
        loop {
            let mut part = String::new();
            if chars.peek() == Some(&'"') {
                chars.next();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => part.push('\n'),
                            Some('t') => part.push('\t'),
                            Some('r') => part.push('\r'),
                            Some('u') => {
                                let hex: String = chars.by_ref().take(4).collect();
                                let code = u32::from_str_radix(&hex, 16).unwrap();
                                part.push(char::from_u32(code).unwrap());
                            }
                            Some(other) => part.push(other),
                            None => panic!("{line}"),
                        },
                        c => part.push(c),
                    }
                }
            } else {
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == ' ' {
                        break;
                    }
                    part.push(c);
                    chars.next();
                }
            }
            path.push(part);
            match chars.next() {
                Some('.') => {}
                Some(' ') => return path,
                other => panic!("{other:?} after a key in {line:?}"),
            }
        }
    }

    /// Test 6: in every block, the first keys come first and every other
    /// key follows in a plain bytewise sort of its parts, computed here
    /// with no list of the keys the writer knows.
    #[test]
    fn every_block_is_in_key_order() {
        let mut texts: Vec<(&str, String)> = fixtures::CANONICAL
            .iter()
            .map(|&(name, text)| (name, text.to_owned()))
            .collect();
        let mut rng = Rng(11);
        for _ in 0..50 {
            texts.push(("random", write(&random_board(&mut rng))));
        }
        for (name, text) in &texts {
            for (header, paths) in blocks(text) {
                let first: &[&str] = if header.is_empty() {
                    &["schema", "requires"]
                } else if header.starts_with("[node.") {
                    &["text", "kind"]
                } else {
                    &["kind", "from", "to"]
                };
                let lead = paths
                    .iter()
                    .take_while(|path| path.len() == 1 && first.contains(&path[0].as_str()))
                    .count();
                let leading: Vec<&str> = paths[..lead].iter().map(|p| p[0].as_str()).collect();
                let mut expected: Vec<&str> = first
                    .iter()
                    .copied()
                    .filter(|k| leading.contains(k))
                    .collect();
                expected.dedup();
                assert_eq!(leading, expected, "{name} {header}");
                let rest = &paths[lead..];
                for pair in rest.windows(2) {
                    assert!(
                        pair[0] < pair[1],
                        "{name} {header}: {:?} then {:?}",
                        pair[0],
                        pair[1]
                    );
                }
                if header.starts_with("[node.") || header.starts_with("[edge.") {
                    assert!(
                        rest.iter().all(|p| !first.contains(&p[0].as_str())),
                        "{name} {header}: a first key after the others"
                    );
                }
            }
        }
    }

    /// The lines one text has and the other has not, by trimming the
    /// common head and tail.
    fn changed<'a>(before: &'a str, after: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
        let a: Vec<&str> = before.lines().collect();
        let b: Vec<&str> = after.lines().collect();
        let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let room = a.len().min(b.len()) - head;
        let tail = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take(room)
            .take_while(|(x, y)| x == y)
            .count();
        (
            a[head..a.len() - tail].to_vec(),
            b[head..b.len() - tail].to_vec(),
        )
    }

    #[track_caller]
    fn one_line(before: &str, after: &str, what: &str) {
        let (removed, added) = changed(before, after);
        assert_eq!(
            (removed.len(), added.len()),
            (1, 1),
            "{what}: -{removed:?} +{added:?}"
        );
    }

    #[track_caller]
    fn one_line_or_one_more(before: &str, after: &str, what: &str) {
        let (removed, added) = changed(before, after);
        assert!(
            removed.len() <= 1 && added.len() == 1,
            "{what}: -{removed:?} +{added:?}"
        );
    }

    /// Tests 4 and 6: editing one field of one element changes one line;
    /// a field that was at its default, and so absent, gains one.
    #[test]
    fn one_edit_changes_one_line() {
        for (name, text) in fixtures::CANONICAL {
            let board = read(text);
            let before = write(&board);
            for id in board.nodes.keys() {
                let edit = |change: &dyn Fn(&mut Node)| {
                    let mut copy = board.clone();
                    change(copy.nodes.get_mut(id).unwrap());
                    write(&copy)
                };
                let what = |field: &str| format!("{name} node {id} {field}");
                one_line(&before, &edit(&|n| n.at[0] += 20), &what("move"));
                one_line(
                    &before,
                    &edit(&|n| n.size = [n.size[0] + 40, n.size[1]]),
                    &what("resize"),
                );
                let relabel = edit(&|n| n.text = format!("{}\nand more", n.text));
                one_line_or_one_more(&before, &relabel, &what("relabel"));
                let recolour = edit(&|n| {
                    n.style.color = match n.style.color {
                        Color::Token(Token::Green) => Color::Token(Token::Blue),
                        _ => Color::Token(Token::Green),
                    }
                });
                one_line_or_one_more(&before, &recolour, &what("recolour"));
                let restack = edit(&|n| n.z = Some(format!("{}z", n.z.as_deref().unwrap_or("V"))));
                one_line_or_one_more(&before, &restack, &what("restack"));
            }
            for id in board.edges.keys() {
                let edit = |change: &dyn Fn(&mut Edge)| {
                    let mut copy = board.clone();
                    change(copy.edges.get_mut(id).unwrap());
                    write(&copy)
                };
                let what = |field: &str| format!("{name} edge {id} {field}");
                let restack = edit(&|e| e.z = Some(format!("{}z", e.z.as_deref().unwrap_or("V"))));
                one_line_or_one_more(&before, &restack, &what("restack"));
                let recolour = edit(&|e| {
                    e.style.color = match e.style.color {
                        Color::Token(Token::Red) => Color::Token(Token::Blue),
                        _ => Color::Token(Token::Red),
                    }
                });
                one_line_or_one_more(&before, &recolour, &what("recolour"));
                if !board.edges[id].labels.is_empty() {
                    let relabel = edit(&|e| e.labels[0].text.push_str("\nmore"));
                    one_line(&before, &relabel, &what("relabel"));
                }
                if let End::Node { .. } = board.edges[id].to {
                    let rebind = edit(&|e| {
                        if let End::Node { side, .. } = &mut e.to {
                            *side = Some(Named::Known(Side::Bottom));
                        }
                    });
                    if rebind != before {
                        one_line(&before, &rebind, &what("rebind"));
                    }
                }
            }
        }
    }

    #[test]
    fn adding_or_deleting_an_element_adds_or_removes_one_block() {
        let board = read(fixtures::CANONICAL[0].1);
        let before = write(&board);
        let node_id = board.nodes.keys().nth(3).unwrap().clone();
        let edge_id = board.edges.keys().nth(3).unwrap().clone();

        let mut added = board.clone();
        let mut node = board.nodes[&node_id].clone();
        node.at[0] += 400;
        added.nodes.insert(Id::new("m0000000").unwrap(), node);
        let edge = board.edges[&edge_id].clone();
        let mut both = added.clone();
        both.edges.insert(Id::new("m0000001").unwrap(), edge);
        for (after, header) in [(&added, "[node.m0000000]"), (&both, "[edge.m0000001]")] {
            let previous = if header.starts_with("[edge") {
                write(&added)
            } else {
                before.clone()
            };
            let block = write(after);
            let (removed, new) = changed(&previous, &block);
            assert!(removed.is_empty(), "{removed:?}");
            assert!(new.contains(&header) && new.contains(&""), "{new:?}");
            let fields = block
                .split(&format!("{header}\n"))
                .nth(1)
                .unwrap()
                .split("\n\n")
                .next()
                .unwrap()
                .lines()
                .count();
            assert_eq!(new.len(), fields + 2, "{new:?}");
        }

        let mut deleted = board.clone();
        let gone = deleted.nodes.remove(&node_id).unwrap();
        let without = write(&deleted);
        let (removed, new) = changed(&before, &without);
        assert!(new.is_empty(), "{new:?}");
        assert!(
            removed.contains(&format!("[node.{node_id}]").as_str()),
            "{removed:?}"
        );
        let mut alone = Board::default();
        alone.nodes.insert(node_id.clone(), gone);
        // The alone board's lines, less its `schema` line.
        assert_eq!(
            removed.len(),
            write(&alone).lines().count() - 1,
            "{removed:?}"
        );
    }

    const POOL: &[&str] = &[
        "\"", "\\", "\n", "\t", "\r", "\u{0}", "\u{1f}", "\u{7f}", "[", "]", "#", "=", "'", " ",
        "a", "é", "→", "🚀", "\u{a0}", "\u{feff}", "<<<<<<<", ".", "\"\"\"", "e\u{301}",
    ];

    fn random_string(rng: &mut Rng, carriage: bool) -> String {
        let mut text = String::new();
        for _ in 0..rng.below(13) {
            let piece = *rng.pick(POOL);
            if carriage || piece != "\r" {
                text.push_str(piece);
            }
        }
        text
    }

    fn random_id(rng: &mut Rng) -> Id {
        const CHARS: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyzABZ_-";
        let longest = if rng.one_in(8) { Id::MAX_LEN } else { 8 };
        let len = 1 + rng.below(longest);
        let text: String = (0..len).map(|_| char::from(*rng.pick(CHARS))).collect();
        Id::new(&text).unwrap()
    }

    fn random_key(rng: &mut Rng) -> String {
        if rng.one_in(3) {
            random_string(rng, true)
        } else {
            (*rng.pick(&[
                "ext", "a", "zz", "style", "at", "kind", "x-y", "k_9", "Z", "from",
            ]))
            .to_owned()
        }
    }

    const FLOATS: &[f64] = &[
        0.0,
        -0.0,
        1.0,
        0.25,
        0.1,
        1e15,
        1e16,
        1e21,
        1e-5,
        1.5e-7,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
        -1.5,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];

    fn random_value(rng: &mut Rng, depth: usize) -> Value {
        let moments = [
            "1979-05-27T07:32:00Z",
            "1979-05-27",
            "07:32:00.25",
            "07:32",
            "1979-05-27T07:32:00.5-08:00",
        ];
        match rng.below(if depth > 3 { 6 } else { 8 }) {
            0 => Value::String(random_string(rng, true)),
            1 => Value::Integer(rng.next() as i64 >> rng.below(64)),
            2 => Value::Float(if rng.one_in(2) {
                *rng.pick(FLOATS)
            } else {
                f64::from_bits(rng.next())
            }),
            3 => Value::Boolean(rng.one_in(2)),
            4 => Value::Datetime(rng.pick(&moments).parse().unwrap()),
            5 => Value::String(String::new()),
            6 => Value::Array(
                (0..rng.below(4))
                    .map(|_| random_value(rng, depth + 1))
                    .collect(),
            ),
            _ => Value::Table(random_table(rng, depth + 1)),
        }
    }

    fn random_table(rng: &mut Rng, depth: usize) -> Table {
        (0..rng.below(4))
            .map(|_| (random_key(rng), random_value(rng, depth)))
            .collect()
    }

    fn extra(rng: &mut Rng) -> Extra {
        if rng.one_in(3) {
            random_table(rng, 1)
        } else {
            Extra::new()
        }
    }

    fn named<T: Vocabulary>(rng: &mut Rng) -> Named<T> {
        if rng.one_in(5) {
            Named::Other(random_string(rng, true))
        } else {
            Named::Known(*rng.pick(T::ALL))
        }
    }

    fn int(rng: &mut Rng) -> i32 {
        match rng.below(4) {
            0 => *rng.pick(&[0, 1, -1, 500, i32::MAX, i32::MIN]),
            1 => rng.next() as i32,
            _ => rng.below(2000) as i32 - 500,
        }
    }

    fn color(rng: &mut Rng) -> Color {
        match rng.below(4) {
            0 => Color::Hex([rng.next() as u8, rng.next() as u8, rng.next() as u8]),
            1 => Color::Unknown(if rng.one_in(2) {
                "#ABCdef".into()
            } else {
                random_string(rng, true)
            }),
            _ => Color::Token(*rng.pick(Token::ALL)),
        }
    }

    fn end(rng: &mut Rng) -> End {
        if rng.one_in(4) {
            End::Free {
                at: [int(rng), int(rng)],
                extra: extra(rng),
            }
        } else {
            End::Node {
                node: random_string(rng, true),
                side: rng.one_in(2).then(|| named(rng)),
                anchor: rng.one_in(3).then(|| [int(rng), int(rng)]),
                extra: extra(rng),
            }
        }
    }

    fn random_board(rng: &mut Rng) -> Board {
        let mut board = Board {
            schema: if rng.one_in(8) {
                1 + rng.below(1000) as i32
            } else {
                1
            },
            requires: (0..rng.below(3))
                .map(|_| random_string(rng, true))
                .collect(),
            extra: extra(rng),
            ..Board::default()
        };
        for _ in 0..rng.below(8) {
            let node = Node {
                text: random_string(rng, false),
                kind: (!rng.one_in(8)).then(|| named::<NodeKind>(rng)),
                at: [int(rng), int(rng)],
                parent: rng.one_in(4).then(|| random_string(rng, true)),
                shape: named::<Shape>(rng),
                size: [int(rng), int(rng)],
                style: NodeStyle {
                    color: color(rng),
                    fill: named::<Fill>(rng),
                    stroke: named::<Dash>(rng),
                    extra: extra(rng),
                },
                z: (!rng.one_in(8)).then(|| random_string(rng, true)),
                extra: extra(rng),
            };
            board.nodes.insert(random_id(rng), node);
        }
        for _ in 0..rng.below(6) {
            let edge = Edge {
                kind: (!rng.one_in(8)).then(|| named::<EdgeKind>(rng)),
                from: end(rng),
                to: end(rng),
                heads: if rng.one_in(2) {
                    Edge::DEFAULT_HEADS
                } else {
                    [named::<Head>(rng), named::<Head>(rng)]
                },
                labels: (0..rng.below(3))
                    .map(|_| Label {
                        text: random_string(rng, false),
                        at: if rng.one_in(2) {
                            Label::DEFAULT_AT
                        } else {
                            int(rng)
                        },
                        extra: extra(rng),
                    })
                    .collect(),
                route: named::<Route>(rng),
                style: EdgeStyle {
                    color: color(rng),
                    stroke: named::<Dash>(rng),
                    extra: extra(rng),
                },
                z: (!rng.one_in(8)).then(|| random_string(rng, true)),
                extra: extra(rng),
            };
            board.edges.insert(random_id(rng), edge);
        }
        board
    }

    /// The rules every written file keeps, whatever the model: §4.11 rule 1.
    #[track_caller]
    fn assert_clean_bytes(text: &str) {
        assert!(text.ends_with('\n') && !text.ends_with("\n\n"), "{text:?}");
        assert!(!text.starts_with('\u{feff}'));
        for line in text.split('\n') {
            assert!(!line.ends_with(' '), "trailing space in {line:?}");
            assert!(
                !line.chars().any(|c| c.is_ascii_control() || c == '\u{7f}'),
                "raw control character in {line:?}"
            );
        }
    }

    fn round_trips(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        for _ in 0..count {
            let board = random_board(&mut rng);
            let written = write(&board);
            assert_clean_bytes(&written);
            let again = parse(&written).unwrap_or_else(|error| panic!("{error}\n{written}"));
            assert_eq!(write(&again), written);
        }
    }

    /// Test 3: `write(parse(write(m))) == write(m)` over random models.
    #[test]
    fn random_models_round_trip() {
        round_trips(500, 3);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_random_models_round_trip() {
        round_trips(20_000, 4);
    }

    fn strings_survive(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        for _ in 0..count {
            let text = random_string(&mut rng, true);
            let mut board = Board::default();
            let mut node =
                read("schema = 1\n[node.a]\nat = [0, 0]\nsize = [1, 1]\n").nodes["a"].clone();
            node.text = text.clone();
            node.extra.insert(text.clone(), Value::String(text.clone()));
            board.nodes.insert(Id::new("a").unwrap(), node);
            board.extra.insert(
                text.clone(),
                Value::Array(vec![Value::String(text.clone())]),
            );
            let written = write(&board);
            assert_clean_bytes(&written);
            let back = parse(&written).unwrap_or_else(|error| panic!("{error}\n{written}"));
            let node = &back.nodes["a"];
            assert_eq!(node.text, text.replace("\r\n", "\n").replace('\r', "\n"));
            assert_eq!(
                node.extra.get(&text).and_then(Value::as_str),
                Some(text.as_str())
            );
            assert_eq!(back.extra.get(&text), board.extra.get(&text));
        }
    }

    /// Test 3: strings, in labels, in keys and in unknown content, and
    /// floats come back as written.
    #[test]
    fn strings_and_floats_survive_write_then_parse() {
        strings_survive(2_000, 5);
        let mut rng = Rng(6);
        let floats: Vec<f64> = FLOATS
            .iter()
            .copied()
            .chain((0..2_000).map(|_| f64::from_bits(rng.next())))
            .collect();
        let mut board = Board::default();
        board.extra.insert(
            "f".into(),
            Value::Array(floats.iter().map(|&x| Value::Float(x)).collect()),
        );
        let written = write(&board);
        assert_clean_bytes(&written);
        let back = read(&written);
        let Some(Value::Array(items)) = back.extra.get("f") else {
            panic!("{written}");
        };
        assert_eq!(items.len(), floats.len());
        for (item, x) in items.iter().zip(&floats) {
            let Value::Float(y) = item else {
                panic!("{x:?} read back as {item:?}");
            };
            assert!(
                y.to_bits() == x.to_bits() || (x.is_nan() && y.is_nan()),
                "{x:?} read back as {y:?}"
            );
        }
        let ints = read("schema = 1\nx = [1, 1.0, -0.0, 0]\n");
        assert_eq!(write(&ints), "schema = 1\nx = [1, 1.0, -0.0, 0]\n");
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_strings_survive_write_then_parse() {
        strings_survive(20_000, 7);
    }

    #[test]
    fn model_maps_are_not_relied_on_for_order() {
        let keys = ["b", "a", "é", "Z", "_", "-", ""];
        let mut board = Board::default();
        for key in keys {
            board.extra.insert(key.into(), Value::Integer(1));
        }
        let written: Vec<Vec<String>> = blocks(&write(&board))[0].1[1..].to_vec();
        let mut sorted: Vec<&[u8]> = keys.iter().map(|k| k.as_bytes()).collect();
        sorted.sort_unstable();
        let expected: Vec<Vec<String>> = sorted
            .into_iter()
            .map(|k| vec![String::from_utf8(k.to_vec()).unwrap()])
            .collect();
        assert_eq!(written, expected);
    }
}
