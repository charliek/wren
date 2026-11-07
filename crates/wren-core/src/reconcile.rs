//! Off-owner workspace reconciliation. A [`ReconcileJob`] performs the directory
//! walk and note reads on another thread; the Session validates the returned
//! [`ReconcileResult`] and merges it in order through `Op::FinishReconcile`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::fs_store::{RelPath, StoreReader};
use crate::index::NOT_UTF8;
use crate::model::{NoteId, Workspace};
use crate::scan;

// Process-wide so a result can never match a different Session, including a
// replacement Session that reuses the same root and generation number.
static TICKETS: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_ticket() -> u64 {
    TICKETS.fetch_add(1, Ordering::Relaxed)
}

/// The Session state a job's snapshot was taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tags {
    pub ticket: u64,
    pub generation: u64,
    pub tree_revision: u64,
}

pub(crate) struct Scanned {
    pub model: Workspace,
    /// Whether identities, paths, parents, or metadata differ from the snapshot.
    pub changed: bool,
    /// Notes whose disk contents differ from the snapshot's indexed text, or
    /// whose contents are unavailable; unchanged notes are omitted.
    pub contents: Vec<(NoteId, RelPath, Result<String, String>)>,
}

type Payload = Option<Result<Scanned, String>>;

/// A snapshot of the Session's tree and indexed text plus a duplicated root
/// descriptor. Running it never mutates the Session.
pub struct ReconcileJob {
    reader: StoreReader,
    previous: Workspace,
    indexed: HashMap<NoteId, Arc<str>>,
    result: ReconcileResult,
}

impl std::fmt::Debug for ReconcileJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReconcileJob")
            .field("tags", &self.result.tags)
            .finish_non_exhaustive()
    }
}

impl ReconcileJob {
    pub(crate) fn new(
        tags: Tags,
        reader: StoreReader,
        previous: Workspace,
        indexed: HashMap<NoteId, Arc<str>>,
    ) -> Self {
        Self {
            reader,
            previous,
            indexed,
            result: ReconcileResult {
                tags,
                payload: Arc::new(Mutex::new(None)),
            },
        }
    }

    /// The handle to return to the Session. Returning it without a completed
    /// `run` (for example, after a worker thread failed) releases the Session's
    /// in-flight slot and schedules a retry.
    pub fn result(&self) -> ReconcileResult {
        self.result.clone()
    }

    /// Blocking filesystem I/O: call it away from the Session owner.
    pub fn run(self) -> ReconcileResult {
        let Self {
            reader,
            previous,
            indexed,
            result,
        } = self;
        let mut contents = Vec::new();
        let scanned = scan::scan_with_contents(&reader, Some(&previous), |note, file| {
            let update = match file {
                Ok(file)
                    if indexed
                        .get(&note.id)
                        .is_some_and(|text| text.as_bytes() == file.bytes) =>
                {
                    return;
                }
                Ok(file) => String::from_utf8(file.bytes.clone()).map_err(|_| NOT_UTF8.to_owned()),
                Err(reason) => Err(reason.to_owned()),
            };
            contents.push((note.id, note.path.clone(), update));
        });
        let outcome = scanned
            .map(|model| Scanned {
                changed: tree_differs(&previous, &model),
                model,
                contents,
            })
            .map_err(|error| error.to_string());
        *result
            .payload
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(outcome);
        result
    }
}

/// A job's completion handle. Clones share one payload, which the Session
/// takes at most once.
#[derive(Clone)]
pub struct ReconcileResult {
    pub(crate) tags: Tags,
    payload: Arc<Mutex<Payload>>,
}

impl std::fmt::Debug for ReconcileResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReconcileResult")
            .field("tags", &self.tags)
            .finish_non_exhaustive()
    }
}

impl ReconcileResult {
    pub(crate) fn take(&self) -> Payload {
        self.payload
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// A scan's alphabetical/default order differs from persisted manual and title
/// order, so this compares identities and metadata, not enumeration order.
pub(crate) fn tree_differs(old: &Workspace, new: &Workspace) -> bool {
    let sections: HashMap<_, _> = old
        .sections
        .iter()
        .map(|item| (item.id, (&item.path, &item.name)))
        .collect();
    let groups: HashMap<_, _> = old
        .groups
        .iter()
        .map(|item| (item.id, (&item.path, item.section)))
        .collect();
    let notes: HashMap<_, _> = old
        .notes
        .iter()
        .map(|item| {
            (
                item.id,
                (&item.path, item.modified, item.bytes, item.parent),
            )
        })
        .collect();
    sections.len() != new.sections.len()
        || groups.len() != new.groups.len()
        || notes.len() != new.notes.len()
        || new
            .sections
            .iter()
            .any(|item| sections.get(&item.id) != Some(&(&item.path, &item.name)))
        || new
            .groups
            .iter()
            .any(|item| groups.get(&item.id) != Some(&(&item.path, item.section)))
        || new.notes.iter().any(|item| {
            notes.get(&item.id) != Some(&(&item.path, item.modified, item.bytes, item.parent))
        })
}
