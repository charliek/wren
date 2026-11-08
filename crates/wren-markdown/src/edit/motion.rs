//! Home and the document start and end (§5.5): motions without edits.

use super::*;

/// The smart Home cycle: to the line's text start, and from there to
/// column 0. A cursor inside the indentation or markup moves forward to the
/// text start. The editor handles the stop at a wrapped row's start before
/// planning (see [`Intent::Home`]).
pub(super) fn home<L: Lines + ?Sized>(base: &L, sel: Selection, extend: bool) -> EditPlan {
    let sel = clamp_selection(base, sel);
    let head = sel.head;
    let text = base.line(head.line);
    let floor = bom_len(head.line, text);
    let text_start = text_start(base, head.line, floor);
    let col = if head.col == text_start {
        floor
    } else {
        text_start
    };
    move_to(sel, Pos::new(head.line, col), extend)
}

/// Line `l`'s text start, at or after `floor` (its byte order mark).
pub(super) fn text_start<L: Lines + ?Sized>(base: &L, l: usize, floor: usize) -> usize {
    let text = base.line(l);
    // No block markup starts with a letter, so the scan, which reads every
    // line up to this one, can be skipped.
    if text[floor..].starts_with(char::is_alphabetic) {
        return floor;
    }
    let an = Analysis::through(base, l);
    an.recs[l].info.text_start.clamp(floor, text.len())
}

pub(super) fn document_start<L: Lines + ?Sized>(
    base: &L,
    sel: Selection,
    extend: bool,
) -> EditPlan {
    let sel = clamp_selection(base, sel);
    move_to(sel, Pos::new(0, bom_len(0, base.line(0))), extend)
}

pub(super) fn document_end<L: Lines + ?Sized>(base: &L, sel: Selection, extend: bool) -> EditPlan {
    let sel = clamp_selection(base, sel);
    let last = base.line_count() - 1;
    move_to(sel, Pos::new(last, base.line(last).len()), extend)
}

/// Move the head to `to`, keeping the anchor when extending.
fn move_to(sel: Selection, to: Pos, extend: bool) -> EditPlan {
    unchanged(if extend {
        Selection::new(sel.anchor, to)
    } else {
        Selection::caret(to)
    })
}

#[cfg(test)]
mod tests {
    use crate::edit::tests::check;
    use crate::{plan, Document, EditPlan, Intent, Options, Pos, Selection};

    const HOME: Intent = Intent::Home { extend: false };
    const SHIFT_HOME: Intent = Intent::Home { extend: true };

    #[test]
    fn home_cycles_between_text_start_and_column_0() {
        check(
            HOME,
            &[
                ("- [ ] foo|", "- [ ] |foo"),
                ("- [ ] |foo", "|- [ ] foo"),
                ("|- [ ] foo", "- [ ] |foo"),
                ("- [|x] foo", "- [x] |foo"),
                ("  |  text", "    |text"),
                ("# Ti|tle", "# |Title"),
                ("> > - a|", "> > - |a"),
                ("- a\n  b|", "- a\n  |b"),
                ("plain|", "|plain"),
                ("|plain", "|plain"),
                ("|", "|"),
                ("- |", "|- "),
                ("```\n  co|de", "```\n  |code"),
                ("\u{feff}- a|", "\u{feff}- |a"),
                ("\u{feff}- |a", "\u{feff}|- a"),
            ],
        );
    }

    #[test]
    fn shift_home_extends_through_the_same_stops() {
        check(
            SHIFT_HOME,
            &[
                ("- foo|", "- »foo«"),
                ("- f«oo»", "- »f«oo"),
                ("- »foo«", "»- foo«"),
                ("«- foo»", "«- »foo"),
            ],
        );
    }

    #[test]
    fn document_start_and_end() {
        check(
            Intent::DocumentStart { extend: false },
            &[("a\nb|c", "|a\nbc"), ("\u{feff}a\nb|", "\u{feff}|a\nb")],
        );
        check(
            Intent::DocumentStart { extend: true },
            &[("a\nb|c", "»a\nb«c")],
        );
        let doc = Document::new("a\r\nbc");
        let end = |extend| {
            plan(
                &doc,
                Selection::caret(Pos::new(0, 1)),
                Intent::DocumentEnd { extend },
                &Options::default(),
            )
        };
        assert_eq!(
            end(false),
            EditPlan {
                edits: Vec::new(),
                selection: Selection::caret(Pos::new(1, 2))
            }
        );
        assert_eq!(
            end(true).selection,
            Selection::new(Pos::new(0, 1), Pos::new(1, 2))
        );
    }
}
