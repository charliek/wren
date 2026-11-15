//! Spell check's app side: the bundled dictionary, loaded off the UI thread
//! at launch, the user's `words.txt`, and the proofreader built from the two
//! that the editor is handed.

use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::Task;
use wren_editor::Proofreader;
use wren_spell::{Hunspell, Speller, WordList, WordSource};

use super::board::Surface;
use super::workspace::WarningKind;
use super::{App, Message};

/// One read of `words.txt`: its text, `None` when there is no such file, or
/// why it could not be read.
type Read = Result<Option<String>, String>;

/// What the launch load brings back: the dictionary and the first read of
/// `words.txt`.
#[derive(Clone)]
pub struct Loaded {
    dictionary: Result<Arc<Hunspell>, String>,
    words: Read,
}

impl fmt::Debug for Loaded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Loaded")
            .field("dictionary", &self.dictionary.as_ref().map(|_| "en_US"))
            .field("words", &self.words)
            .finish()
    }
}

impl Loaded {
    #[cfg(test)]
    pub(super) fn new(dictionary: Arc<Hunspell>, words: Option<&Path>) -> Self {
        Self {
            dictionary: Ok(dictionary),
            words: words.map_or(Ok(None), read),
        }
    }

    fn failed(reason: String) -> Self {
        Self {
            dictionary: Err(reason),
            words: Ok(None),
        }
    }
}

/// Load the bundled dictionary and read `words` on a thread of its own, so
/// the UI never waits for them.
pub(super) fn load(words: Option<PathBuf>) -> Task<Message> {
    Task::perform(threaded_load(words), Message::SpellLoaded)
}

async fn threaded_load(words: Option<PathBuf>) -> Loaded {
    // iced polls a task's future on the UI thread until it first waits, so
    // the work itself goes to a thread.
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("wren-spell".into())
        .spawn(move || {
            let _ = sender.send(Loaded {
                dictionary: Hunspell::en_us()
                    .map(Arc::new)
                    .map_err(|error| error.to_string()),
                words: words.as_deref().map_or(Ok(None), read),
            });
        });
    match spawned {
        Ok(_) => receiver
            .await
            .unwrap_or_else(|_| Loaded::failed("the dictionary loader stopped".into())),
        Err(error) => Loaded::failed(format!("could not start the dictionary loader: {error}")),
    }
}

fn read(path: &Path) -> Read {
    wren_core::settings::read_text(path).map_err(|error| error.to_string())
}

/// The dictionary, the user's word list and the proofreader made of them.
pub(super) struct Spelling {
    /// `words.txt`, or `None` with nowhere to keep settings.
    path: Option<PathBuf>,
    dictionary: Option<Arc<Hunspell>>,
    /// The last read of `words.txt`; `None` before the first.
    words: Option<Read>,
    list: Arc<WordList>,
    warnings: Vec<String>,
    /// Built from the dictionary and the list, once the dictionary is here.
    proofreader: Option<Arc<dyn Proofreader>>,
}

impl Spelling {
    pub(super) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            dictionary: None,
            words: None,
            list: Arc::default(),
            warnings: Vec::new(),
            proofreader: None,
        }
    }

    #[cfg(test)]
    pub(super) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Whether the dictionary has loaded.
    pub(super) fn ready(&self) -> bool {
        self.dictionary.is_some()
    }

    /// The proofreader, once the dictionary has loaded. The same `Arc`
    /// until the dictionary or the word list changes.
    pub(super) fn proofreader(&self) -> Option<&Arc<dyn Proofreader>> {
        self.proofreader.as_ref()
    }

    /// Take the launch load. A read of `words.txt` made since it started is
    /// newer than the load's, so it stays. Returns why the dictionary could
    /// not load, if it could not.
    pub(super) fn loaded(&mut self, loaded: Loaded) -> Result<(), String> {
        if self.words.is_none() {
            self.take_words(loaded.words);
        }
        self.dictionary = Some(loaded.dictionary?);
        self.rebuild();
        Ok(())
    }

    /// Read `words.txt` again, rebuilding the list only when its text
    /// changed. Returns whether it did.
    pub(super) fn reload_words(&mut self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let read = read(path);
        if self.words.as_ref() == Some(&read) {
            return false;
        }
        self.take_words(read);
        true
    }

    fn take_words(&mut self, read: Read) {
        let (list, warnings) = match &read {
            Ok(Some(text)) => WordList::parse(text),
            Ok(None) => (WordList::default(), Vec::new()),
            Err(error) => (
                WordList::default(),
                vec![format!("could not read the file: {error}")],
            ),
        };
        self.list = Arc::new(list);
        self.warnings = warnings;
        self.words = Some(read);
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.proofreader = self.dictionary.as_ref().map(|dictionary| {
            let speller = Speller::new(vec![
                Box::new(Shared(Arc::clone(dictionary))),
                Box::new(Shared(Arc::clone(&self.list))),
            ]);
            Arc::new(Checker(speller)) as Arc<dyn Proofreader>
        });
    }

    /// The word list's warnings, after its path, as a settings warning reads.
    pub(super) fn warning(&self) -> Option<String> {
        let path = self.path.as_ref()?;
        (!self.warnings.is_empty())
            .then(|| format!("{}: {}", path.display(), self.warnings.join("; ")))
    }
}

impl App {
    /// Hand the editor the proofreader only while the setting is on and a
    /// Markdown note is open for editing in it (§3.4); before the dictionary
    /// loads there is none to hand. A board shown as raw text is not
    /// Markdown, so it gets none, and the menu closes when the surface
    /// changes. The same proofreader again keeps an open menu.
    pub(super) fn sync_proofreader(&mut self) {
        self.editor.set_proofreader(self.wanted_proofreader());
    }

    fn wanted_proofreader(&self) -> Option<Arc<dyn Proofreader>> {
        let wanted = self.preferences.settings.spell_check
            && self.surface() == Surface::Text
            && self.ensure_editor_ready().is_ok();
        wanted
            .then(|| self.spelling.proofreader().cloned())
            .flatten()
    }

    pub(super) fn spell_loaded(&mut self, loaded: Loaded) {
        if let Err(error) = self.spelling.loaded(loaded) {
            self.banner = Some(format!("Spell check is unavailable: {error}"));
            self.banner_kind = WarningKind::General;
        }
        self.sync_proofreader();
        // The first read of `words.txt` may bring warnings of its own.
        if self.settings_warning_changed() {
            let _ = self.show_settings_warning();
        }
    }
}

/// A source shared between the spellers built over a launch: the dictionary
/// outlives every change of the word list.
struct Shared<T>(Arc<T>);

impl<T: WordSource> WordSource for Shared<T> {
    fn accepts(&self, word: &str) -> bool {
        self.0.accepts(word)
    }

    fn suggest(&self, word: &str, out: &mut Vec<String>) {
        self.0.suggest(word, out);
    }
}

/// The speller as the editor asks for it.
struct Checker(Speller);

impl Proofreader for Checker {
    fn marks(&self, line: &str, prose: &[Range<usize>], out: &mut Vec<Range<usize>>) {
        self.0.marks(line, prose, out);
    }

    fn suggestions(&self, line: &str, mark: Range<usize>, limit: usize) -> Vec<String> {
        line.get(mark)
            .map_or_else(Vec::new, |word| self.0.suggestions(word, limit))
    }
}

/// The bundled dictionary, loaded once for every test that needs it.
#[cfg(test)]
pub(super) fn dictionary() -> Arc<Hunspell> {
    static DICTIONARY: std::sync::OnceLock<Arc<Hunspell>> = std::sync::OnceLock::new();
    Arc::clone(DICTIONARY.get_or_init(|| Arc::new(Hunspell::en_us().unwrap())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(spelling: &Spelling, line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let whole = 0..line.len();
        spelling
            .proofreader()
            .unwrap()
            .marks(line, std::slice::from_ref(&whole), &mut out);
        out.into_iter().map(|mark| line[mark].to_string()).collect()
    }

    use iced::widget::text_editor::{Cursor, Position};
    use wren_core::ops::{Op, Session};
    use wren_editor::SpellEvent;

    use crate::app::columns::Column;
    use crate::app::commands::{Command, CommandId, Disposition};
    use crate::app::palette::Key;
    use crate::app::Options;

    /// An app with one note open, `settings` in its `config.conf`, and
    /// `words.txt` beside it holding `words` when that is `Some`.
    fn spell_app(settings: &str, note: &str, words: Option<&str>) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let workspace = directory.path().join("workspace");
        std::fs::create_dir_all(workspace.join("Alpha/Inbox")).unwrap();
        std::fs::write(workspace.join("Alpha/Inbox/First.md"), note).unwrap();
        let config = directory.path().join("config/config.conf");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, settings).unwrap();
        if let Some(words) = words {
            std::fs::write(config.with_file_name("words.txt"), words).unwrap();
        }
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.workspace_mode = true;
        app.session = Some(Session::open(&workspace).unwrap());
        app.sync_editor(true);
        let first = app.model().unwrap().notes[0].id;
        let _ = app.process_core(Op::OpenNote { note: Some(first) });
        (directory, app)
    }

    fn load(app: &mut App) {
        let loaded = Loaded::new(dictionary(), app.spelling.path());
        let _ = app.update(Message::SpellLoaded(loaded));
    }

    fn caret(app: &mut App, line: usize, column: usize) -> Cursor {
        let cursor = Cursor {
            position: Position { line, column },
            selection: None,
        };
        app.editor.move_cursor(cursor);
        cursor
    }

    fn open(app: &mut App) -> Option<String> {
        let _ = app.update(Message::Editor(wren_editor::Message::Spell(
            SpellEvent::OpenAtCaret,
        )));
        app.editor.spell_menu().map(|menu| menu.word().to_string())
    }

    const ADVANCED: &str = "editor-engine = advanced\n";

    #[test]
    fn an_accepted_suggestion_reaches_the_session_draft_as_one_edit() {
        let (_directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", None);
        load(&mut app);
        app.column = Column::Notes;
        let cursor = caret(&mut app, 0, 4);
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.editor.cursor(), cursor);
        assert_eq!(app.editor.spell_menu().unwrap().items()[0], "misspelled");
        let revision = app.edit_revision;
        let _ = app.update(Message::Editor(wren_editor::Message::Spell(
            SpellEvent::Accept(0),
        )));
        assert!(app.editor.spell_menu().is_none());
        assert_eq!(app.editor.text(), "A misspelled word\n");
        let document = app.session.as_ref().unwrap().document().unwrap();
        assert_eq!(document.text, "A misspelled word\n");
        assert!(document.dirty());
        assert_eq!(app.edit_revision, revision + 1);
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            "A mispelled word\n"
        );
    }

    #[test]
    fn the_proofreader_is_withheld_until_loaded_with_the_setting_off_and_without_a_note() {
        let (directory, mut app) = spell_app(
            "editor-engine = advanced\nspell-check = off\n",
            "A mispelled word\n",
            None,
        );
        let _ = caret(&mut app, 0, 4);
        assert!(app.wanted_proofreader().is_none());
        assert_eq!(open(&mut app), None);
        load(&mut app);
        assert!(app.spelling.ready());
        assert!(app.wanted_proofreader().is_none());
        assert_eq!(open(&mut app), None);

        let config = directory.path().join("config/config.conf");
        std::fs::write(&config, ADVANCED).unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(app.preferences.settings.spell_check);
        assert!(app.wanted_proofreader().is_some());
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
        // Turning it off takes the proofreader, and the open menu with it.
        std::fs::write(&config, "editor-engine = advanced\nspell-check = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(app.editor.spell_menu().is_none());

        std::fs::write(&config, ADVANCED).unwrap();
        let _ = app.reload_settings(false);
        let _ = app.process_core(Op::OpenNote { note: None });
        let _ = app.update(Message::Tick);
        assert!(app.ensure_editor_ready().is_err());
        assert!(app.wanted_proofreader().is_none());
    }

    #[test]
    fn the_classic_engine_opens_no_menu_whatever_the_host_hands_it() {
        let (_directory, mut app) = spell_app("", "A mispelled word\n", None);
        load(&mut app);
        assert!(app.wanted_proofreader().is_some());
        let _ = caret(&mut app, 0, 4);
        assert_eq!(open(&mut app), None);
    }

    #[test]
    fn a_right_click_on_a_word_gives_the_editor_the_keyboard_and_a_close_does_not() {
        let (_directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", None);
        load(&mut app);
        app.column = Column::Notes;
        let close = app.edit(wren_editor::Message::Spell(SpellEvent::Close));
        assert_eq!(close.units(), 0);
        assert_eq!(app.column, Column::Notes);
        let task = app.edit(wren_editor::Message::Spell(SpellEvent::OpenAt(
            iced::Point::ORIGIN,
        )));
        assert_eq!(app.column, Column::Editor);
        assert!(
            task.units() > 0,
            "the editor is focused, unfocusing the rest"
        );
    }

    #[test]
    fn the_word_list_reloads_with_the_settings_and_its_warnings_join_theirs() {
        let (directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", Some("two words\n"));
        let words = directory.path().join("config/words.txt");
        assert_eq!(app.banner, None);
        load(&mut app);
        let warning = format!("{}: line 1: not one word", words.display());
        assert_eq!(app.banner.as_deref(), Some(warning.as_str()));
        let _ = caret(&mut app, 0, 4);
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));

        // On focus: the list changed, so the menu closes and the word is
        // now correct; the banner retracts.
        std::fs::write(&words, "mispelled\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(app.banner, None);
        assert!(app.editor.spell_menu().is_none());
        assert_eq!(open(&mut app), None);

        // A settings warning and a list warning show together, and the
        // list's survives a reload of an unchanged list.
        std::fs::write(&words, "mispelled\nnot one\n").unwrap();
        let config = directory.path().join("config/config.conf");
        std::fs::write(&config, "editor-engine = advanced\nmystery = 1\n").unwrap();
        let both = app.reload_settings(false).unwrap();
        assert!(both.contains("mystery"), "{both}");
        assert!(
            both.ends_with(&format!("{}: line 2: not one word", words.display())),
            "{both}"
        );
        assert!(app
            .reload_settings(true)
            .unwrap()
            .ends_with("line 2: not one word"));
        assert_eq!(app.banner.as_deref(), Some(both.as_str()));
    }

    fn run(app: &mut App, id: CommandId) -> Disposition {
        let _ = app.update(Message::Command(Command::Run(id)));
        app.last_processed.as_ref().unwrap().disposition
    }

    fn palette_row(app: &App, id: CommandId) -> (bool, Option<&'static str>) {
        let row = app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .find(|row| row.key == Key::Command(id))
            .unwrap();
        (row.enabled, app.command_state(id))
    }

    #[test]
    fn the_suggest_command_needs_the_advanced_engine_the_setting_the_dictionary_and_a_note() {
        let (directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", None);
        let config = directory.path().join("config/config.conf");
        assert!(!app.command_enabled(CommandId::SpellSuggest));
        load(&mut app);
        assert!(app.command_enabled(CommandId::SpellSuggest));
        for id in [CommandId::ToggleSpellCheck, CommandId::OpenWords] {
            assert!(app.command_enabled(id));
        }
        std::fs::write(&config, "editor-engine = advanced\nspell-check = off\n").unwrap();
        let _ = app.reload_settings(false);
        assert!(!app.command_enabled(CommandId::SpellSuggest));
        std::fs::write(&config, "").unwrap();
        let _ = app.reload_settings(false);
        assert!(!app.command_enabled(CommandId::SpellSuggest));
        std::fs::write(&config, ADVANCED).unwrap();
        let _ = app.reload_settings(false);
        assert!(app.command_enabled(CommandId::SpellSuggest));
        let _ = app.process_core(Op::OpenNote { note: None });
        assert!(!app.command_enabled(CommandId::SpellSuggest));
        assert_eq!(run(&mut app, CommandId::SpellSuggest), Disposition::Blocked);
        for id in [CommandId::ToggleSpellCheck, CommandId::OpenWords] {
            assert!(app.command_enabled(id));
        }
    }

    #[test]
    fn suggest_from_the_palette_closes_it_focuses_the_editor_and_then_opens_the_menu() {
        let (_directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", None);
        load(&mut app);
        let cursor = caret(&mut app, 0, 11);
        app.column = Column::Notes;
        let _ = app.update(Message::Command(Command::Run(CommandId::CommandPalette)));
        assert_eq!(palette_row(&app, CommandId::SpellSuggest), (true, None));
        let outcome = app.dispatch(Command::Run(CommandId::SpellSuggest));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert!(app.palette.is_none());
        assert_eq!(app.column, Column::Editor);
        // The focus operation comes first; the menu opens on the message
        // chained after it.
        assert!(outcome.task.units() >= 2);
        assert!(app.editor.spell_menu().is_none());
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
        assert_eq!(app.editor.cursor(), cursor);

        // With no misspelled word at the caret it does nothing.
        let _ = app.update(Message::Escape);
        let _ = caret(&mut app, 0, 0);
        assert_eq!(
            run(&mut app, CommandId::SpellSuggest),
            Disposition::Completed
        );
        assert_eq!(open(&mut app), None);
        assert_eq!(app.editor.text(), "A mispelled word\n");
    }

    #[test]
    fn the_toggle_flips_saves_and_applies_the_setting_and_its_row_shows_it() {
        let (directory, mut app) = spell_app(
            "# mine\neditor-engine = advanced\n",
            "A mispelled word\n",
            None,
        );
        let config = directory.path().join("config/config.conf");
        load(&mut app);
        let _ = caret(&mut app, 0, 4);
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
        let _ = app.update(Message::Command(Command::Run(CommandId::CommandPalette)));
        assert_eq!(
            palette_row(&app, CommandId::ToggleSpellCheck),
            (true, Some("on"))
        );
        assert_eq!(
            run(&mut app, CommandId::ToggleSpellCheck),
            Disposition::Completed
        );
        assert!(app.palette.is_none());
        assert!(app.editor.spell_menu().is_none());
        assert!(app.wanted_proofreader().is_none());
        assert_eq!(app.command_state(CommandId::ToggleSpellCheck), Some("off"));
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "# mine\neditor-engine = advanced\nspell-check = off\n"
        );
        assert_eq!(open(&mut app), None);
        let _ = app.update(Message::Command(Command::Run(CommandId::CommandPalette)));
        assert_eq!(
            palette_row(&app, CommandId::ToggleSpellCheck),
            (true, Some("off"))
        );
        assert_eq!(
            run(&mut app, CommandId::ToggleSpellCheck),
            Disposition::Completed
        );
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "# mine\neditor-engine = advanced\nspell-check = on\n"
        );
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));

        // On the classic engine it still flips and saves.
        let _ = app.update(Message::Escape);
        assert_eq!(
            run(&mut app, CommandId::SwitchEngine),
            Disposition::Completed
        );
        assert_eq!(
            run(&mut app, CommandId::ToggleSpellCheck),
            Disposition::Completed
        );
        assert!(!app.preferences.settings.spell_check);
        assert!(std::fs::read_to_string(&config)
            .unwrap()
            .ends_with("spell-check = off\n"));
    }

    #[test]
    fn the_toggle_applies_with_nowhere_to_save_and_the_word_list_has_nowhere_to_open() {
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, None);
        assert_eq!(
            run(&mut app, CommandId::ToggleSpellCheck),
            Disposition::Completed
        );
        assert!(!app.preferences.settings.spell_check);
        assert_eq!(run(&mut app, CommandId::OpenWords), Disposition::Blocked);
        let blocked = app.last_processed.as_ref().unwrap().warning.clone();
        assert_eq!(run(&mut app, CommandId::OpenSettings), Disposition::Blocked);
        assert_eq!(app.last_processed.as_ref().unwrap().warning, blocked);
    }

    #[test]
    fn a_word_list_warning_survives_the_next_saved_toggle() {
        let (directory, mut app) = spell_app(ADVANCED, "text\n", Some("not one\n"));
        load(&mut app);
        let words = directory.path().join("config/words.txt");
        let warning = format!("{}: line 1: not one word", words.display());
        assert_eq!(
            run(&mut app, CommandId::ToggleSpellCheck),
            Disposition::CommittedWarning
        );
        assert_eq!(
            app.last_processed.as_ref().unwrap().warning.as_deref(),
            Some(warning.as_str())
        );
        assert_eq!(app.banner.as_deref(), Some(warning.as_str()));
    }

    #[test]
    fn the_suggest_key_routes_to_the_editor_with_or_without_markdown_assist() {
        use iced::keyboard::{Key as Pressed, Modifiers};
        use wren_core::keybind::Platform;
        use wren_editor::KeyRoute;

        let (_directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", None);
        let keymap = app.preferences.keymap.clone();
        let trigger = keymap.accel(CommandId::SpellSuggest.key()).unwrap().clone();
        assert_eq!(trigger.key, ".");
        for assist in [true, false] {
            assert_eq!(
                super::super::preferences::key_route(&keymap, &trigger, assist),
                KeyRoute::Suggest
            );
        }
        let modifiers = match Platform::current() {
            Platform::Mac => Modifiers::LOGO,
            Platform::Linux => Modifiers::ALT,
        };
        let press = Pressed::Character(".".into());
        let editor = |app: &App| {
            app.editor
                .key_message(press.clone(), modifiers)
                .into_iter()
                .map(|message| format!("{message:?}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(editor(&app), ["Spell(OpenAtCaret)"]);
        let _ = app.update(Message::Command(Command::Run(
            CommandId::ToggleMarkdownAssist,
        )));
        assert!(!app.editor.markdown_assist());
        assert_eq!(editor(&app), ["Spell(OpenAtCaret)"]);

        // The app leaves the key to the editor, so it never acts twice or
        // outside the editor.
        load(&mut app);
        let _ = caret(&mut app, 0, 4);
        let _ = app.update(crate::app::preferences::shortcut(CommandId::SpellSuggest));
        assert!(app.editor.spell_menu().is_none());
    }

    #[test]
    fn one_escape_closes_only_the_menu_and_the_next_closes_the_find_bar() {
        use crate::keys::{Input, Nav};

        let (_directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", None);
        load(&mut app);
        assert_eq!(run(&mut app, CommandId::Find), Disposition::Completed);
        assert!(app.find_open);
        for escape in [Nav::Escape, Nav::FieldEscape] {
            let _ = caret(&mut app, 0, 4);
            assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
            let task = app.update(Message::Key(Input::Nav(escape)));
            assert!(app.editor.spell_menu().is_none());
            assert!(app.find_open, "{escape:?}");
            assert_eq!(app.column, Column::Editor);
            // No focus change: the editor keeps the keyboard.
            assert_eq!(task.units(), 0, "{escape:?}");
        }
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(!app.find_open);

        // The Escape message itself closes the menu before anything else.
        let _ = app.update(Message::Command(Command::Run(CommandId::Find)));
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
        let _ = app.update(Message::Escape);
        assert!(app.editor.spell_menu().is_none());
        assert!(app.find_open);
    }

    #[test]
    fn the_app_closes_the_menu_for_the_palette_a_dialog_the_theme_settings_and_window_focus() {
        let (directory, mut app) = spell_app(ADVANCED, "A mispelled word\n", None);
        let config = directory.path().join("config/config.conf");
        load(&mut app);
        let _ = caret(&mut app, 0, 4);
        let reopen = |app: &mut App| assert_eq!(open(app).as_deref(), Some("mispelled"));
        let closes = |app: &mut App, message: Message| {
            let _ = app.update(message);
            assert!(app.editor.spell_menu().is_none());
        };

        reopen(&mut app);
        closes(
            &mut app,
            Message::PaletteOpen(crate::app::palette::Mode::Commands),
        );
        closes(&mut app, Message::PaletteClose);
        reopen(&mut app);
        closes(
            &mut app,
            Message::Command(Command::Run(CommandId::RenameNote)),
        );
        assert!(app.dialog.is_some());
        closes(&mut app, Message::CancelDialog);
        reopen(&mut app);
        let other = if app.definition == wren_core::theme::DARK {
            wren_core::theme::LIGHT
        } else {
            wren_core::theme::DARK
        };
        closes(&mut app, Message::ChooseTheme(Some(other)));
        reopen(&mut app);
        // The same theme again is no change.
        let _ = app.update(Message::ChooseTheme(Some(other)));
        assert!(app.editor.spell_menu().is_some());
        std::fs::write(&config, "editor-engine = advanced\nline-numbers = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(app.editor.spell_menu().is_none());
        reopen(&mut app);
        // A reload that finds nothing changed keeps it.
        assert_eq!(app.reload_settings(false), None);
        assert!(app.editor.spell_menu().is_some());
        let window = iced::window::Id::unique();
        closes(
            &mut app,
            Message::Window(window, iced::window::Event::Unfocused),
        );
        reopen(&mut app);
        closes(
            &mut app,
            Message::Command(Command::Run(CommandId::SwitchEngine)),
        );
        assert_eq!(
            run(&mut app, CommandId::SwitchEngine),
            Disposition::Completed
        );
        reopen(&mut app);
        closes(&mut app, Message::Core(Op::OpenNote { note: None }));
    }

    #[test]
    fn the_proofreader_comes_with_the_dictionary_and_follows_the_word_list() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("words.txt");
        std::fs::write(&path, "catppuccin\n").unwrap();
        let mut spelling = Spelling::new(Some(path.clone()));
        assert!(!spelling.ready());
        assert!(spelling.proofreader().is_none());
        spelling
            .loaded(Loaded::new(dictionary(), Some(&path)))
            .unwrap();
        assert!(spelling.ready());
        let line = "Catppuccin themes apear";
        assert_eq!(marks(&spelling, line), ["apear"]);
        let checker = spelling.proofreader().unwrap();
        assert_eq!(
            checker
                .suggestions(line, 18..23, 5)
                .first()
                .map(String::as_str),
            Some("pear")
        );
        assert!(checker.suggestions(line, 18..40, 5).is_empty());

        // An unchanged file keeps the same proofreader; a changed one
        // rebuilds it.
        let before = Arc::clone(checker);
        assert!(!spelling.reload_words());
        assert!(Arc::ptr_eq(&before, spelling.proofreader().unwrap()));
        std::fs::write(&path, "catppuccin\napear\n").unwrap();
        assert!(spelling.reload_words());
        assert!(!Arc::ptr_eq(&before, spelling.proofreader().unwrap()));
        assert!(marks(&spelling, line).is_empty());
        std::fs::remove_file(&path).unwrap();
        assert!(spelling.reload_words());
        assert_eq!(marks(&spelling, line), ["Catppuccin", "apear"]);
        assert_eq!(spelling.warning(), None);
    }

    #[test]
    fn word_list_warnings_name_the_file_and_an_unreadable_one_leaves_the_dictionary() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("words.txt");
        std::fs::write(&path, "# mine\ntwo words\nwren\n").unwrap();
        let mut spelling = Spelling::new(Some(path.clone()));
        assert!(spelling.reload_words());
        assert_eq!(
            spelling.warning(),
            Some(format!("{}: line 2: not one word", path.display()))
        );
        spelling
            .loaded(Loaded::new(dictionary(), Some(&path)))
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(spelling.reload_words());
        let warning = spelling.warning().unwrap();
        assert!(
            warning.starts_with(&format!("{}: could not read the file", path.display())),
            "{warning}"
        );
        assert_eq!(marks(&spelling, "a misteak"), ["misteak"]);
        assert!(!spelling.reload_words());
    }

    #[test]
    fn a_read_made_while_the_dictionary_loads_is_kept() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("words.txt");
        let stale = Loaded::new(dictionary(), Some(&path));
        std::fs::write(&path, "apear\n").unwrap();
        let mut spelling = Spelling::new(Some(path));
        assert!(spelling.reload_words());
        spelling.loaded(stale).unwrap();
        assert!(marks(&spelling, "apear").is_empty());
    }

    #[test]
    fn a_failed_load_reports_why_and_leaves_no_proofreader() {
        let mut spelling = Spelling::new(None);
        assert_eq!(
            spelling.loaded(Loaded::failed("broken".into())),
            Err("broken".into())
        );
        assert!(!spelling.ready());
        assert!(spelling.proofreader().is_none());
        assert!(!spelling.reload_words());
        assert_eq!(spelling.warning(), None);
    }

    #[test]
    fn the_launch_load_brings_the_dictionary_and_the_first_read() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("words.txt");
        std::fs::write(&path, "wren\n").unwrap();
        let loaded = iced::futures::executor::block_on(threaded_load(Some(path)));
        assert!(loaded.dictionary.is_ok());
        assert_eq!(loaded.words, Ok(Some("wren\n".into())));
    }

    /// A workspace of a Markdown note, a board and a board that does not
    /// parse, with boards and spell check on and the dictionary loaded.
    /// The Markdown note is open.
    fn board_app() -> (tempfile::TempDir, App) {
        use crate::app::board::tests::{conflicted, open_markdown, FLOW};

        let directory = crate::test_support::tempdir();
        let workspace = directory.path().join("workspace");
        let group = workspace.join("Alpha/Inbox");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("First.md"), "A mispelled word\n").unwrap();
        std::fs::write(group.join("Flow.wren-board"), FLOW).unwrap();
        let broken = format!("# A mispelled word\n{}", conflicted());
        std::fs::write(group.join("Broken.wren-board"), broken).unwrap();
        let config = directory.path().join("config/config.conf");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, ADVANCED).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.preferences.boards = true;
        app.workspace_mode = true;
        app.session = Some(Session::open_with(&workspace, app.session_options()).unwrap());
        app.sync_editor(true);
        load(&mut app);
        let _ = open_markdown(&mut app, "First");
        (directory, app)
    }

    #[test]
    fn spell_check_is_for_markdown_text_and_never_a_board_or_its_raw_text() {
        use crate::app::board::tests::{open_board, open_markdown};
        use crate::app::board::Surface;

        let (_directory, mut app) = board_app();
        assert_eq!(app.surface(), Surface::Text);
        assert!(app.command_enabled(CommandId::SpellSuggest));
        let _ = caret(&mut app, 0, 4);
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));

        // Opening a board withdraws the proofreader, which closes the menu,
        // without waiting for a message to pass through `update`.
        let _ = open_board(&mut app, "Flow");
        assert_eq!(app.surface(), Surface::Board);
        assert!(app.editor.spell_menu().is_none());
        assert!(app.wanted_proofreader().is_none());
        assert!(!app.command_enabled(CommandId::SpellSuggest));

        // A board that does not parse is edited as text, but it is not
        // Markdown: no proofreader, no menu, and the command is disabled.
        let _ = open_board(&mut app, "Broken");
        assert_eq!(app.surface(), Surface::Raw);
        assert!(app.ensure_editor_ready().is_ok());
        assert!(app.editor.text().starts_with("# A mispelled word\n"));
        assert!(app.wanted_proofreader().is_none());
        assert!(!app.command_enabled(CommandId::SpellSuggest));
        let _ = caret(&mut app, 0, 6);
        assert_eq!(open(&mut app), None);
        assert_eq!(run(&mut app, CommandId::SpellSuggest), Disposition::Blocked);
        assert!(app.editor.spell_menu().is_none());

        // Back on Markdown the proofreader returns.
        let _ = open_markdown(&mut app, "First");
        assert_eq!(app.surface(), Surface::Text);
        assert!(app.wanted_proofreader().is_some());
        assert!(app.command_enabled(CommandId::SpellSuggest));
        let _ = caret(&mut app, 0, 4);
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
    }

    #[test]
    fn one_escape_on_a_text_surface_beside_boards_closes_only_the_menu() {
        use crate::app::board::Surface;
        use crate::keys::{Input, Nav};

        let (_directory, mut app) = board_app();
        assert_eq!(run(&mut app, CommandId::Find), Disposition::Completed);
        let _ = caret(&mut app, 0, 4);
        assert_eq!(open(&mut app).as_deref(), Some("mispelled"));
        let cursor = app.editor.cursor();
        let task = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(app.editor.spell_menu().is_none());
        assert_eq!(app.surface(), Surface::Text);
        assert!(app.find_open);
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.editor.cursor(), cursor);
        assert_eq!(app.editor.text(), "A mispelled word\n");
        assert_eq!(task.units(), 0);
        // The next Escape is the rest of the ladder: it closes the find bar.
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(!app.find_open);
    }
}
