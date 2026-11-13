//! Connector routes: the polyline between two ends, the side each end
//! meets its node on, and the line and arrowheads drawn along it.
//!
//! A route is computed from the board and never stored. An elbow route
//! avoids the other nodes where a small fixed set of candidate routes
//! allows it (§3.7 of the plan has the rule): each connector is routed on
//! its own against a lookup of the node boxes, so a later change can
//! re-route only the connectors near it. A straight route is one segment
//! between the ends, whatever lies between them.

use std::collections::HashMap;

use crate::scene::{Point, Rect, Segment};
use crate::shape;
use crate::{Head, Id, Named, Route, Shape, Side, Vocabulary};

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
/// when it has to turn back or go around a node, and how far a detour
/// keeps from the boxes it passes.
pub const ELBOW_MARGIN: f32 = 20.0;
/// How far a middle segment moves aside, per step, off a line another
/// connector already runs on.
pub const NUDGE: f32 = ELBOW_MARGIN / 2.0;
/// What a turn costs when candidate routes are compared, in board units of
/// length.
pub const TURN_COST: f32 = 2.0 * ELBOW_MARGIN;
/// What a segment on another connector's line or on a node's centre line
/// costs.
pub const SHARED_COST: f32 = 10.0 * ELBOW_MARGIN;
/// What crossing another connector's line costs.
pub const CROSSING_COST: f32 = 10.0 * ELBOW_MARGIN;
/// A middle segment on the centre line of a node no further than this
/// along the line runs where that node's connectors run.
pub const REACH: f32 = 6.0 * ELBOW_MARGIN;
/// Parallel segments this close are on one line.
pub const LINE_TOLERANCE: f32 = 2.0;
/// Segments on one line that share more than this length are drawn on top
/// of one another.
pub const SHARED_LENGTH: f32 = 4.0;
/// A route this close to a node's box enters it.
const CLEARANCE: f32 = 1.0;
/// At most this many channel positions are tried per template, nearest
/// the template's own first; a two-channel template takes this many of
/// each.
const CHANNELS: usize = 6;
const PAIR_CHANNELS: usize = 4;
/// How many times a detour is pushed past the boxes it still crosses.
const PUSHES: usize = 4;
/// At most this many boxes near a connector's ends give it channel
/// positions, the nearest first.
const NEAR: usize = 16;
/// The lookup one connector may do while it is routed, counted in grid
/// cells visited and in boxes and segments looked at. A connector that
/// runs out, on a crowded board or along a very long route, keeps the best
/// route it has scored, else its plain route, so routing costs at most
/// this much per connector whatever the size of the board.
const BUDGET: u32 = 12288;
/// The same while its middle segments move off other lines; one that runs
/// out stays where it is.
const SEPARATE_BUDGET: u32 = 1024;

/// How an end attached to a node meets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    /// No `side` or `anchor`: the router picks the side, the nominal one
    /// whenever the two boxes overlap on one axis (§5.12 rule 4).
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
    /// side, the side an anchor faces, or the side the router chose for a
    /// floating end. `None` at a free end.
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

/// The route of `kind` between two ends with nothing else on the board.
pub fn route(kind: Route, from: &Terminal, to: &Terminal) -> Routed {
    route_among(kind, from, to, &Obstacles::default(), [None, None])
}

/// The route of `kind` between two ends among the node boxes of
/// `obstacles`, `ends` naming the index of each end's own node there, so
/// that the route may leave and enter them.
///
/// A floating end is drawn on its nominal side whenever the two boxes
/// overlap on at most one axis (§5.12 rule 4); when they are apart on both
/// axes an elbow route may take any side of a floating end. Pinned sides
/// and anchors are always honoured, and a straight route is one segment
/// between the nominal ends.
pub fn route_among(
    kind: Route,
    from: &Terminal,
    to: &Terminal,
    obstacles: &Obstacles,
    ends: [Option<usize>; 2],
) -> Routed {
    Router::new(&obstacles.0, ends, None, BUDGET, Scratch::default()).route(kind, from, to)
}

/// One connector of a board, for [`route_all`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Connector {
    pub kind: Route,
    pub ends: [Terminal; 2],
    /// The index of each end's node in the boxes given to [`route_all`].
    pub nodes: [Option<usize>; 2],
}

/// The routes of every connector of a board among the boxes of all its
/// nodes, in the order given, which is also the order in which a middle
/// segment that would lie on an earlier connector's line moves aside.
///
/// Connectors whose sides are fixed (straight ones, pinned and anchored
/// ends, and floating ends that rule 4 holds to their nominal side) are
/// routed against the boxes alone. Those with a free choice of side are
/// then routed, in order, against the boxes and the lines of the fixed
/// connectors and of the free ones before them, so that none runs along
/// or needlessly crosses another. Last, middle segments that still lie on
/// another connector's line move aside by [`NUDGE`] steps.
pub fn route_all(boxes: &[Rect], connectors: &[Connector]) -> Vec<Routed> {
    Pass::new(boxes, connectors, None).run()
}

/// [`route_all`] for a board whose drawn nodes and connectors are named
/// by `box_ids` and `ids` (each ascending, as the board keeps them),
/// keeping in `memo` what each connector's routing read. A connector whose
/// inputs are those of the build `memo` holds, and none of whose lookups
/// would read anything different now, takes the route it had; every other
/// one is routed again. The result is always what [`route_all`] gives.
pub(crate) fn route_all_kept(
    boxes: &[Rect],
    box_ids: &[&Id],
    connectors: &[Connector],
    ids: &[&Id],
    memo: &mut Memo,
) -> Vec<Routed> {
    let keep = Keep { box_ids, ids, memo };
    Pass::new(boxes, connectors, Some(keep)).run()
}

/// What [`route_all_kept`] keeps between builds: the node boxes, and for
/// each connector its inputs, its route before and after the separation
/// pass, and what each of the two read.
#[derive(Clone, Debug, Default)]
pub(crate) struct Memo {
    boxes: Vec<(Id, Rect)>,
    kept: Vec<Kept>,
    /// How many connectors the last build routed again, and how many it
    /// separated again, instead of taking them from the memo.
    pub(crate) rerouted: usize,
    pub(crate) reseparated: usize,
}

impl Memo {
    pub(crate) fn is_empty(&self) -> bool {
        self.boxes.is_empty() && self.kept.is_empty()
    }
}

/// One connector as the build before routed it.
#[derive(Clone, Debug)]
struct Kept {
    id: Id,
    kind: Route,
    ends: [Terminal; 2],
    nodes: [Option<Id>; 2],
    /// The route before the separation pass, and what routing it read
    /// from the node boxes and from the lines already drawn.
    routed: Routed,
    reads: [Reads; 2],
    /// The route after it, and what separating it read from the node
    /// boxes and from the lines placed.
    separated: Routed,
    apart: [Reads; 2],
}

/// The names [`route_all_kept`] keeps its memo by.
struct Keep<'a, 'm> {
    box_ids: &'a [&'a Id],
    ids: &'a [&'a Id],
    memo: &'m mut Memo,
}

/// What a connector's lookups read from one grid, kept so that a later
/// build can tell whether they would read the same. A query's result and
/// its cost are made from the entries of the cells it visits, so the
/// cells are what it read.
#[derive(Clone, Debug, Default)]
struct Reads {
    /// Off when nothing is kept, as in [`route_all`].
    on: bool,
    /// The cells of each query, as first and last column and first and
    /// last row; a query within the one before is not listed again.
    spans: Vec<[i32; 4]>,
    /// The cells of all of them, once there is one.
    hull: [i32; 4],
    /// The most cells one query visited one by one: it would have read
    /// every rectangle instead in a grid holding fewer.
    most: i64,
    /// Whether a query read every rectangle.
    every: bool,
}

impl Reads {
    fn kept(on: bool) -> Self {
        Self {
            on,
            ..Self::default()
        }
    }

    fn cells(&mut self, (x, y, count): Span) {
        if !self.on {
            return;
        }
        let span = [x[0], x[1], y[0], y[1]];
        self.most = self.most.max(count);
        match self.spans.last() {
            Some(&last) if within(span, last) => {}
            Some(_) => {
                self.hull = hull_of(self.hull, span);
                self.spans.push(span);
            }
            None => {
                self.hull = span;
                self.spans.push(span);
            }
        }
    }

    fn every(&mut self) {
        self.every |= self.on;
    }

    /// Whether these lookups might read something else from a grid that
    /// now holds `len` rectangles and has changed in `dirty`.
    fn changed(&self, dirty: &Dirty, len: usize) -> bool {
        if self.every {
            // A grid that changed nowhere holds as many as before.
            return !dirty.is_empty();
        }
        if self.spans.is_empty() {
            return false;
        }
        if self.most > len as i64 || dirty.all {
            return true;
        }
        dirty
            .cells
            .iter()
            .any(|&d| meets(self.hull, d) && self.spans.iter().any(|&s| meets(s, d)))
    }
}

/// Whether two blocks of cells share one.
fn meets(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] <= b[1] && b[0] <= a[1] && a[2] <= b[3] && b[2] <= a[3]
}

/// Whether block `a` lies within block `b`.
fn within(a: [i32; 4], b: [i32; 4]) -> bool {
    b[0] <= a[0] && a[1] <= b[1] && b[2] <= a[2] && a[3] <= b[3]
}

fn hull_of(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].min(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].max(b[3]),
    ]
}

/// Where one grid changed since the build before: the cells of every
/// rectangle added, removed or moved, at its old place and its new, and
/// `all` when one of them was too large for cells, since every query
/// reads those.
#[derive(Clone, Debug, Default)]
struct Dirty {
    all: bool,
    cells: Vec<[i32; 4]>,
}

impl Dirty {
    fn rect(&mut self, r: Rect) {
        let (x, y, count) = Grid::cells_of(r);
        if count > MAX_CELLS {
            self.all = true;
        } else {
            self.cells.push([x[0], x[1], y[0], y[1]]);
        }
    }

    /// The segments of a route, as a line lookup holds them.
    fn segments(&mut self, points: &[Point]) {
        for pair in points.windows(2) {
            self.rect(grow(segment_box(pair[0], pair[1]), LINE_TOLERANCE));
        }
    }

    fn is_empty(&self) -> bool {
        !self.all && self.cells.is_empty()
    }
}

/// Whether a connector is routed with a side to choose, and so after the
/// fixed ones and against their lines.
fn is_free(c: &Connector) -> bool {
    c.kind == Route::Elbow && has_choice(&c.ends[0], &c.ends[1])
}

/// One routing of a board's connectors.
struct Pass<'a> {
    obstacles: Grid,
    count: usize,
    connectors: &'a [Connector],
    /// Each connector as the build before kept it, when there was one
    /// with the same id; `same` when its inputs are the same too.
    prev: Vec<Option<Kept>>,
    same: Vec<bool>,
    /// Where the boxes changed, where the lines a free connector routes
    /// against changed, and where those the separation pass places.
    boxes: Dirty,
    lines: Dirty,
    placed: Dirty,
    keep: Option<Keep<'a, 'a>>,
    /// What each connector read, routing and separating, while kept.
    reads: Vec<[Reads; 2]>,
    apart: Vec<[Reads; 2]>,
    scratch: Scratch,
    rerouted: usize,
    reseparated: usize,
}

impl<'a> Pass<'a> {
    fn new(boxes: &[Rect], connectors: &'a [Connector], keep: Option<Keep<'a, 'a>>) -> Self {
        let n = connectors.len();
        let mut pass = Self {
            obstacles: Grid::new(boxes.iter().copied()),
            count: boxes.len(),
            connectors,
            prev: (0..n).map(|_| None).collect(),
            same: vec![false; n],
            boxes: Dirty::default(),
            lines: Dirty::default(),
            placed: Dirty::default(),
            keep: None,
            reads: Vec::new(),
            apart: Vec::new(),
            scratch: Scratch::default(),
            rerouted: 0,
            reseparated: 0,
        };
        if let Some(mut keep) = keep {
            pass.reads.resize_with(n, Default::default);
            pass.apart.resize_with(n, Default::default);
            pass.match_memo(&mut keep, boxes);
            pass.keep = Some(keep);
        }
        pass
    }

    /// Takes each connector of the memo for the connector of this build
    /// with its id, and marks what changed since: the boxes added, removed
    /// or moved, and every line of a connector that is gone.
    fn match_memo(&mut self, keep: &mut Keep<'_, '_>, boxes: &[Rect]) {
        let mut old = std::mem::take(&mut keep.memo.boxes).into_iter().peekable();
        let mut now = Vec::with_capacity(boxes.len());
        for (&id, &r) in keep.box_ids.iter().zip(boxes) {
            while let Some((_, gone)) = old.next_if(|(k, _)| k < id) {
                self.boxes.rect(gone);
            }
            let name = match old.next_if(|(k, _)| k == id) {
                Some((name, was)) => {
                    if was != r {
                        self.boxes.rect(was);
                        self.boxes.rect(r);
                    }
                    name
                }
                None => {
                    self.boxes.rect(r);
                    id.clone()
                }
            };
            now.push((name, r));
        }
        for (_, gone) in old {
            self.boxes.rect(gone);
        }
        keep.memo.boxes = now;
        let mut old = std::mem::take(&mut keep.memo.kept).into_iter().peekable();
        let mut gone = Vec::new();
        for (i, (&id, c)) in keep.ids.iter().zip(self.connectors).enumerate() {
            gone.extend(std::iter::from_fn(|| old.next_if(|k| k.id < *id)));
            if let Some(k) = old.next_if(|k| k.id == *id) {
                let names = c
                    .nodes
                    .map(|n| n.and_then(|n| keep.box_ids.get(n).copied()));
                self.same[i] = k.kind == c.kind
                    && k.ends == c.ends
                    && k.nodes.iter().zip(names).all(|(a, b)| a.as_ref() == b);
                self.prev[i] = Some(k);
            }
        }
        gone.extend(old);
        // A connector whose inputs changed may change class, fixed to free
        // or elbow to straight, and then stop being in the lines of
        // connectors routed before it is: its old lines are changed for
        // all of them, as a gone connector's are.
        let changed = self.prev.iter().zip(&self.same).filter(|(_, &same)| !same);
        for k in gone.iter().chain(changed.filter_map(|(k, _)| k.as_ref())) {
            for points in [&k.routed.points, &k.separated.points] {
                self.lines.segments(points);
                self.placed.segments(points);
            }
        }
    }

    /// The routes, in the order of the connectors.
    fn run(mut self) -> Vec<Routed> {
        let n = self.connectors.len();
        let free: Vec<bool> = self.connectors.iter().map(is_free).collect();
        let mut routes: Vec<Option<Routed>> = vec![None; n];
        for i in (0..n).filter(|&i| !free[i]) {
            routes[i] = Some(self.route(i, None));
        }
        // Only a free connector looks at the lines drawn.
        let mut drawn = Lines::default();
        if free.contains(&true) {
            drawn = Lines::new(segments(
                routes.iter().map(Option::as_ref).enumerate(),
                |_, _| true,
            ));
        }
        for i in (0..n).filter(|&i| free[i]) {
            let routed = self.route(i, Some(&drawn));
            for pair in routed.points.windows(2) {
                drawn.insert(pair[0], pair[1], i);
            }
            routes[i] = Some(routed);
        }
        let mut routes: Vec<Routed> = routes.into_iter().flatten().collect();
        let before = self.keep.is_some().then(|| routes.clone());
        self.separate(&mut routes);
        if let (Some(keep), Some(before)) = (self.keep.take(), before) {
            let connectors = self.connectors;
            // The names of a connector the memo held are taken from it.
            let names = |i: usize, prev: Option<Kept>| match prev {
                Some(prev) if self.same[i] => (prev.id, prev.nodes),
                prev => {
                    let id = prev.map_or_else(|| keep.ids[i].clone(), |prev| prev.id);
                    let node = |n: Option<usize>| {
                        n.and_then(|n| keep.box_ids.get(n)).map(|&id| id.clone())
                    };
                    (id, connectors[i].nodes.map(node))
                }
            };
            keep.memo.kept = before
                .into_iter()
                .zip(&routes)
                .zip(self.reads.into_iter().zip(self.apart))
                .zip(self.prev)
                .enumerate()
                .map(|(i, (((routed, separated), (reads, apart)), prev))| {
                    let (id, nodes) = names(i, prev);
                    Kept {
                        id,
                        kind: connectors[i].kind,
                        ends: connectors[i].ends,
                        nodes,
                        routed,
                        reads,
                        separated: separated.clone(),
                        apart,
                    }
                })
                .collect();
            keep.memo.rerouted = self.rerouted;
            keep.memo.reseparated = self.reseparated;
        }
        routes
    }

    /// The route of connector `i`, against `lines` when it has a side to
    /// choose: the one the memo holds when nothing it read has changed,
    /// else routed again. A route that differs from the one before marks
    /// its old and new lines changed for the connectors after it.
    fn route(&mut self, i: usize, lines: Option<&Lines>) -> Routed {
        let len = lines.map_or(0, |l| l.segments.len());
        if let Some(prev) = self.prev[i].as_mut().filter(|_| self.same[i]) {
            if !prev.reads[0].changed(&self.boxes, self.count)
                && !prev.reads[1].changed(&self.lines, len)
            {
                self.reads[i] = std::mem::take(&mut prev.reads);
                return prev.routed.clone();
            }
        }
        let connector = &self.connectors[i];
        let record = self.keep.is_some();
        let scratch = std::mem::take(&mut self.scratch);
        let mut router = Router::new(&self.obstacles, connector.nodes, lines, BUDGET, scratch);
        router.reads = [Reads::kept(record), Reads::kept(record)];
        let routed = router.route(connector.kind, &connector.ends[0], &connector.ends[1]);
        let Router { scratch, reads, .. } = router;
        self.scratch = scratch;
        if record {
            self.reads[i] = reads;
            self.rerouted += 1;
            let old = self.prev[i].as_ref().map(|k| &k.routed);
            if !old.is_some_and(|old| old.same(&routed)) {
                for points in old.map(|r| &r.points).into_iter().chain([&routed.points]) {
                    self.lines.segments(points);
                    self.placed.segments(points);
                }
            }
        }
        routed
    }

    /// Moves aside, in connector order, each middle segment of an elbow
    /// route that lies on a line already drawn: a straight connector's, the
    /// first or last segment of any connector's, or an earlier connector's
    /// middle segment. It moves by [`NUDGE`], twice that or three times,
    /// either way, to the first place that is on no other line, enters no
    /// more boxes and keeps every segment's direction, preferring fewer
    /// crossings and then the longer of its neighbours; where there is
    /// none it stays. A connector whose route and reading are those of the
    /// build before takes its separated route from the memo.
    fn separate(&mut self, routes: &mut [Routed]) {
        let connectors = self.connectors;
        // Only a middle segment moves.
        let middle = |(c, r): (&Connector, &Routed)| c.kind == Route::Elbow && r.points.len() > 3;
        if !connectors.iter().zip(routes.iter()).any(middle) {
            return;
        }
        let ends = |i: usize, j: usize| {
            let last = routes[i].points.len().saturating_sub(2);
            connectors[i].kind != Route::Elbow || j == 0 || j == last
        };
        let mut placed = Lines::new(segments(routes.iter().map(Some).enumerate(), ends));
        let record = self.keep.is_some();
        for (i, connector) in connectors.iter().enumerate() {
            if connector.kind != Route::Elbow {
                continue;
            }
            let len = placed.segments.len();
            let points = &mut routes[i].points;
            let prev = self.prev[i].as_mut().filter(|_| self.same[i]);
            let reuse = prev.filter(|prev| {
                same_points(&prev.routed.points, points)
                    && !prev.apart[0].changed(&self.boxes, self.count)
                    && !prev.apart[1].changed(&self.placed, len)
            });
            if let Some(prev) = reuse {
                self.apart[i] = std::mem::take(&mut prev.apart);
                points.clone_from(&prev.separated.points);
            } else {
                let scratch = std::mem::take(&mut self.scratch);
                let mut router = Router::new(
                    &self.obstacles,
                    connector.nodes,
                    None,
                    SEPARATE_BUDGET,
                    scratch,
                );
                router.reads = [Reads::kept(record), Reads::kept(record)];
                let mut j = 1;
                while j + 2 < points.len() {
                    let shares = placed.shares(
                        points[j],
                        points[j + 1],
                        Some(i),
                        &mut router.budget,
                        &mut router.reads[1],
                    );
                    match shares {
                        Some(true) => {
                            if let Some(moved) = nudge(&mut router, &mut placed, points, j, i) {
                                *points = moved;
                            }
                        }
                        Some(false) => {}
                        None => break,
                    }
                    j += 1;
                }
                let Router { scratch, reads, .. } = router;
                self.scratch = scratch;
                if record {
                    self.apart[i] = reads;
                    self.reseparated += 1;
                    let old = self.prev[i].as_ref().map(|k| &k.separated.points);
                    if !old.is_some_and(|old| same_points(old, points)) {
                        for points in old.into_iter().chain([&*points]) {
                            self.placed.segments(points);
                        }
                    }
                }
            }
            for j in 1..points.len().saturating_sub(2) {
                placed.insert(points[j], points[j + 1], i);
            }
        }
    }
}

/// Where an end meets its node and the side it leaves by, a floating end
/// and a free point taking `nominal`. A connector whose other end names
/// no node draws its stub from here.
pub fn leave(terminal: &Terminal, nominal: Side) -> (Point, Side) {
    let (point, side, _) = end(terminal, nominal, nominal);
    (point, side)
}

/// Where an end is, the side it leaves by, and the side it reports: a
/// floating end on `side`, a free point leaving by `nominal`.
fn end(terminal: &Terminal, side: Side, nominal: Side) -> (Point, Side, Option<Side>) {
    match *terminal {
        Terminal::Point(p) => (p, nominal, None),
        Terminal::Node {
            bounds,
            shape,
            attach,
        } => {
            let (point, side) = match attach {
                Attach::Floating => (shape::side_point(shape, bounds, side), side),
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

/// Whether two spans overlap by more than a point.
fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> bool {
    a0 < b1 && b0 < a1
}

/// Whether a floating end of this pair may take a side other than its
/// nominal one: only when the boxes are apart on both axes, so rule 4
/// holds wherever it applies.
fn apart_on_both_axes(a: Rect, b: Rect) -> bool {
    !overlap(a.x, a.right(), b.x, b.right()) && !overlap(a.y, a.bottom(), b.y, b.bottom())
}

/// Whether an elbow route between these ends has a side to choose.
fn has_choice(from: &Terminal, to: &Terminal) -> bool {
    let floating = |t: &Terminal| {
        matches!(
            t,
            Terminal::Node {
                attach: Attach::Floating,
                ..
            }
        )
    };
    (floating(from) || floating(to)) && apart_on_both_axes(from.bounds(), to.bounds())
}

/// The sides an end may take, `nominal` first: the first `n` of the
/// array.
fn choices(terminal: &Terminal, nominal: Side, free: bool) -> ([Side; 4], usize) {
    let floating = matches!(
        terminal,
        Terminal::Node {
            attach: Attach::Floating,
            ..
        }
    );
    let mut sides = [nominal; 4];
    if !(free && floating) {
        return (sides, 1);
    }
    for (slot, &side) in sides[1..]
        .iter_mut()
        .zip(Side::ALL.iter().filter(|&&s| s != nominal))
    {
        *slot = side;
    }
    (sides, 4)
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
fn simplify(mut points: Vec<Point>) -> Vec<Point> {
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return points;
    };
    // In place: the points kept are a prefix that never passes the point
    // being read.
    let mut kept = 0;
    for read in 0..points.len() {
        let p = points[read];
        if kept > 0 && points[kept - 1] == p {
            continue;
        }
        if kept >= 2 {
            let (before, middle) = (points[kept - 2], points[kept - 1]);
            let straight = (before.x == middle.x
                && middle.x == p.x
                && (middle.y - before.y) * (p.y - middle.y) >= 0.0)
                || (before.y == middle.y
                    && middle.y == p.y
                    && (middle.x - before.x) * (p.x - middle.x) >= 0.0);
            if straight {
                kept -= 1;
            }
        }
        points[kept] = p;
        kept += 1;
    }
    if kept < 2 {
        return vec![first, last];
    }
    points.truncate(kept);
    points
}

fn distance(a: Point, b: Point) -> f32 {
    (b.x - a.x).hypot(b.y - a.y)
}

/// The node boxes a route keeps out of, looked up by area.
#[derive(Clone, Debug, Default)]
pub struct Obstacles(Grid);

impl Obstacles {
    /// The boxes, each named by its index in the order given.
    pub fn new(boxes: impl IntoIterator<Item = Rect>) -> Self {
        Self(Grid::new(boxes))
    }
}

/// Rectangles bucketed into square cells, so a query visits only those
/// near its area. A rectangle over too many cells is kept in a list every
/// query checks, and a query over too many cells checks every rectangle,
/// so neither a huge box nor a huge query costs more than a scan.
///
/// A cell's list holds each rectangle's index, marked with
/// [`FIRST_COLUMN`] in the cells of its first column and [`FIRST_ROW`] in
/// those of its first row, ascending.
#[derive(Clone, Debug, Default)]
struct Grid {
    rects: Vec<Rect>,
    /// The cells of the rectangles the grid was made with, when they lie
    /// close enough together.
    array: Array,
    /// The cells of the others and of rectangles added later.
    map: HashMap<(i32, i32), Vec<u32>>,
    large: Vec<u32>,
}

/// The marks of a cell entry; the index is in the bits below them. A
/// board is far from 2^30 boxes or line segments within the 64 MiB a
/// note may hold.
const FIRST_COLUMN: u32 = 1 << 31;
const FIRST_ROW: u32 = 1 << 30;
const INDEX: u32 = FIRST_ROW - 1;

const CELL: f32 = 256.0;
const MAX_CELLS: i64 = 64;

/// Every cell of a block, row by row, a query reading one without
/// hashing: cell `k`'s list is `entries[starts[k]..starts[k + 1]]`.
#[derive(Clone, Debug, Default)]
struct Array {
    /// The block's first cell, and its cells across and down.
    origin: [i32; 2],
    size: [usize; 2],
    starts: Vec<u32>,
    entries: Vec<u32>,
}

impl Array {
    fn index(&self, cx: i32, cy: i32) -> Option<usize> {
        let dx = usize::try_from(i64::from(cx) - i64::from(self.origin[0])).ok()?;
        let dy = usize::try_from(i64::from(cy) - i64::from(self.origin[1])).ok()?;
        (dx < self.size[0] && dy < self.size[1]).then(|| dy * self.size[0] + dx)
    }

    fn get(&self, cx: i32, cy: i32) -> &[u32] {
        match self.index(cx, cy) {
            Some(k) => &self.entries[self.starts[k] as usize..self.starts[k + 1] as usize],
            None => &[],
        }
    }
}

/// The cells of a rectangle, as first and last column, first and last
/// row, and their number.
type Span = ([i32; 2], [i32; 2], i64);

/// The entries of a rectangle's cells, each with its marks.
fn entries(index: u32, (x, y, _): Span) -> impl Iterator<Item = ((i32, i32), u32)> {
    (x[0]..=x[1]).flat_map(move |cx| {
        (y[0]..=y[1]).map(move |cy| {
            let column = if cx == x[0] { FIRST_COLUMN } else { 0 };
            let row = if cy == y[0] { FIRST_ROW } else { 0 };
            ((cx, cy), index | column | row)
        })
    })
}

impl Grid {
    /// A grid of `rects`: in an array when the block of cells they fill
    /// holds no more than four for each entry (with some to spare for a
    /// small board), else in the map.
    fn new(rects: impl IntoIterator<Item = Rect>) -> Self {
        let rects: Vec<Rect> = rects.into_iter().collect();
        let spans: Vec<Span> = rects.iter().map(|&r| Self::cells_of(r)).collect();
        let mut grid = Self {
            rects,
            ..Self::default()
        };
        let (mut low, mut high, mut count) = ([i32::MAX; 2], [i32::MIN; 2], 0u64);
        for (index, &(x, y, cells)) in spans.iter().enumerate() {
            if cells > MAX_CELLS {
                grid.large.push(index as u32);
                continue;
            }
            low = [low[0].min(x[0]), low[1].min(y[0])];
            high = [high[0].max(x[1]), high[1].max(y[1])];
            count += cells as u64;
        }
        if count == 0 {
            return grid;
        }
        let across = |k: usize| (i64::from(high[k]) - i64::from(low[k]) + 1) as u64;
        if across(0).saturating_mul(across(1)) > 4 * count + 1024 {
            for (index, &span) in spans.iter().enumerate() {
                if span.2 <= MAX_CELLS {
                    for (cell, entry) in entries(index as u32, span) {
                        grid.map.entry(cell).or_default().push(entry);
                    }
                }
            }
            return grid;
        }
        let mut array = Array {
            origin: low,
            size: [across(0) as usize, across(1) as usize],
            starts: vec![0; (across(0) * across(1)) as usize + 1],
            entries: vec![0; count as usize],
        };
        let small = || {
            spans
                .iter()
                .enumerate()
                .filter(|(_, span)| span.2 <= MAX_CELLS)
                .flat_map(|(index, &span)| entries(index as u32, span))
        };
        for ((cx, cy), _) in small() {
            if let Some(k) = array.index(cx, cy) {
                array.starts[k + 1] += 1;
            }
        }
        for k in 1..array.starts.len() {
            array.starts[k] += array.starts[k - 1];
        }
        let mut next = array.starts.clone();
        for ((cx, cy), entry) in small() {
            if let Some(k) = array.index(cx, cy) {
                array.entries[next[k] as usize] = entry;
                next[k] += 1;
            }
        }
        grid.array = array;
        grid
    }

    fn cells_of(r: Rect) -> Span {
        // A saturating cast: coordinates are clamped far inside i32 anyway.
        let cell = |v: f32| (v / CELL).floor() as i32;
        let (x, y) = ([cell(r.x), cell(r.right())], [cell(r.y), cell(r.bottom())]);
        let count =
            (i64::from(x[1]) - i64::from(x[0]) + 1) * (i64::from(y[1]) - i64::from(y[0]) + 1);
        (x, y, count)
    }

    fn insert(&mut self, rect: Rect) {
        let index = self.rects.len() as u32;
        self.rects.push(rect);
        let span = Self::cells_of(rect);
        if span.2 > MAX_CELLS {
            self.large.push(index);
            return;
        }
        for (cell, entry) in entries(index, span) {
            self.map.entry(cell).or_default().push(entry);
        }
    }

    /// The indices of the rectangles touching `area`, each once, in no
    /// particular order; each cell visited and each rectangle looked at is
    /// taken from `budget`. When the budget runs out the query stops,
    /// leaves it at 0 and returns false; `out` is then incomplete.
    fn query(&self, area: Rect, out: &mut Vec<u32>, budget: &mut u32, reads: &mut Reads) -> bool {
        out.clear();
        let span = Self::cells_of(area);
        let (x, y, count) = span;
        let touching = |&i: &u32| touches(self.rects[i as usize], area);
        if count > self.rects.len() as i64 {
            reads.every();
            if !spend(budget, self.rects.len()) {
                return false;
            }
            out.extend((0..self.rects.len() as u32).filter(touching));
            return true;
        }
        reads.cells(span);
        // A rectangle over several of these cells is taken from the first
        // of them, where its first column or the query's meets its first
        // row or the query's, so no sort is needed to drop the repeats.
        for cx in x[0]..=x[1] {
            for cy in y[0]..=y[1] {
                let made = self.array.get(cx, cy);
                let added = match self.map.is_empty() {
                    true => &[][..],
                    false => self.map.get(&(cx, cy)).map_or(&[][..], Vec::as_slice),
                };
                if !spend(budget, 1 + made.len() + added.len()) {
                    return false;
                }
                let (column, row) = (cx == x[0], cy == y[0]);
                out.extend(made.iter().chain(added).filter_map(|&entry| {
                    let first =
                        (column || entry & FIRST_COLUMN != 0) && (row || entry & FIRST_ROW != 0);
                    let i = entry & INDEX;
                    (first && touching(&i)).then_some(i)
                }));
            }
        }
        if !spend(budget, self.large.len()) {
            return false;
        }
        out.extend(self.large.iter().filter(|i| touching(i)));
        true
    }
}

/// Takes `n` from `budget`, or empties it and returns false when it holds
/// less.
fn spend(budget: &mut u32, n: usize) -> bool {
    match u32::try_from(n).ok().and_then(|n| budget.checked_sub(n)) {
        Some(left) => {
            *budget = left;
            true
        }
        None => {
            *budget = 0;
            false
        }
    }
}

fn touches(a: Rect, b: Rect) -> bool {
    a.x <= b.right() && b.x <= a.right() && a.y <= b.bottom() && b.y <= a.bottom()
}

fn grow(r: Rect, by: f32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.width + 2.0 * by, r.height + 2.0 * by)
}

/// The smallest box holding both.
fn hull(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}

fn segment_box(p: Point, q: Point) -> Rect {
    hull(Rect::new(p.x, p.y, 0.0, 0.0), Rect::new(q.x, q.y, 0.0, 0.0))
}

/// Whether the segment from `p` to `q` reaches inside `r` grown by
/// `clearance`. Exact for the axis-aligned segments of an elbow route.
fn enters(p: Point, q: Point, r: Rect, clearance: f32) -> bool {
    let s = segment_box(p, q);
    s.x < r.right() + clearance
        && s.right() > r.x - clearance
        && s.y < r.bottom() + clearance
        && s.bottom() > r.y - clearance
}

/// Segments already drawn, with the connector each belongs to.
#[derive(Clone, Debug, Default)]
struct Lines {
    segments: Vec<(Point, Point, usize)>,
    grid: Grid,
    found: Vec<u32>,
}

impl Lines {
    /// The lines of `segments`, each with its connector.
    fn new(segments: Vec<(Point, Point, usize)>) -> Self {
        let boxes = segments
            .iter()
            .map(|&(p, q, _)| grow(segment_box(p, q), LINE_TOLERANCE));
        Self {
            grid: Grid::new(boxes),
            segments,
            found: Vec::new(),
        }
    }

    fn insert(&mut self, p: Point, q: Point, owner: usize) {
        self.segments.push((p, q, owner));
        self.grid.insert(grow(segment_box(p, q), LINE_TOLERANCE));
    }

    /// The segments of other connectors near the one from `p` to `q`, as
    /// [`Grid::query`] finds them within `budget`.
    fn near(
        &self,
        p: Point,
        q: Point,
        owner: Option<usize>,
        found: &mut Vec<u32>,
        budget: &mut u32,
        reads: &mut Reads,
    ) -> bool {
        let complete = self.grid.query(segment_box(p, q), found, budget, reads);
        found.retain(|&i| Some(self.segments[i as usize].2) != owner);
        complete
    }

    /// Whether the segment from `p` to `q` lies on another connector's;
    /// `None` when `budget` runs out first.
    fn shares(
        &mut self,
        p: Point,
        q: Point,
        owner: Option<usize>,
        budget: &mut u32,
        reads: &mut Reads,
    ) -> Option<bool> {
        let mut found = std::mem::take(&mut self.found);
        let complete = self.near(p, q, owner, &mut found, budget, reads);
        let shared = found.iter().any(|&i| {
            let (a, b, _) = self.segments[i as usize];
            collinear_overlap(p, q, a, b) > SHARED_LENGTH
        });
        self.found = found;
        complete.then_some(shared)
    }

    /// How many other connectors' segments the route crosses; `None` when
    /// `budget` runs out first.
    fn crossings(
        &mut self,
        points: &[Point],
        owner: Option<usize>,
        budget: &mut u32,
        reads: &mut Reads,
    ) -> Option<u32> {
        let mut found = std::mem::take(&mut self.found);
        let (mut count, mut complete) = (0, true);
        for pair in points.windows(2) {
            if !self.near(pair[0], pair[1], owner, &mut found, budget, reads) {
                complete = false;
                break;
            }
            count += found
                .iter()
                .filter(|&&i| {
                    let (a, b, _) = self.segments[i as usize];
                    crosses(pair[0], pair[1], a, b)
                })
                .count() as u32;
        }
        self.found = found;
        complete.then_some(count)
    }
}

/// How long two segments run on one line together; 0 unless both are
/// horizontal or both vertical, no more than [`LINE_TOLERANCE`] apart.
fn collinear_overlap(p: Point, q: Point, a: Point, b: Point) -> f32 {
    let span = |u0: f32, u1: f32, v0: f32, v1: f32| {
        (u0.max(u1).min(v0.max(v1)) - u0.min(u1).max(v0.min(v1))).max(0.0)
    };
    if p.y == q.y && a.y == b.y && (p.y - a.y).abs() <= LINE_TOLERANCE {
        span(p.x, q.x, a.x, b.x)
    } else if p.x == q.x && a.x == b.x && (p.x - a.x).abs() <= LINE_TOLERANCE {
        span(p.y, q.y, a.y, b.y)
    } else {
        0.0
    }
}

/// Whether the axis-aligned segment from `p` to `q` crosses the segment
/// from `a` to `b` at a point inside both; meeting at an end is not a
/// crossing.
fn crosses(p: Point, q: Point, a: Point, b: Point) -> bool {
    let inside = |v: f32, s0: f32, s1: f32| s0.min(s1) < v && v < s0.max(s1);
    if p.y == q.y {
        let y = p.y;
        if (a.y - y) * (b.y - y) >= 0.0 {
            return false;
        }
        let x = a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y);
        inside(x, p.x, q.x)
    } else if p.x == q.x {
        let x = p.x;
        if (a.x - x) * (b.x - x) >= 0.0 {
            return false;
        }
        let y = a.y + (x - a.x) * (b.y - a.y) / (b.x - a.x);
        inside(y, p.y, q.y)
    } else {
        false
    }
}

/// How a candidate route compares: fewer node boxes crossed first, then
/// the lower cost.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Score {
    crossings: u32,
    cost: f32,
}

impl Score {
    fn beats(self, other: Option<Score>) -> bool {
        other.is_none_or(|o| {
            self.crossings < o.crossings || (self.crossings == o.crossings && self.cost < o.cost)
        })
    }
}

/// Coordinates along a side's axis (`u`) and across it (`v`).
#[derive(Clone, Copy)]
struct Frame(bool);

impl Frame {
    fn of(side: Side) -> Self {
        Self(side.is_horizontal())
    }

    fn point(self, u: f32, v: f32) -> Point {
        if self.0 {
            Point::new(u, v)
        } else {
            Point::new(v, u)
        }
    }

    fn u(self, p: Point) -> f32 {
        if self.0 {
            p.x
        } else {
            p.y
        }
    }

    fn v(self, p: Point) -> f32 {
        if self.0 {
            p.y
        } else {
            p.x
        }
    }

    fn span_u(self, r: Rect) -> (f32, f32) {
        if self.0 {
            (r.x, r.right())
        } else {
            (r.y, r.bottom())
        }
    }

    fn span_v(self, r: Rect) -> (f32, f32) {
        Self(!self.0).span_u(r)
    }
}

/// +1 when leaving `side` increases the coordinate, else −1.
fn sign(side: Side) -> f32 {
    match side {
        Side::Right | Side::Bottom => 1.0,
        Side::Top | Side::Left => -1.0,
    }
}

/// The middle of the gap between two spans, if they are apart.
fn gap_middle(a: (f32, f32), b: (f32, f32)) -> Option<f32> {
    if a.1 <= b.0 {
        Some((a.1 + b.0) / 2.0)
    } else if b.1 <= a.0 {
        Some((b.1 + a.0) / 2.0)
    } else {
        None
    }
}

/// Channel positions to try: the valid ones of `values`, nearest
/// `primary` first, at most `cap`.
fn channels(primary: f32, values: &[f32], ok: impl Fn(f32) -> bool, cap: usize) -> Vec<f32> {
    let mut out: Vec<f32> = values
        .iter()
        .copied()
        .filter(|&c| c.is_finite() && ok(c))
        .collect();
    out.sort_by(|a, b| {
        (a - primary)
            .abs()
            .total_cmp(&(b - primary).abs())
            .then(a.total_cmp(b))
    });
    out.dedup();
    out.truncate(cap);
    out
}

/// Whether a route leaves its first point straight out of `da` and
/// arrives at its last straight in through `db`.
fn leaves_and_enters(points: &[Point], da: Side, db: Side) -> bool {
    let out = |from: Point, to: Point, side: Side| {
        let level = if side.is_horizontal() {
            from.y == to.y
        } else {
            from.x == to.x
        };
        level && ahead(from, to, side) > 0.0
    };
    let n = points.len();
    n >= 2 && out(points[0], points[1], da) && out(points[n - 1], points[n - 2], db)
}

/// The least cost any route between these sides can have: the distance
/// between the ends along both axes and the turns the sides need.
fn fewest(a: Point, da: Side, b: Point, db: Side) -> f32 {
    let turns = if da.is_horizontal() != db.is_horizontal() {
        1.0
    } else if da == db.opposite() && ahead(a, b, da) > 0.0 && (a.x == b.x || a.y == b.y) {
        0.0
    } else {
        2.0
    };
    (b.x - a.x).abs() + (b.y - a.y).abs() + TURN_COST * turns
}

fn length_of(points: &[Point]) -> f32 {
    points.windows(2).map(|w| distance(w[0], w[1])).sum()
}

/// Working memory one connector's routing leaves for the next.
#[derive(Default)]
struct Scratch {
    found: Vec<u32>,
    hits: Vec<u32>,
    candidates: Vec<Vec<Point>>,
    /// For each box, the last [`Router::hits`] call that counted it, so
    /// a box entered by several segments is counted once without a sort.
    seen: Vec<u32>,
    call: u32,
}

/// Routes one connector among the node boxes and, for a connector with a
/// side to choose, the lines already drawn: the fixed connectors' and
/// those of the free connectors before it.
struct Router<'a> {
    obstacles: &'a Grid,
    /// The index of each end's node among the obstacles.
    own: [Option<usize>; 2],
    lines: Option<&'a Lines>,
    scratch: Scratch,
    /// What lookup this connector may still do ([`BUDGET`]).
    budget: u32,
    /// What its lookups read from the boxes and from the lines, while a
    /// memo keeps it.
    reads: [Reads; 2],
}

impl<'a> Router<'a> {
    fn new(
        obstacles: &'a Grid,
        own: [Option<usize>; 2],
        lines: Option<&'a Lines>,
        budget: u32,
        scratch: Scratch,
    ) -> Self {
        Self {
            obstacles,
            own,
            lines,
            scratch,
            budget,
            reads: Default::default(),
        }
    }

    fn route(&mut self, kind: Route, from: &Terminal, to: &Terminal) -> Routed {
        let (ra, rb) = (from.bounds(), to.bounds());
        let exit = nominal_exit(ra, rb);
        let entry = exit.opposite();
        if kind == Route::Straight {
            let (a, da, a_shown) = end(from, exit, exit);
            let (b, db, b_shown) = end(to, entry, entry);
            return Routed {
                points: vec![a, b],
                sides: [a_shown, b_shown],
                outward: [da, db],
            };
        }
        let free = apart_on_both_axes(ra, rb);
        let (from_sides, n) = choices(from, exit, free);
        let from_sides = &from_sides[..n];
        let (to_sides, n) = choices(to, entry, free);
        let to_sides = &to_sides[..n];
        let single = from_sides.len() == 1 && to_sides.len() == 1;
        let mut near: Option<Vec<Rect>> = None;
        let mut best: Option<(Score, Routed)> = None;
        let mut hits = std::mem::take(&mut self.scratch.hits);
        let mut candidates = std::mem::take(&mut self.scratch.candidates);
        let mut first = true;
        'pairs: for &sa in from_sides {
            for &sb in to_sides {
                if self.budget == 0 {
                    break 'pairs;
                }
                let (a, da, a_shown) = end(from, sa, exit);
                let (b, db, b_shown) = end(to, sb, entry);
                let routed = |points| Routed {
                    points,
                    sides: [a_shown, b_shown],
                    outward: [da, db],
                };
                // Ends that meet, as on touching sides, have nothing to
                // route. The plain route of the first pair is always a
                // candidate, so a degenerate layout still draws one.
                let plain = if a == b {
                    Some(vec![a, b])
                } else if first {
                    Some(elbow(a, da, ra, b, db, rb))
                } else {
                    None
                };
                first = false;
                if let Some(points) = plain {
                    let base =
                        length_of(&points) + TURN_COST * points.len().saturating_sub(2) as f32;
                    if let Some(score) = self.score(&points, best.as_ref().map(|b| b.0), &mut hits)
                    {
                        if score.beats(best.as_ref().map(|b| b.0)) {
                            let clean = score.crossings == 0 && score.cost == base;
                            best = Some((score, routed(points)));
                            if single && clean {
                                // Nothing between the same sides is shorter
                                // or turns less than the plain route.
                                break;
                            }
                        }
                    }
                    if a == b {
                        continue;
                    }
                }
                let clean = best.as_ref().is_some_and(|b| b.0.crossings == 0);
                if clean && fewest(a, da, b, db) >= best.as_ref().map_or(f32::MAX, |b| b.0.cost) {
                    continue;
                }
                if near.is_none() {
                    near = Some(self.near(ra, rb));
                }
                candidates.clear();
                let around = near.as_deref().unwrap_or_default();
                self.templates((a, da, ra), (b, db, rb), around, !clean, &mut candidates);
                for points in candidates.drain(..) {
                    let points = simplify(points);
                    if !leaves_and_enters(&points, da, db) {
                        continue;
                    }
                    let Some(score) = self.score(&points, best.as_ref().map(|b| b.0), &mut hits)
                    else {
                        if self.budget == 0 {
                            break 'pairs;
                        }
                        continue;
                    };
                    if score.beats(best.as_ref().map(|b| b.0)) {
                        best = Some((score, routed(points)));
                    }
                }
            }
        }
        self.scratch.hits = hits;
        self.scratch.candidates = candidates;
        match best {
            Some((_, routed)) => routed,
            None => {
                // Out of budget before the plain route of the first pair
                // was scored: it is drawn as it is.
                let (a, da, a_shown) = end(from, exit, exit);
                let (b, db, b_shown) = end(to, entry, entry);
                let points = if a == b {
                    vec![a, b]
                } else {
                    elbow(a, da, ra, b, db, rb)
                };
                Routed {
                    points,
                    sides: [a_shown, b_shown],
                    outward: [da, db],
                }
            }
        }
    }

    /// The boxes near the ends, other than the ends' own, whose edges give
    /// a detour its channels: those around both ends together, or around
    /// each end when that area is large, the [`NEAR`] nearest an end kept.
    fn near(&mut self, ra: Rect, rb: Rect) -> Vec<Rect> {
        let margin = 4.0 * ELBOW_MARGIN;
        let area = grow(hull(ra, rb), margin);
        let areas = if Grid::cells_of(area).2 <= MAX_CELLS {
            vec![area]
        } else {
            vec![grow(ra, margin), grow(rb, margin)]
        };
        let mut picked: Vec<u32> = Vec::new();
        for area in areas {
            if !self.obstacles.query(
                area,
                &mut self.scratch.found,
                &mut self.budget,
                &mut self.reads[0],
            ) {
                return Vec::new();
            }
            picked.extend(
                self.scratch
                    .found
                    .iter()
                    .filter(|&&i| !self.own.contains(&Some(i as usize))),
            );
        }
        let rects = &self.obstacles.rects;
        let gap = |i: u32| {
            let r = rects[i as usize];
            let apart = |e: Rect| {
                (r.x - e.right()).max(e.x - r.right()).max(0.0)
                    + (r.y - e.bottom()).max(e.y - r.bottom()).max(0.0)
            };
            apart(ra).min(apart(rb))
        };
        picked.sort_unstable();
        picked.dedup();
        picked.sort_by(|&i, &j| gap(i).total_cmp(&gap(j)).then(i.cmp(&j)));
        picked.truncate(NEAR);
        picked.into_iter().map(|i| rects[i as usize]).collect()
    }

    /// The candidate routes between two sides: the plain route's shape with
    /// its channel moved to each position worth trying (the middle of a
    /// gap, [`ELBOW_MARGIN`] outside a nearby box, [`NUDGE`] either side of
    /// the plain one), and detours pushed past the boxes they cross.
    fn templates(
        &mut self,
        (a, da, ra): (Point, Side, Rect),
        (b, db, rb): (Point, Side, Rect),
        near: &[Rect],
        pushes: bool,
        out: &mut Vec<Vec<Point>>,
    ) {
        let m = ELBOW_MARGIN;
        let f = Frame::of(da);
        let (au, av, bu, bv) = (f.u(a), f.v(a), f.u(b), f.v(b));
        let sa = sign(da);
        let sb = sign(db);
        let edges = |span: fn(Frame, Rect) -> (f32, f32)| -> Vec<f32> {
            near.iter()
                .flat_map(|&r| {
                    let (lo, hi) = span(f, r);
                    [lo - m, hi + m]
                })
                .collect()
        };
        let (edges_u, edges_v) = (edges(Frame::span_u), edges(Frame::span_v));
        let (ra_u, rb_u) = (f.span_u(ra), f.span_u(rb));
        let (ra_v, rb_v) = (f.span_v(ra), f.span_v(rb));
        if da.is_horizontal() == db.is_horizontal() {
            let (qa, qb) = (au + sa * m, bu + sb * m);
            let p3 = move |c: f32| vec![a, f.point(c, av), f.point(c, bv), b];
            let p5 = move |k: f32| {
                vec![
                    a,
                    f.point(qa, av),
                    f.point(qa, k),
                    f.point(qb, k),
                    f.point(qb, bv),
                    b,
                ]
            };
            let beyond = [ra_v.0.min(rb_v.0) - m, ra_v.1.max(rb_v.1) + m];
            if sa == -sb && (bu - au) * sa > 0.0 {
                // Facing sides: a Z with its middle anywhere between the
                // ends, or out, around and back in.
                let mid = (au + bu) / 2.0;
                let mut values = vec![mid, mid - NUDGE, mid + NUDGE, qa, qb];
                values.extend(&edges_u);
                let ok = |c: f32| (c - au) * sa > 0.0 && (c - bu) * sb > 0.0;
                for c in channels(mid, &values, ok, CHANNELS) {
                    out.push(p3(c));
                }
                if (qb - qa) * sa >= 0.0 {
                    let mut values = beyond.to_vec();
                    values.extend(&edges_v);
                    for k in channels(av, &values, |_| true, CHANNELS) {
                        out.push(p5(k));
                    }
                    for dir in [-1.0, 1.0] {
                        if let Some(k) = pushes
                            .then(|| self.push_out(&p5, av, dir, Frame::span_v, f))
                            .flatten()
                        {
                            out.push(p5(k));
                        }
                    }
                }
            } else if sa == -sb {
                // Sides facing away: out of each, across, and back in.
                let k0 = gap_middle(ra_v, rb_v).unwrap_or(ra_v.1.max(rb_v.1) + m);
                let mut values = vec![k0, k0 - NUDGE, k0 + NUDGE];
                values.extend(beyond);
                values.extend(&edges_v);
                for k in channels(k0, &values, |_| true, CHANNELS) {
                    out.push(p5(k));
                }
                for dir in [-1.0, 1.0] {
                    if let Some(k) = pushes
                        .then(|| self.push_out(&p5, k0, dir, Frame::span_v, f))
                        .flatten()
                    {
                        out.push(p5(k));
                    }
                }
            } else {
                // The same side of both: a U beyond the further box, or
                // beyond the boxes in its way.
                let edge = if sa > 0.0 {
                    ra_u.1.max(rb_u.1) + m
                } else {
                    ra_u.0.min(rb_u.0) - m
                };
                let mut values = vec![edge, edge + sa * NUDGE];
                values.extend(&edges_u);
                let ok = |c: f32| (c - edge) * sa >= 0.0;
                for c in channels(edge, &values, ok, CHANNELS) {
                    out.push(p3(c));
                }
                if let Some(c) = pushes
                    .then(|| self.push_out(&p3, edge, sa, Frame::span_u, f))
                    .flatten()
                {
                    out.push(p3(c));
                }
            }
        } else {
            // Perpendicular sides: one turn where the corner allows it,
            // else out of each side and across between them.
            out.push(vec![a, f.point(bu, av), b]);
            let (qa, qb) = (au + sa * m, bv + sb * m);
            let mut first = vec![qa];
            first.extend(gap_middle(ra_u, rb_u));
            first.extend(&edges_u);
            let mut second = vec![qb];
            second.extend(gap_middle(ra_v, rb_v));
            second.extend(&edges_v);
            let firsts = channels(qa, &first, |c| (c - au) * sa > 0.0, PAIR_CHANNELS);
            let seconds = channels(qb, &second, |c| (c - bv) * sb > 0.0, PAIR_CHANNELS);
            for &c1 in &firsts {
                for &c2 in &seconds {
                    out.push(vec![
                        a,
                        f.point(c1, av),
                        f.point(c1, c2),
                        f.point(bu, c2),
                        b,
                    ]);
                }
            }
        }
    }

    /// A channel position for `template` past every box its route crosses,
    /// moving from `start` in direction `dir` along the coordinate `span`
    /// reads, at most [`PUSHES`] times; `None` if it is already clear or
    /// cannot move.
    fn push_out(
        &mut self,
        template: &dyn Fn(f32) -> Vec<Point>,
        start: f32,
        dir: f32,
        span: fn(Frame, Rect) -> (f32, f32),
        f: Frame,
    ) -> Option<f32> {
        let mut c = start;
        let mut hits = Vec::new();
        for _ in 0..PUSHES {
            if !self.hits(&simplify(template(c)), &mut hits) {
                return None;
            }
            if hits.is_empty() {
                return (c != start).then_some(c);
            }
            let past = hits
                .iter()
                .map(|&i| {
                    let (lo, hi) = span(f, self.obstacles.rects[i as usize]);
                    if dir > 0.0 {
                        hi + ELBOW_MARGIN
                    } else {
                        lo - ELBOW_MARGIN
                    }
                })
                .fold(c, |acc, v| if (v - acc) * dir > 0.0 { v } else { acc });
            if past == c || !past.is_finite() {
                break;
            }
            c = past;
        }
        (c != start).then_some(c)
    }

    /// The node boxes the route enters, each once: any but its own, and
    /// its own except by the segment that leaves or arrives at it. False,
    /// with `out` incomplete, when the budget runs out.
    fn hits(&mut self, points: &[Point], out: &mut Vec<u32>) -> bool {
        out.clear();
        let last = points.len().saturating_sub(2);
        let grid = self.obstacles;
        let Scratch {
            found, seen, call, ..
        } = &mut self.scratch;
        if seen.len() < grid.rects.len() {
            seen.resize(grid.rects.len(), 0);
        }
        *call = call.wrapping_add(1);
        if *call == 0 {
            seen.fill(0);
            *call = 1;
        }
        for (i, pair) in points.windows(2).enumerate() {
            let (p, q) = (pair[0], pair[1]);
            let area = grow(segment_box(p, q), CLEARANCE);
            if !grid.query(area, found, &mut self.budget, &mut self.reads[0]) {
                return false;
            }
            for &k in found.iter() {
                let from = self.own[0] == Some(k as usize);
                let to = self.own[1] == Some(k as usize);
                if (from && i == 0) || (to && i == last) || seen[k as usize] == *call {
                    continue;
                }
                let clearance = if from || to { 0.0 } else { CLEARANCE };
                if enters(p, q, grid.rects[k as usize], clearance) {
                    seen[k as usize] = *call;
                    out.push(k);
                }
            }
        }
        true
    }

    /// How many middle segments lie on the centre line of a node within
    /// [`REACH`] along it, and how many segments lie on a line already
    /// drawn; and how many of those lines it crosses.
    /// `None` when the budget runs out.
    fn penalties(&mut self, points: &[Point]) -> Option<(u32, u32)> {
        let mut shared = 0;
        let n = points.len().saturating_sub(1);
        let grid = self.obstacles;
        for (i, pair) in points.windows(2).enumerate() {
            if i == 0 || i + 1 == n {
                continue;
            }
            let (p, q) = (pair[0], pair[1]);
            let (horizontal, vertical) = (p.y == q.y, p.x == q.x);
            if horizontal == vertical {
                continue;
            }
            let s = segment_box(p, q);
            let area = if horizontal {
                Rect::new(
                    s.x - REACH,
                    s.y - LINE_TOLERANCE,
                    s.width + 2.0 * REACH,
                    2.0 * LINE_TOLERANCE,
                )
            } else {
                Rect::new(
                    s.x - LINE_TOLERANCE,
                    s.y - REACH,
                    2.0 * LINE_TOLERANCE,
                    s.height + 2.0 * REACH,
                )
            };
            if !grid.query(
                area,
                &mut self.scratch.found,
                &mut self.budget,
                &mut self.reads[0],
            ) {
                return None;
            }
            let on_centre = self.scratch.found.iter().any(|&k| {
                let c = grid.rects[k as usize].center();
                !self.own.contains(&Some(k as usize))
                    && if horizontal {
                        (c.y - p.y).abs() <= LINE_TOLERANCE
                    } else {
                        (c.x - p.x).abs() <= LINE_TOLERANCE
                    }
            });
            shared += u32::from(on_centre);
        }
        let mut crossed = 0;
        if let Some(lines) = self.lines {
            for pair in points.windows(2) {
                let (p, q) = (pair[0], pair[1]);
                if !lines.near(
                    p,
                    q,
                    None,
                    &mut self.scratch.found,
                    &mut self.budget,
                    &mut self.reads[1],
                ) {
                    return None;
                }
                let mut on_line = false;
                for &k in &self.scratch.found {
                    let (a, b, _) = lines.segments[k as usize];
                    if collinear_overlap(p, q, a, b) > SHARED_LENGTH {
                        on_line = true;
                    } else if crosses(p, q, a, b) {
                        crossed += 1;
                    }
                }
                shared += u32::from(on_line);
            }
        }
        Some((shared, crossed))
    }

    /// The score of a candidate, or `None` when it cannot beat `best` or
    /// the budget runs out.
    fn score(
        &mut self,
        points: &[Point],
        best: Option<Score>,
        hits: &mut Vec<u32>,
    ) -> Option<Score> {
        let turns = points.len().saturating_sub(2) as f32;
        let mut cost = length_of(points) + TURN_COST * turns;
        if best.is_some_and(|b| b.crossings == 0 && cost >= b.cost) {
            return None;
        }
        if !self.hits(points, hits) {
            return None;
        }
        let crossings = hits.len() as u32;
        if best.is_some_and(|b| {
            crossings > b.crossings || (crossings == b.crossings && cost >= b.cost)
        }) {
            return None;
        }
        let (shared, crossed) = self.penalties(points)?;
        cost += SHARED_COST * shared as f32 + CROSSING_COST * crossed as f32;
        Some(Score { crossings, cost })
    }
}

/// The segments of the routes, in order, each with its connector's index;
/// `keep` is given that index and the segment's.
fn segments<'a>(
    routes: impl Iterator<Item = (usize, Option<&'a Routed>)>,
    keep: impl Fn(usize, usize) -> bool,
) -> Vec<(Point, Point, usize)> {
    let mut out = Vec::new();
    for (i, routed) in routes {
        let Some(routed) = routed else {
            continue;
        };
        for (j, pair) in routed.points.windows(2).enumerate() {
            if keep(i, j) {
                out.push((pair[0], pair[1], i));
            }
        }
    }
    out
}

/// The route with middle segment `j` moved aside, as [`separate`] says.
fn nudge(
    router: &mut Router<'_>,
    placed: &mut Lines,
    points: &[Point],
    j: usize,
    owner: usize,
) -> Option<Vec<Point>> {
    let (p, q) = (points[j], points[j + 1]);
    let (before, after) = (points[j - 1], points[j + 2]);
    let horizontal = p.y == q.y;
    let across = |pt: Point| if horizontal { pt.y } else { pt.x };
    let level = |a: Point, b: Point| if horizontal { a.x == b.x } else { a.y == b.y };
    if !(horizontal || p.x == q.x) || !level(before, p) || !level(q, after) {
        return None;
    }
    let mut hits = Vec::new();
    if !router.hits(points, &mut hits) {
        return None;
    }
    let entered = hits.len();
    let shares = |placed: &mut Lines,
                  (budget, reads): (&mut u32, &mut Reads),
                  a: Point,
                  b: Point| { placed.shares(a, b, Some(owner), budget, reads) };
    let shortest = |index: usize| {
        if index == 0 || index + 2 == points.len() {
            ARROW_LENGTH
        } else {
            1.0
        }
    };
    let (lead, trail) = (across(p) - across(before), across(after) - across(q));
    let mut best: Option<((u32, f32, f32), Vec<Point>)> = None;
    for d in [1.0, -1.0, 2.0, -2.0, 3.0, -3.0].map(|n| n * NUDGE) {
        let (new_lead, new_trail) = (lead + d, trail - d);
        if new_lead * lead <= 0.0
            || new_trail * trail <= 0.0
            || new_lead.abs() < shortest(j - 1)
            || new_trail.abs() < shortest(j + 1)
        {
            continue;
        }
        let mut moved = points.to_vec();
        for at in [j, j + 1] {
            if horizontal {
                moved[at].y += d;
            } else {
                moved[at].x += d;
            }
        }
        let (budget, reads) = (&mut router.budget, &mut router.reads[1]);
        if shares(placed, (budget, reads), moved[j], moved[j + 1])?
            || (shares(placed, (budget, reads), moved[j - 1], moved[j])?
                && !shares(placed, (budget, reads), points[j - 1], points[j])?)
            || (shares(placed, (budget, reads), moved[j + 1], moved[j + 2])?
                && !shares(placed, (budget, reads), points[j + 1], points[j + 2])?)
        {
            continue;
        }
        if !router.hits(&moved, &mut hits) {
            return None;
        }
        if hits.len() > entered {
            continue;
        }
        let key = (
            placed.crossings(
                &moved,
                Some(owner),
                &mut router.budget,
                &mut router.reads[1],
            )?,
            d.abs(),
            -new_lead.abs().min(new_trail.abs()),
        );
        if best.as_ref().is_none_or(|(b, _)| key < *b) {
            best = Some((key, moved));
        }
    }
    best.map(|(_, moved)| moved)
}

/// Whether two polylines are the same to the bit, so that `-0.0` and
/// `0.0`, which compare equal, are not taken for one another.
fn same_points(a: &[Point], b: &[Point]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(p, q)| p.x.to_bits() == q.x.to_bits() && p.y.to_bits() == q.y.to_bits())
}

impl Routed {
    /// Whether two routes are the same to the bit: a route kept from an
    /// earlier build stands for a new one only then.
    pub(crate) fn same(&self, other: &Self) -> bool {
        same_points(&self.points, &other.points)
            && self.sides == other.sides
            && self.outward == other.outward
    }

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
        let none = |_: Rect, _: Rect| Vec::new();
        assert!(routing_table(none) > 20_000);
    }

    #[test]
    fn the_routing_table_among_obstacles() {
        // The same table with boxes between the two, beside each, across
        // the gap and around the first, so every detour is exercised.
        let around = |ra: Rect, rb: Rect| {
            let (ca, cb) = (ra.center(), rb.center());
            let mid = Point::new((ca.x + cb.x) / 2.0, (ca.y + cb.y) / 2.0);
            vec![
                Rect::new(mid.x - 20.0, mid.y - 20.0, 40.0, 40.0),
                Rect::new(ra.x - 60.0, ra.y - 60.0, 30.0, 30.0),
                Rect::new(ra.right() + 25.0, ra.y, 10.0, ra.height),
                Rect::new(ra.x, ra.bottom() + 25.0, ra.width, 10.0),
                Rect::new(rb.right() + 5.0, rb.bottom() + 5.0, 50.0, 50.0),
                Rect::new(mid.x - 400.0, mid.y - 2.0, 800.0, 4.0),
            ]
        };
        assert!(routing_table(around) > 20_000);
    }

    /// The boxes of `boxes` that a segment of `points` reaches inside.
    fn entered(points: &[Point], boxes: &[Rect]) -> Vec<usize> {
        (0..boxes.len())
            .filter(|&i| points.windows(2).any(|w| enters(w[0], w[1], boxes[i], 0.0)))
            .collect()
    }

    /// Elbow routes between two floating nodes with a third box in the way
    /// or beside them: the route keeps out of it, rule 4 still holds where
    /// it applies, and a box out of the way changes nothing.
    #[test]
    fn obstacles_between_and_beside_the_ends() {
        use Side::*;
        struct Case {
            name: &'static str,
            a: Rect,
            b: Rect,
            obstacle: Rect,
            /// The sides when rule 4 fixes them.
            sides: Option<[Side; 2]>,
            /// Whether the box is out of the way, so the route is the one
            /// drawn with no box there.
            aside: bool,
        }
        let cases = [
            Case {
                name: "between, in a column",
                a: A,
                b: Rect::new(0.0, 240.0, 120.0, 60.0),
                obstacle: Rect::new(0.0, 120.0, 120.0, 60.0),
                sides: Some([Bottom, Top]),
                aside: false,
            },
            Case {
                name: "between, in a row",
                a: A,
                b: Rect::new(400.0, 0.0, 120.0, 60.0),
                obstacle: Rect::new(200.0, -10.0, 120.0, 80.0),
                sides: Some([Right, Left]),
                aside: false,
            },
            Case {
                name: "wider than both, in a column",
                a: A,
                b: Rect::new(20.0, 300.0, 120.0, 60.0),
                obstacle: Rect::new(-100.0, 140.0, 400.0, 60.0),
                sides: Some([Bottom, Top]),
                aside: false,
            },
            Case {
                name: "across the Z of an offset column",
                a: A,
                b: Rect::new(80.0, 240.0, 120.0, 60.0),
                obstacle: Rect::new(40.0, 130.0, 120.0, 40.0),
                sides: Some([Bottom, Top]),
                aside: false,
            },
            Case {
                name: "at the corner of a diagonal",
                a: A,
                b: Rect::new(300.0, 300.0, 120.0, 60.0),
                obstacle: Rect::new(320.0, 0.0, 80.0, 60.0),
                sides: None,
                aside: false,
            },
            Case {
                name: "on the turn of a diagonal",
                a: A,
                b: Rect::new(300.0, 300.0, 120.0, 60.0),
                obstacle: Rect::new(0.0, 300.0, 200.0, 60.0),
                sides: None,
                aside: false,
            },
            Case {
                name: "beside a column",
                a: A,
                b: Rect::new(0.0, 240.0, 120.0, 60.0),
                obstacle: Rect::new(200.0, 120.0, 120.0, 60.0),
                sides: Some([Bottom, Top]),
                aside: true,
            },
            Case {
                name: "beside a row",
                a: A,
                b: Rect::new(400.0, 0.0, 120.0, 60.0),
                obstacle: Rect::new(200.0, 200.0, 120.0, 60.0),
                sides: Some([Right, Left]),
                aside: true,
            },
            Case {
                name: "beside a diagonal",
                a: A,
                b: Rect::new(300.0, 300.0, 120.0, 60.0),
                obstacle: Rect::new(-300.0, 400.0, 120.0, 60.0),
                sides: None,
                aside: true,
            },
        ];
        for case in cases {
            let name = case.name;
            let obstacles = Obstacles::new([case.a, case.b, case.obstacle]);
            for &shape in Shape::ALL {
                let end = |bounds| Terminal::Node {
                    bounds,
                    shape,
                    attach: Attach::Floating,
                };
                let (from, to) = (end(case.a), end(case.b));
                let routed = route_among(Route::Elbow, &from, &to, &obstacles, [Some(0), Some(1)]);
                assert_sound(Route::Elbow, &from, &to, &routed, name);
                let pts = &routed.points;
                assert!(
                    entered(pts, &[case.obstacle]).is_empty(),
                    "{name}, {shape:?}: {pts:?} runs through the box"
                );
                if let Some(sides) = case.sides {
                    assert_eq!(routed.sides, sides.map(Some), "{name}, {shape:?}");
                }
                let [da, db] = routed.sides.map(|s| s.unwrap());
                assert!(leaves_and_enters(pts, da, db), "{name}, {shape:?}: {pts:?}");
                if case.aside {
                    assert_eq!(
                        *pts,
                        route(Route::Elbow, &from, &to).points,
                        "{name}, {shape:?}"
                    );
                }
                // A straight route is the same with or without the box.
                assert_eq!(
                    route_among(Route::Straight, &from, &to, &obstacles, [Some(0), Some(1)]),
                    route(Route::Straight, &from, &to),
                    "{name}, {shape:?}"
                );
            }
        }
    }

    /// A box closed in on every side by touching boxes: every candidate
    /// crosses one, and the best is still drawn, sound and the same each
    /// time.
    #[test]
    fn a_route_is_drawn_when_avoidance_is_impossible() {
        let walls = [
            Rect::new(-120.0, -60.0, 360.0, 60.0),
            Rect::new(-120.0, 60.0, 360.0, 60.0),
            Rect::new(-120.0, 0.0, 120.0, 60.0),
            Rect::new(120.0, 0.0, 120.0, 60.0),
        ];
        let b = Rect::new(600.0, 400.0, 120.0, 60.0);
        let mut boxes = vec![A, b];
        boxes.extend(walls);
        let obstacles = Obstacles::new(boxes.clone());
        for (fa, fb) in [
            (Attach::Floating, Attach::Floating),
            (Attach::Side(Side::Left), Attach::Floating),
            (Attach::Anchor([500, 500]), Attach::Side(Side::Top)),
        ] {
            let (from, to) = (node(A, fa), node(b, fb));
            let routed = route_among(Route::Elbow, &from, &to, &obstacles, [Some(0), Some(1)]);
            assert_sound(Route::Elbow, &from, &to, &routed, "walled in");
            assert!(!entered(&routed.points, &walls).is_empty(), "{routed:?}");
            assert_eq!(
                routed,
                route_among(Route::Elbow, &from, &to, &obstacles, [Some(0), Some(1)])
            );
            let all = route_all(
                &boxes,
                &[Connector {
                    kind: Route::Elbow,
                    ends: [from, to],
                    nodes: [Some(0), Some(1)],
                }],
            );
            assert_eq!(all, [routed]);
        }
    }

    /// A query of the box lookup, against its definition: every box
    /// touching the area once, and a cost of one per cell visited plus one
    /// per box listed in it and per box too large for cells (or one per box
    /// when the area covers more cells than there are boxes), all of it
    /// spent or, when it is more than the budget holds, the budget emptied.
    /// Boxes close together (kept in an array), far apart (in the map),
    /// large, and added after the lookup was made.
    #[test]
    fn a_lookup_finds_each_touching_box_once_at_its_cost() {
        let mut rng = Rng(5);
        for round in 0..400 {
            let spread = *rng.pick(&[600, 3_000, 40_000, 4_000_000]);
            let rect = |rng: &mut Rng| {
                let wide = if rng.one_in(12) { 20_000 } else { 400 };
                Rect::new(
                    rng.below(spread) as f32 - 300.0,
                    rng.below(spread) as f32 - 300.0,
                    rng.below(wide) as f32,
                    rng.below(400) as f32,
                )
            };
            let made: Vec<Rect> = (0..rng.below(60)).map(|_| rect(&mut rng)).collect();
            let mut grid = Grid::new(made.iter().copied());
            let mut all = made;
            for _ in 0..rng.below(20) {
                let added = rect(&mut rng);
                grid.insert(added);
                all.push(added);
            }
            let spans: Vec<Span> = all.iter().map(|&r| Grid::cells_of(r)).collect();
            for _ in 0..30 {
                let area = rect(&mut rng);
                let (x, y, count) = Grid::cells_of(area);
                let cost = if count > all.len() as i64 {
                    all.len() as u64
                } else {
                    let listed = |(sx, sy, n): Span| {
                        let across = (sx[1].min(x[1]) - sx[0].max(x[0]) + 1).max(0);
                        let down = (sy[1].min(y[1]) - sy[0].max(y[0]) + 1).max(0);
                        if n > MAX_CELLS {
                            1
                        } else {
                            (across * down) as u64
                        }
                    };
                    count as u64 + spans.iter().map(|&span| listed(span)).sum::<u64>()
                };
                let start = rng.below(cost as usize * 2 + 2) as u32;
                let mut budget = start;
                let mut found = Vec::new();
                let complete = grid.query(area, &mut found, &mut budget, &mut Reads::default());
                let case = format!("{round}: {area:?} with {start} of {cost}");
                if cost > u64::from(start) {
                    assert!(!complete && budget == 0, "{case}");
                    continue;
                }
                assert!(complete, "{case}");
                assert_eq!(u64::from(start - budget), cost, "{case}");
                found.sort_unstable();
                let touching: Vec<u32> = (0..all.len() as u32)
                    .filter(|&i| touches(all[i as usize], area))
                    .collect();
                assert_eq!(found, touching, "{case}");
            }
        }
    }

    /// The work of routing one connector does not grow with the board:
    /// among thousands of boxes stacked on one spot, a lookup there costs
    /// more than the budget allows, so each connector keeps its plain
    /// route. Before the budget, each connector searched every one of the
    /// boxes for every candidate, and this took minutes.
    #[test]
    fn a_crowded_connector_keeps_its_plain_route() {
        let mut boxes: Vec<Rect> = (0..5_000)
            .map(|i| Rect::new((i % 7) as f32, (i % 5) as f32, 100.0, 60.0))
            .collect();
        let far = boxes.len();
        boxes.push(Rect::new(5_000.0, 5_000.0, 100.0, 60.0));
        let connectors: Vec<Connector> = (0..40)
            .map(|i| Connector {
                kind: Route::Elbow,
                ends: [
                    node(boxes[i], Attach::Floating),
                    node(boxes[far], Attach::Floating),
                ],
                nodes: [Some(i), Some(far)],
            })
            .collect();
        let routes = route_all(&boxes, &connectors);
        for (c, routed) in connectors.iter().zip(&routes) {
            let (ra, rb) = (c.ends[0].bounds(), c.ends[1].bounds());
            let (a, da, _) = end(&c.ends[0], Side::Bottom, Side::Bottom);
            let (b, db, _) = end(&c.ends[1], Side::Top, Side::Top);
            assert_eq!(routed.points, elbow(a, da, ra, b, db, rb));
            assert_eq!(routed.sides, [Some(Side::Bottom), Some(Side::Top)]);
        }
    }

    #[test]
    fn a_middle_segment_moves_off_a_nodes_centre_line() {
        // A Z down from A to B, offset: its middle would run at y = 150,
        // the centre line of C beside it, where C's connectors run.
        let b = Rect::new(60.0, 240.0, 120.0, 60.0);
        let c = Rect::new(200.0, 120.0, 120.0, 60.0);
        let alone = route(
            Route::Elbow,
            &node(A, Attach::Floating),
            &node(b, Attach::Floating),
        );
        assert_eq!(alone.points[1].y, 150.0);
        let obstacles = Obstacles::new([A, b, c]);
        let routed = route_among(
            Route::Elbow,
            &node(A, Attach::Floating),
            &node(b, Attach::Floating),
            &obstacles,
            [Some(0), Some(1)],
        );
        assert_eq!(
            routed.points,
            [
                p(60.0, 60.0),
                p(60.0, 140.0),
                p(120.0, 140.0),
                p(120.0, 240.0)
            ]
        );
        // Far enough along the line, the node no longer counts.
        let far = Rect::new(60.0 + 120.0 + REACH + 1.0, 120.0, 120.0, 60.0);
        let obstacles = Obstacles::new([A, b, far]);
        let routed = route_among(
            Route::Elbow,
            &node(A, Attach::Floating),
            &node(b, Attach::Floating),
            &obstacles,
            [Some(0), Some(1)],
        );
        assert_eq!(routed.points, alone.points);
    }

    #[test]
    fn connectors_on_one_line_move_apart() {
        // Two Zs down between two rows that cross over: both middles fall
        // at y = 130 and overlap from x = 90 to 160. The second moves a
        // nudge aside; ends and sides stay.
        let boxes = [
            A,
            Rect::new(100.0, 200.0, 120.0, 60.0),
            Rect::new(130.0, 0.0, 120.0, 60.0),
            Rect::new(30.0, 200.0, 120.0, 60.0),
        ];
        let connector = |i: usize, j: usize| Connector {
            kind: Route::Elbow,
            ends: [
                node(boxes[i], Attach::Floating),
                node(boxes[j], Attach::Floating),
            ],
            nodes: [Some(i), Some(j)],
        };
        let routes = route_all(&boxes, &[connector(0, 1), connector(2, 3)]);
        assert_eq!(
            routes[0].points,
            [
                p(60.0, 60.0),
                p(60.0, 130.0),
                p(160.0, 130.0),
                p(160.0, 200.0)
            ]
        );
        assert_eq!(
            routes[1].points,
            [
                p(190.0, 60.0),
                p(190.0, 140.0),
                p(90.0, 140.0),
                p(90.0, 200.0)
            ]
        );
        for routed in &routes {
            assert_eq!(routed.sides, [Some(Side::Bottom), Some(Side::Top)]);
        }
        // Each alone keeps the middle of the gap.
        let alone = route_all(&boxes, &[connector(2, 3)]);
        assert_eq!(alone[0].points[1].y, 130.0);
    }

    #[test]
    fn a_free_connector_keeps_off_the_lines_of_fixed_ones() {
        // C to D runs straight down a column. A to B, apart on both axes,
        // would otherwise turn into B's top beside that line, cross it, or
        // share D's top; it takes a route that does none of these.
        let boxes = [
            Rect::new(200.0, 0.0, 120.0, 60.0),
            Rect::new(0.0, 240.0, 120.0, 60.0),
            Rect::new(0.0, 0.0, 120.0, 60.0),
            Rect::new(0.0, 120.0, 120.0, 60.0),
        ];
        let floating = |i: usize| node(boxes[i], Attach::Floating);
        let routes = route_all(
            &boxes,
            &[
                Connector {
                    kind: Route::Elbow,
                    ends: [floating(2), floating(3)],
                    nodes: [Some(2), Some(3)],
                },
                Connector {
                    kind: Route::Elbow,
                    ends: [floating(0), floating(1)],
                    nodes: [Some(0), Some(1)],
                },
            ],
        );
        assert_eq!(routes[0].points, [p(60.0, 60.0), p(60.0, 120.0)]);
        let free = &routes[1].points;
        assert!(entered(free, &boxes[2..]).is_empty(), "{free:?}");
        for w in free.windows(2) {
            assert!(
                !crosses(w[0], w[1], p(60.0, 60.0), p(60.0, 120.0)),
                "{free:?}"
            );
            assert!(collinear_overlap(w[0], w[1], p(60.0, 60.0), p(60.0, 120.0)) == 0.0);
        }
        assert_eq!(routes[1].sides, [Some(Side::Bottom), Some(Side::Right)]);
    }

    /// The length that two connectors' routes run on one line together.
    fn shared_length(a: &Routed, b: &Routed) -> f32 {
        a.points
            .windows(2)
            .flat_map(|w| {
                b.points
                    .windows(2)
                    .map(move |v| collinear_overlap(w[0], w[1], v[0], v[1]))
            })
            .sum()
    }

    /// Connectors that each choose a side see the ones chosen before them:
    /// three from one node to a column below and to its right no longer
    /// share a trunk out of its bottom (170 and 370 px did), and a pair
    /// each way between two boxes that others force into a Z no longer
    /// share their first and last segments (190 px did).
    #[test]
    fn free_connectors_keep_off_each_others_lines() {
        let fan = [
            A,
            Rect::new(300.0, 200.0, 120.0, 60.0),
            Rect::new(300.0, 400.0, 120.0, 60.0),
            Rect::new(300.0, 600.0, 120.0, 60.0),
        ];
        let square = |x: f32, y: f32| Rect::new(x, y, 100.0, 100.0);
        let pair = [
            square(0.0, 0.0),
            square(300.0, 300.0),
            square(300.0, 0.0),
            square(0.0, 300.0),
        ];
        for (name, boxes, ends) in [
            ("fan-out", &fan, [(0, 1), (0, 2), (0, 3)].as_slice()),
            ("both ways", &pair, [(0, 1), (1, 0)].as_slice()),
        ] {
            let connectors: Vec<Connector> = ends
                .iter()
                .map(|&(i, j)| Connector {
                    kind: Route::Elbow,
                    ends: [
                        node(boxes[i], Attach::Floating),
                        node(boxes[j], Attach::Floating),
                    ],
                    nodes: [Some(i), Some(j)],
                })
                .collect();
            let routes = route_all(boxes, &connectors);
            assert_eq!(routes, route_all(boxes, &connectors), "{name}");
            for (k, (c, routed)) in connectors.iter().zip(&routes).enumerate() {
                assert_sound(Route::Elbow, &c.ends[0], &c.ends[1], routed, name);
                let others: Vec<Rect> = (0..boxes.len())
                    .filter(|&i| c.nodes[0] != Some(i) && c.nodes[1] != Some(i))
                    .map(|i| boxes[i])
                    .collect();
                assert!(
                    entered(&routed.points, &others).is_empty(),
                    "{name}: {routed:?}"
                );
                for other in &routes[..k] {
                    assert!(
                        shared_length(routed, other) <= SHARED_LENGTH,
                        "{name}: {routed:?} on {other:?}"
                    );
                }
            }
        }
    }

    /// Random boxes and connectors of every end form: routing all of them
    /// is deterministic, finite and sound, honours pinned sides, and keeps
    /// rule 4 for floating ends.
    fn routes_hold(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        let coordinate = |rng: &mut Rng| match rng.below(8) {
            0 => *rng.pick(&[0.0, -16_777_216.0, 16_777_215.0, 0.5]),
            _ => rng.below(1_200) as f32 - 200.0,
        };
        for round in 0..count {
            let boxes: Vec<Rect> = (0..1 + rng.below(14))
                .map(|_| {
                    let (x, y) = (coordinate(&mut rng), coordinate(&mut rng));
                    let wide = if rng.one_in(10) { 4_000 } else { 200 };
                    let w = 1.0 + rng.below(wide) as f32;
                    let h = 1.0 + rng.below(200) as f32;
                    Rect::new(x, y, w, h)
                })
                .collect();
            let connectors: Vec<Connector> = (0..rng.below(12))
                .map(|_| {
                    let end = |rng: &mut Rng| {
                        if rng.one_in(6) {
                            (
                                Terminal::Point(Point::new(coordinate(rng), coordinate(rng))),
                                None,
                            )
                        } else {
                            let i = rng.below(boxes.len());
                            let attach = match rng.below(4) {
                                0 => Attach::Side(*rng.pick(Side::ALL)),
                                1 => Attach::Anchor([
                                    rng.below(1_200) as i32 - 100,
                                    rng.below(1_200) as i32 - 100,
                                ]),
                                _ => Attach::Floating,
                            };
                            let shape = *rng.pick(Shape::ALL);
                            (
                                Terminal::Node {
                                    bounds: boxes[i],
                                    shape,
                                    attach,
                                },
                                Some(i),
                            )
                        }
                    };
                    let (from, a) = end(&mut rng);
                    let (to, b) = end(&mut rng);
                    let kind = if rng.one_in(4) {
                        Route::Straight
                    } else {
                        Route::Elbow
                    };
                    Connector {
                        kind,
                        ends: [from, to],
                        nodes: [a, b],
                    }
                })
                .collect();
            let routes = route_all(&boxes, &connectors);
            assert_eq!(routes, route_all(&boxes, &connectors), "{round}");
            assert_eq!(routes.len(), connectors.len());
            for (c, routed) in connectors.iter().zip(&routes) {
                let case = format!("{round}: {c:?}");
                assert_sound(c.kind, &c.ends[0], &c.ends[1], routed, &case);
                let (ra, rb) = (c.ends[0].bounds(), c.ends[1].bounds());
                let held = !apart_on_both_axes(ra, rb);
                let exit = nominal_exit(ra, rb);
                for (k, nominal) in [(0, exit), (1, exit.opposite())] {
                    if let Terminal::Node {
                        attach: Attach::Floating,
                        ..
                    } = c.ends[k]
                    {
                        if held || c.kind == Route::Straight {
                            assert_eq!(routed.sides[k], Some(nominal), "{case}: {routed:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn random_routes_hold() {
        routes_hold(300, 11);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_random_routes_hold() {
        routes_hold(20_000, 12);
    }

    /// Every pair of ends (floating, the four pinned sides, anchors) on
    /// every layout, every shape, both routes, with a free end too, among
    /// the boxes `others` places for the pair. Returns the node pairs run.
    fn routing_table(others: impl Fn(Rect, Rect) -> Vec<Rect>) -> usize {
        let mut cases = 0;
        for (name, ra, rb) in layouts() {
            let mut boxes = vec![ra, rb];
            boxes.extend(others(ra, rb));
            let obstacles = Obstacles::new(boxes);
            let route = |kind, from: &Terminal, to: &Terminal| {
                let own = |t: &Terminal, i| matches!(t, Terminal::Node { .. }).then_some(i);
                let routed = route_among(kind, from, to, &obstacles, [own(from, 0), own(to, 1)]);
                assert_eq!(
                    routed,
                    route_among(kind, from, to, &obstacles, [own(from, 0), own(to, 1)]),
                    "routing is deterministic"
                );
                routed
            };
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
        cases
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
        // A free end routes as a point, leaves by its nominal side and
        // reports no side. Apart on both axes, the floating end may take
        // any side, and one turn into A's right side beats a Z into its
        // bottom.
        let free = route(
            Route::Elbow,
            &Terminal::Point(p(300.0, 200.0)),
            &node(A, Attach::Floating),
        );
        assert_eq!(free.sides, [None, Some(Side::Right)]);
        assert_eq!(
            free.points,
            [p(300.0, 200.0), p(300.0, 30.0), p(120.0, 30.0)]
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
