//! The active column: which of Sections, Groups, Notes, or the editor has the
//! keyboard, and moving the selection within and between columns.

use iced::Task;
use wren_core::model::{GroupId, NoteId, Parent, SectionId};

use super::commands::{Command, Disposition, Gateway};
use super::navigation::matches_name;
use super::palette::SearchTarget;
use super::{App, Message};

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
        let task = self.activate_column(previous);
        self.finish_action(Disposition::Completed, None, task)
    }

    /// Esc in a browser column: back to writing, keeping the selection.
    pub(super) fn return_to_editor(&mut self) -> Task<Message> {
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
