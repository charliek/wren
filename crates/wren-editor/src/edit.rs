//! Range edits and the line endings their inserted text takes.

use std::borrow::Cow;

use iced::advanced::graphics::text::cosmic_text::LineIter;
use iced::widget::text_editor::{LineEnding, Position};

use crate::backend::{self, Lines};

/// One replacement in an [`EditorView::apply`](crate::EditorView::apply)
/// plan: the text from `start` to `end` becomes `text`.
///
/// Positions are a line index and a UTF-8 byte column, as everywhere in this
/// crate. Each line break in `text` (`\n`, or any other ending) is written
/// with the ending of the line it splits.
#[derive(Debug, Clone, PartialEq)]
pub struct Replace {
    pub start: Position,
    pub end: Position,
    pub text: String,
}

impl Replace {
    /// Insert `text` at `at`.
    pub fn insert(at: Position, text: impl Into<String>) -> Self {
        Self {
            start: at,
            end: at,
            text: text.into(),
        }
    }
}

/// `edits` from the top of the document down, or `None` when one is off
/// the text, splits a character, runs backwards, or overlaps another.
/// Insertions at the same place keep their order.
pub(crate) fn ordered<'a>(edits: &'a [Replace], lines: Lines<'_>) -> Option<Vec<&'a Replace>> {
    let mut ordered: Vec<_> = edits.iter().collect();
    ordered.sort_by_key(|edit| (key(edit.start), key(edit.end)));
    let valid = ordered.iter().all(|edit| {
        addressable(lines, edit.start)
            && addressable(lines, edit.end)
            && key(edit.start) <= key(edit.end)
    }) && ordered
        .windows(2)
        .all(|pair| key(pair[0].end) <= key(pair[1].start));
    valid.then_some(ordered)
}

fn key(position: Position) -> (usize, usize) {
    (position.line, position.column)
}

fn addressable(lines: Lines<'_>, position: Position) -> bool {
    lines.get(position.line).is_some_and(|line| {
        position.column <= line.text.len() && line.text.is_char_boundary(position.column)
    })
}

/// The ending for a line break inserted in place of `start..end`: that of
/// the line being split, which after the deletion carries `end`'s ending.
/// The last line has none, so it borrows the line above's, else LF.
pub(crate) fn split_ending(lines: Lines<'_>, start: Position, end: Position) -> LineEnding {
    let own = |line: usize| {
        lines
            .get(line)
            .map(|line| line.ending)
            .filter(|ending| *ending != LineEnding::None)
    };
    own(end.line)
        .or_else(|| start.line.checked_sub(1).and_then(own))
        .unwrap_or(LineEnding::Lf)
}

/// `text` with every line break (LF, CRLF, CR, or LFCR) written as `ending`.
pub(crate) fn with_breaks(text: &str, ending: LineEnding) -> Cow<'_, str> {
    let lines = || LineIter::new(text).map(|(range, found)| (range, backend::ending(found)));
    if lines().all(|(_, found)| found == ending || found == LineEnding::None) {
        return Cow::Borrowed(text);
    }
    let mut written = String::with_capacity(text.len());
    for (range, found) in lines() {
        written.push_str(&text[range]);
        if found != LineEnding::None {
            written.push_str(ending.as_str());
        }
    }
    Cow::Owned(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::Content;
    use crate::BackendKind;

    fn at(line: usize, column: usize) -> Position {
        Position { line, column }
    }

    #[test]
    fn breaks_take_the_requested_ending() {
        let text = "a\nb\r\nc\rd\n\re";
        assert_eq!(with_breaks(text, LineEnding::CrLf), "a\r\nb\r\nc\r\nd\r\ne");
        assert_eq!(with_breaks(text, LineEnding::Lf), "a\nb\nc\nd\ne");
        assert_eq!(with_breaks("\n\n", LineEnding::Cr), "\r\r");
        assert!(matches!(
            with_breaks("x\r\ny\r\n", LineEnding::CrLf),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            with_breaks("plain", LineEnding::Cr),
            Cow::Borrowed(_)
        ));
        assert_eq!(with_breaks("", LineEnding::CrLf), "");
    }

    #[test]
    fn a_split_takes_the_ending_of_the_line_it_splits() {
        for kind in BackendKind::ALL {
            let content = Content::new(kind, "a\r\nb\nc\rlast");
            content.with_lines(|lines| {
                assert_eq!(split_ending(lines, at(0, 1), at(0, 1)), LineEnding::CrLf);
                assert_eq!(split_ending(lines, at(0, 1), at(1, 0)), LineEnding::Lf);
                assert_eq!(split_ending(lines, at(2, 0), at(2, 1)), LineEnding::Cr);
                // The last line has no ending of its own.
                assert_eq!(split_ending(lines, at(3, 2), at(3, 4)), LineEnding::Cr);
                assert_eq!(split_ending(lines, at(1, 0), at(3, 4)), LineEnding::CrLf);
                assert_eq!(split_ending(lines, at(0, 0), at(3, 0)), LineEnding::Lf);
            });
            for text in ["", "one line"] {
                Content::new(kind, text).with_lines(|lines| {
                    assert_eq!(split_ending(lines, at(0, 0), at(0, 0)), LineEnding::Lf);
                });
            }
        }
    }

    #[test]
    fn plans_are_ordered_top_down_and_invalid_ones_are_refused() {
        let content = Content::new(BackendKind::Iced, "héllo\nworld");
        let edit = |start: Position, end: Position| Replace {
            start,
            end,
            text: String::new(),
        };
        content.with_lines(|lines| {
            let plan = [
                edit(at(1, 0), at(1, 5)),
                Replace::insert(at(0, 3), "a"),
                Replace::insert(at(0, 3), "b"),
                edit(at(0, 3), at(0, 4)),
            ];
            let order: Vec<_> = ordered(&plan, lines)
                .unwrap()
                .into_iter()
                .map(|edit| plan.iter().position(|other| std::ptr::eq(edit, other)))
                .collect();
            assert_eq!(order, [Some(1), Some(2), Some(3), Some(0)]);
            assert!(ordered(&[], lines).unwrap().is_empty());

            for invalid in [
                // Overlapping.
                vec![edit(at(0, 0), at(0, 4)), edit(at(0, 3), at(1, 0))],
                // Backwards.
                vec![edit(at(1, 2), at(1, 1))],
                // Inside é.
                vec![edit(at(0, 2), at(0, 4))],
                // Past the line or the text.
                vec![Replace::insert(at(0, 7), "x")],
                vec![Replace::insert(at(2, 0), "x")],
            ] {
                assert!(ordered(&invalid, lines).is_none(), "{invalid:?}");
            }
        });
    }
}
