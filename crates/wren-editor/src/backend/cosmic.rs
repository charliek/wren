//! `Backend::Cosmic`: a cosmic-text editor that Wren owns.
//!
//! Ported from `iced_graphics` 0.14.0, `src/text/editor.rs`
//! (<https://github.com/iced-rs/iced>), used under the MIT License:
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
//! (<https://github.com/pop-os/cosmic-text>, Copyright 2022 System76), used
//! under the MIT License, which cosmic-text offers alongside Apache-2.0.
//!
//! Modified by Wren: the editor is owned rather than hidden behind iced's
//! `text::Editor`; the buffer sits in an `Arc` so the widget can hand the
//! renderer a weak reference (`fill_raw`); the caret, selection, and IME
//! rectangles come from the visible layout runs and are `None` instead of a
//! panic when the cursor is not laid out; the font-system lock is taken once
//! per entry point and survives poisoning; highlight bounds tolerate
//! unshaped lines; edits accumulate the topmost changed line instead of
//! overwriting it; and `replace_text` keeps the scroll position.

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

pub struct Engine {
    editor: cosmic_text::Editor<'static>,
    font: Font,
    bounds: Size,
    /// The first line whose highlighting is stale; the next update tells the
    /// highlighter to resume there.
    topmost_line_changed: Option<usize>,
    version: Version,
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
        self.mark_changed(0);
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
                            editor.action(
                                font_system,
                                cosmic_text::Action::Motion(to_motion(motion)),
                            );
                        }
                        // Others collapse the selection to the side they point at.
                        _ => editor.set_cursor(match motion.direction() {
                            Direction::Left => start,
                            Direction::Right => end,
                        }),
                    }
                } else {
                    editor.action(font_system, cosmic_text::Action::Motion(to_motion(motion)));
                }
            }
            Action::Select(motion) => {
                let cursor = editor.cursor();
                if editor.selection_bounds().is_none() {
                    editor.set_selection(cosmic_text::Selection::Normal(cursor));
                }
                editor.action(font_system, cosmic_text::Action::Motion(to_motion(motion)));
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
        self.editor.set_cursor(cosmic_text::Cursor {
            line: cursor.position.line,
            index: cursor.position.column,
            affinity: cosmic_text::Affinity::Before,
        });
        // Like iced, `None` leaves an existing selection in place.
        if let Some(selection) = cursor.selection {
            self.editor
                .set_selection(cosmic_text::Selection::Normal(cosmic_text::Cursor {
                    line: selection.line,
                    index: selection.column,
                    affinity: cosmic_text::Affinity::Before,
                }));
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

    pub fn highlight<H: Highlighter>(
        &mut self,
        font: Font,
        highlighter: &mut H,
        format_highlight: impl Fn(&H::Highlight) -> highlighter::Format<Font>,
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

        let mut font_system = font_system();
        let attributes = graphics::to_attributes(font);
        for line in &mut buffer_mut(&mut self.editor).lines[current_line..=last_visible_line] {
            let mut list = cosmic_text::AttrsList::new(&attributes);
            for (range, highlight) in highlighter.highlight_line(line.text()) {
                let format = format_highlight(&highlight);
                if format.color.is_some() || format.font.is_some() {
                    list.add_span(
                        range,
                        &cosmic_text::Attrs {
                            color_opt: format.color.map(graphics::to_color),
                            ..format
                                .font
                                .map_or_else(|| attributes.clone(), graphics::to_attributes)
                        },
                    );
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
        engine.highlight(font, highlighter, |highlight| highlight.to_format());
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
}
