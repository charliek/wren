//! Keybinding triggers, platform defaults, and user overrides.
//!
//! Triggers use Ghostty/Roost syntax (`super+shift+p`, `alt+bracketleft`).
//! Default tables, and `keybind` lines, may also name two logical
//! modifiers, which a [`Scheme`] resolves: `mod`, the command key (⌘ on
//! macOS; Ctrl on Linux, or Alt with `linux-modifier = alt`), and `opt`, the
//! board's secondary key (⌥ on macOS; on Linux whichever of Ctrl and Alt
//! `mod` is not). Actions are the stable command keys (`navigation.jump`,
//! `palette.commands`, …); this module does not know the command table, so
//! callers validate action names.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

/// Linux's command key, the `linux-modifier` setting. macOS always uses ⌘.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LinuxModifier {
    /// Ctrl, as other Linux apps use.
    #[default]
    Ctrl,
    /// Alt, the way ⌘ works on the Mac, leaving Ctrl to text editing.
    Alt,
}

impl LinuxModifier {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ctrl" => Some(Self::Ctrl),
            "alt" => Some(Self::Alt),
            _ => None,
        }
    }

    /// The config-file spelling.
    pub const fn value(self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::Alt => "alt",
        }
    }
}

/// The two logical modifiers of default tables and `keybind` lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Logical {
    /// `mod`: the command key.
    Mod,
    /// `opt`: the board's secondary key, beside `mod` on the arrows.
    Opt,
}

/// What a keymap is built for: the platform and Linux's command key, which
/// together say what `mod` and `opt` stand for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scheme {
    pub platform: Platform,
    /// Read on Linux only.
    pub linux: LinuxModifier,
}

impl Default for Scheme {
    fn default() -> Self {
        Self::current(LinuxModifier::default())
    }
}

impl Scheme {
    pub const fn new(platform: Platform, linux: LinuxModifier) -> Self {
        Self { platform, linux }
    }

    /// The scheme of the platform Wren runs on.
    pub const fn current(linux: LinuxModifier) -> Self {
        Self::new(Platform::current(), linux)
    }

    /// The modifier `token` stands for, as a trigger names it. A board
    /// binds both Mod+arrow and Opt+arrow, so with Alt as Linux's command
    /// key the secondary one is Ctrl.
    pub const fn modifier(self, token: Logical) -> &'static str {
        match (self.platform, self.linux, token) {
            (Platform::Mac, _, Logical::Mod) => "super",
            (Platform::Mac, _, Logical::Opt) => "alt",
            (Platform::Linux, LinuxModifier::Ctrl, Logical::Mod) => "ctrl",
            (Platform::Linux, LinuxModifier::Ctrl, Logical::Opt) => "alt",
            (Platform::Linux, LinuxModifier::Alt, Logical::Mod) => "alt",
            (Platform::Linux, LinuxModifier::Alt, Logical::Opt) => "ctrl",
        }
    }

    /// `trigger` with its `mod` and `opt` replaced by what they stand for.
    pub fn resolve(self, trigger: &str) -> String {
        trigger
            .split('+')
            .map(|token| match logical(token) {
                Some(token) => self.modifier(token),
                None => token,
            })
            .collect::<Vec<_>>()
            .join("+")
    }

    /// Parse a trigger that may name `mod` or `opt`.
    pub fn parse(self, trigger: &str) -> Option<Accel> {
        parse(trigger, Some(self))
    }

    /// Whether `accel` holds the command key, with or without Shift, and
    /// nothing else.
    pub fn is_mod(self, accel: &Accel) -> bool {
        let mut mods = Mods {
            shift: accel.mods.shift,
            ..Mods::default()
        };
        if let Some(hold) = modifier(self.modifier(Logical::Mod)) {
            hold(&mut mods);
        }
        accel.mods == mods
    }
}

fn logical(token: &str) -> Option<Logical> {
    match token.trim().to_ascii_lowercase().as_str() {
        "mod" => Some(Logical::Mod),
        "opt" => Some(Logical::Opt),
        _ => None,
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
/// the key; `+` itself may be written as `plus`. Here `opt` is Alt and `mod`
/// is no modifier; [`Scheme::parse`] reads both as the logical ones.
pub fn parse_trigger(trigger: &str) -> Option<Accel> {
    parse(trigger, None)
}

fn parse(trigger: &str, scheme: Option<Scheme>) -> Option<Accel> {
    let mut mods = Mods::default();
    let mut key = None;
    for token in trigger.trim().split('+') {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() {
            return None;
        }
        let token = match (scheme, logical(&token)) {
            (Some(scheme), Some(logical)) => scheme.modifier(logical),
            _ => token.as_str(),
        };
        match modifier(token) {
            Some(apply) => apply(&mut mods),
            None if key.is_none() => key = Some(canonical_key(token)?),
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
        // What Shift+/ types on a US layout, and what a board reads as it
        // on any layout.
        let question = Mods {
            shift: true,
            ..Mods::default()
        };
        if self.mods == question && self.key == "/" {
            return "?".into();
        }
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

/// The `edit.*` actions on their key, which a default table puts after a
/// modifier. They are Linux defaults only: on macOS the editor and the Edit
/// menu own ⌘Z, ⌘C and the rest.
const EDIT_KEYS: [(&str, &str); 6] = [
    ("z", "edit.undo"),
    ("shift+z", "edit.redo"),
    ("c", "edit.copy"),
    ("x", "edit.cut"),
    ("v", "edit.paste"),
    ("a", "edit.select_all"),
];

fn resolved(
    scheme: Scheme,
    bindings: impl IntoIterator<Item = (String, &'static str)>,
) -> Vec<(String, &'static str)> {
    bindings
        .into_iter()
        .map(|(trigger, action)| (scheme.resolve(&trigger), action))
        .collect()
}

fn owned(bindings: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
    bindings
        .iter()
        .map(|(trigger, action)| ((*trigger).to_string(), *action))
        .collect()
}

/// Default `(trigger, action)` pairs, in display-priority order: the first
/// trigger listed for an action is the one menus and buttons show.
pub fn default_bindings(scheme: Scheme) -> Vec<(String, &'static str)> {
    let mut bindings: Vec<(String, &'static str)> = owned(&[
        ("mod+shift+p", "palette.commands"),
        ("mod+k", "palette.commands"),
        ("mod+p", "navigation.jump"),
        ("mod+shift+f", "navigation.search"),
        ("mod+f", "navigation.find"),
        ("mod+shift+o", "navigation.outline"),
        ("mod+n", "note.new"),
        ("mod+shift+n", "note.new_dated"),
        ("mod+bracketleft", "navigation.back"),
        ("mod+bracketright", "navigation.forward"),
        ("mod+shift+bracketleft", "navigation.column_left"),
        ("mod+shift+bracketright", "navigation.column_right"),
        ("mod+r", "note.rename"),
        ("mod+y", "view.focus"),
        ("mod+shift+y", "view.collapse_columns"),
        // `+` is Shift+`=` on US layouts, and most people press it unshifted.
        // Toolkits report the shifted press as either key, so bind both.
        ("mod+plus", "font.increase"),
        ("mod+equal", "font.increase"),
        ("mod+shift+equal", "font.increase"),
        ("mod+shift+plus", "font.increase"),
        ("mod+minus", "font.decrease"),
        ("mod+0", "font.reset"),
        ("mod+enter", "markdown.toggle_task"),
        ("mod+b", "markdown.bold"),
        ("mod+i", "markdown.italic"),
        ("mod+e", "markdown.code"),
        // Move line is Alt+↑/↓ in both Linux modes, as in code editors.
        ("alt+up", "edit.move_line_up"),
        ("alt+down", "edit.move_line_down"),
    ]);
    // Ctrl+., the usual "quick fix" key, is the emoji key of IBus on GNOME
    // and never reaches the app there.
    let suggest = match (scheme.platform, scheme.linux) {
        (Platform::Linux, LinuxModifier::Ctrl) => "alt+period",
        _ => "mod+period",
    };
    bindings.push((suggest.to_string(), "editor.spell_suggest"));
    bindings.extend(owned(&[
        ("mod+comma", "settings.open"),
        ("mod+q", "app.quit"),
    ]));
    if scheme.platform == Platform::Linux {
        bindings.extend(EDIT_KEYS.map(|(key, action)| (format!("mod+{key}"), action)));
        // With Alt as the command key, the desktop's Ctrl habits keep
        // working: find, quit, and every text field's editing keys.
        if scheme.linux == LinuxModifier::Alt {
            bindings.extend(owned(&[
                ("ctrl+f", "navigation.find"),
                ("ctrl+q", "app.quit"),
            ]));
            bindings.extend(EDIT_KEYS.map(|(key, action)| (format!("ctrl+{key}"), action)));
        }
    }
    resolved(scheme, bindings)
}

/// Default board-layer `(trigger, action)` pairs, in display-priority order.
/// Board commands take `mod`, and stepping and resizing `opt`; a trigger
/// they share with the base layer is one only an editor command uses there
/// (Mod+Enter, the `edit.*` keys, Alt+Up and Alt+Down), so it never shadows
/// an app shortcut.
pub fn default_board_bindings(scheme: Scheme) -> Vec<(String, &'static str)> {
    let mut bindings: Vec<(String, &'static str)> = owned(&[
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
        ("]", "board.arrange.front"),
        ("[", "board.arrange.back"),
        ("shift+]", "board.arrange.forward"),
        ("shift+[", "board.arrange.backward"),
        ("0", "board.color.neutral"),
        ("1", "board.color.red"),
        ("2", "board.color.orange"),
        ("3", "board.color.yellow"),
        ("4", "board.color.green"),
        ("5", "board.color.cyan"),
        ("6", "board.color.purple"),
        ("7", "board.color.blue"),
        ("/", "board.actions"),
        ("shift+f10", "board.actions"),
        ("shift+/", "board.keys"),
    ]);
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
            (format!("mod+{dir}"), connect),
            (dir.to_string(), select),
            (format!("shift+{dir}"), extend),
            (format!("mod+shift+{dir}"), connector),
            (format!("opt+{dir}"), step),
        ]);
    }
    bindings.extend(owned(&[
        ("opt+shift+right", "board.resize.wider"),
        ("opt+shift+left", "board.resize.narrower"),
        ("opt+shift+down", "board.resize.taller"),
        ("opt+shift+up", "board.resize.shorter"),
        ("mod+enter", "board.label.commit"),
        ("mod+a", "board.select.all"),
        ("mod+shift+a", "board.select.connected"),
        ("mod+d", "board.duplicate"),
        ("mod+c", "board.copy"),
        ("mod+x", "board.cut"),
        ("mod+v", "board.paste"),
        ("mod+z", "board.undo"),
        ("mod+shift+z", "board.redo"),
    ]));
    // With Alt as the command key, the desktop's Ctrl editing keys keep
    // working on a board too.
    if scheme.platform == Platform::Linux && scheme.linux == LinuxModifier::Alt {
        bindings.extend(owned(&[
            ("ctrl+a", "board.select.all"),
            ("ctrl+c", "board.copy"),
            ("ctrl+x", "board.cut"),
            ("ctrl+v", "board.paste"),
            ("ctrl+z", "board.undo"),
            ("ctrl+shift+z", "board.redo"),
        ]));
    }
    resolved(scheme, bindings)
}

/// Default `(trigger, action)` pairs of the task commands, which the keymap
/// takes only while the `tasks` setting is on; with it off the keymap is the
/// one without them, so their triggers do what they did before.
///
/// `task.set_due` is Mod+D, and `task.edit`, the task form, Mod+Shift+Enter
/// (discovery/tasks.md §2 item 21): Editor commands, so the board layer's
/// Mod+D, which only a focused board reads, keeps duplicating there.
pub fn default_task_bindings(scheme: Scheme) -> Vec<(String, &'static str)> {
    resolved(
        scheme,
        owned(&[("mod+d", "task.set_due"), ("mod+shift+enter", "task.edit")]),
    )
}

/// The resolved trigger → action tables, one per [`Layer`], and the
/// [`Scheme`] they were resolved for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    scheme: Scheme,
    base: Vec<(Accel, String)>,
    board: Vec<(Accel, String)>,
}

impl Keymap {
    /// Apply `user` `(trigger, action)` overrides, in order, on top of
    /// `defaults`, reading `mod` and `opt` in either as `scheme` says, so a
    /// keymap built again for another [`LinuxModifier`] moves a user's
    /// `mod+…` line with the defaults. `layer_of` places each action in its
    /// layer, and `None` means the action is unknown. A later binding for
    /// the same trigger replaces the earlier one in its own layer only;
    /// `unbind` removes a trigger from the base layer and `board.unbind`
    /// from the board layer. Invalid lines are skipped with a warning. A
    /// default whose action `layer_of` does not know is skipped, so one
    /// default list serves a host that leaves some commands out.
    pub fn build<'a>(
        scheme: Scheme,
        defaults: impl IntoIterator<Item = (String, &'a str)>,
        user: &[(String, String)],
        layer_of: impl Fn(&str) -> Option<Layer>,
    ) -> (Self, Vec<String>) {
        let mut keymap = Self {
            scheme,
            ..Self::default()
        };
        let mut warnings = Vec::new();
        for (trigger, action) in defaults {
            if let (Some(accel), Some(layer)) = (scheme.parse(&trigger), layer_of(action)) {
                let table = keymap.layer_mut(layer);
                table.retain(|(bound, _)| *bound != accel);
                table.push((accel, action.into()));
            }
        }
        for (trigger, action) in user {
            let Some(accel) = scheme.parse(trigger) else {
                warnings.push(format!("keybind: invalid trigger {trigger:?}"));
                continue;
            };
            let needs_modifier = || {
                format!(
                    "keybind: {trigger:?} needs ctrl, alt, or super (only f1–f24 may be bound alone)"
                )
            };
            let action = action.trim();
            let (layer, unbind) = match action {
                UNBIND => (Layer::Base, true),
                BOARD_UNBIND => (Layer::Board, true),
                _ => match layer_of(action) {
                    Some(layer) => (layer, false),
                    // Outside the board layer the trigger is checked first,
                    // so a line that was already wrong keeps its warning.
                    None if !accel.is_bindable() => {
                        warnings.push(needs_modifier());
                        continue;
                    }
                    None => {
                        warnings.push(format!("keybind: unknown action {action:?}"));
                        continue;
                    }
                },
            };
            if layer == Layer::Base && !accel.is_bindable() {
                warnings.push(needs_modifier());
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

    /// The scheme the keymap was built for.
    pub fn scheme(&self) -> Scheme {
        self.scheme
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
    use std::collections::HashSet;

    const MAC: Scheme = Scheme::new(Platform::Mac, LinuxModifier::Ctrl);
    const CTRL: Scheme = Scheme::new(Platform::Linux, LinuxModifier::Ctrl);
    const ALT: Scheme = Scheme::new(Platform::Linux, LinuxModifier::Alt);
    /// Every scheme a default table is generated for. The Mac ignores the
    /// Linux mode, which `the_mac_ignores_the_linux_modifier` checks.
    const SCHEMES: [Scheme; 3] = [MAC, CTRL, ALT];

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

    fn both_defaults(scheme: Scheme) -> Vec<(String, &'static str)> {
        let mut defaults = default_bindings(scheme);
        defaults.extend(default_board_bindings(scheme));
        defaults
    }

    fn user(lines: &[(&str, &str)]) -> Vec<(String, String)> {
        lines
            .iter()
            .map(|(trigger, action)| ((*trigger).to_owned(), (*action).to_owned()))
            .collect()
    }

    fn literal(table: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
        table
            .iter()
            .map(|(trigger, action)| ((*trigger).to_owned(), *action))
            .collect()
    }

    /// The macOS base layer as Wren shipped it before Linux had a command
    /// key, which `mod` must reproduce exactly.
    const MAC_BASE: &[(&str, &str)] = &[
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
        ("super+shift+y", "view.collapse_columns"),
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
        ("alt+up", "edit.move_line_up"),
        ("alt+down", "edit.move_line_down"),
        ("super+period", "editor.spell_suggest"),
        ("super+comma", "settings.open"),
        ("super+q", "app.quit"),
    ];

    /// The macOS task defaults as they were.
    const MAC_TASK: &[(&str, &str)] = &[
        ("super+d", "task.set_due"),
        ("super+shift+enter", "task.edit"),
    ];

    /// The macOS board layer as it was.
    const MAC_BOARD: &[(&str, &str)] = &[
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
        ("]", "board.arrange.front"),
        ("[", "board.arrange.back"),
        ("shift+]", "board.arrange.forward"),
        ("shift+[", "board.arrange.backward"),
        ("0", "board.color.neutral"),
        ("1", "board.color.red"),
        ("2", "board.color.orange"),
        ("3", "board.color.yellow"),
        ("4", "board.color.green"),
        ("5", "board.color.cyan"),
        ("6", "board.color.purple"),
        ("7", "board.color.blue"),
        ("/", "board.actions"),
        ("shift+f10", "board.actions"),
        ("shift+/", "board.keys"),
        ("super+left", "board.connect.left"),
        ("left", "board.select.left"),
        ("shift+left", "board.select.extend_left"),
        ("super+shift+left", "board.select.connector_left"),
        ("alt+left", "board.move.left"),
        ("super+right", "board.connect.right"),
        ("right", "board.select.right"),
        ("shift+right", "board.select.extend_right"),
        ("super+shift+right", "board.select.connector_right"),
        ("alt+right", "board.move.right"),
        ("super+up", "board.connect.up"),
        ("up", "board.select.up"),
        ("shift+up", "board.select.extend_up"),
        ("super+shift+up", "board.select.connector_up"),
        ("alt+up", "board.move.up"),
        ("super+down", "board.connect.down"),
        ("down", "board.select.down"),
        ("shift+down", "board.select.extend_down"),
        ("super+shift+down", "board.select.connector_down"),
        ("alt+down", "board.move.down"),
        ("alt+shift+right", "board.resize.wider"),
        ("alt+shift+left", "board.resize.narrower"),
        ("alt+shift+down", "board.resize.taller"),
        ("alt+shift+up", "board.resize.shorter"),
        ("super+enter", "board.label.commit"),
        ("super+a", "board.select.all"),
        ("super+shift+a", "board.select.connected"),
        ("super+d", "board.duplicate"),
        ("super+c", "board.copy"),
        ("super+x", "board.cut"),
        ("super+v", "board.paste"),
        ("super+z", "board.undo"),
        ("super+shift+z", "board.redo"),
    ];

    #[test]
    fn the_mac_tables_are_unchanged() {
        assert_eq!(default_bindings(MAC), literal(MAC_BASE));
        assert_eq!(default_task_bindings(MAC), literal(MAC_TASK));
        assert_eq!(default_board_bindings(MAC), literal(MAC_BOARD));
    }

    #[test]
    fn the_mac_ignores_the_linux_modifier() {
        let alt = Scheme::new(Platform::Mac, LinuxModifier::Alt);
        assert_eq!(default_bindings(alt), default_bindings(MAC));
        assert_eq!(default_task_bindings(alt), default_task_bindings(MAC));
        assert_eq!(default_board_bindings(alt), default_board_bindings(MAC));
        assert_eq!(alt.parse("mod+opt+k"), MAC.parse("super+alt+k"));
    }

    /// Linux's base layer in Ctrl mode: Mod is Ctrl.
    const CTRL_BASE: &[(&str, &str)] = &[
        ("ctrl+shift+p", "palette.commands"),
        ("ctrl+k", "palette.commands"),
        ("ctrl+p", "navigation.jump"),
        ("ctrl+shift+f", "navigation.search"),
        ("ctrl+f", "navigation.find"),
        ("ctrl+shift+o", "navigation.outline"),
        ("ctrl+n", "note.new"),
        ("ctrl+shift+n", "note.new_dated"),
        ("ctrl+bracketleft", "navigation.back"),
        ("ctrl+bracketright", "navigation.forward"),
        ("ctrl+shift+bracketleft", "navigation.column_left"),
        ("ctrl+shift+bracketright", "navigation.column_right"),
        ("ctrl+r", "note.rename"),
        ("ctrl+y", "view.focus"),
        ("ctrl+shift+y", "view.collapse_columns"),
        ("ctrl+plus", "font.increase"),
        ("ctrl+equal", "font.increase"),
        ("ctrl+shift+equal", "font.increase"),
        ("ctrl+shift+plus", "font.increase"),
        ("ctrl+minus", "font.decrease"),
        ("ctrl+0", "font.reset"),
        ("ctrl+enter", "markdown.toggle_task"),
        ("ctrl+b", "markdown.bold"),
        ("ctrl+i", "markdown.italic"),
        ("ctrl+e", "markdown.code"),
        ("alt+up", "edit.move_line_up"),
        ("alt+down", "edit.move_line_down"),
        ("alt+period", "editor.spell_suggest"),
        ("ctrl+comma", "settings.open"),
        ("ctrl+q", "app.quit"),
        ("ctrl+z", "edit.undo"),
        ("ctrl+shift+z", "edit.redo"),
        ("ctrl+c", "edit.copy"),
        ("ctrl+x", "edit.cut"),
        ("ctrl+v", "edit.paste"),
        ("ctrl+a", "edit.select_all"),
    ];

    /// Linux's base layer in Alt mode: Mod is Alt, and Ctrl keeps find, quit
    /// and the editing keys.
    const ALT_BASE: &[(&str, &str)] = &[
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
        ("alt+shift+y", "view.collapse_columns"),
        ("alt+plus", "font.increase"),
        ("alt+equal", "font.increase"),
        ("alt+shift+equal", "font.increase"),
        ("alt+shift+plus", "font.increase"),
        ("alt+minus", "font.decrease"),
        ("alt+0", "font.reset"),
        ("alt+enter", "markdown.toggle_task"),
        ("alt+b", "markdown.bold"),
        ("alt+i", "markdown.italic"),
        ("alt+e", "markdown.code"),
        ("alt+up", "edit.move_line_up"),
        ("alt+down", "edit.move_line_down"),
        ("alt+period", "editor.spell_suggest"),
        ("alt+comma", "settings.open"),
        ("alt+q", "app.quit"),
        ("alt+z", "edit.undo"),
        ("alt+shift+z", "edit.redo"),
        ("alt+c", "edit.copy"),
        ("alt+x", "edit.cut"),
        ("alt+v", "edit.paste"),
        ("alt+a", "edit.select_all"),
        ("ctrl+f", "navigation.find"),
        ("ctrl+q", "app.quit"),
        ("ctrl+z", "edit.undo"),
        ("ctrl+shift+z", "edit.redo"),
        ("ctrl+c", "edit.copy"),
        ("ctrl+x", "edit.cut"),
        ("ctrl+v", "edit.paste"),
        ("ctrl+a", "edit.select_all"),
    ];

    /// Linux's board bindings with a modifier, in Ctrl mode: Mod is Ctrl and
    /// Opt Alt. The bare keys before them are the Mac's.
    const CTRL_BOARD: &[(&str, &str)] = &[
        ("ctrl+left", "board.connect.left"),
        ("left", "board.select.left"),
        ("shift+left", "board.select.extend_left"),
        ("ctrl+shift+left", "board.select.connector_left"),
        ("alt+left", "board.move.left"),
        ("ctrl+right", "board.connect.right"),
        ("right", "board.select.right"),
        ("shift+right", "board.select.extend_right"),
        ("ctrl+shift+right", "board.select.connector_right"),
        ("alt+right", "board.move.right"),
        ("ctrl+up", "board.connect.up"),
        ("up", "board.select.up"),
        ("shift+up", "board.select.extend_up"),
        ("ctrl+shift+up", "board.select.connector_up"),
        ("alt+up", "board.move.up"),
        ("ctrl+down", "board.connect.down"),
        ("down", "board.select.down"),
        ("shift+down", "board.select.extend_down"),
        ("ctrl+shift+down", "board.select.connector_down"),
        ("alt+down", "board.move.down"),
        ("alt+shift+right", "board.resize.wider"),
        ("alt+shift+left", "board.resize.narrower"),
        ("alt+shift+down", "board.resize.taller"),
        ("alt+shift+up", "board.resize.shorter"),
        ("ctrl+enter", "board.label.commit"),
        ("ctrl+a", "board.select.all"),
        ("ctrl+shift+a", "board.select.connected"),
        ("ctrl+d", "board.duplicate"),
        ("ctrl+c", "board.copy"),
        ("ctrl+x", "board.cut"),
        ("ctrl+v", "board.paste"),
        ("ctrl+z", "board.undo"),
        ("ctrl+shift+z", "board.redo"),
    ];

    /// Linux's board bindings with a modifier, in Alt mode: Mod is Alt and Opt
    /// Ctrl, and Ctrl keeps the editing keys.
    const ALT_BOARD: &[(&str, &str)] = &[
        ("alt+left", "board.connect.left"),
        ("left", "board.select.left"),
        ("shift+left", "board.select.extend_left"),
        ("alt+shift+left", "board.select.connector_left"),
        ("ctrl+left", "board.move.left"),
        ("alt+right", "board.connect.right"),
        ("right", "board.select.right"),
        ("shift+right", "board.select.extend_right"),
        ("alt+shift+right", "board.select.connector_right"),
        ("ctrl+right", "board.move.right"),
        ("alt+up", "board.connect.up"),
        ("up", "board.select.up"),
        ("shift+up", "board.select.extend_up"),
        ("alt+shift+up", "board.select.connector_up"),
        ("ctrl+up", "board.move.up"),
        ("alt+down", "board.connect.down"),
        ("down", "board.select.down"),
        ("shift+down", "board.select.extend_down"),
        ("alt+shift+down", "board.select.connector_down"),
        ("ctrl+down", "board.move.down"),
        ("ctrl+shift+right", "board.resize.wider"),
        ("ctrl+shift+left", "board.resize.narrower"),
        ("ctrl+shift+down", "board.resize.taller"),
        ("ctrl+shift+up", "board.resize.shorter"),
        ("alt+enter", "board.label.commit"),
        ("alt+a", "board.select.all"),
        ("alt+shift+a", "board.select.connected"),
        ("alt+d", "board.duplicate"),
        ("alt+c", "board.copy"),
        ("alt+x", "board.cut"),
        ("alt+v", "board.paste"),
        ("alt+z", "board.undo"),
        ("alt+shift+z", "board.redo"),
        ("ctrl+a", "board.select.all"),
        ("ctrl+c", "board.copy"),
        ("ctrl+x", "board.cut"),
        ("ctrl+v", "board.paste"),
        ("ctrl+z", "board.undo"),
        ("ctrl+shift+z", "board.redo"),
    ];

    #[test]
    fn the_linux_tables_follow_the_mode() {
        for (scheme, base, task, board) in [
            (CTRL, CTRL_BASE, ["ctrl+d", "ctrl+shift+enter"], CTRL_BOARD),
            (ALT, ALT_BASE, ["alt+d", "alt+shift+enter"], ALT_BOARD),
        ] {
            assert_eq!(default_bindings(scheme), literal(base), "{scheme:?}");
            assert_eq!(
                default_task_bindings(scheme),
                literal(&[(task[0], "task.set_due"), (task[1], "task.edit")]),
                "{scheme:?}"
            );
            let defaults = default_board_bindings(scheme);
            let (bare, modified) = defaults.split_at(39);
            assert_eq!(bare, literal(&MAC_BOARD[..39]), "{scheme:?}");
            assert_eq!(modified, literal(board), "{scheme:?}");
        }
    }

    #[test]
    fn the_logical_modifiers_follow_the_platform_and_mode() {
        for (scheme, primary, secondary) in [
            (MAC, "super", "alt"),
            (CTRL, "ctrl", "alt"),
            (ALT, "alt", "ctrl"),
        ] {
            assert_eq!(scheme.modifier(Logical::Mod), primary);
            assert_eq!(scheme.modifier(Logical::Opt), secondary);
            assert_eq!(
                scheme.resolve("mod+shift+opt+left"),
                format!("{primary}+shift+{secondary}+left")
            );
        }
        assert_eq!(Scheme::default().platform, Platform::current());
        assert_eq!(Scheme::default().linux, LinuxModifier::Ctrl);
    }

    #[test]
    fn the_linux_modifier_parses_ctrl_and_alt() {
        assert_eq!(LinuxModifier::default(), LinuxModifier::Ctrl);
        for mode in [LinuxModifier::Ctrl, LinuxModifier::Alt] {
            assert_eq!(LinuxModifier::parse(mode.value()), Some(mode));
        }
        assert_eq!(LinuxModifier::parse(" Alt "), Some(LinuxModifier::Alt));
        for bad in ["", "super", "control", "mod", "ctrl+alt"] {
            assert_eq!(LinuxModifier::parse(bad), None, "{bad:?}");
        }
    }

    /// The task defaults: Mod+D sets the due date (item 4.1), and
    /// Mod+Shift+Enter opens the task form (item 6.1).
    #[test]
    fn the_task_defaults_are_the_due_date_and_form_keys() {
        for (scheme, trigger, form) in [
            (MAC, "super+d", "super+shift+enter"),
            (CTRL, "ctrl+d", "ctrl+shift+enter"),
            (ALT, "alt+d", "alt+shift+enter"),
        ] {
            assert_eq!(
                default_task_bindings(scheme),
                [
                    (trigger.to_string(), "task.set_due"),
                    (form.to_string(), "task.edit")
                ],
                "{scheme:?}"
            );
            // Neither layer binds the form's key.
            let (keymap, _) = Keymap::build(scheme, both_defaults(scheme), &[], by_name);
            assert_eq!(keymap.action(&accel(form)), None, "{scheme:?}");
            assert_eq!(keymap.board_action(&accel(form)), None, "{scheme:?}");
            // The base layer has no due date trigger, and the board layer's
            // is `board.duplicate`, read only by a focused board.
            assert_eq!(keymap.action(&accel(trigger)), None, "{scheme:?}");
            assert_eq!(
                keymap.board_action(&accel(trigger)),
                Some("board.duplicate"),
                "{scheme:?}"
            );
        }
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
            // Without a scheme `mod` stands for nothing.
            "mod+k",
        ] {
            assert_eq!(parse_trigger(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn mod_and_opt_parse_as_the_schemes_modifiers() {
        for (scheme, primary, secondary) in [
            (MAC, "super", "alt"),
            (CTRL, "ctrl", "alt"),
            (ALT, "alt", "ctrl"),
        ] {
            assert_eq!(
                scheme.parse("Mod+Shift+K"),
                Some(accel(&format!("{primary}+shift+k")))
            );
            assert_eq!(
                scheme.parse("opt+left"),
                Some(accel(&format!("{secondary}+left")))
            );
            assert_eq!(
                scheme.parse("mod+opt+1"),
                Some(accel(&format!("{primary}+{secondary}+1")))
            );
            // Literal modifiers stay as written; `option` is always Alt.
            assert_eq!(scheme.parse("ctrl+k"), Some(accel("ctrl+k")));
            assert_eq!(scheme.parse("option+k"), Some(accel("alt+k")));
            assert!(scheme.is_mod(&scheme.parse("mod+shift+k").unwrap()));
            assert!(!scheme.is_mod(&scheme.parse("opt+k").unwrap()));
            assert!(!scheme.is_mod(&scheme.parse("mod+opt+k").unwrap()));
            for invalid in ["mod", "mod+", "mod+opt", "mod+k+j"] {
                assert_eq!(scheme.parse(invalid), None, "{invalid:?}");
            }
        }
    }

    #[test]
    fn a_mod_keybind_line_follows_the_switch() {
        let lines = user(&[
            ("mod+shift+k", "note.new"),
            ("opt+shift+left", "board.connect.left"),
            ("mod+n", "unbind"),
        ]);
        for (scheme, new, connect) in [
            (MAC, "super+shift+k", "alt+shift+left"),
            (CTRL, "ctrl+shift+k", "alt+shift+left"),
            (ALT, "alt+shift+k", "ctrl+shift+left"),
        ] {
            let (keymap, warnings) = Keymap::build(scheme, both_defaults(scheme), &lines, by_name);
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(keymap.scheme(), scheme);
            assert_eq!(keymap.accel("note.new"), Some(&accel(new)), "{scheme:?}");
            assert_eq!(
                keymap.board_action(&accel(connect)),
                Some("board.connect.left"),
                "{scheme:?}"
            );
            // `mod+n` unbinds the mode's own default.
            let n = scheme.parse("mod+n").unwrap();
            assert_eq!(keymap.action(&n), None, "{scheme:?}");
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
    fn default_labels_follow_the_mode() {
        for (scheme, palette, undo, quit, connect, step, suggest) in [
            (MAC, "⇧⌘P", None, "⌘Q", "⌘→", "⌥→", "⌘."),
            (
                CTRL,
                "Ctrl+Shift+P",
                Some("Ctrl+Z"),
                "Ctrl+Q",
                "Ctrl+→",
                "Alt+→",
                "Alt+.",
            ),
            (
                ALT,
                "Alt+Shift+P",
                Some("Alt+Z"),
                "Alt+Q",
                "Alt+→",
                "Ctrl+→",
                "Alt+.",
            ),
        ] {
            let platform = scheme.platform;
            let (keymap, _) = Keymap::build(scheme, both_defaults(scheme), &[], by_name);
            let label = |action| keymap.label(action, platform);
            assert_eq!(label("palette.commands").as_deref(), Some(palette));
            assert_eq!(label("edit.undo").as_deref(), undo);
            assert_eq!(label("app.quit").as_deref(), Some(quit));
            assert_eq!(label("board.connect.right").as_deref(), Some(connect));
            assert_eq!(label("board.move.right").as_deref(), Some(step));
            assert_eq!(label("editor.spell_suggest").as_deref(), Some(suggest));
        }
    }

    #[test]
    fn shift_slash_is_shown_as_the_question_mark_it_types() {
        for platform in [Platform::Mac, Platform::Linux] {
            assert_eq!(accel("shift+/").label(platform), "?");
            assert_eq!(accel("shift+slash").label(platform), "?");
        }
        assert_eq!(accel("/").label(Platform::Linux), "/");
        assert_eq!(accel("ctrl+shift+/").label(Platform::Linux), "Ctrl+Shift+/");
        assert_eq!(accel("super+shift+/").label(Platform::Mac), "⇧⌘/");
    }

    #[test]
    fn the_palette_is_mod_shift_p() {
        for (scheme, trigger) in [
            (MAC, "super+shift+p"),
            (CTRL, "ctrl+shift+p"),
            (ALT, "alt+shift+p"),
        ] {
            let (keymap, warnings) = Keymap::build(scheme, default_bindings(scheme), &[], base);
            assert!(warnings.is_empty());
            assert_eq!(keymap.action(&accel(trigger)), Some("palette.commands"));
            assert_eq!(keymap.accel("palette.commands"), Some(&accel(trigger)));
        }
    }

    #[test]
    fn editor_commands_use_mod_and_move_line_alt() {
        for scheme in SCHEMES {
            let (keymap, _) = Keymap::build(scheme, default_bindings(scheme), &[], base);
            for (key, action) in [
                ("enter", "markdown.toggle_task"),
                ("b", "markdown.bold"),
                ("i", "markdown.italic"),
                ("e", "markdown.code"),
            ] {
                assert_eq!(
                    keymap.action(&scheme.parse(&format!("mod+{key}")).unwrap()),
                    Some(action),
                    "{scheme:?}"
                );
            }
            assert_eq!(keymap.action(&accel("alt+up")), Some("edit.move_line_up"));
            assert_eq!(
                keymap.action(&accel("alt+down")),
                Some("edit.move_line_down")
            );
        }
    }

    /// Exception: Ctrl+., the usual "quick fix" key, is IBus's emoji key on
    /// GNOME, so in Ctrl mode suggest is Alt+.; everywhere else it is Mod+.
    #[test]
    fn the_suggest_key_is_mod_period_but_alt_period_in_ctrl_mode() {
        for (scheme, trigger, label) in [
            (MAC, "super+period", "⌘."),
            (CTRL, "alt+.", "Alt+."),
            (ALT, "alt+.", "Alt+."),
        ] {
            let (keymap, _) = Keymap::build(scheme, default_bindings(scheme), &[], base);
            assert_eq!(keymap.action(&accel(trigger)), Some("editor.spell_suggest"));
            assert_eq!(
                keymap
                    .label("editor.spell_suggest", scheme.platform)
                    .as_deref(),
                Some(label)
            );
        }
        let (keymap, _) = Keymap::build(CTRL, default_bindings(CTRL), &[], base);
        assert_eq!(keymap.action(&accel("ctrl+period")), None);
    }

    /// Exception: in Alt mode, Ctrl+F and Ctrl+Q find and quit too, after
    /// Alt+F and Alt+Q, which menus show.
    #[test]
    fn alt_mode_also_finds_and_quits_on_ctrl() {
        let (keymap, _) = Keymap::build(ALT, default_bindings(ALT), &[], base);
        for (key, action) in [("f", "navigation.find"), ("q", "app.quit")] {
            assert_eq!(keymap.action(&accel(&format!("ctrl+{key}"))), Some(action));
            assert_eq!(keymap.accel(action), Some(&accel(&format!("alt+{key}"))));
        }
        let (keymap, _) = Keymap::build(CTRL, default_bindings(CTRL), &[], base);
        assert_eq!(keymap.action(&accel("alt+f")), None);
        assert_eq!(keymap.action(&accel("alt+q")), None);
    }

    /// Exception: in Alt mode the editing keys of every other text field,
    /// Ctrl+Z, Ctrl+Shift+Z, Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+A, keep working
    /// beside Mod's; in Ctrl mode they are Mod's. The Mac has no `edit.*`
    /// default: the editor and the Edit menu own ⌘Z and the rest there.
    #[test]
    fn the_edit_actions_are_mod_on_linux_with_ctrl_aliases_in_alt_mode() {
        let edits = [
            ("z", "edit.undo"),
            ("shift+z", "edit.redo"),
            ("c", "edit.copy"),
            ("x", "edit.cut"),
            ("v", "edit.paste"),
            ("a", "edit.select_all"),
        ];
        for scheme in [CTRL, ALT] {
            let (keymap, _) = Keymap::build(scheme, default_bindings(scheme), &[], base);
            for (key, action) in edits {
                let primary = scheme.parse(&format!("mod+{key}")).unwrap();
                assert_eq!(keymap.action(&primary), Some(action), "{scheme:?}");
                assert_eq!(keymap.accel(action), Some(&primary), "{scheme:?}");
                let ctrl = accel(&format!("ctrl+{key}"));
                assert_eq!(keymap.action(&ctrl), Some(action), "{scheme:?}");
                // Ctrl mode binds none of them on Alt.
                let alt = accel(&format!("alt+{key}"));
                assert_eq!(
                    keymap.action(&alt),
                    (scheme == ALT).then_some(action),
                    "{scheme:?}"
                );
            }
        }
        assert!(default_bindings(MAC)
            .iter()
            .all(|(_, action)| !EDIT_KEYS.iter().any(|(_, edit)| edit == action)));
        // Redo is Mod+Shift+Z only, so Ctrl+Y is free for Focus in Ctrl mode.
        let (keymap, _) = Keymap::build(CTRL, both_defaults(CTRL), &[], by_name);
        assert_eq!(keymap.action(&accel("ctrl+y")), Some("view.focus"));
        assert_eq!(keymap.board_action(&accel("ctrl+y")), None);
    }

    /// Exception, the board's: in Alt mode its select all, copy, cut, paste,
    /// undo and redo keep their Ctrl keys beside Mod's.
    #[test]
    fn alt_mode_boards_keep_the_ctrl_editing_keys() {
        let (keymap, _) = Keymap::build(ALT, both_defaults(ALT), &[], by_name);
        for (key, action) in [
            ("a", "board.select.all"),
            ("c", "board.copy"),
            ("x", "board.cut"),
            ("v", "board.paste"),
            ("z", "board.undo"),
            ("shift+z", "board.redo"),
        ] {
            assert_eq!(
                keymap.board_action(&accel(&format!("alt+{key}"))),
                Some(action)
            );
            assert_eq!(
                keymap.board_action(&accel(&format!("ctrl+{key}"))),
                Some(action)
            );
            assert_eq!(keymap.accel(action), Some(&accel(&format!("alt+{key}"))));
        }
        for scheme in [MAC, CTRL] {
            let defaults = default_board_bindings(scheme);
            let ctrl = defaults.iter().filter(|(trigger, _)| {
                let accel = accel(trigger);
                accel.mods.ctrl && !scheme.is_mod(&accel)
            });
            assert_eq!(ctrl.count(), 0, "{scheme:?}");
        }
    }

    /// Every default trigger is bound at most once in its layer: the base
    /// layer with the task defaults, which join it, and the board layer.
    #[test]
    fn every_trigger_is_bound_once_per_layer_and_scheme() {
        for scheme in SCHEMES {
            for (layer, table) in [
                (
                    Layer::Base,
                    default_bindings(scheme)
                        .into_iter()
                        .chain(default_task_bindings(scheme))
                        .collect::<Vec<_>>(),
                ),
                (Layer::Board, default_board_bindings(scheme)),
            ] {
                let mut seen = HashSet::new();
                for (trigger, action) in table {
                    assert!(
                        seen.insert(accel(&trigger)),
                        "{scheme:?} {layer:?}: {trigger} bound twice"
                    );
                    assert_eq!(
                        action.starts_with("board."),
                        layer == Layer::Board,
                        "{scheme:?}: {trigger} → {action}"
                    );
                }
            }
        }
    }

    /// Every base-layer and task default is on Mod alone (with or without
    /// Shift), but for the exceptions, each tested above: move line, Alt+.
    /// in Ctrl mode, and Alt mode's Ctrl keys.
    #[test]
    fn every_base_default_uses_mod_but_the_exceptions() {
        for scheme in SCHEMES {
            let exceptions: Vec<&str> = match (scheme.platform, scheme.linux) {
                (Platform::Mac, _) => vec!["alt+up", "alt+down"],
                (Platform::Linux, LinuxModifier::Ctrl) => {
                    vec!["alt+up", "alt+down", "alt+period"]
                }
                (Platform::Linux, LinuxModifier::Alt) => vec![
                    "ctrl+f",
                    "ctrl+q",
                    "ctrl+z",
                    "ctrl+shift+z",
                    "ctrl+c",
                    "ctrl+x",
                    "ctrl+v",
                    "ctrl+a",
                ],
            };
            let mut found = Vec::new();
            for (trigger, _) in default_bindings(scheme)
                .into_iter()
                .chain(default_task_bindings(scheme))
            {
                if !scheme.is_mod(&accel(&trigger)) {
                    found.push(trigger);
                }
            }
            assert_eq!(found, exceptions, "{scheme:?}");
        }
    }

    #[test]
    fn user_bindings_override_unbind_and_warn() {
        let user = user(&[
            ("alt+shift+p", "unbind"),
            ("ctrl+shift+p", "palette.commands"),
            ("alt+p", "view.focus"),
            ("alt+x", "no.such"),
            ("alt+", "view.focus"),
        ]);
        let (keymap, warnings) = Keymap::build(ALT, default_bindings(ALT), &user, |action| {
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
        let user = user(&[
            ("f5", "settings.reload"),
            ("shift+f13", "view.focus"),
            ("shift+enter", "view.focus"),
            ("p", "navigation.jump"),
        ]);
        let (keymap, warnings) = Keymap::build(CTRL, Vec::new(), &user, base);
        assert_eq!(keymap.action(&accel("f5")), Some("settings.reload"));
        assert_eq!(keymap.action(&accel("shift+f13")), Some("view.focus"));
        assert_eq!(keymap.action(&accel("shift+enter")), None);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn the_keymap_holds_each_layers_defaults_in_order() {
        for scheme in SCHEMES {
            let (keymap, warnings) = Keymap::build(scheme, both_defaults(scheme), &[], by_name);
            assert!(warnings.is_empty(), "{warnings:?}");
            let expect = |table: Vec<(String, &'static str)>| -> Vec<(Accel, &'static str)> {
                table
                    .into_iter()
                    .map(|(trigger, action)| (accel(&trigger), action))
                    .collect()
            };
            let base: Vec<(Accel, &str)> = keymap
                .bindings()
                .map(|(accel, action)| (accel.clone(), action))
                .collect();
            assert_eq!(base, expect(default_bindings(scheme)), "{scheme:?}");
            let board: Vec<(Accel, &str)> = keymap
                .board_bindings()
                .map(|(accel, action)| (accel.clone(), action))
                .collect();
            assert_eq!(board, expect(default_board_bindings(scheme)), "{scheme:?}");
            // Ten of the 64 Phase 2 commands have no default key: the two
            // unlettered tools and reshapes, `board.select.none`, and the
            // five connector style commands. Twelve of the 25 of Phase 3
            // have none: align, distribute, fill and `board.jump`. The
            // keys sheet has `?`.
            let actions: HashSet<&str> = board.iter().map(|(_, action)| *action).collect();
            assert_eq!(actions.len(), 54 + 13 + 1, "{scheme:?}");
        }
    }

    #[test]
    fn board_defaults_share_a_trigger_only_with_editor_commands() {
        // The base layer wins a shared trigger, so a board default may only
        // reuse one whose base command is inert on a board: an editor
        // command, run only while the text editor has the keyboard.
        let editor = [
            "markdown.toggle_task",
            "edit.move_line_up",
            "edit.move_line_down",
        ];
        let edits = EDIT_KEYS.map(|(_, action)| action);
        for scheme in SCHEMES {
            let (keymap, _) = Keymap::build(scheme, default_bindings(scheme), &[], base);
            let shared: HashSet<&str> = default_board_bindings(scheme)
                .iter()
                .filter_map(|(trigger, _)| keymap.action(&accel(trigger)))
                .collect();
            let mut expected: HashSet<&str> = editor.into();
            if scheme.platform == Platform::Linux {
                expected.extend(edits);
            }
            assert_eq!(shared, expected, "{scheme:?}");
        }
    }

    #[test]
    fn board_defaults_follow_mod_and_opt() {
        for scheme in SCHEMES {
            let (keymap, _) = Keymap::build(scheme, both_defaults(scheme), &[], by_name);
            for (trigger, action) in [
                ("v", "board.tool.select"),
                ("shift+r", "board.shape.rectangle"),
                ("tab", "board.connect.next"),
                ("left", "board.select.left"),
                ("shift+up", "board.select.extend_up"),
                ("opt+down", "board.move.down"),
                ("opt+shift+right", "board.resize.wider"),
                ("shift+1", "board.zoom.fit"),
                ("backspace", "board.delete"),
                ("delete", "board.delete"),
                ("mod+enter", "board.label.commit"),
                ("mod+right", "board.connect.right"),
                ("mod+shift+down", "board.select.connector_down"),
                ("mod+z", "board.undo"),
                ("mod+shift+z", "board.redo"),
                ("]", "board.arrange.front"),
                ("shift+[", "board.arrange.backward"),
                ("0", "board.color.neutral"),
                ("7", "board.color.blue"),
                ("/", "board.actions"),
                ("shift+f10", "board.actions"),
                ("shift+/", "board.keys"),
            ] {
                assert_eq!(
                    keymap.board_action(&scheme.parse(trigger).unwrap()),
                    Some(action),
                    "{scheme:?} {trigger}"
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
                scheme.parse("mod+shift+z").as_ref()
            );
            // Redo is Mod+Shift+Z alone: Ctrl+Y redoes on no board.
            assert_eq!(keymap.board_action(&accel("ctrl+y")), None, "{scheme:?}");
        }
    }

    #[test]
    fn a_binding_replaces_only_within_its_layer() {
        let user = user(&[
            ("alt+up", "board.move.down"),
            ("x", "board.delete"),
            ("x", "note.new"),
            ("ctrl+alt+d", "board.duplicate"),
        ]);
        for (scheme, duplicate, new) in [(CTRL, "ctrl+d", "Ctrl+N"), (ALT, "alt+d", "Alt+N")] {
            let (keymap, warnings) = Keymap::build(scheme, both_defaults(scheme), &user, by_name);
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
                keymap.board_action(&accel(duplicate)),
                Some("board.duplicate")
            );
            assert_eq!(
                keymap.label("board.delete", Platform::Linux).as_deref(),
                Some("X")
            );
            assert_eq!(
                keymap.label("note.new", Platform::Linux).as_deref(),
                Some(new)
            );
        }
    }

    #[test]
    fn unbind_clears_the_base_layer_and_board_unbind_the_board_layer() {
        let user = user(&[
            ("alt+down", "unbind"),
            ("ctrl+enter", "board.unbind"),
            ("tab", "board.unbind"),
            ("tab", "unbind"),
        ]);
        let (keymap, warnings) = Keymap::build(CTRL, both_defaults(CTRL), &user, by_name);
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
        let (keymap, _) = Keymap::build(MAC, both_defaults(MAC), &[], by_name);
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
        let user = user(&[("alt+x", "board.delete")]);
        let (keymap, warnings) = Keymap::build(ALT, both_defaults(ALT), &user, only_base);
        assert_eq!(keymap.board_bindings().count(), 0);
        assert_eq!(keymap.action(&accel("alt+n")), Some("note.new"));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("unknown action"), "{warnings:?}");
    }

    #[test]
    fn a_bare_key_with_an_unknown_action_still_needs_a_modifier() {
        // 0.1.1 checked the trigger before the action; keep its warning.
        let user = user(&[("x", "no.such"), ("alt+x", "no.such")]);
        let known = |action: &str| by_name(action).filter(|_| action != "no.such");
        let (_, warnings) = Keymap::build(CTRL, both_defaults(CTRL), &user, known);
        assert_eq!(
            warnings,
            [
                "keybind: \"x\" needs ctrl, alt, or super (only f1–f24 may be bound alone)",
                "keybind: unknown action \"no.such\"",
            ]
        );
    }

    #[test]
    fn retain_board_drops_only_board_bindings() {
        let (mut keymap, _) = Keymap::build(CTRL, both_defaults(CTRL), &[], by_name);
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
