//! Arranging elements: the stacking order (§5.16 rule 2, over §4.7), and
//! aligning and distributing nodes (rule 3).

use std::collections::BTreeSet;

use super::node::carried;
use super::{saturate, Alignment, Axis, Change, Plan, Stack};
use crate::scene::Source;
use crate::z::{self, Element};
use crate::{placements, Board, Edge, Id, Node};

/// Moves the elements `ids` names in the stacking order of the scope each
/// is drawn in, keeping their relative order: above or below every
/// sibling, or one place past the next unselected sibling. Each scope is
/// arranged on its own, since stacking is per parent. Only the moved
/// elements take new keys, except in the renumber fallback of §4.7, which
/// rewrites the scope in the same step. Already at the end changes nothing.
pub(super) fn restack(board: &Board, ids: &[Source], to: Stack) -> Plan {
    let placed = placements(board);
    let mut scopes: BTreeSet<Option<&Id>> = BTreeSet::new();
    for source in ids {
        match source {
            Source::Node(id) => {
                if let Some(placed) = placed.get(id) {
                    scopes.insert(placed.parent);
                }
            }
            Source::Edge(id) => {
                if board.edges.contains_key(id) {
                    scopes.insert(None);
                }
            }
        }
    }
    let chosen: BTreeSet<&Source> = ids.iter().collect();
    let mut changes = Vec::new();
    for parent in scopes {
        let scope = z::scope(board, parent);
        let picked: Vec<bool> = scope
            .iter()
            .map(|(element, _)| chosen.contains(&source(*element)))
            .collect();
        let order = arranged(&picked, to);
        changes.extend(rekey(board, &scope, &order));
    }
    Plan {
        changes,
        ..Plan::default()
    }
}

fn source(element: Element<'_>) -> Source {
    match element {
        Element::Node(id) => Source::Node(id.clone()),
        Element::Edge(id) => Source::Edge(id.clone()),
    }
}

/// The scope's drawing order after the command, as indices into the scope
/// as it is, each with whether that element moves and so needs a new key.
/// `picked` says which elements are selected.
fn arranged(picked: &[bool], to: Stack) -> Vec<(usize, bool)> {
    let n = picked.len();
    match to {
        // The selected elements above the topmost unselected one are
        // already in place.
        Stack::Front => match picked.iter().rposition(|&p| !p) {
            None => (0..n).map(|i| (i, false)).collect(),
            Some(top) => {
                let below = (0..n).filter(|&i| !picked[i]).map(|i| (i, false));
                let raised = (0..n).filter(|&i| picked[i]).map(|i| (i, i < top));
                below.chain(raised).collect()
            }
        },
        Stack::Back => match picked.iter().position(|&p| !p) {
            None => (0..n).map(|i| (i, false)).collect(),
            Some(bottom) => {
                let lowered = (0..n).filter(|&i| picked[i]).map(|i| (i, i > bottom));
                let above = (0..n).filter(|&i| !picked[i]).map(|i| (i, false));
                lowered.chain(above).collect()
            }
        },
        Stack::Forward => forward(picked),
        // Backward is forward in the order read from the top.
        Stack::Backward => {
            let reversed: Vec<bool> = picked.iter().rev().copied().collect();
            forward(&reversed)
                .into_iter()
                .rev()
                .map(|(i, moved)| (n - 1 - i, moved))
                .collect()
        }
    }
}

/// Each run of selected elements passes the one unselected element just
/// above it; a run already at the top stays.
fn forward(picked: &[bool]) -> Vec<(usize, bool)> {
    let n = picked.len();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if !picked[i] {
            out.push((i, false));
            i += 1;
            continue;
        }
        let end = (i..n).find(|&k| !picked[k]).unwrap_or(n);
        if end < n {
            out.push((end, false));
            out.extend((i..end).map(|k| (k, true)));
            i = end + 1;
        } else {
            out.extend((i..end).map(|k| (k, false)));
            i = end;
        }
    }
    out
}

/// The changes that give `scope`, rearranged into `order`, keys in that
/// order: a new key for each moved element between the keys of the
/// elements that stay, or, when one does not fit, the renumber fallback of
/// §4.7: the staying elements are renumbered and the moved ones keyed
/// between them, or, should that not do, the whole scope is numbered.
fn rekey(
    board: &Board,
    scope: &[(Element<'_>, Option<&str>)],
    order: &[(usize, bool)],
) -> Vec<Change> {
    let old: Vec<Option<&str>> = order.iter().map(|&(i, _)| scope[i].1).collect();
    let moved: Vec<bool> = order.iter().map(|&(_, moved)| moved).collect();
    let keys = between(&old, &moved)
        .or_else(|| {
            let staying = moved.iter().filter(|&&moved| !moved).count();
            let mut fresh = z::renumbered(staying).into_iter();
            let renumbered: Vec<Option<String>> = moved
                .iter()
                .map(|&moved| if moved { None } else { fresh.next() })
                .collect();
            let spelled: Vec<Option<&str>> = renumbered.iter().map(Option::as_deref).collect();
            let keys = between(&spelled, &moved)?;
            Some(
                keys.into_iter()
                    .zip(renumbered)
                    .map(|(new, staying)| new.or(staying))
                    .collect(),
            )
        })
        .unwrap_or_else(|| z::renumbered(order.len()).into_iter().map(Some).collect());
    let mut changes = Vec::new();
    for (&(i, _), key) in order.iter().zip(keys) {
        let (element, was) = scope[i];
        let Some(key) = key.filter(|key| was != Some(key.as_str())) else {
            continue;
        };
        changes.push(match element {
            Element::Node(id) => Change::PutNode(
                id.clone(),
                Box::new(Node {
                    z: Some(key),
                    ..board.nodes[id].clone()
                }),
            ),
            Element::Edge(id) => Change::PutEdge(
                id.clone(),
                Box::new(Edge {
                    z: Some(key),
                    ..board.edges[id].clone()
                }),
            ),
        });
    }
    changes
}

/// A new key for each moved element in `keys`' order, each run of them
/// keyed between the staying keys around it, and `None` for each element
/// that stays; or `None` when a key does not fit. A missing staying key
/// reads as empty, as it sorts.
fn between(keys: &[Option<&str>], moved: &[bool]) -> Option<Vec<Option<String>>> {
    let mut out = Vec::with_capacity(keys.len());
    let mut below: Option<String> = None;
    for (i, &moving) in moved.iter().enumerate() {
        if moving {
            let above = (i + 1..keys.len())
                .find(|&k| !moved[k])
                .map(|k| keys[k].unwrap_or(""));
            let key = z::key_between(below.as_deref(), above)?;
            below = Some(key.clone());
            out.push(Some(key));
        } else {
            below = Some(keys[i].unwrap_or("").to_owned());
            out.push(None);
        }
    }
    Some(out)
}

/// A node that align and distribute move: its absolute box.
struct Subject<'a> {
    id: &'a Id,
    at: [i64; 2],
    size: [i64; 2],
}

/// The nodes among `ids` that align and distribute move: each named node
/// that exists and is not drawn inside another of them, which carries it
/// (§5.16 rule 3), with its absolute box. A size below 1 counts as 1, as
/// it is drawn.
fn subjects<'a>(board: &'a Board, ids: &[Id]) -> Vec<Subject<'a>> {
    let placed = placements(board);
    let chosen: BTreeSet<&Id> = ids
        .iter()
        .filter_map(|id| board.nodes.get_key_value(id).map(|(id, _)| id))
        .collect();
    chosen
        .iter()
        .filter(|id| !carried(&placed, &chosen, id))
        .map(|&id| Subject {
            id,
            at: placed[id].at,
            size: board.nodes[id].size.map(|length| i64::from(length.max(1))),
        })
        .collect()
}

/// Puts each node at its new absolute position on one axis, written
/// relative to the parent it is drawn inside; a node already there is
/// left out.
fn placed_at(board: &Board, axis: usize, moves: &[(&Id, i64)]) -> Plan {
    let placed = placements(board);
    let mut changes = Vec::new();
    for &(id, to) in moves {
        let origin = placed[id]
            .parent
            .map_or(0, |parent| placed[parent].at[axis]);
        let node = &board.nodes[id];
        let mut at = node.at;
        at[axis] = saturate(to - origin);
        if at != node.at {
            changes.push(Change::PutNode(
                id.clone(),
                Box::new(Node { at, ..node.clone() }),
            ));
        }
    }
    Plan {
        changes,
        ..Plan::default()
    }
}

/// `n / d` rounded to the nearest integer, half away from zero; `d > 0`.
fn rounded(n: i128, d: i128) -> i64 {
    let half_up = (n.abs() * 2 + d) / (2 * d);
    let value = if n < 0 { -half_up } else { half_up };
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// Lines the nodes up on one edge or centre line of their bounding box
/// (§5.16 rule 3), when at least two move. Positions are not snapped.
///
/// A centre or middle cannot always be shared exactly, since a node of odd
/// width and one of even width have centres half a unit apart. The widest
/// node goes to the box's centre, rounded half away from zero, and every
/// other node is centred on it, rounded the same way. Every other node then
/// lies within the widest one's span, so the box stays that node's and a
/// second run changes nothing.
pub(super) fn align(board: &Board, ids: &[Id], to: Alignment) -> Plan {
    let nodes = subjects(board, ids);
    if nodes.len() < 2 {
        return Plan::default();
    }
    let axis = match to {
        Alignment::Left | Alignment::Center | Alignment::Right => 0,
        Alignment::Top | Alignment::Middle | Alignment::Bottom => 1,
    };
    let low = nodes.iter().map(|node| node.at[axis]).min().unwrap_or(0);
    let high = nodes
        .iter()
        .map(|node| node.at[axis] + node.size[axis])
        .max()
        .unwrap_or(0);
    let widest = nodes.iter().map(|node| node.size[axis]).max().unwrap_or(1);
    let doubled = i128::from(low) + i128::from(high);
    let start = rounded(doubled - i128::from(widest), 2);
    let centre = 2 * i128::from(start) + i128::from(widest);
    let moves: Vec<(&Id, i64)> = nodes
        .iter()
        .map(|node| {
            let to = match to {
                Alignment::Left | Alignment::Top => low,
                Alignment::Right | Alignment::Bottom => high - node.size[axis],
                Alignment::Center | Alignment::Middle => {
                    rounded(centre - i128::from(node.size[axis]), 2)
                }
            };
            (node.id, to)
        })
        .collect();
    placed_at(board, axis, &moves)
}

/// Makes the gaps between neighbouring nodes along `axis` equal, the two
/// outermost staying (§5.16 rule 3), when there are at least three. Nodes
/// are taken in order of their centres, then their starts, then their ids;
/// each is placed from the exact fraction, rounded once, half away from
/// zero, and not snapped. Overlapping nodes give a negative gap.
pub(super) fn distribute(board: &Board, ids: &[Id], axis: Axis) -> Plan {
    let mut nodes = subjects(board, ids);
    if nodes.len() < 3 {
        return Plan::default();
    }
    let axis = match axis {
        Axis::Horizontal => 0,
        Axis::Vertical => 1,
    };
    nodes.sort_by_key(|node| {
        let start = node.at[axis];
        (
            2 * i128::from(start) + i128::from(node.size[axis]),
            start,
            node.id,
        )
    });
    let last = nodes.len() - 1;
    let first = i128::from(nodes[0].at[axis]);
    let sizes: i128 = nodes[..last]
        .iter()
        .map(|node| i128::from(node.size[axis]))
        .sum();
    let free = i128::from(nodes[last].at[axis]) - first - sizes;
    let gaps = last as i128;
    let mut before = 0_i128;
    let mut moves = Vec::with_capacity(last - 1);
    for (k, node) in nodes.iter().enumerate().take(last).skip(1) {
        before += i128::from(nodes[k - 1].size[axis]);
        let exact = (first + before) * gaps + k as i128 * free;
        moves.push((node.id, rounded(exact, gaps)));
    }
    placed_at(board, axis, &moves)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{edges, line_diff, nodes, Listed};
    use super::super::{plan, Intent};
    use super::*;
    use crate::fixtures::Rng;
    use crate::{parse, write};

    /// A root scope of three nodes and a connector, and a scope of two
    /// children inside `a`.
    const STACK: &str = r#"schema = 1

[node.a]
kind = "shape"
at = [0, 0]
size = [200, 200]
z = "V"

[node.b]
kind = "shape"
at = [300, 0]
size = [100, 100]
z = "W"

[node.c]
kind = "shape"
at = [600, 0]
size = [100, 100]
z = "X"

[node.k]
kind = "shape"
at = [20, 20]
parent = "a"
size = [40, 40]
z = "V"

[node.m]
kind = "shape"
at = [100, 20]
parent = "a"
size = [40, 40]
z = "W"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
z = "Y"
"#;

    /// A selection, a command, the `z` lines it changes and the order after.
    type Case<'a, O> = (Vec<Source>, Stack, &'a [&'a str], O);

    fn restacked(text: &str, ids: Vec<Source>, to: Stack) -> String {
        let mut board = parse(text).unwrap();
        let plan = plan(
            &board,
            &Intent::Restack { ids, to },
            &mut Listed(Vec::new()),
        );
        board.apply(&plan.changes);
        write(&board)
    }

    /// The `z` lines that change, each as `<header> <old> <new>`.
    fn rekeyed(text: &str, ids: Vec<Source>, to: Stack) -> Vec<String> {
        assert_eq!(
            write(&parse(text).unwrap()),
            text,
            "the fixture is canonical"
        );
        let after = restacked(text, ids, to);
        let before = parse(text).unwrap();
        let board = parse(&after).unwrap();
        let mut out = Vec::new();
        for (id, node) in &board.nodes {
            if node.z != before.nodes[id].z {
                out.push(format!(
                    "node.{id} {} {}",
                    before.nodes[id].z.as_deref().unwrap_or("-"),
                    node.z.as_deref().unwrap_or("-")
                ));
            }
        }
        for (id, edge) in &board.edges {
            if edge.z != before.edges[id].z {
                out.push(format!(
                    "edge.{id} {} {}",
                    before.edges[id].z.as_deref().unwrap_or("-"),
                    edge.z.as_deref().unwrap_or("-")
                ));
            }
        }
        // Each rekeyed element is its `z` line and nothing else: changed,
        // or added where it had none.
        let diff = line_diff(text, &after);
        let lines: usize = out
            .iter()
            .map(|line| if line.contains(" - ") { 1 } else { 2 })
            .sum();
        assert_eq!(diff.len(), lines, "{diff:?}");
        assert!(
            diff.iter().all(|line| line[1..].starts_with("z = ")),
            "{diff:?}"
        );
        out
    }

    /// The scope `parent` in drawing order, by element.
    fn drawn(text: &str, parent: Option<&str>) -> Vec<String> {
        let board = parse(text).unwrap();
        let parent = parent.map(|name| Id::new(name).unwrap());
        z::scope(&board, parent.as_ref())
            .iter()
            .map(|(element, _)| match element {
                Element::Node(id) => format!("node.{id}"),
                Element::Edge(id) => format!("edge.{id}"),
            })
            .collect()
    }

    #[test]
    fn each_command_on_one_element() {
        use Stack::{Back, Backward, Forward, Front};
        let table: Vec<Case<'_, [&str; 4]>> = vec![
            // Above the connector, the scope's top.
            (
                nodes(&["a"]),
                Front,
                &["node.a V Y01"],
                ["b", "c", "e", "a"],
            ),
            (edges(&["e"]), Back, &["edge.e Y Uzz"], ["e", "a", "b", "c"]),
            // One place past the next sibling.
            (
                nodes(&["a"]),
                Forward,
                &["node.a V WV"],
                ["b", "a", "c", "e"],
            ),
            (
                nodes(&["c"]),
                Forward,
                &["node.c X Y01"],
                ["a", "b", "e", "c"],
            ),
            (
                edges(&["e"]),
                Backward,
                &["edge.e Y WV"],
                ["a", "b", "e", "c"],
            ),
            (
                nodes(&["b"]),
                Backward,
                &["node.b W Uzz"],
                ["b", "a", "c", "e"],
            ),
        ];
        for (ids, to, expected, order) in table {
            let label = format!("{ids:?} {to:?}");
            assert_eq!(rekeyed(STACK, ids.clone(), to), expected, "{label}");
            let after = restacked(STACK, ids, to);
            let order: Vec<String> = order
                .iter()
                .map(|name| {
                    let kind = if *name == "e" { "edge" } else { "node" };
                    format!("{kind}.{name}")
                })
                .collect();
            assert_eq!(drawn(&after, None), order, "{label}");
        }
    }

    #[test]
    fn already_at_the_end_changes_nothing() {
        use Stack::{Back, Backward, Forward, Front};
        for (ids, to) in [
            (edges(&["e"]), Front),
            (edges(&["e"]), Forward),
            (nodes(&["a"]), Back),
            (nodes(&["a"]), Backward),
            (nodes(&["m"]), Front),
            (nodes(&["k"]), Back),
            // The whole scope, at either end.
            ([nodes(&["a", "b", "c"]), edges(&["e"])].concat(), Front),
            ([nodes(&["a", "b", "c"]), edges(&["e"])].concat(), Backward),
            // The top two are already the top.
            ([nodes(&["c"]), edges(&["e"])].concat(), Front),
            ([nodes(&["c"]), edges(&["e"])].concat(), Forward),
            (nodes(&["a", "b"]), Back),
            // Nothing there, or named as the other kind.
            (nodes(&["gone"]), Front),
            (edges(&["a"]), Front),
            (nodes(&["e"]), Back),
            (Vec::new(), Front),
        ] {
            assert!(rekeyed(STACK, ids.clone(), to).is_empty(), "{ids:?} {to:?}");
        }
    }

    #[test]
    fn a_multiple_selection_keeps_its_order() {
        use Stack::{Back, Backward, Forward, Front};
        let table: Vec<Case<'_, &[&str]>> = vec![
            (
                nodes(&["a", "c"]),
                Front,
                &["node.a V Y01", "node.c X Y02"],
                &["node.b", "edge.e", "node.a", "node.c"],
            ),
            // `e` is already above every unselected sibling: only `a` moves.
            (
                [nodes(&["a"]), edges(&["e"])].concat(),
                Front,
                &["node.a V XV"],
                &["node.b", "node.c", "node.a", "edge.e"],
            ),
            (
                [nodes(&["b"]), edges(&["e"])].concat(),
                Back,
                &["node.b W Uzz", "edge.e Y UzzV"],
                &["node.b", "edge.e", "node.a", "node.c"],
            ),
            // A run passes the one sibling above it, together.
            (
                nodes(&["a", "b"]),
                Forward,
                &["node.a V XV", "node.b W Xk"],
                &["node.c", "node.a", "node.b", "edge.e"],
            ),
            // Two runs each pass their own.
            (
                nodes(&["a", "c"]),
                Forward,
                &["node.a V X", "node.c X Y01"],
                &["node.b", "node.a", "edge.e", "node.c"],
            ),
            (
                [nodes(&["b"]), edges(&["e"])].concat(),
                Backward,
                &["node.b W Uzz", "edge.e Y W"],
                &["node.b", "node.a", "edge.e", "node.c"],
            ),
        ];
        for (ids, to, expected, order) in table {
            let label = format!("{ids:?} {to:?}");
            assert_eq!(rekeyed(STACK, ids.clone(), to), expected, "{label}");
            assert_eq!(drawn(&restacked(STACK, ids, to), None), order, "{label}");
        }
    }

    /// Stacking is per parent: a child moves among its siblings only, and
    /// a selection over two scopes arranges each.
    #[test]
    fn a_child_moves_among_its_siblings() {
        assert_eq!(
            rekeyed(STACK, nodes(&["k"]), Stack::Front),
            ["node.k V W01"]
        );
        assert_eq!(
            rekeyed(STACK, nodes(&["m"]), Stack::Backward),
            ["node.m W Uzz"]
        );
        assert_eq!(
            rekeyed(STACK, nodes(&["k", "a"]), Stack::Front),
            ["node.a V Y01", "node.k V W01"]
        );
        let after = restacked(STACK, nodes(&["k", "a"]), Stack::Front);
        assert_eq!(drawn(&after, Some("a")), ["node.m", "node.k"]);
        // A hand-written parent and keys: the scope is the drawn one.
        let text = "schema = 1\n\n[node.box]\nkind = \"shape\"\nat = [0, 0]\nsize = [400, 400]\nz = \"a\"\n\n\
                    [node.in1]\nkind = \"shape\"\nat = [0, 0]\nparent = \"box\"\nsize = [20, 20]\nz = \"a1\"\n\n\
                    [node.in2]\nkind = \"shape\"\nat = [40, 0]\nparent = \"box\"\nsize = [20, 20]\nz = \"a2\"\n\n\
                    [node.lost]\nkind = \"shape\"\nat = [600, 0]\nparent = \"gone\"\nsize = [20, 20]\nz = \"c\"\n\n\
                    [node.out]\nkind = \"shape\"\nat = [600, 0]\nsize = [20, 20]\nz = \"b\"\n";
        assert_eq!(
            rekeyed(text, nodes(&["in1"]), Stack::Forward),
            ["node.in1 a1 a21"]
        );
        // A node whose parent names nothing is a root, above `out`.
        assert_eq!(
            rekeyed(text, nodes(&["lost"]), Stack::Backward),
            ["node.lost c aV"]
        );
    }

    #[test]
    fn ties_fall_to_the_id_and_a_key_that_fits_nowhere_renumbers() {
        let node = |id: &str, z: Option<&str>| {
            let z = z.map_or(String::new(), |z| format!("z = \"{z}\"\n"));
            format!("\n[node.{id}]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\n{z}")
        };
        let board = |nodes: &[(&str, Option<&str>)]| {
            let mut text = String::from("schema = 1\n");
            for (id, z) in nodes {
                text += &node(id, *z);
            }
            text
        };
        // Equal keys: `p` is below `q` by id, and passes it with a key
        // between theirs and `r`'s.
        let tied = board(&[("p", Some("V")), ("q", Some("V")), ("r", Some("W"))]);
        assert_eq!(
            rekeyed(&tied, nodes(&["p"]), Stack::Forward),
            ["node.p V VV"]
        );
        assert_eq!(
            rekeyed(&tied, nodes(&["q"]), Stack::Backward),
            ["node.q V Uzz"]
        );
        // No key between two equal ones: the scope is renumbered, the
        // moved node keyed in the gap, all in one step.
        let three = board(&[("p", Some("V")), ("q", Some("V")), ("r", Some("V"))]);
        assert_eq!(
            rekeyed(&three, nodes(&["p"]), Stack::Forward),
            ["node.p V V02", "node.q V V01", "node.r V V03"]
        );
        assert_eq!(
            drawn(&restacked(&three, nodes(&["p"]), Stack::Forward), None),
            ["node.q", "node.p", "node.r"]
        );
        // A key outside the alphabet, or past 64 characters, next to the
        // slot.
        let long = "z".repeat(70);
        for odd in ["a.b", long.as_str()] {
            let text = board(&[("p", Some("V")), ("q", Some(odd))]);
            assert_eq!(
                rekeyed(&text, nodes(&["p"]), Stack::Front),
                ["node.p V V02".to_owned(), format!("node.q {odd} V01")]
            );
        }
        // A missing key sorts as empty: a key goes above it without a
        // renumber, and nothing below it without one.
        let missing = board(&[("p", None), ("q", Some("V"))]);
        assert_eq!(
            rekeyed(&missing, nodes(&["q"]), Stack::Backward),
            ["node.p - V01", "node.q V Uzz"]
        );
        assert_eq!(
            rekeyed(&missing, nodes(&["p"]), Stack::Forward),
            ["node.p - V01"]
        );
    }

    /// The order a scope is drawn in after `to`, worked out from the
    /// selection alone: front and back by partition, forward and backward
    /// by swapping each selected run with the element beyond it.
    fn expected(order: &[usize], picked: &BTreeSet<usize>, to: Stack) -> Vec<usize> {
        let (selected, rest): (Vec<usize>, Vec<usize>) =
            order.iter().partition(|i| picked.contains(i));
        match to {
            Stack::Front => [rest, selected].concat(),
            Stack::Back => [selected, rest].concat(),
            Stack::Forward | Stack::Backward => {
                let mut out: Vec<usize> = order.to_vec();
                if to == Stack::Backward {
                    out.reverse();
                }
                let mut i = 0;
                while i < out.len() {
                    if !picked.contains(&out[i]) {
                        i += 1;
                        continue;
                    }
                    let mut end = i;
                    while end < out.len() && picked.contains(&out[end]) {
                        end += 1;
                    }
                    if end < out.len() {
                        out[i..=end].rotate_right(1);
                        i = end + 1;
                    } else {
                        i = end;
                    }
                }
                if to == Stack::Backward {
                    out.reverse();
                }
                out
            }
        }
    }

    /// Runs `to` on the root scope of `board` with `picked` selected, and
    /// checks the drawn order against [`expected`] and that only the moved
    /// elements were rekeyed, unless the scope was renumbered. Gives the
    /// board after and whether it was renumbered.
    fn step(board: &Board, picked: &BTreeSet<usize>, to: Stack) -> (Board, bool) {
        let scope = z::scope(board, None);
        let elements: Vec<Source> = scope.iter().map(|(e, _)| source(*e)).collect();
        let ids: Vec<Source> = picked.iter().map(|&i| elements[i].clone()).collect();
        let plan = plan(board, &Intent::Restack { ids, to }, &mut Listed(Vec::new()));
        let mut after = board.clone();
        after.apply(&plan.changes);
        let now: Vec<Source> = z::scope(&after, None)
            .iter()
            .map(|(e, _)| source(*e))
            .collect();
        let order: Vec<usize> = (0..elements.len()).collect();
        let want: Vec<Source> = expected(&order, picked, to)
            .into_iter()
            .map(|i| elements[i].clone())
            .collect();
        assert_eq!(now, want, "{to:?} {picked:?}\n{}", write(board));
        let rekeyed: Vec<usize> = (0..elements.len())
            .filter(|&i| key_of(&after, &elements[i]) != key_of(board, &elements[i]))
            .collect();
        let renumbered = rekeyed.iter().any(|i| !picked.contains(i));
        for i in &rekeyed {
            let key = key_of(&after, &elements[*i]).unwrap();
            assert!(
                (1..=z::MAX_LEN).contains(&key.len())
                    && key.bytes().all(|c| c.is_ascii_alphanumeric())
                    && !key.ends_with('0'),
                "{key}"
            );
        }
        (after, renumbered)
    }

    fn key_of(board: &Board, element: &Source) -> Option<String> {
        match element {
            Source::Node(id) => board.nodes[id].z.clone(),
            Source::Edge(id) => board.edges[id].z.clone(),
        }
    }

    /// Two neighbours passing each other forward, over and over, halve one
    /// gap each time until no key fits; the scope is then renumbered and
    /// it goes on.
    #[test]
    fn five_hundred_alternating_steps_hold_and_renumber() {
        let text =
            "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n\n\
                    [node.x]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"W\"\n\n\
                    [node.y]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"X\"\n\n\
                    [node.b]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"Y\"\n";
        let mut board = parse(text).unwrap();
        let mut renumbers = 0;
        for _ in 0..500 {
            // The one of x and y that is lower passes the other.
            let (after, renumbered) = step(&board, &BTreeSet::from([1]), Stack::Forward);
            board = after;
            renumbers += usize::from(renumbered);
        }
        assert!(renumbers >= 1, "{renumbers}");
        // Front and back of one element, alternating, never grow a key.
        let mut board = parse(STACK).unwrap();
        for k in 0..500 {
            let to = if k % 2 == 0 {
                Stack::Front
            } else {
                Stack::Back
            };
            let (after, renumbered) = step(&board, &BTreeSet::from([1]), to);
            assert!(!renumbered);
            board = after;
        }
        let longest = z::scope(&board, None)
            .iter()
            .map(|(_, key)| key.map_or(0, str::len))
            .max();
        assert_eq!(longest, Some(3));
    }

    /// Random scopes, with ties, missing and unusable keys, and random
    /// selections: every command gives the order worked out from the
    /// selection alone, and rekeys only what it moved unless it renumbers.
    #[test]
    fn random_scopes_take_every_command() {
        let mut rng = Rng(11);
        const KEYS: &[&str] = &[
            "V", "V", "W", "a1", "a10", "", "0", "a.b", "zzz", "Uzz", "V0",
        ];
        let commands = [Stack::Front, Stack::Back, Stack::Forward, Stack::Backward];
        for _ in 0..500 {
            let mut text = String::from("schema = 1\n");
            for k in 0..2 + rng.below(8) {
                let z = match rng.below(12) {
                    0 => String::new(),
                    _ => format!("z = \"{}\"\n", rng.pick(KEYS)),
                };
                let kind = if rng.one_in(3) { "edge" } else { "node" };
                if kind == "edge" {
                    text += &format!(
                        "\n[edge.e{k}]\nkind = \"edge\"\nfrom = {{ at = [0, 0] }}\nto = {{ at = [9, 9] }}\n{z}"
                    );
                } else {
                    text += &format!(
                        "\n[node.n{k}]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\n{z}"
                    );
                }
            }
            let mut board = parse(&text).unwrap();
            for _ in 0..4 {
                let count = z::scope(&board, None).len();
                let picked: BTreeSet<usize> = (0..count).filter(|_| rng.one_in(3)).collect();
                let to = *rng.pick(&commands);
                board = step(&board, &picked, to).0;
            }
        }
    }

    /// Three nodes of different sizes, one with an odd width, a container
    /// `p` holding `q`, and a connector.
    const ROW: &str = r#"schema = 1

[node.a]
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "V"

[node.b]
kind = "shape"
at = [200, 100]
size = [60, 40]
z = "W"

[node.c]
kind = "shape"
at = [400, 30]
size = [81, 20]
z = "X"

[node.p]
kind = "shape"
at = [0, 300]
size = [300, 200]
z = "Y"

[node.q]
kind = "shape"
at = [40, 40]
parent = "p"
size = [40, 40]
z = "V"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
z = "Z"
"#;

    fn ids(names: &[&str]) -> Vec<Id> {
        names.iter().map(|name| Id::new(name).unwrap()).collect()
    }

    /// The board after `intent` on `text`, and the plan for the same
    /// intent run again on it, which must change nothing.
    fn twice(text: &str, intent: &Intent) -> String {
        let mut board = parse(text).unwrap();
        let first = plan(&board, intent, &mut Listed(Vec::new()));
        board.apply(&first.changes);
        let again = plan(&board, intent, &mut Listed(Vec::new()));
        assert!(again.is_empty(), "{intent:?} again: {again:?}");
        write(&board)
    }

    /// The `at` of each node `intent` moves, by name, in id order.
    fn moved(text: &str, intent: &Intent) -> Vec<(String, [i32; 2])> {
        assert_eq!(
            write(&parse(text).unwrap()),
            text,
            "the fixture is canonical"
        );
        let after = twice(text, intent);
        let (before, board) = (parse(text).unwrap(), parse(&after).unwrap());
        // Only `at` lines change, one per moved node.
        let diff = line_diff(text, &after);
        assert!(
            diff.iter().all(|line| line[1..].starts_with("at = ")),
            "{diff:?}"
        );
        let out: Vec<(String, [i32; 2])> = board
            .nodes
            .iter()
            .filter(|(id, node)| node.at != before.nodes[*id].at)
            .map(|(id, node)| (id.as_str().to_owned(), node.at))
            .collect();
        assert_eq!(diff.len(), 2 * out.len(), "{diff:?}");
        out
    }

    fn at(cases: &[(&str, [i32; 2])]) -> Vec<(String, [i32; 2])> {
        cases
            .iter()
            .map(|(id, at)| ((*id).to_owned(), *at))
            .collect()
    }

    #[test]
    fn align_moves_each_node_to_the_bounding_box() {
        use Alignment::{Bottom, Center, Left, Middle, Right, Top};
        let abc = ids(&["a", "b", "c", "e"]);
        let table = [
            (Left, at(&[("b", [0, 100]), ("c", [0, 30])])),
            (Right, at(&[("a", [361, 0]), ("b", [421, 100])])),
            // The box is 0 to 481; the widest, `a`, goes to its centre,
            // 180.5 rounded away from zero, and the rest centre on `a`:
            // `c`, of odd width, at 200.5 rounded.
            (
                Center,
                at(&[("a", [181, 0]), ("b", [211, 100]), ("c", [201, 30])]),
            ),
            (Top, at(&[("b", [200, 0]), ("c", [400, 0])])),
            (Bottom, at(&[("a", [0, 80]), ("c", [400, 120])])),
            (
                Middle,
                at(&[("a", [0, 40]), ("b", [200, 50]), ("c", [400, 60])]),
            ),
        ];
        for (to, expected) in table {
            let intent = Intent::Align {
                ids: abc.clone(),
                to,
            };
            assert_eq!(moved(ROW, &intent), expected, "{to:?}");
        }
    }

    #[test]
    fn align_needs_two_nodes_and_moves_an_ancestor_only() {
        let align = |names: &[&str], to| Intent::Align {
            ids: ids(names),
            to,
        };
        let board = parse(ROW).unwrap();
        for intent in [
            align(&["a"], Alignment::Left),
            align(&["a", "e", "gone"], Alignment::Right),
            align(&[], Alignment::Top),
            // `q` is carried by `p`, which is alone.
            align(&["p", "q"], Alignment::Right),
            // Already aligned.
            align(&["a", "p"], Alignment::Left),
        ] {
            let plan = plan(&board, &intent, &mut Listed(Vec::new()));
            assert!(plan.is_empty(), "{intent:?}");
        }
        // A child is placed in absolute units and written relative to its
        // parent, which stays.
        assert_eq!(
            moved(ROW, &align(&["q", "b"], Alignment::Top)),
            at(&[("q", [40, -200])])
        );
        assert_eq!(
            moved(ROW, &align(&["q", "a"], Alignment::Left)),
            at(&[("q", [0, 40])])
        );
        // With its ancestor selected the child is not moved itself, nor
        // counted in the box.
        assert_eq!(
            moved(ROW, &align(&["p", "q", "c"], Alignment::Right)),
            at(&[("p", [181, 300])])
        );
    }

    #[test]
    fn distribute_makes_the_gaps_equal_between_the_outermost_two() {
        let distribute = |names: &[&str], axis| Intent::Distribute {
            ids: ids(names),
            axis,
        };
        // `a` ends at 120 and `c` starts at 400: gaps of 110 around `b`.
        assert_eq!(
            moved(ROW, &distribute(&["a", "b", "c"], Axis::Horizontal)),
            at(&[("b", [230, 100])])
        );
        // By centre the order is `a`, `c`, `b`: gaps of 10.
        assert_eq!(
            moved(ROW, &distribute(&["c", "b", "a", "e"], Axis::Vertical)),
            at(&[("c", [400, 70])])
        );
        // `q` goes with `p`, which is one of three: `c` between `a`,
        // ending at 60, and `p`, starting at 300.
        assert_eq!(
            moved(ROW, &distribute(&["a", "p", "q", "c"], Axis::Vertical)),
            at(&[("c", [400, 170])])
        );
        let board = parse(ROW).unwrap();
        for intent in [
            distribute(&["a", "b"], Axis::Horizontal),
            distribute(&["a", "b", "e"], Axis::Vertical),
            distribute(&["p", "q", "a"], Axis::Horizontal),
        ] {
            let plan = plan(&board, &intent, &mut Listed(Vec::new()));
            assert!(plan.is_empty(), "{intent:?}");
        }
    }

    /// Three nodes of `width` at the given starts on the x axis.
    fn three(starts: [i32; 3], width: i32) -> String {
        let mut text = String::from("schema = 1\n");
        for (name, x) in ["n0", "n1", "n2"].iter().zip(starts) {
            text += &format!(
                "\n[node.{name}]\nkind = \"shape\"\nat = [{x}, 0]\nsize = [{width}, 20]\nz = \"V\"\n"
            );
        }
        text
    }

    #[test]
    fn distribute_rounds_once_away_from_zero_and_takes_overlaps() {
        let all = Intent::Distribute {
            ids: ids(&["n0", "n1", "n2"]),
            axis: Axis::Horizontal,
        };
        // Gaps of 10.5: -60.5 and 60.5 round away from zero.
        assert_eq!(
            moved(&three([-101, -50, -40], 20), &all),
            at(&[("n1", [-71, 0])])
        );
        assert_eq!(
            moved(&three([40, 50, 101], 20), &all),
            at(&[("n1", [71, 0])])
        );
        // Overlapping nodes give equal negative gaps.
        assert_eq!(
            moved(&three([0, 50, 60], 100), &all),
            at(&[("n1", [30, 0])])
        );
        // All in one place: already equal, at -width.
        assert!(moved(&three([7, 7, 7], 20), &all).is_empty());
    }

    /// Random rows of nodes: align is idempotent always, and distribute
    /// whenever its result keeps the nodes' order, which holds while no
    /// two neighbours overlap by more than their sizes.
    #[test]
    fn random_rows_align_and_distribute_once() {
        let mut rng = Rng(23);
        let alignments = [
            Alignment::Left,
            Alignment::Center,
            Alignment::Right,
            Alignment::Top,
            Alignment::Middle,
            Alignment::Bottom,
        ];
        for _ in 0..500 {
            let mut text = String::from("schema = 1\n");
            let count = 2 + rng.below(6);
            let mut x = rng.below(400) as i32 - 200;
            for k in 0..count {
                let (w, h) = (1 + rng.below(160) as i32, 1 + rng.below(160) as i32);
                let y = rng.below(600) as i32 - 300;
                let parent = if k > 0 && rng.one_in(4) {
                    format!("parent = \"n{}\"\n", k - 1)
                } else {
                    String::new()
                };
                text += &format!(
                    "\n[node.n{k}]\nkind = \"shape\"\nat = [{x}, {y}]\n{parent}size = [{w}, {h}]\nz = \"V\"\n"
                );
                x += w + rng.below(100) as i32;
            }
            let names: Vec<Id> = (0..count)
                .filter(|_| !rng.one_in(5))
                .map(|k| Id::new(&format!("n{k}")).unwrap())
                .collect();
            let to = *rng.pick(&alignments);
            twice(
                &text,
                &Intent::Align {
                    ids: names.clone(),
                    to,
                },
            );
            let board = parse(&text).unwrap();
            let intent = Intent::Distribute {
                ids: names,
                axis: if rng.one_in(2) {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                },
            };
            let mut after = board.clone();
            after.apply(&plan(&board, &intent, &mut Listed(Vec::new())).changes);
            let again = plan(&after, &intent, &mut Listed(Vec::new()));
            let order = |board: &Board| {
                let Intent::Distribute { ids, axis } = &intent else {
                    unreachable!()
                };
                let axis = usize::from(*axis == Axis::Vertical);
                let mut nodes = subjects(board, ids);
                nodes.sort_by_key(|node| {
                    let start = node.at[axis];
                    (
                        2 * i128::from(start) + i128::from(node.size[axis]),
                        start,
                        node.id.clone(),
                    )
                });
                nodes.iter().map(|node| node.id.clone()).collect::<Vec<_>>()
            };
            if order(&board) == order(&after) {
                assert!(again.is_empty(), "{intent:?}\n{text}");
            }
        }
    }
}
