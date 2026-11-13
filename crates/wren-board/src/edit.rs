//! Editing a board as data. An [`Intent`] is what someone asked for, with
//! every element it touches named by id and every value given whole; it
//! never reads a selection. [`plan`] turns it into the [`Change`]s that do
//! it, without touching the board, and [`Board::apply`] makes them. So a
//! gesture's preview is its plan applied to a scratch copy, and a command
//! is its plan applied once.
//!
//! Every change replaces or removes a whole record, copied from the board
//! with only the edited fields changed, so unknown keys, unknown values and
//! `ext` content of every element survive any edit. The rules are the
//! plan's §5.5 to §5.12; an intent whose plan changes nothing gives an empty
//! [`Plan`], which sends no draft and makes no undo step.

use std::collections::BTreeSet;

use crate::ids::Ids;
use crate::{
    placements, Board, Dash, Dir, Edge, End, Head, Id, Node, NodeKind, Route, Shape, Side,
};

/// What someone asked a board to do. Coordinates are absolute board
/// units, already snapped where a rule snaps them ([`crate::snap`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// A new node with an empty label, the neutral colour and the default
    /// fill, on top of the stacking order, under a generated id (§5.5
    /// rules 3, 4 and 6). With `from`, a connector from there to the new
    /// node too, in the same step: a connector released on empty canvas
    /// (§5.12 rule 1).
    AddNode {
        kind: NodeKind,
        shape: Shape,
        at: [i32; 2],
        size: [i32; 2],
        from: Option<Endpoint>,
    },
    /// A node's label, and its size after the label editor grew it, or as
    /// it was (§5.9 rule 7). The canvas measures; the plan stays pure.
    SetText {
        id: Id,
        text: String,
        size: [i32; 2],
    },
    /// Move the nodes and the free ends of the connectors among `ids` by
    /// `by` (§5.8 rules 2 and 4). A node whose drawn ancestor is also
    /// among `ids` moves with it and not again.
    Move { ids: Vec<Id>, by: [i32; 2] },
    /// Move edges of each node's box, never below one grid cell either
    /// way (§5.8 rules 5 and 6).
    Resize { ids: Vec<Id>, edges: Edges },
    /// Change the shape of each node, keeping its box and label (§5.5
    /// rule 9).
    SetShape { ids: Vec<Id>, shape: Shape },
    /// Remove the elements, the descendants of removed nodes and every
    /// connector attached to a removed node (§5.10 rule 1).
    Delete { ids: Vec<Id> },
    /// Copy the fragment of `ids` (§5.10 rule 2) under new ids, offset by
    /// `by`, on top in the same relative order (§5.10 rule 3; a
    /// duplicate-drag, §5.8 rule 3).
    Duplicate { ids: Vec<Id>, by: [i32; 2] },
    /// Paste clipboard text (§5.10 rules 6 to 9): a board fragment after
    /// its header line, or foreign text that becomes one text node of
    /// `size`, which the canvas measured. `view` is the board area in view
    /// and `nth` counts the pastes of this text into this board, from 1.
    Paste {
        text: String,
        size: [i32; 2],
        view: Area,
        nth: u32,
    },
    /// A connector, with no label, on top of the stacking order (§5.12
    /// rules 1 and 5).
    AddEdge { from: Endpoint, to: Endpoint },
    /// Move one end of a connector (§5.12 rule 6).
    Rebind { id: Id, end: EdgeEnd, to: Endpoint },
    /// Set one style field of each connector (§5.12 rule 9).
    SetEdge { ids: Vec<Id>, field: EdgeField },
    /// Swap each connector's ends, with their heads (§5.12 rule 9).
    Reverse { ids: Vec<Id> },
    /// A connector's first label; an empty text removes it (§5.12 rule 8,
    /// §5.9 rule 8).
    SetEdgeLabel { id: Id, text: String },
    /// Keyboard building from the node `from` (§5.6). `dir` is `None` for
    /// `board.connect.next`, which takes the flow direction and always
    /// creates; `Some` for `board.connect.<dir>`, which connects to a node
    /// in the ideal slot when there is one. `shape` is the armed shape
    /// tool, if any, which always creates.
    Extend {
        from: Id,
        dir: Option<Dir>,
        shape: Option<Shape>,
    },
}

/// How far each edge of a node's box moves, in board units; positive is
/// right or down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// A box in absolute board units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    pub at: [i32; 2],
    pub size: [i32; 2],
}

/// One end of a connector an intent makes or moves. Ring 0 never writes
/// `anchor`, and the UI makes no free end (§5.12 rules 5 and 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// Attached to a node; floating when `side` is `None`.
    Node { node: Id, side: Option<Side> },
    /// A free end.
    Free { at: [i32; 2] },
}

/// Which end of a connector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeEnd {
    From,
    To,
}

/// A connector style field and its new value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeField {
    Route(Route),
    Head(EdgeEnd, Head),
    Stroke(Dash),
}

/// One step of a plan: a whole record put under an id, adding it or
/// replacing the one there, or a record removed.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    PutNode(Id, Box<Node>),
    PutEdge(Id, Box<Edge>),
    RemoveNode(Id),
    RemoveEdge(Id),
}

/// What an intent does to a board.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// The changes, in order. Empty when the intent changes nothing.
    pub changes: Vec<Change>,
    /// What to select once applied, when the intent decides it: what it
    /// created or reached. `None` keeps the selection, less what it removed.
    pub select: Option<Vec<Id>>,
    /// A node it created whose label opens for editing (§5.5 rule 6).
    pub edit: Option<Id>,
    /// Why it changed nothing, when the user should be told.
    pub notice: Option<String>,
}

impl Plan {
    /// Whether applying it changes nothing.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// The plan for `intent` on `board`, drawing any new id from `ids`. It
/// reads only the board and the intent, and changes nothing.
pub fn plan(board: &Board, intent: &Intent, ids: &mut dyn Ids) -> Plan {
    let _ = ids;
    match intent {
        Intent::SetText { id, text, size } => set_text(board, id, text, *size),
        Intent::Move { ids: chosen, by } => moved(board, chosen, *by),
        _ => Plan::default(),
    }
}

/// A label as it is stored: line breaks as LF, and no trailing blank lines
/// (§5.9 rule 4). A blank line holds only white space, so a label of only
/// white space is empty.
pub fn trim_label(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

impl Board {
    /// Makes the changes of a [`Plan`], in order.
    pub fn apply(&mut self, changes: &[Change]) {
        for change in changes {
            match change {
                Change::PutNode(id, node) => {
                    self.nodes.insert(id.clone(), Node::clone(node));
                }
                Change::PutEdge(id, edge) => {
                    self.edges.insert(id.clone(), Edge::clone(edge));
                }
                Change::RemoveNode(id) => {
                    self.nodes.remove(id);
                }
                Change::RemoveEdge(id) => {
                    self.edges.remove(id);
                }
            }
        }
    }
}

/// Sets a node's label, trimmed as it is stored, and its size (§5.9 rules
/// 4 and 7). A label and size the node already has plan nothing.
fn set_text(board: &Board, id: &Id, text: &str, size: [i32; 2]) -> Plan {
    let Some(node) = board.nodes.get(id) else {
        return Plan::default();
    };
    let text = trim_label(text);
    let size = size.map(|length| length.max(1));
    if node.text == text && node.size == size {
        return Plan::default();
    }
    let node = Node {
        text,
        size,
        ..node.clone()
    };
    Plan {
        changes: vec![Change::PutNode(id.clone(), Box::new(node))],
        ..Plan::default()
    }
}

fn moved(board: &Board, ids: &[Id], by: [i32; 2]) -> Plan {
    let chosen: BTreeSet<&Id> = ids.iter().collect();
    let placed = placements(board);
    // The drawn tree is acyclic and at most MAX_DEPTH deep; the bound only
    // keeps a mistake there from looping.
    let carried = |id: &Id| {
        let mut parent = placed.get(id).and_then(|placed| placed.parent);
        for _ in 0..board.nodes.len() {
            match parent {
                Some(above) if chosen.contains(above) => return true,
                Some(above) => parent = placed.get(above).and_then(|placed| placed.parent),
                None => return false,
            }
        }
        false
    };
    let mut changes = Vec::new();
    for &id in &chosen {
        if let Some(node) = board.nodes.get(id) {
            let at = shifted(node.at, by);
            if at != node.at && !carried(id) {
                changes.push(Change::PutNode(
                    id.clone(),
                    Box::new(Node { at, ..node.clone() }),
                ));
            }
        } else if let Some(edge) = board.edges.get(id) {
            let mut edge = edge.clone();
            let mut changed = false;
            for end in [&mut edge.from, &mut edge.to] {
                if let End::Free { at, .. } = end {
                    let to = shifted(*at, by);
                    changed |= to != *at;
                    *at = to;
                }
            }
            if changed {
                changes.push(Change::PutEdge(id.clone(), Box::new(edge)));
            }
        }
    }
    Plan {
        changes,
        ..Plan::default()
    }
}

fn shifted([x, y]: [i32; 2], [dx, dy]: [i32; 2]) -> [i32; 2] {
    [x.saturating_add(dx), y.saturating_add(dy)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, write};

    /// Draws ids from a fixed list, as a test's explicit ids.
    struct Listed(Vec<&'static str>);

    impl Ids for Listed {
        fn draw(&mut self) -> Id {
            Id::new(self.0.remove(0)).unwrap()
        }
    }

    const BOARD: &str = r#"schema = 1

[node.a]
text = "Outer"
kind = "shape"
at = [0, 0]
ext.tool.note = "kept"
size = [200, 120]
z = "V"

[node.b]
text = "Inner"
kind = "shape"
at = [20, 20]
parent = "a"
size = [120, 60]
z = "V"

[node.c]
kind = "future"
at = [400, 0]
mystery = 1
size = [120, 60]
z = "W"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { at = [600, 300] }
z = "X"

[edge.f]
kind = "edge"
from = { node = "a" }
to = { node = "c" }
z = "Y"
"#;

    fn ids(names: &[&str]) -> Vec<Id> {
        names.iter().map(|name| Id::new(name).unwrap()).collect()
    }

    /// The lines that differ after the intent is planned and applied,
    /// before then after.
    fn diff(intent: &Intent) -> Vec<(String, String)> {
        let mut board = parse(BOARD).unwrap();
        let before = write(&board);
        assert_eq!(before, BOARD, "the fixture is canonical");
        let plan = plan(&board, intent, &mut Listed(Vec::new()));
        board.apply(&plan.changes);
        let after = write(&board);
        assert_eq!(before.lines().count(), after.lines().count());
        before
            .lines()
            .zip(after.lines())
            .filter(|(old, new)| old != new)
            .map(|(old, new)| (old.to_owned(), new.to_owned()))
            .collect()
    }

    fn pairs(lines: &[(&str, &str)]) -> Vec<(String, String)> {
        lines
            .iter()
            .map(|(old, new)| ((*old).to_owned(), (*new).to_owned()))
            .collect()
    }

    #[test]
    fn a_move_changes_one_at_line_per_moved_node_and_no_edge_line() {
        let by = [40, -20];
        // Which nodes move, and the one line that changes.
        let table: [(&[&str], [&str; 2]); 4] = [
            (&["a"], ["at = [0, 0]", "at = [40, -20]"]),
            // The unknown kind and its unknown key move like any node.
            (&["c"], ["at = [400, 0]", "at = [440, -20]"]),
            // A child moves with its parent, not again.
            (&["a", "b"], ["at = [0, 0]", "at = [40, -20]"]),
            (&["b"], ["at = [20, 20]", "at = [60, 0]"]),
        ];
        for (moved, [old, new]) in table {
            let intent = Intent::Move {
                ids: ids(moved),
                by,
            };
            assert_eq!(diff(&intent), pairs(&[(old, new)]), "{moved:?}");
        }
    }

    #[test]
    fn a_label_change_is_one_line_and_its_growth_another() {
        let set = |id: &str, text: &str, size| Intent::SetText {
            id: Id::new(id).unwrap(),
            text: text.to_owned(),
            size,
        };
        let text = ("text = \"Inner\"", "text = \"Inner\\nmore\"");
        // Trailing blank lines are not stored, and a CR is a line break.
        for typed in ["Inner\nmore", "Inner\r\nmore\n \n\t\n"] {
            assert_eq!(diff(&set("b", typed, [120, 60])), pairs(&[text]));
        }
        assert_eq!(
            diff(&set("b", "Inner\nmore", [120, 80])),
            pairs(&[text, ("size = [120, 60]", "size = [120, 80]")])
        );
        let board = parse(BOARD).unwrap();
        for same in [
            set("b", "Inner", [120, 60]),
            set("b", "Inner\n\n", [120, 60]),
            set("e", "A connector", [1, 1]),
            set("gone", "x", [1, 1]),
        ] {
            let plan = plan(&board, &same, &mut Listed(Vec::new()));
            assert!(plan.is_empty(), "{same:?}");
        }
        assert_eq!(trim_label("a \n\n b\n \n"), "a \n\n b");
        assert_eq!(trim_label(" \n\t"), "");
    }

    #[test]
    fn a_moved_connector_moves_its_free_end_only() {
        let intent = Intent::Move {
            ids: ids(&["e", "f"]),
            by: [20, 20],
        };
        assert_eq!(
            diff(&intent),
            pairs(&[("to = { at = [600, 300] }", "to = { at = [620, 320] }")])
        );
    }

    #[test]
    fn a_move_that_changes_nothing_is_an_empty_plan() {
        let board = parse(BOARD).unwrap();
        for intent in [
            Intent::Move {
                ids: ids(&["a"]),
                by: [0, 0],
            },
            Intent::Move {
                ids: ids(&["f", "gone"]),
                by: [20, 0],
            },
            Intent::Move {
                ids: Vec::new(),
                by: [20, 0],
            },
        ] {
            let plan = plan(&board, &intent, &mut Listed(Vec::new()));
            assert!(plan.is_empty(), "{intent:?}: {plan:?}");
        }
    }

    #[test]
    fn a_move_stops_at_the_integer_range() {
        let mut board = parse(BOARD).unwrap();
        let intent = Intent::Move {
            ids: ids(&["c"]),
            by: [i32::MAX, i32::MIN],
        };
        let plan = plan(&board, &intent, &mut Listed(Vec::new()));
        board.apply(&plan.changes);
        assert_eq!(board.nodes["c"].at, [i32::MAX, i32::MIN]);
        assert_eq!(write(&parse(&write(&board)).unwrap()), write(&board));
    }

    #[test]
    fn apply_puts_and_removes_whole_records() {
        let mut board = parse(BOARD).unwrap();
        let node = board.nodes["c"].clone();
        let edge = board.edges["f"].clone();
        let [n, e] = [Id::new("n").unwrap(), Id::new("g").unwrap()];
        board.apply(&[
            Change::RemoveNode(Id::new("c").unwrap()),
            Change::RemoveEdge(Id::new("f").unwrap()),
            Change::PutNode(n.clone(), Box::new(node.clone())),
            Change::PutEdge(e.clone(), Box::new(edge.clone())),
        ]);
        assert!(!board.nodes.contains_key("c") && !board.edges.contains_key("f"));
        assert_eq!((&board.nodes[&n], &board.edges[&e]), (&node, &edge));
    }
}
