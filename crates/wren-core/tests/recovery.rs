use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;
use wren_core::fs_store::Fault;
use wren_core::model::NoteId;
use wren_core::ops::{Event, ExternalChangeKind, Op, Session, Trash};

struct NoTrash;
impl Trash for NoTrash {
    fn trash(&mut self, _: &Path) -> Result<(), String> {
        panic!("Recovery tests must never trash files")
    }
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn fixture() -> (TempDir, Session, NoteId) {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("S/G")).unwrap();
    fs::write(root.path().join("S/G/a.md"), "baseline").unwrap();
    let mut session = Session::with_trash(root.path(), Box::new(NoTrash)).unwrap();
    let note = session.model().notes[0].id;
    session
        .apply_at(Op::OpenNote { note: Some(note) }, ms(0))
        .unwrap();
    (root, session, note)
}

fn draft(session: &mut Session, note: NoteId, text: &str) {
    session
        .apply_at(
            Op::EditDraft {
                note,
                contents: text.into(),
            },
            ms(0),
        )
        .unwrap();
}

fn copies(root: &Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(root.join("S/G"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("conflict")
        })
        .collect()
}

fn assert_recovered(root: &Path, session: &Session, text: &str) {
    let document = session.document().unwrap();
    assert!(!document.dirty());
    assert!(session.model().note(document.note).is_some());
    assert_eq!(document.text, text);
    assert_eq!(
        fs::read(root.join(document.path().as_path())).unwrap(),
        text.as_bytes()
    );
    assert!(
        !root.join("S/G/a.md").exists(),
        "External deletion must not resurrect the original"
    );
}

#[test]
fn unrelated_tree_refresh_cannot_orphan_a_deleted_dirty_document() {
    let (root, mut session, note) = fixture();
    draft(&mut session, note, "draft-only");
    fs::remove_file(root.path().join("S/G/a.md")).unwrap();
    session
        .apply_at(Op::CreateSection { name: "New".into() }, ms(100))
        .unwrap();
    assert!(session.model().note(note).is_some());
    let error = session.apply_at(Op::Flush, ms(101)).unwrap_err();
    assert!(error.events.iter().any(|event| matches!(
        event,
        Event::ExternalChange {
            kind: ExternalChangeKind::Conflict { .. },
            ..
        }
    )));
    assert_recovered(root.path(), &session, "draft-only");
}

#[test]
fn retry_backoff_reconciliation_keeps_a_deleted_draft_recoverable() {
    let (root, mut session, note) = fixture();
    draft(&mut session, note, "draft-only");
    session.fail_next(Fault::Write);
    assert!(session.apply_at(Op::Tick, ms(750)).is_err());
    fs::remove_file(root.path().join("S/G/a.md")).unwrap();
    session.apply_at(Op::Reconcile, ms(800)).unwrap();
    assert!(session.model().note(note).is_some());
    assert!(copies(root.path()).is_empty());
    assert!(session.apply_at(Op::Flush, ms(801)).is_err());
    assert_recovered(root.path(), &session, "draft-only");
}

#[test]
fn inactive_write_cannot_bypass_the_loaded_baseline() {
    let (root, mut session, note) = fixture();
    session
        .apply_at(Op::OpenNote { note: None }, ms(0))
        .unwrap();
    fs::write(root.path().join("S/G/a.md"), "external-version").unwrap();
    assert!(session
        .apply_at(
            Op::WriteNote {
                note,
                contents: "stale-local-version".into()
            },
            ms(1)
        )
        .is_err());
    assert_eq!(
        fs::read_to_string(root.path().join("S/G/a.md")).unwrap(),
        "external-version"
    );
    assert!(copies(root.path()).is_empty());
}

#[test]
fn failed_convergent_save_releases_the_ticket_and_can_retry() {
    let (root, mut session, note) = fixture();
    draft(&mut session, note, "converged");
    fs::write(root.path().join("S/G/a.md"), "converged").unwrap();
    session.fail_next(Fault::Write);
    assert!(session.apply_at(Op::Reconcile, ms(100)).is_err());
    assert!(session.document().unwrap().dirty());
    session.apply_at(Op::Flush, ms(101)).unwrap();
    assert!(!session.document().unwrap().dirty());
    assert_eq!(
        fs::read_to_string(root.path().join("S/G/a.md")).unwrap(),
        "converged"
    );
}

#[test]
fn invalid_external_text_cannot_create_a_conflict_copy_feedback_loop() {
    let (root, mut session, note) = fixture();
    draft(&mut session, note, "local version");
    fs::write(root.path().join("S/G/a.md"), [0xff, 0xfe]).unwrap();
    session.fail_next(Fault::DirectorySync);
    assert!(session.apply_at(Op::Reconcile, ms(100)).is_err());
    let copy = copies(root.path());
    assert_eq!(copy.len(), 1);
    let modified = fs::metadata(&copy[0]).unwrap().modified().unwrap();
    for now in [101, 125, 200, 500, 800] {
        session.apply_at(Op::Reconcile, ms(now)).unwrap();
    }
    assert_eq!(copies(root.path()), copy);
    assert_eq!(
        fs::metadata(&copy[0]).unwrap().modified().unwrap(),
        modified
    );
    assert!(session.document().unwrap().dirty());
    fs::write(root.path().join("S/G/a.md"), "external repaired").unwrap();
    session.apply_at(Op::Reconcile, ms(1200)).unwrap();
    assert!(!session.document().unwrap().dirty());
    assert_eq!(session.document().unwrap().text, "external repaired");
    assert_eq!(fs::read_to_string(&copy[0]).unwrap(), "local version");
}

#[test]
fn clean_deleted_note_is_not_kept_as_a_phantom_draft() {
    let (root, mut session, note) = fixture();
    fs::remove_file(root.path().join("S/G/a.md")).unwrap();
    session.apply_at(Op::Reconcile, ms(1)).unwrap();
    assert!(session.document().is_none());
    assert!(session.model().note(note).is_none());
    assert!(copies(root.path()).is_empty());
}
