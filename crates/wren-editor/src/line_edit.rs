//! A host's edits to whole task lines: planned over the text as the
//! Markdown keys read it, with `wren_markdown::task`, and applied as one
//! undo step once each line is checked to read as it did.

use wren_markdown::task::{self, LineEdit, Refused};
use wren_markdown::{Lines, Options, Selection};

use crate::edit::Replace;
use crate::{iced_cursor, markdown_selection, EditorView};

impl EditorView {
    /// Read the text as the Markdown keys plan over it: its lines, and the
    /// planner's options, tasks included ([`EditorView::set_tasks`]).
    pub fn read_markdown<R>(&self, read: impl FnOnce(&dyn Lines, &Options) -> R) -> R {
        let options = self.plan_options();
        self.content.with_lines(|lines| read(&lines, &options))
    }

    /// [`read_markdown`](Self::read_markdown), with the selection a key's
    /// plan is made at: a word or line selection made by a double or
    /// triple click is first given its bounds, as for an [`crate::Intent`].
    pub fn plan_markdown<R>(
        &mut self,
        plan: impl FnOnce(&dyn Lines, Selection, &Options) -> R,
    ) -> R {
        let selection = markdown_selection(self.snapshot_cursor());
        self.read_markdown(|lines, options| plan(lines, selection, options))
    }

    /// Apply `edits`, planned on this text at one selection and each on a
    /// line of its own (`wren_markdown::task::edit` and `toggle` give
    /// them), as one undoable change, leaving the selection they move.
    /// Whatever popover is open closes. Refused, changing nothing, once
    /// any of their lines no longer reads as it did
    /// ([`LineEdit::plan_for`]), and when they do not fit together or the
    /// text ([`Refused::Invalid`]). Returns whether the text changed.
    pub fn apply_line_edits(&mut self, edits: &[LineEdit]) -> Result<bool, Refused> {
        let plan = self.content.with_lines(|lines| task::join(edits, &lines))?;
        if plan.edits.is_empty() {
            self.close_open();
            self.history.break_group();
            if !edits.is_empty() {
                self.select(iced_cursor(plan.selection));
            }
            return Ok(false);
        }
        let replaces: Vec<Replace> = plan.edits.into_iter().map(Replace::from).collect();
        // `apply` changes nothing for edits that are no places in the text,
        // which must not pass for edits that changed nothing.
        if (self.content)
            .with_lines(|lines| crate::edit::ordered(&replaces, lines))
            .is_none()
        {
            return Err(Refused::Invalid);
        }
        Ok(self.apply(&replaces, iced_cursor(plan.selection)))
    }
}

#[cfg(test)]
mod tests {
    use iced::widget::text_editor::{Cursor, Position};
    use wren_markdown::date::Date;
    use wren_markdown::task::Change;
    use wren_markdown::TaskOptions;

    use crate::{BackendKind, EditorView};

    fn tasks_on() -> Option<TaskOptions> {
        Some(TaskOptions {
            today: Date::new(2026, 11, 19).expect("a date"),
        })
    }

    fn at(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    /// The caret's task line edited with `change`.
    fn change_at_caret(ev: &mut EditorView, change: &Change) -> Option<bool> {
        let edit = ev.plan_markdown(|lines, sel, opts| {
            wren_markdown::task::edit(lines, sel.head.line, sel, change, opts)
        })?;
        Some(ev.apply_line_edits(&[edit]).expect("planned on this text"))
    }

    #[test]
    fn a_line_edit_is_one_undo_step_on_both_engines() {
        for kind in BackendKind::ALL {
            let before = "# Note\n- [ ] Send the deck 📅 2026-12-01\n\nprose";
            let mut ev = EditorView::with_backend(kind, before);
            ev.set_tasks(tasks_on());
            let end = "- [ ] Send the deck".len();
            ev.move_cursor(at(1, end));
            assert_eq!(
                change_at_caret(&mut ev, &Change::Cycle),
                Some(true),
                "{kind:?}"
            );
            assert_eq!(
                change_at_caret(
                    &mut ev,
                    &Change::SetPriority(Some(wren_markdown::task::Priority::High))
                ),
                Some(true)
            );
            assert_eq!(
                ev.text(),
                "# Note\n- [/] Send the deck ⏫ 📅 2026-12-01\n\nprose",
                "{kind:?}"
            );
            assert_eq!(
                ev.cursor(),
                at(1, end),
                "{kind:?}: the caret keeps its text"
            );
            assert!(ev.undo());
            assert_eq!(
                ev.text(),
                "# Note\n- [/] Send the deck 📅 2026-12-01\n\nprose"
            );
            assert!(ev.undo());
            assert_eq!(ev.text(), before, "{kind:?}");

            // A change that changes nothing is no edit and no undo step.
            assert_eq!(
                change_at_caret(
                    &mut ev,
                    &Change::SetDue(Some(Date::new(2026, 12, 1).unwrap()))
                ),
                Some(false)
            );
            assert!(!ev.can_undo(), "{kind:?}");
            // A line that is no task's plans nothing.
            ev.move_cursor(at(3, 0));
            assert_eq!(change_at_caret(&mut ev, &Change::Cycle), None, "{kind:?}");
            ev.move_cursor(at(0, 0));
            assert_eq!(change_at_caret(&mut ev, &Change::Cycle), None, "{kind:?}");
        }
    }

    #[test]
    fn a_stale_edit_changes_nothing() {
        for kind in BackendKind::ALL {
            let mut ev = EditorView::with_backend(kind, "- [ ] a\n- [ ] b");
            ev.set_tasks(tasks_on());
            ev.move_cursor(at(0, 7));
            let edit = ev
                .plan_markdown(|lines, sel, opts| {
                    wren_markdown::task::edit(lines, 0, sel, &Change::Cycle, opts)
                })
                .unwrap();
            ev.move_cursor(at(0, 7));
            ev.perform(iced::widget::text_editor::Action::Edit(
                iced::widget::text_editor::Edit::Insert('!'),
            ));
            assert_eq!(
                ev.apply_line_edits(&[edit]),
                Err(wren_markdown::task::Refused::Stale)
            );
            assert_eq!(ev.text(), "- [ ] a!\n- [ ] b", "{kind:?}");
        }
    }

    /// Edits that do not fit together are refused, never applied in part
    /// or passed off as edits that changed nothing.
    #[test]
    fn edits_that_overlap_are_refused_and_change_nothing() {
        for kind in BackendKind::ALL {
            let mut ev = EditorView::with_backend(kind, "- [ ] a\n- [ ] b");
            ev.set_tasks(tasks_on());
            ev.move_cursor(at(0, 7));
            let edits = ev.plan_markdown(|lines, sel, opts| {
                [Change::Cycle, Change::ToggleDone]
                    .iter()
                    .map(|change| wren_markdown::task::edit(lines, 0, sel, change, opts).unwrap())
                    .collect::<Vec<_>>()
            });
            assert_eq!(
                ev.apply_line_edits(&edits),
                Err(wren_markdown::task::Refused::Invalid),
                "{kind:?}"
            );
            assert_eq!(ev.text(), "- [ ] a\n- [ ] b", "{kind:?}");
            assert!(!ev.can_undo(), "{kind:?}");
        }
    }

    #[test]
    fn the_toggle_over_a_selection_is_one_undo_step() {
        for kind in BackendKind::ALL {
            let before = "- [ ] a\n- [-] b ❌ 2026-11-18\n- [x] c";
            let mut ev = EditorView::with_backend(kind, before);
            ev.set_tasks(tasks_on());
            ev.move_cursor(Cursor {
                position: Position { line: 2, column: 7 },
                selection: Some(Position { line: 0, column: 0 }),
            });
            let edits = ev
                .plan_markdown(|lines, sel, opts| wren_markdown::task::toggle(lines, sel, opts))
                .unwrap()
                .unwrap();
            assert_eq!(ev.apply_line_edits(&edits), Ok(true));
            assert_eq!(
                ev.text(),
                "- [x] a ✅ 2026-11-19\n- [x] b ✅ 2026-11-19\n- [x] c",
                "{kind:?}"
            );
            assert_eq!(
                ev.cursor(),
                Cursor {
                    position: Position { line: 2, column: 7 },
                    selection: Some(Position { line: 0, column: 0 }),
                },
                "{kind:?}"
            );
            assert!(ev.undo());
            assert_eq!(ev.text(), before, "{kind:?}: one step");
        }
    }

    /// Select all of a 50,000-line note of tasks and toggle: planned in one
    /// read of the note, checked in another and applied, on both engines,
    /// well inside a bound a debug build keeps (each takes about a third
    /// of a second). Undoing it is the history's own cost, not timed here;
    /// that a toggle is one undo step is tested on a short note.
    #[test]
    fn a_long_toggle_is_one_pass_and_one_step() {
        let text: String = (0..25_000)
            .map(|i| format!("- [ ] task {i}\n  its next line\n"))
            .collect();
        for kind in BackendKind::ALL {
            let mut ev = EditorView::with_backend(kind, &text);
            ev.set_tasks(tasks_on());
            let last = ev.line_count() - 1;
            ev.move_cursor(Cursor {
                position: Position {
                    line: last,
                    column: 0,
                },
                selection: Some(Position { line: 0, column: 0 }),
            });
            let started = std::time::Instant::now();
            let edits = ev
                .plan_markdown(|lines, sel, opts| wren_markdown::task::toggle(lines, sel, opts))
                .unwrap()
                .unwrap();
            assert_eq!(ev.apply_line_edits(&edits), Ok(true), "{kind:?}");
            let elapsed = started.elapsed();
            assert!(
                elapsed < std::time::Duration::from_secs(4),
                "{kind:?}: {elapsed:?}"
            );
            let after = ev.text();
            assert!(
                after.starts_with("- [x] task 0 ✅ 2026-11-19\n"),
                "{kind:?}"
            );
            assert_eq!(after.matches('✅').count(), 25_000, "{kind:?}");
        }
    }
}
