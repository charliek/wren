//! User settings (`config.conf`): theme, keybindings, editor options.

use std::path::PathBuf;

use iced::theme::Mode as SystemMode;
use iced::Task;
use wren_core::keybind::{self, Keymap, Platform};
use wren_core::settings::{self, Settings, ThemeChoice};
use wren_core::theme::{self as definitions, Appearance, Theme};

use super::commands::{Command, CommandId, Disposition, Gateway};
use super::palette::Mode;
use super::{focus_editor, workspace, App, Message};
use crate::keys::{Input, Nav};

/// Settings plus the derived keymap and the file they came from.
pub struct Preferences {
    pub settings: Settings,
    pub keymap: Keymap,
    pub path: Option<PathBuf>,
    /// The bytes last loaded, so focus-triggered reloads skip unchanged files.
    loaded: Option<String>,
}

impl Preferences {
    /// Load from `path`; `None` means defaults with nowhere to persist.
    pub fn load(path: Option<PathBuf>) -> Self {
        let loaded = path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let settings = loaded
            .as_deref()
            .map_or_else(Settings::default, Settings::parse);
        let mut preferences = Self {
            keymap: Keymap::default(),
            settings,
            path,
            loaded,
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
        let Some(path) = &self.path else {
            return false;
        };
        let text = std::fs::read_to_string(path).ok();
        if !force && text == self.loaded {
            return false;
        }
        self.settings = text
            .as_deref()
            .map_or_else(Settings::default, Settings::parse);
        self.loaded = text;
        self.rebuild_keymap();
        true
    }

    /// The theme to show for the system `mode`, plus a warning when the
    /// configured theme id is unknown.
    pub fn theme(&self, mode: SystemMode) -> (Theme, Option<String>) {
        let appearance = match mode {
            SystemMode::Light => Appearance::Light,
            SystemMode::Dark | SystemMode::None => Appearance::Dark,
        };
        let id = self.settings.theme.resolve(appearance);
        match definitions::builtin(id) {
            Some(theme) => (theme, None),
            None => (
                definitions::for_appearance(appearance),
                Some(format!("Unknown theme {id:?}; using the default.")),
            ),
        }
    }

    fn warning(&self) -> Option<String> {
        let mut warnings = self.settings.warnings.clone();
        if let Some(warning) = self.theme(SystemMode::Dark).1 {
            warnings.push(warning);
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
    /// The displayed shortcut for a command key, in platform notation.
    pub(super) fn shortcut_label(&self, action: &str) -> Option<String> {
        self.preferences.keymap.label(action, Platform::current())
    }

    /// Recompute the theme from the explicit override, settings, and system.
    pub(super) fn apply_theme(&mut self) {
        let theme = self
            .theme_override
            .unwrap_or_else(|| self.preferences.theme(self.system_mode).0);
        if theme != self.definition {
            self.definition = theme;
            self.toolkit_theme = crate::theme::from_definition(theme);
        }
    }

    /// Re-read `config.conf` and apply it. Returns any settings warning.
    pub(super) fn reload_settings(&mut self, force: bool) -> Option<String> {
        if !self.preferences.reload(force) {
            return None;
        }
        self.apply_preferences();
        let warning = self.preferences.warning();
        if let Some(warning) = &warning {
            self.banner = Some(warning.clone());
            self.banner_kind = workspace::WarningKind::General;
        }
        warning
    }

    pub(super) fn apply_preferences(&mut self) {
        self.apply_theme();
        let settings = &self.preferences.settings;
        self.editor.set_font_size(settings.editor_font_size);
        self.editor.set_line_numbers(settings.line_numbers);
    }

    pub(super) fn startup_settings_warning(&self) -> Option<String> {
        self.preferences.warning()
    }

    /// Persist a theme choice from the picker; `None` follows the system.
    pub(super) fn choose_theme(&mut self, id: Option<&'static str>) -> Gateway {
        let choice = id.map_or_else(ThemeChoice::system, |id| ThemeChoice::Fixed(id.into()));
        self.preferences.settings.theme = choice.clone();
        self.theme_override = None;
        self.apply_theme();
        self.palette = None;
        let warning = self.preferences.path.clone().and_then(|path| {
            settings::write_value(&path, "theme", &choice.value())
                .map(|()| {
                    self.preferences.loaded = std::fs::read_to_string(&path).ok();
                })
                .err()
                .map(|error| format!("Could not save the theme to {}: {error}", path.display()))
        });
        if let Some(warning) = &warning {
            self.banner = Some(warning.clone());
            self.banner_kind = workspace::WarningKind::General;
        }
        self.finish_action(
            if warning.is_some() {
                Disposition::CommittedWarning
            } else {
                Disposition::Completed
            },
            warning,
            focus_editor(),
        )
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
        CommandId::parse(key).map_or_else(Task::none, |id| self.run_shortcut(id))
    }

    /// Route a key press: overlays first, then the keymap.
    pub(super) fn key_input(&mut self, input: Input) -> Task<Message> {
        let accel = match input {
            Input::Nav(Nav::Escape) => return self.update(Message::Escape),
            Input::Nav(nav) => {
                return match (&self.palette, nav) {
                    (Some(_), Nav::Up) => self.palette_step(-1).task,
                    (Some(_), Nav::Down) => self.palette_step(1).task,
                    (Some(_), Nav::Enter) => self.activate_palette(None).task,
                    _ => Task::none(),
                }
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
        self.run_shortcut(id)
    }

    fn run_shortcut(&mut self, id: CommandId) -> Task<Message> {
        // Quitting stays available everywhere. Otherwise an open dialog or
        // palette owns the keyboard: only the palette shortcut (to close it)
        // gets through.
        if id != CommandId::Quit {
            if self.dialog.is_some() {
                return Task::none();
            }
            if self.palette.is_some() {
                return if id == CommandId::CommandPalette {
                    self.close_palette().task
                } else {
                    Task::none()
                };
            }
        }
        if id == CommandId::CommandPalette {
            return self.open_palette(Mode::Commands).task;
        }
        self.dispatch(Command::Run(id)).task
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
