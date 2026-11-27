//! Writing a task from outside the editor (discovery/tasks.md §10,
//! discovery/tasks-build.md item 9.1): a row of the task view, or a note
//! in the Notes column, whose task or task switch is changed while its
//! text is not what the user is typing in.
//!
//! Every such write goes through [`App::write_task_row`] or
//! [`App::set_note_tasks_hidden`], and a note has one writer:
//!
//! - **The open note** is written by its editor. The change is planned on
//!   the editor's text (`wren_core::task_write`), applied to it as one
//!   undo step, and reaches the Session as any edit does, by `EditDraft`
//!   ([`App::text_changed`]), so `sync_editor` finds the texts equal and
//!   the editor keeps its undo history. The caret and the selection stay
//!   on the text they were on, and the view where it was scrolled to.
//!   The note's file is looked at first: one another program has written
//!   since the Session last read or saved it is taken into the editor
//!   before the change is planned, or, over a draft, blocks the change.
//! - **A note that is not open** is written by the Session
//!   (`Op::EditTaskLine`, `Op::EditTaskSwitch`), which settles the open
//!   document first and writes with a compare-and-swap.
//!
//! Both plan with the same rules, so a row reads the same whichever note
//! it is in: the line is found where the row has it or at the one place a
//! task of that text is now, and a row whose task has changed is refused
//! with "This task changed; the list was refreshed" ([`Refusal::refresh`]).
//!
//! Each write that changed something leaves its inverse on a stack of the
//! app's ([`App::undo_task_write`]): the line or the front matter the
//! write left is expected, and what was there before is written back,
//! through the same two paths. It is the view's undo, not the editor's:
//! an inverse applied to the open note is one more step of the editor's
//! own history.

use std::time::Instant;

use iced::widget::text_editor::{Cursor, Position};
use wren_core::fs_store::RelPath;
use wren_core::kind::NoteKind;
use wren_core::model::NoteId;
use wren_core::ops::{Document, Op};
use wren_core::task_write::{self, LineChange, Refused, SwitchChange};
use wren_core::tasks::TaskRecord;
use wren_markdown::task::{self, Change};
use wren_markdown::Pos;

use super::commands::{Disposition, Gateway};
use super::{App, Message};

/// How many writes can be undone.
const MAX_UNDO: usize = 100;

/// The task a change is for: a row as the task index gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowTarget {
    pub note: NoteId,
    /// The note's path when the row was listed: what the row shows. The
    /// note is found by its id, which a rename inside Wren keeps.
    pub path: RelPath,
    /// The line the task was on, from 0.
    pub line: usize,
    /// That line as it read, without its ending.
    pub expect: String,
}

impl RowTarget {
    pub(super) fn of(record: &TaskRecord) -> Self {
        Self {
            note: record.note,
            path: record.path.clone(),
            line: record.line,
            expect: record.source.clone(),
        }
    }
}

/// What to do to a row's task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RowChange {
    /// A change planned on the line as it is found; a stamp it writes is
    /// dated [`App::today`].
    Change(Change),
    /// The line itself, without an ending.
    Line(String),
}

/// A row's task as a write left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowWrite {
    pub note: NoteId,
    /// The note's path now.
    pub path: RelPath,
    /// The line the task is on now, from 0.
    pub line: usize,
    /// The line as it read.
    pub before: String,
    /// The line as it reads now: `before` when nothing changed.
    pub after: String,
    /// Whether anything was written. A change that changes nothing (a
    /// Highest task raised) is no write and leaves nothing to undo.
    pub changed: bool,
    /// Whether the note is the open one, written by its editor: it is a
    /// draft until autosave saves it.
    pub open: bool,
    /// What went wrong after the write was made: its durability is
    /// unconfirmed.
    pub warning: Option<String>,
}

impl RowWrite {
    /// The row as it reads now: what the next change to it, made before
    /// the list is refreshed, is for.
    pub(super) fn target(&self) -> RowTarget {
        RowTarget {
            note: self.note,
            path: self.path.clone(),
            line: self.line,
            expect: self.after.clone(),
        }
    }
}

/// A note's task switch as a write left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SwitchWrite {
    pub note: NoteId,
    pub path: RelPath,
    /// The front matter as it read, from `---` through its closing line;
    /// empty for none.
    pub before: String,
    /// The front matter now.
    pub after: String,
    pub changed: bool,
    pub open: bool,
    pub warning: Option<String>,
}

/// Why nothing was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Refusal {
    pub message: String,
    /// The task or the front matter is not what the change was made for:
    /// whatever lists the note's tasks should list them afresh.
    pub refresh: bool,
}

impl Refusal {
    fn blocked(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            refresh: false,
        }
    }

    fn stale(message: &str) -> Self {
        Self {
            message: message.into(),
            refresh: true,
        }
    }

    fn of(refused: Refused, stale: &str) -> Self {
        match refused {
            Refused::Changed => Self::stale(stale),
            Refused::Invalid(why) => Self::blocked(why),
        }
    }
}

/// A write as the app's actions report one: what it wrote or why it did
/// not, and the action's gateway, whose task the caller runs.
pub(super) struct TaskWrite<T> {
    pub result: Result<T, Refusal>,
    pub gateway: Gateway,
}

impl<T> TaskWrite<T> {
    /// This write with `first` run before its own task: what was settled
    /// on the way to it.
    fn after(mut self, first: iced::Task<Message>) -> Self {
        let own = std::mem::replace(&mut self.gateway.task, iced::Task::none());
        self.gateway.task = iced::Task::batch([first, own]);
        self
    }
}

/// The inverse of a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Inverse {
    /// The line the write left, and the line to write back.
    Line { target: RowTarget, text: String },
    /// The front matter the write left, and what to put back.
    Switch {
        note: NoteId,
        expect: String,
        replace: String,
    },
}

/// What undoing a write wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Undone {
    Line(RowWrite),
    Switch(SwitchWrite),
}

/// What the Session's events said of the task write in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Seen {
    Line {
        line: usize,
        before: String,
        after: String,
    },
    Switch {
        before: String,
        after: String,
    },
    /// The task was found, with nothing to change.
    Kept {
        line: usize,
        source: String,
    },
    Stale,
}

/// The app's task writes: what can be undone, newest last, and what the
/// write in progress has heard from the Session.
#[derive(Debug, Default)]
pub(super) struct Writes {
    undo: Vec<Inverse>,
    pub(super) seen: Option<Seen>,
    /// How many inverses were ever recorded, which names the newest.
    recorded: u64,
}

/// Names the write that is next to undo, so that something offering to
/// undo one write (a message's Undo) never undoes another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WriteMark {
    recorded: u64,
    depth: usize,
}

impl Writes {
    fn record(&mut self, inverse: Inverse) {
        if self.undo.len() == MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(inverse);
        self.recorded += 1;
    }

    /// The mark of the write next to undo, if there is one. A write
    /// recorded or undone since moves it.
    pub(super) fn mark(&self) -> Option<WriteMark> {
        (!self.undo.is_empty()).then_some(WriteMark {
            recorded: self.recorded,
            depth: self.undo.len(),
        })
    }
}

fn pos(position: Position) -> Pos {
    Pos::new(position.line, position.column)
}

fn position(pos: Pos) -> Position {
    Position {
        line: pos.line,
        column: pos.col,
    }
}

/// `cursor` with each of its ends moved by `moved`.
fn mapped(cursor: Cursor, moved: impl Fn(Pos) -> Pos) -> Cursor {
    Cursor {
        position: position(moved(pos(cursor.position))),
        selection: cursor.selection.map(|end| position(moved(pos(end)))),
    }
}

impl App {
    /// Change the task of the row `target`: the one function every write
    /// to a task from outside the editor goes through. The open note is
    /// changed through its editor, any other by the Session. A change
    /// that is written can be undone with [`App::undo_task_write`].
    pub(super) fn write_task_row(
        &mut self,
        target: &RowTarget,
        change: &RowChange,
    ) -> TaskWrite<RowWrite> {
        let written = self.apply_row(target, change);
        if let Ok(write) = &written.result {
            if write.changed {
                self.task_writes.record(Inverse::Line {
                    target: write.target(),
                    text: write.before.clone(),
                });
            }
        }
        written
    }

    /// "Hide tasks in this note", and its reverse: write `tasks: off` into
    /// the note's front matter, or take it out, through the same two
    /// paths as a row's change. A note that already reads so is left as
    /// it is. Undone with [`App::undo_task_write`].
    pub(super) fn set_note_tasks_hidden(
        &mut self,
        note: NoteId,
        hidden: bool,
    ) -> TaskWrite<SwitchWrite> {
        let change = if hidden {
            SwitchChange::Hide
        } else {
            SwitchChange::Show
        };
        let written = self.apply_switch(note, &change);
        if let Ok(write) = &written.result {
            if write.changed {
                self.task_writes.record(Inverse::Switch {
                    note,
                    expect: write.after.clone(),
                    replace: write.before.clone(),
                });
            }
        }
        written
    }

    /// Whether a task write is left to undo.
    pub(super) fn can_undo_task_write(&self) -> bool {
        !self.task_writes.undo.is_empty()
    }

    /// Undo the last task write that has not been undone: its inverse,
    /// through the same path, expecting what the write left. Undone or
    /// found changed, the write is forgotten, so the next undo is of the
    /// one before; an undo that could not run for another reason (the
    /// open note's draft could not be saved) is kept to try again.
    pub(super) fn undo_task_write(&mut self) -> TaskWrite<Undone> {
        let Some(inverse) = self.task_writes.undo.last().cloned() else {
            // Not worth a banner: the key was pressed once too often.
            let refusal = Refusal::blocked("Nothing to undo");
            let gateway = self.blocked(refusal.message.clone());
            return TaskWrite {
                result: Err(refusal),
                gateway,
            };
        };
        let (result, gateway) = match &inverse {
            Inverse::Line { target, text } => {
                let done = self.apply_row(target, &RowChange::Line(text.clone()));
                (done.result.map(Undone::Line), done.gateway)
            }
            Inverse::Switch {
                note,
                expect,
                replace,
            } => {
                let change = SwitchChange::Restore {
                    expect: expect.clone(),
                    replace: replace.clone(),
                };
                let done = self.apply_switch(*note, &change);
                (done.result.map(Undone::Switch), done.gateway)
            }
        };
        let settled = match &result {
            Ok(_) => true,
            Err(refusal) => refusal.refresh,
        };
        if settled {
            self.task_writes.undo.pop();
        }
        TaskWrite { result, gateway }
    }

    fn task_write_refused<T>(&mut self, refusal: Refusal) -> TaskWrite<T> {
        self.banner = Some(refusal.message.clone());
        self.banner_kind = super::workspace::WarningKind::General;
        let gateway = self.blocked(refusal.message.clone());
        TaskWrite {
            result: Err(refusal),
            gateway,
        }
    }

    /// The note a write is for and whether it is the open one, or why
    /// nothing can be written now.
    ///
    /// `settled` takes the task of whatever had to be settled first (the
    /// open note's reconciliation with its file), which the caller runs
    /// whether or not anything is then written.
    fn task_write_note(
        &mut self,
        note: NoteId,
        stale: &str,
        settled: &mut iced::Task<Message>,
    ) -> Result<(RelPath, bool), Refusal> {
        if !self.preferences.tasks {
            return Err(Refusal::blocked("Tasks are off"));
        }
        if self.loading.is_some() {
            return Err(Refusal::blocked(
                "A workspace is loading; wait for it to finish",
            ));
        }
        let Some(session) = &self.session else {
            return Err(Refusal::blocked("No workspace is open"));
        };
        let Some(entry) = session.model().note(note) else {
            return Err(Refusal::stale(stale));
        };
        // Before either writer is chosen: a board that does not parse is
        // shown as text in the editor, and is no note of tasks for that.
        if entry.kind != NoteKind::Markdown {
            return Err(Refusal::blocked("Only a Markdown note holds tasks"));
        }
        let path = entry.path.clone();
        let open = session.document().map(|document| document.note) == Some(note);
        if open {
            // The editor is the open note's writer only while it holds it.
            self.editor_holds_note().map_err(Refusal::blocked)?;
            if self.editor_note != Some(note) {
                return Err(Refusal::blocked("The editor does not hold the open note"));
            }
            self.open_note_as_on_disk(note, stale, settled)?;
        }
        // A done or cancelled stamp is today's, though the day changed
        // since the timer last looked.
        self.check_day();
        Ok((path, open))
    }

    /// Before the open note is changed through its editor: its file is
    /// still what the Session read or last saved, or the two are brought
    /// in step. Another program may have written the file since, with
    /// the next reconciliation yet to come, and a change planned on the
    /// editor's text would then pass for one made to the note as it is
    /// (an undo would be reported done, of a task that has changed).
    ///
    /// With no draft, the editor takes the file's text here, and the
    /// change is planned on that. With a draft, the two are in conflict,
    /// which is the document's own reconciliation to settle: nothing is
    /// written, and the draft is kept.
    ///
    /// The reconciliation is an action of its own, whose task goes to
    /// `settled`: it may close a task view whose item is gone, and then
    /// hands the keyboard to the note, which must happen whatever becomes
    /// of the write.
    fn open_note_as_on_disk(
        &mut self,
        note: NoteId,
        stale: &str,
        settled: &mut iced::Task<Message>,
    ) -> Result<(), Refusal> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        if !(session.document_changed_on_disk()).map_err(Refusal::blocked)? {
            return Ok(());
        }
        if session.document().is_some_and(Document::dirty) {
            return Err(Refusal::blocked(
                "The open note changed outside Wren; nothing was written, and its draft is kept",
            ));
        }
        let mut reconciled = self.process_core(Op::Reconcile);
        *settled = std::mem::replace(&mut reconciled.task, iced::Task::none());
        if let Some(error) = closed_error(&reconciled) {
            return Err(Refusal::blocked(error));
        }
        // Removed outside Wren, it is no longer the open note.
        let open = (self.session.as_ref())
            .and_then(|session| session.document())
            .is_some_and(|document| document.note == note);
        if open && self.editor_note == Some(note) {
            Ok(())
        } else {
            Err(Refusal::stale(stale))
        }
    }

    fn apply_row(&mut self, target: &RowTarget, change: &RowChange) -> TaskWrite<RowWrite> {
        let mut settled = iced::Task::none();
        let found = self.task_write_note(target.note, task_write::CHANGED, &mut settled);
        let written = match found {
            Ok((path, open)) => self.apply_row_on(target, change, path, open),
            Err(refusal) => self.task_write_refused(refusal),
        };
        written.after(settled)
    }

    /// [`App::apply_row`] to the note at `path`, the open one or not.
    fn apply_row_on(
        &mut self,
        target: &RowTarget,
        change: &RowChange,
        path: RelPath,
        open: bool,
    ) -> TaskWrite<RowWrite> {
        let line_change = match change {
            RowChange::Change(change) => LineChange::Change {
                change: change.clone(),
                today: self.today(),
            },
            RowChange::Line(text) => LineChange::Line(text.clone()),
        };
        let unchanged = |warning| RowWrite {
            note: target.note,
            path: path.clone(),
            line: target.line,
            before: target.expect.clone(),
            after: target.expect.clone(),
            changed: false,
            open,
            warning,
        };
        if open {
            return match self.edit_open_row(target, change, &line_change) {
                Ok((line, before, after)) => {
                    let gateway =
                        self.finish_action(Disposition::Completed, None, iced::Task::none());
                    let write = RowWrite {
                        line,
                        changed: before != after,
                        before,
                        after,
                        ..unchanged(None)
                    };
                    TaskWrite {
                        result: Ok(write),
                        gateway,
                    }
                }
                Err(refusal) => self.task_write_refused(refusal),
            };
        }
        self.task_writes.seen = None;
        let gateway = self.process_core(Op::EditTaskLine {
            note: target.note,
            line: target.line,
            expect: target.expect.clone(),
            change: line_change,
        });
        let result = match self.task_writes.seen.take() {
            Some(Seen::Line {
                line,
                before,
                after,
            }) => Ok(RowWrite {
                line,
                before,
                after,
                changed: true,
                ..unchanged(gateway.processed.warning.clone())
            }),
            // Found, perhaps on another line than the row has, with
            // nothing to change.
            Some(Seen::Kept { line, source }) => Ok(RowWrite {
                line,
                before: source.clone(),
                after: source,
                ..unchanged(None)
            }),
            Some(Seen::Stale) => Err(Refusal::stale(task_write::CHANGED)),
            Some(Seen::Switch { .. }) | None => match closed_error(&gateway) {
                Some(error) => Err(Refusal::blocked(error)),
                None => Ok(unchanged(None)),
            },
        };
        TaskWrite { result, gateway }
    }

    /// Apply a row's change to the open note's editor. Returns the line
    /// the task is on with its text before and after, which are the same
    /// when nothing changes.
    fn edit_open_row(
        &mut self,
        target: &RowTarget,
        change: &RowChange,
        line_change: &LineChange,
    ) -> Result<(usize, String, String), Refusal> {
        let started = Instant::now();
        let source = self.editor.text();
        let plan = task_write::plan_line(&source, target.line, &target.expect, line_change)
            .map_err(|refused| Refusal::of(refused, task_write::CHANGED))?;
        if !plan.changes() {
            return Ok((plan.line, plan.before, plan.after));
        }
        // The view first: taking it gives a double click's selection its
        // bounds, which planning at the selection would otherwise move.
        let view = self.editor.viewport();
        let applied = match change {
            // The editor's own line edit, as a menu's row makes it: the
            // caret and the selection keep the text they are on.
            RowChange::Change(change) => {
                let edit = self.editor.plan_markdown(|lines, selection, options| {
                    task::edit(lines, plan.line, selection, change, options)
                });
                match edit {
                    Some(edit) if edit.line == plan.line && edit.text == plan.after => {
                        self.editor.apply_line_edits(&[edit]).unwrap_or(false)
                    }
                    _ => false,
                }
            }
            RowChange::Line(_) => {
                let landing = mapped(view.cursor(), |at| plan.moved(at));
                let replaces: Vec<wren_editor::Replace> =
                    plan.replaces().into_iter().map(Into::into).collect();
                self.editor.apply(&replaces, landing)
            }
        };
        self.editor
            .restore_viewport(view.with_cursor(self.editor.cursor()));
        if !applied {
            return Err(Refusal::blocked("The editor could not make that change"));
        }
        self.text_changed(started);
        Ok((plan.line, plan.before, plan.after))
    }

    fn apply_switch(&mut self, note: NoteId, change: &SwitchChange) -> TaskWrite<SwitchWrite> {
        let mut settled = iced::Task::none();
        let found = self.task_write_note(note, task_write::SWITCH_CHANGED, &mut settled);
        let written = match found {
            Ok((path, open)) => self.apply_switch_on(note, change, path, open),
            Err(refusal) => self.task_write_refused(refusal),
        };
        written.after(settled)
    }

    /// [`App::apply_switch`] to the note at `path`, the open one or not.
    fn apply_switch_on(
        &mut self,
        note: NoteId,
        change: &SwitchChange,
        path: RelPath,
        open: bool,
    ) -> TaskWrite<SwitchWrite> {
        let write = |seen: Option<(String, String)>, warning| {
            let changed = seen.is_some();
            let (before, after) = seen.unwrap_or_default();
            SwitchWrite {
                note,
                path: path.clone(),
                before,
                after,
                changed,
                open,
                warning,
            }
        };
        if open {
            return match self.edit_open_switch(change) {
                Ok(seen) => {
                    let gateway =
                        self.finish_action(Disposition::Completed, None, iced::Task::none());
                    TaskWrite {
                        result: Ok(write(seen, None)),
                        gateway,
                    }
                }
                Err(refusal) => self.task_write_refused(refusal),
            };
        }
        self.task_writes.seen = None;
        let gateway = self.process_core(Op::EditTaskSwitch {
            note,
            change: change.clone(),
        });
        let result = match self.task_writes.seen.take() {
            Some(Seen::Switch { before, after }) => Ok(write(
                Some((before, after)),
                gateway.processed.warning.clone(),
            )),
            Some(Seen::Stale) => Err(Refusal::stale(task_write::SWITCH_CHANGED)),
            Some(Seen::Line { .. } | Seen::Kept { .. }) | None => match closed_error(&gateway) {
                Some(error) => Err(Refusal::blocked(error)),
                None => Ok(write(None, None)),
            },
        };
        TaskWrite { result, gateway }
    }

    /// Change the open note's front matter in its editor. Returns the
    /// front matter before and after, or `None` when nothing changes.
    fn edit_open_switch(
        &mut self,
        change: &SwitchChange,
    ) -> Result<Option<(String, String)>, Refusal> {
        let started = Instant::now();
        let source = self.editor.text();
        let plan = task_write::plan_switch(&source, change)
            .map_err(|refused| Refusal::of(refused, task_write::SWITCH_CHANGED))?;
        let Some(plan) = plan else {
            return Ok(None);
        };
        let view = self.editor.viewport();
        let landing = mapped(view.cursor(), |at| plan.moved(&source, at));
        // The lines go in with the endings the plan gives them, which are
        // the ones the Session writes to a note that is not open: front
        // matter put back is put back byte for byte. Lines taken from two
        // places are two edits made together, one step to undo.
        let replaces: Vec<wren_editor::Replace> = (plan.replaces(&source).into_iter())
            .map(Into::into)
            .collect();
        let applied = self.editor.apply_exact(&replaces, landing);
        self.editor
            .restore_viewport(view.with_cursor(self.editor.cursor()));
        if !applied {
            return Err(Refusal::blocked("The editor could not make that change"));
        }
        self.text_changed(started);
        // As the editor holds it, which is what an undo will find there.
        let after = task_write::front_matter_of(&self.editor.text()).to_owned();
        Ok(Some((plan.before, after)))
    }

    /// Forget what could be undone: another workspace is open.
    pub(super) fn clear_task_writes(&mut self) {
        self.task_writes = Writes::default();
    }

    /// What a note's task write reads from the Session's events
    /// (`process_events`).
    pub(super) fn task_write_seen(&mut self, seen: Seen) {
        self.task_writes.seen = Some(seen);
    }
}

/// Why the Session wrote nothing, when it did not.
fn closed_error(gateway: &Gateway) -> Option<String> {
    gateway.core_error.clone().or_else(|| {
        (gateway.processed.disposition == Disposition::Blocked)
            .then(|| gateway.processed.warning.clone())
            .flatten()
    })
}

/// The harness's side of `tasks.view` (`tasks_harness.rs`): a row's
/// change, hiding a note's tasks, and undo, each through the function the
/// task view's own keys and menus call.
#[cfg(feature = "smoke-test")]
mod harness {
    use serde_json::{json, Value};
    use wren_markdown::date::Date;
    use wren_markdown::task::{Change, Priority};

    use super::super::test_requests::string;
    use super::*;
    use crate::test_ipc::Request;

    /// A `change` parameter: `"toggle"`, `"cycle"`, `"raise"`, `"lower"`,
    /// or an object with one of `status`, `due`, `priority`, `add_owner`,
    /// `remove_owner`, `description`, or `line` (the line itself).
    fn row_change(value: &Value) -> Result<RowChange, String> {
        if let Some(name) = value.as_str() {
            return Ok(RowChange::Change(match name {
                "toggle" => Change::ToggleDone,
                "cycle" => Change::Cycle,
                "raise" => Change::RaisePriority,
                "lower" => Change::LowerPriority,
                other => return Err(format!("unknown change {other:?}")),
            }));
        }
        let fields = value
            .as_object()
            .filter(|fields| fields.len() == 1)
            .ok_or("change must be a name or an object with one field")?;
        let (name, value) = fields.iter().next().expect("one field");
        let text = || {
            value
                .as_str()
                .ok_or_else(|| format!("{name} must be a string"))
        };
        Ok(RowChange::Change(match name.as_str() {
            "line" => return Ok(RowChange::Line(text()?.to_owned())),
            "status" => {
                let mut symbols = text()?.chars();
                match (symbols.next(), symbols.next()) {
                    (Some(symbol), None) => Change::SetStatus(symbol),
                    _ => return Err("status must be one character".into()),
                }
            }
            "due" if value.is_null() => Change::SetDue(None),
            "due" => Change::SetDue(Some(
                Date::parse_iso(text()?).ok_or("due must be YYYY-MM-DD or null")?,
            )),
            "priority" if value.is_null() => Change::SetPriority(None),
            "priority" => {
                let wanted = text()?;
                let level = (Priority::ALL.into_iter())
                    .find(|level| level.name().eq_ignore_ascii_case(wanted))
                    .ok_or_else(|| format!("unknown priority {wanted:?}"))?;
                Change::SetPriority(Some(level))
            }
            "add_owner" => Change::AddOwner(text()?.to_owned()),
            "remove_owner" => Change::RemoveOwner(text()?.to_owned()),
            "description" => Change::SetDescription(text()?.to_owned()),
            other => return Err(format!("unknown change {other:?}")),
        }))
    }

    fn row_json(write: &RowWrite) -> Value {
        json!({
            "path": write.path.as_path().to_string_lossy(),
            "line": write.line,
            "before": write.before,
            "after": write.after,
            "changed": write.changed,
            "open": write.open,
            "warning": write.warning,
        })
    }

    fn switch_json(write: &SwitchWrite) -> Value {
        json!({
            "path": write.path.as_path().to_string_lossy(),
            "before": write.before,
            "after": write.after,
            "changed": write.changed,
            "open": write.open,
            "warning": write.warning,
        })
    }

    impl App {
        fn note_at(&self, path: &str) -> Result<NoteId, String> {
            let path = RelPath::new(path).map_err(|error| error.to_string())?;
            (self.session.as_ref())
                .and_then(|session| session.model().notes.iter().find(|note| note.path == path))
                .map(|note| note.id)
                .ok_or_else(|| format!("no note at {}", path.as_path().display()))
        }

        /// The row a `write` names: with `expect`, as given, which a test
        /// uses for a row that is out of date; without, the task the
        /// index has on that line.
        fn row_target(&self, write: &Value) -> Result<RowTarget, String> {
            let note = self.note_at(string(write, "path")?)?;
            let line = (write.get("line").and_then(Value::as_u64))
                .and_then(|line| usize::try_from(line).ok())
                .ok_or("line must be usize")?;
            let tasks = (self.session.as_ref())
                .and_then(|session| session.tasks())
                .ok_or("No task index")?;
            let path = RelPath::new(string(write, "path")?).map_err(|error| error.to_string())?;
            if let Some(expect) = write.get("expect") {
                let expect = expect.as_str().ok_or("expect must be a string")?.to_owned();
                return Ok(RowTarget {
                    note,
                    path,
                    line,
                    expect,
                });
            }
            let found = (tasks.tasks_under(Some(&path))).find(|task| task.line == line);
            found
                .map(RowTarget::of)
                .ok_or_else(|| format!("no task on line {line}"))
        }

        /// What the index holds of the note at `path` now.
        fn note_tasks_json(&self, path: &RelPath) -> Value {
            let Some(session) = &self.session else {
                return Value::Null;
            };
            let Some(tasks) = session.tasks() else {
                return Value::Null;
            };
            let note = session.model().notes.iter().find(|note| note.path == *path);
            let flags = note.and_then(|note| tasks.flags(note.id));
            json!({
                "path": path.as_path().to_string_lossy(),
                "hidden": flags.is_some_and(|flags| flags.hidden),
                "tasks": tasks
                    .tasks_under(Some(path))
                    .map(|task| json!({"line": task.line, "source": task.source}))
                    .collect::<Vec<_>>(),
            })
        }

        /// `tasks.view` with `write`, `hide` or `undo`: the write, then
        /// what the view's snapshot says with the write's outcome beside
        /// it. `None` for a request that asks for none of them.
        pub(in super::super) fn task_write_request(
            &mut self,
            request: &Request,
        ) -> Result<Option<iced::Task<super::super::Message>>, String> {
            let write = request.params.get("write");
            let hide = request.params.get("hide");
            let undo = request.params.get("undo").and_then(Value::as_bool) == Some(true);
            if write.is_none() && hide.is_none() && !undo {
                return Ok(None);
            }
            // Off, each answers as the plain request does, before
            // anything of it is read.
            if !self.preferences.tasks {
                request.respond(Ok(self.task_view_snapshot()));
                return Ok(Some(iced::Task::none()));
            }
            let (mut row, mut switch) = (None, None);
            // The note the request is for, written or not.
            let attempted;
            let (result, gateway) = if let Some(write) = write {
                let target = self.row_target(write)?;
                let change = row_change(write.get("change").ok_or("change is missing")?)?;
                attempted = Some(target.note);
                let done = self.write_task_row(&target, &change);
                (done.result.map(|write| row = Some(write)), done.gateway)
            } else if let Some(hide) = hide {
                let note = self.note_at(string(hide, "path")?)?;
                let hidden = (hide.get("hidden").and_then(Value::as_bool))
                    .ok_or("hidden must be a boolean")?;
                attempted = Some(note);
                let done = self.set_note_tasks_hidden(note, hidden);
                (done.result.map(|write| switch = Some(write)), done.gateway)
            } else {
                attempted = self.task_writes.undo.last().map(|inverse| match inverse {
                    Inverse::Line { target, .. } => target.note,
                    Inverse::Switch { note, .. } => *note,
                });
                let done = self.undo_task_write();
                let result = done.result.map(|undone| match undone {
                    Undone::Line(write) => row = Some(write),
                    Undone::Switch(write) => switch = Some(write),
                });
                (result, done.gateway)
            };
            // Where the note is now, if it still is: a refused write
            // says what the index holds of it too, which is the refreshed
            // list a stale row is told to take.
            let path = (self.session.as_ref())
                .zip(attempted)
                .and_then(|(session, note)| session.model().note(note))
                .map(|note| note.path.clone());
            let mut answer = self.task_view_snapshot();
            answer["write"] = row.as_ref().map_or(Value::Null, row_json);
            answer["switch"] = switch.as_ref().map_or(Value::Null, switch_json);
            answer["refused"] = match &result {
                Ok(()) => Value::Null,
                Err(refusal) => json!({"message": refusal.message, "refresh": refusal.refresh}),
            };
            answer["note"] = path.map_or(Value::Null, |path| self.note_tasks_json(&path));
            answer["can_undo"] = json!(self.can_undo_task_write());
            answer["disposition"] = json!(gateway.processed.disposition.key());
            answer["warning"] = json!(gateway.processed.warning);
            request.respond(Ok(answer));
            Ok(Some(gateway.task))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use iced::widget::text_editor::{Action, Edit, Motion};
    use wren_core::fs_store::Fault;
    use wren_core::ops::Session;
    use wren_markdown::date::Date;
    use wren_markdown::task::Priority;

    use super::super::board::tests::FLOW;
    use super::super::board::Surface;
    use super::super::commands::{Command, CommandId};
    use super::super::tasks_view::Scope;
    use super::super::{Message, Options};
    use super::*;

    const MAIL: &str = "- [ ] Mail the deck 📅 2026-12-01";
    const MAILED: &str = "- [x] Mail the deck 📅 2026-12-01 ✅ 2026-11-19";
    const A: &str = "# A\n\n- [ ] Mail the deck 📅 2026-12-01\n- [ ] Book the room\n";
    const B: &str = "# B\n\n- [ ] Order lunch\n";
    const LUNCH: &str = "- [ ] Order lunch";
    const ON: &str = "tasks = on\nboards = on\n";

    /// An app on a workspace whose group `Work/Tasks` holds the notes `A`
    /// and `B` with the texts given and the board `Flow`, and whose group
    /// `Work/Other` holds the note `C`, with `A` open and today pinned to
    /// Thursday 19 November 2026.
    fn fixture_with(settings: &str, a: &str, b: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        for (path, text) in [
            ("Work/Tasks/A.md", a),
            ("Work/Tasks/B.md", b),
            ("Work/Tasks/Flow.wren-board", FLOW),
            ("Work/Other/C.md", "- [ ] in another group\n"),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let config = directory.path().join("config.conf");
        std::fs::write(&config, settings).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.workspace_mode = true;
        app.session = Some(Session::open_with(&root, app.session_options()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        app.pin_today(Date::new(2026, 11, 19).unwrap());
        open(&mut app, "A");
        (directory, app)
    }

    fn fixture() -> (tempfile::TempDir, App) {
        fixture_with(ON, A, B)
    }

    /// The fixture on each engine.
    fn engines() -> impl Iterator<Item = (tempfile::TempDir, App)> {
        engines_with(A, B)
    }

    /// The fixture with those notes on each engine.
    fn engines_with<'a>(
        a: &'a str,
        b: &'a str,
    ) -> impl Iterator<Item = (tempfile::TempDir, App)> + 'a {
        wren_editor::BackendKind::ALL.into_iter().map(move |kind| {
            let engine = match kind {
                wren_editor::BackendKind::Iced => "classic",
                wren_editor::BackendKind::Owned => "advanced",
            };
            let (directory, app) = fixture_with(&format!("editor-engine = {engine}\n{ON}"), a, b);
            assert_eq!(app.editor.backend(), kind);
            (directory, app)
        })
    }

    fn file(directory: &tempfile::TempDir, title: &str) -> PathBuf {
        (directory.path()).join(format!("workspace/Work/Tasks/{title}.md"))
    }

    fn on_disk(directory: &tempfile::TempDir, title: &str) -> String {
        std::fs::read_to_string(file(directory, title)).unwrap()
    }

    fn note(app: &App, title: &str) -> NoteId {
        (app.model().unwrap().notes.iter())
            .find(|note| note.title == title)
            .unwrap_or_else(|| panic!("no note titled {title}"))
            .id
    }

    fn open(app: &mut App, title: &str) {
        let target = app.note_target(note(app, title));
        let outcome = app.dispatch(Command::OpenNote(target));
        assert!(outcome.processed.disposition.committed());
    }

    /// The task view of the fixture's group, over the open note.
    fn show(app: &mut App) {
        let scope = Scope::Group(RelPath::new("Work/Tasks").unwrap());
        let outcome = app.dispatch(Command::ShowTasks(scope));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.surface(), Surface::Tasks);
    }

    fn back(app: &mut App) {
        let outcome = app.dispatch(Command::Run(CommandId::Back));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
    }

    /// The row of the task on `line` of the note, as the index lists it.
    fn row(app: &App, title: &str, line: usize) -> RowTarget {
        let path = RelPath::new(format!("Work/Tasks/{title}.md")).unwrap();
        let tasks = app.session.as_ref().unwrap().tasks().unwrap();
        let found = tasks
            .tasks_under(Some(&path))
            .find(|task| task.line == line);
        RowTarget::of(found.unwrap_or_else(|| panic!("no task on line {line} of {title}")))
    }

    /// The index's lines of the note, each as `line source`.
    fn listed(app: &App, title: &str) -> Vec<String> {
        let path = RelPath::new(format!("Work/Tasks/{title}.md")).unwrap();
        let tasks = app.session.as_ref().unwrap().tasks().unwrap();
        tasks
            .tasks_under(Some(&path))
            .map(|task| format!("{} {}", task.line, task.source))
            .collect()
    }

    fn hidden(app: &App, title: &str) -> bool {
        let tasks = app.session.as_ref().unwrap().tasks().unwrap();
        tasks.flags(note(app, title)).unwrap().hidden
    }

    fn change(change: Change) -> RowChange {
        RowChange::Change(change)
    }

    fn toggle(app: &mut App, target: &RowTarget) -> Result<RowWrite, Refusal> {
        app.write_task_row(target, &change(Change::ToggleDone))
            .result
    }

    /// The done toggle on the row the index lists for that line.
    fn toggle_row(app: &mut App, title: &str, line: usize) -> Result<RowWrite, Refusal> {
        let target = row(app, title, line);
        toggle(app, &target)
    }

    fn undo(app: &mut App) -> Result<Undone, Refusal> {
        app.undo_task_write().result
    }

    fn draft(app: &App) -> String {
        let document = app.session.as_ref().unwrap().document();
        document.map_or_else(String::new, |document| document.text.clone())
    }

    fn dirty(app: &App) -> bool {
        let document = app.session.as_ref().unwrap().document();
        document.is_some_and(wren_core::ops::Document::dirty)
    }

    fn paste(text: &str) -> Message {
        Message::Editor(wren_editor::Message::Action(Action::Edit(Edit::Paste(
            Arc::new(text.into()),
        ))))
    }

    fn editor_undo(app: &mut App) {
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
    }

    fn at(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    fn fail_next_write(app: &mut App) {
        app.session.as_mut().unwrap().fail_next(Fault::Write);
    }

    fn stale() -> Refusal {
        Refusal::stale(task_write::CHANGED)
    }

    /// The contract's sequence (discovery/tasks-build.md item 9.1): type in
    /// note A, change A from the view, change the closed note B, undo in
    /// the view, return to A.
    #[test]
    fn a_note_typed_in_then_changed_from_the_view_keeps_its_history_selection_and_bytes() {
        for (directory, mut app) in engines() {
            let kind = app.editor.backend();
            let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
                Motion::DocumentEnd,
            ))));
            let _ = app.update(paste("- [ ] typed"));
            let typed = format!("{A}- [ ] typed");
            assert_eq!(app.editor.text(), typed);
            // "Mail" selected, on the line the view will change.
            let selected = Cursor {
                position: Position {
                    line: 2,
                    column: 10,
                },
                selection: Some(Position { line: 2, column: 6 }),
            };
            app.editor.move_cursor(selected);
            show(&mut app);
            assert_eq!(
                on_disk(&directory, "A"),
                typed,
                "{kind:?}: the view flushed"
            );

            // A, the open note, through its editor.
            let wrote = toggle_row(&mut app, "A", 2).unwrap();
            let toggled = typed.replace(MAIL, MAILED);
            assert_eq!(
                (
                    wrote.line,
                    &*wrote.before,
                    &*wrote.after,
                    wrote.changed,
                    wrote.open
                ),
                (2, MAIL, MAILED, true, true),
                "{kind:?}"
            );
            assert_eq!(wrote.path, RelPath::new("Work/Tasks/A.md").unwrap());
            assert_eq!(app.editor.text(), toggled, "{kind:?}");
            assert_eq!(draft(&app), toggled, "{kind:?}: by EditDraft");
            assert!(dirty(&app), "{kind:?}");
            assert_eq!(
                on_disk(&directory, "A"),
                typed,
                "{kind:?}: a draft until saved"
            );
            assert_eq!(
                app.editor.cursor(),
                selected,
                "{kind:?}: the selection keeps its text"
            );
            assert_eq!(app.surface(), Surface::Tasks);
            assert!(app.ensure_editor_ready().is_err(), "{kind:?}: still hidden");
            // The index has the line as written, so the row is found again.
            assert_eq!(row(&app, "A", 2).expect, MAILED);

            // B, which is not open, through the Session: A's draft is
            // settled first.
            let cycle = change(Change::Cycle);
            let wrote = app
                .write_task_row(&row(&app, "B", 2), &cycle)
                .result
                .unwrap();
            assert_eq!(
                (
                    wrote.line,
                    &*wrote.before,
                    &*wrote.after,
                    wrote.changed,
                    wrote.open
                ),
                (2, LUNCH, "- [/] Order lunch", true, false),
                "{kind:?}"
            );
            assert_eq!(on_disk(&directory, "B"), B.replace("[ ]", "[/]"));
            assert_eq!(listed(&app, "B"), ["2 - [/] Order lunch"]);
            assert_eq!(on_disk(&directory, "A"), toggled, "{kind:?}: settled first");
            assert!(!dirty(&app), "{kind:?}");
            assert_eq!(
                app.editor.text(),
                toggled,
                "{kind:?}: the editor was not reloaded"
            );

            // Undo in the view takes back the last write, B's.
            let Ok(Undone::Line(undone)) = undo(&mut app) else {
                panic!("{kind:?}: the undo was refused");
            };
            assert_eq!(
                (&*undone.before, &*undone.after),
                ("- [/] Order lunch", LUNCH)
            );
            assert_eq!(on_disk(&directory, "B"), B, "{kind:?}");
            assert_eq!(listed(&app, "B"), [format!("2 {LUNCH}")]);
            assert_eq!(app.editor.text(), toggled, "{kind:?}");

            // Back in A: its text, selection, draft and bytes.
            back(&mut app);
            assert_eq!(app.surface(), Surface::Text);
            assert_eq!(app.editor_note, Some(note(&app, "A")));
            assert_eq!(app.editor.text(), toggled, "{kind:?}");
            assert_eq!(app.editor.cursor(), selected, "{kind:?}");
            assert_eq!(draft(&app), toggled, "{kind:?}");
            assert!(!dirty(&app), "{kind:?}");
            assert_eq!(on_disk(&directory, "A"), toggled, "{kind:?}");
            // And its undo history: the view's change is one step, the
            // typing before it another.
            editor_undo(&mut app);
            assert_eq!(
                app.editor.text(),
                typed,
                "{kind:?}: one step for the view's change"
            );
            assert_eq!(draft(&app), typed, "{kind:?}");
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), A, "{kind:?}: then the typing");
            assert_eq!(draft(&app), A, "{kind:?}");
            assert!(!app.editor.can_undo(), "{kind:?}");

            // The view's own undo of A's change finds the line as the
            // editor's undo left it, and is refused and forgotten.
            assert!(app.can_undo_task_write());
            assert_eq!(undo(&mut app), Err(stale()), "{kind:?}");
            assert!(!app.can_undo_task_write());
            assert_eq!(app.editor.text(), A, "{kind:?}");
        }
    }

    #[test]
    fn the_views_undo_of_the_open_note_is_one_more_step_of_the_editors_history() {
        for (_directory, mut app) in engines() {
            let kind = app.editor.backend();
            // The caret in the description, on the line to change.
            app.editor.move_cursor(at(2, 10));
            show(&mut app);
            toggle_row(&mut app, "A", 2).unwrap();
            assert_eq!(app.editor.cursor(), at(2, 10), "{kind:?}");
            let Ok(Undone::Line(undone)) = undo(&mut app) else {
                panic!("{kind:?}: the undo was refused");
            };
            assert!(undone.open && undone.changed);
            assert_eq!((&*undone.before, &*undone.after), (MAILED, MAIL));
            assert_eq!(app.editor.text(), A, "{kind:?}");
            assert_eq!(draft(&app), A, "{kind:?}");
            // The stamp went from the end of the line and the box changed:
            // the caret between them is where it was.
            assert_eq!(app.editor.cursor(), at(2, 10), "{kind:?}");
            assert!(!app.can_undo_task_write());
            assert_eq!(undo(&mut app).unwrap_err().message, "Nothing to undo");

            // The editor's history holds both, each one step.
            back(&mut app);
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), A.replace(MAIL, MAILED), "{kind:?}");
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), A, "{kind:?}");
            assert!(!app.editor.can_undo(), "{kind:?}");
        }
    }

    #[test]
    fn the_caret_and_selection_move_only_with_text_on_the_changed_line() {
        for (_directory, mut app) in engines() {
            let kind = app.editor.backend();
            // On another line: unchanged, whatever the change.
            let elsewhere = Cursor {
                position: Position { line: 3, column: 9 },
                selection: Some(Position { line: 0, column: 2 }),
            };
            app.editor.move_cursor(elsewhere);
            show(&mut app);
            let high = change(Change::SetPriority(Some(Priority::High)));
            let wrote = app
                .write_task_row(&row(&app, "A", 2), &high)
                .result
                .unwrap();
            assert_eq!(wrote.after, "- [ ] Mail the deck ⏫ 📅 2026-12-01");
            assert_eq!(app.editor.cursor(), elsewhere, "{kind:?}");
            assert!(undo(&mut app).is_ok());
            assert_eq!(app.editor.cursor(), elsewhere, "{kind:?}");

            // At the end of the line, after the due date: the priority
            // goes in before it, and the caret stays after its date.
            back(&mut app);
            app.editor.move_cursor(at(2, MAIL.len()));
            show(&mut app);
            let wrote = app
                .write_task_row(&row(&app, "A", 2), &high)
                .result
                .unwrap();
            assert_eq!(app.editor.cursor(), at(2, wrote.after.len()), "{kind:?}");
            // Undone, it is at the end again.
            assert!(undo(&mut app).is_ok());
            assert_eq!(app.editor.cursor(), at(2, MAIL.len()), "{kind:?}");
        }
    }

    #[test]
    fn a_row_out_of_date_is_refused_and_nothing_is_written() {
        let (directory, mut app) = fixture();
        show(&mut app);
        // A form or a menu opened for a row holds the line it read. By
        // the time it writes, the line reads differently.
        for title in ["A", "B"] {
            let mut held = row(&app, title, 2);
            held.expect.push_str(" as it was");
            let refused = toggle(&mut app, &held);
            assert_eq!(refused, Err(stale()), "{title}");
            assert_eq!(app.banner.as_deref(), Some(task_write::CHANGED), "{title}");
            app.banner = None;
        }
        assert_eq!(app.editor.text(), A);
        assert!(!app.editor.can_undo() && !dirty(&app));
        assert_eq!(
            (on_disk(&directory, "A"), on_disk(&directory, "B")),
            (A.into(), B.into())
        );
        assert!(!app.can_undo_task_write());

        // A note that is gone from the tree.
        let mut gone = row(&app, "B", 2);
        gone.note = NoteId(9_999);
        assert_eq!(toggle(&mut app, &gone), Err(stale()));
        // A change the task refuses is no stale row.
        let odd = change(Change::SetStatus(']'));
        for title in ["A", "B"] {
            let refused = app
                .write_task_row(&row(&app, title, 2), &odd)
                .result
                .unwrap_err();
            assert!(!refused.refresh, "{title}: {refused:?}");
        }
        // One that changes nothing writes nothing, and leaves no undo.
        let same = change(Change::SetDue(Date::new(2026, 12, 1)));
        let kept = app
            .write_task_row(&row(&app, "A", 2), &same)
            .result
            .unwrap();
        assert!(!kept.changed && kept.open);
        let kept = app.write_task_row(&row(&app, "B", 2), &change(Change::SetDue(None)));
        assert!(!kept.result.unwrap().changed);
        assert!(!app.can_undo_task_write());
        assert!(!app.editor.can_undo() && !dirty(&app));
    }

    #[test]
    fn a_second_action_on_a_row_is_for_the_line_the_first_wrote() {
        let (directory, mut app) = fixture();
        show(&mut app);
        for title in ["A", "B"] {
            // The row as the list holds it until it is refreshed.
            let held = row(&app, title, 2);
            let first = toggle(&mut app, &held).unwrap();
            let raise = change(Change::RaisePriority);
            let second = app.write_task_row(&first.target(), &raise).result.unwrap();
            assert_eq!(second.before, first.after, "{title}");
            assert!(second.after.contains('🔼'), "{title}: {}", second.after);
            assert_eq!(listed(&app, title)[0], format!("2 {}", second.after));
            // The row as first listed is out of date now.
            assert_eq!(toggle(&mut app, &held), Err(stale()), "{title}");
            app.banner = None;
            // Two undos take both back.
            assert!(undo(&mut app).is_ok() && undo(&mut app).is_ok());
            assert_eq!(listed(&app, title)[0], format!("2 {}", held.expect));
        }
        assert!(!app.can_undo_task_write());
        assert_eq!(app.editor.text(), A);
        assert_eq!(on_disk(&directory, "B"), B);
    }

    #[test]
    fn an_undo_takes_the_path_the_note_is_on_when_it_runs() {
        let (directory, mut app) = fixture();
        show(&mut app);
        // Written through the editor while A was open, and through the
        // Session while B was not.
        assert!(toggle_row(&mut app, "A", 2).unwrap().open);
        assert!(!toggle_row(&mut app, "B", 2).unwrap().open);
        // Now B is the open note and A is not.
        open(&mut app, "B");
        assert_eq!(on_disk(&directory, "A"), A.replace(MAIL, MAILED));
        show(&mut app);
        let Ok(Undone::Line(undone)) = undo(&mut app) else {
            panic!("the undo was refused");
        };
        assert!(undone.open, "B's, through its editor");
        assert_eq!(app.editor.text(), B);
        assert!(app.editor.can_undo());
        let Ok(Undone::Line(undone)) = undo(&mut app) else {
            panic!("the undo was refused");
        };
        assert!(!undone.open, "A's, by the Session");
        assert_eq!(on_disk(&directory, "A"), A);
        assert_eq!(on_disk(&directory, "B"), B, "settled before A was written");
        assert!(!dirty(&app) && !app.can_undo_task_write());
    }

    #[test]
    fn an_edit_made_outside_before_undo_refuses_the_undo() {
        let (directory, mut app) = fixture();
        show(&mut app);
        // The closed note.
        toggle_row(&mut app, "B", 2).unwrap();
        let theirs = on_disk(&directory, "B").replace("lunch", "lunches");
        std::fs::write(file(&directory, "B"), &theirs).unwrap();
        assert_eq!(undo(&mut app), Err(stale()));
        assert_eq!(app.banner.as_deref(), Some(task_write::CHANGED));
        assert_eq!(on_disk(&directory, "B"), theirs, "their bytes stand");
        assert!(
            listed(&app, "B")[0].contains("lunches"),
            "the list is refreshed"
        );
        assert!(!app.can_undo_task_write(), "the undo is forgotten");

        // The open note, saved and then changed on disk, with no
        // reconciliation yet: the editor still holds the line the undo
        // expects. The undo looks at the file through the Session first,
        // so the editor takes their text and the undo is of a task that
        // has changed.
        toggle_row(&mut app, "A", 2).unwrap();
        assert!(app.process_core(Op::Flush).core_error.is_none());
        let ours = app.editor.text();
        let theirs = ours.replace("Mail the deck", "Mail the decks");
        std::fs::write(file(&directory, "A"), &theirs).unwrap();
        assert_eq!(app.editor.text(), ours, "not reconciled yet");
        assert_eq!(undo(&mut app), Err(stale()));
        assert_eq!(app.editor.text(), theirs);
        assert_eq!(draft(&app), theirs);
        assert_eq!(on_disk(&directory, "A"), theirs);
        assert!(!dirty(&app) && !app.can_undo_task_write());
        assert!(listed(&app, "A")[0].contains("decks"));
        // A change for the note as it is now goes through.
        assert!(toggle_row(&mut app, "A", 2).unwrap().changed);
        assert!(undo(&mut app).is_ok());
        assert_eq!(app.editor.text(), theirs);
    }

    #[test]
    fn a_draft_in_conflict_with_the_file_is_kept_and_nothing_is_written_over_it() {
        let (directory, mut app) = fixture();
        show(&mut app);
        // A change from the view, still a draft, and then another
        // program's write to the same note.
        toggle_row(&mut app, "A", 2).unwrap();
        let ours = A.replace(MAIL, MAILED);
        assert!(dirty(&app));
        let theirs = A.replace("Book the room", "Book the hall");
        std::fs::write(file(&directory, "A"), &theirs).unwrap();
        // Neither an undo nor another change is made on top of it: that
        // is for the document's own reconciliation to settle.
        let refused = undo(&mut app).unwrap_err();
        assert!(!refused.refresh, "{refused:?}");
        assert!(refused.message.contains("outside Wren"), "{refused:?}");
        let target = row(&app, "A", 3);
        let refused = toggle(&mut app, &target).unwrap_err();
        assert!(!refused.refresh, "{refused:?}");
        let hide = app.set_note_tasks_hidden(note(&app, "A"), true);
        assert!(!hide.result.unwrap_err().refresh);
        assert_eq!(app.editor.text(), ours);
        assert_eq!(draft(&app), ours, "the draft is kept");
        assert!(dirty(&app));
        assert_eq!(on_disk(&directory, "A"), theirs);
        assert!(app.can_undo_task_write(), "kept to try again");
        // Another note is still written, once the draft is settled, which
        // here means its conflict copy.
        assert_eq!(app.surface(), Surface::Tasks);
    }

    #[test]
    fn a_board_shown_as_text_is_no_note_of_tasks() {
        let (directory, mut app) = fixture();
        let flow = (directory.path()).join("workspace/Work/Tasks/Flow.wren-board");
        let broken = "- [ ] not a board\n";
        std::fs::write(&flow, broken).unwrap();
        assert!(app.process_core(Op::Reconcile).core_error.is_none());
        open(&mut app, "Flow");
        // It does not parse, so the editor shows its text.
        assert_eq!(app.surface(), Surface::Raw);
        assert_eq!(app.editor.text(), broken);
        let board = note(&app, "Flow");
        let hide = app.set_note_tasks_hidden(board, true);
        let refused = hide.result.unwrap_err();
        assert!(
            !refused.refresh && refused.message.contains("Markdown"),
            "{refused:?}"
        );
        let target = RowTarget {
            note: board,
            path: RelPath::new("Work/Tasks/Flow.wren-board").unwrap(),
            line: 0,
            expect: "- [ ] not a board".into(),
        };
        let refused = toggle(&mut app, &target).unwrap_err();
        assert!(refused.message.contains("Markdown"), "{refused:?}");
        assert_eq!(app.editor.text(), broken);
        assert!(!app.editor.can_undo() && !dirty(&app));
        assert!(!app.can_undo_task_write());
        assert_eq!(std::fs::read_to_string(&flow).unwrap(), broken);
    }

    #[test]
    fn showing_is_refused_when_it_would_make_front_matter_of_the_body() {
        let stacked = "---\ntasks: off\n---\n---\ntitle: keep\n---\nbody\n";
        let (directory, mut app) = fixture_with(ON, stacked, stacked);
        for title in ["A", "B"] {
            let shown = app.set_note_tasks_hidden(note(&app, title), false);
            let refused = shown.result.unwrap_err();
            assert!(
                !refused.refresh && refused.message.contains("front matter"),
                "{title}: {refused:?}"
            );
            assert!(hidden(&app, title), "{title}");
        }
        assert_eq!(app.editor.text(), stacked);
        assert!(!app.editor.can_undo() && !dirty(&app));
        assert_eq!(on_disk(&directory, "B"), stacked);
        assert!(!app.can_undo_task_write());
    }

    #[test]
    fn front_matter_put_back_in_the_open_note_keeps_its_line_endings() {
        // A CRLF block over an LF body.
        let mixed = "---\r\ntasks: off\r\n---\r\n- [ ] a\nbody\n";
        for (directory, mut app) in engines_with(mixed, mixed) {
            let kind = app.editor.backend();
            let (a, b) = (note(&app, "A"), note(&app, "B"));
            assert_eq!(app.editor.text(), mixed);
            for id in [a, b] {
                assert!(app.set_note_tasks_hidden(id, false).result.unwrap().changed);
            }
            assert_eq!(app.editor.text(), "- [ ] a\nbody\n", "{kind:?}");
            assert_eq!(on_disk(&directory, "B"), "- [ ] a\nbody\n");
            // Undone, each is as it was byte for byte, the open note
            // through its editor.
            assert!(matches!(undo(&mut app), Ok(Undone::Switch(write)) if !write.open));
            assert_eq!(on_disk(&directory, "B"), mixed, "{kind:?}");
            assert!(matches!(undo(&mut app), Ok(Undone::Switch(write)) if write.open));
            assert_eq!(app.editor.text(), mixed, "{kind:?}");
            assert_eq!(draft(&app), mixed, "{kind:?}");
            // Each of the two changes is one step of the editor's history.
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), "- [ ] a\nbody\n", "{kind:?}");
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), mixed, "{kind:?}");
            assert!(!app.editor.can_undo(), "{kind:?}");
        }
    }

    #[test]
    fn front_matter_is_not_put_back_where_it_would_run_into_what_was_typed_since() {
        // The block is the whole note, with no ending after it.
        let only = "---\ntasks: off\n---";
        let (_directory, mut app) = fixture_with(ON, only, B);
        let a = note(&app, "A");
        assert!(app.set_note_tasks_hidden(a, false).result.unwrap().changed);
        assert_eq!(app.editor.text(), "");
        let _ = app.update(paste("- [ ] typed\n"));
        let refused = undo(&mut app).unwrap_err();
        assert_eq!(refused, Refusal::stale(task_write::SWITCH_CHANGED));
        assert_eq!(app.editor.text(), "- [ ] typed\n");
        assert!(!hidden(&app, "A") && !app.can_undo_task_write());

        // White space typed since would sit after `---` on a line that
        // still closed the block, for the next change to take away.
        for typed in [" \t\nbody\n", " "] {
            let (directory, mut app) = fixture_with(ON, only, only);
            for title in ["A", "B"] {
                let shown = app.set_note_tasks_hidden(note(&app, title), false);
                assert!(shown.result.unwrap().changed);
            }
            app.editor.replace_text(typed, true);
            app.text_changed(Instant::now());
            std::fs::write(file(&directory, "B"), typed).unwrap();
            // The closed note's, then the open one's through its editor.
            for _ in 0..2 {
                let refused = undo(&mut app).unwrap_err();
                assert_eq!(
                    refused,
                    Refusal::stale(task_write::SWITCH_CHANGED),
                    "{typed:?}"
                );
            }
            assert_eq!(app.editor.text(), typed);
            assert_eq!(on_disk(&directory, "B"), typed);
        }
    }

    #[test]
    fn both_writers_refuse_a_block_whose_last_line_break_would_run_into_the_bodys() {
        let front = "---\ntasks: off\n---\nbody\n";
        for (directory, mut app) in engines_with(front, front) {
            let kind = app.editor.backend();
            for title in ["B", "A"] {
                let shown = app.set_note_tasks_hidden(note(&app, title), false);
                assert!(shown.result.unwrap().changed);
            }
            // Since then each body has gained a CR-only blank line at its
            // top. The block's last LF and that CR would read as one line
            // ending: the Session would write a line fewer, and the
            // editor would have to change the CR to keep them apart.
            let odd = "\rbody\n";
            app.editor.replace_text(odd, true);
            app.text_changed(Instant::now());
            std::fs::write(file(&directory, "B"), odd).unwrap();
            for _ in 0..2 {
                let refused = undo(&mut app).unwrap_err();
                assert_eq!(
                    refused,
                    Refusal::stale(task_write::SWITCH_CHANGED),
                    "{kind:?}"
                );
            }
            assert_eq!(app.editor.text(), odd, "{kind:?}");
            assert_eq!(draft(&app), odd, "{kind:?}");
            assert_eq!(on_disk(&directory, "B"), odd, "{kind:?}");
            assert!(!app.can_undo_task_write());
        }
    }

    #[test]
    fn hiding_is_refused_where_the_note_may_have_a_tasks_key_wren_does_not_read() {
        for front in [
            // Front matter to another reader, though not to Wren: no
            // second block goes above it.
            "---\n\"tasks\": [a, b]\n---\n- [ ] a\n",
            // Front matter to both, with a key in a spelling Wren cannot
            // tell from `tasks`.
            "---\ntitle: A\n\"\\u0074asks\": [a, b]\n---\n- [ ] a\n",
            "---\ntitle: A\n{tasks: [a]}\n---\n- [ ] a\n",
        ] {
            let (directory, mut app) = fixture_with(ON, front, front);
            for title in ["A", "B"] {
                let hide = app.set_note_tasks_hidden(note(&app, title), true);
                let refused = hide.result.unwrap_err();
                assert!(!refused.refresh, "{title} {front:?}: {refused:?}");
                assert!(!hidden(&app, title), "{front:?}");
            }
            assert_eq!(app.editor.text(), front);
            assert!(!app.editor.can_undo() && !dirty(&app));
            assert_eq!(on_disk(&directory, "B"), front);
            assert!(!app.can_undo_task_write());
        }
    }

    #[test]
    fn showing_is_refused_when_a_mark_in_the_body_would_become_the_files() {
        let front = "---\ntasks: off\n---\n\u{feff}- [ ] a\n";
        let (directory, mut app) = fixture_with(ON, front, front);
        for title in ["A", "B"] {
            let shown = app.set_note_tasks_hidden(note(&app, title), false);
            let refused = shown.result.unwrap_err();
            assert!(
                !refused.refresh && refused.message.contains("byte order mark"),
                "{title}: {refused:?}"
            );
            assert!(hidden(&app, title));
        }
        assert_eq!(app.editor.text(), front);
        assert!(!app.editor.can_undo() && !dirty(&app));
        assert_eq!(on_disk(&directory, "B"), front);
        assert!(!app.can_undo_task_write());
    }

    #[test]
    fn what_the_disk_check_settles_hands_the_keyboard_on_whatever_becomes_of_the_write() {
        // Refused, and written.
        for refused in [true, false] {
            let (directory, mut app) = fixture();
            // The view of another group has the keyboard, over note A.
            let scope = Scope::Group(RelPath::new("Work/Other").unwrap());
            let shown = app.dispatch(Command::ShowTasks(scope));
            assert_eq!(shown.processed.disposition, Disposition::Completed);
            assert!(app.task_view_has_keys());
            toggle_row(&mut app, "A", 2).unwrap();
            assert!(app.process_core(Op::Flush).core_error.is_none());
            // Outside Wren that group goes, and A is written: its task
            // changed, or another line of it.
            std::fs::remove_dir_all(directory.path().join("workspace/Work/Other")).unwrap();
            let word = if refused {
                "Mail the deck"
            } else {
                "Book the room"
            };
            let theirs = on_disk(&directory, "A").replace(word, "theirs");
            std::fs::write(file(&directory, "A"), &theirs).unwrap();

            // The undo looks at A's file first. That reconciliation finds
            // the view's group gone, so the note takes the pane, and the
            // keyboard with it: a task the undo's own gateway carries.
            let done = app.undo_task_write();
            assert_eq!(app.surface(), Surface::Text, "the view went with its group");
            assert_eq!(done.result.is_err(), refused, "{:?}", done.result);
            assert!(
                done.gateway.task.units() > 0,
                "refused {refused}: the focus handoff is not dropped"
            );
            if refused {
                assert_eq!(done.result, Err(stale()));
                assert_eq!(app.editor.text(), theirs);
            } else {
                assert_eq!(app.editor.text(), theirs.replace(MAILED, MAIL));
            }
        }
        // With nothing to settle there is nothing to carry.
        let (_directory, mut app) = fixture();
        show(&mut app);
        let target = row(&app, "A", 2);
        let done = app.write_task_row(&target, &change(Change::ToggleDone));
        assert!(done.result.is_ok());
        assert_eq!(done.gateway.task.units(), 0);
    }

    #[test]
    fn lines_taken_from_two_places_leave_a_selection_between_them_as_it_was() {
        let front = "---\ntasks: off\ntitle: keep\ntasks: off\n---\n- [ ] a\n";
        for (_directory, mut app) in engines_with(front, B) {
            let kind = app.editor.backend();
            // `keep` selected, in the key that stays.
            app.editor.move_cursor(Cursor {
                position: Position {
                    line: 2,
                    column: 11,
                },
                selection: Some(Position { line: 2, column: 7 }),
            });
            let a = note(&app, "A");
            assert!(app.set_note_tasks_hidden(a, false).result.unwrap().changed);
            assert_eq!(app.editor.text(), "---\ntitle: keep\n---\n- [ ] a\n");
            let kept = Cursor {
                position: Position {
                    line: 1,
                    column: 11,
                },
                selection: Some(Position { line: 1, column: 7 }),
            };
            assert_eq!(app.editor.cursor(), kept, "{kind:?}");
            // Both lines went in one step, and come back in one.
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), front, "{kind:?}");
            assert!(!app.editor.can_undo(), "{kind:?}");
        }
    }

    #[test]
    fn a_caret_in_a_long_description_stays_put_through_a_change_and_its_undo() {
        let words = "word ".repeat(120);
        let long = format!("# A\n\n- [ ] {words}📅 2026-12-01\n");
        for (_directory, mut app) in engines_with(&long, B) {
            let kind = app.editor.backend();
            app.editor.move_cursor(at(2, 300));
            show(&mut app);
            assert!(toggle_row(&mut app, "A", 2).unwrap().changed);
            assert_eq!(app.editor.cursor(), at(2, 300), "{kind:?}");
            assert!(undo(&mut app).is_ok());
            assert_eq!(app.editor.text(), long, "{kind:?}");
            assert_eq!(app.editor.cursor(), at(2, 300), "{kind:?}");
        }
    }

    #[test]
    fn a_line_that_would_be_no_task_is_not_written() {
        let (directory, mut app) = fixture();
        show(&mut app);
        for title in ["A", "B"] {
            let target = row(&app, title, 2);
            let plain = RowChange::Line("plain text".into());
            let refused = app.write_task_row(&target, &plain).result.unwrap_err();
            assert!(!refused.refresh, "{title}: {refused:?}");
            assert_eq!(listed(&app, title)[0], format!("2 {}", target.expect));
        }
        assert_eq!(app.editor.text(), A);
        assert_eq!(on_disk(&directory, "B"), B);
        assert!(!app.editor.can_undo() && !app.can_undo_task_write());
    }

    #[test]
    fn a_change_of_nothing_to_a_closed_note_says_where_the_task_is_now() {
        let (directory, mut app) = fixture();
        show(&mut app);
        // The row as listed, before another program moves the task down.
        let held = row(&app, "B", 2);
        let moved = B.replace(LUNCH, &format!("new\nlines\n{LUNCH}"));
        std::fs::write(file(&directory, "B"), &moved).unwrap();
        let modified = std::fs::metadata(file(&directory, "B"))
            .unwrap()
            .modified()
            .unwrap();
        let kept = app.write_task_row(&held, &change(Change::SetDue(None)));
        let kept = kept.result.unwrap();
        assert_eq!(
            (kept.line, &*kept.before, &*kept.after, kept.changed),
            (4, LUNCH, LUNCH, false)
        );
        // Nothing was written, and the list has the note as it is.
        assert_eq!(on_disk(&directory, "B"), moved);
        assert_eq!(
            std::fs::metadata(file(&directory, "B"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert_eq!(listed(&app, "B"), [format!("4 {LUNCH}")]);
        assert!(!app.can_undo_task_write());
        // The row it gives is the one the next change is for.
        assert!(toggle(&mut app, &kept.target()).unwrap().changed);
    }

    #[test]
    fn a_draft_that_cannot_be_saved_blocks_a_write_to_another_note() {
        let (directory, mut app) = fixture();
        show(&mut app);
        toggle_row(&mut app, "A", 2).unwrap();
        let toggled = A.replace(MAIL, MAILED);
        assert!(dirty(&app));

        fail_next_write(&mut app);
        let refused = toggle_row(&mut app, "B", 2).unwrap_err();
        assert!(!refused.refresh, "{refused:?}");
        assert!(app.banner.is_some());
        assert_eq!(on_disk(&directory, "B"), B, "nothing was written");
        assert_eq!(on_disk(&directory, "A"), A);
        assert_eq!(draft(&app), toggled, "the draft is kept");
        assert_eq!(app.editor.text(), toggled);
        assert!(dirty(&app));
        assert_eq!(app.surface(), Surface::Tasks);

        // An undo that cannot run for that reason is kept: the last write
        // was A's, so make one to B first.
        let wrote = toggle_row(&mut app, "B", 2).unwrap();
        assert!(wrote.changed && !dirty(&app));
        assert_eq!(on_disk(&directory, "A"), toggled, "saved by the retry");
        let raise = change(Change::RaisePriority);
        assert!(app
            .write_task_row(&row(&app, "A", 2), &raise)
            .result
            .is_ok());
        assert!(dirty(&app));
        // Newest first: A's raise is undone through the editor, then B's
        // toggle needs the draft saved, which fails.
        assert!(matches!(undo(&mut app), Ok(Undone::Line(write)) if write.open));
        fail_next_write(&mut app);
        let refused = undo(&mut app).unwrap_err();
        assert!(!refused.refresh, "{refused:?}");
        assert!(app.can_undo_task_write(), "kept to try again");
        assert!(matches!(undo(&mut app), Ok(Undone::Line(write)) if !write.open));
        assert_eq!(on_disk(&directory, "B"), B);
    }

    #[test]
    fn hiding_a_notes_tasks_writes_its_front_matter_through_the_same_two_paths() {
        for (directory, mut app) in engines() {
            let kind = app.editor.backend();
            let (a, b) = (note(&app, "A"), note(&app, "B"));
            app.editor.move_cursor(at(2, 10));
            show(&mut app);

            // The open note, in its editor.
            let wrote = app.set_note_tasks_hidden(a, true).result.unwrap();
            let block = "---\ntasks: off\n---\n";
            assert!(wrote.changed && wrote.open);
            assert_eq!((&*wrote.before, &*wrote.after), ("", block), "{kind:?}");
            let hidden_a = format!("{block}{A}");
            assert_eq!(app.editor.text(), hidden_a, "{kind:?}");
            assert_eq!(draft(&app), hidden_a, "{kind:?}");
            assert_eq!(
                app.editor.cursor(),
                at(5, 10),
                "{kind:?}: the caret keeps its text"
            );
            // Its tasks leave the index at once.
            assert!(
                hidden(&app, "A") && listed(&app, "A").is_empty(),
                "{kind:?}"
            );
            // Hidden already: nothing to write, nothing to undo.
            let again = app.set_note_tasks_hidden(a, true).result.unwrap();
            assert!(!again.changed);

            // The closed note, by the Session.
            let wrote = app.set_note_tasks_hidden(b, true).result.unwrap();
            assert!(wrote.changed && !wrote.open);
            assert_eq!((&*wrote.before, &*wrote.after), ("", block), "{kind:?}");
            assert_eq!(on_disk(&directory, "B"), format!("{block}{B}"), "{kind:?}");
            assert!(
                hidden(&app, "B") && listed(&app, "B").is_empty(),
                "{kind:?}"
            );
            assert_eq!(
                on_disk(&directory, "A"),
                hidden_a,
                "{kind:?}: settled first"
            );

            // Undo, newest first: B's front matter, then A's.
            assert!(matches!(undo(&mut app), Ok(Undone::Switch(write)) if !write.open));
            assert_eq!(on_disk(&directory, "B"), B, "{kind:?}");
            assert!(!hidden(&app, "B"));
            assert_eq!(listed(&app, "B"), [format!("2 {LUNCH}")]);
            assert!(matches!(undo(&mut app), Ok(Undone::Switch(write)) if write.open));
            assert_eq!(app.editor.text(), A, "{kind:?}");
            assert_eq!(app.editor.cursor(), at(2, 10), "{kind:?}");
            assert!(!hidden(&app, "A"));
            assert_eq!(listed(&app, "A").len(), 2);
            assert!(!app.can_undo_task_write());

            // The editor's history holds A's two changes, one step each.
            back(&mut app);
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), hidden_a, "{kind:?}");
            editor_undo(&mut app);
            assert_eq!(app.editor.text(), A, "{kind:?}");
            assert!(!app.editor.can_undo(), "{kind:?}");
        }
    }

    #[test]
    fn the_editor_and_the_session_write_the_same_front_matter() {
        let sources = [
            "- [ ] a\n",
            "- [ ] a\r\n- [ ] b\r\n",
            "\u{feff}# T\r\n- [ ] a",
            "---\n# a rule, then a heading\n- [ ] a\n",
            "---\ntitle: T\n---\n- [ ] a\n",
            "\u{feff}---\r\ntitle: T\r\n...\r\n- [ ] a\r\n",
            "---\ntitle: T\n---",
            "---\ntasks: on\ntitle: T\n---\n- [ ] a\n",
            "",
        ];
        for source in sources {
            let (directory, mut app) = fixture_with(ON, source, source);
            let (a, b) = (note(&app, "A"), note(&app, "B"));
            assert_eq!(app.editor.text(), source);
            for hide in [true, false] {
                let open = app.set_note_tasks_hidden(a, hide).result.unwrap();
                let closed = app.set_note_tasks_hidden(b, hide).result.unwrap();
                assert!(open.changed && closed.changed, "{hide} {source:?}");
                assert_eq!(
                    (&open.before, &open.after),
                    (&closed.before, &closed.after),
                    "{hide} {source:?}"
                );
                assert_eq!(
                    app.editor.text(),
                    on_disk(&directory, "B"),
                    "{hide} {source:?}"
                );
                assert_eq!(hidden(&app, "A"), hide, "{source:?}");
                assert_eq!(hidden(&app, "B"), hide, "{source:?}");
            }
            // Undo of the two removals, then of the two additions, gives
            // each note back byte for byte.
            for _ in 0..2 {
                assert!(
                    undo(&mut app).is_ok() && undo(&mut app).is_ok(),
                    "{source:?}"
                );
                assert_eq!(app.editor.text(), on_disk(&directory, "B"), "{source:?}");
            }
            assert_eq!(app.editor.text(), source);
            assert_eq!(on_disk(&directory, "B"), source);
            assert!(!app.can_undo_task_write());
        }
    }

    #[test]
    fn front_matter_changed_since_refuses_the_undo_and_obsidians_key_refuses_showing() {
        let obsidian = "---\nTP_ignore_this_file: true\n---\n- [ ] a\n";
        let (directory, mut app) = fixture_with(ON, obsidian, B);
        let (a, b) = (note(&app, "A"), note(&app, "B"));
        let refused = app.set_note_tasks_hidden(a, false).result.unwrap_err();
        assert!(refused.message.contains("TP_ignore_this_file") && !refused.refresh);
        assert_eq!(app.editor.text(), obsidian);
        assert!(!app.editor.can_undo());

        assert!(app.set_note_tasks_hidden(b, true).result.unwrap().changed);
        let theirs = format!("---\ntasks: off\ntitle: B\n---\n{B}");
        std::fs::write(file(&directory, "B"), &theirs).unwrap();
        let refused = undo(&mut app).unwrap_err();
        assert_eq!(refused, Refusal::stale(task_write::SWITCH_CHANGED));
        assert_eq!(on_disk(&directory, "B"), theirs);
        assert!(!app.can_undo_task_write());
    }

    #[test]
    fn a_write_runs_with_no_view_and_over_a_board_and_never_with_tasks_off() {
        // With the note showing in its editor (the Notes column's "Hide
        // tasks in this note"), the change is the editor's all the same.
        let (directory, mut app) = fixture();
        assert_eq!(app.surface(), Surface::Text);
        let wrote = toggle_row(&mut app, "A", 2).unwrap();
        assert!(wrote.open);
        assert_eq!(app.editor.text(), A.replace(MAIL, MAILED));
        editor_undo(&mut app);
        assert_eq!(app.editor.text(), A);

        // A board is the open document: another note is written by the
        // Session, and the board is no note of tasks.
        open(&mut app, "Flow");
        assert_eq!(app.surface(), Surface::Board);
        let wrote = toggle_row(&mut app, "B", 2).unwrap();
        assert!(wrote.changed && !wrote.open);
        assert!(on_disk(&directory, "B").contains("[x] Order lunch"));
        let board = RowTarget {
            note: note(&app, "Flow"),
            path: RelPath::new("Work/Tasks/Flow.wren-board").unwrap(),
            line: 0,
            expect: "- [ ] a".into(),
        };
        let refused = toggle(&mut app, &board).unwrap_err();
        assert!(
            !refused.refresh && refused.message.contains("Markdown"),
            "{refused:?}"
        );
        let flow = (directory.path()).join("workspace/Work/Tasks/Flow.wren-board");
        assert_eq!(std::fs::read_to_string(flow).unwrap(), FLOW);

        // Tasks off: nothing is reachable.
        let (directory, mut app) = fixture_with("tasks = off\nboards = on\n", A, B);
        let target = RowTarget {
            note: note(&app, "A"),
            path: RelPath::new("Work/Tasks/A.md").unwrap(),
            line: 2,
            expect: "- [ ] Mail the deck 📅 2026-12-01".into(),
        };
        let other = RowTarget {
            note: note(&app, "B"),
            ..target.clone()
        };
        for target in [target, other] {
            let refused = toggle(&mut app, &target).unwrap_err();
            assert_eq!(refused, Refusal::blocked("Tasks are off"));
            let hide = app.set_note_tasks_hidden(target.note, true);
            assert_eq!(hide.result.unwrap_err().message, "Tasks are off");
        }
        assert_eq!(app.editor.text(), A);
        assert!(!app.editor.can_undo() && !app.can_undo_task_write());
        assert_eq!(
            (on_disk(&directory, "A"), on_disk(&directory, "B")),
            (A.into(), B.into())
        );
        assert!(!directory.path().join("workspace/.wren/tasks.toml").exists());
    }

    #[test]
    fn a_write_whose_durability_is_unconfirmed_is_reported_and_can_be_undone() {
        let (directory, mut app) = fixture();
        show(&mut app);
        app.session
            .as_mut()
            .unwrap()
            .fail_next(Fault::DirectorySync);
        let target = row(&app, "B", 2);
        let done = app.write_task_row(&target, &change(Change::Cycle));
        // The bytes are published: it is a write, with what went wrong
        // after it said beside it and in the banner.
        assert_eq!(
            done.gateway.processed.disposition,
            Disposition::CommittedWarning
        );
        let wrote = done.result.unwrap();
        assert!(wrote.changed && !wrote.open);
        let warning = wrote.warning.expect("the store's outcome is reported");
        assert!(warning.contains("durability"), "{warning}");
        assert_eq!(app.banner.as_deref(), Some(&*warning));
        assert_eq!(on_disk(&directory, "B"), B.replace("[ ]", "[/]"));
        assert_eq!(listed(&app, "B"), ["2 - [/] Order lunch"]);
        assert!(app.can_undo_task_write());
        assert!(matches!(undo(&mut app), Ok(Undone::Line(write)) if write.warning.is_none()));
        assert_eq!(on_disk(&directory, "B"), B);
    }

    #[test]
    fn another_workspace_forgets_what_could_be_undone() {
        let (_directory, mut app) = fixture();
        toggle_row(&mut app, "B", 2).unwrap();
        assert!(app.can_undo_task_write());
        app.hydrate_navigation();
        assert!(!app.can_undo_task_write());
    }

    #[cfg(feature = "smoke-test")]
    #[test]
    fn the_harness_writes_hides_and_undoes_through_the_same_functions() {
        use serde_json::{json, Value};

        use crate::test_ipc::Request;

        fn ask(app: &mut App, params: Value) -> Result<Value, String> {
            let (request, mut reply) = Request::detached("tasks.view", params);
            let _ = app.handle_test(request);
            reply.try_recv().expect("the request is answered at once")
        }

        let (directory, mut app) = fixture();
        let (a, b) = ("Work/Tasks/A.md", "Work/Tasks/B.md");
        show(&mut app);
        // The open note, by the line the index has.
        let answer = ask(
            &mut app,
            json!({"write": {"path": a, "line": 2, "change": "toggle"}}),
        );
        let answer = answer.unwrap();
        assert_eq!(answer["write"]["after"], MAILED);
        assert_eq!(
            (
                &answer["write"]["open"],
                &answer["write"]["changed"],
                &answer["refused"]
            ),
            (&json!(true), &json!(true), &Value::Null)
        );
        assert_eq!(
            answer["note"]["tasks"][0],
            json!({"line": 2, "source": MAILED})
        );
        assert_eq!(
            (&answer["can_undo"], &answer["open"]),
            (&json!(true), &json!(true))
        );
        assert_eq!(answer["disposition"], "completed");
        // A closed note, with each kind of change.
        for (change, after) in [
            (json!("cycle"), "- [/] Order lunch"),
            (json!({"priority": "high"}), "- [/] Order lunch ⏫"),
            (json!("lower"), "- [/] Order lunch 🔼"),
            (json!({"priority": null}), "- [/] Order lunch"),
            (
                json!({"due": "2026-12-04"}),
                "- [/] Order lunch 📅 2026-12-04",
            ),
            (json!({"due": null}), "- [/] Order lunch"),
            (json!({"add_owner": "sam"}), "- [/] Order lunch @sam"),
            (json!({"remove_owner": "sam"}), "- [/] Order lunch"),
            (json!({"description": "Order dinner"}), "- [/] Order dinner"),
            (json!({"status": "-"}), "- [-] Order dinner ❌ 2026-11-19"),
            (json!({"line": "- [ ] Order lunch"}), LUNCH),
        ] {
            let write = json!({"write": {"path": b, "line": 2, "change": change}});
            let answer = ask(&mut app, write).unwrap();
            assert_eq!(answer["write"]["after"], after, "{change}");
            assert_eq!(answer["write"]["open"], false);
        }
        assert_eq!(on_disk(&directory, "B"), B);
        // A row out of date, by the line it holds.
        let held = json!({"path": b, "line": 2, "expect": "- [ ] Order", "change": "toggle"});
        let answer = ask(&mut app, json!({"write": held})).unwrap();
        assert_eq!(
            answer["refused"],
            json!({"message": task_write::CHANGED, "refresh": true})
        );
        assert_eq!(
            (&answer["write"], &answer["disposition"]),
            (&Value::Null, &json!("blocked"))
        );
        // Hide, and undo.
        let answer = ask(&mut app, json!({"hide": {"path": b, "hidden": true}})).unwrap();
        assert_eq!(answer["switch"]["after"], "---\ntasks: off\n---\n");
        assert_eq!(
            answer["note"],
            json!({"path": b, "hidden": true, "tasks": []})
        );
        let answer = ask(&mut app, json!({"undo": true})).unwrap();
        assert_eq!(answer["switch"]["after"], "");
        assert_eq!(answer["note"]["hidden"], false);
        assert_eq!(on_disk(&directory, "B"), B);
        // What the request does not understand is an error.
        for bad in [
            json!({"write": {"path": b, "line": 2, "change": "frobnicate"}}),
            json!({"write": {"path": b, "line": 9, "change": "toggle"}}),
            json!({"write": {"path": "Work/Tasks/None.md", "line": 2, "change": "toggle"}}),
            json!({"hide": {"path": b}}),
        ] {
            assert!(ask(&mut app, bad.clone()).is_err(), "{bad}");
        }

        // A line that would be no task is not written, and leaves no
        // undo of its own.
        let depth = app.task_writes.undo.len();
        let plain = json!({"write": {"path": b, "line": 2, "change": {"line": "plain text"}}});
        let answer = ask(&mut app, plain).unwrap();
        assert_eq!(answer["refused"]["refresh"], false);
        assert_eq!(answer["write"], Value::Null);
        assert_eq!(app.task_writes.undo.len(), depth);
        assert_eq!(on_disk(&directory, "B"), B);
        // A refused write says what the index holds of its note now.
        let theirs = B.replace(LUNCH, "- [ ] Order dinner");
        std::fs::write(file(&directory, "B"), &theirs).unwrap();
        let held = json!({"path": b, "line": 2, "expect": LUNCH, "change": "toggle"});
        let answer = ask(&mut app, json!({"write": held})).unwrap();
        assert_eq!(answer["refused"]["refresh"], true);
        assert_eq!(
            answer["note"],
            json!({"path": b, "hidden": false,
                   "tasks": [{"line": 2, "source": "- [ ] Order dinner"}]})
        );

        // With tasks off each answers that they are off, as the plain
        // question does, whatever it asks for, and does nothing.
        let (directory, mut app) = fixture_with("tasks = off\nboards = on\n", A, B);
        for request in [
            json!({"write": {"path": a, "line": 2, "change": "toggle"}}),
            json!({"write": {"path": "no/such.md", "change": "frobnicate"}}),
            json!({"hide": {"path": b, "hidden": true}}),
            json!({"undo": true}),
            json!({}),
        ] {
            assert_eq!(ask(&mut app, request).unwrap(), json!({"tasks": false}));
        }
        assert_eq!(
            (on_disk(&directory, "A"), on_disk(&directory, "B")),
            (A.into(), B.into())
        );
    }

    /// Where a row's due cell is, for the calendar over a row.
    fn cell() -> iced::Rectangle {
        iced::Rectangle::new(iced::Point::new(320.0, 180.0), iced::Size::new(90.0, 22.0))
    }

    #[test]
    fn the_calendar_over_a_row_writes_through_the_rows_path() {
        use super::super::calendar::{place, Event};

        let (directory, mut app) = fixture();
        // Only over the task view.
        assert!(app.open_row_calendar(row(&app, "B", 2), cell()).is_none());
        show(&mut app);

        // A note that is not open, with no due date: today, hung under the
        // cell; a pick writes it to the file.
        assert!(app.open_row_calendar(row(&app, "B", 2), cell()).is_some());
        let open = app.calendar.as_ref().expect("open");
        assert_eq!((open.line(), open.has_due()), (2, false));
        assert_eq!(open.model.selected(), Date::new(2026, 11, 19).unwrap());
        let bounds = app.calendar_bounds().expect("placed");
        assert_eq!(bounds.position(), place(cell(), app.viewport));
        let monday = Date::new(2026, 11, 23).unwrap();
        let _ = app.update(Message::Calendar(Event::Pick(monday)));
        assert!(app.calendar.is_none());
        assert_eq!(app.surface(), Surface::Tasks);
        assert_eq!(
            on_disk(&directory, "B"),
            B.replace(LUNCH, "- [ ] Order lunch 📅 2026-11-23")
        );

        // The open note: on its due date, cleared through its editor, and
        // undone through the same path.
        assert!(app.open_row_calendar(row(&app, "A", 2), cell()).is_some());
        let open = app.calendar.as_ref().expect("open");
        assert!(open.has_due());
        assert_eq!(open.model.current(), Some(Date::new(2026, 12, 1).unwrap()));
        let _ = app.update(Message::Calendar(Event::Clear));
        assert_eq!(draft(&app), A.replace(MAIL, "- [ ] Mail the deck"));
        assert!(undo(&mut app).is_ok());
        assert_eq!(draft(&app), A);

        // Escape writes nothing.
        assert!(app.open_row_calendar(row(&app, "A", 2), cell()).is_some());
        let _ = app.update(Message::Calendar(Event::Field(
            crate::date_field::FieldEvent::Escape,
        )));
        assert!(app.calendar.is_none());
        assert_eq!(draft(&app), A);

        // A row whose task changed since it was listed is refused.
        let target = row(&app, "B", 2);
        let theirs = B.replace(LUNCH, "- [ ] Order dinner");
        std::fs::write(file(&directory, "B"), &theirs).unwrap();
        assert!(app.open_row_calendar(target, cell()).is_some());
        let _ = app.update(Message::Calendar(Event::Pick(monday)));
        assert!(app.calendar.is_none());
        assert_eq!(on_disk(&directory, "B"), theirs);
        assert_eq!(app.banner.as_deref(), Some(task_write::CHANGED));

        // The palette taking the keyboard closes it, writing nothing.
        assert!(app.open_row_calendar(row(&app, "A", 2), cell()).is_some());
        let _ = app.update(Message::Command(Command::Run(CommandId::CommandPalette)));
        assert!(app.calendar.is_none());
        assert_eq!(draft(&app), A);
    }

    #[test]
    fn the_form_over_a_row_saves_through_the_rows_path() {
        use super::super::task_form::Event;

        const NESTED: &str = "# B\n\n- [ ] Order lunch\n    - [ ] Call the caterer\n";
        let (directory, mut app) = fixture_with(ON, A, NESTED);
        // Only over the task view.
        assert!(app.open_row_task_form(row(&app, "B", 3)).is_none());
        show(&mut app);

        // A nested task of a note that is not open: read from its line,
        // written in place.
        assert!(app.open_row_task_form(row(&app, "B", 3)).is_some());
        let open = app.task_form.as_ref().expect("open");
        assert_eq!(open.form.description(), "Call the caterer");
        let _ = app.update(Message::TaskForm(Event::Description(
            "Call the florist".into(),
        )));
        let _ = app.update(Message::TaskForm(Event::Save));
        assert!(app.task_form.is_none());
        assert_eq!(app.surface(), Surface::Tasks);
        assert_eq!(
            on_disk(&directory, "B"),
            NESTED.replace("caterer", "florist")
        );

        // The open note, through its editor.
        assert!(app.open_row_task_form(row(&app, "A", 3)).is_some());
        let _ = app.update(Message::TaskForm(Event::Priority(Some(Priority::High))));
        let _ = app.update(Message::TaskForm(Event::Save));
        assert!(app.task_form.is_none());
        assert_eq!(
            draft(&app),
            A.replace(
                "Book the room",
                &format!("Book the room {}", Priority::High.glyph())
            )
        );

        // A task changed since its row was listed refuses Save, and the
        // form stays open with why.
        let target = row(&app, "B", 2);
        let theirs = on_disk(&directory, "B").replace("Order lunch", "Order dinner");
        std::fs::write(file(&directory, "B"), &theirs).unwrap();
        assert!(app.open_row_task_form(target).is_some());
        let _ = app.update(Message::TaskForm(Event::Description("Order tea".into())));
        let _ = app.update(Message::TaskForm(Event::Save));
        let open = app.task_form.as_ref().expect("still open");
        assert_eq!(open.error.as_deref(), Some(task_write::CHANGED));
        assert_eq!(on_disk(&directory, "B"), theirs);
        let _ = app.update(Message::TaskForm(Event::Cancel));
        assert!(app.task_form.is_none());
    }
}
