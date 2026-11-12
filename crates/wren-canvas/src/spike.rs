//! The Phase 1 spike: generated scenes, the state a host keeps for the
//! bare pan-and-zoom widget, and a prototype of in-canvas label editing.
//!
//! It exists to answer how a scene is best painted (one renderer
//! transformation over cached geometry, or geometry rebuilt at each zoom)
//! and whether a label can be edited by the canvas itself under one focus
//! id. The viewer replaces it; nothing here reads or writes a board file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iced::advanced::text::{self, Paragraph as _};
use iced::{Element, Font, Pixels, Size};
use wren_board::scene::{
    Draw, Face, Item, Line, Measure, Point, Rect, Scene, Segment, Source, Stroke,
};
use wren_board::{Id, Palette, Rgba};

use crate::layers::{self, Plan};
use crate::Viewport;

/// How long after the last pan or zoom step the view counts as at rest.
pub const SETTLE: Duration = Duration::from_millis(150);
/// Wheel events closer together than this are one burst and keep the
/// pan-or-zoom choice of its first event.
pub const BURST: Duration = Duration::from_millis(150);
/// Logical pixels panned per wheel line.
pub const LINE: f32 = 40.0;
/// The zoom factor per wheel line.
pub const ZOOM_STEP: f32 = 1.1;

const LABEL_SIZE: f32 = 14.0;
const NODE: (f32, f32) = (160.0, 64.0);
const PITCH: (f32, f32) = (220.0, 120.0);
const STROKE: f32 = 1.5;

/// How the widget turns the scene into geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Geometry built once in board units; the renderer moves and scales it.
    Transform,
    /// Geometry rebuilt for the visible items at each viewport, positions
    /// and sizes already in screen pixels.
    Rebuild,
    /// [`Strategy::Transform`] while the view moves, [`Strategy::Rebuild`]
    /// once it has been at rest for [`SETTLE`].
    Settle,
}

/// Which labels the rebuilt geometry holds. Glyphs are what a software
/// renderer spends its frame on once much of a board is in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Labels {
    /// Labels whose text is smaller than this on screen are not drawn.
    pub min_px: f32,
    /// While the view moves, labels are drawn only if at most this many
    /// of their lines are in view.
    pub moving_lines: usize,
}

impl Labels {
    pub const ALWAYS: Self = Self {
        min_px: 0.0,
        moving_lines: usize::MAX,
    };

    /// What the spike measured the software renderer to need: nothing
    /// under 4 px, and nothing while the view moves.
    pub const SOFTWARE: Self = Self {
        min_px: 4.0,
        moving_lines: 0,
    };
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Wheel {
        delta: Scroll,
        /// The pointer, widget-local.
        at: (f32, f32),
        /// The zoom modifier was held.
        zoom: bool,
        /// Shift was held.
        sideways: bool,
    },
    /// A middle-button drag step, in screen pixels.
    Drag {
        dx: f32,
        dy: f32,
    },
    /// An arrow key on the focused canvas: captured, and it does nothing.
    Arrow,
    Edit(Edit),
}

/// A wheel delta as the toolkit reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    Lines { x: f32, y: f32 },
    Pixels { x: f32, y: f32 },
}

/// One step of label editing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Open the editor on the node nearest the view's top-left corner.
    Open,
    Insert(String),
    Backspace,
    Left,
    Right,
    /// The input method's composition; empty content ends it.
    Preedit {
        content: String,
        selection: Option<Range<usize>>,
    },
    /// Text the input method committed.
    Commit(String),
    /// Leave the editor, keeping the text.
    Close,
}

/// An open label editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub node: usize,
    pub text: String,
    /// A byte offset into `text`, on a character boundary.
    pub caret: usize,
    pub preedit: Option<Preedit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preedit {
    pub content: String,
    pub selection: Option<Range<usize>>,
}

/// A place where a later shape covers part of an earlier label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlap {
    /// Board units: inside the covering shape, over the covered label's
    /// lines, and clear of the covering shape's own label.
    pub region: Rect,
    pub fill: Rgba,
}

/// What the host keeps between frames for one spike board.
pub struct View {
    scene: Scene,
    nodes: Vec<Node>,
    overlaps: Vec<Overlap>,
    plan: Plan,
    cap: usize,
    forced: Option<usize>,
    palette: Palette,
    font: Font,
    measure: Box<dyn Measure>,
    viewport: Viewport,
    strategy: Strategy,
    labels: Option<Labels>,
    revision: u64,
    moved_at: Option<Instant>,
    burst: Option<(Instant, bool)>,
    scale_factor: f32,
    label: Option<Label>,
    #[cfg(feature = "test-support")]
    pub probe: Probe,
}

/// A revision no view has had.
fn revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The scene items of one generated node.
#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Rect,
    text: usize,
}

impl View {
    /// `count` labelled rounded rectangles in a grid, each connected to its
    /// right and lower neighbour; every tenth label has CJK text and an
    /// emoji, and one shape in ten sits over its left neighbour's label.
    pub fn grid(count: usize, palette: Palette, font: Font) -> Self {
        let measure = Measurer::new(font);
        let (scene, nodes, overlaps) = grid(count, &palette, &measure);
        Self::with(scene, nodes, overlaps, palette, font, Box::new(measure))
    }

    /// One line of text per label, each written between two bars so the
    /// ink shows its advance, under a first row of the two bars alone.
    pub fn rows(labels: &[String], palette: Palette, font: Font) -> Self {
        let measure = Measurer::new(font);
        let scene = rows(labels, &palette, &measure);
        Self::with(
            scene,
            Vec::new(),
            Vec::new(),
            palette,
            font,
            Box::new(measure),
        )
    }

    fn with(
        scene: Scene,
        nodes: Vec<Node>,
        overlaps: Vec<Overlap>,
        palette: Palette,
        font: Font,
        measure: Box<dyn Measure>,
    ) -> Self {
        let plan = layers::split(&scene.items, layers::CAP);
        Self {
            scene,
            nodes,
            overlaps,
            plan,
            cap: layers::CAP,
            forced: None,
            palette,
            font,
            measure,
            viewport: Viewport::default(),
            strategy: Strategy::Rebuild,
            labels: None,
            revision: revision(),
            moved_at: None,
            burst: None,
            scale_factor: 1.0,
            label: None,
            #[cfg(feature = "test-support")]
            probe: Probe::default(),
        }
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn overlaps(&self) -> &[Overlap] {
        &self.overlaps
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    /// The label rule, or `None` to take the renderer's.
    pub fn labels(&self) -> Option<Labels> {
        self.labels
    }

    pub fn set_labels(&mut self, labels: Option<Labels>) {
        self.labels = labels;
        self.revision = revision();
    }

    pub fn label(&self) -> Option<&Label> {
        self.label.as_ref()
    }

    /// Changes whenever the scene or its layers do, so the widget knows
    /// its geometry is stale. No two views share one: the widget keeps its
    /// geometry when the host swaps the view under it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn font(&self) -> Font {
        self.font
    }

    pub fn line_height(&self, face: Face, size: f32) -> f32 {
        self.measure.line_height(face, size)
    }

    /// Whether the view moved less than [`SETTLE`] before `now`.
    pub fn moving(&self, now: Instant) -> bool {
        self.moved_at
            .is_some_and(|moved| now.saturating_duration_since(moved) < SETTLE)
    }

    /// When a moving view comes to rest.
    pub fn settles_at(&self) -> Option<Instant> {
        self.moved_at.map(|moved| moved + SETTLE)
    }

    pub fn set_viewport(&mut self, viewport: Viewport, now: Instant) {
        if viewport != self.viewport {
            self.viewport = viewport;
            self.moved_at = Some(now);
        }
    }

    pub fn set_strategy(&mut self, strategy: Strategy) {
        self.strategy = strategy;
        self.revision = revision();
    }

    /// The window's scale factor, which `Pixels` wheel deltas are in.
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor.max(0.1);
    }

    /// Paint in at most `cap` layers.
    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap;
        self.replan();
    }

    /// Paint in exactly `layers` equal runs of items, whatever overlaps:
    /// for measuring what a layer costs.
    pub fn force_layers(&mut self, layers: Option<usize>) {
        self.forced = layers;
        self.replan();
    }

    fn replan(&mut self) {
        self.plan = match self.forced {
            Some(count) => {
                let count = count.clamp(1, 255);
                let total = self.scene.items.len().max(1);
                Plan {
                    of: (0..self.scene.items.len())
                        .map(|index| u8::try_from(index * count / total).unwrap_or(u8::MAX))
                        .collect(),
                    count: count.min(self.scene.items.len()),
                    overflow: 0,
                }
            }
            None => layers::split(&self.scene.items, self.cap),
        };
        self.revision = revision();
    }

    /// Apply a widget message. Returns whether anything changed.
    pub fn update(&mut self, message: Message, now: Instant) -> bool {
        match message {
            Message::Wheel {
                delta,
                at,
                zoom,
                sideways,
            } => {
                let zoom = match self.burst {
                    Some((last, choice)) if now.saturating_duration_since(last) < BURST => choice,
                    _ => zoom,
                };
                self.burst = Some((now, zoom));
                let (x, y, lines) = match delta {
                    Scroll::Lines { x, y } if sideways && x == 0.0 => (y, 0.0, y),
                    Scroll::Lines { x, y } => (x, y, y),
                    Scroll::Pixels { x, y } => {
                        let unit = self.scale_factor * LINE;
                        (x / unit, y / unit, y / unit)
                    }
                };
                let viewport = if zoom {
                    self.viewport.zoomed(ZOOM_STEP.powf(lines), at)
                } else {
                    self.viewport.panned(-x * LINE, -y * LINE)
                };
                self.set_viewport(viewport, now);
            }
            Message::Drag { dx, dy } => {
                self.set_viewport(self.viewport.panned(-dx, -dy), now);
            }
            Message::Arrow => {
                #[cfg(feature = "test-support")]
                self.probe.arrows.set(self.probe.arrows.get() + 1);
            }
            Message::Edit(edit) => return self.edit(edit),
        }
        true
    }

    /// Apply one step of label editing. Returns whether anything changed.
    pub fn edit(&mut self, edit: Edit) -> bool {
        if edit == Edit::Open {
            if self.label.is_some() {
                return false;
            }
            let Some(node) = self.nearest_node() else {
                return false;
            };
            let text = self.label_text(node);
            self.label = Some(Label {
                node,
                caret: text.len(),
                text,
                preedit: None,
            });
            return true;
        }
        let Some(label) = &mut self.label else {
            return false;
        };
        let before = (label.text.len(), label.caret);
        match edit {
            Edit::Open => {}
            Edit::Insert(text) | Edit::Commit(text) => {
                label.preedit = None;
                label.text.insert_str(label.caret, &text);
                label.caret += text.len();
            }
            Edit::Backspace => {
                if let Some((start, _)) = label.text[..label.caret].char_indices().next_back() {
                    label.text.replace_range(start..label.caret, "");
                    label.caret = start;
                }
            }
            Edit::Left => {
                if let Some((start, _)) = label.text[..label.caret].char_indices().next_back() {
                    label.caret = start;
                }
            }
            Edit::Right => {
                if let Some(next) = label.text[label.caret..].chars().next() {
                    label.caret += next.len_utf8();
                }
            }
            Edit::Preedit { content, selection } => {
                label.preedit = (!content.is_empty()).then_some(Preedit { content, selection });
                return true;
            }
            Edit::Close => {
                self.label = None;
                return true;
            }
        }
        let node = label.node;
        let text = label.text.clone();
        if before.0 != text.len() {
            self.set_label(node, &text);
        }
        true
    }

    /// The caret of the open label, in board units.
    pub fn caret(&self) -> Option<Rect> {
        let label = self.label.as_ref()?;
        let node = self.nodes.get(label.node)?;
        let Draw::Text {
            lines, face, size, ..
        } = &self.scene.items.get(node.text)?.draw
        else {
            return None;
        };
        let before = &label.text[..label.caret];
        let row = before.matches('\n').count();
        let line = lines.get(row)?;
        let prefix = before.rsplit('\n').next().unwrap_or("");
        Some(Rect {
            x: line.origin.x + self.measure.width(prefix, *face, *size),
            y: line.origin.y,
            width: 1.0,
            height: self.measure.line_height(*face, *size),
        })
    }

    /// The caret in widget-local pixels, with the label's text size there.
    pub fn caret_on_screen(&self) -> Option<(Rect, f32)> {
        let caret = self.caret()?;
        let (x, y) = self.viewport.to_screen((caret.x, caret.y));
        let zoom = self.viewport.zoom;
        Some((
            Rect {
                x,
                y,
                width: caret.width.max(1.0 / zoom) * zoom,
                height: caret.height * zoom,
            },
            LABEL_SIZE * zoom,
        ))
    }

    /// The bounds of a generated node, in board units.
    pub fn node_bounds(&self, node: usize) -> Option<Rect> {
        self.nodes.get(node).map(|node| node.bounds)
    }

    /// The lines of a generated node's label.
    pub fn node_lines(&self, node: usize) -> &[Line] {
        self.nodes
            .get(node)
            .and_then(|node| self.scene.items.get(node.text))
            .map_or(&[], |item| match &item.draw {
                Draw::Text { lines, .. } => lines.as_slice(),
                Draw::Path { .. } => &[],
            })
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn label_text(&self, node: usize) -> String {
        self.node_lines(node)
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn set_label(&mut self, node: usize, text: &str) {
        let Some(node) = self.nodes.get(node).copied() else {
            return;
        };
        let lines = centred(text, node.bounds, self.measure.as_ref());
        if let Some(item) = self.scene.items.get_mut(node.text) {
            item.bounds = lines_bounds(&lines, self.measure.as_ref()).unwrap_or(node.bounds);
            if let Draw::Text { lines: drawn, .. } = &mut item.draw {
                *drawn = lines;
            }
        }
        self.replan();
    }

    fn nearest_node(&self) -> Option<usize> {
        let corner = (self.viewport.x, self.viewport.y);
        let distance = |node: &Node| {
            let dx = node.bounds.x - corner.0;
            let dy = node.bounds.y - corner.1;
            dx * dx + dy * dy
        };
        (0..self.nodes.len())
            .min_by(|a, b| distance(&self.nodes[*a]).total_cmp(&distance(&self.nodes[*b])))
    }

    /// The widget for this view, focusable under `id`.
    pub fn view(&self, id: &'static str) -> Element<'_, Message> {
        crate::widget::Canvas::new(self, id).into()
    }
}

/// What a run of the spike measures.
#[cfg(feature = "test-support")]
#[derive(Debug, Default)]
pub struct Probe {
    /// Microseconds spent in each recent `draw`.
    pub draws: RefCell<std::collections::VecDeque<u64>>,
    /// How many times a layer's geometry was built.
    pub builds: std::cell::Cell<u64>,
    /// How many layers the last `draw` painted.
    pub layers: std::cell::Cell<usize>,
    /// How many items the geometry builds have painted in all.
    pub painted: std::cell::Cell<usize>,
    /// When the first `draw` returned.
    pub first_draw: std::cell::Cell<Option<Instant>>,
    /// Every wheel delta as the toolkit reported it.
    pub wheel: RefCell<Vec<Scroll>>,
    /// Arrow keys the focused canvas captured.
    pub arrows: std::cell::Cell<u64>,
    /// The input-method cursor the widget last asked for, in window
    /// coordinates, with the preedit text size; `None` while disabled.
    pub ime: std::cell::Cell<Option<([f32; 4], f32)>>,
}

#[cfg(feature = "test-support")]
impl Probe {
    pub const SAMPLES: usize = 256;

    pub(crate) fn drew(&self, started: Instant) {
        let mut draws = self.draws.borrow_mut();
        if draws.len() == Self::SAMPLES {
            draws.pop_front();
        }
        draws.push_back(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
        if self.first_draw.get().is_none() {
            self.first_draw.set(Some(Instant::now()));
        }
    }
}

/// Label widths from the renderer's own text stack, kept per string: a
/// scene asks for the same strings each time it is rebuilt.
struct Measurer {
    font: Font,
    widths: RefCell<HashMap<(String, u32), f32>>,
}

impl Measurer {
    fn new(font: Font) -> Self {
        Self {
            font,
            widths: RefCell::default(),
        }
    }
}

impl Measure for Measurer {
    fn width(&self, text: &str, face: Face, size: f32) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let key = (text.to_owned(), size.to_bits());
        if let Some(width) = self.widths.borrow().get(&key) {
            return *width;
        }
        let paragraph = <iced::Renderer as text::Renderer>::Paragraph::with_text(text::Text {
            content: text,
            bounds: Size::INFINITE,
            size: Pixels(size),
            line_height: text::LineHeight::Absolute(Pixels(self.line_height(face, size))),
            font: self.font,
            align_x: text::Alignment::Left,
            align_y: iced::alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        });
        let width = paragraph.min_width();
        self.widths.borrow_mut().insert(key, width);
        width
    }

    fn line_height(&self, _face: Face, size: f32) -> f32 {
        if size <= 12.0 {
            16.0
        } else {
            size * 20.0 / 14.0
        }
    }
}

fn id(prefix: char, index: usize) -> Id {
    Id::new(&format!("{prefix}{index}")).expect("a letter and at most twenty digits is an id")
}

fn mix(over: Rgba, under: Rgba, share: f32) -> Rgba {
    let channel = |over: u8, under: u8| {
        (f32::from(over) * share + f32::from(under) * (1.0 - share)).round() as u8
    };
    Rgba::rgb(
        channel(over.r, under.r),
        channel(over.g, under.g),
        channel(over.b, under.b),
    )
}

fn luminance(color: Rgba) -> f32 {
    0.2126 * f32::from(color.r) + 0.7152 * f32::from(color.g) + 0.0722 * f32::from(color.b)
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then_some(Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

fn inset(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x + by,
        y: rect.y + by,
        width: (rect.width - 2.0 * by).max(0.0),
        height: (rect.height - 2.0 * by).max(0.0),
    }
}

fn point(x: f32, y: f32) -> Point {
    Point { x, y }
}

/// A rounded rectangle of radius 8, the `rounded` shape.
fn rounded(rect: Rect) -> Vec<Segment> {
    const RADIUS: f32 = 8.0;
    // The cubic that best fits a quarter circle.
    const KAPPA: f32 = 0.552_284_8;
    let (left, top) = (rect.x, rect.y);
    let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
    let (r, k) = (RADIUS, RADIUS * (1.0 - KAPPA));
    vec![
        Segment::MoveTo(point(left + r, top)),
        Segment::LineTo(point(right - r, top)),
        Segment::CubicTo {
            first: point(right - k, top),
            second: point(right, top + k),
            to: point(right, top + r),
        },
        Segment::LineTo(point(right, bottom - r)),
        Segment::CubicTo {
            first: point(right, bottom - k),
            second: point(right - k, bottom),
            to: point(right - r, bottom),
        },
        Segment::LineTo(point(left + r, bottom)),
        Segment::CubicTo {
            first: point(left + k, bottom),
            second: point(left, bottom - k),
            to: point(left, bottom - r),
        },
        Segment::LineTo(point(left, top + r)),
        Segment::CubicTo {
            first: point(left, top + k),
            second: point(left + k, top),
            to: point(left + r, top),
        },
        Segment::Close,
    ]
}

/// The lines of `text`, split at line breaks only, centred both ways in
/// the label box of a rounded node.
fn centred(text: &str, node: Rect, measure: &dyn Measure) -> Vec<Line> {
    let height = measure.line_height(Face::Regular, LABEL_SIZE);
    let rows: Vec<&str> = text.split('\n').collect();
    let top = node.y + (node.height - height * rows.len() as f32) / 2.0;
    rows.iter()
        .enumerate()
        .map(|(row, text)| {
            let width = measure.width(text, Face::Regular, LABEL_SIZE);
            Line {
                text: (*text).to_owned(),
                origin: point(
                    node.x + (node.width - width) / 2.0,
                    top + height * row as f32,
                ),
                width,
            }
        })
        .collect()
}

fn lines_bounds(lines: &[Line], measure: &dyn Measure) -> Option<Rect> {
    let height = measure.line_height(Face::Regular, LABEL_SIZE);
    lines
        .iter()
        .map(|line| Rect {
            x: line.origin.x,
            y: line.origin.y,
            width: line.width,
            height,
        })
        .reduce(union)
}

/// An elbow connector through `points`, ending in an arrowhead: the line
/// stops at the head's base.
fn connector(points: &[Point], color: Rgba, source: &Source, items: &mut Vec<Item>) {
    const HEAD: (f32, f32) = (10.0, 8.0);
    let [.., before, tip] = points else {
        return;
    };
    let length = ((tip.x - before.x).powi(2) + (tip.y - before.y).powi(2)).sqrt();
    if length <= HEAD.0 {
        return;
    }
    let (ux, uy) = ((tip.x - before.x) / length, (tip.y - before.y) / length);
    let base = point(tip.x - ux * HEAD.0, tip.y - uy * HEAD.0);
    let mut segments = vec![Segment::MoveTo(points[0])];
    segments.extend(
        points[1..points.len() - 1]
            .iter()
            .map(|point| Segment::LineTo(*point)),
    );
    segments.push(Segment::LineTo(base));
    let line = points[..points.len() - 1]
        .iter()
        .chain([&base])
        .map(|point| Rect {
            x: point.x - STROKE / 2.0,
            y: point.y - STROKE / 2.0,
            width: STROKE,
            height: STROKE,
        })
        .reduce(union)
        .unwrap_or_default();
    items.push(Item {
        source: source.clone(),
        bounds: line,
        draw: Draw::Path {
            segments,
            fill: None,
            stroke: Some(Stroke {
                color,
                width: STROKE,
                dash: None,
            }),
        },
    });
    let half = HEAD.1 / 2.0;
    let corners = [
        *tip,
        point(base.x - uy * half, base.y + ux * half),
        point(base.x + uy * half, base.y - ux * half),
    ];
    let head = corners
        .iter()
        .map(|corner| Rect {
            x: corner.x,
            y: corner.y,
            width: 0.0,
            height: 0.0,
        })
        .reduce(union)
        .unwrap_or_default();
    items.push(Item {
        source: source.clone(),
        bounds: head,
        draw: Draw::Path {
            segments: vec![
                Segment::MoveTo(corners[0]),
                Segment::LineTo(corners[1]),
                Segment::LineTo(corners[2]),
                Segment::Close,
            ],
            fill: Some(color),
            stroke: None,
        },
    });
}

fn grid(
    count: usize,
    palette: &Palette,
    measure: &dyn Measure,
) -> (Scene, Vec<Node>, Vec<Overlap>) {
    let columns = ((count as f32).sqrt() * 1.25).ceil().max(1.0) as usize;
    let tint = if palette.dark { 0.16 } else { 0.12 };
    let covers = |index: usize| index % 10 == 9 && !index.is_multiple_of(columns);
    let place = |index: usize| {
        let (column, row) = (index % columns, index / columns);
        let (mut x, mut y) = (column as f32 * PITCH.0, row as f32 * PITCH.1);
        if covers(index) {
            x -= 120.0;
            y += 32.0;
        }
        Rect {
            x,
            y,
            width: NODE.0,
            height: NODE.1,
        }
    };
    let mut items = Vec::with_capacity(count * 6);
    let mut nodes: Vec<Node> = Vec::with_capacity(count);
    let mut overlaps = Vec::new();
    for index in 0..count {
        let bounds = place(index);
        let hue = palette.hues[index % palette.hues.len()];
        let source = Source::Node(id('n', index));
        let (fill, ink) = if covers(index) {
            let ink = if (luminance(hue) - luminance(palette.ink)).abs()
                >= (luminance(hue) - luminance(palette.canvas)).abs()
            {
                palette.ink
            } else {
                palette.canvas
            };
            (hue, ink)
        } else {
            (mix(hue, palette.canvas, tint), palette.ink)
        };
        items.push(Item {
            source: source.clone(),
            bounds: inset(bounds, -STROKE / 2.0),
            draw: Draw::Path {
                segments: rounded(bounds),
                fill: Some(fill),
                stroke: Some(Stroke {
                    color: hue,
                    width: STROKE,
                    dash: None,
                }),
            },
        });
        let text = if index % 10 == 4 {
            format!("検証 {index} 🐦\n二行目のラベル")
        } else {
            format!("Step {index}: validate\nrequest and reply")
        };
        let lines = centred(&text, bounds, measure);
        let label = lines_bounds(&lines, measure).unwrap_or(bounds);
        if covers(index) {
            let covered = nodes
                .last()
                .and_then(|left| intersection(items[left.text].bounds, inset(bounds, 2.0)));
            if let Some(covered) = covered {
                // Stop short of this shape's own label, which is ink too.
                let own = inset(label, -2.0);
                let width = if intersection(covered, own).is_some() {
                    own.x - covered.x
                } else {
                    covered.width
                };
                if width > 0.0 {
                    overlaps.push(Overlap {
                        region: Rect { width, ..covered },
                        fill,
                    });
                }
            }
        }
        nodes.push(Node {
            bounds,
            text: items.len(),
        });
        items.push(Item {
            source: source.clone(),
            bounds: label,
            draw: Draw::Text {
                lines,
                face: Face::Regular,
                size: LABEL_SIZE,
                color: ink,
            },
        });
        let middle = (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        if index + 1 < count && !(index + 1).is_multiple_of(columns) {
            let next = place(index + 1);
            let from = point(bounds.x + bounds.width, middle.1);
            let to = point(next.x, next.y + next.height / 2.0);
            let bend = (from.x + to.x) / 2.0;
            connector(
                &[from, point(bend, from.y), point(bend, to.y), to],
                palette.muted,
                &Source::Edge(id('r', index)),
                &mut items,
            );
        }
        if index + columns < count {
            let below = place(index + columns);
            let from = point(middle.0, bounds.y + bounds.height);
            let to = point(below.x + below.width / 2.0, below.y);
            let bend = (from.y + to.y) / 2.0;
            connector(
                &[from, point(from.x, bend), point(to.x, bend), to],
                palette.muted,
                &Source::Edge(id('d', index)),
                &mut items,
            );
        }
    }
    let bounds = items
        .iter()
        .map(|item| item.bounds)
        .reduce(union)
        .unwrap_or_default();
    (Scene { bounds, items }, nodes, overlaps)
}

fn rows(labels: &[String], palette: &Palette, measure: &dyn Measure) -> Scene {
    let height = measure.line_height(Face::Regular, LABEL_SIZE);
    let items: Vec<Item> = std::iter::once(String::new())
        .chain(labels.iter().cloned())
        .enumerate()
        .map(|(row, label)| {
            let text = format!("|{label}|");
            let width = measure.width(&text, Face::Regular, LABEL_SIZE);
            let origin = point(20.0, 20.0 + row as f32 * (height + 4.0));
            Item {
                source: Source::Node(id('l', row)),
                bounds: Rect {
                    x: origin.x,
                    y: origin.y,
                    width,
                    height,
                },
                draw: Draw::Text {
                    lines: vec![Line {
                        text,
                        origin,
                        width,
                    }],
                    face: Face::Regular,
                    size: LABEL_SIZE,
                    color: palette.ink,
                },
            }
        })
        .collect();
    let bounds = items
        .iter()
        .map(|item| item.bounds)
        .reduce(union)
        .unwrap_or_default();
    Scene { bounds, items }
}

#[cfg(test)]
mod tests {
    use wren_board::scene::FixedMeasure;

    use super::*;

    fn palette() -> Palette {
        Palette {
            canvas: Rgba::rgb(20, 20, 24),
            ink: Rgba::rgb(230, 230, 230),
            muted: Rgba::rgb(120, 120, 130),
            raised: Rgba::rgb(40, 40, 48),
            accent: Rgba::rgb(90, 160, 250),
            issue: Rgba::rgb(240, 80, 80),
            hues: [
                Rgba::rgb(230, 90, 90),
                Rgba::rgb(230, 150, 80),
                Rgba::rgb(220, 200, 90),
                Rgba::rgb(110, 200, 120),
                Rgba::rgb(90, 200, 210),
                Rgba::rgb(170, 130, 230),
                Rgba::rgb(100, 150, 240),
            ],
            dark: true,
        }
    }

    fn fixed(count: usize) -> View {
        let (scene, nodes, overlaps) = grid(count, &palette(), &FixedMeasure);
        View::with(
            scene,
            nodes,
            overlaps,
            palette(),
            Font::DEFAULT,
            Box::new(FixedMeasure),
        )
    }

    #[test]
    fn the_grid_has_every_shape_its_label_and_its_connectors() {
        let view = fixed(1000);
        assert_eq!(view.node_count(), 1000);
        let texts = view
            .scene
            .items
            .iter()
            .filter(|item| matches!(item.draw, Draw::Text { .. }))
            .count();
        assert_eq!(texts, 1000);
        let labelled = |needle: &str| {
            (0..1000)
                .filter(|node| view.label_text(*node).contains(needle))
                .count()
        };
        assert_eq!(labelled("🐦"), 100);
        assert_eq!(labelled("二行目"), 100);
        assert!(view.node_lines(0).len() == 2);
        // Each connector is a line and a head.
        let edges = view
            .scene
            .items
            .iter()
            .filter(|item| matches!(item.source, Source::Edge(_)))
            .count();
        assert!(edges > 1900 * 2 - 200, "{edges}");
        for item in &view.scene.items {
            assert!(item.bounds.x.is_finite() && item.bounds.width >= 0.0);
        }
    }

    #[test]
    fn one_shape_in_ten_covers_its_neighbours_label_in_two_layers() {
        let view = fixed(1000);
        assert!(view.overlaps.len() > 90, "{}", view.overlaps.len());
        assert_eq!(view.plan.count, 2);
        assert_eq!(view.plan.overflow, 0);
        for overlap in &view.overlaps {
            assert!(overlap.region.width >= 8.0 && overlap.region.height >= 8.0);
        }
        // Forcing layers, for cost runs, splits the items evenly.
        let mut forced = fixed(200);
        forced.force_layers(Some(8));
        assert_eq!(forced.plan.count, 8);
        assert!(forced.plan.of.windows(2).all(|pair| pair[0] <= pair[1]));
        forced.force_layers(None);
        assert_eq!(forced.plan.count, 2);
    }

    #[test]
    fn the_wheel_pans_and_the_modifier_zooms_about_the_pointer() {
        let mut view = fixed(10);
        let start = Instant::now();
        let wheel = |y: f32, zoom: bool| Message::Wheel {
            delta: Scroll::Lines { x: 0.0, y },
            at: (100.0, 50.0),
            zoom,
            sideways: false,
        };
        view.update(wheel(-1.0, false), start);
        assert_eq!(
            view.viewport(),
            Viewport {
                x: 0.0,
                y: LINE,
                zoom: 1.0
            }
        );
        let later = start + BURST * 2;
        let under = view.viewport().to_board((100.0, 50.0));
        view.update(wheel(1.0, true), later);
        assert!((view.viewport().zoom - ZOOM_STEP).abs() < 1e-6);
        let after = view.viewport().to_screen(under);
        assert!((after.0 - 100.0).abs() < 0.01 && (after.1 - 50.0).abs() < 0.01);
    }

    #[test]
    fn a_burst_keeps_the_choice_of_its_first_event() {
        let mut view = fixed(10);
        let start = Instant::now();
        let wheel = |zoom: bool| Message::Wheel {
            delta: Scroll::Lines { x: 0.0, y: -1.0 },
            at: (0.0, 0.0),
            zoom,
            sideways: false,
        };
        view.update(wheel(false), start);
        // The modifier goes down mid-burst: still a pan.
        view.update(wheel(true), start + BURST / 2);
        assert_eq!(view.viewport().zoom, 1.0);
        assert_eq!(view.viewport().y, 2.0 * LINE);
        // A new burst reads the modifier again.
        view.update(wheel(true), start + BURST * 3);
        assert!(view.viewport().zoom < 1.0);
    }

    #[test]
    fn pixel_deltas_are_logical_after_the_scale_factor_and_shift_pans_sideways() {
        let mut view = fixed(10);
        view.set_scale_factor(2.0);
        let start = Instant::now();
        view.update(
            Message::Wheel {
                delta: Scroll::Pixels { x: 0.0, y: -30.0 },
                at: (0.0, 0.0),
                zoom: false,
                sideways: false,
            },
            start,
        );
        assert_eq!(view.viewport().y, 15.0);
        view.update(
            Message::Wheel {
                delta: Scroll::Lines { x: 0.0, y: -1.0 },
                at: (0.0, 0.0),
                zoom: false,
                sideways: true,
            },
            start + BURST * 2,
        );
        assert_eq!((view.viewport().x, view.viewport().y), (LINE, 15.0));
    }

    #[test]
    fn the_view_is_moving_until_it_has_rested() {
        let mut view = fixed(10);
        let start = Instant::now();
        assert!(!view.moving(start));
        view.update(Message::Drag { dx: 4.0, dy: 0.0 }, start);
        assert_eq!(view.viewport().x, -4.0);
        assert!(view.moving(start + SETTLE / 2));
        assert!(!view.moving(start + SETTLE));
        assert_eq!(view.settles_at(), Some(start + SETTLE));
    }

    #[test]
    fn a_label_takes_typing_a_composition_and_its_commit() {
        let mut view = fixed(10);
        assert!(view.edit(Edit::Open));
        let label = view.label().expect("an open label").clone();
        assert_eq!(label.node, 0);
        assert_eq!(label.text, "Step 0: validate\nrequest and reply");
        assert_eq!(label.caret, label.text.len());
        let revision = view.revision();

        view.edit(Edit::Insert("!".into()));
        assert_eq!(view.label_text(0), "Step 0: validate\nrequest and reply!");
        assert!(view.revision() > revision);
        // A view made later never repeats a revision an earlier one had.
        assert!(fixed(10).revision() > view.revision());

        // A composition shows as preedit and leaves the text alone.
        view.edit(Edit::Preedit {
            content: "にほん".into(),
            selection: Some(0..9),
        });
        let composing = view.label().expect("still open");
        assert_eq!(
            composing.preedit.as_ref().map(|p| p.content.as_str()),
            Some("にほん")
        );
        assert_eq!(view.label_text(0), "Step 0: validate\nrequest and reply!");

        view.edit(Edit::Commit("日本".into()));
        let committed = view.label().expect("still open");
        assert_eq!(committed.preedit, None);
        assert_eq!(
            view.label_text(0),
            "Step 0: validate\nrequest and reply!日本"
        );

        view.edit(Edit::Backspace);
        view.edit(Edit::Left);
        view.edit(Edit::Insert("x".into()));
        assert_eq!(
            view.label_text(0),
            "Step 0: validate\nrequest and reply!x日"
        );
        view.edit(Edit::Right);
        view.edit(Edit::Right);
        view.edit(Edit::Right);
        assert_eq!(
            view.label().map(|label| label.caret),
            Some(view.label_text(0).len())
        );

        assert!(view.edit(Edit::Close));
        assert_eq!(view.label(), None);
        assert_eq!(
            view.label_text(0),
            "Step 0: validate\nrequest and reply!x日"
        );
        // With no label open, editing steps do nothing.
        assert!(!view.edit(Edit::Insert("z".into())));
    }

    #[test]
    fn the_caret_follows_the_text_and_the_viewport() {
        let mut view = fixed(10);
        view.edit(Edit::Open);
        let lines = view.node_lines(0).to_vec();
        let caret = view.caret().expect("a caret");
        // At the end of the second line.
        assert_eq!(caret.y, lines[1].origin.y);
        assert!(caret.x > lines[1].origin.x);
        view.edit(Edit::Insert("\n".into()));
        let below = view.caret().expect("a caret");
        assert!(below.y > caret.y);

        for zoom in [0.5, 2.0] {
            let viewport = Viewport {
                x: -30.0,
                y: -20.0,
                zoom,
            };
            view.set_viewport(viewport, Instant::now());
            let (on_screen, size) = view.caret_on_screen().expect("a caret");
            let board = view.caret().expect("a caret");
            assert_eq!(on_screen.x, (board.x + 30.0) * zoom);
            assert_eq!(on_screen.y, (board.y + 20.0) * zoom);
            assert_eq!(on_screen.height, board.height * zoom);
            assert_eq!(size, LABEL_SIZE * zoom);
        }
    }
}
