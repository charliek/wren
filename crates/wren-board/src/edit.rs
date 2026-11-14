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
pub use crate::scene::Source;
use crate::z::{self, Element};
use crate::{
    access, Access, Board, Dash, Dir, Edge, EdgeKind, EdgeStyle, End, Extra, Head, Id, Named, Node,
    NodeKind, Reason, Route, Shape, Side,
};

mod connect;
mod copy;
mod delete;
mod node;
#[cfg(test)]
mod property;

pub(crate) use copy::{copies, fragment, Fragment, Layout};

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
    /// among `ids` moves with it and not again. Each element is named with
    /// its kind, since a node and a connector may share an id (§4.12).
    Move { ids: Vec<Source>, by: [i32; 2] },
    /// Move edges of each node's box, never below one grid cell either
    /// way (§5.8 rules 5 and 6).
    Resize { ids: Vec<Id>, edges: Edges },
    /// Change the shape of each node, keeping its box and label (§5.5
    /// rule 9).
    SetShape { ids: Vec<Id>, shape: Shape },
    /// Remove the elements, the descendants of removed nodes and every
    /// connector attached to a removed node (§5.10 rule 1). Each element
    /// is named with its kind, as for [`Intent::Move`].
    Delete { ids: Vec<Source> },
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
    /// A connector, with no label, on top of the stacking order, selected
    /// (§5.12 rules 1 and 5). Nothing is made when an end names a node that
    /// is not there or both ends name one node.
    AddEdge { from: Endpoint, to: Endpoint },
    /// Move one end of a connector (§5.12 rule 6): onto a node, or, for an
    /// end that is a free point, to another point.
    Rebind { id: Id, end: EdgeEnd, to: Endpoint },
    /// Set one style field of each connector (§5.12 rule 9).
    SetEdge { ids: Vec<Id>, field: EdgeField },
    /// Swap each connector's ends, mirroring its labels' places so they
    /// stay where they are drawn (§5.12 rule 9). The heads stay by
    /// position, so the arrow turns round with the direction.
    Reverse { ids: Vec<Id> },
    /// A connector's first label (§5.12 rule 8), sent on each change of
    /// its text with `commit` false (§5.9 rule 6) and, when it ends empty,
    /// once more at the commit with `commit` true. While it is edited an
    /// empty text keeps the item, with its place and unknown keys, so
    /// retyping fills it again; committed empty it is removed (§5.9 rule 8).
    SetEdgeLabel { id: Id, text: String, commit: bool },
    /// Keyboard building from the node `from`, or from the `to` node of the
    /// connector `from` (§5.6). `dir` is `None` for `board.connect.next`,
    /// which takes the flow direction and always creates; `Some` for
    /// `board.connect.<dir>`, which connects to a node in the ideal slot
    /// when there is one. `shape` is the armed shape tool, if any, which
    /// always creates.
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
/// reads only the board and the intent, and changes nothing. A read-only
/// board (§4.10) gets an empty plan with a notice, whatever the intent.
///
/// No intent consults `locked` (§4.5): ring 0 gives it no effect.
pub fn plan(board: &Board, intent: &Intent, ids: &mut dyn Ids) -> Plan {
    if let Access::ReadOnly(reason) = access(board) {
        return Plan::notice(format!("This board is read-only: it {}.", because(&reason)));
    }
    match intent {
        Intent::AddNode {
            kind,
            shape,
            at,
            size,
            from,
        } => node::add(board, *kind, *shape, *at, *size, from.as_ref(), ids),
        Intent::SetText { id, text, size } => node::set_text(board, id, text, *size),
        Intent::Move { ids: chosen, by } => node::moved(board, chosen, *by),
        Intent::Resize { ids: chosen, edges } => node::resize(board, chosen, *edges),
        Intent::SetShape { ids: chosen, shape } => node::set_shape(board, chosen, *shape),
        Intent::Delete { ids: chosen } => delete::delete(board, chosen),
        Intent::Duplicate { ids: chosen, by } => copy::duplicate(board, chosen, *by, ids),
        Intent::Paste {
            text,
            size,
            view,
            nth,
        } => crate::clip::paste(board, text, *size, *view, *nth, ids),
        Intent::AddEdge { from, to } => connect::add(board, from, to, ids),
        Intent::Rebind { id, end, to } => connect::rebind(board, id, *end, to),
        Intent::SetEdge { ids: chosen, field } => connect::set_field(board, chosen, *field),
        Intent::Reverse { ids: chosen } => connect::reverse(board, chosen),
        Intent::SetEdgeLabel { id, text, commit } => connect::set_label(board, id, text, *commit),
        Intent::Extend { from, dir, shape } => connect::extend(board, from, *dir, *shape, ids),
    }
}

impl Plan {
    /// A plan that changes nothing and says why.
    pub(crate) fn notice(text: String) -> Self {
        Self {
            notice: Some(text),
            ..Self::default()
        }
    }
}

/// Why a board, or a pasted fragment, may not be edited, to follow "it" or
/// "the fragment".
pub(crate) fn because(reason: &Reason) -> String {
    match reason {
        Reason::Schema(_) => format!("is {reason}"),
        Reason::Requires(_) => reason.to_string(),
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

/// Keys for `count` new elements on top of the root scope, in order, and
/// the changes that renumber the scope first when no key fits above its top
/// (§4.7's fallback, in the same step).
pub(crate) fn on_top(board: &Board, count: usize) -> (Vec<String>, Vec<Change>) {
    let scope = z::scope(board, None);
    let keys: Vec<Option<&str>> = scope.iter().map(|(_, key)| *key).collect();
    let insert = z::insert(&keys, keys.len(), count);
    let mut changes = Vec::new();
    for ((element, old), new) in scope.iter().zip(insert.renumber.unwrap_or_default()) {
        if *old == Some(new.as_str()) {
            continue;
        }
        changes.push(match *element {
            Element::Node(id) => Change::PutNode(
                id.clone(),
                Box::new(Node {
                    z: Some(new),
                    ..board.nodes[id].clone()
                }),
            ),
            Element::Edge(id) => Change::PutEdge(
                id.clone(),
                Box::new(Edge {
                    z: Some(new),
                    ..board.edges[id].clone()
                }),
            ),
        });
    }
    (insert.keys, changes)
}

/// The names a new id must not take besides the board's own ids: the
/// `parent` or connector end that names no node. A new element under such
/// a name would silently adopt a dangling reference, re-parenting a node or
/// attaching a connector nobody touched.
pub(crate) fn dangling_names(board: &Board) -> BTreeSet<Id> {
    let ends = board.edges.values().flat_map(|edge| [&edge.from, &edge.to]);
    let names = board
        .nodes
        .values()
        .filter_map(|node| node.parent.as_deref())
        .chain(ends.filter_map(|end| match end {
            End::Node { node, .. } => Some(node.as_str()),
            End::Free { .. } => None,
        }));
    names
        .filter(|name| !board.nodes.contains_key(*name))
        .filter_map(Id::new)
        .collect()
}

/// A new connector between two ends: an elbow route, an arrowhead at the
/// `to` end only, no label, the neutral colour (§5.6 rule 9, §5.12).
pub(crate) fn connector(from: &Endpoint, to: &Endpoint, z: Option<String>) -> Edge {
    let end = |endpoint: &Endpoint| match endpoint {
        Endpoint::Node { node, side } => End::Node {
            node: node.as_str().to_owned(),
            side: side.map(Named::Known),
            anchor: None,
            extra: Extra::new(),
        },
        Endpoint::Free { at } => End::Free {
            at: *at,
            extra: Extra::new(),
        },
    };
    Edge {
        kind: Some(Named::Known(EdgeKind::Edge)),
        from: end(from),
        to: end(to),
        heads: Edge::DEFAULT_HEADS,
        labels: Vec::new(),
        route: Named::Known(Route::Elbow),
        style: EdgeStyle::default(),
        z,
        extra: Extra::new(),
    }
}

/// `value` within the `i32` range.
pub(crate) fn saturate(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{check, parse, write};

    /// Draws ids from a fixed list, as a test's explicit ids.
    pub(crate) struct Listed(pub Vec<&'static str>);

    impl Ids for Listed {
        fn draw(&mut self) -> Id {
            assert!(!self.0.is_empty(), "the test listed too few ids");
            Id::new(self.0.remove(0)).unwrap()
        }
    }

    /// Unknown content on touched and untouched elements: `ext` on `a`, an
    /// unknown kind with an unknown key and `locked` on `c`.
    pub(crate) const BOARD: &str = r#"schema = 1

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
locked = true
mystery = 1
size = [120, 60]
z = "W"

[node.t]
text = "Note"
kind = "text"
at = [400, 200]
size = [120, 20]
z = "S"

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

    pub(crate) fn ids(names: &[&str]) -> Vec<Id> {
        names.iter().map(|name| Id::new(name).unwrap()).collect()
    }

    /// The nodes named `names`, each with its kind.
    pub(crate) fn nodes(names: &[&str]) -> Vec<Source> {
        ids(names).into_iter().map(Source::Node).collect()
    }

    /// The connectors named `names`, each with its kind.
    pub(crate) fn edges(names: &[&str]) -> Vec<Source> {
        ids(names).into_iter().map(Source::Edge).collect()
    }

    /// The lines that differ between two files, in file order: a removed
    /// line as `-line` and an added one as `+line`, by a longest common
    /// subsequence. Empty lines are left out, so where a block lands among
    /// the empty lines around it does not matter.
    pub(crate) fn line_diff(before: &str, after: &str) -> Vec<String> {
        let a: Vec<&str> = before.lines().collect();
        let b: Vec<&str> = after.lines().collect();
        let mut common = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                common[i][j] = if a[i] == b[j] {
                    common[i + 1][j + 1] + 1
                } else {
                    common[i + 1][j].max(common[i][j + 1])
                };
            }
        }
        let (mut i, mut j, mut out) = (0, 0, Vec::new());
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                (i, j) = (i + 1, j + 1);
            } else if j == b.len() || (i < a.len() && common[i + 1][j] >= common[i][j + 1]) {
                out.push(format!("-{}", a[i]));
                i += 1;
            } else {
                out.push(format!("+{}", b[j]));
                j += 1;
            }
        }
        out.retain(|line| line.len() > 1);
        out
    }

    /// The plan for `intent` on `text`, with new ids drawn from `fresh`,
    /// and the board after it.
    fn planned(text: &str, intent: &Intent, fresh: &[&'static str]) -> (Plan, Board) {
        let mut board = parse(text).unwrap();
        let plan = plan(&board, intent, &mut Listed(fresh.to_vec()));
        board.apply(&plan.changes);
        let written = write(&board);
        assert_eq!(write(&parse(&written).unwrap()), written, "round trip");
        (plan, board)
    }

    /// The lines of `write` the intent changes on [`BOARD`].
    fn changed(intent: &Intent, fresh: &[&'static str]) -> Vec<String> {
        assert_eq!(
            write(&parse(BOARD).unwrap()),
            BOARD,
            "the fixture is canonical"
        );
        let (_, board) = planned(BOARD, intent, fresh);
        line_diff(BOARD, &write(&board))
    }

    #[track_caller]
    fn assert_lines(intent: Intent, fresh: &[&'static str], expected: &[&str]) {
        assert_eq!(changed(&intent, fresh), expected, "{intent:?}");
    }

    #[track_caller]
    fn assert_empty(text: &str, intent: Intent) {
        let board = parse(text).unwrap();
        let plan = plan(&board, &intent, &mut Listed(Vec::new()));
        assert!(plan.is_empty(), "{intent:?}: {plan:?}");
    }

    #[test]
    fn a_new_node_is_one_block_on_top() {
        let add = |kind, shape, at, size, from| Intent::AddNode {
            kind,
            shape,
            at,
            size,
            from,
        };
        let block = |extra: &[&'static str]| {
            let mut lines = vec!["+[node.n1]", "+kind = \"shape\"", "+at = [40, 80]"];
            lines.extend(extra);
            lines.extend(["+size = [120, 60]", "+z = \"Y01\""]);
            lines
        };
        let a = Endpoint::Node {
            node: Id::new("a").unwrap(),
            side: None,
        };
        let table: Vec<(Intent, Vec<&str>)> = vec![
            (
                add(NodeKind::Shape, Shape::Rect, [40, 80], [120, 60], None),
                block(&[]),
            ),
            (
                add(NodeKind::Shape, Shape::Diamond, [40, 80], [120, 60], None),
                block(&["+shape = \"diamond\""]),
            ),
            // A text node keeps the default shape.
            (
                add(NodeKind::Text, Shape::Diamond, [0, 0], [120, 20], None),
                vec![
                    "+[node.n1]",
                    "+kind = \"text\"",
                    "+at = [0, 0]",
                    "+size = [120, 20]",
                    "+z = \"Y01\"",
                ],
            ),
            // With a connector from `a`: one block each, the connector
            // above the node.
            (
                add(NodeKind::Shape, Shape::Rect, [40, 80], [120, 60], Some(a)),
                [
                    block(&[]),
                    vec![
                        "+[edge.n2]",
                        "+kind = \"edge\"",
                        "+from = { node = \"a\" }",
                        "+to = { node = \"n1\" }",
                        "+z = \"Y02\"",
                    ],
                ]
                .concat(),
            ),
        ];
        for (intent, expected) in table {
            assert_lines(intent, &["n1", "n2"], &expected);
        }
    }

    #[test]
    fn a_new_node_is_selected_and_opens_its_label() {
        let intent = Intent::AddNode {
            kind: NodeKind::Shape,
            shape: Shape::Rect,
            at: [0, 0],
            size: [0, -5],
            from: None,
        };
        let (plan, board) = planned(BOARD, &intent, &["n1"]);
        assert_eq!(plan.select, Some(ids(&["n1"])));
        assert_eq!(plan.edit, Some(Id::new("n1").unwrap()));
        // A size is at least 1 each way.
        assert_eq!(board.nodes["n1"].size, [1, 1]);
    }

    #[test]
    fn a_connector_from_a_node_that_is_gone_makes_nothing() {
        let gone = Endpoint::Node {
            node: Id::new("gone").unwrap(),
            side: None,
        };
        assert_empty(
            BOARD,
            Intent::AddNode {
                kind: NodeKind::Shape,
                shape: Shape::Rect,
                at: [0, 0],
                size: [120, 60],
                from: Some(gone),
            },
        );
    }

    /// A key that is not in the alphabet on top of the scope: the whole
    /// scope is renumbered in the same step (§4.7).
    #[test]
    fn a_new_node_above_a_key_that_fits_nothing_renumbers_the_scope() {
        let text =
            "schema = 1\n\n[node.p]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n\n\
                    [node.q]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"a.b\"\n";
        let intent = Intent::AddNode {
            kind: NodeKind::Shape,
            shape: Shape::Rect,
            at: [0, 0],
            size: [20, 20],
            from: None,
        };
        let (_, board) = planned(text, &intent, &["n1"]);
        assert_eq!(
            line_diff(text, &write(&board)),
            [
                "+[node.n1]",
                "+kind = \"shape\"",
                "+at = [0, 0]",
                "+size = [20, 20]",
                "+z = \"V04\"",
                "-z = \"V\"",
                "+z = \"V01\"",
                "-z = \"a.b\"",
                "+z = \"V03\"",
            ]
        );
    }

    /// A new element never takes a name that a dangling `parent` or
    /// connector end already gives, which would adopt it.
    #[test]
    fn a_new_id_avoids_names_that_dangle() {
        let text = "schema = 1\n\n[node.p]\nkind = \"shape\"\nat = [0, 0]\nparent = \"lost\"\n\
                    size = [20, 20]\nz = \"V\"\n\n[edge.e]\nkind = \"edge\"\nfrom = { node = \"p\" }\n\
                    to = { node = \"gone\" }\nz = \"W\"\n";
        let intent = Intent::AddNode {
            kind: NodeKind::Shape,
            shape: Shape::Rect,
            at: [0, 0],
            size: [20, 20],
            from: None,
        };
        let (plan, board) = planned(text, &intent, &["gone", "lost", "p", "e", "n1"]);
        assert_eq!(plan.select, Some(ids(&["n1"])));
        assert_eq!(check(&board), check(&parse(text).unwrap()));
    }

    #[test]
    fn a_label_change_is_one_line() {
        let set = |id: &str, text: &str, size| Intent::SetText {
            id: Id::new(id).unwrap(),
            text: text.into(),
            size,
        };
        let table: Vec<(Intent, &[&str])> = vec![
            (
                set("a", "Two\nlines", [200, 120]),
                &["-text = \"Outer\"", "+text = \"Two\\nlines\""],
            ),
            // Trailing blank lines are trimmed, and CR becomes LF.
            (
                set("a", "Two\r\nlines\n  \n\t\n", [200, 120]),
                &["-text = \"Outer\"", "+text = \"Two\\nlines\""],
            ),
            // With the size the label editor grew it to.
            (
                set("b", "Inner\nlonger", [120, 80]),
                &[
                    "-text = \"Inner\"",
                    "+text = \"Inner\\nlonger\"",
                    "-size = [120, 60]",
                    "+size = [120, 80]",
                ],
            ),
            // An empty label is no line.
            (set("a", "", [200, 120]), &["-text = \"Outer\""]),
            (set("c", "Here", [120, 60]), &["+text = \"Here\""]),
        ];
        for (intent, expected) in table {
            assert_lines(intent, &[], expected);
        }
        for intent in [
            set("a", "Outer", [200, 120]),
            set("a", "Outer\n\n", [200, 120]),
            set("e", "Edge", [1, 1]),
            set("gone", "x", [1, 1]),
        ] {
            assert_empty(BOARD, intent);
        }
    }

    #[test]
    fn labels_lose_trailing_blank_lines_only() {
        let table = [
            ("", ""),
            ("a", "a"),
            ("a  ", "a  "),
            ("a\n", "a"),
            ("a\n \t\n\n", "a"),
            ("\na", "\na"),
            ("a\n\nb\n", "a\n\nb"),
            ("a\r\n\r\n", "a"),
            ("a\rb", "a\nb"),
            ("  \n ", ""),
        ];
        for (text, trimmed) in table {
            assert_eq!(trim_label(text), trimmed, "{text:?}");
        }
    }

    #[test]
    fn a_move_changes_one_at_line_per_moved_node_and_no_edge_line() {
        let by = [40, -20];
        // Which nodes move, and the one line that changes.
        let table: [(&[&str], [&str; 2]); 4] = [
            (&["a"], ["-at = [0, 0]", "+at = [40, -20]"]),
            // The unknown kind and its unknown key move like any node, and
            // `locked` is not consulted.
            (&["c"], ["-at = [400, 0]", "+at = [440, -20]"]),
            // A child moves with its parent, not again.
            (&["a", "b"], ["-at = [0, 0]", "+at = [40, -20]"]),
            (&["b"], ["-at = [20, 20]", "+at = [60, 0]"]),
        ];
        for (moved, expected) in table {
            let intent = Intent::Move {
                ids: nodes(moved),
                by,
            };
            assert_lines(intent, &[], &expected);
        }
    }

    #[test]
    fn a_moved_connector_moves_its_free_end_only() {
        let intent = Intent::Move {
            ids: edges(&["e", "f"]),
            by: [20, 20],
        };
        assert_lines(
            intent,
            &[],
            &["-to = { at = [600, 300] }", "+to = { at = [620, 320] }"],
        );
    }

    #[test]
    fn a_move_that_changes_nothing_is_an_empty_plan() {
        for intent in [
            Intent::Move {
                ids: nodes(&["a"]),
                by: [0, 0],
            },
            Intent::Move {
                ids: edges(&["f", "gone"]),
                by: [20, 0],
            },
            Intent::Move {
                ids: Vec::new(),
                by: [20, 0],
            },
        ] {
            assert_empty(BOARD, intent);
        }
    }

    #[test]
    fn a_move_stops_at_the_integer_range() {
        let intent = Intent::Move {
            ids: nodes(&["c"]),
            by: [i32::MAX, i32::MIN],
        };
        let (_, board) = planned(BOARD, &intent, &[]);
        assert_eq!(board.nodes["c"].at, [i32::MAX, i32::MIN]);
    }

    /// A node and a connector sharing an id (§4.12): a move names which.
    #[test]
    fn a_move_moves_only_the_kind_it_names() {
        let text = "schema = 1\n\n[node.v]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n\n[edge.v]\nkind = \"edge\"\nfrom = { at = [0, 100] }\nto = { at = [80, 100] }\nz = \"W\"\n";
        let by = [20, 0];
        let free = |board: &Board| match &board.edges["v"].from {
            End::Free { at, .. } => *at,
            End::Node { .. } => panic!("a free end"),
        };
        let (_, board) = planned(
            text,
            &Intent::Move {
                ids: nodes(&["v"]),
                by,
            },
            &[],
        );
        assert_eq!((board.nodes["v"].at, free(&board)), ([20, 0], [0, 100]));
        let (_, board) = planned(
            text,
            &Intent::Move {
                ids: edges(&["v"]),
                by,
            },
            &[],
        );
        assert_eq!((board.nodes["v"].at, free(&board)), ([0, 0], [20, 100]));
    }

    #[test]
    fn a_resize_moves_the_edges_it_names() {
        let resize = |names: &[&str], left, top, right, bottom| Intent::Resize {
            ids: ids(names),
            edges: Edges {
                left,
                top,
                right,
                bottom,
            },
        };
        let table: Vec<(Intent, &[&str])> = vec![
            // The right or bottom edge is the size line alone.
            (
                resize(&["a"], 0, 0, 20, 0),
                &["-size = [200, 120]", "+size = [220, 120]"],
            ),
            // The left or top edge moves the corner too.
            (
                resize(&["a"], -20, 0, 0, 0),
                &[
                    "-at = [0, 0]",
                    "+at = [-20, 0]",
                    "-size = [200, 120]",
                    "+size = [220, 120]",
                ],
            ),
            // Every node named, connectors ignored.
            (
                resize(&["a", "b", "e"], 0, 0, 0, 20),
                &[
                    "-size = [200, 120]",
                    "+size = [200, 140]",
                    "-size = [120, 60]",
                    "+size = [120, 80]",
                ],
            ),
            // Never below one cell: the moved edge stops one cell from the
            // one that stays.
            (
                resize(&["b"], 0, 0, -200, 0),
                &["-size = [120, 60]", "+size = [20, 60]"],
            ),
            (
                resize(&["a"], 300, 0, 0, 0),
                &[
                    "-at = [0, 0]",
                    "+at = [180, 0]",
                    "-size = [200, 120]",
                    "+size = [20, 120]",
                ],
            ),
        ];
        for (intent, expected) in table {
            assert_lines(intent, &[], expected);
        }
        assert_empty(BOARD, resize(&["a"], 0, 0, 0, 0));
        assert_empty(BOARD, resize(&["e", "gone"], 0, 0, 20, 20));
        // A box already narrower than a cell is not made smaller, nor grown
        // by this rule, but grows when asked.
        let small =
            "schema = 1\n\n[node.s]\nkind = \"shape\"\nat = [0, 0]\nsize = [10, 10]\nz = \"V\"\n";
        assert_empty(small, resize(&["s"], 0, 0, -20, -20));
        let (_, board) = planned(small, &resize(&["s"], 0, 0, 20, 0), &[]);
        assert_eq!(board.nodes["s"].size, [30, 10]);
    }

    #[test]
    fn a_reshape_changes_shape_nodes_only() {
        let reshape = |names: &[&str], shape| Intent::SetShape {
            ids: ids(names),
            shape,
        };
        assert_lines(
            reshape(&["a", "b", "c", "t", "e"], Shape::Diamond),
            &[],
            &["+shape = \"diamond\"", "+shape = \"diamond\""],
        );
        assert_empty(BOARD, reshape(&["a"], Shape::Rect));
        assert_empty(BOARD, reshape(&["c", "t", "gone"], Shape::Pill));
    }

    #[test]
    fn delete_removes_whole_blocks() {
        let delete = |ids: Vec<Source>| Intent::Delete { ids };
        assert_lines(
            delete(nodes(&["b"])),
            &[],
            &[
                "-[node.b]",
                "-text = \"Inner\"",
                "-kind = \"shape\"",
                "-at = [20, 20]",
                "-parent = \"a\"",
                "-size = [120, 60]",
                "-z = \"V\"",
            ],
        );
        assert_lines(
            delete(edges(&["e"])),
            &[],
            &[
                "-[edge.e]",
                "-kind = \"edge\"",
                "-from = { node = \"a\" }",
                "-to = { at = [600, 300] }",
                "-z = \"X\"",
            ],
        );
        // A node takes its descendants and its connectors with it.
        let (plan, board) = planned(BOARD, &delete(nodes(&["a"])), &[]);
        let left: Vec<&str> = board.nodes.keys().map(Id::as_str).collect();
        assert_eq!(left, ["c", "t"]);
        assert!(board.edges.is_empty());
        assert_eq!(plan.select, Some(Vec::new()));
        let (_, board) = planned(BOARD, &delete(nodes(&["c"])), &[]);
        let left: Vec<&str> = board.edges.keys().map(Id::as_str).collect();
        assert_eq!(left, ["e"]);
        assert_empty(BOARD, delete(nodes(&["gone"])));
        assert_empty(BOARD, delete(edges(&["gone"])));
        // A node named as a connector is not deleted, nor the other way.
        assert_empty(BOARD, delete(edges(&["a"])));
        assert_empty(BOARD, delete(nodes(&["e"])));
        assert_empty(BOARD, delete(Vec::new()));
    }

    /// Integrity damage around a deleted node never turns into a parent
    /// that names nothing.
    #[test]
    fn delete_follows_stored_parents_through_damage() {
        let node = |id: &str, parent: Option<&str>| {
            let parent = parent.map_or(String::new(), |p| format!("parent = \"{p}\"\n"));
            format!("\n[node.{id}]\nkind = \"shape\"\nat = [0, 0]\n{parent}size = [20, 20]\nz = \"V\"\n")
        };
        // x and y are a cycle, w hangs from it, v is an orphan.
        let mut text = String::from("schema = 1\n");
        for (id, parent) in [
            ("v", Some("gone")),
            ("w", Some("x")),
            ("x", Some("y")),
            ("y", Some("x")),
        ] {
            text += &node(id, parent);
        }
        // An edge with the same id as a node.
        text += "\n[edge.v]\nkind = \"edge\"\nfrom = { at = [0, 0] }\nto = { at = [9, 9] }\nz = \"V\"\n";
        let before = check(&parse(&text).unwrap());
        let (_, board) = planned(&text, &Intent::Delete { ids: nodes(&["x"]) }, &[]);
        let left: Vec<&str> = board.nodes.keys().map(Id::as_str).collect();
        assert_eq!(left, ["v"]);
        for issue in check(&board) {
            assert!(before.contains(&issue), "{issue:?}");
        }
        // An id naming a node and a connector removes the one named with
        // its kind, and both only when both are named (§4.12).
        let (_, board) = planned(&text, &Intent::Delete { ids: nodes(&["v"]) }, &[]);
        assert!(!board.nodes.contains_key("v") && board.edges.contains_key("v"));
        let (_, board) = planned(&text, &Intent::Delete { ids: edges(&["v"]) }, &[]);
        assert!(board.nodes.contains_key("v") && board.edges.is_empty());
        let both = [nodes(&["v"]), edges(&["v"])].concat();
        let (_, board) = planned(&text, &Intent::Delete { ids: both }, &[]);
        assert!(!board.nodes.contains_key("v") && board.edges.is_empty());

        // A chain 34 deep: below level 32 nodes are drawn as roots, yet
        // still go with the node above them.
        let mut chain = String::from("schema = 1\n");
        for k in 0..34 {
            let parent = (k > 0).then(|| format!("n{:02}", k - 1));
            chain += &node(&format!("n{k:02}"), parent.as_deref());
        }
        let (_, board) = planned(
            &chain,
            &Intent::Delete {
                ids: nodes(&["n31"]),
            },
            &[],
        );
        assert_eq!(board.nodes.len(), 31);
        assert!(check(&board).is_empty(), "{:?}", check(&board));
    }

    /// A cut is copy then delete (§5.10 rule 5), so the copy must carry
    /// every node the delete removes, even one a parent cycle or the depth
    /// cap draws as a root elsewhere, each where it was drawn.
    #[test]
    fn a_cut_copies_every_node_it_deletes() {
        let node = |id: &str, parent: Option<&str>, at: [i32; 2]| {
            let parent = parent.map_or(String::new(), |p| format!("parent = \"{p}\"\n"));
            format!(
                "\n[node.{id}]\nkind = \"shape\"\nat = [{}, {}]\n{parent}size = [20, 20]\nz = \"V\"\n",
                at[0], at[1]
            )
        };
        let mut cycle = String::from("schema = 1\n");
        cycle += &node("x", Some("y"), [0, 0]);
        cycle += &node("y", Some("x"), [400, 400]);
        let mut chain = String::from("schema = 1\n");
        for k in 0..34 {
            let parent = (k > 0).then(|| format!("n{:02}", k - 1));
            chain += &node(&format!("n{k:02}"), parent.as_deref(), [10, 10]);
        }
        for (text, cut) in [(&cycle, "x"), (&chain, "n31")] {
            let board = parse(text).unwrap();
            let copied = crate::clip::copy(&board, &ids(&[cut])).unwrap();
            let body = copied.strip_prefix(crate::clip::HEADER).unwrap();
            let fragment = parse(body).unwrap();
            let (_, after) = planned(text, &Intent::Delete { ids: nodes(&[cut]) }, &[]);
            let removed: Vec<&Id> = board
                .nodes
                .keys()
                .filter(|id| !after.nodes.contains_key(*id))
                .collect();
            let carried: Vec<&Id> = fragment.nodes.keys().collect();
            assert_eq!(carried, removed, "{cut}");
            assert!(check(&fragment).is_empty(), "{:?}", check(&fragment));
            let (drawn, copy) = (crate::placements(&board), crate::placements(&fragment));
            for id in carried {
                assert_eq!(copy[id].at, drawn[id].at, "{id:?}");
            }
        }
    }

    #[test]
    fn duplicate_copies_the_fragment_on_top() {
        let duplicate = |names: &[&str]| Intent::Duplicate {
            ids: ids(names),
            by: [20, 20],
        };
        // `a` brings its child and the connector from it to a free point;
        // `f` stays, since `c` is not copied.
        assert_lines(
            duplicate(&["a"]),
            &["d1", "d2", "d3"],
            &[
                "+[node.d1]",
                "+text = \"Outer\"",
                "+kind = \"shape\"",
                "+at = [20, 20]",
                "+ext.tool.note = \"kept\"",
                "+size = [200, 120]",
                "+z = \"Y01\"",
                "+[node.d2]",
                "+text = \"Inner\"",
                "+kind = \"shape\"",
                "+at = [20, 20]",
                "+parent = \"d1\"",
                "+size = [120, 60]",
                "+z = \"V\"",
                "+[edge.d3]",
                "+kind = \"edge\"",
                "+from = { node = \"d1\" }",
                "+to = { at = [620, 320] }",
                "+z = \"Y02\"",
            ],
        );
        // A child alone is copied at the top level, at its absolute place.
        assert_lines(
            duplicate(&["b"]),
            &["d1"],
            &[
                "+[node.d1]",
                "+text = \"Inner\"",
                "+kind = \"shape\"",
                "+at = [40, 40]",
                "+size = [120, 60]",
                "+z = \"Y01\"",
            ],
        );
        // Unknown kinds and keys are copied as they are.
        assert_lines(
            duplicate(&["c", "a", "b"]),
            &["d1", "d2", "d3", "d4", "d5"],
            &[
                "+[node.d1]",
                "+text = \"Outer\"",
                "+kind = \"shape\"",
                "+at = [20, 20]",
                "+ext.tool.note = \"kept\"",
                "+size = [200, 120]",
                "+z = \"Y01\"",
                "+[node.d2]",
                "+text = \"Inner\"",
                "+kind = \"shape\"",
                "+at = [20, 20]",
                "+parent = \"d1\"",
                "+size = [120, 60]",
                "+z = \"V\"",
                "+[node.d3]",
                "+kind = \"future\"",
                "+at = [420, 20]",
                "+locked = true",
                "+mystery = 1",
                "+size = [120, 60]",
                "+z = \"Y02\"",
                "+[edge.d4]",
                "+kind = \"edge\"",
                "+from = { node = \"d1\" }",
                "+to = { at = [620, 320] }",
                "+z = \"Y03\"",
                "+[edge.d5]",
                "+kind = \"edge\"",
                "+from = { node = \"d1\" }",
                "+to = { node = \"d3\" }",
                "+z = \"Y04\"",
            ],
        );
        let (plan, _) = planned(BOARD, &duplicate(&["a"]), &["d1", "d2", "d3"]);
        assert_eq!(plan.select, Some(ids(&["d1", "d2", "d3"])));
        // A selection without a node has no fragment.
        assert_empty(BOARD, duplicate(&["e", "f"]));
        assert_empty(BOARD, duplicate(&["gone"]));
    }

    /// A copy never has a parent that names nothing, or a cycle: a node is
    /// copied inside its parent only when it is drawn there.
    #[test]
    fn duplicate_lays_damaged_parents_out_as_roots() {
        let text = "schema = 1\n\n\
                    [node.x]\nkind = \"shape\"\nat = [5, 5]\nparent = \"y\"\nsize = [20, 20]\nz = \"V\"\n\n\
                    [node.y]\nkind = \"shape\"\nat = [0, 0]\nparent = \"x\"\nsize = [20, 20]\nz = \"W\"\n\n\
                    [node.v]\nkind = \"shape\"\nat = [40, 0]\nparent = \"gone\"\nsize = [20, 20]\n";
        let board = parse(text).unwrap();
        let mut copy = board.clone();
        let plan = plan(
            &board,
            &Intent::Duplicate {
                ids: ids(&["x", "y", "v"]),
                by: [20, 0],
            },
            &mut Listed(vec!["d1", "d2", "d3"]),
        );
        copy.apply(&plan.changes);
        for id in ["d1", "d2", "d3"] {
            assert_eq!(copy.nodes[id].parent, None, "{id}");
            assert!(copy.nodes[id].z.is_some(), "{id}");
        }
        let mut issues: Vec<_> = check(&copy)
            .into_iter()
            .map(|issue| issue.id().clone())
            .collect();
        issues.dedup();
        assert_eq!(issues, ids(&["v", "x", "y"]));
    }

    #[test]
    fn a_read_only_board_plans_nothing() {
        let text = "schema = 1\nrequires = [\"frames\"]\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n";
        let board = parse(text).unwrap();
        let plan = plan(
            &board,
            &Intent::Delete { ids: nodes(&["a"]) },
            &mut Listed(Vec::new()),
        );
        assert!(plan.is_empty());
        assert_eq!(
            plan.notice.as_deref(),
            Some("This board is read-only: it needs features this build does not have: frames.")
        );
        let newer = parse("schema = 2\n").unwrap();
        let plan = super::plan(
            &newer,
            &Intent::AddNode {
                kind: NodeKind::Shape,
                shape: Shape::Rect,
                at: [0, 0],
                size: [20, 20],
                from: None,
            },
            &mut Listed(Vec::new()),
        );
        assert_eq!(
            plan.notice.as_deref(),
            Some("This board is read-only: it is written for board schema 2.")
        );
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
