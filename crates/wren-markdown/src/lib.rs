//! Markdown structure and editing rules for plain-text editors.
//!
//! The crate knows nothing about a UI toolkit. An editor hands it the
//! document's lines through [`Lines`] and gets back line classifications
//! ([`block`]), a heading outline ([`outline`]), and, for each editing
//! [`Intent`], an [`EditPlan`] of replacements to apply.
//!
//! Positions are `(line, byte column)` pairs. Line text never includes its
//! line ending, and inserted text always uses `'\n'`; the editor translates
//! it to the ending of the line being split.

use std::ops::Range;

pub mod block;
pub mod edit;
pub mod outline;

pub use edit::plan;

/// Random access to a document's lines.
///
/// `line(i)` is the text of line `i` without its line ending. A document
/// always has at least one line, and a trailing line ending is followed by
/// an empty last line, matching how editors present text.
pub trait Lines {
    fn line_count(&self) -> usize;
    fn line(&self, i: usize) -> &str;
}

impl Lines for [&str] {
    fn line_count(&self) -> usize {
        self.len()
    }

    fn line(&self, i: usize) -> &str {
        self[i]
    }
}

impl<const N: usize> Lines for [&str; N] {
    fn line_count(&self) -> usize {
        N
    }

    fn line(&self, i: usize) -> &str {
        self[i]
    }
}

impl<T: Lines + ?Sized> Lines for &T {
    fn line_count(&self) -> usize {
        (**self).line_count()
    }

    fn line(&self, i: usize) -> &str {
        (**self).line(i)
    }
}

/// How a line ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
    LfCr,
    /// The last line, which has no ending.
    None,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
            Self::LfCr => "\n\r",
            Self::None => "",
        }
    }
}

/// A whole document held as one string, split on `\n`, `\r\n`, `\r`, and
/// `\n\r`.
///
/// The split follows cosmic-text's line iterator exactly (`\r\n` is tried
/// before `\n\r`), so line numbers agree with the editor buffer.
#[derive(Clone, Debug)]
pub struct Document<'a> {
    text: &'a str,
    lines: Vec<(Range<usize>, LineEnding)>,
}

impl<'a> Document<'a> {
    pub fn new(text: &'a str) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < bytes.len() {
            let ending = match (bytes[i], bytes.get(i + 1)) {
                (b'\r', Some(b'\n')) => LineEnding::CrLf,
                (b'\n', Some(b'\r')) => LineEnding::LfCr,
                (b'\n', _) => LineEnding::Lf,
                (b'\r', _) => LineEnding::Cr,
                _ => {
                    i += 1;
                    continue;
                }
            };
            lines.push((start..i, ending));
            i += ending.as_str().len();
            start = i;
        }
        lines.push((start..bytes.len(), LineEnding::None));
        Self { text, lines }
    }

    pub fn text(&self) -> &'a str {
        self.text
    }

    pub fn ending(&self, i: usize) -> LineEnding {
        self.lines[i].1
    }

    /// The byte range of line `i` in the whole text, without its ending.
    pub fn line_range(&self, i: usize) -> Range<usize> {
        self.lines[i].0.clone()
    }

    /// The byte offset of `pos` in the whole text.
    pub fn offset(&self, pos: Pos) -> usize {
        self.lines[pos.line].0.start + pos.col
    }
}

impl Lines for Document<'_> {
    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, i: usize) -> &str {
        &self.text[self.lines[i].0.clone()]
    }
}

/// A position: a line index and a UTF-8 byte offset into that line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// A selection. `head` is where the cursor is; `anchor` is the fixed end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub const fn new(anchor: Pos, head: Pos) -> Self {
        Self { anchor, head }
    }

    /// An empty selection: just a cursor.
    pub const fn caret(pos: Pos) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn start(&self) -> Pos {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> Pos {
        self.anchor.max(self.head)
    }
}

/// Replace the text between `start` and `end` with `text`. Newlines in
/// `text` are always `'\n'`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Replace {
    pub start: Pos,
    pub end: Pos,
    pub text: String,
}

/// The result of planning an [`Intent`].
///
/// `edits` are in document order, do not overlap, and are all expressed in
/// positions of the text *before* the plan; apply them last to first.
/// `selection` is expressed in positions of the text *after* the plan.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EditPlan {
    pub edits: Vec<Replace>,
    pub selection: Selection,
}

/// An editing request that depends on the Markdown around the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intent {
    /// Break the line, continuing or exiting lists and quotes.
    Enter,
    /// Break the line inside the current item, without a new marker.
    ShiftEnter,
    Tab,
    ShiftTab,
    Backspace,
    /// Delete one grapheme cluster forward.
    Delete,
    /// Delete back to the previous Unicode word boundary.
    DeleteWordBackward,
    /// Delete forward to the next Unicode word boundary.
    DeleteWordForward,
    /// Delete back to the line's text start, or from there to column 0.
    DeleteToTextStart,
    /// The smart Home cycle. `row_start` is the byte column where the
    /// cursor's visual row begins when that row is a wrapped continuation,
    /// which only the editor's layout knows.
    Home {
        extend: bool,
        row_start: Option<usize>,
    },
    DocumentStart {
        extend: bool,
    },
    DocumentEnd {
        extend: bool,
    },
    ToggleTask,
    Bold,
    Italic,
    Code,
    MoveLineUp,
    MoveLineDown,
}

/// Planner settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Options {
    /// Spaces per indent step outside lists, and between tab stops.
    pub indent_width: usize,
    /// Whether edits renumber ordered lists.
    pub renumber: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            indent_width: block::TAB_STOP,
            renumber: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(text: &str) -> Vec<(&str, LineEnding)> {
        let doc = Document::new(text);
        (0..doc.line_count())
            .map(|i| (&text[doc.line_range(i)], doc.ending(i)))
            .collect()
    }

    #[test]
    fn document_splits_all_four_endings_like_cosmic_text() {
        use LineEnding::*;
        assert_eq!(
            split("LF\nCRLF\r\nCR\rLFCR\n\rNONE"),
            [
                ("LF", Lf),
                ("CRLF", CrLf),
                ("CR", Cr),
                ("LFCR", LfCr),
                ("NONE", None)
            ]
        );
    }

    #[test]
    fn document_always_has_a_last_line_without_ending() {
        use LineEnding::*;
        assert_eq!(split(""), [("", None)]);
        assert_eq!(split("a"), [("a", None)]);
        assert_eq!(split("a\n"), [("a", Lf), ("", None)]);
        assert_eq!(split("\r\n"), [("", CrLf), ("", None)]);
        assert_eq!(split("\n\n"), [("", Lf), ("", Lf), ("", None)]);
    }

    #[test]
    fn document_prefers_crlf_over_lfcr() {
        use LineEnding::*;
        // "\n\r\n" is LF-CR then a lone LF, and "\r\n\r" is CR-LF then CR.
        assert_eq!(split("a\n\r\nb"), [("a", LfCr), ("", Lf), ("b", None)]);
        assert_eq!(split("a\r\n\rb"), [("a", CrLf), ("", Cr), ("b", None)]);
        assert_eq!(split("a\r\rb"), [("a", Cr), ("", Cr), ("b", None)]);
        assert_eq!(split("a\n\n\rb"), [("a", Lf), ("", LfCr), ("b", None)]);
    }

    #[test]
    fn document_offsets_and_ranges() {
        let doc = Document::new("ab\r\ncdé\ne");
        assert_eq!(doc.line_range(1), 4..8);
        assert_eq!(doc.offset(Pos::new(1, 2)), 6);
        assert_eq!(doc.offset(Pos::new(2, 0)), 9);
        assert_eq!(doc.text(), "ab\r\ncdé\ne");
    }

    #[test]
    fn slices_and_arrays_are_lines() {
        fn count<L: Lines + ?Sized>(lines: &L) -> (usize, String) {
            (
                lines.line_count(),
                lines.line(lines.line_count() - 1).into(),
            )
        }
        let v: Vec<&str> = "a b".split(' ').collect();
        assert_eq!(count(&v[..]), (2, "b".into()));
        assert_eq!(count(&["x"]), (1, "x".into()));
        assert_eq!(count(&&v[..]), (2, "b".into()));
    }

    #[test]
    fn selection_orders_its_ends() {
        let sel = Selection::new(Pos::new(2, 1), Pos::new(1, 5));
        assert_eq!(sel.start(), Pos::new(1, 5));
        assert_eq!(sel.end(), Pos::new(2, 1));
        assert!(!sel.is_empty());
        assert!(Selection::caret(Pos::new(3, 0)).is_empty());
    }
}
