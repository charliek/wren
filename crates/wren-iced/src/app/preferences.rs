//! User settings (`config.conf`): theme, keybindings, editor options.

use std::path::{Path, PathBuf};

use iced::keyboard::key::Physical;
use iced::theme::Mode as SystemMode;
use iced::{Font, Task};
use wren_core::keybind::{self, Accel, Keymap, Layer, Platform};
use wren_core::settings::{self, EditorEngine, Settings, ThemeChoice};
use wren_core::theme::{self as definitions, Appearance, Theme};
use wren_editor::{BackendKind, KeyPolicy, KeyRoute};

use super::board::Surface;
use super::commands::{CommandId, Disposition, Gateway, Scope};
use super::palette::{Key, Mode};
use super::{workspace, App, Message};
use crate::fonts;
use crate::keys::{Input, Nav};
use wren_canvas::Command as BoardCommand;

/// The theme or editor font picker row being previewed. `theme` is
/// `Some(None)` while previewing "System".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Preview {
    pub theme: Option<Option<&'static str>>,
    pub font: Option<&'static str>,
}

/// What the Board menu items were last enabled from; see `App::sync_menu`.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq)]
pub struct MenuInputs {
    state: MenuState,
    /// The open board's selection, which changes with neither its epoch
    /// nor its revision.
    selection: Option<std::collections::BTreeSet<wren_board::scene::Source>>,
}

/// Everything else the Board menu's enabling reads: the board by its epoch
/// (a reload or another board) and its revision (an edit).
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MenuState {
    loading: bool,
    boards: bool,
    session: bool,
    group: bool,
    surface: Surface,
    export: bool,
    board: Option<(u64, u64)>,
    writable: bool,
    typing: bool,
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
    /// The `tasks` setting as it was at launch, which is what the app runs
    /// with: the macOS menu bar is built once, so a later value waits for a
    /// restart.
    pub tasks: bool,
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
/// command's key is the editor's intent, or with `tasks` on a task
/// command's, the toggle's included, the command itself.
fn key_policy(keymap: &Keymap, markdown_assist: bool, tasks: bool) -> KeyPolicy {
    let keymap = keymap.clone();
    std::sync::Arc::new(move |key, physical, modifiers| {
        crate::keys::shortcut(key, physical, modifiers).map_or(KeyRoute::Pass, |trigger| {
            key_route(&keymap, &trigger, markdown_assist, tasks)
        })
    })
}

/// Where the editor sends a press of `trigger`. Without Markdown assist the
/// Markdown commands are off, so their keys keep the editor's own meaning:
/// Ctrl+Enter breaks the line and Alt+↑ moves up. The suggest key is the
/// editor's either way. With `tasks` on the toggle writes stamps, which
/// the app does, so its key comes back to the app as the command.
pub(super) fn key_route(
    keymap: &Keymap,
    trigger: &Accel,
    markdown_assist: bool,
    tasks: bool,
) -> KeyRoute {
    match keymap.action(trigger).and_then(CommandId::parse) {
        None => KeyRoute::Pass,
        Some(id) => match id.editor_route() {
            None => KeyRoute::Host,
            Some(KeyRoute::Intent(_)) if !markdown_assist => KeyRoute::Pass,
            Some(KeyRoute::Intent(wren_editor::Intent::ToggleTask)) if tasks => {
                KeyRoute::Command(id.payload())
            }
            Some(route) => route,
        },
    }
}

/// The keymap layer of a command key; `None` for a key that names none.
pub(super) fn layer_of(action: &str) -> Option<Layer> {
    CommandId::parse(action).map(|id| match id.scope() {
        Scope::Board => Layer::Board,
        Scope::App | Scope::Editor => Layer::Base,
    })
}

/// Where a focused board sends a key press (§5.1): `Host` for an app
/// shortcut, which the board leaves to `App::key_input`; `Command` for a
/// board command, captured and published as the command; `Pass` for
/// anything else, captured so it never reaches the columns, and text while
/// a label or the connect query is open.
pub(super) use wren_canvas::KeyRoute as BoardRoute;

/// The board's say over its keys, from the same keymap and key reading as
/// the app's shortcuts. Escape always runs the board's ladder; an input
/// method's preedit and a held Space are the widget's to see first.
pub(super) fn board_policy(keymap: &Keymap) -> wren_canvas::KeyPolicy {
    use iced::keyboard::{key::Named, Key};
    let keymap = keymap.clone();
    std::sync::Arc::new(move |key, physical, modifiers, repeat| {
        if matches!(key, Key::Named(Named::Escape)) {
            return BoardRoute::Pass;
        }
        // Bare keys are board triggers, so read the press as `accel` does,
        // not as `shortcut`, which drops them.
        let Some(trigger) = crate::keys::accel(key, physical, modifiers) else {
            return BoardRoute::Pass;
        };
        let route = board_route(&keymap, &trigger, repeat);
        match digit_row(&keymap, &trigger, physical) {
            Some(fallback) => match board_route(&keymap, &fallback, repeat) {
                // The app reads the typed character, not this fallback.
                BoardRoute::Host => route,
                found => found,
            },
            None => route,
        }
    })
}

/// The press of a digit-row key read as its digit, when the character it
/// typed is bound in neither layer (§5.1 rule 7). `accel` names an
/// unshifted key by the character typed, so on AZERTY the 1 key is `&`.
fn digit_row(keymap: &Keymap, trigger: &Accel, physical: Physical) -> Option<Accel> {
    use iced::keyboard::key::Code;
    let Physical::Code(code) = physical else {
        return None;
    };
    let digit = match code {
        Code::Digit0 => "0",
        Code::Digit1 => "1",
        Code::Digit2 => "2",
        Code::Digit3 => "3",
        Code::Digit4 => "4",
        Code::Digit5 => "5",
        Code::Digit6 => "6",
        Code::Digit7 => "7",
        Code::Digit8 => "8",
        Code::Digit9 => "9",
        _ => return None,
    };
    let unbound = keymap.action(trigger).is_none() && keymap.board_action(trigger).is_none();
    (unbound && trigger.key != digit).then(|| Accel {
        mods: trigger.mods,
        key: digit.to_owned(),
    })
}

/// Where a focused board sends a press of `trigger`: the base layer's App
/// commands first, then the board layer. A held key repeats only the
/// commands that step (§5.1 rule 4).
pub(super) fn board_route(keymap: &Keymap, trigger: &Accel, repeat: bool) -> BoardRoute {
    let app = |action: &str| CommandId::parse(action).is_some_and(|id| id.scope() == Scope::App);
    if keymap.action(trigger).is_some_and(app) {
        return BoardRoute::Host;
    }
    match keymap
        .board_action(trigger)
        .and_then(CommandId::parse)
        .and_then(CommandId::board_command)
    {
        Some(command) if !repeat || super::commands::board_repeats(command) => {
            BoardRoute::Command(command)
        }
        _ => BoardRoute::Pass,
    }
}

/// Escape with any modifiers, which a focused board never routes.
fn is_escape(trigger: &Accel) -> bool {
    trigger.key == "escape"
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

/// The keymap of `platform`'s defaults and the user's `keybinds`, before
/// the board bindings the app shadows are dropped. The task defaults join
/// only while `tasks` is on.
pub(super) fn build_keymap(
    platform: Platform,
    keybinds: &[(String, String)],
    tasks: bool,
) -> (Keymap, Vec<String>) {
    let task_defaults = if tasks {
        keybind::default_task_bindings(platform)
    } else {
        Vec::new()
    };
    Keymap::build(
        keybind::default_bindings(platform)
            .into_iter()
            .chain(keybind::default_board_bindings(platform))
            .chain(task_defaults),
        keybinds,
        // With tasks off a task command does not exist, so a `keybind`
        // line naming one is an unknown action.
        |action| {
            layer_of(action)
                .filter(|_| tasks || !CommandId::parse(action).is_some_and(CommandId::tasks))
        },
    )
}

impl Preferences {
    /// Load from `path`; `None` means defaults with nowhere to persist.
    pub fn load(path: Option<PathBuf>) -> Self {
        let (loaded, settings) = read(path.as_deref());
        let mut preferences = Self {
            keymap: Keymap::default(),
            boards: settings.boards,
            tasks: settings.tasks,
            settings,
            path,
            loaded,
            shown_warning: None,
        };
        preferences.rebuild_keymap();
        preferences
    }

    pub(super) fn rebuild_keymap(&mut self) {
        let (mut keymap, mut warnings) =
            build_keymap(Platform::current(), &self.settings.keybinds, self.tasks);
        // A board binding on a trigger the base layer gives an App command
        // could never run, since the app's shortcut wins, and neither could
        // one on Escape, which `board_policy` keeps for the ladder; drop
        // it, and say so when the user wrote it.
        let shadowed: Vec<Accel> = keymap
            .board_bindings()
            .filter(|(accel, _)| {
                is_escape(accel) || board_route(&keymap, accel, false) == BoardRoute::Host
            })
            .map(|(accel, _)| accel.clone())
            .collect();
        for (trigger, action) in &self.settings.keybinds {
            let action = action.trim();
            if let Some(accel) = keybind::parse_trigger(trigger)
                .filter(|accel| shadowed.contains(accel))
                .filter(|accel| keymap.board_action(accel) == Some(action))
            {
                warnings.push(if is_escape(&accel) {
                    format!("keybind: {trigger:?} is the board's Escape ladder, so it cannot be {action}")
                } else {
                    format!(
                        "keybind: {trigger:?} is {} everywhere, so it cannot be {action} on a board",
                        keymap.action(&accel).unwrap_or_default()
                    )
                });
            }
        }
        keymap.retain_board(|accel, _| !shadowed.contains(accel));
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

    /// What to say when the file asks for a `boards` or `tasks` value other
    /// than the one the app launched with.
    fn restart_notice(&self) -> Option<String> {
        let waiting: Vec<&str> = [
            ("`boards`", self.settings.boards != self.boards),
            ("`tasks`", self.settings.tasks != self.tasks),
        ]
        .into_iter()
        .filter_map(|(name, waits)| waits.then_some(name))
        .collect();
        (!waiting.is_empty()).then(|| format!("Restart Wren to apply {}.", waiting.join(" and ")))
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
            self.editor.close_popover();
            self.definition = theme;
            self.toolkit_theme = crate::theme::from_definition(theme);
            let palette = self.board_palette();
            if let Some(board) = &mut self.board {
                board.set_palette(palette);
            }
        }
    }

    /// Re-read `config.conf` and `words.txt` and apply them. Returns any
    /// settings warning, or `None` when neither changed.
    ///
    /// An engine switch keeps the editor widget's tree state, and with it
    /// keyboard focus, so nothing is refocused here.
    pub(super) fn reload_settings(&mut self, force: bool) -> Option<String> {
        #[cfg(target_os = "macos")]
        let keymap = self.preferences.keymap.clone();
        let words = self.spelling.reload_words();
        if self.preferences.reload(force) {
            self.apply_preferences();
        } else if words {
            self.sync_proofreader();
        } else {
            return None;
        }
        #[cfg(target_os = "macos")]
        if self.window.is_some() && self.preferences.keymap != keymap {
            self.install_menu();
        }
        self.show_settings_warning()
    }

    /// The warnings of `config.conf` and of `words.txt`, each after its path.
    fn settings_warning(&self) -> Option<String> {
        let warnings: Vec<String> = [self.preferences.warning(), self.spelling.warning()]
            .into_iter()
            .flatten()
            .collect();
        (!warnings.is_empty()).then(|| warnings.join("; "))
    }

    /// The settings warning with the restart notice after it. The notice is
    /// shown like a warning and retracted like one, but a setting that waits
    /// for a restart has not failed.
    fn shown_settings_warning(&self) -> Option<String> {
        match (self.settings_warning(), self.preferences.restart_notice()) {
            (Some(warning), Some(notice)) => Some(format!("{warning} {notice}")),
            (warning, notice) => warning.or(notice),
        }
    }

    /// Show the settings warning in the banner, or retract the one shown
    /// before when there is none. Returns the warning.
    pub(super) fn show_settings_warning(&mut self) -> Option<String> {
        let warning = self.settings_warning();
        let shown = self.shown_settings_warning();
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

    /// Apply the settings to the theme and editor. A change of settings
    /// closes the suggestions menu, as any popover of the editor's (rule
    /// 4.4.8).
    pub(super) fn apply_preferences(&mut self) {
        self.editor.close_popover();
        self.apply_shown();
        let settings = &self.preferences.settings;
        self.editor.set_line_height(settings.editor_line_height);
        self.editor.set_font_size(settings.editor_font_size);
        self.editor.set_line_numbers(settings.line_numbers);
        let assist = self.markdown_assist();
        self.editor.set_markdown_assist(assist);
        self.editor.set_key_policy(key_policy(
            &self.preferences.keymap,
            assist,
            self.preferences.tasks,
        ));
        self.sync_task_options();
        let covered = self.board_covered();
        if let Some(view) = &mut self.board {
            view.set_key_policy(board_policy(&self.preferences.keymap));
            view.set_covered(covered.0, covered.1);
        }
        self.apply_editor_engine();
        self.sync_proofreader();
    }

    /// Whether the editor's Markdown keys act: as the setting says, and
    /// never in a board's raw text, which is not Markdown.
    fn markdown_assist(&self) -> bool {
        self.preferences.settings.markdown_assist && self.surface() != Surface::Raw
    }

    /// Bring the editor's Markdown keys, its task menus and its
    /// proofreader in step with the surface.
    pub(super) fn apply_markdown_assist(&mut self) {
        self.sync_task_menus();
        let assist = self.markdown_assist();
        if assist != self.editor.markdown_assist() {
            self.editor.set_markdown_assist(assist);
            self.editor.set_key_policy(key_policy(
                &self.preferences.keymap,
                assist,
                self.preferences.tasks,
            ));
        }
        self.sync_proofreader();
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
    /// headings the same way (`sync_outline`), and the jump palette a
    /// board's elements (`sync_board_nodes`).
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
        let actions = self.sync_actions();
        Task::batch([self.sync_outline(), self.sync_board_nodes(), actions])
    }

    /// Whether the settings warning differs from the one last shown.
    pub(super) fn settings_warning_changed(&self) -> bool {
        self.shown_settings_warning() != self.preferences.shown_warning
    }

    pub(super) fn startup_settings_warning(&mut self) -> Option<String> {
        let warning = self.settings_warning();
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

    /// Flip `spell-check`, apply it, and save it. On the classic engine it
    /// still flips, for when the advanced one is chosen.
    pub(super) fn toggle_spell_check(&mut self) -> Gateway {
        let enabled = !self.preferences.settings.spell_check;
        self.preferences.settings.spell_check = enabled;
        self.save_toggle("spell-check", if enabled { "on" } else { "off" })
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
            CommandId::ToggleSpellCheck => Some(if settings.spell_check { "on" } else { "off" }),
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
        self.open_user_file(|path| path.to_path_buf(), settings::ensure_exists)
    }

    /// Open `words.txt` beside `config.conf` in the user's editor, creating
    /// it with a comment header (rule 4.5.3).
    pub(super) fn open_words(&mut self) -> Gateway {
        self.open_user_file(settings::words_path, settings::ensure_words_exist)
    }

    /// Open the file `locate` finds from the settings path in the user's
    /// editor, after `ensure` has created it if need be.
    fn open_user_file(
        &mut self,
        locate: fn(&Path) -> PathBuf,
        ensure: fn(&Path) -> std::io::Result<()>,
    ) -> Gateway {
        let Some(path) = self.preferences.path.as_deref().map(locate) else {
            return self.blocked("No settings location is available (set HOME or WREN_CONFIG)");
        };
        if let Err(error) = ensure(&path) {
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

    /// Build the menu bar for the keymap and the `boards` setting the app
    /// runs with; the setting is fixed until a restart, so a reload that
    /// rebuilds the menu keeps or leaves out the Board menu as before.
    #[cfg(target_os = "macos")]
    pub(super) fn install_menu(&mut self) {
        let installed = crate::macos_menu::install(
            &self.preferences.keymap,
            self.preferences.boards,
            self.preferences.tasks,
        );
        // The new items start enabled, whatever the last sync set.
        self.menu_inputs = None;
        if let Err(error) = installed {
            self.banner = Some(format!("Could not build the menu bar: {error}"));
            self.banner_kind = workspace::WarningKind::General;
        }
    }

    /// Whether the open board holds the keyboard as text: a label is open
    /// (an input method's preedit lives only in one) or keyboard Connecting
    /// takes a query. The board's own keys then run no command a menu
    /// lists (§5.1 rules 2 and 3), and the menu must not either, since
    /// AppKit gives a ⌘ key equivalent to the menu before the board sees it.
    #[cfg(any(target_os = "macos", test, feature = "smoke-test"))]
    fn board_typing(&self) -> bool {
        self.board.as_ref().is_some_and(|view| {
            let machine = view.machine();
            machine.editing().is_some() || machine.seeking().is_some()
        })
    }

    /// Whether a menu item would be greyed for typing: a board command
    /// while the board takes text.
    #[cfg(any(target_os = "macos", test, feature = "smoke-test"))]
    fn menu_typing_blocks(&self, id: CommandId) -> bool {
        matches!(id, CommandId::Board(_)) && self.board_typing()
    }

    /// Whether the menu item for a command key is enabled: exactly when its
    /// command would run, except that a board command is greyed while the
    /// board takes text. The align items, and the distribute items, which
    /// each share one rule (§5.16 rule 3), count a large selection once
    /// between them.
    #[cfg(any(target_os = "macos", test, feature = "smoke-test"))]
    pub(super) fn menu_enabled(&self) -> impl FnMut(&str) -> bool + '_ {
        use wren_canvas::Command as C;
        let (mut align, mut distribute) = (None, None);
        move |key| {
            let Some(id) = CommandId::parse(key) else {
                return false;
            };
            if self.menu_typing_blocks(id) {
                return false;
            }
            let shared = match id {
                CommandId::Board(C::Align(_)) => &mut align,
                CommandId::Board(C::Distribute(_)) => &mut distribute,
                _ => return self.command_enabled(id),
            };
            *shared.get_or_insert_with(|| self.command_enabled(id))
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn menu_state(&self) -> MenuState {
        MenuState {
            loading: self.loading.is_some(),
            boards: self.preferences.boards,
            session: self.session.is_some(),
            group: self.selected_group.is_some(),
            surface: self.surface(),
            export: self.board_export.is_some(),
            board: self
                .board
                .as_ref()
                .map(|view| (view.epoch(), self.board_revision)),
            writable: self
                .board
                .as_ref()
                .is_some_and(wren_canvas::BoardView::writable),
            typing: self.board_typing(),
        }
    }

    /// Whether the Board menu's items were last enabled from what the app
    /// holds now. The selection is compared where it lies, not copied.
    #[cfg(any(target_os = "macos", test))]
    pub(super) fn menu_current(&self) -> bool {
        self.menu_inputs.as_ref().is_some_and(|inputs| {
            inputs.state == self.menu_state()
                && inputs.selection.as_ref()
                    == self.board.as_ref().map(|view| view.machine().selection())
        })
    }

    /// Bring the Board menu's items up to date, after every message. They
    /// are worked out again only when something they read has changed, so
    /// the autosave tick and a pointer passing over the board cost a
    /// compare, not a count of the selection.
    #[cfg(any(target_os = "macos", test))]
    pub(super) fn sync_menu(&mut self) {
        if self.menu_current() {
            return;
        }
        #[cfg(target_os = "macos")]
        crate::macos_menu::sync_board(self.menu_enabled());
        #[cfg(test)]
        {
            let board = crate::menu::MENUS
                .iter()
                .find(|menu| menu.kind == crate::menu::Kind::Board)
                .map_or_else(Vec::new, |menu| crate::menu::commands(menu.rows));
            let synced = {
                let mut enabled = self.menu_enabled();
                board.iter().map(|(key, _)| (*key, enabled(key))).collect()
            };
            self.menu_synced = synced;
            self.menu_syncs += 1;
        }
        self.menu_inputs = Some(MenuInputs {
            state: self.menu_state(),
            selection: self
                .board
                .as_ref()
                .map(|view| view.machine().selection().clone()),
        });
    }

    /// Run a command chosen from the native menu bar, with the same rules
    /// as its keyboard shortcut. A board command does nothing while the
    /// board takes text, as its greyed item does on a Mac.
    #[cfg(any(target_os = "macos", test, feature = "smoke-test"))]
    pub(super) fn menu_command(&mut self, key: &str) -> Task<Message> {
        if CommandId::parse(key).is_some_and(|id| self.menu_typing_blocks(id)) {
            return Task::none();
        }
        #[cfg(target_os = "macos")]
        if key == crate::menu::CHECK_FOR_UPDATES {
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
        // A press of Escape the calendar's field took: it was the
        // calendar's alone.
        if matches!(input, Input::Nav(Nav::Escape | Nav::FieldEscape))
            && (self.take_calendar_escape() || self.take_form_escape())
        {
            return Task::none();
        }
        // The task form owns the keyboard: its own keys took what it gives
        // a meaning, Escape and its save keys included (`form_keys.rs`), so
        // they are taken only once the form is shown, never while an input
        // method composes or its calendar is open; of the app's shortcuts
        // only quitting runs.
        if self.task_form.is_some() {
            return match input {
                Input::Nav(Nav::Escape | Nav::FieldEscape) => self.update(Message::Escape),
                Input::Nav(_) => Task::none(),
                Input::Accel(accel) => {
                    match self
                        .preferences
                        .keymap
                        .action(&accel)
                        .and_then(CommandId::parse)
                    {
                        Some(CommandId::Quit) => self.run_shortcut(CommandId::Quit),
                        _ => Task::none(),
                    }
                }
            };
        }
        let accel = match input {
            // A press the editor's popover has already taken, in the key's
            // order with the typing around it, does nothing more here. The
            // suggestions menu is closed from here instead: the editor
            // leaves it Escape with no word.
            Input::Nav(Nav::Escape | Nav::FieldEscape)
                if self.editor.take_claimed_escape() || self.editor.dismiss_popover() =>
            {
                return Task::none();
            }
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
            // The keys sheet runs nothing: the arrows scroll it and Enter
            // closes it.
            Input::Nav(nav)
                if self
                    .palette
                    .as_ref()
                    .is_some_and(|palette| palette.mode == Mode::BoardKeys) =>
            {
                let step = |y| {
                    iced::widget::operation::scroll_by(
                        "palette-results",
                        iced::widget::operation::AbsoluteOffset { x: 0.0, y },
                    )
                };
                return match nav {
                    Nav::Up => step(-48.0),
                    Nav::Down => step(48.0),
                    Nav::Enter => self.close_palette().task,
                    _ => Task::none(),
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
        // The sheet's own key closes it, as `?` typed in its filter does.
        if self
            .palette
            .as_ref()
            .is_some_and(|palette| palette.mode == Mode::BoardKeys)
            && self.preferences.keymap.board_action(&accel) == Some(BoardCommand::Keys.key())
        {
            return self.close_palette().task;
        }
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
            if self.dialog.is_some() || self.task_form.is_some() {
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
    super::commands::definitions().map(|definition| definition.key)
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
            for (trigger, action) in keybind::default_bindings(platform)
                .into_iter()
                .chain(keybind::default_board_bindings(platform))
            {
                assert!(
                    action_names().any(|name| name == action),
                    "{trigger} → {action} is not a command"
                );
            }
        }
    }

    #[test]
    fn no_default_board_key_shadows_an_app_command() {
        // The base layer is read first, so a board default on a trigger the
        // base layer gives to an App command could never run; one it gives
        // to an Editor command is inert while a board has the keyboard.
        for platform in [Platform::Mac, Platform::Linux] {
            let (keymap, _) = Keymap::build(keybind::default_bindings(platform), &[], layer_of);
            for (trigger, action) in keybind::default_board_bindings(platform) {
                assert!(
                    wren_canvas::Command::parse(action).is_some(),
                    "{trigger} → {action} is not a board command"
                );
                let accel = keybind::parse_trigger(&trigger).unwrap();
                if let Some(base) = keymap.action(&accel).and_then(CommandId::parse) {
                    assert_eq!(
                        base.scope(),
                        Scope::Editor,
                        "{trigger} is {} in the base layer",
                        base.key()
                    );
                }
            }
        }
    }

    fn trigger(trigger: &str) -> Accel {
        keybind::parse_trigger(trigger).unwrap()
    }

    /// `p` is the app modifier and `m` the editing one, for the platform
    /// the tests run on.
    fn modifiers() -> (&'static str, &'static str) {
        match Platform::current() {
            Platform::Mac => ("super", "super"),
            Platform::Linux => ("alt", "ctrl"),
        }
    }

    #[test]
    fn the_app_keymap_keeps_the_0_1_1_base_layer_and_every_board_default() {
        for platform in [Platform::Mac, Platform::Linux] {
            let (keymap, warnings) = Keymap::build(
                keybind::default_bindings(platform)
                    .into_iter()
                    .chain(keybind::default_board_bindings(platform)),
                &[],
                layer_of,
            );
            assert!(warnings.is_empty());
            let (base, _) = Keymap::build(keybind::default_bindings(platform), &[], |_| {
                Some(Layer::Base)
            });
            assert!(keymap.bindings().eq(base.bindings()), "{platform:?}");
            let defaults = keybind::default_board_bindings(platform);
            assert!(keymap
                .board_bindings()
                .map(|(accel, action)| (accel.to_string(), action))
                .eq(defaults
                    .iter()
                    .map(|(bound, action)| (trigger(bound).to_string(), *action))));
        }
        // What the app loads is that keymap, with nothing dropped.
        let preferences = Preferences::load(None);
        assert!(preferences.settings.warnings.is_empty());
        assert_eq!(
            preferences.keymap.board_bindings().count(),
            keybind::default_board_bindings(Platform::current()).len()
        );
    }

    #[test]
    fn a_board_key_goes_to_the_app_the_board_or_the_text_in_that_order() {
        use wren_board::Dir;
        use wren_canvas::{Command as C, Grow, Tool};
        let keymap = Preferences::load(None).keymap;
        let (p, m) = modifiers();
        let route = |key: &str, repeat| board_route(&keymap, &trigger(key), repeat);
        assert_eq!(
            route("v", false),
            BoardRoute::Command(C::Tool(Tool::Select))
        );
        assert_eq!(route("tab", false), BoardRoute::Command(C::ConnectNext));
        assert_eq!(route("shift+1", false), BoardRoute::Command(C::ZoomFit));
        assert_eq!(route("f2", false), BoardRoute::Command(C::EditLabel));
        // The base layer gives these to editor commands, inert on a board.
        assert_eq!(
            route(&format!("{m}+enter"), false),
            BoardRoute::Command(C::CommitLabel)
        );
        assert_eq!(
            route("alt+up", false),
            BoardRoute::Command(C::Move(Dir::Up))
        );
        // App shortcuts stay the app's.
        for key in ["p", "n", "shift+p", "bracketleft", "y", "0"] {
            assert_eq!(
                route(&format!("{p}+{key}"), false),
                BoardRoute::Host,
                "{key}"
            );
        }
        if Platform::current() == Platform::Linux {
            assert_eq!(route("ctrl+f", false), BoardRoute::Host);
            assert_eq!(route("ctrl+y", false), BoardRoute::Command(C::Redo));
        }
        // Unbound keys are captured as text.
        assert_eq!(route("k", false), BoardRoute::Pass);
        assert_eq!(route("shift+k", false), BoardRoute::Pass);
        assert_eq!(route(&format!("{m}+b"), false), BoardRoute::Pass);
        // Held keys repeat stepping only.
        assert_eq!(
            route("left", true),
            BoardRoute::Command(C::Select(Dir::Left))
        );
        assert_eq!(
            route("shift+down", true),
            BoardRoute::Command(C::ExtendSelection(Dir::Down))
        );
        assert_eq!(
            route("alt+right", true),
            BoardRoute::Command(C::Move(Dir::Right))
        );
        assert_eq!(
            route("alt+shift+right", true),
            BoardRoute::Command(C::Resize(Grow::Wider))
        );
        for key in ["tab", "delete", "backspace", "enter", "v"] {
            assert_eq!(route(key, true), BoardRoute::Pass, "{key}");
        }
    }

    #[test]
    fn the_board_policy_reads_bare_keys_and_keeps_escape_for_the_ladder() {
        use iced::keyboard::key::{Code, Named, Physical};
        use iced::keyboard::{Key, Modifiers};
        use wren_canvas::{Command as C, Tool};
        let policy = board_policy(&Preferences::load(None).keymap);
        let app = match Platform::current() {
            Platform::Mac => Modifiers::LOGO,
            Platform::Linux => Modifiers::ALT,
        };
        let character = |c: &str, code, modifiers| {
            policy(
                &Key::Character(c.into()),
                Physical::Code(code),
                modifiers,
                false,
            )
        };
        let named = |named, code, modifiers| {
            policy(&Key::Named(named), Physical::Code(code), modifiers, false)
        };
        assert_eq!(
            character("v", Code::KeyV, Modifiers::empty()),
            BoardRoute::Command(C::Tool(Tool::Select))
        );
        assert_eq!(
            character("a", Code::KeyA, Modifiers::empty()),
            BoardRoute::Command(C::Tool(Tool::Connector))
        );
        // Shift+1 types `!`; the binding names the key.
        assert_eq!(
            character("!", Code::Digit1, Modifiers::SHIFT),
            BoardRoute::Command(C::ZoomFit)
        );
        // `?` is Shift+/ on US layouts, and `/` alone the actions palette.
        // Where `?` is another key, AZERTY's `,` with Shift, the key is
        // nothing to the keymap and the board asks again as Shift+/.
        for typed in ["?", "/"] {
            assert_eq!(
                character(typed, Code::Slash, Modifiers::SHIFT),
                BoardRoute::Command(C::Keys)
            );
        }
        assert_eq!(
            character("/", Code::Slash, Modifiers::empty()),
            BoardRoute::Command(C::Actions)
        );
        assert_eq!(
            character(",", Code::KeyM, Modifiers::SHIFT),
            BoardRoute::Pass
        );
        assert_eq!(
            named(Named::Tab, Code::Tab, Modifiers::empty()),
            BoardRoute::Command(C::ConnectNext)
        );
        assert_eq!(
            named(Named::Delete, Code::Delete, Modifiers::empty()),
            BoardRoute::Command(C::Delete)
        );
        assert_eq!(character("p", Code::KeyP, app), BoardRoute::Host);
        assert_eq!(
            character("k", Code::KeyK, Modifiers::empty()),
            BoardRoute::Pass
        );
        for modifiers in [Modifiers::empty(), Modifiers::CTRL, app] {
            assert_eq!(
                named(Named::Escape, Code::Escape, modifiers),
                BoardRoute::Pass
            );
        }
        assert_eq!(
            policy(
                &Key::Named(Named::Tab),
                Physical::Code(Code::Tab),
                Modifiers::empty(),
                true
            ),
            BoardRoute::Pass
        );
    }

    /// §5.1 rule 7: on AZERTY the unshifted digit row types `&`, `"`, `'`,
    /// `(` and `-`, which no binding names, so the key reads as its digit;
    /// a character a binding does name keeps its own meaning.
    #[test]
    fn the_digit_row_falls_back_to_the_physical_key_when_the_character_is_unbound() {
        use iced::keyboard::key::{Code, Physical};
        use iced::keyboard::{Key, Modifiers};
        use wren_board::Token;
        use wren_canvas::Command as C;
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        let (p, _) = modifiers();
        std::fs::write(&path, format!("keybind = {p}+1 = board.color.blue\n")).unwrap();
        let preferences = Preferences::load(Some(path));
        assert!(
            preferences.settings.warnings.is_empty(),
            "{:?}",
            preferences.settings.warnings
        );
        let policy = board_policy(&preferences.keymap);
        let press = |c: &str, code, modifiers| {
            policy(
                &Key::Character(c.into()),
                Physical::Code(code),
                modifiers,
                false,
            )
        };
        for (typed, code, token) in [
            ("&", Code::Digit1, Token::Red),
            ("\"", Code::Digit3, Token::Yellow),
            ("'", Code::Digit4, Token::Green),
            ("(", Code::Digit5, Token::Cyan),
            ("-", Code::Digit6, Token::Purple),
            // Non-Latin text already reads the physical key.
            ("é", Code::Digit2, Token::Orange),
            ("1", Code::Digit1, Token::Red),
        ] {
            assert_eq!(
                press(typed, code, Modifiers::empty()),
                BoardRoute::Command(C::Color(token)),
                "{typed}"
            );
        }
        // The fallback keeps the modifiers: a user's chord on the digit.
        let chord = match Platform::current() {
            Platform::Mac => Modifiers::LOGO,
            Platform::Linux => Modifiers::ALT,
        };
        assert_eq!(
            press("&", Code::Digit1, chord),
            BoardRoute::Command(C::Color(Token::Blue))
        );
        // A digit key that types a bound character is that character's,
        // and a key off the digit row has no fallback.
        assert_eq!(
            press("/", Code::Digit7, Modifiers::empty()),
            BoardRoute::Command(C::Actions)
        );
        assert_eq!(press("&", Code::KeyQ, Modifiers::empty()), BoardRoute::Pass);
        assert_eq!(
            press("_", Code::Digit8, Modifiers::empty()),
            BoardRoute::Pass
        );
        // A held digit does one thing.
        assert_eq!(
            policy(
                &Key::Character("&".into()),
                Physical::Code(Code::Digit1),
                Modifiers::empty(),
                true
            ),
            BoardRoute::Pass
        );
    }

    #[test]
    fn a_board_binding_on_an_app_key_is_dropped_and_a_user_one_warns() {
        let directory = crate::test_support::tempdir();
        let path = directory.path().join("config.conf");
        let (p, m) = modifiers();
        std::fs::write(
            &path,
            format!(
                "keybind = {p}+n = board.delete\nkeybind = {m}+b = board.duplicate\nkeybind = {m}+d = note.rename\nkeybind = x = board.delete\nkeybind = escape = board.select.all\nkeybind = shift+escape = board.copy\n"
            ),
        )
        .unwrap();
        let preferences = Preferences::load(Some(path));
        let keymap = &preferences.keymap;
        let warnings = &preferences.settings.warnings;
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(
            warnings[0].contains("note.new") && warnings[0].contains("board.delete"),
            "{warnings:?}"
        );
        // Escape is the ladder's with any modifiers, so these never run.
        assert!(
            warnings[1].contains("\"escape\" is the board's Escape ladder")
                && warnings[1].contains("board.select.all"),
            "{warnings:?}"
        );
        assert!(warnings[2].contains("\"shift+escape\""), "{warnings:?}");
        assert_eq!(keymap.board_action(&trigger("escape")), None);
        assert_eq!(keymap.board_action(&trigger("shift+escape")), None);
        assert_eq!(keymap.board_action(&trigger(&format!("{p}+n"))), None);
        assert_eq!(keymap.action(&trigger(&format!("{p}+n"))), Some("note.new"));
        // An editor command's key is free on a board.
        assert_eq!(
            keymap.board_action(&trigger(&format!("{m}+b"))),
            Some("board.duplicate")
        );
        // A default the user's own app binding covers goes quietly.
        assert_eq!(keymap.board_action(&trigger(&format!("{m}+d"))), None);
        assert_eq!(
            keymap.accel("board.duplicate"),
            Some(&trigger(&format!("{m}+b")))
        );
        assert_eq!(keymap.board_action(&trigger("x")), Some("board.delete"));
        assert_eq!(keymap.accel("board.delete"), Some(&trigger("x")));
    }

    #[test]
    fn missing_settings_use_platform_defaults_and_follow_the_system() {
        let preferences = Preferences::load(None);
        assert!(preferences.settings.warnings.is_empty());
        assert!(preferences.boards);
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
        assert!(app.preferences.boards, "on by default");
        assert!(app.banner.is_none());

        std::fs::write(&path, "boards = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `boards`.")
        );
        assert!(app.preferences.boards);
        assert!(!app.preferences.settings.boards);

        // A settings warning is still returned, with the notice beside it.
        std::fs::write(&path, "boards = off\nmystery = 1\n").unwrap();
        let warning = app.reload_settings(false).unwrap();
        assert!(warning.contains("mystery") && !warning.contains("Restart"));
        let banner = app.banner.clone().unwrap();
        assert!(
            banner.starts_with(&warning) && banner.ends_with("Restart Wren to apply `boards`.")
        );

        // Back at the launch value there is nothing to wait for.
        std::fs::write(&path, "boards = on\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(app.banner.is_none());

        // A launch with the setting off runs with it, and removing the line
        // waits for the default.
        std::fs::write(&path, "boards = off\n").unwrap();
        let (mut app, _) = App::with_config(options(), None, Some(path.clone()));
        assert!(!app.preferences.boards);
        assert!(app.banner.is_none());
        std::fs::write(&path, "").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `boards`.")
        );
        assert!(!app.preferences.boards);
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
            layer_of,
        );
        assert!(warnings.is_empty());
        let policy = key_policy(&keymap, true, false);
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
        let plain = key_policy(&keymap, false, false);
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
