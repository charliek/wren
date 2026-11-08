//! Wren's Markdown text-editor widget.
//!
//! Forked from `iced_widget` 0.14.2, `src/text_editor.rs`
//! (<https://github.com/iced-rs/iced>, commit
//! 54abf81d13fec06d4d9ac754b03ce2c3313e8a1f), used under the MIT License:
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
//! Wren changes: concrete renderer, theme, highlighter, and message types; no
//! placeholder; and extra drawing for the mockup's line-number gutter,
//! current-line band, accent caret, and completed-task strike-through. Stock
//! `Content` hides its renderer editor, and the line geometry lives in that
//! editor's cosmic-text buffer, so this module owns its own `Content`.

use std::cell::RefCell;
use std::ops::{DerefMut, Range};
use std::sync::Arc;

use iced::advanced::graphics::text::cosmic_text::{self, Scroll};
use iced::advanced::layout::{self, Layout};
use iced::advanced::mouse;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::highlighter::Highlighter as _;
use iced::advanced::text::{self, LineHeight, Renderer as _, Text, Wrapping};
use iced::advanced::widget::{self, operation, tree, Widget};
use iced::advanced::{clipboard, input_method, Clipboard, InputMethod, Shell};
use iced::keyboard;
use iced::time::{Duration, Instant};
use iced::widget::text_editor::{
    Action, Binding, Cursor, Edit, KeyPress, LineEnding, Position, Selection, Status,
};
use iced::{
    alignment, window, Border, Element, Event, Length, Padding, Pixels, Point, Rectangle, Renderer,
    Size, Theme, Vector,
};

use crate::backend::{Backend, BackendKind, Lines, Placement};
use crate::highlighter::{MarkdownHighlighter, Settings as HighlighterSettings};
use crate::keys::KeyPolicy;
use crate::{Message, Style};

const TEXT_SIZE: f32 = 14.0;
/// Row height as a multiple of the text size: the mockup's 24 px rows at
/// 14 px text.
pub const DEFAULT_LINE_HEIGHT: f32 = 1.7;
/// Horizontal inset of each mockup row (`editorWrap` padding).
const ROW_INSET: f32 = 8.0;
/// How far a code band reaches left of the text, into the gutter padding.
const CODE_BAND_OUTSET: f32 = 8.0;
const GUTTER_WIDTH: f32 = 52.0;
const GUTTER_PADDING_RIGHT: f32 = 18.0;
/// Text inset after the row inset when the gutter is hidden.
const BARE_TEXT_INSET: f32 = 8.0;
const VERTICAL_PADDING: f32 = 24.0;
const CARET_WIDTH: f32 = 2.0;
const CARET_HEIGHT: f32 = 18.0;
/// JetBrains Mono OS/2 `sxHeight` (550) over its 1000 units per em; other
/// monospace fonts sit close enough for the strike-through.
const X_HEIGHT_EM: f32 = 0.55;
/// JetBrains Mono's advance (600 units per 1000-unit em): the width of the
/// cell that shows a selected line break.
const ADVANCE_EM: f32 = 0.6;
/// An inline code chip's height, in ems: the glyph box plus about 2 px a
/// side at the default size.
const CHIP_HEIGHT_EM: f32 = 1.3;
const CHIP_PADDING_X: f32 = 2.0;
const CHIP_RADIUS: f32 = 3.0;
const STRIKE_THICKNESS: f32 = 1.0;

/// Text size, row height, caret height, and gutter width. The caret and
/// gutter scale from the mockup's 14 px text / 18 px caret / 52 px gutter;
/// rows are the text size times a line-height ratio, in whole pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub text: f32,
    pub line: f32,
    pub caret: f32,
    /// The gutter's width; only its number column scales, so larger text
    /// keeps room for four digits.
    pub gutter: f32,
}

impl Metrics {
    /// Metrics for `text` px text in rows `line_height` times as tall.
    pub fn new(text: f32, line_height: f32) -> Self {
        let scale = text / TEXT_SIZE;
        let line = (text * line_height).round();
        Self {
            text,
            line,
            // A tight line height must not push the caret into the next row.
            caret: (CARET_HEIGHT * scale).round().min(line),
            gutter: GUTTER_PADDING_RIGHT + ((GUTTER_WIDTH - GUTTER_PADDING_RIGHT) * scale).round(),
        }
    }

    pub fn for_size(text: f32) -> Self {
        Self::new(text, DEFAULT_LINE_HEIGHT)
    }

    fn line_height(self) -> LineHeight {
        LineHeight::Absolute(Pixels(self.line))
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::for_size(TEXT_SIZE)
    }
}

#[cfg(test)]
fn line_height() -> LineHeight {
    Metrics::default().line_height()
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

/// The text and cursor of a [`TextEditor`].
pub struct Content(RefCell<Backend>);

impl Content {
    /// Creates a [`Content`] with the given text on the `kind` engine.
    pub fn new(kind: BackendKind, text: &str) -> Self {
        Self(RefCell::new(Backend::with_text(kind, text)))
    }

    pub fn kind(&self) -> BackendKind {
        self.0.borrow().kind()
    }

    /// Replaces the text on the same engine; see [`Backend::replace_text`].
    pub fn replace_text(&mut self, text: &str) {
        self.0.get_mut().replace_text(text);
    }

    /// Performs an [`Action`] on the [`Content`].
    pub fn perform(&mut self, action: Action) {
        self.0.get_mut().perform(action);
    }

    /// Moves the current cursor to reflect the given one.
    pub fn move_to(&mut self, cursor: Cursor) {
        self.0.get_mut().move_to(cursor);
    }

    /// See [`Backend::replace_range`].
    pub fn replace_range(&mut self, start: Position, end: Position, text: &str) {
        self.0.get_mut().replace_range(start, end, text);
    }

    /// See [`Backend::restore_view`].
    pub fn restore_view(&mut self, cursor: Cursor, scroll: Scroll) {
        self.0.get_mut().restore_view(cursor, scroll);
    }

    /// See [`Backend::return_to`].
    pub fn return_to(&mut self, cursor: Cursor, scroll: Scroll) {
        self.0.get_mut().return_to(cursor, scroll);
    }

    /// See [`Backend::scroll_to_line`].
    pub fn scroll_to_line(&mut self, line: usize, placement: Placement) {
        self.0.get_mut().scroll_to_line(line, placement);
    }

    /// Where the view is scrolled.
    pub fn scroll(&self) -> Scroll {
        self.0.borrow().scroll()
    }

    /// Returns the current cursor position of the [`Content`].
    pub fn cursor(&self) -> Cursor {
        self.0.borrow().cursor()
    }

    /// Returns the amount of lines of the [`Content`].
    pub fn line_count(&self) -> usize {
        self.0.borrow().line_count()
    }

    /// Runs `f` over the borrowed lines, without copying any text.
    pub fn with_lines<R>(&self, f: impl FnOnce(Lines<'_>) -> R) -> R {
        f(self.0.borrow().lines())
    }

    /// Returns the text of the [`Content`].
    pub fn text(&self) -> String {
        self.with_lines(|lines| {
            let mut contents = String::new();
            let mut lines = lines.iter().peekable();
            while let Some(line) = lines.next() {
                contents.push_str(&line.text);
                if lines.peek().is_some() {
                    contents.push_str(if line.ending == LineEnding::None {
                        LineEnding::default().as_str()
                    } else {
                        line.ending.as_str()
                    });
                }
            }
            contents
        })
    }

    /// Returns the selected text of the [`Content`].
    pub fn selection(&self) -> Option<String> {
        self.0.borrow().copy()
    }

    /// Lays the text out in `size` the way the widget does, with the test font.
    #[cfg(test)]
    pub fn lay_out(&mut self, size: Size) {
        crate::fixtures::load_font();
        let mut highlighter = MarkdownHighlighter::new(&HighlighterSettings {
            palette: crate::fixtures::DARK,
            font: crate::fixtures::FONT,
            front_matter_end: None,
        });
        let backend = self.0.get_mut();
        backend.update(
            size,
            crate::fixtures::FONT,
            Pixels(TEXT_SIZE),
            line_height(),
            Wrapping::WordOrGlyph,
            &mut highlighter,
        );
        backend.highlight(crate::fixtures::FONT, &mut highlighter);
    }

    /// Whether `line` still has its layout from the last update.
    #[cfg(test)]
    pub fn is_laid_out(&self, line: usize) -> bool {
        self.0.borrow().buffer().lines[line].layout_opt().is_some()
    }

    /// The caret or selection rectangles; see [`Backend::selection`].
    #[cfg(test)]
    pub fn selection_geometry(&self) -> Option<Selection> {
        self.0.borrow().selection()
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// A multi-line Markdown editor with a line-number gutter.
pub struct TextEditor<'a> {
    id: widget::Id,
    content: &'a Content,
    highlighter_settings: HighlighterSettings,
    style: Style,
    line_numbers: bool,
    metrics: Metrics,
    /// The widest the column of gutter and text grows; past it the column
    /// is centred.
    max_width: Option<f32>,
    markdown_assist: bool,
    key_policy: Option<&'a KeyPolicy>,
    last_status: Option<Status>,
}

impl<'a> TextEditor<'a> {
    pub fn new(
        content: &'a Content,
        id: &'static str,
        highlighter_settings: HighlighterSettings,
        style: Style,
        line_numbers: bool,
        metrics: Metrics,
        markdown_assist: bool,
    ) -> Self {
        Self {
            id: widget::Id::from(id),
            content,
            highlighter_settings,
            style,
            line_numbers,
            metrics,
            max_width: None,
            markdown_assist,
            key_policy: None,
            last_status: None,
        }
    }

    /// Cap the column of gutter and text at `max_width`, centred in the
    /// widget; `None` lets it fill the widget.
    pub fn max_width(mut self, max_width: Option<f32>) -> Self {
        self.max_width = max_width;
        self
    }

    /// Ask `policy` about each key press first (see [`KeyPolicy`]).
    pub fn key_policy(mut self, policy: Option<&'a KeyPolicy>) -> Self {
        self.key_policy = policy;
        self
    }

    /// The space on either side of the column in a widget `width` wide,
    /// in whole pixels so the text stays on the pixel grid.
    fn margin(&self, width: f32) -> f32 {
        self.max_width
            .map_or(0.0, |max| ((width - max) / 2.0).max(0.0).floor())
    }

    /// The column's horizontal extent in `bounds`.
    fn column(&self, bounds: Rectangle) -> (f32, f32) {
        let margin = self.margin(bounds.width);
        (bounds.x + margin, bounds.width - 2.0 * margin)
    }

    /// The space around the text in a widget `width` wide.
    fn padding(&self, width: f32) -> Padding {
        let margin = self.margin(width);
        let left = if self.line_numbers {
            ROW_INSET + self.metrics.gutter
        } else {
            ROW_INSET + BARE_TEXT_INSET
        };
        Padding {
            top: VERTICAL_PADDING,
            right: margin + ROW_INSET,
            bottom: VERTICAL_PADDING,
            left: margin + left,
        }
    }

    fn input_method<'b>(&self, state: &'b State, layout: Layout<'_>) -> InputMethod<&'b str> {
        let Some(Focus {
            is_window_focused: true,
            ..
        }) = &state.focus
        else {
            return InputMethod::Disabled;
        };

        let bounds = layout.bounds();
        let text_bounds = bounds.shrink(self.padding(bounds.width));
        let translation = text_bounds.position() - Point::ORIGIN;
        let editor = self.content.0.borrow();
        let Some(cursor) = editor.ime_position() else {
            return InputMethod::Disabled;
        };
        let height = row_height_at(editor.buffer(), cursor.y).unwrap_or(self.metrics.line);

        InputMethod::Enabled {
            cursor: Rectangle::new(cursor + translation, Size::new(1.0, height)),
            purpose: input_method::Purpose::Normal,
            preedit: state.preedit.as_ref().map(input_method::Preedit::as_ref),
        }
    }
}

struct State {
    focus: Option<Focus>,
    preedit: Option<input_method::Preedit>,
    last_click: Option<mouse::Click>,
    drag_click: Option<mouse::click::Kind>,
    partial_scroll: f32,
    highlighter: RefCell<MarkdownHighlighter>,
    highlighter_settings: HighlighterSettings,
}

impl State {
    fn new(highlighter_settings: HighlighterSettings) -> Self {
        Self {
            focus: None,
            preedit: None,
            last_click: None,
            drag_click: None,
            partial_scroll: 0.0,
            highlighter: RefCell::new(MarkdownHighlighter::new(&highlighter_settings)),
            highlighter_settings,
        }
    }

    /// Whether an input method is composing text. An opened input method
    /// starts with an empty preedit, which is not composing yet.
    fn is_composing(&self) -> bool {
        self.preedit
            .as_ref()
            .is_some_and(|preedit| !preedit.content.is_empty())
    }

    /// Take the input method's preedit. Returns whether it starts a
    /// composition: the first non-empty preedit.
    fn set_preedit(&mut self, preedit: Option<input_method::Preedit>) -> bool {
        let starts =
            !self.is_composing() && preedit.as_ref().is_some_and(|new| !new.content.is_empty());
        self.preedit = preedit;
        starts
    }
}

/// Take a preedit; when it starts a composition, scroll the caret into view
/// so the candidate window has a real anchor. Returns whether it did.
fn take_preedit(state: &mut State, content: &Content, preedit: input_method::Preedit) -> bool {
    let starts = state.set_preedit(Some(preedit));
    if starts {
        content.0.borrow_mut().reveal_cursor();
    }
    starts
}

#[derive(Debug, Clone)]
struct Focus {
    updated_at: Instant,
    now: Instant,
    is_window_focused: bool,
}

impl Focus {
    const CURSOR_BLINK_INTERVAL_MILLIS: u128 = 500;

    fn now() -> Self {
        let now = Instant::now();
        Self {
            updated_at: now,
            now,
            is_window_focused: true,
        }
    }

    fn is_cursor_visible(&self) -> bool {
        self.is_window_focused
            && ((self.now - self.updated_at).as_millis() / Self::CURSOR_BLINK_INTERVAL_MILLIS)
                .is_multiple_of(2)
    }
}

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.focus.is_some()
    }

    fn focus(&mut self) {
        self.focus = Some(Focus::now());
    }

    fn unfocus(&mut self) {
        self.focus = None;
        // Preedit events reach only a focused editor, so one that ends while
        // another field has the keyboard would leave the guard on.
        self.preedit = None;
    }
}

impl Widget<Message, Theme, Renderer> for TextEditor<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(self.highlighter_settings))
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let mut editor = self.content.0.borrow_mut();
        let state = tree.state.downcast_mut::<State>();

        if state.highlighter_settings.font != self.highlighter_settings.font {
            editor.font_changing();
        }
        if state.highlighter_settings != self.highlighter_settings {
            state
                .highlighter
                .borrow_mut()
                .update(&self.highlighter_settings);
            state.highlighter_settings = self.highlighter_settings;
        }

        let limits = limits.width(Length::Fill).height(Length::Fill);
        editor.update(
            limits.shrink(self.padding(limits.max().width)).max(),
            self.highlighter_settings.font,
            Pixels(self.metrics.text),
            self.metrics.line_height(),
            Wrapping::WordOrGlyph,
            state.highlighter.borrow_mut().deref_mut(),
        );

        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let is_redraw = matches!(event, Event::Window(window::Event::RedrawRequested(_now)));

        match event {
            Event::Window(window::Event::Unfocused) => {
                if let Some(focus) = &mut state.focus {
                    focus.is_window_focused = false;
                }
            }
            Event::Window(window::Event::Focused) => {
                if let Some(focus) = &mut state.focus {
                    focus.is_window_focused = true;
                    focus.updated_at = Instant::now();
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(focus) = state.focus.as_mut().filter(|f| f.is_window_focused) {
                    focus.now = *now;
                    let millis_until_redraw = Focus::CURSOR_BLINK_INTERVAL_MILLIS
                        - (focus.now - focus.updated_at).as_millis()
                            % Focus::CURSOR_BLINK_INTERVAL_MILLIS;
                    shell.request_redraw_at(
                        focus.now + Duration::from_millis(millis_until_redraw as u64),
                    );
                }
            }
            _ => {}
        }

        if let Some(update) = Update::from_event(
            event,
            state,
            layout.bounds(),
            self.padding(layout.bounds().width),
            cursor,
            crate::keys::Context {
                markdown_assist: self.markdown_assist,
                composing: state.is_composing(),
                policy: self.key_policy,
            },
        ) {
            match update {
                Update::Click(click) => {
                    let action = match click.kind() {
                        mouse::click::Kind::Single => Action::Click(click.position()),
                        mouse::click::Kind::Double => Action::SelectWord,
                        mouse::click::Kind::Triple => Action::SelectLine,
                    };
                    state.focus = Some(Focus::now());
                    state.last_click = Some(click);
                    state.drag_click = Some(click.kind());
                    shell.publish(Message::Action(action));
                    shell.capture_event();
                }
                Update::Drag(position) => {
                    shell.publish(Message::Action(Action::Drag(position)));
                }
                Update::Release => {
                    state.drag_click = None;
                }
                Update::Scroll(lines) => {
                    let bounds = self.content.0.borrow().bounds();
                    if bounds.height >= i32::MAX as f32 {
                        return;
                    }
                    let lines = lines + state.partial_scroll;
                    state.partial_scroll = lines.fract();
                    shell.publish(Message::Action(Action::Scroll {
                        lines: lines as i32,
                    }));
                    shell.capture_event();
                }
                Update::InputMethod(update) => match update {
                    Ime::Toggle(is_open) => {
                        state.set_preedit(is_open.then(input_method::Preedit::new));
                        shell.request_redraw();
                    }
                    Ime::Preedit { content, selection } => {
                        let preedit = input_method::Preedit {
                            content,
                            selection,
                            text_size: Some(Pixels(self.metrics.text)),
                        };
                        // The iced engine scrolls to the caret when it is
                        // next laid out.
                        if take_preedit(state, self.content, preedit) {
                            shell.invalidate_layout();
                        }
                        shell.request_redraw();
                    }
                    Ime::Commit(text) => {
                        shell.publish(Message::Action(Action::Edit(Edit::Paste(Arc::new(text)))));
                    }
                },
                Update::Binding(binding) => {
                    if !matches!(binding, Binding::Unfocus) {
                        shell.capture_event();
                    }
                    apply_binding(binding, self.content, state, clipboard, shell);
                    if let Some(focus) = &mut state.focus {
                        focus.updated_at = Instant::now();
                    }
                }
            }
        }

        let status = {
            let is_hovered = cursor.is_over(layout.bounds());
            if state.focus.is_some() {
                Status::Focused { is_hovered }
            } else if is_hovered {
                Status::Hovered
            } else {
                Status::Active
            }
        };

        if is_redraw {
            self.last_status = Some(status);
            shell.request_input_method(&self.input_method(state, layout));
        } else if self
            .last_status
            .is_some_and(|last_status| status != last_status)
        {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let mut editor = self.content.0.borrow_mut();
        let state = tree.state.downcast_ref::<State>();
        let style = self.style;

        editor.highlight(
            self.highlighter_settings.font,
            state.highlighter.borrow_mut().deref_mut(),
        );

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            style.background,
        );

        let text_bounds = bounds.shrink(self.padding(bounds.width));
        let (column_x, column_width) = self.column(bounds);
        // Rows span the full width, but scroll out under the vertical padding.
        let rows_clip = Rectangle {
            x: bounds.x,
            y: text_bounds.y,
            width: bounds.width,
            height: text_bounds.height,
        };
        let rows = visual_rows(editor.buffer(), &state.highlighter.borrow());
        let cursor = editor.cursor();
        let cursor_line = cursor.position.line;

        // Code bands start just left of the text so the gutter stays clear.
        let code_left = text_bounds.x - CODE_BAND_OUTSET;
        for row in rows.iter().filter(|row| row.code) {
            let band = Rectangle {
                x: code_left,
                y: text_bounds.y + row.top,
                width: (column_x + column_width - ROW_INSET - code_left).max(0.0),
                height: row.height,
            };
            if let Some(band) = band.intersection(&rows_clip) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: band,
                        ..renderer::Quad::default()
                    },
                    style.code_block,
                );
            }
        }

        for row in rows.iter().filter(|row| row.line == cursor_line) {
            let band = Rectangle {
                x: column_x + ROW_INSET,
                y: text_bounds.y + row.top,
                width: (column_width - 2.0 * ROW_INSET).max(0.0),
                height: row.height,
            };
            if let Some(band) = band.intersection(&rows_clip) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: band,
                        ..renderer::Quad::default()
                    },
                    style.current_line,
                );
            }
        }

        let chip_height = self.metrics.text * CHIP_HEIGHT_EM;
        for row in &rows {
            for &(start, end) in &row.code_spans {
                let chip = Rectangle {
                    x: text_bounds.x + start - CHIP_PADDING_X,
                    y: text_bounds.y + row.top + (row.height - chip_height) / 2.0,
                    width: end - start + 2.0 * CHIP_PADDING_X,
                    height: chip_height,
                };
                if let Some(chip) = chip.intersection(&rows_clip) {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: chip,
                            border: Border::default().rounded(CHIP_RADIUS),
                            ..renderer::Quad::default()
                        },
                        style.code_block,
                    );
                }
            }
        }

        if self.line_numbers {
            let right = column_x + ROW_INSET + self.metrics.gutter - GUTTER_PADDING_RIGHT;
            clip(renderer, rows_clip, |renderer| {
                for row in rows.iter().filter(|row| row.starts_line) {
                    let color = if row.line == cursor_line {
                        style.gutter_current
                    } else {
                        style.gutter
                    };
                    renderer.fill_text(
                        Text {
                            content: (row.line + 1).to_string(),
                            bounds: Size::new(
                                self.metrics.gutter - GUTTER_PADDING_RIGHT,
                                self.metrics.line,
                            ),
                            size: Pixels(self.metrics.text),
                            line_height: self.metrics.line_height(),
                            font: self.highlighter_settings.font,
                            align_x: text::Alignment::Right,
                            align_y: alignment::Vertical::Top,
                            shaping: text::Shaping::Basic,
                            wrapping: Wrapping::None,
                        },
                        Point::new(right, text_bounds.y + row.number_top),
                        color,
                        rows_clip,
                    );
                }
            });
        }

        let translation = text_bounds.position() - Point::ORIGIN;
        let selection = state
            .focus
            .as_ref()
            .and_then(|focus| Some((focus, editor.selection()?)));

        if let Some((_, Selection::Range(ranges))) = &selection {
            let breaks = selected_breaks(&rows, cursor, self.metrics.text * ADVANCE_EM);
            for range in ranges
                .iter()
                .copied()
                .chain(breaks)
                .filter_map(|range| text_bounds.intersection(&(range + translation)))
            {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: range,
                        ..renderer::Quad::default()
                    },
                    style.text_selection,
                );
            }
        }

        // The owned engine hands the renderer a weak buffer reference, which
        // a later mutation would detach, so the text goes after every query.
        clip(renderer, text_bounds, |renderer| {
            editor.draw(renderer, text_bounds.position(), style.value, text_bounds);
        });

        // Quads sort beneath text within a layer; strikes and the caret must
        // paint over the glyphs.
        renderer.with_layer(rows_clip, |renderer| {
            for (row, &(start, end)) in rows
                .iter()
                .flat_map(|row| row.strikes.iter().map(move |strike| (row, strike)))
            {
                let y =
                    (text_bounds.y + row.baseline - self.metrics.text * X_HEIGHT_EM / 2.0).round();
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: text_bounds.x + start,
                            y,
                            width: end - start,
                            height: STRIKE_THICKNESS,
                        },
                        ..renderer::Quad::default()
                    },
                    style.strike,
                );
            }

            if let Some((focus, Selection::Caret(position))) = &selection {
                if focus.is_cursor_visible() {
                    let row =
                        row_height_at(editor.buffer(), position.y).unwrap_or(self.metrics.line);
                    // A heading's taller row gets a caret as much taller.
                    let height = (self.metrics.caret * row / self.metrics.line)
                        .round()
                        .min(row);
                    let caret = Rectangle {
                        x: position.x + translation.x,
                        y: position.y + translation.y + ((row - height) / 2.0).round(),
                        width: CARET_WIDTH,
                        height,
                    };
                    if let Some(caret) = caret.intersection(&rows_clip) {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: caret,
                                ..renderer::Quad::default()
                            },
                            style.caret,
                        );
                    }
                }
            }
        });
    }

    fn mouse_interaction(
        &self,
        _tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        operation.focusable(Some(&self.id), layout.bounds(), state);
    }
}

impl<'a> From<TextEditor<'a>> for Element<'a, Message> {
    fn from(text_editor: TextEditor<'a>) -> Self {
        Self::new(text_editor)
    }
}

/// How far inside its clip rectangle [`clip`] places the layer.
const CLIP_INSET: f32 = 0.01;

/// Draws `draw` clipped to `bounds` on every renderer. tiny-skia masks text
/// only when the text's own extent pokes out of its layer, and the editor's
/// text and gutter numbers are sized exactly to their areas, so a row
/// scrolled half out of view would paint into the padding. A layer a hair
/// inside `bounds` makes it mask; wgpu rounds its scissor to whole pixels,
/// where the hair vanishes.
fn clip(renderer: &mut Renderer, bounds: Rectangle, draw: impl FnOnce(&mut Renderer)) {
    renderer.with_layer(bounds.shrink(CLIP_INSET), draw);
}

fn apply_binding(
    binding: Binding<Message>,
    content: &Content,
    state: &mut State,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, Message>,
) {
    let mut publish = |action| shell.publish(Message::Action(action));

    match binding {
        Binding::Unfocus => {
            operation::Focusable::unfocus(state);
            state.drag_click = None;
        }
        Binding::Copy => {
            if let Some(selection) = content.selection() {
                clipboard.write(clipboard::Kind::Standard, selection);
            }
        }
        Binding::Cut => {
            if let Some(selection) = content.selection() {
                clipboard.write(clipboard::Kind::Standard, selection);
                publish(Action::Edit(Edit::Delete));
            }
        }
        Binding::Paste => {
            if let Some(contents) = clipboard.read(clipboard::Kind::Standard) {
                publish(Action::Edit(Edit::Paste(Arc::new(contents))));
            }
        }
        Binding::Move(motion) => publish(Action::Move(motion)),
        Binding::Select(motion) => publish(Action::Select(motion)),
        Binding::SelectWord => publish(Action::SelectWord),
        Binding::SelectLine => publish(Action::SelectLine),
        Binding::SelectAll => publish(Action::SelectAll),
        Binding::Insert(c) => publish(Action::Edit(Edit::Insert(c))),
        Binding::Enter => publish(Action::Edit(Edit::Enter)),
        Binding::Backspace => publish(Action::Edit(Edit::Backspace)),
        Binding::Delete => publish(Action::Edit(Edit::Delete)),
        Binding::Sequence(sequence) => {
            for binding in sequence {
                apply_binding(binding, content, state, clipboard, shell);
            }
        }
        Binding::Custom(message) => shell.publish(message),
    }
}

enum Update {
    Click(mouse::Click),
    Drag(Point),
    Release,
    Scroll(f32),
    InputMethod(Ime),
    Binding(Binding<Message>),
}

enum Ime {
    Toggle(bool),
    Preedit {
        content: String,
        selection: Option<Range<usize>>,
    },
    Commit(String),
}

impl Update {
    fn from_event(
        event: &Event,
        state: &State,
        bounds: Rectangle,
        padding: Padding,
        cursor: mouse::Cursor,
        keys: crate::keys::Context<'_>,
    ) -> Option<Self> {
        match event {
            Event::Mouse(event) => match event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if let Some(cursor_position) = cursor.position_in(bounds) {
                        let cursor_position =
                            cursor_position - Vector::new(padding.left, padding.top);
                        let click = mouse::Click::new(
                            cursor_position,
                            mouse::Button::Left,
                            state.last_click,
                        );
                        Some(Update::Click(click))
                    } else if state.focus.is_some() {
                        Some(Update::Binding(Binding::Unfocus))
                    } else {
                        None
                    }
                }
                mouse::Event::ButtonReleased(mouse::Button::Left) => Some(Update::Release),
                mouse::Event::CursorMoved { .. } => match state.drag_click {
                    Some(mouse::click::Kind::Single) => {
                        let cursor_position =
                            cursor.position_in(bounds)? - Vector::new(padding.left, padding.top);
                        Some(Update::Drag(cursor_position))
                    }
                    _ => None,
                },
                mouse::Event::WheelScrolled { delta } if cursor.is_over(bounds) => {
                    Some(Update::Scroll(match delta {
                        mouse::ScrollDelta::Lines { y, .. } => {
                            if y.abs() > 0.0 {
                                y.signum() * -(y.abs() * 4.0).max(1.0)
                            } else {
                                0.0
                            }
                        }
                        mouse::ScrollDelta::Pixels { y, .. } => -y / 4.0,
                    }))
                }
                _ => None,
            },
            Event::InputMethod(event) => match event {
                input_method::Event::Opened | input_method::Event::Closed => Some(
                    Update::InputMethod(Ime::Toggle(matches!(event, input_method::Event::Opened))),
                ),
                input_method::Event::Preedit(content, selection) if state.focus.is_some() => {
                    Some(Update::InputMethod(Ime::Preedit {
                        content: content.clone(),
                        selection: selection.clone(),
                    }))
                }
                input_method::Event::Commit(content) if state.focus.is_some() => {
                    Some(Update::InputMethod(Ime::Commit(content.clone())))
                }
                _ => None,
            },
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                modifiers,
                text,
                ..
            }) => {
                let status = if state.focus.is_some() {
                    Status::Focused {
                        is_hovered: cursor.is_over(bounds),
                    }
                } else {
                    Status::Active
                };
                crate::keys::key_binding(
                    KeyPress {
                        key: key.clone(),
                        modified_key: modified_key.clone(),
                        physical_key: *physical_key,
                        modifiers: *modifiers,
                        text: text.clone(),
                        status,
                    },
                    keys,
                )
                .map(Update::Binding)
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Line geometry
// ---------------------------------------------------------------------------

/// One visible visual row of the editor, relative to the text origin.
#[derive(Debug, Clone, PartialEq)]
struct VisualRow {
    /// Zero-based logical line.
    line: usize,
    top: f32,
    height: f32,
    baseline: f32,
    /// Where a body-row-tall box for the line number goes, so the number's
    /// baseline meets the row's; on a body row that is `top`.
    number_top: f32,
    /// Whether this is the first visual row of its logical line.
    starts_line: bool,
    /// Whether this is the last visual row of its logical line, which may
    /// lie below the viewport.
    ends_line: bool,
    /// Where the row's glyphs end.
    width: f32,
    /// Horizontal `(start, end)` of each strike-through on the row.
    strikes: Vec<(f32, f32)>,
    /// Horizontal `(start, end)` of each inline code span on the row.
    code_spans: Vec<(f32, f32)>,
    /// Whether the row belongs to a fenced code block.
    code: bool,
}

/// Collects the visible rows of `buffer` (already scrolled by the editor),
/// with the decoration `highlighter` left for each line.
fn visual_rows(buffer: &cosmic_text::Buffer, highlighter: &MarkdownHighlighter) -> Vec<VisualRow> {
    let body = buffer.metrics();
    buffer
        .layout_runs()
        .map(|run| {
            let decor = highlighter.decor(run.line_i);
            let spans = |ranges: &[Range<usize>]| -> Vec<(f32, f32)> {
                ranges
                    .iter()
                    .filter_map(|range| {
                        glyph_span(
                            run.text,
                            run.glyphs
                                .iter()
                                .map(|glyph| (glyph.start..glyph.end, glyph.x, glyph.w)),
                            range.clone(),
                        )
                    })
                    .collect()
            };
            let largest = run
                .glyphs
                .iter()
                .map(|glyph| glyph.font_size)
                .fold(body.font_size, f32::max);
            VisualRow {
                line: run.line_i,
                top: run.line_top,
                height: run.line_height,
                baseline: run.line_y,
                number_top: number_top(
                    run.line_top,
                    run.line_height,
                    run.line_y,
                    largest / body.font_size,
                    body.line_height,
                ),
                width: run.line_w,
                starts_line: starts_line(run.glyphs.iter().map(|glyph| glyph.start)),
                ends_line: buffer.lines[run.line_i]
                    .layout_opt()
                    .and_then(|layout| layout.last())
                    .is_none_or(|last| std::ptr::eq(last.glyphs.as_slice(), run.glyphs)),
                code: decor.is_some_and(|decor| decor.code),
                strikes: decor.map(|decor| spans(&decor.strikes)).unwrap_or_default(),
                code_spans: decor
                    .map(|decor| spans(&decor.code_spans))
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// The top of a `body_height` box whose body-sized text shares its baseline
/// with a row at `top`, `height` tall, with its baseline at `baseline`, whose
/// largest glyphs are `scale` times the body size. cosmic-text centres a
/// row's glyph box in it, so the baseline sits `(ascent - descent) / 2` below
/// the row's middle; for body glyphs that offset is the row's divided by
/// `scale`.
fn number_top(top: f32, height: f32, baseline: f32, scale: f32, body_height: f32) -> f32 {
    let offset = baseline - (top + height / 2.0);
    baseline - body_height / 2.0 - offset / scale
}

/// The height of the visual row at `y` (relative to the text origin), the
/// top of which a caret or selection rectangle shares.
fn row_height_at(buffer: &cosmic_text::Buffer, y: f32) -> Option<f32> {
    buffer
        .layout_runs()
        .find(|run| y + 0.5 >= run.line_top && y < run.line_top + run.line_height)
        .map(|run| run.line_height)
}

/// A cell `advance` wide after each selected line break, at the end of its
/// line's last row, so a selection reads as one block across empty lines.
/// The breaks are those of the lines from the selection's first through
/// the one before its last.
fn selected_breaks(rows: &[VisualRow], cursor: Cursor, advance: f32) -> Vec<Rectangle> {
    let Some(anchor) = cursor.selection else {
        return Vec::new();
    };
    let first = anchor.line.min(cursor.position.line);
    let last = anchor.line.max(cursor.position.line);
    rows.iter()
        .filter(|row| row.ends_line && (first..last).contains(&row.line))
        .map(|row| Rectangle {
            x: row.width,
            y: row.top,
            width: advance,
            height: row.height,
        })
        .collect()
}

/// A wrapped continuation row starts past byte 0; an empty line has no glyphs.
fn starts_line(glyph_starts: impl Iterator<Item = usize>) -> bool {
    glyph_starts.min().is_none_or(|start| start == 0)
}

/// Horizontal extent of the non-blank glyphs of one row that fall inside
/// byte `range` of `line`. Glyphs are `(bytes, x, width)`.
fn glyph_span(
    line: &str,
    glyphs: impl Iterator<Item = (Range<usize>, f32, f32)>,
    range: Range<usize>,
) -> Option<(f32, f32)> {
    glyphs
        .filter(|(bytes, _, _)| bytes.start >= range.start && bytes.end <= range.end)
        .filter(|(bytes, _, _)| {
            line.get(bytes.clone())
                .is_some_and(|text| !text.trim().is_empty())
        })
        .map(|(_, x, width)| (x, x + width))
        .reduce(|(start, end), (x0, x1)| (start.min(x0), end.max(x1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    const LINE_HEIGHT: f32 = 24.0;

    #[test]
    fn metrics_scale_from_the_mockup_sizes() {
        assert_eq!(
            Metrics::default(),
            Metrics {
                text: 14.0,
                line: 24.0,
                caret: 18.0,
                gutter: 52.0,
            }
        );
        assert_eq!(
            Metrics::for_size(21.0),
            Metrics {
                text: 21.0,
                line: 36.0,
                caret: 27.0,
                gutter: 69.0,
            }
        );
    }

    #[test]
    fn rows_follow_the_line_height_ratio_and_keep_the_caret_inside() {
        assert_eq!(Metrics::new(14.0, 2.0).line, 28.0);
        assert_eq!(Metrics::new(14.0, 1.3).line, 18.0);
        let tight = Metrics::new(14.0, 1.2);
        assert_eq!((tight.line, tight.caret), (17.0, 17.0));
        // A new text size keeps the ratio.
        assert_eq!(Metrics::new(20.0, 2.0).line, 40.0);
        assert_eq!(
            Metrics::new(20.0, 2.0).gutter,
            Metrics::for_size(20.0).gutter
        );
    }

    #[test]
    fn losing_focus_ends_a_composition() {
        use operation::Focusable as _;

        let mut state = State::new(HighlighterSettings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
            front_matter_end: None,
        });
        state.focus();
        state.preedit = Some(input_method::Preedit {
            content: "にほ".into(),
            selection: None,
            text_size: None,
        });
        assert!(state.is_composing());
        state.unfocus();
        state.focus();
        assert!(!state.is_composing());
    }

    fn preedit(content: &str) -> input_method::Preedit {
        input_method::Preedit {
            content: content.into(),
            selection: None,
            text_size: None,
        }
    }

    #[test]
    fn only_the_first_non_empty_preedit_starts_a_composition() {
        let mut state = State::new(HighlighterSettings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
            front_matter_end: None,
        });
        // An input method opens with an empty preedit.
        assert!(!state.set_preedit(Some(input_method::Preedit::new())));
        assert!(state.set_preedit(Some(preedit("に"))));
        assert!(!state.set_preedit(Some(preedit("にほ"))));
        // Cleared, it can start again.
        assert!(!state.set_preedit(Some(preedit(""))));
        assert!(state.set_preedit(Some(preedit("ん"))));
        assert!(!state.set_preedit(None));
        assert!(state.set_preedit(Some(preedit("a"))));
    }

    #[test]
    fn a_composition_starting_off_screen_scrolls_the_caret_into_view() {
        fixtures::load_font();
        let text = (0..60)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let size = Size::new(300.0, 120.0);
        for kind in BackendKind::ALL {
            let content = Content::new(kind, &text);
            let mut highlighter = MarkdownHighlighter::new(&HighlighterSettings {
                palette: fixtures::DARK,
                font: fixtures::FONT,
                front_matter_end: None,
            });
            let lay_out = |content: &Content, highlighter: &mut MarkdownHighlighter| {
                content.0.borrow_mut().update(
                    size,
                    fixtures::FONT,
                    Pixels(TEXT_SIZE),
                    line_height(),
                    Wrapping::WordOrGlyph,
                    highlighter,
                );
            };
            lay_out(&content, &mut highlighter);
            let scroll_by = |content: &Content, highlighter: &mut MarkdownHighlighter, lines| {
                content.0.borrow_mut().perform(Action::Scroll { lines });
                lay_out(content, highlighter);
            };
            scroll_by(&content, &mut highlighter, 30);
            let away = content.scroll();
            assert!(away.line > 0, "{kind:?}");
            let mut state = State::new(HighlighterSettings {
                palette: fixtures::DARK,
                font: fixtures::FONT,
                front_matter_end: None,
            });

            // Opening the input method moves nothing; composing does.
            state.set_preedit(Some(input_method::Preedit::new()));
            assert!(
                take_preedit(&mut state, &content, preedit("に")),
                "{kind:?}"
            );
            lay_out(&content, &mut highlighter);
            assert_eq!(content.scroll().line, 0, "{kind:?}");
            assert_eq!(content.cursor().position.line, 0, "{kind:?}");
            assert!(
                matches!(content.0.borrow().selection(), Some(Selection::Caret(_))),
                "{kind:?}"
            );

            // Scrolling away mid-composition stays put on the next preedit.
            scroll_by(&content, &mut highlighter, 30);
            assert!(
                !take_preedit(&mut state, &content, preedit("にほ")),
                "{kind:?}"
            );
            lay_out(&content, &mut highlighter);
            assert_eq!(content.scroll(), away, "{kind:?}");

            // One long wrapped line, scrolled down with the caret at its start.
            let content = Content::new(kind, &"word ".repeat(400));
            lay_out(&content, &mut highlighter);
            scroll_by(&content, &mut highlighter, 10);
            assert!(content.scroll().vertical > 0.0, "{kind:?}");
            let mut state = State::new(HighlighterSettings {
                palette: fixtures::DARK,
                font: fixtures::FONT,
                front_matter_end: None,
            });
            assert!(
                take_preedit(&mut state, &content, preedit("に")),
                "{kind:?}"
            );
            lay_out(&content, &mut highlighter);
            assert_eq!(content.scroll().vertical, 0.0, "{kind:?}");
            assert_eq!(content.cursor().position.column, 0, "{kind:?}");
            assert!(
                matches!(
                    content.0.borrow().selection(),
                    Some(Selection::Caret(caret)) if caret == Point::ORIGIN
                ),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn continuation_rows_do_not_start_a_line() {
        assert!(starts_line([].into_iter()));
        assert!(starts_line([0, 1, 2].into_iter()));
        assert!(starts_line([2, 1, 0].into_iter()));
        assert!(!starts_line([14, 15].into_iter()));
    }

    #[test]
    fn strike_covers_struck_glyphs_without_edge_blanks() {
        let line = "- [x] ab cd ";
        let glyphs = line
            .char_indices()
            .map(|(i, c)| (i..i + c.len_utf8(), i as f32 * 8.0, 8.0));
        // Bytes 6..11 are "ab cd"; the marker and trailing blank stay clear.
        assert_eq!(glyph_span(line, glyphs, 6..12), Some((48.0, 88.0)));
    }

    #[test]
    fn strike_on_a_wrapped_row_covers_only_that_row() {
        let line = "- [x] alpha beta";
        // The second visual row holds "beta" starting again at x = 0.
        let row = [(12..13, 0.0, 8.0), (13..14, 8.0, 8.0), (14..15, 16.0, 8.0)];
        assert_eq!(glyph_span(line, row.into_iter(), 6..16), Some((0.0, 24.0)));
        let blank_row = [(11..12, 0.0, 8.0)];
        assert_eq!(glyph_span(line, blank_row.into_iter(), 6..16), None);
    }

    #[test]
    fn padding_reserves_the_gutter_only_when_shown() {
        let content = Content::new(BackendKind::Iced, "");
        let editor = |line_numbers| {
            TextEditor::new(
                &content,
                "e",
                fixtures::SETTINGS,
                fixtures::STYLE,
                line_numbers,
                Metrics::default(),
                true,
            )
        };
        let shown = editor(true).padding(600.0);
        let hidden = editor(false).padding(600.0);
        assert_eq!((shown.left, shown.top, shown.bottom), (60.0, 24.0, 24.0));
        assert_eq!(hidden.left, 16.0);
    }

    #[test]
    fn a_maximum_width_centres_the_column_once_the_widget_is_wider() {
        let content = Content::new(BackendKind::Iced, "");
        let editor = TextEditor::new(
            &content,
            "e",
            fixtures::SETTINGS,
            fixtures::STYLE,
            true,
            Metrics::default(),
            true,
        )
        .max_width(Some(760.0));
        let narrow = editor.padding(600.0);
        assert_eq!((narrow.left, narrow.right), (60.0, 8.0));
        let wide = editor.padding(1001.0);
        assert_eq!((wide.left, wide.right), (60.0 + 120.0, 8.0 + 120.0));
        let bounds = Rectangle::new(Point::new(280.0, 98.0), Size::new(1000.0, 600.0));
        assert_eq!(editor.column(bounds), (400.0, 760.0));
        let text = bounds.shrink(editor.padding(bounds.width));
        // The text column is the 760 px column less the gutter and insets.
        assert_eq!((text.x, text.width), (460.0, 692.0));
        let uncapped = editor.max_width(None);
        assert_eq!(uncapped.column(bounds), (280.0, 1000.0));
    }

    fn laid_out(kind: BackendKind, text: &str, width: f32) -> Vec<VisualRow> {
        laid_out_in(kind, text, Size::new(width, 1000.0))
    }

    fn laid_out_in(kind: BackendKind, text: &str, size: Size) -> Vec<VisualRow> {
        fixtures::load_font();
        let mut editor = Backend::with_text(kind, text);
        let mut highlighter = MarkdownHighlighter::new(&HighlighterSettings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
            front_matter_end: None,
        });
        editor.update(
            size,
            fixtures::FONT,
            Pixels(TEXT_SIZE),
            line_height(),
            Wrapping::WordOrGlyph,
            &mut highlighter,
        );
        editor.highlight(fixtures::FONT, &mut highlighter);
        visual_rows(editor.buffer(), &highlighter)
    }

    #[test]
    fn wrapped_lines_number_only_their_first_visual_row() {
        for kind in BackendKind::ALL {
            let rows = laid_out(
                kind,
                "short\nwrapped words wrapped words wrapped words\n\nend",
                120.0,
            );
            let numbered: Vec<_> = rows
                .iter()
                .filter(|row| row.starts_line)
                .map(|row| row.line)
                .collect();
            assert_eq!(numbered, [0, 1, 2, 3], "{kind:?}");
            assert!(rows.iter().filter(|row| row.line == 1).count() > 1);
            for pair in rows.windows(2) {
                assert_eq!(pair[1].top, pair[0].top + LINE_HEIGHT);
            }
        }
    }

    #[test]
    fn headings_get_taller_rows_on_the_owned_engine_only() {
        let text = "# A long first-level heading that wraps\nbody\n## Two\n### Three\n#### Four\n#";
        let heights = |kind| -> Vec<(usize, f32)> {
            laid_out(kind, text, 300.0)
                .iter()
                .map(|row| (row.line, row.height))
                .collect()
        };
        let classic = heights(BackendKind::Iced);
        assert!(classic.iter().all(|&(_, height)| height == LINE_HEIGHT));
        let owned = heights(BackendKind::Owned);
        // round(24 × 1.35), round(24 × 1.18), round(24 × 1.05).
        assert_eq!(
            owned,
            [
                (0, 32.0),
                (0, 32.0),
                (1, 24.0),
                (2, 28.0),
                (3, 25.0),
                (4, 24.0),
                (5, 32.0)
            ]
        );
        let rows = laid_out(BackendKind::Owned, text, 300.0);
        for pair in rows.windows(2) {
            assert_eq!(pair[1].top, pair[0].top + pair[0].height);
        }
    }

    #[test]
    fn line_numbers_share_the_baseline_of_their_row() {
        let rows = laid_out(BackendKind::Owned, "# Title\nbody\n## Two\n\n#", 600.0);
        let body = &rows[1];
        assert_eq!(body.number_top, body.top);
        // A body-sized number set at `number_top` puts its baseline where a
        // body row puts its own.
        let body_baseline = body.baseline - body.top;
        for row in [&rows[0], &rows[2], &rows[4]] {
            assert!(row.height > LINE_HEIGHT);
            assert!(
                (row.number_top + body_baseline - row.baseline).abs() < 0.01,
                "{row:?}"
            );
        }
        // An empty row centres it.
        let empty = &rows[3];
        assert!((empty.number_top - empty.top).abs() < 0.01);
        for kind in BackendKind::ALL {
            for row in laid_out(kind, "plain\n- [x] done", 600.0) {
                assert!((row.number_top - row.top).abs() < 0.01, "{kind:?}");
            }
        }
    }

    #[test]
    fn selected_line_breaks_get_a_cell_at_the_end_of_their_last_row() {
        for kind in BackendKind::ALL {
            let rows = laid_out(kind, "ab\n\nwrapped words wrapped words\nlast", 120.0);
            let select = |from: (usize, usize), to: (usize, usize)| Cursor {
                position: Position {
                    line: to.0,
                    column: to.1,
                },
                selection: Some(Position {
                    line: from.0,
                    column: from.1,
                }),
            };
            let cells = selected_breaks(&rows, select((3, 2), (0, 1)), 8.0);
            let wrapped: Vec<_> = rows.iter().filter(|row| row.line == 2).collect();
            assert!(wrapped.len() > 1, "{kind:?}");
            let expected = [
                (rows[0].width, rows[0].top),
                (0.0, rows[1].top),
                (wrapped.last().unwrap().width, wrapped.last().unwrap().top),
            ];
            let found: Vec<_> = cells.iter().map(|cell| (cell.x, cell.y)).collect();
            assert_eq!(found, expected, "{kind:?}");
            assert!(rows[0].width > 0.0);
            assert!(cells
                .iter()
                .all(|cell| cell.width == 8.0 && cell.height == LINE_HEIGHT));
            // A selection within one line, or none, selects no break.
            assert!(selected_breaks(&rows, select((2, 0), (2, 5)), 8.0).is_empty());
            let caret = Cursor {
                position: Position { line: 1, column: 0 },
                selection: None,
            };
            assert!(selected_breaks(&rows, caret, 8.0).is_empty());

            // A line that wraps on below the viewport has no break in view.
            let text = "a\nwrapped words wrapped words wrapped words\nz";
            let rows = laid_out_in(kind, text, Size::new(120.0, 2.0 * LINE_HEIGHT));
            assert_eq!(rows.iter().map(|row| row.line).collect::<Vec<_>>(), [0, 1]);
            let cells = selected_breaks(&rows, select((0, 0), (2, 1)), 8.0);
            let found: Vec<_> = cells.iter().map(|cell| (cell.x, cell.y)).collect();
            assert_eq!(found, [(rows[0].width, rows[0].top)], "{kind:?}");
        }
    }

    #[test]
    fn inline_code_spans_measure_their_chips_from_the_glyphs() {
        for kind in BackendKind::ALL {
            let rows = laid_out(kind, "run `x y` now\n- [x] `done`", 600.0);
            assert_eq!(rows[0].code_spans.len(), 1, "{kind:?}");
            let (start, end) = rows[0].code_spans[0];
            // "run " is four cells, and "`x y`" five more.
            let cell = start / 4.0;
            assert!(cell > 7.0 && cell < 10.0, "{start}");
            assert!((end - start - 5.0 * cell).abs() < 0.5, "{start}..{end}");
            assert!(rows[1].code_spans.is_empty());
        }
    }

    #[test]
    fn only_completed_tasks_outside_code_are_struck() {
        for kind in BackendKind::ALL {
            let rows = laid_out(
                kind,
                "- [x] done\n- [ ] open\n```\n- [x] code\n```\n- [X] Done\n- a\n  - [x] b",
                600.0,
            );
            let struck: Vec<_> = rows
                .iter()
                .filter(|row| !row.strikes.is_empty())
                .map(|row| row.line)
                .collect();
            assert_eq!(struck, [0, 5, 7], "{kind:?}");
            let (start, end) = rows[0].strikes[0];
            // "- [x] " is six monospace cells; "done" is four more.
            let cell = start / 6.0;
            assert!(cell > 7.0 && cell < 10.0, "{start}");
            assert!((end - start - 4.0 * cell).abs() < 0.5, "{start}..{end}");
        }
    }
}
