//! The intents that make or change single nodes: add, relabel, move,
//! resize and reshape (§5.5, §5.8, §5.9).

use std::collections::BTreeSet;

use super::{
    connector, dangling_names, on_top, saturate, trim_label, Change, Edges, Endpoint, Plan,
};
use crate::ids::{fresh, Ids};
use crate::scene::Source;
use crate::{placements, Board, End, Extra, Id, Named, Node, NodeKind, NodeStyle, Shape, GRID};

/// A new node on top of the root scope, with an empty label, the neutral
/// colour and the default fill, and with `from` a connector from there to
/// it (§5.5 rule 6, §5.12 rule 1). A text node has no outline, so it keeps
/// the default shape whatever `shape` says. A connector from a node that is
/// not there makes nothing: the gesture it ends belonged to that node.
pub(super) fn add(
    board: &Board,
    kind: NodeKind,
    shape: Shape,
    at: [i32; 2],
    size: [i32; 2],
    from: Option<&Endpoint>,
    ids: &mut dyn Ids,
) -> Plan {
    if let Some(Endpoint::Node { node, .. }) = from {
        if !board.nodes.contains_key(node) {
            return Plan::default();
        }
    }
    let mut taken = dangling_names(board);
    let id = fresh(board, &taken, ids);
    taken.insert(id.clone());
    let (keys, mut changes) = on_top(board, 1 + usize::from(from.is_some()));
    let mut keys = keys.into_iter();
    let node = Node {
        text: String::new(),
        kind: Some(Named::Known(kind)),
        at,
        parent: None,
        shape: Named::Known(match kind {
            NodeKind::Shape => shape,
            NodeKind::Text => Shape::Rect,
        }),
        size: size.map(|length| length.max(1)),
        style: NodeStyle::default(),
        z: keys.next(),
        extra: Extra::new(),
    };
    changes.push(Change::PutNode(id.clone(), Box::new(node)));
    if let Some(from) = from {
        let edge = fresh(board, &taken, ids);
        let to = Endpoint::Node {
            node: id.clone(),
            side: None,
        };
        changes.push(Change::PutEdge(
            edge,
            Box::new(connector(from, &to, keys.next())),
        ));
    }
    Plan {
        changes,
        select: Some(vec![id.clone()]),
        edit: Some(id),
        notice: None,
    }
}

/// A node's label, trimmed of trailing blank lines, and its size as the
/// label editor left it (§5.9 rules 4 and 7). An empty label is kept: the
/// label editor deletes an empty text node when it commits (§5.9 rule 8),
/// not on every keystroke that empties it on the way.
pub(super) fn set_text(board: &Board, id: &Id, text: &str, size: [i32; 2]) -> Plan {
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

/// Moves the nodes and the free ends of the connectors among `ids` (§5.8
/// rules 2 and 4). A node whose drawn ancestor is also among them moves
/// with it and not again; coordinates stop at the `i32` range.
pub(super) fn moved(board: &Board, ids: &[Source], by: [i32; 2]) -> Plan {
    let chosen: BTreeSet<&Id> = ids.iter().filter_map(Source::node).collect();
    let edges: BTreeSet<&Id> = ids.iter().filter_map(Source::edge).collect();
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
        }
    }
    for &id in &edges {
        if let Some(edge) = board.edges.get(id) {
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

/// Moves the edges of each node's box (§5.8 rules 5 and 6). Where that
/// would leave the box narrower or shorter than a cell, the moved edge stops
/// one cell from the one that stayed; a box already smaller than a cell is
/// never made smaller, nor grown by this rule.
pub(super) fn resize(board: &Board, ids: &[Id], edges: Edges) -> Plan {
    let chosen: BTreeSet<&Id> = ids.iter().collect();
    let mut changes = Vec::new();
    for id in chosen {
        let Some(node) = board.nodes.get(id) else {
            continue;
        };
        let (x, w) = span(node.at[0], node.size[0], edges.left, edges.right);
        let (y, h) = span(node.at[1], node.size[1], edges.top, edges.bottom);
        if [x, y] == node.at && [w, h] == node.size {
            continue;
        }
        let node = Node {
            at: [x, y],
            size: [w, h],
            ..node.clone()
        };
        changes.push(Change::PutNode(id.clone(), Box::new(node)));
    }
    Plan {
        changes,
        ..Plan::default()
    }
}

/// One axis of a resize: the new start and length.
fn span(at: i32, size: i32, start: i32, end: i32) -> (i32, i32) {
    let (at, size) = (i64::from(at), i64::from(size));
    let mut low = at + i64::from(start);
    let mut high = at + size + i64::from(end);
    let least = size.clamp(1, i64::from(GRID));
    if high - low < least {
        if start != 0 && end == 0 {
            low = high - least;
        } else {
            high = low + least;
        }
    }
    let low = saturate(low);
    (low, saturate(high - i64::from(low)).max(1))
}

/// Sets the shape of each shape node among `ids`, keeping its box and
/// label (§5.5 rule 9). A text node has no outline and a node of another
/// kind draws as a placeholder whatever its shape, so neither is changed.
pub(super) fn set_shape(board: &Board, ids: &[Id], shape: Shape) -> Plan {
    let chosen: BTreeSet<&Id> = ids.iter().collect();
    let mut changes = Vec::new();
    for id in chosen {
        let Some(node) = board.nodes.get(id) else {
            continue;
        };
        let is_shape = node.kind.as_ref().and_then(Named::known) == Some(NodeKind::Shape);
        if !is_shape || node.shape == Named::Known(shape) {
            continue;
        }
        let node = Node {
            shape: Named::Known(shape),
            ..node.clone()
        };
        changes.push(Change::PutNode(id.clone(), Box::new(node)));
    }
    Plan {
        changes,
        ..Plan::default()
    }
}
