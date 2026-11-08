use std::path::PathBuf;

use iced::Task;
use wren_core::model::{ItemRef, Parent};
use wren_core::ops::Op;

use super::palette::{Mode, SearchTarget};
use super::workspace::DialogKind;
use super::{focus_editor, App, Message};
use crate::panes::Pane;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandId {
    SwitchWorkspace,
    AddWorkspace,
    RemoveWorkspace,
    NewSection,
    NewGroup,
    NewNote,
    NewDatedNote,
    RenameNote,
    RenameGroup,
    RenameSection,
    DeleteNote,
    DeleteGroup,
    DeleteSection,
    RevealNote,
    RevealGroup,
    RevealSection,
    MoveNote,
    MoveGroup,
    ReorderSection,
    ReorderGroup,
    OpenConflictCopy,
    Jump,
    Search,
    Find,
    Back,
    Forward,
    Focus,
    ResetWidths,
    SortModified,
    SortTitle,
    ChooseTheme,
    ChooseFont,
    TextBigger,
    TextSmaller,
    TextReset,
    OpenSettings,
    ReloadSettings,
    CommandPalette,
    Quit,
}

#[derive(Debug, Clone, Copy)]
pub struct Definition {
    pub id: CommandId,
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
}

macro_rules! definitions {
    ($(($id:ident, $key:literal, $label:expr, $group:literal)),* $(,)?) => {
        pub const DEFINITIONS: &[Definition] = &[$(Definition {
            id: CommandId::$id, key: $key, label: $label, group: $group,
        }),*];
    };
}

#[cfg(target_os = "macos")]
const REVEAL_NOTE: &str = "Reveal note in Finder";
#[cfg(target_os = "macos")]
const REVEAL_GROUP: &str = "Reveal group in Finder";
#[cfg(target_os = "macos")]
const REVEAL_SECTION: &str = "Reveal section in Finder";
#[cfg(not(target_os = "macos"))]
const REVEAL_NOTE: &str = "Reveal note in Files";
#[cfg(not(target_os = "macos"))]
const REVEAL_GROUP: &str = "Reveal group in Files";
#[cfg(not(target_os = "macos"))]
const REVEAL_SECTION: &str = "Reveal section in Files";

/// Palette section order; `Hidden` commands are bindable but not listed.
pub const GROUPS: [&str; 5] = ["This note", "Go", "View", "Workspace", "App"];

// These keys are the stable production/test protocol, independent of labels.
definitions![
    (NewNote, "note.new", "New note…", "This note"),
    (
        NewDatedNote,
        "note.new_dated",
        "New dated note",
        "This note"
    ),
    (RenameNote, "note.rename", "Rename note…", "This note"),
    (DeleteNote, "note.delete", "Delete note…", "This note"),
    (RevealNote, "note.reveal", REVEAL_NOTE, "This note"),
    (MoveNote, "note.move", "Move note to…", "This note"),
    (
        OpenConflictCopy,
        "note.open_conflict",
        "Open conflict copy",
        "This note"
    ),
    (NewGroup, "group.new", "New group…", "Workspace"),
    (NewSection, "section.new", "New section…", "Workspace"),
    (RenameGroup, "group.rename", "Rename group…", "Workspace"),
    (DeleteGroup, "group.delete", "Delete group…", "Workspace"),
    (RevealGroup, "group.reveal", REVEAL_GROUP, "Workspace"),
    (MoveGroup, "group.move", "Move group to…", "Workspace"),
    (ReorderGroup, "group.reorder", "Reorder group…", "Workspace"),
    (
        ReorderSection,
        "section.reorder",
        "Reorder section…",
        "Workspace"
    ),
    (
        RenameSection,
        "section.rename",
        "Rename section…",
        "Workspace"
    ),
    (
        DeleteSection,
        "section.delete",
        "Delete section…",
        "Workspace"
    ),
    (RevealSection, "section.reveal", REVEAL_SECTION, "Workspace"),
    (Jump, "navigation.jump", "Jump to note…", "Go"),
    (Search, "navigation.search", "Search workspace…", "Go"),
    (Find, "navigation.find", "Find in note", "Go"),
    (Back, "navigation.back", "Back", "Go"),
    (Forward, "navigation.forward", "Forward", "Go"),
    (Focus, "view.focus", "Toggle focus mode", "View"),
    (
        ResetWidths,
        "view.reset_widths",
        "Reset column widths",
        "View"
    ),
    (
        SortModified,
        "notes.sort_modified",
        "Sort notes: modified",
        "View"
    ),
    (SortTitle, "notes.sort_title", "Sort notes: title", "View"),
    (ChooseTheme, "theme.choose", "Change theme…", "View"),
    (ChooseFont, "font.choose", "Change editor font…", "View"),
    (TextBigger, "font.increase", "Increase text size", "View"),
    (TextSmaller, "font.decrease", "Decrease text size", "View"),
    (TextReset, "font.reset", "Reset text size", "View"),
    (
        SwitchWorkspace,
        "workspace.switch",
        "Switch workspace…",
        "Workspace"
    ),
    (AddWorkspace, "workspace.add", "Add workspace…", "Workspace"),
    (
        RemoveWorkspace,
        "workspace.remove",
        "Remove workspace from list",
        "Workspace"
    ),
    (OpenSettings, "settings.open", "Open settings file", "App"),
    (ReloadSettings, "settings.reload", "Reload settings", "App"),
    (Quit, "app.quit", "Quit Wren", "App"),
    (
        CommandPalette,
        "palette.commands",
        "Show all commands",
        "Hidden"
    ),
];

impl CommandId {
    pub fn key(self) -> &'static str {
        DEFINITIONS
            .iter()
            .find(|definition| definition.id == self)
            .expect("complete command table")
            .key
    }

    pub fn parse(key: &str) -> Option<Self> {
        DEFINITIONS
            .iter()
            .find(|definition| definition.key == key)
            .map(|definition| definition.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemTarget {
    pub generation: u64,
    pub item: ItemRef,
    pub parent: Option<Parent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReorderTarget {
    pub source: ItemTarget,
    pub before: Option<ItemTarget>,
}

#[derive(Debug, Clone)]
pub enum Command {
    Run(CommandId),
    ItemActions(ItemTarget),
    ItemAction { target: ItemTarget, id: CommandId },
    Reorder(ReorderTarget),
    Rename(ItemRef),
    Delete(ItemRef),
    Reveal(ItemRef),
    OpenNote(SearchTarget),
    SelectParent(Parent),
    SelectSection(wren_core::model::SectionId),
    Focus(Option<bool>),
    Width { pane: Pane, width: f32 },
    Sort(String),
    Filter { pane: String, query: String },
    OpenWorkspace { path: PathBuf, name: Option<String> },
    RemoveWorkspace(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Blocked,
    CommittedWarning,
    Completed,
    AsyncPending,
}

impl Disposition {
    #[cfg(feature = "smoke-test")]
    pub fn key(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::CommittedWarning => "committed-warning",
            Self::Completed => "completed",
            Self::AsyncPending => "async-pending",
        }
    }

    pub fn committed(self) -> bool {
        matches!(self, Self::Completed | Self::CommittedWarning)
    }
}

#[derive(Debug, Clone)]
pub struct Processed {
    pub correlation: u64,
    pub disposition: Disposition,
    pub warning: Option<String>,
}

pub struct Gateway {
    pub processed: Processed,
    pub task: Task<Message>,
    pub core_error: Option<String>,
}

impl App {
    pub(super) fn finish_action(
        &mut self,
        disposition: Disposition,
        warning: Option<String>,
        task: Task<Message>,
    ) -> Gateway {
        self.action_serial = self.action_serial.wrapping_add(1);
        let processed = Processed {
            correlation: self.action_serial,
            disposition,
            warning,
        };
        self.last_processed = Some(processed.clone());
        Gateway {
            processed,
            task,
            core_error: None,
        }
    }

    pub(super) fn blocked(&mut self, warning: impl Into<String>) -> Gateway {
        self.finish_action(Disposition::Blocked, Some(warning.into()), Task::none())
    }

    pub(super) fn command_enabled(&self, id: CommandId) -> bool {
        use CommandId::*;
        if self.loading.is_some() {
            return false;
        }
        match id {
            AddWorkspace => self.folder_request.is_none(),
            SwitchWorkspace | ChooseTheme | ChooseFont | TextBigger | TextSmaller | TextReset
            | OpenSettings | ReloadSettings | CommandPalette | Quit => true,
            RemoveWorkspace => self
                .config
                .as_ref()
                .is_some_and(|config| config.writable() && !config.workspaces().is_empty()),
            NewSection | Focus | ResetWidths | Jump | Search => self.session.is_some(),
            NewGroup | RenameSection | DeleteSection | RevealSection | ReorderSection => {
                self.selected_section.is_some() && self.session.is_some()
            }
            NewNote | NewDatedNote | SortModified | SortTitle | RenameGroup | DeleteGroup
            | RevealGroup | MoveGroup | ReorderGroup => {
                self.selected_group.is_some() && self.session.is_some()
            }
            RenameNote | DeleteNote | RevealNote | MoveNote | Find => {
                self.selected_note.is_some() || (!self.workspace_mode && id == Find)
            }
            OpenConflictCopy => self.conflict_copy_target().is_some(),
            Back => self
                .session
                .as_ref()
                .is_some_and(|session| session.can_back()),
            Forward => self
                .session
                .as_ref()
                .is_some_and(|session| session.can_forward()),
        }
    }

    pub(super) fn dispatch(&mut self, command: Command) -> Gateway {
        if self.loading.is_some() {
            return self.blocked("Wait for the workspace to finish opening");
        }
        if self.dialog.is_some() {
            return self.blocked("Close the item dialog before running another command");
        }
        match command {
            Command::Run(id) => {
                if !self.command_enabled(id) {
                    let needs_group = matches!(id, CommandId::NewNote | CommandId::NewDatedNote)
                        && self.selected_section.is_some();
                    return self.blocked(if needs_group {
                        "Create a group first; notes live in groups"
                    } else {
                        "This command is unavailable in the current context"
                    });
                }
                self.run_command(id)
            }
            Command::ItemActions(target) => {
                if !self.item_target_valid(target) {
                    return self.blocked("The row belongs to an old workspace or a changed item");
                }
                self.open_palette(Mode::ItemActions(target.item))
            }
            Command::ItemAction { target, id } => self.run_item_action(target, id),
            Command::Reorder(target) => self.reorder_item(target),
            Command::Rename(target) => self.command_dialog(DialogKind::Rename(target)),
            Command::Delete(target) => self.command_dialog(DialogKind::Delete(target)),
            Command::Reveal(target) => self.reveal(target),
            Command::OpenNote(target) => self.activate_note(target),
            Command::SelectParent(parent) => {
                let outcome = self.process_core(Op::SelectParent { parent });
                if outcome.processed.disposition.committed() {
                    self.reveal_selection();
                }
                outcome
            }
            Command::SelectSection(section) => {
                let outcome = self.process_core(Op::SelectSection { section });
                if outcome.processed.disposition.committed() {
                    self.reveal_selection();
                }
                outcome
            }
            Command::Focus(enabled) => {
                if self.session.is_none() {
                    return self.blocked("No workspace is open");
                }
                self.cancel_gestures();
                self.focus = enabled.unwrap_or(!self.focus);
                self.finish_action(Disposition::Completed, None, focus_editor())
            }
            Command::Width { pane, width } => {
                if self.session.is_none() {
                    return self.blocked("No workspace is open");
                }
                let width = self.widths.set(pane, width);
                self.process_core(Op::SetWidth {
                    pane: pane.key().into(),
                    width: width.round() as u16,
                })
            }
            Command::Sort(mode) => {
                let Some(group) = self.selected_group else {
                    return self.blocked("Select a group first");
                };
                self.process_core(Op::SetSort { group, mode })
            }
            Command::Filter { pane, query } => {
                if let Err(error) = wren_core::index::validate_query(&query) {
                    self.banner = Some(error.into());
                    self.banner_kind = super::workspace::WarningKind::General;
                    return self.blocked(error);
                }
                if !self.filters.set(&pane, query) {
                    return self.blocked("Unknown browser filter");
                }
                self.finish_action(Disposition::Completed, None, Task::none())
            }
            Command::OpenWorkspace { path, name } => {
                if self.loading.is_some() {
                    return self.blocked("A workspace is already loading");
                }
                self.registration_name = name;
                let outcome = self.process_core(Op::OpenWorkspace { path });
                if outcome.processed.disposition == Disposition::Blocked {
                    self.registration_name = None;
                }
                outcome
            }
            Command::RemoveWorkspace(path) => self.remove_registration(&path),
        }
    }

    fn command_dialog(&mut self, kind: DialogKind) -> Gateway {
        let valid = match kind {
            DialogKind::Rename(target) | DialogKind::Delete(target) => {
                self.item_name(target).is_some()
            }
            _ => true,
        };
        if !valid || self.session.is_none() || self.loading.is_some() {
            return self.blocked("The item no longer exists or the workspace is loading");
        }
        self.palette = None;
        let task = self.show_dialog(kind);
        self.finish_action(Disposition::Completed, None, task)
    }

    fn run_command(&mut self, id: CommandId) -> Gateway {
        use CommandId::*;
        match id {
            SwitchWorkspace => self.open_palette(Mode::Workspaces),
            AddWorkspace => self.pick_workspace(),
            RemoveWorkspace => self.open_workspace_removal(),
            NewSection => self.command_dialog(DialogKind::CreateSection),
            NewGroup => self.command_dialog(DialogKind::CreateGroup(
                self.selected_section.expect("enabled section"),
            )),
            NewNote => self.command_dialog(DialogKind::CreateNote(
                self.selected_group.expect("enabled group"),
            )),
            NewDatedNote => {
                let outcome = self.process_core(Op::CreateNote {
                    group: self.selected_group.expect("enabled group"),
                    title: wren_core::ops::dated_title(),
                });
                if outcome.processed.disposition.committed() {
                    self.palette = None;
                    self.reveal_selection();
                }
                outcome
            }
            RenameNote | RenameGroup | RenameSection | DeleteNote | DeleteGroup | DeleteSection
            | RevealNote | RevealGroup | RevealSection => {
                let target = match id {
                    RenameNote | DeleteNote | RevealNote => {
                        ItemRef::Note(self.selected_note.expect("enabled note"))
                    }
                    RenameGroup | DeleteGroup | RevealGroup => {
                        ItemRef::Group(self.selected_group.expect("enabled group"))
                    }
                    _ => ItemRef::Section(self.selected_section.expect("enabled section")),
                };
                match id {
                    RenameNote | RenameGroup | RenameSection => {
                        self.dispatch(Command::Rename(target))
                    }
                    DeleteNote | DeleteGroup | DeleteSection => {
                        self.dispatch(Command::Delete(target))
                    }
                    _ => self.dispatch(Command::Reveal(target)),
                }
            }
            MoveNote => {
                self.open_palette(Mode::MoveNote(self.selected_note.expect("enabled note")))
            }
            MoveGroup => {
                self.open_palette(Mode::MoveGroup(self.selected_group.expect("enabled group")))
            }
            ReorderSection => self.open_palette(Mode::Reorder(ItemRef::Section(
                self.selected_section.expect("enabled section"),
            ))),
            ReorderGroup => self.open_palette(Mode::Reorder(ItemRef::Group(
                self.selected_group.expect("enabled group"),
            ))),
            OpenConflictCopy => {
                let Some(target) = self.conflict_copy_target() else {
                    return self.blocked("The conflict copy no longer exists in this workspace");
                };
                self.dispatch(Command::OpenNote(target))
            }
            Jump => self.open_palette(Mode::Jump),
            Search => self.open_palette(Mode::Search),
            Find => {
                self.palette = None;
                self.editor.break_group();
                self.find_open = true;
                self.finish_action(
                    Disposition::Completed,
                    None,
                    iced::widget::operation::focus("find-query"),
                )
            }
            Back | Forward => {
                let outcome = self.process_core(if id == Back { Op::Back } else { Op::Forward });
                if outcome.processed.disposition.committed() {
                    self.palette = None;
                    self.reveal_selection();
                }
                outcome
            }
            Focus => {
                self.palette = None;
                self.dispatch(Command::Focus(None))
            }
            ResetWidths => {
                self.cancel_gestures();
                self.widths.reset_all();
                let outcome = self.process_core(Op::ResetWidths);
                self.close_palette_after(outcome)
            }
            SortModified | SortTitle => {
                let outcome = self.dispatch(Command::Sort(
                    if id == SortModified {
                        "modified"
                    } else {
                        "title"
                    }
                    .into(),
                ));
                self.close_palette_after(outcome)
            }
            ChooseTheme => self.open_palette(Mode::Themes),
            ChooseFont => self.open_palette(Mode::Fonts),
            TextBigger => self.step_text_size(Some(1.0)),
            TextSmaller => self.step_text_size(Some(-1.0)),
            TextReset => self.step_text_size(None),
            CommandPalette => self.open_palette(Mode::Commands),
            OpenSettings => self.open_settings(),
            ReloadSettings => {
                self.palette = None;
                let warning = self.reload_settings(true);
                self.finish_action(
                    if warning.is_some() {
                        Disposition::CommittedWarning
                    } else {
                        Disposition::Completed
                    },
                    warning,
                    focus_editor(),
                )
            }
            Quit => {
                self.palette = None;
                match self.request_quit() {
                    Ok(task) => self.finish_action(Disposition::Completed, None, task),
                    Err(error) => self.blocked(error),
                }
            }
        }
    }

    pub(super) fn item_target(&self, item: ItemRef) -> Option<ItemTarget> {
        let model = self.model()?;
        let parent = match item {
            ItemRef::Section(section) => {
                model.section(section)?;
                None
            }
            ItemRef::Group(group) => Some(Parent::Section(model.group(group)?.section)),
            ItemRef::Note(note) => Some(Parent::Group(model.note(note)?.group)),
        };
        Some(ItemTarget {
            generation: self.workspace_generation(),
            item,
            parent,
        })
    }

    pub(super) fn item_target_valid(&self, target: ItemTarget) -> bool {
        self.item_target(target.item) == Some(target)
    }

    fn run_item_action(&mut self, target: ItemTarget, id: CommandId) -> Gateway {
        if !self.item_target_valid(target) {
            return self.blocked("The row belongs to an old workspace or a changed item");
        }
        if !item_commands(target.item).contains(&id) {
            return self.blocked("This action does not apply to this row");
        }
        use CommandId::*;
        match id {
            RenameNote | RenameGroup | RenameSection => self.dispatch(Command::Rename(target.item)),
            DeleteNote | DeleteGroup | DeleteSection => self.dispatch(Command::Delete(target.item)),
            RevealNote | RevealGroup | RevealSection => self.dispatch(Command::Reveal(target.item)),
            MoveNote => match target.item {
                ItemRef::Note(note) => self.open_palette(Mode::MoveNote(note)),
                _ => unreachable!("validated note action"),
            },
            MoveGroup => match target.item {
                ItemRef::Group(group) => self.open_palette(Mode::MoveGroup(group)),
                _ => unreachable!("validated group action"),
            },
            ReorderSection | ReorderGroup => self.open_palette(Mode::Reorder(target.item)),
            _ => self.blocked("This action does not apply to this row"),
        }
    }

    fn reorder_item(&mut self, target: ReorderTarget) -> Gateway {
        let valid_anchor = target.before.is_none_or(|before| {
            self.item_target_valid(before)
                && before.parent == target.source.parent
                && before.item != target.source.item
                && matches!(
                    (target.source.item, before.item),
                    (ItemRef::Section(_), ItemRef::Section(_))
                        | (ItemRef::Group(_), ItemRef::Group(_))
                )
        });
        if !self.item_target_valid(target.source)
            || matches!(target.source.item, ItemRef::Note(_))
            || !valid_anchor
        {
            return self
                .blocked("The reorder source or position no longer exists in its original parent");
        }
        self.process_core(Op::Reorder {
            target: target.source.item,
            before: target.before.map(|before| before.item),
        })
    }

    fn conflict_copy_target(&self) -> Option<SearchTarget> {
        let (generation, path) = self.conflict_copy.as_ref()?;
        if *generation != self.workspace_generation() {
            return None;
        }
        let note = self
            .model()?
            .notes
            .iter()
            .find(|note| &note.path == path)?
            .id;
        Some(SearchTarget {
            generation: *generation,
            index_revision: self.session.as_ref()?.index().revision(),
            note,
            query: String::new(),
            content_match: None,
        })
    }

    pub(super) fn close_palette_after(&mut self, mut outcome: Gateway) -> Gateway {
        if outcome.processed.disposition.committed() {
            self.palette = None;
            if self.dialog.is_none() {
                outcome.task = outcome.task.chain(focus_editor());
            }
        }
        outcome
    }

    fn reveal(&mut self, target: ItemRef) -> Gateway {
        // Reconcile and then resolve the current stable ID. No stored pathname
        // or shell text is allowed to become an operating-system argument.
        let refresh = self.process_core(Op::Reconcile);
        if refresh.processed.disposition == Disposition::Blocked {
            return refresh;
        }
        let Some(model) = self.model() else {
            return self.blocked("No workspace is open");
        };
        let Some(relative) = model.path(target) else {
            return self.blocked("The item no longer exists");
        };
        let root = model.root.clone();
        let mut path = root.clone();
        for part in relative.as_path().components() {
            let std::path::Component::Normal(part) = part else {
                return self.blocked("The item has an invalid workspace path");
            };
            path.push(part);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if !metadata.file_type().is_symlink() => {}
                _ => return self.blocked("The reveal target is missing or contains a symlink"),
            }
        }
        if std::fs::canonicalize(&root).ok().as_deref() != Some(root.as_path()) {
            return self.blocked("The workspace root has changed");
        }
        #[cfg(target_os = "macos")]
        let launched = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn();
        #[cfg(not(target_os = "macos"))]
        let launched = std::process::Command::new("xdg-open")
            .arg(if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(&root)
            })
            .spawn();
        match launched {
            Ok(mut child) => {
                self.palette = None;
                let correlation = self.action_serial.wrapping_add(1);
                let generation = self.workspace_generation();
                self.finish_action(
                    Disposition::AsyncPending,
                    None,
                    Task::perform(
                        async move {
                            child
                                .wait()
                                .map_err(|error| error.to_string())
                                .and_then(|status| {
                                    if status.success() {
                                        Ok(())
                                    } else {
                                        Err(format!("File browser exited with {status}"))
                                    }
                                })
                        },
                        move |result| Message::Revealed {
                            correlation,
                            generation,
                            result,
                        },
                    ),
                )
            }
            Err(error) => self.blocked(format!("Could not reveal the item: {error}")),
        }
    }
}

pub(super) fn item_commands(item: ItemRef) -> &'static [CommandId] {
    use CommandId::*;
    match item {
        ItemRef::Section(_) => &[RenameSection, DeleteSection, RevealSection, ReorderSection],
        ItemRef::Group(_) => &[
            RenameGroup,
            DeleteGroup,
            RevealGroup,
            MoveGroup,
            ReorderGroup,
        ],
        ItemRef::Note(_) => &[RenameNote, DeleteNote, RevealNote, MoveNote],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    use crate::editor_view;
    use iced::widget::text_editor::{Action as EditorAction, Edit, Motion};
    use wren_core::config::Config;
    use wren_core::fs_store::{Fault, RelPath};
    use wren_core::model::{GroupId, NoteId, SectionId};
    use wren_core::ops::Session;

    use crate::app::palette::Key;
    use crate::cli::Options;

    fn fixture() -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        for path in [
            "Alpha",
            "Alpha/A",
            "Alpha/B",
            "Alpha/C",
            "Beta",
            "Beta/Foreign",
            "Gamma",
        ] {
            std::fs::create_dir(directory.path().join(path)).unwrap();
        }
        for (path, text) in [
            ("Alpha/A/First.md", "first body"),
            ("Alpha/A/Peer.md", "peer body"),
            ("Beta/Foreign/Other.md", "other body"),
        ] {
            std::fs::write(directory.path().join(path), text).unwrap();
        }
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        let first = note(&app, "First");
        let _ = app.process_core(Op::OpenNote { note: Some(first) });
        (directory, app)
    }

    fn note(app: &App, title: &str) -> NoteId {
        app.model()
            .unwrap()
            .notes
            .iter()
            .find(|note| note.title == title)
            .unwrap()
            .id
    }

    fn group(app: &App, name: &str) -> GroupId {
        app.model()
            .unwrap()
            .groups
            .iter()
            .find(|group| group.name == name)
            .unwrap()
            .id
    }

    fn section(app: &App, name: &str) -> SectionId {
        app.model()
            .unwrap()
            .sections
            .iter()
            .find(|section| section.name == name)
            .unwrap()
            .id
    }

    fn append(app: &mut App, text: &str) {
        let _ = app.edit(editor_view::Message::Action(EditorAction::Move(
            Motion::DocumentEnd,
        )));
        let _ = app.edit(editor_view::Message::Action(EditorAction::Edit(
            Edit::Paste(Arc::new(text.into())),
        )));
    }

    fn choose(app: &mut App, title: &str) -> Gateway {
        let index = app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .position(|row| row.title == title)
            .unwrap();
        app.activate_palette(Some(index))
    }

    fn group_order(app: &App, parent: SectionId) -> Vec<String> {
        app.model()
            .unwrap()
            .groups
            .iter()
            .filter(|group| group.section == parent)
            .map(|group| group.name.clone())
            .collect()
    }

    #[test]
    fn full_command_protocol_is_unique_and_round_trips() {
        let mut keys = HashSet::new();
        for definition in DEFINITIONS {
            assert!(keys.insert(definition.key));
            assert_eq!(CommandId::parse(definition.key), Some(definition.id));
        }
        for key in [
            "workspace.add",
            "workspace.switch",
            "workspace.remove",
            "section.new",
            "group.new",
            "note.new",
            "note.new_dated",
            "note.rename",
            "group.rename",
            "section.rename",
            "note.delete",
            "group.delete",
            "section.delete",
            "note.reveal",
            "note.move",
            "group.move",
            "navigation.jump",
            "navigation.search",
            "navigation.find",
            "navigation.back",
            "navigation.forward",
            "view.focus",
            "view.reset_widths",
            "notes.sort_modified",
            "notes.sort_title",
            "theme.choose",
            "settings.open",
            "settings.reload",
            "app.quit",
            "palette.commands",
        ] {
            assert!(CommandId::parse(key).is_some(), "{key}");
        }
        assert!(CommandId::parse("save").is_none());
        for key in ["section.reorder", "group.reorder", "note.open_conflict"] {
            assert!(CommandId::parse(key).is_some(), "{key}");
        }
    }

    #[test]
    fn palette_query_limits_retain_the_last_valid_query_results_and_editor() {
        let (_directory, mut app) = fixture();
        append(&mut app, " retained draft");
        let text = app.editor.text();
        let cursor = app.editor.cursor();
        let selected = app.selected_note;
        for mode in [
            Mode::Commands,
            Mode::Jump,
            Mode::Search,
            Mode::Workspaces,
            Mode::MoveNote(selected.unwrap()),
            Mode::MoveGroup(group(&app, "A")),
            Mode::ItemActions(ItemRef::Note(selected.unwrap())),
            Mode::Reorder(ItemRef::Section(section(&app, "Alpha"))),
        ] {
            let _ = app.open_palette(mode);
            let _ = app.palette_query("First".into());
            let keys: Vec<_> = app
                .palette
                .as_ref()
                .unwrap()
                .results
                .iter()
                .map(|row| row.key.clone())
                .collect();
            for query in ["x".repeat(4097), "word ".repeat(129)] {
                let outcome = app.palette_query(query);
                assert_eq!(outcome.processed.disposition, Disposition::Blocked);
                let palette = app.palette.as_ref().unwrap();
                assert_eq!(palette.query, "First");
                assert_eq!(
                    palette
                        .results
                        .iter()
                        .map(|row| row.key.clone())
                        .collect::<Vec<_>>(),
                    keys
                );
                assert!(palette.error.as_deref().unwrap().contains("limited"));
                assert_eq!(app.editor.text(), text);
                assert_eq!(app.editor.cursor(), cursor);
                assert_eq!(app.selected_note, selected);
            }
            let _ = app.update(Message::PaletteQuery("Second".into()));
            assert_eq!(app.palette.as_ref().unwrap().query, "Second");
            assert!(app.palette.as_ref().unwrap().error.is_none());
        }
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    }

    #[test]
    fn pane_query_limits_reject_before_replacing_every_filter_or_editing_the_draft() {
        let (directory, mut app) = fixture();
        append(&mut app, " retained draft");
        let text = app.editor.text();
        let cursor = app.editor.cursor();
        for pane in ["sections", "groups", "notes"] {
            let valid = app.dispatch(Command::Filter {
                pane: pane.into(),
                query: "First".into(),
            });
            assert!(valid.processed.disposition.committed());
            for query in ["é".repeat(2049), "a ".repeat(129)] {
                let outcome = app.dispatch(Command::Filter {
                    pane: pane.into(),
                    query,
                });
                assert_eq!(outcome.processed.disposition, Disposition::Blocked);
                assert!(app.banner.as_deref().unwrap().contains("limited"));
                assert_eq!(app.editor.text(), text);
                assert_eq!(app.editor.cursor(), cursor);
            }
        }
        assert_eq!(app.filters.sections, "First");
        assert_eq!(app.filters.groups, "First");
        assert_eq!(app.filters.notes, "First");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Alpha/A/First.md")).unwrap(),
            "first body"
        );
        for query in ["x".repeat(4096), "a ".repeat(128)] {
            assert!(app
                .dispatch(Command::Filter {
                    pane: "notes".into(),
                    query
                })
                .processed
                .disposition
                .committed());
        }
    }

    #[test]
    fn direct_row_actions_rename_the_explicit_nonselected_note_without_a_note_transition() {
        let (directory, mut app) = fixture();
        append(&mut app, " local draft");
        let selected = app.selected_note;
        let text = app.editor.text();
        let peer = note(&app, "Peer");
        let target = app.item_target(ItemRef::Note(peer)).unwrap();
        assert!(app
            .dispatch(Command::ItemActions(target))
            .processed
            .disposition
            .committed());
        assert_eq!(app.palette.as_ref().unwrap().source, Some(target));
        assert!(choose(&mut app, "Rename note…")
            .processed
            .disposition
            .committed());
        assert!(
            matches!(app.dialog.as_ref().unwrap().kind, DialogKind::Rename(ItemRef::Note(id)) if id == peer)
        );
        assert_eq!(app.selected_note, selected);
        assert_eq!(app.editor.text(), text);
        app.dialog.as_mut().unwrap().name = "Renamed peer".into();
        let _ = app.submit_dialog(false);
        assert!(directory.path().join("Alpha/A/Renamed peer.md").exists());
        assert!(!directory.path().join("Alpha/A/Peer.md").exists());
        assert_eq!(app.selected_note, selected);
        assert_eq!(app.editor.text(), text);
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Alpha/A/First.md")).unwrap(),
            "first body"
        );
    }

    #[test]
    fn native_context_and_double_click_route_to_the_explicit_row_with_generation_guards() {
        use crate::drag::{DragEvent, RowKind};
        let (_directory, mut app) = fixture();
        append(&mut app, " draft");
        let selected = app.selected_note;
        let peer = note(&app, "Peer");
        let generation = app.gesture_generation;
        let _ = app.drag_event(
            generation,
            DragEvent::Context {
                generation,
                row: RowKind::Note(peer),
            },
        );
        assert_eq!(
            app.palette.as_ref().unwrap().mode,
            Mode::ItemActions(ItemRef::Note(peer))
        );
        assert_eq!(app.selected_note, selected);
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        let _ = app.close_palette();
        let _ = app.drag_event(
            generation,
            DragEvent::Rename {
                generation,
                row: RowKind::Note(peer),
            },
        );
        assert!(
            app.dialog.is_none(),
            "opening a context palette retired the old gesture generation"
        );
        let generation = app.gesture_generation;
        let _ = app.drag_event(
            generation,
            DragEvent::Rename {
                generation: generation + 1,
                row: RowKind::Note(peer),
            },
        );
        assert!(
            app.dialog.is_none(),
            "a mismatched gesture generation is ignored"
        );
        let _ = app.drag_event(
            generation,
            DragEvent::Rename {
                generation,
                row: RowKind::Note(peer),
            },
        );
        assert!(
            matches!(app.dialog.as_ref().unwrap().kind, DialogKind::Rename(ItemRef::Note(note)) if note == peer)
        );
        assert_eq!(app.selected_note, selected);
    }

    #[test]
    fn row_actions_reject_changed_parent_generation_and_wrong_item_kind() {
        let (_directory, mut app) = fixture();
        let target = app.item_target(ItemRef::Group(group(&app, "A"))).unwrap();
        assert_eq!(
            app.dispatch(Command::ItemAction {
                target,
                id: CommandId::RenameNote
            })
            .processed
            .disposition,
            Disposition::Blocked
        );
        let mut stale = target;
        stale.generation += 1;
        assert_eq!(
            app.dispatch(Command::ItemActions(stale))
                .processed
                .disposition,
            Disposition::Blocked
        );
        let to = section(&app, "Beta");
        let group = match target.item {
            ItemRef::Group(group) => group,
            _ => unreachable!(),
        };
        let _ = app.process_core(Op::MoveGroup { group, to });
        assert_eq!(
            app.dispatch(Command::ItemAction {
                target,
                id: CommandId::RenameGroup
            })
            .processed
            .disposition,
            Disposition::Blocked
        );
        assert!(app.dialog.is_none());
    }

    #[test]
    fn row_rename_of_the_active_note_still_uses_the_dirty_document_barrier() {
        let (directory, mut app) = fixture();
        append(&mut app, " retained");
        let selected = app.selected_note;
        let target = app.item_target(ItemRef::Note(selected.unwrap())).unwrap();
        let _ = app.dispatch(Command::ItemAction {
            target,
            id: CommandId::RenameNote,
        });
        app.dialog.as_mut().unwrap().name = "Must not publish".into();
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        let _ = app.submit_dialog(false);
        assert_eq!(
            app.last_processed.as_ref().unwrap().disposition,
            Disposition::Blocked
        );
        assert!(app.dialog.as_ref().unwrap().error.is_some());
        assert_eq!(app.selected_note, selected);
        assert!(app.editor.text().ends_with(" retained"));
        assert!(!directory
            .path()
            .join("Alpha/A/Must not publish.md")
            .exists());
    }

    #[test]
    fn direct_row_delete_and_reorder_actions_use_the_row_not_the_document_parent() {
        let (_directory, mut app) = fixture();
        let selected = app.selected_note;
        let target = app
            .item_target(ItemRef::Group(group(&app, "Foreign")))
            .unwrap();
        let _ = app.dispatch(Command::ItemAction {
            target,
            id: CommandId::DeleteGroup,
        });
        assert!(
            matches!(app.dialog.as_ref().unwrap().kind, DialogKind::Delete(item) if item == target.item)
        );
        let _ = app.update(Message::CancelDialog);
        let _ = app.dispatch(Command::ItemAction {
            target,
            id: CommandId::ReorderGroup,
        });
        assert_eq!(
            app.palette.as_ref().unwrap().mode,
            Mode::Reorder(target.item)
        );
        assert_eq!(app.palette.as_ref().unwrap().source, Some(target));
        assert_eq!(app.selected_note, selected);
    }

    #[test]
    fn palette_reorder_preserves_hidden_siblings_and_persists_section_and_group_order() {
        let (directory, mut app) = fixture();
        let alpha = section(&app, "Alpha");
        let beta = section(&app, "Beta");
        let gamma = section(&app, "Gamma");
        let _ = app.open_palette(Mode::Reorder(ItemRef::Section(gamma)));
        let _ = app.palette_query("Before Alpha".into());
        let outcome = app.activate_palette(None);
        assert!(outcome.processed.disposition.committed());
        assert!(outcome.task.units() > 0);
        assert!(app.palette.is_none());
        assert_eq!(
            app.model()
                .unwrap()
                .sections
                .iter()
                .map(|section| section.id)
                .collect::<Vec<_>>(),
            vec![gamma, alpha, beta]
        );
        let _ = app.dispatch(Command::Filter {
            pane: "groups".into(),
            query: "C".into(),
        });
        let _ = app.open_palette(Mode::Reorder(ItemRef::Group(group(&app, "C"))));
        assert!(app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .any(|row| row.title == "Before A"));
        let _ = app.palette_query("Before A".into());
        assert!(app.activate_palette(None).processed.disposition.committed());
        assert_eq!(group_order(&app, alpha), ["C", "A", "B"]);
        let _ = app.open_palette(Mode::Reorder(ItemRef::Group(group(&app, "A"))));
        let _ = app.palette_query("Move to end".into());
        assert!(app.activate_palette(None).processed.disposition.committed());
        assert_eq!(group_order(&app, alpha), ["C", "B", "A"]);
        drop(app);
        let session = Session::open(directory.path()).unwrap();
        let alpha = session
            .model()
            .sections
            .iter()
            .find(|section| section.name == "Alpha")
            .unwrap()
            .id;
        assert_eq!(
            session
                .model()
                .sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            ["Gamma", "Alpha", "Beta"]
        );
        assert_eq!(
            session
                .model()
                .groups
                .iter()
                .filter(|group| group.section == alpha)
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            ["C", "B", "A"]
        );
    }

    #[test]
    fn keyboard_enter_does_not_retarget_a_removed_reorder_anchor_after_passive_refresh() {
        let (directory, mut app) = fixture();
        let alpha = section(&app, "Alpha");
        let _ = app.open_palette(Mode::Reorder(ItemRef::Group(group(&app, "C"))));
        let index = app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .position(|row| row.title == "Before B")
            .unwrap();
        app.palette.as_mut().unwrap().active = Some(index);
        std::fs::remove_dir(directory.path().join("Alpha/B")).unwrap();
        let _ = app.process_core(Op::Reconcile);
        let outcome = app.activate_palette(None);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert_eq!(group_order(&app, alpha), ["A", "C"]);
        assert!(app.palette.as_ref().unwrap().error.is_some());
    }

    #[test]
    fn captured_reorder_keys_cannot_turn_a_moved_or_deleted_anchor_into_append() {
        for removed in [false, true] {
            let (directory, mut app) = fixture();
            let alpha = section(&app, "Alpha");
            let source = group(&app, "C");
            let anchor = group(&app, "B");
            let _ = app.open_palette(Mode::Reorder(ItemRef::Group(source)));
            let key = app
                .palette
                .as_ref()
                .unwrap()
                .results
                .iter()
                .find(|row| row.title == "Before B")
                .unwrap()
                .key
                .clone();
            let Key::Reorder(captured) = key.clone() else {
                panic!("typed reorder key")
            };
            if removed {
                std::fs::remove_dir(directory.path().join("Alpha/B")).unwrap();
                let _ = app.process_core(Op::Reconcile);
            } else {
                let _ = app.process_core(Op::MoveGroup {
                    group: anchor,
                    to: section(&app, "Beta"),
                });
            }
            let order = group_order(&app, alpha);
            let generation = app.workspace_generation();
            assert_eq!(
                app.activate_palette_key(generation, key)
                    .processed
                    .disposition,
                Disposition::Blocked
            );
            assert_eq!(
                app.dispatch(Command::Reorder(captured))
                    .processed
                    .disposition,
                Disposition::Blocked
            );
            assert_eq!(group_order(&app, alpha), order);
            assert!(app.palette.is_some());
        }
    }

    #[test]
    fn reorder_rejects_moved_source_cross_section_anchor_notes_and_old_generation() {
        let (_directory, mut app) = fixture();
        let source = app.item_target(ItemRef::Group(group(&app, "C"))).unwrap();
        let before = app
            .item_target(ItemRef::Group(group(&app, "Foreign")))
            .unwrap();
        assert_eq!(
            app.dispatch(Command::Reorder(ReorderTarget {
                source,
                before: Some(before)
            }))
            .processed
            .disposition,
            Disposition::Blocked
        );
        let mut stale = source;
        stale.generation += 1;
        assert_eq!(
            app.dispatch(Command::Reorder(ReorderTarget {
                source: stale,
                before: None
            }))
            .processed
            .disposition,
            Disposition::Blocked
        );
        let _ = app.open_palette(Mode::Reorder(source.item));
        let _ = app.process_core(Op::MoveGroup {
            group: group(&app, "C"),
            to: section(&app, "Beta"),
        });
        assert_eq!(
            app.dispatch(Command::Reorder(ReorderTarget {
                source,
                before: None
            }))
            .processed
            .disposition,
            Disposition::Blocked
        );
        assert_eq!(
            choose(&mut app, "Move to end").processed.disposition,
            Disposition::Blocked
        );
        assert!(app.palette.is_some());
        let note = app.selected_note.unwrap();
        assert_eq!(
            app.open_palette(Mode::Reorder(ItemRef::Note(note)))
                .processed
                .disposition,
            Disposition::Blocked
        );
    }

    #[test]
    fn deleted_reorder_source_is_blocked_without_replaying_a_different_source() {
        let (directory, mut app) = fixture();
        let alpha = section(&app, "Alpha");
        let source = app.item_target(ItemRef::Group(group(&app, "C"))).unwrap();
        let _ = app.open_palette(Mode::Reorder(source.item));
        std::fs::remove_dir(directory.path().join("Alpha/C")).unwrap();
        let _ = app.process_core(Op::Reconcile);
        assert_eq!(
            app.activate_palette(None).processed.disposition,
            Disposition::Blocked
        );
        assert_eq!(
            app.dispatch(Command::Reorder(ReorderTarget {
                source,
                before: None
            }))
            .processed
            .disposition,
            Disposition::Blocked
        );
        assert_eq!(group_order(&app, alpha), ["A", "B"]);
        assert!(app.palette.is_some());
    }

    #[test]
    fn committed_reorder_metadata_warning_dismisses_picker_and_prevents_replay() {
        let (_directory, mut app) = fixture();
        let alpha = section(&app, "Alpha");
        let _ = app.open_palette(Mode::Reorder(ItemRef::Group(group(&app, "C"))));
        let _ = app.palette_query("Before A".into());
        app.session.as_mut().unwrap().fail_next(Fault::FileSync);
        let outcome = app.activate_palette(None);
        assert_eq!(outcome.processed.disposition, Disposition::CommittedWarning);
        assert_eq!(group_order(&app, alpha), ["C", "A", "B"]);
        assert!(app.palette.is_none());
        assert!(outcome.task.units() > 0);
        assert!(app.banner.is_some());
        assert_eq!(
            app.activate_palette(None).processed.disposition,
            Disposition::Blocked
        );
    }

    #[test]
    fn sort_and_reset_palette_actions_schedule_editor_focus_after_closing() {
        for id in [
            CommandId::SortTitle,
            CommandId::SortModified,
            CommandId::ResetWidths,
        ] {
            let (_directory, mut app) = fixture();
            let _ = app.open_palette(Mode::Commands);
            let outcome = app.activate_palette_key(app.workspace_generation(), Key::Command(id));
            assert!(outcome.processed.disposition.committed(), "{id:?}");
            assert!(app.palette.is_none(), "{id:?}");
            assert!(
                outcome.task.units() > 0,
                "{id:?} must return a runtime focus operation"
            );
        }
    }

    #[test]
    fn successful_move_pickers_schedule_focus_but_a_blocked_move_keeps_the_palette() {
        let (_directory, mut app) = fixture();
        let selected = app.selected_note.unwrap();
        append(&mut app, " retained");
        let _ = app.open_palette(Mode::MoveNote(selected));
        let _ = app.palette_query("Beta Foreign".into());
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        let blocked = app.activate_palette(None);
        assert_eq!(blocked.processed.disposition, Disposition::Blocked);
        assert!(app.palette.is_some());
        assert_eq!(blocked.task.units(), 0);
        let outcome = app.activate_palette(None);
        assert!(outcome.processed.disposition.committed());
        assert!(app.palette.is_none());
        assert!(outcome.task.units() > 0);
        let _ = app.open_palette(Mode::MoveGroup(group(&app, "C")));
        let _ = app.palette_query("Beta".into());
        let outcome = app.activate_palette(None);
        assert!(outcome.processed.disposition.committed());
        assert!(app.palette.is_none());
        assert!(outcome.task.units() > 0);
        assert_eq!(group_order(&app, section(&app, "Beta")), ["Foreign", "C"]);
    }

    #[test]
    fn read_only_reorder_is_blocked_without_dismissing_or_refocusing_away_from_the_palette() {
        let (directory, mut app) = fixture();
        drop(app.session.take());
        std::fs::write(directory.path().join(".wren/workspace.toml"), "invalid = [").unwrap();
        app.session = Some(Session::open(directory.path()).unwrap());
        app.sync_editor(true);
        let _ = app.open_palette(Mode::Reorder(ItemRef::Group(group(&app, "C"))));
        let _ = app.palette_query("Before A".into());
        let outcome = app.activate_palette(None);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert_eq!(outcome.task.units(), 0);
        assert!(app
            .palette
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("read-only"));
        assert_eq!(group_order(&app, section(&app, "Alpha")), ["A", "B", "C"]);
    }

    #[test]
    fn registered_workspace_replacement_is_blocked_before_flushing_or_canonicalizing() {
        use std::os::unix::fs::symlink;
        let (directory, mut app) = fixture();
        let registrations = crate::test_support::tempdir();
        let ancestor = registrations.path().join("ancestor");
        std::fs::create_dir(&ancestor).unwrap();
        let stored = ancestor.join("Stored");
        std::fs::create_dir(&stored).unwrap();
        let foreign = registrations.path().join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::create_dir(foreign.join("Stored")).unwrap();
        let mut config = Config::load_from(&registrations.path().join("config"));
        config.add(&stored, Some("Stored")).unwrap();
        app.config = Some(config);
        append(&mut app, " retained draft");
        let selected = app.selected_note;
        let generation = app.workspace_generation();
        let _ = app.open_palette(Mode::Workspaces);
        let _ = app.palette_query("Stored".into());
        std::fs::rename(&ancestor, registrations.path().join("original")).unwrap();
        symlink(&foreign, &ancestor).unwrap();
        let outcome = app.activate_palette(None);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert!(outcome
            .processed
            .warning
            .as_deref()
            .unwrap()
            .contains("symlink"));
        assert_eq!(app.model().unwrap().root, directory.path());
        assert_eq!(app.workspace_generation(), generation);
        assert_eq!(app.selected_note, selected);
        assert!(app.loading.is_none());
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(app.palette.as_ref().unwrap().query, "Stored");
        assert!(app.palette.as_ref().unwrap().error.is_some());
        assert_eq!(app.config.as_ref().unwrap().workspaces()[0].path, stored);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Alpha/A/First.md")).unwrap(),
            "first body"
        );
    }

    #[test]
    fn valid_stored_workspace_can_start_opening_with_read_only_optional_config() {
        use std::os::unix::fs::PermissionsExt;
        let (_directory, mut app) = fixture();
        let registrations = crate::test_support::tempdir();
        let stored = registrations.path().join("Stored");
        std::fs::create_dir(&stored).unwrap();
        let config_dir = registrations.path().join("config");
        let mut config = Config::load_from(&config_dir);
        config.add(&stored, None).unwrap();
        std::fs::set_permissions(
            config_dir.join("config.toml"),
            std::fs::Permissions::from_mode(0o444),
        )
        .unwrap();
        app.config = Some(Config::load_from(&config_dir));
        assert!(!app.config.as_ref().unwrap().writable());
        let _ = app.open_palette(Mode::Workspaces);
        let _ = app.palette_query("Stored".into());
        let outcome = app.activate_palette(None);
        assert_eq!(outcome.processed.disposition, Disposition::AsyncPending);
        assert!(app.loading.is_some());
    }

    #[test]
    fn conflict_copy_command_resolves_a_current_relative_model_note_through_the_barrier() {
        let (_directory, mut app) = fixture();
        let peer = note(&app, "Peer");
        let path = app.model().unwrap().note(peer).unwrap().path.clone();
        app.conflict_copy = Some((app.workspace_generation(), path.clone()));
        app.banner = Some("A conflict copy was saved".into());
        append(&mut app, " retained");
        let selected = app.selected_note;
        let banner = app.banner.clone();
        assert!(app.command_enabled(CommandId::OpenConflictCopy));
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        let outcome = app.dispatch(Command::Run(CommandId::OpenConflictCopy));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert_eq!(app.selected_note, selected);
        assert!(app.editor.text().ends_with(" retained"));
        assert!(app.banner.is_some());
        assert_eq!(app.conflict_copy, Some((app.workspace_generation(), path)));
        assert!(banner.is_some());
        let outcome = app.dispatch(Command::Run(CommandId::OpenConflictCopy));
        assert!(outcome.processed.disposition.committed());
        assert_eq!(app.selected_note, Some(peer));
        assert_eq!(app.editor.text(), "peer body");
    }

    #[test]
    fn generated_conflict_copy_opens_the_preserved_local_bytes_not_the_external_original() {
        let (directory, mut app) = fixture();
        append(&mut app, " local draft");
        std::fs::write(
            directory.path().join("Alpha/A/First.md"),
            "external version",
        )
        .unwrap();
        let outcome = app.process_core(Op::Flush);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert_eq!(app.editor.text(), "external version");
        let (_, path) = app
            .conflict_copy
            .clone()
            .expect("core-generated relative conflict copy");
        assert!(!path.as_path().is_absolute());
        assert!(app.command_enabled(CommandId::OpenConflictCopy));
        let outcome = app.dispatch(Command::Run(CommandId::OpenConflictCopy));
        assert!(outcome.processed.disposition.committed());
        assert_eq!(app.editor.text(), "first body local draft");
        assert_eq!(
            app.model()
                .unwrap()
                .note(app.selected_note.unwrap())
                .unwrap()
                .path,
            path
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Alpha/A/First.md")).unwrap(),
            "external version"
        );
    }

    #[test]
    fn stale_or_missing_conflict_copy_never_opens_an_arbitrary_path_or_changes_the_draft() {
        let (_directory, mut app) = fixture();
        append(&mut app, " retained");
        let selected = app.selected_note;
        let text = app.editor.text();
        for (generation, path) in [
            (app.workspace_generation() + 1, "Alpha/A/Peer.md"),
            (app.workspace_generation(), "Alpha/A/Missing.md"),
        ] {
            app.conflict_copy = Some((generation, RelPath::new(path).unwrap()));
            app.banner = Some("Keep conflict warning".into());
            assert!(!app.command_enabled(CommandId::OpenConflictCopy));
            assert_eq!(
                app.dispatch(Command::Run(CommandId::OpenConflictCopy))
                    .processed
                    .disposition,
                Disposition::Blocked
            );
            assert_eq!(app.editor.text(), text);
            assert_eq!(app.selected_note, selected);
            assert_eq!(app.banner.as_deref(), Some("Keep conflict warning"));
        }
        assert!(RelPath::new("/tmp/external.md").is_err());
    }
}
