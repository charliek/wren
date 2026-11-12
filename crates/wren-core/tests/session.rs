use std::collections::BTreeMap;
use std::fs::{self, File, FileTimes};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;
use wren_core::fs_store::{Fault, RelPath};
use wren_core::kind::NoteKind;
use wren_core::model::{GroupId, ItemRef, NoteId, SectionId};
use wren_core::ops::{Event, ExternalChangeKind, Op, Session, Trash};

const ORIGINAL: &str = "original\n";
const DRAFT: &str = "my unsaved draft\n";
const EXTERNAL: &str = "external writer\n";
const AFTER_RELOCATION: &str = "edited after relocation\n";
const PREPUBLICATION: [Fault; 4] = [
    Fault::Write,
    Fault::Permissions,
    Fault::FileSync,
    Fault::BeforePublish,
];

#[derive(Clone)]
struct Quarantine {
    directory: Arc<TempDir>,
    entries: Arc<Mutex<Vec<(PathBuf, PathBuf)>>>,
}

impl Quarantine {
    fn new() -> Self {
        Self {
            directory: Arc::new(TempDir::new().unwrap()),
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn fake(&self, fail: bool) -> Box<dyn Trash> {
        Box::new(FakeTrash {
            quarantine: self.clone(),
            fail,
        })
    }

    fn destinations(&self) -> Vec<PathBuf> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .map(|(_, destination)| destination.clone())
            .collect()
    }
}

struct FakeTrash {
    quarantine: Quarantine,
    fail: bool,
}

impl Trash for FakeTrash {
    fn trash(&mut self, path: &Path) -> Result<(), String> {
        if self.fail {
            return Err("test quarantine refused deletion".into());
        }
        let mut entries = self.quarantine.entries.lock().unwrap();
        let destination = self.quarantine.directory.path().join(format!(
            "{:04}-{}",
            entries.len(),
            path.file_name().unwrap().to_string_lossy()
        ));
        fs::rename(path, &destination).map_err(|error| error.to_string())?;
        entries.push((path.to_path_buf(), destination));
        Ok(())
    }
}

struct Fixture {
    root: TempDir,
    quarantine: Quarantine,
}

impl Fixture {
    fn empty() -> Self {
        let fixture = Self {
            root: TempDir::new().unwrap(),
            quarantine: Quarantine::new(),
        };
        assert!(!fixture
            .quarantine
            .directory
            .path()
            .starts_with(fixture.path()));
        fixture
    }

    fn standard() -> Self {
        let fixture = Self::empty();
        for (path, bytes) in [
            ("A/G/Note.md", ORIGINAL),
            ("A/G/deep/Deep.md", "deep\n"),
            ("A/L/Loose.md", "loose\n"),
            ("B/H/Other.md", "other\n"),
        ] {
            fixture.write(path, bytes.as_bytes());
        }
        fixture
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let absolute = self.path().join(path);
        fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        fs::write(absolute, bytes).unwrap();
    }

    fn read(&self, path: impl AsRef<Path>) -> Vec<u8> {
        fs::read(self.path().join(path)).unwrap()
    }

    fn session(&self) -> Session {
        self.session_with_failure(false)
    }

    fn session_with_failure(&self, fail: bool) -> Session {
        Session::with_trash(self.path(), self.quarantine.fake(fail)).unwrap()
    }
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

fn at(session: &mut Session, op: Op, time: u64) -> Vec<Event> {
    let label = format!("{op:?} at {time} ms");
    session
        .apply_at(op, ms(time))
        .unwrap_or_else(|error| panic!("{label}: {error}; committed events: {:?}", error.events))
}

fn run(session: &mut Session, op: Op) -> Vec<Event> {
    at(session, op, 0)
}

fn note(session: &Session, path: &str) -> NoteId {
    session
        .model()
        .notes
        .iter()
        .find(|note| note.path.as_path() == Path::new(path))
        .unwrap_or_else(|| panic!("missing note {path}; model: {:?}", session.model()))
        .id
}

fn section(session: &Session, path: &str) -> SectionId {
    session
        .model()
        .sections
        .iter()
        .find(|section| section.path.as_path() == Path::new(path))
        .unwrap_or_else(|| panic!("missing section {path}"))
        .id
}

fn group(session: &Session, path: &str) -> GroupId {
    session
        .model()
        .groups
        .iter()
        .find(|group| group.path.as_path() == Path::new(path))
        .unwrap_or_else(|| panic!("missing group {path}"))
        .id
}

fn open(session: &mut Session, note: NoteId) {
    run(session, Op::OpenNote { note: Some(note) });
}

fn edit(session: &mut Session, note: NoteId, contents: &str, time: u64) {
    at(
        session,
        Op::EditDraft {
            note,
            contents: contents.into(),
        },
        time,
    );
}

fn assert_document(session: &Session, note: NoteId, text: &str, dirty: bool) {
    let document = session.document().expect("expected an open document");
    assert_eq!(document.note, note);
    assert_eq!(document.text, text);
    assert_eq!(document.dirty(), dirty, "document: {document:?}");
}

fn save_after_relocation(session: &mut Session, fixture: &Fixture, id: NoteId, path: &str) {
    edit(session, id, AFTER_RELOCATION, 10_000);
    let events = at(session, Op::Flush, 10_001);
    assert!(saved(&events, id));
    assert_eq!(external_events(&events), 0);
    assert_document(session, id, AFTER_RELOCATION, false);
    assert_eq!(session.model().note(id).unwrap().path, rel(path));
    assert_eq!(fixture.read(path), AFTER_RELOCATION.as_bytes());
}

fn saved(events: &[Event], expected: NoteId) -> bool {
    events
        .iter()
        .any(|event| matches!(event, Event::NoteSaved { note, .. } if *note == expected))
}

fn external_events(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::ExternalChange { .. }))
        .count()
}

fn conflict_copy(events: &[Event]) -> PathBuf {
    let copies: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::ExternalChange {
                kind: ExternalChangeKind::Conflict { copy },
                ..
            } => Some(copy.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copies.len(), 1, "expected one conflict event: {events:?}");
    copies[0].clone()
}

#[derive(Debug, PartialEq, Eq)]
enum DiskEntry {
    Directory(u64),
    File {
        bytes: Vec<u8>,
        inode: u64,
        mode: u32,
    },
    Symlink(PathBuf),
}

fn disk_tree(root: &Path) -> BTreeMap<PathBuf, DiskEntry> {
    fn visit(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, DiskEntry>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let absolute = entry.path();
            let relative = absolute.strip_prefix(root).unwrap().to_path_buf();
            if relative == Path::new(".wren") || relative == Path::new(".wren.lock") {
                continue;
            }
            let metadata = fs::symlink_metadata(&absolute).unwrap();
            let value = if metadata.file_type().is_symlink() {
                DiskEntry::Symlink(fs::read_link(&absolute).unwrap())
            } else if metadata.is_dir() {
                visit(root, &absolute, entries);
                DiskEntry::Directory(metadata.ino())
            } else {
                DiskEntry::File {
                    bytes: fs::read(&absolute).unwrap(),
                    inode: metadata.ino(),
                    mode: metadata.mode() & 0o7777,
                }
            };
            entries.insert(relative, value);
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn model_paths(session: &Session) -> BTreeMap<ItemRef, PathBuf> {
    let model = session.model();
    model
        .sections
        .iter()
        .map(|item| (ItemRef::Section(item.id), item.path.as_path().to_path_buf()))
        .chain(
            model
                .groups
                .iter()
                .map(|item| (ItemRef::Group(item.id), item.path.as_path().to_path_buf())),
        )
        .chain(
            model
                .notes
                .iter()
                .map(|item| (ItemRef::Note(item.id), item.path.as_path().to_path_buf())),
        )
        .collect()
}

fn metadata(fixture: &Fixture) -> toml::Table {
    String::from_utf8(fixture.read(".wren/workspace.toml"))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn empty_workspace_holds_a_persistent_lock_until_drop_and_ignores_it() {
    let fixture = Fixture::empty();
    let session = fixture.session();
    assert!(session.model().sections.is_empty());
    assert!(session.model().notes.is_empty());
    assert!(session.document().is_none());
    let lock = fixture.path().join(".wren.lock");
    let inode = fs::metadata(&lock).unwrap().ino();
    let error = Session::with_trash(fixture.path(), fixture.quarantine.fake(false)).unwrap_err();
    assert!(error.message.contains("locked"), "{error}");
    assert!(lock.is_file());
    drop(session);
    assert_eq!(fs::metadata(&lock).unwrap().ino(), inode);
    let reopened = fixture.session();
    assert!(reopened.model().sections.is_empty());
    assert!(reopened.model().warnings.is_empty());
    assert_eq!(fs::metadata(lock).unwrap().ino(), inode);
}

#[test]
fn scanning_exposes_grouped_and_deep_notes_but_ignores_section_level_hidden_and_non_markdown() {
    let fixture = Fixture::standard();
    for path in [
        "root.md",
        "A/Stray.md",
        ".hidden/note.md",
        "A/.private.md",
        "A/.cache/note.md",
        "A/G/attachment.png",
        "A/G/.private/note.md",
    ] {
        fixture.write(path, b"ignored");
    }
    let mut session = fixture.session();
    let loose = note(&session, "A/L/Loose.md");
    let deep = note(&session, "A/G/deep/Deep.md");
    assert_eq!(session.model().notes.len(), 4);
    assert_eq!(session.model().sections.len(), 2);
    assert_eq!(session.model().groups.len(), 3);
    assert_eq!(
        session.model().note(loose).unwrap().group,
        group(&session, "A/L")
    );
    assert_eq!(
        session.model().note(deep).unwrap().group,
        group(&session, "A/G")
    );
    assert_eq!(
        session.model().note(deep).unwrap().subtitle.as_deref(),
        Some("deep/Deep.md")
    );
    assert!(session
        .model()
        .warnings
        .iter()
        .any(|warning| warning.message.contains("deeper")));
    assert!(session.model().warnings.iter().any(|warning| {
        warning.path.ends_with("A/Stray.md") && warning.message.contains("outside a group")
    }));
    let before = model_paths(&session);
    let disk = disk_tree(fixture.path());
    for time in [0, 1000, 2000] {
        at(&mut session, Op::Reconcile, time);
        assert_eq!(model_paths(&session), before);
    }
    assert_eq!(disk_tree(fixture.path()), disk);
}

#[test]
fn create_operations_sanitize_names_and_suffix_collisions_without_overwriting() {
    let fixture = Fixture::empty();
    let mut session = fixture.session();
    for _ in 0..2 {
        run(
            &mut session,
            Op::CreateSection {
                name: " A/B ".into(),
            },
        );
    }
    let first = section(&session, "A-B");
    let second = section(&session, "A-B 2");
    assert_ne!(first, second);
    for _ in 0..2 {
        run(
            &mut session,
            Op::CreateGroup {
                section: first,
                name: " G:Q? ".into(),
            },
        );
    }
    let parent = group(&session, "A-B/G-Q-");
    fixture.write("A-B/G-Q-/N-X.md", b"do not overwrite");
    for _ in 0..2 {
        let events = run(
            &mut session,
            Op::CreateNote {
                group: parent,
                title: " N/X.MD.md. ".into(),
                kind: NoteKind::Markdown,
            },
        );
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::Created(ItemRef::Note(_)))));
    }
    assert_eq!(fixture.read("A-B/G-Q-/N-X.md"), b"do not overwrite");
    for path in ["A-B/G-Q-/N-X 2.md", "A-B/G-Q-/N-X 3.md"] {
        assert_eq!(fixture.read(path), b"");
        assert_eq!(
            session.model().note(note(&session, path)).unwrap().group,
            parent
        );
    }
    assert!(fixture.path().join("A-B 2").is_dir());
    assert!(fixture.path().join("A-B/G-Q- 2").is_dir());
}

#[test]
fn long_unicode_titles_remain_valid_and_bounded_after_collision_suffixes() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let g = group(&session, "A/G");
    let title = "界é".repeat(150);
    let mut paths = Vec::new();
    for _ in 0..2 {
        let events = run(
            &mut session,
            Op::CreateNote {
                group: g,
                title: title.clone(),
                kind: NoteKind::Markdown,
            },
        );
        let id = events
            .iter()
            .find_map(|event| match event {
                Event::Created(ItemRef::Note(id)) => Some(*id),
                _ => None,
            })
            .unwrap();
        let path = session.model().note(id).unwrap().path.clone();
        let name = path.file_name().to_str().unwrap();
        assert!(name.len() <= 240);
        assert!(name.ends_with(".md"));
        assert_eq!(fixture.read(path.as_path()), b"");
        paths.push(path);
    }
    assert_ne!(paths[0], paths[1]);
    assert!(paths[1].file_name().to_str().unwrap().ends_with(" 2.md"));
}

#[test]
fn section_names_ending_in_md_are_legal_directories_and_survive_reopen() {
    let fixture = Fixture::empty();
    let mut session = fixture.session();
    run(
        &mut session,
        Op::CreateSection {
            name: "Archive.md".into(),
        },
    );
    let id = section(&session, "Archive.md");
    run(
        &mut session,
        Op::CreateGroup {
            section: id,
            name: "G".into(),
        },
    );
    let g = group(&session, "Archive.md/G");
    run(
        &mut session,
        Op::CreateNote {
            group: g,
            title: "Note".into(),
            kind: NoteKind::Markdown,
        },
    );
    assert!(fixture.path().join("Archive.md").is_dir());
    assert!(session.state().writable());
    drop(session);
    let reopened = fixture.session();
    note(&reopened, "Archive.md/G/Note.md");
    assert!(reopened.state().writable());
}

#[test]
fn group_names_ending_in_md_are_legal_for_creation_reorder_and_rename() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let a = section(&session, "A");
    run(
        &mut session,
        Op::CreateGroup {
            section: a,
            name: "Archive.md".into(),
        },
    );
    let archive = group(&session, "A/Archive.md");
    let g = group(&session, "A/G");
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Group(archive),
            before: Some(ItemRef::Group(g)),
        },
    );
    run(
        &mut session,
        Op::Rename {
            target: ItemRef::Group(archive),
            new_name: "Renamed.md".into(),
        },
    );
    assert_eq!(session.model().group(archive).unwrap().name, "Renamed.md");
    assert!(fixture.path().join("A/Renamed.md").is_dir());
    assert!(session.state().writable());
    drop(session);
    let reopened = fixture.session();
    assert_eq!(
        reopened
            .model()
            .groups
            .iter()
            .filter(|item| item.section == section(&reopened, "A"))
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["Renamed.md", "G", "L"]
    );
}

#[test]
fn inactive_write_note_cannot_bypass_the_loaded_baseline_or_mutate_another_document() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let inactive = note(&session, "A/G/Note.md");
    let active = note(&session, "B/H/Other.md");
    open(&mut session, active);
    edit(&mut session, active, DRAFT, 0);
    let before = disk_tree(fixture.path());
    let model = model_paths(&session);
    let error = session
        .apply_at(
            Op::WriteNote {
                note: inactive,
                contents: "stale write".into(),
            },
            ms(1),
        )
        .unwrap_err();
    assert!(error.message.contains("active document"));
    assert!(error.events.is_empty());
    assert_eq!(disk_tree(fixture.path()), before);
    assert_eq!(model_paths(&session), model);
    assert_document(&session, active, DRAFT, true);
    assert_eq!(session.document().unwrap().error, None);
    assert_eq!(session.state().selection(), Some(rel("B/H/Other.md")));
    assert_eq!(fixture.read("A/G/Note.md"), ORIGINAL.as_bytes());
    assert_eq!(fixture.read("B/H/Other.md"), b"other\n");
}

#[test]
fn active_write_note_is_atomic_preserves_permissions_and_saves_the_loaded_document() {
    let fixture = Fixture::standard();
    let path = fixture.path().join("A/G/Note.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    let original_file = File::open(&path).unwrap();
    let inode = original_file.metadata().unwrap().ino();
    let events = run(
        &mut session,
        Op::WriteNote {
            note: id,
            contents: DRAFT.into(),
        },
    );
    assert!(saved(&events, id));
    assert_eq!(external_events(&events), 0);
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o640);
    assert_document(&session, id, DRAFT, false);
    assert_eq!(session.model().note(id).unwrap().bytes, DRAFT.len());
    assert_eq!(session.document().unwrap().error, None);
    assert_eq!(external_events(&run(&mut session, Op::Reconcile)), 0);
    assert_eq!(note(&session, "A/G/Note.md"), id);
}

#[test]
fn renaming_open_note_flushes_and_remaps_selection_history_and_collision_title() {
    let fixture = Fixture::standard();
    fixture.write("A/G/Taken.md", b"occupied");
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 10);
    let events = at(
        &mut session,
        Op::Rename {
            target: ItemRef::Note(id),
            new_name: "Taken.md".into(),
        },
        11,
    );
    assert!(saved(&events, id));
    assert_eq!(session.model().note(id).unwrap().title, "Taken 2");
    assert_eq!(fixture.read("A/G/Taken 2.md"), DRAFT.as_bytes());
    assert_eq!(fixture.read("A/G/Taken.md"), b"occupied");
    assert!(!fixture.path().join("A/G/Note.md").exists());
    assert_document(&session, id, DRAFT, false);
    assert_eq!(session.state().selection(), Some(rel("A/G/Taken 2.md")));
    assert_eq!(
        session.state().last_open_note(),
        Some(rel("A/G/Taken 2.md"))
    );
    assert_eq!(session.state().history(), [rel("A/G/Taken 2.md")]);
    assert!(events.iter().any(|event| matches!(event, Event::NoteMoved { note, new_path } if *note == id && new_path == Path::new("A/G/Taken 2.md"))));
    save_after_relocation(&mut session, &fixture, id, "A/G/Taken 2.md");
    assert!(!fixture.path().join("A/G/Note.md").exists());
    drop(session);
    let reopened = fixture.session();
    assert_eq!(reopened.document().unwrap().text, AFTER_RELOCATION);
    assert_eq!(
        reopened
            .model()
            .note(reopened.document().unwrap().note)
            .unwrap()
            .path,
        rel("A/G/Taken 2.md")
    );
}

#[test]
fn moving_open_note_flushes_preserves_id_and_history_and_never_overwrites() {
    let fixture = Fixture::standard();
    fixture.write("B/H/Note.md", b"occupied");
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    let destination = group(&session, "B/H");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 10);
    let events = at(
        &mut session,
        Op::MoveNote {
            note: id,
            to: destination,
        },
        11,
    );
    assert!(saved(&events, id));
    assert_eq!(session.model().note(id).unwrap().group, destination);
    assert_eq!(session.model().note(id).unwrap().path, rel("B/H/Note 2.md"));
    assert_eq!(fixture.read("B/H/Note 2.md"), DRAFT.as_bytes());
    assert_eq!(fixture.read("B/H/Note.md"), b"occupied");
    assert!(!fixture.path().join("A/G/Note.md").exists());
    assert_document(&session, id, DRAFT, false);
    assert_eq!(session.state().history(), [rel("B/H/Note 2.md")]);
    assert_eq!(session.state().selection(), Some(rel("B/H/Note 2.md")));
    save_after_relocation(&mut session, &fixture, id, "B/H/Note 2.md");
    assert!(!fixture.path().join("A/G/Note.md").exists());
    let other = note(&session, "B/H/Other.md");
    open(&mut session, other);
    run(&mut session, Op::Back);
    assert_document(&session, id, AFTER_RELOCATION, false);
}

#[test]
fn moving_a_deep_note_to_another_group_preserves_inode_and_removes_its_subtitle() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/deep/Deep.md");
    let inode = fs::metadata(fixture.path().join("A/G/deep/Deep.md"))
        .unwrap()
        .ino();
    let destination = group(&session, "B/H");
    run(
        &mut session,
        Op::MoveNote {
            note: id,
            to: destination,
        },
    );
    let moved = session.model().note(id).unwrap();
    assert_eq!(moved.group, destination);
    assert_eq!(moved.subtitle, None);
    assert_eq!(moved.path, rel("B/H/Deep.md"));
    assert_eq!(fixture.read("B/H/Deep.md"), b"deep\n");
    assert_eq!(
        fs::metadata(fixture.path().join("B/H/Deep.md"))
            .unwrap()
            .ino(),
        inode
    );
    assert!(!fixture.path().join("A/G/deep/Deep.md").exists());
}

#[test]
fn renaming_group_flushes_deep_document_and_remaps_all_descendant_ids_and_state() {
    let fixture = Fixture::standard();
    fixture.write(
        ".wren/workspace.toml",
        b"order = ['A', 'A/G', 'B', 'B/H']\n[sort]\n'A/G' = 'title'\n",
    );
    let mut session = fixture.session();
    let id = note(&session, "A/G/deep/Deep.md");
    let direct = note(&session, "A/G/Note.md");
    let g = group(&session, "A/G");
    open(&mut session, direct);
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 10);
    let events = at(
        &mut session,
        Op::Rename {
            target: ItemRef::Group(g),
            new_name: "Renamed".into(),
        },
        11,
    );
    assert!(saved(&events, id));
    assert_eq!(note(&session, "A/Renamed/Note.md"), direct);
    assert_eq!(note(&session, "A/Renamed/deep/Deep.md"), id);
    assert_eq!(group(&session, "A/Renamed"), g);
    assert_eq!(
        session.model().note(id).unwrap().subtitle.as_deref(),
        Some("deep/Deep.md")
    );
    assert_eq!(fixture.read("A/Renamed/deep/Deep.md"), DRAFT.as_bytes());
    assert!(!fixture.path().join("A/G").exists());
    assert_eq!(
        session.state().selection(),
        Some(rel("A/Renamed/deep/Deep.md"))
    );
    assert_eq!(
        session.state().history(),
        [rel("A/Renamed/Note.md"), rel("A/Renamed/deep/Deep.md")]
    );
    assert_eq!(session.state().sort(&rel("A/Renamed")), "title");
    assert!(session.state().order().contains(&rel("A/Renamed")));
    save_after_relocation(&mut session, &fixture, id, "A/Renamed/deep/Deep.md");
    assert!(!fixture.path().join("A/G").exists());
    run(&mut session, Op::Back);
    assert_document(&session, direct, ORIGINAL, false);
}

#[test]
fn moving_group_remaps_parent_deep_notes_history_sort_and_order_with_stable_ids() {
    let fixture = Fixture::standard();
    fixture.write("B/G/Occupied.md", b"keep");
    fixture.write(
        ".wren/workspace.toml",
        b"order = ['A', 'A/G', 'B', 'B/H', 'B/G']\n[sort]\n'A/G' = 'title'\n",
    );
    let mut session = fixture.session();
    let g = group(&session, "A/G");
    let b = section(&session, "B");
    let direct = note(&session, "A/G/Note.md");
    let deep = note(&session, "A/G/deep/Deep.md");
    open(&mut session, direct);
    open(&mut session, deep);
    edit(&mut session, deep, DRAFT, 10);
    let events = at(&mut session, Op::MoveGroup { group: g, to: b }, 11);
    assert!(saved(&events, deep));
    assert_eq!(session.model().group(g).unwrap().section, b);
    assert_eq!(session.model().group(g).unwrap().path, rel("B/G 2"));
    assert_eq!(note(&session, "B/G 2/Note.md"), direct);
    assert_eq!(note(&session, "B/G 2/deep/Deep.md"), deep);
    assert_eq!(session.model().note(deep).unwrap().group, g);
    assert_eq!(fixture.read("B/G 2/deep/Deep.md"), DRAFT.as_bytes());
    assert_eq!(fixture.read("B/G/Occupied.md"), b"keep");
    assert!(!fixture.path().join("A/G").exists());
    assert_eq!(session.state().selection(), Some(rel("B/G 2/deep/Deep.md")));
    assert_eq!(session.state().sort(&rel("B/G 2")), "title");
    assert!(session.state().order().contains(&rel("B/G 2")));
    assert_eq!(
        session.state().history(),
        [rel("B/G 2/Note.md"), rel("B/G 2/deep/Deep.md")]
    );
    save_after_relocation(&mut session, &fixture, deep, "B/G 2/deep/Deep.md");
    assert!(!fixture.path().join("A/G").exists());
    run(&mut session, Op::Back);
    assert_document(&session, direct, ORIGINAL, false);
    drop(session);
    let reopened = fixture.session();
    assert_eq!(reopened.state().sort(&rel("B/G 2")), "title");
    note(&reopened, "B/G 2/deep/Deep.md");
}

#[test]
fn renaming_section_flushes_and_remaps_descendants_without_matching_similar_prefixes() {
    let fixture = Fixture::standard();
    fixture.write("AB/G/Keep.md", b"unrelated");
    let mut session = fixture.session();
    let a = section(&session, "A");
    let g = group(&session, "A/G");
    let id = note(&session, "A/G/Note.md");
    let deep = note(&session, "A/G/deep/Deep.md");
    let loose = note(&session, "A/L/Loose.md");
    let unrelated = note(&session, "AB/G/Keep.md");
    open(&mut session, loose);
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 10);
    let events = at(
        &mut session,
        Op::Rename {
            target: ItemRef::Section(a),
            new_name: "Work".into(),
        },
        11,
    );
    assert!(saved(&events, id));
    assert_eq!(section(&session, "Work"), a);
    assert_eq!(group(&session, "Work/G"), g);
    assert_eq!(note(&session, "Work/G/Note.md"), id);
    assert_eq!(note(&session, "Work/G/deep/Deep.md"), deep);
    assert_eq!(note(&session, "Work/L/Loose.md"), loose);
    assert_eq!(note(&session, "AB/G/Keep.md"), unrelated);
    assert_eq!(fixture.read("Work/G/Note.md"), DRAFT.as_bytes());
    assert_eq!(fixture.read("AB/G/Keep.md"), b"unrelated");
    assert!(!fixture.path().join("A").exists());
    assert_eq!(
        session.state().last_open_note(),
        Some(rel("Work/G/Note.md"))
    );
    assert_eq!(
        session.state().history(),
        [rel("Work/L/Loose.md"), rel("Work/G/Note.md")]
    );
    save_after_relocation(&mut session, &fixture, id, "Work/G/Note.md");
    assert!(!fixture.path().join("A").exists());
}

#[test]
fn section_and_group_reordering_changes_only_metadata_and_survives_reopen() {
    let fixture = Fixture::standard();
    fixture.write("A/Z/Last.md", b"last");
    let mut session = fixture.session();
    let a = section(&session, "A");
    let b = section(&session, "B");
    let g = group(&session, "A/G");
    let z = group(&session, "A/Z");
    let before = disk_tree(fixture.path());
    let ids = model_paths(&session);
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Section(b),
            before: Some(ItemRef::Section(a)),
        },
    );
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Group(z),
            before: Some(ItemRef::Group(g)),
        },
    );
    assert_eq!(
        session
            .model()
            .sections
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [b, a]
    );
    assert_eq!(
        session
            .model()
            .groups
            .iter()
            .filter(|item| item.section == a)
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [z, g, group(&session, "A/L")]
    );
    assert_eq!(model_paths(&session), ids);
    assert_eq!(disk_tree(fixture.path()), before);
    assert!(fixture.path().join(".wren/workspace.toml").is_file());
    drop(session);
    let reopened = fixture.session();
    assert_eq!(
        reopened
            .model()
            .sections
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["B", "A"]
    );
    assert_eq!(
        reopened
            .model()
            .groups
            .iter()
            .filter(|item| item.section == section(&reopened, "A"))
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["Z", "G", "L"]
    );
    assert_eq!(disk_tree(fixture.path()), before);
}

#[test]
fn invalid_ids_destinations_and_reorders_do_not_damage_disk_or_model() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    let g = group(&session, "A/G");
    let h = group(&session, "B/H");
    let a = section(&session, "A");
    let bad_note = NoteId(u64::MAX);
    let bad_group = GroupId(u64::MAX);
    let bad_section = SectionId(u64::MAX);
    let before = disk_tree(fixture.path());
    let model = model_paths(&session);
    for op in [
        Op::CreateGroup {
            section: bad_section,
            name: "No".into(),
        },
        Op::CreateNote {
            group: bad_group,
            title: "No".into(),
            kind: NoteKind::Markdown,
        },
        Op::Rename {
            target: ItemRef::Note(bad_note),
            new_name: "No".into(),
        },
        Op::Delete {
            target: ItemRef::Section(bad_section),
        },
        Op::MoveNote {
            note: id,
            to: bad_group,
        },
        Op::MoveNote {
            note: bad_note,
            to: h,
        },
        Op::MoveGroup {
            group: g,
            to: bad_section,
        },
        Op::MoveGroup {
            group: bad_group,
            to: a,
        },
        Op::WriteNote {
            note: bad_note,
            contents: "damage".into(),
        },
        Op::OpenNote {
            note: Some(bad_note),
        },
        Op::EditDraft {
            note: id,
            contents: "not active".into(),
        },
        Op::Reorder {
            target: ItemRef::Note(id),
            before: None,
        },
        Op::Reorder {
            target: ItemRef::Group(g),
            before: Some(ItemRef::Group(h)),
        },
        Op::Reorder {
            target: ItemRef::Section(a),
            before: Some(ItemRef::Group(g)),
        },
        Op::Back,
        Op::Forward,
    ] {
        let label = format!("{op:?}");
        assert!(session.apply_at(op, ms(0)).is_err(), "{label}");
        assert_eq!(disk_tree(fixture.path()), before, "{label}");
        assert_eq!(model_paths(&session), model, "{label}");
        assert!(session.document().is_none(), "{label}");
    }
    assert!(fixture.quarantine.destinations().is_empty());
}

#[test]
fn invalid_names_do_not_create_or_rename_any_item() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let a = section(&session, "A");
    let g = group(&session, "A/G");
    let id = note(&session, "A/G/Note.md");
    let before = disk_tree(fixture.path());
    let model = model_paths(&session);
    for name in ["", " ", ".", "..", ".hidden", ".md"] {
        for op in [
            Op::CreateSection { name: name.into() },
            Op::CreateGroup {
                section: a,
                name: name.into(),
            },
            Op::CreateNote {
                group: g,
                title: name.into(),
                kind: NoteKind::Markdown,
            },
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: name.into(),
            },
            Op::Rename {
                target: ItemRef::Group(g),
                new_name: name.into(),
            },
            Op::Rename {
                target: ItemRef::Section(a),
                new_name: name.into(),
            },
        ] {
            let label = format!("{op:?}");
            assert!(session.apply_at(op, ms(0)).is_err(), "{label}");
            assert_eq!(disk_tree(fixture.path()), before, "{label}");
            assert_eq!(model_paths(&session), model, "{label}");
        }
    }
}

#[test]
fn deleting_open_note_flushes_to_quarantine_and_prunes_history_and_selection() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    let other = note(&session, "B/H/Other.md");
    open(&mut session, other);
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 10);
    let events = at(
        &mut session,
        Op::Delete {
            target: ItemRef::Note(id),
        },
        11,
    );
    assert!(saved(&events, id));
    assert!(session.document().is_none());
    assert!(session.model().note(id).is_none());
    assert!(!fixture.path().join("A/G/Note.md").exists());
    let destinations = fixture.quarantine.destinations();
    assert_eq!(destinations.len(), 1);
    assert_eq!(fs::read(&destinations[0]).unwrap(), DRAFT.as_bytes());
    assert_eq!(session.state().selection(), None);
    assert_eq!(session.state().last_open_note(), None);
    assert_eq!(session.state().history(), [rel("B/H/Other.md")]);
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Selected(None))));
    drop(session);
    assert!(fixture.session().document().is_none());
}

#[test]
fn deleting_containers_quarantines_all_ignored_contents_and_removes_only_descendants() {
    for delete_section in [false, true] {
        let fixture = Fixture::standard();
        fixture.write("A/G/.hidden/secret.bin", b"hidden bytes");
        fixture.write("A/G/attachment.bin", b"attachment bytes");
        fixture.write("A/.private", b"section private");
        fixture.write("AB/G/Keep.md", b"similar prefix");
        let mut session = fixture.session();
        let id = note(&session, "A/G/deep/Deep.md");
        let unaffected = note(&session, "AB/G/Keep.md");
        open(&mut session, id);
        edit(&mut session, id, DRAFT, 10);
        let target = if delete_section {
            ItemRef::Section(section(&session, "A"))
        } else {
            ItemRef::Group(group(&session, "A/G"))
        };
        let events = at(&mut session, Op::Delete { target }, 11);
        assert!(saved(&events, id));
        assert!(session.document().is_none());
        assert!(session.model().note(id).is_none());
        assert_eq!(note(&session, "AB/G/Keep.md"), unaffected);
        let destinations = fixture.quarantine.destinations();
        assert_eq!(destinations.len(), 1);
        let base = if delete_section {
            destinations[0].join("G")
        } else {
            destinations[0].clone()
        };
        assert_eq!(
            fs::read(base.join("deep/Deep.md")).unwrap(),
            DRAFT.as_bytes()
        );
        assert_eq!(
            fs::read(base.join(".hidden/secret.bin")).unwrap(),
            b"hidden bytes"
        );
        assert_eq!(
            fs::read(base.join("attachment.bin")).unwrap(),
            b"attachment bytes"
        );
        assert_eq!(fixture.read("AB/G/Keep.md"), b"similar prefix");
        assert!(!fixture
            .path()
            .join(if delete_section { "A" } else { "A/G" })
            .exists());
        if delete_section {
            assert_eq!(
                fs::read(destinations[0].join(".private")).unwrap(),
                b"section private"
            );
            assert!(session.model().sections.iter().all(|item| item.name != "A"));
        } else {
            assert_eq!(fixture.read("A/.private"), b"section private");
            note(&session, "A/L/Loose.md");
        }
    }
}

#[test]
fn trash_failure_never_falls_back_to_permanent_deletion_or_prunes_the_model() {
    for kind in 0..3 {
        let fixture = Fixture::standard();
        fixture.write("A/G/attachment.bin", b"keep attachment");
        let mut session = fixture.session_with_failure(true);
        let id = note(&session, "A/G/Note.md");
        open(&mut session, id);
        edit(&mut session, id, DRAFT, 10);
        let target = match kind {
            0 => ItemRef::Note(id),
            1 => ItemRef::Group(group(&session, "A/G")),
            _ => ItemRef::Section(section(&session, "A")),
        };
        let model = model_paths(&session);
        let error = session.apply_at(Op::Delete { target }, ms(11)).unwrap_err();
        assert!(error.message.contains("quarantine refused"));
        assert!(saved(&error.events, id));
        assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
        assert_eq!(fixture.read("A/G/attachment.bin"), b"keep attachment");
        assert_eq!(model_paths(&session), model);
        assert_document(&session, id, DRAFT, false);
        assert_eq!(session.state().selection(), Some(rel("A/G/Note.md")));
        assert!(fixture.quarantine.destinations().is_empty());
    }
}

#[test]
fn same_parent_moves_and_same_name_renames_are_safe_noops_on_disk_and_ids() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    let g = group(&session, "A/G");
    let a = section(&session, "A");
    let before = disk_tree(fixture.path());
    let model = model_paths(&session);
    for op in [
        Op::MoveNote { note: id, to: g },
        Op::MoveGroup { group: g, to: a },
        Op::Rename {
            target: ItemRef::Note(id),
            new_name: "Note.md".into(),
        },
        Op::Rename {
            target: ItemRef::Group(g),
            new_name: "G".into(),
        },
        Op::Rename {
            target: ItemRef::Section(a),
            new_name: "A".into(),
        },
    ] {
        run(&mut session, op);
        assert_eq!(disk_tree(fixture.path()), before);
        assert_eq!(model_paths(&session), model);
    }
}

#[test]
fn switching_workspace_flushes_old_and_replaces_generation_history_selection_and_lock() {
    let old = Fixture::standard();
    let new = Fixture::empty();
    new.write("New/G/Only.md", b"new workspace");
    new.write(
        ".wren/workspace.toml",
        b"last_open_note = 'New/G/Only.md'\nhistory = ['New/G/Only.md']\n[widths.columns]\nnotes = 300\n",
    );
    let mut session = old.session();
    let old_note = note(&session, "A/G/Note.md");
    open(&mut session, old_note);
    edit(&mut session, old_note, DRAFT, 10);
    let generation = session.generation();
    let events = at(
        &mut session,
        Op::OpenWorkspace {
            path: new.path().to_path_buf(),
        },
        11,
    );
    assert!(saved(&events, old_note));
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::WorkspaceOpened)));
    assert_eq!(old.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_eq!(session.generation(), generation + 1);
    assert_eq!(session.model().root, new.path().canonicalize().unwrap());
    assert_eq!(session.model().notes.len(), 1);
    let new_note = note(&session, "New/G/Only.md");
    assert_document(&session, new_note, "new workspace", false);
    assert_eq!(session.state().width("notes"), Some(300));
    assert_eq!(session.state().history(), [rel("New/G/Only.md")]);
    assert!(!session.can_back());
    assert!(!session.can_forward());
    assert!(Session::with_trash(new.path(), new.quarantine.fake(false)).is_err());
    let old_reopened = old.session();
    assert_eq!(old_reopened.document().unwrap().text, DRAFT);
    run(
        &mut session,
        Op::Delete {
            target: ItemRef::Note(new_note),
        },
    );
    assert_eq!(
        old.quarantine.destinations().len(),
        1,
        "injected trash must survive workspace replacement"
    );
    assert!(new.quarantine.destinations().is_empty());
    assert_eq!(
        fs::read(&old.quarantine.destinations()[0]).unwrap(),
        b"new workspace"
    );
}

#[test]
fn switching_to_same_workspace_flushes_without_replacing_state_or_releasing_lock() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 10);
    let generation = session.generation();
    let model = model_paths(&session);
    let events = at(
        &mut session,
        Op::OpenWorkspace {
            path: fixture.path().join("."),
        },
        11,
    );
    assert!(saved(&events, id));
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::WorkspaceOpened)));
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_document(&session, id, DRAFT, false);
    assert_eq!(session.generation(), generation);
    assert_eq!(model_paths(&session), model);
    assert!(Session::with_trash(fixture.path(), fixture.quarantine.fake(false)).is_err());
}

#[test]
fn failed_workspace_replacement_keeps_old_state_document_history_and_writer_lock() {
    let fixture = Fixture::standard();
    let other = Fixture::standard();
    let _other_writer = other.session();
    let invalid = Fixture::empty();
    invalid.write("not-a-directory", b"file");
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    let loose = note(&session, "A/L/Loose.md");
    open(&mut session, loose);
    open(&mut session, id);
    let generation = session.generation();
    let model = model_paths(&session);
    for path in [
        other.path().to_path_buf(),
        invalid.path().join("missing"),
        invalid.path().join("not-a-directory"),
    ] {
        edit(&mut session, id, DRAFT, 10);
        let error = session
            .apply_at(Op::OpenWorkspace { path }, ms(11))
            .unwrap_err();
        assert!(!error.message.is_empty());
        assert!(!error
            .events
            .iter()
            .any(|event| matches!(event, Event::WorkspaceOpened)));
        assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
        assert_document(&session, id, DRAFT, false);
        assert_eq!(session.generation(), generation);
        assert_eq!(model_paths(&session), model);
        assert_eq!(session.state().selection(), Some(rel("A/G/Note.md")));
        assert_eq!(
            session.state().history(),
            [rel("A/L/Loose.md"), rel("A/G/Note.md")]
        );
        assert!(session.can_back());
        assert!(Session::with_trash(fixture.path(), fixture.quarantine.fake(false)).is_err());
    }
}

#[test]
fn autosave_writes_at_exactly_750_ms_idle_and_not_before() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 100);
    assert_document(&session, id, DRAFT, true);
    let measurement = session.last_save_micros();
    for time in [100, 500, 849] {
        let events = at(&mut session, Op::Tick, time);
        assert!(!saved(&events, id));
        assert_eq!(fixture.read("A/G/Note.md"), ORIGINAL.as_bytes());
        assert_eq!(session.last_save_micros(), measurement);
    }
    let events = at(&mut session, Op::Tick, 850);
    assert!(saved(&events, id));
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_document(&session, id, DRAFT, false);
    assert_eq!(session.document().unwrap().error, None);
    assert!(!session.pending_durability());
}

#[test]
fn continuous_typing_saves_latest_bytes_in_multiple_750_ms_windows() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    let mut previous = ORIGINAL.to_string();
    for start in [0, 800, 1600] {
        let mut latest = String::new();
        for offset in (0..=700).step_by(100) {
            latest = format!("draft at {}", start + offset);
            edit(&mut session, id, &latest, start + offset);
            let events = at(&mut session, Op::Tick, start + offset);
            assert!(!saved(&events, id));
            assert_eq!(fixture.read("A/G/Note.md"), previous.as_bytes());
        }
        let events = at(&mut session, Op::Tick, start + 750);
        assert!(saved(&events, id), "window starting {start}");
        assert_eq!(fixture.read("A/G/Note.md"), latest.as_bytes());
        assert_document(&session, id, &latest, false);
        previous = latest;
    }
}

#[test]
fn failed_autosaves_back_off_exponentially_without_busy_retries_and_flush_bypasses_backoff() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    session.fail_next(Fault::Write);
    assert!(session
        .apply_at(Op::Tick, ms(750))
        .unwrap_err()
        .message
        .contains("Write"));
    session.fail_next(Fault::FileSync);
    for time in [751, 800, 1000, 1500, 1749] {
        let events = at(&mut session, Op::Tick, time);
        assert!(!saved(&events, id));
        assert_document(&session, id, DRAFT, true);
        assert_eq!(fixture.read("A/G/Note.md"), ORIGINAL.as_bytes());
    }
    assert!(session
        .apply_at(Op::Tick, ms(1750))
        .unwrap_err()
        .message
        .contains("FileSync"));
    session.fail_next(Fault::BeforePublish);
    for time in [1751, 2000, 3000, 3749] {
        at(&mut session, Op::Tick, time);
        assert_eq!(fixture.read("A/G/Note.md"), ORIGINAL.as_bytes());
    }
    assert!(session
        .apply_at(Op::Tick, ms(3750))
        .unwrap_err()
        .message
        .contains("BeforePublish"));
    let events = at(&mut session, Op::Flush, 3751);
    assert!(saved(&events, id));
    assert_document(&session, id, DRAFT, false);
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
}

#[test]
fn undo_to_baseline_clears_dirty_and_error_without_writing_or_consuming_a_fault() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    session.fail_next(Fault::Write);
    assert!(session.apply_at(Op::Tick, ms(750)).is_err());
    edit(&mut session, id, ORIGINAL, 800);
    assert_document(&session, id, ORIGINAL, false);
    assert_eq!(session.document().unwrap().error, None);
    let before = disk_tree(fixture.path());
    session.fail_next(Fault::BeforePublish);
    for time in [1000, 2000, 20_000] {
        let events = at(&mut session, Op::Tick, time);
        assert!(!saved(&events, id));
    }
    assert_eq!(disk_tree(fixture.path()), before);
    edit(&mut session, id, "new edit", 20_100);
    assert!(session
        .apply_at(Op::Tick, ms(20_850))
        .unwrap_err()
        .message
        .contains("BeforePublish"));
    assert_eq!(fixture.read("A/G/Note.md"), ORIGINAL.as_bytes());
}

#[test]
fn a_new_draft_after_success_is_not_cleared_by_old_self_write_reconciliation() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, "first saved revision", 0);
    at(&mut session, Op::Tick, 750);
    edit(&mut session, id, "new unsaved revision", 800);
    for time in [801, 1000, 1549] {
        let events = at(&mut session, Op::Reconcile, time);
        assert_eq!(external_events(&events), 0);
        assert_document(&session, id, "new unsaved revision", true);
        assert_eq!(fixture.read("A/G/Note.md"), b"first saved revision");
    }
    let events = at(&mut session, Op::Tick, 1550);
    assert!(saved(&events, id));
    assert_document(&session, id, "new unsaved revision", false);
    assert_eq!(fixture.read("A/G/Note.md"), b"new unsaved revision");
}

#[test]
fn back_forward_and_new_history_branch_flush_drafts_and_persist_navigation() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let first = note(&session, "A/G/Note.md");
    let second = note(&session, "A/L/Loose.md");
    let third = note(&session, "B/H/Other.md");
    open(&mut session, first);
    edit(&mut session, first, DRAFT, 10);
    let events = at(&mut session, Op::OpenNote { note: Some(second) }, 11);
    assert!(saved(&events, first));
    edit(&mut session, second, "second draft", 20);
    let events = at(&mut session, Op::Back, 21);
    assert!(saved(&events, second));
    assert_document(&session, first, DRAFT, false);
    assert!(!session.can_back());
    assert!(session.can_forward());
    at(&mut session, Op::Forward, 22);
    assert_document(&session, second, "second draft", false);
    at(&mut session, Op::Back, 23);
    open(&mut session, third);
    assert!(session.can_back());
    assert!(!session.can_forward());
    assert!(session.apply_at(Op::Forward, ms(24)).is_err());
    assert_eq!(
        session.state().history(),
        [rel("A/G/Note.md"), rel("B/H/Other.md")]
    );
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_eq!(fixture.read("A/L/Loose.md"), b"second draft");
    drop(session);
    let mut reopened = fixture.session();
    assert_eq!(reopened.document().unwrap().text, "other\n");
    assert!(reopened.can_back());
    run(&mut reopened, Op::Back);
    assert_eq!(reopened.document().unwrap().text, DRAFT);
}

#[derive(Clone, Copy, Debug)]
enum Barrier {
    Flush,
    SwitchNote,
    Deselect,
    RenameNote,
    RenameGroup,
    RenameSection,
    MoveNote,
    MoveGroup,
    DeleteNote,
    DeleteGroup,
    DeleteSection,
    SwitchWorkspace,
}

const BARRIERS: [Barrier; 12] = [
    Barrier::Flush,
    Barrier::SwitchNote,
    Barrier::Deselect,
    Barrier::RenameNote,
    Barrier::RenameGroup,
    Barrier::RenameSection,
    Barrier::MoveNote,
    Barrier::MoveGroup,
    Barrier::DeleteNote,
    Barrier::DeleteGroup,
    Barrier::DeleteSection,
    Barrier::SwitchWorkspace,
];

fn barrier_op(kind: Barrier, session: &Session, other: &Fixture) -> Op {
    let id = note(session, "A/G/Note.md");
    let g = group(session, "A/G");
    let a = section(session, "A");
    let b = section(session, "B");
    match kind {
        Barrier::Flush => Op::Flush,
        Barrier::SwitchNote => Op::OpenNote {
            note: Some(note(session, "B/H/Other.md")),
        },
        Barrier::Deselect => Op::OpenNote { note: None },
        Barrier::RenameNote => Op::Rename {
            target: ItemRef::Note(id),
            new_name: "Renamed".into(),
        },
        Barrier::RenameGroup => Op::Rename {
            target: ItemRef::Group(g),
            new_name: "Renamed".into(),
        },
        Barrier::RenameSection => Op::Rename {
            target: ItemRef::Section(a),
            new_name: "Renamed".into(),
        },
        Barrier::MoveNote => Op::MoveNote {
            note: id,
            to: group(session, "B/H"),
        },
        Barrier::MoveGroup => Op::MoveGroup { group: g, to: b },
        Barrier::DeleteNote => Op::Delete {
            target: ItemRef::Note(id),
        },
        Barrier::DeleteGroup => Op::Delete {
            target: ItemRef::Group(g),
        },
        Barrier::DeleteSection => Op::Delete {
            target: ItemRef::Section(a),
        },
        Barrier::SwitchWorkspace => Op::OpenWorkspace {
            path: other.path().to_path_buf(),
        },
    }
}

#[test]
fn all_prepublication_faults_preserve_original_and_draft_and_block_every_flush_barrier() {
    for kind in BARRIERS {
        for fault in PREPUBLICATION {
            let fixture = Fixture::standard();
            let other = Fixture::standard();
            let mut session = fixture.session();
            let id = note(&session, "A/G/Note.md");
            open(&mut session, id);
            edit(&mut session, id, DRAFT, 10);
            let op = barrier_op(kind, &session, &other);
            let disk = disk_tree(fixture.path());
            let model = model_paths(&session);
            let state = fixture.read(".wren/workspace.toml");
            let generation = session.generation();
            session.fail_next(fault);
            let error = session.apply_at(op, ms(11)).unwrap_err();
            assert!(
                error
                    .message
                    .contains(&format!("injected storage failure: {fault:?}")),
                "{kind:?}/{fault:?}: {error}"
            );
            assert!(!saved(&error.events, id), "{kind:?}/{fault:?}");
            assert_eq!(disk_tree(fixture.path()), disk, "{kind:?}/{fault:?}");
            assert_eq!(model_paths(&session), model, "{kind:?}/{fault:?}");
            assert_eq!(
                fixture.read(".wren/workspace.toml"),
                state,
                "{kind:?}/{fault:?}"
            );
            assert_document(&session, id, DRAFT, true);
            assert!(
                session.document().unwrap().error.is_some(),
                "{kind:?}/{fault:?}"
            );
            assert_eq!(session.generation(), generation);
            assert!(fixture.quarantine.destinations().is_empty());
            assert!(!session.pending_durability());
            assert!(Session::with_trash(fixture.path(), fixture.quarantine.fake(false)).is_err());
            assert_eq!(other.read("A/G/Note.md"), ORIGINAL.as_bytes());
            let events = at(&mut session, Op::Flush, 12);
            assert!(saved(&events, id), "retry {kind:?}/{fault:?}");
            assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
            assert_document(&session, id, DRAFT, false);
        }
    }
}

#[test]
fn inactive_write_rejection_does_not_consume_faults_and_active_write_failures_retain_drafts() {
    for fault in PREPUBLICATION {
        let fixture = Fixture::standard();
        let mut session = fixture.session();
        let active = note(&session, "A/G/Note.md");
        let inactive = note(&session, "B/H/Other.md");
        open(&mut session, active);
        let before = disk_tree(fixture.path());
        let model = model_paths(&session);
        session.fail_next(fault);
        let error = session
            .apply_at(
                Op::WriteNote {
                    note: inactive,
                    contents: "stale write".into(),
                },
                ms(0),
            )
            .unwrap_err();
        assert!(error.message.contains("active document"));
        assert_eq!(disk_tree(fixture.path()), before, "{fault:?}");
        assert_document(&session, active, ORIGINAL, false);
        let error = session
            .apply_at(
                Op::WriteNote {
                    note: active,
                    contents: DRAFT.into(),
                },
                ms(1),
            )
            .unwrap_err();
        assert!(
            error
                .message
                .contains(&format!("injected storage failure: {fault:?}")),
            "{fault:?}: {error}"
        );
        assert_eq!(disk_tree(fixture.path()), before, "{fault:?}");
        assert_eq!(model_paths(&session), model, "{fault:?}");
        assert_document(&session, active, DRAFT, true);
        assert!(session.document().unwrap().error.is_some());
        assert!(!saved(&error.events, active));
        assert!(saved(&at(&mut session, Op::Flush, 2), active));
        assert_document(&session, active, DRAFT, false);
        assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    }
}

#[test]
fn prepublication_faults_on_note_creation_leave_no_partial_files_or_created_events() {
    for fault in PREPUBLICATION {
        let fixture = Fixture::standard();
        let mut session = fixture.session();
        let g = group(&session, "A/G");
        let before = disk_tree(fixture.path());
        let model = model_paths(&session);
        session.fail_next(fault);
        let error = session
            .apply_at(
                Op::CreateNote {
                    group: g,
                    title: "Failed".into(),
                    kind: NoteKind::Markdown,
                },
                ms(0),
            )
            .unwrap_err();
        assert!(
            error
                .message
                .contains(&format!("injected storage failure: {fault:?}")),
            "{fault:?}: {error}"
        );
        assert_eq!(disk_tree(fixture.path()), before, "{fault:?}");
        assert_eq!(model_paths(&session), model, "{fault:?}");
        assert!(error.events.is_empty(), "{fault:?}: {:?}", error.events);
    }
}

#[test]
fn directory_sync_failure_publishes_bytes_but_retains_dirty_error_and_advances_baseline() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    session.fail_next(Fault::DirectorySync);
    let error = session.apply_at(Op::Flush, ms(100)).unwrap_err();
    assert!(error.message.contains("durability"));
    assert!(!saved(&error.events, id));
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_document(&session, id, DRAFT, true);
    assert!(session
        .document()
        .unwrap()
        .error
        .as_ref()
        .unwrap()
        .contains("DirectorySync"));
    assert!(session.pending_durability());
    let events = at(&mut session, Op::Flush, 101);
    assert!(saved(&events, id));
    assert_eq!(
        external_events(&events),
        0,
        "retry must not conflict with our own publication"
    );
    assert_document(&session, id, DRAFT, false);
    assert_eq!(session.document().unwrap().error, None);
    assert!(!session.pending_durability());
    assert_eq!(session.model().notes.len(), 4);
}

#[test]
fn postpublication_directory_sync_faults_block_all_transitions_until_a_durable_retry() {
    for kind in BARRIERS {
        let fixture = Fixture::standard();
        let other = Fixture::standard();
        let mut session = fixture.session();
        let id = note(&session, "A/G/Note.md");
        open(&mut session, id);
        edit(&mut session, id, DRAFT, 0);
        let model = model_paths(&session);
        let generation = session.generation();
        let op = barrier_op(kind, &session, &other);
        session.fail_next(Fault::DirectorySync);
        let error = session.apply_at(op, ms(100)).unwrap_err();
        assert!(error.message.contains("DirectorySync"), "{kind:?}: {error}");
        assert!(!saved(&error.events, id), "{kind:?}");
        assert_eq!(external_events(&error.events), 0, "{kind:?}");
        assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes(), "{kind:?}");
        assert_eq!(model_paths(&session), model, "{kind:?}");
        assert_document(&session, id, DRAFT, true);
        assert!(session.document().unwrap().error.is_some(), "{kind:?}");
        assert!(session.pending_durability(), "{kind:?}");
        assert_eq!(session.generation(), generation);
        assert!(fixture.quarantine.destinations().is_empty());
        assert_eq!(other.read("A/G/Note.md"), ORIGINAL.as_bytes());
        let events = at(&mut session, Op::Flush, 101);
        assert!(saved(&events, id), "{kind:?}");
        assert_eq!(external_events(&events), 0, "{kind:?}");
        assert_document(&session, id, DRAFT, false);
        assert!(!session.pending_durability(), "{kind:?}");
        assert_eq!(session.model().notes.len(), 4);
    }
}

#[test]
fn new_edits_after_uncertain_publication_are_not_cleared_or_saved_as_a_false_conflict() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    session.fail_next(Fault::DirectorySync);
    assert!(session.apply_at(Op::Tick, ms(750)).is_err());
    edit(&mut session, id, "newer than published", 800);
    at(&mut session, Op::Reconcile, 801);
    assert_document(&session, id, "newer than published", true);
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    let events = at(&mut session, Op::Flush, 802);
    assert!(saved(&events, id));
    assert_eq!(external_events(&events), 0);
    assert_document(&session, id, "newer than published", false);
    assert_eq!(fixture.read("A/G/Note.md"), b"newer than published");
    assert!(!session.pending_durability());
}

#[test]
fn note_creation_durability_error_returns_created_events_and_committed_model() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let g = group(&session, "A/G");
    session.fail_next(Fault::DirectorySync);
    let error = session
        .apply_at(
            Op::CreateNote {
                group: g,
                title: "Published".into(),
                kind: NoteKind::Markdown,
            },
            ms(0),
        )
        .unwrap_err();
    let id = note(&session, "A/G/Published.md");
    assert!(error.message.contains("durability"));
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::Created(ItemRef::Note(created)) if *created == id)));
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::TreeChanged)));
    assert_eq!(fixture.read("A/G/Published.md"), b"");
    assert!(session.pending_durability());
    at(&mut session, Op::Tick, 1000);
    assert!(!session.pending_durability());
    assert_eq!(note(&session, "A/G/Published.md"), id);
}

#[test]
fn relocation_durability_error_returns_committed_events_paths_and_retryable_pending_sync() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    session.fail_next(Fault::DirectorySync);
    let error = session
        .apply_at(
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: "Published".into(),
            },
            ms(10),
        )
        .unwrap_err();
    assert!(error.message.contains("moved"));
    assert!(error.events.iter().any(|event| matches!(event, Event::NoteMoved { note, new_path } if *note == id && new_path == Path::new("A/G/Published.md"))));
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::TreeChanged)));
    assert_eq!(
        session.model().note(id).unwrap().path,
        rel("A/G/Published.md")
    );
    assert_eq!(session.state().selection(), Some(rel("A/G/Published.md")));
    assert_eq!(fixture.read("A/G/Published.md"), ORIGINAL.as_bytes());
    assert!(!fixture.path().join("A/G/Note.md").exists());
    assert!(session.pending_durability());
    at(&mut session, Op::Flush, 11);
    assert!(!session.pending_durability());
    assert_document(&session, id, ORIGINAL, false);
}

#[test]
fn operation_error_keeps_successful_flush_events_when_later_rename_is_invalid() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 10);
    let error = session
        .apply_at(
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: ".hidden".into(),
            },
            ms(11),
        )
        .unwrap_err();
    assert!(error.message.contains("invalid name"));
    assert!(
        saved(&error.events, id),
        "committed flush must not disappear from OpError.events"
    );
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_eq!(session.model().note(id).unwrap().path, rel("A/G/Note.md"));
    assert_document(&session, id, DRAFT, false);
}

#[test]
fn clean_external_edits_reload_once_per_distinct_bytes_revision() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    for (index, contents) in [EXTERNAL, "another external revision"]
        .into_iter()
        .enumerate()
    {
        fixture.write("A/G/Note.md", contents.as_bytes());
        let events = at(&mut session, Op::Reconcile, 1000 * (index as u64 + 1));
        assert_eq!(external_events(&events), 1);
        assert!(events.iter().any(|event| matches!(event, Event::ExternalChange { note, kind: ExternalChangeKind::Reloaded } if *note == id)));
        assert_document(&session, id, contents, false);
        fixture.write("A/G/Note.md", contents.as_bytes());
        for offset in 1..=3 {
            let events = at(
                &mut session,
                Op::Reconcile,
                1000 * (index as u64 + 1) + offset,
            );
            assert_eq!(external_events(&events), 0);
        }
        assert_eq!(fixture.read("A/G/Note.md"), contents.as_bytes());
    }
}

#[test]
fn same_length_external_edit_with_restored_mtime_is_detected_by_bytes() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    let path = fixture.path().join("A/G/Note.md");
    let before = fs::metadata(&path).unwrap();
    let replacement = b"changed!\n";
    assert_eq!(replacement.len(), ORIGINAL.len());
    fs::write(&path, replacement).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        before.modified().unwrap()
    );
    let events = at(&mut session, Op::Reconcile, 1000);
    assert_eq!(external_events(&events), 1);
    assert_document(&session, id, "changed!\n", false);
    assert_eq!(fixture.read("A/G/Note.md"), replacement);
}

#[test]
fn self_write_hints_and_periodic_ticks_do_not_emit_reloaded_or_duplicate_saves() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    let events = session
        .apply(Op::WriteNote {
            note: id,
            contents: DRAFT.into(),
        })
        .unwrap();
    assert!(saved(&events, id));
    for time in [1000, 2000, 3000, 10_000] {
        let events = at(&mut session, Op::Tick, time);
        assert_eq!(external_events(&events), 0);
        assert!(!saved(&events, id));
        let events = at(&mut session, Op::Reconcile, time);
        assert_eq!(external_events(&events), 0);
    }
    assert_document(&session, id, DRAFT, false);
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
    assert_eq!(session.model().notes.len(), 4);
}

#[test]
fn dirty_external_conflicts_keep_both_versions_and_allocate_collision_safe_copies() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    fixture.write("A/G/Note.md", EXTERNAL.as_bytes());
    let events = at(&mut session, Op::Reconcile, 1000);
    let first = conflict_copy(&events);
    assert_eq!(fixture.read(&first), DRAFT.as_bytes());
    assert_eq!(first.parent(), Some(Path::new("A/G")));
    assert!(first
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("Note (conflict "));
    assert_eq!(fixture.read("A/G/Note.md"), EXTERNAL.as_bytes());
    assert_document(&session, id, EXTERNAL, false);
    edit(&mut session, id, "second local draft", 1100);
    fixture.write("A/G/Note.md", b"second external edit");
    let second = conflict_copy(&at(&mut session, Op::Reconcile, 2000));
    assert_ne!(first, second);
    assert_eq!(fixture.read(first), DRAFT.as_bytes());
    assert_eq!(fixture.read(second), b"second local draft");
    assert_eq!(fixture.read("A/G/Note.md"), b"second external edit");
    assert_document(&session, id, "second external edit", false);
    assert!(!session.pending_durability());
    assert_eq!(session.model().notes.len(), 6);
}

#[test]
fn failed_conflict_copy_keeps_original_and_draft_and_blocks_every_transition() {
    for kind in BARRIERS {
        for fault in PREPUBLICATION {
            let fixture = Fixture::standard();
            let other = Fixture::standard();
            let mut session = fixture.session();
            let id = note(&session, "A/G/Note.md");
            open(&mut session, id);
            edit(&mut session, id, DRAFT, 0);
            fixture.write("A/G/Note.md", EXTERNAL.as_bytes());
            let op = barrier_op(kind, &session, &other);
            let before = disk_tree(fixture.path());
            let model = model_paths(&session);
            session.fail_next(fault);
            let error = session.apply_at(op, ms(100)).unwrap_err();
            assert!(
                error
                    .message
                    .contains(&format!("injected storage failure: {fault:?}")),
                "{kind:?}/{fault:?}: {error}"
            );
            assert_eq!(external_events(&error.events), 0, "{kind:?}/{fault:?}");
            assert_eq!(disk_tree(fixture.path()), before, "{kind:?}/{fault:?}");
            assert_eq!(model_paths(&session), model, "{kind:?}/{fault:?}");
            assert_eq!(fixture.read("A/G/Note.md"), EXTERNAL.as_bytes());
            assert_document(&session, id, DRAFT, true);
            assert!(session.document().unwrap().error.is_some());
            assert!(fixture.quarantine.destinations().is_empty());
            assert_eq!(session.model().root, fixture.path().canonicalize().unwrap());
        }
    }
}

#[test]
fn conflict_during_note_or_ancestor_deletion_cancels_before_trashing_the_recovery_copy() {
    for kind in [
        Barrier::DeleteNote,
        Barrier::DeleteGroup,
        Barrier::DeleteSection,
    ] {
        let fixture = Fixture::standard();
        let other = Fixture::empty();
        let mut session = fixture.session();
        let id = note(&session, "A/G/Note.md");
        open(&mut session, id);
        edit(&mut session, id, DRAFT, 0);
        fixture.write("A/G/Note.md", EXTERNAL.as_bytes());
        let op = barrier_op(kind, &session, &other);
        let error = session.apply_at(op, ms(100)).unwrap_err();
        assert!(error.message.contains("cancelled"), "{kind:?}: {error}");
        let copy = conflict_copy(&error.events);
        assert_eq!(fixture.read(copy), DRAFT.as_bytes());
        assert_eq!(fixture.read("A/G/Note.md"), EXTERNAL.as_bytes());
        assert_document(&session, id, EXTERNAL, false);
        assert!(fixture.quarantine.destinations().is_empty());
        section(&session, "A");
        group(&session, "A/G");
        assert_eq!(session.model().notes.len(), 5);
    }
}

#[test]
fn externally_deleted_clean_open_note_clears_selection_instead_of_recreating_it() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    fs::remove_file(fixture.path().join("A/G/Note.md")).unwrap();
    let events = at(&mut session, Op::Reconcile, 1000);
    assert!(session.document().is_none());
    assert!(session.model().note(id).is_none());
    assert_eq!(session.state().selection(), None);
    assert_eq!(session.state().last_open_note(), None);
    assert!(events.iter().any(|event| matches!(event, Event::ExternalChange { note, kind: ExternalChangeKind::Deleted } if *note == id)));
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Selected(None))));
    assert!(!fixture.path().join("A/G/Note.md").exists());
    assert_eq!(external_events(&at(&mut session, Op::Reconcile, 2000)), 0);
}

#[test]
fn externally_deleted_dirty_note_is_recovered_durably_next_to_original_without_recreating_it() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    fs::remove_file(fixture.path().join("A/G/Note.md")).unwrap();
    let events = at(&mut session, Op::Reconcile, 1000);
    let copy = conflict_copy(&events);
    let recovered = session.document().unwrap().note;
    assert_ne!(recovered, id);
    assert_document(&session, recovered, DRAFT, false);
    assert_eq!(
        session.model().note(recovered).unwrap().path.as_path(),
        copy
    );
    assert_eq!(fixture.read(&copy), DRAFT.as_bytes());
    assert_eq!(session.state().selection().unwrap().as_path(), copy);
    assert_eq!(session.state().last_open_note().unwrap().as_path(), copy);
    assert!(!fixture.path().join("A/G/Note.md").exists());
    assert!(session.model().note(id).is_none());
    assert!(!session.pending_durability());
    assert_eq!(external_events(&at(&mut session, Op::Tick, 2000)), 0);
    save_after_relocation(&mut session, &fixture, recovered, copy.to_str().unwrap());
    assert!(!fixture.path().join("A/G/Note.md").exists());
}

#[test]
fn deleted_parent_keeps_dirty_document_and_reports_error_without_inventing_directories() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    let other = note(&session, "B/H/Other.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    fs::remove_dir_all(fixture.path().join("A/G")).unwrap();
    let error = session.apply_at(Op::Reconcile, ms(1000)).unwrap_err();
    assert!(!error.message.is_empty());
    assert_document(&session, id, DRAFT, true);
    assert!(session.document().unwrap().error.is_some());
    assert!(session
        .apply_at(Op::OpenNote { note: Some(other) }, ms(1001))
        .is_err());
    assert_document(&session, id, DRAFT, true);
    assert!(!fixture.path().join("A/G").exists());
    assert_eq!(fixture.read("B/H/Other.md"), b"other\n");
    assert!(fixture.quarantine.destinations().is_empty());
}

#[test]
fn invalid_utf8_external_notes_never_become_deletion_or_destroy_either_version() {
    for dirty in [false, true] {
        let fixture = Fixture::standard();
        let mut session = fixture.session();
        let id = note(&session, "A/G/Note.md");
        open(&mut session, id);
        if dirty {
            edit(&mut session, id, DRAFT, 0);
        }
        fixture.write("A/G/Note.md", b"\xff\xfeexternal");
        let events = match session.apply_at(Op::Reconcile, ms(1000)) {
            Err(error) => {
                assert!(error.message.contains("UTF-8"));
                assert_eq!(external_events(&error.events), 0);
                assert_document(&session, id, if dirty { DRAFT } else { ORIGINAL }, dirty);
                assert!(session.document().unwrap().error.is_some());
                error.events
            }
            Ok(events) => {
                assert!(dirty, "a clean invalid UTF-8 note cannot be loaded as text");
                let copy = conflict_copy(&events);
                let recovered = session.document().unwrap().note;
                assert_ne!(recovered, id);
                assert_document(&session, recovered, DRAFT, false);
                assert_eq!(
                    session.model().note(recovered).unwrap().path.as_path(),
                    copy
                );
                assert_eq!(fixture.read(&copy), DRAFT.as_bytes());
                assert!(events.iter().any(
                    |event| matches!(event, Event::Warning(message) if message.contains("UTF-8"))
                ));
                assert!(!session.pending_durability());
                events
            }
        };
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::ExternalChange {
                kind: ExternalChangeKind::Deleted | ExternalChangeKind::Reloaded,
                ..
            }
        )));
        assert_eq!(fixture.read("A/G/Note.md"), b"\xff\xfeexternal");
        assert!(session.model().note(id).is_some());
        if dirty {
            let copies = disk_tree(fixture.path())
                .into_iter()
                .filter(|(path, _)| path.to_string_lossy().contains("(conflict "))
                .collect::<Vec<_>>();
            assert_eq!(copies.len(), 1);
            assert_eq!(fixture.read(&copies[0].0), DRAFT.as_bytes());
        }
    }
}

#[test]
fn permission_denied_is_not_interpreted_as_external_deletion_or_cleaned_draft() {
    for dirty in [false, true] {
        let fixture = Fixture::standard();
        let mut session = fixture.session();
        let id = note(&session, "A/G/Note.md");
        open(&mut session, id);
        if dirty {
            edit(&mut session, id, DRAFT, 0);
        }
        let path = fixture.path().join("A/G/Note.md");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
        let result = session.apply_at(Op::Reconcile, ms(1000));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let error =
            result.expect_err("permission regression tests require an unprivileged process");
        assert!(!error.message.is_empty());
        assert_eq!(external_events(&error.events), 0);
        assert_document(&session, id, if dirty { DRAFT } else { ORIGINAL }, dirty);
        assert!(session.document().unwrap().error.is_some());
        assert!(session.model().note(id).is_some());
        assert_eq!(fixture.read("A/G/Note.md"), ORIGINAL.as_bytes());
        assert_eq!(session.model().notes.len(), 4);
    }
}

#[test]
fn frequent_reconcile_and_tick_respect_failed_conflict_backoff_without_copy_loops() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    fixture.write("A/G/Note.md", EXTERNAL.as_bytes());
    session.fail_next(Fault::Write);
    assert!(session.apply_at(Op::Reconcile, ms(100)).is_err());
    session.fail_next(Fault::BeforePublish);
    for time in [101, 200, 500, 1000, 1099] {
        let events = at(&mut session, Op::Reconcile, time);
        assert_eq!(external_events(&events), 0);
        let events = at(&mut session, Op::Tick, time);
        assert_eq!(external_events(&events), 0);
        assert_document(&session, id, DRAFT, true);
        assert_eq!(session.model().notes.len(), 4);
        assert_eq!(fixture.read("A/G/Note.md"), EXTERNAL.as_bytes());
    }
    let error = session.apply_at(Op::Reconcile, ms(1100)).unwrap_err();
    assert!(error.message.contains("BeforePublish"));
    for time in [1101, 2000, 3099] {
        assert_eq!(external_events(&at(&mut session, Op::Reconcile, time)), 0);
        assert_eq!(external_events(&at(&mut session, Op::Tick, time)), 0);
        assert_document(&session, id, DRAFT, true);
    }
    let events = at(&mut session, Op::Reconcile, 3100);
    let copy = conflict_copy(&events);
    assert_eq!(fixture.read(copy), DRAFT.as_bytes());
    assert_document(&session, id, EXTERNAL, false);
    for time in [3101, 4000, 5000, 10_000] {
        assert_eq!(external_events(&at(&mut session, Op::Tick, time)), 0);
        assert_eq!(external_events(&at(&mut session, Op::Reconcile, time)), 0);
        assert_eq!(session.model().notes.len(), 5);
    }
}

#[test]
fn uncertain_conflict_copy_durability_retains_draft_then_reuses_copy_on_retry() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    fixture.write("A/G/Note.md", EXTERNAL.as_bytes());
    session.fail_next(Fault::DirectorySync);
    let error = session.apply_at(Op::Reconcile, ms(100)).unwrap_err();
    assert!(error.message.contains("durability"));
    assert_eq!(external_events(&error.events), 0);
    assert_document(&session, id, DRAFT, true);
    assert!(session.pending_durability());
    let copies = disk_tree(fixture.path())
        .keys()
        .filter(|path| path.to_string_lossy().contains("(conflict "))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(copies.len(), 1);
    assert_eq!(fixture.read(&copies[0]), DRAFT.as_bytes());
    for time in [101, 500, 1099] {
        at(&mut session, Op::Reconcile, time);
        assert_document(&session, id, DRAFT, true);
    }
    let events = at(&mut session, Op::Reconcile, 1100);
    assert_eq!(conflict_copy(&events), copies[0]);
    assert_document(&session, id, EXTERNAL, false);
    assert_eq!(session.model().notes.len(), 5);
    at(&mut session, Op::Flush, 1101);
    assert!(!session.pending_durability());
    assert_eq!(fixture.read("A/G/Note.md"), EXTERNAL.as_bytes());
    assert_eq!(fixture.read(&copies[0]), DRAFT.as_bytes());
}

#[test]
fn symlink_replacement_of_note_or_ancestor_cannot_redirect_any_mutation() {
    for replace_ancestor in [false, true] {
        let fixture = Fixture::standard();
        let outside = Fixture::empty();
        outside.write("Note.md", b"outside must survive");
        let mut session = fixture.session();
        let id = note(&session, "A/G/Note.md");
        let g = group(&session, "A/G");
        let h = group(&session, "B/H");
        let source = fixture.path().join(if replace_ancestor {
            "A/G"
        } else {
            "A/G/Note.md"
        });
        let displaced = fixture.path().join(".displaced");
        fs::rename(&source, &displaced).unwrap();
        symlink(
            if replace_ancestor {
                outside.path().to_path_buf()
            } else {
                outside.path().join("Note.md")
            },
            &source,
        )
        .unwrap();
        for op in [
            Op::WriteNote {
                note: id,
                contents: "damage".into(),
            },
            Op::OpenNote { note: Some(id) },
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: "Damage".into(),
            },
            Op::MoveNote { note: id, to: h },
            Op::Delete {
                target: ItemRef::Note(id),
            },
        ] {
            assert!(session.apply_at(op, ms(0)).is_err());
            assert_eq!(outside.read("Note.md"), b"outside must survive");
            assert!(fs::symlink_metadata(&source)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        if replace_ancestor {
            for op in [
                Op::CreateNote {
                    group: g,
                    title: "Damage".into(),
                    kind: NoteKind::Markdown,
                },
                Op::Delete {
                    target: ItemRef::Group(g),
                },
            ] {
                assert!(session.apply_at(op, ms(0)).is_err());
            }
        }
        assert!(!outside.path().join("Damage.md").exists());
        assert!(!fixture.path().join("B/H/Note.md").exists());
        assert!(fixture.quarantine.destinations().is_empty());
        assert_eq!(
            fs::read(if replace_ancestor {
                displaced.join("Note.md")
            } else {
                displaced
            })
            .unwrap(),
            ORIGINAL.as_bytes()
        );
    }
}

#[test]
fn symlink_workspace_roots_and_symlink_lock_files_are_rejected_without_touching_targets() {
    let fixture = Fixture::standard();
    let outside = Fixture::empty();
    let root_link = outside.path().join("workspace-link");
    symlink(fixture.path(), &root_link).unwrap();
    assert!(Session::with_trash(&root_link, outside.quarantine.fake(false)).is_err());
    assert!(!fixture.path().join(".wren.lock").exists());
    outside.write("lock-target", b"not a Wren lock");
    symlink(
        outside.path().join("lock-target"),
        fixture.path().join(".wren.lock"),
    )
    .unwrap();
    assert!(Session::with_trash(fixture.path(), fixture.quarantine.fake(false)).is_err());
    assert_eq!(outside.read("lock-target"), b"not a Wren lock");
    assert_eq!(fixture.read("A/G/Note.md"), ORIGINAL.as_bytes());
}

#[test]
fn malformed_or_unsupported_metadata_is_preserved_through_actual_note_and_structure_ops() {
    let sources: &[&[u8]] = &[
        b"layout = [",
        b"\xff\xfe",
        b"schema_version = 99\n",
        b"history = ['../escape.md']\n",
    ];
    for source in sources {
        let fixture = Fixture::standard();
        fixture.write(".wren/workspace.toml", source);
        let mut session = fixture.session();
        assert!(!session.state().writable());
        assert!(session
            .model()
            .warnings
            .iter()
            .any(|warning| warning.message.contains("metadata is read-only")));
        let id = note(&session, "A/G/Note.md");
        let g = group(&session, "A/G");
        open(&mut session, id);
        run(
            &mut session,
            Op::WriteNote {
                note: id,
                contents: DRAFT.into(),
            },
        );
        run(
            &mut session,
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: "Renamed".into(),
            },
        );
        run(
            &mut session,
            Op::CreateNote {
                group: g,
                title: "New".into(),
                kind: NoteKind::Markdown,
            },
        );
        run(
            &mut session,
            Op::Delete {
                target: ItemRef::Note(id),
            },
        );
        assert_eq!(fixture.read(".wren/workspace.toml"), *source);
        assert_eq!(fixture.read("A/G/New.md"), b"");
        assert!(!fixture.path().join("A/G/Renamed.md").exists());
        assert_eq!(
            fs::read(&fixture.quarantine.destinations()[0]).unwrap(),
            DRAFT.as_bytes()
        );
    }
}

#[test]
fn unknown_metadata_values_survive_actual_selection_reorder_move_rename_and_delete_ops() {
    let fixture = Fixture::standard();
    let source = "schema_version = 1\nfuture_flag = true\nfuture_array = [1, 'two', { three = 3 }]\nfuture_date = 2026-11-04T12:30:00Z\n[widths.columns]\nfuture_pane = { x = [false, 2] }\n[plugin]\nselection = '../plugin-owned'\npath = 'A/G/Note.md'\n";
    fixture.write(".wren/workspace.toml", source.as_bytes());
    let original: toml::Table = source.parse().unwrap();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    let g = group(&session, "A/G");
    let a = section(&session, "A");
    let b = section(&session, "B");
    open(&mut session, id);
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Section(b),
            before: Some(ItemRef::Section(a)),
        },
    );
    run(&mut session, Op::MoveGroup { group: g, to: b });
    run(
        &mut session,
        Op::Rename {
            target: ItemRef::Note(id),
            new_name: "Renamed".into(),
        },
    );
    run(
        &mut session,
        Op::Delete {
            target: ItemRef::Note(id),
        },
    );
    let saved = metadata(&fixture);
    for key in [
        "future_flag",
        "future_array",
        "future_date",
        "widths",
        "plugin",
    ] {
        assert_eq!(saved[key], original[key], "{key}");
    }
    assert_eq!(fixture.read("B/G/deep/Deep.md"), b"deep\n");
    assert!(!fixture.path().join("B/G/Renamed.md").exists());
    assert_eq!(
        fs::read(&fixture.quarantine.destinations()[0]).unwrap(),
        ORIGINAL.as_bytes()
    );
    assert!(session.state().writable());
}

#[test]
fn metadata_conflict_after_committed_rename_preserves_disk_change_and_returns_its_events() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    let external_metadata = b"layout = 'outline'\nplugin_value = 42\n";
    fixture.write(".wren/workspace.toml", external_metadata);
    let error = session
        .apply_at(
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: "Committed".into(),
            },
            ms(1),
        )
        .unwrap_err();
    assert!(error.message.contains("workspace.toml"));
    assert!(error.events.iter().any(|event| matches!(event, Event::NoteMoved { note, new_path } if *note == id && new_path == Path::new("A/G/Committed.md"))));
    assert!(error
        .events
        .iter()
        .any(|event| matches!(event, Event::TreeChanged)));
    assert!(error.events.iter().any(
        |event| matches!(event, Event::Warning(message) if message.contains("remain applied"))
    ));
    assert_eq!(fixture.read("A/G/Committed.md"), ORIGINAL.as_bytes());
    assert!(!fixture.path().join("A/G/Note.md").exists());
    assert_eq!(
        session.model().note(id).unwrap().path,
        rel("A/G/Committed.md")
    );
    assert_eq!(fixture.read(".wren/workspace.toml"), external_metadata);
}

#[test]
fn structural_before_publish_failures_leave_existing_items_and_ids_untouched() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let a = section(&session, "A");
    let b = section(&session, "B");
    let g = group(&session, "A/G");
    let id = note(&session, "A/G/Note.md");
    let before = disk_tree(fixture.path());
    let model = model_paths(&session);
    for op in [
        Op::CreateSection {
            name: "Failed".into(),
        },
        Op::CreateGroup {
            section: a,
            name: "Failed".into(),
        },
        Op::Rename {
            target: ItemRef::Note(id),
            new_name: "Failed".into(),
        },
        Op::Rename {
            target: ItemRef::Group(g),
            new_name: "Failed".into(),
        },
        Op::Rename {
            target: ItemRef::Section(a),
            new_name: "Failed".into(),
        },
        Op::MoveNote {
            note: id,
            to: group(&session, "B/H"),
        },
        Op::MoveGroup { group: g, to: b },
    ] {
        let label = format!("{op:?}");
        session.fail_next(Fault::BeforePublish);
        let error = session.apply_at(op, ms(0)).unwrap_err();
        assert!(error.message.contains("BeforePublish"), "{label}: {error}");
        assert_eq!(disk_tree(fixture.path()), before, "{label}");
        assert_eq!(model_paths(&session), model, "{label}");
        assert!(error.events.is_empty(), "{label}: {:?}", error.events);
    }
}

#[test]
fn unrelated_external_additions_and_deletions_preserve_existing_ids_and_never_reuse_deleted_ids() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let keep = note(&session, "A/G/Note.md");
    let removed = note(&session, "A/L/Loose.md");
    let old_max = model_paths(&session)
        .keys()
        .map(|item| match item {
            ItemRef::Section(id) => id.0,
            ItemRef::Group(id) => id.0,
            ItemRef::Note(id) => id.0,
        })
        .max()
        .unwrap();
    fs::remove_file(fixture.path().join("A/L/Loose.md")).unwrap();
    fixture.write("A/G/Added.md", b"added");
    at(&mut session, Op::Reconcile, 1000);
    assert_eq!(note(&session, "A/G/Note.md"), keep);
    assert!(session.model().note(removed).is_none());
    let added = note(&session, "A/G/Added.md");
    assert!(added.0 > old_max);
    fixture.write("A/L/Loose.md", b"new incarnation");
    at(&mut session, Op::Reconcile, 2000);
    let replacement = note(&session, "A/L/Loose.md");
    assert_ne!(replacement, removed);
    assert!(replacement.0 > added.0);
    assert_eq!(note(&session, "A/G/Note.md"), keep);
    assert_eq!(fixture.read("A/L/Loose.md"), b"new incarnation");
}

#[test]
fn external_bytes_equal_to_current_draft_are_not_misclassified_as_a_conflict() {
    let fixture = Fixture::standard();
    let mut session = fixture.session();
    let id = note(&session, "A/G/Note.md");
    open(&mut session, id);
    edit(&mut session, id, DRAFT, 0);
    fixture.write("A/G/Note.md", DRAFT.as_bytes());
    let events = at(&mut session, Op::Reconcile, 1000);
    assert_eq!(external_events(&events), 0);
    assert!(saved(&events, id));
    assert_document(&session, id, DRAFT, false);
    assert_eq!(session.model().notes.len(), 4);
    assert_eq!(fixture.read("A/G/Note.md"), DRAFT.as_bytes());
}

// -- The op-by-kind table: every op that touches a note keeps its extension,
// -- title and kind.

fn other_kind(kind: NoteKind) -> NoteKind {
    match kind {
        NoteKind::Markdown => NoteKind::Board,
        NoteKind::Board => NoteKind::Markdown,
    }
}

fn created_note(events: &[Event]) -> NoteId {
    events
        .iter()
        .find_map(|event| match event {
            Event::Created(ItemRef::Note(id)) => Some(*id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no note was created: {events:?}"))
}

fn assert_note_kind(
    session: &Session,
    fixture: &Fixture,
    id: NoteId,
    path: &str,
    title: &str,
    kind: NoteKind,
) {
    let note = session
        .model()
        .note(id)
        .unwrap_or_else(|| panic!("missing note {id:?} at {path}"));
    assert_eq!(note.path, rel(path));
    assert_eq!(note.title, title);
    assert_eq!(note.kind, kind);
    assert_eq!(NoteKind::from_name(note.path.file_name()), Some(kind));
    assert!(fixture.path().join(path).is_file(), "{path} is not on disk");
    if let Some(document) = session.document().filter(|document| document.note == id) {
        assert_eq!(*document.path(), rel(path));
    }
}

/// Runs the table for one kind. `open` opens a session that lists that kind.
fn ops_keep_the_note_kind(kind: NoteKind, open_session: impl Fn(&Fixture) -> Session) {
    let ext = kind.extension();
    let other_ext = other_kind(kind).extension();
    let fixture = Fixture::empty();
    fixture.write("A/G/Other.md", b"other\n");
    fs::create_dir_all(fixture.path().join("A/H")).unwrap();
    fs::create_dir_all(fixture.path().join("B/K")).unwrap();
    let mut session = open_session(&fixture);
    let g = group(&session, "A/G");
    let h = group(&session, "A/H");
    let k = group(&session, "B/K");
    let b = section(&session, "B");

    // Create, with the kind's own extension typed into the title.
    let events = at(
        &mut session,
        Op::CreateNote {
            group: g,
            title: format!("Flow{}", ext.to_uppercase()),
            kind,
        },
        0,
    );
    let id = created_note(&events);
    let path = format!("A/G/Flow{ext}");
    assert_note_kind(&session, &fixture, id, &path, "Flow", kind);
    assert_eq!(fixture.read(&path), kind.template().as_bytes());

    // Open and flush.
    at(&mut session, Op::OpenNote { note: Some(id) }, 1);
    assert_document(&session, id, kind.template(), false);
    edit(&mut session, id, "edited\n", 2);
    let events = at(&mut session, Op::Flush, 3);
    assert!(saved(&events, id));
    assert_eq!(fixture.read(&path), b"edited\n");
    assert_note_kind(&session, &fixture, id, &path, "Flow", kind);

    // Rename: the other kind's extension is part of the title, never the kind.
    let rename = |session: &mut Session, name: String, time| {
        at(
            session,
            Op::Rename {
                target: ItemRef::Note(id),
                new_name: name,
            },
            time,
        );
    };
    rename(&mut session, format!("Plan{other_ext}"), 4);
    let path = format!("A/G/Plan{other_ext}{ext}");
    assert_note_kind(
        &session,
        &fixture,
        id,
        &path,
        &format!("Plan{other_ext}"),
        kind,
    );
    rename(&mut session, format!("Plan{ext}{ext}"), 5);
    let path = format!("A/G/Plan{ext}");
    assert_note_kind(&session, &fixture, id, &path, "Plan", kind);

    // Move, move its group, rename its group, reorder its group.
    at(&mut session, Op::MoveNote { note: id, to: h }, 6);
    assert_note_kind(
        &session,
        &fixture,
        id,
        &format!("A/H/Plan{ext}"),
        "Plan",
        kind,
    );
    at(&mut session, Op::MoveGroup { group: h, to: b }, 7);
    assert_note_kind(
        &session,
        &fixture,
        id,
        &format!("B/H/Plan{ext}"),
        "Plan",
        kind,
    );
    at(
        &mut session,
        Op::Rename {
            target: ItemRef::Group(h),
            new_name: "Moved".into(),
        },
        8,
    );
    let path = format!("B/Moved/Plan{ext}");
    assert_note_kind(&session, &fixture, id, &path, "Plan", kind);
    at(
        &mut session,
        Op::Reorder {
            target: ItemRef::Group(h),
            before: Some(ItemRef::Group(k)),
        },
        9,
    );
    assert_note_kind(&session, &fixture, id, &path, "Plan", kind);
    assert_document(&session, id, "edited\n", false);

    // External reload of the clean document.
    fixture.write(&path, EXTERNAL.as_bytes());
    let events = at(&mut session, Op::Reconcile, 100);
    assert!(events.iter().any(|event| matches!(
        event,
        Event::ExternalChange { note, kind: ExternalChangeKind::Reloaded } if *note == id
    )));
    assert_document(&session, id, EXTERNAL, false);
    assert_note_kind(&session, &fixture, id, &path, "Plan", kind);

    // A dirty document changed on disk: the draft goes to a copy of the same kind.
    edit(&mut session, id, DRAFT, 200);
    fixture.write(&path, b"second writer\n");
    let events = at(&mut session, Op::Reconcile, 300);
    let copy = conflict_copy(&events);
    let copy_name = copy.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(copy.parent(), Some(Path::new("B/Moved")));
    assert!(copy_name.starts_with("Plan (conflict "), "{copy_name}");
    assert!(copy_name.ends_with(&format!("){ext}")), "{copy_name}");
    assert_eq!(fixture.read(&copy), DRAFT.as_bytes());
    let copy_id = note(&session, copy.to_str().unwrap());
    let copy_title = &copy_name[..copy_name.len() - ext.len()];
    assert_note_kind(
        &session,
        &fixture,
        copy_id,
        copy.to_str().unwrap(),
        copy_title,
        kind,
    );
    assert_document(&session, id, "second writer\n", false);
    assert_note_kind(&session, &fixture, id, &path, "Plan", kind);

    // Back and forward across kinds.
    let other = note(&session, "A/G/Other.md");
    at(&mut session, Op::OpenNote { note: Some(other) }, 400);
    at(&mut session, Op::Back, 401);
    assert_document(&session, id, "second writer\n", false);
    assert_note_kind(&session, &fixture, id, &path, "Plan", kind);
    at(&mut session, Op::Forward, 402);
    assert_document(&session, other, "other\n", false);
    at(&mut session, Op::Back, 403);
    assert_eq!(session.document().unwrap().note, id);

    // Delete.
    at(
        &mut session,
        Op::Delete {
            target: ItemRef::Note(id),
        },
        500,
    );
    assert!(session.model().note(id).is_none());
    assert!(!fixture.path().join(&path).exists());
    let destinations = fixture.quarantine.destinations();
    assert_eq!(destinations.len(), 1);
    assert!(destinations[0]
        .to_str()
        .unwrap()
        .ends_with(&format!("Plan{ext}")));
    assert_eq!(fs::read(&destinations[0]).unwrap(), b"second writer\n");
    assert_note_kind(
        &session,
        &fixture,
        copy_id,
        copy.to_str().unwrap(),
        copy_title,
        kind,
    );
}

#[test]
fn op_by_kind_table_markdown() {
    ops_keep_the_note_kind(NoteKind::Markdown, Fixture::session);
}

#[test]
fn markdown_ops_beside_a_board_of_the_same_title_collide_only_by_full_name() {
    let fixture = Fixture::empty();
    let board = b"schema = 1\n";
    fixture.write("A/G/Flow.wren-board", board);
    fixture.write("A/H/Flow.wren-board", board);
    fixture.write("A/H/Other.md", b"other\n");
    let mut session = fixture.session();
    let g = group(&session, "A/G");
    let h = group(&session, "A/H");

    let events = run(
        &mut session,
        Op::CreateNote {
            group: g,
            title: "Flow".into(),
            kind: NoteKind::Markdown,
        },
    );
    let flow = created_note(&events);
    assert_note_kind(
        &session,
        &fixture,
        flow,
        "A/G/Flow.md",
        "Flow",
        NoteKind::Markdown,
    );
    let other = note(&session, "A/H/Other.md");
    run(
        &mut session,
        Op::Rename {
            target: ItemRef::Note(other),
            new_name: "Flow".into(),
        },
    );
    assert_note_kind(
        &session,
        &fixture,
        other,
        "A/H/Flow.md",
        "Flow",
        NoteKind::Markdown,
    );
    run(&mut session, Op::MoveNote { note: flow, to: h });
    assert_note_kind(
        &session,
        &fixture,
        flow,
        "A/H/Flow 2.md",
        "Flow 2",
        NoteKind::Markdown,
    );
    assert_eq!(fixture.read("A/G/Flow.wren-board"), board);
    assert_eq!(fixture.read("A/H/Flow.wren-board"), board);
}

#[test]
fn board_ops_write_board_names_and_bytes() {
    // Session::open lists Markdown only, so each board here is created by the
    // op under test and checked on disk after any op that rescans.
    let fixture = Fixture::empty();
    fixture.write("A/G/Flow.md", b"note\n");
    fs::create_dir_all(fixture.path().join("A/H")).unwrap();
    let mut session = fixture.session();
    let g = group(&session, "A/G");
    let h = group(&session, "A/H");
    let create = |session: &mut Session, title: &str| {
        created_note(&run(
            session,
            Op::CreateNote {
                group: g,
                title: title.into(),
                kind: NoteKind::Board,
            },
        ))
    };

    let flow = create(&mut session, "Flow");
    assert_note_kind(
        &session,
        &fixture,
        flow,
        "A/G/Flow.wren-board",
        "Flow",
        NoteKind::Board,
    );
    assert_eq!(fixture.read("A/G/Flow.wren-board"), b"schema = 1\n");
    let second = create(&mut session, "Flow.WREN-BOARD");
    assert_note_kind(
        &session,
        &fixture,
        second,
        "A/G/Flow 2.wren-board",
        "Flow 2",
        NoteKind::Board,
    );

    open(&mut session, flow);
    edit(&mut session, flow, DRAFT, 10);
    fixture.write("A/G/Flow.wren-board", EXTERNAL.as_bytes());
    let error = session.apply_at(Op::Flush, ms(11)).unwrap_err();
    let copy = conflict_copy(&error.events);
    let name = copy.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("Flow (conflict "), "{name}");
    assert!(name.ends_with(").wren-board"), "{name}");
    assert_eq!(fixture.read(&copy), DRAFT.as_bytes());
    assert_eq!(fixture.read("A/G/Flow.wren-board"), EXTERNAL.as_bytes());

    let renamed = create(&mut session, "Renamed");
    run(
        &mut session,
        Op::Rename {
            target: ItemRef::Note(renamed),
            new_name: "Plan.md".into(),
        },
    );
    assert_eq!(fixture.read("A/G/Plan.md.wren-board"), b"schema = 1\n");
    let moved = create(&mut session, "Flow.md");
    assert_eq!(fixture.read("A/G/Flow.md.wren-board"), b"schema = 1\n");
    run(&mut session, Op::MoveNote { note: moved, to: h });
    assert_eq!(fixture.read("A/H/Flow.md.wren-board"), b"schema = 1\n");
    assert_eq!(fixture.read("A/G/Flow.md"), b"note\n");
}
