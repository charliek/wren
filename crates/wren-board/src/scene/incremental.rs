//! The differential test of [`Cache::build`]: on boards of every kind the
//! generators make, after each step of a random sequence of changes, the
//! scene a cache gives is the scene a build of the whole board gives, item
//! for item in its debug dump.

use std::fmt::Write as _;

use super::tests::random_board;
use super::*;
use crate::fixtures::Rng;
use crate::style::tests::{DARK, LIGHT};
use crate::{parse, Color, EdgeStyle, Extra, Head, Label, NodeStyle, Route, Token, Vocabulary};

/// Where a generated board puts node `i`.
type At = Box<dyn Fn(usize) -> [i64; 2]>;

/// The boards the perf tests time and the shapes a flowchart takes, at
/// `nodes` shapes: the generator's name and the board.
fn generated(kind: usize, nodes: usize, rng: &mut Rng) -> (&'static str, Board) {
    let grid = |i: usize| [(i % 40) as i64 * 200, (i / 40) as i64 * 100];
    let mut edges: Vec<(usize, usize, String, String)> = Vec::new();
    let plain = |a: usize, b: usize| (a, b, String::new(), String::new());
    let (name, count, at, size): (_, usize, At, [i64; 2]) = match kind {
        0 => {
            edges.extend(
                (0..nodes)
                    .filter(|i| i % 40 != 39 && i + 1 < nodes)
                    .map(|i| plain(i, i + 1)),
            );
            ("grid", nodes, Box::new(grid), [160, 64])
        }
        1 => {
            edges.extend(
                (0..nodes)
                    .filter(|i| i % 40 != 39)
                    .map(|i| plain(i, (i + 41).min(nodes - 1))),
            );
            ("diagonal", nodes, Box::new(grid), [160, 64])
        }
        2 => {
            edges.extend((0..nodes).map(|i| plain(i, rng.below(nodes))));
            ("long random", nodes, Box::new(grid), [160, 64])
        }
        3 => {
            edges.extend((1..nodes).map(|i| plain(i, 0)));
            ("fan-in", nodes, Box::new(grid), [160, 64])
        }
        4 => {
            let far = nodes;
            edges.extend((0..nodes).map(|i| plain(i, far)));
            let at = move |i: usize| {
                if i == far {
                    [5_000, 5_000]
                } else {
                    [(i % 7) as i64, (i % 5) as i64]
                }
            };
            ("stacked", nodes + 1, Box::new(at), [100, 60])
        }
        5 => {
            // Every connector pinned to a side, or anchored, at each end.
            let sides = ["top", "right", "bottom", "left"];
            for i in 0..nodes {
                let b = rng.below(nodes);
                let end = |rng: &mut Rng| match rng.below(3) {
                    0 => format!(", anchor = [{}, {}]", rng.below(1001), rng.below(1001)),
                    _ => format!(", side = \"{}\"", rng.pick(&sides)),
                };
                edges.push((i, b, end(rng), end(rng)));
            }
            ("pinned sides", nodes, Box::new(grid), [160, 64])
        }
        _ => {
            // Free ends, some at both ends, among the grid.
            for i in 0..nodes {
                edges.push((i, rng.below(nodes), String::new(), String::new()));
            }
            ("free ends", nodes, Box::new(grid), [160, 64])
        }
    };
    let shapes = Shape::ALL;
    let mut text = String::from("schema = 1\n");
    for i in 0..count {
        let [x, y] = at(i);
        let _ = write!(
            text,
            "\n[node.n{i:04}]\ntext = \"Step {i}\\nchecks the order\"\nkind = \"shape\"\n\
             at = [{x}, {y}]\nshape = \"{}\"\nsize = [{}, {}]\nz = \"V{i}\"\n",
            shapes[i % shapes.len()].name(),
            size[0],
            size[1]
        );
    }
    for (k, (a, b, from, to)) in edges.into_iter().enumerate() {
        let free_ends = name == "free ends";
        let from = if free_ends && rng.one_in(2) {
            format!("{{ at = [{}, {}] }}", rng.below(8000), rng.below(2500))
        } else {
            format!("{{ node = \"n{a:04}\"{from} }}")
        };
        let to = if free_ends && rng.one_in(3) {
            format!("{{ at = [{}, {}] }}", rng.below(8000), rng.below(2500))
        } else {
            format!("{{ node = \"n{b:04}\"{to} }}")
        };
        let _ = write!(
            text,
            "\n[edge.e{k:04}]\nkind = \"edge\"\nfrom = {from}\nto = {to}\n\
             labels = [{{ text = \"next\" }}]\nz = \"W{k}\"\n"
        );
    }
    (name, parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}")))
}

/// Nodes nested in frames up to four deep, with connectors between nodes
/// at every level.
fn nested(nodes: usize, rng: &mut Rng) -> Board {
    let mut text = String::from("schema = 1\n");
    for i in 0..nodes {
        let parent = if i > 0 && !rng.one_in(4) {
            format!("parent = \"n{:04}\"\n", rng.below(i))
        } else {
            String::new()
        };
        let kind = *rng.pick(&["shape", "shape", "frame", "text"]);
        let _ = write!(
            text,
            "\n[node.n{i:04}]\ntext = \"Box {i}\"\nkind = \"{kind}\"\nat = [{}, {}]\n\
             size = [{}, {}]\n{parent}z = \"V{}\"\n",
            rng.below(600) as i64 - 100,
            rng.below(400) as i64 - 100,
            40 + rng.below(300),
            30 + rng.below(200),
            rng.below(20)
        );
    }
    for k in 0..nodes {
        let _ = write!(
            text,
            "\n[edge.e{k:04}]\nkind = \"edge\"\nfrom = {{ node = \"n{:04}\" }}\n\
             to = {{ node = \"n{:04}\" }}\nroute = \"{}\"\n",
            rng.below(nodes),
            rng.below(nodes),
            rng.pick(&["elbow", "elbow", "straight"])
        );
    }
    parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn some_id<'a>(rng: &mut Rng, ids: impl Iterator<Item = &'a Id>) -> Option<Id> {
    let ids: Vec<&Id> = ids.collect();
    (!ids.is_empty()).then(|| (*rng.pick(&ids)).clone())
}

fn end(rng: &mut Rng, board: &Board) -> End {
    let node = some_id(rng, board.nodes.keys());
    match (rng.below(6), node) {
        (0, _) | (_, None) => End::Free {
            at: [rng.below(4000) as i32 - 500, rng.below(3000) as i32 - 500],
            extra: Extra::new(),
        },
        (1, Some(node)) => End::Node {
            node: node.to_string(),
            side: Some(Named::Known(*rng.pick(Side::ALL))),
            anchor: None,
            extra: Extra::new(),
        },
        (2, Some(node)) => End::Node {
            node: node.to_string(),
            side: None,
            anchor: Some([rng.below(1200) as i32 - 100, rng.below(1200) as i32 - 100]),
            extra: Extra::new(),
        },
        (3, _) => End::Node {
            node: "gone".into(),
            side: None,
            anchor: None,
            extra: Extra::new(),
        },
        (_, Some(node)) => End::Node {
            node: node.to_string(),
            side: None,
            anchor: None,
            extra: Extra::new(),
        },
    }
}

/// An offset: a step of the grid, a few, or far across the board.
fn by(rng: &mut Rng) -> [i32; 2] {
    let reach = *rng.pick(&[20, 200, 2000, 9000]);
    let d = |rng: &mut Rng| rng.below(2 * reach + 1) as i32 - reach as i32;
    [d(rng), d(rng)]
}

/// One random change to `board`, as an edit could make it; `history` holds
/// the boards before, for an undo. Returns what it did.
fn change(
    rng: &mut Rng,
    board: &mut Board,
    history: &mut Vec<Board>,
    fresh: &mut u32,
) -> &'static str {
    let node = some_id(rng, board.nodes.keys());
    let edge = some_id(rng, board.edges.keys());
    let what = rng.below(15);
    if what == 0 && !history.is_empty() {
        let back = rng.below(history.len().min(4)) + 1;
        let at = history.len() - back;
        *board = history[at].clone();
        history.truncate(at);
        return "undo";
    }
    history.push(board.clone());
    match (what, node, edge) {
        (1, Some(id), _) => {
            let n = board.nodes.get_mut(&id).unwrap();
            let [dx, dy] = by(rng);
            n.at = [n.at[0].saturating_add(dx), n.at[1].saturating_add(dy)];
            "move a node"
        }
        (2, Some(_), _) => {
            let [dx, dy] = by(rng);
            let ids: Vec<Id> = board.nodes.keys().cloned().collect();
            for id in ids.iter().filter(|_| rng.one_in(3)) {
                let n = board.nodes.get_mut(id).unwrap();
                n.at = [n.at[0].saturating_add(dx), n.at[1].saturating_add(dy)];
            }
            "move several"
        }
        (3, Some(id), _) => {
            let n = board.nodes.get_mut(&id).unwrap();
            n.size = [20 + rng.below(400) as i32, 20 + rng.below(300) as i32];
            "resize"
        }
        (4, Some(id), _) => {
            let n = board.nodes.get_mut(&id).unwrap();
            let words = [
                "Start",
                "checks",
                "the order",
                "\n",
                "a",
                "注文",
                "long-unbroken-word",
                " ",
            ];
            n.text = (0..rng.below(5)).map(|_| *rng.pick(&words)).collect();
            "change a label"
        }
        (5, _, _) => {
            *fresh += 1;
            let id = Id::new(&format!("new{fresh}")).unwrap();
            let node = Node {
                text: "New".into(),
                kind: Some(Named::Known(NodeKind::Shape)),
                at: [rng.below(8000) as i32 - 200, rng.below(2600) as i32 - 200],
                parent: None,
                shape: Named::Known(*rng.pick(Shape::ALL)),
                size: [120, 60],
                style: NodeStyle::default(),
                z: Some(format!("V{}", rng.below(50))),
                extra: Extra::new(),
            };
            board.nodes.insert(id, node);
            "add a node"
        }
        (6, Some(id), _) => {
            board.nodes.remove(&id);
            if rng.one_in(2) {
                board.edges.retain(|_, e| {
                    ![&e.from, &e.to]
                        .iter()
                        .any(|end| matches!(end, End::Node { node, .. } if *node == id.as_str()))
                });
            }
            "delete a node"
        }
        (7, _, _) => {
            *fresh += 1;
            // Sorts anywhere among the connectors: before, between or after.
            let name = format!("{}{fresh}", rng.pick(&["a", "e0", "e05", "f", "z"]));
            let edge = Edge {
                kind: Some(Named::Known(crate::EdgeKind::Edge)),
                from: end(rng, board),
                to: end(rng, board),
                heads: Edge::DEFAULT_HEADS,
                labels: vec![Label {
                    text: "new".into(),
                    at: Label::DEFAULT_AT,
                    extra: Extra::new(),
                }],
                route: Named::Known(*rng.pick(&[Route::Elbow, Route::Elbow, Route::Straight])),
                style: EdgeStyle::default(),
                z: None,
                extra: Extra::new(),
            };
            board.edges.insert(Id::new(&name).unwrap(), edge);
            "add a connector"
        }
        (8, _, Some(id)) => {
            board.edges.remove(&id);
            "delete a connector"
        }
        (9, Some(id), _) => {
            let parent = if rng.one_in(3) {
                None
            } else {
                some_id(rng, board.nodes.keys()).map(|p| p.to_string())
            };
            board.nodes.get_mut(&id).unwrap().parent = parent;
            "re-parent"
        }
        (10, _, Some(id)) => {
            let fresh_end = end(rng, board);
            let e = board.edges.get_mut(&id).unwrap();
            if rng.one_in(2) {
                e.from = fresh_end;
            } else {
                e.to = fresh_end;
            }
            "change an end"
        }
        (11, _, Some(id)) => {
            let e = board.edges.get_mut(&id).unwrap();
            match rng.below(4) {
                0 => e.route = Named::Known(*rng.pick(&[Route::Elbow, Route::Straight])),
                1 => e.heads = [Named::Known(Head::Arrow), Named::Known(Head::None)],
                2 => e.labels[..].iter_mut().for_each(|l| l.text = "Yes".into()),
                _ => e.style.stroke = Named::Known(Dash::Dashed),
            }
            "restyle a connector"
        }
        (12, Some(id), _) => {
            let n = board.nodes.get_mut(&id).unwrap();
            n.style.color = Color::Token(*rng.pick(&[Token::Red, Token::Blue, Token::Neutral]));
            n.style.fill = Named::Known(*rng.pick(&[Fill::None, Fill::Solid, Fill::Tint]));
            "restyle a node"
        }
        (13, Some(id), _) => {
            board.nodes.get_mut(&id).unwrap().z = Some(format!("{}", rng.below(100)));
            "restack"
        }
        (_, Some(id), _) => {
            let shape = *rng.pick(Shape::ALL);
            board.nodes.get_mut(&id).unwrap().shape = Named::Known(shape);
            "reshape"
        }
        _ => {
            history.pop();
            "nothing"
        }
    }
}

/// `steps` random changes to `board`, a cache following each: after every
/// one the cached scene equals a whole build's. Returns how many connectors
/// were routed, and how many there were, over all the steps.
#[track_caller]
fn follow(name: &str, mut board: Board, steps: usize, rng: &mut Rng) -> (usize, usize) {
    let mut cache = Cache::default();
    let mut history = Vec::new();
    let mut fresh = 0;
    let (mut routed, mut connectors) = (0, 0);
    let mut palette = LIGHT;
    for step in 0..=steps {
        let what = if step == 0 {
            "open"
        } else if rng.one_in(25) {
            palette = if palette == LIGHT { DARK } else { LIGHT };
            "change the palette"
        } else if rng.one_in(40) {
            cache.relayout();
            "lay out again"
        } else {
            change(rng, &mut board, &mut history, &mut fresh)
        };
        let whole = build(&board, &palette, &FixedMeasure);
        let kept = cache.build(&board, &palette, &FixedMeasure);
        let (a, b) = (format!("{whole:?}"), format!("{kept:?}"));
        if a != b {
            let first = a
                .split("Item {")
                .zip(b.split("Item {"))
                .position(|(x, y)| x != y)
                .unwrap_or(0);
            panic!(
                "{name}, step {step} ({what}): the cached scene differs, first at item {first}\n\
                 whole: {}\ncached: {}",
                a.split("Item {").nth(first).unwrap_or(""),
                b.split("Item {").nth(first).unwrap_or("")
            );
        }
        if step > 0 {
            routed += cache.rebuilt().routed;
            connectors += board.edges.len();
        }
    }
    (routed, connectors)
}

fn differential(seed: u64, boards: usize, nodes: usize, steps: usize) {
    let mut rng = Rng(seed);
    let (mut routed, mut connectors) = (0, 0);
    for round in 0..boards {
        let kind = round % 9;
        let (name, board) = match kind {
            0..=6 => generated(kind, nodes, &mut rng),
            7 => ("nested parents", nested(nodes / 2 + 2, &mut rng)),
            _ => ("the fuzz generator", random_board(&mut rng)),
        };
        let (r, c) = follow(&format!("{name} #{round}"), board, steps, &mut rng);
        routed += r;
        connectors += c;
    }
    // The cache earns its keep: most connectors are taken from it.
    assert!(
        routed * 2 < connectors,
        "{routed} of {connectors} routed again"
    );
}

#[test]
fn a_cached_scene_is_the_whole_build_after_every_change() {
    differential(21, 18, 24, 24);
}

/// The case the full count first found: on the third board, a long
/// random one, a node moved far enough that a connector after another in
/// id order went from fixed to free, so it left the lines the earlier one
/// had routed against.
#[test]
fn a_connector_that_turns_free_takes_its_lines_from_those_before_it() {
    differential(22, 3, 60, 30);
}

#[test]
#[ignore = "full count; run by make fuzz-board"]
fn fuzz_a_cached_scene_is_the_whole_build_after_every_change() {
    differential(22, 900, 60, 30);
    differential(23, 45, 400, 24);
}

/// What a change costs: a label typed routes nothing and lays out one
/// node; a node moved on a grid of neighbour connectors routes only the
/// connectors near it; the release that ends a drag, the board its last
/// preview showed, routes nothing.
#[test]
fn a_change_routes_and_lays_out_only_what_it_can_affect() {
    let mut rng = Rng(5);
    let (_, mut board) = generated(0, 400, &mut rng);
    let mut cache = Cache::default();
    let built = |cache: &mut Cache, board: &Board| {
        let scene = cache.build(board, &LIGHT, &FixedMeasure);
        assert_eq!(scene, build(board, &LIGHT, &FixedMeasure));
        cache.rebuilt()
    };
    let opened = built(&mut cache, &board);
    assert_eq!(opened.routed, board.edges.len());
    let id = Id::new("n0123").unwrap();
    board.nodes.get_mut(&id).unwrap().text = "Typed".into();
    let typed = built(&mut cache, &board);
    assert_eq!((typed.routed, typed.separated, typed.painted), (0, 0, 1));
    board.nodes.get_mut(&id).unwrap().at[0] += 40;
    let moved = built(&mut cache, &board);
    assert!(moved.routed > 0 && moved.routed <= 16, "{moved:?}");
    let again = built(&mut cache, &board);
    assert_eq!((again.routed, again.painted), (0, 0));
    cache.clear();
    assert_eq!(built(&mut cache, &board).routed, board.edges.len());
}
