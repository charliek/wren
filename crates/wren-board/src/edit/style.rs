//! The colour, fill and outline of the selected elements (§5.16 rule 1).

use std::collections::BTreeSet;

use super::{Change, Plan, StyleField};
use crate::scene::Source;
use crate::{Board, Color, Dash, Edge, Id, Named, Node, NodeKind};

/// Sets one style field of each element among `ids` that has it. Colour is
/// a shape's, a text node's and a connector's; fill a shape's; the outline
/// a shape's and a connector's line. A node of an unknown kind or none
/// draws as a placeholder whatever its style, so it is left alone, as
/// reshaping leaves it. An element already holding the value is not
/// rewritten, so a command that changes nothing sends no draft.
pub(super) fn set_style(board: &Board, ids: &[Source], field: StyleField) -> Plan {
    let nodes: BTreeSet<&Id> = ids.iter().filter_map(Source::node).collect();
    let edges: BTreeSet<&Id> = ids.iter().filter_map(Source::edge).collect();
    let mut changes = Vec::new();
    for id in nodes {
        let Some(node) = board.nodes.get(id) else {
            continue;
        };
        if let Some(node) = styled_node(node, field) {
            changes.push(Change::PutNode(id.clone(), Box::new(node)));
        }
    }
    for id in edges {
        let Some(edge) = board.edges.get(id) else {
            continue;
        };
        if let Some(edge) = styled_edge(edge, field) {
            changes.push(Change::PutEdge(id.clone(), Box::new(edge)));
        }
    }
    Plan {
        changes,
        ..Plan::default()
    }
}

/// The stroke the outline command reads and sets on `element`: a shape's
/// outline or a connector's line. `None` for an element without one.
pub fn stroke_of<'b>(board: &'b Board, element: &Source) -> Option<&'b Named<Dash>> {
    match element {
        Source::Node(id) => board
            .nodes
            .get(id)
            .filter(|node| kind(node) == Some(NodeKind::Shape))
            .map(|node| &node.style.stroke),
        Source::Edge(id) => board.edges.get(id).map(|edge| &edge.style.stroke),
    }
}

fn kind(node: &Node) -> Option<NodeKind> {
    node.kind.as_ref().and_then(Named::known)
}

/// `node` with `field` set, or `None` when it has no such field or holds
/// the value already.
fn styled_node(node: &Node, field: StyleField) -> Option<Node> {
    let kind = kind(node)?;
    let mut node = node.clone();
    let changed = match field {
        StyleField::Color(token) => replace(&mut node.style.color, Color::Token(token)),
        StyleField::Fill(fill) if kind == NodeKind::Shape => {
            replace(&mut node.style.fill, Named::Known(fill))
        }
        StyleField::Stroke(dash) if kind == NodeKind::Shape => {
            replace(&mut node.style.stroke, Named::Known(dash))
        }
        StyleField::Fill(_) | StyleField::Stroke(_) => false,
    };
    changed.then_some(node)
}

/// `edge` with `field` set, or `None` when it holds the value already. A
/// connector has no fill.
fn styled_edge(edge: &Edge, field: StyleField) -> Option<Edge> {
    let mut edge = edge.clone();
    let changed = match field {
        StyleField::Color(token) => replace(&mut edge.style.color, Color::Token(token)),
        StyleField::Stroke(dash) => replace(&mut edge.style.stroke, Named::Known(dash)),
        StyleField::Fill(_) => false,
    };
    changed.then_some(edge)
}

/// Puts `value` in `slot`, saying whether that changed it.
fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    let changed = *slot != value;
    *slot = value;
    changed
}

#[cfg(test)]
mod tests {
    use super::super::tests::{edges, line_diff, nodes, Listed};
    use super::super::{plan, Intent};
    use super::*;
    use crate::{parse, write, Fill, Token};

    /// A shape with unknown content and a literal colour, a text node, a
    /// placeholder of an unknown kind, a shape whose colour and fill are
    /// values this build does not know, and a connector that shares an id
    /// with a node.
    const STYLED: &str = r##"schema = 1

[node.a]
text = "Shape"
kind = "shape"
at = [0, 0]
ext.tool.note = "kept"
size = [120, 60]
style.color = "#AbCdEf"
z = "V"

[node.b]
text = "Words"
kind = "text"
at = [0, 120]
size = [120, 20]
z = "W"

[node.c]
kind = "future"
at = [200, 0]
size = [120, 60]
z = "X"

[node.d]
text = "Odd"
kind = "shape"
at = [200, 120]
size = [120, 60]
style.color = "teal"
style.fill = "gradient"
style.stroke = "dotted"
z = "Y"

[edge.a]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
mystery = 1
z = "Z"
"##;

    fn set(ids: Vec<Source>, field: StyleField) -> Intent {
        Intent::SetStyle { ids, field }
    }

    fn both(names: &[&str]) -> Vec<Source> {
        let mut picked = nodes(names);
        picked.extend(edges(names));
        picked
    }

    /// The lines of the canonical file the intent changes.
    #[track_caller]
    fn lines(intent: &Intent) -> Vec<String> {
        let mut board = parse(STYLED).unwrap();
        let written = write(&board);
        let plan = plan(&board, intent, &mut Listed(Vec::new()));
        board.apply(&plan.changes);
        let after = write(&board);
        assert_eq!(write(&parse(&after).unwrap()), after, "round trip");
        line_diff(&written, &after)
    }

    #[test]
    fn a_colour_is_one_line_on_each_node_and_connector_that_draws_it() {
        // The literal colour, read case-insensitively and written in lower
        // case, gives way to the token; the text node and the connector
        // gain a line; the placeholder is left alone.
        assert_eq!(
            lines(&set(both(&["a", "b", "c"]), StyleField::Color(Token::Red))),
            [
                "-style.color = \"#abcdef\"",
                "+style.color = \"red\"",
                "+style.color = \"red\"",
                "+style.color = \"red\"",
            ]
        );
        // An unknown colour on a node it names is replaced; the unknown
        // fill and outline beside it stay.
        assert_eq!(
            lines(&set(nodes(&["d"]), StyleField::Color(Token::Blue))),
            ["-style.color = \"teal\"", "+style.color = \"blue\""]
        );
        // Neutral is the default, so it is written by removing the line.
        assert_eq!(
            lines(&set(nodes(&["a"]), StyleField::Color(Token::Neutral))),
            ["-style.color = \"#abcdef\""]
        );
    }

    #[test]
    fn fill_and_outline_are_a_shapes_and_the_outline_a_connectors() {
        assert_eq!(
            lines(&set(both(&["a", "b", "c"]), StyleField::Fill(Fill::Solid))),
            ["+style.fill = \"solid\""]
        );
        assert_eq!(
            lines(&set(
                both(&["a", "b", "c"]),
                StyleField::Stroke(Dash::Dashed)
            )),
            ["+style.stroke = \"dashed\"", "+style.stroke = \"dashed\""]
        );
        // A node and a connector sharing an id are told apart by kind.
        assert_eq!(
            lines(&set(edges(&["a"]), StyleField::Stroke(Dash::Dashed))),
            ["+style.stroke = \"dashed\""]
        );
        assert_eq!(
            lines(&set(nodes(&["d"]), StyleField::Fill(Fill::None))),
            ["-style.fill = \"gradient\"", "+style.fill = \"none\""]
        );
    }

    #[test]
    fn a_value_already_held_or_nothing_that_takes_it_plans_nothing() {
        let board = parse(STYLED).unwrap();
        let empty = |intent: Intent| {
            let plan = plan(&board, &intent, &mut Listed(Vec::new()));
            assert!(plan.is_empty(), "{intent:?}: {plan:?}");
        };
        empty(set(Vec::new(), StyleField::Color(Token::Red)));
        empty(set(
            both(&["b", "c", "gone"]),
            StyleField::Color(Token::Neutral),
        ));
        empty(set(both(&["b", "c"]), StyleField::Fill(Fill::Solid)));
        empty(set(edges(&["a"]), StyleField::Fill(Fill::None)));
        empty(set(nodes(&["a"]), StyleField::Fill(Fill::Tint)));
        empty(set(both(&["a"]), StyleField::Stroke(Dash::Solid)));
        empty(set(nodes(&["b", "c"]), StyleField::Stroke(Dash::Dashed)));
        // A read-only board plans nothing, with a notice.
        let future = parse("schema = 2\n").unwrap();
        let refused = plan(
            &future,
            &set(nodes(&["a"]), StyleField::Color(Token::Red)),
            &mut Listed(Vec::new()),
        );
        assert!(refused.is_empty() && refused.notice.is_some());
    }

    #[test]
    fn the_outline_is_read_from_shapes_and_connectors_only() {
        let board = parse(STYLED).unwrap();
        let id = |name| Id::new(name).unwrap();
        let solid = Named::Known(Dash::Solid);
        assert_eq!(stroke_of(&board, &Source::Node(id("a"))), Some(&solid));
        assert_eq!(stroke_of(&board, &Source::Edge(id("a"))), Some(&solid));
        assert_eq!(
            stroke_of(&board, &Source::Node(id("d"))),
            Some(&Named::Other("dotted".to_owned()))
        );
        for name in ["b", "c", "gone"] {
            assert_eq!(stroke_of(&board, &Source::Node(id(name))), None, "{name}");
        }
    }
}
