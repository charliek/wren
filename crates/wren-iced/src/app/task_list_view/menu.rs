//! The menu over the task view ([`crate::app::host_menu`]): what it is
//! for, opening it, its selection's reveal, a choice, and its layers.

use iced::{Element, Rectangle, Task};
use wren_editor::Row;

use super::clicks::Click;
use crate::app::host_menu::{self, HostMenu};
use crate::app::tasks_view::{Event, Scope};
use crate::app::{App, Message};

/// What a menu over the view is for, with what its rows name.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum MenuFor {
    /// "Show tasks in": each row's scope, `None` for one there is none of.
    Scope(Vec<Option<Scope>>),
    /// Anyone, Me, then these people.
    Owner(Vec<String>),
    GroupBy,
    Due,
    Priority,
    /// A row's status, priority or task menu, for the task it was opened
    /// on, at the entry of the listing `generation`.
    Row {
        kind: crate::app::task_menus::MenuKind,
        target: crate::app::task_target::RowTarget,
        /// Where the task's box is in its line, as the task index found it.
        boxed: usize,
        entry: usize,
        generation: u64,
    },
}

impl MenuFor {
    #[cfg(feature = "smoke-test")]
    pub(in crate::app) const fn name(&self) -> &'static str {
        use crate::app::task_menus::MenuKind;
        match self {
            Self::Scope(_) => "scope",
            Self::Owner(_) => "owner",
            Self::GroupBy => "group",
            Self::Due => "due",
            Self::Priority => "priority",
            Self::Row { kind, .. } => match kind {
                MenuKind::Status => "row_status",
                MenuKind::Priority => "row_priority",
                MenuKind::Task => "row_task",
            },
        }
    }
}

impl App {
    /// Where a menu hangs whose control was never drawn.
    pub(in crate::app) fn unplaced_anchor(&self) -> Rectangle {
        Rectangle::new(
            iced::Point::new(self.viewport.width / 2.0 - 100.0, 96.0),
            iced::Size::ZERO,
        )
    }

    /// Open a menu over the view, under `anchor`, in place of any open,
    /// with the row `selected` selected, or else the checked one.
    pub(in crate::app) fn open_host_menu(
        &mut self,
        purpose: MenuFor,
        title: Option<String>,
        rows: Vec<Row>,
        anchor: Rectangle,
        selected: Option<u32>,
    ) -> Task<Message> {
        self.task_list.describe = None;
        self.task_list.menus += 1;
        let mut menu = HostMenu::new(self.task_list.menus, purpose, title, rows, anchor);
        if let Some(id) = selected {
            menu.select_id(id);
        }
        self.task_list.menu = Some(menu);
        self.reveal_task_menu()
    }

    /// Scroll the open menu's rows, when they scroll, to show its selected
    /// row.
    pub(super) fn reveal_task_menu(&mut self) -> Task<Message> {
        let window = self.viewport;
        match self
            .task_list
            .menu
            .as_mut()
            .and_then(|menu| menu.reveal(window))
        {
            Some(y) => iced::widget::operation::scroll_to(
                host_menu::ROWS,
                iced::widget::operation::AbsoluteOffset { x: 0.0, y },
            ),
            None => Task::none(),
        }
    }

    /// Act on the row `row` of the open menu, and close it.
    pub(super) fn choose_task_menu(&mut self, row: u32) -> Task<Message> {
        let Some(menu) = self.task_list.menu.take() else {
            return Task::none();
        };
        if let MenuFor::Row {
            target,
            boxed,
            entry,
            generation,
            ..
        } = menu.purpose
        {
            return self.choose_row_menu(&target, boxed, entry, generation, row);
        }
        self.choose_filter_menu(menu.purpose, row)
    }

    /// Close the menu over the view, if one is open: Escape's step in its
    /// precedence, after the calendar and the form.
    pub(in crate::app) fn close_task_menu(&mut self) -> bool {
        self.task_list.menu.take().is_some()
    }

    /// The menu over the view, as the window's overlay layers.
    pub(in crate::app) fn task_menu_layers(
        &self,
    ) -> Option<(Element<'_, Message>, Element<'_, Message>)> {
        let menu = self
            .task_list
            .menu
            .as_ref()
            .filter(|_| self.task_view().is_some())?;
        // The view's serial: the menu's input is read in the view's batch.
        let serial = self.task_list.serials.get();
        Some(menu.layers(self.definition, self.viewport, move |event| {
            Message::TaskView(Event::Click {
                serial,
                click: Click::Menu(event),
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn a_stale_menu_choice_and_a_press_outside_do_nothing() {
        let (directory, mut app) = fixture();
        control(&mut app, Control::GroupBy);
        let first = app.task_list.menu.as_ref().unwrap().instance();
        menu_event(&mut app, host_menu::Event::Outside { instance: first });
        assert!(app.task_list.menu.is_none());
        control(&mut app, Control::GroupBy);
        let second = app.task_list.menu.as_ref().unwrap().instance();
        assert_ne!(first, second);
        // A click on a row of the first menu chooses nothing, and a press
        // outside the first closes nothing.
        menu_event(
            &mut app,
            host_menu::Event::Choose {
                instance: first,
                row: 1,
            },
        );
        menu_event(&mut app, host_menu::Event::Outside { instance: first });
        assert!(app.task_list.menu.is_some());
        assert_eq!(tasks_toml(&directory), None);
        menu_event(
            &mut app,
            host_menu::Event::Choose {
                instance: second,
                row: 1,
            },
        );
        assert!(app.task_list.menu.is_none());
        let labels: Vec<&str> = (app.task_list.listing.groups.iter())
            .map(|group| group.label.as_str())
            .collect();
        assert_eq!(labels[0], "Overdue");
        assert!(tasks_toml(&directory).unwrap().contains("group = \"due\""));
        // The palette takes the keyboard, and the menu goes.
        control(&mut app, Control::Owner);
        assert!(app.task_list.menu.is_some());
        let _ = app.open_palette(crate::app::palette::Mode::Commands);
        let _ = app.update(Message::Tick);
        assert!(app.task_list.menu.is_none());
    }
}
