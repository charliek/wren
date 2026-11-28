//! Opening a row's note at its task's line.

use iced::Task;

use crate::app::columns::Column;
use crate::app::palette::SearchTarget;
use crate::app::{App, Message};

/// What opening a row's note says when its task is no longer there.
pub(in crate::app) const STALE_OPEN: &str =
    "This task changed since the list was built; the note is open";

impl App {
    /// Open the note of the entry `entry` at its task's line, and give the
    /// editor the keyboard with the caret there.
    pub(in crate::app) fn open_task_at(&mut self, entry: usize) -> Task<Message> {
        let Some(record) = self.task_list.listing.record(entry) else {
            return Task::none();
        };
        let task = crate::app::task_target::RowTarget::of(record);
        self.open_task(&task)
    }

    /// Open the note of `task` at the line a write would find it on, and
    /// give the editor the keyboard with the caret there.
    pub(in crate::app) fn open_task(
        &mut self,
        record: &crate::app::task_target::RowTarget,
    ) -> Task<Message> {
        let target = SearchTarget {
            generation: self.workspace_generation(),
            index_revision: self
                .session
                .as_ref()
                .map_or(0, |session| session.index().revision()),
            note: record.note,
            query: String::new(),
            content_match: None,
        };
        // The note already open under the view records no visit: it shows
        // after the view as well, so Back from it is the view.
        let open = (self.session.as_ref())
            .and_then(|session| session.document())
            .map(|document| document.note);
        let under = self
            .task_views
            .shown_id()
            .filter(|_| open == Some(record.note));
        let outcome = self.activate_note(target);
        if let Some(stop) = under
            .filter(|_| outcome.processed.disposition.committed() && self.task_view().is_none())
        {
            self.task_views.pass(stop);
        }
        if !outcome.processed.disposition.committed()
            || self.selected_note != Some(record.note)
            || self.task_view().is_some()
            || self.surface() != crate::app::board::Surface::Text
        {
            return outcome.task;
        }
        // The task where a write would find it: on the line the record
        // names while it is still that task, else the one task line of the
        // same text (a twin in a fence is no task). With none, or two, the
        // note opens where it was, and says why.
        let text = self.editor.text();
        let Some(line) = wren_core::task_write::locate(&text, record.line, &record.expect) else {
            self.notify(crate::app::notice::Notice::new(vec![
                crate::app::notice::Part::Text(STALE_OPEN.into()),
            ]));
            self.column = Column::Editor;
            return Task::batch([outcome.task, self.column_focus()]);
        };
        self.editor
            .scroll_to_line(line, wren_editor::Placement::NearTop);
        self.editor.move_cursor(iced::widget::text_editor::Cursor {
            position: iced::widget::text_editor::Position { line, column: 0 },
            selection: None,
        });
        self.column = Column::Editor;
        Task::batch([outcome.task, self.column_focus()])
    }
}
