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

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;

#[cfg(test)]
mod fixtures;
mod read;
pub mod scene;
mod write;

pub use read::{parse, ParseError};
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
}
