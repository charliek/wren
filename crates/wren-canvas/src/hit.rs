//! What lies under a point of the widget, and what lies inside a marquee
//! (the plan's §5.3 rules 4 and 5).
//!
//! A node is found by its own box and shape, taken from the board, in the
//! stacking order the scene draws; a connector by what the scene draws for
//! it: its line, arrowheads and labels. Handles and the reach of a
//! connector are measured in screen pixels, so they are as easy to take at
//! any zoom.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use wren_board::edit::EdgeEnd;
use wren_board::scene::{self, Draw, Point, Rect, Scene, Segment, Source};
use wren_board::{placements, shape, Board, Id, Named, Node, NodeKind, Placed, Side};

use crate::handles::{self, Handle};
use crate::Viewport;

/// A press within this many screen pixels of a connector's line takes it.
pub const EDGE_REACH: f32 = 6.0;

/// Straight pieces a curve is cut into for hit testing.
const CURVE_STEPS: usize = 16;

/// What a point of the widget lands on, in the order a press tries them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    /// A resize handle of the selected node.
    Resize(Id, Handle),
    /// A connect handle of the node that shows them.
    Connect(Id, Side),
    /// An endpoint handle of the selected connector.
    End(Id, EdgeEnd),
    Node(Id),
    Edge(Id),
    Canvas,
}

impl Hit {
    /// The element the point belongs to, a handle's included.
    pub fn element(&self) -> Option<Source> {
        match self {
            Self::Resize(id, _) | Self::Connect(id, _) | Self::Node(id) => {
                Some(Source::Node(id.clone()))
            }
            Self::End(id, _) | Self::Edge(id) => Some(Source::Edge(id.clone())),
            Self::Canvas => None,
        }
    }
}

/// The handles on screen, which a press tries before any element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown<'a> {
    /// The node showing its eight resize handles.
    pub resize: Option<&'a Id>,
    /// The node showing its four connect handles.
    pub connect: Option<&'a Id>,
    /// The connector showing a handle at each end.
    pub ends: Option<&'a Id>,
}

/// Where each node of a board is drawn, parents applied.
pub struct Frames<'a> {
    board: &'a Board,
    placed: BTreeMap<&'a Id, Placed<'a>>,
}

impl<'a> Frames<'a> {
    pub fn new(board: &'a Board) -> Self {
        Self {
            board,
            placed: placements(board),
        }
    }

    /// The node's absolute top-left corner, exactly as stored.
    pub fn corner(&self, id: &Id) -> Option<[i64; 2]> {
        self.placed.get(id).map(|placed| placed.at)
    }

    /// The node's box as the scene draws it, which clamps out-of-range
    /// values (§3.7): the corner within ±2^24 and the size to at least 1
    /// and to what keeps the far edge within 2^24.
    pub fn get(&self, id: &Id) -> Option<Rect> {
        let corner = self.corner(id)?;
        let size = self.board.nodes.get(id)?.size;
        let limit = scene::COORDINATE_LIMIT;
        let span = |start: i64, length: i32| {
            let start = start.clamp(-limit, limit - 1);
            let length = i64::from(length).clamp(1, limit - start);
            (start as f32, length as f32)
        };
        let (x, width) = span(corner[0], size[0]);
        let (y, height) = span(corner[1], size[1]);
        Some(Rect::new(x, y, width, height))
    }
}

/// What the widget-local point `at` lands on (§5.3 rule 4): the handles
/// `shown`, then nodes from the top of the stacking order down, then
/// connectors, then the canvas.
///
/// A node takes a point inside its outline, whatever its fill; free text
/// and a placeholder take their whole box. Every node comes before any
/// connector. A node the scene draws nothing for, which is free text with
/// a blank label, takes no point.
pub fn hit(board: &Board, scene: &Scene, viewport: Viewport, at: (f32, f32), shown: Shown) -> Hit {
    let frames = Frames::new(board);
    if let Some(found) = handle(board, scene, &frames, viewport, at, shown) {
        return found;
    }
    let (x, y) = viewport.to_board(at);
    let point = Point::new(x, y);
    if let Some(id) = node_at(board, scene, &frames, point) {
        return Hit::Node(id.clone());
    }
    let reach = EDGE_REACH / viewport.zoom;
    for item in scene.items.iter().rev() {
        let Source::Edge(id) = &item.source else {
            continue;
        };
        if !holds(grown(item.bounds, reach), point) {
            continue;
        }
        let on = match &item.draw {
            Draw::Path { segments, fill, .. } => {
                distance(segments, point) <= reach || (fill.is_some() && within(segments, point))
            }
            Draw::Text { .. } => holds(item.bounds, point),
        };
        if on {
            return Hit::Edge(id.clone());
        }
    }
    Hit::Canvas
}

/// The topmost node that takes the board point `point`.
fn node_at<'a>(
    board: &Board,
    scene: &'a Scene,
    frames: &Frames<'_>,
    point: Point,
) -> Option<&'a Id> {
    stacking(scene).into_iter().rev().find(|&id| {
        let (Some(node), Some(bounds)) = (board.nodes.get(id), frames.get(id)) else {
            return false;
        };
        takes(node, bounds, point)
    })
}

/// Whether the node drawn in `bounds` takes the board point `point`: inside
/// a shape's outline, or anywhere in the box of free text or a placeholder.
fn takes(node: &Node, bounds: Rect, point: Point) -> bool {
    holds(bounds, point)
        && match node.kind.as_ref().and_then(Named::known) {
            Some(NodeKind::Shape) => {
                within(&shape::outline(shape::drawn(&node.shape), bounds), point)
            }
            _ => true,
        }
}

/// Where a connector end dropped at the widget-local point `at` attaches
/// (§5.12 rules 1 and 5): pinned to a side of the node whose side handle
/// the point is within reach of ([`handles::pin_reach`]), else floating on
/// the node under the point. The handles lie on the outline, so half of
/// each one's reach is outside its node, on the canvas or on a node behind
/// it; a node in front of the point hides the handles of those it covers.
/// Of several handles in reach the nearest is taken, then the node in
/// front. `None` anywhere else: a connector and a handle of the selection
/// are canvas to a dragged end.
pub fn attach(
    board: &Board,
    scene: &Scene,
    viewport: Viewport,
    at: (f32, f32),
) -> Option<(Id, Option<Side>)> {
    let frames = Frames::new(board);
    let (x, y) = viewport.to_board(at);
    let point = Point::new(x, y);
    let mut pinned: Option<(f32, Side, &Id)> = None;
    // From the front down to the node under the point, once.
    for id in stacking(scene).into_iter().rev() {
        let (Some(node), Some(bounds)) = (board.nodes.get(id), frames.get(id)) else {
            continue;
        };
        let reach = handles::pin_reach(bounds, viewport);
        if holds(grown(bounds, reach / viewport.zoom), point) {
            for (side, centre) in handles::sides(node, bounds, viewport) {
                let apart = (centre.0 - at.0).hypot(centre.1 - at.1);
                // Of two as near, the one found first is in front.
                if apart <= reach && pinned.is_none_or(|(least, ..)| apart < least) {
                    pinned = Some((apart, side, id));
                }
            }
        }
        if takes(node, bounds, point) {
            let (id, side) = pinned.map_or((id, None), |(_, side, id)| (id, Some(side)));
            return Some((id.clone(), side));
        }
    }
    pinned.map(|(_, side, id)| (id.clone(), Some(side)))
}

/// The nodes whose boxes, and the connectors whose drawn bounds, lie
/// entirely inside `area`, in board units (§5.3 rule 5). A connector the
/// scene draws nothing for is in no area.
pub fn inside(board: &Board, scene: &Scene, area: Rect) -> BTreeSet<Source> {
    let covers = |bounds: Rect| {
        bounds.x >= area.x
            && bounds.y >= area.y
            && bounds.right() <= area.right()
            && bounds.bottom() <= area.bottom()
    };
    let frames = Frames::new(board);
    let mut found: BTreeSet<Source> = board
        .nodes
        .keys()
        .filter(|&id| frames.get(id).is_some_and(covers))
        .map(|id| Source::Node(id.clone()))
        .collect();
    let mut drawn: HashMap<&Id, Rect> = HashMap::new();
    for item in &scene.items {
        if let Source::Edge(id) = &item.source {
            drawn
                .entry(id)
                .and_modify(|bounds| *bounds = union(*bounds, item.bounds))
                .or_insert(item.bounds);
        }
    }
    found.extend(
        drawn
            .into_iter()
            .filter(|(_, bounds)| covers(*bounds))
            .map(|(id, _)| Source::Edge(id.clone())),
    );
    found
}

/// The handle within reach of `at`, the nearest when several are.
fn handle(
    board: &Board,
    scene: &Scene,
    frames: &Frames<'_>,
    viewport: Viewport,
    at: (f32, f32),
    shown: Shown,
) -> Option<Hit> {
    // How far a handle's centre is from the point, when within reach.
    let near = |centre: (f32, f32)| {
        let apart = (centre.0 - at.0).abs().max((centre.1 - at.1).abs());
        (apart <= handles::REACH).then_some(apart)
    };
    let nearest = |found: Vec<(f32, Hit)>| {
        found
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, hit)| hit)
    };
    if let Some(id) = shown.resize {
        let found = frames.get(id).into_iter().flat_map(|bounds| {
            handles::resize(bounds, viewport)
                .into_iter()
                .filter_map(|(handle, centre)| {
                    Some((near(centre)?, Hit::Resize(id.clone(), handle)))
                })
        });
        if let Some(hit) = nearest(found.collect()) {
            return Some(hit);
        }
    }
    if let Some(id) = shown.connect {
        let found = frames.get(id).into_iter().flat_map(|bounds| {
            handles::connect(bounds, viewport)
                .into_iter()
                .filter_map(|(side, centre)| Some((near(centre)?, Hit::Connect(id.clone(), side))))
        });
        if let Some(hit) = nearest(found.collect()) {
            return Some(hit);
        }
    }
    if let Some(id) = shown.ends {
        let found = handles::ends(board, scene, id)
            .into_iter()
            .flat_map(|ends| {
                [EdgeEnd::From, EdgeEnd::To]
                    .into_iter()
                    .zip(ends)
                    .filter_map(|(end, point)| {
                        let centre = viewport.to_screen((point.x, point.y));
                        Some((near(centre)?, Hit::End(id.clone(), end)))
                    })
            });
        if let Some(hit) = nearest(found.collect()) {
            return Some(hit);
        }
    }
    None
}

/// The nodes the scene draws, back to front: each where its first item
/// is, since the scene draws a node's own items together and the marker of
/// a flagged one again at the very end.
fn stacking(scene: &Scene) -> Vec<&Id> {
    let mut seen = HashSet::new();
    scene
        .items
        .iter()
        .filter_map(|item| match &item.source {
            Source::Node(id) if seen.insert(id) => Some(id),
            _ => None,
        })
        .collect()
}

fn holds(bounds: Rect, point: Point) -> bool {
    point.x >= bounds.x
        && point.x <= bounds.right()
        && point.y >= bounds.y
        && point.y <= bounds.bottom()
}

fn grown(bounds: Rect, by: f32) -> Rect {
    Rect::new(
        bounds.x - by,
        bounds.y - by,
        bounds.width + 2.0 * by,
        bounds.height + 2.0 * by,
    )
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

/// A path as runs of straight pieces, one run per subpath, with each curve
/// cut into [`CURVE_STEPS`] pieces and a closed subpath ending where it
/// began.
fn flatten(segments: &[Segment]) -> Vec<Vec<Point>> {
    let mut runs: Vec<Vec<Point>> = Vec::new();
    let lerp =
        |a: Point, b: Point, t: f32| Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
    for segment in segments {
        let last = runs.last().and_then(|run| run.last().copied());
        match *segment {
            Segment::MoveTo(point) => runs.push(vec![point]),
            Segment::LineTo(point) => match runs.last_mut() {
                Some(run) => run.push(point),
                None => runs.push(vec![point]),
            },
            Segment::QuadTo { control, to } => {
                let (Some(from), Some(run)) = (last, runs.last_mut()) else {
                    continue;
                };
                for step in 1..=CURVE_STEPS {
                    let t = step as f32 / CURVE_STEPS as f32;
                    run.push(lerp(lerp(from, control, t), lerp(control, to, t), t));
                }
            }
            Segment::CubicTo { first, second, to } => {
                let (Some(from), Some(run)) = (last, runs.last_mut()) else {
                    continue;
                };
                for step in 1..=CURVE_STEPS {
                    let t = step as f32 / CURVE_STEPS as f32;
                    let (a, b, c) = (
                        lerp(from, first, t),
                        lerp(first, second, t),
                        lerp(second, to, t),
                    );
                    run.push(lerp(lerp(a, b, t), lerp(b, c, t), t));
                }
            }
            Segment::Close => {
                if let Some(run) = runs.last_mut() {
                    if let Some(&start) = run.first() {
                        run.push(start);
                    }
                }
            }
        }
    }
    runs
}

/// Whether `point` is inside the path, each subpath taken as closed: a
/// ray to the right crosses the outline an odd number of times.
fn within(segments: &[Segment], point: Point) -> bool {
    let mut inside = false;
    for run in flatten(segments) {
        let closing = run.last().copied().zip(run.first().copied());
        let pieces = run.windows(2).map(|pair| (pair[0], pair[1])).chain(closing);
        for (a, b) in pieces {
            if (a.y > point.y) != (b.y > point.y) {
                let x = a.x + (b.x - a.x) * (point.y - a.y) / (b.y - a.y);
                if point.x < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// The distance from `point` to the nearest piece of the path, in the
/// path's units; infinite for a path with no point.
fn distance(segments: &[Segment], point: Point) -> f32 {
    let to_piece = |a: Point, b: Point| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length = dx * dx + dy * dy;
        let t = if length > 0.0 {
            (((point.x - a.x) * dx + (point.y - a.y) * dy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (point.x - (a.x + dx * t)).hypot(point.y - (a.y + dy * t))
    };
    flatten(segments)
        .iter()
        .flat_map(|run| {
            let alone = (run.len() == 1).then(|| (run[0], run[0]));
            run.windows(2).map(|pair| (pair[0], pair[1])).chain(alone)
        })
        .map(|(a, b)| to_piece(a, b))
        .fold(f32::INFINITY, f32::min)
}

#[cfg(test)]
mod tests {
    use wren_board::scene::FixedMeasure;
    use wren_board::{parse, route};

    use super::*;
    use crate::fixtures;

    const BOARD: &str = r#"schema = 1

[node.back]
text = "Back"
kind = "shape"
at = [0, 0]
size = [200, 120]
z = "A"

[node.front]
text = "Front"
kind = "shape"
at = [100, 60]
size = [200, 120]
style.fill = "none"
z = "B"

[node.inner]
text = "Inner"
kind = "shape"
at = [20, 20]
parent = "group"
size = [80, 40]
z = "A"

[node.group]
kind = "shape"
at = [400, 300]
size = [200, 120]
z = "C"

[node.deep]
kind = "shape"
at = [10, 10]
parent = "inner"
size = [20, 20]
z = "A"

[node.round]
kind = "shape"
at = [0, 300]
shape = "ellipse"
size = [120, 60]
z = "D"

[node.gem]
kind = "shape"
at = [200, 300]
shape = "diamond"
size = [120, 60]
z = "E"

[node.note]
text = "hi"
kind = "text"
at = [0, 500]
size = [120, 40]
z = "F"

[node.blank]
kind = "text"
at = [200, 500]
size = [120, 40]
z = "G"

[node.odd]
kind = "future"
at = [400, 500]
size = [120, 60]
z = "H"

[edge.wire]
kind = "edge"
from = { node = "round" }
to = { at = [60, 460] }
labels = [
  { text = "Yes" },
]
route = "straight"
z = "I"

[edge.over]
kind = "edge"
from = { at = [-100, 30] }
to = { at = [400, 30] }
route = "straight"
z = "J"
"#;

    struct World {
        board: Board,
        scene: Scene,
    }

    impl World {
        fn new(text: &str) -> Self {
            let board = parse(text).expect("a board");
            let scene = scene::build(&board, &fixtures::palette(), &FixedMeasure);
            Self { board, scene }
        }

        /// What the board point `(x, y)` lands on through `viewport`.
        fn at(&self, viewport: Viewport, x: f32, y: f32) -> Hit {
            self.shown(viewport, x, y, Shown::default())
        }

        fn shown(&self, viewport: Viewport, x: f32, y: f32, shown: Shown) -> Hit {
            let at = viewport.to_screen((x, y));
            hit(&self.board, &self.scene, viewport, at, shown)
        }
    }

    fn id(name: &str) -> Id {
        Id::new(name).expect("an id")
    }

    fn node(name: &str) -> Hit {
        Hit::Node(id(name))
    }

    /// Three zooms, each looked at from somewhere else.
    fn viewports() -> [Viewport; 3] {
        [
            Viewport {
                x: -37.0,
                y: 12.5,
                zoom: 0.25,
            },
            Viewport::default(),
            Viewport {
                x: 141.0,
                y: -60.0,
                zoom: 4.0,
            },
        ]
    }

    #[test]
    fn a_point_lands_on_the_topmost_node_at_any_zoom() {
        let world = World::new(BOARD);
        for viewport in viewports() {
            let cases = [
                ((50.0, 30.0), node("back")),
                // Where the two overlap, the one in front, though unfilled.
                ((150.0, 90.0), node("front")),
                ((250.0, 150.0), node("front")),
                ((350.0, 100.0), Hit::Canvas),
                ((-1.0, 60.0), Hit::Canvas),
                ((50.0, 121.0), Hit::Canvas),
            ];
            for ((x, y), want) in cases {
                assert_eq!(world.at(viewport, x, y), want, "({x}, {y}) at {viewport:?}");
            }
        }
    }

    #[test]
    fn a_dragged_end_attaches_to_the_node_under_it_and_to_nothing_else() {
        let world = World::new(BOARD);
        for viewport in viewports() {
            let over = |x: f32, y: f32| {
                let at = viewport.to_screen((x, y));
                attach(&world.board, &world.scene, viewport, at)
            };
            let floating = |name: &str| Some((id(name), None));
            // The middle of a node, the one in front where two overlap, a
            // child where its parents put it.
            assert_eq!(over(50.0, 60.0), floating("back"));
            assert_eq!(over(200.0, 120.0), floating("front"));
            assert_eq!(over(460.0, 340.0), floating("inner"));
            // Free text and a placeholder by their boxes; text the scene
            // draws nothing for, not at all.
            assert_eq!(over(60.0, 520.0), floating("note"));
            assert_eq!(over(460.0, 530.0), floating("odd"));
            assert_eq!(over(260.0, 520.0), None);
            // The corner of an ellipse's box, a connector, a connector's
            // label and the canvas are nothing to attach to.
            assert_eq!(over(3.0, 303.0), None);
            assert_eq!(over(-50.0, 30.0), None);
            assert_eq!(over(60.0, 410.0), None);
            assert_eq!(over(700.0, 100.0), None);
        }
    }

    #[test]
    fn a_dragged_end_is_pinned_by_the_side_handle_it_is_near() {
        let world = World::new(BOARD);
        // Screen pixels are board units at 100 %.
        let viewport = Viewport::default();
        let over = |x: f32, y: f32| attach(&world.board, &world.scene, viewport, (x, y));
        let pinned = |name: &str, side| Some((id(name), Some(side)));
        let floating = |name: &str| Some((id(name), None));
        // `back` is 200 by 120 at the origin.
        assert_eq!(over(100.0, 0.0), pinned("back", Side::Top));
        assert_eq!(over(106.0, 8.0), pinned("back", Side::Top));
        assert_eq!(over(107.0, 8.0), floating("back"));
        assert_eq!(over(0.0, 60.0), pinned("back", Side::Left));
        assert_eq!(over(4.0, 64.0), pinned("back", Side::Left));
        // Half of a handle's reach is outside the outline, on the canvas.
        assert_eq!(over(100.0, -6.0), pinned("back", Side::Top));
        assert_eq!(over(94.0, -8.0), pinned("back", Side::Top));
        assert_eq!(over(100.0, -11.0), None);
        assert_eq!(over(-9.0, 60.0), pinned("back", Side::Left));
        // A node in front hides the handles it covers: `front` lies over
        // the middle of `back`'s right side, which is the middle of its own
        // top side, and its left side's middle is where `back`'s bottom
        // side's is.
        assert_eq!(over(203.0, 62.0), pinned("front", Side::Top));
        assert_eq!(over(204.0, 80.0), floating("front"));
        assert_eq!(over(104.0, 122.0), pinned("front", Side::Left));
        // Of two handles as near, the node in front, wherever the point
        // is; and from behind, a handle of the node in front still reaches.
        assert_eq!(over(96.0, 118.0), pinned("front", Side::Left));
        assert_eq!(over(97.0, 124.0), pinned("front", Side::Left));
        assert_eq!(over(196.0, 54.0), pinned("front", Side::Top));
        // A child's handles reach onto its parent, whose own they are not.
        assert_eq!(over(460.0, 322.0), pinned("inner", Side::Top));
        assert_eq!(over(460.0, 316.0), pinned("inner", Side::Top));
        assert_eq!(over(500.0, 340.0), pinned("inner", Side::Right));
        assert_eq!(over(460.0, 309.0), floating("group"));

        // A diamond's side handles are its corners; an ellipse's lie on
        // its outline.
        assert_eq!(over(260.0, 304.0), pinned("gem", Side::Top));
        assert_eq!(over(204.0, 330.0), pinned("gem", Side::Left));
        assert_eq!(over(196.0, 330.0), pinned("gem", Side::Left));
        assert_eq!(over(60.0, 357.0), pinned("round", Side::Bottom));

        // The reach is screen pixels: at 400 % it is two and a half board
        // units, and at 25 % a node this size keeps a middle that floats.
        let at = |viewport: Viewport, x: f32, y: f32| {
            let at = viewport.to_screen((x, y));
            attach(&world.board, &world.scene, viewport, at)
        };
        let close = Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 4.0,
        };
        assert_eq!(at(close, 100.0, 2.0), pinned("back", Side::Top));
        assert_eq!(at(close, 100.0, 3.0), floating("back"));
        assert_eq!(at(close, 100.0, -3.0), None);
        let far = Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 0.25,
        };
        assert_eq!(at(far, 50.0, 60.0), floating("back"));
        assert_eq!(at(far, 100.0, 20.0), pinned("back", Side::Top));
    }

    #[test]
    fn a_child_is_found_where_its_parents_put_it() {
        let world = World::new(BOARD);
        for viewport in viewports() {
            let cases = [
                // `group` is at 400,300; `inner` 20,20 inside it; `deep`
                // 10,10 inside that.
                ((410.0, 310.0), node("group")),
                ((425.0, 325.0), node("inner")),
                ((495.0, 355.0), node("inner")),
                ((435.0, 335.0), node("deep")),
                ((449.0, 349.0), node("deep")),
                ((451.0, 351.0), node("inner")),
                ((505.0, 365.0), node("group")),
                // Where `inner` would be were its `at` absolute.
                ((30.0, 30.0), node("back")),
            ];
            for ((x, y), want) in cases {
                assert_eq!(world.at(viewport, x, y), want, "({x}, {y}) at {viewport:?}");
            }
        }
    }

    #[test]
    fn a_shape_takes_only_what_is_inside_its_outline() {
        let world = World::new(BOARD);
        for viewport in viewports() {
            let cases = [
                // The ellipse at 0,300, 120 by 60.
                ((60.0, 330.0), node("round")),
                ((4.0, 304.0), Hit::Canvas),
                ((116.0, 356.0), Hit::Canvas),
                ((60.0, 302.0), node("round")),
                // The diamond at 200,300.
                ((260.0, 330.0), node("gem")),
                ((210.0, 306.0), Hit::Canvas),
                ((310.0, 354.0), Hit::Canvas),
                ((232.0, 316.0), node("gem")),
                // Free text takes its whole box, text or not.
                ((5.0, 505.0), node("note")),
                ((110.0, 535.0), node("note")),
                // A placeholder takes its box too.
                ((460.0, 530.0), node("odd")),
                // Free text with nothing to draw takes nothing.
                ((260.0, 520.0), Hit::Canvas),
            ];
            for ((x, y), want) in cases {
                assert_eq!(world.at(viewport, x, y), want, "({x}, {y}) at {viewport:?}");
            }
        }
    }

    #[test]
    fn a_connector_is_taken_within_six_screen_pixels() {
        let world = World::new(BOARD);
        let wire = Hit::Edge(id("wire"));
        // `wire` runs straight down from the ellipse's bottom, at x = 60.
        for viewport in viewports() {
            let reach = EDGE_REACH / viewport.zoom;
            assert_eq!(world.at(viewport, 60.0, 400.0), wire, "{viewport:?}");
            assert_eq!(
                world.at(viewport, 60.0 + reach * 0.9, 370.0),
                wire,
                "{viewport:?}"
            );
            assert_eq!(
                world.at(viewport, 60.0 - reach * 1.1, 370.0),
                Hit::Canvas,
                "{viewport:?}"
            );
            // Past its free end by more than the reach.
            assert_eq!(
                world.at(viewport, 60.0, 460.0 + reach * 1.1),
                Hit::Canvas,
                "{viewport:?}"
            );
        }
    }

    #[test]
    fn a_connector_s_arrowhead_and_label_are_the_connector() {
        let world = World::new(BOARD);
        let wire = Hit::Edge(id("wire"));
        let viewport = Viewport {
            x: 0.0,
            y: 300.0,
            zoom: 4.0,
        };
        // Inside the arrowhead, 2 units from its base and further than
        // that from its sides and from the line's end, where the reach is
        // 1.5 units.
        assert_eq!((route::ARROW_LENGTH, route::ARROW_WIDTH), (10.0, 8.0));
        assert_eq!(world.at(viewport, 60.0, 452.0), wire);
        // The label sits at the middle of the route, on a backing wider
        // than the reach.
        let label = world
            .scene
            .items
            .iter()
            .find(|item| {
                item.source == Source::Edge(id("wire")) && matches!(item.draw, Draw::Text { .. })
            })
            .expect("the label");
        let edge = label.bounds.x + 1.0;
        assert!(60.0 - edge > EDGE_REACH / viewport.zoom);
        assert_eq!(world.at(viewport, edge, label.bounds.y + 1.0), wire);
    }

    #[test]
    fn every_node_comes_before_any_connector() {
        let world = World::new(BOARD);
        // `over` is drawn above everything and crosses `back`.
        for viewport in viewports() {
            assert_eq!(world.at(viewport, 50.0, 30.0), node("back"), "{viewport:?}");
            assert_eq!(
                world.at(viewport, -50.0, 30.0),
                Hit::Edge(id("over")),
                "{viewport:?}"
            );
        }
    }

    #[test]
    fn handles_come_before_everything_and_only_when_shown() {
        let world = World::new(BOARD);
        let front = id("front");
        let shown = Shown {
            resize: Some(&front),
            ..Shown::default()
        };
        for viewport in viewports() {
            // Just outside the top-left corner of `front`, inside `back`.
            let (x, y) = viewport.to_screen((100.0, 60.0));
            let at = (x - 3.0, y - 3.0);
            let plain = hit(&world.board, &world.scene, viewport, at, Shown::default());
            assert_eq!(plain, node("back"), "{viewport:?}");
            let handled = hit(&world.board, &world.scene, viewport, at, shown);
            assert_eq!(
                handled,
                Hit::Resize(front.clone(), Handle::NorthWest),
                "{viewport:?}"
            );
            // Five screen pixels off its centre, still the handle; seven,
            // what lies there.
            let (x, y) = viewport.to_screen((300.0, 180.0));
            let near = hit(
                &world.board,
                &world.scene,
                viewport,
                (x + 5.0, y + 5.0),
                shown,
            );
            assert_eq!(near, Hit::Resize(front.clone(), Handle::SouthEast));
            let far = hit(
                &world.board,
                &world.scene,
                viewport,
                (x + 7.0, y + 7.0),
                shown,
            );
            assert_eq!(far, Hit::Canvas, "{viewport:?}");
        }
        // A side's middle handle, where the side is long enough.
        let viewport = Viewport::default();
        assert_eq!(
            world.shown(viewport, 200.0, 180.0, shown),
            Hit::Resize(front.clone(), Handle::South)
        );
    }

    #[test]
    fn where_handles_overlap_the_nearest_is_taken() {
        let world = World::new(BOARD);
        let deep = id("deep");
        let shown = Shown {
            resize: Some(&deep),
            ..Shown::default()
        };
        // `deep` is 20 units square at 430,330: 5 px at 25 %.
        let viewport = Viewport {
            x: 400.0,
            y: 300.0,
            zoom: 0.25,
        };
        let (x, y) = viewport.to_screen((450.0, 350.0));
        let found = hit(
            &world.board,
            &world.scene,
            viewport,
            (x + 1.0, y + 1.0),
            shown,
        );
        assert_eq!(found, Hit::Resize(deep.clone(), Handle::SouthEast));
        let (x, y) = viewport.to_screen((430.0, 330.0));
        let found = hit(
            &world.board,
            &world.scene,
            viewport,
            (x - 1.0, y - 1.0),
            shown,
        );
        assert_eq!(found, Hit::Resize(deep, Handle::NorthWest));
    }

    #[test]
    fn connect_and_endpoint_handles_are_taken_where_they_sit() {
        let world = World::new(BOARD);
        let (back, wire) = (id("back"), id("wire"));
        for viewport in viewports() {
            let shown = Shown {
                connect: Some(&back),
                ends: Some(&wire),
                ..Shown::default()
            };
            let frame = Frames::new(&world.board).get(&back).expect("the node");
            for (side, centre) in handles::connect(frame, viewport) {
                let found = hit(&world.board, &world.scene, viewport, centre, shown);
                assert_eq!(found, Hit::Connect(back.clone(), side), "{viewport:?}");
            }
            // `wire` leaves the ellipse's bottom and ends free at 60,460.
            assert_eq!(
                world.shown(viewport, 60.0, 360.0, shown),
                Hit::End(wire.clone(), EdgeEnd::From),
                "{viewport:?}"
            );
            assert_eq!(
                world.shown(viewport, 60.0, 460.0, shown),
                Hit::End(wire.clone(), EdgeEnd::To),
                "{viewport:?}"
            );
            assert_eq!(
                world.at(viewport, 60.0, 460.0),
                Hit::Edge(wire.clone()),
                "{viewport:?}"
            );
        }
    }

    #[test]
    fn a_resize_handle_wins_over_a_connect_handle_and_an_endpoint() {
        let world = World::new(BOARD);
        let (round, wire) = (id("round"), id("wire"));
        // The ellipse's bottom middle is its south handle and where `wire`
        // begins.
        let shown = Shown {
            resize: Some(&round),
            connect: None,
            ends: Some(&wire),
        };
        assert_eq!(
            world.shown(Viewport::default(), 60.0, 360.0, shown),
            Hit::Resize(round, Handle::South)
        );
    }

    #[test]
    fn a_marquee_takes_what_lies_entirely_inside_it() {
        let world = World::new(BOARD);
        let names = |area: Rect| -> Vec<String> {
            inside(&world.board, &world.scene, area)
                .iter()
                .map(|source| source.id().to_string())
                .collect()
        };
        // `back` exactly, edge to edge: `front` and `over` reach outside.
        assert_eq!(names(Rect::new(0.0, 0.0, 200.0, 120.0)), ["back"]);
        assert!(names(Rect::new(1.0, 0.0, 199.0, 120.0)).is_empty());
        assert_eq!(
            names(Rect::new(-10.0, -10.0, 320.0, 200.0)),
            ["back", "front"]
        );
        // A parent with its children, by their absolute boxes.
        assert_eq!(
            names(Rect::new(390.0, 290.0, 220.0, 140.0)),
            ["deep", "group", "inner"]
        );
        assert_eq!(
            names(Rect::new(415.0, 315.0, 90.0, 50.0)),
            ["deep", "inner"]
        );
        // A connector with its arrowhead and its label; and blank free
        // text, which a point cannot take.
        assert_eq!(
            names(Rect::new(-10.0, 290.0, 140.0, 180.0)),
            ["round", "wire"]
        );
        assert_eq!(names(Rect::new(40.0, 370.0, 40.0, 70.0)), [""; 0]);
        assert_eq!(names(Rect::new(190.0, 490.0, 140.0, 60.0)), ["blank"]);
        assert_eq!(
            names(Rect::new(-200.0, -10.0, 700.0, 200.0)),
            ["back", "front", "over"]
        );
        assert!(names(Rect::new(1000.0, 1000.0, 0.0, 0.0)).is_empty());
    }

    #[test]
    fn a_frame_is_the_box_the_scene_draws() {
        // Every outlined node of the samples, and one past each limit.
        let far = r#"schema = 1

[node.far]
kind = "shape"
at = [2000000000, -2000000000]
size = [50, 0]

[node.wide]
kind = "shape"
at = [16777000, 0]
size = [2000000000, 60]
"#;
        for text in [fixtures::CHECKOUT_FLOW, fixtures::SHAPES, BOARD, far] {
            let world = World::new(text);
            let frames = Frames::new(&world.board);
            let mut seen = 0;
            for id in stacking(&world.scene) {
                let first = world
                    .scene
                    .items
                    .iter()
                    .find(|item| matches!(&item.source, Source::Node(of) if of == id))
                    .expect("a drawn node");
                if matches!(first.draw, Draw::Path { .. }) {
                    let frame = frames.get(id).expect("a placed node");
                    assert_eq!(
                        grown(frame, route::STROKE_WIDTH / 2.0),
                        first.bounds,
                        "{id}"
                    );
                    seen += 1;
                }
            }
            assert!(seen > 0);
        }
    }

    #[test]
    fn the_stacking_order_ignores_the_markers_of_flagged_nodes() {
        // `low` has no `z`, so it is flagged, and its marker is the last
        // item of the scene; it is still drawn under `high`.
        let world = World::new(
            r#"schema = 1

[node.high]
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "B"

[node.low]
kind = "shape"
at = [0, 0]
size = [120, 60]
"#,
        );
        let last = world.scene.items.last().expect("items");
        assert_eq!(last.source, Source::Node(id("low")));
        assert_eq!(world.at(Viewport::default(), 60.0, 30.0), node("high"));
    }

    #[test]
    fn nothing_is_hit_on_an_empty_board_or_far_away() {
        let empty = World::new(wren_board::EMPTY);
        assert_eq!(empty.at(Viewport::default(), 0.0, 0.0), Hit::Canvas);
        assert!(inside(&empty.board, &empty.scene, Rect::new(-1e6, -1e6, 2e6, 2e6)).is_empty());
        let world = World::new(BOARD);
        for point in [(1e7, 1e7), (f32::NAN, 0.0), (f32::INFINITY, 0.0)] {
            let found = hit(
                &world.board,
                &world.scene,
                Viewport::default(),
                point,
                Shown::default(),
            );
            assert_eq!(found, Hit::Canvas, "{point:?}");
        }
    }

    #[test]
    fn curves_are_measured_as_drawn() {
        // A quarter circle of radius 100 about the origin, as the shapes
        // draw one.
        let k = 0.552_284_8 * 100.0;
        let arc = [
            Segment::MoveTo(Point::new(100.0, 0.0)),
            Segment::CubicTo {
                first: Point::new(100.0, k),
                second: Point::new(k, 100.0),
                to: Point::new(0.0, 100.0),
            },
            Segment::LineTo(Point::new(0.0, 0.0)),
            Segment::Close,
        ];
        let on = Point::new(70.71, 70.71);
        assert!(distance(&arc, on) < 0.2);
        assert!(within(&arc, Point::new(69.0, 69.0)));
        assert!(!within(&arc, Point::new(72.0, 72.0)));
        assert!(within(&arc, Point::new(1.0, 1.0)));
        assert!(!within(&arc, Point::new(-1.0, 1.0)));
        // A lone point and an empty path.
        let dot = [Segment::MoveTo(Point::new(3.0, 4.0))];
        assert_eq!(distance(&dot, Point::new(0.0, 0.0)), 5.0);
        assert_eq!(distance(&[], Point::new(0.0, 0.0)), f32::INFINITY);
        assert!(!within(&[], Point::new(0.0, 0.0)));
    }
}
