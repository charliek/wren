//! Deleting elements (§5.10 rule 1).

use std::collections::{BTreeMap, BTreeSet};

use super::{Change, Plan};
use crate::{Board, End, Id};

/// Removes the elements named by `ids`, their [`family`], and every
/// connector attached at either end to a removed node. An id naming both a
/// node and a connector removes both.
pub(super) fn delete(board: &Board, ids: &[Id]) -> Plan {
    let nodes = family(board, ids);
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

/// The nodes among `ids` and every node whose stored `parent` chain
/// reaches one of them: the descendants that delete removes and copy
/// carries (§5.10 rules 1 and 2), so a cut pastes back all it removed.
///
/// The stored `parent` is followed, not only the drawn one, so a node in a
/// parent cycle with one of them, or nested past level 32 under one, is in
/// the family. Leaving it behind would give it a parent naming nothing, a
/// new issue the user did not make, and could move a node drawn under it:
/// in a cycle of three, deleting one makes the next a root and draws the
/// last inside it.
pub(super) fn family<'a>(board: &'a Board, ids: &[Id]) -> BTreeSet<&'a Id> {
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
    nodes
}
