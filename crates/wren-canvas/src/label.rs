//! The label editor's input (the plan's §5.9): what the widget makes of
//! the keys and the input method while a label is open.

use std::ops::Range;

/// One edit of the open label, stamped with the view's epoch in
/// [`crate::Message::Label`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelAction {
    /// Typed or committed text; a line break is `"\n"`.
    Insert(String),
    /// Move the caret, extending the selection when `select` is set.
    Move {
        motion: Motion,
        select: bool,
    },
    SelectAll,
    /// Erase the selection, or the character before the caret.
    Backspace,
    /// Erase the selection, or the character after the caret.
    Delete,
    /// Put the selected text on the clipboard.
    Copy,
    /// Put the selected text on the clipboard and erase it.
    Cut,
    /// Put the label back as it was when this edit began (§5.9 rule 3).
    Undo,
    /// Restore the typing [`LabelAction::Undo`] took back.
    Redo,
    /// The input method's composition so far, and its selection as a byte
    /// range of `text`; empty text ends it.
    Preedit {
        text: String,
        selection: Option<Range<usize>>,
    },
}

/// Where a caret move goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    Start,
    End,
}
