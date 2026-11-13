//! Which keys a focused board takes, and what it makes of them (the plan's
//! §5.1): Escape's ladder and a held Space first, then the host's
//! [`KeyPolicy`], which knows the keymap. A key the host claims is left to
//! it; every other key is taken, so it never falls through to the host's
//! column keys, and is a command, the connect query's text or nothing.

use std::sync::Arc;

use iced::keyboard::{key, Key, Modifiers};
use wren_board::Dir;

use crate::command::Command;
use crate::machine::{self, Input};

/// Where the host sends a key press that reaches the focused board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// The host's shortcut: the board leaves the key alone.
    Host,
    /// A command the board layer binds to the key.
    Command(Command),
    /// Neither: the board's own key.
    Pass,
}

/// The host's say over each key press the focused board is given: the key
/// as typed without modifiers, the physical key, the modifiers, and
/// whether the press repeats a held key. See
/// [`crate::BoardView::set_key_policy`].
pub type KeyPolicy = Arc<dyn Fn(&Key, key::Physical, Modifiers, bool) -> KeyRoute + Send + Sync>;

/// What the board makes of one key press.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Keyed {
    /// Not the board's.
    Leave,
    /// Taken, sending these in order: none for a key that only must not
    /// reach the host.
    Take(Vec<Out>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Out {
    Input(Input),
    Command(Command),
}

/// The board's state, as far as a key press reads it.
#[derive(Clone, Copy)]
pub(crate) struct Context<'a> {
    /// A label is open, so keys are its text.
    pub(crate) editing: bool,
    /// Escape has a rung of its ladder to act on ([`machine::Machine::escapes`]).
    pub(crate) escapes: bool,
    pub(crate) policy: Option<&'a KeyPolicy>,
}

/// What the focused board makes of a press of `key`, which typed `text`.
pub(crate) fn route(
    key: &Key,
    physical: key::Physical,
    modifiers: Modifiers,
    text: Option<&str>,
    repeat: bool,
    context: Context<'_>,
) -> Keyed {
    use key::Named;
    let shortcut = modifiers.control() || modifiers.alt() || modifiers.logo();
    match key {
        // The ladder's rungs past the overlays are the board's; on the last
        // one Escape is the host's, which keeps the board focused (§5.14).
        Key::Named(Named::Escape) if context.escapes => {
            return Keyed::Take(vec![Out::Input(Input::Key {
                key: machine::Key::Escape,
                repeat,
            })]);
        }
        Key::Named(Named::Escape) => return Keyed::Leave,
        // Held, Space turns a left drag into a pan; in a label it is text.
        Key::Named(Named::Space) if !shortcut && !context.editing => {
            let held = (!repeat).then_some(Out::Input(Input::Space { held: true }));
            return Keyed::Take(held.into_iter().collect());
        }
        _ => {}
    }
    let route = match context.policy {
        Some(policy) => policy(key, physical, modifiers, repeat),
        None if shortcut => KeyRoute::Host,
        None => KeyRoute::Pass,
    };
    match route {
        KeyRoute::Host => Keyed::Leave,
        // In a label only these two leave the text (§5.1 rule 3); every
        // other key is the label's.
        KeyRoute::Command(command @ (Command::CommitLabel | Command::ConnectNext))
            if context.editing =>
        {
            Keyed::Take(vec![Out::Command(command)])
        }
        KeyRoute::Command(_) | KeyRoute::Pass if context.editing => Keyed::Take(Vec::new()),
        KeyRoute::Command(command) => Keyed::Take(vec![Out::Command(command)]),
        KeyRoute::Pass => Keyed::Take(plain(key, text, shortcut, repeat)),
    }
}

/// An unbound key outside a label: the keys keyboard Connecting reads, and
/// typed text for its query (§5.12 rule 3).
fn plain(key: &Key, text: Option<&str>, shortcut: bool, repeat: bool) -> Vec<Out> {
    use key::Named;
    let read = |key| vec![Out::Input(Input::Key { key, repeat })];
    match key {
        Key::Named(Named::Enter) => read(machine::Key::Enter),
        Key::Named(Named::Backspace) => read(machine::Key::Backspace),
        Key::Named(Named::ArrowLeft) => read(machine::Key::Arrow(Dir::Left)),
        Key::Named(Named::ArrowRight) => read(machine::Key::Arrow(Dir::Right)),
        Key::Named(Named::ArrowUp) => read(machine::Key::Arrow(Dir::Up)),
        Key::Named(Named::ArrowDown) => read(machine::Key::Arrow(Dir::Down)),
        _ if shortcut => Vec::new(),
        _ => text
            .into_iter()
            .flat_map(str::chars)
            .filter(|c| !c.is_control())
            .map(|c| Out::Input(Input::Char(c)))
            .collect(),
    }
}

/// Whether `key` is one of the four arrows.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn is_arrow(key: &Key) -> bool {
    use key::Named;
    matches!(
        key,
        Key::Named(Named::ArrowUp | Named::ArrowDown | Named::ArrowLeft | Named::ArrowRight)
    )
}

/// The modifiers the machine reads, resolved for the platform: the
/// duplicate-drag key is ⌥ on macOS and Ctrl elsewhere (§5.8 rule 3).
pub(crate) fn mods(modifiers: Modifiers) -> machine::Mods {
    machine::Mods {
        shift: modifiers.shift(),
        duplicate: if cfg!(target_os = "macos") {
            modifiers.alt()
        } else {
            modifiers.control()
        },
    }
}

#[cfg(test)]
mod tests {
    use key::{Code, Named, Physical};

    use super::*;
    use crate::command::Tool;

    const IDLE: Context<'static> = Context {
        editing: false,
        escapes: false,
        policy: None,
    };

    fn press(key: Key, modifiers: Modifiers, text: Option<&str>, context: Context<'_>) -> Keyed {
        route(
            &key,
            Physical::Code(Code::KeyA),
            modifiers,
            text,
            false,
            context,
        )
    }

    fn character(c: &str) -> Key {
        Key::Character(c.into())
    }

    fn input(input: Input) -> Keyed {
        Keyed::Take(vec![Out::Input(input)])
    }

    /// A policy in the shape the host builds: Alt is the app's, bare keys
    /// and Ctrl chords are board commands, and the rest pass.
    fn policy() -> KeyPolicy {
        Arc::new(|key, _, modifiers, repeat| {
            if modifiers.alt() {
                return KeyRoute::Host;
            }
            match key.as_ref() {
                Key::Character("d") if !repeat => {
                    KeyRoute::Command(Command::Tool(Tool::Shape(wren_board::Shape::Diamond)))
                }
                Key::Named(Named::Tab) if !repeat => KeyRoute::Command(Command::ConnectNext),
                Key::Named(Named::Enter) if modifiers.control() => {
                    KeyRoute::Command(Command::CommitLabel)
                }
                Key::Named(Named::Enter) => KeyRoute::Command(Command::EditLabel),
                Key::Named(Named::ArrowRight) => KeyRoute::Command(Command::Select(Dir::Right)),
                Key::Character("z") if modifiers.control() => KeyRoute::Command(Command::Undo),
                _ => KeyRoute::Pass,
            }
        })
    }

    #[test]
    fn escape_is_taken_only_when_the_ladder_has_a_rung_for_it() {
        let escape = Key::Named(Named::Escape);
        assert_eq!(
            press(escape.clone(), Modifiers::default(), None, IDLE),
            Keyed::Leave
        );
        let rung = Context {
            escapes: true,
            ..IDLE
        };
        let pressed = input(Input::Key {
            key: machine::Key::Escape,
            repeat: false,
        });
        assert_eq!(
            press(escape.clone(), Modifiers::default(), None, rung),
            pressed
        );
        // Whatever the policy and the modifiers say.
        let policy = policy();
        let ruled = Context {
            policy: Some(&policy),
            ..rung
        };
        assert_eq!(press(escape.clone(), Modifiers::ALT, None, ruled), pressed);
        // A held Escape repeats into the machine, which acts on none.
        let held = route(
            &escape,
            Physical::Code(Code::Escape),
            Modifiers::default(),
            None,
            true,
            rung,
        );
        assert_eq!(
            held,
            input(Input::Key {
                key: machine::Key::Escape,
                repeat: true
            })
        );
    }

    #[test]
    fn space_is_the_pan_key_outside_a_label_and_its_repeats_send_nothing() {
        let space = Key::Named(Named::Space);
        for modifiers in [Modifiers::default(), Modifiers::SHIFT] {
            assert_eq!(
                press(space.clone(), modifiers, Some(" "), IDLE),
                input(Input::Space { held: true })
            );
        }
        let repeat = route(
            &space,
            Physical::Code(Code::Space),
            Modifiers::default(),
            Some(" "),
            true,
            IDLE,
        );
        assert_eq!(repeat, Keyed::Take(Vec::new()));
        for shortcut in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            assert_eq!(press(space.clone(), shortcut, None, IDLE), Keyed::Leave);
        }
        let editing = Context {
            editing: true,
            ..IDLE
        };
        assert_eq!(
            press(space, Modifiers::default(), Some(" "), editing),
            Keyed::Take(Vec::new())
        );
    }

    #[test]
    fn without_a_policy_shortcuts_are_the_hosts_and_every_other_key_the_boards() {
        for shortcut in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            for key in [
                character("k"),
                Key::Named(Named::ArrowUp),
                Key::Named(Named::Tab),
            ] {
                assert_eq!(press(key, shortcut, None, IDLE), Keyed::Leave);
            }
        }
        assert_eq!(
            press(Key::Named(Named::ArrowUp), Modifiers::SHIFT, None, IDLE),
            input(Input::Key {
                key: machine::Key::Arrow(Dir::Up),
                repeat: false
            })
        );
        assert_eq!(
            press(
                Key::Named(Named::Tab),
                Modifiers::default(),
                Some("\t"),
                IDLE
            ),
            Keyed::Take(Vec::new())
        );
        assert_eq!(
            press(character("c"), Modifiers::SHIFT, Some("C"), IDLE),
            input(Input::Char('C'))
        );
    }

    #[test]
    fn the_policy_decides_between_the_host_a_command_and_the_board() {
        let policy = policy();
        let ruled = Context {
            policy: Some(&policy),
            ..IDLE
        };
        let plain = Modifiers::default();
        // The app's key is left alone, even a bare one.
        assert_eq!(
            press(character("k"), Modifiers::ALT, Some("k"), ruled),
            Keyed::Leave
        );
        // A board command.
        assert_eq!(
            press(character("d"), plain, Some("d"), ruled),
            Keyed::Take(vec![Out::Command(Command::Tool(Tool::Shape(
                wren_board::Shape::Diamond
            )))])
        );
        assert_eq!(
            press(Key::Named(Named::ArrowRight), plain, None, ruled),
            Keyed::Take(vec![Out::Command(Command::Select(Dir::Right))])
        );
        // An unbound key is still taken, as the query's text or nothing.
        assert_eq!(
            press(character("q"), plain, Some("q"), ruled),
            input(Input::Char('q'))
        );
        assert_eq!(
            press(Key::Named(Named::Backspace), plain, None, ruled),
            input(Input::Key {
                key: machine::Key::Backspace,
                repeat: false
            })
        );
        assert_eq!(
            press(Key::Named(Named::F5), plain, None, ruled),
            Keyed::Take(Vec::new())
        );
        // A held key the policy does not repeat is taken and does nothing.
        let held = route(
            &Key::Named(Named::Tab),
            Physical::Code(Code::Tab),
            plain,
            None,
            true,
            ruled,
        );
        assert_eq!(held, Keyed::Take(Vec::new()));
    }

    #[test]
    fn in_a_label_only_commit_and_next_leave_the_text() {
        let policy = policy();
        let editing = Context {
            editing: true,
            escapes: true,
            policy: Some(&policy),
        };
        let plain = Modifiers::default();
        assert_eq!(
            press(Key::Named(Named::Enter), Modifiers::CTRL, None, editing),
            Keyed::Take(vec![Out::Command(Command::CommitLabel)])
        );
        assert_eq!(
            press(Key::Named(Named::Tab), plain, Some("\t"), editing),
            Keyed::Take(vec![Out::Command(Command::ConnectNext)])
        );
        // Bound or not, the rest are the label's.
        for (key, modifiers, text) in [
            (Key::Named(Named::Enter), plain, Some("\n")),
            (Key::Named(Named::ArrowRight), plain, None),
            (character("d"), plain, Some("d")),
            (character("z"), Modifiers::CTRL, None),
            (character("q"), plain, Some("q")),
        ] {
            assert_eq!(
                press(key.clone(), modifiers, text, editing),
                Keyed::Take(Vec::new()),
                "{key:?}"
            );
        }
        // The host's keys stay the host's, and Escape commits.
        assert_eq!(
            press(character("k"), Modifiers::ALT, None, editing),
            Keyed::Leave
        );
        assert_eq!(
            press(Key::Named(Named::Escape), plain, None, editing),
            input(Input::Key {
                key: machine::Key::Escape,
                repeat: false
            })
        );
    }

    #[test]
    fn typed_text_skips_control_characters_and_shortcut_presses_type_nothing() {
        assert_eq!(
            press(character("é"), Modifiers::default(), Some("é"), IDLE),
            input(Input::Char('é'))
        );
        assert_eq!(
            press(
                Key::Named(Named::Delete),
                Modifiers::default(),
                Some("\u{7f}"),
                IDLE
            ),
            Keyed::Take(Vec::new())
        );
        let typed = plain(&character("a"), Some("a"), true, false);
        assert_eq!(typed, Vec::new());
        assert!(is_arrow(&Key::Named(Named::ArrowDown)));
        assert!(!is_arrow(&Key::Named(Named::Tab)));
    }

    #[test]
    fn the_duplicate_drag_key_is_the_platforms() {
        let shifted = mods(Modifiers::SHIFT);
        assert!(shifted.shift && !shifted.duplicate);
        let (duplicate, other) = if cfg!(target_os = "macos") {
            (Modifiers::ALT, Modifiers::CTRL)
        } else {
            (Modifiers::CTRL, Modifiers::ALT)
        };
        assert!(mods(duplicate).duplicate);
        assert!(!mods(other).duplicate);
        assert_eq!(mods(Modifiers::default()), machine::Mods::default());
    }
}
