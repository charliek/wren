//! The "Create or edit task" form's model (discovery/tasks.md §6, item 3):
//! a task line read into fields, the fields as the user changes them, and
//! the one [`Change`] that writes them back.
//!
//! **Only what the user changed is written.** The form keeps the values it
//! opened with and sends a [`Change::Compound`] of the fields that differ
//! from them, in this order: the description, the owners removed and
//! added, the status, the priority and the due date. A field set back to
//! what it was is no change, so saving a form nobody touched writes
//! nothing, and a line written by hand (two priority glyphs, tokens in an
//! order of its own) is never tidied for a field the user did not change.
//! The fields compare as the line reads them: the status by its type (`[X]`
//! is done, as `[x]` is, and `[?]` open), the due date by the day it
//! names (`mon` is the 23rd the line already has), the description with
//! the white space around it dropped.
//!
//! **Owners** are part of the description's text where they are written
//! there (`Call @sam`), and may stand among the tokens too. The owner chips
//! the form shows are the owners of the line it would write. Adding one is
//! [`Change::AddOwner`] (`@name` at the end of the description). Removing
//! one takes it out of the description field's text, where the user sees
//! it go, and out of the tokens with [`Change::RemoveOwner`]. Names compare
//! without regard to case, as the design's completion does: `@Sam` is
//! `@sam`'s chip.
//!
//! **A line that is no task** becomes one, as the plain toggle
//! (`markdown.toggle_task`) makes it one: a blank line gets `- [ ] `, in
//! its quote if it is in one; a list item gets a box after its marker; a
//! line of text gets `- [ ] ` before its text, which becomes the
//! description. The form then writes that line with its fields. A heading,
//! a thematic break, code, and anything else the toggle gives no box,
//! opens no form; nor does a heading's line or a thematic break inside a
//! task's item, or a Setext heading's text.

use std::fmt;

use super::{parse, Change, DateField, Priority};
use crate::block::{self, Boxes};
use crate::date::{self, Date, Parsed};
use crate::status::{self, StatusType};
use crate::{Intent, Lines, Options, Pos, Selection};

/// What the form was opened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormKind {
    /// A task: "Edit task".
    Edit,
    /// A line with no text of its own (a blank line, an empty list item):
    /// "New task".
    New,
    /// A line of text, or a list item with text, that is no task: "Make a
    /// task", its text the description.
    Make,
}

/// Why the form cannot be saved as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The due field names no date: its text, trimmed.
    NotADate(String),
    /// The due field holds a date in the shape `YYYY-MM-DD` that is no
    /// real day, and not the one the line already had.
    NotReal(String),
}

/// As the calendar's caption says it.
impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADate(_) => f.write_str("Not a date. Try fri, +3d or 2026-12-01."),
            Self::NotReal(text) => write!(f, "{text} is not a real date"),
        }
    }
}

impl std::error::Error for Problem {}

/// The fields as the line read them when the form opened.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Original {
    description: String,
    status: StatusType,
    /// The due date's text as written, and what it reads as.
    due: Option<(String, Parsed)>,
    priority: Option<Priority>,
    /// Every owner the line is read as having, in line order
    /// ([`TaskLine::owners`]): its name, and whether it is written in the
    /// description.
    owners: Vec<(String, bool)>,
}

/// The form over one task line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    kind: FormKind,
    /// The line the task's box is on, or goes on.
    line: usize,
    /// That line's text when the form opened.
    expect: String,
    /// The task line the fields start from: `expect`, or for a line that
    /// is no task, `expect` with the toggle's box.
    base: String,
    /// The box of `base`.
    task: block::Task,
    today: Date,
    original: Original,
    description: String,
    status: StatusType,
    due: String,
    priority: Option<Priority>,
    /// The user typed in the description field: its text is theirs, and
    /// no longer the original's with the mentions removed.
    edited: bool,
    /// Owners the user added, in order, spelled as added.
    added: Vec<String>,
    /// The places among the original owners (`Original::owners`) of the
    /// mentions the user removed, each its own: adding the owner back
    /// takes its removal back, and the mention stays as written.
    removed: Vec<usize>,
    /// What the owners field holds, not yet added: written, and saved, as
    /// typed, when it is an owner name.
    pending: String,
}

/// `lines` with line `index` read as `text`.
struct Overlay<'a, L: Lines + ?Sized> {
    lines: &'a L,
    index: usize,
    text: &'a str,
}

impl<L: Lines + ?Sized> Lines for Overlay<'_, L> {
    fn line_count(&self) -> usize {
        self.lines.line_count()
    }

    fn line(&self, i: usize) -> &str {
        if i == self.index {
            self.text
        } else {
            self.lines.line(i)
        }
    }
}

impl Form {
    /// The form for line `line` of `lines`: for the task the line belongs
    /// to (its box's line, or a line of the item's text after it), or for
    /// a line the plain toggle would give a box, the task it would make.
    /// `None` with tasks off in `opts`, and on a line no task can go on.
    pub fn open<L: Lines + ?Sized>(lines: &L, line: usize, opts: &Options) -> Option<Self> {
        let today = opts.tasks?.today;
        // A heading or a thematic break is no task's, nor can it become
        // one: not even one inside a task's item.
        if line >= lines.line_count() || crate::edit::heading_or_break(lines, line, opts.boxes()) {
            return None;
        }
        if let Some((k, task)) = crate::edit::task_item(lines, line, opts.boxes()) {
            let expect = lines.line(k).to_owned();
            return Self::read(FormKind::Edit, k, expect.clone(), expect, task, today);
        }
        let plan = crate::edit::plan(
            lines,
            Selection::caret(Pos::new(line, 0)),
            Intent::ToggleTask,
            opts,
        );
        let [edit] = plan.edits.as_slice() else {
            return None;
        };
        let k = edit.start.line;
        if edit.end.line != k || edit.text.contains(['\n', '\r']) {
            return None;
        }
        let expect = lines.line(k);
        let base = format!(
            "{}{}{}",
            expect.get(..edit.start.col)?,
            edit.text,
            expect.get(edit.end.col..)?
        );
        let overlay = Overlay {
            lines,
            index: k,
            text: &base,
        };
        let (at, task) = crate::edit::task_item(&overlay, k, opts.boxes())?;
        if at != k {
            return None;
        }
        let kind = if parse(&base, task)?.description().is_empty() {
            FormKind::New
        } else {
            FormKind::Make
        };
        Self::read(kind, k, expect.to_owned(), base, task, today)
    }

    /// The form for the task `task` of `text`, line `line` of a note, as the
    /// note reads it: a task whose line read alone may be none (nested in a
    /// quote or a list). The form edits it, and expects `text` there.
    pub fn edit_task(line: usize, text: &str, task: block::Task, today: Date) -> Option<Self> {
        Self::read(
            FormKind::Edit,
            line,
            text.to_owned(),
            text.to_owned(),
            task,
            today,
        )
    }

    fn read(
        kind: FormKind,
        line: usize,
        expect: String,
        base: String,
        task: block::Task,
        today: Date,
    ) -> Option<Self> {
        let reading = parse(&base, task)?;
        let original = Original {
            description: reading.description_text().to_owned(),
            status: task.status,
            due: reading
                .due()
                .map(|token| (reading.text(&token.date).to_owned(), token.value)),
            priority: reading.priority().map(|token| token.level),
            owners: reading
                .owners()
                .iter()
                .map(|owner| {
                    let in_description = reading.description().contains(&owner.range.start);
                    (reading.text(&owner.name).to_owned(), in_description)
                })
                .collect(),
        };
        Some(Self {
            kind,
            line,
            expect,
            task,
            today,
            description: original.description.clone(),
            status: original.status,
            due: original
                .due
                .as_ref()
                .map(|(text, _)| text.clone())
                .unwrap_or_default(),
            priority: original.priority,
            edited: false,
            added: Vec::new(),
            removed: Vec::new(),
            pending: String::new(),
            original,
            base,
        })
    }

    pub fn kind(&self) -> FormKind {
        self.kind
    }

    /// The line the task's box is on, or goes on.
    pub fn line(&self) -> usize {
        self.line
    }

    /// That line's text when the form opened, which saving replaces.
    pub fn expect(&self) -> &str {
        &self.expect
    }

    pub fn today(&self) -> Date {
        self.today
    }

    /// Today is now `today`: the day a status writes its stamp with, and
    /// the due field's words are read from.
    pub fn set_today(&mut self, today: Date) {
        self.today = today;
    }

    /// Whether line [`line`](Self::line) of `lines` still opens this form:
    /// it reads as it did, in the same place in the note, with the same
    /// box or the same box to come. Saving is refused once it does not.
    pub fn holds<L: Lines + ?Sized>(&self, lines: &L, opts: &Options) -> bool {
        Self::open(lines, self.line, opts).is_some_and(|now| {
            now.kind == self.kind
                && now.line == self.line
                && now.expect == self.expect
                && now.base == self.base
                && now.task == self.task
        })
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    /// The description field now holds `text`, the user's own: the
    /// mentions removed from it are as the text has them, and what it names
    /// stays where it is typed. Removed owners among the tokens stay
    /// removed.
    pub fn set_description(&mut self, text: &str) {
        text.clone_into(&mut self.description);
        self.edited = true;
        let owners = &self.original.owners;
        self.removed.retain(|place| !owners[*place].1);
    }

    pub fn status(&self) -> StatusType {
        self.status
    }

    pub fn set_status(&mut self, status: StatusType) {
        self.status = status;
    }

    pub fn priority(&self) -> Option<Priority> {
        self.priority
    }

    pub fn set_priority(&mut self, priority: Option<Priority>) {
        self.priority = priority;
    }

    /// The due field's text.
    pub fn due(&self) -> &str {
        &self.due
    }

    /// The due field now holds `text`, as typed.
    pub fn set_due(&mut self, text: &str) {
        text.clone_into(&mut self.due);
    }

    /// The due field set to `date` as a date picked is, or emptied.
    pub fn pick_due(&mut self, date: Option<Date>) {
        self.due = date.map(|date| date.to_string()).unwrap_or_default();
    }

    /// What the due field says: no date (empty), a date, the line's own
    /// date that is no real day, or why it says nothing the form can write.
    pub fn due_value(&self) -> Result<Option<Parsed>, Problem> {
        let text = self.due.trim();
        if text.is_empty() {
            return Ok(None);
        }
        match date::parse(text, self.today) {
            Some(Parsed::Date(date)) => Ok(Some(Parsed::Date(date))),
            Some(Parsed::Invalid) => match &self.original.due {
                Some((written, Parsed::Invalid)) if written == text => Ok(Some(Parsed::Invalid)),
                _ => Err(Problem::NotReal(text.to_owned())),
            },
            None => Err(Problem::NotADate(text.to_owned())),
        }
    }

    /// The date the due field names, for a calendar to open on.
    pub fn due_date(&self) -> Option<Date> {
        match self.due_value() {
            Ok(Some(Parsed::Date(date))) => Some(date),
            _ => None,
        }
    }

    /// The owners of the line the form would write, each name once, in
    /// line order, spelled as first written: the owners field's chips, so
    /// without a name still being typed there.
    pub fn owners(&self) -> Vec<String> {
        let line = self.rewritten(self.due_change().unwrap_or(None), false);
        let Some(task) = block::task_box(&line, self.task.start, Boxes::Status) else {
            return Vec::new();
        };
        let Some(reading) = parse(&line, task) else {
            return Vec::new();
        };
        let mut owners: Vec<String> = Vec::new();
        for name in reading.owner_names() {
            if !owners.iter().any(|owner| same_name(owner, name)) {
                owners.push(name.to_owned());
            }
        }
        owners
    }

    /// The owners field now holds `text`, not yet added: it is written,
    /// and saved, as typed when it names an owner the task lacks.
    pub fn set_pending_owner(&mut self, text: &str) {
        text.clone_into(&mut self.pending);
    }

    pub fn pending_owner(&self) -> &str {
        &self.pending
    }

    /// Add the owner `name` (an `@` before it is dropped): `false` when it
    /// is no owner name, or the task has that owner already. An owner the
    /// user removed comes back as it was written, in its place: its
    /// mentions' removals are taken back.
    pub fn add_owner(&mut self, name: &str) -> bool {
        let name = name.trim();
        let name = name.strip_prefix('@').unwrap_or(name);
        if !super::is_owner_name(name) {
            return false;
        }
        if self.restore(name) {
            return true;
        }
        if self.owners().iter().any(|owner| same_name(owner, name)) {
            return false;
        }
        self.added.push(name.to_owned());
        true
    }

    /// Take back the removals of the original mentions of `name`: whether
    /// there were any.
    fn restore(&mut self, name: &str) -> bool {
        let owners = &self.original.owners;
        let before = self.removed.len();
        self.removed
            .retain(|place| !same_name(&owners[*place].0, name));
        let restored = self.removed.len() != before;
        if restored {
            self.refresh_description();
        }
        restored
    }

    /// Remove the owner `name`, however it is spelled on the line: from the
    /// owners added; each original mention by its own removal; and from
    /// the description field's text once that is the user's own.
    pub fn remove_owner(&mut self, name: &str) {
        let name = name.strip_prefix('@').unwrap_or(name);
        let added = self.added.len();
        self.added.retain(|owner| !same_name(owner, name));
        let mut shown = self.added.len() != added;
        for (place, (owner, in_description)) in self.original.owners.iter().enumerate() {
            let typed_over = *in_description && self.edited;
            if same_name(owner, name) && !typed_over && !self.removed.contains(&place) {
                self.removed.push(place);
                shown = true;
            }
        }
        if !self.edited {
            self.refresh_description();
            // A mention a removal revealed (`@bob` once `@sam@bob` lost
            // `@sam`) is no original owner's: it goes from the description's
            // text, which becomes the user's own. One the owner's own chip
            // stood for is gone, and one its removal revealed (`@sam` of
            // `@Sam@sam`) stays for a click of its own.
            if shown || !self.owners().iter().any(|owner| same_name(owner, name)) {
                return;
            }
            let description = self.description.clone();
            self.set_description(&description);
        }
        let spellings: Vec<String> = description_owners(&self.description)
            .into_iter()
            .filter(|owner| same_name(owner, name))
            .collect();
        for spelling in spellings {
            self.description = without_owner(&self.description, &spelling);
        }
    }

    /// The description field, while it is not the user's own: the
    /// original's with the removed mentions taken out.
    fn refresh_description(&mut self) {
        if self.edited {
            return;
        }
        let removals = Change::Compound(removals(&self.removed));
        let line = super::rewrite(&self.base, self.task, &removals, self.today);
        let text = line.as_deref().and_then(|line| {
            let task = block::task_box(line, self.task.start, Boxes::Status)?;
            parse(line, task).map(|reading| reading.description_text().to_owned())
        });
        self.description = text.unwrap_or_else(|| self.original.description.clone());
    }

    /// The change the form writes: the fields that differ from the line
    /// as it was read, as one [`Change::Compound`], empty when none does.
    /// Refused while the due field names no date it can write.
    pub fn change(&self) -> Result<Change, Problem> {
        let due = self.due_change()?;
        Ok(Change::Compound(self.changes_with(due, true)))
    }

    /// The line [`change`](Self::change) writes.
    pub fn written(&self) -> Result<String, Problem> {
        Ok(self.rewritten(self.due_change()?, true))
    }

    /// The line the form would write, with the due date as the line has it
    /// while the due field names none it can write: what the form shows.
    pub fn preview(&self) -> String {
        self.rewritten(self.due_change().unwrap_or(None), true)
    }

    /// The base line with the changes the fields make, `due` the due
    /// date's, and with the owner being typed when `pending`.
    fn rewritten(&self, due: Option<Change>, pending: bool) -> String {
        let change = Change::Compound(self.changes_with(due, pending));
        super::rewrite(&self.base, self.task, &change, self.today)
            .unwrap_or_else(|| self.base.clone())
    }

    /// Whether saving writes anything: a field changed, or the line is no
    /// task yet.
    pub fn writes(&self) -> bool {
        self.kind != FormKind::Edit
            || self
                .change()
                .is_ok_and(|change| change != Change::Compound(Vec::new()))
    }

    /// The done or cancelled stamp of the line the form would write.
    pub fn stamp(&self) -> Option<(DateField, Date)> {
        let line = self.preview();
        let task = block::task_box(&line, self.task.start, Boxes::Status)?;
        let reading = parse(&line, task)?;
        [DateField::Done, DateField::Cancelled]
            .into_iter()
            .find_map(|field| match reading.date(field)?.value {
                Parsed::Date(date) => Some((field, date)),
                Parsed::Invalid => None,
            })
    }

    /// The due date's change: `None` for none.
    fn due_change(&self) -> Result<Option<Change>, Problem> {
        let original = self.original.due.as_ref().map(|(_, value)| *value);
        Ok(match self.due_value()? {
            None if original.is_some() => Some(Change::SetDue(None)),
            Some(Parsed::Date(date)) if original != Some(Parsed::Date(date)) => {
                Some(Change::SetDue(Some(date)))
            }
            _ => None,
        })
    }

    fn changes_with(&self, due: Option<Change>, pending: bool) -> Vec<Change> {
        let typed = self.pending.trim();
        let typed = typed.strip_prefix('@').unwrap_or(typed);
        let typed = (pending && super::is_owner_name(typed)).then_some(typed);
        // An owner being typed takes back the removals of its mentions, as
        // adding it does.
        let owners = &self.original.owners;
        let removed: Vec<usize> = (self.removed.iter().copied())
            .filter(|place| typed.is_none_or(|name| !same_name(&owners[*place].0, name)))
            .collect();
        // Each mention removed by its place on the line as read, before
        // anything else changes it.
        let mut changes = removals(&removed);
        // The description field is the user's own once typed in; until then
        // it is the original's, the removals above taking mentions out.
        if self.edited && self.description.trim() != self.original.description {
            changes.push(Change::SetDescription(self.description.clone()));
        }
        // An owner added is written once its key is no owner's of the line
        // already: not one the description names, an original mention left
        // in place, nor one added before it.
        let mut present: Vec<String> = owners
            .iter()
            .enumerate()
            .filter(|(place, (_, in_description))| {
                !removed.contains(place) && !(self.edited && *in_description)
            })
            .map(|(_, (name, _))| super::fold_name(name))
            .collect();
        if self.edited {
            present.extend(
                description_owners(&self.description)
                    .iter()
                    .map(|name| super::fold_name(name)),
            );
        }
        for name in self.added.iter().map(String::as_str).chain(typed) {
            let key = super::fold_name(name);
            if !present.contains(&key) {
                present.push(key);
                changes.push(Change::AddOwner(name.to_owned()));
            }
        }
        if self.status != self.original.status {
            let row = status::STATUSES
                .iter()
                .find(|row| row.kind == self.status)
                .expect("every type has a status");
            changes.push(Change::SetStatus(row.symbol));
        }
        if self.priority != self.original.priority {
            changes.push(Change::SetPriority(self.priority));
        }
        changes.extend(due);
        changes
    }
}

/// Whether two owner names are one owner's.
fn same_name(a: &str, b: &str) -> bool {
    super::fold_name(a) == super::fold_name(b)
}

/// The changes that remove the original owners at `places`, the last
/// first, so each place is still read on the line as it was.
fn removals(places: &[usize]) -> Vec<Change> {
    let mut places = places.to_vec();
    places.sort_unstable_by(|a, b| b.cmp(a));
    places.dedup();
    places.into_iter().map(Change::RemoveOwnerAt).collect()
}

/// A description's text as a task line after a box, for reading and
/// writing it on its own.
fn scratch(description: &str) -> String {
    format!("- [ ] {description}")
}

/// The box [`scratch`] puts before a description.
const SCRATCH_BOX: usize = 2;

/// The owners `description` names, wherever they are in it.
fn description_owners(description: &str) -> Vec<String> {
    let line = scratch(description);
    let reading =
        block::task_box(&line, SCRATCH_BOX, Boxes::Status).and_then(|task| parse(&line, task));
    reading.map_or_else(Vec::new, |reading| {
        reading.owner_names().map(str::to_owned).collect()
    })
}

/// `description` without the owner `name`, spelled exactly so.
fn without_owner(description: &str, name: &str) -> String {
    let line = scratch(description);
    let prefix = line.len() - description.len();
    block::task_box(&line, SCRATCH_BOX, Boxes::Status)
        .and_then(|task| parse(&line, task))
        .and_then(|reading| reading.remove_owner(name).get(prefix..).map(str::to_owned))
        .unwrap_or_else(|| description.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, TaskOptions};

    fn today() -> Date {
        Date::new(2026, 11, 19).expect("a date")
    }

    fn opts() -> Options {
        Options {
            tasks: Some(TaskOptions { today: today() }),
            ..Options::default()
        }
    }

    fn day(text: &str) -> Date {
        Date::parse_iso(text).expect("a date")
    }

    /// The form for line `line` of `text`.
    fn form(text: &str, line: usize) -> Form {
        Form::open(&Document::new(text), line, &opts()).expect("a form")
    }

    fn compound(form: &Form) -> Vec<Change> {
        match form.change().expect("a change") {
            Change::Compound(changes) => changes,
            other => panic!("{other:?}"),
        }
    }

    /// A step done to a form, the change it makes, and the line written.
    type Case<'a> = (&'a dyn Fn(&mut Form), Vec<Change>, &'a str);

    const FULL: &str = "- [/] Draft the runbook @alice ⏫ 📅 2026-11-23 @bob ^r1";

    #[test]
    fn a_task_reads_into_its_fields() {
        let form = form(FULL, 0);
        assert_eq!(form.kind(), FormKind::Edit);
        assert_eq!(form.description(), "Draft the runbook @alice");
        assert_eq!(form.status(), StatusType::Doing);
        assert_eq!(form.due(), "2026-11-23");
        assert_eq!(form.priority(), Some(Priority::High));
        assert_eq!(form.owners(), ["alice", "bob"]);
        assert_eq!(form.preview(), FULL);
    }

    #[test]
    fn a_form_nobody_changed_writes_nothing() {
        for line in [
            FULL,
            // Two priorities: setting either would tidy them into one.
            "- [ ] Two ⏫ 🔼 📅 2026-12-01",
            // An order of its own, a VS16, an alias and a field.
            "- [X] Hand 📅 2026-12-01 ⏫\u{fe0f} ✅ 2026-11-18",
            "- [?] Unknown 🗓 2026-12-02 [due:: 2026-12-03]",
            "> - [-] Quoted ❌ 2026-11-17",
            "- [ ] Unreal 📅 2026-02-30",
        ] {
            let form = form(line, 0);
            assert_eq!(compound(&form), [], "{line}");
            assert!(!form.writes(), "{line}");
            assert_eq!(form.written().unwrap(), line, "{line}");
        }
    }

    #[test]
    fn a_field_set_back_to_what_it_was_is_no_change() {
        let mut form = form("- [X] Done ⏫ ⏫ 📅 2026-11-23", 0);
        form.set_status(StatusType::Open);
        form.set_status(StatusType::Done);
        form.set_priority(None);
        form.set_priority(Some(Priority::High));
        // The same day, by a word and with white space.
        form.set_due(" mon ");
        form.set_description(" Done  ");
        assert_eq!(compound(&form), []);
    }

    #[test]
    fn each_field_changed_is_the_one_change() {
        let cases: [Case; 7] = [
            (
                &|form| form.set_description("Draft the plan @alice"),
                vec![Change::SetDescription("Draft the plan @alice".into())],
                "- [/] Draft the plan @alice ⏫ 📅 2026-11-23 @bob ^r1",
            ),
            (
                &|form| form.set_status(StatusType::Done),
                vec![Change::SetStatus('x')],
                // An owner among the tokens is looked past, never moved.
                "- [x] Draft the runbook @alice ⏫ 📅 2026-11-23 @bob ✅ 2026-11-19 ^r1",
            ),
            (
                &|form| form.set_priority(Some(Priority::Low)),
                vec![Change::SetPriority(Some(Priority::Low))],
                "- [/] Draft the runbook @alice 🔽 📅 2026-11-23 @bob ^r1",
            ),
            (
                &|form| form.set_priority(None),
                vec![Change::SetPriority(None)],
                "- [/] Draft the runbook @alice 📅 2026-11-23 @bob ^r1",
            ),
            (
                &|form| form.set_due("fri"),
                vec![Change::SetDue(Some(day("2026-11-20")))],
                "- [/] Draft the runbook @alice ⏫ 📅 2026-11-20 @bob ^r1",
            ),
            (
                &|form| form.set_due(""),
                vec![Change::SetDue(None)],
                "- [/] Draft the runbook @alice ⏫ @bob ^r1",
            ),
            (
                &|form| assert!(form.add_owner("@sam")),
                vec![Change::AddOwner("sam".into())],
                "- [/] Draft the runbook @alice @sam ⏫ 📅 2026-11-23 @bob ^r1",
            ),
        ];
        for (act, changes, written) in cases {
            let mut form = form(FULL, 0);
            act(&mut form);
            assert_eq!(compound(&form), changes, "{written}");
            assert_eq!(form.written().unwrap(), written);
            assert_eq!(form.preview(), written);
            assert!(form.writes());
        }
    }

    #[test]
    fn every_field_changed_writes_in_one_change() {
        let mut form = form(FULL, 0);
        form.set_description("Ship it @alice");
        form.remove_owner("bob");
        assert!(form.add_owner("priya"));
        form.set_status(StatusType::Cancelled);
        form.set_priority(Some(Priority::Highest));
        form.set_due("+2w");
        assert_eq!(
            compound(&form),
            [
                // `@bob`, the line's second owner, removed where it is.
                Change::RemoveOwnerAt(1),
                Change::SetDescription("Ship it @alice".into()),
                Change::AddOwner("priya".into()),
                Change::SetStatus('-'),
                Change::SetPriority(Some(Priority::Highest)),
                Change::SetDue(Some(day("2026-12-03"))),
            ]
        );
        assert_eq!(
            form.written().unwrap(),
            "- [-] Ship it @alice @priya 🔺 📅 2026-12-03 ❌ 2026-11-19 ^r1"
        );
        assert_eq!(form.owners(), ["alice", "priya"]);
        assert_eq!(
            form.stamp(),
            Some((DateField::Cancelled, day("2026-11-19")))
        );
    }

    #[test]
    fn removing_an_owner_of_the_description_edits_its_text() {
        let mut form = form("- [ ] Call @Sam about @sam, the deck @priya", 0);
        assert_eq!(form.owners(), ["Sam", "priya"]);
        form.remove_owner("sam");
        assert_eq!(form.description(), "Call about, the deck @priya");
        assert_eq!(form.owners(), ["priya"]);
        // Typed back into the description, it is the line's again.
        form.set_description("Call @sam");
        assert_eq!(form.owners(), ["sam"]);
        assert_eq!(
            compound(&form),
            [Change::SetDescription("Call @sam".into())]
        );
    }

    #[test]
    fn an_owner_removed_and_added_back_keeps_its_mention() {
        let line = "- [ ] Call @Sam tomorrow";
        for again in ["sam", "SAM", "Sam"] {
            let mut form = form(line, 0);
            form.remove_owner("Sam");
            assert_eq!(form.description(), "Call tomorrow");
            assert!(form.add_owner(again), "{again}");
            assert_eq!(form.description(), "Call @Sam tomorrow");
            assert_eq!(compound(&form), [], "{again}");
            assert_eq!(form.written().unwrap(), line);
        }
        // Once the description was edited after, it is added at its end.
        let mut form = form(line, 0);
        form.remove_owner("sam");
        form.set_description("Call tomorrow at ten");
        assert!(form.add_owner("sam"));
        assert_eq!(form.written().unwrap(), "- [ ] Call tomorrow at ten @sam");
    }

    #[test]
    fn an_owner_being_typed_takes_its_removal_back() {
        let line = "- [ ] Call @Sam tomorrow";
        let mut form = form(line, 0);
        form.remove_owner("Sam");
        assert_eq!(form.preview(), "- [ ] Call tomorrow");
        form.set_pending_owner("sam");
        assert_eq!(form.preview(), line);
        assert_eq!(form.written().unwrap(), line);
        assert_eq!(compound(&form), []);
    }

    #[test]
    fn removals_are_each_their_own_and_come_back_in_either_order() {
        let line = "- [ ] Ask @Sam and @priya now";
        for first in ["sam", "priya"] {
            let mut form = form(line, 0);
            form.remove_owner("Sam");
            form.remove_owner("priya");
            assert_eq!(form.description(), "Ask and now");
            assert!(form.add_owner(first));
            let expected = if first == "sam" {
                "- [ ] Ask @Sam and now"
            } else {
                "- [ ] Ask and @priya now"
            };
            assert_eq!(form.written().unwrap(), expected, "{first}");
            assert!(form.add_owner(if first == "sam" { "PRIYA" } else { "SAM" }));
            assert_eq!(form.written().unwrap(), line, "{first}");
            assert_eq!(compound(&form), []);
        }
    }

    #[test]
    fn a_mention_typed_in_the_description_is_the_users_own() {
        // The token `@sam` removed, then `@samantha` typed a letter at a
        // time: `@sam` on the way takes nothing back.
        let mut form = form("- [ ] Call ⏫ @sam", 0);
        form.remove_owner("sam");
        for typed in ["Call @s", "Call @sam", "Call @samantha"] {
            form.set_description(typed);
        }
        assert_eq!(form.written().unwrap(), "- [ ] Call @samantha ⏫");
        assert_eq!(form.owners(), ["samantha"]);
        // A mention typed stays where it is typed, the token still gone.
        form.set_description("Call @sam first");
        assert_eq!(form.written().unwrap(), "- [ ] Call @sam first ⏫");
    }

    #[test]
    fn an_owner_added_and_typed_in_the_description_is_written_once() {
        let mut form = form("- [ ] Call", 0);
        assert!(form.add_owner("Sam"));
        form.set_description("Call @sam");
        assert_eq!(form.owners(), ["sam"]);
        assert_eq!(
            compound(&form),
            [Change::SetDescription("Call @sam".into())]
        );
        assert_eq!(form.written().unwrap(), "- [ ] Call @sam");
        // And the other way round, an owner typed being added.
        let mut form = self::form("- [ ] Call @priya", 0);
        form.set_pending_owner("@Priya");
        assert_eq!(form.preview(), "- [ ] Call @priya");
        assert_eq!(compound(&form), []);
    }

    #[test]
    fn an_owner_being_typed_is_written_and_saved_as_typed() {
        let mut form = form("- [ ] Call 📅 2026-12-01", 0);
        form.set_pending_owner("@Sam");
        assert_eq!(form.preview(), "- [ ] Call @Sam 📅 2026-12-01");
        assert_eq!(form.written().unwrap(), form.preview());
        assert_eq!(
            form.owners(),
            Vec::<String>::new(),
            "no chip while it is typed"
        );
        form.set_pending_owner("a@b");
        assert_eq!(form.preview(), "- [ ] Call 📅 2026-12-01", "no owner name");
        assert_eq!(form.pending_owner(), "a@b");
    }

    #[test]
    fn owners_added_and_removed_compare_without_case() {
        let mut form = form("- [ ] Plan @bob", 0);
        assert!(!form.add_owner("Bob"), "there already");
        assert!(!form.add_owner("a@b"), "no owner name");
        assert!(!form.add_owner(""), "no owner name");
        assert!(form.add_owner("Sam"));
        assert!(!form.add_owner("sam"));
        form.remove_owner("SAM");
        assert_eq!(compound(&form), []);
        // An owner among the tokens, removed and added back.
        let mut form = self::form("- [ ] Plan 📅 2026-12-01 @bob", 0);
        form.remove_owner("bob");
        assert_eq!(compound(&form), [Change::RemoveOwnerAt(0)]);
        assert_eq!(form.preview(), "- [ ] Plan 📅 2026-12-01");
        assert!(form.add_owner("BOB"));
        assert_eq!(compound(&form), []);
    }

    /// `@bob` of `@sam@bob` is an owner only once `@sam` is removed: its
    /// chip's remove takes it out too.
    /// One click removes the mentions its chip stood for: `@sam`, which
    /// removing `@Sam` revealed, stays for a click of its own.
    #[test]
    fn a_click_removes_only_the_mentions_its_chip_showed() {
        let mut form = form("- [ ] Call @Sam@sam", 0);
        assert_eq!(form.owners(), ["Sam"]);
        form.remove_owner("Sam");
        assert_eq!(form.owners(), ["sam"]);
        assert_eq!(form.preview(), "- [ ] Call @sam");
        form.remove_owner("sam");
        assert_eq!(form.owners(), Vec::<String>::new());
        assert_eq!(form.written().unwrap(), "- [ ] Call");
    }

    #[test]
    fn an_owner_a_removal_revealed_is_removed_too() {
        let mut form = form("- [ ] Call @sam@bob", 0);
        form.remove_owner("sam");
        assert_eq!(form.owners(), ["bob"]);
        form.remove_owner("bob");
        assert_eq!(form.owners(), Vec::<String>::new());
        assert_eq!(form.written().unwrap(), "- [ ] Call");
    }

    #[test]
    fn a_due_field_that_names_no_date_is_a_problem() {
        let mut form = form("- [ ] Plan 📅 2026-12-01", 0);
        form.set_due("someday");
        assert_eq!(form.change(), Err(Problem::NotADate("someday".into())));
        assert_eq!(form.written(), Err(Problem::NotADate("someday".into())));
        // The preview keeps the line's own date meanwhile.
        assert_eq!(form.preview(), "- [ ] Plan 📅 2026-12-01");
        form.set_due("2026-02-30");
        assert_eq!(form.change(), Err(Problem::NotReal("2026-02-30".into())));
        assert_eq!(
            Problem::NotADate("x".into()).to_string(),
            "Not a date. Try fri, +3d or 2026-12-01."
        );
        // A date that is no real day, as the line has it, is kept.
        let mut form = self::form("- [ ] Plan 📅 2026-02-30", 0);
        assert_eq!(form.due_value(), Ok(Some(Parsed::Invalid)));
        form.set_priority(Some(Priority::Medium));
        assert_eq!(form.written().unwrap(), "- [ ] Plan 🔼 📅 2026-02-30");
        form.pick_due(Some(day("2026-03-01")));
        assert_eq!(form.due(), "2026-03-01");
        assert_eq!(form.written().unwrap(), "- [ ] Plan 🔼 📅 2026-03-01");
    }

    #[test]
    fn a_blank_line_makes_a_new_task_in_its_container() {
        for (text, line, written) in [
            ("", 0, "- [ ] Call the bank 📅 2026-11-20"),
            ("# Notes\n\nprose", 1, "- [ ] Call the bank 📅 2026-11-20"),
            ("> quoted\n> ", 1, "> - [ ] Call the bank 📅 2026-11-20"),
            ("- a\n- ", 1, "- [ ] Call the bank 📅 2026-11-20"),
            ("  - a\n  - ", 1, "  - [ ] Call the bank 📅 2026-11-20"),
            ("1. a\n2. ", 1, "2. [ ] Call the bank 📅 2026-11-20"),
        ] {
            let mut form = form(text, line);
            assert_eq!(form.kind(), FormKind::New, "{text:?}");
            assert_eq!(form.line(), line);
            assert!(form.writes(), "a new task is written as it is");
            form.set_description("Call the bank");
            form.set_due("fri");
            assert_eq!(form.written().unwrap(), written, "{text:?}");
        }
        // Nothing filled in still writes the box.
        assert_eq!(form("", 0).written().unwrap(), "- [ ] ");
    }

    #[test]
    fn a_line_of_text_becomes_a_task_with_its_text() {
        for (text, line, description, written) in [
            (
                "Call the bank",
                0,
                "Call the bank",
                "- [ ] Call the bank ⏫",
            ),
            ("- a list item", 0, "a list item", "- [ ] a list item ⏫"),
            ("> - quoted", 0, "quoted", "> - [ ] quoted ⏫"),
            (
                "Pay rent 📅 2026-12-01",
                0,
                "Pay rent",
                "- [ ] Pay rent ⏫ 📅 2026-12-01",
            ),
        ] {
            let mut form = form(text, line);
            assert_eq!(form.kind(), FormKind::Make, "{text}");
            assert_eq!(form.description(), description);
            form.set_priority(Some(Priority::High));
            assert_eq!(form.written().unwrap(), written);
        }
        // A line of an item's text after its first: the item gets the box.
        let form = form("- first\n  second", 1);
        assert_eq!((form.kind(), form.line()), (FormKind::Make, 0));
        assert_eq!(form.description(), "first");
    }

    #[test]
    fn a_line_of_a_tasks_text_opens_its_task() {
        let form = form("- [ ] Plan ⏫\n  more of it", 1);
        assert_eq!((form.kind(), form.line()), (FormKind::Edit, 0));
        assert_eq!(form.expect(), "- [ ] Plan ⏫");
    }

    #[test]
    fn no_form_where_no_task_can_go_or_with_tasks_off() {
        for (text, line) in [
            ("# Heading", 0),
            ("- [ ] Parent\n  # Heading", 1),
            ("- [ ] Parent\n  ***", 1),
            ("- Parent\n  ---", 1),
            ("- Parent\n  ## Heading", 1),
            ("Setext\n======", 0),
            ("Setext\n======", 1),
            ("Two lines\nof Setext\n---", 1),
            ("---", 0),
            ("```\n- [ ] fenced\n```", 1),
            ("```\n\n```", 1),
            ("    indented code", 0),
        ] {
            assert_eq!(
                Form::open(&Document::new(text), line, &opts()),
                None,
                "{text}"
            );
        }
        assert_eq!(
            Form::open(&Document::new("- [ ] a"), 0, &Options::default()),
            None
        );
        assert_eq!(Form::open(&Document::new("a"), 1, &opts()), None);
    }

    #[test]
    fn a_form_holds_while_its_line_reads_as_it_did() {
        let text = "# Tasks\n- [ ] Plan\n\nprose";
        for line in [1, 2, 3] {
            let form = form(text, line);
            assert!(form.holds(&Document::new(text), &opts()), "{line}");
            for changed in [
                "# Tasks\n- [ ] Plan!\n\nprose\n",
                "# Tasks\n- [x] Plan\n\nx\nprose",
                "# Tasks\n\n- [ ] Plan\n\nprose",
                "```\n- [ ] Plan\n\nprose",
            ] {
                let now = Document::new(changed);
                if Form::open(&now, line, &opts()).as_ref() != Some(&form) {
                    assert!(!form.holds(&now, &opts()), "{line} {changed:?}");
                }
            }
        }
        let form = form(text, 1);
        assert!(!form.holds(&Document::new("# Tasks\n- [ ] Plan!"), &opts()));
        assert!(!form.holds(&Document::new("# Tasks"), &opts()));
    }

    #[test]
    fn a_status_change_stamps_with_today() {
        let mut form = form("- [ ] Plan", 0);
        form.set_today(day("2026-11-21"));
        form.set_status(StatusType::Done);
        assert_eq!(form.written().unwrap(), "- [x] Plan ✅ 2026-11-21");
        assert_eq!(form.stamp(), Some((DateField::Done, day("2026-11-21"))));
        // Words in the due field read from the new today.
        form.set_due("tomorrow");
        assert_eq!(form.due_date(), Some(day("2026-11-22")));
    }
}
