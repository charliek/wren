//! A one-line field a host opens in place of what it shows, to edit it:
//! iced's own text input, so typing, selection, paste and an input
//! method's composition are the toolkit's, with Enter and Escape taken
//! before it. The task view's description edit is one (item 10.5).
//!
//! While the field has the keyboard:
//!
//! - Enter with no modifiers is [`LineEvent::Enter`], and Escape with no
//!   Ctrl, Alt or logo key [`LineEvent::Escape`]. Escape with one of them
//!   is no one's: it is taken, so the text input does not give up the
//!   keyboard for it.
//! - Tab and Shift+Tab do nothing, so the keyboard stays in the field.
//! - Tab or Escape with Ctrl, Alt or the logo key that the keymap gives an
//!   `edit.*` action is that edit ([`crate::field_keys`]), as in any field.
//! - Every other key, the shortcuts included, is the text input's.
//!
//! Each shortcut (Ctrl, Alt or the logo key held, or a function key;
//! AltGr's text is no shortcut) is reported, at its place in the batch, as
//! [`LineEvent::Shortcut`], `held` when the field took it (it does nothing
//! or is the field's own) or came after the field closed: the host runs
//! the app's command for one that is not held there and then, and drops
//! the app's copy of every one.
//!
//! While an input method composes (its preedit holds text), Enter (with
//! any modifiers) and Tab are taken and do nothing, so a composition is
//! never written half made, and a plain Escape is the input method's,
//! reported as [`LineEvent::HeldEscape`] so the host's own key handling,
//! which hears of every plain Escape, knows it was no one's.
//!
//! **A close is a routing boundary.** Escape, and an Enter with text in
//! the field (not white space alone), close it: the rest of its batch was
//! typed before the user could see the field go, and is dropped, never
//! typed. A plain Escape among it is reported as [`LineEvent::HeldEscape`]
//! and a shortcut as a held [`LineEvent::Shortcut`], so the host drops its
//! copies too. An Enter on an empty field closes
//! nothing, and the field keeps the keyboard.
//!
//! The field reports [`LineEvent::Blur`] whenever it loses the keyboard
//! inside the window: a focus moved elsewhere, or a press of the pointer
//! outside it. The window losing the keyboard is no blur: the text input
//! keeps its focus for when the window has it again.
//!
//! Each field the host opens has its own number: a field built at the same
//! place in the widget tree as one that closed starts afresh.

use std::cell::RefCell;
use std::rc::Rc;

use iced::advanced::input_method;
use iced::advanced::layout::{self, Layout};
use iced::advanced::text;
use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{tree, Id, Operation, Tree};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell, Widget};
use iced::keyboard::key::Named;
use iced::keyboard::{self, Key, Modifiers};
use iced::widget::text_input::{self, TextInput};
use iced::{Element, Event, Font, Length, Rectangle, Size, Vector};
use wren_core::keybind::Accel;

use crate::field_keys::FieldKeys;

/// What the field hands the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEvent {
    /// The field's text is now this: typed, pasted, cut or composed.
    Typed(String),
    /// Enter, with no input method composing.
    Enter,
    /// A plain Escape, with no input method composing.
    Escape,
    /// A plain Escape that is no one's: the input method's while it
    /// composed, or one after the field closed in its batch.
    HeldEscape,
    /// A shortcut pressed while the field had the keyboard, `held` when
    /// the field took it or it came after the field closed in its batch.
    Shortcut { accel: Accel, held: bool },
    /// The field lost the keyboard inside the window.
    Blur,
    /// An input method started (`true`) or stopped composing in the field.
    Composing(bool),
}

/// The field's text input, given its look by the host.
type Input<'a, Message, Theme, Renderer> = TextInput<'a, Message, Theme, Renderer>;

/// How the host gives the text input its look.
type Configure<'a, Message, Theme, Renderer> =
    Rc<dyn Fn(Input<'a, Message, Theme, Renderer>) -> Input<'a, Message, Theme, Renderer> + 'a>;

/// The field numbered `instance`, showing `value`, whose text input
/// `configure` gives its look (id, font, style). Everything it reports
/// goes through `on_event`.
pub fn line_field<'a, Message, Theme, Renderer>(
    value: &str,
    instance: u64,
    configure: impl Fn(Input<'a, Message, Theme, Renderer>) -> Input<'a, Message, Theme, Renderer> + 'a,
    on_event: impl Fn(LineEvent) -> Message + 'a,
) -> LineField<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    let mut field = LineField {
        input: Element::new(iced::widget::Space::new()),
        instance,
        live: Rc::new(RefCell::new(value.to_owned())),
        configure: Rc::new(configure),
        on_event: Rc::new(on_event),
        keys: FieldKeys::default(),
    };
    field.input = field.build(value);
    field
}

pub struct LineField<'a, Message, Theme, Renderer>
where
    Theme: text_input::Catalog,
    Renderer: text::Renderer<Font = Font>,
{
    input: Element<'a, Message, Theme, Renderer>,
    instance: u64,
    /// The field's text as the events of this batch leave it.
    live: Rc<RefCell<String>>,
    configure: Configure<'a, Message, Theme, Renderer>,
    on_event: Rc<dyn Fn(LineEvent) -> Message + 'a>,
    /// The keymap's text editing keys, handed to the text input as its own.
    keys: FieldKeys,
}

#[derive(Debug, Default)]
struct State {
    /// The field these are of.
    instance: u64,
    /// The input method's preedit holds text.
    composing: bool,
    /// The text input had the keyboard after the last event.
    focused: bool,
    /// A key of the field's closed it: it takes no more input.
    closed: bool,
}

/// What the field does with a key press before the text input sees it:
/// `None` leaves it to the text input; `Some` takes it, publishing the
/// event it holds, if any.
pub fn route(key: &Key, modifiers: Modifiers, composing: bool) -> Option<Option<LineEvent>> {
    let Key::Named(named) = key.as_ref() else {
        return None;
    };
    // The Escape a host navigates with, as the editor's popovers read it.
    let plain_escape = wren_editor::is_plain_escape(modifiers);
    if composing {
        return match named {
            Named::Enter | Named::Tab => Some(None),
            Named::Escape if plain_escape => Some(Some(LineEvent::HeldEscape)),
            Named::Escape => Some(None),
            _ => None,
        };
    }
    match named {
        Named::Escape if plain_escape => Some(Some(LineEvent::Escape)),
        Named::Escape | Named::Tab => Some(None),
        Named::Enter if modifiers.is_empty() => Some(Some(LineEvent::Enter)),
        _ => None,
    }
}

/// The shortcut a press is, as the app's key handling reads it: none for
/// AltGr's text, which is the focused widget's.
fn shortcut(event: &Event) -> Option<Accel> {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        physical_key,
        modifiers,
        text,
        ..
    }) = event
    else {
        return None;
    };
    if wren_editor::is_alt_graph(key, *physical_key, *modifiers, text.as_deref()) {
        return None;
    }
    crate::keys::shortcut(key, *physical_key, *modifiers)
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

impl<'a, Message, Theme, Renderer> LineField<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    /// The text input showing `value`, which reports what it is typed into.
    fn build(&self, value: &str) -> Element<'a, Message, Theme, Renderer> {
        let live = Rc::clone(&self.live);
        let on_event = Rc::clone(&self.on_event);
        let input = TextInput::new("", value).on_input(move |text: String| {
            live.borrow_mut().clone_from(&text);
            on_event(LineEvent::Typed(text))
        });
        crate::field_keys::field_keys((self.configure)(input), &self.keys)
    }

    /// Hand the text input the keymap's text editing `keys` as its own
    /// ([`crate::field_keys`]).
    #[must_use]
    pub fn field_keys(mut self, keys: &FieldKeys) -> Self {
        self.keys = keys.clone();
        let value = self.live.borrow().clone();
        self.input = self.build(&value);
        self
    }

    fn input_focused(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer) -> bool {
        let mut focus = HasFocus(false);
        self.input
            .as_widget_mut()
            .operate(tree, layout, renderer, &mut focus);
        focus.0
    }

    /// Whether Enter closes the field: with text in it.
    fn enter_closes(&self) -> bool {
        !self.live.borrow().trim().is_empty()
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for LineField<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            instance: self.instance,
            ..State::default()
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.input)]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.instance != self.instance {
            *state = State {
                instance: self.instance,
                ..State::default()
            };
        }
        tree.diff_children(std::slice::from_ref(&self.input));
    }

    fn size(&self) -> Size<Length> {
        self.input.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let child = self
            .input
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(child.size(), vec![child])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let child = layout.children().next().expect("the input's layout");
        self.input
            .as_widget_mut()
            .operate(&mut tree.children[0], child, renderer, operation);
        // Focused by an operation, before any event: a later one that moves
        // the focus elsewhere is a blur too.
        if self.input_focused(&mut tree.children[0], child, renderer) {
            tree.state.downcast_mut::<State>().focused = true;
        }
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
        let was = tree.state.downcast_ref::<State>().composing;
        'update: {
            let child = layout.children().next().expect("the input's layout");
            // A field that closed in this batch takes nothing more: what is
            // left of the batch is dropped, and the host told to drop its
            // copies of a plain Escape and of the shortcuts.
            if tree.state.downcast_ref::<State>().closed {
                if let Event::Keyboard(keyboard::Event::KeyPressed {
                    key: Key::Named(Named::Escape),
                    modifiers,
                    ..
                }) = event
                {
                    if wren_editor::is_plain_escape(*modifiers) {
                        shell.publish((self.on_event)(LineEvent::HeldEscape));
                    }
                }
                if let Some(accel) = shortcut(event) {
                    shell.publish((self.on_event)(LineEvent::Shortcut { accel, held: true }));
                }
                if !matches!(event, Event::Window(_)) {
                    shell.capture_event();
                    break 'update;
                }
            }
            let before = self.input_focused(&mut tree.children[0], child, renderer);
            let state = tree.state.downcast_mut::<State>();
            // A focus moved by an operation since the last event.
            if state.focused && !before {
                state.focused = false;
                state.composing = false;
                shell.publish((self.on_event)(LineEvent::Blur));
            }
            match event {
                Event::InputMethod(input_method::Event::Preedit(content, _)) if before => {
                    state.composing = !content.is_empty();
                }
                Event::InputMethod(
                    input_method::Event::Commit(_)
                    | input_method::Event::Closed
                    | input_method::Event::Opened,
                ) => state.composing = false,
                Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if before => {
                    // Tab or Escape with a modifier that the keymap gives an
                    // edit is that edit, the field keys' to hand the input.
                    let edit = !state.composing
                        && matches!(key, Key::Named(Named::Tab | Named::Escape))
                        && !wren_editor::is_plain_escape(*modifiers)
                        && !(modifiers.is_empty() || *modifiers == Modifiers::SHIFT)
                        && self.keys.rewrite(event).is_some();
                    let taken = if edit {
                        None
                    } else {
                        route(key, *modifiers, state.composing)
                    };
                    if let Some(accel) = shortcut(event) {
                        shell.publish((self.on_event)(LineEvent::Shortcut {
                            accel,
                            held: taken.is_some(),
                        }));
                    }
                    if let Some(taken) = taken {
                        let closes = match &taken {
                            Some(LineEvent::Escape) => true,
                            Some(LineEvent::Enter) => self.enter_closes(),
                            _ => false,
                        };
                        if closes {
                            tree.state.downcast_mut::<State>().closed = true;
                        }
                        if let Some(taken) = taken {
                            shell.publish((self.on_event)(taken));
                        }
                        shell.capture_event();
                        break 'update;
                    }
                    // The text input reads the modifiers it last heard change,
                    // which a field made while one was held never heard: the
                    // press's own say what it is, ⌘V or Ctrl+V say.
                    self.input.as_widget_mut().update(
                        &mut tree.children[0],
                        &Event::Keyboard(keyboard::Event::ModifiersChanged(*modifiers)),
                        child,
                        cursor,
                        renderer,
                        clipboard,
                        shell,
                        viewport,
                    );
                }
                _ => {}
            }
            self.input.as_widget_mut().update(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
            let after = self.input_focused(&mut tree.children[0], child, renderer);
            let state = tree.state.downcast_mut::<State>();
            if before && !after {
                state.composing = false;
                shell.publish((self.on_event)(LineEvent::Blur));
            }
            state.focused = after;
        }
        let now = tree.state.downcast_ref::<State>().composing;
        if now != was {
            shell.publish((self.on_event)(LineEvent::Composing(now)));
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let child = layout.children().next().expect("the input's layout");
        self.input.as_widget().mouse_interaction(
            &tree.children[0],
            child,
            cursor,
            viewport,
            renderer,
        )
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
        let child = layout.children().next().expect("the input's layout");
        self.input.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let child = layout.children().next().expect("the input's layout");
        self.input.as_widget_mut().overlay(
            &mut tree.children[0],
            child,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<LineField<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn from(field: LineField<'a, Message, Theme, Renderer>) -> Self {
        Element::new(field)
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::widget::operation;
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::Location;
    use iced::{window, Point, Theme};

    use super::*;

    const ID: &str = "line-field-test";

    /// One field, laid out once, fed whole batches of events: one widget
    /// instance for each batch, as iced hands a frame's events to the
    /// widgets of one view; the host's view of the text, as the last
    /// `Typed` left it, is the next batch's value.
    struct Field {
        value: String,
        instance: u64,
        keys: FieldKeys,
        tree: Tree,
        node: layout::Node,
    }

    fn element(
        value: &str,
        instance: u64,
        keys: &FieldKeys,
    ) -> Element<'static, LineEvent, Theme, ()> {
        line_field(
            value,
            instance,
            |input| input.id(ID).width(200),
            |event| event,
        )
        .field_keys(keys)
        .into()
    }

    impl Field {
        fn new(value: &str) -> Self {
            Self::with_keys(value, FieldKeys::default())
        }

        fn with_keys(value: &str, keys: FieldKeys) -> Self {
            let mut element = element(value, 1, &keys);
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(
                &mut tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 100.0)),
            );
            let mut field = Self {
                value: value.to_owned(),
                instance: 1,
                keys,
                tree,
                node,
            };
            field.focus();
            field
        }

        fn operate(&mut self, operation: &mut dyn Operation) {
            let mut element = element(&self.value, self.instance, &self.keys);
            element.as_widget_mut().operate(
                &mut self.tree,
                Layout::new(&self.node),
                &(),
                operation,
            );
        }

        fn focus(&mut self) {
            self.operate(&mut operation::focusable::focus(Id::new(ID)));
        }

        fn unfocus(&mut self) {
            self.operate(&mut operation::focusable::unfocus());
        }

        fn focused(&mut self) -> bool {
            let mut focus = HasFocus(false);
            self.operate(&mut focus);
            focus.0
        }

        fn batch_with(
            &mut self,
            events: &[Event],
            pointer: Option<Point>,
            clipboard: &mut dyn Clipboard,
        ) -> (Vec<LineEvent>, Vec<bool>) {
            let mut element = element(&self.value, self.instance, &self.keys);
            element.as_widget_mut().diff(&mut self.tree);
            let mut messages = Vec::new();
            let mut captured = Vec::new();
            for event in events {
                let mut shell = Shell::new(&mut messages);
                let cursor = pointer.map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available);
                element.as_widget_mut().update(
                    &mut self.tree,
                    &as_delivered(event),
                    Layout::new(&self.node),
                    cursor,
                    &(),
                    clipboard,
                    &mut shell,
                    &Rectangle::new(Point::ORIGIN, Size::new(400.0, 100.0)),
                );
                captured.push(shell.event_status() == iced::event::Status::Captured);
            }
            for message in &messages {
                if let LineEvent::Typed(value) = message {
                    self.value.clone_from(value);
                }
            }
            (messages, captured)
        }

        fn batch(&mut self, events: &[Event]) -> (Vec<LineEvent>, Vec<bool>) {
            self.batch_with(events, None, &mut iced::advanced::clipboard::Null)
        }
    }

    /// `event` as the app's root hands it on: on Linux an Alt press's
    /// text is dropped there (`keys::alt_filter`).
    fn as_delivered(event: &Event) -> Event {
        #[cfg(not(target_os = "macos"))]
        return crate::keys::tests::without_alt_text(event);
        #[cfg(target_os = "macos")]
        return event.clone();
    }

    fn press(key: Key, modifiers: Modifiers, text: Option<&str>) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Code(Code::KeyA),
            location: Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        })
    }

    fn named(named: Named) -> Event {
        press(Key::Named(named), Modifiers::empty(), None)
    }

    fn typed(character: &str) -> Event {
        press(
            Key::Character(character.into()),
            Modifiers::empty(),
            Some(character),
        )
    }

    fn preedit(text: &str) -> Event {
        Event::InputMethod(input_method::Event::Preedit(text.into(), None))
    }

    fn commit(text: &str) -> Event {
        Event::InputMethod(input_method::Event::Commit(text.into()))
    }

    fn text(value: &str) -> LineEvent {
        LineEvent::Typed(value.into())
    }

    fn shortcut(trigger: &str, held: bool) -> LineEvent {
        LineEvent::Shortcut {
            accel: wren_core::keybind::parse_trigger(trigger).expect("a trigger"),
            held,
        }
    }

    #[test]
    fn typing_is_the_inputs_tab_stays_and_enter_and_escape_are_the_hosts() {
        let mut line = Field::new("Order");
        let (messages, captured) = line.batch(&[
            typed(" "),
            typed("t"),
            named(Named::Tab),
            press(Key::Named(Named::Tab), Modifiers::SHIFT, None),
            typed("e"),
            named(Named::Enter),
        ]);
        assert_eq!(
            messages,
            [
                text("Order "),
                text("Order t"),
                text("Order te"),
                LineEvent::Enter
            ]
        );
        assert_eq!(captured, [true; 6]);
        assert!(line.focused(), "Tab never takes the keyboard from it");
        let mut line = Field::new("Order");
        let (messages, captured) = line.batch(&[named(Named::Escape)]);
        assert_eq!((messages, captured), (vec![LineEvent::Escape], vec![true]));
        // Escape with Ctrl, Alt or logo is no one's, and taken: the field
        // keeps the keyboard and its text.
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            let mut line = Field::new("Order");
            let escape = press(Key::Named(Named::Escape), modifiers, None);
            let (messages, _) = line.batch(&[escape, typed("x")]);
            assert_eq!(messages.last(), Some(&text("Orderx")), "{modifiers:?}");
            assert!(line.focused(), "{modifiers:?}");
        }
    }

    #[test]
    fn an_enter_on_an_empty_field_closes_nothing() {
        for value in ["", "   "] {
            let mut line = Field::new(value);
            let (messages, captured) =
                line.batch(&[named(Named::Enter), typed("x"), named(Named::Enter)]);
            assert_eq!(
                messages,
                [
                    LineEvent::Enter,
                    text(&format!("{value}x")),
                    LineEvent::Enter
                ],
                "{value:?}"
            );
            assert_eq!(captured, [true; 3]);
        }
    }

    #[test]
    fn a_key_that_closes_the_field_is_the_last_input_it_takes() {
        let ctrl_z = press(Key::Character("z".into()), Modifiers::CTRL, None);
        let cases: [(Vec<Event>, Vec<LineEvent>); 4] = [
            // Enter, then Space and a letter: never typed.
            (
                vec![named(Named::Enter), typed(" "), typed("x")],
                vec![LineEvent::Enter],
            ),
            // Escape, Escape: one close; the second is no one's, and the
            // host told so it drops its copy.
            (
                vec![named(Named::Escape), named(Named::Escape)],
                vec![LineEvent::Escape, LineEvent::HeldEscape],
            ),
            // A shortcut after the close: the host drops its copy.
            (
                vec![named(Named::Enter), ctrl_z.clone()],
                vec![LineEvent::Enter, shortcut("ctrl+z", true)],
            ),
            // A commit after the close has no field to go to.
            (
                vec![named(Named::Escape), commit("y"), typed("x")],
                vec![LineEvent::Escape],
            ),
        ];
        for (events, expected) in cases {
            let mut line = Field::new("Order");
            let (messages, captured) = line.batch(&events);
            assert_eq!(messages, expected, "{events:?}");
            assert!(captured.iter().all(|captured| *captured), "{events:?}");
        }
        // The next field opened there starts afresh.
        let mut line = Field::new("Order");
        let _ = line.batch(&[named(Named::Escape)]);
        line.instance = 2;
        line.focus();
        let (messages, _) = line.batch(&[typed("s")]);
        assert_eq!(messages, [text("Orders")]);
    }

    #[test]
    fn each_shortcut_is_reported_and_the_inputs_to_take() {
        let mut line = Field::new("Order");
        let (messages, _) = line.batch(&[
            press(Key::Character("z".into()), Modifiers::CTRL, None),
            press(Key::Named(Named::F2), Modifiers::empty(), None),
            press(Key::Named(Named::Enter), Modifiers::CTRL, None),
            press(Key::Named(Named::Escape), Modifiers::CTRL, None),
            press(Key::Named(Named::Tab), Modifiers::CTRL, None),
            typed("s"),
        ]);
        // Ctrl+Escape and Ctrl+Tab are taken, doing nothing: held.
        assert_eq!(
            messages,
            [
                shortcut("ctrl+z", false),
                shortcut("f2", false),
                shortcut("ctrl+enter", false),
                shortcut("ctrl+escape", true),
                shortcut("ctrl+tab", true),
                text("Orders"),
            ]
        );
        assert!(line.focused());
    }

    #[test]
    fn while_an_input_method_composes_enter_and_escape_are_its_own() {
        let mut line = Field::new("Order ");
        let (messages, captured) = line.batch(&[
            preedit("ふ"),
            named(Named::Enter),
            named(Named::Escape),
            commit("ふ"),
            named(Named::Enter),
            typed("x"),
        ]);
        assert_eq!(
            messages,
            [
                LineEvent::Composing(true),
                LineEvent::HeldEscape,
                text("Order ふ"),
                LineEvent::Composing(false),
                LineEvent::Enter,
            ]
        );
        assert_eq!(captured, [false, true, true, true, true, true]);
        // Tab, and Enter or Escape with a modifier, are the composition's
        // too: taken, and their app copies held.
        let mut line = Field::new("Order ");
        let (messages, captured) = line.batch(&[
            preedit("ふ"),
            named(Named::Tab),
            press(Key::Named(Named::Enter), Modifiers::CTRL, None),
            press(Key::Named(Named::Escape), Modifiers::ALT, None),
        ]);
        assert_eq!(
            messages,
            [
                LineEvent::Composing(true),
                shortcut("ctrl+enter", true),
                shortcut("alt+escape", true),
            ]
        );
        assert_eq!(captured, [false, true, true, true]);
        assert!(line.focused());
        // A preedit that empties again ends the composition.
        let mut line = Field::new("Order");
        let (messages, _) = line.batch(&[preedit("x"), preedit(""), named(Named::Escape)]);
        assert_eq!(
            messages,
            [
                LineEvent::Composing(true),
                LineEvent::Composing(false),
                LineEvent::Escape
            ]
        );
    }

    #[test]
    fn it_reports_losing_the_keyboard_in_the_window_and_not_the_window_losing_it() {
        // A press outside the field takes the keyboard from it: a blur.
        let mut line = Field::new("Order");
        let outside = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let (messages, _) = line.batch_with(
            &[outside],
            Some(Point::new(380.0, 90.0)),
            &mut iced::advanced::clipboard::Null,
        );
        assert_eq!(messages, [LineEvent::Blur]);
        assert!(!line.focused());
        // Focus moved elsewhere by an operation is a blur, reported at the
        // next event, once; an unfocused field takes no key.
        let mut line = Field::new("Order");
        line.unfocus();
        let moved = Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(1.0, 1.0),
        });
        let (messages, captured) = line.batch(&[moved.clone(), moved, typed("x")]);
        assert_eq!(messages, [LineEvent::Blur]);
        assert_eq!(captured, [false, false, false]);
        // The window losing the keyboard leaves the field as it was.
        let mut line = Field::new("Order");
        let (messages, _) = line.batch(&[
            Event::Window(window::Event::Unfocused),
            Event::Window(window::Event::Focused),
            typed("s"),
        ]);
        assert_eq!(messages, [text("Orders")]);
        assert!(line.focused());
    }

    /// With the keymap's text editing keys, in Linux's Alt mode Alt+V
    /// pastes in the field as Ctrl does, and is reported as the shortcut
    /// the app's key handling reads.
    // Real iced input on Linux, where its clipboard keys are Ctrl's.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn edit_paste_in_alt_mode_pastes() {
        use crate::field_keys::tests::{chord, linux, Kept};
        use wren_core::keybind::LinuxModifier;

        let mut line = Field::with_keys("Order ", linux(LinuxModifier::Alt));
        let mut kept = Kept(Some("tea".into()));
        let (messages, _) = line.batch_with(&[chord("v", Modifiers::ALT)], None, &mut kept);
        assert_eq!(messages, [shortcut("alt+v", false), text("Order tea")]);
        // Tab or Escape with a modifier that the keymap gives `edit.paste`
        // pastes; plain Tab and Escape keep their meaning.
        let scheme = wren_core::keybind::Scheme::new(
            wren_core::keybind::Platform::Linux,
            LinuxModifier::Ctrl,
        );
        let edits = [
            (
                scheme.parse("alt+escape").unwrap(),
                wren_editor::EditKey::Paste,
            ),
            (
                scheme.parse("ctrl+tab").unwrap(),
                wren_editor::EditKey::Paste,
            ),
        ];
        let keys = FieldKeys::new(scheme, edits, []);
        let mut line = Field::with_keys("Order ", keys);
        let (messages, _) = line.batch_with(
            &[
                press(Key::Named(Named::Escape), Modifiers::ALT, None),
                press(Key::Named(Named::Tab), Modifiers::CTRL, None),
                named(Named::Tab),
                named(Named::Escape),
            ],
            None,
            &mut kept,
        );
        assert_eq!(
            messages,
            [
                shortcut("alt+escape", false),
                text("Order tea"),
                shortcut("ctrl+tab", false),
                text("Order teatea"),
                LineEvent::Escape,
            ]
        );
        // In Ctrl mode Alt+V is no paste.
        let mut line = Field::with_keys("Order ", linux(LinuxModifier::Ctrl));
        let (messages, _) = line.batch_with(&[chord("v", Modifiers::ALT)], None, &mut kept);
        assert_eq!(messages, [shortcut("alt+v", false)]);
    }
}
