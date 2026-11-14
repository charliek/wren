//! Arranging elements: the stacking order (§5.16 rule 2, over §4.7).

use std::collections::BTreeSet;

use super::{Change, Plan, Stack};
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
}
