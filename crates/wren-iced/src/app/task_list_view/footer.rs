//! The pane's footer under the task view: its counts, what it leaves out,
//! and its keys' hint.

use iced::widget::row;
use iced::{Element, Shrink};

use crate::app::tasks_view::Scope;
use crate::app::{App, Message};
use crate::ellipsis::ellipsis;
use crate::{chrome, fonts, theme};

/// The space between the footer's counts and its hint.
const FOOTER_GAP: f32 = 12.0;

/// What separates the pieces of the footer's hint.
const HINT_SEPARATOR: &str = " \u{b7} ";

/// What the footer says the list's keys are.
pub(in crate::app) const HINT: &str =
    "Space toggle \u{b7} S D P \u{b7} E edit \u{b7} F2 rename \u{b7} \u{21b5} open \u{b7} R refresh";

impl App {
    /// The pane's footer under the view, `width` wide: its counts, what it
    /// leaves out, and as much of its keys' hint as the room they leave
    /// holds, in whole pieces from the first ([`App::task_list_hint`]).
    /// The counts never give way; what is left out ends in "…" only when
    /// the window is too narrow for it beside them.
    pub(in crate::app) fn task_list_footer(&self, width: f32) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let (counts, left) = self.task_list.footer.clone();
        let open = format!("{} open \u{b7} ", counts.open_and_doing);
        let overdue = format!("{} overdue", counts.overdue);
        let left = left.map(|left| format!(" \u{b7} {left}"));
        let label = |content: &str| self.label_widths.of(content, 11.0, fonts::UI);
        let used = label(&open) + label(&overdue) + left.as_deref().map_or(0.0, label);
        // The gaps either side of the space between the counts and the hint.
        let hint = self.task_list_hint(width - used - 2.0 * FOOTER_GAP);
        let mut pieces = row![
            chrome::label(open, 11, colors.muted),
            chrome::label(
                overdue,
                11,
                if counts.overdue > 0 {
                    colors.danger
                } else {
                    colors.muted
                }
            ),
        ];
        if let Some(left) = left {
            pieces = pieces.push(ellipsis(left, 11.0, theme::color(colors.muted)).width(Shrink));
        }
        let mut line = row![pieces.clip(true), iced::widget::space::horizontal()]
            .spacing(FOOTER_GAP)
            .align_y(iced::Center)
            .clip(true);
        if !hint.is_empty() {
            line = line.push(chrome::label(hint, 11, colors.faint));
        }
        line.into()
    }

    /// The pieces of the keys' hint ([`HINT`]) that fit in `room`, whole
    /// and from the first, with their separators; empty when not even the
    /// first does. A word is never cut.
    pub(in crate::app) fn task_list_hint(&self, room: f32) -> String {
        let pieces: Vec<&str> = HINT.split(HINT_SEPARATOR).collect();
        let widths = (pieces.iter().enumerate()).map(|(index, piece)| {
            let gap = if index > 0 {
                self.label_widths.of(HINT_SEPARATOR, 11.0, fonts::UI)
            } else {
                0.0
            };
            gap + self.label_widths.of(piece, 11.0, fonts::UI)
        });
        let shown = crate::app::task_footer::pieces_that_fit(widths, room);
        pieces[..shown].join(HINT_SEPARATOR)
    }

    /// The footer's counts for `scope`, and what it says is left out.
    pub(in crate::app) fn task_list_counts(
        &self,
        scope: &Scope,
    ) -> (wren_core::tasks::Counts, Option<String>) {
        let Some(tasks) = self.session.as_ref().and_then(|session| session.tasks()) else {
            return (wren_core::tasks::Counts::default(), None);
        };
        let counts = tasks.counts_under(scope.prefix(), self.today());
        let left = tasks.left_out_under(scope.prefix());
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let mut parts = Vec::new();
        if left.hidden_notes > 0 {
            parts.push(format!(
                "{} ({}, tasks: off)",
                plural(left.hidden_notes, "note", "notes"),
                plural(left.hidden_tasks, "task", "tasks")
            ));
        }
        if left.excluded_notes > 0 {
            parts.push(format!(
                "{} in hidden folders ({})",
                plural(left.excluded_notes, "note", "notes"),
                plural(left.excluded_tasks, "task", "tasks")
            ));
        }
        if left.unreadable_notes > 0 {
            parts.push(plural(
                left.unreadable_notes,
                "unreadable note",
                "unreadable notes",
            ));
        }
        let left = (!parts.is_empty()).then(|| format!("Left out: {}", parts.join(", ")));
        (counts, left)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    /// A narrow window: the footer's hint gives way in whole pieces from
    /// its end, never a word cut, before the counts do.
    #[test]
    fn the_footer_hint_gives_way_in_whole_pieces() {
        crate::fonts::load();
        let (_directory, app) = fixture();
        let pieces: Vec<&str> = HINT.split(HINT_SEPARATOR).collect();
        assert_eq!(pieces.len(), 6);
        assert_eq!(app.task_list_hint(10_000.0), HINT);
        assert_eq!(app.task_list_hint(0.0), "");
        assert_eq!(app.task_list_hint(-40.0), "");
        let width = |text: &str| crate::app::task_footer::label_width(text, 11.0, fonts::UI);
        let mut room = 0.0;
        for count in 1..=pieces.len() {
            let gap = if count > 1 {
                width(HINT_SEPARATOR)
            } else {
                0.0
            };
            room += gap + width(pieces[count - 1]);
            let whole = pieces[..count].join(HINT_SEPARATOR);
            assert_eq!(app.task_list_hint(room), whole, "{room}");
            assert_eq!(
                app.task_list_hint(room - 0.5),
                pieces[..count - 1].join(HINT_SEPARATOR),
                "{room}"
            );
        }
        // Every room shows whole pieces from the first.
        for room in (0..600).step_by(7) {
            let shown = app.task_list_hint(room as f32);
            assert!(
                shown.is_empty()
                    || (1..=pieces.len())
                        .any(|count| shown == pieces[..count].join(HINT_SEPARATOR)),
                "{shown:?}"
            );
        }
    }
}
