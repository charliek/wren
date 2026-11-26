//! The task view's keys (item 10.3): a wrapper over the view that, while
//! the view has the keyboard, hands the host every key press before the
//! widgets inside see it, and a probe that tells the host where a widget
//! was drawn.
//!
//! The view has no text field, so no key would otherwise be any widget's.
//! The wrapper reports each press as a [`ListKey`] and captures it, except
//! Escape and the shortcuts (anything with Ctrl, Alt or the logo key, and
//! the function keys), which still reach the app's own key handling: it
//! reports those only so the host can drop its copy of one that comes
//! after its batch was stopped.
//!
//! **A batch boundary.** Each report says whether it is the first the
//! wrapper made in its batch of events, and whether a mouse press came
//! before it in the batch. A key the host reads as opening something (a
//! menu, the note) stops the rest of its batch, which was typed before the
//! user could see what opened, so the host drops it (drop, don't forward);
//! and a key after a press in its batch is dropped too, since the press
//! may have opened something the key could not see.

use std::cell::{Cell, RefCell};

use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{layout, mouse, overlay, renderer, Clipboard, Layout, Shell, Widget};
use iced::keyboard::{self, key::Physical, Key, Modifiers};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

/// A key press, as the host reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Press {
    pub key: Key,
    pub physical: Physical,
    pub modifiers: Modifiers,
    pub text: Option<String>,
}

impl Press {
    /// A shortcut, which the app's key handling reads too: Ctrl, Alt or
    /// the logo key held, or a function key.
    pub fn is_shortcut(&self) -> bool {
        crate::keys::shortcut(&self.key, self.physical, self.modifiers).is_some()
    }

    /// Escape with no Ctrl, Alt or logo key.
    pub fn is_plain_escape(&self) -> bool {
        self.key == Key::Named(keyboard::key::Named::Escape)
            && wren_editor::is_plain_escape(self.modifiers)
    }
}

/// What the wrapper reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListKey {
    pub press: Press,
    /// The first report of the wrapper's batch of events.
    pub first: bool,
    /// A mouse press came before it in its batch.
    pub after_press: bool,
}

/// `content` with the view's keys: while `active`, every key press is
/// reported through `on_key`.
pub fn list_keys<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    active: bool,
    on_key: impl Fn(ListKey) -> Message + 'a,
) -> ListKeys<'a, Message, Theme, Renderer> {
    ListKeys {
        content: content.into(),
        active,
        reported: Cell::new(false),
        pressed: Cell::new(false),
        on_key: Box::new(on_key),
    }
}

pub struct ListKeys<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    active: bool,
    /// A key was reported in this batch.
    reported: Cell<bool>,
    /// A mouse press came in this batch.
    pressed: Cell<bool>,
    on_key: Box<dyn Fn(ListKey) -> Message + 'a>,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for ListKeys<'_, Message, Theme, Renderer>
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
            true,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                text,
                ..
            }),
        ) = (self.active, event)
        {
            // AltGr's text is no one's here: the view types nothing.
            if !wren_editor::is_alt_graph(key, *physical_key, *modifiers, text.as_deref()) {
                let press = Press {
                    key: key.clone(),
                    physical: *physical_key,
                    modifiers: *modifiers,
                    text: text.as_ref().map(ToString::to_string),
                };
                let passes = press.is_shortcut() || press.is_plain_escape();
                let first = !self.reported.replace(true);
                shell.publish((self.on_key)(ListKey {
                    press,
                    first,
                    after_press: self.pressed.get(),
                }));
                if !passes {
                    shell.capture_event();
                    return;
                }
            }
        }
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event {
            self.pressed.set(true);
        }
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

impl<'a, Message, Theme, Renderer> From<ListKeys<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(keys: ListKeys<'a, Message, Theme, Renderer>) -> Self {
        Element::new(keys)
    }
}

/// Where widgets were last drawn, by the host's name for each.
pub type Drawn<K> = RefCell<Vec<(K, Rectangle)>>;

/// `content`, which records in `drawn`, under `name`, the window
/// rectangle it was last drawn at.
pub fn probe<'a, K: Copy + PartialEq + 'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    drawn: &'a Drawn<K>,
    name: K,
) -> Probe<'a, K, Message, Theme, Renderer> {
    Probe {
        content: content.into(),
        drawn,
        name,
    }
}

pub struct Probe<'a, K, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    drawn: &'a Drawn<K>,
    name: K,
}

impl<K: Copy + PartialEq, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Probe<'_, K, Message, Theme, Renderer>
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
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
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
        let bounds = layout.bounds();
        {
            let mut drawn = self.drawn.borrow_mut();
            match drawn.iter_mut().find(|(name, _)| *name == self.name) {
                Some((_, at)) => *at = bounds,
                None => drawn.push((self.name, bounds)),
            }
        }
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

impl<'a, K: Copy + PartialEq + 'a, Message, Theme, Renderer>
    From<Probe<'a, K, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(probe: Probe<'a, K, Message, Theme, Renderer>) -> Self {
        Element::new(probe)
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::layout;
    use iced::keyboard::key::{Code, Named};
    use iced::keyboard::Location;
    use iced::widget::{button, column};
    use iced::{Point, Theme};

    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum Out {
        Key(ListKey),
        Clicked,
    }

    fn key(key: Key, code: Code, modifiers: Modifiers, text: Option<&str>) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        })
    }

    fn letter(c: &str, code: Code) -> Event {
        key(Key::Character(c.into()), code, Modifiers::empty(), Some(c))
    }

    /// One widget instance fed `events` as one batch, with the cursor at
    /// `cursor`: what it publishes, and whether it captured each event.
    fn batch(active: bool, cursor: Point, events: &[Event]) -> (Vec<Out>, Vec<bool>) {
        let mut element: Element<'static, Out, Theme, ()> = list_keys(
            column![button("press").on_press(Out::Clicked).width(100).height(20)],
            active,
            Out::Key,
        )
        .into();
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(400.0, 400.0)),
        );
        let mut messages = Vec::new();
        let mut captured = Vec::new();
        for event in events {
            let mut shell = Shell::new(&mut messages);
            element.as_widget_mut().update(
                &mut tree,
                event,
                Layout::new(&node),
                mouse::Cursor::Available(cursor),
                &(),
                &mut iced::advanced::clipboard::Null,
                &mut shell,
                &Rectangle::new(Point::ORIGIN, Size::new(400.0, 400.0)),
            );
            captured.push(shell.event_status() == iced::event::Status::Captured);
        }
        (messages, captured)
    }

    fn reported(messages: &[Out]) -> Vec<(String, bool, bool)> {
        messages
            .iter()
            .filter_map(|message| match message {
                Out::Key(ListKey {
                    press,
                    first,
                    after_press,
                }) => Some((format!("{:?}", press.key), *first, *after_press)),
                Out::Clicked => None,
            })
            .collect()
    }

    #[test]
    fn keys_are_reported_and_taken_but_escape_and_shortcuts_pass() {
        let escape = key(
            Key::Named(Named::Escape),
            Code::Escape,
            Modifiers::empty(),
            None,
        );
        let undo = key(
            Key::Character("z".into()),
            Code::KeyZ,
            Modifiers::CTRL,
            None,
        );
        let down = key(
            Key::Named(Named::ArrowDown),
            Code::ArrowDown,
            Modifiers::empty(),
            None,
        );
        let (messages, captured) = batch(
            true,
            Point::new(300.0, 300.0),
            &[letter("j", Code::KeyJ), down, escape, undo],
        );
        let reported = reported(&messages);
        assert_eq!(reported.len(), 4);
        assert_eq!(
            reported
                .iter()
                .map(|(_, first, _)| *first)
                .collect::<Vec<_>>(),
            [true, false, false, false]
        );
        assert_eq!(captured, [true, true, false, false]);
    }

    #[test]
    fn inactive_it_reports_nothing() {
        let (messages, captured) =
            batch(false, Point::new(300.0, 300.0), &[letter("j", Code::KeyJ)]);
        assert!(messages.is_empty());
        assert_eq!(captured, [false]);
    }

    #[test]
    fn a_key_after_a_press_in_its_batch_says_so() {
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let (messages, _) = batch(
            true,
            Point::new(10.0, 10.0),
            &[
                letter("k", Code::KeyK),
                press.clone(),
                release,
                letter("j", Code::KeyJ),
            ],
        );
        assert!(
            messages.contains(&Out::Clicked),
            "the button inside hears the press"
        );
        let keys = reported(&messages);
        assert!(keys[0].1);
        assert_eq!((keys[1].1, keys[1].2), (false, true));
        // A press anywhere counts: it may have opened something.
        let (messages, _) = batch(
            true,
            Point::new(300.0, 300.0),
            &[press, letter("j", Code::KeyJ)],
        );
        assert_eq!(
            reported(&messages),
            [("Character(\"j\")".to_owned(), true, true)]
        );
    }
}
