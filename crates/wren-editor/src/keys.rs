//! The editor's own shortcuts, checked before iced's stock key bindings.

use std::sync::Arc;

use iced::keyboard::key::{self, Named};
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{self, Binding, KeyPress, Motion};

use crate::proof::OpenMenu;
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
}

/// Route a key press, with `menu` open (rule 4.4.4). Up and Down with no
/// modifier move its selection and Enter with no modifier accepts the row
/// the host's menu has selected, or closes a menu with nothing to offer.
/// Escape is the host's: the editor gives it no binding, so it neither drops
/// focus nor captures the press, and the host closes the menu (rule 4.4.8).
/// A modifier key alone does nothing. Any other press closes the menu and is
/// then routed by [`key_binding`]. With no menu, routing is
/// [`key_binding`]'s alone.
pub(crate) fn route(
    key_press: KeyPress,
    context: Context,
    menu: Option<OpenMenu>,
) -> Option<Routed> {
    if !matches!(key_press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    if menu.is_none() {
        return key_binding(key_press, context).map(Routed::Binding);
    }
    let plain = key_press.modifiers.is_empty();
    let event = match key_press.key.as_ref() {
        Key::Named(Named::Escape) => return None,
        Key::Named(named) if is_modifier(named) => return None,
        Key::Named(Named::ArrowUp) if plain => SpellEvent::Move(-1),
        Key::Named(Named::ArrowDown) if plain => SpellEvent::Move(1),
        Key::Named(Named::Enter) if plain => SpellEvent::AcceptSelected,
        _ => return Some(Routed::Close(key_binding(key_press, context))),
    };
    Some(Routed::Binding(Binding::Custom(Message::Spell(event))))
}

/// Whether `named` is a modifier key, which pressed alone leaves an open
/// menu alone.
fn is_modifier(named: Named) -> bool {
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

/// The messages a binding publishes, for a host that runs key routing
/// without a window: what [`crate::widget`] publishes for it, except that
/// the clipboard is not there to copy to or paste from, and dropping focus
/// is no message. `selection` is whether text is selected, which Cut needs.
#[cfg(feature = "test-support")]
pub(crate) fn messages(binding: Binding<Message>, selection: bool, out: &mut Vec<Message>) {
    use iced::widget::text_editor::{Action, Edit};

    let action = match binding {
        Binding::Custom(message) => return out.push(message),
        Binding::Sequence(sequence) => {
            for binding in sequence {
                messages(binding, selection, out);
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
        Binding::Cut if selection => Action::Edit(Edit::Delete),
        Binding::Cut | Binding::Copy | Binding::Paste | Binding::Unfocus => return,
    };
    out.push(Message::Action(action));
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
            Some(OpenMenu { selected, items: 3 }),
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
            }),
        );
        assert!(routed.is_none());
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
}
