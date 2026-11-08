//! Plain-text editing for the keys Markdown assist would plan, used when it
//! is off.
//!
//! Only the keys whose stock handling is wrong for any text get a plan here:
//! Backspace deletes a grapheme cluster rather than a scalar, Tab inserts
//! spaces rather than nothing, and ⌘⌫ deletes to column 0. Delete, the word
//! deletes, and the document motions know nothing of Markdown, so the
//! planner handles them either way.

use unicode_segmentation::UnicodeSegmentation;
use wren_markdown::{EditPlan, Intent, Lines as _, Pos, Replace, Selection};

use crate::backend::Lines;

/// Columns between tab stops.
const TAB_STOP: usize = 4;

const BOM: char = '\u{feff}';

/// The plain plan for `intent`, or `None` when the planner's own is already
/// plain.
pub(crate) fn plan(lines: Lines<'_>, sel: Selection, intent: Intent) -> Option<EditPlan> {
    Some(match intent {
        Intent::Tab => {
            let start = sel.start();
            let before = &lines.line(start.line)[..start.col];
            let spaces = TAB_STOP - column(before) % TAB_STOP;
            replace(sel, " ".repeat(spaces))
        }
        Intent::Backspace => delete_back(lines, sel, |line, col| {
            line[..col]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(start, _)| start)
        }),
        Intent::DeleteToTextStart => delete_back(lines, sel, |_, _| 0),
        Intent::Delete
        | Intent::DeleteWordBackward
        | Intent::DeleteWordForward
        | Intent::DocumentStart { .. }
        | Intent::DocumentEnd { .. } => return None,
        // Enter and Home are the stock ones; Shift+Tab and the Markdown
        // commands do nothing.
        Intent::Enter
        | Intent::ShiftEnter
        | Intent::Home { .. }
        | Intent::ShiftTab
        | Intent::ToggleTask
        | Intent::Bold
        | Intent::Italic
        | Intent::Code
        | Intent::MoveLineUp
        | Intent::MoveLineDown => unchanged(sel),
    })
}

/// The display column at the end of `text`, with tabs at every
/// [`TAB_STOP`].
fn column(text: &str) -> usize {
    text.chars().fold(0, |column, c| {
        if c == '\t' {
            (column / TAB_STOP + 1) * TAB_STOP
        } else {
            column + 1
        }
    })
}

/// Delete the selection, or back from the cursor to `to(line, column)`. At
/// column 0 the line break goes. A byte order mark is never deleted.
fn delete_back(lines: Lines<'_>, sel: Selection, to: impl Fn(&str, usize) -> usize) -> EditPlan {
    if !sel.is_empty() {
        return replace(sel, String::new());
    }
    let head = sel.head;
    let line = lines.line(head.line);
    let floor = if head.line == 0 && line.starts_with(BOM) {
        BOM.len_utf8()
    } else {
        0
    };
    let start = if head.col > floor {
        Pos::new(head.line, to(line, head.col).max(floor))
    } else if head.col == 0 && head.line > 0 {
        Pos::new(head.line - 1, lines.line(head.line - 1).len())
    } else {
        return unchanged(sel);
    };
    replace(Selection::new(start, head), String::new())
}

/// Replace the selection with `text`, leaving the cursor after it.
fn replace(sel: Selection, text: String) -> EditPlan {
    let start = sel.start();
    let after = match text.rsplit_once('\n') {
        Some((before, last)) => Pos::new(start.line + before.matches('\n').count() + 1, last.len()),
        None => Pos::new(start.line, start.col + text.len()),
    };
    EditPlan {
        edits: vec![Replace {
            start,
            end: sel.end(),
            text,
        }],
        selection: Selection::caret(after),
    }
}

fn unchanged(sel: Selection) -> EditPlan {
    EditPlan {
        edits: Vec::new(),
        selection: sel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::Content;
    use crate::BackendKind;

    fn planned(text: &str, sel: Selection, intent: Intent) -> Option<EditPlan> {
        Content::new(BackendKind::Iced, text).with_lines(|lines| plan(lines, sel, intent))
    }

    fn caret(line: usize, col: usize) -> Selection {
        Selection::caret(Pos::new(line, col))
    }

    fn edit(start: (usize, usize), end: (usize, usize), text: &str) -> Replace {
        Replace {
            start: Pos::new(start.0, start.1),
            end: Pos::new(end.0, end.1),
            text: text.into(),
        }
    }

    #[test]
    fn tab_pads_to_the_next_multiple_of_four_columns() {
        for (text, col, spaces) in [
            ("", 0, 4),
            ("ab", 2, 2),
            ("abcd", 4, 4),
            ("\tx", 2, 3),
            ("é", 2, 3),
        ] {
            let plan = planned(text, caret(0, col), Intent::Tab).unwrap();
            assert_eq!(plan.edits, [edit((0, col), (0, col), &" ".repeat(spaces))]);
            assert_eq!(plan.selection, caret(0, col + spaces));
        }
        let selected = Selection::new(Pos::new(0, 1), Pos::new(0, 3));
        let plan = planned("abcd", selected, Intent::Tab).unwrap();
        assert_eq!(plan.edits, [edit((0, 1), (0, 3), "   ")]);
    }

    #[test]
    fn backspace_deletes_a_grapheme_cluster_or_the_line_break() {
        let flag = "🇯🇵";
        let text = format!("a{flag}e\u{301}\nb");
        let after_accent = 1 + flag.len() + 3;
        let plan = planned(&text, caret(0, after_accent), Intent::Backspace).unwrap();
        assert_eq!(
            plan.edits,
            [edit((0, 1 + flag.len()), (0, after_accent), "")]
        );
        let plan = planned(&text, caret(0, 1 + flag.len()), Intent::Backspace).unwrap();
        assert_eq!(plan.edits, [edit((0, 1), (0, 1 + flag.len()), "")]);
        let plan = planned(&text, caret(1, 0), Intent::Backspace).unwrap();
        assert_eq!(plan.edits, [edit((0, after_accent), (1, 0), "")]);
        assert_eq!(plan.selection, caret(0, after_accent));
        assert!(planned("x", caret(0, 0), Intent::Backspace)
            .unwrap()
            .edits
            .is_empty());
        assert!(planned("\u{feff}x", caret(0, 3), Intent::Backspace)
            .unwrap()
            .edits
            .is_empty());
    }

    #[test]
    fn the_line_delete_goes_to_column_0_and_selections_are_deleted() {
        let plan = planned("- [ ] foo", caret(0, 9), Intent::DeleteToTextStart).unwrap();
        assert_eq!(plan.edits, [edit((0, 0), (0, 9), "")]);
        let selected = Selection::new(Pos::new(1, 1), Pos::new(0, 1));
        let plan = planned("ab\ncd", selected, Intent::Backspace).unwrap();
        assert_eq!(plan.edits, [edit((0, 1), (1, 1), "")]);
        assert_eq!(plan.selection, caret(0, 1));
    }

    #[test]
    fn markdown_keys_do_nothing_and_plain_ones_use_the_planner() {
        for intent in [
            Intent::ShiftTab,
            Intent::ToggleTask,
            Intent::Bold,
            Intent::MoveLineUp,
        ] {
            let plan = planned("- a", caret(0, 3), intent).unwrap();
            assert!(plan.edits.is_empty(), "{intent:?}");
            assert_eq!(plan.selection, caret(0, 3));
        }
        for intent in [
            Intent::Delete,
            Intent::DeleteWordBackward,
            Intent::DeleteWordForward,
            Intent::DocumentEnd { extend: false },
        ] {
            assert!(planned("- a", caret(0, 3), intent).is_none(), "{intent:?}");
        }
    }
}
