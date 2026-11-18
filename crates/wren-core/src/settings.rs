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
/// The user's spelling word list, beside the settings file.
pub const WORDS_FILE_NAME: &str = "words.txt";
const MAX_BYTES: u64 = 1024 * 1024;
pub const MIN_FONT_SIZE: f32 = 9.0;
pub const MAX_FONT_SIZE: f32 = 32.0;
pub const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Row height as a multiple of the text size: 24 px rows at 14 pt.
pub const DEFAULT_LINE_HEIGHT: f32 = 1.7;
pub const MIN_LINE_HEIGHT: f32 = 1.2;
pub const MAX_LINE_HEIGHT: f32 = 2.2;
/// Characters of text before a line wraps; focus mode uses it when
/// `editor-wrap-column` is `none`.
pub const DEFAULT_WRAP_COLUMN: u16 = 100;
pub const MIN_WRAP_COLUMN: u16 = 20;
pub const MAX_WRAP_COLUMN: u16 = 400;

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

/// The editor's text engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditorEngine {
    /// iced's own text editor.
    #[default]
    Classic,
    /// Wren's own cosmic-text editor.
    Advanced,
}

impl EditorEngine {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "classic" => Some(Self::Classic),
            "advanced" => Some(Self::Advanced),
            _ => None,
        }
    }

    /// The config-file spelling.
    pub fn value(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Advanced => "advanced",
        }
    }
}

/// When a board's tool dock shows each button's key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeyBadges {
    /// While Space is held on the board.
    #[default]
    Hold,
    On,
    Off,
}

impl KeyBadges {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "hold" => Some(Self::Hold),
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// The config-file spelling.
    pub fn value(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub editor_font_size: f32,
    /// A font family name; `None` uses the bundled JetBrains Mono.
    pub editor_font_family: Option<String>,
    /// Row height as a multiple of the text size.
    pub editor_line_height: f32,
    /// Characters of text before a line wraps; `None` wraps at the pane's
    /// edge outside focus mode.
    pub editor_wrap_column: Option<u16>,
    pub line_numbers: bool,
    pub editor_engine: EditorEngine,
    /// The Markdown-aware Enter, Tab, Backspace, and Home.
    pub markdown_assist: bool,
    /// Spell check, which the advanced engine alone carries out.
    pub spell_check: bool,
    /// Whether board notes are listed and opened, read when a workspace
    /// opens.
    pub boards: bool,
    /// Whether a focused board's footer names the keys that fit what it
    /// shows.
    pub board_hints: bool,
    /// Whether a board shows its tool dock.
    pub board_toolbar: bool,
    /// When the tool dock's buttons show their keys.
    pub board_key_badges: KeyBadges,
    /// `(trigger, action)` in file order; later lines win.
    pub keybinds: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            editor_font_size: DEFAULT_FONT_SIZE,
            editor_font_family: None,
            editor_line_height: DEFAULT_LINE_HEIGHT,
            editor_wrap_column: Some(DEFAULT_WRAP_COLUMN),
            line_numbers: true,
            editor_engine: EditorEngine::default(),
            markdown_assist: true,
            spell_check: true,
            boards: true,
            board_hints: true,
            board_toolbar: true,
            board_key_badges: KeyBadges::Hold,
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
                "editor-font-family" => match value.trim() {
                    "" => invalid(),
                    family => settings.editor_font_family = Some(family.into()),
                },
                "editor-line-height" => match value.parse::<f32>() {
                    Ok(ratio) if (MIN_LINE_HEIGHT..=MAX_LINE_HEIGHT).contains(&ratio) => {
                        settings.editor_line_height = ratio
                    }
                    _ => invalid(),
                },
                "editor-wrap-column" => match value.trim().to_ascii_lowercase().as_str() {
                    "none" => settings.editor_wrap_column = None,
                    number => match number.parse::<u16>() {
                        Ok(column) if (MIN_WRAP_COLUMN..=MAX_WRAP_COLUMN).contains(&column) => {
                            settings.editor_wrap_column = Some(column)
                        }
                        _ => invalid(),
                    },
                },
                "line-numbers" => match parse_bool(value) {
                    Some(enabled) => settings.line_numbers = enabled,
                    None => invalid(),
                },
                "editor-engine" => match EditorEngine::parse(value) {
                    Some(engine) => settings.editor_engine = engine,
                    None => invalid(),
                },
                "markdown-assist" => match parse_bool(value) {
                    Some(enabled) => settings.markdown_assist = enabled,
                    None => invalid(),
                },
                "spell-check" => match parse_bool(value) {
                    Some(enabled) => settings.spell_check = enabled,
                    None => invalid(),
                },
                "boards" => match parse_bool(value) {
                    Some(enabled) => settings.boards = enabled,
                    None => invalid(),
                },
                "board-hints" => match parse_bool(value) {
                    Some(enabled) => settings.board_hints = enabled,
                    None => invalid(),
                },
                "board-toolbar" => match parse_bool(value) {
                    Some(enabled) => settings.board_toolbar = enabled,
                    None => invalid(),
                },
                "board-key-badges" => match KeyBadges::parse(value) {
                    Some(badges) => settings.board_key_badges = badges,
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

/// Read the settings text; `None` when the file does not exist. Anything but
/// a regular file (a symlink to one is fine), files over 1 MiB and invalid
/// UTF-8 are errors. The file is opened without blocking, so a FIFO with no
/// writer cannot hang the caller.
pub fn read_text(path: &Path) -> io::Result<Option<String>> {
    use rustix::fs::{Mode, OFlags};
    use std::io::Read;

    let too_large = || io::Error::other(format!("larger than {} KiB", MAX_BYTES / 1024));
    let file = match rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(file) => fs::File::from(file),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    if metadata.len() > MAX_BYTES {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_large());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
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

/// The spelling word list for the settings file at `settings`: `words.txt`
/// in the same directory.
pub fn words_path(settings: &Path) -> PathBuf {
    settings.with_file_name(WORDS_FILE_NAME)
}

/// What a new `words.txt` holds: comments only, so it adds no word.
pub const WORDS_TEMPLATE: &str = "\
# Wren's spelling word list: one word per line, each accepted as correctly
# spelled. A word in lower case is also accepted capitalised and in capitals;
# one with a capital letter is accepted exactly as written.
# Lines starting with # are comments. Changes apply when Wren regains focus
# or on “Reload settings”.
";

/// Create the word list at `path`, with only [`WORDS_TEMPLATE`], if there is
/// no such file yet. An existing file is never written.
pub fn ensure_words_exist(path: &Path) -> io::Result<()> {
    if read_text(path)?.is_some() {
        return Ok(());
    }
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let created = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path);
    match created {
        Ok(mut file) => file.write_all(WORDS_TEMPLATE.as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
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

# Editor text size in points (9–32). Cmd/Alt + plus, minus, and 0 change it.
# editor-font-size = 14

# Editor font family: any installed font. “Change editor font…” lists the
# monospace ones.
# editor-font-family = \"JetBrains Mono\"

# Editor row height as a multiple of the text size (1.2–2.2).
# editor-line-height = 1.7

# Characters of text before a line wraps (20–400), measured in the width of
# the font's \"0\"; none wraps at the pane's edge. The text starts at the left
# of the pane; focus mode centres it, at 100 characters when this is none.
# editor-wrap-column = 100

# Show the line-number gutter.
# line-numbers = true

# Editor engine: classic (the current editor) or advanced (Wren's new
# editor engine, under evaluation).
# editor-engine = classic

# Markdown assist: Enter continues lists and quotes, Tab nests list items,
# Backspace removes markup, and Home stops at the text after the markup.
# Off, the Markdown commands (toggle task, bold, move line, ...) are disabled.
# markdown-assist = on

# Spell check: misspelled words are underlined, and Alt+. (Cmd+. on macOS)
# or a right-click offers corrections. It needs editor-engine = advanced.
# Your own words go in words.txt, beside this file, one per line.
# spell-check = on

# Boards: `.wren-board` notes of shapes and connectors, drawn on a canvas.
# Off, boards are neither listed nor opened, and their files are left alone.
# Applies when Wren restarts.
# boards = on

# Board hints: the footer under a board names the keys that fit what is
# selected or armed.
# board-hints = on

# Board toolbar: a dock of tools at the bottom of a board, for the mouse.
# board-toolbar = on

# Board key badges: when the dock's buttons show their keys. hold shows
# them while Space is held on the board; on always; off never.
# board-key-badges = hold

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
             editor-font-family = \"Fira Code\"\n\
             line-numbers = off\n\
             keybind = ctrl+shift+p = palette.commands\n\
             keybind = \"alt+k = unbind\"\n\
             editor-font-size = 100\n\
             mystery = 1\n\
             no equals sign\n\
             editor-font-family = \"\"\n",
        );
        assert_eq!(
            settings.theme,
            ThemeChoice::Adaptive {
                light: "solarized-light".into(),
                dark: "nord".into()
            }
        );
        assert_eq!(settings.editor_font_size, 16.0);
        assert_eq!(settings.editor_font_family.as_deref(), Some("Fira Code"));
        assert!(!settings.line_numbers);
        assert_eq!(
            settings.keybinds,
            [
                ("ctrl+shift+p".into(), "palette.commands".into()),
                ("alt+k".into(), "unbind".into())
            ]
        );
        assert_eq!(settings.warnings.len(), 4, "{:?}", settings.warnings);
        assert!(settings.warnings[0].contains("line 8"));
        assert!(settings.warnings[1].contains("mystery"));
        assert!(settings.warnings[2].contains("line 10"));
        assert!(settings.warnings[3].contains("editor-font-family"));
    }

    #[test]
    fn editor_engine_parses_either_engine_and_warns_on_others() {
        assert_eq!(Settings::default().editor_engine, EditorEngine::Classic);
        let settings = Settings::parse("editor-engine = advanced\n");
        assert_eq!(settings.editor_engine, EditorEngine::Advanced);
        assert!(settings.warnings.is_empty());
        let settings = Settings::parse("editor-engine = Advanced\neditor-engine = \"classic\"\n");
        assert_eq!(settings.editor_engine, EditorEngine::Classic);
        let settings = Settings::parse("editor-engine = advanced\neditor-engine = gpui\n");
        assert_eq!(settings.editor_engine, EditorEngine::Advanced);
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("editor-engine"));
        for engine in [EditorEngine::Classic, EditorEngine::Advanced] {
            assert_eq!(EditorEngine::parse(engine.value()), Some(engine));
        }
    }

    #[test]
    fn editor_line_height_accepts_ratios_in_range_and_warns_otherwise() {
        assert_eq!(Settings::default().editor_line_height, DEFAULT_LINE_HEIGHT);
        for (value, ratio) in [("1.2", 1.2), ("2.2", 2.2), ("1.45", 1.45), ("2", 2.0)] {
            let settings = Settings::parse(&format!("editor-line-height = {value}\n"));
            assert_eq!(settings.editor_line_height, ratio, "{value}");
            assert!(settings.warnings.is_empty());
        }
        for value in ["1.19", "2.3", "24px", "", "NaN"] {
            let settings = Settings::parse(&format!(
                "editor-line-height = 1.5\neditor-line-height = {value}\n"
            ));
            assert_eq!(settings.editor_line_height, 1.5, "{value}");
            assert_eq!(settings.warnings.len(), 1, "{value}");
            assert!(settings.warnings[0].contains("editor-line-height"));
        }
    }

    #[test]
    fn editor_wrap_column_takes_characters_in_range_or_none() {
        assert_eq!(
            Settings::default().editor_wrap_column,
            Some(DEFAULT_WRAP_COLUMN)
        );
        for (value, column) in [
            ("80", Some(80)),
            ("20", Some(20)),
            ("400", Some(400)),
            ("none", None),
            ("None", None),
        ] {
            let settings = Settings::parse(&format!("editor-wrap-column = {value}\n"));
            assert_eq!(settings.editor_wrap_column, column, "{value}");
            assert!(settings.warnings.is_empty());
        }
        for value in ["19", "401", "80ch", "80.5", "", "-1"] {
            let settings = Settings::parse(&format!(
                "editor-wrap-column = none\neditor-wrap-column = {value}\n"
            ));
            assert_eq!(settings.editor_wrap_column, None, "{value}");
            assert_eq!(settings.warnings.len(), 1, "{value}");
            assert!(settings.warnings[0].contains("editor-wrap-column"));
        }
    }

    #[test]
    fn markdown_assist_defaults_on_and_parses_booleans() {
        assert!(Settings::default().markdown_assist);
        for (value, enabled) in [
            ("off", false),
            ("on", true),
            ("false", false),
            ("YES", true),
        ] {
            let settings = Settings::parse(&format!("markdown-assist = {value}\n"));
            assert_eq!(settings.markdown_assist, enabled, "{value}");
            assert!(settings.warnings.is_empty());
        }
        let settings = Settings::parse("markdown-assist = off\nmarkdown-assist = sometimes\n");
        assert!(!settings.markdown_assist);
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("markdown-assist"));
    }

    #[test]
    fn spell_check_defaults_on_and_parses_booleans() {
        assert!(Settings::default().spell_check);
        for (value, enabled) in [("off", false), ("on", true), ("no", false), ("TRUE", true)] {
            let settings = Settings::parse(&format!("spell-check = {value}\n"));
            assert_eq!(settings.spell_check, enabled, "{value}");
            assert!(settings.warnings.is_empty());
        }
        let settings = Settings::parse("spell-check = off\nspell-check = maybe\n");
        assert!(!settings.spell_check);
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("spell-check"));
        assert_eq!(
            with_value("theme = nord\n", "spell-check", "off"),
            "theme = nord\nspell-check = off\n"
        );
    }

    #[test]
    fn the_word_list_sits_beside_the_settings_file() {
        let settings = Path::new("/home/someone/.config/wren/config.conf");
        assert_eq!(
            words_path(settings),
            Path::new("/home/someone/.config/wren/words.txt")
        );
        assert_eq!(
            words_path(Path::new("elsewhere.conf")),
            Path::new("words.txt")
        );
    }

    #[test]
    fn a_new_word_list_holds_only_comments_and_an_existing_one_is_kept() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/words.txt");
        ensure_words_exist(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), WORDS_TEMPLATE);
        assert!(WORDS_TEMPLATE
            .lines()
            .all(|line| line.trim_start().starts_with('#')));
        fs::write(&path, "wren\n").unwrap();
        ensure_words_exist(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "wren\n");
        let folder = directory.path().join("folder.txt");
        fs::create_dir(&folder).unwrap();
        assert!(ensure_words_exist(&folder).is_err());
    }

    #[test]
    fn boards_default_on_and_parse_booleans() {
        assert!(Settings::default().boards);
        assert!(Settings::parse("").boards);
        for (value, enabled) in [
            ("on", true),
            ("yes", true),
            ("true", true),
            ("ON", true),
            ("\"on\"", true),
            ("off", false),
            ("no", false),
            ("false", false),
        ] {
            let settings = Settings::parse(&format!("boards = {value}\n"));
            assert_eq!(settings.boards, enabled, "{value}");
            assert!(settings.warnings.is_empty(), "{value}");
        }
        for bad in ["sometimes", "1", ""] {
            let settings = Settings::parse(&format!("boards = off\nboards = {bad}\n"));
            assert!(!settings.boards, "{bad:?} keeps the earlier value");
            assert_eq!(settings.warnings.len(), 1, "{bad:?}");
            assert!(settings.warnings[0].contains("line 2: invalid boards value"));
        }
        assert!(TEMPLATE.contains("\n# boards = on\n"));
        let off = with_value(TEMPLATE, "boards", "off");
        assert!(!Settings::parse(&off).boards);
        assert!(off.contains("\n# boards = on\n") && off.ends_with("boards = off\n"));
    }

    #[test]
    fn board_hints_default_on_and_parse_booleans() {
        assert!(Settings::default().board_hints);
        assert!(Settings::parse("").board_hints);
        for (value, enabled) in [
            ("on", true),
            ("yes", true),
            ("true", true),
            ("ON", true),
            ("\"on\"", true),
            ("off", false),
            ("no", false),
            ("false", false),
        ] {
            let settings = Settings::parse(&format!("board-hints = {value}\n"));
            assert_eq!(settings.board_hints, enabled, "{value}");
            assert!(settings.boards, "{value}");
            assert!(settings.warnings.is_empty(), "{value}");
        }
        for bad in ["sometimes", "1", ""] {
            let settings = Settings::parse(&format!("board-hints = off\nboard-hints = {bad}\n"));
            assert!(!settings.board_hints, "{bad:?} keeps the earlier value");
            assert_eq!(settings.warnings.len(), 1, "{bad:?}");
            assert!(settings.warnings[0].contains("line 2: invalid board-hints value"));
        }
        assert!(TEMPLATE.contains("\n# board-hints = on\n"));
        let off = with_value(TEMPLATE, "board-hints", "off");
        assert!(!Settings::parse(&off).board_hints);
        assert!(off.contains("\n# board-hints = on\n") && off.ends_with("board-hints = off\n"));
    }

    #[test]
    fn board_toolbar_defaults_on_and_parses_booleans() {
        assert!(Settings::default().board_toolbar);
        assert!(Settings::parse("").board_toolbar);
        for (value, enabled) in [
            ("on", true),
            ("yes", true),
            ("off", false),
            ("FALSE", false),
        ] {
            let settings = Settings::parse(&format!("board-toolbar = {value}\n"));
            assert_eq!(settings.board_toolbar, enabled, "{value}");
            assert!(settings.warnings.is_empty(), "{value}");
        }
        let settings = Settings::parse("board-toolbar = off\nboard-toolbar = hold\n");
        assert!(!settings.board_toolbar, "a bad value keeps the earlier one");
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("line 2: invalid board-toolbar value"));
        assert!(TEMPLATE.contains("\n# board-toolbar = on\n"));
        let off = with_value(TEMPLATE, "board-toolbar", "off");
        assert!(!Settings::parse(&off).board_toolbar);
        assert!(off.contains("\n# board-toolbar = on\n") && off.ends_with("board-toolbar = off\n"));
    }

    #[test]
    fn board_key_badges_default_to_hold_and_parse_three_values() {
        assert_eq!(Settings::default().board_key_badges, KeyBadges::Hold);
        assert_eq!(Settings::parse("").board_key_badges, KeyBadges::Hold);
        for (value, badges) in [
            ("hold", KeyBadges::Hold),
            ("on", KeyBadges::On),
            ("OFF", KeyBadges::Off),
            ("\"on\"", KeyBadges::On),
        ] {
            let settings = Settings::parse(&format!("board-key-badges = {value}\n"));
            assert_eq!(settings.board_key_badges, badges, "{value}");
            assert!(settings.warnings.is_empty(), "{value}");
        }
        for badges in [KeyBadges::Hold, KeyBadges::On, KeyBadges::Off] {
            assert_eq!(KeyBadges::parse(badges.value()), Some(badges));
        }
        // `yes` and `true` are not among them: hold is neither.
        for bad in ["yes", "true", "always", ""] {
            let settings = Settings::parse(&format!(
                "board-key-badges = on\nboard-key-badges = {bad}\n"
            ));
            assert_eq!(settings.board_key_badges, KeyBadges::On, "{bad:?}");
            assert_eq!(settings.warnings.len(), 1, "{bad:?}");
            assert!(settings.warnings[0].contains("line 2: invalid board-key-badges value"));
        }
        assert!(TEMPLATE.contains("\n# board-key-badges = hold\n"));
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
    fn a_fifo_or_a_directory_is_an_error_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("config.conf");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // A FIFO with no writer would block a plain read for good.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(read_text(&fifo).is_err()));
        let read = receiver.recv_timeout(std::time::Duration::from_secs(3));
        assert_eq!(read, Ok(true), "still blocked after 3 s, or read");
        assert!(read_text(directory.path()).is_err());
        // A symlink to a regular file still reads.
        let target = directory.path().join("real.conf");
        fs::write(&target, "theme = nord\n").unwrap();
        let link = directory.path().join("linked.conf");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(read_text(&link).unwrap().as_deref(), Some("theme = nord\n"));
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
