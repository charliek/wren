//! `edit.move_line_up` and `edit.move_line_down` (§5.7).

use super::*;

/// Move the lines the selection covers up or down by one, keeping the
/// selection on them, then renumber the ordered runs around them.
///
/// Line contents move; line endings stay where they are, because a plan
/// writes every break as `'\n'` and the editor gives it the ending of the
/// line being split. A document with one kind of ending is unaffected, and
/// the last line still has no ending (`a⏎b|` up is `b|⏎a`).
pub(super) fn move_lines<L: Lines + ?Sized>(
    base: &L,
    sel: Selection,
    up: bool,
    opts: &Options,
) -> EditPlan {
    let sel = clamp_selection(base, sel);
    let count = base.line_count();
    let (first, last) = covered(&sel);
    if (up && first == 0) || (!up && last + 1 >= count) {
        return unchanged(sel);
    }
    let (lo, hi) = if up {
        (first - 1, last)
    } else {
        (first, last + 1)
    };
    // A byte order mark stays at the very start of the document.
    let bom = &base.line(0)[..bom_len(0, base.line(0))];
    let own = |j: usize| {
        if j == 0 {
            &base.line(0)[bom.len()..]
        } else {
            base.line(j)
        }
    };
    let mut order: Vec<usize> = (lo..=hi).collect();
    if up {
        order.rotate_left(1);
    } else {
        order.rotate_right(1);
    }
    let mut d = Draft::new(base);
    for (k, &from) in order.iter().enumerate() {
        let j = lo + k;
        let head = if j == 0 { bom } else { "" };
        d.set(j, format!("{head}{}", own(from)));
    }
    let mut marks = sel;
    for p in [&mut marks.anchor, &mut marks.head] {
        let col = p.col - if p.line == 0 { bom.len() } else { 0 };
        let line = if up { p.line - 1 } else { p.line + 1 };
        *p = if line < count {
            Pos::new(line, col + if line == 0 { bom.len() } else { 0 })
        } else {
            Pos::new(count - 1, d.line(count - 1).len())
        };
    }
    let an = Analysis::new(base, None);
    let code = (lo..=hi).any(|j| an.recs[j].info.kind.is_code_like());
    if opts.renumber && !code {
        let was: Vec<Option<u32>> = (0..count)
            .map(|j| match an.item_slot(j).and_then(|s| an.item_here(j, s)) {
                Some(ListItem {
                    marker: Marker::Ordered { number, .. },
                    ..
                }) => Some(number),
                _ => None,
            })
            .collect();
        let runs: Vec<Run> = (lo.saturating_sub(1)..=(hi + 1).min(count - 1))
            .map(|line| Run { line, slot: None })
            .collect();
        renumber_runs(&mut d, &runs, &was, &mut marks);
    }
    finish(base, &d, marks)
}

#[cfg(test)]
mod tests {
    use crate::edit::tests::{apply, check, check_with};
    use crate::{plan, Document, Intent, Options, Pos, Selection};

    #[test]
    fn move_line_swaps_with_its_neighbour() {
        check(
            Intent::MoveLineUp,
            &[
                ("a\nb|", "b|\na"),
                ("a\nb|\nc", "b|\na\nc"),
                ("a|\nb", "a|\nb"),
                ("a\n«b\nc»", "«b\nc»\na"),
                ("a\n«b\n»c", "«b\n»a\nc"),
                ("\u{feff}a\nb|", "\u{feff}b|\na"),
                ("\u{feff}a|\nb", "\u{feff}a|\nb"),
            ],
        );
        check(
            Intent::MoveLineDown,
            &[
                ("a|\nb", "b\na|"),
                ("a|\nb\nc", "b\na|\nc"),
                ("«a\nb»\nc", "c\n«a\nb»"),
                ("«a\n»b\nc", "b\n«a\n»c"),
                ("\u{feff}a|\nb", "\u{feff}b\na|"),
                ("\u{feff}|a\nb", "\u{feff}b\n|a"),
            ],
        );
    }

    #[test]
    fn move_line_at_the_bottom_does_nothing() {
        let doc = Document::new("a\nb");
        let opts = Options::default();
        let caret = Selection::caret(Pos::new(1, 1));
        assert_eq!(plan(&doc, caret, Intent::MoveLineDown, &opts).edits, []);
        // A selection that ends at column 0 of the last line covers the line
        // before; moving it down takes the mark to the end of the document.
        let sel = Selection::new(Pos::new(0, 0), Pos::new(1, 0));
        let moved = plan(&doc, sel, Intent::MoveLineDown, &opts);
        assert_eq!(apply(&doc, &moved), "b\na");
        assert_eq!(
            moved.selection,
            Selection::new(Pos::new(1, 0), Pos::new(1, 1))
        );
    }

    #[test]
    fn move_line_renumbers_the_runs_around_it() {
        check(
            Intent::MoveLineUp,
            &[
                ("1. a\n2. b|", "1. b|\n2. a"),
                ("1. a\n2. b\n3. c|", "1. a\n2. c|\n3. b"),
                ("5. a\n6. b|", "5. b|\n6. a"),
                ("1. a\n2. b\n# h|", "1. a\n# h|\n2. b"),
                ("1. a\n1. b|", "1. b|\n1. a"),
                ("1. a\n   - x|\n2. b", "   - x|\n1. a\n2. b"),
            ],
        );
        check(
            Intent::MoveLineDown,
            &[
                ("1. a|\n2. b\n3. c", "1. b\n2. a|\n3. c"),
                ("x|\n1. a\n2. b", "1. a\nx|\n2. b"),
            ],
        );
    }

    #[test]
    fn move_line_never_renumbers_code() {
        check(
            Intent::MoveLineUp,
            &[
                ("```\n1. x\n2. y|\n```", "```\n2. y|\n1. x\n```"),
                ("1. a\n2. b\n```|", "1. a\n```|\n2. b"),
            ],
        );
        let opts = Options {
            renumber: false,
            ..Options::default()
        };
        check_with(Intent::MoveLineUp, &opts, &[("1. a\n2. b|", "2. b|\n1. a")]);
    }
}
