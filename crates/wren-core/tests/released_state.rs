//! Mixed versions: every state file written beside boards must still load in
//! released Wren, whose validator is frozen in `support/released_v0_1_1.rs`,
//! and with boards off the file must be the one `main` writes.

use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;
use toml::{Table, Value};
use wren_core::fs_store::{FsStore, RelPath};
use wren_core::kind::NoteKind;
use wren_core::model::{GroupId, ItemRef, NoteId, SectionId};
use wren_core::ops::{Event, ExternalChangeKind, Op, Session, SessionOptions, Trash};
use wren_core::workspace_state::WorkspaceState;

#[path = "support/released_v0_1_1.rs"]
mod released;

const STATE: &str = ".wren/workspace.toml";
const BOARD: &str = "schema = 1\n";

/// What `main` (393ae14) writes for `scripted_ops`; see `record_main_bytes`.
const MAIN_STATE: &[u8] = include_bytes!("fixtures/main-state.toml");

struct Bin {
    directory: TempDir,
    count: usize,
}

impl Trash for Bin {
    fn trash(&mut self, path: &Path) -> Result<(), String> {
        self.count += 1;
        let name = path.file_name().unwrap().to_string_lossy();
        fs::rename(
            path,
            self.directory.path().join(format!("{}-{name}", self.count)),
        )
        .map_err(|error| error.to_string())
    }
}

struct Workspace {
    root: TempDir,
}

impl Workspace {
    fn new(files: &[(&str, &str)]) -> Self {
        let workspace = Self {
            root: TempDir::new().unwrap(),
        };
        for (path, text) in files {
            workspace.write(path, text.as_bytes());
        }
        workspace
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn open(&self, boards: bool) -> Session {
        let bin = Bin {
            directory: TempDir::new().unwrap(),
            count: 0,
        };
        Session::with_options(self.path(), Box::new(bin), SessionOptions { boards }).unwrap()
    }

    fn state_bytes(&self) -> Option<Vec<u8>> {
        fs::read(self.path().join(STATE)).ok()
    }

    fn table(&self) -> Table {
        String::from_utf8(self.state_bytes().expect("a state file"))
            .unwrap()
            .parse()
            .unwrap()
    }

    /// The state file as a fresh load reads it. The open Session holds the
    /// workspace lock, so the file is read from a copy.
    fn load_state(&self) -> WorkspaceState {
        let copy = TempDir::new().unwrap();
        fs::create_dir(copy.path().join(".wren")).unwrap();
        fs::write(copy.path().join(STATE), self.state_bytes().unwrap()).unwrap();
        WorkspaceState::load(&FsStore::open(copy.path()).unwrap())
    }
}

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

fn rels(paths: &[&str]) -> Vec<RelPath> {
    paths.iter().copied().map(rel).collect()
}

fn step(session: &mut Session, op: Op, time: u64) {
    let label = format!("{op:?} at {time}");
    session
        .apply_at(op, Duration::from_millis(time))
        .unwrap_or_else(|error| panic!("{label}: {error}"));
}

fn note(session: &Session, path: &str) -> NoteId {
    session
        .model()
        .notes
        .iter()
        .find(|note| note.path.as_path() == Path::new(path))
        .unwrap_or_else(|| panic!("missing note {path}"))
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

fn section(session: &Session, path: &str) -> SectionId {
    session
        .model()
        .sections
        .iter()
        .find(|section| section.path.as_path() == Path::new(path))
        .unwrap_or_else(|| panic!("missing section {path}"))
        .id
}

fn open(session: &mut Session, path: &str, time: u64) {
    let note = note(session, path);
    step(session, Op::OpenNote { note: Some(note) }, time);
}

fn document_path(session: &Session) -> Option<RelPath> {
    session.document().map(|document| document.path().clone())
}

fn is_markdown(path: &str) -> bool {
    NoteKind::from_name(Path::new(path).file_name().unwrap()) == Some(NoteKind::Markdown)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect(),
        Some(value) => vec![value.as_str().unwrap().to_owned()],
        None => Vec::new(),
    }
}

/// The state file loads in released Wren, its v1 keys name Markdown notes
/// only, and `[navigation]` exists only beside another kind and is believed
/// when read back.
fn check(workspace: &Workspace, session: &Session, label: &str) {
    check_file(workspace, label);
    let Some(bytes) = workspace.state_bytes() else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    let loaded = workspace.load_state();
    let state = session.state();
    assert_eq!(loaded.history(), state.history(), "{label}\n{text}");
    assert_eq!(
        loaded.history_position(),
        state.history_position(),
        "{label}\n{text}"
    );
    assert_eq!(loaded.recent(), state.recent(), "{label}\n{text}");
    assert_eq!(
        loaded.last_open_note(),
        state.last_open_note(),
        "{label}\n{text}"
    );
    assert_eq!(loaded.selection(), state.selection(), "{label}\n{text}");
}

fn check_file(workspace: &Workspace, label: &str) {
    let Some(bytes) = workspace.state_bytes() else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    released::accepts(&bytes).unwrap_or_else(|error| panic!("{label}: {error}\n{text}"));
    let table: Table = text.parse().unwrap();
    for key in ["history", "recent", "last_open_note"] {
        for path in strings(table.get(key)) {
            assert!(is_markdown(&path), "{label}: {key} names {path}\n{text}");
        }
    }
    if let Some(navigation) = table.get("navigation").and_then(Value::as_table) {
        let other_kind = ["history", "recent", "last_open"]
            .into_iter()
            .flat_map(|key| strings(navigation.get(key)))
            .any(|path| !is_markdown(&path));
        assert!(other_kind, "{label}: [navigation] names no board\n{text}");
    }
    let loaded = workspace.load_state();
    assert!(loaded.writable(), "{label}: {:?}", loaded.warning());
    let (lists, last_open) = match table.get("navigation").and_then(Value::as_table) {
        Some(navigation) => (navigation, "last_open"),
        None => (&table, "last_open_note"),
    };
    let paths = |key| strings(lists.get(key));
    let shown = |paths: Vec<RelPath>| -> Vec<String> {
        paths
            .iter()
            .map(|path| path.as_path().to_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(shown(loaded.history()), paths("history"), "{label}\n{text}");
    assert_eq!(shown(loaded.recent()), paths("recent"), "{label}\n{text}");
    assert_eq!(
        shown(loaded.last_open_note().into_iter().collect()),
        paths(last_open),
        "{label}\n{text}"
    );
}

#[test]
fn the_frozen_validator_refuses_another_kind_in_a_v1_note_key() {
    for (bytes, accepted) in [
        (
            &b"history = ['A/G/x.md']\nrecent = ['A/G/x.md']\nlast_open_note = 'A/G/x.md'"[..],
            true,
        ),
        (b"selection = 'A/G/x.wren-board'", true),
        (
            b"[navigation]\nhistory = ['A/G/x.wren-board']\nlast_open = 'A/G/x.wren-board'",
            true,
        ),
        (b"history = ['A/G/x.wren-board']", false),
        (b"recent = ['A/G/x.wren-board']", false),
        (b"last_open_note = 'A/G/x.wren-board'", false),
    ] {
        assert_eq!(
            released::accepts(bytes).is_ok(),
            accepted,
            "{}",
            String::from_utf8_lossy(bytes)
        );
    }
}

fn mixed_workspace() -> Workspace {
    Workspace::new(&[
        ("A/G/One.md", "one\n"),
        ("A/G/Flow.wren-board", BOARD),
        ("A/H/Two.md", "two\n"),
    ])
}

#[test]
fn back_and_forward_across_kinds_keep_boards_out_of_the_v1_keys() {
    let workspace = mixed_workspace();
    let mut session = workspace.open(true);
    for (index, path) in ["A/G/One.md", "A/G/Flow.wren-board", "A/H/Two.md"]
        .into_iter()
        .enumerate()
    {
        open(&mut session, path, index as u64);
        check(&workspace, &session, path);
    }
    for (time, op) in [(10, Op::Back), (11, Op::Back), (12, Op::Forward)] {
        let label = format!("{op:?} at {time}");
        step(&mut session, op, time);
        check(&workspace, &session, &label);
    }
    assert_eq!(document_path(&session), Some(rel("A/G/Flow.wren-board")));

    let table = workspace.table();
    assert_eq!(strings(table.get("history")), ["A/G/One.md", "A/H/Two.md"]);
    // The real position is on the board; the v1 one on the Markdown visit before it.
    assert_eq!(table["history_position"].as_integer(), Some(0));
    assert_eq!(strings(table.get("recent")), ["A/H/Two.md", "A/G/One.md"]);
    assert_eq!(table["last_open_note"].as_str(), Some("A/G/One.md"));
    assert_eq!(table["selection"].as_str(), Some("A/G/Flow.wren-board"));
    let navigation = table["navigation"].as_table().unwrap();
    assert_eq!(
        strings(navigation.get("history")),
        ["A/G/One.md", "A/G/Flow.wren-board", "A/H/Two.md"]
    );
    assert_eq!(navigation["history_position"].as_integer(), Some(1));
    assert_eq!(
        strings(navigation.get("recent")),
        ["A/H/Two.md", "A/G/One.md", "A/G/Flow.wren-board"]
    );
    assert_eq!(
        navigation["last_open"].as_str(),
        Some("A/G/Flow.wren-board")
    );
}

#[test]
fn quitting_with_a_board_open_reopens_it_and_without_boards_reopens_the_markdown_note() {
    let workspace = mixed_workspace();
    let mut session = workspace.open(true);
    open(&mut session, "A/G/One.md", 1);
    open(&mut session, "A/G/Flow.wren-board", 2);
    open(&mut session, "A/H/Two.md", 3);
    step(&mut session, Op::Back, 4);
    drop(session);
    let written = workspace.state_bytes().unwrap();

    let session = workspace.open(true);
    assert_eq!(document_path(&session), Some(rel("A/G/Flow.wren-board")));
    assert_eq!(
        session.state().history(),
        rels(&["A/G/One.md", "A/G/Flow.wren-board", "A/H/Two.md"])
    );
    assert_eq!(session.state().history_position(), Some(1));
    assert!(session.can_back() && session.can_forward());
    drop(session);
    assert_eq!(workspace.state_bytes().unwrap(), written, "open wrote");

    // With boards off this build reopens what released Wren reopens: the
    // Markdown note opened last, at its place in the Markdown history.
    let mut session = workspace.open(false);
    assert_eq!(document_path(&session), Some(rel("A/H/Two.md")));
    assert!(session.can_back() && !session.can_forward());
    assert_eq!(workspace.state_bytes().unwrap(), written, "open wrote");
    step(&mut session, Op::Back, 5);
    assert_eq!(document_path(&session), Some(rel("A/G/One.md")));
    check(&workspace, &session, "back with boards off");
    let table = workspace.table();
    assert!(!table.contains_key("navigation"), "{table}");
    assert_eq!(strings(table.get("history")), ["A/G/One.md", "A/H/Two.md"]);
}

#[test]
fn a_board_named_in_selection_reopens_after_restart() {
    let workspace = mixed_workspace();
    workspace.write(
        STATE,
        b"schema_version = 1\nselection = 'A/G/Flow.wren-board'\nlast_open_note = 'A/G/One.md'\n",
    );
    let session = workspace.open(true);
    assert_eq!(document_path(&session), Some(rel("A/G/Flow.wren-board")));
    drop(session);
    let session = workspace.open(false);
    assert_eq!(document_path(&session), Some(rel("A/G/One.md")));
}

#[test]
fn renaming_the_group_of_the_open_board_and_deleting_it_keep_the_file_valid() {
    let workspace = mixed_workspace();
    let mut session = workspace.open(true);
    open(&mut session, "A/G/One.md", 1);
    open(&mut session, "A/G/Flow.wren-board", 2);
    let g = group(&session, "A/G");
    step(
        &mut session,
        Op::Rename {
            target: ItemRef::Group(g),
            new_name: "Moved".into(),
        },
        3,
    );
    check(&workspace, &session, "rename the group");
    let table = workspace.table();
    assert_eq!(table["selection"].as_str(), Some("A/Moved/Flow.wren-board"));
    assert_eq!(
        table["navigation"]["last_open"].as_str(),
        Some("A/Moved/Flow.wren-board")
    );
    assert_eq!(table["last_open_note"].as_str(), Some("A/Moved/One.md"));

    let board = note(&session, "A/Moved/Flow.wren-board");
    step(
        &mut session,
        Op::Delete {
            target: ItemRef::Note(board),
        },
        4,
    );
    check(&workspace, &session, "delete the open board");
    let table = workspace.table();
    assert!(!table.contains_key("navigation"), "{table}");
    assert_eq!(strings(table.get("history")), ["A/Moved/One.md"]);
}

#[test]
fn navigation_left_stale_by_an_older_build_loses_to_the_v1_keys() {
    let workspace = mixed_workspace();
    // This build wrote [navigation]; an older build then visited Two, which
    // changed the v1 keys and kept the unknown table as it was.
    workspace.write(
        STATE,
        br#"schema_version = 1
history = ["A/G/One.md", "A/H/Two.md"]
history_position = 1
last_open_note = "A/H/Two.md"
recent = ["A/G/One.md", "A/H/Two.md"]
selection = "A/H/Two.md"

[navigation]
history = ["A/G/One.md", "A/G/Flow.wren-board"]
history_position = 1
last_open = "A/G/Flow.wren-board"
recent = ["A/G/One.md", "A/G/Flow.wren-board"]
"#,
    );
    let mut session = workspace.open(true);
    assert_eq!(document_path(&session), Some(rel("A/H/Two.md")));
    assert_eq!(
        session.state().history(),
        rels(&["A/G/One.md", "A/H/Two.md"])
    );
    assert_eq!(session.state().history_position(), Some(1));
    assert_eq!(session.state().last_open_note(), Some(rel("A/H/Two.md")));
    step(&mut session, Op::Back, 1);
    assert_eq!(document_path(&session), Some(rel("A/G/One.md")));
    check(&workspace, &session, "back");
    assert!(!workspace.table().contains_key("navigation"));
}

/// The ops whose state file `main` recorded in `fixtures/main-state.toml`.
/// `record_main_bytes` runs the same ops on `main`; keep the two in step.
fn scripted_ops(session: &mut Session) {
    let mut time = 0;
    let mut next = || {
        time += 1000;
        time
    };
    open(session, "A/G/One.md", next());
    open(session, "A/H/Two.md", next());
    step(session, Op::Back, next());
    open(session, "B/K/Three.md", next());
    let g = group(session, "A/G");
    step(
        session,
        Op::CreateNote {
            group: g,
            title: "Fresh".into(),
            kind: NoteKind::Markdown,
        },
        next(),
    );
    let two = note(session, "A/H/Two.md");
    step(
        session,
        Op::Rename {
            target: ItemRef::Note(two),
            new_name: "Second".into(),
        },
        next(),
    );
    let one = note(session, "A/G/One.md");
    let k = group(session, "B/K");
    step(session, Op::MoveNote { note: one, to: k }, next());
    let h = group(session, "A/H");
    step(
        session,
        Op::Rename {
            target: ItemRef::Group(h),
            new_name: "Hall".into(),
        },
        next(),
    );
    step(
        session,
        Op::SetWidth {
            pane: "notes".into(),
            width: 300,
        },
        next(),
    );
    step(
        session,
        Op::SetSort {
            group: k,
            mode: "title".into(),
        },
        next(),
    );
    let hall = group(session, "A/Hall");
    step(
        session,
        Op::Reorder {
            target: ItemRef::Group(hall),
            before: Some(ItemRef::Group(g)),
        },
        next(),
    );
    let b = section(session, "B");
    let a = section(session, "A");
    step(
        session,
        Op::Reorder {
            target: ItemRef::Section(b),
            before: Some(ItemRef::Section(a)),
        },
        next(),
    );
    open(session, "A/G/Fresh.md", next());
    let three = note(session, "B/K/Three.md");
    step(
        session,
        Op::Delete {
            target: ItemRef::Note(three),
        },
        next(),
    );
    open(session, "A/Hall/Second.md", next());
    step(session, Op::Back, next());
    step(session, Op::Back, next());
}

fn script_workspace(board: bool) -> Workspace {
    let mut files = vec![
        ("A/G/One.md", "one\n"),
        ("A/H/Two.md", "two\n"),
        ("B/K/Three.md", "three\n"),
    ];
    if board {
        files.push(("A/G/Flow.wren-board", BOARD));
    }
    Workspace::new(&files)
}

#[test]
fn with_boards_off_the_state_file_is_the_one_main_writes() {
    let workspace = script_workspace(true);
    let mut session = Session::with_trash(
        workspace.path(),
        Box::new(Bin {
            directory: TempDir::new().unwrap(),
            count: 0,
        }),
    )
    .unwrap();
    scripted_ops(&mut session);
    assert_eq!(
        String::from_utf8(workspace.state_bytes().unwrap()).unwrap(),
        String::from_utf8(MAIN_STATE.to_vec()).unwrap()
    );
}

#[test]
fn with_boards_on_and_none_on_disk_the_state_file_is_the_one_main_writes() {
    let workspace = script_workspace(false);
    let mut session = workspace.open(true);
    scripted_ops(&mut session);
    assert_eq!(
        String::from_utf8(workspace.state_bytes().unwrap()).unwrap(),
        String::from_utf8(MAIN_STATE.to_vec()).unwrap()
    );
}

/// A xorshift generator, so a failing walk replays from its seed.
struct Walk(u64);

impl Walk {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.below(items.len())])
    }
}

fn random_walk(seed: u64, steps: usize) {
    let workspace = Workspace::new(&[
        ("A/G/One.md", "one\n"),
        ("A/G/Flow.wren-board", BOARD),
        ("A/H/Two.md", "two\n"),
        ("B/K/Map.wren-board", BOARD),
        ("B/K/Three.md", "three\n"),
    ]);
    let mut walk = Walk(seed);
    let mut boards = true;
    let mut session = workspace.open(boards);
    let mut time = 0;
    let mut conflicts = 0;
    for index in 0..steps {
        time += 1000;
        // By path: the model orders notes by mtime, which would make the
        // walk depend on the clock.
        let mut notes: Vec<_> = session.model().notes.iter().collect();
        notes.sort_by(|left, right| left.path.cmp(&right.path));
        let notes: Vec<_> = notes.into_iter().map(|note| note.id).collect();
        let mut groups: Vec<_> = session.model().groups.iter().collect();
        groups.sort_by(|left, right| left.path.cmp(&right.path));
        let groups: Vec<_> = groups.into_iter().map(|group| group.id).collect();
        let op = match walk.below(12) {
            0 | 1 => walk
                .pick(&notes)
                .map(|note| Op::OpenNote { note: Some(note) }),
            2 => session.can_back().then_some(Op::Back),
            3 => session.can_forward().then_some(Op::Forward),
            4 => walk.pick(&groups).map(|group| Op::CreateNote {
                group,
                title: format!("N{index}"),
                kind: if boards && walk.below(2) == 0 {
                    NoteKind::Board
                } else {
                    NoteKind::Markdown
                },
            }),
            5 => walk.pick(&notes).map(|note| Op::Rename {
                target: ItemRef::Note(note),
                new_name: format!("R{index}"),
            }),
            6 => walk.pick(&notes).and_then(|note| {
                let from = session.model().note(note).unwrap().group;
                let others: Vec<_> = groups.iter().copied().filter(|g| *g != from).collect();
                walk.pick(&others).map(|to| Op::MoveNote { note, to })
            }),
            7 => (notes.len() > 3)
                .then(|| walk.pick(&notes))
                .flatten()
                .map(|note| Op::Delete {
                    target: ItemRef::Note(note),
                }),
            8 => walk.pick(&groups).map(|group| Op::Rename {
                target: ItemRef::Group(group),
                new_name: format!("G{index}"),
            }),
            9 => (notes.len() > 3)
                .then(|| walk.pick(&notes))
                .flatten()
                .map(|note| {
                    let path = session.model().note(note).unwrap().path.clone();
                    fs::remove_file(workspace.path().join(path.as_path())).unwrap();
                    Op::Reconcile
                }),
            // A dirty document changed on disk: its draft goes to a conflict
            // copy, which becomes the open note.
            10 => document_path(&session).map(|path| {
                let open = session.document().unwrap().note;
                time += 1;
                step(
                    &mut session,
                    Op::EditDraft {
                        note: open,
                        contents: format!("draft {index}\n"),
                    },
                    time,
                );
                workspace.write(path.as_path().to_str().unwrap(), b"external\n");
                Op::Reconcile
            }),
            _ => {
                drop(session);
                boards = walk.below(5) != 0;
                session = workspace.open(boards);
                // A note named in `selection` reopens, whatever its kind.
                let selection = workspace
                    .state_bytes()
                    .map(|_| workspace.table())
                    .and_then(|table| table.get("selection")?.as_str().map(rel));
                let listed = selection
                    .filter(|path| session.model().notes.iter().any(|note| note.path == *path));
                if let Some(selection) = listed {
                    assert_eq!(
                        document_path(&session),
                        Some(selection),
                        "seed {seed} step {index}: reopen with boards {boards}"
                    );
                }
                None
            }
        };
        let label = format!("seed {seed} step {index}: {op:?}");
        let before = workspace.state_bytes();
        if let Some(op) = op {
            let events = session
                .apply_at(op, Duration::from_millis(time))
                .unwrap_or_else(|error| panic!("{label}: {error}"));
            conflicts += events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        Event::ExternalChange {
                            kind: ExternalChangeKind::Conflict { .. },
                            ..
                        }
                    )
                })
                .count();
        }
        // Some steps change the Session's lists without saving them, as they
        // always have (opening prunes them, a reconcile can lose the open
        // note, opening the open note is a no-op); a step that saves must
        // save what the Session holds.
        if workspace.state_bytes() != before {
            check(&workspace, &session, &label);
        } else {
            check_file(&workspace, &label);
        }
    }
    assert!(conflicts > 0, "seed {seed}: the walk made no conflict copy");
}

#[test]
fn a_random_walk_over_mixed_kinds_keeps_the_state_file_loadable() {
    random_walk(0x5eed_b0a2d, 200);
}

#[test]
#[ignore = "the full count; run with --ignored"]
fn a_long_random_walk_over_mixed_kinds_keeps_the_state_file_loadable() {
    for seed in [0x5eed_b0a2d, 7, 0xdead_beef] {
        random_walk(seed, 2_000);
    }
}
