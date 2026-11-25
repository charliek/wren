//! The task menus (discovery/tasks.md §2 items 20 and 22, §5.1, §5.7): a
//! right-click on a task line opens the task menu, on its box the status
//! menu, and on its priority glyph the priority menu; `task.set_status`
//! and `task.set_priority` open the status and priority menus at the
//! caret's task; a right-click on a due date opens the calendar (item 4.1,
//! `calendar.rs`); and a click on the box cycles the status. They exist only
//! with the `tasks` setting on, and only on the advanced engine, which
//! alone opens popovers. Every choice rewrites the task line through
//! `wren_markdown::task::edit`, as one undo step, refused once the line no
//! longer reads as it did.
//!
//! The menus' rows, in order:
//!
//! - status: Open (Space), Doing (`/`), Done (`x`), Cancelled (`-`), the
//!   characters that go in the box;
//! - priority: High (H), Medium (M), Low (L), Highest (⇧H), Lowest (⇧L),
//!   None (N), the common three first (Q7);
//! - task: the status rows, then Set due date… (D), which opens the
//!   calendar, Priority… (P), which opens the priority menu, and Edit task…
//!   (E). Edit task is shown and cannot be chosen until the task form (item
//!   6.1) exists.
//!
//! The row of the task's status or priority is checked, and the menu opens
//! with it selected. A box with a symbol of no status of its own (`[?]`)
//! checks no status: its task reads as open, but Open would write `[ ]`.

use std::ops::Range;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use iced::Task;
use wren_editor::{
    Accelerator, Anchor, BoxClick, Choice, Clicked, Hit, MenuProvider, MenuSource,
    Mode as PopoverMode, Provider, Requested, Row, SpellEvent, TaskBox,
};
use wren_markdown::block::{self, Boxes, Scanner};
use wren_markdown::span::{LineSpans, SpanKind};
use wren_markdown::status::{self, STATUSES};
use wren_markdown::task::{self, Change, DateField, Priority, TaskLine};
use wren_markdown::{Pos, Selection};

use super::commands::{CommandId, Gateway};
use super::{App, Message};

/// Which of the task menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuKind {
    /// The status rows, then the due date, priority and edit rows.
    Task,
    Status,
    Priority,
}

impl MenuKind {
    /// The number a request for the menu goes by
    /// ([`wren_editor::KeyRoute::Popover`]): its command's payload. The
    /// task menu is opened by a right-click only.
    fn request(self) -> Option<u32> {
        match self {
            Self::Task => None,
            Self::Status => Some(CommandId::TaskSetStatus.payload()),
            Self::Priority => Some(CommandId::TaskSetPriority.payload()),
        }
    }

    fn of_request(id: u32) -> Option<Self> {
        [Self::Status, Self::Priority]
            .into_iter()
            .find(|kind| kind.request() == Some(id))
    }
}

/// What a row of a task menu does, carried as the row's number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskRow {
    /// Write this symbol in the box, with the stamps its status asks for.
    Status(char),
    /// Set the priority, or remove it.
    Priority(Option<Priority>),
    /// Set the due date: the calendar, for the task.
    Due,
    /// Open the priority menu for the task.
    PriorityMenu,
    /// Edit the task (the task form, item 6.1): not yet chosen.
    Edit,
}

/// The tag of a task row's number: bit 30, with bit 31 (a command's
/// payload, [`CommandId::payload`]) clear, so neither is taken for the
/// other.
const ROW: u32 = 0x4000_0000;
/// Where a task row's kind sits in its number, over 24 bits of payload.
const KIND_SHIFT: u32 = 24;
const PAYLOAD_MASK: u32 = (1 << KIND_SHIFT) - 1;

impl TaskRow {
    pub(super) fn id(self) -> u32 {
        let (kind, payload) = match self {
            Self::Status(symbol) => (1, u32::from(symbol)),
            Self::Priority(None) => (2, 0),
            Self::Priority(Some(level)) => (2, level_number(level)),
            Self::Due => (3, 0),
            Self::PriorityMenu => (4, 0),
            Self::Edit => (5, 0),
        };
        ROW | (kind << KIND_SHIFT) | payload
    }

    /// The row a number names, if it is exactly one's.
    pub(super) fn from_id(id: u32) -> Option<Self> {
        if id & !(ROW | (0x3f << KIND_SHIFT) | PAYLOAD_MASK) != 0 || id & ROW == 0 {
            return None;
        }
        let payload = id & PAYLOAD_MASK;
        let row = match (id >> KIND_SHIFT) & 0x3f {
            1 => Self::Status(char::from_u32(payload)?),
            2 if payload == 0 => Self::Priority(None),
            2 => Self::Priority(Some(
                *Priority::ALL.get(usize::try_from(payload).ok()? - 1)?,
            )),
            3 => Self::Due,
            4 => Self::PriorityMenu,
            5 => Self::Edit,
            _ => return None,
        };
        (row.id() == id).then_some(row)
    }
}

/// A level's place in [`Priority::ALL`], from one.
fn level_number(level: Priority) -> u32 {
    let place = Priority::ALL.iter().position(|each| *each == level);
    u32::try_from(place.expect("every level is listed") + 1).expect("five levels")
}

/// The priority menu's rows, in Q7's order: a level and its key.
const PRIORITY_ROWS: [(Option<Priority>, Accelerator); 6] = [
    (Some(Priority::High), Accelerator::plain('h')),
    (Some(Priority::Medium), Accelerator::plain('m')),
    (Some(Priority::Low), Accelerator::plain('l')),
    (Some(Priority::Highest), Accelerator::shifted('h')),
    (Some(Priority::Lowest), Accelerator::shifted('l')),
    (None, Accelerator::plain('n')),
];

/// The status rows for `task`'s box: each status Wren knows, its box drawn
/// before it, the one the box holds checked. Done writes the `X` a box
/// holds already, so choosing the status it has changes nothing.
fn status_rows(task: block::Task) -> Vec<Row> {
    let known = status::status(task.symbol).name != status::UNKNOWN;
    STATUSES
        .iter()
        .map(|status| {
            let current = known && status.kind == task.status;
            let symbol = if current { task.symbol } else { status.symbol };
            Row::new(TaskRow::Status(symbol).id(), status.name)
                .glyph(format!("[{}]", status.symbol))
                .accelerator(Accelerator::plain(status.symbol))
                .checked(current)
        })
        .collect()
}

/// The priority rows, the task's level checked.
fn priority_rows(current: Option<Priority>) -> Vec<Row> {
    PRIORITY_ROWS
        .iter()
        .map(|&(level, key)| {
            let (glyph, name) = level.map_or(("·".to_owned(), "None"), |level| {
                (level.glyph().to_string(), level.name())
            });
            Row::new(TaskRow::Priority(level).id(), name)
                .glyph(glyph)
                .accelerator(key)
                .checked(level == current)
        })
        .collect()
}

/// The rows of the menu of `kind` for the task `line` reads.
fn rows(kind: MenuKind, line: &TaskLine<'_>) -> Vec<Row> {
    let current = line.priority().map(|token| token.level);
    match kind {
        MenuKind::Status => status_rows(line.task()),
        MenuKind::Priority => priority_rows(current),
        MenuKind::Task => {
            let mut rows = status_rows(line.task());
            let due = if line.due().is_some() {
                "Change due date…"
            } else {
                "Set due date…"
            };
            rows.push(
                Row::new(TaskRow::Due.id(), due)
                    .glyph(DateField::Due.glyph().to_string())
                    .accelerator(Accelerator::plain('d'))
                    .opens_menu(),
            );
            rows.push(
                Row::new(TaskRow::PriorityMenu.id(), "Priority…")
                    .glyph(current.unwrap_or(Priority::High).glyph().to_string())
                    .accelerator(Accelerator::plain('p'))
                    .opens_menu(),
            );
            rows.push(
                Row::new(TaskRow::Edit.id(), "Edit task…")
                    .accelerator(Accelerator::plain('e'))
                    .enabled(false),
            );
            rows
        }
    }
}

/// Where the status or priority menu hangs for a request: the box, or the
/// priority glyph when the task has one.
fn request_anchor(kind: MenuKind, line: &TaskLine<'_>) -> Range<usize> {
    let task = line.task();
    match (kind, line.priority()) {
        (MenuKind::Priority, Some(token)) => token.range.clone(),
        _ => task.start..task.end,
    }
}

/// The task a line's box opens, as its spans read it.
fn task_of(spans: &LineSpans) -> Option<block::Task> {
    spans.spans.iter().find_map(|span| match span.kind {
        SpanKind::Box(task) => Some(task),
        _ => None,
    })
}

/// The menus' rows, for right-clicks and requests alike. It holds nothing:
/// every answer is read from the line it is asked about.
pub(super) struct TaskMenus;

/// The one provider and source of menus the app hands the editor, so that
/// naming it again is naming the same one.
static TASK_MENUS: LazyLock<Arc<TaskMenus>> = LazyLock::new(|| Arc::new(TaskMenus));

impl TaskMenus {
    /// The menu a right-click on `clicked` opens: none off a task line.
    fn answer(clicked: &Clicked) -> Option<(MenuKind, Range<usize>)> {
        task_of(&clicked.spans)?;
        let span = match &clicked.hit {
            Some(Hit::Span(span)) => Some(span),
            _ => None,
        };
        let glyph = || glyph_under(clicked);
        Some(match span.map(|span| (&span.kind, span.range.clone())) {
            Some((SpanKind::Box(_), range)) => (MenuKind::Status, range),
            Some((SpanKind::Priority(_), range)) => (MenuKind::Priority, range),
            // An owner and every other token open the task menu under
            // themselves; the due date is the calendar's
            // ([`TaskMenus::due_date`]).
            Some((kind, range)) if kind.is_task() => (MenuKind::Task, range),
            _ => (MenuKind::Task, glyph()),
        })
    }
}

impl TaskMenus {
    /// Whether a right-click on `clicked` fell on a task's due date, which
    /// opens the calendar: the app takes the keyboard for it.
    fn due_date(clicked: &Clicked) -> bool {
        task_of(&clicked.spans).is_some()
            && matches!(
                &clicked.hit,
                Some(Hit::Span(span))
                    if matches!(span.kind, SpanKind::Date { field: DateField::Due, .. })
            )
    }
}

impl MenuProvider for TaskMenus {
    fn menu(&self, clicked: &Clicked) -> Option<Requested> {
        self.menu_in_task(clicked, None)
    }

    /// On a line of a task's text after its box, the task menu for that
    /// task, hung under the character clicked; its rows act on the task
    /// (the app finds its box from the line the menu hangs on).
    fn menu_in_task(&self, clicked: &Clicked, owner: Option<&TaskBox>) -> Option<Requested> {
        if let Some(owner) = owner.filter(|_| task_of(&clicked.spans).is_none()) {
            let task = task_of(&owner.spans)?;
            let line = task::parse(&owner.text, task)?;
            return Some(Requested::Rows {
                range: glyph_under(clicked),
                rows: rows(MenuKind::Task, &line),
            });
        }
        if Self::due_date(clicked) {
            return Some(Requested::Keyboard);
        }
        let (kind, range) = Self::answer(clicked)?;
        let task = task_of(&clicked.spans)?;
        let line = task::parse(&clicked.text, task)?;
        Some(Requested::Rows {
            range,
            rows: rows(kind, &line),
        })
    }
}

/// The character a right-click fell on, or the end of its line past it.
fn glyph_under(clicked: &Clicked) -> Range<usize> {
    match clicked.column {
        Some(column) => {
            let width = clicked.text[column..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
            column..column + width
        }
        None => clicked.text.len()..clicked.text.len(),
    }
}

impl MenuSource for TaskMenus {
    /// The status or priority menu, for the box the caret's line opens as
    /// that line alone reads. The app checks the line against the whole
    /// note when it is handed the request ([`App::open_task_menu`]).
    ///
    /// A line with no box of its own may still belong to a task, as a line
    /// of its text after the box, which only the whole note says: the
    /// answer is then that the app decides what opens, and the keys after
    /// the request in its batch, which neither can route, are dropped
    /// ([`Requested::Keyboard`]), never typed into the note.
    fn menu(&self, id: u32, line: &str, _caret: usize) -> Option<Requested> {
        let kind = MenuKind::of_request(id)?;
        let info = Scanner::new(Boxes::Status).next_line(line);
        let parsed = (info.item)
            .and_then(|item| item.task)
            .and_then(|task| task::parse(line, task));
        let Some(parsed) = parsed else {
            return Some(Requested::Keyboard);
        };
        Some(Requested::Rows {
            range: request_anchor(kind, &parsed),
            rows: rows(kind, &parsed),
        })
    }
}

impl App {
    /// Give the editor the task menus while tasks are on and it shows a
    /// note's text: the provider a right-click asks after spelling, which
    /// so stays first, and the source of the status and priority menus.
    /// With tasks off the editor keeps the suggestions menu alone, as
    /// before tasks.
    pub(super) fn sync_task_menus(&mut self) {
        // A board's raw text is no note's, and has no task menus.
        if self.preferences.tasks && self.surface() == super::board::Surface::Text {
            let menus = Arc::clone(&*TASK_MENUS);
            self.editor.set_menu_providers(vec![
                Provider::Spelling,
                Provider::Host(Arc::clone(&menus) as Arc<dyn MenuProvider>),
            ]);
            // The calendar's request reads the whole note, which the app
            // keeps up for it; the menus' requests are the task menus'.
            self.editor
                .set_menu_source(Some(Arc::clone(&self.task_source) as Arc<dyn MenuSource>));
        } else {
            self.editor.set_menu_providers(vec![Provider::Spelling]);
            self.editor.set_menu_source(None);
        }
    }

    /// The task provider's place in the list the editor was given.
    fn task_provider(&self) -> Option<usize> {
        self.preferences.tasks.then_some(1)
    }

    /// Whether the task menus can open: tasks on, a Markdown note on the
    /// advanced engine, which alone opens popovers.
    fn task_menus_open(&self) -> bool {
        self.editor.backend() == wren_editor::BackendKind::Owned && self.caret_on_task()
    }

    /// `task.set_status` or `task.set_priority`, from the palette or the
    /// harness: open the menu at the caret's task, and give the editor the
    /// keyboard so its keys reach the menu.
    pub(super) fn run_task_menu(&mut self, kind: MenuKind) -> Gateway {
        self.palette = None;
        if !self.task_menus_open() {
            return self.blocked("Put the caret on a task line");
        }
        let focus = self.return_to_editor();
        if self.open_task_menu(kind) {
            self.finish_action(super::commands::Disposition::Completed, None, focus)
        } else {
            self.blocked("The editor opened no menu")
        }
    }

    /// A request for a task menu from a key the editor routed
    /// ([`wren_editor::KeyRoute::Popover`]): open it as the command does,
    /// or with none open what is open closes, as for any request. `None`
    /// for a request of no task menu's.
    pub(super) fn request_task_menu(&mut self, id: u32) -> Option<Task<Message>> {
        let kind = MenuKind::of_request(id)?;
        self.column = super::columns::Column::Editor;
        if !self.task_menus_open() || !self.open_task_menu(kind) {
            self.editor.close_popover();
        }
        Some(Task::none())
    }

    /// Open the status or priority menu at the caret's task. On the box's
    /// own line the editor opens it as its request, which the editor's
    /// widget has opened the same for the keys after it in their batch;
    /// on a line of the item's text after the box, or where that line
    /// alone does not read as a task, the app opens it on the box's line.
    /// Returns whether a menu opened.
    fn open_task_menu(&mut self, kind: MenuKind) -> bool {
        let Some(line) = self.caret_task_box() else {
            self.editor.close_popover();
            return false;
        };
        let caret = self.editor.cursor().position.line;
        if let Some(id) = kind.request().filter(|_| caret == line) {
            let _ = self.editor.spell(SpellEvent::Request(id));
            if self
                .editor
                .popover()
                .is_some_and(|open| open.line() == line)
            {
                return true;
            }
        }
        self.open_menu_on(kind, line)
    }

    /// Open the menu of `kind` on the task whose box is on line `line`,
    /// under the box, or for the priority menu its glyph.
    fn open_menu_on(&mut self, kind: MenuKind, line: usize) -> bool {
        let Some(text) = self.editor.line_text(line) else {
            return false;
        };
        let Some(task) = task_of(&self.editor.spans_at(line)) else {
            return false;
        };
        let Some(parsed) = task::parse(&text, task) else {
            return false;
        };
        let (range, rows) = (request_anchor(kind, &parsed), rows(kind, &parsed));
        self.editor
            .open_popover(Anchor::Range { line, range }, PopoverMode::Menu, rows)
    }

    /// The line of the box of the task the caret is on, reading the whole
    /// note: the caret's own line, or for a line of an item's text after
    /// its box, the box's.
    fn caret_task_box(&self) -> Option<usize> {
        let caret = self.editor.cursor().position.line;
        self.task_box_of(caret)
    }

    /// The line of the box of the task that line `line` belongs to.
    fn task_box_of(&self, line: usize) -> Option<usize> {
        let nothing = Change::Compound(Vec::new());
        let at = Selection::caret(Pos::new(line, 0));
        self.editor.read_markdown(|lines, options| {
            task::edit(lines, line, at, &nothing, options).map(|edit| edit.line)
        })
    }

    /// A row chosen from a menu a provider opened for a right-click
    /// ([`wren_editor::EditorView::take_provided`]), handed back to that
    /// provider: the task menus' rows act on the task they were opened
    /// for. A row of no provider of the app's does nothing.
    pub(super) fn provided_row(&mut self, provider: usize, choice: &Choice) -> Task<Message> {
        if Some(provider) != self.task_provider() {
            return Task::none();
        }
        self.task_row(choice)
    }

    /// A row chosen from a menu the app opened itself
    /// ([`wren_editor::EditorView::take_choice`]): a task menu's opened by
    /// a request, or the priority menu Priority… opens.
    pub(super) fn chosen_row(&mut self, choice: &Choice) -> Task<Message> {
        self.task_row(choice)
    }

    /// Carry out a task menu's row for the task its menu hangs on.
    fn task_row(&mut self, choice: &Choice) -> Task<Message> {
        let Some(row) = TaskRow::from_id(choice.id) else {
            return Task::none();
        };
        self.column = super::columns::Column::Editor;
        // The task the menu hangs on: its box's line, or the line of the
        // box whose text the menu's line continues.
        let open =
            self.task_lines_editable() && self.editor.backend() == wren_editor::BackendKind::Owned;
        let Some(line) = self.task_box_of(choice.line).filter(|_| open) else {
            return self
                .blocked("This command is unavailable in the current context")
                .task;
        };
        let change = match row {
            TaskRow::Status(symbol) => Change::SetStatus(symbol),
            TaskRow::Priority(level) => Change::SetPriority(level),
            TaskRow::PriorityMenu => {
                let _ = self.open_menu_on(MenuKind::Priority, line);
                return Task::none();
            }
            TaskRow::Due => {
                let at = Anchor::Range {
                    line: choice.line,
                    range: choice.range.clone(),
                };
                return self.open_calendar(line, at).unwrap_or_else(Task::none);
            }
            TaskRow::Edit => return Task::none(),
        };
        self.run_task_change(Some(change), Some(line)).task
    }

    /// A click on a task's box ([`wren_editor::Message::BoxClick`]): its
    /// status cycles, open → doing → done → open and cancelled → open, as
    /// one undo step, the caret staying where it was. Nothing happens once
    /// the text has changed since the box was pressed, or with no task
    /// there.
    pub(super) fn click_box(&mut self, click: BoxClick) -> Task<Message> {
        self.column = super::columns::Column::Editor;
        let Some(line) = self.editor.box_clicked(click) else {
            return Task::none();
        };
        if !self.task_lines_editable() || self.task_box_of(line) != Some(line) {
            // No task of a note's there, a board's raw text say: the click
            // is one in the text.
            let started = Instant::now();
            if self.editor.click_in_text(click) {
                self.text_changed(started);
            }
            return Task::none();
        }
        self.run_task_change(Some(Change::Cycle), Some(line)).task
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::Named;
    use iced::keyboard::{Key, Modifiers};
    use iced::widget::text_editor::{Cursor, Position};
    use wren_core::ops::{Op, Session};
    use wren_markdown::date::Date;

    use super::super::commands::{Command, Disposition};
    use super::super::tasks::Clock;
    use super::super::Options;
    use super::*;

    const NOTE: &str = "# Tasks\n\
        \n\
        - [ ] Open task @sam 📅 2026-11-23\n\
        - [/] Doing task ⏫\n\
        - [x] Done task ✅ 2026-11-18\n\
        - [-] Cancelled task ❌ 2026-11-17\n\
        - [?] Unknown task\n\
        - [X] Shouted task\n\
        - [ ] VS16 task ⏫\u{fe0f} 📅 2026-12-01\n\
        - [ ] Alias task 🗓 2026-12-02 ^abc\n\
        \x20 its next line\n\
        \n\
        Plain text.\n\
        ```\n\
        - [ ] code\n\
        ```\n";

    /// An app on `engine` with `tasks`, today pinned to Thursday 19
    /// November 2026, and `text` open as a note of a workspace of its own.
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

    fn line(app: &App, at: usize) -> String {
        app.editor.text().split('\n').nth(at).unwrap().to_owned()
    }

    fn caret(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    /// Lay the editor's text out as a frame does, after an edit.
    fn lay_out(app: &mut App) {
        let palette = crate::theme::editor_colors(app.definition).1;
        app.editor.lay_out(800.0, palette);
    }

    /// A right-click on the character at byte `column` of `at`, as the
    /// widget publishes it.
    fn right_click(app: &mut App, at: usize, column: usize) {
        lay_out(app);
        if let Some(message) = app.editor.right_click_message(at, column) {
            let _ = app.update(Message::Editor(message));
        }
    }

    /// A key pressed into the focused editor, as the widget routes it.
    fn press(app: &mut App, key: Key, modifiers: Modifiers) {
        for message in app.editor.key_message(key, modifiers) {
            let _ = app.update(Message::Editor(message));
        }
    }

    fn character(text: &str) -> Key {
        Key::Character(text.into())
    }

    /// A row as drawn: its glyph, label and key, and whether it is checked
    /// and can be chosen.
    type Shown = (Option<String>, String, Option<String>, bool, bool);

    /// The open menu's rows.
    fn shown(app: &App) -> Vec<Shown> {
        let popover = app.editor.popover().expect("a menu");
        (0..popover.labels().len())
            .map(|row| {
                (
                    popover.glyph(row).map(str::to_owned),
                    popover.labels()[row].clone(),
                    popover.hint(row),
                    popover.is_checked(row),
                    popover.is_enabled(row),
                )
            })
            .collect()
    }

    fn labels(app: &App) -> Vec<String> {
        app.editor.popover().expect("a menu").labels().to_vec()
    }

    /// What a right-click at byte `column` of `line` of `text` is asked
    /// about, as the editor reads a lone task line.
    fn clicked(text: &str, column: Option<usize>) -> Clicked {
        let info = Scanner::new(Boxes::Status).next_line(text);
        let spans = wren_markdown::span::spans(text, &info, Boxes::Status);
        let hit = column
            .and_then(|column| spans.at(column).cloned())
            .map(Hit::Span);
        Clicked {
            line: 0,
            text: text.to_owned(),
            spans,
            column,
            hit,
        }
    }

    #[test]
    fn the_provider_answers_for_each_place_on_a_task_line() {
        let text = "- [/] Mail @sam the deck ⏫\u{fe0f} 📅 2026-12-01 ✅ 2026-11-18 #home";
        let at = |token: &str| text.find(token).unwrap();
        let cases: &[(Option<usize>, MenuKind, Range<usize>)] = &[
            // The box, on each of its characters.
            (Some(2), MenuKind::Status, 2..5),
            (Some(3), MenuKind::Status, 2..5),
            (Some(4), MenuKind::Status, 2..5),
            // The priority glyph, with its U+FE0F.
            (Some(at("⏫")), MenuKind::Priority, at("⏫")..at("⏫") + 6),
            // An owner, a stamp and a tag: the task menu under them.
            (
                Some(at("@sam") + 1),
                MenuKind::Task,
                at("@sam")..at("@sam") + 4,
            ),
            (Some(at("✅")), MenuKind::Task, at("✅")..at("✅") + 14),
            (Some(at("#home")), MenuKind::Task, at("#home")..text.len()),
            // The description: the task menu under the character clicked.
            (Some(at("deck")), MenuKind::Task, at("deck")..at("deck") + 1),
            (Some(0), MenuKind::Task, 0..1),
            // Past the end of the text: under its end.
            (None, MenuKind::Task, text.len()..text.len()),
        ];
        for (column, kind, range) in cases {
            let answer = TaskMenus::answer(&clicked(text, *column));
            assert_eq!(answer, Some((*kind, range.clone())), "{column:?}");
            let Some(Requested::Rows { rows, .. }) =
                MenuProvider::menu(&TaskMenus, &clicked(text, *column))
            else {
                panic!("{column:?}: no rows");
            };
            let first = rows.first().map(|row| row.label.as_str());
            let expected = match kind {
                MenuKind::Priority => (6, Some("High")),
                MenuKind::Status => (4, Some("Open")),
                MenuKind::Task => (7, Some("Open")),
            };
            assert_eq!((rows.len(), first), expected, "{column:?}");
        }
        // The due date: the calendar, which takes the keyboard.
        for column in [at("📅"), at("2026-12")] {
            assert_eq!(
                MenuProvider::menu(&TaskMenus, &clicked(text, Some(column))),
                Some(Requested::Keyboard),
                "{column}"
            );
        }
        // No task line, no menu: the click passes on.
        for text in [
            "Plain text.",
            "# Heading",
            "- a plain item",
            "- [ ]no space",
        ] {
            assert_eq!(TaskMenus::answer(&clicked(text, Some(2))), None, "{text:?}");
            assert!(MenuProvider::menu(&TaskMenus, &clicked(text, Some(2))).is_none());
        }
    }

    #[test]
    fn the_rows_mark_the_tasks_status_and_priority_and_name_their_keys() {
        let menu = |text: &str, kind: MenuKind| {
            let info = Scanner::new(Boxes::Status).next_line(text);
            let parsed = task::parse(text, info.item.unwrap().task.unwrap()).unwrap();
            rows(kind, &parsed)
        };
        let keys = |rows: &[Row]| -> Vec<String> {
            rows.iter()
                .map(|row| row.accelerator.map(Accelerator::label).unwrap_or_default())
                .collect()
        };
        fn checked(rows: &[Row]) -> Vec<&str> {
            rows.iter()
                .filter(|row| row.checked)
                .map(|row| row.label.as_str())
                .collect()
        }
        let status = menu("- [ ] a", MenuKind::Status);
        assert_eq!(
            status
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["Open", "Doing", "Done", "Cancelled"]
        );
        assert_eq!(
            status
                .iter()
                .map(|row| row.glyph.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["[ ]", "[/]", "[x]", "[-]"]
        );
        assert_eq!(keys(&status), ["Space", "/", "X", "-"]);
        for (text, mark) in [
            ("- [ ] a", vec!["Open"]),
            ("- [/] a", vec!["Doing"]),
            ("- [x] a", vec!["Done"]),
            ("- [X] a", vec!["Done"]),
            ("- [-] a", vec!["Cancelled"]),
            // A symbol of no status of its own marks none.
            ("- [?] a", vec![]),
        ] {
            assert_eq!(checked(&menu(text, MenuKind::Status)), mark, "{text}");
        }
        // Done on an `X` writes the `X` back: no change.
        let shouted = menu("- [X] a", MenuKind::Status);
        assert_eq!(TaskRow::from_id(shouted[2].id), Some(TaskRow::Status('X')));
        assert_eq!(TaskRow::from_id(shouted[0].id), Some(TaskRow::Status(' ')));

        let priority = menu("- [ ] a", MenuKind::Priority);
        assert_eq!(
            priority
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["High", "Medium", "Low", "Highest", "Lowest", "None"]
        );
        assert_eq!(
            keys(&priority),
            ["H", "M", "L", "\u{21e7}H", "\u{21e7}L", "N"]
        );
        assert_eq!(
            priority
                .iter()
                .map(|row| row.glyph.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["⏫", "🔼", "🔽", "🔺", "⏬", "·"]
        );
        for (text, mark) in [
            ("- [ ] a", "None"),
            ("- [ ] a 🔺", "Highest"),
            ("- [ ] a ⏫", "High"),
            ("- [ ] a ⏫\u{fe0f}", "High"),
            ("- [ ] a 🔼", "Medium"),
            ("- [ ] a 🔽", "Low"),
            ("- [ ] a ⏬", "Lowest"),
            // The last indicator wins on read.
            ("- [ ] a ⏬ 🔼", "Medium"),
        ] {
            assert_eq!(checked(&menu(text, MenuKind::Priority)), [mark], "{text}");
        }

        let task = menu("- [/] a ⏬", MenuKind::Task);
        let shape: Vec<(&str, String, bool, bool)> = task
            .iter()
            .map(|row| {
                (
                    row.label.as_str(),
                    row.accelerator.map(Accelerator::label).unwrap_or_default(),
                    row.checked,
                    row.enabled,
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("Open", "Space".into(), false, true),
                ("Doing", "/".into(), true, true),
                ("Done", "X".into(), false, true),
                ("Cancelled", "-".into(), false, true),
                ("Set due date…", "D".into(), false, true),
                ("Priority…", "P".into(), false, true),
                ("Edit task…", "E".into(), false, false),
            ]
        );
        assert_eq!(task[5].glyph.as_deref(), Some("⏬"), "the task's priority");
        assert_eq!(task[4].glyph.as_deref(), Some("📅"));
        let dated = menu("- [ ] a 📅 2026-12-01", MenuKind::Task);
        assert_eq!(dated[4].label, "Change due date…");
        assert_eq!(dated[5].glyph.as_deref(), Some("⏫"), "no priority yet");
    }

    #[test]
    fn a_rows_number_names_it_and_no_other_number_does() {
        let rows = [
            TaskRow::Status(' '),
            TaskRow::Status('/'),
            TaskRow::Status('x'),
            TaskRow::Status('X'),
            TaskRow::Status('-'),
            TaskRow::Status('界'),
            TaskRow::Priority(None),
            TaskRow::Due,
            TaskRow::PriorityMenu,
            TaskRow::Edit,
        ]
        .into_iter()
        .chain(Priority::ALL.map(|level| TaskRow::Priority(Some(level))));
        for row in rows {
            assert_eq!(TaskRow::from_id(row.id()), Some(row), "{row:?}");
            assert_eq!(row.id() & 0x8000_0000, 0, "no command's payload");
            assert_eq!(CommandId::from_payload(row.id()), None, "{row:?}");
        }
        for id in [
            0,
            1,
            7,
            ROW,
            ROW | (2 << KIND_SHIFT) | 6,
            ROW | (9 << KIND_SHIFT),
            CommandId::TaskSetStatus.payload(),
            CommandId::TaskPriorityHigh.payload(),
            u32::MAX,
        ] {
            assert_eq!(TaskRow::from_id(id), None, "{id:#x}");
        }
        assert_eq!(
            MenuKind::of_request(CommandId::TaskSetStatus.payload()),
            Some(MenuKind::Status)
        );
        assert_eq!(
            MenuKind::of_request(CommandId::TaskSetPriority.payload()),
            Some(MenuKind::Priority)
        );
        assert_eq!(
            MenuKind::of_request(CommandId::TaskCycleStatus.payload()),
            None
        );
    }

    /// A right-click on a token of a task, then a key: the line it writes,
    /// as one undo step, and the session's draft with it.
    #[test]
    fn each_menus_keys_write_the_line_as_one_undo_step() {
        use Modifiers as M;
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let none = M::empty();
        // (the line, the token right-clicked, the key, the line after)
        let cases: &[(&str, &str, Key, Modifiers, &str)] = &[
            // The status menu, on the box.
            (
                "- [ ] Open",
                "[",
                character("/"),
                none,
                "- [/] Open task @sam 📅 2026-11-23",
            ),
            (
                "- [ ] Open",
                "]",
                character("x"),
                none,
                "- [x] Open task @sam 📅 2026-11-23 ✅ 2026-11-19",
            ),
            (
                "- [ ] Open",
                " ]",
                character("-"),
                none,
                "- [-] Open task @sam 📅 2026-11-23 ❌ 2026-11-19",
            ),
            (
                "- [/] Doing",
                "[",
                Key::Named(Named::Space),
                none,
                "- [ ] Doing task ⏫",
            ),
            (
                "- [x] Done",
                "x",
                Key::Named(Named::Space),
                none,
                "- [ ] Done task",
            ),
            (
                "- [x] Done",
                "x",
                character("-"),
                none,
                "- [-] Done task ❌ 2026-11-19",
            ),
            (
                "- [-] Cancelled",
                "-]",
                character("/"),
                none,
                "- [/] Cancelled task",
            ),
            (
                "- [?] Unknown",
                "?",
                Key::Named(Named::Space),
                none,
                "- [ ] Unknown task",
            ),
            (
                "- [?] Unknown",
                "?",
                character("x"),
                none,
                "- [x] Unknown task ✅ 2026-11-19",
            ),
            // The priority menu, on the glyph.
            (
                "- [/] Doing",
                "⏫",
                character("m"),
                none,
                "- [/] Doing task 🔼",
            ),
            (
                "- [/] Doing",
                "⏫",
                character("l"),
                none,
                "- [/] Doing task 🔽",
            ),
            (
                "- [/] Doing",
                "⏫",
                character("h"),
                M::SHIFT,
                "- [/] Doing task 🔺",
            ),
            (
                "- [/] Doing",
                "⏫",
                character("l"),
                M::SHIFT,
                "- [/] Doing task ⏬",
            ),
            (
                "- [/] Doing",
                "⏫",
                character("n"),
                none,
                "- [/] Doing task",
            ),
            (
                "- [ ] VS16",
                "⏫",
                character("m"),
                none,
                "- [ ] VS16 task 🔼 📅 2026-12-01",
            ),
            (
                "- [ ] VS16",
                "⏫",
                character("n"),
                none,
                "- [ ] VS16 task 📅 2026-12-01",
            ),
            // The task menu, on the description, an owner and the date.
            (
                "- [ ] Open",
                "task",
                character("x"),
                none,
                "- [x] Open task @sam 📅 2026-11-23 ✅ 2026-11-19",
            ),
            (
                "- [ ] Open",
                "@sam",
                character("/"),
                none,
                "- [/] Open task @sam 📅 2026-11-23",
            ),
            // (The due date opens the calendar: see `calendar.rs`.)
            (
                "- [ ] Open",
                "Open",
                character("-"),
                none,
                "- [-] Open task @sam 📅 2026-11-23 ❌ 2026-11-19",
            ),
            (
                "- [x] Done",
                "✅",
                Key::Named(Named::Space),
                none,
                "- [ ] Done task",
            ),
        ];
        for (prefix, token, key, modifiers, after) in cases {
            let at = line_of(NOTE, prefix);
            let column = NOTE.split('\n').nth(at).unwrap().find(token).unwrap();
            app.editor.move_cursor(caret(1, 0));
            right_click(&mut app, at, column);
            assert!(app.editor.popover().is_some(), "{prefix} {token}: a menu");
            assert_eq!(app.editor.popover().unwrap().provider(), Some(1));
            press(&mut app, key.clone(), *modifiers);
            assert!(app.editor.popover().is_none(), "{prefix} {token} {key:?}");
            assert_eq!(line(&app, at), *after, "{prefix} {token} {key:?}");
            let draft = &app.session.as_ref().unwrap().document().unwrap().text;
            assert_eq!(draft, &app.editor.text(), "the edit reaches the session");
            assert_eq!(app.editor.cursor(), caret(1, 0), "the caret stays");
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            assert_eq!(app.editor.text(), NOTE, "{prefix} {token}: one undo step");
            assert!(!app.editor.can_undo(), "{prefix} {token}");
        }
        // Choosing what the task has changes nothing and is no undo step.
        for (prefix, token, key) in [
            ("- [ ] Open", "[", Key::Named(Named::Space)),
            ("- [X] Shouted", "X", character("x")),
            ("- [/] Doing", "⏫", character("h")),
        ] {
            let at = line_of(NOTE, prefix);
            let column = NOTE.split('\n').nth(at).unwrap().find(token).unwrap();
            right_click(&mut app, at, column);
            press(&mut app, key, none);
            assert_eq!(app.editor.text(), NOTE, "{prefix}");
            assert!(!app.editor.can_undo(), "{prefix}");
        }
    }

    /// The task menu's Priority… opens the priority menu for the same
    /// task, whose keys then write; Change due date… opens the calendar;
    /// Edit task… cannot be chosen; Escape closes a menu and writes
    /// nothing.
    #[test]
    fn priority_opens_the_priority_menu_and_escape_writes_nothing() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let at = line_of(NOTE, "- [ ] Open");
        let column = NOTE.split('\n').nth(at).unwrap().find("task").unwrap();
        right_click(&mut app, at, column);
        assert_eq!(
            labels(&app),
            [
                "Open",
                "Doing",
                "Done",
                "Cancelled",
                "Change due date…",
                "Priority…",
                "Edit task…"
            ]
        );
        assert_eq!(
            app.editor.popover().unwrap().selected(),
            Some(0),
            "Open, checked"
        );
        // `e` does nothing, and the menu stays.
        press(&mut app, character("e"), Modifiers::empty());
        assert!(app.editor.popover().is_some());
        assert_eq!(app.editor.text(), NOTE);
        // `d` opens the calendar for the task, and Escape closes it.
        press(&mut app, character("d"), Modifiers::empty());
        assert!(app.editor.popover().is_none());
        assert_eq!(app.calendar.as_ref().map(|open| open.line()), Some(at));
        let _ = app.update(Message::Key(crate::keys::Input::Nav(
            crate::keys::Nav::Escape,
        )));
        assert!(app.calendar.is_none());
        assert_eq!(app.editor.text(), NOTE);
        right_click(&mut app, at, column);
        press(&mut app, character("p"), Modifiers::empty());
        assert_eq!(
            labels(&app),
            ["High", "Medium", "Low", "Highest", "Lowest", "None"]
        );
        let popover = app.editor.popover().unwrap();
        assert_eq!(popover.line(), at);
        assert_eq!(popover.selected(), Some(5), "None, checked");
        press(&mut app, character("h"), Modifiers::empty());
        assert_eq!(line(&app, at), "- [ ] Open task @sam ⏫ 📅 2026-11-23");
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), NOTE);

        // Escape on each menu: closed, nothing written.
        for (token, kind) in [("[", "status"), ("⏫", "priority"), ("task", "task")] {
            let at = line_of(NOTE, "- [/] Doing");
            let column = NOTE.split('\n').nth(at).unwrap().find(token).unwrap();
            right_click(&mut app, at, column);
            assert!(app.editor.popover().is_some(), "{kind}");
            press(&mut app, Key::Named(Named::Escape), Modifiers::empty());
            assert!(app.editor.popover().is_none(), "{kind}");
            assert_eq!(app.editor.text(), NOTE, "{kind}");
            assert!(!app.editor.can_undo(), "{kind}");
        }
        // Up and Down pass over the rows that cannot be chosen.
        right_click(&mut app, at, column);
        for _ in 0..5 {
            press(&mut app, Key::Named(Named::ArrowDown), Modifiers::empty());
        }
        assert_eq!(
            app.editor.popover().unwrap().selected(),
            Some(5),
            "Priority…"
        );
        press(&mut app, Key::Named(Named::Enter), Modifiers::empty());
        assert_eq!(labels(&app)[0], "High", "Enter on Priority… opens its menu");
    }

    /// A row acts on the task its menu was opened for by the right-click,
    /// whatever line the caret is on, and the caret stays there.
    #[test]
    fn a_right_clicked_menu_acts_on_its_own_task_not_the_carets() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let (a, b) = (
            line_of(NOTE, "- [ ] Open"),
            line_of(NOTE, "- [-] Cancelled"),
        );
        app.editor.move_cursor(caret(a, 4));
        right_click(&mut app, b, 3);
        press(&mut app, character("x"), Modifiers::empty());
        assert_eq!(line(&app, b), "- [x] Cancelled task ✅ 2026-11-19");
        assert_eq!(line(&app, a), "- [ ] Open task @sam 📅 2026-11-23");
        assert_eq!(app.editor.cursor(), caret(a, 4));
        // The menu is stale once its line changes under it: the row
        // writes nothing.
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        right_click(&mut app, b, 3);
        let popover = app.editor.popover().unwrap();
        let menu = popover.instance();
        app.editor.move_cursor(caret(b, 6));
        let _ = app.update(Message::Editor(wren_editor::Message::Action(
            iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert('!')),
        )));
        let edited = app.editor.text();
        let _ = app.update(Message::Editor(wren_editor::Message::Spell(
            SpellEvent::AcceptOf { menu, row: Some(1) },
        )));
        assert_eq!(app.editor.text(), edited);
    }

    #[test]
    fn spelling_comes_first_on_a_misspelled_word_of_a_task_line() {
        let text = "- [ ] Send the mispelled deck\n";
        let (_directory, mut app) = note_app("advanced", "on", text);
        let loaded =
            super::super::spelling::Loaded::new(super::super::spelling::dictionary(), None);
        let _ = app.update(Message::SpellLoaded(loaded));
        right_click(&mut app, 0, text.find("mispelled").unwrap() + 2);
        assert_eq!(
            app.editor.spell_menu().map(|menu| menu.word()),
            Some("mispelled")
        );
        press(&mut app, Key::Named(Named::Escape), Modifiers::empty());
        let _ = app.editor.close_popover();
        right_click(&mut app, 0, text.find("deck").unwrap());
        assert!(app.editor.spell_menu().is_none());
        assert_eq!(labels(&app).len(), 7, "the task menu");
    }

    #[test]
    fn set_status_and_set_priority_open_their_menus_at_the_carets_task() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let run =
            |app: &mut App, id: CommandId| app.dispatch(Command::Run(id)).processed.disposition;
        let doing = line_of(NOTE, "- [/] Doing");
        app.editor.move_cursor(caret(doing, 8));
        assert_eq!(
            run(&mut app, CommandId::TaskSetStatus),
            Disposition::Completed
        );
        let popover = app.editor.popover().unwrap();
        assert_eq!(
            (popover.line(), popover.range()),
            (doing, 2..5),
            "under the box"
        );
        assert_eq!(
            shown(&app)[1],
            (
                Some("[/]".into()),
                "Doing".into(),
                Some("/".into()),
                true,
                true
            )
        );
        press(&mut app, character("x"), Modifiers::empty());
        assert_eq!(line(&app, doing), "- [x] Doing task ⏫ ✅ 2026-11-19");
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        // The priority menu hangs under the glyph, or with none the box.
        assert_eq!(
            run(&mut app, CommandId::TaskSetPriority),
            Disposition::Completed
        );
        let glyph = NOTE.split('\n').nth(doing).unwrap().find('⏫').unwrap();
        assert_eq!(app.editor.popover().unwrap().range(), glyph..glyph + 3);
        assert_eq!(
            app.editor.popover().unwrap().selected(),
            Some(0),
            "High, checked"
        );
        press(&mut app, character("l"), Modifiers::SHIFT);
        assert_eq!(line(&app, doing), "- [/] Doing task ⏬");
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        // On the line after a box, the menu opens on the box's line.
        let next = line_of(NOTE, "  its next line");
        app.editor.move_cursor(caret(next, 3));
        assert_eq!(
            run(&mut app, CommandId::TaskSetPriority),
            Disposition::Completed
        );
        assert_eq!(app.editor.popover().unwrap().line(), next - 1);
        press(&mut app, character("m"), Modifiers::empty());
        assert_eq!(
            line(&app, next - 1),
            "- [ ] Alias task 🔼 🗓 2026-12-02 ^abc"
        );
        assert_eq!(app.editor.cursor(), caret(next, 3));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        // Off a task, in code included, they are unavailable.
        for prefix in ["Plain text.", "- [ ] code", "# Tasks"] {
            app.editor.move_cursor(caret(line_of(NOTE, prefix), 0));
            for id in [CommandId::TaskSetStatus, CommandId::TaskSetPriority] {
                assert!(!app.command_enabled(id), "{prefix} {id:?}");
                assert_eq!(run(&mut app, id), Disposition::Blocked, "{prefix} {id:?}");
                assert!(app.editor.popover().is_none(), "{prefix}");
            }
        }
        assert_eq!(app.editor.text(), NOTE);

        // On the classic engine, and with tasks off, they do nothing.
        for (engine, tasks) in [("classic", "on"), ("advanced", "off")] {
            let (_directory, mut app) = note_app(engine, tasks, NOTE);
            app.editor.move_cursor(caret(doing, 8));
            for id in [CommandId::TaskSetStatus, CommandId::TaskSetPriority] {
                assert!(!app.command_enabled(id), "{engine} {tasks} {id:?}");
                assert_eq!(run(&mut app, id), Disposition::Blocked, "{engine} {tasks}");
                assert!(app.editor.popover().is_none(), "{engine} {tasks}");
            }
            // Nor does a right-click on the box open a menu.
            right_click(&mut app, doing, 3);
            assert!(app.editor.popover().is_none(), "{engine} {tasks}");
            assert_eq!(app.editor.text(), NOTE, "{engine} {tasks}");
        }
    }

    /// A key a `keybind` line gives `task.set_status` asks the editor for
    /// the menu, so a key after it in its batch reaches the menu.
    #[test]
    fn a_bound_key_opens_the_status_menu_as_a_request() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let keybinds = vec![("alt+shift+s".to_owned(), "task.set_status".to_owned())];
        let (keymap, warnings) = super::super::preferences::build_keymap(
            wren_core::keybind::Platform::current(),
            &keybinds,
            true,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        app.preferences.keymap = keymap;
        app.apply_preferences();
        let open = line_of(NOTE, "- [ ] Open");
        app.editor.move_cursor(caret(open, 6));
        let messages = app
            .editor
            .key_message(character("s"), Modifiers::ALT | Modifiers::SHIFT);
        let payload = CommandId::TaskSetStatus.payload();
        assert!(
            matches!(
                messages.as_slice(),
                [wren_editor::Message::Spell(SpellEvent::Request(id))] if *id == payload
            ),
            "{messages:?}"
        );
        for message in messages {
            let _ = app.update(Message::Editor(message));
        }
        assert_eq!(labels(&app), ["Open", "Doing", "Done", "Cancelled"]);
        let menu = app.editor.popover().unwrap().instance();
        // A choice names the menu, as the widget names it.
        let messages = app.editor.key_message(character("/"), Modifiers::empty());
        assert!(
            matches!(
                messages.as_slice(),
                [wren_editor::Message::Spell(SpellEvent::AcceptOf { menu: m, row: Some(1) })]
                    if *m == menu
            ),
            "{messages:?}"
        );
        for message in messages {
            let _ = app.update(Message::Editor(message));
        }
        assert_eq!(line(&app, open), "- [/] Open task @sam 📅 2026-11-23");
        // Off a task line the key opens nothing.
        app.editor
            .move_cursor(caret(line_of(NOTE, "Plain text."), 0));
        for message in app
            .editor
            .key_message(character("s"), Modifiers::ALT | Modifiers::SHIFT)
        {
            let _ = app.update(Message::Editor(message));
        }
        assert!(app.editor.popover().is_none());
    }

    /// A click on the box cycles the status, open → doing → done → open
    /// and cancelled → open, as one undo step, leaving the caret.
    #[test]
    fn a_click_on_the_box_cycles_the_status() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let click = |app: &mut App, at: usize, column: usize| {
            lay_out(app);
            let message = app.editor.click_message(at, column).expect("laid out");
            let _ = app.update(Message::Editor(message));
        };
        let open = line_of(NOTE, "- [ ] Open");
        app.editor.move_cursor(caret(1, 0));
        for (column, after) in [
            (2, "- [/] Open task @sam 📅 2026-11-23"),
            (3, "- [x] Open task @sam 📅 2026-11-23 ✅ 2026-11-19"),
            (4, "- [ ] Open task @sam 📅 2026-11-23"),
        ] {
            click(&mut app, open, column);
            assert_eq!(line(&app, open), after, "{column}");
            assert_eq!(app.editor.cursor(), caret(1, 0), "the caret stays");
        }
        for _ in 0..3 {
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        }
        assert_eq!(app.editor.text(), NOTE, "one undo step a click");
        for (prefix, after) in [
            ("- [-] Cancelled", "- [ ] Cancelled task"),
            ("- [?] Unknown", "- [/] Unknown task"),
            ("- [X] Shouted", "- [ ] Shouted task"),
            ("- [x] Done", "- [ ] Done task"),
        ] {
            let at = line_of(NOTE, prefix);
            click(&mut app, at, 3);
            assert_eq!(line(&app, at), after, "{prefix}");
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            assert_eq!(app.editor.text(), NOTE, "{prefix}");
        }
        let draft = &app.session.as_ref().unwrap().document().unwrap().text;
        assert_eq!(draft, NOTE);
        // Off the box a click places the caret, and writes nothing.
        let column = NOTE.split('\n').nth(open).unwrap().find("task").unwrap();
        click(&mut app, open, column);
        assert_eq!(app.editor.text(), NOTE);
        assert_eq!(app.editor.cursor().position.line, open);
        // A box in code is no task's.
        let code = line_of(NOTE, "- [ ] code");
        click(&mut app, code, 3);
        assert_eq!(app.editor.text(), NOTE);

        // On the classic engine and with tasks off, a click on the box is
        // a click in the text.
        for (engine, tasks) in [("classic", "on"), ("advanced", "off")] {
            let (_directory, mut app) = note_app(engine, tasks, NOTE);
            lay_out(&mut app);
            assert!(
                matches!(
                    app.editor.click_message(open, 3),
                    Some(wren_editor::Message::Action(_))
                ),
                "{engine} {tasks}"
            );
            click(&mut app, open, 3);
            assert_eq!(app.editor.text(), NOTE, "{engine} {tasks}");
        }
    }

    #[test]
    fn a_click_on_a_box_whose_text_has_changed_writes_nothing() {
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let open = line_of(NOTE, "- [ ] Open");
        lay_out(&mut app);
        let message = app.editor.click_message(open, 3).unwrap();
        app.editor.move_cursor(caret(open, 0));
        let _ = app.update(Message::Editor(wren_editor::Message::Action(
            iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert('\n')),
        )));
        let edited = app.editor.text();
        let _ = app.update(Message::Editor(message));
        assert_eq!(app.editor.text(), edited);
    }

    /// A right-click on a line of a task's text after its box opens the
    /// task menu there, whose rows act on that task; spelling still comes
    /// first on a misspelled word of the line.
    #[test]
    fn a_right_click_on_a_tasks_next_line_opens_its_task_menu() {
        let text = "- [/] First line ⏫\n  continued with a mispelled word\n\nPlain.\n";
        let (_directory, mut app) = note_app("advanced", "on", text);
        let column = "  continued with".len();
        app.editor.move_cursor(caret(3, 0));
        right_click(&mut app, 1, column);
        let popover = app.editor.popover().expect("the task menu");
        assert_eq!(popover.line(), 1, "it hangs where the click was");
        assert_eq!(popover.range(), column..column + 1);
        assert_eq!(labels(&app).len(), 7);
        assert!(popover.is_checked(1), "the task's own status, Doing");
        press(&mut app, character("x"), Modifiers::empty());
        assert_eq!(line(&app, 0), "- [x] First line ⏫ ✅ 2026-11-19");
        assert_eq!(line(&app, 1), "  continued with a mispelled word");
        assert_eq!(app.editor.cursor(), caret(3, 0));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        // Priority… opens the priority menu on the task's own line.
        right_click(&mut app, 1, column);
        press(&mut app, character("p"), Modifiers::empty());
        assert_eq!(
            app.editor.popover().map(wren_editor::Popover::line),
            Some(0)
        );
        press(&mut app, character("m"), Modifiers::empty());
        assert_eq!(line(&app, 0), "- [/] First line 🔼");
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), text);
        // Below the task, no menu.
        right_click(&mut app, 3, 1);
        assert!(app.editor.popover().is_none());
        // Spelling first.
        let loaded =
            super::super::spelling::Loaded::new(super::super::spelling::dictionary(), None);
        let _ = app.update(Message::SpellLoaded(loaded));
        right_click(
            &mut app,
            1,
            text.find("mispelled").unwrap() - "- [/] First line ⏫\n".len() + 2,
        );
        assert_eq!(
            app.editor.spell_menu().map(|menu| menu.word()),
            Some("mispelled")
        );
    }

    /// The source of the status menu answers for a box on the caret's
    /// line, and for any other line leaves it to the app, so the keys
    /// after the request in its batch are dropped, never typed; the app
    /// then opens the menu on the box of the task the line belongs to.
    #[test]
    fn a_request_off_a_box_line_is_the_apps_to_resolve() {
        let status = CommandId::TaskSetStatus.payload();
        let source = |line: &str| MenuSource::menu(&TaskMenus, status, line, 0);
        assert!(matches!(source("- [ ] a"), Some(Requested::Rows { .. })));
        for line in ["  its next line", "Plain text.", ""] {
            assert_eq!(source(line), Some(Requested::Keyboard), "{line:?}");
        }
        assert_eq!(MenuSource::menu(&TaskMenus, 7, "- [ ] a", 0), None);

        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let keybinds = vec![("alt+shift+s".to_owned(), "task.set_status".to_owned())];
        let (keymap, _) = super::super::preferences::build_keymap(
            wren_core::keybind::Platform::current(),
            &keybinds,
            true,
        );
        app.preferences.keymap = keymap;
        app.apply_preferences();
        let next = line_of(NOTE, "  its next line");
        app.editor.move_cursor(caret(next, 3));
        press(&mut app, character("s"), Modifiers::ALT | Modifiers::SHIFT);
        assert_eq!(
            app.editor.popover().map(wren_editor::Popover::line),
            Some(next - 1)
        );
        press(&mut app, character("x"), Modifiers::empty());
        assert_eq!(
            line(&app, next - 1),
            "- [x] Alias task 🗓 2026-12-02 ✅ 2026-11-19 ^abc"
        );
        assert_eq!(line(&app, next), "  its next line");
    }
}
