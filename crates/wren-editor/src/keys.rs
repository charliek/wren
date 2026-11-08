//! The editor's own shortcuts, checked before iced's stock key bindings.

use iced::widget::text_editor::{self, Binding, KeyPress};

use crate::Message;

/// Custom key-binding handler.  Intercepts Undo, Redo, and Find shortcuts
/// before delegating to `Binding::from_key_press`.
///
/// Shortcuts handled:
/// - `Ctrl/Cmd+Z` → [`Message::Undo`]
/// - `Ctrl/Cmd+Shift+Z` → [`Message::Redo`]
/// - `Ctrl+Y` (non-macOS only) → [`Message::Redo`]
/// - `Ctrl/Cmd+F` → [`Message::Find`]
pub(crate) fn key_binding(key_press: KeyPress) -> Option<Binding<Message>> {
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

    // Delegate everything else (insert, copy/paste, motion, etc.) to stock iced.
    Binding::from_key_press(key_press)
}

#[cfg(test)]
mod tests {
    use super::*;

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
                assert!(key_binding(KeyPress {
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
                })
                .is_none());
            }
            let binding = key_binding(KeyPress {
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
            });
            assert!(matches!(binding, Some(Binding::Custom(_))));
        }
    }
}
