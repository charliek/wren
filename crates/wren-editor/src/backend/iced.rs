//! `Backend::Iced`: iced's own `text::Editor`.

use iced::advanced::graphics::text::Editor as RawEditor;
use iced::advanced::text::editor::Editor as _;
use iced::advanced::text::Renderer as _;
use std::sync::Arc;

use iced::widget::text_editor::{Action, Cursor, Edit, Position};
use iced::{Color, Point, Rectangle, Renderer};

/// After a font change iced reshapes only the scrolled-to lines, and the
/// caret and IME queries panic on an unshaped cursor line (an off-screen
/// cursor, after scrolling or a text size change). A cursor move makes the
/// next update shape up to the cursor and scroll to it; cosmic-text ignores a
/// move to the same place, so step away first.
pub(super) fn reshape_to_cursor(editor: &mut RawEditor) {
    let cursor = editor.cursor();
    let elsewhere = if cursor.position.line == 0 && cursor.position.column == 0 {
        (editor.line_count() > 1).then_some(Position { line: 1, column: 0 })
    } else {
        Some(Position { line: 0, column: 0 })
    };
    if let Some(position) = elsewhere {
        editor.move_to(Cursor {
            position,
            selection: None,
        });
        editor.move_to(cursor);
    }
}

/// iced has no range edit, so select the range and paste over it.
pub(super) fn replace_range(editor: &mut RawEditor, start: Position, end: Position, text: &str) {
    editor.move_to(Cursor {
        position: end,
        selection: Some(start),
    });
    editor.perform(Action::Edit(Edit::Paste(Arc::new(text.to_owned()))));
}

pub(super) fn draw(
    editor: &RawEditor,
    renderer: &mut Renderer,
    position: Point,
    color: Color,
    clip_bounds: Rectangle,
) {
    renderer.fill_editor(editor, position, color, clip_bounds);
}
