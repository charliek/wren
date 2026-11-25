//! The editor's own shortcuts, checked before iced's stock key bindings.

use std::sync::Arc;

use iced::keyboard::key::{self, Named};
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{self, Binding, KeyPress, Motion};

use crate::popover::{is_plain_escape, Accelerator, MenuKey, Mode, OpenMenu, Stroke};
use crate::{Intent, Message, SpellEvent};

/// Where the host sends a key press that reaches the focused editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRoute {
    /// A host shortcut. The editor ignores the key, so a shortcut never also
    /// edits the text.
    Host,
    /// A host-bound editor command, carried out as [`Message::Intent`].
    Intent(Intent),
    /// A host-bound request for corrections at the caret, carried out as
    /// [`Message::Spell`] with [`SpellEvent::OpenAtCaret`].
    Suggest,
    /// A host-bound request for the popover the host names with this
    /// number, carried out as [`Message::Spell`] with
    /// [`SpellEvent::Request`]: the host's [`crate::MenuSource`] says what
    /// opens.
    Popover(u32),
    /// A host command the host names with this number, which acts on the
    /// editor's text: the editor captures the key and publishes
    /// [`Message::Command`] in its order with the batch's other messages,
    /// and does nothing else with it.
    Command(u32),
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

/// Where a key press goes.
pub(crate) enum Routed {
    /// To the editor, which captures the press unless the binding is
    /// [`Binding::Unfocus`].
    Binding(Binding<Message>),
    /// Close the open menu, then handle the press as if it had not been
    /// open: through this binding, or, with none, not at all and without
    /// capturing it, so it still reaches the host.
    Close(Option<Binding<Message>>),
    /// Publish this message and leave the press uncaptured, so the host
    /// hears of it from its own key handling too.
    Pass(Message),
}

/// What is open, as key routing needs it; see [`crate::popover`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum Target<'a> {
    /// The suggestions menu.
    SpellMenu,
    /// The suggestions menu, asked for earlier in the batch, which may not
    /// have opened.
    SpellOpening,
    /// A menu of the host's, with its rows' accelerators.
    Menu(&'a [Option<Accelerator>]),
    /// Suggestions.
    Suggest,
    /// Nothing of the editor's: the host took the keyboard for a request
    /// of this batch.
    Keyboard,
}

impl<'a> Target<'a> {
    /// The target for a host's open `popover`.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn of(popover: &'a crate::Popover) -> Self {
        match popover.mode() {
            Mode::Menu if popover.is_spelling() => Self::SpellMenu,
            Mode::Menu => Self::Menu(popover.accelerators()),
            Mode::Suggest => Self::Suggest,
        }
    }

    /// Whether the target takes the presses made in the editor, as a menu
    /// does: `Some(true)` when the host has yet to say whether one opened.
    pub(crate) fn modal(self) -> Option<bool> {
        match self {
            Self::SpellMenu | Self::Menu(_) => Some(false),
            Self::SpellOpening => Some(true),
            Self::Suggest | Self::Keyboard => None,
        }
    }

    /// Whether the host closes the target for Escape from its own key
    /// handling, with no word from the editor: the suggestions menu.
    pub(crate) fn leaves_escape(self) -> bool {
        matches!(self, Self::SpellMenu | Self::SpellOpening)
    }
}

/// The suggestions menu.
impl From<OpenMenu> for Target<'_> {
    fn from(menu: OpenMenu) -> Self {
        if menu.opening {
            Self::SpellOpening
        } else {
            Self::SpellMenu
        }
    }
}

/// Route a key press, with `popover` open (rule 4.4.4). With no popover,
/// and for every press while an input method composes, routing is
/// [`key_binding`]'s alone: a popover takes no key from a composition.
pub(crate) fn route<'a>(
    key_press: KeyPress,
    context: Context,
    popover: Option<impl Into<Target<'a>>>,
) -> Option<Routed> {
    if !matches!(key_press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    match popover.map(Into::into) {
        None => key_binding(key_press, context).map(Routed::Binding),
        Some(_) if context.composing => key_binding(key_press, context).map(Routed::Binding),
        Some(Target::SpellMenu) => route_spell_menu(key_press, context, false),
        Some(Target::SpellOpening) => route_spell_menu(key_press, context, true),
        Some(Target::Menu(accelerators)) => route_menu(key_press, context, accelerators),
        Some(Target::Suggest) => route_suggest(key_press, context),
        Some(Target::Keyboard) => route_keyboard(key_press, context),
    }
}

/// Route a key press with the suggestions menu open, or, `opening`, after
/// a request for it earlier in the batch, which may not have opened it.
/// Up and Down with no modifier move its selection and Enter with no
/// modifier accepts the row the host's menu has selected, or closes a menu
/// with nothing to offer; after a request the three go to the host as
/// [`SpellEvent::Key`], except that one the policy gives the host closes
/// the menu and goes to the host uncaptured. Escape is the host's: the
/// editor gives it no binding, so it neither drops focus nor captures the
/// press, and the host closes the menu (rule 4.4.8). A modifier key alone
/// does nothing. Any other press closes the menu and is then routed by
/// [`key_binding`].
fn route_spell_menu(key_press: KeyPress, context: Context, opening: bool) -> Option<Routed> {
    let plain = key_press.modifiers.is_empty();
    let key = match key_press.key.as_ref() {
        Key::Named(Named::Escape) => return None,
        Key::Named(named) if is_modifier(named) => return None,
        Key::Named(Named::ArrowUp) if plain => MenuKey::Up,
        Key::Named(Named::ArrowDown) if plain => MenuKey::Down,
        Key::Named(Named::Enter) if plain => MenuKey::Enter,
        _ => return Some(Routed::Close(key_binding(key_press, context))),
    };
    let event = if opening {
        // With no menu open the host would take the key, so it must not
        // be captured for a menu the host may not have opened.
        let host = context.policy.is_some_and(|policy| {
            policy(&key_press.key, key_press.physical_key, key_press.modifiers) == KeyRoute::Host
        });
        if host {
            return Some(Routed::Close(None));
        }
        SpellEvent::Key(key)
    } else {
        key.on(Mode::Menu)?
    };
    Some(popover_event(event))
}

/// Route a key press with a menu of the host's open, whose rows have these
/// `accelerators`. Up and Down with no modifier move its selection, Enter
/// with no modifier accepts the selected row, and a row's accelerator
/// accepts the row, or for a row that cannot be chosen is taken and does
/// nothing, the menu staying open. Escape with no Ctrl, Alt or logo key goes to the host
/// in its order with the batch's other messages, uncaptured: the host
/// closes the menu for it and drops its own handling of the press
/// ([`crate::EditorView::take_claimed_escape`]). A modifier key alone does
/// nothing. Any other press closes the menu and is then routed by
/// [`key_binding`].
fn route_menu(
    key_press: KeyPress,
    context: Context,
    accelerators: &[Option<Accelerator>],
) -> Option<Routed> {
    let (key, modifiers) = (&key_press.key, key_press.modifiers);
    let event = match key.as_ref() {
        Key::Named(Named::Escape) if is_plain_escape(modifiers) => {
            return Some(Routed::Pass(Message::Spell(SpellEvent::Key(
                MenuKey::Escape,
            ))));
        }
        Key::Named(named) if is_modifier(named) => return None,
        Key::Named(Named::ArrowUp) if modifiers.is_empty() => SpellEvent::Move(-1),
        Key::Named(Named::ArrowDown) if modifiers.is_empty() => SpellEvent::Move(1),
        Key::Named(Named::Enter) if modifiers.is_empty() => SpellEvent::AcceptSelected,
        _ => {
            let text = key_press.text.as_deref();
            let pressed = |accelerator: &Accelerator, by_text: bool| {
                if by_text {
                    accelerator.matches_press(key, key_press.physical_key, modifiers, text)
                } else {
                    accelerator.matches(key, key_press.physical_key, modifiers)
                }
            };
            // The key itself first, then what it types, so a row's key is
            // never another row's by its text.
            let row = [false, true].into_iter().find_map(|by_text| {
                accelerators
                    .iter()
                    .enumerate()
                    .find_map(|(row, accelerator)| {
                        accelerator
                            .filter(|accelerator| pressed(accelerator, by_text))
                            .map(|accelerator| (row, accelerator))
                    })
            });
            match row {
                // A key that chooses nothing: taken, so it does not type.
                Some((_, accelerator)) if accelerator.is_inert() => {
                    return Some(Routed::Binding(Binding::Sequence(Vec::new())));
                }
                Some((row, _)) => SpellEvent::Accept(row),
                None => return Some(Routed::Close(key_binding(key_press, context))),
            }
        }
    };
    Some(popover_event(event))
}

/// Route a key press with suggestions open. Up, Down, Enter and Tab with
/// no modifier are theirs, whatever the host has bound them to: they go to
/// the host as [`SpellEvent::Key`], captured. Escape with no Ctrl, Alt or
/// logo key goes the same way but uncaptured, as on a menu of the host's.
/// Every other press is routed by [`key_binding`] as with nothing open,
/// and what it does to the text decides whether the suggestions stay.
fn route_suggest(key_press: KeyPress, context: Context) -> Option<Routed> {
    let plain = key_press.modifiers.is_empty();
    let key = match key_press.key.as_ref() {
        Key::Named(Named::Escape) if is_plain_escape(key_press.modifiers) => {
            return Some(Routed::Pass(Message::Spell(SpellEvent::Key(
                MenuKey::Escape,
            ))));
        }
        Key::Named(Named::ArrowUp) if plain => MenuKey::Up,
        Key::Named(Named::ArrowDown) if plain => MenuKey::Down,
        Key::Named(Named::Enter) if plain => MenuKey::Enter,
        Key::Named(Named::Tab) if plain => MenuKey::Tab,
        _ => return key_binding(key_press, context).map(Routed::Binding),
    };
    Some(popover_event(SpellEvent::Key(key)))
}

/// Route a key press made after a request or a right-click, earlier in
/// the batch, that the host answered by taking the keyboard. Nothing of
/// the editor's is open, so Escape and a key the policy gives the host are
/// the host's, as with nothing open, and a modifier alone does nothing.
/// Every other press is one the host's field would take: it is routed as
/// a [`SpellEvent::Stroke`], which the widget takes and drops, since input
/// in the frame of a handover is no one's ([`crate::popover`]). A key that
/// is neither named nor one character is no stroke, and is not taken.
fn route_keyboard(key_press: KeyPress, context: Context) -> Option<Routed> {
    match key_press.key.as_ref() {
        Key::Named(Named::Escape) => return key_binding(key_press, context).map(Routed::Binding),
        Key::Named(named) if is_modifier(named) => return None,
        _ => {}
    }
    let hosts = context.policy.is_some_and(|policy| {
        policy(&key_press.key, key_press.physical_key, key_press.modifiers) == KeyRoute::Host
    });
    if hosts {
        return None;
    }
    Stroke::of(&key_press).map(|stroke| popover_event(SpellEvent::Stroke(stroke)))
}

fn popover_event(event: SpellEvent) -> Routed {
    Routed::Binding(Binding::Custom(Message::Spell(event)))
}

/// Whether `named` is a modifier key, which pressed alone leaves an open
/// menu alone.
pub(crate) fn is_modifier(named: Named) -> bool {
    matches!(
        named,
        Named::Alt
            | Named::AltGraph
            | Named::CapsLock
            | Named::Control
            | Named::Fn
            | Named::FnLock
            | Named::Hyper
            | Named::Meta
            | Named::NumLock
            | Named::Shift
            | Named::Super
            | Named::Symbol
            | Named::SymbolLock
    )
}

/// The messages a binding publishes, for key routing run without the
/// widget: what [`crate::widget`] publishes for it, except that the
/// clipboard is not there to copy to or paste from, and dropping focus is
/// no message. `selected` is whether text is selected; with none, Copy and
/// Cut take `line`, the caret line's [`crate::Clip`] text, asked for only then.
pub(crate) fn messages(
    binding: Binding<Message>,
    selected: bool,
    line: &dyn Fn() -> String,
    out: &mut Vec<Message>,
) {
    use iced::widget::text_editor::{Action, Edit};

    let action = match binding {
        Binding::Custom(message) => return out.push(message),
        Binding::Sequence(sequence) => {
            for binding in sequence {
                messages(binding, selected, line, out);
            }
            return;
        }
        Binding::Move(motion) => Action::Move(motion),
        Binding::Select(motion) => Action::Select(motion),
        Binding::SelectWord => Action::SelectWord,
        Binding::SelectLine => Action::SelectLine,
        Binding::SelectAll => Action::SelectAll,
        Binding::Insert(c) => Action::Edit(Edit::Insert(c)),
        Binding::Enter => Action::Edit(Edit::Enter),
        Binding::Backspace => Action::Edit(Edit::Backspace),
        Binding::Delete => Action::Edit(Edit::Delete),
        Binding::Cut | Binding::Copy => {
            let line = (!selected).then(|| std::sync::Arc::new(line()));
            return clip_messages(matches!(binding, Binding::Cut), line, out);
        }
        Binding::Paste | Binding::Unfocus => return,
    };
    out.push(Message::Action(action));
}

/// What Copy (`cut` false) or Cut publishes once the clipboard has its text:
/// with nothing selected, the caret's whole `line`; otherwise word that a
/// selection went, so a later paste of the same text is not taken for a
/// copied line, and for Cut the delete. The widget and [`messages`] both
/// decide through here.
pub(crate) fn clip_messages(
    cut: bool,
    line: Option<std::sync::Arc<String>>,
    out: &mut Vec<Message>,
) {
    use crate::Clip;
    use iced::widget::text_editor::{Action, Edit};

    match line {
        Some(text) if cut => out.push(Message::Clip(Clip::CutLine(text))),
        Some(text) => out.push(Message::Clip(Clip::CopyLine(text))),
        None => {
            out.push(Message::Clip(Clip::Selection));
            if cut {
                out.push(Message::Action(Action::Edit(Edit::Delete)));
            }
        }
    }
}

/// Custom key-binding handler.  Leaves an input method its editing keys
/// while it composes, asks the host's [`KeyPolicy`], then
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
/// (macOS ⌘← as Home, ⌃H as Backspace) carry over; macOS's ⌃A, ⌃E, ⌃B, and
/// ⌃F motions are mapped here, as iced drops them. Backspace,
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
    let named = match key.as_ref() {
        Key::Named(named) => Some(named),
        _ => None,
    };
    // Whatever the modifiers: Ctrl+Enter is still an Enter to an input method.
    if context.composing
        && matches!(
            named,
            Some(Named::Enter | Named::Tab | Named::Backspace | Named::Delete)
        )
    {
        return None;
    }

    match context
        .policy
        .map_or(KeyRoute::Pass, |policy| policy(key, physical, m))
    {
        KeyRoute::Host => return None,
        // An editor command never edits the text under an open preedit.
        KeyRoute::Intent(_) if context.composing => return None,
        KeyRoute::Intent(wanted) => return Some(intent(wanted)),
        // An open menu would close at once: composing closes it.
        KeyRoute::Suggest if context.composing => return None,
        KeyRoute::Suggest => {
            return Some(Binding::Custom(Message::Spell(SpellEvent::OpenAtCaret)));
        }
        // Nor is a popover asked for while an input method composes.
        KeyRoute::Popover(_) if context.composing => return None,
        KeyRoute::Popover(id) => {
            return Some(Binding::Custom(Message::Spell(SpellEvent::Request(id))));
        }
        // Nor does a host command act on the text under a preedit.
        KeyRoute::Command(_) if context.composing => return None,
        KeyRoute::Command(id) => return Some(Binding::Custom(Message::Command(id))),
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

    let macos = cfg!(target_os = "macos");
    macos
        .then(|| ctrl_motion(key, m))
        .flatten()
        .or_else(|| Binding::from_key_press(key_press))
        .filter(|binding| types_with(binding, m, macos))
        .map(|binding| editing(binding, m, context.markdown_assist))
}

/// macOS's ⌃A, ⌃E, ⌃B, and ⌃F. iced 0.14 converts them to Home, End, Left,
/// and Right, then reads the unconverted key, so it never moves for them.
fn ctrl_motion(key: &Key, m: Modifiers) -> Option<Binding<Message>> {
    if m != Modifiers::CTRL {
        return None;
    }
    let motion = match key.as_ref() {
        Key::Character("a") => Motion::Home,
        Key::Character("e") => Motion::End,
        Key::Character("b") => Motion::Left,
        Key::Character("f") => Motion::Right,
        _ => return None,
    };
    Some(Binding::Move(motion))
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

    /// A host that binds Cmd/Ctrl+Enter and Cmd/Ctrl+B to editor commands,
    /// and Cmd/Ctrl+F and Cmd/Ctrl+Z to its own.
    fn host_policy() -> KeyPolicy {
        Arc::new(|key: &Key, _, modifiers: Modifiers| {
            if !modifiers.command() {
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
        let command = Modifiers::COMMAND;
        assert_eq!(
            named(Named::Enter, command, context),
            Some(Intent::ToggleTask)
        );
        let letter =
            |letter: &str, context| press(Key::Character(letter.into()), command, None, context);
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
                modifiers: command,
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
    fn the_suggest_route_asks_for_the_menu_at_the_caret_with_or_without_assist() {
        let policy: KeyPolicy = Arc::new(|key: &Key, _, modifiers: Modifiers| {
            if modifiers == Modifiers::ALT && *key == Key::Character(".".into()) {
                KeyRoute::Suggest
            } else {
                KeyRoute::Pass
            }
        });
        for context in [ASSIST, PLAIN] {
            let context = Context {
                policy: Some(&policy),
                ..context
            };
            assert!(matches!(
                press(
                    Key::Character(".".into()),
                    Modifiers::ALT,
                    Some("."),
                    context
                ),
                Some(Binding::Custom(Message::Spell(SpellEvent::OpenAtCaret)))
            ));
            // Any other press of the key types it.
            assert!(matches!(
                press(
                    Key::Character(".".into()),
                    Modifiers::empty(),
                    Some("."),
                    context
                ),
                Some(Binding::Insert('.'))
            ));
        }
        let composing = Context {
            policy: Some(&policy),
            ..COMPOSING
        };
        assert!(press(
            Key::Character(".".into()),
            Modifiers::ALT,
            Some("."),
            composing
        )
        .is_none());
    }

    /// How a focused press routes with a menu open whose selected row is
    /// `selected`.
    fn routed(
        key: Key,
        modifiers: Modifiers,
        text: Option<&str>,
        context: Context,
        selected: Option<usize>,
    ) -> Option<Routed> {
        route(
            KeyPress {
                modified_key: key.clone(),
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                key,
                modifiers,
                text: text.map(Into::into),
                status: text_editor::Status::Focused { is_hovered: false },
            },
            context,
            Some(OpenMenu {
                selected,
                items: 3,
                opening: false,
            }),
        )
    }

    /// The menu event a press routes to, if it routes to one.
    fn menu_event(routed: Option<Routed>) -> Option<SpellEvent> {
        match routed {
            Some(Routed::Binding(Binding::Custom(Message::Spell(event)))) => Some(event),
            _ => None,
        }
    }

    #[test]
    fn an_open_menu_takes_plain_up_down_and_enter() {
        let none = Modifiers::empty();
        let named =
            |named, selected| menu_event(routed(Key::Named(named), none, None, ASSIST, selected));
        assert_eq!(named(Named::ArrowUp, Some(1)), Some(SpellEvent::Move(-1)));
        assert_eq!(named(Named::ArrowDown, Some(1)), Some(SpellEvent::Move(1)));
        // Enter accepts whatever the host's menu has selected, which closes
        // a menu with only the inactive row; Up and Down are still the
        // menu's then, so neither reaches the text.
        assert_eq!(
            named(Named::Enter, Some(2)),
            Some(SpellEvent::AcceptSelected)
        );
        assert_eq!(named(Named::Enter, None), Some(SpellEvent::AcceptSelected));
        assert_eq!(named(Named::ArrowDown, None), Some(SpellEvent::Move(1)));
        // The same without Markdown assist.
        assert_eq!(
            menu_event(routed(
                Key::Named(Named::Enter),
                none,
                Some("\r"),
                PLAIN,
                Some(0)
            )),
            Some(SpellEvent::AcceptSelected)
        );
    }

    #[test]
    fn escape_and_a_lone_modifier_leave_an_open_menu_alone() {
        for (key, modifiers) in [
            (Named::Escape, Modifiers::empty()),
            (Named::Escape, Modifiers::SHIFT),
            (Named::Escape, Modifiers::COMMAND),
            (Named::Shift, Modifiers::SHIFT),
            (Named::Control, Modifiers::CTRL),
            (Named::Alt, Modifiers::ALT),
            (Named::Super, Modifiers::LOGO),
            (Named::Meta, Modifiers::empty()),
            (Named::CapsLock, Modifiers::empty()),
        ] {
            // No binding: Escape neither unfocuses nor is captured.
            assert!(
                routed(Key::Named(key), modifiers, None, ASSIST, Some(0)).is_none(),
                "{key:?} {modifiers:?}"
            );
        }
        // Without a menu, Escape drops focus as before.
        assert!(matches!(
            press(Key::Named(Named::Escape), Modifiers::empty(), None, ASSIST),
            Some(Binding::Unfocus)
        ));
    }

    #[test]
    fn any_other_key_closes_the_menu_then_routes_as_if_it_were_shut() {
        let close = |key: Key, modifiers, text: Option<&str>, context| match routed(
            key,
            modifiers,
            text,
            context,
            Some(0),
        ) {
            Some(Routed::Close(binding)) => binding,
            _ => panic!("not closed"),
        };
        assert!(matches!(
            close(
                Key::Character("a".into()),
                Modifiers::empty(),
                Some("a"),
                ASSIST
            ),
            Some(Binding::Insert('a'))
        ));
        // Modified arrows and a modified Enter.
        assert!(matches!(
            close(Key::Named(Named::ArrowDown), Modifiers::SHIFT, None, ASSIST),
            Some(Binding::Select(Motion::Down))
        ));
        assert!(matches!(
            close(
                Key::Named(Named::Enter),
                Modifiers::SHIFT,
                Some("\r"),
                ASSIST
            ),
            Some(Binding::Custom(Message::Intent(Intent::ShiftEnter)))
        ));
        assert!(matches!(
            close(
                Key::Named(Named::ArrowLeft),
                Modifiers::empty(),
                None,
                ASSIST
            ),
            Some(Binding::Move(Motion::Left))
        ));
        assert!(matches!(
            close(
                Key::Named(Named::Backspace),
                Modifiers::empty(),
                None,
                ASSIST
            ),
            Some(Binding::Custom(Message::Intent(Intent::Backspace)))
        ));
        assert!(matches!(
            close(Key::Character("z".into()), Modifiers::COMMAND, None, ASSIST),
            Some(Binding::Custom(Message::Undo))
        ));
        // A host shortcut closes the menu and still reaches the host.
        let policy = host_policy();
        let hosted = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        assert!(close(Key::Character("f".into()), Modifiers::COMMAND, None, hosted).is_none());
        // A key the editor ignores closes it too.
        assert!(close(Key::Named(Named::F5), Modifiers::empty(), None, ASSIST).is_none());
    }

    #[test]
    fn the_suggest_key_on_an_open_menu_closes_and_opens_it_again() {
        let policy: KeyPolicy = Arc::new(|key: &Key, _, modifiers: Modifiers| {
            if modifiers == Modifiers::ALT && *key == Key::Character(".".into()) {
                KeyRoute::Suggest
            } else {
                KeyRoute::Pass
            }
        });
        let context = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        assert!(matches!(
            routed(
                Key::Character(".".into()),
                Modifiers::ALT,
                None,
                context,
                Some(0)
            ),
            Some(Routed::Close(Some(Binding::Custom(Message::Spell(
                SpellEvent::OpenAtCaret
            )))))
        ));
    }

    #[test]
    fn an_unfocused_editor_routes_nothing_to_its_menu() {
        let routed = route(
            KeyPress {
                key: Key::Named(Named::ArrowDown),
                modified_key: Key::Named(Named::ArrowDown),
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                modifiers: Modifiers::empty(),
                text: None,
                status: text_editor::Status::Active,
            },
            ASSIST,
            Some(OpenMenu {
                selected: Some(0),
                items: 3,
                opening: false,
            }),
        );
        assert!(routed.is_none());
    }

    #[test]
    fn a_menu_opened_earlier_in_the_batch_sends_its_keys_to_the_host() {
        let opening = Some(OpenMenu {
            selected: None,
            items: 0,
            opening: true,
        });
        let routed = |named, modifiers| {
            route(
                KeyPress {
                    modified_key: Key::Named(named),
                    key: Key::Named(named),
                    physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                    modifiers,
                    text: None,
                    status: text_editor::Status::Focused { is_hovered: false },
                },
                ASSIST,
                opening,
            )
        };
        let none = Modifiers::empty();
        for (named, key) in [
            (Named::ArrowUp, MenuKey::Up),
            (Named::ArrowDown, MenuKey::Down),
            (Named::Enter, MenuKey::Enter),
        ] {
            assert_eq!(
                menu_event(routed(named, none)),
                Some(SpellEvent::Key(key)),
                "{named:?}"
            );
        }
        // Escape and a lone modifier stay as with any open menu, and any
        // other press closes it first.
        assert!(routed(Named::Escape, none).is_none());
        assert!(routed(Named::Shift, Modifiers::SHIFT).is_none());
        assert!(matches!(
            routed(Named::ArrowDown, Modifiers::SHIFT),
            Some(Routed::Close(Some(Binding::Select(Motion::Down))))
        ));
    }

    #[test]
    fn a_key_the_host_takes_is_left_to_it_after_an_opening_in_the_batch() {
        let policy: KeyPolicy = Arc::new(|key: &Key, _, modifiers: Modifiers| {
            if *key == Key::Named(Named::Enter) && modifiers.is_empty() {
                KeyRoute::Host
            } else {
                KeyRoute::Pass
            }
        });
        let context = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        let enter = |opening| {
            route(
                KeyPress {
                    modified_key: Key::Named(Named::Enter),
                    key: Key::Named(Named::Enter),
                    physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                    modifiers: Modifiers::empty(),
                    text: None,
                    status: text_editor::Status::Focused { is_hovered: false },
                },
                context,
                Some(OpenMenu {
                    selected: Some(0),
                    items: 1,
                    opening,
                }),
            )
        };
        // Uncaptured, so the host still runs its command if nothing opened.
        assert!(matches!(enter(true), Some(Routed::Close(None))));
        // A menu open when the batch starts takes Enter, as before.
        assert_eq!(menu_event(enter(false)), Some(SpellEvent::AcceptSelected));
    }

    #[test]
    fn host_editor_commands_wait_for_a_composition_to_end() {
        let policy = host_policy();
        let composing = Context {
            policy: Some(&policy),
            ..COMPOSING
        };
        let command = Modifiers::COMMAND;
        assert!(press(Key::Named(Named::Enter), command, None, composing).is_none());
        for key in [Named::Tab, Named::Backspace, Named::Delete] {
            assert!(
                press(Key::Named(key), command, None, composing).is_none(),
                "{key:?}"
            );
        }
        assert!(press(Key::Character("b".into()), command, None, composing).is_none());
        // Host shortcuts and the editor's own keys are unchanged.
        assert!(press(Key::Character("f".into()), command, None, composing).is_none());
        assert!(matches!(
            press(
                Key::Character("a".into()),
                Modifiers::empty(),
                Some("a"),
                composing
            ),
            Some(Binding::Insert('a'))
        ));
    }

    #[test]
    fn macos_ctrl_letters_move_and_others_do_not() {
        let ctrl = |letter: &str, m| ctrl_motion(&Key::Character(letter.into()), m);
        let moved = |letter| match ctrl(letter, Modifiers::CTRL) {
            Some(Binding::Move(motion)) => Some(motion),
            _ => None,
        };
        assert_eq!(moved("a"), Some(Motion::Home));
        assert_eq!(moved("e"), Some(Motion::End));
        assert_eq!(moved("b"), Some(Motion::Left));
        assert_eq!(moved("f"), Some(Motion::Right));
        assert!(ctrl("k", Modifiers::CTRL).is_none());
        assert!(ctrl("a", Modifiers::CTRL | Modifiers::SHIFT).is_none());
        assert!(ctrl("a", Modifiers::LOGO).is_none());
        assert!(matches!(
            editing(Binding::Move(Motion::Home), Modifiers::CTRL, true),
            Binding::Custom(Message::Intent(Intent::Home { extend: false }))
        ));
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

    // ── Popovers ────────────────────────────────────────────────────────────

    /// The status menu's keys: Space, `/`, `x`, and Shift+H.
    const ACCELERATORS: [Option<Accelerator>; 5] = [
        Some(Accelerator::plain(' ')),
        Some(Accelerator::plain('/')),
        Some(Accelerator::plain('x')),
        None,
        Some(Accelerator::shifted('h')),
    ];

    /// The status menu, a menu of the host's.
    const MENU: Target<'static> = Target::Menu(&ACCELERATORS);

    const EVERY_TARGET: [Target<'static>; 5] = [
        Target::SpellMenu,
        Target::SpellOpening,
        MENU,
        Target::Suggest,
        Target::Keyboard,
    ];

    fn focused(key: Key, modifiers: Modifiers, text: Option<&str>) -> KeyPress {
        KeyPress {
            modified_key: key.clone(),
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            key,
            modifiers,
            text: text.map(Into::into),
            status: text_editor::Status::Focused { is_hovered: false },
        }
    }

    /// How a focused press routes to `target`.
    fn to_popover(
        target: Target<'_>,
        key: Key,
        modifiers: Modifiers,
        text: Option<&str>,
        context: Context,
    ) -> Option<Routed> {
        route(focused(key, modifiers, text), context, Some(target))
    }

    /// Whether a press is published as the popover's Escape and left
    /// uncaptured.
    fn is_ordered_escape(routed: Option<Routed>) -> bool {
        matches!(
            routed,
            Some(Routed::Pass(Message::Spell(SpellEvent::Key(
                MenuKey::Escape
            ))))
        )
    }

    #[test]
    fn a_menu_of_the_hosts_takes_its_keys_and_its_accelerators() {
        let menu = |key: Key, modifiers, text: Option<&str>| {
            to_popover(MENU, key, modifiers, text, ASSIST)
        };
        let accepted =
            |key: Key, modifiers, text: Option<&str>| menu_event(menu(key, modifiers, text));
        let none = Modifiers::empty();
        assert_eq!(
            accepted(Key::Named(Named::Space), none, Some(" ")),
            Some(SpellEvent::Accept(0))
        );
        assert_eq!(
            accepted(Key::Character("/".into()), none, Some("/")),
            Some(SpellEvent::Accept(1))
        );
        assert_eq!(
            accepted(Key::Character("x".into()), none, Some("x")),
            Some(SpellEvent::Accept(2))
        );
        assert_eq!(
            accepted(Key::Character("h".into()), Modifiers::SHIFT, Some("H")),
            Some(SpellEvent::Accept(4))
        );
        assert_eq!(
            accepted(Key::Named(Named::ArrowUp), none, None),
            Some(SpellEvent::Move(-1))
        );
        assert_eq!(
            accepted(Key::Named(Named::ArrowDown), none, None),
            Some(SpellEvent::Move(1))
        );
        assert_eq!(
            accepted(Key::Named(Named::Enter), none, Some("\r")),
            Some(SpellEvent::AcceptSelected)
        );
        // A key that is no row's closes the menu and types, as does an
        // accelerator's letter without its Shift or with one it lacks.
        for (key, modifiers, text, typed) in [
            ("h", none, "h", 'h'),
            ("y", none, "y", 'y'),
            ("x", Modifiers::SHIFT, "X", 'X'),
        ] {
            assert!(
                matches!(
                    menu(Key::Character(key.into()), modifiers, Some(text)),
                    Some(Routed::Close(Some(Binding::Insert(c)))) if c == typed
                ),
                "{key} {modifiers:?}"
            );
        }
        // Tab and a modified arrow are other keys too, and a shortcut is
        // the shortcut.
        assert!(matches!(
            menu(Key::Named(Named::Tab), none, Some("\t")),
            Some(Routed::Close(Some(Binding::Custom(Message::Intent(
                Intent::Tab
            )))))
        ));
        assert!(matches!(
            menu(Key::Named(Named::ArrowDown), Modifiers::SHIFT, None),
            Some(Routed::Close(Some(Binding::Select(Motion::Down))))
        ));
        assert!(matches!(
            menu(Key::Character("x".into()), Modifiers::COMMAND, None),
            Some(Routed::Close(Some(Binding::Cut)))
        ));
        // A host shortcut closes the menu and still reaches the host.
        let policy = host_policy();
        let hosted = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        assert!(matches!(
            to_popover(
                MENU,
                Key::Character("f".into()),
                Modifiers::COMMAND,
                None,
                hosted
            ),
            Some(Routed::Close(None))
        ));
        // A lone modifier leaves it alone.
        assert!(menu(Key::Named(Named::Shift), Modifiers::SHIFT, None).is_none());
    }

    /// A host that binds Cmd/Ctrl+Enter to a command of its own numbered
    /// 7, and Alt+P to one numbered 8.
    fn command_policy() -> KeyPolicy {
        Arc::new(|key: &Key, _, modifiers: Modifiers| match key.as_ref() {
            Key::Named(Named::Enter) if modifiers == Modifiers::COMMAND => KeyRoute::Command(7),
            Key::Character("p") if modifiers == Modifiers::ALT => KeyRoute::Command(8),
            _ => KeyRoute::Pass,
        })
    }

    /// A host command's key is captured and published as the command, with
    /// or without Markdown assist, never under a preedit, and closes an
    /// open menu first, as any other key no row of it takes.
    #[test]
    fn a_host_commands_key_carries_its_number_to_the_host() {
        let policy = command_policy();
        for context in [ASSIST, PLAIN] {
            let context = Context {
                policy: Some(&policy),
                ..context
            };
            assert!(matches!(
                press(Key::Named(Named::Enter), Modifiers::COMMAND, None, context),
                Some(Binding::Custom(Message::Command(7)))
            ));
            assert!(matches!(
                press(Key::Character("p".into()), Modifiers::ALT, None, context),
                Some(Binding::Custom(Message::Command(8)))
            ));
            // Without the modifier the keys are the text's.
            assert!(matches!(
                press(
                    Key::Character("p".into()),
                    Modifiers::empty(),
                    Some("p"),
                    context
                ),
                Some(Binding::Insert('p'))
            ));
        }
        let composing = Context {
            policy: Some(&policy),
            ..COMPOSING
        };
        assert!(press(Key::Character("p".into()), Modifiers::ALT, None, composing).is_none());
        let hosted = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        assert!(matches!(
            to_popover(
                MENU,
                Key::Character("p".into()),
                Modifiers::ALT,
                None,
                hosted
            ),
            Some(Routed::Close(Some(Binding::Custom(Message::Command(8)))))
        ));
        assert!(matches!(
            to_popover(
                Target::SpellMenu,
                Key::Character("p".into()),
                Modifiers::ALT,
                None,
                hosted
            ),
            Some(Routed::Close(Some(Binding::Custom(Message::Command(8)))))
        ));
        // Suggestions leave it to the text's routing, which publishes it.
        assert!(matches!(
            to_popover(
                Target::Suggest,
                Key::Character("p".into()),
                Modifiers::ALT,
                None,
                hosted
            ),
            Some(Routed::Binding(Binding::Custom(Message::Command(8))))
        ));
    }

    #[test]
    fn escape_on_a_menu_of_the_hosts_or_on_suggestions_is_sent_on_in_its_order() {
        for target in [MENU, Target::Suggest] {
            // With no Ctrl, Alt or logo key it is the popover's: a message,
            // so the host dismisses it before what is typed next;
            // uncaptured and with no binding, so the editor keeps the
            // keyboard and the host's own key handling hears of it.
            for modifiers in [Modifiers::empty(), Modifiers::SHIFT] {
                assert!(
                    is_ordered_escape(to_popover(
                        target,
                        Key::Named(Named::Escape),
                        modifiers,
                        None,
                        ASSIST
                    )),
                    "{target:?} {modifiers:?}"
                );
            }
        }
        // With one of them it is a shortcut, the host's by another road,
        // and any other key to the popover: a menu closes first.
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            assert!(!is_plain_escape(modifiers));
            assert!(
                matches!(
                    to_popover(MENU, Key::Named(Named::Escape), modifiers, None, ASSIST),
                    Some(Routed::Close(Some(Binding::Unfocus)))
                ),
                "{modifiers:?}"
            );
            assert!(
                matches!(
                    to_popover(
                        Target::Suggest,
                        Key::Named(Named::Escape),
                        modifiers,
                        None,
                        ASSIST
                    ),
                    Some(Routed::Binding(Binding::Unfocus))
                ),
                "{modifiers:?}"
            );
        }
        assert!(is_plain_escape(Modifiers::empty()) && is_plain_escape(Modifiers::SHIFT));
        // The suggestions menu leaves every Escape to the host's key
        // handling, with no word.
        for target in [Target::SpellMenu, Target::SpellOpening] {
            for modifiers in [Modifiers::empty(), Modifiers::CTRL] {
                assert!(
                    to_popover(target, Key::Named(Named::Escape), modifiers, None, ASSIST)
                        .is_none(),
                    "{target:?} {modifiers:?}"
                );
            }
        }
    }

    #[test]
    fn a_letter_closes_the_suggestions_menu_open_or_asked_for() {
        // It has no accelerators, whether the host has opened it yet or
        // not: the letter closes it and types.
        for target in [Target::SpellMenu, Target::SpellOpening] {
            assert!(
                matches!(
                    to_popover(
                        target,
                        Key::Character("x".into()),
                        Modifiers::empty(),
                        Some("x"),
                        ASSIST
                    ),
                    Some(Routed::Close(Some(Binding::Insert('x'))))
                ),
                "{target:?}"
            );
        }
    }

    #[test]
    fn suggestions_take_plain_up_down_enter_and_tab_whatever_the_host_has_bound() {
        let none = Modifiers::empty();
        let policy: KeyPolicy = Arc::new(|key: &Key, _, modifiers: Modifiers| {
            if *key == Key::Named(Named::Tab) && modifiers.is_empty() {
                KeyRoute::Host
            } else {
                KeyRoute::Pass
            }
        });
        let hosted = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        for context in [ASSIST, PLAIN, hosted] {
            for (named, text, key) in [
                (Named::ArrowUp, None, MenuKey::Up),
                (Named::ArrowDown, None, MenuKey::Down),
                (Named::Enter, Some("\r"), MenuKey::Enter),
                (Named::Tab, Some("\t"), MenuKey::Tab),
            ] {
                // Captured, and resolved by the host against the rows it
                // has.
                assert_eq!(
                    menu_event(to_popover(
                        Target::Suggest,
                        Key::Named(named),
                        none,
                        text,
                        context
                    )),
                    Some(SpellEvent::Key(key)),
                    "{named:?}"
                );
            }
        }
        // With nothing open the host's Tab is the host's.
        assert!(press(Key::Named(Named::Tab), none, Some("\t"), hosted).is_none());
    }

    #[test]
    fn every_other_key_passes_suggestions_as_if_nothing_were_open() {
        let suggest = |key: Key, modifiers, text: Option<&str>| match to_popover(
            Target::Suggest,
            key,
            modifiers,
            text,
            ASSIST,
        ) {
            Some(Routed::Binding(binding)) => Some(binding),
            Some(Routed::Close(_) | Routed::Pass(_)) => panic!("not the text's"),
            None => None,
        };
        let none = Modifiers::empty();
        assert!(matches!(
            suggest(Key::Character("e".into()), none, Some("e")),
            Some(Binding::Insert('e'))
        ));
        // Even a letter a menu would take as an accelerator.
        assert!(matches!(
            suggest(Key::Character("x".into()), none, Some("x")),
            Some(Binding::Insert('x'))
        ));
        assert!(matches!(
            suggest(Key::Named(Named::Space), none, Some(" ")),
            Some(Binding::Insert(' '))
        ));
        assert!(matches!(
            suggest(Key::Named(Named::Backspace), none, None),
            Some(Binding::Custom(Message::Intent(Intent::Backspace)))
        ));
        assert!(matches!(
            suggest(Key::Named(Named::ArrowLeft), none, None),
            Some(Binding::Move(Motion::Left))
        ));
        // The popover's keys with a modifier are the text's.
        assert!(matches!(
            suggest(Key::Named(Named::ArrowDown), Modifiers::SHIFT, None),
            Some(Binding::Select(Motion::Down))
        ));
        assert!(matches!(
            suggest(Key::Named(Named::Enter), Modifiers::SHIFT, Some("\r")),
            Some(Binding::Custom(Message::Intent(Intent::ShiftEnter)))
        ));
        assert!(matches!(
            suggest(Key::Named(Named::Tab), Modifiers::SHIFT, Some("\t")),
            Some(Binding::Custom(Message::Intent(Intent::ShiftTab)))
        ));
        assert!(matches!(
            suggest(Key::Character("z".into()), Modifiers::COMMAND, None),
            Some(Binding::Custom(Message::Undo))
        ));
        // A lone modifier and a key the editor ignores do nothing.
        assert!(suggest(Key::Named(Named::Shift), Modifiers::SHIFT, None).is_none());
        assert!(suggest(Key::Named(Named::F5), none, None).is_none());
    }

    #[test]
    fn an_input_method_keeps_every_key_from_a_popover_while_it_composes() {
        let none = Modifiers::empty();
        for target in EVERY_TARGET {
            let composing =
                |key: Key, text: Option<&str>| to_popover(target, key, none, text, COMPOSING);
            // Enter and Tab are the input method's own, as with nothing
            // open: a menu opened while it composes accepts no row for
            // them, and suggestions choose none.
            assert!(
                composing(Key::Named(Named::Enter), Some("\r")).is_none(),
                "{target:?}"
            );
            assert!(
                composing(Key::Named(Named::Tab), Some("\t")).is_none(),
                "{target:?}"
            );
            // The arrows move, a letter types, and Escape drops focus, as
            // they do then: no key is the popover's, an accelerator
            // included.
            assert!(
                matches!(
                    composing(Key::Named(Named::ArrowDown), None),
                    Some(Routed::Binding(Binding::Move(Motion::Down)))
                ),
                "{target:?}"
            );
            assert!(
                matches!(
                    composing(Key::Character("x".into()), Some("x")),
                    Some(Routed::Binding(Binding::Insert('x')))
                ),
                "{target:?}"
            );
            assert!(
                matches!(
                    composing(Key::Named(Named::Escape), None),
                    Some(Routed::Binding(Binding::Unfocus))
                ),
                "{target:?}"
            );
        }
    }

    #[test]
    fn a_host_that_took_the_keyboard_is_handed_each_key_as_a_stroke() {
        let none = Modifiers::empty();
        let keyboard = |key: Key, modifiers, text: Option<&str>, context| {
            to_popover(Target::Keyboard, key, modifiers, text, context)
        };
        let stroke = |key: Key, modifiers, text: Option<&str>| match menu_event(keyboard(
            key, modifiers, text, ASSIST,
        )) {
            Some(SpellEvent::Stroke(stroke)) => Some(stroke),
            _ => None,
        };
        // The key, its modifiers and what it types, for the host's field:
        // captured, and the editor does nothing with it.
        let f = stroke(Key::Character("f".into()), none, Some("f")).unwrap();
        assert_eq!(
            (f.character(), f.named(), f.text(), f.modifiers()),
            (Some('f'), None, Some('f'), none)
        );
        let shifted = stroke(Key::Character("h".into()), Modifiers::SHIFT, Some("H")).unwrap();
        assert_eq!(
            (shifted.character(), shifted.text()),
            (Some('h'), Some('H'))
        );
        for (named, text) in [
            (Named::Enter, Some("\r")),
            (Named::Tab, Some("\t")),
            (Named::Backspace, None),
            (Named::ArrowDown, None),
            (Named::Space, Some(" ")),
        ] {
            let stroke = stroke(Key::Named(named), none, text).unwrap();
            assert_eq!(stroke.named(), Some(named));
            // Control characters are not what a key types.
            assert_eq!(stroke.text(), (named == Named::Space).then_some(' '));
        }
        let paste = stroke(Key::Character("v".into()), Modifiers::COMMAND, None).unwrap();
        assert_eq!(
            (paste.character(), paste.text(), paste.modifiers()),
            (Some('v'), None, Modifiers::COMMAND)
        );
        // Nothing of the editor's is open, so Escape is the host's as it
        // is with nothing open, and so is a key the host has bound.
        assert!(matches!(
            keyboard(Key::Named(Named::Escape), none, None, ASSIST),
            Some(Routed::Binding(Binding::Unfocus))
        ));
        let policy = host_policy();
        let hosted = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        assert!(keyboard(Key::Character("f".into()), Modifiers::COMMAND, None, hosted).is_none());
        // A lone modifier does nothing, and a key of more than one
        // character cannot be carried.
        assert!(keyboard(Key::Named(Named::Shift), Modifiers::SHIFT, None, ASSIST).is_none());
        assert!(keyboard(Key::Character("ab".into()), none, Some("ab"), ASSIST).is_none());
    }

    #[test]
    fn a_host_key_asks_for_a_popover_of_the_hosts() {
        let policy: KeyPolicy = Arc::new(|key: &Key, _, modifiers: Modifiers| {
            if modifiers == Modifiers::COMMAND && *key == Key::Character("d".into()) {
                KeyRoute::Popover(7)
            } else {
                KeyRoute::Pass
            }
        });
        let context = Context {
            policy: Some(&policy),
            ..ASSIST
        };
        let command_d = |context| {
            press(
                Key::Character("d".into()),
                Modifiers::COMMAND,
                None,
                context,
            )
        };
        assert!(matches!(
            command_d(context),
            Some(Binding::Custom(Message::Spell(SpellEvent::Request(7))))
        ));
        // Not while an input method composes.
        assert!(command_d(Context {
            policy: Some(&policy),
            ..COMPOSING
        })
        .is_none());
        // On an open menu it closes the menu and asks, as the suggest key
        // does; suggestions pass it on.
        for target in [MENU, Target::SpellMenu] {
            assert!(
                matches!(
                    to_popover(
                        target,
                        Key::Character("d".into()),
                        Modifiers::COMMAND,
                        None,
                        context
                    ),
                    Some(Routed::Close(Some(Binding::Custom(Message::Spell(
                        SpellEvent::Request(7)
                    )))))
                ),
                "{target:?}"
            );
        }
        assert!(matches!(
            to_popover(
                Target::Suggest,
                Key::Character("d".into()),
                Modifiers::COMMAND,
                None,
                context
            ),
            Some(Routed::Binding(Binding::Custom(Message::Spell(
                SpellEvent::Request(7)
            ))))
        ));
    }

    #[test]
    fn an_unfocused_editor_routes_nothing_to_a_popover() {
        for target in EVERY_TARGET {
            let routed = route(
                KeyPress {
                    status: text_editor::Status::Active,
                    ..focused(Key::Named(Named::Enter), Modifiers::empty(), None)
                },
                ASSIST,
                Some(target),
            );
            assert!(routed.is_none(), "{target:?}");
        }
    }

    #[test]
    fn only_a_menu_takes_the_editors_presses() {
        assert_eq!(MENU.modal(), Some(false));
        assert_eq!(Target::SpellMenu.modal(), Some(false));
        // The host has yet to say whether the suggestions menu opened.
        assert_eq!(Target::SpellOpening.modal(), Some(true));
        assert_eq!(Target::Suggest.modal(), None);
        assert_eq!(Target::Keyboard.modal(), None);
        for target in EVERY_TARGET {
            assert_eq!(
                target.leaves_escape(),
                matches!(target, Target::SpellMenu | Target::SpellOpening),
                "{target:?}"
            );
        }
    }
}
