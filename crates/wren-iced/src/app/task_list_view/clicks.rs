//! The task view's clicks, and the routing of all of its input.

use iced::Task;
use wren_core::tasks_file::Layout;

use super::{Cell, Control};
use crate::app::columns::Column;
use crate::app::host_menu;
use crate::app::tasks_view::Event;
use crate::app::{App, Message};

/// A click in the view or in its menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Click {
    /// A row, by the listing it was drawn from and its entry there.
    Select {
        generation: u64,
        entry: usize,
    },
    /// A row's source: open the note at its line.
    Open {
        generation: u64,
        entry: usize,
    },
    /// A cell of a row.
    Cell {
        generation: u64,
        entry: usize,
        cell: Cell,
    },
    /// A right-click on a row: its task menu.
    RowMenu {
        generation: u64,
        entry: usize,
    },
    /// A double-click on a row's description: edit it in place. It comes
    /// in the batch of its second press, after that press's `Select`.
    Describe {
        generation: u64,
        entry: usize,
    },
    Control(Control),
    /// A segment of the List | Board switch: show its layout.
    Layout(Layout),
    Menu(host_menu::Event),
}

impl App {
    /// Input from the list.
    pub(in crate::app) fn task_list_event(&mut self, event: Event) -> Task<Message> {
        // A key from the view goes through its batch's gate before anything
        // asks what the pane now shows.
        if let Event::Key(key) = event {
            return self.task_list_key(key);
        }
        if let Event::Field {
            serial,
            instance,
            event,
        } = event
        {
            return self.describe_event(serial, instance, event);
        }
        if let Event::Click {
            click: Click::Menu(host_menu::Event::Scrolled { instance, y }),
            ..
        } = event
        {
            if let Some(menu) =
                (self.task_list.menu.as_mut()).filter(|menu| menu.instance() == instance)
            {
                menu.scrolled(y);
            }
            return Task::none();
        }
        if let Event::Click { serial, .. } = event {
            self.enter_batch(serial);
        }
        if self.task_view().is_none() || self.dialog.is_some() || self.palette.is_some() {
            return Task::none();
        }
        match event {
            Event::Press => {
                self.column = Column::Editor;
                self.column_focus()
            }
            Event::Scrolled { y, height } => {
                // The user scrolling the list ends an edit in place, writing
                // nothing, before any key after it in the batch: only the
                // list's own scroll (the reveal of the row as the field
                // opens), which is where the view last put it, keeps it.
                if (y - self.task_list.scroll).abs() > 0.5 && self.cancel_describe() {
                    self.task_list.stopped = true;
                }
                self.task_list.scroll = y;
                self.task_list.height = Some(height);
                // An edit whose row is out of the rows built has no field.
                if (self.task_list.describe.as_ref())
                    .is_some_and(|describe| !self.built_rows().contains(&describe.entry))
                {
                    self.task_list.describe = None;
                }
                Task::none()
            }
            // A row of the menu released on: chosen only when its press was
            // allowed to choose it, as it was decided at the press.
            Event::Click {
                click: Click::Menu(host_menu::Event::Choose { instance, row }),
                ..
            } => {
                self.task_list.stopped = true;
                if self.task_list.armed.take() != Some((instance, row)) {
                    return Task::none();
                }
                let chosen =
                    (self.task_list.menu.as_ref()).and_then(|menu| menu.choose(instance, row));
                match chosen {
                    Some(row) => self.choose_task_menu(row),
                    None => Task::none(),
                }
            }
            Event::Click { click, .. } => {
                // A press ends what its batch may still do; one after
                // something opened in the batch, or after another press, is
                // dropped, but for the second press of a double-click on a
                // description, which selects its row and then edits it.
                let first = !std::mem::replace(&mut self.task_list.stopped, true);
                // A press over the menu's card may choose the row it is on
                // at its release: only the batch's first press, of the
                // primary button, with no key reaching the menu before it.
                if let Click::Menu(host_menu::Event::Pressed {
                    instance,
                    at,
                    primary,
                }) = click
                {
                    let window = self.viewport;
                    self.task_list.armed = (first && primary && !self.task_list.menu_keyed)
                        .then_some(self.task_list.menu.as_ref())
                        .flatten()
                        .filter(|menu| menu.instance() == instance)
                        .and_then(|menu| menu.row_at(at, window))
                        .map(|row| (instance, row));
                    return Task::none();
                }
                // The exception is spent on the one edit it lets open.
                let second = matches!(click, Click::Describe { generation, entry }
                    if self.task_list.selected_in_batch == Some((generation, entry)));
                if second {
                    self.task_list.selected_in_batch = None;
                }
                if !first && !second {
                    return Task::none();
                }
                self.task_list_click(click)
            }
            Event::Key(_) | Event::Field { .. } => Task::none(),
        }
    }

    /// A click, read in its batch.
    fn task_list_click(&mut self, click: Click) -> Task<Message> {
        let current = |app: &Self, generation: u64, entry: usize| {
            generation == app.task_list.generation && app.task_list.listing.record(entry).is_some()
        };
        // A click anywhere in the view ends an edit in place, writing
        // nothing, before it does what it does.
        self.task_list.describe = None;
        match click {
            Click::Select { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.task_list.selected_in_batch = Some((generation, entry));
                self.column = Column::Editor;
                self.task_list.focus = None;
                let task = self.select_entry(entry);
                Task::batch([task, self.column_focus()])
            }
            Click::Open { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                let _ = self.select_entry(entry);
                self.open_task_at(entry)
            }
            Click::Cell {
                generation,
                entry,
                cell,
            } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                let selected = self.select_entry(entry);
                let acted = self.click_cell(entry, cell);
                // The calendar the click opened takes the keyboard alone.
                if self.calendar.is_some() || self.task_form.is_some() {
                    return Task::batch([selected, acted]);
                }
                Task::batch([selected, acted, self.column_focus()])
            }
            Click::Describe { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                self.open_describe(entry)
            }
            Click::RowMenu { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                let selected = self.select_entry(entry);
                let opened = self.open_entry_menu(crate::app::task_menus::MenuKind::Task, entry);
                Task::batch([selected, opened, self.column_focus()])
            }
            // A control the layout shown does not draw (a status chip, Show
            // all or Group by on the board) is nothing to click: a click
            // queued before a switch does nothing after it.
            Click::Control(control) if !self.control_shown(control) => Task::none(),
            Click::Control(control) => {
                // A click leaves the keyboard with the list: only keys
                // walk the controls.
                self.column = Column::Editor;
                let task = self.press_control(control);
                self.task_list.focus = None;
                Task::batch([task, self.column_focus()])
            }
            Click::Layout(layout) => {
                // As a control's click: the keyboard stays with the body.
                self.column = Column::Editor;
                let task = self
                    .change_layout(|_| layout)
                    .map_or_else(Task::none, |outcome| outcome.task);
                self.task_list.focus = None;
                Task::batch([task, self.column_focus()])
            }
            Click::Menu(event) => match event {
                host_menu::Event::Outside { instance } => {
                    if (self.task_list.menu.as_ref())
                        .is_some_and(|menu| menu.instance() == instance)
                    {
                        self.task_list.menu = None;
                    }
                    Task::none()
                }
                host_menu::Event::Scrolled { .. }
                | host_menu::Event::Pressed { .. }
                | host_menu::Event::Choose { .. } => Task::none(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn a_press_outside_the_menu_then_enter_in_one_batch_drops_enter() {
        let (_directory, mut app) = fixture();
        control(&mut app, Control::GroupBy);
        let instance = app.task_list.menu.as_ref().unwrap().instance();
        // The press is taken by the layer under the menu, so the view's
        // keys never see it; it carries the view's serial, and stops the
        // batch, so Enter does not open the selected note.
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Click {
            serial,
            click: Click::Menu(host_menu::Event::Outside { instance }),
        }));
        in_batch(&mut app, serial, &[named(Named::Enter)]);
        assert!(app.task_list.menu.is_none());
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        // Enter in the next batch opens it.
        batch(&mut app, &[named(Named::Enter)]);
        assert!(app.task_view().is_none());
    }

    #[test]
    fn a_click_on_a_row_of_a_listing_since_rebuilt_does_nothing() {
        let (_directory, mut app) = fixture();
        let generation = app.task_list.generation;
        let order_badges = app.task_list.listing.rows().next().unwrap();
        // Doing's chip has the keys: Enter narrows the list to Review the
        // budget, which now stands where Order badges was drawn. The List |
        // Board switch is the Tab stop after the scope chip (item 12.1).
        batch(
            &mut app,
            &[
                named(Named::Tab),
                named(Named::Tab),
                named(Named::Tab),
                named(Named::Tab),
            ],
        );
        assert_eq!(app.task_list.focus, Some(Control::Chip(1)));
        let serial = app.task_list.next_serial();
        in_batch(&mut app, serial, &[named(Named::Enter)]);
        assert_eq!(descriptions(&app), ["Review the budget @sam"]);
        assert_ne!(app.task_list.generation, generation);
        // The press on Order badges' source, queued in the same batch.
        let _ = app.update(Message::TaskView(Event::Click {
            serial,
            click: Click::Open {
                generation,
                entry: order_badges,
            },
        }));
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        // In a batch of its own it still names the old listing.
        click(
            &mut app,
            Click::Open {
                generation,
                entry: order_badges,
            },
        );
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert_eq!(document(&app), None);
        // The row as drawn now opens.
        let entry = app.task_list.listing.rows().next().unwrap();
        let generation = app.task_list.generation;
        click(&mut app, Click::Open { generation, entry });
        assert_eq!(document(&app).as_deref(), Some("Work/Planning/Roadmap.md"));
    }
}
