use std::fs::{self, File, FileTimes};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use wren_core::fs_store::{Fault, RelPath};
use wren_core::index::{QueryKind, Scope};
use wren_core::model::{GroupId, ItemRef, NoteId, Parent, SectionId};
use wren_core::ops::{Event, Op, Session, Trash};

struct PrivateTrash {
    directory: TempDir,
    next: usize,
}

impl Trash for PrivateTrash {
    fn trash(&mut self, path: &Path) -> Result<(), String> {
        let destination = self.directory.path().join(self.next.to_string());
        self.next += 1;
        fs::rename(path, destination).map_err(|error| error.to_string())
    }
}

fn session(root: &Path) -> Session {
    Session::with_trash(
        root,
        Box::new(PrivateTrash {
            directory: TempDir::new().unwrap(),
            next: 0,
        }),
    )
    .unwrap()
}

fn write(root: &Path, path: &str, text: impl AsRef<[u8]>) {
    let absolute = root.join(path);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(absolute, text).unwrap();
}

fn fixture() -> TempDir {
    let root = TempDir::new().unwrap();
    for (path, text) in [
        ("A/G/Note 2.md", "first body"),
        ("A/G/Note 10.md", "second body"),
        ("A/L/Loose.md", "loose body"),
        ("B/H/Other.md", "another body"),
    ] {
        write(root.path(), path, text);
    }
    root
}

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

fn note(session: &Session, path: &str) -> NoteId {
    session
        .model()
        .notes
        .iter()
        .find(|note| note.path == rel(path))
        .unwrap_or_else(|| panic!("missing {path}"))
        .id
}

fn section(session: &Session, path: &str) -> SectionId {
    session
        .model()
        .sections
        .iter()
        .find(|section| section.path == rel(path))
        .unwrap()
        .id
}

fn group(session: &Session, path: &str) -> GroupId {
    session
        .model()
        .groups
        .iter()
        .find(|group| group.path == rel(path))
        .unwrap()
        .id
}

fn run(session: &mut Session, op: Op) -> Vec<Event> {
    session
        .apply_at(op, Duration::ZERO)
        .unwrap_or_else(|error| panic!("{error}; committed events: {:?}", error.events))
}

fn open(session: &mut Session, note: NoteId) {
    run(session, Op::OpenNote { note: Some(note) });
}

fn edit(session: &mut Session, note: NoteId, text: &str) {
    run(
        session,
        Op::EditDraft {
            note,
            contents: text.into(),
        },
    );
}

fn hits(session: &Session, query: &str) -> Vec<NoteId> {
    session
        .index()
        .query(query, Scope::Workspace, QueryKind::Search, 100)
        .iter()
        .map(|hit| hit.note)
        .collect()
}

fn titles(session: &Session, group: GroupId) -> Vec<&str> {
    session
        .model()
        .notes
        .iter()
        .filter(|note| note.group == group)
        .map(|note| note.title.as_str())
        .collect()
}

fn group_names(session: &Session, section: SectionId) -> Vec<&str> {
    session
        .model()
        .groups
        .iter()
        .filter(|group| group.section == section)
        .map(|group| group.name.as_str())
        .collect()
}

fn group_move_fixture() -> TempDir {
    let root = TempDir::new().unwrap();
    for (path, text) in [
        ("Alpha/C/Note 2.md", "active group needle"),
        ("Alpha/C/Note 10.md", "second group needle"),
        ("Alpha/C/deep/ZDeep.md", "deep group needle"),
        ("Alpha/Source 2/Stay.md", "source stay"),
        ("Alpha/Source 10/Stay.md", "other source stay"),
        ("Beta/Foreign 2/Note.md", "foreign two"),
        ("Beta/Foreign 10/Note.md", "foreign ten"),
        ("Beta/Visible A/Note.md", "visible a"),
        ("Beta/Visible Z/Note.md", "visible z"),
        ("Beta/Hidden A/Note.md", "hidden a"),
        ("Beta/Hidden Z/Note.md", "hidden z"),
        ("Gamma/Other 2/Note.md", "other two"),
        ("Gamma/Other 10/Note.md", "other ten"),
    ] {
        write(root.path(), path, text);
    }
    write(
        root.path(),
        ".wren/workspace.toml",
        r#"order = ['Alpha/C', 'Alpha/Source 10', 'Alpha/Source 2', 'Gamma', 'Beta', 'Alpha', 'Gamma/Other 10', 'Gamma/Other 2', 'Beta/Visible Z', 'Beta/Hidden Z', 'Beta/Visible A', 'Beta/Hidden A', 'Beta/Absent']
[sort]
'Alpha/C' = 'title'
[future]
selection = 'Alpha/C/Note 2.md'
opaque = ['untouched', { value = 42 }]
[widths.future]
custom = 72
"#,
    );
    root
}

fn assert_appended_group_order(session: &Session) {
    assert_eq!(
        group_names(session, section(session, "Beta")),
        [
            "Visible Z",
            "Hidden Z",
            "Visible A",
            "Hidden A",
            "Foreign 2",
            "Foreign 10",
            "C"
        ]
    );
    assert_eq!(
        group_names(session, section(session, "Alpha")),
        ["Source 10", "Source 2"]
    );
    assert_eq!(
        group_names(session, section(session, "Gamma")),
        ["Other 10", "Other 2"]
    );
    assert_eq!(
        session
            .model()
            .sections
            .iter()
            .map(|section| section.name.as_str())
            .collect::<Vec<_>>(),
        ["Gamma", "Beta", "Alpha"]
    );
    assert!(session.state().order().contains(&rel("Beta/Absent")));
}

fn set_modified(root: &Path, path: &str, seconds: u64) {
    File::open(root.join(path))
        .unwrap()
        .set_times(
            FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
        )
        .unwrap();
}

#[test]
fn moving_a_parent_selected_active_note_restores_its_destination_on_restart() {
    for select_section in [false, true] {
        let root = TempDir::new().unwrap();
        write(root.path(), "A/G/a.md", "active body");
        write(root.path(), "A/G/b.md", "remaining body");
        fs::create_dir_all(root.path().join("B/H")).unwrap();
        let mut current = session(root.path());
        let g = group(&current, "A/G");
        run(
            &mut current,
            Op::SetSort {
                group: g,
                mode: "title".into(),
            },
        );
        let selection = if select_section {
            Op::SelectSection {
                section: section(&current, "A"),
            }
        } else {
            Op::SelectParent {
                parent: Parent::Group(g),
            }
        };
        run(&mut current, selection);
        let active = note(&current, "A/G/a.md");
        assert_eq!(current.document().unwrap().note, active);
        let to = group(&current, "B/H");
        run(&mut current, Op::MoveNote { note: active, to });
        assert_eq!(current.selected_parent(), Some(Parent::Group(to)));
        drop(current);
        let reopened = session(root.path());
        assert_eq!(reopened.document().unwrap().path(), &rel("B/H/a.md"));
        assert_eq!(
            reopened.selected_parent(),
            Some(Parent::Group(group(&reopened, "B/H")))
        );
        assert_eq!(
            fs::read_to_string(root.path().join("B/H/a.md")).unwrap(),
            "active body"
        );
    }
}

#[test]
fn leaving_an_unreadable_active_note_revalidates_its_closed_index_even_on_metadata_failure() {
    for close in [false, true] {
        for metadata_failure in [false, true] {
            let root = fixture();
            let mut current = session(root.path());
            let outgoing = note(&current, "A/G/Note 2.md");
            let next = note(&current, "A/G/Note 10.md");
            open(&mut current, outgoing);
            write(root.path(), "A/G/Note 2.md", b"\xff invalid");
            assert!(current.apply_at(Op::Reconcile, Duration::ZERO).is_err());
            assert_eq!(current.document().unwrap().text, "first body");
            assert_eq!(hits(&current, "first body"), [outgoing]);
            if metadata_failure {
                let path = root.path().join(".wren/workspace.toml");
                let mut source = fs::read_to_string(&path).unwrap();
                source.push_str("\nexternal_review = true\n");
                fs::write(path, source).unwrap();
            }
            let result = current.apply_at(
                Op::OpenNote {
                    note: (!close).then_some(next),
                },
                Duration::ZERO,
            );
            assert_eq!(result.is_err(), metadata_failure);
            assert_eq!(
                current.document().map(|document| document.note),
                (!close).then_some(next)
            );
            assert!(hits(&current, "first body").is_empty());
            assert_eq!(current.index().content(outgoing), None);
            assert!(current
                .index()
                .limitation(outgoing)
                .unwrap()
                .contains("UTF-8"));
            assert_eq!(
                fs::read(root.path().join("A/G/Note 2.md")).unwrap(),
                b"\xff invalid"
            );
            write(root.path(), "A/G/Note 2.md", "recovered searchable");
            run(&mut current, Op::Reconcile);
            assert_eq!(hits(&current, "recovered searchable"), [outgoing]);
        }
    }
}

#[test]
fn load_builds_descriptor_read_content_with_exact_group_and_section_scopes() {
    let root = fixture();
    write(root.path(), "A/G/deep/Deep.md", "deep body");
    let session = session(root.path());
    assert_eq!(session.index().len(), 5);
    assert!(!session.index().is_empty());
    let loose = note(&session, "A/L/Loose.md");
    let deep = note(&session, "A/G/deep/Deep.md");
    assert_eq!(session.index().preview(loose), Some("loose body"));
    assert_eq!(session.index().content(deep), Some("deep body"));
    assert_eq!(
        session
            .index()
            .query(
                "body",
                Scope::Section(section(&session, "A")),
                QueryKind::Search,
                100
            )
            .len(),
        4
    );
    assert_eq!(
        session
            .index()
            .query(
                "body",
                Scope::Group(group(&session, "A/L")),
                QueryKind::Filter,
                100
            )
            .iter()
            .map(|hit| hit.note)
            .collect::<Vec<_>>(),
        [loose]
    );
}

#[test]
fn queries_previews_and_content_do_not_read_the_filesystem() {
    let root = fixture();
    let session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    let path = root.path().join("A");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let results = hits(&session, "first body");
    let preview = session.index().preview(id).map(str::to_owned);
    let content = session.index().content(id).map(str::to_owned);
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(results, [id]);
    assert_eq!(preview.as_deref(), Some("first body"));
    assert_eq!(content.as_deref(), Some("first body"));
}

#[test]
fn drafts_are_immediately_searchable_and_a_failed_save_does_not_restore_stale_disk_text() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    open(&mut session, id);
    let revision = session.index().revision();
    edit(&mut session, id, "draft café İSTANBUL");
    assert!(session.index().revision() > revision);
    assert_eq!(hits(&session, "draft café"), [id]);
    assert!(hits(&session, "first body").is_empty());
    assert_eq!(
        fs::read_to_string(root.path().join("A/G/Note 2.md")).unwrap(),
        "first body"
    );
    session.fail_next(Fault::Write);
    assert!(session.apply_at(Op::Flush, Duration::ZERO).is_err());
    assert_eq!(session.index().content(id), Some("draft café İSTANBUL"));
    assert!(session.document().unwrap().dirty());
    run(&mut session, Op::Flush);
    assert!(!session.document().unwrap().dirty());
    assert_eq!(
        fs::read_to_string(root.path().join("A/G/Note 2.md")).unwrap(),
        "draft café İSTANBUL"
    );
    let hit = &session
        .index()
        .query("i\u{307}s", Scope::Workspace, QueryKind::Search, 100)[0];
    assert_eq!(
        &session.document().unwrap().text[hit.content_match.clone().unwrap()],
        "İS"
    );
    edit(&mut session, id, "later unsaved text");
    edit(&mut session, id, "draft café İSTANBUL");
    assert!(!session.document().unwrap().dirty());
    assert!(hits(&session, "later unsaved").is_empty());
}

#[test]
fn own_write_rename_move_ancestor_rename_and_delete_keep_index_ids_paths_and_scope_current() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    let destination = group(&session, "B/H");
    open(&mut session, id);
    run(
        &mut session,
        Op::WriteNote {
            note: id,
            contents: "distinctive written body".into(),
        },
    );
    run(
        &mut session,
        Op::Rename {
            target: ItemRef::Note(id),
            new_name: "Renamed".into(),
        },
    );
    assert!(session
        .index()
        .query("Note 2", Scope::Workspace, QueryKind::Jump, 100)
        .is_empty());
    run(
        &mut session,
        Op::MoveNote {
            note: id,
            to: destination,
        },
    );
    let section_b = section(&session, "B");
    assert_eq!(session.selected_parent(), Some(Parent::Group(destination)));
    assert_eq!(
        session.index().query(
            "distinctive",
            Scope::Section(section_b),
            QueryKind::Search,
            100
        )[0]
        .note,
        id
    );
    assert!(session
        .index()
        .query(
            "distinctive",
            Scope::Section(section(&session, "A")),
            QueryKind::Search,
            100
        )
        .is_empty());
    run(
        &mut session,
        Op::Rename {
            target: ItemRef::Section(section_b),
            new_name: "Archive".into(),
        },
    );
    let hit = &session
        .index()
        .query("distinctive", Scope::Workspace, QueryKind::Search, 100)[0];
    assert_eq!(hit.note, id);
    assert_eq!(hit.path, rel("Archive/H/Renamed.md"));
    assert_eq!(hit.title, "Renamed");
    run(
        &mut session,
        Op::Delete {
            target: ItemRef::Group(destination),
        },
    );
    assert!(session.index().content(id).is_none());
    assert!(hits(&session, "distinctive").is_empty());
    assert_eq!(session.index().len(), session.model().notes.len());
    assert!(!root.path().join("Archive/H").exists());
}

#[test]
fn partial_note_creation_and_relocation_publications_are_indexed_on_error() {
    let root = fixture();
    let mut session = session(root.path());
    let g = group(&session, "A/G");
    session.fail_next(Fault::DirectorySync);
    let error = session
        .apply_at(
            Op::CreateNote {
                group: g,
                title: "Published".into(),
            },
            Duration::ZERO,
        )
        .unwrap_err();
    let id = note(&session, "A/G/Published.md");
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::Created(ItemRef::Note(note)) if *note == id)));
    assert_eq!(session.index().content(id), Some(""));
    assert_eq!(
        session
            .index()
            .query("Published", Scope::Workspace, QueryKind::Jump, 100)[0]
            .note,
        id
    );
    run(&mut session, Op::Flush);
    session.fail_next(Fault::DirectorySync);
    let to = group(&session, "B/H");
    let error = session
        .apply_at(Op::MoveNote { note: id, to }, Duration::ZERO)
        .unwrap_err();
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::NoteMoved { note, .. } if *note == id)));
    assert_eq!(
        session
            .index()
            .query("Published", Scope::Group(to), QueryKind::Jump, 100)[0]
            .path,
        rel("B/H/Published.md")
    );
}

#[test]
fn committed_rename_is_indexed_even_if_optional_metadata_conflicts() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    open(&mut session, id);
    write(root.path(), ".wren/workspace.toml", "layout = 'outline'\n");
    let error = session
        .apply_at(
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: "Committed".into(),
            },
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::NoteMoved { note, .. } if *note == id)));
    let hit = &session
        .index()
        .query("Committed", Scope::Workspace, QueryKind::Jump, 100)[0];
    assert_eq!(hit.note, id);
    assert_eq!(hit.path, rel("A/G/Committed.md"));
    assert_eq!(session.index().content(id), Some("first body"));
    assert_eq!(
        fs::read_to_string(root.path().join(".wren/workspace.toml")).unwrap(),
        "layout = 'outline'\n"
    );
}

#[test]
fn inactive_same_size_restored_mtime_edit_refreshes_index_without_requiring_tree_metadata_change() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    let other = note(&session, "B/H/Other.md");
    open(&mut session, other);
    let path = root.path().join("A/G/Note 2.md");
    let original = fs::metadata(&path).unwrap();
    let revision = session.index().revision();
    fs::write(&path, "other text").unwrap();
    File::open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(original.modified().unwrap()))
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), original.len());
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        original.modified().unwrap()
    );
    run(&mut session, Op::Reconcile);
    assert_eq!(hits(&session, "other text"), [id]);
    assert!(hits(&session, "first body").is_empty());
    assert!(session.index().revision() > revision);
    let revision = session.index().revision();
    run(&mut session, Op::Reconcile);
    assert_eq!(session.index().revision(), revision);
    assert_eq!(session.document().unwrap().note, other);
    assert_eq!(session.document().unwrap().text, "another body");
}

#[test]
fn external_create_move_rename_and_delete_remove_old_index_paths() {
    let root = fixture();
    let mut session = session(root.path());
    write(root.path(), "A/G/New.md", "new external needle");
    run(&mut session, Op::Reconcile);
    let created = note(&session, "A/G/New.md");
    assert_eq!(hits(&session, "external needle"), [created]);
    fs::rename(
        root.path().join("A/G/New.md"),
        root.path().join("B/H/Relocated.md"),
    )
    .unwrap();
    run(&mut session, Op::Reconcile);
    assert!(session.index().content(created).is_none());
    let relocated = note(&session, "B/H/Relocated.md");
    assert_eq!(
        session.index().query(
            "external needle",
            Scope::Section(section(&session, "B")),
            QueryKind::Search,
            100
        )[0]
        .path,
        rel("B/H/Relocated.md")
    );
    fs::remove_file(root.path().join("B/H/Relocated.md")).unwrap();
    run(&mut session, Op::Reconcile);
    assert!(session.index().content(relocated).is_none());
    assert!(hits(&session, "external needle").is_empty());
}

#[test]
fn inactive_invalid_utf8_and_unreadable_content_warn_once_without_erasing_titles() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    write(root.path(), "A/G/Note 2.md", [0xff, 0xfe]);
    let events = run(&mut session, Op::Reconcile);
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Warning(message) if message.contains("UTF-8"))));
    assert_eq!(session.index().content(id), None);
    assert_eq!(
        session
            .index()
            .query("Note 2", Scope::Workspace, QueryKind::Jump, 100)[0]
            .note,
        id
    );
    assert!(session.index().limitation(id).unwrap().contains("UTF-8"));
    for _ in 0..3 {
        assert!(!run(&mut session, Op::Reconcile)
            .iter()
            .any(|event| matches!(event, Event::Warning(_))));
    }
    write(root.path(), "A/G/Note 2.md", "fixed bytes");
    run(&mut session, Op::Reconcile);
    assert_eq!(session.index().limitation(id), None);
    assert_eq!(hits(&session, "fixed bytes"), [id]);
    let path = root.path().join("A/G/Note 2.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let events = run(&mut session, Op::Reconcile);
    let content_missing = session.index().content(id).is_none();
    let repeat = run(&mut session, Op::Reconcile);
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(content_missing, "The permission test must run unprivileged");
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Warning(message) if message.contains("unavailable"))));
    assert!(!repeat
        .iter()
        .any(|event| matches!(event, Event::Warning(_))));
    assert_eq!(
        session
            .index()
            .query("Note 2", Scope::Workspace, QueryKind::Jump, 100)[0]
            .note,
        id
    );
}

#[test]
fn symlink_replacements_never_index_external_contents() {
    for ancestor in [false, true] {
        let root = fixture();
        let outside = TempDir::new().unwrap();
        write(outside.path(), "Note 2.md", "foreign secret");
        let mut session = session(root.path());
        let id = note(&session, "A/G/Note 2.md");
        if ancestor {
            fs::rename(root.path().join("A/G"), root.path().join("parked")).unwrap();
            symlink(outside.path(), root.path().join("A/G")).unwrap();
        } else {
            fs::remove_file(root.path().join("A/G/Note 2.md")).unwrap();
            symlink(
                outside.path().join("Note 2.md"),
                root.path().join("A/G/Note 2.md"),
            )
            .unwrap();
        }
        run(&mut session, Op::Reconcile);
        assert!(hits(&session, "foreign secret").is_empty());
        assert!(session.index().content(id).is_none());
        assert_eq!(
            fs::read_to_string(outside.path().join("Note 2.md")).unwrap(),
            "foreign secret"
        );
    }
}

#[test]
fn an_active_reconciliation_error_does_not_block_inactive_content_updates_or_clear_live_text() {
    let root = fixture();
    let mut session = session(root.path());
    let active = note(&session, "A/G/Note 2.md");
    let inactive = note(&session, "B/H/Other.md");
    open(&mut session, active);
    write(root.path(), "A/G/Note 2.md", [0xff]);
    write(root.path(), "B/H/Other.md", "independent external update");
    assert!(session.apply_at(Op::Reconcile, Duration::ZERO).is_err());
    assert_eq!(session.document().unwrap().text, "first body");
    assert_eq!(session.index().content(active), Some("first body"));
    assert_eq!(hits(&session, "independent external"), [inactive]);
    assert_eq!(fs::read(root.path().join("A/G/Note 2.md")).unwrap(), [0xff]);
}

#[test]
fn partial_conflict_copy_publication_is_indexed_while_the_original_draft_remains_truth() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    open(&mut session, id);
    edit(&mut session, id, "unique draft recovery");
    write(root.path(), "A/G/Note 2.md", "unique external version");
    session.fail_next(Fault::DirectorySync);
    let error = session.apply_at(Op::Flush, Duration::ZERO).unwrap_err();
    assert!(error.message.contains("Conflict copy published"));
    assert_eq!(session.document().unwrap().text, "unique draft recovery");
    assert!(session.document().unwrap().dirty());
    assert_eq!(session.index().len(), 5);
    let copies: Vec<_> = session
        .model()
        .notes
        .iter()
        .filter(|note| note.title.contains("conflict"))
        .collect();
    assert_eq!(copies.len(), 1);
    let copy = copies[0].id;
    let copy_path = copies[0].path.clone();
    let draft_hits = hits(&session, "unique draft recovery");
    assert_eq!(draft_hits.len(), 2);
    assert!(draft_hits.contains(&id) && draft_hits.contains(&copy));
    assert_eq!(session.index().content(copy), Some("unique draft recovery"));
    assert_eq!(
        fs::read_to_string(root.path().join(copy_path.as_path())).unwrap(),
        "unique draft recovery"
    );
    let error = session
        .apply_at(Op::Flush, Duration::from_secs(1))
        .unwrap_err();
    assert!(error.message.contains("transition was cancelled"));
    assert_eq!(session.index().len(), 5);
    assert_eq!(hits(&session, "unique draft recovery"), [copy]);
    assert_eq!(hits(&session, "unique external version"), [id]);
}

#[test]
fn failed_selection_flush_keeps_the_old_parent_note_metadata_and_draft_index() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    open(&mut session, id);
    edit(&mut session, id, "selection must not advance");
    let parent = session.selected_parent();
    let selection = session.state().selection();
    let last_open = session.state().last_open_note();
    let to = Parent::Group(group(&session, "B/H"));
    session.fail_next(Fault::Write);
    let error = session
        .apply_at(Op::SelectParent { parent: to }, Duration::ZERO)
        .unwrap_err();
    assert!(error.events.is_empty());
    assert_eq!(session.selected_parent(), parent);
    assert_eq!(session.state().selection(), selection);
    assert_eq!(session.state().last_open_note(), last_open);
    assert_eq!(session.document().unwrap().note, id);
    assert!(session.document().unwrap().dirty());
    assert_eq!(hits(&session, "selection must not advance"), [id]);
    run(&mut session, Op::SelectParent { parent: to });
    assert_eq!(session.selected_parent(), Some(to));
    assert_eq!(
        fs::read_to_string(root.path().join("A/G/Note 2.md")).unwrap(),
        "selection must not advance"
    );
}

#[test]
fn empty_group_and_section_selections_round_trip_without_reopening_an_old_note() {
    for selected in ["Empty", "A/Empty"] {
        let root = fixture();
        fs::create_dir_all(root.path().join(selected)).unwrap();
        let mut session = session(root.path());
        let old = note(&session, "A/G/Note 2.md");
        open(&mut session, old);
        let parent = if selected == "Empty" {
            Parent::Section(section(&session, selected))
        } else {
            Parent::Group(group(&session, selected))
        };
        run(&mut session, Op::SelectParent { parent });
        assert_eq!(session.selected_parent(), Some(parent));
        assert!(session.document().is_none());
        assert_eq!(session.state().selection(), Some(rel(selected)));
        assert_eq!(session.state().last_open_note(), None);
        drop(session);
        let reopened = self::session(root.path());
        let restored = reopened.selected_parent().unwrap();
        assert_eq!(reopened.model().parent_path(restored), Some(&rel(selected)));
        assert!(reopened.document().is_none());
        assert_eq!(reopened.state().last_open_note(), None);
    }
}

#[test]
fn section_selection_picks_the_first_manually_ordered_group_even_if_empty() {
    let root = fixture();
    fs::create_dir_all(root.path().join("B/Empty")).unwrap();
    let mut session = session(root.path());
    let loose = note(&session, "A/L/Loose.md");
    let l = group(&session, "A/L");
    let g = group(&session, "A/G");
    let section_a = section(&session, "A");
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Group(l),
            before: Some(ItemRef::Group(g)),
        },
    );
    run(&mut session, Op::SelectSection { section: section_a });
    assert_eq!(session.selected_parent(), Some(Parent::Group(l)));
    assert_eq!(session.document().unwrap().note, loose);
    let empty = group(&session, "B/Empty");
    let h = group(&session, "B/H");
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Group(empty),
            before: Some(ItemRef::Group(h)),
        },
    );
    let section_b = section(&session, "B");
    run(&mut session, Op::SelectSection { section: section_b });
    assert_eq!(session.selected_parent(), Some(Parent::Group(empty)));
    assert!(session.document().is_none());
    assert_eq!(session.state().selection(), Some(rel("B/Empty")));
    drop(session);
    let reopened = self::session(root.path());
    assert_eq!(
        reopened
            .model()
            .parent_path(reopened.selected_parent().unwrap()),
        Some(&rel("B/Empty"))
    );
}

#[test]
fn selecting_the_current_parent_does_not_replace_the_live_document() {
    let root = fixture();
    let mut session = session(root.path());
    let g = group(&session, "A/G");
    let parent = Parent::Group(g);
    run(
        &mut session,
        Op::SetSort {
            group: g,
            mode: "title".into(),
        },
    );
    run(&mut session, Op::SelectParent { parent });
    let ptr = session.document().unwrap().text.as_ptr();
    let id = session.document().unwrap().note;
    run(&mut session, Op::SelectParent { parent });
    assert_eq!(session.document().unwrap().text.as_ptr(), ptr);
    assert_eq!(session.document().unwrap().note, id);
    assert_eq!(session.document().unwrap().text, "first body");
}

#[test]
fn layout_width_reset_and_sort_are_metadata_only_and_persist_with_unknown_values() {
    let root = fixture();
    write(
        root.path(),
        ".wren/workspace.toml",
        "layout = 'columns'\nfuture = ['untouched']\n[widths.future]\ncustom = 72\n",
    );
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    let g = group(&session, "A/G");
    open(&mut session, id);
    edit(&mut session, id, "view preferences must not flush");
    let ptr = session.document().unwrap().text.as_ptr();
    for op in [
        Op::SetLayout {
            layout: "outline".into(),
        },
        Op::SetWidth {
            layout: "columns".into(),
            pane: "groups".into(),
            width: 333,
        },
        Op::SetWidth {
            layout: "outline".into(),
            pane: "sections".into(),
            width: 999,
        },
        Op::SetSort {
            group: g,
            mode: "title".into(),
        },
    ] {
        let events = run(&mut session, op);
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::Selected(_) | Event::NoteSaved { .. } | Event::DraftChanged(_)
        )));
        assert_eq!(session.document().unwrap().text.as_ptr(), ptr);
        assert!(session.document().unwrap().dirty());
    }
    assert_eq!(
        fs::read_to_string(root.path().join("A/G/Note 2.md")).unwrap(),
        "first body"
    );
    assert_eq!(session.state().layout(), "outline");
    assert_eq!(session.state().width("columns", "groups"), Some(333));
    assert_eq!(session.state().width("outline", "sections"), Some(420));
    assert_eq!(titles(&session, g), ["Note 2", "Note 10"]);
    run(&mut session, Op::Flush);
    drop(session);
    let mut reopened = self::session(root.path());
    assert_eq!(reopened.state().layout(), "outline");
    assert_eq!(reopened.state().width("columns", "groups"), Some(333));
    assert_eq!(reopened.state().width("outline", "sections"), Some(420));
    assert_eq!(reopened.state().sort(&rel("A/G")), "title");
    assert_eq!(
        reopened.document().unwrap().text,
        "view preferences must not flush"
    );
    run(&mut reopened, Op::ResetWidths);
    for (layout, pane, width) in [
        ("columns", "sections", 200),
        ("columns", "groups", 220),
        ("columns", "notes", 280),
        ("outline", "sections", 260),
        ("outline", "notes", 280),
    ] {
        assert_eq!(reopened.state().width(layout, pane), Some(width));
    }
    let metadata: toml::Table = fs::read_to_string(root.path().join(".wren/workspace.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        metadata["future"].as_array().unwrap()[0].as_str(),
        Some("untouched")
    );
    assert_eq!(
        metadata["widths"]["future"]["custom"].as_integer(),
        Some(72)
    );
}

#[test]
fn corrupt_optional_metadata_allows_in_memory_view_changes_but_is_never_overwritten() {
    let root = fixture();
    let original = "layout = [ broken";
    write(root.path(), ".wren/workspace.toml", original);
    let mut session = session(root.path());
    assert!(!session.state().writable());
    let id = note(&session, "A/G/Note 2.md");
    let g = group(&session, "A/G");
    open(&mut session, id);
    edit(&mut session, id, "live draft");
    let ptr = session.document().unwrap().text.as_ptr();
    for op in [
        Op::SetLayout {
            layout: "outline".into(),
        },
        Op::SetWidth {
            layout: "columns".into(),
            pane: "notes".into(),
            width: 470,
        },
        Op::ResetWidths,
        Op::SetSort {
            group: g,
            mode: "title".into(),
        },
    ] {
        run(&mut session, op);
        assert_eq!(session.document().unwrap().text.as_ptr(), ptr);
        assert_eq!(session.document().unwrap().text, "live draft");
        assert!(session.document().unwrap().dirty());
        assert_eq!(
            fs::read_to_string(root.path().join(".wren/workspace.toml")).unwrap(),
            original
        );
    }
    assert_eq!(session.state().layout(), "outline");
    assert_eq!(titles(&session, g), ["Note 2", "Note 10"]);
}

#[test]
fn preference_persistence_errors_preserve_the_applied_view_change_and_live_dirty_document() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    open(&mut session, id);
    edit(&mut session, id, "still dirty");
    let ptr = session.document().unwrap().text.as_ptr();
    session.fail_next(Fault::Write);
    let error = session
        .apply_at(
            Op::SetLayout {
                layout: "outline".into(),
            },
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::Warning(_))));
    assert_eq!(session.state().layout(), "outline");
    assert_eq!(session.document().unwrap().text.as_ptr(), ptr);
    assert!(session.document().unwrap().dirty());
    assert_eq!(
        fs::read_to_string(root.path().join("A/G/Note 2.md")).unwrap(),
        "first body"
    );
    run(
        &mut session,
        Op::SetLayout {
            layout: "outline".into(),
        },
    );
    assert_eq!(session.index().content(id), Some("still dirty"));
}

#[test]
fn invalid_preference_and_selection_requests_leave_views_selection_and_drafts_unchanged() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    let g = group(&session, "A/G");
    let parent = Parent::Group(g);
    open(&mut session, id);
    edit(&mut session, id, "dirty truth");
    let selection = session.state().selection();
    for op in [
        Op::SetLayout {
            layout: "future".into(),
        },
        Op::SetWidth {
            layout: "outline".into(),
            pane: "groups".into(),
            width: 200,
        },
        Op::SetSort {
            group: g,
            mode: "future".into(),
        },
        Op::SetSort {
            group: GroupId(u64::MAX),
            mode: "title".into(),
        },
        Op::SelectParent {
            parent: Parent::Section(SectionId(u64::MAX)),
        },
    ] {
        assert!(session.apply_at(op, Duration::ZERO).is_err());
        assert_eq!(session.state().layout(), "columns");
        assert_eq!(session.state().selection(), selection);
        assert_eq!(session.selected_parent(), Some(parent));
        assert_eq!(session.document().unwrap().text, "dirty truth");
        assert!(session.document().unwrap().dirty());
    }
    assert_eq!(
        fs::read_to_string(root.path().join("A/G/Note 2.md")).unwrap(),
        "first body"
    );
}

#[test]
fn sorting_is_per_group_natural_or_modified_and_keeps_ids_history_and_manual_container_order() {
    let root = fixture();
    write(root.path(), "B/H/Other 2.md", "two");
    write(root.path(), "B/H/Other 10.md", "ten");
    for (path, seconds) in [
        ("A/G/Note 2.md", 100),
        ("A/G/Note 10.md", 200),
        ("B/H/Other.md", 100),
        ("B/H/Other 2.md", 300),
        ("B/H/Other 10.md", 200),
    ] {
        set_modified(root.path(), path, seconds);
    }
    let mut session = session(root.path());
    let g = group(&session, "A/G");
    let parent = Parent::Group(g);
    let h = group(&session, "B/H");
    assert_eq!(titles(&session, g), ["Note 10", "Note 2"]);
    assert_eq!(titles(&session, h), ["Other 2", "Other 10", "Other"]);
    let id2 = note(&session, "A/G/Note 2.md");
    let id10 = note(&session, "A/G/Note 10.md");
    open(&mut session, id2);
    open(&mut session, id10);
    let a = section(&session, "A");
    let b = section(&session, "B");
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Section(b),
            before: Some(ItemRef::Section(a)),
        },
    );
    run(
        &mut session,
        Op::SetSort {
            group: g,
            mode: "title".into(),
        },
    );
    assert_eq!(titles(&session, g), ["Note 2", "Note 10"]);
    assert_eq!(titles(&session, h), ["Other 2", "Other 10", "Other"]);
    assert_eq!(
        session
            .model()
            .sections
            .iter()
            .map(|section| section.id)
            .collect::<Vec<_>>(),
        [b, a]
    );
    assert_eq!(note(&session, "A/G/Note 2.md"), id2);
    assert_eq!(note(&session, "A/G/Note 10.md"), id10);
    run(&mut session, Op::Back);
    assert_eq!(session.document().unwrap().note, id2);
    run(&mut session, Op::Forward);
    assert_eq!(session.document().unwrap().note, id10);
    run(&mut session, Op::SelectParent { parent });
    assert_eq!(session.document().unwrap().note, id2);
    drop(session);
    let mut reopened = self::session(root.path());
    let g = group(&reopened, "A/G");
    assert_eq!(titles(&reopened, g), ["Note 2", "Note 10"]);
    assert_eq!(
        reopened
            .model()
            .sections
            .iter()
            .map(|section| section.name.as_str())
            .collect::<Vec<_>>(),
        ["B", "A"]
    );
    run(
        &mut reopened,
        Op::SetSort {
            group: g,
            mode: "modified".into(),
        },
    );
    assert_eq!(titles(&reopened, g), ["Note 10", "Note 2"]);
}

#[test]
fn modified_sort_reorders_on_own_save_and_external_reconciliation_with_natural_ties() {
    let root = fixture();
    set_modified(root.path(), "A/G/Note 2.md", 100);
    set_modified(root.path(), "A/G/Note 10.md", 100);
    let mut session = session(root.path());
    let g = group(&session, "A/G");
    assert_eq!(titles(&session, g), ["Note 2", "Note 10"]);
    let id10 = note(&session, "A/G/Note 10.md");
    open(&mut session, id10);
    run(
        &mut session,
        Op::WriteNote {
            note: id10,
            contents: "newest own save".into(),
        },
    );
    assert_eq!(titles(&session, g), ["Note 10", "Note 2"]);
    set_modified(root.path(), "A/G/Note 2.md", 4_000_000_000);
    run(&mut session, Op::Reconcile);
    assert_eq!(titles(&session, g), ["Note 2", "Note 10"]);
    assert_eq!(
        session
            .index()
            .query("", Scope::Group(g), QueryKind::Filter, 100)
            .iter()
            .map(|hit| hit.title.as_str())
            .collect::<Vec<_>>(),
        ["Note 2", "Note 10"]
    );
}

#[test]
fn moving_a_group_preserves_its_sort_selection_history_and_index_scope_on_reopen() {
    let root = fixture();
    let mut session = session(root.path());
    let g = group(&session, "A/G");
    let parent = Parent::Group(g);
    let destination = section(&session, "B");
    run(
        &mut session,
        Op::SetSort {
            group: g,
            mode: "title".into(),
        },
    );
    run(&mut session, Op::SelectParent { parent });
    let id = session.document().unwrap().note;
    run(
        &mut session,
        Op::MoveGroup {
            group: g,
            to: destination,
        },
    );
    assert_eq!(session.selected_parent(), Some(parent));
    assert_eq!(session.document().unwrap().note, id);
    assert_eq!(titles(&session, g), ["Note 2", "Note 10"]);
    assert_eq!(session.state().selection(), Some(rel("B/G")));
    assert_eq!(session.state().sort(&rel("B/G")), "title");
    assert_eq!(
        session.index().query(
            "first body",
            Scope::Section(destination),
            QueryKind::Search,
            100
        )[0]
        .note,
        id
    );
    drop(session);
    let reopened = self::session(root.path());
    assert_eq!(
        reopened
            .model()
            .parent_path(reopened.selected_parent().unwrap()),
        Some(&rel("B/G"))
    );
    assert_eq!(reopened.document().unwrap().path(), &rel("B/G/Note 2.md"));
    assert_eq!(reopened.state().sort(&rel("B/G")), "title");
}

#[test]
fn cross_section_group_move_appends_to_complete_manual_order_and_persists_remapped_state() {
    let root = group_move_fixture();
    let metadata_path = root.path().join(".wren/workspace.toml");
    let original: toml::Table = fs::read_to_string(&metadata_path).unwrap().parse().unwrap();
    let mut current = session(root.path());
    let moved = group(&current, "Alpha/C");
    let active = note(&current, "Alpha/C/Note 2.md");
    let deep = note(&current, "Alpha/C/deep/ZDeep.md");
    let previous = note(&current, "Alpha/Source 2/Stay.md");
    let destination = section(&current, "Beta");
    assert_eq!(current.state().order()[0], rel("Alpha/C"));
    assert_eq!(
        group_names(&current, destination),
        [
            "Visible Z",
            "Hidden Z",
            "Visible A",
            "Hidden A",
            "Foreign 2",
            "Foreign 10"
        ]
    );
    open(&mut current, previous);
    run(
        &mut current,
        Op::SelectParent {
            parent: Parent::Group(moved),
        },
    );
    edit(&mut current, active, "saved active group needle");
    let text = current.document().unwrap().text.as_ptr();
    let events = run(
        &mut current,
        Op::MoveGroup {
            group: moved,
            to: destination,
        },
    );
    assert_appended_group_order(&current);
    assert_eq!(group(&current, "Beta/C"), moved);
    assert_eq!(note(&current, "Beta/C/Note 2.md"), active);
    assert_eq!(note(&current, "Beta/C/deep/ZDeep.md"), deep);
    assert_eq!(current.selected_parent(), Some(Parent::Group(moved)));
    assert_eq!(current.document().unwrap().text.as_ptr(), text);
    assert!(!current.document().unwrap().dirty());
    assert_eq!(current.document().unwrap().path(), &rel("Beta/C/Note 2.md"));
    assert_eq!(current.state().selection(), Some(rel("Beta/C")));
    assert_eq!(
        current.state().last_open_note(),
        Some(rel("Beta/C/Note 2.md"))
    );
    assert_eq!(current.state().sort(&rel("Beta/C")), "title");
    assert_eq!(
        current.state().history(),
        [rel("Alpha/Source 2/Stay.md"), rel("Beta/C/Note 2.md")]
    );
    assert_eq!(current.state().history_position(), Some(1));
    assert_eq!(current.state().recent(), current.state().history());
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::NoteSaved { note, .. } if *note == active)));
    assert!(events.iter().any(|event| matches!(event, Event::NoteMoved { note, new_path } if *note == deep && new_path == Path::new("Beta/C/deep/ZDeep.md"))));
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::Selected(_))));
    let hit = &current.index().query(
        "saved active group needle",
        Scope::Section(destination),
        QueryKind::Search,
        100,
    )[0];
    assert_eq!(hit.note, active);
    assert_eq!(hit.path, rel("Beta/C/Note 2.md"));
    assert!(current
        .index()
        .query(
            "group needle",
            Scope::Section(section(&current, "Alpha")),
            QueryKind::Search,
            100,
        )
        .is_empty());
    assert_eq!(
        fs::read_to_string(root.path().join("Beta/C/Note 2.md")).unwrap(),
        "saved active group needle"
    );
    let saved: toml::Table = fs::read_to_string(&metadata_path).unwrap().parse().unwrap();
    for key in ["future", "widths"] {
        assert_eq!(saved[key], original[key]);
    }
    let order = current.state().order();
    drop(current);
    let mut reopened = session(root.path());
    assert_appended_group_order(&reopened);
    assert_eq!(reopened.state().order(), order);
    assert_eq!(
        reopened.document().unwrap().path(),
        &rel("Beta/C/Note 2.md")
    );
    assert_eq!(reopened.state().selection(), Some(rel("Beta/C")));
    assert_eq!(reopened.state().sort(&rel("Beta/C")), "title");
    run(&mut reopened, Op::Back);
    assert_eq!(
        reopened.document().unwrap().path(),
        &rel("Alpha/Source 2/Stay.md")
    );
    run(&mut reopened, Op::Forward);
    assert_eq!(
        reopened.document().unwrap().path(),
        &rel("Beta/C/Note 2.md")
    );
}

#[test]
fn cross_section_group_move_preserves_natural_fallback_order_without_writable_metadata() {
    for metadata in [None, Some("layout = [ broken")] {
        let root = group_move_fixture();
        let metadata_path = root.path().join(".wren/workspace.toml");
        if let Some(source) = metadata {
            fs::write(&metadata_path, source).unwrap();
        } else {
            fs::remove_file(&metadata_path).unwrap();
        }
        let mut current = session(root.path());
        let moved = group(&current, "Alpha/C");
        let destination = section(&current, "Beta");
        assert_eq!(
            group_names(&current, destination),
            [
                "Foreign 2",
                "Foreign 10",
                "Hidden A",
                "Hidden Z",
                "Visible A",
                "Visible Z"
            ]
        );
        run(
            &mut current,
            Op::MoveGroup {
                group: moved,
                to: destination,
            },
        );
        let expected = [
            "Foreign 2",
            "Foreign 10",
            "Hidden A",
            "Hidden Z",
            "Visible A",
            "Visible Z",
            "C",
        ];
        assert_eq!(group_names(&current, destination), expected);
        run(&mut current, Op::Reconcile);
        assert_eq!(group_names(&current, destination), expected);
        if let Some(source) = metadata {
            assert!(!current.state().writable());
            assert_eq!(fs::read_to_string(&metadata_path).unwrap(), source);
        } else {
            drop(current);
            let reopened = session(root.path());
            assert_eq!(group_names(&reopened, section(&reopened, "Beta")), expected);
        }
    }
}

#[test]
fn same_section_group_moves_and_renames_keep_their_manual_position() {
    let root = group_move_fixture();
    let mut current = session(root.path());
    let moved = group(&current, "Beta/Hidden Z");
    let destination = section(&current, "Beta");
    let original = current.state().order();
    for op in [
        Op::MoveGroup {
            group: moved,
            to: destination,
        },
        Op::Rename {
            target: ItemRef::Group(moved),
            new_name: "Hidden Z".into(),
        },
    ] {
        run(&mut current, op);
        assert_eq!(current.state().order(), original);
    }
    run(
        &mut current,
        Op::Rename {
            target: ItemRef::Group(moved),
            new_name: "A renamed".into(),
        },
    );
    let expected = [
        "Visible Z",
        "A renamed",
        "Visible A",
        "Hidden A",
        "Foreign 2",
        "Foreign 10",
    ];
    assert_eq!(group_names(&current, destination), expected);
    assert_eq!(
        current.state().order(),
        original
            .into_iter()
            .map(|path| if path == rel("Beta/Hidden Z") {
                rel("Beta/A renamed")
            } else {
                path
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(group(&current, "Beta/A renamed"), moved);
    drop(current);
    let reopened = session(root.path());
    assert_eq!(group_names(&reopened, section(&reopened, "Beta")), expected);
}

#[test]
fn cross_section_group_move_prepublish_failures_leave_order_and_paths_unchanged() {
    for fault in [Fault::BeforePublish, Fault::Write] {
        let root = group_move_fixture();
        let mut current = session(root.path());
        let moved = group(&current, "Alpha/C");
        let active = note(&current, "Alpha/C/Note 2.md");
        let destination = section(&current, "Beta");
        open(&mut current, active);
        if fault == Fault::Write {
            edit(&mut current, active, "dirty barrier must block move");
        }
        let text = current.document().unwrap().text.as_ptr();
        let order = current.state().order();
        let groups = current.model().groups.clone();
        let history = current.state().history();
        let recent = current.state().recent();
        let metadata = fs::read(root.path().join(".wren/workspace.toml")).unwrap();
        current.fail_next(fault);
        let error = current
            .apply_at(
                Op::MoveGroup {
                    group: moved,
                    to: destination,
                },
                Duration::ZERO,
            )
            .unwrap_err();
        assert!(error.message.contains(&format!("{fault:?}")));
        assert!(error.events.is_empty());
        assert_eq!(current.state().order(), order);
        assert_eq!(current.state().history(), history);
        assert_eq!(current.state().recent(), recent);
        assert_eq!(current.state().selection(), Some(rel("Alpha/C/Note 2.md")));
        assert_eq!(current.selected_parent(), Some(Parent::Group(moved)));
        assert_eq!(current.document().unwrap().text.as_ptr(), text);
        assert_eq!(
            current.document().unwrap().path(),
            &rel("Alpha/C/Note 2.md")
        );
        assert_eq!(current.document().unwrap().dirty(), fault == Fault::Write);
        assert_eq!(
            current
                .model()
                .groups
                .iter()
                .map(|group| (group.id, group.section, &group.path))
                .collect::<Vec<_>>(),
            groups
                .iter()
                .map(|group| (group.id, group.section, &group.path))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            fs::read(root.path().join(".wren/workspace.toml")).unwrap(),
            metadata
        );
        assert!(root.path().join("Alpha/C").is_dir());
        assert!(!root.path().join("Beta/C").exists());
        assert_eq!(
            current.index().query(
                "group needle",
                Scope::Section(destination),
                QueryKind::Search,
                100,
            ),
            []
        );
    }
}

#[test]
fn cross_section_group_move_postpublication_faults_keep_appended_order_before_retry() {
    for fault in [
        Fault::DirectorySync,
        Fault::Write,
        Fault::Permissions,
        Fault::FileSync,
    ] {
        let root = group_move_fixture();
        let mut current = session(root.path());
        let moved = group(&current, "Alpha/C");
        let active = note(&current, "Alpha/C/Note 2.md");
        let destination = section(&current, "Beta");
        open(&mut current, active);
        let text = current.document().unwrap().text.as_ptr();
        current.fail_next(fault);
        let error = current
            .apply_at(
                Op::MoveGroup {
                    group: moved,
                    to: destination,
                },
                Duration::ZERO,
            )
            .unwrap_err();
        assert!(error.message.contains(&format!("{fault:?}")));
        assert!(error.events.iter().any(|event| matches!(event, Event::NoteMoved { note, new_path } if *note == active && new_path == Path::new("Beta/C/Note 2.md"))));
        assert!(error
            .events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged)));
        assert!(!error
            .events
            .iter()
            .any(|event| matches!(event, Event::Selected(_))));
        assert_appended_group_order(&current);
        assert_eq!(group(&current, "Beta/C"), moved);
        assert_eq!(current.document().unwrap().text.as_ptr(), text);
        assert_eq!(current.document().unwrap().path(), &rel("Beta/C/Note 2.md"));
        assert_eq!(current.state().selection(), Some(rel("Beta/C/Note 2.md")));
        assert_eq!(current.state().history(), [rel("Beta/C/Note 2.md")]);
        assert_eq!(hits(&current, "active group needle"), [active]);
        assert!(!root.path().join("Alpha/C").exists());
        assert!(root.path().join("Beta/C").is_dir());
        let order = current.state().order();
        run(&mut current, Op::Reconcile);
        run(&mut current, Op::Flush);
        run(
            &mut current,
            Op::SetLayout {
                layout: "outline".into(),
            },
        );
        assert_appended_group_order(&current);
        assert_eq!(current.state().order(), order);
        assert!(!current.pending_durability());
        assert!(!root.path().join("Beta/C 2").exists());
        drop(current);
        let reopened = session(root.path());
        assert_appended_group_order(&reopened);
        assert_eq!(
            reopened.document().unwrap().path(),
            &rel("Beta/C/Note 2.md")
        );
    }
}

#[test]
fn cross_section_group_move_metadata_conflicts_keep_committed_order_without_overwriting_disk() {
    let root = group_move_fixture();
    let metadata_path = root.path().join(".wren/workspace.toml");
    let mut current = session(root.path());
    let moved = group(&current, "Alpha/C");
    let active = note(&current, "Alpha/C/Note 2.md");
    let destination = section(&current, "Beta");
    open(&mut current, active);
    let mut external = fs::read_to_string(&metadata_path).unwrap();
    external.push_str("\n[external]\npreserve = true\n");
    fs::write(&metadata_path, &external).unwrap();
    let error = current
        .apply_at(
            Op::MoveGroup {
                group: moved,
                to: destination,
            },
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.message.contains("workspace.toml"));
    assert!(error.events.iter().any(
        |event| matches!(event, Event::Warning(message) if message.contains("remain applied"))
    ));
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::NoteMoved { note, .. } if *note == active)));
    assert_appended_group_order(&current);
    assert_eq!(group(&current, "Beta/C"), moved);
    assert_eq!(current.document().unwrap().path(), &rel("Beta/C/Note 2.md"));
    assert_eq!(current.state().history(), [rel("Beta/C/Note 2.md")]);
    assert_eq!(hits(&current, "active group needle"), [active]);
    let order = current.state().order();
    run(&mut current, Op::Reconcile);
    assert!(current
        .apply_at(
            Op::SetLayout {
                layout: "outline".into(),
            },
            Duration::ZERO,
        )
        .is_err());
    assert_eq!(current.state().order(), order);
    assert_appended_group_order(&current);
    assert_eq!(fs::read_to_string(&metadata_path).unwrap(), external);
    assert!(!root.path().join("Alpha/C").exists());
    assert!(root.path().join("Beta/C").is_dir());
    assert!(!root.path().join("Beta/C 2").exists());
}

#[test]
fn jump_recency_updates_for_back_forward_and_restores_from_history() {
    let root = fixture();
    let mut session = session(root.path());
    let first = note(&session, "A/G/Note 2.md");
    let second = note(&session, "A/G/Note 10.md");
    open(&mut session, first);
    open(&mut session, second);
    assert_eq!(
        session
            .index()
            .query("", Scope::Workspace, QueryKind::Jump, 1)[0]
            .note,
        second
    );
    run(&mut session, Op::Back);
    assert_eq!(
        session
            .index()
            .query("", Scope::Workspace, QueryKind::Jump, 1)[0]
            .note,
        first
    );
    run(&mut session, Op::Forward);
    assert_eq!(
        session
            .index()
            .query("", Scope::Workspace, QueryKind::Jump, 1)[0]
            .note,
        second
    );
    drop(session);
    let reopened = self::session(root.path());
    assert_eq!(
        reopened
            .index()
            .query("", Scope::Workspace, QueryKind::Jump, 1)[0]
            .path,
        rel("A/G/Note 10.md")
    );
}

#[test]
fn group_selection_restores_its_last_note_rather_than_the_new_first_sorted_note() {
    let root = fixture();
    write(root.path(), "A/L/Loose 2.md", "another loose note");
    write(
        root.path(),
        ".wren/workspace.toml",
        "selection = 'A/L'\nlast_open_note = 'A/L/Loose.md'\n",
    );
    set_modified(root.path(), "A/L/Loose.md", 100);
    set_modified(root.path(), "A/L/Loose 2.md", 200);
    let session = session(root.path());
    let l = group(&session, "A/L");
    assert_eq!(titles(&session, l), ["Loose 2", "Loose"]);
    assert_eq!(session.document().unwrap().path(), &rel("A/L/Loose.md"));
    assert_eq!(session.selected_parent(), Some(Parent::Group(l)));
}

#[test]
fn a_persisted_section_selection_restores_to_its_first_group() {
    let root = fixture();
    write(
        root.path(),
        ".wren/workspace.toml",
        "selection = 'A'\nlast_open_note = 'A/G/Note 10.md'\n",
    );
    let session = session(root.path());
    let g = group(&session, "A/G");
    assert_eq!(session.selected_parent(), Some(Parent::Group(g)));
    assert_eq!(session.document().unwrap().path(), &rel("A/G/Note 10.md"));
}

#[test]
fn unchanged_reconciliation_after_manual_and_title_sort_is_quiet() {
    let root = fixture();
    let mut session = session(root.path());
    let g = group(&session, "A/G");
    run(
        &mut session,
        Op::SetSort {
            group: g,
            mode: "title".into(),
        },
    );
    let a = section(&session, "A");
    let b = section(&session, "B");
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Section(b),
            before: Some(ItemRef::Section(a)),
        },
    );
    let revision = session.index().revision();
    for _ in 0..3 {
        assert!(run(&mut session, Op::Reconcile).is_empty());
        assert_eq!(session.index().revision(), revision);
    }
}

#[test]
fn partial_delete_commit_removes_index_content_even_when_metadata_cannot_be_saved() {
    let root = fixture();
    let mut session = session(root.path());
    let id = note(&session, "A/G/Note 2.md");
    open(&mut session, id);
    write(root.path(), ".wren/workspace.toml", "layout = 'outline'\n");
    let error = session
        .apply_at(
            Op::Delete {
                target: ItemRef::Note(id),
            },
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::TreeChanged)));
    assert!(session.index().content(id).is_none());
    assert!(session
        .index()
        .query("Note 2", Scope::Workspace, QueryKind::Jump, 100)
        .is_empty());
    assert!(hits(&session, "first body").is_empty());
    assert!(!root.path().join("A/G/Note 2.md").exists());
    assert_eq!(
        fs::read_to_string(root.path().join(".wren/workspace.toml")).unwrap(),
        "layout = 'outline'\n"
    );
}

#[test]
fn invalid_utf8_at_load_preserves_title_and_surfaces_a_non_repeating_limitation() {
    let root = fixture();
    write(root.path(), "A/G/Invalid.md", [0xff]);
    let mut session = session(root.path());
    let id = note(&session, "A/G/Invalid.md");
    assert_eq!(session.index().len(), 5);
    assert_eq!(session.index().content(id), None);
    assert!(session.index().limitation(id).unwrap().contains("UTF-8"));
    assert!(
        session
            .model()
            .warnings
            .iter()
            .any(|warning| warning.message.contains("Invalid.md")
                && warning.message.contains("UTF-8"))
    );
    assert_eq!(
        session
            .index()
            .query("Invalid", Scope::Workspace, QueryKind::Jump, 100)[0]
            .note,
        id
    );
    assert!(run(&mut session, Op::Reconcile).is_empty());
    assert!(
        session
            .model()
            .warnings
            .iter()
            .any(|warning| warning.message.contains("Invalid.md")
                && warning.message.contains("UTF-8"))
    );
    assert_eq!(
        fs::read(root.path().join("A/G/Invalid.md")).unwrap(),
        [0xff]
    );
}

fn recent_paths(session: &Session, count: usize) -> Vec<RelPath> {
    session
        .index()
        .query("", Scope::Workspace, QueryKind::Jump, count)
        .into_iter()
        .map(|hit| hit.path)
        .collect()
}

#[test]
fn history_branching_retains_independent_mru_order_on_restart_and_same_note_open_is_not_a_visit() {
    let root = fixture();
    let mut session = session(root.path());
    let a = note(&session, "A/G/Note 2.md");
    let b = note(&session, "A/G/Note 10.md");
    let c = note(&session, "A/L/Loose.md");
    open(&mut session, a);
    open(&mut session, b);
    run(&mut session, Op::Back);
    open(&mut session, c);
    assert_eq!(
        session.state().history(),
        [rel("A/G/Note 2.md"), rel("A/L/Loose.md")]
    );
    assert_eq!(session.state().history_position(), Some(1));
    assert_eq!(
        session.state().recent(),
        [
            rel("A/G/Note 10.md"),
            rel("A/G/Note 2.md"),
            rel("A/L/Loose.md")
        ]
    );
    let expected = vec![
        rel("A/L/Loose.md"),
        rel("A/G/Note 2.md"),
        rel("A/G/Note 10.md"),
    ];
    assert_eq!(recent_paths(&session, 3), expected);
    let revision = session.index().revision();
    assert!(run(&mut session, Op::OpenNote { note: Some(c) }).is_empty());
    assert_eq!(session.index().revision(), revision);
    assert_eq!(session.state().history().len(), 2);
    drop(session);
    let reopened = self::session(root.path());
    assert_eq!(recent_paths(&reopened, 3), expected);
    assert_eq!(reopened.state().history_position(), Some(1));
    assert!(!reopened.can_forward());
}

#[test]
fn clean_back_and_forward_commit_history_position_and_mru_before_optional_metadata_faults() {
    for fault in [
        Fault::Write,
        Fault::Permissions,
        Fault::FileSync,
        Fault::BeforePublish,
        Fault::DirectorySync,
    ] {
        for forward in [false, true] {
            let root = fixture();
            let mut session = session(root.path());
            let a = note(&session, "A/G/Note 2.md");
            let b = note(&session, "A/G/Note 10.md");
            open(&mut session, a);
            open(&mut session, b);
            if forward {
                run(&mut session, Op::Back);
            }
            let target = if forward { b } else { a };
            let position = usize::from(forward);
            assert!(!session.document().unwrap().dirty());
            session.fail_next(fault);
            let error = session
                .apply_at(if forward { Op::Forward } else { Op::Back }, Duration::ZERO)
                .unwrap_err();
            assert!(error
                .events
                .iter()
                .any(|event| matches!(event, Event::Selected(Some(note)) if *note == target)));
            assert_eq!(
                session.document().unwrap().note,
                target,
                "{fault:?}, forward={forward}"
            );
            assert_eq!(session.state().history_position(), Some(position));
            assert_eq!(session.can_back(), forward);
            assert_eq!(session.can_forward(), !forward);
            assert_eq!(
                recent_paths(&session, 1)[0],
                session.model().note(target).unwrap().path
            );
            run(&mut session, if forward { Op::Back } else { Op::Forward });
            assert_eq!(session.state().history_position(), Some(1 - position));
            assert_eq!(session.state().history().len(), 2);
            drop(session);
            let reopened = self::session(root.path());
            assert_eq!(reopened.state().history_position(), Some(1 - position));
            assert_eq!(reopened.can_back(), !forward);
            assert_eq!(reopened.can_forward(), forward);
        }
    }
}

#[test]
fn real_metadata_permission_failure_after_clean_back_does_not_desynchronize_the_pointer() {
    let root = fixture();
    let mut session = session(root.path());
    let a = note(&session, "A/G/Note 2.md");
    let b = note(&session, "A/G/Note 10.md");
    open(&mut session, a);
    open(&mut session, b);
    let directory = root.path().join(".wren");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
    let result = session.apply_at(Op::Back, Duration::ZERO);
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    let error = result.expect_err("The permission test must run unprivileged");
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::Selected(Some(note)) if *note == a)));
    assert_eq!(session.document().unwrap().note, a);
    assert_eq!(session.state().history_position(), Some(0));
    assert!(!session.can_back());
    assert!(session.can_forward());
    run(&mut session, Op::Forward);
    assert_eq!(session.document().unwrap().note, b);
}

#[test]
fn failed_history_flushes_and_failed_target_loads_do_not_advance_visits_or_mru() {
    for forward in [false, true] {
        for fault in [
            Fault::Write,
            Fault::Permissions,
            Fault::FileSync,
            Fault::BeforePublish,
            Fault::DirectorySync,
        ] {
            let root = fixture();
            let mut session = session(root.path());
            let a = note(&session, "A/G/Note 2.md");
            let b = note(&session, "A/G/Note 10.md");
            open(&mut session, a);
            open(&mut session, b);
            if forward {
                run(&mut session, Op::Back);
            }
            let active = session.document().unwrap().note;
            edit(&mut session, active, "retained dirty draft");
            let history = session.state().history();
            let position = session.state().history_position();
            let recent = session.state().recent();
            let ranking = recent_paths(&session, 4);
            session.fail_next(fault);
            assert!(session
                .apply_at(if forward { Op::Forward } else { Op::Back }, Duration::ZERO)
                .is_err());
            assert_eq!(session.document().unwrap().note, active);
            assert!(session.document().unwrap().dirty());
            assert_eq!(session.state().history(), history);
            assert_eq!(session.state().history_position(), position);
            assert_eq!(session.state().recent(), recent);
            assert_eq!(recent_paths(&session, 4), ranking);
        }
    }
    let root = fixture();
    let mut session = session(root.path());
    let a = note(&session, "A/G/Note 2.md");
    let b = note(&session, "A/G/Note 10.md");
    open(&mut session, a);
    open(&mut session, b);
    let recent = session.state().recent();
    write(root.path(), "A/G/Note 2.md", [0xff]);
    assert!(session.apply_at(Op::Back, Duration::ZERO).is_err());
    assert_eq!(session.document().unwrap().note, b);
    assert_eq!(session.state().history_position(), Some(1));
    assert_eq!(session.state().recent(), recent);
    assert!(!session.can_forward());
    assert!(session
        .apply_at(Op::OpenNote { note: Some(a) }, Duration::ZERO)
        .is_err());
    assert_eq!(session.state().history().len(), 2);
    assert_eq!(session.state().recent(), recent);
}

#[test]
fn duplicate_visit_occurrence_and_forward_position_survive_restart_and_unchanged_reconcile() {
    let root = fixture();
    let mut session = session(root.path());
    let a = note(&session, "A/G/Note 2.md");
    let b = note(&session, "A/G/Note 10.md");
    let c = note(&session, "A/L/Loose.md");
    for id in [a, b, a, c] {
        open(&mut session, id);
    }
    for _ in 0..3 {
        run(&mut session, Op::Back);
    }
    assert_eq!(session.document().unwrap().note, a);
    assert_eq!(session.state().history_position(), Some(0));
    let recent = recent_paths(&session, 3);
    run(&mut session, Op::Reconcile);
    assert_eq!(session.state().history_position(), Some(0));
    assert!(!session.can_back());
    drop(session);
    let mut reopened = self::session(root.path());
    assert_eq!(reopened.state().history_position(), Some(0));
    assert_eq!(recent_paths(&reopened, 3), recent);
    assert!(!reopened.can_back());
    assert!(reopened.can_forward());
    for (position, path) in [
        (1, "A/G/Note 10.md"),
        (2, "A/G/Note 2.md"),
        (3, "A/L/Loose.md"),
    ] {
        run(&mut reopened, Op::Forward);
        assert_eq!(reopened.state().history_position(), Some(position));
        assert_eq!(reopened.document().unwrap().path(), &rel(path));
    }
    assert!(!reopened.can_forward());
}

#[test]
fn pruned_entries_remap_the_selected_occurrence_instead_of_finding_the_last_duplicate() {
    let root = fixture();
    for path in ["A/L/Missing 1.md", "A/L/Missing 2.md"] {
        write(root.path(), path, "will be deleted");
    }
    let mut session = session(root.path());
    let a = note(&session, "A/G/Note 2.md");
    let b = note(&session, "A/G/Note 10.md");
    let c = note(&session, "A/L/Loose.md");
    let missing1 = note(&session, "A/L/Missing 1.md");
    let missing2 = note(&session, "A/L/Missing 2.md");
    for id in [missing1, a, missing2, b, a, c] {
        open(&mut session, id);
    }
    for _ in 0..4 {
        run(&mut session, Op::Back);
    }
    assert_eq!(session.state().history_position(), Some(1));
    drop(session);
    fs::remove_file(root.path().join("A/L/Missing 1.md")).unwrap();
    fs::remove_file(root.path().join("A/L/Missing 2.md")).unwrap();
    let mut reopened = self::session(root.path());
    assert_eq!(reopened.state().history_position(), Some(0));
    assert!(!reopened.can_back());
    assert_eq!(
        reopened.state().history(),
        [
            rel("A/G/Note 2.md"),
            rel("A/G/Note 10.md"),
            rel("A/G/Note 2.md"),
            rel("A/L/Loose.md")
        ]
    );
    assert!(!reopened
        .state()
        .recent()
        .iter()
        .any(|path| path.as_path().to_string_lossy().contains("Missing")));
    let b = note(&reopened, "A/G/Note 10.md");
    run(
        &mut reopened,
        Op::Delete {
            target: ItemRef::Note(b),
        },
    );
    assert_eq!(reopened.state().history_position(), Some(0));
    let ptr = reopened.document().unwrap().text.as_ptr();
    let revision = reopened.index().revision();
    run(&mut reopened, Op::Forward);
    assert_eq!(reopened.document().unwrap().text.as_ptr(), ptr);
    assert_eq!(reopened.document().unwrap().path(), &rel("A/G/Note 2.md"));
    assert_eq!(reopened.state().history_position(), Some(1));
    assert!(
        reopened.index().revision() > revision,
        "Back/Forward to another occurrence still counts as a visit"
    );
    drop(reopened);
    let reopened = self::session(root.path());
    assert_eq!(reopened.state().history_position(), Some(1));
    assert!(reopened.can_back() && reopened.can_forward());
}

#[test]
fn deleting_the_selected_history_entry_prunes_visits_and_mru_without_losing_forward_navigation() {
    let root = fixture();
    let mut session = session(root.path());
    let a = note(&session, "A/G/Note 2.md");
    let b = note(&session, "A/G/Note 10.md");
    let c = note(&session, "A/L/Loose.md");
    for id in [a, b, a, c] {
        open(&mut session, id);
    }
    run(&mut session, Op::Back);
    assert_eq!(session.state().history_position(), Some(2));
    fs::remove_file(root.path().join("A/G/Note 2.md")).unwrap();
    run(&mut session, Op::Reconcile);
    assert!(session.document().is_none());
    assert_eq!(
        session.state().history(),
        [rel("A/G/Note 10.md"), rel("A/L/Loose.md")]
    );
    assert_eq!(session.state().history_position(), Some(0));
    assert_eq!(
        session.state().recent(),
        [rel("A/G/Note 10.md"), rel("A/L/Loose.md")]
    );
    assert!(!session.can_back());
    assert!(session.can_forward());
    drop(session);
    let mut reopened = self::session(root.path());
    assert!(reopened.document().is_none());
    assert_eq!(reopened.state().history_position(), Some(0));
    assert!(reopened.can_forward());
    run(&mut reopened, Op::Forward);
    assert_eq!(reopened.document().unwrap().path(), &rel("A/L/Loose.md"));
}

#[test]
fn mru_and_the_current_duplicate_occurrence_follow_note_group_and_section_moves() {
    let root = fixture();
    let mut session = session(root.path());
    let a = note(&session, "A/G/Note 2.md");
    let b = note(&session, "A/G/Note 10.md");
    let c = note(&session, "A/L/Loose.md");
    for id in [a, b, a, c] {
        open(&mut session, id);
    }
    run(&mut session, Op::Back);
    let g = group(&session, "A/G");
    let destination = section(&session, "B");
    run(
        &mut session,
        Op::MoveGroup {
            group: g,
            to: destination,
        },
    );
    run(
        &mut session,
        Op::Rename {
            target: ItemRef::Section(destination),
            new_name: "Archive".into(),
        },
    );
    let to = group(&session, "A/L");
    run(&mut session, Op::MoveNote { note: b, to });
    assert_eq!(session.state().history_position(), Some(2));
    assert_eq!(
        session.state().recent(),
        [
            rel("A/L/Note 10.md"),
            rel("A/L/Loose.md"),
            rel("Archive/G/Note 2.md")
        ]
    );
    drop(session);
    let mut reopened = self::session(root.path());
    assert_eq!(reopened.state().history_position(), Some(2));
    assert_eq!(
        recent_paths(&reopened, 3),
        [
            rel("Archive/G/Note 2.md"),
            rel("A/L/Loose.md"),
            rel("A/L/Note 10.md")
        ]
    );
    run(&mut reopened, Op::Back);
    assert_eq!(reopened.document().unwrap().path(), &rel("A/L/Note 10.md"));
}

#[test]
fn navigation_retention_is_capped_for_visits_mru_and_loaded_overlong_metadata() {
    let root = TempDir::new().unwrap();
    for number in 0..120 {
        write(root.path(), &format!("S/G/{number}.md"), "body");
    }
    let mut session = session(root.path());
    for number in 0..120 {
        let id = note(&session, &format!("S/G/{number}.md"));
        open(&mut session, id);
    }
    assert_eq!(session.state().history().len(), 100);
    assert_eq!(session.state().recent().len(), 100);
    assert_eq!(session.state().history()[0], rel("S/G/20.md"));
    assert_eq!(session.state().history_position(), Some(99));
    let ranking = recent_paths(&session, 120);
    drop(session);
    let reopened = self::session(root.path());
    assert_eq!(recent_paths(&reopened, 120), ranking);
    drop(reopened);
    let paths = (0..120)
        .map(|number| format!("'S/G/{number}.md'"))
        .collect::<Vec<_>>()
        .join(",");
    write(root.path(), ".wren/workspace.toml", format!("history = [{paths}]\nhistory_position = 10\nrecent = [{paths}]\nselection = 'S/G/10.md'\n"));
    let reopened = self::session(root.path());
    assert_eq!(reopened.state().history().len(), 100);
    assert_eq!(reopened.state().history_position(), Some(0));
    assert_eq!(reopened.document().unwrap().path(), &rel("S/G/10.md"));
    assert_eq!(reopened.state().recent().len(), 100);
    assert_eq!(reopened.state().recent().last(), Some(&rel("S/G/119.md")));
}

#[test]
fn corrupt_new_navigation_fields_leave_metadata_read_only_and_never_damage_markdown() {
    for source in [
        "history_position = 0",
        "history = ['A/L/Loose.md']\nhistory_position = -1",
        "history = ['A/L/Loose.md']\nhistory_position = 1",
        "history = ['A/L/Loose.md']\nhistory_position = '0'",
        "recent = 'A/L/Loose.md'",
        "recent = ['A/L/Loose.md', 'A/L/Loose.md']",
        "recent = ['../escape.md']",
    ] {
        let root = fixture();
        write(root.path(), ".wren/workspace.toml", source);
        let mut session = session(root.path());
        assert!(!session.state().writable(), "{source}");
        let id = note(&session, "A/L/Loose.md");
        open(&mut session, id);
        run(
            &mut session,
            Op::WriteNote {
                note: id,
                contents: "safe changed Markdown".into(),
            },
        );
        assert_eq!(
            fs::read_to_string(root.path().join("A/L/Loose.md")).unwrap(),
            "safe changed Markdown"
        );
        assert_eq!(
            fs::read_to_string(root.path().join(".wren/workspace.toml")).unwrap(),
            source
        );
    }
}

#[test]
fn exact_empty_section_selection_stays_selected_through_sort_preferences_and_reopen() {
    let root = fixture();
    fs::create_dir(root.path().join("Empty")).unwrap();
    let mut session = session(root.path());
    let g = group(&session, "A/G");
    let empty = section(&session, "Empty");
    let other = note(&session, "B/H/Other.md");
    open(&mut session, other);
    let parent = Parent::Section(empty);
    run(&mut session, Op::SelectParent { parent });
    assert!(session.document().is_none());
    assert_eq!(session.selected_parent(), Some(parent));
    assert_eq!(session.state().selection(), Some(rel("Empty")));
    assert_eq!(session.state().last_open_note(), None);
    for mode in ["title", "modified"] {
        run(
            &mut session,
            Op::SetSort {
                group: g,
                mode: mode.into(),
            },
        );
        assert!(session.document().is_none());
        assert_eq!(session.selected_parent(), Some(parent));
    }
    drop(session);
    let mut reopened = self::session(root.path());
    let restored = Parent::Section(section(&reopened, "Empty"));
    assert!(reopened.document().is_none());
    assert_eq!(reopened.selected_parent(), Some(restored));
    let g = group(&reopened, "A/G");
    run(
        &mut reopened,
        Op::SetSort {
            group: g,
            mode: "title".into(),
        },
    );
    assert!(reopened.document().is_none());
    assert_eq!(reopened.selected_parent(), Some(restored));
    let section_b = section(&reopened, "B");
    run(&mut reopened, Op::SelectSection { section: section_b });
    assert_eq!(reopened.document().unwrap().path(), &rel("B/H/Other.md"));
}

#[test]
fn section_row_selects_an_empty_section_and_obeys_the_existing_flush_barrier() {
    let root = fixture();
    fs::create_dir(root.path().join("Empty")).unwrap();
    let mut session = session(root.path());
    let active = note(&session, "A/L/Loose.md");
    open(&mut session, active);
    edit(&mut session, active, "retained at section barrier");
    let empty = section(&session, "Empty");
    let recent = session.state().recent();
    session.fail_next(Fault::Write);
    assert!(session
        .apply_at(Op::SelectSection { section: empty }, Duration::ZERO)
        .is_err());
    assert_eq!(session.document().unwrap().note, active);
    assert_eq!(session.state().recent(), recent);
    run(&mut session, Op::SelectSection { section: empty });
    assert!(session.document().is_none());
    assert_eq!(session.selected_parent(), Some(Parent::Section(empty)));
    assert_eq!(session.state().selection(), Some(rel("Empty")));
}

#[test]
fn body_preview_does_not_replace_the_deep_path_subtitle() {
    let root = fixture();
    write(
        root.path(),
        "A/G/deep/Deep.md",
        "# Heading\r\n\r\nFirst body line\r\nSecond body line",
    );
    let session = session(root.path());
    let id = note(&session, "A/G/deep/Deep.md");
    assert_eq!(
        session.model().note(id).unwrap().subtitle.as_deref(),
        Some("deep/Deep.md")
    );
    assert_eq!(session.index().preview(id), Some("First body line"));
    let hit = &session
        .index()
        .query("Deep", Scope::Workspace, QueryKind::Jump, 10)[0];
    assert_eq!(hit.snippet.text, "First body line");
}

#[test]
fn unreadable_subtree_reconcile_keeps_model_and_draft_but_invalidates_and_recovers_closed_content()
{
    let root = fixture();
    let mut session = session(root.path());
    let active = note(&session, "B/H/Other.md");
    let closed = note(&session, "A/G/Note 2.md");
    open(&mut session, active);
    edit(&mut session, active, "authoritative unsaved draft");
    let old = session.model().note(closed).unwrap().clone();
    let path = root.path().join("A/G");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let events = run(&mut session, Op::Reconcile);
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(session.model().note(closed).is_some());
    assert_eq!(session.model().note(closed).unwrap().modified, old.modified);
    assert!(
        session.index().content(closed).is_none(),
        "The permission test must run unprivileged"
    );
    assert!(session
        .index()
        .limitation(closed)
        .unwrap()
        .contains("scan incomplete"));
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Warning(message) if message.contains("unavailable"))));
    assert_eq!(
        session.index().content(active),
        Some("authoritative unsaved draft")
    );
    assert!(session.document().unwrap().dirty());
    run(&mut session, Op::Reconcile);
    assert_eq!(session.index().content(closed), Some("first body"));
    assert_eq!(session.index().limitation(closed), None);
    assert_eq!(
        session.document().unwrap().text,
        "authoritative unsaved draft"
    );
}

#[test]
fn root_scan_failure_retains_the_model_and_active_text_and_recovers_index_limitations() {
    let root = fixture();
    let mut session = session(root.path());
    let active = note(&session, "B/H/Other.md");
    let closed = note(&session, "A/G/Note 2.md");
    open(&mut session, active);
    edit(&mut session, active, "live draft during root failure");
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o000)).unwrap();
    let result = session.apply_at(Op::Reconcile, Duration::ZERO);
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err(), "The permission test must run unprivileged");
    assert_eq!(session.model().notes.len(), 4);
    assert_eq!(
        session.index().content(active),
        Some("live draft during root failure")
    );
    assert_eq!(session.index().content(closed), None);
    assert!(session
        .index()
        .limitation(closed)
        .unwrap()
        .contains("scan failed"));
    run(&mut session, Op::Reconcile);
    assert_eq!(session.index().content(closed), Some("first body"));
    assert_eq!(session.index().limitation(closed), None);
    assert_eq!(
        session.document().unwrap().text,
        "live draft during root failure"
    );
    assert!(session.document().unwrap().dirty());
    run(&mut session, Op::Flush);
    assert_eq!(session.document().unwrap().error, None);
}

#[test]
#[ignore = "real-filesystem release workload: 5000 notes / 25 MB; run explicitly with --release"]
fn benchmark_filesystem_5000_notes_25mb_load_reconcile_and_edits() {
    use std::sync::mpsc;
    use std::time::Instant;

    fn report(label: &str, mut samples: Vec<Duration>) {
        samples.sort();
        eprintln!(
            "{label}: {} samples, median {:?}, max {:?}",
            samples.len(),
            samples[samples.len() / 2],
            samples.last().unwrap()
        );
    }

    let root = TempDir::new().unwrap();
    let mut body = "Ordinary work notes, migration planning, follow-up items. ".repeat(100);
    body.truncate(5000);
    for number in 0..5000 {
        write(root.path(), &format!("S/G/Meeting {number}.md"), &body);
    }
    let started = Instant::now();
    let mut session = session(root.path());
    let initial_load = started.elapsed();
    assert_eq!(session.model().notes.len(), 5000);
    assert_eq!(
        session
            .model()
            .notes
            .iter()
            .map(|note| note.bytes)
            .sum::<usize>(),
        25_000_000
    );
    assert_eq!(session.index().retained_bytes(), 50_000_000);
    assert!(session.model().warnings.is_empty());
    let jump = session
        .index()
        .query("mtg42", Scope::Workspace, QueryKind::Jump, 5000);
    assert!(jump.len() >= 40);
    let exact = jump
        .iter()
        .position(|hit| hit.title == "Meeting 42")
        .unwrap();
    assert!(jump
        .iter()
        .all(|hit| hit.score > 0 && hit.content_match.is_none()));
    assert!(jump.windows(2).all(|pair| pair[0].score >= pair[1].score));
    eprintln!("real-filesystem Jump mtg42: {} matches; Meeting 42 rank {}, score {}; top score {} (equal-score ties follow model order)", jump.len(), exact + 1, jump[exact].score, jump[0].score);
    eprintln!("real-filesystem initial load: {initial_load:?}; 5000 notes, 25000000 original bytes, {} retained bytes; streaming scan (one file at a time)", session.index().retained_bytes());

    let active = note(&session, "S/G/Meeting 0.md");
    let inactive = note(&session, "S/G/Meeting 42.md");
    open(&mut session, active);
    let mut unchanged = Vec::new();
    for _ in 0..20 {
        let revision = session.index().revision();
        let started = Instant::now();
        let events = run(&mut session, Op::Reconcile);
        unchanged.push(started.elapsed());
        assert!(events.is_empty());
        assert_eq!(session.index().revision(), revision);
    }
    report("real-filesystem unchanged reconcile", unchanged);

    let path = root.path().join("S/G/Meeting 42.md");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut external_refresh = Vec::new();
    for sample in 0..20 {
        let marker = if sample % 2 == 0 {
            "external alpha"
        } else {
            "external bravo"
        };
        let stale = if sample % 2 == 0 {
            "external bravo"
        } else {
            "external alpha"
        };
        let mut text = body.clone();
        text.replace_range(4200..4200 + marker.len(), marker);
        fs::write(&path, &text).unwrap();
        File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 5000);
        let started = Instant::now();
        let events = run(&mut session, Op::Reconcile);
        external_refresh.push(started.elapsed());
        assert!(!events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged)));
        assert_eq!(hits(&session, marker), [inactive]);
        assert!(hits(&session, stale).is_empty());
        assert_eq!(session.document().unwrap().note, active);
    }
    report(
        "real-filesystem one same-size/restored-mtime external edit refresh",
        external_refresh,
    );

    let mut refreshes = Vec::new();
    let mut edits = Vec::new();
    let mut saves = Vec::new();
    let mut queued_to_saved = Vec::new();
    for sample in 0..20 {
        let preexisting = format!("{body}\nunsaved before refresh {sample}");
        edit(&mut session, active, &preexisting);
        let queued_text = format!("{body}\nqueued during refresh {sample}");
        let (ready_tx, ready_rx) = mpsc::channel();
        let (edit_tx, edit_rx) = mpsc::channel::<(Instant, String)>();
        let timings = std::thread::scope(|scope| {
            let session = &mut session;
            let worker = scope.spawn(move || {
                let started = Instant::now();
                ready_tx.send(()).unwrap();
                run(session, Op::Reconcile);
                let refresh = started.elapsed();
                assert_eq!(session.document().unwrap().text, preexisting);
                assert_eq!(session.index().content(active), Some(preexisting.as_str()));
                assert!(session.document().unwrap().dirty());
                let (queued_at, text) = edit_rx.recv().unwrap();
                let started = Instant::now();
                edit(session, active, &text);
                let edit = started.elapsed();
                let started = Instant::now();
                run(session, Op::Flush);
                (refresh, edit, started.elapsed(), queued_at.elapsed())
            });
            ready_rx.recv().unwrap();
            edit_tx.send((Instant::now(), queued_text.clone())).unwrap();
            worker.join().unwrap()
        });
        refreshes.push(timings.0);
        edits.push(timings.1);
        saves.push(timings.2);
        queued_to_saved.push(timings.3);
        assert_eq!(
            fs::read_to_string(root.path().join("S/G/Meeting 0.md")).unwrap(),
            queued_text
        );
        assert_eq!(session.document().unwrap().text, queued_text);
        assert!(!session.document().unwrap().dirty());
        assert_eq!(session.index().content(active), Some(queued_text.as_str()));
        assert!(session.index().retained_bytes() <= wren_core::index::MAX_CONTENT_BYTES);
    }
    report(
        "real-filesystem reconcile with an unsaved active draft",
        refreshes,
    );
    report(
        "queued editor operation after reconcile (core service time)",
        edits,
    );
    report(
        "queued flush after reconcile (read/compare/write/fsync service time)",
        saves,
    );
    report(
        "edit enqueued as reconcile starts to persisted clean (queue wait included)",
        queued_to_saved,
    );
    eprintln!("Synchronous Op::Reconcile: edits/flushes queue behind reconciliation.");

    // Background reconciliation, as the desktop app runs it: Tick snapshots on
    // the owner, a worker scans, and FinishReconcile merges on the owner.
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let begin = |session: &mut Session, clock: &mut Duration| {
        *clock += Duration::from_secs(2);
        let started = Instant::now();
        let events = session.apply_at(Op::Tick, *clock).unwrap();
        let snapshot = started.elapsed();
        assert!(events.is_empty());
        (session.take_reconcile_job().unwrap(), snapshot, *clock)
    };
    let scan_on_worker = |job: wren_core::reconcile::ReconcileJob| {
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = job.run();
            (result, started.elapsed())
        })
    };
    let finish = |session: &mut Session, result, now| {
        let started = Instant::now();
        let events = session
            .apply_at(Op::FinishReconcile(result), now)
            .unwrap_or_else(|error| panic!("{error}; committed events: {:?}", error.events));
        (events, started.elapsed())
    };

    let (mut snapshots, mut scans, mut applies, mut owner) = (vec![], vec![], vec![], vec![]);
    for _ in 0..20 {
        let revision = session.index().revision();
        let (job, snapshot, now) = begin(&mut session, &mut clock);
        let (result, scan) = scan_on_worker(job).join().unwrap();
        let (events, apply) = finish(&mut session, result, now);
        assert!(events.is_empty());
        assert_eq!(session.index().revision(), revision);
        snapshots.push(snapshot);
        scans.push(scan);
        applies.push(apply);
        owner.push(snapshot + apply);
    }
    report("background unchanged: owner snapshot (Tick)", snapshots);
    report("background unchanged: worker scan", scans);
    report("background unchanged: owner apply", applies);
    report("background unchanged: owner total", owner);

    let (mut snapshots, mut scans, mut applies, mut owner) = (vec![], vec![], vec![], vec![]);
    for sample in 0..20 {
        let marker = if sample % 2 == 0 {
            "worker alpha"
        } else {
            "worker bravo"
        };
        let stale = if sample % 2 == 0 {
            "worker bravo"
        } else {
            "worker alpha"
        };
        let mut text = body.clone();
        text.replace_range(4200..4200 + marker.len(), marker);
        fs::write(&path, &text).unwrap();
        File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        let (job, snapshot, now) = begin(&mut session, &mut clock);
        let (result, scan) = scan_on_worker(job).join().unwrap();
        let (events, apply) = finish(&mut session, result, now);
        assert!(!events
            .iter()
            .any(|event| matches!(event, Event::TreeChanged)));
        assert_eq!(hits(&session, marker), [inactive]);
        assert!(hits(&session, stale).is_empty());
        snapshots.push(snapshot);
        scans.push(scan);
        applies.push(apply);
        owner.push(snapshot + apply);
    }
    report(
        "background same-size external edit: owner snapshot (Tick)",
        snapshots,
    );
    report("background same-size external edit: worker scan", scans);
    report("background same-size external edit: owner apply", applies);
    report("background same-size external edit: owner total", owner);

    let (mut scans, mut applies, mut edits, mut saved) = (vec![], vec![], vec![], vec![]);
    let mut owner = Vec::new();
    for sample in 0..20 {
        let preexisting = format!("{body}\nunsaved before background scan {sample}");
        session
            .apply_at(
                Op::EditDraft {
                    note: active,
                    contents: preexisting,
                },
                clock + Duration::from_secs(2),
            )
            .unwrap();
        let (job, snapshot, now) = begin(&mut session, &mut clock);
        let worker = scan_on_worker(job);
        // The owner keeps serving input and saving while the worker scans.
        let queued_text = format!("{body}\nedited during background scan {sample}");
        let started = Instant::now();
        session
            .apply_at(
                Op::EditDraft {
                    note: active,
                    contents: queued_text.clone(),
                },
                now,
            )
            .unwrap();
        edits.push(started.elapsed());
        session.apply_at(Op::Flush, now).unwrap();
        saved.push(started.elapsed());
        let (result, scan) = worker.join().unwrap();
        let (events, apply) = finish(&mut session, result, now);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::ExternalChange { .. })),
            "own saves during a background scan are not external changes: {events:?}"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("S/G/Meeting 0.md")).unwrap(),
            queued_text
        );
        assert_eq!(session.document().unwrap().text, queued_text);
        assert!(!session.document().unwrap().dirty());
        assert_eq!(session.index().content(active), Some(queued_text.as_str()));
        assert_eq!(session.model().notes.len(), 5000);
        scans.push(scan);
        applies.push(apply);
        owner.push(snapshot + apply);
    }
    report("background with live draft: worker scan", scans);
    report("background with live draft: owner apply", applies);
    report(
        "background with live draft: owner total (snapshot + apply)",
        owner,
    );
    report(
        "edit applied while the worker scans (core service time)",
        edits,
    );
    report(
        "edit applied while the worker scans to persisted clean (edit + flush)",
        saved,
    );
    eprintln!("These are core owner timings on warm filesystem caches, not native input, rendering, or key-to-photon latency; there is no <30 ms reconciliation target.");
}

#[test]
fn successful_workspace_switch_replaces_the_entire_index_and_parent_selection() {
    let root = fixture();
    let other_root = TempDir::new().unwrap();
    write(
        other_root.path(),
        "Different/G/Unique.md",
        "new workspace only",
    );
    write(
        other_root.path(),
        ".wren/workspace.toml",
        "selection = 'Different/G/Unique.md'\nlayout = 'outline'\n",
    );
    let mut session = session(root.path());
    let old_note = note(&session, "A/G/Note 2.md");
    open(&mut session, old_note);
    edit(&mut session, old_note, "old workspace draft");
    let generation = session.generation();
    run(
        &mut session,
        Op::OpenWorkspace {
            path: other_root.path().to_path_buf(),
        },
    );
    assert_ne!(session.generation(), generation);
    assert_eq!(session.index().len(), 1);
    assert_eq!(session.state().layout(), "outline");
    assert!(hits(&session, "old workspace draft").is_empty());
    assert_eq!(hits(&session, "new workspace only").len(), 1);
    assert_eq!(
        session
            .model()
            .parent_path(session.selected_parent().unwrap()),
        Some(&rel("Different/G"))
    );
    assert_eq!(
        fs::read_to_string(root.path().join("A/G/Note 2.md")).unwrap(),
        "old workspace draft"
    );
}
