//! The task view's keys, as any layout of it reads them: the batch gate,
//! the app's shortcuts run in place, a held key's repeats, the menu over
//! the view, and Tab and the arrows over the controls. The list's own
//! keys, which move its selection, are in [`super::list`].

use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::Task;

use super::copies::AppCopy;
use super::Control;
use crate::app::host_menu::MenuKey;
use crate::app::task_list;
use crate::app::{App, Message};
use crate::list_keys::{ListKey, Press};

impl App {
    /// The slots Tab walks, in order.
    fn task_view_slots(&self) -> Vec<Option<Control>> {
        let filters = self
            .task_list
            .built
            .as_ref()
            .map(|built| built.filters.clone())
            .unwrap_or_default();
        let mut slots = vec![Some(Control::Scope)];
        slots.extend((0..task_list::CHIPS.len()).map(|chip| Some(Control::Chip(chip))));
        if !filters.statuses.is_empty() {
            slots.push(Some(Control::ShowAll));
        }
        slots.extend(
            [
                Control::Owner,
                Control::GroupBy,
                Control::Due,
                Control::Priority,
                Control::HideOld,
            ]
            .map(Some),
        );
        slots.push(None);
        slots
    }

    /// Tab (`step` 1) or Shift+Tab (-1); `controls` keeps to the controls,
    /// as ← and → do.
    fn step_focus(&mut self, step: isize, controls: bool) {
        let mut slots = self.task_view_slots();
        if controls {
            slots.retain(Option::is_some);
        }
        let count = slots.len() as isize;
        let at = slots
            .iter()
            .position(|slot| *slot == self.task_list.focus)
            .map_or(if step > 0 { -1 } else { count }, |at| at as isize);
        self.task_list.focus = slots[(at + step).rem_euclid(count) as usize];
    }

    /// Read input of the batch `serial`: a new batch starts afresh.
    pub(in crate::app) fn enter_batch(&mut self, serial: u64) {
        let list = &mut self.task_list;
        if serial != list.batch {
            list.batch = serial;
            list.stopped = false;
            list.menu_keyed = false;
            list.copies.clear();
            list.selected_in_batch = None;
        }
    }

    /// A key reported by the view's key wrapper, at its place in its batch.
    /// See the module's description.
    pub(in crate::app) fn task_list_key(&mut self, key: ListKey) -> Task<Message> {
        self.enter_batch(key.serial);
        let press = key.press;
        let copy = if press.is_plain_escape() {
            Some(AppCopy::Escape)
        } else {
            crate::keys::shortcut(&press.key, press.physical, press.modifiers)
                .map(AppCopy::Shortcut)
        };
        // What the key does is done here, in its place in its batch, or
        // not at all: the app's copy, which comes after the batch, is
        // always dropped.
        if let Some(copy) = &copy {
            self.task_list.copies.push(copy.clone());
        }
        // Typed after something opened or was pressed in its batch, or
        // after the view lost the keyboard (the note it opened has it):
        // dropped.
        if self.task_list.stopped || key.after_press || !self.task_view_has_keys() {
            return Task::none();
        }
        // A held key's repeats toggle nothing and open nothing: a write or
        // a menu at the keyboard's repeat rate is no one's choice.
        if press.repeat && self.ignores_repeat(&press, copy.as_ref()) {
            return Task::none();
        }
        // Any key that reaches an open menu, a chord it holds included,
        // leaves no press after it in the batch a choice.
        if self.task_list.menu.is_some() {
            self.task_list.menu_keyed = true;
        }
        match copy {
            // Escape is read here, in its place: it closes the menu, which
            // stops the batch, or gives a control's keyboard back to the
            // list.
            Some(AppCopy::Escape) => {
                if self.close_task_menu() {
                    self.task_list.stopped = true;
                } else {
                    self.unfocus_task_control();
                }
                Task::none()
            }
            // F2 edits the selected row's description in place: the view's
            // own key, whatever the keymap gives it elsewhere.
            Some(AppCopy::Shortcut(accel))
                if crate::app::task_describe::is_describe_key(&accel) =>
            {
                let (task, stops) = self.describe_key();
                if stops {
                    self.task_list.stopped = true;
                }
                task
            }
            // The view's own chords (toggle and undo) act here, in their
            // place in the batch, and so do the app's own shortcuts: one
            // that takes the keyboard from the list (the palette, a
            // dialog, another surface) stops the rest of the batch, which
            // was typed for the list. A shortcut of no command, or of the
            // focused editor's (not mounted under the view), runs nothing,
            // as the app's key handling would run nothing for it.
            Some(AppCopy::Shortcut(accel)) => {
                let id = (self.preferences.keymap.action(&accel))
                    .and_then(crate::app::commands::CommandId::parse);
                if let Some(task) = id.and_then(|id| self.task_view_shortcut(id)) {
                    return task;
                }
                let Some(id) = id.filter(|id| id.scope() != crate::app::commands::Scope::Editor)
                else {
                    return Task::none();
                };
                let task = self.run_shortcut(id);
                if !self.task_view_has_keys() {
                    self.task_list.stopped = true;
                }
                task
            }
            None => {
                let (task, stops) = self.task_list_press(&press);
                if stops {
                    self.task_list.stopped = true;
                }
                task
            }
        }
    }

    /// Whether a repeat of `press`, whose copy for the app is `copy`, does
    /// nothing: Space, Enter, the toggle and undo chords, + and -, and
    /// every key that opens something (S, P, D, E, F2, ↓ on a control with
    /// a menu), and on an open menu all but its arrows.
    fn ignores_repeat(&self, press: &Press, copy: Option<&AppCopy>) -> bool {
        use crate::app::task_rows::RowAct;
        match copy {
            Some(AppCopy::Escape) => false,
            Some(AppCopy::Shortcut(accel)) => {
                crate::app::task_describe::is_describe_key(accel)
                    || matches!(
                        self.preferences.keymap.action(accel),
                        Some("markdown.toggle_task" | "edit.undo")
                    )
            }
            None if self.task_list.menu.is_some() => !matches!(
                press.key,
                Key::Named(Named::ArrowUp | Named::ArrowDown | Named::PageUp | Named::PageDown)
            ),
            None => {
                matches!(press.key, Key::Named(Named::Enter | Named::Space))
                    || matches!(
                        RowAct::of(press),
                        Some(
                            RowAct::StatusMenu
                                | RowAct::PriorityMenu
                                | RowAct::Calendar
                                | RowAct::Form
                                | RowAct::Raise
                                | RowAct::Lower
                        )
                    )
                    || (press.key == Key::Named(Named::ArrowDown)
                        && self.task_list.focus.is_some_and(Control::opens_menu))
            }
        }
    }

    /// A key for the view: the menu's while one is open, else the focused
    /// control's, else the list's. Returns whether it stops its batch.
    fn task_list_press(&mut self, press: &Press) -> (Task<Message>, bool) {
        let shift = press.modifiers.shift();
        if let Some(menu) = &mut self.task_list.menu {
            return match menu.key(
                &press.key,
                press.physical,
                press.modifiers,
                press.text.as_deref(),
            ) {
                MenuKey::Choose(row) => (self.choose_task_menu(row), true),
                MenuKey::Moved => (self.reveal_task_menu(), false),
                MenuKey::Held => (Task::none(), false),
            };
        }
        if press.key == Key::Named(Named::Tab) {
            self.step_focus(if shift { -1 } else { 1 }, false);
            return (Task::none(), false);
        }
        if let Some(control) = self.task_list.focus {
            return match &press.key {
                Key::Named(Named::ArrowLeft) => {
                    self.step_focus(-1, true);
                    (Task::none(), false)
                }
                Key::Named(Named::ArrowRight) => {
                    self.step_focus(1, true);
                    (Task::none(), false)
                }
                Key::Named(Named::Enter | Named::Space) => {
                    (self.press_control(control), control.opens_menu())
                }
                Key::Named(Named::ArrowDown) if control.opens_menu() => {
                    (self.press_control(control), true)
                }
                _ => (Task::none(), false),
            };
        }
        self.list_press(press)
    }

    /// Escape's step for a control: the keyboard goes back to the list.
    pub(in crate::app) fn unfocus_task_control(&mut self) -> bool {
        self.task_view_has_keys() && self.task_list.focus.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn tab_walks_the_controls_and_escape_gives_the_keyboard_back() {
        let (_directory, mut app) = fixture();
        batch(&mut app, &[named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        batch(&mut app, &[named(Named::Tab), named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Chip(1)));
        // Space presses a chip, and the keyboard stays on it.
        batch(&mut app, &[named(Named::Space)]);
        assert_eq!(descriptions(&app), ["Review the budget @sam"]);
        assert_eq!(app.task_list.focus, Some(Control::Chip(1)));
        // Show all now stands after the chips.
        batch(
            &mut app,
            &[
                named(Named::ArrowRight),
                named(Named::ArrowRight),
                named(Named::ArrowRight),
            ],
        );
        assert_eq!(app.task_list.focus, Some(Control::ShowAll));
        batch(&mut app, &[named(Named::Enter)]);
        assert_eq!(app.task_list.listing.shown, 12);
        assert_eq!(app.task_list.focus, Some(Control::Chip(0)));
        // The selection stayed with its task through the chips. j is no
        // list key while a control has the keyboard.
        assert_eq!(selected(&app), "Review the budget @sam");
        batch(&mut app, &[letter("j")]);
        assert_eq!(selected(&app), "Review the budget @sam");
        // Escape gives it back to the list, and the list's keys work.
        batch(&mut app, &[named(Named::Escape)]);
        assert_eq!(app.task_list.focus, None);
        batch(&mut app, &[letter("j")]);
        assert_eq!(selected(&app), "Fix the date");
        // Shift+Tab from the list goes to the last control, round the end.
        let mut back = named(Named::Tab);
        back.modifiers = Modifiers::SHIFT;
        batch(&mut app, &[back.clone()]);
        assert_eq!(app.task_list.focus, Some(Control::HideOld));
        batch(&mut app, &[named(Named::Tab)]);
        assert_eq!(app.task_list.focus, None);
        batch(&mut app, &[named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        batch(&mut app, &[back]);
        assert_eq!(app.task_list.focus, None);
    }

    #[test]
    fn the_scope_menu_opens_by_keys_and_a_choice_shows_that_scope() {
        let (_directory, mut app) = fixture();
        // The tree's selection names the group and note below the scope.
        let roadmap = (app.model().unwrap().notes.iter())
            .find(|note| note.title == "Roadmap")
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote {
            note: Some(roadmap),
        });
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        // Enter on the chip opens the menu; the keys after it in its batch
        // are dropped, Escape too, so the menu stays.
        batch(
            &mut app,
            &[
                named(Named::Tab),
                named(Named::Enter),
                letter("g"),
                named(Named::Escape),
            ],
        );
        let menu = app.task_list.menu.as_ref().expect("the scope menu");
        assert_eq!(menu.title(), Some("Show tasks in"));
        assert!(matches!(menu.purpose, MenuFor::Scope(_)));
        let checked: Vec<bool> = menu.rows().iter().map(|row| row.checked).collect();
        assert_eq!(checked, [true, false, false, false]);
        assert!(app.task_list.copies.is_empty(), "the copy found its report");
        assert!(matches!(app.task_view(), Some(Scope::Workspace)));
        // In a batch of its own, Escape closes it without a choice.
        batch(&mut app, &[named(Named::Escape)]);
        assert!(app.task_list.menu.is_none());
        assert!(matches!(app.task_view(), Some(Scope::Workspace)));
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        // Its accelerator chooses a row: the section.
        batch(&mut app, &[named(Named::Enter)]);
        batch(&mut app, &[letter("s")]);
        assert_eq!(
            app.task_view(),
            Some(&Scope::Section(RelPath::new("Work").unwrap()))
        );
        assert!(app.task_list.menu.is_none());
        // A new scope starts at the list.
        assert_eq!(app.task_list.focus, None);
        // Down and Enter choose the group.
        batch(&mut app, &[named(Named::Tab), named(Named::ArrowDown)]);
        let menu = app.task_list.menu.as_ref().unwrap();
        assert_eq!(menu.selected(), Some(1));
        let labels: Vec<&str> = menu.rows().iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "workspace (workspace)",
                "Work (section)",
                "Planning (group)",
                "Roadmap (note)"
            ]
        );
        batch(&mut app, &[named(Named::ArrowDown)]);
        batch(&mut app, &[named(Named::Enter)]);
        assert_eq!(
            app.task_view(),
            Some(&Scope::Group(RelPath::new("Work/Planning").unwrap()))
        );
    }

    #[test]
    fn enter_then_back_in_one_batch_opens_the_note_and_back_does_not_run() {
        let (_directory, mut app) = fixture();
        let back = chord(&app, "navigation.back", "[");
        batch(&mut app, &[named(Named::Home)]);
        assert_eq!(selected(&app), "Order badges");
        // Enter opens the note; Back and Escape after it in the batch were
        // pressed at the view, and are dropped, their app copies too.
        batch(
            &mut app,
            &[named(Named::Enter), back.clone(), named(Named::Escape)],
        );
        assert!(app.task_view().is_none());
        assert_eq!(document(&app).as_deref(), Some("Work/Meetings/Standup.md"));
        assert!(
            app.task_list.copies.is_empty(),
            "each copy found its report"
        );
        // In a batch of its own, Back runs: to the view.
        let accel = crate::keys::shortcut(&back.key, back.physical, back.modifiers).unwrap();
        let _ = app.update(Message::Key(Input::Accel(accel)));
        assert!(app.task_view().is_some());
    }

    #[test]
    fn escape_then_a_key_on_an_open_menu_closes_it_and_drops_the_key() {
        let (_directory, mut app) = fixture();
        batch(&mut app, &[named(Named::Tab)]);
        batch(&mut app, &[named(Named::Enter)]);
        assert!(app.task_list.menu.is_some());
        // Escape, read in its place, closes the menu; `s` after it is
        // dropped, never chosen from the menu that was open.
        batch(&mut app, &[named(Named::Escape), letter("s")]);
        assert!(app.task_list.menu.is_none());
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        assert!(app.task_list.copies.is_empty());
    }

    #[test]
    fn each_dropped_copy_is_matched_to_its_own_key() {
        let (_directory, mut app) = fixture();
        let back = chord(&app, "navigation.back", "[");
        let f5 = press(Key::Named(Named::F5), None);
        batch(&mut app, &[named(Named::Tab)]);
        // An unbound function key, Enter on the chip, then Back: the
        // function key's copy is the app's (and does nothing), Back's is
        // dropped with the batch Enter stopped.
        batch(&mut app, &[f5, named(Named::Enter), back]);
        assert!(app.task_list.menu.is_some());
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert!(app.task_list.copies.is_empty());
        // A key after a press in its batch is dropped too.
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Key(ListKey {
            press: letter("w"),
            serial,
            after_press: true,
        })));
        assert!(app.task_list.menu.is_some());
    }

    /// A shortcut the list read is done with at its place in its batch,
    /// even when it ran nothing there: its copy is dropped, never run by
    /// the app's key handling after the batch, though the keymap gave the
    /// key a command in between.
    #[test]
    fn a_shortcut_the_list_read_is_never_run_by_its_copy() {
        let (_directory, mut app) = fixture();
        let f5 = press(Key::Named(Named::F5), None);
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Key(ListKey {
            press: f5.clone(),
            serial,
            after_press: false,
        })));
        assert!(app.palette.is_none(), "F5 is bound to nothing");
        let keybinds = vec![("f5".to_owned(), "palette.commands".to_owned())];
        let scheme = app.preferences.keymap.scheme();
        let (keymap, warnings) = crate::app::preferences::build_keymap(scheme, &keybinds, true);
        assert!(warnings.is_empty(), "{warnings:?}");
        app.preferences.keymap = keymap;
        copies(&mut app, std::slice::from_ref(&f5));
        assert!(app.palette.is_none(), "the copy ran nothing");
        assert!(app.task_list.copies.is_empty());
        // In a batch of its own, F5 now opens the palette, once.
        batch(&mut app, &[f5]);
        assert!(app.palette.is_some());
        assert!(app.task_list.copies.is_empty());
    }
}
