//! Which renderer layer each scene item is painted in.
//!
//! Both renderers paint a layer's shapes first and its text last, so a
//! label that comes before a filled shape covering it would show through
//! that shape. A fill is therefore painted one layer above any earlier
//! label it overlaps, and every other item stays as low as the earlier
//! items it overlaps allow. The count is then the depth of the deepest
//! stack of labels and fills, not the number of overlaps on the board.
//!
//! Two approximations keep that depth from spreading through a connected
//! diagram. Items that overlap by less than two units count as touching,
//! so a connector does not carry the layer of the shape it starts at to
//! the shape it ends at. And a stroke crossing an earlier label stays in
//! the label's layer and is painted under it: lifting it would cost a
//! layer for a line 1.5 units wide.

use std::collections::HashMap;

use wren_board::scene::{Draw, Item, Point, Rect, Segment};

/// The most layers a scene is painted in. Each layer with shapes costs
/// wgpu three passes, so a pathological stack shares the last layer and
/// its labels show through there.
pub const CAP: usize = 4;

/// The side of a cell of the overlap index, in board units.
const CELL: f32 = 256.0;

/// Footprints spanning more cells than this on a side are checked against
/// every later item instead of being indexed.
const WIDE: i64 = 16;

/// How far a footprint is drawn in from an item's bounds: half the widest
/// stroke and a little more, so outlines that meet do not overlap.
const TOUCH: f32 = 1.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The layer of each item, parallel to the scene's items.
    pub of: Vec<u8>,
    /// How many layers are used; 0 for an empty scene.
    pub count: usize,
    /// Fills that cover an earlier label and still share its layer,
    /// because the cap was reached. Those labels show through.
    pub overflow: usize,
}

impl Plan {
    /// The indices of the items painted in `layer`, back to front.
    pub fn items(&self, layer: usize) -> impl Iterator<Item = usize> + '_ {
        self.of
            .iter()
            .enumerate()
            .filter(move |(_, of)| usize::from(**of) == layer)
            .map(|(index, _)| index)
    }
}

/// Assign each item of a scene, back to front, to a layer of at most
/// `cap` (at least one).
pub fn split(items: &[Item], cap: usize) -> Plan {
    let last = u8::try_from(cap.clamp(1, usize::from(u8::MAX)) - 1).unwrap_or(u8::MAX);
    let mut index = Index::default();
    let mut plan = Plan {
        of: Vec::with_capacity(items.len()),
        ..Plan::default()
    };
    let mut footprint = Vec::new();
    for (position, item) in items.iter().enumerate() {
        let covers = matches!(&item.draw, Draw::Path { fill: Some(_), .. });
        let mut layer = 0u8;
        let mut overflowed = false;
        footprint.clear();
        footprints(item, &mut footprint);
        for area in &footprint {
            index.overlapping(*area, |earlier, earlier_area| {
                if !intersects(earlier_area, *area) {
                    return;
                }
                let below = plan.of[earlier];
                if covers && matches!(&items[earlier].draw, Draw::Text { .. }) {
                    overflowed |= below == last;
                    layer = layer.max(below.saturating_add(1).min(last));
                } else {
                    layer = layer.max(below);
                }
            });
        }
        plan.overflow += usize::from(overflowed);
        plan.count = plan.count.max(usize::from(layer) + 1);
        plan.of.push(layer);
        for area in &footprint {
            index.insert(position, *area);
        }
    }
    plan
}

/// The areas an item is taken to cover: its bounds drawn in by [`TOUCH`],
/// or, for a path that is only stroked, each segment's own box, so an
/// elbow does not claim the rectangle it bends around.
fn footprints(item: &Item, areas: &mut Vec<Rect>) {
    match &item.draw {
        Draw::Path {
            segments,
            fill: None,
            stroke: Some(_),
        } => {
            let mut start = None;
            let mut from = None;
            for segment in segments {
                let (to, via): (Point, [Option<Point>; 2]) = match *segment {
                    Segment::MoveTo(to) => {
                        start = Some(to);
                        from = Some(to);
                        continue;
                    }
                    Segment::LineTo(to) => (to, [None, None]),
                    Segment::QuadTo { control, to } => (to, [Some(control), None]),
                    Segment::CubicTo { first, second, to } => (to, [Some(first), Some(second)]),
                    Segment::Close => match start {
                        Some(start) => (start, [None, None]),
                        None => continue,
                    },
                };
                if let Some(from) = from {
                    areas.push(
                        via.into_iter()
                            .flatten()
                            .chain([to])
                            .fold(spot(from), |area, point| union(area, spot(point))),
                    );
                }
                from = Some(to);
            }
        }
        Draw::Path {
            fill: None,
            stroke: None,
            ..
        } => {}
        Draw::Path { .. } | Draw::Text { .. } => areas.push(drawn_in(item.bounds)),
    }
}

fn spot(point: Point) -> Rect {
    Rect {
        x: point.x,
        y: point.y,
        width: 0.0,
        height: 0.0,
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}

/// A rectangle drawn in by [`TOUCH`] on each side, down to its middle
/// line and no further.
fn drawn_in(bounds: Rect) -> Rect {
    let dx = TOUCH.min(bounds.width / 2.0);
    let dy = TOUCH.min(bounds.height / 2.0);
    Rect {
        x: bounds.x + dx,
        y: bounds.y + dy,
        width: bounds.width - 2.0 * dx,
        height: bounds.height - 2.0 * dy,
    }
}

/// Whether two areas overlap. Shared edges do not count, and an area with
/// no thickness, a line, overlaps what it passes through.
fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// A uniform grid over the footprints seen so far.
#[derive(Default)]
struct Index {
    cells: HashMap<(i64, i64), Vec<(usize, Rect)>>,
    wide: Vec<(usize, Rect)>,
}

impl Index {
    fn insert(&mut self, item: usize, area: Rect) {
        match span(area) {
            Some((columns, rows)) => {
                for column in columns.0..=columns.1 {
                    for row in rows.0..=rows.1 {
                        self.cells
                            .entry((column, row))
                            .or_default()
                            .push((item, area));
                    }
                }
            }
            None => self.wide.push((item, area)),
        }
    }

    /// Every earlier footprint that may overlap `area`; one in several
    /// shared cells is reported once per cell.
    fn overlapping(&self, area: Rect, mut each: impl FnMut(usize, Rect)) {
        let mut each = |entry: &(usize, Rect)| each(entry.0, entry.1);
        self.wide.iter().for_each(&mut each);
        match span(area) {
            Some((columns, rows)) => {
                for column in columns.0..=columns.1 {
                    for row in rows.0..=rows.1 {
                        if let Some(cell) = self.cells.get(&(column, row)) {
                            cell.iter().for_each(&mut each);
                        }
                    }
                }
            }
            None => self.cells.values().flatten().for_each(&mut each),
        }
    }
}

/// The inclusive cell ranges an area covers, or `None` when it is too
/// large to index (or not finite).
fn span(area: Rect) -> Option<((i64, i64), (i64, i64))> {
    let cell = |value: f32| -> Option<i64> {
        // Scene geometry is clamped far inside the range an i64 holds.
        value.is_finite().then(|| (value / CELL).floor() as i64)
    };
    let columns = (cell(area.x)?, cell(area.x + area.width)?);
    let rows = (cell(area.y)?, cell(area.y + area.height)?);
    (columns.1 - columns.0 < WIDE && rows.1 - rows.0 < WIDE).then_some((columns, rows))
}

#[cfg(test)]
mod tests {
    use wren_board::scene::{Face, Line, Source, Stroke};
    use wren_board::{Id, Rgba};

    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn point(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn source() -> Source {
        Source::Node(Id::new("n").expect("a valid id"))
    }

    fn fill(bounds: Rect) -> Item {
        Item {
            source: source(),
            bounds,
            draw: Draw::Path {
                segments: Vec::new(),
                fill: Some(Rgba::rgb(1, 2, 3)),
                stroke: None,
            },
        }
    }

    /// A stroked line through `points`.
    fn stroke(points: &[(f32, f32)]) -> Item {
        let bounds = points
            .iter()
            .map(|(x, y)| rect(x - 0.75, y - 0.75, 1.5, 1.5))
            .reduce(union)
            .unwrap_or_default();
        let mut segments = vec![Segment::MoveTo(point(points[0].0, points[0].1))];
        segments.extend(
            points[1..]
                .iter()
                .map(|(x, y)| Segment::LineTo(point(*x, *y))),
        );
        Item {
            source: source(),
            bounds,
            draw: Draw::Path {
                segments,
                fill: None,
                stroke: Some(Stroke {
                    color: Rgba::rgb(1, 2, 3),
                    width: 1.5,
                    dash: None,
                }),
            },
        }
    }

    fn label(bounds: Rect) -> Item {
        Item {
            source: source(),
            bounds,
            draw: Draw::Text {
                lines: vec![Line {
                    text: "label".into(),
                    origin: point(bounds.x, bounds.y),
                    width: bounds.width,
                }],
                face: Face::Regular,
                size: 14.0,
                color: Rgba::rgb(0, 0, 0),
            },
        }
    }

    /// A shape at `x` with its label inside it.
    fn node(x: f32) -> [Item; 2] {
        [
            fill(rect(x, 0.0, 160.0, 64.0)),
            label(rect(x + 20.0, 12.0, 120.0, 40.0)),
        ]
    }

    #[test]
    fn shapes_that_do_not_cover_a_label_share_one_layer() {
        let items: Vec<Item> = (0..50).flat_map(|i| node(i as f32 * 200.0)).collect();
        let plan = split(&items, CAP);
        assert_eq!(plan.count, 1);
        assert!(plan.of.iter().all(|layer| *layer == 0));
        assert_eq!(plan.overflow, 0);
        assert_eq!(split(&[], CAP), Plan::default());
    }

    #[test]
    fn a_fill_over_an_earlier_label_moves_up_with_its_own_label() {
        // The second shape covers the right half of the first one's label.
        let items: Vec<Item> = [node(0.0), node(100.0)].into_iter().flatten().collect();
        let plan = split(&items, CAP);
        assert_eq!(plan.of, [0, 0, 1, 1]);
        assert_eq!(plan.count, 2);
        assert_eq!(plan.items(1).collect::<Vec<_>>(), [2, 3]);
    }

    #[test]
    fn overlaps_far_apart_do_not_add_up() {
        let mut items = Vec::new();
        for pair in 0..40 {
            let x = pair as f32 * 1000.0;
            items.extend(node(x));
            items.extend(node(x + 100.0));
        }
        let plan = split(&items, CAP);
        assert_eq!(plan.count, 2);
        assert_eq!(plan.overflow, 0);
    }

    #[test]
    fn a_connector_does_not_carry_a_layer_to_the_shape_it_reaches() {
        // Pairs of covering shapes along a row, each joined to the next
        // pair by a connector from edge to edge, as a flowchart is.
        let mut items = Vec::new();
        for pair in 0..10 {
            let x = pair as f32 * 600.0;
            items.extend(node(x));
            items.extend(node(x + 100.0));
            items.push(stroke(&[
                (x + 260.0, 32.0),
                (x + 430.0, 32.0),
                (x + 430.0, 50.0),
                (x + 590.0, 50.0),
            ]));
            // The arrowhead, its tip on the next shape's edge.
            items.push(fill(rect(x + 590.0, 46.0, 10.0, 8.0)));
        }
        let plan = split(&items, CAP);
        assert_eq!(plan.count, 2);
        assert_eq!(plan.overflow, 0);
        assert_eq!(&plan.of[..6], [0, 0, 1, 1, 0, 0]);
    }

    #[test]
    fn a_stack_deeper_than_the_cap_shares_the_last_layer() {
        let items: Vec<Item> = (0..6).flat_map(|i| node(i as f32 * 10.0)).collect();
        let plan = split(&items, 4);
        assert_eq!(plan.of, [0, 0, 1, 1, 2, 2, 3, 3, 3, 3, 3, 3]);
        assert_eq!(plan.count, 4);
        assert_eq!(plan.overflow, 2);
        let one = split(&items, 1);
        assert_eq!(one.count, 1);
        assert_eq!(one.overflow, 5);
        // A cap of zero is one layer, not none.
        assert_eq!(split(&items, 0).count, 1);
    }

    #[test]
    fn a_later_item_never_goes_below_one_it_overlaps() {
        let items = vec![
            label(rect(0.0, 0.0, 100.0, 20.0)),
            fill(rect(50.0, 0.0, 100.0, 20.0)),
            // Crosses the raised fill only, and is drawn after it.
            stroke(&[(120.0, 10.0), (220.0, 10.0)]),
            label(rect(200.0, 0.0, 100.0, 20.0)),
            fill(rect(400.0, 0.0, 100.0, 20.0)),
        ];
        assert_eq!(split(&items, CAP).of, [0, 1, 1, 1, 0]);
    }

    #[test]
    fn a_stroke_over_a_label_stays_in_its_layer() {
        let items = vec![
            label(rect(0.0, 0.0, 100.0, 20.0)),
            stroke(&[(-500.0, 5.0), (1500.0, 5.0)]),
        ];
        assert_eq!(split(&items, CAP).count, 1);
    }

    #[test]
    fn an_elbow_covers_its_segments_and_not_the_corner_it_turns() {
        let items = vec![
            label(rect(0.0, 0.0, 100.0, 20.0)),
            fill(rect(50.0, 0.0, 100.0, 20.0)),
            // Bends around the raised fill without touching it.
            stroke(&[(40.0, 40.0), (200.0, 40.0), (200.0, -40.0)]),
            // Crosses it.
            stroke(&[(100.0, -40.0), (100.0, 40.0)]),
        ];
        assert_eq!(split(&items, CAP).of, [0, 1, 0, 1]);
    }

    #[test]
    fn bounds_that_touch_or_barely_overlap_do_not_count() {
        let items = vec![
            label(rect(0.0, 0.0, 100.0, 20.0)),
            fill(rect(100.0, 0.0, 100.0, 20.0)),
            fill(rect(0.0, 20.0, 100.0, 20.0)),
            // Outlines that meet overlap by their stroke.
            fill(rect(98.5, -30.0, 100.0, 31.5)),
        ];
        assert_eq!(split(&items, CAP).count, 1);
        let items = vec![
            label(rect(0.0, 0.0, 100.0, 20.0)),
            fill(rect(97.0, 0.0, 100.0, 20.0)),
        ];
        assert_eq!(split(&items, CAP).count, 2);
    }

    #[test]
    fn huge_and_non_finite_bounds_are_still_ordered() {
        let items = vec![
            label(rect(0.0, 0.0, 100.0, 20.0)),
            // Too wide to index: found through the wide list.
            fill(rect(-1.0e7, -1.0e7, 2.0e7, 2.0e7)),
            label(rect(9000.0, 9000.0, 100.0, 20.0)),
            fill(rect(9050.0, 9000.0, 100.0, 20.0)),
            fill(rect(f32::NAN, 0.0, f32::INFINITY, 1.0)),
        ];
        let plan = split(&items, CAP);
        assert_eq!(plan.of, [0, 1, 1, 2, 0]);
    }

    #[test]
    fn the_index_agrees_with_checking_every_pair() {
        // A seeded scatter of shapes, labels and lines, dense enough to stack.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |range: f32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f32 / 10_000.0 * range
        };
        let mut items = Vec::new();
        for index in 0..400 {
            let (x, y) = (next(1200.0), next(1200.0));
            let (width, height) = (20.0 + next(300.0), 10.0 + next(90.0));
            items.push(match index % 3 {
                0 => fill(rect(x, y, width, height)),
                1 => label(rect(x, y, width, height)),
                _ => stroke(&[(x, y), (x + width, y), (x + width, y + height)]),
            });
        }
        let cap = 6;
        let areas: Vec<Vec<Rect>> = items
            .iter()
            .map(|item| {
                let mut areas = Vec::new();
                footprints(item, &mut areas);
                areas
            })
            .collect();
        let mut expected: Vec<u8> = Vec::new();
        for (position, item) in items.iter().enumerate() {
            let covers = matches!(&item.draw, Draw::Path { fill: Some(_), .. });
            let mut layer = 0;
            for earlier in 0..position {
                let overlap = areas[earlier]
                    .iter()
                    .any(|a| areas[position].iter().any(|b| intersects(*a, *b)));
                if !overlap {
                    continue;
                }
                let label = matches!(&items[earlier].draw, Draw::Text { .. });
                let above = u8::from(covers && label);
                layer = layer.max((expected[earlier] + above).min(cap - 1));
            }
            expected.push(layer);
        }
        let plan = split(&items, usize::from(cap));
        assert_eq!(plan.of, expected);
        assert!(plan.count > 2, "the scatter should stack");
    }
}
