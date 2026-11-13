//! Walking the selection with the arrow keys (§5.7 of the board plan), and
//! the target search of keyboard connecting (§5.12 rule 3).
//!
//! Geometry is in absolute board units, compared in integers: centres are
//! doubled so they stay whole.

use std::collections::{BTreeMap, BTreeSet};

use crate::place::{boxes, node_of, Bounds};
use crate::{scene, Board, Dir, Id, Side};

/// The element an arrow key selects from the element `from` going `dir`,
/// or `None` when nothing lies that way (§5.7 rules 1 and 2).
///
/// From a node, a candidate must have its centre ahead of the node's
/// centre. It is then in a tier: 0, in line (either centre lies strictly
/// inside the other node's span across the direction); 1, its centre is
/// past the node's edge on that side and the two share a connector; 2,
/// it is at least as far along the direction as across it; 3, its centre
/// is past the node's edge. A candidate in none of these is not one. The
/// lowest tier wins, then the smallest distance along plus twice the
/// distance across, then the id.
///
/// From a connector, the end node that lies further that way, by centre;
/// the `from` end on a tie, and the one end that names a node when the
/// other does not. An id that names both a node and a connector walks
/// from the node.
pub fn next(board: &Board, from: &Id, dir: Dir) -> Option<Id> {
    let boxes = boxes(board);
    if let Some(&origin) = boxes.get(from.as_str()) {
        return from_node(board, &boxes, from, origin, dir);
    }
    let edge = board.edges.get(from)?;
    let end = |end| {
        let (id, _) = board.nodes.get_key_value(node_of(end)?)?;
        Some((id, along(boxes.get(id.as_str())?.centre(), dir)))
    };
    match (end(&edge.from), end(&edge.to)) {
        (Some((a, at_a)), Some((b, at_b))) => Some(if at_b > at_a { b } else { a }.clone()),
        (Some((id, _)), None) | (None, Some((id, _))) => Some(id.clone()),
        (None, None) => None,
    }
}

/// How far `centre` lies in direction `dir`.
fn along([x, y]: [i64; 2], dir: Dir) -> i64 {
    match dir {
        Dir::Right => x,
        Dir::Left => -x,
        Dir::Down => y,
        Dir::Up => -y,
    }
}

fn from_node(
    board: &Board,
    boxes: &BTreeMap<&str, Bounds>,
    from: &Id,
    origin: Bounds,
    dir: Dir,
) -> Option<Id> {
    let linked: BTreeSet<&str> = board
        .edges
        .values()
        .filter_map(|edge| match (node_of(&edge.from), node_of(&edge.to)) {
            (Some(a), Some(b)) if a == from.as_str() => Some(b),
            (Some(a), Some(b)) if b == from.as_str() => Some(a),
            _ => None,
        })
        .collect();
    let o = origin.centre();
    let horizontal = matches!(dir, Dir::Left | Dir::Right);
    // Doubled, like the centres: the node's span across the direction and
    // its edge on that side.
    let (o_low, o_high) = if horizontal {
        (2 * origin.y, 2 * origin.bottom())
    } else {
        (2 * origin.x, 2 * origin.right())
    };
    let edge = match dir {
        Dir::Right => 2 * origin.right(),
        Dir::Left => -2 * origin.x,
        Dir::Down => 2 * origin.bottom(),
        Dir::Up => -2 * origin.y,
    };
    boxes
        .iter()
        .filter(|&(&id, _)| id != from.as_str())
        .filter_map(|(&id, &other)| {
            let n = other.centre();
            let main = along(n, dir) - along(o, dir);
            if main <= 0 {
                return None;
            }
            let (o_across, n_across) = if horizontal {
                (o[1], n[1])
            } else {
                (o[0], n[0])
            };
            let cross = (n_across - o_across).abs();
            let (n_low, n_high) = if horizontal {
                (2 * other.y, 2 * other.bottom())
            } else {
                (2 * other.x, 2 * other.right())
            };
            let in_line =
                (o_low < n_across && n_across < o_high) || (n_low < o_across && o_across < n_high);
            let past_edge = along(n, dir) > edge;
            let tier = if in_line {
                0
            } else if past_edge && linked.contains(id) {
                1
            } else if main >= cross {
                2
            } else if past_edge {
                3
            } else {
                return None;
            };
            Some((tier, main + 2 * cross, id))
        })
        .min()
        .and_then(|(_, _, id)| Id::new(id))
}

/// The node an arrow selects when nothing is selected and the cursor is
/// gone (§5.7 rule 2): the one whose top-left corner has the smallest
/// `(y, x)`, ties by id.
pub fn first(board: &Board) -> Option<Id> {
    boxes(board)
        .into_iter()
        .min_by_key(|&(id, bounds)| (bounds.y, bounds.x, id))
        .and_then(|(id, _)| Id::new(id))
}

/// The connector that meets the node `node` on the side `dir` names, the
/// lowest id first (§5.7 rule 4): its pinned side there, else the side the
/// scene draws it on (§5.12 rule 4). A connector whose other end names no
/// node meets it where its stub leaves.
pub fn connector(board: &Board, node: &Id, dir: Dir) -> Option<Id> {
    if !board.nodes.contains_key(node) {
        return None;
    }
    let side = match dir {
        Dir::Left => Side::Left,
        Dir::Right => Side::Right,
        Dir::Up => Side::Top,
        Dir::Down => Side::Bottom,
    };
    let sides = scene::sides(board);
    board.edges.iter().find_map(|(id, edge)| {
        let drawn = sides.get(id)?;
        let meets = [&edge.from, &edge.to]
            .into_iter()
            .zip(drawn)
            .any(|(end, drawn)| node_of(end) == Some(node.as_str()) && *drawn == Some(side));
        meets.then(|| id.clone())
    })
}

/// The best target for the query of keyboard connecting (§5.12 rule 3):
/// of the nodes whose label contains `query`, ignoring case, those it
/// starts first, then by the `(y, x)` of their top-left corner, then by
/// id. `None` for an empty query.
pub fn query(board: &Board, query: &str) -> Option<Id> {
    let query = query.to_lowercase();
    if query.is_empty() {
        return None;
    }
    let boxes = boxes(board);
    board
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            let label = node.text.to_lowercase();
            let bounds = boxes.get(id.as_str())?;
            label
                .contains(&query)
                .then(|| ((!label.starts_with(&query), bounds.y, bounds.x, id), id))
        })
        .min_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, id)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    /// Shape nodes `(id, [x, y], [w, h], label)` and connectors
    /// `(id, from, to, extra)`, the extra lines going into the edge block.
    fn board(nodes: &[(&str, [i32; 2], [i32; 2], &str)], edges: &[(&str, &str, &str)]) -> Board {
        let mut text = String::from("schema = 1\n");
        for (id, [x, y], [w, h], label) in nodes {
            text += &format!(
                "\n[node.{id}]\ntext = \"{label}\"\nkind = \"shape\"\nat = [{x}, {y}]\n\
                 size = [{w}, {h}]\nz = \"V\"\n"
            );
        }
        for (id, from, to) in edges {
            text +=
                &format!("\n[edge.{id}]\nkind = \"edge\"\nfrom = {from}\nto = {to}\nz = \"W\"\n");
        }
        parse(&text).unwrap()
    }

    const S: [i32; 2] = [120, 60];

    #[test]
    fn in_line_beats_nearer_off_line() {
        // b is straight below and far; c is near but off to the side.
        let g = board(
            &[
                ("a", [0, 0], S, ""),
                ("b", [0, 400], S, ""),
                ("c", [200, 100], S, ""),
            ],
            &[],
        );
        assert_eq!(next(&g, &id("a"), Dir::Down), Some(id("b")));
        assert_eq!(next(&g, &id("b"), Dir::Up), Some(id("a")));
        // Nothing lies to the left.
        assert_eq!(next(&g, &id("a"), Dir::Left), None);
    }

    #[test]
    fn a_shared_connector_comes_before_the_cone() {
        // From a branch child up: its parent is far across but joined, so
        // it beats an unjoined node closer in the cone (tier 1 before 2).
        let nodes = [
            ("p", [0, 0], S, ""),
            ("q", [400, 120], S, ""),
            ("r", [280, -200], S, ""),
        ];
        let joined = board(&nodes, &[("e", "{ node = \"p\" }", "{ node = \"q\" }")]);
        assert_eq!(next(&joined, &id("q"), Dir::Up), Some(id("p")));
        let apart = board(&nodes, &[]);
        assert_eq!(next(&apart, &id("q"), Dir::Up), Some(id("r")));
    }

    #[test]
    fn the_tiers_rank_in_order() {
        // o at the origin; each candidate below in one tier.
        let o = ("o", [0, 0], S, "");
        // Tier 2: below the edge, more along than across.
        let two = ("t2", [200, 300], S, "");
        // Tier 3: past the edge but more across than along.
        let three = ("t3", [600, 80], S, "");
        let g = board(&[o, three, two], &[]);
        assert_eq!(next(&g, &id("o"), Dir::Down), Some(id("t2")));
        let g = board(&[o, three], &[]);
        assert_eq!(next(&g, &id("o"), Dir::Down), Some(id("t3")));
        // Ahead by centre but neither past the edge nor in the cone: no
        // candidate.
        let g = board(&[o, ("x", [600, 10], S, "")], &[]);
        assert_eq!(next(&g, &id("o"), Dir::Down), None);
    }

    #[test]
    fn ties_go_by_score_then_id() {
        let g = board(
            &[
                ("o", [0, 0], S, ""),
                ("m", [-200, 200], S, ""),
                ("k", [200, 200], S, ""),
            ],
            &[],
        );
        assert_eq!(next(&g, &id("o"), Dir::Down), Some(id("k")));
    }

    #[test]
    fn from_a_connector_the_end_further_that_way() {
        let g = board(
            &[("a", [0, 0], S, ""), ("b", [300, 200], S, "")],
            &[
                ("e", "{ node = \"a\" }", "{ node = \"b\" }"),
                ("f", "{ node = \"a\" }", "{ node = \"gone\" }"),
                ("g", "{ at = [0, 0] }", "{ at = [9, 9] }"),
            ],
        );
        assert_eq!(next(&g, &id("e"), Dir::Left), Some(id("a")));
        assert_eq!(next(&g, &id("e"), Dir::Right), Some(id("b")));
        assert_eq!(next(&g, &id("e"), Dir::Up), Some(id("a")));
        assert_eq!(next(&g, &id("e"), Dir::Down), Some(id("b")));
        assert_eq!(next(&g, &id("f"), Dir::Right), Some(id("a")));
        assert_eq!(next(&g, &id("g"), Dir::Right), None);
        // A tie goes to the `from` end.
        let g = board(
            &[("a", [0, 0], S, ""), ("b", [0, 200], S, "")],
            &[("e", "{ node = \"b\" }", "{ node = \"a\" }")],
        );
        assert_eq!(next(&g, &id("e"), Dir::Left), Some(id("b")));
        assert_eq!(next(&g, &id("gone"), Dir::Left), None);
    }

    #[test]
    fn a_child_walks_by_where_it_is_drawn() {
        let text = "schema = 1\n\n[node.p]\nkind = \"shape\"\nat = [1000, 0]\nsize = [400, 400]\nz = \"V\"\n\n\
                    [node.c]\nkind = \"shape\"\nat = [20, 20]\nparent = \"p\"\nsize = [120, 60]\nz = \"V\"\n\n\
                    [node.d]\nkind = \"shape\"\nat = [1020, 600]\nsize = [120, 60]\nz = \"V\"\n";
        let g = parse(text).unwrap();
        assert_eq!(next(&g, &id("d"), Dir::Up), Some(id("c")));
    }

    #[test]
    fn the_first_node_is_top_then_left() {
        let g = board(
            &[
                ("a", [100, 0], S, ""),
                ("b", [0, 0], S, ""),
                ("c", [-500, 20], S, ""),
            ],
            &[],
        );
        assert_eq!(first(&g), Some(id("b")));
        assert_eq!(first(&Board::default()), None);
    }

    #[test]
    fn a_connector_is_found_by_the_side_it_meets() {
        let g = board(
            &[
                ("a", [0, 0], S, ""),
                ("b", [300, 0], S, ""),
                ("c", [0, 300], S, ""),
            ],
            &[
                // Floating: right out of a, into b's left.
                ("e1", "{ node = \"a\" }", "{ node = \"b\" }"),
                // Floating: out of a's bottom.
                ("e2", "{ node = \"a\" }", "{ node = \"c\" }"),
                // Pinned to a's top whatever the geometry.
                ("e0", "{ node = \"c\" }", "{ node = \"a\", side = \"top\" }"),
                // Dangling: its stub leaves a's right.
                ("e3", "{ node = \"a\" }", "{ node = \"gone\" }"),
            ],
        );
        assert_eq!(connector(&g, &id("a"), Dir::Right), Some(id("e1")));
        assert_eq!(connector(&g, &id("b"), Dir::Left), Some(id("e1")));
        assert_eq!(connector(&g, &id("a"), Dir::Down), Some(id("e2")));
        // Both e0 and e2 leave c's top: the lower id.
        assert_eq!(connector(&g, &id("c"), Dir::Up), Some(id("e0")));
        assert_eq!(connector(&g, &id("a"), Dir::Up), Some(id("e0")));
        assert_eq!(connector(&g, &id("a"), Dir::Left), None);
        assert_eq!(connector(&g, &id("e1"), Dir::Left), None);
        let g = board(
            &[("a", [0, 0], S, "")],
            &[("e3", "{ node = \"a\" }", "{ node = \"gone\" }")],
        );
        assert_eq!(connector(&g, &id("a"), Dir::Right), Some(id("e3")));
    }

    #[test]
    fn the_query_prefers_prefixes_then_reading_order() {
        let g = board(
            &[
                ("a", [0, 0], S, "Send receipt"),
                ("b", [0, 900], S, "End"),
                ("c", [0, 300], S, "Check stock"),
                ("d", [300, 300], S, "Charge card"),
                ("e", [0, 600], S, "CHECKOUT"),
            ],
            &[],
        );
        assert_eq!(query(&g, "end"), Some(id("b")));
        assert_eq!(query(&g, "che"), Some(id("c")));
        assert_eq!(query(&g, "cha"), Some(id("d")));
        assert_eq!(query(&g, "c"), Some(id("c")));
        assert_eq!(query(&g, "ipt"), Some(id("a")));
        assert_eq!(query(&g, "zzz"), None);
        assert_eq!(query(&g, ""), None);
    }
}
