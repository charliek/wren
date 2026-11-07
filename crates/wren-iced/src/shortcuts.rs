use iced::keyboard::{key::Named, Event, Key, Modifiers};

use crate::app::commands::CommandId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    Command(CommandId),
    Commands,
    Escape,
    Up,
    Down,
    Enter,
}

pub fn pressed(event: &Event) -> Option<Shortcut> {
    let Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if !modifiers.command() {
        return match key {
            Key::Named(Named::Escape) => Some(Shortcut::Escape),
            Key::Named(Named::ArrowUp) => Some(Shortcut::Up),
            Key::Named(Named::ArrowDown) => Some(Shortcut::Down),
            Key::Named(Named::Enter) => Some(Shortcut::Enter),
            _ => None,
        };
    }
    let Key::Character(character) = key else {
        return None;
    };
    command(character, *modifiers)
}

fn command(character: &str, modifiers: Modifiers) -> Option<Shortcut> {
    use CommandId::*;
    let character = character.to_ascii_lowercase();
    let id = match (character.as_str(), modifiers.shift(), modifiers.alt()) {
        ("k", false, false) => return Some(Shortcut::Commands),
        ("p", false, false) => Jump,
        ("f", true, false) => Search,
        ("f", false, false) => Find,
        ("n", false, false) => NewNote,
        ("n", true, false) => NewDatedNote,
        ("[", false, false) => Back,
        ("]", false, false) => Forward,
        ("\\", false, false) => Focus,
        ("1", false, true) => Columns,
        ("2", false, true) => Outline,
        _ => return None,
    };
    Some(Shortcut::Command(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_and_dated_shortcuts_are_typed_and_modified_keys_do_not_alias() {
        assert_eq!(
            command("N", Modifiers::CTRL | Modifiers::SHIFT),
            Some(Shortcut::Command(CommandId::NewDatedNote))
        );
        assert_eq!(
            command("1", Modifiers::CTRL | Modifiers::ALT),
            Some(Shortcut::Command(CommandId::Columns))
        );
        assert_eq!(command("p", Modifiers::CTRL | Modifiers::ALT), None);
        assert_eq!(command("s", Modifiers::CTRL), None);
    }
}
