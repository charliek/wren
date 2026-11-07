use iced::Task;
use wren_core::model::Parent;
use wren_core::ops::Op;

use super::commands::Command;
use super::palette::SearchTarget;
use super::{App, Message};
use crate::drag::{DragEvent, RowKind};
use crate::panes::{self, Layout, ResizeEvent};

impl App {
    pub(super) fn cancel_file_drag(&mut self) {
        self.gesture_generation = self.gesture_generation.wrapping_add(1);
        self.spring_section = None;
        self.drag_preview = None;
    }

    pub(super) fn cancel_gestures(&mut self) {
        self.cancel_file_drag();
        if let Some((layout, pane, original)) = self.resizing.take() {
            self.widths.set(layout, pane, original);
        }
    }

    pub(super) fn settle_resize(&mut self) {
        if let Some((layout, pane, _)) = self.resizing.take() {
            let width = self.widths.get(layout, pane).round() as u16;
            let _ = self.process_core(Op::SetWidth {
                layout: layout.key().into(),
                pane: pane.key().into(),
                width,
            });
        }
    }

    pub(super) fn drag_event(
        &mut self,
        generation: u64,
        layout: Layout,
        event: DragEvent,
    ) -> Task<Message> {
        if generation != self.gesture_generation || layout != self.layout || self.loading.is_some()
        {
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
                let command = match row {
                    RowKind::Section(section) if self.layout == Layout::Outline => {
                        Command::ToggleSection(section)
                    }
                    RowKind::Section(section) => Command::SelectSection(section),
                    RowKind::NoGroup(section) => Command::SelectParent(Parent::Section(section)),
                    RowKind::Group(group) => Command::SelectParent(Parent::Group(group)),
                    RowKind::Note(note) => {
                        let hit = self.note_hits().into_iter().find(|hit| hit.note == note);
                        Command::OpenNote(SearchTarget {
                            generation: self.workspace_generation(),
                            index_revision: self
                                .session
                                .as_ref()
                                .map_or(0, |session| session.index().revision()),
                            note,
                            query: self.filters.notes.clone(),
                            content_match: hit.and_then(|hit| hit.content_match),
                        })
                    }
                };
                return self.dispatch(command).task;
            }
            DragEvent::Rename {
                generation: tag,
                row,
            } if tag == generation => {
                if let Some(item) = row.item() {
                    return self.dispatch(Command::Rename(item)).task;
                }
            }
            DragEvent::Context {
                generation: tag,
                row,
            } if tag == generation => {
                if let Some(target) = row.item().and_then(|item| self.item_target(item)) {
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

    pub(super) fn resize_event(&mut self, generation: u64, event: ResizeEvent) -> Task<Message> {
        if generation != self.gesture_generation
            || self.loading.is_some()
            || self.palette.is_some()
            || self.dialog.is_some()
        {
            return Task::none();
        }
        match event {
            ResizeEvent::Started { layout, pane } if layout == self.layout => {
                self.drag_preview = None;
                self.spring_section = None;
                self.editor.break_group();
                self.resizing = Some((layout, pane, self.widths.get(layout, pane)));
                return iced::widget::operation::focus(panes::grip_id(layout, pane));
            }
            ResizeEvent::Changed {
                layout,
                pane,
                width,
            } if layout == self.layout => {
                self.widths.set(layout, pane, width);
            }
            ResizeEvent::Ended {
                layout,
                pane,
                width,
            }
            | ResizeEvent::Reset {
                layout,
                pane,
                width,
            }
            | ResizeEvent::Step {
                layout,
                pane,
                width,
            } if layout == self.layout => {
                self.resizing = None;
                let task = self
                    .dispatch(Command::Width {
                        layout,
                        pane,
                        width,
                    })
                    .task;
                return task.chain(iced::widget::operation::focus(panes::grip_id(layout, pane)));
            }
            ResizeEvent::Cancelled {
                layout,
                pane,
                width,
            } if layout == self.layout => {
                self.widths.set(layout, pane, width);
                self.resizing = None;
            }
            ResizeEvent::Tab { reverse } => {
                return if reverse {
                    iced::widget::operation::focus_previous()
                } else {
                    iced::widget::operation::focus_next()
                }
            }
            _ => {}
        }
        Task::none()
    }
}
