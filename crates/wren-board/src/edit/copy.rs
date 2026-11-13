//! Copies of a fragment: duplicate (§5.10 rules 2 and 3), and the part of
//! paste that puts a parsed fragment into a board (§5.10 rules 7 and 8).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{dangling_names, on_top, saturate, Change, Plan};
use crate::ids::{fresh, Ids};
use crate::z::{self, Element};
use crate::{placements, Board, Edge, End, Id, Node, Placed};

/// The elements a selection carries into a copy (§5.10 rule 2): its
/// nodes, their descendants (the [`super::delete::family`] that delete
/// removes), and every connector whose ends are
/// each one of those nodes or a free point, provided one end is such a node
/// or the connector itself is selected. A selection with no node has none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fragment {
    pub nodes: BTreeSet<Id>,
    pub edges: BTreeSet<Id>,
}

impl Fragment {
    fn holds(&self, element: Element<'_>) -> bool {
        match element {
            Element::Node(id) => self.nodes.contains(id),
            Element::Edge(id) => self.edges.contains(id),
        }
    }

    /// Whether every end of `edge` is a node of the fragment or a free
    /// point.
    pub fn carries(&self, edge: &Edge) -> bool {
        [&edge.from, &edge.to].into_iter().all(|end| match end {
            End::Node { node, .. } => self.nodes.contains(node.as_str()),
            End::Free { .. } => true,
        })
    }
}

/// The fragment of the selection `ids` on `board`.
pub(crate) fn fragment(board: &Board, ids: &[Id]) -> Fragment {
    let mut part = Fragment {
        nodes: super::delete::family(board, ids)
            .into_iter()
            .cloned()
            .collect(),
        ..Fragment::default()
    };
    if part.nodes.is_empty() {
        return part;
    }
    let chosen: BTreeSet<&Id> = ids.iter().collect();
    let edges: BTreeSet<Id> = board
        .edges
        .iter()
        .filter(|(id, edge)| {
            let touches = [&edge.from, &edge.to].into_iter().any(
                |end| matches!(end, End::Node { node, .. } if part.nodes.contains(node.as_str())),
            );
            part.carries(edge) && (touches || chosen.contains(id))
        })
        .map(|(id, _)| id.clone())
        .collect();
    part.edges = edges;
    part
}

/// Copies of the fragment of `ids`, offset by `by`, on top in the same
/// relative order, selected (§5.10 rule 3; a duplicate-drag, §5.8 rule 3).
pub(super) fn duplicate(board: &Board, ids: &[Id], by: [i32; 2], source: &mut dyn Ids) -> Plan {
    let part = fragment(board, ids);
    if part.nodes.is_empty() {
        return Plan::default();
    }
    copies(board, board, &part, by.map(i64::from), source)
}

/// The fragment `part` of `source` laid out for a copy: its elements in
/// `source`'s drawing order, the parent each node keeps (the one it is drawn
/// inside, when that is in the fragment too), and every node's place.
pub(crate) struct Layout<'a> {
    pub order: Vec<Element<'a>>,
    pub parents: HashMap<&'a Id, &'a Id>,
    pub placed: BTreeMap<&'a Id, Placed<'a>>,
}

impl<'a> Layout<'a> {
    pub fn of(source: &'a Board, part: &Fragment) -> Self {
        let placed = placements(source);
        let order: Vec<Element<'a>> = z::drawn(source)
            .into_iter()
            .filter(|element| part.holds(*element))
            .collect();
        let parents = order
            .iter()
            .filter_map(|element| match *element {
                Element::Node(id) => {
                    let parent = placed.get(id)?.parent?;
                    part.nodes.contains(parent).then_some((id, parent))
                }
                Element::Edge(_) => None,
            })
            .collect();
        Self {
            order,
            parents,
            placed,
        }
    }

    /// Whether `element` sits at the top level of the copy.
    pub fn is_root(&self, element: Element<'_>) -> bool {
        match element {
            Element::Node(id) => !self.parents.contains_key(id),
            Element::Edge(_) => true,
        }
    }

    /// The stacking keys of the copy: `root` gives the keys of the
    /// top-level elements, and each copied parent's own elements are keyed
    /// afresh in their order.
    pub fn keys(&self, root: Vec<String>) -> HashMap<Element<'a>, String> {
        let mut keys: HashMap<Element<'a>, String> = HashMap::new();
        let tops = self.order.iter().filter(|element| self.is_root(**element));
        keys.extend(tops.copied().zip(root));
        let mut scopes: BTreeMap<&Id, Vec<Element<'a>>> = BTreeMap::new();
        for element in &self.order {
            if let Element::Node(id) = element {
                if let Some(parent) = self.parents.get(id) {
                    scopes.entry(parent).or_default().push(*element);
                }
            }
        }
        for scope in scopes.into_values() {
            let fresh = z::insert(&[], 0, scope.len()).keys;
            keys.extend(scope.into_iter().zip(fresh));
        }
        keys
    }

    /// The node `id` of the copy before its ids are remapped: the parent it
    /// keeps, or none and its absolute corner moved by `by`.
    pub fn node(&self, source: &Board, id: &Id, by: [i64; 2]) -> Node {
        let node = &source.nodes[id];
        match self.parents.get(id) {
            Some(parent) => Node {
                parent: Some(parent.as_str().to_owned()),
                ..node.clone()
            },
            None => {
                let at = self.placed.get(id).map_or([0, 0], |place| place.at);
                Node {
                    parent: None,
                    at: [saturate(at[0] + by[0]), saturate(at[1] + by[1])],
                    ..node.clone()
                }
            }
        }
    }
}

/// Copies of the fragment `part` of `source` put into `target` under new
/// ids: every reference inside the fragment renamed, top-level nodes and
/// free ends moved by `by`, the top-level elements on top of `target`'s
/// root scope in their relative order, and all of them selected, nodes
/// first. A node is copied inside the parent it is drawn in when that is in
/// the fragment, and otherwise at the top level at its absolute place, so
/// no copy has a missing parent or a cycle.
pub(crate) fn copies(
    target: &Board,
    source: &Board,
    part: &Fragment,
    by: [i64; 2],
    ids: &mut dyn Ids,
) -> Plan {
    let layout = Layout::of(source, part);
    let mut taken = dangling_names(target);
    let mut names: HashMap<Element<'_>, Id> = HashMap::new();
    for element in &layout.order {
        let id = fresh(target, &taken, ids);
        taken.insert(id.clone());
        names.insert(*element, id);
    }
    let node_name = |id: &str| {
        let (id, _) = source.nodes.get_key_value(id)?;
        names.get(&Element::Node(id))
    };
    let roots = layout
        .order
        .iter()
        .filter(|element| layout.is_root(**element))
        .count();
    let (root_keys, mut changes) = on_top(target, roots);
    let mut keys = layout.keys(root_keys);
    let (mut select, mut connectors, mut edges) = (Vec::new(), Vec::new(), Vec::new());
    for element in &layout.order {
        let new = names[element].clone();
        let z = keys.remove(element);
        match *element {
            Element::Node(id) => {
                let mut node = layout.node(source, id, by);
                node.parent = node
                    .parent
                    .as_deref()
                    .and_then(node_name)
                    .map(|name| name.as_str().to_owned());
                node.z = z;
                changes.push(Change::PutNode(new.clone(), Box::new(node)));
                select.push(new);
            }
            Element::Edge(id) => {
                let mut edge = source.edges[id].clone();
                let mut whole = true;
                for end in [&mut edge.from, &mut edge.to] {
                    match end {
                        End::Node { node, .. } => match node_name(node) {
                            Some(name) => *node = name.as_str().to_owned(),
                            None => whole = false,
                        },
                        End::Free { at, .. } => {
                            *at = [
                                saturate(i64::from(at[0]) + by[0]),
                                saturate(i64::from(at[1]) + by[1]),
                            ];
                        }
                    }
                }
                if whole {
                    edge.z = z;
                    edges.push(Change::PutEdge(new.clone(), Box::new(edge)));
                    connectors.push(new);
                }
            }
        }
    }
    changes.extend(edges);
    // Nodes come first, so the cursor can be the first pasted node.
    select.extend(connectors);
    Plan {
        changes,
        select: Some(select),
        ..Plan::default()
    }
}
