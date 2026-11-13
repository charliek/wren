//! Where keyboard building puts a new node (§5.6 of the board plan).
//!
//! Everything here is in absolute board units, as integers: a child node
//! is compared by where it is drawn, not by its stored `at`.

use std::collections::BTreeMap;

use crate::{placements, snap, Board, Dir, End, Id, GRID};

/// A new node's size (§5.5 rule 2): six cells by three.
pub const SIZE: [i32; 2] = [120, 60];

/// A new text node's size (§5.5 rule 2).
pub const TEXT_SIZE: [i32; 2] = [120, 20];

/// The gap between a node and the one built from it: along a horizontal
/// direction, then along a vertical one (§5.6 rule 2).
pub const GAP: [i32; 2] = [80, 60];

/// How many steps across the direction the search for a free slot takes,
/// each way, before it gives up (§5.6 rule 4).
pub const STEPS: i64 = 200;

/// A node's box where it is drawn, in absolute board units; each length
/// at least 1, as the scene draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl Bounds {
    fn new(at: [i64; 2], size: [i32; 2]) -> Self {
        Self {
            x: at[0],
            y: at[1],
            w: i64::from(size[0].max(1)),
            h: i64::from(size[1].max(1)),
        }
    }

    pub fn right(self) -> i64 {
        self.x + self.w
    }

    pub fn bottom(self) -> i64 {
        self.y + self.h
    }

    /// The centre, doubled so it stays an integer.
    pub fn centre(self) -> [i64; 2] {
        [2 * self.x + self.w, 2 * self.y + self.h]
    }

    /// Whether the two boxes share area; touching edges do not.
    fn meets(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    fn grown(self, by: i64) -> Self {
        Self {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2 * by,
            h: self.h + 2 * by,
        }
    }
}

/// Every node's box where it is drawn, by id.
pub(crate) fn boxes(board: &Board) -> BTreeMap<&str, Bounds> {
    placements(board)
        .into_iter()
        .map(|(id, placed)| (id.as_str(), Bounds::new(placed.at, board.nodes[id].size)))
        .collect()
}

/// The board's flow direction (§5.6 rule 1): each connector between two
/// nodes votes with its nominal exit side (§5.12 rule 4), pins ignored.
/// The majority wins; a tie that includes down, or no vote at all, is
/// down, and other ties go right, then up, then left. A connector from a
/// node to itself joins no two nodes and does not vote.
pub fn flow(board: &Board) -> Dir {
    let boxes = boxes(board);
    let mut votes: BTreeMap<u8, usize> = BTreeMap::new();
    for edge in board.edges.values() {
        let (Some(from), Some(to)) = (node_of(&edge.from), node_of(&edge.to)) else {
            continue;
        };
        if from == to {
            continue;
        }
        let (Some(&a), Some(&b)) = (boxes.get(from), boxes.get(to)) else {
            continue;
        };
        *votes.entry(rank(exit(a, b))).or_default() += 1;
    }
    let most = votes.values().copied().max().unwrap_or(0);
    // Ranked down, right, up, left: the first with the most votes wins.
    [Dir::Down, Dir::Right, Dir::Up, Dir::Left]
        .into_iter()
        .find(|&dir| votes.get(&rank(dir)).copied().unwrap_or(0) == most)
        .unwrap_or(Dir::Down)
}

fn rank(dir: Dir) -> u8 {
    match dir {
        Dir::Down => 0,
        Dir::Right => 1,
        Dir::Up => 2,
        Dir::Left => 3,
    }
}

/// The node an end names, if it is attached.
pub(crate) fn node_of(end: &End) -> Option<&str> {
    match end {
        End::Node { node, .. } => Some(node),
        End::Free { .. } => None,
    }
}

/// The nominal exit side from `from` towards `to` (§5.12 rule 4), as
/// [`crate::route::nominal_exit`] gives it, in integers.
fn exit(from: Bounds, to: Bounds) -> Dir {
    if to.y >= from.bottom() {
        Dir::Down
    } else if to.bottom() <= from.y {
        Dir::Up
    } else if to.centre()[0] >= from.centre()[0] {
        Dir::Right
    } else {
        Dir::Left
    }
}

/// The ideal slot's corner for a node of `size` built from `source` going
/// `dir`, snapped to the grid (§5.6 rule 2).
fn ideal(source: Bounds, [w, h]: [i64; 2], dir: Dir) -> [i32; 2] {
    let [across, down] = GAP.map(i64::from);
    let (x, y) = match dir {
        Dir::Down => (source.x + (source.w - w) / 2, source.bottom() + down),
        Dir::Up => (source.x + (source.w - w) / 2, source.y - down - h),
        Dir::Right => (source.right() + across, source.y + (source.h - h) / 2),
        Dir::Left => (source.x - across - w, source.y + (source.h - h) / 2),
    };
    [snap::line(x), snap::line(y)]
}

/// Whether a node of `size` at `at` keeps one cell clear of every node
/// (§5.6 rule 3).
fn free(boxes: &BTreeMap<&str, Bounds>, at: [i32; 2], size: [i32; 2]) -> bool {
    let slot = Bounds::new(at.map(i64::from), size).grown(i64::from(GRID));
    !boxes.values().any(|&other| slot.meets(other))
}

/// The top-left corner of the nearest free slot for a node of `size`
/// built from the node `from` going `dir` (§5.6 rules 2 to 4): the ideal
/// slot if it is free, else the first free one `k` pitches across the
/// direction, `k` from 1, the right or lower side before the other. The
/// pitch is the node's width and a horizontal gap across a vertical
/// direction, its height and a vertical gap across a horizontal one.
/// `None` when `from` names no node or no slot within [`STEPS`] is free.
pub fn slot(board: &Board, from: &Id, size: [i32; 2], dir: Dir) -> Option<[i32; 2]> {
    let boxes = boxes(board);
    let source = *boxes.get(from.as_str())?;
    let size = size.map(|length| length.max(1));
    let start = ideal(source, size.map(i64::from), dir);
    let [across, down] = GAP.map(i64::from);
    let (pitch, axis) = match dir {
        Dir::Up | Dir::Down => (i64::from(size[0]) + across, 0),
        Dir::Left | Dir::Right => (i64::from(size[1]) + down, 1),
    };
    let steps = (1..=STEPS).flat_map(|k| [k, -k]);
    std::iter::once(0).chain(steps).find_map(|k| {
        let mut at = start;
        at[axis] = snap::line(i64::from(start[axis]) + k * pitch);
        free(&boxes, at, size).then_some(at)
    })
}

/// The node in the ideal slot of a node of `size` built from `from` going
/// `dir`, which `board.connect.<dir>` joins instead of creating one (§5.6
/// rule 6): of the nodes other than `from` whose boxes share area with the
/// slot, the nearest by centre distance, ties by id.
pub fn occupant(board: &Board, from: &Id, size: [i32; 2], dir: Dir) -> Option<Id> {
    let boxes = boxes(board);
    let source = *boxes.get(from.as_str())?;
    let size = size.map(|length| length.max(1));
    let slot = Bounds::new(ideal(source, size.map(i64::from), dir).map(i64::from), size);
    let [cx, cy] = slot.centre();
    boxes
        .iter()
        .filter(|&(&id, &other)| id != from.as_str() && slot.meets(other))
        .min_by_key(|&(&id, &other)| {
            let [x, y] = other.centre();
            let (dx, dy) = (i128::from(x - cx), i128::from(y - cy));
            (dx * dx + dy * dy, id)
        })
        .and_then(|(&id, _)| Id::new(id))
}

/// The corner of an unconnected node of `size` that `board.connect.next`
/// makes with nothing selected (§5.6 rule 5): centred on `centre`, the
/// middle of the view, snapped as a click places one (§5.5 rule 3), then
/// stepped right by the node's width and a horizontal gap until it is
/// free. `None` when no slot within [`STEPS`] is.
pub fn start(board: &Board, centre: [f32; 2], size: [i32; 2]) -> Option<[i32; 2]> {
    let boxes = boxes(board);
    let size = size.map(|length| length.max(1));
    let first = snap::centred(centre, size);
    let pitch = i64::from(size[0]) + i64::from(GAP[0]);
    (0..=STEPS).find_map(|k| {
        let at = [snap::line(i64::from(first[0]) + k * pitch), first[1]];
        free(&boxes, at, size).then_some(at)
    })
}

/// Whether a connector runs from the node `from` to the node `to`.
pub fn joins(board: &Board, from: &Id, to: &Id) -> bool {
    board.edges.values().any(|edge| {
        node_of(&edge.from) == Some(from.as_str()) && node_of(&edge.to) == Some(to.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    /// A board of shape nodes, each `(id, [x, y])` at the default size,
    /// and connectors between named nodes.
    fn board(nodes: &[(&str, [i32; 2])], edges: &[(&str, &str)]) -> Board {
        let mut text = String::from("schema = 1\n");
        for (id, [x, y]) in nodes {
            text += &format!(
                "\n[node.{id}]\nkind = \"shape\"\nat = [{x}, {y}]\nsize = [120, 60]\nz = \"V\"\n"
            );
        }
        for (k, (from, to)) in edges.iter().enumerate() {
            text += &format!(
                "\n[edge.e{k}]\nkind = \"edge\"\nfrom = {{ node = \"{from}\" }}\n\
                 to = {{ node = \"{to}\" }}\nz = \"W\"\n"
            );
        }
        parse(&text).unwrap()
    }

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    #[test]
    fn the_ideal_slot_is_a_gap_away_and_centred_across() {
        let b = board(&[("s", [0, 0])], &[]);
        let table = [
            (Dir::Down, [0, 120]),
            (Dir::Up, [0, -120]),
            (Dir::Right, [200, 0]),
            (Dir::Left, [-200, 0]),
        ];
        for (dir, want) in table {
            assert_eq!(slot(&b, &id("s"), SIZE, dir), Some(want), "{dir:?}");
        }
        // A smaller node is centred across the source, then snapped.
        assert_eq!(slot(&b, &id("s"), [40, 20], Dir::Down), Some([40, 120]));
        assert_eq!(slot(&b, &id("s"), [40, 20], Dir::Right), Some([200, 20]));
        // An off-grid source gives a snapped slot.
        let b = board(&[("s", [7, 3])], &[]);
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Down), Some([0, 120]));
    }

    #[test]
    fn a_taken_slot_steps_across_right_or_down_first() {
        // Below s is taken: the slot one pitch (w + 80) to the right.
        let b = board(&[("s", [0, 0]), ("t", [0, 120])], &[]);
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Down), Some([200, 120]));
        // Then the left, at the same distance.
        let b = board(&[("s", [0, 0]), ("t", [0, 120]), ("u", [200, 120])], &[]);
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Down), Some([-200, 120]));
        // Across a horizontal direction the pitch is h + 60, down first.
        let b = board(&[("s", [0, 0]), ("t", [200, 0])], &[]);
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Right), Some([200, 120]));
    }

    #[test]
    fn free_means_one_cell_clear() {
        // A node one cell from the slot's edge leaves it free; closer
        // takes it.
        let b = board(&[("s", [0, 0]), ("t", [140, 120])], &[]);
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Down), Some([0, 120]));
        let b = board(&[("s", [0, 0]), ("t", [139, 120])], &[]);
        assert_ne!(slot(&b, &id("s"), SIZE, Dir::Down), Some([0, 120]));
        let b = board(&[("s", [0, 0]), ("t", [0, 200])], &[]);
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Down), Some([0, 120]));
        let b = board(&[("s", [0, 0]), ("t", [0, 199])], &[]);
        assert_ne!(slot(&b, &id("s"), SIZE, Dir::Down), Some([0, 120]));
    }

    #[test]
    fn the_search_gives_up_after_its_steps() {
        // One wide node fills the row below every slot within reach.
        let text = "schema = 1\n\n[node.s]\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\nz = \"V\"\n\n\
                    [node.w]\nkind = \"shape\"\nat = [-100000, 100]\nsize = [200000, 100]\nz = \"V\"\n";
        let b = parse(text).unwrap();
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Down), None);
        assert_eq!(slot(&b, &id("s"), SIZE, Dir::Up), Some([0, -120]));
        assert_eq!(slot(&b, &id("gone"), SIZE, Dir::Up), None);
    }

    #[test]
    fn a_child_is_placed_from_where_it_is_drawn() {
        let text = "schema = 1\n\n[node.p]\nkind = \"shape\"\nat = [1000, 1000]\nsize = [400, 400]\nz = \"V\"\n\n\
                    [node.c]\nkind = \"shape\"\nat = [20, 20]\nparent = \"p\"\nsize = [120, 60]\nz = \"V\"\n";
        let b = parse(text).unwrap();
        // Below c, inside p: taken by p itself; the search goes across and
        // past p, never relative to it.
        assert_eq!(slot(&b, &id("c"), SIZE, Dir::Up), Some([1020, 900]));
        let at = slot(&b, &id("c"), SIZE, Dir::Down).unwrap();
        assert!(at[0] >= 1420 || at[0] + 120 <= 980, "{at:?}");
    }

    #[test]
    fn coordinates_stay_in_range_at_the_edge() {
        let text = format!(
            "schema = 1\n\n[node.s]\nkind = \"shape\"\nat = [{}, {}]\nsize = [120, 60]\nz = \"V\"\n",
            i32::MAX - 100,
            i32::MAX - 50
        );
        let b = parse(&text).unwrap();
        for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
            let at = slot(&b, &id("s"), SIZE, dir).unwrap();
            assert!(at.iter().all(|v| v % GRID == 0), "{dir:?} {at:?}");
        }
    }

    #[test]
    fn the_flow_is_the_majority_of_nominal_exits() {
        let nodes = [
            ("a", [0, 0]),
            ("b", [0, 200]),
            ("c", [300, 0]),
            ("d", [300, 200]),
            ("e", [-300, 0]),
        ];
        let table: &[(&[(&str, &str)], Dir)] = &[
            (&[], Dir::Down),
            (&[("a", "c")], Dir::Right),
            (&[("a", "c"), ("b", "d"), ("a", "b")], Dir::Right),
            // A tie with down is down.
            (&[("a", "c"), ("a", "b")], Dir::Down),
            // Other ties: right, then up, then left.
            (&[("b", "a"), ("a", "c")], Dir::Right),
            (&[("b", "a"), ("a", "e")], Dir::Up),
            (&[("a", "e")], Dir::Left),
            // A diagonal: wholly below is down.
            (&[("a", "d")], Dir::Down),
            // A self-loop and a dangling end do not vote.
            (&[("a", "a"), ("a", "gone"), ("a", "e")], Dir::Left),
        ];
        for &(edges, want) in table {
            assert_eq!(flow(&board(&nodes, edges)), want, "{edges:?}");
        }
    }

    #[test]
    fn the_occupant_is_the_nearest_node_in_the_slot() {
        let b = board(&[("s", [0, 0]), ("t", [180, 10]), ("u", [260, -20])], &[]);
        assert_eq!(occupant(&b, &id("s"), SIZE, Dir::Right), Some(id("t")));
        assert_eq!(occupant(&b, &id("s"), SIZE, Dir::Down), None);
        // Touching the slot is not in it.
        let b = board(&[("s", [0, 0]), ("t", [320, 0])], &[]);
        assert_eq!(occupant(&b, &id("s"), SIZE, Dir::Right), None);
        // Equal distances go to the lower id.
        let b = board(&[("s", [0, 0]), ("y", [200, -40]), ("x", [200, 40])], &[]);
        assert_eq!(occupant(&b, &id("s"), SIZE, Dir::Right), Some(id("x")));
    }

    #[test]
    fn an_unconnected_node_starts_centred_and_steps_right() {
        let empty = Board::default();
        assert_eq!(start(&empty, [0.0, 0.0], SIZE), Some([-60, -20]));
        let b = board(&[("a", [-60, -20])], &[]);
        assert_eq!(start(&b, [0.0, 0.0], SIZE), Some([140, -20]));
        let b = board(&[("a", [-60, -20]), ("b", [140, -20])], &[]);
        assert_eq!(start(&b, [10.0, 10.0], SIZE), Some([360, -20]));
    }

    #[test]
    fn joins_reads_the_direction() {
        let b = board(&[("a", [0, 0]), ("b", [0, 200])], &[("a", "b")]);
        assert!(joins(&b, &id("a"), &id("b")));
        assert!(!joins(&b, &id("b"), &id("a")));
    }
}
