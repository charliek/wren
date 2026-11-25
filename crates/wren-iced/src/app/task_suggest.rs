//! Task suggestions (discovery/tasks.md §6 item 2, §3.1 Q8, §3.2 Q12;
//! item 5.1): typing on a task line offers "📅 Due date…", which opens
//! the calendar, a date word written at once, a priority, or after an `@`
//! the owners already used in the workspace. What is offered, and which
//! offers Enter may choose, is `wren_markdown::task::suggest`'s rule; this
//! module hands it to the editor as its source of suggestions and carries
//! out the row chosen.
//!
//! They exist with `tasks` and `tasks-suggest` on, on the advanced engine
//! (which alone opens popovers), on the line of a task's box as the whole
//! note reads it: not inside a fenced block, and not on a later line of a
//! task's text, but on a task nested under another item. With either
//! setting off the editor has no source of suggestions, as before tasks:
//! Tab indents and Enter continues a list.
//!
//! A row chosen rewrites the task's line through
//! `wren_markdown::task::edit`, as one undo step: the typed word out and
//! the token written, or the `@name` completed. "Due date…" opens the
//! calendar under the word, which takes the word out with the date it
//! writes, and leaves it where it is when cancelled.
//!
//! **Owners** come from the Markdown notes the search index holds
//! (`wren_core::owners::Owners`), brought up to the index after every
//! message while the caret is on a task line, on the UI thread within a
//! measured budget: a note is read again only when its source changed, so
//! typing reads the one note typed in, leaving out the line typed on. The
//! owners offered are always those of the index as it is now, of this
//! workspace: another index starts afresh, and with none they are dropped.
//! When they change, open suggestions are asked for again, and a row of
//! owners carries the generation of the list it was offered from, so a
//! row of an older list chooses nothing.

use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use iced::Task;
use wren_core::owners::Owner;
use wren_editor::{Choice, Row, SuggestSource, Suggestion};
use wren_markdown::block;
use wren_markdown::date::Date;
use wren_markdown::task::suggest::{self, Item, Known};
use wren_markdown::task::{self, Priority};

use super::{App, Message};

/// The tag of a suggestion row's number: bit 29, with bits 30 (a task
/// menu's row) and 31 (a command's payload) clear, so none is taken for
/// another.
const ROW: u32 = 0x2000_0000;
const KIND_SHIFT: u32 = 24;
const PAYLOAD_MASK: u32 = (1 << KIND_SHIFT) - 1;
/// Where an owner row's list generation sits in its payload, over the
/// owner's place in that list.
const GENERATION_SHIFT: u32 = 16;
const PLACE_MASK: u32 = (1 << GENERATION_SHIFT) - 1;

/// What a suggestion row does, carried as its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SuggestRow {
    Calendar,
    Due(Date),
    Priority(Priority),
    /// The owner at `place` of the list of `generation` (its low 8 bits).
    Owner {
        generation: u8,
        place: u16,
    },
}

impl SuggestRow {
    fn id(self) -> u32 {
        let (kind, payload) = match self {
            Self::Calendar => (1, 0),
            Self::Due(date) => (
                2,
                (u32::from(date.year()) << 9)
                    | (u32::from(date.month()) << 5)
                    | u32::from(date.day()),
            ),
            Self::Priority(level) => (3, level_number(level)),
            Self::Owner { generation, place } => (
                4,
                (u32::from(generation) << GENERATION_SHIFT) | u32::from(place),
            ),
        };
        ROW | (kind << KIND_SHIFT) | payload
    }

    /// The row a number names, if it is exactly one's.
    pub(super) fn from_id(id: u32) -> Option<Self> {
        if id & !(ROW | (0x1f << KIND_SHIFT) | PAYLOAD_MASK) != 0 || id & ROW == 0 {
            return None;
        }
        let payload = id & PAYLOAD_MASK;
        let row = match (id >> KIND_SHIFT) & 0x1f {
            1 => Self::Calendar,
            2 => Self::Due(Date::new(
                u16::try_from(payload >> 9).ok()?,
                u8::try_from((payload >> 5) & 0xf).ok()?,
                u8::try_from(payload & 0x1f).ok()?,
            )?),
            3 => {
                Self::Priority(*Priority::ALL.get(usize::try_from(payload).ok()?.checked_sub(1)?)?)
            }
            4 => Self::Owner {
                generation: u8::try_from(payload >> GENERATION_SHIFT).ok()?,
                place: u16::try_from(payload & PLACE_MASK).ok()?,
            },
            _ => return None,
        };
        (row.id() == id).then_some(row)
    }
}

/// A level's place in [`Priority::ALL`], from one.
fn level_number(level: Priority) -> u32 {
    let place = Priority::ALL.iter().position(|each| *each == level);
    u32::try_from(place.expect("every level is listed") + 1).expect("five levels")
}

/// The low 8 bits of a list generation, which an owner row carries.
fn short(generation: u64) -> u8 {
    (generation & 0xff) as u8
}

/// What the source answers from.
struct State {
    /// The caret's line and its task box, when the whole note reads it as
    /// the line of a task's box ([`App::sync_due_target`]).
    line: Option<(usize, block::Task)>,
    today: Date,
    /// The owners, best first, as offered and as the rule reads them.
    owners: Arc<Vec<Owner>>,
    known: Arc<Vec<Known>>,
    /// The generation of that list ([`wren_core::owners::Owners`]).
    generation: u64,
}

/// The editor's source of suggestions while they are on. It holds what
/// the app last told it, so it answers the same for the same line until
/// the app tells it more, which it does only between messages.
pub(super) struct TaskSuggestions {
    state: Mutex<State>,
}

impl Default for TaskSuggestions {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                line: None,
                today: Date::new(2026, 1, 1).expect("a date"),
                owners: Arc::default(),
                known: Arc::default(),
                generation: 0,
            }),
        }
    }
}

impl TaskSuggestions {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The owner an owner row names, if it was offered from the list the
    /// source holds now.
    fn owner(&self, generation: u8, place: u16) -> Option<String> {
        let state = self.state();
        (short(state.generation) == generation)
            .then(|| {
                state
                    .owners
                    .get(usize::from(place))
                    .map(|owner| owner.name.clone())
            })
            .flatten()
    }

    /// Offer `owners`, the list of `generation`.
    fn set_owners(&self, owners: Arc<Vec<Owner>>, generation: u64) {
        let known = owners
            .iter()
            .map(|owner| Known::new(owner.name.clone()))
            .collect();
        let mut state = self.state();
        state.owners = owners;
        state.known = Arc::new(known);
        state.generation = generation;
    }

    /// The owners it offers now, best first.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn owners(&self) -> Arc<Vec<Owner>> {
        Arc::clone(&self.state().owners)
    }
}

impl State {
    fn row(&self, suggested: &suggest::Suggested) -> Option<Row> {
        let row = match &suggested.item {
            Item::Calendar => Row::new(SuggestRow::Calendar.id(), &suggested.label)
                .glyph(task::DateField::Due.glyph().to_string())
                .detail("calendar")
                .opens_menu(),
            Item::Due(date) => Row::new(SuggestRow::Due(*date).id(), &suggested.label)
                .glyph(task::DateField::Due.glyph().to_string())
                .detail(date.short().to_string()),
            Item::Priority(level) => Row::new(SuggestRow::Priority(*level).id(), &suggested.label)
                .glyph(level.glyph().to_string()),
            Item::Owner { index, .. } => {
                let notes = self.owners.get(*index)?.notes;
                let place = u16::try_from(*index).ok()?;
                let id = SuggestRow::Owner {
                    generation: short(self.generation),
                    place,
                }
                .id();
                Row::new(id, &suggested.label).detail(if notes == 1 {
                    "1 note".to_owned()
                } else {
                    format!("{notes} notes")
                })
            }
        };
        Some(row.tentative(!suggested.enter))
    }
}

impl SuggestSource for TaskSuggestions {
    /// Nothing: whether a line is a task's is the whole note's to say, so
    /// the source answers only when told the line's number.
    fn suggest(&self, _line: &str, _caret: usize) -> Option<Suggestion> {
        None
    }

    fn suggest_on(&self, index: usize, line: &str, caret: usize) -> Option<Suggestion> {
        let state = self.state();
        let (_, task) = state.line.filter(|(at, _)| *at == index)?;
        let offer = suggest::suggest(line, caret, task, state.today, &state.known)?;
        let rows = offer
            .items
            .iter()
            .filter_map(|item| state.row(item))
            .collect();
        Some(Suggestion {
            range: offer.range,
            rows,
        })
    }
}

impl App {
    /// Whether suggestions are on: both settings.
    fn suggestions_on(&self) -> bool {
        self.preferences.tasks && self.preferences.settings.tasks_suggest
    }

    /// Tell the source of suggestions the caret's line and its task box,
    /// when the whole note reads it as the line of a task's box: from
    /// [`App::sync_due_target`], which finds that out once for each text,
    /// caret line and setting.
    pub(super) fn suggest_on_line(&self, line: Option<(usize, block::Task)>) {
        self.suggestions.state().line = line;
    }

    /// Bring the editor's source of suggestions in step with the settings,
    /// today and the workspace's owners, after every message.
    pub(super) fn sync_suggestions(&mut self) {
        let on = self.suggestions_on();
        if on != self.suggest_installed {
            self.editor.set_suggest_source(
                on.then(|| Arc::clone(&self.suggestions) as Arc<dyn SuggestSource>),
            );
            self.suggest_installed = on;
        }
        let line = {
            let mut state = self.suggestions.state();
            state.today = self.today();
            state.line.map(|(line, _)| line)
        };
        let Some(identity) = (self.session.as_ref()).map(|session| session.index().identity())
        else {
            if !self.owners.ranked().is_empty() {
                self.owners.clear();
                self.offer_owners();
            }
            return;
        };
        // Another workspace: what was read from the one before goes now,
        // wanted or not.
        if self
            .owners
            .identity()
            .is_some_and(|known| known != identity)
        {
            self.owners.clear();
            self.offer_owners();
        }
        // The task form offers the same owners, on either engine and with
        // suggestions off; the line it writes is a whole one.
        let target = if self.task_form.is_some() {
            Some(None)
        } else {
            line.filter(|_| on).map(Some)
        };
        let (Some(line), Some(session)) = (target, self.session.as_ref()) else {
            return;
        };
        let index = session.index();
        // The line typed on names no one yet.
        let skip = line.and_then(|line| session.document().map(|document| (document.note, line)));
        if self.owners.is_current(identity, index.revision(), skip) {
            return;
        }
        let changed = self.owners.update(
            identity,
            index.revision(),
            index.markdown_sources(),
            skip,
            SystemTime::now(),
        );
        if changed {
            self.offer_owners();
        }
    }

    /// Hand the source the owners as they are now, and ask again for the
    /// suggestions open, which may be owners of the list before.
    fn offer_owners(&mut self) {
        self.suggestions
            .set_owners(Arc::clone(self.owners.ranked()), self.owners.generation());
        self.editor.refresh_suggestions();
    }

    /// The task box the whole note reads on line `line`, when the line is
    /// a task's box line.
    pub(super) fn box_on(&self, line: usize) -> Option<block::Task> {
        let nothing = task::Change::Compound(Vec::new());
        let at = wren_markdown::Selection::caret(wren_markdown::Pos::new(line, 0));
        self.editor.read_markdown(|lines, options| {
            task::edit(lines, line, at, &nothing, options)
                .filter(|edit| edit.line == line)
                .map(|edit| edit.task)
        })
    }

    /// A row chosen from the suggestions ([`wren_editor::Mode::Suggest`]):
    /// rewrite the task's line, or open the calendar. `None` for a row of
    /// no suggestion's. A row of owners offered from a list that has since
    /// changed chooses nothing.
    pub(super) fn suggestion_row(&mut self, choice: &Choice) -> Option<Task<Message>> {
        let row = SuggestRow::from_id(choice.id)?;
        self.column = super::columns::Column::Editor;
        let open = self.suggestions_on()
            && self.task_lines_editable()
            && self.editor.backend() == wren_editor::BackendKind::Owned;
        let text = self.editor.line_text(choice.line).filter(|_| open);
        let Some((text, task)) = text.zip(self.box_on(choice.line)) else {
            return Some(Task::none());
        };
        let range = choice.range.clone();
        let item = match row {
            SuggestRow::Calendar => {
                return Some(self.open_calendar_for_word(choice.line, range));
            }
            SuggestRow::Due(date) => Item::Due(date),
            SuggestRow::Priority(level) => Item::Priority(level),
            SuggestRow::Owner { generation, place } => {
                let Some(name) = self.suggestions.owner(generation, place) else {
                    return Some(Task::none());
                };
                Item::Owner {
                    name,
                    index: usize::from(place),
                }
            }
        };
        let change = suggest::completion(&text, range.clone(), task, &item)?;
        Some(self.write_completion(choice.line, &text, &range, &change))
    }

    /// Write `change` to the task whose box is on `line`, which reads
    /// `text`, for the word at `range`, as one undo step. A word at the
    /// end of its line leaves the caret at the end of the new line, ready
    /// for Enter; elsewhere where the word was.
    pub(super) fn write_completion(
        &mut self,
        line: usize,
        text: &str,
        range: &Range<usize>,
        change: &task::Change,
    ) -> Task<Message> {
        let started = std::time::Instant::now();
        let edit = self.editor.plan_markdown(|lines, selection, options| {
            task::edit(lines, line, selection, change, options)
        });
        let Some(edit) = edit.filter(|edit| edit.line == line) else {
            return self.blocked("Put the caret on a task line").task;
        };
        let at_end = text[range.end..].trim().is_empty();
        let end = edit.text.len();
        match self.editor.apply_line_edits(&[edit]) {
            Ok(changed) => {
                if at_end {
                    self.editor.move_cursor(iced::widget::text_editor::Cursor {
                        position: iced::widget::text_editor::Position { line, column: end },
                        selection: None,
                    });
                }
                if changed {
                    self.text_changed(started);
                }
                Task::none()
            }
            Err(refused) => self.blocked(refused.to_string()).task,
        }
    }

    /// What `tasks.owners` answers: the owners offered, best first, with
    /// their note counts, and whether they were read from the index as it
    /// is now.
    #[cfg(feature = "smoke-test")]
    pub(super) fn owners_snapshot(&self) -> serde_json::Value {
        let owners = self.suggestions.owners();
        let current = self.session.as_ref().is_some_and(|session| {
            self.owners.identity() == Some(session.index().identity())
                && self.owners.revision() == Some(session.index().revision())
        });
        serde_json::json!({
            "owners": owners
                .iter()
                .map(|owner| serde_json::json!({"name": owner.name, "notes": owner.notes}))
                .collect::<Vec<_>>(),
            "current": current,
            "generation": self.owners.generation(),
            "on": self.suggestions_on(),
        })
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::Named;
    use iced::keyboard::{Key, Modifiers};
    use iced::widget::text_editor::{Cursor, Position};
    use wren_core::ops::{Op, Session};
    use wren_editor::Mode;

    use super::super::calendar::Event as CalendarEvent;
    use super::super::tasks::Clock;
    use super::super::Options;
    use super::*;
    use crate::date_field::FieldEvent;

    const LIST: &str = "# Tasks\n\
        \n\
        - [ ] Mail the deck\n\
        \n\
        ```\n\
        - [ ] fenced\n\
        ```\n";

    /// Another note of the workspace, naming owners on task lines.
    const MEETING: &str = "# Meeting\n\
        \n\
        - [ ] Send the notes @Priya.K\n\
        - [ ] Book the room @sam\n\
        - [x] Order lunch @Sam\n\
        Mail @prose about it.\n";

    /// An app on `engine` with `settings`, today pinned to Thursday 19
    /// November 2026, and the note `List` (holding `LIST`) open in a
    /// workspace with the note `Meeting`, the caret at the end of the
    /// task line.
    fn app_with(engine: &str, settings: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("workspace/Work/Tasks");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("List.md"), LIST).unwrap();
        std::fs::write(group.join("Meeting.md"), MEETING).unwrap();
        let config = directory.path().join("config/config.conf");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, format!("editor-engine = {engine}\n{settings}")).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.clock = Clock::new(Some(Date::parse_iso("2026-11-19").unwrap()));
        app.apply_preferences();
        app.workspace_mode = true;
        let workspace = directory.path().join("workspace");
        app.session = Some(Session::open_with(&workspace, app.session_options()).unwrap());
        app.sync_editor(true);
        open(&mut app, "List");
        assert_eq!(app.editor.text(), LIST, "{engine}");
        caret(&mut app, 2, "- [ ] Mail the deck".len());
        (directory, app)
    }

    fn app(engine: &str) -> (tempfile::TempDir, App) {
        app_with(engine, "tasks = on\n")
    }

    fn open(app: &mut App, title: &str) {
        let note = app
            .model()
            .unwrap()
            .notes
            .iter()
            .find(|note| note.title == title)
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote { note: Some(note) });
    }

    fn caret(app: &mut App, line: usize, column: usize) {
        app.editor.move_cursor(Cursor {
            position: Position { line, column },
            selection: None,
        });
        let _ = app.update(Message::Tick);
    }

    /// The owners are read after every message while the caret is on a
    /// task line: one more message, and they are those of the index now.
    fn scan_now(app: &mut App) {
        let _ = app.update(Message::Tick);
        let index = app.session.as_ref().unwrap().index();
        assert_eq!(app.owners.identity(), Some(index.identity()));
        assert_eq!(app.owners.revision(), Some(index.revision()));
    }

    /// A key pressed into the focused editor, as the widget routes it.
    fn press(app: &mut App, key: Key) {
        for message in app.editor.key_message(key, Modifiers::empty()) {
            let _ = app.update(Message::Editor(message));
        }
    }

    fn named(app: &mut App, key: Named) {
        press(app, Key::Named(key));
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            let key = if character == ' ' {
                Key::Named(Named::Space)
            } else {
                Key::Character(character.to_string().into())
            };
            press(app, key);
        }
    }

    fn line(app: &App, at: usize) -> String {
        app.editor.text().split('\n').nth(at).unwrap().to_owned()
    }

    /// The open suggestions' rows: glyph, label and note.
    fn rows(app: &App) -> Vec<(Option<String>, String, Option<String>)> {
        let Some(popover) = app.editor.popover() else {
            return Vec::new();
        };
        assert_eq!(popover.mode(), Mode::Suggest);
        (0..popover.labels().len())
            .map(|row| {
                (
                    popover.glyph(row).map(str::to_owned),
                    popover.labels()[row].clone(),
                    popover.detail(row).map(str::to_owned),
                )
            })
            .collect()
    }

    fn labels(app: &App) -> Vec<String> {
        rows(app).into_iter().map(|(_, label, _)| label).collect()
    }

    #[test]
    fn row_numbers_name_their_rows_and_no_others() {
        let rows = [
            SuggestRow::Calendar,
            SuggestRow::Due(Date::parse_iso("2026-11-19").unwrap()),
            SuggestRow::Due(Date::parse_iso("9999-12-31").unwrap()),
            SuggestRow::Priority(Priority::High),
            SuggestRow::Priority(Priority::Lowest),
            SuggestRow::Owner {
                generation: 0,
                place: 0,
            },
            SuggestRow::Owner {
                generation: 255,
                place: u16::MAX,
            },
        ];
        for row in rows {
            assert_eq!(SuggestRow::from_id(row.id()), Some(row), "{row:?}");
            // Never a task menu's row nor a command's payload.
            assert!(super::super::task_menus::TaskRow::from_id(row.id()).is_none());
            assert!(super::super::commands::CommandId::from_payload(row.id()).is_none());
        }
        for id in [0, 1, ROW, ROW | (9 << KIND_SHIFT), 0x4000_0000 | ROW] {
            assert_eq!(SuggestRow::from_id(id), None, "{id:#x}");
        }
    }

    #[test]
    fn typing_on_a_task_line_offers_dates_priorities_and_owners() {
        let (_directory, mut app) = app("advanced");
        type_text(&mut app, " to");
        assert_eq!(
            rows(&app),
            [
                (Some("📅".into()), "Today".into(), Some("Thu 19 Nov".into())),
                (
                    Some("📅".into()),
                    "Tomorrow".into(),
                    Some("Fri 20 Nov".into())
                ),
            ]
        );
        // A space closes them; the text is the user's.
        type_text(&mut app, " ");
        assert!(app.editor.popover().is_none());
        type_text(&mut app, "hi");
        assert_eq!(
            rows(&app),
            [
                (Some("⏫".into()), "High priority".into(), None),
                (Some("🔺".into()), "Highest priority".into(), None),
            ]
        );
        named(&mut app, Named::Backspace);
        assert!(app.editor.popover().is_none());
        named(&mut app, Named::Backspace);
        type_text(&mut app, "du");
        assert_eq!(
            rows(&app),
            [(
                Some("📅".into()),
                "Due date…".into(),
                Some("calendar".into())
            )]
        );
        assert_eq!(line(&app, 2), "- [ ] Mail the deck to du");
    }

    #[test]
    fn owners_come_from_the_workspace_case_aside_and_complete_in_place() {
        let (_directory, mut app) = app("advanced");
        scan_now(&mut app);
        let names: Vec<String> = app
            .suggestions
            .owners()
            .iter()
            .map(|owner| owner.name.clone())
            .collect();
        // `@sam` and `@Sam` are one owner, in two notes' worth of lines;
        // prose is no task line.
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"Priya.K".to_owned()));
        type_text(&mut app, " @");
        assert_eq!(labels(&app).len(), 2);
        type_text(&mut app, "pr");
        assert_eq!(
            rows(&app),
            [(None, "@Priya.K".into(), Some("1 note".into()))]
        );
        // Enter leaves a name as typed, as it may be a new one; Tab
        // completes it.
        let mut enter = app_with("advanced", "tasks = on\n");
        scan_now(&mut enter.1);
        type_text(&mut enter.1, " @pr");
        named(&mut enter.1, Named::Enter);
        assert_eq!(line(&enter.1, 2), "- [ ] Mail the deck @pr");
        assert_eq!(line(&enter.1, 3), "- [ ] ");
        named(&mut app, Named::Tab);
        assert_eq!(line(&app, 2), "- [ ] Mail the deck @Priya.K");
        assert_eq!(
            app.editor.cursor().position,
            Position {
                line: 2,
                column: 28
            }
        );
        assert!(app.editor.popover().is_none());
        // One undo step.
        assert!(app.editor.undo());
        assert_eq!(line(&app, 2), "- [ ] Mail the deck @pr");
    }

    #[test]
    fn tab_writes_a_date_or_a_priority_and_takes_the_word_out_in_one_step() {
        let (_directory, mut app) = app("advanced");
        type_text(&mut app, " to");
        named(&mut app, Named::Tab);
        assert_eq!(line(&app, 2), "- [ ] Mail the deck 📅 2026-11-19");
        // The caret at the end of the line, ready for Enter.
        let end = line(&app, 2).len();
        assert_eq!(
            app.editor.cursor().position,
            Position {
                line: 2,
                column: end
            }
        );
        assert!(app.editor.popover().is_none());
        assert!(app.editor.undo());
        assert_eq!(line(&app, 2), "- [ ] Mail the deck to");
        assert!(app.editor.redo());
        // Up or Down then Enter: the row picked.
        type_text(&mut app, " lo");
        named(&mut app, Named::ArrowDown);
        named(&mut app, Named::Enter);
        assert_eq!(line(&app, 2), "- [ ] Mail the deck ⏬ 📅 2026-11-19");
        assert_eq!(
            app.editor.text().matches('\n').count(),
            LIST.matches('\n').count()
        );
        // Only what the line lacks: no second due date, no second priority.
        type_text(&mut app, " to");
        assert!(app.editor.popover().is_none());
        for _ in 0..3 {
            named(&mut app, Named::Backspace);
        }
        type_text(&mut app, " hi");
        assert!(app.editor.popover().is_none());
        // Saved to the session's draft as any edit is.
        let document = app.session.as_ref().unwrap().document().unwrap();
        assert_eq!(document.text, app.editor.text());
    }

    /// The Enter table: at the end of a task line, Enter after each word
    /// continues the list as it does with no suggestions, and the word
    /// stays.
    #[test]
    fn enter_after_an_ordinary_word_continues_the_list() {
        for word in [
            "to", "hi", "lo", "me", "fr", "sa", "today", "due", "@sa", "@sam",
        ] {
            let (_directory, mut app) = app("advanced");
            scan_now(&mut app);
            type_text(&mut app, &format!(" {word}"));
            assert!(
                app.editor.popover().is_some(),
                "{word}: something is offered"
            );
            named(&mut app, Named::Enter);
            let expected = LIST.replacen(
                "- [ ] Mail the deck\n",
                &format!("- [ ] Mail the deck {word}\n- [ ] \n"),
                1,
            );
            assert_eq!(app.editor.text(), expected, "{word}");
            assert!(app.editor.popover().is_none(), "{word}");
            assert_eq!(
                app.editor.cursor().position,
                Position { line: 3, column: 6 }
            );
        }
        // Prose that offers nothing on its way.
        let (_directory, mut app) = app("advanced");
        type_text(&mut app, " with sam to discuss");
        named(&mut app, Named::Enter);
        assert_eq!(
            line(&app, 2) + "|" + &line(&app, 3),
            "- [ ] Mail the deck with sam to discuss|- [ ] "
        );
    }

    #[test]
    fn du_and_enter_open_the_calendar_which_takes_the_word_with_its_date() {
        let (_directory, mut app) = app("advanced");
        type_text(&mut app, " du");
        named(&mut app, Named::Enter);
        let open = app.calendar.as_ref().expect("the calendar opens");
        assert_eq!(open.line(), 2);
        assert_eq!(
            open.anchor(),
            &wren_editor::Anchor::Range {
                line: 2,
                range: 20..22
            }
        );
        // The word stays while it is open, and when it is cancelled.
        assert_eq!(line(&app, 2), "- [ ] Mail the deck du");
        let _ = app.update(Message::Calendar(CalendarEvent::Field(FieldEvent::Escape)));
        assert!(app.calendar.is_none());
        assert_eq!(line(&app, 2), "- [ ] Mail the deck du");
        // Again, and a date picked: the word goes, in one step.
        named(&mut app, Named::Backspace);
        type_text(&mut app, "u");
        named(&mut app, Named::Enter);
        assert!(app.calendar.is_some());
        let _ = app.update(Message::Calendar(CalendarEvent::Field(FieldEvent::Typed(
            "fri".into(),
        ))));
        let _ = app.update(Message::Calendar(CalendarEvent::Field(FieldEvent::Enter)));
        assert!(app.calendar.is_none());
        assert_eq!(line(&app, 2), "- [ ] Mail the deck 📅 2026-11-20");
        let end = line(&app, 2).len();
        assert_eq!(
            app.editor.cursor().position,
            Position {
                line: 2,
                column: end
            }
        );
        assert!(app.editor.undo());
        assert_eq!(line(&app, 2), "- [ ] Mail the deck du");
    }

    #[test]
    fn escape_hides_them_for_the_line() {
        let (_directory, mut app) = app("advanced");
        type_text(&mut app, " to");
        assert!(app.editor.popover().is_some());
        let _ = app.update(Message::Escape);
        assert!(app.editor.popover().is_none());
        type_text(&mut app, " hi");
        assert!(app.editor.popover().is_none());
        assert_eq!(line(&app, 2), "- [ ] Mail the deck to hi");
        // On another line they come back.
        named(&mut app, Named::Enter);
        type_text(&mut app, "Call bob hi");
        assert_eq!(labels(&app).len(), 2);
    }

    #[test]
    fn only_on_a_task_line_of_the_note() {
        let (_directory, mut app) = app("advanced");
        // A line that reads as a task alone, inside a fenced block.
        caret(&mut app, 5, "- [ ] fenced".len());
        type_text(&mut app, " to");
        assert!(app.editor.popover().is_none());
        // The heading.
        caret(&mut app, 0, "# Tasks".len());
        type_text(&mut app, " to");
        assert!(app.editor.popover().is_none());
    }

    #[test]
    fn off_the_setting_the_classic_engine_or_tasks_nothing_is_offered() {
        for (engine, settings) in [
            ("advanced", "tasks = on\ntasks-suggest = off\n"),
            ("advanced", "tasks = off\n"),
            ("advanced", ""),
            ("classic", "tasks = on\n"),
        ] {
            let (_directory, mut app) = app_with(engine, settings);
            type_text(&mut app, " to");
            assert!(app.editor.popover().is_none(), "{engine} {settings:?}");
            named(&mut app, Named::Enter);
            assert_eq!(
                line(&app, 2) + "|" + &line(&app, 3),
                "- [ ] Mail the deck to|- [ ] ",
                "{engine} {settings:?}"
            );
            // Tab indents the item under the one before.
            type_text(&mut app, "x hi");
            named(&mut app, Named::Tab);
            assert_eq!(line(&app, 3), "  - [ ] x hi", "{engine} {settings:?}");
            assert!(app.calendar.is_none());
        }
    }

    #[test]
    fn the_setting_applies_at_once() {
        let (_directory, mut app) = app("advanced");
        app.preferences.settings.tasks_suggest = false;
        let _ = app.update(Message::Tick);
        type_text(&mut app, " to");
        assert!(app.editor.popover().is_none());
        app.preferences.settings.tasks_suggest = true;
        let _ = app.update(Message::Tick);
        type_text(&mut app, " hi");
        assert_eq!(labels(&app).len(), 2);
    }

    #[test]
    fn the_owners_follow_the_index_and_drop_another_workspaces() {
        let (directory, mut app) = app("advanced");
        let index = |app: &App| {
            let index = app.session.as_ref().unwrap().index();
            (index.identity(), index.revision())
        };
        let names = |app: &App| -> Vec<String> {
            app.suggestions
                .owners()
                .iter()
                .map(|owner| owner.name.clone())
                .collect()
        };
        // Read for the index as it is, with the caret on a task line.
        assert_eq!(app.owners.identity(), Some(index(&app).0));
        assert_eq!(app.owners.revision(), Some(index(&app).1));
        assert_eq!(names(&app).len(), 2);
        let generation = app.owners.generation();
        // Typing on the line names no one: the half-typed name is left
        // out, and the list, and so the rows offered, stay as they were.
        type_text(&mut app, " @ze");
        assert_eq!(app.owners.revision(), Some(index(&app).1));
        assert!(!names(&app).iter().any(|name| name.starts_with("ze")));
        assert_eq!(app.owners.generation(), generation);
        type_text(&mut app, "d");
        // Once the caret leaves the line, it counts.
        named(&mut app, Named::Enter);
        assert!(names(&app).contains(&"zed".to_owned()));
        assert!(app.owners.generation() > generation);
        // Renamed, and removed: read again from the notes as they are, at
        // once.
        let text = app.editor.text().replace("@zed", "@zoe");
        app.editor.replace_text(&text, true);
        let note = app.selected_note.unwrap();
        let _ = app.apply_core(Op::EditDraft {
            note,
            contents: text,
        });
        scan_now(&mut app);
        assert!(names(&app).contains(&"zoe".to_owned()));
        assert!(!names(&app).contains(&"zed".to_owned()));
        // A row of a list that has since changed chooses nothing.
        let old = SuggestRow::Owner {
            generation: short(app.owners.generation().wrapping_sub(1)),
            place: 0,
        };
        let at = app.editor.cursor().position.line;
        type_text(&mut app, "Ask @z");
        let range = app.editor.popover().unwrap().range();
        assert_eq!(labels(&app), ["@zoe"]);
        let _ = app.suggestion_row(&Choice {
            id: old.id(),
            mode: Mode::Suggest,
            line: at,
            range,
        });
        assert_eq!(line(&app, at), "- [ ] Ask @z");
        // Another workspace: the list goes at once, the caret on a task
        // line or not.
        let other = directory.path().join("other/Work/Notes");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("Plain.md"), "Plain\n").unwrap();
        let workspace = directory.path().join("other");
        app.session = Some(Session::open_with(&workspace, app.session_options()).unwrap());
        let _ = app.update(Message::Tick);
        assert!(names(&app).is_empty());
        assert_eq!(app.owners.identity(), None);
    }

    #[test]
    fn a_change_of_owners_refreshes_the_suggestions_open() {
        let (_directory, mut app) = app("advanced");
        type_text(&mut app, " @pr");
        assert_eq!(labels(&app), ["@Priya.K"]);
        // Another note renames the owner while they are open, as a change
        // on disk would: the list changes, and the suggestions follow.
        let sources: Vec<_> = app
            .session
            .as_ref()
            .unwrap()
            .index()
            .markdown_sources()
            .into_iter()
            .map(|(note, modified, text)| {
                (
                    note,
                    modified,
                    std::sync::Arc::from(text.replace("@Priya.K", "@Priyanka")),
                )
            })
            .collect();
        let skip = app
            .session
            .as_ref()
            .unwrap()
            .document()
            .map(|document| (document.note, 2));
        assert!(app.owners.update(99, 0, sources, skip, SystemTime::now()));
        app.offer_owners();
        assert_eq!(labels(&app), ["@Priyanka"]);
    }

    #[test]
    fn a_nested_task_is_one_and_a_code_span_is_none() {
        let (_directory, mut app) = app("advanced");
        let text = "- Parent\n    - [ ] Call\n- [ ] Run `echo  now`\n";
        app.editor.replace_text(text, false);
        let note = app.selected_note.unwrap();
        let _ = app.apply_core(Op::EditDraft {
            note,
            contents: text.to_owned(),
        });
        // A child task four spaces in, which its line alone reads as code.
        caret(&mut app, 1, "    - [ ] Call".len());
        type_text(&mut app, " fr");
        assert_eq!(labels(&app), ["Friday"]);
        named(&mut app, Named::Tab);
        assert_eq!(line(&app, 1), "    - [ ] Call 📅 2026-11-20");
        // `du` inside a code span offers nothing.
        caret(&mut app, 2, "- [ ] Run `echo ".len());
        type_text(&mut app, "du");
        assert!(app.editor.popover().is_none());
        assert_eq!(line(&app, 2), "- [ ] Run `echo du now`");
    }
}
