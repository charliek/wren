//! `Backend::Owned`: a cosmic-text editor that Wren owns (users see it as `editor-engine = advanced`).
//!
//! Ported from `iced_graphics` 0.14.0, `src/text/editor.rs`
//! (<https://github.com/iced-rs/iced>, commit
//! 3997291f318a8bc06fa522f5579836fb3feb94df), used under the MIT License:
//!
//! > Copyright 2019 Héctor Ramón, Iced contributors
//! >
//! > Permission is hereby granted, free of charge, to any person obtaining a
//! > copy of this software and associated documentation files (the
//! > "Software"), to deal in the Software without restriction, including
//! > without limitation the rights to use, copy, modify, merge, publish,
//! > distribute, sublicense, and/or sell copies of the Software, and to permit
//! > persons to whom the Software is furnished to do so, subject to the
//! > following conditions:
//! >
//! > The above copyright notice and this permission notice shall be included
//! > in all copies or substantial portions of the Software.
//! >
//! > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! > OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! > MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
//! > NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! > DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//! > OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
//! > USE OR OTHER DEALINGS IN THE SOFTWARE.
//!
//! The caret's offset within a glyph cluster follows `cursor_glyph_opt` and
//! `cursor_position` in cosmic-text 0.15.0, `src/edit/editor.rs`
//! (<https://github.com/pop-os/cosmic-text>, commit
//! c82ee1c5b5b8032e91eaff1cb34294b538727a7d, Copyright 2022 System76), used
//! under the MIT License, which cosmic-text offers alongside Apache-2.0.
//!
//! Modified by Wren: the editor is owned rather than hidden behind iced's
//! `text::Editor`; the buffer sits in an `Arc` so the widget can hand the
//! renderer a weak reference (`fill_raw`); the caret, selection, and IME
//! rectangles come from the visible layout runs and are `None` instead of a
//! panic when the cursor is not laid out; the font-system lock is taken once
//! per entry point and survives poisoning; highlight bounds tolerate
//! unshaped lines; edits accumulate the topmost changed line instead of
//! overwriting it; `replace_text` keeps the scroll position; and the engine,
//! not cosmic-text's editor, owns the goal column of vertical motions, so it
//! can forget it on every other motion, click, and edit.

use std::sync::{Arc, PoisonError, RwLockWriteGuard, Weak};

use iced::advanced::graphics::text::{
    self as graphics, cosmic_text, FontSystem, Raw, Renderer as _, Version,
};
use iced::advanced::text::editor::Direction;
use iced::advanced::text::highlighter::{self, Highlighter};
use iced::advanced::text::{LineHeight, Wrapping};
use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position, Selection};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size};

use cosmic_text::Edit as _;

use crate::highlighter::Scale;

pub struct Engine {
    editor: cosmic_text::Editor<'static>,
    font: Font,
    bounds: Size,
    /// The first line whose highlighting is stale; the next update tells the
    /// highlighter to resume there.
    topmost_line_changed: Option<usize>,
    version: Version,
    /// The x that Up, Down, PageUp, and PageDown aim for, kept across a run
    /// of them so a short line does not pull the column in.
    goal_x: Option<i32>,
}

impl Engine {
    pub fn with_text(text: &str) -> Self {
        let mut font_system = font_system();
        let mut buffer = cosmic_text::Buffer::new_empty(cosmic_text::Metrics {
            font_size: 1.0,
            line_height: 1.0,
        });
        buffer.set_text(
            font_system.raw(),
            text,
            &cosmic_text::Attrs::new(),
            cosmic_text::Shaping::Advanced,
            None,
        );
        Self {
            editor: cosmic_text::Editor::new(Arc::new(buffer)),
            font: Font::default(),
            bounds: Size::ZERO,
            topmost_line_changed: Some(0),
            version: font_system.version(),
            goal_x: None,
        }
    }

    /// Replace the whole text, keeping the scroll position and clamping the
    /// cursor into the new text.
    pub fn replace_text(&mut self, text: &str) {
        self.debug_assert_unshared();
        let mut font_system = font_system();
        let attributes = graphics::to_attributes(self.font);
        let cursor = self.editor.cursor();
        let buffer = buffer_mut(&mut self.editor);
        let scroll = buffer.scroll();
        buffer.set_text(
            font_system.raw(),
            text,
            &attributes,
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.set_scroll(scroll);
        let cursor = clamp(buffer, cursor);
        self.editor.set_selection(cosmic_text::Selection::None);
        self.editor.set_cursor(cursor);
        self.goal_x = None;
        self.mark_changed(0);
    }

    /// Replace the text between `start` and `end` with `text`, taken
    /// literally, and put the cursor after it.
    pub fn replace_range(&mut self, start: Position, end: Position, text: &str) {
        self.debug_assert_unshared();
        let start = to_cursor(start);
        self.editor.set_selection(cosmic_text::Selection::None);
        self.editor.delete_range(start, to_cursor(end));
        let after = self.editor.insert_at(start, text, None);
        self.editor.set_cursor(after);
        self.goal_x = None;
        self.mark_changed(start.line);
    }

    /// Put the cursor at `cursor` and the view at `scroll`, then scroll only
    /// as far as needed to show the cursor.
    pub fn restore_view(&mut self, cursor: Cursor, scroll: cosmic_text::Scroll) {
        self.move_to(cursor);
        let cursor = self.editor.cursor();
        let laid_out = self.bounds != Size::ZERO;
        let mut font_system = font_system();
        let buffer = buffer_mut(&mut self.editor);
        buffer.set_scroll(scroll);
        // Before the first layout there is no view to keep the cursor in.
        if laid_out {
            buffer.shape_until_cursor(font_system.raw(), cursor, false);
        }
    }

    /// Put the cursor at `cursor` and the view at `scroll`, keeping that view
    /// even when it does not show the cursor.
    pub fn return_to(&mut self, cursor: Cursor, scroll: cosmic_text::Scroll) {
        self.move_to(cursor);
        let laid_out = self.bounds != Size::ZERO;
        let mut font_system = font_system();
        // Shaping now settles the move, which would otherwise scroll to the
        // cursor on the next update.
        if laid_out {
            self.editor.shape_as_needed(font_system.raw(), false);
        }
        let buffer = buffer_mut(&mut self.editor);
        buffer.set_scroll(scroll);
        if laid_out {
            buffer.shape_until_scroll(font_system.raw(), false);
        }
    }

    /// Put `line`'s first row `above` pixels below the top of the view,
    /// stopping at the end of the text.
    pub fn scroll_to(&mut self, line: usize, above: f32) {
        self.debug_assert_unshared();
        let laid_out = self.bounds != Size::ZERO;
        let mut font_system = font_system();
        let buffer = buffer_mut(&mut self.editor);
        buffer.set_scroll(cosmic_text::Scroll::new(line, -above, 0.0));
        // Shaping settles the negative offset onto the lines above and keeps
        // the view inside the text; before the first layout the update does.
        if laid_out {
            buffer.shape_until_scroll(font_system.raw(), false);
        }
    }

    pub fn buffer(&self) -> &cosmic_text::Buffer {
        buffer(&self.editor)
    }

    pub fn line_count(&self) -> usize {
        self.buffer().lines.len()
    }

    pub fn copy(&self) -> Option<String> {
        self.editor.copy_selection()
    }

    pub fn bounds(&self) -> Size {
        self.bounds
    }

    pub fn cursor(&self) -> Cursor {
        let cursor = self.editor.cursor();
        let selection = match self.editor.selection() {
            cosmic_text::Selection::None => None,
            cosmic_text::Selection::Normal(anchor)
            | cosmic_text::Selection::Line(anchor)
            | cosmic_text::Selection::Word(anchor) => Some(Position {
                line: anchor.line,
                column: anchor.index,
            }),
        };
        Cursor {
            position: Position {
                line: cursor.line,
                column: cursor.index,
            },
            selection,
        }
    }

    pub fn perform(&mut self, action: Action) {
        self.debug_assert_unshared();
        let mut font_system = font_system();
        let font_system = font_system.raw();
        let editor = &mut self.editor;
        let goal_x = &mut self.goal_x;
        if !matches!(
            action,
            Action::Move(_) | Action::Select(_) | Action::Scroll { .. }
        ) {
            *goal_x = None;
        }

        match action {
            Action::Move(motion) => {
                if let Some((start, end)) = editor.selection_bounds() {
                    editor.set_selection(cosmic_text::Selection::None);
                    match motion {
                        // These motions apply as-is even with a selection.
                        Motion::Home
                        | Motion::End
                        | Motion::DocumentStart
                        | Motion::DocumentEnd => {
                            move_cursor(editor, goal_x, font_system, motion);
                        }
                        // Others collapse the selection to the side they point at.
                        _ => {
                            *goal_x = None;
                            editor.set_cursor(match motion.direction() {
                                Direction::Left => start,
                                Direction::Right => end,
                            });
                        }
                    }
                } else {
                    move_cursor(editor, goal_x, font_system, motion);
                }
            }
            Action::Select(motion) => {
                let cursor = editor.cursor();
                if editor.selection_bounds().is_none() {
                    editor.set_selection(cosmic_text::Selection::Normal(cursor));
                }
                move_cursor(editor, goal_x, font_system, motion);
                deselect_if_empty(editor);
            }
            Action::SelectWord => {
                let cursor = editor.cursor();
                editor.set_selection(cosmic_text::Selection::Word(cursor));
            }
            Action::SelectLine => {
                let cursor = editor.cursor();
                editor.set_selection(cosmic_text::Selection::Line(cursor));
            }
            Action::SelectAll => {
                let buffer = buffer(editor);
                if buffer.lines.len() > 1
                    || buffer
                        .lines
                        .first()
                        .is_some_and(|line| !line.text().is_empty())
                {
                    let cursor = editor.cursor();
                    editor.set_selection(cosmic_text::Selection::Normal(cosmic_text::Cursor {
                        line: 0,
                        index: 0,
                        ..cursor
                    }));
                    editor.action(
                        font_system,
                        cosmic_text::Action::Motion(cosmic_text::Motion::BufferEnd),
                    );
                }
            }
            Action::Edit(edit) => {
                let topmost_before = editor
                    .selection_bounds()
                    .map_or_else(|| editor.cursor(), |(start, _)| start)
                    .line;
                match edit {
                    Edit::Insert(c) => editor.action(font_system, cosmic_text::Action::Insert(c)),
                    Edit::Paste(text) => editor.insert_string(&text, None),
                    Edit::Indent => editor.action(font_system, cosmic_text::Action::Indent),
                    Edit::Unindent => editor.action(font_system, cosmic_text::Action::Unindent),
                    Edit::Enter => editor.action(font_system, cosmic_text::Action::Enter),
                    Edit::Backspace => editor.action(font_system, cosmic_text::Action::Backspace),
                    Edit::Delete => editor.action(font_system, cosmic_text::Action::Delete),
                }
                let topmost_after = editor
                    .selection_bounds()
                    .map_or_else(|| editor.cursor(), |(start, _)| start)
                    .line;
                self.mark_changed(topmost_before.min(topmost_after));
            }
            Action::Click(position) => editor.action(
                font_system,
                cosmic_text::Action::Click {
                    x: position.x as i32,
                    y: position.y as i32,
                },
            ),
            Action::Drag(position) => {
                editor.action(
                    font_system,
                    cosmic_text::Action::Drag {
                        x: position.x as i32,
                        y: position.y as i32,
                    },
                );
                deselect_if_empty(editor);
            }
            Action::Scroll { lines } => {
                let pixels = lines as f32 * buffer(editor).metrics().line_height;
                editor.action(font_system, cosmic_text::Action::Scroll { pixels });
            }
        }
    }

    pub fn move_to(&mut self, cursor: Cursor) {
        self.debug_assert_unshared();
        self.goal_x = None;
        self.editor.set_cursor(to_cursor(cursor.position));
        // Like iced, `None` leaves an existing selection in place.
        if let Some(selection) = cursor.selection {
            self.editor
                .set_selection(cosmic_text::Selection::Normal(to_cursor(selection)));
        }
    }

    pub fn update(
        &mut self,
        new_bounds: Size,
        new_font: Font,
        new_size: Pixels,
        new_line_height: LineHeight,
        new_wrapping: Wrapping,
        highlighter: &mut impl Highlighter,
    ) {
        self.debug_assert_unshared();
        let mut font_system = font_system();
        let buffer = buffer_mut(&mut self.editor);

        if font_system.version() != self.version {
            for line in &mut buffer.lines {
                line.reset();
            }
            self.version = font_system.version();
            self.topmost_line_changed = Some(0);
        }

        if new_font != self.font {
            let attributes = graphics::to_attributes(new_font);
            for line in &mut buffer.lines {
                let _ = line.set_attrs_list(cosmic_text::AttrsList::new(&attributes));
            }
            self.font = new_font;
            self.topmost_line_changed = Some(0);
        }

        let metrics = buffer.metrics();
        let new_line_height = new_line_height.to_absolute(new_size);
        if new_size.0 != metrics.font_size || new_line_height.0 != metrics.line_height {
            buffer.set_metrics(
                font_system.raw(),
                cosmic_text::Metrics::new(new_size.0, new_line_height.0),
            );
            // Sized spans hold metrics derived from the old body metrics.
            self.topmost_line_changed = Some(0);
        }

        let new_wrap = graphics::to_wrap(new_wrapping);
        if new_wrap != buffer.wrap() {
            buffer.set_wrap(font_system.raw(), new_wrap);
        }

        if new_bounds != self.bounds {
            buffer.set_size(
                font_system.raw(),
                Some(new_bounds.width),
                Some(new_bounds.height),
            );
            self.bounds = new_bounds;
        }

        if let Some(line) = self.topmost_line_changed.take() {
            highlighter.change_line(line);
        }

        self.editor.shape_as_needed(font_system.raw(), false);
        // The caret is placed from the cursor line's layout, which shaping a
        // scrolled-away viewport would otherwise leave missing.
        let line = self.editor.cursor().line;
        let _ = buffer_mut(&mut self.editor).line_layout(font_system.raw(), line);
    }

    /// Style the visible lines from `highlighter`. Unlike iced's
    /// `Format`, each span also carries a [`Scale`], which sets its text
    /// size and its row's height from the body metrics.
    pub fn highlight<H: Highlighter>(
        &mut self,
        font: Font,
        highlighter: &mut H,
        format_highlight: impl Fn(&H::Highlight) -> (highlighter::Format<Font>, Scale),
    ) {
        self.debug_assert_unshared();
        let buffer = self.buffer();
        let Some(last_line) = buffer.lines.len().checked_sub(1) else {
            return;
        };
        let scroll = buffer.scroll();
        let mut window = (self.bounds.height / buffer.metrics().line_height).ceil() as i32;
        let last_visible_line = buffer
            .lines
            .get(scroll.line..)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .find_map(|(i, line)| {
                // An unshaped line is assumed to take one row.
                let rows = line.layout_opt().map_or(1, Vec::len) as i32;
                if window > rows {
                    window -= rows;
                    None
                } else {
                    Some(scroll.line + i)
                }
            })
            .unwrap_or(last_line)
            .min(last_line);

        let current_line = highlighter.current_line();
        if current_line > last_visible_line {
            return;
        }

        let body = buffer.metrics();
        let mut font_system = font_system();
        let attributes = graphics::to_attributes(font);
        for line in &mut buffer_mut(&mut self.editor).lines[current_line..=last_visible_line] {
            let mut list = cosmic_text::AttrsList::new(&attributes);
            for (range, highlight) in highlighter.highlight_line(line.text()) {
                let (format, scale) = format_highlight(&highlight);
                let sized = scale != Scale::BODY;
                if format.color.is_some() || format.font.is_some() || sized {
                    let mut span = cosmic_text::Attrs {
                        color_opt: format.color.map(graphics::to_color),
                        ..format
                            .font
                            .map_or_else(|| attributes.clone(), graphics::to_attributes)
                    };
                    if sized {
                        // Whole-pixel rows keep every row below on the pixel grid.
                        span = span.metrics(cosmic_text::Metrics::new(
                            body.font_size * scale.text,
                            (body.line_height * scale.row).round(),
                        ));
                    }
                    list.add_span(range, &span);
                }
            }
            let _ = line.set_attrs_list(list);
        }
        self.editor.shape_as_needed(font_system.raw(), false);
    }

    /// The caret or the selection rectangles, relative to the text origin;
    /// `None` for a caret outside the visible rows.
    pub fn selection(&self) -> Option<Selection> {
        match self.editor.selection_bounds() {
            Some((start, end)) => Some(Selection::Range(self.ranges(start, end))),
            None => self.caret().map(Selection::Caret),
        }
    }

    /// Where an input method should anchor: the caret or the selection's
    /// first visible row, else the edge of the view nearest the cursor.
    pub fn ime_position(&self) -> Option<Point> {
        let anchor = match self.editor.selection_bounds() {
            Some((start, end)) => self.ranges(start, end).first().map(Rectangle::position),
            None => self.caret(),
        };
        anchor.or_else(|| {
            let buffer = self.buffer();
            let below = self.editor.cursor().line >= buffer.scroll().line;
            let line_height = buffer.metrics().line_height;
            Some(Point::new(
                0.0,
                if below {
                    (self.bounds.height - line_height).max(0.0)
                } else {
                    0.0
                },
            ))
        })
    }

    /// Hand the buffer to the renderer. The weak reference is taken last, so
    /// no later mutation in this frame detaches it. tiny-skia ignores
    /// `Raw::clip_bounds`, so the caller clips with a layer.
    pub fn draw(
        &self,
        renderer: &mut Renderer,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let cosmic_text::BufferRef::Arc(buffer) = self.editor.buffer_ref() else {
            return;
        };
        debug_assert_eq!(Arc::strong_count(buffer), 1, "the engine shares its buffer");
        let buffer: Weak<cosmic_text::Buffer> = Arc::downgrade(buffer);
        renderer.fill_raw(Raw {
            buffer,
            position,
            color,
            clip_bounds,
        });
    }

    fn mark_changed(&mut self, line: usize) {
        self.topmost_line_changed =
            Some(self.topmost_line_changed.map_or(line, |old| old.min(line)));
    }

    /// The buffer is only ever held strongly here; a second owner would turn
    /// every edit into a silent copy that the renderer never sees.
    fn debug_assert_unshared(&self) {
        if let cosmic_text::BufferRef::Arc(buffer) = self.editor.buffer_ref() {
            debug_assert_eq!(Arc::strong_count(buffer), 1, "the engine shares its buffer");
        }
    }

    fn caret(&self) -> Option<Point> {
        let cursor = self.editor.cursor();
        let buffer = self.buffer();
        let layout = buffer.lines.get(cursor.line)?.layout_opt()?;
        let (row, x) = caret_row(layout, cursor)?;
        let target = layout.get(row)?;
        let run = buffer
            .layout_runs()
            .skip_while(|run| run.line_i < cursor.line)
            .take_while(|run| run.line_i == cursor.line)
            .find(|run| {
                std::ptr::eq(run.glyphs, target.glyphs.as_slice()) && run.line_w == target.w
            })?;
        Some(Point::new(x, run.line_top))
    }

    fn ranges(&self, start: cosmic_text::Cursor, end: cosmic_text::Cursor) -> Vec<Rectangle> {
        self.buffer()
            .layout_runs()
            .skip_while(|run| run.line_i < start.line)
            .take_while(|run| run.line_i <= end.line)
            .filter_map(|run| {
                let from = if run.line_i == start.line {
                    start.index
                } else {
                    0
                };
                let to = if run.line_i == end.line {
                    end.index
                } else {
                    run.text.len()
                };
                let (x, width) = row_span(run.glyphs, run.line_w, from, to);
                (width > 0.0).then_some(Rectangle {
                    x,
                    y: run.line_top,
                    width,
                    height: run.line_height,
                })
            })
            .collect()
    }
}

/// The global font system. A panic elsewhere while it was held leaves no
/// half-written state that matters here, so a poisoned lock is still used.
fn font_system() -> RwLockWriteGuard<'static, FontSystem> {
    graphics::font_system()
        .write()
        .unwrap_or_else(PoisonError::into_inner)
}

fn buffer<'a>(editor: &'a cosmic_text::Editor<'static>) -> &'a cosmic_text::Buffer {
    match editor.buffer_ref() {
        cosmic_text::BufferRef::Owned(buffer) => buffer,
        cosmic_text::BufferRef::Borrowed(buffer) => buffer,
        cosmic_text::BufferRef::Arc(buffer) => buffer,
    }
}

fn buffer_mut<'a>(editor: &'a mut cosmic_text::Editor<'static>) -> &'a mut cosmic_text::Buffer {
    match editor.buffer_ref_mut() {
        cosmic_text::BufferRef::Owned(buffer) => buffer,
        cosmic_text::BufferRef::Borrowed(buffer) => buffer,
        cosmic_text::BufferRef::Arc(buffer) => Arc::make_mut(buffer),
    }
}

/// `cursor` moved onto valid bytes of `buffer`.
fn clamp(buffer: &cosmic_text::Buffer, cursor: cosmic_text::Cursor) -> cosmic_text::Cursor {
    let line = cursor.line.min(buffer.lines.len().saturating_sub(1));
    let text = buffer.lines.get(line).map_or("", |line| line.text());
    let mut index = cursor.index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    cosmic_text::Cursor {
        line,
        index,
        ..cursor
    }
}

/// Move the cursor by `motion`. A vertical motion aims for `goal_x`, and
/// keeps it for the next one; any other motion forgets it.
fn move_cursor(
    editor: &mut cosmic_text::Editor<'static>,
    goal_x: &mut Option<i32>,
    font_system: &mut cosmic_text::FontSystem,
    motion: Motion,
) {
    let vertical = matches!(
        motion,
        Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown
    );
    let from = editor.cursor();
    let buffer = buffer_mut(editor);
    let moved = match motion {
        Motion::PageUp | Motion::PageDown => {
            let down = motion == Motion::PageDown;
            let row = if down {
                cosmic_text::Motion::Down
            } else {
                cosmic_text::Motion::Up
            };
            page_rows(buffer, font_system, from, down).and_then(|rows| {
                (0..rows).try_fold((from, *goal_x), |(cursor, goal), _| {
                    buffer.cursor_motion(font_system, cursor, goal, row)
                })
            })
        }
        _ => buffer.cursor_motion(
            font_system,
            from,
            goal_x.filter(|_| vertical),
            to_motion(motion),
        ),
    };
    match moved {
        Some((cursor, goal)) => {
            editor.set_cursor(cursor);
            *goal_x = goal.filter(|_| vertical);
        }
        None if !vertical => *goal_x = None,
        None => {}
    }
}

/// How many rows a page moves from `cursor`: as many as fit in the view,
/// each counted at its own height, so a page over taller heading rows moves
/// fewer of them (cosmic-text counts body rows). At the edge of the text one
/// more row lets the last motion settle there as cosmic-text's would.
fn page_rows(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    cursor: cosmic_text::Cursor,
    down: bool,
) -> Option<usize> {
    let page = buffer.size().1?;
    let body = buffer.metrics().line_height;
    let start = buffer.layout_cursor(font_system, cursor)?;
    let (mut line, mut row) = (start.line, start.layout);
    let (mut rows, mut moved) = (0, 0.0);
    loop {
        if down {
            let count = buffer.line_layout(font_system, line)?.len();
            if row + 1 < count {
                row += 1;
            } else if line + 1 < buffer.lines.len() {
                (line, row) = (line + 1, 0);
            } else {
                return Some(rows + 1);
            }
        } else if row > 0 {
            row -= 1;
        } else if line > 0 {
            line -= 1;
            row = buffer
                .line_layout(font_system, line)?
                .len()
                .saturating_sub(1);
        } else {
            return Some(rows + 1);
        }
        let height = buffer
            .line_layout(font_system, line)?
            .get(row)
            .and_then(|layout| layout.line_height_opt)
            .unwrap_or(body);
        if moved + height > page {
            return Some(rows);
        }
        moved += height;
        rows += 1;
    }
}

fn deselect_if_empty(editor: &mut cosmic_text::Editor<'static>) {
    if let Some((start, end)) = editor.selection_bounds() {
        if start.line == end.line && start.index == end.index {
            editor.set_selection(cosmic_text::Selection::None);
        }
    }
}

/// The visual row of `layout` holding `cursor`, and the caret's x in it.
fn caret_row(
    layout: &[cosmic_text::LayoutLine],
    cursor: cosmic_text::Cursor,
) -> Option<(usize, f32)> {
    for (row, line) in layout.iter().enumerate() {
        let start = line.glyphs.first().map_or(0, |glyph| glyph.start);
        let end = line.glyphs.last().map_or(0, |glyph| glyph.end);
        if start > cursor.index {
            // A row that wraps on a space drops the space, leaving the cursor
            // between rows: it belongs at the end of the previous one.
            let previous = layout.get(row.checked_sub(1)?)?;
            return Some((row - 1, previous.w));
        }
        let before_end = match cursor.affinity {
            cosmic_text::Affinity::Before => cursor.index <= end,
            cosmic_text::Affinity::After => cursor.index < end,
        };
        if before_end {
            return Some((row, caret_x(&line.glyphs, cursor.index)));
        }
    }
    let last = layout.len().checked_sub(1)?;
    Some((last, layout[last].w))
}

/// The caret's x at byte `index` of a row, splitting a multi-character glyph
/// (a ligature) evenly between its characters.
fn caret_x(glyphs: &[cosmic_text::LayoutGlyph], index: usize) -> f32 {
    for glyph in glyphs {
        let rtl = glyph.level.is_rtl();
        if index == glyph.start {
            return if rtl { glyph.x + glyph.w } else { glyph.x };
        }
        if index > glyph.start && index < glyph.end {
            let before = (glyph.start..index).len() as f32;
            let total = (glyph.start..glyph.end).len() as f32;
            let offset = glyph.w * before / total;
            return if rtl {
                glyph.x + glyph.w - offset
            } else {
                glyph.x + offset
            };
        }
    }
    glyphs.last().map_or(0.0, |glyph| {
        if glyph.level.is_rtl() {
            glyph.x
        } else {
            glyph.x + glyph.w
        }
    })
}

/// The `(x, width)` of bytes `from..to` within one visual row.
fn row_span(
    glyphs: &[cosmic_text::LayoutGlyph],
    row_width: f32,
    from: usize,
    to: usize,
) -> (f32, f32) {
    let start = glyphs.first().map_or(0, |glyph| glyph.start);
    let end = glyphs.last().map_or(0, |glyph| glyph.end);
    let range = start.max(from)..end.min(to);
    if range.is_empty() {
        (0.0, 0.0)
    } else if range.start == start && range.end == end {
        (0.0, row_width)
    } else {
        let first = glyphs
            .iter()
            .position(|glyph| range.start <= glyph.start)
            .unwrap_or(0);
        let mut glyphs = glyphs.iter();
        let x = glyphs.by_ref().take(first).map(|glyph| glyph.w).sum();
        let width = glyphs
            .take_while(|glyph| range.end > glyph.start)
            .map(|glyph| glyph.w)
            .sum();
        (x, width)
    }
}

fn to_cursor(position: Position) -> cosmic_text::Cursor {
    cosmic_text::Cursor {
        line: position.line,
        index: position.column,
        affinity: cosmic_text::Affinity::Before,
    }
}

fn to_motion(motion: Motion) -> cosmic_text::Motion {
    match motion {
        Motion::Left => cosmic_text::Motion::Left,
        Motion::Right => cosmic_text::Motion::Right,
        Motion::Up => cosmic_text::Motion::Up,
        Motion::Down => cosmic_text::Motion::Down,
        Motion::WordLeft => cosmic_text::Motion::LeftWord,
        Motion::WordRight => cosmic_text::Motion::RightWord,
        Motion::Home => cosmic_text::Motion::Home,
        Motion::End => cosmic_text::Motion::End,
        Motion::PageUp => cosmic_text::Motion::PageUp,
        Motion::PageDown => cosmic_text::Motion::PageDown,
        Motion::DocumentStart => cosmic_text::Motion::BufferStart,
        Motion::DocumentEnd => cosmic_text::Motion::BufferEnd,
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::text::highlighter::Highlighter as _;

    use super::*;
    use crate::fixtures;
    use crate::highlighter::{MarkdownHighlighter, Settings};

    fn highlighter() -> MarkdownHighlighter {
        MarkdownHighlighter::new(&Settings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
            front_matter_end: None,
        })
    }

    fn update(engine: &mut Engine, highlighter: &mut MarkdownHighlighter, height: f32, font: Font) {
        engine.update(
            Size::new(300.0, height),
            font,
            Pixels(14.0),
            LineHeight::Absolute(Pixels(24.0)),
            Wrapping::WordOrGlyph,
            highlighter,
        );
        engine.highlight(font, highlighter, |highlight| {
            (highlight.to_format(), highlight.scale())
        });
    }

    fn weak(engine: &Engine) -> Weak<cosmic_text::Buffer> {
        let cosmic_text::BufferRef::Arc(buffer) = engine.editor.buffer_ref() else {
            panic!("the engine keeps its buffer in an Arc");
        };
        Arc::downgrade(buffer)
    }

    #[test]
    fn an_edit_detaches_the_renderers_weak_buffer_instead_of_copying_it() {
        fixtures::load_font();
        let mut engine = Engine::with_text("hello");
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 200.0, fixtures::FONT);
        let drawn = weak(&engine);
        assert!(drawn.upgrade().is_some());
        engine.perform(Action::Move(Motion::DocumentEnd));
        engine.perform(Action::Edit(Edit::Insert('!')));
        assert!(drawn.upgrade().is_none());
        assert_eq!(engine.buffer().lines[0].text(), "hello!");
        let next = weak(&engine);
        assert_eq!(next.strong_count(), 1);
    }

    #[test]
    fn highlighting_styles_the_visible_lines() {
        fixtures::load_font();
        let mut engine = Engine::with_text("# Title\nplain\n`code`");
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 200.0, fixtures::FONT);
        let spans = |line: usize| engine.buffer().lines[line].attrs_list().spans().len();
        assert!(spans(0) > 0);
        assert_eq!(spans(1), 0);
        assert!(spans(2) > 0);
        assert_eq!(highlighter.current_line(), 3);
    }

    #[test]
    fn edits_and_replacement_rehighlight_from_the_topmost_changed_line() {
        fixtures::load_font();
        let text = (0..10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut engine = Engine::with_text(&text);
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 400.0, fixtures::FONT);
        assert_eq!(engine.topmost_line_changed, None);
        for line in [5, 2, 7] {
            engine.move_to(Cursor {
                position: Position { line, column: 0 },
                selection: None,
            });
            engine.perform(Action::Edit(Edit::Insert('x')));
        }
        assert_eq!(engine.topmost_line_changed, Some(2));
        update(&mut engine, &mut highlighter, 400.0, fixtures::FONT);
        assert_eq!(engine.topmost_line_changed, None);
        engine.replace_text("other");
        assert_eq!(engine.topmost_line_changed, Some(0));
    }

    #[test]
    fn a_font_change_restyles_every_line() {
        fixtures::load_font();
        let mut engine = Engine::with_text("a\nb");
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 200.0, fixtures::FONT);
        let has = |engine: &Engine, family: cosmic_text::Family<'_>| {
            engine.buffer().lines[1].attrs_list().defaults().family == family
        };
        assert!(has(&engine, cosmic_text::Family::Name("JetBrains Mono")));
        update(&mut engine, &mut highlighter, 200.0, Font::MONOSPACE);
        assert!(has(&engine, cosmic_text::Family::Monospace));
    }

    #[test]
    fn replace_text_keeps_the_scroll_and_clamps_the_cursor() {
        fixtures::load_font();
        let text = (0..80)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut engine = Engine::with_text(&text);
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        engine.move_to(Cursor {
            position: Position {
                line: 40,
                column: 7,
            },
            selection: Some(Position {
                line: 40,
                column: 0,
            }),
        });
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        let scroll = engine.buffer().scroll();
        assert!(scroll.line > 30, "{scroll:?}");

        engine.replace_text(&text.replace("line 3", "LINE 3"));
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        assert_eq!(engine.buffer().scroll(), scroll);
        assert_eq!(
            engine.cursor().position,
            Position {
                line: 40,
                column: 7
            }
        );
        assert_eq!(engine.cursor().selection, None);

        engine.replace_text("short\nlé");
        assert_eq!(engine.cursor().position, Position { line: 1, column: 3 });
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        assert!(matches!(engine.selection(), Some(Selection::Caret(_))));
    }

    #[test]
    fn caret_positions_split_ligatures_and_follow_rtl_glyphs() {
        fixtures::load_font();
        let mut engine = Engine::with_text("ab");
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 200.0, fixtures::FONT);
        let glyphs = engine.buffer().lines[0].layout_opt().unwrap()[0]
            .glyphs
            .clone();
        let mut ligature = glyphs[0].clone();
        ligature.end = 2;
        ligature.w = 20.0;
        ligature.x = 10.0;
        assert_eq!(caret_x(std::slice::from_ref(&ligature), 0), 10.0);
        assert_eq!(caret_x(std::slice::from_ref(&ligature), 1), 20.0);
        assert_eq!(caret_x(std::slice::from_ref(&ligature), 2), 30.0);
        // Hebrew shapes right to left; borrow its bidi level.
        let mut hebrew = Engine::with_text("שלום");
        update(&mut hebrew, &mut highlighter, 200.0, fixtures::FONT);
        let rtl = &hebrew.buffer().lines[0].layout_opt().unwrap()[0].glyphs[0];
        assert!(rtl.level.is_rtl());
        ligature.level = rtl.level;
        assert_eq!(caret_x(std::slice::from_ref(&ligature), 0), 30.0);
        assert_eq!(caret_x(std::slice::from_ref(&ligature), 1), 20.0);
        assert_eq!(caret_x(&[], 0), 0.0);
    }

    fn at(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    #[test]
    fn vertical_motions_keep_their_goal_column_until_anything_else_moves() {
        fixtures::load_font();
        let mut engine = Engine::with_text("0123456789\nab\n0123456789\n0123456789\n0123456789");
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 400.0, fixtures::FONT);
        let mut step = |engine: &mut Engine, action: Action| {
            engine.perform(action);
            update(engine, &mut highlighter, 400.0, fixtures::FONT);
            engine.cursor()
        };
        let down = || Action::Move(Motion::Down);
        let up = || Action::Move(Motion::Up);

        engine.move_to(at(0, 8));
        assert_eq!(step(&mut engine, down()), at(1, 2));
        assert_eq!(step(&mut engine, down()), at(2, 8));
        // A horizontal motion sets a new goal.
        assert_eq!(step(&mut engine, Action::Move(Motion::Left)), at(2, 7));
        assert_eq!(step(&mut engine, up()), at(1, 2));
        assert_eq!(step(&mut engine, up()), at(0, 7));

        // So does a programmatic move, which cosmic-text's own editor keeps.
        engine.move_to(at(0, 9));
        assert_eq!(step(&mut engine, down()), at(1, 2));
        engine.move_to(at(1, 1));
        assert_eq!(step(&mut engine, down()), at(2, 1));

        // And an edit, a click, and a selection.
        engine.move_to(at(0, 9));
        assert_eq!(step(&mut engine, down()), at(1, 2));
        assert_eq!(step(&mut engine, Action::Edit(Edit::Insert('c'))), at(1, 3));
        assert_eq!(step(&mut engine, down()), at(2, 3));
        assert_eq!(
            step(&mut engine, Action::Click(Point::new(1.0, 30.0))),
            at(1, 0)
        );
        assert_eq!(step(&mut engine, down()), at(2, 0));
        assert_eq!(
            step(&mut engine, Action::Select(Motion::End))
                .position
                .column,
            10
        );
        assert_eq!(
            step(&mut engine, Action::Select(Motion::Down)).position,
            Position {
                line: 3,
                column: 10
            }
        );

        // Selecting vertically keeps the goal like moving does.
        engine.move_to(at(0, 9));
        assert_eq!(
            step(&mut engine, Action::Select(Motion::Down)).position,
            Position { line: 1, column: 3 }
        );
        assert_eq!(
            step(&mut engine, Action::Select(Motion::Down)).position,
            Position { line: 2, column: 9 }
        );
        // Scrolling is not a motion and keeps it.
        // Collapsing the selection to its end is a motion of its own.
        assert_eq!(step(&mut engine, Action::Move(Motion::Right)), at(2, 9));
        assert_eq!(step(&mut engine, down()), at(3, 9));
        let _ = step(&mut engine, Action::Scroll { lines: 1 });
        engine.perform(Action::Move(Motion::Up));
        engine.perform(Action::Move(Motion::Up));
        engine.perform(Action::Move(Motion::Up));
        assert_eq!(engine.cursor(), at(0, 9));
    }

    #[test]
    fn page_motions_keep_the_goal_column() {
        fixtures::load_font();
        let text = (0..60)
            .map(|i| {
                if i % 2 == 0 {
                    "long line here".to_owned()
                } else {
                    "x".to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut engine = Engine::with_text(&text);
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        engine.move_to(at(0, 9));
        engine.perform(Action::Move(Motion::PageDown));
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        assert_eq!(engine.cursor(), at(5, 1));
        engine.perform(Action::Move(Motion::PageDown));
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        assert_eq!(engine.cursor(), at(10, 9));
        engine.perform(Action::Move(Motion::PageUp));
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        assert_eq!(engine.cursor(), at(5, 1));
        engine.perform(Action::Move(Motion::Up));
        assert_eq!(engine.cursor(), at(4, 9));
    }

    #[test]
    fn heading_rows_place_the_caret_selection_and_clicks_by_their_own_height() {
        fixtures::load_font();
        let mut engine = Engine::with_text("# Title\nbody\n## Two");
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 400.0, fixtures::FONT);
        let caret = |engine: &mut Engine, highlighter: &mut MarkdownHighlighter, at| {
            engine.move_to(at);
            update(engine, highlighter, 400.0, fixtures::FONT);
            match engine.selection() {
                Some(Selection::Caret(point)) => point,
                other => panic!("{other:?}"),
            }
        };
        // Rows: H1 32 px, body 24 px, H2 28 px.
        assert_eq!(caret(&mut engine, &mut highlighter, at(1, 0)).y, 32.0);
        assert_eq!(caret(&mut engine, &mut highlighter, at(2, 0)).y, 56.0);
        // Heading glyphs are wider than body ones.
        let heading = caret(&mut engine, &mut highlighter, at(0, 7)).x;
        let body = caret(&mut engine, &mut highlighter, at(1, 4)).x;
        assert!(heading / 5.0 > body / 4.0 * 1.3, "{heading} {body}");

        engine.move_to(Cursor {
            position: Position { line: 2, column: 4 },
            selection: Some(Position { line: 0, column: 2 }),
        });
        update(&mut engine, &mut highlighter, 400.0, fixtures::FONT);
        let Some(Selection::Range(ranges)) = engine.selection() else {
            panic!("a selection");
        };
        let rows: Vec<_> = ranges.iter().map(|range| (range.y, range.height)).collect();
        assert_eq!(rows, [(0.0, 32.0), (32.0, 24.0), (56.0, 28.0)]);

        for (y, line) in [(31.0, 0), (33.0, 1), (55.0, 1), (57.0, 2), (83.0, 2)] {
            engine.perform(Action::Click(Point::new(1.0, y)));
            assert_eq!(engine.cursor().position.line, line, "y = {y}");
        }
    }

    #[test]
    fn a_new_text_size_or_line_height_resizes_the_heading_rows() {
        fixtures::load_font();
        let mut engine = Engine::with_text("# Title\nbody");
        let mut highlighter = highlighter();
        let heights = |engine: &mut Engine, highlighter: &mut MarkdownHighlighter, line: f32| {
            engine.update(
                Size::new(300.0, 400.0),
                fixtures::FONT,
                Pixels(14.0),
                LineHeight::Absolute(Pixels(line)),
                Wrapping::WordOrGlyph,
                highlighter,
            );
            engine.highlight(fixtures::FONT, highlighter, |highlight| {
                (highlight.to_format(), highlight.scale())
            });
            engine
                .buffer()
                .layout_runs()
                .map(|run| run.line_height)
                .collect::<Vec<_>>()
        };
        assert_eq!(heights(&mut engine, &mut highlighter, 24.0), [32.0, 24.0]);
        assert_eq!(heights(&mut engine, &mut highlighter, 28.0), [38.0, 28.0]);
        assert_eq!(heights(&mut engine, &mut highlighter, 18.0), [24.0, 18.0]);
    }

    #[test]
    fn a_page_counts_heading_rows_at_their_own_height() {
        fixtures::load_font();
        // Ten H1 rows of 32 px: a 120 px page holds three, not five.
        let text = (0..10)
            .map(|i| format!("# Heading {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut engine = Engine::with_text(&text);
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        engine.move_to(at(0, 4));
        engine.perform(Action::Move(Motion::PageDown));
        update(&mut engine, &mut highlighter, 120.0, fixtures::FONT);
        assert_eq!(engine.cursor(), at(3, 4));
        engine.perform(Action::Move(Motion::PageUp));
        assert_eq!(engine.cursor(), at(0, 4));
        // At the end of the text a page settles on the last row.
        engine.move_to(at(9, 4));
        engine.perform(Action::Move(Motion::PageDown));
        assert_eq!(engine.cursor().position.line, 9);
    }

    #[test]
    fn range_replacement_and_restored_views_rehighlight_and_forget_the_goal() {
        fixtures::load_font();
        let mut engine = Engine::with_text("0123456789\nab\n0123456789");
        let mut highlighter = highlighter();
        update(&mut engine, &mut highlighter, 400.0, fixtures::FONT);
        engine.move_to(at(0, 8));
        engine.perform(Action::Move(Motion::Down));
        assert!(engine.goal_x.is_some());
        engine.replace_range(
            Position { line: 1, column: 0 },
            Position { line: 1, column: 2 },
            "AB\r\nC",
        );
        assert_eq!(engine.goal_x, None);
        assert_eq!(engine.topmost_line_changed, Some(1));
        assert_eq!(engine.cursor(), at(2, 1));
        assert_eq!(engine.buffer().lines[1].text(), "AB");
        assert_eq!(engine.buffer().lines[2].text(), "C");
        engine.perform(Action::Move(Motion::Down));
        assert!(engine.goal_x.is_some());
        engine.restore_view(at(0, 1), cosmic_text::Scroll::default());
        assert_eq!(engine.goal_x, None);
    }
}
