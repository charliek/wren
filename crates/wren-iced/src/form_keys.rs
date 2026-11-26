//! The task form's keys (discovery/tasks.md §6, item 3; item 6.1): a
//! wrapper over the form's card that takes the keys the form gives a
//! meaning before the text inputs and buttons inside it see them, and moves
//! the keyboard from field to field.
//!
//! The card is a row of *slots* in the order Tab walks them. A slot is a
//! text input (the description, the due date, the owners), a row of
//! choices (the status, the priority), or a button (Save, Cancel). A text
//! slot has the keyboard by the toolkit's own focus; a choice or a button
//! has it by this wrapper's word, with no text input focused, so no key
//! types anywhere.
//!
//! - Tab and Shift+Tab move to the next and the previous slot, round the
//!   ends, at once: the keys after them in their batch of events reach the
//!   slot they moved to.
//! - Enter saves, from any slot but these: the owners field with a name
//!   typed in it accepts the name, and a button is pressed. Space presses a
//!   button too.
//! - A plain Escape (no Ctrl, Alt or logo key) cancels, except in the
//!   owners field with a name typed, where it empties the field. Escape
//!   with one of them held is no one's.
//! - On a row of choices, Left and Right step through them, and a
//!   character is offered to the row (the status menu's box characters,
//!   the priority menu's letters).
//! - In the due field Down opens the calendar; in the owners field Up and
//!   Down move through the suggestions, Tab with a name typed takes the
//!   owner offered for it before it moves on, and Backspace with nothing
//!   typed removes the last owner. How many owners are offered is the
//!   host's word as it drew the form: once the keyboard has moved, or the
//!   card published anything (an edit, an owner removed, a choice), in the
//!   batch, it is unknown, and Up or Down counts as a choice, so that Enter
//!   hands it to the host, which adds the owner it chose or none, and never
//!   saves. What the card publishes forgets the owner chosen, as the host
//!   does.
//! - Every other key, the shortcuts included, is the focused input's, or
//!   no one's.
//!
//! While an input method composes in a field, no key is the form's: Enter
//! is taken and does nothing, so a composition is never saved half made,
//! and a plain Escape is the input method's, reported as
//! [`FormKey::HeldEscape`] so the host's key handling, which hears of every
//! plain Escape, drops its copy.
//!
//! **A close is a routing boundary.** A key that saves, cancels, presses a
//! button or opens the calendar is the last input the form takes in its
//! batch: the rest was typed before the user could see the form go, and is
//! dropped, never typed and never claimed. A plain Escape among it is
//! reported as [`FormKey::HeldEscape`], so the host's key handling drops
//! its copy. The host's key handling hears of a batch's shortcuts after
//! the form has, so of those in a batch that closed the form only once
//! the form is gone: every shortcut press the form hears is counted and
//! captured ([`FormKeys::claims`]), so the host knows its copy for the
//! form's.
//!
//! While the calendar is open over the form ([`FormKeys::passive`]) the
//! wrapper takes nothing: the calendar's field has the keyboard.
//!
//! It reports [`FormKey::Composing`] whenever an input method starts or
//! stops composing in one of the form's fields, so the host knows whether
//! one was composing when the form closed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use iced::advanced::input_method;
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::operation::{self, Focusable};
use iced::advanced::widget::{tree, Id, Operation, Tree};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell, Widget};
use iced::keyboard::key::Named;
use iced::keyboard::{self, Key, Modifiers};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

/// What a slot of the form is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// A text input with no keys of its own: the description.
    Text,
    /// The due date's text input: Down opens the calendar.
    Due,
    /// The owners' text input.
    Owners,
    /// A row of choices.
    Choice,
    /// A button.
    Button,
}

/// A slot of the form: its kind, and for a text input its widget id.
#[derive(Debug, Clone)]
pub struct Slot {
    pub kind: SlotKind,
    pub id: Option<Id>,
}

/// Whether a key press is one of the form's save keys: its physical key
/// and modifiers too, which the keymap reads.
pub type SaveKeys = Rc<dyn Fn(&Key, keyboard::key::Physical, Modifiers) -> bool>;

/// What the form's keys hand the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormKey {
    /// Tab, Shift+Tab or a press moved the keyboard to this slot.
    Focus(usize),
    /// Enter: save.
    Save,
    /// A plain Escape: cancel.
    Escape,
    /// A plain Escape in the owners field with a name typed: empty it.
    Clear,
    /// A plain Escape that is no one's: the input method's while it
    /// composed, or one after the form closed in its batch.
    HeldEscape,
    /// Enter or Space on the button of this slot.
    Press(usize),
    /// Enter in the owners field with a name typed: add it as typed, or
    /// the owner chosen with Up and Down.
    Accept,
    /// Tab or Shift+Tab in the owners field with a name typed, before the
    /// keyboard moves on: add the owner offered for it, as Tab takes a
    /// suggestion in the note.
    Complete,
    /// Left (-1) or Right (1) on a row of choices.
    Step(i32),
    /// A character typed on a row of choices.
    Pick(String),
    /// Up (-1) or Down (1) in the owners field.
    Suggest(i32),
    /// Down in the due field: open the calendar.
    Calendar,
    /// Backspace in the owners field with nothing typed.
    RemoveLast,
    /// The due field saw input of its own after the calendar gave it back
    /// the keyboard: see [`FormKeys::handback`].
    HandbackOver,
    /// An input method started (`true`) or stopped composing in a field.
    Composing(bool),
}

impl FormKey {
    /// Whether the key ends the form's batch: it closes the form, or hands
    /// the keyboard to the calendar.
    fn closes(&self) -> bool {
        matches!(
            self,
            Self::Save | Self::Escape | Self::Press(_) | Self::Calendar
        )
    }
}

/// Where a key press goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    /// To the focused input, or no one.
    Pass,
    /// Taken, and nothing done.
    Drop,
    /// Taken, as this.
    Key(FormKey),
    /// Tab (`false`) or Shift+Tab (`true`).
    Tab(bool),
}

/// Where a key press goes from a slot of `kind`, with an input method
/// `composing` or not, with a name `typed` in the owners field or not, and
/// `save` when the press is one of the form's save keys (the form's own
/// key, the done toggle's).
pub fn route(
    kind: SlotKind,
    key: &Key,
    modifiers: Modifiers,
    composing: bool,
    typed: bool,
    save: bool,
) -> Routed {
    if save {
        return if composing {
            Routed::Drop
        } else {
            Routed::Key(FormKey::Save)
        };
    }
    let plain = modifiers.is_empty();
    let plain_escape = wren_editor::is_plain_escape(modifiers);
    let Key::Named(named) = key.as_ref() else {
        let unmodified = !(modifiers.control() || modifiers.alt() || modifiers.logo());
        return match key {
            Key::Character(character) if kind == SlotKind::Choice && unmodified && !composing => {
                Routed::Key(FormKey::Pick(character.to_string()))
            }
            _ => Routed::Pass,
        };
    };
    if composing {
        return match named {
            Named::Enter => Routed::Drop,
            Named::Escape if plain_escape => Routed::Key(FormKey::HeldEscape),
            Named::Escape => Routed::Drop,
            _ => Routed::Pass,
        };
    }
    let owners_typed = kind == SlotKind::Owners && typed;
    match named {
        Named::Escape if plain_escape && owners_typed => Routed::Key(FormKey::Clear),
        Named::Escape if plain_escape => Routed::Key(FormKey::Escape),
        Named::Escape => Routed::Drop,
        Named::Tab if !(modifiers.control() || modifiers.alt() || modifiers.logo()) => {
            Routed::Tab(modifiers.shift())
        }
        Named::Enter if plain => match kind {
            _ if owners_typed => Routed::Key(FormKey::Accept),
            SlotKind::Button => Routed::Key(FormKey::Press(usize::MAX)),
            _ => Routed::Key(FormKey::Save),
        },
        Named::Space if plain && kind == SlotKind::Button => {
            Routed::Key(FormKey::Press(usize::MAX))
        }
        Named::Space if plain && kind == SlotKind::Choice => Routed::Key(FormKey::Pick(" ".into())),
        Named::ArrowLeft if plain && kind == SlotKind::Choice => Routed::Key(FormKey::Step(-1)),
        Named::ArrowRight if plain && kind == SlotKind::Choice => Routed::Key(FormKey::Step(1)),
        Named::ArrowUp if plain && kind == SlotKind::Owners => Routed::Key(FormKey::Suggest(-1)),
        Named::ArrowDown if plain && kind == SlotKind::Owners => Routed::Key(FormKey::Suggest(1)),
        Named::ArrowDown if plain && kind == SlotKind::Due => Routed::Key(FormKey::Calendar),
        Named::Backspace if plain && kind == SlotKind::Owners && !typed => {
            Routed::Key(FormKey::RemoveLast)
        }
        _ => Routed::Pass,
    }
}

/// The slot after `slot` of `count`, or before it when `back`, round the
/// ends.
pub fn next_slot(slot: usize, count: usize, back: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if back {
        (slot + count - 1) % count
    } else {
        (slot + 1) % count
    }
}

/// The form's card with its keys: `content`, whose slots are `slots`, with
/// slot `focused` holding the keyboard as the host last heard, and `typed`
/// the owners field's text as the events of this batch leave it (the host
/// writes it from the field's `on_input`). Everything it reports goes
/// through `on_key`.
pub fn form_keys<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    slots: Vec<Slot>,
    focused: usize,
    typed: Rc<RefCell<String>>,
    on_key: impl Fn(FormKey) -> Message + 'a,
) -> FormKeys<'a, Message, Theme, Renderer> {
    FormKeys {
        content: content.into(),
        slots,
        slot: Cell::new(focused),
        stopped: Cell::new(false),
        passive: false,
        handback: false,
        handback_over: Cell::new(false),
        suggestions: Cell::new(Some(0)),
        chosen: Cell::new(false),
        save: None,
        claims: None,
        typed,
        on_key: Box::new(on_key),
    }
}

pub struct FormKeys<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    slots: Vec<Slot>,
    /// The slot with the keyboard, as the events of this batch leave it.
    slot: Cell<usize>,
    /// A key of this batch closed the form: it takes no more input.
    stopped: Cell<bool>,
    passive: bool,
    /// The calendar gave the due field back the keyboard: see
    /// [`FormKeys::handback`].
    handback: bool,
    /// Input of the field's own came in this batch.
    handback_over: Cell<bool>,
    /// How many owners the owners field offers, while the host's count
    /// holds: `None` once the keyboard moved, or the card published
    /// anything, in this batch.
    suggestions: Cell<Option<usize>>,
    /// One of them is chosen with Up or Down, as this batch leaves it.
    chosen: Cell<bool>,
    save: Option<SaveKeys>,
    claims: Option<&'static std::sync::atomic::AtomicUsize>,
    typed: Rc<RefCell<String>>,
    on_key: Box<dyn Fn(FormKey) -> Message + 'a>,
}

impl<'a, Message, Theme, Renderer> FormKeys<'a, Message, Theme, Renderer> {
    /// Take no key: something over the form (the calendar) has the
    /// keyboard.
    #[must_use]
    pub fn passive(mut self, passive: bool) -> Self {
        self.passive = passive;
        self
    }

    /// The owners field offers `count` owners, and the host holds an owner
    /// chosen with Up or Down when `chosen`, offered still or not: Enter and
    /// Tab then hand it to the host, never saving, and Escape forgets it,
    /// whether or not a name is typed. The count holds until the keyboard
    /// moves, or the form's content publishes anything, in the batch.
    #[must_use]
    pub fn owner_choice(mut self, count: usize, chosen: bool) -> Self {
        self.suggestions = Cell::new(Some(count));
        self.chosen = Cell::new(chosen);
        self
    }

    /// The form's save keys, which save from any slot once the form is
    /// shown, but not while an input method composes, nor while the form
    /// is [`passive`](Self::passive).
    #[must_use]
    pub fn save_keys(mut self, save: SaveKeys) -> Self {
        self.save = Some(save);
        self
    }

    /// Count each shortcut press the form takes in `claims`, and capture
    /// it: the host's key handling hears of the same press after the form,
    /// captured, and drops its copy as the form's, whether the form has
    /// closed by then or not.
    #[must_use]
    pub fn claims(mut self, claims: &'static std::sync::atomic::AtomicUsize) -> Self {
        self.claims = Some(claims);
        self
    }

    /// The calendar over the form closed and gave the due field the
    /// keyboard: an input method may still be composing for the calendar's
    /// field, and what it commits is not the due field's. Until the form
    /// sees input of its own (a preedit with text, a key, a press), it
    /// drops what an input method commits and a preedit that empties, and
    /// then reports [`FormKey::HandbackOver`] once. As the editor's
    /// handback does ([`wren_editor::EditorView::hand_back`]).
    #[must_use]
    pub fn handback(mut self, handback: bool) -> Self {
        self.handback = handback;
        self
    }
}

#[derive(Debug, Default)]
struct State {
    /// An input method's preedit holds text.
    composing: bool,
    /// The text input that had the keyboard when it began.
    composing_in: Option<Id>,
}

/// The id of the text input of the form that has the keyboard.
struct FocusedId(Option<Id>);

impl Operation for FocusedId {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if state.is_focused() {
            self.0 = id.cloned();
        }
    }
}

impl<'a, Message, Theme, Renderer> FormKeys<'a, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn focused_id(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
    ) -> Option<Id> {
        let mut focused = FocusedId(None);
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut focused);
        focused.0
    }

    /// Give slot `slot` the keyboard: its text input's focus, or for a
    /// choice or a button, no text input's.
    fn focus(&mut self, slot: usize, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer) {
        let target = self.slots.get(slot).and_then(|slot| slot.id.clone());
        let content = self.content.as_widget_mut();
        match target {
            Some(id) => content.operate(
                &mut tree.children[0],
                layout,
                renderer,
                &mut operation::focusable::focus(id),
            ),
            None => content.operate(
                &mut tree.children[0],
                layout,
                renderer,
                &mut operation::focusable::unfocus(),
            ),
        }
        self.slot.set(slot);
        self.suggestions.set(None);
    }

    /// Empty the owners field's text input at once, as the host empties its
    /// copy for a name accepted or cleared, so the keys after it in their
    /// batch start afresh: select all its text, then delete it, which it
    /// reports as typing.
    #[allow(clippy::too_many_arguments)]
    fn empty_owners(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.typed.borrow_mut().clear();
        let Some(id) = self
            .slots
            .iter()
            .find(|slot| slot.kind == SlotKind::Owners)
            .and_then(|slot| slot.id.clone())
        else {
            return;
        };
        let content = self.content.as_widget_mut();
        content.operate(
            &mut tree.children[0],
            layout,
            renderer,
            &mut operation::text_input::select_all(id),
        );
        let backspace = Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Backspace),
            modified_key: Key::Named(Named::Backspace),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Backspace),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        });
        content.update(
            &mut tree.children[0],
            &backspace,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    /// The due field's own input came: the handback is over.
    fn end_handback(&self, shell: &mut Shell<'_, Message>) {
        if self.handback && !self.handback_over.replace(true) {
            shell.publish((self.on_key)(FormKey::HandbackOver));
        }
    }

    fn publish(&self, shell: &mut Shell<'_, Message>, key: FormKey) {
        if key.closes() {
            self.stopped.set(true);
        }
        shell.publish((self.on_key)(key));
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for FormKeys<'a, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
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
        let child = self
            .content
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
        let child = layout.children().next().expect("the content's layout");
        self.content
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
        // A shortcut press while the form has the keyboard is the form's,
        // its save keys and those after it closed in its batch included:
        // whatever the form or its field does with it, the host's copy is
        // claimed.
        let claimed = !self.passive
            && match event {
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    physical_key,
                    modifiers,
                    ..
                }) => crate::keys::shortcut(key, *physical_key, *modifiers).is_some(),
                _ => false,
            };
        'update: {
            let child = layout.children().next().expect("the content's layout");
            // A form that closed in this batch takes nothing more: what is left
            // of the batch is dropped, and the host told to drop its copy of a
            // plain Escape.
            if self.stopped.get() && !matches!(event, Event::Window(_)) {
                if let Event::Keyboard(keyboard::Event::KeyPressed {
                    key: Key::Named(Named::Escape),
                    modifiers,
                    ..
                }) = event
                {
                    if wren_editor::is_plain_escape(*modifiers) {
                        shell.publish((self.on_key)(FormKey::HeldEscape));
                    }
                }
                shell.capture_event();
                break 'update;
            }
            let passive = self.passive || self.stopped.get();
            // After the calendar gave the due field the keyboard: what an input
            // method commits, and a preedit that empties, before input of the
            // field's own, was the calendar's.
            let held = self.handback && !self.handback_over.get();
            match event {
                Event::InputMethod(input_method::Event::Commit(_)) if held && !passive => {
                    shell.capture_event();
                    break 'update;
                }
                Event::InputMethod(input_method::Event::Preedit(content, _))
                    if held && !passive && content.is_empty() =>
                {
                    shell.capture_event();
                    break 'update;
                }
                Event::InputMethod(input_method::Event::Preedit(..))
                | Event::Keyboard(keyboard::Event::KeyPressed { .. })
                | Event::Mouse(mouse::Event::ButtonPressed(_))
                    if held && !passive =>
                {
                    self.end_handback(shell);
                }
                _ => {}
            }
            if passive {
                // The calendar's field has the keyboard: a composition the form
                // had is over for it.
                tree.state.downcast_mut::<State>().composing = false;
            } else {
                // A composition whose text input lost the keyboard (to a choice,
                // a button or another field) is over, whatever the input method
                // says.
                if tree.state.downcast_ref::<State>().composing {
                    let now = self.focused_id(tree, child, renderer);
                    let state = tree.state.downcast_mut::<State>();
                    if now.is_none() || now != state.composing_in {
                        state.composing = false;
                    }
                }
                let composing_in = match event {
                    Event::InputMethod(input_method::Event::Preedit(content, _))
                        if !content.is_empty() =>
                    {
                        self.focused_id(tree, child, renderer)
                    }
                    _ => None,
                };
                let state = tree.state.downcast_mut::<State>();
                match event {
                    // A composition the window lost the keyboard in has ended,
                    // whether or not the input method says so.
                    Event::Window(iced::window::Event::Unfocused) => state.composing = false,
                    Event::InputMethod(input_method::Event::Preedit(content, _)) => {
                        state.composing = !content.is_empty();
                        if state.composing {
                            state.composing_in = composing_in;
                        }
                    }
                    Event::InputMethod(
                        input_method::Event::Commit(_)
                        | input_method::Event::Closed
                        | input_method::Event::Opened,
                    ) => state.composing = false,
                    Event::Keyboard(keyboard::Event::KeyPressed {
                        key,
                        modified_key,
                        physical_key,
                        modifiers,
                        ..
                    }) => {
                        // A character is offered to a row of choices as typed,
                        // Shift and all: Shift+H picks Highest, not High.
                        let routed_key = match (key, modified_key) {
                            (Key::Character(_), Key::Character(_)) => modified_key,
                            _ => key,
                        };
                        let slot = self.slot.get();
                        let kind = self
                            .slots
                            .get(slot)
                            .map_or(SlotKind::Button, |slot| slot.kind);
                        let typed = !self.typed.borrow().trim().is_empty()
                            || (kind == SlotKind::Owners && self.chosen.get());
                        let save = self
                            .save
                            .as_ref()
                            .is_some_and(|save| save(key, *physical_key, *modifiers));
                        match route(kind, routed_key, *modifiers, state.composing, typed, save) {
                            Routed::Pass => {
                                // The text input reads the modifiers it last
                                // heard change, which a press's own say.
                                self.content.as_widget_mut().update(
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
                            Routed::Drop => {
                                shell.capture_event();
                                break 'update;
                            }
                            Routed::Tab(back) => {
                                if kind == SlotKind::Owners && typed {
                                    self.chosen.set(false);
                                    self.suggestions.set(None);
                                    self.publish(shell, FormKey::Complete);
                                    self.empty_owners(
                                        tree, child, cursor, renderer, clipboard, shell, viewport,
                                    );
                                }
                                let next = next_slot(slot, self.slots.len(), back);
                                self.focus(next, tree, child, renderer);
                                shell.publish((self.on_key)(FormKey::Focus(next)));
                                shell.capture_event();
                                break 'update;
                            }
                            Routed::Key(key) => {
                                let empties = matches!(key, FormKey::Accept | FormKey::Clear);
                                match key {
                                    // With the count unknown the host may
                                    // have chosen one: Enter hands it over.
                                    FormKey::Suggest(_) if self.suggestions.get() != Some(0) => {
                                        self.chosen.set(true);
                                    }
                                    FormKey::Accept | FormKey::Clear => {
                                        self.chosen.set(false);
                                        self.suggestions.set(None);
                                    }
                                    FormKey::RemoveLast => self.suggestions.set(None),
                                    _ => {}
                                }
                                let key = match key {
                                    FormKey::Press(_) => FormKey::Press(slot),
                                    key => key,
                                };
                                self.publish(shell, key);
                                if empties {
                                    self.empty_owners(
                                        tree, child, cursor, renderer, clipboard, shell, viewport,
                                    );
                                }
                                shell.capture_event();
                                break 'update;
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Whatever the form's content publishes (an edit of a field, an
            // input method's commit, an owner's remove button, a choice, a
            // suggestion taken) may change the owners offered, and the host
            // forgets the owner chosen for it: the count is unknown and none
            // is chosen. A key that only moves a caret publishes nothing and
            // keeps the choice.
            let mut published = Vec::new();
            let mut inner = Shell::new(&mut published);
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                &mut inner,
                viewport,
            );
            if !inner.is_empty() {
                self.chosen.set(false);
                self.suggestions.set(None);
            }
            shell.merge(inner, std::convert::identity);
            // A press that gave a text input of the form the keyboard moves
            // the slot to it.
            let pressed = matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(_))
                    | Event::Touch(iced::touch::Event::FingerPressed { .. })
            );
            if pressed && !passive {
                let focused = self.focused_id(tree, child, renderer);
                let found = focused.and_then(|id| {
                    self.slots
                        .iter()
                        .position(|slot| slot.id.as_ref() == Some(&id))
                });
                if let Some(found) = found.filter(|found| *found != self.slot.get()) {
                    // The host forgets the owner chosen once the keyboard
                    // leaves the owners field, as Tab's move does.
                    let left = self.slots.get(self.slot.get()).map(|slot| slot.kind);
                    if left == Some(SlotKind::Owners) {
                        self.chosen.set(false);
                    }
                    self.slot.set(found);
                    self.suggestions.set(None);
                    tree.state.downcast_mut::<State>().composing = false;
                    shell.publish((self.on_key)(FormKey::Focus(found)));
                }
            }
        }
        if claimed {
            if let Some(claims) = self.claims {
                shell.capture_event();
                claims.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let now = tree.state.downcast_ref::<State>().composing;
        if now != was {
            shell.publish((self.on_key)(FormKey::Composing(now)));
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
        let child = layout.children().next().expect("the content's layout");
        self.content.as_widget().mouse_interaction(
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
        let child = layout.children().next().expect("the content's layout");
        self.content.as_widget().draw(
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
        let child = layout.children().next().expect("the content's layout");
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            child,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<FormKeys<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(keys: FormKeys<'a, Message, Theme, Renderer>) -> Self {
        Element::new(keys)
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::widget::operation;
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::Location;
    use iced::widget::{button, column, text_input};
    use iced::{Point, Theme};

    use super::*;

    /// What the test form publishes: its keys, or a field's new text.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Out {
        Key(FormKey),
        Typed(&'static str, String),
        Pressed,
    }

    const IDS: [&str; 3] = ["description", "due", "owners"];

    fn slots() -> Vec<Slot> {
        let text = |kind, id: &'static str| Slot {
            kind,
            id: Some(Id::new(id)),
        };
        vec![
            text(SlotKind::Text, IDS[0]),
            Slot {
                kind: SlotKind::Choice,
                id: None,
            },
            text(SlotKind::Due, IDS[1]),
            text(SlotKind::Owners, IDS[2]),
            Slot {
                kind: SlotKind::Button,
                id: None,
            },
        ]
    }

    /// A form of three fields, a button, and the slots in between, laid
    /// out once and fed whole batches of events, one widget instance for
    /// each, as iced hands a frame's events to one view.
    struct Form {
        values: [String; 3],
        focused: usize,
        /// The calendar gave the due field the keyboard back.
        handback: bool,
        tree: Tree,
        node: layout::Node,
    }

    fn element(
        values: &[String; 3],
        focused: usize,
        handback: bool,
    ) -> Element<'static, Out, Theme, ()> {
        let typed = Rc::new(RefCell::new(values[2].clone()));
        let live = Rc::clone(&typed);
        let field = |at: usize| {
            let id = IDS[at];
            text_input("", &values[at])
                .id(id)
                .width(200)
                .on_input(move |text: String| Out::Typed(id, text))
        };
        let owners =
            text_input("", &values[2])
                .id(IDS[2])
                .width(200)
                .on_input(move |text: String| {
                    live.borrow_mut().clone_from(&text);
                    Out::Typed(IDS[2], text)
                });
        let content = column![
            field(0),
            field(1),
            owners,
            button("Save").on_press(Out::Pressed)
        ];
        // Ctrl+Enter saves.
        let save: SaveKeys = Rc::new(|key: &Key, _, modifiers: Modifiers| {
            *key == Key::Named(Named::Enter) && modifiers == Modifiers::CTRL
        });
        form_keys(content, slots(), focused, typed, Out::Key)
            .save_keys(save)
            .owner_choice(2, false)
            .handback(handback)
            .into()
    }

    impl Form {
        fn new() -> Self {
            let values = [String::new(), String::new(), String::new()];
            let mut element = element(&values, 0, false);
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(
                &mut tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 400.0)),
            );
            let mut form = Self {
                values,
                focused: 0,
                handback: false,
                tree,
                node,
            };
            form.operate(&mut operation::focusable::focus(Id::new(IDS[0])));
            form
        }

        fn operate(&mut self, operation: &mut dyn Operation) {
            let mut element = element(&self.values, self.focused, self.handback);
            element.as_widget_mut().operate(
                &mut self.tree,
                Layout::new(&self.node),
                &(),
                operation,
            );
        }

        /// The id of the field with the keyboard.
        fn focused_field(&mut self) -> Option<Id> {
            let mut focused = FocusedId(None);
            self.operate(&mut focused);
            focused.0
        }

        /// What one widget instance publishes for `events`, and whether it
        /// captured each; then the host's model as the messages leave it.
        fn batch(&mut self, events: &[Event]) -> (Vec<Out>, Vec<bool>) {
            let mut element = element(&self.values, self.focused, self.handback);
            element.as_widget_mut().diff(&mut self.tree);
            let mut messages = Vec::new();
            let mut captured = Vec::new();
            for event in events {
                let mut shell = Shell::new(&mut messages);
                element.as_widget_mut().update(
                    &mut self.tree,
                    event,
                    Layout::new(&self.node),
                    mouse::Cursor::Unavailable,
                    &(),
                    &mut iced::advanced::clipboard::Null,
                    &mut shell,
                    &Rectangle::new(Point::ORIGIN, Size::new(400.0, 400.0)),
                );
                captured.push(shell.event_status() == iced::event::Status::Captured);
            }
            for message in &messages {
                match message {
                    Out::Typed(id, text) => {
                        let at = IDS.iter().position(|each| each == id).unwrap();
                        self.values[at].clone_from(text);
                    }
                    Out::Key(FormKey::Focus(slot)) => self.focused = *slot,
                    Out::Key(FormKey::HandbackOver) => self.handback = false,
                    Out::Key(FormKey::Accept | FormKey::Complete | FormKey::Clear) => {
                        self.values[2].clear();
                    }
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

    fn typed_text(field: &'static str, text: &str) -> Out {
        Out::Typed(field, text.into())
    }

    fn key(key: FormKey) -> Out {
        Out::Key(key)
    }

    /// Shift on a row of choices is kept: the character offered is the
    /// one the key types, so Shift+H offers `H` (Highest), not `h`.
    #[test]
    fn a_shifted_character_is_offered_as_typed() {
        let mut form = Form::new();
        let shifted = |base: &str, typed: &str| {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character(base.into()),
                modified_key: Key::Character(typed.into()),
                physical_key: Physical::Code(Code::KeyH),
                location: Location::Standard,
                modifiers: Modifiers::SHIFT,
                text: Some(typed.into()),
                repeat: false,
            })
        };
        let (messages, _) = form.batch(&[
            named(Named::Tab),
            shifted("h", "H"),
            shifted("l", "L"),
            typed("h"),
            shifted("/", "?"),
        ]);
        assert_eq!(
            messages,
            [
                key(FormKey::Focus(1)),
                key(FormKey::Pick("H".into())),
                key(FormKey::Pick("L".into())),
                key(FormKey::Pick("h".into())),
                key(FormKey::Pick("?".into())),
            ]
        );
    }

    /// With claims to count in, every shortcut press the form hears is
    /// captured and counted, before and after the key that closes it, its
    /// save keys included; a key that types is neither.
    #[test]
    fn every_shortcut_the_form_hears_is_captured_and_counted() {
        let claims: &'static std::sync::atomic::AtomicUsize =
            Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
        let values = [String::new(), String::new(), String::new()];
        let live = Rc::new(RefCell::new(String::new()));
        let save: SaveKeys = Rc::new(|key: &Key, _, modifiers: Modifiers| {
            *key == Key::Named(Named::Enter) && modifiers == Modifiers::CTRL
        });
        let mut element: Element<'static, Out, Theme, ()> = form_keys(
            column![text_input("", &values[0]).id(IDS[0])],
            slots(),
            0,
            live,
            Out::Key,
        )
        .save_keys(save)
        .claims(claims)
        .into();
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(400.0, 400.0)),
        );
        let jump = press(Key::Character("p".into()), Modifiers::ALT, None);
        let mut captured = Vec::new();
        for event in [jump.clone(), typed("a"), ctrl_enter(), jump] {
            let mut messages = Vec::new();
            let mut shell = Shell::new(&mut messages);
            element.as_widget_mut().update(
                &mut tree,
                &event,
                Layout::new(&node),
                mouse::Cursor::Unavailable,
                &(),
                &mut iced::advanced::clipboard::Null,
                &mut shell,
                &Rectangle::new(Point::ORIGIN, Size::new(400.0, 400.0)),
            );
            captured.push(shell.event_status() == iced::event::Status::Captured);
        }
        assert_eq!(captured, [true, false, true, true]);
        assert_eq!(claims.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[test]
    fn tab_walks_the_slots_within_one_batch() {
        let mut form = Form::new();
        let shift_tab = press(Key::Named(Named::Tab), Modifiers::SHIFT, None);
        let (messages, captured) = form.batch(&[
            typed("a"),
            named(Named::Tab),
            // The status row: a character is offered to it, and types
            // nowhere.
            typed("x"),
            named(Named::ArrowRight),
            named(Named::Tab),
            typed("f"),
            named(Named::Tab),
            typed("s"),
            shift_tab.clone(),
            typed("r"),
        ]);
        assert_eq!(
            messages,
            [
                typed_text("description", "a"),
                key(FormKey::Focus(1)),
                key(FormKey::Pick("x".into())),
                key(FormKey::Step(1)),
                key(FormKey::Focus(2)),
                typed_text("due", "f"),
                key(FormKey::Focus(3)),
                typed_text("owners", "s"),
                // Shift+Tab with a name typed takes the owner offered.
                key(FormKey::Complete),
                // The field empties at once.
                typed_text("owners", ""),
                key(FormKey::Focus(2)),
                typed_text("due", "fr"),
            ]
        );
        assert_eq!(captured, [true; 10]);
        assert_eq!(form.focused_field(), Some(Id::new("due")));
        // Round the ends, both ways: from the button to the description.
        let (messages, _) = form.batch(&[
            named(Named::Tab),
            named(Named::Tab),
            named(Named::Tab),
            shift_tab,
        ]);
        assert_eq!(
            messages,
            [
                key(FormKey::Focus(3)),
                key(FormKey::Focus(4)),
                key(FormKey::Focus(0)),
                key(FormKey::Focus(4)),
            ]
        );
        assert_eq!(form.focused_field(), None, "a button has no field's focus");
    }

    #[test]
    fn enter_saves_and_ends_the_batch() {
        let mut form = Form::new();
        let (messages, captured) = form.batch(&[
            typed("a"),
            named(Named::Enter),
            typed("b"),
            named(Named::Escape),
            named(Named::Tab),
        ]);
        assert_eq!(
            messages,
            [
                typed_text("description", "a"),
                key(FormKey::Save),
                key(FormKey::HeldEscape)
            ]
        );
        assert_eq!(captured, [true; 5]);
        assert_eq!(form.values[0], "a", "nothing after Enter is typed");
    }

    #[test]
    fn escape_cancels_unless_a_name_is_typed_in_the_owners_field() {
        let mut form = Form::new();
        let (messages, _) = form.batch(&[named(Named::Escape), typed("a")]);
        assert_eq!(messages, [key(FormKey::Escape)]);
        // Escape with Ctrl, Alt or logo is no one's.
        let mut form = Form::new();
        let (messages, captured) =
            form.batch(&[press(Key::Named(Named::Escape), Modifiers::CTRL, None)]);
        assert!(messages.is_empty() && captured == [true]);
        // In the owners field: Enter accepts the name typed, Escape empties
        // the field, and then, with nothing typed, they save and cancel.
        let mut form = Form::new();
        form.batch(&[named(Named::Tab), named(Named::Tab), named(Named::Tab)]);
        let (messages, _) = form.batch(&[
            typed("s"),
            named(Named::Enter),
            typed("p"),
            named(Named::Escape),
            named(Named::Backspace),
            named(Named::ArrowDown),
            named(Named::ArrowUp),
            named(Named::Enter),
        ]);
        assert_eq!(
            messages,
            [
                typed_text("owners", "s"),
                key(FormKey::Accept),
                typed_text("owners", ""),
                typed_text("owners", "p"),
                key(FormKey::Clear),
                typed_text("owners", ""),
                key(FormKey::RemoveLast),
                key(FormKey::Suggest(1)),
                key(FormKey::Suggest(-1)),
                // An owner chosen: Enter adds it, nothing typed.
                key(FormKey::Accept),
                typed_text("owners", ""),
            ]
        );
        // Tab with a name typed takes the owner offered before it moves on.
        let mut form = Form::new();
        form.batch(&[named(Named::Tab), named(Named::Tab), named(Named::Tab)]);
        let (messages, _) = form.batch(&[typed("s"), named(Named::Tab)]);
        assert_eq!(
            messages,
            [
                typed_text("owners", "s"),
                key(FormKey::Complete),
                typed_text("owners", ""),
                key(FormKey::Focus(4))
            ]
        );
    }

    #[test]
    fn down_in_the_due_field_opens_the_calendar_and_ends_the_batch() {
        let mut form = Form::new();
        form.batch(&[named(Named::Tab), named(Named::Tab)]);
        let (messages, captured) = form.batch(&[
            typed("f"),
            named(Named::ArrowDown),
            typed("r"),
            named(Named::Enter),
        ]);
        assert_eq!(messages, [typed_text("due", "f"), key(FormKey::Calendar)]);
        assert_eq!(captured, [true; 4]);
    }

    #[test]
    fn a_button_is_pressed_by_enter_and_space() {
        let mut form = Form::new();
        let shift_tab = press(Key::Named(Named::Tab), Modifiers::SHIFT, None);
        form.batch(&[shift_tab]);
        let (messages, _) = form.batch(&[named(Named::Space)]);
        assert_eq!(messages, [key(FormKey::Press(4))]);
        let mut form = Form::new();
        form.batch(&[press(Key::Named(Named::Tab), Modifiers::SHIFT, None)]);
        let (messages, _) = form.batch(&[named(Named::Enter), named(Named::Space)]);
        assert_eq!(messages, [key(FormKey::Press(4))]);
    }

    #[test]
    fn while_an_input_method_composes_no_key_is_the_forms() {
        let mut form = Form::new();
        let preedit = Event::InputMethod(input_method::Event::Preedit("か".into(), None));
        let commit = Event::InputMethod(input_method::Event::Commit("家".into()));
        let (messages, captured) = form.batch(&[
            preedit,
            named(Named::Enter),
            named(Named::Escape),
            named(Named::Tab),
            commit,
            named(Named::Enter),
        ]);
        assert_eq!(
            messages,
            [
                key(FormKey::Composing(true)),
                key(FormKey::HeldEscape),
                typed_text("description", "家"),
                key(FormKey::Composing(false)),
                key(FormKey::Save),
            ]
        );
        assert_eq!(
            captured[1..4],
            [true, true, false],
            "Enter and Escape are taken"
        );
        assert_eq!(form.focused_field(), Some(Id::new("description")));
    }

    #[test]
    fn shortcuts_pass_to_the_field_and_the_host() {
        let mut form = Form::new();
        let (messages, captured) = form.batch(&[
            press(Key::Character("j".into()), Modifiers::CTRL, None),
            press(Key::Character("k".into()), Modifiers::ALT, None),
        ]);
        assert!(messages.is_empty());
        assert_eq!(captured, [false, false], "the host hears them");
    }

    #[test]
    fn a_passive_form_takes_no_key() {
        let values = [String::new(), String::new(), String::new()];
        let typed = Rc::new(RefCell::new(String::new()));
        let mut element: Element<'static, Out, Theme, ()> = form_keys(
            column![text_input("", "").id("x")],
            slots(),
            0,
            typed,
            Out::Key,
        )
        .passive(true)
        .into();
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(400.0, 400.0)),
        );
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        element.as_widget_mut().update(
            &mut tree,
            &named(Named::Enter),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &(),
            &mut iced::advanced::clipboard::Null,
            &mut shell,
            &Rectangle::new(Point::ORIGIN, Size::new(400.0, 400.0)),
        );
        assert!(messages.is_empty(), "{values:?}");
    }

    fn routed(
        kind: SlotKind,
        key: &Key,
        modifiers: Modifiers,
        composing: bool,
        typed: bool,
    ) -> Routed {
        route(kind, key, modifiers, composing, typed, false)
    }

    fn ctrl_enter() -> Event {
        press(Key::Named(Named::Enter), Modifiers::CTRL, None)
    }

    #[test]
    fn a_save_key_saves_from_any_slot_once_shown_and_not_while_composing() {
        let mut form = Form::new();
        let (messages, captured) = form.batch(&[named(Named::Tab), ctrl_enter(), typed("a")]);
        assert_eq!(messages, [key(FormKey::Focus(1)), key(FormKey::Save)]);
        assert_eq!(captured, [true; 3], "nothing after it is typed");
        // While an input method composes, it is the input method's.
        let mut form = Form::new();
        let preedit = Event::InputMethod(input_method::Event::Preedit("か".into(), None));
        let (messages, captured) = form.batch(&[preedit, ctrl_enter()]);
        assert_eq!(messages, [key(FormKey::Composing(true))]);
        assert!(captured[1], "taken, and dropped");
    }

    #[test]
    fn escape_and_enter_in_the_owners_field_empty_it_at_once() {
        for done in [named(Named::Escape), named(Named::Enter)] {
            let mut form = Form::new();
            form.batch(&[named(Named::Tab), named(Named::Tab), named(Named::Tab)]);
            let (messages, _) = form.batch(&[typed("s"), typed("a"), typed("m"), done, typed("b")]);
            assert_eq!(
                messages.last(),
                Some(&typed_text("owners", "b")),
                "{messages:?}"
            );
            assert!(messages.contains(&typed_text("owners", "")), "{messages:?}");
            assert_eq!(form.values[2], "b");
        }
    }

    #[test]
    fn an_owner_chosen_with_nothing_typed_is_taken_by_enter_and_tab() {
        for (done, taken) in [
            (named(Named::Enter), FormKey::Accept),
            (named(Named::Tab), FormKey::Complete),
        ] {
            let mut form = Form::new();
            form.batch(&[named(Named::Tab), named(Named::Tab), named(Named::Tab)]);
            let (messages, _) = form.batch(&[named(Named::ArrowDown), done]);
            assert_eq!(messages[..2], [key(FormKey::Suggest(1)), key(taken)]);
        }
        // Typing after the choice forgets it.
        let mut form = Form::new();
        form.batch(&[named(Named::Tab), named(Named::Tab), named(Named::Tab)]);
        let (messages, _) =
            form.batch(&[named(Named::ArrowDown), typed("s"), named(Named::Escape)]);
        assert_eq!(messages.last(), Some(&typed_text("owners", "")));
        assert!(messages.contains(&key(FormKey::Clear)));
    }

    #[test]
    fn a_composition_ends_when_its_field_loses_the_keyboard() {
        let mut form = Form::new();
        let preedit = Event::InputMethod(input_method::Event::Preedit("か".into(), None));
        let (messages, _) = form.batch(&[preedit]);
        assert_eq!(messages, [key(FormKey::Composing(true))]);
        // A click on a choice takes the keyboard from the field.
        form.operate(&mut operation::focusable::unfocus());
        let (messages, _) = form.batch(&[named(Named::Enter)]);
        assert_eq!(
            messages,
            [key(FormKey::Save), key(FormKey::Composing(false))]
        );
    }

    #[test]
    fn a_shortcut_after_the_form_closed_in_its_batch_is_dropped() {
        let mut form = Form::new();
        let jump = press(Key::Character("p".into()), Modifiers::ALT, None);
        let (messages, captured) = form.batch(&[named(Named::Enter), jump]);
        assert_eq!(messages, [key(FormKey::Save)]);
        assert_eq!(captured, [true, true]);
    }

    #[test]
    fn the_window_losing_the_keyboard_ends_a_composition() {
        let mut form = Form::new();
        let preedit = Event::InputMethod(input_method::Event::Preedit("か".into(), None));
        let blur = Event::Window(iced::window::Event::Unfocused);
        let (messages, _) = form.batch(&[preedit, blur, named(Named::Enter)]);
        assert_eq!(
            messages,
            [
                key(FormKey::Composing(true)),
                key(FormKey::Composing(false)),
                key(FormKey::Save)
            ]
        );
    }

    #[test]
    fn after_the_calendar_a_late_commit_is_not_the_due_fields() {
        let mut form = Form::new();
        form.batch(&[named(Named::Tab), named(Named::Tab)]);
        form.handback = true;
        let empty = Event::InputMethod(input_method::Event::Preedit(String::new(), None));
        let commit = Event::InputMethod(input_method::Event::Commit("金".into()));
        let (messages, _) = form.batch(&[empty, commit.clone()]);
        assert_eq!(messages, [], "the calendar's composition is dropped");
        assert!(form.handback);
        // The field's own input ends it, once; then a commit is the field's.
        let (messages, _) = form.batch(&[typed("f"), typed("r"), commit]);
        assert_eq!(
            messages,
            [
                key(FormKey::HandbackOver),
                typed_text("due", "f"),
                typed_text("due", "fr"),
                typed_text("due", "fr金"),
            ]
        );
        assert!(!form.handback);
    }

    #[test]
    fn route_names_each_slots_keys() {
        let plain = Modifiers::empty();
        let enter = Key::Named(Named::Enter);
        assert_eq!(
            routed(SlotKind::Owners, &enter, plain, false, true),
            Routed::Key(FormKey::Accept)
        );
        assert_eq!(
            routed(SlotKind::Owners, &enter, plain, false, false),
            Routed::Key(FormKey::Save)
        );
        assert_eq!(
            routed(SlotKind::Due, &enter, plain, true, false),
            Routed::Drop,
            "Enter while composing"
        );
        assert_eq!(
            routed(
                SlotKind::Text,
                &Key::Character("h".into()),
                plain,
                false,
                false
            ),
            Routed::Pass
        );
        assert_eq!(
            routed(
                SlotKind::Choice,
                &Key::Character("H".into()),
                Modifiers::SHIFT,
                false,
                false
            ),
            Routed::Key(FormKey::Pick("H".into()))
        );
        assert_eq!(
            routed(
                SlotKind::Choice,
                &Key::Character("k".into()),
                Modifiers::CTRL,
                false,
                false
            ),
            Routed::Pass
        );
        assert_eq!(next_slot(0, 7, true), 6);
        assert_eq!(next_slot(6, 7, false), 0);
    }

    /// A key that only moves the owners field's caret publishes no edit,
    /// so the owner chosen stays chosen, as the host keeps it: Enter adds
    /// it and does not save.
    #[test]
    fn a_caret_key_keeps_the_owner_chosen() {
        for caret in [Named::Home, Named::End, Named::ArrowLeft, Named::ArrowRight] {
            let mut form = Form::new();
            form.batch(&[named(Named::Tab), named(Named::Tab), named(Named::Tab)]);
            let (messages, _) =
                form.batch(&[named(Named::ArrowDown), named(caret), named(Named::Enter)]);
            assert_eq!(
                messages,
                [
                    key(FormKey::Suggest(1)),
                    key(FormKey::Accept),
                    typed_text("owners", ""),
                ],
                "{caret:?}"
            );
        }
        // An edit, even one that leaves the text as it was, forgets it.
        let mut form = Form::new();
        form.batch(&[named(Named::Tab), named(Named::Tab), named(Named::Tab)]);
        let (messages, _) = form.batch(&[
            named(Named::ArrowDown),
            named(Named::Delete),
            named(Named::Enter),
        ]);
        assert_eq!(
            messages,
            [
                key(FormKey::Suggest(1)),
                typed_text("owners", ""),
                key(FormKey::Save),
            ]
        );
    }

    /// A form whose owners field offered `count` owners as it was drawn
    /// with slot `focused` holding the keyboard, fed one batch of `events`.
    fn owners_batch(count: usize, focused: usize, events: &[Event]) -> Vec<Out> {
        owners_batch_at(count, false, focused, None, events)
    }

    /// [`owners_batch`] with the host holding an owner chosen when
    /// `chosen`, and the pointer over the card's child `over`: the three
    /// text inputs, then a button (an owner's remove, or a choice).
    fn owners_batch_at(
        count: usize,
        chosen: bool,
        focused: usize,
        over: Option<usize>,
        events: &[Event],
    ) -> Vec<Out> {
        let live = Rc::new(RefCell::new(String::new()));
        let typed = Rc::clone(&live);
        let content = column![
            text_input("", "").id(IDS[0]),
            text_input("", "").id(IDS[1]),
            text_input("", "").id(IDS[2]).on_input(move |text: String| {
                live.borrow_mut().clone_from(&text);
                Out::Typed(IDS[2], text)
            }),
            button("×").on_press(Out::Pressed),
        ];
        let mut element: Element<'static, Out, Theme, ()> =
            form_keys(content, slots(), focused, typed, Out::Key)
                .owner_choice(count, chosen)
                .into();
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(400.0, 400.0)),
        );
        if let Some(id) = slots()[focused].id.clone() {
            element.as_widget_mut().operate(
                &mut tree,
                Layout::new(&node),
                &(),
                &mut operation::focusable::focus(id),
            );
        }
        let cursor = over.map_or(mouse::Cursor::Unavailable, |at| {
            let card = Layout::new(&node).children().next().unwrap();
            let input = card.children().nth(at).unwrap();
            mouse::Cursor::Available(input.bounds().center())
        });
        let mut messages = Vec::new();
        for event in events {
            let mut shell = Shell::new(&mut messages);
            element.as_widget_mut().update(
                &mut tree,
                event,
                Layout::new(&node),
                cursor,
                &(),
                &mut iced::advanced::clipboard::Null,
                &mut shell,
                &Rectangle::new(Point::ORIGIN, Size::new(400.0, 400.0)),
            );
        }
        messages
    }

    /// A press that takes the keyboard out of the owners field forgets the
    /// owner chosen, as the host does: back in the field in the same
    /// batch, Tab adds nothing the user did not pick.
    #[test]
    fn a_press_that_leaves_the_owners_field_forgets_the_owner_chosen() {
        let (due, owners) = (2, 3);
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let messages = owners_batch_at(
            2,
            false,
            owners,
            Some(1),
            &[
                named(Named::ArrowDown),
                press,
                named(Named::Tab),
                named(Named::Tab),
            ],
        );
        assert_eq!(
            messages,
            [
                key(FormKey::Suggest(1)),
                key(FormKey::Focus(due)),
                key(FormKey::Focus(owners)),
                key(FormKey::Focus(4)),
            ]
        );
    }

    /// The host counts the owners offered only for the owners field with
    /// the keyboard. Moved there in the batch, the form does not know how
    /// many there are, so Down counts as a choice and Enter hands it to
    /// the host, which adds the owner it chose, or none: never a save.
    #[test]
    fn an_owner_chosen_where_the_count_is_unknown_is_never_saved_past() {
        let (due, owners) = (2, 3);
        let messages = owners_batch(
            0,
            due,
            &[
                named(Named::Tab),
                named(Named::ArrowDown),
                named(Named::Enter),
            ],
        );
        assert_eq!(
            messages,
            [
                key(FormKey::Focus(owners)),
                key(FormKey::Suggest(1)),
                key(FormKey::Accept),
                typed_text("owners", ""),
            ]
        );
        // Nor once an owner is removed, which may offer it again.
        let messages = owners_batch(
            0,
            owners,
            &[
                named(Named::Backspace),
                named(Named::ArrowDown),
                named(Named::Enter),
            ],
        );
        assert!(!messages.contains(&key(FormKey::Save)), "{messages:?}");
        // With none offered, as the host counted them, Down chooses nothing
        // and Enter saves; with no Down, Enter saves.
        let messages = owners_batch(0, owners, &[named(Named::ArrowDown), named(Named::Enter)]);
        assert_eq!(messages, [key(FormKey::Suggest(1)), key(FormKey::Save)]);
        let messages = owners_batch(0, due, &[named(Named::Tab), named(Named::Enter)]);
        assert_eq!(messages, [key(FormKey::Focus(owners)), key(FormKey::Save)]);
    }

    fn click() -> [Event; 2] {
        [
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]
    }

    /// An owner's remove button (or anything else the card publishes)
    /// may change the owners offered: none were, as the host counted
    /// them, but Down then counts as a choice and Enter never saves.
    #[test]
    fn what_the_card_publishes_leaves_the_count_unknown() {
        let owners = 3;
        let [press, release] = click();
        let messages = owners_batch_at(
            0,
            false,
            owners,
            Some(3),
            &[press, release, named(Named::ArrowDown), named(Named::Enter)],
        );
        assert_eq!(
            messages,
            [Out::Pressed, key(FormKey::Suggest(1)), key(FormKey::Accept)]
        );
    }

    /// A choice clicked forgets the owner chosen, as the host does: back
    /// in the owners field in the same batch, Tab adds nothing.
    #[test]
    fn a_choice_clicked_forgets_the_owner_chosen() {
        let owners = 3;
        let [press, release] = click();
        let messages = owners_batch_at(
            2,
            false,
            owners,
            Some(3),
            &[named(Named::ArrowDown), press, release, named(Named::Tab)],
        );
        assert_eq!(
            messages,
            [
                key(FormKey::Suggest(1)),
                Out::Pressed,
                key(FormKey::Focus(4)),
            ]
        );
    }

    /// The host holds an owner chosen that is no longer offered: Enter
    /// hands it back as Accept, and never saves.
    #[test]
    fn an_owner_chosen_no_longer_offered_is_never_saved_past() {
        let owners = 3;
        let messages = owners_batch_at(0, true, owners, None, &[named(Named::Enter)]);
        assert_eq!(messages, [key(FormKey::Accept), typed_text("owners", "")]);
    }

    /// The form's text fields wrapped as the task form wraps them
    /// ([`crate::field_keys`]): in Linux's Alt mode, Alt+A, Alt+C and
    /// Alt+V select, copy and paste in the field with the keyboard, under
    /// the form's keys, and in Ctrl mode Alt does none of it.
    // Real iced input on Linux, where its clipboard keys are Ctrl's.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_form_s_fields_take_the_keymap_s_editing_keys() {
        use crate::field_keys::tests::{chord, linux, Kept};
        use crate::field_keys::{field_keys, FieldKeys};
        use wren_core::keybind::LinuxModifier;

        let run = |keys: &FieldKeys, events: &[Event]| {
            let element = |value: &str| -> Element<'static, Out, Theme, ()> {
                let content = column![
                    field_keys(
                        text_input("", value)
                            .id(IDS[0])
                            .width(200)
                            .on_input(|text: String| Out::Typed(IDS[0], text)),
                        keys,
                    ),
                    field_keys(text_input("", "").id(IDS[1]).width(200), keys),
                    field_keys(text_input("", "").id(IDS[2]).width(200), keys),
                    button("Save").on_press(Out::Pressed)
                ];
                form_keys(
                    content,
                    slots(),
                    0,
                    Rc::new(RefCell::new(String::new())),
                    Out::Key,
                )
                .into()
            };
            let mut value = String::from("abc");
            let mut first = element(&value);
            let mut tree = Tree::new(&first);
            let node = first.as_widget_mut().layout(
                &mut tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 400.0)),
            );
            first.as_widget_mut().operate(
                &mut tree,
                Layout::new(&node),
                &(),
                &mut operation::focusable::focus(Id::new(IDS[0])),
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
                    &Rectangle::new(Point::ORIGIN, Size::new(400.0, 400.0)),
                );
                for message in &messages {
                    if let Out::Typed(_, text) = message {
                        value.clone_from(text);
                    }
                }
                published.extend(messages);
            }
            (published, kept.0)
        };
        let end = press(Key::Named(Named::End), Modifiers::empty(), None);
        let alt = Modifiers::ALT;
        let events = [chord("a", alt), chord("c", alt), end, chord("v", alt)];
        let (published, kept) = run(&linux(LinuxModifier::Alt), &events);
        assert_eq!(published, [Out::Typed(IDS[0], "abcabc".into())]);
        assert_eq!(kept.as_deref(), Some("abc"));
        let (published, kept) = run(&linux(LinuxModifier::Ctrl), &events);
        assert_eq!((published, kept), (Vec::new(), None));
    }
}
