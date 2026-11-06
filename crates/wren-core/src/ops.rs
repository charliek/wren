use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::autosave::Autosave;
use crate::fs_store::{DiskFile, Expected, Fault, FsStore, RelPath, WriteReceipt};
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
    fn new(entry: Note, file: DiskFile) -> Result<Self, String> {
        let text = String::from_utf8(file.bytes.clone())
            .map_err(|_| "The note is not valid UTF-8; it was left untouched".to_string())?;
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

pub struct Session {
    store: FsStore,
    model: Workspace,
    state: WorkspaceState,
    watcher: Option<Watch>,
    generation: u64,
    document: Option<Document>,
    trash: Box<dyn Trash>,
    started: Instant,
    last_poll: Duration,
    last_durability_retry: Duration,
    history: Vec<NoteId>,
    history_index: Option<usize>,
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
        let mut model = scan::scan(&store, None).map_err(|error| error.to_string())?;
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
        let history: Vec<_> = state
            .history()
            .iter()
            .filter_map(|path| {
                model
                    .notes
                    .iter()
                    .find(|note| note.path == *path)
                    .map(|note| note.id)
            })
            .collect();
        let selected = state
            .last_open_note()
            .or_else(|| state.selection())
            .and_then(|path| {
                model
                    .notes
                    .iter()
                    .find(|note| note.path == path)
                    .map(|note| note.id)
            });
        let document = selected.and_then(|note| {
            let entry = model.note(note)?.clone();
            let path = entry.path.clone();
            match store
                .read(&path)
                .map_err(|error| error.to_string())
                .and_then(|file| Document::new(entry, file))
            {
                Ok(document) => Some(document),
                Err(message) => {
                    model.warnings.push(model::ScanWarning {
                        path: store.root().join(path.as_path()),
                        message,
                    });
                    None
                }
            }
        });
        let mut session = Self {
            store,
            model,
            state,
            watcher,
            generation,
            document,
            trash,
            started: Instant::now(),
            last_poll: Duration::ZERO,
            last_durability_retry: Duration::ZERO,
            history,
            history_index: None,
            last_save_micros: 0,
        };
        session.sort_model();
        if let Some(note) = session.document.as_ref().map(|document| document.note) {
            session.history_index = session
                .history
                .iter()
                .rposition(|candidate| *candidate == note);
            if session.history_index.is_none() {
                session.record_history(note);
            }
        }
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
        let result = self.execute(op, now, &mut events);
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
            Op::OpenNote { note } => self.select(note, now, true, events),
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
                self.select(Some(note), now, false, events)?;
                self.history_index = Some(index);
                Ok(())
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
        record: bool,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        if self.document.as_ref().map(|document| document.note) == note {
            return Ok(());
        }
        self.flush_active(now, true, events)?;
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
        self.document = document;
        let path = note.map(|note| self.note_path(note)).transpose()?;
        self.state.set_selection(path.as_ref());
        self.state.set_last_open_note(path.as_ref());
        if record {
            if let Some(note) = note {
                self.record_history(note);
            }
        }
        events.push(Event::Selected(note));
        self.persist_metadata(events)
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
                    return Err(error);
                }
            }
        }
        self.refresh_tree(events)
    }

    fn relocate(
        &mut self,
        target: ItemRef,
        from: &RelPath,
        parent: Option<&RelPath>,
        name: &str,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let affected: Vec<_> = self
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
        if let Some(document) = &mut self.document {
            if let Some(entry) = self.model.note(document.note) {
                document.entry = entry.clone();
            }
        }
        self.state.remap_paths(from, &receipt.path);
        for note in affected {
            if let Some(note) = self.model.note(note) {
                events.push(Event::NoteMoved {
                    note: note.id,
                    new_path: note.path.as_path().to_path_buf(),
                });
            }
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
        let mut scanned =
            scan::scan(&self.store, Some(&self.model)).map_err(|error| error.to_string())?;
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
        let changed = scanned
            .sections
            .iter()
            .map(|item| (&item.path, &item.name))
            .ne(self
                .model
                .sections
                .iter()
                .map(|item| (&item.path, &item.name)))
            || scanned
                .groups
                .iter()
                .map(|item| (&item.path, item.section))
                .ne(self
                    .model
                    .groups
                    .iter()
                    .map(|item| (&item.path, item.section)))
            || scanned
                .notes
                .iter()
                .map(|item| (&item.path, item.modified, item.bytes, item.parent))
                .ne(self
                    .model
                    .notes
                    .iter()
                    .map(|item| (&item.path, item.modified, item.bytes, item.parent)));
        self.model = scanned;
        self.sort_model();
        self.prune_history();
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
        self.model
            .sections
            .sort_by_key(|item| order.get(&item.path).copied().unwrap_or(usize::MAX));
        self.model.groups.sort_by_key(|item| {
            (
                item.section,
                order.get(&item.path).copied().unwrap_or(usize::MAX),
            )
        });
    }

    fn persist_metadata(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        if !self.state.writable() {
            return Ok(());
        }
        let history = self
            .history
            .iter()
            .filter_map(|note| self.model.note(*note).map(|note| note.path.clone()))
            .collect::<Vec<_>>();
        self.state.set_history(&history);
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
        if self.history.len() > 100 {
            self.history.remove(0);
        }
        self.history_index = Some(self.history.len() - 1);
    }

    fn prune_history(&mut self) {
        let active = self
            .document
            .as_ref()
            .map(|document| document.note)
            .or_else(|| {
                self.history_index
                    .and_then(|index| self.history.get(index))
                    .copied()
            });
        self.history.retain(|id| self.model.note(*id).is_some());
        self.history_index =
            active.and_then(|note| self.history.iter().rposition(|id| *id == note));
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
