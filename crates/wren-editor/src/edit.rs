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

impl From<wren_markdown::Replace> for Replace {
    fn from(replace: wren_markdown::Replace) -> Self {
        let position = |at: wren_markdown::Pos| Position {
            line: at.line,
            column: at.col,
        };
        Self {
            start: position(replace.start),
            end: position(replace.end),
            text: replace.text,
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

/// The ending for a line break inserted in place of a range ending at `end`:
/// that of the line being split, which after the deletion carries `end`'s
/// ending. The last line has none, so it borrows the line above `end`'s,
/// even when that line is deleted too (Select All), else LF.
pub(crate) fn split_ending(lines: Lines<'_>, end: Position) -> LineEnding {
    let own = |line: usize| {
        lines
            .get(line)
            .map(|line| line.ending)
            .filter(|ending| *ending != LineEnding::None)
    };
    own(end.line)
        .or_else(|| end.line.checked_sub(1).and_then(own))
        .unwrap_or(LineEnding::Lf)
}

/// The ending for the line breaks in `text`, which replaces `start..end`.
///
/// It is [`split_ending`]'s, unless that would make a break run into the
/// bytes on either side and read back as one: a lone CR line above a break
/// that starts with LF (`\r` + `\n`), a lone LF above one that starts with
/// CR, or a break that ends in CR before an empty rest of line whose own
/// ending starts with LF. The editor would show a line that a reload loses.
/// Then the first ending that keeps every break apart is used instead,
/// trying the line above's first.
pub(crate) fn break_ending(
    lines: Lines<'_>,
    start: Position,
    end: Position,
    text: &str,
) -> LineEnding {
    let chosen = split_ending(lines, end);
    if !text.contains(['\r', '\n']) {
        return chosen;
    }
    let ending_of = |line: usize| lines.get(line).map(|line| line.ending);
    let above = match start.line.checked_sub(1) {
        Some(line) if start.column == 0 => ending_of(line).unwrap_or(LineEnding::None),
        _ => LineEnding::None,
    };
    let after = lines.get(end.line).map(|line| {
        if line.text.len() > end.column {
            ""
        } else {
            line.ending.as_str()
        }
    });
    let breaks = |text: &str| wren_markdown::line_ranges(text).count() - 1;
    let apart = |ending: LineEnding| {
        let inserted = with_breaks(text, ending);
        let after = after.unwrap_or("");
        let joined = format!("{}{inserted}{after}", above.as_str());
        breaks(&joined) == breaks(above.as_str()) + breaks(&inserted) + breaks(after)
    };
    [
        chosen,
        above,
        LineEnding::Lf,
        LineEnding::CrLf,
        LineEnding::Cr,
        LineEnding::LfCr,
    ]
    .into_iter()
    .filter(|ending| *ending != LineEnding::None)
    .find(|ending| apart(*ending))
    .unwrap_or(chosen)
}

/// Whether ending `above` followed directly by `below` (an empty line's)
/// reads back as fewer breaks: `\r` + `\n`, or `\n` + `\r`.
fn fuse(above: LineEnding, below: LineEnding) -> bool {
    let breaks = |text: &str| wren_markdown::line_ranges(text).count() - 1;
    let joined = format!("{}{}", above.as_str(), below.as_str());
    breaks(&joined) < breaks(above.as_str()) + breaks(below.as_str())
}

/// The first empty line in `lines` (a range of line indices) whose ending
/// runs into the line above's, as a deletion can leave it (`a\rb\nc` less
/// `b`), with an ending that keeps it apart from both neighbours. Saved
/// as is, the line would be gone on reload.
pub(crate) fn fused_line(
    lines: Lines<'_>,
    range: std::ops::RangeInclusive<usize>,
) -> Option<(usize, LineEnding)> {
    let line = |index: usize| lines.get(index);
    range.filter(|index| *index > 0).find_map(|index| {
        let own = line(index)?;
        let above = line(index - 1)?.ending;
        if !own.text.is_empty() || !fuse(above, own.ending) {
            return None;
        }
        let below = line(index + 1)
            .filter(|below| below.text.is_empty())
            .map_or(LineEnding::None, |below| below.ending);
        [
            above,
            LineEnding::Lf,
            LineEnding::CrLf,
            LineEnding::Cr,
            LineEnding::LfCr,
        ]
        .into_iter()
        .find(|ending| !fuse(above, *ending) && !fuse(*ending, below))
        .map(|ending| (index, ending))
    })
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
                assert_eq!(split_ending(lines, at(0, 1)), LineEnding::CrLf);
                assert_eq!(split_ending(lines, at(1, 0)), LineEnding::Lf);
                assert_eq!(split_ending(lines, at(2, 1)), LineEnding::Cr);
                // The last line has no ending of its own, so it borrows the
                // one above, whatever the edit starts on.
                assert_eq!(split_ending(lines, at(3, 4)), LineEnding::Cr);
                assert_eq!(split_ending(lines, at(3, 0)), LineEnding::Cr);
            });
            Content::new(kind, "one\r\ntwo").with_lines(|lines| {
                assert_eq!(split_ending(lines, at(1, 3)), LineEnding::CrLf);
            });
            for text in ["", "one line"] {
                Content::new(kind, text).with_lines(|lines| {
                    assert_eq!(split_ending(lines, at(0, 0)), LineEnding::Lf);
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
