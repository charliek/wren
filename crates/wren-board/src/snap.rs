//! The editing grid (§5.8 of the board plan). Snapping is always on for
//! creating, moving and resizing; the results are integers.
//!
//! The grid is in absolute board units, so a child node snaps by where it
//! is drawn, not by its stored `at`. A value halfway between two grid
//! lines goes to the larger one, so every cell snaps alike on both sides
//! of zero.

use std::cmp::Ordering;

use crate::edit::{Area, Edges};
use crate::GRID;

/// The grid line nearest `value`, in board units: a multiple of
/// [`crate::GRID`]. Past the `i32` range it is the last grid line inside
/// it, and NaN is 0.
pub fn grid(value: f32) -> i32 {
    clamp(nearest(f64::from(value)))
}

/// [`grid`] of each coordinate of a point.
pub fn point([x, y]: [f32; 2]) -> [i32; 2] {
    [grid(x), grid(y)]
}

/// The top-left corner, snapped, of a node of `size` centred on a point: a
/// click with a shape or the text tool armed (§5.5 rule 3).
pub fn centred([x, y]: [f32; 2], [w, h]: [i32; 2]) -> [i32; 2] {
    let corner = |at: f32, size: i32| clamp(nearest(f64::from(at) - f64::from(size) / 2.0));
    [corner(x, w), corner(y, h)]
}

/// The box dragged out from `press` to `release`, both corners snapped and
/// at least one cell each way (§5.5 rule 3). Where the two corners snap to
/// one line, the box takes the cell on the release's side of it.
pub fn span(press: [f32; 2], release: [f32; 2]) -> Area {
    let axis = |from: f32, to: f32| {
        let (a, b) = (nearest(f64::from(from)), nearest(f64::from(to)));
        let (low, high) = match b.cmp(&a) {
            Ordering::Greater => (a, b),
            Ordering::Less => (b, a),
            Ordering::Equal if to >= from => (a, a + G),
            Ordering::Equal => (a - G, a),
        };
        (clamp(low), clamp(high - low))
    };
    let (x, w) = axis(press[0], release[0]);
    let (y, h) = axis(press[1], release[1]);
    Area {
        at: [x, y],
        size: [w, h],
    }
}

/// How far a pointer move of `by` moves the selection, when the pressed
/// node's absolute top-left corner is `at`: the move that puts that corner
/// on the grid line nearest where the pointer took it (§5.8 rule 2). Every
/// selected node moves by the same amount, so their offsets from each
/// other, on the grid or not, are kept.
pub fn drag(at: [i64; 2], by: [f32; 2]) -> [i32; 2] {
    let axis = |at: i64, by: f32| saturate(nearest(at as f64 + f64::from(by)).saturating_sub(at));
    [axis(at[0], by[0]), axis(at[1], by[1])]
}

/// The eight resize handles of a selected node (§5.8 rule 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];

    /// Which edges it drags: left, top, right, bottom.
    pub fn drags(self) -> [bool; 4] {
        match self {
            Self::TopLeft => [true, true, false, false],
            Self::Top => [false, true, false, false],
            Self::TopRight => [false, true, true, false],
            Self::Right => [false, false, true, false],
            Self::BottomRight => [false, false, true, true],
            Self::Bottom => [false, false, false, true],
            Self::BottomLeft => [true, false, false, true],
            Self::Left => [true, false, false, false],
        }
    }
}

/// How far each edge of a node's box moves when `handle` is dragged by
/// `by`, for a box whose absolute top-left corner is `at` (§5.8 rule 5).
/// Each dragged edge goes to the grid line nearest where the pointer took
/// it, but no closer to the opposite edge than the grid line one cell from
/// it; the other edges stay, so the opposite corner does.
pub fn resize(at: [i64; 2], size: [i32; 2], handle: Handle, by: [f32; 2]) -> Edges {
    let [left, top, right, bottom] = handle.drags();
    let axis = |at: i64, size: i32, by: f32, start: bool, end: bool| {
        let far = at + i64::from(size);
        let near = if start {
            nearest(at as f64 + f64::from(by)).min(floor(far - G))
        } else {
            at
        };
        let far_moved = if end {
            nearest(far as f64 + f64::from(by)).max(ceil(at + G))
        } else {
            far
        };
        (saturate(near - at), saturate(far_moved - far))
    };
    let (left, right) = axis(at[0], size[0], by[0], left, right);
    let (top, bottom) = axis(at[1], size[1], by[1], top, bottom);
    Edges {
        left,
        top,
        right,
        bottom,
    }
}

const G: i64 = GRID as i64;

/// The grid line nearest `value`, ties to the larger; NaN is 0.
fn nearest(value: f64) -> i64 {
    let cells = (value / G as f64 + 0.5).floor();
    if cells.is_nan() {
        return 0;
    }
    // Far past any board; keeps the multiplication in range.
    let cells = cells.clamp(-1e15, 1e15) as i64;
    cells * G
}

/// The grid line at or below `value`.
fn floor(value: i64) -> i64 {
    value.div_euclid(G) * G
}

/// The grid line at or above `value`.
fn ceil(value: i64) -> i64 {
    -floor(-value)
}

/// A grid line as an `i32`, the last grid line inside the range past it.
fn clamp(line: i64) -> i32 {
    let limit = i64::from(i32::MAX) / G * G;
    line.clamp(-limit, limit) as i32
}

fn saturate(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::Rng;

    #[test]
    fn values_snap_to_the_nearest_line_ties_up() {
        let table: &[(f32, i32)] = &[
            (0.0, 0),
            (-0.0, 0),
            (9.99, 0),
            (10.0, 20),
            (29.0, 20),
            (30.0, 40),
            (-9.99, 0),
            (-10.0, 0),
            (-10.01, -20),
            (-30.0, -20),
            (-30.5, -40),
            (1e12, 2_147_483_640),
            (-1e12, -2_147_483_640),
            (f32::INFINITY, 2_147_483_640),
            (f32::NEG_INFINITY, -2_147_483_640),
            (f32::NAN, 0),
        ];
        for &(value, line) in table {
            assert_eq!(grid(value), line, "{value}");
        }
        assert_eq!(point([10.0, -10.5]), [20, -20]);
    }

    #[test]
    fn every_cell_snaps_alike() {
        let mut rng = Rng(5);
        for _ in 0..20_000 {
            // Quarter units, exact in f32 at these sizes.
            let value = (rng.below(400_000) as f32 - 200_000.0) / 4.0;
            let cells = rng.below(200) as i32 - 100;
            let line = grid(value);
            assert_eq!(line % GRID, 0);
            assert!((f64::from(value) - f64::from(line)).abs() <= 10.0);
            assert_eq!(grid(value + (cells * GRID) as f32), line + cells * GRID);
        }
    }

    #[test]
    fn a_click_centres_a_node_and_snaps_its_corner() {
        assert_eq!(centred([100.0, 100.0], [120, 60]), [40, 80]);
        assert_eq!(centred([0.0, 0.0], [120, 20]), [-60, 0]);
        assert_eq!(centred([-1.0, 9.0], [120, 60]), [-60, -20]);
    }

    #[test]
    fn a_drag_out_snaps_both_corners_and_keeps_one_cell() {
        let area = |at: [i32; 2], size: [i32; 2]| Area { at, size };
        let table: &[([f32; 2], [f32; 2], Area)] = &[
            ([13.0, 17.0], [95.0, 64.0], area([20, 20], [80, 40])),
            // Up and to the left of the press.
            ([95.0, 64.0], [13.0, 17.0], area([20, 20], [80, 40])),
            ([13.0, 17.0], [95.0, 5.0], area([20, 0], [80, 20])),
            // Both corners on one line: the cell on the release's side.
            ([13.0, 13.0], [14.0, 14.0], area([20, 20], [20, 20])),
            ([13.0, 13.0], [12.0, 12.0], area([0, 0], [20, 20])),
            ([13.0, 13.0], [13.0, 12.0], area([20, 0], [20, 20])),
            ([13.0, 13.0], [9.0, 9.0], area([0, 0], [20, 20])),
            ([-50.0, 0.0], [-75.0, 41.0], area([-80, 0], [40, 40])),
        ];
        for &(press, release, want) in table {
            assert_eq!(span(press, release), want, "{press:?} {release:?}");
        }
    }

    #[test]
    fn a_move_puts_the_pressed_corner_on_the_nearest_line() {
        let table: &[([i64; 2], [f32; 2], [i32; 2])] = &[
            ([0, 0], [9.0, 11.0], [0, 20]),
            ([40, 40], [-31.0, 4.0], [-40, 0]),
            // Off the grid: the corner lands on it.
            ([5, 7], [12.0, 3.0], [15, 13]),
            ([5, 7], [-6.0, -18.0], [-5, -27]),
            ([-13, 0], [0.0, 0.0], [-7, 0]),
            // A child drawn far out, by its absolute corner.
            ([3_000_000_007, 0], [4.0, 0.0], [13, 0]),
        ];
        for &(at, by, want) in table {
            assert_eq!(drag(at, by), want, "{at:?} {by:?}");
        }
        let mut rng = Rng(9);
        for _ in 0..10_000 {
            let at = [rng.below(100_000) as i64 - 50_000, rng.below(997) as i64];
            let by = [
                rng.below(8_000) as f32 / 4.0 - 1_000.0,
                rng.below(8_000) as f32 / 4.0 - 1_000.0,
            ];
            let [dx, dy] = drag(at, by);
            for (at, by, d) in [(at[0], by[0], dx), (at[1], by[1], dy)] {
                let landed = at + i64::from(d);
                assert_eq!(landed % G, 0);
                assert!((at as f64 + f64::from(by) - landed as f64).abs() <= 10.0);
            }
        }
    }

    fn edges(left: i32, top: i32, right: i32, bottom: i32) -> Edges {
        Edges {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn each_handle_moves_its_own_edges_to_the_grid() {
        // A box at (40, 40), 120 by 60, dragged by (13, -57).
        let by = [13.0, -57.0];
        let table = [
            (Handle::TopLeft, edges(20, -60, 0, 0)),
            (Handle::Top, edges(0, -60, 0, 0)),
            (Handle::TopRight, edges(0, -60, 20, 0)),
            (Handle::Right, edges(0, 0, 20, 0)),
            // The bottom edge would cross the top: one cell is left.
            (Handle::BottomRight, edges(0, 0, 20, -40)),
            (Handle::Bottom, edges(0, 0, 0, -40)),
            (Handle::BottomLeft, edges(20, 0, 0, -40)),
            (Handle::Left, edges(20, 0, 0, 0)),
        ];
        for (handle, want) in table {
            assert_eq!(resize([40, 40], [120, 60], handle, by), want, "{handle:?}");
        }
    }

    #[test]
    fn a_resize_keeps_one_cell_and_the_opposite_corner() {
        // A box's corner and size, the handle, the pointer's move, the edges.
        type Case = ([i64; 2], [i32; 2], Handle, [f32; 2], Edges);
        let table: &[Case] = &[
            (
                [40, 40],
                [120, 60],
                Handle::Left,
                [500.0, 0.0],
                edges(100, 0, 0, 0),
            ),
            (
                [40, 40],
                [120, 60],
                Handle::Right,
                [-500.0, 0.0],
                edges(0, 0, -100, 0),
            ),
            (
                [40, 40],
                [120, 60],
                Handle::Top,
                [0.0, 90.0],
                edges(0, 40, 0, 0),
            ),
            // Off the grid: the dragged edge snaps, and one cell from the
            // other edge is the grid line a cell or more away.
            (
                [5, 5],
                [30, 30],
                Handle::Left,
                [100.0, 0.0],
                edges(-5, 0, 0, 0),
            ),
            (
                [5, 5],
                [30, 30],
                Handle::BottomRight,
                [0.0, 0.0],
                edges(0, 0, 5, 5),
            ),
            // Narrower than a cell: dragging an edge gives it one.
            (
                [0, 0],
                [5, 5],
                Handle::Right,
                [0.0, 0.0],
                edges(0, 0, 15, 0),
            ),
            (
                [0, 0],
                [5, 5],
                Handle::Bottom,
                [2.0, 2.0],
                edges(0, 0, 0, 15),
            ),
        ];
        for &(at, size, handle, by, want) in table {
            assert_eq!(
                resize(at, size, handle, by),
                want,
                "{at:?} {size:?} {handle:?}"
            );
        }
        let mut rng = Rng(13);
        for _ in 0..10_000 {
            let at = [
                rng.below(2_000) as i64 - 1_000,
                rng.below(2_000) as i64 - 1_000,
            ];
            let size = [1 + rng.below(400) as i32, 1 + rng.below(400) as i32];
            let handle = *rng.pick(&Handle::ALL);
            let by = [
                rng.below(4_000) as f32 / 2.0 - 1_000.0,
                rng.below(4_000) as f32 / 2.0 - 1_000.0,
            ];
            let moved = resize(at, size, handle, by);
            let [left, top, right, bottom] = handle.drags();
            let x = [at[0], at[0] + i64::from(size[0])];
            let y = [at[1], at[1] + i64::from(size[1])];
            let x = [x[0] + i64::from(moved.left), x[1] + i64::from(moved.right)];
            let y = [y[0] + i64::from(moved.top), y[1] + i64::from(moved.bottom)];
            for (dragged, delta, edge) in [
                (left, moved.left, x[0]),
                (top, moved.top, y[0]),
                (right, moved.right, x[1]),
                (bottom, moved.bottom, y[1]),
            ] {
                if dragged {
                    assert_eq!(edge % G, 0, "{at:?} {size:?} {handle:?} {by:?}");
                } else {
                    assert_eq!(delta, 0);
                }
            }
            if left || right {
                assert!(x[1] - x[0] >= G, "{at:?} {size:?} {handle:?} {by:?}");
            }
            if top || bottom {
                assert!(y[1] - y[0] >= G, "{at:?} {size:?} {handle:?} {by:?}");
            }
        }
    }
}
