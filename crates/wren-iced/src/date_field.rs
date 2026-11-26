//! The calendar's field: iced's own text input, so typing, selection,
//! paste and an input method's composition are the toolkit's, with the
//! calendar's keys taken before it (discovery/tasks.md §6, "Calendar
//! popover").
//!
//! The field builds its text input itself and keeps the text as it stands
//! through a batch of events, which iced hands to one widget before the
//! host applies any message: so Left and Right know whether the field is
//! empty after the keys before them, and a key that moves the selection
//! empties the field at once, as the host empties its copy.
//!
//! While the field has the keyboard:
//!
//! - Up and Down, and Page Up and Page Down, are the calendar's
//!   ([`FieldEvent::Key`]), and empty the field; so are Left and Right
//!   while the field is empty, which with text in it move the field's
//!   caret.
//! - Enter is [`FieldEvent::Enter`], and Escape with no Ctrl, Alt or logo
//!   key [`FieldEvent::Escape`]. Escape with one of them is no one's: it
//!   is taken, so the text input does not give up the keyboard for it.
//! - Tab and Shift+Tab do nothing, so the keyboard stays in the field.
//! - Every other key, the shortcuts included, is the text input's.
//!
//! While an input method composes (its preedit holds text), no key is the
//! calendar's: Enter is taken and does nothing, so a composition is never
//! written half made, and a plain Escape is the input method's, reported as
//! [`FieldEvent::HeldEscape`] so the host's own key handling, which hears
//! of every plain Escape, knows it was no one's.
//!
//! **A close is a routing boundary.** A key of the field's that closes the
//! calendar, Escape or an Enter that writes (one the field's text names a
//! date for, or with nothing typed), is the last input the field takes:
//! the rest of its batch was typed before the user could see the calendar
//! go, and is dropped, never typed and never claimed. A plain Escape among
//! it is reported as [`FieldEvent::HeldEscape`], so the host's key handling
//! drops its copy too: two Escapes in one batch close the calendar once.
//! An Enter that writes nothing, and an Escape the input method holds,
//! close nothing, and the field keeps the keyboard.
//!
//! The field reports [`FieldEvent::Composing`] whenever an input method
//! starts or stops composing in it, so the host knows whether one was
//! composing when the calendar closed: only then may a commit still come
//! for it after the field has gone.
//!
//! The field reports [`FieldEvent::Blur`] when a focus moved elsewhere takes
//! the keyboard from it. Presses of the pointer are the host's to judge,
//! since a press on the calendar's own buttons moves the toolkit's focus
//! too. When the window loses the keyboard, the field reports
//! [`FieldEvent::WindowLost`] in the event's own order, and that is a close
//! as above: a composition's commit or Enter later in the batch were typed
//! for a field that has gone.

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
use iced::{window, Element, Event, Font, Length, Rectangle, Size, Vector};
use wren_markdown::calendar;
use wren_markdown::date::{self, Date, Parsed};

use crate::field_keys::FieldKeys;

/// What the field hands the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldEvent {
    /// The field's text is now this: typed, pasted, cut or composed.
    Typed(String),
    /// A key the calendar takes.
    Key(calendar::Key),
    /// Enter, with no input method composing.
    Enter,
    /// A plain Escape, with no input method composing.
    Escape,
    /// A plain Escape that is no one's: the input method's while it
    /// composed, or one after the field closed in its batch.
    HeldEscape,
    /// A focus moved elsewhere took the keyboard from the field.
    Blur,
    /// The window lost the keyboard while the field had it.
    WindowLost,
    /// An input method started (`true`) or stopped composing in the field.
    Composing(bool),
}

/// The field's text input, given its look by the host.
type Input<'a, Message, Theme, Renderer> = TextInput<'a, Message, Theme, Renderer>;

/// How the host gives the text input its look.
type Configure<'a, Message, Theme, Renderer> =
    Rc<dyn Fn(Input<'a, Message, Theme, Renderer>) -> Input<'a, Message, Theme, Renderer> + 'a>;

/// A field showing `value`, or `placeholder` while it is empty, whose text
/// input `configure` gives its look (id, font, style), read relative to
/// `today` to tell whether Enter writes. Everything it reports goes through
/// `on_event`.
pub fn date_field<'a, Message, Theme, Renderer>(
    placeholder: &str,
    value: &str,
    today: Date,
    configure: impl Fn(Input<'a, Message, Theme, Renderer>) -> Input<'a, Message, Theme, Renderer> + 'a,
    on_event: impl Fn(FieldEvent) -> Message + 'a,
) -> DateField<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    let on_event: Rc<dyn Fn(FieldEvent) -> Message + 'a> = Rc::new(on_event);
    let configure: Configure<'a, Message, Theme, Renderer> = Rc::new(configure);
    let live = Rc::new(RefCell::new(value.to_owned()));
    let mut field = DateField {
        input: Element::new(iced::widget::Space::new()),
        placeholder: placeholder.to_owned(),
        today,
        live,
        configure,
        on_event,
        keys: FieldKeys::default(),
    };
    field.input = field.build(value);
    field
}

pub struct DateField<'a, Message, Theme, Renderer>
where
    Theme: text_input::Catalog,
    Renderer: text::Renderer<Font = Font>,
{
    input: Element<'a, Message, Theme, Renderer>,
    placeholder: String,
    today: Date,
    /// The field's text as the events of this batch leave it.
    live: Rc<RefCell<String>>,
    configure: Configure<'a, Message, Theme, Renderer>,
    on_event: Rc<dyn Fn(FieldEvent) -> Message + 'a>,
    /// The keymap's text editing keys, handed to the text input as its own.
    keys: FieldKeys,
}

#[derive(Debug, Default)]
struct State {
    /// The input method's preedit holds text.
    composing: bool,
    /// The text input had the keyboard after the last event.
    focused: bool,
    /// A key of the field's closed the calendar, or the window lost the
    /// keyboard: the field takes no more input.
    closed: bool,
}

/// What the field does with a key press before the text input sees it:
/// `None` leaves it to the text input; `Some` takes it, publishing the
/// event it holds, if any.
pub fn route(
    key: &Key,
    modifiers: Modifiers,
    composing: bool,
    empty: bool,
) -> Option<Option<FieldEvent>> {
    let Key::Named(named) = key.as_ref() else {
        return None;
    };
    // The Escape a host navigates with, as the editor's popovers read it.
    let plain_escape = wren_editor::is_plain_escape(modifiers);
    if composing {
        return match named {
            Named::Enter => Some(None),
            Named::Escape if plain_escape => Some(Some(FieldEvent::HeldEscape)),
            Named::Escape => Some(None),
            _ => None,
        };
    }
    let plain = modifiers.is_empty();
    let event = match named {
        Named::Escape if plain_escape => FieldEvent::Escape,
        Named::Escape | Named::Tab => return Some(None),
        Named::Enter if plain => FieldEvent::Enter,
        Named::ArrowUp if plain => FieldEvent::Key(calendar::Key::Up),
        Named::ArrowDown if plain => FieldEvent::Key(calendar::Key::Down),
        Named::PageUp if plain => FieldEvent::Key(calendar::Key::PageUp),
        Named::PageDown if plain => FieldEvent::Key(calendar::Key::PageDown),
        Named::ArrowLeft if plain && empty => FieldEvent::Key(calendar::Key::Left),
        Named::ArrowRight if plain && empty => FieldEvent::Key(calendar::Key::Right),
        _ => return None,
    };
    Some(Some(event))
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

impl<'a, Message, Theme, Renderer> DateField<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    /// The text input showing `value`, which reports what it is typed into.
    fn build(&self, value: &str) -> Element<'a, Message, Theme, Renderer> {
        let live = Rc::clone(&self.live);
        let on_event = Rc::clone(&self.on_event);
        let input =
            text_input::TextInput::new(&self.placeholder, value).on_input(move |text: String| {
                live.borrow_mut().clone_from(&text);
                on_event(FieldEvent::Typed(text))
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

    /// Whether Enter writes, closing the calendar: with nothing typed, or
    /// with text that names a date, as the calendar reads it.
    fn enter_writes(&self) -> bool {
        let live = self.live.borrow();
        live.trim().is_empty() || matches!(date::parse(&live, self.today), Some(Parsed::Date(_)))
    }

    /// Empty the field, as the host empties its copy for a key that moves
    /// the selection.
    fn empty(&mut self, tree: &mut Tree) {
        if self.live.borrow().is_empty() {
            return;
        }
        self.live.borrow_mut().clear();
        self.input = self.build("");
        tree.diff(self.input.as_widget());
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for DateField<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.input)]
    }

    fn diff(&self, tree: &mut Tree) {
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
            // A field that closed in this batch takes nothing more: what is left
            // of the batch is dropped, and the host told to drop its copy of a
            // plain Escape.
            if tree.state.downcast_ref::<State>().closed {
                if let Event::Keyboard(keyboard::Event::KeyPressed {
                    key: Key::Named(Named::Escape),
                    modifiers,
                    ..
                }) = event
                {
                    if wren_editor::is_plain_escape(*modifiers) {
                        shell.publish((self.on_event)(FieldEvent::HeldEscape));
                    }
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
                shell.publish((self.on_event)(FieldEvent::Blur));
            }
            match event {
                Event::Window(window::Event::Unfocused) if before => {
                    state.closed = true;
                    state.focused = false;
                    shell.publish((self.on_event)(FieldEvent::WindowLost));
                    break 'update;
                }
                Event::InputMethod(input_method::Event::Preedit(content, _)) if before => {
                    state.composing = !content.is_empty();
                }
                Event::InputMethod(
                    input_method::Event::Commit(_)
                    | input_method::Event::Closed
                    | input_method::Event::Opened,
                ) => state.composing = false,
                Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if before => {
                    let empty = self.live.borrow().is_empty();
                    if let Some(taken) = route(key, *modifiers, state.composing, empty) {
                        match &taken {
                            Some(FieldEvent::Key(
                                calendar::Key::Up
                                | calendar::Key::Down
                                | calendar::Key::PageUp
                                | calendar::Key::PageDown,
                            )) => self.empty(&mut tree.children[0]),
                            Some(FieldEvent::Escape) => {
                                tree.state.downcast_mut::<State>().closed = true;
                            }
                            Some(FieldEvent::Enter) if self.enter_writes() => {
                                tree.state.downcast_mut::<State>().closed = true;
                            }
                            _ => {}
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
            let pressed = matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(_))
                    | Event::Touch(iced::touch::Event::FingerPressed { .. })
            );
            if before && !after {
                state.composing = false;
                if !pressed {
                    shell.publish((self.on_event)(FieldEvent::Blur));
                }
            }
            state.focused = after;
        }
        let now = tree.state.downcast_ref::<State>().composing;
        if now != was {
            shell.publish((self.on_event)(FieldEvent::Composing(now)));
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

impl<'a, Message, Theme, Renderer> From<DateField<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: text_input::Catalog + 'a,
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn from(field: DateField<'a, Message, Theme, Renderer>) -> Self {
        Element::new(field)
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::widget::operation;
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::Location;
    use iced::{Point, Theme};

    use super::*;

    const ID: &str = "date-field-test";

    /// One field, laid out once, fed whole batches of events: one widget
    /// instance for each batch, as iced hands a frame's events to the
    /// widgets of one view.
    struct Field {
        value: String,
        tree: Tree,
        node: layout::Node,
    }

    fn element(value: &str) -> Element<'static, FieldEvent, Theme, ()> {
        date_field(
            "type a date",
            value,
            Date::new(2026, 11, 19).expect("a date"),
            |input| input.id(ID).width(200),
            |event| event,
        )
        .into()
    }

    impl Field {
        fn new(value: &str) -> Self {
            let mut element = element(value);
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(
                &mut tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 100.0)),
            );
            Self {
                value: value.to_owned(),
                tree,
                node,
            }
        }

        fn operate(&mut self, operation: &mut dyn Operation) {
            let mut element = element(&self.value);
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

        /// What one widget instance publishes for `events`, one batch, and
        /// whether it captured each; then the host's view of the text, as
        /// the last `Typed` left it, is the next batch's value.
        fn batch(
            &mut self,
            events: &[Event],
            pointer: Option<Point>,
        ) -> (Vec<FieldEvent>, Vec<bool>) {
            let mut element = element(&self.value);
            element.as_widget_mut().diff(&mut self.tree);
            let mut messages = Vec::new();
            let mut captured = Vec::new();
            for event in events {
                let mut shell = Shell::new(&mut messages);
                let cursor = pointer.map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available);
                element.as_widget_mut().update(
                    &mut self.tree,
                    event,
                    Layout::new(&self.node),
                    cursor,
                    &(),
                    &mut iced::advanced::clipboard::Null,
                    &mut shell,
                    &Rectangle::new(Point::ORIGIN, Size::new(400.0, 100.0)),
                );
                captured.push(shell.event_status() == iced::event::Status::Captured);
            }
            for message in &messages {
                match message {
                    FieldEvent::Typed(value) => self.value.clone_from(value),
                    FieldEvent::Key(
                        calendar::Key::Up
                        | calendar::Key::Down
                        | calendar::Key::PageUp
                        | calendar::Key::PageDown,
                    ) => self.value.clear(),
                    _ => {}
                }
            }
            (messages, captured)
        }
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

    fn text(value: &str) -> FieldEvent {
        FieldEvent::Typed(value.into())
    }

    #[test]
    fn the_calendars_keys_are_taken_once_each_and_typing_is_the_fields() {
        let mut date = Field::new("");
        date.focus();
        let (messages, captured) = date.batch(
            &[
                named(Named::ArrowDown),
                named(Named::ArrowUp),
                named(Named::PageDown),
                named(Named::PageUp),
                named(Named::ArrowRight),
                named(Named::ArrowLeft),
                typed("f"),
                typed("r"),
                // With text in the field, Left and Right move its caret.
                named(Named::ArrowLeft),
                named(Named::ArrowRight),
                typed("i"),
                named(Named::Tab),
                named(Named::Enter),
            ],
            None,
        );
        use calendar::Key as K;
        assert_eq!(
            messages,
            [
                FieldEvent::Key(K::Down),
                FieldEvent::Key(K::Up),
                FieldEvent::Key(K::PageDown),
                FieldEvent::Key(K::PageUp),
                FieldEvent::Key(K::Right),
                FieldEvent::Key(K::Left),
                text("f"),
                text("fr"),
                text("fri"),
                FieldEvent::Enter,
            ]
        );
        assert_eq!(captured, [true; 13]);
        assert!(date.focused());
    }

    #[test]
    fn within_one_batch_the_field_and_the_calendar_keys_agree() {
        // From empty: `f` fills the field, so Left is then the field's
        // and moves its caret before the `f`.
        let mut date = Field::new("");
        date.focus();
        let (messages, _) = date.batch(&[typed("f"), named(Named::ArrowLeft), typed("r")], None);
        assert_eq!(messages, [text("f"), text("rf")]);
        // A key that moves the selection empties the field at once, so
        // what is typed after it starts afresh, and Left is the calendar's
        // again.
        let mut date = Field::new("fri");
        date.focus();
        let (messages, _) = date.batch(
            &[
                named(Named::ArrowDown),
                named(Named::ArrowLeft),
                typed("x"),
                named(Named::PageUp),
                named(Named::ArrowRight),
            ],
            None,
        );
        use calendar::Key as K;
        assert_eq!(
            messages,
            [
                FieldEvent::Key(K::Down),
                FieldEvent::Key(K::Left),
                text("x"),
                FieldEvent::Key(K::PageUp),
                FieldEvent::Key(K::Right),
            ]
        );
        // The next batch's view shows the field empty, and agrees.
        assert_eq!(date.value, "");
        let (messages, _) = date.batch(&[typed("t")], None);
        assert_eq!(messages, [text("t")]);
    }

    #[test]
    fn only_a_plain_escape_is_the_calendars_and_none_unfocuses_the_field() {
        // Shift is no matter.
        for modifiers in [Modifiers::empty(), Modifiers::SHIFT] {
            let mut date = Field::new("fri");
            date.focus();
            let (messages, captured) =
                date.batch(&[press(Key::Named(Named::Escape), modifiers, None)], None);
            assert_eq!(messages, [FieldEvent::Escape], "{modifiers:?}");
            assert_eq!(captured, [true]);
            // The text input would have given up the keyboard for it.
            assert!(date.focused());
        }
        // Ctrl, Alt or logo make it no one's, taken so that the field keeps
        // the keyboard, and nothing closes.
        let mut date = Field::new("fri");
        date.focus();
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            let before = date.value.clone();
            let (messages, captured) = date.batch(
                &[
                    press(Key::Named(Named::Escape), modifiers, None),
                    typed("x"),
                ],
                None,
            );
            // The field kept the keyboard and its text: the letter is typed.
            assert_eq!(messages, [text(&format!("{}x", before))], "{modifiers:?}");
            assert_eq!(captured, [true, true], "{modifiers:?}");
            assert!(date.focused(), "{modifiers:?}");
        }
    }

    #[test]
    fn a_key_that_closes_the_calendar_is_the_last_input_the_field_takes() {
        let cases: [(&str, Vec<Event>, Vec<FieldEvent>); 5] = [
            // Escape, Escape: one close; the second is no one's, and the
            // host told so it drops its copy.
            (
                "fri",
                vec![named(Named::Escape), named(Named::Escape)],
                vec![FieldEvent::Escape, FieldEvent::HeldEscape],
            ),
            // Escape, then a letter: never typed.
            (
                "fri",
                vec![named(Named::Escape), typed("x")],
                vec![FieldEvent::Escape],
            ),
            // An Enter that writes, then a letter, Down or a commit.
            (
                "fri",
                vec![
                    named(Named::Enter),
                    typed("x"),
                    named(Named::ArrowDown),
                    commit("y"),
                ],
                vec![FieldEvent::Enter],
            ),
            // With nothing typed Enter writes the day selected.
            (
                "",
                vec![named(Named::Enter), typed("x")],
                vec![FieldEvent::Enter],
            ),
            // An Enter that writes nothing closes nothing: the field keeps
            // the keyboard.
            (
                "blorp",
                vec![named(Named::Enter), typed("x"), named(Named::Escape)],
                vec![FieldEvent::Enter, text("blorpx"), FieldEvent::Escape],
            ),
        ];
        for (value, events, expected) in cases {
            let mut date = Field::new(value);
            date.focus();
            let (messages, captured) = date.batch(&events, None);
            assert_eq!(messages, expected, "{value:?} {events:?}");
            assert!(captured.iter().all(|captured| *captured), "{value:?}");
        }
        // An Escape the input method holds keeps the field too.
        let mut date = Field::new("");
        date.focus();
        let (messages, _) = date.batch(
            &[
                preedit("\u{3075}"),
                named(Named::Escape),
                commit("\u{3075}"),
                typed("x"),
            ],
            None,
        );
        assert_eq!(
            messages,
            [
                FieldEvent::Composing(true),
                FieldEvent::HeldEscape,
                text("\u{3075}"),
                FieldEvent::Composing(false),
                text("\u{3075}x")
            ]
        );
    }

    /// A clipboard holding `0`.
    struct Holding(String);

    impl Clipboard for Holding {
        fn read(&self, _kind: iced::advanced::clipboard::Kind) -> Option<String> {
            Some(self.0.clone())
        }

        fn write(&mut self, _kind: iced::advanced::clipboard::Kind, contents: String) {
            self.0 = contents;
        }
    }

    #[test]
    fn a_paste_works_with_the_modifier_held_since_before_the_field_existed() {
        // No `ModifiersChanged` reached the new field: only the press says
        // the command modifier is down.
        let mut date = Field::new("");
        date.focus();
        let mut element = element(&date.value);
        element.as_widget_mut().diff(&mut date.tree);
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        element.as_widget_mut().update(
            &mut date.tree,
            &press(Key::Character("v".into()), Modifiers::COMMAND, None),
            Layout::new(&date.node),
            mouse::Cursor::Unavailable,
            &(),
            &mut Holding("fri".into()),
            &mut shell,
            &Rectangle::new(Point::ORIGIN, Size::new(400.0, 100.0)),
        );
        assert_eq!(messages, [text("fri")]);
    }

    #[test]
    fn while_an_input_method_composes_no_key_is_the_calendars() {
        let mut date = Field::new("");
        date.focus();
        let (messages, captured) = date.batch(
            &[
                preedit("ふ"),
                named(Named::ArrowDown),
                named(Named::Enter),
                named(Named::Escape),
                press(Key::Named(Named::Escape), Modifiers::CTRL, None),
                commit("ふ"),
                named(Named::Enter),
            ],
            None,
        );
        // Down is the input method's (the text input does nothing with
        // it), Enter during the composition is taken and does nothing,
        // and a plain Escape is the input method's; after the commit,
        // Enter is the calendar's again (and writes nothing for `ふ`).
        // The host hears when the composition starts and ends.
        assert_eq!(
            messages,
            [
                FieldEvent::Composing(true),
                FieldEvent::HeldEscape,
                text("ふ"),
                FieldEvent::Composing(false),
                FieldEvent::Enter
            ]
        );
        assert_eq!(captured, [false, false, true, true, true, true, true]);
        // A preedit that empties again ends the composition.
        let (messages, _) = date.batch(&[preedit("x"), preedit(""), named(Named::Enter)], None);
        assert_eq!(
            messages,
            [
                FieldEvent::Composing(true),
                FieldEvent::Composing(false),
                FieldEvent::Enter
            ]
        );
    }

    #[test]
    fn after_the_window_loses_the_keyboard_the_field_takes_nothing() {
        // Preedit, the window's focus lost, the composition's commit and
        // Enter, in one batch: the field says the window went, in order,
        // and the commit and Enter are no one's.
        let mut date = Field::new("");
        date.focus();
        let (messages, captured) = date.batch(
            &[
                preedit("ふ"),
                Event::Window(window::Event::Unfocused),
                commit("fri"),
                named(Named::Enter),
                typed("x"),
            ],
            None,
        );
        // Closed mid-composition: the host was told it began, and never
        // that it ended.
        assert_eq!(
            messages,
            [FieldEvent::Composing(true), FieldEvent::WindowLost]
        );
        assert_eq!(captured, [false, false, true, true, true]);
        // With no composition, nothing but the loss.
        let mut date = Field::new("fri");
        date.focus();
        let (messages, _) = date.batch(
            &[
                Event::Window(window::Event::Unfocused),
                named(Named::Enter),
                Event::Window(window::Event::Focused),
                typed("x"),
            ],
            None,
        );
        assert_eq!(messages, [FieldEvent::WindowLost]);
    }

    #[test]
    fn it_reports_a_focus_moved_elsewhere_and_nothing_else() {
        let mut date = Field::new("");
        date.focus();
        // A press outside the field moves the toolkit's focus, which the
        // host judges: no blur.
        let outside = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let (messages, _) = date.batch(&[outside], Some(Point::new(380.0, 90.0)));
        assert!(messages.is_empty(), "{messages:?}");
        assert!(!date.focused());
        // Focus moved elsewhere by an operation is a blur, reported at the
        // next event, once.
        date.focus();
        let (messages, _) = date.batch(&[typed("a")], None);
        assert_eq!(messages, [text("a")]);
        date.unfocus();
        let moved = Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(1.0, 1.0),
        });
        let (messages, _) = date.batch(&[moved.clone(), moved], None);
        assert_eq!(messages, [FieldEvent::Blur]);
        // An unfocused field takes no key.
        let mut date = Field::new("");
        let (messages, captured) = date.batch(&[named(Named::ArrowDown), typed("x")], None);
        assert!(messages.is_empty());
        assert_eq!(captured, [false, false]);
    }

    #[test]
    fn route_leaves_modified_keys_to_the_text_input() {
        let ctrl = Modifiers::CTRL;
        for named in [
            Named::ArrowUp,
            Named::ArrowDown,
            Named::ArrowLeft,
            Named::ArrowRight,
            Named::PageUp,
            Named::Enter,
        ] {
            assert_eq!(
                route(&Key::Named(named), ctrl, false, true),
                None,
                "{named:?}"
            );
            assert_eq!(
                route(&Key::Named(named), Modifiers::SHIFT, false, true),
                None,
                "{named:?}"
            );
        }
        assert_eq!(route(&Key::Character("v".into()), ctrl, false, true), None);
        assert_eq!(
            route(&Key::Named(Named::Home), Modifiers::empty(), false, true),
            None
        );
        assert_eq!(
            route(&Key::Named(Named::Tab), Modifiers::SHIFT, false, true),
            Some(None)
        );
    }

    /// With the keymap's text editing keys, in Linux's Alt mode Alt+A,
    /// Alt+C and Alt+V select, copy and paste in the field, as Ctrl does;
    /// in Ctrl mode Alt does none of it.
    // Real iced input on Linux, where its clipboard keys are Ctrl's.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_field_takes_the_keymap_s_editing_keys() {
        use crate::field_keys::tests::{chord, linux, Kept};
        use crate::field_keys::FieldKeys;
        use wren_core::keybind::LinuxModifier;

        let run = |keys: &FieldKeys, events: &[Event]| {
            let element = |value: &str| -> Element<'static, FieldEvent, Theme, ()> {
                date_field(
                    "type a date",
                    value,
                    Date::new(2026, 11, 19).expect("a date"),
                    |input| input.id(ID).width(200),
                    |event| event,
                )
                .field_keys(keys)
                .into()
            };
            let mut value = String::from("fri");
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
            let mut kept = Kept::default();
            let mut published = Vec::new();
            for event in events {
                let mut element = element(&value);
                element.as_widget_mut().diff(&mut tree);
                let mut messages = Vec::new();
                let mut shell = Shell::new(&mut messages);
                element.as_widget_mut().update(
                    &mut tree,
                    &crate::keys::tests::without_alt_text(event),
                    Layout::new(&node),
                    mouse::Cursor::Unavailable,
                    &(),
                    &mut kept,
                    &mut shell,
                    &Rectangle::new(Point::ORIGIN, Size::new(400.0, 100.0)),
                );
                for message in &messages {
                    if let FieldEvent::Typed(text) = message {
                        value.clone_from(text);
                    }
                }
                published.extend(messages);
            }
            (published, kept.0)
        };
        let alt = Modifiers::ALT;
        let events = [
            chord("a", alt),
            chord("c", alt),
            press(Key::Named(Named::End), Modifiers::empty(), None),
            chord("v", alt),
        ];
        let (published, kept) = run(&linux(LinuxModifier::Alt), &events);
        assert_eq!(published, [FieldEvent::Typed("frifri".into())]);
        assert_eq!(kept.as_deref(), Some("fri"));
        let (published, kept) = run(&linux(LinuxModifier::Ctrl), &events);
        assert_eq!((published, kept), (Vec::new(), None));
    }
}
