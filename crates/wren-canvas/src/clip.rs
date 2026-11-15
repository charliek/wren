//! Paths in screen pixels cut down to a window around the widget.
//!
//! A board coordinate may be millions of units from the view, and the
//! software renderer's rasteriser panics on a path that many pixels
//! across. So a sub-path that reaches past the window is cut to it before
//! it is drawn: one wholly outside is dropped, a line is cut where it
//! crosses, and a curve is split until each piece is inside or outside.
//! The window is far wider than the widget, so its edges, where a cut
//! outline turns or ends, are never seen. A path inside it is left alone.
//!
//! A fill and a stroke are cut differently: a fill as an area, closed
//! along the window's edge; a stroke as a line, ended where it leaves. A
//! cut dashed stroke starts again on a whole dash period from the start
//! of its sub-path, so its dashes stay where they were.

/// A point in screen pixels.
pub(crate) type Pixel = (f64, f64);

/// One step of a path in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Piece {
    Move(Pixel),
    Line(Pixel),
    Quad(Pixel, Pixel),
    Cubic(Pixel, Pixel, Pixel),
    Close,
}

/// An axis-aligned rectangle in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Window {
    pub(crate) left: f64,
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
}

/// A curve piece narrower and shorter than this, in pixels, that still
/// crosses the window's edge is drawn as its chord: it is far out of view.
const TINY: f64 = 1.0;

/// Splitting a curve stops here: 2^-48 of any curve on a board is tiny.
const DEPTH: u32 = 48;

impl Window {
    fn holds(&self, point: Pixel) -> bool {
        self.left <= point.0
            && point.0 <= self.right
            && self.top <= point.1
            && point.1 <= self.bottom
    }

    fn contains(&self, bounds: &Window) -> bool {
        self.left <= bounds.left
            && bounds.right <= self.right
            && self.top <= bounds.top
            && bounds.bottom <= self.bottom
    }

    /// Whether `bounds` reaches into the window. No bounds with a NaN do.
    fn meets(&self, bounds: &Window) -> bool {
        bounds.left <= self.right
            && self.left <= bounds.right
            && bounds.top <= self.bottom
            && self.top <= bounds.bottom
    }

    /// The smallest rectangle holding `points`; all NaN if any of them is,
    /// which meets no window.
    fn around(points: impl IntoIterator<Item = Pixel>) -> Self {
        points.into_iter().fold(
            Self {
                left: f64::INFINITY,
                top: f64::INFINITY,
                right: f64::NEG_INFINITY,
                bottom: f64::NEG_INFINITY,
            },
            |bounds, (x, y)| {
                if x.is_nan() || y.is_nan() || bounds.left.is_nan() {
                    return Self {
                        left: f64::NAN,
                        top: f64::NAN,
                        right: f64::NAN,
                        bottom: f64::NAN,
                    };
                }
                Self {
                    left: bounds.left.min(x),
                    top: bounds.top.min(y),
                    right: bounds.right.max(x),
                    bottom: bounds.bottom.max(y),
                }
            },
        )
    }
}

/// Whether every point of `pieces`, control points too, is in `window`,
/// so the path needs no cutting. A NaN anywhere answers no.
pub(crate) fn inside(pieces: &[Piece], window: &Window) -> bool {
    pieces
        .iter()
        .all(|piece| points(piece).all(|point| window.holds(point)))
}

/// `pieces` cut for a fill: each sub-path closed and cut as an area.
pub(crate) fn area(pieces: &[Piece], window: &Window) -> Vec<Piece> {
    let mut out = Vec::new();
    for sub in subpaths(pieces) {
        match place(&sub, window) {
            Place::Out => {}
            Place::In => out.extend(sub.pieces()),
            Place::Across => {
                let mut edges = sub.edges(true);
                for side in SIDES {
                    edges = cut_area(&edges, window, side);
                }
                if let Some(last) = edges.last() {
                    out.push(Piece::Move(last.to()));
                    out.extend(edges.iter().map(Edge::piece));
                    out.push(Piece::Close);
                }
            }
        }
    }
    out
}

/// `pieces` cut for a stroke whose dash pattern repeats every `period`
/// pixels, if it is dashed.
pub(crate) fn outline(pieces: &[Piece], window: &Window, period: Option<f64>) -> Vec<Piece> {
    let mut out = Vec::new();
    for sub in subpaths(pieces) {
        match place(&sub, window) {
            Place::Out => {}
            Place::In => out.extend(sub.pieces()),
            Place::Across => cut_outline(&sub, window, period, &mut out),
        }
    }
    out
}

/// One sub-path: where it starts, its steps, and whether it is closed.
struct Subpath {
    start: Pixel,
    edges: Vec<Edge>,
    closed: bool,
}

/// A step of a sub-path, from where the step before it ended.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edge {
    Line(Pixel),
    Quad(Pixel, Pixel),
    Cubic(Pixel, Pixel, Pixel),
}

impl Edge {
    fn to(&self) -> Pixel {
        match *self {
            Edge::Line(to) | Edge::Quad(_, to) | Edge::Cubic(_, _, to) => to,
        }
    }

    fn piece(&self) -> Piece {
        match *self {
            Edge::Line(to) => Piece::Line(to),
            Edge::Quad(control, to) => Piece::Quad(control, to),
            Edge::Cubic(first, second, to) => Piece::Cubic(first, second, to),
        }
    }

    /// The edge's points from `from`, control points included: a curve
    /// lies inside their hull.
    fn hull(&self, from: Pixel) -> Window {
        match *self {
            Edge::Line(to) => Window::around([from, to]),
            Edge::Quad(control, to) => Window::around([from, control, to]),
            Edge::Cubic(first, second, to) => Window::around([from, first, second, to]),
        }
    }
}

impl Subpath {
    fn pieces(&self) -> impl Iterator<Item = Piece> + '_ {
        std::iter::once(Piece::Move(self.start))
            .chain(self.edges.iter().map(Edge::piece))
            .chain(self.closed.then_some(Piece::Close))
    }

    /// The sub-path's steps and, with `close`, a last line back to its
    /// start if it does not end there.
    fn edges(&self, close: bool) -> Vec<Edge> {
        let mut edges = self.edges.clone();
        if close && edges.last().is_none_or(|last| last.to() != self.start) {
            edges.push(Edge::Line(self.start));
        }
        edges
    }
}

fn points(piece: &Piece) -> impl Iterator<Item = Pixel> {
    let (points, count): ([Pixel; 3], usize) = match *piece {
        Piece::Move(to) | Piece::Line(to) => ([to, to, to], 1),
        Piece::Quad(control, to) => ([control, to, to], 2),
        Piece::Cubic(first, second, to) => ([first, second, to], 3),
        Piece::Close => ([(0.0, 0.0); 3], 0),
    };
    points.into_iter().take(count)
}

/// `pieces` as sub-paths. A step before any move starts from the origin,
/// as the renderers start one.
fn subpaths(pieces: &[Piece]) -> Vec<Subpath> {
    let mut subs: Vec<Subpath> = Vec::new();
    let mut open = false;
    for piece in pieces {
        let edge = match *piece {
            Piece::Move(to) => {
                subs.push(Subpath {
                    start: to,
                    edges: Vec::new(),
                    closed: false,
                });
                open = true;
                continue;
            }
            Piece::Close => {
                if let Some(sub) = subs.last_mut().filter(|_| open) {
                    sub.closed = true;
                    open = false;
                }
                continue;
            }
            Piece::Line(to) => Edge::Line(to),
            Piece::Quad(control, to) => Edge::Quad(control, to),
            Piece::Cubic(first, second, to) => Edge::Cubic(first, second, to),
        };
        if !open {
            // After a close the pen is back at the start of the last
            // sub-path; before any move, at the origin.
            let start = subs.last().map_or((0.0, 0.0), |sub| sub.start);
            subs.push(Subpath {
                start,
                edges: Vec::new(),
                closed: false,
            });
            open = true;
        }
        if let Some(sub) = subs.last_mut() {
            sub.edges.push(edge);
        }
    }
    subs
}

enum Place {
    In,
    Out,
    Across,
}

fn place(sub: &Subpath, window: &Window) -> Place {
    let bounds = Window::around(
        std::iter::once(sub.start).chain(sub.edges.iter().flat_map(|edge| points(&edge.piece()))),
    );
    if window.contains(&bounds) {
        Place::In
    } else if window.meets(&bounds) {
        Place::Across
    } else {
        Place::Out
    }
}

/// `edge` from `from` as edges each inside `window` or a line.
fn split(from: Pixel, edge: Edge, window: &Window, depth: u32, out: &mut Vec<Edge>) {
    let hull = edge.hull(from);
    let small = hull.right - hull.left < TINY && hull.bottom - hull.top < TINY;
    match edge {
        Edge::Line(_) => out.push(edge),
        _ if window.contains(&hull) => out.push(edge),
        _ if !window.meets(&hull) || small || depth == 0 => out.push(Edge::Line(edge.to())),
        Edge::Quad(control, to) => {
            let (a, b) = (mid(from, control), mid(control, to));
            let at = mid(a, b);
            split(from, Edge::Quad(a, at), window, depth - 1, out);
            split(at, Edge::Quad(b, to), window, depth - 1, out);
        }
        Edge::Cubic(first, second, to) => {
            let (a, b, c) = (mid(from, first), mid(first, second), mid(second, to));
            let (d, e) = (mid(a, b), mid(b, c));
            let at = mid(d, e);
            split(from, Edge::Cubic(a, d, at), window, depth - 1, out);
            split(at, Edge::Cubic(e, c, to), window, depth - 1, out);
        }
    }
}

fn mid(a: Pixel, b: Pixel) -> Pixel {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// The sub-path's edges with every curve inside or outside `window`.
fn straddled(start: Pixel, edges: &[Edge], window: &Window) -> Vec<Edge> {
    let mut out = Vec::with_capacity(edges.len());
    let mut from = start;
    for edge in edges {
        split(from, *edge, window, DEPTH, &mut out);
        from = edge.to();
    }
    out
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Top,
    Right,
    Bottom,
}

const SIDES: [Side; 4] = [Side::Left, Side::Top, Side::Right, Side::Bottom];

impl Side {
    fn keeps(self, window: &Window, point: Pixel) -> bool {
        match self {
            Side::Left => point.0 >= window.left,
            Side::Top => point.1 >= window.top,
            Side::Right => point.0 <= window.right,
            Side::Bottom => point.1 <= window.bottom,
        }
    }

    /// Where the line from `a` to `b` crosses this side's edge.
    fn cross(self, window: &Window, a: Pixel, b: Pixel) -> Pixel {
        let along = |edge: f64, a: f64, b: f64| (edge - a) / (b - a);
        match self {
            Side::Left | Side::Right => {
                let x = if matches!(self, Side::Left) {
                    window.left
                } else {
                    window.right
                };
                (x, a.1 + (b.1 - a.1) * along(x, a.0, b.0))
            }
            Side::Top | Side::Bottom => {
                let y = if matches!(self, Side::Top) {
                    window.top
                } else {
                    window.bottom
                };
                (a.0 + (b.0 - a.0) * along(y, a.1, b.1), y)
            }
        }
    }
}

/// One pass of Sutherland–Hodgman: the closed loop of `edges` (it starts
/// where its last edge ends) cut to one side of the window. A curve is
/// only ever wholly inside, so only lines are cut.
fn cut_area(edges: &[Edge], window: &Window, side: Side) -> Vec<Edge> {
    let Some(last) = edges.last() else {
        return Vec::new();
    };
    // The curves were split against the whole window, so each is inside
    // every side; split each pass's input again all the same, since the
    // passes before may have left a line where a curve was.
    let edges = straddled(last.to(), edges, window);
    let mut out = Vec::with_capacity(edges.len() + 2);
    let mut from = last.to();
    for edge in &edges {
        let to = edge.to();
        match (side.keeps(window, from), side.keeps(window, to)) {
            (true, true) => out.push(*edge),
            (true, false) => out.push(Edge::Line(side.cross(window, from, to))),
            (false, true) => {
                out.push(Edge::Line(side.cross(window, from, to)));
                out.push(*edge);
            }
            (false, false) => {}
        }
        from = to;
    }
    out
}

/// The sub-path cut as a stroke into `out`: each run inside the window
/// its own sub-path, started back on a whole dash period.
fn cut_outline(sub: &Subpath, window: &Window, period: Option<f64>, out: &mut Vec<Piece>) {
    let edges = straddled(sub.start, &sub.edges(sub.closed), window);
    let period = period.filter(|period| period.is_finite() && *period > 0.0);
    let mut from = sub.start;
    // Whether the last piece out ends where this edge starts.
    let mut joined = false;
    let mut walked = 0.0;
    for edge in &edges {
        let to = edge.to();
        match edge {
            Edge::Line(_) => {
                let length = (to.0 - from.0).hypot(to.1 - from.1);
                if let Some((enter, leave)) = liang_barsky(from, to, window) {
                    let at = |t: f64| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
                    if !(joined && enter == 0.0) {
                        let back = period.map_or(0.0, |period| (walked + enter * length) % period);
                        let start = if length > 0.0 {
                            at(enter - back / length)
                        } else {
                            from
                        };
                        out.push(Piece::Move(start));
                    }
                    out.push(Piece::Line(at(leave)));
                    joined = leave == 1.0;
                } else {
                    joined = false;
                }
                walked += length;
            }
            Edge::Quad(..) | Edge::Cubic(..) => {
                // Split against the window: wholly inside it.
                if !joined {
                    let back = period.map_or(0.0, |period| walked % period);
                    out.push(Piece::Move(behind(from, edge, back)));
                    if back > 0.0 {
                        out.push(Piece::Line(from));
                    }
                }
                out.push(edge.piece());
                joined = true;
                walked += curve_length(from, edge);
            }
        }
        from = to;
    }
}

/// The point `back` pixels before `from` along the tangent `edge` leaves
/// it by, so a dash restarted there is in phase at `from`.
fn behind(from: Pixel, edge: &Edge, back: f64) -> Pixel {
    let toward = match *edge {
        Edge::Line(to) => to,
        Edge::Quad(control, to) => {
            if control == from {
                to
            } else {
                control
            }
        }
        Edge::Cubic(first, second, to) => [first, second, to]
            .into_iter()
            .find(|point| *point != from)
            .unwrap_or(to),
    };
    let (dx, dy) = (toward.0 - from.0, toward.1 - from.1);
    let length = dx.hypot(dy);
    if back == 0.0 || length == 0.0 {
        from
    } else {
        (from.0 - dx / length * back, from.1 - dy / length * back)
    }
}

/// A curve's length, near enough for a dash's phase: halfway between its
/// chord and its control polygon.
fn curve_length(from: Pixel, edge: &Edge) -> f64 {
    let distance = |a: Pixel, b: Pixel| (b.0 - a.0).hypot(b.1 - a.1);
    let (polygon, to) = match *edge {
        Edge::Line(to) => (distance(from, to), to),
        Edge::Quad(control, to) => (distance(from, control) + distance(control, to), to),
        Edge::Cubic(first, second, to) => (
            distance(from, first) + distance(first, second) + distance(second, to),
            to,
        ),
    };
    (polygon + distance(from, to)) / 2.0
}

/// The share of the line from `a` to `b` inside `window`, as the
/// parameters it enters and leaves at, if any of it is.
fn liang_barsky(a: Pixel, b: Pixel, window: &Window) -> Option<(f64, f64)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut enter, mut leave) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-dx, a.0 - window.left),
        (dx, window.right - a.0),
        (-dy, a.1 - window.top),
        (dy, window.bottom - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                enter = enter.max(t);
            } else {
                leave = leave.min(t);
            }
        }
    }
    (enter <= leave && !enter.is_nan() && !leave.is_nan()).then_some((enter, leave))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Window = Window {
        left: -100.0,
        top: -100.0,
        right: 900.0,
        bottom: 700.0,
    };

    fn all_points(pieces: &[Piece]) -> Vec<Pixel> {
        pieces.iter().flat_map(points).collect()
    }

    fn within(pieces: &[Piece], window: &Window, slack: f64) {
        for (x, y) in all_points(pieces) {
            assert!(
                window.left - slack <= x
                    && x <= window.right + slack
                    && window.top - slack <= y
                    && y <= window.bottom + slack,
                "({x}, {y}) in {pieces:?}"
            );
        }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Vec<Piece> {
        vec![
            Piece::Move((x, y)),
            Piece::Line((x + w, y)),
            Piece::Line((x + w, y + h)),
            Piece::Line((x, y + h)),
            Piece::Close,
        ]
    }

    /// Twice the signed area of a closed polygon of lines.
    fn area_of(pieces: &[Piece]) -> f64 {
        let mut total = 0.0;
        let mut start = (0.0, 0.0);
        let mut from = (0.0, 0.0);
        for piece in pieces {
            match *piece {
                Piece::Move(to) => {
                    start = to;
                    from = to;
                }
                Piece::Line(to) => {
                    total += from.0 * to.1 - to.0 * from.1;
                    from = to;
                }
                Piece::Close => {
                    total += from.0 * start.1 - start.0 * from.1;
                    from = start;
                }
                _ => panic!("lines only"),
            }
        }
        total.abs() / 2.0
    }

    #[test]
    fn a_path_inside_the_window_is_left_alone() {
        let path = rect(10.0, 10.0, 120.0, 60.0);
        assert!(inside(&path, &WINDOW));
        assert_eq!(area(&path, &WINDOW), path);
        assert_eq!(outline(&path, &WINDOW, Some(10.0)), path);
        let mut nan = path.clone();
        nan[1] = Piece::Line((f64::NAN, 10.0));
        assert!(!inside(&nan, &WINDOW));
        // A sub-path no number describes is never drawn.
        assert_eq!(area(&nan, &WINDOW), []);
        assert_eq!(outline(&nan, &WINDOW, None), []);
    }

    #[test]
    fn a_sub_path_wholly_outside_is_dropped_and_the_rest_kept() {
        // The two heads of a connector, one in view and one 24 million
        // pixels away.
        let near = [
            Piece::Move((100.0, 100.0)),
            Piece::Line((110.0, 95.0)),
            Piece::Line((110.0, 105.0)),
            Piece::Close,
        ];
        let far = [
            Piece::Move((-24e6, 24e6)),
            Piece::Line((-24e6 + 10.0, 24e6 - 5.0)),
            Piece::Line((-24e6 + 10.0, 24e6 + 5.0)),
            Piece::Close,
        ];
        let both: Vec<Piece> = near.iter().chain(&far).copied().collect();
        assert!(!inside(&both, &WINDOW));
        assert_eq!(area(&both, &WINDOW), near);
        assert_eq!(outline(&both, &WINDOW, None), near);
        assert_eq!(area(&far, &WINDOW), []);
    }

    #[test]
    fn a_long_line_is_cut_where_it_crosses_the_window() {
        let line = [
            Piece::Move((400.0, 300.0)),
            Piece::Line((400.0 - 16e6, 300.0 + 16e6)),
        ];
        let cut = outline(&line, &WINDOW, None);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0], Piece::Move((400.0, 300.0)));
        let Piece::Line((x, y)) = cut[1] else {
            panic!("{cut:?}")
        };
        // It leaves through the bottom edge, still on its own line.
        assert!((y - WINDOW.bottom).abs() < 1e-6, "{y}");
        assert!((x - (400.0 - (y - 300.0))).abs() < 1e-6, "{x}");
        within(&cut, &WINDOW, 0.0);

        // Across the whole window and out the other side.
        let far = f64::from(1u32 << 22);
        let through = [Piece::Move((-far, 300.0)), Piece::Line((far, 300.0))];
        assert_eq!(
            outline(&through, &WINDOW, None),
            [Piece::Move((-100.0, 300.0)), Piece::Line((900.0, 300.0))]
        );
        // Missing it altogether, though its bounds meet the window.
        let past = [
            Piece::Move((-5e6, -5e6 + 2000.0)),
            Piece::Line((5e6, 5e6 + 2000.0)),
        ];
        assert_eq!(outline(&past, &WINDOW, None), []);
    }

    #[test]
    fn a_cut_dash_starts_on_a_whole_period_from_the_sub_path_start() {
        let (start, period) = (-1e6 - 3.0, 10.0);
        let line = [Piece::Move((start, 300.0)), Piece::Line((1e6, 300.0))];
        let cut = outline(&line, &WINDOW, Some(period));
        let Piece::Move((x, _)) = cut[0] else {
            panic!("{cut:?}")
        };
        assert!(x <= WINDOW.left && x > WINDOW.left - period, "{x}");
        let phase = (x - start) % period;
        assert!(
            phase.abs() < 1e-6 || (period - phase).abs() < 1e-6,
            "{phase}"
        );
        let Piece::Line((x, y)) = cut[1] else {
            panic!("{cut:?}")
        };
        assert!((x - WINDOW.right).abs() < 1e-6 && y == 300.0, "{cut:?}");

        // Out and back in: a second run starts in phase too.
        let zigzag = [
            Piece::Move((400.0, 300.0)),
            Piece::Line((400.0, -1e6)),
            Piece::Line((403.0, -1e6)),
            Piece::Line((403.0, 300.0)),
        ];
        let cut = outline(&zigzag, &WINDOW, Some(period));
        let moves: Vec<Pixel> = cut
            .iter()
            .filter_map(|piece| match piece {
                Piece::Move(at) => Some(*at),
                _ => None,
            })
            .collect();
        assert_eq!(moves.len(), 2, "{cut:?}");
        let walked = (1e6 + 300.0) + 3.0 + (1e6 + moves[1].1);
        assert!((walked % period).abs() < 1e-6, "{walked}");
    }

    #[test]
    fn an_area_reaching_out_is_closed_along_the_window() {
        // A shape 2^25 pixels on a side, its corner in view.
        let side = f64::from(1u32 << 25);
        let huge = rect(400.0, 300.0, side, side);
        let cut = area(&huge, &WINDOW);
        within(&cut, &WINDOW, 0.0);
        assert!((area_of(&cut) - 500.0 * 400.0).abs() < 1e-3, "{cut:?}");

        // Covering the whole window: the window itself.
        let cover = rect(-16e6, -16e6, 32e6, 32e6);
        let cut = area(&cover, &WINDOW);
        within(&cut, &WINDOW, 0.0);
        assert!((area_of(&cut) - 1000.0 * 800.0).abs() < 1e-3, "{cut:?}");

        // Its stroke is the two sides in the window, each its own run.
        let cut = outline(&huge, &WINDOW, None);
        within(&cut, &WINDOW, 0.0);
        assert_eq!(
            cut,
            [
                Piece::Move((400.0, 300.0)),
                Piece::Line((900.0, 300.0)),
                Piece::Move((400.0, 700.0)),
                Piece::Line((400.0, 300.0)),
            ]
        );
    }

    #[test]
    fn an_open_sub_path_is_filled_as_if_closed() {
        let open = [
            Piece::Move((400.0, 300.0)),
            Piece::Line((400.0 + 1e7, 300.0)),
            Piece::Line((400.0, 300.0 + 1e7)),
        ];
        let cut = area(&open, &WINDOW);
        within(&cut, &WINDOW, 0.0);
        // The triangle's corner in view: 500 by 400, less the corner
        // beyond its hypotenuse, which is far out.
        assert!((area_of(&cut) - 500.0 * 400.0).abs() < 1e-3, "{cut:?}");
    }

    #[test]
    fn a_curve_across_the_window_is_split_to_pieces_inside_it() {
        // A quarter of an ellipse 20 million pixels across, through view.
        let k = 0.552_284_75 * 1e7;
        let quarter = [
            Piece::Move((400.0 - 1e7, 300.0)),
            Piece::Cubic(
                (400.0 - 1e7, 300.0 - k),
                (400.0 - k, 300.0 - 1e7),
                (400.0, 300.0 - 1e7),
            ),
            Piece::Line((400.0, 300.0)),
            Piece::Close,
        ];
        let stroke = outline(&quarter, &WINDOW, None);
        within(&stroke, &WINDOW, 0.0);
        let fill = area(&quarter, &WINDOW);
        within(&fill, &WINDOW, 0.0);
        assert!(!fill.is_empty() && !stroke.is_empty());
        // A curve near the window is kept whole.
        let small = [
            Piece::Move((0.0, 0.0)),
            Piece::Quad((50.0, -2000.0), (100.0, 0.0)),
        ];
        let stroke = outline(&small, &WINDOW, None);
        within(&stroke, &WINDOW, 0.0);
        assert!(
            stroke.iter().any(|piece| matches!(piece, Piece::Quad(..))),
            "{stroke:?}"
        );
    }

    #[test]
    fn steps_before_any_move_start_at_the_origin() {
        let path = [Piece::Line((1e7, 0.0)), Piece::Line((0.0, 10.0))];
        let cut = outline(&path, &WINDOW, None);
        assert_eq!(cut[0], Piece::Move((0.0, 0.0)));
        within(&cut, &WINDOW, 0.0);
    }
}
