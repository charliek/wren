//! The eight shapes of `kind = "shape"`: their outlines, the boxes their
//! labels wrap in, and the points where a connector meets them.
//!
//! Every function takes the node's box in absolute board units. A box with
//! a negative or NaN size is drawn as an empty one, so nothing here
//! produces a NaN. The constants are the final values of §3.7 of the plan.

use crate::scene::{Point, Rect, Segment};
use crate::{Named, Shape, Side};

/// The corner radius of `rounded`, reduced to half the shorter side of a
/// smaller box.
pub const ROUNDED_RADIUS: f32 = 8.0;
/// The largest half-height of a cylinder's elliptical cap; it is otherwise
/// a sixth of the height.
pub const CYLINDER_CAP: f32 = 12.0;
/// The largest amplitude of a document's wavy bottom edge; it is otherwise
/// an eighth of the height. The wave spans twice the amplitude.
pub const DOCUMENT_WAVE: f32 = 8.0;
/// The label inset from the sides of most shapes.
pub const LABEL_SIDE: f32 = 8.0;
/// The label inset from the top and bottom of most shapes.
pub const LABEL_END: f32 = 4.0;
/// The label inset of an ellipse on each side, as a fraction of its width
/// and height. The box's corners then lie just inside the ellipse.
pub const ELLIPSE_INSET: f32 = 0.15;

/// The control-point distance that makes a cubic Bézier a quarter ellipse,
/// as a fraction of the radius.
const KAPPA: f32 = 0.552_284_8;

/// The shape a node is drawn with: an unknown value draws as `rect`.
pub fn drawn(shape: &Named<Shape>) -> Shape {
    shape.known().unwrap_or(Shape::Rect)
}

/// The closed outline that is filled and stroked.
pub fn outline(shape: Shape, bounds: Rect) -> Vec<Segment> {
    let r = normal(bounds);
    let (x, y, w, h) = (r.x, r.y, r.width, r.height);
    let (right, bottom, c) = (r.right(), r.bottom(), r.center());
    let mut path = Builder::default();
    match shape {
        Shape::Rect => {
            path.polygon(&[
                Point::new(x, y),
                Point::new(right, y),
                Point::new(right, bottom),
                Point::new(x, bottom),
            ]);
        }
        Shape::Rounded => path.rounded(r, ROUNDED_RADIUS.min(w / 2.0).min(h / 2.0)),
        Shape::Pill => path.rounded(r, w.min(h) / 2.0),
        Shape::Ellipse => {
            path.move_to(Point::new(c.x, y));
            path.quarter(Point::new(right, y), Point::new(right, c.y));
            path.quarter(Point::new(right, bottom), Point::new(c.x, bottom));
            path.quarter(Point::new(x, bottom), Point::new(x, c.y));
            path.quarter(Point::new(x, y), Point::new(c.x, y));
            path.close();
        }
        Shape::Diamond => {
            path.polygon(&[
                Point::new(c.x, y),
                Point::new(right, c.y),
                Point::new(c.x, bottom),
                Point::new(x, c.y),
            ]);
        }
        Shape::Parallelogram => {
            let k = skew(w, h);
            path.polygon(&[
                Point::new(x + k, y),
                Point::new(right, y),
                Point::new(right - k, bottom),
                Point::new(x, bottom),
            ]);
        }
        Shape::Cylinder => {
            let cap = cylinder_cap(h);
            path.move_to(Point::new(x, y + cap));
            path.quarter(Point::new(x, y), Point::new(c.x, y));
            path.quarter(Point::new(right, y), Point::new(right, y + cap));
            path.line_to(Point::new(right, bottom - cap));
            path.quarter(Point::new(right, bottom), Point::new(c.x, bottom));
            path.quarter(Point::new(x, bottom), Point::new(x, bottom - cap));
            path.close();
        }
        Shape::Document => {
            let a = document_wave(h);
            let base = bottom - a;
            path.polygon_open(&[
                Point::new(x, y),
                Point::new(right, y),
                Point::new(right, base),
            ]);
            path.quad(
                Point::new(x + w * 0.75, base - 2.0 * a),
                Point::new(c.x, base),
            );
            path.quad(
                Point::new(x + w * 0.25, base + 2.0 * a),
                Point::new(x, base),
            );
            path.close();
        }
    }
    path.segments
}

/// Lines stroked over the outline and never filled: the front rim of a
/// cylinder's top cap. Empty for every other shape.
pub fn detail(shape: Shape, bounds: Rect) -> Vec<Segment> {
    let r = normal(bounds);
    let mut path = Builder::default();
    if shape == Shape::Cylinder {
        let cap = cylinder_cap(r.height);
        let (rim, low) = (r.y + cap, r.y + 2.0 * cap);
        path.move_to(Point::new(r.x, rim));
        path.quarter(Point::new(r.x, low), Point::new(r.center().x, low));
        path.quarter(Point::new(r.right(), low), Point::new(r.right(), rim));
    }
    path.segments
}

/// The box a label wraps in and is centred in, inset from the node's box.
/// When the insets meet, it is an empty box at the centre.
pub fn label_box(shape: Shape, bounds: Rect) -> Rect {
    let r = normal(bounds);
    let (w, h) = (r.width, r.height);
    // Insets from the left and right, then from the top and the bottom.
    let (side, top, bottom) = match shape {
        Shape::Rect | Shape::Rounded => (LABEL_SIDE, LABEL_END, LABEL_END),
        Shape::Pill => (h / 4.0, LABEL_END, LABEL_END),
        Shape::Ellipse => (w * ELLIPSE_INSET, h * ELLIPSE_INSET, h * ELLIPSE_INSET),
        Shape::Diamond => (w / 6.0, h / 6.0, h / 6.0),
        Shape::Parallelogram => (skew(w, h) + LABEL_END, LABEL_END, LABEL_END),
        Shape::Cylinder => (LABEL_SIDE, 2.0 * cylinder_cap(h), LABEL_END),
        Shape::Document => (LABEL_SIDE, LABEL_END, 2.0 * document_wave(h)),
    };
    let width = w - 2.0 * side;
    let height = h - top - bottom;
    let c = r.center();
    let (x, width) = if width > 0.0 {
        (r.x + side, width)
    } else {
        (c.x, 0.0)
    };
    let (y, height) = if height > 0.0 {
        (r.y + top, height)
    } else {
        (c.y, 0.0)
    };
    Rect::new(x, y, width, height)
}

/// The middle of a side, on the outline: where an end pinned to that side,
/// or floating on it, meets the shape. It is the middle of the box's side
/// for every shape but the slanted sides of a parallelogram and the wavy
/// bottom of a document.
pub fn side_point(shape: Shape, bounds: Rect, side: Side) -> Point {
    let r = normal(bounds);
    let c = r.center();
    match (shape, side) {
        (_, Side::Top) => Point::new(c.x, r.y),
        (Shape::Parallelogram, Side::Right) => {
            Point::new(r.right() - skew(r.width, r.height) / 2.0, c.y)
        }
        (_, Side::Right) => Point::new(r.right(), c.y),
        (Shape::Document, Side::Bottom) => Point::new(c.x, r.bottom() - document_wave(r.height)),
        (_, Side::Bottom) => Point::new(c.x, r.bottom()),
        (Shape::Parallelogram, Side::Left) => Point::new(r.x + skew(r.width, r.height) / 2.0, c.y),
        (_, Side::Left) => Point::new(r.x, c.y),
    }
}

/// An `anchor`: a point of the node's box, per-mille of its size from the
/// top-left corner, each part clamped to 0 to 1000.
pub fn anchor_point(bounds: Rect, anchor: [i32; 2]) -> Point {
    let r = normal(bounds);
    let [fx, fy] = anchor.map(|v| v.clamp(0, 1000) as f32 / 1000.0);
    Point::new(r.x + r.width * fx, r.y + r.height * fy)
}

/// How far a parallelogram's top edge is shifted right of its bottom edge.
fn skew(w: f32, h: f32) -> f32 {
    (w / 4.0).min(h / 3.0)
}

fn cylinder_cap(h: f32) -> f32 {
    (h / 6.0).min(CYLINDER_CAP)
}

fn document_wave(h: f32) -> f32 {
    (h / 8.0).min(DOCUMENT_WAVE)
}

/// The box with a negative or NaN size taken as empty.
fn normal(r: Rect) -> Rect {
    Rect::new(r.x, r.y, r.width.max(0.0), r.height.max(0.0))
}

/// A path under construction that drops lines of zero length, so a
/// degenerate rounded corner or pill adds nothing.
#[derive(Default)]
struct Builder {
    segments: Vec<Segment>,
    current: Point,
}

impl Builder {
    fn move_to(&mut self, to: Point) {
        self.segments.push(Segment::MoveTo(to));
        self.current = to;
    }

    fn line_to(&mut self, to: Point) {
        if to != self.current {
            self.segments.push(Segment::LineTo(to));
            self.current = to;
        }
    }

    fn quad(&mut self, control: Point, to: Point) {
        self.segments.push(Segment::QuadTo { control, to });
        self.current = to;
    }

    /// A quarter ellipse from the current point to `to`, inside the
    /// corner of their bounding box at `corner`.
    fn quarter(&mut self, corner: Point, to: Point) {
        if to == self.current {
            return;
        }
        let from = self.current;
        let toward = |p: Point| {
            Point::new(
                p.x + (corner.x - p.x) * KAPPA,
                p.y + (corner.y - p.y) * KAPPA,
            )
        };
        self.segments.push(Segment::CubicTo {
            first: toward(from),
            second: toward(to),
            to,
        });
        self.current = to;
    }

    fn close(&mut self) {
        self.segments.push(Segment::Close);
    }

    fn polygon_open(&mut self, points: &[Point]) {
        let mut points = points.iter();
        if let Some(&first) = points.next() {
            self.move_to(first);
        }
        for &point in points {
            self.line_to(point);
        }
    }

    fn polygon(&mut self, points: &[Point]) {
        self.polygon_open(points);
        self.close();
    }

    /// A rectangle with quarter-circle corners of radius `radius`, which
    /// is at most half of either side.
    fn rounded(&mut self, r: Rect, radius: f32) {
        let (x, y, right, bottom) = (r.x, r.y, r.right(), r.bottom());
        self.move_to(Point::new(x + radius, y));
        self.line_to(Point::new(right - radius, y));
        self.quarter(Point::new(right, y), Point::new(right, y + radius));
        self.line_to(Point::new(right, bottom - radius));
        self.quarter(
            Point::new(right, bottom),
            Point::new(right - radius, bottom),
        );
        self.line_to(Point::new(x + radius, bottom));
        self.quarter(Point::new(x, bottom), Point::new(x, bottom - radius));
        self.line_to(Point::new(x, y + radius));
        self.quarter(Point::new(x, y), Point::new(x + radius, y));
        self.close();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Vocabulary;

    /// A path as polylines, one per subpath, each curve cut into 64 lines.
    pub(crate) fn flatten(segments: &[Segment]) -> Vec<Vec<Point>> {
        const STEPS: usize = 64;
        let mut lines: Vec<Vec<Point>> = Vec::new();
        let mut current = Point::default();
        let mut start = Point::default();
        for &segment in segments {
            match segment {
                Segment::MoveTo(p) => {
                    lines.push(vec![p]);
                    current = p;
                    start = p;
                }
                Segment::LineTo(p) => {
                    lines.last_mut().unwrap().push(p);
                    current = p;
                }
                Segment::QuadTo { control, to } => {
                    for i in 1..=STEPS {
                        let t = i as f32 / STEPS as f32;
                        let u = 1.0 - t;
                        let at = |a: f32, b: f32, c: f32| u * u * a + 2.0 * u * t * b + t * t * c;
                        lines.last_mut().unwrap().push(Point::new(
                            at(current.x, control.x, to.x),
                            at(current.y, control.y, to.y),
                        ));
                    }
                    current = to;
                }
                Segment::CubicTo { first, second, to } => {
                    for i in 1..=STEPS {
                        let t = i as f32 / STEPS as f32;
                        let u = 1.0 - t;
                        let at = |a: f32, b: f32, c: f32, d: f32| {
                            u * u * u * a
                                + 3.0 * u * u * t * b
                                + 3.0 * u * t * t * c
                                + t * t * t * d
                        };
                        lines.last_mut().unwrap().push(Point::new(
                            at(current.x, first.x, second.x, to.x),
                            at(current.y, first.y, second.y, to.y),
                        ));
                    }
                    current = to;
                }
                Segment::Close => {
                    lines.last_mut().unwrap().push(start);
                    current = start;
                }
            }
        }
        lines
    }

    /// The distance from `p` to the nearest point of the path.
    pub(crate) fn distance(segments: &[Segment], p: Point) -> f32 {
        let mut best = f32::INFINITY;
        for line in flatten(segments) {
            for pair in line.windows(2) {
                best = best.min(to_segment(p, pair[0], pair[1]));
            }
            if let [only] = line.as_slice() {
                best = best.min(((p.x - only.x).powi(2) + (p.y - only.y).powi(2)).sqrt());
            }
        }
        best
    }

    fn to_segment(p: Point, a: Point, b: Point) -> f32 {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length = dx * dx + dy * dy;
        let t = if length == 0.0 {
            0.0
        } else {
            (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0.0, 1.0)
        };
        let (qx, qy) = (a.x + t * dx, a.y + t * dy);
        ((p.x - qx).powi(2) + (p.y - qy).powi(2)).sqrt()
    }

    fn points(segments: &[Segment]) -> Vec<Point> {
        segments
            .iter()
            .flat_map(|segment| match *segment {
                Segment::MoveTo(p) | Segment::LineTo(p) => vec![p],
                Segment::QuadTo { control, to } => vec![control, to],
                Segment::CubicTo { first, second, to } => vec![first, second, to],
                Segment::Close => vec![],
            })
            .collect()
    }

    const BOXES: [Rect; 6] = [
        Rect::new(0.0, 0.0, 120.0, 60.0),
        Rect::new(-37.0, 210.5, 40.0, 160.0),
        Rect::new(100.0, 100.0, 12.0, 6.0),
        Rect::new(5.0, 5.0, 1.0, 1.0),
        Rect::new(5.0, 5.0, 0.0, 0.0),
        Rect::new(3.0, 4.0, 300.0, 300.0),
    ];

    #[test]
    fn every_outline_is_closed_finite_and_inside_its_box() {
        for &shape in Shape::ALL {
            for r in BOXES {
                let path = outline(shape, r);
                assert!(
                    matches!(path.first(), Some(Segment::MoveTo(_))),
                    "{shape:?}"
                );
                assert_eq!(path.last(), Some(&Segment::Close), "{shape:?} {r:?}");
                for p in points(&path).into_iter().chain(points(&detail(shape, r))) {
                    assert!(p.x.is_finite() && p.y.is_finite(), "{shape:?} {r:?}");
                }
                for line in flatten(&path) {
                    for p in line {
                        let slack = 1e-3;
                        assert!(
                            p.x >= r.x - slack
                                && p.x <= r.right() + slack
                                && p.y >= r.y - slack
                                && p.y <= r.bottom() + slack,
                            "{shape:?} {r:?}: {p:?} outside the box"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn outlines_touch_every_side_of_their_box() {
        // Each shape fills its box: the flattened outline reaches all four
        // sides, so a selection box drawn around the node fits it.
        let r = Rect::new(10.0, 20.0, 120.0, 60.0);
        for &shape in Shape::ALL {
            let line: Vec<Point> = flatten(&outline(shape, r)).concat();
            let min_x = line.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
            let max_x = line.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
            let min_y = line.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let max_y = line.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            for (got, want) in [(min_x, 10.0), (max_x, 130.0), (min_y, 20.0), (max_y, 80.0)] {
                assert!((got - want).abs() < 0.01, "{shape:?}: {got} != {want}");
            }
        }
    }

    #[test]
    fn rect_and_diamond_outlines_are_exact() {
        let r = Rect::new(0.0, 0.0, 120.0, 60.0);
        let p = Point::new;
        assert_eq!(
            outline(Shape::Rect, r),
            [
                Segment::MoveTo(p(0.0, 0.0)),
                Segment::LineTo(p(120.0, 0.0)),
                Segment::LineTo(p(120.0, 60.0)),
                Segment::LineTo(p(0.0, 60.0)),
                Segment::Close,
            ]
        );
        assert_eq!(
            outline(Shape::Diamond, r),
            [
                Segment::MoveTo(p(60.0, 0.0)),
                Segment::LineTo(p(120.0, 30.0)),
                Segment::LineTo(p(60.0, 60.0)),
                Segment::LineTo(p(0.0, 30.0)),
                Segment::Close,
            ]
        );
        // A skew of min(120 / 4, 60 / 3) = 20.
        assert_eq!(
            outline(Shape::Parallelogram, r),
            [
                Segment::MoveTo(p(20.0, 0.0)),
                Segment::LineTo(p(120.0, 0.0)),
                Segment::LineTo(p(100.0, 60.0)),
                Segment::LineTo(p(0.0, 60.0)),
                Segment::Close,
            ]
        );
    }

    #[test]
    fn shape_parameters_follow_the_box() {
        let r = Rect::new(0.0, 0.0, 120.0, 60.0);
        // A pill's ends are semicircles: radius h / 2, so no straight side.
        let pill = outline(Shape::Pill, r);
        assert_eq!(pill[0], Segment::MoveTo(Point::new(30.0, 0.0)));
        assert_eq!(pill[1], Segment::LineTo(Point::new(90.0, 0.0)));
        assert!(!pill.contains(&Segment::LineTo(Point::new(120.0, 30.0))));
        // A tall pill rounds by half its width instead.
        let tall = outline(Shape::Pill, Rect::new(0.0, 0.0, 20.0, 100.0));
        assert_eq!(tall[0], Segment::MoveTo(Point::new(10.0, 0.0)));
        // The rounded radius shrinks in a small box.
        let small = outline(Shape::Rounded, Rect::new(0.0, 0.0, 10.0, 6.0));
        assert_eq!(small[0], Segment::MoveTo(Point::new(3.0, 0.0)));
        assert_eq!(
            outline(Shape::Rounded, r)[0],
            Segment::MoveTo(Point::new(8.0, 0.0))
        );
        // Cylinder caps: h / 6 = 10 here, capped at 12 when taller.
        assert_eq!(
            outline(Shape::Cylinder, r)[0],
            Segment::MoveTo(Point::new(0.0, 10.0))
        );
        assert_eq!(
            outline(Shape::Cylinder, Rect::new(0.0, 0.0, 120.0, 300.0))[0],
            Segment::MoveTo(Point::new(0.0, 12.0))
        );
        // The cylinder's rim is drawn, open, from side to side through the
        // bottom of the cap.
        let rim = detail(Shape::Cylinder, r);
        assert_eq!(rim[0], Segment::MoveTo(Point::new(0.0, 10.0)));
        assert!(!rim.contains(&Segment::Close));
        assert!(distance(&rim, Point::new(60.0, 20.0)) < 1e-3);
        for &shape in Shape::ALL {
            if shape != Shape::Cylinder {
                assert!(detail(shape, r).is_empty(), "{shape:?}");
            }
        }
        // The document's wave: amplitude h / 8 = 7.5, so the sides stop at
        // 52.5 and the wave dips to the bottom of the box.
        let document = outline(Shape::Document, r);
        assert_eq!(document[2], Segment::LineTo(Point::new(120.0, 52.5)));
        assert!(distance(&document, Point::new(30.0, 60.0)) < 1e-3);
        assert!(distance(&document, Point::new(90.0, 45.0)) < 1e-3);
    }

    #[test]
    fn label_boxes_by_shape() {
        let r = Rect::new(0.0, 0.0, 120.0, 60.0);
        let cases = [
            (Shape::Rect, Rect::new(8.0, 4.0, 104.0, 52.0)),
            (Shape::Rounded, Rect::new(8.0, 4.0, 104.0, 52.0)),
            (Shape::Pill, Rect::new(15.0, 4.0, 90.0, 52.0)),
            (Shape::Ellipse, Rect::new(18.0, 9.0, 84.0, 42.0)),
            (Shape::Diamond, Rect::new(20.0, 10.0, 80.0, 40.0)),
            (Shape::Parallelogram, Rect::new(24.0, 4.0, 72.0, 52.0)),
            (Shape::Cylinder, Rect::new(8.0, 20.0, 104.0, 36.0)),
            (Shape::Document, Rect::new(8.0, 4.0, 104.0, 41.0)),
        ];
        for (shape, want) in cases {
            let got = label_box(shape, r);
            for (a, b) in [
                (got.x, want.x),
                (got.y, want.y),
                (got.width, want.width),
                (got.height, want.height),
            ] {
                assert!((a - b).abs() < 1e-4, "{shape:?}: {got:?} != {want:?}");
            }
        }
        // Moving the box moves the label box with it.
        let moved = label_box(Shape::Rect, Rect::new(-100.0, 40.0, 120.0, 60.0));
        assert_eq!(moved, Rect::new(-92.0, 44.0, 104.0, 52.0));
    }

    #[test]
    fn label_boxes_never_turn_inside_out() {
        for &shape in Shape::ALL {
            for r in BOXES.into_iter().chain([
                Rect::new(0.0, 0.0, -5.0, -5.0),
                Rect::new(0.0, 0.0, f32::NAN, 8.0),
            ]) {
                let b = label_box(shape, r);
                assert!(b.width >= 0.0 && b.height >= 0.0, "{shape:?} {r:?}: {b:?}");
                assert!(b.x.is_finite() && b.y.is_finite(), "{shape:?} {r:?}: {b:?}");
                if r.width >= 0.0 && r.height >= 0.0 {
                    assert!(
                        b.x >= r.x
                            && b.right() <= r.right()
                            && b.y >= r.y
                            && b.bottom() <= r.bottom(),
                        "{shape:?} {r:?}: {b:?} leaves the node"
                    );
                }
            }
        }
        // Insets that meet leave an empty box at the centre.
        assert_eq!(
            label_box(Shape::Rect, Rect::new(5.0, 5.0, 2.0, 2.0)),
            Rect::new(6.0, 6.0, 0.0, 0.0)
        );
        assert_eq!(
            label_box(Shape::Rect, Rect::new(0.0, 0.0, 100.0, 6.0)),
            Rect::new(8.0, 3.0, 84.0, 0.0)
        );
    }

    #[test]
    fn side_points_lie_on_every_outline() {
        for &shape in Shape::ALL {
            for r in BOXES {
                let path = outline(shape, r);
                for &side in Side::ALL {
                    let p = side_point(shape, r, side);
                    let off = distance(&path, p);
                    assert!(
                        off < 1e-3,
                        "{shape:?} {side:?} {r:?}: {off} from the outline"
                    );
                }
            }
        }
    }

    #[test]
    fn side_points_are_side_middles() {
        let r = Rect::new(0.0, 0.0, 120.0, 60.0);
        let p = Point::new;
        for &shape in Shape::ALL {
            let at = |side| side_point(shape, r, side);
            assert_eq!(at(Side::Top), p(60.0, 0.0), "{shape:?}");
            let (left, right, bottom) = match shape {
                Shape::Parallelogram => (p(10.0, 30.0), p(110.0, 30.0), p(60.0, 60.0)),
                Shape::Document => (p(0.0, 30.0), p(120.0, 30.0), p(60.0, 52.5)),
                _ => (p(0.0, 30.0), p(120.0, 30.0), p(60.0, 60.0)),
            };
            assert_eq!(at(Side::Left), left, "{shape:?}");
            assert_eq!(at(Side::Right), right, "{shape:?}");
            assert_eq!(at(Side::Bottom), bottom, "{shape:?}");
        }
    }

    #[test]
    fn anchors_are_per_mille_of_the_box_and_clamped() {
        let r = Rect::new(100.0, 200.0, 120.0, 60.0);
        let p = Point::new;
        let cases = [
            ([0, 0], p(100.0, 200.0)),
            ([500, 0], p(160.0, 200.0)),
            ([1000, 500], p(220.0, 230.0)),
            ([500, 1000], p(160.0, 260.0)),
            ([0, 500], p(100.0, 230.0)),
            ([250, 750], p(130.0, 245.0)),
            ([-5, 2000], p(100.0, 260.0)),
            ([i32::MIN, i32::MAX], p(100.0, 260.0)),
        ];
        for (anchor, want) in cases {
            assert_eq!(anchor_point(r, anchor), want, "{anchor:?}");
        }
        // The four side-centre anchors are the side points of a box.
        for (anchor, side) in [
            ([500, 0], Side::Top),
            ([1000, 500], Side::Right),
            ([500, 1000], Side::Bottom),
            ([0, 500], Side::Left),
        ] {
            assert_eq!(anchor_point(r, anchor), side_point(Shape::Rect, r, side));
        }
    }

    #[test]
    fn unknown_shapes_draw_as_rect() {
        assert_eq!(drawn(&Named::Other("hexagon".into())), Shape::Rect);
        assert_eq!(drawn(&Named::Known(Shape::Pill)), Shape::Pill);
    }
}
