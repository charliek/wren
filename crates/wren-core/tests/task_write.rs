//! Writing a task line, and a note's task switch, to a note that is not
//! open (`Op::EditTaskLine`, `Op::EditTaskSwitch`): against a real
//! workspace, with other programs writing to it.

use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;
use wren_core::fs_store::{Fault, RelPath};
use wren_core::index::{QueryKind, Scope};
use wren_core::model::NoteId;
use wren_core::ops::{Event, ExternalChangeKind, Op, OpError, Session, SessionOptions, Trash};
use wren_core::task_write::{LineChange, SwitchChange, CHANGED, SWITCH_CHANGED};
use wren_core::tasks::NoteFlags;
use wren_markdown::date::Date;
use wren_markdown::status::StatusType;
use wren_markdown::task::{Change, Priority};

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
            boards: true,
        },
    )
}

fn write(root: &Path, path: &str, text: impl AsRef<[u8]>) {
    let absolute = root.join(path);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(absolute, text).unwrap();
}

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap()
}

const LIST: &str = "# List\n\n- [ ] Mail the deck 📅 2026-12-01\n- [ ] Book the room\n";
const LIST_PATH: &str = "Work/Tasks/List.md";
const OPEN: &str = "# Open\n\n- [ ] the open note's task\n";
const OPEN_PATH: &str = "Work/Tasks/Open.md";

/// One group with the notes `List` and `Open` and a board.
fn fixture() -> TempDir {
    let root = TempDir::new().unwrap();
    write(root.path(), LIST_PATH, LIST);
    write(root.path(), OPEN_PATH, OPEN);
    write(
        root.path(),
        "Work/Tasks/Flow.wren-board",
        "- [ ] a board's line\n",
    );
    root
}

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

fn note(session: &Session, path: &str) -> NoteId {
    let found = (session.model().notes.iter()).find(|note| note.path == rel(path));
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

fn fail(session: &mut Session, op: Op) -> OpError {
    match session.apply_at(op, Duration::ZERO) {
        Ok(events) => panic!("the op went through: {events:?}"),
        Err(error) => error,
    }
}

/// Thursday 19 November 2026, the design page's day.
fn today() -> Date {
    Date::new(2026, 11, 19).unwrap()
}

fn change(change: Change) -> LineChange {
    LineChange::Change {
        change,
        today: today(),
    }
}

fn edit_line(note: NoteId, line: usize, expect: &str, change: LineChange) -> Op {
    Op::EditTaskLine {
        note,
        line,
        expect: expect.into(),
        change,
    }
}

fn toggle(note: NoteId, line: usize, expect: &str) -> Op {
    edit_line(note, line, expect, change(Change::ToggleDone))
}

/// The task index's lines of `path` as `line source`.
fn listed(session: &Session, path: &str) -> Vec<String> {
    (session.tasks().expect("tasks are on"))
        .tasks_under(Some(&rel(path)))
        .map(|task| format!("{} {}", task.line, task.source))
        .collect()
}

fn flags(session: &Session, note: NoteId) -> NoteFlags {
    session.tasks().unwrap().flags(note).unwrap()
}

/// Whether search finds `words` in the note at `path`.
fn searched(session: &Session, words: &str, path: &str) -> bool {
    (session.index())
        .query(words, Scope::Workspace, QueryKind::Search, 10)
        .iter()
        .any(|hit| hit.path == rel(path))
}

fn written(events: &[Event]) -> Option<(usize, &str, &str)> {
    events.iter().find_map(|event| match event {
        Event::TaskLineWritten {
            line,
            before,
            after,
            ..
        } => Some((*line, before.as_str(), after.as_str())),
        _ => None,
    })
}

fn switched(events: &[Event]) -> Option<(&str, &str)> {
    events.iter().find_map(|event| match event {
        Event::TaskSwitchWritten { before, after, .. } => Some((before.as_str(), after.as_str())),
        _ => None,
    })
}

fn saved(events: &[Event], note: NoteId) -> bool {
    events
        .iter()
        .any(|event| matches!(event, Event::NoteSaved { note: saved, .. } if *saved == note))
}

fn stale(events: &[Event], note: NoteId) -> bool {
    events
        .iter()
        .any(|event| matches!(event, Event::TaskWriteStale { note: of } if *of == note))
}

/// The files of the fixture's group: a conflict copy would be one more.
fn files(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join("Work/Tasks"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const FILES: [&str; 3] = ["Flow.wren-board", "List.md", "Open.md"];
const MAIL: &str = "- [ ] Mail the deck 📅 2026-12-01";
const MAILED: &str = "- [x] Mail the deck 📅 2026-12-01 ✅ 2026-11-19";

#[test]
fn a_closed_notes_task_line_is_written_and_both_indexes_follow() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    let before = session.model().note(list).unwrap().clone();
    assert!(!searched(&session, "2026-11-19", LIST_PATH));

    let events = run(&mut session, toggle(list, 2, MAIL));
    assert_eq!(written(&events), Some((2, MAIL, MAILED)));
    assert!(saved(&events, list));
    assert_eq!(
        read(root.path(), LIST_PATH),
        format!("# List\n\n{MAILED}\n- [ ] Book the room\n")
    );
    // The task index and the search index hold the line as written, and
    // the model the file's new size.
    assert_eq!(
        listed(&session, LIST_PATH),
        [format!("2 {MAILED}"), "3 - [ ] Book the room".to_owned()]
    );
    let task = (session.tasks().unwrap())
        .tasks_under(Some(&rel(LIST_PATH)))
        .next()
        .unwrap()
        .clone();
    assert_eq!(task.status, StatusType::Done);
    assert!(searched(&session, "2026-11-19", LIST_PATH));
    let after = session.model().note(list).unwrap();
    assert_eq!(
        after.bytes,
        before.bytes + " ✅ 2026-11-19".len(),
        "the model follows the file"
    );
    assert!(session.document().is_none(), "nothing was opened");
    assert_eq!(files(root.path()), FILES);

    // The row is found again by what was written: a second change to it,
    // and undo, which expects that line and writes the old one back.
    let events = run(
        &mut session,
        edit_line(
            list,
            2,
            MAILED,
            change(Change::SetPriority(Some(Priority::High))),
        ),
    );
    let (_, _, raised) = written(&events).unwrap();
    assert_eq!(raised, "- [x] Mail the deck ⏫ 📅 2026-12-01 ✅ 2026-11-19");
    let raised = raised.to_owned();
    let undo = edit_line(list, 2, &raised, LineChange::Line(MAILED.into()));
    assert_eq!(
        written(&run(&mut session, undo)),
        Some((2, &*raised, MAILED))
    );
    let undo = edit_line(list, 2, MAILED, LineChange::Line(MAIL.into()));
    run(&mut session, undo);
    assert_eq!(read(root.path(), LIST_PATH), LIST);
    assert_eq!(
        listed(&session, LIST_PATH),
        [format!("2 {MAIL}"), "3 - [ ] Book the room".to_owned()]
    );
}

#[test]
fn a_change_that_changes_nothing_writes_nothing() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    let modified = fs::metadata(root.path().join(LIST_PATH))
        .unwrap()
        .modified()
        .unwrap();
    let due = Change::SetDue(Date::new(2026, 12, 1));
    let events = run(&mut session, edit_line(list, 2, MAIL, change(due)));
    assert_eq!(written(&events), None);
    assert!(!saved(&events, list));
    let now = fs::metadata(root.path().join(LIST_PATH))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(now, modified);
    // A change the task refuses is an error, and writes nothing either.
    let error = fail(
        &mut session,
        edit_line(list, 2, MAIL, change(Change::SetStatus(']'))),
    );
    assert_ne!(error.message, CHANGED);
    assert!(!stale(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), LIST);
}

#[test]
fn the_open_note_a_board_and_a_note_that_is_gone_are_refused() {
    let root = fixture();
    let mut session = session(root.path());
    let (list, open) = (note(&session, LIST_PATH), note(&session, OPEN_PATH));
    let board = note(&session, "Work/Tasks/Flow.wren-board");
    run(&mut session, Op::OpenNote { note: Some(open) });

    // The open note's one writer is its editor.
    let line = "- [ ] the open note's task";
    let error = fail(&mut session, toggle(open, 2, line));
    assert!(error.message.contains("editor"), "{error}");
    assert!(error.events.is_empty());
    let hide = Op::EditTaskSwitch {
        note: open,
        change: SwitchChange::Hide,
    };
    assert!(fail(&mut session, hide).message.contains("editor"));
    assert_eq!(session.document().unwrap().text, OPEN);
    assert!(!session.document().unwrap().dirty());
    assert_eq!(read(root.path(), OPEN_PATH), OPEN);

    let error = fail(&mut session, toggle(board, 0, "- [ ] a board's line"));
    assert!(error.message.contains("Markdown"), "{error}");
    assert_eq!(
        read(root.path(), "Work/Tasks/Flow.wren-board"),
        "- [ ] a board's line\n"
    );

    // A note the model no longer has: the list is out of date.
    let error = fail(&mut session, toggle(NoteId(9_999), 2, MAIL));
    assert_eq!(error.message, CHANGED);
    assert!(stale(&error.events, NoteId(9_999)));

    // One deleted outside Wren since the last scan.
    fs::remove_file(root.path().join(LIST_PATH)).unwrap();
    let error = fail(&mut session, toggle(list, 2, MAIL));
    assert_eq!(error.message, CHANGED);
    assert!(stale(&error.events, list));
    assert!(listed(&session, LIST_PATH).is_empty());
    assert_eq!(files(root.path()), ["Flow.wren-board", "Open.md"]);
}

#[test]
fn bytes_that_are_no_text_refuse_the_write_and_take_the_notes_old_tasks_away() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    let under = Some(rel("Work/Tasks"));
    let counts = |session: &Session| {
        let counts = session
            .tasks()
            .unwrap()
            .counts_under(under.as_ref(), today());
        (counts.open_and_doing, counts.overdue)
    };
    assert_eq!(listed(&session, LIST_PATH).len(), 2);
    assert_eq!(counts(&session), (3, 0));
    assert!(searched(&session, "deck", LIST_PATH));
    // Another program leaves bytes that are no text; no scan has seen
    // them. The toggle and the switch each read them and refuse.
    let bytes = [b'-', b' ', 0xff, 0xfe, b'\n'];
    write(root.path(), LIST_PATH, bytes);
    let hide = Op::EditTaskSwitch {
        note: list,
        change: SwitchChange::Hide,
    };
    for op in [toggle(list, 2, MAIL), hide] {
        let error = fail(&mut session, op);
        assert!(error.message.contains("not valid UTF-8"), "{error}");
        // What the refusal found is known at once: the old tasks, their
        // count and their text in search are gone.
        assert!(listed(&session, LIST_PATH).is_empty());
        assert!(flags(&session, list).unreadable);
        assert_eq!(counts(&session), (1, 0));
        assert!(!searched(&session, "deck", LIST_PATH));
        assert_eq!(session.index().content(list), None);
        assert_eq!(session.model().note(list).unwrap().bytes, bytes.len());
    }
    assert_eq!(fs::read(root.path().join(LIST_PATH)).unwrap(), bytes);
}

#[test]
fn with_tasks_off_neither_op_runs() {
    let root = fixture();
    let mut session = session_with(root.path(), SessionOptions::default());
    let list = note(&session, LIST_PATH);
    for op in [
        toggle(list, 2, MAIL),
        Op::EditTaskSwitch {
            note: list,
            change: SwitchChange::Hide,
        },
    ] {
        let error = fail(&mut session, op);
        assert!(error.message.contains("turned off"), "{error}");
        assert!(error.events.is_empty());
    }
    assert_eq!(read(root.path(), LIST_PATH), LIST);
    assert!(!root.path().join(".wren/tasks.toml").exists());
}

#[test]
fn a_line_that_moved_is_found_and_one_that_cannot_be_told_apart_is_refused() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);

    // Another program put lines above the task: the index still has it
    // on line 2.
    let moved = format!("# List\n\nnew\nlines\n{MAIL}\n- [ ] Book the room\n");
    write(root.path(), LIST_PATH, &moved);
    let events = run(&mut session, toggle(list, 2, MAIL));
    assert_eq!(written(&events), Some((4, MAIL, MAILED)));
    assert_eq!(read(root.path(), LIST_PATH), moved.replace(MAIL, MAILED));
    assert_eq!(listed(&session, LIST_PATH)[0], format!("4 {MAILED}"));

    // Two task lines of one text. Where the row said, the line is written.
    let twins = "- [ ] same\n- [ ] other\n- [ ] same\n";
    write(root.path(), LIST_PATH, twins);
    run(&mut session, Op::Reconcile);
    run(&mut session, toggle(list, 2, "- [ ] same"));
    let toggled = "- [ ] same\n- [ ] other\n- [x] same ✅ 2026-11-19\n";
    assert_eq!(read(root.path(), LIST_PATH), toggled);

    // Moved, they cannot be told apart: nothing is written, the list is
    // refreshed from the file, and no conflict copy is made.
    let shifted = "intro\n- [ ] same\n- [ ] other\n\n- [ ] same\n";
    write(root.path(), LIST_PATH, shifted);
    let error = fail(&mut session, toggle(list, 2, "- [ ] same"));
    assert_eq!(error.message, CHANGED);
    assert!(stale(&error.events, list));
    assert!(written(&error.events).is_none() && !saved(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), shifted);
    assert_eq!(
        listed(&session, LIST_PATH),
        ["1 - [ ] same", "2 - [ ] other", "4 - [ ] same"]
    );
    assert_eq!(files(root.path()), FILES);

    // A line whose text changed is refused the same way.
    let error = fail(&mut session, toggle(list, 2, "- [ ] another"));
    assert_eq!(error.message, CHANGED);
    assert_eq!(read(root.path(), LIST_PATH), shifted);
}

#[test]
fn a_line_of_the_same_text_in_a_fence_or_in_front_matter_is_never_written() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    // The task the row named is quoted in a fence now, and its twin
    // below is the one task of that text.
    let fenced = "```\n- [ ] same\n```\n- [ ] same\n";
    write(root.path(), LIST_PATH, fenced);
    let events = run(&mut session, toggle(list, 1, "- [ ] same"));
    assert_eq!(written(&events).map(|(line, ..)| line), Some(3));
    assert_eq!(
        read(root.path(), LIST_PATH),
        "```\n- [ ] same\n```\n- [x] same ✅ 2026-11-19\n"
    );
    // In front matter and nowhere else: no task at all.
    let front = "---\nkey: v\n- [ ] same\n---\nbody\n";
    write(root.path(), LIST_PATH, front);
    let error = fail(&mut session, toggle(list, 2, "- [ ] same"));
    assert_eq!(error.message, CHANGED);
    assert_eq!(read(root.path(), LIST_PATH), front);
}

#[test]
fn a_file_that_changes_under_the_write_is_left_as_the_other_writer_made_it() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    // Another program saves between this write's read and its
    // publication. Its line still reads as expected, so only the
    // compare-and-swap can tell.
    let theirs = format!("{LIST}- [ ] theirs\n");
    let (path, text) = (root.path().join(LIST_PATH), theirs.clone());
    session.race_next_write(move || fs::write(path, text).unwrap());
    let error = fail(&mut session, toggle(list, 2, MAIL));
    assert_eq!(error.message, CHANGED);
    assert!(stale(&error.events, list));
    assert!(written(&error.events).is_none() && !saved(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), theirs, "their bytes stand");
    assert_eq!(files(root.path()), FILES, "no conflict copy");
    // The list is refreshed from what is there.
    assert_eq!(listed(&session, LIST_PATH).len(), 3);
    assert!(searched(&session, "theirs", LIST_PATH));
    // The same action again, on the refreshed row, goes through.
    let events = run(&mut session, toggle(list, 2, MAIL));
    assert_eq!(written(&events), Some((2, MAIL, MAILED)));
    assert_eq!(read(root.path(), LIST_PATH), theirs.replace(MAIL, MAILED));
}

#[test]
fn line_endings_and_a_byte_order_mark_are_kept() {
    let root = fixture();
    let crlf = "\u{feff}- [ ] top\r\n  - [ ] nested\r\n\r\nlast";
    write(root.path(), LIST_PATH, crlf);
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    // The index gives the first line with its mark, which the write
    // expects and keeps.
    assert_eq!(
        listed(&session, LIST_PATH),
        ["0 \u{feff}- [ ] top", "1   - [ ] nested"]
    );
    let events = run(&mut session, toggle(list, 0, "\u{feff}- [ ] top"));
    let done = "\u{feff}- [x] top ✅ 2026-11-19";
    assert_eq!(written(&events), Some((0, "\u{feff}- [ ] top", done)));
    run(
        &mut session,
        edit_line(list, 1, "  - [ ] nested", change(Change::Cycle)),
    );
    assert_eq!(
        fs::read(root.path().join(LIST_PATH)).unwrap(),
        "\u{feff}- [x] top ✅ 2026-11-19\r\n  - [/] nested\r\n\r\nlast".as_bytes()
    );
    assert_eq!(
        listed(&session, LIST_PATH),
        [format!("0 {done}"), "1   - [/] nested".to_owned()]
    );
    // Undo, given the line with or without its mark.
    let undo = edit_line(list, 0, done, LineChange::Line("- [ ] top".into()));
    run(&mut session, undo);
    let undo = LineChange::Line("  - [ ] nested".into());
    run(&mut session, edit_line(list, 1, "  - [/] nested", undo));
    assert_eq!(
        fs::read(root.path().join(LIST_PATH)).unwrap(),
        crlf.as_bytes()
    );
}

#[test]
fn the_open_notes_draft_is_settled_before_another_note_is_written() {
    let root = fixture();
    let mut session = session(root.path());
    let (list, open) = (note(&session, LIST_PATH), note(&session, OPEN_PATH));
    run(&mut session, Op::OpenNote { note: Some(open) });
    let draft = format!("{OPEN}typed\n");
    let contents = draft.clone();
    run(
        &mut session,
        Op::EditDraft {
            note: open,
            contents,
        },
    );
    assert!(session.document().unwrap().dirty());

    // A flush that fails blocks the write, and keeps the draft.
    session.fail_next(Fault::Write);
    let error = fail(&mut session, toggle(list, 2, MAIL));
    assert_ne!(error.message, CHANGED);
    assert!(written(&error.events).is_none());
    assert_eq!(read(root.path(), LIST_PATH), LIST);
    assert_eq!(read(root.path(), OPEN_PATH), OPEN);
    assert_eq!(session.document().unwrap().text, draft);
    assert!(session.document().unwrap().dirty());

    // Once it can be saved, it is, and then the other note is written.
    let events = run(&mut session, toggle(list, 2, MAIL));
    assert!(saved(&events, open) && saved(&events, list));
    let order: Vec<NoteId> = events
        .iter()
        .filter_map(|event| match event {
            Event::NoteSaved { note, .. } => Some(*note),
            _ => None,
        })
        .collect();
    assert_eq!(order, [open, list], "the draft first");
    assert_eq!(read(root.path(), OPEN_PATH), draft);
    assert!(!session.document().unwrap().dirty());
    assert_eq!(session.document().unwrap().note, open);
    assert_eq!(read(root.path(), LIST_PATH), LIST.replace(MAIL, MAILED));
}

#[test]
fn a_draft_in_conflict_is_copied_and_blocks_the_write_to_the_other_note() {
    let root = fixture();
    let mut session = session(root.path());
    let (list, open) = (note(&session, LIST_PATH), note(&session, OPEN_PATH));
    run(&mut session, Op::OpenNote { note: Some(open) });
    let contents = format!("{OPEN}typed\n");
    run(
        &mut session,
        Op::EditDraft {
            note: open,
            contents,
        },
    );
    write(root.path(), OPEN_PATH, "theirs\n");
    let error = fail(&mut session, toggle(list, 2, MAIL));
    // The open document's own rule: its draft goes to a conflict copy.
    assert!(error.events.iter().any(|event| matches!(
        event,
        Event::ExternalChange {
            kind: ExternalChangeKind::Conflict { .. },
            ..
        }
    )));
    assert_eq!(files(root.path()).len(), 4);
    assert_eq!(
        read(root.path(), LIST_PATH),
        LIST,
        "the task is not written"
    );
    assert!(written(&error.events).is_none());
}

/// Tick until the Session hands out a background job.
fn begin(session: &mut Session, clock: &mut Duration) -> wren_core::reconcile::ReconcileJob {
    *clock += Duration::from_secs(2);
    run_at(session, Op::Tick, *clock);
    session.take_reconcile_job().expect("a job is due")
}

#[test]
fn a_scan_begun_before_the_write_cannot_put_the_old_line_back() {
    let root = fixture();
    let mut session = session(root.path());
    session.set_background_reconcile(true);
    let mut clock = Duration::from_secs(10);
    let list = note(&session, LIST_PATH);
    // The job reads the note as another program left it, before the task
    // is written.
    let theirs = LIST.replace("Book the room", "Book the big room");
    write(root.path(), LIST_PATH, &theirs);
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    let events = run_at(&mut session, toggle(list, 2, MAIL), clock);
    assert_eq!(written(&events), Some((2, MAIL, MAILED)));
    let wrote = theirs.replace(MAIL, MAILED);
    assert_eq!(read(root.path(), LIST_PATH), wrote);
    let size = session.model().note(list).unwrap().bytes;
    assert_eq!(size, wrote.len());

    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert_eq!(
        listed(&session, LIST_PATH),
        [
            format!("2 {MAILED}"),
            "3 - [ ] Book the big room".to_owned()
        ]
    );
    assert!(searched(&session, "2026-11-19", LIST_PATH));
    assert_eq!(session.model().note(list).unwrap().bytes, size);
    // The next scan agrees with the index.
    let revision = session.tasks().unwrap().revision();
    let job = begin(&mut session, &mut clock);
    let result = job.run();
    run_at(&mut session, Op::FinishReconcile(result), clock);
    assert_eq!(session.tasks().unwrap().revision(), revision);
    assert_eq!(listed(&session, LIST_PATH)[0], format!("2 {MAILED}"));
}

#[test]
fn a_write_whose_durability_is_unconfirmed_says_so_and_is_still_known() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    session.fail_next(Fault::DirectorySync);
    let error = fail(&mut session, toggle(list, 2, MAIL));
    assert!(error.message.contains("durability"), "{error}");
    assert!(error.message.contains("DirectorySync"), "{error}");
    // The bytes are published: the event says what was written, so it
    // can be undone, and the indexes hold it. It is not reported saved.
    assert_eq!(written(&error.events), Some((2, MAIL, MAILED)));
    assert!(!saved(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), LIST.replace(MAIL, MAILED));
    assert_eq!(listed(&session, LIST_PATH)[0], format!("2 {MAILED}"));
    assert!(session.pending_durability());
    run(&mut session, Op::Flush);
    assert!(!session.pending_durability());
}

#[test]
fn an_edit_made_outside_before_undo_refuses_the_undo() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    run(&mut session, toggle(list, 2, MAIL));
    // Another program changes the written line before the undo.
    let theirs = read(root.path(), LIST_PATH).replace("Mail the deck", "Mail the decks");
    write(root.path(), LIST_PATH, &theirs);
    let undo = edit_line(list, 2, MAILED, LineChange::Line(MAIL.into()));
    let error = fail(&mut session, undo);
    assert_eq!(error.message, CHANGED);
    assert!(stale(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), theirs);
    assert!(listed(&session, LIST_PATH)[0].contains("decks"));
}

fn switch(note: NoteId, change: SwitchChange) -> Op {
    Op::EditTaskSwitch { note, change }
}

#[test]
fn hiding_a_closed_notes_tasks_writes_its_front_matter_and_empties_its_list() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    assert_eq!(listed(&session, LIST_PATH).len(), 2);

    let events = run(&mut session, switch(list, SwitchChange::Hide));
    assert_eq!(switched(&events), Some(("", "---\ntasks: off\n---\n")));
    assert!(saved(&events, list));
    let hidden = format!("---\ntasks: off\n---\n{LIST}");
    assert_eq!(read(root.path(), LIST_PATH), hidden);
    // Its tasks leave every list at once.
    assert!(listed(&session, LIST_PATH).is_empty());
    assert!(flags(&session, list).hidden);
    // Hidden already: nothing to write.
    let events = run(&mut session, switch(list, SwitchChange::Hide));
    assert!(switched(&events).is_none() && !saved(&events, list));

    // Undo puts the front matter back as it was: none.
    let undo = SwitchChange::Restore {
        expect: "---\ntasks: off\n---\n".into(),
        replace: String::new(),
    };
    let events = run(&mut session, switch(list, undo.clone()));
    assert_eq!(switched(&events), Some(("---\ntasks: off\n---\n", "")));
    assert_eq!(read(root.path(), LIST_PATH), LIST);
    assert_eq!(listed(&session, LIST_PATH).len(), 2);
    assert!(!flags(&session, list).hidden);
    // The same undo again finds front matter that is not what it expects.
    let error = fail(&mut session, switch(list, undo));
    assert_eq!(error.message, SWITCH_CHANGED);
    assert!(stale(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), LIST);
}

#[test]
fn showing_takes_the_line_out_and_undo_puts_it_back_where_it_was() {
    let root = fixture();
    let front = "\u{feff}---\r\ntasks: off\r\ntitle: List\r\n---\r\n- [ ] a\r\n";
    write(root.path(), LIST_PATH, front);
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    assert!(listed(&session, LIST_PATH).is_empty());

    let events = run(&mut session, switch(list, SwitchChange::Show));
    let (before, after) = switched(&events).unwrap();
    assert_eq!(before, "---\r\ntasks: off\r\ntitle: List\r\n---\r\n");
    assert_eq!(after, "---\r\ntitle: List\r\n---\r\n");
    let (before, after) = (before.to_owned(), after.to_owned());
    assert_eq!(
        read(root.path(), LIST_PATH),
        "\u{feff}---\r\ntitle: List\r\n---\r\n- [ ] a\r\n"
    );
    assert_eq!(listed(&session, LIST_PATH), ["3 - [ ] a"]);

    // Another program edits the front matter before the undo: refused,
    // with nothing written and no conflict copy.
    let theirs = "\u{feff}---\r\ntitle: Theirs\r\n---\r\n- [ ] a\r\n";
    write(root.path(), LIST_PATH, theirs);
    let undo = SwitchChange::Restore {
        expect: after,
        replace: before,
    };
    let error = fail(&mut session, switch(list, undo.clone()));
    assert_eq!(error.message, SWITCH_CHANGED);
    assert_eq!(read(root.path(), LIST_PATH), theirs);
    assert_eq!(files(root.path()), FILES);

    // With the front matter as the change left it, undo is exact.
    write(
        root.path(),
        LIST_PATH,
        "\u{feff}---\r\ntitle: List\r\n---\r\n- [ ] a\r\n- [ ] b\r\n",
    );
    run(&mut session, switch(list, undo));
    assert_eq!(read(root.path(), LIST_PATH), format!("{front}- [ ] b\r\n"));
    assert!(listed(&session, LIST_PATH).is_empty());
}

#[test]
fn obsidians_key_is_not_wrens_to_remove() {
    let root = fixture();
    let front = "---\nTP_ignore_this_file: true\n---\n- [ ] a\n";
    write(root.path(), LIST_PATH, front);
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    let error = fail(&mut session, switch(list, SwitchChange::Show));
    assert!(error.message.contains("TP_ignore_this_file"), "{error}");
    assert!(!stale(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), front);
}

#[test]
fn a_front_matter_change_races_another_writer_like_a_line_does() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    let theirs = format!("{LIST}theirs\n");
    let (path, text) = (root.path().join(LIST_PATH), theirs.clone());
    session.race_next_write(move || fs::write(path, text).unwrap());
    let error = fail(&mut session, switch(list, SwitchChange::Hide));
    assert_eq!(error.message, SWITCH_CHANGED);
    assert!(stale(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), theirs);
    assert_eq!(files(root.path()), FILES);
    assert_eq!(listed(&session, LIST_PATH).len(), 2);
}

fn kept(events: &[Event]) -> Option<(usize, &str)> {
    events.iter().find_map(|event| match event {
        Event::TaskLineKept { line, source, .. } => Some((*line, source.as_str())),
        _ => None,
    })
}

#[test]
fn a_change_that_changes_nothing_says_where_the_task_is_and_refreshes_the_indexes() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    // Another program moves the task down and adds one; the indexes
    // still hold the note as it was.
    let moved = LIST.replace(MAIL, &format!("new\n- [ ] theirs\n{MAIL}"));
    write(root.path(), LIST_PATH, &moved);
    let modified = fs::metadata(root.path().join(LIST_PATH))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(listed(&session, LIST_PATH)[0], format!("2 {MAIL}"));

    let due = Change::SetDue(Date::new(2026, 12, 1));
    let events = run(&mut session, edit_line(list, 2, MAIL, change(due)));
    // Nothing is written, and the row is resolved to where the task is.
    assert_eq!(kept(&events), Some((4, MAIL)));
    assert!(written(&events).is_none() && !saved(&events, list));
    assert_eq!(read(root.path(), LIST_PATH), moved);
    let now = fs::metadata(root.path().join(LIST_PATH))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(now, modified);
    // The bytes read reach both indexes and the model.
    assert_eq!(
        listed(&session, LIST_PATH),
        [
            "3 - [ ] theirs".to_owned(),
            format!("4 {MAIL}"),
            "5 - [ ] Book the room".to_owned()
        ]
    );
    assert!(searched(&session, "theirs", LIST_PATH));
    assert_eq!(session.model().note(list).unwrap().bytes, moved.len());

    // The switch the same: a note hidden outside Wren, asked to hide.
    let hidden = format!("---\ntasks: off\n---\n{moved}");
    write(root.path(), LIST_PATH, &hidden);
    let events = run(&mut session, switch(list, SwitchChange::Hide));
    assert!(switched(&events).is_none() && !saved(&events, list));
    assert_eq!(read(root.path(), LIST_PATH), hidden);
    assert!(flags(&session, list).hidden);
    assert!(listed(&session, LIST_PATH).is_empty());
}

#[test]
fn a_line_written_whole_that_is_no_task_is_refused() {
    let root = fixture();
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    for text in ["plain text", "- no box", "```"] {
        let error = fail(
            &mut session,
            edit_line(list, 2, MAIL, LineChange::Line(text.into())),
        );
        assert_ne!(error.message, CHANGED, "{text}");
        assert!(written(&error.events).is_none() && !stale(&error.events, list));
    }
    assert_eq!(read(root.path(), LIST_PATH), LIST);
    assert_eq!(listed(&session, LIST_PATH)[0], format!("2 {MAIL}"));
}

#[test]
fn showing_never_makes_front_matter_of_the_notes_own_lines() {
    let root = fixture();
    let front = "---\ntasks: off\n---\n---\ntitle: keep\n---\nbody\n";
    write(root.path(), LIST_PATH, front);
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    let error = fail(&mut session, switch(list, SwitchChange::Show));
    assert!(error.message.contains("front matter"), "{error}");
    assert_ne!(error.message, SWITCH_CHANGED);
    assert!(switched(&error.events).is_none() && !stale(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), front);
    assert!(flags(&session, list).hidden);
}

#[test]
fn front_matter_is_not_put_back_where_it_would_run_into_the_body() {
    let root = fixture();
    // The block is the whole note, with no ending after its closing line.
    let front = "---\ntasks: off\n---";
    write(root.path(), LIST_PATH, front);
    let mut session = session(root.path());
    let list = note(&session, LIST_PATH);
    let events = run(&mut session, switch(list, SwitchChange::Show));
    let (before, after) = switched(&events).unwrap();
    assert_eq!((before, after), (front, ""));
    assert_eq!(read(root.path(), LIST_PATH), "");
    // Text written since would follow the closing line on its own line.
    write(root.path(), LIST_PATH, "- [ ] body\n");
    let undo = SwitchChange::Restore {
        expect: String::new(),
        replace: front.into(),
    };
    let error = fail(&mut session, switch(list, undo.clone()));
    assert_eq!(error.message, SWITCH_CHANGED);
    assert!(stale(&error.events, list));
    assert_eq!(read(root.path(), LIST_PATH), "- [ ] body\n");
    assert_eq!(listed(&session, LIST_PATH), ["0 - [ ] body"]);
    // With the note empty again, it is put back as it was.
    write(root.path(), LIST_PATH, "");
    run(&mut session, switch(list, undo));
    assert_eq!(read(root.path(), LIST_PATH), front);
}

#[test]
fn the_open_documents_file_is_compared_with_what_the_document_was_read_from() {
    let root = fixture();
    let mut session = session(root.path());
    assert_eq!(session.document_changed_on_disk(), Ok(false), "none open");
    let open = note(&session, OPEN_PATH);
    run(&mut session, Op::OpenNote { note: Some(open) });
    assert_eq!(session.document_changed_on_disk(), Ok(false));
    // A draft is no change on disk; a save moves the baseline with it.
    let contents = format!("{OPEN}typed\n");
    run(
        &mut session,
        Op::EditDraft {
            note: open,
            contents,
        },
    );
    assert_eq!(session.document_changed_on_disk(), Ok(false));
    run(&mut session, Op::Flush);
    assert_eq!(session.document_changed_on_disk(), Ok(false));
    // Another program writes it, or takes it away. Asking changes nothing.
    write(root.path(), OPEN_PATH, "theirs\n");
    assert_eq!(session.document_changed_on_disk(), Ok(true));
    assert_eq!(session.document().unwrap().text, format!("{OPEN}typed\n"));
    fs::remove_file(root.path().join(OPEN_PATH)).unwrap();
    assert_eq!(session.document_changed_on_disk(), Ok(true));
    write(root.path(), OPEN_PATH, format!("{OPEN}typed\n"));
    assert_eq!(session.document_changed_on_disk(), Ok(false));
}
