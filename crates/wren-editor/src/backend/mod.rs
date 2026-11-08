//! The text engines behind [`Content`](crate::widget::Content).
//!
//! The widget and [`EditorView`](crate::EditorView) reach the engine only
//! through [`Backend`], so each engine is a drop-in for the others.

mod cosmic;
mod iced;

use std::borrow::Cow;

use ::iced::advanced::graphics::text::cosmic_text;
use ::iced::advanced::graphics::text::Editor as RawEditor;
use ::iced::advanced::text::editor::Editor as _;
use ::iced::advanced::text::{LineHeight, Wrapping};
use ::iced::widget::text_editor::{Action, Cursor, Line, LineEnding, Selection};
use ::iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size};

use crate::highlighter::MarkdownHighlighter;

/// Which text engine an editor runs on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// iced's own `text::Editor`, drawn with `fill_editor`.
    #[default]
    Iced,
    /// A cosmic-text editor owned by this crate, drawn with `fill_raw`.
    Cosmic,
}

impl BackendKind {
    pub const ALL: [Self; 2] = [Self::Iced, Self::Cosmic];
}

/// A document's text, cursor, and layout.
pub enum Backend {
    Iced(RawEditor),
    Cosmic(Box<cosmic::Engine>),
}

impl Backend {
    pub fn with_text(kind: BackendKind, text: &str) -> Self {
        match kind {
            BackendKind::Iced => Self::Iced(RawEditor::with_text(text)),
            BackendKind::Cosmic => Self::Cosmic(Box::new(cosmic::Engine::with_text(text))),
        }
    }

    pub fn kind(&self) -> BackendKind {
        match self {
            Self::Iced(_) => BackendKind::Iced,
            Self::Cosmic(_) => BackendKind::Cosmic,
        }
    }

    /// Replace the whole text. The cursor is clamped into the new text, and
    /// where the engine allows it the scroll position is kept.
    pub fn replace_text(&mut self, text: &str) {
        match self {
            Self::Iced(editor) => *editor = RawEditor::with_text(text),
            Self::Cosmic(engine) => engine.replace_text(text),
        }
    }

    pub fn perform(&mut self, action: Action) {
        match self {
            Self::Iced(editor) => editor.perform(action),
            Self::Cosmic(engine) => engine.perform(action),
        }
    }

    pub fn move_to(&mut self, cursor: Cursor) {
        match self {
            Self::Iced(editor) => editor.move_to(cursor),
            Self::Cosmic(engine) => engine.move_to(cursor),
        }
    }

    /// The cursor, with columns as byte offsets into the line.
    pub fn cursor(&self) -> Cursor {
        match self {
            Self::Iced(editor) => editor.cursor(),
            Self::Cosmic(engine) => engine.cursor(),
        }
    }

    /// The caret or the selection rectangles, relative to the text origin;
    /// `None` when the engine cannot place them.
    pub fn selection(&self) -> Option<Selection> {
        match self {
            Self::Iced(editor) => Some(editor.selection()),
            Self::Cosmic(engine) => engine.selection(),
        }
    }

    /// Where an input method should anchor its candidate window, relative to
    /// the text origin.
    pub fn ime_position(&self) -> Option<Point> {
        match self {
            Self::Iced(editor) => match editor.selection() {
                Selection::Caret(position) => Some(position),
                Selection::Range(ranges) => {
                    Some(ranges.first().cloned().unwrap_or_default().position())
                }
            },
            Self::Cosmic(engine) => engine.ime_position(),
        }
    }

    /// The selected text, with every line ending copied as LF.
    pub fn copy(&self) -> Option<String> {
        match self {
            Self::Iced(editor) => editor.copy(),
            Self::Cosmic(engine) => engine.copy(),
        }
    }

    pub fn line_count(&self) -> usize {
        match self {
            Self::Iced(editor) => editor.line_count(),
            Self::Cosmic(engine) => engine.line_count(),
        }
    }

    /// The size the text was last laid out in.
    pub fn bounds(&self) -> Size {
        match self {
            Self::Iced(editor) => editor.bounds(),
            Self::Cosmic(engine) => engine.bounds(),
        }
    }

    /// The shaped, scrolled text.
    pub fn buffer(&self) -> &cosmic_text::Buffer {
        match self {
            Self::Iced(editor) => editor.buffer(),
            Self::Cosmic(engine) => engine.buffer(),
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
            Self::Cosmic(_) => {}
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
            Self::Cosmic(engine) => {
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
            Self::Cosmic(engine) => {
                engine.highlight(font, highlighter, |highlight| highlight.to_format())
            }
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
            Self::Cosmic(engine) => engine.draw(renderer, position, color, clip_bounds),
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

#[cfg(test)]
mod tests {
    use ::iced::advanced::text::highlighter::Highlighter as _;
    use ::iced::widget::text_editor::{Edit, Motion, Position};

    use super::*;
    use crate::fixtures;
    use crate::highlighter::Settings;

    const TEXT: &str = "# Title\nshort\nwrapped words wrapped words wrapped words\n\n- [x] done 🐦";

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
        let (mut cosmic, mut cosmic_highlighter) = laid_out(BackendKind::Cosmic, TEXT, size);
        for (step, action) in steps.into_iter().enumerate() {
            iced.perform(action.clone());
            cosmic.perform(action);
            update(&mut iced, &mut iced_highlighter, size, fixtures::FONT);
            update(&mut cosmic, &mut cosmic_highlighter, size, fixtures::FONT);
            assert_eq!(iced.cursor(), cosmic.cursor(), "step {step}");
            let expected = iced.selection().unwrap();
            let actual = cosmic.selection().expect("a visible caret");
            assert!(
                close(&expected, &actual),
                "step {step}: {expected:?} vs {actual:?}"
            );
        }
    }

    #[test]
    fn an_off_screen_caret_has_no_rectangle_and_the_ime_anchors_at_the_near_edge() {
        let text = (0..60)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let size = Size::new(300.0, 120.0);
        let (mut cosmic, mut highlighter) = laid_out(BackendKind::Cosmic, &text, size);
        cosmic.perform(Action::Scroll { lines: 30 });
        update(&mut cosmic, &mut highlighter, size, fixtures::FONT);
        assert!(cosmic.selection().is_none());
        assert_eq!(cosmic.ime_position(), Some(Point::ORIGIN));

        cosmic.perform(Action::Move(Motion::DocumentEnd));
        update(&mut cosmic, &mut highlighter, size, fixtures::FONT);
        assert!(matches!(cosmic.selection(), Some(Selection::Caret(_))));
        cosmic.perform(Action::Scroll { lines: -60 });
        update(&mut cosmic, &mut highlighter, size, fixtures::FONT);
        assert!(cosmic.selection().is_none());
        assert_eq!(cosmic.ime_position(), Some(Point::new(0.0, 96.0)));
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
