//! Connector routes: the polyline between two ends, the side each end
//! meets its node on, and the line and arrowheads drawn along it.
//!
//! A route is computed from the ends alone and never stored. There is no
//! obstacle avoidance: an elbow route avoids only the two nodes it joins,
//! and only where it has to turn back around them.

use crate::scene::{Point, Rect, Segment};
use crate::shape;
use crate::{Head, Named, Route, Shape, Side, Vocabulary};

/// The width of a connector's line and of every shape's outline.
pub const STROKE_WIDTH: f32 = 1.5;
/// A dashed line or outline: on, then off.
pub const DASH: [f32; 2] = [6.0, 4.0];
/// An arrowhead is a filled triangle this long, its tip at the end of the
/// route and the line ending at its base.
pub const ARROW_LENGTH: f32 = 10.0;
/// The width of an arrowhead's base.
pub const ARROW_WIDTH: f32 = 8.0;
/// How far an elbow route runs straight out of a side before it may turn,
/// when it has to turn back or go around a node.
pub const ELBOW_MARGIN: f32 = 20.0;

/// How an end attached to a node meets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    /// No `side` or `anchor`: the router picks the nominal side.
    Floating,
    /// Pinned to the middle of a side.
    Side(Side),
    /// A point of the node's box, per-mille from its top-left corner.
    Anchor([i32; 2]),
}

impl Attach {
    /// An endpoint's `side` and `anchor`, as §4.4 reads them: `anchor`
    /// wins, and an unknown side reads as none.
    pub fn of(side: Option<&Named<Side>>, anchor: Option<[i32; 2]>) -> Self {
        match (anchor, side.and_then(Named::known)) {
            (Some(anchor), _) => Self::Anchor(anchor),
            (None, Some(side)) => Self::Side(side),
            (None, None) => Self::Floating,
        }
    }
}

/// A connector end with its geometry resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Terminal {
    /// Attached to a node, by its absolute box and drawn shape.
    Node {
        bounds: Rect,
        shape: Shape,
        attach: Attach,
    },
    /// A free end at absolute board coordinates.
    Point(Point),
}

impl Terminal {
    /// The node's box, or an empty box at a free end.
    pub fn bounds(&self) -> Rect {
        match *self {
            Self::Node { bounds, .. } => bounds,
            Self::Point(p) => Rect::new(p.x, p.y, 0.0, 0.0),
        }
    }
}

/// The nominal exit side of a connector from `from` to `to` (§5.12 rule
/// 4): down when `to` lies wholly below `from`'s bottom edge, up when
/// wholly above its top edge, otherwise right or left by centre, right on
/// a tie. The nominal entry side at `to` is its opposite.
pub fn nominal_exit(from: Rect, to: Rect) -> Side {
    if to.y >= from.bottom() {
        Side::Bottom
    } else if to.bottom() <= from.y {
        Side::Top
    } else if to.center().x >= from.center().x {
        Side::Right
    } else {
        Side::Left
    }
}

/// The drawn route of `route`: an unknown or reserved value draws as
/// `straight`.
pub fn drawn(route: &Named<Route>) -> Route {
    route.known().unwrap_or(Route::Straight)
}

/// Whether `head` draws an arrowhead: anything but `none` does.
pub fn has_arrow(head: &Named<Head>) -> bool {
    head.known() != Some(Head::None)
}

/// A computed route.
#[derive(Clone, Debug, PartialEq)]
pub struct Routed {
    /// The polyline from the `from` end to the `to` end, at least two
    /// points. An elbow route's segments are all horizontal or vertical.
    pub points: Vec<Point>,
    /// The side the route meets each node on, `from` then `to`: the pinned
    /// side, the side an anchor faces, or the nominal side of a floating
    /// end. `None` at a free end.
    pub sides: [Option<Side>; 2],
    /// The direction the route leaves each end in, outward, so an end
    /// with no length still has a heading for its arrowhead.
    outward: [Side; 2],
}

/// The line and arrowheads of a route, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    /// The stroked line, shortened under each arrowhead.
    pub line: Vec<Segment>,
    /// Each arrowhead as a closed, filled triangle, `from` end first.
    pub heads: Vec<Vec<Segment>>,
}

/// The route of `kind` between two ends. A floating end is always drawn on
/// its nominal side, so it meets the rule of §5.12 rule 4 for every pair.
pub fn route(kind: Route, from: &Terminal, to: &Terminal) -> Routed {
    let (from_box, to_box) = (from.bounds(), to.bounds());
    let exit = nominal_exit(from_box, to_box);
    let (a, a_side, a_shown) = end(from, exit);
    let (b, b_side, b_shown) = end(to, exit.opposite());
    // Ends that meet, as on touching sides, have nothing to route.
    let points = if a == b || kind == Route::Straight {
        vec![a, b]
    } else {
        elbow(a, a_side, from_box, b, b_side, to_box)
    };
    Routed {
        points,
        sides: [a_shown, b_shown],
        outward: [a_side, b_side],
    }
}

/// Where an end meets its node and the side it leaves by, a floating end
/// and a free point taking `nominal`. A connector whose other end names
/// no node draws its stub from here.
pub fn leave(terminal: &Terminal, nominal: Side) -> (Point, Side) {
    let (point, side, _) = end(terminal, nominal);
    (point, side)
}

/// Where an end is, the side it leaves by, and the side it reports.
fn end(terminal: &Terminal, nominal: Side) -> (Point, Side, Option<Side>) {
    match *terminal {
        Terminal::Point(p) => (p, nominal, None),
        Terminal::Node {
            bounds,
            shape,
            attach,
        } => {
            let (point, side) = match attach {
                Attach::Floating => (shape::side_point(shape, bounds, nominal), nominal),
                Attach::Side(side) => (shape::side_point(shape, bounds, side), side),
                Attach::Anchor(anchor) => (
                    shape::anchor_point(bounds, anchor),
                    anchor_side(anchor, nominal),
                ),
            };
            (point, side, Some(side))
        }
    }
}

/// The side an anchor faces: the side of the box nearest to it, the
/// nominal side among equally near ones, else the first in `Side::ALL`.
fn anchor_side(anchor: [i32; 2], nominal: Side) -> Side {
    let [fx, fy] = anchor.map(|v| v.clamp(0, 1000));
    let distance = |side: Side| match side {
        Side::Top => fy,
        Side::Right => 1000 - fx,
        Side::Bottom => 1000 - fy,
        Side::Left => fx,
    };
    let nearest = Side::ALL.iter().map(|&s| distance(s)).min().unwrap_or(0);
    if distance(nominal) == nearest {
        return nominal;
    }
    Side::ALL
        .iter()
        .copied()
        .find(|&s| distance(s) == nearest)
        .unwrap_or(nominal)
}

/// The unit vector pointing out of `side`.
fn outward(side: Side) -> (f32, f32) {
    match side {
        Side::Top => (0.0, -1.0),
        Side::Right => (1.0, 0.0),
        Side::Bottom => (0.0, 1.0),
        Side::Left => (-1.0, 0.0),
    }
}

fn step(p: Point, side: Side, by: f32) -> Point {
    let (dx, dy) = outward(side);
    Point::new(p.x + dx * by, p.y + dy * by)
}

/// How far `to` lies past `from` in the direction out of `side`.
fn ahead(from: Point, to: Point, side: Side) -> f32 {
    let (dx, dy) = outward(side);
    (to.x - from.x) * dx + (to.y - from.y) * dy
}

/// An orthogonal route from `a`, leaving out of side `da` of box `ra`, to
/// `b`, entering through side `db` of box `rb`.
fn elbow(a: Point, da: Side, ra: Rect, b: Point, db: Side, rb: Rect) -> Vec<Point> {
    let m = ELBOW_MARGIN;
    let points = if da.is_horizontal() == db.is_horizontal() {
        if da == db.opposite() {
            if ahead(a, b, da) > 0.0 {
                // Facing sides: a Z through the middle of the gap.
                if da.is_horizontal() {
                    let mid = (a.x + b.x) / 2.0;
                    vec![a, Point::new(mid, a.y), Point::new(mid, b.y), b]
                } else {
                    let mid = (a.y + b.y) / 2.0;
                    vec![a, Point::new(a.x, mid), Point::new(b.x, mid), b]
                }
            } else {
                // Sides facing away: out of each, then across between the
                // boxes, or past both when they overlap on that axis.
                let (qa, qb) = (step(a, da, m), step(b, db, m));
                if da.is_horizontal() {
                    let y = between(ra.y, ra.bottom(), rb.y, rb.bottom());
                    vec![a, qa, Point::new(qa.x, y), Point::new(qb.x, y), qb, b]
                } else {
                    let x = between(ra.x, ra.right(), rb.x, rb.right());
                    vec![a, qa, Point::new(x, qa.y), Point::new(x, qb.y), qb, b]
                }
            }
        } else {
            // The same side of both: a U beyond the further box.
            let edge = match da {
                Side::Top => ra.y.min(rb.y) - m,
                Side::Right => ra.right().max(rb.right()) + m,
                Side::Bottom => ra.bottom().max(rb.bottom()) + m,
                Side::Left => ra.x.min(rb.x) - m,
            };
            if da.is_horizontal() {
                vec![a, Point::new(edge, a.y), Point::new(edge, b.y), b]
            } else {
                vec![a, Point::new(a.x, edge), Point::new(b.x, edge), b]
            }
        }
    } else {
        let corner = if da.is_horizontal() {
            Point::new(b.x, a.y)
        } else {
            Point::new(a.x, b.y)
        };
        if ahead(a, corner, da) > 0.0 && ahead(b, corner, db) > 0.0 {
            // One turn, out of both sides.
            vec![a, corner, b]
        } else {
            let (qa, qb) = (step(a, da, m), step(b, db, m));
            let turn = if da.is_horizontal() {
                Point::new(qa.x, qb.y)
            } else {
                Point::new(qb.x, qa.y)
            };
            vec![a, qa, turn, qb, b]
        }
    };
    simplify(points)
}

/// A coordinate between two spans: the middle of the gap when they are
/// apart, else [`ELBOW_MARGIN`] past the further end of both.
fn between(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    if a1 <= b0 {
        (a1 + b0) / 2.0
    } else if b1 <= a0 {
        (b1 + a0) / 2.0
    } else {
        a1.max(b1) + ELBOW_MARGIN
    }
}

/// Drops repeated points and the middle of three in a straight run, so a
/// route has no zero-length segment and no needless bend. A point where
/// the route turns back is kept.
fn simplify(points: Vec<Point>) -> Vec<Point> {
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return points;
    };
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        if out.last() == Some(&p) {
            continue;
        }
        if let [.., before, middle] = out.as_slice() {
            let straight = (before.x == middle.x
                && middle.x == p.x
                && (middle.y - before.y) * (p.y - middle.y) >= 0.0)
                || (before.y == middle.y
                    && middle.y == p.y
                    && (middle.x - before.x) * (p.x - middle.x) >= 0.0);
            if straight {
                out.pop();
            }
        }
        out.push(p);
    }
    if out.len() < 2 {
        return vec![first, last];
    }
    out
}

fn distance(a: Point, b: Point) -> f32 {
    (b.x - a.x).hypot(b.y - a.y)
}

impl Routed {
    /// The length along the route.
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| distance(w[0], w[1])).sum()
    }

    /// The point `at` per-mille of the way along the route from the `from`
    /// end, clamped to 0 to 1000: where a connector label sits.
    pub fn point_at(&self, at: i32) -> Point {
        let target = self.length() * at.clamp(0, 1000) as f32 / 1000.0;
        along(&self.points, target)
    }

    /// The line, and an arrowhead at each end whose flag is set.
    pub fn draw(&self, heads: [bool; 2]) -> Drawn {
        let length = self.length();
        let mut trims = heads.map(|on| if on { ARROW_LENGTH } else { 0.0 });
        let wanted = trims[0] + trims[1];
        if wanted > length {
            trims = trims.map(|t| t * length / wanted);
        }
        let start = along(&self.points, trims[0]);
        let stop = along(&self.points, length - trims[1]);
        let mut line = vec![Segment::MoveTo(start)];
        let mut walked = 0.0;
        for pair in self.points.windows(2) {
            walked += distance(pair[0], pair[1]);
            if walked > trims[0] && walked < length - trims[1] {
                line.push(Segment::LineTo(pair[1]));
            }
        }
        line.push(Segment::LineTo(stop));

        let mut drawn = Vec::new();
        if heads[0] {
            let tip = self.points[0];
            let toward = self.points.iter().copied().find(|&p| p != tip);
            drawn.push(arrowhead(tip, toward, self.outward[0]));
        }
        if heads[1] {
            let tip = self.points[self.points.len() - 1];
            let toward = self.points.iter().rev().copied().find(|&p| p != tip);
            drawn.push(arrowhead(tip, toward, self.outward[1]));
        }
        Drawn { line, heads: drawn }
    }
}

/// The point `target` along a polyline, clamped to its ends.
fn along(points: &[Point], target: f32) -> Point {
    let mut left = target.max(0.0);
    for pair in points.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        let d = distance(p, q);
        if left <= d && d > 0.0 {
            let t = left / d;
            return Point::new(p.x + (q.x - p.x) * t, p.y + (q.y - p.y) * t);
        }
        left -= d;
    }
    points.last().copied().unwrap_or_default()
}

/// A filled triangle with its tip at `tip`, pointing away from `toward`
/// (the route's previous point), or into the side the end leaves by when
/// the route has no length.
fn arrowhead(tip: Point, toward: Option<Point>, out: Side) -> Vec<Segment> {
    let (ux, uy) = match toward {
        Some(p) => {
            let d = distance(p, tip);
            ((tip.x - p.x) / d, (tip.y - p.y) / d)
        }
        None => {
            let (x, y) = outward(out);
            (-x, -y)
        }
    };
    let base = Point::new(tip.x - ux * ARROW_LENGTH, tip.y - uy * ARROW_LENGTH);
    let (nx, ny) = (-uy * ARROW_WIDTH / 2.0, ux * ARROW_WIDTH / 2.0);
    vec![
        Segment::MoveTo(tip),
        Segment::LineTo(Point::new(base.x + nx, base.y + ny)),
        Segment::LineTo(Point::new(base.x - nx, base.y - ny)),
        Segment::Close,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::Rng;
    use crate::shape::tests::distance as off_outline;

    fn node(bounds: Rect, attach: Attach) -> Terminal {
        Terminal::Node {
            bounds,
            shape: Shape::Rect,
            attach,
        }
    }

    fn p(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    const A: Rect = Rect::new(0.0, 0.0, 120.0, 60.0);

    /// Box positions relative to `A`, with the name of the case.
    fn layouts() -> Vec<(&'static str, Rect, Rect)> {
        vec![
            ("right", A, Rect::new(240.0, 20.0, 120.0, 60.0)),
            ("left", A, Rect::new(-240.0, -10.0, 120.0, 60.0)),
            ("below", A, Rect::new(30.0, 160.0, 120.0, 60.0)),
            ("above", A, Rect::new(-30.0, -160.0, 120.0, 60.0)),
            ("below right", A, Rect::new(300.0, 200.0, 80.0, 40.0)),
            ("above left", A, Rect::new(-300.0, -200.0, 80.0, 40.0)),
            ("below left", A, Rect::new(-300.0, 200.0, 80.0, 40.0)),
            ("above right", A, Rect::new(300.0, -200.0, 80.0, 40.0)),
            ("touching right", A, Rect::new(120.0, 0.0, 120.0, 60.0)),
            ("touching below", A, Rect::new(0.0, 60.0, 120.0, 60.0)),
            ("touching corner", A, Rect::new(120.0, 60.0, 120.0, 60.0)),
            ("overlapping", A, Rect::new(60.0, 30.0, 120.0, 60.0)),
            ("inside", A, Rect::new(20.0, 10.0, 40.0, 20.0)),
            ("coincident", A, A),
            ("zero-size apart", A, Rect::new(300.0, 300.0, 0.0, 0.0)),
            ("zero-size inside", A, Rect::new(60.0, 30.0, 0.0, 0.0)),
            ("zero-size on the edge", A, Rect::new(120.0, 30.0, 0.0, 0.0)),
            (
                "both zero-size",
                Rect::new(0.0, 0.0, 0.0, 0.0),
                Rect::new(40.0, 40.0, 0.0, 0.0),
            ),
            (
                "both zero-size, coincident",
                Rect::new(5.0, 5.0, 0.0, 0.0),
                Rect::new(5.0, 5.0, 0.0, 0.0),
            ),
            (
                "far",
                A,
                Rect::new(16_000_000.0, -16_000_000.0, 120.0, 60.0),
            ),
        ]
    }

    fn attaches() -> Vec<Attach> {
        let mut all = vec![Attach::Floating];
        all.extend(Side::ALL.iter().map(|&s| Attach::Side(s)));
        all.extend([[500, 0], [1000, 1000], [250, 600], [500, 500]].map(Attach::Anchor));
        all
    }

    /// Every invariant a route must hold, whatever its ends.
    fn assert_sound(kind: Route, from: &Terminal, to: &Terminal, routed: &Routed, case: &str) {
        let points = &routed.points;
        assert!(points.len() >= 2, "{case}: {points:?}");
        for q in points {
            assert!(q.x.is_finite() && q.y.is_finite(), "{case}: {points:?}");
        }
        if kind == Route::Elbow {
            for w in points.windows(2) {
                assert!(
                    w[0].x == w[1].x || w[0].y == w[1].y,
                    "{case}: a diagonal elbow segment in {points:?}"
                );
            }
        } else {
            assert_eq!(points.len(), 2, "{case}");
        }
        for (terminal, &point, side) in [
            (from, &points[0], routed.sides[0]),
            (to, points.last().unwrap(), routed.sides[1]),
        ] {
            match *terminal {
                Terminal::Point(at) => {
                    assert_eq!(point, at, "{case}");
                    assert_eq!(side, None, "{case}");
                }
                Terminal::Node {
                    bounds,
                    shape,
                    attach,
                } => {
                    let side = side.expect("a node end reports its side");
                    match attach {
                        Attach::Anchor(anchor) => {
                            assert_eq!(point, shape::anchor_point(bounds, anchor), "{case}")
                        }
                        Attach::Side(pinned) => {
                            assert_eq!(side, pinned, "{case}: a pinned end moved");
                            assert_eq!(point, shape::side_point(shape, bounds, side), "{case}");
                        }
                        Attach::Floating => {
                            assert_eq!(point, shape::side_point(shape, bounds, side), "{case}")
                        }
                    }
                    if !matches!(attach, Attach::Anchor(_)) {
                        let outline = shape::outline(shape, bounds);
                        let off = off_outline(&outline, point);
                        assert!(
                            off < 0.01 * (1.0 + bounds.x.abs().max(bounds.y.abs()) * 1e-3),
                            "{case}: end {point:?} is {off} off the outline"
                        );
                    }
                }
            }
        }
        let drawn = routed.draw([true, true]);
        for segment in drawn.line.iter().chain(drawn.heads.iter().flatten()) {
            let ps: Vec<Point> = match *segment {
                Segment::MoveTo(q) | Segment::LineTo(q) => vec![q],
                Segment::Close => vec![],
                other => panic!("{case}: a route draws only lines, not {other:?}"),
            };
            for q in ps {
                assert!(q.x.is_finite() && q.y.is_finite(), "{case}: {drawn:?}");
            }
        }
        let mid = routed.point_at(500);
        assert!(mid.x.is_finite() && mid.y.is_finite(), "{case}");
    }

    #[test]
    fn the_routing_table() {
        // Every pair of ends (floating, the four pinned sides, anchors) on
        // every layout, every shape, both routes, with a free end too.
        let mut cases = 0;
        for (name, ra, rb) in layouts() {
            for &shape in Shape::ALL {
                for &fa in &attaches() {
                    for &fb in &attaches() {
                        for kind in [Route::Straight, Route::Elbow] {
                            let from = Terminal::Node {
                                bounds: ra,
                                shape,
                                attach: fa,
                            };
                            let to = Terminal::Node {
                                bounds: rb,
                                shape,
                                attach: fb,
                            };
                            let case = format!("{name}, {shape:?}, {fa:?} to {fb:?}, {kind:?}");
                            assert_sound(kind, &from, &to, &route(kind, &from, &to), &case);
                            cases += 1;
                        }
                    }
                }
                for kind in [Route::Straight, Route::Elbow] {
                    let free = Terminal::Point(rb.center());
                    for &fa in &attaches() {
                        let from = Terminal::Node {
                            bounds: ra,
                            shape,
                            attach: fa,
                        };
                        let case = format!("{name}, {shape:?}, {fa:?} to a free end, {kind:?}");
                        assert_sound(kind, &from, &free, &route(kind, &from, &free), &case);
                        assert_sound(kind, &free, &from, &route(kind, &free, &from), &case);
                    }
                    let both = Terminal::Point(ra.center());
                    let case = format!("{name}, two free ends, {kind:?}");
                    assert_sound(kind, &both, &free, &route(kind, &both, &free), &case);
                }
            }
        }
        assert!(cases > 20_000, "{cases}");
    }

    #[test]
    fn the_sixteen_side_pairs_on_a_side_by_side_layout() {
        // B to the right of A and lower, apart on both axes. Each pinned
        // pair gives the expected polyline.
        let b = Rect::new(240.0, 100.0, 120.0, 60.0);
        use Side::*;
        let cases: [(Side, Side, Vec<Point>); 16] = [
            (
                Right,
                Left,
                vec![
                    p(120.0, 30.0),
                    p(180.0, 30.0),
                    p(180.0, 130.0),
                    p(240.0, 130.0),
                ],
            ),
            (
                Right,
                Top,
                vec![p(120.0, 30.0), p(300.0, 30.0), p(300.0, 100.0)],
            ),
            (
                Right,
                Right,
                vec![
                    p(120.0, 30.0),
                    p(380.0, 30.0),
                    p(380.0, 130.0),
                    p(360.0, 130.0),
                ],
            ),
            (
                Right,
                Bottom,
                vec![
                    p(120.0, 30.0),
                    p(140.0, 30.0),
                    p(140.0, 180.0),
                    p(300.0, 180.0),
                    p(300.0, 160.0),
                ],
            ),
            (
                Bottom,
                Top,
                vec![
                    p(60.0, 60.0),
                    p(60.0, 80.0),
                    p(300.0, 80.0),
                    p(300.0, 100.0),
                ],
            ),
            (
                Bottom,
                Left,
                vec![p(60.0, 60.0), p(60.0, 130.0), p(240.0, 130.0)],
            ),
            (
                Bottom,
                Bottom,
                vec![
                    p(60.0, 60.0),
                    p(60.0, 180.0),
                    p(300.0, 180.0),
                    p(300.0, 160.0),
                ],
            ),
            (
                Bottom,
                Right,
                vec![
                    p(60.0, 60.0),
                    p(60.0, 80.0),
                    p(380.0, 80.0),
                    p(380.0, 130.0),
                    p(360.0, 130.0),
                ],
            ),
            (
                Left,
                Right,
                vec![
                    p(0.0, 30.0),
                    p(-20.0, 30.0),
                    p(-20.0, 80.0),
                    p(380.0, 80.0),
                    p(380.0, 130.0),
                    p(360.0, 130.0),
                ],
            ),
            (
                Left,
                Left,
                vec![
                    p(0.0, 30.0),
                    p(-20.0, 30.0),
                    p(-20.0, 130.0),
                    p(240.0, 130.0),
                ],
            ),
            (
                Left,
                Top,
                vec![
                    p(0.0, 30.0),
                    p(-20.0, 30.0),
                    p(-20.0, 80.0),
                    p(300.0, 80.0),
                    p(300.0, 100.0),
                ],
            ),
            (
                Left,
                Bottom,
                vec![
                    p(0.0, 30.0),
                    p(-20.0, 30.0),
                    p(-20.0, 180.0),
                    p(300.0, 180.0),
                    p(300.0, 160.0),
                ],
            ),
            (
                Top,
                Bottom,
                vec![
                    p(60.0, 0.0),
                    p(60.0, -20.0),
                    p(180.0, -20.0),
                    p(180.0, 180.0),
                    p(300.0, 180.0),
                    p(300.0, 160.0),
                ],
            ),
            (
                Top,
                Top,
                vec![
                    p(60.0, 0.0),
                    p(60.0, -20.0),
                    p(300.0, -20.0),
                    p(300.0, 100.0),
                ],
            ),
            (
                Top,
                Left,
                vec![
                    p(60.0, 0.0),
                    p(60.0, -20.0),
                    p(220.0, -20.0),
                    p(220.0, 130.0),
                    p(240.0, 130.0),
                ],
            ),
            (
                Top,
                Right,
                vec![
                    p(60.0, 0.0),
                    p(60.0, -20.0),
                    p(380.0, -20.0),
                    p(380.0, 130.0),
                    p(360.0, 130.0),
                ],
            ),
        ];
        for (sa, sb, want) in cases {
            let routed = route(
                Route::Elbow,
                &node(A, Attach::Side(sa)),
                &node(b, Attach::Side(sb)),
            );
            assert_eq!(routed.points, want, "{sa:?} to {sb:?}");
            assert_eq!(routed.sides, [Some(sa), Some(sb)]);
            let straight = route(
                Route::Straight,
                &node(A, Attach::Side(sa)),
                &node(b, Attach::Side(sb)),
            );
            assert_eq!(straight.points, [want[0], *want.last().unwrap()]);
        }
    }

    #[test]
    fn elbow_routes_leave_and_enter_through_their_sides() {
        // Away from the degenerate layouts, the first segment leaves its
        // side outward and the last arrives through the other inward.
        for (name, ra, rb) in layouts().into_iter().take(11) {
            for &sa in Side::ALL {
                for &sb in Side::ALL {
                    let routed = route(
                        Route::Elbow,
                        &node(ra, Attach::Side(sa)),
                        &node(rb, Attach::Side(sb)),
                    );
                    let pts = &routed.points;
                    if pts[0] == pts[pts.len() - 1] {
                        continue;
                    }
                    let first = ahead(pts[0], pts[1], sa);
                    let last = ahead(pts[pts.len() - 1], pts[pts.len() - 2], sb);
                    assert!(first > 0.0, "{name}, {sa:?} to {sb:?}: {pts:?}");
                    assert!(last > 0.0, "{name}, {sa:?} to {sb:?}: {pts:?}");
                }
            }
        }
    }

    #[test]
    fn nominal_sides() {
        use Side::*;
        let cases = [
            (Rect::new(0.0, 200.0, 120.0, 60.0), Bottom),
            (Rect::new(0.0, 60.0, 120.0, 60.0), Bottom),
            (Rect::new(500.0, 61.0, 10.0, 10.0), Bottom),
            (Rect::new(0.0, -200.0, 120.0, 60.0), Top),
            (Rect::new(-500.0, -10.0, 10.0, 10.0), Top),
            (Rect::new(300.0, 0.0, 120.0, 60.0), Right),
            (Rect::new(300.0, 59.0, 120.0, 60.0), Right),
            (Rect::new(-300.0, -59.0, 120.0, 60.0), Left),
            (Rect::new(1.0, 1.0, 120.0, 60.0), Right),
            (Rect::new(-1.0, 1.0, 120.0, 60.0), Left),
            (A, Right),
        ];
        for (to, want) in cases {
            assert_eq!(nominal_exit(A, to), want, "{to:?}");
        }
        assert_eq!(Bottom.opposite(), Top);
        assert_eq!(Left.opposite(), Right);
    }

    /// Two boxes that overlap on one axis and are apart on the other, and
    /// the direction from the first to the second.
    fn overlap_on_one_axis(rng: &mut Rng) -> (Rect, Rect, Side) {
        let size = |rng: &mut Rng| 1.0 + rng.below(300) as f32;
        let a = Rect::new(
            rng.below(2_000) as f32 - 1_000.0,
            rng.below(2_000) as f32 - 1_000.0,
            size(rng),
            size(rng),
        );
        let (w, h) = (size(rng), size(rng));
        let gap = 1.0 + rng.below(400) as f32;
        let side = *rng.pick(Side::ALL);
        // Across the direction, the spans share at least one unit.
        let across = |rng: &mut Rng, start: f32, own: f32, other: f32| {
            let lo = start - other + 1.0;
            let hi = start + own - 1.0;
            lo + rng.below((hi - lo).max(0.0) as usize + 1) as f32
        };
        let b = match side {
            Side::Right => Rect::new(a.right() + gap, across(rng, a.y, a.height, h), w, h),
            Side::Left => Rect::new(a.x - gap - w, across(rng, a.y, a.height, h), w, h),
            Side::Bottom => Rect::new(across(rng, a.x, a.width, w), a.bottom() + gap, w, h),
            Side::Top => Rect::new(across(rng, a.x, a.width, w), a.y - gap - h, w, h),
        };
        (a, b, side)
    }

    #[test]
    fn floating_ends_take_the_nominal_side_when_apart_on_one_axis() {
        let mut rng = Rng(0x00c0_ffee);
        for round in 0..2_000 {
            let (ra, rb, side) = overlap_on_one_axis(&mut rng);
            let overlap_x = ra.x < rb.right() && rb.x < ra.right();
            let overlap_y = ra.y < rb.bottom() && rb.y < ra.bottom();
            assert!(overlap_x != overlap_y, "{round}: {ra:?} {rb:?}");
            let shape = *rng.pick(Shape::ALL);
            let end = |bounds| Terminal::Node {
                bounds,
                shape,
                attach: Attach::Floating,
            };
            for kind in [Route::Straight, Route::Elbow] {
                let routed = route(kind, &end(ra), &end(rb));
                let case = format!("{round}: {ra:?} to {rb:?}, {shape:?}, {kind:?}");
                assert_eq!(routed.sides, [Some(side), Some(side.opposite())], "{case}");
                let pts = &routed.points;
                assert_eq!(pts[0], shape::side_point(shape, ra, side), "{case}");
                assert_eq!(
                    pts[pts.len() - 1],
                    shape::side_point(shape, rb, side.opposite()),
                    "{case}"
                );
                if kind == Route::Elbow {
                    // Out of the exit side and in through the entry side.
                    assert!(ahead(pts[0], pts[1], side) > 0.0, "{case}: {pts:?}");
                    let n = pts.len();
                    assert!(
                        ahead(pts[n - 1], pts[n - 2], side.opposite()) > 0.0,
                        "{case}: {pts:?}"
                    );
                    assert!(
                        n <= 4,
                        "{case}: facing sides need at most two turns: {pts:?}"
                    );
                }
                // The same pair the other way round swaps the sides.
                let back = route(kind, &end(rb), &end(ra));
                assert_eq!(back.sides, [Some(side.opposite()), Some(side)], "{case}");
            }
        }
    }

    #[test]
    fn pinned_and_free_ends() {
        let b = Rect::new(240.0, 0.0, 120.0, 60.0);
        // A pinned end keeps its side; the floating one still takes the
        // nominal side of the pair.
        let routed = route(
            Route::Elbow,
            &node(A, Attach::Side(Side::Top)),
            &node(b, Attach::Floating),
        );
        assert_eq!(routed.sides, [Some(Side::Top), Some(Side::Left)]);
        assert_eq!(
            routed.points,
            [
                p(60.0, 0.0),
                p(60.0, -20.0),
                p(220.0, -20.0),
                p(220.0, 30.0),
                p(240.0, 30.0)
            ]
        );
        // An unknown side reads as none, and an anchor wins over a side.
        assert_eq!(
            Attach::of(Some(&Named::Other("north".into())), None),
            Attach::Floating
        );
        assert_eq!(
            Attach::of(Some(&Named::Known(Side::Left)), Some([500, 0])),
            Attach::Anchor([500, 0])
        );
        assert_eq!(
            Attach::of(Some(&Named::Known(Side::Left)), None),
            Attach::Side(Side::Left)
        );
        // A free end routes as a point and reports no side.
        let free = route(
            Route::Elbow,
            &Terminal::Point(p(300.0, 200.0)),
            &node(A, Attach::Floating),
        );
        assert_eq!(free.sides, [None, Some(Side::Bottom)]);
        assert_eq!(
            free.points,
            [
                p(300.0, 200.0),
                p(300.0, 130.0),
                p(60.0, 130.0),
                p(60.0, 60.0)
            ]
        );
    }

    #[test]
    fn anchors_face_their_nearest_side() {
        use Side::*;
        let cases = [
            ([500, 0], Right, Top),
            ([1000, 500], Bottom, Right),
            ([500, 1000], Top, Bottom),
            ([0, 500], Right, Left),
            ([100, 100], Bottom, Top),
            ([500, 500], Bottom, Bottom),
            ([500, 500], Left, Left),
            ([0, 0], Right, Top),
            ([0, 0], Left, Left),
            ([-50, 2000], Right, Bottom),
        ];
        for (anchor, nominal, want) in cases {
            assert_eq!(anchor_side(anchor, nominal), want, "{anchor:?} {nominal:?}");
        }
        let b = Rect::new(240.0, 0.0, 120.0, 60.0);
        let routed = route(
            Route::Elbow,
            &node(A, Attach::Anchor([500, 0])),
            &node(b, Attach::Anchor([0, 500])),
        );
        assert_eq!(routed.sides, [Some(Top), Some(Left)]);
        assert_eq!(routed.points[0], p(60.0, 0.0));
    }

    #[test]
    fn coincident_and_zero_size_routes() {
        // A connector from a node to itself loops around below it.
        let routed = route(
            Route::Elbow,
            &node(A, Attach::Floating),
            &node(A, Attach::Floating),
        );
        assert_eq!(routed.sides, [Some(Side::Right), Some(Side::Left)]);
        assert_eq!(
            routed.points,
            [
                p(120.0, 30.0),
                p(140.0, 30.0),
                p(140.0, 80.0),
                p(-20.0, 80.0),
                p(-20.0, 30.0),
                p(0.0, 30.0)
            ]
        );
        // Two coincident points make a route of no length.
        let dot = Terminal::Point(p(5.0, 5.0));
        let routed = route(Route::Elbow, &dot, &dot);
        assert_eq!(routed.points, [p(5.0, 5.0), p(5.0, 5.0)]);
        assert_eq!(routed.length(), 0.0);
        let drawn = routed.draw([true, true]);
        assert_eq!(drawn.heads.len(), 2);
        // With no length the head points into the side it entered by.
        assert_eq!(drawn.heads[1][0], Segment::MoveTo(p(5.0, 5.0)));
        assert_eq!(
            drawn.heads[1][1],
            Segment::LineTo(p(1.0, -5.0)),
            "the entry side is the top, so the head points down"
        );
    }

    #[test]
    fn straight_and_unknown_routes() {
        let b = Rect::new(240.0, 100.0, 120.0, 60.0);
        let routed = route(
            Route::Straight,
            &node(A, Attach::Floating),
            &node(b, Attach::Floating),
        );
        assert_eq!(routed.points, [p(60.0, 60.0), p(300.0, 100.0)]);
        assert_eq!(drawn(&Named::Known(Route::Elbow)), Route::Elbow);
        assert_eq!(drawn(&Named::Other("curved".into())), Route::Straight);
        assert_eq!(drawn(&Named::Other("zigzag".into())), Route::Straight);
        assert!(has_arrow(&Named::Known(Head::Arrow)));
        assert!(!has_arrow(&Named::Known(Head::None)));
        assert!(has_arrow(&Named::Other("diamond".into())));
    }

    #[test]
    fn arrowheads_sit_at_the_ends_and_the_line_stops_at_their_base() {
        let routed = Routed {
            points: vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0)],
            sides: [Some(Side::Right), Some(Side::Top)],
            outward: [Side::Right, Side::Top],
        };
        let drawn = routed.draw([false, true]);
        assert_eq!(
            drawn.line,
            [
                Segment::MoveTo(p(0.0, 0.0)),
                Segment::LineTo(p(100.0, 0.0)),
                Segment::LineTo(p(100.0, 40.0)),
            ]
        );
        assert_eq!(
            drawn.heads,
            [vec![
                Segment::MoveTo(p(100.0, 50.0)),
                Segment::LineTo(p(96.0, 40.0)),
                Segment::LineTo(p(104.0, 40.0)),
                Segment::Close,
            ]]
        );
        let both = routed.draw([true, true]);
        assert_eq!(both.line[0], Segment::MoveTo(p(10.0, 0.0)));
        assert_eq!(
            both.heads[0],
            [
                Segment::MoveTo(p(0.0, 0.0)),
                Segment::LineTo(p(10.0, -4.0)),
                Segment::LineTo(p(10.0, 4.0)),
                Segment::Close,
            ]
        );
        let none = routed.draw([false, false]);
        assert_eq!(
            none.line,
            [
                Segment::MoveTo(p(0.0, 0.0)),
                Segment::LineTo(p(100.0, 0.0)),
                Segment::LineTo(p(100.0, 50.0)),
            ]
        );
        assert!(none.heads.is_empty());
        // A route shorter than two heads shares its length between them.
        let short = Routed {
            points: vec![p(0.0, 0.0), p(8.0, 0.0)],
            sides: [None, None],
            outward: [Side::Right, Side::Left],
        };
        let drawn = short.draw([true, true]);
        assert_eq!(
            drawn.line,
            [Segment::MoveTo(p(4.0, 0.0)), Segment::LineTo(p(4.0, 0.0))]
        );
    }

    #[test]
    fn points_along_the_route() {
        let routed = Routed {
            points: vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0)],
            sides: [None, None],
            outward: [Side::Right, Side::Top],
        };
        assert_eq!(routed.length(), 200.0);
        for (at, want) in [
            (0, p(0.0, 0.0)),
            (250, p(50.0, 0.0)),
            (500, p(100.0, 0.0)),
            (750, p(100.0, 50.0)),
            (1000, p(100.0, 100.0)),
            (-1, p(0.0, 0.0)),
            (5000, p(100.0, 100.0)),
        ] {
            assert_eq!(routed.point_at(at), want, "{at}");
        }
    }

    #[test]
    fn simplify_keeps_turns_and_drops_straight_runs() {
        assert_eq!(
            simplify(vec![
                p(0.0, 0.0),
                p(0.0, 0.0),
                p(5.0, 0.0),
                p(10.0, 0.0),
                p(10.0, 5.0)
            ]),
            [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 5.0)]
        );
        // Turning back on itself is a turn.
        assert_eq!(
            simplify(vec![p(0.0, 0.0), p(10.0, 0.0), p(5.0, 0.0)]),
            [p(0.0, 0.0), p(10.0, 0.0), p(5.0, 0.0)]
        );
        assert_eq!(
            simplify(vec![p(1.0, 1.0), p(1.0, 1.0)]),
            [p(1.0, 1.0), p(1.0, 1.0)]
        );
    }

    #[test]
    fn stroke_constants() {
        assert_eq!(STROKE_WIDTH, 1.5);
        assert_eq!(DASH, [6.0, 4.0]);
        assert_eq!((ARROW_LENGTH, ARROW_WIDTH), (10.0, 8.0));
    }
}
