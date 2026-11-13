use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::Task;
use wren_core::kind::NoteKind;
use wren_core::model::{GroupId, ItemRef, Parent, SectionId, Workspace};
#[cfg(feature = "smoke-test")]
use wren_core::ops::Document;
use wren_core::ops::{Event, ExternalChangeKind, Op, Session, SessionOptions};
use wren_core::reconcile::ReconcileResult;

use super::board::{self, Surface};
use super::commands::{Disposition, Gateway};
use super::{focus_editor, App, Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WarningKind {
    General,
    Document,
    Metadata,
}

struct EffectBefore {
    path: Option<PathBuf>,
    order: Vec<ItemRef>,
}

impl EffectBefore {
    fn capture(session: &Session, op: &Op) -> Self {
        let target = match op {
            Op::Rename { target, .. } | Op::Delete { target } | Op::Reorder { target, .. } => {
                Some(*target)
            }
            Op::MoveNote { note, .. } => Some(ItemRef::Note(*note)),
            Op::MoveGroup { group, .. } => Some(ItemRef::Group(*group)),
            _ => None,
        };
        Self {
            path: target
                .and_then(|target| session.model().path(target))
                .map(|path| path.as_path().to_path_buf()),
            order: if matches!(op, Op::Reorder { .. }) {
                session
                    .model()
                    .sections
                    .iter()
                    .map(|section| ItemRef::Section(section.id))
                    .chain(
                        session
                            .model()
                            .groups
                            .iter()
                            .map(|group| ItemRef::Group(group.id)),
                    )
                    .collect()
            } else {
                Vec::new()
            },
        }
    }

    fn committed(&self, session: &Session, op: &Op, events: &[Event]) -> bool {
        if events.iter().any(|event| {
            matches!(
                event,
                Event::ExternalChange {
                    kind: ExternalChangeKind::Conflict { .. },
                    ..
                }
            )
        }) {
            // Recovery can remove the old identity and select a copy, but the
            // barrier cancels the requested transition before its own effect.
            return false;
        }
        let tree_changed = events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged));
        match op {
            Op::CreateSection { .. } => events
                .iter()
                .any(|event| matches!(event, Event::Created(ItemRef::Section(_)))),
            Op::CreateGroup { .. } => events
                .iter()
                .any(|event| matches!(event, Event::Created(ItemRef::Group(_)))),
            Op::CreateNote { .. } => events
                .iter()
                .any(|event| matches!(event, Event::Created(ItemRef::Note(_)))),
            Op::OpenNote { note } => {
                session.document().map(|document| document.note) == *note
                    && events
                        .iter()
                        .any(|event| matches!(event, Event::Selected(selected) if selected == note))
            }
            Op::SelectParent { .. } | Op::SelectSection { .. } | Op::Back | Op::Forward => events
                .iter()
                .any(|event| matches!(event, Event::Selected(_))),
            Op::Rename { target, .. } => {
                tree_changed
                    && self.path.as_deref().is_some_and(|before| {
                        session
                            .model()
                            .path(*target)
                            .is_some_and(|after| before != after.as_path())
                    })
            }
            Op::MoveNote { note, to } => session.model().note(*note).is_some_and(|note| {
                note.group == *to && self.path.as_deref() != Some(note.path.as_path())
            }),
            Op::MoveGroup { group, to } => session.model().group(*group).is_some_and(|group| {
                group.section == *to && self.path.as_deref() != Some(group.path.as_path())
            }),
            Op::Delete { target } => {
                tree_changed && self.path.is_some() && session.model().path(*target).is_none()
            }
            Op::Reorder { .. } => {
                self.order
                    != session
                        .model()
                        .sections
                        .iter()
                        .map(|section| ItemRef::Section(section.id))
                        .chain(
                            session
                                .model()
                                .groups
                                .iter()
                                .map(|group| ItemRef::Group(group.id)),
                        )
                        .collect::<Vec<_>>()
            }
            Op::SetWidth { pane, .. } => session.state().width(pane).is_some(),
            Op::ResetWidths => true,
            Op::SetSort { group, mode } => session
                .model()
                .group(*group)
                .is_some_and(|group| session.state().sort(&group.path) == mode),
            _ => false,
        }
    }
}

/// Whether `event` presses Tab (`Some(false)`) or Shift+Tab
/// (`Some(true)`) with no other modifier.
pub(super) fn tab(event: &iced::keyboard::Event) -> Option<bool> {
    use iced::keyboard::{key::Named, Event, Key};
    match event {
        Event::KeyPressed {
            key: Key::Named(Named::Tab),
            modifiers,
            ..
        } if !(modifiers.control() || modifiers.alt() || modifiers.logo()) => {
            Some(modifiers.shift())
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogKind {
    CreateSection,
    CreateGroup(SectionId),
    /// A note of the kind the dialog's pills choose, which it opens on.
    CreateNote(GroupId, NoteKind),
    Rename(ItemRef),
    Delete(ItemRef),
}

pub(super) struct Dialog {
    pub kind: DialogKind,
    pub name: String,
    pub error: Option<String>,
}

#[cfg(feature = "smoke-test")]
#[derive(Debug, Clone)]
pub(super) struct DocumentProjection {
    pub note: wren_core::model::NoteId,
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
    pub correlation: u64,
    pub previous_workspace_mode: bool,
    pub previous_workspace_path: String,
    pub model: Option<Workspace>,
    pub note_hits: Vec<wren_core::index::Hit>,
    pub note_sort: String,
    #[cfg(feature = "smoke-test")]
    pub document: Option<DocumentProjection>,
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

/// Opens `path` in `previous`, which keeps its options, or in a new Session
/// with `options`.
fn load_workspace(
    previous: Option<Session>,
    path: PathBuf,
    options: SessionOptions,
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
                .and_then(|trash| Session::with_options(&path, Box::new(trash), options)),
            None => Session::open_with(&path, options),
        };
        #[cfg(not(feature = "smoke-test"))]
        let result = Session::open_with(&path, options);
        match result {
            Ok(mut session) => {
                // Replacement Sessions created by OpenWorkspace inherit this.
                session.set_background_reconcile(true);
                LoadOutcome {
                    session: Some(session),
                    result: Ok(Vec::new()),
                    opened: true,
                }
            }
            Err(error) => LoadOutcome {
                session: None,
                result: Err(error),
                opened: false,
            },
        }
    }
}

struct LoadJob {
    previous: Option<Session>,
    path: PathBuf,
    options: SessionOptions,
    #[cfg(feature = "smoke-test")]
    sandbox: Option<PathBuf>,
}

async fn threaded_load(job: LoadJob) -> Arc<Mutex<Option<LoadOutcome>>> {
    // iced polls immediately-ready task futures on the UI thread. A plain
    // async block around Session::open would still block rendering and input.
    let job = Arc::new(Mutex::new(Some(job)));
    let worker_job = Arc::clone(&job);
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("wren-workspace".into())
        .spawn(move || {
            let job = worker_job
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .expect("one workspace load owner");
            let outcome = load_workspace(
                job.previous,
                job.path,
                job.options,
                #[cfg(feature = "smoke-test")]
                job.sandbox,
            );
            let _ = sender.send(outcome);
        });
    let outcome = match spawned {
        Ok(_) => receiver.await.unwrap_or_else(|_| LoadOutcome {
            session: None,
            result: Err(wren_core::ops::OpError { message: "The workspace loader stopped unexpectedly. Notes were flushed before loading; reopen the workspace to continue.".into(), events: Vec::new() }),
            opened: false,
        }),
        Err(error) => {
            let job = job.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take().expect("unstarted workspace job retains its session");
            LoadOutcome { session: job.previous, result: Err(wren_core::ops::OpError { message: format!("Could not start workspace loading: {error}"), events: Vec::new() }), opened: false }
        }
    };
    Arc::new(Mutex::new(Some(outcome)))
}

impl App {
    // All adapters use this processed gateway. The legacy Result adapter below
    // remains for existing callers, but never decides whether a picker dismisses.
    pub(super) fn process_core(&mut self, op: Op) -> Gateway {
        let passive = matches!(op, Op::Tick | Op::EditDraft { .. } | Op::FinishReconcile(_));
        let previous_processed = self.last_processed.clone();
        if self.loading.is_some() {
            return self.blocked("A workspace is already loading; wait for it to finish");
        }
        // The op flushes, moves or replaces the document, so the board's
        // pending input becomes its draft first. That draft is passive and
        // settles nothing in turn.
        if !passive {
            if let Err(error) = self.settle_board() {
                return self.blocked(error);
            }
        }
        if let Op::OpenWorkspace { path } = op {
            return match self.begin_workspace(path) {
                Ok(task) => {
                    let outcome = self.finish_action(Disposition::AsyncPending, None, task);
                    if let Some(loading) = &mut self.loading {
                        loading.correlation = outcome.processed.correlation;
                    }
                    outcome
                }
                Err(error) => {
                    self.banner = Some(error.clone());
                    self.blocked(error)
                }
            };
        }
        let Some(session) = &mut self.session else {
            return self.blocked("No workspace is open");
        };
        let before = EffectBefore::capture(session, &op);
        // A board this op opens is timed from here.
        #[cfg(feature = "smoke-test")]
        {
            self.board_perf.opening = matches!(op, Op::OpenNote { .. } | Op::Back | Op::Forward)
                .then(std::time::Instant::now);
        }
        let (events, mut error) = match session.apply(op.clone()) {
            Ok(events) => (events, None),
            Err(error) => (error.events, Some(error.message)),
        };
        let mut core_error = error.clone();
        let effect_committed = before.committed(session, &op, &events);
        let committed = error.is_none() || effect_committed;
        // CreateNote persists selection only in its follow-up OpenNote operation.
        let read_only_metadata = error.is_none()
            && effect_committed
            && !matches!(op, Op::CreateNote { .. })
            && !session.state().writable();
        if read_only_metadata {
            error = Some(
                session
                    .state()
                    .warning()
                    .unwrap_or("Workspace metadata is read-only; changes will not be remembered")
                    .to_owned(),
            );
        }
        if matches!(op, Op::Flush | Op::Tick | Op::FinishReconcile(_))
            && error.is_none()
            && self.banner_kind == WarningKind::Document
            && session
                .document()
                .is_none_or(|document| document.error.is_none() && !document.dirty())
        {
            self.banner = self.metadata_warning.clone();
            self.banner_kind = WarningKind::Metadata;
        }
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
                    DialogKind::CreateNote(expected, expected_kind),
                    Op::CreateNote { group, kind, .. },
                    Event::Created(ItemRef::Note(_)),
                ) => expected == group && expected_kind == kind,
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
        #[cfg(feature = "smoke-test")]
        {
            self.board_perf.opening = None;
        }
        let follow_up = match created {
            Some(ItemRef::Note(note)) => Some(Op::OpenNote { note: Some(note) }),
            // A new group is where the next note goes, so select it.
            Some(ItemRef::Group(group)) => Some(Op::SelectParent {
                parent: Parent::Group(group),
            }),
            _ => None,
        };
        if let Some(op) = follow_up {
            let selection = self.process_core(op);
            if core_error.is_none() {
                core_error = selection.core_error;
            }
            if error.is_none() {
                error = selection.processed.warning;
            }
        }
        if let Some(error) = &error {
            self.banner = Some(error.clone());
            self.banner_kind = if read_only_metadata {
                WarningKind::Metadata
            } else if self
                .session
                .as_ref()
                .and_then(Session::document)
                .is_some_and(|document| document.error.is_some())
            {
                WarningKind::Document
            } else if committed {
                WarningKind::Metadata
            } else {
                WarningKind::General
            };
            if self.banner_kind == WarningKind::Metadata {
                self.metadata_warning = Some(error.clone());
            }
        }
        let revision = self
            .session
            .as_ref()
            .map_or(0, |session| session.index().revision());
        // Empty section/group changes do not revise the note index.
        let tree_changed = events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged));
        let palette_before = self
            .palette
            .as_ref()
            .filter(|palette| palette.revision != revision || tree_changed)
            .map(|palette| {
                (
                    palette.active,
                    palette
                        .results
                        .iter()
                        .map(|row| row.key.clone())
                        .collect::<Vec<_>>(),
                )
            });
        let reveal_palette = if let Some((active, keys)) = palette_before {
            self.refresh_palette(false);
            self.palette.as_ref().is_some_and(|palette| {
                palette.active != active
                    || !palette.results.iter().map(|row| &row.key).eq(keys.iter())
            })
        } else {
            false
        };
        if matches!(created, Some(ItemRef::Note(_))) {
            self.column = super::columns::Column::Editor;
        }
        let focus = if created.is_some()
            || (committed
                && matches!(
                    op,
                    Op::OpenNote { note: Some(_) }
                        | Op::SelectParent { .. }
                        | Op::SelectSection { .. }
                        | Op::Back
                        | Op::Forward
                )) {
            self.column_focus()
        } else {
            Task::none()
        };
        let task = if reveal_palette {
            Task::batch([focus, self.reveal_palette_row()])
        } else {
            focus
        };
        #[cfg(feature = "smoke-test")]
        let task = Task::batch([task, self.time_board_open()]);
        let mut outcome = self.finish_action(
            if error.is_some() {
                if committed {
                    Disposition::CommittedWarning
                } else {
                    Disposition::Blocked
                }
            } else {
                Disposition::Completed
            },
            error,
            task,
        );
        outcome.core_error = core_error;
        if passive {
            self.last_processed = previous_processed;
        }
        outcome
    }

    pub(super) fn apply_core(&mut self, op: Op) -> Result<Task<Message>, String> {
        let result = self.process_core(op);
        match result.core_error.or_else(|| {
            (result.processed.disposition == Disposition::Blocked)
                .then_some(result.processed.warning)
                .flatten()
        }) {
            Some(error) => Err(error),
            None => Ok(result.task),
        }
    }

    /// Runs the scan prepared by the last Tick on its own thread. The Session
    /// stays here and merges the returned result as an ordered operation.
    pub(super) fn spawn_reconcile(&mut self, task: Task<Message>) -> Task<Message> {
        let Some(job) = self.session.as_mut().and_then(Session::take_reconcile_job) else {
            return task;
        };
        let result = job.result();
        let (sender, receiver) = iced::futures::channel::oneshot::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("wren-reconcile".into())
            .spawn(move || {
                drop(job.run());
                let _ = sender.send(());
            });
        if let Err(error) = spawned {
            // The unrun job's result releases the Session's slot and retries.
            self.banner = Some(format!(
                "Could not start background reconciliation: {error}"
            ));
        }
        let finished = Task::perform(
            async move {
                // A dropped sender (a panicked worker) still returns the handle.
                let _ = receiver.await;
            },
            move |()| Message::Reconciled(result),
        );
        Task::batch([task, finished])
    }

    pub(super) fn reconciled(&mut self, result: ReconcileResult) -> Task<Message> {
        // A loading Session released its in-flight job when OpenWorkspace began.
        if self.loading.is_some() || self.session.is_none() {
            return Task::none();
        }
        self.process_core(Op::FinishReconcile(result)).task
    }

    /// How every Session of this run lists a workspace: by the settings as
    /// they were at launch.
    pub(super) fn session_options(&self) -> SessionOptions {
        SessionOptions {
            boards: self.preferences.boards,
        }
    }

    fn begin_workspace(&mut self, path: PathBuf) -> Result<Task<Message>, String> {
        if !path.is_absolute() {
            return Err("Workspace path must be absolute".into());
        }
        if self.session.is_some() {
            let _ = self.apply_core(Op::Flush)?;
        }
        self.cancel_gestures();
        let note_hits = self.note_hits();
        let note_sort = self.note_sort().to_owned();
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
            correlation: self.action_serial.wrapping_add(1),
            previous_workspace_mode: self.workspace_mode,
            previous_workspace_path: self.workspace_path.clone(),
            model,
            note_hits,
            note_sort,
            #[cfg(feature = "smoke-test")]
            document,
            generation: self.session.as_ref().map_or(0, Session::generation),
            #[cfg(feature = "smoke-test")]
            pending_durability: self
                .session
                .as_ref()
                .is_some_and(Session::pending_durability),
            #[cfg(feature = "smoke-test")]
            last_save_micros: self.session.as_ref().map_or(0, Session::last_save_micros),
        });
        self.folder_request = None;
        self.folder_correlation = None;
        self.workspace_mode = true;
        self.workspace_path = path.to_string_lossy().into_owned();
        let previous = self.session.take();
        #[cfg(feature = "smoke-test")]
        let sandbox = self.test_sandbox.clone();
        Ok(Task::perform(
            threaded_load(LoadJob {
                previous,
                path,
                options: self.session_options(),
                #[cfg(feature = "smoke-test")]
                sandbox,
            }),
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
        let correlation = self
            .loading
            .as_ref()
            .map_or(self.action_serial, |loading| loading.correlation);
        let accepted = outcome.result.is_ok() || outcome.opened;
        let previous_mode = self
            .loading
            .as_ref()
            .is_none_or(|loading| loading.previous_workspace_mode);
        if !accepted {
            self.workspace_mode = previous_mode;
            self.workspace_path = self.loading.as_ref().map_or_else(String::new, |loading| {
                loading.previous_workspace_path.clone()
            });
        } else if !previous_mode && self.config.is_none() {
            let config = wren_core::config::Config::load();
            self.registry_warning = config.warning().map(str::to_owned);
            self.config = Some(config);
        }
        self.loading = None;
        self.session = outcome.session;
        let (events, mut error) = match outcome.result {
            Ok(events) => (events, None),
            Err(error) => (error.events, Some(error.message)),
        };
        self.process_events(&events);
        if outcome.opened {
            self.conflict_copy = None;
            self.metadata_warning = self
                .session
                .as_ref()
                .and_then(|session| session.state().warning().map(str::to_owned));
            self.banner_kind = WarningKind::General;
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
            self.hydrate_navigation();
        }
        if outcome.opened {
            if let Some(session) = &self.session {
                if session.document().is_none() && session.selected_parent().is_none() {
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
        if outcome.opened {
            self.reveal_selection();
        }
        if let Some(session) = &self.session {
            self.workspace_path = session.model().root.to_string_lossy().into_owned();
        }
        let name = self.registration_name.take();
        if !accepted {
            self.refresh_palette(false);
            if let Some(palette) = &mut self.palette {
                palette.error = error.clone();
            }
        }
        if accepted {
            self.palette = None;
            self.dialog = None;
            let registry_error = self.register_active_workspace(name.as_deref());
            if error.is_none() {
                error = registry_error;
            }
        }
        if let Some(error) = &error {
            self.banner = Some(error.clone());
        }
        if self.flush_on_load && !self.quit_pending && self.session.is_some() {
            let _ = self.process_core(Op::Flush);
        }
        self.flush_on_load = false;
        self.last_processed = Some(super::commands::Processed {
            correlation,
            disposition: if error.is_some() {
                if accepted {
                    Disposition::CommittedWarning
                } else {
                    Disposition::Blocked
                }
            } else {
                Disposition::Completed
            },
            warning: error.clone(),
        });
        #[cfg(feature = "smoke-test")]
        if let Some(request) = self.workspace_reply.take() {
            request.respond(
                error
                    .clone()
                    .map_or_else(|| Ok(self.session_snapshot()), Err),
            );
        }
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
        if accepted {
            focus_editor()
        } else if self.palette.is_some() {
            iced::widget::operation::focus("palette-query")
        } else {
            Task::none()
        }
    }

    fn process_events(&mut self, events: &[Event]) -> Option<ItemRef> {
        let mut created = None;
        for event in events {
            match event {
                Event::Created(item @ (ItemRef::Note(_) | ItemRef::Group(_))) => {
                    created = Some(*item);
                }
                Event::Warning(message) => self.banner = Some(message.clone()),
                Event::NoteSaved { note, .. } => {
                    let document = self.session.as_ref().and_then(Session::document);
                    let saved = document.filter(|document| document.note == *note);
                    if let (Some(view), Some(document)) = (&mut self.board, saved) {
                        view.saved(&document.text);
                    }
                }
                Event::ExternalChange { kind, .. } => match kind {
                    ExternalChangeKind::Reloaded => {}
                    ExternalChangeKind::Conflict { copy } => {
                        self.conflict_copy = wren_core::fs_store::RelPath::new(copy)
                            .ok()
                            .map(|path| (self.workspace_generation(), path));
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

    /// Brings the surface in step with the Session's open document: which
    /// of the editor and the board viewer shows it, and with what text. It
    /// decides again only when another note is open or the Session's text is
    /// not what the surface last took, so a surface's own draft changes
    /// nothing here.
    pub(super) fn sync_editor(&mut self, workspace_changed: bool) {
        let Some(session) = &self.session else {
            return;
        };
        let note = session.document().map(|document| document.note);
        let text = session
            .document()
            .map_or("", |document| document.text.as_str());
        let board_note = note.filter(|note| {
            session
                .model()
                .note(*note)
                .is_some_and(|entry| entry.kind == NoteKind::Board)
        });
        let identity_changed = workspace_changed || self.editor_note != note;
        let kind_changed = board_note.is_some() != (self.surface() != Surface::Text);
        let taken = match &self.board {
            Some(view) => view.synced() == text,
            None => self.editor.text() == text,
        };
        if identity_changed || kind_changed || !taken {
            let mut error = None;
            // The view of the same board takes the new text in place and
            // keeps its viewport.
            let reloaded = !identity_changed
                && board_note.is_some()
                && self
                    .board
                    .as_mut()
                    .is_some_and(|view| match view.reload(text) {
                        Ok(()) => {
                            self.board_revision += 1;
                            true
                        }
                        Err(reason) => {
                            error = Some(reason);
                            false
                        }
                    });
            if !reloaded {
                if let (Some(view), Some(previous)) = (self.board.take(), self.editor_note) {
                    board::close(&view, previous, &mut self.board_viewports);
                }
                if workspace_changed {
                    self.board_viewports.clear();
                }
                if let (Some(note), None) = (board_note, &error) {
                    #[cfg(feature = "smoke-test")]
                    let started = self
                        .board_perf
                        .opening
                        .unwrap_or_else(std::time::Instant::now);
                    let palette = crate::theme::board_palette(self.definition);
                    match board::open(text, note, palette, &self.board_viewports) {
                        Ok(mut view) => {
                            view.set_key_policy(super::preferences::board_policy(
                                &self.preferences.keymap,
                            ));
                            self.board = Some(view);
                            self.board_revision = 0;
                            #[cfg(feature = "smoke-test")]
                            self.board_perf.opened(started);
                        }
                        Err(reason) => error = Some(reason),
                    }
                }
                let shown = if self.board.is_some() { "" } else { text };
                self.editor.replace_text(shown, !identity_changed);
                self.words = shown.split_whitespace().count();
                self.words_due = None;
            }
            self.board_error = error;
            self.edit_revision += 1;
            self.editor_note = note;
            if self.board.is_some() {
                self.find_open = false;
            }
        }
        self.selected_note = note;
        if let Some(entry) = note.and_then(|note| session.model().note(note)) {
            self.selected_group = Some(entry.group);
            self.selected_section = session
                .model()
                .group(entry.group)
                .map(|group| group.section);
        } else {
            self.selected_section = session
                .selected_parent()
                .and_then(|parent| match parent {
                    Parent::Section(section) => Some(section),
                    Parent::Group(group) => session.model().group(group).map(|group| group.section),
                })
                .or(self
                    .selected_section
                    .filter(|id| session.model().section(*id).is_some()))
                .or_else(|| session.model().sections.first().map(|section| section.id));
            self.selected_group = match session.selected_parent() {
                Some(Parent::Group(group)) => Some(group),
                Some(Parent::Section(_)) => None,
                None => self.selected_group.filter(|id| {
                    session
                        .model()
                        .group(*id)
                        .is_some_and(|group| Some(group.section) == self.selected_section)
                }),
            };
        }
        self.apply_markdown_assist();
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

    #[cfg(test)]
    pub(super) fn select_parent(&mut self, parent: Parent) -> Result<(), String> {
        let outcome = self.dispatch(super::commands::Command::SelectParent(parent));
        outcome.processed.warning.map_or(Ok(()), Err)
    }

    pub(super) fn ensure_editor_ready(&self) -> Result<(), String> {
        if self.loading.is_some() {
            return Err("The editor is disabled while a workspace is loading".into());
        }
        if self.workspace_mode && self.session.as_ref().and_then(Session::document).is_none() {
            return Err("Select a note before editing".into());
        }
        // Only the visible surface may write: the editor behind a board is
        // empty, and its text must never become the board's draft.
        if self.surface() == Surface::Board {
            return Err("The open note is a board; the text editor is not showing it".into());
        }
        Ok(())
    }

    pub(super) fn window_unfocused(&mut self) -> Result<(), String> {
        self.cancel_file_drag();
        self.settle_resize();
        self.cancel_gestures();
        self.editor.break_group();
        // Whatever the board's settling came to, the draft the Session
        // already holds is still flushed.
        let settled = self.put_board_away();
        if self.loading.is_some() {
            self.flush_on_load = true;
        } else if self.session.is_some() {
            let _ = self.apply_core(Op::Flush)?;
        }
        settled
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
        // A draft that did not reach the Session is on the banner, and the
        // dialog stays shut.
        if self.put_board_away().is_err() {
            return Task::none();
        }
        self.editor.break_group();
        self.settle_resize();
        self.cancel_gestures();
        self.palette = None;
        let name = match kind {
            DialogKind::CreateNote(..) => wren_core::ops::dated_title(),
            DialogKind::Rename(target) => self.item_name(target).unwrap_or_default().into(),
            _ => String::new(),
        };
        let focus_name = matches!(
            kind,
            DialogKind::CreateSection
                | DialogKind::CreateGroup(_)
                | DialogKind::CreateNote(..)
                | DialogKind::Rename(_)
        );
        self.dialog = Some(Dialog {
            kind,
            name,
            error: None,
        });
        if focus_name {
            iced::widget::operation::focus("item-name")
        } else {
            Task::none()
        }
    }

    /// The New note dialog's kind, when it offers the choice: only with
    /// the `boards` setting on (§5.4).
    fn note_kind_mut(&mut self) -> Option<&mut NoteKind> {
        if !self.preferences.boards {
            return None;
        }
        match &mut self.dialog {
            Some(Dialog {
                kind: DialogKind::CreateNote(_, kind),
                ..
            }) => Some(kind),
            _ => None,
        }
    }

    /// A kind pill was clicked. The click took the keyboard from the name
    /// field, so hand it back.
    pub(super) fn choose_note_kind(&mut self, chosen: NoteKind) -> Task<Message> {
        let Some(kind) = self.note_kind_mut() else {
            return Task::none();
        };
        *kind = chosen;
        iced::widget::operation::focus("item-name")
    }

    /// Tab or Shift+Tab in the New note dialog: the next or previous kind,
    /// wrapping. Anywhere else such a Tab does nothing.
    pub(super) fn step_note_kind(&mut self, reverse: bool) {
        let Some(kind) = self.note_kind_mut() else {
            return;
        };
        let kinds = NoteKind::ALL;
        let at = kinds.iter().position(|each| each == kind).unwrap_or(0);
        let step = if reverse { kinds.len() - 1 } else { 1 };
        *kind = kinds[(at + step) % kinds.len()];
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
            DialogKind::CreateNote(group, kind) => Op::CreateNote {
                group,
                title: name,
                kind,
            },
            DialogKind::Rename(target) => Op::Rename {
                target,
                new_name: name,
            },
            DialogKind::Delete(target) => Op::Delete { target },
        };
        let outcome = self.process_core(op);
        if outcome.processed.disposition.committed() {
            self.dialog = None;
            self.reveal_selection();
            outcome.task.chain(self.column_focus())
        } else {
            if let Some(dialog) = &mut self.dialog {
                dialog.error = outcome.processed.warning;
            }
            Task::none()
        }
    }
}

#[cfg(test)]
mod tests {
    use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position};
    use wren_core::fs_store::Fault;

    use super::*;
    use crate::cli::Options;

    fn fixture() -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        std::fs::create_dir_all(directory.path().join("Inbox/General")).unwrap();
        std::fs::write(
            directory.path().join("Inbox/General/Note.md"),
            "first\nsecond",
        )
        .unwrap();
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        app.sync_editor(true);
        let note = app.model().unwrap().notes[0].id;
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        (directory, app)
    }

    fn append(app: &mut App, text: &str) {
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        ))));
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
            Edit::Paste(Arc::new(text.into())),
        ))));
    }

    fn returning(app: &mut App, outcome: LoadOutcome) -> Arc<Mutex<Option<LoadOutcome>>> {
        app.workspace_mode = true;
        app.load_generation += 1;
        app.loading = Some(Loading {
            tag: app.load_generation,
            correlation: 0,
            previous_workspace_mode: app.workspace_mode,
            previous_workspace_path: app.workspace_path.clone(),
            model: None,
            note_hits: Vec::new(),
            note_sort: "modified".into(),
            #[cfg(feature = "smoke-test")]
            document: None,
            generation: 0,
            #[cfg(feature = "smoke-test")]
            pending_durability: false,
            #[cfg(feature = "smoke-test")]
            last_save_micros: 0,
        });
        Arc::new(Mutex::new(Some(outcome)))
    }

    fn load(previous: Option<Session>, path: PathBuf) -> LoadOutcome {
        load_with(previous, path, SessionOptions::default())
    }

    fn load_with(previous: Option<Session>, path: PathBuf, options: SessionOptions) -> LoadOutcome {
        load_workspace(
            previous,
            path,
            options,
            #[cfg(feature = "smoke-test")]
            None,
        )
    }

    #[test]
    fn a_workspace_whose_open_note_is_a_board_loads_onto_the_board_surface() {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("Inbox/General");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("Note.md"), "note").unwrap();
        std::fs::write(group.join("Flow.wren-board"), wren_board::EMPTY).unwrap();
        let options = SessionOptions { boards: true };
        let mut earlier = Session::open_with(directory.path(), options).unwrap();
        let of_kind = |session: &Session, kind| {
            let notes = &session.model().notes;
            notes.iter().find(|note| note.kind == kind).unwrap().id
        };
        for kind in [NoteKind::Markdown, NoteKind::Board] {
            let note = Some(of_kind(&earlier, kind));
            earlier.apply(Op::OpenNote { note }).unwrap();
        }
        drop(earlier);

        let (mut app, _) = App::new(Options::default());
        app.preferences.boards = true;
        let slot = returning(&mut app, load_with(None, directory.path().into(), options));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.surface(), Surface::Board);
        assert_eq!(app.editor.text(), "");
        let view = app.board.as_ref().unwrap();
        assert_eq!(view.synced(), wren_board::EMPTY);
        assert!(view.fit_pending());
        // The Session holds the workspace's writer lock.
        drop(app);

        // With the setting off the same workspace opens the Markdown note
        // that was open before the board.
        let (mut app, _) = App::new(Options::default());
        let slot = returning(&mut app, load(None, directory.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.surface(), Surface::Text);
        assert_eq!(app.editor.text(), "note");
    }

    #[test]
    fn a_workspace_lists_boards_only_when_the_setting_was_on_at_launch() {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("Inbox/General");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("Note.md"), "note").unwrap();
        std::fs::write(group.join("Flow.wren-board"), wren_board::EMPTY).unwrap();
        let kinds = |session: &Session| -> Vec<(String, NoteKind)> {
            let mut notes: Vec<_> = session
                .model()
                .notes
                .iter()
                .map(|note| (note.title.clone(), note.kind))
                .collect();
            notes.sort();
            notes
        };

        let off = load(None, directory.path().into()).session.unwrap();
        assert_eq!(kinds(&off), [("Note".to_owned(), NoteKind::Markdown)]);
        assert!(off.model().warnings.is_empty());
        // A workspace opened from that Session keeps its options.
        let off = load_with(
            Some(off),
            directory.path().into(),
            SessionOptions { boards: true },
        )
        .session
        .unwrap();
        assert_eq!(kinds(&off), [("Note".to_owned(), NoteKind::Markdown)]);
        drop(off);

        let on = load_with(
            None,
            directory.path().into(),
            SessionOptions { boards: true },
        )
        .session
        .unwrap();
        assert_eq!(
            kinds(&on),
            [
                ("Flow".to_owned(), NoteKind::Board),
                ("Note".to_owned(), NoteKind::Markdown)
            ]
        );

        // The app passes the launch value, not the file's later one.
        let (mut app, _) = App::new(Options::default());
        app.preferences.settings.boards = true;
        assert_eq!(app.session_options(), SessionOptions { boards: false });
        app.preferences.boards = true;
        app.preferences.settings.boards = false;
        assert_eq!(app.session_options(), SessionOptions { boards: true });
    }

    #[test]
    fn legacy_core_adapter_distinguishes_read_only_metadata_notices_from_core_errors() {
        let (directory, mut app) = fixture();
        drop(app.session.take());
        let metadata = directory.path().join(".wren/workspace.toml");
        let original = b"layout = [malformed optional metadata\n";
        std::fs::write(&metadata, original).unwrap();
        app.session = Some(Session::open(directory.path()).unwrap());
        app.sync_editor(true);
        let note = app.model().unwrap().notes[0].id;
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        assert_eq!(
            app.last_processed.as_ref().unwrap().disposition,
            Disposition::CommittedWarning
        );
        assert!(app.last_processed.as_ref().unwrap().warning.is_some());
        assert_eq!(app.selected_note, Some(note));
        let _ = app
            .apply_core(Op::CreateSection {
                name: "Still writable".into(),
            })
            .unwrap();
        assert!(directory.path().join("Still writable").is_dir());
        append(&mut app, " retained draft");
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        assert!(app.apply_core(Op::Flush).is_err());
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_eq!(std::fs::read(&metadata).unwrap(), original);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/General/Note.md")).unwrap(),
            "first\nsecond retained draft"
        );
        assert!(app.metadata_warning.is_some());
    }

    #[test]
    fn an_intent_reaches_the_core_draft_and_takes_the_editor_column() {
        let (directory, mut app) = fixture();
        append(&mut app, "\n- item");
        app.column = super::super::columns::Column::Notes;
        let revision = app.edit_revision;
        let _ = app.update(Message::Editor(wren_editor::Message::Intent(
            wren_editor::Intent::Enter,
        )));
        assert_eq!(app.column, super::super::columns::Column::Editor);
        assert_eq!(app.edit_revision, revision + 1);
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            "first\nsecond\n- item\n- "
        );
        // A motion changes no text, so nothing is drafted.
        let _ = app.update(Message::Editor(wren_editor::Message::Intent(
            wren_editor::Intent::Home { extend: false },
        )));
        assert_eq!(app.edit_revision, revision + 1);
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/General/Note.md")).unwrap(),
            "first\nsecond\n- item\n- "
        );
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
            std::fs::read_to_string(directory.path().join("Inbox/General/Note.md")).unwrap(),
            "first\nsecond"
        );
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            "first\nsecond"
        );
        let _ = app.update(Message::Editor(wren_editor::Message::Redo));
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            app.editor.text()
        );
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/General/Note.md")).unwrap(),
            app.editor.text()
        );
    }

    #[test]
    fn rename_move_and_theme_changes_preserve_live_history_and_cursor() {
        let (directory, mut app) = fixture();
        // Created on disk: a CreateGroup op would select the new group and close the note.
        std::fs::create_dir_all(directory.path().join("Archive/Shelf")).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        let destination = app
            .model()
            .unwrap()
            .sections
            .iter()
            .find(|section| section.name == "Archive")
            .unwrap()
            .id;
        let shelf = app
            .model()
            .unwrap()
            .groups
            .iter()
            .find(|group| group.name == "Shelf")
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
        let _ = app.apply_core(Op::MoveNote { note, to: shelf }).unwrap();
        let _ = app.update(Message::ChooseTheme(Some(wren_core::theme::LIGHT)));
        assert_eq!(app.editor.cursor(), cursor);
        assert!(app.editor.can_undo());
        assert_eq!(app.selected_note, Some(note));
        assert_eq!(app.selected_section, Some(destination));
        assert_eq!(app.selected_group, Some(shelf));
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), "first\nsecond");
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            app.editor.text()
        );
        let _ = app.update(Message::Editor(wren_editor::Message::Redo));
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
            std::fs::read_to_string(directory.path().join("Inbox/General/Note.md")).unwrap(),
            "first\nsecond"
        );
        app.window_unfocused().unwrap();
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Inbox/General/Note.md")).unwrap(),
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
        assert!(directory.path().join("Inbox/General/Published.md").exists());
        assert!(!directory.path().join("Inbox/General/Note.md").exists());
        assert_eq!(
            app.session
                .as_ref()
                .unwrap()
                .document()
                .unwrap()
                .path()
                .as_path(),
            std::path::Path::new("Inbox/General/Published.md")
        );
        assert_eq!(app.editor.cursor(), cursor);
        assert!(app.editor.can_undo());
    }

    #[test]
    fn committed_create_error_still_projects_and_selects_the_new_note() {
        let (_directory, mut app) = fixture();
        let group = app.selected_group.unwrap();
        app.session
            .as_mut()
            .unwrap()
            .fail_next(Fault::DirectorySync);
        assert!(app
            .apply_core(Op::CreateNote {
                group,
                title: "Created".into(),
                kind: NoteKind::Markdown,
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
            let group = app.selected_group.unwrap();
            let dialog = match kind {
                0 => DialogKind::CreateSection,
                1 => DialogKind::CreateGroup(section),
                _ => DialogKind::CreateNote(group, NoteKind::Markdown),
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
                _ => "Inbox/General/Created.md",
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
                _ => DialogKind::CreateNote(app.selected_group.unwrap(), NoteKind::Markdown),
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
                _ => "Inbox/General/Created.md",
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
            directory.path().join("Inbox/General/Note.md"),
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
        std::fs::write(
            directory.path().join("Inbox/General/Note.md"),
            "external original",
        )
        .unwrap();
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
        std::fs::remove_file(directory.path().join("Inbox/General/Note.md")).unwrap();
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
    fn conflict_copy_link_uses_the_relative_path_and_survives_reloads_until_banner_dismissal() {
        for removed in [false, true] {
            let (directory, mut app) = fixture();
            append(&mut app, " local recovery");
            let original = directory.path().join("Inbox/General/Note.md");
            if removed {
                std::fs::remove_file(&original).unwrap();
            } else {
                std::fs::write(&original, "external bytes").unwrap();
            }
            let result = app.process_core(Op::Flush);
            assert_eq!(result.processed.disposition, Disposition::Blocked);
            let (generation, copy) = app
                .conflict_copy
                .clone()
                .expect("saved conflict has a link");
            assert_eq!(generation, app.workspace_generation());
            assert!(!copy.as_path().is_absolute());
            assert_eq!(
                copy.parent().unwrap().as_path(),
                std::path::Path::new("Inbox/General")
            );
            assert_eq!(
                std::fs::read_to_string(directory.path().join(copy.as_path())).unwrap(),
                "first\nsecond local recovery"
            );
            assert!(app
                .model()
                .unwrap()
                .notes
                .iter()
                .any(|note| note.path == copy));
            let _ = app.process_core(Op::Reconcile);
            let _ = app.update(Message::Tick);
            assert_eq!(app.conflict_copy, Some((generation, copy)));
            assert!(app.banner.is_some());
            let _ = app.update(Message::DismissBanner);
            assert!(app.conflict_copy.is_none());
        }
    }

    #[test]
    fn a_replacement_workspace_clears_the_old_conflict_link_but_a_failed_open_does_not() {
        let (directory, mut app) = fixture();
        append(&mut app, " local recovery");
        std::fs::write(
            directory.path().join("Inbox/General/Note.md"),
            "external bytes",
        )
        .unwrap();
        let _ = app.process_core(Op::Flush);
        let link = app
            .conflict_copy
            .clone()
            .expect("saved conflict has a link");
        let previous = app.session.take();
        let slot = returning(&mut app, load(previous, directory.path().join("missing")));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.conflict_copy, Some(link));
        let next = crate::test_support::tempdir();
        std::fs::create_dir_all(next.path().join("Inbox/General")).unwrap();
        std::fs::write(next.path().join("Inbox/General/Other.md"), "new workspace").unwrap();
        let previous = app.session.take();
        let slot = returning(&mut app, load(previous, next.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.model().unwrap().root, next.path());
        assert!(app.conflict_copy.is_none());
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
        let next = crate::test_support::tempdir();
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
    fn same_root_open_registers_after_success_without_resetting_navigation_or_generation() {
        let (directory, mut app) = fixture();
        let configdir = crate::test_support::tempdir();
        app.config = Some(wren_core::config::Config::load_from(configdir.path()));
        app.filters.sections = "Inbox".into();
        app.focus = true;
        app.widths.set(crate::panes::Pane::Sections, 330.0);
        let generation = app.workspace_generation();
        let previous = app.session.take();
        let outcome = load(previous, directory.path().into());
        assert!(!outcome.opened);
        assert!(outcome.result.is_ok());
        let slot = returning(&mut app, outcome);
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.workspace_generation(), generation);
        assert_eq!(app.filters.sections, "Inbox");
        assert!(app.focus);
        assert_eq!(app.widths.get(crate::panes::Pane::Sections), 330.0);
        assert_eq!(
            app.config.as_ref().unwrap().active(),
            Some(directory.path())
        );
        assert_eq!(app.config.as_ref().unwrap().workspaces().len(), 1);
    }

    #[test]
    fn failed_open_retains_workspace_picker_query_and_does_not_advance_registry() {
        let (directory, mut app) = fixture();
        let configdir = crate::test_support::tempdir();
        let mut config = wren_core::config::Config::load_from(configdir.path());
        config.add(directory.path(), None).unwrap();
        config.set_active(directory.path()).unwrap();
        app.config = Some(config);
        let _ = app.open_palette(super::super::palette::Mode::Workspaces);
        let _ = app.palette_query("retryable query".into());
        app.filters.notes = "Note".into();
        let previous = app.session.take();
        let outcome = load(previous, directory.path().join("missing"));
        let slot = returning(&mut app, outcome);
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.palette.as_ref().unwrap().query, "retryable query");
        assert!(app.palette.as_ref().unwrap().error.is_some());
        assert_eq!(app.filters.notes, "Note");
        assert_eq!(
            app.config.as_ref().unwrap().active(),
            Some(directory.path())
        );
        assert_eq!(app.model().unwrap().root, directory.path());
        assert_eq!(
            app.last_processed.as_ref().unwrap().disposition,
            Disposition::Blocked
        );
    }

    #[test]
    fn missing_startup_root_leaves_real_onboarding_and_switch_commands_usable() {
        let configdir = crate::test_support::tempdir();
        let root = crate::test_support::tempdir();
        let missing = root.path().to_path_buf();
        let mut config = wren_core::config::Config::load_from(configdir.path());
        config.add(&missing, Some("Missing")).unwrap();
        config.set_active(&missing).unwrap();
        drop(root);
        let (mut app, _) = App::with_config(Options::default(), Some(config), None);
        assert!(app.loading.is_none());
        assert!(app.workspace_mode);
        assert!(app.session.is_none());
        assert!(app.editor.text().is_empty());
        assert!(app
            .registry_warning
            .as_deref()
            .unwrap()
            .contains("unavailable"));
        assert_eq!(
            app.config.as_ref().unwrap().active(),
            Some(missing.as_path())
        );
        assert!(app.command_enabled(super::super::commands::CommandId::AddWorkspace));
        let outcome = app.open_palette(super::super::palette::Mode::Workspaces);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.palette.as_ref().unwrap().results.len(), 2);
    }

    #[test]
    fn startup_revalidates_stored_symlink_ancestors_before_scheduling_any_open() {
        let root = crate::test_support::tempdir();
        let configdir = crate::test_support::tempdir();
        let ancestor = root.path().join("registered-parent");
        let stored = ancestor.join("workspace");
        let foreign = root.path().join("foreign");
        std::fs::create_dir_all(&stored).unwrap();
        std::fs::create_dir_all(foreign.join("workspace/Inbox/General")).unwrap();
        std::fs::write(
            foreign.join("workspace/Inbox/General/Foreign.md"),
            "foreign bytes",
        )
        .unwrap();
        let mut config = wren_core::config::Config::load_from(configdir.path());
        config.add(&stored, Some("Registered")).unwrap();
        config.set_active(&stored).unwrap();
        let bytes = std::fs::read(configdir.path().join("config.toml")).unwrap();
        std::fs::rename(&ancestor, root.path().join("original-parent")).unwrap();
        std::os::unix::fs::symlink(&foreign, &ancestor).unwrap();
        for config in [
            config,
            wren_core::config::Config::load_from(configdir.path()),
        ] {
            let (app, _) = App::with_config(Options::default(), Some(config), None);
            assert!(
                app.loading.is_none(),
                "a stored root must be validated before starting its loader"
            );
            assert!(app.session.is_none());
            assert!(app.workspace_mode);
            assert!(app.editor.text().is_empty());
            assert!(app.registry_warning.as_deref().unwrap().contains("symlink"));
            assert_eq!(
                app.config.as_ref().unwrap().active(),
                Some(stored.as_path())
            );
            assert_eq!(app.config.as_ref().unwrap().workspaces().len(), 1);
            assert!(!foreign.join("workspace/.wren.lock").exists());
        }
        assert_eq!(
            std::fs::read(configdir.path().join("config.toml")).unwrap(),
            bytes
        );
    }

    #[test]
    fn an_explicit_fresh_workspace_still_opens_when_global_configuration_is_invalid() {
        let configdir = crate::test_support::tempdir();
        std::fs::write(configdir.path().join("config.toml"), "invalid = [").unwrap();
        let root = crate::test_support::tempdir();
        std::fs::create_dir_all(root.path().join("Inbox/General")).unwrap();
        std::fs::write(root.path().join("Inbox/General/Fresh.md"), "fresh bytes").unwrap();
        let (mut app, _) = App::with_config(
            Options {
                workspace: Some(root.path().into()),
                ..Options::default()
            },
            Some(wren_core::config::Config::load_from(configdir.path())),
            None,
        );
        assert!(app.loading.is_some());
        let slot = Arc::new(Mutex::new(Some(load(None, root.path().into()))));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.model().unwrap().root, root.path());
        assert_eq!(app.editor.text(), "fresh bytes");
        assert!(app
            .registry_warning
            .as_deref()
            .unwrap()
            .contains("read-only"));
        assert_eq!(
            app.last_processed.as_ref().unwrap().disposition,
            Disposition::CommittedWarning
        );
        append(&mut app, " edited");
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("Inbox/General/Fresh.md")).unwrap(),
            "fresh bytes edited"
        );
        assert_eq!(
            std::fs::read_to_string(configdir.path().join("config.toml")).unwrap(),
            "invalid = ["
        );
    }

    #[test]
    fn committed_switch_hydrates_new_preferences_and_restores_an_empty_parent_not_first_note() {
        let (_directory, mut app) = fixture();
        let next = crate::test_support::tempdir();
        std::fs::create_dir(next.path().join("Empty")).unwrap();
        std::fs::create_dir_all(next.path().join("WithNote/Group")).unwrap();
        std::fs::write(next.path().join("WithNote/Group/Other.md"), "other").unwrap();
        let mut preferences = Session::open(next.path()).unwrap();
        let empty = preferences
            .model()
            .sections
            .iter()
            .find(|section| section.name == "Empty")
            .unwrap()
            .id;
        preferences
            .apply(Op::SelectParent {
                parent: Parent::Section(empty),
            })
            .unwrap();
        preferences
            .apply(Op::SetWidth {
                pane: "sections".into(),
                width: 310,
            })
            .unwrap();
        drop(preferences);
        app.filters.groups = "stale".into();
        app.focus = true;
        let previous = app.session.take();
        let slot = returning(&mut app, load(previous, next.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.widths.get(crate::panes::Pane::Sections), 310.0);
        assert_eq!(app.filters, super::super::navigation::Filters::default());
        assert!(!app.focus);
        assert!(app.selected_note.is_none());
        assert_eq!(app.selected_section, Some(empty));
        assert!(app.editor.text().is_empty());
    }

    #[test]
    fn committed_workspace_switch_discards_old_dialog_targets_even_when_new_ids_collide() {
        let (_directory, mut app) = fixture();
        let original = app.selected_note.unwrap();
        let _ = app.show_dialog(DialogKind::Rename(ItemRef::Note(original)));
        app.dialog.as_mut().unwrap().name = "Wrong workspace".into();
        let next = crate::test_support::tempdir();
        std::fs::create_dir_all(next.path().join("Inbox/General")).unwrap();
        std::fs::write(next.path().join("Inbox/General/Other.md"), "other").unwrap();
        let previous = app.session.take();
        let slot = returning(&mut app, load(previous, next.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.selected_note, Some(original));
        assert!(app.dialog.is_none());
        let _ = app.submit_dialog(false);
        assert!(next.path().join("Inbox/General/Other.md").exists());
        assert!(!next
            .path()
            .join("Inbox/General/Wrong workspace.md")
            .exists());
    }

    #[test]
    fn registry_failure_after_switch_keeps_new_session_and_warning_but_cannot_replay_picker() {
        let (_directory, mut app) = fixture();
        let next = crate::test_support::tempdir();
        std::fs::create_dir_all(next.path().join("Section/Group")).unwrap();
        std::fs::write(next.path().join("Section/Group/Other.md"), "other").unwrap();
        let configdir = crate::test_support::tempdir();
        std::fs::write(configdir.path().join("config.toml"), "invalid = [").unwrap();
        app.config = Some(wren_core::config::Config::load_from(configdir.path()));
        let _ = app.open_palette(super::super::palette::Mode::Workspaces);
        let previous = app.session.take();
        let slot = returning(&mut app, load(previous, next.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.model().unwrap().root, next.path());
        assert_eq!(app.editor.text(), "other");
        assert!(app.palette.is_none());
        assert!(app.registry_warning.is_some());
        assert_eq!(
            app.last_processed.as_ref().unwrap().disposition,
            Disposition::CommittedWarning
        );
        let warning = app.registry_warning.clone();
        let _ = app.process_core(Op::Flush);
        assert_eq!(app.registry_warning, warning);
        assert_eq!(
            std::fs::read_to_string(configdir.path().join("config.toml")).unwrap(),
            "invalid = ["
        );
    }

    #[test]
    fn load_generation_guards_edits_operations_and_stale_results() {
        let (_directory, mut app) = fixture();
        let next = crate::test_support::tempdir();
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
    fn threaded_loading_returns_the_serial_owner_without_an_iced_or_tokio_runtime() {
        let (directory, mut app) = fixture();
        let session = app.session.take().unwrap();
        let generation = session.generation();
        struct WakeThread(std::thread::Thread);
        impl std::task::Wake for WakeThread {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(Arc::new(WakeThread(std::thread::current())));
        let mut context = std::task::Context::from_waker(&waker);
        let mut future = Box::pin(threaded_load(LoadJob {
            previous: Some(session),
            path: directory.path().into(),
            options: SessionOptions::default(),
            #[cfg(feature = "smoke-test")]
            sandbox: None,
        }));
        let started = std::time::Instant::now();
        let outcome = loop {
            match std::future::Future::poll(future.as_mut(), &mut context) {
                std::task::Poll::Ready(outcome) => break outcome,
                std::task::Poll::Pending => {
                    assert!(started.elapsed() < std::time::Duration::from_secs(5));
                    std::thread::park_timeout(std::time::Duration::from_millis(100));
                }
            }
        };
        let outcome = outcome.lock().unwrap().take().unwrap();
        assert!(outcome.result.is_ok());
        assert!(!outcome.opened);
        assert_eq!(outcome.session.as_ref().unwrap().generation(), generation);
        assert_eq!(
            outcome.session.as_ref().unwrap().document().unwrap().text,
            "first\nsecond"
        );
    }

    #[test]
    fn loaded_workspaces_reconcile_off_the_owner_and_drop_results_during_loads() {
        let (directory, mut app) = fixture();
        drop(app.session.take());
        let slot = returning(&mut app, load(None, directory.path().into()));
        let _ = app.workspace_loaded(app.load_generation, slot);
        std::fs::write(
            directory.path().join("Inbox/General/External.md"),
            "outside",
        )
        .unwrap();
        let prepare = |app: &mut App| {
            // A watcher hint or the one-second poll prepares the job.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let _ = app.process_core(Op::Tick);
                if let Some(job) = app.session.as_mut().unwrap().take_reconcile_job() {
                    return job;
                }
                assert!(std::time::Instant::now() < deadline, "no reconcile job");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let job = prepare(&mut app);
        assert_eq!(app.model().unwrap().notes.len(), 1, "Tick must not scan");

        let session = app.session.take();
        let slot = returning(&mut app, load(session, directory.path().into()));
        assert!(app.reconciled(job.run()).units() == 0);
        let _ = app.workspace_loaded(app.load_generation, slot);
        assert_eq!(app.model().unwrap().notes.len(), 1);

        // The retained Session released the dropped job and scans again.
        let result = prepare(&mut app).run();
        let _ = app.update(Message::Reconciled(result));
        assert_eq!(app.model().unwrap().notes.len(), 2);
        assert!(app.banner.is_none(), "{:?}", app.banner);
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
        let empty = crate::test_support::tempdir();
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
        assert!(directory.path().join("Inbox/General/Note.md").exists());
        let _ = app.update(Message::Escape);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn autosave_subscription_exists_only_while_the_session_is_owned_by_the_app() {
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
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
    fn deferred_blur_flush_preserves_successful_and_failed_load_correlations_and_metadata_warnings()
    {
        for load_succeeds in [false, true] {
            for flush_fails in [false, true] {
                let (directory, mut app) = fixture();
                app.session.as_mut().unwrap().fail_next(Fault::FileSync);
                let result = app.process_core(Op::SetWidth {
                    pane: "notes".into(),
                    width: 300,
                });
                assert_eq!(result.processed.disposition, Disposition::CommittedWarning);
                let metadata_warning = app.metadata_warning.clone().unwrap();
                append(&mut app, " retained draft");
                let mut session = app.session.take().unwrap();
                if flush_fails {
                    session.fail_next(Fault::Write);
                }
                let slot = returning(
                    &mut app,
                    LoadOutcome {
                        session: Some(session),
                        result: if load_succeeds {
                            Ok(Vec::new())
                        } else {
                            Err(wren_core::ops::OpError {
                                message: "Requested workspace is unavailable".into(),
                                events: Vec::new(),
                            })
                        },
                        opened: false,
                    },
                );
                let correlation = app.action_serial.wrapping_add(100);
                app.loading.as_mut().unwrap().correlation = correlation;
                app.window_unfocused().unwrap();
                assert!(app.flush_on_load);
                let _ = app.workspace_loaded(app.load_generation, slot);
                let processed = app.last_processed.as_ref().unwrap();
                assert_eq!(processed.correlation, correlation);
                assert_eq!(
                    processed.disposition,
                    if load_succeeds {
                        Disposition::Completed
                    } else {
                        Disposition::Blocked
                    }
                );
                assert_eq!(
                    processed.warning.as_deref(),
                    (!load_succeeds).then_some("Requested workspace is unavailable")
                );
                assert_eq!(app.metadata_warning.as_ref(), Some(&metadata_warning));
                assert_eq!(app.editor.text(), "first\nsecond retained draft");
                let document = app.session.as_ref().unwrap().document().unwrap();
                assert_eq!(document.dirty(), flush_fails);
                assert_eq!(document.error.is_some(), flush_fails);
                if flush_fails {
                    assert_eq!(app.banner_kind, WarningKind::Document);
                    assert!(app.banner.as_deref().unwrap().contains("Write"));
                } else {
                    assert_eq!(
                        std::fs::read_to_string(directory.path().join("Inbox/General/Note.md"))
                            .unwrap(),
                        app.editor.text()
                    );
                }
                assert!(!app.flush_on_load);
                let explicit = app.process_core(Op::Flush).processed;
                assert_ne!(explicit.correlation, correlation);
                assert_eq!(
                    app.last_processed.as_ref().unwrap().correlation,
                    explicit.correlation
                );
                assert_eq!(explicit.disposition, Disposition::Completed);
                assert_eq!(app.metadata_warning.as_ref(), Some(&metadata_warning));
            }
        }
    }

    #[test]
    fn deferred_blur_flush_does_not_replace_a_committed_registry_warning_load_result() {
        for flush_fails in [false, true] {
            let (_directory, mut app) = fixture();
            let configdir = crate::test_support::tempdir();
            std::fs::write(configdir.path().join("config.toml"), "invalid = [").unwrap();
            app.config = Some(wren_core::config::Config::load_from(configdir.path()));
            append(&mut app, " retained draft");
            let mut session = app.session.take().unwrap();
            if flush_fails {
                session.fail_next(Fault::Write);
            }
            let slot = returning(
                &mut app,
                LoadOutcome {
                    session: Some(session),
                    result: Ok(Vec::new()),
                    opened: false,
                },
            );
            let correlation = app.action_serial.wrapping_add(100);
            app.loading.as_mut().unwrap().correlation = correlation;
            app.window_unfocused().unwrap();
            let _ = app.workspace_loaded(app.load_generation, slot);
            let processed = app.last_processed.as_ref().unwrap();
            assert_eq!(processed.correlation, correlation);
            assert_eq!(processed.disposition, Disposition::CommittedWarning);
            assert_eq!(processed.warning, app.registry_warning);
            assert!(processed.warning.as_deref().unwrap().contains("read-only"));
            assert_eq!(
                app.session.as_ref().unwrap().document().unwrap().dirty(),
                flush_fails
            );
            assert_eq!(app.editor.text(), "first\nsecond retained draft");
            if flush_fails {
                assert_eq!(app.banner_kind, WarningKind::Document);
            }
        }
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
        use std::os::unix::fs::PermissionsExt;

        let first = crate::test_support::tempdir();
        let second = crate::test_support::tempdir();
        let sandbox = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        std::fs::create_dir_all(second.path().join("Section/Group")).unwrap();
        std::fs::write(second.path().join("Section/Group/Note.md"), "quarantined").unwrap();
        let options = SessionOptions::default();
        let first = load_workspace(
            None,
            first.path().into(),
            options,
            Some(sandbox.path().into()),
        )
        .session
        .unwrap();
        let outcome = load_workspace(Some(first), second.path().into(), options, None);
        assert!(outcome.result.is_ok());
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
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
        assert!(!second.path().join("Section/Group/Note.md").exists());
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
