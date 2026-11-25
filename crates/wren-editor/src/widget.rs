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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::{DerefMut, Range};
use std::sync::Arc;

use iced::advanced::graphics::text::cosmic_text::{self, Scroll};
use iced::advanced::layout::{self, Layout};
use iced::advanced::mouse;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::svg::{self, Renderer as _, Svg};
use iced::advanced::text::highlighter::Highlighter as _;
use iced::advanced::text::{self, LineHeight, Paragraph as _, Renderer as _, Text, Wrapping};
use iced::advanced::widget::{self, operation, tree, Widget};
use iced::advanced::{clipboard, input_method, overlay, Clipboard, InputMethod, Shell};
use iced::keyboard;
use iced::time::{Duration, Instant};
use iced::widget::text_editor::{
    Action, Binding, Cursor, Edit, KeyPress, LineEnding, Position, Selection, Status,
};
use iced::{
    alignment, window, Border, Color, Element, Event, Font, Length, Padding, Pixels, Point,
    Rectangle, Renderer, Size, Theme, Vector,
};

use crate::backend::{Backend, BackendKind, Lines, Placement};
use crate::highlighter::{MarkdownHighlighter, Settings as HighlighterSettings};
use crate::hit::{self, Hit};
use crate::keys::{KeyPolicy, Routed, Target};
use crate::popover::{
    self, CaretLine, MenuOverlay, MenuPress, MenuSource, Now, Opened, Popover, ResolvedPress,
    Sources, SuggestSource,
};
use crate::proof::{self, Proofreader, SpellEvent, SpellMenu};
#[cfg(any(test, feature = "test-support"))]
use crate::proof::{DrawnMark, SpellFrame};
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
const BARE_TEXT_INSET: f32 = 16.0;
const VERTICAL_PADDING: f32 = 24.0;
/// The least space right of left-placed text.
const RIGHT_INSET: f32 = 24.0;
/// The least space either side of centred text: the larger of a fixed
/// width and a share of the widget's.
const CENTRED_MARGIN: f32 = 48.0;
const CENTRED_MARGIN_SHARE: f32 = 0.06;
const CARET_WIDTH: f32 = 2.0;
const CARET_HEIGHT: f32 = 18.0;
/// JetBrains Mono OS/2 `sxHeight` (550) over its 1000 units per em; other
/// monospace fonts sit close enough for the strike-through.
const X_HEIGHT_EM: f32 = 0.55;
/// An inline code chip's height, in ems: the glyph box plus about 2 px a
/// side at the default size.
const CHIP_HEIGHT_EM: f32 = 1.3;
const CHIP_PADDING_X: f32 = 2.0;
const CHIP_RADIUS: f32 = 3.0;
const STRIKE_THICKNESS: f32 = 1.0;
/// How far below the zigzag's top each square of its 4 px period sits.
const ZIGZAG_STEPS: [f32; 4] = [2.0, 1.0, 0.0, 1.0];
const ZIGZAG_HEIGHT: f32 = 3.0;
/// How far the zigzag's top sits below the baseline.
const ZIGZAG_DROP: f32 = 2.0;

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

    /// See [`Backend::reveal`].
    pub fn reveal(&mut self, position: Position) {
        self.0.get_mut().reveal(position);
    }

    /// Where a popover anchored to `range` of `line` hangs; see
    /// [`anchor_row`].
    pub fn anchor_row(&self, line: usize, range: Range<usize>) -> Option<AnchorRow> {
        anchor_row(&self.0.borrow(), line, range)
    }

    /// Where `point`, relative to the text origin, falls among the marks
    /// `marks` gives for a line and its text; see [`place_under`].
    pub fn place_under(
        &self,
        point: Point,
        marks: impl FnOnce(usize, &str) -> Vec<Range<usize>>,
    ) -> Option<Place> {
        place_under(&self.0.borrow(), point, marks)
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
            tasks: None,
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
    }

    /// Lays the text out in `size` as [`Content::lay_out`] does, with a
    /// highlighter given `front_matter_end`, and returns the visible rows
    /// with the marks `proofreader` finds.
    #[cfg(test)]
    fn rows(
        &mut self,
        size: Size,
        front_matter_end: Option<usize>,
        proofreader: Option<&dyn Proofreader>,
        caret: Option<Position>,
    ) -> Vec<VisualRow> {
        crate::fixtures::load_font();
        let mut highlighter = MarkdownHighlighter::new(&HighlighterSettings {
            palette: crate::fixtures::DARK,
            font: crate::fixtures::FONT,
            front_matter_end,
            tasks: None,
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
        visual_rows(backend, &highlighter, proofreader, caret, None)
    }

    /// The marks a frame of `size` draws, as [`Content::rows`] lays it out,
    /// with the text origin at the window's.
    #[cfg(test)]
    pub fn drawn_marks(
        &mut self,
        size: Size,
        front_matter_end: Option<usize>,
        proofreader: Option<&dyn Proofreader>,
        caret: Option<Position>,
    ) -> Vec<DrawnMark> {
        let rows = self.rows(size, front_matter_end, proofreader, caret);
        drawn_marks(&rows, Point::ORIGIN, Rectangle::new(Point::ORIGIN, size))
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

/// How wide the text runs and where it sits in the widget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Measure {
    /// Characters before a line wraps, in widths of the font's `0`; `None`
    /// wraps at the widget's edge.
    pub wrap_column: Option<u16>,
    /// Centre the text, with the gutter in its left margin, rather than
    /// start it at the widget's left.
    pub centred: bool,
}

/// The horizontal layout of a widget's column, in pixels from its left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Columns {
    /// The span the current-line and code bands, and the gutter, sit in.
    band: (f32, f32),
    /// The text's left edge and width.
    text: (f32, f32),
}

/// A multi-line Markdown editor with a line-number gutter.
pub struct TextEditor<'a> {
    id: widget::Id,
    content: &'a Content,
    highlighter_settings: HighlighterSettings,
    style: Style,
    line_numbers: bool,
    metrics: Metrics,
    measure: Measure,
    markdown_assist: bool,
    key_policy: Option<&'a KeyPolicy>,
    proofreader: Option<&'a dyn Proofreader>,
    /// The open popover: the suggestions menu's, or the host's.
    popover: Option<&'a Popover>,
    /// The host's sources of menus and suggestions, asked for what a
    /// message of this batch opens.
    sources: Sources<'a>,
    /// The open popover as the events this widget has handled leave it; see
    /// [`popover::OpenMenu`]. The host builds a new widget once it has
    /// applied them.
    menu_now: Now,
    /// Where the host learns the text's origin in the window.
    origin: Option<&'a Cell<Option<Point>>>,
    /// The host's verdict on the presses it resolved.
    resolved_press: ResolvedPress,
    /// A left press of this batch was left to the host, so the widget cannot
    /// tell what the left presses after it make until the next batch.
    left_to_host: Cell<bool>,
    /// Where the last frame's marks are recorded for the host's tests.
    #[cfg(any(test, feature = "test-support"))]
    spell_frame: Option<&'a RefCell<SpellFrame>>,
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
            measure: Measure::default(),
            markdown_assist,
            key_policy: None,
            proofreader: None,
            popover: None,
            sources: Sources::default(),
            menu_now: Now::new(),
            origin: None,
            resolved_press: ResolvedPress::default(),
            left_to_host: Cell::new(false),
            #[cfg(any(test, feature = "test-support"))]
            spell_frame: None,
            last_status: None,
        }
    }

    /// Set how wide the text runs and where it sits; by default it fills
    /// the widget from the left.
    pub fn measure(mut self, measure: Measure) -> Self {
        self.measure = measure;
        self
    }

    /// Ask `policy` about each key press first (see [`KeyPolicy`]).
    pub fn key_policy(mut self, policy: Option<&'a KeyPolicy>) -> Self {
        self.key_policy = policy;
        self
    }

    /// Mark the misspellings `proofreader` finds on the drawn rows. The
    /// iced engine never asks it.
    pub fn proofreader(mut self, proofreader: Option<&'a dyn Proofreader>) -> Self {
        self.proofreader = proofreader;
        self
    }

    /// Show `menu`, the open suggestions menu, and route keys and presses to
    /// it.
    pub fn spell_menu(self, menu: Option<&'a SpellMenu>) -> Self {
        self.popover(menu.map(|menu| &**menu))
    }

    /// Show `popover`, the open popover, and route keys and presses to it
    /// as its mode has them.
    pub fn popover(mut self, popover: Option<&'a Popover>) -> Self {
        self.popover = popover;
        self.menu_now.start(popover);
        self
    }

    /// The host's source of menus, asked what a request for one opens.
    pub fn menu_source(mut self, source: Option<&'a dyn MenuSource>) -> Self {
        self.sources.menus = source;
        self
    }

    /// The host's source of suggestions, asked what a character typed
    /// opens, unless it is typed on `suppressed`, the line the host hides
    /// suggestions for.
    pub fn suggest_source(
        mut self,
        source: Option<&'a dyn SuggestSource>,
        suppressed: Option<usize>,
    ) -> Self {
        self.sources.suggestions = source;
        self.menu_now.suppress(suppressed);
        self
    }

    /// Record in `origin` where the text starts in the window.
    pub fn origin(mut self, origin: &'a Cell<Option<Point>>) -> Self {
        self.origin = Some(origin);
        self
    }

    /// What the host made of the presses it resolved, which the next left
    /// press counts from.
    pub(crate) fn resolved_press(mut self, resolved: ResolvedPress) -> Self {
        self.resolved_press = resolved;
        self
    }

    /// Record each frame's marks in `frame`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn spell_frame(mut self, frame: &'a RefCell<SpellFrame>) -> Self {
        self.spell_frame = Some(frame);
        self
    }

    /// The space left of the text: the gutter and its inset, or a bare
    /// inset without line numbers.
    fn lead(&self) -> f32 {
        ROW_INSET
            + if self.line_numbers {
                self.metrics.gutter
            } else {
                BARE_TEXT_INSET
            }
    }

    /// The column in a widget `width` wide, for a font whose `0` is
    /// `advance` wide. Left-placed text starts after the gutter and wraps at
    /// the wrap column or `RIGHT_INSET` from the edge, and the bands span
    /// the widget. Centred text keeps equal margins of at least the gutter,
    /// and the bands span the gutter and the text. Edges are whole pixels,
    /// so the text stays on the pixel grid.
    fn columns(&self, width: f32, advance: f32) -> Columns {
        let lead = self.lead();
        // Rounded, then a pixel of slack: a run of `n` glyphs can sum a hair
        // past `n` advances and must not wrap its last one, and rounding
        // keeps float noise in the advance from moving the edge.
        let wrap = self
            .measure
            .wrap_column
            .map(|column| (f32::from(column) * advance).round() + 1.0);
        if self.measure.centred {
            let margin = lead
                .max(CENTRED_MARGIN)
                .max((width * CENTRED_MARGIN_SHARE).floor());
            let available = (width - 2.0 * margin).max(0.0).floor();
            let text_width = wrap.map_or(available, |wrap| wrap.min(available));
            // Never left of the gutter, even in a widget too narrow for
            // two margins.
            let text_left = ((width - text_width) / 2.0).floor().max(lead);
            Columns {
                band: (text_left - lead, text_left + text_width + ROW_INSET),
                text: (text_left, text_width),
            }
        } else {
            let available = (width - lead - RIGHT_INSET).max(0.0).floor();
            Columns {
                band: (0.0, width),
                text: (lead, wrap.map_or(available, |wrap| wrap.min(available))),
            }
        }
    }

    /// The bands' horizontal extent in `bounds`.
    fn band(&self, bounds: Rectangle, advance: f32) -> (f32, f32) {
        let (left, right) = self.columns(bounds.width, advance).band;
        (bounds.x + left, right - left)
    }

    /// The space around the text in a widget `width` wide.
    fn padding(&self, width: f32, advance: f32) -> Padding {
        let (left, text_width) = self.columns(width, advance).text;
        Padding {
            top: VERTICAL_PADDING,
            right: (width - left - text_width).max(0.0),
            bottom: VERTICAL_PADDING,
            left,
        }
    }

    /// Lay the text out for a widget of `size`; [`Widget::layout`] without
    /// the renderer, which it never needs.
    fn lay_out(&self, state: &mut State, size: Size) {
        let mut editor = self.content.0.borrow_mut();

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
        let (font, text_size) = (self.highlighter_settings.font, self.metrics.text);
        if (state.advance.font, state.advance.size) != (font, text_size) {
            state.advance = Advance::measure(font, text_size);
        }

        editor.update(
            layout::Limits::new(Size::ZERO, size)
                .shrink(self.padding(size.width, state.advance.width))
                .max(),
            self.highlighter_settings.font,
            Pixels(self.metrics.text),
            self.metrics.line_height(),
            Wrapping::WordOrGlyph,
            state.highlighter.borrow_mut().deref_mut(),
        );
        drop(editor);

        // What the open popover is first laid out with, so that only a
        // change made while it is open closes it (rule 4.4.8).
        if self.popover.is_none() {
            state.menu_look = None;
        } else if state.menu_look.is_none() {
            state.menu_look = Some((size, self.look()));
        }
    }

    /// Handle `event` for a widget at `bounds`; [`Widget::update`] without
    /// the renderer, which it never needs.
    fn on_event(
        &mut self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let is_redraw = matches!(event, Event::Window(window::Event::RedrawRequested(_now)));
        if state.resolved_seen != self.resolved_press.count {
            state.resolved_seen = self.resolved_press.count;
            state.click_kind = self.resolved_press.kind;
        }
        // A size or look changed since the open menu was laid out closes it
        // before the event is handled, which then acts as if it had not
        // been open.
        if state
            .menu_look
            .take_if(|(size, look)| *size != bounds.size() || *look != self.look())
            .is_some()
            && self.menu_now.is_open()
        {
            self.publish(shell, Message::Spell(SpellEvent::Close));
        }
        let mut accelerators = Vec::new();
        let target = self.target(&mut accelerators);
        // Whether the open popover closes (rule 4.4.8).
        let mut closing = false;

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => {
                // The host closes the suggestions menu on Escape, so it is
                // not told. Any other popover is sent the key in its order.
                if target.is_some_and(Target::leaves_escape) {
                    self.menu_now.closed();
                }
            }
            Event::Window(window::Event::Unfocused) => {
                if let Some(focus) = &mut state.focus {
                    focus.is_window_focused = false;
                }
                closing = true;
            }
            Event::Mouse(mouse::Event::ButtonPressed(_)) if !cursor.is_over(bounds) => {
                // The press goes on to do what it does today.
                closing = true;
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
            bounds,
            self.padding(bounds.width, state.advance.width),
            cursor,
            crate::keys::Context {
                markdown_assist: self.markdown_assist,
                composing: state.is_composing(),
                policy: self.key_policy,
            },
            target,
        ) {
            let update = match update {
                Update::Click(click) if self.left_to_host.get() => {
                    Update::Press(Pressed::Left(click))
                }
                update => update,
            };
            match update {
                Update::Click(click) => {
                    let kind = popover::click_kind(click.kind(), state.click_kind);
                    let action = match kind {
                        mouse::click::Kind::Single => Action::Click(click.position()),
                        mouse::click::Kind::Double => Action::SelectWord,
                        mouse::click::Kind::Triple => Action::SelectLine,
                    };
                    state.focus = Some(Focus::now());
                    state.last_click = Some(click);
                    state.click_kind = Some(kind);
                    state.drag_click = Some(kind);
                    self.publish(shell, Message::Action(action));
                    shell.capture_event();
                }
                Update::RightClick(position) => {
                    // Only a misspelled word answers; anywhere else the
                    // press leaves focus and the caret alone.
                    if self.misspelling_under(state, position).is_some() {
                        state.focus = Some(Focus::now());
                        state.drag_click = None;
                        // So a slight drift of the fingers during the click
                        // cannot add up with an earlier scroll's remainder
                        // to a whole line, which would close the menu.
                        state.partial_scroll = 0.0;
                        self.publish(shell, Message::Spell(SpellEvent::OpenAt(position)));
                        shell.capture_event();
                    }
                }
                Update::Dismiss(button) => {
                    // A press in the editor closes the menu and does
                    // nothing more, so the caret stays (rule 4.4.5), and
                    // the press after it is no double click.
                    if button == mouse::Button::Left {
                        state.click_kind = None;
                    }
                    self.publish(shell, Message::Spell(SpellEvent::Close));
                    shell.capture_event();
                }
                Update::Press(press) => {
                    // Only the host knows whether a menu opened. The press
                    // is taken as it would be with none, and the host only
                    // closes the menu if one did.
                    let event = match press {
                        Pressed::Left(click) => {
                            let kind = popover::click_kind(click.kind(), state.click_kind);
                            state.focus = Some(Focus::now());
                            state.last_click = Some(click);
                            // Until the host's verdict arrives with the next
                            // batch, the left presses after this one are its
                            // to resolve too.
                            state.click_kind = Some(kind);
                            self.left_to_host.set(true);
                            // The host drops the drag unless it takes the
                            // press as a single click.
                            state.drag_click = Some(mouse::click::Kind::Single);
                            MenuPress::Left(click.position(), kind)
                        }
                        Pressed::Right(position) => {
                            state.drag_click = None;
                            state.partial_scroll = 0.0;
                            MenuPress::Right(position)
                        }
                    };
                    self.publish(shell, Message::Spell(SpellEvent::Press(event)));
                    shell.capture_event();
                }
                Update::Drag(position) => {
                    self.publish(shell, Message::Action(Action::Drag(position)));
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
                    // A trackpad sends wheel events that move no whole line
                    // as fingers touch and lift, which a two-finger click
                    // does; as an action they would close the menu that
                    // click opened (rule 4.4.8).
                    let lines = lines as i32;
                    if lines != 0 {
                        self.publish(shell, Message::Action(Action::Scroll { lines }));
                    }
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
                            closing = true;
                        }
                        shell.request_redraw();
                    }
                    Ime::Commit(text) => {
                        self.publish(
                            shell,
                            Message::Action(Action::Edit(Edit::Paste(Arc::new(text)))),
                        );
                    }
                },
                Update::Key(routed) => {
                    let binding = match routed {
                        Routed::Binding(binding) => Some(binding),
                        Routed::Close(binding) => {
                            // Before the key's own message, which then acts
                            // as if the menu had not been open.
                            self.publish(shell, Message::Spell(SpellEvent::Close));
                            binding
                        }
                        Routed::Pass(message) => {
                            // Uncaptured, so the host's own key handling
                            // hears of the press too.
                            self.publish(shell, message);
                            None
                        }
                    };
                    if let Some(binding) = binding {
                        if !matches!(binding, Binding::Unfocus) {
                            shell.capture_event();
                        }
                        apply_binding(binding, self, state, clipboard, shell);
                        if let Some(focus) = &mut state.focus {
                            focus.updated_at = Instant::now();
                        }
                    }
                }
            }
        }

        let focused = state.focus.is_some();
        closing |= state.was_focused && !focused;
        state.was_focused = focused;
        if closing && self.menu_now.is_open() {
            self.publish(shell, Message::Spell(SpellEvent::Close));
        } else if closing {
            // Nor does a host that took the keyboard keep it past a
            // press outside, or the editor's or the window's blur.
            self.menu_now.closed();
        }
        // A menu opened later in this batch is laid out afresh.
        if self.menu_now.get().is_none() {
            state.menu_look = None;
        }

        let status = {
            let is_hovered = cursor.is_over(bounds);
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
            shell.request_input_method(&self.input_method(state, bounds));
        } else if self
            .last_status
            .is_some_and(|last_status| status != last_status)
        {
            shell.request_redraw();
        }
    }

    /// What the widget is drawn with that, changed, closes an open menu.
    fn look(&self) -> Look {
        Look {
            style: self.style,
            palette: self.highlighter_settings.palette,
            font: self.highlighter_settings.font,
            metrics: self.metrics,
            line_numbers: self.line_numbers,
            measure: self.measure,
        }
    }

    /// Publish `message`, keeping what is open as the host will leave it
    /// once it applies the message.
    fn publish(&self, shell: &mut Shell<'_, Message>, message: Message) {
        self.menu_now
            .publish(shell, message, &|| self.caret_line(), self.sources);
    }

    /// The caret's line as the host has it.
    fn caret_line(&self) -> Option<CaretLine> {
        let cursor = self.content.cursor();
        let caret = cursor.position;
        self.content.with_lines(|lines| {
            Some(CaretLine {
                index: caret.line,
                text: lines.get(caret.line)?.text.into_owned(),
                caret: caret.column,
                selected: cursor.selection.is_some_and(|anchor| anchor != caret),
            })
        })
    }

    /// What is open, as key routing needs it, with the rows' accelerators
    /// in `accelerators`.
    fn target<'t>(
        &self,
        accelerators: &'t mut Vec<Option<popover::Accelerator>>,
    ) -> Option<Target<'t>> {
        let (opened, open) = self.menu_now.live()?;
        let (spelling, rows) = self.menu_now.rows(self.popover);
        *accelerators = rows;
        Some(match opened {
            Opened::Menu if open.opening => Target::SpellOpening,
            Opened::Menu if spelling => Target::SpellMenu,
            Opened::Menu => Target::Menu(accelerators),
            Opened::Suggest => Target::Suggest,
            Opened::Keyboard => Target::Keyboard,
        })
    }

    /// Whether routing `event` ends the open popover or changes its rows,
    /// for the overlay, which sees a batch before the widget routes it
    /// ([`popover::InOrder`]). `focused` and `composing` are the widget's
    /// state when the batch starts; composing follows the batch's preedits
    /// as the widget's will.
    fn ending<'b>(&'b self, focused: bool, mut composing: bool) -> impl FnMut(&Event) -> bool + 'b {
        move |event| match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                modifiers,
                text,
                ..
            }) => {
                let mut accelerators = Vec::new();
                let Some(target) = self.target(&mut accelerators) else {
                    return true;
                };
                let escape = matches!(
                    key.as_ref(),
                    keyboard::Key::Named(keyboard::key::Named::Escape)
                );
                let routed = crate::keys::route(
                    KeyPress {
                        key: key.clone(),
                        modified_key: modified_key.clone(),
                        physical_key: *physical_key,
                        modifiers: *modifiers,
                        text: text.clone(),
                        status: if focused {
                            Status::Focused { is_hovered: false }
                        } else {
                            Status::Active
                        },
                    },
                    crate::keys::Context {
                        markdown_assist: self.markdown_assist,
                        composing,
                        policy: self.key_policy,
                    },
                    Some(target),
                );
                match routed {
                    // The host closes the suggestions menu for Escape.
                    None => focused && escape && target.leaves_escape() && !composing,
                    Some(Routed::Binding(Binding::Custom(Message::Spell(
                        SpellEvent::Move(_)
                        | SpellEvent::Key(popover::MenuKey::Up | popover::MenuKey::Down),
                    )))) => false,
                    Some(_) => true,
                }
            }
            Event::InputMethod(input_method::Event::Preedit(content, _)) if focused => {
                let starts = !composing && !content.is_empty();
                composing = !content.is_empty();
                starts
            }
            Event::InputMethod(input_method::Event::Commit(_)) => focused,
            Event::InputMethod(input_method::Event::Opened | input_method::Event::Closed) => {
                composing = false;
                false
            }
            Event::Window(window::Event::Unfocused) => true,
            _ => false,
        }
    }

    /// The open popover as an overlay of a widget at `bounds`: placed under
    /// its anchor's first row as the last layout left it (rule 4.4.3), or
    /// `None` when that row is out of view. Tells the host where the text
    /// starts in the window, which is where `bounds` puts it.
    fn menu_overlay<'b>(
        &'b self,
        state: &'b mut State,
        bounds: Rectangle,
    ) -> Option<MenuOverlay<'b>> {
        let text_bounds = bounds.shrink(self.padding(bounds.width, state.advance.width));
        if let Some(origin) = self.origin {
            origin.set(Some(text_bounds.position()));
        }
        // A popover opened in this batch is the host's to draw, once it has
        // opened it.
        let menu =
            (self.popover).filter(|_| self.menu_now.get().is_some() && !self.menu_now.is_own())?;
        let row = anchor_row(&self.content.0.borrow(), menu.line(), menu.range())?;
        let style = self.style.menu;
        let (text_size, width) = (style.text_size.to_bits(), bounds.width.to_bits());
        // Compared against the rows as they are: a popover's labels are
        // copied only when they are fitted afresh.
        let fitted = state.menu_fit.as_ref().is_some_and(|(key, _)| {
            key.items == menu.labels()
                && (key.font, key.text_size, key.width) == (style.font, text_size, width)
        });
        if !fitted {
            let fit = popover::fit(menu.labels(), bounds.width, |text| {
                popover::measure(text, style.font, style.text_size)
            });
            let key = FitKey {
                items: menu.labels().to_vec(),
                font: style.font,
                text_size,
                width,
            };
            state.menu_fit = Some((key, fit));
        }
        let state = &*state;
        let (_, fit) = state.menu_fit.as_ref()?;
        let size = Size::new(fit.width, popover::height(fit.labels.len()));
        Some(MenuOverlay {
            menu,
            labels: &fit.labels,
            bounds: popover::place(
                bounds,
                text_bounds.x + row.left,
                text_bounds.y + row.top,
                text_bounds.y + row.bottom,
                size,
            ),
            style,
            now: self.menu_now.cell(),
            #[cfg(any(test, feature = "test-support"))]
            frame: self.spell_frame,
        })
    }

    /// The host's proofreader, on the engine that checks spelling (§3.3).
    fn speller(&self) -> Option<&'a dyn Proofreader> {
        self.proofreader
            .filter(|_| self.content.kind() == BackendKind::Owned)
    }

    /// The misspelled word under `point`, relative to the text origin; see
    /// [`Self::hit_under`]. With no speller there is none to look for.
    fn misspelling_under(&self, state: &State, point: Point) -> Option<(usize, Range<usize>)> {
        self.speller()?;
        let (line, hit) = self.hit_under(state, point)?;
        Some((line, hit.misspelling()?))
    }

    /// What lies under `point`, relative to the text origin, and its line:
    /// a misspelled word before a span ([`crate::hit`]), as the last layout
    /// placed them and the widget's highlighter read the line. Only the
    /// owned engine has misspellings (§3.3); both have spans.
    fn hit_under(&self, state: &State, point: Point) -> Option<(usize, Hit)> {
        let highlighter = state.highlighter.borrow();
        let place = self.content.place_under(point, |line, text| {
            (self.speller().zip(highlighter.decor(line)))
                .map(|(proofreader, decor)| proof::line_marks(proofreader, text, &decor.prose))
                .unwrap_or_default()
        })?;
        let span = || highlighter.spans(place.line)?.at(place.column?).cloned();
        Some((place.line, hit::resolve(place.mark, span)?))
    }

    fn input_method<'b>(&self, state: &'b State, bounds: Rectangle) -> InputMethod<&'b str> {
        let Some(Focus {
            is_window_focused: true,
            ..
        }) = &state.focus
        else {
            return InputMethod::Disabled;
        };

        let text_bounds = bounds.shrink(self.padding(bounds.width, state.advance.width));
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
    /// The last left press inside the editor, as iced counts clicks.
    last_click: Option<mouse::Click>,
    /// What the last left press was taken as, `None` when it only closed
    /// the menu.
    click_kind: Option<mouse::click::Kind>,
    /// The host verdicts already taken into `click_kind`.
    resolved_seen: u64,
    drag_click: Option<mouse::click::Kind>,
    partial_scroll: f32,
    highlighter: RefCell<MarkdownHighlighter>,
    highlighter_settings: HighlighterSettings,
    advance: Advance,
    /// The open menu's width and rows, and what they were fitted to.
    menu_fit: Option<(FitKey, popover::Fit)>,
    /// Whether the editor had focus after the last event, so losing it
    /// closes the menu.
    was_focused: bool,
    /// The size and look the open menu was first laid out with, so a
    /// change to them while it is open closes it.
    menu_look: Option<(Size, Look)>,
    zigzags: RefCell<Zigzags>,
}

/// What a menu's width and rows were fitted to.
#[derive(Debug, Clone, PartialEq)]
struct FitKey {
    items: Vec<String>,
    font: Font,
    text_size: u32,
    width: u32,
}

/// What the widget is drawn with: colours, font, text size, gutter and
/// measure, all set by the host.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    style: Style,
    palette: crate::Palette,
    font: Font,
    metrics: Metrics,
    line_numbers: bool,
    measure: Measure,
}

/// The width of the font's `0` at a text size: the unit of the wrap column.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Advance {
    font: Font,
    size: f32,
    width: f32,
}

impl Advance {
    /// Shape a `0` in `font` at `size`.
    fn measure(font: Font, size: f32) -> Self {
        let paragraph = <Renderer as text::Renderer>::Paragraph::with_text(Text {
            content: "0",
            bounds: Size::INFINITE,
            size: Pixels(size),
            line_height: LineHeight::default(),
            font,
            align_x: text::Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: Wrapping::None,
        });
        Self {
            font,
            size,
            width: paragraph.min_width(),
        }
    }
}

impl State {
    fn new(highlighter_settings: HighlighterSettings) -> Self {
        Self {
            focus: None,
            preedit: None,
            last_click: None,
            click_kind: None,
            resolved_seen: 0,
            drag_click: None,
            partial_scroll: 0.0,
            highlighter: RefCell::new(MarkdownHighlighter::new(&highlighter_settings)),
            // Layout measures again when the text size differs.
            advance: Advance::measure(highlighter_settings.font, TEXT_SIZE),
            highlighter_settings,
            menu_fit: None,
            was_focused: false,
            menu_look: None,
            zigzags: RefCell::new(Zigzags::default()),
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
        let limits = limits.width(Length::Fill).height(Length::Fill);
        self.lay_out(tree.state.downcast_mut::<State>(), limits.max());
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
        self.on_event(state, event, layout.bounds(), cursor, clipboard, shell);
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

        let text_bounds = bounds.shrink(self.padding(bounds.width, state.advance.width));
        let (column_x, column_width) = self.band(bounds, state.advance.width);
        // Rows span the full width, but scroll out under the vertical padding.
        let rows_clip = Rectangle {
            x: bounds.x,
            y: text_bounds.y,
            width: bounds.width,
            height: text_bounds.height,
        };
        let cursor = editor.cursor();
        let cursor_line = cursor.position.line;
        let selection = state
            .focus
            .as_ref()
            .and_then(|focus| Some((focus, editor.selection()?)));
        let rows = visual_rows(
            &editor,
            &state.highlighter.borrow(),
            self.proofreader,
            hiding_caret(state.focus.is_some(), cursor),
            self.popover,
        );

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

        if let Some((_, Selection::Range(ranges))) = &selection {
            let breaks = selected_breaks(&rows, cursor, state.advance.width);
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

        for tint in rows.iter().filter_map(|row| {
            let (start, end) = row.tint?;
            let tint = Rectangle {
                x: text_bounds.x + start,
                y: text_bounds.y + row.top,
                width: end - start,
                height: row.height,
            };
            tint.intersection(&rows_clip)
        }) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: tint,
                    ..renderer::Quad::default()
                },
                style.text_selection,
            );
        }

        // Over the selection, which must not hide a mark, and under the
        // glyphs, which paint over it.
        let origin = text_bounds.position();
        let zigzags = rows.iter().flat_map(|row| {
            row.marks
                .iter()
                .filter(|mark| mark.shown)
                .filter_map(move |mark| mark.zigzag(row, origin))
        });
        for (bounds, handle) in
            state
                .zigzags
                .borrow_mut()
                .frame(zigzags, rows_clip, style.misspelling)
        {
            renderer.draw_svg(Svg::new(handle), bounds, rows_clip);
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(frame) = self.spell_frame {
            // The menu's overlay draws after the widget and records itself.
            *frame.borrow_mut() = SpellFrame {
                marks: drawn_marks(&rows, origin, rows_clip),
                menu: None,
            };
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

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        _renderer: &Renderer,
        _viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let this: &'b Self = self;
        let state = tree.state.downcast_mut::<State>();
        let ends = this.ending(state.focus.is_some(), state.is_composing());
        let menu = this.menu_overlay(state, layout.bounds() + translation)?;
        Some(overlay::Element::new(Box::new(popover::InOrder::new(
            menu, ends,
        ))))
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
    editor: &TextEditor<'_>,
    state: &mut State,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, Message>,
) {
    let content = editor.content;
    let mut act = |action| editor.publish(shell, Message::Action(action));

    match binding {
        Binding::Unfocus => {
            operation::Focusable::unfocus(state);
            state.drag_click = None;
        }
        Binding::Copy | Binding::Cut => {
            let line = match content
                .selection()
                .filter(|selection| !selection.is_empty())
            {
                Some(selection) => {
                    clipboard.write(clipboard::Kind::Standard, selection);
                    None
                }
                // Nothing selected: the caret's whole line, as VS Code does.
                None => {
                    let line = content.cursor().position.line;
                    let text = content.with_lines(|lines| crate::edit::line_clip(lines, line));
                    clipboard.write(clipboard::Kind::Standard, text.clone());
                    Some(Arc::new(text))
                }
            };
            let mut messages = Vec::new();
            crate::keys::clip_messages(matches!(binding, Binding::Cut), line, &mut messages);
            for message in messages {
                editor.publish(shell, message);
            }
        }
        Binding::Paste => {
            if let Some(contents) = clipboard.read(clipboard::Kind::Standard) {
                editor.publish(shell, Message::Clip(crate::Clip::Paste(Arc::new(contents))));
            }
        }
        Binding::Move(motion) => act(Action::Move(motion)),
        Binding::Select(motion) => act(Action::Select(motion)),
        Binding::SelectWord => act(Action::SelectWord),
        Binding::SelectLine => act(Action::SelectLine),
        Binding::SelectAll => act(Action::SelectAll),
        Binding::Insert(c) => {
            // A key pressed while an input method composes asks the source
            // of suggestions nothing, which the host cannot tell from the
            // edit alone.
            if editor.sources.suggestions.is_some() && state.is_composing() {
                editor.publish(shell, Message::Spell(SpellEvent::Hold));
            }
            editor.publish(shell, Message::Action(Action::Edit(Edit::Insert(c))));
        }
        Binding::Enter => act(Action::Edit(Edit::Enter)),
        Binding::Backspace => act(Action::Edit(Edit::Backspace)),
        Binding::Delete => act(Action::Edit(Edit::Delete)),
        Binding::Sequence(sequence) => {
            for binding in sequence {
                apply_binding(binding, editor, state, clipboard, shell);
            }
        }
        Binding::Custom(message) => {
            // As a right-click that opens the menu does.
            if matches!(
                message,
                Message::Spell(SpellEvent::OpenAtCaret | SpellEvent::Request(_))
            ) {
                state.partial_scroll = 0.0;
            }
            editor.publish(shell, message);
        }
    }
}

enum Update {
    Click(mouse::Click),
    /// A right-click, relative to the text origin.
    RightClick(Point),
    /// A press of this button in the editor while the menu is open.
    Dismiss(mouse::Button),
    /// A left or right press in the editor after an opening earlier in the
    /// batch, which may not have opened a menu.
    Press(Pressed),
    Drag(Point),
    Release,
    Scroll(f32),
    InputMethod(Ime),
    Key(Routed),
}

enum Pressed {
    Left(mouse::Click),
    /// Relative to the text origin.
    Right(Point),
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
        popover: Option<Target<'_>>,
    ) -> Option<Self> {
        // Only a menu takes the editor's presses, or what may be one;
        // suggestions leave them to the text.
        let menu = popover.and_then(Target::modal);
        match event {
            Event::Mouse(event) => match event {
                mouse::Event::ButtonPressed(button) if menu.is_some() && cursor.is_over(bounds) => {
                    let position = cursor
                        .position_in(bounds)
                        .map(|position| position - Vector::new(padding.left, padding.top));
                    Some(match (menu == Some(true), button, position) {
                        (true, mouse::Button::Left, Some(position)) => {
                            Update::Press(Pressed::Left(mouse::Click::new(
                                position,
                                mouse::Button::Left,
                                state.last_click,
                            )))
                        }
                        (true, mouse::Button::Right, Some(position)) => {
                            Update::Press(Pressed::Right(position))
                        }
                        _ => Update::Dismiss(*button),
                    })
                }
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
                        Some(Update::Key(Routed::Binding(Binding::Unfocus)))
                    } else {
                        None
                    }
                }
                mouse::Event::ButtonPressed(mouse::Button::Right) => {
                    let position = cursor.position_in(bounds)?;
                    Some(Update::RightClick(
                        position - Vector::new(padding.left, padding.top),
                    ))
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
                crate::keys::route(
                    KeyPress {
                        key: key.clone(),
                        modified_key: modified_key.clone(),
                        physical_key: *physical_key,
                        modifiers: *modifiers,
                        text: text.clone(),
                        status,
                    },
                    keys,
                    popover,
                )
                .map(Update::Key)
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
    /// The misspelled words with glyphs on the row.
    marks: Vec<RowMark>,
    /// Horizontal `(start, end)` of the open menu's word on the row, which
    /// is tinted (rule 4.4.2).
    tint: Option<(f32, f32)>,
}

/// A misspelled word's glyphs on one visual row.
#[derive(Debug, Clone, PartialEq)]
struct RowMark {
    /// The word's byte range in its line.
    range: Range<usize>,
    /// Horizontal extent of the word's glyphs on the row.
    left: f32,
    right: f32,
    /// False while the caret hides the mark (rule 4.3.3).
    shown: bool,
}

/// A misspelled word's byte range in its line, and whether it is shown.
type LineMark = (Range<usize>, bool);

/// The caret that hides a mark ending at it (rule 4.3.3): the cursor's
/// position while the editor has focus and nothing is selected. It is read
/// from the cursor, not from the caret the widget draws, which is missing
/// when the caret's row is out of view though the rest of a wrapped word
/// is not.
fn hiding_caret(focused: bool, cursor: Cursor) -> Option<Position> {
    (focused && cursor.selection.is_none()).then_some(cursor.position)
}

/// Collects the visible rows of `editor`'s buffer (already scrolled by the
/// editor), with the decoration `highlighter` left for each line and the
/// misspellings `proofreader` finds in its prose, the mark ending at `caret`
/// hidden, and the anchor of the open popover, `menu`, tinted if it tints
/// it. Only the owned engine asks the proofreader (§3.3).
fn visual_rows(
    editor: &Backend,
    highlighter: &MarkdownHighlighter,
    proofreader: Option<&dyn Proofreader>,
    caret: Option<Position>,
    menu: Option<&Popover>,
) -> Vec<VisualRow> {
    let buffer = editor.buffer();
    let tint = menu.and_then(Popover::tint);
    let body = buffer.metrics();
    let proofreader = proofreader.filter(|_| editor.kind() == BackendKind::Owned);
    // The marks of the line the last run belonged to, asked once per line
    // however many rows it wraps to.
    let mut line_marks: Option<(usize, Vec<LineMark>)> = None;
    buffer
        .layout_runs()
        .map(|run| {
            let decor = highlighter.decor(run.line_i);
            let glyphs = || {
                run.glyphs
                    .iter()
                    .map(|glyph| (glyph.start..glyph.end, glyph.x, glyph.w))
            };
            let marks = match (proofreader, decor) {
                (Some(proofreader), Some(decor)) => {
                    if line_marks
                        .as_ref()
                        .is_none_or(|(line, _)| *line != run.line_i)
                    {
                        let marks = proof::line_marks(proofreader, run.text, &decor.prose)
                            .into_iter()
                            .map(|mark| {
                                let hidden = caret.is_some_and(|caret| {
                                    caret.line == run.line_i
                                        && proof::caret_hides(run.text, &mark, caret.column)
                                });
                                (mark, !hidden)
                            })
                            .collect();
                        line_marks = Some((run.line_i, marks));
                    }
                    line_marks
                        .iter()
                        .flat_map(|(_, marks)| marks)
                        .filter_map(|(range, shown)| {
                            let (left, right) = glyph_span(run.text, glyphs(), range.clone())?;
                            Some(RowMark {
                                range: range.clone(),
                                left,
                                right,
                                shown: *shown,
                            })
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            let spans = |ranges: &[Range<usize>]| -> Vec<(f32, f32)> {
                ranges
                    .iter()
                    .filter_map(|range| glyph_span(run.text, glyphs(), range.clone()))
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
                marks,
                tint: tint
                    .as_ref()
                    .filter(|(line, _)| *line == run.line_i)
                    .and_then(|(_, range)| glyph_span(run.text, glyphs(), range.clone())),
            }
        })
        .collect()
}

/// The zigzag under one row's glyphs of a misspelled word (rule 4.3.4): 1 px
/// squares from `left` to `right`, each 0 to 2 px below `top`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Zigzag {
    left: f32,
    right: f32,
    top: f32,
}

impl Zigzag {
    /// Under glyphs from `left` to `right` on a row with its baseline at
    /// `baseline` and its bottom at `bottom`, all in logical pixels; `None`
    /// when no whole pixel lies between the glyphs' edges.
    fn new(left: f32, right: f32, baseline: f32, bottom: f32) -> Option<Self> {
        let (left, right) = (left.ceil(), right.floor());
        (right > left).then(|| Self {
            left,
            right,
            top: (baseline + ZIGZAG_DROP)
                .round()
                .min((bottom - ZIGZAG_HEIGHT).floor()),
        })
    }

    fn bounds(self) -> Rectangle {
        Rectangle {
            x: self.left,
            y: self.top,
            width: self.right - self.left,
            height: ZIGZAG_HEIGHT,
        }
    }

    /// One square per whole pixel, the phase starting at `left`.
    fn squares(self) -> impl Iterator<Item = Rectangle> {
        let count = (self.right - self.left) as usize;
        (0..count).map(move |i| Rectangle {
            x: self.left + i as f32,
            y: self.top + ZIGZAG_STEPS[i % ZIGZAG_STEPS.len()],
            width: 1.0,
            height: 1.0,
        })
    }
}

/// The zigzag pictures of a frame, one per word and row, each shared by
/// every zigzag that draws the same squares in the same colour. tiny-skia
/// damages every changed primitive on its own and blits a picture's whole
/// extent each time it repaints a region the picture touches, so one picture
/// of a whole frame's squares cost a large blit per region on a note switch,
/// and 1 px quads seconds a frame. A picture per word keeps each blit to the
/// word's few pixels, and the vendored iced_tiny_skia merges the words'
/// damage into the text's. A picture is kept while frames draw it, so an
/// unchanged frame repeats the same handles at the same bounds and damages
/// nothing, and a word of a width already drawn reuses its picture.
#[derive(Debug, Default)]
struct Zigzags {
    pictures: HashMap<ZigzagKey, Option<ZigzagPicture>>,
}

/// What a zigzag picture draws: the squares of a zigzag `width` pixels wide
/// inside `cut` (relative to its left and top, as `f32` bits), in `color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ZigzagKey {
    width: u32,
    cut: [u32; 4],
    color: [u8; 4],
}

#[derive(Debug, Clone, PartialEq)]
struct ZigzagPicture {
    /// Where it is drawn, relative to its zigzag's left and top.
    bounds: Rectangle,
    handle: svg::Handle,
}

impl Zigzags {
    /// Where to draw each of `zigzags`, cut to `clip`, in `color`, and the
    /// picture that draws it. Pictures no zigzag of this frame draws are
    /// dropped.
    fn frame(
        &mut self,
        zigzags: impl IntoIterator<Item = Zigzag>,
        clip: Rectangle,
        color: Color,
    ) -> Vec<(Rectangle, svg::Handle)> {
        let mut kept = HashMap::new();
        let mut drawn = Vec::new();
        for zigzag in zigzags {
            let Some(visible) = zigzag.bounds().intersection(&clip) else {
                continue;
            };
            let origin = Vector::new(zigzag.left, zigzag.top);
            let cut = visible - origin;
            let width = zigzag.right - zigzag.left;
            let key = ZigzagKey {
                width: width as u32,
                cut: [cut.x, cut.y, cut.width, cut.height].map(f32::to_bits),
                color: color.into_rgba8(),
            };
            let picture = match kept.get(&key) {
                Some(picture) => Option::clone(picture),
                None => self.pictures.remove(&key).unwrap_or_else(|| {
                    let squares: Vec<Rectangle> = Zigzag {
                        left: 0.0,
                        right: width,
                        top: 0.0,
                    }
                    .squares()
                    .filter_map(|square| square.intersection(&cut))
                    .collect();
                    zigzag_svg(&squares, color).map(|(bounds, source)| ZigzagPicture {
                        bounds,
                        handle: svg::Handle::from_memory(source.into_bytes()),
                    })
                }),
            };
            if let Some(picture) = &picture {
                drawn.push((picture.bounds + origin, picture.handle.clone()));
            }
            kept.insert(key, picture);
        }
        self.pictures = kept;
        drawn
    }
}

/// The SVG source that fills `squares` with `color`, and where it is drawn:
/// the whole pixels around the squares, so each square keeps its place on
/// the pixel grid.
fn zigzag_svg(squares: &[Rectangle], color: Color) -> Option<(Rectangle, String)> {
    let first = squares.first()?;
    let (mut left, mut top) = (first.x, first.y);
    let (mut right, mut bottom) = (first.x + first.width, first.y + first.height);
    for square in squares {
        left = left.min(square.x);
        top = top.min(square.y);
        right = right.max(square.x + square.width);
        bottom = bottom.max(square.y + square.height);
    }
    let bounds = Rectangle {
        x: left.floor(),
        y: top.floor(),
        width: right.ceil() - left.floor(),
        height: bottom.ceil() - top.floor(),
    };
    let mut path = String::with_capacity(squares.len() * 24);
    for square in squares {
        path.push_str(&format!(
            "M{} {}h{}v{}h-{}z",
            square.x - bounds.x,
            square.y - bounds.y,
            square.width,
            square.height,
            square.width
        ));
    }
    let [r, g, b, _] = color.into_rgba8();
    let (width, height) = (bounds.width, bounds.height);
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}' \
         viewBox='0 0 {width} {height}'><path fill='#{r:02x}{g:02x}{b:02x}' \
         fill-opacity='{}' d='{path}'/></svg>",
        color.a
    );
    Some((bounds, source))
}

impl RowMark {
    /// The mark's zigzag on `row`, whose text origin is at `origin`.
    fn zigzag(&self, row: &VisualRow, origin: Point) -> Option<Zigzag> {
        Zigzag::new(
            origin.x + self.left,
            origin.x + self.right,
            origin.y + row.baseline,
            origin.y + row.top + row.height,
        )
    }
}

/// The marks of the drawn `rows`, whose text origin is at `origin`, one per
/// word in the order they are drawn, with the parts of their zigzags inside
/// `clip`.
#[cfg(any(test, feature = "test-support"))]
fn drawn_marks(rows: &[VisualRow], origin: Point, clip: Rectangle) -> Vec<DrawnMark> {
    let mut marks: Vec<DrawnMark> = Vec::new();
    for row in rows {
        for mark in &row.marks {
            let rect = mark
                .zigzag(row, origin)
                .and_then(|zigzag| zigzag.bounds().intersection(&clip));
            let index = match marks
                .iter()
                .position(|drawn| drawn.line == row.line && drawn.range == mark.range)
            {
                Some(index) => index,
                None => {
                    marks.push(DrawnMark {
                        line: row.line,
                        range: mark.range.clone(),
                        shown: mark.shown,
                        rects: Vec::new(),
                    });
                    marks.len() - 1
                }
            };
            marks[index].rects.extend(rect);
        }
    }
    marks
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

/// The row a popover hangs under, relative to the text origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorRow {
    /// The anchor's extent on the row, of no width for a place between two
    /// characters.
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl AnchorRow {
    pub fn bounds(self) -> Rectangle {
        Rectangle {
            x: self.left,
            y: self.top,
            width: self.right - self.left,
            height: self.bottom - self.top,
        }
    }
}

/// Where a popover anchored to `range` of `line` hangs: the extent of the
/// range's glyphs on the first visible row that holds any, and that row's
/// top and bottom. A range with no glyphs of its own, which an empty one
/// never has, is the place where it starts: where the engine draws its
/// caret, when the caret is there with nothing selected, on the side of a
/// wrap the caret is on; otherwise where a caret there would stand
/// ([`Backend::caret_at`]), which between two rows of a wrapped line is the
/// start of the second.
fn anchor_row(editor: &Backend, line: usize, range: Range<usize>) -> Option<AnchorRow> {
    let spanned = editor
        .buffer()
        .layout_runs()
        .filter(|run| run.line_i == line)
        .find_map(|run| {
            let glyphs = run
                .glyphs
                .iter()
                .map(|glyph| (glyph.start..glyph.end, glyph.x, glyph.w));
            let (left, right) = glyph_span(run.text, glyphs, range.clone())?;
            Some(AnchorRow {
                left,
                right,
                top: run.line_top,
                bottom: run.line_top + run.line_height,
            })
        });
    spanned.or_else(|| {
        let place = Position {
            line,
            column: range.start,
        };
        let cursor = editor.cursor();
        let live = cursor.position == place && cursor.selection.is_none_or(|end| end == place);
        let (caret, height) =
            (live.then(|| editor.caret()).flatten()).or_else(|| editor.caret_at(place))?;
        Some(AnchorRow {
            left: caret.x,
            right: caret.x,
            top: caret.y,
            bottom: caret.y + height,
        })
    })
}

/// Where a point falls in the laid out text; see [`place_under`].
pub struct Place {
    /// The line of the row under the point.
    pub line: usize,
    /// The misspelled word there.
    pub mark: Option<Range<usize>>,
    /// The byte in the line at which the glyph under the point starts.
    /// Not looked for under a misspelled word, which is the hit there
    /// ([`crate::hit`]).
    pub column: Option<usize>,
}

/// Where `point`, relative to the text origin, falls: the visible row
/// holding it, and on that row
///
/// - the first of the marks `marks` gives for the row's line whose glyphs
///   on the row span `point`'s x, and with none
/// - the glyph that holds `point`'s x. A span is hit by its glyph and not
///   by the extent of its glyphs: in text of two directions the glyphs of
///   a span can lie apart, with a glyph of no span between them.
fn place_under(
    editor: &Backend,
    point: Point,
    marks: impl FnOnce(usize, &str) -> Vec<Range<usize>>,
) -> Option<Place> {
    let run = editor
        .buffer()
        .layout_runs()
        .find(|run| point.y >= run.line_top && point.y < run.line_top + run.line_height)?;
    let glyphs = || {
        run.glyphs
            .iter()
            .map(|glyph| (glyph.start..glyph.end, glyph.x, glyph.w))
    };
    let mark = marks(run.line_i, run.text).into_iter().find(|mark| {
        glyph_span(run.text, glyphs(), mark.clone())
            .is_some_and(|(left, right)| point.x >= left && point.x < right)
    });
    let column = match mark {
        Some(_) => None,
        None => glyphs()
            .find(|(_, x, width)| point.x >= *x && point.x < x + width)
            .map(|(bytes, _, _)| bytes.start),
    };
    Some(Place {
        line: run.line_i,
        mark,
        column,
    })
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
            tasks: None,
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
            tasks: None,
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
                tasks: None,
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
                tasks: None,
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
                tasks: None,
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

    /// JetBrains Mono's `0` at 14 px: 600 units of a 1000-unit em.
    const ADVANCE: f32 = 8.4;

    fn editor_with(content: &Content, line_numbers: bool, measure: Measure) -> TextEditor<'_> {
        TextEditor::new(
            content,
            "e",
            fixtures::SETTINGS,
            fixtures::STYLE,
            line_numbers,
            Metrics::default(),
            true,
        )
        .measure(measure)
    }

    #[test]
    fn padding_reserves_the_gutter_only_when_shown() {
        let content = Content::new(BackendKind::Iced, "");
        let shown = editor_with(&content, true, Measure::default()).padding(600.0, ADVANCE);
        let hidden = editor_with(&content, false, Measure::default()).padding(600.0, ADVANCE);
        assert_eq!(
            (shown.left, shown.right, shown.top, shown.bottom),
            (60.0, 24.0, 24.0, 24.0)
        );
        assert_eq!((hidden.left, hidden.right), (24.0, 24.0));
    }

    #[test]
    fn left_placed_text_wraps_at_the_column_or_short_of_the_edge() {
        let content = Content::new(BackendKind::Iced, "");
        let editor = editor_with(
            &content,
            true,
            Measure {
                wrap_column: Some(100),
                centred: false,
            },
        );
        // 100 advances of 8.4 px, rounded, and a pixel of slack.
        let wide = editor.columns(1200.0, ADVANCE);
        assert_eq!(wide.text, (60.0, 841.0));
        assert_eq!(wide.band, (0.0, 1200.0));
        let padding = editor.padding(1200.0, ADVANCE);
        assert_eq!((padding.left, padding.right), (60.0, 299.0));
        // Narrower than the column, the text stops RIGHT_INSET short.
        assert_eq!(editor.columns(600.0, ADVANCE).text, (60.0, 516.0));
        // The wrap column follows the font's advance.
        assert_eq!(editor.columns(1200.0, 10.8).text, (60.0, 1081.0));
        let bounds = Rectangle::new(Point::new(280.0, 98.0), Size::new(1200.0, 600.0));
        assert_eq!(editor.band(bounds, ADVANCE), (280.0, 1200.0));
    }

    #[test]
    fn centred_text_keeps_equal_margins_with_the_gutter_in_the_left_one() {
        let content = Content::new(BackendKind::Iced, "");
        let editor = editor_with(
            &content,
            true,
            Measure {
                wrap_column: Some(100),
                centred: true,
            },
        );
        let wide = editor.columns(1600.0, ADVANCE);
        assert_eq!(wide.text, (379.0, 841.0));
        // The bands start at the gutter and end at the text's inset.
        assert_eq!(wide.band, (319.0, 1228.0));
        // Narrow, the margins shrink to the gutter's width, never below.
        assert_eq!(editor.columns(800.0, ADVANCE).text, (60.0, 680.0));
        // The larger of 48 px and 6% of the width.
        let wider = editor.columns(2000.0, 20.0).text;
        assert_eq!(wider, (120.0, 1760.0));
        let bare = editor_with(
            &content,
            false,
            Measure {
                wrap_column: None,
                centred: true,
            },
        );
        assert_eq!(bare.columns(600.0, ADVANCE).text, (48.0, 504.0));
    }

    #[test]
    fn the_wrap_column_holds_exactly_that_many_characters() {
        fixtures::load_font();
        let advance = Advance::measure(fixtures::FONT, TEXT_SIZE).width;
        assert!((advance - ADVANCE).abs() < 0.01, "{advance}");
        let content = Content::new(BackendKind::Iced, "");
        for column in [20, 80, 100, 120] {
            let editor = editor_with(
                &content,
                true,
                Measure {
                    wrap_column: Some(column),
                    centred: false,
                },
            );
            let width = editor.columns(4000.0, advance).text.1;
            for kind in BackendKind::ALL {
                for glyph in ["0", "m", "i"] {
                    let line = glyph.repeat(usize::from(column));
                    assert_eq!(
                        laid_out(kind, &line, width).len(),
                        1,
                        "{kind:?} {column} {glyph}"
                    );
                    let over = format!("{line}{glyph}");
                    assert_eq!(
                        laid_out(kind, &over, width).len(),
                        2,
                        "{kind:?} {column} {glyph}"
                    );
                }
            }
        }
    }

    fn laid_out(kind: BackendKind, text: &str, width: f32) -> Vec<VisualRow> {
        laid_out_in(kind, text, Size::new(width, 1000.0))
    }

    fn laid_out_in(kind: BackendKind, text: &str, size: Size) -> Vec<VisualRow> {
        spelled(kind, text, size, None, None)
    }

    /// The rows of `text` laid out in `size`, marked by `proofreader`, the
    /// mark ending at `caret` hidden.
    fn spelled(
        kind: BackendKind,
        text: &str,
        size: Size,
        proofreader: Option<&dyn Proofreader>,
        caret: Option<Position>,
    ) -> Vec<VisualRow> {
        let mut content = Content::new(kind, text);
        content.rows(size, None, proofreader, caret)
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

    // ── Spelling marks ─────────────────────────────────────────────────────

    const LONG_TYPO: &str = "zzzzzzzzzzqqqqqqqqqqzzzzzzzzzzqqqqqqqqqq";

    fn rejects() -> fixtures::Rejects {
        fixtures::Rejects::new(&["teh", "Teh", "wouldn", LONG_TYPO])
    }

    /// `(line, mark text, left, right, shown)` for each row mark.
    fn row_marks<'a>(text: &'a str, rows: &[VisualRow]) -> Vec<(usize, &'a str, f32, f32, bool)> {
        let lines: Vec<&str> = text.split('\n').collect();
        rows.iter()
            .flat_map(|row| {
                let line = lines[row.line];
                row.marks.iter().map(move |mark| {
                    (
                        row.line,
                        &line[mark.range.clone()],
                        mark.left,
                        mark.right,
                        mark.shown,
                    )
                })
            })
            .collect()
    }

    fn near(found: f32, expected: f32) -> bool {
        (found - expected).abs() < 0.5
    }

    #[test]
    fn marks_cover_the_glyphs_of_each_misspelled_word() {
        let proofreader = rejects();
        let text = "Teh cat sat on teh mat\nfine words `teh` here";
        let rows = spelled(
            BackendKind::Owned,
            text,
            Size::new(600.0, 200.0),
            Some(&proofreader),
            None,
        );
        let marks = row_marks(text, &rows);
        assert_eq!(marks.len(), 2, "{marks:?}");
        for ((line, word, left, right, shown), cells) in marks.iter().zip([0.0..3.0, 15.0..18.0]) {
            assert_eq!(
                (*line, *word, *shown),
                (0, &text[cells.start as usize..cells.end as usize], true)
            );
            assert!(near(*left, cells.start * ADVANCE), "{left}");
            assert!(near(*right, cells.end * ADVANCE), "{right}");
        }
        // Once per line: the code span is not prose, so the second line has
        // no mark, but it is still asked about its prose.
        assert_eq!(proofreader.calls(), 2);
    }

    #[test]
    fn a_word_wrapped_mid_word_is_marked_on_each_of_its_rows() {
        let proofreader = rejects();
        let text = format!("ab {LONG_TYPO} cd");
        let rows = spelled(
            BackendKind::Owned,
            &text,
            Size::new(120.0, 400.0),
            Some(&proofreader),
            None,
        );
        assert!(rows.len() >= 3, "{rows:?}");
        let marked: Vec<&VisualRow> = rows.iter().filter(|row| !row.marks.is_empty()).collect();
        assert!(marked.len() >= 3, "{rows:?}");
        let mut width = 0.0;
        for (i, row) in marked.iter().enumerate() {
            let [mark] = row.marks.as_slice() else {
                panic!("one mark a row: {row:?}");
            };
            assert_eq!(mark.range, 3..3 + LONG_TYPO.len());
            // Continuation rows start the word at their left edge.
            if i > 0 {
                assert!(near(mark.left, 0.0), "{mark:?}");
            }
            width += mark.right - mark.left;
        }
        assert!(near(width, LONG_TYPO.len() as f32 * ADVANCE), "{width}");
        // One line, however many rows it wraps to.
        assert_eq!(proofreader.calls(), 1);
    }

    #[test]
    fn a_word_on_a_hanging_row_is_marked_under_its_glyphs() {
        let proofreader = rejects();
        // Fourteen cells a row: "gamma teh" wraps, hanging under "alpha".
        let text = "- alpha beta gamma teh";
        let rows = spelled(
            BackendKind::Owned,
            text,
            Size::new(120.0, 200.0),
            Some(&proofreader),
            None,
        );
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].marks.is_empty());
        let [mark] = rows[1].marks.as_slice() else {
            panic!("{rows:?}");
        };
        assert_eq!(&text[mark.range.clone()], "teh");
        // Two cells of hang, then "gamma ".
        assert!(near(mark.left, 8.0 * ADVANCE), "{mark:?}");
        assert!(near(mark.right, 11.0 * ADVANCE), "{mark:?}");
    }

    #[test]
    fn a_heading_row_marks_its_larger_glyphs() {
        let proofreader = rejects();
        let text = "# Teh title\nteh body";
        let rows = spelled(
            BackendKind::Owned,
            text,
            Size::new(600.0, 200.0),
            Some(&proofreader),
            None,
        );
        let marks = row_marks(text, &rows);
        assert_eq!(marks.len(), 2, "{marks:?}");
        let heading = marks[0].3 - marks[0].2;
        let body = marks[1].3 - marks[1].2;
        assert!(near(body, 3.0 * ADVANCE), "{body}");
        // H1 text is 1.35 em.
        assert!(near(heading, 3.0 * ADVANCE * 1.35), "{heading}");
        // The `# ` marker is not prose.
        assert!(marks[0].2 > ADVANCE * 2.0, "{marks:?}");
    }

    #[test]
    fn the_caret_at_a_marks_end_hides_only_that_mark() {
        let proofreader = rejects();
        // Half-typed `wouldn't`, straight and curly.
        let text = "teh teh\nwouldn' wouldn’ teh";
        let size = Size::new(600.0, 200.0);
        let at = |line, column| Some(Position { line, column });
        let shown = |caret| -> Vec<bool> {
            spelled(BackendKind::Owned, text, size, Some(&proofreader), caret)
                .iter()
                .flat_map(|row| row.marks.iter().map(|mark| mark.shown))
                .collect()
        };
        assert_eq!(shown(None), [true, true, true, true, true]);
        assert_eq!(shown(at(0, 3)), [false, true, true, true, true]);
        assert_eq!(shown(at(0, 7)), [true, false, true, true, true]);
        // Inside the word, or a space past it, hides nothing.
        assert_eq!(shown(at(0, 2)), [true; 5]);
        assert_eq!(shown(at(0, 4)), [true; 5]);
        // Only apostrophes between the caret and the end: `wouldn'`, and
        // `wouldn’` with a curly one.
        assert_eq!(shown(at(1, 7)), [true, true, false, true, true]);
        assert_eq!(shown(at(1, 8)), [true; 5]);
        let curly = "wouldn' wouldn’".len();
        assert_eq!(shown(at(1, curly)), [true, true, true, false, true]);
        // The same column on another line hides nothing.
        assert_eq!(shown(at(1, 3)), [true; 5]);
    }

    #[test]
    fn only_a_focused_caret_with_nothing_selected_hides_a_mark() {
        let cursor = Cursor {
            position: Position { line: 2, column: 5 },
            selection: None,
        };
        assert_eq!(hiding_caret(true, cursor), Some(cursor.position));
        assert_eq!(hiding_caret(false, cursor), None);
        let selected = Cursor {
            selection: Some(Position { line: 2, column: 1 }),
            ..cursor
        };
        assert_eq!(hiding_caret(true, selected), None);
    }

    #[test]
    fn a_caret_on_a_row_below_the_view_still_hides_its_wrapped_word() {
        let proofreader = rejects();
        let text = format!("ab {LONG_TYPO}");
        // Two rows in view; the word wraps to at least three.
        let size = Size::new(120.0, 2.0 * LINE_HEIGHT);
        let mut content = Content::new(BackendKind::Owned, &text);
        content.move_to(Cursor {
            position: Position {
                line: 0,
                column: text.len(),
            },
            selection: None,
        });
        content.rows(size, None, Some(&proofreader), None);
        content.perform(Action::Scroll { lines: -10 });
        let rows = content.rows(size, None, Some(&proofreader), None);
        assert_eq!(content.scroll().vertical, 0.0, "{:?}", content.scroll());
        assert!(rows.iter().any(|row| !row.marks.is_empty()), "{rows:?}");
        // The widget draws no caret for a row out of view.
        assert!(content.selection_geometry().is_none());

        let caret = hiding_caret(true, content.cursor());
        let rows = content.rows(size, None, Some(&proofreader), caret);
        let shown: Vec<bool> = rows
            .iter()
            .flat_map(|row| row.marks.iter().map(|mark| mark.shown))
            .collect();
        assert!(
            !shown.is_empty() && shown.iter().all(|shown| !shown),
            "{rows:?}"
        );
    }

    #[test]
    fn the_classic_engine_never_asks_the_proofreader() {
        let proofreader = rejects();
        let text = "Teh cat\n# teh\n- [x] teh";
        let size = Size::new(600.0, 200.0);
        let rows = spelled(BackendKind::Iced, text, size, Some(&proofreader), None);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.marks.is_empty()));
        assert_eq!(proofreader.calls(), 0);
        // With none set, the owned engine marks nothing.
        let rows = spelled(BackendKind::Owned, text, size, None, None);
        assert!(rows.iter().all(|row| row.marks.is_empty()));
        assert_eq!(
            spelled(BackendKind::Owned, text, size, Some(&proofreader), None)
                .iter()
                .map(|row| row.marks.len())
                .sum::<usize>(),
            3
        );
    }

    #[test]
    fn lines_without_prose_are_not_asked_about() {
        let proofreader = rejects();
        let text = "```\nteh\n```\n\n---\nteh";
        let rows = spelled(
            BackendKind::Owned,
            text,
            Size::new(600.0, 300.0),
            Some(&proofreader),
            None,
        );
        assert_eq!(row_marks(text, &rows).len(), 1);
        assert_eq!(proofreader.calls(), 1);
    }

    // ── The zigzag ─────────────────────────────────────────────────────────

    /// `(x, y)` of each square.
    fn squares(zigzag: Zigzag) -> Vec<(f32, f32)> {
        zigzag
            .squares()
            .map(|square| {
                assert_eq!(square.size(), Size::new(1.0, 1.0));
                (square.x, square.y)
            })
            .collect()
    }

    #[test]
    fn the_zigzag_steps_two_one_zero_one_from_its_left_pixel() {
        // Glyphs from 10.3 to 19.7: whole pixels 11 to 19.
        let zigzag = Zigzag::new(10.3, 19.7, 30.4, 48.0).unwrap();
        assert_eq!(
            zigzag,
            Zigzag {
                left: 11.0,
                right: 19.0,
                top: 32.0
            }
        );
        assert_eq!(
            squares(zigzag),
            [
                (11.0, 34.0),
                (12.0, 33.0),
                (13.0, 32.0),
                (14.0, 33.0),
                (15.0, 34.0),
                (16.0, 33.0),
                (17.0, 32.0),
                (18.0, 33.0),
            ]
        );
        assert_eq!(
            zigzag.bounds(),
            Rectangle::new(Point::new(11.0, 32.0), Size::new(8.0, 3.0))
        );
        // Whole edges are kept; the top rounds half up.
        let zigzag = Zigzag::new(4.0, 6.0, 7.5, 48.0).unwrap();
        assert_eq!(
            zigzag,
            Zigzag {
                left: 4.0,
                right: 6.0,
                top: 10.0
            }
        );
        assert_eq!(squares(zigzag), [(4.0, 12.0), (5.0, 11.0)]);
    }

    #[test]
    fn the_zigzag_moves_up_to_stay_inside_its_row() {
        // round(38.2 + 2) = 40 would put the squares at 40..43, past 41.
        let zigzag = Zigzag::new(0.0, 4.0, 38.2, 41.0).unwrap();
        assert_eq!(zigzag.top, 38.0);
        assert!(squares(zigzag)
            .iter()
            .all(|&(_, y)| (38.0..41.0).contains(&y)));
        // A fractional bottom: the squares stay above it.
        assert_eq!(Zigzag::new(0.0, 4.0, 38.2, 41.6).unwrap().top, 38.0);
        // Room enough: it stays where it is.
        assert_eq!(Zigzag::new(0.0, 4.0, 38.2, 43.0).unwrap().top, 40.0);
    }

    #[test]
    fn a_zigzag_without_a_whole_pixel_draws_nothing() {
        assert_eq!(Zigzag::new(10.2, 10.9, 0.0, 24.0), None);
        assert_eq!(Zigzag::new(10.5, 10.5, 0.0, 24.0), None);
        assert_eq!(Zigzag::new(12.0, 10.0, 0.0, 24.0), None);
        assert_eq!(
            squares(Zigzag::new(10.0, 11.0, 0.0, 24.0).unwrap()),
            [(10.0, 4.0)]
        );
    }

    #[test]
    fn each_zigzag_is_a_picture_of_its_squares_on_the_pixel_grid() {
        // Two words' zigzags, the second cut by the rows' clip at y = 41.5.
        let clip = Rectangle::new(Point::new(0.0, 0.0), Size::new(400.0, 41.5));
        let zigzags = [
            Zigzag::new(10.3, 14.0, 30.4, 48.0).unwrap(),
            Zigzag::new(20.0, 22.0, 37.0, 48.0).unwrap(),
        ];
        let color = Color::from_rgb8(0xe0, 0x6c, 0x6c);
        let drawn = Zigzags::default().frame(zigzags, clip, color);
        // Each from its first square's left and its top step to its last
        // square or the cut half pixel, rounded out to whole pixels.
        assert_eq!(
            drawn.iter().map(|(bounds, _)| *bounds).collect::<Vec<_>>(),
            [
                Rectangle::new(Point::new(11.0, 32.0), Size::new(3.0, 3.0)),
                Rectangle::new(Point::new(20.0, 40.0), Size::new(2.0, 2.0)),
            ]
        );
        let sources: Vec<String> = drawn
            .iter()
            .map(|(_, handle)| match handle.data() {
                svg::Data::Bytes(bytes) => String::from_utf8(bytes.to_vec()).unwrap(),
                svg::Data::Path(path) => panic!("a picture from a file: {path:?}"),
            })
            .collect();
        assert_eq!(
            sources,
            [
                "<svg xmlns='http://www.w3.org/2000/svg' width='3' height='3' \
                 viewBox='0 0 3 3'><path fill='#e06c6c' fill-opacity='1' \
                 d='M0 2h1v1h-1zM1 1h1v1h-1zM2 0h1v1h-1z'/></svg>",
                "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2' \
                 viewBox='0 0 2 2'><path fill='#e06c6c' fill-opacity='1' \
                 d='M0 1h1v0.5h-1zM1 0h1v1h-1z'/></svg>",
            ]
        );
        assert_eq!(zigzag_svg(&[], color), None);
        // A cut that holds no square draws nothing.
        let above = Rectangle::new(Point::new(0.0, 0.0), Size::new(400.0, 40.0));
        let lone = Zigzag::new(20.0, 21.0, 37.0, 48.0).unwrap();
        assert_eq!(Zigzags::default().frame([lone], above, color), []);
    }

    #[test]
    fn zigzag_pictures_are_shared_by_what_they_draw_and_kept_while_drawn() {
        let clip = Rectangle::new(Point::ORIGIN, Size::new(400.0, 400.0));
        let red = Color::from_rgb8(0xe0, 0x6c, 0x6c);
        let blue = Color::from_rgb8(0x40, 0x60, 0xe0);
        // Two words of one width on two rows, and a wider one.
        let words = [
            Zigzag::new(10.0, 30.0, 30.0, 48.0).unwrap(),
            Zigzag::new(50.0, 70.0, 54.0, 72.0).unwrap(),
            Zigzag::new(10.0, 40.0, 78.0, 96.0).unwrap(),
        ];
        let mut zigzags = Zigzags::default();
        let first = zigzags.frame(words, clip, red);
        assert_eq!(first[0].1.id(), first[1].1.id());
        assert_ne!(first[0].1.id(), first[2].1.id());
        assert_eq!(first[1].0, first[0].0 + Vector::new(40.0, 24.0));
        // An unchanged frame draws the same handles at the same bounds, so
        // the renderer sees unchanged primitives.
        assert_eq!(zigzags.frame(words, clip, red), first);
        // Another note's word of the same width reuses the picture.
        let other = Zigzag::new(100.0, 120.0, 200.0, 218.0).unwrap();
        let moved = zigzags.frame([other], clip, red);
        assert_eq!(moved[0].1.id(), first[0].1.id());
        assert_eq!(zigzags.pictures.len(), 1);
        assert_ne!(
            zigzags.frame([other], clip, blue)[0].1.id(),
            first[0].1.id()
        );
        assert_eq!(zigzags.frame([], clip, red), []);
        assert!(zigzags.pictures.is_empty());
    }

    #[test]
    fn the_zigzag_sits_under_the_baseline_inside_every_row() {
        let proofreader = rejects();
        let text = format!("# Teh title\nteh body\n### teh\nab {LONG_TYPO}");
        let rows = spelled(
            BackendKind::Owned,
            &text,
            Size::new(160.0, 600.0),
            Some(&proofreader),
            None,
        );
        let origin = Point::new(60.0, 24.0);
        let mut drawn = 0;
        for row in &rows {
            for mark in &row.marks {
                let zigzag = mark.zigzag(row, origin).unwrap();
                let baseline = origin.y + row.baseline;
                let bottom = origin.y + row.top + row.height;
                assert_eq!(zigzag.top, (baseline + ZIGZAG_DROP).round(), "{row:?}");
                assert!(zigzag.top + ZIGZAG_HEIGHT <= bottom, "{row:?}");
                assert_eq!(zigzag.left, (origin.x + mark.left).ceil());
                assert_eq!(zigzag.right, (origin.x + mark.right).floor());
                // The phase restarts on every row, heading rows included,
                // and the squares keep their size.
                let first = zigzag.squares().next().unwrap();
                assert_eq!(first.y, zigzag.top + 2.0);
                assert_eq!(first.size(), Size::new(1.0, 1.0));
                drawn += 1;
            }
        }
        // The heading, the body word, the H3, and the long word's rows.
        assert!(drawn >= 5, "{drawn}");
        assert!(rows[0].height > LINE_HEIGHT);
    }

    #[test]
    fn drawn_marks_join_a_words_rows_and_clip_them() {
        let row = |line, top: f32, marks: Vec<RowMark>| VisualRow {
            line,
            top,
            height: LINE_HEIGHT,
            baseline: top + 17.0,
            number_top: top,
            starts_line: true,
            ends_line: true,
            width: 100.0,
            strikes: Vec::new(),
            code_spans: Vec::new(),
            code: false,
            marks,
            tint: None,
        };
        let mark = |range: Range<usize>, left, right, shown| RowMark {
            range,
            left,
            right,
            shown,
        };
        let rows = [
            row(
                0,
                0.0,
                vec![mark(0..3, 0.0, 25.2, true), mark(8..20, 60.0, 100.0, true)],
            ),
            row(0, 24.0, vec![mark(8..20, 0.0, 40.4, true)]),
            // No whole pixel: listed, with no zigzag.
            row(1, 48.0, vec![mark(4..5, 10.2, 10.8, false)]),
            row(2, 72.0, vec![mark(0..4, 0.0, 30.0, true)]),
        ];
        let origin = Point::new(60.0, 24.0);
        // The last row's zigzag falls below the clip.
        let clip = Rectangle::new(Point::new(0.0, 24.0), Size::new(400.0, 90.0));
        let marks = drawn_marks(&rows, origin, clip);
        let rect = |x, y, width| Rectangle::new(Point::new(x, y), Size::new(width, 3.0));
        assert_eq!(
            marks,
            [
                DrawnMark {
                    line: 0,
                    range: 0..3,
                    shown: true,
                    rects: vec![rect(60.0, 43.0, 25.0)],
                },
                DrawnMark {
                    line: 0,
                    range: 8..20,
                    shown: true,
                    rects: vec![rect(120.0, 43.0, 40.0), rect(60.0, 67.0, 40.0)],
                },
                DrawnMark {
                    line: 1,
                    range: 4..5,
                    shown: false,
                    rects: Vec::new(),
                },
                DrawnMark {
                    line: 2,
                    range: 0..4,
                    shown: true,
                    rects: Vec::new(),
                },
            ]
        );
    }

    // ── The suggestions menu ───────────────────────────────────────────────

    /// The widget's bounds in the window, for event tests.
    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 600.0,
        height: 400.0,
    };

    /// A widget over `content`, checked by `proofreader`, with `menu` open.
    fn spelling<'a>(
        content: &'a Content,
        proofreader: &'a dyn Proofreader,
        menu: Option<&'a SpellMenu>,
    ) -> TextEditor<'a> {
        editor_with(content, true, Measure::default())
            .proofreader(Some(proofreader))
            .spell_menu(menu)
    }

    /// A fresh widget state, laid out for [`BOUNDS`].
    fn laid_out_state(editor: &TextEditor<'_>) -> State {
        fixtures::load_font();
        let mut state = State::new(fixtures::SETTINGS);
        editor.lay_out(&mut state, BOUNDS.size());
        state
    }

    /// What the widget publishes for `event` with the pointer at `pointer`,
    /// and whether it captured it.
    fn send(
        editor: &mut TextEditor<'_>,
        state: &mut State,
        event: Event,
        pointer: Option<Point>,
    ) -> (Vec<Message>, bool) {
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        let cursor = pointer.map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available);
        editor.on_event(
            state,
            &event,
            BOUNDS,
            cursor,
            &mut clipboard::Null,
            &mut shell,
        );
        let captured = shell.event_status() == iced::event::Status::Captured;
        (messages, captured)
    }

    fn press(button: mouse::Button) -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(button))
    }

    /// The window point over cell `cells` of row `row` of a widget at
    /// [`BOUNDS`] with the gutter shown: 60 px of lead, 24 px of padding.
    fn over(cells: f32, row: f32) -> Point {
        Point::new(60.0 + cells * ADVANCE, 24.0 + row * LINE_HEIGHT + 12.0)
    }

    #[test]
    fn a_right_click_on_a_misspelling_takes_the_keyboard_and_asks_for_the_menu() {
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let mut editor = spelling(&content, &proofreader, None);
        let mut state = laid_out_state(&editor);
        state.drag_click = Some(mouse::click::Kind::Single);
        assert!(state.focus.is_none());
        let (messages, captured) = send(
            &mut editor,
            &mut state,
            press(mouse::Button::Right),
            Some(over(5.0, 0.0)),
        );
        let [Message::Spell(SpellEvent::OpenAt(at))] = messages.as_slice() else {
            panic!("{messages:?}");
        };
        assert!(
            (at.x - 5.0 * ADVANCE).abs() < 0.01 && at.y == 12.0,
            "{at:?}"
        );
        assert!(captured);
        assert!(state.focus.is_some(), "the editor takes the keyboard");
        assert!(state.drag_click.is_none(), "a drag in progress ends");
        assert_eq!(content.cursor().position, Position { line: 0, column: 0 });
    }

    /// What a hit is, in a word.
    fn hit_kind(hit: &Hit) -> &'static str {
        use wren_markdown::span::SpanKind;
        match hit {
            Hit::Misspelling(_) => "misspelling",
            Hit::Span(span) => match span.kind {
                SpanKind::Box(_) => "box",
                SpanKind::Owner { .. } => "owner",
                SpanKind::Tag => "tag",
                SpanKind::BlockId => "block id",
                SpanKind::Link { .. } => "link",
                SpanKind::Code => "code",
                _ => "another span",
            },
        }
    }

    /// One hit test serves a misspelled word and the classifier's spans:
    /// the word wins where it lies in a span, the owned engine alone has
    /// misspellings, and both engines have the spans.
    #[test]
    fn the_hit_under_a_point_is_a_misspelling_before_a_span_on_both_engines() {
        let proofreader = fixtures::Rejects::new(&["sam", "plna", "home", "teh"]);
        let text = "- [/] Mail @sam the [plna](https://x.io) #home ^b1\nplain teh `code`";
        let settings = HighlighterSettings {
            tasks: Some(wren_markdown::TaskOptions {
                today: wren_markdown::date::Date::new(2026, 11, 19).expect("a date"),
            }),
            ..fixtures::SETTINGS
        };
        for kind in BackendKind::ALL {
            let content = Content::new(kind, text);
            let editor = TextEditor::new(
                &content,
                "e",
                settings,
                fixtures::STYLE,
                true,
                Metrics::default(),
                true,
            )
            .proofreader(Some(&proofreader));
            fixtures::load_font();
            let mut state = State::new(settings);
            editor.lay_out(&mut state, BOUNDS.size());
            let spelled = kind == BackendKind::Owned;
            // A word in a span is the word where spelling is checked, and
            // the span where it is not.
            let word = |range: Range<usize>, span: &'static str, whole: Range<usize>| {
                if spelled {
                    Some(("misspelling", range))
                } else {
                    Some((span, whole))
                }
            };
            for (cells, row, expected) in [
                (0.5, 0.0, None),
                (3.5, 0.0, Some(("box", 2..5))),
                (7.5, 0.0, None),
                (11.5, 0.0, Some(("owner", 11..15))),
                (13.5, 0.0, word(12..15, "owner", 11..15)),
                (20.5, 0.0, Some(("link", 20..40))),
                (22.5, 0.0, word(21..25, "link", 20..40)),
                (30.5, 0.0, Some(("link", 20..40))),
                (41.5, 0.0, Some(("tag", 41..46))),
                (43.5, 0.0, word(42..46, "tag", 41..46)),
                (48.5, 0.0, Some(("block id", 47..50))),
                (55.0, 0.0, None),
                (2.5, 1.0, None),
                (7.5, 1.0, spelled.then_some(("misspelling", 6..9))),
                (12.5, 1.0, Some(("code", 10..16))),
                (2.5, 2.0, None),
            ] {
                let point = Point::new(cells * ADVANCE, row * LINE_HEIGHT + 12.0);
                let found = editor.hit_under(&state, point);
                assert_eq!(
                    found
                        .as_ref()
                        .map(|(_, hit)| (hit_kind(hit), hit.range().clone())),
                    expected,
                    "{kind:?} cell {cells} row {row}"
                );
                assert!(
                    found.as_ref().is_none_or(|(line, _)| *line == row as usize),
                    "{kind:?} cell {cells} row {row}"
                );
                // The right-click's own test is this one, kept to the words.
                assert_eq!(
                    editor.misspelling_under(&state, point),
                    found.and_then(|(line, hit)| Some((line, hit.misspelling()?))),
                    "{kind:?} cell {cells} row {row}"
                );
            }
        }
    }

    /// The host's tasks reach the glyphs on both engines, and a new day
    /// restyles the lines the widget already laid out: the same due date is
    /// drawn in the date colour, then overdue, with nothing in the text
    /// changed.
    #[test]
    fn a_new_day_redraws_a_passed_due_date_overdue_on_both_engines() {
        use iced::advanced::graphics::text::to_color;

        let text = "- [ ] Pay the invoice @sam 📅 2026-11-20\nplain";
        let date = text.find('📅').expect("a due date");
        let owner = text.find('@').expect("an owner");
        let day = |day| HighlighterSettings {
            tasks: Some(wren_markdown::TaskOptions {
                today: wren_markdown::date::Date::new(2026, 11, day).expect("a date"),
            }),
            ..fixtures::SETTINGS
        };
        for kind in BackendKind::ALL {
            let content = Content::new(kind, text);
            let colour_at = |byte: usize| {
                let backend = content.0.borrow();
                let line = &backend.buffer().lines[0];
                line.attrs_list().get_span(byte).color_opt
            };
            let weight_at = |byte: usize| {
                let backend = content.0.borrow();
                let line = &backend.buffer().lines[0];
                line.attrs_list().get_span(byte).weight
            };
            fixtures::load_font();
            let mut state = State::new(day(19));
            let widget = |settings| {
                TextEditor::new(
                    &content,
                    "e",
                    settings,
                    fixtures::STYLE,
                    true,
                    Metrics::default(),
                    true,
                )
            };
            widget(day(19)).lay_out(&mut state, BOUNDS.size());
            let palette = fixtures::DARK;
            assert_eq!(colour_at(date), Some(to_color(palette.date)), "{kind:?}");
            assert_eq!(colour_at(owner), Some(to_color(palette.owner)), "{kind:?}");
            let regular = weight_at(date);
            // Due today is not late.
            widget(day(20)).lay_out(&mut state, BOUNDS.size());
            assert_eq!(colour_at(date), Some(to_color(palette.date)), "{kind:?}");
            widget(day(21)).lay_out(&mut state, BOUNDS.size());
            assert_eq!(colour_at(date), Some(to_color(palette.overdue)), "{kind:?}");
            assert_eq!(weight_at(date), cosmic_text::Weight::BOLD, "{kind:?}");
            assert_ne!(regular, cosmic_text::Weight::BOLD, "{kind:?}");
            assert_eq!(colour_at(owner), Some(to_color(palette.owner)), "{kind:?}");
            // With tasks off the same glyphs are body text.
            widget(fixtures::SETTINGS).lay_out(&mut state, BOUNDS.size());
            assert_eq!(colour_at(date), None, "{kind:?}");
            assert_eq!(colour_at(owner), None, "{kind:?}");
        }
    }

    /// In text of two directions a span's glyphs can lie apart. `ב* ג` is
    /// laid out right to left, so the `ג` after the emphasis stands between
    /// its `A` and its closing `*`: a span is hit by the glyph under the
    /// pointer, never by the stretch from its first glyph to its last.
    #[test]
    fn a_glyph_between_a_spans_glyphs_is_not_the_span() {
        let text = "x *A ב* ג";
        let after = text.find('ג').expect("the letter after the emphasis");
        let emphasis = text.find('*').expect("an opener")..text.rfind('*').expect("a closer") + 1;
        for kind in BackendKind::ALL {
            let content = Content::new(kind, text);
            let editor = editor_with(&content, true, Measure::default());
            let state = laid_out_state(&editor);
            // The middle of each glyph on the row, by the byte it starts at.
            let middles: Vec<(usize, f32)> = {
                let backend = content.0.borrow();
                let run = backend.buffer().layout_runs().next().expect("a row");
                run.glyphs
                    .iter()
                    .map(|glyph| (glyph.start, glyph.x + glyph.w / 2.0))
                    .collect()
            };
            let middle = |byte: usize| {
                let (_, x) = middles
                    .iter()
                    .find(|(start, _)| *start == byte)
                    .expect("a glyph");
                Point::new(*x, 12.0)
            };
            let inside: Vec<f32> = middles
                .iter()
                .filter(|(start, _)| emphasis.contains(start))
                .map(|(_, x)| *x)
                .collect();
            let (left, right) = inside
                .iter()
                .fold((f32::MAX, f32::MIN), |(left, right), x| {
                    (left.min(*x), right.max(*x))
                });
            let point = middle(after);
            assert!(
                left < point.x && point.x < right,
                "{kind:?}: the letter stands between the emphasis's glyphs: {middles:?}"
            );
            assert_eq!(editor.hit_under(&state, point), None, "{kind:?}");
            // Every glyph of the emphasis is the emphasis, wherever it stands.
            for (byte, _) in text
                .char_indices()
                .filter(|(byte, _)| emphasis.contains(byte))
            {
                let hit = editor.hit_under(&state, middle(byte));
                assert_eq!(
                    hit.map(|(line, hit)| (line, hit.range().clone())),
                    Some((0, emphasis.clone())),
                    "{kind:?} byte {byte}"
                );
            }
            assert_eq!(editor.hit_under(&state, middle(0)), None, "{kind:?}");
        }
    }

    #[test]
    fn a_right_click_anywhere_else_changes_nothing() {
        let proofreader = rejects();
        for (kind, point) in [
            (BackendKind::Owned, over(1.0, 0.0)),
            (BackendKind::Owned, over(9.0, 0.0)),
            (BackendKind::Owned, over(1.0, 2.0)),
            (BackendKind::Owned, Point::new(20.0, 36.0)),
            (BackendKind::Owned, Point::new(700.0, 36.0)),
            // The classic engine has no marks to open a menu for.
            (BackendKind::Iced, over(5.0, 0.0)),
        ] {
            let content = Content::new(kind, "cat teh");
            let mut editor = spelling(&content, &proofreader, None);
            let mut state = laid_out_state(&editor);
            let (messages, captured) = send(
                &mut editor,
                &mut state,
                press(mouse::Button::Right),
                Some(point),
            );
            assert!(messages.is_empty(), "{kind:?} {point:?}: {messages:?}");
            assert!(!captured, "{kind:?} {point:?}");
            assert!(state.focus.is_none(), "{kind:?} {point:?}");
        }
        // Without a proofreader, nothing answers.
        let content = Content::new(BackendKind::Owned, "cat teh");
        let mut editor = editor_with(&content, true, Measure::default());
        let mut state = laid_out_state(&editor);
        let (messages, _) = send(
            &mut editor,
            &mut state,
            press(mouse::Button::Right),
            Some(over(5.0, 0.0)),
        );
        assert!(messages.is_empty());
    }

    #[test]
    fn the_open_menus_word_is_tinted_on_each_of_its_rows() {
        let text = format!("ab {LONG_TYPO} cd teh");
        let menu = SpellMenu::new(0, 3..3 + LONG_TYPO.len(), LONG_TYPO.into(), Vec::new());
        let mut content = Content::new(BackendKind::Owned, &text);
        content.lay_out(Size::new(120.0, 400.0));
        let highlighter = MarkdownHighlighter::new(&fixtures::SETTINGS);
        let rows = visual_rows(&content.0.borrow(), &highlighter, None, None, Some(&menu));
        let tinted: Vec<(f32, f32)> = rows.iter().filter_map(|row| row.tint).collect();
        assert!(tinted.len() >= 3, "{rows:?}");
        // Exactly the word's glyphs: `teh` and the spaces stay clear.
        let width: f32 = tinted.iter().map(|(start, end)| end - start).sum();
        assert!(near(width, LONG_TYPO.len() as f32 * ADVANCE), "{width}");
        let other = SpellMenu::new(1, 3..3 + LONG_TYPO.len(), LONG_TYPO.into(), Vec::new());
        let rows = visual_rows(&content.0.borrow(), &highlighter, None, None, Some(&other));
        assert!(
            rows.iter().all(|row| row.tint.is_none()),
            "another line's word"
        );
    }

    fn key(key: keyboard::Key, modifiers: keyboard::Modifiers, text: Option<&str>) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            key,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        })
    }

    fn named(named: keyboard::key::Named) -> Event {
        key(
            keyboard::Key::Named(named),
            keyboard::Modifiers::empty(),
            None,
        )
    }

    fn moved_to(point: Point) -> Event {
        Event::Mouse(mouse::Event::CursorMoved { position: point })
    }

    fn spell_events(messages: &[Message]) -> Vec<SpellEvent> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::Spell(event) => Some(*event),
                _ => None,
            })
            .collect()
    }

    fn the_menu() -> SpellMenu {
        SpellMenu::new(0, 4..7, "teh".into(), vec!["the".into(), "ten".into()])
    }

    /// A focused widget over "cat teh" with `menu` open, after one event,
    /// so it has seen itself once.
    fn focused<'a>(
        content: &'a Content,
        proofreader: &'a dyn Proofreader,
        menu: &'a SpellMenu,
    ) -> (TextEditor<'a>, State) {
        let mut editor = spelling(content, proofreader, Some(menu));
        let mut state = laid_out_state(&editor);
        state.focus = Some(Focus::now());
        let (messages, _) = send(&mut editor, &mut state, moved_to(over(1.0, 0.0)), None);
        assert!(messages.is_empty(), "{messages:?}");
        (editor, state)
    }

    #[test]
    fn an_open_menu_takes_its_keys_and_leaves_escape_to_the_host() {
        use keyboard::key::Named;
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = the_menu();
        // Each key in a frame of its own, with the menu as the host left it.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let (messages, captured) = send(&mut editor, &mut state, named(Named::ArrowDown), None);
        assert_eq!(spell_events(&messages), [SpellEvent::Move(1)]);
        assert!(captured);
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let (messages, captured) = send(&mut editor, &mut state, named(Named::Enter), None);
        assert_eq!(spell_events(&messages), [SpellEvent::AcceptSelected]);
        assert!(captured);
        // A lone Shift: nothing, and the menu stays open.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let shift = key(
            keyboard::Key::Named(Named::Shift),
            keyboard::Modifiers::SHIFT,
            None,
        );
        let (messages, captured) = send(&mut editor, &mut state, shift, None);
        assert!(messages.is_empty() && !captured);
        assert!(editor.menu_now.get().is_some());
        // Escape: no message, not captured, and the editor keeps focus.
        let (messages, captured) = send(&mut editor, &mut state, named(Named::Escape), None);
        assert!(messages.is_empty() && !captured, "{messages:?}");
        assert!(state.focus.is_some());
    }

    #[test]
    fn another_key_closes_the_menu_first_and_is_then_handled_once() {
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = the_menu();
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let typed = key(
            keyboard::Key::Character("x".into()),
            keyboard::Modifiers::empty(),
            Some("x"),
        );
        let (messages, captured) = send(&mut editor, &mut state, typed, None);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Close),
                    Message::Action(Action::Edit(Edit::Insert('x')))
                ]
            ),
            "{messages:?}"
        );
        assert!(captured);
        // A key the editor leaves alone: closed, and not captured, so the
        // host still sees it.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let f5 = named(keyboard::key::Named::F5);
        let (messages, captured) = send(&mut editor, &mut state, f5, None);
        assert_eq!(spell_events(&messages), [SpellEvent::Close]);
        assert_eq!(messages.len(), 1);
        assert!(!captured);
    }

    #[test]
    fn a_press_in_the_editor_closes_the_menu_and_leaves_the_caret() {
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = the_menu();
        for button in [mouse::Button::Left, mouse::Button::Right] {
            let (mut editor, mut state) = focused(&content, &proofreader, &menu);
            let (messages, captured) =
                send(&mut editor, &mut state, press(button), Some(over(5.0, 0.0)));
            assert!(
                matches!(messages.as_slice(), [Message::Spell(SpellEvent::Close)]),
                "{button:?}: {messages:?}"
            );
            assert!(captured);
            assert!(state.drag_click.is_none() && state.focus.is_some());
        }
        // Outside the editor: closed, and the press does what it does
        // today, which drops the editor's focus.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let outside = Point::new(BOUNDS.width + 10.0, 30.0);
        let (messages, captured) = send(
            &mut editor,
            &mut state,
            press(mouse::Button::Left),
            Some(outside),
        );
        assert_eq!(spell_events(&messages), [SpellEvent::Close]);
        assert!(!captured);
        assert!(state.focus.is_none());
    }

    #[test]
    fn the_widget_closes_the_menu_on_every_change_it_sees() {
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = the_menu();
        let neutral = || moved_to(over(1.0, 0.0));

        // The window loses focus.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let (messages, _) = send(
            &mut editor,
            &mut state,
            Event::Window(window::Event::Unfocused),
            None,
        );
        assert_eq!(spell_events(&messages), [SpellEvent::Close], "window");

        // The editor loses focus, as when the host focuses another field.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        operation::Focusable::unfocus(&mut state);
        let (messages, _) = send(&mut editor, &mut state, neutral(), None);
        assert_eq!(spell_events(&messages), [SpellEvent::Close], "focus");

        // The editor is resized.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        let wider = Rectangle {
            width: BOUNDS.width + 1.0,
            ..BOUNDS
        };
        editor.on_event(
            &mut state,
            &neutral(),
            wider,
            mouse::Cursor::Unavailable,
            &mut clipboard::Null,
            &mut shell,
        );
        assert_eq!(spell_events(&messages), [SpellEvent::Close], "resized");

        // The theme, the font or the measure changes.
        let looks: [fn(TextEditor<'_>) -> TextEditor<'_>; 4] = [
            |mut editor| {
                editor.style.misspelling = iced::Color::WHITE;
                editor
            },
            |mut editor| {
                editor.highlighter_settings.palette = fixtures::LIGHT;
                editor
            },
            |mut editor| {
                editor.metrics = Metrics::for_size(18.0);
                editor
            },
            |editor| {
                editor.measure(Measure {
                    wrap_column: Some(80),
                    centred: true,
                })
            },
        ];
        for (i, change) in looks.into_iter().enumerate() {
            let (editor, mut state) = focused(&content, &proofreader, &menu);
            let mut editor = change(editor);
            let (messages, _) = send(&mut editor, &mut state, neutral(), None);
            assert_eq!(spell_events(&messages), [SpellEvent::Close], "look {i}");
        }

        // An input method starts composing.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let preedit = Event::InputMethod(input_method::Event::Preedit("に".into(), None));
        let (messages, _) = send(&mut editor, &mut state, preedit, None);
        assert_eq!(spell_events(&messages), [SpellEvent::Close], "composing");

        // Nothing changed: nothing closes.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let (messages, _) = send(&mut editor, &mut state, neutral(), None);
        assert!(messages.is_empty());
    }

    #[test]
    fn a_change_made_before_the_menu_opened_does_not_close_it() {
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = the_menu();
        let neutral = || moved_to(over(1.0, 0.0));
        fn changed(editor: TextEditor<'_>) -> TextEditor<'_> {
            let mut editor = editor.measure(Measure {
                wrap_column: Some(80),
                centred: true,
            });
            editor.style.misspelling = iced::Color::WHITE;
            editor.metrics = Metrics::for_size(18.0);
            editor
        }
        let wider = Size::new(BOUNDS.width + 1.0, BOUNDS.height);
        // The widget's last event saw no menu, then the theme, the text
        // size and the measure change and the host opens the menu, as the
        // palette and the harness do, before the widget's next event.
        let mut editor = spelling(&content, &proofreader, None);
        let mut state = laid_out_state(&editor);
        state.focus = Some(Focus::now());
        assert!(send(&mut editor, &mut state, neutral(), None).0.is_empty());
        let mut editor = changed(spelling(&content, &proofreader, Some(&menu)));
        editor.lay_out(&mut state, BOUNDS.size());
        let (messages, _) = send(&mut editor, &mut state, neutral(), None);
        assert!(messages.is_empty(), "{messages:?}");
        // A later change while it is open still closes it.
        let mut editor = spelling(&content, &proofreader, Some(&menu));
        editor.lay_out(&mut state, BOUNDS.size());
        let (messages, _) = send(&mut editor, &mut state, neutral(), None);
        assert_eq!(spell_events(&messages), [SpellEvent::Close]);

        // The same change, then the suggest key opening the menu itself.
        let content = Content::new(BackendKind::Owned, "cat teh");
        let policy = suggest_policy();
        let mut editor = spelling(&content, &proofreader, None).key_policy(Some(&policy));
        let mut state = laid_out_state(&editor);
        state.focus = Some(Focus::now());
        assert!(send(&mut editor, &mut state, neutral(), None).0.is_empty());
        let mut editor = changed(spelling(&content, &proofreader, None)).key_policy(Some(&policy));
        editor.lay_out(&mut state, wider);
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        content
            .0
            .borrow_mut()
            .perform(Action::Move(iced::widget::text_editor::Motion::DocumentEnd));
        editor.on_event(
            &mut state,
            &suggest_key(),
            Rectangle::new(BOUNDS.position(), wider),
            mouse::Cursor::Unavailable,
            &mut clipboard::Null,
            &mut shell,
        );
        assert_eq!(spell_events(&messages), [SpellEvent::OpenAtCaret]);
    }

    #[test]
    fn with_no_menu_open_the_widget_never_asks_to_close_one() {
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let mut editor = spelling(&content, &proofreader, None);
        let mut state = laid_out_state(&editor);
        state.focus = Some(Focus::now());
        for event in [
            moved_to(over(1.0, 0.0)),
            Event::Window(window::Event::Unfocused),
            press(mouse::Button::Left),
        ] {
            let (messages, _) = send(&mut editor, &mut state, event, Some(Point::new(900.0, 0.0)));
            assert!(spell_events(&messages).is_empty(), "{messages:?}");
        }
        // Escape is the editor's again: it drops focus.
        state.focus = Some(Focus::now());
        let (_, captured) = send(
            &mut editor,
            &mut state,
            named(keyboard::key::Named::Escape),
            None,
        );
        assert!(!captured);
        assert!(state.focus.is_none());
    }

    /// The open menu's bounds over `text` in a widget of `bounds`, its word
    /// at `mark` of `line`, with the content scrolled `scroll` rows.
    fn placed(
        text: &str,
        bounds: Rectangle,
        line: usize,
        mark: Range<usize>,
        items: &[&str],
        scroll: i32,
    ) -> Option<(Rectangle, Vec<String>)> {
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, text);
        let word = text.split('\n').nth(line).unwrap()[mark.clone()].to_owned();
        let menu = SpellMenu::new(
            line,
            mark,
            word,
            items.iter().map(|&item| item.to_owned()).collect(),
        );
        let editor = spelling(&content, &proofreader, Some(&menu));
        fixtures::load_font();
        let mut state = State::new(fixtures::SETTINGS);
        editor.lay_out(&mut state, bounds.size());
        if scroll != 0 {
            content
                .0
                .borrow_mut()
                .perform(Action::Scroll { lines: scroll });
            editor.lay_out(&mut state, bounds.size());
        }
        let overlay = editor.menu_overlay(&mut state, bounds)?;
        Some((overlay.bounds, overlay.labels.to_vec()))
    }

    #[test]
    fn the_menu_sits_under_its_words_first_row() {
        let bounds = Rectangle::new(Point::new(200.0, 40.0), Size::new(600.0, 400.0));
        let (menu, labels) = placed("cat teh", bounds, 0, 4..7, &["the", "ten"], 0).unwrap();
        // Text starts 60 px in and 24 px down; `teh` four cells along.
        assert_eq!(menu.x, (200.0 + 60.0 + 4.0 * ADVANCE).round());
        assert_eq!(menu.y, 40.0 + 24.0 + LINE_HEIGHT + 4.0);
        assert_eq!(menu.size(), Size::new(160.0, 68.0));
        assert_eq!(labels, ["the", "ten"]);

        // A word wrapped across rows: under its first row, which is the
        // second, since the word does not fit after `ab `; 160 px wide, it
        // is pulled left to stay 8 px inside a 200 px editor.
        let text = format!("ab {LONG_TYPO}");
        let narrow = Rectangle::new(Point::ORIGIN, Size::new(200.0, 400.0));
        let (menu, _) = placed(&text, narrow, 0, 3..3 + LONG_TYPO.len(), &[], 0).unwrap();
        assert_eq!(menu.x, 200.0 - 8.0 - 160.0, "{menu:?}");
        assert_eq!(menu.y, 24.0 + 2.0 * LINE_HEIGHT + 4.0);
        // With nothing to offer, one row.
        assert_eq!(menu.height, 38.0);
    }

    #[test]
    fn a_word_at_the_right_edge_or_on_the_last_row_keeps_the_menu_inside() {
        // The last visible row: 300 px tall, 24 px padding, 10 rows.
        let text: String = (0..20).map(|i| format!("line{i} teh\n")).collect();
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(600.0, 300.0));
        let (menu, _) = placed(&text, bounds, 10, 7..10, &["the"], 0).unwrap();
        let row_top = 24.0 + 10.0 * LINE_HEIGHT;
        assert_eq!(menu.y + menu.height, row_top - 4.0, "above: {menu:?}");

        // At the right edge of a wide line.
        let line = format!("{} teh", "a".repeat(55));
        let (menu, _) = placed(&line, bounds, 0, 56..59, &["the"], 0).unwrap();
        assert_eq!(menu.x + menu.width, 600.0 - 8.0);

        // An editor 150 px wide: the menu narrows to fit inside.
        let tiny = Rectangle::new(Point::ORIGIN, Size::new(150.0, 300.0));
        let (menu, labels) = placed(
            "teh",
            tiny,
            0,
            0..3,
            &["the", "a much longer suggestion"],
            0,
        )
        .unwrap();
        assert_eq!((menu.x, menu.width), (8.0, 134.0));
        assert!(labels[1].ends_with('…'), "{labels:?}");
    }

    #[test]
    fn a_word_out_of_view_has_no_menu_drawn() {
        let text: String = (0..40).map(|i| format!("line{i} teh\n")).collect();
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(600.0, 300.0));
        assert!(placed(&text, bounds, 0, 6..9, &["the"], 20).is_none());
        assert!(placed(&text, bounds, 0, 6..9, &["the"], 0).is_some());
    }

    // iced hands every event queued since the last frame to the same widget,
    // overlay first, before the host applies any message, so each test
    // below sends its events to one widget.

    fn three() -> SpellMenu {
        SpellMenu::new(
            0,
            4..7,
            "teh".into(),
            vec!["the".into(), "ten".into(), "tea".into()],
        )
    }

    fn typed(text: &str) -> Event {
        key(
            keyboard::Key::Character(text.into()),
            keyboard::Modifiers::empty(),
            Some(text),
        )
    }

    #[test]
    fn keys_in_one_batch_see_the_menu_as_the_earlier_keys_left_it() {
        use keyboard::key::Named;
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = three();
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        for (event, expected) in [
            (named(Named::ArrowDown), SpellEvent::Move(1)),
            (named(Named::ArrowDown), SpellEvent::Move(1)),
            // Stopped at the end, as the host's menu stops.
            (named(Named::ArrowDown), SpellEvent::Move(1)),
            (named(Named::ArrowUp), SpellEvent::Move(-1)),
            (named(Named::Enter), SpellEvent::AcceptSelected),
        ] {
            let (messages, _) = send(&mut editor, &mut state, event, None);
            assert_eq!(spell_events(&messages), [expected]);
        }
        // Accepting closed the menu: the next Enter is the text's again.
        let (messages, _) = send(&mut editor, &mut state, named(Named::Enter), None);
        assert!(
            matches!(messages.as_slice(), [Message::Intent(crate::Intent::Enter)]),
            "{messages:?}"
        );
    }

    #[test]
    fn a_key_that_closed_the_menu_leaves_the_next_key_to_the_text() {
        use keyboard::key::Named;
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = three();
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let (messages, _) = send(&mut editor, &mut state, typed("x"), None);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Close),
                    Message::Action(Action::Edit(Edit::Insert('x')))
                ]
            ),
            "{messages:?}"
        );
        for (event, arrow) in [
            (
                named(Named::ArrowDown),
                iced::widget::text_editor::Motion::Down,
            ),
            (named(Named::ArrowUp), iced::widget::text_editor::Motion::Up),
        ] {
            let (messages, _) = send(&mut editor, &mut state, event, None);
            assert!(
                matches!(messages.as_slice(), [Message::Action(Action::Move(motion))] if *motion == arrow),
                "{messages:?}"
            );
        }
        let (messages, _) = send(&mut editor, &mut state, named(Named::Enter), None);
        assert!(
            matches!(messages.as_slice(), [Message::Intent(crate::Intent::Enter)]),
            "{messages:?}"
        );
    }

    #[test]
    fn after_escape_the_next_key_is_the_texts_as_the_host_closes_the_menu() {
        use keyboard::key::Named;
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = three();
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let (messages, captured) = send(&mut editor, &mut state, named(Named::Escape), None);
        assert!(messages.is_empty() && !captured, "{messages:?}");
        assert!(state.focus.is_some());
        let (messages, _) = send(&mut editor, &mut state, named(Named::Enter), None);
        assert!(
            matches!(messages.as_slice(), [Message::Intent(crate::Intent::Enter)]),
            "{messages:?}"
        );
        // Nor is the menu drawn or handed the pointer any longer.
        assert!(editor.menu_overlay(&mut state, BOUNDS).is_none());
    }

    /// The middle of row `row` of the menu `editor` draws.
    fn menu_row(editor: &TextEditor<'_>, state: &mut State, row: usize) -> Point {
        let bounds = editor.menu_overlay(state, BOUNDS).unwrap().bounds;
        Point::new(bounds.center_x(), bounds.y + 4.0 + 30.0 * row as f32 + 15.0)
    }

    /// What `overlay` publishes for `event` with the pointer at `pointer`.
    fn to_menu(overlay: &MenuOverlay<'_>, event: Event, pointer: Point) -> Vec<Message> {
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        overlay.on_event(&event, mouse::Cursor::Available(pointer), &mut shell);
        messages
    }

    #[test]
    fn a_key_after_the_pointer_in_one_batch_sees_what_the_pointer_did() {
        use keyboard::key::Named;
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = three();

        // Hovering the third row, then Enter, accepts the third row.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let point = menu_row(&editor, &mut state, 2);
        {
            let overlay = editor.menu_overlay(&mut state, BOUNDS).unwrap();
            let messages = to_menu(&overlay, moved_to(point), point);
            assert_eq!(spell_events(&messages), [SpellEvent::Hover(2)]);
            // Over the row it now selects, nothing new.
            assert!(to_menu(&overlay, moved_to(point), point).is_empty());
        }
        let (messages, _) = send(&mut editor, &mut state, named(Named::Enter), None);
        assert_eq!(spell_events(&messages), [SpellEvent::AcceptSelected]);

        // A click that accepted leaves the next key to the text, and the
        // overlay, built once for the batch, takes no more of the pointer.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let (first, third) = (
            menu_row(&editor, &mut state, 1),
            menu_row(&editor, &mut state, 2),
        );
        {
            let overlay = editor.menu_overlay(&mut state, BOUNDS).unwrap();
            let messages = to_menu(&overlay, press(mouse::Button::Left), first);
            assert_eq!(spell_events(&messages), [SpellEvent::Accept(1)]);
            assert!(to_menu(&overlay, moved_to(third), third).is_empty());
            assert!(to_menu(&overlay, press(mouse::Button::Left), third).is_empty());
        }
        let (messages, _) = send(&mut editor, &mut state, named(Named::ArrowDown), None);
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Action(Action::Move(
                    iced::widget::text_editor::Motion::Down
                ))]
            ),
            "{messages:?}"
        );
        // Nor is it drawn again.
        assert!(editor.menu_overlay(&mut state, BOUNDS).is_none());
    }

    /// What one batch of `events` publishes as iced hands it over with the
    /// menu open: the overlay sees every event first, and the widget each
    /// one the overlay did not capture.
    fn over_open_menu(
        editor: &mut TextEditor<'_>,
        state: &mut State,
        events: Vec<(Event, Option<Point>)>,
    ) -> Vec<Message> {
        let mut published = Vec::new();
        let mut captured = Vec::new();
        {
            let overlay = editor.menu_overlay(state, BOUNDS).unwrap();
            for (event, pointer) in &events {
                let mut shell = Shell::new(&mut published);
                let cursor = pointer.map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available);
                overlay.on_event(event, cursor, &mut shell);
                captured.push(shell.event_status() == iced::event::Status::Captured);
            }
        }
        for ((event, pointer), captured) in events.into_iter().zip(captured) {
            if !captured {
                published.extend(send(editor, state, event, pointer).0);
            }
        }
        published
    }

    #[test]
    fn a_press_off_an_open_menu_comes_in_its_order_with_the_keys() {
        use keyboard::key::Named;
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let menu = three();
        // A key that closes the menu, then a press on `cat`: the press is
        // a click, as with no menu.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let messages = over_open_menu(
            &mut editor,
            &mut state,
            vec![
                (named(Named::ArrowLeft), None),
                (press(mouse::Button::Left), Some(over(1.0, 0.0))),
            ],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Close),
                    Message::Action(Action::Move(_)),
                    Message::Action(Action::Click(_)),
                ]
            ),
            "{messages:?}"
        );
        // The press first: it only closes the menu, and the key after it
        // is the text's.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let messages = over_open_menu(
            &mut editor,
            &mut state,
            vec![
                (press(mouse::Button::Left), Some(over(1.0, 0.0))),
                (named(Named::ArrowLeft), None),
            ],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Close),
                    Message::Action(Action::Move(_)),
                ]
            ),
            "{messages:?}"
        );
        // A press on a row is still the menu's, ahead of the keys.
        let (mut editor, mut state) = focused(&content, &proofreader, &menu);
        let row = menu_row(&editor, &mut state, 1);
        let messages = over_open_menu(
            &mut editor,
            &mut state,
            vec![
                (named(Named::ArrowDown), None),
                (press(mouse::Button::Left), Some(row)),
            ],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Accept(1)),
                    Message::Action(Action::Move(_)),
                ]
            ),
            "{messages:?}"
        );
    }

    #[test]
    fn keys_after_an_opening_in_one_batch_go_to_the_menu_it_opens() {
        use keyboard::key::Named;
        let proofreader = rejects();
        let content = Content::new(BackendKind::Owned, "cat teh");
        let mut editor = spelling(&content, &proofreader, None);
        let mut state = laid_out_state(&editor);
        let (messages, _) = send(
            &mut editor,
            &mut state,
            press(mouse::Button::Right),
            Some(over(5.0, 0.0)),
        );
        assert!(matches!(
            spell_events(&messages).as_slice(),
            [SpellEvent::OpenAt(_)]
        ));
        // The host resolves the menu's keys against what it opened.
        for (event, expected) in [
            (
                named(Named::ArrowDown),
                SpellEvent::Key(proof::MenuKey::Down),
            ),
            (named(Named::Enter), SpellEvent::Key(proof::MenuKey::Enter)),
        ] {
            let (messages, _) = send(&mut editor, &mut state, event, None);
            assert_eq!(spell_events(&messages), [expected]);
        }
    }

    /// An editor over `text` whose proofreader rejects `teh` and `zzz` and
    /// offers `the` and `ten` for `teh`, with the caret at `column`.
    fn host(text: &str, column: usize) -> crate::EditorView {
        let proofreader =
            fixtures::Rejects::new(&["teh", "zzz"]).suggesting(&[("teh", &["the", "ten"])]);
        let mut ev = crate::EditorView::with_backend(BackendKind::Owned, text);
        ev.set_proofreader(Some(Arc::new(proofreader)));
        ev.move_cursor(Cursor {
            position: Position { line: 0, column },
            selection: None,
        });
        ev
    }

    /// Alt+. bound to the suggest command.
    fn suggest_policy() -> KeyPolicy {
        Arc::new(|key: &keyboard::Key, _, modifiers: keyboard::Modifiers| {
            if modifiers == keyboard::Modifiers::ALT && *key == keyboard::Key::Character(".".into())
            {
                crate::KeyRoute::Suggest
            } else {
                crate::KeyRoute::Pass
            }
        })
    }

    fn suggest_key() -> Event {
        key(
            keyboard::Key::Character(".".into()),
            keyboard::Modifiers::ALT,
            None,
        )
    }

    /// Send `events` to one focused widget over `ev`, as iced hands over a
    /// frame's events, then carry out what it published as the host does.
    fn one_batch(ev: &mut crate::EditorView, events: Vec<Event>) -> Vec<Message> {
        one_batch_at(ev, events.into_iter().map(|event| (event, None)).collect())
    }

    /// [`one_batch`] with the pointer where each event names it.
    fn one_batch_at(
        ev: &mut crate::EditorView,
        events: Vec<(Event, Option<Point>)>,
    ) -> Vec<Message> {
        let mut state = None;
        batch_in(ev, &mut state, events)
    }

    /// [`one_batch_at`] on the tree state `state` holds, laid out afresh
    /// when it holds none, so that batches in turn see one widget's state.
    fn batch_in(
        ev: &mut crate::EditorView,
        state: &mut Option<State>,
        events: Vec<(Event, Option<Point>)>,
    ) -> Vec<Message> {
        let policy = suggest_policy();
        ev.set_key_policy(policy.clone());
        let messages: Vec<Message> = {
            let mut editor = editor_with(&ev.content, true, Measure::default())
                .key_policy(Some(&policy))
                .proofreader(ev.proofreader.as_deref())
                .spell_menu(ev.menu.as_ref())
                .resolved_press(ev.resolved_press);
            let state = state.get_or_insert_with(|| {
                let mut state = laid_out_state(&editor);
                state.focus = Some(Focus::now());
                state
            });
            events
                .into_iter()
                .flat_map(|(event, pointer)| send(&mut editor, state, event, pointer).0)
                .collect()
        };
        for message in messages.clone() {
            match message {
                Message::Action(action) => ev.perform(action),
                Message::Intent(intent) => ev.intent(intent),
                Message::Spell(event) => ev.spell(event),
                Message::Clip(clip) => ev.clip(clip),
                Message::Undo => ev.undo(),
                Message::Redo => ev.redo(),
                Message::Find => false,
            };
        }
        messages
    }

    #[test]
    fn keys_after_an_opening_that_finds_no_word_go_to_the_text() {
        use keyboard::key::Named;
        // The caret is on `cat`, which is spelled right.
        let mut ev = host("cat teh", 1);
        let messages = one_batch(
            &mut ev,
            vec![suggest_key(), named(Named::Enter), named(Named::ArrowDown)],
        );
        // The host, which opened nothing, handles Enter as a plain key, and
        // Enter leaves no menu either way, so Down is the text's.
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::OpenAtCaret),
                    Message::Spell(SpellEvent::Key(proof::MenuKey::Enter)),
                    Message::Action(Action::Move(iced::widget::text_editor::Motion::Down)),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "c\nat teh");
        assert_eq!(ev.cursor().position, Position { line: 1, column: 0 });
        assert!(ev.spell_menu().is_none());
        // A word at the caret still opens the menu for the keys after it.
        let mut ev = host("cat teh", 7);
        let messages = one_batch(&mut ev, vec![suggest_key(), named(Named::Enter)]);
        assert_eq!(
            spell_events(&messages),
            [
                SpellEvent::OpenAtCaret,
                SpellEvent::Key(proof::MenuKey::Enter)
            ]
        );
        assert_eq!(ev.text(), "cat the");
    }

    #[test]
    fn a_press_after_an_opening_in_one_batch_clicks_unless_a_menu_opened() {
        let left = || press(mouse::Button::Left);
        // The caret is on `cat`, which is spelled right, so the opening
        // finds nothing and the click moves the caret as it does alone.
        let mut alone = host("cat teh", 1);
        one_batch_at(&mut alone, vec![(left(), Some(over(5.0, 0.0)))]);
        let mut ev = host("cat teh", 1);
        let messages = one_batch_at(
            &mut ev,
            vec![(suggest_key(), None), (left(), Some(over(5.0, 0.0)))],
        );
        assert!(
            matches!(
                spell_events(&messages).as_slice(),
                [
                    SpellEvent::OpenAtCaret,
                    SpellEvent::Press(MenuPress::Left(_, mouse::click::Kind::Single))
                ]
            ),
            "{messages:?}"
        );
        assert_ne!(alone.cursor().position.column, 1);
        assert_eq!(ev.cursor(), alone.cursor());
        assert!(ev.spell_menu().is_none());

        // At the end of `teh` a menu opens, and the press only closes it:
        // neither it nor the drag it starts moves the caret (rule 4.4.5).
        let mut ev = host("cat teh", 7);
        one_batch_at(
            &mut ev,
            vec![
                (suggest_key(), None),
                (left(), Some(over(1.0, 0.0))),
                (moved_to(over(3.0, 0.0)), Some(over(3.0, 0.0))),
            ],
        );
        assert_eq!(ev.text(), "cat teh");
        assert_eq!(
            ev.cursor(),
            Cursor {
                position: Position { line: 0, column: 7 },
                selection: None
            }
        );
        assert!(ev.spell_menu().is_none());

        // A right press on `teh` after an opening that found nothing opens
        // the menu there.
        let mut ev = host("cat teh", 1);
        one_batch_at(
            &mut ev,
            vec![
                (suggest_key(), None),
                (press(mouse::Button::Right), Some(over(5.0, 0.0))),
            ],
        );
        assert_eq!(ev.spell_menu().map(SpellMenu::word), Some("teh"));
        assert_eq!(ev.cursor().position.column, 1);
    }

    #[test]
    fn a_press_that_only_closed_the_menu_starts_no_double_click() {
        let left = || (press(mouse::Button::Left), Some(over(1.0, 0.0)));
        let caret = |ev: &crate::EditorView| ev.cursor();
        let at = |column| Cursor {
            position: Position { line: 0, column },
            selection: None,
        };
        let word = |ev: &crate::EditorView| ev.selected_text();

        // The menu opens at the end of `teh` and the press only closes it,
        // so the press at the same point in the next batch is a click.
        let mut ev = host("cat teh", 7);
        let mut state = None;
        batch_in(&mut ev, &mut state, vec![(suggest_key(), None), left()]);
        assert_eq!(caret(&ev), at(7));
        let messages = batch_in(&mut ev, &mut state, vec![left()]);
        assert!(
            matches!(messages.as_slice(), [Message::Action(Action::Click(_))]),
            "{messages:?}"
        );
        assert_eq!(caret(&ev), at(1));
        // And the one after that a double click on `cat`.
        let messages = batch_in(&mut ev, &mut state, vec![left()]);
        assert!(
            matches!(messages.as_slice(), [Message::Action(Action::SelectWord)]),
            "{messages:?}"
        );
        assert_eq!(word(&ev).as_deref(), Some("cat"));

        // In one batch the same: close, click, then a double click.
        let mut ev = host("cat teh", 7);
        let mut state = None;
        batch_in(
            &mut ev,
            &mut state,
            vec![(suggest_key(), None), left(), left()],
        );
        assert_eq!(caret(&ev), at(1));
        batch_in(&mut ev, &mut state, vec![left()]);
        assert_eq!(word(&ev).as_deref(), Some("cat"));
        let mut ev = host("cat teh", 7);
        one_batch_at(&mut ev, vec![(suggest_key(), None), left(), left(), left()]);
        assert_eq!(word(&ev).as_deref(), Some("cat"));

        // An opening that finds nothing leaves the presses a double click.
        let mut ev = host("cat teh", 1);
        one_batch_at(&mut ev, vec![(suggest_key(), None), left(), left()]);
        assert_eq!(word(&ev).as_deref(), Some("cat"));

        // The press after the one that closed drags as a single click does.
        let mut ev = host("cat teh", 7);
        one_batch_at(
            &mut ev,
            vec![
                (suggest_key(), None),
                left(),
                left(),
                (moved_to(over(3.0, 0.0)), Some(over(3.0, 0.0))),
            ],
        );
        assert_eq!(word(&ev).as_deref(), Some("at"));

        // The widget lets every press it leaves to the host drag, so the
        // host drops the drag of a double click after an opening that found
        // nothing, which keeps the word selected.
        let mut ev = host("cat teh", 1);
        one_batch_at(
            &mut ev,
            vec![
                (suggest_key(), None),
                left(),
                left(),
                (moved_to(over(5.0, 0.0)), Some(over(5.0, 0.0))),
            ],
        );
        assert_eq!(word(&ev).as_deref(), Some("cat"));

        // A menu open when the batch starts: the press closes it, and the
        // press after it is a click, even right after a click there.
        let mut ev = host("cat teh", 7);
        let mut state = None;
        batch_in(&mut ev, &mut state, vec![left()]);
        batch_in(
            &mut ev,
            &mut state,
            vec![(press(mouse::Button::Right), Some(over(5.0, 0.0)))],
        );
        assert_eq!(ev.spell_menu().map(SpellMenu::word), Some("teh"));
        batch_in(&mut ev, &mut state, vec![left()]);
        assert!(ev.spell_menu().is_none());
        let messages = batch_in(&mut ev, &mut state, vec![left()]);
        assert!(
            matches!(messages.as_slice(), [Message::Action(Action::Click(_))]),
            "{messages:?}"
        );
        assert_eq!(caret(&ev), at(1));
    }

    #[test]
    fn a_suggest_press_after_a_caret_move_in_one_batch_opens_where_the_caret_went() {
        use keyboard::key::Named;
        // From the end of `teh` onto `line`, which is spelled right: Enter
        // breaks the line where the caret went, and nothing is lost.
        let mut ev = host("another teh line", 11);
        let messages = one_batch(
            &mut ev,
            vec![named(Named::ArrowRight), suggest_key(), named(Named::Enter)],
        );
        assert_eq!(
            spell_events(&messages),
            [
                SpellEvent::OpenAtCaret,
                SpellEvent::Key(proof::MenuKey::Enter)
            ]
        );
        assert_eq!(ev.text(), "another teh \nline");
        assert_eq!(ev.cursor().position, Position { line: 1, column: 0 });
        assert!(ev.spell_menu().is_none());
        // From `line` back to the end of `teh`: Enter accepts the first
        // suggestion and breaks no line.
        let mut ev = host("another teh line", 12);
        one_batch(
            &mut ev,
            vec![named(Named::ArrowLeft), suggest_key(), named(Named::Enter)],
        );
        assert_eq!(ev.text(), "another the line");
        assert_eq!(
            ev.cursor().position,
            Position {
                line: 0,
                column: 11
            }
        );
        assert!(ev.spell_menu().is_none());
        // Up and Down after the move reach the menu the host opened.
        let mut ev = host("another teh line", 12);
        one_batch(
            &mut ev,
            vec![
                named(Named::ArrowLeft),
                suggest_key(),
                named(Named::ArrowDown),
                named(Named::Enter),
            ],
        );
        assert_eq!(ev.text(), "another ten line");
        // And to the text when it opened nothing.
        let mut ev = host("another teh line\nnext", 11);
        one_batch(
            &mut ev,
            vec![
                named(Named::ArrowRight),
                suggest_key(),
                named(Named::ArrowDown),
            ],
        );
        assert_eq!(ev.text(), "another teh line\nnext");
        assert_eq!(ev.cursor().position.line, 1);
        assert!(ev.spell_menu().is_none());
    }

    #[test]
    fn enter_after_an_opening_in_one_batch_accepts_the_row_the_host_selected() {
        use keyboard::key::Named;
        let down = || named(Named::ArrowDown);
        // Two suggestions: Down stops at the second, as the host's menu does.
        let mut ev = host("cat teh", 7);
        one_batch(
            &mut ev,
            vec![suggest_key(), down(), down(), down(), named(Named::Enter)],
        );
        assert_eq!(ev.text(), "cat ten");
        let mut ev = host("cat teh", 7);
        one_batch(
            &mut ev,
            vec![
                suggest_key(),
                down(),
                down(),
                down(),
                named(Named::ArrowUp),
                named(Named::Enter),
            ],
        );
        assert_eq!(ev.text(), "cat the");
        // No suggestions: Enter closes the menu and the text stays.
        let mut ev = host("cat zzz", 7);
        let messages = one_batch(&mut ev, vec![suggest_key(), down(), named(Named::Enter)]);
        assert!(
            messages
                .iter()
                .all(|message| matches!(message, Message::Spell(_))),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "cat zzz");
        assert!(ev.spell_menu().is_none());
    }

    fn wheel(delta: mouse::ScrollDelta) -> Event {
        Event::Mouse(mouse::Event::WheelScrolled { delta })
    }

    fn pixels(y: f32) -> mouse::ScrollDelta {
        mouse::ScrollDelta::Pixels { x: 0.0, y }
    }

    /// The wheel events a trackpad sends around a two-finger click: fingers
    /// touching or lifting (no delta, from macOS as pixels or as lines), a
    /// sideways touch, and a slight drift of under one line.
    fn no_whole_line() -> [mouse::ScrollDelta; 5] {
        [
            pixels(0.0),
            mouse::ScrollDelta::Lines { x: 0.0, y: 0.0 },
            mouse::ScrollDelta::Pixels { x: 6.0, y: 0.0 },
            pixels(-3.0),
            pixels(3.0),
        ]
    }

    /// `delta` the other way, as a drift back.
    fn opposite(delta: mouse::ScrollDelta) -> mouse::ScrollDelta {
        match delta {
            mouse::ScrollDelta::Lines { x, y } => mouse::ScrollDelta::Lines { x: -x, y: -y },
            mouse::ScrollDelta::Pixels { x, y } => mouse::ScrollDelta::Pixels { x: -x, y: -y },
        }
    }

    #[test]
    fn wheel_events_of_no_whole_line_keep_a_menu_opened_by_a_right_click() {
        let on_word = Some(over(5.0, 0.0));
        let right = || (press(mouse::Button::Right), on_word);
        for delta in no_whole_line() {
            // In the batch after the click.
            let mut ev = host("cat teh", 1);
            let mut state = None;
            batch_in(&mut ev, &mut state, vec![right()]);
            assert_eq!(ev.spell_menu().map(SpellMenu::word), Some("teh"));
            let messages = batch_in(&mut ev, &mut state, vec![(wheel(delta), on_word)]);
            assert!(messages.is_empty(), "{delta:?}: {messages:?}");
            assert!(ev.spell_menu().is_some(), "{delta:?}");
            // Another, as when fingers touch and then lift.
            batch_in(&mut ev, &mut state, vec![(wheel(opposite(delta)), on_word)]);
            assert!(ev.spell_menu().is_some(), "{delta:?}");

            // In the click's own batch, before the menu is first drawn.
            let mut ev = host("cat teh", 1);
            let messages = one_batch_at(
                &mut ev,
                vec![
                    right(),
                    (wheel(delta), on_word),
                    (wheel(opposite(delta)), on_word),
                ],
            );
            assert!(
                matches!(messages.as_slice(), [Message::Spell(SpellEvent::OpenAt(_))]),
                "{delta:?}: {messages:?}"
            );
            assert!(ev.spell_menu().is_some(), "{delta:?}");
            assert_eq!(ev.cursor().position, Position { line: 0, column: 1 });
        }
    }

    #[test]
    fn a_scroll_left_over_from_before_the_right_click_does_not_close_its_menu() {
        let on_word = Some(over(5.0, 0.0));
        let mut ev = host("cat teh", 1);
        let mut state = None;
        // Three quarters of a line, kept for the next wheel event.
        let messages = batch_in(&mut ev, &mut state, vec![(wheel(pixels(-3.0)), on_word)]);
        assert!(messages.is_empty(), "{messages:?}");
        batch_in(
            &mut ev,
            &mut state,
            vec![(press(mouse::Button::Right), on_word)],
        );
        assert!(ev.spell_menu().is_some());
        // Half a line more would have made a whole one.
        let messages = batch_in(&mut ev, &mut state, vec![(wheel(pixels(-2.0)), on_word)]);
        assert!(messages.is_empty(), "{messages:?}");
        assert!(ev.spell_menu().is_some());
    }

    #[test]
    fn a_scroll_left_over_from_before_the_suggest_key_does_not_close_its_menu() {
        let on_word = Some(over(5.0, 0.0));
        let mut ev = host("cat teh", 5);
        let mut state = None;
        let messages = batch_in(&mut ev, &mut state, vec![(wheel(pixels(-3.0)), on_word)]);
        assert!(messages.is_empty(), "{messages:?}");
        batch_in(&mut ev, &mut state, vec![(suggest_key(), on_word)]);
        assert_eq!(ev.spell_menu().map(SpellMenu::word), Some("teh"));
        let messages = batch_in(&mut ev, &mut state, vec![(wheel(pixels(-1.0)), on_word)]);
        assert!(messages.is_empty(), "{messages:?}");
        assert!(ev.spell_menu().is_some());
    }

    #[test]
    fn a_scroll_of_a_whole_line_still_closes_the_menu() {
        let text = format!("cat teh\n{}", "line\n".repeat(60));
        let on_word = Some(over(5.0, 0.0));
        for (delta, lines) in [
            (pixels(-4.0), 1),
            (pixels(8.0), -2),
            (mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 }, 4),
        ] {
            let mut ev = host(&text, 1);
            let mut state = None;
            batch_in(
                &mut ev,
                &mut state,
                vec![(press(mouse::Button::Right), on_word)],
            );
            assert!(ev.spell_menu().is_some());
            let messages = batch_in(&mut ev, &mut state, vec![(wheel(delta), on_word)]);
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Action(Action::Scroll { lines: l })] if *l == lines
                ),
                "{delta:?}: {messages:?}"
            );
            assert!(ev.spell_menu().is_none(), "{delta:?}");
        }
        // Small steps that add up to a line close it on the step that does.
        let mut ev = host(&text, 1);
        let mut state = None;
        batch_in(
            &mut ev,
            &mut state,
            vec![(press(mouse::Button::Right), on_word)],
        );
        batch_in(&mut ev, &mut state, vec![(wheel(pixels(-2.0)), on_word)]);
        assert!(ev.spell_menu().is_some());
        batch_in(&mut ev, &mut state, vec![(wheel(pixels(-2.0)), on_word)]);
        assert!(ev.spell_menu().is_none());
    }

    #[test]
    fn wheel_events_add_up_to_whole_lines_and_are_always_taken() {
        for kind in [BackendKind::Iced, BackendKind::Owned] {
            let content = Content::new(kind, &"line\n".repeat(60));
            let mut editor = editor_with(&content, true, Measure::default());
            let mut state = laid_out_state(&editor);
            let mut scrolled = Vec::new();
            for delta in [
                pixels(0.0),
                pixels(-3.0),
                pixels(-3.0),
                pixels(-1.0),
                pixels(-1.0),
                mouse::ScrollDelta::Lines { x: 0.0, y: -0.1 },
                pixels(2.0),
                mouse::ScrollDelta::Lines { x: 0.0, y: 0.0 },
            ] {
                let (messages, captured) =
                    send(&mut editor, &mut state, wheel(delta), Some(over(1.0, 0.0)));
                assert!(captured, "{kind:?} {delta:?}");
                scrolled.extend(messages.into_iter().map(|message| match message {
                    Message::Action(Action::Scroll { lines }) => lines,
                    message => panic!("{kind:?}: {message:?}"),
                }));
            }
            // 0.75, then 1.5 (a line), 0.75, 1 (a line), a line delta,
            // which always moves at least one line, and -0.5 twice.
            assert_eq!(scrolled, [1, 1, 1], "{kind:?}");
        }
    }

    // ── The popover ────────────────────────────────────────────────────────

    use crate::fixtures::{status_rows, KEYBOARD, STATUS};
    use crate::{Anchor, Choice, Intent, MenuKey, Mode, Row, Stroke};

    const TASK: &str = "- [ ] call sam";

    fn plain_rows(labels: &[&str]) -> Vec<Row> {
        (0..)
            .zip(labels)
            .map(|(id, label)| Row::new(id, *label))
            .collect()
    }

    fn caret(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    /// An editor on `kind` over `text`, the caret at `(line, column)`.
    fn editing(kind: BackendKind, text: &str, line: usize, column: usize) -> crate::EditorView {
        let mut ev = crate::EditorView::with_backend(kind, text);
        ev.move_cursor(caret(line, column));
        ev
    }

    /// Hang `rows` under `range` of `line` in `mode`, on either engine: the
    /// host opens popovers on the owned one alone, and the widget shows and
    /// routes what it is handed on both.
    fn hang(
        ev: &mut crate::EditorView,
        mode: Mode,
        line: usize,
        range: Range<usize>,
        rows: Vec<Row>,
    ) {
        let text = ev.line_text(line).unwrap()[range.clone()].to_owned();
        ev.close_popover();
        ev.popover = Some(Popover::new(mode, line, range, text, rows));
    }

    /// An editor on `kind` over [`TASK`], the caret after `ca`, with `rows`
    /// hung under the box in `mode`.
    fn on_task(kind: BackendKind, mode: Mode, rows: Vec<Row>) -> crate::EditorView {
        let mut ev = editing(kind, TASK, 0, 8);
        hang(&mut ev, mode, 0, 2..5, rows);
        ev
    }

    /// The middle of row `row` of the popover [`on_task`] hangs: 160 px
    /// wide, 4 px under the first text row, at the box's left edge.
    fn on_task_row(row: usize) -> Point {
        Point::new(
            (60.0 + 2.0 * ADVANCE).round() + 80.0,
            24.0 + LINE_HEIGHT + 4.0 + 4.0 + 30.0 * row as f32 + 15.0,
        )
    }

    /// A request the test source of menus opens nothing for.
    const NOTHING: u32 = 9;

    /// Alt+S asks for [`STATUS`], Alt+N for [`NOTHING`] and Alt+K for
    /// [`KEYBOARD`].
    fn request_policy() -> KeyPolicy {
        Arc::new(
            |key: &keyboard::Key, _, modifiers: keyboard::Modifiers| match key.as_ref() {
                _ if modifiers != keyboard::Modifiers::ALT => crate::KeyRoute::Pass,
                keyboard::Key::Character("s") => crate::KeyRoute::Popover(STATUS),
                keyboard::Key::Character("n") => crate::KeyRoute::Popover(NOTHING),
                keyboard::Key::Character("k") => crate::KeyRoute::Popover(KEYBOARD),
                _ => crate::KeyRoute::Pass,
            },
        )
    }

    fn request_key(letter: &str) -> Event {
        key(
            keyboard::Key::Character(letter.into()),
            keyboard::Modifiers::ALT,
            None,
        )
    }

    /// An editor on `kind` over `text`, the caret at `(line, column)`,
    /// whose host answers the requests of [`request_policy`].
    fn requesting(kind: BackendKind, text: &str, line: usize, column: usize) -> crate::EditorView {
        let mut ev = editing(kind, text, line, column);
        ev.set_key_policy(request_policy());
        ev.set_menu_source(Some(Arc::new(fixtures::Menus::default())));
        ev
    }

    /// What a host did with a message: the row it chose, and whether the
    /// message was a stroke, which the host takes for a field of its own.
    #[derive(Debug, Default, PartialEq)]
    struct Applied {
        choice: Option<Choice>,
        stroke: Option<Stroke>,
    }

    /// Carry out `message` as a host does.
    fn host_applies(ev: &mut crate::EditorView, message: Message) -> Applied {
        let mut applied = Applied::default();
        match message {
            Message::Action(action) => ev.perform(action),
            Message::Intent(intent) => ev.intent(intent),
            Message::Spell(SpellEvent::Stroke(stroke)) => {
                applied.stroke = Some(stroke);
                false
            }
            Message::Spell(event) => {
                let changed = ev.spell(event);
                applied.choice = ev.take_choice();
                changed
            }
            Message::Clip(clip) => ev.clip(clip),
            Message::Undo => ev.undo(),
            Message::Redo => ev.redo(),
            Message::Find => false,
        };
        applied
    }

    /// One batch of `events` to a focused widget over `ev`, built as the
    /// editor's view builds it, as iced hands a frame's events over: the
    /// overlay sees each first, then the widget each one the overlay left,
    /// with no pointer where the overlay still holds it. Returns each
    /// message in the order it was published, with the event it was
    /// published for, and whether each event was captured.
    fn widget_events(
        ev: &crate::EditorView,
        state: &mut Option<State>,
        events: Vec<(Event, Option<Point>)>,
    ) -> (Vec<(usize, Message)>, Vec<bool>) {
        let mut out = Vec::new();
        let mut captured = vec![false; events.len()];
        let mut editor = editor_with(&ev.content, true, Measure::default())
            .key_policy(ev.key_policy.as_ref())
            .proofreader(ev.proofreader.as_deref())
            .popover(ev.popover())
            .menu_source(ev.menu_source())
            .suggest_source(ev.suggest_source(), ev.suggest_suppressed())
            .origin(&ev.origin)
            .resolved_press(ev.resolved_press);
        let state = state.get_or_insert_with(|| {
            let mut state = laid_out_state(&editor);
            state.focus = Some(Focus::now());
            state
        });
        editor.lay_out(state, BOUNDS.size());
        let mut held = vec![false; events.len()];
        let (focused, composing) = (state.focus.is_some(), state.is_composing());
        if let Some(overlay) = editor.menu_overlay(state, BOUNDS) {
            let ends = editor.ending(focused, composing);
            let mut overlay = popover::InOrder::new(overlay, ends);
            for (at, ((event, pointer), captured)) in events.iter().zip(&mut captured).enumerate() {
                let mut messages = Vec::new();
                let mut shell = Shell::new(&mut messages);
                let cursor = pointer.map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available);
                overlay.on_event(event, cursor, &mut shell);
                *captured = shell.event_status() == iced::event::Status::Captured;
                out.extend(messages.into_iter().map(|message| (at, message)));
            }
            for ((_, pointer), held) in events.iter().zip(&mut held) {
                *held = overlay.interaction(*pointer) != mouse::Interaction::None;
            }
        }
        for (at, (((event, pointer), captured), held)) in
            events.into_iter().zip(&mut captured).zip(held).enumerate()
        {
            if !*captured {
                let pointer = pointer.filter(|_| !held);
                let (published, taken) = send(&mut editor, state, event, pointer);
                out.extend(published.into_iter().map(|message| (at, message)));
                *captured = taken;
            }
        }
        (out, captured)
    }

    /// [`widget_events`], as the messages in the order they were published
    /// and the captures.
    fn widget_batch(
        ev: &crate::EditorView,
        state: &mut Option<State>,
        events: Vec<(Event, Option<Point>)>,
    ) -> (Vec<Message>, Vec<bool>) {
        let (messages, captured) = widget_events(ev, state, events);
        let messages = messages.into_iter().map(|(_, message)| message).collect();
        (messages, captured)
    }

    /// [`widget_batch`], then what it published carried out as the host
    /// does ([`host_applies`]). Returns the messages, whether each event
    /// was captured, and the row chosen.
    fn popover_batch(
        ev: &mut crate::EditorView,
        state: &mut Option<State>,
        events: Vec<(Event, Option<Point>)>,
    ) -> (Vec<Message>, Vec<bool>, Option<Choice>) {
        let (published, captured) = widget_batch(ev, state, events);
        let mut choice = None;
        for message in published.clone() {
            choice = host_applies(ev, message).choice.or(choice);
        }
        (published, captured, choice)
    }

    /// [`popover_batch`] of keys, on a fresh widget.
    fn keys_batch(
        ev: &mut crate::EditorView,
        events: Vec<Event>,
    ) -> (Vec<Message>, Vec<bool>, Option<Choice>) {
        let events = events.into_iter().map(|event| (event, None)).collect();
        popover_batch(ev, &mut None, events)
    }

    fn space() -> Event {
        key(
            keyboard::Key::Named(keyboard::key::Named::Space),
            keyboard::Modifiers::empty(),
            Some(" "),
        )
    }

    fn chose(id: u32, mode: Mode) -> Option<Choice> {
        Some(Choice {
            id,
            mode,
            line: 0,
            range: 2..5,
        })
    }

    #[test]
    fn a_menu_takes_up_down_enter_and_its_accelerators() {
        use keyboard::key::Named;
        for kind in BackendKind::ALL {
            // Keys of one batch see the menu as the earlier ones left it.
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let (messages, captured, choice) = keys_batch(
                &mut ev,
                vec![
                    named(Named::ArrowDown),
                    named(Named::ArrowDown),
                    named(Named::ArrowDown),
                    named(Named::ArrowUp),
                    named(Named::Enter),
                ],
            );
            assert_eq!(
                spell_events(&messages),
                [
                    SpellEvent::Move(1),
                    SpellEvent::Move(1),
                    SpellEvent::Move(1),
                    SpellEvent::Move(-1),
                    SpellEvent::AcceptSelected
                ],
                "{kind:?}"
            );
            assert_eq!(messages.len(), 5, "{kind:?}");
            assert_eq!(captured, [true; 5], "{kind:?}");
            assert_eq!(choice, chose(21, Mode::Menu), "{kind:?}");
            assert!(ev.popover().is_none(), "{kind:?}");

            // An accelerator chooses its row whatever is selected.
            for (event, id) in [(space(), 20), (typed("/"), 21), (typed("x"), 22)] {
                let mut ev = on_task(kind, Mode::Menu, status_rows());
                let (messages, captured, choice) =
                    keys_batch(&mut ev, vec![named(Named::ArrowDown), event]);
                assert_eq!(
                    spell_events(&messages)[1],
                    SpellEvent::Accept(id as usize - 20),
                    "{kind:?} {id}"
                );
                assert_eq!(captured, [true, true], "{kind:?} {id}");
                assert_eq!(choice, chose(id, Mode::Menu), "{kind:?} {id}");
                assert_eq!((ev.text().as_str(), ev.cursor()), (TASK, caret(0, 8)));
            }

            // Once it has chosen, the same key is the text's.
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let (messages, _, choice) = keys_batch(&mut ev, vec![typed("x"), typed("x")]);
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Spell(SpellEvent::Accept(2)),
                        Message::Action(Action::Edit(Edit::Insert('x')))
                    ]
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(choice, chose(22, Mode::Menu), "{kind:?}");
            assert_eq!(ev.text(), "- [ ] caxll sam", "{kind:?}");
        }
    }

    #[test]
    fn any_other_key_closes_a_menu_first_and_escape_is_sent_on_in_its_order() {
        use keyboard::key::Named;
        for kind in BackendKind::ALL {
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let (messages, captured, choice) = keys_batch(&mut ev, vec![typed("y"), typed("x")]);
            // The second key finds no menu, so it is no accelerator.
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Spell(SpellEvent::Close),
                        Message::Action(Action::Edit(Edit::Insert('y'))),
                        Message::Action(Action::Edit(Edit::Insert('x'))),
                    ]
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [true, true], "{kind:?}");
            assert_eq!(choice, None, "{kind:?}");
            assert_eq!(ev.text(), "- [ ] cayxll sam", "{kind:?}");

            // Escape on a menu of the host's goes to the host in its
            // order, uncaptured; the editor keeps the keyboard, and the
            // key after it is the text's.
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let mut state = None;
            let (messages, captured, _) = popover_batch(
                &mut ev,
                &mut state,
                vec![(named(Named::Escape), None), (typed("x"), None)],
            );
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Spell(SpellEvent::Key(MenuKey::Escape)),
                        Message::Action(Action::Edit(Edit::Insert('x')))
                    ]
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [false, true], "{kind:?}");
            assert!(state.unwrap().focus.is_some(), "{kind:?}");
            assert_eq!(ev.text(), "- [ ] caxll sam", "{kind:?}");
            // The host's own key handling hears of that press too, and
            // drops it, once.
            assert!(ev.take_claimed_escape(), "{kind:?}");
            assert!(!ev.take_claimed_escape(), "{kind:?}");

            // With Ctrl held it is no Escape of the menu's: any other key,
            // which closes the menu, and nothing is claimed.
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let control_escape = key(
                keyboard::Key::Named(Named::Escape),
                keyboard::Modifiers::CTRL,
                None,
            );
            let (messages, captured, _) =
                popover_batch(&mut ev, &mut None, vec![(control_escape, None)]);
            assert_eq!(spell_events(&messages), [SpellEvent::Close], "{kind:?}");
            assert_eq!(captured, [false], "{kind:?}");
            assert!(
                ev.popover().is_none() && !ev.take_claimed_escape(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_click_on_a_row_chooses_it_and_a_press_in_the_editor_only_closes_a_menu() {
        let left = || press(mouse::Button::Left);
        for kind in BackendKind::ALL {
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let row = on_task_row(1);
            let (messages, captured, choice) = popover_batch(
                &mut ev,
                &mut None,
                vec![(moved_to(row), Some(row)), (left(), Some(row))],
            );
            assert_eq!(
                spell_events(&messages),
                [SpellEvent::Hover(1), SpellEvent::Accept(1)],
                "{kind:?}"
            );
            assert_eq!(captured, [true, true], "{kind:?}");
            assert_eq!(choice, chose(21, Mode::Menu), "{kind:?}");
            assert_eq!(ev.cursor(), caret(0, 8), "{kind:?}");

            // Off the menu, on the text: the press closes it and the caret
            // stays; the next press is a click again.
            for button in [mouse::Button::Left, mouse::Button::Right] {
                let mut ev = on_task(kind, Mode::Menu, status_rows());
                let mut state = None;
                let on_text = Some(over(1.0, 0.0));
                let (messages, captured, choice) =
                    popover_batch(&mut ev, &mut state, vec![(press(button), on_text)]);
                assert!(
                    matches!(messages.as_slice(), [Message::Spell(SpellEvent::Close)]),
                    "{kind:?} {button:?}: {messages:?}"
                );
                assert_eq!(captured, [true], "{kind:?} {button:?}");
                assert_eq!((choice, ev.cursor()), (None, caret(0, 8)), "{kind:?}");
                assert!(ev.popover().is_none(), "{kind:?} {button:?}");
                popover_batch(&mut ev, &mut state, vec![(left(), on_text)]);
                assert_eq!(ev.cursor(), caret(0, 1), "{kind:?} {button:?}");
            }

            // Outside the editor the press closes it and goes on.
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let outside = Some(Point::new(700.0, 100.0));
            let (messages, captured, _) =
                popover_batch(&mut ev, &mut None, vec![(left(), outside)]);
            assert_eq!(spell_events(&messages), [SpellEvent::Close], "{kind:?}");
            assert_eq!(captured, [false], "{kind:?}");
            assert_eq!(ev.cursor(), caret(0, 8), "{kind:?}");
        }
    }

    #[test]
    fn suggestions_take_up_down_enter_and_tab_and_leave_the_rest_to_the_text() {
        use keyboard::key::Named;
        for kind in BackendKind::ALL {
            let suggestions = || on_task(kind, Mode::Suggest, plain_rows(&["a", "b", "c"]));
            // Their keys go to the host, which has them open.
            for accept in [Named::Enter, Named::Tab] {
                let mut ev = suggestions();
                let (messages, captured, choice) = keys_batch(
                    &mut ev,
                    vec![
                        named(Named::ArrowDown),
                        named(Named::ArrowDown),
                        named(Named::ArrowUp),
                        named(accept),
                    ],
                );
                assert_eq!(
                    spell_events(&messages),
                    [
                        SpellEvent::Key(proof::MenuKey::Down),
                        SpellEvent::Key(proof::MenuKey::Down),
                        SpellEvent::Key(proof::MenuKey::Up),
                        SpellEvent::Key(if accept == Named::Tab {
                            proof::MenuKey::Tab
                        } else {
                            proof::MenuKey::Enter
                        }),
                    ],
                    "{kind:?} {accept:?}"
                );
                assert_eq!(captured, [true; 4], "{kind:?} {accept:?}");
                assert_eq!(choice, chose(1, Mode::Suggest), "{kind:?} {accept:?}");
                assert_eq!((ev.text().as_str(), ev.cursor()), (TASK, caret(0, 8)));
            }

            // Typing and Backspace reach the text with no close before
            // them. With no source to ask, the host has nothing to offer
            // afterwards.
            let mut ev = suggestions();
            let (messages, captured, _) = keys_batch(&mut ev, vec![typed("x")]);
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Action(Action::Edit(Edit::Insert('x')))]
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [true], "{kind:?}");
            assert_eq!(ev.text(), "- [ ] caxll sam", "{kind:?}");
            assert!(ev.popover().is_none(), "{kind:?}");
            let mut ev = suggestions();
            let (messages, _, _) =
                keys_batch(&mut ev, vec![named(Named::Backspace), named(Named::Enter)]);
            // The Enter after it is the text's: nothing can have opened.
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Intent(Intent::Backspace),
                        Message::Intent(Intent::Enter)
                    ]
                ),
                "{kind:?}: {messages:?}"
            );

            // A motion is the text's too, and closes them at the host.
            let mut ev = suggestions();
            let (messages, _, _) = keys_batch(&mut ev, vec![named(Named::ArrowLeft)]);
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Action(Action::Move(
                        iced::widget::text_editor::Motion::Left
                    ))]
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(ev.cursor(), caret(0, 7), "{kind:?}");
            assert!(ev.popover().is_none(), "{kind:?}");
        }
    }

    #[test]
    fn suggestions_leave_presses_to_the_text_and_escape_to_the_host() {
        use keyboard::key::Named;
        let left = || press(mouse::Button::Left);
        for kind in BackendKind::ALL {
            let suggestions = || on_task(kind, Mode::Suggest, plain_rows(&["a", "b", "c"]));
            // A press in the editor is the click it is with nothing open.
            let mut ev = suggestions();
            let (messages, captured, _) =
                popover_batch(&mut ev, &mut None, vec![(left(), Some(over(1.0, 0.0)))]);
            assert!(
                matches!(messages.as_slice(), [Message::Action(Action::Click(_))]),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [true], "{kind:?}");
            assert_eq!(ev.cursor(), caret(0, 1), "{kind:?}");
            assert!(ev.popover().is_none(), "{kind:?}");

            // A row is still chosen with a click.
            let mut ev = suggestions();
            let row = on_task_row(2);
            let (messages, _, choice) =
                popover_batch(&mut ev, &mut None, vec![(left(), Some(row))]);
            assert_eq!(spell_events(&messages), [SpellEvent::Accept(2)], "{kind:?}");
            assert_eq!(choice, chose(2, Mode::Suggest), "{kind:?}");

            // Escape goes to the host in its order, uncaptured; the editor
            // keeps the keyboard, and the keys after it are the text's.
            let mut ev = suggestions();
            let mut state = None;
            let (messages, captured, _) = popover_batch(
                &mut ev,
                &mut state,
                vec![
                    (named(Named::Escape), None),
                    (named(Named::ArrowDown), None),
                ],
            );
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Spell(SpellEvent::Key(MenuKey::Escape)),
                        Message::Action(Action::Move(iced::widget::text_editor::Motion::Down))
                    ]
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [false, true], "{kind:?}");
            assert!(state.unwrap().focus.is_some(), "{kind:?}");
            assert!(ev.popover().is_none(), "{kind:?}");
            // The host's own key handling hears of that press too, and
            // drops it, once.
            assert!(ev.take_claimed_escape(), "{kind:?}");
            assert!(!ev.take_claimed_escape(), "{kind:?}");
        }
    }

    #[test]
    fn an_input_method_composing_keeps_its_keys_from_a_popover() {
        use keyboard::key::Named;
        for kind in BackendKind::ALL {
            for mode in [Mode::Menu, Mode::Suggest] {
                // The popover opened while a preedit was already under
                // way, so the preedit's start did not close it.
                let mut ev = on_task(kind, mode, status_rows());
                let mut editor =
                    editor_with(&ev.content, true, Measure::default()).popover(ev.popover());
                let mut state = laid_out_state(&editor);
                state.focus = Some(Focus::now());
                state.preedit = Some(input_method::Preedit {
                    content: "に".into(),
                    selection: None,
                    text_size: None,
                });
                // Enter and Tab are the input method's: no row is chosen.
                for named_key in [Named::Enter, Named::Tab] {
                    let (messages, captured) =
                        send(&mut editor, &mut state, named(named_key), None);
                    assert!(
                        messages.is_empty() && !captured,
                        "{kind:?} {mode:?} {named_key:?}"
                    );
                }
                // The arrows and a letter are the text's, a row's
                // accelerator included.
                let (messages, _) = send(&mut editor, &mut state, named(Named::ArrowDown), None);
                assert!(
                    spell_events(&messages).is_empty(),
                    "{kind:?} {mode:?}: {messages:?}"
                );
                drop(editor);
                let mut editor =
                    editor_with(&ev.content, true, Measure::default()).popover(ev.popover());
                let (messages, _) = send(&mut editor, &mut state, typed("x"), None);
                assert!(
                    matches!(
                        messages.as_slice(),
                        [Message::Action(Action::Edit(Edit::Insert('x')))]
                    ),
                    "{kind:?} {mode:?}: {messages:?}"
                );
                drop(editor);

                // A preedit that starts closes it.
                hang(&mut ev, mode, 0, 2..5, plain_rows(&["a"]));
                let preedit = Event::InputMethod(input_method::Event::Preedit("に".into(), None));
                let (messages, _, _) = popover_batch(&mut ev, &mut None, vec![(preedit, None)]);
                assert_eq!(
                    spell_events(&messages),
                    [SpellEvent::Close],
                    "{kind:?} {mode:?}"
                );
                assert!(ev.popover().is_none(), "{kind:?} {mode:?}");
            }
        }
    }

    /// An owned-engine editor over `text` with a source that completes
    /// `due`, `duplicate` and `today`, the caret at `(line, column)`.
    fn completing(text: &str, line: usize, column: usize) -> crate::EditorView {
        let mut ev = editing(BackendKind::Owned, text, line, column);
        ev.set_suggest_source(Some(Arc::new(fixtures::Completes::new(&[
            "due",
            "duplicate",
            "today",
        ]))));
        ev
    }

    fn suggested(id: u32, range: Range<usize>) -> Option<Choice> {
        Some(Choice {
            id,
            mode: Mode::Suggest,
            line: 0,
            range,
        })
    }

    #[test]
    fn keys_after_typing_in_one_batch_go_to_the_suggestions_it_opens() {
        use keyboard::key::Named;
        // `du` and the key that accepts, all in one batch: the widget asks
        // the source what `du` opens, as the host will, so the key is the
        // suggestions'.
        for (accept, key) in [
            (Named::Enter, proof::MenuKey::Enter),
            (Named::Tab, proof::MenuKey::Tab),
        ] {
            let mut ev = completing("", 0, 0);
            let (messages, captured, choice) =
                keys_batch(&mut ev, vec![typed("d"), typed("u"), named(accept)]);
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Action(Action::Edit(Edit::Insert('d'))),
                        Message::Action(Action::Edit(Edit::Insert('u'))),
                        Message::Spell(SpellEvent::Key(sent)),
                    ] if *sent == key
                ),
                "{accept:?}: {messages:?}"
            );
            assert_eq!(captured, [true; 3], "{accept:?}");
            assert_eq!(choice, suggested(0, 0..2), "{accept:?}");
            assert_eq!((ev.text().as_str(), ev.cursor()), ("du", caret(0, 2)));
            assert!(ev.popover().is_none(), "{accept:?}");
        }
        // Down first chooses the second row, and the keys after the choice
        // are the text's again.
        let mut ev = completing("", 0, 0);
        let (messages, _, choice) = keys_batch(
            &mut ev,
            vec![
                typed("d"),
                typed("u"),
                named(Named::ArrowDown),
                named(Named::Enter),
                named(Named::Enter),
            ],
        );
        assert!(
            matches!(
                &messages[2..],
                [
                    Message::Spell(SpellEvent::Key(proof::MenuKey::Down)),
                    Message::Spell(SpellEvent::Key(proof::MenuKey::Enter)),
                    Message::Intent(Intent::Enter),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(choice, suggested(1, 0..2));
        assert_eq!(ev.text(), "du\n");

        // They follow each letter of the batch: a third narrows them, and
        // a fourth that matches nothing closes them, so Enter is the
        // text's.
        let mut ev = completing("", 0, 0);
        let (messages, _, choice) = keys_batch(
            &mut ev,
            vec![typed("d"), typed("u"), typed("p"), named(Named::Tab)],
        );
        assert_eq!(spell_events(&messages), [SpellEvent::Key(MenuKey::Tab)]);
        assert_eq!(choice, suggested(1, 0..3));
        let mut ev = completing("", 0, 0);
        let (messages, _, choice) = keys_batch(
            &mut ev,
            vec![typed("d"), typed("u"), typed("x"), named(Named::Enter)],
        );
        assert!(
            matches!(messages.last(), Some(Message::Intent(Intent::Enter))),
            "{messages:?}"
        );
        assert_eq!((choice, ev.text().as_str()), (None, "dux\n"));
    }

    #[test]
    fn keys_after_typing_that_opens_nothing_are_the_texts() {
        use keyboard::key::Named;
        // One letter offers nothing, which the widget knows as the host
        // will: Tab and Enter are the text's, published as with no source.
        let mut ev = completing("- a\n- b", 1, 3);
        let (messages, captured, choice) = keys_batch(
            &mut ev,
            vec![typed("d"), named(Named::Tab), named(Named::Enter)],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Action(Action::Edit(Edit::Insert('d'))),
                    Message::Intent(Intent::Tab),
                    Message::Intent(Intent::Enter),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(captured, [true; 3]);
        assert_eq!(choice, None);
        assert_eq!(ev.text(), "- a\n  - bd\n  - ");

        // The same keys in an editor with no source.
        let mut ev = editing(BackendKind::Owned, "", 0, 0);
        let (messages, _, _) =
            keys_batch(&mut ev, vec![typed("d"), typed("u"), named(Named::Enter)]);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Action(Action::Edit(Edit::Insert('d'))),
                    Message::Action(Action::Edit(Edit::Insert('u'))),
                    Message::Intent(Intent::Enter),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "du\n");
    }

    #[test]
    fn after_a_key_the_widget_cannot_follow_a_popovers_keys_are_the_texts() {
        use keyboard::key::Named;
        // A deletion: what is left of the line is the editor's to work
        // out, so the widget opens nothing for the rest of the batch. The
        // host offers `due` again when it deletes, and closes it when Tab,
        // the text's, acts.
        let mut ev = completing("duex", 0, 4);
        let (messages, _, choice) =
            keys_batch(&mut ev, vec![named(Named::Backspace), named(Named::Tab)]);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Intent(Intent::Backspace),
                    Message::Intent(Intent::Tab),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(choice, None);
        assert!(ev.popover().is_none());
        assert!(
            ev.text().starts_with("due") && ev.text().len() > 3,
            "{:?}",
            ev.text()
        );
        // In a batch of its own, the same Tab finds them open.
        let mut ev = completing("duex", 0, 4);
        let mut state = None;
        popover_batch(&mut ev, &mut state, vec![(named(Named::Backspace), None)]);
        let (_, _, choice) = popover_batch(&mut ev, &mut state, vec![(named(Named::Tab), None)]);
        assert_eq!(choice, suggested(0, 0..3));

        // A motion, then letters that would open them where the caret was.
        let mut ev = completing("x ", 0, 2);
        let (messages, _, choice) = keys_batch(
            &mut ev,
            vec![
                named(Named::ArrowLeft),
                typed("d"),
                typed("u"),
                named(Named::Enter),
            ],
        );
        assert!(
            matches!(messages.last(), Some(Message::Intent(Intent::Enter))),
            "{messages:?}"
        );
        assert_eq!(choice, None);
        assert_eq!(ev.text(), "xdu\n ");
        // The same for a request: the host opens the menu, and the key
        // after it, the text's, closes it.
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, _, choice) = keys_batch(
            &mut ev,
            vec![named(Named::ArrowLeft), request_key("s"), typed("x")],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Action(Action::Move(_)),
                    Message::Spell(SpellEvent::Request(STATUS)),
                    Message::Action(Action::Edit(Edit::Insert('x'))),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(choice, None);
        assert!(ev.popover().is_none());
        assert_eq!(ev.text(), "- [ ] cxall sam");
    }

    #[test]
    fn suggestions_follow_the_typing_from_batch_to_batch() {
        use keyboard::key::Named;
        let mut ev = completing("pay ", 0, 4);
        let mut state = None;
        let keys = |ev: &mut crate::EditorView, state: &mut Option<State>, events: Vec<Event>| {
            let events = events.into_iter().map(|event| (event, None)).collect();
            popover_batch(ev, state, events)
        };
        let labels = |ev: &crate::EditorView| ev.popover().map(|popover| popover.labels().to_vec());
        keys(&mut ev, &mut state, vec![typed("d")]);
        assert_eq!(labels(&ev), None);
        keys(&mut ev, &mut state, vec![typed("u")]);
        assert_eq!(labels(&ev), Some(vec!["due".into(), "duplicate".into()]));
        // They hang under what was typed, and stay there as it grows.
        let hung = |ev: &crate::EditorView, state: &mut Option<State>| {
            let editor = editor_with(&ev.content, true, Measure::default()).popover(ev.popover());
            let state = state.as_mut().unwrap();
            editor.lay_out(state, BOUNDS.size());
            editor
                .menu_overlay(state, BOUNDS)
                .map(|overlay| (overlay.bounds.position(), overlay.labels.to_vec()))
        };
        let under_du = Point::new((60.0 + 4.0 * ADVANCE).round(), 24.0 + LINE_HEIGHT + 4.0);
        assert_eq!(
            hung(&ev, &mut state),
            Some((under_du, vec!["due".into(), "duplicate".into()]))
        );
        keys(&mut ev, &mut state, vec![typed("p")]);
        assert_eq!(
            hung(&ev, &mut state),
            Some((under_du, vec!["duplicate".into()]))
        );
        keys(&mut ev, &mut state, vec![named(Named::Backspace)]);
        assert_eq!(labels(&ev).map(|labels| labels.len()), Some(2));
        // Open when the batch starts, they take Down and Enter as before.
        let (messages, _, choice) = keys(
            &mut ev,
            &mut state,
            vec![named(Named::ArrowDown), named(Named::Enter)],
        );
        assert_eq!(
            spell_events(&messages),
            [
                SpellEvent::Key(proof::MenuKey::Down),
                SpellEvent::Key(proof::MenuKey::Enter)
            ]
        );
        assert_eq!(choice, suggested(1, 4..6));
        assert_eq!(ev.text(), "pay du");

        // Escape hides them for the line: typing on asks for nothing, and
        // Enter is the text's.
        let mut ev = completing("", 0, 0);
        let mut state = None;
        keys(&mut ev, &mut state, vec![typed("d"), typed("u")]);
        let (messages, captured, _) = keys(&mut ev, &mut state, vec![named(Named::Escape)]);
        assert_eq!(spell_events(&messages), [SpellEvent::Key(MenuKey::Escape)]);
        assert_eq!(captured, [false]);
        assert!(ev.popover().is_none());
        assert_eq!(ev.suggest_suppressed(), Some(0));
        keys(&mut ev, &mut state, vec![typed("e"), named(Named::Enter)]);
        assert_eq!(ev.text(), "due\n");
        // On the next line they are offered again.
        keys(&mut ev, &mut state, vec![typed("t"), typed("o")]);
        assert_eq!(labels(&ev), Some(vec!["today".into()]));
        assert_eq!(ev.suggest_suppressed(), None);
    }

    #[test]
    fn the_widget_closes_a_popover_of_either_mode_on_every_change_it_sees() {
        for kind in BackendKind::ALL {
            for mode in [Mode::Menu, Mode::Suggest] {
                let selection = Cursor {
                    position: Position { line: 0, column: 8 },
                    selection: Some(Position {
                        line: 0,
                        column: 11,
                    }),
                };
                let open = || {
                    let mut ev = on_task(kind, mode, plain_rows(&["a", "b"]));
                    // Under the popover, as opening leaves a selection.
                    ev.content.move_to(selection);
                    let mut state = None;
                    let neutral = (moved_to(over(1.0, 0.0)), None);
                    let (messages, _, _) = popover_batch(&mut ev, &mut state, vec![neutral]);
                    assert!(messages.is_empty(), "{kind:?} {mode:?}: {messages:?}");
                    (ev, state)
                };
                let closed = |name: &str, ev: &crate::EditorView, messages: &[Message]| {
                    assert_eq!(
                        spell_events(messages),
                        [SpellEvent::Close],
                        "{kind:?} {mode:?} {name}"
                    );
                    assert!(ev.popover().is_none(), "{kind:?} {mode:?} {name}");
                    assert_eq!(ev.cursor(), selection, "{kind:?} {mode:?} {name}");
                    assert_eq!(ev.text(), TASK, "{kind:?} {mode:?} {name}");
                };

                // The window loses focus.
                let (mut ev, mut state) = open();
                let unfocused = (Event::Window(window::Event::Unfocused), None);
                let (messages, _, _) = popover_batch(&mut ev, &mut state, vec![unfocused]);
                closed("window", &ev, &messages);

                // The editor loses focus, as when the host focuses another
                // field.
                let (mut ev, mut state) = open();
                operation::Focusable::unfocus(state.as_mut().unwrap());
                let neutral = (moved_to(over(1.0, 0.0)), None);
                let (messages, _, _) = popover_batch(&mut ev, &mut state, vec![neutral]);
                closed("focus", &ev, &messages);

                // The pointer is pressed outside the editor.
                let (mut ev, mut state) = open();
                let outside = (press(mouse::Button::Left), Some(Point::new(700.0, 100.0)));
                let (messages, _, _) = popover_batch(&mut ev, &mut state, vec![outside]);
                closed("a press outside", &ev, &messages);

                // The editor is resized, or its look changes.
                let (mut ev, state) = open();
                let mut state = state.unwrap();
                let mut messages = Vec::new();
                {
                    let mut editor =
                        editor_with(&ev.content, true, Measure::default()).popover(ev.popover());
                    let mut shell = Shell::new(&mut messages);
                    let wider = Rectangle {
                        width: BOUNDS.width + 1.0,
                        ..BOUNDS
                    };
                    editor.on_event(
                        &mut state,
                        &moved_to(over(1.0, 0.0)),
                        wider,
                        mouse::Cursor::Unavailable,
                        &mut clipboard::Null,
                        &mut shell,
                    );
                }
                for message in messages.clone() {
                    if let Message::Spell(event) = message {
                        ev.spell(event);
                    }
                }
                closed("resized", &ev, &messages);

                let (mut ev, state) = open();
                let mut state = state.unwrap();
                let messages = {
                    let mut editor =
                        editor_with(&ev.content, true, Measure::default()).popover(ev.popover());
                    editor.metrics = Metrics::for_size(18.0);
                    send(&mut editor, &mut state, moved_to(over(1.0, 0.0)), None).0
                };
                for message in messages.clone() {
                    if let Message::Spell(event) = message {
                        ev.spell(event);
                    }
                }
                closed("a new look", &ev, &messages);

                // A whole line scrolled closes it; less does not.
                let (mut ev, mut state) = open();
                let on_text = Some(over(1.0, 0.0));
                let (messages, _, _) =
                    popover_batch(&mut ev, &mut state, vec![(wheel(pixels(-2.0)), on_text)]);
                assert!(messages.is_empty(), "{kind:?} {mode:?}: {messages:?}");
                assert!(ev.popover().is_some(), "{kind:?} {mode:?}");
                let (messages, _, _) =
                    popover_batch(&mut ev, &mut state, vec![(wheel(pixels(-2.0)), on_text)]);
                assert!(
                    matches!(
                        messages.as_slice(),
                        [Message::Action(Action::Scroll { lines: 1 })]
                    ),
                    "{kind:?} {mode:?}: {messages:?}"
                );
                assert!(ev.popover().is_none(), "{kind:?} {mode:?}");
            }
        }
    }

    /// Where the popover of `labels` hangs over `text` in a widget of
    /// `bounds` on `kind`, anchored to `range` of `line`, with the content
    /// scrolled `scroll` rows; and where the host is told its anchor is.
    fn hung(
        kind: BackendKind,
        text: &str,
        bounds: Rectangle,
        (line, range): (usize, Range<usize>),
        labels: &[&str],
        scroll: i32,
    ) -> Option<(Rectangle, Rectangle)> {
        let mut ev = editing(kind, text, 0, 0);
        hang(&mut ev, Mode::Menu, line, range, plain_rows(labels));
        let editor = editor_with(&ev.content, true, Measure::default())
            .popover(ev.popover())
            .origin(&ev.origin);
        fixtures::load_font();
        let mut state = State::new(fixtures::SETTINGS);
        editor.lay_out(&mut state, bounds.size());
        if scroll != 0 {
            ev.content
                .0
                .borrow_mut()
                .perform(Action::Scroll { lines: scroll });
            editor.lay_out(&mut state, bounds.size());
        }
        let placed = editor.menu_overlay(&mut state, bounds)?.bounds;
        Some((placed, ev.popover_anchor().unwrap()))
    }

    fn rectangle(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(width, height))
    }

    fn near_rectangle(found: Rectangle, expected: Rectangle) -> bool {
        near(found.x, expected.x)
            && near(found.y, expected.y)
            && near(found.width, expected.width)
            && near(found.height, expected.height)
    }

    #[test]
    fn a_popover_hangs_under_its_anchor_at_the_anchors_left_edge() {
        let bounds = Rectangle::new(Point::new(200.0, 40.0), Size::new(600.0, 400.0));
        for kind in BackendKind::ALL {
            // A range: the box of a task. Text starts 60 px in and 24 px
            // down, and the box two cells along.
            let (placed, anchor) =
                hung(kind, TASK, bounds, (0, 2..5), &["a", "b", "c"], 0).unwrap();
            let left = 200.0 + 60.0 + 2.0 * ADVANCE;
            assert_eq!(
                placed,
                rectangle(left.round(), 40.0 + 24.0 + LINE_HEIGHT + 4.0, 160.0, 98.0),
                "{kind:?}"
            );
            // The host is told the same place: the anchor's glyphs on its
            // row, in the window.
            assert!(
                near_rectangle(anchor, rectangle(left, 64.0, 3.0 * ADVANCE, LINE_HEIGHT)),
                "{kind:?}: {anchor:?}"
            );
            assert_eq!(placed.y, anchor.y + anchor.height + 4.0, "{kind:?}");
            assert_eq!(placed.x, anchor.x.round(), "{kind:?}");

            // A place between two characters, as the caret is: no width.
            for (name, text, at, cells) in [
                ("inside a word", TASK, (0, 8..8), 8.0),
                ("before a space", TASK, (0, 5..5), 5.0),
                ("the line's start", TASK, (0, 0..0), 0.0),
                ("the line's end", TASK, (0, 14..14), 14.0),
                ("an empty line", "one\n\nthree", (1, 0..0), 0.0),
                ("the text's end", "one\n", (1, 0..0), 0.0),
            ] {
                let row = at.0 as f32;
                let (placed, anchor) = hung(kind, text, bounds, at, &["a"], 0).unwrap();
                let left = 260.0 + cells * ADVANCE;
                assert!(
                    near_rectangle(
                        anchor,
                        rectangle(left, 64.0 + row * LINE_HEIGHT, 0.0, LINE_HEIGHT)
                    ),
                    "{kind:?} {name}: {anchor:?}"
                );
                assert_eq!(placed.x, anchor.x.round(), "{kind:?} {name}");
                assert_eq!(placed.y, anchor.y + LINE_HEIGHT + 4.0, "{kind:?} {name}");
            }

            // Spaces alone have no glyphs of their own to span: the place
            // where they start.
            let (_, anchor) = hung(kind, "a   b", bounds, (0, 1..4), &["a"], 0).unwrap();
            assert!(
                near_rectangle(anchor, rectangle(260.0 + ADVANCE, 64.0, 0.0, LINE_HEIGHT)),
                "{kind:?}: {anchor:?}"
            );
        }
    }

    #[test]
    fn a_popover_goes_over_its_anchor_or_moves_left_to_stay_inside() {
        for kind in BackendKind::ALL {
            // The last visible row: 300 px tall, 24 px padding, 10 rows.
            let text: String = (0..20).map(|i| format!("line{i} box\n")).collect();
            let bounds = Rectangle::new(Point::ORIGIN, Size::new(600.0, 300.0));
            let (placed, anchor) = hung(kind, &text, bounds, (10, 7..10), &["a", "b"], 0).unwrap();
            assert_eq!(anchor.y, 24.0 + 10.0 * LINE_HEIGHT, "{kind:?}");
            assert_eq!(placed.y + placed.height, anchor.y - 4.0, "{kind:?}");
            // The row above it has room under it.
            let (placed, anchor) = hung(kind, &text, bounds, (4, 6..9), &["a", "b"], 0).unwrap();
            assert_eq!(placed.y, anchor.y + anchor.height + 4.0, "{kind:?}");

            // At the right edge of a wide line it stays 8 px inside.
            let line = format!("{} box", "a".repeat(55));
            let (placed, anchor) = hung(kind, &line, bounds, (0, 56..59), &["a"], 0).unwrap();
            assert_eq!(placed.x + placed.width, 600.0 - 8.0, "{kind:?}");
            assert!(
                near(anchor.x, 60.0 + 56.0 * ADVANCE),
                "{kind:?}: {anchor:?}"
            );

            // An anchor scrolled out of view has no popover drawn, and the
            // host is told no place for it.
            assert!(
                hung(kind, &text, bounds, (0, 6..9), &["a"], 12).is_none(),
                "{kind:?}"
            );
            let mut ev = editing(kind, &text, 0, 0);
            let editor = editor_with(&ev.content, true, Measure::default()).origin(&ev.origin);
            let mut state = laid_out_state(&editor);
            ev.content
                .0
                .borrow_mut()
                .perform(Action::Scroll { lines: 12 });
            editor.lay_out(&mut state, BOUNDS.size());
            assert!(editor.menu_overlay(&mut state, BOUNDS).is_none());
            let top = Anchor::Range {
                line: 0,
                range: 6..9,
            };
            assert_eq!(ev.anchor_bounds(&top), None, "{kind:?}");
            hang(&mut ev, Mode::Menu, 0, 6..9, plain_rows(&["a"]));
            assert_eq!(ev.popover_anchor(), None, "{kind:?}");
        }
    }

    #[test]
    fn on_a_wrapped_line_a_popover_hangs_under_its_anchors_own_row() {
        // 200 px wide: 116 px of text, thirteen cells a row, so the line
        // wraps before `cccc` and before `eeee`.
        let narrow = Rectangle::new(Point::ORIGIN, Size::new(200.0, 400.0));
        let text = "aaaa bbbb cccc dddd eeee ffff";
        let on_row = |row: f32, cells: f32, width: f32| {
            rectangle(
                60.0 + cells * ADVANCE,
                24.0 + row * LINE_HEIGHT,
                width * ADVANCE,
                LINE_HEIGHT,
            )
        };
        for kind in BackendKind::ALL {
            // `cccc` starts the second row.
            let (placed, anchor) = hung(kind, text, narrow, (0, 10..14), &["a"], 0).unwrap();
            assert!(
                near_rectangle(anchor, on_row(1.0, 0.0, 4.0)),
                "{kind:?}: {anchor:?}"
            );
            assert_eq!(placed.y, 24.0 + 2.0 * LINE_HEIGHT + 4.0, "{kind:?}");
            // 160 px wide, it is pulled left to stay 8 px inside.
            assert_eq!(placed.x, 200.0 - 8.0 - 160.0, "{kind:?}");

            // Places on the second row, at the start of the third, which
            // is the row of the character after it, and at the line's end.
            for (at, row, cells) in [(17, 1.0, 7.0), (20, 2.0, 0.0), (29, 2.0, 9.0)] {
                let (_, anchor) = hung(kind, text, narrow, (0, at..at), &["a"], 0).unwrap();
                assert!(
                    near_rectangle(anchor, on_row(row, cells, 0.0)),
                    "{kind:?} {at}: {anchor:?}"
                );
            }
            // A range across the wrap hangs under its first row.
            let (placed, anchor) = hung(kind, text, narrow, (0, 5..14), &["a"], 0).unwrap();
            assert!(
                near_rectangle(anchor, on_row(0.0, 5.0, 4.0)),
                "{kind:?}: {anchor:?}"
            );
            assert_eq!(placed.y, 24.0 + LINE_HEIGHT + 4.0, "{kind:?}");
        }
    }

    #[test]
    fn the_host_learns_where_any_anchor_is_in_the_window() {
        let bounds = Rectangle::new(Point::new(200.0, 40.0), Size::new(600.0, 400.0));
        for kind in BackendKind::ALL {
            // With no popover open: the caret, and a range.
            let ev = editing(kind, "one\n- [ ] call sam", 1, 8);
            assert_eq!(
                ev.anchor_bounds(&Anchor::Caret),
                None,
                "{kind:?}: not laid out"
            );
            let editor = editor_with(&ev.content, true, Measure::default()).origin(&ev.origin);
            fixtures::load_font();
            let mut state = State::new(fixtures::SETTINGS);
            editor.lay_out(&mut state, bounds.size());
            assert!(editor.menu_overlay(&mut state, bounds).is_none());
            let at_caret = ev.anchor_bounds(&Anchor::Caret).unwrap();
            assert!(
                near_rectangle(
                    at_caret,
                    rectangle(260.0 + 8.0 * ADVANCE, 64.0 + LINE_HEIGHT, 0.0, LINE_HEIGHT)
                ),
                "{kind:?}: {at_caret:?}"
            );
            let the_box = Anchor::Range {
                line: 1,
                range: 2..5,
            };
            let at_box = ev.anchor_bounds(&the_box).unwrap();
            assert!(
                near_rectangle(
                    at_box,
                    rectangle(
                        260.0 + 2.0 * ADVANCE,
                        64.0 + LINE_HEIGHT,
                        3.0 * ADVANCE,
                        LINE_HEIGHT
                    )
                ),
                "{kind:?}: {at_box:?}"
            );
            assert_eq!(ev.popover_anchor(), None, "{kind:?}: nothing open");
            // Off the text there is no place.
            for (line, range) in [(2, 0..0), (0, 2..9)] {
                let off = Anchor::Range { line, range };
                assert_eq!(ev.anchor_bounds(&off), None, "{kind:?}: {off:?}");
            }
            // Where the widget moves, the place follows.
            let moved = bounds + Vector::new(10.0, 5.0);
            assert!(editor.menu_overlay(&mut state, moved).is_none());
            let after = ev.anchor_bounds(&the_box).unwrap();
            assert_eq!(
                (after.x - at_box.x, after.y - at_box.y),
                (10.0, 5.0),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn only_the_suggestions_menu_tints_its_anchor() {
        let mut content = Content::new(BackendKind::Owned, TASK);
        content.lay_out(Size::new(400.0, 400.0));
        let highlighter = MarkdownHighlighter::new(&fixtures::SETTINGS);
        let rows = |popover: &Popover| {
            visual_rows(&content.0.borrow(), &highlighter, None, None, Some(popover))
        };
        let plain = Popover::new(Mode::Menu, 0, 2..5, "[ ]".into(), status_rows());
        assert!(rows(&plain).iter().all(|row| row.tint.is_none()));
        let spelling = plain.spelling();
        let tint = rows(&spelling)[0].tint.unwrap();
        assert!(near(tint.0, 2.0 * ADVANCE) && near(tint.1, 5.0 * ADVANCE));
    }

    // ── Events of one batch ────────────────────────────────────────────────

    #[test]
    fn escape_dismisses_suggestions_before_what_is_typed_after_it_in_one_batch() {
        use keyboard::key::Named;
        let escape_e_enter = || {
            vec![
                (named(Named::Escape), None),
                (typed("e"), None),
                (named(Named::Enter), None),
            ]
        };
        // Suggestions for `du` are open when the batch starts. Escape is
        // resolved before `e`, so `due` is not offered again and Enter is
        // the text's.
        let mut ev = completing("", 0, 0);
        let mut state = None;
        popover_batch(
            &mut ev,
            &mut state,
            vec![(typed("d"), None), (typed("u"), None)],
        );
        assert!(ev.popover().is_some());
        let (messages, captured, choice) = popover_batch(&mut ev, &mut state, escape_e_enter());
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Key(MenuKey::Escape)),
                    Message::Action(Action::Edit(Edit::Insert('e'))),
                    Message::Intent(Intent::Enter),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(captured, [false, true, true]);
        assert_eq!(choice, None);
        assert_eq!((ev.text().as_str(), ev.cursor()), ("due\n", caret(1, 0)));
        assert!(state.unwrap().focus.is_some());
        // The host's own key handling drops that Escape, once.
        assert!(ev.take_claimed_escape());
        assert!(!ev.take_claimed_escape());

        // The same with the typing that opened them in the batch too.
        let mut ev = completing("", 0, 0);
        let mut events = vec![(typed("d"), None), (typed("u"), None)];
        events.extend(escape_e_enter());
        let (messages, _, choice) = popover_batch(&mut ev, &mut None, events);
        assert_eq!(spell_events(&messages), [SpellEvent::Key(MenuKey::Escape)]);
        assert_eq!(choice, None);
        assert_eq!((ev.text().as_str(), ev.cursor()), ("due\n", caret(1, 0)));
        assert!(ev.take_claimed_escape());

        // Typing that opened nothing, which the widget knows: Escape is
        // the host's alone, as with no source. The editor publishes
        // nothing and drops focus, as it always has for it.
        let mut ev = completing("", 0, 0);
        let mut state = None;
        let (messages, captured, _) = popover_batch(
            &mut ev,
            &mut state,
            vec![(typed("d"), None), (named(Named::Escape), None)],
        );
        assert!(spell_events(&messages).is_empty(), "{messages:?}");
        assert_eq!(captured, [true, false]);
        assert!(!ev.take_claimed_escape());
        assert_eq!(ev.suggest_suppressed(), None);
        assert!(state.unwrap().focus.is_none());
    }

    /// The text, the caret, the row chosen and whether a popover is open
    /// after `batches` of keys go, each batch in turn, to an editor on
    /// `kind` over [`TASK`] whose host answers requests.
    fn after_batches(
        kind: BackendKind,
        batches: Vec<Vec<Event>>,
    ) -> (String, Cursor, Option<u32>, bool) {
        let mut ev = requesting(kind, TASK, 0, 8);
        let mut state = None;
        let mut chosen = None;
        for batch in batches {
            let events = batch.into_iter().map(|event| (event, None)).collect();
            let (_, _, choice) = popover_batch(&mut ev, &mut state, events);
            chosen = choice.map(|choice| choice.id).or(chosen);
        }
        (ev.text(), ev.cursor(), chosen, ev.popover().is_some())
    }

    #[test]
    fn keys_after_a_request_do_the_same_in_its_batch_as_in_the_next() {
        use keyboard::key::Named;
        let cases: [(&str, Vec<Event>, &str, Option<u32>); 7] = [
            // An accelerator of the menu chooses its row.
            ("an accelerator", vec![typed("x")], TASK, Some(22)),
            ("the space bar", vec![space()], TASK, Some(20)),
            // The menu's own keys drive it.
            (
                "down and enter",
                vec![named(Named::ArrowDown), named(Named::Enter)],
                TASK,
                Some(21),
            ),
            // Escape closes it, and the letter after it is the text's.
            (
                "escape and a letter",
                vec![named(Named::Escape), typed("x")],
                "- [ ] caxll sam",
                None,
            ),
            // Any other key closes it and is the text's.
            ("another letter", vec![typed("y")], "- [ ] cayll sam", None),
            (
                "a motion and a letter",
                vec![named(Named::ArrowLeft), typed("x")],
                "- [ ] cxall sam",
                None,
            ),
            (
                "a deletion",
                vec![named(Named::Backspace)],
                "- [ ] cll sam",
                None,
            ),
        ];
        for (name, keys, text, chosen) in cases {
            let mut one: Vec<Event> = vec![request_key("s")];
            one.extend(keys.iter().cloned());
            let together = after_batches(BackendKind::Owned, vec![one]);
            let apart = after_batches(BackendKind::Owned, vec![vec![request_key("s")], keys]);
            assert_eq!(together, apart, "{name}");
            assert_eq!(
                (together.0.as_str(), together.2, together.3),
                (text, chosen, false),
                "{name}"
            );
        }
        // A request the host opens nothing for leaves each key to the
        // text, carried out once, as with no request; and so does every
        // request on the iced engine, which opens no popover.
        let keys = || {
            vec![
                typed("x"),
                named(Named::Enter),
                named(Named::ArrowUp),
                typed("y"),
            ]
        };
        for (kind, letter) in [
            (BackendKind::Owned, "n"),
            (BackendKind::Iced, "n"),
            (BackendKind::Iced, "s"),
        ] {
            let mut one = vec![request_key(letter)];
            one.extend(keys());
            let together = after_batches(kind, vec![one]);
            let alone = after_batches(kind, vec![keys()]);
            assert_eq!(together, alone, "{kind:?} {letter}");
            assert_eq!(together.2, None, "{kind:?} {letter}");
        }
    }

    #[test]
    fn a_request_opens_the_menu_for_the_keys_after_it_in_its_batch() {
        use keyboard::key::Named;
        // The widget asks the host's source what the request opens, so the
        // keys after it are routed against the menu's rows: published as a
        // menu's own events, not left for the host to make out.
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, captured, choice) = keys_batch(
            &mut ev,
            vec![request_key("s"), named(Named::ArrowDown), typed("x")],
        );
        assert_eq!(
            spell_events(&messages),
            [
                SpellEvent::Request(STATUS),
                SpellEvent::Move(1),
                SpellEvent::Accept(2)
            ]
        );
        assert_eq!(captured, [true; 3]);
        assert_eq!(choice, chose(22, Mode::Menu));
        assert_eq!((ev.text().as_str(), ev.cursor()), (TASK, caret(0, 8)));

        // After letters typed in the batch, it is asked about the line
        // they leave, and the host opens the same menu there.
        let mut ev = requesting(BackendKind::Owned, "- [ ] ", 0, 6);
        let (messages, _, choice) = keys_batch(
            &mut ev,
            vec![typed("a"), typed("b"), request_key("s"), typed("/")],
        );
        assert_eq!(
            spell_events(&messages),
            [SpellEvent::Request(STATUS), SpellEvent::Accept(1)]
        );
        assert_eq!(choice, chose(21, Mode::Menu));
        assert_eq!(ev.text(), "- [ ] ab");

        // A press in the editor only closes the menu, and Escape is sent
        // on in its order and claimed.
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, _, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (request_key("s"), None),
                (press(mouse::Button::Left), Some(over(1.0, 0.0))),
            ],
        );
        assert_eq!(
            spell_events(&messages),
            [SpellEvent::Request(STATUS), SpellEvent::Close]
        );
        assert_eq!(messages.len(), 2);
        assert!(ev.popover().is_none());
        assert_eq!(ev.cursor(), caret(0, 8));
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, captured, _) = keys_batch(
            &mut ev,
            vec![request_key("s"), named(Named::Escape), typed("x")],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Request(STATUS)),
                    Message::Spell(SpellEvent::Key(MenuKey::Escape)),
                    Message::Action(Action::Edit(Edit::Insert('x')))
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(captured, [true, false, true]);
        assert!(ev.take_claimed_escape() && !ev.take_claimed_escape());
        assert_eq!(ev.text(), "- [ ] caxll sam");

        // A request that opens nothing: the press is the click it is, and
        // Escape the host's alone.
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        popover_batch(
            &mut ev,
            &mut None,
            vec![
                (request_key("n"), None),
                (press(mouse::Button::Left), Some(over(1.0, 0.0))),
            ],
        );
        assert_eq!(ev.cursor(), caret(0, 1));
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, captured, _) =
            keys_batch(&mut ev, vec![request_key("n"), named(Named::Escape)]);
        assert_eq!(spell_events(&messages), [SpellEvent::Request(NOTHING)]);
        assert_eq!(captured, [true, false]);
        assert!(!ev.take_claimed_escape());
    }

    #[test]
    fn a_host_that_takes_the_keyboard_is_handed_the_keys_after_its_request() {
        use keyboard::key::Named;
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let events: Vec<(Event, Option<Point>)> = [
            request_key("k"),
            typed("f"),
            typed("r"),
            named(Named::Backspace),
            named(Named::ArrowDown),
            named(Named::Enter),
        ]
        .into_iter()
        .map(|event| (event, None))
        .collect();
        let (messages, captured) = widget_batch(&ev, &mut None, events);
        assert_eq!(captured, [true; 6]);
        assert!(matches!(
            messages[0],
            Message::Spell(SpellEvent::Request(KEYBOARD))
        ));
        // Each key in its order, with what it types, for the field the
        // host opened; the editor does nothing with any of them.
        let strokes: Vec<Stroke> = messages[1..]
            .iter()
            .map(|message| host_applies(&mut ev, message.clone()).stroke.unwrap())
            .collect();
        assert_eq!(
            strokes
                .iter()
                .map(|stroke| (stroke.text(), stroke.named()))
                .collect::<Vec<_>>(),
            [
                (Some('f'), None),
                (Some('r'), None),
                (None, Some(Named::Backspace)),
                (None, Some(Named::ArrowDown)),
                (None, Some(Named::Enter)),
            ]
        );
        assert_eq!((ev.text().as_str(), ev.cursor()), (TASK, caret(0, 8)));
        assert!(ev.popover().is_none());

        // Nothing of the editor's is open: Escape is the host's alone, and
        // a press in the editor is the click it is, after which the keys
        // are the text's again.
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, captured, _) =
            keys_batch(&mut ev, vec![request_key("k"), named(Named::Escape)]);
        assert_eq!(spell_events(&messages), [SpellEvent::Request(KEYBOARD)]);
        assert_eq!(captured, [true, false]);
        assert!(!ev.take_claimed_escape());
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, _, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (request_key("k"), None),
                (press(mouse::Button::Left), Some(over(1.0, 0.0))),
                (typed("x"), None),
            ],
        );
        assert!(
            matches!(
                &messages[1..],
                [
                    Message::Action(Action::Click(_)),
                    Message::Action(Action::Edit(Edit::Insert('x')))
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "-x [ ] call sam");
    }

    #[test]
    fn a_letter_after_the_suggest_key_in_its_batch_closes_the_menu_and_types() {
        // As the suggestions menu was built: the letter closes the menu
        // the request may have opened, and is typed.
        let mut ev = host("cat teh", 7);
        let mut state = None;
        let messages = batch_in(
            &mut ev,
            &mut state,
            vec![(suggest_key(), None), (typed("x"), None)],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::OpenAtCaret),
                    Message::Spell(SpellEvent::Close),
                    Message::Action(Action::Edit(Edit::Insert('x'))),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!((ev.text().as_str(), ev.cursor()), ("cat tehx", caret(0, 8)));
        assert!(ev.spell_menu().is_none());
    }

    /// The suggestions menu open on `teh` of "cat teh", with two rows.
    fn spelled_menu() -> crate::EditorView {
        let mut ev = host("cat teh", 7);
        ev.spell(SpellEvent::OpenAtCaret);
        assert_eq!(
            ev.spell_menu().map(SpellMenu::items),
            Some(&["the".to_owned(), "ten".to_owned()][..])
        );
        ev
    }

    /// The middle of row `row` of the menu [`spelled_menu`] opens.
    fn on_spelled_row(row: usize) -> Point {
        Point::new(
            (60.0 + 4.0 * ADVANCE).round() + 80.0,
            24.0 + LINE_HEIGHT + 4.0 + 4.0 + 30.0 * row as f32 + 15.0,
        )
    }

    #[test]
    fn a_key_that_ends_a_popover_comes_before_a_later_press_on_a_row() {
        use keyboard::key::Named;
        let left = || press(mouse::Button::Left);
        // Enter, then a press on the second row: Enter accepts the first,
        // which is selected, and the press finds no menu.
        let mut ev = spelled_menu();
        let (messages, captured, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (named(Named::Enter), None),
                (left(), Some(on_spelled_row(1))),
            ],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::AcceptSelected),
                    Message::Action(Action::Click(_)),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(captured, [true, true]);
        assert_eq!(ev.text(), "cat the");
        assert!(ev.spell_menu().is_none());

        // A key that closes the menu, then the press: nothing is accepted.
        let mut ev = spelled_menu();
        let (messages, _, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![(typed("x"), None), (left(), Some(on_spelled_row(1)))],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Close),
                    Message::Action(Action::Edit(Edit::Insert('x'))),
                    Message::Action(Action::Click(_)),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "cat tehx");

        // The press first is the menu's, and the key after it the text's.
        let mut ev = spelled_menu();
        let (messages, _, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (left(), Some(on_spelled_row(1))),
                (named(Named::Enter), None),
            ],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Accept(1)),
                    Message::Intent(Intent::Enter),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "cat ten\n");

        // Up and Down leave the rows as they are drawn: a press on a row
        // after one still chooses that row, ahead of the key.
        let mut ev = spelled_menu();
        let (messages, _, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (named(Named::ArrowDown), None),
                (left(), Some(on_spelled_row(1))),
            ],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Accept(1)),
                    Message::Action(Action::Move(_)),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "cat ten");

        // The host's menus and suggestions alike.
        for kind in BackendKind::ALL {
            for (mode, ending) in [
                (Mode::Menu, typed("y")),
                (Mode::Menu, named(Named::Enter)),
                (Mode::Menu, named(Named::Escape)),
                (Mode::Suggest, named(Named::Tab)),
                (Mode::Suggest, named(Named::Escape)),
                (Mode::Suggest, typed("x")),
            ] {
                let mut ev = on_task(kind, mode, status_rows());
                let (_, _, choice) = popover_batch(
                    &mut ev,
                    &mut None,
                    vec![(ending.clone(), None), (left(), Some(on_task_row(2)))],
                );
                // Never the row pressed: the first, or none.
                assert_ne!(
                    choice.map(|choice| choice.id),
                    Some(22),
                    "{kind:?} {mode:?} {ending:?}"
                );
                assert!(ev.popover().is_none(), "{kind:?} {mode:?} {ending:?}");
            }
        }
    }

    #[test]
    fn an_event_that_leaves_the_popover_as_it_is_leaves_its_rows_the_pointer() {
        use keyboard::key::Named;
        let left = || press(mouse::Button::Left);
        let accepts = |ev: &mut crate::EditorView, before: Event, row: Point, name: &str| {
            let (messages, _, _) =
                popover_batch(ev, &mut None, vec![(before, None), (left(), Some(row))]);
            assert!(
                matches!(messages.last(), Some(Message::Spell(SpellEvent::Accept(1)))),
                "{name}: {messages:?}"
            );
        };
        // An input method that opens, or reports an empty preedit, starts
        // no composition: the menu stays, and the row pressed is accepted.
        for (name, before) in [
            (
                "an empty preedit",
                Event::InputMethod(input_method::Event::Preedit(String::new(), None)),
            ),
            (
                "an input method opening",
                Event::InputMethod(input_method::Event::Opened),
            ),
            (
                "a lone modifier",
                key(
                    keyboard::Key::Named(Named::Shift),
                    keyboard::Modifiers::SHIFT,
                    None,
                ),
            ),
        ] {
            let mut ev = spelled_menu();
            accepts(&mut ev, before, on_spelled_row(1), name);
            assert_eq!(ev.text(), "cat ten", "{name}");
        }
        // A key the editor ignores passes suggestions by, and a key of a
        // composition is none of a popover's.
        for kind in BackendKind::ALL {
            let mut ev = on_task(kind, Mode::Suggest, status_rows());
            accepts(&mut ev, named(Named::F5), on_task_row(1), "an ignored key");
        }
        // A preedit that starts a composition closes the menu: the press
        // after it is the editor's.
        let mut ev = spelled_menu();
        let (messages, _, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (
                    Event::InputMethod(input_method::Event::Preedit("に".into(), None)),
                    None,
                ),
                (left(), Some(on_spelled_row(1))),
            ],
        );
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, Message::Spell(SpellEvent::Accept(_)))),
            "{messages:?}"
        );
        assert_eq!(ev.text(), "cat teh");
    }

    #[test]
    fn a_key_pressed_while_an_input_method_composes_opens_no_suggestions() {
        use keyboard::key::Named;
        let source = Arc::new(fixtures::Completes::new(&["due", "duplicate"]));
        let mut ev = editing(BackendKind::Owned, "d", 0, 1);
        ev.set_suggest_source(Some(source.clone()));
        fn widget<'a>(
            ev: &'a crate::EditorView,
            source: &'a fixtures::Completes,
        ) -> (TextEditor<'a>, State) {
            let editor = editor_with(&ev.content, true, Measure::default())
                .suggest_source(Some(source), None);
            let mut state = laid_out_state(&editor);
            state.focus = Some(Focus::now());
            state.preedit = Some(input_method::Preedit {
                content: "に".into(),
                selection: None,
                text_size: None,
            });
            (editor, state)
        }
        // The letter completes `du`, and the source would offer for it.
        let (messages, captured) = {
            let (mut editor, mut state) = widget(&ev, &source);
            let (messages, captured) = send(&mut editor, &mut state, typed("u"), None);
            // Nor does the widget take the keys after it for suggestions'.
            assert_eq!(editor.menu_now.live(), None);
            let (after, taken) = send(&mut editor, &mut state, named(Named::ArrowDown), None);
            assert!(spell_events(&after).is_empty() && taken, "{after:?}");
            for named_key in [Named::Enter, Named::Tab] {
                let (after, taken) = send(&mut editor, &mut state, named(named_key), None);
                assert!(after.is_empty() && !taken, "{named_key:?}: {after:?}");
            }
            (messages, captured)
        };
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Hold),
                    Message::Action(Action::Edit(Edit::Insert('u'))),
                ]
            ),
            "{messages:?}"
        );
        assert!(captured);
        for message in messages {
            host_applies(&mut ev, message);
        }
        assert_eq!(ev.text(), "du");
        assert!(ev.popover().is_none());
        assert!(source.asked().is_empty());

        // The composed text, committed, asks nothing either.
        let messages = {
            let (mut editor, mut state) = widget(&ev, &source);
            let commit = Event::InputMethod(input_method::Event::Commit("e".into()));
            send(&mut editor, &mut state, commit, None).0
        };
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Action(Action::Edit(Edit::Paste(_)))]
            ),
            "{messages:?}"
        );
        for message in messages {
            host_applies(&mut ev, message);
        }
        assert_eq!(ev.text(), "due");
        assert!(ev.popover().is_none());
        assert!(source.asked().is_empty());

        // The same letter with no composition is asked about, by the
        // widget and then the editor; and with no source nothing is held.
        ev.replace_text("d", false);
        ev.move_cursor(caret(0, 1));
        popover_batch(&mut ev, &mut None, vec![(typed("u"), None)]);
        assert!(ev.popover().is_some());
        assert_eq!(source.asked(), [("du".to_owned(), 2), ("du".to_owned(), 2)]);
        let plain = editing(BackendKind::Owned, "d", 0, 1);
        let mut editor = editor_with(&plain.content, true, Measure::default());
        let mut state = laid_out_state(&editor);
        state.focus = Some(Focus::now());
        state.preedit = Some(input_method::Preedit {
            content: "に".into(),
            selection: None,
            text_size: None,
        });
        let (messages, _) = send(&mut editor, &mut state, typed("u"), None);
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Action(Action::Edit(Edit::Insert('u')))]
            ),
            "{messages:?}"
        );
    }

    #[test]
    fn suggestions_take_tab_from_a_host_binding_and_nothing_open_leaves_it_the_hosts() {
        use keyboard::key::Named;
        let hosted = || {
            let mut ev = completing("- a\n- ", 1, 2);
            ev.set_key_policy(Arc::new(
                |key: &keyboard::Key, _, modifiers: keyboard::Modifiers| {
                    if *key == keyboard::Key::Named(Named::Tab) && modifiers.is_empty() {
                        crate::KeyRoute::Host
                    } else {
                        crate::KeyRoute::Pass
                    }
                },
            ));
            ev
        };
        let keys = |ev: &mut crate::EditorView, state: &mut Option<State>, events: Vec<Event>| {
            let events = events.into_iter().map(|event| (event, None)).collect();
            popover_batch(ev, state, events)
        };
        let due = Some(Choice {
            id: 0,
            mode: Mode::Suggest,
            line: 1,
            range: 2..4,
        });

        // Open when the batch starts, or opened by the letters of the
        // batch, which the widget knows either way: Tab is theirs, and
        // captured, so no host hears of it as its own.
        let mut ev = hosted();
        let mut state = None;
        keys(&mut ev, &mut state, vec![typed("d"), typed("u")]);
        let (messages, captured, choice) = keys(&mut ev, &mut state, vec![named(Named::Tab)]);
        assert_eq!(spell_events(&messages), [SpellEvent::Key(MenuKey::Tab)]);
        assert_eq!((captured, choice), (vec![true], due.clone()));
        let mut ev = hosted();
        let (messages, captured, choice) = keys(
            &mut ev,
            &mut None,
            vec![typed("d"), typed("u"), named(Named::Tab)],
        );
        assert_eq!(spell_events(&messages), [SpellEvent::Key(MenuKey::Tab)]);
        assert_eq!((captured, choice), (vec![true; 3], due));

        // Nothing opened, which the widget knows too: the key is the
        // host's alone. The editor publishes nothing for it, does not
        // indent, and leaves the press uncaptured for the host's command.
        let mut ev = hosted();
        let (messages, captured, choice) =
            keys(&mut ev, &mut None, vec![typed("d"), named(Named::Tab)]);
        assert!(spell_events(&messages).is_empty(), "{messages:?}");
        assert_eq!((captured, choice), (vec![true, false], None));
        assert_eq!(ev.text(), "- a\n- d");
        // No press but Escape is ever claimed.
        assert!(!ev.take_claimed_escape());
    }

    // ── Where a place is ───────────────────────────────────────────────────

    /// Where the engine draws its caret.
    fn drawn_caret(content: &Content) -> Point {
        match content.0.borrow().selection() {
            Some(iced::widget::text_editor::Selection::Caret(point)) => point,
            other => panic!("no caret: {other:?}"),
        }
    }

    #[test]
    fn the_carets_place_is_where_the_engine_draws_the_caret() {
        // Latin, four Hebrew letters, Latin: the Hebrew run reads right to
        // left. Each engine places a caret there in its own way, and the
        // place reported for the caret is the engine's.
        let text = "ab \u{5e9}\u{5dc}\u{5d5}\u{5dd} cd";
        let size = Size::new(400.0, 200.0);
        for kind in BackendKind::ALL {
            for (at, _) in text.char_indices().chain([(text.len(), ' ')]) {
                let mut content = Content::new(kind, text);
                content.lay_out(size);
                content.move_to(caret(0, at));
                content.lay_out(size);
                let drawn = drawn_caret(&content);
                let place = content.anchor_row(0, at..at).unwrap();
                assert!(
                    near(place.left, drawn.x) && near(place.top, drawn.y),
                    "{kind:?} {at}: {place:?} {drawn:?}"
                );
                assert_eq!(
                    (place.right, place.bottom - place.top),
                    (place.left, LINE_HEIGHT),
                    "{kind:?} {at}"
                );
                // A place the caret is not at is where the engine would
                // draw the caret there.
                content.move_to(caret(0, if at == 0 { 1 } else { 0 }));
                content.lay_out(size);
                let elsewhere = content.anchor_row(0, at..at).unwrap();
                assert!(
                    near(elsewhere.left, drawn.x) && near(elsewhere.top, drawn.y),
                    "{kind:?} {at}: {elsewhere:?} {drawn:?}"
                );
            }
        }
        // The owned engine stands before the first Hebrew letter at that
        // letter's right edge, as the text reads.
        let mut content = Content::new(BackendKind::Owned, text);
        content.lay_out(size);
        let (left, right, rtl) = {
            let backend = content.0.borrow();
            let run = backend.buffer().layout_runs().next().unwrap();
            let glyph = run.glyphs.iter().find(|glyph| glyph.start == 3).unwrap();
            (glyph.x, glyph.x + glyph.w, glyph.level.is_rtl())
        };
        assert!(rtl && right - left > 1.0);
        assert!(near(content.anchor_row(0, 3..3).unwrap().left, right));
    }

    #[test]
    fn at_a_wrap_the_carets_place_is_on_the_carets_side_of_it() {
        // 200 px wide: thirteen cells a row, so the line wraps before
        // `cccc`.
        let text = "aaaa bbbb cccc dddd";
        let size = Size::new(116.0, 200.0);
        for kind in BackendKind::ALL {
            let mut content = Content::new(kind, text);
            content.lay_out(size);
            // End takes the caret to the end of the first row, where it
            // is drawn, though the same byte starts the second.
            content
                .0
                .borrow_mut()
                .perform(Action::Move(iced::widget::text_editor::Motion::End));
            content.lay_out(size);
            let at = content.cursor().position.column;
            let drawn = drawn_caret(&content);
            assert!(
                drawn.y == 0.0 && drawn.x > 8.0 * ADVANCE,
                "{kind:?}: {drawn:?} {at}"
            );
            let place = content.anchor_row(0, at..at).unwrap();
            assert!(
                near(place.left, drawn.x) && place.top == 0.0,
                "{kind:?}: {place:?} {drawn:?}"
            );
            // The same place with the caret elsewhere is the start of the
            // second row, where a word that starts there is.
            content.move_to(caret(0, 0));
            content.lay_out(size);
            let standalone = content.anchor_row(0, 10..10).unwrap();
            assert_eq!(
                (standalone.left, standalone.top),
                (0.0, LINE_HEIGHT),
                "{kind:?}"
            );
            // And a selection draws no caret: the place is as it stands
            // alone.
            content.move_to(Cursor {
                position: Position {
                    line: 0,
                    column: 10,
                },
                selection: Some(Position { line: 0, column: 2 }),
            });
            content.lay_out(size);
            assert_eq!(content.anchor_row(0, 10..10), Some(standalone), "{kind:?}");
        }
    }

    #[test]
    fn an_unchanged_popover_is_not_fitted_again() {
        let mut ev = on_task(BackendKind::Owned, Mode::Menu, plain_rows(&["one", "two"]));
        let mut state = {
            let editor = editor_with(&ev.content, true, Measure::default());
            laid_out_state(&editor)
        };
        // The labels fitted, and where the fit keeps them.
        let mut fitted = |ev: &crate::EditorView, bounds: Rectangle| {
            let editor = editor_with(&ev.content, true, Measure::default()).popover(ev.popover());
            let labels = editor
                .menu_overlay(&mut state, bounds)
                .unwrap()
                .labels
                .to_vec();
            let (key, fit) = state.menu_fit.as_ref().unwrap();
            (labels, key.items.as_ptr(), fit.width)
        };
        let first = fitted(&ev, BOUNDS);
        assert_eq!(first.0, ["one", "two"]);
        // Frame after frame, the same rows are compared and not copied.
        assert_eq!(fitted(&ev, BOUNDS), first);
        // Other rows, or another width, are fitted afresh.
        hang(&mut ev, Mode::Menu, 0, 2..5, plain_rows(&["one", "three"]));
        let second = fitted(&ev, BOUNDS);
        assert_eq!(second.0, ["one", "three"]);
        let narrow = Rectangle {
            width: 150.0,
            ..BOUNDS
        };
        assert_eq!(fitted(&ev, narrow).2, 134.0);
    }

    // ── Every opener and every key ─────────────────────────────────────────

    /// Who a press went to.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Owner {
        /// The open popover.
        Popover,
        /// The text: the editor carried out the press.
        Text,
        /// The host's own key handling: its Escape, or its shortcut.
        App,
        /// The host's field, which took the keyboard for a request.
        Field,
    }

    /// What comes before the press, in its batch or, for the last three,
    /// in an earlier one, so that the host has drawn what is open.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Opener {
        Nothing,
        SuggestKeyOnAWord,
        SuggestKeyOffAWord,
        RightClickOnAWord,
        RequestForAMenu,
        RequestForNothing,
        RequestForTheKeyboard,
        TypingThatOffers,
        TypingThatOffersNothing,
        SpellMenuOpen,
        MenuOpen,
        SuggestionsOpen,
    }

    const OPENERS: [Opener; 12] = [
        Opener::Nothing,
        Opener::SuggestKeyOnAWord,
        Opener::SuggestKeyOffAWord,
        Opener::RightClickOnAWord,
        Opener::RequestForAMenu,
        Opener::RequestForNothing,
        Opener::RequestForTheKeyboard,
        Opener::TypingThatOffers,
        Opener::TypingThatOffersNothing,
        Opener::SpellMenuOpen,
        Opener::MenuOpen,
        Opener::SuggestionsOpen,
    ];

    /// The presses of the table, in the order of [`owners`]: Up, Down,
    /// Enter, Tab, Escape, Ctrl+Escape, a letter, a letter that is an
    /// accelerator of the status menu, Backspace, and a left press on the
    /// text, off any popover.
    const PRESSES: [&str; 10] = [
        "up",
        "down",
        "enter",
        "tab",
        "escape",
        "ctrl+escape",
        "y",
        "x",
        "backspace",
        "press",
    ];

    fn the_press(name: &str) -> (Event, Option<Point>) {
        use keyboard::key::Named;
        let event = match name {
            "up" => named(Named::ArrowUp),
            "down" => named(Named::ArrowDown),
            "enter" => named(Named::Enter),
            "tab" => named(Named::Tab),
            "escape" => named(Named::Escape),
            "ctrl+escape" => key(
                keyboard::Key::Named(Named::Escape),
                keyboard::Modifiers::CTRL,
                None,
            ),
            "backspace" => named(Named::Backspace),
            "press" => return (press(mouse::Button::Left), Some(over(1.0, 1.0))),
            letter => typed(letter),
        };
        (event, None)
    }

    /// Who each of [`PRESSES`] goes to after `opener` on the owned engine,
    /// one letter a press: `P` the popover, `T` the text, `A` the host's
    /// key handling, `F` the host's field.
    fn owners(opener: Opener) -> &'static str {
        match opener {
            Opener::Nothing
            | Opener::SuggestKeyOffAWord
            | Opener::RequestForNothing
            | Opener::TypingThatOffersNothing => "TTTTAATTTT",
            // The suggestions menu: Tab and a letter close it and are the
            // text's, and a press in the editor only closes it.
            Opener::SuggestKeyOnAWord | Opener::RightClickOnAWord | Opener::SpellMenuOpen => {
                "PPPTPATTTP"
            }
            // A menu of the host's, which has an accelerator.
            Opener::RequestForAMenu | Opener::MenuOpen => "PPPTPATPTP",
            Opener::RequestForTheKeyboard => "FFFFAAFFFT",
            // Suggestions leave every other key and the press to the text.
            Opener::TypingThatOffers | Opener::SuggestionsOpen => "PPPPPATTTT",
        }
    }

    /// The table's note: a task with a misspelled word, and a second line.
    const TABLE_NOTE: &str = "- [ ] teh cat \nnext";

    /// An editor on `kind` over [`TABLE_NOTE`] with every source a host
    /// has, the caret where `opener` needs it, and what an earlier batch
    /// opened, if `opener` is one of those.
    fn table_editor(kind: BackendKind, opener: Opener) -> (crate::EditorView, Option<State>) {
        let mut ev = crate::EditorView::with_backend(kind, TABLE_NOTE);
        let proofreader = fixtures::Rejects::new(&["teh"]).suggesting(&[("teh", &["the", "ten"])]);
        ev.set_proofreader(Some(Arc::new(proofreader)));
        ev.set_menu_source(Some(Arc::new(fixtures::Menus::default())));
        ev.set_suggest_source(Some(Arc::new(fixtures::Completes::new(&[
            "due",
            "duplicate",
        ]))));
        let requests = request_policy();
        let suggests = suggest_policy();
        ev.set_key_policy(Arc::new(move |key, physical, modifiers| {
            match suggests(key, physical, modifiers) {
                crate::KeyRoute::Pass => requests(key, physical, modifiers),
                route => route,
            }
        }));
        let column = match opener {
            Opener::SuggestKeyOnAWord | Opener::SpellMenuOpen => 9,
            Opener::TypingThatOffers
            | Opener::TypingThatOffersNothing
            | Opener::SuggestionsOpen => 14,
            _ => 12,
        };
        ev.move_cursor(caret(0, column));
        let mut state = None;
        let owned = kind == BackendKind::Owned;
        match opener {
            Opener::SpellMenuOpen => {
                ev.spell(SpellEvent::OpenAtCaret);
                assert!(ev.spell_menu().is_some());
            }
            // The iced engine opens no popover; the widget routes what it
            // is handed the same on both.
            Opener::MenuOpen if owned => {
                ev.spell(SpellEvent::Request(STATUS));
            }
            Opener::MenuOpen => hang(&mut ev, Mode::Menu, 0, 2..5, status_rows()),
            Opener::SuggestionsOpen if owned => {
                popover_batch(
                    &mut ev,
                    &mut state,
                    vec![(typed("d"), None), (typed("u"), None)],
                );
            }
            Opener::SuggestionsOpen => {
                hang(&mut ev, Mode::Suggest, 0, 14..14, plain_rows(&["due"]));
            }
            _ => return (ev, state),
        }
        assert!(ev.popover().is_some(), "{kind:?} {opener:?}");
        (ev, state)
    }

    /// The events of `opener` that share the press's batch.
    fn opening_events(opener: Opener) -> Vec<(Event, Option<Point>)> {
        match opener {
            Opener::SuggestKeyOnAWord | Opener::SuggestKeyOffAWord => vec![(suggest_key(), None)],
            Opener::RightClickOnAWord => {
                vec![(press(mouse::Button::Right), Some(over(7.0, 0.0)))]
            }
            Opener::RequestForAMenu => vec![(request_key("s"), None)],
            Opener::RequestForNothing => vec![(request_key("n"), None)],
            Opener::RequestForTheKeyboard => vec![(request_key("k"), None)],
            Opener::TypingThatOffers => vec![(typed("d"), None), (typed("u"), None)],
            Opener::TypingThatOffersNothing => vec![(typed("d"), None)],
            Opener::Nothing
            | Opener::SpellMenuOpen
            | Opener::MenuOpen
            | Opener::SuggestionsOpen => Vec::new(),
        }
    }

    /// Everyone who acted on the press named `name`, which published
    /// `messages`: the popover, the text or the host's field as the host
    /// carries each message out, and then the host's own key handling,
    /// which hears of every Escape.
    fn who_took(ev: &mut crate::EditorView, name: &str, messages: Vec<Message>) -> Vec<Owner> {
        let mut took = Vec::new();
        let alone = messages.len() == 1;
        for message in messages {
            let open = ev.popover().map(Popover::mode);
            match &message {
                Message::Spell(event) => match event {
                    SpellEvent::Stroke(_) => took.push(Owner::Field),
                    SpellEvent::Move(_)
                    | SpellEvent::Hover(_)
                    | SpellEvent::Accept(_)
                    | SpellEvent::AcceptSelected
                    | SpellEvent::Dismiss => {
                        assert!(open.is_some(), "{name}: {event:?} with nothing open");
                        took.push(Owner::Popover);
                    }
                    // Escape with nothing open is no one's yet: the host's
                    // key handling takes it below.
                    SpellEvent::Key(MenuKey::Escape) if open.is_none() => {}
                    SpellEvent::Key(key) => took.push(match open.map(|mode| key.on(mode)) {
                        Some(Some(_)) => Owner::Popover,
                        _ => Owner::Text,
                    }),
                    SpellEvent::Press(_) => took.push(match open {
                        Some(Mode::Menu) => Owner::Popover,
                        _ => Owner::Text,
                    }),
                    // A press in the editor that only closes a menu is the
                    // menu's; before another key's own message, a close
                    // is that key's side effect.
                    SpellEvent::Close if name == "press" && alone => took.push(Owner::Popover),
                    SpellEvent::Close => {}
                    other => panic!("{name}: {other:?}"),
                },
                _ => took.push(Owner::Text),
            }
            host_applies(ev, message);
        }
        match name {
            // The host's key handling: a press a popover claimed is done
            // with; the suggestions menu is closed from here; and
            // otherwise the Escape is the host's own.
            "escape" if ev.take_claimed_escape() => {}
            "escape" if ev.dismiss_popover() => took.push(Owner::Popover),
            "escape" | "ctrl+escape" => took.push(Owner::App),
            _ => {}
        }
        took
    }

    #[test]
    fn after_every_opener_each_press_has_exactly_one_owner() {
        for kind in BackendKind::ALL {
            for opener in OPENERS {
                // The iced engine has no spelling and opens no popover:
                // after any opener of the batch, nothing is open.
                let owned = kind == BackendKind::Owned;
                let drawn = matches!(opener, Opener::MenuOpen | Opener::SuggestionsOpen);
                if !owned && opener == Opener::SpellMenuOpen {
                    continue;
                }
                let expected = if owned || drawn {
                    owners(opener)
                } else {
                    owners(Opener::Nothing)
                };
                for (name, letter) in PRESSES.into_iter().zip(expected.chars()) {
                    let (mut ev, mut state) = table_editor(kind, opener);
                    let mut events = opening_events(opener);
                    let opening = events.len();
                    events.push(the_press(name));
                    // The opener's messages come first: in its batch nothing
                    // is drawn, so no overlay publishes ahead of them.
                    let (published, _) = widget_events(&ev, &mut state, events);
                    let mut messages = Vec::new();
                    for (at, message) in published {
                        if at < opening {
                            assert!(messages.is_empty());
                            host_applies(&mut ev, message);
                        } else {
                            messages.push(message);
                        }
                    }
                    let before = (ev.text(), ev.cursor());
                    let took = who_took(&mut ev, name, messages.clone());
                    let owner = match letter {
                        'P' => Owner::Popover,
                        'T' => Owner::Text,
                        'A' => Owner::App,
                        _ => Owner::Field,
                    };
                    let case = format!("{kind:?} {opener:?} {name}: {messages:?}");
                    assert_eq!(took, [owner], "{case}");
                    // And it was carried out once: a letter the text took
                    // is in the text once more, and what another owner
                    // took left the text alone.
                    let count = |text: &str| text.matches(name).count();
                    let typed = usize::from(owner == Owner::Text && name.len() == 1);
                    assert_eq!(count(&ev.text()), count(&before.0) + typed, "{case}");
                    if matches!(owner, Owner::App | Owner::Field) {
                        assert_eq!((ev.text(), ev.cursor()), before, "{case}");
                    }
                    // No Escape is left claimed for a later press.
                    assert!(!ev.take_claimed_escape(), "{case}");
                }
            }
        }
    }
}
