//! Which keys a focused board takes, and what it makes of them (the plan's
//! §5.1): an input method's composition first, then Escape's ladder and a
//! held Space, then the host's [`KeyPolicy`], which knows the keymap. A
//! key the host claims is left to it; every other key is taken, so it
//! never falls through to the host's column keys, and is a command, an
//! edit of the open label ([`label`]), the connect query's text or nothing.
//! In a label, the keys the policy gives the board's select all,
//! clipboard, undo and redo are the text's own ([`label_command`]), so
//! they follow the host's keymap and its command key.

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
    /// Keyboard Connecting, so typed keys are its query.
    pub(crate) connecting: bool,
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
        // Held, Space turns a left drag into a pan; in a label and in the
        // connect query it is text.
        Key::Named(Named::Space) if !shortcut && !context.editing && !context.connecting => {
            let held = (!repeat).then_some(Out::Input(Input::Space { held: true }));
            return Keyed::Take(held.into_iter().collect());
        }
        _ => {}
    }
    // AltGr's text is the label's ahead of every binding, the app's too;
    // the app's own copy of the press ignores it (`wren-iced`'s `keys`).
    if context.editing {
        if let Some(typing) = alt_graph(key, physical, modifiers, text) {
            return Keyed::Take(typing.into_iter().map(Out::Label).collect());
        }
    }
    let route = match context.policy {
        Some(policy) => resolve(&**policy, key, physical, modifiers, text, repeat),
        // With no keymap to ask, a chord the label reads is the label's.
        None if shortcut && context.editing && label(key, physical, modifiers, text).is_some() => {
            KeyRoute::Pass
        }
        None if shortcut => KeyRoute::Host,
        None => KeyRoute::Pass,
    };
    match route {
        KeyRoute::Host => Keyed::Leave,
        // In keyboard Connecting a key without Ctrl, Alt or the logo key
        // is the query's or one it reads, bound or not, so the tool
        // letters type; every other key is taken and does nothing.
        _ if context.connecting => Keyed::Take(if shortcut {
            Vec::new()
        } else {
            plain(key, text, false, repeat)
        }),
        // Every key the host leaves is the label's, and is taken whether
        // or not the label has a use for it.
        _ if context.editing => Keyed::Take(in_label(
            key,
            physical,
            modifiers,
            text,
            repeat,
            route,
            context.policy,
        )),
        KeyRoute::Command(command) => Keyed::Take(vec![Out::Command(command)]),
        KeyRoute::Pass => Keyed::Take(plain(key, text, shortcut, repeat)),
    }
}

/// Where `policy` sends a press of `key` that typed `text`, as a focused
/// board reads it, for a host that reads its keys the same way (the keys
/// sheet's own key).
///
/// A keymap spells `?` as Shift+/, the key that types it on a US layout;
/// another layout types it elsewhere (Shift with `,` on AZERTY, with `ß` on
/// German), so a typed `?` is asked as Shift+/ first, as the digit row's
/// symbols are asked as digits (§5.1 rule 7). First, because the key that
/// types it may be another board command's by its place: on Dvorak it is
/// Shift with the key a keymap calls `[`. A key the app has bound stays the
/// app's, as it is ahead of every board key.
pub fn resolve(
    policy: &(dyn Fn(&Key, key::Physical, Modifiers, bool) -> KeyRoute + Send + Sync),
    key: &Key,
    physical: key::Physical,
    modifiers: Modifiers,
    text: Option<&str>,
    repeat: bool,
) -> KeyRoute {
    let shortcut = modifiers.control() || modifiers.alt() || modifiers.logo();
    let asked = (text == Some("?") && !shortcut).then(|| {
        policy(
            &Key::Character("/".into()),
            key::Physical::Code(key::Code::Slash),
            Modifiers::SHIFT,
            repeat,
        )
    });
    match (asked, policy(key, physical, modifiers, repeat)) {
        (Some(KeyRoute::Command(_)), KeyRoute::Host) => KeyRoute::Host,
        (Some(bound @ KeyRoute::Command(_)), _) => bound,
        (Some(asked), KeyRoute::Pass) => asked,
        (_, route) => route,
    }
}

/// What an open label makes of a key the host left to the board, which
/// the policy routed as `route` (§5.1 rule 3), in this order:
/// - text typed without Ctrl or the logo key (nor Alt off macOS) types,
///   whatever the key is bound to;
/// - the label's caret and erasing keys move and erase;
/// - commit and next leave the text, the only commands that do;
/// - the label's line break;
/// - a key the policy gives the board's select all, clipboard, undo or
///   redo is the text's. It is asked as a first press, since a held paste
///   pastes again in a label while the board's does not.
///
/// Without a `policy` to read those keys from, the label reads its fixed
/// chords instead.
fn in_label(
    key: &Key,
    physical: key::Physical,
    modifiers: Modifiers,
    text: Option<&str>,
    repeat: bool,
    route: KeyRoute,
    policy: Option<&KeyPolicy>,
) -> Vec<Out> {
    use key::Named;
    let edits = |actions: Vec<LabelAction>| actions.into_iter().map(Out::Label).collect();
    if let Some(typing) = typed(text, modifiers) {
        return edits(typing);
    }
    let caret_or_erase = matches!(
        key,
        Key::Named(
            Named::ArrowLeft
                | Named::ArrowRight
                | Named::ArrowUp
                | Named::ArrowDown
                | Named::Home
                | Named::End
                | Named::Backspace
                | Named::Delete
        )
    );
    if caret_or_erase {
        if let Some(fixed) = read_label(key, physical, modifiers, text, false) {
            return edits(fixed);
        }
    }
    if let KeyRoute::Command(command @ (Command::CommitLabel | Command::ConnectNext)) = route {
        return vec![Out::Command(command)];
    }
    let Some(policy) = policy else {
        return edits(label(key, physical, modifiers, text).unwrap_or_default());
    };
    if let Some(fixed) = read_label(key, physical, modifiers, text, false) {
        return edits(fixed);
    }
    let first = if repeat {
        policy(key, physical, modifiers, false)
    } else {
        route
    };
    match first {
        KeyRoute::Command(command) => edits(label_command(command).into_iter().collect()),
        KeyRoute::Host | KeyRoute::Pass => Vec::new(),
    }
}

/// The text AltGr types on Linux, where it can arrive as Ctrl+Alt: printable
/// text other than the key's own letter, which a real Ctrl+Alt chord would
/// echo if it typed anything.
fn alt_graph(
    key: &Key,
    physical: key::Physical,
    modifiers: Modifiers,
    text: Option<&str>,
) -> Option<Vec<LabelAction>> {
    if cfg!(target_os = "macos") || !(modifiers.control() && modifiers.alt()) || modifiers.logo() {
        return None;
    }
    let text: String = text?.chars().filter(|c| !c.is_control()).collect();
    let mut letters = text.chars();
    let echoes = match (letters.next(), letters.next(), key.to_latin(physical)) {
        (Some(typed), None, Some(own)) => typed.to_lowercase().eq(own.to_lowercase()),
        _ => false,
    };
    (!text.is_empty() && !echoes).then(|| vec![LabelAction::Insert(text)])
}

/// What an open label makes of the board command a key is bound to: the
/// board's select all, clipboard, undo and redo are the text's.
fn label_command(command: Command) -> Option<LabelAction> {
    Some(match command {
        Command::SelectAll => LabelAction::SelectAll,
        Command::Copy => LabelAction::Copy,
        Command::Cut => LabelAction::Cut,
        Command::Paste => LabelAction::Paste,
        Command::Undo => LabelAction::Undo,
        Command::Redo => LabelAction::Redo,
        _ => return None,
    })
}

/// What a key does in an open label (§5.9 rule 3) with no host policy, or
/// `None` for a key the label has no use for. Enter breaks the line, with
/// Shift or without; the arrows, Home and End move the caret, by words
/// and to the ends with the platform's modifiers, and select with Shift;
/// ⌘ or Ctrl with A, C, X, V, Z and Shift+Z are the text's own; anything
/// else that types, types. With a policy those chords are the policy's
/// ([`label_command`]).
pub(crate) fn label(
    key: &Key,
    physical: key::Physical,
    modifiers: Modifiers,
    text: Option<&str>,
) -> Option<Vec<LabelAction>> {
    read_label(key, physical, modifiers, text, true)
}

/// [`label`], reading the fixed ⌘ or Ctrl chords only with `chords`.
fn read_label(
    key: &Key,
    physical: key::Physical,
    modifiers: Modifiers,
    text: Option<&str>,
    chords: bool,
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
    if let Some(typing) = alt_graph(key, physical, modifiers, text) {
        return Some(typing);
    }
    if chords && command && !modifiers.alt() {
        return match (key.to_latin(physical), modifiers.shift()) {
            (Some('a'), false) => Some(vec![LabelAction::SelectAll]),
            (Some('c'), false) => Some(vec![LabelAction::Copy]),
            (Some('x'), false) => Some(vec![LabelAction::Cut]),
            (Some('v'), false) => Some(vec![LabelAction::Paste]),
            (Some('z'), false) => Some(vec![LabelAction::Undo]),
            (Some('z'), true) => Some(vec![LabelAction::Redo]),
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
/// It is Ctrl in both Linux modes, since window managers take Alt+drag.
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
        connecting: false,
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

    /// A `?` typed by another key than Shift+/, as on AZERTY (Shift with
    /// the `,` key), is asked as Shift+/; in a label it stays text.
    #[test]
    fn a_question_mark_from_any_key_is_asked_as_shift_slash() {
        let policy: KeyPolicy = Arc::new(|key, physical, modifiers, _| {
            let slash = physical == Physical::Code(Code::Slash);
            match key.as_ref() {
                Key::Character("/") if slash && modifiers == Modifiers::SHIFT => {
                    KeyRoute::Command(Command::Keys)
                }
                _ => KeyRoute::Pass,
            }
        });
        let ruled = Context {
            policy: Some(&policy),
            ..IDLE
        };
        let keys = Keyed::Take(vec![Out::Command(Command::Keys)]);
        let azerty = |text, modifiers, context| {
            route(
                &character(","),
                Physical::Code(Code::KeyM),
                modifiers,
                text,
                false,
                context,
            )
        };
        assert_eq!(azerty(Some("?"), Modifiers::SHIFT, ruled), keys);
        assert_eq!(
            route(
                &character("/"),
                Physical::Code(Code::Slash),
                Modifiers::SHIFT,
                Some("?"),
                false,
                ruled
            ),
            keys
        );
        // The `,` key without Shift, and `?` with a shortcut modifier.
        assert_eq!(
            azerty(Some(","), Modifiers::default(), ruled),
            press(character(","), Modifiers::default(), Some(","), IDLE)
        );
        assert_ne!(
            azerty(Some("?"), Modifiers::SHIFT | Modifiers::CTRL, ruled),
            keys
        );
        let editing = Context {
            editing: true,
            ..ruled
        };
        assert_eq!(
            azerty(Some("?"), Modifiers::SHIFT, editing),
            edits(&[insert("?")])
        );
    }

    /// On Dvorak `?` is Shift with the key a keymap calls `[`, which has a
    /// command of its own: the `?` typed wins, and the same key without
    /// one, or with the sheet unbound, keeps its command.
    #[test]
    fn a_question_mark_typed_by_another_command_s_key_is_still_asked_as_shift_slash() {
        let back = Command::Arrange(wren_board::edit::Stack::Backward);
        let bound = |sheet: bool| -> KeyPolicy {
            Arc::new(move |_, physical, modifiers, _| {
                let shifted = modifiers == Modifiers::SHIFT;
                match physical {
                    Physical::Code(Code::Slash) if shifted && sheet => {
                        KeyRoute::Command(Command::Keys)
                    }
                    Physical::Code(Code::BracketLeft) if shifted => KeyRoute::Command(back),
                    _ => KeyRoute::Pass,
                }
            })
        };
        let dvorak = |text, policy: &KeyPolicy| {
            let context = Context {
                policy: Some(policy),
                ..IDLE
            };
            route(
                &character("?"),
                Physical::Code(Code::BracketLeft),
                Modifiers::SHIFT,
                text,
                false,
                context,
            )
        };
        let took = |command| Keyed::Take(vec![Out::Command(command)]);
        assert_eq!(dvorak(Some("?"), &bound(true)), took(Command::Keys));
        assert_eq!(dvorak(Some("{"), &bound(true)), took(back));
        assert_eq!(dvorak(Some("?"), &bound(false)), took(back));
        // A key the app has bound is the app's, `?` or not.
        let host: KeyPolicy = Arc::new(|_, physical, _, _| match physical {
            Physical::Code(Code::Slash) => KeyRoute::Command(Command::Keys),
            _ => KeyRoute::Host,
        });
        assert_eq!(dvorak(Some("?"), &host), Keyed::Leave);
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

    /// §5.1 rule 3: in keyboard Connecting a printable key is the query's,
    /// bound or not, and only Enter, Escape, the arrows and Backspace act;
    /// the host's keys stay the host's.
    #[test]
    fn in_keyboard_connecting_a_letter_is_query_text() {
        let policy = policy();
        let connecting = Context {
            connecting: true,
            escapes: true,
            policy: Some(&policy),
            ..IDLE
        };
        let plain = Modifiers::default();
        let chars =
            |text: &str| Keyed::Take(text.chars().map(|c| Out::Input(Input::Char(c))).collect());
        // `d` arms the diamond tool elsewhere.
        assert_eq!(
            press(character("d"), plain, Some("d"), connecting),
            chars("d")
        );
        assert_eq!(
            press(
                character("d"),
                plain,
                Some("d"),
                Context {
                    policy: Some(&policy),
                    ..IDLE
                }
            ),
            Keyed::Take(vec![Out::Command(Command::Tool(Tool::Shape(
                wren_board::Shape::Diamond
            )))])
        );
        assert_eq!(
            press(character("c"), Modifiers::SHIFT, Some("C"), connecting),
            chars("C")
        );
        assert_eq!(
            press(Key::Named(Named::Space), plain, Some(" "), connecting),
            chars(" ")
        );
        let read = |key| input(Input::Key { key, repeat: false });
        for (key, read_as) in [
            (Named::Enter, machine::Key::Enter),
            (Named::Backspace, machine::Key::Backspace),
            (Named::ArrowRight, machine::Key::Arrow(Dir::Right)),
            (Named::ArrowUp, machine::Key::Arrow(Dir::Up)),
            (Named::Escape, machine::Key::Escape),
        ] {
            assert_eq!(
                press(Key::Named(key), plain, None, connecting),
                read(read_as),
                "{key:?}"
            );
        }
        // Tab, a Ctrl chord and Delete do nothing, and are not the host's.
        for (key, modifiers, text) in [
            (Key::Named(Named::Tab), plain, Some("\t")),
            (character("z"), Modifiers::COMMAND, None),
            (Key::Named(Named::ArrowRight), Modifiers::CTRL, None),
            (Key::Named(Named::Delete), plain, None),
        ] {
            assert_eq!(
                press(key.clone(), modifiers, text, connecting),
                Keyed::Take(Vec::new()),
                "{key:?}"
            );
        }
        // An Alt chord is the host's.
        assert_eq!(
            press(character("n"), Modifiers::ALT, None, connecting),
            Keyed::Leave
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
        // Ctrl+Y does not redo (discovery/keybindings.md §2 item 4).
        assert_eq!(lettered("y", Code::KeyY, command), None);
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
            connecting: false,
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

    /// The command key and whether the Ctrl editing keys bind beside it,
    /// for each scheme the host builds its board layer for.
    const MODES: [(&str, Modifiers, bool); 3] = [
        ("macOS", Modifiers::LOGO, false),
        ("Linux Ctrl mode", Modifiers::CTRL, false),
        ("Linux Alt mode", Modifiers::ALT, true),
    ];

    /// A board layer's select all, clipboard, undo, redo and duplicate keys
    /// in the shape the host builds them for a mode: the command key with
    /// A, C, X, V, D, Z and Shift+Z, and in Alt mode Ctrl with them too.
    fn mode_policy(command_key: Modifiers, ctrl_too: bool) -> KeyPolicy {
        Arc::new(move |key, physical, modifiers, _| {
            let held = modifiers.difference(Modifiers::SHIFT);
            if held != command_key && !(ctrl_too && held == Modifiers::CTRL) {
                return KeyRoute::Pass;
            }
            KeyRoute::Command(match (key.to_latin(physical), modifiers.shift()) {
                (Some('a'), false) => Command::SelectAll,
                (Some('c'), false) => Command::Copy,
                (Some('x'), false) => Command::Cut,
                (Some('v'), false) => Command::Paste,
                (Some('d'), false) => Command::Duplicate,
                (Some('z'), false) => Command::Undo,
                (Some('z'), true) => Command::Redo,
                _ => return KeyRoute::Pass,
            })
        })
    }

    /// discovery/keybindings.md §2 items 3 and 4, §4: an open label's select
    /// all, clipboard, undo and redo keys are the ones the host's policy
    /// gives the board's commands, so they follow the command key: ⌘ on
    /// macOS, Ctrl in Linux's Ctrl mode, and Alt, or Ctrl, in its Alt mode.
    /// Ctrl+Y redoes in none of them.
    #[test]
    fn in_a_label_the_policy_s_select_all_clipboard_and_undo_keys_are_the_text_s() {
        let table = [
            ("a", Code::KeyA, false, LabelAction::SelectAll),
            ("c", Code::KeyC, false, LabelAction::Copy),
            ("x", Code::KeyX, false, LabelAction::Cut),
            ("v", Code::KeyV, false, LabelAction::Paste),
            ("z", Code::KeyZ, false, LabelAction::Undo),
            ("z", Code::KeyZ, true, LabelAction::Redo),
        ];
        for (mode, command_key, ctrl_too) in MODES {
            let policy = mode_policy(command_key, ctrl_too);
            let ruled = Context {
                policy: Some(&policy),
                ..IDLE
            };
            let editing = Context {
                editing: true,
                escapes: true,
                ..ruled
            };
            let chord = |letter: &str, code, held: Modifiers, shift: bool, context| {
                let modifiers = if shift { held | Modifiers::SHIFT } else { held };
                route(
                    &character(letter),
                    Physical::Code(code),
                    modifiers,
                    None,
                    false,
                    context,
                )
            };
            let mut held = vec![command_key];
            if ctrl_too {
                held.push(Modifiers::CTRL);
            }
            for &key in &held {
                for (letter, code, shift, action) in table.clone() {
                    assert_eq!(
                        chord(letter, code, key, shift, editing),
                        edits(&[action]),
                        "{mode}: {letter} with {key:?}, Shift {shift}"
                    );
                }
                // Outside a label they are the board's commands.
                assert_eq!(
                    chord("c", Code::KeyC, key, false, ruled),
                    Keyed::Take(vec![Out::Command(Command::Copy)]),
                    "{mode}"
                );
                // A board command with no meaning in a label does nothing.
                assert_eq!(
                    chord("d", Code::KeyD, key, false, editing),
                    Keyed::Take(Vec::new()),
                    "{mode}"
                );
            }
            // Ctrl+Y, and the letters with a modifier the mode does not
            // bind, do nothing in the label.
            let unbound = [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO]
                .into_iter()
                .filter(|other| !held.contains(other));
            for other in unbound {
                for (letter, code, shift, _) in table.clone() {
                    assert_eq!(
                        chord(letter, code, other, shift, editing),
                        Keyed::Take(Vec::new()),
                        "{mode}: {letter} with {other:?}"
                    );
                }
            }
            for key in [command_key, Modifiers::CTRL] {
                assert_eq!(
                    chord("y", Code::KeyY, key, false, editing),
                    Keyed::Take(Vec::new()),
                    "{mode}: Y with {key:?}"
                );
            }
        }
    }

    /// The label's text keys are only what the policy binds: an unbound
    /// Ctrl+C copies nothing, a rebound chord copies, and a bare key bound
    /// to undo types its letter.
    #[test]
    fn a_label_s_text_keys_are_the_ones_the_policy_binds() {
        let policy: KeyPolicy = Arc::new(|key, _, modifiers, _| match key.as_ref() {
            Key::Character("c") if modifiers == Modifiers::CTRL | Modifiers::SHIFT => {
                KeyRoute::Command(Command::Copy)
            }
            Key::Character("u") if modifiers.is_empty() => KeyRoute::Command(Command::Undo),
            _ => KeyRoute::Pass,
        });
        let editing = Context {
            editing: true,
            escapes: true,
            policy: Some(&policy),
            ..IDLE
        };
        assert_eq!(
            press(character("c"), Modifiers::CTRL, None, editing),
            Keyed::Take(Vec::new())
        );
        assert_eq!(
            press(
                character("c"),
                Modifiers::CTRL | Modifiers::SHIFT,
                None,
                editing
            ),
            edits(&[LabelAction::Copy])
        );
        assert_eq!(
            press(character("u"), Modifiers::default(), Some("u"), editing),
            edits(&[insert("u")])
        );
    }

    /// An open label, with `policy` to ask.
    fn editing(policy: &KeyPolicy) -> Context<'_> {
        Context {
            editing: true,
            escapes: true,
            policy: Some(policy),
            ..IDLE
        }
    }

    /// `route` for a press of `key` on the `code` key.
    fn pressed(
        key: Key,
        code: Code,
        modifiers: Modifiers,
        text: Option<&str>,
        repeat: bool,
        context: Context<'_>,
    ) -> Keyed {
        route(&key, Physical::Code(code), modifiers, text, repeat, context)
    }

    /// A held paste pastes again in a label, though the host's policy, as
    /// the board's paste does not repeat, routes a held one nowhere.
    #[test]
    fn a_held_paste_pastes_again_in_a_label_and_once_on_the_board() {
        for (mode, command_key, ctrl_too) in MODES {
            let keys = mode_policy(command_key, ctrl_too);
            let policy: KeyPolicy = Arc::new(move |key, physical, modifiers, repeat| {
                match keys(key, physical, modifiers, repeat) {
                    KeyRoute::Command(Command::Paste) if repeat => KeyRoute::Pass,
                    route => route,
                }
            });
            let v = |repeat, context| {
                pressed(
                    character("v"),
                    Code::KeyV,
                    command_key,
                    None,
                    repeat,
                    context,
                )
            };
            for repeat in [false, true] {
                assert_eq!(
                    v(repeat, editing(&policy)),
                    edits(&[LabelAction::Paste]),
                    "{mode}, held {repeat}"
                );
            }
            let board = Context {
                policy: Some(&policy),
                ..IDLE
            };
            assert_eq!(
                v(false, board),
                Keyed::Take(vec![Out::Command(Command::Paste)])
            );
            assert_eq!(v(true, board), Keyed::Take(Vec::new()), "{mode}");
        }
    }

    /// Printable text typed bare or with Shift types in a label even on a
    /// key bound to commit or next; Tab and Mod+Enter keep both.
    #[test]
    fn typed_text_types_in_a_label_whatever_commit_or_next_it_is_bound_to() {
        let policy: KeyPolicy = Arc::new(|key, _, modifiers, _| match key.as_ref() {
            Key::Character("r") if modifiers == Modifiers::SHIFT => {
                KeyRoute::Command(Command::ConnectNext)
            }
            Key::Character("t") if modifiers.is_empty() => KeyRoute::Command(Command::CommitLabel),
            Key::Named(Named::Tab) => KeyRoute::Command(Command::ConnectNext),
            Key::Named(Named::Enter) if modifiers.command() => {
                KeyRoute::Command(Command::CommitLabel)
            }
            _ => KeyRoute::Pass,
        });
        let label = editing(&policy);
        let r = pressed(
            character("r"),
            Code::KeyR,
            Modifiers::SHIFT,
            Some("R"),
            false,
            label,
        );
        assert_eq!(r, edits(&[insert("R")]));
        let t = pressed(
            character("t"),
            Code::KeyT,
            Modifiers::empty(),
            Some("t"),
            false,
            label,
        );
        assert_eq!(t, edits(&[insert("t")]));
        let tab = pressed(
            Key::Named(Named::Tab),
            Code::Tab,
            Modifiers::empty(),
            Some("\t"),
            false,
            label,
        );
        assert_eq!(tab, Keyed::Take(vec![Out::Command(Command::ConnectNext)]));
        let commit = pressed(
            Key::Named(Named::Enter),
            Code::Enter,
            Modifiers::COMMAND,
            Some("\r"),
            false,
            label,
        );
        assert_eq!(
            commit,
            Keyed::Take(vec![Out::Command(Command::CommitLabel)])
        );
        // Outside a label the binding holds.
        let board = Context {
            policy: Some(&policy),
            ..IDLE
        };
        assert_eq!(
            pressed(
                character("r"),
                Code::KeyR,
                Modifiers::SHIFT,
                Some("R"),
                false,
                board
            ),
            Keyed::Take(vec![Out::Command(Command::ConnectNext)])
        );
    }

    /// The label's caret and erasing keys win over a policy that gives
    /// their chords the board's clipboard or undo; and on macOS, so does
    /// the text ⌥ with a letter types.
    #[test]
    fn the_label_s_own_keys_win_over_the_policy_s_label_actions() {
        let policy: KeyPolicy = Arc::new(|_, _, _, _| KeyRoute::Command(Command::Copy));
        let label = editing(&policy);
        let named = |named, code, modifiers| {
            pressed(Key::Named(named), code, modifiers, None, false, label)
        };
        assert_eq!(
            named(Named::ArrowLeft, Code::ArrowLeft, JUMP),
            edits(&[moved(Motion::WordLeft, false)])
        );
        assert_eq!(
            named(Named::ArrowRight, Code::ArrowRight, JUMP | Modifiers::SHIFT),
            edits(&[moved(Motion::WordRight, true)])
        );
        assert_eq!(
            named(Named::Backspace, Code::Backspace, JUMP),
            edits(&[moved(Motion::WordLeft, true), LabelAction::Backspace])
        );
        assert_eq!(
            named(Named::Delete, Code::Delete, JUMP),
            edits(&[moved(Motion::WordRight, true), LabelAction::Backspace])
        );
        assert_eq!(
            named(Named::Home, Code::Home, Modifiers::empty()),
            edits(&[moved(Motion::LineStart, false)])
        );
        assert_eq!(
            named(Named::End, Code::End, JUMP),
            edits(&[moved(Motion::End, false)])
        );
        // ⌥C types `ç` on macOS; off it, Alt is a command key and copies.
        let option_c = pressed(
            character("c"),
            Code::KeyC,
            Modifiers::ALT,
            Some("ç"),
            false,
            label,
        );
        if cfg!(target_os = "macos") {
            assert_eq!(option_c, edits(&[insert("ç")]));
        } else {
            assert_eq!(option_c, edits(&[LabelAction::Copy]));
        }
        // A chord with no fixed meaning is the policy's.
        assert_eq!(
            pressed(
                character("c"),
                Code::KeyC,
                Modifiers::COMMAND,
                None,
                false,
                label
            ),
            edits(&[LabelAction::Copy])
        );
    }

    /// Off macOS, AltGr can arrive as Ctrl+Alt with the text it types: in a
    /// label that text types, in either Linux mode, while a real Ctrl+Alt
    /// chord, which types no text or its own letter, keeps its binding.
    #[test]
    fn altgr_text_types_in_a_label_and_a_ctrl_alt_chord_keeps_its_binding() {
        let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
        for (mode, command_key, ctrl_too) in MODES {
            let keys = mode_policy(command_key, ctrl_too);
            let policy: KeyPolicy =
                Arc::new(move |key, physical, modifiers, repeat| match key.as_ref() {
                    Key::Character("e") if modifiers == ctrl_alt => {
                        KeyRoute::Command(Command::Undo)
                    }
                    _ => keys(key, physical, modifiers, repeat),
                });
            let label = editing(&policy);
            let e = |text| pressed(character("e"), Code::KeyE, ctrl_alt, text, false, label);
            if cfg!(target_os = "macos") {
                assert_eq!(e(Some("€")), edits(&[LabelAction::Undo]), "{mode}");
            } else {
                assert_eq!(e(Some("€")), edits(&[insert("€")]), "{mode}");
            }
            assert_eq!(e(None), edits(&[LabelAction::Undo]), "{mode}");
            assert_eq!(e(Some("\u{5}")), edits(&[LabelAction::Undo]), "{mode}");
            assert_eq!(e(Some("E")), edits(&[LabelAction::Undo]), "{mode}");
        }
        // And with no policy to ask.
        let altgr = label(
            &character("e"),
            Physical::Code(Code::KeyE),
            ctrl_alt,
            Some("€"),
        );
        assert_eq!(
            altgr,
            (!cfg!(target_os = "macos")).then(|| vec![insert("€")])
        );
    }

    /// A function key the policy gives the board's undo undoes in a label.
    #[test]
    fn a_function_key_s_label_action_is_the_label_s() {
        let policy: KeyPolicy = Arc::new(|key, _, modifiers, _| match key.as_ref() {
            Key::Named(Named::F6) if modifiers == Modifiers::SHIFT => {
                KeyRoute::Command(Command::Undo)
            }
            _ => KeyRoute::Pass,
        });
        let label = editing(&policy);
        let f6 = |modifiers| {
            pressed(
                Key::Named(Named::F6),
                Code::F6,
                modifiers,
                None,
                false,
                label,
            )
        };
        assert_eq!(f6(Modifiers::SHIFT), edits(&[LabelAction::Undo]));
        assert_eq!(f6(Modifiers::empty()), Keyed::Take(Vec::new()));
    }

    /// A label's undo is the board layer's, on macOS as on Linux: with
    /// Mod+Z given `board.unbind` it undoes nothing in a label, and with
    /// Mod+Z rebound to the board's copy and Mod+U to its undo, Mod+Z copies
    /// and Mod+U undoes. Typing, the caret and commit are as before.
    #[test]
    fn a_label_s_undo_follows_the_board_layer_unbound_or_rebound() {
        for (mode, command_key, ctrl_too) in MODES {
            let keys = mode_policy(command_key, ctrl_too);
            let z_held = move |key: &Key, physical, modifiers: Modifiers| {
                key.to_latin(physical) == Some('z')
                    && !modifiers.shift()
                    && modifiers == command_key
            };
            let keys_unbound = keys.clone();
            let unbound: KeyPolicy = Arc::new(move |key, physical, modifiers, repeat| {
                if z_held(key, physical, modifiers) {
                    return KeyRoute::Pass;
                }
                keys_unbound(key, physical, modifiers, repeat)
            });
            let rebound: KeyPolicy = Arc::new(move |key, physical, modifiers, repeat| {
                if z_held(key, physical, modifiers) {
                    return KeyRoute::Command(Command::Copy);
                }
                match key.as_ref() {
                    Key::Character("u") if modifiers == command_key => {
                        KeyRoute::Command(Command::Undo)
                    }
                    Key::Named(Named::Enter) if modifiers == command_key => {
                        KeyRoute::Command(Command::CommitLabel)
                    }
                    _ => keys(key, physical, modifiers, repeat),
                }
            });
            let chord = |letter: &str, code, modifiers, policy| {
                pressed(
                    character(letter),
                    code,
                    modifiers,
                    None,
                    false,
                    editing(policy),
                )
            };
            assert_eq!(
                chord("z", Code::KeyZ, command_key, &unbound),
                Keyed::Take(Vec::new()),
                "{mode}"
            );
            assert_eq!(
                chord("z", Code::KeyZ, command_key, &rebound),
                edits(&[LabelAction::Copy]),
                "{mode}"
            );
            assert_eq!(
                chord("u", Code::KeyU, command_key, &rebound),
                edits(&[LabelAction::Undo]),
                "{mode}"
            );
            for policy in [&unbound, &rebound] {
                // Redo and the clipboard keep their keys.
                assert_eq!(
                    chord("z", Code::KeyZ, command_key | Modifiers::SHIFT, policy),
                    edits(&[LabelAction::Redo]),
                    "{mode}"
                );
                assert_eq!(
                    chord("a", Code::KeyA, command_key, policy),
                    edits(&[LabelAction::SelectAll]),
                    "{mode}"
                );
                let label = editing(policy);
                assert_eq!(
                    pressed(
                        character("q"),
                        Code::KeyQ,
                        Modifiers::empty(),
                        Some("q"),
                        false,
                        label
                    ),
                    edits(&[insert("q")])
                );
                assert_eq!(
                    pressed(
                        Key::Named(Named::ArrowLeft),
                        Code::ArrowLeft,
                        JUMP,
                        None,
                        false,
                        label
                    ),
                    edits(&[moved(Motion::WordLeft, false)])
                );
            }
            assert_eq!(
                pressed(
                    Key::Named(Named::Enter),
                    Code::Enter,
                    command_key,
                    None,
                    false,
                    editing(&rebound)
                ),
                Keyed::Take(vec![Out::Command(Command::CommitLabel)]),
                "{mode}"
            );
        }
    }

    /// Branch review, severity 3: AltGr's text types in a label even where
    /// the app binds the Ctrl+Alt chord it arrives as; a real Ctrl+Alt
    /// chord stays the app's.
    #[test]
    fn altgr_text_types_in_a_label_ahead_of_the_app_s_binding() {
        let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
        let policy: KeyPolicy = Arc::new(move |key, _, modifiers, _| match key.as_ref() {
            Key::Character("e") if modifiers == ctrl_alt => KeyRoute::Host,
            _ => KeyRoute::Pass,
        });
        let label = editing(&policy);
        let e = |text| pressed(character("e"), Code::KeyE, ctrl_alt, text, false, label);
        if cfg!(target_os = "macos") {
            assert_eq!(e(Some("€")), Keyed::Leave);
        } else {
            assert_eq!(e(Some("€")), edits(&[insert("€")]));
        }
        assert_eq!(e(None), Keyed::Leave);
        assert_eq!(e(Some("e")), Keyed::Leave);
        // Outside a label the board reads no text with Ctrl held.
        let board = Context {
            policy: Some(&policy),
            ..IDLE
        };
        assert_eq!(
            pressed(
                character("e"),
                Code::KeyE,
                ctrl_alt,
                Some("€"),
                false,
                board
            ),
            Keyed::Leave
        );
    }

    /// Branch review, severity 3: with `backspace = board.connect.next`,
    /// Backspace erases in a label; the caret and erasing keys come before
    /// a rebound commit or next, and Tab and Mod+Enter still leave it.
    #[test]
    fn the_caret_and_erase_keys_come_before_a_rebound_commit_or_next() {
        let policy: KeyPolicy = Arc::new(|key, _, modifiers, _| match key.as_ref() {
            Key::Named(Named::Backspace | Named::ArrowLeft | Named::Tab) => {
                KeyRoute::Command(Command::ConnectNext)
            }
            Key::Named(Named::Delete | Named::Home) => KeyRoute::Command(Command::CommitLabel),
            Key::Named(Named::Enter) if modifiers.command() || modifiers.shift() => {
                KeyRoute::Command(Command::CommitLabel)
            }
            _ => KeyRoute::Pass,
        });
        let label = editing(&policy);
        let named = |named, code, modifiers| {
            pressed(Key::Named(named), code, modifiers, None, false, label)
        };
        let plain = Modifiers::empty();
        assert_eq!(
            named(Named::Backspace, Code::Backspace, plain),
            edits(&[LabelAction::Backspace])
        );
        assert_eq!(
            named(Named::Backspace, Code::Backspace, JUMP),
            edits(&[moved(Motion::WordLeft, true), LabelAction::Backspace])
        );
        assert_eq!(
            named(Named::Delete, Code::Delete, plain),
            edits(&[LabelAction::Delete])
        );
        assert_eq!(
            named(Named::ArrowLeft, Code::ArrowLeft, plain),
            edits(&[moved(Motion::Left, false)])
        );
        assert_eq!(
            named(Named::Home, Code::Home, plain),
            edits(&[moved(Motion::LineStart, false)])
        );
        let leaves = |command| Keyed::Take(vec![Out::Command(command)]);
        assert_eq!(
            named(Named::Tab, Code::Tab, plain),
            leaves(Command::ConnectNext)
        );
        assert_eq!(
            named(Named::Enter, Code::Enter, Modifiers::COMMAND),
            leaves(Command::CommitLabel)
        );
        // A line break is no caret key: a rebound Shift+Enter commits.
        assert_eq!(
            named(Named::Enter, Code::Enter, Modifiers::SHIFT),
            leaves(Command::CommitLabel)
        );
        let board = Context {
            policy: Some(&policy),
            ..IDLE
        };
        assert_eq!(
            pressed(
                Key::Named(Named::Backspace),
                Code::Backspace,
                Modifiers::empty(),
                None,
                false,
                board
            ),
            Keyed::Take(vec![Out::Command(Command::ConnectNext)])
        );
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
