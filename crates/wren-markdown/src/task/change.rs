//! The line edit: one [`Change`] to one task line, written back as one
//! replacement that carries the line it was read from.
//!
//! Every writer of a task line goes through [`edit`]: the toggle key, the
//! status and priority menus, the task form, auto-suggest, and writes from
//! a view to a note that is not open. A change is applied to the line as
//! [`TaskLine`]'s writers apply it, one after another for a
//! [`Change::Compound`], and the result is a single [`Replace`], so an
//! editor makes it one undo step. The [`LineEdit`] keeps the line's text
//! as it was read, and [`LineEdit::plan_for`] refuses the plan once the
//! line has changed.
//!
//! # Statuses and stamps
//!
//! A done stamp (`✅`) and a cancelled stamp (`❌`) follow the status's
//! *type*, as Obsidian Tasks' do:
//!
//! - Into done, the line gets `✅` and today's date and loses every `❌`;
//!   into cancelled, the other way round; into open or doing, it loses
//!   both. A stamp that is there is re-dated in place, and the others of
//!   its kind go, so the line is left with exactly one. A stamp whose date
//!   is no real one is treated like any other.
//! - A change between two symbols of one type (`[x]` to `[X]`, or `[ ]` to
//!   a symbol with no status, which is open too) changes the box only and
//!   keeps whatever stamps the line has.
//! - Setting the symbol the box already holds changes nothing.
//!
//! Typing a symbol into the box is ordinary typing and never comes here,
//! so it writes no stamp.

use std::fmt;
use std::ops::Range;

use super::{parse, DateField, Edit, Priority, TaskLine, Token};
use crate::block::{self, Boxes};
use crate::date::Date;
use crate::status::{self, StatusType};
use crate::{EditPlan, Lines, Options, Pos, Replace, Selection};

/// A change to a task line.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    /// The done toggle: an open or doing task, or one whose symbol has no
    /// status, becomes done (`x`); a done or cancelled one becomes open.
    ToggleDone,
    /// One step of open → doing → done → open. A cancelled task becomes
    /// open; it is never cycled into.
    Cycle,
    /// The box's symbol. `]` and line breaks are refused.
    SetStatus(char),
    /// The due date, or none. A due date that is there is re-dated in
    /// place, whatever it is written with; none removes every one.
    SetDue(Option<Date>),
    /// The priority, or none. Every indicator on the line is replaced by
    /// one, so this tidies a line with several even to its own level.
    SetPriority(Option<Priority>),
    /// One level up: Lowest, Low, none, Medium, High, Highest. Nothing
    /// changes at Highest.
    RaisePriority,
    /// One level down. Nothing changes at Lowest.
    LowerPriority,
    /// `@name` at the end of the description, unless it is there already.
    /// A name that is not one ([`super::is_owner_name`]) is refused.
    AddOwner(String),
    /// Every `@name` on the line, with the white space before it.
    RemoveOwner(String),
    /// The description, keeping every token after it.
    SetDescription(String),
    /// Bytes `range` of the line, as this step reads it, replaced with
    /// the text: a word typed on the line that auto-suggest completes
    /// (`@sa` to `@sam`), or with no text takes out, as a token is taken
    /// out, with the white space before it. Refused for a range off the
    /// line, inside a character or before the description, and for text
    /// with a line break.
    ReplaceWord(Range<usize>, String),
    /// Each change in order, each reading what the one before wrote, as
    /// one replacement: what the task form sends. It should hold only the
    /// fields the user changed, since setting a priority tidies duplicate
    /// indicators, and a description before owners, since a description's
    /// owners are part of its text.
    Compound(Vec<Change>),
}

/// A change to a task line, ready to apply to the document it was planned
/// on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineEdit {
    /// The line the task's box is on.
    pub line: usize,
    /// The task's box on that line when the edit was planned.
    pub task: block::Task,
    /// That line's text when the edit was planned.
    pub expect: String,
    /// The line as the edit writes it: `expect` when nothing changes.
    pub text: String,
    /// The task's description in [`text`](Self::text).
    pub description: Range<usize>,
    /// No edit when nothing changes, else one [`Replace`] on
    /// [`line`](Self::line), and the selection after it.
    pub plan: EditPlan,
}

/// The line a [`LineEdit`] was planned on has changed since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stale;

impl fmt::Display for Stale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the task line changed")
    }
}

impl std::error::Error for Stale {}

impl LineEdit {
    /// Whether applying the edit changes nothing.
    pub fn is_noop(&self) -> bool {
        self.plan.edits.is_empty()
    }

    /// The plan, while line [`line`](Self::line) of `lines` is still
    /// [`expect`](Self::expect) and still that task, its box where it was:
    /// [`Stale`] once the line has changed or gone, or reads as no task
    /// any more (an opening fence or front matter above it, a lost list).
    /// The plan's selection is the one it was planned at.
    ///
    /// Only the line is compared, so the same text at the same index after
    /// other changes (the first of two equal lines deleted) still takes
    /// the plan, which then writes the very same line. A host that needs
    /// the document itself unchanged checks its own revision.
    pub fn plan_for<L: Lines + ?Sized>(&self, lines: &L) -> Result<&EditPlan, Stale> {
        let holds = self.reads(lines)
            && crate::edit::task_item(lines, self.line, Boxes::Status)
                == Some((self.line, self.task));
        if holds {
            Ok(&self.plan)
        } else {
            Err(Stale)
        }
    }

    /// Whether line [`line`](Self::line) of `lines` is still
    /// [`expect`](Self::expect): the part of [`plan_for`](Self::plan_for)'s
    /// check that reads no other line.
    pub(super) fn reads<L: Lines + ?Sized>(&self, lines: &L) -> bool {
        self.line < lines.line_count() && lines.line(self.line) == self.expect
    }
}

/// Plan `change` to the task that line `line` of `lines` belongs to (the
/// line itself, or the task item it continues), with `sel` the editor's
/// selection.
///
/// `None` with tasks off in `opts`, on a line that belongs to no task,
/// and for a change that is refused (a symbol no box holds, a name that
/// is no owner's). A change that changes nothing is a [`LineEdit`] with no
/// edit.
///
/// The selection's ends on the task's line move with the text around
/// them. An end inside text that was replaced goes to the end of the new
/// text: a caret in a description that is set lands at the end of the new
/// one. An end where text goes in stays before it, so writing a due date
/// at the caret leaves the caret at the end of the description.
pub fn edit<L: Lines + ?Sized>(
    lines: &L,
    line: usize,
    sel: Selection,
    change: &Change,
    opts: &Options,
) -> Option<LineEdit> {
    opts.tasks?;
    let (k, task) = crate::edit::task_item(lines, line, opts.boxes())?;
    edit_task(lines, k, task, sel, change, opts)
}

/// [`edit`] of the task whose box `task` is on line `k`, as
/// `task_item` found it: for a caller that has found many tasks in one
/// scan.
pub(super) fn edit_task<L: Lines + ?Sized>(
    lines: &L,
    k: usize,
    task: block::Task,
    sel: Selection,
    change: &Change,
    opts: &Options,
) -> Option<LineEdit> {
    let today = opts.tasks?.today;
    let first = task;
    let expect = lines.line(k);
    let col = |pos: Pos| (pos.line == k).then(|| floor_char(expect, pos.col));
    let ends = [col(sel.anchor), col(sel.head)];
    let mut carets: Vec<usize> = ends.iter().flatten().copied().collect();
    let mut writer = Writer {
        line: expect.to_owned(),
        task,
        carets: &mut carets,
        today,
    };
    writer.change(change)?;
    let (text, task) = (writer.line, writer.task);
    let description = parse(&text, task)?.description();
    if text == expect {
        return Some(LineEdit {
            line: k,
            task: first,
            expect: text.clone(),
            text,
            description,
            plan: EditPlan {
                edits: Vec::new(),
                selection: sel,
            },
        });
    }
    let mut mapped = carets.into_iter();
    let mut place = |pos: Pos, end: Option<usize>| match end {
        Some(_) => Pos::new(k, mapped.next().unwrap_or(pos.col)),
        None => pos,
    };
    let selection = Selection::new(place(sel.anchor, ends[0]), place(sel.head, ends[1]));
    let edits = vec![replacement(k, expect, &text)];
    Some(LineEdit {
        line: k,
        task: first,
        expect: expect.to_owned(),
        text,
        description,
        plan: EditPlan { edits, selection },
    })
}

/// A task line being rewritten one writer at a time. Each step reads the
/// line again, so a step sees what the one before it wrote.
struct Writer<'c> {
    line: String,
    task: block::Task,
    /// Byte offsets in the line that move with each step's edits.
    carets: &'c mut [usize],
    today: Date,
}

impl Writer<'_> {
    /// Apply `change`, a compound's changes in order. Compounds are
    /// walked with a stack of their own, so no nesting runs out of the
    /// thread's.
    fn change(&mut self, change: &Change) -> Option<()> {
        let mut todo = vec![change];
        while let Some(change) = todo.pop() {
            match change {
                Change::Compound(changes) => todo.extend(changes.iter().rev()),
                change => self.one(change)?,
            }
        }
        Some(())
    }

    fn one(&mut self, change: &Change) -> Option<()> {
        match change {
            Change::ToggleDone => self.status(status::status(self.task.symbol).next),
            Change::Cycle => self.status(cycled(self.task.status)),
            Change::SetStatus(symbol) => self.status(*symbol),
            Change::SetDue(date) => self.step(|r| Some(r.date_edits(DateField::Due, *date))),
            Change::SetPriority(level) => self.step(|r| Some(r.priority_edits(*level))),
            Change::RaisePriority => self.shift_priority(1),
            Change::LowerPriority => self.shift_priority(-1),
            Change::AddOwner(name) => self.step(|r| r.owner_addition(name)),
            Change::RemoveOwner(name) => self.step(|r| Some(r.owner_removals(name))),
            Change::SetDescription(text) => self.step(|r| Some(r.description_edits(text))),
            Change::ReplaceWord(range, text) => self.step(|r| r.word_edits(range, text)),
            Change::Compound(_) => self.change(change),
        }
    }

    /// Make the edits `plan` returns for the line as it reads now.
    fn step(&mut self, plan: impl FnOnce(&TaskLine) -> Option<Vec<Edit>>) -> Option<()> {
        let reading = parse(&self.line, self.task)?;
        let mut edits = plan(&reading)?;
        // An edit that writes what is there changes nothing, and must not
        // move a caret inside it.
        edits.retain(|(range, text)| reading.line().get(range.clone()) != Some(text.as_str()));
        if edits.is_empty() {
            return Some(());
        }
        edits.sort_by_key(|(range, _)| range.start);
        let text = reading.apply(edits.clone());
        // The box never moves, so the line is read again from the same `[`.
        let task = block::task_box(&text, self.task.start, Boxes::Status)?;
        for caret in self.carets.iter_mut() {
            *caret = moved(*caret, &edits);
        }
        self.line = text;
        self.task = task;
        Some(())
    }

    /// The box set to `symbol`, with the stamps its type asks for.
    fn status(&mut self, symbol: char) -> Option<()> {
        let from = self.task.status;
        if symbol == self.task.symbol {
            return Some(());
        }
        if matches!(symbol, ']' | '\n' | '\r') {
            return None;
        }
        self.step(|r| Some(vec![(r.task().symbol_range(), symbol.to_string())]))?;
        let to = status::status(symbol).kind;
        if to == from {
            return Some(());
        }
        let stamp = match to {
            StatusType::Done => Some(DateField::Done),
            StatusType::Cancelled => Some(DateField::Cancelled),
            StatusType::Open | StatusType::Doing => None,
        };
        // Every stamp goes but the last of the kind the new type keeps,
        // which is then re-dated in place; with none, one is written.
        self.step(|r| {
            let keep = stamp
                .and_then(|field| r.date(field))
                .map(|last| last.range.clone());
            let mut ranges: Vec<Range<usize>> = r
                .tokens()
                .iter()
                .filter_map(|token| match token {
                    Token::Date(date) if date.field != DateField::Due => Some(date.range.clone()),
                    _ => None,
                })
                .filter(|range| Some(range) != keep.as_ref())
                .collect();
            ranges.sort_by_key(|range| range.start);
            Some(r.removals(ranges.into_iter()))
        })?;
        let today = self.today;
        match stamp {
            Some(field) => self.step(|r| Some(r.date_edits(field, Some(today)))),
            None => Some(()),
        }
    }

    /// The priority `by` levels up (down when negative), or no change past
    /// either end.
    fn shift_priority(&mut self, by: isize) -> Option<()> {
        let current = parse(&self.line, self.task)?
            .priority()
            .map(|token| token.level);
        let at = LADDER.iter().position(|level| *level == current)?;
        match at.checked_add_signed(by).and_then(|to| LADDER.get(to)) {
            Some(level) => self.step(|r| Some(r.priority_edits(*level))),
            None => Some(()),
        }
    }
}

/// The priorities from the lowest to the highest, with none between Low
/// and Medium (design §5.7).
const LADDER: [Option<Priority>; 6] = [
    Some(Priority::Lowest),
    Some(Priority::Low),
    None,
    Some(Priority::Medium),
    Some(Priority::High),
    Some(Priority::Highest),
];

/// The symbol a cycle writes after a status of type `kind`.
fn cycled(kind: StatusType) -> char {
    let next = match kind {
        StatusType::Open => StatusType::Doing,
        StatusType::Doing => StatusType::Done,
        StatusType::Done | StatusType::Cancelled => StatusType::Open,
    };
    status::STATUSES
        .iter()
        .find(|row| row.kind == next)
        .map_or(status::OPEN, |row| row.symbol)
}

/// Where byte `at` of a line lands after `edits`, which are sorted and do
/// not overlap. Inside replaced text it goes to the end of the new text;
/// at the start of an edit, an insertion's included, it stays before it.
fn moved(at: usize, edits: &[Edit]) -> usize {
    let (mut added, mut removed) = (0, 0);
    for (range, text) in edits {
        if at <= range.start {
            break;
        }
        if at < range.end {
            return (range.start + added + text.len()).saturating_sub(removed);
        }
        added += text.len();
        removed += range.len();
    }
    (at + added).saturating_sub(removed)
}

/// `col` moved onto `line`, back to a character boundary.
fn floor_char(line: &str, col: usize) -> usize {
    let mut col = col.min(line.len());
    while !line.is_char_boundary(col) {
        col -= 1;
    }
    col
}

/// The one replacement on line `line` that turns `old` into `new`: the
/// text between what they start and end with in common.
fn replacement(line: usize, old: &str, new: &str) -> Replace {
    let same = |a: &mut dyn Iterator<Item = char>, b: &mut dyn Iterator<Item = char>| -> usize {
        a.zip(b)
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum()
    };
    let prefix = same(&mut old.chars(), &mut new.chars());
    let (old_rest, new_rest) = (&old[prefix..], &new[prefix..]);
    let suffix = same(&mut old_rest.chars().rev(), &mut new_rest.chars().rev());
    Replace {
        start: Pos::new(line, prefix),
        end: Pos::new(line, old.len() - suffix),
        text: new_rest[..new_rest.len() - suffix].to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::tests::{apply, check_with, parse, render, tasks_on};
    use crate::task::tests::Rng;
    use crate::{Document, Intent};

    fn day(text: &str) -> Date {
        Date::parse_iso(text).unwrap_or_else(|| panic!("{text} is a date"))
    }

    fn owner(name: &str) -> Change {
        Change::AddOwner(name.into())
    }

    /// Plan `change` at the caret's line of the fixture `before`, with
    /// tasks on and today 2026-11-19, under LF and CRLF endings.
    fn planned(before: &str, change: &Change, ending: &str) -> (String, Option<LineEdit>) {
        let (text, sel) = parse(before);
        let input = text.replace('\n', ending);
        let doc = Document::new(&input);
        let edit = edit(&doc, sel.head.line, sel, change, &tasks_on());
        let got = edit.as_ref().map(|edit| {
            assert!(edit.plan.edits.len() <= 1, "{before:?}: {edit:?}");
            for replace in &edit.plan.edits {
                assert_eq!(replace.start.line, edit.line, "{before:?}");
                assert_eq!(replace.end.line, edit.line, "{before:?}");
                assert!(!replace.text.contains(['\n', '\r']), "{before:?}");
            }
            assert_eq!(edit.plan_for(&doc), Ok(&edit.plan), "{before:?}");
            assert_eq!(edit.expect, doc.line(edit.line), "{before:?}");
            let after = apply(&doc, &edit.plan);
            let after = Document::new(&after);
            assert_eq!(after.line(edit.line), edit.text, "{before:?}");
            let description = &edit.text[edit.description.clone()];
            let read = crate::edit::task_item(&after, edit.line, Boxes::Status)
                .and_then(|(_, task)| parse_line(&edit.text, task));
            assert_eq!(read.as_deref(), Some(description), "{before:?}");
            render(&after.text().replace(ending, "\n"), edit.plan.selection)
        });
        (got.unwrap_or_default(), edit)
    }

    fn parse_line(line: &str, task: block::Task) -> Option<String> {
        super::parse(line, task).map(|read| read.description_text().to_owned())
    }

    /// Each `before → after` case, in the `|` caret notation; an `after`
    /// equal to `before` must be a plan with no edit.
    #[track_caller]
    fn check(change: &Change, cases: &[(&str, &str)]) {
        for (before, after) in cases {
            for ending in ["\n", "\r\n"] {
                let (got, edit) = planned(before, change, ending);
                let edit = edit.unwrap_or_else(|| panic!("{change:?} on {before:?} is refused"));
                assert_eq!(got, *after, "{change:?} on {before:?} ({ending:?})");
                assert_eq!(edit.is_noop(), before == after, "{change:?} on {before:?}");
            }
        }
    }

    /// Each fixture is refused: `edit` returns `None`.
    #[track_caller]
    fn refused(change: &Change, cases: &[&str]) {
        for before in cases {
            assert_eq!(
                planned(before, change, "\n").1,
                None,
                "{change:?} on {before:?}"
            );
        }
    }

    #[test]
    fn toggle_done_stamps_a_finished_task_and_unstamps_a_reopened_one() {
        check(
            &Change::ToggleDone,
            &[
                ("- [ ] a|", "- [x] a| ✅ 2026-11-19"),
                ("- [/] a|", "- [x] a| ✅ 2026-11-19"),
                ("- [界] a|", "- [x] a| ✅ 2026-11-19"),
                ("- [x] a ✅ 2026-11-18|", "- [ ] a|"),
                ("- [X] a ✅ 2026-11-18|", "- [ ] a|"),
                ("- [-] a ❌ 2026-11-18|", "- [ ] a|"),
                // Done by hand: no stamp to remove.
                ("- [x] a|", "- [ ] a|"),
                // The stamp goes after the due date, before a block id.
                (
                    "- [ ] a 📅 2026-12-01|",
                    "- [x] a 📅 2026-12-01| ✅ 2026-11-19",
                ),
                ("- [ ] a ^abc|", "- [x] a ✅ 2026-11-19 ^abc|"),
                (
                    "- [ ] a ⏫ 📅 2026-12-01 ^abc|",
                    "- [x] a ⏫ 📅 2026-12-01 ✅ 2026-11-19 ^abc|",
                ),
                (
                    "- [ ] a 📅 2026-12-01 #home|",
                    "- [x] a 📅 2026-12-01 #home| ✅ 2026-11-19",
                ),
                // A hand-ordered line keeps its order.
                (
                    "- [x] a ✅ 2026-11-18 📅 2026-12-01|",
                    "- [ ] a 📅 2026-12-01|",
                ),
                ("- [x] a ✅ 2026-11-18 @sam|", "- [ ] a @sam|"),
                // Every stamp goes when the task reopens.
                ("- [-] a ✅ 2026-11-01 ❌ 2026-11-18|", "- [ ] a|"),
                ("- [x] a ✅ 2026-11-01 ✅ 2026-11-02|", "- [ ] a|"),
                // Into done, a stamp there is re-dated in place, its
                // duplicates go, and a cancelled stamp goes.
                ("- [ ] a ✅ 2026-11-01|", "- [x] a ✅ 2026-11-19|"),
                (
                    "- [ ] a ✅ 2026-11-01 ✅ 2026-11-02|",
                    "- [x] a ✅ 2026-11-19|",
                ),
                ("- [ ] a ❌ 2026-11-18|", "- [x] a| ✅ 2026-11-19"),
                // An invalid stamp is a stamp.
                ("- [ ] a ✅ 2026-02-30|", "- [x] a ✅ 2026-11-19|"),
                ("- [x] a ✅ 2026-02-30|", "- [ ] a|"),
                ("- [-] a ❌ 2026-13-01|", "- [ ] a|"),
                // A stamp written with U+FE0F keeps it, or goes whole.
                (
                    "- [ ] a ✅\u{fe0f} 2026-11-01|",
                    "- [x] a ✅\u{fe0f} 2026-11-19|",
                ),
                ("- [x] a ✅\u{fe0f} 2026-11-18|", "- [ ] a|"),
                // The caret moves with the text around it.
                ("- [|x] a ✅ 2026-11-18", "- [| ] a"),
                ("- [x|] a ✅ 2026-11-18", "- [ |] a"),
                ("- [ ] a|b", "- [x] a|b ✅ 2026-11-19"),
                ("|- [ ] a", "|- [x] a ✅ 2026-11-19"),
                ("- [x] a ✅ 2026-1|1-18", "- [ ] a|"),
                ("- [界] a|", "- [x] a| ✅ 2026-11-19"),
                ("- [|界] a", "- [|x] a ✅ 2026-11-19"),
                ("- [界|] a", "- [x|] a ✅ 2026-11-19"),
                // A task with no description.
                ("- [ ]|", "- [x]| ✅ 2026-11-19"),
                ("- [ ] |", "- [x] |✅ 2026-11-19"),
            ],
        );
    }

    #[test]
    fn cycle_steps_open_doing_done_and_reopens_cancelled() {
        check(
            &Change::Cycle,
            &[
                ("- [ ] a|", "- [/] a|"),
                ("- [界] a|", "- [/] a|"),
                ("- [/] a|", "- [x] a| ✅ 2026-11-19"),
                ("- [x] a ✅ 2026-11-18|", "- [ ] a|"),
                ("- [X] a|", "- [ ] a|"),
                ("- [-] a ❌ 2026-11-18|", "- [ ] a|"),
                // Doing has no stamp: a stray one goes.
                ("- [ ] a ❌ 2026-11-18|", "- [/] a|"),
            ],
        );
    }

    #[test]
    fn set_status_writes_the_stamps_of_a_new_type_only() {
        let set = |symbol| Change::SetStatus(symbol);
        check(
            &set('-'),
            &[
                ("- [ ] a|", "- [-] a| ❌ 2026-11-19"),
                (
                    "- [/] a 📅 2026-12-01|",
                    "- [-] a 📅 2026-12-01| ❌ 2026-11-19",
                ),
                ("- [x] a ✅ 2026-11-18|", "- [-] a| ❌ 2026-11-19"),
                // Cancelled goes before done in the writing order.
                ("- [-] a ❌ 2026-11-18|", "- [-] a ❌ 2026-11-18|"),
            ],
        );
        check(
            &set('x'),
            &[
                ("- [-] a ❌ 2026-11-18|", "- [x] a| ✅ 2026-11-19"),
                ("- [/] a ✅ 2026-11-01|", "- [x] a ✅ 2026-11-19|"),
                // Already done: nothing changes, not even the date.
                ("- [x] a ✅ 2026-11-01|", "- [x] a ✅ 2026-11-01|"),
                // One done symbol to another keeps the stamp, or its lack.
                ("- [X] a ✅ 2026-11-01|", "- [x] a ✅ 2026-11-01|"),
                ("- [X] a|", "- [x] a|"),
            ],
        );
        check(
            &set('X'),
            &[("- [x] a ✅ 2026-11-01|", "- [X] a ✅ 2026-11-01|")],
        );
        check(
            &set('/'),
            &[
                ("- [ ] a|", "- [/] a|"),
                ("- [-] a ❌ 2026-11-18|", "- [/] a|"),
                ("- [x] a ✅ 2026-11-18 ^id|", "- [/] a ^id|"),
            ],
        );
        check(
            &set(' '),
            &[
                ("- [ ] a|", "- [ ] a|"),
                // An unknown symbol is open: nothing but the box changes.
                ("- [界] a ✅ 2026-11-01|", "- [ ] a ✅ 2026-11-01|"),
                ("- [x] a ✅ 2026-11-18|", "- [ ] a|"),
            ],
        );
        check(
            &set('界'),
            &[
                ("- [ ] a|", "- [界] a|"),
                ("- [/] a|", "- [界] a|"),
                ("- [x] a ✅ 2026-11-18|", "- [界] a|"),
                ("- [界] a|", "- [界] a|"),
            ],
        );
        for symbol in [']', '\n', '\r'] {
            refused(&set(symbol), &["- [ ] a|", "- [x] a ✅ 2026-11-18|"]);
        }
    }

    #[test]
    fn set_due_writes_replaces_and_clears() {
        let due = Change::SetDue(Some(day("2026-12-01")));
        check(
            &due,
            &[
                ("- [ ] a|", "- [ ] a| 📅 2026-12-01"),
                ("- [/] a|", "- [/] a| 📅 2026-12-01"),
                (
                    "- [-] a ❌ 2026-11-18|",
                    "- [-] a 📅 2026-12-01 ❌ 2026-11-18|",
                ),
                ("- [界] a|", "- [界] a| 📅 2026-12-01"),
                ("- [ ] a 📅 2026-11-30|", "- [ ] a 📅 2026-12-01|"),
                ("- [ ] a 📅 2026-12-01|", "- [ ] a 📅 2026-12-01|"),
                // An alias is re-dated in place, as it is written.
                ("- [ ] a 🗓 2026-11-30|", "- [ ] a 🗓 2026-12-01|"),
                (
                    "- [ ] a 📆\u{fe0f}2026-11-30|",
                    "- [ ] a 📆\u{fe0f}2026-12-01|",
                ),
                ("- [ ] a [due:: 2026-11-30]|", "- [ ] a [due:: 2026-12-01]|"),
                ("- [ ] a 📅 2026-02-30|", "- [ ] a 📅 2026-12-01|"),
                ("- [ ] a ⏫ ^id|", "- [ ] a ⏫ 📅 2026-12-01 ^id|"),
                (
                    "- [x] a ✅ 2026-11-18|",
                    "- [x] a 📅 2026-12-01 ✅ 2026-11-18|",
                ),
                ("- [ ] a @sam|", "- [ ] a @sam| 📅 2026-12-01"),
            ],
        );
        check(
            &Change::SetDue(None),
            &[
                ("- [ ] a 📅 2026-11-30|", "- [ ] a|"),
                ("- [ ] a 📅 2026-11-30 🗓 2026-12-02|", "- [ ] a|"),
                ("- [ ] a [due:: 2026-11-30] ⏫|", "- [ ] a ⏫|"),
                (
                    "- [x] a 📅 2026-11-30 ✅ 2026-11-18|",
                    "- [x] a ✅ 2026-11-18|",
                ),
                ("- [ ] a|", "- [ ] a|"),
            ],
        );
    }

    #[test]
    fn priority_is_set_raised_and_lowered_one_level_at_a_time() {
        use Priority::*;
        check(
            &Change::SetPriority(Some(High)),
            &[
                ("- [ ] a|", "- [ ] a| ⏫"),
                ("- [ ] a 🔼|", "- [ ] a ⏫|"),
                ("- [ ] a ⏫|", "- [ ] a ⏫|"),
                // Setting replaces every indicator, the line's own level too.
                ("- [ ] a 🔽 🔺|", "- [ ] a ⏫|"),
                ("- [ ] a ⏫ ⏫|", "- [ ] a ⏫|"),
                // Another level is a new glyph, written bare; the same
                // level stays as written.
                ("- [ ] a 🔼\u{fe0f}|", "- [ ] a ⏫|"),
                ("- [ ] a ⏫\u{fe0f}|", "- [ ] a ⏫\u{fe0f}|"),
                ("- [ ] a 📅 2026-12-01|", "- [ ] a ⏫ 📅 2026-12-01|"),
                ("- [x] a ✅ 2026-11-18|", "- [x] a ⏫ ✅ 2026-11-18|"),
            ],
        );
        check(
            &Change::SetPriority(None),
            &[
                ("- [ ] a ⏫ 📅 2026-12-01|", "- [ ] a 📅 2026-12-01|"),
                ("- [ ] a 🔽 🔺|", "- [ ] a|"),
                ("- [ ] a|", "- [ ] a|"),
            ],
        );
        check(
            &Change::RaisePriority,
            &[
                ("- [ ] a ⏬|", "- [ ] a 🔽|"),
                ("- [ ] a 🔽|", "- [ ] a|"),
                ("- [ ] a|", "- [ ] a| 🔼"),
                ("- [ ] a 🔼|", "- [ ] a ⏫|"),
                ("- [ ] a ⏫|", "- [ ] a 🔺|"),
                ("- [ ] a ⏫\u{fe0f}|", "- [ ] a 🔺|"),
                // At the top nothing changes, duplicates included.
                ("- [ ] a 🔺|", "- [ ] a 🔺|"),
                ("- [ ] a ⏬ 🔺|", "- [ ] a ⏬ 🔺|"),
                // The last indicator is the level; a step tidies the rest.
                ("- [ ] a 🔺 ⏫|", "- [ ] a 🔺|"),
                ("- [x] a 🔽 ✅ 2026-11-18|", "- [x] a ✅ 2026-11-18|"),
            ],
        );
        check(
            &Change::LowerPriority,
            &[
                ("- [ ] a 🔺|", "- [ ] a ⏫|"),
                ("- [ ] a ⏫|", "- [ ] a 🔼|"),
                ("- [ ] a 🔼|", "- [ ] a|"),
                ("- [ ] a|", "- [ ] a| 🔽"),
                ("- [ ] a 🔽|", "- [ ] a ⏬|"),
                ("- [ ] a ⏬|", "- [ ] a ⏬|"),
                ("- [ ] a 🔺 ⏬|", "- [ ] a 🔺 ⏬|"),
                ("- [ ] a ⏬ 🔼|", "- [ ] a|"),
            ],
        );
    }

    #[test]
    fn owners_are_added_at_the_end_of_the_description_and_removed_whole() {
        check(
            &owner("sam"),
            &[
                ("- [ ] a|", "- [ ] a| @sam"),
                ("- [ ] a 📅 2026-12-01|", "- [ ] a @sam 📅 2026-12-01|"),
                ("- [ ] a #home ⏫|", "- [ ] a #home @sam ⏫|"),
                ("- [ ] a @priya|", "- [ ] a @priya| @sam"),
                ("- [ ] a @sam|", "- [ ] a @sam|"),
                ("- [ ] ask @sam about it|", "- [ ] ask @sam about it|"),
                ("- [ ] a ^id|", "- [ ] a @sam ^id|"),
                ("- [ ] |", "- [ ] |@sam"),
            ],
        );
        check(
            &owner("jane.doe"),
            &[("- [x] a ✅ 2026-11-18|", "- [x] a @jane.doe ✅ 2026-11-18|")],
        );
        refused(&owner(""), &["- [ ] a|"]);
        refused(&owner("two words"), &["- [ ] a|"]);
        refused(&owner("@sam"), &["- [ ] a|"]);
        refused(&owner("1st"), &["- [ ] a|"]);
        let remove = Change::RemoveOwner("sam".into());
        check(
            &remove,
            &[
                ("- [ ] ask @sam about it|", "- [ ] ask about it|"),
                ("- [ ] a @sam 📅 2026-12-01|", "- [ ] a 📅 2026-12-01|"),
                ("- [ ] @sam a|", "- [ ] a|"),
                ("- [ ] a @sam @priya|", "- [ ] a @priya|"),
                ("- [ ] a 📅 2026-12-01 @sam|", "- [ ] a 📅 2026-12-01|"),
                ("- [ ] ask @sam.|", "- [ ] ask.|"),
                ("- [ ] a @Sam|", "- [ ] a @Sam|"),
                ("- [ ] a|", "- [ ] a|"),
                ("- [ ] a @sa|m b", "- [ ] a| b"),
            ],
        );
    }

    #[test]
    fn set_description_keeps_the_tokens_and_the_block_id() {
        let set = |text: &str| Change::SetDescription(text.into());
        check(
            &set("new text"),
            &[
                (
                    "- [ ] a| 📅 2026-12-01 ^id",
                    "- [ ] new text| 📅 2026-12-01 ^id",
                ),
                (
                    "- [ ] a|bc ⏫ 📅 2026-12-01",
                    "- [ ] new text| ⏫ 📅 2026-12-01",
                ),
                ("|- [ ] a ⏫", "|- [ ] new text ⏫"),
                ("- [ ] a ⏫|", "- [ ] new text ⏫|"),
                ("- [ ] 📅 2026-12-01|", "- [ ] new text 📅 2026-12-01|"),
                ("- [ ]|", "- [ ]| new text"),
                ("- [ ] new text|", "- [ ] new text|"),
                ("- [x] a ✅ 2026-11-18|", "- [x] new text ✅ 2026-11-18|"),
            ],
        );
        check(
            &set("call @sam\nabout it "),
            &[("- [ ] a| ⏫", "- [ ] call @sam about it| ⏫")],
        );
        check(
            &set(""),
            &[("- [ ] a| ⏫", "- [ ] |⏫"), ("- [ ] a|", "- [ ] |")],
        );
    }

    #[test]
    fn a_compound_change_is_one_replacement() {
        let form = Change::Compound(vec![
            Change::SetDescription("Send the deck".into()),
            Change::SetStatus('/'),
            Change::SetDue(Some(day("2026-12-01"))),
            Change::SetPriority(Some(Priority::High)),
            owner("sam"),
        ]);
        check(
            &form,
            &[
                ("- [ ] a| 🔽", "- [/] Send the deck| @sam ⏫ 📅 2026-12-01"),
                (
                    "- [x] old ✅ 2026-11-18 ^id|",
                    "- [/] Send the deck @sam ⏫ 📅 2026-12-01 ^id|",
                ),
                (
                    "> 3. [ ] a 🗓 2026-11-30 #home|",
                    "> 3. [/] Send the deck @sam ⏫ 🗓 2026-12-01 #home|",
                ),
            ],
        );
        // Done, with a stamp, in the same replacement.
        let done = Change::Compound(vec![Change::ToggleDone, owner("sam")]);
        check(&done, &[("- [ ] a|", "- [x] a| @sam ✅ 2026-11-19")]);
        // Nothing to do, and an empty compound.
        let same = Change::Compound(vec![Change::SetStatus(' '), owner("sam")]);
        check(&same, &[("- [ ] a @sam|", "- [ ] a @sam|")]);
        check(&Change::Compound(Vec::new()), &[("- [ ] a|", "- [ ] a|")]);
        // A refused step refuses the whole change.
        let bad = Change::Compound(vec![Change::ToggleDone, owner("no one")]);
        refused(&bad, &["- [ ] a|"]);
        // Nested compounds apply in order.
        let nested = Change::Compound(vec![
            Change::Compound(vec![Change::Cycle, Change::Cycle]),
            Change::RaisePriority,
        ]);
        check(&nested, &[("- [ ] a|", "- [x] a| 🔼 ✅ 2026-11-19")]);
        // Writing what is there moves no caret inside it.
        let same_due = Change::Compound(vec![
            Change::SetDue(Some(day("2026-12-01"))),
            Change::ToggleDone,
        ]);
        check(
            &same_due,
            &[(
                "- [ ] a 📅 2026-|12-01",
                "- [x] a 📅 2026-|12-01 ✅ 2026-11-19",
            )],
        );
        // Nesting as deep as a caller likes runs on no deeper stack.
        let mut deep = Change::Cycle;
        for _ in 0..200_000 {
            deep = Change::Compound(vec![deep]);
        }
        let doc = Document::new("- [ ] a");
        let planned = edit(&doc, 0, Selection::default(), &deep, &tasks_on());
        assert_eq!(planned.map(|edit| edit.text), Some("- [/] a".to_owned()));
        // Dropping it is the caller's: take it apart without recursion.
        let mut todo = vec![deep];
        while let Some(change) = todo.pop() {
            if let Change::Compound(changes) = change {
                todo.extend(changes);
            }
        }
    }

    #[test]
    fn the_change_finds_the_task_in_any_container() {
        check(
            &Change::ToggleDone,
            &[
                ("> - [ ] a|", "> - [x] a| ✅ 2026-11-19"),
                ("> > - [/] a|", "> > - [x] a| ✅ 2026-11-19"),
                ("3. [ ] a|", "3. [x] a| ✅ 2026-11-19"),
                ("1) [ ] a|", "1) [x] a| ✅ 2026-11-19"),
                ("* [ ] a|", "* [x] a| ✅ 2026-11-19"),
                ("+ [ ] a|", "+ [x] a| ✅ 2026-11-19"),
                ("- [ ]\ta|", "- [x]\ta| ✅ 2026-11-19"),
                ("- [x]\ta\t✅ 2026-11-18|", "- [ ]\ta|"),
                ("- a\n  - [ ] b|", "- a\n  - [x] b| ✅ 2026-11-19"),
                ("\u{feff}- [ ] a|", "\u{feff}- [x] a| ✅ 2026-11-19"),
                (
                    "x\n\n- [ ] a|\n- [ ] b",
                    "x\n\n- [x] a| ✅ 2026-11-19\n- [ ] b",
                ),
                // A line of the task's text edits the task's line.
                ("- [ ] a\n  b|", "- [x] a ✅ 2026-11-19\n  b|"),
                ("- [ ] a\nlazy|", "- [x] a ✅ 2026-11-19\nlazy|"),
                ("> - [ ] a\n>   b|", "> - [x] a ✅ 2026-11-19\n>   b|"),
                // A selection's ends on other lines stay.
                ("«x\n- [ ] a»", "«x\n- [x] a» ✅ 2026-11-19"),
                ("- [ ] «a\nb»", "- [x] «a ✅ 2026-11-19\nb»"),
            ],
        );
        refused(
            &Change::ToggleDone,
            &[
                "- a|",
                "a|",
                "|",
                "- [ ] a\n\n|",
                "- [ ] a\n  - b|",
                "```\n- [ ] a|\n```",
                "    - [ ] a|",
                "---\ntitle: x\n- [ ] a|\n---",
                "# [ ] a|",
            ],
        );
    }

    #[test]
    fn nothing_is_a_task_line_with_tasks_off() {
        let doc = Document::new("- [ ] a\n- [x] b ✅ 2026-11-18");
        let off = Options::default();
        for line in 0..2 {
            let caret = Selection::caret(Pos::new(line, 0));
            for change in [Change::ToggleDone, Change::Cycle, owner("sam")] {
                assert_eq!(edit(&doc, line, caret, &change, &off), None);
            }
        }
        assert_eq!(
            edit(&doc, 9, Selection::default(), &Change::Cycle, &tasks_on()),
            None
        );
    }

    #[test]
    fn a_changed_line_refuses_the_plan() {
        let text = "intro\n- [ ] a 📅 2026-12-01\n- [ ] b";
        let doc = Document::new(text);
        let caret = Selection::caret(Pos::new(1, 7));
        let due = Change::SetDue(Some(day("2026-12-02")));
        let planned = edit(&doc, 1, caret, &due, &tasks_on()).expect("a task line");
        assert_eq!(planned.line, 1);
        assert_eq!(planned.expect, "- [ ] a 📅 2026-12-01");
        assert_eq!(planned.text, "- [ ] a 📅 2026-12-02");
        assert_eq!(planned.plan_for(&doc), Ok(&planned.plan));
        // The same line elsewhere in a changed document still holds.
        let elsewhere = Document::new("intro!\n- [ ] a 📅 2026-12-01\n- [ ] b c");
        assert_eq!(planned.plan_for(&elsewhere), Ok(&planned.plan));
        for changed in [
            "intro\n- [ ] ab 📅 2026-12-01\n- [ ] b",
            "intro\n- [x] a 📅 2026-12-01\n- [ ] b",
            "intro\n- [ ] a 📅 2026-12-01 \n- [ ] b",
            "new\nintro\n- [ ] a 📅 2026-12-01\n- [ ] b",
            "intro",
            "",
        ] {
            let doc = Document::new(changed);
            assert_eq!(planned.plan_for(&doc), Err(Stale), "{changed:?}");
        }
        // The same text that is no task any more: code, front matter.
        let text = "intro\nmore\n- [ ] a\nend";
        let doc = Document::new(text);
        let caret = Selection::caret(Pos::new(2, 7));
        let planned = edit(&doc, 2, caret, &Change::ToggleDone, &tasks_on()).expect("a task");
        assert_eq!(planned.plan_for(&doc), Ok(&planned.plan));
        for changed in [
            "```\nmore\n- [ ] a\nend",
            "---\ntitle: x\n- [ ] a\n---",
            "intro\n```\n- [ ] a\n```",
        ] {
            let doc = Document::new(changed);
            assert_eq!(doc.line(2), planned.expect);
            assert_eq!(planned.plan_for(&doc), Err(Stale), "{changed:?}");
        }
        // The same task at the same index, after other changes, holds.
        let twice = Document::new("- [ ] a\n- [ ] a\n- [ ] a");
        let planned = edit(&twice, 1, caret, &Change::Cycle, &tasks_on()).expect("a task");
        let fewer = Document::new("- [ ] a\n- [ ] a");
        assert_eq!(planned.plan_for(&fewer), Ok(&planned.plan));
        assert_eq!(Stale.to_string(), "the task line changed");
    }

    #[test]
    fn the_replacement_is_the_changed_text_only() {
        assert_eq!(
            replacement(4, "- [ ] a", "- [x] a ✅ 2026-11-19"),
            Replace {
                start: Pos::new(4, 3),
                end: Pos::new(4, 7),
                text: "x] a ✅ 2026-11-19".into(),
            }
        );
        // Common bytes inside a character never split it.
        let r = replacement(0, "a 🔼 b", "a 🔽 b");
        assert_eq!((r.start.col, r.end.col, r.text.as_str()), (2, 6, "🔽"));
        assert_eq!(moved(5, &[(2..4, "xyz".into())]), 6);
        assert_eq!(moved(3, &[(2..4, "xyz".into())]), 5);
        assert_eq!(moved(2, &[(2..2, "xyz".into())]), 2);
        assert_eq!(moved(1, &[(2..4, String::new())]), 1);
    }

    /// Enter on a done task (Q11 and contract 1.4): the new line is open
    /// and gets no stamp. A stamp after the caret is the user's split, and
    /// moves down with the rest of the text as it is; the line above keeps
    /// the space before it, as Enter keeps it anywhere.
    #[test]
    fn enter_on_a_finished_task_continues_open_and_copies_no_stamp() {
        check_with(
            Intent::Enter,
            &tasks_on(),
            &[
                ("- [x] a ✅ 2026-11-19|", "- [x] a ✅ 2026-11-19\n- [ ] |"),
                ("- [X] a ✅ 2026-11-19|", "- [X] a ✅ 2026-11-19\n- [ ] |"),
                ("- [-] a ❌ 2026-11-19|", "- [-] a ❌ 2026-11-19\n- [ ] |"),
                (
                    "- [x] a 📅 2026-12-01 ✅ 2026-11-19|",
                    "- [x] a 📅 2026-12-01 ✅ 2026-11-19\n- [ ] |",
                ),
                ("- [x] a |✅ 2026-11-19", "- [x] a \n- [ ] |✅ 2026-11-19"),
            ],
        );
    }

    /// A random change, compounds of up to three included.
    fn random_change(rng: &mut Rng, depth: usize) -> Change {
        let date = || Date::new(2026, 12, 1);
        match rng.below(if depth > 0 { 11 } else { 12 }) {
            0 => Change::ToggleDone,
            1 => Change::Cycle,
            2 => Change::SetStatus(rng.pick(&[' ', 'x', 'X', '/', '-', '?', '界'])),
            3 => Change::SetDue(date()),
            4 => Change::SetDue(None),
            5 => {
                Change::SetPriority(rng.pick(&[None, Some(Priority::High), Some(Priority::Lowest)]))
            }
            6 => Change::RaisePriority,
            7 => Change::LowerPriority,
            8 => owner(rng.pick(&["sam", "priya", "jane.doe"])),
            9 => Change::RemoveOwner(rng.pick(&["sam", "priya"]).into()),
            10 => Change::SetDescription(
                rng.pick(&["", "x", "call @sam", "a #tag", "b 📅 2026-12-02"])
                    .into(),
            ),
            _ => Change::Compound(
                (0..rng.below(4))
                    .map(|_| random_change(rng, depth + 1))
                    .collect(),
            ),
        }
    }

    /// The status type a change leaves, when it sets one.
    fn status_after(change: &Change, from: block::Task) -> Option<StatusType> {
        match change {
            Change::ToggleDone => Some(status::status(status::status(from.symbol).next).kind),
            Change::Cycle => Some(status::status(cycled(from.status)).kind),
            Change::SetStatus(symbol) => Some(status::status(*symbol).kind),
            _ => None,
        }
    }

    fn line_edit_holds(rng: &mut Rng) {
        let line = crate::task::tests::random_line(rng);
        let doc_text = format!("intro\n{line}\nafter");
        let doc = Document::new(&doc_text);
        let Some((k, task)) = crate::edit::task_item(&doc, 1, Boxes::Status) else {
            return;
        };
        let at = |rng: &mut Rng| {
            let starts: Vec<usize> = line
                .char_indices()
                .map(|(at, _)| at)
                .chain([line.len()])
                .collect();
            Pos::new(1, rng.pick(&starts))
        };
        let sel = Selection::new(at(rng), at(rng));
        let change = random_change(rng, 0);
        let context = format!("{change:?} on {line:?}");
        let Some(edit) = edit(&doc, 1, sel, &change, &tasks_on()) else {
            panic!("refused: {context}");
        };
        assert_eq!(
            (edit.line, edit.expect.as_str()),
            (k, line.as_str()),
            "{context}"
        );
        assert!(edit.plan.edits.len() <= 1, "{context}");
        assert_eq!(edit.plan_for(&doc), Ok(&edit.plan), "{context}");
        let after = apply(&doc, &edit.plan);
        let after = Document::new(&after);
        assert_eq!(after.line_count(), 3, "{context}");
        assert_eq!(after.line(1), edit.text, "{context}");
        // The edited line is still the task, its box where it was.
        let (_, read) = crate::edit::task_item(&after, 1, Boxes::Status).expect(&context);
        assert_eq!(read.start, task.start, "{context}");
        for end in [edit.plan.selection.anchor, edit.plan.selection.head] {
            let text = after.line(end.line);
            assert!(
                end.col <= text.len() && text.is_char_boundary(end.col),
                "{context}"
            );
        }
        // A status change into a type leaves that type's stamps only.
        if let Some(kind) = status_after(&change, task).filter(|kind| *kind != task.status) {
            let reading = super::parse(after.line(1), read).expect(&context);
            let count = |field| reading.dates(field).count();
            let (done, cancelled) = (count(DateField::Done), count(DateField::Cancelled));
            match kind {
                StatusType::Done => assert_eq!((done, cancelled), (1, 0), "{context}"),
                StatusType::Cancelled => assert_eq!((done, cancelled), (0, 1), "{context}"),
                _ => assert_eq!((done, cancelled), (0, 0), "{context}"),
            }
        }
    }

    fn corpus(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        for _ in 0..count {
            line_edit_holds(&mut rng);
        }
    }

    /// Every change on random task lines: one replacement, a task line
    /// after it, a selection on the text, and the stamps the status asks.
    #[test]
    fn random_line_edits_hold() {
        corpus(2000, 5);
    }

    #[test]
    #[ignore = "full count; cargo test -p wren-markdown fuzz_line_edit -- --ignored"]
    fn fuzz_line_edit() {
        corpus(300_000, 6);
    }
}
