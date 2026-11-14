//! The acceptance script of §5.20, replayed against the editing functions
//! alone: [`edit::plan`], [`place`], [`walk`] and [`history`], with no
//! canvas and no app. A small driver stands in for the canvas machine's
//! keyboard states (Idle, Armed, Editing, keyboard Connecting) and does
//! only what the script needs. The result must be the keyboard model's
//! layout in `golden/accept/checkout-flow.toml`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::edit::{self, Endpoint, Intent, Plan};
use crate::history::{History, Snapshot, Unit};
use crate::ids::Seeded;
use crate::scene::Source;
use crate::{parse, place, walk, write, Board, Dir, End, Id, Named, NodeKind, Shape, EMPTY, GRID};

const GOLDEN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/golden/accept/checkout-flow.toml"
));

#[derive(Clone, Debug, PartialEq)]
enum State {
    Idle,
    Armed(Shape),
    Editing(Id),
    Connecting { target: Option<Id>, query: String },
}

/// What the script's keys were.
#[derive(Debug, Default, PartialEq)]
struct Count {
    label: usize,
    query: usize,
    chords: usize,
}

struct Driver {
    board: Board,
    state: State,
    selection: Vec<Id>,
    cursor: Option<Id>,
    ids: Seeded,
    history: History,
    now: Instant,
    count: Count,
}

impl Driver {
    fn new(seed: u64) -> Self {
        Self {
            board: parse(EMPTY).unwrap(),
            state: State::Idle,
            selection: Vec::new(),
            cursor: None,
            ids: Seeded::new(seed),
            history: History::new(),
            now: Instant::now(),
            count: Count::default(),
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            board: self.board.clone(),
            selection: self
                .selection
                .iter()
                .filter_map(|id| Source::of(&self.board, id))
                .collect(),
            cursor: self
                .cursor
                .as_ref()
                .and_then(|id| Source::of(&self.board, id)),
        }
    }

    fn select(&mut self, ids: Vec<Id>) {
        if let Some(last) = ids.last() {
            self.cursor = Some(last.clone());
        }
        self.selection = ids;
    }

    fn plan(&mut self, intent: &Intent) -> Plan {
        edit::plan(&self.board, intent, &mut self.ids)
    }

    /// Applies a plan as one unit, or as the start of a group when it
    /// opens a label, and takes its selection.
    fn commit(&mut self, plan: Plan) {
        assert!(!plan.is_empty(), "{:?}", plan.notice);
        let before = self.snapshot();
        self.board.apply(&plan.changes);
        match &plan.edit {
            Some(_) => self.history.open_group(before),
            None => self.history.record(before, Unit::One, self.now),
        }
        if let Some(select) = plan.select {
            self.select(select);
        }
        self.state = match plan.edit {
            Some(id) => State::Editing(id),
            None => State::Idle,
        };
    }

    fn one(&self) -> Option<&Id> {
        match self.selection.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }

    fn one_node(&self) -> Option<Id> {
        self.one()
            .filter(|id| self.board.nodes.contains_key(*id))
            .cloned()
    }

    /// The armed shape tool, which a creation takes and resets.
    fn take_shape(&mut self) -> Option<Shape> {
        let shape = match self.state {
            State::Armed(shape) => Some(shape),
            _ => None,
        };
        self.state = State::Idle;
        shape
    }

    fn press(&mut self, key: &str) {
        self.now += Duration::from_millis(100);
        if let Some(text) = key.strip_prefix("type:") {
            for c in text.chars() {
                self.type_char(c);
            }
            return;
        }
        self.count.chords += 1;
        match self.state.clone() {
            State::Editing(id) => match key {
                "escape" => self.commit_label(&id),
                "tab" => {
                    self.commit_label(&id);
                    self.connect_next();
                }
                _ => panic!("{key} while editing"),
            },
            State::Connecting { target, .. } => match key {
                "escape" => self.state = State::Idle,
                "enter" => self.finish_connecting(target),
                _ => {
                    let dir = arrow(key).unwrap_or_else(|| panic!("{key} while connecting"));
                    let source = self.one_node().unwrap();
                    let next = walk::next(&self.board, target.as_ref().unwrap_or(&source), dir);
                    self.state = State::Connecting {
                        target: next.or(target),
                        query: String::new(),
                    };
                }
            },
            State::Idle | State::Armed(_) => self.command(key),
        }
    }

    fn command(&mut self, key: &str) {
        let (mods, name) = key.rsplit_once('+').unwrap_or(("", key));
        match (mods, name) {
            ("", "escape") => match self.state {
                State::Armed(_) => self.state = State::Idle,
                _ => self.selection.clear(),
            },
            ("", "tab") => self.connect_next(),
            ("", "enter") => {
                let id = self.one().expect("one element to edit").clone();
                self.state = State::Editing(id);
            }
            ("", "u") => self.state = State::Armed(Shape::Pill),
            ("", "d") => self.state = State::Armed(Shape::Diamond),
            ("", "c") => {
                assert!(self.one_node().is_some(), "connect needs one node");
                self.state = State::Connecting {
                    target: None,
                    query: String::new(),
                };
            }
            ("mod", name) => {
                let source = self.one_node().expect("one node to connect from");
                let shape = self.take_shape();
                let plan = self.plan(&Intent::Extend {
                    from: source,
                    dir: arrow(name),
                    shape,
                });
                self.commit(plan);
            }
            ("shift+mod", name) => {
                let node = self.one_node().expect("one node");
                let found = walk::connector(&self.board, &node, arrow(name).unwrap());
                self.select(vec![found.expect("a connector on that side")]);
            }
            ("", name) => {
                let dir = arrow(name).unwrap_or_else(|| panic!("unbound key {key}"));
                let next = match self.one() {
                    Some(from) => walk::next(&self.board, from, dir),
                    None => self.cursor.clone().or_else(|| walk::first(&self.board)),
                };
                if let Some(next) = next {
                    self.select(vec![next]);
                }
            }
            _ => panic!("unbound key {key}"),
        }
    }

    /// `board.connect.next`: from the one selected element, or a new
    /// unconnected node in the middle of the view, here the origin.
    fn connect_next(&mut self) {
        let shape = self.take_shape();
        let plan = match self.one() {
            Some(from) => self.plan(&Intent::Extend {
                from: from.clone(),
                dir: None,
                shape,
            }),
            None => {
                let at = place::start(&self.board, [0.0, 0.0], place::SIZE).unwrap();
                self.plan(&Intent::AddNode {
                    kind: NodeKind::Shape,
                    shape: shape.unwrap_or(Shape::Rect),
                    at,
                    size: place::SIZE,
                    from: None,
                })
            }
        };
        self.commit(plan);
    }

    fn type_char(&mut self, c: char) {
        match &mut self.state {
            State::Editing(id) => {
                let id = id.clone();
                self.count.label += 1;
                let intent = match self.board.nodes.get(&id) {
                    Some(node) => Intent::SetText {
                        id: id.clone(),
                        text: format!("{}{c}", node.text),
                        size: node.size,
                    },
                    None => {
                        let edge = &self.board.edges[&id];
                        let text = edge.labels.first().map_or("", |label| &label.text);
                        Intent::SetEdgeLabel {
                            id: id.clone(),
                            text: format!("{text}{c}"),
                            commit: false,
                        }
                    }
                };
                let plan = self.plan(&intent);
                let before = self.snapshot();
                self.board.apply(&plan.changes);
                self.history.record(before, Unit::Label, self.now);
            }
            State::Connecting { target, query } => {
                self.count.query += 1;
                query.push(c);
                *target = walk::query(&self.board, query);
            }
            state => panic!("typed {c:?} in {state:?}"),
        }
    }

    fn commit_label(&mut self, id: &Id) {
        let text = match self.board.nodes.get(id) {
            Some(node) => node.text.clone(),
            None => self.board.edges[id].labels[0].text.clone(),
        };
        assert_eq!(edit::trim_label(&text), text, "nothing to trim");
        self.history.close_group(&self.board);
        self.state = State::Idle;
    }

    fn finish_connecting(&mut self, target: Option<Id>) {
        self.state = State::Idle;
        let source = self.one_node().unwrap();
        let Some(target) = target else { return };
        if target == source || place::joins(&self.board, &source, &target) {
            return;
        }
        let attach = |node: Id| Endpoint::Node { node, side: None };
        let plan = self.plan(&Intent::AddEdge {
            from: attach(source),
            to: attach(target),
        });
        self.commit(plan);
    }
}

fn arrow(name: &str) -> Option<Dir> {
    match name {
        "left" => Some(Dir::Left),
        "right" => Some(Dir::Right),
        "up" => Some(Dir::Up),
        "down" => Some(Dir::Down),
        _ => None,
    }
}

/// The golden layout: each node's shape and corner in cells relative to
/// Start, by label, and each connector as its ends' labels and its own.
struct Layout {
    script: Vec<String>,
    nodes: BTreeMap<String, (String, [i64; 2])>,
    edges: BTreeSet<(String, String, String)>,
}

fn golden() -> Layout {
    let table: toml::Table = GOLDEN.parse().unwrap();
    let text = |value: &toml::Value| value.as_str().unwrap().to_owned();
    let items = |key: &str| table[key].as_array().unwrap().clone();
    let script = items("script").iter().map(text).collect();
    let nodes = items("node")
        .iter()
        .map(|node| {
            let at = node["at"].as_array().unwrap();
            let at = [at[0].as_integer().unwrap(), at[1].as_integer().unwrap()];
            (text(&node["label"]), (text(&node["shape"]), at))
        })
        .collect();
    let edges = items("edge")
        .iter()
        .map(|edge| {
            let label = edge.get("label").map_or(String::new(), text);
            (text(&edge["from"]), text(&edge["to"]), label)
        })
        .collect();
    Layout {
        script,
        nodes,
        edges,
    }
}

fn label_of<'a>(board: &'a Board, end: &End) -> &'a str {
    match end {
        End::Node { node, .. } => &board.nodes[node.as_str()].text,
        End::Free { .. } => panic!("a free end"),
    }
}

fn replay(seed: u64) {
    let layout = golden();
    let mut driver = Driver::new(seed);
    for key in &layout.script {
        driver.press(key);
    }
    let board = &driver.board;

    // 232 key presses: 153 label characters, 9 query letters, 70 chords.
    assert_eq!(
        driver.count,
        Count {
            label: 153,
            query: 9,
            chords: 70
        }
    );
    assert_eq!((board.nodes.len(), board.edges.len()), (15, 17));

    let start = board.nodes.values().find(|n| n.text == "Start").unwrap().at;
    let mut nodes = BTreeMap::new();
    for node in board.nodes.values() {
        assert_eq!(node.size, place::SIZE, "{}", node.text);
        assert_eq!(node.parent, None);
        assert_eq!(node.kind, Some(Named::Known(NodeKind::Shape)));
        assert!(node.at.iter().chain(&node.size).all(|v| v % GRID == 0));
        let cells = [0, 1].map(|i| i64::from(node.at[i] - start[i]) / i64::from(GRID));
        nodes.insert(node.text.clone(), (node.shape.as_str().to_owned(), cells));
    }
    assert_eq!(nodes, layout.nodes);

    let edges: BTreeSet<(String, String, String)> = board
        .edges
        .values()
        .map(|edge| {
            for end in [&edge.from, &edge.to] {
                // No keyboard command writes a side.
                assert!(matches!(
                    end,
                    End::Node {
                        side: None,
                        anchor: None,
                        ..
                    }
                ));
            }
            let label = edge
                .labels
                .first()
                .map_or(String::new(), |l| l.text.clone());
            (
                label_of(board, &edge.from).to_owned(),
                label_of(board, &edge.to).to_owned(),
                label,
            )
        })
        .collect();
    assert_eq!(edges, layout.edges);

    let boxes = place::boxes(board);
    for (a, x) in &boxes {
        for (b, y) in &boxes {
            let apart =
                x.right() <= y.x || y.right() <= x.x || x.bottom() <= y.y || y.bottom() <= x.y;
            assert!(a == b || apart, "{a} and {b} overlap");
        }
    }

    // Idle, with the last connector, Reject order to End, selected.
    assert_eq!(driver.state, State::Idle);
    let [last] = driver.selection.as_slice() else {
        panic!("{:?}", driver.selection)
    };
    let edge = &board.edges[last];
    assert_eq!(
        (label_of(board, &edge.from), label_of(board, &edge.to)),
        ("Reject order", "End")
    );

    let written = write(board);
    assert_eq!(write(&parse(&written).unwrap()), written);

    // One undo step per unit of §5.11, back to the empty board: fifteen
    // creations, each with its label, six connector labels and three
    // connections made by query.
    let mut current = driver.snapshot();
    let mut steps = 0;
    while driver.history.can_undo() {
        current = driver.history.undo(current).unwrap();
        steps += 1;
    }
    assert_eq!(write(&current.board), EMPTY);
    assert_eq!(steps, 24);
}

#[test]
fn the_acceptance_script_builds_the_models_layout() {
    replay(1);
}

/// Ids change nothing but the ids: the layout holds for other seeds.
#[test]
fn the_layout_does_not_depend_on_the_ids() {
    for seed in [2, 3, 0xDEAD_BEEF] {
        replay(seed);
    }
}
