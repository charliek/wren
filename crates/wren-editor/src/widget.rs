//! Wren's Markdown text-editor widget.
//!
//! Forked from `iced_widget` 0.14.2, `src/text_editor.rs`
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
//! Wren changes: concrete renderer, theme, highlighter, and message types; no
//! placeholder; and extra drawing for the mockup's line-number gutter,
//! current-line band, accent caret, and completed-task strike-through. Stock
//! `Content` hides its renderer editor, and the line geometry lives in that
//! editor's cosmic-text buffer, so this module owns its own `Content`.

use std::cell::RefCell;
use std::ops::{DerefMut, Range};
use std::sync::Arc;

use iced::advanced::graphics::text::cosmic_text;
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
    Action, Binding, Cursor, Edit, KeyPress, LineEnding, Selection, Status,
};
use iced::{
    alignment, window, Element, Event, Length, Padding, Pixels, Point, Rectangle, Renderer, Size,
    Theme, Vector,
};

use crate::backend::{Backend, BackendKind, Lines};
use crate::highlighter::{self, MarkdownHighlighter, Settings as HighlighterSettings};
use crate::{Message, Style};

const TEXT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 24.0;
/// Horizontal inset of each mockup row (`editorWrap` padding).
const ROW_INSET: f32 = 8.0;
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
const STRIKE_THICKNESS: f32 = 1.0;

/// Text size, row height, caret height, and gutter width, scaled together
/// from the mockup's 14 px text / 24 px rows / 18 px caret / 52 px gutter.
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
    pub fn for_size(text: f32) -> Self {
        let scale = text / TEXT_SIZE;
        Self {
            text,
            line: (LINE_HEIGHT * scale).round(),
            caret: (CARET_HEIGHT * scale).round(),
            gutter: GUTTER_PADDING_RIGHT + ((GUTTER_WIDTH - GUTTER_PADDING_RIGHT) * scale).round(),
        }
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
    ) -> Self {
        Self {
            id: widget::Id::from(id),
            content,
            highlighter_settings,
            style,
            line_numbers,
            metrics,
            last_status: None,
        }
    }

    fn padding(&self) -> Padding {
        let left = if self.line_numbers {
            ROW_INSET + self.metrics.gutter
        } else {
            ROW_INSET + BARE_TEXT_INSET
        };
        Padding {
            top: VERTICAL_PADDING,
            right: ROW_INSET,
            bottom: VERTICAL_PADDING,
            left,
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

        let text_bounds = layout.bounds().shrink(self.padding());
        let translation = text_bounds.position() - Point::ORIGIN;
        let Some(cursor) = self.content.0.borrow().ime_position() else {
            return InputMethod::Disabled;
        };

        InputMethod::Enabled {
            cursor: Rectangle::new(cursor + translation, Size::new(1.0, self.metrics.line)),
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
    }
}

impl Widget<Message, Theme, Renderer> for TextEditor<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            focus: None,
            preedit: None,
            last_click: None,
            drag_click: None,
            partial_scroll: 0.0,
            highlighter: RefCell::new(MarkdownHighlighter::new(&self.highlighter_settings)),
            highlighter_settings: self.highlighter_settings,
        })
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
            limits.shrink(self.padding()).max(),
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

        if let Some(update) =
            Update::from_event(event, state, layout.bounds(), self.padding(), cursor)
        {
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
                        state.preedit = is_open.then(input_method::Preedit::new);
                        shell.request_redraw();
                    }
                    Ime::Preedit { content, selection } => {
                        state.preedit = Some(input_method::Preedit {
                            content,
                            selection,
                            text_size: Some(Pixels(self.metrics.text)),
                        });
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

        let text_bounds = bounds.shrink(self.padding());
        // Rows span the full width, but scroll out under the vertical padding.
        let rows_clip = Rectangle {
            x: bounds.x,
            y: text_bounds.y,
            width: bounds.width,
            height: text_bounds.height,
        };
        let rows = visual_rows(editor.buffer());
        let cursor_line = editor.cursor().position.line;

        for row in rows.iter().filter(|row| row.line == cursor_line) {
            let band = Rectangle {
                x: bounds.x + ROW_INSET,
                y: text_bounds.y + row.top,
                width: (bounds.width - 2.0 * ROW_INSET).max(0.0),
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

        if self.line_numbers {
            let right = bounds.x + ROW_INSET + self.metrics.gutter - GUTTER_PADDING_RIGHT;
            clip(renderer, rows_clip, |renderer| {
                for row in rows.iter().filter(|row| row.starts_line) {
                    renderer.fill_text(
                        Text {
                            content: (row.line + 1).to_string(),
                            bounds: Size::new(
                                self.metrics.gutter - GUTTER_PADDING_RIGHT,
                                row.height,
                            ),
                            size: Pixels(self.metrics.text),
                            line_height: self.metrics.line_height(),
                            font: self.highlighter_settings.font,
                            align_x: text::Alignment::Right,
                            align_y: alignment::Vertical::Top,
                            shaping: text::Shaping::Basic,
                            wrapping: Wrapping::None,
                        },
                        Point::new(right, text_bounds.y + row.top),
                        style.gutter,
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
            for range in ranges
                .iter()
                .filter_map(|range| text_bounds.intersection(&(*range + translation)))
            {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: range,
                        ..renderer::Quad::default()
                    },
                    style.selection,
                );
            }
        }

        // The Cosmic engine hands the renderer a weak buffer reference, which
        // a later mutation would detach, so the text goes after every query.
        clip(renderer, text_bounds, |renderer| {
            editor.draw(renderer, text_bounds.position(), style.value, text_bounds);
        });

        // Quads sort beneath text within a layer; strikes and the caret must
        // paint over the glyphs.
        renderer.with_layer(rows_clip, |renderer| {
            for row in &rows {
                let Some((start, end)) = row.strike else {
                    continue;
                };
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
                    let caret = Rectangle {
                        x: position.x + translation.x,
                        y: position.y
                            + translation.y
                            + (self.metrics.line - self.metrics.caret) / 2.0,
                        width: CARET_WIDTH,
                        height: self.metrics.caret,
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
            state.focus = None;
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
                crate::keys::key_binding(KeyPress {
                    key: key.clone(),
                    modified_key: modified_key.clone(),
                    physical_key: *physical_key,
                    modifiers: *modifiers,
                    text: text.clone(),
                    status,
                })
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
    /// Whether this is the first visual row of its logical line.
    starts_line: bool,
    /// Horizontal `(start, end)` of the completed-task strike, if any.
    strike: Option<(f32, f32)>,
}

/// Collects the visible rows of `buffer` (already scrolled by the editor).
fn visual_rows(buffer: &cosmic_text::Buffer) -> Vec<VisualRow> {
    let runs: Vec<_> = buffer.layout_runs().collect();
    let Some(last_line) = runs.last().map(|run| run.line_i) else {
        return Vec::new();
    };
    // Fence state crosses lines, so scan from the top through the last visible line.
    let struck: Vec<_> =
        highlighter::done_task_ranges(buffer.lines[..=last_line].iter().map(|line| line.text()))
            .collect();

    runs.iter()
        .map(|run| VisualRow {
            line: run.line_i,
            top: run.line_top,
            height: run.line_height,
            baseline: run.line_y,
            starts_line: starts_line(run.glyphs.iter().map(|glyph| glyph.start)),
            strike: struck[run.line_i].clone().and_then(|range| {
                strike_span(
                    run.text,
                    run.glyphs
                        .iter()
                        .map(|glyph| (glyph.start..glyph.end, glyph.x, glyph.w)),
                    range,
                )
            }),
        })
        .collect()
}

/// A wrapped continuation row starts past byte 0; an empty line has no glyphs.
fn starts_line(glyph_starts: impl Iterator<Item = usize>) -> bool {
    glyph_starts.min().is_none_or(|start| start == 0)
}

/// Horizontal extent of the non-blank glyphs of one row that fall inside the
/// struck byte `range` of `line`. Glyphs are `(bytes, x, width)`.
fn strike_span(
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
    use crate::fixtures;

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
        assert_eq!(strike_span(line, glyphs, 6..12), Some((48.0, 88.0)));
    }

    #[test]
    fn strike_on_a_wrapped_row_covers_only_that_row() {
        let line = "- [x] alpha beta";
        // The second visual row holds "beta" starting again at x = 0.
        let row = [(12..13, 0.0, 8.0), (13..14, 8.0, 8.0), (14..15, 16.0, 8.0)];
        assert_eq!(strike_span(line, row.into_iter(), 6..16), Some((0.0, 24.0)));
        let blank_row = [(11..12, 0.0, 8.0)];
        assert_eq!(strike_span(line, blank_row.into_iter(), 6..16), None);
    }

    #[test]
    fn padding_reserves_the_gutter_only_when_shown() {
        let content = Content::new(BackendKind::Iced, "");
        let settings = HighlighterSettings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
        };
        let style = fixtures::STYLE;
        let shown =
            TextEditor::new(&content, "e", settings, style, true, Metrics::default()).padding();
        let hidden =
            TextEditor::new(&content, "e", settings, style, false, Metrics::default()).padding();
        assert_eq!((shown.left, shown.top, shown.bottom), (60.0, 24.0, 24.0));
        assert_eq!(hidden.left, 16.0);
    }

    fn laid_out(kind: BackendKind, text: &str, width: f32) -> Backend {
        fixtures::load_font();
        let mut editor = Backend::with_text(kind, text);
        let mut highlighter = MarkdownHighlighter::new(&HighlighterSettings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
        });
        editor.update(
            Size::new(width, 1000.0),
            fixtures::FONT,
            Pixels(TEXT_SIZE),
            line_height(),
            Wrapping::WordOrGlyph,
            &mut highlighter,
        );
        editor
    }

    #[test]
    fn wrapped_lines_number_only_their_first_visual_row() {
        for kind in BackendKind::ALL {
            let editor = laid_out(
                kind,
                "short\nwrapped words wrapped words wrapped words\n\nend",
                120.0,
            );
            let rows = visual_rows(editor.buffer());
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
    fn only_completed_tasks_outside_code_are_struck() {
        for kind in BackendKind::ALL {
            let editor = laid_out(
                kind,
                "- [x] done\n- [ ] open\n```\n- [x] code\n```\n- [X] Done",
                600.0,
            );
            let struck: Vec<_> = visual_rows(editor.buffer())
                .iter()
                .filter(|row| row.strike.is_some())
                .map(|row| row.line)
                .collect();
            assert_eq!(struck, [0, 5], "{kind:?}");
            let row = &visual_rows(editor.buffer())[0];
            let (start, end) = row.strike.unwrap();
            // "- [x] " is six monospace cells; "done" is four more.
            let cell = start / 6.0;
            assert!(cell > 7.0 && cell < 10.0, "{start}");
            assert!((end - start - 4.0 * cell).abs() < 0.5, "{start}..{end}");
        }
    }
}
