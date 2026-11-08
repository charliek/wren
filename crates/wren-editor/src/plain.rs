//! Plain-text editing for the keys Markdown assist would plan, used when it
//! is off.
//!
//! Only the keys whose stock handling is wrong for any text get a plan here:
//! Backspace deletes a grapheme cluster rather than a scalar, Tab inserts
//! spaces rather than nothing (or indents the lines of a selection that
//! spans lines), Shift+Tab outdents, and ⌘⌫ deletes to column 0. Delete,
//! the word deletes, and the document motions know nothing of Markdown, so
//! the planner handles them either way.

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
        Intent::Tab if sel.start().line == sel.end().line => {
            let start = sel.start();
            let before = &lines.line(start.line)[..start.col];
            let spaces = TAB_STOP - column(before) % TAB_STOP;
            replace(sel, " ".repeat(spaces))
        }
        Intent::Tab => shift_lines(lines, sel, true),
        Intent::ShiftTab => shift_lines(lines, sel, false),
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
        // Enter and Home are the stock ones, and the Markdown commands do
        // nothing.
        Intent::Enter
        | Intent::ShiftEnter
        | Intent::Home { .. }
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

/// Indent (`indent`) or outdent every line the selection covers by one
/// step, keeping the selection on the same text. A line that starts with a
/// tab gains or loses a tab; any other gains [`TAB_STOP`] spaces, or loses up
/// to that many. Empty lines are not indented, and a selection that ends at
/// column 0 does not cover that line.
fn shift_lines(lines: Lines<'_>, sel: Selection, indent: bool) -> EditPlan {
    let (start, end) = (sel.start(), sel.end());
    let last = if end.line > start.line && end.col == 0 {
        end.line - 1
    } else {
        end.line
    };
    let mut edits = Vec::new();
    let (mut anchor, mut head) = (sel.anchor, sel.head);
    for l in start.line..=last {
        let line = lines.line(l);
        let floor = if l == 0 && line.starts_with(BOM) {
            BOM.len_utf8()
        } else {
            0
        };
        let rest = &line[floor..];
        let unit = if rest.starts_with('\t') { 1 } else { TAB_STOP };
        let (removed, text) = if indent {
            if rest.is_empty() {
                continue;
            }
            (
                0,
                if unit == 1 {
                    "\t".into()
                } else {
                    " ".repeat(unit)
                },
            )
        } else {
            let spaces = rest.bytes().take(unit).take_while(|&b| b == b' ').count();
            (if unit == 1 { 1 } else { spaces }, String::new())
        };
        if removed == 0 && text.is_empty() {
            continue;
        }
        for pos in [&mut anchor, &mut head] {
            if pos.line != l {
                continue;
            }
            pos.col = if indent {
                if pos.col > floor {
                    pos.col + text.len()
                } else {
                    pos.col
                }
            } else if pos.col >= floor + removed {
                pos.col - removed
            } else {
                pos.col.min(floor)
            };
        }
        edits.push(Replace {
            start: Pos::new(l, floor),
            end: Pos::new(l, floor + removed),
            text,
        });
    }
    EditPlan {
        edits,
        selection: Selection::new(anchor, head),
    }
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
    fn tab_over_several_lines_indents_them_and_keeps_the_text() {
        // The selection from the start of "one" to the end of "three".
        let selected = Selection::new(Pos::new(0, 0), Pos::new(3, 5));
        let plan = planned("one\n\ttwo\n\nthree", selected, Intent::Tab).unwrap();
        assert_eq!(
            plan.edits,
            [
                edit((0, 0), (0, 0), "    "),
                edit((1, 0), (1, 0), "\t"),
                edit((3, 0), (3, 0), "    "),
            ]
        );
        assert_eq!(
            plan.selection,
            Selection::new(Pos::new(0, 0), Pos::new(3, 9))
        );
        // A selection that ends at column 0 leaves that line alone, and a
        // byte order mark stays first.
        let selected = Selection::new(Pos::new(1, 0), Pos::new(0, 4));
        let plan = planned("\u{feff}ab\ncd", selected, Intent::Tab).unwrap();
        assert_eq!(plan.edits, [edit((0, 3), (0, 3), "    ")]);
        assert_eq!(
            plan.selection,
            Selection::new(Pos::new(1, 0), Pos::new(0, 8))
        );
    }

    #[test]
    fn shift_tab_outdents_the_covered_lines_one_step() {
        let plan = planned("      a", caret(0, 7), Intent::ShiftTab).unwrap();
        assert_eq!(plan.edits, [edit((0, 0), (0, 4), "")]);
        assert_eq!(plan.selection, caret(0, 3));
        let selected = Selection::new(Pos::new(0, 1), Pos::new(2, 2));
        let plan = planned("  a\n\t\tb\nc\n    d", selected, Intent::ShiftTab).unwrap();
        assert_eq!(
            plan.edits,
            [edit((0, 0), (0, 2), ""), edit((1, 0), (1, 1), "")]
        );
        assert_eq!(
            plan.selection,
            Selection::new(Pos::new(0, 0), Pos::new(2, 2))
        );
        let plan = planned("x", caret(0, 1), Intent::ShiftTab).unwrap();
        assert!(plan.edits.is_empty());
        assert_eq!(plan.selection, caret(0, 1));
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
        for intent in [Intent::ToggleTask, Intent::Bold, Intent::MoveLineUp] {
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
