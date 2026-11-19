//! The limits for untrusted files (plan §3.8, §4.12, §4.16 test 10): one
//! generated board per limit, at its value and one past it. At its value a
//! board loads, draws in full and lists no issue; one past, a board over a
//! cap of the whole file is refused at line 1, and any other is flagged,
//! drawn clamped or cut, and written back byte for byte.

use crate::scene::{
    self, Draw, FixedMeasure, Scene, Source, COORDINATE_LIMIT, MAX_BOARD_LINES, MAX_LABELS,
    MAX_LABEL_BYTES, MAX_LABEL_LINES, MAX_Z,
};
use crate::style::tests::LIGHT;
use crate::{
    parse, write, Board, IssueKind, Limit, NodeStyle, MAX_BOARD_BYTES, MAX_DEPTH, MAX_ELEMENTS,
};

/// A limit a board file can pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cap {
    FileBytes,
    Elements,
    Depth,
    Coordinates,
    Size,
    Anchor,
    LabelAt,
    LabelBytes,
    Labels,
    ZLength,
    TextSize,
    LabelLines,
    BoardLines,
}

impl Cap {
    pub(crate) const ALL: [Self; 13] = [
        Self::FileBytes,
        Self::Elements,
        Self::Depth,
        Self::Coordinates,
        Self::Size,
        Self::Anchor,
        Self::LabelAt,
        Self::LabelBytes,
        Self::Labels,
        Self::ZLength,
        Self::TextSize,
        Self::LabelLines,
        Self::BoardLines,
    ];

    /// Whether the whole file is refused past it, rather than flagged.
    fn refuses(self) -> bool {
        matches!(self, Self::FileBytes | Self::Elements)
    }
}

/// A canonical node block. `fields` are its lines after `kind`, in key
/// order, each ending in a line feed.
fn node(id: &str, text: &str, fields: &str) -> String {
    let text = if text.is_empty() {
        String::new()
    } else {
        format!("text = {}\n", quoted(text))
    };
    format!("\n[node.{id}]\n{text}kind = \"shape\"\n{fields}")
}

/// A canonical connector block from `from` to `to`, with `fields` after
/// them in key order.
fn edge(id: &str, from: &str, to: &str, fields: &str) -> String {
    format!("\n[edge.{id}]\nkind = \"edge\"\nfrom = {from}\nto = {to}\n{fields}")
}

/// `labels = [ … ]` holding these inline items.
fn labels<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let lines: String = items
        .into_iter()
        .map(|item| format!("  {item},\n"))
        .collect();
    format!("labels = [\n{lines}]\n")
}

fn label(text: &str) -> String {
    format!("{{ text = {} }}", quoted(text))
}

/// `text` as the canonical writer spells it, for text of letters, digits
/// and line feeds.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\n', "\\n"))
}

/// The board at `cap`'s value, or one past it. Canonical, so writing what
/// loads gives the same bytes.
pub(crate) fn fixture(cap: Cap, past: bool) -> String {
    let over = usize::from(past);
    let mut out = String::from(crate::EMPTY);
    let plain = "at = [0, 0]\nsize = [120, 60]\nz = \"V\"\n";
    match cap {
        Cap::FileBytes => {
            let around = "schema = 1\npad = \"\"\n".len();
            let pad = "x".repeat(MAX_BOARD_BYTES - around + over);
            out += &format!("pad = \"{pad}\"\n");
        }
        Cap::Elements => {
            for i in 0..MAX_ELEMENTS + over {
                out += &format!(
                    "\n[node.n{i:05}]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\nz = \"V\"\n"
                );
            }
        }
        Cap::Depth => {
            out += &node("n01", "", plain);
            for level in 2..=MAX_DEPTH as usize + over {
                let parent = format!("n{:02}", level - 1);
                out += &node(
                    &format!("n{level:02}"),
                    "",
                    &format!("at = [1, 1]\nparent = \"{parent}\"\nsize = [10, 10]\nz = \"V\"\n"),
                );
            }
        }
        Cap::Coordinates => {
            let limit = COORDINATE_LIMIT;
            let over = i64::from(past);
            let lo = -limit - over;
            let hi = limit - 1;
            let width = 1 + over;
            out += &node(
                "hi",
                "",
                &format!("at = [{hi}, {hi}]\nsize = [{width}, 1]\nz = \"V\"\n"),
            );
            out += &node(
                "kid",
                "",
                &format!("at = [{over}, 0]\nparent = \"hi\"\nsize = [1, 1]\nz = \"V\"\n"),
            );
            out += &node(
                "lo",
                "",
                &format!("at = [{lo}, {lo}]\nsize = [1, 1]\nz = \"V\"\n"),
            );
            let end = limit + over;
            out += &edge(
                "e",
                &format!("{{ at = [{}, 0] }}", -limit),
                &format!("{{ at = [{end}, {end}] }}"),
                "z = \"W\"\n",
            );
        }
        Cap::Size => {
            let side = 1 - i32::from(past);
            out += &node(
                "a",
                "",
                &format!("at = [0, 0]\nsize = [{side}, 1]\nz = \"V\"\n"),
            );
        }
        Cap::Anchor => {
            out += &node("a", "", plain);
            let (low, high) = if past { (-1, 1001) } else { (0, 1000) };
            out += &edge(
                "e",
                &format!("{{ anchor = [{low}, 500], node = \"a\" }}"),
                &format!("{{ anchor = [{high}, 500], node = \"a\" }}"),
                "z = \"W\"\n",
            );
        }
        Cap::LabelAt => {
            let (low, high) = if past { (-1, 1001) } else { (0, 1000) };
            let items = [
                format!("{{ at = {low}, text = \"from\" }}"),
                format!("{{ at = {high}, text = \"to\" }}"),
            ];
            out += &edge(
                "e",
                "{ at = [0, 0] }",
                "{ at = [300, 0] }",
                &format!("{}z = \"W\"\n", labels(items.iter().map(String::as_str))),
            );
        }
        Cap::LabelBytes => {
            let text = "x".repeat(MAX_LABEL_BYTES + over);
            // Wide enough that the line cap is not what cuts it.
            let fields = "at = [0, 0]\nsize = [100000, 60]\nz = \"V\"\n";
            out += &node("a", &text, fields);
            out += &edge(
                "e",
                "{ at = [0, 100] }",
                "{ at = [300, 100] }",
                &format!("{}z = \"W\"\n", labels([label(&text).as_str()])),
            );
        }
        Cap::Labels => {
            let items: Vec<String> = (0..MAX_LABELS + over)
                .map(|i| label(&format!("l{i}")))
                .collect();
            out += &edge(
                "e",
                "{ at = [0, 0] }",
                "{ at = [300, 0] }",
                &format!("{}z = \"W\"\n", labels(items.iter().map(String::as_str))),
            );
        }
        Cap::ZLength => {
            let z = "V".repeat(MAX_Z + over);
            out += &node(
                "a",
                "",
                &format!("at = [0, 0]\nsize = [120, 60]\nz = \"{z}\"\n"),
            );
            out += &edge(
                "e",
                "{ at = [0, 100] }",
                "{ at = [300, 100] }",
                &format!("z = \"{z}\"\n"),
            );
        }
        Cap::TextSize => {
            // The smallest and the largest size, or one past each; a
            // connector's `text-size` is unknown content, never a limit.
            let sizes = NodeStyle::TEXT_SIZES;
            let (low, high) = (
                sizes.start() - i64::from(past),
                sizes.end() + i64::from(past),
            );
            for (id, size) in [("a", low), ("b", high)] {
                let fields = format!(
                    "at = [0, 0]\nsize = [400, 200]\nstyle.text-size = {size}\nz = \"V\"\n"
                );
                out += &node(id, "Aa", &fields);
            }
            out += &edge(
                "e",
                "{ at = [0, 300] }",
                "{ at = [300, 300] }",
                &format!("style.text-size = {}\nz = \"W\"\n", high + 1),
            );
        }
        Cap::LabelLines => {
            // Under the fixed measurer a grapheme is 7 wide, wider than the
            // label box of a shape 10 wide, so each is a line of its own.
            let lines = MAX_LABEL_LINES + over;
            let text = "x".repeat(lines);
            out += &node("a", &text, "at = [0, 0]\nsize = [10, 60]\nz = \"V\"\n");
            let text = vec!["y"; lines].join("\n");
            out += &edge(
                "e",
                "{ at = [0, 100] }",
                "{ at = [300, 100] }",
                &format!("{}z = \"W\"\n", labels([label(&text).as_str()])),
            );
        }
        Cap::BoardLines => {
            // Nodes of a grapheme a line, as above, that take all but one
            // of the board's lines, then a connector drawn after them whose
            // label takes the last line, or one more.
            let nodes = MAX_BOARD_LINES / MAX_LABEL_LINES;
            for i in 0..nodes {
                let lines = MAX_LABEL_LINES - usize::from(i + 1 == nodes);
                let fields = "at = [0, 0]\nsize = [10, 60]\nz = \"V\"\n";
                out += &node(&format!("n{i:03}"), &"x".repeat(lines), fields);
            }
            let text = vec!["y"; 1 + over].join("\n");
            out += &edge(
                "e",
                "{ at = [0, 100] }",
                "{ at = [300, 100] }",
                &format!("{}z = \"W\"\n", labels([label(&text).as_str()])),
            );
        }
    }
    out
}

/// The issues one past `cap` gives: each element flagged for it.
fn flagged(cap: Cap) -> Vec<(&'static str, IssueKind)> {
    let out = |limit| IssueKind::OutOfRange(limit);
    match cap {
        Cap::FileBytes | Cap::Elements => vec![],
        Cap::Depth => vec![("n33", IssueKind::TooDeep)],
        Cap::Coordinates => ["hi", "kid", "lo", "e"]
            .into_iter()
            .map(|id| (id, out(Limit::Coordinates)))
            .collect(),
        Cap::Size => vec![("a", out(Limit::Size))],
        Cap::Anchor => vec![("e", out(Limit::Anchor))],
        Cap::LabelAt => vec![("e", out(Limit::LabelAt))],
        Cap::LabelBytes => vec![("a", out(Limit::LabelBytes)), ("e", out(Limit::LabelBytes))],
        Cap::Labels => vec![("e", out(Limit::Labels))],
        Cap::ZLength => vec![("a", out(Limit::ZLength)), ("e", out(Limit::ZLength))],
        Cap::TextSize => vec![("a", out(Limit::TextSize)), ("b", out(Limit::TextSize))],
        Cap::LabelLines => vec![("a", out(Limit::LabelLines)), ("e", out(Limit::LabelLines))],
        Cap::BoardLines => vec![("e", out(Limit::LabelLines))],
    }
}

/// Each text item of `id`'s element: its lines.
fn texts(scene: &Scene, id: &str) -> Vec<Vec<String>> {
    scene
        .items
        .iter()
        .filter(|item| item.source.id().as_str() == id)
        .filter_map(|item| match &item.draw {
            Draw::Text { lines, .. } => Some(lines.iter().map(|l| l.text.clone()).collect()),
            Draw::Path { .. } => None,
        })
        .collect()
}

/// The font size of `id`'s element's one text.
fn text_size(scene: &Scene, id: &str) -> f32 {
    let sizes: Vec<f32> = (scene.items.iter())
        .filter(|item| item.source.id().as_str() == id)
        .filter_map(|item| match &item.draw {
            Draw::Text { size, .. } => Some(*size),
            Draw::Path { .. } => None,
        })
        .collect();
    assert_eq!(sizes.len(), 1, "{id}");
    sizes[0]
}

/// How many of `id`'s element's texts `scene` cut.
fn cuts(scene: &Scene, id: &str) -> usize {
    scene
        .cut
        .iter()
        .filter(|source| source.id().as_str() == id)
        .count()
}

/// What `cap`'s board draws, at its value or one past it.
fn assert_drawn(cap: Cap, past: bool, board: &Board, scene: &Scene) {
    let limit = COORDINATE_LIMIT as f32;
    let node_box = |id: &str| {
        scene
            .items
            .iter()
            .find(|item| item.source == Source::Node(crate::Id::new(id).unwrap()))
            .map(|item| item.bounds)
            .unwrap()
    };
    match cap {
        Cap::FileBytes | Cap::Elements => {}
        Cap::Depth => {
            let placed = crate::placements(board);
            let last = &placed[&crate::Id::new(&format!("n{}", 32 + usize::from(past))).unwrap()];
            assert_eq!(last.parent.is_none(), past, "{cap:?}");
        }
        Cap::Coordinates => {
            // Drawn within the limit, whatever the stored values; the
            // outline's stroke reaches half its width further.
            let pad = crate::route::STROKE_WIDTH / 2.0;
            for id in ["hi", "kid", "lo"] {
                let b = node_box(id);
                assert!(
                    b.x >= -limit - pad && b.right() <= limit + pad,
                    "{id}: {b:?}"
                );
                assert!(
                    b.y >= -limit - pad && b.bottom() <= limit + pad,
                    "{id}: {b:?}"
                );
            }
        }
        Cap::Size => {
            let b = node_box("a");
            let pad = crate::route::STROKE_WIDTH;
            assert_eq!((b.width - pad, b.height - pad), (1.0, 1.0), "{cap:?}");
        }
        Cap::Anchor | Cap::LabelAt | Cap::ZLength => {}
        Cap::TextSize => {
            // Drawn at the end of the range it is at or past.
            let sizes: Vec<f32> = ["a", "b"]
                .into_iter()
                .map(|id| text_size(scene, id))
                .collect();
            assert_eq!(sizes, [8.0, 96.0], "past {past}");
        }
        Cap::LabelBytes => {
            for id in ["a", "e"] {
                let drawn: usize = texts(scene, id).iter().flatten().map(String::len).sum();
                assert_eq!(drawn, MAX_LABEL_BYTES, "{id}");
            }
        }
        Cap::Labels => assert_eq!(texts(scene, "e").len(), MAX_LABELS),
        Cap::LabelLines => {
            for id in ["a", "e"] {
                let drawn = texts(scene, id);
                assert_eq!(drawn.len(), 1, "{id}");
                assert_eq!(drawn[0].len(), MAX_LABEL_LINES, "{id}");
                assert_eq!(cuts(scene, id), usize::from(past), "{id}");
            }
        }
        Cap::BoardLines => {
            let lines: usize = (scene.items.iter())
                .map(|item| match &item.draw {
                    Draw::Text { lines, .. } => lines.len(),
                    Draw::Path { .. } => 0,
                })
                .sum();
            assert_eq!(lines, MAX_BOARD_LINES);
            assert_eq!(texts(scene, "e"), [["y"]]);
            assert_eq!(scene.cut.len(), usize::from(past));
        }
    }
}

/// §4.16 test 10: each cap at its value and one past it.
#[test]
fn every_limit_holds_at_its_value_and_flags_one_past_it() {
    for cap in Cap::ALL {
        for past in [false, true] {
            let text = fixture(cap, past);
            let board = match parse(&text) {
                Ok(board) => board,
                Err(error) => {
                    assert!(past && cap.refuses(), "{cap:?} past {past}: {error}");
                    assert_eq!((error.line, error.column), (1, 1), "{cap:?}");
                    let names = if cap == Cap::FileBytes {
                        "8,388,609 bytes, over the 8,388,608 (8 MiB)"
                    } else {
                        "50,001 nodes and connectors, over the 50,000"
                    };
                    assert!(error.message.contains(names), "{cap:?}: {error}");
                    continue;
                }
            };
            assert!(!(past && cap.refuses()), "{cap:?} loaded one past its cap");
            assert!(write(&board) == text, "{cap:?} past {past}: not canonical");
            if cap == Cap::Elements {
                assert_eq!(board.nodes.len(), MAX_ELEMENTS);
                assert_eq!(crate::check(&board), []);
                continue;
            }
            let scene = scene::build(&board, &LIGHT, &FixedMeasure);
            let issues: Vec<(String, IssueKind)> = scene::issues(&board, &scene)
                .into_iter()
                .map(|issue| (issue.id().to_string(), issue.kind))
                .collect();
            let want: Vec<(String, IssueKind)> = if past {
                flagged(cap)
                    .into_iter()
                    .map(|(id, kind)| (id.to_owned(), kind))
                    .collect()
            } else {
                vec![]
            };
            assert_eq!(issues, want, "{cap:?} past {past}");
            assert_drawn(cap, past, &board, &scene);
        }
    }
}

/// The line cap needs a measurer, so `check` alone never lists it; a
/// scene's issues do, and its markers sit on the elements it cut.
#[test]
fn only_a_scene_finds_a_label_cut_at_the_line_cap() {
    let board = parse(&fixture(Cap::LabelLines, true)).unwrap();
    assert_eq!(crate::check(&board), []);
    let scene = scene::build(&board, &LIGHT, &FixedMeasure);
    assert_eq!(scene::issues(&board, &scene).len(), 2);
    let markers = scene
        .items
        .iter()
        .filter(|item| {
            matches!(&item.draw, Draw::Path { fill: Some(fill), .. } if *fill == LIGHT.issue)
        })
        .count();
    assert_eq!(markers, 2);
    // A cache that keeps the cut items keeps what they say.
    let mut cache = scene::Cache::default();
    cache.build(&board, &LIGHT, &FixedMeasure);
    let again = cache.build(&board, &LIGHT, &FixedMeasure);
    assert_eq!(scene::issues(&board, again).len(), 2);
}

/// A label whose lines up to the cap are all blank draws nothing, and is
/// still listed as cut: as a node's label and as a connector's.
#[test]
fn a_label_cut_after_only_blank_lines_is_listed() {
    let text = format!("{}hello", "\n".repeat(MAX_LABEL_LINES + 76));
    let mut out = String::from(crate::EMPTY);
    out += &node("a", &text, "at = [0, 0]\nsize = [120, 60]\nz = \"V\"\n");
    out += &edge(
        "e",
        "{ at = [0, 100] }",
        "{ at = [300, 100] }",
        &format!("{}z = \"W\"\n", labels([label(&text).as_str()])),
    );
    let board = parse(&out).unwrap();
    let scene = scene::build(&board, &LIGHT, &FixedMeasure);
    let issues: Vec<(String, IssueKind)> = scene::issues(&board, &scene)
        .into_iter()
        .map(|issue| (issue.id().to_string(), issue.kind))
        .collect();
    let cut = IssueKind::OutOfRange(Limit::LabelLines);
    assert_eq!(issues, [("a".to_owned(), cut), ("e".to_owned(), cut)]);
    assert!(texts(&scene, "a").is_empty());
    assert!(texts(&scene, "e").is_empty());
    let mut cache = scene::Cache::default();
    cache.build(&board, &LIGHT, &FixedMeasure);
    assert_eq!(*cache.build(&board, &LIGHT, &FixedMeasure), scene);
}

/// A cache lays out again what the board's line budget changed for: a
/// label cut to the lines left is drawn whole once an earlier one frees
/// them, and cut again when they are taken back.
#[test]
fn a_cached_scene_follows_the_lines_left_on_the_board() {
    let full = parse(&fixture(Cap::BoardLines, true)).unwrap();
    let mut freed = full.clone();
    let first = crate::Id::new("n000").unwrap();
    freed.nodes.get_mut(&first).unwrap().text = "x".into();
    let mut cache = scene::Cache::default();
    for (board, cut) in [(&full, 1), (&freed, 0), (&full, 1), (&freed, 0)] {
        let whole = scene::build(board, &LIGHT, &FixedMeasure);
        assert_eq!(whole.cut.len(), cut);
        assert_eq!(*cache.build(board, &LIGHT, &FixedMeasure), whole);
    }
    // With no line left, a later label is cut unmeasured and draws nothing.
    let mut spent = full.clone();
    let last = crate::Id::new("n255").unwrap();
    spent.nodes.get_mut(&last).unwrap().text += "x";
    let label = &mut spent.edges.get_mut("e").unwrap().labels[0].text;
    *label = "y".into();
    let scene = cache.build(&spent, &LIGHT, &FixedMeasure).clone();
    assert_eq!(scene, scene::build(&spent, &LIGHT, &FixedMeasure));
    assert!(texts(&scene, "e").is_empty());
    let issues: Vec<String> = (scene::issues(&spent, &scene).iter())
        .map(|issue| issue.id().to_string())
        .collect();
    assert_eq!(issues, ["e"]);
}

/// The small boards of every flagged limit, at and past it, for the
/// mutation runs. The board's line budget takes a board of 280 KB, too
/// large to mutate thousands of times.
pub(crate) fn small() -> Vec<String> {
    Cap::ALL
        .into_iter()
        .filter(|cap| !cap.refuses() && *cap != Cap::BoardLines)
        .flat_map(|cap| [fixture(cap, false), fixture(cap, true)])
        .collect()
}
