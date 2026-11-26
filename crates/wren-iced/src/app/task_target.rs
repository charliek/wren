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
use wren_core::model::NoteId;
use wren_core::ops::Op;
use wren_core::task_write::{self, LineChange, Refused, SwitchChange};
use wren_core::tasks::TaskRecord;
use wren_markdown::task::{self, Change};
use wren_markdown::Pos;

use super::commands::{Disposition, Gateway};
use super::App;

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
    Stale,
}

/// The app's task writes: what can be undone, newest last, and what the
/// write in progress has heard from the Session.
#[derive(Debug, Default)]
pub(super) struct Writes {
    undo: Vec<Inverse>,
    pub(super) seen: Option<Seen>,
}

impl Writes {
    fn record(&mut self, inverse: Inverse) {
        if self.undo.len() == MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(inverse);
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
    fn task_write_note(&mut self, note: NoteId, stale: &str) -> Result<(RelPath, bool), Refusal> {
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
        let path = entry.path.clone();
        let open = session.document().map(|document| document.note) == Some(note);
        if open {
            // The editor is the open note's writer only while it holds it.
            self.editor_holds_note().map_err(Refusal::blocked)?;
            if self.editor_note != Some(note) {
                return Err(Refusal::blocked("The editor does not hold the open note"));
            }
        }
        // A done or cancelled stamp is today's, though the day changed
        // since the timer last looked.
        self.check_day();
        Ok((path, open))
    }

    fn apply_row(&mut self, target: &RowTarget, change: &RowChange) -> TaskWrite<RowWrite> {
        let (path, open) = match self.task_write_note(target.note, task_write::CHANGED) {
            Ok(found) => found,
            Err(refusal) => return self.task_write_refused(refusal),
        };
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
        let (path, open) = match self.task_write_note(note, task_write::SWITCH_CHANGED) {
            Ok(found) => found,
            Err(refusal) => return self.task_write_refused(refusal),
        };
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
            Some(Seen::Line { .. }) | None => match closed_error(&gateway) {
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
        let applied = self.editor.apply(&[plan.replace(&source).into()], landing);
        self.editor
            .restore_viewport(view.with_cursor(self.editor.cursor()));
        if !applied {
            return Err(Refusal::blocked("The editor could not make that change"));
        }
        self.text_changed(started);
        // As the editor wrote it: a line break takes the ending of the
        // line it splits.
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
            if !self.preferences.tasks {
                return Err("Tasks are off".into());
            }
            let (mut row, mut switch) = (None, None);
            let (result, gateway) = if let Some(write) = write {
                let target = self.row_target(write)?;
                let change = row_change(write.get("change").ok_or("change is missing")?)?;
                let done = self.write_task_row(&target, &change);
                (done.result.map(|write| row = Some(write)), done.gateway)
            } else if let Some(hide) = hide {
                let note = self.note_at(string(hide, "path")?)?;
                let hidden = (hide.get("hidden").and_then(Value::as_bool))
                    .ok_or("hidden must be a boolean")?;
                let done = self.set_note_tasks_hidden(note, hidden);
                (done.result.map(|write| switch = Some(write)), done.gateway)
            } else {
                let done = self.undo_task_write();
                let result = done.result.map(|undone| match undone {
                    Undone::Line(write) => row = Some(write),
                    Undone::Switch(write) => switch = Some(write),
                });
                (result, done.gateway)
            };
            let path = (row.as_ref().map(|write| &write.path))
                .or(switch.as_ref().map(|write| &write.path));
            let mut answer = self.task_view_snapshot();
            answer["write"] = row.as_ref().map_or(Value::Null, row_json);
            answer["switch"] = switch.as_ref().map_or(Value::Null, switch_json);
            answer["refused"] = match &result {
                Ok(()) => Value::Null,
                Err(refusal) => json!({"message": refusal.message, "refresh": refusal.refresh}),
            };
            answer["note"] = path.map_or(Value::Null, |path| self.note_tasks_json(path));
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
    /// and `B` with the texts given and the board `Flow`, with `A` open
    /// and today pinned to Thursday 19 November 2026.
    fn fixture_with(settings: &str, a: &str, b: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        for (path, text) in [
            ("Work/Tasks/A.md", a),
            ("Work/Tasks/B.md", b),
            ("Work/Tasks/Flow.wren-board", FLOW),
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
        wren_editor::BackendKind::ALL.into_iter().map(|kind| {
            let engine = match kind {
                wren_editor::BackendKind::Iced => "classic",
                wren_editor::BackendKind::Owned => "advanced",
            };
            let (directory, app) = fixture_with(&format!("editor-engine = {engine}\n{ON}"), A, B);
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

        // The open note, saved and then changed on disk, which the editor
        // reloads.
        toggle_row(&mut app, "A", 2).unwrap();
        assert!(app.process_core(Op::Flush).core_error.is_none());
        let theirs = on_disk(&directory, "A").replace("Mail the deck", "Mail the decks");
        std::fs::write(file(&directory, "A"), &theirs).unwrap();
        assert!(app.process_core(Op::Reconcile).core_error.is_none());
        assert_eq!(app.editor.text(), theirs);
        assert_eq!(undo(&mut app), Err(stale()));
        assert_eq!(app.editor.text(), theirs);
        assert!(!dirty(&app) && !app.can_undo_task_write());
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
            !refused.refresh && refused.message.contains("board"),
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

        // With tasks off each is refused, and the plain question still
        // answers that they are off.
        let (directory, mut app) = fixture_with("tasks = off\nboards = on\n", A, B);
        for request in [
            json!({"write": {"path": a, "line": 2, "change": "toggle"}}),
            json!({"hide": {"path": b, "hidden": true}}),
            json!({"undo": true}),
        ] {
            assert_eq!(ask(&mut app, request).unwrap_err(), "Tasks are off");
        }
        assert_eq!(ask(&mut app, json!({})).unwrap(), json!({"tasks": false}));
        assert_eq!(
            (on_disk(&directory, "A"), on_disk(&directory, "B")),
            (A.into(), B.into())
        );
    }
}
