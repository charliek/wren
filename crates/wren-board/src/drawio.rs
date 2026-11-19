//! A board as a draw.io file: one uncompressed page of draw.io's own
//! built-in shapes, for draw.io to open and edit. The export is one way:
//! draw.io gives every cell a new id when a file is imported or pasted, so
//! nothing promises a file comes back.
//!
//! Written from draw.io's file format (the `mxfile` XML and its style
//! strings) as the plan's study recorded it, with no draw.io code copied;
//! draw.io is Apache-2.0. The plan's §7.4 has the study's rules; which
//! shape style draws which Wren shape, and why each key is written, is in
//! its log, `discovery/boards.md` §8.5 `boards-p3-drawio`, "The mapping".
//!
//! What a board holds that draw.io cannot draw, or that draw.io would
//! change on an ordinary edit, is not in a style string: Wren's id, an
//! unknown kind or shape, and the node a dangling end names are attributes
//! of the `<object>` each element is wrapped in. Colours are resolved to hex
//! for one palette, so the colour tokens are not kept. Connector routes are
//! draw.io's own: an elbow connector is draw.io's orthogonal edge, with
//! none of Wren's computed bends.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::route::{self, Attach, Terminal};
use crate::scene::{self, Point, Rect, MAX_LABELS, SMALL_SIZE, STUB_LENGTH};
use crate::style::{self, Resolved};
use crate::svg::num;
use crate::{
    placements, shape, z, Board, Dash, Edge, End, Fill, Id, Label, Named, Node, NodeKind, Palette,
    Rgba, Route, Shape, Side,
};

/// The file name extension of a draw.io file.
pub const EXTENSION: &str = "drawio";

/// The arrowhead size draw.io is given: its filled triangle is this plus
/// the stroke width long and as wide, which makes it as long as Wren's.
const HEAD_SIZE: &str = "8.5";

/// `board` as a draw.io file whose one page is called `name`, in the
/// colours `palette` resolves.
pub fn write(board: &Board, palette: &Palette, name: &str) -> String {
    let mut out = String::from("<mxfile host=\"Wren\">\n");
    // The page's id, like every cell's added here, holds a dot, which no
    // Wren id does, so it can never be an element's.
    let _ = writeln!(
        out,
        "  <diagram id=\"wren.board\" name=\"{}\">",
        attribute(name)
    );
    let _ = writeln!(
        out,
        "    <mxGraphModel grid=\"1\" gridSize=\"{}\" page=\"0\" math=\"0\" shadow=\"0\">",
        crate::GRID
    );
    out.push_str(
        "      <root>\n        <mxCell id=\"0\"/>\n        <mxCell id=\"1\" parent=\"0\"/>\n",
    );
    Writer::new(board, palette).cells(&mut out);
    out.push_str("      </root>\n    </mxGraphModel>\n  </diagram>\n</mxfile>\n");
    out
}

/// One element of a scope, in Wren's drawing order.
#[derive(Clone, Copy)]
enum Element<'a> {
    Node(&'a Id, &'a Node),
    Edge(&'a Id, &'a Edge),
}

impl<'a> Element<'a> {
    fn order(&self) -> (&'a [u8], &'a str, bool) {
        match *self {
            Self::Node(id, node) => z::order(node.z.as_deref(), id, false),
            Self::Edge(id, edge) => z::order(edge.z.as_deref(), id, true),
        }
    }
}

struct Writer<'a> {
    board: &'a Board,
    palette: &'a Palette,
    /// Each node's absolute box, as the scene draws it.
    boxes: BTreeMap<&'a Id, Rect>,
    /// The node each node is drawn inside.
    parents: BTreeMap<&'a Id, Option<&'a Id>>,
    /// The sides Wren's router took for each connector's ends, worked out
    /// only when an elbow connector has a floating or anchored end.
    sides: BTreeMap<&'a Id, [Option<Side>; 2]>,
}

impl<'a> Writer<'a> {
    fn new(board: &'a Board, palette: &'a Palette) -> Self {
        let placed = placements(board);
        let boxes = scene::boxes(board, &placed);
        let parents = placed.iter().map(|(&id, p)| (id, p.parent)).collect();
        // A pinned end leaves by its own side; the router picks the side
        // of a floating end and of an anchor.
        let chosen = |end: &End| {
            matches!(end, End::Node { side, anchor, .. }
                if !matches!(Attach::of(side.as_ref(), *anchor), Attach::Side(_)))
        };
        let routed = board.edges.values().any(|edge| {
            route::drawn(&edge.route) == Route::Elbow && (chosen(&edge.from) || chosen(&edge.to))
        });
        let sides = if routed {
            scene::sides(board)
        } else {
            BTreeMap::new()
        };
        Self {
            board,
            palette,
            boxes,
            parents,
            sides,
        }
    }

    /// Every element as a cell, in Wren's drawing order: the root scope
    /// (root nodes and every connector) sorted by stacking key, each node
    /// followed at once by its children, which draw.io draws above it.
    /// Connectors are cells of the layer, as they are of the root scope.
    fn cells(&self, out: &mut String) {
        let mut children: BTreeMap<&Id, Vec<Element<'a>>> = BTreeMap::new();
        let mut roots: Vec<Element<'a>> = (self.board.edges.iter())
            .map(|(id, edge)| Element::Edge(id, edge))
            .collect();
        for (id, node) in &self.board.nodes {
            match self.parents.get(id).copied().flatten() {
                Some(parent) => children
                    .entry(parent)
                    .or_default()
                    .push(Element::Node(id, node)),
                None => roots.push(Element::Node(id, node)),
            }
        }
        roots.sort_by(|a, b| a.order().cmp(&b.order()));
        // Depth first, without recursion: a chain of parents is as long as
        // the board allows.
        let mut scopes = vec![roots.into_iter()];
        while let Some(scope) = scopes.last_mut() {
            let Some(element) = scope.next() else {
                scopes.pop();
                continue;
            };
            match element {
                Element::Node(id, node) => {
                    self.node(out, id, node);
                    if let Some(mut inner) = children.remove(id) {
                        inner.sort_by(|a, b| a.order().cmp(&b.order()));
                        scopes.push(inner.into_iter());
                    }
                }
                Element::Edge(id, edge) => self.edge(out, id, edge),
            }
        }
    }

    /// The cell id of a node: its own id, except the two draw.io keeps
    /// for its root and first layer.
    fn node_cell(id: &str) -> String {
        if matches!(id, "0" | "1") {
            format!("{id}.node")
        } else {
            id.to_owned()
        }
    }

    /// The cell id of a connector: its own id, except those two and an id
    /// a node also has. A Wren id never holds a dot, so none of these
    /// meets another cell's id.
    fn edge_cell(&self, id: &Id) -> String {
        if matches!(id.as_str(), "0" | "1") || self.board.nodes.contains_key(id) {
            format!("{id}.edge")
        } else {
            id.to_string()
        }
    }

    fn node(&self, out: &mut String, id: &Id, node: &Node) {
        let Some(&bounds) = self.boxes.get(id) else {
            return;
        };
        let parent = self.parents.get(id).copied().flatten();
        let (parent_cell, origin) = match parent.and_then(|p| Some((p, self.boxes.get(p)?))) {
            Some((parent, corner)) => (Self::node_cell(parent.as_str()), (corner.x, corner.y)),
            None => ("1".to_owned(), (0.0, 0.0)),
        };
        let mut style = String::new();
        let mut extra = Vec::new();
        match node.kind.as_ref().and_then(Named::known) {
            Some(NodeKind::Shape) => {
                let drawn = shape::drawn(&node.shape);
                style += &shape_style(drawn, bounds);
                style += "whiteSpace=wrap;html=1;";
                let fill = node.style.fill.known().unwrap_or(Fill::Tint);
                let resolved = style::resolve(&node.style.color, fill, self.palette);
                colours(&mut style, resolved);
                outline(&mut style, &node.style.stroke);
                if let Named::Other(name) = &node.shape {
                    extra.push(("wrenShape", name.as_str()));
                }
            }
            Some(NodeKind::Text) => {
                let color = style::resolve(&node.style.color, Fill::None, self.palette).stroke;
                style += "text;html=1;whiteSpace=wrap;strokeColor=none;fillColor=none;\
                          align=left;verticalAlign=top;spacing=0;";
                let _ = write!(style, "fontColor={};", hex(color));
            }
            None => {
                // A kind this build does not know, or none: the placeholder
                // the canvas draws, a dashed outline in the muted colour.
                style += "rounded=0;whiteSpace=wrap;html=1;fillColor=none;";
                let _ = write!(
                    style,
                    "strokeColor={};fontColor={};strokeWidth={};",
                    hex(self.palette.muted),
                    hex(self.palette.ink),
                    num(route::STROKE_WIDTH)
                );
                outline(&mut style, &Named::Known(Dash::Dashed));
                if let Some(Named::Other(kind)) = &node.kind {
                    extra.push(("wrenKind", kind.as_str()));
                }
            }
        }
        let _ = write!(style, "fontSize={};", num(node.style.drawn_text_size()));
        let geometry = format!(
            "<mxGeometry{}{} width=\"{}\" height=\"{}\" as=\"geometry\"/>",
            coordinate("x", bounds.x - origin.0),
            coordinate("y", bounds.y - origin.1),
            num(bounds.width),
            num(bounds.height)
        );
        let label = html(&node.text);
        let id_cell = Self::node_cell(id.as_str());
        object(
            out,
            &id_cell,
            id,
            &label,
            &extra,
            &format!(
                "          <mxCell style=\"{}\" vertex=\"1\" parent=\"{}\">\n            \
                 {geometry}\n          </mxCell>\n",
                attribute(&style),
                attribute(&parent_cell)
            ),
        );
    }

    fn edge(&self, out: &mut String, id: &Id, edge: &Edge) {
        let ends = [&edge.from, &edge.to].map(|end| self.end(end));
        // A connector with no end drawn draws nothing; one with a single
        // end drawn draws the canvas's stub, out of that end, which leaves
        // to the right of a `from` end and to the left of a `to` end
        // unless the end is pinned.
        let ends = match ends {
            [Drawn::Missing(_), Drawn::Missing(_)] => return,
            [Drawn::Missing(name), to] => [self.stub(&to, Side::Left, name), to],
            [from, Drawn::Missing(name)] => {
                let to = self.stub(&from, Side::Right, name);
                [from, to]
            }
            ends => ends,
        };
        let cell = self.edge_cell(id);
        let mut style = String::new();
        if route::drawn(&edge.route) == Route::Elbow {
            style += "edgeStyle=orthogonalEdgeStyle;orthogonalLoop=1;jettySize=auto;";
        }
        style += "rounded=0;html=1;";
        // Both heads always: draw.io's own default is an arrow at the end.
        for (prefix, head) in [("start", &edge.heads[0]), ("end", &edge.heads[1])] {
            if route::has_arrow(head) {
                let _ = write!(
                    style,
                    "{prefix}Arrow=block;{prefix}Fill=1;{prefix}Size={HEAD_SIZE};"
                );
            } else {
                let _ = write!(style, "{prefix}Arrow=none;");
            }
        }
        let color = style::resolve(&edge.style.color, Fill::None, self.palette).stroke;
        let _ = write!(
            style,
            "strokeColor={};strokeWidth={};fontColor={};fontSize={};labelBackgroundColor=default;",
            hex(color),
            num(route::STROKE_WIDTH),
            hex(color),
            num(SMALL_SIZE)
        );
        outline(&mut style, &edge.style.stroke);
        let mut terminals = String::new();
        let mut points = String::new();
        let mut extra = Vec::new();
        let elbow = route::drawn(&edge.route) == Route::Elbow;
        let sides = self.sides.get(id).copied().unwrap_or_default();
        for (((end, prefix), (terminal, role, wren)), side) in ends
            .iter()
            .zip(["exit", "entry"])
            .zip([
                ("source", "sourcePoint", "wrenFrom"),
                ("target", "targetPoint", "wrenTo"),
            ])
            .zip(sides)
        {
            match end {
                Drawn::Node {
                    id,
                    attach,
                    terminal: drawn,
                } => {
                    let _ = write!(
                        terminals,
                        " {terminal}=\"{}\"",
                        attribute(&Self::node_cell(id.as_str()))
                    );
                    attachment(&mut style, prefix, drawn);
                    // Every end of an elbow connector leaves by the side
                    // Wren's does, its pinned one or the one the router
                    // chose, so draw.io's own bends go the way Wren's do:
                    // without it draw.io guesses, and on a parallelogram's
                    // slanted side it guesses across the shape.
                    let leaves = match attach {
                        Attach::Side(pinned) => Some(*pinned),
                        Attach::Floating | Attach::Anchor(_) => side,
                    };
                    if let (true, Some(side)) = (elbow, leaves) {
                        let _ = write!(style, "{terminal}PortConstraint={};", compass(side));
                    }
                }
                Drawn::Point(at, missing) => {
                    let _ = writeln!(
                        points,
                        "              <mxPoint{}{} as=\"{role}\"/>",
                        coordinate("x", at.x),
                        coordinate("y", at.y)
                    );
                    if let Some(name) = missing {
                        extra.push((wren, *name));
                    }
                }
                Drawn::Missing(_) => {}
            }
        }
        if let Some(Named::Other(kind)) = &edge.kind {
            extra.push(("wrenKind", kind.as_str()));
        }
        let labels: Vec<&Label> = edge.labels.iter().take(MAX_LABELS).collect();
        let (own, x) = match labels.first() {
            Some(first) => (html(&first.text), coordinate("x", along(first.at))),
            None => (String::new(), String::new()),
        };
        let geometry = if points.is_empty() {
            format!("<mxGeometry{x} relative=\"1\" as=\"geometry\"/>")
        } else {
            format!(
                "<mxGeometry{x} relative=\"1\" as=\"geometry\">\n{points}            </mxGeometry>"
            )
        };
        object(
            out,
            &cell,
            id,
            &own,
            &extra,
            &format!(
                "          <mxCell style=\"{}\" edge=\"1\" parent=\"1\"{terminals}>\n            \
                 {geometry}\n          </mxCell>\n",
                attribute(&style)
            ),
        );
        // A second label and those after it are cells of their own, placed
        // along the connector as its own label is.
        for (n, label) in labels.iter().enumerate().skip(1) {
            let _ = writeln!(
                out,
                "        <mxCell id=\"{}\" value=\"{}\" style=\"edgeLabel;html=1;align=center;\
                 verticalAlign=middle;resizable=0;points=[];fontColor={};fontSize={};\
                 labelBackgroundColor=default;\" vertex=\"1\" connectable=\"0\" parent=\"{}\">\n          \
                 <mxGeometry{} relative=\"1\" as=\"geometry\">\n            \
                 <mxPoint as=\"offset\"/>\n          </mxGeometry>\n        </mxCell>",
                attribute(&format!("{cell}.label{n}")),
                attribute(&html(&label.text)),
                hex(color),
                num(SMALL_SIZE),
                attribute(&cell),
                coordinate("x", along(label.at)),
            );
        }
    }

    /// Where a connector end is drawn.
    fn end<'e>(&self, end: &'e End) -> Drawn<'e, 'a> {
        match end {
            End::Free { at, .. } => Drawn::Point(scene::clamp_point(*at), None),
            End::Node {
                node, side, anchor, ..
            } => {
                let found = Id::new(node).and_then(|id| self.boxes.get_key_value(&id));
                let Some((&id, &bounds)) = found else {
                    return Drawn::Missing(node);
                };
                let shape = match self.board.nodes.get(id) {
                    Some(node) if node.kind == Some(Named::Known(NodeKind::Shape)) => {
                        shape::drawn(&node.shape)
                    }
                    _ => Shape::Rect,
                };
                Drawn::Node {
                    id,
                    attach: Attach::of(side.as_ref(), *anchor),
                    terminal: Terminal::Node {
                        bounds,
                        shape,
                        attach: Attach::of(side.as_ref(), *anchor),
                    },
                }
            }
        }
    }

    /// The free end that stands for an end naming no node: the tip of the
    /// stub the canvas draws out of the `valid` end, which leaves by its
    /// pinned side, the side its anchor faces, or `nominal`.
    fn stub<'e>(&self, valid: &Drawn<'e, 'a>, nominal: Side, missing: &'e str) -> Drawn<'e, 'a> {
        let terminal = match valid {
            Drawn::Node { terminal, .. } => *terminal,
            Drawn::Point(at, _) => Terminal::Point(*at),
            Drawn::Missing(_) => return Drawn::Missing(missing),
        };
        let (start, side) = route::leave(&terminal, nominal);
        let (dx, dy) = match side {
            Side::Top => (0.0, -1.0),
            Side::Right => (1.0, 0.0),
            Side::Bottom => (0.0, 1.0),
            Side::Left => (-1.0, 0.0),
        };
        let tip = Point::new(start.x + dx * STUB_LENGTH, start.y + dy * STUB_LENGTH);
        Drawn::Point(tip, Some(missing))
    }
}

/// A connector end as it is drawn.
enum Drawn<'e, 'a> {
    /// Attached to a drawn node.
    Node {
        id: &'a Id,
        attach: Attach,
        terminal: Terminal,
    },
    /// A free point: the end's own, or, with the name of the node it names
    /// and that does not exist, the tip of the stub that stands for it.
    Point(Point, Option<&'e str>),
    /// An end naming no node, by that name.
    Missing(&'e str),
}

/// The style keys of a shape, drawn in `bounds`, before the label's and the
/// colours'. Each is a draw.io built-in, never a stencil library's shape.
fn shape_style(shape: Shape, bounds: Rect) -> String {
    let (w, h) = (bounds.width.max(0.0), bounds.height.max(0.0));
    match shape {
        Shape::Rect => "rounded=0;".into(),
        // A radius of half the arc size, reduced to half the shorter side.
        Shape::Rounded => format!(
            "rounded=1;absoluteArcSize=1;arcSize={};",
            num(2.0 * shape::ROUNDED_RADIUS)
        ),
        Shape::Pill => "rounded=1;arcSize=50;".into(),
        Shape::Ellipse => "ellipse;".into(),
        Shape::Diamond => "rhombus;".into(),
        Shape::Parallelogram => format!(
            "shape=parallelogram;perimeter=parallelogramPerimeter;fixedSize=1;size={};",
            num(shape::skew(w, h))
        ),
        Shape::Cylinder => format!(
            "shape=cylinder3;boundedLbl=1;backgroundOutline=1;size={};",
            num(shape::cylinder_cap(h))
        ),
        // draw.io's wave is a fraction of the height, its sides stopping half
        // of it above the bottom, where Wren's stop one amplitude above.
        Shape::Document => {
            let wave = if h > 0.0 {
                2.0 * shape::document_wave(h) / h
            } else {
                0.0
            };
            format!("shape=document;boundedLbl=1;size={};", num(wave))
        }
    }
}

/// A shape's fill, outline and label colours.
fn colours(style: &mut String, resolved: Resolved) {
    let fill = resolved.fill.map_or_else(|| "none".to_owned(), hex);
    let _ = write!(
        style,
        "fillColor={fill};strokeColor={};fontColor={};strokeWidth={};",
        hex(resolved.stroke),
        hex(resolved.label),
        num(route::STROKE_WIDTH)
    );
}

/// A dashed outline or line, with Wren's dashes: draw.io scales a dash
/// pattern by the stroke width unless `fixDash` is set.
fn outline(style: &mut String, stroke: &Named<Dash>) {
    if stroke.known() == Some(Dash::Dashed) {
        let [on, off] = route::DASH;
        let _ = write!(
            style,
            "dashed=1;fixDash=1;dashPattern={} {};",
            num(on),
            num(off)
        );
    }
}

/// Where a connector end meets its node: the middle of a pinned side, or an
/// anchor's point of the box, which draw.io would otherwise move onto the
/// outline. A floating end is draw.io's to place. A document's pinned
/// bottom meets the middle of its wave, one amplitude above the box's
/// bottom (`shape::side_point`), where draw.io would put the box's bottom.
fn attachment(style: &mut String, prefix: &str, terminal: &Terminal) {
    let Terminal::Node {
        bounds,
        shape,
        attach,
    } = *terminal
    else {
        return;
    };
    let wave = (shape == Shape::Document && attach == Attach::Side(Side::Bottom))
        .then(|| {
            let h = bounds.height.max(0.0);
            (h > 0.0).then(|| (h - shape::document_wave(h)) / h)
        })
        .flatten();
    let point = match attach {
        Attach::Floating => return,
        Attach::Side(side) => match side {
            Side::Top => [500, 0],
            Side::Right => [1000, 500],
            Side::Bottom => [500, 1000],
            Side::Left => [0, 500],
        },
        Attach::Anchor(anchor) => anchor.map(|v| v.clamp(0, 1000)),
    };
    let y = wave.map_or_else(|| per_mille(point[1]), num);
    let _ = write!(
        style,
        "{prefix}X={};{prefix}Y={y};{prefix}Dx=0;{prefix}Dy=0;",
        per_mille(point[0]),
    );
    if wave.is_some() || matches!(attach, Attach::Anchor(_)) {
        let _ = write!(style, "{prefix}Perimeter=0;");
    }
}

/// A side as draw.io names a direction.
fn compass(side: Side) -> &'static str {
    match side {
        Side::Top => "north",
        Side::Right => "east",
        Side::Bottom => "south",
        Side::Left => "west",
    }
}

/// A label's place along its connector as draw.io reads it, -1 at the
/// start to 1 at the end, from Wren's per-mille, clamped as it is drawn.
fn along(at: i32) -> f32 {
    (2 * at.clamp(0, 1000) - 1000) as f32 / 1000.0
}

/// A per-mille fraction as an exact decimal.
fn per_mille(value: i32) -> String {
    let value = value.clamp(0, 1000);
    if value == 1000 {
        return "1".into();
    }
    let digits = format!("{value:03}");
    let digits = digits.trim_end_matches('0');
    if digits.is_empty() {
        "0".into()
    } else {
        format!("0.{digits}")
    }
}

/// A geometry attribute, left out at 0 as draw.io writes it.
fn coordinate(name: &str, value: f32) -> String {
    if value == 0.0 || !value.is_finite() {
        String::new()
    } else {
        format!(" {name}=\"{}\"", num(value))
    }
}

fn hex(color: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

/// One element as an `<object>` around its cell: its label, Wren's id,
/// and what else the cell cannot hold. `inner` is the cell's own lines.
///
/// Every element starts a line of its own, as draw.io writes a file.
/// Lucidchart's draw.io import refuses a file whose `<object>`, `<mxCell>`
/// and `<mxGeometry>` share a line, though the XML is the same.
fn object(out: &mut String, cell: &str, id: &Id, label: &str, extra: &[(&str, &str)], inner: &str) {
    let _ = write!(
        out,
        "        <object label=\"{}\" wrenId=\"{id}\"",
        attribute(label)
    );
    for (key, value) in extra {
        let _ = write!(out, " {key}=\"{}\"", attribute(value));
    }
    let _ = writeln!(
        out,
        " id=\"{}\">\n{inner}        </object>",
        attribute(cell)
    );
}

/// A label as the HTML of an `html=1` cell: the text drawn (its first
/// [`scene::MAX_LABEL_BYTES`]), escaped, each line break a `<br>`, white
/// space at a line's end dropped as the canvas drops it, and a tab four
/// spaces. A label holding white space that HTML would collapse (at a
/// line's start, a run, or a tab) is set `white-space:pre-wrap`, which
/// keeps it and, as the canvas does, breaks a line only after a whole run
/// and lets the run hang past the line's end; draw.io's sanitiser keeps
/// that style. A label of only white space draws nothing.
fn html(text: &str) -> String {
    let text = scene::drawn_text(text);
    if text.trim().is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(text.len());
    let mut collapsed = false;
    let normal = text.replace("\r\n", "\n").replace('\r', "\n");
    for (row, line) in normal.split('\n').enumerate() {
        if row > 0 {
            out.push_str("<br>");
        }
        let kept: String = line.chars().filter(|&c| c == '\t' || carried(c)).collect();
        let mut spaced = true;
        for c in kept.trim_end().chars() {
            match c {
                ' ' => {
                    collapsed |= spaced;
                    out.push(' ');
                }
                '\t' => {
                    collapsed = true;
                    out.push_str("    ");
                }
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
            spaced = matches!(c, ' ' | '\t');
        }
    }
    if collapsed {
        format!("<div style=\"white-space:pre-wrap\">{out}</div>")
    } else {
        out
    }
}

/// An attribute value: markup escaped, the white space an XML reader would
/// fold as character references, and what XML cannot carry dropped.
fn attribute(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if !carried(c) => {}
            c => out.push(c),
        }
    }
    out
}

/// Whether XML 1.0 can carry `c` in text, line breaks and tabs aside.
fn carried(c: char) -> bool {
    !matches!(c, '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::scene::tests::random_board;
    use crate::style::tests::LIGHT;
    use crate::svg::tests::{document, Element as Xml};
    use crate::{fixtures, parse};

    fn export(text: &str) -> String {
        write(&parse(text).unwrap(), &LIGHT, "Board")
    }

    fn parsed(xml: &str) -> Vec<Xml> {
        document(xml).unwrap_or_else(|e| panic!("{e}\n{xml}"))
    }

    fn get<'x>(element: &'x Xml, key: &str) -> Option<&'x str> {
        element
            .1
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The `<object>` of the element with Wren's `id`.
    fn object<'x>(found: &'x [Xml], id: &str) -> &'x Xml {
        (found.iter())
            .find(|e| e.0 == "object" && get(e, "wrenId") == Some(id))
            .unwrap_or_else(|| panic!("no object for {id}"))
    }

    /// The element after the object of `id`: its `<mxCell>`.
    fn cell<'x>(found: &'x [Xml], id: &str) -> &'x Xml {
        let at = (found.iter())
            .position(|e| e.0 == "object" && get(e, "wrenId") == Some(id))
            .unwrap_or_else(|| panic!("no object for {id}"));
        &found[at + 1]
    }

    fn style<'x>(found: &'x [Xml], id: &str) -> &'x str {
        get(cell(found, id), "style").unwrap()
    }

    /// The key-value pairs of a style string, its bare tokens as keys with
    /// no value.
    fn keys(style: &str) -> BTreeMap<&str, &str> {
        (style.split(';'))
            .filter(|part| !part.is_empty())
            .map(|part| part.split_once('=').unwrap_or((part, "")))
            .collect()
    }

    fn node(id: &str, rest: &str) -> String {
        format!("\n[node.{id}]\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\nz = \"V\"\n{rest}")
    }

    #[test]
    fn an_empty_board_is_the_skeleton() {
        let xml = export("schema = 1\n");
        assert_eq!(
            xml,
            "<mxfile host=\"Wren\">\n  <diagram id=\"wren.board\" name=\"Board\">\n    \
             <mxGraphModel grid=\"1\" gridSize=\"20\" page=\"0\" math=\"0\" shadow=\"0\">\n      \
             <root>\n        <mxCell id=\"0\"/>\n        <mxCell id=\"1\" parent=\"0\"/>\n      \
             </root>\n    </mxGraphModel>\n  </diagram>\n</mxfile>\n"
        );
        let named = write(&Board::default(), &LIGHT, "a \"b\" <c> & d\u{0}\n");
        let found = parsed(&named);
        assert_eq!(get(&found[1], "name"), Some("a \"b\" <c> & d\n"));
    }

    /// Lucidchart imports a draw.io file only when each element starts a
    /// line of its own. Found by hand in November 2026: it refused the same
    /// cells written one `<object>` to a line, and took them once only the
    /// line breaks were added.
    #[test]
    fn every_element_starts_a_line_of_its_own() {
        let text = "schema = 1\n\n[node.a]\ntext = \"A\"\nkind = \"shape\"\nat = [0, 20]\nsize = [120, 60]\nz = \"V\"\n\n\
                    [edge.e]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { at = [300, 40] }\n\
                    labels = [\n  { text = \"one\" },\n  { text = \"two\" },\n]\nz = \"W\"\n";
        let xml = export(text);
        let cells = xml
            .split_once("<mxCell id=\"1\" parent=\"0\"/>\n")
            .and_then(|(_, rest)| rest.split_once("      </root>"))
            .map(|(cells, _)| cells)
            .unwrap();
        // Each line's indent and the tag it opens or closes.
        let outline: Vec<String> = cells
            .lines()
            .map(|line| {
                let tag = line.trim_start();
                let indent = line.len() - tag.len();
                format!("{indent} {}", tag.split(' ').next().unwrap())
            })
            .collect();
        assert_eq!(
            outline,
            [
                "8 <object",
                "10 <mxCell",
                "12 <mxGeometry",
                "10 </mxCell>",
                "8 </object>",
                "8 <object",
                "10 <mxCell",
                "12 <mxGeometry",
                "14 <mxPoint",
                "12 </mxGeometry>",
                "10 </mxCell>",
                "8 </object>",
                "8 <mxCell",
                "10 <mxGeometry",
                "12 <mxPoint",
                "10 </mxGeometry>",
                "8 </mxCell>",
            ],
            "{cells}"
        );
        for xml in [
            xml.clone(),
            write(&parse(COVERAGE).unwrap(), &LIGHT, "Coverage"),
        ] {
            assert!(!xml.contains("><"), "two elements share a line\n{xml}");
            for line in xml.lines() {
                let line = line.trim_start();
                assert!(line.starts_with('<') && line.ends_with('>'), "{line}");
            }
        }
    }

    #[test]
    fn every_shape_is_a_built_in_with_its_own_size() {
        let mut text = String::from("schema = 1\n");
        let shapes = [
            "rect",
            "rounded",
            "pill",
            "ellipse",
            "diamond",
            "parallelogram",
            "cylinder",
            "document",
            "hexagon",
        ];
        for (k, shape) in shapes.iter().enumerate() {
            text += &format!(
                "\n[node.s{k}]\nkind = \"shape\"\nat = [{}, 0]\nshape = \"{shape}\"\nsize = [120, 60]\nz = \"V\"\n",
                k * 140
            );
        }
        let found = parsed(&export(&text));
        let expect = [
            ("s0", "rounded=0;"),
            ("s1", "rounded=1;absoluteArcSize=1;arcSize=16;"),
            ("s2", "rounded=1;arcSize=50;"),
            ("s3", "ellipse;"),
            ("s4", "rhombus;"),
            (
                "s5",
                "shape=parallelogram;perimeter=parallelogramPerimeter;fixedSize=1;size=20;",
            ),
            (
                "s6",
                "shape=cylinder3;boundedLbl=1;backgroundOutline=1;size=10;",
            ),
            ("s7", "shape=document;boundedLbl=1;size=0.25;"),
            ("s8", "rounded=0;"),
        ];
        for (id, start) in expect {
            let style = style(&found, id);
            assert!(style.starts_with(start), "{id}: {style}");
            assert!(!style.contains("mxgraph."), "{id}: a stencil");
            let keys = keys(style);
            assert_eq!(keys["whiteSpace"], "wrap");
            assert_eq!(keys["html"], "1");
            assert_eq!(keys["fontSize"], "14");
            assert_eq!(keys["strokeWidth"], "1.5");
            assert!(!keys.contains_key("fontFamily"), "{id}: no font family");
        }
        assert_eq!(get(object(&found, "s8"), "wrenShape"), Some("hexagon"));
        assert_eq!(get(object(&found, "s7"), "wrenShape"), None);
        // A geometry at 0 is left out, as draw.io writes it.
        let geometry = &found[found.iter().position(|e| e.0 == "mxGeometry").unwrap()];
        assert_eq!(
            geometry.1,
            [
                ("width".to_owned(), "120".to_owned()),
                ("height".to_owned(), "60".to_owned()),
                ("as".to_owned(), "geometry".to_owned())
            ]
        );
    }

    /// A node's label size is its `fontSize`, clamped as the canvas draws
    /// it; a connector's labels stay small, whatever its style says.
    #[test]
    fn a_label_s_size_is_its_font_size() {
        let mut text = String::from("schema = 1\n");
        for (id, kind, size) in [
            ("a", "shape", "20"),
            ("b", "text", "28"),
            ("c", "sticker", "12"),
            ("d", "shape", "200"),
            ("e", "shape", "\"big\""),
        ] {
            text += &format!(
                "\n[node.{id}]\ntext = \"{id}\"\nkind = \"{kind}\"\nat = [0, 0]\nsize = [120, 60]\n\
                 style.text-size = {size}\nz = \"V\"\n"
            );
        }
        text += "\n[edge.f]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"b\" }\n\
                 labels = [\n  { text = \"f\" },\n]\nstyle.text-size = 28\nz = \"W\"\n";
        let found = parsed(&export(&text));
        let sizes: Vec<String> = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(|id| keys(style(&found, id))["fontSize"].to_owned())
            .collect();
        assert_eq!(sizes, ["20", "28", "12", "96", "14"]);
        // The connector's first label is the edge cell's own.
        assert_eq!(keys(style(&found, "f"))["fontSize"], "12");
    }

    #[test]
    fn colours_are_the_palettes_and_the_tokens_are_not_kept() {
        let mut text = String::from("schema = 1\n");
        for (k, (color, fill)) in [
            ("neutral", "tint"),
            ("red", "none"),
            ("green", "solid"),
            ("#123456", "tint"),
            ("#zzzzzz", "solid"),
            ("mauve", "sparkly"),
        ]
        .iter()
        .enumerate()
        {
            text += &node(
                &format!("c{k}"),
                &format!("style.color = \"{color}\"\nstyle.fill = \"{fill}\"\n"),
            );
        }
        let found = parsed(&export(&text));
        let colours = |id: &str| {
            let keys = keys(style(&found, id));
            [keys["fillColor"], keys["strokeColor"], keys["fontColor"]].map(str::to_owned)
        };
        assert_eq!(colours("c0"), ["#e4e5e5", "#202428", "#202428"]);
        assert_eq!(colours("c1"), ["none", "#c03030", "#202428"]);
        assert_eq!(colours("c2"), ["#309040", "#309040", "#ffffff"]);
        assert_eq!(colours("c3"), ["#e3e7eb", "#123456", "#202428"]);
        // A colour this build does not know draws as neutral, and so does
        // an unknown fill as tint: what the canvas draws.
        assert_eq!(colours("c4"), ["#e0e4e8", "#202428", "#202428"]);
        assert_eq!(colours("c5"), ["#e4e5e5", "#202428", "#202428"]);
        let xml = export(&text);
        for token in [
            "neutral", "red", "green", "mauve", "zzzzzz", "sparkly", "tint",
        ] {
            assert!(!xml.contains(token), "{token} in the export");
        }
    }

    #[test]
    fn free_text_and_placeholders_draw_as_the_canvas_draws_them() {
        let text = "schema = 1\n\n[node.f]\ntext = \"free\"\nkind = \"text\"\nat = [10, 20]\n\
                    size = [150, 60]\nstyle.color = \"blue\"\nz = \"V\"\n\n[node.m]\n\
                    text = \"mystery\"\nkind = \"sticker\"\nat = [0, 100]\nsize = [80, 40]\n\
                    src = \"/etc/hostname\"\nlink = \"javascript:alert(1)\"\nz = \"W\"\n\n\
                    [node.n]\nat = [0, 200]\nsize = [80, 40]\nz = \"X\"\n";
        let xml = export(text);
        let found = parsed(&xml);
        let free = keys(style(&found, "f"));
        assert!(style(&found, "f").starts_with("text;"));
        assert_eq!(
            (
                free["fontColor"],
                free["fillColor"],
                free["strokeColor"],
                free["align"]
            ),
            ("#3060d0", "none", "none", "left")
        );
        assert_eq!(free["verticalAlign"], "top");
        for id in ["m", "n"] {
            let keys = keys(style(&found, id));
            assert_eq!(
                (keys["dashed"], keys["fillColor"], keys["strokeColor"]),
                ("1", "none", "#808890"),
                "{id}"
            );
            assert_eq!(keys["dashPattern"], "6 4");
        }
        assert_eq!(get(object(&found, "m"), "wrenKind"), Some("sticker"));
        assert_eq!(get(object(&found, "n"), "wrenKind"), None);
        // Ring 0 reads no file and follows no link: neither reaches draw.io.
        for absent in ["hostname", "javascript", "link=", "image="] {
            assert!(!xml.contains(absent), "{absent} in the export");
        }
    }

    #[test]
    fn labels_are_escaped_html_that_keeps_their_spaces() {
        let cases = [
            ("a < b & \"c\"", "a &lt; b &amp; &quot;c&quot;"),
            ("<b>not bold</b>", "&lt;b&gt;not bold&lt;/b&gt;"),
            ("two\nlines", "two<br>lines"),
            ("crlf\r\nand\rcr", "crlf<br>and<br>cr"),
            // White space HTML would collapse keeps its width in pre-wrap,
            // which breaks a line after a whole run, as the canvas does.
            (
                "  lead  and   runs  ",
                "<div style=\"white-space:pre-wrap\">  lead  and   runs</div>",
            ),
            (
                "alpha     beta",
                "<div style=\"white-space:pre-wrap\">alpha     beta</div>",
            ),
            (
                "tab\there",
                "<div style=\"white-space:pre-wrap\">tab    here</div>",
            ),
            (
                "one\n two",
                "<div style=\"white-space:pre-wrap\">one<br> two</div>",
            ),
            (
                "a \u{7} b \u{7}",
                "<div style=\"white-space:pre-wrap\">a  b</div>",
            ),
            ("one space each ", "one space each"),
            ("blank\n\nline", "blank<br><br>line"),
            ("a\u{0}b\u{7}c\u{fffe}\u{ffff}d", "abcd"),
            ("no\u{a0}break", "no\u{a0}break"),
            ("; key=value; 100%", "; key=value; 100%"),
            ("  \n\t ", ""),
            ("שלום 漢字 👩🏽‍💻", "שלום 漢字 👩🏽‍💻"),
        ];
        for (label, want) in cases {
            assert_eq!(html(label), want, "{label:?}");
            let mut board = Board::default();
            let mut node = parse(&format!("schema = 1\n{}", node("n", ""))).unwrap();
            node.nodes.values_mut().for_each(|n| n.text = label.into());
            board.nodes = node.nodes;
            let found = parsed(&write(&board, &LIGHT, "Board"));
            assert_eq!(get(object(&found, "n"), "label"), Some(want), "{label:?}");
        }
        // The label drawn: its first 16 KiB.
        let long = "é".repeat(scene::MAX_LABEL_BYTES);
        assert_eq!(html(&long).len(), scene::MAX_LABEL_BYTES);
    }

    #[test]
    fn stacking_is_wrens_with_connectors_in_the_layer() {
        // Edges and nodes interleave by z; a child follows its parent at
        // once whatever its own key; ties fall to the id.
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [300, 300]\nz = \"5\"\n\n\
                    [node.b]\nkind = \"shape\"\nat = [20, 20]\nparent = \"a\"\nsize = [40, 40]\nz = \"z\"\n\n\
                    [node.c]\nkind = \"shape\"\nat = [400, 0]\nsize = [40, 40]\nz = \"1\"\n\n\
                    [node.d]\nkind = \"shape\"\nat = [400, 100]\nsize = [40, 40]\nz = \"9\"\n\n\
                    [node.e]\nkind = \"shape\"\nat = [-10, 30]\nparent = \"b\"\nsize = [10, 10]\nz = \"0\"\n\n\
                    [edge.k]\nkind = \"edge\"\nfrom = { node = \"b\" }\nto = { node = \"e\" }\nz = \"7\"\n\n\
                    [edge.l]\nkind = \"edge\"\nfrom = { node = \"c\" }\nto = { node = \"d\" }\nz = \"5\"\n";
        let found = parsed(&export(text));
        let order: Vec<&str> = (found.iter())
            .filter(|e| e.0 == "object")
            .map(|e| get(e, "wrenId").unwrap())
            .collect();
        assert_eq!(order, ["c", "a", "b", "e", "l", "k", "d"]);
        // A child's geometry is relative to its parent's corner.
        let parent = |id: &str| get(cell(&found, id), "parent").unwrap().to_owned();
        assert_eq!(parent("b"), "a");
        assert_eq!(parent("e"), "b");
        assert_eq!(parent("k"), "1");
        let at = found
            .iter()
            .position(|e| get(e, "wrenId") == Some("e"))
            .unwrap();
        assert_eq!(get(&found[at + 2], "x"), Some("-10"));
        assert_eq!(get(&found[at + 2], "y"), Some("30"));
        for id in ["k", "l"] {
            let keys = keys(style(&found, id));
            assert!(
                !keys.keys().any(|k| k.starts_with("wren")),
                "{id}: no wren keys in a style"
            );
        }
    }

    #[test]
    fn a_missing_parent_and_a_cycle_are_laid_out_as_roots() {
        let text = "schema = 1\n\n[node.o]\nkind = \"shape\"\nat = [5, 6]\nparent = \"gone\"\nsize = [10, 10]\nz = \"V\"\n\n\
                    [node.p]\nkind = \"shape\"\nat = [7, 8]\nparent = \"q\"\nsize = [10, 10]\nz = \"V\"\n\n\
                    [node.q]\nkind = \"shape\"\nat = [9, 10]\nparent = \"p\"\nsize = [10, 10]\nz = \"V\"\n\n\
                    [node.s]\nkind = \"shape\"\nat = [11, 12]\nparent = \"s\"\nsize = [10, 10]\nz = \"V\"\n";
        let found = parsed(&export(text));
        for (id, x) in [("o", "5"), ("p", "7"), ("q", "9"), ("s", "11")] {
            assert_eq!(get(cell(&found, id), "parent"), Some("1"), "{id}");
            let at = found
                .iter()
                .position(|e| get(e, "wrenId") == Some(id))
                .unwrap();
            assert_eq!(get(&found[at + 2], "x"), Some(x), "{id}");
        }
    }

    #[test]
    fn ids_draw_io_reserves_or_that_two_elements_share_are_made_unique() {
        let text = "schema = 1\n\n[node.0]\nkind = \"shape\"\nat = [0, 0]\nsize = [10, 10]\nz = \"V\"\n\n\
                    [node.1]\nkind = \"shape\"\nat = [50, 0]\nsize = [10, 10]\nz = \"V\"\n\n\
                    [node.x]\nkind = \"shape\"\nat = [100, 0]\nsize = [10, 10]\nz = \"V\"\n\n\
                    [edge.0]\nkind = \"edge\"\nfrom = { node = \"0\" }\nto = { node = \"1\" }\nlabels = [\n  { text = \"a\" },\n  { text = \"b\" },\n]\nz = \"W\"\n\n\
                    [edge.x]\nkind = \"edge\"\nfrom = { node = \"x\" }\nto = { node = \"1\" }\nz = \"W\"\n";
        let found = parsed(&export(text));
        let ids: Vec<&str> = found.iter().filter_map(|e| get(e, "id")).collect();
        assert_eq!(
            ids,
            [
                "wren.board",
                "0",
                "1",
                "0.node",
                "1.node",
                "x",
                "0.edge",
                "0.edge.label1",
                "x.edge"
            ]
        );
        let at = found
            .iter()
            .position(|e| get(e, "id") == Some("0.edge"))
            .unwrap();
        let edge = &found[at + 1];
        assert_eq!(get(edge, "source"), Some("0.node"));
        assert_eq!(get(edge, "target"), Some("1.node"));
        // Every element keeps Wren's id beside the cell's.
        let wren: Vec<&str> = found.iter().filter_map(|e| get(e, "wrenId")).collect();
        assert_eq!(wren, ["0", "1", "x", "0", "x"]);
    }

    #[test]
    fn connectors_keep_their_heads_route_dashes_and_ends() {
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [100, 50]\nz = \"V\"\n\n\
                    [node.b]\nkind = \"shape\"\nat = [300, 0]\nshape = \"diamond\"\nsize = [100, 50]\nz = \"V\"\n\n\
                    [edge.d]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"b\" }\nz = \"W\"\n\n\
                    [edge.s]\nkind = \"edge\"\nfrom = { node = \"a\", side = \"top\" }\n\
                    to = { anchor = [100, 250], node = \"b\" }\nheads = [\"arrow\", \"none\"]\nroute = \"straight\"\n\
                    style.color = \"red\"\nstyle.stroke = \"dashed\"\nz = \"W\"\n\n\
                    [edge.u]\nkind = \"wire\"\nfrom = { at = [10, -20] }\nto = { node = \"b\", side = \"sideways\" }\n\
                    heads = [\"triangle\", \"none\"]\nroute = \"curved\"\nz = \"W\"\n";
        let found = parsed(&export(text));
        let d = keys(style(&found, "d"));
        assert_eq!(d["edgeStyle"], "orthogonalEdgeStyle");
        assert_eq!(
            (d["rounded"], d["startArrow"], d["endArrow"]),
            ("0", "none", "block")
        );
        assert_eq!((d["endFill"], d["endSize"]), ("1", "8.5"));
        assert_eq!((d["strokeColor"], d["fontColor"]), ("#202428", "#202428"));
        assert_eq!(
            (d["fontSize"], d["labelBackgroundColor"]),
            ("12", "default")
        );
        assert!(
            !d.contains_key("exitX") && !d.contains_key("entryX"),
            "floating"
        );
        let s = keys(style(&found, "s"));
        assert!(!s.contains_key("edgeStyle"));
        assert_eq!((s["startArrow"], s["endArrow"]), ("block", "none"));
        assert_eq!(
            (s["strokeColor"], s["dashed"], s["dashPattern"]),
            ("#c03030", "1", "6 4")
        );
        assert_eq!(
            (s["exitX"], s["exitY"], s["exitDx"], s["exitDy"]),
            ("0.5", "0", "0", "0")
        );
        assert!(
            !s.contains_key("exitPerimeter"),
            "a side's middle is on the outline"
        );
        // An anchor is the box's point, not one moved onto the diamond.
        assert_eq!(
            (s["entryX"], s["entryY"], s["entryPerimeter"]),
            ("0.1", "0.25", "0")
        );
        // An unknown head draws as an arrow, an unknown route straight and an
        // unknown side as none; the connector's unknown kind is kept.
        let u = keys(style(&found, "u"));
        assert_eq!((u["startArrow"], u["endArrow"]), ("block", "none"));
        assert!(!u.contains_key("edgeStyle") && !u.contains_key("curved"));
        assert!(!u.contains_key("entryX"));
        assert_eq!(get(object(&found, "u"), "wrenKind"), Some("wire"));
        let u = cell(&found, "u");
        assert_eq!((get(u, "source"), get(u, "target")), (None, Some("b")));
        let at = found
            .iter()
            .position(|e| get(e, "wrenId") == Some("u"))
            .unwrap();
        assert_eq!(found[at + 3].0, "mxPoint");
        assert_eq!(
            (
                get(&found[at + 3], "x"),
                get(&found[at + 3], "y"),
                get(&found[at + 3], "as")
            ),
            (Some("10"), Some("-20"), Some("sourcePoint"))
        );
    }

    /// Every attached end of an elbow connector names the side it leaves
    /// by, or draw.io guesses: from a parallelogram's slanted left side it
    /// ran the first segment right, across the shape.
    #[test]
    fn an_elbow_connectors_pinned_and_anchored_ends_leave_by_their_sides() {
        let text =
            "schema = 1\n\n[node.p]\nkind = \"shape\"\nat = [280, 0]\nshape = \"parallelogram\"\n\
                    size = [120, 60]\nz = \"V\"\n\n\
                    [node.r]\nkind = \"shape\"\nat = [280, 200]\nsize = [120, 60]\nz = \"V\"\n\n\
                    [edge.e]\nkind = \"edge\"\nfrom = { node = \"p\", side = \"left\" }\n\
                    to = { node = \"r\", side = \"left\" }\nz = \"W\"\n\n\
                    [edge.a]\nkind = \"edge\"\nfrom = { anchor = [1000, 400], node = \"p\" }\n\
                    to = { anchor = [500, 950], node = \"r\" }\nz = \"W\"\n\n\
                    [edge.s]\nkind = \"edge\"\nfrom = { node = \"p\", side = \"left\" }\n\
                    to = { node = \"r\", side = \"left\" }\nroute = \"straight\"\nz = \"W\"\n";
        let found = parsed(&export(text));
        let e = keys(style(&found, "e"));
        assert_eq!(
            (e["sourcePortConstraint"], e["targetPortConstraint"]),
            ("west", "west")
        );
        assert_eq!((e["exitX"], e["exitY"]), ("0", "0.5"));
        // An anchor leaves by the side of the box nearest to it.
        let a = keys(style(&found, "a"));
        assert_eq!(
            (a["sourcePortConstraint"], a["targetPortConstraint"]),
            ("east", "south")
        );
        // A straight connector has no bends for draw.io to choose.
        let s = keys(style(&found, "s"));
        assert!(!s.contains_key("sourcePortConstraint") && !s.contains_key("targetPortConstraint"));
    }

    /// A document's pinned bottom meets the middle of its wave, as the
    /// canvas draws it, not the box's bottom a wave's height below.
    #[test]
    fn a_documents_pinned_bottom_meets_its_wave() {
        let text =
            "schema = 1\n\n[node.doc]\nkind = \"shape\"\nat = [600, 0]\nshape = \"document\"\n\
                    size = [120, 80]\nz = \"V\"\n\n\
                    [node.r]\nkind = \"shape\"\nat = [600, 200]\nsize = [120, 60]\nz = \"V\"\n\n\
                    [edge.d]\nkind = \"edge\"\nfrom = { node = \"doc\", side = \"bottom\" }\n\
                    to = { node = \"r\", side = \"top\" }\nz = \"W\"\n\n\
                    [edge.u]\nkind = \"edge\"\nfrom = { node = \"r\", side = \"bottom\" }\n\
                    to = { node = \"doc\", side = \"bottom\" }\nroute = \"straight\"\nz = \"W\"\n";
        let found = parsed(&export(text));
        let d = keys(style(&found, "d"));
        // 80 high, a wave of 8: the middle of the wave is 72 down.
        assert_eq!(
            (d["exitX"], d["exitY"], d["exitPerimeter"]),
            ("0.5", "0.9", "0")
        );
        assert_eq!((d["entryY"], d.get("entryPerimeter")), ("0", None));
        let u = keys(style(&found, "u"));
        assert_eq!((u["exitY"], u.get("exitPerimeter")), ("1", None));
        assert_eq!((u["entryY"], u["entryPerimeter"]), ("0.9", "0"));
    }

    #[test]
    fn a_dangling_end_is_the_tip_of_its_stub_and_two_draw_nothing() {
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [100, 50]\nz = \"V\"\n\n\
                    [edge.t]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"gone\" }\nz = \"W\"\n\n\
                    [edge.f]\nkind = \"edge\"\nfrom = { node = \"<gone>\" }\nto = { node = \"a\", side = \"bottom\" }\nz = \"W\"\n\n\
                    [edge.n]\nkind = \"edge\"\nfrom = { node = \"x\" }\nto = { node = \"y\" }\nz = \"W\"\n";
        let found = parsed(&export(text));
        let point = |id: &str| {
            let at = found
                .iter()
                .position(|e| get(e, "wrenId") == Some(id))
                .unwrap();
            let p = &found[at + 3];
            assert_eq!(p.0, "mxPoint");
            (
                get(p, "x").unwrap_or("0").to_owned(),
                get(p, "y").unwrap_or("0").to_owned(),
                get(p, "as").unwrap().to_owned(),
            )
        };
        // Out of the right side's middle by the stub's 24 units.
        assert_eq!(
            point("t"),
            ("124".into(), "25".into(), "targetPoint".into())
        );
        assert_eq!(get(object(&found, "t"), "wrenTo"), Some("gone"));
        // Out of the pinned side.
        assert_eq!(point("f"), ("50".into(), "74".into(), "sourcePoint".into()));
        assert_eq!(get(object(&found, "f"), "wrenFrom"), Some("<gone>"));
        assert_eq!(get(cell(&found, "f"), "target"), Some("a"));
        assert!(found.iter().all(|e| get(e, "wrenId") != Some("n")));
    }

    #[test]
    fn labels_are_the_connectors_own_then_cells_of_their_own() {
        let mut labels = String::new();
        for k in 0..20 {
            labels += &format!(
                "  {{ at = {}, text = \"L{k}\" }},\n",
                [-50, 0, 250, 1000, 1500][k % 5]
            );
        }
        let text = format!(
            "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [10, 10]\nz = \"V\"\n\n\
             [node.b]\nkind = \"shape\"\nat = [300, 0]\nsize = [10, 10]\nz = \"V\"\n\n\
             [edge.e]\nkind = \"edge\"\nfrom = {{ node = \"a\" }}\nto = {{ node = \"b\" }}\nlabels = [\n{labels}]\nz = \"W\"\n\n\
             [edge.m]\nkind = \"edge\"\nfrom = {{ node = \"a\" }}\nto = {{ node = \"b\" }}\nlabels = [\n  {{ text = \"mid\" }},\n]\nz = \"W\"\n"
        );
        let found = parsed(&export(&text));
        assert_eq!(get(object(&found, "e"), "label"), Some("L0"));
        let at = found
            .iter()
            .position(|e| get(e, "wrenId") == Some("e"))
            .unwrap();
        assert_eq!(get(&found[at + 2], "x"), Some("-1"), "clamped to the start");
        let cells: Vec<&Xml> = (found.iter())
            .filter(|e| get(e, "parent") == Some("e"))
            .collect();
        // The first 16 are drawn; the connector holds one of them.
        assert_eq!(cells.len(), scene::MAX_LABELS - 1);
        let xs: Vec<Option<&str>> = (found.iter())
            .filter(|e| e.0 == "mxGeometry" && get(e, "relative") == Some("1"))
            .map(|e| get(e, "x"))
            .collect();
        assert_eq!(
            &xs[..6],
            [
                Some("-1"),
                Some("-1"),
                Some("-0.5"),
                Some("1"),
                Some("1"),
                Some("-1")
            ]
        );
        assert_eq!(get(cells[0], "value"), Some("L1"));
        assert_eq!(get(cells[0], "id"), Some("e.label1"));
        assert_eq!(get(cells[0], "connectable"), Some("0"));
        // A label in the middle has no x.
        let at = found
            .iter()
            .position(|e| get(e, "wrenId") == Some("m"))
            .unwrap();
        assert_eq!(get(&found[at + 2], "x"), None);
    }

    #[test]
    fn numbers_are_exact_decimals() {
        assert_eq!(
            [0, 1, 10, 100, 125, 250, 500, 999, 1000, -5, 2000].map(per_mille),
            ["0", "0.001", "0.01", "0.1", "0.125", "0.25", "0.5", "0.999", "1", "0", "1"]
        );
        assert_eq!(
            [0, 250, 500, 750, 1000].map(along),
            [-1.0, -0.5, 0.0, 0.5, 1.0]
        );
        assert_eq!(coordinate("x", 0.0), "");
        assert_eq!(coordinate("x", -0.0), "");
        assert_eq!(coordinate("y", -12.5), " y=\"-12.5\"");
    }

    #[test]
    fn out_of_range_values_are_written_as_drawn() {
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [2147483647, -2147483648]\nsize = [-5, 0]\nz = \"V\"\n";
        let found = parsed(&export(text));
        let at = found
            .iter()
            .position(|e| get(e, "wrenId") == Some("a"))
            .unwrap();
        let geometry = &found[at + 2];
        assert_eq!(
            [
                get(geometry, "x"),
                get(geometry, "y"),
                get(geometry, "width"),
                get(geometry, "height")
            ],
            [Some("16777215"), Some("-16777216"), Some("1"), Some("1")]
        );
    }

    #[test]
    fn the_export_is_deterministic() {
        for (name, text) in fixtures::all() {
            let Ok(board) = parse(text) else { continue };
            assert_eq!(
                write(&board, &LIGHT, name),
                write(&board.clone(), &LIGHT, name),
                "{name}"
            );
        }
    }

    /// Every element is one `<object>`, and every id is unique; each node
    /// and each connector with an end drawn has a cell.
    pub(crate) fn holds(board: &Board, xml: &str) {
        let found = parsed(xml);
        let ids: Vec<&str> = found.iter().filter_map(|e| get(e, "id")).collect();
        let unique: std::collections::BTreeSet<&str> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "duplicate ids\n{xml}");
        let objects = found.iter().filter(|e| e.0 == "object").count();
        let drawn_edges = (board.edges.values())
            .filter(|edge| {
                [&edge.from, &edge.to].iter().any(|end| match end {
                    End::Free { .. } => true,
                    End::Node { node, .. } => board.nodes.contains_key(node.as_str()),
                })
            })
            .count();
        assert_eq!(objects, board.nodes.len() + drawn_edges, "{xml}");
        // Every cell's parent and every terminal is a cell written before
        // it, or the layer.
        let mut seen = std::collections::BTreeSet::from(["0"]);
        let mut last_id = None;
        for element in &found {
            if let Some(id) = get(element, "id") {
                last_id = Some(id);
            }
            if element.0 == "mxCell" {
                for key in ["parent", "source", "target"] {
                    if let Some(other) = get(element, key) {
                        assert!(
                            seen.contains(other) || key != "parent",
                            "{key} {other} before it is written\n{xml}"
                        );
                    }
                }
                if let Some(id) = get(element, "id").or(last_id) {
                    seen.insert(id);
                }
            }
        }
    }

    /// The draw.io study's second board, every case the sample flow leaves
    /// out, written in schema 1 (`golden/drawio/coverage.wren-board`).
    const COVERAGE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/golden/drawio/coverage.wren-board"
    ));

    #[test]
    fn the_coverage_board_exports_every_element_it_draws() {
        let board = parse(COVERAGE).unwrap();
        assert_eq!(crate::write(&board), COVERAGE, "the board is canonical");
        assert_eq!((board.nodes.len(), board.edges.len()), (126, 45));
        let xml = write(&board, &LIGHT, "Coverage");
        holds(&board, &xml);
        let found = parsed(&xml);
        // The ids draw.io keeps for itself, a node in a cycle and one whose
        // parent is gone are top-level cells; nested frames nest.
        for (id, parent) in [
            ("0", "1"),
            ("cyc-a", "1"),
            ("cyc-self", "1"),
            ("orphan", "1"),
            ("fc", "frame-inner"),
            ("frame-inner", "frame-outer"),
        ] {
            assert_eq!(get(cell(&found, id), "parent"), Some(parent), "{id}");
        }
        assert_eq!(
            get(object(&found, "x-dangling"), "wrenTo"),
            Some("deleted-node")
        );
        assert_eq!(get(object(&found, "note-a"), "wrenKind"), Some("note"));
        // An elbow connector's floating end leaves by the side Wren's router
        // chose; a pinned end by its own.
        let r2 = keys(style(&found, "r2"));
        assert_eq!(
            (r2["sourcePortConstraint"], r2["targetPortConstraint"]),
            ("south", "west")
        );
        let pinned = keys(style(&found, "self"));
        assert_eq!(
            (
                pinned["sourcePortConstraint"],
                pinned["targetPortConstraint"]
            ),
            ("east", "south")
        );
        assert_eq!((pinned["exitX"], pinned["entryY"]), ("1", "1"));
        // Ring 0 reads no file a node names: no image is embedded.
        assert!(!xml.contains("data:image") && !xml.contains("warehouse"));
    }

    #[test]
    fn random_boards_export_as_well_formed_files() {
        let mut rng = fixtures::Rng(21);
        for _ in 0..300 {
            let board = random_board(&mut rng);
            holds(&board, &write(&board, &LIGHT, "Random"));
        }
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_random_boards_export_as_well_formed_files() {
        let mut rng = fixtures::Rng(22);
        for _ in 0..20_000 {
            let board = random_board(&mut rng);
            holds(&board, &write(&board, &LIGHT, "Random"));
        }
    }
}
