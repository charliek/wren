//! The calendar (discovery/tasks.md §6, "Calendar popover"; item 4.1): a
//! card hung under a task's due date, or the caret, with a field to type a
//! date into, which picks and writes the task's `📅` token. It is drawn by
//! the app over the window, not by the editor, so that its field is iced's
//! own text input (an input method's composition, selection and paste),
//! and the same card can serve the task form and the summary view later.
//!
//! It opens from `task.set_due` (⌘D / Ctrl+D, the palette), on the
//! advanced engine with tasks on, at the caret's task; from a right-click
//! on a due date (a provider that takes the keyboard); from the task
//! menu's Set due date… row; and from the suggestions' "Due date…" for a
//! typed `du` (`task_suggest.rs`), under that word, which the date written
//! takes the place of. Opening gives its field the keyboard.
//!
//! **Input in the frame of a handover is dropped.** The keys after ⌘D in
//! its batch of events were typed before the calendar could be seen, so
//! they reach neither the note nor the field: the editor's widget drops
//! them for a request the app's source of menus answers by taking the
//! keyboard on a task's box line (`TaskMenus`, in `task_menus.rs`), and
//! while the calendar is open the app applies no message of the editor's
//! at all, which covers a request on a line of a task's text after its
//! box, and a menu row chosen with keys after it. Escape and the app's shortcuts stay the app's. Off a task
//! nothing claims the keys, and they reach the note as before; nor inside
//! a fenced block, where a line may read as a task's but is none
//! ([`TaskSource`]). The same holds the other way. A key that closes the
//! calendar is the last its field takes in its batch, as is the window
//! losing the keyboard (`date_field.rs`). And when it closes while an input
//! method composes in its field, the editor then drops what one commits
//! until it sees input of its own ([`wren_editor::EditorView::hand_back`]):
//! a composition begun in the field is never the note's. Closed with none
//! composing, what an input method commits next (the emoji picker's,
//! dictation's) is the note's.
//!
//! It closes, writing nothing and leaving the note's caret and selection
//! as they were, on a plain Escape (Ctrl, Alt or logo with it make it no
//! one's), a press outside it, a scroll over it or outside, the window or
//! the field losing the keyboard (the editor takes it back unless another
//! widget holds it), the palette or a dialog opening, another
//! note, surface or engine, and any change to the note's text (a stale
//! anchor). Enter writes the date the field names, or with nothing typed
//! the day selected, as one undo step through `wren_markdown::task::edit`,
//! refused once the task's line no longer reads as it did; a field that
//! names no date writes nothing and stays open. A click on a day, or on a
//! quick pick, writes that day; Clear removes the due date. Closing gives
//! the editor back the keyboard, except to the palette or a dialog.
//!
//! It also opens for a row of the task view ([`Target::Row`]), hung under
//! the row's due cell, and writes the date through the one path every
//! write from the view takes (`App::write_task_row`, `task_target.rs`),
//! which finds the task where the row has it or refuses. It closes,
//! writing nothing, once the view has gone.

use iced::widget::{
    button, column, container, mouse_area, opaque, pin, rich_text, row, span, stack,
};
use iced::{Element, Fill, Point, Rectangle, Size, Task};
use wren_core::model::NoteId;
use wren_editor::{Anchor, MenuSource, Requested};
use wren_markdown::block;
use wren_markdown::calendar::{Calendar, Day, Outcome, WEEKDAY_HEADS};
use wren_markdown::date::{self, Date, Parsed};
use wren_markdown::span::SpanKind;
use wren_markdown::task::{self, Change};
use wren_markdown::{Pos, Selection};

use super::board::Surface;
use super::commands::{CommandId, Disposition, Gateway};
use super::task_menus::TaskMenus;
use super::task_target::RowTarget;
use super::{App, Message};
use crate::date_field::{self, FieldEvent};
use crate::{chrome, fonts, theme};

/// The field's widget id, which takes the keyboard when the calendar opens.
pub(super) const FIELD: &str = "calendar-field";

/// What the calendar's card hands the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A key, or the keyboard lost, from the field.
    Field(FieldEvent),
    /// A click on a day, or on a quick pick: write it.
    Pick(Date),
    /// Show the month this many months on.
    Show(i32),
    /// Remove the due date.
    Clear,
    /// A press inside the card off its buttons and field: the field keeps
    /// the keyboard.
    Refocus,
    /// A press outside the card, or a scroll anywhere.
    Outside,
    /// Whether a widget held the keyboard after the field lost it.
    FocusChecked(bool),
}

/// The open calendar.
#[derive(Debug, Clone)]
pub(super) struct Open {
    pub(super) model: Calendar,
    /// The task the date is written to.
    target: Target,
    /// The task has a due token, which Clear removes.
    has_due: bool,
    /// An input method is composing in the field, as it last said.
    composing: bool,
}

/// The task the calendar writes its date to.
#[derive(Debug, Clone)]
enum Target {
    /// A task of the note in the editor.
    Editor {
        /// The note it was opened over, and its text's revision then.
        note: Option<NoteId>,
        revision: u64,
        /// The line of the task's box, which the due date is written on,
        /// and its text when the calendar opened.
        line: usize,
        text: String,
        /// Where the card hangs: the due token, or the caret.
        anchor: Anchor,
        /// The anchor's place when the calendar opened, for a frame in
        /// which the editor cannot say.
        opened_at: Option<Rectangle>,
        /// The first line in view when it opened: a scroll closes it.
        top: usize,
        /// A word on the task's line that the date written takes the place
        /// of: the `du` a suggestion opened it for (`task_suggest.rs`).
        /// Cancelled, the word stays.
        erase: Option<std::ops::Range<usize>>,
    },
    /// A row of the task view, and the window's rectangle the card hangs
    /// under: the row's due cell.
    Row(RowTarget, Rectangle),
}

/// What a calendar over a row of the task view hangs on in the note: no
/// place of the editor's.
#[cfg_attr(not(any(test, feature = "smoke-test")), allow(dead_code))]
static NO_ANCHOR: Anchor = Anchor::Caret;

#[cfg_attr(not(any(test, feature = "smoke-test")), allow(dead_code))]
impl Open {
    /// The line of the task's box, in the editor or in the row's note.
    pub(super) fn line(&self) -> usize {
        match &self.target {
            Target::Editor { line, .. } => *line,
            Target::Row(target, _) => target.line,
        }
    }

    /// Where the card hangs in the editor's text; the caret for a row.
    pub(super) fn anchor(&self) -> &Anchor {
        match &self.target {
            Target::Editor { anchor, .. } => anchor,
            Target::Row(..) => &NO_ANCHOR,
        }
    }

    pub(super) fn has_due(&self) -> bool {
        self.has_due
    }
}

impl Open {
    /// Whether an input method is composing in the field, as it last said.
    pub(super) fn composing(&self) -> bool {
        self.composing
    }
}

/// The source of menus the app gives the editor while tasks are on: the
/// task menus' requests, as [`TaskMenus`] answers them, and the calendar's,
/// which takes the keyboard only on the caret's line, and only where the
/// whole note reads it as the line of a task's box. A line that reads as
/// one alone but is no task in the note (inside a fenced block, say)
/// claims nothing, so the keys after the request reach the note.
#[derive(Default)]
pub(super) struct TaskSource {
    /// The caret's line and its text, when its box opens a task in the note,
    /// as the app last found it ([`App::sync_due_target`]).
    due: std::sync::Mutex<Option<(usize, String)>>,
    /// Whether the task form's request takes the keyboard: tasks on and a
    /// Markdown note in the editor, as the app last found it.
    form: std::sync::atomic::AtomicBool,
    /// Whether the editor is on the engine that opens popovers: on the
    /// other the form's request alone is answered.
    advanced: std::sync::atomic::AtomicBool,
}

impl TaskSource {
    fn due(&self) -> std::sync::MutexGuard<'_, Option<(usize, String)>> {
        self.due
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl MenuSource for TaskSource {
    fn menu(&self, id: u32, line: &str, caret: usize) -> Option<Requested> {
        if !self.advanced.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        MenuSource::menu(&TaskMenus, id, line, caret)
    }

    /// The form is a dialog, where it can open: the keys after its request
    /// in their batch, an Escape included, are no one's.
    fn holds_escape(&self, id: u32) -> bool {
        id == CommandId::TaskEdit.payload() && self.form.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The calendar, where it opens, holds an Escape after its request
    /// as the form does; off a task the keys after it are the note's.
    fn holds_escape_when_taken(&self, id: u32) -> bool {
        id == CommandId::TaskSetDue.payload()
    }

    fn menu_on(&self, id: u32, index: usize, line: &str, caret: usize) -> Option<Requested> {
        // The task form opens on any line of a note, or says why not: the
        // keys after its request in their batch are no one's either way.
        if id == CommandId::TaskEdit.payload() {
            let form = self.form.load(std::sync::atomic::Ordering::Relaxed);
            return form.then_some(Requested::Keyboard);
        }
        if id != CommandId::TaskSetDue.payload() {
            return self.menu(id, line, caret);
        }
        let due = self.due();
        let on_box = due
            .as_ref()
            .is_some_and(|(at, text)| *at == index && text == line);
        on_box.then_some(Requested::Keyboard)
    }
}

/// What the caret's task line was last found for: the text's revision, the
/// caret's line, and whether a calendar could open.
pub(super) type DueKey = (u64, usize, bool);

/// The card's width, and the heights its rows are given, which fix its
/// height so that it can be placed before it is laid out.
pub(super) const WIDTH: f32 = 276.0;
const PADDING: f32 = 8.0;
const SPACING: f32 = 7.0;
const FIELD_HEIGHT: f32 = 30.0;
const CAPTION_HEIGHT: f32 = 17.0;
const CHIPS_HEIGHT: f32 = 22.0;
const NAV_HEIGHT: f32 = 22.0;
const HEAD_HEIGHT: f32 = 18.0;
const DAY_HEIGHT: f32 = 29.0;
const DAY_GAP: f32 = 2.0;
const FOOT_HEIGHT: f32 = 22.0;
/// The card's height.
pub(super) const HEIGHT: f32 = 2.0 * PADDING
    + FIELD_HEIGHT
    + CAPTION_HEIGHT
    + CHIPS_HEIGHT
    + NAV_HEIGHT
    + (HEAD_HEIGHT + 6.0 * DAY_HEIGHT + 6.0 * DAY_GAP)
    + FOOT_HEIGHT
    + 5.0 * SPACING;
/// The room kept between the card and the window's edges, and between it
/// and its anchor.
const MARGIN: f32 = 8.0;
const GAP: f32 = 4.0;

/// The quick picks, as typed: a click writes the date each names.
const CHIPS: [(&str, &str); 5] = [
    ("Today", "today"),
    ("Tomorrow", "tomorrow"),
    ("Fri", "fri"),
    ("Next Mon", "next mon"),
    ("+1w", "+1w"),
];

/// Whether any widget holds the keyboard.
struct HeldFocus(bool);

impl iced::advanced::widget::Operation<bool> for HeldFocus {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<bool>),
    ) {
        operate(self);
    }

    fn focusable(
        &mut self,
        _id: Option<&iced::advanced::widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        self.0 |= state.is_focused();
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<bool> {
        iced::advanced::widget::operation::Outcome::Some(self.0)
    }
}

/// The date the quick pick labelled `label` writes, read from `today`.
pub(super) fn chip_date(label: &str, today: Date) -> Option<Date> {
    let (_, word) = CHIPS.iter().find(|(chip, _)| *chip == label)?;
    match date::parse(word, today)? {
        Parsed::Date(date) => Some(date),
        Parsed::Invalid => None,
    }
}

/// Where the card's top left goes for an anchor at `anchor` in a window of
/// `window`: under the anchor at its left edge, above it when there is no
/// room below and there is above, and inside the window, clear of the
/// editor's footer along its bottom.
pub(super) fn place(anchor: Rectangle, window: Size) -> Point {
    let bottom = window.height - super::workspace_view::FOOTER - MARGIN;
    let below = anchor.y + anchor.height + GAP;
    let above = anchor.y - GAP - HEIGHT;
    let fits_below = below + HEIGHT <= bottom;
    let y = if fits_below || above < MARGIN {
        below
    } else {
        above
    };
    let y = y.min(bottom - HEIGHT).max(MARGIN);
    let x = anchor.x.min(window.width - MARGIN - WIDTH).max(MARGIN);
    Point::new(x, y)
}

impl App {
    /// Whether the calendar can open at the caret: tasks on, a Markdown
    /// note on the advanced engine, which alone opens popovers, and the
    /// caret on a task.
    pub(super) fn calendar_can_open(&self) -> bool {
        self.editor.backend() == wren_editor::BackendKind::Owned && self.caret_on_task()
    }

    /// Tell the source of the calendar's request whether the caret's line
    /// is the line of a task's box, as the whole note reads it; once for
    /// each text, caret line and setting. After every message.
    pub(super) fn sync_due_target(&mut self) {
        self.release_task_caches();
        let form = self.task_form_takes_keys();
        self.task_source
            .form
            .store(form, std::sync::atomic::Ordering::Relaxed);
        self.task_source.advanced.store(
            self.editor.backend() == wren_editor::BackendKind::Owned,
            std::sync::atomic::Ordering::Relaxed,
        );
        let can = self.preferences.tasks
            && self.editor.backend() == wren_editor::BackendKind::Owned
            && self.markdown_in_editor()
            && self.ensure_editor_ready().is_ok();
        let line = self.editor.cursor().position.line;
        let key = (self.editor.revision(), line, can);
        if self.due_key == Some(key) {
            return;
        }
        self.due_key = Some(key);
        let boxed = can.then(|| self.box_on(line)).flatten();
        let target = boxed
            .and_then(|_| self.editor.line_text(line))
            .map(|text| (line, text));
        self.suggest_on_line(boxed.map(|task| (line, task)));
        *self.task_source.due() = target;
    }

    /// `task.set_due` from the palette, the menu or the harness: open the
    /// calendar at the caret's task.
    pub(super) fn run_set_due(&mut self) -> Gateway {
        self.palette = None;
        if !self.calendar_can_open() {
            return self.blocked("Put the caret on a task line");
        }
        let caret = self.editor.cursor().position;
        let at = Anchor::Range {
            line: caret.line,
            range: caret.column..caret.column,
        };
        match self.open_calendar(caret.line, at) {
            Some(task) => self.finish_action(Disposition::Completed, None, task),
            None => self.blocked("Put the caret on a task line"),
        }
    }

    /// A request from the editor for the calendar, `task.set_due`'s key
    /// ([`wren_editor::KeyRoute::Popover`]): open it at the caret's task.
    /// `None` for a request of something else.
    pub(super) fn request_calendar(&mut self, id: u32) -> Option<Task<Message>> {
        if id != CommandId::TaskSetDue.payload() {
            return None;
        }
        self.column = super::columns::Column::Editor;
        self.editor.close_popover();
        if !self.calendar_can_open() {
            return Some(Task::none());
        }
        let caret = self.editor.cursor().position;
        let at = Anchor::Range {
            line: caret.line,
            range: caret.column..caret.column,
        };
        Some(
            self.open_calendar(caret.line, at)
                .unwrap_or_else(Task::none),
        )
    }

    /// The editor handed the app a right-click on a due date that a
    /// provider took by taking the keyboard: open the calendar at it.
    pub(super) fn calendar_for_click(&mut self, clicked: &wren_editor::Clicked) -> Task<Message> {
        let date = clicked.spans.spans.iter().find(|span| {
            matches!(
                span.kind,
                SpanKind::Date {
                    field: task::DateField::Due,
                    ..
                }
            ) && clicked
                .column
                .is_some_and(|column| span.range.contains(&column))
        });
        let at = match date {
            Some(span) => Anchor::Range {
                line: clicked.line,
                range: span.range.clone(),
            },
            None => Anchor::Range {
                line: clicked.line,
                range: clicked.text.len()..clicked.text.len(),
            },
        };
        self.open_calendar(clicked.line, at)
            .unwrap_or_else(Task::none)
    }

    /// Open the calendar for the task line `line` belongs to, on its due
    /// date or today, hung under its due token or else `at`, and give its
    /// field the keyboard. `None` where there is no task to open it for.
    pub(super) fn open_calendar(&mut self, line: usize, at: Anchor) -> Option<Task<Message>> {
        if self.task_form.is_some()
            || self.editor.backend() != wren_editor::BackendKind::Owned
            || !self.preferences.tasks
            || !self.markdown_in_editor()
            || self.ensure_editor_ready().is_err()
        {
            return None;
        }
        let line = self.box_line_of(line)?;
        let text = self.editor.line_text(line)?;
        let parsed = task::parse(&text, boxed(&self.editor.spans_at(line))?)?;
        let due = parsed.due().map(|token| (token.range.clone(), token.value));
        let anchor = match &due {
            Some((range, _)) => Anchor::Range {
                line,
                range: range.clone(),
            },
            None => at,
        };
        let current = match due {
            Some((_, Parsed::Date(date))) => Some(date),
            _ => None,
        };
        self.editor.close_popover();
        self.palette = None;
        self.column = super::columns::Column::Editor;
        self.calendar = Some(Open {
            model: Calendar::new(self.today(), current),
            target: Target::Editor {
                note: self.selected_note,
                revision: self.editor.revision(),
                line,
                text,
                opened_at: self.editor.anchor_bounds(&anchor),
                anchor,
                top: self.editor.first_visible_line(),
                erase: None,
            },
            has_due: due.is_some(),
            composing: false,
        });
        Some(iced::widget::operation::focus(FIELD))
    }

    /// Open the calendar for the task of a row of the task view, on its
    /// due date or today, hung under `at` (the row's due cell), and give
    /// its field the keyboard. The task's box is at byte `boxed` of its
    /// line, as the task index found it. `None` with no task view showing,
    /// or where there is no task there.
    pub(super) fn open_row_calendar(
        &mut self,
        target: RowTarget,
        boxed: usize,
        at: Rectangle,
    ) -> Option<Task<Message>> {
        if !self.preferences.tasks || self.task_form.is_some() || self.surface() != Surface::Tasks {
            return None;
        }
        let due = super::task_menus::row_task(&target.expect, boxed)?
            .due()
            .map(|token| token.value);
        let current = match due {
            Some(Parsed::Date(date)) => Some(date),
            _ => None,
        };
        self.take_calendar();
        self.palette = None;
        self.column = super::columns::Column::Editor;
        self.calendar = Some(Open {
            model: Calendar::new(self.today(), current),
            target: Target::Row(target, at),
            has_due: due.is_some(),
            composing: false,
        });
        Some(iced::widget::operation::focus(FIELD))
    }

    /// Open the calendar for the task whose box is on `line`, under the
    /// word at `range` of it, which the date it writes takes the place of:
    /// "📅 Due date…" chosen from the suggestions for `du`.
    pub(super) fn open_calendar_for_word(
        &mut self,
        line: usize,
        range: std::ops::Range<usize>,
    ) -> Task<Message> {
        let at = Anchor::Range {
            line,
            range: range.clone(),
        };
        let Some(task) = self.open_calendar(line, at) else {
            return Task::none();
        };
        if let Some(Target::Editor {
            line: on, erase, ..
        }) = self.calendar.as_mut().map(|open| &mut open.target)
        {
            if *on == line {
                *erase = Some(range);
            }
        }
        task
    }

    /// The line of the box of the task line `line` belongs to.
    fn box_line_of(&self, line: usize) -> Option<usize> {
        let nothing = Change::Compound(Vec::new());
        let at = Selection::caret(Pos::new(line, 0));
        self.editor.read_markdown(|lines, options| {
            task::edit(lines, line, at, &nothing, options).map(|edit| edit.line)
        })
    }

    /// Close the calendar, whatever the reason. An input method composing
    /// in its field may still commit: the editor then drops what one
    /// commits until it sees input of its own
    /// ([`wren_editor::EditorView::hand_back`]). With none composing, what
    /// one commits next (the emoji picker's, dictation's) is the note's.
    fn take_calendar(&mut self) -> Option<Open> {
        let open = self.calendar.take()?;
        // A row's calendar was never over the note's text.
        if open.composing && matches!(open.target, Target::Editor { .. }) {
            self.editor.hand_back();
        }
        Some(open)
    }

    /// Close the calendar, writing nothing, and give the editor back the
    /// keyboard.
    pub(super) fn cancel_calendar(&mut self) -> Task<Message> {
        if self.take_calendar().is_none() {
            return Task::none();
        }
        self.return_to_editor()
    }

    /// The window lost the keyboard: close the calendar, writing nothing,
    /// and give the editor the keyboard for when the window has it again.
    pub(super) fn close_calendar_for_window(&mut self) -> Task<Message> {
        self.cancel_calendar()
    }

    pub(super) fn calendar_event(&mut self, event: Event) -> Task<Message> {
        match event {
            // A plain Escape the field took is claimed whether or not the
            // calendar is still open to close: the app's key handling hears
            // of the same press.
            Event::Field(FieldEvent::Escape | FieldEvent::HeldEscape) => {
                self.calendar_escapes += 1;
            }
            // After a blur: with nothing else holding the keyboard, the
            // editor takes it back.
            Event::FocusChecked(held) => {
                let free = !held && self.calendar.is_none() && self.task_form.is_none();
                return if free && self.palette.is_none() && self.dialog.is_none() {
                    self.return_to_editor()
                } else {
                    Task::none()
                };
            }
            _ => {}
        }
        let Some(open) = &mut self.calendar else {
            return Task::none();
        };
        match event {
            Event::Field(FieldEvent::Composing(composing)) => open.composing = composing,
            Event::Field(FieldEvent::Typed(text)) => {
                open.model.set_typed(&text);
            }
            Event::Field(FieldEvent::Key(key)) => {
                open.model.key(key);
            }
            Event::Field(FieldEvent::Enter) => match open.model.enter() {
                Outcome::Write(date) => return self.write_due(Some(date)),
                Outcome::Clear => return self.write_due(None),
                Outcome::Invalid(_) => {}
            },
            Event::Field(FieldEvent::Escape | FieldEvent::WindowLost) | Event::Outside => {
                return self.cancel_calendar();
            }
            Event::Field(FieldEvent::HeldEscape) | Event::FocusChecked(_) => {}
            // A focus moved elsewhere took the keyboard: it keeps it, and
            // with none holding it (an unfocus, say) the editor takes it.
            // The palette and the dialogs take it themselves.
            Event::Field(FieldEvent::Blur) => {
                self.take_calendar();
                if self.palette.is_some() || self.dialog.is_some() {
                    return Task::none();
                }
                return iced::advanced::widget::operate(HeldFocus(false))
                    .map(|held| Message::Calendar(Event::FocusChecked(held)));
            }
            Event::Pick(date) => return self.write_due(Some(date)),
            Event::Clear => return self.write_due(None),
            Event::Show(months) => {
                open.model.show(months);
                return iced::widget::operation::focus(FIELD);
            }
            Event::Refocus => return iced::widget::operation::focus(FIELD),
        }
        Task::none()
    }

    /// Write `due` as the task's due date, or remove it, as one undo step,
    /// close the calendar and give the editor (or the task view) back the
    /// keyboard. Nothing is written once the task's line no longer reads as
    /// it did.
    fn write_due(&mut self, due: Option<Date>) -> Task<Message> {
        let Some(open) = self.take_calendar() else {
            return Task::none();
        };
        let focus = self.return_to_editor();
        if self.calendar_stale(&open) {
            return Task::batch([
                focus,
                self.blocked("The task changed; nothing was written").task,
            ]);
        }
        match open.target {
            Target::Editor {
                line, text, erase, ..
            } => self.write_editor_due(line, &text, erase, due, focus),
            Target::Row(target, _) => {
                let what = super::task_rows::What::Due(due);
                let written = self.write_view_row(&target, None, Change::SetDue(due), what);
                Task::batch([focus, written.gateway.task])
            }
        }
    }

    /// [`App::write_due`] to the task whose box is on line `line` of the
    /// editor's text, which read `text` when the calendar opened.
    fn write_editor_due(
        &mut self,
        line: usize,
        text: &str,
        erase: Option<std::ops::Range<usize>>,
        due: Option<Date>,
        focus: Task<Message>,
    ) -> Task<Message> {
        if let Some(range) = erase.filter(|_| due.is_some()) {
            let change = Change::Compound(vec![
                task::suggest::erase(range.clone()),
                Change::SetDue(due),
            ]);
            let written = self.write_completion(line, text, &range, &change);
            return Task::batch([focus, written]);
        }
        let started = std::time::Instant::now();
        let change = Change::SetDue(due);
        let edit = self.editor.plan_markdown(|lines, selection, options| {
            task::edit(lines, line, selection, &change, options)
        });
        let Some(edit) = edit.filter(|edit| edit.line == line) else {
            return Task::batch([focus, self.blocked("Put the caret on a task line").task]);
        };
        // The caret and the selection where the line edit moved them,
        // except an end inside a due date that another date replaces: the
        // line edit moves it to the end of the new token, and a date is
        // written at the same length, so it stays where it was.
        let before = self.editor.cursor();
        let replaced = due
            .and_then(|_| task::parse(&edit.expect, edit.task))
            .and_then(|line| line.due().map(|token| token.range.clone()));
        match self.editor.apply_line_edits(&[edit]) {
            Ok(changed) => {
                if changed {
                    if let Some(replaced) = replaced {
                        let after = self.editor.cursor();
                        let inside = |at: iced::widget::text_editor::Position| {
                            at.line == line
                                && replaced.start < at.column
                                && at.column < replaced.end
                        };
                        let keep = |old: iced::widget::text_editor::Position, new| {
                            if inside(old) {
                                old
                            } else {
                                new
                            }
                        };
                        let position = keep(before.position, after.position);
                        let selection = before
                            .selection
                            .map(|anchor| keep(anchor, after.selection.unwrap_or(after.position)))
                            .filter(|anchor| *anchor != position);
                        if position != after.position || selection != after.selection {
                            self.editor.move_cursor(iced::widget::text_editor::Cursor {
                                position,
                                selection,
                            });
                        }
                    }
                    self.text_changed(started);
                }
                focus
            }
            Err(refused) => Task::batch([focus, self.blocked(refused.to_string()).task]),
        }
    }

    /// Whether what the calendar was opened for has gone: the note, its
    /// text, or the way it is shown; for a row, the task view. A row's
    /// task that changed is the write's to refuse.
    fn calendar_stale(&self, open: &Open) -> bool {
        match &open.target {
            Target::Editor {
                note,
                revision,
                line,
                text,
                ..
            } => {
                !self.preferences.tasks
                    || self.loading.is_some()
                    || self.editor.backend() != wren_editor::BackendKind::Owned
                    || !self.markdown_in_editor()
                    || self.selected_note != *note
                    || self.editor.revision() != *revision
                    || self.editor.line_text(*line).as_deref() != Some(text.as_str())
            }
            Target::Row(..) => {
                !self.preferences.tasks
                    || self.loading.is_some()
                    || self.surface() != Surface::Tasks
            }
        }
    }

    /// Close the calendar once what it was opened for has gone, or the
    /// editor scrolled, or the palette or a dialog took the keyboard; and
    /// read the field again on a new day. After every message.
    pub(super) fn sync_calendar(&mut self) -> Task<Message> {
        let today = self.today();
        let Some(open) = &mut self.calendar else {
            return Task::none();
        };
        open.model.set_today(today);
        let taken = self.palette.is_some() || self.dialog.is_some();
        let open = self.calendar.as_ref().expect("open");
        let scrolled = match &open.target {
            Target::Editor { top, .. } => self.editor.first_visible_line() != *top,
            Target::Row(..) => false,
        };
        if !taken && !self.calendar_stale(open) && !scrolled {
            return Task::none();
        }
        self.take_calendar();
        if taken {
            Task::none()
        } else {
            self.column_focus()
        }
    }

    /// `base` with the calendar over it while it is open: a clear layer
    /// over the whole window that closes it on a press or a scroll, and
    /// the card, at its anchor.
    pub(super) fn calendar_view<'a>(&'a self, base: Element<'a, Message>) -> Element<'a, Message> {
        // With tasks off it never opens, and the window is drawn as before
        // tasks. With them on (a setting read at launch), the same three
        // layers whether it is open or not, so the window's own widgets keep
        // their place in the tree, and with it their state (the find bar's
        // focus, say), as it opens and closes.
        if !self.preferences.tasks {
            return base;
        }
        // The task form is a layer of these too, never open with the
        // calendar over the note.
        if let Some(form) = &self.task_form {
            let (scrim, form) = self.task_form_layers(form);
            return stack![base, scrim, form].into();
        }
        let open = self.calendar.as_ref().zip(self.calendar_bounds());
        let Some((open, card)) = open else {
            // The task view's menu takes the same two layers.
            if let Some((scrim, menu)) = self.task_menu_layers() {
                return stack![base, scrim, menu].into();
            }
            return stack![base, iced::widget::Space::new(), iced::widget::Space::new()].into();
        };
        let at = card.position();
        let outside = mouse_area(container(iced::widget::space()).width(Fill).height(Fill))
            .on_press(Message::Calendar(Event::Outside))
            .on_right_press(Message::Calendar(Event::Outside))
            .on_middle_press(Message::Calendar(Event::Outside))
            .on_scroll(|_| Message::Calendar(Event::Outside));
        let card = mouse_area(self.calendar_card(&open.model, open.has_due, Message::Calendar))
            .on_press(Message::Calendar(Event::Refocus))
            .on_scroll(|_| Message::Calendar(Event::Outside));
        stack![base, opaque(outside), pin(opaque(card)).x(at.x).y(at.y)].into()
    }

    /// Where the open calendar's card is in the window: placed from its
    /// anchor as the editor last laid it out, or where it was when the
    /// calendar opened; for a row, from the row's due cell.
    pub(super) fn calendar_bounds(&self) -> Option<Rectangle> {
        let open = self.calendar.as_ref()?;
        let anchor = match &open.target {
            Target::Editor {
                anchor, opened_at, ..
            } => self
                .editor
                .anchor_bounds(anchor)
                .or(*opened_at)
                .unwrap_or(Rectangle::new(
                    Point::new((self.viewport.width - WIDTH) / 2.0, 96.0),
                    Size::ZERO,
                )),
            Target::Row(_, at) => *at,
        };
        Some(Rectangle::new(
            place(anchor, self.viewport),
            Size::new(WIDTH, HEIGHT),
        ))
    }

    /// The calendar's card for `model`, with Clear while `has_due`, its
    /// input sent as `send` makes it a message: over the note, or over the
    /// task form.
    pub(super) fn calendar_card<'a>(
        &'a self,
        model: &'a Calendar,
        has_due: bool,
        send: fn(Event) -> Message,
    ) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let field = date_field::date_field(
            "fri, tomorrow, +3d, 2026-12-01",
            model.typed(),
            model.today(),
            move |input| {
                input
                    .id(FIELD)
                    .font(fonts::EDITOR)
                    .size(13)
                    .padding([6, 8])
                    .width(Fill)
                    .style(move |_, status| chrome::input(definition, status, true))
            },
            move |event| send(Event::Field(event)),
        )
        .field_keys(&self.preferences.field_keys);
        let caption = model.caption();
        let caption = container(chrome::label(
            caption.to_string(),
            12,
            if caption.is_invalid() {
                colors.danger
            } else {
                colors.muted
            },
        ))
        .height(CAPTION_HEIGHT)
        .padding([0, 2]);
        let chips = row(CHIPS.iter().filter_map(|(label, _)| {
            let date = chip_date(label, model.today())?;
            Some(chip(definition, label, send(Event::Pick(date))))
        }))
        .spacing(4)
        .height(CHIPS_HEIGHT);
        let arrow = |glyph: &'static str, months: i32| {
            button(
                container(chrome::label(glyph, 14, colors.body))
                    .center_x(22)
                    .center_y(NAV_HEIGHT),
            )
            .padding(0)
            .on_press(send(Event::Show(months)))
            .style(move |_, status| chrome::control(definition, status, false, true))
        };
        let nav = row![
            arrow("‹", -1),
            container(
                chrome::label(model.month().to_string(), 13, colors.text).font(fonts::SEMIBOLD)
            )
            .center_x(Fill),
            arrow("›", 1),
        ]
        .height(NAV_HEIGHT)
        .align_y(iced::Center);
        let heads = row(WEEKDAY_HEADS.iter().map(|head| {
            container(chrome::label(*head, 11, colors.faint).font(fonts::SEMIBOLD))
                .center_x(Fill)
                .height(HEAD_HEIGHT)
                .into()
        }))
        .spacing(DAY_GAP);
        let weeks = model.grid().map(|week| {
            row(week.iter().map(|cell| match cell {
                Some(day) => day_button(definition, *day, send(Event::Pick(day.date))),
                None => container(iced::widget::space()).width(Fill).into(),
            }))
            .spacing(DAY_GAP)
            .height(DAY_HEIGHT)
            .into()
        });
        let grid = column(std::iter::once(heads.into()).chain(weeks)).spacing(DAY_GAP);
        let clear: Element<'_, Message> = if has_due {
            chip(definition, "Clear", send(Event::Clear))
        } else {
            iced::widget::space().into()
        };
        let foot = row![
            clear,
            iced::widget::space::horizontal(),
            chrome::label("↵ pick · ↑↓ week · PgUp/PgDn month", 11, colors.faint),
        ]
        .height(FOOT_HEIGHT)
        .align_y(iced::Center);
        container(
            column![
                container(field).height(FIELD_HEIGHT),
                caption,
                chips,
                nav,
                grid,
                foot
            ]
            .spacing(SPACING),
        )
        .padding(PADDING)
        .width(WIDTH)
        .height(HEIGHT)
        .style(move |_| container::Style {
            border: iced::Border {
                color: theme::color(colors.dialog_border),
                width: 1.0,
                radius: 9.0.into(),
            },
            ..chrome::surface(definition, colors.raised, 9.0, true)
        })
        .into()
    }
}

/// A quick pick, or Clear: a small rounded button.
fn chip<'a>(
    definition: wren_core::theme::Theme,
    label: &'a str,
    message: Message,
) -> Element<'a, Message> {
    let colors = definition.colors;
    button(chrome::label(label, 12, colors.body))
        .padding([2, 6])
        .on_press(message)
        .style(move |_, status| {
            let mut style = chrome::control(definition, status, false, false);
            style.border.radius = 11.0.into();
            style
        })
        .into()
}

/// A day of the grid: the selection filled with the accent, today ringed
/// in it, the task's own date underlined, and the neighbouring months'
/// days faint.
fn day_button<'a>(
    definition: wren_core::theme::Theme,
    day: Day,
    message: Message,
) -> Element<'a, Message> {
    let colors = definition.colors;
    let color = if day.selected {
        colors.editor
    } else if day.today {
        colors.accent
    } else if day.in_month {
        colors.body
    } else {
        colors.faint
    };
    let mut number = theme::color(color);
    if !day.in_month && !day.selected {
        number.a *= 0.7;
    }
    let font = if day.selected || day.today {
        fonts::SEMIBOLD
    } else {
        fonts::UI
    };
    let label: iced::widget::text::Rich<'_, (), Message> =
        rich_text![span(day.date.day().to_string())
            .color(number)
            .font(font)
            .size(12.5)
            .underline(day.current)];
    button(container(label).center_x(Fill).center_y(Fill))
        .padding(0)
        .width(Fill)
        .height(DAY_HEIGHT)
        .on_press(message)
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: if day.selected {
                    Some(theme::color(colors.accent).into())
                } else if hovered {
                    Some(theme::color(colors.selected).into())
                } else {
                    None
                },
                border: iced::Border {
                    color: theme::color(colors.accent),
                    width: if day.today && !day.selected { 1.0 } else { 0.0 },
                    radius: 6.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

/// The task whose box a line's spans read.
fn boxed(spans: &wren_markdown::span::LineSpans) -> Option<block::Task> {
    spans.spans.iter().find_map(|span| match span.kind {
        SpanKind::Box(task) => Some(task),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use iced::keyboard::{Key, Modifiers};
    use iced::widget::text_editor::{Action, Cursor, Edit, Position};
    use wren_core::ops::{Op, Session};
    use wren_markdown::calendar::Key as CalendarKey;

    use super::super::commands::{Command, CommandId, Disposition};
    use super::super::tasks::Clock;
    use super::super::Options;
    use super::*;
    use crate::keys::{Input, Nav};
    use std::sync::Arc;

    const NOTE: &str = "# Tasks\n\
        \n\
        - [ ] Open task @sam 📅 2026-11-23\n\
        - [ ] Undated task\n\
        - [ ] Alias task 🗓 2026-12-02 ^abc\n\
        \x20 its next line\n\
        - [ ] Unreal task 📅 2026-02-30\n\
        \n\
        Plain text.\n";

    /// An app on `engine` with `tasks`, today pinned to Thursday 19
    /// November 2026, and `text` open as the first of two notes.
    fn note_app(engine: &str, tasks: &str, text: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("workspace/Work/Tasks");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("A List.md"), text).unwrap();
        std::fs::write(group.join("B Other.md"), "- [ ] Other\n").unwrap();
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
        let note = first_note(&app, "A List");
        let _ = app.process_core(Op::OpenNote { note: Some(note) });
        assert_eq!(app.editor.text(), text, "{engine}");
        (directory, app)
    }

    fn first_note(app: &App, title: &str) -> NoteId {
        app.model()
            .unwrap()
            .notes
            .iter()
            .find(|note| note.title == title)
            .unwrap()
            .id
    }

    fn day(text: &str) -> Date {
        Date::parse_iso(text).unwrap()
    }

    fn line_of(text: &str, prefix: &str) -> usize {
        text.split('\n')
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix:?}"))
    }

    fn line(app: &App, at: usize) -> String {
        app.editor.line_text(at).unwrap()
    }

    fn caret(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    /// The key bound to `task.set_due` on this platform, as the editor
    /// routes it, carried out as the app carries out what the editor
    /// publishes.
    fn set_due_key(app: &mut App) -> Vec<wren_editor::Message> {
        let modifiers = if cfg!(target_os = "macos") {
            Modifiers::LOGO
        } else {
            Modifiers::CTRL
        };
        let messages = app
            .editor
            .key_message(Key::Character("d".into()), modifiers);
        for message in messages.clone() {
            let _ = app.update(Message::Editor(message));
        }
        messages
    }

    fn send(app: &mut App, event: Event) {
        let _ = app.update(Message::Calendar(event));
    }

    fn field(app: &mut App, event: FieldEvent) {
        send(app, Event::Field(event));
    }

    fn typed(app: &mut App, text: &str) {
        field(app, FieldEvent::Typed(text.into()));
    }

    fn model(app: &App) -> &Calendar {
        &app.calendar.as_ref().expect("the calendar is open").model
    }

    /// Open the calendar with the caret at `at` on the line that starts
    /// with `prefix`, by its key.
    fn open_at(app: &mut App, prefix: &str, at: usize) -> usize {
        let number = line_of(NOTE, prefix);
        app.editor.move_cursor(caret(number, at));
        let messages = set_due_key(app);
        let id = CommandId::TaskSetDue.payload();
        assert!(
            matches!(
                messages.as_slice(),
                [wren_editor::Message::Spell(wren_editor::SpellEvent::Request(request))]
                    if *request == id
            ),
            "{messages:?}"
        );
        assert!(app.calendar.is_some(), "{prefix:?}");
        number
    }

    /// The note's text, the caret and the undo history are as before.
    fn untouched(app: &App, cursor: Cursor) {
        assert_eq!(app.editor.text(), NOTE);
        assert_eq!(app.editor.cursor(), cursor);
        assert!(!app.editor.can_undo());
        let draft = &app.session.as_ref().unwrap().document().unwrap().text;
        assert_eq!(draft, NOTE);
    }

    #[test]
    fn the_key_opens_it_on_the_tasks_date_or_today_and_the_field_writes_it() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        // On a task with a due date, it opens on that date, hung under it.
        let open = open_at(&mut app, "- [ ] Open", 4);
        assert_eq!(model(&app).selected(), day("2026-11-23"));
        assert_eq!(model(&app).current(), Some(day("2026-11-23")));
        let token = line(&app, open).find('📅').unwrap();
        assert_eq!(
            app.calendar.as_ref().unwrap().anchor(),
            &Anchor::Range {
                line: open,
                range: token..line(&app, open).len()
            }
        );
        assert!(app.calendar.as_ref().unwrap().has_due());
        // Typing moves the selection, and Enter writes it as one undo step,
        // the caret staying on its character.
        typed(&mut app, "f");
        typed(&mut app, "fr");
        typed(&mut app, "fri");
        assert_eq!(model(&app).selected(), day("2026-11-20"));
        field(&mut app, FieldEvent::Enter);
        assert!(app.calendar.is_none());
        assert_eq!(line(&app, open), "- [ ] Open task @sam 📅 2026-11-20");
        assert_eq!(app.editor.cursor(), caret(open, 4));
        assert_eq!(app.column, super::super::columns::Column::Editor);
        let draft = &app.session.as_ref().unwrap().document().unwrap().text;
        assert_eq!(draft, &app.editor.text());
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), NOTE);
        assert!(!app.editor.can_undo(), "one undo step");

        // On a task with none, it opens on today, hung at the caret, and
        // the token goes at the end of the line.
        let undated = open_at(&mut app, "- [ ] Undated", 7);
        assert_eq!(model(&app).selected(), day("2026-11-19"));
        assert_eq!(model(&app).current(), None);
        assert!(!app.calendar.as_ref().unwrap().has_due());
        assert_eq!(
            app.calendar.as_ref().unwrap().anchor(),
            &Anchor::Range {
                line: undated,
                range: 7..7
            }
        );
        typed(&mut app, "+2w");
        field(&mut app, FieldEvent::Enter);
        assert_eq!(line(&app, undated), "- [ ] Undated task 📅 2026-12-03");
        assert_eq!(app.editor.cursor(), caret(undated, 7));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), NOTE);
        assert!(!app.editor.can_undo());
    }

    #[test]
    fn an_alias_keeps_its_glyph_a_line_after_the_box_opens_the_box_and_clear_removes_it() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let alias = line_of(NOTE, "- [ ] Alias");
        // From the item's next line, the calendar is the task's.
        open_at(&mut app, "  its next line", 3);
        assert_eq!(app.calendar.as_ref().unwrap().line(), alias);
        // The source claims nothing on that line, so the widget hands on
        // what follows the key in its batch: the app drops it.
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
            Edit::Insert('x'),
        ))));
        assert_eq!(app.editor.text(), NOTE);
        assert_eq!(model(&app).selected(), day("2026-12-02"));
        field(&mut app, FieldEvent::Key(CalendarKey::Down));
        field(&mut app, FieldEvent::Enter);
        assert_eq!(line(&app, alias), "- [ ] Alias task 🗓 2026-12-09 ^abc");
        assert_eq!(app.editor.cursor(), caret(alias + 1, 3));
        // Clear removes the due date, as one undo step.
        open_at(&mut app, "- [ ] Alias", 0);
        send(&mut app, Event::Clear);
        assert!(app.calendar.is_none());
        assert_eq!(line(&app, alias), "- [ ] Alias task ^abc");
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(line(&app, alias), "- [ ] Alias task 🗓 2026-12-09 ^abc");
        // A date that is no real day opens on today, and is replaced.
        let unreal = open_at(&mut app, "- [ ] Unreal", 0);
        assert_eq!(model(&app).current(), None);
        assert_eq!(model(&app).selected(), day("2026-11-19"));
        assert!(app.calendar.as_ref().unwrap().has_due());
        send(&mut app, Event::Pick(day("2026-11-25")));
        assert_eq!(line(&app, unreal), "- [ ] Unreal task 📅 2026-11-25");
    }

    #[test]
    fn a_field_that_names_no_date_writes_nothing_and_stays_open() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let cursor = caret(line_of(NOTE, "- [ ] Open"), 2);
        app.editor.move_cursor(cursor);
        set_due_key(&mut app);
        for text in ["blorp", "2026-02-30"] {
            typed(&mut app, text);
            field(&mut app, FieldEvent::Enter);
            assert!(app.calendar.is_some(), "{text}");
            assert!(model(&app).caption().is_invalid(), "{text}");
            untouched(&app, cursor);
        }
    }

    #[test]
    fn each_way_of_closing_without_writing_leaves_the_note_caret_and_selection() {
        let open_line = line_of(NOTE, "- [ ] Open");
        let selected = Cursor {
            position: Position {
                line: open_line,
                column: 9,
            },
            selection: Some(Position {
                line: open_line,
                column: 6,
            }),
        };
        type Close = fn(&mut App);
        let closes: [(&str, Close); 7] = [
            ("escape", |app| {
                field(app, FieldEvent::Escape);
                // The app's key handling hears of the same press, which was
                // the calendar's alone: the find bar stays open.
                let _ = app.update(Message::Key(Input::Nav(Nav::FieldEscape)));
            }),
            ("escape heard by the app", |app| {
                let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
            }),
            ("outside", |app| send(app, Event::Outside)),
            ("blur", |app| field(app, FieldEvent::Blur)),
            ("window", |app| {
                let _ = app.update(Message::Window(
                    iced::window::Id::unique(),
                    iced::window::Event::Unfocused,
                ));
            }),
            ("palette", |app| {
                let _ = app.dispatch(Command::Run(CommandId::CommandPalette));
            }),
            ("scroll", |app| {
                app.editor.scroll_to_line(6, wren_editor::Placement::Top);
                assert_ne!(app.editor.first_visible_line(), 0, "it scrolled");
            }),
        ];
        for (name, close) in closes {
            let (_directory, mut app) = note_app("advanced", "on", NOTE);
            app.find_open = true;
            app.editor.move_cursor(selected);
            let cursor = app.editor.cursor();
            set_due_key(&mut app);
            assert!(app.calendar.is_some(), "{name}");
            typed(&mut app, "fri");
            field(&mut app, FieldEvent::Key(CalendarKey::Down));
            close(&mut app);
            let _ = app.update(Message::Tick);
            assert!(app.calendar.is_none(), "{name}");
            untouched(&app, cursor);
            assert_eq!(app.editor.cursor(), selected, "{name}");
            if name.starts_with("escape") {
                assert!(app.find_open, "{name}: one action for one Escape");
                assert_eq!(app.calendar_escapes, 0, "{name}");
            }
        }
    }

    #[test]
    fn input_in_the_frame_of_the_handover_never_reaches_the_note() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let cursor = caret(line_of(NOTE, "- [ ] Open"), 4);
        app.editor.move_cursor(cursor);
        // The editor's widget drops what follows the request in its batch;
        // whatever it had already published after the request, the app
        // applies none of it while the calendar is open.
        set_due_key(&mut app);
        for message in [
            wren_editor::Message::Action(Action::Edit(Edit::Insert('f'))),
            wren_editor::Message::Action(Action::Edit(Edit::Paste(Arc::new("ri".into())))),
            wren_editor::Message::Action(Action::Edit(Edit::Enter)),
            wren_editor::Message::Intent(wren_editor::Intent::ToggleTask),
            wren_editor::Message::Undo,
        ] {
            let _ = app.update(Message::Editor(message));
        }
        assert!(app.calendar.is_some());
        assert_eq!(model(&app).typed(), "");
        untouched(&app, cursor);
        // Each key the field hands over acts once.
        field(&mut app, FieldEvent::Key(CalendarKey::Down));
        assert_eq!(model(&app).selected(), day("2026-11-30"));
        field(&mut app, FieldEvent::Key(CalendarKey::PageDown));
        assert_eq!(model(&app).selected(), day("2026-12-30"));
        field(&mut app, FieldEvent::HeldEscape);
        assert!(app.calendar.is_some(), "an input method's Escape");
        let _ = app.update(Message::Key(Input::Nav(Nav::FieldEscape)));
        assert!(app.calendar.is_some(), "heard by the app, it is no one's");
        assert_eq!(app.calendar_escapes, 0);
    }

    #[test]
    fn every_close_hands_the_keyboard_back_to_the_editor() {
        type Close = fn(&mut App);
        let closes: [(&str, Close); 8] = [
            ("escape", |app| field(app, FieldEvent::Escape)),
            ("outside", |app| send(app, Event::Outside)),
            ("blur", |app| field(app, FieldEvent::Blur)),
            ("window", |app| field(app, FieldEvent::WindowLost)),
            ("palette", |app| {
                let _ = app.dispatch(Command::Run(CommandId::CommandPalette));
            }),
            ("stale", |app| {
                app.editor
                    .replace_text(&NOTE.replace("Open task", "Opened task"), true);
                let _ = app.update(Message::Tick);
            }),
            ("enter", |app| field(app, FieldEvent::Enter)),
            ("clear", |app| send(app, Event::Clear)),
        ];
        for (name, close) in closes {
            for composing in [false, true] {
                let (_directory, mut app) = note_app("advanced", "on", NOTE);
                app.editor
                    .move_cursor(caret(line_of(NOTE, "- [ ] Open"), 4));
                set_due_key(&mut app);
                assert!(!app.editor.handed_back(), "{name}");
                if composing {
                    field(&mut app, FieldEvent::Composing(true));
                }
                close(&mut app);
                let _ = app.update(Message::Tick);
                assert!(app.calendar.is_none(), "{name}");
                // Closed mid-composition, an input method's commits are
                // dropped until the editor's own input
                // (`wren_editor::EditorView::hand_back`); with none
                // composing, what one commits next (the emoji picker's,
                // dictation's) is the note's.
                assert_eq!(app.editor.handed_back(), composing, "{name}");
                // The editor's own input ends it (with the palette closed,
                // which takes the keyboard while it is open).
                let _ = app.close_palette();
                let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
                    iced::widget::text_editor::Motion::Right,
                ))));
                assert!(!app.editor.handed_back(), "{name}");
            }
        }
        // A composition that ended before the close holds back nothing.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [ ] Open"), 4));
        set_due_key(&mut app);
        field(&mut app, FieldEvent::Composing(true));
        field(&mut app, FieldEvent::Composing(false));
        field(&mut app, FieldEvent::Escape);
        assert!(app.calendar.is_none());
        assert!(!app.editor.handed_back());
    }

    #[test]
    fn a_closing_key_is_the_last_the_field_takes_in_its_batch() {
        // Escape, Escape in one batch: the field closes on the first and
        // reports the second as no one's (`date_field.rs`); the app's key
        // handling hears both, and drops both. The find bar is the next
        // batch's.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let cursor = caret(line_of(NOTE, "- [ ] Open"), 4);
        app.editor.move_cursor(cursor);
        app.find_open = true;
        set_due_key(&mut app);
        field(&mut app, FieldEvent::Escape);
        field(&mut app, FieldEvent::HeldEscape);
        let _ = app.update(Message::Key(Input::Nav(Nav::FieldEscape)));
        let _ = app.update(Message::Key(Input::Nav(Nav::FieldEscape)));
        assert!(app.calendar.is_none());
        assert!(app.find_open, "the find bar stays for the next batch");
        assert_eq!(app.calendar_escapes, 0);
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(!app.find_open);
        untouched(&app, cursor);
        // What the old field publishes after a close the app drops.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor.move_cursor(cursor);
        set_due_key(&mut app);
        send(&mut app, Event::Outside);
        typed(&mut app, "x");
        field(&mut app, FieldEvent::Enter);
        untouched(&app, cursor);
    }

    #[test]
    fn a_write_keeps_the_caret_and_selection_where_they_still_are() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let open = line_of(NOTE, "- [ ] Open");
        let text = NOTE.split('\n').nth(open).unwrap();
        let month = text.find("-11-").unwrap() + 1;
        // A selection inside the date, of the month: a date of the same
        // length is written, and the selection stays on the month.
        let selected = Cursor {
            position: Position {
                line: open,
                column: month + 2,
            },
            selection: Some(Position {
                line: open,
                column: month,
            }),
        };
        app.editor.move_cursor(selected);
        set_due_key(&mut app);
        typed(&mut app, "fri");
        field(&mut app, FieldEvent::Enter);
        assert_eq!(line(&app, open), "- [ ] Open task @sam 📅 2026-11-20");
        assert_eq!(app.editor.cursor(), selected);
        assert_eq!(app.editor.selected_text().as_deref(), Some("11"));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), NOTE);
        assert!(!app.editor.can_undo(), "one undo step");
        // Clear with the caret at the end of the line: the end is past the
        // new one, and only it moves, to the new end.
        let end = caret(open, text.len());
        app.editor.move_cursor(end);
        set_due_key(&mut app);
        send(&mut app, Event::Clear);
        assert_eq!(line(&app, open), "- [ ] Open task @sam");
        assert_eq!(
            app.editor.cursor(),
            caret(open, "- [ ] Open task @sam".len())
        );
    }

    /// Clearing a due date keeps a selection of text that survives it
    /// where the line edit moved it: the block id after the date.
    #[test]
    fn clearing_the_date_keeps_a_selection_after_it() {
        let note = "- [ ] Ship 📅 2026-12-01 ^abc\n";
        let (_directory, mut app) = note_app("advanced", "on", note);
        let text = "- [ ] Ship 📅 2026-12-01 ^abc";
        let id = text.find('^').unwrap();
        let selected = Cursor {
            position: Position {
                line: 0,
                column: text.len(),
            },
            selection: Some(Position {
                line: 0,
                column: id,
            }),
        };
        app.editor.move_cursor(selected);
        set_due_key(&mut app);
        assert!(app.calendar.is_some());
        send(&mut app, Event::Clear);
        assert_eq!(line(&app, 0), "- [ ] Ship ^abc");
        assert_eq!(app.editor.selected_text().as_deref(), Some("^abc"));
        // Backwards, and with a date picked: the same text stays selected.
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        let backwards = Cursor {
            position: selected.selection.unwrap(),
            selection: Some(selected.position),
        };
        app.editor.move_cursor(backwards);
        set_due_key(&mut app);
        send(&mut app, Event::Clear);
        assert_eq!(app.editor.selected_text().as_deref(), Some("^abc"));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        app.editor.move_cursor(selected);
        set_due_key(&mut app);
        typed(&mut app, "fri");
        field(&mut app, FieldEvent::Enter);
        assert_eq!(line(&app, 0), "- [ ] Ship 📅 2026-11-20 ^abc");
        assert_eq!(app.editor.cursor(), selected);
    }

    #[test]
    fn an_escape_with_ctrl_alt_or_logo_is_not_the_calendars() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let cursor = caret(line_of(NOTE, "- [ ] Open"), 4);
        app.editor.move_cursor(cursor);
        app.find_open = true;
        set_due_key(&mut app);
        // The field takes it and says nothing (date_field.rs); the app hears
        // a shortcut, which claims nothing and closes nothing.
        let trigger = wren_core::keybind::parse_trigger("ctrl+escape").unwrap();
        let _ = app.update(Message::Key(Input::Accel(trigger)));
        assert!(app.calendar.is_some());
        assert_eq!(app.calendar_escapes, 0);
        // The next plain Escape is the calendar's, once, and the find bar's
        // after it.
        field(&mut app, FieldEvent::Escape);
        let _ = app.update(Message::Key(Input::Nav(Nav::FieldEscape)));
        assert!(app.calendar.is_none() && app.find_open);
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(!app.find_open);
        untouched(&app, cursor);
    }

    #[test]
    fn a_blur_gives_the_editor_the_keyboard_unless_something_else_holds_it() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let cursor = caret(line_of(NOTE, "- [ ] Open"), 4);
        app.editor.move_cursor(cursor);
        set_due_key(&mut app);
        // The blur closes it and asks who holds the keyboard.
        let asked = app.update(Message::Calendar(Event::Field(FieldEvent::Blur)));
        assert!(app.calendar.is_none());
        assert!(asked.units() > 0);
        // No one: the editor takes it back.
        let focus = app.update(Message::Calendar(Event::FocusChecked(false)));
        assert!(focus.units() > 0);
        assert_eq!(app.column, super::super::columns::Column::Editor);
        // Someone (the find bar's field, say): it keeps it.
        let none = app.update(Message::Calendar(Event::FocusChecked(true)));
        assert_eq!(none.units(), 0);
        // The palette took it: nothing is asked.
        set_due_key(&mut app);
        app.palette = None;
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        let _ = app.update(Message::Calendar(Event::Field(FieldEvent::Blur)));
        assert!(app.calendar.is_none());
        let none = app.update(Message::Calendar(Event::FocusChecked(false)));
        assert_eq!(none.units(), 0, "the palette keeps the keyboard");
        untouched(&app, cursor);
    }

    /// The blur's question is answered after the task form opened in the
    /// same batch: the form keeps the keyboard, and the note behind it is
    /// not given it.
    #[test]
    fn a_blur_answered_after_the_task_form_opened_leaves_the_form_the_keyboard() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "- [ ] Open");
        app.editor.move_cursor(caret(at, 4));
        set_due_key(&mut app);
        let _ = app.update(Message::Calendar(Event::Field(FieldEvent::Blur)));
        assert!(app.calendar.is_none());
        assert!(app.open_task_form(at).is_some());
        let none = app.update(Message::Calendar(Event::FocusChecked(false)));
        assert_eq!(none.units(), 0, "the note is not given the keyboard");
        let form = app.task_form.as_ref().expect("the form stays open");
        assert_eq!(form.focus, super::super::task_form::Slot::Description);
    }

    #[test]
    fn a_stale_anchor_closes_it_without_writing() {
        // The note's text changes under it.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let open = open_at(&mut app, "- [ ] Open", 4);
        typed(&mut app, "fri");
        let changed = NOTE.replace("Open task", "Opened task");
        app.editor.replace_text(&changed, true);
        let _ = app.update(Message::Tick);
        assert!(app.calendar.is_none());
        assert_eq!(line(&app, open), "- [ ] Opened task @sam 📅 2026-11-23");
        // A choice arriving for a line that has changed since writes
        // nothing.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let open = open_at(&mut app, "- [ ] Open", 4);
        let _ = app.editor.perform(Action::Move(
            iced::widget::text_editor::Motion::DocumentStart,
        ));
        let _ = app.editor.perform(Action::Edit(Edit::Insert('x')));
        send(&mut app, Event::Pick(day("2026-11-20")));
        assert!(app.calendar.is_none());
        assert_eq!(line(&app, open), "- [ ] Open task @sam 📅 2026-11-23");
        // Another note.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        open_at(&mut app, "- [ ] Open", 4);
        let other = first_note(&app, "B Other");
        let _ = app.process_core(Op::OpenNote { note: Some(other) });
        let _ = app.update(Message::Tick);
        assert!(app.calendar.is_none());
        assert_eq!(app.editor.text(), "- [ ] Other\n");
    }

    #[test]
    fn a_new_day_moves_today_and_what_is_typed() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        open_at(&mut app, "- [ ] Undated", 0);
        typed(&mut app, "fri");
        app.pin_today(day("2026-11-20"));
        let _ = app.update(Message::Tick);
        assert_eq!(model(&app).today(), day("2026-11-20"));
        assert_eq!(model(&app).selected(), day("2026-11-27"));
    }

    #[test]
    fn the_command_is_the_palettes_on_a_task_line_on_the_advanced_engine_only() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let undated = line_of(NOTE, "- [ ] Undated");
        app.editor.move_cursor(caret(undated, 3));
        assert!(app.command_enabled(CommandId::TaskSetDue));
        let outcome = app.dispatch(Command::Run(CommandId::TaskSetDue));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.calendar.as_ref().map(Open::line), Some(undated));
        send(&mut app, Event::Pick(day("2026-11-21")));
        assert_eq!(line(&app, undated), "- [ ] Undated task 📅 2026-11-21");
        // Off a task line, it is disabled and opens nothing, and its key
        // claims nothing: a letter after it is the note's.
        app.editor.move_cursor(caret(line_of(NOTE, "Plain"), 0));
        assert!(!app.command_enabled(CommandId::TaskSetDue));
        assert!(set_due_key(&mut app).len() == 1);
        assert!(app.calendar.is_none());
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
            Edit::Insert('x'),
        ))));
        assert!(app.editor.text().contains("xPlain text."));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        let outcome = app.dispatch(Command::Run(CommandId::TaskSetDue));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert!(app.calendar.is_none());

        // On classic the command is disabled, and its key opens nothing
        // and changes nothing.
        let (_directory, mut app) = note_app("classic", "on", NOTE);
        app.editor.move_cursor(caret(undated, 3));
        assert!(!app.command_enabled(CommandId::TaskSetDue));
        set_due_key(&mut app);
        assert!(app.calendar.is_none());
        assert_eq!(app.editor.text(), NOTE);
        let outcome = app.dispatch(Command::Run(CommandId::TaskSetDue));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);

        // With tasks off it does not exist: no key, no palette row.
        let (_directory, mut app) = note_app("advanced", "off", NOTE);
        app.editor.move_cursor(caret(undated, 3));
        assert!(!app.command_enabled(CommandId::TaskSetDue));
        let messages = set_due_key(&mut app);
        assert!(
            !messages.iter().any(|message| matches!(
                message,
                wren_editor::Message::Spell(wren_editor::SpellEvent::Request(_))
            )),
            "{messages:?}"
        );
        assert!(app.calendar.is_none());
        assert_eq!(app.editor.text(), NOTE);
    }

    /// A right-click on the character at byte `column` of `at`, as the
    /// widget publishes it, and the work it leaves the runtime.
    fn right_click(app: &mut App, at: usize, column: usize) -> Task<Message> {
        let palette = crate::theme::editor_colors(app.definition).1;
        app.editor.lay_out(800.0, palette);
        let message = app.editor.right_click_message(at, column).expect("a click");
        app.update(Message::Editor(message))
    }

    #[test]
    fn a_right_click_on_a_due_date_or_the_menus_row_opens_it_for_that_task() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let dated = line_of(NOTE, "- [ ] Open");
        let text = NOTE.split('\n').nth(dated).unwrap();
        let token = text.find('📅').unwrap();
        // The caret elsewhere: the calendar is the clicked task's.
        let cursor = caret(line_of(NOTE, "Plain"), 2);
        app.editor.move_cursor(cursor);
        let focus = right_click(&mut app, dated, text.find("2026").unwrap());
        assert!(app.editor.popover().is_none());
        // Only the field's focus: none of the editor's competes with it.
        assert_eq!(focus.units(), 1);
        let open = app.calendar.as_ref().expect("the calendar");
        assert_eq!(open.line(), dated);
        assert_eq!(
            open.anchor(),
            &Anchor::Range {
                line: dated,
                range: token..text.len()
            }
        );
        assert_eq!(model(&app).selected(), day("2026-11-23"));
        typed(&mut app, "tomorrow");
        field(&mut app, FieldEvent::Enter);
        assert_eq!(line(&app, dated), "- [ ] Open task @sam 📅 2026-11-20");
        assert_eq!(app.editor.cursor(), cursor);
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), NOTE);

        // The task menu's Set due date… (D), from a line after the box:
        // the box's task, and what follows the D in its batch is dropped.
        let next = line_of(NOTE, "  its next line");
        let _ = right_click(&mut app, next, 4);
        assert!(app.editor.popover().is_some(), "the task menu");
        for key in ["d", "f", "r"] {
            for message in app
                .editor
                .key_message(Key::Character(key.into()), Modifiers::empty())
            {
                let _ = app.update(Message::Editor(message));
            }
        }
        let open = app.calendar.as_ref().expect("the calendar");
        assert_eq!(open.line(), next - 1);
        assert_eq!(model(&app).selected(), day("2026-12-02"));
        assert_eq!(model(&app).typed(), "");
        untouched(&app, cursor);
        send(&mut app, Event::Clear);
        assert_eq!(line(&app, next - 1), "- [ ] Alias task ^abc");
    }

    #[test]
    fn the_source_takes_the_keyboard_for_the_calendar_on_a_tasks_box_line_only() {
        const FENCED: &str =
            "- [ ] a task\n  its next line\n\n```\n- [ ] fenced\n```\nPlain text.\n";
        let (_directory, mut app) = note_app("advanced", "on", FENCED);
        let id = CommandId::TaskSetDue.payload();
        for (line, keyboard) in [(0, true), (1, false), (4, false), (6, false)] {
            app.editor.move_cursor(caret(line, 0));
            let _ = app.update(Message::Tick);
            let text = FENCED.split('\n').nth(line).unwrap();
            assert_eq!(
                app.task_source.menu_on(id, line, text, 0),
                keyboard.then_some(Requested::Keyboard),
                "{text:?}"
            );
            // Asked about another line, or the same line read otherwise
            // (typed into earlier in the batch), it claims nothing.
            assert_eq!(app.task_source.menu_on(id, line + 1, text, 0), None);
            assert_eq!(app.task_source.menu_on(id, line, "- [ ] other", 0), None);
        }
        // The status and priority menus' requests are the task menus'.
        app.editor.move_cursor(caret(0, 0));
        let _ = app.update(Message::Tick);
        let status = CommandId::TaskSetStatus.payload();
        assert_eq!(
            app.task_source.menu_on(status, 0, "- [ ] a task", 0),
            MenuSource::menu(&TaskMenus, status, "- [ ] a task", 0)
        );
        // Off the advanced engine, or with tasks off, nothing.
        let (_directory, mut app) = note_app("classic", "on", FENCED);
        let _ = app.update(Message::Tick);
        assert_eq!(app.task_source.menu_on(id, 0, "- [ ] a task", 0), None);
        assert_eq!(
            CommandId::TaskSetDue.editor_route(),
            Some(wren_editor::KeyRoute::Popover(id))
        );
    }

    #[test]
    fn the_board_layer_keeps_its_duplicate_key_with_tasks_on() {
        use super::super::preferences::SCHEMES;
        use super::super::preferences::{board_route, build_keymap, BoardRoute};
        use wren_core::keybind::parse_trigger;
        for (scheme, trigger) in SCHEMES.into_iter().zip(["super+d", "ctrl+d", "alt+d"]) {
            let (keymap, warnings) = build_keymap(scheme, &[], true);
            assert!(warnings.is_empty(), "{warnings:?}");
            let accel = parse_trigger(trigger).unwrap();
            assert_eq!(keymap.action(&accel), Some("task.set_due"));
            assert_eq!(
                board_route(&keymap, &accel, false),
                BoardRoute::Command(wren_canvas::Command::Duplicate),
                "{scheme:?}"
            );
        }
    }

    #[test]
    fn the_windows_root_is_as_before_with_tasks_off_and_fixed_with_them_on() {
        use iced::advanced::widget::Tree;
        let base = || -> Element<'static, Message> { iced::widget::Space::new().into() };
        let space = Tree::new(base());
        // Tasks off: the window's own root, unwrapped.
        let (_directory, app) = note_app("advanced", "off", NOTE);
        let root = Tree::new(app.calendar_view(base()));
        assert!(root.tag == space.tag && root.children.is_empty());
        // Tasks on: one stack of three layers, the window's own first,
        // closed and open alike.
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let closed = Tree::new(app.calendar_view(base()));
        assert_eq!(closed.children.len(), 3);
        assert!(closed.children[0].tag == space.tag);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [ ] Open"), 4));
        set_due_key(&mut app);
        assert!(app.calendar.is_some());
        let open = Tree::new(app.calendar_view(base()));
        assert!(open.tag == closed.tag);
        assert_eq!(open.children.len(), 3);
        assert!(open.children[0].tag == space.tag);
        assert!(
            open.children[1].tag != space.tag,
            "the layer over the window"
        );
    }

    #[test]
    fn the_card_hangs_under_its_anchor_and_stays_in_the_window() {
        let window = Size::new(1000.0, 800.0);
        let anchor = |x, y| Rectangle::new(Point::new(x, y), Size::new(80.0, 20.0));
        // Under the anchor at its left edge.
        assert_eq!(
            place(anchor(300.0, 100.0), window),
            Point::new(300.0, 124.0)
        );
        // Above it with no room below.
        let low = place(anchor(300.0, 700.0), window);
        assert_eq!(low, Point::new(300.0, 700.0 - GAP - HEIGHT));
        // Pulled in from the right edge and the left.
        assert_eq!(
            place(anchor(990.0, 100.0), window).x,
            1000.0 - MARGIN - WIDTH
        );
        assert_eq!(place(anchor(-20.0, 100.0), window).x, MARGIN);
        // A window too short for it keeps its top inside.
        let short = Size::new(1000.0, 300.0);
        assert_eq!(place(anchor(300.0, 150.0), short).y, MARGIN);
        // With room under the anchor only by pulling it up, it is pulled
        // up rather than flipped.
        let footer = super::super::workspace_view::FOOTER;
        let window = Size::new(1000.0, HEIGHT + 100.0);
        let y = place(anchor(300.0, 150.0), window).y;
        assert_eq!(y, window.height - footer - MARGIN - HEIGHT);
        // macOS's default window, 1024×640: under an anchor near the top,
        // flipped above one near the bottom, and for an anchor anywhere in
        // the editor's text (below its header, above its footer) never over
        // the footer nor past the window's top.
        let mac = Size::new(1024.0, 640.0);
        assert_eq!(place(anchor(400.0, 120.0), mac).y, 144.0);
        let flipped = place(anchor(400.0, 560.0), mac);
        assert_eq!(flipped.y, 560.0 - GAP - HEIGHT);
        assert!(flipped.y + HEIGHT <= 560.0, "above its anchor");
        for top in (97..610).step_by(3) {
            let at = place(anchor(400.0, top as f32), mac);
            assert!(at.y >= MARGIN, "{top}");
            assert!(at.y + HEIGHT <= mac.height - footer - MARGIN, "{top}");
            assert!(at.x + WIDTH <= mac.width - MARGIN, "{top}");
        }
    }

    /// A note of 20,000 lines: headings, prose, lists with continuations,
    /// tasks and a fenced block in every block of ten, ending on `last`.
    fn long_note(last: &str) -> String {
        note_of(2_000, last)
    }

    /// `blocks` blocks of ten lines, as [`long_note`]'s, ending on `last`.
    fn note_of(blocks: usize, last: &str) -> String {
        let mut text = String::new();
        for block in 0..blocks {
            text.push_str(&format!(
                "## Section {block}\n\
                 Ordinary prose about the work, with a few words.\n\
                 - [ ] Task {block} @sam 📅 2026-12-01\n\
                 \x20 its next line\n\
                 - item {block}\n\
                 ```\n\
                 - [ ] fenced\n\
                 ```\n\
                 > quoted\n\
                 \n"
            ));
        }
        text.push_str(last);
        text
    }

    /// With tasks off, a message syncs none of the tasks' state: no
    /// calendar, form, caret target or suggestions, and no read of today.
    #[test]
    fn with_tasks_off_a_message_does_no_task_work() {
        for tasks in ["off", "on"] {
            let (_directory, mut app) = note_app("advanced", tasks, NOTE);
            app.due_key = None;
            let open = line_of(NOTE, "- [ ] Open");
            app.editor.move_cursor(caret(open, 3));
            let _ = app.update(Message::Tick);
            let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
                Edit::Insert('x'),
            ))));
            let today = app.suggestions.state_today();
            if tasks == "off" {
                assert_eq!(app.due_key, None);
                assert_eq!(today, Date::new(2026, 1, 1).unwrap(), "never set");
                assert_eq!(app.task_lines_scanned(), 0);
            } else {
                assert!(app.due_key.is_some());
                assert_eq!(today, day("2026-11-19"));
            }
        }
    }

    /// Another note in the editor gives back the lines kept of the one
    /// before, and the task index's copy of its draft.
    #[test]
    fn another_note_gives_back_what_was_kept() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let open = line_of(NOTE, "- [ ] Open");
        let end = NOTE.split('\n').nth(open).unwrap().len();
        app.editor.move_cursor(caret(open, end));
        let _ = app.update(Message::Tick);
        for c in " @z".chars() {
            let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
                Edit::Insert(c),
            ))));
        }
        let holds_draft =
            |app: &App| (app.session.as_ref().unwrap().tasks().unwrap()).holds_draft();
        assert!(app.task_lines_kept());
        assert!(holds_draft(&app));
        let other = first_note(&app, "B Other");
        let _ = app.process_core(Op::OpenNote { note: Some(other) });
        app.release_task_caches();
        assert!(!app.task_lines_kept());
        assert!(!holds_draft(&app));
        // A message reads only the note shown now.
        let _ = app.update(Message::Tick);
        assert!(!holds_draft(&app));
    }

    /// Typing at the end of a 20,000-line note reads a bounded number of
    /// lines a keystroke to find the caret line's task, for the calendar's
    /// source, the suggestions and the footer, not the note down to the
    /// caret.
    #[test]
    fn typing_at_the_end_of_a_long_note_reads_a_few_lines() {
        for last in ["Plain words ", "- [ ] Last task "] {
            let text = long_note(last);
            let (_directory, mut app) = note_app("advanced", "on", &text);
            let end = app.editor.line_count() - 1;
            assert!(end >= 20_000);
            app.editor.move_cursor(caret(end, last.len()));
            let _ = app.update(Message::Tick);
            let _ = app.caret_task();
            for c in "typed words".chars() {
                let before = app.task_lines_scanned();
                let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
                    Edit::Insert(c),
                ))));
                let _ = app.caret_task();
                let read = app.task_lines_scanned() - before;
                assert!(
                    read <= 2 * wren_markdown::checkpoints::EVERY as u64,
                    "{last:?}: {read} lines read for one keystroke"
                );
            }
            let task = app.box_on(end);
            assert_eq!(task.is_some(), last.starts_with("- [ ]"), "{last:?}");
        }
    }

    /// The caret line's task found from the kept states is the one a read
    /// of the whole note finds, on every line, after edits anywhere.
    #[test]
    fn the_box_found_from_kept_states_is_the_whole_notes() {
        let text = note_of(30, "- [ ] Last task");
        let (_directory, mut app) = note_app("advanced", "on", &text);
        let full = |app: &App, line: usize| {
            let nothing = Change::Compound(Vec::new());
            let at = Selection::caret(Pos::new(line, 0));
            app.editor.read_markdown(|lines, options| {
                task::edit(lines, line, at, &nothing, options)
                    .filter(|edit| edit.line == line)
                    .map(|edit| edit.task)
            })
        };
        let check = |app: &App| {
            for line in 0..app.editor.line_count() {
                assert_eq!(app.box_on(line), full(app, line), "line {line}");
            }
        };
        check(&app);
        // A fence opened, then closed, near the top; a list item made lazy.
        for (line, insert) in [(3, "```\n"), (3, "x"), (40, "- [/] ")] {
            app.editor.move_cursor(caret(line, 0));
            for c in insert.chars() {
                let action = if c == '\n' {
                    Edit::Enter
                } else {
                    Edit::Insert(c)
                };
                let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
                    action,
                ))));
            }
            check(&app);
        }
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        check(&app);
    }

    /// Typing at the end of a 20,000-line note, with tasks on, on a prose
    /// line and on a task line: the time of each keystroke's message,
    /// the footer's read of the caret's task included, against the same
    /// with tasks off.
    #[test]
    #[ignore = "release performance benchmark: 20,000 lines; run explicitly with --release"]
    fn perf_typing_at_the_end_of_a_20000_line_note() {
        for (what, last) in [("prose", "Plain words "), ("task", "- [ ] Last task ")] {
            for tasks in ["off", "on"] {
                let text = long_note(last);
                let (_directory, mut app) = note_app("advanced", tasks, &text);
                let end = app.editor.line_count() - 1;
                app.editor.move_cursor(caret(end, last.len()));
                let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
                    iced::widget::text_editor::Motion::DocumentEnd,
                ))));
                let mut times = Vec::new();
                for c in "and some more words typed".chars().cycle().take(200) {
                    let started = std::time::Instant::now();
                    let _ = app.update(Message::Editor(wren_editor::Message::Action(
                        Action::Edit(Edit::Insert(c)),
                    )));
                    let _ = std::hint::black_box(app.caret_task());
                    times.push(started.elapsed());
                }
                times.sort();
                eprintln!(
                    "typing at the end of 20,000 lines, {what} line, tasks {tasks}: \
                     min {:?}, median {:?}, p90 {:?}",
                    times[0], times[100], times[180]
                );
            }
        }
    }
}
