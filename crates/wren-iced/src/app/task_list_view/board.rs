//! The task view's board layout (discovery/tasks.md §11): a column for
//! each status, Open, Doing, Done and Cancelled, all four always shown.
//! Item 12.1 draws the columns empty; their cards, counts and scrolling
//! are item 12.2's, and the board's own keys item 12.3's.

use iced::widget::{column, container, row};
use iced::{Element, Fill, Task};
use wren_markdown::status::StatusType;

use crate::app::{App, Message};
use crate::list_keys::Press;
use crate::{chrome, fonts, theme};

/// The board's columns, left to right, with their headings.
pub(in crate::app) const COLUMNS: [(StatusType, &str); 4] = [
    (StatusType::Open, "Open"),
    (StatusType::Doing, "Doing"),
    (StatusType::Done, "Done"),
    (StatusType::Cancelled, "Cancelled"),
];

impl App {
    /// The board under the view's controls.
    pub(super) fn board_body(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let mut columns = row![].spacing(10).height(Fill);
        for (_, label) in COLUMNS {
            let head = chrome::label(label.to_uppercase(), 11, colors.muted).font(fonts::SEMIBOLD);
            let lane = container(column![container(head).padding([2, 4])].spacing(6))
                .padding(8)
                .width(Fill)
                .height(Fill)
                .clip(true)
                .style(move |_| container::Style {
                    background: Some(theme::color(colors.pane).into()),
                    border: iced::Border {
                        color: theme::color(colors.divider),
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                });
            columns = columns.push(lane);
        }
        container(columns)
            .padding(12)
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// A key for the board itself, which has the keyboard. Returns whether
    /// it stops its batch. The board's keys are item 12.3's; until then it
    /// reads none, so no key acts on a card it does not show.
    pub(super) fn board_press(&mut self, _press: &Press) -> (Task<Message>, bool) {
        (Task::none(), false)
    }
}

#[cfg(test)]
mod tests {
    use wren_core::tasks_file::Layout;

    use super::super::keys::task_view_slots;
    use super::super::test_support::*;
    use crate::app::commands::{CommandId, Disposition};

    fn layout(app: &App) -> Layout {
        app.task_list.layout()
    }

    fn shift(press: Press) -> Press {
        Press {
            modifiers: Modifiers::SHIFT,
            ..press
        }
    }

    /// The switch, `B` and the palette command each flip the layout; the
    /// selection stays with its task, and only Board is written.
    #[test]
    fn the_switch_b_and_the_command_each_flip_the_layout() {
        let (directory, mut app) = fixture();
        batch(&mut app, &[letter("j"), letter("j")]);
        let chosen = selected(&app);
        assert_eq!(layout(&app), Layout::List);

        // A click on the Board segment.
        click(&mut app, Click::Layout(Layout::Board));
        assert_eq!(layout(&app), Layout::Board);
        assert_eq!(app.task_list.focus, None, "the body keeps the keyboard");
        assert_eq!(
            tasks_toml(&directory).unwrap(),
            "schema_version = 1\n\n[views.\".\"]\nlayout = \"board\"\n"
        );
        // A click on the segment pressed changes nothing.
        click(&mut app, Click::Layout(Layout::Board));
        assert_eq!(layout(&app), Layout::Board);
        // The board's hint names no list key.
        assert_eq!(app.task_view_hint(), "");
        assert_eq!(app.task_list_hint(10_000.0), "");

        // `B`, and Shift+B, while the body has the keyboard.
        batch(&mut app, &[letter("b")]);
        assert_eq!(layout(&app), Layout::List);
        assert_eq!(selected(&app), chosen, "the selection kept its task");
        assert_eq!(tasks_toml(&directory).unwrap(), "schema_version = 1\n");
        batch(&mut app, &[shift(letter("B"))]);
        assert_eq!(layout(&app), Layout::Board);

        // The palette command, from the palette, which closes.
        let _ = app.dispatch(Command::Run(CommandId::CommandPalette));
        assert!(app.palette.is_some());
        let outcome = app.dispatch(Command::Run(CommandId::ToggleTaskBoard));
        assert!(outcome.processed.disposition.committed());
        let _ = app.update(Message::Tick);
        assert!(app.palette.is_none());
        assert_eq!(layout(&app), Layout::List);
        assert!(app.task_view_has_keys());
        assert_eq!(selected(&app), chosen);
        // Enter or Space on the switch flips it, and it keeps the keyboard.
        batch(&mut app, &[named(Named::Tab), named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Layout));
        batch(&mut app, &[named(Named::Enter)]);
        assert_eq!(layout(&app), Layout::Board);
        assert_eq!(app.task_list.focus, Some(Control::Layout));
        batch(&mut app, &[named(Named::Space)]);
        assert_eq!(layout(&app), Layout::List);
        // The harness's control click presses it as Enter does.
        control(&mut app, Control::Layout);
        assert_eq!(layout(&app), Layout::Board);
        assert_eq!(app.task_list.focus, None);
    }

    /// `B` is the body's: on a control, under a menu or a held key's
    /// repeat it does nothing.
    #[test]
    fn b_is_read_only_while_the_body_has_the_keyboard() {
        let (directory, mut app) = fixture();
        // On a control the control's keys win.
        batch(&mut app, &[named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        batch(&mut app, &[letter("b")]);
        assert_eq!(layout(&app), Layout::List);
        // A menu over the view takes it: the owner menu has no B.
        batch(&mut app, &[named(Named::Escape)]);
        control(&mut app, Control::Owner);
        assert!(app.task_list.menu.is_some());
        batch(&mut app, &[letter("b")]);
        assert_eq!(layout(&app), Layout::List);
        assert!(app.task_list.menu.is_some());
        batch(&mut app, &[named(Named::Escape)]);
        assert!(app.task_list.menu.is_none());
        // A held B flips once.
        let held = Press {
            repeat: true,
            ..letter("b")
        };
        batch(&mut app, &[letter("b"), held.clone(), held]);
        assert_eq!(layout(&app), Layout::Board);
        // On the board F2 opens no field over a card it does not show
        // yet (item 12.3).
        batch(&mut app, &[press(Key::Named(Named::F2), None)]);
        assert!(app.task_list.describe.is_none());
        // The description edited in place has its keys: B types there.
        batch(&mut app, &[letter("b")]);
        batch(&mut app, &[press(Key::Named(Named::F2), None)]);
        let instance = app.task_list.describe.as_ref().unwrap().instance;
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Field {
            serial,
            instance,
            event: crate::line_field::LineEvent::Typed("b".into()),
        }));
        assert_eq!(app.task_list.describe.as_ref().unwrap().text, "b");
        assert_eq!(layout(&app), Layout::List);
        assert!(tasks_toml(&directory).is_some());
    }

    /// Switching is no stop in Back and Forward, and lets the held rows go.
    #[test]
    fn switching_is_no_history_stop_and_lets_held_rows_go() {
        let (_directory, mut app) = fixture();
        let roadmap = (app.model().unwrap().notes.iter())
            .find(|note| note.title == "Roadmap")
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote {
            note: Some(roadmap),
        });
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        let steps = |app: &App| (app.can_step(true), app.can_step(false));
        let before = steps(&app);
        assert_eq!(before, (true, false));
        // A row written is held where it stood.
        batch(&mut app, &[named(Named::Space)]);
        let _ = app.update(Message::Tick);
        assert!(!app.task_list.held.is_empty());
        batch(&mut app, &[letter("b")]);
        assert_eq!(layout(&app), Layout::Board);
        assert!(app.task_list.held.is_empty(), "the held rows let go");
        assert_eq!(steps(&app), before);
        batch(&mut app, &[letter("b")]);
        assert_eq!(steps(&app), before);
        let _ = app.dispatch(Command::Run(CommandId::ToggleTaskBoard));
        assert_eq!(steps(&app), before);
        // Back leaves the view at once, and Forward shows it on its board.
        let _ = app.dispatch(Command::Run(CommandId::Back));
        let _ = app.update(Message::Tick);
        assert_eq!(app.task_view(), None);
        assert_eq!(document(&app).as_deref(), Some("Work/Planning/Roadmap.md"));
        assert!(!app.command_enabled(CommandId::ToggleTaskBoard));
        let _ = app.dispatch(Command::Run(CommandId::Forward));
        let _ = app.update(Message::Tick);
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert_eq!(layout(&app), Layout::Board);
    }

    /// Each scope keeps its own layout across a restart; a filter change
    /// on the board writes its filter beside it.
    #[test]
    fn the_layout_survives_a_restart_for_each_scope() {
        let (directory, mut app) = fixture();
        batch(&mut app, &[letter("b")]);
        let group = Scope::Group(RelPath::new("Work/Planning").unwrap());
        let _ = app.dispatch(Command::ShowTasks(group.clone()));
        let _ = app.update(Message::Tick);
        assert_eq!(layout(&app), Layout::List);
        control(&mut app, Control::HideOld);
        assert_eq!(
            tasks_toml(&directory).unwrap(),
            "schema_version = 1\n\n[views.\".\"]\nlayout = \"board\"\n\n\
             [views.\"Work/Planning\"]\nhide_old = false\n"
        );
        drop(app);
        let mut app = launch(directory.path(), &root(&directory));
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        assert_eq!(layout(&app), Layout::Board);
        control(&mut app, Control::Owner);
        batch(&mut app, &[letter("m")]);
        assert_eq!(
            tasks_toml(&directory).unwrap(),
            "schema_version = 1\n\n[views.\".\"]\nlayout = \"board\"\nowner = \"me\"\n\n\
             [views.\"Work/Planning\"]\nhide_old = false\n"
        );
        let _ = app.dispatch(Command::ShowTasks(group));
        let _ = app.update(Message::Tick);
        assert_eq!(layout(&app), Layout::List);
        assert!(!app.task_list.listing.entries.is_empty());
    }

    /// The board hides the status chips, Show all and Group by, which are
    /// then no Tab stops; a control the switch hides gives the keyboard
    /// back to the body.
    #[test]
    fn the_board_hides_the_list_only_controls() {
        let (_directory, mut app) = fixture();
        control(&mut app, Control::Chip(1));
        let list = app.task_list.built.as_ref().unwrap().filters.clone();
        assert_eq!(
            task_view_slots(&list),
            [
                Some(Control::Scope),
                Some(Control::Layout),
                Some(Control::Chip(0)),
                Some(Control::Chip(1)),
                Some(Control::Chip(2)),
                Some(Control::Chip(3)),
                Some(Control::ShowAll),
                Some(Control::Owner),
                Some(Control::GroupBy),
                Some(Control::Due),
                Some(Control::Priority),
                Some(Control::HideOld),
                None,
            ]
        );
        // A chip has the keyboard when the command switches to the board.
        batch(
            &mut app,
            &[named(Named::Tab), named(Named::Tab), named(Named::Tab)],
        );
        assert_eq!(app.task_list.focus, Some(Control::Chip(0)));
        let _ = app.dispatch(Command::Run(CommandId::ToggleTaskBoard));
        let _ = app.update(Message::Tick);
        assert_eq!(layout(&app), Layout::Board);
        assert_eq!(app.task_list.focus, None);
        let board = app.task_list.built.as_ref().unwrap().filters.clone();
        assert_eq!(
            task_view_slots(&board),
            [
                Some(Control::Scope),
                Some(Control::Layout),
                Some(Control::Owner),
                Some(Control::Due),
                Some(Control::Priority),
                Some(Control::HideOld),
                None,
            ]
        );
        // Tab walks exactly those, and ← → keep to the controls.
        let mut walked = Vec::new();
        for _ in 0..7 {
            batch(&mut app, &[named(Named::Tab)]);
            walked.push(app.task_list.focus);
        }
        assert_eq!(walked, task_view_slots(&board));
        batch(&mut app, &[named(Named::Tab), named(Named::ArrowLeft)]);
        assert_eq!(app.task_list.focus, Some(Control::HideOld));
        // The status chips still remember their filter for the list.
        assert_eq!(board.statuses, list.statuses);
        // Where the chips and Group by were drawn is forgotten once the
        // board's bar is built: nothing hangs under them.
        let at = iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(10.0, 10.0));
        (app.task_list.drawn.borrow_mut()).extend([
            (Control::Chip(0), at),
            (Control::ShowAll, at),
            (Control::GroupBy, at),
            (Control::Owner, at),
        ]);
        let _ = app.view();
        let drawn: Vec<Control> = (app.task_list.drawn.borrow().iter())
            .map(|(control, _)| *control)
            .collect();
        assert!(drawn.contains(&Control::Owner));
        assert!(!drawn.iter().any(|control| matches!(
            control,
            Control::Chip(_) | Control::ShowAll | Control::GroupBy
        )));
    }

    /// A switch, whatever makes it, closes the menu over the view; on the
    /// board a click on a control it does not draw does nothing.
    #[test]
    fn a_switch_closes_the_menu_and_hidden_controls_take_no_click() {
        let (directory, mut app) = fixture();
        control(&mut app, Control::GroupBy);
        assert!(app.task_list.menu.is_some());
        // The command as a configured shortcut runs it, the menu open.
        let _ = app.run_shortcut(CommandId::ToggleTaskBoard);
        let _ = app.update(Message::Tick);
        assert_eq!(layout(&app), Layout::Board);
        assert!(app.task_list.menu.is_none(), "Group by's menu closed");
        // The switch's own click closes a row's menu too.
        click(&mut app, Click::Layout(Layout::List));
        let row = Click::RowMenu {
            generation: app.task_list.generation,
            entry: app.task_list.selected.as_ref().unwrap().entry,
        };
        click(&mut app, row);
        assert!(app.task_list.menu.is_some());
        let _ = app.dispatch(Command::Run(CommandId::ToggleTaskBoard));
        assert!(app.task_list.menu.is_none(), "the row's menu closed");
        let _ = app.update(Message::Tick);
        let before = tasks_toml(&directory);
        for hidden in [Control::GroupBy, Control::Chip(1), Control::ShowAll] {
            control(&mut app, hidden);
            assert!(app.task_list.menu.is_none(), "{hidden:?}");
        }
        let filters = app.task_list.built.as_ref().unwrap().filters.clone();
        assert!(filters.statuses.is_empty());
        assert_eq!(tasks_toml(&directory), before);
        // What the board draws still takes its click.
        control(&mut app, Control::Owner);
        assert!(app.task_list.menu.is_some());
    }

    /// The macOS View menu's Toggle Task Board follows the command: enabled
    /// while a view shows, greyed otherwise.
    #[test]
    fn the_menu_item_follows_the_view() {
        let (_directory, mut app) = fixture();
        let item = |app: &App| {
            (app.menu_synced.iter())
                .find(|(key, _)| *key == "tasks.toggle_board")
                .map(|(_, enabled)| *enabled)
        };
        let _ = app.update(Message::Tick);
        assert_eq!(item(&app), Some(true));
        let roadmap = (app.model().unwrap().notes.iter())
            .find(|note| note.title == "Roadmap")
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote {
            note: Some(roadmap),
        });
        let _ = app.update(Message::Tick);
        assert_eq!(app.task_view(), None);
        assert_eq!(item(&app), Some(false));
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        assert_eq!(item(&app), Some(true));
        let mut enabled = app.menu_enabled();
        assert!(enabled("tasks.toggle_board"));
    }

    /// The command lives only while a view shows, and not at all with
    /// tasks off.
    #[test]
    fn the_command_needs_a_view_and_tasks_on() {
        let (directory, mut app) = fixture();
        assert!(app.command_enabled(CommandId::ToggleTaskBoard));
        let roadmap = (app.model().unwrap().notes.iter())
            .find(|note| note.title == "Roadmap")
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote {
            note: Some(roadmap),
        });
        assert_eq!(app.task_view(), None);
        assert!(!app.command_enabled(CommandId::ToggleTaskBoard));
        let outcome = app.dispatch(Command::Run(CommandId::ToggleTaskBoard));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert_eq!(tasks_toml(&directory), None);
        drop(app);

        // Tasks off: no command, no key, no file.
        let config = directory.path().join("config.conf");
        std::fs::write(&config, "tasks = off\n").unwrap();
        let (mut app, _) = App::with_config(
            Options {
                editor: true,
                ..Options::default()
            },
            None,
            Some(config),
        );
        app.workspace_mode = true;
        app.session = Some(Session::open_with(&root(&directory), app.session_options()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        assert!(CommandId::ToggleTaskBoard.tasks());
        assert!(!app.command_enabled(CommandId::ToggleTaskBoard));
        let outcome = app.dispatch(Command::Run(CommandId::ToggleTaskBoard));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert_eq!(app.preferences.keymap.accel("tasks.toggle_board"), None);
        assert_eq!(tasks_toml(&directory), None);
    }
}
