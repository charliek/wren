//! The acceptance script of the plan's §5.20, replayed as a unit test of
//! the machine: every key goes through [`BoardView::key`], as the widget
//! sends it, and what the board sends back through [`BoardView::update`],
//! on a new empty board. The policy stands in for the host's keymap with
//! the default board bindings the script reaches, and the tool and shape
//! letters, so that a label or the query typed with them still types. The
//! result must be the keyboard model's layout, which `wren-board` keeps in
//! `golden/accept/checkout-flow.toml` with the script itself.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::keyboard::key::{Named, NativeCode, Physical};
use iced::keyboard::{Key as Pressed, Modifiers};
use wren_board::{Board, Dir, End, Named as Value, NodeKind, Shape, EMPTY, GRID};

use crate::command::{Command, Tool};
use crate::machine::State;
use crate::{fixtures, BoardView, KeyPolicy, KeyRoute, Message};

const GOLDEN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../wren-board/golden/accept/checkout-flow.toml"
));

/// The model's layout: the script, each node's shape and corner in cells
/// relative to Start by label, and each connector by its ends' labels and
/// its own.
struct Layout {
    script: Vec<String>,
    nodes: BTreeMap<String, (String, [i32; 2])>,
    edges: BTreeSet<(String, String, String)>,
}

/// The golden file, read with no TOML parser: it is data this crate's
/// tests share with `wren-board`'s, and this crate depends on no `toml`.
fn golden() -> Layout {
    let quoted = |text: &str| -> Vec<String> {
        text.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    };
    let (_, rest) = GOLDEN.split_once("script = [").expect("a script");
    let (script, records) = rest.split_once("\n]").expect("the script's end");
    let mut layout = Layout {
        script: quoted(script),
        nodes: BTreeMap::new(),
        edges: BTreeSet::new(),
    };
    for record in records.split("[[").skip(1) {
        let (kind, body) = record.split_once("]]").expect("a record");
        let field = |name: &str| {
            body.lines()
                .find_map(|line| line.strip_prefix(&format!("{name} = ")))
                .map(str::to_owned)
        };
        let text = |name: &str| field(name).map(|value| quoted(&value).remove(0));
        match kind {
            "node" => {
                let at = field("at").expect("at");
                let cells: Vec<i32> = at
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|cell| cell.trim().parse().expect("a cell"))
                    .collect();
                let label = text("label").expect("a label");
                let shape = text("shape").expect("a shape");
                layout.nodes.insert(label, (shape, [cells[0], cells[1]]));
            }
            "edge" => {
                layout.edges.insert((
                    text("from").expect("from"),
                    text("to").expect("to"),
                    text("label").unwrap_or_default(),
                ));
            }
            other => panic!("a {other} record"),
        }
    }
    layout
}

/// The default board bindings (§5.19) of the keys the script presses, and
/// the tool and shape letters.
fn policy() -> KeyPolicy {
    Arc::new(|key, _, modifiers, repeat| {
        let (shift, command) = (modifiers.shift(), modifiers.command());
        if modifiers.alt() || (modifiers.control() && !command) {
            return KeyRoute::Host;
        }
        let arrow = |named: &Named| match named {
            Named::ArrowLeft => Some(Dir::Left),
            Named::ArrowRight => Some(Dir::Right),
            Named::ArrowUp => Some(Dir::Up),
            Named::ArrowDown => Some(Dir::Down),
            _ => None,
        };
        let route = match key {
            Pressed::Named(Named::Tab) if !shift && !command && !repeat => {
                Some(Command::ConnectNext)
            }
            Pressed::Named(Named::Enter) if command => Some(Command::CommitLabel),
            Pressed::Named(Named::Enter) if !shift && !repeat => Some(Command::EditLabel),
            Pressed::Named(Named::Backspace | Named::Delete) if !repeat => Some(Command::Delete),
            Pressed::Named(named) => arrow(named).map(|dir| match (shift, command) {
                (false, false) => Command::Select(dir),
                (true, false) => Command::ExtendSelection(dir),
                (false, true) => Command::Connect(dir),
                (true, true) => Command::SelectConnector(dir),
            }),
            Pressed::Character(c) if command => match (c.to_lowercase().as_str(), shift) {
                ("z", false) => Some(Command::Undo),
                ("z", true) => Some(Command::Redo),
                _ => None,
            },
            Pressed::Character(c) if !repeat => {
                let shape = match c.to_lowercase().as_str() {
                    "r" => Some(Shape::Rect),
                    "o" => Some(Shape::Ellipse),
                    "d" => Some(Shape::Diamond),
                    "u" => Some(Shape::Pill),
                    "y" => Some(Shape::Cylinder),
                    "p" => Some(Shape::Parallelogram),
                    _ => None,
                };
                match (c.to_lowercase().as_str(), shape, shift) {
                    (_, Some(shape), true) => Some(Command::Shape(shape)),
                    (_, Some(shape), false) => Some(Command::Tool(Tool::Shape(shape))),
                    ("v", _, false) => Some(Command::Tool(Tool::Select)),
                    ("h", _, false) => Some(Command::Tool(Tool::Hand)),
                    ("t", _, false) => Some(Command::Tool(Tool::Text)),
                    ("c" | "a", _, false) => Some(Command::Tool(Tool::Connector)),
                    _ => None,
                }
            }
            _ => None,
        };
        route.map_or(KeyRoute::Pass, KeyRoute::Command)
    })
}

/// What the script's keys were.
#[derive(Debug, Default, PartialEq)]
struct Count {
    label: usize,
    query: usize,
    chords: usize,
}

struct Player {
    view: BoardView,
    now: Instant,
    count: Count,
}

impl Player {
    fn new() -> Self {
        fixtures::load_fonts();
        let mut view =
            BoardView::open(EMPTY, fixtures::palette(), fixtures::FONT).expect("the empty board");
        let now = Instant::now();
        view.update(
            Message::Resized {
                width: 1000.0,
                height: 600.0,
            },
            now,
        );
        view.set_key_policy(policy());
        let epoch = view.epoch();
        view.update(Message::Input(epoch, crate::Input::Focus(true)), now);
        Self {
            view,
            now,
            count: Count::default(),
        }
    }

    /// Press `key` and send on what the board makes of it; it must be the
    /// board's.
    fn press(&mut self, key: Pressed, modifiers: Modifiers, text: Option<&str>) {
        self.now += Duration::from_millis(60);
        let physical = Physical::Unidentified(NativeCode::Unidentified);
        let sent = self
            .view
            .key(&key, physical, modifiers, text, false, false)
            .unwrap_or_else(|| panic!("{key:?} with {modifiers:?} is the host's"));
        for message in sent {
            self.view.update(message, self.now);
        }
    }

    /// One step of the golden script: a key after any of the modifiers
    /// `shift` and `mod`, or `type:` and text typed one key at a time.
    fn step(&mut self, step: &str) {
        if let Some(text) = step.strip_prefix("type:") {
            for c in text.chars() {
                match self.view.machine().state() {
                    State::Editing => self.count.label += 1,
                    State::Connecting => self.count.query += 1,
                    state => panic!("typed {c:?} in {state:?}"),
                }
                let typed = c.to_string();
                let (key, modifiers) = match c {
                    ' ' => (Pressed::Named(Named::Space), Modifiers::empty()),
                    c if c.is_uppercase() => {
                        (Pressed::Character(typed.as_str().into()), Modifiers::SHIFT)
                    }
                    _ => (
                        Pressed::Character(typed.as_str().into()),
                        Modifiers::empty(),
                    ),
                };
                self.press(key, modifiers, Some(&typed));
            }
            return;
        }
        self.count.chords += 1;
        let (mods, name) = step.rsplit_once('+').unwrap_or(("", step));
        let mut modifiers = Modifiers::empty();
        for part in mods.split('+').filter(|part| !part.is_empty()) {
            modifiers |= match part {
                "shift" => Modifiers::SHIFT,
                "mod" => Modifiers::COMMAND,
                other => panic!("no modifier {other}"),
            };
        }
        let key = match name {
            "tab" => Pressed::Named(Named::Tab),
            "enter" => Pressed::Named(Named::Enter),
            "escape" => Pressed::Named(Named::Escape),
            "left" => Pressed::Named(Named::ArrowLeft),
            "right" => Pressed::Named(Named::ArrowRight),
            "up" => Pressed::Named(Named::ArrowUp),
            "down" => Pressed::Named(Named::ArrowDown),
            letter => Pressed::Character(letter.into()),
        };
        let text = matches!(key, Pressed::Character(_)).then_some(name);
        self.press(key, modifiers, text);
    }
}

fn label_of<'a>(board: &'a Board, end: &End) -> &'a str {
    match end {
        End::Node { node, .. } => &board.nodes[node.as_str()].text,
        End::Free { .. } => panic!("a free end"),
    }
}

#[test]
fn the_acceptance_script_builds_the_flowchart_by_keyboard() {
    let layout = golden();
    let mut player = Player::new();
    for step in &layout.script {
        player.step(step);
    }

    // 232 key presses: 153 label characters, 9 query letters, 70 chords.
    assert_eq!(
        player.count,
        Count {
            label: 153,
            query: 9,
            chords: 70
        }
    );
    let view = &player.view;
    let board = view.board();
    assert_eq!((board.nodes.len(), board.edges.len()), (15, 17));

    // Every node still 120 by 60 and on the grid, where the model put it
    // relative to Start.
    let start = board.nodes.values().find(|n| n.text == "Start").unwrap().at;
    let mut nodes = BTreeMap::new();
    for node in board.nodes.values() {
        assert_eq!(node.size, [120, 60], "{}", node.text);
        assert_eq!(node.parent, None);
        assert_eq!(node.kind, Some(Value::Known(NodeKind::Shape)));
        assert!(node.at.iter().all(|v| v % GRID == 0), "{}", node.text);
        let cells = [0, 1].map(|i| (node.at[i] - start[i]) / GRID);
        nodes.insert(node.text.clone(), (node.shape.as_str().to_owned(), cells));
    }
    assert_eq!(nodes, layout.nodes);
    let shapes = |shape: Shape| {
        board
            .nodes
            .values()
            .filter(|n| n.shape == Value::Known(shape))
            .count()
    };
    assert_eq!(
        (
            shapes(Shape::Pill),
            shapes(Shape::Diamond),
            shapes(Shape::Rect)
        ),
        (2, 3, 10)
    );

    // No two node bounds intersect: none has a parent, so `at` is where
    // it is.
    for (a, x) in &board.nodes {
        for (b, y) in &board.nodes {
            let apart =
                (0..2).any(|i| x.at[i] + x.size[i] <= y.at[i] || y.at[i] + y.size[i] <= x.at[i]);
            assert!(a == b || apart, "{} and {} overlap", x.text, y.text);
        }
    }

    // Every end attached to a node, floating; the six branches labelled.
    let edges: BTreeSet<(String, String, String)> = board
        .edges
        .values()
        .map(|edge| {
            for end in [&edge.from, &edge.to] {
                assert!(
                    matches!(end, End::Node { node, side: None, anchor: None, .. }
                        if board.nodes.contains_key(node.as_str())),
                    "{end:?}"
                );
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
    let branches: Vec<&str> = edges
        .iter()
        .map(|(_, _, label)| label.as_str())
        .filter(|label| !label.is_empty())
        .collect();
    assert_eq!(branches.len(), 6);
    assert!(branches.iter().all(|label| ["Yes", "No"].contains(label)));

    // Both loops are cycles along the connectors.
    let next: BTreeMap<&str, Vec<&str>> =
        edges
            .iter()
            .fold(BTreeMap::new(), |mut next, (from, to, _)| {
                next.entry(from.as_str()).or_default().push(to.as_str());
                next
            });
    let reaches = |from: &str, to: &str| {
        let mut seen = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(at) = stack.pop() {
            for &onward in next.get(at).into_iter().flatten() {
                if onward == to {
                    return true;
                }
                if seen.insert(onward) {
                    stack.push(onward);
                }
            }
        }
        false
    };
    for (a, b) in [
        ("Check stock", "Notify customer"),
        ("Charge card", "Retry payment"),
    ] {
        assert!(reaches(a, b) && reaches(b, a), "{a} and {b}");
    }

    // Idle, with the last connector, Reject order to End, selected.
    assert_eq!(view.machine().state(), State::Idle);
    let selected: Vec<_> = view.machine().selection().iter().collect();
    let [last] = selected[..] else {
        panic!("{selected:?}")
    };
    let edge = &board.edges[last];
    assert_eq!(
        (label_of(board, &edge.from), label_of(board, &edge.to)),
        ("Reject order", "End")
    );

    // The draft is canonical and reads back as the same board.
    let mut view = player.view;
    let draft = view.draft();
    assert_eq!(draft, wren_board::write(view.board()));
    let reread = wren_board::parse(&draft).expect("the draft parses");
    assert_eq!(wren_board::write(&reread), draft);

    // Undoing every unit gives the canonical empty board: fifteen
    // creations, each with its label, six connector labels and three
    // connections made by query (§5.11 rule 2).
    let mut now = player.now;
    let mut steps = 0;
    while view.can_undo() {
        now += Duration::from_millis(60);
        view.command(Command::Undo, now);
        steps += 1;
    }
    assert_eq!(view.draft(), EMPTY);
    assert_eq!(steps, 24);
    assert!(view.pristine());
}
