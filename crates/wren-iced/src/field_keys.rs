//! The text editing keys of the app's text fields on Linux
//! (discovery/keybindings.md §4). iced's text input takes Ctrl for copy,
//! cut, paste and select all, and the keymap may put those keys elsewhere.
//! [`field_keys`] wraps a text input:
//!
//! - A press the keymap gives an `edit.*` action, other than the input's
//!   own press for it, reaches the input as that Ctrl press, with the
//!   modifiers it reads (the ones it last heard change) set for that press
//!   alone: Alt mode's Alt keys, or a user's own. The input has no history,
//!   so undo and redo reach it as Ctrl+Z and Ctrl+Shift+Z, which it
//!   ignores: they never type their letter.
//! - Otherwise, one of the input's own clipboard chords that the keymap
//!   gives another command, or that a `keybind` line unbinds, is taken from
//!   the focused input, so one press does one thing.
//! - Before any other press the input hears the press's own modifiers: an
//!   input mounted while a modifier was held never heard it go down.
//!
//! With the default Ctrl-mode keymap only that last holds, which changes
//! nothing for an input that saw the modifiers change. On macOS the keys
//! are not wrapped at all; [`field_keys_owning`] still gives a key to its
//! owner there.
//!
//! [`field_keys_owning`] also takes the presses of one key from the
//! field before anything else, for a field whose host closes on that key
//! (the board's keys sheet on its own toggle).

use std::sync::Arc;

use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{tree, Id, Operation, Tree};
use iced::advanced::{layout, mouse, overlay, renderer, Clipboard, Layout, Shell, Widget};
use iced::keyboard::{self, Key, Modifiers};
use iced::{Element, Event, Length, Rectangle, Size, Vector};
use wren_core::keybind::{Accel, Mods, Platform, Scheme};
use wren_editor::EditKey;

/// What the wrapper does with a Linux keymap's keys; with none (macOS),
/// nothing at all.
#[derive(Debug, Clone, Default)]
pub struct FieldKeys(Option<Arc<Table>>);

#[derive(Debug, Default)]
struct Table {
    /// Alt mode's editing keys that are not the input's own presses.
    rewrites: Vec<(Accel, EditKey)>,
    /// The input's own clipboard chords that the keymap gives elsewhere.
    taken: Vec<Accel>,
}

/// The letter of the press a text input knows for `edit`, and whether
/// Shift is held for it.
fn native(edit: EditKey) -> (&'static str, bool) {
    match edit {
        EditKey::Undo => ("z", false),
        EditKey::Redo => ("z", true),
        EditKey::Copy => ("c", false),
        EditKey::Cut => ("x", false),
        EditKey::Paste => ("v", false),
        EditKey::SelectAll => ("a", false),
    }
}

/// The clipboard or select-all key that iced's text widgets take `accel`
/// for on their own on `platform`, whatever the keymap says: the command
/// key (Ctrl, or ⌘ on macOS) with C, X, V or A, other modifiers or none,
/// but Paste never with Alt.
pub fn native_edit(platform: Platform, accel: &Accel) -> Option<EditKey> {
    let command = match platform {
        Platform::Mac => accel.mods.logo,
        Platform::Linux => accel.mods.ctrl,
    };
    if !command {
        return None;
    }
    Some(match accel.key.as_str() {
        "c" => EditKey::Copy,
        "x" => EditKey::Cut,
        "v" if !accel.mods.alt => EditKey::Paste,
        "a" => EditKey::SelectAll,
        _ => return None,
    })
}

/// Every trigger that [`native_edit`] takes on Linux.
pub fn native_chords() -> impl Iterator<Item = Accel> {
    ["c", "x", "v", "a"].into_iter().flat_map(|key| {
        (0..8u8).filter_map(move |held| {
            let accel = Accel {
                mods: Mods {
                    ctrl: true,
                    shift: held & 1 != 0,
                    alt: held & 2 != 0,
                    logo: held & 4 != 0,
                },
                key: key.to_owned(),
            };
            native_edit(Platform::Linux, &accel).map(|_| accel)
        })
    })
}

impl FieldKeys {
    /// The wrapper for a keymap of `scheme`: its `edits` (each trigger the
    /// keymap gives an `edit.*` action, and that action's key), rewritten
    /// but for those on the input's own press, and `taken`, the input's own
    /// chords the keymap gives elsewhere, but for those it rewrites.
    pub fn new(
        scheme: Scheme,
        edits: impl IntoIterator<Item = (Accel, EditKey)>,
        taken: impl IntoIterator<Item = Accel>,
    ) -> Self {
        if scheme.platform == Platform::Mac {
            return Self(None);
        }
        let own = |accel: &Accel, edit: EditKey| {
            let (letter, shift) = native(edit);
            *accel
                == Accel {
                    mods: Mods {
                        ctrl: true,
                        shift,
                        ..Mods::default()
                    },
                    key: letter.to_owned(),
                }
        };
        let rewrites: Vec<(Accel, EditKey)> = edits
            .into_iter()
            .filter(|(accel, edit)| !own(accel, *edit))
            .collect();
        // A chord the keymap gives another edit is that edit, not nothing.
        let taken = taken
            .into_iter()
            .filter(|accel| rewrites.iter().all(|(bound, _)| bound != accel))
            .collect();
        Self(Some(Arc::new(Table { rewrites, taken })))
    }

    fn press(event: &Event) -> Option<(Accel, Modifiers)> {
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) = event
        else {
            return None;
        };
        Some((
            crate::keys::shortcut(key, *physical_key, *modifiers)?,
            *modifiers,
        ))
    }

    /// `event` as the text input knows it, when it is a press of one of
    /// the rewritten keys: the press, and the modifiers the input must read
    /// for it.
    pub fn rewrite(&self, event: &Event) -> Option<(Modifiers, Event)> {
        let table = self.0.as_deref()?;
        let Event::Keyboard(keyboard::Event::KeyPressed {
            physical_key,
            location,
            repeat,
            ..
        }) = event
        else {
            return None;
        };
        let (accel, _) = Self::press(event)?;
        let edit = table
            .rewrites
            .iter()
            .find(|(bound, _)| *bound == accel)
            .map(|(_, edit)| *edit)?;
        let (letter, shift) = native(edit);
        let held = if shift {
            Modifiers::CTRL | Modifiers::SHIFT
        } else {
            Modifiers::CTRL
        };
        let press = Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character(letter.into()),
            modified_key: Key::Character(letter.into()),
            physical_key: *physical_key,
            location: *location,
            modifiers: held,
            text: None,
            repeat: *repeat,
        });
        Some((held, press))
    }

    /// Whether `event` presses one of the input's own chords that the
    /// keymap gives elsewhere.
    pub fn takes(&self, event: &Event) -> bool {
        self.0.as_deref().is_some_and(|table| {
            Self::press(event).is_some_and(|(accel, _)| table.taken.contains(&accel))
        })
    }

    /// Hand `event` on through `update` as the text input knows it: a
    /// press with its own modifiers set first, and a rewritten key as its
    /// Ctrl press, the modifiers put back after it. Unwrapped keys (macOS)
    /// hand it on as it is.
    fn deliver(&self, event: &Event, mut update: impl FnMut(&Event)) {
        let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = event else {
            return update(event);
        };
        if self.0.is_none() {
            return update(event);
        }
        let modifiers = *modifiers;
        match self.rewrite(event) {
            Some((held, press)) => {
                update(&Event::Keyboard(keyboard::Event::ModifiersChanged(held)));
                update(&press);
                update(&Event::Keyboard(keyboard::Event::ModifiersChanged(
                    modifiers,
                )));
            }
            None => {
                update(&Event::Keyboard(keyboard::Event::ModifiersChanged(
                    modifiers,
                )));
                update(event);
            }
        }
    }
}

/// `content`, a text input, with `keys` handed to it as its own. The
/// wrapper has no state of its own, so the input's tree is the same with
/// it or without it.
pub fn field_keys<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    keys: &FieldKeys,
) -> Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    let content = content.into();
    if keys.0.is_none() {
        return content;
    }
    Element::new(KeyedField {
        content,
        keys: keys.clone(),
        owner: None,
    })
}

/// [`field_keys`], and the presses `owns` names taken from the focused
/// field before anything else and published as `on_owned`, so the field
/// never types or edits for them.
pub fn field_keys_owning<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    keys: &FieldKeys,
    owns: impl Fn(&Key, keyboard::key::Physical, Modifiers) -> bool + 'a,
    on_owned: impl Fn() -> Message + 'a,
) -> Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    Element::new(KeyedField {
        content: content.into(),
        keys: keys.clone(),
        owner: Some(Owner {
            owns: Box::new(owns),
            on_owned: Box::new(on_owned),
        }),
    })
}

/// Whether a press, by its key, physical key and modifiers, is the owner's.
type Owns<'a> = Box<dyn Fn(&Key, keyboard::key::Physical, Modifiers) -> bool + 'a>;

/// The key a field's host takes from it ([`field_keys_owning`]).
struct Owner<'a, Message> {
    owns: Owns<'a>,
    on_owned: Box<dyn Fn() -> Message + 'a>,
}

/// Whether a focusable widget in a tree has the keyboard.
struct HasFocus(bool);

impl Operation for HasFocus {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        self.0 |= state.is_focused();
    }
}

struct KeyedField<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    keys: FieldKeys,
    owner: Option<Owner<'a, Message>>,
}

impl<Message, Theme, Renderer> KeyedField<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn focused(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer) -> bool {
        let mut focus = HasFocus(false);
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, &mut focus);
        focus.0
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for KeyedField<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let (
            Some(owner),
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }),
        ) = (&self.owner, event)
        {
            if (owner.owns)(key, *physical_key, *modifiers) {
                let message = (owner.on_owned)();
                if self.focused(tree, layout, renderer) {
                    shell.publish(message);
                    shell.capture_event();
                    return;
                }
            }
        }
        // The input does nothing with it: the app runs the keymap's
        // command for it, if it has one.
        if self.keys.takes(event) && self.focused(tree, layout, renderer) {
            shell.capture_event();
            return;
        }
        let content = self.content.as_widget_mut();
        self.keys.deliver(event, |event| {
            content.update(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
        });
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #[cfg(not(target_os = "macos"))]
    use iced::advanced::clipboard;
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::Location;
    use wren_core::keybind::LinuxModifier;

    use super::*;

    /// A clipboard that keeps what is written to it.
    #[cfg(not(target_os = "macos"))]
    #[derive(Default)]
    pub(crate) struct Kept(pub(crate) Option<String>);

    #[cfg(not(target_os = "macos"))]
    impl Clipboard for Kept {
        fn read(&self, kind: clipboard::Kind) -> Option<String> {
            self.0.clone().filter(|_| kind == clipboard::Kind::Standard)
        }

        fn write(&mut self, kind: clipboard::Kind, contents: String) {
            if kind == clipboard::Kind::Standard {
                self.0 = Some(contents);
            }
        }
    }

    /// The `edit.*` defaults of a Linux keymap in `mode`: Mod+Z,
    /// Mod+Shift+Z, Mod+C, Mod+X, Mod+V and Mod+A, and in Alt mode their
    /// Ctrl versions too.
    fn linux_edits(mode: LinuxModifier) -> Vec<(Accel, EditKey)> {
        let scheme = Scheme::new(Platform::Linux, mode);
        let edits = [
            ("z", EditKey::Undo),
            ("shift+z", EditKey::Redo),
            ("c", EditKey::Copy),
            ("x", EditKey::Cut),
            ("v", EditKey::Paste),
            ("a", EditKey::SelectAll),
        ];
        let mut bindings = Vec::new();
        for (key, edit) in edits {
            bindings.push((scheme.parse(&format!("mod+{key}")).unwrap(), edit));
            if mode == LinuxModifier::Alt {
                bindings.push((scheme.parse(&format!("ctrl+{key}")).unwrap(), edit));
            }
        }
        bindings
    }

    /// The text fields' keys of a default Linux keymap in `mode`.
    pub(crate) fn linux(mode: LinuxModifier) -> FieldKeys {
        FieldKeys::new(Scheme::new(Platform::Linux, mode), linux_edits(mode), [])
    }

    /// A press of `letter` with `modifiers`, typing it as Linux reports
    /// Alt+letter, so that only the root's filter keeps it from typing.
    pub(crate) fn chord(letter: &str, modifiers: Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character(letter.into()),
            modified_key: Key::Character(letter.into()),
            physical_key: Physical::Code(Code::KeyA),
            location: Location::Standard,
            modifiers,
            text: (!modifiers.control()).then(|| letter.into()),
            repeat: false,
        })
    }

    fn accel(trigger: &str) -> Accel {
        wren_core::keybind::parse_trigger(trigger).unwrap()
    }

    #[test]
    fn alt_mode_rewrites_its_keys_to_the_input_s_ctrl_presses() {
        let alt = linux(LinuxModifier::Alt);
        let rewrites = &alt.0.as_deref().unwrap().rewrites;
        // Alt mode keeps the Alt keys only; Ctrl is the input's own.
        assert_eq!(rewrites.len(), 6);
        assert!(rewrites
            .iter()
            .all(|(accel, _)| accel.mods.alt && !accel.mods.ctrl));
        let Some((
            held,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                text,
                modifiers,
                ..
            }),
        )) = alt.rewrite(&chord("c", Modifiers::ALT))
        else {
            panic!("Alt+C is a rewrite");
        };
        assert_eq!((held, modifiers), (Modifiers::CTRL, Modifiers::CTRL));
        assert_eq!((key, text), (Key::Character("c".into()), None));
        let Some((held, _)) = alt.rewrite(&chord("z", Modifiers::ALT | Modifiers::SHIFT)) else {
            panic!("Alt+Shift+Z is a rewrite");
        };
        assert_eq!(held, Modifiers::CTRL | Modifiers::SHIFT);
        assert!(alt.rewrite(&chord("c", Modifiers::CTRL)).is_none());
        assert!(alt.rewrite(&chord("c", Modifiers::empty())).is_none());
        assert!(alt.rewrite(&chord("y", Modifiers::ALT)).is_none());
        assert!(!alt.takes(&chord("v", Modifiers::CTRL)));
    }

    #[test]
    fn ctrl_mode_s_defaults_rewrite_nothing_and_a_user_s_edit_key_rewrites() {
        let scheme = Scheme::new(Platform::Linux, LinuxModifier::Ctrl);
        let defaults = linux(LinuxModifier::Ctrl);
        for letter in ["b", "z", "c", "x", "v", "a"] {
            for modifiers in [Modifiers::ALT, Modifiers::CTRL] {
                let press = chord(letter, modifiers);
                assert!(defaults.rewrite(&press).is_none(), "{letter}");
                assert!(!defaults.takes(&press), "{letter}");
            }
        }
        // `keybind = alt+c = edit.copy` copies in a text field too.
        let mut edits = linux_edits(LinuxModifier::Ctrl);
        edits.push((accel("alt+c"), EditKey::Copy));
        let keys = FieldKeys::new(scheme, edits, []);
        let Some((held, _)) = keys.rewrite(&chord("c", Modifiers::ALT)) else {
            panic!("a user's Alt+C is a rewrite");
        };
        assert_eq!(held, Modifiers::CTRL);
        assert!(keys.rewrite(&chord("c", Modifiers::CTRL)).is_none());
    }

    #[test]
    fn a_chord_given_another_edit_is_that_edit_not_taken() {
        // Alt mode with `keybind = ctrl+v = edit.copy`: Ctrl+V is iced's
        // paste given elsewhere, and copies.
        let scheme = Scheme::new(Platform::Linux, LinuxModifier::Alt);
        let mut edits = linux_edits(LinuxModifier::Alt);
        edits.retain(|(bound, _)| *bound != accel("ctrl+v"));
        edits.push((accel("ctrl+v"), EditKey::Copy));
        let keys = FieldKeys::new(scheme, edits, [accel("ctrl+v")]);
        let ctrl_v = chord("v", Modifiers::CTRL);
        assert!(!keys.takes(&ctrl_v));
        let Some((_, Event::Keyboard(keyboard::Event::KeyPressed { key, .. }))) =
            keys.rewrite(&ctrl_v)
        else {
            panic!("Ctrl+V is Copy's");
        };
        assert_eq!(key, Key::Character("c".into()));
    }

    #[test]
    fn on_macos_there_is_no_wrapper() {
        for linux in [LinuxModifier::Ctrl, LinuxModifier::Alt] {
            let scheme = Scheme::new(Platform::Mac, linux);
            let keys = FieldKeys::new(scheme, linux_edits(linux), [accel("super+v")]);
            assert!(keys.0.is_none());
            assert!(keys.rewrite(&chord("c", Modifiers::ALT)).is_none());
            assert!(!keys.takes(&chord("v", Modifiers::LOGO)));
        }
        assert!(FieldKeys::default().0.is_none());
    }

    #[test]
    fn the_native_chords_are_iced_s_clipboard_keys() {
        let chords: Vec<Accel> = native_chords().collect();
        // Eight modifier sets for C, X and A; Paste never with Alt.
        assert_eq!(chords.len(), 8 * 3 + 4);
        assert!(chords.contains(&accel("ctrl+shift+v")));
        assert!(!chords.contains(&accel("ctrl+alt+v")));
        assert_eq!(
            native_edit(Platform::Linux, &accel("ctrl+a")),
            Some(EditKey::SelectAll)
        );
        assert_eq!(native_edit(Platform::Linux, &accel("alt+a")), None);
        assert_eq!(native_edit(Platform::Linux, &accel("super+a")), None);
        assert_eq!(
            native_edit(Platform::Mac, &accel("super+a")),
            Some(EditKey::SelectAll)
        );
        assert_eq!(native_edit(Platform::Mac, &accel("ctrl+a")), None);
    }

    /// Real iced input on Linux, where its clipboard keys are Ctrl's.
    #[cfg(not(target_os = "macos"))]
    mod input {
        use iced::advanced::widget::operation;
        use iced::keyboard::key::Named;
        use iced::widget::text_input;
        use iced::{Point, Theme};

        use super::*;

        const ID: &str = "field-keys-test";

        fn end() -> Event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::End),
                modified_key: Key::Named(Named::End),
                physical_key: Physical::Code(Code::End),
                location: Location::Standard,
                modifiers: Modifiers::empty(),
                text: None,
                repeat: false,
            })
        }

        /// What one run of a text input did: what it published, whether
        /// each event was captured, and what the clipboard holds.
        struct Run {
            published: Vec<String>,
            captured: Vec<bool>,
            kept: Option<String>,
        }

        /// A text input under the app's root filter, `keys` handed to it,
        /// holding `value`, with `clip` on the clipboard, fed `events`, one
        /// widget instance each, as the host rebuilds the view. It starts
        /// focused, the caret at the end, unless not `focused`. With
        /// `announce` each press follows the change to its modifiers, as
        /// the window reports them; without, the input was mounted with
        /// them already held and never hears them change.
        fn run(
            keys: &FieldKeys,
            value: &str,
            clip: Option<&str>,
            events: &[Event],
            focused: bool,
            announce: bool,
        ) -> Run {
            let mut value = value.to_owned();
            let element = |value: &str| -> Element<'static, String, Theme, ()> {
                crate::keys::alt_filter(field_keys(
                    text_input("", value)
                        .id(ID)
                        .width(200)
                        .on_input(|text: String| text),
                    keys,
                ))
                .into()
            };
            let mut first = element(&value);
            let mut tree = Tree::new(&first);
            let node = first.as_widget_mut().layout(
                &mut tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 100.0)),
            );
            if focused {
                first.as_widget_mut().operate(
                    &mut tree,
                    Layout::new(&node),
                    &(),
                    &mut operation::focusable::focus(Id::new(ID)),
                );
            }
            let mut kept = Kept(clip.map(str::to_owned));
            let mut published = Vec::new();
            let mut captured = Vec::new();
            for event in events {
                let mut sent = Vec::new();
                if let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = event {
                    if announce {
                        sent.push(Event::Keyboard(keyboard::Event::ModifiersChanged(
                            *modifiers,
                        )));
                    }
                }
                sent.push(event.clone());
                for event in &sent {
                    let mut element = element(&value);
                    element.as_widget_mut().diff(&mut tree);
                    let mut messages = Vec::new();
                    let mut shell = Shell::new(&mut messages);
                    element.as_widget_mut().update(
                        &mut tree,
                        event,
                        Layout::new(&node),
                        mouse::Cursor::Unavailable,
                        &(),
                        &mut kept,
                        &mut shell,
                        &Rectangle::new(Point::ORIGIN, Size::new(400.0, 100.0)),
                    );
                    if matches!(event, Event::Keyboard(keyboard::Event::KeyPressed { .. })) {
                        captured.push(shell.event_status() == iced::event::Status::Captured);
                    }
                    if let Some(last) = messages.last() {
                        value.clone_from(last);
                    }
                    published.extend(messages);
                }
            }
            Run {
                published,
                captured,
                kept: kept.0,
            }
        }

        fn field(keys: &FieldKeys, value: &str, events: &[Event]) -> (Vec<String>, Option<String>) {
            let run = run(keys, value, None, events, true, true);
            (run.published, run.kept)
        }

        #[test]
        fn in_alt_mode_alt_edits_a_text_field_as_ctrl_does() {
            let keys = linux(LinuxModifier::Alt);
            let alt = Modifiers::ALT;
            for modifiers in [Modifiers::ALT, Modifiers::CTRL] {
                let (published, kept) = field(
                    &keys,
                    "abc",
                    &[
                        chord("a", modifiers),
                        chord("c", modifiers),
                        end(),
                        chord("v", modifiers),
                    ],
                );
                assert_eq!(published, ["abcabc"], "{modifiers:?}");
                assert_eq!(kept.as_deref(), Some("abc"), "{modifiers:?}");
            }
            let (published, kept) = field(&keys, "abc", &[chord("a", alt), chord("x", alt)]);
            assert_eq!(
                (published, kept.as_deref()),
                (vec![String::new()], Some("abc"))
            );
            // The input has no history: undo and redo do nothing, and never
            // type their letter.
            let (published, kept) = field(
                &keys,
                "abc",
                &[chord("z", alt), chord("z", alt | Modifiers::SHIFT)],
            );
            assert_eq!((published, kept), (Vec::<String>::new(), None));
            // Any other Alt key is no edit of the field's.
            let (published, _) = field(&keys, "abc", &[chord("b", alt)]);
            assert!(published.is_empty(), "{published:?}");
        }

        #[test]
        fn in_ctrl_mode_the_default_field_does_what_it_does_without_the_wrapper() {
            let keys = linux(LinuxModifier::Ctrl);
            let ctrl = Modifiers::CTRL;
            let alt = Modifiers::ALT;
            let cases: [&[Event]; 4] = [
                &[chord("a", ctrl), chord("c", ctrl), end(), chord("v", ctrl)],
                &[chord("a", alt), chord("c", alt)],
                &[chord("v", alt)],
                &[chord("a", ctrl), chord("x", ctrl)],
            ];
            for events in cases {
                let wrapped = run(&keys, "abc", Some("clip"), events, true, true);
                let bare = run(
                    &FieldKeys::default(),
                    "abc",
                    Some("clip"),
                    events,
                    true,
                    true,
                );
                assert_eq!(wrapped.published, bare.published, "{events:?}");
                assert_eq!(wrapped.captured, bare.captured, "{events:?}");
                assert_eq!(wrapped.kept, bare.kept, "{events:?}");
            }
            let (published, kept) = field(&keys, "abc", &[chord("a", alt), chord("c", alt)]);
            assert_eq!((published, kept), (Vec::<String>::new(), None));
            // A user's `alt+x = edit.cut` cuts in the field.
            let mut edits = linux_edits(LinuxModifier::Ctrl);
            edits.push((accel("alt+x"), EditKey::Cut));
            let scheme = Scheme::new(Platform::Linux, LinuxModifier::Ctrl);
            let keys = FieldKeys::new(scheme, edits, []);
            let (published, kept) = field(&keys, "abc", &[chord("a", ctrl), chord("x", alt)]);
            assert_eq!(
                (published, kept.as_deref()),
                (vec![String::new()], Some("abc"))
            );
        }

        #[test]
        fn a_chord_given_another_edit_does_that_edit_in_the_field() {
            let scheme = Scheme::new(Platform::Linux, LinuxModifier::Alt);
            let mut edits = linux_edits(LinuxModifier::Alt);
            edits.retain(|(bound, _)| *bound != accel("ctrl+v"));
            edits.push((accel("ctrl+v"), EditKey::Copy));
            let keys = FieldKeys::new(scheme, edits, [accel("ctrl+v")]);
            let run = run(
                &keys,
                "abc",
                Some("clip"),
                &[chord("a", Modifiers::CTRL), chord("v", Modifiers::CTRL)],
                true,
                true,
            );
            assert!(run.published.is_empty(), "{:?}", run.published);
            assert_eq!(run.kept.as_deref(), Some("abc"));
        }

        /// A field owning `?`, as the keys sheet's filter owns its toggle,
        /// and Ctrl+X, as it owns a toggle bound there.
        fn owned(events: &[Event], clip: Option<&str>) -> (Vec<String>, Vec<bool>, Option<String>) {
            let mut value = String::from("abc");
            let keys = linux(LinuxModifier::Ctrl);
            let element = |value: &str| -> Element<'static, String, Theme, ()> {
                crate::keys::alt_filter(field_keys_owning(
                    text_input("", value)
                        .id(ID)
                        .width(200)
                        .on_input(|text: String| text),
                    &keys,
                    |key: &Key, _, modifiers: Modifiers| match key.as_ref() {
                        Key::Character("?") => modifiers == Modifiers::SHIFT,
                        Key::Character("x") => modifiers == Modifiers::CTRL,
                        _ => false,
                    },
                    || "<owned>".to_owned(),
                ))
                .into()
            };
            let mut first = element(&value);
            let mut tree = Tree::new(&first);
            let node = first.as_widget_mut().layout(
                &mut tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 100.0)),
            );
            first.as_widget_mut().operate(
                &mut tree,
                Layout::new(&node),
                &(),
                &mut operation::focusable::focus(Id::new(ID)),
            );
            let mut kept = Kept(clip.map(str::to_owned));
            let (mut published, mut captured) = (Vec::new(), Vec::new());
            for event in events {
                let mut element = element(&value);
                element.as_widget_mut().diff(&mut tree);
                let mut messages = Vec::new();
                let mut shell = Shell::new(&mut messages);
                element.as_widget_mut().update(
                    &mut tree,
                    event,
                    Layout::new(&node),
                    mouse::Cursor::Unavailable,
                    &(),
                    &mut kept,
                    &mut shell,
                    &Rectangle::new(Point::ORIGIN, Size::new(400.0, 100.0)),
                );
                captured.push(shell.event_status() == iced::event::Status::Captured);
                if let Some(last) = messages.last().filter(|last| *last != "<owned>") {
                    value.clone_from(last);
                }
                published.extend(messages);
            }
            (published, captured, kept.0)
        }

        #[test]
        fn an_owned_key_is_taken_from_the_field_and_its_text_is_not() {
            let question = Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character("?".into()),
                modified_key: Key::Character("?".into()),
                physical_key: Physical::Code(Code::Slash),
                location: Location::Standard,
                modifiers: Modifiers::SHIFT,
                text: Some("?".into()),
                repeat: false,
            });
            // A typed `?` is the owner's, and types nothing.
            let (published, captured, _) = owned(&[question], None);
            assert_eq!(
                (published, captured),
                (vec!["<owned>".to_owned()], vec![true])
            );
            // `what?` pasted filters.
            let (published, _, _) = owned(&[end(), chord("v", Modifiers::CTRL)], Some("what?"));
            assert_eq!(published, ["abcwhat?"]);
            // A shortcut the owner names is its too: no cut.
            let (published, captured, kept) = owned(
                &[chord("a", Modifiers::CTRL), chord("x", Modifiers::CTRL)],
                Some("sentinel"),
            );
            assert_eq!(published, ["<owned>"]);
            assert_eq!((captured[1], kept.as_deref()), (true, Some("sentinel")));
        }

        #[test]
        fn a_chord_the_keymap_gives_elsewhere_is_taken_from_the_focused_field() {
            // `keybind = ctrl+v = navigation.find`, or `= unbind`.
            for mode in [LinuxModifier::Ctrl, LinuxModifier::Alt] {
                let scheme = Scheme::new(Platform::Linux, mode);
                let keys = FieldKeys::new(scheme, linux_edits(mode), [accel("ctrl+v")]);
                let paste = [end(), chord("v", Modifiers::CTRL)];
                let focused = run(&keys, "abc", Some("clip"), &paste, true, true);
                assert!(focused.published.is_empty(), "{:?}", focused.published);
                assert!(focused.captured[1], "{mode:?}");
                // An unfocused field leaves the press alone.
                let unfocused = run(&keys, "abc", Some("clip"), &paste, false, true);
                assert!(unfocused.published.is_empty());
                assert!(!unfocused.captured[1], "{mode:?}");
                // The field's other chords are its own still.
                let copy = run(
                    &keys,
                    "abc",
                    None,
                    &[chord("a", Modifiers::CTRL), chord("c", Modifiers::CTRL)],
                    true,
                    true,
                );
                assert_eq!(copy.kept.as_deref(), Some("abc"), "{mode:?}");
            }
        }

        #[test]
        fn a_field_mounted_with_ctrl_held_pastes_on_ctrl_v() {
            // Ctrl held to open the find bar: the field never heard Ctrl go
            // down, and hears no change before V.
            for mode in [LinuxModifier::Ctrl, LinuxModifier::Alt] {
                let events = [chord("v", Modifiers::CTRL)];
                let run = run(&linux(mode), "abc", Some("clip"), &events, true, false);
                assert_eq!(run.published, ["abcclip"], "{mode:?}");
            }
            // As iced's own input does not.
            let bare = run(
                &FieldKeys::default(),
                "abc",
                Some("clip"),
                &[chord("v", Modifiers::CTRL)],
                true,
                false,
            );
            assert!(bare.published.is_empty(), "{:?}", bare.published);
        }
    }
}
