//! The text engines behind [`Content`](crate::widget::Content).
//!
//! The widget and [`EditorView`](crate::EditorView) reach the engine only
//! through [`Backend`], so each engine is a drop-in for the others.

mod iced;

use std::borrow::Cow;

use ::iced::advanced::graphics::text::cosmic_text;
use ::iced::advanced::graphics::text::Editor as RawEditor;
use ::iced::advanced::text::editor::Editor as _;
use ::iced::advanced::text::{LineHeight, Wrapping};
use ::iced::widget::text_editor::{Action, Cursor, Line, LineEnding, Selection};
use ::iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size};

use crate::highlighter::MarkdownHighlighter;

/// A document's text, cursor, and layout.
pub(crate) enum Backend {
    /// iced's own `text::Editor`, drawn with `fill_editor`.
    Iced(RawEditor),
}

impl Backend {
    pub fn with_text(text: &str) -> Self {
        Self::Iced(RawEditor::with_text(text))
    }

    pub fn perform(&mut self, action: Action) {
        match self {
            Self::Iced(editor) => editor.perform(action),
        }
    }

    pub fn move_to(&mut self, cursor: Cursor) {
        match self {
            Self::Iced(editor) => editor.move_to(cursor),
        }
    }

    /// The cursor, with columns as byte offsets into the line.
    pub fn cursor(&self) -> Cursor {
        match self {
            Self::Iced(editor) => editor.cursor(),
        }
    }

    /// The caret or the selection rectangles, relative to the text origin;
    /// `None` when the engine cannot place them.
    pub fn selection(&self) -> Option<Selection> {
        match self {
            Self::Iced(editor) => Some(editor.selection()),
        }
    }

    /// Where an input method should anchor its candidate window, relative to
    /// the text origin.
    pub fn ime_position(&self) -> Option<Point> {
        match self.selection()? {
            Selection::Caret(position) => Some(position),
            Selection::Range(ranges) => {
                Some(ranges.first().cloned().unwrap_or_default().position())
            }
        }
    }

    /// The selected text, with every line ending copied as LF.
    pub fn copy(&self) -> Option<String> {
        match self {
            Self::Iced(editor) => editor.copy(),
        }
    }

    pub fn line_count(&self) -> usize {
        match self {
            Self::Iced(editor) => editor.line_count(),
        }
    }

    /// The size the text was last laid out in.
    pub fn bounds(&self) -> Size {
        match self {
            Self::Iced(editor) => editor.bounds(),
        }
    }

    /// The shaped, scrolled text.
    pub fn buffer(&self) -> &cosmic_text::Buffer {
        match self {
            Self::Iced(editor) => editor.buffer(),
        }
    }

    pub fn lines(&self) -> Lines<'_> {
        Lines(self.buffer())
    }

    /// Prepare for [`Backend::update`] to switch fonts.
    pub fn font_changing(&mut self) {
        match self {
            Self::Iced(editor) => iced::reshape_to_cursor(editor),
        }
    }

    /// Lay the text out for `bounds` and shape what is visible.
    pub fn update(
        &mut self,
        bounds: Size,
        font: Font,
        size: Pixels,
        line_height: LineHeight,
        wrapping: Wrapping,
        highlighter: &mut MarkdownHighlighter,
    ) {
        match self {
            Self::Iced(editor) => {
                editor.update(bounds, font, size, line_height, wrapping, highlighter);
            }
        }
    }

    /// Apply the highlighter's spans to the visible lines.
    pub fn highlight(&mut self, font: Font, highlighter: &mut MarkdownHighlighter) {
        match self {
            Self::Iced(editor) => {
                editor.highlight(font, highlighter, |highlight| highlight.to_format())
            }
        }
    }

    /// Draw the text at `position`, clipped to `clip_bounds`.
    pub fn draw(
        &self,
        renderer: &mut Renderer,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        match self {
            Self::Iced(editor) => iced::draw(editor, renderer, position, color, clip_bounds),
        }
    }
}

/// Borrowed lines of a document; nothing is copied.
#[derive(Clone, Copy)]
pub struct Lines<'a>(&'a cosmic_text::Buffer);

impl<'a> Lines<'a> {
    pub fn len(self) -> usize {
        self.0.lines.len()
    }

    pub fn get(self, index: usize) -> Option<Line<'a>> {
        self.0.lines.get(index).map(|line| Line {
            text: Cow::Borrowed(line.text()),
            ending: ending(line.ending()),
        })
    }

    pub fn iter(self) -> impl Iterator<Item = Line<'a>> {
        self.0.lines.iter().map(|line| Line {
            text: Cow::Borrowed(line.text()),
            ending: ending(line.ending()),
        })
    }
}

fn ending(ending: cosmic_text::LineEnding) -> LineEnding {
    match ending {
        cosmic_text::LineEnding::Lf => LineEnding::Lf,
        cosmic_text::LineEnding::CrLf => LineEnding::CrLf,
        cosmic_text::LineEnding::Cr => LineEnding::Cr,
        cosmic_text::LineEnding::LfCr => LineEnding::LfCr,
        cosmic_text::LineEnding::None => LineEnding::None,
    }
}
