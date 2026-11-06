use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::Task;
use wren_core::model::{GroupId, ItemRef, NoteId, Parent, SectionId, Workspace};
#[cfg(feature = "smoke-test")]
use wren_core::ops::Document;
use wren_core::ops::{Event, ExternalChangeKind, Op, Session};

use super::{focus_editor, App, Message};

#[derive(Debug, Clone)]
pub enum DialogKind {
    CreateSection,
    CreateGroup(SectionId),
    CreateNote(Parent),
    Rename(ItemRef),
    Delete(ItemRef),
    MoveNote(NoteId),
    MoveGroup(GroupId),
}

pub(super) struct Dialog {
    pub kind: DialogKind,
    pub name: String,
    pub destination: Option<Parent>,
    pub error: Option<String>,
}

#[cfg(feature = "smoke-test")]
#[derive(Debug, Clone)]
pub(super) struct DocumentProjection {
    pub note: NoteId,
    pub text: String,
    pub path: PathBuf,
    pub dirty: bool,
    pub error: Option<String>,
}

#[cfg(feature = "smoke-test")]
impl From<&Document> for DocumentProjection {
    fn from(document: &Document) -> Self {
        Self {
            note: document.note,
            text: document.text.clone(),
            path: document.path().as_path().to_path_buf(),
            dirty: document.dirty(),
            error: document.error.clone(),
        }
    }
}

#[derive(Debug)]
pub(super) struct Loading {
    pub tag: u64,
    pub model: Option<Workspace>,
    #[cfg(feature = "smoke-test")]
    pub document: Option<DocumentProjection>,
    #[cfg(feature = "smoke-test")]
    pub generation: u64,
    #[cfg(feature = "smoke-test")]
    pub pending_durability: bool,
    #[cfg(feature = "smoke-test")]
    pub last_save_micros: u128,
}

#[derive(Debug)]
pub struct LoadOutcome {
    session: Option<Session>,
    result: Result<Vec<Event>, wren_core::ops::OpError>,
    opened: bool,
}

fn load_workspace(
    previous: Option<Session>,
    path: PathBuf,
    #[cfg(feature = "smoke-test")] sandbox: Option<PathBuf>,
) -> LoadOutcome {
    if let Some(mut session) = previous {
        let generation = session.generation();
        let result = session.apply(Op::OpenWorkspace { path });
        let opened = session.generation() != generation;
        LoadOutcome {
            session: Some(session),
            result,
            opened,
        }
    } else {
        #[cfg(feature = "smoke-test")]
        let result = match sandbox {
            Some(sandbox) => crate::test_trash::Quarantine::new(&sandbox)
                .map_err(|message| wren_core::ops::OpError {
                    message,
                    events: Vec::new(),
                })
                .and_then(|trash| Session::with_trash(&path, Box::new(trash))),
            None => Session::open(&path),
        };
        #[cfg(not(feature = "smoke-test"))]
        let result = Session::open(&path);
        match result {
            Ok(session) => LoadOutcome {
                session: Some(session),
                result: Ok(Vec::new()),
                opened: true,
            },
            Err(error) => LoadOutcome {
                session: None,
                result: Err(error),
                opened: false,
            },
        }
    }
}

impl App {
    // This is the single entry point for both widgets and private test requests.
    // Opening moves the same serial session owner to a background task.
    pub(super) fn apply_core(&mut self, op: Op) -> Result<Task<Message>, String> {
        if self.loading.is_some() {
            return Err("A workspace is already loading; wait for it to finish".into());
        }
        if let Op::OpenWorkspace { path } = op {
            return self.begin_workspace(path).inspect_err(|error| {
                self.banner = Some(error.clone());
            });
        }
        let result = self
            .session
            .as_mut()
            .ok_or("No workspace is open")?
            .apply(op.clone());
        let (events, mut error) = match result {
            Ok(events) => (events, None),
            Err(error) => (error.events, Some(error.message)),
        };
        if matches!(op, Op::Flush) && error.is_none() {
            self.banner = None;
        }
        // Publication may precede a metadata or selection failure. Only a
        // matching Created event proves this dialog must not replay its action.
        let creation_committed = self.dialog.as_ref().is_some_and(|dialog| {
            events.iter().any(|event| match (&dialog.kind, &op, event) {
                (
                    DialogKind::CreateSection,
                    Op::CreateSection { .. },
                    Event::Created(ItemRef::Section(_)),
                ) => true,
                (
                    DialogKind::CreateGroup(expected),
                    Op::CreateGroup { section, .. },
                    Event::Created(ItemRef::Group(_)),
                ) => expected == section,
                (
                    DialogKind::CreateNote(expected),
                    Op::CreateNote { parent, .. },
                    Event::Created(ItemRef::Note(_)),
                ) => expected == parent,
                _ => false,
            })
        });
        if creation_committed {
            self.dialog = None;
        }
        let created = self.process_events(&events);
        if !events.is_empty() || error.is_some() {
            self.sync_editor(false);
        }
        if let Some(note) = created {
            if let Err(selection_error) = self.apply_core(Op::OpenNote { note: Some(note) }) {
                if error.is_none() {
                    error = Some(selection_error);
                }
            }
        }
        if let Some(error) = error {
            self.banner = Some(error.clone());
            Err(error)
        } else if created.is_some()
            || matches!(op, Op::OpenNote { note: Some(_) } | Op::Back | Op::Forward)
        {
            Ok(focus_editor())
        } else {
            Ok(Task::none())
        }
    }

    fn begin_workspace(&mut self, path: PathBuf) -> Result<Task<Message>, String> {
        if !path.is_absolute() {
            return Err("Workspace path must be absolute".into());
        }
        if self.session.is_some() {
            let _ = self.apply_core(Op::Flush)?;
        }
        let model = self.session.as_ref().map(|session| {
            let source = session.model();
            let mut model = Workspace::empty(source.root.clone());
            model.sections = source.sections.clone();
            model.groups = source.groups.clone();
            model.notes = source.notes.clone();
            model.warnings = source.warnings.clone();
            model
        });
        #[cfg(feature = "smoke-test")]
        let document = self.document_projection();
        self.load_generation = self.load_generation.wrapping_add(1);
        let tag = self.load_generation;
        self.loading = Some(Loading {
            tag,
            model,
            #[cfg(feature = "smoke-test")]
            document,
            #[cfg(feature = "smoke-test")]
            generation: self.session.as_ref().map_or(0, Session::generation),
            #[cfg(feature = "smoke-test")]
            pending_durability: self
                .session
                .as_ref()
                .is_some_and(Session::pending_durability),
            #[cfg(feature = "smoke-test")]
            last_save_micros: self.session.as_ref().map_or(0, Session::last_save_micros),
        });
        self.workspace_mode = true;
        self.workspace_path = path.to_string_lossy().into_owned();
        self.dialog = None;
        let previous = self.session.take();
        #[cfg(feature = "smoke-test")]
        let sandbox = self.test_sandbox.clone();
        Ok(Task::perform(
            async move {
                let outcome = load_workspace(
                    previous,
                    path,
                    #[cfg(feature = "smoke-test")]
                    sandbox,
                );
                Arc::new(Mutex::new(Some(outcome)))
            },
            move |outcome| Message::Loaded(tag, outcome),
        ))
    }

    pub(super) fn workspace_loaded(
        &mut self,
        tag: u64,
        outcome: Arc<Mutex<Option<LoadOutcome>>>,
    ) -> Task<Message> {
        if self.loading.as_ref().map(|loading| loading.tag) != Some(tag) {
            return Task::none();
        }
        let Some(outcome) = outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        else {
            return Task::none();
        };
        self.loading = None;
        self.session = outcome.session;
        let (events, mut error) = match outcome.result {
            Ok(events) => (events, None),
            Err(error) => (error.events, Some(error.message)),
        };
        self.process_events(&events);
        if outcome.opened {
            self.selected_section = None;
            self.selected_group = None;
            self.banner = self.session.as_ref().and_then(|session| {
                session
                    .model()
                    .warnings
                    .first()
                    .map(|warning| format!("{}: {}", warning.path.display(), warning.message))
            });
        }
        self.sync_editor(outcome.opened);
        if outcome.opened {
            if let Some(session) = &self.session {
                if session.document().is_none() {
                    if let Some(note) = session.model().notes.first().map(|note| note.id) {
                        if let Err(selection_error) =
                            self.apply_core(Op::OpenNote { note: Some(note) })
                        {
                            if error.is_none() {
                                error = Some(selection_error);
                            }
                        }
                    }
                }
            }
        }
        if let Some(session) = &self.session {
            self.workspace_path = session.model().root.to_string_lossy().into_owned();
        }
        if let Some(error) = &error {
            self.banner = Some(error.clone());
        }
        #[cfg(feature = "smoke-test")]
        if let Some(request) = self.workspace_reply.take() {
            request.respond(
                error
                    .clone()
                    .map_or_else(|| Ok(self.session_snapshot()), Err),
            );
        }
        if self.flush_on_load && !self.quit_pending && self.session.is_some() {
            let _ = self.apply_core(Op::Flush);
        }
        self.flush_on_load = false;
        if self.quit_pending {
            self.quit_pending = false;
            let result = self.request_quit();
            #[cfg(feature = "smoke-test")]
            if let Some(request) = self.quit_reply.take() {
                request.respond(
                    result
                        .as_ref()
                        .map(|_| serde_json::Value::Null)
                        .map_err(Clone::clone),
                );
            }
            return result.unwrap_or_else(|_| Task::none());
        }
        if outcome.opened && error.is_none() {
            focus_editor()
        } else {
            Task::none()
        }
    }

    fn process_events(&mut self, events: &[Event]) -> Option<NoteId> {
        let mut created = None;
        for event in events {
            match event {
                Event::Created(ItemRef::Note(note)) => created = Some(*note),
                Event::Warning(message) => self.banner = Some(message.clone()),
                Event::ExternalChange { kind, .. } => match kind {
                    ExternalChangeKind::Reloaded => {}
                    ExternalChangeKind::Conflict { copy } => {
                        self.banner = Some(format!(
                            "External changes were preserved. Your draft was saved to {}.",
                            copy.display()
                        ));
                    }
                    ExternalChangeKind::Deleted => {
                        self.banner = Some("The open note was removed outside Wren.".into());
                    }
                },
                _ => {}
            }
        }
        created
    }

    fn sync_editor(&mut self, workspace_changed: bool) {
        let Some(session) = &self.session else {
            return;
        };
        let note = session.document().map(|document| document.note);
        let text = session
            .document()
            .map_or("", |document| document.text.as_str());
        let identity_changed = workspace_changed || self.editor_note != note;
        if identity_changed || self.editor.text() != text {
            self.editor.replace_text(text, !identity_changed);
            self.words = text.split_whitespace().count();
            self.edit_revision += 1;
            self.editor_note = note;
        }
        self.selected_note = note;
        if let Some(entry) = note.and_then(|note| session.model().note(note)) {
            match entry.parent {
                Parent::Section(section) => {
                    self.selected_section = Some(section);
                    self.selected_group = None;
                }
                Parent::Group(group) => {
                    self.selected_group = Some(group);
                    self.selected_section = session.model().group(group).map(|group| group.section);
                }
            }
        } else {
            self.selected_section = self
                .selected_section
                .filter(|id| session.model().section(*id).is_some())
                .or_else(|| session.model().sections.first().map(|section| section.id));
            self.selected_group = self.selected_group.filter(|id| {
                session
                    .model()
                    .group(*id)
                    .is_some_and(|group| Some(group.section) == self.selected_section)
            });
        }
    }

    pub(super) fn model(&self) -> Option<&Workspace> {
        self.session.as_ref().map(Session::model).or_else(|| {
            self.loading
                .as_ref()
                .and_then(|loading| loading.model.as_ref())
        })
    }

    #[cfg(feature = "smoke-test")]
    pub(super) fn document_projection(&self) -> Option<DocumentProjection> {
        self.session
            .as_ref()
            .and_then(Session::document)
            .map(DocumentProjection::from)
            .or_else(|| {
                self.loading
                    .as_ref()
                    .and_then(|loading| loading.document.clone())
            })
    }

    pub(super) fn selected_parent(&self) -> Option<Parent> {
        self.selected_group
            .map(Parent::Group)
            .or_else(|| self.selected_section.map(Parent::Section))
    }

    pub(super) fn select_parent(&mut self, parent: Parent) -> Result<(), String> {
        if self.loading.is_some() {
            return Err("Workspace is still loading".into());
        }
        let model = self.model().ok_or("No workspace is open")?;
        if model.parent_path(parent).is_none() {
            return Err("The destination no longer exists".into());
        }
        let note = model
            .notes
            .iter()
            .find(|note| note.parent == parent)
            .map(|note| note.id);
        let section = match parent {
            Parent::Section(section) => section,
            Parent::Group(group) => model.group(group).ok_or("Group no longer exists")?.section,
        };
        let _ = self.apply_core(Op::Flush)?;
        let _ = self.apply_core(Op::OpenNote { note })?;
        self.selected_section = Some(section);
        self.selected_group = match parent {
            Parent::Section(_) => None,
            Parent::Group(group) => Some(group),
        };
        Ok(())
    }

    pub(super) fn ensure_editor_ready(&self) -> Result<(), String> {
        if self.loading.is_some() {
            return Err("The editor is disabled while a workspace is loading".into());
        }
        if self.workspace_mode && self.session.as_ref().and_then(Session::document).is_none() {
            return Err("Select a note before editing".into());
        }
        Ok(())
    }

    pub(super) fn window_unfocused(&mut self) -> Result<(), String> {
        self.editor.break_group();
        if self.loading.is_some() {
            self.flush_on_load = true;
        } else if self.session.is_some() {
            let _ = self.apply_core(Op::Flush)?;
        }
        Ok(())
    }

    pub(super) fn request_quit(&mut self) -> Result<Task<Message>, String> {
        if self.loading.is_some() {
            self.quit_pending = true;
            return Ok(Task::none());
        }
        if self.session.is_some() {
            let _ = self.apply_core(Op::Flush)?;
        }
        Ok(iced::exit())
    }

    pub(super) fn item_name(&self, target: ItemRef) -> Option<&str> {
        let model = self.model()?;
        match target {
            ItemRef::Section(id) => model.section(id).map(|section| section.name.as_str()),
            ItemRef::Group(id) => model.group(id).map(|group| group.name.as_str()),
            ItemRef::Note(id) => model.note(id).map(|note| note.title.as_str()),
        }
    }

    pub(super) fn show_dialog(&mut self, kind: DialogKind) -> Task<Message> {
        if self.session.is_none() || self.loading.is_some() {
            return Task::none();
        }
        self.editor.break_group();
        let (name, destination) = match kind {
            DialogKind::CreateNote(_) => (wren_core::ops::dated_title(), None),
            DialogKind::Rename(target) => (self.item_name(target).unwrap_or_default().into(), None),
            DialogKind::MoveNote(note) => (
                String::new(),
                self.model()
                    .and_then(|model| model.note(note))
                    .map(|note| note.parent),
            ),
            DialogKind::MoveGroup(group) => (
                String::new(),
                self.model()
                    .and_then(|model| model.group(group))
                    .map(|group| Parent::Section(group.section)),
            ),
            _ => (String::new(), None),
        };
        let focus_name = matches!(
            kind,
            DialogKind::CreateSection
                | DialogKind::CreateGroup(_)
                | DialogKind::CreateNote(_)
                | DialogKind::Rename(_)
        );
        self.dialog = Some(Dialog {
            kind,
            name,
            destination,
            error: None,
        });
        if focus_name {
            iced::widget::operation::focus("item-name")
        } else {
            Task::none()
        }
    }

    pub(super) fn submit_dialog(&mut self, confirm_trash: bool) -> Task<Message> {
        let Some(dialog) = &self.dialog else {
            return Task::none();
        };
        if matches!(dialog.kind, DialogKind::Delete(_)) != confirm_trash {
            return Task::none();
        }
        let name = dialog.name.clone();
        let op = match dialog.kind {
            DialogKind::CreateSection => Op::CreateSection { name },
            DialogKind::CreateGroup(section) => Op::CreateGroup { section, name },
            DialogKind::CreateNote(parent) => Op::CreateNote {
                parent,
                title: name,
            },
            DialogKind::Rename(target) => Op::Rename {
                target,
                new_name: name,
            },
            DialogKind::Delete(target) => Op::Delete { target },
            DialogKind::MoveNote(note) => match dialog.destination {
                Some(to) => Op::MoveNote { note, to },
                None => return Task::none(),
            },
            DialogKind::MoveGroup(group) => match dialog.destination {
                Some(Parent::Section(to)) => Op::MoveGroup { group, to },
                _ => return Task::none(),
            },
        };
        match self.apply_core(op) {
            Ok(task) => {
                self.dialog = None;
                task.chain(focus_editor())
            }
            Err(error) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.error = Some(error);
                    Task::none()
                } else {
                    focus_editor()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position};
    use wren_core::fs_store::Fault;
    use wren_iced::editor_view;

    use super::*;
    use crate::cli::Options;

    fn fixture() -> (tempfile::TempDir, App) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("Inbox")).unwrap();
        std::fs::write(directory.path().join("Inbox/Note.md"), "first\nsecond").unwrap();
        let (mut app, _) = App::new(Options::default());
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        app.sync_editor(true);
        let note = app.model().unwrap().notes[0].id;
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        (directory, app)
    }

    fn append(app: &mut App, text: &str) {
        let _ = app.update(Message::Editor(editor_view::Message::Action(Action::Move(
            Motion::DocumentEnd,
        ))));
        let _ = app.update(Message::Editor(editor_view::Message::Action(Action::Edit(
            Edit::Paste(Arc::new(text.into())),
        ))));
    }

    fn returning(app: &mut App, outcome: LoadOutcome) -> Arc<Mutex<Option<LoadOutcome>>> {
        app.workspace_mode = true;
        app.load_generation += 1;
        app.loading = Some(Loading {
            tag: app.load_generation,
            model: None,
            #[cfg(feature = "smoke-test")]
            document: None,
            #[cfg(feature = "smoke-test")]
            generation: 0,
            #[cfg(feature = "smoke-test")]
            pending_durability: false,
            #[cfg(feature = "smoke-test")]
            last_save_micros: 0,
        });
        Arc::new(Mutex::new(Some(outcome)))
    }

    fn load(previous: Option<Session>, path: PathBuf) -> LoadOutcome {
        load_workspace(
            previous,
            path,
            #[cfg(feature = "smoke-test")]
            None,
        )
    }

    #[test]
    fn editor_actions_undo_and_redo_update_the_active_core_draft() {
        let (directory, mut app) = fixture();
        append(&mut app, " café");
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            "first\nsecond café"
        );
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/Note.md")).unwrap(),
            "first\nsecond"
        );
        let _ = app.update(Message::Editor(editor_view::Message::Undo));
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            "first\nsecond"
        );
        let _ = app.update(Message::Editor(editor_view::Message::Redo));
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            app.editor.text()
        );
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/Note.md")).unwrap(),
            app.editor.text()
        );
    }

    #[test]
    fn rename_move_and_theme_changes_preserve_live_history_and_cursor() {
        let (_directory, mut app) = fixture();
        let _ = app
            .apply_core(Op::CreateSection {
                name: "Archive".into(),
            })
            .unwrap();
        let destination = app
            .model()
            .unwrap()
            .sections
            .iter()
            .find(|section| section.name == "Archive")
            .unwrap()
            .id;
        let note = app.selected_note.unwrap();
        append(&mut app, " edit");
        let cursor = app.editor.cursor();
        let _ = app
            .apply_core(Op::Rename {
                target: ItemRef::Note(note),
                new_name: "Renamed".into(),
            })
            .unwrap();
        let _ = app
            .apply_core(Op::MoveNote {
                note,
                to: Parent::Section(destination),
            })
            .unwrap();
        let _ = app.update(Message::ChooseTheme(Some(wren_core::theme::LIGHT)));
        assert_eq!(app.editor.cursor(), cursor);
        assert!(app.editor.can_undo());
        assert_eq!(app.selected_note, Some(note));
        assert_eq!(app.selected_section, Some(destination));
        let _ = app.update(Message::Editor(editor_view::Message::Undo));
        assert_eq!(app.editor.text(), "first\nsecond");
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            app.editor.text()
        );
        let _ = app.update(Message::Editor(editor_view::Message::Redo));
        assert_eq!(app.editor.text(), "first\nsecond edit");
    }

    #[test]
    fn failed_quit_and_blur_barriers_retain_the_session_and_draft() {
        let (directory, mut app) = fixture();
        append(&mut app, " unsaved");
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        assert!(app.request_quit().is_err());
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        assert!(app.banner.is_some());
        app.session.as_mut().unwrap().fail_next(Fault::FileSync);
        let _ = app.update(Message::Window(
            iced::window::Id::unique(),
            iced::window::Event::Unfocused,
        ));
        assert!(app
            .session
            .as_ref()
            .unwrap()
            .document()
            .unwrap()
            .error
            .is_some());
        assert_eq!(app.editor.text(), "first\nsecond unsaved");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/Note.md")).unwrap(),
            "first\nsecond"
        );
        app.window_unfocused().unwrap();
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/Note.md")).unwrap(),
            app.editor.text()
        );
    }

    #[test]
    fn selecting_an_empty_parent_must_flush_before_changing_selection() {
        let (_directory, mut app) = fixture();
        let _ = app
            .apply_core(Op::CreateSection {
                name: "Empty".into(),
            })
            .unwrap();
        let destination = app
            .model()
            .unwrap()
            .sections
            .iter()
            .find(|section| section.name == "Empty")
            .unwrap()
            .id;
        let before = (app.selected_section, app.selected_group, app.selected_note);
        append(&mut app, " retained");
        app.session
            .as_mut()
            .unwrap()
            .fail_next(Fault::BeforePublish);
        assert!(app.select_parent(Parent::Section(destination)).is_err());
        assert_eq!(
            (app.selected_section, app.selected_group, app.selected_note),
            before
        );
        app.select_parent(Parent::Section(destination)).unwrap();
        assert_eq!(app.selected_section, Some(destination));
        assert_eq!(app.selected_note, None);
        assert!(app.editor.text().is_empty());
        append(&mut app, "not allowed");
        assert!(app.editor.text().is_empty());
    }

    #[test]
    fn committed_rename_error_keeps_real_paths_and_editor_history() {
        let (directory, mut app) = fixture();
        append(&mut app, " saved");
        let _ = app.apply_core(Op::Flush).unwrap();
        let cursor = app.editor.cursor();
        let note = app.selected_note.unwrap();
        app.session
            .as_mut()
            .unwrap()
            .fail_next(Fault::DirectorySync);
        assert!(app
            .apply_core(Op::Rename {
                target: ItemRef::Note(note),
                new_name: "Published".into()
            })
            .is_err());
        assert!(directory.path().join("Inbox/Published.md").exists());
        assert!(!directory.path().join("Inbox/Note.md").exists());
        assert_eq!(
            app.session
                .as_ref()
                .unwrap()
                .document()
                .unwrap()
                .path()
                .as_path(),
            std::path::Path::new("Inbox/Published.md")
        );
        assert_eq!(app.editor.cursor(), cursor);
        assert!(app.editor.can_undo());
    }

    #[test]
    fn committed_create_error_still_projects_and_selects_the_new_note() {
        let (_directory, mut app) = fixture();
        let parent = app.selected_parent().unwrap();
        app.session
            .as_mut()
            .unwrap()
            .fail_next(Fault::DirectorySync);
        assert!(app
            .apply_core(Op::CreateNote {
                parent,
                title: "Created".into()
            })
            .is_err());
        let created = app
            .model()
            .unwrap()
            .notes
            .iter()
            .find(|note| note.title == "Created")
            .unwrap()
            .id;
        assert_eq!(app.selected_note, Some(created));
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().note,
            created
        );
        assert!(app.editor.text().is_empty());
    }

    #[test]
    fn committed_creations_close_the_dialog_but_keep_the_error_banner() {
        for kind in 0..3 {
            let (directory, mut app) = fixture();
            let section = app.selected_section.unwrap();
            let parent = app.selected_parent().unwrap();
            let dialog = match kind {
                0 => DialogKind::CreateSection,
                1 => DialogKind::CreateGroup(section),
                _ => DialogKind::CreateNote(parent),
            };
            let _ = app.show_dialog(dialog);
            app.dialog.as_mut().unwrap().name = "Created".into();
            app.session.as_mut().unwrap().fail_next(if kind == 2 {
                Fault::DirectorySync
            } else {
                Fault::FileSync
            });
            let _ = app.update(Message::SubmitDialog);
            assert!(app.dialog.is_none());
            let error = app.banner.clone().expect("publication error stays visible");
            let counts = {
                let model = app.model().unwrap();
                (model.sections.len(), model.groups.len(), model.notes.len())
            };
            let path = match kind {
                0 => "Created",
                1 => "Inbox/Created",
                _ => "Inbox/Created.md",
            };
            assert!(directory.path().join(path).exists());
            if kind != 2 {
                assert!(error.contains("workspace.toml"));
            } else {
                assert_eq!(app.editor.text(), "");
                assert_eq!(
                    app.item_name(ItemRef::Note(app.selected_note.unwrap())),
                    Some("Created")
                );
            }
            let _ = app.update(Message::SubmitDialog);
            let model = app.model().unwrap();
            assert_eq!(
                counts,
                (model.sections.len(), model.groups.len(), model.notes.len())
            );
            assert_eq!(app.banner.as_ref(), Some(&error));
            assert!(!directory
                .path()
                .join(path.replace("Created", "Created 2"))
                .exists());
        }
    }

    #[test]
    fn unpublished_creations_keep_a_retryable_dialog() {
        for kind in 0..3 {
            let (directory, mut app) = fixture();
            let dialog = match kind {
                0 => DialogKind::CreateSection,
                1 => DialogKind::CreateGroup(app.selected_section.unwrap()),
                _ => DialogKind::CreateNote(app.selected_parent().unwrap()),
            };
            let _ = app.show_dialog(dialog);
            app.dialog.as_mut().unwrap().name = "Created".into();
            app.session
                .as_mut()
                .unwrap()
                .fail_next(Fault::BeforePublish);
            let _ = app.update(Message::SubmitDialog);
            let dialog = app.dialog.as_ref().expect("creation remains retryable");
            assert!(dialog.error.is_some());
            assert_eq!(dialog.name, "Created");
            let path = match kind {
                0 => "Created",
                1 => "Inbox/Created",
                _ => "Inbox/Created.md",
            };
            assert!(!directory.path().join(path).exists());
            let _ = app.update(Message::SubmitDialog);
            assert!(app.dialog.is_none());
            assert!(directory.path().join(path).exists());
        }
    }

    #[test]
    fn unrelated_flush_events_do_not_dismiss_a_creation_dialog() {
        let (_directory, mut app) = fixture();
        append(&mut app, " saved");
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        let _ = app.show_dialog(DialogKind::CreateSection);
        app.dialog.as_mut().unwrap().name = "Still pending".into();
        let _ = app.apply_core(Op::Flush).unwrap();
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(app.dialog.as_ref().unwrap().name, "Still pending");
    }

    #[test]
    fn clean_external_reload_preserves_cursor_and_uses_actual_session_text() {
        let (directory, mut app) = fixture();
        app.editor.move_cursor(Cursor {
            position: Position { line: 1, column: 3 },
            selection: None,
        });
        std::fs::write(
            directory.path().join("Inbox/Note.md"),
            "external\nreplacement",
        )
        .unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_eq!(app.editor.text(), "external\nreplacement");
        assert_eq!(
            app.editor.cursor().position,
            Position { line: 1, column: 3 }
        );
        assert_eq!(
            app.editor.text(),
            app.session.as_ref().unwrap().document().unwrap().text
        );
    }

    #[test]
    fn conflict_error_reloads_the_original_without_reverting_committed_events() {
        let (directory, mut app) = fixture();
        append(&mut app, " local");
        std::fs::write(directory.path().join("Inbox/Note.md"), "external original").unwrap();
        assert!(app.apply_core(Op::Flush).is_err());
        assert_eq!(app.editor.text(), "external original");
        assert_eq!(
            app.editor.text(),
            app.session.as_ref().unwrap().document().unwrap().text
        );
        assert!(app
            .model()
            .unwrap()
            .notes
            .iter()
            .any(|note| note.title.contains("(conflict ")));
    }

    #[test]
    fn deleted_dirty_note_conflict_selects_the_saved_recovery_identity_even_on_error() {
        let (directory, mut app) = fixture();
        let old = app.selected_note;
        append(&mut app, " recovered");
        std::fs::remove_file(directory.path().join("Inbox/Note.md")).unwrap();
        assert!(app.apply_core(Op::Flush).is_err());
        assert_ne!(app.selected_note, old);
        assert_eq!(
            app.selected_note,
            app.session
                .as_ref()
                .unwrap()
                .document()
                .map(|document| document.note)
        );
        assert_eq!(app.editor.text(), "first\nsecond recovered");
    }

    #[test]
    fn a_failed_workspace_switch_returns_the_original_session_and_projection() {
        let (directory, mut app) = fixture();
        append(&mut app, " retained");
        let _ = app.apply_core(Op::Flush).unwrap();
        let cursor = app.editor.cursor();
        let previous = app.session.take();
        let outcome = load(previous, directory.path().join("missing"));
        assert!(outcome.result.is_err());
        let slot = returning(&mut app, outcome);
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.model().unwrap().root, directory.path());
        assert_eq!(app.editor.text(), "first\nsecond retained");
        assert_eq!(app.editor.cursor(), cursor);
        assert!(app.editor.can_undo());
        assert!(app.banner.is_some());
    }

    #[test]
    fn failed_preswitch_flush_does_not_transfer_session_ownership() {
        let (_directory, mut app) = fixture();
        let next = tempfile::tempdir().unwrap();
        append(&mut app, " retained");
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        assert!(app
            .apply_core(Op::OpenWorkspace {
                path: next.path().into()
            })
            .is_err());
        assert!(app.loading.is_none());
        assert!(app.session.is_some());
        assert_eq!(app.editor.text(), "first\nsecond retained");
    }

    #[test]
    fn load_generation_guards_edits_operations_and_stale_results() {
        let (_directory, mut app) = fixture();
        let next = tempfile::tempdir().unwrap();
        let _pending = app
            .apply_core(Op::OpenWorkspace {
                path: next.path().into(),
            })
            .unwrap();
        let before = app.editor.text();
        assert!(app.session.is_none());
        assert!(app.ensure_editor_ready().is_err());
        assert!(app.apply_core(Op::Flush).is_err());
        assert!(app
            .apply_core(Op::OpenWorkspace {
                path: next.path().into()
            })
            .is_err());
        append(&mut app, "ignored");
        assert_eq!(app.editor.text(), before);
        let stale = Arc::new(Mutex::new(Some(load(None, next.path().into()))));
        let _ = app.workspace_loaded(app.load_generation.wrapping_sub(1), Arc::clone(&stale));
        assert!(stale.lock().unwrap().is_some());
        assert!(app.loading.is_some());
        let _ = app.request_quit().unwrap();
        assert!(app.quit_pending);
    }

    #[test]
    fn loaded_workspace_selects_its_first_note_and_keeps_empty_sections_usable() {
        let (directory, mut app) = fixture();
        let first = app.selected_note;
        let previous = app.session.take();
        drop(previous);
        let slot = returning(&mut app, load(None, directory.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.selected_note, first);
        let empty = tempfile::tempdir().unwrap();
        std::fs::create_dir(empty.path().join("Empty")).unwrap();
        let previous = app.session.take();
        let slot = returning(&mut app, load(previous, empty.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert!(app.selected_section.is_some());
        assert!(app.selected_note.is_none());
        assert!(app.editor.text().is_empty());
        assert!(app.ensure_editor_ready().is_err());
    }

    #[test]
    fn delete_dialog_ignores_enter_submission_without_explicit_confirmation() {
        let (directory, mut app) = fixture();
        let note = app.selected_note.unwrap();
        let _ = app.show_dialog(DialogKind::Delete(ItemRef::Note(note)));
        let _ = app.update(Message::SubmitDialog);
        assert!(app.dialog.is_some());
        assert!(directory.path().join("Inbox/Note.md").exists());
        let _ = app.update(Message::Escape);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn autosave_subscription_exists_only_while_the_session_is_owned_by_the_app() {
        let (mut app, _) = App::new(Options::default());
        let count =
            |app: &App| iced::advanced::subscription::into_recipes(app.subscription()).len();
        let baseline = count(&app);
        let (_directory, loaded) = fixture();
        app.session = loaded.session;
        assert_eq!(count(&app), baseline + 1);
        app.session = None;
        assert_eq!(count(&app), baseline);
    }

    #[test]
    fn quit_during_loading_waits_and_a_failed_returned_flush_keeps_the_draft() {
        let (_directory, mut app) = fixture();
        append(&mut app, " retained");
        let mut session = app.session.take().unwrap();
        session.fail_next(Fault::Write);
        let slot = returning(
            &mut app,
            LoadOutcome {
                session: Some(session),
                result: Ok(Vec::new()),
                opened: false,
            },
        );
        let _ = app.request_quit().unwrap();
        assert!(app.quit_pending);
        app.window_unfocused().unwrap();
        assert!(app.flush_on_load);
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert!(!app.quit_pending);
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(app.editor.text(), "first\nsecond retained");
        assert!(app.banner.is_some());
    }

    #[cfg(feature = "smoke-test")]
    #[test]
    fn test_quarantine_survives_a_core_workspace_switch() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let sandbox = tempfile::tempdir().unwrap();
        std::fs::create_dir(second.path().join("Section")).unwrap();
        std::fs::write(second.path().join("Section/Note.md"), "quarantined").unwrap();
        let first = load_workspace(None, first.path().into(), Some(sandbox.path().into()))
            .session
            .unwrap();
        let outcome = load_workspace(Some(first), second.path().into(), None);
        assert!(outcome.result.is_ok());
        let (mut app, _) = App::new(Options::default());
        let slot = returning(&mut app, outcome);
        let _ = app.workspace_loaded(app.load_generation, slot);
        let note = app.selected_note.unwrap();
        let _ = app
            .apply_core(Op::Delete {
                target: ItemRef::Note(note),
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("trash/item-00000000")).unwrap(),
            "quarantined"
        );
        assert!(!second.path().join("Section/Note.md").exists());
    }

    #[cfg(feature = "smoke-test")]
    #[test]
    fn transport_failure_does_not_exit_or_discard_a_dirty_document() {
        let (_directory, mut app) = fixture();
        append(&mut app, " retained");
        let _ = app.update(Message::Test(crate::test_ipc::Incoming::Failed(
            "closed".into(),
        )));
        assert!(app
            .banner
            .as_deref()
            .unwrap()
            .contains("window remains open"));
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(app.editor.text(), "first\nsecond retained");
    }
}
