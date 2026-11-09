//! Keyboard input: iced key events → keybinding triggers, plus the root
//! filter that keeps Alt shortcuts from typing text on Linux.

use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{layout, mouse, overlay, renderer, Clipboard, Layout, Shell, Widget};
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::{Element, Event, Length, Rectangle, Size, Vector};
use wren_core::keybind::{canonical_key, Accel, Mods};

/// Keys the app handles itself regardless of the keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Escape,
    /// Esc that a text field already handled (it unfocuses itself).
    FieldEscape,
    Up,
    Down,
    Left,
    Right,
    Enter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Nav(Nav),
    Accel(Accel),
}

/// Classify a key press. Unmodified Escape/arrows/Enter are navigation;
/// anything with Ctrl, Alt, or Super, and any function key, becomes a
/// trigger for the keymap.
pub fn input(event: &keyboard::Event) -> Option<Input> {
    let keyboard::Event::KeyPressed {
        key,
        physical_key,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    if let Some(trigger) = shortcut(key, *physical_key, *modifiers) {
        return Some(Input::Accel(trigger));
    }
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    match key {
        Key::Named(Named::Escape) => Some(Input::Nav(Nav::Escape)),
        Key::Named(Named::ArrowUp) => Some(Input::Nav(Nav::Up)),
        Key::Named(Named::ArrowDown) => Some(Input::Nav(Nav::Down)),
        Key::Named(Named::ArrowLeft) => Some(Input::Nav(Nav::Left)),
        Key::Named(Named::ArrowRight) => Some(Input::Nav(Nav::Right)),
        Key::Named(Named::Enter) => Some(Input::Nav(Nav::Enter)),
        _ => None,
    }
}

/// The keymap trigger a key press is, if it can be one: anything with Ctrl,
/// Alt, or Super, and any function key. The app's shortcuts and the editor's
/// key policy both read keys through here, so they never disagree.
pub fn shortcut(
    key: &Key,
    physical: keyboard::key::Physical,
    modifiers: Modifiers,
) -> Option<Accel> {
    accel(key, physical, modifiers).filter(|accel| {
        modifiers.control()
            || modifiers.alt()
            || modifiers.logo()
            || wren_core::keybind::is_function_key(&accel.key)
    })
}

/// Whether the app acts on a key press a widget may already have handled.
/// Arrows and Enter that the editor, a text field, or a resize grip captured
/// belong to that widget; Escape and shortcuts always reach the app.
pub fn reaches_app(input: &Input, status: iced::event::Status) -> bool {
    match input {
        Input::Nav(Nav::Escape | Nav::FieldEscape) | Input::Accel(_) => true,
        Input::Nav(_) => status == iced::event::Status::Ignored,
    }
}

/// Mark an Esc a text field captured, so the app can tell leaving a filter
/// box apart from leaving a column.
pub fn with_status(input: Input, status: iced::event::Status) -> Input {
    match input {
        Input::Nav(Nav::Escape) if status == iced::event::Status::Captured => {
            Input::Nav(Nav::FieldEscape)
        }
        input => input,
    }
}

/// The trigger for a key press, using the physical key's Latin equivalent so
/// bindings work on non-Latin layouts and with macOS Option compositions
/// (⌘⌥1 reports `¡` as its character).
pub fn accel(key: &Key, physical: keyboard::key::Physical, modifiers: Modifiers) -> Option<Accel> {
    let name = match key.as_ref() {
        Key::Named(named) => named_key(named)?.to_string(),
        // Shift turns `[` into `{` on US layouts; bindings name the key, so
        // `shift+bracketleft` must match. Letters stay as typed.
        Key::Character(character)
            if modifiers.shift() && !character.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            match unshifted(physical) {
                Some(base) => base.to_string(),
                None if character.is_ascii() => character.to_lowercase(),
                None => Key::Character("\u{3042}".into())
                    .to_latin(physical)?
                    .to_string(),
            }
        }
        Key::Character(character) if character.is_ascii() => character.to_lowercase(),
        // Option compositions (`¡`, `å`) and non-Latin layouts: use the
        // physical key. iced's `to_latin` keeps Latin-1 characters, so ask it
        // with a non-Latin placeholder to force the physical lookup.
        Key::Character(_) => Key::Character("\u{3042}".into())
            .to_latin(physical)?
            .to_string(),
        Key::Unidentified => return None,
    };
    Some(Accel {
        mods: Mods {
            ctrl: modifiers.control(),
            alt: modifiers.alt(),
            shift: modifiers.shift(),
            logo: modifiers.logo(),
        },
        key: canonical_key(&name)?,
    })
}

/// The unshifted US character on a physical punctuation or digit key.
fn unshifted(physical: keyboard::key::Physical) -> Option<char> {
    use keyboard::key::{Code, Physical};
    let Physical::Code(code) = physical else {
        return None;
    };
    Some(match code {
        Code::BracketLeft => '[',
        Code::BracketRight => ']',
        Code::Minus => '-',
        Code::Equal => '=',
        Code::Comma => ',',
        Code::Period => '.',
        Code::Slash => '/',
        Code::Backslash => '\\',
        Code::Semicolon => ';',
        Code::Quote => '\'',
        Code::Backquote => '`',
        Code::Digit0 => '0',
        Code::Digit1 => '1',
        Code::Digit2 => '2',
        Code::Digit3 => '3',
        Code::Digit4 => '4',
        Code::Digit5 => '5',
        Code::Digit6 => '6',
        Code::Digit7 => '7',
        Code::Digit8 => '8',
        Code::Digit9 => '9',
        _ => return None,
    })
}

fn named_key(named: Named) -> Option<&'static str> {
    Some(match named {
        Named::Enter => "enter",
        Named::Escape => "escape",
        Named::Space => "space",
        Named::Tab => "tab",
        Named::Backspace => "backspace",
        Named::Delete => "delete",
        Named::ArrowLeft => "left",
        Named::ArrowRight => "right",
        Named::ArrowUp => "up",
        Named::ArrowDown => "down",
        Named::Home => "home",
        Named::End => "end",
        Named::PageUp => "pageup",
        Named::PageDown => "pagedown",
        Named::F1 => "f1",
        Named::F2 => "f2",
        Named::F3 => "f3",
        Named::F4 => "f4",
        Named::F5 => "f5",
        Named::F6 => "f6",
        Named::F7 => "f7",
        Named::F8 => "f8",
        Named::F9 => "f9",
        Named::F10 => "f10",
        Named::F11 => "f11",
        Named::F12 => "f12",
        Named::F13 => "f13",
        Named::F14 => "f14",
        Named::F15 => "f15",
        Named::F16 => "f16",
        Named::F17 => "f17",
        Named::F18 => "f18",
        Named::F19 => "f19",
        Named::F20 => "f20",
        Named::F21 => "f21",
        Named::F22 => "f22",
        Named::F23 => "f23",
        Named::F24 => "f24",
        _ => return None,
    })
}

/// Linux reports text for Alt+letter, which iced's text widgets would insert
/// alongside the Alt shortcut. Alt+letter never types in Linux desktop apps,
/// so the text is dropped before it reaches any widget. macOS keeps it:
/// Option is a composition key there.
fn strip_alt_text(event: &Event) -> Option<Event> {
    if cfg!(target_os = "macos") {
        return None;
    }
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            modifiers,
            text: Some(_),
            repeat,
        }) if modifiers.alt() && !modifiers.control() => {
            Some(Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: modified_key.clone(),
                physical_key: *physical_key,
                location: *location,
                modifiers: *modifiers,
                text: None,
                repeat: *repeat,
            }))
        }
        _ => None,
    }
}

/// Wraps the root element and applies [`strip_alt_text`] to every event.
pub struct AltFilter<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

pub fn alt_filter<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> AltFilter<'a, Message, Theme, Renderer> {
    AltFilter {
        content: content.into(),
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for AltFilter<'_, Message, Theme, Renderer>
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
        let filtered = strip_alt_text(event);
        self.content.as_widget_mut().update(
            tree,
            filtered.as_ref().unwrap_or(event),
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
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

impl<'a, Message, Theme, Renderer> From<AltFilter<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(filter: AltFilter<'a, Message, Theme, Renderer>) -> Self {
        Element::new(filter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::Location;
    use wren_core::keybind::parse_trigger;

    fn press(key: Key, code: Code, modifiers: Modifiers, text: Option<&str>) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        }
    }

    #[test]
    fn modified_presses_become_canonical_triggers() {
        let palette = press(
            Key::Character("P".into()),
            Code::KeyP,
            Modifiers::ALT | Modifiers::SHIFT,
            Some("P"),
        );
        assert_eq!(
            input(&palette),
            Some(Input::Accel(parse_trigger("alt+shift+p").unwrap()))
        );
        // macOS reports ⌘⌥1 as `¡`; the physical key still identifies it.
        let layout = press(
            Key::Character("¡".into()),
            Code::Digit1,
            Modifiers::LOGO | Modifiers::ALT,
            None,
        );
        assert_eq!(
            input(&layout),
            Some(Input::Accel(parse_trigger("super+alt+1").unwrap()))
        );
        let back = press(
            Key::Character("[".into()),
            Code::BracketLeft,
            Modifiers::LOGO,
            None,
        );
        assert_eq!(
            input(&back),
            Some(Input::Accel(parse_trigger("super+bracketleft").unwrap()))
        );
    }

    #[test]
    fn plain_navigation_keys_are_not_triggers_and_typing_is_ignored() {
        let escape = press(
            Key::Named(Named::Escape),
            Code::Escape,
            Modifiers::empty(),
            None,
        );
        assert_eq!(input(&escape), Some(Input::Nav(Nav::Escape)));
        let typed = press(
            Key::Character("p".into()),
            Code::KeyP,
            Modifiers::empty(),
            Some("p"),
        );
        assert_eq!(input(&typed), None);
        let shifted = press(
            Key::Character("P".into()),
            Code::KeyP,
            Modifiers::SHIFT,
            Some("P"),
        );
        assert_eq!(input(&shifted), None);
        let function = press(Key::Named(Named::F13), Code::F13, Modifiers::SHIFT, None);
        assert_eq!(
            input(&function),
            Some(Input::Accel(parse_trigger("shift+f13").unwrap()))
        );
    }

    #[test]
    fn arrows_are_navigation_and_shift_brackets_name_the_physical_key() {
        for (named, code, nav) in [
            (Named::ArrowLeft, Code::ArrowLeft, Nav::Left),
            (Named::ArrowRight, Code::ArrowRight, Nav::Right),
            (Named::ArrowUp, Code::ArrowUp, Nav::Up),
            (Named::ArrowDown, Code::ArrowDown, Nav::Down),
            (Named::Enter, Code::Enter, Nav::Enter),
        ] {
            let event = press(Key::Named(named), code, Modifiers::empty(), None);
            assert_eq!(input(&event), Some(Input::Nav(nav)));
        }
        // Shift+[ reports `{` on US layouts; bindings name the physical key.
        for (character, code, trigger) in [
            ("{", Code::BracketLeft, "alt+shift+bracketleft"),
            ("}", Code::BracketRight, "alt+shift+bracketright"),
        ] {
            let event = press(
                Key::Character(character.into()),
                code,
                Modifiers::ALT | Modifiers::SHIFT,
                None,
            );
            assert_eq!(
                input(&event),
                Some(Input::Accel(parse_trigger(trigger).unwrap()))
            );
        }
    }

    #[test]
    fn captured_arrows_and_enter_stay_with_their_widget() {
        use iced::event::Status;
        for nav in [Nav::Up, Nav::Down, Nav::Left, Nav::Right, Nav::Enter] {
            assert!(reaches_app(&Input::Nav(nav), Status::Ignored));
            assert!(!reaches_app(&Input::Nav(nav), Status::Captured));
        }
        assert!(reaches_app(&Input::Nav(Nav::Escape), Status::Captured));
        assert_eq!(
            with_status(Input::Nav(Nav::Escape), Status::Captured),
            Input::Nav(Nav::FieldEscape)
        );
        assert_eq!(
            with_status(Input::Nav(Nav::Escape), Status::Ignored),
            Input::Nav(Nav::Escape)
        );
        let accel = Input::Accel(parse_trigger("alt+shift+bracketleft").unwrap());
        assert!(reaches_app(&accel, Status::Captured));
    }

    #[test]
    fn alt_text_is_stripped_off_macos_only() {
        let event = Event::Keyboard(press(
            Key::Character("n".into()),
            Code::KeyN,
            Modifiers::ALT,
            Some("n"),
        ));
        let stripped = strip_alt_text(&event);
        if cfg!(target_os = "macos") {
            assert!(stripped.is_none());
        } else {
            assert!(matches!(
                stripped,
                Some(Event::Keyboard(keyboard::Event::KeyPressed {
                    text: None,
                    ..
                }))
            ));
        }
        let altgr_like = Event::Keyboard(press(
            Key::Character("@".into()),
            Code::KeyQ,
            Modifiers::ALT | Modifiers::CTRL,
            Some("@"),
        ));
        assert!(strip_alt_text(&altgr_like).is_none());
    }
}
