//! Deleting elements (§5.10 rule 1).

use std::collections::{BTreeMap, BTreeSet};

use super::{Change, Plan};
use crate::{Board, End, Id};

/// Removes the elements named by `ids`, every node whose `parent` chain
/// reaches a removed node, and every connector attached at either end to a
/// removed node. An id naming both a node and a connector removes both.
///
/// Descendants are followed by the stored `parent`, not only the drawn
/// one, so a node in a parent cycle with a removed node, or nested too deep
/// under one, goes with it: none is left naming a node that is gone, which
/// would be a new integrity issue the user did not make.
pub(super) fn delete(board: &Board, ids: &[Id]) -> Plan {
    let mut children: BTreeMap<&str, Vec<&Id>> = BTreeMap::new();
    for (id, node) in &board.nodes {
        if let Some(parent) = &node.parent {
            children.entry(parent.as_str()).or_default().push(id);
        }
    }
    let mut nodes: BTreeSet<&Id> = BTreeSet::new();
    let mut queue: Vec<&Id> = ids
        .iter()
        .filter_map(|id| board.nodes.get_key_value(id).map(|(id, _)| id))
        .collect();
    while let Some(id) = queue.pop() {
        if nodes.insert(id) {
            queue.extend(children.get(id.as_str()).into_iter().flatten());
        }
    }
    let chosen: BTreeSet<&Id> = ids.iter().collect();
    let names: BTreeSet<&str> = nodes.iter().map(|id| id.as_str()).collect();
    let attached = |end: &End| match end {
        End::Node { node, .. } => names.contains(node.as_str()),
        End::Free { .. } => false,
    };
    let edges: Vec<&Id> = board
        .edges
        .iter()
        .filter(|(id, edge)| chosen.contains(id) || attached(&edge.from) || attached(&edge.to))
        .map(|(id, _)| id)
        .collect();
    if nodes.is_empty() && edges.is_empty() {
        return Plan::default();
    }
    let changes = edges
        .into_iter()
        .map(|id| Change::RemoveEdge(id.clone()))
        .chain(nodes.into_iter().map(|id| Change::RemoveNode(id.clone())))
        .collect();
    Plan {
        changes,
        select: Some(Vec::new()),
        ..Plan::default()
    }
}
