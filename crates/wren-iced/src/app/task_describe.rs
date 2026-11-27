//! Editing a task's description in place in the task view
//! (discovery/tasks.md §9; discovery/tasks-build.md item 10.5).
//!
//! - **Opening.** F2 on the selected row, while the list has the keyboard
//!   (as file managers rename), or a double-click on a row's description,
//!   turns the description into a field ([`crate::line_field`]) holding it
//!   as written: the text before the line's first token, `@owner`s
//!   included, which is what `Change::SetDescription` replaces. F2 is the
//!   view's own key there, whatever the keymap gives it elsewhere. The
//!   rest of the batch F2 came in, typed before the field could be seen,
//!   is dropped, and so are the app's copies of its keys.
//! - **Enter** writes the field's text as the row's description through
//!   [`App::write_view_row`], one undo step with a message with Undo,
//!   the row held where it stands; a row whose task changed since the list
//!   was built is refused and the list refreshed. An empty description
//!   (white space alone) is refused with a message, and the field stays.
//! - **Escape** closes the field, writing nothing, and the list keeps the
//!   keyboard. The field's plain Escape is one credit of the one Escape
//!   contract (`App::take_escape_claim`), through the view's copies.
//! - **Blur cancels.** The field losing the keyboard inside the window (a
//!   click elsewhere, the palette or a dialog, another column) closes it
//!   and writes nothing: a write is only ever what Enter asks for. The
//!   window losing the keyboard is no blur; the field is there when it
//!   comes back.
//! - **Tab** does nothing: the keyboard stays in the field.
//! - **One at a time.** Opening a menu, the calendar, the form, a filter,
//!   another scope or leaving the view, and any click in the view, ends
//!   the edit first, writing nothing (`App::sync_task_list` holds the rest).
//! - **The app's shortcuts** stay the app's while the field is open, and
//!   run at their place in the batch, as the field reports them: one that
//!   takes the keyboard from the view (the palette, a dialog, another
//!   surface) ends the edit there, writing nothing, so an Enter after it
//!   in the batch writes nothing, and the app's copy is spent. The view's
//!   own chords (its toggle and undo) and the editor's (the field's own
//!   edits) run nothing: `edit.undo`'s chord in the field never undoes a
//!   task write. Keys after Enter or Escape in the batch that closed the
//!   field are dropped, their copies too.
//! - **One field, on one row.** The field is drawn on the one row it was
//!   opened on, under its group (a task under two owners shows twice), and
//!   follows that task through each rebuild of the list by its note and
//!   its line's text, never to a twin that was a task of its own before;
//!   when the list no longer shows it there, a rebuild moves it out of the
//!   rows built, or the user scrolls the list, the edit ends, writing
//!   nothing. The write itself
//!   expects the line the row listed, so a task changed since is refused.
//! - **Letters are the field's**, R included: the list's keys wait until
//!   the field closes.
//! - **An input method** composes in the field as in any of iced's text
//!   inputs; Enter while it composes is taken and does nothing. Closed by
//!   a click while one composes, what it commits next has no field to go
//!   to: the editor is not mounted under the view, and the list types
//!   nothing, so the calendar's hand-back to the editor has no place here.

use iced::{Element, Fill, Task};
use wren_core::keybind::{Accel, Mods};
use wren_core::tasks::TaskRecord;
use wren_markdown::task::Change;

use super::commands::CommandId;
use super::notice::{Notice, Part};
use super::task_list::{Entry, GroupKey, Listing, RowKey};
use super::task_list_view::AppCopy;
use super::task_rows::What;
use super::task_target::RowTarget;
use super::tasks_view::Event;
use super::{App, Message};
use crate::line_field::{line_field, LineEvent};
use crate::{chrome, fonts};

/// The field's widget id, which takes the keyboard when it opens.
pub(super) const FIELD: &str = "task-describe";

/// What an empty description's Enter says.
pub(super) const EMPTY: &str = "A task needs a description; nothing was written";

/// A row's description being edited in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Describe {
    /// The row's task, as listed when the field opened: what the write
    /// expects to find.
    pub target: RowTarget,
    /// The occurrence of the row being edited, followed through each
    /// rebuild of the listing: the task by note and source line, under the
    /// group it was opened in (a row under two owners shows twice).
    pub key: RowKey,
    pub group: GroupKey,
    /// Its entry in the listing as last built: the one place the field is
    /// drawn.
    pub entry: usize,
    /// Where the task's box is on its line, as the index found it.
    pub boxed: usize,
    /// The field's text.
    pub text: String,
    /// The field's number: what it reports from a field since gone is
    /// no one's.
    pub instance: u64,
    /// An input method is composing in the field, as it last said.
    pub composing: bool,
}

impl Describe {
    /// The entry of the occurrence being edited in `listing`, rebuilt: the
    /// same task (its note and its source line, nearest the line it was
    /// on) under the same group; `None` once it is gone from there.
    pub(super) fn find_in(&self, listing: &Listing) -> Option<usize> {
        let entry = listing.find(&self.key, Some(&self.group))?;
        (group_of(listing, entry) == Some(&self.group)).then_some(entry)
    }
}

/// The key of the group the row at `entry` is shown under.
fn group_of(listing: &Listing, entry: usize) -> Option<&GroupKey> {
    match listing.entries.get(entry)? {
        Entry::Row { group, .. } => Some(&listing.groups[*group].key),
        Entry::Header(_) => None,
    }
}

/// Whether `accel` is the key that edits a description: F2 alone.
pub(super) fn is_describe_key(accel: &Accel) -> bool {
    accel.key == "f2" && accel.mods == Mods::default()
}

impl App {
    /// F2 on the list: edit the selected row's description. Returns whether
    /// it stops its batch, which it does when the field opens.
    pub(super) fn describe_key(&mut self) -> (Task<Message>, bool) {
        if self.task_list.menu.is_some() || self.task_list.focus.is_some() {
            return (Task::none(), false);
        }
        let Some(entry) = (self.task_list.selected.as_ref())
            .map(|selected| selected.entry)
            .filter(|&entry| self.task_list.listing.record(entry).is_some())
        else {
            return (Task::none(), false);
        };
        let task = self.open_describe(entry);
        (task, self.task_list.describe.is_some())
    }

    /// Open the field on the row at `entry`, in place of any open, and give
    /// it the keyboard with the caret at the end of its text.
    pub(super) fn open_describe(&mut self, entry: usize) -> Task<Message> {
        let Some(record) = self.task_list.listing.record(entry) else {
            return Task::none();
        };
        let target = RowTarget::of(record);
        let key = RowKey::of(record);
        let boxed = record.box_at;
        let Some(group) = group_of(&self.task_list.listing, entry).cloned() else {
            return Task::none();
        };
        let Some(text) = super::task_menus::row_task(&target.expect, boxed)
            .map(|line| line.description_text().to_owned())
        else {
            return Task::none();
        };
        self.task_list.menu = None;
        self.task_list.focus = None;
        self.task_list.describes += 1;
        self.task_list.describe = Some(Describe {
            target,
            key,
            group,
            entry,
            boxed,
            text,
            instance: self.task_list.describes,
            composing: false,
        });
        let scroll = self.select_entry(entry);
        Task::batch([scroll, iced::widget::operation::focus(FIELD)])
    }

    /// Close the field, writing nothing. Returns whether one was open.
    pub(super) fn cancel_describe(&mut self) -> bool {
        self.task_list.describe.take().is_some()
    }

    /// Follow the occurrence being edited into the listing just built from
    /// `previous` (the records it was built from before), or close the
    /// field, writing nothing, once it is gone from it. A task that left
    /// its line is followed only to the one line of its note that reads as
    /// it did, when no other line read so before (that one is a twin, a
    /// task of its own, as for a held row), and the write goes there too:
    /// the field never shows on one line and writes another. Two such
    /// lines cannot be told apart, and close the field.
    pub(super) fn follow_describe(&mut self, previous: &[TaskRecord], scroll: f32) {
        let list = &mut self.task_list;
        let Some(describe) = &mut list.describe else {
            return;
        };
        let key = &describe.key;
        let twin = |record: &TaskRecord| record.note == key.note && record.source == key.source;
        let same: Vec<usize> = (list.listing.records.iter())
            .filter(|record| twin(record))
            .map(|record| record.line)
            .collect();
        if !same.contains(&key.line) {
            let others = (previous.iter())
                .filter(|record| twin(record) && record.line != key.line)
                .count();
            match same.as_slice() {
                &[line] if others == 0 => {
                    describe.key.line = line;
                    describe.target.line = line;
                }
                _ => {
                    list.describe = None;
                    return;
                }
            }
        }
        match describe.find_in(&list.listing) {
            Some(entry) => describe.entry = entry,
            None => list.describe = None,
        }
        // Moved out of the rows built at `scroll`, where the list stays, it
        // has no field to type in, and the list's keys wait while one is
        // open: the edit ends.
        let built = self.built_rows_at(scroll);
        if (self.task_list.describe.as_ref())
            .is_some_and(|describe| !built.contains(&describe.entry))
        {
            self.task_list.describe = None;
        }
    }

    /// Whether the field can still have the keyboard: the view has it, with
    /// no menu or control over the list.
    fn describe_holds(&self) -> bool {
        self.task_view_has_keys() && self.task_list.menu.is_none() && self.task_list.focus.is_none()
    }

    /// A shortcut the open field saw and did not take: the app's command
    /// for it runs here, at its place in the batch, and its copy is spent.
    /// One that takes the keyboard from the view (the palette, a dialog,
    /// another surface) closes the field first, writing nothing, so an
    /// Enter later in the batch writes nothing either. The view's own
    /// chords (its toggle and undo) and the editor's (the field's edits)
    /// run nothing.
    fn describe_shortcut(&mut self, accel: Accel) -> Task<Message> {
        self.task_list
            .copies
            .push_back((AppCopy::Shortcut(accel.clone()), true));
        let id = (self.preferences.keymap.action(&accel)).and_then(CommandId::parse);
        let Some(id) = id.filter(|id| {
            id.scope() == super::commands::Scope::App
                && !matches!(id, CommandId::ToggleTask | CommandId::EditUndo)
        }) else {
            return Task::none();
        };
        let task = self.run_shortcut(id);
        if !self.describe_holds() || self.task_view().is_none() {
            self.task_list.describe = None;
            self.task_list.stopped = true;
        }
        task
    }

    /// What the field numbered `instance` reported, in the batch `serial`.
    pub(super) fn describe_event(
        &mut self,
        serial: u64,
        instance: u64,
        event: LineEvent,
    ) -> Task<Message> {
        self.enter_batch(serial);
        let current = (self.task_list.describe.as_ref())
            .is_some_and(|describe| describe.instance == instance);
        // The app's key handling hears its own copy of each of these after
        // the batch: an Escape the field took or held, and a shortcut it
        // saw, whose command runs here, in its place, unless the field held
        // it or has closed. Each copy is spent.
        match &event {
            LineEvent::Escape | LineEvent::HeldEscape => {
                self.task_list.copies.push_back((AppCopy::Escape, true));
            }
            LineEvent::Shortcut { accel, held } => {
                if *held || !current {
                    (self.task_list.copies).push_back((AppCopy::Shortcut(accel.clone()), true));
                    return Task::none();
                }
                return self.describe_shortcut(accel.clone());
            }
            _ => {}
        }
        let Some(describe) =
            (self.task_list.describe.as_mut()).filter(|describe| describe.instance == instance)
        else {
            return Task::none();
        };
        match event {
            LineEvent::Typed(text) => describe.text = text,
            LineEvent::Composing(composing) => describe.composing = composing,
            LineEvent::Enter => return self.finish_describe(),
            LineEvent::Escape => {
                self.task_list.describe = None;
                self.task_list.stopped = true;
                return self.task_view_focus();
            }
            // Whatever took the keyboard keeps it.
            LineEvent::Blur => self.task_list.describe = None,
            LineEvent::HeldEscape | LineEvent::Shortcut { .. } => {}
        }
        Task::none()
    }

    /// Enter in the field: write its text as the row's description, or
    /// refuse an empty one and keep the field.
    fn finish_describe(&mut self) -> Task<Message> {
        let Some(describe) = self.task_list.describe.take() else {
            return Task::none();
        };
        if describe.text.trim().is_empty() {
            self.task_list.describe = Some(describe);
            self.notify(Notice::new(vec![Part::Text(EMPTY.into())]));
            return Task::none();
        }
        self.task_list.stopped = true;
        let written = self.write_view_row(
            &describe.target,
            Some(describe.boxed),
            Change::SetDescription(describe.text),
            What::Description,
        );
        Task::batch([written.gateway.task, self.task_view_focus()])
    }

    /// The field over the row `describe` edits, in the batch `serial`.
    pub(super) fn describe_field(&self, describe: &Describe, serial: u64) -> Element<'_, Message> {
        let definition = self.definition;
        let instance = describe.instance;
        line_field(
            &describe.text,
            instance,
            move |input| {
                input
                    .id(FIELD)
                    .font(fonts::UI)
                    .size(13.5)
                    .padding([1, 4])
                    .width(Fill)
                    .style(move |_, status| chrome::input(definition, status, true))
            },
            move |event| {
                Message::TaskView(Event::Field {
                    serial,
                    instance,
                    event,
                })
            },
        )
        .field_keys(&self.preferences.field_keys)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::{Named, NativeCode, Physical};
    use iced::keyboard::{Key, Modifiers};
    use wren_core::fs_store::RelPath;
    use wren_core::ops::Session;
    use wren_markdown::date::Date;

    use super::super::commands::Command;
    use super::super::task_list_view::{Cell, Click, Control};
    use super::super::tasks_view::Scope;
    use super::super::Options;
    use super::*;
    use crate::keys::{Input, Nav};
    use crate::list_keys::{ListKey, Press};

    const A: &str = "# A\n\n- [ ] Mail the deck 📅 2026-12-01\n- [ ] Book the room\n";
    const B: &str = "# B\n\n- [ ] Order lunch @sam 📅 2026-12-02\n";
    const CHANGED: &str = "This task changed; the list was refreshed";

    /// An app on a workspace whose group `Work/Tasks` holds the notes `A`
    /// (open) and `B`, with the group's task view showing, today pinned
    /// to Thursday 19 November 2026, and `config` added to its settings.
    fn fixture_with(config: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        for (path, text) in [("Work/Tasks/A.md", A), ("Work/Tasks/B.md", B)] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let file = directory.path().join("config.conf");
        std::fs::write(
            &file,
            format!("tasks = on\neditor-engine = advanced\n{config}"),
        )
        .unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(file));
        app.workspace_mode = true;
        app.session = Some(Session::open_with(&root, app.session_options()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        app.pin_today(Date::new(2026, 11, 19).unwrap());
        let note = (app.model().unwrap().notes.iter())
            .find(|note| note.title == "A")
            .unwrap()
            .id;
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
        fixture_with("")
    }

    fn show(app: &mut App) {
        let scope = Scope::Group(RelPath::new("Work/Tasks").unwrap());
        let _ = app.dispatch(Command::ShowTasks(scope));
        let _ = app.update(Message::Tick);
        assert!(app.task_view_has_keys());
    }

    fn on_disk(directory: &tempfile::TempDir, title: &str) -> String {
        let path = directory
            .path()
            .join(format!("workspace/Work/Tasks/{title}.md"));
        std::fs::read_to_string(path).unwrap()
    }

    fn draft(app: &App) -> String {
        (app.session.as_ref().unwrap().document().unwrap().text).clone()
    }

    fn entry(app: &App, description: &str) -> usize {
        let listing = &app.task_list.listing;
        listing
            .rows()
            .find(|&row| {
                listing
                    .record(row)
                    .unwrap()
                    .description
                    .starts_with(description)
            })
            .unwrap_or_else(|| panic!("no row {description:?}"))
    }

    fn rows(app: &App) -> Vec<String> {
        let listing = &app.task_list.listing;
        (listing.rows())
            .map(|row| {
                let description = &listing.record(row).unwrap().description;
                description.trim_end_matches(" @sam").to_owned()
            })
            .collect()
    }

    fn press(key: Key, modifiers: Modifiers, text: Option<&str>) -> Press {
        Press {
            key,
            physical: Physical::Unidentified(NativeCode::Unidentified),
            modifiers,
            text: text.map(str::to_owned),
            repeat: false,
        }
    }

    fn letter(text: &str) -> Press {
        press(Key::Character(text.into()), Modifiers::empty(), Some(text))
    }

    fn space() -> Press {
        press(Key::Named(Named::Space), Modifiers::empty(), Some(" "))
    }

    fn f2() -> Press {
        press(Key::Named(Named::F2), Modifiers::empty(), None)
    }

    /// The press of the chord the keymap binds to `action`, with the key
    /// `key`.
    fn chord_press(app: &App, action: &str, key: Key) -> Press {
        [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO]
            .into_iter()
            .map(|modifiers| press(key.clone(), modifiers, None))
            .find(|press| {
                accel(press)
                    .is_some_and(|accel| app.preferences.keymap.action(&accel) == Some(action))
            })
            .unwrap_or_else(|| panic!("{action} is bound to no chord of {key:?}"))
    }

    fn accel(press: &Press) -> Option<Accel> {
        crate::keys::shortcut(&press.key, press.physical, press.modifiers)
    }

    fn chord(app: &App, action: &str, key: Key) -> Accel {
        accel(&chord_press(app, action, key)).unwrap()
    }

    fn undo_key(app: &App) -> Accel {
        chord(app, "edit.undo", Key::Character("z".into()))
    }

    /// One batch of presses on the list, while its key wrapper reports
    /// them: each report, then the app's copies of Escape and shortcuts.
    fn list_batch(app: &mut App, presses: &[Press]) {
        let serial = app.task_list.next_serial();
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

    /// One batch of what the open field reports, then the app's copies of
    /// the Escapes and shortcuts among it, as its key handling hears them.
    fn field_batch(app: &mut App, events: &[LineEvent]) {
        let serial = app.task_list.next_serial();
        let instance = app.task_list.describes;
        for event in events {
            let _ = app.update(Message::TaskView(Event::Field {
                serial,
                instance,
                event: event.clone(),
            }));
        }
        for event in events {
            match event {
                LineEvent::Escape | LineEvent::HeldEscape => {
                    let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
                }
                LineEvent::Shortcut { accel, .. } => {
                    let _ = app.update(Message::Key(Input::Accel(accel.clone())));
                }
                _ => {}
            }
        }
    }

    fn typed(text: &str) -> LineEvent {
        LineEvent::Typed(text.into())
    }

    fn click(app: &mut App, click: Click) {
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Click { serial, click }));
    }

    fn select(app: &mut App, description: &str) {
        let entry = entry(app, description);
        let generation = app.task_list.generation;
        click(app, Click::Select { generation, entry });
    }

    /// The second press of a double-click on a row's description: the row
    /// selected, then the edit, in one batch.
    fn double_click(app: &mut App, description: &str) {
        let entry = entry(app, description);
        let generation = app.task_list.generation;
        let serial = app.task_list.next_serial();
        for click in [
            Click::Select { generation, entry },
            Click::Describe { generation, entry },
        ] {
            let _ = app.update(Message::TaskView(Event::Click { serial, click }));
        }
    }

    fn field_text(app: &App) -> Option<String> {
        app.task_list
            .describe
            .as_ref()
            .map(|describe| describe.text.clone())
    }

    fn notice(app: &App) -> Option<String> {
        app.notice().map(|shown| shown.notice.text())
    }

    #[test]
    fn f2_opens_the_field_and_enter_writes_a_closed_note_with_undo() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        // The description as written: owners in, tokens out.
        assert_eq!(field_text(&app).as_deref(), Some("Order lunch @sam"));
        assert!(app.task_list.copies.is_empty(), "F2's copy was the view's");
        assert!(app.task_view_has_keys());
        field_batch(&mut app, &[typed("Order tea @sam"), LineEvent::Enter]);
        assert_eq!(field_text(&app), None);
        let after = B.replace("Order lunch", "Order tea");
        assert_eq!(on_disk(&directory, "B"), after);
        assert_eq!(
            notice(&app).as_deref(),
            Some("Description changed · wrote line 3 of Work/Tasks/B.md")
        );
        // The row stays where it stood, selected.
        let selected = app.task_list.selected.as_ref().unwrap().entry;
        assert_eq!(selected, entry(&app, "Order tea"));
        // One undo step: the message's Undo.
        let serial = app.notice().unwrap().serial;
        let _ = app.undo_notice(serial);
        assert_eq!(on_disk(&directory, "B"), B);
        // The open note was never touched.
        assert_eq!(draft(&app), A);
    }

    #[test]
    fn a_double_click_opens_the_field_and_enter_writes_the_open_note() {
        let (directory, mut app) = fixture();
        double_click(&mut app, "Book the room");
        assert_eq!(field_text(&app).as_deref(), Some("Book the room"));
        field_batch(&mut app, &[typed("Book the big room"), LineEvent::Enter]);
        let after = A.replace("Book the room", "Book the big room");
        assert_eq!(draft(&app), after);
        assert_eq!(on_disk(&directory, "A"), A, "a draft until autosave");
        assert_eq!(
            notice(&app).as_deref(),
            Some("Description changed · wrote line 4 of Work/Tasks/A.md")
        );
        // The undo chord in the view undoes it, one step.
        let undo = chord_press(&app, "edit.undo", Key::Character("z".into()));
        list_batch(&mut app, &[undo]);
        assert_eq!(draft(&app), A);
        // A double-click on another row while one edits moves the edit.
        double_click(&mut app, "Book the room");
        double_click(&mut app, "Mail the deck");
        assert_eq!(field_text(&app).as_deref(), Some("Mail the deck"));
        assert_eq!(draft(&app), A);
    }

    #[test]
    fn escape_and_a_blur_write_nothing() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order cake"), LineEvent::Escape]);
        assert_eq!(field_text(&app), None);
        assert_eq!(on_disk(&directory, "B"), B);
        assert!(app.task_view_has_keys(), "the list keeps the keyboard");
        assert!(app.task_list.copies.is_empty(), "one credit, taken");
        assert_eq!(notice(&app), None);
        // Nothing is left to swallow the next Escape: it closes a menu.
        list_batch(&mut app, &[letter("s")]);
        assert!(app.task_list.menu.is_some());
        list_batch(
            &mut app,
            &[press(Key::Named(Named::Escape), Modifiers::empty(), None)],
        );
        assert!(app.task_list.menu.is_none());
        // A blur: the field closes, and whatever took the keyboard keeps it.
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order cake"), LineEvent::Blur]);
        assert_eq!(field_text(&app), None);
        assert_eq!(on_disk(&directory, "B"), B);
        // An Escape an input method held closes nothing.
        list_batch(&mut app, &[f2()]);
        field_batch(
            &mut app,
            &[LineEvent::Composing(true), LineEvent::HeldEscape],
        );
        assert!(field_text(&app).is_some());
        assert!(app.task_list.copies.is_empty());
    }

    #[test]
    fn an_empty_description_is_refused_and_the_field_stays() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("   "), LineEvent::Enter]);
        assert_eq!(notice(&app).as_deref(), Some(EMPTY));
        assert_eq!(field_text(&app).as_deref(), Some("   "));
        assert_eq!(on_disk(&directory, "B"), B);
        field_batch(&mut app, &[typed("Order tea"), LineEvent::Enter]);
        assert_eq!(
            on_disk(&directory, "B"),
            B.replace("Order lunch @sam", "Order tea")
        );
    }

    #[test]
    fn a_stale_row_is_refused_and_the_list_shows_the_note_as_it_is() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        let theirs = B.replace("Order lunch", "Order dinner");
        std::fs::write(directory.path().join("workspace/Work/Tasks/B.md"), &theirs).unwrap();
        field_batch(&mut app, &[typed("Order tea"), LineEvent::Enter]);
        assert_eq!(on_disk(&directory, "B"), theirs);
        assert_eq!(notice(&app).as_deref(), Some(CHANGED));
        assert_eq!(field_text(&app), None);
        assert!(rows(&app).contains(&"Order dinner".to_owned()));
        assert!(!rows(&app).contains(&"Order lunch".to_owned()));
    }

    #[test]
    fn an_input_methods_commit_is_written_once_it_is_done() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order "), LineEvent::Composing(true)]);
        assert!(app.task_list.describe.as_ref().unwrap().composing);
        // The commit, as the text input types it, then Enter.
        field_batch(
            &mut app,
            &[
                typed("Order お茶"),
                LineEvent::Composing(false),
                LineEvent::Enter,
            ],
        );
        assert_eq!(
            on_disk(&directory, "B"),
            B.replace("Order lunch @sam", "Order お茶")
        );
    }

    #[test]
    fn the_batch_that_opens_or_closes_the_field_drops_the_rest() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        // [F2, x], [F2, Space]: the field opens as it was, nothing typed
        // into it and nothing toggled.
        for rest in [letter("x"), space()] {
            list_batch(&mut app, &[f2(), rest]);
            assert_eq!(field_text(&app).as_deref(), Some("Order lunch @sam"));
            assert_eq!(on_disk(&directory, "B"), B);
            field_batch(&mut app, &[LineEvent::Escape]);
        }
        // [F2, the undo chord]: the chord's copy is dropped.
        let toggle = chord(&app, "markdown.toggle_task", Key::Named(Named::Enter));
        list_batch(&mut app, &[space()]);
        assert!(on_disk(&directory, "B").contains("[x]"));
        let undo = undo_key(&app);
        let undo_press = chord_press(&app, "edit.undo", Key::Character("z".into()));
        list_batch(&mut app, &[f2(), undo_press]);
        assert!(on_disk(&directory, "B").contains("[x]"), "not undone");
        field_batch(&mut app, &[LineEvent::Escape]);
        list_batch(&mut app, &[space()]);
        assert_eq!(on_disk(&directory, "B"), B);
        // [Enter, Space]: what the list might hear after the close in the
        // batch is dropped, so Space toggles nothing.
        list_batch(&mut app, &[f2()]);
        let serial = app.task_list.next_serial();
        let instance = app.task_list.describes;
        let _ = app.update(Message::TaskView(Event::Field {
            serial,
            instance,
            event: typed("Order tea @sam"),
        }));
        let _ = app.update(Message::TaskView(Event::Field {
            serial,
            instance,
            event: LineEvent::Enter,
        }));
        let _ = app.update(Message::TaskView(Event::Key(ListKey {
            press: space(),
            serial,
            after_press: false,
        })));
        let tea = B.replace("Order lunch", "Order tea");
        assert_eq!(on_disk(&directory, "B"), tea);
        // [Enter, ⌘↵] and [Enter, ⌘Z]: the chords the field saw after its
        // close are dropped, so neither toggles nor undoes the write.
        list_batch(&mut app, &[f2()]);
        field_batch(
            &mut app,
            &[
                typed("Order tea now @sam"),
                LineEvent::Enter,
                LineEvent::Shortcut {
                    accel: toggle,
                    held: true,
                },
                LineEvent::Shortcut {
                    accel: undo.clone(),
                    held: true,
                },
            ],
        );
        assert_eq!(
            on_disk(&directory, "B"),
            B.replace("Order lunch", "Order tea now")
        );
        assert!(app.task_list.copies.is_empty());
    }

    #[test]
    fn the_undo_chord_in_the_open_field_is_the_fields_in_alt_mode_too() {
        for config in ["", "linux-modifier = alt\n"] {
            let (directory, mut app) = fixture_with(config);
            select(&mut app, "Order lunch");
            list_batch(&mut app, &[space()]);
            assert!(on_disk(&directory, "B").contains("[x]"), "{config}");
            list_batch(&mut app, &[f2()]);
            // `edit.paste`'s chord pastes in the field (the text input's,
            // `line_field.rs`), and its copy does nothing of the view's;
            // `edit.undo`'s undoes no task write while the field is open.
            let paste = chord(&app, "edit.paste", Key::Character("v".into()));
            let undo = undo_key(&app);
            field_batch(
                &mut app,
                &[
                    LineEvent::Shortcut {
                        accel: paste,
                        held: false,
                    },
                    typed("Order lunch @sam tea"),
                    LineEvent::Shortcut {
                        accel: undo,
                        held: false,
                    },
                ],
            );
            assert_eq!(field_text(&app).as_deref(), Some("Order lunch @sam tea"));
            assert!(on_disk(&directory, "B").contains("[x]"), "{config}");
            assert!(app.task_list.copies.is_empty(), "{config}");
            // The chord, then Enter, in one batch: the app's copy of the
            // chord comes after the write, and is still the field's.
            let undo = undo_key(&app);
            field_batch(
                &mut app,
                &[
                    LineEvent::Shortcut {
                        accel: undo,
                        held: false,
                    },
                    LineEvent::Enter,
                ],
            );
            assert!(
                on_disk(&directory, "B").contains("- [x] Order lunch @sam tea 📅"),
                "{config}"
            );
        }
    }

    #[test]
    fn a_menu_a_filter_r_or_leaving_the_view_ends_the_edit_first() {
        let (directory, mut app) = fixture();
        let open = |app: &mut App| {
            select(app, "Order lunch");
            list_batch(app, &[f2()]);
            field_batch(app, &[typed("Order cake")]);
            assert!(field_text(app).is_some());
        };
        // A click on a cell: the priority menu, and no edit.
        open(&mut app);
        let generation = app.task_list.generation;
        let lunch = entry(&app, "Order lunch");
        click(
            &mut app,
            Click::Cell {
                generation,
                entry: lunch,
                cell: Cell::Priority,
            },
        );
        assert!(app.task_list.menu.is_some());
        assert_eq!(field_text(&app), None);
        list_batch(
            &mut app,
            &[press(Key::Named(Named::Escape), Modifiers::empty(), None)],
        );
        // A filter.
        open(&mut app);
        click(&mut app, Click::Control(Control::HideOld));
        assert_eq!(field_text(&app), None);
        click(&mut app, Click::Control(Control::HideOld));
        // The palette.
        open(&mut app);
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        let _ = app.update(Message::Tick);
        assert_eq!(field_text(&app), None);
        let _ = app.update(Message::Escape);
        // Back leaves the view.
        open(&mut app);
        let _ = app.dispatch(Command::Run(CommandId::Back));
        let _ = app.update(Message::Tick);
        assert_eq!(field_text(&app), None);
        assert_eq!(on_disk(&directory, "B"), B);
        assert_eq!(draft(&app), A);
        // The view again: nothing of the edit is left.
        let _ = app.dispatch(Command::Run(CommandId::Forward));
        let _ = app.update(Message::Tick);
        assert_eq!(field_text(&app), None);
    }

    /// A chord the keymap binds to `action`: a letter with Ctrl, Alt or
    /// the logo key, and Shift or not.
    fn bound(app: &App, action: &str) -> Accel {
        let mods = [
            Modifiers::CTRL,
            Modifiers::ALT,
            Modifiers::LOGO,
            Modifiers::CTRL | Modifiers::SHIFT,
            Modifiers::ALT | Modifiers::SHIFT,
            Modifiers::LOGO | Modifiers::SHIFT,
        ];
        ('a'..='z')
            .flat_map(|letter| mods.map(|modifiers| (letter, modifiers)))
            .filter_map(|(letter, modifiers)| {
                accel(&press(
                    Key::Character(letter.to_string().into()),
                    modifiers,
                    None,
                ))
            })
            .find(|accel| app.preferences.keymap.action(accel) == Some(action))
            .unwrap_or_else(|| panic!("{action} is bound to no letter chord"))
    }

    #[test]
    fn a_shortcut_that_takes_the_keyboard_ends_the_edit_at_its_place_in_the_batch() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea @sam")]);
        // The palette's chord, then Enter, in one batch: the palette opens,
        // the edit ends before Enter, and Enter writes nothing.
        let palette = bound(&app, "palette.commands");
        field_batch(
            &mut app,
            &[
                LineEvent::Shortcut {
                    accel: palette,
                    held: false,
                },
                LineEvent::Enter,
            ],
        );
        assert!(app.palette.is_some(), "the chord ran, once");
        assert_eq!(field_text(&app), None);
        assert_eq!(on_disk(&directory, "B"), B);
        assert!(app.task_list.copies.is_empty(), "its copy was spent");
        // A chord the field held (taken while an input method composed)
        // runs nothing.
        let _ = app.update(Message::Escape);
        let _ = app.update(Message::Tick);
        list_batch(&mut app, &[f2()]);
        let palette = bound(&app, "palette.commands");
        field_batch(
            &mut app,
            &[LineEvent::Shortcut {
                accel: palette,
                held: true,
            }],
        );
        assert!(app.palette.is_none());
        assert!(field_text(&app).is_some());
    }

    #[test]
    fn a_row_under_two_owners_edits_in_the_one_it_was_opened_in() {
        let (_directory, mut app) = fixture();
        std::fs::write(
            _directory.path().join("workspace/Work/Tasks/B.md"),
            "# B\n\n- [ ] Order lunch @sam @kim\n",
        )
        .unwrap();
        let _ = app.process_core(wren_core::ops::Op::Reconcile);
        app.change_filters(|filters| filters.group_by = wren_core::tasks_file::GroupBy::Owner);
        let _ = app.update(Message::Tick);
        let listing = &app.task_list.listing;
        let shown: Vec<usize> = (listing.rows())
            .filter(|&row| {
                listing
                    .record(row)
                    .unwrap()
                    .description
                    .starts_with("Order lunch")
            })
            .collect();
        assert_eq!(shown.len(), 2, "under each owner");
        let second = shown[1];
        let generation = app.task_list.generation;
        let serial = app.task_list.next_serial();
        for click in [
            Click::Select {
                generation,
                entry: second,
            },
            Click::Describe {
                generation,
                entry: second,
            },
        ] {
            let _ = app.update(Message::TaskView(Event::Click { serial, click }));
        }
        assert_eq!(app.task_list.describe.as_ref().unwrap().entry, second);
        // A rebuild keeps it on that occurrence.
        let _ = app.process_core(wren_core::ops::Op::Reconcile);
        list_batch(&mut app, &[]);
        assert_eq!(app.task_list.describe.as_ref().unwrap().entry, second);
    }

    #[test]
    fn an_edit_ends_when_its_task_leaves_the_line_for_another() {
        let (directory, mut app) = fixture_with("");
        let b = directory.path().join("workspace/Work/Tasks/B.md");
        std::fs::write(&b, "# B\n\n- [ ] Order lunch\n- [ ] Call Kim\n").unwrap();
        let _ = app.process_core(wren_core::ops::Op::Reconcile);
        let _ = app.update(Message::Tick);
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea")]);
        // Another program takes the line out: Call Kim moves onto it.
        std::fs::write(&b, "# B\n\n- [ ] Call Kim, today\n").unwrap();
        let _ = app.process_core(wren_core::ops::Op::Reconcile);
        let _ = app.update(Message::Tick);
        assert_eq!(
            field_text(&app),
            None,
            "the edit never moves to another task"
        );
        assert_eq!(on_disk(&directory, "B"), "# B\n\n- [ ] Call Kim, today\n");
        // A task that only moved down keeps its edit, and the write finds it.
        std::fs::write(&b, "# B\n\n- [ ] Order lunch\n").unwrap();
        let _ = app.process_core(wren_core::ops::Op::Reconcile);
        let _ = app.update(Message::Tick);
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea")]);
        std::fs::write(&b, "# B\n\nIntro, longer.\n\n- [ ] Order lunch\n").unwrap();
        let _ = app.process_core(wren_core::ops::Op::Reconcile);
        let _ = app.update(Message::Tick);
        assert_eq!(field_text(&app).as_deref(), Some("Order tea"));
        field_batch(&mut app, &[LineEvent::Enter]);
        assert_eq!(
            on_disk(&directory, "B"),
            "# B\n\nIntro, longer.\n\n- [ ] Order tea\n"
        );
    }

    /// Ring review, astra 3: a press on the message over the view is a
    /// press outside the field, which the message takes before the field
    /// hears of it: the edit ends before the message's own dismissal or
    /// Undo, and an Enter after it in the batch writes nothing.
    #[test]
    fn a_press_on_the_message_ends_the_edit_before_a_later_enter() {
        use iced::advanced::layout::{self, Layout};
        use iced::advanced::widget::Tree;
        use iced::advanced::Shell;
        use iced::mouse::{self as pointer, Button};
        use iced::{Event as Input, Length, Point, Rectangle, Size};

        let (directory, mut app) = fixture();
        // A message showing: a write to A's first task, and its Undo.
        select(&mut app, "Mail the deck");
        list_batch(&mut app, &[space()]);
        let serial = app.notice().unwrap().serial;
        let written = draft(&app);
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea @sam")]);
        // The message as drawn, and a press and release on its words.
        let mut frame: Element<'static, Message, iced::Theme, ()> =
            super::super::notice::frame_for_tests(
                serial,
                iced::widget::Space::new()
                    .width(Length::Fixed(200.0))
                    .height(Length::Fixed(20.0))
                    .into(),
                Some(
                    iced::widget::Space::new()
                        .width(Length::Fixed(40.0))
                        .height(Length::Fixed(16.0))
                        .into(),
                ),
                wren_core::theme::DARK,
            );
        let limits = layout::Limits::new(Size::ZERO, Size::new(600.0, 100.0));
        let viewport = Rectangle::new(Point::ORIGIN, Size::new(600.0, 100.0));
        let mut tree = Tree::new(&frame);
        let node = frame.as_widget_mut().layout(&mut tree, &(), &limits);
        // The message's words: the frame's message, its face, its line,
        // the line's first child.
        let mut words = Layout::new(&node);
        for step in [0, 0, 0, 0] {
            words = words.children().nth(step).unwrap();
        }
        let at = words.bounds().center();
        let mut send = |event: Input| {
            let mut messages = Vec::new();
            let mut shell = Shell::new(&mut messages);
            frame.as_widget_mut().update(
                &mut tree,
                &event,
                Layout::new(&node),
                pointer::Cursor::Available(at),
                &(),
                &mut iced::advanced::clipboard::Null,
                &mut shell,
                &viewport,
            );
            messages
        };
        let mut messages = send(Input::Mouse(pointer::Event::ButtonPressed(Button::Left)));
        messages.extend(send(Input::Mouse(pointer::Event::ButtonReleased(
            Button::Left,
        ))));
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Notice(super::super::notice::Event::Pressed(_)),
                    Message::Notice(super::super::notice::Event::Dismiss(_))
                ]
            ),
            "{messages:?}"
        );
        // The batch: the press, the release, then Enter in the field.
        let batch = app.task_list.next_serial();
        let instance = app.task_list.describes;
        for message in messages {
            let _ = app.update(message);
        }
        let _ = app.update(Message::TaskView(Event::Field {
            serial: batch,
            instance,
            event: LineEvent::Enter,
        }));
        assert_eq!(field_text(&app), None);
        assert_eq!(on_disk(&directory, "B"), B);
        assert_eq!(draft(&app), written, "and the message undid nothing");
    }

    /// Ring review, astra 1: the field shows where it writes. A task that
    /// moved, then a twin put where it was, leaves the twin alone; a move
    /// to two lines at once closes the field.
    #[test]
    fn an_edit_writes_the_line_it_followed_its_task_to() {
        let (directory, mut app) = fixture();
        let b = directory.path().join("workspace/Work/Tasks/B.md");
        let rescan = |app: &mut App, text: &str| {
            std::fs::write(&b, text).unwrap();
            let _ = app.process_core(wren_core::ops::Op::Reconcile);
            let _ = app.update(Message::Tick);
        };
        rescan(&mut app, "# B\n\n- [ ] Order lunch\n");
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea")]);
        // Moved down to line 5, then its twin written on line 3.
        rescan(&mut app, "# B\n\nIntro.\n\n- [ ] Order lunch\n");
        assert_eq!(app.task_list.describe.as_ref().unwrap().key.line, 4);
        rescan(&mut app, "# B\n\n- [ ] Order lunch\n\n- [ ] Order lunch\n");
        let describe = app.task_list.describe.as_ref().unwrap();
        assert_eq!((describe.key.line, describe.target.line), (4, 4));
        field_batch(&mut app, &[LineEvent::Enter]);
        assert_eq!(
            on_disk(&directory, "B"),
            "# B\n\n- [ ] Order lunch\n\n- [ ] Order tea\n"
        );
        // A move to two lines at once: no telling which, the field closes.
        rescan(&mut app, "# B\n\n- [ ] Order lunch\n");
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order cake")]);
        rescan(
            &mut app,
            "# B\n\nIntro.\n- [ ] Order lunch\n- [ ] Order lunch\n",
        );
        assert_eq!(field_text(&app), None);
        field_batch(&mut app, &[LineEvent::Enter]);
        assert_eq!(
            on_disk(&directory, "B"),
            "# B\n\nIntro.\n- [ ] Order lunch\n- [ ] Order lunch\n"
        );
    }

    /// CodeRabbit on PR #13: a twin that was a task of its own before
    /// the rebuild is not where the edited one went: the field closes
    /// rather than rename it.
    #[test]
    fn an_edit_never_moves_to_a_twin_that_was_listed_before() {
        let (directory, mut app) = fixture();
        let b = directory.path().join("workspace/Work/Tasks/B.md");
        let rescan = |app: &mut App, text: &str| {
            std::fs::write(&b, text).unwrap();
            let _ = app.process_core(wren_core::ops::Op::Reconcile);
            let _ = app.update(Message::Tick);
        };
        rescan(
            &mut app,
            "# B\n\n## One\n- [ ] Call\n\n## Two\n- [ ] Call\n",
        );
        select(&mut app, "Call");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Call Kim")]);
        assert_eq!(app.task_list.describe.as_ref().unwrap().key.line, 3);
        // Another program turns the first into prose: the second is left.
        rescan(&mut app, "# B\n\n## One\nCall\n\n## Two\n- [ ] Call\n");
        assert_eq!(field_text(&app), None);
        field_batch(&mut app, &[LineEvent::Enter]);
        assert_eq!(
            on_disk(&directory, "B"),
            "# B\n\n## One\nCall\n\n## Two\n- [ ] Call\n"
        );
    }

    /// CodeRabbit on PR #13: a rebuild that moves the edited row out of
    /// the rows built ends the edit, so the list's keys work again.
    #[test]
    fn an_edit_ends_when_a_rebuild_moves_its_row_out_of_the_rows_built() {
        let (directory, mut app) = fixture();
        let b = directory.path().join("workspace/Work/Tasks/B.md");
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea")]);
        let mut text = String::from("# B\n\n");
        for at in 0..300 {
            text.push_str(&format!("- [ ] Early {at} 📅 2026-11-20\n"));
        }
        text.push_str("- [ ] Order lunch @sam 📅 2026-12-02\n");
        std::fs::write(&b, text).unwrap();
        let _ = app.process_core(wren_core::ops::Op::Reconcile);
        let _ = app.update(Message::Tick);
        assert_eq!(field_text(&app), None);
        assert!(app.task_list.describe.is_none());
    }

    /// Review of the CodeRabbit round: whether the edited row is built is
    /// asked where the list stays after a rebuild, its offset clamped to
    /// the shorter list, not where it was.
    #[test]
    fn an_edit_far_down_stays_when_a_rebuild_shortens_the_list_above_it() {
        let (directory, mut app) = fixture();
        let b = directory.path().join("workspace/Work/Tasks/B.md");
        let rescan = |app: &mut App, text: &str| {
            std::fs::write(&b, text).unwrap();
            let _ = app.process_core(wren_core::ops::Op::Reconcile);
            let _ = app.update(Message::Tick);
        };
        let mut long = String::from("# B\n\n");
        for at in 0..300 {
            long.push_str(&format!("- [ ] Early {at}\n"));
        }
        long.push_str("- [ ] Order lunch\n- [ ] Zed\n");
        rescan(&mut app, &long);
        let scroll = app.task_list.scroll;
        let _ = app.update(Message::TaskView(Event::Scrolled {
            y: scroll,
            height: 400.0,
        }));
        select(&mut app, "Order lunch");
        assert!(app.task_list.scroll > 2000.0, "{}", app.task_list.scroll);
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea")]);
        rescan(
            &mut app,
            "# B\n\n- [ ] Early 0\n- [ ] Order lunch\n- [ ] Zed\n",
        );
        assert_eq!(field_text(&app).as_deref(), Some("Order tea"));
        field_batch(&mut app, &[LineEvent::Enter]);
        assert!(on_disk(&directory, "B").contains("- [ ] Order tea\n"));
    }

    #[test]
    fn the_user_scrolling_ends_the_edit_before_a_later_enter() {
        let (directory, mut app) = fixture();
        select(&mut app, "Order lunch");
        list_batch(&mut app, &[f2()]);
        field_batch(&mut app, &[typed("Order tea @sam")]);
        // The list's own scroll, where the view put it: the edit stays.
        let at = app.task_list.scroll;
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Scrolled {
            y: at,
            height: 400.0,
        }));
        assert!(field_text(&app).is_some());
        // The user's scroll, then Enter, in one batch: nothing written.
        let instance = app.task_list.describes;
        let _ = app.update(Message::TaskView(Event::Scrolled {
            y: at + 10.0,
            height: 400.0,
        }));
        let _ = app.update(Message::TaskView(Event::Field {
            serial,
            instance,
            event: LineEvent::Enter,
        }));
        assert_eq!(field_text(&app), None);
        assert_eq!(on_disk(&directory, "B"), B);
    }

    #[test]
    fn four_presses_in_one_batch_open_the_field_once() {
        let (_directory, mut app) = fixture();
        let entry = entry(&app, "Order lunch");
        let generation = app.task_list.generation;
        let before = app.task_list.describes;
        let serial = app.task_list.next_serial();
        let select = Click::Select { generation, entry };
        let describe = Click::Describe { generation, entry };
        for click in [
            select.clone(),
            select.clone(),
            describe.clone(),
            select.clone(),
            select,
            describe,
        ] {
            let _ = app.update(Message::TaskView(Event::Click { serial, click }));
        }
        assert_eq!(app.task_list.describes, before + 1);
        assert!(field_text(&app).is_some());
    }
}
