//! What a loaded board may have wrong without failing to load, and whether
//! this build may edit it.
//!
//! Nothing here repairs or drops anything. A flagged board opens, draws as
//! the plan's §4.12 table says, and keeps its content byte for byte through
//! a save; [`placements`] is how it draws its parent tree.

use std::collections::BTreeMap;
use std::fmt;

use crate::scene::Source;
use crate::{Board, End, Id, Node};

/// The deepest nesting drawn inside parents: a node more levels down than
/// this, counting a root as level 1, is laid out as a root.
pub const MAX_DEPTH: u32 = 32;

/// The schema this build reads and writes.
pub const SCHEMA: i32 = 1;

/// The `requires` features this build knows. Ring 0 defines none.
const FEATURES: &[&str] = &[];

/// Whether this build may change a board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Editable,
    /// The board draws, pans, zooms, selects, copies and exports, and is
    /// never written.
    ReadOnly(Reason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A schema other than the one this build writes.
    Schema(i32),
    /// The features in `requires` this build does not know, sorted.
    Requires(Vec<String>),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(schema) => write!(f, "written for board schema {schema}"),
            Self::Requires(features) => {
                write!(f, "needs features this build does not have: ")?;
                f.write_str(&features.join(", "))
            }
        }
    }
}

/// Whether the board may be edited: read-only when its schema is not this
/// build's or it requires a feature this build does not know.
pub fn access(board: &Board) -> Access {
    if board.schema != SCHEMA {
        return Access::ReadOnly(Reason::Schema(board.schema));
    }
    let mut unknown: Vec<String> = board
        .requires
        .iter()
        .filter(|feature| !FEATURES.contains(&feature.as_str()))
        .cloned()
        .collect();
    if unknown.is_empty() {
        return Access::Editable;
    }
    unknown.sort_unstable();
    unknown.dedup();
    Access::ReadOnly(Reason::Requires(unknown))
}

/// Something wrong with one element that still lets the board load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub element: Source,
    pub kind: IssueKind,
}

impl Issue {
    pub fn id(&self) -> &Id {
        match &self.element {
            Source::Node(id) | Source::Edge(id) => id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    /// A connector end names no node.
    DanglingEdge {
        from: bool,
        to: bool,
    },
    /// `parent` names no node; the node is laid out as a root.
    MissingParent,
    /// The node is in a parent cycle, itself included; it is laid out as a
    /// root.
    ParentCycle,
    /// The node is nested more than [`MAX_DEPTH`] levels down; it is laid
    /// out as a root.
    TooDeep,
    /// A node and a connector share the id. Both are drawn.
    SharedId,
    MissingKind,
    MissingZ,
    /// A `z` that is empty or holds a character outside `[0-9A-Za-z]`.
    InvalidZ,
}

impl IssueKind {
    /// The kind's name, as the test harness reports it.
    pub fn name(self) -> &'static str {
        match self {
            Self::DanglingEdge { .. } => "dangling_edge",
            Self::MissingParent => "missing_parent",
            Self::ParentCycle => "parent_cycle",
            Self::TooDeep => "too_deep",
            Self::SharedId => "shared_id",
            Self::MissingKind => "missing_kind",
            Self::MissingZ => "missing_z",
            Self::InvalidZ => "invalid_z",
        }
    }
}

/// The board's integrity issues: nodes in id order, then connectors, each
/// element's issues in a fixed order.
pub fn check(board: &Board) -> Vec<Issue> {
    let mut issues = Vec::new();
    let tree = resolve(board);
    for ((id, node), placed) in board.nodes.iter().zip(&tree) {
        let mut flag = |kind| {
            issues.push(Issue {
                element: Source::Node(id.clone()),
                kind,
            })
        };
        if node.kind.is_none() {
            flag(IssueKind::MissingKind);
        }
        if let Some(kind) = z_issue(node.z.as_deref()) {
            flag(kind);
        }
        if let Some(kind) = placed.flag {
            flag(kind);
        }
        if board.edges.contains_key(id) {
            flag(IssueKind::SharedId);
        }
    }
    for (id, edge) in &board.edges {
        let mut flag = |kind| {
            issues.push(Issue {
                element: Source::Edge(id.clone()),
                kind,
            })
        };
        if edge.kind.is_none() {
            flag(IssueKind::MissingKind);
        }
        if let Some(kind) = z_issue(edge.z.as_deref()) {
            flag(kind);
        }
        let dangles = |end: &End| match end {
            End::Node { node, .. } => !board.nodes.contains_key(node.as_str()),
            End::Free { .. } => false,
        };
        let (from, to) = (dangles(&edge.from), dangles(&edge.to));
        if from || to {
            flag(IssueKind::DanglingEdge { from, to });
        }
        if board.nodes.contains_key(id) {
            flag(IssueKind::SharedId);
        }
    }
    issues
}

/// A key longer than 64 characters is an out-of-range value, listed with
/// the limits rather than here.
fn z_issue(z: Option<&str>) -> Option<IssueKind> {
    match z {
        None => Some(IssueKind::MissingZ),
        Some(z) if z.is_empty() || !z.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            Some(IssueKind::InvalidZ)
        }
        Some(_) => None,
    }
}

/// Where a node is drawn once the parent rules are applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed<'a> {
    /// The node it is drawn inside, or `None` when it is laid out as a
    /// root: it has no parent, its parent names no node, it is in a parent
    /// cycle, or it is nested too deep.
    pub parent: Option<&'a Id>,
    /// The absolute top-left corner: the stored `at`, plus the corner of
    /// the parent it is drawn inside.
    pub at: [i64; 2],
}

/// Every node's place in the drawn parent tree. A node's stored `at` is
/// taken as absolute whenever it is laid out as a root, as §4.12 says, so
/// the result never depends on the order of the file.
pub fn placements(board: &Board) -> BTreeMap<&Id, Placed<'_>> {
    let ids: Vec<&Id> = board.nodes.keys().collect();
    board
        .nodes
        .keys()
        .zip(resolve(board))
        .map(|(id, placed)| {
            let parent = placed.parent.and_then(|index| ids.get(index).copied());
            (
                id,
                Placed {
                    parent,
                    at: placed.at,
                },
            )
        })
        .collect()
}

/// One node's resolution, by index into the nodes in id order.
#[derive(Clone, Copy, Debug)]
struct Resolved {
    parent: Option<usize>,
    at: [i64; 2],
    /// The level the node would have under its stored parents, a root
    /// being 1; it keeps counting past [`MAX_DEPTH`].
    depth: u32,
    flag: Option<IssueKind>,
}

/// How a walk up the stored parents ended.
enum Stop {
    /// At a node with no parent.
    Root,
    /// At a node whose parent names no node.
    Missing,
    /// At the node at this position of the walk, met a second time.
    Cycle(usize),
    /// At a node an earlier walk resolved.
    Known(usize),
}

/// Resolves every node, in id order. Each node is walked once, without
/// recursion, so a long chain or a large cycle costs linear time and no
/// stack.
fn resolve(board: &Board) -> Vec<Resolved> {
    let nodes: Vec<(&Id, &Node)> = board.nodes.iter().collect();
    let parent: Vec<Option<Option<usize>>> = nodes
        .iter()
        .map(|(_, node)| {
            node.parent
                .as_deref()
                .map(|name| nodes.binary_search_by(|(id, _)| id.as_str().cmp(name)).ok())
        })
        .collect();
    let own = |index: usize| {
        nodes
            .get(index)
            .map_or([0, 0], |(_, node)| node.at.map(i64::from))
    };
    let mut done: Vec<Option<Resolved>> = vec![None; nodes.len()];
    let mut on_path: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut path = Vec::new();
    for start in 0..nodes.len() {
        if done[start].is_some() {
            continue;
        }
        path.clear();
        let mut current = start;
        let stop = loop {
            if done[current].is_some() {
                break Stop::Known(current);
            }
            if let Some(position) = on_path[current] {
                break Stop::Cycle(position);
            }
            on_path[current] = Some(path.len());
            path.push(current);
            match parent[current] {
                None => break Stop::Root,
                Some(None) => break Stop::Missing,
                Some(Some(next)) => current = next,
            }
        };
        let root = |index: usize, flag| Resolved {
            parent: None,
            at: own(index),
            depth: 1,
            flag,
        };
        let mut below = path.len();
        match stop {
            Stop::Root | Stop::Missing => {
                below -= 1;
                let flag = matches!(stop, Stop::Missing).then_some(IssueKind::MissingParent);
                done[path[below]] = Some(root(path[below], flag));
            }
            Stop::Cycle(position) => {
                below = position;
                for &member in &path[position..] {
                    done[member] = Some(root(member, Some(IssueKind::ParentCycle)));
                }
            }
            Stop::Known(_) => {}
        }
        let mut above = match stop {
            Stop::Known(index) => index,
            _ => path[below],
        };
        for &child in path[..below].iter().rev() {
            let Some(up) = done[above] else {
                break;
            };
            let depth = up.depth.saturating_add(1);
            let at = own(child);
            done[child] = Some(if depth > MAX_DEPTH {
                Resolved {
                    parent: None,
                    at,
                    depth,
                    flag: Some(IssueKind::TooDeep),
                }
            } else {
                Resolved {
                    parent: Some(above),
                    at: [up.at[0] + at[0], up.at[1] + at[1]],
                    depth,
                    flag: None,
                }
            });
            above = child;
        }
    }
    done.into_iter()
        .map(|resolved| {
            resolved.unwrap_or(Resolved {
                parent: None,
                at: [0, 0],
                depth: 1,
                flag: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, Rng};
    use crate::{parse, write};

    /// Reads a canonical board, and asserts it is canonical, so each case
    /// also proves that `write` keeps the flagged content byte for byte.
    #[track_caller]
    fn canonical(text: &str) -> Board {
        let board = parse(text).unwrap_or_else(|error| panic!("{error}\n{text}"));
        assert_eq!(write(&board), text, "the case is not canonical");
        board
    }

    /// A canonical node block. `fields` are the lines between `at` and
    /// `size` in key order (`parent`, then `shape`).
    fn node(id: &str, at: [i32; 2], fields: &str) -> String {
        format!(
            "\n[node.{id}]\nkind = \"shape\"\nat = [{}, {}]\n{fields}size = [120, 60]\nz = \"V\"\n",
            at[0], at[1]
        )
    }

    fn child(id: &str, at: [i32; 2], parent: &str) -> String {
        node(id, at, &format!("parent = \"{parent}\"\n"))
    }

    fn named(issues: &[Issue]) -> Vec<(String, &'static str)> {
        issues
            .iter()
            .map(|issue| (issue.id().to_string(), issue.kind.name()))
            .collect()
    }

    fn pairs(list: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
        list.iter()
            .map(|&(id, kind)| (id.to_owned(), kind))
            .collect()
    }

    /// Each node's drawn parent and absolute corner.
    fn layout(board: &Board) -> Vec<(String, Option<String>, [i64; 2])> {
        placements(board)
            .into_iter()
            .map(|(id, placed)| (id.to_string(), placed.parent.map(Id::to_string), placed.at))
            .collect()
    }

    fn row(id: &str, parent: Option<&str>, at: [i64; 2]) -> (String, Option<String>, [i64; 2]) {
        (id.to_owned(), parent.map(str::to_owned), at)
    }

    #[test]
    fn committed_boards_have_only_their_known_issues() {
        for (name, text) in fixtures::CANONICAL.iter().chain([&fixtures::HAND_WRITTEN]) {
            let board = parse(text).unwrap();
            let expected: &[(&str, &str)] = match *name {
                // An edge with every field shares its id with the node
                // with every field.
                "fields.out" => &[("full", "shared_id"), ("full", "shared_id")],
                "Shapes" => &[("dangling", "dangling_edge")],
                _ => &[],
            };
            assert_eq!(named(&check(&board)), pairs(expected), "{name}");
        }
        let flow = parse(fixtures::CANONICAL[10].1).unwrap();
        assert_eq!(fixtures::CANONICAL[10].0, "Checkout flow");
        assert_eq!(access(&flow), Access::Editable);
        let empty = parse("").unwrap();
        assert_eq!((check(&empty), access(&empty)), (vec![], Access::Editable));
    }

    /// Test 7's read-only rows.
    #[test]
    fn unknown_features_and_newer_schemas_are_read_only() {
        let access_of = |text: &str| access(&parse(text).unwrap());
        assert_eq!(access_of("schema = 1\nrequires = []\n"), Access::Editable);
        assert_eq!(
            access_of("schema = 2\n"),
            Access::ReadOnly(Reason::Schema(2))
        );
        assert_eq!(
            access_of("schema = 2\nrequires = [\"frames\"]\n"),
            Access::ReadOnly(Reason::Schema(2))
        );
        assert_eq!(
            access_of("schema = 1\nrequires = [\"zz\", \"frames\", \"Not A Feature!\", \"zz\"]\n"),
            Access::ReadOnly(Reason::Requires(vec![
                "Not A Feature!".into(),
                "frames".into(),
                "zz".into()
            ]))
        );
        let future = parse(fixtures::CANONICAL[13].1).unwrap();
        assert_eq!(fixtures::CANONICAL[13].0, "Future");
        assert_eq!(
            access(&future),
            Access::ReadOnly(Reason::Requires(vec!["frames".into()]))
        );
        assert_eq!(check(&future), vec![]);
        assert_eq!(
            Reason::Requires(vec!["a".into(), "b".into()]).to_string(),
            "needs features this build does not have: a, b"
        );
        assert_eq!(Reason::Schema(3).to_string(), "written for board schema 3");
    }

    #[test]
    fn dangling_ends_are_flagged_per_connector() {
        let mut text = format!("schema = 1\n{}", node("a", [0, 0], ""));
        for (id, from, to) in [
            ("both", "{ node = \"x\" }", "{ node = \"y\" }"),
            ("free", "{ at = [0, 0] }", "{ node = \"gone\" }"),
            ("ok", "{ node = \"a\" }", "{ at = [5, 5] }"),
            ("one", "{ node = \"a\" }", "{ node = \"not an id!\" }"),
        ] {
            text.push_str(&format!(
                "\n[edge.{id}]\nkind = \"edge\"\nfrom = {from}\nto = {to}\nz = \"W\"\n"
            ));
        }
        let issues = check(&canonical(&text));
        let kinds: Vec<(String, IssueKind)> = issues
            .iter()
            .map(|issue| (issue.id().to_string(), issue.kind))
            .collect();
        let dangling = |from, to| IssueKind::DanglingEdge { from, to };
        assert_eq!(
            kinds,
            [
                ("both".to_owned(), dangling(true, true)),
                ("free".to_owned(), dangling(false, true)),
                ("one".to_owned(), dangling(false, true)),
            ]
        );
        assert!(issues
            .iter()
            .all(|issue| matches!(issue.element, Source::Edge(_))));
    }

    #[test]
    fn a_missing_parent_leaves_a_root_at_its_stored_corner() {
        let text = format!(
            "schema = 1\n{}{}",
            child("a", [10, 20], "gone"),
            child("b", [1, 2], "a")
        );
        let board = canonical(&text);
        assert_eq!(named(&check(&board)), pairs(&[("a", "missing_parent")]));
        assert_eq!(
            layout(&board),
            [row("a", None, [10, 20]), row("b", Some("a"), [11, 22])]
        );
    }

    #[test]
    fn every_node_of_a_parent_cycle_is_a_root() {
        // A self-parent, a cycle of two, a cycle of three, and a node
        // hanging from a cycle, which is drawn inside its laid-out parent.
        let text = format!(
            "schema = 1\n{}{}{}{}{}{}{}",
            child("c1", [100, 0], "c2"),
            child("c2", [200, 0], "c3"),
            child("c3", [300, 0], "c1"),
            child("p", [7, 7], "c2"),
            child("s", [5, 6], "s"),
            child("t1", [10, 0], "t2"),
            child("t2", [20, 0], "t1"),
        );
        let board = canonical(&text);
        assert_eq!(
            named(&check(&board)),
            pairs(&[
                ("c1", "parent_cycle"),
                ("c2", "parent_cycle"),
                ("c3", "parent_cycle"),
                ("s", "parent_cycle"),
                ("t1", "parent_cycle"),
                ("t2", "parent_cycle"),
            ])
        );
        assert_eq!(
            layout(&board),
            [
                row("c1", None, [100, 0]),
                row("c2", None, [200, 0]),
                row("c3", None, [300, 0]),
                row("p", Some("c2"), [207, 7]),
                row("s", None, [5, 6]),
                row("t1", None, [10, 0]),
                row("t2", None, [20, 0]),
            ]
        );
    }

    /// A chain of `levels` nodes `n01` (the root) to `nNN`, each 10 right
    /// of its parent.
    fn chain(levels: usize) -> String {
        let mut text = format!("schema = 1\n{}", node("n01", [10, 0], ""));
        for level in 2..=levels {
            text.push_str(&child(
                &format!("n{level:02}"),
                [10, 1],
                &format!("n{:02}", level - 1),
            ));
        }
        text
    }

    #[test]
    fn nesting_past_32_levels_is_laid_out_from_the_root() {
        let deepest = MAX_DEPTH as usize;
        let board = canonical(&chain(deepest));
        assert_eq!(check(&board), vec![]);
        let last = placements(&board)[&Id::new(&format!("n{deepest:02}")).unwrap()];
        assert_eq!(last.at, [10 * deepest as i64, deepest as i64 - 1]);

        let board = canonical(&chain(deepest + 2));
        assert_eq!(
            named(&check(&board)),
            pairs(&[("n33", "too_deep"), ("n34", "too_deep")])
        );
        let places = layout(&board);
        assert_eq!(places[31], row("n32", Some("n31"), [320, 31]));
        assert_eq!(places[32], row("n33", None, [10, 1]));
        assert_eq!(places[33], row("n34", None, [10, 1]));
    }

    #[test]
    fn a_parent_of_an_unknown_kind_still_places_its_children() {
        let text = concat!(
            "schema = 1\n\n",
            "[node.frame]\nkind = \"frame\"\nat = [100, 200]\nsize = [400, 300]\nz = \"V\"\n\n",
            "[node.inner]\nkind = \"mystery\"\nat = [10, 20]\nparent = \"frame\"\nsize = [200, 100]\nz = \"V\"\n\n",
            "[node.leaf]\nkind = \"shape\"\nat = [-5, 5]\nparent = \"inner\"\nsize = [10, 10]\nz = \"V\"\n",
        );
        let board = canonical(text);
        assert_eq!(check(&board), vec![]);
        assert_eq!(
            layout(&board),
            [
                row("frame", None, [100, 200]),
                row("inner", Some("frame"), [110, 220]),
                row("leaf", Some("inner"), [105, 225]),
            ]
        );
    }

    #[test]
    fn parent_sums_do_not_overflow() {
        let mut text = format!("schema = 1\n{}", node("n01", [i32::MAX, i32::MIN], ""));
        for level in 2..=MAX_DEPTH as usize {
            text.push_str(&child(
                &format!("n{level:02}"),
                [i32::MAX, i32::MIN],
                &format!("n{:02}", level - 1),
            ));
        }
        let board = canonical(&text);
        let last = placements(&board)[&Id::new("n32").unwrap()];
        let depth = i64::from(MAX_DEPTH);
        assert_eq!(
            last.at,
            [depth * i64::from(i32::MAX), depth * i64::from(i32::MIN)]
        );
    }

    #[test]
    fn a_node_and_a_connector_sharing_an_id_are_both_flagged() {
        let text = format!(
            "schema = 1\n{}\n[edge.a]\nkind = \"edge\"\nfrom = {{ node = \"a\" }}\nto = {{ node = \"a\" }}\nz = \"W\"\n",
            node("a", [0, 0], "")
        );
        let issues = check(&canonical(&text));
        assert_eq!(
            named(&issues),
            pairs(&[("a", "shared_id"), ("a", "shared_id")])
        );
        assert_eq!(issues[0].element, Source::Node(Id::new("a").unwrap()));
        assert_eq!(issues[1].element, Source::Edge(Id::new("a").unwrap()));
    }

    #[test]
    fn missing_kinds_and_bad_z_keys_are_flagged() {
        let long = "V".repeat(65);
        let text = format!(
            concat!(
                "schema = 1\n\n",
                "[node.bare]\nat = [0, 0]\nsize = [1, 1]\n\n",
                "[node.empty]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\nz = \"\"\n\n",
                "[node.long]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\nz = \"{long}\"\n\n",
                "[node.odd]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\nz = \"a-1\"\n\n",
                "[node.wide]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\nz = \"é\"\n\n",
                "[edge.e-bare]\nfrom = {{ node = \"odd\" }}\nto = {{ node = \"wide\" }}\n\n",
                "[edge.e-odd]\nkind = \"pipe\"\nfrom = {{ node = \"odd\" }}\nto = {{ node = \"wide\" }}\nz = \"0 1\"\n",
            ),
            long = long
        );
        let board = canonical(&text);
        assert_eq!(
            named(&check(&board)),
            pairs(&[
                ("bare", "missing_kind"),
                ("bare", "missing_z"),
                ("empty", "invalid_z"),
                ("odd", "invalid_z"),
                ("wide", "invalid_z"),
                ("e-bare", "missing_kind"),
                ("e-bare", "missing_z"),
                ("e-odd", "invalid_z"),
            ])
        );
    }

    /// Unknown values draw as their fallbacks and are never flagged.
    #[test]
    fn unknown_values_and_out_of_range_numbers_are_not_flagged_yet() {
        let text = concat!(
            "schema = 1\n\n",
            "[node.a]\nkind = \"sticky\"\nat = [2147483647, 0]\nshape = \"star\"\nsize = [-1, 0]\n",
            "style.color = \"teal\"\nstyle.fill = \"hatched\"\nz = \"V\"\n\n",
            "[edge.e]\nkind = \"pipe\"\nfrom = { anchor = [-5, 5000], node = \"a\", side = \"middle\" }\n",
            "to = { at = [0, 0] }\nheads = [\"x\", \"y\"]\nlabels = [\n  { at = 2000, text = \"far\" },\n]\n",
            "route = \"curved\"\nz = \"W\"\n",
        );
        assert_eq!(check(&canonical(text)), vec![]);
    }

    /// A chain and a cycle each 20,000 long resolve without recursion.
    #[test]
    fn long_chains_and_cycles_cost_no_stack() {
        let template = parse(&format!("schema = 1\n{}", node("x", [1, 1], "")))
            .unwrap()
            .nodes["x"]
            .clone();
        let count = 20_000;
        let id = |index: usize| Id::new(&format!("n{index:05}")).unwrap();
        for cycle in [false, true] {
            let mut board = Board::default();
            for index in 0..count {
                let mut node = template.clone();
                node.parent =
                    (cycle || index > 0).then(|| id((index + count - 1) % count).to_string());
                board.nodes.insert(id(index), node);
            }
            let issues = check(&board);
            let expected = if cycle {
                count
            } else {
                count - MAX_DEPTH as usize
            };
            assert_eq!(issues.len(), expected);
            assert!(placements(&board)
                .values()
                .all(|placed| placed.at[0] <= i64::from(MAX_DEPTH)));
        }
    }

    fn random_tree(rng: &mut Rng) -> Board {
        let count = 1 + rng.below(60);
        let id = |index: usize| format!("n{index}");
        let mut text = String::from("schema = 1\n");
        for index in 0..count {
            let parent = match rng.below(10) {
                0..=2 => None,
                3 => Some("gone".to_owned()),
                4..=6 if index > 0 => Some(id(index - 1)),
                _ => Some(id(rng.below(count))),
            };
            text.push_str(&format!(
                "[node.{}]\nat = [{}, {}]\nsize = [1, 1]\n",
                id(index),
                rng.next() as i32,
                rng.next() as i32
            ));
            if let Some(parent) = parent {
                text.push_str(&format!("parent = \"{parent}\"\n"));
            }
        }
        parse(&text).unwrap()
    }

    /// Whatever the parents, each drawn parent is the stored one, a walk up
    /// the drawn parents ends within the depth limit, each corner is its
    /// parent's plus its own, and a node is a root exactly when it is
    /// flagged or has no stored parent.
    fn trees_hold(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        for _ in 0..count {
            let board = random_tree(&mut rng);
            let places = placements(&board);
            let issues = check(&board);
            for (id, placed) in &places {
                let node = &board.nodes[*id];
                let own = node.at.map(i64::from);
                let flagged = issues.iter().any(|issue| {
                    issue.id() == *id
                        && matches!(
                            issue.kind,
                            IssueKind::MissingParent | IssueKind::ParentCycle | IssueKind::TooDeep
                        )
                });
                match placed.parent {
                    Some(parent) => {
                        assert!(!flagged);
                        assert_eq!(node.parent.as_deref(), Some(parent.as_str()));
                        let up = places[parent].at;
                        assert_eq!(placed.at, [up[0] + own[0], up[1] + own[1]]);
                    }
                    None => {
                        assert_eq!(placed.at, own);
                        assert_eq!(flagged, node.parent.is_some(), "{id}");
                    }
                }
                let mut levels = 1;
                let mut walk = placed.parent;
                while let Some(up) = walk {
                    levels += 1;
                    assert!(levels <= MAX_DEPTH, "{id} is drawn {levels} deep");
                    walk = places[up].parent;
                }
            }
            assert_eq!(write(&parse(&write(&board)).unwrap()), write(&board));
        }
    }

    #[test]
    fn random_parent_trees_resolve() {
        trees_hold(1_000, 8);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_random_parent_trees_resolve() {
        trees_hold(50_000, 9);
    }

    /// Test 9: every file that loads also checks.
    #[test]
    fn every_committed_board_checks() {
        for (name, text) in fixtures::all() {
            let board = parse(text).unwrap();
            let _ = (check(&board), access(&board), placements(&board));
            assert_eq!(
                write(&parse(&write(&board)).unwrap()),
                write(&board),
                "{name}"
            );
        }
    }
}
