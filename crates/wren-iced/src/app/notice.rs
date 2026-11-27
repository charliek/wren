//! A message with Undo (discovery/tasks.md §2 item 26, §10.2): one line at
//! the foot of the editor pane that says what an action just wrote, with a
//! button that takes it back. It shows for a few seconds, until the next
//! message replaces it, or until Undo or a click on it dismisses it.
//!
//! Any action can leave one ([`App::notify`]). What its Undo does is named
//! by [`Undo`]: the app's last task write, which covers every write from
//! outside the editor (`task_target.rs`), a group's exclusion, or the
//! columns' task counts. An Undo for a task write runs only while that
//! write is still the next to undo, so a message never undoes another
//! write than the one it names. Undoing a task write, from here or with
//! the view's undo key, leaves a message of its own that names what it
//! undid and the note and line it wrote, or why it wrote nothing: the
//! write may be in a note out of sight. A press anywhere on the message
//! is a press outside the description field being edited, which ends it
//! before the message's Undo or dismissal runs.
//!
//! The message exists only with tasks on, and is drawn in a layer of the
//! pane that is there only then, so with tasks off the window is as it was.

use std::time::{Duration, Instant};

use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{layout, mouse, overlay, renderer, Clipboard, Layout, Shell};
use iced::widget::{button, container, rich_text, row, span};
use iced::{Element, Fill, Rectangle, Task};
use wren_core::model::{GroupId, ItemRef};
use wren_core::ops::Op;

use super::commands::{Disposition, Gateway};
use super::task_target::WriteMark;
use super::{App, Message};
use crate::{chrome, fonts, theme};

/// How long a message shows.
const SHOWN_FOR: Duration = Duration::from_secs(6);

/// A run of a message's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Part {
    Text(String),
    /// A file's words, as written there: drawn in the editor's font.
    Code(String),
}

/// What a message's Undo takes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Undo {
    /// The app's task write that `mark` names, while it is the next to
    /// undo ([`App::undo_task_write`]).
    TaskWrite(WriteMark),
    /// A group's exclusion from task views, set back to `excluded`.
    Excluded { group: GroupId, excluded: bool },
    /// The columns' task counts, shown again or hidden again.
    Counts(bool),
}

/// A message, as an action leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Notice {
    pub parts: Vec<Part>,
    pub undo: Option<Undo>,
}

/// A message as it shows: numbered, so a click on one already replaced
/// does nothing, and with the time it goes.
#[derive(Debug, Clone)]
pub(super) struct Shown {
    pub serial: u64,
    pub notice: Notice,
    until: Instant,
}

/// Input from the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Its Undo, on the message `serial`.
    Undo(u64),
    /// A click on the message elsewhere.
    Dismiss(u64),
    /// A mouse press anywhere on the message `serial`, before its Undo or
    /// its dismissal: a press outside a row's description field, which
    /// the message takes before the field could hear of it.
    Pressed(u64),
}

impl Notice {
    pub(super) fn new(parts: Vec<Part>) -> Self {
        Self { parts, undo: None }
    }

    pub(super) fn with_undo(mut self, undo: Undo) -> Self {
        self.undo = Some(undo);
        self
    }

    /// The message's words as one line, the code runs unmarked.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn text(&self) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Text(text) | Part::Code(text) => text.as_str(),
            })
            .collect()
    }

    /// After the counts were shown or hidden: saved to config.conf when
    /// `written`, else for this session only.
    pub(super) fn counts(shown: bool, written: bool) -> Self {
        let value = if shown { "on" } else { "off" };
        let done = if shown { "shown" } else { "hidden" };
        let parts = if written {
            vec![
                Part::Text(format!("Task counts {done} · saved ")),
                Part::Code(format!("task-counts = {value}")),
                Part::Text(" to config.conf".into()),
            ]
        } else {
            vec![
                Part::Text(format!("Task counts {done} for this session · ")),
                Part::Code(format!("task-counts = {value}")),
                Part::Text(" was not saved to config.conf".into()),
            ]
        };
        Self::new(parts).with_undo(Undo::Counts(!shown))
    }
}

impl App {
    /// Show `notice` in place of any message showing.
    pub(super) fn notify(&mut self, notice: Notice) {
        self.notice_serial += 1;
        self.notice = Some(Shown {
            serial: self.notice_serial,
            notice,
            until: Instant::now() + SHOWN_FOR,
        });
    }

    /// The message showing, if any.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn notice(&self) -> Option<&Shown> {
        self.notice.as_ref()
    }

    /// Let a message go once its time is up.
    pub(super) fn expire_notice(&mut self, now: Instant) {
        if self.notice.as_ref().is_some_and(|shown| now >= shown.until) {
            self.notice = None;
        }
    }

    pub(super) fn notice_event(&mut self, event: Event) -> Task<Message> {
        match event {
            // A press outside the description field ends its edit, writing
            // nothing, as a press anywhere else in the window does.
            Event::Pressed(_) => {
                if self.cancel_describe() {
                    return self.task_view_focus();
                }
                Task::none()
            }
            Event::Undo(serial) => self.undo_notice(serial).task,
            Event::Dismiss(serial) => {
                if self
                    .notice
                    .as_ref()
                    .is_some_and(|shown| shown.serial == serial)
                {
                    self.notice = None;
                }
                Task::none()
            }
        }
    }

    /// Run the Undo of the message `serial`, if it is the one showing and
    /// has one. The message goes either way.
    pub(super) fn undo_notice(&mut self, serial: u64) -> Gateway {
        let undo = match &self.notice {
            Some(shown) if shown.serial == serial => shown.notice.undo,
            _ => return self.blocked("That message is gone"),
        };
        self.notice = None;
        let Some(undo) = undo else {
            return self.blocked("That message has nothing to undo");
        };
        if self.dialog.is_some() || self.task_form.is_some() || self.calendar.is_some() {
            return self.blocked("Close what is open first");
        }
        match undo {
            Undo::TaskWrite(mark) => {
                if self.task_writes.mark() != Some(mark) {
                    return self.blocked("That change can no longer be undone here");
                }
                self.undo_and_report()
            }
            Undo::Excluded { group, excluded } => self.process_core(Op::SetTasksExcluded {
                target: ItemRef::Group(group),
                excluded,
            }),
            Undo::Counts(shown) => {
                self.preferences.settings.task_counts = shown;
                let warning = self.save_setting("task-counts", if shown { "on" } else { "off" });
                self.finish_action(
                    if warning.is_some() {
                        Disposition::CommittedWarning
                    } else {
                        Disposition::Completed
                    },
                    warning,
                    Task::none(),
                )
            }
        }
    }

    /// The message, centred at the foot of the pane over what it shows.
    pub(super) fn notice_view(&self) -> Option<Element<'_, Message>> {
        let shown = self.notice.as_ref()?;
        let colors = self.definition.colors;
        let ink = theme::color(colors.editor);
        let spans = shown.notice.parts.iter().map(|part| match part {
            Part::Text(text) => span(text.clone()).font(fonts::UI).size(12.5),
            Part::Code(text) => span(text.clone()).font(fonts::EDITOR).size(12.0),
        });
        let words =
            rich_text::<(), Message, iced::Theme, iced::Renderer>(spans.collect::<Vec<_>>())
                .color(ink)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph);
        let undo = shown.notice.undo.map(|_| {
            chrome::label("Undo", 12, colors.editor)
                .font(fonts::SEMIBOLD)
                .wrapping(iced::widget::text::Wrapping::None)
                .into()
        });
        Some(frame(shown.serial, words.into(), undo, self.definition))
    }
}

/// A message's frame around `words` and the face of its Undo, for the
/// message `serial`. Its widgets' state is the serial's (iced's keyed
/// column keeps a child's state when only its key changes, so the frame
/// keys itself), so a press on one message is never the press of the one
/// that replaces it: the release that ends it there finds a button no
/// press began, which does nothing. Every press and release over the
/// message is its own, so none reaches what it covers.
pub(super) fn frame<'a, R>(
    serial: u64,
    words: Element<'a, Message, iced::Theme, R>,
    undo: Option<Element<'a, Message, iced::Theme, R>>,
    definition: wren_core::theme::Theme,
) -> Element<'a, Message, iced::Theme, R>
where
    R: iced::advanced::text::Renderer + 'a,
{
    let colors = definition.colors;
    let mut line = row![words].spacing(10).align_y(iced::Center);
    if let Some(face) = undo {
        line = line.push(
            button(face)
                .padding([2, 8])
                .on_press(Message::Notice(Event::Undo(serial)))
                .style(move |_, status| undo_button(definition, status)),
        );
    }
    let message = button(
        container(line)
            .id("task-notice")
            .padding(iced::Padding {
                top: 7.0,
                right: 8.0,
                bottom: 7.0,
                left: 12.0,
            })
            .style(move |_| chrome::surface(definition, colors.text, 7.0, false)),
    )
    .padding(0)
    .on_press(Message::Notice(Event::Dismiss(serial)))
    .style(|_, _| button::Style::default());
    container(Owned {
        serial,
        content: message.into(),
    })
    .width(Fill)
    .center_x(Fill)
    .padding([0, 16])
    .into()
}

#[cfg(test)]
pub(super) use frame as frame_for_tests;

/// The message `serial`'s content: its state starts afresh when another
/// message takes its place, and it takes every mouse press and release
/// over it that nothing inside it took.
struct Owned<'a, R> {
    serial: u64,
    content: Element<'a, Message, iced::Theme, R>,
}

impl<'a, R: iced::advanced::Renderer + 'a> From<Owned<'a, R>>
    for Element<'a, Message, iced::Theme, R>
{
    fn from(owned: Owned<'a, R>) -> Self {
        Element::new(owned)
    }
}

impl<R: iced::advanced::Renderer> iced::advanced::Widget<Message, iced::Theme, R> for Owned<'_, R> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<u64>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(self.serial)
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        let serial = tree.state.downcast_mut::<u64>();
        if *serial == self.serial {
            tree.diff_children(std::slice::from_ref(&self.content));
        } else {
            *serial = self.serial;
            tree.children = vec![Tree::new(&self.content)];
        }
    }

    fn size(&self) -> iced::Size<iced::Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &R, limits: &layout::Limits) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &R,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let over = cursor.is_over(layout.bounds());
        if over && matches!(event, iced::Event::Mouse(mouse::Event::ButtonPressed(_))) {
            shell.publish(Message::Notice(Event::Pressed(self.serial)));
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let button = matches!(
            event,
            iced::Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::ButtonReleased(_))
        );
        if button && !shell.is_event_captured() && over {
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &R,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut R,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &R,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, R>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

/// The message's Undo: its text and a thin border in the pane's colour,
/// on the message's own.
fn undo_button(definition: wren_core::theme::Theme, status: button::Status) -> button::Style {
    let colors = definition.colors;
    let mut edge = theme::color(colors.editor);
    edge.a = if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        0.8
    } else {
        0.45
    };
    button::Style {
        background: None,
        text_color: theme::color(colors.editor),
        border: iced::Border {
            color: edge,
            width: 1.0,
            radius: 5.0.into(),
        },
        ..button::Style::default()
    }
}
