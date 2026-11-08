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

/// Action name that removes any binding on a trigger.
pub const UNBIND: &str = "unbind";

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

/// The resolved trigger → action table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<(Accel, String)>,
}

impl Keymap {
    /// Apply `user` `(trigger, action)` overrides, in order, on top of
    /// `defaults`. A later binding for the same trigger replaces the earlier
    /// one; `unbind` removes it. Invalid lines are skipped with a warning.
    pub fn build<'a>(
        defaults: impl IntoIterator<Item = (String, &'a str)>,
        user: &[(String, String)],
        known_action: impl Fn(&str) -> bool,
    ) -> (Self, Vec<String>) {
        let mut keymap = Self::default();
        let mut warnings = Vec::new();
        for (trigger, action) in defaults {
            if let Some(accel) = parse_trigger(&trigger) {
                keymap.bind(accel, action.into());
            }
        }
        for (trigger, action) in user {
            let Some(accel) = parse_trigger(trigger) else {
                warnings.push(format!("keybind: invalid trigger {trigger:?}"));
                continue;
            };
            if !accel.is_bindable() {
                warnings.push(format!(
                    "keybind: {trigger:?} needs ctrl, alt, or super (only f1–f24 may be bound alone)"
                ));
                continue;
            }
            let action = action.trim();
            if action == UNBIND {
                keymap.bindings.retain(|(bound, _)| *bound != accel);
            } else if known_action(action) {
                // A user binding takes display priority over the defaults.
                keymap.bindings.retain(|(bound, _)| *bound != accel);
                keymap.bindings.insert(0, (accel, action.into()));
            } else {
                warnings.push(format!("keybind: unknown action {action:?}"));
            }
        }
        (keymap, warnings)
    }

    fn bind(&mut self, accel: Accel, action: String) {
        self.bindings.retain(|(bound, _)| *bound != accel);
        self.bindings.push((accel, action));
    }

    pub fn action(&self, accel: &Accel) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(bound, _)| bound == accel)
            .map(|(_, action)| action.as_str())
    }

    /// The trigger to display for `action`, if it is bound at all.
    pub fn accel(&self, action: &str) -> Option<&Accel> {
        self.bindings
            .iter()
            .find(|(_, bound)| bound == action)
            .map(|(accel, _)| accel)
    }

    pub fn label(&self, action: &str, platform: Platform) -> Option<String> {
        self.accel(action).map(|accel| accel.label(platform))
    }

    pub fn bindings(&self) -> impl Iterator<Item = (&Accel, &str)> {
        self.bindings
            .iter()
            .map(|(accel, action)| (accel, action.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accel(trigger: &str) -> Accel {
        parse_trigger(trigger).unwrap()
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
            let (keymap, warnings) = Keymap::build(default_bindings(platform), &[], |_| true);
            assert!(warnings.is_empty());
            assert_eq!(keymap.action(&accel(trigger)), Some("palette.commands"));
            assert_eq!(keymap.accel("palette.commands"), Some(&accel(trigger)));
        }
    }

    #[test]
    fn editor_commands_use_command_on_mac_and_ctrl_on_linux() {
        for (platform, modifier) in [(Platform::Mac, "super"), (Platform::Linux, "ctrl")] {
            let (keymap, _) = Keymap::build(default_bindings(platform), &[], |_| true);
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
                action != "no.such"
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
        let (keymap, warnings) = Keymap::build(Vec::new(), &user, |_| true);
        assert_eq!(keymap.action(&accel("f5")), Some("settings.reload"));
        assert_eq!(keymap.action(&accel("shift+f13")), Some("view.focus"));
        assert_eq!(keymap.action(&accel("shift+enter")), None);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }
}
