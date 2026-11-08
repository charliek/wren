//! User settings from `config.conf`.
//!
//! The file is user-owned plain text in Ghostty/Roost syntax: one
//! `key = value` per line, `#` comments, and repeatable `keybind` lines.
//! It lives at `$XDG_CONFIG_HOME/wren/config.conf` (`~/.config/wren/` on
//! both macOS and Linux, as Roost does) unless `WREN_CONFIG` names a file.
//! A missing file means defaults. Unknown keys are warned about, never fatal,
//! and Wren only rewrites the single line of a setting it changes.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "config.conf";
const MAX_BYTES: u64 = 1024 * 1024;
pub const MIN_FONT_SIZE: f32 = 9.0;
pub const MAX_FONT_SIZE: f32 = 32.0;
pub const DEFAULT_FONT_SIZE: f32 = 14.0;

/// Which theme to show: a fixed theme, or one per system appearance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeChoice {
    Fixed(String),
    Adaptive { light: String, dark: String },
}

impl Default for ThemeChoice {
    fn default() -> Self {
        Self::system()
    }
}

impl ThemeChoice {
    pub fn system() -> Self {
        Self::Adaptive {
            light: crate::theme::LIGHT.id.into(),
            dark: crate::theme::DARK.id.into(),
        }
    }

    /// Parse `system`, `<id>`, or `light:<id>,dark:<id>` (either order).
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("system") {
            return Some(Self::system());
        }
        if !value.contains(':') {
            return (!value.is_empty() && !value.contains(',')).then(|| Self::Fixed(value.into()));
        }
        let (mut light, mut dark) = (None, None);
        for part in value.split(',') {
            match part.trim().split_once(':')? {
                ("light", id) if !id.trim().is_empty() => light = Some(id.trim().to_string()),
                ("dark", id) if !id.trim().is_empty() => dark = Some(id.trim().to_string()),
                _ => return None,
            }
        }
        Some(Self::Adaptive {
            light: light?,
            dark: dark?,
        })
    }

    pub fn resolve(&self, appearance: crate::theme::Appearance) -> &str {
        match (self, appearance) {
            (Self::Fixed(id), _) => id,
            (Self::Adaptive { light, .. }, crate::theme::Appearance::Light) => light,
            (Self::Adaptive { dark, .. }, crate::theme::Appearance::Dark) => dark,
        }
    }

    /// The config-file spelling.
    pub fn value(&self) -> String {
        match self {
            Self::Fixed(id) => id.clone(),
            choice if *choice == Self::system() => "system".into(),
            Self::Adaptive { light, dark } => format!("light:{light},dark:{dark}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub editor_font_size: f32,
    pub line_numbers: bool,
    /// `(trigger, action)` in file order; later lines win.
    pub keybinds: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            editor_font_size: DEFAULT_FONT_SIZE,
            line_numbers: true,
            keybinds: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

/// Split a line into `(key, value)`, or `None` for blanks and comments.
fn entry(line: &str) -> Option<Result<(&str, &str), ()>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    Some(
        line.split_once('=')
            .map(|(key, value)| (key.trim(), unquote(value)))
            .ok_or(()),
    )
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            let Some(entry) = entry(line) else { continue };
            let Ok((key, value)) = entry else {
                settings
                    .warnings
                    .push(format!("line {number}: expected `key = value`"));
                continue;
            };
            let mut invalid = || {
                settings
                    .warnings
                    .push(format!("line {number}: invalid {key} value {value:?}"))
            };
            match key {
                "theme" => match ThemeChoice::parse(value) {
                    Some(theme) => settings.theme = theme,
                    None => invalid(),
                },
                "editor-font-size" => match value.parse::<f32>() {
                    Ok(size) if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size) => {
                        settings.editor_font_size = size
                    }
                    _ => invalid(),
                },
                "line-numbers" => match parse_bool(value) {
                    Some(enabled) => settings.line_numbers = enabled,
                    None => invalid(),
                },
                "keybind" => match value.rsplit_once('=') {
                    Some((trigger, action)) if !trigger.trim().is_empty() => settings
                        .keybinds
                        .push((trigger.trim().into(), action.trim().into())),
                    _ => invalid(),
                },
                _ => settings
                    .warnings
                    .push(format!("line {number}: unknown setting {key:?}")),
            }
        }
        settings
    }

    /// Load from `path`; a missing file yields defaults.
    pub fn load(path: &Path) -> Self {
        match read_text(path) {
            Ok(Some(text)) => Self::parse(&text),
            Ok(None) => Self::default(),
            Err(error) => Self {
                warnings: vec![format!("{}: {error}", path.display())],
                ..Self::default()
            },
        }
    }
}

/// Read the settings text; `None` when the file does not exist. Files over
/// 1 MiB and invalid UTF-8 are errors.
pub fn read_text(path: &Path) -> io::Result<Option<String>> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_BYTES => Err(io::Error::other(format!(
            "larger than {} KiB",
            MAX_BYTES / 1024
        ))),
        Ok(_) => fs::read_to_string(path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The settings file location: `WREN_CONFIG`, else the XDG config directory.
pub fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WREN_CONFIG").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("wren").join(FILE_NAME))
}

/// Replace the last active `key = …` line, or append one, keeping every
/// other line (comments, keybinds, unknown keys) byte-for-byte.
pub fn with_value(text: &str, key: &str, value: &str) -> String {
    let replacement = format!("{key} = {value}");
    let mut lines: Vec<&str> = text.lines().collect();
    let target = lines
        .iter()
        .rposition(|line| matches!(entry(line), Some(Ok((found, _))) if found == key));
    let mut out = String::with_capacity(text.len() + replacement.len() + 1);
    match target {
        Some(index) => {
            lines[index] = &replacement;
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
        }
        None => {
            out.push_str(text);
            if !text.is_empty() && !text.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&replacement);
            out.push('\n');
        }
    }
    out
}

/// Set one setting in the file at `path`, creating it (and its directory)
/// with the commented template when missing. Publishes atomically.
pub fn write_value(path: &Path, key: &str, value: &str) -> io::Result<()> {
    let current = read_text(path)?.unwrap_or_else(|| TEMPLATE.to_string());
    publish(path, &with_value(&current, key, value))
}

/// Create the file with the commented template if it does not exist yet.
pub fn ensure_exists(path: &Path) -> io::Result<()> {
    match read_text(path)? {
        Some(_) => Ok(()),
        None => publish(path, TEMPLATE),
    }
}

/// Atomically replace the file. A symlinked `config.conf` (for example into
/// a dotfiles repository) is written through, and existing permissions are
/// kept.
fn publish(path: &Path, text: &str) -> io::Result<()> {
    let path = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("settings path has no parent directory"))?;
    fs::create_dir_all(directory)?;
    let (temporary, mut file) = tempfile_in(directory)?;
    let written = (|| {
        if let Ok(metadata) = fs::metadata(&path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn tempfile_in(directory: &Path) -> io::Result<(PathBuf, fs::File)> {
    for attempt in 0..100u32 {
        let path = directory.join(format!(".{FILE_NAME}.{}.{attempt}.tmp", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(
        "could not create a temporary settings file",
    ))
}

pub const TEMPLATE: &str = "\
# Wren settings. Lines are `key = value`; `#` starts a comment.
# Changes apply when Wren regains focus or on “Reload settings”.

# Theme: system, a theme id, or light:<id>,dark:<id>.
# Built-in themes: wren-dark, wren-light, nord, gruvbox-dark, solarized-light,
# rose-pine, everforest-dark, tokyo-night, dracula, catppuccin-mocha,
# catppuccin-macchiato.
# theme = system

# Editor text size in points (9–32).
# editor-font-size = 14

# Show the line-number gutter.
# line-numbers = true

# Keybindings: keybind = <trigger> = <action>. Later lines win; use `unbind`
# to remove a default. Modifiers: ctrl, alt, shift, super (cmd).
# keybind = ctrl+shift+p = palette.commands
# keybind = alt+k = unbind
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Appearance;

    #[test]
    fn defaults_follow_the_system_appearance() {
        let settings = Settings::parse("");
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.theme.resolve(Appearance::Dark), "wren-dark");
        assert_eq!(settings.theme.resolve(Appearance::Light), "wren-light");
        assert_eq!(settings.theme.value(), "system");
    }

    #[test]
    fn parses_every_setting_and_warns_on_bad_lines() {
        let settings = Settings::parse(
            "# comment\n\
             theme = light:solarized-light, dark:nord\n\
             editor-font-size = 16\n\
             line-numbers = off\n\
             keybind = ctrl+shift+p = palette.commands\n\
             keybind = \"alt+k = unbind\"\n\
             editor-font-size = 100\n\
             mystery = 1\n\
             no equals sign\n",
        );
        assert_eq!(
            settings.theme,
            ThemeChoice::Adaptive {
                light: "solarized-light".into(),
                dark: "nord".into()
            }
        );
        assert_eq!(settings.editor_font_size, 16.0);
        assert!(!settings.line_numbers);
        assert_eq!(
            settings.keybinds,
            [
                ("ctrl+shift+p".into(), "palette.commands".into()),
                ("alt+k".into(), "unbind".into())
            ]
        );
        assert_eq!(settings.warnings.len(), 3, "{:?}", settings.warnings);
        assert!(settings.warnings[0].contains("line 7"));
        assert!(settings.warnings[1].contains("mystery"));
        assert!(settings.warnings[2].contains("line 9"));
    }

    #[test]
    fn theme_choice_round_trips() {
        for value in ["system", "nord", "light:wren-light,dark:nord"] {
            assert_eq!(ThemeChoice::parse(value).unwrap().value(), value);
        }
        for invalid in ["", "light:a", "light:a,dusk:b", "a,b", "dark:"] {
            assert_eq!(ThemeChoice::parse(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn with_value_rewrites_only_the_last_active_line() {
        let text = "# theme = old\ntheme = a\nkeybind = alt+k = unbind\ntheme = b\n";
        assert_eq!(
            with_value(text, "theme", "nord"),
            "# theme = old\ntheme = a\nkeybind = alt+k = unbind\ntheme = nord\n"
        );
        assert_eq!(
            with_value("x = 1", "theme", "nord"),
            "x = 1\ntheme = nord\n"
        );
        assert_eq!(with_value("", "theme", "nord"), "theme = nord\n");
    }

    #[test]
    fn write_value_creates_from_template_and_preserves_user_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/config.conf");
        write_value(&path, "theme", "nord").unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.starts_with("# Wren settings."));
        assert!(written.ends_with("theme = nord\n"));
        fs::write(&path, "keybind = alt+k = unbind\ntheme = nord\n").unwrap();
        write_value(&path, "theme", "system").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "keybind = alt+k = unbind\ntheme = system\n"
        );
        assert_eq!(Settings::load(&path).theme, ThemeChoice::system());
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["config.conf"]);
    }

    #[cfg(unix)]
    #[test]
    fn writes_go_through_symlinks_and_keep_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("dotfiles/wren.conf");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "editor-font-size = 15\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("config.conf");
        symlink(&target, &link).unwrap();
        write_value(&link, "theme", "nord").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "editor-font-size = 15\ntheme = nord\n"
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn oversized_and_non_utf8_files_are_errors_not_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.conf");
        fs::write(&path, b"theme = \xff\n").unwrap();
        assert!(read_text(&path).is_err());
        assert_eq!(Settings::load(&path).warnings.len(), 1);
        fs::write(&path, vec![b'#'; MAX_BYTES as usize + 1]).unwrap();
        assert!(read_text(&path).is_err());
    }

    #[test]
    fn missing_file_is_defaults_and_template_parses_cleanly() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            Settings::load(&directory.path().join("absent.conf")),
            Settings::default()
        );
        assert_eq!(Settings::parse(TEMPLATE), Settings::default());
        let path = directory.path().join("config.conf");
        ensure_exists(&path).unwrap();
        ensure_exists(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);
    }
}
