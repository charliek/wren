//! The task line commands (discovery/tasks.md §6, "Commands"): cycle the
//! status, set, raise or lower the priority, clear the due date, and the
//! done toggle with its stamps. Each rewrites the caret's task line, or for
//! the toggle every task the selection covers, through
//! `wren_markdown::task`: a [`Change`] planned as a line edit that carries
//! the line it expects, applied by the editor as one undo step. They run
//! on both engines, and only while the `tasks` setting is on: with it off
//! none is listed or runs, and `markdown.toggle_task` is the plain toggle.

use std::time::Instant;

use iced::Task;
use wren_markdown::task::{self, Change};
use wren_markdown::{Pos, Selection};

use super::commands::{Command, CommandId, Disposition, Gateway};
use super::{App, Message};

impl App {
    /// Whether the caret's line belongs to a task (its box's line, or a
    /// line of the item's text after it), so a line command has a task to
    /// change: never with tasks off, nor without a Markdown note in the
    /// editor on its text surface. What it finds is kept until the text,
    /// the caret's line or the engine changes, since the palette asks once
    /// per row on every refresh.
    pub(super) fn caret_on_task(&self) -> bool {
        if !self.task_lines_editable() {
            return false;
        }
        let key = self.memo_key();
        if let Some((kept, found)) = self.task_memo.on_task.get() {
            if kept == key {
                return found;
            }
        }
        let found = self.on_task(key.line);
        self.task_memo.on_task.set(Some((key, found)));
        found
    }

    /// Whether the editor holds task lines a command may change: tasks on,
    /// and a Markdown note in the editor on its text surface.
    pub(super) fn task_lines_editable(&self) -> bool {
        self.preferences.tasks && self.markdown_in_editor() && self.ensure_editor_ready().is_ok()
    }

    /// Whether line `line` of the editor's text belongs to a task.
    fn on_task(&self, line: usize) -> bool {
        // A change of no fields plans an edit wherever a task is.
        let nothing = Change::Compound(Vec::new());
        let caret = Selection::caret(Pos::new(line, 0));
        self.editor.read_markdown(|lines, options| {
            task::edit(lines, line, caret, &nothing, options).is_some()
        })
    }

    /// Run a task line command, or `markdown.toggle_task` with tasks on,
    /// at the editor's caret (the toggle: every task the selection covers),
    /// then give the editor the keyboard: from the palette, a key or the
    /// harness alike. A command that changes nothing (raising a Highest
    /// task) still completes; edits the editor refuses are a failure.
    pub(super) fn run_task_command(&mut self, id: CommandId) -> Gateway {
        self.run_task_command_at(id, None)
    }

    /// [`App::run_task_command`] on the task of `line` when it is given:
    /// the task a menu's row was chosen for, whatever line the caret is on.
    /// The toggle then toggles that one task.
    fn run_task_command_at(&mut self, id: CommandId, line: Option<usize>) -> Gateway {
        self.run_task_change(id.task_change(), line)
    }

    /// [`App::run_task_command_at`] for `change`, or the toggle for none:
    /// what a task menu's row and a click on a box make too.
    pub(super) fn run_task_change(
        &mut self,
        change: Option<Change>,
        line: Option<usize>,
    ) -> Gateway {
        // A done or cancelled stamp is today's, though the day changed
        // since the timer last looked.
        self.check_day();
        self.palette = None;
        let focus = self.return_to_editor();
        let started = Instant::now();
        let edits = self.editor.plan_markdown(|lines, sel, options| {
            let one = |at: usize, change: &Change| {
                task::edit(lines, at, sel, change, options).map(|edit| Ok(vec![edit]))
            };
            match (&change, line) {
                (Some(change), at) => one(at.unwrap_or(sel.head.line), change),
                (None, Some(at)) => one(at, &Change::ToggleDone),
                (None, None) => task::toggle(lines, sel, options),
            }
        });
        let edits = match edits {
            Some(Ok(edits)) => edits,
            Some(Err(refused)) => return self.blocked(refused.to_string()),
            None if change.is_none() && line.is_none() => {
                // A selection with no task in it: the plain toggle gives
                // its lines boxes, which an open box needs no stamp for.
                let task = self.edit(wren_editor::Message::Intent(
                    wren_editor::Intent::ToggleTask,
                ));
                return self.finish_action(Disposition::Completed, None, task.chain(focus));
            }
            None => return self.blocked("Put the caret on a task line"),
        };
        match self.editor.apply_line_edits(&edits) {
            Ok(changed) => {
                if changed {
                    self.text_changed(started);
                }
                self.finish_action(Disposition::Completed, None, focus)
            }
            Err(refused) => self.blocked(refused.to_string()),
        }
    }

    /// Run the command a key bound to it in the editor names
    /// ([`wren_editor::Message::Command`]): whatever popover is open closes
    /// first, as for any other key that acts on the text. A number that
    /// names no command does nothing.
    pub(super) fn run_payload(&mut self, payload: u32) -> Task<Message> {
        self.column = super::columns::Column::Editor;
        self.editor.close_popover();
        match CommandId::from_payload(payload) {
            Some(id) => self.dispatch(Command::Run(id)).task,
            None => Task::none(),
        }
    }

    /// Run the command a row chosen from a menu names, a row that runs one
    /// ([`wren_editor::RowKind::Command`], from
    /// [`wren_editor::EditorView::take_command`]), whoever opened the
    /// menu. A task line command acts on the task the menu was opened for,
    /// the line its anchor is on, which need not be the caret's; any other
    /// command runs as from the palette. A number that names no command
    /// does nothing.
    pub(super) fn run_command_row(&mut self, choice: &wren_editor::Choice) -> Task<Message> {
        self.column = super::columns::Column::Editor;
        let Some(id) = CommandId::from_payload(choice.id) else {
            return Task::none();
        };
        let at_line =
            id.task_change().is_some() || (id == CommandId::ToggleTask && self.preferences.tasks);
        if !at_line {
            return self.dispatch(Command::Run(id)).task;
        }
        let enabled = self.loading.is_none()
            && self.dialog.is_none()
            && self.task_lines_editable()
            && (id != CommandId::ToggleTask || self.editor.markdown_assist())
            && self.on_task(choice.line);
        if !enabled {
            return self
                .blocked("This command is unavailable in the current context")
                .task;
        }
        self.run_task_command_at(id, Some(choice.line)).task
    }
}

#[cfg(test)]
mod tests {
    use iced::widget::text_editor::{Cursor, Position};
    use wren_core::ops::{Op, Session};
    use wren_markdown::date::Date;

    use super::super::commands::{definitions, Command, CommandId, Disposition, Scope, GROUPS};
    use super::super::palette::{Key as Row, Mode};
    use super::super::tasks::Clock;
    use super::super::Options;
    use super::*;

    /// The Tasks group in the palette's order: the line commands, with
    /// the status and priority menus (item 3.4) and the calendar (item 4.1)
    /// before the commands of their kind, then the task form (item 6.1).
    const TASKS_GROUP: [CommandId; 14] = [
        CommandId::TaskSetStatus,
        CommandId::TaskCycleStatus,
        CommandId::TaskSetPriority,
        CommandId::TaskPriorityHigh,
        CommandId::TaskPriorityMedium,
        CommandId::TaskPriorityLow,
        CommandId::TaskPriorityHighest,
        CommandId::TaskPriorityLowest,
        CommandId::TaskPriorityNone,
        CommandId::TaskRaisePriority,
        CommandId::TaskLowerPriority,
        CommandId::TaskSetDue,
        CommandId::TaskClearDue,
        CommandId::TaskEdit,
    ];

    /// The line commands, in the palette's order.
    const LINE_COMMANDS: [CommandId; 10] = [
        CommandId::TaskCycleStatus,
        CommandId::TaskPriorityHigh,
        CommandId::TaskPriorityMedium,
        CommandId::TaskPriorityLow,
        CommandId::TaskPriorityHighest,
        CommandId::TaskPriorityLowest,
        CommandId::TaskPriorityNone,
        CommandId::TaskRaisePriority,
        CommandId::TaskLowerPriority,
        CommandId::TaskClearDue,
    ];

    const NOTE: &str = "# Tasks\n\
        \n\
        - [ ] Open task @sam 📅 2026-11-23\n\
        - [/] Doing task ⏫\n\
        - [x] Done task ✅ 2026-11-18\n\
        - [-] Cancelled task ❌ 2026-11-17\n\
        - [?] Unknown task\n\
        - [ ] VS16 task ⏫\u{fe0f} 📅 2026-12-01\n\
        - [ ] Alias task 🗓 2026-12-02 ^abc\n\
        \x20 its next line\n\
        \n\
        Plain text.\n\
        - a plain item\n";

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

    fn caret(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    fn line_of(text: &str, prefix: &str) -> usize {
        text.split('\n')
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix:?}"))
    }

    /// Run `id` with the caret at the end of `line`: the line after it,
    /// the caret's column, the disposition, and whether one undo brings
    /// the note back.
    fn run_at_end(app: &mut App, line: usize, id: CommandId) -> (String, usize, Disposition) {
        let before = app.editor.text();
        let end = before.split('\n').nth(line).unwrap().len();
        app.editor.move_cursor(caret(line, end));
        let outcome = app.dispatch(Command::Run(id));
        let text = app.editor.text();
        let draft = &app.session.as_ref().unwrap().document().unwrap().text;
        assert_eq!(&text, draft, "{id:?}: the edit reaches the session");
        let after = text.split('\n').nth(line).unwrap().to_owned();
        let column = app.editor.cursor().position.column;
        assert_eq!(app.editor.cursor().position.line, line, "{id:?}");
        if text != before {
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            assert_eq!(app.editor.text(), before, "{id:?}: one undo step");
            assert!(!app.editor.can_undo(), "{id:?}: one undo step and no more");
        }
        (after, column, outcome.processed.disposition)
    }

    #[test]
    fn each_line_command_rewrites_the_caret_line_on_both_engines() {
        use CommandId as C;
        let cases: &[(&str, CommandId, &str)] = &[
            // Cycle: open → doing → done (stamped) → open; cancelled and
            // unknown symbols as their types say.
            (
                "- [ ] Open",
                C::TaskCycleStatus,
                "- [/] Open task @sam 📅 2026-11-23",
            ),
            (
                "- [/] Doing",
                C::TaskCycleStatus,
                "- [x] Doing task ⏫ ✅ 2026-11-19",
            ),
            ("- [x] Done", C::TaskCycleStatus, "- [ ] Done task"),
            (
                "- [-] Cancelled",
                C::TaskCycleStatus,
                "- [ ] Cancelled task",
            ),
            ("- [?] Unknown", C::TaskCycleStatus, "- [/] Unknown task"),
            // Priority: written in Obsidian's order before the due date,
            // replacing any in place, bare, a U+FE0F after it included.
            (
                "- [ ] Open",
                C::TaskPriorityHigh,
                "- [ ] Open task @sam ⏫ 📅 2026-11-23",
            ),
            ("- [/] Doing", C::TaskPriorityMedium, "- [/] Doing task 🔼"),
            (
                "- [x] Done",
                C::TaskPriorityLow,
                "- [x] Done task 🔽 ✅ 2026-11-18",
            ),
            (
                "- [?] Unknown",
                C::TaskPriorityHighest,
                "- [?] Unknown task 🔺",
            ),
            (
                "- [-] Cancelled",
                C::TaskPriorityLowest,
                "- [-] Cancelled task ⏬ ❌ 2026-11-17",
            ),
            ("- [/] Doing", C::TaskPriorityNone, "- [/] Doing task"),
            (
                "- [ ] VS16",
                C::TaskPriorityMedium,
                "- [ ] VS16 task 🔼 📅 2026-12-01",
            ),
            (
                "- [ ] VS16",
                C::TaskPriorityNone,
                "- [ ] VS16 task 📅 2026-12-01",
            ),
            ("- [/] Doing", C::TaskRaisePriority, "- [/] Doing task 🔺"),
            ("- [/] Doing", C::TaskLowerPriority, "- [/] Doing task 🔼"),
            (
                "- [ ] Open",
                C::TaskRaisePriority,
                "- [ ] Open task @sam 🔼 📅 2026-11-23",
            ),
            (
                "- [ ] Open",
                C::TaskLowerPriority,
                "- [ ] Open task @sam 🔽 📅 2026-11-23",
            ),
            (
                "- [ ] VS16",
                C::TaskRaisePriority,
                "- [ ] VS16 task 🔺 📅 2026-12-01",
            ),
            (
                "- [?] Unknown",
                C::TaskLowerPriority,
                "- [?] Unknown task 🔽",
            ),
            // Clear due: any due token, an alias too; nothing else moves.
            ("- [ ] Open", C::TaskClearDue, "- [ ] Open task @sam"),
            ("- [ ] VS16", C::TaskClearDue, "- [ ] VS16 task ⏫\u{fe0f}"),
            ("- [ ] Alias", C::TaskClearDue, "- [ ] Alias task ^abc"),
            // Nothing to change: the line stays, and the command completes.
            (
                "- [x] Done",
                C::TaskClearDue,
                "- [x] Done task ✅ 2026-11-18",
            ),
        ];
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            for (prefix, id, after) in cases {
                let line = line_of(NOTE, prefix);
                let (got, column, disposition) = run_at_end(&mut app, line, *id);
                assert_eq!(got, *after, "{engine} {id:?} on {prefix:?}");
                assert_eq!(disposition, Disposition::Completed, "{engine} {id:?}");
                // The caret stays at the end of what it was after: a token
                // written at the end goes after it.
                assert!(column <= got.len(), "{engine} {id:?}");
            }
            // Raising a Highest task, or lowering a Lowest one, changes
            // nothing and completes.
            let (_directory, mut app) = note_app(engine, "on", "- [ ] top 🔺\n- [ ] bottom ⏬\n");
            for (line, id, after) in [
                (0, C::TaskRaisePriority, "- [ ] top 🔺"),
                (1, C::TaskLowerPriority, "- [ ] bottom ⏬"),
            ] {
                let (got, _, disposition) = run_at_end(&mut app, line, id);
                assert_eq!((got.as_str(), disposition), (after, Disposition::Completed));
                assert!(
                    !app.editor.can_undo(),
                    "{engine}: no undo step for no change"
                );
            }
        }
    }

    #[test]
    fn the_caret_keeps_its_place_in_the_text() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            let line = line_of(NOTE, "- [/] Doing");
            // In the description, the caret stays on its character.
            app.editor.move_cursor(caret(line, "- [/] Doi".len()));
            let _ = app.dispatch(Command::Run(CommandId::TaskCycleStatus));
            assert_eq!(
                app.editor.cursor(),
                caret(line, "- [x] Doi".len()),
                "{engine}"
            );
            // At the end, a stamp goes after it.
            let end = "- [x] Doing task ⏫".len();
            app.editor.move_cursor(caret(line, end));
            let _ = app.dispatch(Command::Run(CommandId::TaskCycleStatus));
            assert_eq!(
                app.editor.cursor(),
                caret(line, "- [ ] Doing task ⏫".len()),
                "{engine}"
            );
            // On a line after the box, the box's line changes and the
            // caret does not move.
            let next = line_of(NOTE, "  its next line");
            app.editor.move_cursor(caret(next, 3));
            let _ = app.dispatch(Command::Run(CommandId::TaskPriorityHigh));
            let text = app.editor.text();
            assert_eq!(
                text.split('\n').nth(next - 1),
                Some("- [ ] Alias task ⏫ 🗓 2026-12-02 ^abc"),
                "{engine}"
            );
            assert_eq!(app.editor.cursor(), caret(next, 3), "{engine}");
        }
    }

    #[test]
    fn off_a_task_line_every_line_command_is_unavailable_and_changes_nothing() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            for prefix in ["# Tasks", "Plain text.", "- a plain item"] {
                let line = line_of(NOTE, prefix);
                app.editor.move_cursor(caret(line, 0));
                assert!(!app.caret_on_task(), "{engine} {prefix:?}");
                for id in LINE_COMMANDS {
                    let outcome = app.dispatch(Command::Run(id));
                    assert_eq!(
                        outcome.processed.disposition,
                        Disposition::Blocked,
                        "{engine} {id:?} on {prefix:?}"
                    );
                    assert_eq!(app.editor.text(), NOTE, "{engine} {id:?}");
                }
            }
            // A blank line is no task's either.
            app.editor.move_cursor(caret(1, 0));
            assert!(!app.caret_on_task(), "{engine}");
            app.editor
                .move_cursor(caret(line_of(NOTE, "- [ ] Open"), 0));
            assert!(app.caret_on_task(), "{engine}");
        }
    }

    /// The palette's Tasks group: its rows in Q7's order, enabled only on
    /// a task line, and absent with tasks off.
    #[test]
    fn the_palette_lists_the_tasks_group_only_with_tasks_on() {
        let listed = |app: &mut App| -> Vec<(CommandId, &'static str, bool)> {
            let _ = app.open_palette(Mode::Commands);
            let rows = app
                .palette
                .as_ref()
                .unwrap()
                .results
                .iter()
                .filter_map(|row| match row.key {
                    Row::Command(id) => Some((id, row.group, row.enabled)),
                    _ => None,
                })
                .collect();
            let _ = app.close_palette();
            rows
        };
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [ ] Open"), 0));
        let rows = listed(&mut app);
        let tasks: Vec<(CommandId, bool)> = rows
            .iter()
            .filter(|(id, group, _)| *group == "Tasks" && !id.task_entry())
            .map(|(id, _, enabled)| (*id, *enabled))
            .collect();
        assert_eq!(
            tasks,
            TASKS_GROUP.map(|id| (id, true)).to_vec(),
            "every line command, enabled on a task line"
        );
        // Show tasks (item 10.2) ends the group, the app's own command.
        assert_eq!(
            rows.iter()
                .rfind(|(_, group, _)| *group == "Tasks")
                .map(|(id, _, enabled)| (*id, *enabled)),
            Some((CommandId::ShowTasks, true))
        );
        // The group follows Markdown's among the groups a note lists.
        let groups: Vec<&str> = rows.iter().map(|(_, group, _)| *group).collect();
        let markdown = groups
            .iter()
            .rposition(|group| *group == "Markdown")
            .unwrap();
        assert_eq!(groups[markdown + 1], "Tasks");
        app.editor
            .move_cursor(caret(line_of(NOTE, "Plain text."), 0));
        // Off a task line only the form is enabled: it makes the line one.
        // Show tasks needs no task line.
        let disabled = listed(&mut app);
        assert!(disabled
            .iter()
            .filter(|(_, group, _)| *group == "Tasks")
            .all(|(id, _, enabled)| *enabled
                == matches!(*id, CommandId::TaskEdit | CommandId::ShowTasks)));

        let (_directory, mut app) = note_app("advanced", "off", NOTE);
        app.editor
            .move_cursor(caret(line_of(NOTE, "- [ ] Open"), 0));
        let rows = listed(&mut app);
        assert!(rows
            .iter()
            .all(|(id, group, _)| *group != "Tasks" && !id.tasks()));
        for id in LINE_COMMANDS {
            let outcome = app.dispatch(Command::Run(id));
            assert_eq!(
                outcome.processed.disposition,
                Disposition::Blocked,
                "{id:?}"
            );
            assert_eq!(app.editor.text(), NOTE);
        }
        assert!(GROUPS.contains(&"Tasks"));
    }

    /// The table's rules for the task commands: a stable `task.` key, a
    /// label, the Tasks group, Editor scope, routed back to the app as
    /// themselves, no default key, and out of the macOS menu table.
    #[test]
    fn the_task_commands_keep_the_command_tables_rules() {
        // The entry points (item 10.2) are App commands of their own,
        // checked in `task_entry.rs`; these rules are the line commands'.
        let rows: Vec<_> = definitions()
            .filter(|row| row.id.tasks() && !row.id.task_entry())
            .collect();
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            TASKS_GROUP
        );
        for row in &rows {
            assert!(row.key.starts_with("task."), "{}", row.key);
            assert!(row.label.contains("task"), "{}", row.label);
            assert_eq!(row.group, "Tasks", "{}", row.key);
            assert_eq!(row.scope, Scope::Editor, "{}", row.key);
            assert_eq!(CommandId::parse(row.key), Some(row.id));
            // A menu's command is a request for the menu (item 3.4), and
            // so are the calendar's (item 4.1) and the form's (item 6.1).
            let opens = row.id.task_menu().is_some()
                || matches!(row.id, CommandId::TaskSetDue | CommandId::TaskEdit);
            let route = if opens {
                wren_editor::KeyRoute::Popover(row.id.payload())
            } else {
                wren_editor::KeyRoute::Command(row.id.payload())
            };
            assert_eq!(row.id.editor_route(), Some(route), "{}", row.key);
            assert!(row.id.task_change().is_some() != opens);
        }
        for scheme in super::super::preferences::SCHEMES {
            let defaults = wren_core::keybind::default_bindings(scheme)
                .into_iter()
                .chain(wren_core::keybind::default_task_bindings(scheme));
            for (trigger, action) in defaults {
                // ⌘D / Ctrl+D sets the due date (item 4.1) and ⇧⌘↵ /
                // Ctrl+Shift+Enter opens the form (item 6.1; design §2 item
                // 21).
                assert!(
                    !action.starts_with("task.") || matches!(action, "task.set_due" | "task.edit"),
                    "{scheme:?}: {trigger} → {action} (Q13: palette only)"
                );
                // Of the entry points only Show tasks has a key (§10,
                // 2026-11-26).
                let id = CommandId::parse(action).expect("a default names a command");
                assert!(
                    !id.task_entry() || id == CommandId::ShowTasks,
                    "{scheme:?}: {trigger} → {action}"
                );
            }
        }
        // Being Editor scope, none is in the macOS menu table, which holds
        // App and Board commands only (`menu.rs`'s own invariant).
        // Payloads name their commands, and a plain number names none.
        for row in definitions() {
            assert_eq!(CommandId::from_payload(row.id.payload()), Some(row.id));
        }
        for plain in [0, 1, 2, 7, 1000] {
            assert_eq!(CommandId::from_payload(plain), None);
        }
        assert_eq!(CommandId::from_payload(u32::MAX), None);
        // The toggle stays a Markdown command whose key is the editor's
        // intent; the app reroutes it only with tasks on (preferences).
        assert!(!CommandId::ToggleTask.tasks());
        assert_eq!(
            CommandId::ToggleTask.editor_route(),
            Some(wren_editor::KeyRoute::Intent(
                wren_editor::Intent::ToggleTask
            ))
        );
    }

    /// A `keybind` line binds a task command with tasks on, and its key
    /// runs the command from the focused editor; with tasks off the line is
    /// refused and the key types as before.
    #[test]
    fn a_bound_task_command_runs_from_its_key_only_with_tasks_on() {
        use iced::keyboard::{Key, Modifiers};
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let keybinds = vec![("alt+shift+c".to_owned(), "task.cycle_status".to_owned())];
        let (keymap, warnings) = super::super::preferences::build_keymap(
            wren_core::keybind::Scheme::default(),
            &keybinds,
            true,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        app.preferences.keymap = keymap;
        app.apply_preferences();
        let line = line_of(NOTE, "- [ ] Open");
        app.editor.move_cursor(caret(line, 3));
        let messages = app.editor.key_message(
            Key::Character("c".into()),
            Modifiers::ALT | Modifiers::SHIFT,
        );
        assert!(
            matches!(messages.as_slice(), [wren_editor::Message::Command(payload)]
                if *payload == CommandId::TaskCycleStatus.payload()),
            "{messages:?}"
        );
        for message in messages {
            let _ = app.update(Message::Editor(message));
        }
        assert_eq!(
            app.editor.text().split('\n').nth(line),
            Some("- [/] Open task @sam 📅 2026-11-23")
        );

        let (_, warnings) = super::super::preferences::build_keymap(
            wren_core::keybind::Scheme::default(),
            &keybinds,
            false,
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    /// `markdown.toggle_task` with tasks on writes the stamps its status
    /// change asks for, from the palette and from its key, on both
    /// engines; typed by hand, an `x` writes none (Q11).
    #[test]
    fn the_toggle_stamps_with_tasks_on_from_its_key_and_the_palette() {
        use iced::keyboard::key::Named;
        use iced::keyboard::{Key, Modifiers};
        use wren_core::keybind::Platform;

        let toggle_key = match Platform::current() {
            Platform::Mac => Modifiers::LOGO,
            Platform::Linux => Modifiers::CTRL,
        };
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", NOTE);
            let open = line_of(NOTE, "- [ ] Open");
            let (after, column, disposition) = run_at_end(&mut app, open, CommandId::ToggleTask);
            assert_eq!(
                after, "- [x] Open task @sam 📅 2026-11-23 ✅ 2026-11-19",
                "{engine}"
            );
            assert_eq!(
                column,
                "- [x] Open task @sam 📅 2026-11-23".len(),
                "{engine}"
            );
            assert_eq!(disposition, Disposition::Completed);
            for (prefix, after) in [
                ("- [/] Doing", "- [x] Doing task ⏫ ✅ 2026-11-19"),
                ("- [x] Done", "- [ ] Done task"),
                ("- [-] Cancelled", "- [ ] Cancelled task"),
                ("- [?] Unknown", "- [x] Unknown task ✅ 2026-11-19"),
            ] {
                let line = line_of(NOTE, prefix);
                let (got, _, _) = run_at_end(&mut app, line, CommandId::ToggleTask);
                assert_eq!(got, after, "{engine} {prefix:?}");
            }

            // The key: routed to the app as the command, not the editor's
            // plain intent.
            app.editor.move_cursor(caret(open, 0));
            let messages = app.editor.key_message(Key::Named(Named::Enter), toggle_key);
            assert!(
                matches!(messages.as_slice(), [wren_editor::Message::Command(payload)]
                    if *payload == CommandId::ToggleTask.payload()),
                "{engine}: {messages:?}"
            );
            for message in messages {
                let _ = app.update(Message::Editor(message));
            }
            assert_eq!(
                app.editor.text().split('\n').nth(open),
                Some("- [x] Open task @sam 📅 2026-11-23 ✅ 2026-11-19"),
                "{engine}"
            );
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            assert_eq!(app.editor.text(), NOTE, "{engine}");

            // Typing `x` into the box is plain typing: no stamp.
            app.editor.move_cursor(caret(open, 3));
            let _ = app.update(Message::Editor(wren_editor::Message::Action(
                iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Delete),
            )));
            let _ = app.update(Message::Editor(wren_editor::Message::Action(
                iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert(
                    'x',
                )),
            )));
            assert_eq!(
                app.editor.text().split('\n').nth(open),
                Some("- [x] Open task @sam 📅 2026-11-23"),
                "{engine}"
            );
        }
    }

    /// Over a selection the toggle follows the plain toggle's rule, with
    /// stamps, as one undo step; a selection with no task gives its lines
    /// boxes as the plain toggle does.
    #[test]
    fn the_toggle_over_a_selection_is_one_undo_step_with_stamps() {
        let text = "- [ ] a\n- [-] b ❌ 2026-11-17\n- [X] c ✅ 2026-11-01\nprose\n\nplain\nmore\n";
        let selected = |app: &mut App, from: usize, to: usize| {
            let end = app.editor.text().split('\n').nth(to).unwrap().len();
            app.editor.move_cursor(Cursor {
                position: Position {
                    line: to,
                    column: end,
                },
                selection: Some(Position {
                    line: from,
                    column: 0,
                }),
            });
            app.dispatch(Command::Run(CommandId::ToggleTask))
        };
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", text);
            let outcome = selected(&mut app, 0, 3);
            assert_eq!(outcome.processed.disposition, Disposition::Completed);
            assert_eq!(
                app.editor.text(),
                "- [x] a ✅ 2026-11-19\n- [x] b ✅ 2026-11-19\n- [X] c ✅ 2026-11-01\nprose\n\nplain\nmore\n",
                "{engine}"
            );
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            assert_eq!(app.editor.text(), text, "{engine}: one undo step");
            assert!(!app.editor.can_undo(), "{engine}");
            // Only finished tasks: they all open, losing their stamps.
            let _ = selected(&mut app, 1, 2);
            assert_eq!(
                app.editor.text(),
                "- [ ] a\n- [ ] b\n- [ ] c\nprose\n\nplain\nmore\n",
                "{engine}"
            );
            let _ = app.update(Message::Editor(wren_editor::Message::Undo));
            // No task: boxes, as the plain toggle gives them.
            let _ = selected(&mut app, 5, 6);
            assert_eq!(
                app.editor.text(),
                "- [ ] a\n- [-] b ❌ 2026-11-17\n- [X] c ✅ 2026-11-01\nprose\n\n- [ ] plain\n- [ ] more\n",
                "{engine}"
            );
        }
    }

    /// With tasks off the toggle is the plain one: no stamp, from the key
    /// or the palette.
    #[test]
    fn with_tasks_off_the_toggle_writes_no_stamp() {
        use iced::keyboard::key::Named;
        use iced::keyboard::{Key, Modifiers};
        use wren_core::keybind::Platform;

        let toggle_key = match Platform::current() {
            Platform::Mac => Modifiers::LOGO,
            Platform::Linux => Modifiers::CTRL,
        };
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "off", NOTE);
            let open = line_of(NOTE, "- [ ] Open");
            let (after, _, _) = run_at_end(&mut app, open, CommandId::ToggleTask);
            assert_eq!(after, "- [x] Open task @sam 📅 2026-11-23", "{engine}");
            app.editor.move_cursor(caret(open, 0));
            let messages = app.editor.key_message(Key::Named(Named::Enter), toggle_key);
            assert!(
                matches!(
                    messages.as_slice(),
                    [wren_editor::Message::Intent(
                        wren_editor::Intent::ToggleTask
                    )]
                ),
                "{engine}: {messages:?}"
            );
        }
    }

    /// A row that runs a command runs it when chosen, by its accelerator
    /// or Enter; a row of any other kind runs nothing, whatever its number,
    /// a command's payload included.
    #[test]
    fn only_a_command_row_runs_its_command() {
        use iced::keyboard::key::Named;
        use iced::keyboard::{Key, Modifiers};
        use wren_editor::{Accelerator, Anchor, Mode as PopoverMode, Row as PopoverRow};

        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let line = line_of(NOTE, "- [/] Doing");
        let press = |app: &mut App, key: Key| {
            for message in app.editor.key_message(key, Modifiers::empty()) {
                let _ = app.update(Message::Editor(message));
            }
        };
        let open = |app: &mut App, rows: Vec<PopoverRow>| {
            app.editor.move_cursor(caret(line, 3));
            assert!(app
                .editor
                .open_popover(Anchor::Caret, PopoverMode::Menu, rows));
        };
        open(
            &mut app,
            vec![
                PopoverRow::command(CommandId::TaskPriorityLow.payload(), "Low")
                    .accelerator(Accelerator::plain('l')),
            ],
        );
        press(&mut app, Key::Character("l".into()));
        assert!(app.editor.popover().is_none());
        let low = replaced_line(NOTE, line, "- [/] Doing task 🔽");
        assert_eq!(app.editor.text(), low);

        // `0x8000_0000` is `note.new`'s payload, the table's first row.
        assert_eq!(
            CommandId::from_payload(0x8000_0000),
            Some(CommandId::NewNote)
        );
        for id in [
            0,
            7,
            0x8000_0000,
            CommandId::TaskPriorityHigh.payload(),
            CommandId::ToggleTask.payload(),
            u32::MAX,
        ] {
            open(&mut app, vec![PopoverRow::new(id, "Plain")]);
            press(&mut app, Key::Named(Named::Enter));
            assert!(app.editor.popover().is_none(), "{id:#x}");
            assert_eq!(app.editor.text(), low, "{id:#x}");
            assert!(app.dialog.is_none() && app.palette.is_none(), "{id:#x}");
        }
    }

    /// A row that runs a task command acts on the task its menu was opened
    /// for, as a provider's menu for a right-click is (its anchor's line),
    /// not the caret's: the caret on task A, a menu hung on task B.
    #[test]
    fn a_command_row_acts_on_the_task_its_menu_was_opened_for() {
        use wren_editor::{Anchor, Mode as PopoverMode, Row as PopoverRow};

        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let (a, b) = (
            line_of(NOTE, "- [ ] Open"),
            line_of(NOTE, "- [-] Cancelled"),
        );
        let choose = |app: &mut App, row: usize| {
            app.editor.move_cursor(caret(a, 4));
            let rows = vec![
                PopoverRow::command(CommandId::TaskPriorityHigh.payload(), "High"),
                PopoverRow::command(CommandId::ToggleTask.payload(), "Toggle"),
            ];
            let anchor = Anchor::Range {
                line: b,
                range: 0..1,
            };
            assert!(app.editor.open_popover(anchor, PopoverMode::Menu, rows));
            let _ = app.update(Message::Editor(wren_editor::Message::Spell(
                wren_editor::SpellEvent::Accept(row),
            )));
            assert!(app.editor.popover().is_none());
        };
        choose(&mut app, 0);
        let high = replaced_line(NOTE, b, "- [-] Cancelled task ⏫ ❌ 2026-11-17");
        assert_eq!(app.editor.text(), high, "B has the priority, A not");
        assert_eq!(app.editor.cursor(), caret(a, 4), "the caret stays on A");
        choose(&mut app, 1);
        assert_eq!(
            app.editor.text(),
            replaced_line(&high, b, "- [ ] Cancelled task ⏫"),
            "the toggle reopens B alone"
        );
        let draft = &app.session.as_ref().unwrap().document().unwrap().text;
        assert_eq!(draft, &app.editor.text(), "the edits reach the session");
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), NOTE);
        // A menu hung on a line of no task runs nothing there.
        let plain = line_of(NOTE, "Plain text.");
        app.editor.move_cursor(caret(a, 4));
        let anchor = Anchor::Range {
            line: plain,
            range: 0..1,
        };
        let rows = vec![PopoverRow::command(
            CommandId::TaskCycleStatus.payload(),
            "Cycle",
        )];
        assert!(app.editor.open_popover(anchor, PopoverMode::Menu, rows));
        let _ = app.update(Message::Editor(wren_editor::Message::Spell(
            wren_editor::SpellEvent::Accept(0),
        )));
        assert_eq!(app.editor.text(), NOTE, "not A's either");
    }

    /// `text` with line `line` replaced by `new`.
    fn replaced_line(text: &str, line: usize, new: &str) -> String {
        let mut lines: Vec<&str> = text.split('\n').collect();
        lines[line] = new;
        lines.join("\n")
    }

    /// A selection over a parent task, its child and the parent's text
    /// after the child's toggles each task once: an edit apiece, which the
    /// editor applies together.
    #[test]
    fn a_selection_over_a_parent_and_its_child_toggles_each_once() {
        let text = "- [ ] parent\n  - [ ] child\n\n  parent continuation\n";
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", text);
            app.editor.move_cursor(Cursor {
                position: Position {
                    line: 3,
                    column: 21,
                },
                selection: Some(Position { line: 0, column: 0 }),
            });
            let outcome = app.dispatch(Command::Run(CommandId::ToggleTask));
            assert_eq!(outcome.processed.disposition, Disposition::Completed);
            assert_eq!(
                app.editor.text(),
                "- [x] parent ✅ 2026-11-19\n  - [x] child ✅ 2026-11-19\n\n  parent continuation\n",
                "{engine}"
            );
        }
    }

    /// The harness drives any popover: `editor.open_menu` opens a menu of
    /// command rows, `editor.get` reports it whatever it is, and
    /// `editor.key` presses a row's key, whose command then runs.
    #[cfg(feature = "smoke-test")]
    #[test]
    fn the_harness_presses_keys_into_any_popover() {
        use serde_json::json;

        let call = |app: &mut App, op: &str, params: serde_json::Value| {
            let (request, mut reply) = crate::test_ipc::Request::detached(op, params);
            let _ = app.handle_test(request);
            reply.try_recv().unwrap()
        };
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        let line = line_of(NOTE, "- [/] Doing");
        call(&mut app, "editor.cursor", json!({"line":line,"column":3})).unwrap();
        let rows = json!({"rows":[
            {"command":"task.priority.high","label":"High","key":"h"},
            {"command":"task.priority.highest","label":"Highest","key":"h","shift":true},
            {"command":"task.cycle_status"},
        ]});
        let snapshot = call(&mut app, "editor.open_menu", rows.clone()).unwrap();
        let popover = &snapshot["popover"];
        assert_eq!(popover["mode"], "menu");
        assert_eq!(popover["spelling"], false);
        assert_eq!(popover["line"], line);
        assert_eq!(popover["selected"], 0);
        assert_eq!(popover["rows"][1]["command"], "task.priority.highest");
        assert_eq!(popover["rows"][2]["label"], "task.cycle_status");
        let snapshot = call(&mut app, "editor.key", json!({"key":"h","mods":["shift"]})).unwrap();
        assert!(snapshot["popover"].is_null());
        assert_eq!(
            snapshot["text"].as_str().unwrap().split('\n').nth(line),
            Some("- [/] Doing task 🔺")
        );
        // Enter chooses the selected row; any other key closes the menu
        // and types.
        call(&mut app, "editor.open_menu", rows.clone()).unwrap();
        call(&mut app, "editor.key", json!({"key":"down"})).unwrap();
        call(&mut app, "editor.key", json!({"key":"down"})).unwrap();
        let snapshot = call(&mut app, "editor.key", json!({"key":"enter"})).unwrap();
        assert_eq!(
            snapshot["text"].as_str().unwrap().split('\n').nth(line),
            Some("- [x] Doing task 🔺 ✅ 2026-11-19")
        );
        // A key bound to a task command reaches the app from the editor.
        let error = call(&mut app, "shortcut", json!({"id":"task.cycle_status"}))
            .expect_err("no key is bound by default");
        assert!(error.contains("no key"), "{error}");
        // The classic engine opens no menu of the app's.
        let (_directory, mut app) = note_app("classic", "on", NOTE);
        let error = call(&mut app, "editor.open_menu", rows).expect_err("classic");
        assert!(error.contains("no menu"), "{error}");
    }

    /// The harness reaches the app as the widget does: `editor.command`'s
    /// toggle writes the stamps with tasks on; `editor.open_menu` refuses
    /// a command this run does not have and a menu under the palette; and
    /// `editor.key` leaves uncaptured a shortcut that only closes a menu,
    /// so the app runs it.
    #[cfg(feature = "smoke-test")]
    #[test]
    fn the_harness_routes_keys_and_menus_as_the_widget_does() {
        use serde_json::json;

        let call = |app: &mut App, op: &str, params: serde_json::Value| {
            let (request, mut reply) = crate::test_ipc::Request::detached(op, params);
            let _ = app.handle_test(request);
            reply.try_recv().unwrap()
        };
        let open = line_of(NOTE, "- [ ] Open");
        let (_directory, mut app) = note_app("advanced", "on", NOTE);
        call(&mut app, "editor.cursor", json!({"line":open,"column":3})).unwrap();
        let snapshot = call(&mut app, "editor.command", json!({"intent":"toggle_task"})).unwrap();
        assert_eq!(
            snapshot["text"].as_str().unwrap().split('\n').nth(open),
            Some("- [x] Open task @sam 📅 2026-11-23 ✅ 2026-11-19")
        );

        // A shortcut of the app's with a menu open: the menu closes and the
        // app runs the shortcut, here the command palette.
        let rows = json!({"rows":[{"command":"task.cycle_status","key":"c"}]});
        let snapshot = call(&mut app, "editor.open_menu", rows.clone()).unwrap();
        assert_eq!(
            snapshot["popover"]["rows"][0]["command"],
            "task.cycle_status"
        );
        let trigger = app
            .preferences
            .keymap
            .accel("palette.commands")
            .unwrap()
            .clone();
        let mut mods = vec![];
        for (held, name) in [
            (trigger.mods.shift, "shift"),
            (trigger.mods.ctrl, "ctrl"),
            (trigger.mods.alt, "alt"),
            (trigger.mods.logo, "logo"),
        ] {
            if held {
                mods.push(name);
            }
        }
        let snapshot = call(
            &mut app,
            "editor.key",
            json!({"key":trigger.key,"mods":mods}),
        )
        .unwrap();
        assert!(snapshot["popover"].is_null());
        assert!(app.palette.is_some(), "the shortcut reached the app");
        // No menu opens under the palette.
        let error = call(&mut app, "editor.open_menu", rows.clone()).expect_err("the palette");
        assert!(error.contains("palette"), "{error}");
        assert!(app.editor.popover().is_none());
        let _ = app.close_palette();

        // With tasks off, a task command is no row.
        let (_directory, mut app) = note_app("advanced", "off", NOTE);
        let error = call(&mut app, "editor.open_menu", rows).expect_err("tasks off");
        assert!(error.contains("unavailable"), "{error}");
        assert!(app.editor.popover().is_none());
        let rows = json!({"rows":[{"command":"markdown.bold"}]});
        call(&mut app, "editor.open_menu", rows).unwrap();
        call(&mut app, "editor.cursor", json!({"line":open,"column":3})).unwrap();
        let snapshot = call(&mut app, "editor.command", json!({"intent":"toggle_task"})).unwrap();
        assert_eq!(
            snapshot["text"].as_str().unwrap().split('\n').nth(open),
            Some("- [x] Open task @sam 📅 2026-11-23")
        );
    }

    /// Midnight passed after the timer last looked: the toggle stamps the
    /// new day, not the one the editor was last told of.
    #[test]
    fn a_toggle_after_midnight_stamps_today() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = note_app(engine, "on", "- [ ] x\n");
            app.sync_task_options();
            app.clock
                .pass_midnight(Date::parse_iso("2026-11-20").unwrap());
            let (after, _, disposition) = run_at_end(&mut app, 0, CommandId::ToggleTask);
            assert_eq!(after, "- [x] x ✅ 2026-11-20", "{engine}");
            assert_eq!(disposition, Disposition::Completed);
        }
    }
}
