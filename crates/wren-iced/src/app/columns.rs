//! The active column: which of Sections, Groups, Notes, or the editor has the
//! keyboard, and moving the selection within and between columns.

use iced::Task;
use wren_core::model::{GroupId, NoteId, Parent, SectionId};

use super::commands::{Command, Disposition, Gateway};
use super::navigation::matches_name;
use super::palette::SearchTarget;
use super::{App, Message};
use crate::panes::{Pane, PANES};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Column {
    Sections,
    Groups,
    Notes,
    #[default]
    Editor,
}

impl Column {
    #[cfg(feature = "smoke-test")]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Sections => "sections",
            Self::Groups => "groups",
            Self::Notes => "notes",
            Self::Editor => "editor",
        }
    }

    pub const fn is_browser(self) -> bool {
        !matches!(self, Self::Editor)
    }

    const fn index(self) -> usize {
        match self {
            Self::Sections => 0,
            Self::Groups => 1,
            Self::Notes => 2,
            Self::Editor => 3,
        }
    }

    /// How many browser panes, from the left, are folded while this column
    /// has been reached from the keyboard. Notes never folds.
    const fn fold(self) -> usize {
        let index = self.index();
        if index < 2 {
            index
        } else {
            2
        }
    }

    const fn of_pane(pane: Pane) -> Self {
        match pane {
            Pane::Sections => Self::Sections,
            Pane::Groups => Self::Groups,
            Pane::Notes => Self::Notes,
        }
    }
}

impl App {
    /// Section rows in display order, after the Sections filter.
    pub(super) fn visible_sections(&self) -> Vec<SectionId> {
        self.model().map_or_else(Vec::new, |model| {
            model
                .sections
                .iter()
                .filter(|section| matches_name(&self.filters.sections, &section.name))
                .map(|section| section.id)
                .collect()
        })
    }

    /// Group rows of the browsed section in display order, after the Groups filter.
    pub(super) fn visible_groups(&self) -> Vec<GroupId> {
        let Some(section) = self.browser_section() else {
            return Vec::new();
        };
        self.model().map_or_else(Vec::new, |model| {
            model
                .groups
                .iter()
                .filter(|group| {
                    group.section == section && matches_name(&self.filters.groups, &group.name)
                })
                .map(|group| group.id)
                .collect()
        })
    }

    pub(super) fn note_target(&self, note: NoteId) -> SearchTarget {
        let hit = self.note_hits().into_iter().find(|hit| hit.note == note);
        SearchTarget {
            generation: self.workspace_generation(),
            index_revision: self
                .session
                .as_ref()
                .map_or(0, |session| session.index().revision()),
            note,
            query: self.filters.notes.clone(),
            content_match: hit.and_then(|hit| hit.content_match),
        }
    }

    /// The keyboard follows the active column: the editor takes focus only
    /// when it is the active column, so arrows reach the browser otherwise.
    pub(super) fn column_focus(&self) -> Task<Message> {
        if self.column.is_browser() {
            iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
        } else {
            iced::widget::operation::focus("note-editor")
        }
    }

    /// Make `column` active without selecting anything, as a click inside it does.
    pub(super) fn activate_column(&mut self, column: Column) -> Task<Message> {
        self.column = column;
        if column.is_browser() {
            self.focus = false;
        }
        self.column_focus()
    }

    /// The browser panes shown: folded ones (left of the active column after
    /// a keyboard move) drop out and the breadcrumb stands in for them.
    pub(super) fn visible_panes(&self) -> &'static [Pane] {
        &PANES[self.fold.min(2)..]
    }

    /// The folded columns, left to right, as the breadcrumb shows them.
    pub(super) fn folded_columns(&self) -> Vec<Column> {
        PANES[..self.fold.min(2)]
            .iter()
            .map(|&pane| Column::of_pane(pane))
            .collect()
    }

    /// Open `column` from the breadcrumb: it unfolds, with everything to its
    /// right, and takes the keyboard.
    pub(super) fn open_column(&mut self, column: Column) -> Gateway {
        if self.session.is_none() {
            return self.blocked("No workspace is open");
        }
        self.fold = self.fold.min(column.fold());
        let task = self.activate_column(column);
        self.finish_action(Disposition::Completed, None, task)
    }

    /// ↑ / ↓ in a browser column: select the neighbouring row. Selection is
    /// live, exactly like a click, so the next column and the editor follow.
    pub(super) fn step_column(&mut self, delta: isize) -> Gateway {
        if self.session.is_none() {
            return self.blocked("No workspace is open");
        }
        let command = match self.column {
            Column::Sections => {
                let rows = self.visible_sections();
                neighbour(&rows, self.selected_section, delta).map(Command::SelectSection)
            }
            Column::Groups => {
                let rows = self.visible_groups();
                neighbour(&rows, self.selected_group, delta)
                    .map(|group| Command::SelectParent(Parent::Group(group)))
            }
            Column::Notes => {
                let rows: Vec<_> = self.note_hits().into_iter().map(|hit| hit.note).collect();
                neighbour(&rows, self.selected_note, delta)
                    .map(|note| Command::OpenNote(self.note_target(note)))
            }
            Column::Editor => None,
        };
        match command {
            Some(command) => self.dispatch(command),
            None => self.finish_action(Disposition::Completed, None, Task::none()),
        }
    }

    /// ↵, →, or Cmd/Alt+Shift+]: move one column right. It needs a selection
    /// in the current column, so an empty group stays put.
    pub(super) fn column_right(&mut self) -> Gateway {
        if self.session.is_none() {
            return self.blocked("No workspace is open");
        }
        let next = match self.column {
            Column::Sections if self.selected_section.is_some() => Column::Groups,
            Column::Groups if self.selected_group.is_some() => Column::Notes,
            Column::Notes if self.selected_note.is_some() => Column::Editor,
            Column::Sections => return self.blocked("Select a section first"),
            Column::Groups => return self.blocked("Select a group first"),
            Column::Notes => return self.blocked("Select a note first"),
            Column::Editor => return self.blocked("The editor is the rightmost column"),
        };
        self.column = next;
        self.fold = next.fold();
        let task = self.column_focus();
        self.finish_action(Disposition::Completed, None, task)
    }

    /// ←, or Cmd/Alt+Shift+[: move one column left. From the editor in focus
    /// mode this leaves focus mode, since the Notes column is hidden there.
    pub(super) fn column_left(&mut self) -> Gateway {
        if self.session.is_none() {
            return self.blocked("No workspace is open");
        }
        let previous = match self.column {
            Column::Sections => return self.blocked("Sections is the leftmost column"),
            Column::Groups => Column::Sections,
            Column::Notes => Column::Groups,
            Column::Editor => Column::Notes,
        };
        // Moving left only ever unfolds; it never hides a column you could see.
        self.fold = self.fold.min(previous.fold());
        let task = self.activate_column(previous);
        self.finish_action(Disposition::Completed, None, task)
    }

    /// Esc: back to writing, keeping the selection. Leaving a browser column
    /// this way folds like moving right; Esc in the editor changes nothing.
    pub(super) fn return_to_editor(&mut self) -> Task<Message> {
        if self.column.is_browser() {
            self.fold = Column::Editor.fold();
        }
        self.column = Column::Editor;
        self.column_focus()
    }
}

/// The row `delta` steps from `current`, clamped to the list. With nothing
/// selected (or the selection filtered out), the first row.
fn neighbour<T: Copy + PartialEq>(rows: &[T], current: Option<T>, delta: isize) -> Option<T> {
    let last = rows.len().checked_sub(1)?;
    let index = match current.and_then(|current| rows.iter().position(|row| *row == current)) {
        Some(index) => index.saturating_add_signed(delta).min(last),
        None => 0,
    };
    let next = rows[index];
    (Some(next) != current).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::neighbour;

    #[test]
    fn neighbour_clamps_and_reports_no_move_at_the_ends() {
        let rows = [10, 20, 30];
        assert_eq!(neighbour(&rows, Some(20), 1), Some(30));
        assert_eq!(neighbour(&rows, Some(20), -1), Some(10));
        assert_eq!(neighbour(&rows, Some(30), 1), None);
        assert_eq!(neighbour(&rows, Some(10), -1), None);
        assert_eq!(neighbour(&rows, None, 1), Some(10));
        assert_eq!(neighbour(&rows, Some(99), -1), Some(10));
        assert_eq!(neighbour::<i32>(&[], Some(1), 1), None);
    }
}
