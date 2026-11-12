//! User settings (`config.conf`): theme, keybindings, editor options.

use std::path::{Path, PathBuf};

use iced::theme::Mode as SystemMode;
use iced::{Font, Task};
use wren_core::keybind::{self, Accel, Keymap, Platform};
use wren_core::settings::{self, EditorEngine, Settings, ThemeChoice};
use wren_core::theme::{self as definitions, Appearance, Theme};
use wren_editor::{BackendKind, KeyPolicy, KeyRoute};

use super::board::Surface;
use super::commands::{CommandId, Disposition, Gateway, Scope};
use super::palette::{Key, Mode};
use super::{workspace, App, Message};
use crate::fonts;
use crate::keys::{Input, Nav};

/// The theme or editor font picker row being previewed. `theme` is
/// `Some(None)` while previewing "System".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Preview {
    pub theme: Option<Option<&'static str>>,
    pub font: Option<&'static str>,
}

/// Settings plus the derived keymap and the file they came from.
pub struct Preferences {
    pub settings: Settings,
    pub keymap: Keymap,
    pub path: Option<PathBuf>,
    /// The `boards` setting as it was at launch, which is what the app runs
    /// with: a Session lists boards or not for its whole life, so a later
    /// value waits for a restart.
    pub boards: bool,
    /// The bytes last loaded, so focus-triggered reloads skip unchanged files.
    loaded: Option<String>,
    /// The warning last shown for these settings, so a clean reload can
    /// retract it.
    shown_warning: Option<String>,
}

/// The editor engine a setting names.
pub(super) fn backend_kind(engine: EditorEngine) -> BackendKind {
    match engine {
        EditorEngine::Classic => BackendKind::Iced,
        EditorEngine::Advanced => BackendKind::Owned,
    }
}

/// The editor's say over its keys, from the keymap and key reading the app's
/// own shortcuts use: a host command's key is the app's, and an editor
/// command's key is the editor's intent.
fn key_policy(keymap: &Keymap, markdown_assist: bool) -> KeyPolicy {
    let keymap = keymap.clone();
    std::sync::Arc::new(move |key, physical, modifiers| {
        crate::keys::shortcut(key, physical, modifiers).map_or(KeyRoute::Pass, |trigger| {
            key_route(&keymap, &trigger, markdown_assist)
        })
    })
}

/// Where the editor sends a press of `trigger`. Without Markdown assist the
/// editor commands are off, so their keys keep the editor's own meaning:
/// Ctrl+Enter breaks the line and Alt+↑ moves up.
pub(super) fn key_route(keymap: &Keymap, trigger: &Accel, markdown_assist: bool) -> KeyRoute {
    match keymap.action(trigger).and_then(CommandId::parse) {
        None => KeyRoute::Pass,
        Some(id) => match id.intent() {
            None => KeyRoute::Host,
            Some(intent) if markdown_assist => KeyRoute::Intent(intent),
            Some(_) => KeyRoute::Pass,
        },
    }
}

/// The theme for a picker row: `None` follows the system.
fn theme_for(id: Option<&str>, mode: SystemMode) -> Theme {
    let appearance = appearance(mode);
    let choice = id.map_or_else(ThemeChoice::system, |id| ThemeChoice::Fixed(id.into()));
    definitions::builtin(choice.resolve(appearance))
        .unwrap_or_else(|| definitions::for_appearance(appearance))
}

fn appearance(mode: SystemMode) -> Appearance {
    match mode {
        SystemMode::Light => Appearance::Light,
        SystemMode::Dark | SystemMode::None => Appearance::Dark,
    }
}

/// Read and parse the settings file; read errors become warnings.
fn read(path: Option<&Path>) -> (Option<String>, Settings) {
    let Some(path) = path else {
        return (None, Settings::default());
    };
    match settings::read_text(path) {
        Ok(text) => {
            let parsed = text
                .as_deref()
                .map_or_else(Settings::default, Settings::parse);
            (text, parsed)
        }
        Err(error) => (
            None,
            Settings {
                warnings: vec![format!("could not read the file: {error}")],
                ..Settings::default()
            },
        ),
    }
}

impl Preferences {
    /// Load from `path`; `None` means defaults with nowhere to persist.
    pub fn load(path: Option<PathBuf>) -> Self {
        let (loaded, settings) = read(path.as_deref());
        let mut preferences = Self {
            keymap: Keymap::default(),
            boards: settings.boards,
            settings,
            path,
            loaded,
            shown_warning: None,
        };
        preferences.rebuild_keymap();
        preferences
    }

    fn rebuild_keymap(&mut self) {
        let (keymap, warnings) = Keymap::build(
            keybind::default_bindings(Platform::current()),
            &self.settings.keybinds,
            |action| CommandId::parse(action).is_some(),
        );
        self.keymap = keymap;
        self.settings.warnings.extend(warnings);
    }

    /// Re-read the file. Returns `false` when nothing changed.
    fn reload(&mut self, force: bool) -> bool {
        if self.path.is_none() {
            return false;
        }
        let (text, settings) = read(self.path.as_deref());
        if !force && text == self.loaded {
            return false;
        }
        self.settings = settings;
        self.loaded = text;
        self.rebuild_keymap();
        true
    }

    /// The theme to show for the system `mode`, plus a warning when the
    /// configured theme id is unknown.
    pub fn theme(&self, mode: SystemMode) -> (Theme, Option<String>) {
        let appearance = appearance(mode);
        let id = self.settings.theme.resolve(appearance);
        match definitions::builtin(id) {
            Some(theme) => (theme, None),
            None => (
                definitions::for_appearance(appearance),
                Some(format!("Unknown theme {id:?}; using the default.")),
            ),
        }
    }

    /// The configured editor font, plus a warning when that family is not
    /// installed.
    pub fn editor_font(&self) -> (Font, Option<String>) {
        let family = self.settings.editor_font_family.as_deref();
        match fonts::editor_font(family) {
            Some(font) => (font, None),
            None => (
                fonts::EDITOR,
                Some(format!(
                    "Unknown editor font {:?}; using {}.",
                    family.unwrap_or_default(),
                    fonts::EDITOR_FAMILY
                )),
            ),
        }
    }

    /// What to say when the file asks for a `boards` value other than the
    /// one the app launched with.
    fn restart_notice(&self) -> Option<String> {
        (self.settings.boards != self.boards).then(|| "Restart Wren to apply `boards`.".to_owned())
    }

    fn warning(&self) -> Option<String> {
        let mut warnings = self.settings.warnings.clone();
        warnings.extend(self.editor_font().1);
        for mode in [SystemMode::Dark, SystemMode::Light] {
            if let Some(warning) = self.theme(mode).1 {
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
        }
        (!warnings.is_empty()).then(|| {
            let location = self
                .path
                .as_ref()
                .map_or_else(|| "settings".to_string(), |path| path.display().to_string());
            format!("{location}: {}", warnings.join("; "))
        })
    }
}

impl App {
    /// Where the editor's text wraps and sits: from the left at the
    /// setting's column, or centred in focus mode, which always wraps at a
    /// column.
    pub(super) fn editor_measure(&self) -> wren_editor::Measure {
        let column = self.preferences.settings.editor_wrap_column;
        wren_editor::Measure {
            wrap_column: if self.focus {
                Some(column.unwrap_or(settings::DEFAULT_WRAP_COLUMN))
            } else {
                column
            },
            centred: self.focus,
        }
    }

    /// The displayed shortcut for a command key, in platform notation.
    pub(super) fn shortcut_label(&self, action: &str) -> Option<String> {
        self.preferences.keymap.label(action, Platform::current())
    }

    /// Recompute the theme from the picker preview, the explicit override,
    /// settings, and system.
    pub(super) fn apply_theme(&mut self) {
        let theme = match self.preview.theme {
            Some(id) => theme_for(id, self.system_mode),
            None => self
                .theme_override
                .unwrap_or_else(|| self.preferences.theme(self.system_mode).0),
        };
        if theme != self.definition {
            self.definition = theme;
            self.toolkit_theme = crate::theme::from_definition(theme);
            let palette = self.board_palette();
            if let Some(board) = &mut self.board {
                board.set_palette(palette);
            }
        }
    }

    /// Re-read `config.conf` and apply it. Returns any settings warning.
    ///
    /// An engine switch keeps the editor widget's tree state, and with it
    /// keyboard focus, so nothing is refocused here.
    pub(super) fn reload_settings(&mut self, force: bool) -> Option<String> {
        #[cfg(target_os = "macos")]
        let keymap = self.preferences.keymap.clone();
        if !self.preferences.reload(force) {
            return None;
        }
        self.apply_preferences();
        #[cfg(target_os = "macos")]
        if self.window.is_some() && self.preferences.keymap != keymap {
            crate::macos_menu::install(&self.preferences.keymap);
        }
        let warning = self.preferences.warning();
        // The restart notice is shown like a warning and retracted like one,
        // but a setting that waits for a restart has not failed.
        let shown = match (&warning, self.preferences.restart_notice()) {
            (Some(warning), Some(notice)) => Some(format!("{warning} {notice}")),
            (warning, notice) => warning.clone().or(notice),
        };
        match &shown {
            Some(shown) => {
                self.banner = Some(shown.clone());
                self.banner_kind = workspace::WarningKind::General;
            }
            None if self.banner.is_some() && self.banner == self.preferences.shown_warning => {
                self.banner = None;
            }
            None => {}
        }
        self.preferences.shown_warning = shown;
        warning
    }

    /// Apply the settings to the theme and editor.
    pub(super) fn apply_preferences(&mut self) {
        self.apply_shown();
        let settings = &self.preferences.settings;
        self.editor.set_line_height(settings.editor_line_height);
        self.editor.set_font_size(settings.editor_font_size);
        self.editor.set_line_numbers(settings.line_numbers);
        let assist = self.markdown_assist();
        self.editor.set_markdown_assist(assist);
        self.editor
            .set_key_policy(key_policy(&self.preferences.keymap, assist));
        self.apply_editor_engine();
    }

    /// Whether the editor's Markdown keys act: as the setting says, and
    /// never in a board's raw text, which is not Markdown.
    fn markdown_assist(&self) -> bool {
        self.preferences.settings.markdown_assist && self.surface() != Surface::Raw
    }

    /// Bring the editor's Markdown keys in step with the surface.
    pub(super) fn apply_markdown_assist(&mut self) {
        let assist = self.markdown_assist();
        if assist != self.editor.markdown_assist() {
            self.editor.set_markdown_assist(assist);
            self.editor
                .set_key_policy(key_policy(&self.preferences.keymap, assist));
        }
    }

    /// Rebuild the editor on the configured engine from the session draft,
    /// or from the editor's own text when no note is open (the `--editor`
    /// spike). The cursor, find query, font, size, and gutter carry over;
    /// undo history does not. Behind a board the editor stays empty: the
    /// board's text is never loaded into it.
    fn apply_editor_engine(&mut self) {
        let kind = backend_kind(self.preferences.settings.editor_engine);
        if kind == self.editor.backend() {
            return;
        }
        let text = if self.surface() == Surface::Board {
            String::new()
        } else {
            self.session
                .as_ref()
                .and_then(|session| session.document())
                .map_or_else(|| self.editor.text(), |document| document.text.clone())
        };
        self.editor.set_backend(kind, &text);
    }

    /// Show the theme and editor font: the saved ones, or the picker row
    /// being previewed.
    fn apply_shown(&mut self) {
        self.apply_theme();
        let font = self
            .preview
            .font
            .map_or_else(|| self.preferences.editor_font().0, Font::with_name);
        self.editor.set_font(font);
    }

    /// Follow the theme or font picker's highlighted row. Moving onto a row
    /// other than the saved choice previews it; leaving the picker any way
    /// but choosing a row restores the saved choice. The outline previews
    /// headings the same way (`sync_outline`).
    pub(super) fn sync_preview(&mut self) -> Task<Message> {
        let highlighted = self.palette.as_ref().and_then(|palette| {
            let row = palette.results.get(palette.active?)?;
            (!row.current).then_some(&row.key)
        });
        let preview = match highlighted {
            Some(Key::Theme(id)) => Preview {
                theme: Some(*id),
                font: None,
            },
            Some(Key::Font(family)) => Preview {
                theme: None,
                font: Some(*family),
            },
            _ => Preview::default(),
        };
        if preview != self.preview {
            self.preview = preview;
            self.apply_shown();
        }
        self.sync_outline()
    }

    pub(super) fn startup_settings_warning(&mut self) -> Option<String> {
        let warning = self.preferences.warning();
        self.preferences.shown_warning = warning.clone();
        warning
    }

    /// Persist a theme choice from the picker; `None` follows the system.
    pub(super) fn choose_theme(&mut self, id: Option<&'static str>) -> Gateway {
        let choice = id.map_or_else(ThemeChoice::system, |id| ThemeChoice::Fixed(id.into()));
        self.preferences.settings.theme = choice.clone();
        self.theme_override = None;
        self.palette = None;
        self.preview = Preview::default();
        self.apply_shown();
        let warning = self.save_setting("theme", &choice.value());
        let task = self.column_focus();
        self.saved(warning, task)
    }

    /// Persist an editor font family from the picker.
    pub(super) fn choose_font(&mut self, family: &'static str) -> Gateway {
        self.preferences.settings.editor_font_family = Some(family.into());
        self.palette = None;
        self.preview = Preview::default();
        self.apply_shown();
        let warning = self.save_setting("editor-font-family", &format!("\"{family}\""));
        let task = self.column_focus();
        self.saved(warning, task)
    }

    /// Step the editor text size by `delta` points, or back to the default
    /// for `None`, and save it. At either limit this is a no-op.
    ///
    /// On a board the same commands zoom it instead, one stop up or down or
    /// back to 100 %: a look at the board, which changes no setting.
    pub(super) fn step_text_size(&mut self, delta: Option<f32>) -> Gateway {
        let current = self.preferences.settings.editor_font_size;
        let size = delta.map_or(settings::DEFAULT_FONT_SIZE, |delta| {
            (current + delta).clamp(settings::MIN_FONT_SIZE, settings::MAX_FONT_SIZE)
        });
        // Leaving the palette returns focus to the active column; a shortcut
        // leaves focus where it was (a filter field, say).
        let task = if self.palette.take().is_some() {
            self.column_focus()
        } else {
            Task::none()
        };
        if let Some(board) = &mut self.board {
            match delta {
                Some(delta) if delta > 0.0 => board.zoom_step(1),
                Some(delta) if delta < 0.0 => board.zoom_step(-1),
                Some(_) => {}
                None => board.zoom_reset(),
            }
            return self.finish_action(Disposition::Completed, None, task);
        }
        if size == current {
            return self.finish_action(Disposition::Completed, None, task);
        }
        self.preferences.settings.editor_font_size = size;
        self.editor.set_font_size(size);
        let warning = self.save_setting("editor-font-size", &size.to_string());
        self.saved(warning, task)
    }

    /// Flip `markdown-assist`, apply it, and save it.
    pub(super) fn toggle_markdown_assist(&mut self) -> Gateway {
        let enabled = !self.preferences.settings.markdown_assist;
        self.preferences.settings.markdown_assist = enabled;
        self.save_toggle("markdown-assist", if enabled { "on" } else { "off" })
    }

    /// Move the editor to the other engine, keeping its text and cursor
    /// (undo history is dropped), and save the choice.
    pub(super) fn toggle_editor_engine(&mut self) -> Gateway {
        let engine = match self.preferences.settings.editor_engine {
            EditorEngine::Classic => EditorEngine::Advanced,
            EditorEngine::Advanced => EditorEngine::Classic,
        };
        self.preferences.settings.editor_engine = engine;
        self.save_toggle("editor-engine", engine.value())
    }

    /// Apply a flipped setting before saving it, so it takes effect even
    /// with nowhere to save or when the save fails.
    fn save_toggle(&mut self, key: &str, value: &str) -> Gateway {
        self.apply_preferences();
        let task = if self.palette.take().is_some() {
            self.column_focus()
        } else {
            Task::none()
        };
        let warning = self.save_setting(key, value);
        self.saved(warning, task)
    }

    /// The current value a settings command flips, for its palette row.
    pub(super) fn command_state(&self, id: CommandId) -> Option<&'static str> {
        let settings = &self.preferences.settings;
        match id {
            CommandId::ToggleMarkdownAssist => Some(if settings.markdown_assist {
                "on"
            } else {
                "off"
            }),
            CommandId::SwitchEngine => Some(settings.editor_engine.value()),
            _ => None,
        }
    }

    /// Write one setting to `config.conf`, then re-read the merged file so
    /// edits made outside Wren since the last reload apply now rather than
    /// being marked as seen. Returns the warning to report, if any.
    fn save_setting(&mut self, key: &str, value: &str) -> Option<String> {
        let path = self.preferences.path.clone()?;
        match settings::write_value(&path, key, value) {
            Ok(()) => self.reload_settings(true),
            Err(error) => {
                let warning = format!("Could not save {key} to {}: {error}", path.display());
                self.banner = Some(warning.clone());
                self.banner_kind = workspace::WarningKind::General;
                Some(warning)
            }
        }
    }

    fn saved(&mut self, warning: Option<String>, task: Task<Message>) -> Gateway {
        let disposition = if warning.is_some() {
            Disposition::CommittedWarning
        } else {
            Disposition::Completed
        };
        self.finish_action(disposition, warning, task)
    }

    /// Open `config.conf` in the user's editor, creating it from the template.
    pub(super) fn open_settings(&mut self) -> Gateway {
        let Some(path) = self.preferences.path.clone() else {
            return self.blocked("No settings location is available (set HOME or WREN_CONFIG)");
        };
        if let Err(error) = settings::ensure_exists(&path) {
            return self.blocked(format!("Could not create {}: {error}", path.display()));
        }
        #[cfg(target_os = "macos")]
        let launched = std::process::Command::new("open")
            .arg("-t")
            .arg(&path)
            .spawn();
        #[cfg(not(target_os = "macos"))]
        let launched = std::process::Command::new("xdg-open").arg(&path).spawn();
        match launched {
            Ok(mut child) => {
                // Reap the launcher without blocking the UI.
                std::thread::spawn(move || child.wait());
                self.palette = None;
                self.finish_action(Disposition::Completed, None, Task::none())
            }
            Err(error) => self.blocked(format!("Could not open {}: {error}", path.display())),
        }
    }

    /// Run a command chosen from the native menu bar, with the same rules
    /// as its keyboard shortcut.
    #[cfg(target_os = "macos")]
    pub(super) fn menu_command(&mut self, key: &str) -> Task<Message> {
        if key == crate::macos_menu::CHECK_FOR_UPDATES {
            if let Err(error) = wren_sparkle::check_for_updates() {
                self.banner = Some(format!("Could not check for updates: {error}"));
                self.banner_kind = workspace::WarningKind::General;
            }
            crate::macos_menu::sync_updates();
            return Task::none();
        }
        CommandId::parse(key).map_or_else(Task::none, |id| self.run_shortcut(id))
    }

    /// Route a key press: overlays first, then the keymap.
    pub(super) fn key_input(&mut self, input: Input) -> Task<Message> {
        let accel = match input {
            Input::Nav(Nav::Escape) => return self.update(Message::Escape),
            // A filter box handed back the keyboard: stay in its column. The
            // palette, a dialog, and the find bar still close as with Esc.
            Input::Nav(Nav::FieldEscape) => {
                return if self.palette.is_some() || self.dialog.is_some() || self.find_open {
                    self.update(Message::Escape)
                } else {
                    self.column_focus()
                };
            }
            Input::Nav(nav) => {
                if self.palette.is_some() {
                    return match nav {
                        Nav::Up => self.palette_step(-1).task,
                        Nav::Down => self.palette_step(1).task,
                        Nav::Enter => self.activate_palette(None).task,
                        _ => Task::none(),
                    };
                }
                if self.dialog.is_some() || !self.column.is_browser() {
                    return Task::none();
                }
                return match nav {
                    Nav::Up => self.step_column(-1).task,
                    Nav::Down => self.step_column(1).task,
                    Nav::Enter | Nav::Right => self.column_right().task,
                    Nav::Left => self.column_left().task,
                    Nav::Escape | Nav::FieldEscape => Task::none(),
                };
            }
            Input::Accel(accel) => accel,
        };
        let Some(id) = self
            .preferences
            .keymap
            .action(&accel)
            .and_then(CommandId::parse)
        else {
            return Task::none();
        };
        // The focused editor's key policy runs these; the same press
        // reaches here too, and must not act twice or outside the editor.
        if id.scope() == Scope::Editor {
            return Task::none();
        }
        self.run_shortcut(id)
    }

    pub(super) fn run_shortcut(&mut self, id: CommandId) -> Task<Message> {
        // Quitting stays available everywhere. Otherwise an open dialog or
        // palette owns the keyboard: only the palette shortcut (to close it)
        // gets through.
        if id != CommandId::Quit {
            if self.dialog.is_some() {
                return Task::none();
            }
            if let Some(palette) = &self.palette {
                return match (id, palette.mode) {
                    (CommandId::CommandPalette, _) => self.close_palette().task,
                    // ⌘P again widens a section or group jump to every note.
                    (CommandId::Jump, Mode::JumpSection | Mode::JumpGroup) => {
                        self.open_palette(Mode::Jump).task
                    }
                    _ => Task::none(),
                };
            }
        }
        if id == CommandId::CommandPalette {
            return self.open_palette(Mode::Commands).task;
        }
        self.run_scoped(id).task
    }
}

/// Every bindable action name, for documentation and validation.
#[cfg(test)]
pub fn action_names() -> impl Iterator<Item = &'static str> {
    super::commands::DEFINITIONS
        .iter()
        .map(|definition| definition.key)
}

/// The message a default shortcut for `id` produces.
#[cfg(test)]
pub fn shortcut(id: CommandId) -> Message {
    let preferences = Preferences::load(None);
    let accel = preferences
        .keymap
        .accel(id.key())
        .unwrap_or_else(|| panic!("{} has no default shortcut", id.key()))
        .clone();
    Message::Key(Input::Accel(accel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::commands::Command;

    #[test]
    fn every_default_binding_names_a_real_command() {
        for platform in [Platform::Mac, Platform::Linux] {
            for (trigger, action) in keybind::default_bindings(platform) {
                assert!(
                    action_names().any(|name| name == action),
                    "{trigger} → {action} is not a command"
                );
            }
        }
    }

    #[test]
    fn missing_settings_use_platform_defaults_and_follow_the_system() {
        let preferences = Preferences::load(None);
        assert!(preferences.settings.warnings.is_empty());
        assert_eq!(preferences.theme(SystemMode::Light).0, definitions::LIGHT);
        assert_eq!(preferences.theme(SystemMode::Dark).0, definitions::DARK);
        let palette = keybind::parse_trigger(match Platform::current() {
            Platform::Mac => "super+shift+p",
            Platform::Linux => "alt+shift+p",
        })
        .unwrap();
        assert_eq!(
            preferences.keymap.action(&palette),
            Some("palette.commands")
        );
    }

    #[test]
    fn unreadable_settings_warn_and_light_theme_typos_are_reported() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, b"theme = \xff\n").unwrap();
        let preferences = Preferences::load(Some(path.clone()));
        assert!(preferences.warning().unwrap().contains("could not read"));
        std::fs::write(&path, "theme = light:solarised-light,dark:nord\n").unwrap();
        let preferences = Preferences::load(Some(path));
        assert!(preferences.warning().unwrap().contains("solarised-light"));
    }

    #[test]
    fn a_clean_reload_retracts_the_settings_banner_and_applies_the_theme() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "mystery = 1\n").unwrap();
        let options = super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(path.clone()));
        assert!(app.banner.as_deref().unwrap().contains("mystery"));
        std::fs::write(&path, "theme = nord\n").unwrap();
        assert!(app.reload_settings(false).is_none());
        assert!(app.banner.is_none());
        assert_eq!(app.definition, definitions::NORD);
        // Choosing a theme re-reads the merged file, picking up outside edits.
        std::fs::write(&path, "theme = nord\nline-numbers = false\n").unwrap();
        let _ = app.choose_theme(Some(definitions::ROSE_PINE.id));
        assert_eq!(app.definition, definitions::ROSE_PINE);
        assert!(!app.preferences.settings.line_numbers);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "theme = rose-pine\nline-numbers = false\n");
    }

    #[test]
    fn a_changed_boards_setting_waits_for_a_restart_and_says_so() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        let options = || super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (mut app, _) = App::with_config(options(), None, Some(path.clone()));
        assert!(!app.preferences.boards);
        assert!(app.banner.is_none());

        std::fs::write(&path, "boards = on\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `boards`.")
        );
        assert!(!app.preferences.boards);
        assert!(app.preferences.settings.boards);

        // A settings warning is still returned, with the notice beside it.
        std::fs::write(&path, "boards = on\nmystery = 1\n").unwrap();
        let warning = app.reload_settings(false).unwrap();
        assert!(warning.contains("mystery") && !warning.contains("Restart"));
        let banner = app.banner.clone().unwrap();
        assert!(
            banner.starts_with(&warning) && banner.ends_with("Restart Wren to apply `boards`.")
        );

        // Back at the launch value there is nothing to wait for.
        std::fs::write(&path, "boards = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(app.banner.is_none());

        // A launch with the setting on runs with it, and turning it off waits.
        std::fs::write(&path, "boards = on\n").unwrap();
        let (mut app, _) = App::with_config(options(), None, Some(path.clone()));
        assert!(app.preferences.boards);
        assert!(app.banner.is_none());
        std::fs::write(&path, "").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `boards`.")
        );
        assert!(app.preferences.boards);
    }

    #[test]
    fn focus_mode_centres_a_wrapped_column_and_otherwise_follows_the_setting() {
        use wren_editor::Measure;
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        let options = || super::super::Options {
            editor: true,
            ..Default::default()
        };
        let measure = |wrap_column, centred| Measure {
            wrap_column,
            centred,
        };
        let (mut app, _) = App::with_config(options(), None, Some(path.clone()));
        assert_eq!(app.editor_measure(), measure(Some(100), false));
        app.focus = true;
        assert_eq!(app.editor_measure(), measure(Some(100), true));
        std::fs::write(&path, "editor-wrap-column = none\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(app.editor_measure(), measure(Some(100), true));
        app.focus = false;
        assert_eq!(app.editor_measure(), measure(None, false));
        std::fs::write(&path, "editor-wrap-column = 72\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(app.editor_measure(), measure(Some(72), false));
        app.focus = true;
        assert_eq!(app.editor_measure(), measure(Some(72), true));
    }

    #[test]
    fn the_line_height_applies_on_reload_and_survives_text_size_steps() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "editor-line-height = 2\n").unwrap();
        let options = super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(path.clone()));
        assert_eq!(app.editor.row_height(), 28.0);
        let _ = app.step_text_size(Some(1.0));
        assert_eq!(app.editor.row_height(), 30.0);
        std::fs::write(&path, "editor-font-size = 15\neditor-line-height = 3\n").unwrap();
        assert!(app
            .reload_settings(false)
            .unwrap()
            .contains("editor-line-height"));
        assert_eq!(app.editor.row_height(), 26.0);
    }

    #[test]
    fn the_editor_engine_applies_at_startup_and_switches_live_keeping_the_document() {
        use iced::widget::text_editor::{Action, Edit, Motion};

        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "editor-engine = advanced\n").unwrap();
        let options = || super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (app, _) = App::with_config(options(), None, Some(path.clone()));
        assert_eq!(app.editor.backend(), BackendKind::Owned);

        std::fs::write(&path, "editor-font-size = 18\nline-numbers = off\n").unwrap();
        let (mut app, _) = App::with_config(options(), None, Some(path.clone()));
        assert_eq!(app.editor.backend(), BackendKind::Iced);
        let _ = app.edit(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        )));
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Insert(
            '!',
        ))));
        app.editor.set_find_query("the");
        let text = app.editor.text();
        let cursor = app.editor.cursor();
        let finds = app.editor.find_count();
        let revision = app.edit_revision;
        assert!(app.editor.can_undo());

        std::fs::write(
            &path,
            "editor-font-size = 18\nline-numbers = off\neditor-engine = advanced\n",
        )
        .unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(app.editor.backend(), BackendKind::Owned);
        assert_eq!(app.editor.text(), text);
        assert_eq!(app.editor.cursor(), cursor);
        assert_eq!(app.editor.find_count(), finds);
        assert_eq!(app.editor.font_size(), 18.0);
        assert_eq!(app.edit_revision, revision);
        assert!(!app.editor.can_undo());

        // An unrelated change leaves the editor, and its history, alone;
        // switching back rebuilds it.
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Insert(
            '?',
        ))));
        let text = app.editor.text();
        std::fs::write(&path, "editor-font-size = 18\neditor-engine = advanced\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(app.editor.backend(), BackendKind::Owned);
        assert!(app.editor.can_undo());
        std::fs::write(&path, "editor-engine = classic\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(app.editor.backend(), BackendKind::Iced);
        assert_eq!(app.editor.text(), text);
        assert!(!app.editor.can_undo());
    }

    #[test]
    fn the_editor_key_policy_follows_the_keymap() {
        use iced::keyboard::key::{Code, Physical};
        use iced::keyboard::{key::Named, Key, Modifiers};
        use wren_editor::Intent;

        let user = [("alt+t".to_string(), "markdown.italic".to_string())];
        let (keymap, warnings) = Keymap::build(
            keybind::default_bindings(Platform::current()),
            &user,
            |action| CommandId::parse(action).is_some(),
        );
        assert!(warnings.is_empty());
        let policy = key_policy(&keymap, true);
        let (editing, app) = match Platform::current() {
            Platform::Mac => (Modifiers::LOGO, Modifiers::LOGO),
            Platform::Linux => (Modifiers::CTRL, Modifiers::ALT),
        };
        let letter = |letter: &str, code, modifiers| {
            policy(
                &Key::Character(letter.into()),
                Physical::Code(code),
                modifiers,
            )
        };
        assert_eq!(
            policy(
                &Key::Named(Named::Enter),
                Physical::Code(Code::Enter),
                editing
            ),
            KeyRoute::Intent(Intent::ToggleTask)
        );
        assert_eq!(
            letter("b", Code::KeyB, editing),
            KeyRoute::Intent(Intent::Bold)
        );
        assert_eq!(
            letter("e", Code::KeyE, editing),
            KeyRoute::Intent(Intent::Code)
        );
        assert_eq!(
            policy(
                &Key::Named(Named::ArrowDown),
                Physical::Code(Code::ArrowDown),
                Modifiers::ALT
            ),
            KeyRoute::Intent(Intent::MoveLineDown)
        );
        // A user binding of an editor command is the editor's too.
        assert_eq!(
            letter("t", Code::KeyT, Modifiers::ALT),
            KeyRoute::Intent(Intent::Italic)
        );
        // App shortcuts are the app's alone, Find included.
        assert_eq!(letter("p", Code::KeyP, app), KeyRoute::Host);
        assert_eq!(letter("f", Code::KeyF, app), KeyRoute::Host);
        if Platform::current() == Platform::Linux {
            assert_eq!(letter("f", Code::KeyF, Modifiers::CTRL), KeyRoute::Host);
        }
        // Unbound keys and typing stay with the editor.
        assert_eq!(letter("z", Code::KeyZ, editing), KeyRoute::Pass);
        assert_eq!(letter("b", Code::KeyB, Modifiers::empty()), KeyRoute::Pass);
        assert_eq!(
            policy(
                &Key::Named(Named::Enter),
                Physical::Code(Code::Enter),
                Modifiers::empty()
            ),
            KeyRoute::Pass
        );
        // Without assist the editor commands' keys are the editor's own
        // again, and the app's stay the app's.
        let plain = key_policy(&keymap, false);
        assert_eq!(
            plain(
                &Key::Named(Named::Enter),
                Physical::Code(Code::Enter),
                editing
            ),
            KeyRoute::Pass
        );
        assert_eq!(
            plain(
                &Key::Named(Named::ArrowUp),
                Physical::Code(Code::ArrowUp),
                Modifiers::ALT
            ),
            KeyRoute::Pass
        );
        assert_eq!(
            plain(&Key::Character("p".into()), Physical::Code(Code::KeyP), app),
            KeyRoute::Host
        );
    }

    #[test]
    fn markdown_assist_applies_live_to_the_keys() {
        use iced::widget::text_editor::{Action, Edit, Motion};
        use wren_editor::Intent;

        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "").unwrap();
        let options = super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(path.clone()));
        assert!(app.editor.markdown_assist());
        let _ = app.edit(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        )));
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Paste(
            std::sync::Arc::new("\nplain\n- item".into()),
        ))));
        let _ = app.edit(wren_editor::Message::Intent(Intent::Enter));
        assert!(app.editor.text().ends_with("\n- item\n- "));

        std::fs::write(&path, "markdown-assist = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(!app.editor.markdown_assist());
        let _ = app.edit(wren_editor::Message::Intent(Intent::Enter));
        assert!(app.editor.text().ends_with("\n- item\n- \n"));
    }

    #[test]
    fn the_markdown_assist_command_flips_saves_and_applies_the_setting() {
        use iced::widget::text_editor::{Action, Edit, Motion};
        use wren_editor::Intent;

        let (mut app, path, _directory) = app_with_settings("# mine\ntheme = nord\n");
        let run = |app: &mut App| {
            let _ = app.update(Message::Command(Command::Run(
                CommandId::ToggleMarkdownAssist,
            )));
            assert_eq!(app.last_processed.as_ref().unwrap().warning, None);
        };
        assert_eq!(
            app.command_state(CommandId::ToggleMarkdownAssist),
            Some("on")
        );
        run(&mut app);
        assert!(!app.editor.markdown_assist());
        assert_eq!(
            app.command_state(CommandId::ToggleMarkdownAssist),
            Some("off")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\ntheme = nord\nmarkdown-assist = off\n"
        );
        let _ = app.edit(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        )));
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Paste(
            std::sync::Arc::new("\n- item".into()),
        ))));
        let _ = app.edit(wren_editor::Message::Intent(Intent::Enter));
        assert!(app.editor.text().ends_with("\n- item\n"));
        // The Markdown commands follow it.
        assert!(!app.command_enabled(CommandId::Bold));

        run(&mut app);
        assert!(app.editor.markdown_assist());
        assert!(app.command_enabled(CommandId::Bold));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\ntheme = nord\nmarkdown-assist = on\n"
        );
    }

    #[test]
    fn the_engine_command_switches_live_and_saves_keeping_text_and_cursor() {
        use iced::widget::text_editor::{Action, Edit, Motion};

        let (mut app, path, _directory) = app_with_settings("");
        assert_eq!(app.editor.backend(), BackendKind::Iced);
        let _ = app.edit(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        )));
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Insert(
            '!',
        ))));
        let text = app.editor.text();
        let cursor = app.editor.cursor();
        assert!(app.editor.can_undo());

        // From the palette, which the command closes.
        let _ = app.update(Message::Command(Command::Run(CommandId::CommandPalette)));
        let row = app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .find(|row| row.key == Key::Command(CommandId::SwitchEngine))
            .unwrap();
        assert!(row.enabled);
        assert_eq!(app.command_state(CommandId::SwitchEngine), Some("classic"));
        let _ = app.update(Message::Command(Command::Run(CommandId::SwitchEngine)));
        assert!(app.palette.is_none());
        assert_eq!(app.editor.backend(), BackendKind::Owned);
        assert_eq!(app.editor.text(), text);
        assert_eq!(app.editor.cursor(), cursor);
        assert!(!app.editor.can_undo());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "editor-engine = advanced\n"
        );
        assert_eq!(app.command_state(CommandId::SwitchEngine), Some("advanced"));

        let _ = app.update(Message::Command(Command::Run(CommandId::SwitchEngine)));
        assert_eq!(app.editor.backend(), BackendKind::Iced);
        assert_eq!(app.editor.text(), text);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "editor-engine = classic\n"
        );
    }

    #[test]
    fn the_settings_commands_apply_with_nowhere_to_save() {
        let options = super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (mut app, _) = App::with_config(options, None, None);
        let _ = app.update(Message::Command(Command::Run(CommandId::SwitchEngine)));
        let _ = app.update(Message::Command(Command::Run(
            CommandId::ToggleMarkdownAssist,
        )));
        assert_eq!(app.editor.backend(), BackendKind::Owned);
        assert!(!app.editor.markdown_assist());
        assert!(app.last_processed.as_ref().unwrap().disposition.committed());
    }

    #[test]
    fn an_engine_switch_rebuilds_from_the_session_draft() {
        use wren_core::ops::{Op, Session};

        let workspace = crate::test_support::tempdir();
        std::fs::create_dir_all(workspace.path().join("Alpha/Inbox")).unwrap();
        std::fs::write(workspace.path().join("Alpha/Inbox/First.md"), "first body").unwrap();
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "").unwrap();
        let options = super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(path.clone()));
        app.workspace_mode = true;
        app.session = Some(Session::open(workspace.path()).unwrap());
        app.sync_editor(true);
        let first = app.model().unwrap().notes[0].id;
        let _ = app.process_core(Op::OpenNote { note: Some(first) });
        assert_eq!(app.editor.text(), "first body");
        // The session's draft is the document of record.
        let _ = app.process_core(Op::EditDraft {
            note: first,
            contents: "first body, drafted".into(),
        });
        assert_eq!(
            app.session.as_ref().unwrap().document().unwrap().text,
            "first body, drafted"
        );
        // Diverge the widget from the session to show which copy the
        // rebuild reads.
        app.editor.replace_text("stale widget copy", true);

        std::fs::write(&path, "editor-engine = advanced\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(app.editor.backend(), BackendKind::Owned);
        assert_eq!(app.editor.text(), "first body, drafted");
    }

    #[test]
    fn text_size_shortcuts_step_clamp_reset_and_save_one_line() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "# mine\neditor-font-size = 31\ntheme = nord\n").unwrap();
        let options = super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(path.clone()));
        let size = |app: &App| app.preferences.settings.editor_font_size;
        let _ = app.update(shortcut(CommandId::TextBigger));
        assert_eq!(size(&app), 32.0);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\neditor-font-size = 32\ntheme = nord\n"
        );
        let _ = app.update(shortcut(CommandId::TextBigger));
        assert_eq!(size(&app), settings::MAX_FONT_SIZE);
        let _ = app.update(shortcut(CommandId::TextSmaller));
        assert_eq!(size(&app), 31.0);
        let _ = app.update(shortcut(CommandId::TextReset));
        assert_eq!(size(&app), settings::DEFAULT_FONT_SIZE);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("editor-font-size = 14\n"));
        assert_eq!(app.last_processed.as_ref().unwrap().warning, None);
        // `=` and `+`, however the toolkit reports the shift, reach the command.
        for trigger in ["plus", "shift+plus", "equal", "shift+equal"] {
            let accel =
                keybind::parse_trigger(&format!("{}+{trigger}", Platform::current().primary()))
                    .unwrap();
            assert_eq!(app.preferences.keymap.action(&accel), Some("font.increase"));
        }
    }

    fn app_with_settings(text: &str) -> (App, std::path::PathBuf, tempfile::TempDir) {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(&path, text).unwrap();
        let options = super::super::Options {
            editor: true,
            ..Default::default()
        };
        let (app, _) = App::with_config(options, None, Some(path.clone()));
        (app, path, directory)
    }

    fn active_key(app: &App) -> Key {
        let palette = app.palette.as_ref().unwrap();
        palette.results[palette.active.unwrap()].key.clone()
    }

    #[test]
    fn the_theme_picker_previews_the_highlighted_row_and_restores_on_close() {
        let (mut app, path, _directory) = app_with_settings("theme = nord\n");
        let _ = app.update(Message::Command(Command::Run(CommandId::ChooseTheme)));
        // It opens on the saved theme, so nothing changes yet.
        assert_eq!(active_key(&app), Key::Theme(Some(definitions::NORD.id)));
        assert_eq!(app.definition, definitions::NORD);
        let _ = app.update(Message::Key(Input::Nav(Nav::Down)));
        assert_eq!(app.definition, definitions::GRUVBOX_DARK);
        let _ = app.update(Message::PaletteQuery("light".into()));
        assert_eq!(active_key(&app), Key::Theme(Some(definitions::LIGHT.id)));
        assert_eq!(app.definition, definitions::LIGHT);
        let _ = app.update(Message::Escape);
        assert_eq!(app.definition, definitions::NORD);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme = nord\n");
        // Enter keeps the previewed theme and saves it.
        let _ = app.update(Message::Command(Command::Run(CommandId::ChooseTheme)));
        let _ = app.update(Message::Key(Input::Nav(Nav::Down)));
        let _ = app.update(Message::Key(Input::Nav(Nav::Enter)));
        assert!(app.palette.is_none());
        assert_eq!(app.definition, definitions::GRUVBOX_DARK);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "theme = gruvbox-dark\n"
        );
    }

    #[test]
    fn the_font_picker_lists_the_bundled_font_and_saves_a_quoted_family() {
        let (mut app, path, _directory) = app_with_settings("editor-font-size = 16\n");
        let _ = app.update(Message::Command(Command::Run(CommandId::ChooseFont)));
        assert_eq!(active_key(&app), Key::Font(fonts::EDITOR_FAMILY));
        let rows = &app.palette.as_ref().unwrap().results;
        assert!(rows.iter().filter(|row| row.current).count() == 1);
        if let Some(other) = rows
            .iter()
            .find(|row| !row.current)
            .map(|row| row.key.clone())
        {
            // Highlight another installed monospace family: it previews.
            while active_key(&app) != other {
                let _ = app.update(Message::Key(Input::Nav(Nav::Down)));
            }
            let Key::Font(family) = other else {
                unreachable!()
            };
            assert_eq!(app.preview.font, Some(family));
            let _ = app.update(Message::PaletteClose);
            assert_eq!(app.preview, Preview::default());
        }
        let _ = app.update(Message::Command(Command::Run(CommandId::ChooseFont)));
        let _ = app.update(Message::Key(Input::Nav(Nav::Enter)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "editor-font-size = 16\neditor-font-family = \"JetBrains Mono\"\n"
        );
        assert_eq!(
            app.preferences.settings.editor_font_family.as_deref(),
            Some(fonts::EDITOR_FAMILY)
        );
        assert!(app.banner.is_none());
    }

    #[test]
    fn an_unknown_editor_font_warns_and_uses_the_bundled_font() {
        let (app, _path, _directory) =
            app_with_settings("editor-font-family = \"No Such Font Wren\"\n");
        assert_eq!(app.preferences.editor_font().0, fonts::EDITOR);
        assert!(app
            .banner
            .as_deref()
            .unwrap()
            .contains("Unknown editor font \"No Such Font Wren\""));
    }

    #[test]
    fn settings_file_theme_keybinds_and_reload() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        std::fs::write(
            &path,
            "theme = nord\nkeybind = ctrl+shift+p = palette.commands\nkeybind = alt+x = bogus\n",
        )
        .unwrap();
        let mut preferences = Preferences::load(Some(path.clone()));
        assert_eq!(preferences.theme(SystemMode::Light).0, definitions::NORD);
        assert_eq!(
            preferences
                .keymap
                .action(&keybind::parse_trigger("ctrl+shift+p").unwrap()),
            Some("palette.commands")
        );
        assert!(preferences.warning().unwrap().contains("bogus"));
        assert!(!preferences.reload(false));
        std::fs::write(&path, "theme = mystery\n").unwrap();
        assert!(preferences.reload(false));
        let (theme, warning) = preferences.theme(SystemMode::Light);
        assert_eq!(theme, definitions::LIGHT);
        assert!(warning.unwrap().contains("mystery"));
    }
}
