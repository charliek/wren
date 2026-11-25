//! What lies at a place in the text: one hit test for a misspelled word and
//! for the classifier's spans.
//!
//! Two sources say what a range of a line is. The classifier's typed spans
//! ([`wren_markdown::span`]) are static: a link, a code span, a task's box,
//! its due date. The misspellings are dynamic and the host's: the
//! [`Proofreader`](crate::Proofreader) it passes in marks them in the
//! line's prose, which is what the spans leave of the line. The classifier
//! knows no speller and the speller no spans; they meet here, as two lists
//! of ranges of one line, and [`resolve`] puts them in order:
//!
//! 1. A misspelled word comes first, so its suggestions are offered before
//!    anything the span it lies in has to offer: an owner, a tag or a link
//!    whose text is misspelled is a misspelling.
//! 2. Otherwise the narrowest span that holds the place, which is the
//!    innermost where spans nest ([`LineSpans::at`]). It is looked for
//!    only when no word is there.
//!
//! The place is a byte column ([`at`]: the character that starts there) or
//! a point of the laid out text (the widget's `place_under`). Under a
//! point, a misspelled word is hit across the extent of its glyphs on the
//! row, as its mark is drawn, and a span by the one glyph under the point:
//! in text of two directions a span's glyphs can lie apart, with a glyph
//! of no span between them. Whatever acts on a place, a right-click's menu
//! or a click, reads its `Hit` and takes it or leaves it to the next in
//! line.
//!
//! A right-click's providers are asked about the whole place ([`Clicked`]):
//! the line, its spans, the glyph under the pointer and the hit there, so
//! that one of them can take a place that is no hit at all. The widget and
//! the editor both read it from the lines themselves ([`Reading`]), never
//! from the widget's highlighter, so that the two ask a provider about the
//! same thing.

use std::ops::Range;

use iced::Point;
use wren_markdown::span::{LineSpans, Span, SpanKind};
use wren_markdown::TaskOptions;

use crate::popover::{Clicked, TaskBox};
use crate::proof::{self, Proofreader};
use crate::widget::Content;

/// What a place in a line is: a misspelled word, or else the innermost of
/// the line's typed spans. Spelling comes first, so a misspelled owner, tag
/// or link text is a misspelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    /// A misspelled word, as the host's proofreader marked it: its byte
    /// range in the line.
    Misspelling(Range<usize>),
    /// The innermost of the classifier's spans there.
    Span(Span),
}

impl Hit {
    /// The bytes of the line the hit covers.
    pub fn range(&self) -> &Range<usize> {
        match self {
            Self::Misspelling(range) => range,
            Self::Span(span) => &span.range,
        }
    }

    /// The misspelled word's range, when the hit is one.
    pub fn misspelling(self) -> Option<Range<usize>> {
        match self {
            Self::Misspelling(range) => Some(range),
            Self::Span(_) => None,
        }
    }
}

/// The hit at a place where `mark` is the misspelled word, if one is
/// there, and `span` finds the classifier's span: the word, or else the
/// span, which is not looked for under a word.
pub(crate) fn resolve(
    mark: Option<Range<usize>>,
    span: impl FnOnce() -> Option<Span>,
) -> Option<Hit> {
    match mark {
        Some(mark) => Some(Hit::Misspelling(mark)),
        None => span().map(Hit::Span),
    }
}

/// How the lines are read for a hit test made without the widget's
/// highlighter: by a fresh scan of the lines down to the one asked about
/// (§3.3), with the editor's settings.
#[derive(Clone, Copy)]
pub(crate) struct Reading<'a> {
    /// The host's proofreader, on the engine that checks spelling.
    pub speller: Option<&'a dyn Proofreader>,
    /// The text's `block::front_matter_end`.
    pub front_matter_end: Option<usize>,
    /// The host's tasks, when they are on.
    pub tasks: Option<TaskOptions>,
}

impl Reading<'_> {
    /// The two lists a hit test reads for `line` of `content`: the
    /// misspellings, the same marks a frame draws there, and the typed
    /// spans, whose prose the proofreader was asked about. Both empty past
    /// the text.
    pub(crate) fn ranges(self, content: &Content, line: usize) -> (Vec<Range<usize>>, LineSpans) {
        content.with_lines(|lines| {
            let Some(text) = lines.get(line) else {
                return Default::default();
            };
            let spans =
                crate::highlighter::spans_at(&lines, line, self.front_matter_end, self.tasks);
            let marks = self
                .speller
                .map(|proofreader| proof::line_marks(proofreader, &text.text, &spans.prose()))
                .unwrap_or_default();
            (marks, spans)
        })
    }

    /// What a right-click at `point`, relative to the text origin, falls
    /// on, as `content` is laid out. `None` off every row of the text.
    pub(crate) fn clicked(self, content: &Content, point: Point) -> Option<Clicked> {
        let mut spans = LineSpans::default();
        let place = content.place_under(point, |line, _| {
            let (marks, read) = self.ranges(content, line);
            spans = read;
            marks
        })?;
        let text =
            content.with_lines(|lines| lines.get(place.line).map(|line| line.text.into_owned()))?;
        let hit = resolve(place.mark, || spans.at(place.column?).cloned());
        Some(Clicked {
            line: place.line,
            text,
            spans,
            column: place.column,
            hit,
        })
    }
}

/// The hit at the character that starts at byte `column` of a line with
/// these misspellings (`marks`) and `spans`.
/// Whether the character that starts at byte `column` lies in a task's box,
/// `[` to `]`, among a line's `spans`: what a click on the box is.
pub(crate) fn on_box(spans: &LineSpans, column: usize) -> bool {
    spans
        .spans
        .iter()
        .any(|span| matches!(span.kind, SpanKind::Box(_)) && span.range.contains(&column))
}

impl Reading<'_> {
    /// [`Self::clicked`], with the task the clicked line belongs to when
    /// it is a line of the task's text after its box.
    pub(crate) fn clicked_in(
        self,
        content: &Content,
        point: Point,
    ) -> Option<(Clicked, Option<TaskBox>)> {
        let clicked = self.clicked(content, point)?;
        let task = self.task_box(content, &clicked);
        Some((clicked, task))
    }

    /// The box line of the task whose text `clicked`'s line continues, as
    /// the whole text reads it: none on a line with a box of its own, off
    /// every task, and with the host's tasks off.
    pub(crate) fn task_box(self, content: &Content, clicked: &Clicked) -> Option<TaskBox> {
        let tasks = self.tasks?;
        if (clicked.spans.spans.iter()).any(|span| matches!(span.kind, SpanKind::Box(_))) {
            return None;
        }
        let options = wren_markdown::Options {
            indent_width: 4,
            renumber: true,
            tasks: Some(tasks),
        };
        let caret = wren_markdown::Selection::caret(wren_markdown::Pos::new(clicked.line, 0));
        let nothing = wren_markdown::task::Change::Compound(Vec::new());
        content.with_lines(|lines| {
            let edit = wren_markdown::task::edit(&lines, clicked.line, caret, &nothing, &options)?;
            let line = edit.line;
            let text = lines.get(line)?.text.into_owned();
            let spans =
                crate::highlighter::spans_at(&lines, line, self.front_matter_end, self.tasks);
            (line != clicked.line).then_some(TaskBox { line, text, spans })
        })
    }
}

pub(crate) fn at(column: usize, marks: &[Range<usize>], spans: &LineSpans) -> Option<Hit> {
    let mark = marks.iter().find(|mark| mark.contains(&column));
    resolve(mark.cloned(), || spans.at(column).cloned())
}

#[cfg(test)]
mod tests {
    use wren_markdown::block::{Boxes, Scanner};
    use wren_markdown::span::{self, SpanKind};

    use super::*;

    const LINE: &str = "- [ ] Call @sam [plna](u) #home 🆔 a1";

    /// The spans of [`LINE`], a task line of a host with tasks on.
    fn spans() -> LineSpans {
        let info = Scanner::new(Boxes::Status).next_line(LINE);
        span::spans(LINE, &info, Boxes::Status)
    }

    /// The text a hit covers, and `true` for a misspelling.
    fn text(hit: Option<Hit>) -> Option<(&'static str, bool)> {
        hit.map(|hit| {
            (
                &LINE[hit.range().clone()],
                matches!(hit, Hit::Misspelling(_)),
            )
        })
    }

    #[test]
    fn a_misspelling_comes_before_the_span_it_lies_in() {
        let spans = spans();
        // A speller that marks the owner's name, the link's text and the
        // tag's name.
        let marks = [12..15, 17..21, 27..31];
        for (column, word, span) in [
            (12, "sam", "@sam"),
            (14, "sam", "@sam"),
            (17, "plna", "[plna](u)"),
            (20, "plna", "[plna](u)"),
            (27, "home", "#home"),
            (30, "home", "#home"),
        ] {
            assert_eq!(
                text(at(column, &marks, &spans)),
                Some((word, true)),
                "{column}"
            );
            assert_eq!(
                text(at(column, &[], &spans)),
                Some((span, false)),
                "{column}"
            );
        }
        // Beside the word, the span is still there to be hit.
        for (column, span) in [
            (11, "@sam"),
            (16, "[plna](u)"),
            (21, "[plna](u)"),
            (26, "#home"),
        ] {
            assert_eq!(
                text(at(column, &marks, &spans)),
                Some((span, false)),
                "{column}"
            );
        }
    }

    #[test]
    fn a_column_in_a_task_token_is_that_token() {
        let spans = spans();
        let kind = |column: usize| match at(column, &[], &spans) {
            Some(Hit::Span(span)) => Some(span.kind),
            other => panic!("column {column}: {other:?}"),
        };
        assert!(matches!(kind(2), Some(SpanKind::Box(task)) if task.symbol == ' '));
        assert_eq!(kind(4).map(|kind| kind.is_task()), Some(true));
        assert_eq!(kind(13), Some(SpanKind::Owner { name: 12..15 }));
        assert_eq!(kind(28), Some(SpanKind::Tag));
        assert_eq!(kind(32), Some(SpanKind::Id { id: 37..39 }));
        assert_eq!(kind(38), Some(SpanKind::Id { id: 37..39 }));
    }

    #[test]
    fn a_column_outside_every_range_is_no_hit() {
        let spans = spans();
        for column in [0, 1, 5, 6, 9, 10, 15, 25, 31, LINE.len(), LINE.len() + 4] {
            assert_eq!(at(column, &[12..15, 17..21], &spans), None, "{column}");
        }
        // A line with no span has its misspellings still.
        let bare = LineSpans::default();
        assert_eq!(at(3, &[0..4, 5..9], &bare), Some(Hit::Misspelling(0..4)));
        assert_eq!(at(4, &[0..4, 5..9], &bare), None);
        assert_eq!(at(9, &[0..4, 5..9], &bare), None);
    }

    #[test]
    fn a_span_is_not_looked_for_under_a_word() {
        let span = spans().at(13).cloned();
        assert_eq!(
            resolve(Some(12..15), || panic!("looked for a span under a word")),
            Some(Hit::Misspelling(12..15))
        );
        assert_eq!(resolve(None, || span.clone()), span.map(Hit::Span));
        assert_eq!(resolve(None, || None), None);
    }
}
