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
//! write than the one it names. Undoing leaves no message of its own: what
//! it put back shows where the action showed.
//!
//! The message exists only with tasks on, and is drawn in a layer of the
//! pane that is there only then, so with tasks off the window is as it was.

use std::time::{Duration, Instant};

use iced::widget::{button, container, rich_text, row, span};
use iced::{Element, Fill, Task};
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

    /// After the counts were shown or hidden and the choice saved.
    pub(super) fn counts(shown: bool) -> Self {
        let value = if shown { "on" } else { "off" };
        Self::new(vec![
            Part::Text(format!(
                "Task counts {} · saved ",
                if shown { "shown" } else { "hidden" }
            )),
            Part::Code(format!("task-counts = {value}")),
            Part::Text(" to config.conf".into()),
        ])
        .with_undo(Undo::Counts(!shown))
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
                self.undo_task_write().gateway
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
        let definition = self.definition;
        let colors = definition.colors;
        let ink = theme::color(colors.editor);
        let spans = shown.notice.parts.iter().map(|part| match part {
            Part::Text(text) => span(text.clone()).font(fonts::UI).size(12.5),
            Part::Code(text) => span(text.clone()).font(fonts::EDITOR).size(12.0),
        });
        let words =
            rich_text::<(), Message, iced::Theme, iced::Renderer>(spans.collect::<Vec<_>>())
                .color(ink)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph);
        let serial = shown.serial;
        let mut line = row![words].spacing(10).align_y(iced::Center);
        if shown.notice.undo.is_some() {
            line = line.push(
                button(
                    chrome::label("Undo", 12, colors.editor)
                        .font(fonts::SEMIBOLD)
                        .wrapping(iced::widget::text::Wrapping::None),
                )
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
        Some(
            container(message)
                .width(Fill)
                .center_x(Fill)
                .padding([0, 16])
                .into(),
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
