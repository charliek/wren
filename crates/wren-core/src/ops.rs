use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::autosave::Autosave;
use crate::fs_store::{DiskFile, Expected, Fault, FsStore, RelPath, WriteReceipt};
use crate::index::Index;
use crate::model::{
    self, Group, GroupId, ItemRef, Note, NoteId, Parent, Section, SectionId, Workspace,
};
use crate::reconcile::{self, ReconcileJob, ReconcileResult, Scanned, Tags};
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
        group: GroupId,
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
        to: GroupId,
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
    /// Select exactly this parent; a section parent means no group is selected.
    SelectParent {
        parent: Parent,
    },
    /// Section-row navigation: the first ordered group, otherwise the empty
    /// section. `selected_parent` exposes the resolved parent.
    SelectSection {
        section: SectionId,
    },
    SetWidth {
        pane: String,
        width: u16,
    },
    ResetWidths,
    SetSort {
        group: GroupId,
        mode: String,
    },
    EditDraft {
        note: NoteId,
        contents: String,
    },
    Flush,
    Tick,
    Reconcile,
    /// Merges a background scan from [`Session::take_reconcile_job`]. Results
    /// from another Session, an older job, or a since-changed tree are dropped.
    FinishReconcile(ReconcileResult),
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
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut context = trash::TrashContext::new();
        // On macOS, move items with NSFileManager instead of asking Finder
        // over Apple Events: no Automation permission prompt and no
        // dependency on Finder, at the cost of Finder's "Put Back".
        #[cfg(target_os = "macos")]
        {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            context.set_delete_method(DeleteMethod::NsFileManager);
        }
        context
            .delete(path)
            .map_err(|error| format!("Could not move item to OS trash: {error}"))
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
    background_reconcile: bool,
    reconcile_requested: bool,
    reconcile_in_flight: Option<u64>,
    reconcile_job: Option<ReconcileJob>,
    // Advanced whenever IDs or paths may change; a background result taken
    // from an older revision cannot be merged.
    tree_revision: u64,
    // Notes whose model metadata or index text the owner refreshed while a job
    // was in flight. Their newer owner state wins over the job's snapshot read.
    touched: HashSet<NoteId>,
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
        let mut model = scan::scan_with_contents(store.reader(), None, |note, contents| {
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
            background_reconcile: false,
            reconcile_requested: false,
            reconcile_in_flight: None,
            reconcile_job: None,
            tree_revision: 0,
            touched: HashSet::new(),
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

    /// When enabled, watcher hints and periodic polls during `Op::Tick` prepare
    /// a [`ReconcileJob`] instead of scanning on the owner. The frontend must run
    /// every job from [`Self::take_reconcile_job`] off the owner and return its
    /// result with `Op::FinishReconcile`. `Op::Reconcile` stays synchronous.
    pub fn set_background_reconcile(&mut self, enabled: bool) {
        self.background_reconcile = enabled;
    }

    pub fn take_reconcile_job(&mut self) -> Option<ReconcileJob> {
        self.reconcile_job.take()
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
            self.selected_parent = Some(Parent::Group(document.entry.group));
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
        if matches!(
            op,
            Op::CreateSection { .. }
                | Op::CreateGroup { .. }
                | Op::CreateNote { .. }
                | Op::Rename { .. }
                | Op::Delete { .. }
                | Op::MoveNote { .. }
                | Op::MoveGroup { .. }
        ) {
            self.tree_revision += 1;
        }
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
            Op::CreateNote { group, title } => {
                let path = self.parent_path(Parent::Group(group))?;
                let receipt = self
                    .store
                    .create_file(&path, &title, b"")
                    .map_err(|error| error.to_string())?;
                let id = NoteId(self.model.allocate_id());
                self.model.notes.push(Note {
                    id,
                    group,
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
            Op::SetWidth { pane, width } => {
                self.state.set_width(&pane, width)?;
                self.persist_metadata(events)
            }
            Op::ResetWidths => {
                for (pane, width) in [("sections", 200), ("groups", 220), ("notes", 280)] {
                    self.state.set_width(pane, width)?;
                }
                self.persist_metadata(events)
            }
            Op::SetSort { group, mode } => {
                let path = self.parent_path(Parent::Group(group))?;
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
            Op::FinishReconcile(result) => self.finish_reconcile(&result, now, events),
            Op::Rename { target, new_name } => {
                let from = self.item_path(target)?;
                self.barrier(&from, now, events)?;
                self.relocate(target, &from, from.parent().as_ref(), &new_name, events)
            }
            Op::MoveNote { note, to } => {
                let from = self.note_path(note)?;
                let parent = self.parent_path(Parent::Group(to))?;
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
                // The frontend cannot deliver a result while the Session is
                // away loading; a retained Session must still be able to rescan.
                self.reconcile_in_flight = None;
                self.reconcile_job = None;
                self.reconcile_requested = true;
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
                replacement.background_reconcile = self.background_reconcile;
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
                    .map(|note| Parent::Group(note.group))
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
                self.touch(document.note);
                self.sort_model();
            }
            if let Some(outgoing) = self.document.as_ref().map(|document| document.note) {
                // A scan keeps a live draft authoritative, including after a
                // disk read error. Once closed, only fresh disk bytes may be
                // searched; do this before optional metadata can fail.
                self.index.read_note(&self.store, outgoing);
                self.touch(outgoing);
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
        let Parent::Group(group) = parent else {
            return None;
        };
        self.model
            .notes
            .iter()
            .find(|note| note.group == group)
            .map(|note| note.id)
    }

    /// A section's first group, or the bare section when it has none yet.
    fn section_parent(&self, section: SectionId) -> Parent {
        self.model
            .groups
            .iter()
            .find(|group| group.section == section)
            .map_or(Parent::Section(section), |group| Parent::Group(group.id))
    }

    fn resolve_selection(&self, path: &RelPath) -> Option<(Parent, Option<NoteId>)> {
        if let Some(note) = self.model.notes.iter().find(|note| note.path == *path) {
            return Some((Parent::Group(note.group), Some(note.id)));
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
                    .map(|section| self.section_parent(section.id))
            })?;
        let last = self.state.last_open_note().and_then(|path| {
            self.model
                .notes
                .iter()
                .find(|note| Parent::Group(note.group) == parent && note.path == path)
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
            if self.background_reconcile {
                self.reconcile_requested = true;
            } else {
                self.reconcile(now, events)?;
            }
        }
        if self.background_reconcile
            && self.reconcile_requested
            && self.reconcile_in_flight.is_none()
        {
            self.start_reconcile(now, events)?;
        }
        Ok(())
    }

    fn start_reconcile(&mut self, now: Duration, events: &mut Vec<Event>) -> Result<(), String> {
        self.reconcile_requested = false;
        let reader = match self.store.reader().try_clone() {
            Ok(reader) => reader,
            Err(error) => {
                events.push(Event::Warning(format!(
                    "Background reconciliation unavailable; rescanning on the main thread: {error}"
                )));
                return self.reconcile(now, events);
            }
        };
        let ticket = reconcile::next_ticket();
        self.touched.clear();
        self.reconcile_in_flight = Some(ticket);
        self.reconcile_job = Some(ReconcileJob::new(
            Tags {
                ticket,
                generation: self.generation,
                tree_revision: self.tree_revision,
            },
            reader,
            self.model.scan_baseline(),
            self.index.content_snapshot(),
        ));
        Ok(())
    }

    fn finish_reconcile(
        &mut self,
        result: &ReconcileResult,
        now: Duration,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let tags = result.tags;
        if self.reconcile_in_flight != Some(tags.ticket) || tags.generation != self.generation {
            return Ok(());
        }
        self.reconcile_job = None;
        let Some(outcome) = result.take() else {
            self.reconcile_in_flight = None;
            self.reconcile_requested = true;
            return Err(
                "Background reconciliation stopped before returning a result; it will be retried"
                    .into(),
            );
        };
        // One fresh read keeps conflict protection for the active note exactly
        // as in synchronous reconciliation, independent of the snapshot. It runs
        // while still in flight so a reload marks the note as touched.
        let document_result = self.reconcile_document(now, events);
        self.reconcile_in_flight = None;
        if self.tree_revision != tags.tree_revision {
            // Own structural changes (or a conflict-copy rescan) happened after
            // the snapshot; its IDs and paths cannot be merged safely.
            self.reconcile_requested = true;
            return document_result;
        }
        let merged = match outcome {
            Ok(scanned) => {
                self.merge_scanned(scanned, events);
                Ok(())
            }
            Err(error) => {
                self.index
                    .scan_failed(self.document.as_ref().map(|document| document.note));
                Err(error)
            }
        };
        document_result.and(merged)
    }

    fn merge_scanned(&mut self, scanned: Scanned, events: &mut Vec<Event>) {
        let Scanned {
            mut model,
            changed,
            contents,
        } = scanned;
        if changed {
            for note in &mut model.notes {
                if self.touched.contains(&note.id) {
                    if let Some(current) = self.model.note(note.id) {
                        note.modified = current.modified;
                        note.bytes = current.bytes;
                    }
                }
            }
            self.tree_revision += 1;
            self.install_tree(model, events);
        } else {
            // Identical to the snapshot. With an unchanged tree revision, the
            // owner's model differs from it only by touched metadata and sort
            // order, so it already is the merged result.
            self.model.warnings = model.warnings;
        }
        let active = self
            .document
            .as_ref()
            .map(|document| (document.note, document.entry.path.clone()));
        for (note, path, update) in &contents {
            let skip = self.touched.contains(note)
                || active
                    .as_ref()
                    .is_some_and(|(id, active_path)| id == note || active_path == path);
            if !skip {
                self.index
                    .ingest_scanned(*note, update.as_deref().map_err(String::as_str));
            }
        }
        self.push_limitation_warnings();
    }

    fn reconcile(&mut self, now: Duration, events: &mut Vec<Event>) -> Result<(), String> {
        let document_result = self.reconcile_document(now, events);
        let refreshed = self.refresh_tree(events);
        document_result.and(refreshed)
    }

    fn reconcile_document(&mut self, now: Duration, events: &mut Vec<Event>) -> Result<(), String> {
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
        document_result
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
        let mut moved_group_section = None;
        match target {
            ItemRef::Note(id) => {
                let parent = receipt.path.parent();
                let destination = self
                    .model
                    .groups
                    .iter()
                    .find(|group| Some(&group.path) == parent.as_ref())
                    .map(|group| group.id);
                if let Some(group) = destination {
                    if let Some(note) = self.model.notes.iter_mut().find(|note| note.id == id) {
                        note.group = group;
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
                        if group.section != section.id {
                            moved_group_section = Some(section.id);
                        }
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
        let ordering = moved_group_section.map_or(Ok(()), |section| {
            let siblings: Vec<_> = self
                .model
                .groups
                .iter()
                .filter(|group| group.section == section)
                .map(|group| group.path.clone())
                .collect();
            let existing: HashSet<_> = self.state.order().into_iter().collect();
            // Reorder's missing-path fallback is lexical. Materialize missing
            // destination siblings in their current natural model order first.
            for sibling in &siblings {
                if sibling != &receipt.path && !existing.contains(sibling) {
                    self.state
                        .reorder(sibling, None, std::slice::from_ref(sibling))?;
                }
            }
            self.state.reorder(&receipt.path, None, &siblings)
        });
        if let Some(document) = &self.document {
            if target == ItemRef::Note(document.note) {
                self.selected_parent = Some(Parent::Group(document.entry.group));
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
        self.sort_model();
        self.refresh_tree(events)?;
        if let Some(error) = receipt.durability_error {
            return Err(format!(
                "Item moved, but durability is unconfirmed: {error}"
            ));
        }
        ordering?;
        self.persist_metadata(events)
    }

    fn finish_structure(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        self.refresh_tree(events)?;
        self.persist_metadata(events)
    }

    fn refresh_tree(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        self.tree_revision += 1;
        let active = self.document.as_ref().map(|document| {
            self.index.set_content(document.note, &document.text, true);
            (document.note, document.entry.path.clone())
        });
        let scanned =
            scan::scan_with_contents(self.store.reader(), Some(&self.model), |note, contents| {
                // A scan is not a document transition. In particular, a replacement
                // at the active path must not insert a new ID or replace its draft.
                if !active
                    .as_ref()
                    .is_some_and(|(id, path)| *id == note.id || *path == note.path)
                {
                    self.index.ingest(note, contents);
                }
            });
        let scanned = match scanned {
            Ok(scanned) => scanned,
            Err(error) => {
                self.index.scan_failed(active.map(|(id, _)| id));
                return Err(error.to_string());
            }
        };
        self.install_tree(scanned, events);
        self.push_limitation_warnings();
        Ok(())
    }

    /// Replaces the model with a scan result. Index content for inactive notes
    /// is the caller's responsibility; metadata and the live draft are synced.
    fn install_tree(&mut self, mut scanned: Workspace, events: &mut Vec<Event>) {
        // A tree refresh is not a document transition. Keep its recovery identity
        // until document reconciliation has durably preserved or closed it.
        if let Some(document) = &self.document {
            if scanned.note(document.note).is_none() {
                let group = document.entry.group;
                if scanned.group(group).is_none() {
                    if let Some(entry) = self.model.group(group) {
                        scanned.groups.push(entry.clone());
                    }
                }
                let section = self.model.group(group).map(|group| group.section);
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
        let changed = reconcile::tree_differs(&self.model, &scanned);
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
        if changed {
            events.push(Event::TreeChanged);
        }
    }

    fn push_limitation_warnings(&mut self) {
        for (path, reason) in self.index.limitations().take(64) {
            self.model.warnings.push(model::ScanWarning {
                path: self.store.root().join(path.as_path()),
                message: format!("{}: {reason}", path.as_path().display()),
            });
        }
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
            .groups
            .iter()
            .map(|group| (group.id, self.state.sort(&group.path) == "title"))
            .collect();
        self.model.notes.sort_by(|left, right| {
            left.group.cmp(&right.group).then_with(|| {
                let modified = if title_sort.get(&left.group).copied().unwrap_or(false) {
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
        self.touch(note);
        self.sort_model();
    }

    fn touch(&mut self, note: NoteId) {
        if self.reconcile_in_flight.is_some() {
            self.touched.insert(note);
        }
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

    /// Native acceptance for the real OS trash; it moves one throwaway file
    /// into the current user's Trash, so it only runs on request:
    /// `cargo test -p wren-core --lib system_trash -- --ignored`.
    #[test]
    #[ignore = "moves a file into the real OS trash"]
    fn system_trash_moves_a_file_into_the_os_trash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wren-system-trash-check.md");
        fs::write(&path, b"safe to delete").unwrap();
        SystemTrash.trash(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn periodic_reconciliation_without_a_watcher_refreshes_exact_closed_note_bytes() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("S/H")).unwrap();
        let path = root.path().join("S/H/note.md");
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
        fs::create_dir_all(root.path().join("S/H")).unwrap();
        fs::write(root.path().join("S/H/note.md"), "baseline").unwrap();
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
            fs::read_to_string(root.path().join("S/H/note.md")).unwrap(),
            "baseline"
        );
    }

    fn background_session(root: &std::path::Path) -> Session {
        let mut session = Session::open(root).unwrap();
        // Hints would only add requests; polling makes job creation deterministic.
        session.watcher = None;
        session.set_background_reconcile(true);
        session
    }

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn begin(session: &mut Session, seconds: u64) -> ReconcileJob {
        let events = session.apply_at(Op::Tick, secs(seconds)).unwrap();
        assert!(
            events.is_empty(),
            "a background request must not scan on the owner"
        );
        session
            .take_reconcile_job()
            .expect("a poll prepares one background job")
    }

    fn finish(
        session: &mut Session,
        result: ReconcileResult,
        seconds: u64,
    ) -> Result<Vec<Event>, OpError> {
        session.apply_at(Op::FinishReconcile(result), secs(seconds))
    }

    fn id_at(session: &Session, path: &str) -> NoteId {
        session
            .model
            .notes
            .iter()
            .find(|note| note.path.as_path() == Path::new(path))
            .unwrap_or_else(|| panic!("{path} is in the model"))
            .id
    }

    fn paths(session: &Session) -> Vec<String> {
        let mut paths: Vec<_> = session
            .model
            .notes
            .iter()
            .map(|note| note.path.as_path().display().to_string())
            .collect();
        paths.sort();
        paths
    }

    fn external_changes(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, Event::ExternalChange { .. }))
            .count()
    }

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("S/G")).unwrap();
        fs::create_dir_all(root.path().join("S/H")).unwrap();
        for (path, text) in [
            ("S/G/a.md", "alpha"),
            ("S/G/b.md", "bravo"),
            ("S/H/c.md", "charlie"),
        ] {
            fs::write(root.path().join(path), text).unwrap();
        }
        root
    }

    fn search(session: &Session, text: &str) -> Vec<NoteId> {
        session
            .index
            .query(
                text,
                crate::index::Scope::Workspace,
                crate::index::QueryKind::Search,
                10,
            )
            .into_iter()
            .map(|hit| hit.note)
            .collect()
    }

    struct Quarantine(tempfile::TempDir);

    impl Trash for Quarantine {
        fn trash(&mut self, path: &Path) -> Result<(), String> {
            fs::rename(path, self.0.path().join(path.file_name().unwrap()))
                .map_err(|error| error.to_string())
        }
    }

    #[test]
    fn background_scan_merges_external_creates_deletes_and_same_size_edits() {
        let root = workspace();
        let mut session = background_session(root.path());
        let b = id_at(&session, "S/G/b.md");
        let unchanged_revision = session.index.revision();
        let job = begin(&mut session, 1);
        assert!(session.take_reconcile_job().is_none());
        let events = finish(&mut session, job.run(), 1).unwrap();
        assert!(events.is_empty());
        assert_eq!(session.index.revision(), unchanged_revision);

        let path = root.path().join("S/G/b.md");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, "BRAVO").unwrap();
        File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        fs::remove_file(root.path().join("S/H/c.md")).unwrap();
        fs::write(root.path().join("S/H/d.md"), "delta").unwrap();
        let job = begin(&mut session, 2);
        // A poll while a job is outstanding must not start a second scan.
        assert!(session.apply_at(Op::Tick, secs(4)).unwrap().is_empty());
        assert!(session.take_reconcile_job().is_none());
        let events = finish(&mut session, job.run(), 4).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged)));
        assert_eq!(paths(&session), ["S/G/a.md", "S/G/b.md", "S/H/d.md"]);
        assert_eq!(id_at(&session, "S/G/b.md"), b);
        assert_eq!(session.index.content(b), Some("BRAVO"));
        assert_eq!(search(&session, "delta"), [id_at(&session, "S/H/d.md")]);
        assert!(search(&session, "charlie").is_empty());
    }

    #[test]
    fn own_rename_create_and_delete_during_a_background_scan_are_never_reverted() {
        for run_before_mutation in [true, false] {
            let root = workspace();
            let quarantine = tempfile::tempdir().unwrap();
            let mut session =
                Session::with_trash(root.path(), Box::new(Quarantine(quarantine))).unwrap();
            session.watcher = None;
            session.set_background_reconcile(true);
            let a = id_at(&session, "S/G/a.md");
            let c = id_at(&session, "S/H/c.md");
            let group = session
                .model
                .groups
                .iter()
                .find(|group| group.name == "H")
                .unwrap()
                .id;
            let job = begin(&mut session, 1);
            let early = run_before_mutation.then(|| job.result());
            let late = if run_before_mutation {
                drop(job.run());
                None
            } else {
                Some(job)
            };
            session
                .apply_at(
                    Op::Rename {
                        target: ItemRef::Note(a),
                        new_name: "renamed".into(),
                    },
                    secs(1),
                )
                .unwrap();
            session
                .apply_at(
                    Op::Delete {
                        target: ItemRef::Note(c),
                    },
                    secs(1),
                )
                .unwrap();
            session
                .apply_at(
                    Op::CreateNote {
                        group,
                        title: "created".into(),
                    },
                    secs(1),
                )
                .unwrap();
            let expected = ["S/G/b.md", "S/G/renamed.md", "S/H/created.md"];
            assert_eq!(paths(&session), expected);
            let result = match (early, late) {
                (Some(result), None) => result,
                (None, Some(job)) => job.run(),
                _ => unreachable!(),
            };
            let events = finish(&mut session, result, 1).unwrap();
            assert!(
                events.is_empty(),
                "a stale scan must be dropped: {events:?}"
            );
            assert_eq!(paths(&session), expected);
            assert_eq!(id_at(&session, "S/G/renamed.md"), a);
            assert!(session.reconcile_requested);

            // The dropped scan is retried against the current tree.
            session.apply_at(Op::Tick, secs(1)).unwrap();
            let job = session.take_reconcile_job().unwrap();
            let events = finish(&mut session, job.run(), 1).unwrap();
            assert_eq!(external_changes(&events), 0);
            assert_eq!(paths(&session), expected);
            assert_eq!(id_at(&session, "S/G/renamed.md"), a);
            assert_eq!(search(&session, "alpha"), [a]);
        }
    }

    #[test]
    fn own_saves_during_a_background_scan_are_not_reported_or_overwritten_by_stale_reads() {
        for case in 0..8 {
            let (switch_away, read_after_save, external_create) =
                (case & 1 != 0, case & 2 != 0, case & 4 != 0);
            let root = workspace();
            let mut session = background_session(root.path());
            let a = id_at(&session, "S/G/a.md");
            let b = id_at(&session, "S/G/b.md");
            session
                .apply_at(Op::OpenNote { note: Some(a) }, secs(0))
                .unwrap();
            session
                .apply_at(
                    Op::EditDraft {
                        note: a,
                        contents: "alpha draft saved mid-scan".into(),
                    },
                    // Recent enough that the polling Tick does not autosave it first.
                    Duration::from_millis(900),
                )
                .unwrap();
            if external_create {
                // Forces a full tree merge rather than the unchanged fast path.
                fs::write(root.path().join("S/H/d.md"), "delta").unwrap();
            }
            let job = begin(&mut session, 1);
            // The worker reads either the pre-save bytes and metadata (the
            // snapshot's view) or the owner's newly published save.
            let early = (!read_after_save).then(|| job.result());
            let late = if read_after_save {
                Some(job)
            } else {
                drop(job.run());
                None
            };
            session.apply_at(Op::Flush, secs(1)).unwrap();
            if switch_away {
                session
                    .apply_at(Op::OpenNote { note: Some(b) }, secs(1))
                    .unwrap();
            }
            let result = late.map_or_else(|| early.unwrap(), ReconcileJob::run);
            let saved = session.model.note(a).unwrap().clone();
            let events = finish(&mut session, result, 1).unwrap();
            assert_eq!(external_changes(&events), 0, "{events:?}");
            assert_eq!(
                events
                    .iter()
                    .any(|event| matches!(event, Event::TreeChanged)),
                external_create
            );
            let mut expected = vec!["S/G/a.md", "S/G/b.md", "S/H/c.md"];
            if external_create {
                expected.push("S/H/d.md");
            }
            assert_eq!(paths(&session), expected);
            assert_eq!(
                fs::read_to_string(root.path().join("S/G/a.md")).unwrap(),
                "alpha draft saved mid-scan"
            );
            let current = session.model.note(a).unwrap();
            assert_eq!(
                (current.modified, current.bytes),
                (saved.modified, saved.bytes)
            );
            assert_eq!(session.index.content(a), Some("alpha draft saved mid-scan"));
            assert!(search(&session, "alpha draft").contains(&a));
            let active = session.document().unwrap();
            assert_eq!(active.note, if switch_away { b } else { a });
            assert!(!active.dirty());
        }
    }

    #[test]
    fn a_dirty_draft_and_an_external_change_found_by_a_background_scan_preserve_both() {
        let root = workspace();
        let mut session = background_session(root.path());
        let a = id_at(&session, "S/G/a.md");
        session
            .apply_at(Op::OpenNote { note: Some(a) }, secs(0))
            .unwrap();
        session
            .apply_at(
                Op::EditDraft {
                    note: a,
                    contents: "my draft".into(),
                },
                Duration::from_millis(900),
            )
            .unwrap();
        fs::write(root.path().join("S/G/a.md"), "external writer").unwrap();
        let job = begin(&mut session, 1);
        let events = finish(&mut session, job.run(), 1).unwrap();
        let copy = events
            .iter()
            .find_map(|event| match event {
                Event::ExternalChange {
                    note,
                    kind: ExternalChangeKind::Conflict { copy },
                } if *note == a => Some(copy.clone()),
                _ => None,
            })
            .expect("conflict copy event");
        assert_eq!(
            fs::read_to_string(root.path().join(&copy)).unwrap(),
            "my draft"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("S/G/a.md")).unwrap(),
            "external writer"
        );
        let document = session.document().unwrap();
        assert_eq!(document.note, a);
        assert_eq!(document.text, "external writer");
        assert!(!document.dirty());
        assert!(session
            .model
            .notes
            .iter()
            .any(|note| note.path.as_path() == copy));
    }

    #[test]
    fn results_from_a_replaced_or_reopened_workspace_are_dropped() {
        let root = workspace();
        let other = tempfile::tempdir().unwrap();
        fs::create_dir_all(other.path().join("Other/G")).unwrap();
        fs::write(other.path().join("Other/G/x.md"), "x-ray").unwrap();
        let mut session = background_session(root.path());
        let job = begin(&mut session, 1);
        session
            .apply_at(
                Op::OpenWorkspace {
                    path: other.path().to_path_buf(),
                },
                secs(1),
            )
            .unwrap();
        assert!(session.background_reconcile);
        let events = finish(&mut session, job.run(), 1).unwrap();
        assert!(events.is_empty());
        assert_eq!(paths(&session), ["Other/G/x.md"]);
        assert!(search(&session, "alpha").is_empty());

        // A different Session for the same root starts with the same generation.
        let result = begin(&mut session, 2).run();
        drop(session);
        let mut reopened = background_session(other.path());
        let before = reopened.index.revision();
        let events = finish(&mut reopened, result, 1).unwrap();
        assert!(events.is_empty());
        assert_eq!(reopened.index.revision(), before);
        assert!(reopened.reconcile_in_flight.is_none());
    }

    #[test]
    fn synchronous_sessions_never_prepare_background_jobs() {
        let root = workspace();
        let mut session = Session::open(root.path()).unwrap();
        session.watcher = None;
        session
            .apply_at(
                Op::OpenWorkspace {
                    path: root.path().to_path_buf(),
                },
                secs(0),
            )
            .unwrap();
        fs::write(root.path().join("S/H/e.md"), "echo").unwrap();
        let events = session.apply_at(Op::Tick, secs(1)).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged)));
        assert!(session.take_reconcile_job().is_none());
        assert!(session.reconcile_in_flight.is_none());
    }

    #[test]
    fn an_abandoned_job_reports_an_error_and_is_retried_and_duplicate_results_are_ignored() {
        let root = workspace();
        let mut session = background_session(root.path());
        let job = begin(&mut session, 1);
        let abandoned = job.result();
        drop(job);
        let error = finish(&mut session, abandoned.clone(), 1).unwrap_err();
        assert!(error.message.contains("retried"), "{}", error.message);
        assert!(finish(&mut session, abandoned, 1).unwrap().is_empty());

        fs::write(root.path().join("S/H/e.md"), "echo").unwrap();
        session.apply_at(Op::Tick, secs(1)).unwrap();
        let result = session.take_reconcile_job().unwrap().run();
        assert!(finish(&mut session, result.clone(), 1)
            .unwrap()
            .iter()
            .any(|event| matches!(event, Event::TreeChanged)));
        assert!(finish(&mut session, result, 1).unwrap().is_empty());
        assert_eq!(search(&session, "echo"), [id_at(&session, "S/H/e.md")]);
    }
}
