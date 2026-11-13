//! Board undo and redo (§3.4 rule 6 and §5.11 of the board plan).
//!
//! An entry is a whole-board snapshot with the selection and cursor from
//! before its unit, bounded by [`MAX_ENTRIES`] and an estimate of the bytes
//! held, as the text editor's history is. Time is passed in.
//!
//! Units are made three ways. [`History::record`] makes one per call,
//! except that a run of moves, or of resizes, coalesces while the selection
//! stays the same and no other unit and no pause of [`RUN_PAUSE`] comes
//! between ([`History::break_run`] ends one at once, for another command or
//! pointer input). A group, from [`History::open_group`] or the first
//! [`Unit::Label`] record, holds everything until [`History::close_group`]:
//! a creation and its label, or one label edit, is then one unit, and a
//! group that nets to no change records nothing.

use std::collections::{BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use toml::Value;

use crate::{write, Board, Color, End, Extra, Id};

/// The most entries kept, undo and redo together.
pub const MAX_ENTRIES: usize = 200;

/// The most bytes the kept snapshots may hold, by [`Snapshot::bytes`].
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

/// The longest pause inside a run of move or resize keys (§5.11 rule 3).
pub const RUN_PAUSE: Duration = Duration::from_millis(1_000);

/// A state to return to.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub board: Board,
    pub selection: BTreeSet<Id>,
    pub cursor: Option<Id>,
}

impl Snapshot {
    /// An estimate of the bytes the snapshot holds: its strings and keys,
    /// plus a fixed amount for each record and value.
    pub fn bytes(&self) -> usize {
        const RECORD: usize = 256;
        let board = &self.board;
        let nodes: usize = board
            .nodes
            .iter()
            .map(|(id, node)| {
                RECORD
                    + id.as_str().len()
                    + node.text.len()
                    + node.parent.as_ref().map_or(0, String::len)
                    + node.shape.as_str().len()
                    + color(&node.style.color)
                    + node.style.fill.as_str().len()
                    + node.style.stroke.as_str().len()
                    + node.z.as_ref().map_or(0, String::len)
                    + table(&node.style.extra)
                    + table(&node.extra)
            })
            .sum();
        let edges: usize = board
            .edges
            .iter()
            .map(|(id, edge)| {
                RECORD
                    + id.as_str().len()
                    + end(&edge.from)
                    + end(&edge.to)
                    + edge
                        .labels
                        .iter()
                        .map(|label| RECORD + label.text.len() + table(&label.extra))
                        .sum::<usize>()
                    + color(&edge.style.color)
                    + edge.z.as_ref().map_or(0, String::len)
                    + table(&edge.style.extra)
                    + table(&edge.extra)
            })
            .sum();
        let ids = self.selection.iter().chain(&self.cursor);
        RECORD
            + nodes
            + edges
            + table(&board.extra)
            + board.requires.iter().map(String::len).sum::<usize>()
            + ids.map(|id| id.as_str().len() + 32).sum::<usize>()
    }
}

fn color(color: &Color) -> usize {
    match color {
        Color::Unknown(text) => text.len(),
        Color::Token(_) | Color::Hex(_) => 0,
    }
}

fn end(end: &End) -> usize {
    match end {
        End::Node {
            node, side, extra, ..
        } => node.len() + side.as_ref().map_or(0, |side| side.as_str().len()) + table(extra),
        End::Free { extra, .. } => table(extra),
    }
}

fn table(table: &Extra) -> usize {
    table
        .iter()
        .map(|(key, value)| 32 + key.len() + self::value(value))
        .sum()
}

fn value(value: &Value) -> usize {
    16 + match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(self::value).sum(),
        Value::Table(inner) => table(inner),
        Value::Integer(_) | Value::Float(_) | Value::Boolean(_) | Value::Datetime(_) => 0,
    }
}

/// How a recorded change joins the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A unit of its own.
    One,
    /// A `board.move.*` key: a run of them is one unit.
    Move,
    /// A `board.resize.*` key: a run of them is one unit.
    Resize,
    /// A label's text change: it joins the open group, or opens one.
    Label,
}

#[derive(Clone, Debug)]
struct Entry {
    snapshot: Snapshot,
    bytes: usize,
}

impl Entry {
    fn new(snapshot: Snapshot) -> Self {
        let bytes = snapshot.bytes();
        Self { snapshot, bytes }
    }
}

/// A run of move or resize keys still open.
#[derive(Clone, Debug)]
struct Run {
    unit: Unit,
    selection: BTreeSet<Id>,
    last: Instant,
}

impl Run {
    fn joins(&self, unit: Unit, selection: &BTreeSet<Id>, now: Instant) -> bool {
        self.unit == unit
            && self.selection == *selection
            && now.saturating_duration_since(self.last) <= RUN_PAUSE
    }
}

/// The undo and redo stacks of one open board.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    group: Option<Entry>,
    run: Option<Run>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a change already made: `before` is the state before it.
    /// Call it only for a plan that changed something. A new unit clears
    /// the redo side; a record that joins the open group or the open run
    /// adds nothing.
    pub fn record(&mut self, before: Snapshot, unit: Unit, now: Instant) {
        let selection = before.selection.clone();
        self.record_with(unit, &selection, now, || before);
    }

    /// As [`History::record`], with `selection` the selection before the
    /// change, but the state before it is asked of `before` only when an
    /// entry keeps it: a change that joins the open group or the open run
    /// takes none, so a held move key does not copy a large board at every
    /// repeat.
    pub fn record_with(
        &mut self,
        unit: Unit,
        selection: &BTreeSet<Id>,
        now: Instant,
        before: impl FnOnce() -> Snapshot,
    ) {
        if unit == Unit::Label {
            self.run = None;
            if self.group.is_none() {
                self.open(before());
            }
            return;
        }
        if self.group.is_none() {
            if let Some(run) = &mut self.run {
                if run.joins(unit, selection, now) {
                    run.last = now;
                    return;
                }
            }
        }
        let before = before();
        self.close_group(&before.board);
        if let Some(run) = &mut self.run {
            if run.joins(unit, &before.selection, now) {
                run.last = now;
                return;
            }
        }
        self.run = matches!(unit, Unit::Move | Unit::Resize).then(|| Run {
            unit,
            selection: before.selection.clone(),
            last: now,
        });
        self.push(Entry::new(before));
    }

    /// Opens a group whose units become one, from `before`: a creation
    /// that opens the label editor (§3.4 rule 4). A group still open is
    /// closed first, as at `before`.
    pub fn open_group(&mut self, before: Snapshot) {
        self.close_group(&before.board);
        self.run = None;
        self.open(before);
    }

    fn open(&mut self, before: Snapshot) {
        let entry = Entry::new(before);
        if entry.bytes > MAX_BYTES {
            self.clear();
            return;
        }
        self.redo.clear();
        self.group = Some(entry);
        self.enforce_limits();
    }

    /// Closes the open group, with `current` the board now: one unit, or
    /// nothing when the board is as it was when the group opened.
    pub fn close_group(&mut self, current: &Board) {
        if let Some(group) = self.group.take() {
            if !same(&group.snapshot.board, current) {
                self.undo.push_back(group);
                self.enforce_limits();
            }
        }
    }

    /// Ends the open run of move or resize keys, so the next starts a new
    /// unit.
    pub fn break_run(&mut self) {
        self.run = None;
    }

    /// Whether a group is open.
    pub fn group_open(&self) -> bool {
        self.group.is_some()
    }

    /// The state to go back to, keeping `current` for redo; `None` when
    /// there is nothing to undo. An open group is closed first.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        self.close_group(&current.board);
        self.run = None;
        let entry = self.undo.pop_back()?;
        let current = Entry::new(current);
        if current.bytes > MAX_BYTES {
            // Without this state, older redo entries would skip a step.
            self.redo.clear();
        } else {
            self.redo.push(current);
            self.enforce_limits();
        }
        Some(entry.snapshot)
    }

    /// The state to go forward to, keeping `current` for undo; `None` when
    /// there is nothing to redo.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        self.close_group(&current.board);
        self.run = None;
        let entry = self.redo.pop()?;
        let current = Entry::new(current);
        if current.bytes > MAX_BYTES {
            self.undo.clear();
        } else {
            self.undo.push_back(current);
            self.enforce_limits();
        }
        Some(entry.snapshot)
    }

    /// Drops everything: another note opened, or the board changed
    /// underneath (§5.11 rule 6).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.group.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn push(&mut self, entry: Entry) {
        if entry.bytes > MAX_BYTES {
            self.clear();
            return;
        }
        self.redo.clear();
        self.undo.push_back(entry);
        self.enforce_limits();
    }

    fn bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .chain(&self.group)
            .map(|entry| entry.bytes)
            .sum()
    }

    fn enforce_limits(&mut self) {
        while self.undo.len() + self.redo.len() + usize::from(self.group.is_some()) > MAX_ENTRIES
            || self.bytes() > MAX_BYTES
        {
            // Keep the nearest states on each side; dropping from the other
            // end would make undo or redo jump over a state.
            if self.undo.pop_front().is_some() {
                continue;
            }
            if self.redo.is_empty() {
                // Only the group is left, and it fits by itself.
                break;
            }
            self.redo.remove(0);
        }
    }
}

/// Whether two boards are the same model: equal, or with the same
/// canonical bytes, which also holds where unknown content holds a NaN
/// (§4.13).
fn same(a: &Board, b: &Board) -> bool {
    a == b || write(a) == write(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    /// A board whose one node's label is `label`.
    fn board(label: &str) -> Board {
        let mut board = parse(
            "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [20, 20]\nz = \"V\"\n",
        )
        .unwrap();
        board.nodes.get_mut("a").unwrap().text = label.into();
        board
    }

    fn state(label: &str, selected: &[&str]) -> Snapshot {
        let selection: BTreeSet<Id> = selected.iter().map(|id| Id::new(id).unwrap()).collect();
        Snapshot {
            board: board(label),
            cursor: selection.iter().next_back().cloned(),
            selection,
        }
    }

    fn label(snapshot: &Snapshot) -> &str {
        &snapshot.board.nodes["a"].text
    }

    /// Undoes everything from `current`, giving each label met.
    fn undo_all(history: &mut History, current: &str) -> Vec<String> {
        let mut current = state(current, &[]);
        let mut met = Vec::new();
        while let Some(before) = history.undo(current) {
            met.push(label(&before).to_owned());
            current = before;
        }
        met
    }

    #[test]
    fn units_undo_and_redo_in_order() {
        let now = Instant::now();
        let mut history = History::new();
        assert!(!history.can_undo() && !history.can_redo());
        for (before, after) in [("0", "1"), ("1", "2"), ("2", "3")] {
            let _ = after;
            history.record(state(before, &["a"]), Unit::One, now);
        }
        let back = history.undo(state("3", &[])).unwrap();
        assert_eq!(
            (label(&back), &back.selection),
            ("2", &state("", &["a"]).selection)
        );
        assert_eq!(back.cursor, Some(Id::new("a").unwrap()));
        let back = history.undo(back).unwrap();
        assert_eq!(label(&back), "1");
        assert!(history.can_redo());
        let forward = history.redo(back).unwrap();
        assert_eq!(label(&forward), "2");
        // A new unit clears the redo side.
        history.record(forward, Unit::One, now);
        assert!(!history.can_redo());
        assert_eq!(undo_all(&mut history, "x"), ["2", "1", "0"]);
        assert!(history.can_redo() && !history.can_undo());
        assert!(history.undo(state("0", &[])).is_none());
    }

    #[test]
    fn a_run_of_moves_is_one_unit_until_something_comes_between() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut history = History::new();
        // Three moves of one selection, each within a second of the last.
        history.record(state("0", &["a"]), Unit::Move, at(0));
        history.record(state("1", &["a"]), Unit::Move, at(900));
        history.record(state("2", &["a"]), Unit::Move, at(1_800));
        // A pause of more than a second.
        history.record(state("3", &["a"]), Unit::Move, at(2_801));
        // A resize is a run of its own.
        history.record(state("4", &["a"]), Unit::Resize, at(2_900));
        history.record(state("5", &["a"]), Unit::Resize, at(3_000));
        // Another selection.
        history.record(state("6", &["b"]), Unit::Resize, at(3_100));
        // Another command or pointer input.
        history.break_run();
        history.record(state("7", &["b"]), Unit::Resize, at(3_200));
        history.record(state("8", &["b"]), Unit::One, at(3_300));
        history.record(state("9", &["b"]), Unit::Move, at(3_400));
        assert_eq!(
            undo_all(&mut history, "10"),
            ["9", "8", "7", "6", "4", "3", "0"]
        );
    }

    #[test]
    fn a_run_and_a_group_take_the_state_before_only_once() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut history = History::new();
        let selection: BTreeSet<Id> = [Id::new("a").unwrap()].into();
        let asked = std::cell::Cell::new(0);
        let mut record = |label: &str, unit: Unit, ms: u64| {
            history.record_with(unit, &selection, at(ms), || {
                asked.set(asked.get() + 1);
                state(label, &["a"])
            });
        };
        for (k, label) in ["0", "1", "2", "3", "4"].into_iter().enumerate() {
            record(label, Unit::Move, 100 * k as u64);
        }
        assert_eq!(asked.get(), 1, "a run of five moves");
        for (k, label) in ["5", "6", "7"].into_iter().enumerate() {
            record(label, Unit::Label, 1_000 + 100 * k as u64);
        }
        assert_eq!(asked.get(), 2, "and a label's three changes");
        history.close_group(&board("8"));
        assert_eq!(undo_all(&mut history, "8"), ["5", "0"]);
    }

    #[test]
    fn undo_ends_a_run() {
        let now = Instant::now();
        let mut history = History::new();
        history.record(state("0", &["a"]), Unit::Move, now);
        let back = history.undo(state("1", &["a"])).unwrap();
        history.record(back, Unit::Move, now);
        assert_eq!(undo_all(&mut history, "2"), ["0"]);
    }

    #[test]
    fn a_creation_and_its_label_are_one_unit() {
        let now = Instant::now();
        let mut history = History::new();
        history.record(state("first", &[]), Unit::One, now);
        // The creation opens a group; each keystroke joins it.
        history.open_group(state("", &[]));
        assert!(history.group_open() && history.can_undo());
        for typed in ["N", "Ne"] {
            history.record(state(typed, &["a"]), Unit::Label, now);
        }
        history.close_group(&board("New"));
        assert!(!history.group_open());
        assert_eq!(undo_all(&mut history, "New"), ["", "first"]);
    }

    #[test]
    fn a_label_edit_opens_its_own_group() {
        let now = Instant::now();
        let mut history = History::new();
        history.record(state("Old", &["a"]), Unit::Label, now);
        history.record(state("Ol", &["a"]), Unit::Label, now);
        // Another unit closes the group first: two units.
        history.record(state("New", &["a"]), Unit::One, now);
        assert_eq!(undo_all(&mut history, "Newer"), ["New", "Old"]);
    }

    #[test]
    fn a_group_that_nets_to_no_change_records_nothing() {
        let now = Instant::now();
        let mut history = History::new();
        history.record(state("first", &[]), Unit::One, now);
        history.record(state("same", &["a"]), Unit::Label, now);
        history.record(state("sam", &["a"]), Unit::Label, now);
        history.close_group(&board("same"));
        assert_eq!(undo_all(&mut history, "same"), ["first"]);
        // Also when unknown content holds a NaN, which is never equal to
        // itself.
        let mut nan = board("same");
        nan.extra.insert("ext".into(), Value::Float(f64::NAN));
        history.record(state("x", &[]), Unit::One, now);
        history.open_group(Snapshot {
            board: nan.clone(),
            selection: BTreeSet::new(),
            cursor: None,
        });
        history.close_group(&nan);
        assert_eq!(undo_all(&mut history, "same"), ["x"]);
        // Undo closes an open group, as it is.
        history.clear();
        history.record(state("a", &[]), Unit::Label, now);
        let back = history.undo(state("b", &[])).unwrap();
        assert_eq!(label(&back), "a");
    }

    #[test]
    fn opening_a_group_closes_the_one_open() {
        let mut history = History::new();
        history.open_group(state("0", &[]));
        history.open_group(state("1", &[]));
        history.close_group(&board("2"));
        assert_eq!(undo_all(&mut history, "2"), ["1", "0"]);
    }

    #[test]
    fn at_most_two_hundred_entries_are_kept() {
        let now = Instant::now();
        let mut history = History::new();
        for k in 0..250 {
            history.record(state(&k.to_string(), &[]), Unit::One, now);
        }
        let met = undo_all(&mut history, "250");
        assert_eq!(met.len(), MAX_ENTRIES);
        assert_eq!(met.last().map(String::as_str), Some("50"));
        // Redo keeps the nearest states when both sides are full.
        let mut current = state("50", &[]);
        let (mut forward, mut last) = (0, String::new());
        while let Some(next) = history.redo(current) {
            forward += 1;
            last = label(&next).to_owned();
            current = next;
        }
        assert_eq!((forward, last.as_str()), (MAX_ENTRIES, "250"));
    }

    #[test]
    fn the_byte_budget_drops_the_oldest_and_refuses_a_huge_one() {
        let now = Instant::now();
        let mut history = History::new();
        let big = "x".repeat(4 * 1024 * 1024);
        for k in 0..12 {
            history.record(state(&format!("{k}{big}"), &[]), Unit::One, now);
        }
        let met = undo_all(&mut history, "end");
        // Seven of about four MiB fit in 32 MiB, with the state undone to.
        assert!((6..=7).contains(&met.len()), "{}", met.len());
        assert!(met
            .last()
            .unwrap()
            .starts_with(&(12 - met.len()).to_string()));
        // A snapshot over the whole budget clears the history rather than
        // let undo skip it.
        let mut history = History::new();
        history.record(state("small", &[]), Unit::One, now);
        history.record(state(&"x".repeat(MAX_BYTES), &[]), Unit::One, now);
        assert!(!history.can_undo() && !history.can_redo());
    }

    #[test]
    fn the_estimate_counts_what_a_snapshot_holds() {
        let small = state("", &[]).bytes();
        let labelled = state(&"x".repeat(1_000), &[]).bytes();
        assert_eq!(labelled - small, 1_000);
        let mut extra = state("", &[]);
        extra.board.nodes.get_mut("a").unwrap().extra.insert(
            "ext".into(),
            Value::Array(vec![Value::String("y".repeat(500)); 4]),
        );
        assert!(extra.bytes() >= small + 2_000);
    }
}
