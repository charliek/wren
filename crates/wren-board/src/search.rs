//! The words of a board that search finds: its labels, never its TOML.

use crate::{placements, Board};

/// The board's labels, one per line: node labels in reading order (top
/// then left by absolute position, ties by id), then connector labels in
/// id order, each connector's in its own order. A label keeps its line
/// breaks; a label of only white space is left out. Kind names, ids and
/// keys are not text a person wrote on the board, so none of them is
/// included.
pub fn search_text(board: &Board) -> String {
    let placed = placements(board);
    let mut nodes: Vec<([i64; 2], &str, &str)> = board
        .nodes
        .iter()
        .map(|(id, node)| {
            let [x, y] = placed.get(id).map_or([0, 0], |p| p.at);
            ([y, x], id.as_str(), node.text.as_str())
        })
        .collect();
    nodes.sort_unstable();
    let edges = board
        .edges
        .values()
        .flat_map(|edge| edge.labels.iter().map(|label| label.text.as_str()));
    let labels: Vec<&str> = nodes
        .into_iter()
        .map(|(_, _, text)| text)
        .chain(edges)
        .filter(|text| !text.trim().is_empty())
        .collect();
    labels.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fixtures, parse};

    fn text_of(source: &str) -> String {
        search_text(&parse(source).unwrap_or_else(|e| panic!("{e}\n{source}")))
    }

    /// Nodes at their absolute corners (through parents), then connectors.
    #[test]
    fn the_order_and_content_table() {
        let board = r#"schema = 1
[node.b]
text = "Second, to the right"
kind = "shape"
at = [100, 0]
size = [10, 10]
[node.a]
text = "First, top left"
kind = "frame"
at = [0, 0]
size = [10, 10]
[node.child]
text = "Child, placed through its parent"
kind = "shape"
at = [0, 5]
parent = "b"
size = [10, 10]
[node.low]
text = "Lowest\nhas two lines"
kind = "text"
at = [-500, 50]
size = [10, 10]
[node.tie1]
text = "Tie, by id first"
kind = "shape"
at = [0, 20]
size = [10, 10]
[node.tie2]
text = "Tie, by id second"
kind = "shape"
at = [0, 20]
size = [10, 10]
[node.blank]
text = "  \n "
kind = "shape"
at = [0, 1]
size = [10, 10]
[node.none]
kind = "shape"
at = [0, 2]
size = [10, 10]
[edge.z]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
labels = [{ text = "edge z" }]
[edge.y]
kind = "edge"
from = { at = [0, 0] }
to = { node = "gone" }
labels = [{ text = "edge y, first", at = 900 }, { text = "" }, { text = "edge y, second", at = 100 }]
"#;
        let want = [
            "First, top left",
            "Second, to the right",
            "Child, placed through its parent",
            "Tie, by id first",
            "Tie, by id second",
            "Lowest\nhas two lines",
            "edge y, first",
            "edge y, second",
            "edge z",
        ];
        assert_eq!(text_of(board), want.join("\n"));
    }

    #[test]
    fn syntax_and_names_are_not_searchable() {
        let text = text_of(fixtures::CANONICAL[0].1);
        for word in [
            "schema", "kind", "shape", "node", "edge", "z =", "V0", "rounded",
        ] {
            assert!(!text.contains(word), "{word} in {text:?}");
        }
        assert_eq!(text_of(crate::EMPTY), "");
    }

    #[test]
    fn the_checkout_flow_reads_top_to_bottom() {
        let (_, flow) = fixtures::CANONICAL
            .iter()
            .find(|(name, _)| *name == "Checkout flow")
            .unwrap();
        let text = text_of(flow);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[..4],
            ["Start", "Receive order", "Valid?", "Reject order"]
        );
        assert_eq!(lines.iter().filter(|l| **l == "No").count(), 3);
        assert!(lines.contains(&"Send receipt"));
    }
}
