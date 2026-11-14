//! Spell check's seam: the host's [`Proofreader`], the suggestions menu
//! ([`SpellMenu`]) and the events that drive it ([`SpellEvent`]).

use std::ops::Range;

use iced::Point;
#[cfg(feature = "test-support")]
use iced::Rectangle;

/// What the host gives the editor to find and correct mistakes. Line by line,
/// so the editor asks only about rows it draws.
pub trait Proofreader: Send + Sync {
    /// The mistakes of `line` that start and end inside `prose`, as byte
    /// ranges of `line`: sorted, apart, on character boundaries. Appends to
    /// `out`.
    fn marks(&self, line: &str, prose: &[Range<usize>], out: &mut Vec<Range<usize>>);
    /// Up to `limit` corrections for the mistake at `mark` in `line`, best
    /// first.
    fn suggestions(&self, line: &str, mark: Range<usize>, limit: usize) -> Vec<String>;
}

/// An input to the suggestions menu; see [`crate::EditorView::spell`].
/// `OpenAt` is relative to the text origin, as a click action's position is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpellEvent {
    /// Open the menu for the misspelled word at the caret.
    OpenAtCaret,
    /// Open the menu for the misspelled word under a right-click.
    OpenAt(Point),
    /// Move the selection by this many rows.
    Move(i32),
    /// The pointer is over this row.
    Hover(usize),
    /// Replace the word with this row's suggestion.
    Accept(usize),
    Close,
}

/// The open suggestions menu: the word it was opened for and what it offers.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellMenu {
    line: usize,
    mark: Range<usize>,
    word: String,
    items: Vec<String>,
    selected: Option<usize>,
}

impl SpellMenu {
    /// The line that holds the word.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The word's byte range in its line.
    pub fn mark(&self) -> Range<usize> {
        self.mark.clone()
    }

    /// The word as it was when the menu opened.
    pub fn word(&self) -> &str {
        &self.word
    }

    /// The suggestions, best first; empty when there are none.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// The selected row, if any row can be selected.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
}

/// The marks and the menu as the last frame drew them, for the host's
/// functional tests.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpellFrame {
    pub marks: Vec<DrawnMark>,
    /// The menu's bounds in window coordinates, when it was drawn.
    pub menu: Option<Rectangle>,
}

/// One mark of a drawn row.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnMark {
    pub line: usize,
    /// The word's byte range in its line.
    pub range: Range<usize>,
    /// Whether the zigzag was drawn, or hidden by the caret at the word's end.
    pub shown: bool,
    /// The zigzag's extent on each row, in window coordinates.
    pub rects: Vec<Rectangle>,
}
