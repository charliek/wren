//! The handles a selection shows (the plan's §5.3 rule 4, §5.5 rule 8 and
//! §5.8 rule 5): where each sits and how near a press must be to take it.
//! Positions are widget-local pixels and sizes are screen pixels, so a
//! handle is as easy to take at any zoom.

use wren_board::scene::{Draw, Point, Rect, Scene, Segment, Source};
use wren_board::{route, Board, Id, Side};

use crate::Viewport;

/// The side of the square a handle is drawn as.
pub const SIZE: f32 = 8.0;

/// A press this near a handle's centre, across and down, takes it.
pub const REACH: f32 = 6.0;

/// How far outside the middle of a side its connect handle sits.
pub const CONNECT_GAP: f32 = 16.0;

/// A side shorter than this on screen has no handle at its middle, which
/// leaves a small node room to be pressed between its corners.
pub const MIDDLE_MIN: f32 = 32.0;

/// One of the eight resize handles of the selected node, by compass point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

impl Handle {
    /// Clockwise from the top-left corner.
    pub const ALL: [Self; 8] = [
        Self::NorthWest,
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
    ];

    /// The name the test harness lists it under.
    pub fn name(self) -> &'static str {
        match self {
            Self::NorthWest => "nw",
            Self::North => "n",
            Self::NorthEast => "ne",
            Self::East => "e",
            Self::SouthEast => "se",
            Self::South => "s",
            Self::SouthWest => "sw",
            Self::West => "w",
        }
    }

    /// The edges of the box a drag of the handle moves: the left or right
    /// one, and the top or bottom one.
    pub fn sides(self) -> [Option<Side>; 2] {
        let across = match self {
            Self::NorthWest | Self::West | Self::SouthWest => Some(Side::Left),
            Self::NorthEast | Self::East | Self::SouthEast => Some(Side::Right),
            Self::North | Self::South => None,
        };
        let down = match self {
            Self::NorthWest | Self::North | Self::NorthEast => Some(Side::Top),
            Self::SouthWest | Self::South | Self::SouthEast => Some(Side::Bottom),
            Self::East | Self::West => None,
        };
        [across, down]
    }
}

/// The resize handles of a node whose box is `bounds`, each with its
/// centre: the four corners, and the middle of each side long enough on
/// screen ([`MIDDLE_MIN`]).
pub fn resize(bounds: Rect, viewport: Viewport) -> Vec<(Handle, (f32, f32))> {
    let wide = bounds.width * viewport.zoom >= MIDDLE_MIN;
    let tall = bounds.height * viewport.zoom >= MIDDLE_MIN;
    Handle::ALL
        .into_iter()
        .filter_map(|handle| {
            let [across, down] = handle.sides();
            let x = match across {
                Some(Side::Left) => bounds.x,
                Some(_) => bounds.right(),
                None if wide => bounds.center().x,
                None => return None,
            };
            let y = match down {
                Some(Side::Top) => bounds.y,
                Some(_) => bounds.bottom(),
                None if tall => bounds.center().y,
                None => return None,
            };
            Some((handle, viewport.to_screen((x, y))))
        })
        .collect()
}

/// The connect handles of a node whose box is `bounds`, each with its
/// centre: [`CONNECT_GAP`] outside the middle of each side of the box, so
/// they never sit on a resize handle.
pub fn connect(bounds: Rect, viewport: Viewport) -> [(Side, (f32, f32)); 4] {
    let centre = bounds.center();
    let at = |x: f32, y: f32, dx: f32, dy: f32| {
        let (x, y) = viewport.to_screen((x, y));
        (x + dx * CONNECT_GAP, y + dy * CONNECT_GAP)
    };
    [
        (Side::Top, at(centre.x, bounds.y, 0.0, -1.0)),
        (Side::Right, at(bounds.right(), centre.y, 1.0, 0.0)),
        (Side::Bottom, at(centre.x, bounds.bottom(), 0.0, 1.0)),
        (Side::Left, at(bounds.x, centre.y, -1.0, 0.0)),
    ]
}

/// Where the two ends of a connector are drawn, `from` then `to`, in board
/// units: where its endpoint handles sit. `None` for a connector with no
/// route, which is one that is not drawn or is drawn as a stub.
///
/// The scene draws a route as one stroked line, cut short under each
/// arrowhead, and then the arrowheads as one filled path, each starting at
/// its tip; so an end with a head is that tip and one without is the
/// line's own end.
pub fn ends(board: &Board, scene: &Scene, id: &Id) -> Option<[Point; 2]> {
    let edge = board.edges.get(id)?;
    let mut paths = scene.items.iter().filter_map(|item| match &item.draw {
        Draw::Path {
            segments, stroke, ..
        } if matches!(&item.source, Source::Edge(of) if of == id) => {
            Some((segments, stroke.is_some()))
        }
        _ => None,
    });
    let (line, true) = paths.next()? else {
        return None;
    };
    let starts = |segments: &[Segment]| -> Vec<Point> {
        segments
            .iter()
            .filter_map(|segment| match segment {
                Segment::MoveTo(point) => Some(*point),
                _ => None,
            })
            .collect()
    };
    // A stub's cross stroke is a second line in its path.
    let [from] = starts(line)[..] else {
        return None;
    };
    let to = match line.last()? {
        Segment::LineTo(point) | Segment::MoveTo(point) => *point,
        _ => return None,
    };
    let heads = edge.heads.each_ref().map(route::has_arrow);
    if heads == [false, false] {
        return Some([from, to]);
    }
    let (arrows, _) = paths.next()?;
    let mut tips = starts(arrows).into_iter();
    let from = if heads[0] { tips.next()? } else { from };
    let to = if heads[1] { tips.next()? } else { to };
    Some([from, to])
}

#[cfg(test)]
mod tests {
    use wren_board::scene::{self, FixedMeasure};
    use wren_board::{parse, End};

    use super::*;
    use crate::fixtures;
    use crate::hit::Frames;

    #[test]
    fn every_handle_has_its_own_name_and_sides() {
        let names: std::collections::HashSet<&str> =
            Handle::ALL.iter().map(|handle| handle.name()).collect();
        assert_eq!(names.len(), 8);
        let sides: std::collections::HashSet<[Option<Side>; 2]> =
            Handle::ALL.iter().map(|handle| handle.sides()).collect();
        assert_eq!(sides.len(), 8);
        assert!(!sides.contains(&[None, None]));
        assert_eq!(
            Handle::SouthEast.sides(),
            [Some(Side::Right), Some(Side::Bottom)]
        );
        assert_eq!(Handle::West.sides(), [Some(Side::Left), None]);
    }

    #[test]
    fn resize_handles_sit_on_the_box_at_any_zoom() {
        let bounds = Rect::new(100.0, 40.0, 120.0, 60.0);
        for zoom in [0.5, 1.0, 2.0] {
            let viewport = Viewport {
                x: -30.0,
                y: 12.0,
                zoom,
            };
            let handles = resize(bounds, viewport);
            // 60 units is 30 px at 50 %, too short for a middle handle.
            let expected = if zoom < 1.0 { 6 } else { 8 };
            assert_eq!(handles.len(), expected, "{zoom}");
            for (handle, centre) in handles {
                let (x, y) = viewport.to_board(centre);
                let [across, down] = handle.sides();
                let want_x = match across {
                    Some(Side::Left) => 100.0,
                    Some(_) => 220.0,
                    None => 160.0,
                };
                let want_y = match down {
                    Some(Side::Top) => 40.0,
                    Some(_) => 100.0,
                    None => 70.0,
                };
                assert!(
                    (x - want_x).abs() < 1e-3 && (y - want_y).abs() < 1e-3,
                    "{handle:?} at {zoom}: {x},{y}"
                );
            }
        }
    }

    #[test]
    fn a_small_node_keeps_only_its_corner_handles() {
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 0.25,
        };
        let handles = resize(Rect::new(0.0, 0.0, 120.0, 60.0), viewport);
        let kept: Vec<Handle> = handles.iter().map(|(handle, _)| *handle).collect();
        assert_eq!(
            kept,
            [
                Handle::NorthWest,
                Handle::NorthEast,
                Handle::SouthEast,
                Handle::SouthWest
            ]
        );
    }

    #[test]
    fn connect_handles_sit_sixteen_pixels_outside_each_side() {
        let bounds = Rect::new(100.0, 40.0, 120.0, 60.0);
        for zoom in [0.5, 1.0, 2.0] {
            let viewport = Viewport {
                x: 10.0,
                y: -5.0,
                zoom,
            };
            let middle = |x: f32, y: f32| viewport.to_screen((x, y));
            let handles = connect(bounds, viewport);
            let (top, right) = (middle(160.0, 40.0), middle(220.0, 70.0));
            let (bottom, left) = (middle(160.0, 100.0), middle(100.0, 70.0));
            assert_eq!(handles[0], (Side::Top, (top.0, top.1 - 16.0)));
            assert_eq!(handles[1], (Side::Right, (right.0 + 16.0, right.1)));
            assert_eq!(handles[2], (Side::Bottom, (bottom.0, bottom.1 + 16.0)));
            assert_eq!(handles[3], (Side::Left, (left.0 - 16.0, left.1)));
            // Never within reach of a resize handle.
            for (_, connect) in handles {
                for (_, resize) in resize(bounds, viewport) {
                    let apart = (connect.0 - resize.0)
                        .abs()
                        .max((connect.1 - resize.1).abs());
                    assert!(apart > 2.0 * REACH, "{zoom}");
                }
            }
        }
    }

    /// How far `point` is from the border of `bounds`, when it lies on or
    /// in the box.
    fn off_border(bounds: Rect, point: Point) -> f32 {
        [
            point.x - bounds.x,
            bounds.right() - point.x,
            point.y - bounds.y,
            bounds.bottom() - point.y,
        ]
        .into_iter()
        .fold(f32::INFINITY, |least, gap| least.min(gap.abs()))
    }

    #[test]
    fn a_connector_s_ends_are_where_it_meets_its_nodes() {
        let board = parse(fixtures::CHECKOUT_FLOW).expect("the sample");
        let scene = scene::build(&board, &fixtures::palette(), &FixedMeasure);
        let frames = Frames::new(&board);
        assert_eq!(board.edges.len(), 17);
        for (id, edge) in &board.edges {
            let ends = ends(&board, &scene, id).expect("a routed connector");
            for (end, point) in [&edge.from, &edge.to].into_iter().zip(ends) {
                let End::Node { node, .. } = end else {
                    panic!("the sample has no free end");
                };
                let bounds = frames
                    .get(&Id::new(node).expect("an id"))
                    .expect("a drawn node");
                // Every shape of the sample meets a side middle on its box.
                assert!(off_border(bounds, point) < 0.01, "{id}: {point:?}");
            }
        }
    }

    #[test]
    fn ends_follow_the_heads_and_a_free_end() {
        let text = r#"schema = 1

[node.a]
kind = "shape"
at = [0, 0]
size = [120, 60]

[edge.both]
kind = "edge"
from = { node = "a" }
to = { at = [400, 30] }
heads = ["arrow", "arrow"]

[edge.none]
kind = "edge"
from = { node = "a" }
to = { at = [400, 30] }
heads = ["none", "none"]
route = "straight"

[edge.start]
kind = "edge"
from = { at = [-200, 30] }
to = { node = "a" }
heads = ["arrow", "none"]

[edge.stub]
kind = "edge"
from = { node = "a" }
to = { node = "gone" }

[edge.lost]
kind = "edge"
from = { node = "gone" }
to = { node = "gone" }
"#;
        let board = parse(text).expect("a board");
        let scene = scene::build(&board, &fixtures::palette(), &FixedMeasure);
        let id = |name: &str| Id::new(name).expect("an id");
        let right = Point::new(120.0, 30.0);
        let free = Point::new(400.0, 30.0);
        assert_eq!(ends(&board, &scene, &id("both")), Some([right, free]));
        assert_eq!(ends(&board, &scene, &id("none")), Some([right, free]));
        assert_eq!(
            ends(&board, &scene, &id("start")),
            Some([Point::new(-200.0, 30.0), Point::new(0.0, 30.0)])
        );
        // A stub has no route, nor has a connector that is not drawn, nor
        // an id that names a node or nothing.
        for name in ["stub", "lost", "a", "nowhere"] {
            assert_eq!(ends(&board, &scene, &id(name)), None, "{name}");
        }
    }
}
