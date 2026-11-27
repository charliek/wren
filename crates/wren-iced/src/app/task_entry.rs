//! The ways into the task view, and the two ways to leave tasks out of it
//! (discovery/tasks.md §2 items 21, 24 and 28, §7, §9; tasks-build item
//! 10.2):
//!
//! - **Header counts.** Each column's header counts the open and doing
//!   tasks of what the column belongs to: Sections the workspace, Groups
//!   the section, Notes the group. The number is red while one of them is
//!   overdue, and the count is pressed while the view of its scope shows.
//!   A click opens that view. `task-counts = off` (and
//!   `view.toggle_task_counts`) hides them.
//! - **`tasks.show`** (Mod+Shift+T) opens the view of where the keyboard
//!   is ([`App::show_tasks_scope`]).
//! - **A row's actions** (the item-actions palette a right-click opens):
//!   Show tasks in it, on every row; Hide tasks from this group, on a
//!   group, kept in `.wren/tasks.toml`; Hide tasks in this note, on a
//!   Markdown note, written into its front matter (`task_target.rs`). Each
//!   hide shows again once done, and leaves a message with Undo.
//! - **Marks.** A note whose front matter hides its tasks is marked
//!   "tasks off" in the Notes column, and an excluded group "hidden".
//!
//! With tasks off none of it exists: no count, mark, command, key or row.

use iced::widget::{button, container, row, tooltip};
use iced::Element;
use wren_core::fs_store::RelPath;
use wren_core::model::{GroupId, ItemRef, NoteId, Parent};
use wren_core::ops::Op;
use wren_core::tasks::Counts;

use super::columns::Column;
use super::commands::{Command, CommandId, Gateway};
use super::notice::{Notice, Part, Undo};
use super::tasks_view::Scope;
use super::{App, Message};
use crate::chrome::{self, Icon};
use crate::{fonts, theme};

/// What a column's header counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeaderCount {
    pub scope: Scope,
    pub counts: Counts,
    /// The view of this scope is what the pane shows.
    pub pressed: bool,
}

/// How a group is left out of task views: by its own entry in
/// `.wren/tasks.toml`, and by its section's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GroupHidden {
    pub own: bool,
    pub inherited: bool,
}

/// The title of the hide on a group, or `None` when it offers none: a
/// group its section alone hides has no entry of its own to change, and a
/// group hidden both ways only loses its own entry, which brings nothing
/// back while its section is out.
fn group_hide_title(hidden: GroupHidden) -> Option<&'static str> {
    match (hidden.own, hidden.inherited) {
        (false, false) => Some("Hide tasks from this group"),
        (true, false) => Some("Show this group\u{2019}s tasks again"),
        (true, true) => Some("Remove this group\u{2019}s own exclusion"),
        (false, true) => None,
    }
}

fn note_hide_title(hidden: bool) -> &'static str {
    if hidden {
        "Show tasks in this note again"
    } else {
        "Hide tasks in this note"
    }
}

impl App {
    /// The scope `tasks.show` opens (prototype `scopeFromActive`): from
    /// the Sections column the selected section, from the Groups column
    /// the selected group, from the Notes column the group it lists, and
    /// from the editor or a board the open note's group. A view in the
    /// pane with the keyboard opens itself again. What is missing falls
    /// back to what holds it, and at last to the workspace.
    pub(super) fn show_tasks_scope(&self) -> Option<Scope> {
        let session = self.session.as_ref()?;
        let model = session.model();
        if self.column == Column::Editor {
            if let Some(scope) = self.task_view() {
                return Some(scope.clone());
            }
        }
        let section = self
            .selected_section
            .and_then(|section| Scope::of(ItemRef::Section(section), model));
        let group = |group: Option<GroupId>| {
            group.and_then(|group| Scope::of(ItemRef::Group(group), model))
        };
        let scope = match self.column {
            Column::Sections => section,
            Column::Groups | Column::Notes => group(self.selected_group).or(section),
            Column::Editor => {
                let open = session
                    .document()
                    .and_then(|document| model.note(document.note))
                    .map(|note| note.group);
                group(open.or(self.selected_group)).or(section)
            }
        };
        Some(scope.unwrap_or(Scope::Workspace))
    }

    /// Whether the columns' headers count tasks: tasks on, and the
    /// `task-counts` setting.
    pub(super) fn task_counts_shown(&self) -> bool {
        self.preferences.tasks && self.preferences.settings.task_counts
    }

    /// What `column`'s header counts, when it counts.
    pub(super) fn header_count(&self, column: Column) -> Option<HeaderCount> {
        if !self.task_counts_shown() || self.loading.is_some() {
            return None;
        }
        let session = self.session.as_ref()?;
        let model = session.model();
        let scope = match column {
            Column::Sections => Scope::Workspace,
            Column::Groups => Scope::of(ItemRef::Section(self.browser_section()?), model)?,
            Column::Notes => match self.browser_parent()? {
                Parent::Group(group) => Scope::of(ItemRef::Group(group), model)?,
                Parent::Section(_) => return None,
            },
            Column::Editor => return None,
        };
        let counts = session.tasks()?.counts_under(scope.prefix(), self.today());
        let pressed = self.task_view() == Some(&scope);
        Some(HeaderCount {
            scope,
            counts,
            pressed,
        })
    }

    /// `column`'s count, for its header: the tasks icon and the number,
    /// red while one is overdue, pressed while its view shows. A click
    /// opens the view.
    pub(super) fn header_count_view(&self, column: Column) -> Option<Element<'_, Message>> {
        let count = self.header_count(column)?;
        let definition = self.definition;
        let colors = definition.colors;
        let pressed = count.pressed;
        let ink = if pressed { colors.accent } else { colors.muted };
        let number = if count.counts.overdue > 0 {
            colors.danger
        } else {
            ink
        };
        let face = row![
            chrome::icon(Icon::Tasks, 13.0, ink),
            chrome::label(count.counts.open_and_doing.to_string(), 11, number)
                .font(fonts::SEMIBOLD),
        ]
        .spacing(4)
        .align_y(iced::Center);
        let tip = match self.shortcut_label(CommandId::ShowTasks.key()) {
            Some(key) => format!("Tasks in {} ({key})", self.task_view_label(&count.scope)),
            None => format!("Tasks in {}", self.task_view_label(&count.scope)),
        };
        let pressable = button(face)
            .height(22)
            .padding([0, 6])
            .on_press(Message::Command(Command::ShowTasks(count.scope)))
            .style(move |_, status| chrome::control(definition, status, pressed, true));
        let placed = container(pressable).id(format!("task-count-{}", column.key()));
        Some(
            tooltip(
                placed,
                container(chrome::label(tip, 11, colors.editor).font(fonts::UI))
                    .padding([4, 7])
                    .style(move |_| chrome::surface(definition, colors.text, 5.0, false)),
                tooltip::Position::Bottom,
            )
            .gap(4)
            .into(),
        )
    }

    /// Whether `note`'s front matter hides its tasks: `None` for a note
    /// the task index does not hold (a board), or with tasks off.
    pub(super) fn note_tasks_hidden(&self, note: NoteId) -> Option<bool> {
        if !self.preferences.tasks {
            return None;
        }
        let tasks = self.session.as_ref()?.tasks()?;
        tasks.flags(note).map(|flags| flags.hidden)
    }

    /// How `group` is left out of task views: `None` with tasks off or
    /// for no group of the workspace.
    pub(super) fn group_tasks_hidden(&self, group: GroupId) -> Option<GroupHidden> {
        if !self.preferences.tasks {
            return None;
        }
        let session = self.session.as_ref()?;
        let path = &session.model().group(group)?.path;
        let excluded = session.tasks()?.excluded();
        Some(GroupHidden {
            own: excluded.contains(path),
            inherited: excluded
                .iter()
                .any(|folder| folder != path && path.as_path().starts_with(folder.as_path())),
        })
    }

    /// The title of the hide on `item` as things stand, or `None` when
    /// `item` offers none (a board, a section, a group only its section
    /// hides, or tasks off).
    pub(super) fn hide_title(&self, item: ItemRef) -> Option<&'static str> {
        match item {
            ItemRef::Group(group) => group_hide_title(self.group_tasks_hidden(group)?),
            ItemRef::Note(note) => Some(note_hide_title(self.note_tasks_hidden(note)?)),
            ItemRef::Section(_) => None,
        }
    }

    /// The muted mark a row carries, if any: "tasks off" on a note its
    /// front matter hides, "hidden" on a group left out of task views (or
    /// under one that is).
    pub(super) fn row_mark(&self, item: ItemRef) -> Option<&'static str> {
        if !self.preferences.tasks {
            return None;
        }
        let session = self.session.as_ref()?;
        let tasks = session.tasks()?;
        match item {
            ItemRef::Note(note) => tasks.flags(note)?.hidden.then_some("tasks off"),
            ItemRef::Group(group) => self.group_mark(&session.model().group(group)?.path),
            ItemRef::Section(_) => None,
        }
    }

    /// A group row's mark, by the group's path: "hidden" when it or its
    /// section is left out of task views.
    pub(super) fn group_mark(&self, path: &RelPath) -> Option<&'static str> {
        if !self.preferences.tasks {
            return None;
        }
        let tasks = self.session.as_ref()?.tasks()?;
        tasks.is_excluded(path).then_some("hidden")
    }

    /// A row's mark, drawn small and muted in a thin frame.
    pub(super) fn mark_view<'a>(&self, mark: &'static str) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        container(chrome::label(mark, 10, colors.faint).font(fonts::UI))
            .padding([0, 4])
            .style(move |_| container::Style {
                border: iced::Border {
                    color: theme::color(colors.divider),
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    /// Whether a row's actions offer `id` for `item`: the task rows only
    /// with tasks on, and Hide tasks in this note only on a Markdown note.
    pub(super) fn item_command_offered(&self, item: ItemRef, id: CommandId) -> bool {
        if !super::commands::item_commands(item).contains(&id) {
            return false;
        }
        if id.tasks() && !self.preferences.tasks {
            return false;
        }
        match id {
            CommandId::HideNoteTasks | CommandId::HideGroupTasks => self.hide_title(item).is_some(),
            _ => true,
        }
    }

    /// A row action's title: Show tasks names the row's kind (the row's
    /// name is beside it, and a title that held it would change with a
    /// rename), and a hide says what it does now.
    pub(super) fn item_action_title(&self, item: ItemRef, id: CommandId) -> String {
        match id {
            CommandId::ShowTasks => match item {
                ItemRef::Section(_) => "Show tasks in this section",
                ItemRef::Group(_) => "Show tasks in this group",
                ItemRef::Note(_) => "Show tasks in this note",
            }
            .into(),
            CommandId::HideGroupTasks | CommandId::HideNoteTasks => {
                self.hide_title(item).unwrap_or_default().into()
            }
            _ => super::commands::definitions()
                .find(|definition| definition.id == id)
                .map_or("", |definition| definition.label)
                .into(),
        }
    }

    /// Hide tasks from `group`, or show them again: kept in
    /// `.wren/tasks.toml`, with a message whose Undo sets it back.
    pub(super) fn toggle_group_tasks(&mut self, group: GroupId) -> Gateway {
        let Some(hidden) = self.group_tasks_hidden(group) else {
            return self.blocked("The group no longer exists");
        };
        if !hidden.own && hidden.inherited {
            return self.blocked("Its section hides this group's tasks");
        }
        let place = self.model().and_then(|model| {
            let group = model.group(group)?;
            let section = model.section(group.section)?;
            Some(format!("{}/{}", section.name, group.name))
        });
        // Saved or not at all: a file that cannot be written leaves the
        // group as it was, and the refusal is the outcome's.
        let outcome = self.process_core(Op::SetTasksExcluded {
            target: ItemRef::Group(group),
            excluded: !hidden.own,
        });
        if outcome.processed.disposition.committed() {
            let place = place.unwrap_or_default();
            let words = match (hidden.own, hidden.inherited) {
                (false, _) => {
                    format!("{place} hidden from task summaries · saved in .wren/tasks.toml")
                }
                (true, false) => {
                    format!("{place} back in task summaries · saved in .wren/tasks.toml")
                }
                (true, true) => format!(
                    "{place} no longer hidden on its own · its section still hides its tasks \
                     · saved in .wren/tasks.toml"
                ),
            };
            self.notify(
                Notice::new(vec![Part::Text(words)]).with_undo(Undo::Excluded {
                    group,
                    excluded: hidden.own,
                }),
            );
        }
        self.close_palette_after(outcome)
    }

    /// Hide the tasks in `note`, or show them again: `tasks: off` written
    /// into its front matter or taken out (`App::set_note_tasks_hidden`),
    /// with a message whose Undo puts the front matter back.
    pub(super) fn toggle_note_tasks(&mut self, note: NoteId) -> Gateway {
        let Some(hidden) = self.note_tasks_hidden(note) else {
            return self.blocked("Only a Markdown note's tasks can be hidden");
        };
        let title = self
            .model()
            .and_then(|model| model.note(note))
            .map(|note| note.title.clone())
            .unwrap_or_default();
        let written = self.set_note_tasks_hidden(note, !hidden);
        if let Ok(write) = &written.result {
            if write.changed {
                let parts = if hidden {
                    vec![
                        Part::Text(format!("Tasks in {title} are back · removed ")),
                        Part::Code("tasks: off".into()),
                        Part::Text(" from its front matter".into()),
                    ]
                } else {
                    vec![
                        Part::Text(format!("Tasks in {title} hidden · wrote ")),
                        Part::Code("tasks: off".into()),
                        Part::Text(" to its front matter".into()),
                    ]
                };
                let notice = Notice::new(parts);
                self.notify(match self.task_writes.mark() {
                    Some(mark) => notice.with_undo(Undo::TaskWrite(mark)),
                    None => notice,
                });
            }
        }
        self.close_palette_after(written.gateway)
    }
}

/// The harness's `tasks.entry` (tasks-build item 10.2). With no
/// parameters it reports `{"tasks": true, "counts", "headers", "marks",
/// "scope", "notice"}`: whether the headers count, each column's count
/// (`scope`, `path`, `open_and_doing`, `overdue`, `pressed`, `tip`, or
/// `null`), the paths of the groups and notes that carry a mark, the scope
/// `tasks.show` would open now, and the message showing (`serial`, `text`,
/// `code`, `undo`) or `null`. With `{"click": column}` it clicks that
/// column's count, with `{"undo": true}` the message's Undo and with
/// `{"dismiss": true}` the message itself, as a pointer does, and answers
/// with the report and the action's `disposition` and `warning`. With
/// tasks off it says only `{"tasks": false}` and refuses the actions.
#[cfg(feature = "smoke-test")]
impl App {
    pub(super) fn task_entry_request(
        &mut self,
        request: &std::sync::Arc<crate::test_ipc::Request>,
    ) -> Result<iced::Task<Message>, String> {
        use serde_json::{json, Value};
        let params = &request.params;
        let acts = ["click", "undo", "dismiss"];
        let Some(act) = acts.into_iter().find(|act| params.get(*act).is_some()) else {
            request.respond(Ok(self.task_entry_snapshot()));
            return Ok(iced::Task::none());
        };
        if !self.preferences.tasks {
            return Err("Tasks are off".into());
        }
        let message = match act {
            "click" => {
                let column = params
                    .get("click")
                    .and_then(Value::as_str)
                    .and_then(|name| {
                        [Column::Sections, Column::Groups, Column::Notes]
                            .into_iter()
                            .find(|column| column.key() == name)
                    })
                    .ok_or("click must be sections, groups or notes")?;
                let count = self
                    .header_count(column)
                    .ok_or("That column's header shows no count")?;
                Message::Command(Command::ShowTasks(count.scope))
            }
            _ => {
                let serial = self.notice().ok_or("No message is showing")?.serial;
                Message::Notice(if act == "undo" {
                    super::notice::Event::Undo(serial)
                } else {
                    super::notice::Event::Dismiss(serial)
                })
            }
        };
        let before = self.action_serial;
        let task = self.update(message);
        let mut answer = self.task_entry_snapshot();
        let processed = self
            .last_processed
            .as_ref()
            .filter(|_| self.action_serial != before);
        answer["disposition"] = json!(processed.map(|processed| processed.disposition.key()));
        answer["warning"] = json!(processed.and_then(|processed| processed.warning.clone()));
        request.respond(Ok(answer));
        Ok(task)
    }

    fn task_entry_snapshot(&self) -> serde_json::Value {
        use serde_json::json;
        if !self.preferences.tasks {
            return json!({"tasks": false});
        }
        let scope = |scope: &Scope| {
            json!({
                "scope": scope.kind(),
                "path": scope
                    .prefix()
                    .map(|path| path.as_path().to_string_lossy().into_owned()),
            })
        };
        let header = |column: Column| {
            self.header_count(column).map(|count| {
                let mut value = scope(&count.scope);
                value["open_and_doing"] = json!(count.counts.open_and_doing);
                value["overdue"] = json!(count.counts.overdue);
                value["pressed"] = json!(count.pressed);
                value["label"] = json!(self.task_view_label(&count.scope));
                value
            })
        };
        let path =
            |path: &wren_core::fs_store::RelPath| path.as_path().to_string_lossy().into_owned();
        let (groups, notes) = self.model().map_or_else(Default::default, |model| {
            let groups: Vec<String> = (model.groups.iter())
                .filter(|group| self.group_mark(&group.path).is_some())
                .map(|group| path(&group.path))
                .collect();
            let notes: Vec<String> = (model.notes.iter())
                .filter(|note| self.row_mark(ItemRef::Note(note.id)).is_some())
                .map(|note| path(&note.path))
                .collect();
            (groups, notes)
        });
        let notice = self.notice().map(|shown| {
            let code: Vec<&str> = (shown.notice.parts.iter())
                .filter_map(|part| match part {
                    Part::Code(code) => Some(code.as_str()),
                    Part::Text(_) => None,
                })
                .collect();
            json!({
                "serial": shown.serial,
                "text": shown.notice.text(),
                "code": code,
                "undo": shown.notice.undo.is_some(),
            })
        });
        json!({
            "tasks": true,
            "counts": self.task_counts_shown(),
            "headers": {
                "sections": header(Column::Sections),
                "groups": header(Column::Groups),
                "notes": header(Column::Notes),
            },
            "marks": {"groups": groups, "notes": notes},
            "scope": self.show_tasks_scope().as_ref().map(scope),
            "notice": notice,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use wren_core::fs_store::RelPath;
    use wren_core::keybind::{LinuxModifier, Platform};
    use wren_core::model::SectionId;
    use wren_core::ops::Session;
    use wren_markdown::date::Date;

    use super::super::board::tests::FLOW;
    use super::super::board::Surface;
    use super::super::commands::{Disposition, ItemTarget};
    use super::super::palette::{Key, Mode};
    use super::super::Options;
    use super::*;
    use crate::keys::Input;

    const A: &str = "# A\n\n- [ ] Mail the deck 📅 2026-11-16\n- [/] Book the room\n- [x] Done\n";
    const B: &str = "# B\n\n- [ ] Order lunch 📅 2026-11-20\n";
    const GROUP: &str = "Work/Tasks";
    const ON: &str = "tasks = on\nboards = on\n";

    /// Sections Work and Home; Work holds the groups Tasks (the notes A
    /// and B and the board Flow) and Spare (Extra), Home the group Chores
    /// (Third). A is open, and today is pinned to Thursday 19 November
    /// 2026: A has one overdue and one doing task, B one due tomorrow.
    fn fixture_with(settings: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        for (path, text) in [
            ("Work/Tasks/A.md", A),
            ("Work/Tasks/B.md", B),
            ("Work/Tasks/Flow.wren-board", FLOW),
            ("Work/Spare/Extra.md", "- [ ] spare\n"),
            ("Home/Chores/Third.md", "- [ ] third\n"),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let config = directory.path().join("config.conf");
        std::fs::write(&config, settings).unwrap();
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
        open(&mut app, "A");
        (directory, app)
    }

    fn fixture() -> (tempfile::TempDir, App) {
        fixture_with(ON)
    }

    fn file(directory: &tempfile::TempDir, path: &str) -> PathBuf {
        directory.path().join("workspace").join(path)
    }

    fn on_disk(directory: &tempfile::TempDir, path: &str) -> String {
        std::fs::read_to_string(file(directory, path)).unwrap()
    }

    fn config(directory: &tempfile::TempDir) -> String {
        std::fs::read_to_string(directory.path().join("config.conf")).unwrap()
    }

    fn note(app: &App, title: &str) -> NoteId {
        (app.model().unwrap().notes.iter())
            .find(|note| note.title == title)
            .unwrap_or_else(|| panic!("no note titled {title}"))
            .id
    }

    fn group(app: &App, path: &str) -> GroupId {
        (app.model().unwrap().groups.iter())
            .find(|group| group.path.as_path().to_str() == Some(path))
            .unwrap()
            .id
    }

    fn section(app: &App, name: &str) -> SectionId {
        (app.model().unwrap().sections.iter())
            .find(|section| section.name == name)
            .unwrap()
            .id
    }

    fn open(app: &mut App, title: &str) {
        let target = app.note_target(note(app, title));
        let outcome = app.dispatch(Command::OpenNote(target));
        assert!(outcome.processed.disposition.committed());
    }

    fn scope(kind: &str, path: &str) -> Scope {
        let path = RelPath::new(path).unwrap();
        match kind {
            "section" => Scope::Section(path),
            "group" => Scope::Group(path),
            _ => Scope::Note(path),
        }
    }

    fn target(app: &App, item: ItemRef) -> ItemTarget {
        app.item_target(item).unwrap()
    }

    /// A row's action, as its palette runs it.
    fn act(app: &mut App, item: ItemRef, id: CommandId) -> Gateway {
        let target = target(app, item);
        app.dispatch(Command::ItemAction { target, id })
    }

    /// The titles a row's actions palette lists.
    fn row_actions(app: &mut App, item: ItemRef) -> Vec<String> {
        let _ = app.open_palette(Mode::ItemActions(item));
        let titles = (app.palette.as_ref().unwrap().results.iter())
            .filter(|row| matches!(row.key, Key::ItemAction { .. }))
            .map(|row| row.title.clone())
            .collect();
        let _ = app.close_palette();
        titles
    }

    fn counts(app: &App, column: Column) -> Option<(Scope, usize, usize, bool)> {
        app.header_count(column).map(|count| {
            (
                count.scope,
                count.counts.open_and_doing,
                count.counts.overdue,
                count.pressed,
            )
        })
    }

    fn undo(app: &mut App) -> Gateway {
        let serial = app.notice().expect("a message shows").serial;
        app.undo_notice(serial)
    }

    /// `tasks.show` opens the selected section from Sections, the selected
    /// group from Groups, the group listed from Notes, the open note's
    /// group from the editor and from a board, and the view itself from
    /// the view (prototype `scopeFromActive`).
    #[test]
    fn show_tasks_opens_the_scope_of_where_the_keyboard_is() {
        let (_directory, mut app) = fixture();
        let work = scope("section", "Work");
        let tasks = scope("group", GROUP);
        for (column, expected) in [
            (Column::Sections, &work),
            (Column::Groups, &tasks),
            (Column::Notes, &tasks),
            (Column::Editor, &tasks),
        ] {
            app.column = column;
            assert_eq!(
                app.show_tasks_scope().as_ref(),
                Some(expected),
                "{column:?}"
            );
        }
        // Another group selected in the Groups column is its own scope.
        app.selected_group = Some(group(&app, "Work/Spare"));
        app.column = Column::Groups;
        assert_eq!(app.show_tasks_scope(), Some(scope("group", "Work/Spare")));
        // The editor follows the open note, not the browsing.
        app.column = Column::Editor;
        assert_eq!(app.show_tasks_scope(), Some(tasks.clone()));
        app.selected_group = Some(group(&app, GROUP));

        // From the editor the command shows the note's group, and from the
        // view the view itself, which is no new transition.
        let outcome = app.dispatch(Command::Run(CommandId::ShowTasks));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.task_view(), Some(&tasks));
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.show_tasks_scope(), Some(tasks.clone()));
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        assert_eq!(app.show_tasks_scope(), Some(Scope::Workspace));
        let can_back = app.can_step(true);
        let outcome = app.dispatch(Command::Run(CommandId::ShowTasks));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert_eq!(app.can_step(true), can_back);
        // From a browser column under the view, that column's scope.
        app.column = Column::Sections;
        assert_eq!(app.show_tasks_scope(), Some(work));

        // From a board: the board's group.
        open(&mut app, "Flow");
        assert_eq!(app.surface(), Surface::Board);
        app.column = Column::Editor;
        assert_eq!(app.show_tasks_scope(), Some(tasks.clone()));
        let _ = app.dispatch(Command::Run(CommandId::ShowTasks));
        assert_eq!(app.task_view(), Some(&tasks));
    }

    /// Mod+Shift+T runs Show tasks from the editor and from a column, in
    /// both of Linux's modes, through the app's key handling.
    #[test]
    fn the_show_tasks_key_runs_it_in_either_mode() {
        for mode in [LinuxModifier::Ctrl, LinuxModifier::Alt] {
            let (_directory, mut app) = fixture();
            app.preferences.settings.linux_modifier = mode;
            app.preferences.rebuild_keymap();
            app.apply_preferences();
            let scheme = app.preferences.keymap.scheme();
            let key = scheme.parse("mod+shift+t").unwrap();
            assert_eq!(app.preferences.keymap.accel("tasks.show"), Some(&key));
            if scheme.platform == Platform::Linux {
                let expected = match mode {
                    LinuxModifier::Ctrl => "ctrl+shift+t",
                    LinuxModifier::Alt => "alt+shift+t",
                };
                assert_eq!(key, scheme.parse(expected).unwrap(), "{mode:?}");
            }
            let _ = app.update(Message::Key(Input::Accel(key.clone())));
            assert_eq!(app.task_view(), Some(&scope("group", GROUP)), "{mode:?}");
            let _ = app.dispatch(Command::Run(CommandId::Back));
            assert_eq!(app.task_view(), None);
            app.column = Column::Sections;
            let _ = app.update(Message::Key(Input::Accel(key)));
            assert_eq!(app.task_view(), Some(&scope("section", "Work")), "{mode:?}");
        }
    }

    /// Each header counts the open and doing tasks of what its column
    /// belongs to, overdue ones against the pinned day, which follows a
    /// change of day; the count shows pressed while its view shows.
    #[test]
    fn the_headers_count_their_scope_and_follow_the_day() {
        let (_directory, mut app) = fixture();
        let tasks = scope("group", GROUP);
        assert_eq!(
            counts(&app, Column::Sections),
            Some((Scope::Workspace, 5, 1, false))
        );
        assert_eq!(
            counts(&app, Column::Groups),
            Some((scope("section", "Work"), 4, 1, false))
        );
        assert_eq!(
            counts(&app, Column::Notes),
            Some((tasks.clone(), 3, 1, false))
        );
        assert_eq!(counts(&app, Column::Editor), None);
        // B's lunch is due on the 20th: overdue from the 21st.
        app.pin_today(Date::new(2026, 11, 21).unwrap());
        assert_eq!(
            counts(&app, Column::Notes),
            Some((tasks.clone(), 3, 2, false))
        );
        app.pin_today(Date::new(2026, 11, 15).unwrap());
        assert_eq!(
            counts(&app, Column::Notes),
            Some((tasks.clone(), 3, 0, false))
        );
        app.pin_today(Date::new(2026, 11, 19).unwrap());
        // Another section's groups count that section.
        app.selected_section = Some(section(&app, "Home"));
        app.selected_group = Some(group(&app, "Home/Chores"));
        assert_eq!(
            counts(&app, Column::Groups),
            Some((scope("section", "Home"), 1, 0, false))
        );
        assert_eq!(
            counts(&app, Column::Notes),
            Some((scope("group", "Home/Chores"), 1, 0, false))
        );
        app.selected_section = Some(section(&app, "Work"));
        app.selected_group = Some(group(&app, GROUP));
        // A click on a count is the view of its scope, and the count is
        // then pressed.
        let _ = app.update(Message::Command(Command::ShowTasks(Scope::Workspace)));
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert_eq!(
            counts(&app, Column::Sections),
            Some((Scope::Workspace, 5, 1, true))
        );
        assert!(!counts(&app, Column::Notes).unwrap().3);
        let _ = app.update(Message::Command(Command::ShowTasks(tasks.clone())));
        assert!(counts(&app, Column::Notes).unwrap().3);
        assert!(!counts(&app, Column::Sections).unwrap().3);
        // Typing a task in the open note counts at once.
        while app.task_view().is_some() {
            let _ = app.dispatch(Command::Run(CommandId::Back));
        }
        use iced::widget::text_editor::{Action, Edit, Motion};
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        ))));
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
            Edit::Paste(std::sync::Arc::new("- [ ] More".into())),
        ))));
        assert_eq!(app.editor.text(), format!("{A}- [ ] More"));
        assert_eq!(counts(&app, Column::Notes), Some((tasks, 4, 1, false)));
    }

    /// `view.toggle_task_counts` hides and shows the counts, saves
    /// `task-counts` to config.conf, and leaves a message whose Undo flips
    /// it back.
    #[test]
    fn toggling_the_counts_writes_the_setting_with_undo() {
        let (directory, mut app) = fixture();
        assert_eq!(app.command_state(CommandId::ToggleTaskCounts), Some("on"));
        let outcome = app.dispatch(Command::Run(CommandId::ToggleTaskCounts));
        assert!(
            outcome.processed.disposition.committed(),
            "{outcome:?}",
            outcome = outcome.processed
        );
        assert!(config(&directory).ends_with("task-counts = off\n"));
        assert!(!app.task_counts_shown());
        assert!([Column::Sections, Column::Groups, Column::Notes]
            .into_iter()
            .all(|column| app.header_count(column).is_none()));
        assert_eq!(app.command_state(CommandId::ToggleTaskCounts), Some("off"));
        let shown = app.notice().unwrap();
        assert_eq!(
            shown.notice.text(),
            "Task counts hidden · saved task-counts = off to config.conf"
        );
        assert_eq!(shown.notice.undo, Some(Undo::Counts(true)));
        let outcome = undo(&mut app);
        assert!(outcome.processed.disposition.committed());
        assert!(config(&directory).ends_with("task-counts = on\n"));
        assert!(app.task_counts_shown() && app.header_count(Column::Notes).is_some());
        assert!(app.notice().is_none(), "an undo leaves no message");
        // A reload reads the file's value.
        std::fs::write(
            directory.path().join("config.conf"),
            format!("{ON}task-counts = off\n"),
        )
        .unwrap();
        let _ = app.dispatch(Command::Run(CommandId::ReloadSettings));
        assert!(!app.task_counts_shown());
    }

    /// Hide tasks from this group: kept in `.wren/tasks.toml`, marked on
    /// the row, left out of the counts, offered as "Show … again", and set
    /// back by the message's Undo.
    #[test]
    fn hiding_a_group_marks_it_and_undo_shows_it_again() {
        let (directory, mut app) = fixture();
        let tasks = group(&app, GROUP);
        let item = ItemRef::Group(tasks);
        assert_eq!(app.row_mark(item), None);
        assert_eq!(
            app.item_action_title(item, CommandId::HideGroupTasks),
            "Hide tasks from this group"
        );
        let outcome = act(&mut app, item, CommandId::HideGroupTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        let stored = on_disk(&directory, ".wren/tasks.toml");
        assert!(stored.contains("Work/Tasks"), "{stored}");
        assert_eq!(app.row_mark(item), Some("hidden"));
        assert_eq!(
            app.group_tasks_hidden(tasks),
            Some(GroupHidden {
                own: true,
                inherited: false
            })
        );
        assert_eq!(counts(&app, Column::Sections).unwrap().1, 2);
        assert_eq!(counts(&app, Column::Notes).unwrap().1, 0);
        assert_eq!(
            app.item_action_title(item, CommandId::HideGroupTasks),
            "Show this group\u{2019}s tasks again"
        );
        let shown = app.notice().unwrap();
        assert_eq!(
            shown.notice.text(),
            "Work/Tasks hidden from task summaries · saved in .wren/tasks.toml"
        );
        // The palette's own command acts on the selected group, and its
        // title says what it does now.
        let _ = app.open_palette(Mode::Commands);
        let title = (app.palette.as_ref().unwrap().results.iter())
            .find(|row| row.key == Key::Command(CommandId::HideGroupTasks))
            .map(|row| row.title.clone());
        assert_eq!(
            title.as_deref(),
            Some("Show this group\u{2019}s tasks again")
        );
        let _ = app.close_palette();

        let outcome = undo(&mut app);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.row_mark(item), None);
        assert_eq!(counts(&app, Column::Notes).unwrap().1, 3);
        assert!(!on_disk(&directory, ".wren/tasks.toml").contains("Work/Tasks"));

        // Hidden again from the palette, then shown again from it, with
        // its own message.
        let _ = app.dispatch(Command::Run(CommandId::HideGroupTasks));
        assert_eq!(app.row_mark(item), Some("hidden"));
        let _ = app.dispatch(Command::Run(CommandId::HideGroupTasks));
        assert_eq!(app.row_mark(item), None);
        assert_eq!(
            app.notice().unwrap().notice.text(),
            "Work/Tasks back in task summaries · saved in .wren/tasks.toml"
        );
    }

    /// Hide tasks in this note, on a note that is not open: `tasks: off`
    /// written into its front matter by the Session, the row marked, its
    /// tasks out of the counts, and the message's Undo puts the file back.
    #[test]
    fn hiding_a_closed_note_writes_its_front_matter_and_undo_restores_it() {
        let (directory, mut app) = fixture();
        let b = note(&app, "B");
        let item = ItemRef::Note(b);
        let outcome = act(&mut app, item, CommandId::HideNoteTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(
            on_disk(&directory, "Work/Tasks/B.md"),
            format!("---\ntasks: off\n---\n{B}")
        );
        assert_eq!(app.row_mark(item), Some("tasks off"));
        assert_eq!(counts(&app, Column::Notes).unwrap().1, 2);
        assert_eq!(
            app.item_action_title(item, CommandId::HideNoteTasks),
            "Show tasks in this note again"
        );
        let shown = app.notice().unwrap();
        assert_eq!(
            shown.notice.text(),
            "Tasks in B hidden · wrote tasks: off to its front matter"
        );
        assert_eq!(shown.notice.parts[1], Part::Code("tasks: off".into()));
        let outcome = undo(&mut app);
        assert!(outcome.processed.disposition.committed());
        assert_eq!(on_disk(&directory, "Work/Tasks/B.md"), B);
        assert_eq!(app.row_mark(item), None);
        assert_eq!(counts(&app, Column::Notes).unwrap().1, 3);

        // Shown again by the action itself, with its own message.
        let _ = act(&mut app, item, CommandId::HideNoteTasks);
        let _ = act(&mut app, item, CommandId::HideNoteTasks);
        assert_eq!(on_disk(&directory, "Work/Tasks/B.md"), B);
        assert_eq!(
            app.notice().unwrap().notice.text(),
            "Tasks in B are back · removed tasks: off from its front matter"
        );
    }

    /// On the open note the hide goes through its editor, as one undo step
    /// of the draft, and the message's Undo puts the text back.
    #[test]
    fn hiding_the_open_note_goes_through_its_editor() {
        let (_directory, mut app) = fixture();
        let a = note(&app, "A");
        let item = ItemRef::Note(a);
        let outcome = act(&mut app, item, CommandId::HideNoteTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.editor.text(), format!("---\ntasks: off\n---\n{A}"));
        let draft = app.session.as_ref().unwrap().document().unwrap();
        assert_eq!(draft.text, app.editor.text());
        assert_eq!(app.row_mark(item), Some("tasks off"));
        assert_eq!(
            counts(&app, Column::Notes).unwrap(),
            (scope("group", GROUP), 1, 0, false)
        );
        let _ = undo(&mut app);
        assert_eq!(app.editor.text(), A);
        assert_eq!(app.row_mark(item), None);
    }

    /// A message's Undo takes back the write it names only: not once
    /// another message replaced it, nor once that write was undone
    /// another way.
    #[test]
    fn a_message_undoes_only_its_own_write() {
        let (directory, mut app) = fixture();
        let b = ItemRef::Note(note(&app, "B"));
        let extra = ItemRef::Note(note(&app, "Extra"));
        let _ = act(&mut app, b, CommandId::HideNoteTasks);
        let first = app.notice().unwrap().serial;
        let _ = act(&mut app, extra, CommandId::HideNoteTasks);
        let outcome = app.undo_notice(first);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert!(app.notice().is_some(), "the newer message stays");
        // Undone another way (the view's ⌘Z), the message's Undo refuses.
        let _ = app.undo_task_write();
        assert_eq!(on_disk(&directory, "Work/Spare/Extra.md"), "- [ ] spare\n");
        let outcome = undo(&mut app);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert!(on_disk(&directory, "Work/Tasks/B.md").starts_with("---\ntasks: off\n"));
        assert!(app.notice().is_none());
        // A dismissed message is gone; one past its time goes on a tick.
        let _ = act(&mut app, b, CommandId::HideNoteTasks);
        let serial = app.notice().unwrap().serial;
        let _ = app.notice_event(super::super::notice::Event::Dismiss(serial));
        assert!(app.notice().is_none());
        let _ = act(&mut app, b, CommandId::HideNoteTasks);
        app.expire_notice(std::time::Instant::now() + std::time::Duration::from_secs(5));
        assert!(app.notice().is_some());
        app.expire_notice(std::time::Instant::now() + std::time::Duration::from_secs(7));
        assert!(app.notice().is_none());
    }

    /// A row's actions with tasks on: Show tasks on every kind, Hide tasks
    /// from this group on a group, and Hide tasks in this note on a
    /// Markdown note but not a board; with tasks off, none of them.
    #[test]
    fn the_row_actions_offer_the_task_rows_by_kind_only_with_tasks_on() {
        let (_directory, mut app) = fixture();
        let work = ItemRef::Section(section(&app, "Work"));
        let tasks = ItemRef::Group(group(&app, GROUP));
        let a = ItemRef::Note(note(&app, "A"));
        let flow = ItemRef::Note(note(&app, "Flow"));
        let ends = |titles: Vec<String>, tail: &[&str]| {
            assert!(
                titles.ends_with(&tail.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>()),
                "{titles:?}"
            );
            titles.len()
        };
        let section_rows = ends(row_actions(&mut app, work), &["Show tasks in this section"]);
        let group_rows = ends(
            row_actions(&mut app, tasks),
            &["Show tasks in this group", "Hide tasks from this group"],
        );
        let note_rows = ends(
            row_actions(&mut app, a),
            &["Show tasks in this note", "Hide tasks in this note"],
        );
        ends(row_actions(&mut app, flow), &["Show tasks in this note"]);
        assert!(!app.item_command_offered(flow, CommandId::HideNoteTasks));
        let outcome = act(&mut app, flow, CommandId::HideNoteTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        // Show tasks from a row opens that row's item.
        let outcome = act(&mut app, work, CommandId::ShowTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.task_view(), Some(&scope("section", "Work")));
        let _ = act(&mut app, a, CommandId::ShowTasks);
        assert_eq!(app.task_view(), Some(&scope("note", "Work/Tasks/A.md")));

        let (_directory, mut app) = fixture_with("tasks = off\nboards = on\n");
        let work = ItemRef::Section(section(&app, "Work"));
        let tasks = ItemRef::Group(group(&app, GROUP));
        let a = ItemRef::Note(note(&app, "A"));
        assert_eq!(row_actions(&mut app, work).len(), section_rows - 1);
        assert_eq!(row_actions(&mut app, tasks).len(), group_rows - 2);
        assert_eq!(row_actions(&mut app, a).len(), note_rows - 2);
        for item in [work, tasks, a] {
            assert!(row_actions(&mut app, item)
                .iter()
                .all(|title| !title.contains("tasks")));
            for id in [
                CommandId::ShowTasks,
                CommandId::HideGroupTasks,
                CommandId::HideNoteTasks,
            ] {
                assert!(!app.item_command_offered(item, id));
                let outcome = act(&mut app, item, id);
                assert_eq!(outcome.processed.disposition, Disposition::Blocked);
            }
        }
    }

    /// With tasks off: no count, no mark, no message, no key, and the
    /// four commands are disabled; the setting `task-counts` changes
    /// nothing.
    #[test]
    fn with_tasks_off_there_is_no_way_in() {
        let (directory, mut app) = fixture_with("tasks = off\nboards = on\ntask-counts = on\n");
        for column in [
            Column::Sections,
            Column::Groups,
            Column::Notes,
            Column::Editor,
        ] {
            assert!(app.header_count(column).is_none());
        }
        assert!(!app.task_counts_shown());
        let a = note(&app, "A");
        assert_eq!(app.row_mark(ItemRef::Note(a)), None);
        assert_eq!(app.note_tasks_hidden(a), None);
        for id in [
            CommandId::ShowTasks,
            CommandId::HideGroupTasks,
            CommandId::HideNoteTasks,
            CommandId::ToggleTaskCounts,
        ] {
            assert!(id.tasks() && id.task_entry());
            assert!(!app.command_enabled(id));
            let outcome = app.dispatch(Command::Run(id));
            assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        }
        let scheme = app.preferences.keymap.scheme();
        let key = scheme.parse("mod+shift+t").unwrap();
        assert_eq!(app.preferences.keymap.action(&key), None);
        assert_eq!(app.preferences.keymap.accel("tasks.show"), None);
        let _ = app.update(Message::Key(Input::Accel(key)));
        assert_eq!(app.task_view(), None);
        assert!(app.notice().is_none());
        assert!(!file(&directory, ".wren/tasks.toml").exists());
        assert_eq!(
            config(&directory),
            "tasks = off\nboards = on\ntask-counts = on\n"
        );
    }

    /// The command table's rules for the entry points: App scope, in the
    /// palette's groups the design names, only with tasks on; Show tasks
    /// alone has a default key.
    #[test]
    fn the_entry_commands_keep_the_command_tables_rules() {
        let rows: Vec<_> = super::super::commands::definitions()
            .filter(|row| row.id.task_entry())
            .map(|row| (row.key, row.group, row.scope))
            .collect();
        use super::super::commands::Scope as CommandScope;
        assert_eq!(
            rows,
            [
                ("tasks.show", "Tasks", CommandScope::App),
                ("note.hide_tasks", "This note", CommandScope::App),
                ("group.hide_tasks", "Workspace", CommandScope::App),
                ("view.toggle_task_counts", "View", CommandScope::App),
            ]
        );
        for (key, _, _) in &rows {
            let id = CommandId::parse(key).unwrap();
            assert!(id.tasks() && id.editor_route().is_none(), "{key}");
            assert!(
                id.task_change().is_none() && id.task_menu().is_none(),
                "{key}"
            );
        }
        for scheme in super::super::preferences::SCHEMES {
            let (keymap, warnings) = super::super::preferences::build_keymap(scheme, &[], true);
            assert!(warnings.is_empty());
            assert_eq!(
                keymap.accel("tasks.show"),
                scheme.parse("mod+shift+t").as_ref()
            );
            for key in [
                "note.hide_tasks",
                "group.hide_tasks",
                "view.toggle_task_counts",
            ] {
                assert_eq!(keymap.accel(key), None, "{scheme:?} {key}");
            }
            let (off, _) = super::super::preferences::build_keymap(scheme, &[], false);
            assert_eq!(off.accel("tasks.show"), None);
        }
    }

    fn hide(app: &mut App, item: ItemRef, excluded: bool) {
        let outcome = app.process_core(Op::SetTasksExcluded {
            target: item,
            excluded,
        });
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
    }

    /// Review of 10.2, finding 1: the message's widgets are keyed by its
    /// serial. A press on one message's Undo and its release on the
    /// message that replaced it undoes nothing, and the release is taken
    /// there, never reaching what the message covers; on the same message
    /// the press and release are its Undo.
    #[test]
    fn a_press_on_a_replaced_message_never_undoes_the_new_one() {
        use iced::advanced::layout::{self, Layout};
        use iced::advanced::widget::Tree;
        use iced::advanced::Shell;
        use iced::mouse::{self as pointer, Button};
        use iced::{Event as Input, Length, Point, Rectangle, Size};

        type Frame<'a> = Element<'a, Message, iced::Theme, ()>;
        let make = |serial: u64| -> Frame<'static> {
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
            )
        };
        let limits = layout::Limits::new(Size::ZERO, Size::new(600.0, 100.0));
        let viewport = Rectangle::new(Point::ORIGIN, Size::new(600.0, 100.0));
        // Each event on a frame of `serial`, as the window would send it at
        // `at`: what it published, and whether it was taken.
        let send = |frame: &mut Frame<'static>,
                    tree: &mut Tree,
                    event: Input,
                    at: Point|
         -> (Vec<Message>, bool) {
            frame.as_widget_mut().diff(tree);
            let node = frame.as_widget_mut().layout(tree, &(), &limits);
            let mut messages = Vec::new();
            let mut shell = Shell::new(&mut messages);
            frame.as_widget_mut().update(
                tree,
                &event,
                Layout::new(&node),
                pointer::Cursor::Available(at),
                &(),
                &mut iced::advanced::clipboard::Null,
                &mut shell,
                &viewport,
            );
            let taken = shell.is_event_captured();
            (messages, taken)
        };
        let mut first = make(1);
        let mut tree = Tree::new(&first);
        let node = first.as_widget_mut().layout(&mut tree, &(), &limits);
        // The Undo button: the frame's message, its face, its line, the
        // line's second child.
        let mut undo = Layout::new(&node);
        for step in [0, 0, 0, 1] {
            undo = undo.children().nth(step).unwrap();
        }
        let at = undo.bounds().center();
        let words = Point::new(undo.bounds().x - 100.0, at.y);
        let press = Input::Mouse(pointer::Event::ButtonPressed(Button::Left));
        let release = Input::Mouse(pointer::Event::ButtonReleased(Button::Left));

        // The same message: the press and release are its Undo.
        assert!(send(&mut first, &mut tree, press.clone(), at).1);
        let (messages, taken) = send(&mut first, &mut tree, release.clone(), at);
        assert!(taken);
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Notice(super::super::notice::Event::Undo(1))]
            ),
            "{messages:?}"
        );

        // Pressed on message 1, released on message 2 in its place.
        assert!(send(&mut first, &mut tree, press.clone(), at).1);
        let mut second = make(2);
        let (messages, taken) = send(&mut second, &mut tree, release.clone(), at);
        assert!(messages.is_empty(), "{messages:?}");
        assert!(taken, "the release is the message's, not the pane's");
        // And on the message's words, nor its dismissal.
        assert!(send(&mut first, &mut tree, press.clone(), words).1);
        let (messages, taken) = send(&mut second, &mut tree, release, words);
        assert!(messages.is_empty() && taken, "{messages:?}");
    }

    /// Review of 10.2, finding 3: the counts' message says they were saved
    /// only when config.conf was written; otherwise they hold for this
    /// session, with Undo.
    #[test]
    fn counts_that_cannot_be_saved_are_said_to_hold_for_this_session() {
        let (directory, mut app) = fixture();
        let blocked = directory.path().join("blocked");
        std::fs::write(&blocked, "a file, not a directory").unwrap();
        app.preferences.path = Some(blocked.join("config.conf"));
        let outcome = app.dispatch(Command::Run(CommandId::ToggleTaskCounts));
        assert_eq!(outcome.processed.disposition, Disposition::CommittedWarning);
        assert!(!app.task_counts_shown());
        let shown = app.notice().unwrap();
        assert_eq!(
            shown.notice.text(),
            "Task counts hidden for this session · task-counts = off was not saved to config.conf"
        );
        assert_eq!(shown.notice.undo, Some(Undo::Counts(true)));
        let _ = undo(&mut app);
        assert!(app.task_counts_shown());
        // With no settings location at all, likewise.
        app.preferences.path = None;
        let _ = app.dispatch(Command::Run(CommandId::ToggleTaskCounts));
        assert!(app
            .notice()
            .unwrap()
            .notice
            .text()
            .ends_with("was not saved to config.conf"));
    }

    /// Review of 10.2, finding 2: a group's exclusion that cannot be saved
    /// changes nothing, says why, and leaves no message.
    #[test]
    fn hiding_a_group_that_cannot_be_saved_changes_nothing() {
        let (directory, mut app) = fixture();
        let tasks = group(&app, GROUP);
        let item = ItemRef::Group(tasks);
        let before = counts(&app, Column::Sections);
        app.session
            .as_mut()
            .unwrap()
            .fail_next(wren_core::fs_store::Fault::Write);
        let outcome = act(&mut app, item, CommandId::HideGroupTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        let warning = outcome.processed.warning.unwrap_or_default();
        assert!(warning.contains("nothing changed"), "{warning}");
        assert_eq!(app.row_mark(item), None);
        assert_eq!(counts(&app, Column::Sections), before);
        assert!(app.notice().is_none());
        assert!(!file(&directory, ".wren/tasks.toml").exists());
        assert_eq!(app.hide_title(item), Some("Hide tasks from this group"));
    }

    /// Review of 10.2, finding 4: a group its section hides too offers to
    /// remove its own entry, says the section still hides it, and once only
    /// the section hides it offers no hide at all.
    #[test]
    fn a_group_its_section_also_hides_says_so() {
        let (_directory, mut app) = fixture();
        let tasks = group(&app, GROUP);
        let item = ItemRef::Group(tasks);
        let work = ItemRef::Section(section(&app, "Work"));
        hide(&mut app, work, true);
        hide(&mut app, item, true);
        assert_eq!(
            app.group_tasks_hidden(tasks),
            Some(GroupHidden {
                own: true,
                inherited: true
            })
        );
        assert_eq!(
            app.hide_title(item),
            Some("Remove this group\u{2019}s own exclusion")
        );
        let outcome = act(&mut app, item, CommandId::HideGroupTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(
            app.notice().unwrap().notice.text(),
            "Work/Tasks no longer hidden on its own · its section still hides its tasks \
             · saved in .wren/tasks.toml"
        );
        assert_eq!(app.row_mark(item), Some("hidden"));
        assert_eq!(counts(&app, Column::Notes).unwrap().1, 0);
        // Only its section hides it now: no hide is offered.
        assert_eq!(app.hide_title(item), None);
        assert!(!app.item_command_offered(item, CommandId::HideGroupTasks));
        assert!(!row_actions(&mut app, item)
            .iter()
            .any(|title| title.contains("exclusion") || title.starts_with("Hide")));
        assert!(!app.command_enabled(CommandId::HideGroupTasks));
        let outcome = act(&mut app, item, CommandId::HideGroupTasks);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        // The message's Undo puts its own entry back.
        let _ = app.dispatch(Command::Run(CommandId::ShowTasks));
        hide(&mut app, work, false);
        assert_eq!(app.hide_title(item), Some("Hide tasks from this group"));
    }
}
