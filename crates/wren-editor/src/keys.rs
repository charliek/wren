//! The editor's own shortcuts, checked before iced's stock key bindings.

use std::sync::Arc;

use iced::keyboard::key::{self, Named};
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{self, Binding, KeyPress, Motion};

use crate::{Intent, Message};

/// Where the host sends a key press that reaches the focused editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRoute {
    /// A host shortcut. The editor ignores the key, so a shortcut never also
    /// edits the text.
    Host,
    /// A host-bound editor command, carried out as [`Message::Intent`].
    Intent(Intent),
    /// Not the host's: the editor handles the key itself.
    Pass,
}

/// The host's say over each key press, asked before any key of the editor's
/// own, with the unmodified key, its physical key, and the modifiers.
///
/// A host builds it from its keymap, so a key bound to a host command
/// reaches only the host. See [`crate::EditorView::set_key_policy`].
pub type KeyPolicy = Arc<dyn Fn(&Key, key::Physical, Modifiers) -> KeyRoute + Send + Sync>;

/// What key routing needs to know beyond the key press itself.
#[derive(Clone, Copy)]
pub(crate) struct Context<'a> {
    pub markdown_assist: bool,
    /// An input method has preedit text, so the keys that edit it are its own.
    pub composing: bool,
    pub policy: Option<&'a KeyPolicy>,
}

/// Custom key-binding handler.  Asks the host's [`KeyPolicy`] first, then
/// intercepts Undo, Redo, and Find shortcuts, and turns the editing keys into
/// [`Message::Intent`]s.
///
/// Shortcuts handled:
/// - `Ctrl/Cmd+Z` → [`Message::Undo`]
/// - `Ctrl/Cmd+Shift+Z` → [`Message::Redo`]
/// - `Ctrl+Y` (non-macOS only) → [`Message::Redo`]
/// - `Ctrl/Cmd+F` → [`Message::Find`]
///
/// The editing keys are read from iced's own bindings, so platform mappings
/// (macOS ⌘← and ⌃A as Home, ⌃H as Backspace) carry over. Backspace,
/// Delete, the word and line deletes, Tab, Shift+Tab, and ⌘↑/⌘↓ are always
/// intents; Enter, Shift+Enter, Home, and Shift+Home only with Markdown
/// assist on, and iced's stock handling otherwise. On macOS a key with ⌘
/// held never types, so an unbound ⌘B inserts nothing.
pub(crate) fn key_binding(key_press: KeyPress, context: Context) -> Option<Binding<Message>> {
    if !matches!(key_press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    let m = key_press.modifiers;
    let key = &key_press.key;
    let physical = key_press.physical_key;

    match context
        .policy
        .map_or(KeyRoute::Pass, |policy| policy(key, physical, m))
    {
        KeyRoute::Host => return None,
        KeyRoute::Intent(wanted) => return Some(intent(wanted)),
        KeyRoute::Pass => {}
    }

    // Undo: Ctrl/Cmd + Z (without Shift)
    if m.command() && !m.shift() && key.to_latin(physical) == Some('z') {
        return Some(Binding::Custom(Message::Undo));
    }

    // Redo: Ctrl/Cmd + Shift + Z
    if m.command() && m.shift() && key.to_latin(physical) == Some('z') {
        return Some(Binding::Custom(Message::Redo));
    }

    // Redo: Ctrl + Y (Linux/Windows only — on macOS this key is unused for redo)
    #[cfg(not(target_os = "macos"))]
    if m.control() && !m.shift() && !m.alt() && !m.logo() && key.to_latin(physical) == Some('y') {
        return Some(Binding::Custom(Message::Redo));
    }

    // Find: Ctrl/Cmd + F
    if m.command() && !m.shift() && key.to_latin(physical) == Some('f') {
        return Some(Binding::Custom(Message::Find));
    }

    let named = match key.as_ref() {
        Key::Named(named) => Some(named),
        _ => None,
    };
    if context.composing
        && matches!(
            named,
            Some(Named::Enter | Named::Tab | Named::Backspace | Named::Delete)
        )
    {
        return None;
    }

    // iced has no Tab binding (its text is a control character), and a Tab
    // the editor ignores could move focus, so the editor always takes it.
    if named == Some(Named::Tab) {
        let plain = !(m.control() || m.alt() || m.logo());
        return plain.then(|| {
            intent(if m.shift() {
                Intent::ShiftTab
            } else {
                Intent::Tab
            })
        });
    }

    Binding::from_key_press(key_press)
        .filter(|binding| types_with(binding, m, cfg!(target_os = "macos")))
        .map(|binding| editing(binding, m, context.markdown_assist))
}

/// Whether `binding` may run with `m` held: on macOS (`macos`), ⌘ is for
/// shortcuts, and an insert under it would type the shortcut's letter.
fn types_with(binding: &Binding<Message>, m: Modifiers, macos: bool) -> bool {
    !(macos && m.logo() && matches!(binding, Binding::Insert(_)))
}

/// `binding` as an intent when it is one of the editing keys.
fn editing(binding: Binding<Message>, m: Modifiers, markdown_assist: bool) -> Binding<Message> {
    // ⌥ on macOS, Ctrl on Linux; Linux Alt too, as in terminals.
    let word = m.jump() || m.alt();
    let extend = matches!(binding, Binding::Select(_));
    let wanted = match binding {
        Binding::Backspace if m.macos_command() => Intent::DeleteToTextStart,
        Binding::Backspace if word => Intent::DeleteWordBackward,
        Binding::Backspace => Intent::Backspace,
        Binding::Delete if word => Intent::DeleteWordForward,
        Binding::Delete => Intent::Delete,
        Binding::Move(Motion::Up) | Binding::Select(Motion::Up) if m.macos_command() => {
            Intent::DocumentStart { extend }
        }
        Binding::Move(Motion::Down) | Binding::Select(Motion::Down) if m.macos_command() => {
            Intent::DocumentEnd { extend }
        }
        Binding::Enter if markdown_assist && m.shift() => Intent::ShiftEnter,
        Binding::Enter if markdown_assist => Intent::Enter,
        Binding::Move(Motion::Home) | Binding::Select(Motion::Home) if markdown_assist => {
            Intent::Home { extend }
        }
        other => return other,
    };
    intent(wanted)
}

fn intent(intent: Intent) -> Binding<Message> {
    Binding::Custom(Message::Intent(intent))
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key;

    use super::*;

    const ASSIST: Context = Context {
        markdown_assist: true,
        composing: false,
        policy: None,
    };
    const PLAIN: Context = Context {
        markdown_assist: false,
        composing: false,
        policy: None,
    };
    const COMPOSING: Context = Context {
        markdown_assist: true,
        composing: true,
        policy: None,
    };

    /// The binding for a focused press of `key` with `modifiers`, typing
    /// `text`.
    fn press(
        key: Key,
        modifiers: Modifiers,
        text: Option<&str>,
        context: Context,
    ) -> Option<Binding<Message>> {
        key_binding(
            KeyPress {
                modified_key: key.clone(),
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                key,
                modifiers,
                text: text.map(Into::into),
                status: text_editor::Status::Focused { is_hovered: false },
            },
            context,
        )
    }

    fn named(named: Named, modifiers: Modifiers, context: Context) -> Option<Intent> {
        let text = match named {
            Named::Enter => Some("\r"),
            Named::Tab => Some("\t"),
            Named::Backspace => Some("\u{8}"),
            _ => None,
        };
        match press(Key::Named(named), modifiers, text, context)? {
            Binding::Custom(Message::Intent(intent)) => Some(intent),
            _ => None,
        }
    }

    #[test]
    fn editing_keys_become_intents() {
        let none = Modifiers::empty();
        let shift = Modifiers::SHIFT;
        let word = if cfg!(target_os = "macos") {
            Modifiers::ALT
        } else {
            Modifiers::CTRL
        };
        for (key, modifiers, intent) in [
            (Named::Enter, none, Intent::Enter),
            (Named::Enter, shift, Intent::ShiftEnter),
            (Named::Tab, none, Intent::Tab),
            (Named::Tab, shift, Intent::ShiftTab),
            (Named::Backspace, none, Intent::Backspace),
            (Named::Backspace, shift, Intent::Backspace),
            (Named::Backspace, word, Intent::DeleteWordBackward),
            (Named::Backspace, Modifiers::ALT, Intent::DeleteWordBackward),
            (Named::Delete, none, Intent::Delete),
            (Named::Delete, word, Intent::DeleteWordForward),
            (Named::Home, none, Intent::Home { extend: false }),
            (Named::Home, shift, Intent::Home { extend: true }),
        ] {
            assert_eq!(
                named(key, modifiers, ASSIST),
                Some(intent),
                "{key:?} {modifiers:?}"
            );
        }
        // The jump modifier widens Home to the document start, iced's way.
        assert_eq!(named(Named::Home, word, ASSIST), None);
    }

    #[test]
    fn the_editor_keeps_tab_but_not_tab_shortcuts() {
        for context in [ASSIST, PLAIN] {
            assert_eq!(
                named(Named::Tab, Modifiers::empty(), context),
                Some(Intent::Tab)
            );
            assert_eq!(
                named(Named::Tab, Modifiers::SHIFT, context),
                Some(Intent::ShiftTab)
            );
        }
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            assert!(press(Key::Named(Named::Tab), modifiers, Some("\t"), ASSIST).is_none());
        }
    }

    #[test]
    fn without_assist_enter_and_home_are_iced_s_own() {
        assert!(matches!(
            press(
                Key::Named(Named::Enter),
                Modifiers::empty(),
                Some("\r"),
                PLAIN
            ),
            Some(Binding::Enter)
        ));
        assert!(matches!(
            press(
                Key::Named(Named::Enter),
                Modifiers::SHIFT,
                Some("\r"),
                PLAIN
            ),
            Some(Binding::Enter)
        ));
        assert!(matches!(
            press(Key::Named(Named::Home), Modifiers::SHIFT, None, PLAIN),
            Some(Binding::Select(Motion::Home))
        ));
        assert_eq!(
            named(Named::Backspace, Modifiers::empty(), PLAIN),
            Some(Intent::Backspace)
        );
        assert_eq!(
            named(Named::Delete, Modifiers::empty(), PLAIN),
            Some(Intent::Delete)
        );
    }

    #[test]
    fn an_input_method_keeps_its_editing_keys_while_composing() {
        for key in [Named::Enter, Named::Tab, Named::Backspace, Named::Delete] {
            assert!(
                press(Key::Named(key), Modifiers::empty(), None, COMPOSING).is_none(),
                "{key:?}"
            );
        }
        assert_eq!(
            named(Named::Home, Modifiers::empty(), COMPOSING),
            Some(Intent::Home { extend: false })
        );
        assert!(matches!(
            press(
                Key::Character("a".into()),
                Modifiers::empty(),
                Some("a"),
                COMPOSING
            ),
            Some(Binding::Insert('a'))
        ));
    }

    /// A host that binds Ctrl+Enter and Ctrl+B to editor commands, and
    /// Ctrl+F and Ctrl+Z to its own.
    fn host_policy() -> KeyPolicy {
        Arc::new(|key: &Key, _, modifiers: Modifiers| {
            if !modifiers.control() {
                return KeyRoute::Pass;
            }
            match key.as_ref() {
                Key::Named(Named::Enter) => KeyRoute::Intent(Intent::ToggleTask),
                Key::Character("b") => KeyRoute::Intent(Intent::Bold),
                Key::Character("f" | "z") => KeyRoute::Host,
                _ => KeyRoute::Pass,
            }
        })
    }

    #[test]
    fn the_host_policy_routes_keys_before_the_editor_s_own() {
        let policy = host_policy();
        let context = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        let ctrl = Modifiers::CTRL;
        assert_eq!(named(Named::Enter, ctrl, context), Some(Intent::ToggleTask));
        let letter =
            |letter: &str, context| press(Key::Character(letter.into()), ctrl, None, context);
        assert!(matches!(
            letter("b", context),
            Some(Binding::Custom(Message::Intent(Intent::Bold)))
        ));
        // A host shortcut is the host's alone, even over the editor's own
        // Find and Undo.
        assert!(letter("f", context).is_none());
        assert!(letter("z", context).is_none());
        assert!(matches!(
            letter("f", ASSIST),
            Some(Binding::Custom(Message::Find))
        ));
        // Everything else is the editor's, as without a policy.
        assert_eq!(
            named(Named::Enter, Modifiers::empty(), context),
            Some(Intent::Enter)
        );
        assert!(matches!(
            press(
                Key::Character("a".into()),
                Modifiers::empty(),
                Some("a"),
                context
            ),
            Some(Binding::Insert('a'))
        ));
        // An unfocused editor never asks.
        let asked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = Arc::clone(&asked);
        let spy: KeyPolicy = Arc::new(move |_: &Key, _, _| {
            seen.store(true, std::sync::atomic::Ordering::Relaxed);
            KeyRoute::Host
        });
        let unfocused = key_binding(
            KeyPress {
                key: Key::Named(Named::Enter),
                modified_key: Key::Named(Named::Enter),
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                modifiers: ctrl,
                text: None,
                status: text_editor::Status::Active,
            },
            Context {
                policy: Some(&spy),
                ..ASSIST
            },
        );
        assert!(unfocused.is_none());
        assert!(!asked.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn command_never_types_on_macos() {
        let insert = Binding::Insert('b');
        assert!(!types_with(&insert, Modifiers::LOGO, true));
        assert!(!types_with(&insert, Modifiers::LOGO | Modifiers::ALT, true));
        assert!(types_with(&insert, Modifiers::ALT, true));
        assert!(types_with(&insert, Modifiers::SHIFT, true));
        assert!(types_with(&Binding::Paste, Modifiers::LOGO, true));
        assert!(types_with(&insert, Modifiers::LOGO, false));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_command_keys_reach_the_line_and_document_ends() {
        let logo = Modifiers::LOGO;
        assert_eq!(
            named(Named::Backspace, logo, ASSIST),
            Some(Intent::DeleteToTextStart)
        );
        assert_eq!(
            named(Named::ArrowLeft, logo, ASSIST),
            Some(Intent::Home { extend: false })
        );
        assert_eq!(
            named(Named::ArrowLeft, logo | Modifiers::SHIFT, ASSIST),
            Some(Intent::Home { extend: true })
        );
        assert_eq!(
            named(Named::ArrowUp, logo, PLAIN),
            Some(Intent::DocumentStart { extend: false })
        );
        assert_eq!(
            named(Named::ArrowDown, logo | Modifiers::SHIFT, ASSIST),
            Some(Intent::DocumentEnd { extend: true })
        );
        // ⌃A is the smart Home on purpose; ⌃H stays a plain Backspace.
        let ctrl = |letter: &str| match press(
            Key::Character(letter.into()),
            Modifiers::CTRL,
            None,
            ASSIST,
        ) {
            Some(Binding::Custom(Message::Intent(intent))) => Some(intent),
            _ => None,
        };
        assert_eq!(ctrl("a"), Some(Intent::Home { extend: false }));
        assert_eq!(ctrl("h"), Some(Intent::Backspace));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unbound_command_letter_types_nothing() {
        assert!(press(
            Key::Character("b".into()),
            Modifiers::LOGO,
            Some("b"),
            ASSIST
        )
        .is_none());
    }

    #[test]
    fn shortcuts_require_editor_focus() {
        use iced::keyboard::{key, Key, Modifiers};
        let command = if cfg!(target_os = "macos") {
            Modifiers::LOGO
        } else {
            Modifiers::CTRL
        };
        for (letter, physical, shift) in [
            ("z", key::Code::KeyZ, false),
            ("z", key::Code::KeyZ, true),
            ("f", key::Code::KeyF, false),
            #[cfg(not(target_os = "macos"))]
            ("y", key::Code::KeyY, false),
        ] {
            for status in [
                text_editor::Status::Active,
                text_editor::Status::Hovered,
                text_editor::Status::Disabled,
            ] {
                assert!(key_binding(
                    KeyPress {
                        key: Key::Character(letter.into()),
                        modified_key: Key::Character(letter.into()),
                        physical_key: key::Physical::Code(physical),
                        modifiers: command
                            | if shift {
                                Modifiers::SHIFT
                            } else {
                                Modifiers::empty()
                            },
                        text: None,
                        status,
                    },
                    ASSIST
                )
                .is_none());
            }
            let binding = key_binding(
                KeyPress {
                    key: Key::Character(letter.into()),
                    modified_key: Key::Character(letter.into()),
                    physical_key: key::Physical::Code(physical),
                    modifiers: command
                        | if shift {
                            Modifiers::SHIFT
                        } else {
                            Modifiers::empty()
                        },
                    text: None,
                    status: text_editor::Status::Focused { is_hovered: false },
                },
                ASSIST,
            );
            assert!(matches!(binding, Some(Binding::Custom(_))));
        }
    }
}
