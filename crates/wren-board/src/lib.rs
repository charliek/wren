//! Boards: canvases of shapes and connectors, saved as `Name.wren-board`.
//!
//! The crate knows nothing about a UI toolkit or about Wren's notes. It
//! holds the model ([`Board`]), the schema-1 file format, and [`scene`],
//! which turns a board, a [`Palette`] and a text measurer into the display
//! list that the canvas widget paints and the SVG writer serialises.
//!
//! The model keeps everything a file holds. Known fields are typed; a
//! string field from a closed set keeps an unknown value as written
//! ([`Named::Other`]); unknown keys and everything under `ext` are kept as
//! TOML values ([`Extra`]), so a load followed by a save loses nothing a
//! newer build wrote.
//!
//! The pipeline, each step a plain function of the one before:
//!
//! - [`parse`] reads any valid TOML of the board's shape into a [`Board`],
//!   or fails with a [`ParseError`] naming a line and column. It never
//!   panics and never repairs anything.
//! - [`check`] lists what is wrong but still loads (dangling ends, parent
//!   cycles, missing kinds and stacking keys); [`access`] says whether this
//!   build may edit the board; [`placements`] resolves the parent tree.
//! - [`write()`] writes the canonical form: deterministic, one field per
//!   line, so one edit changes one line, and `write(parse(write(m)))` is
//!   `write(m)`.
//! - [`scene::build`] lays a board out for drawing, with [`shape`] for
//!   outlines and label boxes, [`route`] for connector paths and [`style`]
//!   for colours. [`svg::write`] serialises a scene; [`search_text`]
//!   gives the labels search indexes.
//!
//! The format, the scene rules and their constants are specified in the
//! plan, `discovery/boards.md`, §3.7 and §4. Golden files under `golden/`
//! pin the format (`format/`), the scene (`scene/`) and the SVG (`svg/`);
//! `WREN_BLESS=1 cargo test -p wren-board` rewrites them for review as a
//! diff. `make fuzz-board` runs the property tests at full count and
//! `make perf-board` times parse, write and scene build, both in release.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;

mod check;
#[cfg(test)]
mod fixtures;
mod read;
pub mod route;
pub mod scene;
mod search;
pub mod shape;
pub mod style;
pub mod svg;
mod write;

pub use check::{
    access, check, placements, Access, Issue, IssueKind, Placed, Reason, MAX_DEPTH, SCHEMA,
};
pub use read::{parse, ParseError};
pub use search::search_text;
pub use write::write;

/// The editing grid, in board units. It is never stored, and off-grid
/// values are valid.
pub const GRID: i32 = 20;

/// The new-board template, which is also the canonical form of an empty
/// board.
pub const EMPTY: &str = "schema = 1\n";

/// Content kept as read and never interpreted: unknown keys, and
/// everything under `ext`.
pub type Extra = toml::Table;

/// A whole board file.
#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    /// `schema`: 1 for every file this build writes. A higher value that
    /// still loads opens read-only.
    pub schema: i32,
    /// `requires`: feature names a build must know to edit the board.
    pub requires: Vec<String>,
    pub nodes: BTreeMap<Id, Node>,
    pub edges: BTreeMap<Id, Edge>,
    /// Unknown top-level keys.
    pub extra: Extra,
}

impl Default for Board {
    /// The empty schema-1 board, which is what [`EMPTY`] reads as.
    fn default() -> Self {
        Self {
            schema: 1,
            requires: Vec::new(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            extra: Extra::new(),
        }
    }
}

/// An element id: 1 to 32 characters of `[A-Za-z0-9_-]`, compared
/// bytewise.
///
/// Only the keys of `node` and `edge` are ids. A reference to a node
/// (`parent`, an endpoint's `node`) is a plain string, because a file may
/// name something that is not a valid id, and that string is kept as
/// written.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(String);

impl Id {
    pub const MAX_LEN: usize = 32;

    /// The id, if `text` is one.
    pub fn new(text: &str) -> Option<Self> {
        let valid = (1..=Self::MAX_LEN).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        valid.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Id {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A string field whose values come from a closed set. A value outside the
/// set is kept as written and drawn as the field's fallback.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Named<T> {
    Known(T),
    Other(String),
}

impl<T: Vocabulary> Named<T> {
    /// `text` as a known value when it names one, exactly and with case;
    /// otherwise kept as written.
    pub fn read(text: String) -> Self {
        T::from_name(&text).map_or(Self::Other(text), Self::Known)
    }

    /// The value as it is written in a file.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(value) => value.name(),
            Self::Other(text) => text,
        }
    }

    /// The known value, or `None` for one this build does not know.
    pub fn known(&self) -> Option<T> {
        match self {
            Self::Known(value) => Some(*value),
            Self::Other(_) => None,
        }
    }
}

/// A closed set of string values, each written as one fixed name.
pub trait Vocabulary: Copy + Sized + 'static {
    /// Every value, in declaration order.
    const ALL: &'static [Self];
    fn name(self) -> &'static str;
    fn from_name(name: &str) -> Option<Self>;
}

macro_rules! vocabulary {
    ($($ty:ident { $($variant:ident => $name:literal),+ $(,)? })+) => {$(
        impl Vocabulary for $ty {
            const ALL: &'static [Self] = &[$(Self::$variant),+];

            fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    )+};
}

vocabulary! {
    NodeKind { Shape => "shape", Text => "text" }
    Shape {
        Rect => "rect",
        Rounded => "rounded",
        Pill => "pill",
        Ellipse => "ellipse",
        Diamond => "diamond",
        Parallelogram => "parallelogram",
        Cylinder => "cylinder",
        Document => "document",
    }
    EdgeKind { Edge => "edge" }
    Side { Top => "top", Right => "right", Bottom => "bottom", Left => "left" }
    Head { None => "none", Arrow => "arrow" }
    Route { Straight => "straight", Elbow => "elbow" }
    Token {
        Neutral => "neutral",
        Red => "red",
        Orange => "orange",
        Yellow => "yellow",
        Green => "green",
        Cyan => "cyan",
        Purple => "purple",
        Blue => "blue",
    }
    Fill { None => "none", Tint => "tint", Solid => "solid" }
    Dash { Solid => "solid", Dashed => "dashed" }
}

/// `[node.<id>]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The label: plain text with LF line breaks. Empty when absent.
    pub text: String,
    /// `None` when the file has no `kind`, which is flagged.
    pub kind: Option<Named<NodeKind>>,
    /// The top-left corner, relative to the parent's when `parent` is set.
    pub at: [i32; 2],
    /// The containing node's id, as written.
    pub parent: Option<String>,
    pub shape: Named<Shape>,
    /// Width and height. Authoritative: never re-measured on load.
    pub size: [i32; 2],
    pub style: NodeStyle,
    /// The stacking key, as written; `None` when absent.
    pub z: Option<String>,
    /// Unknown keys, the reserved ring-1 fields, and `ext`.
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Shape,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Rect,
    Rounded,
    Pill,
    Ellipse,
    Diamond,
    Parallelogram,
    Cylinder,
    Document,
}

/// A node's `style` table.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeStyle {
    pub color: Color,
    pub fill: Named<Fill>,
    pub stroke: Named<Dash>,
    pub extra: Extra,
}

impl Default for NodeStyle {
    fn default() -> Self {
        Self {
            color: Color::Token(Token::Neutral),
            fill: Named::Known(Fill::Tint),
            stroke: Named::Known(Dash::Solid),
            extra: Extra::new(),
        }
    }
}

/// `[edge.<id>]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// `None` when the file has no `kind`, which is flagged.
    pub kind: Option<Named<EdgeKind>>,
    pub from: End,
    pub to: End,
    /// The markers at the `from` end and at the `to` end.
    pub heads: [Named<Head>; 2],
    pub labels: Vec<Label>,
    pub route: Named<Route>,
    pub style: EdgeStyle,
    /// The stacking key, as written; `None` when absent.
    pub z: Option<String>,
    /// Unknown keys, the reserved ring-1 fields, and `ext`.
    pub extra: Extra,
}

impl Edge {
    /// No marker at the `from` end and an arrow at the `to` end.
    pub const DEFAULT_HEADS: [Named<Head>; 2] =
        [Named::Known(Head::None), Named::Known(Head::Arrow)];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Edge,
}

/// One end of a connector.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// Attached to a node. `anchor`, per-mille of the node's box from its
    /// top-left, wins over `side`; with neither, the router picks the side.
    Node {
        node: String,
        side: Option<Named<Side>>,
        anchor: Option<[i32; 2]>,
        extra: Extra,
    },
    /// A free end at absolute board coordinates.
    Free { at: [i32; 2], extra: Extra },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Right => Self::Left,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
        }
    }

    pub const fn is_horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Head {
    None,
    Arrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Straight,
    Elbow,
}

/// One item of a connector's `labels`.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    /// The position along the route from the `from` end, per-mille.
    pub at: i32,
    pub extra: Extra,
}

impl Label {
    /// The middle of the route.
    pub const DEFAULT_AT: i32 = 500;
}

/// A connector's `style` table. A connector has no fill, so a `fill` key
/// here is unknown content.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeStyle {
    pub color: Color,
    pub stroke: Named<Dash>,
    pub extra: Extra,
}

impl Default for EdgeStyle {
    fn default() -> Self {
        Self {
            color: Color::Token(Token::Neutral),
            stroke: Named::Known(Dash::Solid),
            extra: Extra::new(),
        }
    }
}

/// `style.color`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    Token(Token),
    /// A literal `#rrggbb`, used as the hue in every theme.
    Hex([u8; 3]),
    /// Any other string, kept as written and drawn as `neutral`.
    Unknown(String),
}

impl Color {
    /// A token by its exact name, `#rrggbb` in either case, or else the
    /// string kept as written.
    pub fn read(text: String) -> Self {
        if let Some(token) = Token::from_name(&text) {
            return Self::Token(token);
        }
        match text.strip_prefix('#').and_then(hex_rgb) {
            Some(rgb) => Self::Hex(rgb),
            None => Self::Unknown(text),
        }
    }
}

fn hex_rgb(digits: &str) -> Option<[u8; 3]> {
    let bytes = digits.as_bytes();
    if bytes.len() != 6 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let channel = |i: usize| {
        digits
            .get(i..i + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
    };
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// The colour tokens. Each theme's [`Palette`] supplies the seven hues
/// after `Neutral`, which are JSON Canvas presets 1 to 6 plus blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Token {
    Neutral,
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Purple,
    Blue,
}

/// `style.fill`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fill {
    None,
    Tint,
    Solid,
}

/// `style.stroke`: the outline of a shape or the line of a connector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dash {
    Solid,
    Dashed,
}

/// A colour with straight (not premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

/// The colours a theme gives a board. The file names tokens; the palette
/// turns them into colours, so a board never changes with the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Palette {
    pub canvas: Rgba,
    pub ink: Rgba,
    pub muted: Rgba,
    pub raised: Rgba,
    pub accent: Rgba,
    pub issue: Rgba,
    /// Red, orange, yellow, green, cyan, purple and blue, in [`Token`]
    /// order.
    pub hues: [Rgba; 7],
    pub dark: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_pattern_and_length() {
        for valid in ["a", "start", "A-b_9", "0123456789abcdefghjkmnpqrstvwxyz"] {
            assert_eq!(Id::new(valid).map(|id| id.to_string()), Some(valid.into()));
        }
        let too_long = "a".repeat(Id::MAX_LEN + 1);
        for invalid in ["", "a.b", "a b", "é", "a\n", too_long.as_str()] {
            assert_eq!(Id::new(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn ids_sort_bytewise_and_look_up_by_str() {
        let mut nodes = BTreeMap::new();
        for id in ["b", "B", "_", "-", "a", "0"] {
            nodes.insert(Id::new(id).unwrap(), ());
        }
        let order: Vec<&str> = nodes.keys().map(Id::as_str).collect();
        assert_eq!(order, ["-", "0", "B", "_", "a", "b"]);
        assert!(nodes.contains_key("B"));
    }

    /// A board of `nodes` labelled shapes on a 40-column grid, with a
    /// connector to each right-hand neighbour: about twice as many elements
    /// as nodes, the shape of a large flowchart.
    fn grid_board(nodes: usize) -> String {
        let edges = (0..nodes).filter(|i| i % 40 != 39 && i + 1 < nodes);
        board_text(
            nodes,
            |i| [(i % 40) as i64 * 200, (i / 40) as i64 * 100],
            |_| [160, 64],
            edges.map(|i| (i, i + 1)),
        )
    }

    /// A board of `nodes` labelled shapes, node `i` at `at(i)` sized
    /// `size(i)`, with a labelled connector between each pair of `edges`.
    fn board_text(
        nodes: usize,
        at: impl Fn(usize) -> [i64; 2],
        size: impl Fn(usize) -> [i64; 2],
        edges: impl IntoIterator<Item = (usize, usize)>,
    ) -> String {
        let mut out = String::from(EMPTY);
        let shapes = [
            "rect",
            "rounded",
            "pill",
            "ellipse",
            "diamond",
            "parallelogram",
            "cylinder",
            "document",
        ];
        for i in 0..nodes {
            let ([x, y], [w, h]) = (at(i), size(i));
            out += &format!(
                "\n[node.n{i}]\ntext = \"Step {i}\\nchecks the order\"\nkind = \"shape\"\n\
                 at = [{x}, {y}]\nshape = \"{}\"\nsize = [{w}, {h}]\nz = \"V{i}\"\n",
                shapes[i % shapes.len()]
            );
        }
        for (k, (from, to)) in edges.into_iter().enumerate() {
            out += &format!(
                "\n[edge.e{k}]\nkind = \"edge\"\nfrom = {{ node = \"n{from}\" }}\n\
                 to = {{ node = \"n{to}\" }}\nlabels = [\n  {{ text = \"next\" }},\n]\nz = \"W{k}\"\n"
            );
        }
        out
    }

    /// The median of `runs` timings of `work`, in milliseconds.
    fn median_ms(runs: usize, mut work: impl FnMut()) -> f64 {
        let mut times: Vec<f64> = (0..runs)
            .map(|_| {
                let start = std::time::Instant::now();
                work();
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times[runs / 2]
    }

    /// §6.7's `wren-board` budgets, in release: at 10,000 elements a parse
    /// within 75 ms and a write within 10 ms; at 1,000 labelled shapes a
    /// scene build within 5 ms. The other sizes are printed for the record.
    #[test]
    #[ignore = "timings; run by make perf-board in release"]
    fn perf_parse_write_and_scene() {
        let palette = style::tests::LIGHT;
        let measure = scene::FixedMeasure;
        for nodes in [500, 1_000, 5_100] {
            let text = grid_board(nodes);
            let board = parse(&text).unwrap();
            let elements = board.nodes.len() + board.edges.len();
            let parse_ms = median_ms(9, || drop(parse(&text).unwrap()));
            let write_ms = median_ms(9, || drop(write(&board)));
            let scene_ms = median_ms(9, || drop(scene::build(&board, &palette, &measure)));
            let svg_scene = scene::build(&board, &palette, &measure);
            let svg_ms = median_ms(9, || drop(svg::write(&svg_scene, &svg::Options::default())));
            let search_ms = median_ms(9, || drop(search_text(&board)));
            println!(
                "{nodes} shapes, {elements} elements, {} KiB: parse {parse_ms:.2} ms, \
                 write {write_ms:.2} ms, scene {scene_ms:.2} ms, svg {svg_ms:.2} ms, \
                 search text {search_ms:.2} ms",
                text.len() / 1024
            );
            if elements >= 10_000 {
                assert!(parse_ms <= 75.0, "parse of {elements}: {parse_ms:.2} ms");
                assert!(write_ms <= 10.0, "write of {elements}: {write_ms:.2} ms");
            }
            if nodes == 1_000 {
                assert!(scene_ms <= 5.0, "scene of {nodes} shapes: {scene_ms:.2} ms");
            }
        }
        // Many words to a line: 50 boxes 28,000 wide, each label 8,000
        // words (0.8 MB). Joining a word at a time took 6.9 s here.
        let label = "a ".repeat(8_000);
        let mut text = String::from("schema = 1\n");
        for i in 0..50 {
            text += &format!(
                "\n[node.n{i}]\ntext = \"{label}\"\nkind = \"shape\"\nat = [0, {}]\n\
                 size = [28000, 60]\nz = \"V\"\n",
                i * 100
            );
        }
        // The grid with every connector made diagonal, to the next row's
        // right-hand neighbour: each has a side to choose and searches the
        // whole candidate set among dense boxes. Printed for the record.
        let grid = |i: usize| [(i % 40) as i64 * 200, (i / 40) as i64 * 100];
        let edges = (0..1_000usize).filter(|i| i % 40 != 39);
        let diagonal = board_text(
            1_000,
            grid,
            |_| [160, 64],
            edges.map(|i| (i, (i + 41).min(999))),
        );
        let diagonal = parse(&diagonal).unwrap();
        let diagonal_ms = median_ms(9, || drop(scene::build(&diagonal, &palette, &measure)));
        println!("1000 shapes, every connector diagonal: scene {diagonal_ms:.2} ms");

        // Boards on which no connector finds a clean route, so each spends
        // its whole routing budget (route::BUDGET): long random connectors
        // and a fan-in on the grid, and shapes stacked on one spot joined
        // to one far shape. The work grows with the connectors, not with
        // connectors times boxes, so a 10,000-element board leaves most of
        // the 1 s open budget. The 5 ms scene budget of §6.7 is not met by
        // these (recorded in §8.5); the ceilings catch a return of the
        // quadratic growth, which took 0.2 to 0.7 s at 1,000 shapes and 2.6
        // to 15 s at 10,000 elements.
        let mut rng = fixtures::Rng(3);
        let targets: Vec<usize> = (0..5_100).map(|_| rng.below(5_100)).collect();
        for nodes in [1_000, 5_100] {
            let stacked = |i: usize| {
                if i == nodes {
                    [5_000, 5_000]
                } else {
                    [(i % 7) as i64, (i % 5) as i64]
                }
            };
            let boards = [
                (
                    "long random connectors",
                    board_text(
                        nodes,
                        grid,
                        |_| [160, 64],
                        (0..nodes).map(|i| (i, targets[i] % nodes)),
                    ),
                ),
                (
                    "a fan-in",
                    board_text(nodes, grid, |_| [160, 64], (1..nodes).map(|i| (i, 0))),
                ),
                (
                    "stacked shapes to one far shape",
                    board_text(
                        nodes + 1,
                        stacked,
                        |_| [100, 60],
                        (0..nodes).map(|i| (i, nodes)),
                    ),
                ),
            ];
            for (name, text) in boards {
                let board = parse(&text).unwrap();
                let elements = board.nodes.len() + board.edges.len();
                let scene_ms = median_ms(3, || drop(scene::build(&board, &palette, &measure)));
                println!("{nodes} shapes, {elements} elements, {name}: scene {scene_ms:.2} ms");
                let ceiling = if elements >= 10_000 { 600.0 } else { 150.0 };
                assert!(
                    scene_ms <= ceiling,
                    "{name}, {elements} elements: {scene_ms:.2} ms"
                );
            }
        }

        let board = parse(&text).unwrap();
        let wide_ms = median_ms(3, || drop(scene::build(&board, &palette, &measure)));
        println!("50 wide labels of 8,000 words: scene {wide_ms:.2} ms");
        assert!(wide_ms <= 250.0, "scene of wide labels: {wide_ms:.2} ms");
    }
}
