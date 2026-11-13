//! The display list of a board: what the canvas widget paints and the SVG
//! writer serialises, so the two cannot disagree.
//!
//! Coordinates are board units as `f32`: one logical pixel at zoom 1, x
//! growing right and y growing down. Labels arrive wrapped into lines, each
//! placed and measured, so a painter never lays text out itself.

use std::collections::{BTreeMap, HashMap};

use unicode_segmentation::UnicodeSegmentation;

use crate::route::{self, Attach, Connector, Routed, Terminal, STROKE_WIDTH};
use crate::style::{self, Resolved};
use crate::{
    check, placements, shape, Board, Dash, Edge, End, Fill, Id, Named, Node, NodeKind, Palette,
    Rgba, Shape, Side,
};

mod wrap;

/// A node's label: Inter Regular at a fixed logical size, whatever the
/// editor's font settings.
pub const LABEL_SIZE: f32 = 14.0;
pub const LABEL_LINE_HEIGHT: f32 = 20.0;
/// A connector's label and a placeholder's caption.
pub const SMALL_SIZE: f32 = 12.0;
pub const SMALL_LINE_HEIGHT: f32 = 16.0;
/// The canvas-coloured box behind a connector label reaches this far past
/// its text, across and then down.
pub const EDGE_LABEL_PAD: [f32; 2] = [4.0, 2.0];
/// A placeholder's caption is this far in from its box's top-left corner.
pub const CAPTION_INSET: f32 = 4.0;
/// A connector with one end naming no node is drawn as a stub this long
/// from its other end, ending in a cross stroke this long.
pub const STUB_LENGTH: f32 = 24.0;
pub const STUB_CROSS: f32 = 8.0;
/// The diameter of the dot marking a flagged element.
pub const MARKER_SIZE: f32 = 6.0;
/// Resolved coordinates and sizes are drawn within this range, which `f32`
/// holds exactly; the stored values are untouched.
pub const COORDINATE_LIMIT: i64 = 1 << 24;
/// Only this much of a label is drawn, cut at a character boundary.
pub const MAX_LABEL_BYTES: usize = 16 * 1024;
/// Only this many of a connector's labels are drawn.
pub const MAX_LABELS: usize = 16;
/// Elements are stacked by at most this many bytes of their `z`.
pub const MAX_Z: usize = crate::z::MAX_LEN;

/// The height of one line of text at `size`: §3.7's line heights, which
/// the scene lays out with and every painter and writer places text by.
pub fn line_height(size: f32) -> f32 {
    if size <= SMALL_SIZE {
        SMALL_LINE_HEIGHT
    } else {
        size * LABEL_LINE_HEIGHT / LABEL_SIZE
    }
}

/// Text measurement, supplied by the host: the canvas measures with the
/// font it draws with, tests with [`FixedMeasure`].
pub trait Measure {
    /// The advance width of `text` on one line.
    fn width(&self, text: &str, face: Face, size: f32) -> f32;
    fn line_height(&self, face: Face, size: f32) -> f32;
}

/// A label typeface. Ring 0 draws every label in Inter Regular.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Regular,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// A board's display list.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// The union of the items' bounds.
    pub bounds: Rect,
    /// Back to front.
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The element the item belongs to.
    pub source: Source,
    /// Everything the item paints, strokes and text included.
    pub bounds: Rect,
    pub draw: Draw,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Node(Id),
    Edge(Id),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Path {
        segments: Vec<Segment>,
        fill: Option<Rgba>,
        stroke: Option<Stroke>,
    },
    Text {
        lines: Vec<Line>,
        face: Face,
        size: f32,
        color: Rgba,
    },
}

/// One step of a path, in the order SVG and iced's path builder take them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    MoveTo(Point),
    LineTo(Point),
    QuadTo {
        control: Point,
        to: Point,
    },
    CubicTo {
        first: Point,
        second: Point,
        to: Point,
    },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f32,
    /// On and off lengths; `None` is a solid line.
    pub dash: Option<[f32; 2]>,
}

/// One wrapped line of a label.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// The top-left corner of the line's box.
    pub origin: Point,
    /// The measured advance width of `text`.
    pub width: f32,
}

/// A measurer with no font, for tests and golden scene dumps: every
/// grapheme is half the font size wide, and a line is 1.25 times the font
/// size tall. Both factors are exact in binary, so dumps do not drift.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedMeasure;

impl Measure for FixedMeasure {
    fn width(&self, text: &str, _face: Face, size: f32) -> f32 {
        text.graphemes(true).count() as f32 * size * 0.5
    }

    fn line_height(&self, _face: Face, size: f32) -> f32 {
        size * 1.25
    }
}

/// The display list of `board` in `palette`, with labels wrapped by
/// `measure`. Every rule of the plan's §3.7 is applied here and nowhere
/// else: the parent transform, stacking, shapes, colours, labels, the
/// placeholder of an unknown kind, the stub of a dangling connector, the
/// issue markers, and clamped drawing of out-of-range values.
pub fn build(board: &Board, palette: &Palette, measure: &dyn Measure) -> Scene {
    let placed = placements(board);
    let boxes: BTreeMap<&Id, Rect> = board
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            let corner = placed.get(id)?.at;
            Some((id, node_box(corner, node.size)))
        })
        .collect();
    let mut children: BTreeMap<&Id, Vec<Element<'_>>> = BTreeMap::new();
    let mut roots: Vec<Element<'_>> = board
        .edges
        .iter()
        .enumerate()
        .map(|(k, (id, edge))| Element::Edge(id, edge, k))
        .collect();
    for (id, node) in &board.nodes {
        match placed.get(id).and_then(|p| p.parent) {
            Some(parent) => children
                .entry(parent)
                .or_default()
                .push(Element::Node(id, node)),
            None => roots.push(Element::Node(id, node)),
        }
    }

    let places = places(board, &boxes);
    let routes = routes(board, &boxes, &places);
    let mut painter = Painter {
        palette,
        measure,
        items: Vec::new(),
        corners: BTreeMap::new(),
    };
    roots.sort_by(|a, b| a.order().cmp(&b.order()));
    let mut scopes = vec![roots.into_iter()];
    while let Some(scope) = scopes.last_mut() {
        let Some(element) = scope.next() else {
            scopes.pop();
            continue;
        };
        match element {
            Element::Node(id, node) => {
                if let Some(&bounds) = boxes.get(id) {
                    painter.node(id, node, bounds);
                }
                if let Some(mut inner) = children.remove(id) {
                    inner.sort_by(|a, b| a.order().cmp(&b.order()));
                    scopes.push(inner.into_iter());
                }
            }
            Element::Edge(id, edge, k) => {
                painter.edge(id, edge, &places, routes.get(k).and_then(Option::as_ref));
            }
        }
    }

    let mut flagged: Vec<Source> = Vec::new();
    for issue in check(board) {
        if !flagged.contains(&issue.element) {
            flagged.push(issue.element);
        }
    }
    for source in flagged {
        let corner = match &source {
            Source::Node(id) => boxes.get(id).map(|r| Point::new(r.x, r.y)),
            Source::Edge(id) => painter.corners.get(id).copied(),
        };
        if let Some(corner) = corner {
            painter.marker(source, corner);
        }
    }

    let items = painter.items;
    let bounds = items
        .iter()
        .map(|item| item.bounds)
        .reduce(union)
        .unwrap_or_default();
    Scene { bounds, items }
}

/// A drawn node as a connector end finds it: its index among the boxes
/// given to the router, the node, and its box.
#[derive(Clone, Copy)]
struct Place<'a> {
    index: usize,
    node: &'a Node,
    bounds: Rect,
}

/// The drawn nodes by id, for connector ends: one hash lookup each, where
/// walking the ordered maps by string three times over cost as much as
/// routing a 1,000-shape grid.
fn places<'a>(board: &'a Board, boxes: &BTreeMap<&'a Id, Rect>) -> HashMap<&'a str, Place<'a>> {
    // `boxes` holds some of the nodes, in the same order.
    let mut nodes = board.nodes.iter();
    let mut out = HashMap::with_capacity(boxes.len());
    for (index, (&id, &bounds)) in boxes.iter().enumerate() {
        if let Some((_, node)) = nodes.find(|&(stored, _)| stored == id) {
            out.insert(
                id.as_str(),
                Place {
                    index,
                    node,
                    bounds,
                },
            );
        }
    }
    out
}

/// Where a connector end is drawn: a free point, or the box, shape and
/// attachment of the node it names, with that node's index among the
/// boxes. `None` when it names no drawn node.
fn terminal(end: &End, places: &HashMap<&str, Place<'_>>) -> Option<(Terminal, Option<usize>)> {
    match end {
        End::Free { at, .. } => Some((Terminal::Point(clamp_point(*at)), None)),
        End::Node {
            node, side, anchor, ..
        } => {
            let place = places.get(node.as_str())?;
            let shape = match place.node.kind.as_ref().and_then(Named::known) {
                Some(NodeKind::Shape) => shape::drawn(&place.node.shape),
                _ => Shape::Rect,
            };
            let terminal = Terminal::Node {
                bounds: place.bounds,
                shape,
                attach: Attach::of(side.as_ref(), *anchor),
            };
            Some((terminal, Some(place.index)))
        }
    }
}

/// The route of every connector whose two ends are drawn, routed among
/// all the node boxes in id order (§3.7), by the connector's position in
/// `board.edges`.
fn routes(
    board: &Board,
    boxes: &BTreeMap<&Id, Rect>,
    places: &HashMap<&str, Place<'_>>,
) -> Vec<Option<Routed>> {
    let rects: Vec<Rect> = boxes.values().copied().collect();
    let mut positions = Vec::new();
    let mut connectors = Vec::new();
    for (k, edge) in board.edges.values().enumerate() {
        let (Some((from, a)), Some((to, b))) =
            (terminal(&edge.from, places), terminal(&edge.to, places))
        else {
            continue;
        };
        positions.push(k);
        connectors.push(Connector {
            kind: route::drawn(&edge.route),
            ends: [from, to],
            nodes: [a, b],
        });
    }
    let mut out = vec![None; board.edges.len()];
    for (k, routed) in positions
        .into_iter()
        .zip(route::route_all(&rects, &connectors))
    {
        if let Some(slot) = out.get_mut(k) {
            *slot = Some(routed);
        }
    }
    out
}

/// One element in stacking order.
#[derive(Clone, Copy)]
enum Element<'a> {
    Node(&'a Id, &'a Node),
    /// A connector, with its position in `board.edges`.
    Edge(&'a Id, &'a Edge, usize),
}

impl<'a> Element<'a> {
    /// Ascending `z` bytewise (a missing one as empty, a long one by its
    /// first [`MAX_Z`] bytes), then id, then a node before a connector.
    fn order(&self) -> (&'a [u8], &'a str, bool) {
        match *self {
            Self::Node(id, node) => crate::z::order(node.z.as_deref(), id, false),
            Self::Edge(id, edge, _) => crate::z::order(edge.z.as_deref(), id, true),
        }
    }
}

/// A node's absolute box, clamped so that all of it lies within
/// ±[`COORDINATE_LIMIT`] and it is at least 1 by 1: every corner is then an
/// integer that `f32` holds exactly.
fn node_box(corner: [i64; 2], size: [i32; 2]) -> Rect {
    let limit = COORDINATE_LIMIT;
    let span = |start: i64, length: i32| {
        let start = start.clamp(-limit, limit - 1);
        let length = i64::from(length).clamp(1, limit - start);
        (start as f32, length as f32)
    };
    let (x, w) = span(corner[0], size[0]);
    let (y, h) = span(corner[1], size[1]);
    Rect::new(x, y, w, h)
}

fn clamp_point(at: [i32; 2]) -> Point {
    let limit = COORDINATE_LIMIT;
    let [x, y] = at.map(|v| i64::from(v).clamp(-limit, limit) as f32);
    Point::new(x, y)
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}

fn grow(r: Rect, by: f32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.width + 2.0 * by, r.height + 2.0 * by)
}

/// The box around every point of a path, control points included, which
/// holds the curves they shape.
fn path_box(segments: &[Segment]) -> Rect {
    let points = segments.iter().flat_map(|segment| match *segment {
        Segment::MoveTo(p) | Segment::LineTo(p) => vec![p],
        Segment::QuadTo { control, to } => vec![control, to],
        Segment::CubicTo { first, second, to } => vec![first, second, to],
        Segment::Close => vec![],
    });
    points
        .map(|p| Rect::new(p.x, p.y, 0.0, 0.0))
        .reduce(union)
        .unwrap_or_default()
}

/// The label drawn: at most [`MAX_LABEL_BYTES`], cut at a character.
fn drawn_text(text: &str) -> &str {
    if text.len() <= MAX_LABEL_BYTES {
        return text;
    }
    let mut end = MAX_LABEL_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn dash(stroke: &Named<Dash>) -> Option<[f32; 2]> {
    (stroke.known() == Some(Dash::Dashed)).then_some(route::DASH)
}

/// How a label's lines are broken and placed in their box.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// A shape's label: wrapped at the box, each line centred across and
    /// the block centred down.
    Shape,
    /// `kind = "text"`: wrapped at the box, at its top left.
    Free,
    /// A placeholder's caption: broken at line feeds only, at the top left.
    Caption,
    /// A connector's label: broken at line feeds only, centred.
    Connector,
}

impl Layout {
    fn wraps(self) -> bool {
        matches!(self, Self::Shape | Self::Free)
    }

    fn centred(self) -> bool {
        matches!(self, Self::Shape | Self::Connector)
    }
}

struct Painter<'a> {
    palette: &'a Palette,
    measure: &'a dyn Measure,
    items: Vec<Item>,
    /// The top-left corner of each connector drawn, for its issue marker.
    corners: BTreeMap<Id, Point>,
}

impl Painter<'_> {
    fn path(
        &mut self,
        source: &Source,
        segments: Vec<Segment>,
        fill: Option<Rgba>,
        stroke: Option<Stroke>,
        bounds: Rect,
    ) {
        self.items.push(Item {
            source: source.clone(),
            bounds,
            draw: Draw::Path {
                segments,
                fill,
                stroke,
            },
        });
    }

    /// `text` laid out in `area` as `layout` says. Blank lines take their
    /// height and draw nothing.
    fn text(
        &mut self,
        source: &Source,
        text: &str,
        area: Rect,
        layout: Layout,
        size: f32,
        color: Rgba,
    ) {
        let text = drawn_text(text);
        if text.trim().is_empty() {
            return;
        }
        let width = layout.wraps().then_some(area.width);
        let wrapped = wrap::wrap(text, width, Face::Regular, size, self.measure);
        let height = line_height(size);
        let top = if layout.centred() {
            area.y + (area.height - wrapped.len() as f32 * height) / 2.0
        } else {
            area.y
        };
        let lines: Vec<Line> = wrapped
            .into_iter()
            .enumerate()
            .filter(|(_, (line, _))| !line.is_empty())
            .map(|(row, (text, width))| {
                let x = if layout.centred() {
                    area.x + (area.width - width) / 2.0
                } else {
                    area.x
                };
                Line {
                    text,
                    origin: Point::new(x, top + row as f32 * height),
                    width,
                }
            })
            .collect();
        let Some(bounds) = lines
            .iter()
            .map(|line| Rect::new(line.origin.x, line.origin.y, line.width, height))
            .reduce(union)
        else {
            return;
        };
        self.items.push(Item {
            source: source.clone(),
            bounds,
            draw: Draw::Text {
                lines,
                face: Face::Regular,
                size,
                color,
            },
        });
    }

    fn node(&mut self, id: &Id, node: &Node, bounds: Rect) {
        let source = Source::Node(id.clone());
        let outlined = grow(bounds, STROKE_WIDTH / 2.0);
        match node.kind.as_ref().and_then(Named::known) {
            Some(NodeKind::Shape) => {
                let shape = shape::drawn(&node.shape);
                let fill = node.style.fill.known().unwrap_or(Fill::Tint);
                let Resolved {
                    stroke,
                    fill,
                    label,
                } = style::resolve(&node.style.color, fill, self.palette);
                let stroke = Stroke {
                    color: stroke,
                    width: STROKE_WIDTH,
                    dash: dash(&node.style.stroke),
                };
                self.path(
                    &source,
                    shape::outline(shape, bounds),
                    fill,
                    Some(stroke),
                    outlined,
                );
                let detail = shape::detail(shape, bounds);
                if !detail.is_empty() {
                    self.path(&source, detail, None, Some(stroke), outlined);
                }
                let area = shape::label_box(shape, bounds);
                self.text(&source, &node.text, area, Layout::Shape, LABEL_SIZE, label);
            }
            Some(NodeKind::Text) => {
                let color = style::resolve(&node.style.color, Fill::None, self.palette).stroke;
                self.text(&source, &node.text, bounds, Layout::Free, LABEL_SIZE, color);
            }
            None => {
                // A kind this build does not know, or none: a placeholder.
                let stroke = Stroke {
                    color: self.palette.muted,
                    width: STROKE_WIDTH,
                    dash: Some(route::DASH),
                };
                self.path(
                    &source,
                    shape::outline(Shape::Rect, bounds),
                    None,
                    Some(stroke),
                    outlined,
                );
                if let Some(Named::Other(kind)) = &node.kind {
                    let inset = CAPTION_INSET;
                    let corner = Rect::new(bounds.x + inset, bounds.y + inset, 0.0, 0.0);
                    self.text(
                        &source,
                        kind,
                        corner,
                        Layout::Caption,
                        SMALL_SIZE,
                        self.palette.muted,
                    );
                }
                let area = shape::label_box(Shape::Rect, bounds);
                self.text(
                    &source,
                    &node.text,
                    area,
                    Layout::Shape,
                    LABEL_SIZE,
                    self.palette.ink,
                );
            }
        }
    }

    /// A connector: its route when both ends are drawn, else a stub from
    /// the end that is.
    fn edge(
        &mut self,
        id: &Id,
        edge: &Edge,
        places: &HashMap<&str, Place<'_>>,
        routed: Option<&Routed>,
    ) {
        let source = Source::Edge(id.clone());
        let terminal = |end: &End| terminal(end, places).map(|(t, _)| t);
        let color = style::resolve(&edge.style.color, Fill::None, self.palette).stroke;
        let stroke = Stroke {
            color,
            width: STROKE_WIDTH,
            dash: dash(&edge.style.stroke),
        };
        let Some(routed) = routed else {
            match (terminal(&edge.from), terminal(&edge.to)) {
                (Some(valid), None) => self.stub(id, &valid, Side::Right),
                (None, Some(valid)) => self.stub(id, &valid, Side::Left),
                _ => {}
            }
            return;
        };
        let heads = edge.heads.each_ref().map(route::has_arrow);
        let drawn = routed.draw(heads);
        let line_box = grow(path_box(&drawn.line), STROKE_WIDTH / 2.0);
        self.corners
            .insert(id.clone(), Point::new(line_box.x, line_box.y));
        self.path(&source, drawn.line, None, Some(stroke), line_box);
        let arrows: Vec<Segment> = drawn.heads.into_iter().flatten().collect();
        if !arrows.is_empty() {
            let arrows_box = path_box(&arrows);
            self.path(&source, arrows, Some(color), None, arrows_box);
        }
        for label in edge.labels.iter().take(MAX_LABELS) {
            self.edge_label(&source, &label.text, routed.point_at(label.at), color);
        }
    }

    /// A connector label: its lines centred on `center`, on a box of the
    /// canvas colour.
    fn edge_label(&mut self, source: &Source, text: &str, center: Point, color: Rgba) {
        let text = drawn_text(text);
        if text.trim().is_empty() {
            return;
        }
        let lines = wrap::wrap(text, None, Face::Regular, SMALL_SIZE, self.measure);
        let width = lines.iter().map(|(_, w)| *w).fold(0.0, f32::max);
        let height = lines.len() as f32 * line_height(SMALL_SIZE);
        let area = Rect::new(
            center.x - width / 2.0,
            center.y - height / 2.0,
            width,
            height,
        );
        let [px, py] = EDGE_LABEL_PAD;
        let backing = Rect::new(
            area.x - px,
            area.y - py,
            area.width + 2.0 * px,
            area.height + 2.0 * py,
        );
        self.path(
            source,
            shape::outline(Shape::Rect, backing),
            Some(self.palette.canvas),
            None,
            backing,
        );
        self.text(source, text, area, Layout::Connector, SMALL_SIZE, color);
    }

    /// A stub out of the end that names a node, ending in a cross stroke,
    /// in the issue colour.
    fn stub(&mut self, id: &Id, valid: &Terminal, nominal: Side) {
        let source = Source::Edge(id.clone());
        let (start, side) = route::leave(valid, nominal);
        let (dx, dy) = match side {
            Side::Top => (0.0, -1.0),
            Side::Right => (1.0, 0.0),
            Side::Bottom => (0.0, 1.0),
            Side::Left => (-1.0, 0.0),
        };
        let tip = Point::new(start.x + dx * STUB_LENGTH, start.y + dy * STUB_LENGTH);
        let half = STUB_CROSS / 2.0;
        let segments = vec![
            Segment::MoveTo(start),
            Segment::LineTo(tip),
            Segment::MoveTo(Point::new(tip.x - dy * half, tip.y + dx * half)),
            Segment::LineTo(Point::new(tip.x + dy * half, tip.y - dx * half)),
        ];
        let bounds = grow(path_box(&segments), STROKE_WIDTH / 2.0);
        self.corners
            .insert(id.clone(), Point::new(bounds.x, bounds.y));
        let stroke = Stroke {
            color: self.palette.issue,
            width: STROKE_WIDTH,
            dash: None,
        };
        self.path(&source, segments, None, Some(stroke), bounds);
    }

    /// The dot marking a flagged element, centred on its top-left corner.
    fn marker(&mut self, source: Source, corner: Point) {
        let half = MARKER_SIZE / 2.0;
        let bounds = Rect::new(corner.x - half, corner.y - half, MARKER_SIZE, MARKER_SIZE);
        let dot = shape::outline(Shape::Ellipse, bounds);
        self.path(&source, dot, Some(self.palette.issue), None, bounds);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::fixtures::{self, Rng};
    use crate::style::tests::{DARK, LIGHT};
    use crate::{parse, Vocabulary};

    #[track_caller]
    fn read(text: &str) -> Board {
        parse(text).unwrap_or_else(|error| panic!("{error}\n{text}"))
    }

    #[track_caller]
    fn scene(text: &str) -> Scene {
        build(&read(text), &LIGHT, &FixedMeasure)
    }

    fn hex(c: Rgba) -> String {
        if c.a == 255 {
            format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
        }
    }

    fn rect(r: Rect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.width, r.height)
    }

    fn point(p: Point) -> String {
        format!("{},{}", p.x, p.y)
    }

    fn source(source: &Source) -> String {
        match source {
            Source::Node(id) => format!("node {id}"),
            Source::Edge(id) => format!("edge {id}"),
        }
    }

    /// A scene as text, one primitive per line: a path with its paint and
    /// segments, a text item with its paint, then each of its lines.
    pub(crate) fn dump(scene: &Scene) -> String {
        let mut out = format!("bounds {}\n", rect(scene.bounds));
        for item in &scene.items {
            let who = source(&item.source);
            match &item.draw {
                Draw::Path {
                    segments,
                    fill,
                    stroke,
                } => {
                    let _ = write!(out, "{who} path in {}", rect(item.bounds));
                    if let Some(fill) = fill {
                        let _ = write!(out, " fill {}", hex(*fill));
                    }
                    if let Some(stroke) = stroke {
                        let _ = write!(out, " stroke {} {}", hex(stroke.color), stroke.width);
                        if let Some([on, off]) = stroke.dash {
                            let _ = write!(out, " dash {on},{off}");
                        }
                    }
                    out.push(':');
                    for segment in segments {
                        let _ = match *segment {
                            Segment::MoveTo(p) => write!(out, " M {}", point(p)),
                            Segment::LineTo(p) => write!(out, " L {}", point(p)),
                            Segment::QuadTo { control, to } => {
                                write!(out, " Q {} {}", point(control), point(to))
                            }
                            Segment::CubicTo { first, second, to } => {
                                write!(out, " C {} {} {}", point(first), point(second), point(to))
                            }
                            Segment::Close => write!(out, " Z"),
                        };
                    }
                    out.push('\n');
                }
                Draw::Text {
                    lines,
                    face,
                    size,
                    color,
                } => {
                    let _ = writeln!(
                        out,
                        "{who} text in {} {face:?} {size} {}",
                        rect(item.bounds),
                        hex(*color)
                    );
                    for line in lines {
                        let _ = writeln!(
                            out,
                            "  {} w {} {:?}",
                            point(line.origin),
                            line.width,
                            line.text
                        );
                    }
                }
            }
        }
        out
    }

    macro_rules! scene_golden {
        ($name:literal, $board:expr) => {
            (
                $name,
                $board,
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/golden/scene/",
                    $name,
                    ".scene"
                )),
            )
        };
    }

    fn fixture(name: &str) -> &'static str {
        fixtures::all()
            .find(|(fixture, _)| *fixture == name)
            .map(|(_, text)| text)
            .unwrap_or_else(|| panic!("no fixture {name}"))
    }

    /// The scene dumps of `golden/scene/`, under the fixed measurer and the
    /// test palette: the one check that the canvas and the SVG writer are
    /// handed the same wrapped lines.
    pub(crate) fn golden_scenes() -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            scene_golden!("checkout-flow", fixture("Checkout flow")),
            scene_golden!("shapes", fixture("Shapes")),
            scene_golden!("overlap", fixture("Overlap")),
            scene_golden!("future", fixture("Future")),
            scene_golden!("hand-written", fixture("Hand written")),
            scene_golden!("password-reset", fixture("password-reset.out")),
            scene_golden!("shapes-and-ends", fixture("shapes-and-ends.out")),
            scene_golden!("fields", fixture("fields.out")),
            scene_golden!("unknown", fixture("unknown.out")),
            scene_golden!("escapes", fixture("escapes.out")),
        ]
    }

    #[test]
    fn scene_dumps_match_their_golden_files() {
        for (name, board, expected) in golden_scenes() {
            let actual = dump(&scene(board));
            fixtures::assert_golden_file(&format!("scene/{name}.scene"), expected, &actual);
        }
    }

    #[test]
    fn fixed_measure_counts_graphemes() {
        let m = FixedMeasure;
        assert_eq!(m.width("", Face::Regular, 14.0), 0.0);
        assert_eq!(m.width("Start", Face::Regular, 14.0), 35.0);
        // A decomposed é, a flag and a family emoji are one grapheme each.
        assert_eq!(m.width("e\u{301}🇳🇿👨‍👩‍👧", Face::Regular, 12.0), 18.0);
        assert_eq!(m.line_height(Face::Regular, 14.0), 17.5);
    }

    #[test]
    fn line_heights_are_the_planned_ones() {
        assert_eq!(line_height(LABEL_SIZE), 20.0);
        assert_eq!(line_height(SMALL_SIZE), 16.0);
        assert_eq!(line_height(28.0), 40.0);
    }

    fn node(id: &str, fields: &str) -> String {
        format!("\n[node.{id}]\nkind = \"shape\"\nsize = [100, 50]\n{fields}\n")
    }

    /// The node's box as drawn: the extent of its outline's points, which
    /// for every shape but the document is exactly its box.
    #[track_caller]
    fn drawn_box(scene: &Scene, id: &str) -> Rect {
        let Draw::Path { segments, .. } = &outline(scene, id).draw else {
            unreachable!();
        };
        path_box(segments)
    }

    #[track_caller]
    fn outline<'a>(scene: &'a Scene, id: &str) -> &'a Item {
        scene
            .items
            .iter()
            .find(|item| {
                item.source == Source::Node(Id::new(id).unwrap())
                    && matches!(item.draw, Draw::Path { .. })
            })
            .unwrap_or_else(|| panic!("no outline for {id}"))
    }

    /// Each element's id in the order first drawn, `n:` or `e:` before it.
    fn order(scene: &Scene) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for item in &scene.items {
            let name = match &item.source {
                Source::Node(id) => format!("n:{id}"),
                Source::Edge(id) => format!("e:{id}"),
            };
            if seen.last() != Some(&name) {
                seen.push(name);
            }
        }
        seen
    }

    /// The parent transform: through nested parents and an unknown kind;
    /// a missing parent, a self-parent and both cycles leave each node at
    /// its stored corner; a node hanging from a cycle is placed inside it.
    #[test]
    fn the_parent_transform_table() {
        let mut text = String::from("schema = 1\n");
        let rows: &[(&str, &str, [f32; 2])] = &[
            ("a", "at = [100, 100]", [100.0, 100.0]),
            ("b", "at = [10, 20]\nparent = \"a\"", [110.0, 120.0]),
            ("c", "at = [5, -5]\nparent = \"b\"", [115.0, 115.0]),
            ("v", "at = [10, 10]\nparent = \"u\"", [310.0, 10.0]),
            ("m", "at = [7, 8]\nparent = \"gone\"", [7.0, 8.0]),
            ("s", "at = [9, 9]\nparent = \"s\"", [9.0, 9.0]),
            ("p", "at = [40, 0]\nparent = \"q\"", [40.0, 0.0]),
            ("q", "at = [0, 40]\nparent = \"p\"", [0.0, 40.0]),
            ("x", "at = [1, 1]\nparent = \"y\"", [1.0, 1.0]),
            ("y", "at = [2, 2]\nparent = \"z\"", [2.0, 2.0]),
            ("z", "at = [3, 3]\nparent = \"x\"", [3.0, 3.0]),
            ("r", "at = [1, 2]\nparent = \"p\"", [41.0, 2.0]),
        ];
        for (id, fields, _) in rows {
            text += &node(id, fields);
        }
        text += "\n[node.u]\nkind = \"frame\"\nat = [300, 0]\nsize = [200, 200]\n";
        let drawn = scene(&text);
        for &(id, _, [x, y]) in rows {
            let got = drawn_box(&drawn, id);
            assert_eq!((got.x, got.y), (x, y), "{id}");
            assert_eq!((got.width, got.height), (100.0, 50.0), "{id}");
        }
        assert_eq!(drawn_box(&drawn, "u"), Rect::new(300.0, 0.0, 200.0, 200.0));
    }

    /// A chain of 33: the first 32 levels nest, the 33rd is a root at its
    /// stored corner.
    #[test]
    fn a_chain_deeper_than_32_draws_its_tail_as_a_root() {
        let mut text = String::from("schema = 1\n");
        text += &node("k01", "at = [1, 1]");
        for level in 2..=33 {
            let fields = format!("at = [1, 1]\nparent = \"k{:02}\"", level - 1);
            text += &node(&format!("k{level:02}"), &fields);
        }
        let drawn = scene(&text);
        assert_eq!(drawn_box(&drawn, "k32").x, 32.0);
        assert_eq!(drawn_box(&drawn, "k33").x, 1.0);
    }

    /// Siblings by `z` bytewise (missing and empty as empty, long keys by
    /// their first 64 bytes), then id, then a node before a connector;
    /// roots and connectors share one scope; children come directly after
    /// their parent, in their own order.
    #[test]
    fn the_stacking_table() {
        let long = "a".repeat(MAX_Z);
        let mut text = String::from("schema = 1\n");
        for (id, fields) in [
            ("a", "z = \"b\"".to_owned()),
            ("b", "z = \"a\"".to_owned()),
            ("c", "z = \"Z\"\nparent = \"a\"".to_owned()),
            ("d", "z = \"0\"\nparent = \"a\"".to_owned()),
            ("cc", "z = \"0\"\nparent = \"c\"".to_owned()),
            ("e", "z = \"b\"".to_owned()),
            ("g", "z = \"\"".to_owned()),
            ("h", format!("z = \"{long}Z\"")),
            ("i", format!("z = \"{long}0\"")),
            ("j", "z = \"B\"".to_owned()),
            ("k", String::new()),
        ] {
            text += &node(id, &format!("at = [0, 0]\n{fields}"));
        }
        for (id, z) in [("e", "z = \"b\""), ("f", ""), ("a1", "z = \"9\"")] {
            text += &format!(
                "\n[edge.{id}]\nkind = \"edge\"\nfrom = {{ node = \"a\" }}\nto = {{ node = \"b\" }}\n{z}\n"
            );
        }
        let want = [
            "e:f", "n:g", "n:k", "e:a1", "n:j", "n:b", "n:h", "n:i", "n:a", "n:d", "n:c", "n:cc",
            "n:e", "e:e",
        ];
        let got = order(&scene(&text));
        assert_eq!(got[..want.len()], want);
    }

    /// Shape, box and label; the lines drawn, each at its top-left corner.
    #[test]
    fn the_wrap_and_overflow_table() {
        type Row = (
            &'static str,
            [i32; 2],
            &'static str,
            &'static [(&'static str, f32, f32)],
        );
        #[rustfmt::skip]
        let rows: &[Row] = &[
            ("rect", [120, 60], "Start", &[("Start", 42.5, 20.0)]),
            ("rect", [120, 60], "Send reset link", &[("Send reset", 25.0, 10.0), ("link", 46.0, 30.0)]),
            ("rect", [120, 60], "a\nb\nc\nd", &[("a", 56.5, -10.0), ("b", 56.5, 10.0), ("c", 56.5, 30.0), ("d", 56.5, 50.0)]),
            ("rect", [120, 60], "a\n\nc", &[("a", 56.5, 0.0), ("c", 56.5, 40.0)]),
            ("rect", [40, 60], "abcdefghij", &[("abc", 9.5, -10.0), ("def", 9.5, 10.0), ("ghi", 9.5, 30.0), ("j", 16.5, 50.0)]),
            ("rect", [400, 60], "  padded  ", &[("  padded", 172.0, 20.0)]),
            ("ellipse", [200, 100], "Hello", &[("Hello", 82.5, 40.0)]),
            ("diamond", [120, 60], "Notify customer", &[("Notify", 39.0, 10.0), ("customer", 32.0, 30.0)]),
            ("pill", [120, 40], "Pill", &[("Pill", 46.0, 10.0)]),
            ("cylinder", [120, 80], "Data", &[("Data", 46.0, 40.0)]),
            ("document", [120, 80], "Doc", &[("Doc", 49.5, 24.0)]),
            ("parallelogram", [160, 60], "Input", &[("Input", 62.5, 20.0)]),
        ];
        for &(shape, [w, h], label, want) in rows {
            let text = format!(
                "schema = 1\n[node.n]\nkind = \"shape\"\nshape = \"{shape}\"\nat = [0, 0]\nsize = [{w}, {h}]\ntext = {label:?}\n"
            );
            let drawn = scene(&text);
            let lines: Vec<(&str, f32, f32)> = drawn
                .items
                .iter()
                .filter_map(|item| match &item.draw {
                    Draw::Text { lines, .. } => Some(lines),
                    Draw::Path { .. } => None,
                })
                .flatten()
                .map(|line| (line.text.as_str(), line.origin.x, line.origin.y))
                .collect();
            assert_eq!(lines, want, "{shape} {w}x{h} {label:?}");
            // Overflow is drawn in full and never changes the node's box.
            assert_eq!(
                outline(&drawn, "n").bounds,
                grow(Rect::new(0.0, 0.0, w as f32, h as f32), STROKE_WIDTH / 2.0)
            );
        }
    }

    #[test]
    fn free_text_has_no_outline_and_sits_top_left_in_its_colour() {
        let text = "schema = 1\n[node.t]\nkind = \"text\"\nat = [10, 20]\nsize = [70, 40]\n\
                    text = \"Free text here\"\nstyle.color = \"red\"\nstyle.fill = \"solid\"\nz = \"V\"\n";
        let drawn = scene(text);
        assert_eq!(drawn.items.len(), 1);
        let Draw::Text { lines, color, .. } = &drawn.items[0].draw else {
            panic!("{drawn:?}");
        };
        assert_eq!(*color, LIGHT.hues[0]);
        let got: Vec<_> = lines.iter().map(|l| (l.text.as_str(), l.origin)).collect();
        assert_eq!(
            got,
            [
                ("Free text", Point::new(10.0, 20.0)),
                ("here", Point::new(10.0, 40.0))
            ]
        );
    }

    #[test]
    fn unknown_kinds_draw_as_a_captioned_placeholder() {
        let text = "schema = 1\n[node.f]\nkind = \"frame\"\nshape = \"pill\"\nat = [0, 0]\n\
                    size = [200, 100]\ntext = \"Title\"\nstyle.color = \"red\"\nz = \"V\"\n\
                    [node.m]\nat = [300, 0]\nsize = [100, 50]\ntext = \"No kind\"\nz = \"W\"\n";
        let drawn = scene(text);
        let dump = dump(&drawn);
        let want = "\
node f path in -0.75,-0.75 201.5x101.5 stroke #808890 1.5 dash 6,4: M 0,0 L 200,0 L 200,100 L 0,100 Z
node f text in 4,4 30x16 Regular 12 #808890
  4,4 w 30 \"frame\"
node f text in 82.5,40 35x20 Regular 14 #202428
  82.5,40 w 35 \"Title\"
node m path in 299.25,-0.75 101.5x51.5 stroke #808890 1.5 dash 6,4: M 300,0 L 400,0 L 400,50 L 300,50 Z
node m text in 325.5,15 49x20 Regular 14 #202428
  325.5,15 w 49 \"No kind\"
node m path in 297,-3 6x6 fill #d02020:";
        assert!(
            dump.starts_with(&format!("bounds -0.75,-3 401.5x103.75\n{want}")),
            "{dump}"
        );
    }

    #[test]
    fn a_dangling_connector_is_a_stub_in_the_issue_colour() {
        let text = "schema = 1\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [100, 50]\nz = \"V\"\n\
                    [edge.out]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"gone\" }\nz = \"W\"\n\
                    [edge.in]\nkind = \"edge\"\nfrom = { node = \"gone\" }\nto = { node = \"a\", side = \"top\" }\nz = \"X\"\n\
                    [edge.none]\nkind = \"edge\"\nfrom = { node = \"gone\" }\nto = { node = \"lost\" }\nz = \"Y\"\n\
                    [edge.free]\nkind = \"edge\"\nfrom = { at = [500, 500] }\nto = { node = \"gone\" }\nz = \"Z\"\n";
        let dump = dump(&scene(text));
        let want = "\
edge out path in 99.25,20.25 25.5x9.5 stroke #d02020 1.5: M 100,25 L 124,25 M 124,29 L 124,21
edge in path in 45.25,-24.75 9.5x25.5 stroke #d02020 1.5: M 50,0 L 50,-24 M 54,-24 L 46,-24
edge free path in 499.25,495.25 25.5x9.5 stroke #d02020 1.5: M 500,500 L 524,500 M 524,504 L 524,496
edge free path in 496.25,492.25 6x6 fill #d02020:";
        assert!(dump.contains(want), "{dump}");
        for marker in [
            "edge in path in 42.25,-27.75 6x6",
            "edge out path in 96.25,17.25 6x6",
        ] {
            assert!(dump.contains(marker), "{marker}\n{dump}");
        }
        assert!(
            !dump.contains("edge none"),
            "both ends dangle: nothing is drawn\n{dump}"
        );
        assert_eq!(dump.matches("fill #d02020").count(), 3, "{dump}");
    }

    #[test]
    fn every_flagged_element_gets_one_marker_on_top() {
        let drawn = scene(fixture("fields.out"));
        let issues = check(&read(fixture("fields.out")));
        let markers: Vec<&Item> = drawn
            .items
            .iter()
            .rev()
            .take_while(|item| {
                matches!(item.draw, Draw::Path { fill: Some(c), stroke: None, .. } if c == LIGHT.issue)
                    && item.bounds.width == MARKER_SIZE
            })
            .collect();
        let mut flagged: Vec<&Source> = issues.iter().map(|i| &i.element).collect();
        flagged.dedup();
        assert_eq!(markers.len(), flagged.len(), "{issues:?}");
        assert!(!flagged.is_empty());
    }

    /// The Checkout flow sample's routes: no connector enters the box of a
    /// node it does not join, and no two run on one line for more than a
    /// few pixels except within a margin of an end they share.
    #[test]
    fn checkout_flow_routes_keep_out_of_shapes_and_off_each_other() {
        let board = read(fixture("Checkout flow"));
        let placed = placements(&board);
        let boxes: BTreeMap<&Id, Rect> = board
            .nodes
            .iter()
            .map(|(id, node)| (id, node_box(placed[id].at, node.size)))
            .collect();
        let places = places(&board, &boxes);
        let routes: BTreeMap<&Id, Routed> = board
            .edges
            .keys()
            .zip(routes(&board, &boxes, &places))
            .filter_map(|(id, routed)| Some((id, routed?)))
            .collect();
        assert_eq!(routes.len(), 17);
        let joined = |id: &Id| {
            let edge = &board.edges[id];
            [&edge.from, &edge.to].map(|end| match end {
                End::Node { node, .. } => node.clone(),
                End::Free { .. } => String::new(),
            })
        };
        for (&id, routed) in &routes {
            for (&node, &r) in &boxes {
                if joined(id).contains(&node.to_string()) {
                    continue;
                }
                for w in routed.points.windows(2) {
                    let s = path_box(&[Segment::MoveTo(w[0]), Segment::LineTo(w[1])]);
                    let inside =
                        s.x < r.right() && s.right() > r.x && s.y < r.bottom() && s.bottom() > r.y;
                    assert!(!inside, "{id} enters {node}: {:?}", routed.points);
                }
            }
        }
        let few = route::SHARED_LENGTH;
        for (&a, ra) in &routes {
            for (&b, rb) in &routes {
                if a >= b {
                    continue;
                }
                let ends = |r: &Routed| [r.points[0], *r.points.last().unwrap()];
                let common: Vec<Point> = ends(ra)
                    .into_iter()
                    .filter(|p| ends(rb).contains(p))
                    .collect();
                for s in ra.points.windows(2) {
                    for t in rb.points.windows(2) {
                        let shared = shared_away_from(s, t, &common);
                        assert!(
                            shared <= few,
                            "{a} and {b} share {shared} px: {:?} {:?}",
                            ra.points,
                            rb.points
                        );
                    }
                }
            }
        }
    }

    /// How long two axis-aligned segments run on one line, leaving out
    /// what lies within [`route::ELBOW_MARGIN`] of a point in `common`.
    fn shared_away_from(s: &[Point], t: &[Point], common: &[Point]) -> f32 {
        let (horizontal, line, sa, ta) = if s[0].y == s[1].y && t[0].y == t[1].y {
            (true, s[0].y, [s[0].x, s[1].x], [t[0].x, t[1].x])
        } else if s[0].x == s[1].x && t[0].x == t[1].x {
            (false, s[0].x, [s[0].y, s[1].y], [t[0].y, t[1].y])
        } else {
            return 0.0;
        };
        let other = if horizontal { t[0].y } else { t[0].x };
        if (line - other).abs() > route::LINE_TOLERANCE {
            return 0.0;
        }
        let lo = sa[0].min(sa[1]).max(ta[0].min(ta[1]));
        let hi = sa[0].max(sa[1]).min(ta[0].max(ta[1]));
        let mut length = (hi - lo).max(0.0);
        for p in common {
            let (on, along) = if horizontal { (p.y, p.x) } else { (p.x, p.y) };
            if (on - line).abs() <= route::LINE_TOLERANCE {
                let m = route::ELBOW_MARGIN;
                length -= (hi.min(along + m) - lo.max(along - m)).max(0.0);
            }
        }
        length.max(0.0)
    }

    #[test]
    fn connectors_route_label_and_colour() {
        let text = "schema = 1\n\
            [node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [100, 50]\nz = \"V\"\n\
            [node.b]\nkind = \"shape\"\nat = [300, 0]\nsize = [100, 50]\nz = \"V\"\n\
            [edge.e]\nkind = \"wire\"\nfrom = { node = \"a\" }\nto = { node = \"b\" }\n\
            heads = [\"arrow\", \"diamond\"]\nroute = \"curved\"\nstyle.color = \"blue\"\n\
            style.stroke = \"dashed\"\nlabels = [{ text = \"yes\" }, { text = \"\", at = 0 }, { text = \"two\\nlines\", at = 0 }]\nz = \"W\"\n";
        let dump = dump(&scene(text));
        let want = "\
edge e path in 109.25,24.25 181.5x1.5 stroke #3060d0 1.5 dash 6,4: M 110,25 L 290,25
edge e path in 100,21 200x8 fill #3060d0: M 100,25 L 110,21 L 110,29 Z M 300,25 L 290,29 L 290,21 Z
edge e path in 187,15 26x20 fill #ffffff: M 187,15 L 213,15 L 213,35 L 187,35 Z
edge e text in 191,17 18x16 Regular 12 #3060d0
  191,17 w 18 \"yes\"
edge e path in 81,7 38x36 fill #ffffff: M 81,7 L 119,7 L 119,43 L 81,43 Z
edge e text in 85,9 30x32 Regular 12 #3060d0
  91,9 w 18 \"two\"
  85,25 w 30 \"lines\"
";
        assert!(dump.ends_with(want), "{dump}");
    }

    #[test]
    fn only_sixteen_labels_and_sixteen_kib_of_a_label_are_drawn() {
        let labels: Vec<String> = (0..20).map(|i| format!("{{ text = \"L{i}\" }}")).collect();
        let long = "x".repeat(MAX_LABEL_BYTES + 100);
        let text = format!(
            "schema = 1\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [100, 50]\ntext = \"{long}\"\n\
             [edge.e]\nkind = \"edge\"\nfrom = {{ at = [0, 100] }}\nto = {{ at = [500, 100] }}\nlabels = [{}]\n",
            labels.join(", ")
        );
        let drawn = scene(&text);
        let texts = |id: &str, drawn: &Scene| -> Vec<String> {
            drawn
                .items
                .iter()
                .filter(|item| matches!(&item.source, Source::Edge(e) | Source::Node(e) if e.as_str() == id))
                .filter_map(|item| match &item.draw {
                    Draw::Text { lines, .. } => Some(lines.iter().map(|l| l.text.clone()).collect::<String>()),
                    Draw::Path { .. } => None,
                })
                .collect()
        };
        assert_eq!(texts("e", &drawn).len(), MAX_LABELS);
        assert_eq!(texts("a", &drawn).concat().len(), MAX_LABEL_BYTES);
        assert_eq!(
            drawn_text(&"é".repeat(MAX_LABEL_BYTES)).len(),
            MAX_LABEL_BYTES
        );
        assert_eq!(
            drawn_text(&format!("x{}", "é".repeat(MAX_LABEL_BYTES))).len(),
            MAX_LABEL_BYTES - 1
        );
    }

    /// Geometry past ±2^24, sizes below 1, anchors and label positions
    /// outside 0 to 1000 are drawn clamped; the model is untouched.
    #[test]
    fn out_of_range_values_are_drawn_clamped() {
        let text = "schema = 1\n\
            [node.far]\nkind = \"shape\"\nat = [2147483647, -2147483648]\nsize = [0, -5]\nz = \"V\"\n\
            [node.big]\nkind = \"shape\"\nat = [0, 0]\nsize = [2147483647, 50]\nz = \"V\"\n\
            [node.deep]\nkind = \"shape\"\nat = [2147483647, 0]\nparent = \"far\"\nsize = [10, 10]\nz = \"V\"\n\
            [node.a]\nkind = \"shape\"\nat = [0, 100]\nsize = [100, 100]\nz = \"V\"\n\
            [edge.e]\nkind = \"edge\"\nfrom = { node = \"a\", anchor = [2000, -5] }\nto = { at = [-2147483648, 100] }\n\
            heads = [\"none\", \"none\"]\nlabels = [{ text = \"start\", at = -5 }, { text = \"end\", at = 5000 }]\nz = \"W\"\n";
        let board = read(text);
        let before = board.clone();
        let drawn = build(&board, &LIGHT, &FixedMeasure);
        assert_eq!(board, before);
        let limit = COORDINATE_LIMIT as f32;
        // A box stays inside the limit, so one at its edge is 1 wide.
        let far = Rect::new(limit - 1.0, -limit, 1.0, 1.0);
        assert_eq!(drawn_box(&drawn, "far"), far);
        assert_eq!(drawn_box(&drawn, "big"), Rect::new(0.0, 0.0, limit, 50.0));
        assert_eq!(
            drawn_box(&drawn, "deep"),
            Rect {
                height: 10.0,
                ..far
            }
        );
        let dump = dump(&drawn);
        // The anchor is clamped to the top-right corner, the free end to
        // the limit, and the labels to the route's two ends.
        assert!(dump.contains(": M 100,100 L 100,80 L "), "{dump}");
        assert!(dump.contains(" L -16777216,100\n"), "{dump}");
        assert!(dump.contains("\n  85,92 w 30 \"start\"\n"), "{dump}");
        assert!(dump.contains("\n  -16777224,92 w 18 \"end\"\n"), "{dump}");
        for item in &drawn.items {
            assert_finite(item);
        }
    }

    #[track_caller]
    fn assert_finite(item: &Item) {
        let b = item.bounds;
        assert!(
            [b.x, b.y, b.width, b.height].iter().all(|v| v.is_finite())
                && b.width >= 0.0
                && b.height >= 0.0,
            "{item:?}"
        );
        let points: Vec<Point> = match &item.draw {
            Draw::Path { segments, .. } => segments
                .iter()
                .flat_map(|s| match *s {
                    Segment::MoveTo(p) | Segment::LineTo(p) => vec![p],
                    Segment::QuadTo { control, to } => vec![control, to],
                    Segment::CubicTo { first, second, to } => vec![first, second, to],
                    Segment::Close => vec![],
                })
                .collect(),
            Draw::Text { lines, .. } => {
                assert!(lines
                    .iter()
                    .all(|l| l.width.is_finite() && !l.text.is_empty()));
                lines.iter().map(|l| l.origin).collect()
            }
        };
        assert!(
            points.iter().all(|p| p.x.is_finite() && p.y.is_finite()),
            "{item:?}"
        );
    }

    #[test]
    fn the_scene_follows_the_palette() {
        let board = read(fixture("Shapes"));
        let light = build(&board, &LIGHT, &FixedMeasure);
        let dark = build(&board, &DARK, &FixedMeasure);
        assert_eq!(light.items.len(), dark.items.len());
        assert_ne!(light, dark);
        let bounds: Vec<Rect> = light.items.iter().map(|i| i.bounds).collect();
        let dark_bounds: Vec<Rect> = dark.items.iter().map(|i| i.bounds).collect();
        assert_eq!(
            bounds, dark_bounds,
            "a theme changes colours, never geometry"
        );
    }

    #[test]
    fn an_empty_board_has_an_empty_scene() {
        let drawn = scene(crate::EMPTY);
        assert_eq!(
            drawn,
            Scene {
                bounds: Rect::default(),
                items: Vec::new()
            }
        );
    }

    /// A board of up to 12 nodes and 10 connectors whose references mostly
    /// resolve, with every kind, shape, end form and extreme number.
    /// `text` as a TOML basic string.
    fn quoted(text: &str) -> String {
        let mut out = String::from("\"");
        for c in text.chars() {
            match c {
                '"' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                c if c.is_control() => {
                    let _ = write!(out, "\\u{:04X}", u32::from(c));
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    pub(crate) fn random_board(rng: &mut Rng) -> Board {
        fn int(rng: &mut Rng) -> i32 {
            match rng.below(5) {
                0 => *rng.pick(&[0, 1, -1, 1000, i32::MAX, i32::MIN, 16_777_217]),
                1 => rng.next() as i32,
                _ => rng.below(1200) as i32 - 100,
            }
        }
        fn text(rng: &mut Rng) -> String {
            let pieces = [
                "",
                "a",
                "Start",
                " ",
                "\n",
                "注文",
                "👨‍👩‍👧",
                "e\u{301}",
                "\t",
                "\u{0}",
                "<&>",
                "long-unbroken-word",
            ];
            (0..rng.below(6)).map(|_| *rng.pick(&pieces)).collect()
        }
        let ids: Vec<String> = (0..rng.below(12)).map(|i| format!("n{i}")).collect();
        let name = |rng: &mut Rng| -> String {
            if ids.is_empty() || rng.one_in(6) {
                "gone".into()
            } else {
                rng.pick(&ids).clone()
            }
        };
        let mut out = String::from("schema = 1\n");
        for id in &ids {
            let kind = *rng.pick(&["shape", "shape", "shape", "text", "frame"]);
            let shape = rng.pick(Shape::ALL).name();
            let shape = if rng.one_in(8) { "hexagon" } else { shape };
            let _ = write!(
                out,
                "\n[node.{id}]\ntext = {}\nat = [{}, {}]\nshape = \"{shape}\"\nsize = [{}, {}]\nz = \"{}\"\n",
                quoted(&text(rng)),
                int(rng),
                int(rng),
                int(rng),
                int(rng),
                rng.below(100)
            );
            if !rng.one_in(10) {
                let _ = writeln!(out, "kind = \"{kind}\"");
            }
            if rng.one_in(3) {
                let _ = writeln!(out, "parent = \"{}\"", name(rng));
            }
            let _ = writeln!(
                out,
                "style.color = \"{}\"\nstyle.fill = \"{}\"\nstyle.stroke = \"{}\"",
                rng.pick(&["neutral", "red", "blue", "#a0b0c0", "mauve"]),
                rng.pick(&["none", "tint", "solid", "x"]),
                rng.pick(&["solid", "dashed", "dotted"])
            );
        }
        for e in 0..rng.below(10) {
            let end = |rng: &mut Rng| -> String {
                match rng.below(5) {
                    0 => format!("{{ at = [{}, {}] }}", int(rng), int(rng)),
                    1 => format!(
                        "{{ node = \"{}\", side = \"{}\" }}",
                        name(rng),
                        rng.pick(&["top", "right", "bottom", "left", "up"])
                    ),
                    2 => format!(
                        "{{ node = \"{}\", anchor = [{}, {}] }}",
                        name(rng),
                        int(rng),
                        int(rng)
                    ),
                    _ => format!("{{ node = \"{}\" }}", name(rng)),
                }
            };
            let (from, to) = (end(rng), end(rng));
            let _ = write!(
                out,
                "\n[edge.e{e}]\nkind = \"edge\"\nfrom = {from}\nto = {to}\nroute = \"{}\"\nheads = [\"{}\", \"{}\"]\nlabels = [{{ text = {}, at = {} }}]\nz = \"{}\"\n",
                rng.pick(&["elbow", "straight", "curved"]),
                rng.pick(&["none", "arrow"]),
                rng.pick(&["none", "arrow", "odd"]),
                quoted(&text(rng)),
                int(rng),
                rng.below(100)
            );
        }
        read(&out)
    }

    /// Whatever a board holds, the scene is finite, deterministic, every
    /// item lies inside the scene's bounds, and every element with a box
    /// is drawn.
    fn scenes_hold(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        for _ in 0..count {
            let board = random_board(&mut rng);
            let drawn = build(&board, &LIGHT, &FixedMeasure);
            assert_eq!(drawn, build(&board, &LIGHT, &FixedMeasure));
            for item in &drawn.items {
                assert_finite(item);
                // Inside, up to the rounding of f32 sums far from 0.
                let (b, s) = (item.bounds, drawn.bounds);
                let slack = |start: f32, span: f32| (start.abs() + span) * f32::EPSILON * 4.0;
                assert!(b.x >= s.x && b.y >= s.y, "{item:?} {s:?}");
                let right = s.right() + slack(s.x, s.width);
                let bottom = s.bottom() + slack(s.y, s.height);
                assert!(b.right() <= right && b.bottom() <= bottom, "{item:?} {s:?}");
            }
            for (id, node) in &board.nodes {
                let shaped =
                    node.kind.as_ref().and_then(Named::known) != Some(crate::NodeKind::Text);
                if shaped {
                    drawn_box(&drawn, id.as_str());
                }
            }
        }
    }

    #[test]
    fn random_boards_draw() {
        scenes_hold(300, 7);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_random_boards_draw() {
        scenes_hold(20_000, 8);
    }
}
