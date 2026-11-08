//! The editor's own shortcuts, checked before iced's stock key bindings.

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{self, Binding, KeyPress, Motion};

use crate::{Intent, Message};

/// What key routing needs to know beyond the key press itself.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Context {
    pub markdown_assist: bool,
    /// An input method has preedit text, so the keys that edit it are its own.
    pub composing: bool,
}

/// Custom key-binding handler.  Intercepts Undo, Redo, and Find shortcuts,
/// then turns the editing keys into [`Message::Intent`]s.
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
/// assist on, and iced's stock handling otherwise.
pub(crate) fn key_binding(key_press: KeyPress, context: Context) -> Option<Binding<Message>> {
    if !matches!(key_press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    let m = key_press.modifiers;
    let key = &key_press.key;
    let physical = key_press.physical_key;

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

    Binding::from_key_press(key_press).map(|binding| editing(binding, m, context.markdown_assist))
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
    };
    const PLAIN: Context = Context {
        markdown_assist: false,
        composing: false,
    };
    const COMPOSING: Context = Context {
        markdown_assist: true,
        composing: true,
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
