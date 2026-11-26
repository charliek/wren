//! The task view (discovery/tasks.md §9): the tasks of a scope, shown in
//! the editor pane in place of the open note. It is a view, not a document:
//! the note under it stays the Session's document, with its draft, caret
//! and undo history, and neither the editor nor the board viewer is mounted
//! while it shows, so neither takes input ([`Surface::Tasks`]).
//!
//! Back and Forward pass through views as they pass through notes. The
//! Session's history holds notes only and is what `.wren/workspace.toml`
//! stores; a view's stop lives here, in the [`Trail`], hung after the
//! Session's [`Visit`] it was opened from. So nothing of a view is ever
//! written, a restart reopens the last note, and with tasks off the trail
//! is empty and Back and Forward are the Session's alone.
//!
//! This module holds the surface and its history. The list is item 10.3's,
//! the keys and the writes item 10.4's; [`App::task_view_has_keys`],
//! [`App::task_view_key`] and [`App::task_view_focus`] are where they take
//! the keyboard.

use iced::widget::{column, container, mouse_area, row};
use iced::{Element, Fill, Shrink, Task};
use wren_core::fs_store::RelPath;
use wren_core::model::{ItemRef, Workspace};
use wren_core::ops::{Event as CoreEvent, Op, Session, Visit};

use super::board::Surface;
use super::columns::Column;
use super::commands::{Disposition, Gateway};
use super::{App, Message};
use crate::chrome::{self, Icon};
use crate::ellipsis::ellipsis;
use crate::keys::Nav;
use crate::{fonts, theme};

/// How many stops the trail keeps, as many as the Session keeps visits.
const MAX_STOPS: usize = 50;

/// What a task view lists the tasks of. A tree item is named by its path,
/// which a rescan keeps; its id is the running model's alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    // Until the entry points (item 10.2) only the harness opens a view.
    #[cfg_attr(not(any(test, feature = "smoke-test")), allow(dead_code))]
    Workspace,
    Section(RelPath),
    Group(RelPath),
    Note(RelPath),
}

impl Scope {
    /// The scope's kind, as the harness and the header name it.
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Section(_) => "section",
            Self::Group(_) => "group",
            Self::Note(_) => "note",
        }
    }

    /// The path every note in the scope lies under, a note's own for a
    /// note: what the task index is asked for. `None` is the whole
    /// workspace.
    pub(super) const fn prefix(&self) -> Option<&RelPath> {
        match self {
            Self::Workspace => None,
            Self::Section(path) | Self::Group(path) | Self::Note(path) => Some(path),
        }
    }

    /// The scope of a tree item, as the model has it now.
    pub(super) fn of(item: ItemRef, model: &Workspace) -> Option<Self> {
        let path = model.path(item)?.clone();
        Some(match item {
            ItemRef::Section(_) => Self::Section(path),
            ItemRef::Group(_) => Self::Group(path),
            ItemRef::Note(_) => Self::Note(path),
        })
    }

    /// The scope of the `kind` at `path`, as a request names one.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn parse(kind: &str, path: Option<&str>) -> Result<Self, String> {
        let at = || {
            let path = path.ok_or_else(|| format!("a {kind} scope needs a path"))?;
            RelPath::new(path).map_err(|error| error.to_string())
        };
        match kind {
            "workspace" => Ok(Self::Workspace),
            "section" => at().map(Self::Section),
            "group" => at().map(Self::Group),
            "note" => at().map(Self::Note),
            _ => Err("scope must be workspace, section, group or note".into()),
        }
    }

    /// The tree item the scope names, if the model still has it. The
    /// workspace is no item.
    fn item(&self, model: &Workspace) -> Option<ItemRef> {
        match self {
            Self::Workspace => None,
            Self::Section(path) => model
                .sections
                .iter()
                .find(|section| section.path == *path)
                .map(|section| ItemRef::Section(section.id)),
            Self::Group(path) => model
                .groups
                .iter()
                .find(|group| group.path == *path)
                .map(|group| ItemRef::Group(group.id)),
            Self::Note(path) => model
                .notes
                .iter()
                .find(|note| note.path == *path)
                .map(|note| ItemRef::Note(note.id)),
        }
    }

    /// Whether the model still has what the scope names.
    fn exists(&self, model: &Workspace) -> bool {
        matches!(self, Self::Workspace) || self.item(model).is_some()
    }

    /// The name of the item the scope names, as the model has it; `None`
    /// for the workspace, which the model does not name.
    fn name<'a>(&self, model: &'a Workspace) -> Option<&'a str> {
        match self.item(model)? {
            ItemRef::Section(id) => model.section(id).map(|section| section.name.as_str()),
            ItemRef::Group(id) => model.group(id).map(|group| group.name.as_str()),
            ItemRef::Note(id) => model.note(id).map(|note| note.title.as_str()),
        }
    }
}

/// Names one stop of the trail for as long as it is kept, whatever stops
/// come and go around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StopId(u64);

/// A view in Back and Forward history.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stop {
    id: StopId,
    /// The serial of the Session's visit this stop comes after; `None` is
    /// before the first visit.
    after: Option<u64>,
    scope: Scope,
}

/// What Back or Forward does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// There is nothing further that way.
    End,
    /// Show this stop. No note changes.
    Show(StopId),
    /// Leave the view for the note under it, which is the next stop.
    Reveal,
    /// The Session steps to its next note; then this stop shows over it.
    Session(Option<StopId>),
}

/// What the trail reads before a Session op, to follow what the op did to
/// the Session's history.
pub(super) struct Mark {
    /// The serial of the visit the Session was on.
    here: Option<u64>,
    /// The serial of the newest visit.
    newest: Option<u64>,
    /// Each stop's tree item, for an op that moves paths.
    items: Option<Vec<Option<ItemRef>>>,
}

/// The task views in Back and Forward history, and the one in the pane.
///
/// The stops are kept in history's order: sorted by the visit they come
/// after, and within one gap in the order they were opened. While a stop
/// is shown the Session is on the visit the stop comes after. A stop
/// before the first visit shows over the first visit's note; with no visit
/// at all the stops come after the pane with no note in it.
#[derive(Debug, Default)]
pub(super) struct Trail {
    stops: Vec<Stop>,
    shown: Option<usize>,
    serial: u64,
}

fn serial_at(visits: &[Visit], position: Option<usize>) -> Option<u64> {
    position
        .and_then(|position| visits.get(position))
        .map(|visit| visit.serial)
}

impl Trail {
    /// The scope of the view in the pane.
    pub(super) fn shown(&self) -> Option<&Scope> {
        self.shown.map(|index| &self.stops[index].scope)
    }

    /// Open a view of `scope` from where history is, as a visit of a note
    /// is recorded: the views ahead in this gap are dropped, the notes
    /// ahead are the Session's and stay. The view already shown is not
    /// opened twice.
    pub(super) fn open(&mut self, scope: Scope, visits: &[Visit], position: Option<usize>) {
        let (after, at) = match self.shown {
            Some(index) => {
                if self.stops[index].scope == scope {
                    return;
                }
                (self.stops[index].after, index + 1)
            }
            None => {
                let here = serial_at(visits, position);
                (here, self.stops.partition_point(|stop| stop.after < here))
            }
        };
        let ahead = self.stops[at..]
            .iter()
            .take_while(|stop| stop.after == after)
            .count();
        self.stops.drain(at..at + ahead);
        self.serial += 1;
        let id = StopId(self.serial);
        self.stops.insert(at, Stop { id, after, scope });
        self.shown = Some(at);
        if self.stops.len() > MAX_STOPS {
            // The oldest goes, or the farthest ahead when the new one is
            // the oldest.
            if at == 0 {
                self.stops.pop();
            } else {
                self.stops.remove(0);
                self.shown = Some(at - 1);
            }
        }
    }

    /// Show the stop `id`, if it is still kept.
    pub(super) fn show(&mut self, id: StopId) -> bool {
        let index = self.stops.iter().position(|stop| stop.id == id);
        if index.is_some() {
            self.shown = index;
        }
        index.is_some()
    }

    fn hide(&mut self) {
        self.shown = None;
    }

    /// What Back (`back`) or Forward does from here.
    pub(super) fn step(&self, back: bool, visits: &[Visit], position: Option<usize>) -> Step {
        let same_gap =
            |index: usize, other: usize| self.stops[index].after == self.stops[other].after;
        match (self.shown, back) {
            (Some(index), true) => {
                if index > 0 && same_gap(index, index - 1) {
                    Step::Show(self.stops[index - 1].id)
                } else if self.stops[index].after.is_some() || position.is_none() {
                    // The note it was opened over, or the empty pane it
                    // was opened in when there is no visit at all.
                    Step::Reveal
                } else {
                    Step::End
                }
            }
            (Some(index), false) => {
                if index + 1 < self.stops.len() && same_gap(index, index + 1) {
                    return Step::Show(self.stops[index + 1].id);
                }
                match (self.stops[index].after, position) {
                    (Some(_), Some(position)) if position + 1 < visits.len() => Step::Session(None),
                    // Before the first visit, whose note is under the view.
                    (None, Some(_)) => Step::Reveal,
                    _ => Step::End,
                }
            }
            (None, true) => {
                let Some(position) = position else {
                    return Step::End;
                };
                let gap = position
                    .checked_sub(1)
                    .and_then(|previous| serial_at(visits, Some(previous)));
                let last = self.stops.iter().rev().find(|stop| stop.after == gap);
                match (last, position) {
                    (Some(stop), 0) => Step::Show(stop.id),
                    (Some(stop), _) => Step::Session(Some(stop.id)),
                    (None, 0) => Step::End,
                    (None, _) => Step::Session(None),
                }
            }
            (None, false) => {
                let here = serial_at(visits, position);
                // On a visit, the stops before the first visit are behind.
                let first = self.stops.iter().find(|stop| stop.after == here);
                match first {
                    Some(stop) => Step::Show(stop.id),
                    None if position.is_some_and(|position| position + 1 < visits.len()) => {
                        Step::Session(None)
                    }
                    None => Step::End,
                }
            }
        }
    }

    /// Read what [`Self::follow`] compares with after a Session op, before
    /// the op runs. `None` with no stops, when there is nothing to keep in
    /// step.
    pub(super) fn mark(&self, session: &Session, op: &Op) -> Option<Mark> {
        if self.stops.is_empty() {
            return None;
        }
        let visits = session.visits();
        let moves = matches!(
            op,
            Op::Rename { .. } | Op::MoveNote { .. } | Op::MoveGroup { .. }
        );
        Some(Mark {
            here: serial_at(visits, session.visit_position()),
            newest: visits.last().map(|visit| visit.serial),
            items: moves.then(|| {
                self.stops
                    .iter()
                    .map(|stop| stop.scope.item(session.model()))
                    .collect()
            }),
        })
    }

    /// Bring the trail in step with the Session after `op`: its history
    /// ([`Self::follow_visits`]) and its tree ([`Self::follow_tree`]). A
    /// note or a parent asked for takes the pane from a view; a change in
    /// the background does not. Returns whether the view left the pane.
    pub(super) fn follow(
        &mut self,
        mark: Option<&Mark>,
        session: &Session,
        op: &Op,
        events: &[CoreEvent],
        committed: bool,
    ) -> bool {
        let Some(mark) = mark else {
            return false;
        };
        let navigated = committed
            && matches!(
                op,
                Op::OpenNote { .. }
                    | Op::SelectParent { .. }
                    | Op::SelectSection { .. }
                    | Op::Back
                    | Op::Forward
            );
        let was_shown = self.shown.is_some();
        self.follow_visits(mark.here, mark.newest, session.visits(), navigated);
        let tree_changed = events
            .iter()
            .any(|event| matches!(event, CoreEvent::TreeChanged));
        if mark.items.is_some() || tree_changed {
            self.follow_tree(mark.items.as_deref(), session.model());
        }
        was_shown && self.shown.is_none()
    }

    /// Bring the trail in step with the Session's history after an op. A
    /// new visit drops the stops ahead, as it drops the visits ahead, and
    /// takes the view out of the pane; so does a note or a parent asked for
    /// (`navigated`), whether or not it made a visit. A stop whose visit is
    /// gone (its note deleted, or history too long) comes after the visit
    /// before that one. `here` and `newest` are the serials of the visit
    /// the Session was on and of its newest, before the op.
    fn follow_visits(
        &mut self,
        here: Option<u64>,
        newest: Option<u64>,
        visits: &[Visit],
        navigated: bool,
    ) {
        let visited = visits.last().map(|visit| visit.serial) > newest;
        if visited {
            let ahead = match self.shown {
                Some(index) => index + 1,
                None => self.stops.partition_point(|stop| stop.after < here),
            };
            self.stops.truncate(ahead);
            // A view shown from before the first visit was over that
            // visit's note, which the new visit comes after: the view, and
            // the views behind it, come between the two, so Back from the
            // new note is the view it was opened from.
            let before_first = self
                .shown
                .is_some_and(|index| self.stops[index].after.is_none());
            if before_first && here.is_some() {
                for stop in &mut self.stops {
                    stop.after = here;
                }
            }
        }
        if visited || navigated {
            self.shown = None;
        }
        for stop in &mut self.stops {
            let Some(serial) = stop.after else {
                continue;
            };
            let next = visits.partition_point(|visit| visit.serial < serial);
            if visits.get(next).is_none_or(|visit| visit.serial != serial) {
                stop.after = next.checked_sub(1).map(|previous| visits[previous].serial);
            }
        }
    }

    /// Bring the trail in step with the tree: a stop whose item was renamed
    /// or moved (`items`, read before the op) follows it, and one whose
    /// item is gone is dropped, as a deleted note's visits are.
    fn follow_tree(&mut self, items: Option<&[Option<ItemRef>]>, model: &Workspace) {
        for (stop, item) in self.stops.iter_mut().zip(items.unwrap_or_default()) {
            if let Some(scope) = item.and_then(|item| Scope::of(item, model)) {
                stop.scope = scope;
            }
        }
        let shown = self.shown.map(|index| self.stops[index].id);
        self.stops.retain(|stop| stop.scope.exists(model));
        self.shown = shown.and_then(|id| self.stops.iter().position(|stop| stop.id == id));
    }
}

/// Input from the task view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A press inside the view, which takes the keyboard as a press in the
    /// editor does.
    Press,
}

impl App {
    /// The scope of the task view in the pane, when it is what shows.
    /// While a workspace loads the pane shows that instead.
    pub(super) fn task_view(&self) -> Option<&Scope> {
        self.task_views.shown().filter(|_| self.loading.is_none())
    }

    /// Forget every view: another workspace is open.
    pub(super) fn clear_task_views(&mut self) {
        self.task_views = Trail::default();
    }

    /// What Back or Forward does next, through notes and views alike.
    fn history_next(&self, back: bool) -> Step {
        let Some(session) = &self.session else {
            return Step::End;
        };
        self.task_views
            .step(back, session.visits(), session.visit_position())
    }

    /// Whether Back (`back`) or Forward has somewhere to go.
    pub(super) fn can_step(&self, back: bool) -> bool {
        self.history_next(back) != Step::End
    }

    /// Back or Forward, the one way either runs (`run_command`).
    pub(super) fn history_step(&mut self, back: bool) -> Gateway {
        let op = if back { Op::Back } else { Op::Forward };
        match self.history_next(back) {
            // The Session says why there is nothing further.
            Step::End | Step::Session(None) => self.process_core(op),
            // The op puts the view in the pane itself, once the note under
            // it is open, so the keyboard is given once, to the view.
            Step::Session(Some(stop)) => {
                let outcome = self.process_core_showing(op, Some(stop));
                if outcome.processed.disposition.committed() && self.task_view().is_some() {
                    self.cover_note();
                }
                outcome
            }
            Step::Show(stop) => {
                if let Err(blocked) = self.leave_note() {
                    return blocked;
                }
                self.put_task_view(stop);
                let task = self.column_focus();
                self.finish_action(Disposition::Completed, None, task)
            }
            Step::Reveal => {
                // The note of the visit the view comes after is the stop
                // before it. It is the open document, unless it was closed
                // under the view (the document deleted outside Wren, with
                // an earlier visit left): then that visit's note is opened
                // again, which records no new visit and takes the pane.
                if let Some(note) = self.closed_visit() {
                    return self.process_core(Op::OpenNote { note: Some(note) });
                }
                self.task_views.hide();
                let task = self.column_focus();
                self.finish_action(Disposition::Completed, None, task)
            }
        }
    }

    /// The note of the visit the Session is on, when it is not the open
    /// document.
    fn closed_visit(&self) -> Option<wren_core::model::NoteId> {
        let session = self.session.as_ref()?;
        let visit = session.visits().get(session.visit_position()?)?;
        let open = session.document().map(|document| document.note);
        (open != Some(visit.note)).then_some(visit.note)
    }

    /// Open the task view of `scope` in the pane, over the open note. Every
    /// way of opening one comes here through `dispatch`
    /// (`Command::ShowTasks`).
    pub(super) fn show_tasks(&mut self, scope: Scope) -> Gateway {
        if !self.preferences.tasks {
            return self.blocked("Tasks are off");
        }
        let Some(session) = &self.session else {
            return self.blocked("No workspace is open");
        };
        if !scope.exists(session.model()) {
            return self.blocked("The item no longer exists");
        }
        // Each is over the note's text, and would be left over the view.
        if self.calendar.is_some() || self.task_form.is_some() {
            return self.blocked("Close the task form or the calendar first");
        }
        // The view already shown is no transition. Any other is, from a
        // note or from another view.
        if self.task_view() != Some(&scope) {
            if let Err(blocked) = self.leave_note() {
                return blocked;
            }
        }
        if let Some(session) = &self.session {
            self.task_views
                .open(scope, session.visits(), session.visit_position());
        }
        self.cover_note();
        self.palette = None;
        self.column = Column::Editor;
        let task = self.column_focus();
        self.finish_action(Disposition::Completed, None, task)
    }

    /// Before a view takes the pane, from the open note or from another
    /// view, as before any note transition: the board settles and puts its
    /// tool away, and the draft is flushed. A flush that fails blocks the
    /// view, and keeps the draft and what the pane shows.
    fn leave_note(&mut self) -> Result<(), Gateway> {
        if let Err(error) = self.put_board_away() {
            return Err(self.blocked(error));
        }
        self.settle_resize();
        self.cancel_gestures();
        self.editor.break_group();
        if self.session.is_none() {
            return Ok(());
        }
        let flushed = self.process_core(Op::Flush);
        let failed = flushed.processed.disposition == Disposition::Blocked;
        match (&flushed.core_error, failed) {
            (None, false) => Ok(()),
            (_, true) => Err(flushed),
            (Some(error), false) => {
                let error = error.clone();
                Err(self.blocked(error))
            }
        }
    }

    /// Put the stop `id` in the pane. Returns whether it is still kept.
    fn put_task_view(&mut self, id: StopId) -> bool {
        let shown = self.task_views.show(id);
        if shown {
            self.cover_note();
        }
        shown
    }

    /// What is open over the note's text goes when a view covers it.
    fn cover_note(&mut self) {
        self.editor.close_popover();
        self.find_open = false;
    }

    /// Whether the task view has the keyboard: it is in the pane, the pane
    /// is the active column, and nothing that takes the keyboard is open
    /// over it. The app's key handling asks this before it gives a key to
    /// anything of the note's.
    pub(super) fn task_view_has_keys(&self) -> bool {
        self.surface() == Surface::Tasks
            && self.column == Column::Editor
            && self.palette.is_none()
            && self.dialog.is_none()
            && self.calendar.is_none()
            && self.task_form.is_none()
    }

    /// Give the keyboard to the task view. Until the list is a widget that
    /// takes keys (item 10.3), that is taking it from every widget, so
    /// keys reach the app's own handling and [`App::task_view_key`].
    pub(super) fn task_view_focus(&self) -> Task<Message> {
        iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
    }

    /// A navigation key while the task view has the keyboard. The list's
    /// keys are read here (items 10.3 and 10.4); until then none does
    /// anything.
    pub(super) fn task_view_key(&mut self, _key: Nav) -> Task<Message> {
        Task::none()
    }

    pub(super) fn task_view_event(&mut self, event: Event) -> Task<Message> {
        if self.task_view().is_none() || self.dialog.is_some() || self.palette.is_some() {
            return Task::none();
        }
        match event {
            Event::Press => {
                self.column = Column::Editor;
                self.column_focus()
            }
        }
    }

    /// The name of what a view lists, as its header shows it: the item's,
    /// or the workspace's as the title bar names it.
    pub(super) fn task_view_label(&self, scope: &Scope) -> String {
        self.model()
            .and_then(|model| scope.name(model))
            .or_else(|| self.workspace_name())
            .unwrap_or_default()
            .to_owned()
    }

    /// The pane's header over a task view: Back and Forward, then what
    /// the view lists.
    pub(super) fn task_view_header(&self, scope: &Scope) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let label = self.task_view_label(scope);
        let chip = container(
            row![
                chrome::icon(Icon::Tasks, 14.0, colors.accent),
                chrome::label("Tasks in", 13, colors.body),
                ellipsis(label, 13.0, theme::color(colors.text))
                    .font(fonts::SEMIBOLD)
                    .width(Shrink),
                chrome::label(scope.kind(), 11, colors.faint),
            ]
            .spacing(6)
            .align_y(iced::Center),
        )
        .id("task-view-scope")
        .padding([3, 8])
        .clip(true)
        .style(move |_| chrome::surface(self.definition, colors.raised, 6.0, true));
        row![
            self.history_buttons(),
            chip,
            iced::widget::space::horizontal()
        ]
        .spacing(6)
        .align_y(iced::Center)
        .clip(true)
        .into()
    }

    /// The task view itself. The list is item 10.3's; this says what it
    /// will be of.
    pub(super) fn task_view_body(&self, scope: &Scope) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let label = self.task_view_label(scope);
        let place = scope
            .prefix()
            .map(|path| path.as_path().to_string_lossy().into_owned());
        let mut lines = column![iced::widget::text(format!("Tasks in {label}"))
            .size(18)
            .color(theme::color(colors.muted))]
        .spacing(8)
        .align_x(iced::Center);
        if let Some(place) = place {
            lines = lines.push(chrome::label(place, 12, colors.faint));
        }
        mouse_area(
            container(lines)
                .id("task-view")
                .center_x(Fill)
                .center_y(Fill)
                .style(move |_| chrome::surface(self.definition, colors.editor, 0.0, false)),
        )
        .on_press(Message::TaskView(Event::Press))
        .into()
    }

    /// The pane's footer under a task view.
    pub(super) fn task_view_footer(&self, scope: &Scope) -> Element<'_, Message> {
        let colors = self.definition.colors;
        row![
            chrome::label(format!("Tasks · {}", scope.kind()), 11, colors.muted),
            iced::widget::space::horizontal(),
        ]
        .align_y(iced::Center)
        .clip(true)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Instant;

    use iced::widget::text_editor::{Action, Cursor, Edit, Position};
    use wren_core::fs_store::Fault;
    use wren_core::model::{NoteId, Parent};

    use super::super::board::tests::{modified, tick_to, FLOW};
    use super::super::commands::{Command, CommandId};
    use super::super::palette::Mode;
    use super::super::Options;
    use super::*;
    use crate::keys::Input;

    fn visits(serials: &[u64]) -> Vec<Visit> {
        serials
            .iter()
            .map(|&serial| Visit {
                serial,
                note: NoteId(serial),
            })
            .collect()
    }

    fn group(name: &str) -> Scope {
        Scope::Group(RelPath::new(name).unwrap())
    }

    fn id(trail: &Trail, scope: &Scope) -> StopId {
        trail
            .stops
            .iter()
            .find(|stop| stop.scope == *scope)
            .unwrap_or_else(|| panic!("no stop of {scope:?}"))
            .id
    }

    /// The stops in history's order: the visit each comes after, and its
    /// scope's path.
    fn stops(trail: &Trail) -> Vec<(Option<u64>, String)> {
        trail
            .stops
            .iter()
            .map(|stop| {
                let path = stop.scope.prefix().map_or_else(String::new, |path| {
                    path.as_path().to_string_lossy().into_owned()
                });
                (stop.after, path)
            })
            .collect()
    }

    #[test]
    fn with_no_views_back_and_forward_are_the_sessions_own() {
        let trail = Trail::default();
        assert_eq!(trail.step(true, &[], None), Step::End);
        assert_eq!(trail.step(false, &[], None), Step::End);
        let history = visits(&[3, 5, 8]);
        for position in 0..history.len() {
            let back = trail.step(true, &history, Some(position));
            let forward = trail.step(false, &history, Some(position));
            assert_eq!(back != Step::End, position > 0);
            assert_eq!(forward != Step::End, position + 1 < history.len());
            for step in [back, forward] {
                assert!(matches!(step, Step::End | Step::Session(None)), "{step:?}");
            }
        }
    }

    #[test]
    fn a_view_comes_after_the_visit_it_was_opened_from() {
        let history = visits(&[0, 1, 2]);
        let mut trail = Trail::default();
        trail.open(group("A"), &history, Some(1));
        assert_eq!(trail.shown(), Some(&group("A")));
        assert_eq!(stops(&trail), [(Some(1), "A".to_owned())]);
        let a = id(&trail, &group("A"));

        // Back leaves it for the note under it; the notes ahead are the
        // Session's, and Forward steps to the next of them.
        assert_eq!(trail.step(true, &history, Some(1)), Step::Reveal);
        assert_eq!(trail.step(false, &history, Some(1)), Step::Session(None));

        // From its note, Forward shows it again without a note changing.
        trail.hide();
        assert_eq!(trail.step(false, &history, Some(1)), Step::Show(a));
        assert_eq!(trail.step(true, &history, Some(1)), Step::Session(None));
        // From the note after it, Back steps the Session onto its note and
        // shows it over that.
        assert_eq!(trail.step(true, &history, Some(2)), Step::Session(Some(a)));
        // Nowhere else is it in the way.
        assert_eq!(trail.step(true, &history, Some(1)), Step::Session(None));
        assert_eq!(trail.step(false, &history, Some(0)), Step::Session(None));
        assert_eq!(trail.step(false, &history, Some(2)), Step::End);
    }

    #[test]
    fn views_opened_one_from_another_keep_their_order_in_one_gap() {
        let history = visits(&[0, 1]);
        let mut trail = Trail::default();
        for name in ["A", "B", "C"] {
            trail.open(group(name), &history, Some(0));
        }
        let [a, b, c] = ["A", "B", "C"].map(|name| id(&trail, &group(name)));
        assert_eq!(trail.shown(), Some(&group("C")));
        assert_eq!(trail.step(true, &history, Some(0)), Step::Show(b));
        assert!(trail.show(b));
        assert_eq!(trail.step(false, &history, Some(0)), Step::Show(c));
        assert_eq!(trail.step(true, &history, Some(0)), Step::Show(a));
        assert!(trail.show(a));
        assert_eq!(trail.step(true, &history, Some(0)), Step::Reveal);
        // From the note the first is next, and from the note after them
        // the last.
        trail.hide();
        assert_eq!(trail.step(false, &history, Some(0)), Step::Show(a));
        assert_eq!(trail.step(true, &history, Some(1)), Step::Session(Some(c)));

        // The view shown is not opened twice.
        assert!(trail.show(b));
        trail.open(group("B"), &history, Some(0));
        assert_eq!(trail.stops.len(), 3);
        assert_eq!(trail.shown(), Some(&group("B")));

        // A view opened from the middle drops the views ahead in the gap.
        trail.open(group("D"), &history, Some(0));
        assert_eq!(
            stops(&trail),
            [
                (Some(0), "A".to_owned()),
                (Some(0), "B".to_owned()),
                (Some(0), "D".to_owned())
            ]
        );
        // One opened from the note drops them all, and no other gap's.
        trail.hide();
        trail.open(group("E"), &history, Some(1));
        trail.hide();
        trail.open(group("F"), &history, Some(0));
        assert_eq!(
            stops(&trail),
            [(Some(0), "F".to_owned()), (Some(1), "E".to_owned())]
        );
        // A stop that is gone cannot be shown.
        assert!(!trail.show(a));
        assert_eq!(trail.shown(), Some(&group("F")));
    }

    #[test]
    fn a_new_visit_drops_the_views_ahead_and_takes_the_pane() {
        // On the note at 1 of three, with a view after each note.
        let history = visits(&[0, 1, 2]);
        let mut trail = Trail::default();
        for (position, name) in [(0, "A"), (1, "B"), (2, "C")] {
            trail.hide();
            trail.open(group(name), &history, Some(position));
        }
        trail.hide();
        // The Session drops the visit at 2 and records another.
        trail.follow_visits(Some(1), Some(2), &visits(&[0, 1, 3]), true);
        assert_eq!(stops(&trail), [(Some(0), "A".to_owned())]);

        // From a view, the views behind it and itself stay behind the new
        // visit, and the pane is the note's.
        let mut trail = Trail::default();
        for name in ["A", "B", "C"] {
            trail.open(group(name), &history, Some(1));
        }
        let b = id(&trail, &group("B"));
        assert!(trail.show(b));
        trail.follow_visits(Some(1), Some(2), &visits(&[0, 1, 3]), true);
        assert_eq!(trail.shown(), None);
        assert_eq!(
            stops(&trail),
            [(Some(1), "A".to_owned()), (Some(1), "B".to_owned())]
        );
        assert_eq!(
            trail.step(true, &visits(&[0, 1, 3]), Some(2)),
            Step::Session(Some(b))
        );

        // A note asked for that made no visit (the one under the view)
        // takes the pane and drops nothing.
        assert!(trail.show(b));
        trail.follow_visits(Some(1), Some(3), &visits(&[0, 1, 3]), true);
        assert_eq!(trail.shown(), None);
        assert_eq!(trail.stops.len(), 2);
        // An op that neither visits nor asks for a note changes nothing.
        assert!(trail.show(b));
        trail.follow_visits(Some(1), Some(3), &visits(&[0, 1, 3]), false);
        assert_eq!(trail.shown(), Some(&group("B")));
    }

    #[test]
    fn a_view_whose_visit_is_gone_comes_after_the_one_before_it() {
        let history = visits(&[0, 1, 2]);
        let mut trail = Trail::default();
        for (position, name) in [(0, "A"), (1, "B"), (2, "C")] {
            trail.hide();
            trail.open(group(name), &history, Some(position));
        }
        // The note of the visit at 1 is deleted.
        trail.follow_visits(Some(2), Some(2), &visits(&[0, 2]), false);
        assert_eq!(
            stops(&trail),
            [
                (Some(0), "A".to_owned()),
                (Some(0), "B".to_owned()),
                (Some(2), "C".to_owned())
            ]
        );
        assert_eq!(trail.shown(), Some(&group("C")));
        // The first visit goes too: its views come before every visit, and
        // are reached by Back from the first that is left, over its note.
        trail.follow_visits(Some(2), Some(2), &visits(&[2]), false);
        assert_eq!(
            stops(&trail),
            [
                (None, "A".to_owned()),
                (None, "B".to_owned()),
                (Some(2), "C".to_owned())
            ]
        );
        let history = visits(&[2]);
        let [a, b] = ["A", "B"].map(|name| id(&trail, &group(name)));
        trail.hide();
        assert_eq!(trail.step(true, &history, Some(0)), Step::Show(b));
        assert!(trail.show(b));
        assert_eq!(trail.step(false, &history, Some(0)), Step::Reveal);
        assert_eq!(trail.step(true, &history, Some(0)), Step::Show(a));
        assert!(trail.show(a));
        assert_eq!(trail.step(true, &history, Some(0)), Step::End);
        assert_eq!(trail.step(false, &history, Some(0)), Step::Show(b));
    }

    #[test]
    fn with_no_visit_at_all_the_views_come_after_the_empty_pane() {
        let mut trail = Trail::default();
        trail.open(group("A"), &[], None);
        trail.open(group("B"), &[], None);
        let [a, b] = ["A", "B"].map(|name| id(&trail, &group(name)));
        assert_eq!(trail.step(false, &[], None), Step::End);
        assert_eq!(trail.step(true, &[], None), Step::Show(a));
        assert!(trail.show(a));
        assert_eq!(trail.step(false, &[], None), Step::Show(b));
        // Back from the first is the pane with no note, where Forward
        // shows it again.
        assert_eq!(trail.step(true, &[], None), Step::Reveal);
        trail.hide();
        assert_eq!(trail.step(true, &[], None), Step::End);
        assert_eq!(trail.step(false, &[], None), Step::Show(a));
        assert!(trail.show(a));
        // The first note visited comes after them.
        trail.follow_visits(None, None, &visits(&[0]), true);
        assert_eq!(trail.shown(), None);
        assert_eq!(stops(&trail), [(None, "A".to_owned())]);
        assert_eq!(trail.step(true, &visits(&[0]), Some(0)), Step::Show(a));
    }

    #[test]
    fn the_trail_keeps_as_many_stops_as_the_session_keeps_visits() {
        let history = visits(&[0, 1]);
        let mut trail = Trail::default();
        trail.open(group("first"), &history, Some(0));
        trail.hide();
        for index in 0..MAX_STOPS + 5 {
            trail.open(group(&format!("G{index}")), &history, Some(1));
        }
        assert_eq!(trail.stops.len(), MAX_STOPS);
        // The oldest went, and the newest is the one shown.
        assert_eq!(trail.stops[0].scope, group("G5"));
        assert_eq!(trail.shown(), Some(&group(&format!("G{}", MAX_STOPS + 4))));
    }

    #[test]
    fn a_note_opened_from_a_view_before_the_first_visit_comes_straight_after_it() {
        // Three views left before the first visit (their note deleted), the
        // second shown over the first visit's note.
        let history = visits(&[4, 6]);
        let mut trail = Trail::default();
        trail.open(group("A"), &[], None);
        trail.open(group("B"), &[], None);
        trail.open(group("C"), &[], None);
        let [a, b, c] = ["A", "B", "C"].map(|name| id(&trail, &group(name)));
        assert!(trail.show(b));
        assert_eq!(trail.step(false, &history, Some(0)), Step::Show(c));

        // A note is opened from it: the Session drops the visit ahead and
        // records the new one after the note under the view, and the view
        // ahead goes.
        let branched = visits(&[4, 7]);
        trail.follow_visits(Some(4), Some(6), &branched, true);
        assert_eq!(trail.shown(), None);
        assert_eq!(
            stops(&trail),
            [(Some(4), "A".to_owned()), (Some(4), "B".to_owned())]
        );
        // Back from the new note is the view, then the view behind it, then
        // the note they were over.
        assert_eq!(trail.step(true, &branched, Some(1)), Step::Session(Some(b)));
        assert!(trail.show(b));
        assert_eq!(trail.step(true, &branched, Some(0)), Step::Show(a));
        assert!(trail.show(a));
        assert_eq!(trail.step(true, &branched, Some(0)), Step::Reveal);
        trail.hide();
        assert_eq!(trail.step(true, &branched, Some(0)), Step::End);

        // From the note itself the views before the first visit stay there.
        let mut trail = Trail::default();
        trail.open(group("A"), &[], None);
        trail.hide();
        trail.follow_visits(Some(4), Some(6), &branched, true);
        assert_eq!(stops(&trail), [(None, "A".to_owned())]);
    }

    const LIST: &str = "# List\n\n- [ ] Mail the deck\n";
    const NOTES: &str = "# Notes\n\nplain words\n";

    /// A workspace with two sections: `Work/Tasks` holds the notes `List`
    /// and `Notes` and the board `Flow`, `Work/Other` the note `Third`, and
    /// `Home/Chores` the note `Fourth`.
    fn workspace(root: &Path) {
        for (path, text) in [
            ("Work/Tasks/List.md", LIST),
            ("Work/Tasks/Notes.md", NOTES),
            ("Work/Tasks/Flow.wren-board", FLOW),
            ("Work/Other/Third.md", "third\n"),
            ("Home/Chores/Fourth.md", "fourth\n"),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    /// An app whose `config.conf` holds `settings`, on the workspace at
    /// `root`, as a launch opens it.
    fn launch(directory: &Path, root: &Path, settings: &str) -> App {
        let config = directory.join("config.conf");
        std::fs::write(&config, settings).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.workspace_mode = true;
        app.session = Some(Session::open_with(root, app.session_options()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        app
    }

    const ON: &str = "tasks = on\nboards = on\n";

    /// The workspace with tasks and boards on, and `List` open.
    fn fixture() -> (tempfile::TempDir, App) {
        fixture_on(ON)
    }

    /// The same on `settings`.
    fn fixture_on(settings: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        workspace(&root);
        let mut app = launch(directory.path(), &root, settings);
        open(&mut app, "List");
        (directory, app)
    }

    /// The fixture on each engine.
    fn engines() -> impl Iterator<Item = (tempfile::TempDir, App)> {
        wren_editor::BackendKind::ALL.into_iter().map(|kind| {
            let engine = match kind {
                wren_editor::BackendKind::Iced => "classic",
                wren_editor::BackendKind::Owned => "advanced",
            };
            let (directory, app) = fixture_on(&format!("editor-engine = {engine}\n{ON}"));
            assert_eq!(app.editor.backend(), kind);
            (directory, app)
        })
    }

    fn note(app: &App, title: &str) -> NoteId {
        app.model()
            .unwrap()
            .notes
            .iter()
            .find(|note| note.title == title)
            .unwrap_or_else(|| panic!("no note titled {title}"))
            .id
    }

    /// Open a note as a click on its row does.
    fn open(app: &mut App, title: &str) -> Disposition {
        let target = app.note_target(note(app, title));
        app.dispatch(Command::OpenNote(target))
            .processed
            .disposition
    }

    fn show(app: &mut App, scope: Scope) -> Disposition {
        app.dispatch(Command::ShowTasks(scope))
            .processed
            .disposition
    }

    fn tasks_group() -> Scope {
        group("Work/Tasks")
    }

    fn run(app: &mut App, id: CommandId) -> Disposition {
        app.dispatch(Command::Run(id)).processed.disposition
    }

    fn document(app: &App) -> Option<NoteId> {
        app.session
            .as_ref()
            .unwrap()
            .document()
            .map(|document| document.note)
    }

    fn draft(app: &App) -> String {
        app.session
            .as_ref()
            .unwrap()
            .document()
            .map_or_else(String::new, |document| document.text.clone())
    }

    fn dirty(app: &App) -> bool {
        app.session
            .as_ref()
            .unwrap()
            .document()
            .is_some_and(wren_core::ops::Document::dirty)
    }

    fn state_file(root: &Path) -> String {
        std::fs::read_to_string(root.join(".wren/workspace.toml")).unwrap()
    }

    fn paste(text: &str) -> Message {
        Message::Editor(wren_editor::Message::Action(Action::Edit(Edit::Paste(
            Arc::new(text.into()),
        ))))
    }

    /// The view of `scope` is in the pane over the note `under`, which is
    /// still the Session's document and is still what the editor or the
    /// board viewer holds.
    #[track_caller]
    fn assert_view(app: &App, scope: &Scope, under: Option<NoteId>) {
        assert_eq!(app.surface(), Surface::Tasks);
        assert_eq!(app.task_view(), Some(scope));
        assert_eq!(document(app), under);
        assert_eq!(app.editor_note, under);
        assert_eq!(app.selected_note, under);
        match app.document_surface() {
            Surface::Board => assert_eq!(app.board.as_ref().unwrap().synced(), draft(app)),
            _ => assert_eq!(app.editor.text(), draft(app)),
        }
        assert!(app.ensure_editor_ready().is_err());
        assert!(!app.find_open);
    }

    #[test]
    fn back_and_forward_pass_through_notes_views_and_boards() {
        let (_directory, mut app) = fixture();
        let list = note(&app, "List");
        let flow = note(&app, "Flow");
        assert!(!app.command_enabled(CommandId::Back));
        assert!(!app.command_enabled(CommandId::Forward));

        // Note, then the view over it, then a board.
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        assert_view(&app, &tasks_group(), Some(list));
        assert_eq!(app.column, Column::Editor);
        assert!(app.task_view_has_keys());
        assert!(app.command_enabled(CommandId::Back));
        assert!(!app.command_enabled(CommandId::Forward));
        assert_eq!(open(&mut app, "Flow"), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Board);
        assert_eq!(document(&app), Some(flow));

        // Back: the view, over the note it was opened from.
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &tasks_group(), Some(list));
        assert!(app.board.is_none());
        assert!(app.command_enabled(CommandId::Forward));
        // Back: the note, which was never closed.
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(list));
        assert_eq!(app.editor.text(), LIST);
        assert!(!app.command_enabled(CommandId::Back));
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Blocked);
        assert_eq!(app.surface(), Surface::Text);

        // Forward: the view, the board, and no further.
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_view(&app, &tasks_group(), Some(list));
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Board);
        assert_eq!(document(&app), Some(flow));
        assert_eq!(app.board.as_ref().unwrap().synced(), FLOW);
        assert!(!app.command_enabled(CommandId::Forward));
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Blocked);
        assert_eq!(app.surface(), Surface::Board);
        assert_eq!(document(&app), Some(flow));

        // A view over the board: the viewer is put away whole, and comes
        // back as it was.
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(flow));
        assert_eq!(app.document_surface(), Surface::Board);
        for id in [
            CommandId::Board(wren_canvas::Command::SelectAll),
            CommandId::ExportSvg,
        ] {
            assert!(!app.command_enabled(id), "{id:?}");
        }
        let revision = app.board_revision;
        let epoch = app.board.as_ref().unwrap().epoch();
        let _ = app.update(Message::Board(wren_canvas::Message::Command(
            epoch,
            wren_canvas::Command::SelectAll,
        )));
        assert!(app.board.as_ref().unwrap().machine().selection().is_empty());
        assert_eq!(app.board_revision, revision);
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Board);
        assert_eq!(draft(&app), FLOW);
        assert!(!dirty(&app));
    }

    #[test]
    fn the_note_under_a_view_keeps_its_caret_and_its_undo_history() {
        for (_directory, mut app) in engines() {
            let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
                iced::widget::text_editor::Motion::DocumentEnd,
            ))));
            let _ = app.update(paste("- [ ] typed"));
            let typed = format!("{LIST}- [ ] typed");
            assert_eq!(app.editor.text(), typed);
            app.editor.move_cursor(Cursor {
                position: Position { line: 2, column: 4 },
                selection: None,
            });
            let caret = app.editor.cursor();

            assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
            // Opening it flushed the draft, as leaving a note does.
            assert!(!dirty(&app));
            assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
            assert_eq!(app.surface(), Surface::Text);
            assert_eq!(app.editor.text(), typed);
            assert_eq!(app.editor.cursor(), caret);
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            assert_eq!(app.editor.text(), LIST);
            assert_eq!(draft(&app), LIST);
        }
    }

    #[test]
    fn a_draft_that_cannot_be_flushed_keeps_the_view_shut() {
        let (directory, mut app) = fixture();
        let path = directory.path().join("workspace/Work/Tasks/List.md");
        let _ = app.update(paste("unsaved "));
        let typed = format!("unsaved {LIST}");
        let more = format!("unsaved more {LIST}");
        assert!(dirty(&app));
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        assert_eq!(show(&mut app, tasks_group()), Disposition::Blocked);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(app.task_view(), None);
        assert!(!app.command_enabled(CommandId::Forward));
        assert_eq!(draft(&app), typed);
        assert_eq!(app.editor.text(), typed);
        assert!(dirty(&app));
        assert!(app.banner.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), LIST);

        // Forward into a view from its note flushes the same way.
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), typed);
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        let _ = app.update(paste("more "));
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Blocked);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(draft(&app), more);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), typed);
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_view(&app, &tasks_group(), Some(note(&app, "List")));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), more);
    }

    #[test]
    fn leaving_a_view_for_a_note_with_a_draft_that_cannot_be_flushed_stays_on_the_view() {
        let (directory, mut app) = fixture();
        let list = note(&app, "List");
        let path = directory.path().join("workspace/Work/Tasks/List.md");
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        // A change the app itself applies to the note under the view, as a
        // task changed from the list will be: the editor's text, then the
        // draft.
        let changed = "# List\n\n- [x] Mail the deck\n";
        app.editor.replace_text(changed, true);
        app.text_changed(Instant::now());
        assert_eq!(draft(&app), changed);
        assert!(dirty(&app));

        for leave in ["note", "parent", "forward"] {
            let fail = |app: &mut App| app.session.as_mut().unwrap().fail_next(Fault::Write);
            let disposition = match leave {
                "note" => {
                    fail(&mut app);
                    open(&mut app, "Third")
                }
                "parent" => {
                    fail(&mut app);
                    let group = app
                        .model()
                        .unwrap()
                        .groups
                        .iter()
                        .find(|group| group.name == "Chores")
                        .unwrap()
                        .id;
                    app.dispatch(Command::SelectParent(Parent::Group(group)))
                        .processed
                        .disposition
                }
                _ => {
                    // A note ahead of the view, to step forward to.
                    assert_eq!(open(&mut app, "Notes"), Disposition::Completed);
                    assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
                    assert_view(&app, &tasks_group(), Some(list));
                    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
                    let again = "# List\n\n- [/] Mail the deck\n";
                    app.editor.replace_text(again, true);
                    app.text_changed(Instant::now());
                    fail(&mut app);
                    let disposition = run(&mut app, CommandId::Forward);
                    assert_eq!(draft(&app), again);
                    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
                    disposition
                }
            };
            assert_eq!(disposition, Disposition::Blocked, "{leave}");
            assert_view(&app, &tasks_group(), Some(list));
            assert!(dirty(&app), "{leave}");
            assert!(app.banner.is_some(), "{leave}");
            if leave != "forward" {
                assert_eq!(draft(&app), changed, "{leave}");
                assert_eq!(std::fs::read_to_string(&path).unwrap(), LIST);
            }
        }
        // The next try saves the draft and leaves.
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(note(&app, "Notes")));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# List\n\n- [/] Mail the deck\n"
        );
    }

    const EDITOR_COMMANDS: [CommandId; 17] = [
        CommandId::ToggleTask,
        CommandId::Bold,
        CommandId::Italic,
        CommandId::Code,
        CommandId::MoveLineUp,
        CommandId::MoveLineDown,
        CommandId::TaskCycleStatus,
        CommandId::TaskPriorityHigh,
        CommandId::TaskRaisePriority,
        CommandId::TaskClearDue,
        CommandId::TaskSetStatus,
        CommandId::TaskSetPriority,
        CommandId::TaskSetDue,
        CommandId::TaskEdit,
        CommandId::SpellSuggest,
        CommandId::Find,
        CommandId::Outline,
    ];

    #[test]
    fn the_editor_under_a_view_never_writes() {
        for (directory, mut app) in engines() {
            let list = note(&app, "List");
            let path = directory.path().join("workspace/Work/Tasks/List.md");
            // The caret on the task, where every task command is enabled.
            app.editor.move_cursor(Cursor {
                position: Position { line: 2, column: 6 },
                selection: None,
            });
            let _ = app.update(Message::Tick);
            assert!(app.command_enabled(CommandId::TaskCycleStatus));
            assert!(app.command_enabled(CommandId::ToggleTask));
            let caret = app.editor.cursor();
            let saved = modified(&path);

            assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
            let revision = app.edit_revision;
            let untouched = |app: &App| {
                assert_view(app, &tasks_group(), Some(list));
                assert_eq!(draft(app), LIST);
                assert!(!dirty(app));
                assert_eq!(app.editor.text(), LIST);
                assert_eq!(app.editor.cursor(), caret);
                assert_eq!(app.edit_revision, revision);
                assert_eq!(std::fs::read_to_string(&path).unwrap(), LIST);
                assert_eq!(modified(&path), saved);
                assert!(app.calendar.is_none() && app.task_form.is_none());
                assert!(app.editor.popover().is_none());
            };
            untouched(&app);

            // The editor's commands and the task commands, from the palette or
            // a menu.
            for id in EDITOR_COMMANDS {
                assert!(!app.command_enabled(id), "{id:?}");
                assert_eq!(run(&mut app, id), Disposition::Blocked, "{id:?}");
                untouched(&app);
            }
            // Their keys, which only a focused editor runs: the app's copy of
            // each press does nothing.
            for id in [
                CommandId::Bold,
                CommandId::ToggleTask,
                CommandId::TaskSetDue,
                CommandId::TaskEdit,
                CommandId::EditUndo,
            ] {
                let accel = app.preferences.keymap.accel(id.key()).cloned();
                let accel = accel.unwrap_or_else(|| panic!("{id:?} has no key"));
                let _ = app.update(Message::Key(Input::Accel(accel)));
                untouched(&app);
            }
            // Editor messages, as a key, a click or a stale widget would send
            // them.
            for message in [
                paste("bypass"),
                Message::Editor(wren_editor::Message::Intent(wren_editor::Intent::Bold)),
                Message::Editor(wren_editor::Message::Intent(wren_editor::Intent::Enter)),
                Message::Editor(wren_editor::Message::Action(Action::Edit(Edit::Enter))),
                Message::Editor(wren_editor::Message::Command(
                    CommandId::ToggleTask.payload(),
                )),
                Message::Editor(wren_editor::Message::Spell(
                    wren_editor::SpellEvent::OpenAtCaret,
                )),
                Message::Editor(wren_editor::Message::Undo),
                Message::Editor(wren_editor::Message::Redo),
            ] {
                let _ = app.update(message);
                untouched(&app);
            }

            // An engine switch rebuilds the editor from the note under the
            // view, and switching back does too.
            let engine = app.editor.backend();
            for _ in 0..2 {
                assert!(run(&mut app, CommandId::SwitchEngine).committed());
                assert_view(&app, &tasks_group(), Some(list));
                assert_eq!(app.editor.text(), LIST);
                assert!(!dirty(&app));
            }
            assert_eq!(app.editor.backend(), engine);
            assert!(run(&mut app, CommandId::ToggleMarkdownAssist).committed());
            assert!(run(&mut app, CommandId::ToggleMarkdownAssist).committed());

            // Autosave and losing the window's focus have nothing to write.
            tick_to(&mut app, 5);
            app.window_unfocused().unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), LIST);
            assert_eq!(modified(&path), saved);
            assert!(!dirty(&app));
            assert_view(&app, &tasks_group(), Some(list));
        }
    }

    #[test]
    fn escape_and_the_navigation_keys_do_nothing_in_a_view() {
        use crate::keys::Nav;

        let (_directory, mut app) = fixture();
        let list = note(&app, "List");
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        let processed = app
            .last_processed
            .clone()
            .map(|processed| processed.correlation);
        for nav in [
            Nav::Escape,
            Nav::Up,
            Nav::Down,
            Nav::Left,
            Nav::Right,
            Nav::Enter,
        ] {
            let _ = app.update(Message::Key(Input::Nav(nav)));
            assert_view(&app, &tasks_group(), Some(list));
            assert_eq!(app.column, Column::Editor, "{nav:?}");
            assert!(app.task_view_has_keys());
        }
        let _ = app.update(Message::Escape);
        assert_view(&app, &tasks_group(), Some(list));
        assert_eq!(
            app.last_processed
                .clone()
                .map(|processed| processed.correlation),
            processed
        );

        // From a browser column Escape gives the keyboard back to the pane,
        // which is the view's, and the arrows are the column's until then.
        let _ = run(&mut app, CommandId::ColumnLeft);
        assert_eq!(app.column, Column::Notes);
        assert!(!app.task_view_has_keys());
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert_eq!(app.column, Column::Editor);
        assert_view(&app, &tasks_group(), Some(list));
        assert!(app.task_view_has_keys());

        // A press in the view takes the keyboard too.
        let _ = run(&mut app, CommandId::ColumnLeft);
        let _ = app.update(Message::TaskView(Event::Press));
        assert_eq!(app.column, Column::Editor);
        assert!(app.task_view_has_keys());

        // A palette or a dialog over the view keeps it under them, and
        // Escape closes only that.
        assert!(app
            .open_palette(Mode::Commands)
            .processed
            .disposition
            .committed());
        assert!(!app.task_view_has_keys() || app.palette.is_some());
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(app.palette.is_none());
        assert_view(&app, &tasks_group(), Some(list));
    }

    #[test]
    fn a_note_asked_for_takes_the_pane_and_back_returns_to_the_view() {
        let (_directory, mut app) = fixture();
        let list = note(&app, "List");
        let third = note(&app, "Third");
        let scope = Scope::Section(RelPath::new("Work").unwrap());
        assert_eq!(show(&mut app, scope.clone()), Disposition::Completed);

        // The note under the view, asked for again: no visit is made, and
        // the view is one step ahead.
        assert_eq!(open(&mut app, "List"), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(list));
        assert!(!app.command_enabled(CommandId::Back));
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_view(&app, &scope, Some(list));

        // A search result.
        assert!(app
            .open_palette(Mode::Search)
            .processed
            .disposition
            .committed());
        assert_view(&app, &scope, Some(list));
        let target = app.note_target(third);
        assert!(app
            .dispatch(Command::OpenNote(target))
            .processed
            .disposition
            .committed());
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(third));
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &scope, Some(list));

        // A parent chosen in a column opens its first note.
        let chores = app
            .model()
            .unwrap()
            .groups
            .iter()
            .find(|group| group.name == "Chores")
            .unwrap()
            .id;
        assert!(app
            .dispatch(Command::SelectParent(Parent::Group(chores)))
            .processed
            .disposition
            .committed());
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(note(&app, "Fourth")));
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &scope, Some(list));

        // Another view from this one, and the first is behind it.
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(list));
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &scope, Some(list));
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(list));
        // The view shown, asked for again, is not a second stop.
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        assert_eq!(app.task_views.stops.len(), 2);

        // A raw step of the Session's history leaves the view too.
        let _ = app.apply_core(Op::Forward).unwrap();
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(note(&app, "Fourth")));
    }

    /// The sequence of note visits both workspaces of the next test see.
    const VISITS: [&str; 6] = ["Notes", "Third", "Flow", "Fourth", "List", "Third"];

    #[test]
    fn views_leave_the_state_file_as_the_same_note_visits_alone_write_it() {
        let directory = crate::test_support::tempdir();
        let mut files = Vec::new();
        for views in [false, true] {
            let home = directory
                .path()
                .join(if views { "with" } else { "without" });
            let root = home.join("workspace");
            workspace(&root);
            let mut app = launch(&home, &root, ON);
            open(&mut app, "List");
            let mut seen = Vec::new();
            for (index, title) in VISITS.iter().enumerate() {
                if views {
                    let scope = match index % 4 {
                        0 => Scope::Workspace,
                        1 => Scope::Section(RelPath::new("Work").unwrap()),
                        2 => tasks_group(),
                        _ => Scope::Note(RelPath::new("Work/Tasks/List.md").unwrap()),
                    };
                    assert_eq!(show(&mut app, scope), Disposition::Completed);
                    // A view writes nothing, shown or stepped through.
                    let before = state_file(&root);
                    if index % 2 == 1 {
                        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
                        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
                        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
                    }
                    assert_eq!(state_file(&root), before);
                }
                assert_eq!(open(&mut app, title), Disposition::Completed);
                seen.push(state_file(&root));
            }
            // Back to the note before and Forward again: one step each in
            // notes, and with views two more each, through the two views
            // between them.
            let target = note(&app, "List");
            let mut steps = 0;
            while document(&app) != Some(target) || app.surface() == Surface::Tasks {
                assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
                steps += 1;
                assert!(steps < 20);
            }
            let target = note(&app, "Third");
            while document(&app) != Some(target) || app.surface() == Surface::Tasks {
                assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
                steps += 1;
                assert!(steps < 40);
            }
            assert_eq!(steps, if views { 6 } else { 2 });
            seen.push(state_file(&root));
            files.push(seen);
        }
        assert_eq!(files[0], files[1]);
        let last = files[1].last().unwrap();
        assert!(last.contains("history = ["), "{last}");
        for word in ["tasks", "view", "serial"] {
            assert!(!last.contains(word), "{word} in {last}");
        }
    }

    #[test]
    fn a_restart_reopens_the_last_note_and_never_the_view() {
        let (directory, mut app) = fixture();
        let root = directory.path().join("workspace");
        assert_eq!(open(&mut app, "Notes"), Disposition::Completed);
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        let saved = state_file(&root);
        let _ = app.request_quit().unwrap();
        assert_eq!(state_file(&root), saved);
        drop(app);

        let app = launch(directory.path(), &root, ON);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(app.task_view(), None);
        assert!(app.task_views.stops.is_empty());
        assert_eq!(document(&app), Some(note(&app, "Notes")));
        assert_eq!(app.editor.text(), NOTES);
        // History is the notes', as it was written.
        assert!(app.command_enabled(CommandId::Back));
        assert!(!app.command_enabled(CommandId::Forward));
        assert_eq!(state_file(&root), saved);
    }

    #[test]
    fn with_tasks_off_no_view_opens_and_history_is_the_sessions() {
        let (directory, mut app) = fixture();
        let root = directory.path().join("workspace");
        assert_eq!(open(&mut app, "Notes"), Disposition::Completed);
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        let _ = app.request_quit().unwrap();
        drop(app);

        // Tasks are turned off, and Wren started again.
        let mut app = launch(directory.path(), &root, "tasks = off\nboards = on\n");
        assert!(!app.preferences.tasks);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(note(&app, "Notes")));
        let saved = state_file(&root);
        for scope in [Scope::Workspace, tasks_group()] {
            let outcome = app.dispatch(Command::ShowTasks(scope));
            assert_eq!(outcome.processed.disposition, Disposition::Blocked);
            assert_eq!(outcome.processed.warning.as_deref(), Some("Tasks are off"));
            assert_eq!(app.surface(), Surface::Text);
            assert!(app.task_views.stops.is_empty());
        }
        assert_eq!(state_file(&root), saved);
        // Back and Forward are enabled as the Session says, and step it.
        for id in [CommandId::Back, CommandId::Forward] {
            let session = app.session.as_ref().unwrap();
            let expected = if id == CommandId::Back {
                session.can_back()
            } else {
                session.can_forward()
            };
            assert_eq!(app.command_enabled(id), expected, "{id:?}");
        }
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_eq!(document(&app), Some(note(&app, "List")));
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_eq!(document(&app), Some(note(&app, "Notes")));
    }

    #[test]
    fn a_view_is_refused_for_what_is_not_there_or_under_the_task_form() {
        let (_directory, mut app) = fixture();
        for scope in [
            group("Work/Missing"),
            Scope::Section(RelPath::new("Work/Tasks").unwrap()),
            Scope::Note(RelPath::new("Work/Tasks").unwrap()),
        ] {
            assert_eq!(show(&mut app, scope), Disposition::Blocked);
            assert_eq!(app.surface(), Surface::Text);
        }
        assert!(Scope::parse("group", None).is_err());
        assert!(Scope::parse("group", Some("../Work")).is_err());
        assert!(Scope::parse("folder", Some("Work")).is_err());
        assert_eq!(Scope::parse("workspace", None), Ok(Scope::Workspace));
        assert_eq!(Scope::parse("group", Some("Work/Tasks")), Ok(tasks_group()));
        assert_eq!(tasks_group().kind(), "group");
        assert_eq!(
            tasks_group().prefix().map(RelPath::as_path),
            Some(Path::new("Work/Tasks"))
        );
        assert_eq!(Scope::Workspace.prefix(), None);

        // The task form is over the note's text: it is closed first.
        app.editor.move_cursor(Cursor {
            position: Position { line: 2, column: 6 },
            selection: None,
        });
        let _ = app.update(Message::Tick);
        assert!(run(&mut app, CommandId::TaskEdit).committed());
        assert!(app.task_form.is_some());
        assert_eq!(show(&mut app, tasks_group()), Disposition::Blocked);
        assert!(app.task_form.is_some());
        assert_eq!(app.surface(), Surface::Text);
    }

    #[test]
    fn a_view_follows_its_item_through_a_rename_and_a_move_and_goes_with_it() {
        let (directory, mut app) = fixture();
        let list = note(&app, "List");
        let model = app.model().unwrap();
        let tasks = model
            .groups
            .iter()
            .find(|group| group.name == "Tasks")
            .unwrap();
        let (tasks, work) = (tasks.id, tasks.section);
        let home = model
            .sections
            .iter()
            .find(|section| section.name == "Home")
            .unwrap()
            .id;

        // Three views in history: the note's, the section's, and the
        // group's, which is the one shown.
        let note_scope = Scope::Note(RelPath::new("Work/Tasks/List.md").unwrap());
        let section_scope = Scope::Section(RelPath::new("Work").unwrap());
        for scope in [note_scope, section_scope, tasks_group()] {
            assert_eq!(show(&mut app, scope), Disposition::Completed);
        }
        let paths = |app: &App| {
            stops(&app.task_views)
                .into_iter()
                .map(|(_, path)| path)
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(&app), ["Work/Tasks/List.md", "Work", "Work/Tasks"]);

        // The group is renamed: all three still name their items.
        let _ = app
            .apply_core(Op::Rename {
                target: ItemRef::Group(tasks),
                new_name: "Doing".into(),
            })
            .unwrap();
        assert_eq!(paths(&app), ["Work/Doing/List.md", "Work", "Work/Doing"]);
        assert_view(&app, &group("Work/Doing"), Some(list));
        assert_eq!(app.task_view_label(&group("Work/Doing")), "Doing");

        // The section is renamed, the note is renamed, the group is moved.
        let _ = app
            .apply_core(Op::Rename {
                target: ItemRef::Section(work),
                new_name: "Office".into(),
            })
            .unwrap();
        assert_eq!(
            paths(&app),
            ["Office/Doing/List.md", "Office", "Office/Doing"]
        );
        let _ = app
            .apply_core(Op::Rename {
                target: ItemRef::Note(list),
                new_name: "Plan".into(),
            })
            .unwrap();
        let _ = app
            .apply_core(Op::MoveGroup {
                group: tasks,
                to: home,
            })
            .unwrap();
        assert_eq!(paths(&app), ["Home/Doing/Plan.md", "Office", "Home/Doing"]);
        assert_view(&app, &group("Home/Doing"), Some(list));
        assert!(directory
            .path()
            .join("workspace/Home/Doing/Plan.md")
            .exists());

        // The note is deleted: its view goes from history, as its visits
        // do, and the view shown stays, over no note.
        let _ = app
            .apply_core(Op::Delete {
                target: ItemRef::Note(list),
            })
            .unwrap();
        assert_eq!(paths(&app), ["Office", "Home/Doing"]);
        assert_view(&app, &group("Home/Doing"), None);
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &Scope::Section(RelPath::new("Office").unwrap()), None);
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);

        // The group is deleted under the view shown: the pane is the
        // note's again, here no note's, and the other view is a step from
        // it.
        let _ = app
            .apply_core(Op::Delete {
                target: ItemRef::Group(tasks),
            })
            .unwrap();
        assert_eq!(paths(&app), ["Office"]);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(app.task_view(), None);
        assert_eq!(document(&app), None);
        assert!(!app.command_enabled(CommandId::Back));

        // Outside Wren: a folder removed is a scope gone at the next scan.
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Tasks);
        std::fs::remove_dir_all(directory.path().join("workspace/Office")).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert!(app.task_views.stops.is_empty());
        assert_eq!(app.surface(), Surface::Text);
    }

    #[test]
    fn another_workspace_starts_with_no_views() {
        let (_directory, mut app) = fixture();
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        assert_eq!(app.task_views.stops.len(), 1);
        app.hydrate_navigation();
        assert!(app.task_views.stops.is_empty());
        assert_eq!(app.surface(), Surface::Text);
    }

    /// Delete a file or a folder of the workspace as another program
    /// would, and let the Session find out.
    fn delete_outside(app: &mut App, directory: &tempfile::TempDir, path: &str) {
        let path = directory.path().join("workspace").join(path);
        if path.is_dir() {
            std::fs::remove_dir_all(path).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
        let _ = app.process_core(Op::Reconcile);
    }

    fn focuses(app: &App) -> Vec<Option<Surface>> {
        std::mem::take(&mut *app.focus_given.borrow_mut())
    }

    #[test]
    fn back_from_a_view_reopens_its_note_when_the_open_one_was_deleted_under_it() {
        let (directory, mut app) = fixture();
        let list = note(&app, "List");
        assert_eq!(open(&mut app, "Notes"), Disposition::Completed);
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);

        // The note under the views goes, outside Wren: nothing is open, the
        // views stay, and they come after the visit that is left.
        delete_outside(&mut app, &directory, "Work/Tasks/Notes.md");
        assert_view(&app, &tasks_group(), None);
        let visits = app.session.as_ref().unwrap().visits().to_vec();
        assert_eq!(
            visits.iter().map(|visit| visit.note).collect::<Vec<_>>(),
            [list]
        );
        assert_eq!(
            stops(&app.task_views),
            [
                (Some(visits[0].serial), String::new()),
                (Some(visits[0].serial), "Work/Tasks".to_owned())
            ]
        );

        // Between the views no note opens.
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, None);
        // Back from the first is the note of that visit, opened again, and
        // no new visit.
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(list));
        assert_eq!(app.editor.text(), LIST);
        assert_eq!(app.session.as_ref().unwrap().visits(), visits);
        assert!(!app.command_enabled(CommandId::Back));
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(list));

        // A note that cannot be opened again keeps the view in the pane.
        let (directory, mut app) = fixture();
        assert_eq!(open(&mut app, "Notes"), Disposition::Completed);
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        delete_outside(&mut app, &directory, "Work/Tasks/Notes.md");
        std::fs::remove_file(directory.path().join("workspace/Work/Tasks/List.md")).unwrap();
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Blocked);
        assert_view(&app, &Scope::Workspace, None);
        assert!(app.banner.is_some());
    }

    #[test]
    fn a_view_does_not_leave_for_another_while_the_draft_cannot_be_flushed() {
        let (directory, mut app) = fixture();
        let list = note(&app, "List");
        let path = directory.path().join("workspace/Work/Tasks/List.md");
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        let edit = |app: &mut App, text: &str| {
            app.editor.replace_text(text, true);
            app.text_changed(Instant::now());
            assert!(dirty(app));
            app.session.as_mut().unwrap().fail_next(Fault::Write);
        };

        // Another scope asked for from the view.
        let first = "# List\n\n- [x] Mail the deck\n";
        edit(&mut app, first);
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Blocked);
        assert_view(&app, &tasks_group(), Some(list));
        assert_eq!(app.task_views.stops.len(), 1);
        assert_eq!(draft(&app), first);
        assert!(dirty(&app));
        assert!(app.banner.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), LIST);
        // The view shown, asked for again, is no transition and no flush.
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), LIST);
        // The write that was to fail has not been tried: the next try at
        // another scope meets it, and the one after saves and goes.
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Blocked);
        assert_view(&app, &tasks_group(), Some(list));
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(list));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

        // Back and Forward between the two views.
        let second = "# List\n\n- [/] Mail the deck\n";
        for (id, from, to) in [
            (CommandId::Back, Scope::Workspace, tasks_group()),
            (CommandId::Forward, tasks_group(), Scope::Workspace),
        ] {
            let text = if id == CommandId::Back { second } else { LIST };
            let saved = std::fs::read_to_string(&path).unwrap();
            edit(&mut app, text);
            assert_eq!(run(&mut app, id), Disposition::Blocked, "{id:?}");
            assert_view(&app, &from, Some(list));
            assert_eq!(app.task_views.stops.len(), 2);
            assert_eq!(draft(&app), text);
            assert!(dirty(&app));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
            assert_eq!(run(&mut app, id), Disposition::Completed, "{id:?}");
            assert_view(&app, &to, Some(list));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn a_note_opened_from_a_view_left_before_the_first_visit_is_one_step_from_it() {
        let (directory, mut app) = fixture();
        assert_eq!(show(&mut app, Scope::Workspace), Disposition::Completed);
        assert_eq!(open(&mut app, "Notes"), Disposition::Completed);
        let notes = note(&app, "Notes");

        // The note the view was opened from goes while another is open: the
        // view is before the first visit left, and Back shows it over that
        // visit's note.
        delete_outside(&mut app, &directory, "Work/Tasks/List.md");
        assert_eq!(document(&app), Some(notes));
        assert_eq!(stops(&app.task_views), [(None, String::new())]);
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(notes));
        assert!(!app.command_enabled(CommandId::Back));

        // A note opened from it: Back is the view, at once, and then the
        // note it was over.
        assert_eq!(open(&mut app, "Third"), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(notes));
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(document(&app), Some(notes));
        assert!(!app.command_enabled(CommandId::Back));
        // And forward again, through the view to the note.
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_view(&app, &Scope::Workspace, Some(notes));
        assert_eq!(run(&mut app, CommandId::Forward), Disposition::Completed);
        assert_eq!(document(&app), Some(note(&app, "Third")));
    }

    #[test]
    fn a_view_that_goes_in_the_background_gives_the_keyboard_back_only_if_it_had_it() {
        let other = || group("Work/Other");
        let chores = || group("Home/Chores");

        // Under a palette: the search field keeps the keyboard.
        let (directory, mut app) = fixture();
        let list = note(&app, "List");
        assert_eq!(show(&mut app, other()), Disposition::Completed);
        assert!(app
            .open_palette(Mode::Search)
            .processed
            .disposition
            .committed());
        let _ = focuses(&app);
        delete_outside(&mut app, &directory, "Work/Other");
        assert_eq!(app.task_view(), None);
        assert_eq!(app.surface(), Surface::Text);
        assert!(app.palette.is_some());
        assert_eq!(focuses(&app), []);

        // Under a dialog: its field keeps it.
        let _ = app.close_palette();
        assert_eq!(show(&mut app, chores()), Disposition::Completed);
        assert!(run(&mut app, CommandId::NewSection).committed());
        assert!(app.dialog.is_some());
        let _ = focuses(&app);
        delete_outside(&mut app, &directory, "Home/Chores");
        assert_eq!(app.task_view(), None);
        assert!(app.dialog.is_some());
        assert_eq!(focuses(&app), []);

        // With a browser column active: the column keeps it.
        let (directory, mut app) = fixture();
        assert_eq!(show(&mut app, other()), Disposition::Completed);
        let _ = run(&mut app, CommandId::ColumnLeft);
        assert_eq!(app.column, Column::Notes);
        let _ = focuses(&app);
        delete_outside(&mut app, &directory, "Work/Other");
        assert_eq!(app.task_view(), None);
        assert_eq!(focuses(&app), []);
        assert_eq!(app.column, Column::Notes);

        // With the keyboard in the view: the note takes it.
        let (directory, mut app) = fixture();
        assert_eq!(show(&mut app, other()), Disposition::Completed);
        assert!(app.task_view_has_keys());
        let _ = focuses(&app);
        delete_outside(&mut app, &directory, "Work/Other");
        assert_eq!(app.task_view(), None);
        assert_eq!(document(&app), Some(list));
        assert_eq!(focuses(&app), [Some(Surface::Text)]);
    }

    #[test]
    fn the_view_has_no_keys_under_what_takes_the_keyboard_over_it() {
        let (_directory, mut app) = fixture();
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        assert!(app.task_view_has_keys());

        assert!(app
            .open_palette(Mode::Commands)
            .processed
            .disposition
            .committed());
        assert_eq!(app.surface(), Surface::Tasks);
        assert!(!app.task_view_has_keys());
        let _ = app.close_palette();
        assert!(app.task_view_has_keys());

        assert!(run(&mut app, CommandId::NewSection).committed());
        assert!(app.dialog.is_some());
        assert_eq!(app.surface(), Surface::Tasks);
        assert!(!app.task_view_has_keys());
        let _ = app.update(Message::CancelDialog);
        assert!(app.task_view_has_keys());
    }

    #[test]
    fn a_step_of_history_gives_the_keyboard_once_to_what_the_pane_ends_up_showing() {
        let (_directory, mut app) = fixture();
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        assert_eq!(open(&mut app, "Flow"), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Board);

        // Back from the board to the view over the note before it: the
        // note is opened under the view, and never given the keyboard.
        let _ = focuses(&app);
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_eq!(app.surface(), Surface::Tasks);
        assert_eq!(focuses(&app), [Some(Surface::Tasks)]);
        // Each other step gives it once too.
        for (id, surface) in [
            (CommandId::Back, Surface::Text),
            (CommandId::Forward, Surface::Tasks),
            (CommandId::Forward, Surface::Board),
        ] {
            assert_eq!(run(&mut app, id), Disposition::Completed);
            assert_eq!(focuses(&app), [Some(surface)], "{id:?}");
        }
    }

    #[cfg(feature = "smoke-test")]
    #[test]
    fn the_harness_refuses_the_editor_and_the_views_keys_when_neither_has_the_keyboard() {
        use serde_json::json;

        use crate::test_ipc::Request;

        fn ask(
            app: &mut App,
            op: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            let (request, mut reply) = Request::detached(op, params);
            let _ = app.handle_test(request);
            reply.try_recv().expect("the request is answered at once")
        }
        let escape = || json!({"key": {"key": "escape", "mods": []}});

        let (_directory, mut app) = fixture();
        // Over a note the editor can be given the keyboard.
        assert!(ask(&mut app, "editor.focus", json!({})).is_ok());
        assert_eq!(show(&mut app, tasks_group()), Disposition::Completed);
        // Under the view it is not in the window.
        assert!(ask(&mut app, "editor.focus", json!({}))
            .unwrap_err()
            .contains("task view"));
        assert_eq!(
            ask(&mut app, "tasks.view", json!({})).unwrap()["keys"],
            true
        );
        assert!(ask(&mut app, "tasks.view", escape()).is_ok());

        // Under a palette the view has no keys: a key sent to it is
        // refused, and the palette stays open.
        assert!(app
            .open_palette(Mode::Commands)
            .processed
            .disposition
            .committed());
        let state = ask(&mut app, "tasks.view", json!({})).unwrap();
        assert_eq!(
            (&state["open"], &state["keys"]),
            (&json!(true), &json!(false))
        );
        assert!(ask(&mut app, "tasks.view", escape())
            .unwrap_err()
            .contains("does not have the keyboard"));
        assert!(app.palette.is_some());
        let _ = app.close_palette();

        // Under a dialog too.
        assert!(run(&mut app, CommandId::NewSection).committed());
        assert_eq!(
            ask(&mut app, "tasks.view", json!({})).unwrap()["keys"],
            false
        );
        assert!(ask(&mut app, "tasks.view", escape()).is_err());
        assert!(app.dialog.is_some());
    }
}
