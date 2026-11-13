//! The editing property test (§7.3 item E7): seeded random sequences of
//! intents on random boards, some with integrity damage. Applying a plan
//! never panics, `check` reports no issue the board did not have, the
//! canonical bytes read back to themselves, and undoing every step gives
//! the first board back.

use std::collections::BTreeSet;
use std::time::Instant;

use super::*;
use crate::fixtures::Rng;
use crate::history::{History, Snapshot, Unit};
use crate::ids::{from_bits, Seeded};
use crate::scene::Source;
use crate::{
    check, clip, parse, write, Color, Issue, IssueKind, Label, NodeStyle, Token, Vocabulary, GRID,
};

/// Names that ids, parents and connector ends are drawn from, so they meet:
/// a parent or an end may name a node, itself, or nothing, and a node and a
/// connector may share an id.
const NAMES: &[&str] = &["a", "b", "c", "d", "e", "p", "q", "start", "end", "gone"];

const TEXTS: &[&str] = &[
    "",
    "Start",
    "Two\nlines",
    "Trailing\n\n \n",
    "Carriage\r\nreturn\r",
    "é → 🚀",
    "\"quoted\" \\ back",
];

fn name(rng: &mut Rng) -> String {
    (*rng.pick(NAMES)).to_owned()
}

fn id(rng: &mut Rng) -> Id {
    if rng.one_in(4) {
        from_bits(rng.next())
    } else {
        Id::new(rng.pick(NAMES)).unwrap()
    }
}

/// A coordinate: mostly on the grid near the origin, sometimes off it, now
/// and then at the edge of the `i32` range.
fn coordinate(rng: &mut Rng) -> i32 {
    match rng.below(10) {
        0 => *rng.pick(&[i32::MAX, i32::MIN, i32::MAX - 7, 16_777_216]),
        1..=2 => rng.below(2_000) as i32 - 1_000,
        _ => (rng.below(80) as i32 - 40) * GRID,
    }
}

fn length(rng: &mut Rng) -> i32 {
    match rng.below(10) {
        0 => *rng.pick(&[0, -20, 1, 7, i32::MAX]),
        _ => (1 + rng.below(12) as i32) * GRID,
    }
}

/// A stacking key: mostly valid, sometimes missing, empty, outside the
/// alphabet, or too long.
fn key(rng: &mut Rng) -> Option<String> {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    match rng.below(16) {
        0 => None,
        1 => Some(String::new()),
        2 => Some("a.b".into()),
        3 => Some("z".repeat(70)),
        _ => Some(
            (0..1 + rng.below(4))
                .map(|_| char::from(*rng.pick(DIGITS)))
                .collect(),
        ),
    }
}

fn extra(rng: &mut Rng) -> Extra {
    let mut extra = Extra::new();
    if rng.one_in(3) {
        let mut tool = Extra::new();
        tool.insert("note".into(), toml::Value::String("kept".into()));
        let mut ext = Extra::new();
        ext.insert("tool".into(), toml::Value::Table(tool));
        extra.insert("ext".into(), toml::Value::Table(ext));
    }
    if rng.one_in(6) {
        extra.insert("locked".into(), toml::Value::Boolean(true));
    }
    extra
}

fn end(rng: &mut Rng) -> End {
    if rng.one_in(5) {
        End::Free {
            at: [coordinate(rng), coordinate(rng)],
            extra: Extra::new(),
        }
    } else {
        End::Node {
            node: name(rng),
            side: rng.one_in(4).then(|| Named::Known(*rng.pick(Side::ALL))),
            anchor: None,
            extra: Extra::new(),
        }
    }
}

fn random_node(rng: &mut Rng, parent: Option<String>) -> Node {
    Node {
        text: (*rng.pick(TEXTS)).to_owned(),
        kind: match rng.below(10) {
            0 => None,
            1 => Some(Named::Other("future".into())),
            2..=3 => Some(Named::Known(NodeKind::Text)),
            _ => Some(Named::Known(NodeKind::Shape)),
        },
        at: [coordinate(rng), coordinate(rng)],
        parent,
        shape: if rng.one_in(6) {
            Named::Other("hexagon".into())
        } else {
            Named::Known(*rng.pick(Shape::ALL))
        },
        size: [length(rng), length(rng)],
        style: NodeStyle {
            color: Color::Token(*rng.pick(Token::ALL)),
            ..NodeStyle::default()
        },
        z: key(rng),
        extra: extra(rng),
    }
}

fn random_board(rng: &mut Rng) -> Board {
    let mut board = Board::default();
    if rng.one_in(25) {
        board.requires.push("frames".into());
    }
    for _ in 0..rng.below(10) {
        let parent = rng.one_in(4).then(|| name(rng));
        let node = random_node(rng, parent);
        board.nodes.insert(id(rng), node);
    }
    if rng.one_in(20) {
        // A chain deeper than a parent is drawn inside.
        for k in 0..34 {
            let parent = (k > 0).then(|| format!("c{:02}", k - 1));
            let node = random_node(rng, parent);
            board
                .nodes
                .insert(Id::new(&format!("c{k:02}")).unwrap(), node);
        }
    }
    for _ in 0..rng.below(8) {
        let edge = Edge {
            kind: (!rng.one_in(10)).then_some(Named::Known(EdgeKind::Edge)),
            from: end(rng),
            to: end(rng),
            heads: Edge::DEFAULT_HEADS,
            labels: if rng.one_in(3) {
                vec![Label {
                    text: "Yes".into(),
                    at: Label::DEFAULT_AT,
                    extra: Extra::new(),
                }]
            } else {
                Vec::new()
            },
            route: Named::Known(*rng.pick(Route::ALL)),
            style: EdgeStyle::default(),
            z: key(rng),
            extra: extra(rng),
        };
        board.edges.insert(id(rng), edge);
    }
    board
}

/// Some of the board's ids, now and then with one that names nothing.
fn chosen(board: &Board, rng: &mut Rng) -> Vec<Id> {
    let all: Vec<&Id> = board.nodes.keys().chain(board.edges.keys()).collect();
    let mut out = Vec::new();
    for _ in 0..rng.below(4) {
        if all.is_empty() || rng.one_in(8) {
            out.push(Id::new(rng.pick(NAMES)).unwrap());
        } else {
            out.push((*rng.pick(&all)).clone());
        }
    }
    out
}

/// A node of the board, or now and then a name that may name nothing.
fn node_id(board: &Board, rng: &mut Rng) -> Id {
    let nodes: Vec<&Id> = board.nodes.keys().collect();
    if nodes.is_empty() || rng.one_in(8) {
        Id::new(rng.pick(NAMES)).unwrap()
    } else {
        (*rng.pick(&nodes)).clone()
    }
}

fn edge_id(board: &Board, rng: &mut Rng) -> Id {
    let edges: Vec<&Id> = board.edges.keys().collect();
    if edges.is_empty() || rng.one_in(8) {
        Id::new(rng.pick(NAMES)).unwrap()
    } else {
        (*rng.pick(&edges)).clone()
    }
}

fn endpoint(board: &Board, rng: &mut Rng) -> Endpoint {
    if rng.one_in(6) {
        Endpoint::Free {
            at: [coordinate(rng), coordinate(rng)],
        }
    } else {
        Endpoint::Node {
            node: node_id(board, rng),
            side: rng.one_in(3).then(|| *rng.pick(Side::ALL)),
        }
    }
}

fn step(rng: &mut Rng) -> [i32; 2] {
    if rng.one_in(10) {
        [i32::MAX, i32::MIN]
    } else {
        [
            (rng.below(7) as i32 - 3) * GRID,
            (rng.below(7) as i32 - 3) * GRID,
        ]
    }
}

fn random_intent(board: &Board, rng: &mut Rng) -> Intent {
    let dirs = [Dir::Left, Dir::Right, Dir::Up, Dir::Down];
    let ends = [EdgeEnd::From, EdgeEnd::To];
    match rng.below(14) {
        0 => Intent::AddNode {
            kind: if rng.one_in(3) {
                NodeKind::Text
            } else {
                NodeKind::Shape
            },
            shape: *rng.pick(Shape::ALL),
            at: [coordinate(rng), coordinate(rng)],
            size: [length(rng), length(rng)],
            from: rng.one_in(3).then(|| endpoint(board, rng)),
        },
        1 => Intent::SetText {
            id: node_id(board, rng),
            text: (*rng.pick(TEXTS)).to_owned(),
            size: [length(rng), length(rng)],
        },
        2 => Intent::Move {
            ids: chosen(board, rng),
            by: step(rng),
        },
        3 => {
            let [left, top] = step(rng);
            let [right, bottom] = step(rng);
            Intent::Resize {
                ids: chosen(board, rng),
                edges: Edges {
                    left,
                    top,
                    right,
                    bottom,
                },
            }
        }
        4 => Intent::SetShape {
            ids: chosen(board, rng),
            shape: *rng.pick(Shape::ALL),
        },
        5 => Intent::Delete {
            ids: chosen(board, rng),
        },
        6 => Intent::Duplicate {
            ids: chosen(board, rng),
            by: [GRID, GRID],
        },
        7 => {
            let copied = clip::copy(board, &chosen(board, rng));
            Intent::Paste {
                text: copied.unwrap_or_else(|| (*rng.pick(TEXTS)).to_owned()),
                size: [length(rng), length(rng)],
                view: Area {
                    at: [coordinate(rng), coordinate(rng)],
                    size: [rng.below(2_000) as i32, rng.below(2_000) as i32],
                },
                nth: rng.below(4) as u32,
            }
        }
        8 => Intent::AddEdge {
            from: endpoint(board, rng),
            to: endpoint(board, rng),
        },
        9 => Intent::Rebind {
            id: edge_id(board, rng),
            end: *rng.pick(&ends),
            to: endpoint(board, rng),
        },
        10 => Intent::SetEdge {
            ids: chosen(board, rng),
            field: match rng.below(3) {
                0 => EdgeField::Route(*rng.pick(Route::ALL)),
                1 => EdgeField::Head(*rng.pick(&ends), *rng.pick(Head::ALL)),
                _ => EdgeField::Stroke(*rng.pick(Dash::ALL)),
            },
        },
        11 => Intent::Reverse {
            ids: chosen(board, rng),
        },
        12 => Intent::SetEdgeLabel {
            id: edge_id(board, rng),
            text: (*rng.pick(TEXTS)).to_owned(),
        },
        _ => Intent::Extend {
            from: node_id(board, rng),
            dir: rng.one_in(2).then(|| *rng.pick(&dirs)),
            shape: rng.one_in(3).then(|| *rng.pick(Shape::ALL)),
        },
    }
}

/// Whether `issue`, found after a step, is one the board had before it:
/// the same issue on an element that was there (a dangling connector may
/// dangle at no more ends, and reversing it moves which end that is), or,
/// on a new element, a missing `kind` copied
/// from an element that had none.
fn had(issue: &Issue, before: &[Issue], old: &Board) -> bool {
    let existed = match &issue.element {
        Source::Node(id) => old.nodes.contains_key(id),
        Source::Edge(id) => old.edges.contains_key(id),
    };
    if !existed {
        return issue.kind == IssueKind::MissingKind
            && before.iter().any(|old| old.kind == IssueKind::MissingKind);
    }
    before.iter().any(|old| {
        old.element == issue.element
            && match (old.kind, issue.kind) {
                (
                    IssueKind::DanglingEdge { from, to },
                    IssueKind::DanglingEdge {
                        from: now_from,
                        to: now_to,
                    },
                ) => u8::from(now_from) + u8::from(now_to) <= u8::from(from) + u8::from(to),
                (old, now) => old == now,
            }
    })
}

fn snapshot(board: &Board, selection: &BTreeSet<Id>) -> Snapshot {
    Snapshot {
        board: board.clone(),
        selection: selection.clone(),
        cursor: selection.iter().next().cloned(),
    }
}

fn sequences_hold(count: usize, seed: u64) {
    let mut rng = Rng(seed);
    let mut ids = Seeded::new(seed);
    let now = Instant::now();
    let mut changed = 0;
    for _ in 0..count {
        let first = random_board(&mut rng);
        let mut board = first.clone();
        let mut selection = BTreeSet::new();
        let mut history = History::new();
        let mut steps = Vec::new();
        for _ in 0..1 + rng.below(24) {
            let intent = random_intent(&board, &mut rng);
            let before = check(&board);
            let old = board.clone();
            let plan = plan(&board, &intent, &mut ids);
            if plan.is_empty() {
                continue;
            }
            board.apply(&plan.changes);
            let unit = match intent {
                Intent::Move { .. } => Unit::Move,
                Intent::Resize { .. } => Unit::Resize,
                _ => Unit::One,
            };
            history.record(snapshot(&old, &selection), unit, now);
            if let Some(select) = &plan.select {
                selection = select.iter().cloned().collect();
            }
            selection.retain(|id| board.nodes.contains_key(id) || board.edges.contains_key(id));
            let written = write(&board);
            let again = parse(&written).unwrap_or_else(|error| panic!("{error}\n{intent:?}"));
            assert_eq!(write(&again), written, "{intent:?}");
            for issue in check(&board) {
                assert!(
                    had(&issue, &before, &old),
                    "{issue:?} after {intent:?}\n{}",
                    write(&old)
                );
            }
            for id in plan.select.iter().flatten().chain(&plan.edit) {
                assert!(
                    board.nodes.contains_key(id) || board.edges.contains_key(id),
                    "{id} selected after {intent:?}"
                );
            }
            steps.push(intent);
            changed += 1;
        }
        let last = write(&board);
        let mut current = snapshot(&board, &selection);
        while history.can_undo() {
            current = history.undo(current).expect("an undo step");
        }
        assert_eq!(write(&current.board), write(&first), "undo after {steps:?}");
        assert!(current.selection.is_empty());
        while history.can_redo() {
            current = history.redo(current).expect("a redo step");
        }
        assert_eq!(write(&current.board), last, "redo after {steps:?}");
    }
    // Most steps change something, or the property means little.
    assert!(
        changed > count * 3,
        "{changed} changes in {count} sequences"
    );
}

#[test]
fn random_intent_sequences_hold() {
    sequences_hold(500, 5);
}

#[test]
#[ignore = "full count; run by make fuzz-board"]
fn fuzz_random_intent_sequences_hold() {
    sequences_hold(20_000, 6);
}
