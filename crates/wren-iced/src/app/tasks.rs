//! Tasks' app side (discovery/tasks.md): the one clock every task feature
//! reads, and the day change that refreshes them. All of it waits on the
//! `tasks` setting the app launched with.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use iced::Subscription;
use wren_markdown::block::{self, Boxes, LineInfo, Scanner};
use wren_markdown::checkpoints::{self, Checkpoints};
use wren_markdown::date::Date;
use wren_markdown::span::SpanKind;
use wren_markdown::task::{self, Summary};
use wren_markdown::TaskOptions;

use super::board::Surface;
use super::{App, Message};

/// The day a clock that never read the local date holds (tasks off).
fn unread() -> Date {
    Date::new(1970, 1, 1).expect("a date")
}

/// How often a running Wren looks for a new local date while tasks are on.
const DAY_CHECK: Duration = Duration::from_secs(60);

/// Today as the app last saw it, and the date a test pinned.
#[derive(Debug, Clone)]
pub(super) struct Clock {
    /// The date `App::today` gives in place of the local one, set by
    /// `WREN_TEST_TODAY` at launch or the `tasks.set_today` request.
    #[cfg(any(test, feature = "smoke-test"))]
    pinned: Option<Date>,
    /// The day the last check saw.
    seen: Date,
    /// How many day changes the app has handled.
    changes: u64,
}

impl Clock {
    #[cfg_attr(not(any(test, feature = "smoke-test")), allow(unused_variables))]
    pub(super) fn new(pinned: Option<Date>) -> Self {
        #[cfg(any(test, feature = "smoke-test"))]
        let seen = pinned.unwrap_or_else(wren_core::ops::local_today);
        #[cfg(not(any(test, feature = "smoke-test")))]
        let seen = wren_core::ops::local_today();
        Self {
            #[cfg(any(test, feature = "smoke-test"))]
            pinned,
            seen,
            changes: 0,
        }
    }

    /// The clock the app launches with: with tasks off, which reads no
    /// date and handles no day change, one that never read the local date.
    pub(super) fn launch(pinned: Option<Date>, tasks: bool) -> Self {
        if tasks {
            return Self::new(pinned);
        }
        Self {
            #[cfg(any(test, feature = "smoke-test"))]
            pinned,
            seen: pinned.unwrap_or_else(unread),
            changes: 0,
        }
    }

    /// The day the last check saw.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn seen(&self) -> Date {
        self.seen
    }

    /// How many day changes the app has handled.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn changes(&self) -> u64 {
        self.changes
    }

    /// Move the pinned day to `date` without the check the timer makes,
    /// as midnight passes between two of its ticks.
    #[cfg(test)]
    pub(super) fn pass_midnight(&mut self, date: Date) {
        self.pinned = Some(date);
    }
}

/// The caret line's task box as [`App::caret_task`] last found it, or that
/// the line has none, with what it was found for, and the task spelled out
/// for the day it was spelled out on.
#[derive(Debug, Default)]
pub(super) struct TaskMemo {
    kept: RefCell<Option<Memo>>,
    /// Whether the caret's line belongs to a task, as the line commands
    /// last found it, with the editor's text revision, the caret's line
    /// and the engine it was found for.
    pub(super) on_task: std::cell::Cell<Option<(MemoKey, bool)>>,
    /// Whether the task form can open at the caret's line, as `task.edit`
    /// last found it, with what it was found for.
    pub(super) form_at: std::cell::Cell<Option<(MemoKey, bool)>>,
    /// How many times a line was read for its box.
    #[cfg(test)]
    reads: std::cell::Cell<usize>,
    /// How many times a task was spelled out.
    #[cfg(test)]
    formats: std::cell::Cell<usize>,
    /// The editor's lines as the whole note reads them, kept across edits
    /// with the revision of the text they were last brought up to:
    /// see [`App::read_task_line`].
    lines: RefCell<Option<(Option<u64>, Checkpoints)>>,
    /// The note and surface the caches were last kept for: see
    /// [`App::release_task_caches`].
    shown: std::cell::Cell<Option<(Option<wren_core::model::NoteId>, bool)>>,
}

#[derive(Debug)]
struct Memo {
    key: MemoKey,
    found: Option<TaskBox>,
    /// The day `summary` was spelled out for; `None` before it was.
    today: Option<Date>,
    summary: Option<Rc<Summary>>,
}

/// A line's text and its task's box.
type TaskBox = (String, block::Task);

/// What a [`TaskMemo`] holds for: the editor's text revision (which also
/// moves on a note switch and when tasks turn on or off), the caret's line
/// and the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MemoKey {
    revision: u64,
    pub(super) line: usize,
    engine: wren_editor::BackendKind,
}

impl App {
    /// Today's date in local time: the one source every task feature reads.
    pub(super) fn today(&self) -> Date {
        #[cfg(any(test, feature = "smoke-test"))]
        if let Some(pinned) = self.clock.pinned {
            return pinned;
        }
        wren_core::ops::local_today()
    }

    /// Pin today to `date`, then look for the day change as the timer does.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn pin_today(&mut self, date: Date) {
        self.clock.pinned = Some(date);
        self.check_day();
    }

    /// Compare today with the day last seen, handling a change once. With
    /// tasks off there is nothing to refresh, so nothing is handled.
    pub(super) fn check_day(&mut self) {
        if !self.preferences.tasks {
            return;
        }
        let today = self.today();
        if today != self.clock.seen {
            self.clock.seen = today;
            self.day_changed();
        }
    }

    /// The local date moved on while Wren was open. What shows a date
    /// relative to today refreshes from here.
    fn day_changed(&mut self) {
        self.clock.changes += 1;
        self.sync_task_options();
    }

    /// The one value everything that reads a task line is given: `None`
    /// with tasks off, when Wren reads and edits a task line exactly as it
    /// did before tasks, and today's date with them on. It reaches the
    /// editor (`sync_task_options`), which hands it to the Markdown keys,
    /// the highlighter and so spell check's prose; the Session takes the
    /// setting itself (`session_options`), for the index's previews; and
    /// whatever classifies lines here takes `task_boxes`.
    pub(super) fn task_options(&self) -> Option<TaskOptions> {
        self.preferences.tasks.then(|| TaskOptions {
            today: self.today(),
        })
    }

    /// The task boxes the classifier reads in this run: the part of
    /// [`App::task_options`] that does not change with the day.
    pub(super) fn task_boxes(&self) -> Boxes {
        Boxes::with_tasks(self.preferences.tasks)
    }

    /// Give the editor the task options as they are now: at launch, when
    /// the settings are applied, and when the day changes.
    pub(super) fn sync_task_options(&mut self) {
        self.editor.set_tasks(self.task_options());
        self.sync_task_menus();
    }

    /// Whether the editor holds Markdown a task line can be read from: a
    /// note's text, or the in-memory evaluation text outside a workspace.
    pub(super) fn markdown_in_editor(&self) -> bool {
        let open = !self.workspace_mode
            || self
                .session
                .as_ref()
                .is_some_and(|session| session.document().is_some());
        open && self.surface() == Surface::Text
    }

    /// The caret's task spelled out, as the status bar shows it
    /// (discovery/tasks.md §2 item 23): `None` with tasks off, and unless
    /// the caret's line holds a task's box. The line of the box is the
    /// task's own: its tokens are read there, so a later line of the same
    /// item is not. The relative dates count from [`App::today`], read on
    /// each view, so the line follows the caret, every edit and a new day.
    ///
    /// Finding the box scans the note down to the caret's line, so what it
    /// finds is kept ([`TaskMemo`]) until the text, the caret's line or the
    /// engine changes, and the task spelled out until then or a new day: a
    /// view with none of them changed reads and formats nothing.
    pub(super) fn caret_task(&self) -> Option<Rc<Summary>> {
        if !self.preferences.tasks || !self.markdown_in_editor() {
            return None;
        }
        let key = self.memo_key();
        let today = self.today();
        let mut kept = self.task_memo.kept.borrow_mut();
        let memo = match &mut *kept {
            Some(memo) if memo.key == key => memo,
            _ => kept.insert(Memo {
                key,
                found: self.task_box(key.line),
                today: None,
                summary: None,
            }),
        };
        if memo.today != Some(today) {
            memo.today = Some(today);
            memo.summary = memo.found.as_ref().and_then(|(text, box_)| {
                #[cfg(test)]
                self.task_memo.formats.set(self.task_memo.formats.get() + 1);
                let line = task::parse(text, *box_)?;
                Some(Rc::new(task::summary(&line, today)))
            });
        }
        memo.summary.clone()
    }

    /// What is read from the caret's line is kept for: the text's
    /// revision, the caret's line and the engine.
    pub(super) fn memo_key(&self) -> MemoKey {
        MemoKey {
            revision: self.editor.revision(),
            line: self.editor.cursor().position.line,
            engine: self.editor.backend(),
        }
    }

    /// Line `number`'s text and its task's box, when it holds one.
    fn task_box(&self, number: usize) -> Option<TaskBox> {
        #[cfg(test)]
        self.task_memo.reads.set(self.task_memo.reads.get() + 1);
        let text = self.editor.line_text(number)?;
        // Every box opens with one; the lines above are scanned only for a
        // line that might hold one.
        if !text.contains('[') {
            return None;
        }
        let boxes = Boxes::with_tasks(self.editor.tasks().is_some());
        let box_ = self
            .read_task_line(number, |text, info, _| {
                wren_markdown::span::spans(text, info, boxes)
                    .spans
                    .iter()
                    .find_map(|span| match span.kind {
                        SpanKind::Box(task) => Some(task),
                        _ => None,
                    })
            })
            .flatten()?;
        Some((text, box_))
    }

    /// Line `line` of the editor's text as the whole note reads it, handed
    /// to `read` with its text, its classification and the scanner just
    /// after it; `None` past the last line. The scanner's states are kept
    /// across edits (`wren_markdown::checkpoints`), and forgotten from the
    /// first line an edit changed, which the editor names
    /// ([`wren_editor::EditorView::changed_since`]), so a read after typing
    /// compares no lines and scans a few dozen, not the note down to
    /// `line`: what finds the caret line's task after every edit, near the
    /// end of a long note too.
    pub(super) fn read_task_line<R>(
        &self,
        line: usize,
        read: impl FnOnce(&str, &LineInfo, &Scanner) -> R,
    ) -> Option<R> {
        let revision = self.editor.revision();
        // The boxes the editor reads, as its highlighter and plans do.
        let boxes = Boxes::with_tasks(self.editor.tasks().is_some());
        let mut kept = self.task_memo.lines.borrow_mut();
        if kept
            .as_ref()
            .is_none_or(|(_, checkpoints)| checkpoints.boxes() != boxes)
        {
            *kept = Some((None, Checkpoints::new(boxes)));
        }
        let (seen, checkpoints) = kept.as_mut().expect("just made");
        // The editor says which line its changes since the last read began
        // on, when it can; else the lines kept are compared.
        let changed = seen.and_then(|seen| self.editor.changed_since(seen));
        self.editor.read_markdown(|lines, _| {
            if *seen != Some(revision) {
                match changed {
                    Some(first) => checkpoints.update_from(lines, first),
                    None => checkpoints.update(lines),
                };
                *seen = Some(revision);
            }
            checkpoints.line(lines, line, read)
        })
    }

    /// Give back what is kept of a note's lines, here and in the owners'
    /// copy of the note typed in, once the editor shows another note, or a
    /// board, or no note. After every message, with tasks on.
    pub(super) fn release_task_caches(&mut self) {
        let shown = (self.selected_note, self.surface() == Surface::Text);
        if self.task_memo.shown.get() == Some(shown) {
            return;
        }
        self.task_memo.shown.set(Some(shown));
        *self.task_memo.lines.borrow_mut() = None;
        self.owners.release_typing();
    }

    /// Whether lines of the editor's text are kept.
    #[cfg(test)]
    pub(super) fn task_lines_kept(&self) -> bool {
        self.task_memo.lines.borrow().is_some()
    }

    /// How many lines [`App::read_task_line`] has scanned.
    #[cfg(test)]
    pub(super) fn task_lines_scanned(&self) -> u64 {
        self.task_memo
            .lines
            .borrow()
            .as_ref()
            .map_or(0, |(_, checkpoints)| checkpoints.scanned())
    }

    /// The task whose box line `line` of the editor's text opens, as the
    /// whole note reads it: not a line in a fenced block, nor a later line
    /// of a task's text. `None` with tasks off.
    pub(super) fn box_on(&self, line: usize) -> Option<block::Task> {
        if !self.preferences.tasks || self.editor.tasks().is_none() {
            return None;
        }
        self.read_task_line(line, |text, info, scanner| {
            let task = checkpoints::opened_task(info, scanner)?;
            task::parse(text, task).map(|_| task)
        })
        .flatten()
    }

    /// The timer that looks for a new day: only while tasks are on.
    pub(super) fn day_subscription(&self) -> Option<Subscription<Message>> {
        self.preferences
            .tasks
            .then(|| iced::time::every(DAY_CHECK).map(|_| Message::DayCheck))
    }
}

#[cfg(test)]
mod tests {
    use wren_core::keybind;

    use super::super::commands::{definitions, CommandId};
    use super::super::palette::{Key as Row, Mode};
    use super::super::preferences::Preferences;
    use super::super::Options;
    use super::*;

    fn date(text: &str) -> Date {
        Date::parse_iso(text).unwrap()
    }

    /// An app whose `config.conf` holds `settings`.
    fn app(settings: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let config = directory.path().join("config.conf");
        std::fs::write(&config, settings).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (app, _) = App::with_config(options, None, Some(config));
        (directory, app)
    }

    #[test]
    fn today_is_the_local_date_until_a_pin_and_then_the_pin() {
        let (_directory, mut app) = app("tasks = on\n");
        let local = wren_core::ops::local_today();
        let today = app.today();
        assert!(today == local || today == wren_core::ops::local_today());
        app.pin_today(date("2026-11-19"));
        assert_eq!(app.today(), date("2026-11-19"));
        assert_eq!(
            app.today().weekday(),
            wren_markdown::date::Weekday::Thursday
        );
        let pinned = Clock::new(Some(date("2026-11-19")));
        assert_eq!(pinned.seen(), date("2026-11-19"));
        assert_eq!(pinned.changes(), 0);
    }

    /// `WREN_TEST_TODAY`, read at launch into the options, reaches the
    /// clock with no day change.
    #[cfg(feature = "smoke-test")]
    #[test]
    fn the_launch_pin_reaches_today() {
        let options = Options {
            editor: true,
            today: Some(date("2026-11-19")),
            ..Options::default()
        };
        let (app, _) = App::new(options);
        assert_eq!(app.today(), date("2026-11-19"));
        assert_eq!(
            (app.clock.seen(), app.clock.changes()),
            (date("2026-11-19"), 0)
        );
    }

    #[test]
    fn the_day_change_fires_once_per_change() {
        let (_directory, mut app) = app("tasks = on\n");
        app.clock = Clock::new(Some(date("2026-11-19")));
        app.check_day();
        assert_eq!(app.clock.changes(), 0, "the launch day is no change");
        app.pin_today(date("2026-11-20"));
        assert_eq!(
            (app.clock.changes(), app.clock.seen()),
            (1, date("2026-11-20"))
        );
        for _ in 0..3 {
            app.check_day();
            let _ = app.update(Message::DayCheck);
        }
        app.pin_today(date("2026-11-20"));
        assert_eq!(app.clock.changes(), 1, "the same day again is no change");
        app.pin_today(date("2026-11-19"));
        assert_eq!(
            (app.clock.changes(), app.clock.seen()),
            (2, date("2026-11-19"))
        );
    }

    /// With tasks off, launching reads no local date.
    #[test]
    fn with_tasks_off_the_clock_reads_no_date_at_launch() {
        let (_directory, off) = app("tasks = off\n");
        assert_eq!(off.clock.seen(), unread());
        let (_directory, on) = app("tasks = on\n");
        assert_ne!(on.clock.seen(), unread());
    }

    #[test]
    fn with_tasks_off_a_day_check_handles_nothing() {
        let (_directory, mut app) = app("tasks = off\n");
        app.clock = Clock::new(Some(date("2026-11-19")));
        app.pin_today(date("2026-11-20"));
        let _ = app.update(Message::DayCheck);
        assert_eq!(
            (app.clock.changes(), app.clock.seen()),
            (0, date("2026-11-19"))
        );
    }

    /// The dated note and the New note dialog's name read the one clock.
    #[test]
    fn new_dated_notes_take_todays_date_from_the_clock() {
        use wren_core::ops::Session;

        use super::super::commands::Command;
        let directory = crate::test_support::tempdir();
        std::fs::create_dir_all(directory.path().join("Alpha/Inbox")).unwrap();
        std::fs::write(directory.path().join("Alpha/Inbox/First.md"), "first").unwrap();
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        let inbox = app.model().unwrap().groups[0].id;
        let _ = app.dispatch(Command::SelectParent(wren_core::model::Parent::Group(
            inbox,
        )));
        // A date no test machine's clock shows.
        app.pin_today(date("2031-02-03"));
        let _ = app.dispatch(Command::Run(CommandId::NewNote));
        assert_eq!(app.dialog.as_ref().unwrap().name, "2031-02-03");
        let _ = app.update(Message::CancelDialog);
        let _ = app.dispatch(Command::Run(CommandId::NewDatedNote));
        assert!(app
            .model()
            .unwrap()
            .notes
            .iter()
            .any(|note| note.title == "2031-02-03"));
        assert!(directory.path().join("Alpha/Inbox/2031-02-03.md").exists());
    }

    /// A note whose every line reads one way with tasks on and another
    /// with them off.
    const STATUS_NOTE: &str =
        "# Status\n\n- [/] Doing teh draft\n- [界] Wide box\n- [-] Dropped idea\n- [/]\n  ===\n";

    /// An app on `engine` with `STATUS_NOTE` open in a workspace of its
    /// own, opened the way the app opens one, with spell check loaded and
    /// today pinned.
    fn status_app(engine: &str, tasks: &str) -> (tempfile::TempDir, App) {
        use wren_core::ops::{Op, Session};

        use super::super::spelling::{dictionary, Loaded};
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("workspace/Tasks/Status");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("Lines.md"), STATUS_NOTE).unwrap();
        let config = directory.path().join("config/config.conf");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            format!("editor-engine = {engine}\nspell-check = on\ntasks = {tasks}\n"),
        )
        .unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.clock = Clock::new(Some(date("2026-11-19")));
        app.apply_preferences();
        app.workspace_mode = true;
        let workspace = directory.path().join("workspace");
        app.session = Some(Session::open_with(&workspace, app.session_options()).unwrap());
        app.sync_editor(true);
        let note = app.model().unwrap().notes[0].id;
        let _ = app.process_core(Op::OpenNote { note: Some(note) });
        assert_eq!(app.editor.text(), STATUS_NOTE, "{engine}");
        let _ = app.update(Message::SpellLoaded(Loaded::new(dictionary(), None)));
        (directory, app)
    }

    /// The note after `edit` with the caret at `column` of `line`; one
    /// undo then restores it.
    fn after(app: &mut App, line: usize, column: usize, edit: &str) -> String {
        use iced::keyboard::key::Named;
        use iced::keyboard::{Key, Modifiers};
        use iced::widget::text_editor::{Cursor, Position};

        use super::super::commands::Command;
        app.editor.move_cursor(Cursor {
            position: Position { line, column },
            selection: None,
        });
        match edit {
            "toggle" => {
                let _ = app.dispatch(Command::Run(CommandId::ToggleTask));
            }
            key => {
                let named = if key == "enter" {
                    Named::Enter
                } else {
                    Named::Backspace
                };
                for message in app
                    .editor
                    .key_message(Key::Named(named), Modifiers::empty())
                {
                    let _ = app.update(Message::Editor(message));
                }
            }
        }
        let text = app.editor.text();
        let draft = &app.session.as_ref().unwrap().document().unwrap().text;
        assert_eq!(&text, draft, "the edit reaches the session");
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), STATUS_NOTE, "one undo restores the note");
        text
    }

    fn outline_titles(app: &mut App) -> Vec<String> {
        let _ = app.open_palette(Mode::Outline);
        let titles = app
            .palette
            .as_ref()
            .expect("the outline opens")
            .results
            .iter()
            .map(|row| row.title.clone())
            .collect();
        let _ = app.close_palette();
        titles
    }

    /// With tasks on, nothing in the app reads a task line with the
    /// default options: the editor's keys and highlighter, the prose spell
    /// check reads, the outline, and the Session's previews and snippets
    /// all read a box of any status, on both engines. With tasks off the
    /// same paths read the note as Wren did before tasks.
    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a line often has one strike or prose range
    fn every_reader_of_a_task_line_takes_the_apps_task_options() {
        use wren_core::index::{QueryKind, Scope};
        use wren_editor::Palette;
        use wren_markdown::TaskOptions;

        let ink = iced::Color::BLACK;
        let palette = Palette {
            text: ink,
            muted: ink,
            faint: ink,
            accent: ink,
            heading: ink,
            heading2: ink,
            link: ink,
            code: ink,
            quote: ink,
            completed: ink,
            date: ink,
            overdue: ink,
            owner: ink,
        };
        let lines: Vec<&str> = STATUS_NOTE.split('\n').collect();
        let replaced = |line: usize, new: &[&str]| {
            [&lines[..line], new, &lines[line + 1..]]
                .concat()
                .join("\n")
        };
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = status_app(engine, "on");
            let today = date("2026-11-19");
            assert_eq!(app.task_options(), Some(TaskOptions { today }), "{engine}");
            assert_eq!(app.task_boxes(), Boxes::Status, "{engine}");

            // The editor: its options, its highlighter, its keys.
            assert_eq!(app.editor.tasks(), app.task_options(), "{engine}");
            let highlighted = app.editor.highlighted(palette);
            let boxes: Vec<usize> = highlighted.iter().map(|(_, decor)| decor.hang).collect();
            assert_eq!(boxes, [0, 0, 2, 2, 2, 2, 2, 0], "{engine}");
            assert_eq!(highlighted[2].0[0].0, 0..6, "{engine}: the box is a marker");
            assert_eq!(highlighted[3].0[0].0, 0..8, "{engine}: a wide box too");
            assert_eq!(highlighted[4].1.strikes, [6..18], "{engine}: cancelled");
            assert_eq!(
                after(&mut app, 3, lines[3].len(), "enter"),
                replaced(3, &[lines[3], "- [ ] "]),
                "{engine}"
            );
            assert_eq!(
                after(&mut app, 3, 8, "backspace"),
                replaced(3, &["- Wide box"]),
                "{engine}"
            );
            assert_eq!(
                after(&mut app, 2, 0, "toggle"),
                replaced(2, &["- [x] Doing teh draft ✅ 2026-11-19"]),
                "{engine}"
            );
            assert_eq!(
                after(&mut app, 4, 0, "toggle"),
                replaced(4, &["- [ ] Dropped idea"]),
                "{engine}"
            );

            // Spelling: the prose starts after the box, and the editor's
            // proofreader marks the one misspelled word in it.
            assert_eq!(highlighted[2].1.prose, [6..21], "{engine}");
            assert_eq!(highlighted[3].1.prose, [8..16], "{engine}");
            let marks = app.editor.marks(2);
            if engine == "advanced" {
                assert_eq!(marks, [12..15], "teh");
            } else {
                assert!(marks.is_empty(), "the classic engine has no proofreader");
            }

            // The outline: an empty task is no heading's text.
            assert_eq!(outline_titles(&mut app), ["Status"], "{engine}");

            // The Session and its index.
            let session = app.session.as_ref().unwrap();
            assert!(session.options().tasks, "{engine}");
            let note = session.model().notes[0].id;
            assert_eq!(session.index().preview(note), Some("Doing teh draft"));
            let hit = &session
                .index()
                .query("wide", Scope::Workspace, QueryKind::Search, 10)[0];
            assert!(
                hit.snippet
                    .text
                    .contains("Doing teh draft · Wide box · Dropped idea"),
                "{engine}: {:?}",
                hit.snippet.text
            );

            // A new day reaches the editor through the day change.
            app.pin_today(date("2026-11-20"));
            assert_eq!(
                app.editor.tasks(),
                Some(TaskOptions {
                    today: date("2026-11-20")
                }),
                "{engine}"
            );

            // Off, the same paths read the three plain boxes only.
            let (_directory, mut app) = status_app(engine, "off");
            assert_eq!(app.task_options(), None, "{engine}");
            assert_eq!(app.task_boxes(), Boxes::Plain, "{engine}");
            assert_eq!(app.editor.tasks(), None, "{engine}");
            let highlighted = app.editor.highlighted(palette);
            assert_eq!(highlighted[2].0[0].0, 0..2, "{engine}: only the marker");
            assert!(highlighted[4].1.strikes.is_empty(), "{engine}");
            assert_ne!(highlighted[2].1.prose, [6..21], "{engine}");
            assert_eq!(
                after(&mut app, 3, lines[3].len(), "enter"),
                replaced(3, &[lines[3], "- "]),
                "{engine}"
            );
            assert_eq!(
                after(&mut app, 3, 8, "backspace"),
                replaced(3, &["- [界]Wide box"]),
                "{engine}"
            );
            assert_eq!(
                after(&mut app, 4, 0, "toggle"),
                replaced(4, &["- [ ] [-] Dropped idea"]),
                "{engine}"
            );
            assert_eq!(outline_titles(&mut app), ["Status", "[/]"], "{engine}");
            let session = app.session.as_ref().unwrap();
            assert!(!session.options().tasks, "{engine}");
            let note = session.model().notes[0].id;
            assert_eq!(session.index().preview(note), Some("[/] Doing teh draft"));
            app.pin_today(date("2026-11-20"));
            assert_eq!(app.editor.tasks(), None, "{engine}");
        }
    }

    #[test]
    fn the_day_timer_runs_only_with_tasks_on() {
        let (_directory, app) = app("tasks = on\n");
        assert!(app.preferences.tasks);
        assert!(app.day_subscription().is_some());
        for settings in ["", "tasks = off\n"] {
            let (_directory, app) = self::app(settings);
            assert!(!app.preferences.tasks, "{settings:?}");
            assert!(app.day_subscription().is_none(), "{settings:?}");
        }
    }

    #[test]
    fn a_changed_tasks_setting_waits_for_a_restart_and_says_so() {
        let (directory, mut app) = app("");
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "tasks = on\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `tasks`.")
        );
        assert!(!app.preferences.tasks);
        assert!(app.preferences.settings.tasks);
        std::fs::write(&path, "tasks = on\nboards = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `boards` and `tasks`.")
        );
        std::fs::write(&path, "tasks = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(app.banner.is_none());
    }

    /// The gate's rule for every command, which a command that `tasks()`
    /// names must keep: with the setting off it is neither listed nor run.
    #[test]
    fn with_tasks_off_no_task_command_is_listed_or_runs() {
        let (_directory, mut app) = app("");
        assert!(!app.preferences.tasks);
        let _ = app.open_palette(Mode::Commands);
        let listed: Vec<CommandId> = app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .filter_map(|row| match row.key {
                Row::Command(id) => Some(id),
                _ => None,
            })
            .collect();
        let _ = app.close_palette();
        assert!(!listed.is_empty());
        assert!(listed.iter().all(|id| !id.tasks()), "{listed:?}");
        for definition in definitions().filter(|definition| definition.id.tasks()) {
            assert!(!app.command_enabled(definition.id), "{}", definition.key);
        }
    }

    /// With tasks off the keymap is the one Wren had before tasks, on both
    /// platforms, and a `keybind` line naming a task command is refused.
    #[test]
    fn with_tasks_off_the_keymap_is_unchanged_on_both_platforms() {
        for scheme in super::super::preferences::SCHEMES {
            let (off, warnings) = super::super::preferences::build_keymap(scheme, &[], false);
            assert!(warnings.is_empty());
            let (before, _) = wren_core::keybind::Keymap::build(
                scheme,
                keybind::default_bindings(scheme)
                    .into_iter()
                    .chain(keybind::default_board_bindings(scheme)),
                &[],
                super::super::preferences::layer_of,
            );
            assert_eq!(off, before, "{scheme:?}");
            let (on, _) = super::super::preferences::build_keymap(scheme, &[], true);
            let tasks = keybind::default_task_bindings(scheme);
            assert_eq!(
                on.bindings().count(),
                off.bindings().count() + tasks.len(),
                "{scheme:?}"
            );
            for (trigger, action) in &tasks {
                let id = CommandId::parse(action).expect("a task default names a command");
                assert!(id.tasks(), "{trigger} → {action}");
                let accel = keybind::parse_trigger(trigger).unwrap();
                assert_ne!(off.action(&accel), Some(*action), "{scheme:?}");
            }
        }
        // The keys the design gives task commands (discovery/tasks.md §2
        // items 21) stay as they were with tasks off.
        for (scheme, triggers) in super::super::preferences::SCHEMES.into_iter().zip([
            ["super+d", "super+shift+enter", "super+shift+t"],
            ["ctrl+d", "ctrl+shift+enter", "ctrl+shift+t"],
            ["alt+d", "alt+shift+enter", "alt+shift+t"],
        ]) {
            let (off, _) = super::super::preferences::build_keymap(scheme, &[], false);
            for trigger in triggers {
                let accel = keybind::parse_trigger(trigger).unwrap();
                let bound = off.action(&accel).or(off.board_action(&accel));
                assert!(
                    bound
                        .and_then(CommandId::parse)
                        .is_none_or(|id| !id.tasks()),
                    "{scheme:?} {trigger} is {bound:?}"
                );
            }
        }
        let preferences = Preferences::load(None);
        assert!(!preferences.tasks && preferences.settings.warnings.is_empty());
    }

    /// A note of task lines, read by the status bar's task line.
    const FOOTER_NOTE: &str = "# Status bar\n\
        \n\
        - [/] Draft the cutover runbook @alice.chen ⏫ 📅 2026-11-23\n\
        \x20 still the same item, on its next line\n\
        - [ ] Send the deck @sam @priya 🔼 📅 2026-11-16\n\
        Not a task line.\n\
        ```\n\
        - [ ] In a fence\n\
        ```\n\
        > - [x] Quoted 📅 2026-11-12 ✅ 2026-11-16";

    /// The app on `engine` with `tasks`, today pinned to Thursday 19
    /// November 2026 and [`FOOTER_NOTE`] in its editor.
    fn footer_app(engine: &str, tasks: &str) -> (tempfile::TempDir, App) {
        let (directory, mut app) = app(&format!("editor-engine = {engine}\ntasks = {tasks}\n"));
        app.clock = Clock::new(Some(date("2026-11-19")));
        app.sync_task_options();
        app.editor.replace_text(FOOTER_NOTE, false);
        (directory, app)
    }

    /// The caret's task line as the footer shows it, with the caret on
    /// `line`.
    fn footer_at(app: &mut App, line: usize) -> Option<String> {
        app.editor.move_cursor(iced::widget::text_editor::Cursor {
            position: iced::widget::text_editor::Position { line, column: 3 },
            selection: None,
        });
        app.caret_task().map(|summary| summary.text())
    }

    #[test]
    fn the_footer_spells_out_the_caret_lines_task_on_both_engines() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = footer_app(engine, "on");
            assert_eq!(
                footer_at(&mut app, 2).as_deref(),
                Some("Doing · due Mon 23 Nov, in 4 days · ⏫ High · @alice.chen"),
                "{engine}"
            );
            // The item's next line is not the task's own.
            assert_eq!(footer_at(&mut app, 3), None, "{engine}");
            let late = footer_at(&mut app, 4);
            assert_eq!(
                late.as_deref(),
                Some("Open · due Mon 16 Nov, 3 days overdue · 🔼 Medium · @sam @priya"),
                "{engine}"
            );
            assert!(app.caret_task().is_some_and(|summary| summary.overdue()));
            for line in [0, 1, 5, 6, 7, 8] {
                assert_eq!(footer_at(&mut app, line), None, "{engine} line {line}");
            }
            assert_eq!(
                footer_at(&mut app, 9).as_deref(),
                Some("Done · due Thu 12 Nov · yours · done Mon 16 Nov"),
                "{engine}"
            );
        }
    }

    #[test]
    fn the_footers_relative_dates_follow_the_day_change() {
        let (_directory, mut app) = footer_app("advanced", "on");
        let due = |app: &mut App| {
            footer_at(app, 2)
                .unwrap()
                .split(" · ")
                .nth(1)
                .map(str::to_owned)
        };
        assert_eq!(due(&mut app).as_deref(), Some("due Mon 23 Nov, in 4 days"));
        app.pin_today(date("2026-11-22"));
        assert_eq!(due(&mut app).as_deref(), Some("due Mon 23 Nov, tomorrow"));
        app.pin_today(date("2026-11-25"));
        assert_eq!(
            due(&mut app).as_deref(),
            Some("due Mon 23 Nov, 2 days overdue")
        );
    }

    #[test]
    fn with_tasks_off_the_footer_has_no_task() {
        for engine in ["classic", "advanced"] {
            let (_directory, mut app) = footer_app(engine, "off");
            for line in 0..10 {
                assert_eq!(footer_at(&mut app, line), None, "{engine} line {line}");
            }
        }
    }

    /// The line found for the footer is kept, and the task spelled out: a
    /// view with nothing changed reads and formats nothing, and an edit, a
    /// caret move, a note switch, an engine change and tasks turning off
    /// each read afresh. A day change formats again without a read, and so
    /// moves the relative date.
    #[test]
    fn the_footers_task_is_read_and_spelled_out_once_until_something_changes() {
        use iced::widget::text_editor::{Action, Edit, Motion};
        let (_directory, mut app) = footer_app("advanced", "on");
        let counts = |app: &App| (app.task_memo.reads.get(), app.task_memo.formats.get());
        let example = "Doing · due Mon 23 Nov, in 4 days · ⏫ High · @alice.chen";
        assert_eq!(footer_at(&mut app, 2).as_deref(), Some(example));
        assert_eq!(counts(&app), (1, 1));
        for _ in 0..3 {
            assert_eq!(
                app.caret_task().map(|summary| summary.text()).as_deref(),
                Some(example)
            );
        }
        assert_eq!(counts(&app), (1, 1), "nothing changed: no read, no format");
        // A miss is kept too.
        assert_eq!(footer_at(&mut app, 5), None);
        assert_eq!(app.caret_task(), None);
        assert_eq!(counts(&app), (2, 1), "a caret move, and a miss kept");
        // The same line again after a move away is read again.
        assert!(footer_at(&mut app, 2).is_some());
        assert_eq!(counts(&app), (3, 2));
        app.pin_today(date("2026-11-22"));
        assert_eq!(
            app.caret_task()
                .map(|summary| summary.pieces[1].text.clone())
                .as_deref(),
            Some("due Mon 23 Nov, tomorrow")
        );
        assert_eq!(
            counts(&app),
            (3, 3),
            "a day change formats, and reads no line"
        );
        assert!(app.caret_task().is_some());
        assert_eq!(counts(&app), (3, 3));
        // An edit at the line's end: the caret stays on it, and it is read
        // again.
        app.editor.perform(Action::Move(Motion::End));
        assert!(app.caret_task().is_some());
        assert_eq!(counts(&app), (3, 3), "a move along the line keeps its task");
        assert!(app.editor.perform(Action::Edit(Edit::Insert('z'))));
        assert_eq!(app.editor.cursor().position.line, 2);
        assert!(app.caret_task().is_some());
        assert_eq!(counts(&app), (4, 4), "an edit");
        assert!(app.caret_task().is_some());
        assert_eq!(counts(&app), (4, 4));
        // A note switch puts other text in at the same caret line.
        app.editor.replace_text("one\ntwo\n- [ ] Three", true);
        app.editor.move_cursor(iced::widget::text_editor::Cursor {
            position: iced::widget::text_editor::Position { line: 2, column: 0 },
            selection: None,
        });
        assert_eq!(
            app.caret_task().map(|summary| summary.text()).as_deref(),
            Some("Open · yours")
        );
        assert_eq!(counts(&app), (5, 5), "a note switch");
        let text = app.editor.text();
        assert!(app
            .editor
            .set_backend(wren_editor::BackendKind::Iced, &text));
        assert!(app.caret_task().is_some());
        assert_eq!(counts(&app), (6, 6), "an engine change");
        // The editor reading boxes as with tasks off gives no task spans.
        app.editor.set_tasks(None);
        assert_eq!(app.caret_task(), None);
        assert_eq!(counts(&app), (7, 6), "the boxes read changed");
    }
}
