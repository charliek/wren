//! The connector intents (§5.12) and keyboard building (§5.6): add,
//! re-attach, restyle, reverse and relabel a connector, and extend from a
//! node.

use std::collections::BTreeSet;

use super::{connector, dangling_names, node, on_top, trim_label, Change, EdgeEnd, EdgeField};
use super::{Endpoint, Plan};
use crate::ids::{fresh, Ids};
use crate::place::{self, node_of};
use crate::{Board, Dir, Edge, End, Id, Label, Named, NodeKind, Shape};

/// A new connector on top of the root scope, with no label, selected
/// (§5.12 rules 1 and 5). An end on a node stores the node, and its side
/// only when one is given. Nothing is made when an end names a node that
/// is not there, or when both ends name the same node: the pointer cancels
/// a release on the source, and keyboard connecting does nothing there.
pub(super) fn add(board: &Board, from: &Endpoint, to: &Endpoint, ids: &mut dyn Ids) -> Plan {
    if !attachable(board, from) || !attachable(board, to) || same_node(from, to) {
        return Plan::default();
    }
    let id = fresh(board, &dangling_names(board), ids);
    let (keys, mut changes) = on_top(board, 1);
    let edge = connector(from, to, keys.into_iter().next());
    changes.push(Change::PutEdge(id.clone(), Box::new(edge)));
    Plan {
        changes,
        select: Some(vec![id]),
        ..Plan::default()
    }
}

fn attachable(board: &Board, end: &Endpoint) -> bool {
    match end {
        Endpoint::Node { node, .. } => board.nodes.contains_key(node),
        Endpoint::Free { .. } => true,
    }
}

fn same_node(a: &Endpoint, b: &Endpoint) -> bool {
    matches!((a, b), (Endpoint::Node { node: a, .. }, Endpoint::Node { node: b, .. }) if a == b)
}

/// Moves one end of a connector (§5.12 rules 5 and 6). Dropped on a node,
/// it is attached there, pinned only when a side is given, and any anchor
/// goes. Dropped on empty canvas, an end that is a free point moves there
/// and one that is attached stays where it was. As with [`add`], an end
/// never names a node that is not there, nor the node the other end names.
pub(super) fn rebind(board: &Board, id: &Id, which: EdgeEnd, to: &Endpoint) -> Plan {
    let Some(edge) = board.edges.get(id) else {
        return Plan::default();
    };
    let (end, other) = match which {
        EdgeEnd::From => (&edge.from, &edge.to),
        EdgeEnd::To => (&edge.to, &edge.from),
    };
    let moved = match (to, end) {
        (Endpoint::Free { at }, End::Free { extra, .. }) => End::Free {
            at: *at,
            extra: extra.clone(),
        },
        (Endpoint::Free { .. }, End::Node { .. }) => return Plan::default(),
        (Endpoint::Node { node, side }, _) => {
            if !board.nodes.contains_key(node) || node_of(other) == Some(node.as_str()) {
                return Plan::default();
            }
            // Keys a free end keeps as unknown content name an attached
            // end's own fields, so they do not travel to one.
            let mut extra = match end {
                End::Node { extra, .. } => extra.clone(),
                End::Free { extra, .. } => {
                    let mut extra = extra.clone();
                    for key in ["node", "side", "anchor", "at"] {
                        extra.remove(key);
                    }
                    extra
                }
            };
            extra.remove("at");
            End::Node {
                node: node.as_str().to_owned(),
                side: side.map(Named::Known),
                anchor: None,
                extra,
            }
        }
    };
    if &moved == end {
        return Plan::default();
    }
    let mut edge = edge.clone();
    match which {
        EdgeEnd::From => edge.from = moved,
        EdgeEnd::To => edge.to = moved,
    }
    Plan {
        changes: vec![Change::PutEdge(id.clone(), Box::new(edge))],
        ..Plan::default()
    }
}

/// Sets one style field of each connector among `ids` (§5.12 rule 9).
pub(super) fn set_field(board: &Board, ids: &[Id], field: EdgeField) -> Plan {
    let chosen: BTreeSet<&Id> = ids.iter().collect();
    let mut changes = Vec::new();
    for id in chosen {
        let Some(edge) = board.edges.get(id) else {
            continue;
        };
        let mut edge = edge.clone();
        let changed = match field {
            EdgeField::Route(route) => replace(&mut edge.route, Named::Known(route)),
            EdgeField::Head(end, head) => {
                let slot = match end {
                    EdgeEnd::From => &mut edge.heads[0],
                    EdgeEnd::To => &mut edge.heads[1],
                };
                replace(slot, Named::Known(head))
            }
            EdgeField::Stroke(dash) => replace(&mut edge.style.stroke, Named::Known(dash)),
        };
        if changed {
            changes.push(Change::PutEdge(id.clone(), Box::new(edge)));
        }
    }
    Plan {
        changes,
        ..Plan::default()
    }
}

/// Puts `value` in `slot`, saying whether that changed it.
fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    let changed = *slot != value;
    *slot = value;
    changed
}

/// The whole route's length in a label's per-mille `at`.
const WHOLE: i32 = 1000;

/// Swaps each connector's ends (§5.12 rule 9). The heads stay by
/// position: swapped with the ends they would draw the same arrow, and the
/// command would change only what the flow vote and walking read. A
/// label's place is measured from the `from` end, so it is mirrored and
/// stays where it was drawn.
pub(super) fn reverse(board: &Board, ids: &[Id]) -> Plan {
    let chosen: BTreeSet<&Id> = ids.iter().collect();
    let mut changes = Vec::new();
    for id in chosen {
        let Some(edge) = board.edges.get(id) else {
            continue;
        };
        let mirrored = |at: i32| WHOLE.saturating_sub(at);
        let unchanged = edge.from == edge.to
            && edge
                .labels
                .iter()
                .all(|label| mirrored(label.at) == label.at);
        if unchanged {
            continue;
        }
        let edge = Edge {
            from: edge.to.clone(),
            to: edge.from.clone(),
            labels: edge
                .labels
                .iter()
                .map(|label| Label {
                    at: mirrored(label.at),
                    ..label.clone()
                })
                .collect(),
            ..edge.clone()
        };
        changes.push(Change::PutEdge(id.clone(), Box::new(edge)));
    }
    Plan {
        changes,
        ..Plan::default()
    }
}

/// A connector's first label, trimmed as a node's is (§5.9 rule 4); a new
/// one goes in the middle of the route (§5.12 rule 8). An empty text
/// removes that label only at the commit: the label editor sends one
/// intent per change, so removing it on the way to retyping would write
/// the next keystroke into the second label. Further labels are kept as
/// they are.
pub(super) fn set_label(board: &Board, id: &Id, text: &str, commit: bool) -> Plan {
    let Some(edge) = board.edges.get(id) else {
        return Plan::default();
    };
    let text = trim_label(text);
    let mut labels = edge.labels.clone();
    match labels.first_mut() {
        None if text.is_empty() => return Plan::default(),
        None => labels.push(Label {
            text,
            at: Label::DEFAULT_AT,
            extra: crate::Extra::new(),
        }),
        Some(_) if text.is_empty() && commit => {
            labels.remove(0);
        }
        Some(first) if first.text == text => return Plan::default(),
        Some(first) => first.text = text,
    }
    let edge = Edge {
        labels,
        ..edge.clone()
    };
    Plan {
        changes: vec![Change::PutEdge(id.clone(), Box::new(edge))],
        ..Plan::default()
    }
}

/// Keyboard building from `from` (§5.6 rules 5 to 9): a node or, for a
/// connector, the node at its `to` end.
///
/// With a direction and no armed shape, a node in the ideal slot that has
/// no connector to or from the source is joined: one new connector, and
/// that node is selected. Otherwise a new node of the default size, in the
/// armed shape or a rectangle, goes in the nearest free slot that way (the
/// flow direction for `board.connect.next`) with a connector from the
/// source, both in one step, and its label opens. Connectors made here
/// carry no side: `{ node }` at each end.
pub(super) fn extend(
    board: &Board,
    from: &Id,
    dir: Option<Dir>,
    shape: Option<Shape>,
    ids: &mut dyn Ids,
) -> Plan {
    let source = if board.nodes.contains_key(from) {
        from.clone()
    } else {
        let Some(name) = board.edges.get(from).and_then(|edge| node_of(&edge.to)) else {
            return Plan::default();
        };
        match board.nodes.get_key_value(name) {
            Some((id, _)) => id.clone(),
            None => return Plan::default(),
        }
    };
    let attach = |node: &Id| Endpoint::Node {
        node: node.clone(),
        side: None,
    };
    if let (Some(dir), None) = (dir, shape) {
        if let Some(target) = place::occupant(board, &source, place::SIZE, dir) {
            let linked =
                place::joins(board, &source, &target) || place::joins(board, &target, &source);
            if !linked {
                let mut plan = add(board, &attach(&source), &attach(&target), ids);
                plan.select = Some(vec![target]);
                return plan;
            }
        }
    }
    let dir = dir.unwrap_or_else(|| place::flow(board));
    let Some(at) = place::slot(board, &source, place::SIZE, dir) else {
        return Plan::notice(format!(
            "There is no free place for a new shape within {} steps of this one.",
            place::STEPS
        ));
    };
    node::add(
        board,
        NodeKind::Shape,
        shape.unwrap_or(Shape::Rect),
        at,
        place::SIZE,
        Some(&attach(&source)),
        ids,
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::{ids, line_diff, Listed};
    use super::super::{plan, Intent};
    use super::*;
    use crate::{check, parse, write, Dash, Head, Route, Side};

    /// Connectors with unknown content: `e` has `ext`, `locked`, an
    /// unknown key in its `to` end and two labels; `f` has an unknown kind,
    /// a free end with an unknown key, and an unknown key of its own.
    const EDGES: &str = r#"schema = 1

[node.a]
text = "A"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "V"

[node.b]
text = "B"
kind = "shape"
at = [0, 120]
size = [120, 60]
z = "W"

[node.c]
kind = "future"
at = [200, 0]
size = [120, 60]
z = "X"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b", port = 2 }
ext.tool.note = "kept"
labels = [
  { at = 250, text = "Yes" },
  { text = "second" },
]
locked = true
z = "Y"

[edge.f]
kind = "wire"
from = { at = [400, 300], tag = "x" }
to = { node = "c" }
mystery = 1
z = "Z"
"#;

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    fn on(node: &str, side: Option<Side>) -> Endpoint {
        Endpoint::Node {
            node: id(node),
            side,
        }
    }

    /// The plan for `intent` on `text` and the lines of `write` it changes.
    fn diff(text: &str, intent: &Intent, fresh: &[&'static str]) -> (Plan, Vec<String>) {
        let mut board = parse(text).unwrap();
        assert_eq!(write(&board), text, "the fixture is canonical");
        let before = check(&board);
        let plan = plan(&board, intent, &mut Listed(fresh.to_vec()));
        board.apply(&plan.changes);
        let written = write(&board);
        assert_eq!(write(&parse(&written).unwrap()), written, "round trip");
        assert_eq!(check(&board), before, "{intent:?}");
        (plan, line_diff(text, &written))
    }

    #[track_caller]
    fn assert_lines(intent: Intent, fresh: &[&'static str], expected: &[&str]) {
        assert_eq!(diff(EDGES, &intent, fresh).1, expected, "{intent:?}");
    }

    #[track_caller]
    fn assert_empty(text: &str, intent: Intent) {
        let (plan, lines) = diff(text, &intent, &[]);
        assert!(plan.is_empty() && lines.is_empty(), "{intent:?}: {plan:?}");
    }

    #[test]
    fn a_new_connector_is_one_block_on_top_and_selected() {
        let add = |from, to| Intent::AddEdge { from, to };
        let block = |from: &'static str, to: &'static str| {
            vec!["+[edge.n1]", "+kind = \"edge\"", from, to, "+z = \"Z01\""]
        };
        let table = [
            // Dropped on a node: the node only.
            (
                add(on("a", None), on("c", None)),
                block("+from = { node = \"a\" }", "+to = { node = \"c\" }"),
            ),
            // Dropped on a side handle: pinned.
            (
                add(on("a", Some(Side::Right)), on("c", Some(Side::Left))),
                block(
                    "+from = { node = \"a\", side = \"right\" }",
                    "+to = { node = \"c\", side = \"left\" }",
                ),
            ),
            (
                add(Endpoint::Free { at: [0, 300] }, on("b", None)),
                block("+from = { at = [0, 300] }", "+to = { node = \"b\" }"),
            ),
        ];
        for (intent, expected) in table {
            assert_lines(intent.clone(), &["n1"], &expected);
            let (plan, _) = diff(EDGES, &intent, &["n1"]);
            assert_eq!(plan.select, Some(ids(&["n1"])));
            assert_eq!(plan.edit, None);
        }
        // A second connector between the same pair is allowed.
        assert_eq!(
            diff(EDGES, &add(on("a", None), on("b", None)), &["n1"])
                .1
                .len(),
            5
        );
        assert_empty(EDGES, add(on("a", None), on("a", Some(Side::Top))));
        assert_empty(EDGES, add(on("a", None), on("gone", None)));
    }

    #[test]
    fn a_new_id_is_none_already_taken() {
        let intent = Intent::AddEdge {
            from: on("a", None),
            to: on("b", None),
        };
        let (plan, _) = diff(EDGES, &intent, &["a", "e", "f", "n1"]);
        assert_eq!(plan.select, Some(ids(&["n1"])));
    }

    #[test]
    fn rebinding_an_end_changes_its_line() {
        let rebind = |edge: &str, end, to| Intent::Rebind {
            id: id(edge),
            end,
            to,
        };
        let table: Vec<(Intent, &[&str])> = vec![
            // Unknown keys of the end travel with it.
            (
                rebind("e", EdgeEnd::To, on("c", None)),
                &[
                    "-to = { node = \"b\", port = 2 }",
                    "+to = { node = \"c\", port = 2 }",
                ],
            ),
            (
                rebind("e", EdgeEnd::From, on("c", Some(Side::Bottom))),
                &[
                    "-from = { node = \"a\" }",
                    "+from = { node = \"c\", side = \"bottom\" }",
                ],
            ),
            // A free end moves; one dropped on a node attaches.
            (
                rebind("f", EdgeEnd::From, Endpoint::Free { at: [420, 320] }),
                &[
                    "-from = { at = [400, 300], tag = \"x\" }",
                    "+from = { at = [420, 320], tag = \"x\" }",
                ],
            ),
            (
                rebind("f", EdgeEnd::From, on("a", None)),
                &[
                    "-from = { at = [400, 300], tag = \"x\" }",
                    "+from = { node = \"a\", tag = \"x\" }",
                ],
            ),
        ];
        for (intent, expected) in table {
            assert_lines(intent, &[], expected);
        }
        // An attached end dropped on empty canvas stays; so does one
        // dropped where it is, on a node that is gone, or on the node the
        // other end names.
        for intent in [
            rebind("e", EdgeEnd::To, Endpoint::Free { at: [0, 0] }),
            rebind("e", EdgeEnd::From, on("a", None)),
            rebind("e", EdgeEnd::From, on("gone", None)),
            rebind("e", EdgeEnd::From, on("b", None)),
            rebind("gone", EdgeEnd::From, on("b", None)),
        ] {
            assert_empty(EDGES, intent);
        }
    }

    #[test]
    fn rebinding_drops_the_anchor_and_a_free_ends_attached_keys() {
        let text =
            "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n\n\
                    [node.b]\nkind = \"shape\"\nat = [40, 0]\nsize = [20, 20]\nz = \"W\"\n\n\
                    [edge.e]\nkind = \"edge\"\nfrom = { anchor = [500, 0], node = \"a\" }\n\
                    to = { anchor = [1, 2], at = [9, 9], side = \"top\" }\nz = \"X\"\n";
        let (_, lines) = diff(
            text,
            &Intent::Rebind {
                id: id("e"),
                end: EdgeEnd::From,
                to: on("a", Some(Side::Left)),
            },
            &[],
        );
        assert_eq!(
            lines,
            [
                "-from = { anchor = [500, 0], node = \"a\" }",
                "+from = { node = \"a\", side = \"left\" }"
            ]
        );
        let (_, lines) = diff(
            text,
            &Intent::Rebind {
                id: id("e"),
                end: EdgeEnd::To,
                to: on("b", None),
            },
            &[],
        );
        assert_eq!(
            lines,
            [
                "-to = { anchor = [1, 2], at = [9, 9], side = \"top\" }",
                "+to = { node = \"b\" }"
            ]
        );
    }

    #[test]
    fn a_style_command_changes_one_line_per_connector() {
        let set = |names: &[&str], field| Intent::SetEdge {
            ids: ids(names),
            field,
        };
        let table: Vec<(Intent, &[&str])> = vec![
            (
                set(&["e"], EdgeField::Route(Route::Straight)),
                &["+route = \"straight\""],
            ),
            (
                set(&["e"], EdgeField::Head(EdgeEnd::From, Head::Arrow)),
                &["+heads = [\"arrow\", \"arrow\"]"],
            ),
            (
                set(&["e", "f", "a"], EdgeField::Head(EdgeEnd::To, Head::None)),
                &[
                    "+heads = [\"none\", \"none\"]",
                    "+heads = [\"none\", \"none\"]",
                ],
            ),
            (
                set(&["f"], EdgeField::Stroke(Dash::Dashed)),
                &["+style.stroke = \"dashed\""],
            ),
        ];
        for (intent, expected) in table {
            assert_lines(intent, &[], expected);
        }
        for intent in [
            set(&["e", "f"], EdgeField::Route(Route::Elbow)),
            set(&["e"], EdgeField::Head(EdgeEnd::To, Head::Arrow)),
            set(&["e"], EdgeField::Stroke(Dash::Solid)),
            set(&["a", "gone"], EdgeField::Stroke(Dash::Dashed)),
        ] {
            assert_empty(EDGES, intent);
        }
    }

    #[test]
    fn reversing_swaps_the_ends_and_label_places_and_turns_the_arrow() {
        // The default heads stay by position, so the arrow now ends at a.
        assert_lines(
            Intent::Reverse { ids: ids(&["e"]) },
            &[],
            &[
                "-from = { node = \"a\" }",
                "-to = { node = \"b\", port = 2 }",
                "+from = { node = \"b\", port = 2 }",
                "+to = { node = \"a\" }",
                "-  { at = 250, text = \"Yes\" },",
                "+  { at = 750, text = \"Yes\" },",
            ],
        );
        // Reversing twice gives the file back.
        let mut board = parse(EDGES).unwrap();
        for _ in 0..2 {
            let reverse = plan(
                &board,
                &Intent::Reverse {
                    ids: ids(&["e", "f", "a"]),
                },
                &mut Listed(Vec::new()),
            );
            board.apply(&reverse.changes);
        }
        assert_eq!(write(&board), EDGES);
        // A loop with a middle label is unchanged, whatever its heads.
        let text =
            "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n\n\
                    [edge.e]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"a\" }\n\
                    heads = [\"arrow\", \"none\"]\nlabels = [\n  { text = \"x\" },\n]\nz = \"X\"\n";
        assert_empty(text, Intent::Reverse { ids: ids(&["e"]) });
    }

    /// A dangling connector stays dangling at one end when reversed.
    #[test]
    fn reversing_a_dangling_connector_keeps_its_issue() {
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n\n\
                    [edge.e]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"gone\" }\nz = \"X\"\n";
        let mut board = parse(text).unwrap();
        let reverse = plan(
            &board,
            &Intent::Reverse { ids: ids(&["e"]) },
            &mut Listed(Vec::new()),
        );
        board.apply(&reverse.changes);
        assert_eq!(check(&board).len(), 1);
        assert_eq!(board.edges["e"].from, parse(text).unwrap().edges["e"].to);
    }

    #[test]
    fn a_connector_label_is_its_first_item() {
        let label = |edge: &str, text: &str| Intent::SetEdgeLabel {
            id: id(edge),
            text: text.into(),
            commit: false,
        };
        let commit = |edge: &str, text: &str| Intent::SetEdgeLabel {
            id: id(edge),
            text: text.into(),
            commit: true,
        };
        let table: Vec<(Intent, &[&str])> = vec![
            // A relabel, even to several lines, is one line; the place and
            // the second label stay.
            (
                label("e", "Yes,\nshipped\n\n"),
                &[
                    "-  { at = 250, text = \"Yes\" },",
                    "+  { at = 250, text = \"Yes,\\nshipped\" },",
                ],
            ),
            // While editing, an empty text keeps the item and its place.
            (
                label("e", " \n"),
                &[
                    "-  { at = 250, text = \"Yes\" },",
                    "+  { at = 250, text = \"\" },",
                ],
            ),
            // Committed empty, the first label alone is removed.
            (commit("e", " \n"), &["-  { at = 250, text = \"Yes\" },"]),
            // A commit with text is a relabel.
            (
                commit("e", "No"),
                &[
                    "-  { at = 250, text = \"Yes\" },",
                    "+  { at = 250, text = \"No\" },",
                ],
            ),
            // The first label goes in the middle.
            (
                label("f", "No"),
                &["+labels = [", "+  { text = \"No\" },", "+]"],
            ),
        ];
        for (intent, expected) in table {
            assert_lines(intent, &[], expected);
        }
        for intent in [
            label("e", "Yes"),
            label("e", "Yes\r\n"),
            label("f", ""),
            commit("f", ""),
            label("a", "x"),
        ] {
            assert_empty(EDGES, intent);
        }
    }

    /// The label editor sends one intent per change: backspacing a first
    /// label away and typing a new one keeps its place, its unknown keys and
    /// the second label.
    #[test]
    fn a_connector_label_erased_while_editing_keeps_its_item() {
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n\n\
                    [node.b]\nkind = \"shape\"\nat = [0, 100]\nsize = [20, 20]\nz = \"W\"\n\n\
                    [edge.e]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"b\" }\nlabels = [\n  \
                    { at = 250, note = \"kept\", text = \"Yes\" },\n  { text = \"second\" },\n]\nz = \"X\"\n";
        let mut board = parse(text).unwrap();
        assert_eq!(write(&board), text, "the fixture is canonical");
        for typed in ["Ye", "Y", "", "N", "No"] {
            let intent = Intent::SetEdgeLabel {
                id: id("e"),
                text: typed.to_owned(),
                commit: false,
            };
            let step = plan(&board, &intent, &mut Listed(Vec::new()));
            board.apply(&step.changes);
        }
        assert_eq!(
            line_diff(text, &write(&board)),
            [
                "-  { at = 250, note = \"kept\", text = \"Yes\" },",
                "+  { at = 250, note = \"kept\", text = \"No\" },",
            ]
        );
        // Typed into a connector with no label and erased again, the
        // commit leaves the file as it was.
        let mut board = parse(EDGES).unwrap();
        for (typed, commit) in [("N", false), ("", false), ("", true)] {
            let intent = Intent::SetEdgeLabel {
                id: id("f"),
                text: typed.to_owned(),
                commit,
            };
            let step = plan(&board, &intent, &mut Listed(Vec::new()));
            board.apply(&step.changes);
        }
        assert_eq!(write(&board), EDGES);
    }

    /// A tidy board for keyboard building: `s` with `t` below it.
    const BUILT: &str = "schema = 1\n\n[node.s]\ntext = \"S\"\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\nz = \"V\"\n\n\
                         [node.t]\ntext = \"T\"\nkind = \"shape\"\nat = [0, 120]\nsize = [120, 60]\nz = \"W\"\n\n\
                         [edge.st]\nkind = \"edge\"\nfrom = { node = \"s\" }\nto = { node = \"t\" }\nz = \"X\"\n";

    fn extend(from: &str, dir: Option<Dir>, shape: Option<Shape>) -> Intent {
        Intent::Extend {
            from: id(from),
            dir,
            shape,
        }
    }

    #[test]
    fn extending_creates_a_node_and_a_connector_in_one_step() {
        let (plan, lines) = diff(BUILT, &extend("t", None, None), &["n1", "n2"]);
        assert_eq!(
            lines,
            [
                "+[node.n1]",
                "+kind = \"shape\"",
                "+at = [0, 240]",
                "+size = [120, 60]",
                "+z = \"X01\"",
                "+[edge.n2]",
                "+kind = \"edge\"",
                "+from = { node = \"t\" }",
                "+to = { node = \"n1\" }",
                "+z = \"X02\"",
            ]
        );
        assert_eq!(plan.select, Some(ids(&["n1"])));
        assert_eq!(plan.edit, Some(id("n1")));
        // From a connector: its `to` node. Below t is free.
        let (_, from_edge) = diff(BUILT, &extend("st", None, None), &["n1", "n2"]);
        assert_eq!(from_edge, lines);
        // Tab from s: below is t, so the next slot across, to the right.
        let (_, lines) = diff(
            BUILT,
            &extend("s", None, Some(Shape::Diamond)),
            &["n1", "n2"],
        );
        assert_eq!(&lines[2..4], ["+at = [200, 120]", "+shape = \"diamond\""]);
        for intent in [
            extend("gone", None, None),
            extend("gone", Some(Dir::Up), None),
        ] {
            assert_empty(BUILT, intent);
        }
    }

    #[test]
    fn extending_a_way_joins_the_node_already_there() {
        let text = format!(
            "{BUILT}\n[node.u]\ntext = \"U\"\nkind = \"shape\"\nat = [200, 0]\nsize = [120, 60]\nz = \"Y\"\n"
        );
        let text = write(&parse(&text).unwrap());
        // u is in s's right-hand slot and not joined: connect, select it,
        // open no label.
        let (plan, lines) = diff(&text, &extend("s", Some(Dir::Right), None), &["n1"]);
        assert_eq!(
            lines,
            [
                "+[edge.n1]",
                "+kind = \"edge\"",
                "+from = { node = \"s\" }",
                "+to = { node = \"u\" }",
                "+z = \"Y01\"",
            ]
        );
        assert_eq!(plan.select, Some(ids(&["u"])));
        assert_eq!(plan.edit, None);
        // t is below s and already joined: a new node in the next free
        // slot instead.
        let (plan, lines) = diff(&text, &extend("s", Some(Dir::Down), None), &["n1", "n2"]);
        assert_eq!(lines[2], "+at = [200, 120]");
        assert_eq!(plan.edit, Some(id("n1")));
        // Joined the other way counts too, and an armed shape always
        // creates.
        let (_, lines) = diff(&text, &extend("t", Some(Dir::Up), None), &["n1", "n2"]);
        assert_eq!(lines[0], "+[node.n1]");
        let (_, lines) = diff(
            &text,
            &extend("s", Some(Dir::Right), Some(Shape::Rect)),
            &["n1", "n2"],
        );
        assert_eq!(lines[2], "+at = [200, 120]");
    }

    /// The container a child is drawn in fills the child's slots, yet it
    /// is not a node to join: a new node is built outside it.
    #[test]
    fn extending_a_child_does_not_join_its_container() {
        let text = "schema = 1\n\n[node.c]\nkind = \"shape\"\nat = [20, 20]\nparent = \"p\"\nsize = [120, 60]\nz = \"V\"\n\n\
                    [node.p]\nkind = \"shape\"\nat = [0, 0]\nsize = [400, 400]\nz = \"V\"\n";
        let (plan, lines) = diff(text, &extend("c", Some(Dir::Down), None), &["n1", "n2"]);
        assert_eq!(plan.select, Some(ids(&["n1"])));
        assert_eq!(plan.edit, Some(id("n1")));
        assert_eq!(lines[0], "+[node.n1]");
        assert!(
            lines.contains(&"+to = { node = \"n1\" }".to_owned()),
            "{lines:?}"
        );
    }

    #[test]
    fn extending_with_no_free_slot_says_so() {
        let text = "schema = 1\n\n[node.s]\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\nz = \"V\"\n\n\
                    [node.w]\nkind = \"shape\"\nat = [-100000, 100]\nsize = [200000, 100]\nz = \"W\"\n";
        let board = parse(text).unwrap();
        let plan = plan(
            &board,
            &extend("s", Some(Dir::Down), Some(Shape::Rect)),
            &mut Listed(Vec::new()),
        );
        assert!(plan.is_empty());
        assert_eq!(
            plan.notice.as_deref(),
            Some("There is no free place for a new shape within 200 steps of this one.")
        );
    }
}
