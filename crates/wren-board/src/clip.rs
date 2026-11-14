//! The clipboard (§5.10 rules 2, 4 and 6 to 9 of the board plan).
//!
//! iced's clipboard holds text, so a copied fragment is a board file: the
//! line [`HEADER`], then what the canonical writer gives for a board holding
//! only the fragment. Pasting reads it back with the file loader. Any other
//! text pastes as one text node.

use crate::edit::{
    because, copies, fragment, on_top, saturate, trim_label, Area, Change, Fragment, Layout, Plan,
};
use crate::ids::{fresh, Ids};
use crate::scene::{Source, MAX_LABEL_BYTES};
use crate::z::Element;
use crate::{
    access, parse, placements, snap, write, Access, Board, End, Extra, Named, Node, NodeKind,
    NodeStyle, Shape, GRID,
};

/// The first line of a copied fragment. The text after it is a valid board
/// file.
pub const HEADER: &str = "# wren-board fragment 1";

/// The clipboard text for the selection `ids` (§5.10 rules 2 and 4), each
/// element named with its kind, or `None` when the selection holds no node,
/// which leaves nothing to copy.
///
/// A node whose parent is outside the fragment is written at the top level
/// at its absolute place. Each scope is keyed afresh in the source's
/// drawing order, so the fragment stacks as the copied elements did. The
/// fragment keeps the source's `schema` and `requires`, so a build that may
/// not edit the source cannot paste what came from it either.
pub fn copy(board: &Board, ids: &[Source]) -> Option<String> {
    let part = fragment(board, ids);
    if part.nodes.is_empty() {
        return None;
    }
    Some(text(board, &part))
}

/// What cutting the selection `ids` does (§5.10 rule 5): the clipboard text
/// of [`copy`], and the elements to delete, which are the fragment's own.
/// Deleting them also takes every connector attached to a removed node
/// (rule 1); a selected connector the fragment leaves out stays on the
/// board. `None` when the selection holds no node.
pub fn cut(board: &Board, ids: &[Source]) -> Option<(String, Vec<Source>)> {
    let part = fragment(board, ids);
    if part.nodes.is_empty() {
        return None;
    }
    let nodes = part.nodes.iter().cloned().map(Source::Node);
    let removed = nodes.chain(part.edges.iter().cloned().map(Source::Edge));
    Some((text(board, &part), removed.collect()))
}

/// The clipboard text for the fragment `part` of `board`.
fn text(board: &Board, part: &Fragment) -> String {
    let layout = Layout::of(board, part);
    let roots = layout
        .order
        .iter()
        .filter(|element| layout.is_root(**element))
        .count();
    let mut keys = layout.keys(crate::z::insert(&[], 0, roots).keys);
    let mut out = Board {
        schema: board.schema,
        requires: board.requires.clone(),
        ..Board::default()
    };
    for element in &layout.order {
        let z = keys.remove(element);
        match *element {
            Element::Node(id) => {
                let node = Node {
                    z,
                    ..layout.node(board, id, [0, 0])
                };
                out.nodes.insert(id.clone(), node);
            }
            Element::Edge(id) => {
                let mut edge = board.edges[id].clone();
                edge.z = z;
                out.edges.insert(id.clone(), edge);
            }
        }
    }
    format!("{HEADER}\n{}", write(&out))
}

/// Text from outside Wren as a label (§5.10 rule 9): line breaks as LF,
/// control characters other than tab and LF removed, trailing blank lines
/// trimmed. The canvas measures this text for the size it pastes with.
pub fn label(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let kept: String = text
        .chars()
        .filter(|&c| c == '\t' || c == '\n' || !c.is_control())
        .collect();
    trim_label(&kept)
}

/// Whether `text` is a copied fragment rather than foreign text.
pub fn is_fragment(text: &str) -> bool {
    split(text).is_some()
}

/// The fragment after the header line, if `text` starts with it. A CRLF
/// after the header, as a clipboard on another system may give, counts.
fn split(text: &str) -> Option<&str> {
    let rest = text.strip_prefix(HEADER)?;
    if rest.is_empty() {
        return Some(rest);
    }
    rest.strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
}

/// The plan for pasting `text` (§5.10 rules 6 to 9). `size` is the size the
/// canvas measured for foreign text, `view` the board area in view, and
/// `nth` how many times this text has been pasted into this board, this
/// time included.
pub(crate) fn paste(
    board: &Board,
    text: &str,
    size: [i32; 2],
    view: Area,
    nth: u32,
    ids: &mut dyn Ids,
) -> Plan {
    match split(text) {
        Some(fragment) => paste_fragment(board, fragment, view, nth, ids),
        None => paste_text(board, text, size, view, ids),
    }
}

fn paste_fragment(board: &Board, text: &str, view: Area, nth: u32, ids: &mut dyn Ids) -> Plan {
    let source = match parse(text) {
        Ok(source) => source,
        Err(mut error) => {
            // Name the line of the clipboard text, header included.
            error.line += 1;
            return Plan::notice(format!(
                "Cannot paste: the board fragment is not valid at {error}."
            ));
        }
    };
    if let Access::ReadOnly(reason) = access(&source) {
        return Plan::notice(format!(
            "Cannot paste: the board fragment {}.",
            because(&reason)
        ));
    }
    // Every node, and each connector whose node ends are all in the
    // fragment (§5.10 rule 7): one naming any other node is dropped.
    let mut part = Fragment {
        nodes: source.nodes.keys().cloned().collect(),
        ..Fragment::default()
    };
    part.edges = source
        .edges
        .iter()
        .filter(|(_, edge)| part.carries(edge))
        .map(|(id, _)| id.clone())
        .collect();
    let Some(bounds) = bounds(&source, &part) else {
        return Plan::default();
    };
    let step = i64::from(nth) * i64::from(GRID);
    let offset = Area {
        at: [
            saturate(i64::from(bounds.at[0]) + step),
            saturate(i64::from(bounds.at[1]) + step),
        ],
        size: bounds.size,
    };
    let by = if overlaps(offset, view) {
        [step, step]
    } else {
        let corner = snap::centred(centre(view), bounds.size);
        [
            i64::from(corner[0]) - i64::from(bounds.at[0]),
            i64::from(corner[1]) - i64::from(bounds.at[1]),
        ]
    };
    copies(board, &source, &part, by, ids)
}

fn paste_text(board: &Board, text: &str, size: [i32; 2], view: Area, ids: &mut dyn Ids) -> Plan {
    let text = label(text);
    if text.is_empty() {
        return Plan::default();
    }
    if text.len() > MAX_LABEL_BYTES {
        return Plan::notice(format!(
            "Cannot paste: the text is longer than a label may be ({} KiB).",
            MAX_LABEL_BYTES / 1024
        ));
    }
    let size = size.map(|length| length.max(1));
    let id = fresh(board, &crate::edit::dangling_names(board), ids);
    let (keys, mut changes) = on_top(board, 1);
    let node = Node {
        text,
        kind: Some(Named::Known(NodeKind::Text)),
        at: snap::centred(centre(view), size),
        parent: None,
        shape: Named::Known(Shape::Rect),
        size,
        style: NodeStyle::default(),
        z: keys.into_iter().next(),
        extra: Extra::new(),
    };
    changes.push(Change::PutNode(id.clone(), Box::new(node)));
    Plan {
        changes,
        select: Some(vec![id]),
        ..Plan::default()
    }
}

/// The box around the fragment's nodes and its connectors' free ends, in
/// absolute units; `None` when it holds nothing.
fn bounds(source: &Board, part: &Fragment) -> Option<Area> {
    let placed = placements(source);
    let mut corners: Vec<[i64; 4]> = Vec::new();
    for id in &part.nodes {
        let (Some(place), Some(node)) = (placed.get(id), source.nodes.get(id)) else {
            continue;
        };
        let [w, h] = node.size.map(|length| i64::from(length.max(1)));
        corners.push([place.at[0], place.at[1], place.at[0] + w, place.at[1] + h]);
    }
    for id in &part.edges {
        let edge = &source.edges[id];
        for end in [&edge.from, &edge.to] {
            if let End::Free { at, .. } = end {
                let [x, y] = at.map(i64::from);
                corners.push([x, y, x, y]);
            }
        }
    }
    let first = *corners.first()?;
    let [left, top, right, bottom] = corners.iter().fold(first, |a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    });
    Some(Area {
        at: [saturate(left), saturate(top)],
        size: [saturate(right - left), saturate(bottom - top)],
    })
}

/// Whether two boxes share some area; an empty box shares none.
fn overlaps(a: Area, b: Area) -> bool {
    let axis = |k: usize| {
        let (a0, a1) = (
            i64::from(a.at[k]),
            i64::from(a.at[k]) + i64::from(a.size[k]),
        );
        let (b0, b1) = (
            i64::from(b.at[k]),
            i64::from(b.at[k]) + i64::from(b.size[k]),
        );
        a0 < b1 && b0 < a1 && a0 < a1 && b0 < b1
    };
    axis(0) && axis(1)
}

fn centre(view: Area) -> [f32; 2] {
    [0, 1].map(|k| (f64::from(view.at[k]) + f64::from(view.size[k]) / 2.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::tests::{edges, ids, line_diff, nodes, Listed, BOARD, FREE_ENDS};
    use crate::edit::{plan, Intent};
    use crate::fixtures::Rng;
    use crate::ids::Seeded;
    use crate::{check, Issue};

    /// The whole of [`BOARD`] and more, in view.
    const VIEW: Area = Area {
        at: [-200, -200],
        size: [1200, 800],
    };

    fn paste_intent(text: &str, nth: u32, view: Area) -> Intent {
        Intent::Paste {
            text: text.into(),
            size: [120, 40],
            view,
            nth,
        }
    }

    /// Plans and applies a paste, checking that the board still writes,
    /// reads back, and has no integrity issue it did not have, except that
    /// a pasted element may lack the `kind` its fragment gave it none of.
    fn pasted(board: &mut Board, intent: &Intent, ids: &mut dyn Ids) -> Plan {
        let before: Vec<Issue> = check(board);
        let old = board.clone();
        let plan = plan(board, intent, ids);
        board.apply(&plan.changes);
        let written = write(board);
        assert_eq!(write(&parse(&written).unwrap()), written);
        for issue in check(board) {
            let new = !old.nodes.contains_key(issue.id()) && !old.edges.contains_key(issue.id());
            let carried = new && issue.kind == crate::IssueKind::MissingKind;
            assert!(before.contains(&issue) || carried, "new issue {issue:?}");
        }
        plan
    }

    #[test]
    fn a_copy_is_a_header_and_a_board_file() {
        let board = parse(BOARD).unwrap();
        let text = copy(&board, &nodes(&["a"])).unwrap();
        assert_eq!(
            text,
            "# wren-board fragment 1\nschema = 1\n\n\
             [node.a]\ntext = \"Outer\"\nkind = \"shape\"\nat = [0, 0]\next.tool.note = \"kept\"\n\
             size = [200, 120]\nz = \"V\"\n\n\
             [node.b]\ntext = \"Inner\"\nkind = \"shape\"\nat = [20, 20]\nparent = \"a\"\n\
             size = [120, 60]\nz = \"V\"\n\n\
             [edge.e]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { at = [600, 300] }\nz = \"V01\"\n"
        );
        let rest = split(&text).unwrap();
        assert_eq!(write(&parse(rest).unwrap()), rest, "canonical");
        // A child copied without its parent is written at its absolute
        // place at the top level.
        let text = copy(&board, &nodes(&["b"])).unwrap();
        assert!(text.contains("at = [20, 20]\nsize") && !text.contains("parent"));
        // No node, no fragment.
        let unnoded = [edges(&["e", "f"]), nodes(&["gone"])].concat();
        assert_eq!(copy(&board, &unnoded), None);
    }

    /// The clipboard carries a connector with two free ends when it is
    /// selected as a connector, and never through a node of the same id
    /// (§5.10 rule 2).
    #[test]
    fn a_copy_takes_a_free_connector_only_when_it_is_selected() {
        let board = parse(FREE_ENDS).unwrap();
        let a =
            "[node.a]\ntext = \"A\"\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\nz = \"V\"\n";
        // (a) A node and a selected free-ended connector.
        let picked = [nodes(&["a"]), edges(&["x"])].concat();
        assert_eq!(
            copy(&board, &picked).unwrap(),
            format!(
                "{HEADER}\nschema = 1\n\n{a}\n\
                 [edge.x]\nkind = \"edge\"\nfrom = {{ at = [400, 0] }}\nto = {{ at = [400, 200] }}\n\
                 labels = [\n  {{ text = \"Free\" }},\n]\nz = \"V01\"\n"
            )
        );
        // (b) The connector alone: no node, nothing to copy.
        assert_eq!(copy(&board, &edges(&["x"])), None);
        assert_eq!(copy(&board, &edges(&["shared"])), None);
        // (c) The node `shared` alone leaves the connector `shared`.
        assert_eq!(
            copy(&board, &nodes(&["shared"])).unwrap(),
            format!(
                "{HEADER}\nschema = 1\n\n\
                 [node.shared]\ntext = \"Node shared\"\nkind = \"shape\"\nat = [0, 300]\n\
                 size = [120, 60]\nz = \"V\"\n"
            )
        );
    }

    /// Cut deletes what it copies: a selected connector the fragment leaves
    /// out is not among the elements it removes (§5.10 rule 5). The
    /// connectors attached to a removed node go with it in the delete.
    #[test]
    fn a_cut_removes_only_the_fragment() {
        let board = parse(
            "schema = 1\n\n\
             [node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\nz = \"a\"\n\n\
             [node.b]\nkind = \"shape\"\nat = [0, 200]\nsize = [120, 60]\nz = \"b\"\n\n\
             [node.c]\nkind = \"shape\"\nat = [300, 0]\nsize = [120, 60]\nz = \"c\"\n\n\
             [edge.ab]\nkind = \"edge\"\nfrom = { node = \"a\" }\nto = { node = \"b\" }\nz = \"d\"\n\n\
             [edge.bc]\nkind = \"edge\"\nfrom = { node = \"b\" }\nto = { node = \"c\" }\nz = \"e\"\n\n\
             [edge.cf]\nkind = \"edge\"\nfrom = { node = \"c\" }\nto = { at = [600, 0] }\nz = \"f\"\n\n\
             [edge.ff]\nkind = \"edge\"\nfrom = { at = [600, 200] }\nto = { at = [600, 400] }\nz = \"g\"\n",
        )
        .unwrap();
        let cases = [
            // A connector between two nodes that stay.
            ([nodes(&["a"]), edges(&["bc"])].concat(), nodes(&["a"])),
            // A connector with one free end, its node staying.
            ([nodes(&["a"]), edges(&["cf"])].concat(), nodes(&["a"])),
            // One end on a cut node, the other on a node that stays: the
            // delete takes it with `a`, and the paste cannot give it back.
            ([nodes(&["a"]), edges(&["ab"])].concat(), nodes(&["a"])),
            (
                [nodes(&["a", "b"]), edges(&["ff"])].concat(),
                [nodes(&["a", "b"]), edges(&["ab", "ff"])].concat(),
            ),
            (
                [nodes(&["c"]), edges(&["cf"])].concat(),
                [nodes(&["c"]), edges(&["cf"])].concat(),
            ),
        ];
        for (picked, removed) in cases {
            let (text, taken) = cut(&board, &picked).unwrap();
            assert_eq!(Some(text), copy(&board, &picked), "{picked:?}");
            assert_eq!(taken, removed, "{picked:?}");
        }
        assert_eq!(cut(&board, &edges(&["ff"])), None);
    }

    #[test]
    fn a_copy_keeps_what_the_source_requires() {
        let board = parse("schema = 1\nrequires = [\"frames\"]\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n").unwrap();
        let text = copy(&board, &nodes(&["a"])).unwrap();
        assert!(text.starts_with("# wren-board fragment 1\nschema = 1\nrequires = [\"frames\"]\n"));
        let mut target = parse(BOARD).unwrap();
        let plan = pasted(
            &mut target,
            &paste_intent(&text, 1, VIEW),
            &mut Listed(Vec::new()),
        );
        assert!(plan.is_empty());
        assert_eq!(
            plan.notice.as_deref(),
            Some(
                "Cannot paste: the board fragment needs features this build does not have: frames."
            )
        );
        let plan = pasted(
            &mut target,
            &paste_intent("# wren-board fragment 1\nschema = 2\n", 1, VIEW),
            &mut Listed(Vec::new()),
        );
        assert_eq!(
            plan.notice.as_deref(),
            Some("Cannot paste: the board fragment is written for board schema 2.")
        );
    }

    #[test]
    fn pasting_twice_into_the_source_steps_a_cell_each_time() {
        let mut board = parse(BOARD).unwrap();
        let text = copy(&board, &nodes(&["a"])).unwrap();
        let mut source = Listed(vec!["p1", "p2", "p3", "q1", "q2", "q3"]);
        let first = pasted(&mut board, &paste_intent(&text, 1, VIEW), &mut source);
        assert_eq!(first.select, Some(ids(&["p1", "p2", "p3"])));
        assert_eq!(first.edit, None);
        let second = pasted(&mut board, &paste_intent(&text, 2, VIEW), &mut source);
        assert_eq!(second.select, Some(ids(&["q1", "q2", "q3"])));
        assert_eq!(board.nodes["p1"].at, [20, 20]);
        assert_eq!(board.nodes["q1"].at, [40, 40]);
        // The child stays inside its own copied parent.
        assert_eq!(board.nodes["q2"].parent.as_deref(), Some("q1"));
        assert_eq!(board.nodes["q2"].at, [20, 20]);
        let edge = &board.edges["q3"];
        assert_eq!(
            edge.from,
            End::Node {
                node: "q1".into(),
                side: None,
                anchor: None,
                extra: Extra::new()
            }
        );
        assert_eq!(
            edge.to,
            End::Free {
                at: [640, 340],
                extra: Extra::new()
            }
        );
        // On top, the second above the first, in the copied order.
        let keys: Vec<_> = ["p1", "p3", "q1", "q3"]
            .map(|id| {
                board
                    .nodes
                    .get(id)
                    .map_or_else(|| board.edges[id].z.clone(), |n| n.z.clone())
                    .unwrap()
            })
            .to_vec();
        assert_eq!(keys, ["Y01", "Y02", "Y03", "Y04"]);
        // The source is untouched.
        let original = parse(BOARD).unwrap();
        for (id, node) in &original.nodes {
            assert_eq!(&board.nodes[id], node);
        }
    }

    #[test]
    fn pasted_ids_are_new_even_where_every_copied_id_is_taken() {
        let mut board = parse(BOARD).unwrap();
        let text = copy(&board, &nodes(&["a", "c"])).unwrap();
        // The source draws the copied ids first; each is taken.
        let mut source = Listed(vec![
            "a", "b", "c", "e", "f", "t", "n1", "n2", "n3", "n4", "n5",
        ]);
        let plan = pasted(&mut board, &paste_intent(&text, 1, VIEW), &mut source);
        assert_eq!(plan.select, Some(ids(&["n1", "n2", "n3", "n4", "n5"])));
        let diff = line_diff(BOARD, &write(&board));
        assert!(diff.iter().all(|line| line.starts_with('+')), "{diff:?}");
    }

    /// Damage in a hand-made fragment never reaches the board: a connector
    /// to a node outside it is dropped, and a parent the fragment does not
    /// draw it inside (missing, or a cycle) is cleared.
    #[test]
    fn a_damaged_fragment_pastes_whole() {
        let text = "# wren-board fragment 1\nschema = 1\n\n\
            [node.x]\nkind = \"shape\"\nat = [0, 0]\nparent = \"y\"\nsize = [40, 40]\nz = \"V\"\n\n\
            [node.y]\nkind = \"shape\"\nat = [100, 0]\nparent = \"x\"\nsize = [40, 40]\n\n\
            [node.k]\nkind = \"shape\"\nat = [200, 0]\nparent = \"outside\"\nsize = [40, 40]\nz = \"V\"\n\n\
            [edge.x]\nkind = \"edge\"\nfrom = { node = \"x\" }\nto = { node = \"outside\" }\nz = \"W\"\n\n\
            [edge.y]\nkind = \"edge\"\nfrom = { node = \"x\" }\nto = { node = \"y\" }\nz = \"X\"\n";
        let mut board = parse(BOARD).unwrap();
        let plan = pasted(
            &mut board,
            &paste_intent(text, 1, VIEW),
            &mut Seeded::new(1),
        );
        let new = plan.select.unwrap();
        assert_eq!(new.len(), 4, "three nodes and one connector");
        for id in &new[..3] {
            assert_eq!(board.nodes[id].parent, None);
        }
        assert!(board.edges.contains_key(&new[3]));
        assert!(check(&board).is_empty());
        // A duplicate header is a parse error, named by its clipboard line.
        let twice = "# wren-board fragment 1\nschema = 1\n[node.a]\nat = [0, 0]\nsize = [1, 1]\n\
                     [node.a]\nat = [0, 0]\nsize = [1, 1]\n";
        let plan = pasted(
            &mut board,
            &paste_intent(twice, 1, VIEW),
            &mut Seeded::new(1),
        );
        assert!(plan.is_empty());
        let notice = plan.notice.unwrap();
        assert!(
            notice.starts_with("Cannot paste: the board fragment is not valid at line 6,"),
            "{notice}"
        );
    }

    #[test]
    fn a_fragment_out_of_view_is_centred_in_it() {
        let mut board = parse(BOARD).unwrap();
        let text = copy(&board, &nodes(&["b"])).unwrap();
        let view = Area {
            at: [1000, 1000],
            size: [610, 400],
        };
        let plan = pasted(
            &mut board,
            &paste_intent(&text, 1, view),
            &mut Listed(vec!["p"]),
        );
        assert_eq!(plan.select, Some(ids(&["p"])));
        // The centre (1305, 1200) less half of 120 by 60, snapped.
        assert_eq!(board.nodes["p"].at, [1240, 1180]);
        // A view not yet measured holds nothing, so it centres there.
        let empty = Area::default();
        let plan = pasted(
            &mut board,
            &paste_intent(&text, 1, empty),
            &mut Listed(vec!["q"]),
        );
        assert_eq!(plan.select, Some(ids(&["q"])));
        assert_eq!(board.nodes["q"].at, [-60, -20]);
    }

    #[test]
    fn foreign_text_is_one_text_node() {
        let mut board = parse(BOARD).unwrap();
        let intent = paste_intent("Line one\r\nLine\u{7}two\u{85}\t!\n\n  \n", 4, VIEW);
        let plan = pasted(&mut board, &intent, &mut Listed(vec!["p"]));
        assert_eq!(plan.select, Some(ids(&["p"])));
        assert_eq!(plan.edit, None);
        assert_eq!(
            line_diff(BOARD, &write(&board)),
            [
                "+[node.p]",
                "+text = \"Line one\\nLinetwo\\t!\"",
                "+kind = \"text\"",
                "+at = [340, 180]",
                "+size = [120, 40]",
                "+z = \"Y01\"",
            ]
        );
        for empty in ["", "\n\n", " \r\n\t", "\u{0}\u{1b}"] {
            let plan = pasted(
                &mut board,
                &paste_intent(empty, 1, VIEW),
                &mut Listed(Vec::new()),
            );
            assert_eq!(plan, Plan::default(), "{empty:?}");
        }
        let long = "x".repeat(MAX_LABEL_BYTES + 1);
        let plan = pasted(
            &mut board,
            &paste_intent(&long, 1, VIEW),
            &mut Listed(Vec::new()),
        );
        assert!(plan.is_empty());
        assert_eq!(
            plan.notice.as_deref(),
            Some("Cannot paste: the text is longer than a label may be (16 KiB).")
        );
        let fits = "x".repeat(MAX_LABEL_BYTES);
        let plan = pasted(
            &mut board,
            &paste_intent(&fits, 1, VIEW),
            &mut Listed(vec!["q"]),
        );
        assert_eq!(plan.select, Some(ids(&["q"])));
    }

    #[test]
    fn the_header_is_the_whole_first_line() {
        for (text, fragment) in [
            ("# wren-board fragment 1", true),
            ("# wren-board fragment 1\nschema = 1\n", true),
            ("# wren-board fragment 1\r\nschema = 1\r\n", true),
            ("# wren-board fragment 1 \nschema = 1\n", false),
            ("# wren-board fragment 10\n", false),
            (" # wren-board fragment 1\n", false),
            ("schema = 1\n", false),
        ] {
            assert_eq!(is_fragment(text), fragment, "{text:?}");
        }
        let mut board = parse(BOARD).unwrap();
        let text = copy(&board, &nodes(&["t"])).unwrap().replace('\n', "\r\n");
        let plan = pasted(
            &mut board,
            &paste_intent(&text, 1, VIEW),
            &mut Listed(vec!["p"]),
        );
        assert_eq!(board.nodes["p"].text, "Note");
        assert_eq!(plan.select, Some(ids(&["p"])));
    }

    /// Mutations of a copied fragment, and arbitrary text with and
    /// without the header: planning a paste never panics, and whatever it
    /// pastes leaves a board that writes, reads back, and has no new issue.
    fn pastes_hold(count: usize, seed: u64) {
        let board = parse(BOARD).unwrap();
        let copied = copy(&board, &nodes(&["a", "c", "t"])).unwrap();
        let pieces = [
            "\n",
            "\r",
            "\"",
            "[",
            "]",
            "=",
            "{",
            "}",
            ",",
            " ",
            "0",
            "-1",
            "a",
            "z",
            "node",
            "edge",
            "parent",
            "\u{0}",
            "é",
            "# wren-board fragment 1\n",
            "schema = 2\n",
            "at = [",
            "}\n",
        ];
        let mut rng = Rng(seed);
        let mut ids = Seeded::new(seed);
        let mut applied = 0;
        for _ in 0..count {
            let mut text: Vec<char> = copied.chars().collect();
            if rng.one_in(4) {
                text = (0..rng.below(80))
                    .flat_map(|_| rng.pick(&pieces).chars().collect::<Vec<_>>())
                    .collect();
            }
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(text.len() + 1);
                match rng.below(3) {
                    0 if at < text.len() => drop(text.remove(at)),
                    1 => text
                        .splice(at..at, rng.pick(&pieces).chars())
                        .for_each(drop),
                    _ => text.truncate(at),
                }
            }
            let text: String = text.into_iter().collect();
            let view = Area {
                at: [rng.below(4000) as i32 - 2000, rng.below(4000) as i32 - 2000],
                size: [rng.below(2000) as i32, rng.below(2000) as i32],
            };
            let nth = rng.below(4) as u32;
            let mut target = board.clone();
            let plan = pasted(&mut target, &paste_intent(&text, nth, view), &mut ids);
            applied += usize::from(!plan.is_empty());
        }
        // Enough of them paste something for the property to mean much.
        assert!(applied > count / 4, "{applied} of {count}");
    }

    #[test]
    fn arbitrary_pastes_never_panic() {
        pastes_hold(2_000, 1);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_arbitrary_pastes_never_panic() {
        pastes_hold(100_000, 2);
    }
}
