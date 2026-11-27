//! Writes from the task view (discovery/tasks.md §9 and §10;
//! discovery/tasks-build.md item 10.4): keys, clicks and menus on a row
//! change its task in its note, through the one path every write from
//! outside the editor takes ([`App::write_task_row`], `task_target.rs`).
//!
//! - **Keys** on the selected row, while the list has the keyboard: Space,
//!   and `markdown.toggle_task`'s key (⌘↵), toggle done; S opens the status
//!   menu, P the priority menu, D the calendar and E the task form; + and −
//!   raise and lower the priority; R refreshes the list. `edit.undo`'s key
//!   (⌘Z / Ctrl+Z) undoes the last task write.
//! - **Clicks:** the box cycles the status (open → doing → done → open,
//!   cancelled → open), except a box whose symbol has no status of its
//!   own, which is only selected; the priority opens the priority menu and
//!   the due date the calendar; a right-click on a row opens its task menu:
//!   the editor's rows, then "Open the note at this line" and "Hide tasks
//!   in <note>".
//! - **Menus** are the editor's rows (`task_menus.rs`) drawn by the host
//!   ([`super::host_menu`]) under the row's cell, for the task the row
//!   showed when they opened. The calendar and the form open over the row
//!   with the same target (`calendar.rs`, `task_form.rs`).
//! - **Every write** shows a message naming what changed and the note's
//!   line, with Undo (`notice.rs`). A row whose task changed since it was
//!   listed is refused with "This task changed; the list was refreshed",
//!   and the list shows the note as it is now.
//! - **Rows stay put until R.** A row written keeps its group and its place
//!   there, and stays shown whatever the filters say of it now
//!   ([`super::task_list::Hold`]), so the next key acts on the row the user
//!   is looking at. R, another filter or another scope lets the rows move to
//!   where they now belong.

use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::{Rectangle, Task};
use wren_editor::{Accelerator, Row};
use wren_markdown::date::Date;
use wren_markdown::status::{self, StatusType};
use wren_markdown::task::{Change, Priority};

use super::commands::CommandId;
use super::notice::{Notice, Part, Undo};
use super::task_list::{Entry, Hold, RowKey};
use super::task_list_view::{Cell, Control, MenuFor};
use super::task_menus::{self, Follow, MenuKind, TaskRow};
use super::task_target::{Refusal, RowChange, RowTarget, RowWrite, TaskWrite, Undone};
use super::{App, Message};
use crate::list_keys::Press;

/// The task menu's own rows, after the editor's.
const OPEN_ROW: u32 = 1;
const HIDE_ROW: u32 = 2;

/// What a key of the list does to the selected row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowAct {
    Toggle,
    StatusMenu,
    PriorityMenu,
    Calendar,
    Form,
    Raise,
    Lower,
    Refresh,
}

impl RowAct {
    /// The act a plain key names, if any.
    pub(super) fn of(press: &Press) -> Option<Self> {
        if press.key == Key::Named(Named::Space) {
            return Some(Self::Toggle);
        }
        if let Key::Character(text) = &press.key {
            match text.as_str() {
                "+" | "=" => return Some(Self::Raise),
                "-" | "_" => return Some(Self::Lower),
                _ => {}
            }
        }
        match press.key.to_latin(press.physical)?.to_ascii_lowercase() {
            's' => Some(Self::StatusMenu),
            'p' => Some(Self::PriorityMenu),
            'd' => Some(Self::Calendar),
            'e' => Some(Self::Form),
            'r' => Some(Self::Refresh),
            _ => None,
        }
    }
}

/// What a write did, as its message says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum What {
    /// Space or ⌘↵: done, or back to open.
    Toggle,
    /// A click on the box.
    Cycle,
    /// A status menu's row.
    Status,
    Priority(Option<Priority>),
    Raise,
    Lower,
    Due(Option<Date>),
    /// The task form.
    Saved,
}

impl What {
    /// The message's first words, for a task that reads `after` now.
    fn label(self, after: &str) -> String {
        let status = task_menus::row_task(after).map(|line| line.task().status);
        match self {
            Self::Toggle if status == Some(StatusType::Done) => "Done".into(),
            Self::Toggle => "Reopened".into(),
            Self::Cycle | Self::Status => {
                format!("Marked {}", status.unwrap_or(StatusType::Open).name())
            }
            Self::Priority(Some(level)) => format!("{} priority", level.name()),
            Self::Priority(None) => "Priority cleared".into(),
            Self::Raise => "Priority raised".into(),
            Self::Lower => "Priority lowered".into(),
            Self::Due(Some(date)) => format!("Due {}", date.short()),
            Self::Due(None) => "Due date cleared".into(),
            Self::Saved => "Saved".into(),
        }
    }

    fn of_row(row: TaskRow) -> Self {
        match row {
            TaskRow::Priority(level) => Self::Priority(level),
            _ => Self::Status,
        }
    }
}

impl App {
    /// The entry of the selected row, while it is one.
    fn selected_row(&self) -> Option<usize> {
        let entry = self.task_list.selected.as_ref()?.entry;
        self.task_list.listing.record(entry).map(|_| entry)
    }

    fn entry_target(&self, entry: usize) -> Option<RowTarget> {
        self.task_list.listing.record(entry).map(RowTarget::of)
    }

    /// Where a row's cell was last drawn, else the row, else the middle of
    /// the window.
    fn cell_anchor(&self, entry: usize, cell: Cell) -> Rectangle {
        (self.task_list.drawn(Control::Cell(entry, cell)))
            .or_else(|| self.task_list.drawn(Control::Row(entry)))
            .unwrap_or_else(|| self.unplaced_anchor())
    }

    /// A key of the list on the selected row. Returns whether it stops its
    /// batch: one that opens a menu, the calendar or the form does.
    pub(super) fn row_key(&mut self, act: RowAct) -> (Task<Message>, bool) {
        let Some(entry) = self.selected_row().filter(|_| act != RowAct::Refresh) else {
            if act == RowAct::Refresh {
                self.task_list.held.clear();
            }
            return (Task::none(), false);
        };
        let write = |app: &mut Self, change, what| (app.write_entry(entry, change, what), false);
        match act {
            RowAct::Toggle => write(self, Change::ToggleDone, What::Toggle),
            RowAct::Raise => write(self, Change::RaisePriority, What::Raise),
            RowAct::Lower => write(self, Change::LowerPriority, What::Lower),
            RowAct::StatusMenu => (self.open_row_menu(MenuKind::Status, entry), true),
            RowAct::PriorityMenu => (self.open_row_menu(MenuKind::Priority, entry), true),
            RowAct::Calendar => (self.open_entry_calendar(entry), true),
            RowAct::Form => (self.open_entry_form(entry), true),
            RowAct::Refresh => (Task::none(), false),
        }
    }

    /// `markdown.toggle_task`'s key and `edit.undo`'s, while the list has
    /// the keyboard: the selected row's done toggle, and the undo of the
    /// last task write. `None` for any other command, or with the view
    /// without the keyboard; nothing while a menu over the view is open.
    pub(super) fn task_view_shortcut(&mut self, id: CommandId) -> Option<Task<Message>> {
        if !matches!(id, CommandId::ToggleTask | CommandId::EditUndo) || !self.task_view_has_keys()
        {
            return None;
        }
        if self.task_list.menu.is_some() {
            return Some(Task::none());
        }
        Some(match id {
            CommandId::ToggleTask => match self.selected_row() {
                Some(entry) if self.task_list.focus.is_none() => {
                    self.write_entry(entry, Change::ToggleDone, What::Toggle)
                }
                _ => Task::none(),
            },
            _ => self.undo_view_write(),
        })
    }

    /// A click on a cell of the row at `entry`, which is selected.
    pub(super) fn click_cell(&mut self, entry: usize, cell: Cell) -> Task<Message> {
        match cell {
            Cell::Box => {
                let Some(record) = self.task_list.listing.record(entry) else {
                    return Task::none();
                };
                // A symbol of no status of its own is never cycled away.
                if status::status(record.symbol).name == status::UNKNOWN {
                    return Task::none();
                }
                self.write_entry(entry, Change::Cycle, What::Cycle)
            }
            Cell::Priority => self.open_row_menu(MenuKind::Priority, entry),
            Cell::Due => self.open_entry_calendar(entry),
        }
    }

    fn write_entry(&mut self, entry: usize, change: Change, what: What) -> Task<Message> {
        match self.entry_target(entry) {
            Some(target) => self.write_view_row(&target, change, what).gateway.task,
            None => Task::none(),
        }
    }

    /// Change the task of the row `target` from the view: through
    /// [`App::write_task_row`], with a message with Undo, the row held
    /// where it stands; refused, the message says why.
    pub(super) fn write_view_row(
        &mut self,
        target: &RowTarget,
        change: Change,
        what: What,
    ) -> TaskWrite<RowWrite> {
        let place = self.hold_place(target);
        let written = self.write_task_row(target, &RowChange::Change(change));
        match &written.result {
            Ok(write) if write.changed => {
                self.hold_row(place, target, write);
                let words = format!(
                    "{} \u{b7} wrote line {} of {}",
                    what.label(&write.after),
                    write.line + 1,
                    write.path.as_path().display()
                );
                let notice = Notice::new(vec![Part::Text(words)]);
                self.notify(match self.task_writes.mark() {
                    Some(mark) => notice.with_undo(Undo::TaskWrite(mark)),
                    None => notice,
                });
            }
            Ok(_) => {}
            Err(refusal) => self.refused_in_view(refusal),
        }
        written
    }

    /// Say why a write from the view wrote nothing: in the message, in
    /// place of the banner the write path left.
    fn refused_in_view(&mut self, refusal: &Refusal) {
        if self.banner.as_deref() == Some(refusal.message.as_str()) {
            self.banner = None;
        }
        self.notify(Notice::new(vec![Part::Text(refusal.message.clone())]));
    }

    /// `edit.undo`'s key in the view: undo the last task write, through the
    /// same path and with the same check.
    pub(super) fn undo_view_write(&mut self) -> Task<Message> {
        if !self.can_undo_task_write() {
            self.notify(Notice::new(vec![Part::Text(
                "Nothing to undo here.".into(),
            )]));
            return Task::none();
        }
        let undone = self.undo_task_write();
        match &undone.result {
            Ok(done) => {
                if let Undone::Line(write) = done {
                    self.rekey_selection(write.note, write.line, write.line, &write.after);
                }
                // A message's Undo for the write undone is spent.
                if matches!(
                    self.notice.as_ref().and_then(|shown| shown.notice.undo),
                    Some(Undo::TaskWrite(_))
                ) {
                    self.notice = None;
                }
            }
            Err(refusal) => self.refused_in_view(refusal),
        }
        undone.gateway.task
    }

    /// Where the row of `target` stands in the list as shown: what holds
    /// it there once written.
    fn hold_place(&self, target: &RowTarget) -> Option<Hold> {
        let listing = &self.task_list.listing;
        listing.rows().find_map(|entry| {
            let Entry::Row { group, record } = listing.entries[entry] else {
                return None;
            };
            let found = &listing.records[record];
            if found.note != target.note || found.line != target.line {
                return None;
            }
            let group = &listing.groups[group];
            Some(Hold {
                note: target.note,
                line: target.line,
                group: group.key.clone(),
                label: (group.label.clone(), group.detail.clone()),
                index: group.rows.iter().position(|&row| row == record)?,
            })
        })
    }

    /// Hold the row `target` written as `write`: where it stood when first
    /// written since the list was refreshed.
    fn hold_row(&mut self, place: Option<Hold>, target: &RowTarget, write: &RowWrite) {
        let held = &mut self.task_list.held;
        if let Some(hold) =
            (held.iter_mut()).find(|hold| hold.note == target.note && hold.line == target.line)
        {
            hold.line = write.line;
        } else if let Some(mut hold) = place {
            hold.line = write.line;
            held.push(hold);
        }
        self.rekey_selection(target.note, target.line, write.line, &write.after);
    }

    /// The selection on the task at `line` of `note` follows it to `moved`,
    /// where it reads `source`.
    fn rekey_selection(
        &mut self,
        note: wren_core::model::NoteId,
        line: usize,
        moved: usize,
        source: &str,
    ) {
        if let Some(selected) = &mut self.task_list.selected {
            if selected.key.note == note && selected.key.line == line {
                selected.key = RowKey {
                    note,
                    line: moved,
                    source: source.to_owned(),
                };
            }
        }
    }

    /// Open the status, priority or task menu of the row at `entry`, under
    /// its box, or its priority for the priority menu.
    pub(super) fn open_row_menu(&mut self, kind: MenuKind, entry: usize) -> Task<Message> {
        let Some(record) = self.task_list.listing.record(entry) else {
            return Task::none();
        };
        let target = RowTarget::of(record);
        let Some(line) = task_menus::row_task(&target.expect) else {
            return Task::none();
        };
        let mut rows = task_menus::rows(kind, &line);
        if kind == MenuKind::Task {
            rows.push(
                Row::new(OPEN_ROW, "Open the note at this line")
                    .glyph("\u{21b5}")
                    .accelerator(Accelerator::plain('o')),
            );
            rows.push(
                Row::new(
                    HIDE_ROW,
                    format!("Hide tasks in {}", self.note_title(record.note)),
                )
                .glyph("\u{2298}")
                .accelerator(Accelerator::plain('h')),
            );
        }
        let cell = match kind {
            MenuKind::Priority => Cell::Priority,
            MenuKind::Status | MenuKind::Task => Cell::Box,
        };
        let anchor = self.cell_anchor(entry, cell);
        let purpose = MenuFor::Row {
            kind,
            target,
            entry,
            generation: self.task_list.generation,
        };
        self.open_host_menu(purpose, None, rows, anchor)
    }

    /// Open the calendar over the row at `entry`, under its due date.
    fn open_entry_calendar(&mut self, entry: usize) -> Task<Message> {
        let Some(target) = self.entry_target(entry) else {
            return Task::none();
        };
        let anchor = self.cell_anchor(entry, Cell::Due);
        self.task_list.menu = None;
        self.open_row_calendar(target, anchor)
            .unwrap_or_else(Task::none)
    }

    /// Open the task form over the row at `entry`.
    fn open_entry_form(&mut self, entry: usize) -> Task<Message> {
        let Some(target) = self.entry_target(entry) else {
            return Task::none();
        };
        self.task_list.menu = None;
        self.open_row_task_form(target).unwrap_or_else(Task::none)
    }

    /// The entry the row `target` is at now: `entry` while the listing it
    /// was found in still shows, else where the list finds that task.
    fn entry_of(&self, target: &RowTarget, entry: usize, generation: u64) -> Option<usize> {
        let listing = &self.task_list.listing;
        let same = |entry: usize| {
            listing
                .record(entry)
                .is_some_and(|record| record.note == target.note && record.line == target.line)
        };
        if generation == self.task_list.generation && same(entry) {
            return Some(entry);
        }
        let key = RowKey {
            note: target.note,
            line: target.line,
            source: target.expect.clone(),
        };
        listing.find(&key, None)
    }

    /// Act on the row `row` of a row's menu, opened for `target`.
    pub(super) fn choose_row_menu(
        &mut self,
        target: &RowTarget,
        entry: usize,
        generation: u64,
        row: u32,
    ) -> Task<Message> {
        let entry = self.entry_of(target, entry, generation);
        match row {
            OPEN_ROW => entry.map_or_else(Task::none, |entry| self.open_task_at(entry)),
            HIDE_ROW => self.toggle_note_tasks(target.note).task,
            id => match TaskRow::from_id(id) {
                Some(chosen) => match chosen.act() {
                    Ok(change) => {
                        self.write_view_row(target, change, What::of_row(chosen))
                            .gateway
                            .task
                    }
                    Err(Follow::PriorityMenu) => entry.map_or_else(Task::none, |entry| {
                        self.open_row_menu(MenuKind::Priority, entry)
                    }),
                    Err(Follow::Due) => {
                        let anchor = match entry {
                            Some(entry) => self.cell_anchor(entry, Cell::Due),
                            None => self.unplaced_anchor(),
                        };
                        self.open_row_calendar(target.clone(), anchor)
                            .unwrap_or_else(Task::none)
                    }
                    Err(Follow::Edit) => self
                        .open_row_task_form(target.clone())
                        .unwrap_or_else(Task::none),
                },
                None => Task::none(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::{NativeCode, Physical};
    use iced::keyboard::Modifiers;
    use wren_core::fs_store::RelPath;
    use wren_core::ops::Session;

    use super::super::calendar::Event as CalendarEvent;
    use super::super::commands::Command;
    use super::super::host_menu;
    use super::super::task_form::Event as FormEvent;
    use super::super::task_list_view::Click;
    use super::super::tasks_view::{Event, Scope};
    use super::super::Options;
    use super::*;
    use crate::date_field::FieldEvent;
    use crate::keys::{Input, Nav};
    use crate::list_keys::ListKey;

    const A: &str = "# A\n\n- [ ] Mail the deck 📅 2026-12-01\n- [ ] Book the room\n";
    const B: &str = "# B\n\n- [ ] Order lunch\n";
    const LUNCH: &str = "- [ ] Order lunch";
    const CHANGED: &str = "This task changed; the list was refreshed";

    /// An app on a workspace whose group `Work/Tasks` holds the notes `A`
    /// (open) and `B`, with the group's task view showing and today pinned
    /// to Thursday 19 November 2026.
    fn fixture_with(engine: &str, a: &str, b: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        for (path, text) in [("Work/Tasks/A.md", a), ("Work/Tasks/B.md", b)] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let config = directory.path().join("config.conf");
        std::fs::write(&config, format!("tasks = on\neditor-engine = {engine}\n")).unwrap();
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
        let note = note(&app, "A");
        let target = app.note_target(note);
        assert!(app
            .dispatch(Command::OpenNote(target))
            .processed
            .disposition
            .committed());
        show(&mut app);
        (directory, app)
    }

    fn fixture() -> (tempfile::TempDir, App) {
        fixture_with("advanced", A, B)
    }

    fn show(app: &mut App) {
        let scope = Scope::Group(RelPath::new("Work/Tasks").unwrap());
        let _ = app.dispatch(Command::ShowTasks(scope));
        let _ = app.update(Message::Tick);
        assert!(app.task_view_has_keys());
    }

    fn note(app: &App, title: &str) -> wren_core::model::NoteId {
        (app.model().unwrap().notes.iter())
            .find(|note| note.title == title)
            .unwrap()
            .id
    }

    fn on_disk(directory: &tempfile::TempDir, title: &str) -> String {
        let path = directory
            .path()
            .join(format!("workspace/Work/Tasks/{title}.md"));
        std::fs::read_to_string(path).unwrap()
    }

    fn draft(app: &App) -> String {
        app.session
            .as_ref()
            .unwrap()
            .document()
            .unwrap()
            .text
            .clone()
    }

    /// The entry of the row whose description is `description`.
    fn entry(app: &App, description: &str) -> usize {
        let listing = &app.task_list.listing;
        listing
            .rows()
            .find(|&row| listing.record(row).unwrap().description == description)
            .unwrap_or_else(|| panic!("no row {description:?}"))
    }

    fn rows(app: &App) -> Vec<String> {
        let listing = &app.task_list.listing;
        (listing.rows())
            .map(|row| listing.record(row).unwrap().description.clone())
            .collect()
    }

    fn press(key: Key, modifiers: Modifiers, text: Option<&str>) -> Press {
        Press {
            key,
            physical: Physical::Unidentified(NativeCode::Unidentified),
            modifiers,
            text: text.map(str::to_owned),
        }
    }

    fn letter(text: &str) -> Press {
        press(Key::Character(text.into()), Modifiers::empty(), Some(text))
    }

    fn space() -> Press {
        press(Key::Named(Named::Space), Modifiers::empty(), Some(" "))
    }

    fn escape() -> Press {
        press(Key::Named(Named::Escape), Modifiers::empty(), None)
    }

    /// The chord the keymap binds to `action`, with the key `key`.
    fn chord(app: &App, action: &str, key: Key) -> Press {
        [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO]
            .into_iter()
            .map(|modifiers| press(key.clone(), modifiers, None))
            .find(|press| {
                crate::keys::shortcut(&press.key, press.physical, press.modifiers)
                    .is_some_and(|accel| app.preferences.keymap.action(&accel) == Some(action))
            })
            .unwrap_or_else(|| panic!("{action} is bound to no chord of {key:?}"))
    }

    fn undo_key(app: &App) -> Press {
        chord(app, "edit.undo", Key::Character("z".into()))
    }

    fn toggle_key(app: &App) -> Press {
        chord(app, "markdown.toggle_task", Key::Named(Named::Enter))
    }

    /// One batch of presses: the view's key wrapper reports each, then
    /// the app hears its own copies of Escape and the shortcuts.
    fn batch(app: &mut App, presses: &[Press]) {
        let serial = app.task_list.next_serial();
        in_batch(app, serial, presses);
    }

    fn in_batch(app: &mut App, serial: u64, presses: &[Press]) {
        for press in presses {
            let _ = app.update(Message::TaskView(Event::Key(ListKey {
                press: press.clone(),
                serial,
                after_press: false,
            })));
        }
        for press in presses {
            if press.is_plain_escape() {
                let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
            } else if let Some(accel) =
                crate::keys::shortcut(&press.key, press.physical, press.modifiers)
            {
                let _ = app.update(Message::Key(Input::Accel(accel)));
            }
        }
    }

    fn click(app: &mut App, click: Click) {
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Click { serial, click }));
    }

    fn select(app: &mut App, description: &str) {
        let entry = entry(app, description);
        let generation = app.task_list.generation;
        click(app, Click::Select { generation, entry });
        assert_eq!(app.selected_row(), Some(entry));
    }

    fn cell(app: &mut App, description: &str, cell: Cell) {
        let entry = entry(app, description);
        let generation = app.task_list.generation;
        click(
            app,
            Click::Cell {
                generation,
                entry,
                cell,
            },
        );
    }

    fn right_click(app: &mut App, description: &str) {
        let entry = entry(app, description);
        let generation = app.task_list.generation;
        click(app, Click::RowMenu { generation, entry });
    }

    fn notice(app: &App) -> Option<String> {
        app.notice().map(|shown| shown.notice.text())
    }

    fn menu_kind(app: &App) -> Option<MenuKind> {
        match &app.task_list.menu.as_ref()?.purpose {
            MenuFor::Row { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    fn menu_labels(app: &App) -> Vec<String> {
        let menu = app.task_list.menu.as_ref().expect("a menu is open");
        menu.rows().iter().map(|row| row.label.clone()).collect()
    }

    fn choose(app: &mut App, label: &str) {
        let menu = app.task_list.menu.as_ref().expect("a menu is open");
        let row = (menu.rows().iter())
            .find(|row| row.label.starts_with(label))
            .unwrap_or_else(|| panic!("no row {label:?}"));
        let event = host_menu::Event::Choose {
            instance: menu.instance(),
            row: row.id,
        };
        click(app, Click::Menu(event));
    }

    fn undo_by_notice(app: &mut App) {
        let serial = app.notice().expect("a message").serial;
        let _ = app.undo_notice(serial);
        let _ = app.update(Message::Tick);
    }

    #[test]
    fn each_key_writes_a_closed_note_with_a_message_and_undo() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        let medium = Priority::Medium.glyph();
        let low = Priority::Low.glyph();
        let cases: [(Press, String, &str); 4] = [
            (
                space(),
                "- [x] Order lunch ✅ 2026-11-19".into(),
                "Done · wrote line 3 of Work/Tasks/B.md",
            ),
            (
                toggle_key(&app),
                "- [x] Order lunch ✅ 2026-11-19".into(),
                "Done · wrote line 3 of Work/Tasks/B.md",
            ),
            (
                letter("+"),
                format!("- [ ] Order lunch {medium}"),
                "Priority raised · wrote line 3 of Work/Tasks/B.md",
            ),
            (
                letter("-"),
                format!("- [ ] Order lunch {low}"),
                "Priority lowered · wrote line 3 of Work/Tasks/B.md",
            ),
        ];
        for (key, line, message) in cases {
            batch(&mut app, std::slice::from_ref(&key));
            assert_eq!(on_disk(&directory, "B"), B.replace(LUNCH, &line), "{key:?}");
            assert_eq!(notice(&app).as_deref(), Some(message), "{key:?}");
            // The message's Undo, then the same key and ⌘Z in the view.
            undo_by_notice(&mut app);
            assert_eq!(on_disk(&directory, "B"), B, "{key:?}");
            assert_eq!(notice(&app), None);
            batch(&mut app, std::slice::from_ref(&key));
            assert_eq!(on_disk(&directory, "B"), B.replace(LUNCH, &line), "{key:?}");
            let undo = undo_key(&app);
            batch(&mut app, &[undo]);
            assert_eq!(on_disk(&directory, "B"), B, "{key:?}");
            assert_eq!(selected_description(&app), "Order lunch");
        }
        // Undone to the bottom, ⌘Z says so.
        let undo = undo_key(&app);
        batch(&mut app, &[undo]);
        assert_eq!(notice(&app).as_deref(), Some("Nothing to undo here."));
        // The open note was never touched.
        assert_eq!(draft(&app), A);
    }

    fn selected_description(app: &App) -> String {
        let entry = app.selected_row().expect("a row is selected");
        app.task_list
            .listing
            .record(entry)
            .unwrap()
            .description
            .clone()
    }

    #[test]
    fn a_write_to_the_open_note_goes_through_its_editor_on_both_engines() {
        for engine in ["classic", "advanced"] {
            let (directory, mut app) = fixture_with(engine, A, B);
            let caret = app.editor.cursor();
            select(&mut app, "Book the room");
            batch(&mut app, &[space()]);
            let done = A.replace("- [ ] Book the room", "- [x] Book the room ✅ 2026-11-19");
            assert_eq!(draft(&app), done, "{engine}");
            assert_eq!(on_disk(&directory, "A"), A, "a draft until autosave");
            assert_eq!(
                notice(&app).as_deref(),
                Some("Done · wrote line 4 of Work/Tasks/A.md")
            );
            assert_eq!(app.editor.cursor(), caret, "{engine}");
            let undo = undo_key(&app);
            batch(&mut app, &[undo]);
            assert_eq!(draft(&app), A, "{engine}");
            assert_eq!(app.editor.cursor(), caret, "{engine}");
            // The editor's own history holds the write and its undo as
            // steps of its own: undone there, the write is back.
            let _ = app.dispatch(Command::Run(CommandId::Back));
            assert_eq!(app.surface(), super::super::board::Surface::Text);
            let _ = app.edit(wren_editor::Message::Undo);
            assert_eq!(draft(&app), done, "{engine}");
        }
    }

    #[test]
    fn a_click_on_the_box_cycles_and_space_toggles() {
        let (directory, mut app) =
            fixture_with("advanced", A, "# B\n\n- [ ] Order lunch\n- [?] Ask\n");
        let b = |line: &str| format!("# B\n\n{line}\n- [?] Ask\n");
        for (line, message) in [
            ("- [/] Order lunch", "Marked doing"),
            ("- [x] Order lunch ✅ 2026-11-19", "Marked done"),
            ("- [ ] Order lunch", "Marked open"),
        ] {
            cell(&mut app, "Order lunch", Cell::Box);
            assert_eq!(on_disk(&directory, "B"), b(line));
            assert!(notice(&app).unwrap().starts_with(message), "{message}");
            assert_eq!(selected_description(&app), "Order lunch");
        }
        batch(&mut app, &[space()]);
        assert_eq!(
            on_disk(&directory, "B"),
            b("- [x] Order lunch ✅ 2026-11-19")
        );
        assert!(notice(&app).unwrap().starts_with("Done"));
        batch(&mut app, &[space()]);
        assert_eq!(on_disk(&directory, "B"), b(LUNCH));
        assert!(notice(&app).unwrap().starts_with("Reopened"));
        // A box of no status of its own is selected, never cycled.
        let before = notice(&app);
        cell(&mut app, "Ask", Cell::Box);
        assert_eq!(selected_description(&app), "Ask");
        assert_eq!(on_disk(&directory, "B"), b(LUNCH));
        assert_eq!(notice(&app), before);
    }

    #[test]
    fn a_stale_row_is_refused_and_the_list_shows_the_note_as_it_is() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        let theirs = B.replace("Order lunch", "Order dinner");
        std::fs::write(directory.path().join("workspace/Work/Tasks/B.md"), &theirs).unwrap();
        assert!(
            rows(&app).contains(&"Order lunch".to_owned()),
            "not yet read"
        );
        batch(&mut app, &[space()]);
        assert_eq!(on_disk(&directory, "B"), theirs);
        assert_eq!(notice(&app).as_deref(), Some(CHANGED));
        assert_eq!(app.banner, None, "said once, in the message");
        assert!(rows(&app).contains(&"Order dinner".to_owned()));
        assert!(!rows(&app).contains(&"Order lunch".to_owned()));
    }

    #[test]
    fn rows_written_stay_put_until_r() {
        let (directory, mut app) = fixture();
        assert_eq!(
            rows(&app),
            ["Order lunch", "Mail the deck", "Book the room"]
        );
        // Done sorts below open work, but the row stays where it stood,
        // and a second write to it keeps it there.
        select(&mut app, "Mail the deck");
        batch(&mut app, &[space()]);
        assert_eq!(
            rows(&app),
            ["Order lunch", "Mail the deck", "Book the room"]
        );
        assert_eq!(selected_description(&app), "Mail the deck");
        batch(&mut app, &[letter("+")]);
        assert_eq!(
            rows(&app),
            ["Order lunch", "Mail the deck", "Book the room"]
        );
        assert!(draft(&app).contains("- [x] Mail the deck"));
        // Only open work shown: a row done stays until R.
        let open = super::super::task_list::CHIPS
            .iter()
            .position(|status| *status == StatusType::Open)
            .unwrap();
        click(&mut app, Click::Control(Control::Chip(open)));
        assert_eq!(rows(&app), ["Order lunch", "Book the room"]);
        select(&mut app, "Order lunch");
        batch(&mut app, &[space()]);
        assert_eq!(rows(&app), ["Order lunch", "Book the room"]);
        assert!(on_disk(&directory, "B").contains("- [x] Order lunch"));
        batch(&mut app, &[letter("r")]);
        assert_eq!(rows(&app), ["Book the room"]);
        // Without the chip, R puts the rows written back in order.
        click(&mut app, Click::Control(Control::Chip(open)));
        select(&mut app, "Book the room");
        batch(&mut app, &[space()]);
        assert_eq!(
            rows(&app),
            ["Order lunch", "Book the room", "Mail the deck"]
        );
        batch(&mut app, &[letter("R")]);
        assert_eq!(
            rows(&app),
            ["Order lunch", "Mail the deck", "Book the room"]
        );
        assert_eq!(selected_description(&app), "Book the room");
    }

    #[test]
    fn undo_in_the_view_is_the_views_and_in_the_editor_the_editors() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        batch(&mut app, &[space()]);
        assert!(on_disk(&directory, "B").contains("[x]"));
        let _ = app.dispatch(Command::Run(CommandId::Back));
        assert_eq!(app.surface(), super::super::board::Surface::Text);
        // ⌘Z over the note is the editor's: the closed note stays written.
        let key = undo_key(&app);
        let accel = crate::keys::shortcut(&key.key, key.physical, key.modifiers).unwrap();
        let _ = app.update(Message::Key(Input::Accel(accel)));
        assert!(on_disk(&directory, "B").contains("[x]"));
        assert!(app.can_undo_task_write());
        let _ = app.dispatch(Command::Run(CommandId::Forward));
        let _ = app.update(Message::Tick);
        assert!(app.task_view_has_keys());
        let undo = undo_key(&app);
        batch(&mut app, &[undo]);
        assert_eq!(on_disk(&directory, "B"), B);
    }

    #[test]
    fn the_menus_open_by_key_and_click_and_write_their_row() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        // S: the status menu under the box; a key of its own writes.
        batch(&mut app, &[letter("s")]);
        assert_eq!(menu_kind(&app), Some(MenuKind::Status));
        assert_eq!(menu_labels(&app), ["Open", "Doing", "Done", "Cancelled"]);
        batch(&mut app, &[letter("/")]);
        assert!(app.task_list.menu.is_none());
        assert_eq!(on_disk(&directory, "B"), B.replace("[ ]", "[/]"));
        assert_eq!(
            notice(&app).as_deref(),
            Some("Marked doing · wrote line 3 of Work/Tasks/B.md")
        );
        // P, then H: High.
        batch(&mut app, &[letter("p")]);
        assert_eq!(menu_kind(&app), Some(MenuKind::Priority));
        batch(&mut app, &[letter("h")]);
        let high = Priority::High.glyph();
        assert_eq!(
            on_disk(&directory, "B"),
            format!("# B\n\n- [/] Order lunch {high}\n")
        );
        assert!(notice(&app).unwrap().starts_with("High priority"));
        // A click on the priority opens its menu; a row clicked clears it.
        cell(&mut app, "Order lunch", Cell::Priority);
        choose(&mut app, "None");
        assert_eq!(on_disk(&directory, "B"), B.replace("[ ]", "[/]"));
        assert!(notice(&app).unwrap().starts_with("Priority cleared"));
        // A right-click: the task menu, the editor's rows and the view's.
        let entry = entry(&app, "Order lunch");
        let generation = app.task_list.generation;
        click(&mut app, Click::RowMenu { generation, entry });
        assert_eq!(menu_kind(&app), Some(MenuKind::Task));
        assert_eq!(
            menu_labels(&app),
            [
                "Open",
                "Doing",
                "Done",
                "Cancelled",
                "Set due date…",
                "Priority…",
                "Edit task…",
                "Open the note at this line",
                "Hide tasks in B",
            ]
        );
        // Its Priority… row opens the priority menu for the same row.
        batch(&mut app, &[letter("p")]);
        assert_eq!(menu_kind(&app), Some(MenuKind::Priority));
        batch(&mut app, &[escape()]);
        assert!(app.task_list.menu.is_none());
        // H hides the note's tasks, with Undo.
        right_click(&mut app, "Order lunch");
        batch(&mut app, &[letter("h")]);
        assert!(on_disk(&directory, "B").starts_with("---\ntasks: off\n---\n"));
        assert!(!rows(&app).contains(&"Order lunch".to_owned()));
        assert!(notice(&app).unwrap().starts_with("Tasks in B hidden"));
        undo_by_notice(&mut app);
        assert_eq!(on_disk(&directory, "B"), B.replace("[ ]", "[/]"));
        assert!(rows(&app).contains(&"Order lunch".to_owned()));
        // O opens the note at the task's line.
        select(&mut app, "Order lunch");
        right_click(&mut app, "Order lunch");
        batch(&mut app, &[letter("o")]);
        assert_eq!(app.surface(), super::super::board::Surface::Text);
        assert_eq!(app.selected_note, Some(note(&app, "B")));
        assert_eq!(app.editor.cursor().position.line, 2);
    }

    #[test]
    fn the_calendar_and_the_form_open_over_the_row_and_write_it() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        // D: the calendar on today; typed and picked, the date is written.
        batch(&mut app, &[letter("d")]);
        assert!(app.calendar.is_some());
        assert!(!app.task_view_has_keys());
        let _ = app.update(Message::Calendar(CalendarEvent::Field(FieldEvent::Typed(
            "fri".into(),
        ))));
        let _ = app.update(Message::Calendar(CalendarEvent::Field(FieldEvent::Enter)));
        assert!(app.calendar.is_none());
        assert!(app.task_view_has_keys());
        assert_eq!(
            on_disk(&directory, "B"),
            B.replace(LUNCH, "- [ ] Order lunch 📅 2026-11-20")
        );
        assert_eq!(
            notice(&app).as_deref(),
            Some("Due Fri 20 Nov · wrote line 3 of Work/Tasks/B.md")
        );
        // A click on the due date opens it there; Clear clears it.
        cell(&mut app, "Order lunch", Cell::Due);
        assert!(app.calendar.as_ref().unwrap().has_due());
        let _ = app.update(Message::Calendar(CalendarEvent::Clear));
        assert_eq!(on_disk(&directory, "B"), B);
        assert!(notice(&app).unwrap().starts_with("Due date cleared"));
        // Escape writes nothing and gives the list the keyboard.
        batch(&mut app, &[letter("d")]);
        let _ = app.update(Message::Calendar(CalendarEvent::Field(FieldEvent::Escape)));
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(app.calendar.is_none() && app.task_view_has_keys());
        assert_eq!(on_disk(&directory, "B"), B);
        // E: the form over the row; saved, it writes, with a message.
        batch(&mut app, &[letter("e")]);
        assert!(app.task_form.is_some());
        let _ = app.update(Message::TaskForm(FormEvent::Description(
            "Order tea".into(),
        )));
        let _ = app.update(Message::TaskForm(FormEvent::Save));
        assert!(app.task_form.is_none() && app.task_view_has_keys());
        assert_eq!(on_disk(&directory, "B"), B.replace("lunch", "tea"));
        assert_eq!(
            notice(&app).as_deref(),
            Some("Saved · wrote line 3 of Work/Tasks/B.md")
        );
        // Cancelled, it writes nothing.
        batch(&mut app, &[letter("E")]);
        let _ = app.update(Message::TaskForm(FormEvent::Description(
            "Order cake".into(),
        )));
        let _ = app.update(Message::TaskForm(FormEvent::Cancel));
        assert!(app.task_form.is_none());
        assert_eq!(on_disk(&directory, "B"), B.replace("lunch", "tea"));
    }

    #[test]
    fn a_key_that_opens_something_drops_the_rest_of_its_batch() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        // [S, Escape] and [S, x]: the menu opens and stays, nothing written.
        for (rest, what) in [(escape(), "Escape"), (letter("x"), "x")] {
            batch(&mut app, &[letter("s"), rest]);
            assert!(app.task_list.menu.is_some(), "{what}");
            assert_eq!(on_disk(&directory, "B"), B, "{what}");
            batch(&mut app, &[escape()]);
            assert!(app.task_list.menu.is_none(), "{what}");
        }
        // [D, Escape]: the calendar opens and stays.
        batch(&mut app, &[letter("d"), escape()]);
        assert!(app.calendar.is_some());
        let _ = app.update(Message::Calendar(CalendarEvent::Field(FieldEvent::Escape)));
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(app.calendar.is_none());
        // [E, Escape]: the form opens and stays.
        batch(&mut app, &[letter("e"), escape()]);
        assert!(app.task_form.is_some());
        let _ = app.update(Message::TaskForm(FormEvent::Cancel));
        // A click on the box and Space in one batch: the click alone.
        let serial = app.task_list.next_serial();
        let generation = app.task_list.generation;
        let entry = entry(&app, "Order lunch");
        let _ = app.update(Message::TaskView(Event::Click {
            serial,
            click: Click::Cell {
                generation,
                entry,
                cell: Cell::Box,
            },
        }));
        in_batch(&mut app, serial, &[space()]);
        assert_eq!(on_disk(&directory, "B"), B.replace("[ ]", "[/]"));
        // Nothing of all that is left to swallow a later Escape.
        batch(&mut app, &[letter("s")]);
        batch(&mut app, &[escape()]);
        assert!(app.task_list.menu.is_none());
    }
}
