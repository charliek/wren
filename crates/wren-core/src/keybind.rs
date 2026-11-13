//! Keybinding triggers, platform defaults, and user overrides.
//!
//! Triggers use Ghostty/Roost syntax (`super+shift+p`, `alt+bracketleft`).
//! macOS uses Command as the app modifier; Linux uses Alt so Ctrl stays with
//! text editing (Ctrl+Z/C/V/A inside the editor and inputs). Actions are the
//! stable command keys (`navigation.jump`, `palette.commands`, …); this module
//! does not know the command table, so callers validate action names.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

/// A modifier set plus a canonical key name (`p`, `1`, `[`, `enter`, `left`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accel {
    pub mods: Mods,
    pub key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Linux,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Linux
        }
    }

    /// The modifier name app commands use by default.
    pub const fn primary(self) -> &'static str {
        match self {
            Self::Mac => "super",
            Self::Linux => "alt",
        }
    }
}

/// Action name that removes any binding on a trigger in the base layer.
pub const UNBIND: &str = "unbind";

/// Action name that removes a trigger's binding in the board layer.
pub const BOARD_UNBIND: &str = "board.unbind";

/// Which table a binding lives in. The base layer is the app's keymap, read
/// wherever the keyboard is. The board layer is read only by a focused
/// board, after the base layer, so it may bind bare keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Base,
    Board,
}

fn modifier(token: &str) -> Option<fn(&mut Mods)> {
    Some(match token {
        "shift" => |mods| mods.shift = true,
        "ctrl" | "control" => |mods| mods.ctrl = true,
        "alt" | "opt" | "option" => |mods| mods.alt = true,
        "super" | "cmd" | "command" | "meta" | "logo" => |mods| mods.logo = true,
        _ => return None,
    })
}

/// Canonical spelling for a key token, accepting common aliases.
pub fn canonical_key(token: &str) -> Option<String> {
    let lower = token.to_lowercase();
    let named = match lower.as_str() {
        "bracketleft" | "[" => "[",
        "bracketright" | "]" => "]",
        "backslash" | "\\" => "\\",
        "slash" | "/" => "/",
        "comma" | "," => ",",
        "period" | "." => ".",
        "semicolon" | ";" => ";",
        "apostrophe" | "quote" | "'" => "'",
        "grave" | "backquote" | "`" => "`",
        "minus" | "-" => "-",
        "equal" | "=" => "=",
        "plus" | "+" => "+",
        "enter" | "return" => "enter",
        "escape" | "esc" => "escape",
        "space" => "space",
        "tab" => "tab",
        "backspace" => "backspace",
        "delete" | "del" => "delete",
        "left" | "arrowleft" => "left",
        "right" | "arrowright" => "right",
        "up" | "arrowup" => "up",
        "down" | "arrowdown" => "down",
        "home" => "home",
        "end" => "end",
        "pageup" => "pageup",
        "pagedown" => "pagedown",
        _ => {
            let mut chars = lower.chars();
            return match (chars.next(), chars.next()) {
                (Some(character), None) if !character.is_control() => Some(lower),
                _ => {
                    let number = lower.strip_prefix('f')?.parse::<u8>().ok()?;
                    (1..=24).contains(&number).then_some(lower)
                }
            };
        }
    };
    Some(named.into())
}

pub fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number))
}

/// Parse a trigger such as `super+shift+p`. The last non-modifier token is
/// the key; `+` itself may be written as `plus`.
pub fn parse_trigger(trigger: &str) -> Option<Accel> {
    let mut mods = Mods::default();
    let mut key = None;
    for token in trigger.trim().split('+') {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() {
            return None;
        }
        match modifier(&token) {
            Some(apply) => apply(&mut mods),
            None if key.is_none() => key = Some(canonical_key(&token)?),
            None => return None,
        }
    }
    Some(Accel { mods, key: key? })
}

impl Accel {
    /// Whether a key press can produce this trigger. Without Ctrl, Alt, or
    /// Super a key types text (or moves the cursor), so only function keys
    /// may be bound bare or with Shift alone.
    pub fn is_bindable(&self) -> bool {
        self.mods.ctrl || self.mods.alt || self.mods.logo || is_function_key(&self.key)
    }

    /// Human-readable label in the platform's convention: `⌘⇧P` on macOS,
    /// `Alt+Shift+P` elsewhere.
    pub fn label(&self, platform: Platform) -> String {
        let key = match self.key.as_str() {
            "enter" => "↵".into(),
            "escape" => "Esc".into(),
            "left" => "←".into(),
            "right" => "→".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            "space" => "Space".into(),
            "tab" => "Tab".into(),
            "backspace" => "⌫".into(),
            "delete" => "Del".into(),
            other => other.to_uppercase(),
        };
        let Mods {
            ctrl,
            alt,
            shift,
            logo,
        } = self.mods;
        match platform {
            Platform::Mac => {
                let mut label = String::new();
                for (held, glyph) in [(ctrl, '⌃'), (alt, '⌥'), (shift, '⇧'), (logo, '⌘')] {
                    if held {
                        label.push(glyph);
                    }
                }
                label + &key
            }
            Platform::Linux => {
                let mut parts = Vec::new();
                for (held, name) in [
                    (ctrl, "Ctrl"),
                    (alt, "Alt"),
                    (shift, "Shift"),
                    (logo, "Super"),
                ] {
                    if held {
                        parts.push(name.to_string());
                    }
                }
                parts.push(key);
                parts.join("+")
            }
        }
    }
}

impl fmt::Display for Accel {
    /// The config-file spelling, so a displayed trigger can be pasted back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.mods.ctrl, "ctrl+"),
            (self.mods.alt, "alt+"),
            (self.mods.shift, "shift+"),
            (self.mods.logo, "super+"),
        ] {
            if held {
                f.write_str(name)?;
            }
        }
        f.write_str(match self.key.as_str() {
            "+" => "plus",
            other => other,
        })
    }
}

/// Default `(trigger, action)` pairs, in display-priority order: the first
/// trigger listed for an action is the one menus and buttons show.
pub fn default_bindings(platform: Platform) -> Vec<(String, &'static str)> {
    let p = platform.primary();
    let mut bindings = vec![
        (format!("{p}+shift+p"), "palette.commands"),
        (format!("{p}+k"), "palette.commands"),
        (format!("{p}+p"), "navigation.jump"),
        (format!("{p}+shift+f"), "navigation.search"),
        (format!("{p}+f"), "navigation.find"),
        (format!("{p}+shift+o"), "navigation.outline"),
        (format!("{p}+n"), "note.new"),
        (format!("{p}+shift+n"), "note.new_dated"),
        (format!("{p}+bracketleft"), "navigation.back"),
        (format!("{p}+bracketright"), "navigation.forward"),
        (format!("{p}+shift+bracketleft"), "navigation.column_left"),
        (format!("{p}+shift+bracketright"), "navigation.column_right"),
        (format!("{p}+r"), "note.rename"),
        (format!("{p}+y"), "view.focus"),
        // `+` is Shift+`=` on US layouts, and most people press it unshifted.
        // Toolkits report the shifted press as either key, so bind both.
        (format!("{p}+plus"), "font.increase"),
        (format!("{p}+equal"), "font.increase"),
        (format!("{p}+shift+equal"), "font.increase"),
        (format!("{p}+shift+plus"), "font.increase"),
        (format!("{p}+minus"), "font.decrease"),
        (format!("{p}+0"), "font.reset"),
    ];
    // Editor commands take the platform's text-formatting modifier, which is
    // Ctrl on Linux, where Alt belongs to the app.
    let e = match platform {
        Platform::Mac => "super",
        Platform::Linux => "ctrl",
    };
    bindings.extend([
        (format!("{e}+enter"), "markdown.toggle_task"),
        (format!("{e}+b"), "markdown.bold"),
        (format!("{e}+i"), "markdown.italic"),
        (format!("{e}+e"), "markdown.code"),
        ("alt+up".to_string(), "edit.move_line_up"),
        ("alt+down".to_string(), "edit.move_line_down"),
    ]);
    match platform {
        Platform::Mac => bindings.extend([
            ("super+comma".to_string(), "settings.open"),
            ("super+q".to_string(), "app.quit"),
        ]),
        Platform::Linux => bindings.extend([
            // Find is an editor gesture; keep the universal Ctrl+F too.
            ("ctrl+f".to_string(), "navigation.find"),
            ("ctrl+q".to_string(), "app.quit"),
        ]),
    }
    bindings
}

/// Default board-layer `(trigger, action)` pairs, in display-priority order.
/// Board commands take the text-formatting modifier, ⌘ or Ctrl; a trigger
/// they share with the base layer is one only an editor command uses there
/// (Mod+Enter, Alt+Up and Alt+Down), so it never shadows an app shortcut.
pub fn default_board_bindings(platform: Platform) -> Vec<(String, &'static str)> {
    let m = match platform {
        Platform::Mac => "super",
        Platform::Linux => "ctrl",
    };
    let mut bindings: Vec<(String, &'static str)> = [
        ("v", "board.tool.select"),
        ("h", "board.tool.hand"),
        ("r", "board.tool.rectangle"),
        ("o", "board.tool.ellipse"),
        ("d", "board.tool.diamond"),
        ("u", "board.tool.pill"),
        ("y", "board.tool.cylinder"),
        ("p", "board.tool.parallelogram"),
        ("t", "board.tool.text"),
        ("c", "board.tool.connector"),
        ("a", "board.tool.connector"),
        ("shift+r", "board.shape.rectangle"),
        ("shift+o", "board.shape.ellipse"),
        ("shift+d", "board.shape.diamond"),
        ("shift+u", "board.shape.pill"),
        ("shift+y", "board.shape.cylinder"),
        ("shift+p", "board.shape.parallelogram"),
        ("enter", "board.label.edit"),
        ("f2", "board.label.edit"),
        ("tab", "board.connect.next"),
        ("backspace", "board.delete"),
        ("delete", "board.delete"),
        ("shift+1", "board.zoom.fit"),
        ("shift+2", "board.zoom.selection"),
    ]
    .into_iter()
    .map(|(trigger, action)| (trigger.to_string(), action))
    .collect();
    for (dir, [connect, select, extend, connector, step]) in [
        (
            "left",
            [
                "board.connect.left",
                "board.select.left",
                "board.select.extend_left",
                "board.select.connector_left",
                "board.move.left",
            ],
        ),
        (
            "right",
            [
                "board.connect.right",
                "board.select.right",
                "board.select.extend_right",
                "board.select.connector_right",
                "board.move.right",
            ],
        ),
        (
            "up",
            [
                "board.connect.up",
                "board.select.up",
                "board.select.extend_up",
                "board.select.connector_up",
                "board.move.up",
            ],
        ),
        (
            "down",
            [
                "board.connect.down",
                "board.select.down",
                "board.select.extend_down",
                "board.select.connector_down",
                "board.move.down",
            ],
        ),
    ] {
        bindings.extend([
            (format!("{m}+{dir}"), connect),
            (dir.to_string(), select),
            (format!("shift+{dir}"), extend),
            (format!("{m}+shift+{dir}"), connector),
            (format!("alt+{dir}"), step),
        ]);
    }
    bindings.extend([
        ("alt+shift+right".to_string(), "board.resize.wider"),
        ("alt+shift+left".to_string(), "board.resize.narrower"),
        ("alt+shift+down".to_string(), "board.resize.taller"),
        ("alt+shift+up".to_string(), "board.resize.shorter"),
        (format!("{m}+enter"), "board.label.commit"),
        (format!("{m}+a"), "board.select.all"),
        (format!("{m}+shift+a"), "board.select.connected"),
        (format!("{m}+d"), "board.duplicate"),
        (format!("{m}+c"), "board.copy"),
        (format!("{m}+x"), "board.cut"),
        (format!("{m}+v"), "board.paste"),
        (format!("{m}+z"), "board.undo"),
        (format!("{m}+shift+z"), "board.redo"),
    ]);
    // ⌘Y is Focus mode on macOS; Ctrl+Y is the usual redo elsewhere.
    if platform == Platform::Linux {
        bindings.push(("ctrl+y".to_string(), "board.redo"));
    }
    bindings
}

/// The resolved trigger → action tables, one per [`Layer`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    base: Vec<(Accel, String)>,
    board: Vec<(Accel, String)>,
}

impl Keymap {
    /// Apply `user` `(trigger, action)` overrides, in order, on top of
    /// `defaults`. `layer_of` places each action in its layer, and `None`
    /// means the action is unknown. A later binding for the same trigger
    /// replaces the earlier one in its own layer only; `unbind` removes a
    /// trigger from the base layer and `board.unbind` from the board layer.
    /// Invalid lines are skipped with a warning. A default whose action
    /// `layer_of` does not know is skipped, so one default list serves a
    /// host that leaves some commands out.
    pub fn build<'a>(
        defaults: impl IntoIterator<Item = (String, &'a str)>,
        user: &[(String, String)],
        layer_of: impl Fn(&str) -> Option<Layer>,
    ) -> (Self, Vec<String>) {
        let mut keymap = Self::default();
        let mut warnings = Vec::new();
        for (trigger, action) in defaults {
            if let (Some(accel), Some(layer)) = (parse_trigger(&trigger), layer_of(action)) {
                let table = keymap.layer_mut(layer);
                table.retain(|(bound, _)| *bound != accel);
                table.push((accel, action.into()));
            }
        }
        for (trigger, action) in user {
            let Some(accel) = parse_trigger(trigger) else {
                warnings.push(format!("keybind: invalid trigger {trigger:?}"));
                continue;
            };
            let action = action.trim();
            let (layer, unbind) = match action {
                UNBIND => (Layer::Base, true),
                BOARD_UNBIND => (Layer::Board, true),
                _ => match layer_of(action) {
                    Some(layer) => (layer, false),
                    None => {
                        warnings.push(format!("keybind: unknown action {action:?}"));
                        continue;
                    }
                },
            };
            if layer == Layer::Base && !accel.is_bindable() {
                warnings.push(format!(
                    "keybind: {trigger:?} needs ctrl, alt, or super (only f1–f24 may be bound alone)"
                ));
                continue;
            }
            let table = keymap.layer_mut(layer);
            table.retain(|(bound, _)| *bound != accel);
            if !unbind {
                // A user binding takes display priority over the defaults.
                table.insert(0, (accel, action.into()));
            }
        }
        (keymap, warnings)
    }

    fn layer_mut(&mut self, layer: Layer) -> &mut Vec<(Accel, String)> {
        match layer {
            Layer::Base => &mut self.base,
            Layer::Board => &mut self.board,
        }
    }

    /// The base layer's action for `accel`.
    pub fn action(&self, accel: &Accel) -> Option<&str> {
        find_action(&self.base, accel)
    }

    /// The board layer's action for `accel`.
    pub fn board_action(&self, accel: &Accel) -> Option<&str> {
        find_action(&self.board, accel)
    }

    /// The trigger to display for `action`, from either layer, if it is
    /// bound at all.
    pub fn accel(&self, action: &str) -> Option<&Accel> {
        self.base
            .iter()
            .chain(&self.board)
            .find(|(_, bound)| bound == action)
            .map(|(accel, _)| accel)
    }

    pub fn label(&self, action: &str, platform: Platform) -> Option<String> {
        self.accel(action).map(|accel| accel.label(platform))
    }

    /// The base layer's bindings.
    pub fn bindings(&self) -> impl Iterator<Item = (&Accel, &str)> {
        iterate(&self.base)
    }

    /// The board layer's bindings.
    pub fn board_bindings(&self) -> impl Iterator<Item = (&Accel, &str)> {
        iterate(&self.board)
    }

    /// Keep only the board-layer bindings `keep` accepts, for a host that
    /// drops the ones the base layer would shadow.
    pub fn retain_board(&mut self, mut keep: impl FnMut(&Accel, &str) -> bool) {
        self.board.retain(|(accel, action)| keep(accel, action));
    }
}

fn find_action<'a>(table: &'a [(Accel, String)], accel: &Accel) -> Option<&'a str> {
    table
        .iter()
        .find(|(bound, _)| bound == accel)
        .map(|(_, action)| action.as_str())
}

fn iterate(table: &[(Accel, String)]) -> impl Iterator<Item = (&Accel, &str)> {
    table.iter().map(|(accel, action)| (accel, action.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accel(trigger: &str) -> Accel {
        parse_trigger(trigger).unwrap()
    }

    fn base(_: &str) -> Option<Layer> {
        Some(Layer::Base)
    }

    /// The layer the app gives an action: board commands are named `board.*`.
    fn by_name(action: &str) -> Option<Layer> {
        Some(if action.starts_with("board.") {
            Layer::Board
        } else {
            Layer::Base
        })
    }

    fn both_defaults(platform: Platform) -> Vec<(String, &'static str)> {
        let mut defaults = default_bindings(platform);
        defaults.extend(default_board_bindings(platform));
        defaults
    }

    /// `default_bindings` as Wren 0.1.1 shipped it. Boards add a layer and
    /// must not change this one.
    fn frozen_base(platform: Platform) -> Vec<(&'static str, &'static str)> {
        let mut bindings = match platform {
            Platform::Mac => vec![
                ("super+shift+p", "palette.commands"),
                ("super+k", "palette.commands"),
                ("super+p", "navigation.jump"),
                ("super+shift+f", "navigation.search"),
                ("super+f", "navigation.find"),
                ("super+shift+o", "navigation.outline"),
                ("super+n", "note.new"),
                ("super+shift+n", "note.new_dated"),
                ("super+bracketleft", "navigation.back"),
                ("super+bracketright", "navigation.forward"),
                ("super+shift+bracketleft", "navigation.column_left"),
                ("super+shift+bracketright", "navigation.column_right"),
                ("super+r", "note.rename"),
                ("super+y", "view.focus"),
                ("super+plus", "font.increase"),
                ("super+equal", "font.increase"),
                ("super+shift+equal", "font.increase"),
                ("super+shift+plus", "font.increase"),
                ("super+minus", "font.decrease"),
                ("super+0", "font.reset"),
                ("super+enter", "markdown.toggle_task"),
                ("super+b", "markdown.bold"),
                ("super+i", "markdown.italic"),
                ("super+e", "markdown.code"),
            ],
            Platform::Linux => vec![
                ("alt+shift+p", "palette.commands"),
                ("alt+k", "palette.commands"),
                ("alt+p", "navigation.jump"),
                ("alt+shift+f", "navigation.search"),
                ("alt+f", "navigation.find"),
                ("alt+shift+o", "navigation.outline"),
                ("alt+n", "note.new"),
                ("alt+shift+n", "note.new_dated"),
                ("alt+bracketleft", "navigation.back"),
                ("alt+bracketright", "navigation.forward"),
                ("alt+shift+bracketleft", "navigation.column_left"),
                ("alt+shift+bracketright", "navigation.column_right"),
                ("alt+r", "note.rename"),
                ("alt+y", "view.focus"),
                ("alt+plus", "font.increase"),
                ("alt+equal", "font.increase"),
                ("alt+shift+equal", "font.increase"),
                ("alt+shift+plus", "font.increase"),
                ("alt+minus", "font.decrease"),
                ("alt+0", "font.reset"),
                ("ctrl+enter", "markdown.toggle_task"),
                ("ctrl+b", "markdown.bold"),
                ("ctrl+i", "markdown.italic"),
                ("ctrl+e", "markdown.code"),
            ],
        };
        bindings.extend([
            ("alt+up", "edit.move_line_up"),
            ("alt+down", "edit.move_line_down"),
        ]);
        bindings.extend(match platform {
            Platform::Mac => [("super+comma", "settings.open"), ("super+q", "app.quit")],
            Platform::Linux => [("ctrl+f", "navigation.find"), ("ctrl+q", "app.quit")],
        });
        bindings
    }

    #[test]
    fn triggers_accept_aliases_and_canonicalize_keys() {
        assert_eq!(accel("cmd+Shift+P"), accel("super+shift+p"));
        assert_eq!(accel("option+bracketleft").key, "[");
        assert_eq!(accel("ctrl+plus").key, "+");
        assert_eq!(accel("alt+Return").key, "enter");
        assert_eq!(accel("f12").key, "f12");
        for invalid in [
            "",
            "ctrl+",
            "ctrl+shift",
            "a+b",
            "ctrl++",
            "f25",
            "super+tabby",
        ] {
            assert_eq!(parse_trigger(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn labels_follow_each_platform_convention_and_display_round_trips() {
        let palette = accel("super+shift+p");
        assert_eq!(palette.label(Platform::Mac), "⇧⌘P");
        let chord = accel("ctrl+alt+1");
        assert_eq!(chord.label(Platform::Linux), "Ctrl+Alt+1");
        assert_eq!(chord.label(Platform::Mac), "⌃⌥1");
        for trigger in ["super+shift+p", "ctrl+alt+1", "alt+plus", "shift+enter"] {
            assert_eq!(
                parse_trigger(&accel(trigger).to_string()),
                Some(accel(trigger))
            );
        }
    }

    #[test]
    fn palette_defaults_are_command_shift_p_on_mac_and_alt_shift_p_on_linux() {
        for (platform, trigger) in [
            (Platform::Mac, "super+shift+p"),
            (Platform::Linux, "alt+shift+p"),
        ] {
            let (keymap, warnings) = Keymap::build(default_bindings(platform), &[], base);
            assert!(warnings.is_empty());
            assert_eq!(keymap.action(&accel(trigger)), Some("palette.commands"));
            assert_eq!(keymap.accel("palette.commands"), Some(&accel(trigger)));
        }
    }

    #[test]
    fn editor_commands_use_command_on_mac_and_ctrl_on_linux() {
        for (platform, modifier) in [(Platform::Mac, "super"), (Platform::Linux, "ctrl")] {
            let (keymap, _) = Keymap::build(default_bindings(platform), &[], base);
            for (key, action) in [
                ("enter", "markdown.toggle_task"),
                ("b", "markdown.bold"),
                ("i", "markdown.italic"),
                ("e", "markdown.code"),
            ] {
                assert_eq!(
                    keymap.action(&accel(&format!("{modifier}+{key}"))),
                    Some(action)
                );
            }
            assert_eq!(keymap.action(&accel("alt+up")), Some("edit.move_line_up"));
            assert_eq!(
                keymap.action(&accel("alt+down")),
                Some("edit.move_line_down")
            );
        }
    }

    #[test]
    fn defaults_bind_each_trigger_once() {
        for platform in [Platform::Mac, Platform::Linux] {
            let defaults = default_bindings(platform);
            let mut seen = std::collections::HashSet::new();
            for (trigger, _) in &defaults {
                assert!(seen.insert(accel(trigger)), "{trigger} bound twice");
            }
        }
    }

    #[test]
    fn user_bindings_override_unbind_and_warn() {
        let user = [
            ("alt+shift+p".to_string(), "unbind".to_string()),
            ("ctrl+shift+p".to_string(), "palette.commands".to_string()),
            ("alt+p".to_string(), "view.focus".to_string()),
            ("alt+x".to_string(), "no.such".to_string()),
            ("alt+".to_string(), "view.focus".to_string()),
        ];
        let (keymap, warnings) =
            Keymap::build(default_bindings(Platform::Linux), &user, |action| {
                (action != "no.such").then_some(Layer::Base)
            });
        assert_eq!(keymap.action(&accel("alt+shift+p")), None);
        assert_eq!(
            keymap.accel("palette.commands"),
            Some(&accel("ctrl+shift+p"))
        );
        assert_eq!(keymap.action(&accel("alt+p")), Some("view.focus"));
        assert_eq!(keymap.accel("view.focus"), Some(&accel("alt+p")));
        assert_eq!(keymap.accel("navigation.jump"), None);
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("no.such"));
        assert!(warnings[1].contains("invalid trigger"));
    }

    #[test]
    fn bare_keys_other_than_function_keys_are_rejected() {
        let user = [
            ("f5".to_string(), "settings.reload".to_string()),
            ("shift+f13".to_string(), "view.focus".to_string()),
            ("shift+enter".to_string(), "view.focus".to_string()),
            ("p".to_string(), "navigation.jump".to_string()),
        ];
        let (keymap, warnings) = Keymap::build(Vec::new(), &user, base);
        assert_eq!(keymap.action(&accel("f5")), Some("settings.reload"));
        assert_eq!(keymap.action(&accel("shift+f13")), Some("view.focus"));
        assert_eq!(keymap.action(&accel("shift+enter")), None);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn the_base_layer_equals_the_0_1_1_defaults_with_the_board_layer_present() {
        for platform in [Platform::Mac, Platform::Linux] {
            let (keymap, warnings) = Keymap::build(both_defaults(platform), &[], by_name);
            assert!(warnings.is_empty(), "{warnings:?}");
            let frozen: Vec<(Accel, &str)> = frozen_base(platform)
                .into_iter()
                .map(|(trigger, action)| (accel(trigger), action))
                .collect();
            let base: Vec<(Accel, &str)> = keymap
                .bindings()
                .map(|(accel, action)| (accel.clone(), action))
                .collect();
            assert_eq!(base, frozen, "{platform:?}");
            let board: Vec<(Accel, &str)> = keymap
                .board_bindings()
                .map(|(accel, action)| (accel.clone(), action))
                .collect();
            let defaults: Vec<(Accel, &str)> = default_board_bindings(platform)
                .into_iter()
                .map(|(trigger, action)| (accel(&trigger), action))
                .collect();
            assert_eq!(board, defaults, "{platform:?}");
            // Ten of the 64 Phase 2 commands have no default key: the two
            // unlettered tools and reshapes, `board.select.none`, and the
            // five connector style commands.
            let actions: std::collections::HashSet<&str> =
                board.iter().map(|(_, action)| *action).collect();
            assert_eq!(actions.len(), 54, "{platform:?}");
        }
    }

    #[test]
    fn board_defaults_share_a_trigger_only_with_editor_commands() {
        // The base layer wins a shared trigger, so a board default may only
        // reuse one whose base command is inert on a board: an editor
        // command, run only while the text editor has the keyboard.
        for platform in [Platform::Mac, Platform::Linux] {
            let (keymap, _) = Keymap::build(default_bindings(platform), &[], base);
            let shared: std::collections::HashSet<&str> = default_board_bindings(platform)
                .iter()
                .filter_map(|(trigger, _)| keymap.action(&accel(trigger)))
                .collect();
            assert_eq!(
                shared,
                [
                    "markdown.toggle_task",
                    "edit.move_line_up",
                    "edit.move_line_down"
                ]
                .into(),
                "{platform:?}"
            );
        }
    }

    #[test]
    fn board_defaults_bind_each_trigger_once_and_only_board_actions() {
        for platform in [Platform::Mac, Platform::Linux] {
            let mut seen = std::collections::HashSet::new();
            for (trigger, action) in default_board_bindings(platform) {
                assert!(seen.insert(accel(&trigger)), "{trigger} bound twice");
                assert!(action.starts_with("board."), "{trigger} → {action}");
            }
        }
    }

    #[test]
    fn board_defaults_follow_the_platform_modifier() {
        for (platform, m) in [(Platform::Mac, "super"), (Platform::Linux, "ctrl")] {
            let (keymap, _) = Keymap::build(both_defaults(platform), &[], by_name);
            for (trigger, action) in [
                ("v".to_string(), "board.tool.select"),
                ("shift+r".to_string(), "board.shape.rectangle"),
                ("tab".to_string(), "board.connect.next"),
                ("left".to_string(), "board.select.left"),
                ("shift+up".to_string(), "board.select.extend_up"),
                ("alt+down".to_string(), "board.move.down"),
                ("alt+shift+right".to_string(), "board.resize.wider"),
                ("shift+1".to_string(), "board.zoom.fit"),
                ("backspace".to_string(), "board.delete"),
                ("delete".to_string(), "board.delete"),
                (format!("{m}+enter"), "board.label.commit"),
                (format!("{m}+right"), "board.connect.right"),
                (format!("{m}+shift+down"), "board.select.connector_down"),
                (format!("{m}+z"), "board.undo"),
                (format!("{m}+shift+z"), "board.redo"),
            ] {
                assert_eq!(
                    keymap.board_action(&accel(&trigger)),
                    Some(action),
                    "{trigger}"
                );
            }
            assert_eq!(keymap.accel("board.tool.connector"), Some(&accel("c")));
            assert_eq!(
                keymap.board_action(&accel("a")),
                Some("board.tool.connector")
            );
            assert_eq!(keymap.accel("board.label.edit"), Some(&accel("enter")));
            assert_eq!(
                keymap.accel("board.redo"),
                Some(&accel(&format!("{m}+shift+z")))
            );
            // ⌘Y stays Focus mode on macOS; Linux adds the common Ctrl+Y.
            let ctrl_y = keymap.board_action(&accel("ctrl+y"));
            assert_eq!(
                ctrl_y,
                (platform == Platform::Linux).then_some("board.redo")
            );
        }
    }

    #[test]
    fn a_binding_replaces_only_within_its_layer() {
        let user = [
            ("alt+up".to_string(), "board.move.down".to_string()),
            ("x".to_string(), "board.delete".to_string()),
            ("x".to_string(), "note.new".to_string()),
            ("ctrl+alt+d".to_string(), "board.duplicate".to_string()),
        ];
        let (keymap, warnings) = Keymap::build(both_defaults(Platform::Linux), &user, by_name);
        assert_eq!(keymap.action(&accel("alt+up")), Some("edit.move_line_up"));
        assert_eq!(
            keymap.board_action(&accel("alt+up")),
            Some("board.move.down")
        );
        // A bare key is legal in the board layer and still warns in the base.
        assert_eq!(keymap.board_action(&accel("x")), Some("board.delete"));
        assert_eq!(keymap.action(&accel("x")), None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("needs ctrl"), "{warnings:?}");
        // A user binding takes display priority within its layer.
        assert_eq!(keymap.accel("board.delete"), Some(&accel("x")));
        assert_eq!(keymap.accel("board.duplicate"), Some(&accel("ctrl+alt+d")));
        assert_eq!(
            keymap.board_action(&accel("ctrl+d")),
            Some("board.duplicate")
        );
        assert_eq!(
            keymap.label("board.delete", Platform::Linux).as_deref(),
            Some("X")
        );
        assert_eq!(
            keymap.label("note.new", Platform::Linux).as_deref(),
            Some("Alt+N")
        );
    }

    #[test]
    fn unbind_clears_the_base_layer_and_board_unbind_the_board_layer() {
        let user = [
            ("alt+down".to_string(), "unbind".to_string()),
            ("ctrl+enter".to_string(), "board.unbind".to_string()),
            ("tab".to_string(), "board.unbind".to_string()),
            ("tab".to_string(), "unbind".to_string()),
        ];
        let (keymap, warnings) = Keymap::build(both_defaults(Platform::Linux), &user, by_name);
        assert_eq!(keymap.action(&accel("alt+down")), None);
        assert_eq!(
            keymap.board_action(&accel("alt+down")),
            Some("board.move.down")
        );
        assert_eq!(
            keymap.action(&accel("ctrl+enter")),
            Some("markdown.toggle_task")
        );
        assert_eq!(keymap.board_action(&accel("ctrl+enter")), None);
        assert_eq!(keymap.accel("board.label.commit"), None);
        assert_eq!(keymap.board_action(&accel("tab")), None);
        assert_eq!(keymap.accel("board.connect.next"), None);
        // `unbind` on a bare key names the base layer, where it cannot bind.
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("\"tab\""), "{warnings:?}");
    }

    #[test]
    fn the_base_readers_never_see_the_board_layer() {
        let (keymap, _) = Keymap::build(both_defaults(Platform::Mac), &[], by_name);
        assert!(keymap
            .bindings()
            .all(|(_, action)| !action.starts_with("board.")));
        assert!(keymap
            .board_bindings()
            .all(|(_, action)| action.starts_with("board.")));
        assert_eq!(keymap.action(&accel("v")), None);
        assert_eq!(keymap.action(&accel("super+z")), None);
        assert_eq!(keymap.board_action(&accel("super+shift+p")), None);
    }

    #[test]
    fn an_unknown_default_is_skipped_and_an_unknown_user_action_warns() {
        let only_base = |action: &str| (!action.starts_with("board.")).then_some(Layer::Base);
        let user = [("alt+x".to_string(), "board.delete".to_string())];
        let (keymap, warnings) = Keymap::build(both_defaults(Platform::Linux), &user, only_base);
        assert_eq!(keymap.board_bindings().count(), 0);
        assert_eq!(keymap.action(&accel("alt+n")), Some("note.new"));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("unknown action"), "{warnings:?}");
    }

    #[test]
    fn retain_board_drops_only_board_bindings() {
        let (mut keymap, _) = Keymap::build(both_defaults(Platform::Linux), &[], by_name);
        let base: Vec<_> = keymap
            .bindings()
            .map(|(accel, action)| (accel.clone(), action.to_owned()))
            .collect();
        keymap.retain_board(|accel, action| *accel != self::accel("tab") && action != "board.copy");
        assert_eq!(keymap.board_action(&accel("tab")), None);
        assert_eq!(keymap.accel("board.copy"), None);
        assert_eq!(keymap.board_action(&accel("v")), Some("board.tool.select"));
        assert!(keymap
            .bindings()
            .map(|(accel, action)| (accel.clone(), action.to_owned()))
            .eq(base));
    }
}
