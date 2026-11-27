//! The "Create or edit task" form (discovery/tasks.md §6, item 3; item
//! 6.1): a dialog over the note with the caret's task in fields (the
//! description, the status, the due date with the calendar, the priority
//! and the owners) and a "Writes" line that shows, as the fields change,
//! the Markdown that saving writes.
//!
//! It opens from `task.edit` (⇧⌘↵ / Ctrl+Shift+Enter, the palette) at the
//! caret's line, and from the task menu's Edit task… row, with tasks on,
//! on both engines: it is the app's own dialog, not a popover of the
//! editor's. On a task it edits the task; on a blank line, or a line the
//! plain toggle would give a box, it makes one there
//! (`wren_markdown::task::Form`). Opening gives the description field the
//! keyboard. The keys after the request in its batch of events were typed
//! before the form could be seen, and reach neither the note nor the form:
//! the editor's widget drops them for a request the app's source of menus
//! answers by taking the keyboard (`calendar::TaskSource`), and while the
//! form is open the app applies no message of the editor's at all.
//!
//! **Saving** writes the fields the user changed as one compound line
//! edit, through the editor as one undo step: on a task with
//! `wren_markdown::task::edit` and [`wren_editor::EditorView::apply_line_edits`],
//! and on a line that is no task yet as one replacement of the line
//! ([`wren_editor::EditorView::apply`]). It is refused, and the form stays
//! open with a message and what was typed in it, once the note's text
//! changed in any way since the form opened (a reload of the note from
//! disk), and while the due field names no date. A form nobody changed
//! closes and writes nothing.
//!
//! **Keys** (`form_keys.rs`): Tab and Shift+Tab walk the description, the
//! status, the due date, the priority, the owners, Save and Cancel; Enter
//! saves; Escape cancels. The owners field takes Enter, and Escape, for
//! the name typed in it first; the due field's Down, or its button, opens
//! the calendar (`calendar.rs`) under it, whose field then has the
//! keyboard, and which gives it back to the due field however it closes.
//!
//! It closes, writing nothing, on Escape, Cancel and a press outside it,
//! and when what it was opened for goes: another note, a workspace
//! loading, the palette or a dialog opening. Closing gives the editor back
//! the keyboard with its caret and selection as they were (after a save,
//! where the line edit moved them). Closed while an input method composes
//! in one of its fields, the editor then drops what one commits until it
//! sees input of its own ([`wren_editor::EditorView::hand_back`]): a
//! composition begun in the form is never the note's. Closed with none
//! composing, what one commits next (the emoji picker's) is the note's.
//!
//! It also opens for a row of the task view ([`Target::Row`]): it reads
//! the task from the row's line, and Save writes the fields changed
//! through the one path every write from the view takes
//! (`App::write_task_row`, `task_target.rs`), which refuses, the form
//! staying open with why, once the task no longer reads as the row has
//! it. It closes, writing nothing, once the view has gone.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use iced::widget::{
    button, column, container, mouse_area, opaque, pin, row, stack, text, text_input,
};
use iced::{Element, Fill, Point, Rectangle, Size, Task};
use wren_core::model::NoteId;
use wren_markdown::calendar::{self, Calendar, Caption, Outcome};
use wren_markdown::date::Parsed;
use wren_markdown::status::{StatusType, STATUSES};
use wren_markdown::task::{self, DateField, Form, FormKind, Priority};

use super::board::Surface;
use super::calendar::{place, Event as CalendarEvent, FIELD as CALENDAR_FIELD};
use super::commands::{CommandId, Disposition, Gateway};
use super::task_target::RowTarget;
use super::{App, Message};
use crate::date_field::FieldEvent;
use crate::form_keys::{self, FormKey, Slot as KeySlot, SlotKind};
use crate::{chrome, fonts, theme};

/// The shortcut presses the task form's keys took and captured
/// ([`form_keys::FormKeys::claims`]) whose copies the app's key handling
/// has yet to hear of. Read by [`keys_with_form`], which runs where the
/// app is not.
pub(super) static FORM_CLAIMS: AtomicUsize = AtomicUsize::new(0);

/// The app's copies of key presses with tasks on: as with tasks off
/// ([`super::key_message`]), but a shortcut the task form took is the
/// form's ([`Message::FormShortcut`]).
pub(super) fn keys_with_form(
    event: iced::Event,
    status: iced::event::Status,
    window: iced::window::Id,
) -> Option<Message> {
    copy_of(event, status, window, &FORM_CLAIMS)
}

/// The app's copy of a key press, the form having taken `claims` shortcut
/// presses it has yet to hear of. Each such press reaches the app
/// captured, in order, so a captured shortcut while one is owed is that
/// press: whether the form was still open when its copy arrives or not.
fn copy_of(
    event: iced::Event,
    status: iced::event::Status,
    window: iced::window::Id,
    claims: &AtomicUsize,
) -> Option<Message> {
    let message = super::key_message(event, status, window)?;
    match message {
        Message::Key(crate::keys::Input::Accel(accel))
            if status == iced::event::Status::Captured
                && claims
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                    .is_ok() =>
        {
            Some(Message::FormShortcut(accel))
        }
        message => Some(message),
    }
}

/// The fields' widget ids.
pub(super) const DESCRIPTION: &str = "task-form-description";
pub(super) const DUE: &str = "task-form-due";
pub(super) const OWNERS: &str = "task-form-owners";

/// The form's slots, in the order Tab walks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Slot {
    Description,
    Status,
    Due,
    Priority,
    Owners,
    Save,
    Cancel,
}

impl Slot {
    pub(super) const ALL: [Self; 7] = [
        Self::Description,
        Self::Status,
        Self::Due,
        Self::Priority,
        Self::Owners,
        Self::Save,
        Self::Cancel,
    ];

    pub(super) fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|slot| *slot == self)
            .expect("every slot is listed")
    }

    pub(super) fn kind(self) -> SlotKind {
        match self {
            Self::Description => SlotKind::Text,
            Self::Due => SlotKind::Due,
            Self::Owners => SlotKind::Owners,
            Self::Status | Self::Priority => SlotKind::Choice,
            Self::Save | Self::Cancel => SlotKind::Button,
        }
    }

    pub(super) fn id(self) -> Option<&'static str> {
        match self {
            Self::Description => Some(DESCRIPTION),
            Self::Due => Some(DUE),
            Self::Owners => Some(OWNERS),
            _ => None,
        }
    }

    #[cfg_attr(not(feature = "smoke-test"), allow(dead_code))]
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Status => "status",
            Self::Due => "due",
            Self::Priority => "priority",
            Self::Owners => "owners",
            Self::Save => "save",
            Self::Cancel => "cancel",
        }
    }

    /// Give the slot the keyboard: its field's focus, or for a choice or
    /// a button no field's.
    pub(super) fn focus(self) -> Task<Message> {
        match self.id() {
            Some(id) => iced::widget::operation::focus(id),
            None => iced::advanced::widget::operate(
                iced::advanced::widget::operation::focusable::unfocus(),
            ),
        }
    }
}

/// The priorities in the form's order, from highest to lowest, then none.
pub(super) const PRIORITIES: [Option<Priority>; 6] = [
    Some(Priority::Highest),
    Some(Priority::High),
    Some(Priority::Medium),
    Some(Priority::Low),
    Some(Priority::Lowest),
    None,
];

/// The most owners suggested at once.
const SUGGESTIONS: usize = 6;

/// What the form hands the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A key the form's keys took, or the keyboard moved.
    Key(FormKey),
    /// Enter or Tab in the owners field ([`FormKey::Accept`],
    /// [`FormKey::Complete`]), on the owners of this generation as drawn.
    OwnerKey(FormKey, u64),
    /// The description field's text is now this.
    Description(String),
    /// The due field's text is now this.
    Due(String),
    /// The owners field's text is now this.
    Owner(String),
    /// A status was clicked.
    Status(StatusType),
    /// A priority was clicked.
    Priority(Option<Priority>),
    /// A suggested owner was clicked.
    AddOwner(String),
    /// An owner's remove button was clicked.
    RemoveOwner(String),
    /// The due field's calendar button was clicked.
    OpenCalendar,
    /// Input from the calendar over the form.
    Calendar(CalendarEvent),
    Save,
    /// Cancel, or a press outside the form.
    Cancel,
}

/// The open form.
#[derive(Debug, Clone)]
pub(super) struct Open {
    pub(super) form: Form,
    /// The task Save writes to.
    target: Target,
    /// Where the task is, as the header says it.
    place: String,
    pub(super) focus: Slot,
    /// The calendar gave the due field back the keyboard, and the field
    /// has seen no input of its own since: see `FormKeys::handback`.
    pub(super) due_handback: bool,
    /// An input method is composing in one of the form's fields, and in
    /// the calendar's, as they last said; the calendar's until the due
    /// field sees input of its own after it closed.
    composing: bool,
    calendar_composing: bool,
    /// A choice of an owner was refused, and the owners field's emptying,
    /// which the form's keys report right after it, is ignored: the name
    /// typed stays.
    keep_owner: bool,
    /// The suggested owner chosen with Up and Down, if any, by its
    /// case-folded name: a refresh of the owners that moves it keeps it
    /// chosen, and one that drops it chooses no other.
    pub(super) suggestion: Option<String>,
    /// The calendar, while it is open under the due field.
    pub(super) calendar: Option<Calendar>,
    /// Why the last save wrote nothing.
    pub(super) error: Option<String>,
    /// The owners suggested, with what they were found for: the owners'
    /// generation, the name typed and the owners chosen, case folded. A
    /// view reads them three times, and a key once.
    suggested: RefCell<Option<(SuggestedFor, Vec<String>)>>,
    /// How many times they were found.
    #[cfg(test)]
    found: std::cell::Cell<usize>,
}

/// The task the form writes to.
#[derive(Debug, Clone)]
enum Target {
    /// A line of the note in the editor.
    Editor {
        /// The note it was opened over, and its text's revision then: any
        /// change to the text since (a reload from disk, say) refuses
        /// Save, for the same line at the same place may be another task
        /// now.
        note: Option<NoteId>,
        revision: u64,
        /// The engine the note was in.
        engine: wren_editor::BackendKind,
    },
    /// A row of the task view.
    Row(RowTarget),
}

/// What the owners the form suggests were found for.
type SuggestedFor = (u64, String, Vec<String>);

impl Open {
    pub(super) fn title(&self) -> &'static str {
        match self.form.kind() {
            FormKind::Edit => "Edit task",
            FormKind::New => "New task",
            FormKind::Make => "Make a task",
        }
    }
}

/// The form's width, its insets and the heights its rows above the due
/// field are given, which place the calendar under that field before the
/// form is laid out.
const WIDTH: f32 = 540.0;
const PAD_Y: f32 = 16.0;
const PAD_X: f32 = 18.0;
const GAP: f32 = 11.0;
const HEADER: f32 = 22.0;
const FIELD: f32 = 30.0;
const CHOICES: f32 = 28.0;
const CAPTION: f32 = 16.0;
const LABEL: f32 = 84.0;
const LABEL_GAP: f32 = 10.0;
/// The due field's calendar button.
const BUTTON: f32 = 30.0;
const BUTTON_GAP: f32 = 6.0;

/// The status's name as its button says it.
#[cfg_attr(not(feature = "smoke-test"), allow(dead_code))]
pub(super) fn status_name(status: StatusType) -> &'static str {
    STATUSES
        .iter()
        .find(|row| row.kind == status)
        .map_or("Open", |row| row.name)
}

/// The status a character typed on the status row chooses: the box's
/// characters, as in the status menu.
fn status_key(key: &str) -> Option<StatusType> {
    Some(match key {
        " " => StatusType::Open,
        "/" => StatusType::Doing,
        "x" | "X" => StatusType::Done,
        "-" => StatusType::Cancelled,
        _ => return None,
    })
}

/// The priority a character typed on the priority row chooses: the
/// priority menu's letters, Shift for Highest and Lowest.
fn priority_key(key: &str) -> Option<Option<Priority>> {
    Some(match key {
        "h" => Some(Priority::High),
        "m" | "M" => Some(Priority::Medium),
        "l" => Some(Priority::Low),
        "H" => Some(Priority::Highest),
        "L" => Some(Priority::Lowest),
        "n" | "N" => None,
        _ => return None,
    })
}

/// What the due field's line under it says: what the field names, or that
/// there is no due date.
pub(super) fn due_caption(form: &Form) -> (String, bool) {
    if form.due().trim().is_empty() {
        return ("No due date".to_owned(), false);
    }
    let mut model = Calendar::new(form.today(), None);
    model.set_typed(form.due());
    let caption = model.caption();
    let invalid = caption.is_invalid();
    let text = match caption {
        Caption::Date { date, .. } => format!(
            "{} · {}",
            calendar::long(date),
            calendar::relative(form.today(), date)
        ),
        caption => caption.to_string(),
    };
    (text, invalid)
}

impl App {
    /// Whether the form can open at the caret: tasks on, a Markdown note
    /// in the editor, and the caret on a task, or on a line a task can go
    /// on. What it finds is kept until the text, the caret's line or the
    /// engine changes, as the line commands keep theirs.
    pub(super) fn task_form_can_open(&self) -> bool {
        if !self.task_lines_editable() {
            return false;
        }
        let key = self.memo_key();
        if let Some((kept, found)) = self.task_memo.form_at.get() {
            if kept == key {
                return found;
            }
        }
        let found = self.task_form_at(key.line).is_some();
        self.task_memo.form_at.set(Some((key, found)));
        found
    }

    /// The form for line `line` of the editor's text.
    fn task_form_at(&self, line: usize) -> Option<Form> {
        self.editor
            .read_markdown(|lines, options| Form::open(lines, line, options))
    }

    /// Whether the editor's request of a form may take the keyboard: tasks
    /// on, and a Markdown note in the editor.
    pub(super) fn task_form_takes_keys(&self) -> bool {
        self.preferences.tasks && self.markdown_in_editor() && self.ensure_editor_ready().is_ok()
    }

    /// `task.edit` from the palette or the harness: open the form at the
    /// caret's line.
    pub(super) fn run_task_form(&mut self) -> Gateway {
        self.palette = None;
        let line = self.editor.cursor().position.line;
        match self.open_task_form(line) {
            Some(task) => self.finish_action(Disposition::Completed, None, task),
            None => self.blocked("A task cannot go on this line"),
        }
    }

    /// A request from the editor for the form, `task.edit`'s key
    /// ([`wren_editor::KeyRoute::Popover`]): open it at the caret's line.
    /// `None` for a request of something else.
    pub(super) fn request_task_form(&mut self, id: u32) -> Option<Task<Message>> {
        if id != CommandId::TaskEdit.payload() {
            return None;
        }
        self.column = super::columns::Column::Editor;
        self.editor.close_popover();
        let line = self.editor.cursor().position.line;
        Some(
            self.open_task_form(line)
                .unwrap_or_else(|| self.blocked("A task cannot go on this line").task),
        )
    }

    /// Open the form for line `line` and give its description field the
    /// keyboard. `None` where no task is or can go.
    pub(super) fn open_task_form(&mut self, line: usize) -> Option<Task<Message>> {
        if !self.task_form_takes_keys() || self.dialog.is_some() {
            return None;
        }
        let form = self.task_form_at(line)?;
        self.editor.close_popover();
        self.palette = None;
        // The calendar over the note closes for it, writing nothing.
        if self.calendar.take().is_some_and(|open| open.composing()) {
            self.editor.hand_back();
        }
        self.column = super::columns::Column::Editor;
        let place = format!(
            "{} · line {}",
            self.task_form_note(self.selected_note),
            form.line() + 1
        );
        let target = Target::Editor {
            note: self.selected_note,
            revision: self.editor.revision(),
            engine: self.editor.backend(),
        };
        Some(self.show_task_form(form, target, place))
    }

    /// Open the form for the task of a row of the task view, read from the
    /// row's line with its box at byte `boxed`, where the task index found
    /// it, and give its description field the keyboard. `None` with no task
    /// view showing, or where there is no task there. Save plans the fields
    /// changed on the note's own line.
    pub(super) fn open_row_task_form(
        &mut self,
        target: RowTarget,
        boxed: usize,
    ) -> Option<Task<Message>> {
        if !self.preferences.tasks || self.dialog.is_some() || self.surface() != Surface::Tasks {
            return None;
        }
        let task = super::task_menus::row_task(&target.expect, boxed)?.task();
        let form = Form::edit_task(target.line, &target.expect, task, self.today())?;
        self.palette = None;
        // A calendar over the row closes for it, writing nothing.
        self.calendar = None;
        self.column = super::columns::Column::Editor;
        let place = format!(
            "{} · line {}",
            self.task_form_note(Some(target.note)),
            target.line + 1
        );
        Some(self.show_task_form(form, Target::Row(target), place))
    }

    /// Show `form` for `target`, and give its description field the
    /// keyboard.
    fn show_task_form(&mut self, form: Form, target: Target, place: String) -> Task<Message> {
        // Claims a form before it left and the app never heard of are no
        // one's now.
        FORM_CLAIMS.store(0, Ordering::Relaxed);
        self.task_form = Some(Open {
            form,
            target,
            place,
            focus: Slot::Description,
            due_handback: false,
            composing: false,
            calendar_composing: false,
            keep_owner: false,
            suggestion: None,
            calendar: None,
            error: None,
            suggested: RefCell::new(None),
            #[cfg(test)]
            found: std::cell::Cell::new(0),
        });
        iced::widget::operation::focus(DESCRIPTION)
    }

    /// The note the form is for, as its header names it.
    fn task_form_note(&self, note: Option<NoteId>) -> String {
        let note = note.and_then(|note| {
            self.model()?
                .notes
                .iter()
                .find(|each| each.id == note)
                .cloned()
        });
        let Some(note) = note else {
            return "Editor".to_owned();
        };
        let mut parts: Vec<String> = note
            .path
            .as_path()
            .parent()
            .into_iter()
            .flat_map(|parent| parent.components())
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        parts.push(note.title);
        parts.join(" › ")
    }

    /// Close the form, whatever the reason. An input method composing for
    /// one of its fields, or its calendar's, may still commit: the editor
    /// then drops what one commits until it sees input of its own. With
    /// none composing, what one commits next is the note's.
    fn take_task_form(&mut self) -> Option<Open> {
        let open = self.task_form.take()?;
        // A row's form was never over the note's text.
        let editor = matches!(open.target, Target::Editor { .. });
        if editor && (open.composing || open.calendar_composing || open.due_handback) {
            self.editor.hand_back();
        }
        Some(open)
    }

    /// Close the form, writing nothing, and give the editor back the
    /// keyboard.
    pub(super) fn cancel_task_form(&mut self) -> Task<Message> {
        if self.take_task_form().is_none() {
            return Task::none();
        }
        self.return_to_editor()
    }

    /// A shortcut the form took, as the app's key handling hears it:
    /// pressed for the form, it runs nothing but Quit.
    pub(super) fn form_shortcut(&mut self, accel: &wren_core::keybind::Accel) -> Task<Message> {
        let id = self
            .preferences
            .keymap
            .action(accel)
            .and_then(CommandId::parse);
        if id == Some(CommandId::Quit) {
            self.run_shortcut(CommandId::Quit)
        } else {
            Task::none()
        }
    }

    /// Escape that reached the app with no field of the form's taking it:
    /// the calendar over the form closes, or else the form.
    pub(super) fn escape_task_form(&mut self) -> Task<Message> {
        match &mut self.task_form {
            Some(open) if open.calendar.is_some() => {
                open.calendar = None;
                open.due_handback = open.calendar_composing;
                Slot::Due.focus()
            }
            Some(_) => self.cancel_task_form(),
            None => Task::none(),
        }
    }

    pub(super) fn task_form_event(&mut self, event: Event) -> Task<Message> {
        // A plain Escape the form or its calendar took is claimed whether
        // or not the form is still open: the app's key handling hears of
        // the same press.
        if matches!(
            event,
            Event::Key(FormKey::Escape | FormKey::Clear | FormKey::HeldEscape)
                | Event::Calendar(CalendarEvent::Field(
                    FieldEvent::Escape | FieldEvent::HeldEscape
                ))
        ) {
            self.form_escapes += 1;
        }
        if self.task_form.is_none() {
            return Task::none();
        }
        match event {
            Event::Key(key) => self.task_form_key(key),
            Event::OwnerKey(key, generation) => {
                // The owners changed since the form was drawn: a key that
                // takes one of them takes nothing, and the name typed stays.
                let open = self.task_form.as_mut().expect("open");
                let takes = key == FormKey::Complete || open.suggestion.is_some();
                if takes && generation != self.owners.generation() {
                    open.keep_owner = true;
                    return Task::none();
                }
                self.task_form_key(key)
            }
            Event::Save => self.save_task_form(),
            Event::Cancel => self.cancel_task_form(),
            Event::Calendar(event) => self.task_form_calendar(event),
            Event::OpenCalendar => self.open_form_calendar(),
            event => {
                let open = self.task_form.as_mut().expect("open");
                open.error = None;
                match event {
                    Event::Description(text) => open.form.set_description(&text),
                    Event::Due(text) => open.form.set_due(&text),
                    Event::Owner(text) => {
                        if std::mem::take(&mut open.keep_owner) && text.is_empty() {
                            return Task::none();
                        }
                        open.form.set_pending_owner(&text);
                        open.suggestion = None;
                    }
                    // A choice clicked takes the keyboard from the owners
                    // field, as Tab does, and the owner chosen is forgotten.
                    Event::Status(status) => {
                        open.form.set_status(status);
                        open.focus = Slot::Status;
                        open.suggestion = None;
                    }
                    Event::Priority(priority) => {
                        open.form.set_priority(priority);
                        open.focus = Slot::Priority;
                        open.suggestion = None;
                    }
                    Event::AddOwner(name) => {
                        open.form.add_owner(&name);
                        open.form.set_pending_owner("");
                        open.suggestion = None;
                        open.focus = Slot::Owners;
                        return Slot::Owners.focus();
                    }
                    // The owners offered change: the owner chosen is
                    // forgotten, as the form's keys forget it.
                    Event::RemoveOwner(name) => {
                        open.form.remove_owner(&name);
                        open.suggestion = None;
                    }
                    _ => {}
                }
                Task::none()
            }
        }
    }

    fn task_form_key(&mut self, key: FormKey) -> Task<Message> {
        let suggestions = self.owner_suggestions();
        let open = self.task_form.as_mut().expect("open");
        match key {
            FormKey::Focus(index) => {
                if let Some(slot) = Slot::ALL.get(index) {
                    if open.focus == Slot::Owners && *slot != Slot::Owners {
                        open.suggestion = None;
                    }
                    open.focus = *slot;
                }
            }
            FormKey::Save => return self.save_task_form(),
            FormKey::Escape => return self.cancel_task_form(),
            FormKey::Clear => {
                open.form.set_pending_owner("");
                open.suggestion = None;
            }
            FormKey::HeldEscape => {}
            FormKey::Press(index) => {
                return match Slot::ALL.get(index) {
                    Some(Slot::Save) => self.save_task_form(),
                    Some(Slot::Cancel) => self.cancel_task_form(),
                    _ => Task::none(),
                };
            }
            FormKey::Accept | FormKey::Complete => {
                // Enter adds the name as typed, or the owner chosen with Up
                // and Down; Tab the owner chosen, or else the one offered
                // first, as Tab takes a suggestion in the note.
                let chosen = match (&open.suggestion, key) {
                    // The owner chosen has gone from the list: nothing is
                    // taken, and the name typed stays for another try.
                    (Some(_), _) if chosen_at(open, &suggestions).is_none() => {
                        open.suggestion = None;
                        open.keep_owner = true;
                        return Task::none();
                    }
                    (Some(_), _) => chosen_at(open, &suggestions),
                    (None, FormKey::Complete) => Some(0),
                    (None, _) => None,
                };
                let name = match chosen.and_then(|at| suggestions.get(at)) {
                    Some(name) => name.clone(),
                    None => open.form.pending_owner().trim().to_owned(),
                };
                open.form.add_owner(&name);
                // The field empties as the form's keys empty it.
                open.form.set_pending_owner("");
                open.suggestion = None;
                open.error = None;
            }
            // The due field's own input: it takes what an input method
            // commits now. A composition the calendar left may still commit
            // once the form has gone, so the editor's guard stays armed.
            FormKey::HandbackOver => open.due_handback = false,
            FormKey::Composing(composing) => open.composing = composing,
            FormKey::Step(by) => match open.focus {
                Slot::Status => {
                    let at = STATUSES
                        .iter()
                        .position(|row| row.kind == open.form.status())
                        .unwrap_or(0);
                    let next = stepped(at, STATUSES.len(), by);
                    open.form.set_status(STATUSES[next].kind);
                }
                Slot::Priority => {
                    let at = PRIORITIES
                        .iter()
                        .position(|level| *level == open.form.priority())
                        .unwrap_or(PRIORITIES.len() - 1);
                    let next = stepped(at, PRIORITIES.len(), by);
                    open.form.set_priority(PRIORITIES[next]);
                }
                _ => {}
            },
            FormKey::Pick(key) => match open.focus {
                Slot::Status => {
                    if let Some(status) = status_key(&key) {
                        open.form.set_status(status);
                    }
                }
                Slot::Priority => {
                    if let Some(priority) = priority_key(&key) {
                        open.form.set_priority(priority);
                    }
                }
                _ => {}
            },
            FormKey::Suggest(by) => {
                let count = suggestions.len();
                let at = match (chosen_at(open, &suggestions), count) {
                    (_, 0) => None,
                    (None, _) if by > 0 => Some(0),
                    (None, _) => Some(count - 1),
                    (Some(at), _) => Some(stepped(at, count, by)),
                };
                open.suggestion = at.map(|at| task::fold_name(&suggestions[at]));
            }
            FormKey::Calendar => return self.open_form_calendar(),
            FormKey::RemoveLast => {
                if let Some(last) = open.form.owners().pop() {
                    open.form.remove_owner(&last);
                }
            }
        }
        Task::none()
    }

    /// The owners the form suggests for what the owners field holds: the
    /// workspace's owners whose names start with it, those the task has
    /// left out, the one it names exactly first, as the suggestions in the
    /// note offer them (`task_suggest.rs`). Found again only once the
    /// owners, the name typed or the owners chosen change, and from the
    /// owners' case-folded names: the exact match is looked for among all
    /// of them, the rest only until the list is full.
    pub(super) fn owner_suggestions(&self) -> Vec<String> {
        let Some(open) = &self.task_form else {
            return Vec::new();
        };
        if open.focus != Slot::Owners {
            return Vec::new();
        }
        let typed = open.form.pending_owner().trim();
        let typed = task::fold_name(typed.strip_prefix('@').unwrap_or(typed));
        let have: Vec<String> = open
            .form
            .owners()
            .iter()
            .map(|name| task::fold_name(name))
            .collect();
        let key = (self.owners.generation(), typed, have);
        if let Some((kept, found)) = &*open.suggested.borrow() {
            if *kept == key {
                return found.clone();
            }
        }
        #[cfg(test)]
        open.found.set(open.found.get() + 1);
        let (_, typed, have) = &key;
        let ranked = self.owners.ranked();
        let offered = |owner: &&wren_core::owners::Owner| {
            !have.contains(&owner.key) && owner.key.starts_with(typed.as_str())
        };
        let exact = ranked
            .iter()
            .filter(offered)
            .find(|owner| owner.key == *typed);
        let mut found: Vec<String> = exact.map(|owner| owner.name.clone()).into_iter().collect();
        found.extend(
            ranked
                .iter()
                .filter(offered)
                .filter(|owner| owner.key != *typed)
                .take(SUGGESTIONS - found.len())
                .map(|owner| owner.name.clone()),
        );
        *open.suggested.borrow_mut() = Some((key, found.clone()));
        found
    }

    /// Open the calendar under the due field, on the date the field
    /// names, and give its field the keyboard.
    fn open_form_calendar(&mut self) -> Task<Message> {
        let today = self.today();
        let Some(open) = &mut self.task_form else {
            return Task::none();
        };
        open.calendar = Some(Calendar::new(today, open.form.due_date()));
        open.calendar_composing = false;
        open.focus = Slot::Due;
        open.suggestion = None;
        iced::widget::operation::focus(CALENDAR_FIELD)
    }

    /// Input from the calendar over the form. Whatever closes it gives the
    /// due field back the keyboard.
    fn task_form_calendar(&mut self, event: CalendarEvent) -> Task<Message> {
        let Some(open) = &mut self.task_form else {
            return Task::none();
        };
        let Some(model) = &mut open.calendar else {
            return Task::none();
        };
        let picked = match event {
            CalendarEvent::Field(FieldEvent::Composing(composing)) => {
                open.calendar_composing = composing;
                return Task::none();
            }
            CalendarEvent::Field(FieldEvent::Typed(text)) => {
                model.set_typed(&text);
                return Task::none();
            }
            CalendarEvent::Field(FieldEvent::Key(key)) => {
                model.key(key);
                return Task::none();
            }
            CalendarEvent::Field(FieldEvent::Enter) => match model.enter() {
                Outcome::Write(date) => Some(Some(date)),
                Outcome::Clear => Some(None),
                Outcome::Invalid(_) => return Task::none(),
            },
            CalendarEvent::Pick(date) => Some(Some(date)),
            CalendarEvent::Clear => Some(None),
            CalendarEvent::Show(months) => {
                model.show(months);
                return iced::widget::operation::focus(CALENDAR_FIELD);
            }
            CalendarEvent::Refocus => return iced::widget::operation::focus(CALENDAR_FIELD),
            CalendarEvent::Field(FieldEvent::HeldEscape) | CalendarEvent::FocusChecked(_) => {
                return Task::none();
            }
            CalendarEvent::Field(
                FieldEvent::Escape | FieldEvent::WindowLost | FieldEvent::Blur,
            )
            | CalendarEvent::Outside => None,
        };
        if let Some(due) = picked {
            open.form.pick_due(due);
            open.error = None;
        }
        open.calendar = None;
        open.due_handback = open.calendar_composing;
        open.focus = Slot::Due;
        Slot::Due.focus()
    }

    /// Save: write the fields the user changed as one undo step, close the
    /// form and give the editor back the keyboard. Refused, the form open
    /// with why, once its line no longer reads as it did, and while the due
    /// field names no date.
    fn save_task_form(&mut self) -> Task<Message> {
        // A name left in the owners field is written as typed, as the
        // Writes line shows it.
        let Some(open) = &mut self.task_form else {
            return Task::none();
        };
        if let Err(problem) = open.form.change() {
            let warning = problem.to_string();
            open.error = Some(warning.clone());
            open.focus = Slot::Due;
            let focus = Slot::Due.focus();
            return Task::batch([focus, self.blocked(warning).task]);
        }
        if self.task_form_stale() {
            let warning = "The task changed since the form opened; nothing was written".to_owned();
            if let Some(open) = &mut self.task_form {
                open.error = Some(warning.clone());
            }
            return self.blocked(warning).task;
        }
        if let Some(Target::Row(target)) = self.task_form.as_ref().map(|open| &open.target) {
            let target = target.clone();
            return self.save_row_task_form(&target);
        }
        let open = self.take_task_form().expect("open");
        let focus = self.return_to_editor();
        let started = std::time::Instant::now();
        let written = match open.form.kind() {
            FormKind::Edit => self.write_task_edit(&open.form),
            FormKind::New | FormKind::Make => self.write_new_task(&open.form),
        };
        match written {
            Ok(changed) => {
                if changed {
                    self.text_changed(started);
                }
                let _ = self.finish_action(Disposition::Completed, None, Task::none());
                focus
            }
            Err(warning) => Task::batch([focus, self.blocked(warning).task]),
        }
    }

    /// Save over a row of the task view: the fields changed, written to
    /// the row's task through [`App::write_task_row`]. Refused, the form
    /// stays open with why.
    fn save_row_task_form(&mut self, target: &RowTarget) -> Task<Message> {
        let Some(open) = &self.task_form else {
            return Task::none();
        };
        let change = match open.form.change() {
            Ok(change) => change,
            Err(problem) => return self.blocked(problem.to_string()).task,
        };
        let written = self.write_view_row(target, None, change, super::task_rows::What::Saved);
        if let Err(refusal) = &written.result {
            if let Some(open) = &mut self.task_form {
                open.error = Some(refusal.message.clone());
            }
            return written.gateway.task;
        }
        self.take_task_form();
        let focus = self.return_to_editor();
        Task::batch([focus, written.gateway.task])
    }

    /// Whether what the form was opened for has gone, or the note's text
    /// changed at all: a line that reads as it did may be another task's
    /// (the first of two equal lines deleted), so no edit is mapped. Over
    /// a row, whether the task view has gone: a row's task that changed is
    /// the write's to refuse.
    fn task_form_stale(&self) -> bool {
        let Some(open) = &self.task_form else {
            return true;
        };
        match &open.target {
            Target::Editor {
                note,
                revision,
                engine,
            } => {
                !self.task_form_takes_keys()
                    || self.selected_note != *note
                    || self.editor.backend() != *engine
                    || self.editor.revision() != *revision
                    || !self
                        .editor
                        .read_markdown(|lines, options| open.form.holds(lines, options))
            }
            Target::Row(_) => !self.task_form_over_rows(),
        }
    }

    /// Whether a form over a row of the task view still has its view.
    fn task_form_over_rows(&self) -> bool {
        self.preferences.tasks && self.loading.is_none() && self.surface() == Surface::Tasks
    }

    /// Write the form's change to its task as one line edit.
    fn write_task_edit(&mut self, form: &Form) -> Result<bool, String> {
        let change = form.change().map_err(|problem| problem.to_string())?;
        // A stamp is the day the form's preview showed, though the editor
        // has not yet heard the day changed.
        let today = form.today();
        let edit = self.editor.plan_markdown(|lines, selection, options| {
            let options = wren_markdown::Options {
                tasks: options.tasks.map(|mut tasks| {
                    tasks.today = today;
                    tasks
                }),
                ..*options
            };
            task::edit(lines, form.line(), selection, &change, &options)
        });
        let Some(edit) =
            edit.filter(|edit| edit.line == form.line() && edit.expect == form.expect())
        else {
            return Err("The task changed since the form opened; nothing was written".into());
        };
        self.editor
            .apply_line_edits(&[edit])
            .map_err(|refused| refused.to_string())
    }

    /// Write the form's line in place of the line that is no task yet, as
    /// one replacement, the caret at its end.
    fn write_new_task(&mut self, form: &Form) -> Result<bool, String> {
        let written = form.written().map_err(|problem| problem.to_string())?;
        let line = form.line();
        let end = form.expect().len();
        let replace = wren_editor::Replace {
            start: iced::widget::text_editor::Position { line, column: 0 },
            end: iced::widget::text_editor::Position { line, column: end },
            text: written.clone(),
        };
        let caret = iced::widget::text_editor::Cursor {
            position: iced::widget::text_editor::Position {
                line,
                column: written.len(),
            },
            selection: None,
        };
        Ok(self.editor.apply(&[replace], caret))
    }

    /// Close the form once what it was opened for has gone (another note,
    /// a workspace loading, the note no longer in the editor), or the
    /// palette or a dialog took the keyboard; and read its dates on a new
    /// day. After every message. A change to the task's line leaves it
    /// open: saving says so.
    pub(super) fn sync_task_form(&mut self) -> Task<Message> {
        let today = self.today();
        let takes_keys = self.task_form_takes_keys();
        let over_rows = self.task_form_over_rows();
        let note = self.selected_note;
        let taken = self.palette.is_some() || self.dialog.is_some();
        let engine = self.editor.backend();
        let Some(open) = &mut self.task_form else {
            return Task::none();
        };
        open.form.set_today(today);
        if let Some(model) = &mut open.calendar {
            model.set_today(today);
        }
        let held = match &open.target {
            // Another engine is another editor: what the form read is gone.
            Target::Editor {
                note: on,
                engine: shown,
                ..
            } => takes_keys && *on == note && *shown == engine,
            Target::Row(_) => over_rows,
        };
        if held && !taken {
            return Task::none();
        }
        self.take_task_form();
        if taken {
            Task::none()
        } else {
            self.column_focus()
        }
    }

    /// The form's slots for its keys, in Tab's order.
    fn key_slots() -> Vec<KeySlot> {
        Slot::ALL
            .iter()
            .map(|slot| KeySlot {
                kind: slot.kind(),
                id: slot.id().map(iced::advanced::widget::Id::new),
            })
            .collect()
    }

    /// Where the form's card is in the window.
    pub(super) fn task_form_bounds(&self) -> Option<Rectangle> {
        self.task_form.as_ref()?;
        let width = WIDTH.min((self.viewport.width - 32.0).max(120.0));
        let x = ((self.viewport.width - width) / 2.0).max(0.0);
        let top = super::palette::palette_top(self.viewport.height);
        Some(Rectangle::new(Point::new(x, top), Size::new(width, 0.0)))
    }

    /// Where the due field is in the window: what the calendar hangs
    /// under.
    pub(super) fn task_form_due_bounds(&self) -> Option<Rectangle> {
        let card = self.task_form_bounds()?;
        let x = card.x + PAD_X + LABEL + LABEL_GAP;
        let y = card.y + PAD_Y + HEADER + GAP + FIELD + GAP + CHOICES + GAP;
        let width = card.width - 2.0 * PAD_X - LABEL - LABEL_GAP - BUTTON_GAP - BUTTON;
        Some(Rectangle::new(Point::new(x, y), Size::new(width, FIELD)))
    }

    /// Where the calendar over the form is in the window.
    pub(super) fn task_form_calendar_bounds(&self) -> Option<Rectangle> {
        let open = self.task_form.as_ref()?;
        open.calendar.as_ref()?;
        let anchor = self.task_form_due_bounds()?;
        Some(Rectangle::new(
            place(anchor, self.viewport),
            Size::new(super::calendar::WIDTH, super::calendar::HEIGHT),
        ))
    }

    /// The layers over the window while the form is open: a scrim that
    /// cancels it on a press, and the form, with the calendar over it while
    /// that is open. The same layers either way, so the form's fields keep
    /// their state as the calendar opens and closes.
    pub(super) fn task_form_layers<'a>(
        &'a self,
        open: &'a Open,
    ) -> (Element<'a, Message>, Element<'a, Message>) {
        let definition = self.definition;
        let scrim = mouse_area(
            container(iced::widget::space())
                .width(Fill)
                .height(Fill)
                .style(move |_| chrome::scrim(definition)),
        )
        .on_press(Message::TaskForm(Event::Cancel));
        let top = super::palette::palette_top(self.viewport.height);
        let card = container(opaque(self.task_form_card(open)))
            .padding(iced::Padding {
                top,
                right: 16.0,
                bottom: 8.0,
                left: 16.0,
            })
            .width(Fill)
            .height(Fill)
            .align_x(iced::alignment::Horizontal::Center);
        let calendar = open.calendar.as_ref().zip(self.task_form_calendar_bounds());
        let (outside, over): (Element<'a, Message>, Element<'a, Message>) = match calendar {
            Some((model, bounds)) => {
                let send = |event| Message::TaskForm(Event::Calendar(event));
                let outside = mouse_area(container(iced::widget::space()).width(Fill).height(Fill))
                    .on_press(send(CalendarEvent::Outside))
                    .on_right_press(send(CalendarEvent::Outside))
                    .on_middle_press(send(CalendarEvent::Outside))
                    .on_scroll(move |_| send(CalendarEvent::Outside));
                let has_due = !open.form.due().trim().is_empty();
                let card = mouse_area(self.calendar_card(model, has_due, |event| {
                    Message::TaskForm(Event::Calendar(event))
                }))
                .on_press(send(CalendarEvent::Refocus))
                .on_scroll(move |_| send(CalendarEvent::Outside));
                (
                    opaque(outside),
                    pin(opaque(card)).x(bounds.x).y(bounds.y).into(),
                )
            }
            None => (
                iced::widget::Space::new().into(),
                iced::widget::Space::new().into(),
            ),
        };
        (opaque(scrim), stack![card, outside, over].into())
    }

    fn task_form_card<'a>(&'a self, open: &'a Open) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let form = &open.form;
        let send = |event| Message::TaskForm(event);
        let focused = |slot: Slot| open.focus == slot && open.calendar.is_none();
        let label = |name: &'static str, height: f32| {
            container(chrome::label(name, 13, colors.muted))
                .width(LABEL)
                .height(height)
                .align_y(iced::alignment::Vertical::Center)
        };
        let input = |placeholder: &'a str, value: &'a str, id: &'static str| {
            text_input(placeholder, value)
                .id(id)
                .font(fonts::UI)
                .size(13)
                .padding([6, 8])
                .width(Fill)
                .style(move |_, status| chrome::input(definition, status, true))
        };
        let keys = &self.preferences.field_keys;
        let header = row![
            text(open.title())
                .size(15)
                .font(fonts::SEMIBOLD)
                .color(theme::color(colors.text)),
            crate::ellipsis::ellipsis(open.place.as_str(), 12.0, theme::color(colors.faint)),
        ]
        .spacing(10)
        .height(HEADER)
        .align_y(iced::Center)
        .clip(true);
        let description = row![
            label("Description", FIELD),
            container(crate::field_keys::field_keys(
                input("What needs doing", form.description(), DESCRIPTION)
                    .on_input(move |text| send(Event::Description(text))),
                keys,
            ))
            .height(FIELD)
        ]
        .spacing(LABEL_GAP)
        .height(FIELD);
        let statuses = row(STATUSES.iter().map(|row| {
            let chosen = row.kind == form.status();
            choice(
                definition,
                row![
                    chrome::label(format!("[{}]", row.symbol), 11, colors.faint)
                        .font(fonts::EDITOR),
                    chrome::label(
                        row.name,
                        12,
                        if chosen { colors.accent } else { colors.body }
                    ),
                ]
                .spacing(4)
                .align_y(iced::Center)
                .into(),
                chosen,
                chosen && focused(Slot::Status),
                send(Event::Status(row.kind)),
            )
        }))
        .spacing(4);
        let stamp: Element<'_, Message> = match form.stamp() {
            Some((field, date)) => {
                let short = date.short().to_string();
                let day = short.split_once(' ').map_or(short.as_str(), |(_, day)| day);
                let name = if field == DateField::Done {
                    "Done"
                } else {
                    "Cancelled"
                };
                chrome::label(format!("{name} {day}"), 12, colors.muted).into()
            }
            None => iced::widget::space().into(),
        };
        let status = row![
            label("Status", CHOICES),
            statuses,
            iced::widget::space::horizontal(),
            stamp
        ]
        .spacing(LABEL_GAP)
        .height(CHOICES)
        .align_y(iced::Center);
        let (caption, invalid) = due_caption(form);
        let calendar_button = button(
            container(chrome::label("📅", 14, colors.muted))
                .center_x(BUTTON)
                .center_y(BUTTON),
        )
        .padding(0)
        .width(BUTTON)
        .height(BUTTON)
        .on_press(send(Event::OpenCalendar))
        .style(move |_, status| chrome::control(definition, status, false, false));
        let due = row![
            label("Due", FIELD),
            column![
                row![
                    container(crate::field_keys::field_keys(
                        input("fri, +3d, 2026-12-01", form.due(), DUE)
                            .on_input(move |text| send(Event::Due(text))),
                        keys,
                    ))
                    .height(FIELD)
                    .width(Fill),
                    calendar_button
                ]
                .spacing(BUTTON_GAP)
                .height(FIELD),
                container(chrome::label(
                    caption,
                    12,
                    if invalid { colors.danger } else { colors.muted }
                ))
                .height(CAPTION)
                .padding([0, 2]),
            ]
            .spacing(4)
        ]
        .spacing(LABEL_GAP);
        let priorities = row(PRIORITIES.iter().map(|level| {
            let chosen = *level == form.priority();
            let name = level.map_or("None", Priority::name);
            let mut content = row![].spacing(4).align_y(iced::Center);
            if let Some(level) = level {
                content = content.push(chrome::label(level.glyph().to_string(), 11, colors.body));
            }
            content = content.push(chrome::label(
                name,
                12,
                if chosen { colors.accent } else { colors.body },
            ));
            choice(
                definition,
                content.into(),
                chosen,
                chosen && focused(Slot::Priority),
                send(Event::Priority(*level)),
            )
        }))
        .spacing(4)
        .wrap()
        .vertical_spacing(4);
        let priority = row![label("Priority", CHOICES), priorities].spacing(LABEL_GAP);
        let live = Rc::new(RefCell::new(open.form.pending_owner().to_owned()));
        let owners = self.owners_field(open, Rc::clone(&live));
        let preview = form.preview();
        let writes = column![
            container(iced::widget::space())
                .width(Fill)
                .height(1)
                .style(move |_| chrome::surface(definition, colors.divider, 0.0, false)),
            chrome::label("Writes", 13, colors.muted),
            container(
                text(preview)
                    .font(fonts::EDITOR)
                    .size(12)
                    .color(theme::color(colors.text))
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            )
            .padding([6, 8])
            .width(Fill)
            .style(move |_| chrome::surface(definition, colors.code_block, 6.0, false)),
        ]
        .spacing(6);
        let mut content =
            column![header, description, status, due, priority, owners, writes].spacing(GAP);
        if let Some(error) = &open.error {
            content = content.push(chrome::label(error.as_str(), 12, colors.danger));
        }
        let foot = row![
            chrome::label("Tab moves · ↵ saves · Esc cancels", 12, colors.faint),
            iced::widget::space::horizontal(),
            form_button(
                definition,
                "Cancel",
                false,
                focused(Slot::Cancel),
                send(Event::Cancel)
            ),
            form_button(
                definition,
                "Save",
                true,
                focused(Slot::Save),
                send(Event::Save)
            ),
        ]
        .spacing(8)
        .align_y(iced::Center);
        content = content.push(foot);
        let card = container(content)
            .padding([PAD_Y, PAD_X])
            .width(WIDTH)
            .max_width((self.viewport.width - 32.0).max(120.0))
            .style(move |_| chrome::dialog(definition));
        // The form's own key and the done toggle's save it, taken by its
        // keys so that they wait for the form to show, and for an input
        // method's composition and the calendar to end.
        let keymap = self.preferences.keymap.clone();
        let save: form_keys::SaveKeys = Rc::new(move |key, physical, modifiers| {
            crate::keys::shortcut(key, physical, modifiers)
                .and_then(|accel| keymap.action(&accel).and_then(CommandId::parse))
                .is_some_and(|id| matches!(id, CommandId::TaskEdit | CommandId::ToggleTask))
        });
        // Enter and Tab on the owners carry the owners' generation the form
        // was drawn with: a key queued while the list changed takes nothing.
        let generation = self.owners.generation();
        form_keys::form_keys(
            card,
            Self::key_slots(),
            open.focus.index(),
            live,
            move |key| match key {
                FormKey::Accept | FormKey::Complete => {
                    Message::TaskForm(Event::OwnerKey(key, generation))
                }
                key => Message::TaskForm(Event::Key(key)),
            },
        )
        .save_keys(save)
        .claims(&FORM_CLAIMS)
        // The calendar's button hands it the keyboard, as Down does: an
        // Escape after the click in its batch is no one's.
        .stops_on(|message| matches!(message, Message::TaskForm(Event::OpenCalendar)))
        // A choice no longer offered is held all the same: Enter hands it
        // to the app, which adds nothing for it, and never saves.
        .owner_choice(self.owner_suggestions().len(), open.suggestion.is_some())
        .handback(open.due_handback)
        .passive(open.calendar.is_some())
        .into()
    }

    /// The owners row: each owner as a chip with its remove button, the
    /// field for another, and under it the suggestions.
    fn owners_field<'a>(
        &'a self,
        open: &'a Open,
        live: Rc<RefCell<String>>,
    ) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let send = |event| Message::TaskForm(event);
        let mut chips = row![].spacing(4).align_y(iced::Center);
        for name in open.form.owners() {
            let remove = button(chrome::label("×", 14, colors.faint))
                .padding([0, 5])
                .on_press(send(Event::RemoveOwner(name.clone())))
                .style(move |_, status| chrome::control(definition, status, false, true));
            chips = chips.push(
                container(
                    row![chrome::label(format!("@{name}"), 13, colors.owner), remove]
                        .spacing(2)
                        .align_y(iced::Center),
                )
                .padding(iced::Padding {
                    top: 0.0,
                    right: 2.0,
                    bottom: 0.0,
                    left: 8.0,
                })
                .style(move |_| chrome::surface(definition, colors.raised, 11.0, true)),
            );
        }
        let field = text_input("@name", open.form.pending_owner())
            .id(OWNERS)
            .font(fonts::UI)
            .size(13)
            .padding([4, 3])
            .width(Fill)
            .on_input(move |text: String| {
                live.borrow_mut().clone_from(&text);
                Message::TaskForm(Event::Owner(text))
            })
            .style(move |_, status| chrome::input(definition, status, false));
        let field = crate::field_keys::field_keys(field, &self.preferences.field_keys);
        let focused = open.focus == Slot::Owners && open.calendar.is_none();
        let boxed = container(chips.push(field))
            .padding([2, 5])
            .height(FIELD)
            .width(Fill)
            .align_y(iced::alignment::Vertical::Center)
            .style(move |_| container::Style {
                border: iced::Border {
                    color: theme::color(if focused {
                        colors.accent
                    } else {
                        colors.control_border
                    }),
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..chrome::surface(definition, colors.input, 6.0, false)
            })
            .clip(true);
        let suggestions = self.owner_suggestions();
        let mut offered = row![].spacing(4).align_y(iced::Center);
        for (at, name) in suggestions.iter().enumerate() {
            let chosen = open
                .suggestion
                .as_ref()
                .is_some_and(|key| *key == task::fold_name(name));
            // The one Tab takes, drawn in the accent with no fill.
            let tentative = open.suggestion.is_none()
                && at == 0
                && !open.form.pending_owner().trim().is_empty();
            offered = offered.push(
                button(chrome::label(
                    format!("@{name}"),
                    12,
                    if chosen || tentative {
                        colors.accent
                    } else {
                        colors.owner
                    },
                ))
                .padding([2, 8])
                .on_press(send(Event::AddOwner(name.clone())))
                .style(move |_, status| {
                    let mut style = chrome::control(definition, status, chosen, false);
                    style.border.radius = 11.0.into();
                    style
                }),
            );
        }
        if !suggestions.is_empty() {
            offered = offered.push(iced::widget::space::horizontal());
            offered = offered.push(chrome::label(
                "↑↓ choose · ⇥ takes the highlighted · ↵ adds as typed",
                11,
                colors.faint,
            ));
        }
        row![
            container(chrome::label("Owners", 13, colors.muted))
                .width(LABEL)
                .height(FIELD)
                .align_y(iced::alignment::Vertical::Center),
            if suggestions.is_empty() {
                column![boxed]
            } else {
                column![boxed, container(offered).height(22).clip(true)].spacing(4)
            }
        ]
        .spacing(LABEL_GAP)
        .into()
    }
}

/// Where the owner chosen with Up and Down is among `suggestions`, if it
/// is there still.
pub(super) fn chosen_at(open: &Open, suggestions: &[String]) -> Option<usize> {
    let key = open.suggestion.as_ref()?;
    suggestions
        .iter()
        .position(|name| task::fold_name(name) == *key)
}

/// `at` moved `by` among `count`, round the ends.
fn stepped(at: usize, count: usize, by: i32) -> usize {
    let count = i64::try_from(count).unwrap_or(1).max(1);
    let at = i64::try_from(at).unwrap_or(0);
    usize::try_from((at + i64::from(by)).rem_euclid(count)).unwrap_or(0)
}

/// A choice of a row of choices: the chosen one filled with the accent's
/// tint and edged in it, ringed while its row has the keyboard.
fn choice<'a>(
    definition: wren_core::theme::Theme,
    content: Element<'a, Message>,
    chosen: bool,
    ringed: bool,
    message: Message,
) -> Element<'a, Message> {
    let colors = definition.colors;
    button(container(content).center_y(CHOICES - 2.0))
        .padding([0, 7])
        .height(CHOICES)
        .on_press(message)
        .style(move |_, status| {
            let mut style = chrome::control(definition, status, chosen, false);
            if chosen {
                style.border.color = theme::color(colors.accent);
                style.border.width = if ringed { 2.0 } else { 1.0 };
            }
            style
        })
        .into()
}

/// Save, the accent's, or Cancel, ringed while it has the keyboard.
fn form_button<'a>(
    definition: wren_core::theme::Theme,
    name: &'a str,
    primary: bool,
    ringed: bool,
    message: Message,
) -> Element<'a, Message> {
    let colors = definition.colors;
    let label = chrome::label(name, 13, if primary { colors.editor } else { colors.body })
        .font(fonts::MEDIUM);
    button(label)
        .padding([6, 14])
        .on_press(message)
        .style(move |_, status| {
            let mut style = chrome::control(definition, status, false, false);
            if primary {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                let mut fill = theme::color(colors.accent);
                if hovered {
                    fill.a = 0.88;
                }
                style.background = Some(fill.into());
                style.text_color = theme::color(colors.editor);
                style.border.color = theme::color(colors.accent);
            }
            if ringed {
                style.border.color =
                    theme::color(if primary { colors.text } else { colors.accent });
                style.border.width = 2.0;
            }
            style
        })
        .into()
}

/// What the due field says, for the harness.
#[cfg_attr(not(feature = "smoke-test"), allow(dead_code))]
pub(super) fn due_value_name(form: &Form) -> Option<String> {
    match form.due_value() {
        Ok(Some(Parsed::Date(date))) => Some(date.to_string()),
        Ok(Some(Parsed::Invalid)) => Some("invalid".into()),
        Ok(None) => None,
        Err(problem) => Some(problem.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::Named;
    use iced::keyboard::{Key, Modifiers};
    use iced::widget::text_editor::{Cursor, Position};
    use wren_core::ops::{Op, Session};
    use wren_markdown::date::Date;

    use super::super::commands::{Command, CommandId, Disposition};
    use super::super::tasks::Clock;
    use super::super::Options;
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const NOTE: &str = "# Tasks\n\
        \n\
        - [/] Draft the runbook @alice ⏫ 📅 2026-11-23\n\
        - [ ] Two priorities ⏫ 🔼\n\
        \n\
        Plain line\n\
        > \n\
        ```\n\
        - [ ] fenced\n\
        ```\n";

    /// An app on `engine` with `tasks`, today pinned to Thursday 19
    /// November 2026, and `text` open as a note.
    fn note_app(engine: &str, tasks: &str, text: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("workspace/Work/Tasks");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("List.md"), text).unwrap();
        let config = directory.path().join("config/config.conf");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            format!("editor-engine = {engine}\ntasks = {tasks}\n"),
        )
        .unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.clock = Clock::new(Some(Date::parse_iso("2026-11-19").unwrap()));
        app.apply_preferences();
        app.workspace_mode = true;
        let workspace = directory.path().join("workspace");
        app.session = Some(Session::open_with(&workspace, app.session_options()).unwrap());
        app.sync_editor(true);
        let note = app.model().unwrap().notes[0].id;
        let _ = app.process_core(Op::OpenNote { note: Some(note) });
        assert_eq!(app.editor.text(), text, "{engine}");
        (directory, app)
    }

    fn line_of(text: &str, prefix: &str) -> usize {
        text.split('\n')
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix:?}"))
    }

    fn caret(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    fn send(app: &mut App, event: Event) {
        let _ = app.update(Message::TaskForm(event));
    }

    fn key(app: &mut App, key: FormKey) {
        send(app, Event::Key(key));
    }

    fn open(app: &App) -> &Open {
        app.task_form.as_ref().expect("the form is open")
    }

    /// The form's own key, as the focused editor routes it, carried out as
    /// the app carries out what the editor publishes.
    fn form_key(app: &mut App) -> Vec<wren_editor::Message> {
        let messages = widget_press(app, Key::Named(Named::Enter), form_modifiers());
        // The app's key handling hears of the same press afterwards.
        form_key_copy(app);
        messages
    }

    /// The modifiers of the form's key, ⇧⌘↵ or Ctrl+Shift+Enter.
    fn form_modifiers() -> Modifiers {
        if cfg!(target_os = "macos") {
            Modifiers::LOGO | Modifiers::SHIFT
        } else {
            Modifiers::CTRL | Modifiers::SHIFT
        }
    }

    /// A press into the focused editor: what its widget publishes for it,
    /// carried out in order, without the app's own copy of the press.
    fn widget_press(app: &mut App, key: Key, modifiers: Modifiers) -> Vec<wren_editor::Message> {
        let messages = app.editor.key_message(key, modifiers);
        for message in messages.clone() {
            let _ = app.update(Message::Editor(message));
        }
        messages
    }

    /// The app's key handling hearing of a press of the form's key.
    fn form_key_copy(app: &mut App) {
        let accel = app.preferences.keymap.accel("task.edit").cloned();
        if let Some(accel) = accel.filter(|_| app.preferences.tasks) {
            let _ = app.update(Message::Key(crate::keys::Input::Accel(accel)));
        }
    }

    /// The app's key handling hearing of a plain Escape, which it does after
    /// the widgets' messages of the press's batch.
    fn escape_copy(app: &mut App) {
        let _ = app.update(Message::Key(crate::keys::Input::Nav(
            crate::keys::Nav::Escape,
        )));
    }

    /// The note's text in the editor and in the session's draft.
    fn text(app: &App) -> String {
        let text = app.editor.text();
        let draft = &app.session.as_ref().unwrap().document().unwrap().text;
        assert_eq!(&text, draft, "the editor's text is the draft");
        text
    }

    fn replaced(line: usize, new: &str) -> String {
        let mut lines: Vec<&str> = NOTE.split('\n').collect();
        lines[line] = new;
        lines.join("\n")
    }

    #[test]
    fn the_key_opens_the_form_and_saving_writes_one_undo_step_on_both_engines() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            let at = line_of(NOTE, "- [/] Draft");
            let column = "- [/] Draft".len();
            app.editor.move_cursor(caret(at, column));
            let messages = form_key(&mut app);
            let id = CommandId::TaskEdit.payload();
            assert!(
                matches!(
                    messages.as_slice(),
                    [wren_editor::Message::Spell(wren_editor::SpellEvent::Request(request))]
                        if *request == id
                ),
                "{engine}: {messages:?}"
            );
            let form = &open(&app).form;
            assert_eq!(form.kind(), FormKind::Edit, "{engine}");
            assert_eq!(form.description(), "Draft the runbook @alice");
            assert_eq!(form.due(), "2026-11-23");
            assert_eq!(open(&app).focus, Slot::Description);
            // Every field, then Save.
            send(&mut app, Event::Description("Draft the plan @alice".into()));
            send(&mut app, Event::Status(StatusType::Done));
            send(&mut app, Event::Priority(Some(Priority::Low)));
            send(&mut app, Event::Due("fri".into()));
            send(&mut app, Event::Owner("sam".into()));
            key(&mut app, FormKey::Accept);
            let written = "- [x] Draft the plan @alice @sam 🔽 📅 2026-11-20 ✅ 2026-11-19";
            assert_eq!(open(&app).form.preview(), written, "{engine}");
            assert_eq!(text(&app), NOTE, "{engine}: nothing is written before Save");
            key(&mut app, FormKey::Save);
            assert!(app.task_form.is_none(), "{engine}");
            assert_eq!(text(&app), replaced(at, written), "{engine}");
            // The caret was in the description: it ends the new one.
            assert_eq!(
                app.editor.cursor(),
                caret(at, "- [x] Draft the plan @alice".len()),
                "{engine}"
            );
            // No input method composed in it: what one commits next (the
            // emoji picker's) is the note's.
            assert!(!app.editor.handed_back(), "{engine}");
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            assert_eq!(text(&app), NOTE, "{engine}: one undo step");
            assert!(!app.editor.can_undo(), "{engine}");
        }
    }

    /// Closed while an input method composes in one of its fields, or in
    /// its calendar's, the form leaves the editor dropping what one
    /// commits until its own input; closed with none composing, what one
    /// commits next (the emoji picker's) is the note's.
    #[test]
    fn only_a_close_mid_composition_holds_back_what_an_input_method_commits() {
        type Close = fn(&mut App);
        let closes: [(&str, Close); 3] = [
            ("escape", |app| key(app, FormKey::Escape)),
            ("save", |app| key(app, FormKey::Save)),
            ("outside", |app| send(app, Event::Cancel)),
        ];
        for (name, close) in closes {
            for composing in ["none", "field", "calendar", "calendar closed"] {
                let (_directory, mut app) = note_app("advanced", "on", NOTE);
                app.editor
                    .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
                let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
                match composing {
                    "field" => key(&mut app, FormKey::Composing(true)),
                    "calendar" | "calendar closed" => {
                        send(&mut app, Event::OpenCalendar);
                        send(
                            &mut app,
                            Event::Calendar(CalendarEvent::Field(FieldEvent::Composing(true))),
                        );
                        if composing == "calendar closed" {
                            send(&mut app, Event::Calendar(CalendarEvent::Outside));
                            assert!(open(&app).calendar.is_none());
                        }
                    }
                    _ => {}
                }
                close(&mut app);
                assert!(app.task_form.is_none(), "{name} {composing}");
                assert_eq!(
                    app.editor.handed_back(),
                    composing != "none",
                    "{name} {composing}"
                );
            }
        }
        // The reviewer's sequence: a composition in the calendar, the
        // calendar dismissed, then Escape on the form, whose keys report the
        // due field's own input (the Escape) first. The calendar's late
        // commit is still held back from the note.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        send(&mut app, Event::OpenCalendar);
        send(
            &mut app,
            Event::Calendar(CalendarEvent::Field(FieldEvent::Composing(true))),
        );
        send(&mut app, Event::Calendar(CalendarEvent::Outside));
        key(&mut app, FormKey::HandbackOver);
        key(&mut app, FormKey::Escape);
        assert!(app.task_form.is_none());
        assert!(app.editor.handed_back(), "the late commit is dropped");
        // Compositions that ended before the close hold back nothing.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        key(&mut app, FormKey::Composing(true));
        key(&mut app, FormKey::Composing(false));
        send(&mut app, Event::OpenCalendar);
        let composing = |on| Event::Calendar(CalendarEvent::Field(FieldEvent::Composing(on)));
        send(&mut app, composing(true));
        send(&mut app, composing(false));
        send(&mut app, Event::Calendar(CalendarEvent::Outside));
        key(&mut app, FormKey::Escape);
        assert!(!app.editor.handed_back());
    }

    #[test]
    fn a_form_nobody_changed_writes_nothing_and_escape_cancels() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "- [ ] Two");
        app.editor.move_cursor(caret(at, 3));
        let outcome = app.dispatch(Command::Run(CommandId::TaskEdit));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        // A priority set back to the one the line reads is no change, so
        // the line's two glyphs are not tidied into one.
        send(&mut app, Event::Priority(None));
        send(&mut app, Event::Priority(Some(Priority::Medium)));
        send(&mut app, Event::Status(StatusType::Doing));
        send(&mut app, Event::Status(StatusType::Open));
        key(&mut app, FormKey::Save);
        assert!(app.task_form.is_none());
        assert_eq!(text(&app), NOTE);
        assert!(!app.editor.can_undo());
        assert_eq!(app.editor.cursor(), caret(at, 3));

        // Escape cancels, writing nothing, whatever was typed; the app's key
        // handling then hears the same press and drops it.
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        send(&mut app, Event::Description("Something else".into()));
        key(&mut app, FormKey::Escape);
        assert!(app.task_form.is_none());
        let _ = app.update(Message::Key(crate::keys::Input::Nav(
            crate::keys::Nav::FieldEscape,
        )));
        assert_eq!(app.form_escapes, 0, "the press is claimed once");
        assert_eq!(text(&app), NOTE);
        assert_eq!(app.editor.cursor(), caret(at, 3));
        // So does Cancel, which a press outside it sends too.
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        send(&mut app, Event::Status(StatusType::Done));
        send(&mut app, Event::Cancel);
        assert!(app.task_form.is_none());
        assert_eq!(text(&app), NOTE);
    }

    #[test]
    fn a_blank_or_plain_line_becomes_a_task_on_both_engines() {
        for engine in ["classic", "advanced"] {
            let cases = [
                ("", FormKind::New, "- [ ] Call the bank ⏫"),
                ("Plain line", FormKind::Make, "- [ ] Plain line ⏫"),
                ("> ", FormKind::New, "> - [ ] Call the bank ⏫"),
            ];
            for (prefix, kind, written) in cases {
                let (_directory, mut app) = note_app(engine, "on", NOTE);
                // The blank line after the tasks.
                let at = if prefix.is_empty() {
                    line_of(NOTE, "- [ ] Two") + 1
                } else {
                    line_of(NOTE, prefix)
                };
                app.editor.move_cursor(caret(at, 0));
                let _ = form_key(&mut app);
                assert_eq!(open(&app).form.kind(), kind, "{engine} {prefix:?}");
                assert_eq!(
                    open(&app).title(),
                    if kind == FormKind::New {
                        "New task"
                    } else {
                        "Make a task"
                    }
                );
                if kind == FormKind::New {
                    send(&mut app, Event::Description("Call the bank".into()));
                }
                key(&mut app, FormKey::Pick("h".into()));
                assert_eq!(open(&app).form.priority(), None, "not on the description");
                key(&mut app, FormKey::Focus(Slot::Priority.index()));
                key(&mut app, FormKey::Pick("h".into()));
                assert_eq!(open(&app).form.preview(), written);
                key(&mut app, FormKey::Save);
                assert!(app.task_form.is_none());
                assert_eq!(text(&app), replaced(at, written), "{engine} {prefix:?}");
                assert_eq!(app.editor.cursor(), caret(at, written.len()));
                let _ = app.update(Message::Editor(wren_editor::Message::Undo));
                assert_eq!(text(&app), NOTE, "{engine}: one undo step");
            }
        }
    }

    #[test]
    fn no_form_where_no_task_can_go_nor_with_tasks_off() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        for prefix in ["# Tasks", "- [ ] fenced", "```"] {
            app.editor.move_cursor(caret(line_of(NOTE, prefix), 0));
            assert!(!app.command_enabled(CommandId::TaskEdit), "{prefix}");
            let outcome = app.dispatch(Command::Run(CommandId::TaskEdit));
            assert_eq!(outcome.processed.disposition, Disposition::Blocked);
            let _ = form_key(&mut app);
            assert!(app.task_form.is_none(), "{prefix}");
            assert_eq!(text(&app), NOTE);
        }
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "off", NOTE);
            app.editor
                .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
            assert!(!app.command_enabled(CommandId::TaskEdit));
            let outcome = app.dispatch(Command::Run(CommandId::TaskEdit));
            assert_eq!(outcome.processed.disposition, Disposition::Blocked);
            // Its key is no command's: it does what it did before tasks.
            let messages = form_key(&mut app);
            assert!(
                !messages.iter().any(|message| matches!(
                    message,
                    wren_editor::Message::Spell(wren_editor::SpellEvent::Request(_))
                )),
                "{engine}: {messages:?}"
            );
            assert!(app.task_form.is_none());
        }
    }

    #[test]
    fn saving_is_refused_once_the_line_changed_and_the_form_stays() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            let at = line_of(NOTE, "- [/] Draft");
            app.editor.move_cursor(caret(at, 3));
            let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
            send(&mut app, Event::Status(StatusType::Done));
            // The line changes underneath, as a reload from disk changes it.
            let changed = replaced(at, "- [/] Draft the runbook @alice ⏫");
            app.editor.replace_text(&changed, true);
            key(&mut app, FormKey::Save);
            let form = open(&app);
            assert_eq!(
                form.error.as_deref(),
                Some("The task changed since the form opened; nothing was written"),
                "{engine}"
            );
            assert_eq!(app.editor.text(), changed);
            assert_eq!(
                app.last_processed.as_ref().map(|done| done.disposition),
                Some(Disposition::Blocked)
            );
            // A line moved by a line above it is refused too.
            app.editor.replace_text(&format!("\n{NOTE}"), true);
            key(&mut app, FormKey::Save);
            assert!(app.task_form.is_some(), "{engine}");
            assert_eq!(app.editor.text(), format!("\n{NOTE}"));
            key(&mut app, FormKey::Escape);
            assert!(app.task_form.is_none());
        }
        // A new task's line too.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "Plain line");
        app.editor.move_cursor(caret(at, 0));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        let changed = replaced(at, "# Plain line");
        app.editor.replace_text(&changed, true);
        key(&mut app, FormKey::Save);
        assert!(open(&app).error.is_some());
        assert_eq!(app.editor.text(), changed);
    }

    /// The first of two equal task lines deleted under the form: the
    /// second now reads as the first did, at its place, and is another
    /// task. Save is refused, the second task is left as it was, and the
    /// form keeps what was typed in it.
    #[test]
    fn saving_is_refused_once_an_equal_line_takes_the_tasks_place() {
        let note = "# Tasks\n- [ ] Follow up\n- [ ] Follow up\n";
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", note);
            app.editor.move_cursor(caret(1, 3));
            let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
            send(&mut app, Event::Description("Follow up with Sam".into()));
            // The first is deleted on disk, and the note reloaded.
            let changed = "# Tasks\n- [ ] Follow up\n";
            app.editor.replace_text(changed, true);
            assert_eq!(app.editor.line_text(1).as_deref(), Some("- [ ] Follow up"));
            key(&mut app, FormKey::Save);
            let form = open(&app);
            assert_eq!(
                form.error.as_deref(),
                Some("The task changed since the form opened; nothing was written"),
                "{engine}"
            );
            assert_eq!(form.form.description(), "Follow up with Sam", "{engine}");
            assert_eq!(app.editor.text(), changed, "{engine}");
            // So is an edit elsewhere: no line of the old text is trusted.
            let elsewhere = "# Tasks!\n- [ ] Follow up\n- [ ] Follow up\n";
            app.editor.replace_text(elsewhere, true);
            key(&mut app, FormKey::Save);
            assert!(open(&app).error.is_some(), "{engine}");
            assert_eq!(app.editor.text(), elsewhere, "{engine}");
            key(&mut app, FormKey::Escape);
            assert!(app.task_form.is_none());
            // Opened again on the text as it is now, it saves.
            app.editor.move_cursor(caret(1, 3));
            let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
            send(&mut app, Event::Description("Follow up with Sam".into()));
            key(&mut app, FormKey::Save);
            assert!(app.task_form.is_none(), "{engine}");
            assert_eq!(
                app.editor.text(),
                "# Tasks!\n- [ ] Follow up with Sam\n- [ ] Follow up\n",
                "{engine}"
            );
        }
    }

    #[test]
    fn a_due_field_that_names_no_date_keeps_the_form_open() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "- [/] Draft");
        app.editor.move_cursor(caret(at, 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        send(&mut app, Event::Due("someday".into()));
        assert_eq!(
            due_caption(&open(&app).form),
            ("Not a date. Try fri, +3d or 2026-12-01.".to_owned(), true)
        );
        key(&mut app, FormKey::Save);
        assert_eq!(open(&app).focus, Slot::Due);
        assert_eq!(
            open(&app).error.as_deref(),
            Some("Not a date. Try fri, +3d or 2026-12-01.")
        );
        assert_eq!(text(&app), NOTE);
        send(&mut app, Event::Due("+2w".into()));
        assert_eq!(
            due_caption(&open(&app).form).0,
            "Thursday 3 December 2026 · in 2 weeks"
        );
        assert_eq!(open(&app).error, None, "a change clears it");
        send(&mut app, Event::Due(String::new()));
        assert_eq!(due_caption(&open(&app).form), ("No due date".into(), false));
        key(&mut app, FormKey::Save);
        assert_eq!(
            text(&app),
            replaced(at, "- [/] Draft the runbook @alice ⏫")
        );
    }

    #[test]
    fn the_calendar_in_the_form_gives_the_due_field_back_the_keyboard() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            let at = line_of(NOTE, "- [/] Draft");
            app.editor.move_cursor(caret(at, 3));
            let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
            key(&mut app, FormKey::Focus(Slot::Due.index()));
            key(&mut app, FormKey::Calendar);
            let model = open(&app).calendar.as_ref().expect("the calendar is open");
            assert_eq!(model.selected().to_string(), "2026-11-23", "{engine}");
            assert!(app.task_form_calendar_bounds().is_some());
            // Typing in its field and Enter set the due field, not the note.
            let field = |app: &mut App, event| {
                send(app, Event::Calendar(CalendarEvent::Field(event)));
            };
            field(&mut app, FieldEvent::Typed("fri".into()));
            field(&mut app, FieldEvent::Enter);
            assert!(open(&app).calendar.is_none(), "{engine}");
            assert_eq!(open(&app).focus, Slot::Due);
            assert_eq!(open(&app).form.due(), "2026-11-20");
            assert_eq!(text(&app), NOTE);
            // Escape in the calendar closes it alone, and the app's key
            // handling drops its copy of the press.
            send(&mut app, Event::OpenCalendar);
            field(&mut app, FieldEvent::Typed("+1w".into()));
            field(&mut app, FieldEvent::Escape);
            let _ = app.update(Message::Key(crate::keys::Input::Nav(
                crate::keys::Nav::FieldEscape,
            )));
            assert!(open(&app).calendar.is_none());
            assert_eq!(open(&app).form.due(), "2026-11-20", "Escape writes nothing");
            assert_eq!(open(&app).focus, Slot::Due);
            // A click on a day, Clear, and a press outside the calendar.
            send(&mut app, Event::OpenCalendar);
            send(
                &mut app,
                Event::Calendar(CalendarEvent::Pick(Date::parse_iso("2026-11-25").unwrap())),
            );
            assert_eq!(open(&app).form.due(), "2026-11-25");
            send(&mut app, Event::OpenCalendar);
            send(&mut app, Event::Calendar(CalendarEvent::Clear));
            assert_eq!(open(&app).form.due(), "");
            send(&mut app, Event::OpenCalendar);
            send(&mut app, Event::Calendar(CalendarEvent::Outside));
            assert!(open(&app).calendar.is_none());
            assert!(app.task_form.is_some(), "the form stays");
            key(&mut app, FormKey::Save);
            assert_eq!(
                text(&app),
                replaced(at, "- [/] Draft the runbook @alice ⏫"),
                "{engine}"
            );
        }
    }

    #[test]
    fn the_status_and_priority_rows_take_their_keys() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        key(&mut app, FormKey::Focus(Slot::Status.index()));
        for (pick, status) in [
            ("x", StatusType::Done),
            ("-", StatusType::Cancelled),
            (" ", StatusType::Open),
            ("/", StatusType::Doing),
            ("q", StatusType::Doing),
        ] {
            key(&mut app, FormKey::Pick(pick.into()));
            assert_eq!(open(&app).form.status(), status, "{pick:?}");
        }
        key(&mut app, FormKey::Step(1));
        assert_eq!(open(&app).form.status(), StatusType::Done);
        key(&mut app, FormKey::Step(-1));
        key(&mut app, FormKey::Step(-1));
        key(&mut app, FormKey::Step(-1));
        assert_eq!(
            open(&app).form.status(),
            StatusType::Cancelled,
            "round the ends"
        );
        key(&mut app, FormKey::Focus(Slot::Priority.index()));
        for (pick, priority) in [
            ("H", Some(Priority::Highest)),
            ("L", Some(Priority::Lowest)),
            ("m", Some(Priority::Medium)),
            ("l", Some(Priority::Low)),
            ("n", None),
            ("h", Some(Priority::High)),
        ] {
            key(&mut app, FormKey::Pick(pick.into()));
            assert_eq!(open(&app).form.priority(), priority, "{pick:?}");
        }
        // Right walks the chips in rank order and wraps; Left walks back.
        let ranked = [
            Some(Priority::Highest),
            Some(Priority::High),
            Some(Priority::Medium),
            Some(Priority::Low),
            Some(Priority::Lowest),
            None,
        ];
        key(&mut app, FormKey::Pick("H".into()));
        assert_eq!(open(&app).form.priority(), ranked[0]);
        for (at, expected) in ranked.iter().enumerate().skip(1) {
            key(&mut app, FormKey::Step(1));
            assert_eq!(open(&app).form.priority(), *expected, "right to {at}");
        }
        key(&mut app, FormKey::Step(1));
        assert_eq!(
            open(&app).form.priority(),
            ranked[0],
            "right wraps to Highest"
        );
        key(&mut app, FormKey::Step(-1));
        assert_eq!(open(&app).form.priority(), None, "left wraps to None");
        for (at, expected) in ranked.iter().enumerate().rev().skip(1) {
            key(&mut app, FormKey::Step(-1));
            assert_eq!(open(&app).form.priority(), *expected, "left to {at}");
        }
        // The buttons are pressed by their slot.
        key(&mut app, FormKey::Press(Slot::Cancel.index()));
        assert!(app.task_form.is_none());
        assert_eq!(text(&app), NOTE);
    }

    #[test]
    fn owners_are_added_and_removed_from_their_field() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "- [/] Draft");
        app.editor.move_cursor(caret(at, 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        key(&mut app, FormKey::Focus(Slot::Owners.index()));
        send(&mut app, Event::Owner("@priya".into()));
        key(&mut app, FormKey::Accept);
        assert_eq!(open(&app).form.pending_owner(), "");
        assert_eq!(open(&app).form.owners(), ["alice", "priya"]);
        // Backspace with nothing typed removes the last; the remove button
        // removes any, from the description's text too.
        key(&mut app, FormKey::RemoveLast);
        assert_eq!(open(&app).form.owners(), ["alice"]);
        send(&mut app, Event::RemoveOwner("alice".into()));
        assert_eq!(open(&app).form.description(), "Draft the runbook");
        // Escape with a name typed empties the field and the form stays.
        send(&mut app, Event::Owner("bo".into()));
        key(&mut app, FormKey::Clear);
        assert_eq!(open(&app).form.pending_owner(), "");
        assert!(app.task_form.is_some());
        // A name left in the field is added on Save.
        send(&mut app, Event::Owner("sam".into()));
        key(&mut app, FormKey::Save);
        assert_eq!(
            text(&app),
            replaced(at, "- [/] Draft the runbook @sam ⏫ 📅 2026-11-23")
        );
    }

    /// Down and Enter in the batch whose Tab moved to the owners field:
    /// the form's keys cannot know whether an owner is offered, so Enter
    /// reaches the app as Accept, which adds the owner chosen, or with
    /// none chosen and nothing typed adds nothing. Neither saves.
    /// A choice clicked, or an owner's remove, forgets the owner chosen,
    /// as Tab out of the owners field does: back there, Enter adds none.
    #[test]
    fn a_choice_or_a_remove_clicked_forgets_the_owner_chosen() {
        let clicks: [Event; 3] = [
            Event::Status(StatusType::Doing),
            Event::Priority(Some(Priority::Low)),
            Event::RemoveOwner("nobody".into()),
        ];
        for click in clicks {
            let (_directory, mut app) = note_app("advanced", "on", NOTE);
            let at = line_of(NOTE, "- [ ] Two");
            app.editor.move_cursor(caret(at, 3));
            let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
            key(&mut app, FormKey::Focus(Slot::Owners.index()));
            key(&mut app, FormKey::Suggest(1));
            assert_eq!(open(&app).suggestion.as_deref(), Some("alice"));
            send(&mut app, click.clone());
            assert_eq!(open(&app).suggestion, None, "{click:?}");
            key(&mut app, FormKey::Focus(Slot::Owners.index()));
            let generation = app.owners.generation();
            send(&mut app, Event::OwnerKey(FormKey::Accept, generation));
            assert!(open(&app).form.owners().is_empty(), "{click:?}");
        }
    }

    /// Midnight passed after the timer last looked: the form's preview
    /// shows the new day's stamp, and Save writes that stamp.
    #[test]
    fn a_save_after_midnight_writes_the_stamp_the_preview_showed() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "- [ ] Two");
        app.editor.move_cursor(caret(at, 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        app.clock
            .pass_midnight(Date::parse_iso("2026-11-20").unwrap());
        send(&mut app, Event::Status(StatusType::Done));
        let preview = open(&app).form.preview();
        assert!(preview.ends_with("✅ 2026-11-20"), "{preview}");
        key(&mut app, FormKey::Save);
        assert!(app.task_form.is_none());
        assert_eq!(text(&app), replaced(at, &preview));
    }

    #[test]
    fn accept_adds_the_owner_chosen_or_nothing_and_never_saves() {
        for (down, owners) in [(true, vec!["alice".to_owned()]), (false, vec![])] {
            let (_directory, mut app) = note_app("advanced", "on", NOTE);
            let at = line_of(NOTE, "- [ ] Two");
            app.editor.move_cursor(caret(at, 3));
            let before = text(&app);
            let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
            key(&mut app, FormKey::Focus(Slot::Priority.index()));
            key(&mut app, FormKey::Focus(Slot::Owners.index()));
            assert_eq!(app.owner_suggestions(), ["alice"]);
            if down {
                key(&mut app, FormKey::Suggest(1));
            }
            let generation = app.owners.generation();
            send(&mut app, Event::OwnerKey(FormKey::Accept, generation));
            assert_eq!(open(&app).form.owners(), owners, "down: {down}");
            assert_eq!(text(&app), before, "nothing is written");
        }
    }

    #[test]
    fn owners_are_offered_from_the_workspace_tab_takes_one_and_enter_adds_as_typed() {
        // On the classic engine, with suggestions in the note off: the
        // form offers the workspace's owners all the same.
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("workspace/Work/Tasks");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("List.md"), NOTE).unwrap();
        std::fs::write(
            group.join("Other.md"),
            "- [ ] Plan @samantha\n- [ ] Call @Sam\n- [ ] Ask @sam\n- [ ] Ask @sam\n",
        )
        .unwrap();
        // Both notes changed at one time, so the owners rank by name.
        let at = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for name in ["List.md", "Other.md"] {
            let file = std::fs::File::options()
                .write(true)
                .open(group.join(name))
                .unwrap();
            file.set_modified(at).unwrap();
        }
        let config = directory.path().join("config/config.conf");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            "editor-engine = classic\ntasks = on\ntasks-suggest = off\n",
        )
        .unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.clock = Clock::new(Some(Date::parse_iso("2026-11-19").unwrap()));
        app.apply_preferences();
        app.workspace_mode = true;
        let workspace = directory.path().join("workspace");
        app.session = Some(Session::open_with(&workspace, app.session_options()).unwrap());
        app.sync_editor(true);
        let note = (app.model().unwrap().notes.iter())
            .find(|note| note.title == "List")
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote { note: Some(note) });
        let at = line_of(NOTE, "- [ ] Two");
        app.editor.move_cursor(caret(at, 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        key(&mut app, FormKey::Focus(Slot::Owners.index()));
        // The list the note's suggestions read, in the spelling written
        // most, and none the task has.
        assert_eq!(app.owner_suggestions(), ["alice", "sam", "samantha"]);
        send(&mut app, Event::Owner("SA".into()));
        assert_eq!(app.owner_suggestions(), ["sam", "samantha"]);
        // Enter adds the name as typed and never completes it.
        send(&mut app, Event::Owner("sama".into()));
        key(&mut app, FormKey::Accept);
        assert_eq!(open(&app).form.owners(), ["sama"]);
        // Nor changes its spelling for one the workspace has.
        send(&mut app, Event::Owner("SAM".into()));
        key(&mut app, FormKey::Accept);
        assert_eq!(open(&app).form.owners(), ["sama", "SAM"]);
        // Tab takes the owner offered first.
        send(&mut app, Event::Owner("s".into()));
        assert_eq!(app.owner_suggestions(), ["samantha"]);
        key(&mut app, FormKey::Complete);
        assert_eq!(open(&app).form.owners(), ["sama", "SAM", "samantha"]);
        // Up and Down choose, and Enter takes the one chosen.
        key(&mut app, FormKey::RemoveLast);
        key(&mut app, FormKey::RemoveLast);
        assert_eq!(app.owner_suggestions(), ["alice", "sam", "samantha"]);
        key(&mut app, FormKey::Suggest(1));
        key(&mut app, FormKey::Suggest(1));
        key(&mut app, FormKey::Suggest(1));
        key(&mut app, FormKey::Suggest(1));
        assert_eq!(
            open(&app).suggestion.as_deref(),
            Some("alice"),
            "round the ends"
        );
        key(&mut app, FormKey::Suggest(-1));
        key(&mut app, FormKey::Accept);
        assert_eq!(open(&app).form.owners(), ["sama", "samantha"]);
    }

    #[test]
    fn the_task_menus_edit_row_opens_the_form_for_its_task() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "- [ ] Two");
        app.editor.move_cursor(caret(0, 0));
        let palette = crate::theme::editor_colors(app.definition).1;
        app.editor.lay_out(800.0, palette);
        let column = NOTE.split('\n').nth(at).unwrap().find("Two").unwrap();
        let message = app.editor.right_click_message(at, column).expect("a menu");
        let _ = app.update(Message::Editor(message));
        let labels = app
            .editor
            .popover()
            .expect("the task menu")
            .labels()
            .to_vec();
        assert_eq!(labels.last().map(String::as_str), Some("Edit task…"));
        for message in app
            .editor
            .key_message(Key::Character("e".into()), Modifiers::empty())
        {
            let _ = app.update(Message::Editor(message));
        }
        assert!(app.editor.popover().is_none());
        assert_eq!(open(&app).form.line(), at);
        assert_eq!(open(&app).form.kind(), FormKind::Edit);
        key(&mut app, FormKey::Escape);
        assert_eq!(text(&app), NOTE);
        assert_eq!(app.editor.cursor(), caret(0, 0));
    }

    #[test]
    fn the_form_owns_the_keyboard_while_open() {
        let (_directory, mut app) = note_app("classic", "on", NOTE);
        let at = line_of(NOTE, "- [/] Draft");
        app.editor.move_cursor(caret(at, 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        // The editor's messages are dropped: nothing typed reaches the note.
        let typed = wren_editor::Message::Action(iced::widget::text_editor::Action::Edit(
            iced::widget::text_editor::Edit::Insert('z'),
        ));
        let _ = app.update(Message::Editor(typed));
        assert_eq!(text(&app), NOTE);
        // The app's shortcuts do nothing; the palette's does not open it.
        let _ = app.update(super::super::preferences::shortcut(
            CommandId::CommandPalette,
        ));
        assert!(app.palette.is_none());
        assert!(app.task_form.is_some());
        // Nor do its save keys, as the app's key handling hears them: the
        // form's own keys take them once it shows (`form_keys.rs`), so a
        // press in the batch that opened it, or one an input method or the
        // calendar had, saves nothing.
        send(&mut app, Event::Status(StatusType::Done));
        for action in ["task.edit", "markdown.toggle_task"] {
            let accel = app.preferences.keymap.accel(action).unwrap().clone();
            let _ = app.update(Message::Key(crate::keys::Input::Accel(accel)));
            assert!(app.task_form.is_some(), "{action}");
        }
        send(&mut app, Event::OpenCalendar);
        let accel = app.preferences.keymap.accel("task.edit").unwrap().clone();
        let _ = app.update(Message::Key(crate::keys::Input::Accel(accel)));
        assert!(app.task_form.is_some());
        assert_eq!(text(&app), NOTE);
        // What its keys hand it for one saves.
        send(&mut app, Event::Calendar(CalendarEvent::Outside));
        key(&mut app, FormKey::Save);
        assert!(app.task_form.is_none());
        assert_eq!(
            text(&app),
            replaced(
                at,
                "- [x] Draft the runbook @alice ⏫ 📅 2026-11-23 ✅ 2026-11-19"
            )
        );
    }

    /// The owners suggested are found once for each owners' generation,
    /// name typed and owners chosen, however many times a view reads
    /// them; a long list is read no further than they need.
    #[test]
    fn the_owners_suggested_are_found_once_for_what_they_are_for() {
        let (_directory, mut app) = note_app("classic", "on", NOTE);
        app.editor.move_cursor(caret(line_of(NOTE, "- [ ] Two"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        key(&mut app, FormKey::Focus(Slot::Owners.index()));
        let found = |app: &App| open(app).found.get();
        let _ = app.view();
        let first = app.owner_suggestions();
        assert_eq!(first, ["alice"]);
        let before = found(&app);
        for _ in 0..3 {
            let _ = app.view();
            assert_eq!(app.owner_suggestions(), first);
        }
        assert_eq!(found(&app), before, "kept");
        send(&mut app, Event::Owner("al".into()));
        assert_eq!(app.owner_suggestions(), ["alice"]);
        assert_eq!(found(&app), before + 1, "another name typed");
        key(&mut app, FormKey::Complete);
        assert!(app.owner_suggestions().is_empty());
        assert_eq!(found(&app), before + 2, "another owner chosen");
        // Thousands of owners: the one typed whole first, then the best
        // ranked, six in all.
        let mut many: Vec<wren_core::owners::Owner> = (0..5_000)
            .map(|n| wren_core::owners::Owner {
                name: format!("p{n}"),
                key: format!("p{n}"),
                notes: 1,
                score: 1.0,
            })
            .collect();
        many.push(wren_core::owners::Owner {
            name: "P4999x".into(),
            key: "p4999x".into(),
            notes: 1,
            score: 0.5,
        });
        let sources = vec![wren_core::owners::source(
            NoteId(1_000_000),
            std::time::SystemTime::now(),
            &many
                .iter()
                .map(|owner| format!("- [ ] @{}\n", owner.name))
                .collect::<String>(),
        )];
        // Read straight into the owners, with no message after it, which
        // would bring them back to the index.
        let sources = wren_core::owners::lend(&sources);
        app.owners
            .update(1, 1, &sources, None, std::time::SystemTime::now());
        let typed = |app: &mut App, text: &str| {
            app.task_form.as_mut().unwrap().form.set_pending_owner(text);
        };
        typed(&mut app, "p4999");
        let offered = app.owner_suggestions();
        assert_eq!(offered, ["p4999", "P4999x"]);
        typed(&mut app, "p1");
        assert_eq!(app.owner_suggestions().len(), SUGGESTIONS);
    }

    #[test]
    fn the_batch_that_opens_it_neither_saves_nor_closes_it_on_both_engines() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            let blank = line_of(NOTE, "- [ ] Two") + 1;
            app.editor.move_cursor(caret(blank, 0));
            // The opening key, then the done toggle's in the same batch: the
            // editor drops it, and the app's key handling, which hears it
            // too, takes it for nothing.
            let _ = form_key(&mut app);
            let toggle = app
                .preferences
                .keymap
                .accel("markdown.toggle_task")
                .unwrap()
                .clone();
            let _ = app.update(Message::Key(crate::keys::Input::Accel(toggle)));
            assert!(app.task_form.is_some(), "{engine}");
            assert_eq!(text(&app), NOTE, "{engine}: no empty task");
        }
    }

    fn accel_of(app: &App, action: &str) -> wren_core::keybind::Accel {
        app.preferences.keymap.accel(action).unwrap().clone()
    }

    /// The key press `accel` names, as the window reports it.
    fn press_of(accel: &wren_core::keybind::Accel) -> iced::Event {
        use iced::keyboard::{self, key};
        let key = if accel.key == "enter" {
            Key::Named(Named::Enter)
        } else {
            Key::Character(accel.key.as_str().into())
        };
        let mut modifiers = Modifiers::empty();
        modifiers.set(Modifiers::CTRL, accel.mods.ctrl);
        modifiers.set(Modifiers::ALT, accel.mods.alt);
        modifiers.set(Modifiers::SHIFT, accel.mods.shift);
        modifiers.set(Modifiers::LOGO, accel.mods.logo);
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Code(key::Code::KeyA),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    /// The app's copy of the press `accel` with `status`, the form having
    /// taken `claims` presses the app has yet to hear of.
    fn copy(
        app: &mut App,
        accel: &wren_core::keybind::Accel,
        status: iced::event::Status,
        claims: &AtomicUsize,
    ) {
        let id = iced::window::Id::unique();
        if let Some(message) = copy_of(press_of(accel), status, id, claims) {
            let _ = app.update(message);
        }
    }

    #[test]
    fn a_shortcut_the_closing_form_dropped_runs_nothing_but_quit() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        // Enter saved, and the form's keys took the shortcut after it.
        let claims = AtomicUsize::new(1);
        let jump = accel_of(&app, "navigation.jump");
        key(&mut app, FormKey::Save);
        assert!(app.task_form.is_none());
        copy(&mut app, &jump, iced::event::Status::Captured, &claims);
        assert!(app.palette.is_none(), "the app drops its copy");
        // The next press of it, in the next batch, is the app's again.
        copy(&mut app, &jump, iced::event::Status::Ignored, &claims);
        assert!(app.palette.is_some());
    }

    /// The save chord and then the Jump palette's key, in one batch: the
    /// form's keys take and capture both, and the app's copies of them,
    /// heard before the form's messages or after them, run nothing: the
    /// task is saved once, the form stays closed and Jump does not open.
    /// Jump in the next batch, which the editor does not capture, opens;
    /// and a shortcut another widget captured is never taken for the
    /// form's.
    #[test]
    fn shortcuts_of_the_closing_batch_run_nothing_in_either_order() {
        for copies_first in [false, true] {
            let (_directory, mut app) = note_app("advanced", "on", NOTE);
            let at = line_of(NOTE, "- [/] Draft");
            app.editor.move_cursor(caret(at, 3));
            let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
            send(&mut app, Event::Status(StatusType::Done));
            let save = accel_of(&app, "task.edit");
            let jump = accel_of(&app, "navigation.jump");
            // The form's keys took both presses.
            let claims = AtomicUsize::new(2);
            let captured = iced::event::Status::Captured;
            if copies_first {
                copy(&mut app, &save, captured, &claims);
                copy(&mut app, &jump, captured, &claims);
                assert!(app.task_form.is_some());
            }
            key(&mut app, FormKey::Save);
            assert!(app.task_form.is_none());
            if !copies_first {
                copy(&mut app, &save, captured, &claims);
                assert!(app.task_form.is_none(), "the save chord opens no form");
                copy(&mut app, &jump, captured, &claims);
            }
            assert!(app.palette.is_none(), "Jump does not open");
            assert_eq!(claims.load(Ordering::Relaxed), 0);
            let saved = replaced(
                at,
                "- [x] Draft the runbook @alice ⏫ 📅 2026-11-23 ✅ 2026-11-19",
            );
            assert_eq!(text(&app), saved);
            // The next batch: the editor passes Jump on.
            copy(&mut app, &jump, iced::event::Status::Ignored, &claims);
            assert!(app.palette.is_some(), "a later batch's Jump opens");
            // With no claim owed, a captured shortcut is the app's as before.
            let _ = app.close_palette();
            copy(&mut app, &jump, captured, &claims);
            assert!(app.palette.is_some(), "another widget's capture runs");
        }
    }

    /// The owner chosen with Down is held by name: a refresh that moves
    /// it keeps it chosen, and Enter adds it, not the owner now at its
    /// place; a refresh that drops it chooses none, and Enter then adds
    /// nothing and keeps the name typed.
    #[test]
    fn the_owner_chosen_is_held_by_name_across_a_refresh() {
        let owners = |names: &[&str]| -> Vec<wren_core::owners::Source> {
            names
                .iter()
                .enumerate()
                .map(|(n, name)| {
                    wren_core::owners::source(
                        NoteId(1_000 + n as u64),
                        std::time::SystemTime::now()
                            - std::time::Duration::from_secs(86_400 * n as u64),
                        &format!("- [ ] @{name}\n"),
                    )
                })
                .collect()
        };
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor.move_cursor(caret(line_of(NOTE, "- [ ] Two"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        key(&mut app, FormKey::Focus(Slot::Owners.index()));
        // The form's keys straight, with no message after them, which
        // would bring the owners back to the index.
        let press = |app: &mut App, key: FormKey| {
            let _ = app.task_form_key(key);
        };
        let refresh = |app: &mut App, names: &[&str], revision: u64| {
            let now = std::time::SystemTime::now();
            let sources = owners(names);
            (app.owners).update(1, revision, &wren_core::owners::lend(&sources), None, now);
        };
        refresh(&mut app, &["alice", "bob"], 1);
        app.task_form.as_mut().unwrap().form.set_pending_owner("");
        assert_eq!(app.owner_suggestions(), ["alice", "bob"]);
        press(&mut app, FormKey::Suggest(1));
        assert_eq!(open(&app).suggestion.as_deref(), Some("alice"));
        // Bob takes Alice's place before Enter.
        refresh(&mut app, &["bob", "alice"], 2);
        assert_eq!(app.owner_suggestions(), ["bob", "alice"]);
        assert_eq!(chosen_at(open(&app), &app.owner_suggestions()), Some(1));
        press(&mut app, FormKey::Accept);
        assert_eq!(open(&app).form.owners(), ["alice"]);
        // Chosen, then dropped from the list: nothing is added, the name
        // typed stays, and nothing is chosen.
        app.task_form.as_mut().unwrap().form.set_pending_owner("b");
        press(&mut app, FormKey::Suggest(1));
        assert_eq!(open(&app).suggestion.as_deref(), Some("bob"));
        refresh(&mut app, &["carol"], 3);
        press(&mut app, FormKey::Accept);
        assert_eq!(open(&app).form.owners(), ["alice"]);
        assert_eq!(open(&app).form.pending_owner(), "b");
        assert_eq!(open(&app).suggestion, None);
        // The form's keys emptied the field as they took the key: that is
        // ignored, and the name typed stays; what is typed next is not.
        let _ = app.task_form_event(Event::Owner(String::new()));
        assert_eq!(open(&app).form.pending_owner(), "b");
        let _ = app.task_form_event(Event::Owner(String::new()));
        assert_eq!(open(&app).form.pending_owner(), "");
    }

    /// Tab and Enter on the owners carry the owners' generation the form
    /// was drawn with: queued while a refresh put another owner first,
    /// Tab takes nothing and the name typed stays; on the owners as drawn
    /// it takes the first. Enter with none chosen adds the name as typed
    /// whatever the list.
    #[test]
    fn a_queued_owner_key_on_changed_owners_takes_nothing() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor.move_cursor(caret(line_of(NOTE, "- [ ] Two"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        key(&mut app, FormKey::Focus(Slot::Owners.index()));
        send(&mut app, Event::Owner("al".into()));
        assert_eq!(app.owner_suggestions(), ["alice"]);
        let drawn = app.owners.generation();
        // `@albert` comes first before the queued Tab is handled. Read
        // straight into the owners, with no message after it.
        let now = std::time::SystemTime::now();
        let sources = vec![wren_core::owners::source(
            NoteId(1_000),
            now,
            "- [ ] @albert\n- [ ] @albert again\n",
        )];
        (app.owners).update(1, 1, &wren_core::owners::lend(&sources), None, now);
        assert_ne!(app.owners.generation(), drawn);
        assert_eq!(app.owner_suggestions()[0], "albert");
        let _ = app.task_form_event(Event::OwnerKey(FormKey::Complete, drawn));
        let _ = app.task_form_event(Event::Owner(String::new()));
        assert!(open(&app).form.owners().is_empty(), "nothing taken");
        assert_eq!(open(&app).form.pending_owner(), "al");
        // On the owners as drawn now, Tab takes the first.
        let current = app.owners.generation();
        let _ = app.task_form_event(Event::OwnerKey(FormKey::Complete, current));
        assert_eq!(open(&app).form.owners(), ["albert"]);
        // Enter adds a name as typed, the list changed or not.
        let _ = app.task_form_event(Event::Owner("zed".into()));
        let _ = app.task_form_event(Event::OwnerKey(FormKey::Accept, drawn));
        assert_eq!(open(&app).form.owners(), ["albert", "zed"]);
    }

    /// The calendar over the form closed with no input method composing:
    /// the due field takes what one commits next (an emoji picker's), and
    /// only a close mid-composition makes it drop it.
    #[test]
    fn the_due_field_holds_back_a_commit_only_after_a_composing_calendar() {
        for composing in [false, true] {
            for close in ["escape", "outside"] {
                let (_directory, mut app) = note_app("advanced", "on", NOTE);
                app.editor
                    .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
                let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
                send(&mut app, Event::OpenCalendar);
                if composing {
                    send(
                        &mut app,
                        Event::Calendar(CalendarEvent::Field(FieldEvent::Composing(true))),
                    );
                }
                match close {
                    "escape" => {
                        let _ = app.update(Message::Key(crate::keys::Input::Nav(
                            crate::keys::Nav::FieldEscape,
                        )));
                    }
                    _ => send(&mut app, Event::Calendar(CalendarEvent::Outside)),
                }
                let form = open(&app);
                assert!(form.calendar.is_none(), "{close}");
                assert_eq!(form.due_handback, composing, "{close} {composing}");
            }
        }
    }

    /// A Ctrl, Alt or logo Escape dropped in the batch that opened the
    /// form claims nothing: the app's key handling reads it as a shortcut,
    /// so a later plain Escape still closes the form.
    #[test]
    fn a_modified_escape_in_the_opening_batch_claims_nothing() {
        use iced::keyboard::key::Named as Key;
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            app.claim_handover_escape(Some(Key::Escape), modifiers);
        }
        app.claim_handover_escape(Some(Key::Enter), Modifiers::empty());
        assert_eq!(app.form_escapes, 0);
        let _ = app.update(Message::Key(crate::keys::Input::Nav(
            crate::keys::Nav::Escape,
        )));
        assert!(app.task_form.is_none(), "a plain Escape closes it");
        // A plain one, Shift or not, is claimed.
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        app.claim_handover_escape(Some(Key::Escape), Modifiers::SHIFT);
        assert_eq!(app.form_escapes, 1);
        let _ = app.update(Message::Key(crate::keys::Input::Nav(
            crate::keys::Nav::Escape,
        )));
        assert!(app.task_form.is_some(), "the claimed press closes nothing");
    }

    #[test]
    fn another_engine_closes_it_writing_nothing() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        send(&mut app, Event::Status(StatusType::Done));
        let _ = app.update(Message::Command(Command::Run(CommandId::SwitchEngine)));
        assert_ne!(app.editor.backend(), wren_editor::BackendKind::Owned);
        assert!(app.task_form.is_none());
        assert_eq!(text(&app), NOTE);
    }

    #[test]
    fn another_note_or_the_palette_closes_it_writing_nothing() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        send(&mut app, Event::Status(StatusType::Done));
        let _ = app.update(Message::PaletteOpen(super::super::palette::Mode::Commands));
        assert!(app.task_form.is_none());
        assert_eq!(text(&app), NOTE);
        let _ = app.close_palette();
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        let _ = app.process_core(Op::OpenNote { note: None });
        let _ = app.update(Message::Tick);
        assert!(app.task_form.is_none());
    }

    /// Escape on a task menu, then the form's key, in one batch: the
    /// editor's widget sends the Escape to the menu, which closes it, and
    /// asks for the form; the app's key handling hears of both presses
    /// after that. The Escape was the menu's: the form stays open, and no
    /// claim is left over to swallow a later Escape.
    #[test]
    fn escape_on_a_menu_then_the_forms_key_in_one_batch_leaves_the_form_open() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        app.find_open = true;
        let _ = app.dispatch(Command::Run(CommandId::TaskSetStatus));
        assert!(app.editor.popover().is_some());
        widget_press(&mut app, Key::Named(Named::Escape), Modifiers::empty());
        assert!(app.editor.popover().is_none());
        widget_press(&mut app, Key::Named(Named::Enter), form_modifiers());
        assert!(app.task_form.is_some());
        escape_copy(&mut app);
        form_key_copy(&mut app);
        assert!(app.task_form.is_some(), "the Escape was the menu's");
        assert!(app.find_open);
        // The next Escape is the form's alone, and the one after it the
        // find bar's.
        key(&mut app, FormKey::Escape);
        escape_copy(&mut app);
        assert!(app.task_form.is_none());
        assert!(app.find_open, "one action for one Escape");
        escape_copy(&mut app);
        assert!(!app.find_open, "no claim is left over");
        assert_eq!(text(&app), NOTE);
    }

    /// The command key: ⌘ on macOS, Ctrl on Linux in the tests' keymap.
    fn command_modifier() -> Modifiers {
        if cfg!(target_os = "macos") {
            Modifiers::LOGO
        } else {
            Modifiers::CTRL
        }
    }

    /// The app's copy of a plain Escape a focused field captured.
    fn field_escape_copy(app: &mut App) {
        let _ = app.update(Message::Key(crate::keys::Input::Nav(
            crate::keys::Nav::FieldEscape,
        )));
    }

    /// An event of the calendar's field, the calendar over the note.
    fn calendar_field(app: &mut App, event: FieldEvent) {
        let _ = app.update(Message::Calendar(CalendarEvent::Field(event)));
    }

    /// An event of the calendar's field, the calendar over the form.
    fn form_calendar_field(app: &mut App, event: FieldEvent) {
        send(app, Event::Calendar(CalendarEvent::Field(event)));
    }

    /// No Escape is claimed and not yet heard of.
    fn no_claims(app: &mut App) {
        assert_eq!(app.calendar_escapes, 0);
        assert_eq!(app.form_escapes, 0);
        assert!(!app.editor.take_claimed_escape());
    }

    /// Escape, Escape in one batch on the calendar: its field closes it on
    /// the first and reports the second as no one's; the app hears of both
    /// and drops both, so the find bar is the next batch's.
    #[test]
    fn escape_escape_on_the_calendar_in_one_batch_closes_only_the_calendar() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        app.find_open = true;
        widget_press(&mut app, Key::Character("d".into()), command_modifier());
        assert!(app.calendar.is_some());
        calendar_field(&mut app, FieldEvent::Escape);
        calendar_field(&mut app, FieldEvent::HeldEscape);
        field_escape_copy(&mut app);
        field_escape_copy(&mut app);
        assert!(app.calendar.is_none());
        assert!(app.find_open);
        no_claims(&mut app);
        escape_copy(&mut app);
        assert!(!app.find_open);
        assert_eq!(text(&app), NOTE);
    }

    /// Escape, Escape in one batch on the form, or on its calendar: the
    /// first closes what it was pressed in, the second is no one's, and the
    /// app drops its copies of both.
    #[test]
    fn escape_escape_on_the_form_or_its_calendar_in_one_batch_closes_only_that() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            app.editor
                .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
            app.find_open = true;
            let _ = form_key(&mut app);
            send(&mut app, Event::OpenCalendar);
            assert!(open(&app).calendar.is_some(), "{engine}");
            form_calendar_field(&mut app, FieldEvent::Escape);
            form_calendar_field(&mut app, FieldEvent::HeldEscape);
            field_escape_copy(&mut app);
            field_escape_copy(&mut app);
            assert!(open(&app).calendar.is_none(), "{engine}");
            no_claims(&mut app);
            // The form closes on the first of the next batch's.
            key(&mut app, FormKey::Escape);
            key(&mut app, FormKey::HeldEscape);
            field_escape_copy(&mut app);
            field_escape_copy(&mut app);
            assert!(app.task_form.is_none(), "{engine}");
            assert!(app.find_open, "{engine}");
            no_claims(&mut app);
            escape_copy(&mut app);
            assert!(!app.find_open, "{engine}");
            assert_eq!(text(&app), NOTE, "{engine}");
        }
    }

    /// Escape on a task menu, then the calendar's key, in one batch: the
    /// Escape was the menu's, and the calendar stays open for the next.
    #[test]
    fn escape_on_a_menu_then_the_calendars_key_in_one_batch_leaves_the_calendar_open() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        app.find_open = true;
        let _ = app.dispatch(Command::Run(CommandId::TaskSetPriority));
        assert!(app.editor.popover().is_some());
        widget_press(&mut app, Key::Named(Named::Escape), Modifiers::empty());
        widget_press(&mut app, Key::Character("d".into()), command_modifier());
        assert!(app.calendar.is_some());
        escape_copy(&mut app);
        assert!(app.calendar.is_some(), "the Escape was the menu's");
        no_claims(&mut app);
        calendar_field(&mut app, FieldEvent::Escape);
        field_escape_copy(&mut app);
        assert!(app.calendar.is_none());
        assert!(app.find_open, "one action for one Escape");
        escape_copy(&mut app);
        assert!(!app.find_open);
        assert_eq!(text(&app), NOTE);
    }

    /// One batch of events through the editor's widget, as a window hands
    /// a frame's events to it: what the widget publishes, carried out in
    /// order. The app's copies of the presses are the test's to send.
    fn editor_batch(
        app: &mut App,
        inputs: &[wren_editor::BatchInput],
    ) -> Vec<wren_editor::Message> {
        // What the app tells its sources after every message, as a window's
        // frames would have before the batch.
        let _ = app.update(Message::Tick);
        let (style, palette) = crate::theme::editor_colors(app.definition);
        app.editor.lay_out(800.0, palette);
        let messages = app.editor.batch_messages(inputs, 800.0, style, palette);
        for message in messages.clone() {
            let _ = app.update(Message::Editor(message));
        }
        messages
    }

    fn pressed(key: Key, modifiers: Modifiers) -> wren_editor::BatchInput {
        wren_editor::BatchInput::Key(key, modifiers)
    }

    fn plain_escape() -> wren_editor::BatchInput {
        pressed(Key::Named(Named::Escape), Modifiers::empty())
    }

    /// Whatever opens the form or the calendar from the note, then Escape,
    /// in one batch through the editor's widget: the widget drops the
    /// Escape after the handover as no one's, and the app drops its copy,
    /// so what opened stays open. Off a task, the calendar's key opens
    /// nothing and the Escape is the app's.
    #[test]
    fn an_escape_after_the_handover_in_its_batch_closes_nothing() {
        let draft = line_of(NOTE, "- [/] Draft");
        type Opener = fn(&mut App) -> Vec<wren_editor::BatchInput>;
        let openers: [(&str, &str, Opener); 5] = [
            ("classic", "the form's key", |_| {
                vec![pressed(Key::Named(Named::Enter), form_modifiers())]
            }),
            ("advanced", "the form's key", |_| {
                vec![pressed(Key::Named(Named::Enter), form_modifiers())]
            }),
            ("advanced", "the calendar's key", |_| {
                vec![pressed(Key::Character("d".into()), command_modifier())]
            }),
            ("advanced", "a right-click on the due date", |_| {
                let draft = line_of(NOTE, "- [/] Draft");
                let date = NOTE.split('\n').nth(draft).unwrap().find("2026").unwrap();
                vec![wren_editor::BatchInput::RightClick {
                    line: draft,
                    column: date,
                }]
            }),
            ("advanced", "the task menu's due date row", |app| {
                let draft = line_of(NOTE, "- [/] Draft");
                let (_, palette) = crate::theme::editor_colors(app.definition);
                app.editor.lay_out(800.0, palette);
                let message = app.editor.right_click_message(draft, 7).expect("a menu");
                let _ = app.update(Message::Editor(message));
                assert!(app.editor.popover().is_some(), "the task menu");
                vec![pressed(Key::Character("d".into()), Modifiers::empty())]
            }),
        ];
        for (engine, opener, inputs) in openers {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            app.editor.move_cursor(caret(draft, 3));
            app.find_open = true;
            let mut batch = inputs(&mut app);
            batch.push(plain_escape());
            editor_batch(&mut app, &batch);
            let opened = app.task_form.is_some() || app.calendar.is_some();
            assert!(opened, "{engine}: {opener}");
            if opener == "the form's key" {
                form_key_copy(&mut app);
            }
            escape_copy(&mut app);
            let open = app.task_form.is_some() || app.calendar.is_some();
            assert!(open, "{engine}: {opener} stays open");
            no_claims(&mut app);
            escape_copy(&mut app);
            assert!(
                app.task_form.is_none() && app.calendar.is_none(),
                "{engine}: {opener}"
            );
            assert!(app.find_open, "{engine}: {opener}");
            escape_copy(&mut app);
            assert!(!app.find_open, "{engine}: {opener}");
            assert_eq!(text(&app), NOTE, "{engine}: {opener}");
        }
        // Off a task the calendar's key takes nothing: the Escape after it
        // is the app's, and closes the find bar.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "Plain line"), 2));
        app.find_open = true;
        editor_batch(
            &mut app,
            &[
                pressed(Key::Character("d".into()), command_modifier()),
                plain_escape(),
            ],
        );
        assert!(app.calendar.is_none());
        no_claims(&mut app);
        escape_copy(&mut app);
        assert!(!app.find_open, "the Escape was the app's");
        assert_eq!(text(&app), NOTE);
    }

    /// Escape on a task menu, the form's key, then Escape on the form, in
    /// one batch: two ledgers hold a claim at once, the editor's and the
    /// form's, and each of the app's two copies takes exactly one.
    #[test]
    fn two_claims_in_one_batch_are_each_taken_once() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        app.find_open = true;
        let _ = app.dispatch(Command::Run(CommandId::TaskSetStatus));
        assert!(app.editor.popover().is_some());
        editor_batch(
            &mut app,
            &[
                plain_escape(),
                pressed(Key::Named(Named::Enter), form_modifiers()),
                plain_escape(),
            ],
        );
        assert!(app.task_form.is_some());
        assert_eq!(app.form_escapes, 1, "the Escape after the handover");
        escape_copy(&mut app);
        assert!(app.task_form.is_some(), "the menu's Escape");
        assert_eq!(app.form_escapes, 0, "one claim for one copy");
        form_key_copy(&mut app);
        escape_copy(&mut app);
        assert!(app.task_form.is_some(), "the dropped Escape");
        no_claims(&mut app);
        // The next Escape is the form's, and the one after the find bar's.
        key(&mut app, FormKey::Escape);
        field_escape_copy(&mut app);
        assert!(app.task_form.is_none());
        assert!(app.find_open);
        escape_copy(&mut app);
        assert!(!app.find_open);
        assert_eq!(text(&app), NOTE);
    }

    /// A click on the form's calendar button, then Escape, in one batch:
    /// the form's keys report the Escape as no one's, so the calendar
    /// opened by the click stays open and the form with it.
    #[test]
    fn an_escape_after_the_calendars_button_in_its_batch_closes_nothing() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        let _ = app.dispatch(Command::Run(CommandId::TaskEdit));
        send(&mut app, Event::OpenCalendar);
        key(&mut app, FormKey::HeldEscape);
        field_escape_copy(&mut app);
        assert!(open(&app).calendar.is_some());
        no_claims(&mut app);
        escape_copy(&mut app);
        assert!(
            open(&app).calendar.is_none(),
            "the next Escape is the calendar's"
        );
        assert!(app.task_form.is_some());
        assert_eq!(text(&app), NOTE);
    }

    /// Escape that no widget took closes one layer at a time, innermost
    /// first, writing nothing. The calendar over the note, the form, the
    /// editor's popovers, the palette and the dialogs are never open
    /// together (each closes the others as it opens, or cannot open over
    /// them), so each is stacked here only over the find bar.
    #[test]
    fn escape_closes_one_layer_at_a_time_innermost_first() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [/] Draft"), 3));
        type Open = fn(&mut App) -> bool;
        let layers: [(&str, Open); 5] = [
            ("calendar", |app| {
                widget_press(app, Key::Character("d".into()), command_modifier());
                app.calendar.is_some()
            }),
            ("form", |app| {
                let _ = form_key(app);
                app.task_form.is_some()
            }),
            ("menu", |app| {
                let _ = app.dispatch(Command::Run(CommandId::TaskSetStatus));
                app.editor.popover().is_some()
            }),
            ("palette", |app| {
                let _ = app.dispatch(Command::Run(CommandId::CommandPalette));
                app.palette.is_some()
            }),
            ("dialog", |app| {
                let _ = app.show_dialog(super::super::workspace::DialogKind::CreateSection);
                app.dialog.is_some()
            }),
        ];
        for (name, open_layer) in layers {
            app.find_open = true;
            assert!(open_layer(&mut app), "{name}");
            escape_copy(&mut app);
            assert!(!open_layer_still(&app), "{name}");
            assert!(app.find_open, "{name}: one layer for one Escape");
            escape_copy(&mut app);
            assert!(!app.find_open, "{name}");
            assert_eq!(text(&app), NOTE, "{name}");
        }
        // The calendar over the form closes before the form.
        app.find_open = true;
        let _ = form_key(&mut app);
        send(&mut app, Event::OpenCalendar);
        escape_copy(&mut app);
        assert!(open(&app).calendar.is_none());
        escape_copy(&mut app);
        assert!(app.task_form.is_none());
        assert!(app.find_open);
        no_claims(&mut app);
        assert_eq!(text(&app), NOTE);
    }

    /// Whether any layer Escape closes before the find bar is open.
    fn open_layer_still(app: &App) -> bool {
        app.calendar.is_some()
            || app.task_form.is_some()
            || app.editor.popover().is_some()
            || app.palette.is_some()
            || app.dialog.is_some()
    }
}
