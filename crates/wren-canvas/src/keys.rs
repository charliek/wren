//! Which keys a focused board takes, and what it makes of them (the plan's
//! §5.1): an input method's composition first, then Escape's ladder and a
//! held Space, then the host's [`KeyPolicy`], which knows the keymap. A
//! key the host claims is left to it; every other key is taken, so it
//! never falls through to the host's column keys, and is a command, an
//! edit of the open label ([`label`]), the connect query's text or nothing.

use std::sync::Arc;

use iced::keyboard::{key, Key, Modifiers};
use wren_board::Dir;

use crate::command::Command;
use crate::label::{LabelAction, Motion};
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
    Label(LabelAction),
}

/// The board's state, as far as a key press reads it.
#[derive(Clone, Copy)]
pub(crate) struct Context<'a> {
    /// A label is open, so keys are its text.
    pub(crate) editing: bool,
    /// An input method holds preedit text, so every key is its own.
    pub(crate) composing: bool,
    /// Escape has a rung of its ladder to act on ([`machine::Machine::escapes`]).
    pub(crate) escapes: bool,
    /// The key was first pressed before the label opened, as the Enter
    /// that opens one is: its repeats are not the label's.
    pub(crate) held_from_before: bool,
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
    // Whatever the key and the modifiers: Escape, Enter and Tab end or
    // steer a composition, and must neither leave the label nor reach the
    // host (§5.9 rule 9, §5.14 rung 1).
    if context.composing {
        return Keyed::Take(Vec::new());
    }
    // A held key does one thing (§5.1 rule 4): the Enter that opened a
    // label must not go on to replace its text with line breaks.
    if repeat && context.editing && context.held_from_before {
        return Keyed::Take(Vec::new());
    }
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
    let edits = || label(key, physical, modifiers, text);
    let route = match context.policy {
        Some(policy) => policy(key, physical, modifiers, repeat),
        // With no keymap to ask, a chord the label reads is the label's.
        None if shortcut && context.editing && edits().is_some() => KeyRoute::Pass,
        None if shortcut => KeyRoute::Host,
        None => KeyRoute::Pass,
    };
    match route {
        KeyRoute::Host => Keyed::Leave,
        // In a label only these two leave the text (§5.1 rule 3); every
        // other key is the label's, and is taken whether or not the label
        // has a use for it.
        KeyRoute::Command(command @ (Command::CommitLabel | Command::ConnectNext))
            if context.editing =>
        {
            Keyed::Take(vec![Out::Command(command)])
        }
        KeyRoute::Command(_) | KeyRoute::Pass if context.editing => {
            let actions = edits().unwrap_or_default();
            Keyed::Take(actions.into_iter().map(Out::Label).collect())
        }
        KeyRoute::Command(command) => Keyed::Take(vec![Out::Command(command)]),
        KeyRoute::Pass => Keyed::Take(plain(key, text, shortcut, repeat)),
    }
}

/// What a key does in an open label (§5.9 rule 3), or `None` for a key
/// the label has no use for. Enter breaks the line, with Shift or without;
/// the arrows, Home and End move the caret, by words and to the ends with
/// the platform's modifiers, and select with Shift; ⌘ or Ctrl with A, C,
/// X, V, Z and Y are the text's own; anything else that types, types.
pub(crate) fn label(
    key: &Key,
    physical: key::Physical,
    modifiers: Modifiers,
    text: Option<&str>,
) -> Option<Vec<LabelAction>> {
    use key::Named;
    let select = modifiers.shift();
    let moved = |motion| Some(vec![LabelAction::Move { motion, select }]);
    // Erase from the caret to where `motion` takes it.
    let erase = |motion| {
        let reach = LabelAction::Move {
            motion,
            select: true,
        };
        Some(vec![reach, LabelAction::Backspace])
    };
    let (jump, command) = (modifiers.jump(), modifiers.command());
    // ⌘ with an arrow goes to the end of the line or of the text.
    let far = modifiers.macos_command();
    if let Key::Named(named) = key {
        // Alt is the app's modifier off macOS, and ⌥ the word key on it;
        // ⌘ and Ctrl beside it have no meaning here.
        let chord = if cfg!(target_os = "macos") {
            modifiers.control()
        } else {
            modifiers.alt() || modifiers.logo()
        };
        if chord {
            return None;
        }
        return match named {
            Named::Enter if !command && !jump => Some(vec![LabelAction::Insert("\n".to_owned())]),
            Named::Backspace if far => erase(Motion::LineStart),
            Named::Backspace if jump => erase(Motion::WordLeft),
            Named::Backspace => Some(vec![LabelAction::Backspace]),
            Named::Delete if jump => erase(Motion::WordRight),
            Named::Delete => Some(vec![LabelAction::Delete]),
            Named::ArrowLeft if far => moved(Motion::LineStart),
            Named::ArrowRight if far => moved(Motion::LineEnd),
            Named::ArrowUp if far => moved(Motion::Start),
            Named::ArrowDown if far => moved(Motion::End),
            Named::ArrowLeft if jump => moved(Motion::WordLeft),
            Named::ArrowRight if jump => moved(Motion::WordRight),
            Named::ArrowLeft => moved(Motion::Left),
            Named::ArrowRight => moved(Motion::Right),
            Named::ArrowUp => moved(Motion::Up),
            Named::ArrowDown => moved(Motion::Down),
            Named::Home if jump => moved(Motion::Start),
            Named::End if jump => moved(Motion::End),
            Named::Home => moved(Motion::LineStart),
            Named::End => moved(Motion::LineEnd),
            Named::Space if !command => typed(text, modifiers),
            _ => None,
        };
    }
    if command && !modifiers.alt() {
        return match (key.to_latin(physical), modifiers.shift()) {
            (Some('a'), false) => Some(vec![LabelAction::SelectAll]),
            (Some('c'), false) => Some(vec![LabelAction::Copy]),
            (Some('x'), false) => Some(vec![LabelAction::Cut]),
            (Some('v'), false) => Some(vec![LabelAction::Paste]),
            (Some('z'), false) => Some(vec![LabelAction::Undo]),
            (Some('z'), true) => Some(vec![LabelAction::Redo]),
            (Some('y'), false) if !cfg!(target_os = "macos") => Some(vec![LabelAction::Redo]),
            _ => None,
        };
    }
    typed(text, modifiers)
}

/// The text a key press types into a label. A key with ⌘ or Ctrl held
/// never types (§5.9 rule 3), nor one with Alt off macOS, where ⌥ is how
/// accented letters are typed.
fn typed(text: Option<&str>, modifiers: Modifiers) -> Option<Vec<LabelAction>> {
    let alt = modifiers.alt() && !cfg!(target_os = "macos");
    if modifiers.control() || modifiers.logo() || alt {
        return None;
    }
    let text: String = text?.chars().filter(|c| !c.is_control()).collect();
    (!text.is_empty()).then(|| vec![LabelAction::Insert(text)])
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
        composing: false,
        escapes: false,
        held_from_before: false,
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

    fn edits(actions: &[LabelAction]) -> Keyed {
        Keyed::Take(actions.iter().cloned().map(Out::Label).collect())
    }

    fn insert(text: &str) -> LabelAction {
        LabelAction::Insert(text.to_owned())
    }

    fn moved(motion: Motion, select: bool) -> LabelAction {
        LabelAction::Move { motion, select }
    }

    /// The modifier that widens a caret move to a word: Ctrl, or ⌥ on
    /// macOS.
    const JUMP: Modifiers = if cfg!(target_os = "macos") {
        Modifiers::ALT
    } else {
        Modifiers::CTRL
    };

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
                Key::Named(Named::Enter) if modifiers.command() => {
                    KeyRoute::Command(Command::CommitLabel)
                }
                Key::Named(Named::Enter) if !repeat => KeyRoute::Command(Command::EditLabel),
                Key::Named(Named::ArrowRight) => KeyRoute::Command(Command::Select(Dir::Right)),
                Key::Character("z") if modifiers.command() => KeyRoute::Command(Command::Undo),
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
            edits(&[LabelAction::Insert(" ".to_owned())])
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
            ..IDLE
        };
        let plain = Modifiers::default();
        assert_eq!(
            press(Key::Named(Named::Enter), Modifiers::COMMAND, None, editing),
            Keyed::Take(vec![Out::Command(Command::CommitLabel)])
        );
        assert_eq!(
            press(Key::Named(Named::Tab), plain, Some("\t"), editing),
            Keyed::Take(vec![Out::Command(Command::ConnectNext)])
        );
        // Bound to a board command or not, the rest are the label's.
        let table = [
            (Key::Named(Named::Enter), plain, Some("\r"), insert("\n")),
            (
                Key::Named(Named::ArrowRight),
                plain,
                None,
                moved(Motion::Right, false),
            ),
            (character("d"), plain, Some("d"), insert("d")),
            (character("z"), Modifiers::COMMAND, None, LabelAction::Undo),
            (character("q"), plain, Some("q"), insert("q")),
        ];
        for (key, modifiers, text, action) in table {
            assert_eq!(
                press(key.clone(), modifiers, text, editing),
                edits(&[action]),
                "{key:?}"
            );
        }
        // A key the label has no use for is taken all the same.
        assert_eq!(
            press(Key::Named(Named::F5), plain, None, editing),
            Keyed::Take(Vec::new())
        );
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
    fn a_held_key_that_opened_the_label_does_not_type_in_it() {
        let policy = policy();
        let editing = Context {
            editing: true,
            escapes: true,
            policy: Some(&policy),
            ..IDLE
        };
        let enter = |context| {
            route(
                &Key::Named(Named::Enter),
                Physical::Code(Code::Enter),
                Modifiers::default(),
                Some("\r"),
                true,
                context,
            )
        };
        // The Enter that opened the label, held: its repeats do nothing.
        let opener = Context {
            held_from_before: true,
            ..editing
        };
        assert_eq!(enter(opener), Keyed::Take(Vec::new()));
        // An Enter first pressed in the label repeats there as text.
        assert_eq!(enter(editing), edits(&[insert("\n")]));
    }

    /// §5.9 rule 3, key by key: what the label makes of each.
    #[test]
    fn the_label_s_own_keys() {
        let plain = Modifiers::default();
        let shift = Modifiers::SHIFT;
        let command = Modifiers::COMMAND;
        let named = |named, modifiers| {
            label(
                &Key::Named(named),
                Physical::Code(Code::KeyA),
                modifiers,
                None,
            )
        };
        let one = |action| Some(vec![action]);
        // Enter breaks the line, with Shift or without.
        assert_eq!(named(Named::Enter, plain), one(insert("\n")));
        assert_eq!(named(Named::Enter, shift), one(insert("\n")));
        assert_eq!(named(Named::Enter, command), None);
        // The caret: by graphemes, by words, to the ends of the line and
        // of the text; Shift selects.
        for (key, motion) in [
            (Named::ArrowLeft, Motion::Left),
            (Named::ArrowRight, Motion::Right),
            (Named::ArrowUp, Motion::Up),
            (Named::ArrowDown, Motion::Down),
            (Named::Home, Motion::LineStart),
            (Named::End, Motion::LineEnd),
        ] {
            assert_eq!(named(key, plain), one(moved(motion, false)), "{key:?}");
            assert_eq!(named(key, shift), one(moved(motion, true)), "{key:?}");
        }
        for (key, motion) in [
            (Named::ArrowLeft, Motion::WordLeft),
            (Named::ArrowRight, Motion::WordRight),
            (Named::Home, Motion::Start),
            (Named::End, Motion::End),
        ] {
            assert_eq!(named(key, JUMP), one(moved(motion, false)), "{key:?}");
            assert_eq!(
                named(key, JUMP | shift),
                one(moved(motion, true)),
                "{key:?}"
            );
        }
        if cfg!(target_os = "macos") {
            for (key, motion) in [
                (Named::ArrowLeft, Motion::LineStart),
                (Named::ArrowRight, Motion::LineEnd),
                (Named::ArrowUp, Motion::Start),
                (Named::ArrowDown, Motion::End),
            ] {
                assert_eq!(named(key, command), one(moved(motion, false)), "{key:?}");
            }
            // Ctrl with an arrow is the desktop's.
            assert_eq!(named(Named::ArrowLeft, Modifiers::CTRL), None);
        } else {
            // Alt is the app's modifier, and the logo key the desktop's.
            assert_eq!(named(Named::ArrowLeft, Modifiers::ALT), None);
            assert_eq!(named(Named::Backspace, Modifiers::LOGO), None);
        }
        // Erasing: a grapheme, or a word.
        assert_eq!(named(Named::Backspace, plain), one(LabelAction::Backspace));
        assert_eq!(named(Named::Delete, plain), one(LabelAction::Delete));
        assert_eq!(
            named(Named::Backspace, JUMP),
            Some(vec![moved(Motion::WordLeft, true), LabelAction::Backspace])
        );
        assert_eq!(
            named(Named::Delete, JUMP),
            Some(vec![moved(Motion::WordRight, true), LabelAction::Backspace])
        );
        // Tab and the function keys are not the label's.
        assert_eq!(named(Named::Tab, plain), None);
        assert_eq!(named(Named::F2, plain), None);

        // The text's own commands, by the key's Latin letter whatever the
        // layout types.
        let lettered = |typed: &str, code, modifiers| {
            label(&character(typed), Physical::Code(code), modifiers, None)
        };
        for (letter, code, action) in [
            ("a", Code::KeyA, LabelAction::SelectAll),
            ("c", Code::KeyC, LabelAction::Copy),
            ("x", Code::KeyX, LabelAction::Cut),
            ("v", Code::KeyV, LabelAction::Paste),
            ("z", Code::KeyZ, LabelAction::Undo),
        ] {
            assert_eq!(lettered(letter, code, command), one(action), "{letter}");
        }
        assert_eq!(
            lettered("z", Code::KeyZ, command | shift),
            one(LabelAction::Redo)
        );
        assert_eq!(lettered("я", Code::KeyZ, command), one(LabelAction::Undo));
        assert_eq!(
            lettered("y", Code::KeyY, command),
            if cfg!(target_os = "macos") {
                None
            } else {
                one(LabelAction::Redo)
            }
        );
        assert_eq!(lettered("b", Code::KeyB, command), None);

        // Everything else types what it types, control characters left
        // out; a key with ⌘ or Ctrl held never types.
        let typing = |typed: &str, modifiers, text| {
            label(
                &character(typed),
                Physical::Code(Code::KeyA),
                modifiers,
                text,
            )
        };
        assert_eq!(typing("a", plain, Some("a")), one(insert("a")));
        assert_eq!(typing("a", shift, Some("A")), one(insert("A")));
        assert_eq!(typing("é", plain, Some("é")), one(insert("é")));
        assert_eq!(typing("]", plain, Some("]")), one(insert("]")));
        assert_eq!(typing("1", plain, Some("1")), one(insert("1")));
        assert_eq!(typing("a", plain, Some("\u{1}")), None);
        assert_eq!(typing("a", plain, None), None);
        assert_eq!(typing("b", command, Some("b")), None);
        assert_eq!(typing("b", Modifiers::CTRL, Some("b")), None);
        assert_eq!(typing("b", Modifiers::LOGO, Some("b")), None);
        assert_eq!(
            typing("å", Modifiers::ALT, Some("å")),
            if cfg!(target_os = "macos") {
                one(insert("å"))
            } else {
                None
            }
        );
        let space = label(
            &Key::Named(Named::Space),
            Physical::Code(Code::Space),
            shift,
            Some(" "),
        );
        assert_eq!(space, one(insert(" ")));
    }

    /// The plan's review attack 5: while an input method composes, no key
    /// commits the label, leaves it, runs a command or reaches the host.
    #[test]
    fn while_an_input_method_composes_every_key_is_its_own() {
        let policy = policy();
        let composing = Context {
            editing: true,
            composing: true,
            escapes: true,
            held_from_before: false,
            policy: Some(&policy),
        };
        let plain = Modifiers::default();
        for (key, modifiers, text) in [
            (Key::Named(Named::Escape), plain, None),
            (Key::Named(Named::Enter), plain, Some("\r")),
            (Key::Named(Named::Enter), Modifiers::COMMAND, None),
            (Key::Named(Named::Tab), plain, Some("\t")),
            (Key::Named(Named::Backspace), plain, None),
            (Key::Named(Named::Delete), plain, None),
            (Key::Named(Named::ArrowLeft), plain, None),
            (Key::Named(Named::ArrowDown), Modifiers::SHIFT, None),
            (Key::Named(Named::Space), plain, Some(" ")),
            (character("d"), plain, Some("d")),
            (character("z"), Modifiers::COMMAND, None),
            // Even a key the host would claim.
            (character("k"), Modifiers::ALT, Some("k")),
        ] {
            for context in [
                composing,
                Context {
                    policy: None,
                    ..composing
                },
            ] {
                assert_eq!(
                    press(key.clone(), modifiers, text, context),
                    Keyed::Take(Vec::new()),
                    "{key:?} with {modifiers:?}"
                );
            }
        }
    }

    #[test]
    fn without_a_policy_a_chord_the_label_reads_is_the_label_s() {
        let editing = Context {
            editing: true,
            escapes: true,
            ..IDLE
        };
        assert_eq!(
            press(character("a"), Modifiers::COMMAND, None, editing),
            edits(&[LabelAction::SelectAll])
        );
        assert_eq!(
            press(Key::Named(Named::ArrowLeft), JUMP, None, editing),
            edits(&[moved(Motion::WordLeft, false)])
        );
        // Any other chord is the host's, as it is outside a label.
        assert_eq!(
            press(character("k"), Modifiers::COMMAND, None, editing),
            Keyed::Leave
        );
        assert_eq!(
            press(Key::Named(Named::Enter), Modifiers::COMMAND, None, editing),
            Keyed::Leave
        );
        assert_eq!(
            press(character("a"), Modifiers::COMMAND, None, IDLE),
            Keyed::Leave
        );
        // Unmodified keys are the label's text and its caret.
        assert_eq!(
            press(character("r"), Modifiers::default(), Some("r"), editing),
            edits(&[insert("r")])
        );
        assert_eq!(
            press(Key::Named(Named::Tab), Modifiers::default(), None, editing),
            Keyed::Take(Vec::new())
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
