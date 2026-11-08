use iced::Task;
use wren_core::model::Parent;
use wren_core::ops::Op;

use super::columns::Column;
use super::commands::Command;
use super::{App, Message};
use crate::drag::{DragEvent, RowKind};
use crate::panes::{self, ResizeEvent};

impl App {
    pub(super) fn cancel_file_drag(&mut self) {
        self.gesture_generation = self.gesture_generation.wrapping_add(1);
        self.spring_section = None;
        self.drag_preview = None;
    }

    pub(super) fn cancel_gestures(&mut self) {
        self.cancel_file_drag();
        if let Some((pane, original)) = self.resizing.take() {
            self.widths.set(pane, original);
        }
    }

    pub(super) fn settle_resize(&mut self) {
        if let Some((pane, _)) = self.resizing.take() {
            let width = self.widths.get(pane).round() as u16;
            let _ = self.process_core(Op::SetWidth {
                pane: pane.key().into(),
                width,
            });
        }
    }

    pub(super) fn drag_event(&mut self, generation: u64, event: DragEvent) -> Task<Message> {
        if generation != self.gesture_generation || self.loading.is_some() {
            return Task::none();
        }
        match event {
            DragEvent::Preview(preview) if preview.generation == generation => {
                self.drag_preview = Some(preview)
            }
            DragEvent::Spring {
                generation: tag,
                section,
            } if tag == generation => {
                if self
                    .model()
                    .and_then(|model| model.section(section))
                    .is_some()
                {
                    self.spring_section = Some(section);
                }
            }
            DragEvent::Commit {
                generation: tag,
                op,
            } if tag == generation => {
                self.drag_preview = None;
                self.spring_section = None;
                let outcome = self.process_core(op);
                if outcome.processed.disposition.committed() {
                    self.reveal_selection();
                }
                return outcome.task;
            }
            DragEvent::Activate {
                generation: tag,
                row,
            } if tag == generation => {
                return self.click_row(row).task;
            }
            DragEvent::Open {
                generation: tag,
                row,
            } if tag == generation => {
                let selected = self.click_row(row);
                if !selected.processed.disposition.committed() {
                    return selected.task;
                }
                return Task::batch([selected.task, self.column_right().task]);
            }
            DragEvent::Context {
                generation: tag,
                row,
            } if tag == generation => {
                if let Some(target) = self.item_target(row.item()) {
                    return self.dispatch(Command::ItemActions(target)).task;
                }
            }
            DragEvent::Cancelled {
                generation: tag,
                reason,
            } if tag == generation => {
                let _reason = reason;
                self.drag_preview = None;
                self.spring_section = None;
            }
            _ => {}
        }
        Task::none()
    }

    /// A single click selects the row and makes its column active, the same
    /// as reaching it with the arrow keys; the layout never moves under the pointer.
    fn click_row(&mut self, row: RowKind) -> super::commands::Gateway {
        let (column, command) = match row {
            RowKind::Section(section) => (Column::Sections, Command::SelectSection(section)),
            RowKind::Group(group) => (Column::Groups, Command::SelectParent(Parent::Group(group))),
            RowKind::Note(note) => (Column::Notes, Command::OpenNote(self.note_target(note))),
        };
        self.column = column;
        let outcome = self.dispatch(command);
        if outcome.processed.disposition.committed() {
            return outcome;
        }
        // Re-selecting the current row commits nothing but still takes the keyboard.
        let focus = self.column_focus();
        super::commands::Gateway {
            task: Task::batch([outcome.task, focus]),
            ..outcome
        }
    }

    pub(super) fn resize_event(&mut self, generation: u64, event: ResizeEvent) -> Task<Message> {
        if generation != self.gesture_generation
            || self.loading.is_some()
            || self.palette.is_some()
            || self.dialog.is_some()
        {
            return Task::none();
        }
        match event {
            ResizeEvent::Started { pane } => {
                self.drag_preview = None;
                self.spring_section = None;
                self.editor.break_group();
                self.resizing = Some((pane, self.widths.get(pane)));
                return iced::widget::operation::focus(panes::grip_id(pane));
            }
            ResizeEvent::Changed { pane, width } => {
                self.widths.set(pane, width);
            }
            ResizeEvent::Ended { pane, width }
            | ResizeEvent::Reset { pane, width }
            | ResizeEvent::Step { pane, width } => {
                self.resizing = None;
                let task = self.dispatch(Command::Width { pane, width }).task;
                return task.chain(iced::widget::operation::focus(panes::grip_id(pane)));
            }
            ResizeEvent::Cancelled { pane, width } => {
                self.widths.set(pane, width);
                self.resizing = None;
            }
            ResizeEvent::Tab { reverse } => {
                return if reverse {
                    iced::widget::operation::focus_previous()
                } else {
                    iced::widget::operation::focus_next()
                }
            }
        }
        Task::none()
    }
}
