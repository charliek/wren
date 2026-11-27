//! The task index against a real workspace: every point at which the
//! search index takes a note's text keeps the task records current, and
//! `.wren/tasks.toml` is written only when something is set.

use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;
use wren_core::fs_store::RelPath;
use wren_core::index::{QueryKind, Scope};
use wren_core::kind::NoteKind;
use wren_core::model::{GroupId, ItemRef, NoteId, SectionId};
use wren_core::ops::{Event, ExternalChangeKind, Op, Session, SessionOptions, Trash};
use wren_core::tasks::{Counts, LeftOut, NoteFlags, TaskIndex};
use wren_core::tasks_file::{GroupBy, OwnerFilter, TaskFilters};
use wren_markdown::date::Date;
use wren_markdown::status::StatusType;

const TASKS_FILE: &str = ".wren/tasks.toml";

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

fn session_with(root: &Path, options: SessionOptions) -> Session {
    let trash = PrivateTrash {
        directory: TempDir::new().unwrap(),
        next: 0,
    };
    Session::with_options(root, Box::new(trash), options).unwrap()
}

/// A Session with tasks on.
fn session(root: &Path) -> Session {
    session_with(
        root,
        SessionOptions {
            tasks: true,
            ..SessionOptions::default()
        },
    )
}

fn write(root: &Path, path: &str, text: impl AsRef<[u8]>) {
    let absolute = root.join(path);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(absolute, text).unwrap();
}

const MONDAY: &str = "# Monday\n\
    - [ ] agenda 📅 2026-11-18\n\
    - [/] minutes @sam 📅 2026-11-19\n\
    - [x] room ✅ 2026-11-12\n";
const Q1: &str = "Plans.\n\n## Goals\n- [ ] hire ⏫\n- [-] reorg ❌ 2026-11-02\n";
const SHOP: &str = "- [ ] milk 📅 2026-11-17\n";

/// Two sections, three groups, four notes.
fn fixture() -> TempDir {
    let root = TempDir::new().unwrap();
    for (path, text) in [
        ("Work/Meetings/Monday.md", MONDAY),
        ("Work/Plans/Q1.md", Q1),
        ("Home/Lists/Shop.md", SHOP),
        ("Home/Lists/Diary.md", "Nothing to do today.\n"),
    ] {
        write(root.path(), path, text);
    }
    root
}

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

fn note(session: &Session, path: &str) -> NoteId {
    let found = (session.model().notes.iter()).find(|note| note.path == rel(path));
    found.unwrap_or_else(|| panic!("missing {path}")).id
}

fn section(session: &Session, path: &str) -> SectionId {
    let found = (session.model().sections.iter()).find(|section| section.path == rel(path));
    found.unwrap_or_else(|| panic!("missing {path}")).id
}

fn group(session: &Session, path: &str) -> GroupId {
    let found = (session.model().groups.iter()).find(|group| group.path == rel(path));
    found.unwrap_or_else(|| panic!("missing {path}")).id
}

fn run(session: &mut Session, op: Op) -> Vec<Event> {
    run_at(session, op, Duration::ZERO)
}

fn run_at(session: &mut Session, op: Op, now: Duration) -> Vec<Event> {
    session
        .apply_at(op, now)
        .unwrap_or_else(|error| panic!("{error}; committed events: {:?}", error.events))
}

fn open(session: &mut Session, note: NoteId) {
    run(session, Op::OpenNote { note: Some(note) });
}

fn edit(session: &mut Session, note: NoteId, text: &str) {
    let contents = text.into();
    run(session, Op::EditDraft { note, contents });
}

fn tasks(session: &Session) -> &TaskIndex {
    session.tasks().expect("tasks are on")
}

/// The tasks under `scope` as `path:line description`, in query order.
fn listed(session: &Session, scope: Option<&str>) -> Vec<String> {
    let scope = scope.map(rel);
    tasks(session)
        .tasks_under(scope.as_ref())
        .map(|task| {
            let path = task.path.as_path().display();
            format!("{path}:{} {}", task.line, task.description)
        })
        .collect()
}

/// Thursday 19 November 2026, the design page's day.
fn today() -> Date {
    Date::new(2026, 11, 19).unwrap()
}

fn counts(session: &Session, scope: Option<&str>) -> (usize, usize) {
    let scope = scope.map(rel);
    let Counts {
        open_and_doing,
        overdue,
    } = tasks(session).counts_under(scope.as_ref(), today());
    (open_and_doing, overdue)
}

#[test]
fn the_initial_scan_reads_every_markdown_notes_tasks_and_no_boards() {
    let root = fixture();
    write(
        root.path(),
        "Work/Plans/Flow.wren-board",
        "- [ ] a line of a board\n",
    );
    let session = session_with(
        root.path(),
        SessionOptions {
            tasks: true,
            boards: true,
        },
    );
    assert_eq!(session.model().notes.len(), 5);
    // In the tree's order: sections, their groups, each group's notes,
    // then by line.
    assert_eq!(
        listed(&session, None),
        [
            "Home/Lists/Shop.md:0 milk",
            "Work/Meetings/Monday.md:1 agenda",
            "Work/Meetings/Monday.md:2 minutes @sam",
            "Work/Meetings/Monday.md:3 room",
            "Work/Plans/Q1.md:3 hire",
            "Work/Plans/Q1.md:4 reorg",
        ]
    );
    assert_eq!(
        listed(&session, Some("Work/Plans")),
        ["Work/Plans/Q1.md:3 hire", "Work/Plans/Q1.md:4 reorg"]
    );
    assert_eq!(counts(&session, None), (4, 2));
    assert_eq!(counts(&session, Some("Work")), (3, 1));
    assert_eq!(counts(&session, Some("Work/Meetings")), (2, 1));
    assert_eq!(counts(&session, Some("Home/Lists/Diary.md")), (0, 0));
    let monday = note(&session, "Work/Meetings/Monday.md");
    let minutes = tasks(&session)
        .tasks_under(Some(&rel("Work/Meetings")))
        .nth(1)
        .unwrap();
    assert_eq!(minutes.note, monday);
    assert_eq!(minutes.source, "- [/] minutes @sam 📅 2026-11-19");
    assert_eq!(minutes.status, StatusType::Doing);
    assert_eq!(minutes.owners, ["sam"]);
    assert_eq!(minutes.heading.as_deref(), Some("Monday"));
    // The board is listed as a note and holds no tasks.
    let board = note(&session, "Work/Plans/Flow.wren-board");
    assert_eq!(session.model().note(board).unwrap().kind, NoteKind::Board);
    assert_eq!(tasks(&session).flags(board), None);
    assert_eq!(tasks(&session).left_out_under(None), LeftOut::default());
    // Opening wrote nothing.
    assert!(!root.path().join(TASKS_FILE).exists());
}

#[test]
fn with_tasks_off_no_index_is_built_and_no_file_is_read_or_written() {
    let root = fixture();
    let existing = "exclude = [\"Work\"]\n";
    write(root.path(), TASKS_FILE, existing);
    let mut session = session_with(root.path(), SessionOptions::default());
    assert!(session.tasks().is_none());
    assert!(session.tasks_file().is_none());
    assert!(session.model().warnings.is_empty());
    assert_eq!(session.task_filters(None), TaskFilters::default());
    // The ops that would write the file are refused.
    let work = section(&session, "Work");
    for op in [
        Op::SetTasksExcluded {
            target: ItemRef::Section(work),
            excluded: true,
        },
        Op::SetTaskFilters {
            scope: None,
            filters: TaskFilters::default(),
        },
    ] {
        let error = session.apply_at(op, Duration::ZERO).unwrap_err();
        assert!(error.message.contains("Tasks are turned off"), "{error}");
    }
    // A rename and a delete of what the file names leave it as it was.
    let target = ItemRef::Section(work);
    let new_name = "Job".into();
    run(&mut session, Op::Rename { target, new_name });
    let home = ItemRef::Section(section(&session, "Home"));
    run(&mut session, Op::Delete { target: home });
    let monday = note(&session, "Job/Meetings/Monday.md");
    open(&mut session, monday);
    edit(&mut session, monday, "- [ ] typed with tasks off\n");
    run(&mut session, Op::Flush);
    assert!(session.tasks().is_none());
    assert_eq!(
        fs::read_to_string(root.path().join(TASKS_FILE)).unwrap(),
        existing
    );
    // And with no file, none appears.
    let root = fixture();
    let mut session = session_with(root.path(), SessionOptions::default());
    let target = ItemRef::Section(section(&session, "Work"));
    let new_name = "Job".into();
    run(&mut session, Op::Rename { target, new_name });
    assert!(!root.path().join(TASKS_FILE).exists());
}

#[test]
fn the_open_notes_draft_is_the_source_of_its_records() {
    let root = fixture();
    let mut session = session(root.path());
    let q1 = note(&session, "Work/Plans/Q1.md");
    open(&mut session, q1);
    let revision = tasks(&session).revision();
    // Typed, not saved: the records are the draft's, the disk's are old.
    let draft = format!("# Now first\n{Q1}- [ ] typed @zoe\n");
    edit(&mut session, q1, &draft);
    assert!(session.document().unwrap().dirty());
    assert_eq!(
        fs::read_to_string(root.path().join("Work/Plans/Q1.md")).unwrap(),
        Q1
    );
    assert_eq!(
        listed(&session, Some("Work/Plans")),
        [
            "Work/Plans/Q1.md:4 hire",
            "Work/Plans/Q1.md:5 reorg",
            "Work/Plans/Q1.md:6 typed @zoe"
        ]
    );
    assert!(tasks(&session).revision() > revision);
    assert!(tasks(&session).holds_draft());
    assert_eq!(counts(&session, Some("Work/Plans")), (2, 0));
    // A reconcile while the draft is dirty reads the disk's old text and
    // must not put it back.
    run(&mut session, Op::Reconcile);
    assert_eq!(listed(&session, Some("Work/Plans")).len(), 3);
    // Prose typed changes no record: the revision stands.
    let revision = tasks(&session).revision();
    edit(&mut session, q1, &format!("{draft}\nA closing thought."));
    assert_eq!(tasks(&session).revision(), revision);
    // Saved, and then left: the same records, read from nowhere again.
    run(&mut session, Op::Flush);
    assert!(!session.document().unwrap().dirty());
    let read = tasks(&session).lines_read();
    let shop = note(&session, "Home/Lists/Shop.md");
    open(&mut session, shop);
    assert_eq!(listed(&session, Some("Work/Plans")).len(), 3);
    assert_eq!(tasks(&session).revision(), revision);
    assert_eq!(tasks(&session).lines_read(), read);
    assert!(!tasks(&session).holds_draft());
}

#[test]
fn a_synchronous_refresh_reads_what_changed_on_disk() {
    let root = fixture();
    let mut session = session(root.path());
    let revision = tasks(&session).revision();
    // A closed note edited by another program, and a note that is new.
    write(
        root.path(),
        "Home/Lists/Shop.md",
        "- [x] milk ✅ 2026-11-19\n- [ ] eggs\n",
    );
    write(
        root.path(),
        "Home/Lists/New.md",
        "- [ ] found by the scan\n",
    );
    run(&mut session, Op::Reconcile);
    let mut home = listed(&session, Some("Home"));
    home.sort();
    assert_eq!(
        home,
        [
            "Home/Lists/New.md:0 found by the scan",
            "Home/Lists/Shop.md:0 milk",
            "Home/Lists/Shop.md:1 eggs",
        ]
    );
    assert_eq!(counts(&session, Some("Home")), (2, 0));
    assert!(tasks(&session).revision() > revision);
    // A note made by the Session is read as its template stands.
    let lists = group(&session, "Home/Lists");
    run(
        &mut session,
        Op::CreateNote {
            group: lists,
            title: "Made".into(),
            kind: NoteKind::Markdown,
        },
    );
    let made = note(&session, "Home/Lists/Made.md");
    assert_eq!(tasks(&session).flags(made), Some(NoteFlags::default()));
    // A note whose front matter turns its tasks off, as written outside.
    write(
        root.path(),
        "Home/Lists/Shop.md",
        "---\ntasks: off\n---\n- [ ] eggs\n",
    );
    run(&mut session, Op::Reconcile);
    assert_eq!(
        listed(&session, Some("Home")),
        ["Home/Lists/New.md:0 found by the scan"]
    );
    let shop = note(&session, "Home/Lists/Shop.md");
    assert!(tasks(&session).flags(shop).unwrap().hidden);
    assert_eq!(
        tasks(&session).left_out_under(Some(&rel("Home"))),
        LeftOut {
            hidden_notes: 1,
            hidden_tasks: 1,
            ..LeftOut::default()
        }
    );
}

/// Tick until the Session hands out a background job.
fn begin(session: &mut Session, clock: &mut Duration) -> wren_core::reconcile::ReconcileJob {
    *clock += Duration::from_secs(2);
    run_at(session, Op::Tick, *clock);
    session.take_reconcile_job().expect("a job is due")
}

#[test]
fn a_background_scans_result_is_merged_when_it_completes() {
    let root = fixture();
    let mut session = session(root.path());
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    write(
        root.path(),
        "Home/Lists/Shop.md",
        "- [ ] eggs\n- [ ] flour\n",
    );
    write(root.path(), "Home/Lists/New.md", "- [/] found later\n");
    let job = begin(&mut session, &mut clock);
    // Nothing is known until the job's result arrives.
    assert_eq!(
        listed(&session, Some("Home")),
        ["Home/Lists/Shop.md:0 milk"]
    );
    let result = std::thread::spawn(move || job.run()).join().unwrap();
    run_at(&mut session, Op::FinishReconcile(result), clock);
    let mut home = listed(&session, Some("Home"));
    home.sort();
    assert_eq!(
        home,
        [
            "Home/Lists/New.md:0 found later",
            "Home/Lists/Shop.md:0 eggs",
            "Home/Lists/Shop.md:1 flour",
        ]
    );
    // The next job finds the disk as the index has it and sends nothing.
    let revision = tasks(&session).revision();
    let read = tasks(&session).lines_read();
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert_eq!(tasks(&session).revision(), revision);
    assert_eq!(tasks(&session).lines_read(), read);
}

#[test]
fn a_late_scan_cannot_overwrite_a_newer_draft_or_a_newer_write() {
    let root = fixture();
    let mut session = session(root.path());
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let q1 = note(&session, "Work/Plans/Q1.md");
    let shop = note(&session, "Home/Lists/Shop.md");
    let diary = note(&session, "Home/Lists/Diary.md");

    // A newer draft. The open note is dirty when the job starts, so the
    // job reads disk text that differs from the draft it was given.
    open(&mut session, q1);
    run_at(
        &mut session,
        Op::EditDraft {
            note: q1,
            contents: "- [ ] first draft\n".into(),
        },
        clock,
    );
    // Other programs write two closed notes before the job reads them.
    write(root.path(), "Home/Lists/Shop.md", "- [ ] stale shop\n");
    write(root.path(), "Home/Lists/Diary.md", "- [ ] from the scan\n");
    clock += Duration::from_millis(100);
    run_at(&mut session, Op::Tick, clock);
    let job = session.take_reconcile_job().expect("a job is due");
    assert!(session.document().unwrap().dirty(), "not yet saved");
    let result = job.run();
    // While the result is on its way: the draft moves on,
    run_at(
        &mut session,
        Op::EditDraft {
            note: q1,
            contents: "- [ ] second draft\n".into(),
        },
        clock,
    );
    // and one of the closed notes is opened, edited, saved and left.
    run_at(&mut session, Op::OpenNote { note: Some(shop) }, clock);
    assert_eq!(
        listed(&session, Some("Home")),
        ["Home/Lists/Shop.md:0 stale shop"]
    );
    run_at(
        &mut session,
        Op::EditDraft {
            note: shop,
            contents: "- [x] written since ✅ 2026-11-19\n".into(),
        },
        clock,
    );
    run_at(&mut session, Op::Flush, clock);
    run_at(&mut session, Op::OpenNote { note: Some(q1) }, clock);
    run_at(
        &mut session,
        Op::EditDraft {
            note: q1,
            contents: "- [ ] third draft\n".into(),
        },
        clock,
    );
    let events = run_at(&mut session, Op::FinishReconcile(result), clock);
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::ExternalChange { .. })));
    // The result was merged: the note only it knew about is read.
    assert_eq!(
        listed(&session, Some("Home/Lists/Diary.md")),
        ["Home/Lists/Diary.md:0 from the scan"]
    );
    assert!(tasks(&session).flags(diary).is_some());
    // Its older reads of the two notes written since were refused.
    assert_eq!(
        listed(&session, Some("Home/Lists/Shop.md")),
        ["Home/Lists/Shop.md:0 written since"]
    );
    assert_eq!(
        listed(&session, Some("Work/Plans")),
        ["Work/Plans/Q1.md:0 third draft"]
    );
    assert_eq!(
        fs::read_to_string(root.path().join("Home/Lists/Shop.md")).unwrap(),
        "- [x] written since ✅ 2026-11-19\n"
    );
    // The next scan agrees with all of it.
    run_at(&mut session, Op::Flush, clock);
    let revision = tasks(&session).revision();
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert_eq!(tasks(&session).revision(), revision);
    assert_eq!(listed(&session, None).len(), 6);
}

/// Opens `note`, replaces its text, saves it and leaves it for `then`,
/// as an owner at work while a scan's result is on its way.
fn write_and_leave(session: &mut Session, note: NoteId, text: &str, then: NoteId, now: Duration) {
    run_at(session, Op::OpenNote { note: Some(note) }, now);
    let contents = text.into();
    run_at(session, Op::EditDraft { note, contents }, now);
    run_at(session, Op::Flush, now);
    run_at(session, Op::OpenNote { note: Some(then) }, now);
}

#[test]
fn a_late_failed_scan_cannot_clear_what_was_written_since() {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture();
    let mut session = session(root.path());
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let shop = note(&session, "Home/Lists/Shop.md");
    let diary = note(&session, "Home/Lists/Diary.md");
    let monday = note(&session, "Work/Meetings/Monday.md");
    // The job cannot list the root; by the time its failure arrives the
    // root can be read again, and a task has been written.
    let job = begin(&mut session, &mut clock);
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o000)).unwrap();
    let result = job.run();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    write_and_leave(
        &mut session,
        shop,
        "- [x] written since ✅ 2026-11-19\n- [ ] and one more\n",
        diary,
        clock,
    );
    let finished = session.apply_at(Op::FinishReconcile(result), clock);
    assert!(
        finished.is_err(),
        "The permission test must run unprivileged"
    );
    // The note read and written after the scan began is as it was left.
    assert_eq!(
        listed(&session, Some("Home/Lists/Shop.md")),
        [
            "Home/Lists/Shop.md:0 written since",
            "Home/Lists/Shop.md:1 and one more"
        ]
    );
    assert_eq!(tasks(&session).flags(shop), Some(NoteFlags::default()));
    assert_eq!(tasks(&session).flags(diary), Some(NoteFlags::default()));
    assert_eq!(counts(&session, Some("Home")), (1, 0));
    // What only the failed scan would have read is not known until the
    // next one, which reads it again.
    assert!(tasks(&session).flags(monday).unwrap().unreadable);
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert_eq!(tasks(&session).left_out_under(None), LeftOut::default());
    assert_eq!(listed(&session, None).len(), 7);
    assert_eq!(listed(&session, Some("Home/Lists/Shop.md")).len(), 2);
}

#[test]
fn a_late_scan_that_missed_a_note_cannot_take_away_what_was_written_since() {
    let root = fixture();
    let mut session = session(root.path());
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let shop = note(&session, "Home/Lists/Shop.md");
    let diary = note(&session, "Home/Lists/Diary.md");
    // The note is away while the job scans, as while another program
    // replaces it, and back before the result arrives.
    let job = begin(&mut session, &mut clock);
    let (there, away) = (
        root.path().join("Home/Lists/Shop.md"),
        root.path().join("Home/Lists/Shop.away"),
    );
    fs::rename(&there, &away).unwrap();
    let result = job.run();
    fs::rename(&away, &there).unwrap();
    // A second change the scan did see, so that dropping its result is
    // told from merging it.
    write_and_leave(&mut session, shop, "- [ ] written since\n", diary, clock);
    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert!(session.model().note(shop).is_some(), "the note is listed");
    assert_eq!(
        listed(&session, Some("Home/Lists/Shop.md")),
        ["Home/Lists/Shop.md:0 written since"]
    );
    assert_eq!(counts(&session, Some("Home")), (1, 0));
    // The next scan finds the note and agrees.
    let revision = tasks(&session).revision();
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert_eq!(tasks(&session).revision(), revision);
    assert_eq!(session.model().notes.len(), 4);
    assert_eq!(fs::read_to_string(&there).unwrap(), "- [ ] written since\n");
}

#[test]
fn a_late_scan_cannot_bring_back_a_note_found_deleted() {
    let root = fixture();
    let mut session = session(root.path());
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let shop = note(&session, "Home/Lists/Shop.md");
    // The open note, clean, is changed by another program, and so is a
    // closed one: the job reads both changes. Then the open note is
    // deleted, before the result arrives.
    run_at(&mut session, Op::OpenNote { note: Some(shop) }, clock);
    write(root.path(), "Home/Lists/Shop.md", "- [ ] changed outside\n");
    write(
        root.path(),
        "Home/Lists/Diary.md",
        "- [ ] read by the job\n",
    );
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    fs::remove_file(root.path().join("Home/Lists/Shop.md")).unwrap();
    let events = run_at(&mut session, Op::FinishReconcile(result), clock);
    assert!(deleted(&events));
    assert!(session.document().is_none());
    // The text the job read of it is not the note's: the note is gone,
    // from the tree too, and at once.
    assert!(session.model().note(shop).is_none());
    assert_eq!(tasks(&session).flags(shop), None);
    assert!(titled(&session, "Shop").is_empty());
    // The rest of what the job read is merged with no other scan.
    assert_eq!(
        listed(&session, Some("Home")),
        ["Home/Lists/Diary.md:0 read by the job"]
    );
    assert_eq!(counts(&session, Some("Home")), (1, 0));
    assert_eq!(listed(&session, None).len(), 6);
    assert_eq!(tasks(&session).left_out_under(None), LeftOut::default());
}

fn deleted(events: &[Event]) -> bool {
    events.iter().any(|event| {
        let kind = ExternalChangeKind::Deleted;
        matches!(event, Event::ExternalChange { kind: found, .. } if *found == kind)
    })
}

/// The notes a title search for `title` finds.
fn titled(session: &Session, title: &str) -> Vec<NoteId> {
    let hits = (session.index()).query(title, Scope::Workspace, QueryKind::Jump, 10);
    hits.into_iter().map(|hit| hit.note).collect()
}

/// A Session with tasks on or off: what the shared reconcile paths do
/// for the search index either way.
fn either(root: &Path, tasks: bool) -> Session {
    session_with(
        root,
        SessionOptions {
            tasks,
            ..SessionOptions::default()
        },
    )
}

#[test]
fn a_scan_that_agrees_with_a_fresh_deletion_is_merged_at_once() {
    for tasks_on in [false, true] {
        let root = fixture();
        let mut session = either(root.path(), tasks_on);
        session.set_background_reconcile(true);
        let mut clock = Duration::from_secs(10);
        let shop = note(&session, "Home/Lists/Shop.md");
        let q1 = note(&session, "Work/Plans/Q1.md");
        // The open note, clean, is deleted before the job scans, which
        // also reads another note's change.
        run_at(&mut session, Op::OpenNote { note: Some(shop) }, clock);
        fs::remove_file(root.path().join("Home/Lists/Shop.md")).unwrap();
        write(root.path(), "Work/Plans/Q1.md", "- [ ] changed outside\n");
        let job = begin(&mut session, &mut clock);
        let result = job.run();
        let events = run_at(&mut session, Op::FinishReconcile(result), clock);
        assert!(deleted(&events), "tasks: {tasks_on}");
        // Both looks found no file: nothing is held back for another scan.
        assert!(session.model().note(shop).is_none(), "tasks: {tasks_on}");
        assert!(titled(&session, "Shop").is_empty(), "tasks: {tasks_on}");
        assert_eq!(
            session.index().content(q1),
            Some("- [ ] changed outside\n"),
            "tasks: {tasks_on}"
        );
        if tasks_on {
            assert_eq!(
                listed(&session, Some("Work/Plans")),
                ["Work/Plans/Q1.md:0 changed outside"]
            );
            assert_eq!(tasks(&session).flags(shop), None);
        }
    }
}

#[test]
fn a_failed_scan_delivered_with_a_fresh_deletion_still_fails() {
    use std::os::unix::fs::PermissionsExt;
    for tasks_on in [false, true] {
        let root = fixture();
        let mut session = either(root.path(), tasks_on);
        session.set_background_reconcile(true);
        let mut clock = Duration::from_secs(10);
        let shop = note(&session, "Home/Lists/Shop.md");
        let monday = note(&session, "Work/Meetings/Monday.md");
        run_at(&mut session, Op::OpenNote { note: Some(shop) }, clock);
        // The job cannot list the root. Before its failure arrives the
        // root can be read again, and the open note is deleted.
        let job = begin(&mut session, &mut clock);
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o000)).unwrap();
        let result = job.run();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(root.path().join("Home/Lists/Shop.md")).unwrap();
        let finished = session.apply_at(Op::FinishReconcile(result), clock);
        let error = finished.expect_err("The permission test must run unprivileged");
        assert!(deleted(&error.events), "tasks: {tasks_on}");
        // The failure is told, and what the failed scan would have read
        // is not known until the next one.
        assert_eq!(session.index().content(monday), None, "tasks: {tasks_on}");
        assert!(session
            .index()
            .limitation(monday)
            .is_some_and(|reason| reason.contains("scan failed")));
        assert_eq!(session.index().content(shop), None);
        if tasks_on {
            assert!(tasks(&session).flags(monday).unwrap().unreadable);
            assert!(listed(&session, None).is_empty());
        }
        let job = begin(&mut session, &mut clock);
        let result = job.run();
        run_at(&mut session, Op::FinishReconcile(result), clock);
        assert_eq!(session.index().content(monday), Some(MONDAY));
        assert!(session.model().note(shop).is_none());
    }
}

#[test]
fn a_note_found_missing_on_opening_does_not_hold_back_the_scan_that_agrees() {
    for tasks_on in [false, true] {
        let root = fixture();
        let mut session = either(root.path(), tasks_on);
        session.set_background_reconcile(true);
        let mut clock = Duration::from_secs(10);
        let shop = note(&session, "Home/Lists/Shop.md");
        let q1 = note(&session, "Work/Plans/Q1.md");
        // A closed note is deleted and another changed. Each job sees
        // both, and each time the owner tries the deleted note before the
        // result arrives: on a disk that stands still, the first result
        // settles it.
        fs::remove_file(root.path().join("Home/Lists/Shop.md")).unwrap();
        write(root.path(), "Work/Plans/Q1.md", "- [ ] changed outside\n");
        let job = begin(&mut session, &mut clock);
        let result = job.run();
        let opened = session.apply_at(Op::OpenNote { note: Some(shop) }, clock);
        assert!(opened.is_err(), "the note has no file");
        run_at(&mut session, Op::FinishReconcile(result), clock);
        assert!(session.model().note(shop).is_none(), "tasks: {tasks_on}");
        assert!(titled(&session, "Shop").is_empty(), "tasks: {tasks_on}");
        assert_eq!(
            session.index().content(q1),
            Some("- [ ] changed outside\n"),
            "tasks: {tasks_on}"
        );
        if tasks_on {
            assert_eq!(tasks(&session).flags(shop), None);
            assert_eq!(listed(&session, Some("Work/Plans")).len(), 1);
        }
    }
    // A note the owner found, and wrote, is still not taken away by a
    // job that missed it; one found missing first and written after is
    // such a note again.
    let root = fixture();
    let mut session = session(root.path());
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let shop = note(&session, "Home/Lists/Shop.md");
    let diary = note(&session, "Home/Lists/Diary.md");
    let there = root.path().join("Home/Lists/Shop.md");
    let job = begin(&mut session, &mut clock);
    fs::remove_file(&there).unwrap();
    let result = job.run();
    assert!(session
        .apply_at(Op::OpenNote { note: Some(shop) }, clock)
        .is_err());
    fs::write(&there, SHOP).unwrap();
    write_and_leave(&mut session, shop, "- [ ] written since\n", diary, clock);
    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert!(session.model().note(shop).is_some());
    assert_eq!(
        listed(&session, Some("Home/Lists/Shop.md")),
        ["Home/Lists/Shop.md:0 written since"]
    );
}

#[test]
fn a_touched_note_deleted_since_does_not_hold_back_the_scan_that_found_it_gone() {
    for (tasks_on, restored) in [(false, false), (true, false), (false, true), (true, true)] {
        let case = format!("tasks: {tasks_on}, restored: {restored}");
        let root = fixture();
        let mut session = either(root.path(), tasks_on);
        session.set_background_reconcile(true);
        let mut clock = Duration::from_secs(10);
        let shop = note(&session, "Home/Lists/Shop.md");
        let diary = note(&session, "Home/Lists/Diary.md");
        let there = root.path().join("Home/Lists/Shop.md");
        // The note is written and left while the job is out. Then another
        // program deletes it and adds a note, and the job sees both.
        let job = begin(&mut session, &mut clock);
        write_and_leave(&mut session, shop, "- [ ] written since\n", diary, clock);
        fs::remove_file(&there).unwrap();
        write(
            root.path(),
            "Home/Lists/Errands.md",
            "- [ ] post the parcel\n",
        );
        let result = job.run();
        if restored {
            fs::write(&there, "- [ ] written since\n").unwrap();
        }
        run_at(&mut session, Op::FinishReconcile(result), clock);
        let errands = (session.model().notes.iter())
            .find(|note| note.path == rel("Home/Lists/Errands.md"))
            .map(|note| note.id);
        if restored {
            // The note is there again: the result is older than it, and
            // waits for the next scan.
            assert!(session.model().note(shop).is_some(), "{case}");
            assert_eq!(errands, None, "{case}");
            if tasks_on {
                assert_eq!(
                    listed(&session, Some("Home/Lists/Shop.md")),
                    ["Home/Lists/Shop.md:0 written since"]
                );
            }
            let job = begin(&mut session, &mut clock);
            let result = job.run();
            run_at(&mut session, Op::FinishReconcile(result), clock);
            assert!(session.model().note(shop).is_some(), "{case}");
            assert_eq!(titled(&session, "Errands").len(), 1, "{case}");
            continue;
        }
        // Gone indeed: the result is merged at once, the deletion and the
        // new note with it.
        assert!(session.model().note(shop).is_none(), "{case}");
        assert!(titled(&session, "Shop").is_empty(), "{case}");
        let errands = errands.unwrap_or_else(|| panic!("the new note is listed; {case}"));
        assert_eq!(titled(&session, "Errands"), [errands], "{case}");
        assert_eq!(
            session.index().content(errands),
            Some("- [ ] post the parcel\n"),
            "{case}"
        );
        if tasks_on {
            assert_eq!(tasks(&session).flags(shop), None);
            assert_eq!(
                listed(&session, Some("Home")),
                ["Home/Lists/Errands.md:0 post the parcel"]
            );
            assert_eq!(counts(&session, Some("Home")), (1, 0));
        }
    }
}

#[test]
fn a_failed_open_takes_the_files_time_and_size_into_the_model() {
    use std::fs::{File, FileTimes};
    use std::time::SystemTime;
    for tasks_on in [false, true] {
        let root = fixture();
        let mut session = either(root.path(), tasks_on);
        let q1 = note(&session, "Work/Plans/Q1.md");
        // The note is replaced by bytes that are no text, of another
        // length and time. No scan has seen it: opening it fails, and the
        // read that follows finds the file as it is.
        let path = root.path().join("Work/Plans/Q1.md");
        let replaced = [b'-', b' ', 0xff, 0xfe, b'\n'];
        fs::write(&path, replaced).unwrap();
        let changed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let times = FileTimes::new().set_modified(changed);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(times)
            .unwrap();
        let before = session.model().note(q1).unwrap().clone();
        assert_ne!((before.modified, before.bytes), (changed, replaced.len()));
        assert!(session
            .apply_at(Op::OpenNote { note: Some(q1) }, Duration::ZERO)
            .is_err());
        let after = session.model().note(q1).unwrap();
        assert_eq!(
            (after.modified, after.bytes),
            (changed, replaced.len()),
            "tasks: {tasks_on}"
        );
        assert_eq!(session.index().content(q1), None);
    }
}

/// A board with one labelled shape.
fn board(label: &str) -> String {
    format!(
        "schema = 1\n\n[node.a]\ntext = \"{label}\"\nkind = \"shape\"\n\
         at = [0, 0]\nsize = [120, 60]\n"
    )
}

#[test]
fn a_deleted_board_does_not_come_back_from_the_source_that_waited() {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture();
    write(
        root.path(),
        "Work/Plans/Flow.wren-board",
        board("Alpha label"),
    );
    let mut session = session_with(
        root.path(),
        SessionOptions {
            boards: true,
            tasks: false,
        },
    );
    session.set_background_reconcile(true);
    let flow = note(&session, "Work/Plans/Flow.wren-board");
    let at = |millis: u64| Duration::from_millis(10_000 + millis);
    run_at(&mut session, Op::OpenNote { note: Some(flow) }, at(0));
    // An edit that search takes once it has rested, with the job begun:
    // not yet saved.
    let contents = board("Edited label");
    run_at(
        &mut session,
        Op::EditDraft {
            note: flow,
            contents,
        },
        at(0),
    );
    run_at(&mut session, Op::Tick, at(300));
    let job = session.take_reconcile_job().expect("a job is due");
    assert_eq!(session.index().content(flow), Some("Edited label"));
    // Undone: the board is as on disk, and its source waits to be read.
    let contents = board("Alpha label");
    run_at(
        &mut session,
        Op::EditDraft {
            note: flow,
            contents,
        },
        at(400),
    );
    assert!(!session.document().unwrap().dirty());
    // The job fails; the board is deleted before its failure arrives,
    // after the wait.
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o000)).unwrap();
    let result = job.run();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_file(root.path().join("Work/Plans/Flow.wren-board")).unwrap();
    let finished = session.apply_at(Op::FinishReconcile(result), at(1_000));
    let error = finished.expect_err("The permission test must run unprivileged");
    assert!(deleted(&error.events));
    assert!(session.document().is_none());
    session.settle_search();
    assert_eq!(session.index().content(flow), None);
    let found = (session.index()).query("label", Scope::Workspace, QueryKind::Search, 10);
    assert!(found.is_empty(), "{found:?}");
    // Nor does a later op bring it back.
    run_at(&mut session, Op::Tick, at(1_100));
    assert_eq!(session.index().content(flow), None);
}

#[test]
fn a_note_that_fails_to_open_gives_up_the_tasks_it_was_read_with() {
    let root = fixture();
    let mut session = session(root.path());
    let monday = note(&session, "Work/Meetings/Monday.md");
    let q1 = note(&session, "Work/Plans/Q1.md");
    open(&mut session, monday);
    edit(&mut session, monday, &format!("{MONDAY}- [ ] typed\n"));
    // A closed note becomes bytes that are no text; no scan has seen it.
    write(root.path(), "Work/Plans/Q1.md", [b'-', b' ', 0xff, 0xfe]);
    assert_eq!(listed(&session, Some("Work/Plans")).len(), 2);
    let error = session
        .apply_at(Op::OpenNote { note: Some(q1) }, Duration::ZERO)
        .unwrap_err();
    assert!(error.message.contains("not valid UTF-8"), "{error}");
    // The open document stays, draft and all.
    assert_eq!(session.document().unwrap().note, monday);
    assert_eq!(listed(&session, Some("Work/Meetings")).len(), 4);
    // The note that would not open gives none of its old tasks.
    assert!(listed(&session, Some("Work/Plans")).is_empty());
    assert!(tasks(&session).flags(q1).unwrap().unreadable);
    assert_eq!(counts(&session, Some("Work/Plans")), (0, 0));
    assert_eq!(session.index().content(q1), None);
    // Text again, it is read by the next scan and opens.
    write(root.path(), "Work/Plans/Q1.md", Q1);
    run(&mut session, Op::Reconcile);
    assert_eq!(listed(&session, Some("Work/Plans")).len(), 2);
    open(&mut session, q1);
    assert_eq!(session.document().unwrap().note, q1);
}

#[test]
fn a_note_edited_and_left_is_not_read_again_once_search_drops_its_text() {
    let root = fixture();
    let mut session = session(root.path());
    let q1 = note(&session, "Work/Plans/Q1.md");
    let shop = note(&session, "Home/Lists/Shop.md");
    open(&mut session, q1);
    for typed in ["-", "- [ ] typed", "- [ ] typed @zoe\n"] {
        edit(&mut session, q1, &format!("{Q1}{typed}"));
    }
    run(&mut session, Op::Flush);
    open(&mut session, shop);
    assert_eq!(listed(&session, Some("Work/Plans")).len(), 3);
    // Search drops every closed note's text; scans send each again.
    session.limit_search_content(1);
    assert_eq!(session.index().content(q1), None);
    let revision = tasks(&session).revision();
    let read = tasks(&session).lines_read();
    for _ in 0..3 {
        let _ = session.apply_at(Op::Reconcile, Duration::ZERO);
    }
    assert_eq!(tasks(&session).lines_read(), read);
    assert_eq!(tasks(&session).revision(), revision);
    assert_eq!(listed(&session, Some("Work/Plans")).len(), 3);
}

#[test]
fn a_rename_a_move_and_a_delete_carry_the_records_and_the_settings() {
    let root = fixture();
    let mut session = session(root.path());
    let plans = group(&session, "Work/Plans");
    let lists = group(&session, "Home/Lists");
    let meetings = group(&session, "Work/Meetings");
    // Settings that name the paths about to change.
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: ItemRef::Group(lists),
            excluded: true,
        },
    );
    let by_owner = TaskFilters {
        group_by: GroupBy::Owner,
        owner: OwnerFilter::Name("sam".into()),
        ..TaskFilters::default()
    };
    for scope in [
        Some(ItemRef::Group(meetings)),
        Some(ItemRef::Section(section(&session, "Work"))),
        None,
    ] {
        let filters = by_owner.clone();
        run(&mut session, Op::SetTaskFilters { scope, filters });
    }
    assert_eq!(listed(&session, Some("Home")), [] as [&str; 0]);
    assert_eq!(counts(&session, None), (3, 1));

    // Sections reordered: the records follow the tree.
    assert_eq!(
        listed(&session, None)[0],
        "Work/Meetings/Monday.md:1 agenda"
    );
    let revision = tasks(&session).revision();
    run(
        &mut session,
        Op::Reorder {
            target: ItemRef::Group(plans),
            before: Some(ItemRef::Group(meetings)),
        },
    );
    assert_eq!(listed(&session, None)[0], "Work/Plans/Q1.md:3 hire");
    assert!(tasks(&session).revision() > revision);

    // A section renamed: its notes' records follow, with their paths.
    let read = tasks(&session).lines_read();
    let target = ItemRef::Section(section(&session, "Work"));
    let new_name = "Job".into();
    run(&mut session, Op::Rename { target, new_name });
    assert!(listed(&session, Some("Work")).is_empty());
    assert_eq!(
        listed(&session, Some("Job/Plans")),
        ["Job/Plans/Q1.md:3 hire", "Job/Plans/Q1.md:4 reorg"]
    );
    assert_eq!(counts(&session, Some("Job")), (3, 1));
    assert_eq!(tasks(&session).lines_read(), read, "a rename reads no note");
    assert_eq!(
        session.task_filters(Some(ItemRef::Group(meetings))),
        by_owner
    );
    let saved = fs::read_to_string(root.path().join(TASKS_FILE)).unwrap();
    assert!(saved.contains("[views.\"Job/Meetings\"]"), "{saved}");
    assert!(saved.contains("[views.Job]"), "{saved}");
    assert!(!saved.contains("Work"), "{saved}");

    // The excluded group renamed: still excluded, under its new name.
    let target = ItemRef::Group(lists);
    let new_name = "Errands".into();
    run(&mut session, Op::Rename { target, new_name });
    assert_eq!(tasks(&session).excluded(), [rel("Home/Errands")]);
    assert!(listed(&session, Some("Home")).is_empty());
    assert_eq!(
        tasks(&session).left_out_under(None),
        LeftOut {
            excluded_notes: 1,
            excluded_tasks: 1,
            ..LeftOut::default()
        }
    );
    // A note moved into the excluded group is left out; moved back, shown.
    let q1 = note(&session, "Job/Plans/Q1.md");
    run(
        &mut session,
        Op::MoveNote {
            note: q1,
            to: lists,
        },
    );
    assert!(listed(&session, Some("Job/Plans")).is_empty());
    assert!(tasks(&session).flags(q1).unwrap().excluded);
    assert_eq!(counts(&session, None), (2, 1));
    run(
        &mut session,
        Op::MoveNote {
            note: q1,
            to: plans,
        },
    );
    assert_eq!(listed(&session, Some("Job/Plans")).len(), 2);
    // A note renamed keeps its records under its new path.
    let target = ItemRef::Note(q1);
    let new_name = "Quarter".into();
    run(&mut session, Op::Rename { target, new_name });
    assert_eq!(
        listed(&session, Some("Job/Plans/Quarter.md")),
        [
            "Job/Plans/Quarter.md:3 hire",
            "Job/Plans/Quarter.md:4 reorg"
        ]
    );

    // Deleted: a note's records go, and a folder's settings with it.
    run(
        &mut session,
        Op::Delete {
            target: ItemRef::Note(q1),
        },
    );
    assert!(listed(&session, Some("Job/Plans")).is_empty());
    assert_eq!(tasks(&session).flags(q1), None);
    run(
        &mut session,
        Op::Delete {
            target: ItemRef::Group(meetings),
        },
    );
    run(
        &mut session,
        Op::Delete {
            target: ItemRef::Group(lists),
        },
    );
    assert!(listed(&session, None).is_empty());
    assert_eq!(counts(&session, None), (0, 0));
    assert!(tasks(&session).excluded().is_empty());
    let saved = fs::read_to_string(root.path().join(TASKS_FILE)).unwrap();
    assert_eq!(
        saved,
        "schema_version = 1\n\n[views.\".\"]\ngroup = \"owner\"\nowner = \"@sam\"\n\n\
         [views.Job]\ngroup = \"owner\"\nowner = \"@sam\"\n"
    );
    // What was saved is what the next Session reads.
    drop(session);
    let session = self::session(root.path());
    assert_eq!(session.task_filters(None), by_owner);
    assert!(session.model().warnings.is_empty());
}

#[test]
fn a_note_that_cannot_be_read_gives_no_tasks_until_it_can() {
    let root = fixture();
    let mut session = session(root.path());
    let shop = note(&session, "Home/Lists/Shop.md");
    // Bytes that are no text.
    write(root.path(), "Home/Lists/Shop.md", [0xff, 0xfe, b'-', b' ']);
    let _ = session.apply_at(Op::Reconcile, Duration::ZERO);
    assert!(listed(&session, Some("Home")).is_empty());
    assert_eq!(
        tasks(&session).flags(shop),
        Some(NoteFlags {
            unreadable: true,
            ..NoteFlags::default()
        })
    );
    assert_eq!(tasks(&session).left_out_under(None).unreadable_notes, 1);
    assert_eq!(counts(&session, None), (3, 1));
    // The other notes' records stand.
    assert_eq!(listed(&session, Some("Work")).len(), 5);
    // Readable again, with the bytes it had: read again.
    write(root.path(), "Home/Lists/Shop.md", SHOP);
    run(&mut session, Op::Reconcile);
    assert_eq!(
        listed(&session, Some("Home")),
        ["Home/Lists/Shop.md:0 milk"]
    );
    assert_eq!(tasks(&session).left_out_under(None), LeftOut::default());
}

#[test]
fn search_content_dropped_for_its_budget_leaves_the_task_records() {
    let root = fixture();
    let mut session = session(root.path());
    let before = listed(&session, None);
    assert_eq!(before.len(), 6);
    session.limit_search_content(1);
    let dropped: Vec<NoteId> = (session.model().notes.iter())
        .filter(|note| session.index().content(note.id).is_none())
        .map(|note| note.id)
        .collect();
    assert_eq!(dropped.len(), 4, "every note's content is gone");
    assert_eq!(listed(&session, None), before);
    assert_eq!(counts(&session, None), (4, 2));
    // Scans send those notes again each time; the same text is not read
    // again, and nothing moves.
    let revision = tasks(&session).revision();
    let read = tasks(&session).lines_read();
    for _ in 0..3 {
        let _ = session.apply_at(Op::Reconcile, Duration::ZERO);
    }
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    let _ = session.apply_at(Op::FinishReconcile(result), clock);
    session.set_background_reconcile(false);
    assert_eq!(listed(&session, None), before);
    assert_eq!(tasks(&session).revision(), revision);
    assert_eq!(tasks(&session).lines_read(), read);
    // A note search cannot hold still has its changes read: on disk,
    let shop = note(&session, "Home/Lists/Shop.md");
    write(
        root.path(),
        "Home/Lists/Shop.md",
        "- [ ] eggs\n- [ ] flour\n",
    );
    let _ = session.apply_at(Op::Reconcile, Duration::ZERO);
    assert!(session.index().content(shop).is_none());
    assert_eq!(
        listed(&session, Some("Home")),
        ["Home/Lists/Shop.md:0 eggs", "Home/Lists/Shop.md:1 flour"]
    );
    // and as the open note's draft.
    let _ = session.apply_at(Op::OpenNote { note: Some(shop) }, Duration::ZERO);
    let _ = session.apply_at(
        Op::EditDraft {
            note: shop,
            contents: "- [ ] eggs\n- [ ] flour\n- [ ] yeast\n".into(),
        },
        Duration::ZERO,
    );
    assert!(session.index().content(shop).is_none());
    assert_eq!(listed(&session, Some("Home")).len(), 3);
}

#[test]
fn an_unchanged_tick_reads_nothing_and_moves_nothing() {
    let root = fixture();
    let mut session = session(root.path());
    let monday = note(&session, "Work/Meetings/Monday.md");
    open(&mut session, monday);
    let revision = tasks(&session).revision();
    let read = tasks(&session).lines_read();
    let today = today();
    let asked = tasks(&session).counts_under(None, today);
    // Ticks that poll and scan on the owner, a scan asked for outright,
    // and ticks that scan in the background.
    let mut clock = Duration::from_secs(10);
    for _ in 0..3 {
        clock += Duration::from_secs(2);
        run_at(&mut session, Op::Tick, clock);
        run_at(&mut session, Op::Reconcile, clock);
    }
    session.set_background_reconcile(true);
    for _ in 0..3 {
        let job = begin(&mut session, &mut clock);
        let result = job.run();
        run_at(&mut session, Op::FinishReconcile(result), clock);
    }
    // The open note's text sent again as it is, and a flush of nothing.
    run_at(
        &mut session,
        Op::EditDraft {
            note: monday,
            contents: MONDAY.into(),
        },
        clock,
    );
    run_at(&mut session, Op::Flush, clock);
    assert_eq!(tasks(&session).revision(), revision);
    assert_eq!(tasks(&session).lines_read(), read);
    assert_eq!(tasks(&session).counts_under(None, today), asked);
    assert!(!root.path().join(TASKS_FILE).exists());
}

#[test]
fn the_settings_file_appears_with_the_first_setting_and_follows_each_op() {
    let root = fixture();
    let mut session = session(root.path());
    let lists = ItemRef::Group(group(&session, "Home/Lists"));
    let work = ItemRef::Section(section(&session, "Work"));
    let monday = ItemRef::Note(note(&session, "Work/Meetings/Monday.md"));
    let saved = || fs::read_to_string(root.path().join(TASKS_FILE)).ok();
    // Nothing set, nothing written: not by opening, and not by an op that
    // sets a default or takes away an exclusion that was not there.
    run(
        &mut session,
        Op::SetTaskFilters {
            scope: Some(work),
            filters: TaskFilters::default(),
        },
    );
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: lists,
            excluded: false,
        },
    );
    // Nor by a rename or a delete while the file names nothing.
    let diary = ItemRef::Note(note(&session, "Home/Lists/Diary.md"));
    run(&mut session, Op::Delete { target: diary });
    let target = ItemRef::Group(group(&session, "Work/Plans"));
    let new_name = "Planning".into();
    run(&mut session, Op::Rename { target, new_name });
    assert_eq!(saved(), None);

    let revision = tasks(&session).revision();
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: lists,
            excluded: true,
        },
    );
    assert_eq!(
        saved().unwrap(),
        "exclude = [\"Home/Lists\"]\nschema_version = 1\n"
    );
    assert!(tasks(&session).revision() > revision);
    assert!(listed(&session, Some("Home")).is_empty());
    assert_eq!(counts(&session, None), (3, 1));
    assert!(session
        .tasks_file()
        .unwrap()
        .is_excluded(&rel("Home/Lists")));
    // A note cannot be excluded this way: its front matter is its switch.
    let error = session
        .apply_at(
            Op::SetTasksExcluded {
                target: monday,
                excluded: true,
            },
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.message.contains("section or a group"), "{error}");

    let narrowed = TaskFilters {
        statuses: vec![StatusType::Open, StatusType::Doing],
        hide_old: false,
        ..TaskFilters::default()
    };
    for scope in [Some(work), Some(monday)] {
        let filters = narrowed.clone();
        run(&mut session, Op::SetTaskFilters { scope, filters });
    }
    assert_eq!(session.task_filters(Some(work)), narrowed);
    assert_eq!(session.task_filters(Some(monday)), narrowed);
    assert_eq!(session.task_filters(None), TaskFilters::default());
    assert_eq!(session.task_filters(Some(lists)), TaskFilters::default());

    // Another Session reads what this one wrote, exclusion and all.
    drop(session);
    let mut session = self::session(root.path());
    let lists = ItemRef::Group(group(&session, "Home/Lists"));
    let work = ItemRef::Section(section(&session, "Work"));
    assert!(session.model().warnings.is_empty());
    assert_eq!(tasks(&session).excluded(), [rel("Home/Lists")]);
    assert!(listed(&session, Some("Home")).is_empty());
    assert_eq!(session.task_filters(Some(work)), narrowed);
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: lists,
            excluded: false,
        },
    );
    assert_eq!(
        listed(&session, Some("Home")),
        ["Home/Lists/Shop.md:0 milk"]
    );
    assert!(!fs::read_to_string(root.path().join(TASKS_FILE))
        .unwrap()
        .contains("exclude"));
}

#[test]
fn a_settings_file_wren_cannot_read_is_left_alone_with_a_warning() {
    let root = fixture();
    let bad = "exclude = [\"Home\"\n";
    write(root.path(), TASKS_FILE, bad);
    let mut session = session(root.path());
    let warnings: Vec<&str> = (session.model().warnings.iter())
        .map(|warning| warning.message.as_str())
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("Task settings are read-only"),
        "{warnings:?}"
    );
    // Defaults apply, the index is whole, and nothing overwrites the file.
    assert_eq!(listed(&session, None).len(), 6);
    let lists = ItemRef::Group(group(&session, "Home/Lists"));
    let error = session
        .apply_at(
            Op::SetTasksExcluded {
                target: lists,
                excluded: true,
            },
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.message.contains("read-only"), "{error}");
    let target = ItemRef::Section(section(&session, "Home"));
    let new_name = "House".into();
    run(&mut session, Op::Rename { target, new_name });
    assert_eq!(
        fs::read_to_string(root.path().join(TASKS_FILE)).unwrap(),
        bad
    );
}

/// Leaving a folder out is all or nothing (review of item 10.2): when
/// `.wren/tasks.toml` cannot be written, the list, the counts and the
/// file stay as they were and the op fails; a file published whose
/// durability is unconfirmed keeps the change, with a warning.
#[test]
fn an_exclusion_that_cannot_be_saved_changes_nothing() {
    use wren_core::fs_store::Fault;
    let root = fixture();
    let mut session = session(root.path());
    let plans = ItemRef::Group(group(&session, "Work/Plans"));
    let before = counts(&session, None);
    let revision = tasks(&session).revision();
    session.fail_next(Fault::Write);
    let error = session
        .apply_at(
            Op::SetTasksExcluded {
                target: plans,
                excluded: true,
            },
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.message.contains("nothing changed"), "{error}");
    assert!(tasks(&session).excluded().is_empty());
    assert_eq!(counts(&session, None), before);
    assert_eq!(tasks(&session).revision(), revision);
    assert!(session.tasks_file().unwrap().excluded().is_empty());
    assert!(!root.path().join(TASKS_FILE).exists());
    // Asked again with the disk working, it is made and saved.
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: plans,
            excluded: true,
        },
    );
    assert_eq!(tasks(&session).excluded(), [rel("Work/Plans")]);
    assert!(fs::read_to_string(root.path().join(TASKS_FILE))
        .unwrap()
        .contains("Work/Plans"));
    // Published but not confirmed durable: kept, with a warning.
    session.fail_next(Fault::DirectorySync);
    let events = session
        .apply_at(
            Op::SetTasksExcluded {
                target: plans,
                excluded: false,
            },
            Duration::ZERO,
        )
        .unwrap();
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Warning(_))));
    assert!(tasks(&session).excluded().is_empty());
    assert!(!fs::read_to_string(root.path().join(TASKS_FILE))
        .unwrap()
        .contains("Work/Plans"));
}

/// A rename publishes even when `.wren/tasks.toml` cannot then be written
/// (CodeRabbit on PR #13): the remapped exclusion is saved by the next
/// flush, over the bytes last read, so leaving the workspace and coming
/// back does not bring the renamed group's tasks back.
#[test]
fn a_rename_whose_settings_failed_saves_them_at_the_next_flush() {
    use wren_core::fs_store::Fault;
    let root = fixture();
    let mut session = session(root.path());
    let plans = ItemRef::Group(group(&session, "Work/Plans"));
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: plans,
            excluded: true,
        },
    );
    session.fail_next(Fault::Write);
    let new_name = "Goals".into();
    let renamed = session.apply_at(
        Op::Rename {
            target: plans,
            new_name,
        },
        Duration::ZERO,
    );
    let events = match renamed {
        Ok(events) => events,
        Err(error) => error.events,
    };
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Warning(_))));
    assert!(root.path().join("Work/Goals/Q1.md").exists());
    assert!(fs::read_to_string(root.path().join(TASKS_FILE))
        .unwrap()
        .contains("Work/Plans"));
    run(&mut session, Op::Flush);
    let saved = fs::read_to_string(root.path().join(TASKS_FILE)).unwrap();
    assert!(saved.contains("Work/Goals"), "{saved}");
    assert!(!saved.contains("Work/Plans"), "{saved}");
    // Read again from disk, the group stays out of task views.
    let other = TempDir::new().unwrap();
    let away = other.path().to_path_buf();
    run(&mut session, Op::OpenWorkspace { path: away });
    let home = root.path().to_path_buf();
    run(&mut session, Op::OpenWorkspace { path: home });
    assert_eq!(tasks(&session).excluded(), [rel("Work/Goals")]);
}

/// The same retry runs when the workspace is left without a flush first.
#[test]
fn leaving_a_workspace_retries_its_unsaved_task_settings() {
    use wren_core::fs_store::Fault;
    let root = fixture();
    let mut session = session(root.path());
    let plans = ItemRef::Group(group(&session, "Work/Plans"));
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: plans,
            excluded: true,
        },
    );
    session.fail_next(Fault::Write);
    let new_name = "Goals".into();
    let _ = session.apply_at(
        Op::Rename {
            target: plans,
            new_name,
        },
        Duration::ZERO,
    );
    let other = TempDir::new().unwrap();
    let away = other.path().to_path_buf();
    run(&mut session, Op::OpenWorkspace { path: away });
    let saved = fs::read_to_string(root.path().join(TASKS_FILE)).unwrap();
    assert!(saved.contains("Work/Goals"), "{saved}");
}

/// A filter change published whose durability is unconfirmed is said to
/// be published, not "not saved" (CodeRabbit on PR #13).
#[test]
fn a_published_filter_change_is_not_said_to_be_unsaved() {
    use wren_core::fs_store::Fault;
    let root = fixture();
    fs::create_dir_all(root.path().join(".wren")).unwrap();
    let mut session = session(root.path());
    session.fail_next(Fault::DirectorySync);
    let filters = TaskFilters {
        group_by: GroupBy::Owner,
        ..TaskFilters::default()
    };
    let result = session.apply_at(
        Op::SetTaskFilters {
            scope: None,
            filters,
        },
        Duration::ZERO,
    );
    let events = match result {
        Ok(events) => events,
        Err(error) => error.events,
    };
    let warnings: Vec<&String> = (events.iter())
        .filter_map(|event| match event {
            Event::Warning(warning) => Some(warning),
            _ => None,
        })
        .collect();
    assert_eq!(warnings.len(), 1, "{events:?}");
    assert!(warnings[0].contains("was published"), "{}", warnings[0]);
    assert!(!warnings[0].contains("not saved"), "{}", warnings[0]);
    assert!(fs::read_to_string(root.path().join(TASKS_FILE))
        .unwrap()
        .contains("owner"));
}

/// The settings retry is best effort (review of the CodeRabbit round): a
/// `.wren` that will not sync neither fails the flush that retried them
/// nor stops a workspace switch, which only warns.
#[test]
fn a_settings_retry_that_cannot_sync_blocks_no_flush_or_switch() {
    use wren_core::fs_store::Fault;
    let root = fixture();
    let mut session = session(root.path());
    let plans = ItemRef::Group(group(&session, "Work/Plans"));
    run(
        &mut session,
        Op::SetTasksExcluded {
            target: plans,
            excluded: true,
        },
    );
    session.fail_next(Fault::Write);
    let new_name = "Goals".into();
    let _ = session.apply_at(
        Op::Rename {
            target: plans,
            new_name,
        },
        Duration::ZERO,
    );
    // The retry publishes; its sync fails, and so does the sync retried.
    session.fail_times(Fault::DirectorySync, 2);
    run(&mut session, Op::Flush);
    let saved = fs::read_to_string(root.path().join(TASKS_FILE)).unwrap();
    assert!(saved.contains("Work/Goals"), "{saved}");
    assert!(!session.pending_durability());
    // Its sync stays pending, best effort, and an idle tick retries it.
    assert!(session.pending_best_effort());
    session
        .apply_at(Op::Tick, Duration::from_secs(5))
        .expect("a tick never fails on a best-effort sync");
    assert!(!session.pending_best_effort());

    // The same on leaving the workspace, after another unsaved change.
    session.fail_next(Fault::Write);
    let target = ItemRef::Group(group(&session, "Work/Goals"));
    let new_name = "Aims".into();
    let _ = session.apply_at(Op::Rename { target, new_name }, Duration::ZERO);
    session.fail_times(Fault::DirectorySync, 2);
    let other = TempDir::new().unwrap();
    let away = other.path().to_path_buf();
    let events = run(&mut session, Op::OpenWorkspace { path: away });
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Warning(warning) if warning.contains("durability"))));
    let saved = fs::read_to_string(root.path().join(TASKS_FILE)).unwrap();
    assert!(saved.contains("Work/Aims"), "{saved}");
}
