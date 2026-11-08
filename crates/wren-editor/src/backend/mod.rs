//! The text engines behind [`Content`](crate::widget::Content).
//!
//! The widget and [`EditorView`](crate::EditorView) reach the engine only
//! through [`Backend`], so each engine is a drop-in for the others.

mod iced;
mod owned;

use std::borrow::Cow;

use ::iced::advanced::graphics::text::cosmic_text::{self, Scroll};
use ::iced::advanced::graphics::text::Editor as RawEditor;
use ::iced::advanced::text::editor::Editor as _;
use ::iced::advanced::text::{LineHeight, Wrapping};
use ::iced::widget::text_editor::{Action, Cursor, Line, LineEnding, Position, Selection};
use ::iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size};

use crate::highlighter::MarkdownHighlighter;

/// Which text engine an editor runs on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// iced's own `text::Editor`, drawn with `fill_editor`.
    #[default]
    Iced,
    /// A cosmic-text editor owned by this crate, drawn with `fill_raw`.
    Owned,
}

impl BackendKind {
    pub const ALL: [Self; 2] = [Self::Iced, Self::Owned];
}

/// Where [`EditorView::scroll_to_line`](crate::EditorView::scroll_to_line)
/// puts its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// The line's first row at the top of the view.
    Top,
    /// The line's first row a quarter of the way down the view, so the lines
    /// above it show where it sits.
    NearTop,
}

/// A document's text, cursor, and layout.
pub enum Backend {
    Iced(RawEditor),
    Owned(Box<owned::Engine>),
}

impl Backend {
    pub fn with_text(kind: BackendKind, text: &str) -> Self {
        match kind {
            BackendKind::Iced => Self::Iced(RawEditor::with_text(text)),
            BackendKind::Owned => Self::Owned(Box::new(owned::Engine::with_text(text))),
        }
    }

    pub fn kind(&self) -> BackendKind {
        match self {
            Self::Iced(_) => BackendKind::Iced,
            Self::Owned(_) => BackendKind::Owned,
        }
    }

    /// Replace the whole text. The cursor is clamped into the new text, and
    /// where the engine allows it the scroll position is kept.
    pub fn replace_text(&mut self, text: &str) {
        match self {
            Self::Iced(editor) => *editor = RawEditor::with_text(text),
            Self::Owned(engine) => engine.replace_text(text),
        }
    }

    /// Replace the text between `start` and `end` with `text`, taken
    /// literally (its line endings included), and put the cursor after it.
    pub fn replace_range(&mut self, start: Position, end: Position, text: &str) {
        match self {
            Self::Iced(editor) => iced::replace_range(editor, start, end, text),
            Self::Owned(engine) => engine.replace_range(start, end, text),
        }
    }

    /// Where the view is scrolled.
    pub fn scroll(&self) -> Scroll {
        self.buffer().scroll()
    }

    /// Put the cursor at `cursor` and the view where a snapshot left it.
    /// Owned restores `scroll` exactly, then scrolls only as far as needed
    /// to show the cursor; iced keeps its scroll and reveals the cursor on
    /// its next update.
    pub fn restore_view(&mut self, cursor: Cursor, scroll: Scroll) {
        match self {
            Self::Iced(editor) => editor.move_to(cursor),
            Self::Owned(engine) => engine.restore_view(cursor, scroll),
        }
    }

    /// Put the cursor at `cursor` and the view back at `scroll`. Owned keeps
    /// `scroll` exactly, even when the cursor is out of view. iced scrolls
    /// back by whole rows until `scroll.line` is where it was, then moves the
    /// cursor; it settles that scroll in the same update that reveals a moved
    /// cursor, so a cursor above the view still ends up on the top row.
    pub fn return_to(&mut self, cursor: Cursor, scroll: Scroll) {
        match self {
            Self::Iced(editor) => {
                let line = scroll.line.min(editor.line_count().saturating_sub(1));
                iced::scroll_to(editor, line, -scroll.vertical);
                editor.move_to(cursor);
            }
            Self::Owned(engine) => engine.return_to(cursor, scroll),
        }
    }

    /// Scroll so `line` sits at `placement`. Near the end of the text the
    /// view stops at the last line rather than scroll past it. The cursor
    /// does not move; moving it onto `line` afterwards needs no more scroll.
    pub fn scroll_to_line(&mut self, line: usize, placement: Placement) {
        let line_height = self.buffer().metrics().line_height;
        let above = match placement {
            Placement::Top => 0.0,
            Placement::NearTop => (self.bounds().height / line_height / 4.0).floor() * line_height,
        };
        let line = line.min(self.line_count().saturating_sub(1));
        match self {
            Self::Iced(editor) => iced::scroll_to(editor, line, above),
            Self::Owned(engine) => engine.scroll_to(line, above),
        }
    }

    pub fn perform(&mut self, action: Action) {
        match self {
            Self::Iced(editor) => editor.perform(action),
            Self::Owned(engine) => engine.perform(action),
        }
    }

    pub fn move_to(&mut self, cursor: Cursor) {
        match self {
            Self::Iced(editor) => editor.move_to(cursor),
            Self::Owned(engine) => engine.move_to(cursor),
        }
    }

    /// The cursor, with columns as byte offsets into the line.
    pub fn cursor(&self) -> Cursor {
        match self {
            Self::Iced(editor) => editor.cursor(),
            Self::Owned(engine) => engine.cursor(),
        }
    }

    /// The caret or the selection rectangles, relative to the text origin;
    /// `None` when the engine cannot place them.
    pub fn selection(&self) -> Option<Selection> {
        match self {
            Self::Iced(editor) => Some(editor.selection()),
            Self::Owned(engine) => engine.selection(),
        }
    }

    /// Where an input method should anchor its candidate window, relative to
    /// the text origin: the caret, or the selection's first row. When that is
    /// out of view, the anchor keeps its column on the nearest view edge.
    pub fn ime_position(&self) -> Option<Point> {
        let anchor = match self {
            Self::Iced(editor) => match editor.selection() {
                Selection::Caret(position) => position,
                Selection::Range(ranges) => ranges.first().cloned().unwrap_or_default().position(),
            },
            Self::Owned(engine) => engine.ime_anchor()?,
        };
        let line_height = self.buffer().metrics().line_height;
        Some(clamp_ime_anchor(anchor, self.bounds(), line_height))
    }

    /// Scroll as little as needed to show the cursor, keeping the cursor
    /// and the selection. Owned scrolls now; iced on its next update.
    pub fn reveal_cursor(&mut self) {
        match self {
            Self::Iced(editor) => iced::reshape_to_cursor(editor),
            Self::Owned(engine) => engine.reveal_cursor(),
        }
    }

    /// The selected text, with every line ending copied as LF.
    pub fn copy(&self) -> Option<String> {
        match self {
            Self::Iced(editor) => editor.copy(),
            Self::Owned(engine) => engine.copy(),
        }
    }

    pub fn line_count(&self) -> usize {
        match self {
            Self::Iced(editor) => editor.line_count(),
            Self::Owned(engine) => engine.line_count(),
        }
    }

    /// The size the text was last laid out in.
    pub fn bounds(&self) -> Size {
        match self {
            Self::Iced(editor) => editor.bounds(),
            Self::Owned(engine) => engine.bounds(),
        }
    }

    /// The shaped, scrolled text.
    pub fn buffer(&self) -> &cosmic_text::Buffer {
        match self {
            Self::Iced(editor) => editor.buffer(),
            Self::Owned(engine) => engine.buffer(),
        }
    }

    pub fn lines(&self) -> Lines<'_> {
        Lines(self.buffer())
    }

    /// Prepare for [`Backend::update`] to switch fonts.
    pub fn font_changing(&mut self) {
        match self {
            Self::Iced(editor) => iced::reshape_to_cursor(editor),
            // The engine lays out the cursor line itself after every update.
            Self::Owned(_) => {}
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
            Self::Owned(engine) => {
                engine.update(bounds, font, size, line_height, wrapping, highlighter);
            }
        }
    }

    /// Apply the highlighter's spans to the visible lines.
    pub fn highlight(&mut self, font: Font, highlighter: &mut MarkdownHighlighter) {
        match self {
            Self::Iced(editor) => {
                editor.highlight(font, highlighter, |highlight| highlight.to_format())
            }
            Self::Owned(engine) => engine.highlight(font, highlighter, |highlight| {
                (highlight.to_format(), highlight.scale())
            }),
        }
    }

    /// Draw the text at `position`. tiny-skia does not honour `clip_bounds`
    /// for editor text, so the caller also clips with a layer.
    pub fn draw(
        &self,
        renderer: &mut Renderer,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        match self {
            Self::Iced(editor) => iced::draw(editor, renderer, position, color, clip_bounds),
            Self::Owned(engine) => engine.draw(renderer, position, color, clip_bounds),
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

/// The planner reads the buffer's lines in place.
impl wren_markdown::Lines for Lines<'_> {
    fn line_count(&self) -> usize {
        self.0.lines.len()
    }

    fn line(&self, i: usize) -> &str {
        self.0.lines[i].text()
    }
}

pub(crate) fn ending(ending: cosmic_text::LineEnding) -> LineEnding {
    match ending {
        cosmic_text::LineEnding::Lf => LineEnding::Lf,
        cosmic_text::LineEnding::CrLf => LineEnding::CrLf,
        cosmic_text::LineEnding::Cr => LineEnding::Cr,
        cosmic_text::LineEnding::LfCr => LineEnding::LfCr,
        cosmic_text::LineEnding::None => LineEnding::None,
    }
}

/// Keep an input method anchor inside a view of `view` size: a row above
/// the view moves to its top row and one below to its bottom row, keeping
/// its column, so the candidate window stays next to where the text goes.
pub(crate) fn clamp_ime_anchor(anchor: Point, view: Size, line_height: f32) -> Point {
    Point::new(
        anchor.x.clamp(0.0, view.width.max(0.0)),
        anchor.y.clamp(0.0, (view.height - line_height).max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use ::iced::advanced::text::highlighter::Highlighter as _;
    use ::iced::widget::text_editor::{Edit, Motion, Position};

    use super::*;
    use crate::fixtures;
    use crate::highlighter::Settings;

    // H4 is body sized, so both engines lay it out alike.
    const TEXT: &str =
        "#### Title\nshort\nwrapped words wrapped words wrapped words\n\n- [x] done 🐦";

    fn update(
        backend: &mut Backend,
        highlighter: &mut MarkdownHighlighter,
        size: Size,
        font: Font,
    ) {
        backend.update(
            size,
            font,
            Pixels(14.0),
            LineHeight::Absolute(Pixels(24.0)),
            Wrapping::WordOrGlyph,
            highlighter,
        );
        backend.highlight(font, highlighter);
    }

    fn laid_out(kind: BackendKind, text: &str, size: Size) -> (Backend, MarkdownHighlighter) {
        fixtures::load_font();
        let mut backend = Backend::with_text(kind, text);
        let mut highlighter = MarkdownHighlighter::new(&Settings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
            front_matter_end: None,
        });
        update(&mut backend, &mut highlighter, size, fixtures::FONT);
        (backend, highlighter)
    }

    fn close(a: &Selection, b: &Selection) -> bool {
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        match (a, b) {
            (Selection::Caret(a), Selection::Caret(b)) => near(a.x, b.x) && near(a.y, b.y),
            (Selection::Range(a), Selection::Range(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        near(a.x, b.x)
                            && near(a.y, b.y)
                            && near(a.width, b.width)
                            && near(a.height, b.height)
                    })
            }
            _ => false,
        }
    }

    #[test]
    fn caret_and_selection_geometry_matches_iced_on_wrapped_rows() {
        let size = Size::new(120.0, 400.0);
        let mut steps = vec![Action::Move(Motion::DocumentStart)];
        steps.extend(std::iter::repeat_n(Action::Move(Motion::Right), 60));
        steps.extend(std::iter::repeat_n(Action::Move(Motion::Down), 4));
        steps.extend(std::iter::repeat_n(Action::Move(Motion::Up), 4));
        steps.push(Action::Move(Motion::End));
        steps.push(Action::Move(Motion::Home));
        steps.push(Action::Select(Motion::DocumentEnd));
        steps.push(Action::Move(Motion::Left));
        steps.extend(std::iter::repeat_n(Action::Select(Motion::WordRight), 6));
        steps.push(Action::SelectAll);
        let (mut iced, mut iced_highlighter) = laid_out(BackendKind::Iced, TEXT, size);
        let (mut owned, mut owned_highlighter) = laid_out(BackendKind::Owned, TEXT, size);
        for (step, action) in steps.into_iter().enumerate() {
            iced.perform(action.clone());
            owned.perform(action);
            update(&mut iced, &mut iced_highlighter, size, fixtures::FONT);
            update(&mut owned, &mut owned_highlighter, size, fixtures::FONT);
            assert_eq!(iced.cursor(), owned.cursor(), "step {step}");
            let expected = iced.selection().unwrap();
            let actual = owned.selection().expect("a visible caret");
            assert!(
                close(&expected, &actual),
                "step {step}: {expected:?} vs {actual:?}"
            );
        }
    }

    #[test]
    fn an_off_screen_ime_anchor_keeps_its_column_on_the_nearest_edge() {
        let text = (0..60)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let size = Size::new(300.0, 120.0);
        let bottom = 120.0 - 24.0;
        for kind in BackendKind::ALL {
            let (mut backend, mut highlighter) = laid_out(kind, &text, size);
            let mut step = |backend: &mut Backend, action: Action| {
                backend.perform(action);
                update(backend, &mut highlighter, size, fixtures::FONT);
            };
            step(&mut backend, Action::Move(Motion::End));
            let Some(Selection::Caret(caret)) = backend.selection() else {
                panic!("{kind:?}: a visible caret");
            };
            assert!(caret.x > 0.0 && caret.y == 0.0, "{kind:?}: {caret:?}");
            assert_eq!(backend.ime_position(), Some(caret), "{kind:?}");
            step(&mut backend, Action::Scroll { lines: 30 });
            assert_eq!(
                backend.ime_position(),
                Some(Point::new(caret.x, 0.0)),
                "{kind:?}: above the view"
            );
            if kind == BackendKind::Owned {
                assert!(backend.selection().is_none());
            }

            step(&mut backend, Action::Move(Motion::DocumentEnd));
            let Some(Selection::Caret(caret)) = backend.selection() else {
                panic!("{kind:?}: a visible caret");
            };
            step(&mut backend, Action::Scroll { lines: -60 });
            assert_eq!(
                backend.ime_position(),
                Some(Point::new(caret.x, bottom)),
                "{kind:?}: below the view"
            );

            // Revealing the cursor brings the real caret back as the anchor.
            backend.reveal_cursor();
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            let Some(Selection::Caret(shown)) = backend.selection() else {
                panic!("{kind:?}: a revealed caret");
            };
            assert!(shown.y >= 0.0 && shown.y <= bottom, "{kind:?}: {shown:?}");
            assert_eq!(shown.x, caret.x, "{kind:?}");
            assert_eq!(backend.ime_position(), Some(shown), "{kind:?}");
            assert_eq!(backend.cursor().position.line, 59, "{kind:?}");
        }

        // A caret on the scroll line, in a wrapped row scrolled off above.
        let text = format!("{}\nend", "word ".repeat(400));
        for kind in BackendKind::ALL {
            let (mut backend, mut highlighter) = laid_out(kind, &text, size);
            backend.perform(Action::Scroll { lines: 10 });
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            let scroll = backend.scroll();
            assert!(
                scroll.line == 0 && scroll.vertical > 0.0,
                "{kind:?}: {scroll:?}"
            );
            assert_eq!(backend.cursor().position, Position { line: 0, column: 0 });
            assert_eq!(
                backend.ime_position(),
                Some(Point::ORIGIN),
                "{kind:?}: above the view"
            );
        }
    }

    #[test]
    fn reveal_keeps_the_selection_and_scrolls_only_when_needed() {
        let text = (0..60)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let size = Size::new(300.0, 120.0);
        for kind in BackendKind::ALL {
            let (mut backend, mut highlighter) = laid_out(kind, &text, size);
            backend.perform(Action::Select(Motion::Down));
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            let cursor = backend.cursor();
            let scroll = backend.scroll();
            backend.reveal_cursor();
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            assert_eq!(backend.cursor(), cursor, "{kind:?}");
            assert_eq!(backend.scroll(), scroll, "{kind:?}: already in view");

            backend.perform(Action::Scroll { lines: 40 });
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            backend.reveal_cursor();
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            assert_eq!(backend.cursor(), cursor, "{kind:?}");
            assert!(
                backend.scroll().line <= 1,
                "{kind:?}: {:?}",
                backend.scroll()
            );
        }
    }

    #[test]
    fn ime_anchors_clamp_to_the_view_rows_keeping_their_column() {
        let view = Size::new(300.0, 120.0);
        let at = |x, y| clamp_ime_anchor(Point::new(x, y), view, 24.0);
        assert_eq!(at(40.0, 48.0), Point::new(40.0, 48.0));
        assert_eq!(at(40.0, -500.0), Point::new(40.0, 0.0));
        assert_eq!(at(40.0, 96.0), Point::new(40.0, 96.0));
        assert_eq!(at(40.0, 97.0), Point::new(40.0, 96.0));
        assert_eq!(at(40.0, 5000.0), Point::new(40.0, 96.0));
        assert_eq!(at(400.0, 48.0), Point::new(300.0, 48.0));
        assert_eq!(at(-3.0, 48.0), Point::new(0.0, 48.0));
        // A view shorter than a row anchors on its top.
        let short = clamp_ime_anchor(Point::new(10.0, 50.0), Size::new(300.0, 10.0), 24.0);
        assert_eq!(short, Point::new(10.0, 0.0));
        let empty = clamp_ime_anchor(Point::new(10.0, 50.0), Size::ZERO, 24.0);
        assert_eq!(empty, Point::ORIGIN);
    }

    #[test]
    fn a_font_change_with_the_cursor_scrolled_away_does_not_panic() {
        let text = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let size = Size::new(300.0, 120.0);
        for kind in BackendKind::ALL {
            let (mut backend, mut highlighter) = laid_out(kind, &text, size);
            backend.perform(Action::Move(Motion::DocumentEnd));
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            backend.perform(Action::Scroll { lines: -200 });
            update(&mut backend, &mut highlighter, size, fixtures::FONT);
            backend.font_changing();
            update(&mut backend, &mut highlighter, size, Font::MONOSPACE);
            let _ = backend.selection();
            let _ = backend.ime_position();
            backend.perform(Action::Edit(Edit::Insert('!')));
            update(&mut backend, &mut highlighter, size, Font::MONOSPACE);
            assert!(
                matches!(backend.selection(), Some(Selection::Caret(_))),
                "{kind:?}"
            );
            assert_eq!(
                backend.cursor().position,
                Position {
                    line: 199,
                    column: 9
                }
            );
        }
    }

    #[test]
    fn both_engines_borrow_the_same_lines_and_endings() {
        let text = "a\r\nb\rc\n\rd\ne";
        for kind in BackendKind::ALL {
            let backend = Backend::with_text(kind, text);
            let lines = backend.lines();
            assert_eq!(lines.len(), 5, "{kind:?}");
            let endings: Vec<_> = lines.iter().map(|line| line.ending).collect();
            assert_eq!(
                endings,
                [
                    LineEnding::CrLf,
                    LineEnding::Cr,
                    LineEnding::LfCr,
                    LineEnding::Lf,
                    LineEnding::None
                ]
            );
            assert!(matches!(lines.get(2).unwrap().text, Cow::Borrowed("c")));
            assert!(lines.get(5).is_none());
        }
    }
}
