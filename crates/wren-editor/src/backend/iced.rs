//! `Backend::Iced`: iced's own `text::Editor`.

use iced::advanced::graphics::text::Editor as RawEditor;
use iced::advanced::text::editor::Editor as _;
use iced::advanced::text::Renderer as _;
use std::sync::Arc;

use iced::widget::text_editor::{Action, Cursor, Edit, Position, Selection};
use iced::{Color, Point, Rectangle, Renderer};

/// After a font change iced reshapes only the scrolled-to lines, and the
/// caret and IME queries panic on an unshaped cursor line (an off-screen
/// cursor, after scrolling or a text size change). A cursor move makes the
/// next update shape up to the cursor and scroll to it; cosmic-text ignores a
/// move to the same place, so step away first.
pub(super) fn reshape_to_cursor(editor: &mut RawEditor) {
    let cursor = editor.cursor();
    let elsewhere = if cursor.position.line == 0 && cursor.position.column == 0 {
        if editor.line_count() > 1 {
            Some(Position { line: 1, column: 0 })
        } else {
            // One wrapped line can still scroll its start out of view.
            editor
                .line(0)
                .map(|line| line.text.len())
                .filter(|&end| end > 0)
                .map(|column| Position { line: 0, column })
        }
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

/// iced's caret query panics when the cursor line has no layout, which an
/// edit leaves on a line out of view until the view reaches it.
pub(super) fn selection(editor: &RawEditor) -> Option<Selection> {
    let cursor = editor.cursor();
    let laid_out = || {
        editor
            .buffer()
            .lines
            .get(cursor.position.line)
            .is_some_and(|line| line.layout_opt().is_some())
    };
    (cursor.selection.is_some() || laid_out()).then(|| editor.selection())
}

/// iced has no range edit, so select the range and paste over it.
pub(super) fn replace_range(editor: &mut RawEditor, start: Position, end: Position, text: &str) {
    editor.move_to(Cursor {
        position: end,
        selection: Some(start),
    });
    editor.perform(Action::Edit(Edit::Paste(Arc::new(text.to_owned()))));
}

/// iced scrolls only relative to where it is, in whole rows, so count the
/// rows from the top of the view to `above` pixels over `line`. A line not yet
/// laid out counts as one row, which is exact unless it wraps.
pub(super) fn scroll_to(editor: &mut RawEditor, line: usize, above: f32) {
    let buffer = editor.buffer();
    let line_height = buffer.metrics().line_height;
    let scroll = buffer.scroll();
    let rows = |lines: std::ops::Range<usize>| -> f32 {
        buffer.lines[lines]
            .iter()
            .map(|line| line.layout_opt().map_or(1, Vec::len) as f32)
            .sum()
    };
    let to_line = if line >= scroll.line {
        rows(scroll.line..line)
    } else {
        -rows(line..scroll.line)
    } * line_height;
    let pixels = to_line - scroll.vertical - above;
    editor.perform(Action::Scroll {
        lines: (pixels / line_height).round() as i32,
    });
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
