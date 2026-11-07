use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::autosave::Autosave;
use crate::fs_store::{DiskFile, Expected, Fault, FsStore, RelPath, WriteReceipt};
use crate::index::Index;
use crate::model::{
    self, Group, GroupId, ItemRef, Note, NoteId, Parent, Section, SectionId, Workspace,
};
use crate::scan;
use crate::watch::Watch;
use crate::workspace_state::WorkspaceState;

#[derive(Debug, Clone)]
pub enum Op {
    CreateSection {
        name: String,
    },
    CreateGroup {
        section: SectionId,
        name: String,
    },
    CreateNote {
        parent: Parent,
        title: String,
    },
    Rename {
        target: ItemRef,
        new_name: String,
    },
    Delete {
        target: ItemRef,
    },
    MoveNote {
        note: NoteId,
        to: Parent,
    },
    MoveGroup {
        group: GroupId,
        to: SectionId,
    },
    Reorder {
        target: ItemRef,
        before: Option<ItemRef>,
    },
    WriteNote {
        note: NoteId,
        contents: String,
    },
    OpenWorkspace {
        path: PathBuf,
    },
    OpenNote {
        note: Option<NoteId>,
    },
    /// Select exactly this parent, including an empty ungrouped section parent.
    SelectParent {
        parent: Parent,
    },
    /// Section-row navigation: direct notes, then the first ordered group,
    /// otherwise the empty section. `selected_parent` exposes the resolved parent.
    SelectSection {
        section: SectionId,
    },
    SetLayout {
        layout: String,
    },
    SetWidth {
        layout: String,
        pane: String,
        width: u16,
    },
    ResetWidths,
    SetSort {
        parent: Parent,
        mode: String,
    },
    EditDraft {
        note: NoteId,
        contents: String,
    },
    Flush,
    Tick,
    Reconcile,
    Back,
    Forward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalChangeKind {
    Reloaded,
    Conflict { copy: PathBuf },
    Deleted,
}

#[derive(Debug, Clone)]
pub enum Event {
    TreeChanged,
    Created(ItemRef),
    NoteMoved {
        note: NoteId,
        new_path: PathBuf,
    },
    NoteSaved {
        note: NoteId,
        mtime: SystemTime,
    },
    DraftChanged(NoteId),
    Selected(Option<NoteId>),
    ExternalChange {
        note: NoteId,
        kind: ExternalChangeKind,
    },
    WorkspaceOpened,
    Warning(String),
}

#[derive(Debug)]
pub struct OpError {
    pub message: String,
    // A flush or filesystem publication can succeed before a later step fails.
    // Consumers must process these events even when the overall operation fails.
    pub events: Vec<Event>,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OpError {}

pub trait Trash: Send {
    fn trash(&mut self, path: &Path) -> Result<(), String>;
}

pub fn dated_title() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub struct SystemTrash;

impl Trash for SystemTrash {
    fn trash(&mut self, path: &Path) -> Result<(), String> {
        trash::delete(path).map_err(|error| format!("Could not move item to OS trash: {error}"))
    }
}

#[derive(Debug)]
pub struct Document {
    pub note: NoteId,
    pub text: String,
    pub error: Option<String>,
    entry: Note,
    baseline: Vec<u8>,
    autosave: Autosave,
    needs_sync: bool,
    recovery: Option<(RelPath, Vec<u8>)>,
}

impl Document {
    fn new(mut entry: Note, file: DiskFile) -> Result<Self, String> {
        let text = String::from_utf8(file.bytes.clone())
            .map_err(|_| "The note is not valid UTF-8; it was left untouched".to_string())?;
        entry.modified = file.modified;
        entry.bytes = file.bytes.len();
        Ok(Self {
            note: entry.id,
            entry,
            text,
            error: None,
            baseline: file.bytes,
            autosave: Autosave::new(),
            needs_sync: false,
            recovery: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.autosave.dirty() || self.needs_sync
    }

    pub fn path(&self) -> &RelPath {
        &self.entry.path
    }
}

const MAX_NAVIGATION_ENTRIES: usize = 100;

#[derive(Clone, Copy)]
enum Navigation {
    Visit,
    History(usize),
}

pub struct Session {
    store: FsStore,
    model: Workspace,
    state: WorkspaceState,
    index: Index,
    selected_parent: Option<Parent>,
    watcher: Option<Watch>,
    generation: u64,
    document: Option<Document>,
    trash: Box<dyn Trash>,
    started: Instant,
    last_poll: Duration,
    last_durability_retry: Duration,
    history: Vec<NoteId>,
    history_index: Option<usize>,
    recent: Vec<NoteId>,
    last_save_micros: u128,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("model", &self.model)
            .field(
                "document",
                &self.document.as_ref().map(|document| document.note),
            )
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Session {
    pub fn open(path: &Path) -> Result<Self, OpError> {
        Self::with_trash(path, Box::new(SystemTrash))
    }

    pub fn with_trash(path: &Path, trash: Box<dyn Trash>) -> Result<Self, OpError> {
        Self::load(path, trash, 1).map_err(|message| OpError {
            message,
            events: Vec::new(),
        })
    }

    fn load(path: &Path, trash: Box<dyn Trash>, generation: u64) -> Result<Self, String> {
        let store = FsStore::open(path).map_err(|error| error.to_string())?;
        // Subscribe before scanning so an edit during the initial scan is reconciled.
        let watch = Watch::new(store.root(), generation);
        let mut index = Index::default();
        let mut model = scan::scan_with_contents(&store, None, |note, contents| {
            index.ingest(note, contents);
        })
        .map_err(|error| error.to_string())?;
        let watcher = match watch {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                model.warnings.push(model::ScanWarning {
                    path: store.root().to_path_buf(),
                    message: format!("Watcher unavailable; using periodic reconciliation: {error}"),
                });
                None
            }
        };
        let state = WorkspaceState::load(&store);
        if let Some(warning) = state.warning() {
            model.warnings.push(model::ScanWarning {
                path: store.root().join(".wren/workspace.toml"),
                message: warning.into(),
            });
        }
        let notes_by_path: HashMap<_, _> = model
            .notes
            .iter()
            .map(|note| (&note.path, note.id))
            .collect();
        let saved_position = state.history_position();
        let mut history = Vec::new();
        let mut through_position = 0usize;
        for (position, path) in state.history().iter().enumerate() {
            if let Some(note) = notes_by_path.get(path) {
                history.push(*note);
                if saved_position.is_some_and(|saved| position <= saved) {
                    through_position += 1;
                }
            }
        }
        let history_index = saved_position
            .filter(|_| !history.is_empty())
            .map(|_| through_position.saturating_sub(1));
        // Schema-1 files written before MRU support retain their best available
        // visit ordering once; new files never derive MRU from a history branch.
        let recent_paths = state.recent();
        let restoring_recent = !recent_paths.is_empty();
        let recent: Vec<_> = if !restoring_recent {
            history.clone()
        } else {
            recent_paths
                .iter()
                .filter_map(|path| notes_by_path.get(path).copied())
                .collect()
        };
        let mut session = Self {
            store,
            model,
            state,
            index,
            selected_parent: None,
            watcher,
            generation,
            document: None,
            trash,
            started: Instant::now(),
            last_poll: Duration::ZERO,
            last_durability_retry: Duration::ZERO,
            history,
            history_index,
            recent: Vec::new(),
            last_save_micros: 0,
        };
        session.sort_model();
        let selected = session
            .state
            .selection()
            .and_then(|path| session.resolve_selection(&path))
            .or_else(|| {
                session
                    .state
                    .last_open_note()
                    .and_then(|path| session.resolve_selection(&path))
            });
        if let Some((parent, note)) = selected {
            session.selected_parent = Some(parent);
            if let Some(note) = note {
                let entry = session.model.note(note).expect("resolved note").clone();
                let path = entry.path.clone();
                match session
                    .store
                    .read(&path)
                    .map_err(|error| error.to_string())
                    .and_then(|file| Document::new(entry, file))
                {
                    Ok(document) => session.document = Some(document),
                    Err(message) => session.model.warnings.push(model::ScanWarning {
                        path: session.store.root().join(path.as_path()),
                        message,
                    }),
                }
            }
        }
        session.index.sync_metadata(&session.model);
        if let Some(document) = &session.document {
            session
                .index
                .set_content(document.note, &document.text, true);
        }
        for note in recent {
            session.record_recent(note);
        }
        for (path, reason) in session.index.limitations().take(64) {
            session.model.warnings.push(model::ScanWarning {
                path: session.store.root().join(path.as_path()),
                message: format!("{}: {reason}", path.as_path().display()),
            });
        }
        drop(session.index.take_warnings());
        if let Some(note) = session.document.as_ref().map(|document| document.note) {
            if session
                .history_index
                .and_then(|position| session.history.get(position))
                != Some(&note)
            {
                session.history_index = session
                    .history
                    .iter()
                    .rposition(|candidate| *candidate == note);
                if session.history_index.is_none() {
                    session.record_history(note);
                }
            }
            if !restoring_recent {
                session.record_recent(note);
            }
        }
        session.prune_history();
        Ok(session)
    }

    pub fn model(&self) -> &Workspace {
        &self.model
    }
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }
    pub fn state(&self) -> &WorkspaceState {
        &self.state
    }
    pub fn index(&self) -> &Index {
        &self.index
    }
    pub fn selected_parent(&self) -> Option<Parent> {
        self.selected_parent
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn last_save_micros(&self) -> u128 {
        self.last_save_micros
    }
    pub fn pending_durability(&self) -> bool {
        self.store.pending_durability()
    }
    pub fn fail_next(&mut self, fault: Fault) {
        self.store.fail_next(fault);
    }
    pub fn can_back(&self) -> bool {
        self.history_index.is_some_and(|index| index > 0)
    }
    pub fn can_forward(&self) -> bool {
        self.history_index
            .is_some_and(|index| index + 1 < self.history.len())
    }

    pub fn apply(&mut self, op: Op) -> Result<Vec<Event>, OpError> {
        self.apply_at(op, self.started.elapsed())
    }

    pub fn apply_at(&mut self, op: Op, now: Duration) -> Result<Vec<Event>, OpError> {
        let mut events = Vec::new();
        let mut result = self.execute(op, now, &mut events);
        // A conflict copy can be published before its directory sync fails.
        // Reconcile it after the active draft has been returned to the Session.
        if self.document.as_ref().is_some_and(|document| {
            document
                .recovery
                .as_ref()
                .is_some_and(|(copy, _)| !self.model.notes.iter().any(|note| note.path == *copy))
        }) {
            let refreshed = self.refresh_tree(&mut events);
            if result.is_ok() {
                result = refreshed;
            }
        }
        let added = self.index.sync_metadata(&self.model);
        for note in added {
            self.index.read_note(&self.store, note);
        }
        if let Some(document) = &self.document {
            self.index.set_content(document.note, &document.text, true);
            self.selected_parent = Some(document.entry.parent);
        } else if self
            .selected_parent
            .is_some_and(|parent| self.model.parent_path(parent).is_none())
        {
            self.selected_parent = None;
        }
        events.extend(self.index.take_warnings().into_iter().map(Event::Warning));
        events.extend(self.store.take_warnings().into_iter().map(Event::Warning));
        match result {
            Ok(()) => Ok(events),
            Err(message) => Err(OpError { message, events }),
        }
    }

    fn execute(&mut self, op: Op, now: Duration, events: &mut Vec<Event>) -> Result<(), String> {
        match op {
            Op::CreateSection { name } => {
                let path = self
                    .store
                    .create_dir(None, &name)
                    .map_err(|error| error.to_string())?;
                let id = SectionId(self.model.allocate_id());
                self.model.sections.push(Section {
                    id,
                    name: model::dir_name(&path),
                    path: path.clone(),
                });
                events.extend([Event::Created(ItemRef::Section(id)), Event::TreeChanged]);
                let siblings = self
                    .model
                    .sections
                    .iter()
                    .map(|section| section.path.clone())
                    .collect::<Vec<_>>();
                self.state.reorder(&path, None, &siblings)?;
                self.finish_structure(events)
            }
            Op::CreateGroup { section, name } => {
                let parent = self.parent_path(Parent::Section(section))?;
                let path = self
                    .store
                    .create_dir(Some(&parent), &name)
                    .map_err(|error| error.to_string())?;
                let id = GroupId(self.model.allocate_id());
                self.model.groups.push(Group {
                    id,
                    section,
                    name: model::dir_name(&path),
                    path: path.clone(),
                });
                events.extend([Event::Created(ItemRef::Group(id)), Event::TreeChanged]);
                let siblings = self
                    .model
                    .groups
                    .iter()
                    .filter(|group| group.section == section)
                    .map(|group| group.path.clone())
                    .collect::<Vec<_>>();
                self.state.reorder(&path, None, &siblings)?;
                self.finish_structure(events)
            }
            Op::CreateNote { parent, title } => {
                let path = self.parent_path(parent)?;
                let receipt = self
                    .store
                    .create_file(&path, &title, b"")
                    .map_err(|error| error.to_string())?;
                let id = NoteId(self.model.allocate_id());
                self.model.notes.push(Note {
                    id,
                    parent,
                    title: model::note_title(receipt.path.file_name()),
                    path: receipt.path,
                    subtitle: None,
                    modified: receipt.file.modified,
                    bytes: receipt.file.bytes.len(),
                });
                self.sort_model();
                events.extend([Event::Created(ItemRef::Note(id)), Event::TreeChanged]);
                if let Some(error) = receipt.durability_error {
                    return Err(format!(
                        "Note created, but durability is unconfirmed: {error}"
                    ));
                }
                Ok(())
            }
            Op::EditDraft { note, contents } => self.edit_draft(note, contents, now, events),
            Op::WriteNote { note, contents } => {
                if self
                    .document
                    .as_ref()
                    .is_some_and(|document| document.note == note)
                {
                    self.edit_draft(note, contents, now, events)?;
                    self.flush_active(now, true, events)
                } else {
                    Err("WriteNote requires the active document and its loaded baseline".into())
                }
            }
            Op::OpenNote { note } => self.select(note, now, Navigation::Visit, events),
            Op::SelectParent { parent } => self.select_parent(parent, now, events),
            Op::SelectSection { section } => {
                self.parent_path(Parent::Section(section))?;
                let parent = self.section_parent(section);
                self.select_parent(parent, now, events)
            }
            Op::SetLayout { layout } => {
                self.state.set_layout(&layout)?;
                self.persist_metadata(events)
            }
            Op::SetWidth {
                layout,
                pane,
                width,
            } => {
                self.state.set_width(&layout, &pane, width)?;
                self.persist_metadata(events)
            }
            Op::ResetWidths => {
                for (layout, pane, width) in [
                    ("columns", "sections", 200),
                    ("columns", "groups", 220),
                    ("columns", "notes", 280),
                    ("outline", "sections", 260),
                    ("outline", "notes", 280),
                ] {
                    self.state.set_width(layout, pane, width)?;
                }
                self.persist_metadata(events)
            }
            Op::SetSort { parent, mode } => {
                let path = self.parent_path(parent)?;
                self.state.set_sort(&path, &mode)?;
                self.sort_model();
                events.push(Event::TreeChanged);
                self.persist_metadata(events)
            }
            Op::Flush => {
                self.flush_active(now, true, events)?;
                self.store
                    .retry_durability()
                    .map_err(|error| error.to_string())?;
                Ok(())
            }
            Op::Tick => self.tick(now, events),
            Op::Reconcile => self.reconcile(now, events),
            Op::Rename { target, new_name } => {
                let from = self.item_path(target)?;
                self.barrier(&from, now, events)?;
                self.relocate(target, &from, from.parent().as_ref(), &new_name, events)
            }
            Op::MoveNote { note, to } => {
                let from = self.note_path(note)?;
                let parent = self.parent_path(to)?;
                self.barrier(&from, now, events)?;
                let title = self
                    .model
                    .note(note)
                    .ok_or("Note no longer exists")?
                    .title
                    .clone();
                self.relocate(ItemRef::Note(note), &from, Some(&parent), &title, events)
            }
            Op::MoveGroup { group, to } => {
                let target = ItemRef::Group(group);
                let from = self.item_path(target)?;
                let parent = self.parent_path(Parent::Section(to))?;
                self.barrier(&from, now, events)?;
                let name = self
                    .model
                    .group(group)
                    .ok_or("Group no longer exists")?
                    .name
                    .clone();
                self.relocate(target, &from, Some(&parent), &name, events)
            }
            Op::Delete { target } => {
                let path = self.item_path(target)?;
                self.barrier(&path, now, events)?;
                let validated = self
                    .store
                    .validate_target(&path)
                    .map_err(|error| error.to_string())?;
                self.trash.trash(&validated)?;
                let active_removed = self.document.as_ref().is_some_and(|document| {
                    self.model
                        .note(document.note)
                        .is_some_and(|note| note.path.as_path().starts_with(path.as_path()))
                });
                self.model
                    .sections
                    .retain(|item| !item.path.as_path().starts_with(path.as_path()));
                self.model
                    .groups
                    .retain(|item| !item.path.as_path().starts_with(path.as_path()));
                self.model
                    .notes
                    .retain(|item| !item.path.as_path().starts_with(path.as_path()));
                self.state.remove_paths(&path);
                if active_removed {
                    self.document = None;
                    events.push(Event::Selected(None));
                }
                self.prune_history();
                events.push(Event::TreeChanged);
                self.finish_structure(events)
            }
            Op::Reorder { target, before } => {
                if !self.state.writable() {
                    return Err("Workspace metadata is read-only; ordering was not changed".into());
                }
                let path = self.item_path(target)?;
                let siblings: Vec<_> = match target {
                    ItemRef::Section(_) => self
                        .model
                        .sections
                        .iter()
                        .map(|item| item.path.clone())
                        .collect(),
                    ItemRef::Group(id) => {
                        let parent = self
                            .model
                            .group(id)
                            .ok_or("Group no longer exists")?
                            .section;
                        self.model
                            .groups
                            .iter()
                            .filter(|item| item.section == parent)
                            .map(|item| item.path.clone())
                            .collect()
                    }
                    ItemRef::Note(_) => {
                        return Err("Only sections and groups can be reordered".into())
                    }
                };
                let before = before.map(|item| self.item_path(item)).transpose()?;
                self.state.reorder(&path, before.as_ref(), &siblings)?;
                self.sort_model();
                events.push(Event::TreeChanged);
                self.persist_metadata(events)
            }
            Op::OpenWorkspace { path } => {
                self.flush_active(now, true, events)?;
                self.store
                    .retry_durability()
                    .map_err(|error| error.to_string())?;
                if std::fs::canonicalize(&path).ok().as_deref() == Some(self.store.root()) {
                    return Ok(());
                }
                // Construct the replacement before releasing any old state or lock.
                let mut replacement = Self::load(
                    &path,
                    Box::new(SystemTrash),
                    self.generation.wrapping_add(1),
                )?;
                std::mem::swap(&mut replacement.trash, &mut self.trash);
                *self = replacement;
                events.extend([
                    Event::WorkspaceOpened,
                    Event::TreeChanged,
                    Event::Selected(self.document.as_ref().map(|document| document.note)),
                ]);
                Ok(())
            }
            Op::Back | Op::Forward => {
                let current = self.history_index.ok_or("Navigation history is empty")?;
                let index = if matches!(op, Op::Back) {
                    current.checked_sub(1)
                } else {
                    (current + 1 < self.history.len()).then_some(current + 1)
                }
                .ok_or("No further navigation history")?;
                let note = self.history[index];
                self.select(Some(note), now, Navigation::History(index), events)
            }
        }
    }

    fn edit_draft(
        &mut self,
        note: NoteId,
        contents: String,
        now: Duration,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let document = self
            .document
            .as_mut()
            .filter(|document| document.note == note)
            .ok_or("Only the active document can be edited")?;
        if document.text == contents {
            return Ok(());
        }
        document.text = contents;
        if document.text.as_bytes() == document.baseline && !document.needs_sync {
            document.autosave.mark_clean();
            document.error = None;
        } else {
            document.autosave.edited(now);
        }
        events.push(Event::DraftChanged(note));
        Ok(())
    }

    fn select(
        &mut self,
        note: Option<NoteId>,
        now: Duration,
        navigation: Navigation,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        if matches!(navigation, Navigation::Visit)
            && self.document.as_ref().map(|document| document.note) == note
            && (note.is_some() || self.selected_parent.is_none())
        {
            return Ok(());
        }
        self.flush_active(now, true, events)?;
        let parent = note
            .map(|note| {
                self.model
                    .note(note)
                    .map(|note| note.parent)
                    .ok_or("The note no longer exists")
            })
            .transpose()?;
        let path = note.map(|note| self.note_path(note)).transpose()?;
        self.commit_selection(note, parent, path.as_ref(), navigation, events)
    }

    fn select_parent(
        &mut self,
        parent: Parent,
        now: Duration,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        self.parent_path(parent)?;
        self.flush_active(now, true, events)?;
        let note = self.first_in_parent(parent);
        let path = self.parent_path(parent)?;
        self.commit_selection(note, Some(parent), Some(&path), Navigation::Visit, events)
    }

    fn commit_selection(
        &mut self,
        note: Option<NoteId>,
        parent: Option<Parent>,
        selection: Option<&RelPath>,
        navigation: Navigation,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        if self.document.as_ref().map(|document| document.note) != note {
            let document = note
                .map(|note| {
                    let entry = self
                        .model
                        .note(note)
                        .cloned()
                        .ok_or("The note no longer exists")?;
                    let file = self
                        .store
                        .read(&entry.path)
                        .map_err(|error| error.to_string())?;
                    Document::new(entry, file)
                })
                .transpose()?;
            if let Some(document) = &document {
                if let Some(entry) = self
                    .model
                    .notes
                    .iter_mut()
                    .find(|entry| entry.id == document.note)
                {
                    entry.modified = document.entry.modified;
                    entry.bytes = document.entry.bytes;
                }
                self.sort_model();
            }
            if let Some(outgoing) = &self.document {
                // A scan keeps a live draft authoritative, including after a
                // disk read error. Once closed, only fresh disk bytes may be
                // searched; do this before optional metadata can fail.
                self.index.read_note(&self.store, outgoing.note);
            }
            self.document = document;
        }
        self.selected_parent = parent;
        self.state.set_selection(selection);
        let path = note.map(|note| self.note_path(note)).transpose()?;
        self.state.set_last_open_note(path.as_ref());
        if let Some(note) = note {
            match navigation {
                Navigation::Visit => self.record_history(note),
                Navigation::History(position) => self.history_index = Some(position),
            }
            self.record_recent(note);
        }
        events.push(Event::Selected(note));
        self.persist_metadata(events)
    }

    fn first_in_parent(&self, parent: Parent) -> Option<NoteId> {
        self.model
            .notes
            .iter()
            .find(|note| note.parent == parent)
            .map(|note| note.id)
    }

    fn section_parent(&self, section: SectionId) -> Parent {
        let parent = Parent::Section(section);
        if self.first_in_parent(parent).is_some() {
            return parent;
        }
        self.model
            .groups
            .iter()
            .find(|group| group.section == section)
            .map_or(parent, |group| Parent::Group(group.id))
    }

    fn resolve_selection(&self, path: &RelPath) -> Option<(Parent, Option<NoteId>)> {
        if let Some(note) = self.model.notes.iter().find(|note| note.path == *path) {
            return Some((note.parent, Some(note.id)));
        }
        let parent = self
            .model
            .groups
            .iter()
            .find(|group| group.path == *path)
            .map(|group| Parent::Group(group.id))
            .or_else(|| {
                self.model
                    .sections
                    .iter()
                    .find(|section| section.path == *path)
                    .map(|section| Parent::Section(section.id))
            })?;
        let last = self.state.last_open_note().and_then(|path| {
            self.model
                .notes
                .iter()
                .find(|note| note.parent == parent && note.path == path)
                .map(|note| note.id)
        });
        match last {
            Some(note) => Some((parent, Some(note))),
            None => Some((parent, self.first_in_parent(parent))),
        }
    }

    fn barrier(
        &mut self,
        target: &RelPath,
        now: Duration,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        if self.document.as_ref().is_some_and(|document| {
            self.model
                .note(document.note)
                .is_some_and(|note| note.path.as_path().starts_with(target.as_path()))
        }) {
            self.flush_active(now, true, events)?;
        }
        self.store
            .retry_durability()
            .map_err(|error| error.to_string())
    }

    fn flush_active(
        &mut self,
        now: Duration,
        barrier: bool,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let Some(mut document) = self.document.take() else {
            return Ok(());
        };
        if !document.dirty() {
            self.document = Some(document);
            return Ok(());
        }
        let result = self.save_document(&mut document, now, barrier, events);
        self.document = Some(document);
        match result? {
            true if barrier => Err("External changes were preserved in a conflict copy. The requested transition was cancelled; review the conflict before trying again.".into()),
            _ => Ok(()),
        }
    }

    fn save_document(
        &mut self,
        document: &mut Document,
        now: Duration,
        force: bool,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        let path = document.entry.path.clone();
        let Some(ticket) = document.autosave.begin(now, force) else {
            return if force {
                Err("A document save is already in flight".into())
            } else {
                Ok(false)
            };
        };
        let started = Instant::now();
        let result = (|| match self.store.read(&path) {
            Ok(file)
                if file.bytes != document.baseline && file.bytes != document.text.as_bytes() =>
            {
                self.preserve_conflict(document, &path, events)?;
                Ok(true)
            }
            Ok(file) => {
                let receipt = self
                    .store
                    .write(
                        &path,
                        document.text.as_bytes(),
                        Expected::Bytes(&file.bytes),
                    )
                    .map_err(|error| error.to_string())?;
                document.baseline = receipt.file.bytes.clone();
                document.entry.modified = receipt.file.modified;
                document.entry.bytes = receipt.file.bytes.len();
                self.record_file(document.note, &receipt.file);
                if let Some(error) = receipt.durability_error {
                    document.needs_sync = true;
                    return Err(format!(
                        "Bytes published, but durability is unconfirmed: {error}"
                    ));
                }
                document.needs_sync = false;
                document.recovery = None;
                events.push(Event::NoteSaved {
                    note: document.note,
                    mtime: receipt.file.modified,
                });
                Ok(false)
            }
            Err(error) if error.is_not_found() => {
                self.preserve_conflict(document, &path, events)?;
                Ok(true)
            }
            Err(error) => Err(error.to_string()),
        })();
        self.last_save_micros = started.elapsed().as_micros();
        document.autosave.finish(ticket, result.is_ok(), now);
        document.error = result.as_ref().err().cloned();
        result
    }

    fn preserve_conflict(
        &mut self,
        document: &mut Document,
        path: &RelPath,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or("A note has no valid parent directory")?;
        let bytes = document.text.as_bytes();
        let receipt = match document.recovery.as_ref() {
            Some((copy, baseline)) if baseline == bytes => {
                match self.store.write(copy, bytes, Expected::Bytes(baseline)) {
                    Ok(receipt) => receipt,
                    Err(error) if error.is_conflict() || error.is_not_found() => {
                        self.create_conflict(path, &parent, bytes)?
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            _ => self.create_conflict(path, &parent, bytes)?,
        };
        document.recovery = Some((receipt.path.clone(), receipt.file.bytes.clone()));
        if let Some(error) = receipt.durability_error {
            return Err(format!(
                "Conflict copy published, but durability is unconfirmed; draft retained: {error}"
            ));
        }
        let original = document.note;
        // Re-read after preserving the draft: an external writer may have changed
        // the original again while the copy was being durably published.
        match self.store.read(path) {
            Ok(file) => match String::from_utf8(file.bytes.clone()) {
                Ok(text) => {
                    document.text = text;
                    document.entry.modified = file.modified;
                    document.entry.bytes = file.bytes.len();
                    self.record_file(original, &file);
                    document.baseline = file.bytes;
                    document.needs_sync = false;
                }
                Err(_) => {
                    self.select_recovery_copy(document, &receipt, events)?;
                    events.push(Event::Warning("External note is not UTF-8 and was left untouched. The saved conflict copy is open instead.".into()));
                }
            },
            Err(error) if error.is_not_found() => {
                self.select_recovery_copy(document, &receipt, events)?
            }

            Err(error) => return Err(error.to_string()),
        }
        document.recovery = None;
        document.autosave.mark_clean();
        document.error = None;
        self.refresh_tree(events)?;
        events.push(Event::ExternalChange {
            note: original,
            kind: ExternalChangeKind::Conflict {
                copy: receipt.path.as_path().to_path_buf(),
            },
        });
        self.persist_metadata(events)
    }

    fn select_recovery_copy(
        &mut self,
        document: &mut Document,
        receipt: &WriteReceipt,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        self.refresh_tree(events)?;
        let entry = self
            .model
            .notes
            .iter()
            .find(|note| note.path == receipt.path)
            .cloned()
            .ok_or("The conflict copy was saved but could not be selected; draft retained")?;
        document.note = entry.id;
        document.entry = entry;
        document.baseline = receipt.file.bytes.clone();
        document.needs_sync = false;
        self.state.set_last_open_note(Some(&receipt.path));
        self.state.set_selection(Some(&receipt.path));
        self.record_history(document.note);
        self.record_recent(document.note);
        self.sync_navigation_state();
        events.push(Event::Selected(Some(document.note)));
        Ok(())
    }

    fn create_conflict(
        &mut self,
        path: &RelPath,
        parent: &RelPath,
        bytes: &[u8],
    ) -> Result<WriteReceipt, String> {
        let stamp = chrono::Local::now().format("%Y-%m-%d %H%M");
        let title = model::note_title(path.file_name());
        let mut end = title.len().min(180);
        while !title.is_char_boundary(end) {
            end -= 1;
        }
        let title = format!("{} (conflict {stamp})", &title[..end]);
        self.store
            .create_file(parent, &title, bytes)
            .map_err(|error| error.to_string())
    }

    fn tick(&mut self, now: Duration, events: &mut Vec<Event>) -> Result<(), String> {
        if self
            .document
            .as_ref()
            .is_some_and(|document| document.autosave.due(now))
        {
            self.flush_active(now, false, events)?;
        }
        if self.store.pending_durability()
            && now.saturating_sub(self.last_durability_retry) >= Duration::from_secs(1)
        {
            self.last_durability_retry = now;
            self.store
                .retry_durability()
                .map_err(|error| error.to_string())?;
        }
        let hint = self
            .watcher
            .as_mut()
            .and_then(Watch::take_hint)
            .filter(|hint| hint.generation == self.generation);
        if let Some(warning) = hint.as_ref().and_then(|hint| hint.warning.as_ref()) {
            events.push(Event::Warning(warning.clone()));
        }
        if hint.is_some() || now.saturating_sub(self.last_poll) >= Duration::from_secs(1) {
            self.last_poll = now;
            self.reconcile(now, events)?;
        }
        Ok(())
    }

    fn reconcile(&mut self, now: Duration, events: &mut Vec<Event>) -> Result<(), String> {
        let mut document_result = Ok(());
        if let Some(mut document) = self.document.take() {
            let result = (|| {
                let path = document.entry.path.clone();
                match self.store.read(&path) {
                    Ok(file) if file.bytes == document.baseline => {
                        if !document.dirty() {
                            document.error = None;
                        }
                        Ok(true)
                    }
                    Ok(_) if document.dirty() => {
                        self.save_document(&mut document, now, false, events)?;
                        Ok(true)
                    }
                    Ok(file) => {
                        document.text = String::from_utf8(file.bytes.clone()).map_err(|_| {
                            "External note is not valid UTF-8; it was left untouched"
                        })?;
                        document.baseline = file.bytes.clone();
                        document.entry.modified = file.modified;
                        document.entry.bytes = file.bytes.len();
                        document.error = None;
                        self.record_file(document.note, &file);
                        events.push(Event::ExternalChange {
                            note: document.note,
                            kind: ExternalChangeKind::Reloaded,
                        });
                        Ok(true)
                    }
                    Err(error) if error.is_not_found() && document.dirty() => {
                        self.save_document(&mut document, now, false, events)?;
                        Ok(true)
                    }
                    Err(error) if error.is_not_found() => {
                        self.state.set_last_open_note(None);
                        self.state.set_selection(None);
                        events.extend([
                            Event::ExternalChange {
                                note: document.note,
                                kind: ExternalChangeKind::Deleted,
                            },
                            Event::Selected(None),
                        ]);
                        Ok(false)
                    }
                    Err(error) => Err(error.to_string()),
                }
            })();
            match result {
                Ok(true) => self.document = Some(document),
                Ok(false) => {}
                Err(error) => {
                    document.error = Some(error.clone());
                    self.document = Some(document);
                    document_result = Err(error);
                }
            }
        }
        let refreshed = self.refresh_tree(events);
        document_result.and(refreshed)
    }

    fn relocate(
        &mut self,
        target: ItemRef,
        from: &RelPath,
        parent: Option<&RelPath>,
        name: &str,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let affected: HashSet<_> = self
            .model
            .notes
            .iter()
            .filter(|note| note.path.as_path().starts_with(from.as_path()))
            .map(|note| note.id)
            .collect();
        let receipt = self
            .store
            .relocate(from, parent, name, matches!(target, ItemRef::Note(_)))
            .map_err(|error| error.to_string())?;
        self.model.remap_paths(from, &receipt.path);
        match target {
            ItemRef::Note(id) => {
                let parent = receipt.path.parent();
                let destination = self
                    .model
                    .sections
                    .iter()
                    .find(|section| Some(&section.path) == parent.as_ref())
                    .map(|section| Parent::Section(section.id))
                    .or_else(|| {
                        self.model
                            .groups
                            .iter()
                            .find(|group| Some(&group.path) == parent.as_ref())
                            .map(|group| Parent::Group(group.id))
                    });
                if let Some(parent) = destination {
                    if let Some(note) = self.model.notes.iter_mut().find(|note| note.id == id) {
                        note.parent = parent;
                    }
                }
            }
            ItemRef::Group(id) => {
                let parent = receipt.path.parent();
                if let Some(section) = self
                    .model
                    .sections
                    .iter()
                    .find(|section| Some(&section.path) == parent.as_ref())
                {
                    if let Some(group) = self.model.groups.iter_mut().find(|group| group.id == id) {
                        group.section = section.id;
                    }
                }
            }
            ItemRef::Section(_) => {}
        }
        if let Some(document) = &mut self.document {
            if let Some(entry) = self.model.note(document.note) {
                document.entry = entry.clone();
            }
        }
        self.state.remap_paths(from, &receipt.path);
        if let Some(document) = &self.document {
            if target == ItemRef::Note(document.note) {
                self.selected_parent = Some(document.entry.parent);
                self.state.set_selection(Some(&document.entry.path));
                self.state.set_last_open_note(Some(&document.entry.path));
            }
        }
        for note in self
            .model
            .notes
            .iter()
            .filter(|note| affected.contains(&note.id))
        {
            events.push(Event::NoteMoved {
                note: note.id,
                new_path: note.path.as_path().to_path_buf(),
            });
        }
        events.push(Event::TreeChanged);
        self.refresh_tree(events)?;
        if let Some(error) = receipt.durability_error {
            return Err(format!(
                "Item moved, but durability is unconfirmed: {error}"
            ));
        }
        self.persist_metadata(events)
    }

    fn finish_structure(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        self.refresh_tree(events)?;
        self.persist_metadata(events)
    }

    fn refresh_tree(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        let active = self.document.as_ref().map(|document| {
            self.index.set_content(document.note, &document.text, true);
            (document.note, document.entry.path.clone())
        });
        let scanned = scan::scan_with_contents(&self.store, Some(&self.model), |note, contents| {
            // A scan is not a document transition. In particular, a replacement
            // at the active path must not insert a new ID or replace its draft.
            if !active
                .as_ref()
                .is_some_and(|(id, path)| *id == note.id || *path == note.path)
            {
                self.index.ingest(note, contents);
            }
        });
        let mut scanned = match scanned {
            Ok(scanned) => scanned,
            Err(error) => {
                self.index.scan_failed(active.map(|(id, _)| id));
                return Err(error.to_string());
            }
        };
        // A tree refresh is not a document transition. Keep its recovery identity
        // until document reconciliation has durably preserved or closed it.
        if let Some(document) = &self.document {
            if scanned.note(document.note).is_none() {
                let section = match document.entry.parent {
                    Parent::Section(section) => Some(section),
                    Parent::Group(group) => {
                        if scanned.group(group).is_none() {
                            if let Some(entry) = self.model.group(group) {
                                scanned.groups.push(entry.clone());
                            }
                        }
                        self.model.group(group).map(|group| group.section)
                    }
                };
                if let Some(section) = section {
                    if scanned.section(section).is_none() {
                        if let Some(entry) = self.model.section(section) {
                            scanned.sections.push(entry.clone());
                        }
                    }
                }
                scanned
                    .notes
                    .retain(|note| note.path != document.entry.path);
                scanned.notes.push(document.entry.clone());
                scanned.warnings.push(model::ScanWarning {
                    path: self.store.root().join(document.path().as_path()),
                    message: "Open document retained pending disk reconciliation".into(),
                });
            }
        }
        let sections: HashMap<_, _> = self
            .model
            .sections
            .iter()
            .map(|item| (item.id, (&item.path, &item.name)))
            .collect();
        let groups: HashMap<_, _> = self
            .model
            .groups
            .iter()
            .map(|item| (item.id, (&item.path, item.section)))
            .collect();
        let notes: HashMap<_, _> = self
            .model
            .notes
            .iter()
            .map(|item| {
                (
                    item.id,
                    (&item.path, item.modified, item.bytes, item.parent),
                )
            })
            .collect();
        // A scan's alphabetical/default order differs from persisted manual and
        // title order. Compare identities and metadata, not enumeration order.
        let changed = sections.len() != scanned.sections.len()
            || groups.len() != scanned.groups.len()
            || notes.len() != scanned.notes.len()
            || scanned
                .sections
                .iter()
                .any(|item| sections.get(&item.id) != Some(&(&item.path, &item.name)))
            || scanned
                .groups
                .iter()
                .any(|item| groups.get(&item.id) != Some(&(&item.path, item.section)))
            || scanned.notes.iter().any(|item| {
                notes.get(&item.id) != Some(&(&item.path, item.modified, item.bytes, item.parent))
            });
        self.model = scanned;
        self.sort_model();
        if let Some(document) = &mut self.document {
            if let Some(entry) = self.model.note(document.note) {
                document.entry = entry.clone();
            }
        }
        self.prune_history();
        self.index.sync_metadata(&self.model);
        if let Some(document) = &self.document {
            self.index.set_content(document.note, &document.text, true);
        }
        for (path, reason) in self.index.limitations().take(64) {
            self.model.warnings.push(model::ScanWarning {
                path: self.store.root().join(path.as_path()),
                message: format!("{}: {reason}", path.as_path().display()),
            });
        }
        if changed {
            events.push(Event::TreeChanged);
        }
        Ok(())
    }

    fn sort_model(&mut self) {
        let order: BTreeMap<_, _> = self
            .state
            .order()
            .into_iter()
            .enumerate()
            .map(|(index, path)| (path, index))
            .collect();
        self.model.sections.sort_by(|left, right| {
            order
                .get(&left.path)
                .copied()
                .unwrap_or(usize::MAX)
                .cmp(&order.get(&right.path).copied().unwrap_or(usize::MAX))
                .then_with(|| model::natural_cmp(&left.name, &right.name))
                .then_with(|| left.path.cmp(&right.path))
        });
        self.model.groups.sort_by(|left, right| {
            left.section
                .cmp(&right.section)
                .then_with(|| {
                    order
                        .get(&left.path)
                        .copied()
                        .unwrap_or(usize::MAX)
                        .cmp(&order.get(&right.path).copied().unwrap_or(usize::MAX))
                })
                .then_with(|| model::natural_cmp(&left.name, &right.name))
                .then_with(|| left.path.cmp(&right.path))
        });
        let title_sort: HashMap<_, _> = self
            .model
            .sections
            .iter()
            .map(|section| {
                (
                    Parent::Section(section.id),
                    self.state.sort(&section.path) == "title",
                )
            })
            .chain(self.model.groups.iter().map(|group| {
                (
                    Parent::Group(group.id),
                    self.state.sort(&group.path) == "title",
                )
            }))
            .collect();
        self.model.notes.sort_by(|left, right| {
            left.parent.cmp(&right.parent).then_with(|| {
                let modified = if title_sort.get(&left.parent).copied().unwrap_or(false) {
                    std::cmp::Ordering::Equal
                } else {
                    right.modified.cmp(&left.modified)
                };
                modified
                    .then_with(|| model::natural_cmp(&left.title, &right.title))
                    .then_with(|| left.path.cmp(&right.path))
            })
        });
    }

    fn sync_navigation_state(&mut self) {
        let paths: HashMap<_, _> = self
            .model
            .notes
            .iter()
            .map(|note| (note.id, &note.path))
            .collect();
        let history = self
            .history
            .iter()
            .filter_map(|note| paths.get(note).map(|path| (*path).clone()))
            .collect::<Vec<_>>();
        let recent = self
            .recent
            .iter()
            .filter_map(|note| paths.get(note).map(|path| (*path).clone()))
            .collect::<Vec<_>>();
        self.state.set_history(&history);
        self.state.set_history_position(self.history_index);
        self.state.set_recent(&recent);
    }

    fn persist_metadata(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        self.sync_navigation_state();
        if !self.state.writable() {
            return Ok(());
        }
        self.state.save(&mut self.store).map_err(|error| {
            events.push(Event::Warning(format!(
                "Markdown changes remain applied; workspace metadata was not saved: {error}"
            )));
            error
        })
    }

    fn record_history(&mut self, note: NoteId) {
        if self
            .history_index
            .and_then(|index| self.history.get(index))
            .copied()
            == Some(note)
        {
            return;
        }
        if let Some(index) = self.history_index {
            self.history.truncate(index + 1);
        }
        self.history.push(note);
        if self.history.len() > MAX_NAVIGATION_ENTRIES {
            self.history.remove(0);
        }
        self.history_index = Some(self.history.len() - 1);
    }

    fn record_recent(&mut self, note: NoteId) {
        self.recent.retain(|candidate| *candidate != note);
        self.recent.push(note);
        if self.recent.len() > MAX_NAVIGATION_ENTRIES {
            let forgotten = self.recent.remove(0);
            self.index.forget_opened(forgotten);
        }
        self.index.opened(note);
    }

    fn prune_history(&mut self) {
        let ids: HashSet<_> = self.model.notes.iter().map(|note| note.id).collect();
        let mut through_position = 0usize;
        let mut position = 0usize;
        self.history.retain(|id| {
            let keep = ids.contains(id);
            if keep
                && self
                    .history_index
                    .is_some_and(|current| position <= current)
            {
                through_position += 1;
            }
            position += 1;
            keep
        });
        self.history_index = self
            .history_index
            .filter(|_| !self.history.is_empty())
            .map(|_| through_position.saturating_sub(1));
        if self.history.len() > MAX_NAVIGATION_ENTRIES {
            // Include the selected occurrence even when loading an overlong
            // externally supplied history with its pointer in the early part.
            let start = (self.history.len() - MAX_NAVIGATION_ENTRIES)
                .min(self.history_index.unwrap_or(self.history.len()));
            self.history = self.history[start..]
                .iter()
                .take(MAX_NAVIGATION_ENTRIES)
                .copied()
                .collect();
            self.history_index = self.history_index.map(|position| position - start);
        }
        self.recent.retain(|id| ids.contains(id));
        self.sync_navigation_state();
    }

    fn record_file(&mut self, note: NoteId, file: &DiskFile) {
        if let Some(note) = self
            .model
            .notes
            .iter_mut()
            .find(|candidate| candidate.id == note)
        {
            note.modified = file.modified;
            note.bytes = file.bytes.len();
        }
        self.sort_model();
    }

    fn item_path(&self, item: ItemRef) -> Result<RelPath, String> {
        self.model
            .path(item)
            .cloned()
            .ok_or_else(|| "The item no longer exists".into())
    }
    fn note_path(&self, note: NoteId) -> Result<RelPath, String> {
        self.item_path(ItemRef::Note(note))
    }
    fn parent_path(&self, parent: Parent) -> Result<RelPath, String> {
        self.model
            .parent_path(parent)
            .cloned()
            .ok_or_else(|| "The destination no longer exists".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, FileTimes};

    #[test]
    fn periodic_reconciliation_without_a_watcher_refreshes_exact_closed_note_bytes() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("S")).unwrap();
        let path = root.path().join("S/note.md");
        fs::write(&path, "before").unwrap();
        let mut session = Session::open(root.path()).unwrap();
        session.watcher = None;
        let id = session.model.notes[0].id;
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, "after!").unwrap();
        File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        let events = session.apply_at(Op::Tick, Duration::from_secs(2)).unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged)));
        assert_eq!(session.index.content(id), Some("after!"));
        assert!(session
            .index
            .query(
                "before",
                crate::index::Scope::Workspace,
                crate::index::QueryKind::Search,
                10
            )
            .is_empty());
    }

    #[test]
    fn scan_callbacks_do_not_insert_a_replacement_id_at_a_retained_active_draft_path() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("S")).unwrap();
        fs::write(root.path().join("S/note.md"), "baseline").unwrap();
        let mut session = Session::open(root.path()).unwrap();
        let id = session.model.notes[0].id;
        session
            .apply_at(Op::OpenNote { note: Some(id) }, Duration::ZERO)
            .unwrap();
        session
            .apply_at(
                Op::EditDraft {
                    note: id,
                    contents: "live draft".into(),
                },
                Duration::ZERO,
            )
            .unwrap();
        // Exercise recovery retention when a scan can no longer recover the
        // active identity from its previous model, without changing its draft.
        session.model.notes.clear();
        session.apply_at(Op::Reconcile, Duration::ZERO).unwrap();
        assert_eq!(session.model.notes.len(), 1);
        assert_eq!(session.model.notes[0].id, id);
        assert_eq!(session.index.len(), 1);
        assert_eq!(session.index.content(id), Some("live draft"));
        assert_eq!(session.document().unwrap().text, "live draft");
        assert!(session.document().unwrap().dirty());
        assert_eq!(
            fs::read_to_string(root.path().join("S/note.md")).unwrap(),
            "baseline"
        );
    }
}
