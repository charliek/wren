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
use std::rc::Rc;
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
    self, Answer, CaretLine, Edition, MenuOverlay, MenuPress, MenuSource, Now, Opened, Popover,
    Provider, ResolvedPress, Seen, Sources, SuggestSource, Taken,
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

    /// Lays the text out for a text area of `size`, set as `settings` and
    /// `metrics` say, as the widget does for a frame.
    #[cfg(feature = "test-support")]
    pub fn lay_out_as(&self, size: Size, settings: &HighlighterSettings, metrics: Metrics) {
        let mut highlighter = MarkdownHighlighter::new(settings);
        self.0.borrow_mut().update(
            size,
            settings.font,
            Pixels(metrics.text),
            metrics.line_height(),
            Wrapping::WordOrGlyph,
            &mut highlighter,
        );
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
    /// Who a right-click asks for a menu, in order.
    providers: &'a [Provider],
    /// The host's text and list of providers, which a right-click a
    /// provider takes is bound to.
    edition: Edition,
    /// The host took the keyboard earlier in this batch: the input after
    /// the handover is dropped; see [`popover`]. When it took it for a row
    /// of suggestions or a dialog's row chosen in this batch, or for a
    /// request whose source holds Escape ([`crate::MenuSource::holds_escape`]),
    /// Escape after it is dropped too ([`Target::Handed`],
    /// [`Now::holds_escape`]).
    handed_over: Cell<bool>,
    /// The open popover as the events this widget has handled leave it; see
    /// [`popover::OpenMenu`]. The host builds a new widget once it has
    /// applied them.
    menu_now: Now,
    /// Where the host learns the text's origin in the window.
    origin: Option<&'a Cell<Option<Point>>>,
    /// The host's verdict on the presses it resolved.
    resolved_press: ResolvedPress,
    /// The host gave back the keyboard; see
    /// [`crate::EditorView::hand_back`].
    handback: bool,
    /// Input of the editor's own came in this batch, which ends the
    /// handback as the host applies its messages.
    handback_over: Cell<bool>,
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
            providers: &popover::SPELLING_ALONE,
            edition: Edition::default(),
            handed_over: Cell::new(false),
            menu_now: Now::new(),
            origin: None,
            resolved_press: ResolvedPress::default(),
            handback: false,
            handback_over: Cell::new(false),
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

    /// Who a right-click asks for a menu, in order; the suggestions menu
    /// alone unless the host says otherwise. `edition` is the host's text
    /// and this list of its providers, which the editor checks a click
    /// against when the host hands it over.
    pub(crate) fn providers(mut self, providers: &'a [Provider], edition: Edition) -> Self {
        self.providers = providers;
        self.edition = edition;
        self.menu_now.count_from(edition.opened);
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

    /// Whether the host gave back the keyboard and the editor has seen no
    /// input of its own since; see [`crate::EditorView::hand_back`].
    pub(crate) fn handback(mut self, handback: bool) -> Self {
        self.handback = handback;
        self
    }

    /// Whether an input method's commits, and its preedits that empty, are
    /// dropped: after the host gave back the keyboard, until the editor's
    /// own input.
    fn held_back(&self) -> bool {
        self.handback && !self.handback_over.get()
    }

    /// Input of the editor's own that its messages do not show: the host is
    /// told, in the input's order.
    fn end_handback(&self, shell: &mut Shell<'_, Message>) {
        if self.held_back() {
            self.handback_over.set(true);
            shell.publish(Message::HandbackOver);
        }
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
        // A row the overlay chose with the pointer, in this batch or in one
        // the widget saw nothing of, handed the host the keyboard: a
        // composition under way is not the note's.
        if state.handed_by_pointer.take() {
            self.drop_composition(state, shell);
        }
        // A row the overlay chose with the pointer, which it saw before
        // this event, may have handed the host the keyboard.
        self.take_handover(state, shell);
        let mut accelerators = Vec::new();
        let target = self.target(&mut accelerators);
        // The host took the keyboard earlier in this batch: until the
        // batch ends, the keys its field would take are dropped, whatever
        // has opened or closed since.
        let held = self.handed_over.get();
        let routes = Routes {
            presses: target,
            keys: match (held, self.menu_now.holds_escape()) {
                (true, true) => Some(Target::Handed),
                (true, false) => Some(Target::Keyboard),
                (false, _) => target,
            },
        };
        // A dropped composition that ended is committed at once or not at
        // all: any press shows that it was not.
        if matches!(
            event,
            Event::Keyboard(keyboard::Event::KeyPressed { .. })
                | Event::Mouse(mouse::Event::ButtonPressed(_))
        ) {
            state.dropped.pressed();
        }
        self.box_events(state, event, bounds, cursor, shell);
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
            routes,
        ) {
            // After the host gave back the keyboard: what ends the handback
            // (`EditorView::hand_back`).
            if self.held_back() {
                match &update {
                    Update::InputMethod(Ime::Preedit { content, .. }) if !content.is_empty() => {
                        self.end_handback(shell);
                    }
                    Update::Click(_) => self.end_handback(shell),
                    Update::Key(Routed::Binding(binding) | Routed::Close(Some(binding)))
                        if ends_handback(binding) =>
                    {
                        self.handback_over.set(true);
                    }
                    _ => {}
                }
            }
            let update = match update {
                // A second press that makes a double click on the box a
                // click waits on: no click on the box, but the double
                // click's selection. Any other press lets the click on the
                // box go first.
                Update::Click(click) if state.box_cycle.is_some() => {
                    let cycle = state.box_cycle.take().expect("a click waits");
                    let double = popover::click_kind(click.kind(), state.click_kind)
                        == mouse::click::Kind::Double
                        && !self.left_to_host.get()
                        && self.box_under(state, click.position()) == Some(cycle.click.line);
                    if double {
                        Update::BoxDouble(click)
                    } else {
                        self.publish(shell, Message::BoxClick(cycle.click));
                        self.click_update(state, click)
                    }
                }
                Update::Click(click) => self.click_update(state, click),
                update => update,
            };
            match update {
                Update::BoxDouble(click) => {
                    // As a double click, from the place pressed: the first
                    // press of it left the caret where it was.
                    state.focus = Some(Focus::now());
                    state.last_click = Some(click);
                    state.click_kind = Some(mouse::click::Kind::Double);
                    state.drag_click = Some(mouse::click::Kind::Double);
                    self.publish(shell, Message::Action(Action::Click(click.position())));
                    self.publish(shell, Message::Action(Action::SelectWord));
                    shell.capture_event();
                }
                Update::BoxPress(click, line) => {
                    // A press on a task's box: a click on it once let go,
                    // which leaves the caret where it is, or a drag from
                    // where it was pressed once the pointer moves away.
                    if self.menu_now.is_open() {
                        self.publish(shell, Message::Spell(SpellEvent::Close));
                    }
                    state.focus = Some(Focus::now());
                    state.last_click = Some(click);
                    state.click_kind = Some(mouse::click::Kind::Single);
                    state.drag_click = Some(mouse::click::Kind::Single);
                    state.box_press = Some(BoxPress {
                        origin: click.position(),
                        at: Instant::now(),
                        click: crate::BoxClick::new(line, self.edition.revision, click.position()),
                    });
                    shell.capture_event();
                }
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
                    // Only what a provider takes answers, as the
                    // suggestions menu takes a misspelled word; anywhere
                    // else the press leaves focus and the caret alone.
                    if let Some(taken) = self.taken(state, position) {
                        state.focus = Some(Focus::now());
                        state.drag_click = None;
                        // So a slight drift of the fingers during the click
                        // cannot add up with an earlier scroll's remainder
                        // to a whole line, which would close the menu.
                        state.partial_scroll = 0.0;
                        match taken {
                            Taken::Spelling => {
                                let event = SpellEvent::OpenAt(position);
                                self.publish(shell, Message::Spell(event));
                            }
                            Taken::Host {
                                provider,
                                answer,
                                fingerprint,
                            } => {
                                let event = SpellEvent::Context {
                                    provider,
                                    at: position,
                                    seen: Seen::new(self.edition, fingerprint),
                                };
                                // The menu as the editor will open it: the
                                // next popover it opens.
                                let answer = match answer {
                                    Answer::Menu(menu) => {
                                        Answer::Menu(menu.opened_as(self.edition.opened + 1))
                                    }
                                    answer => answer,
                                };
                                self.menu_now.publish_provided(
                                    shell,
                                    Message::Spell(event),
                                    &|| self.caret_line(),
                                    self.sources,
                                    answer,
                                );
                            }
                        }
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
                // While a press on the box is held within its reach, the
                // pointer drags nothing ([`Self::box_events`]).
                Update::Drag(position) => {
                    if state.box_press.is_none() {
                        self.publish(shell, Message::Action(Action::Drag(position)));
                    }
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
                // A key the field of a host that took the keyboard would
                // take: dropped, for the rest of the handover's batch.
                Update::Key(Routed::Binding(Binding::Custom(Message::Spell(
                    SpellEvent::Stroke(stroke),
                )))) => {
                    // The host hears of an Escape from its own key handling
                    // as well: told of it here, in its order, it drops it.
                    if stroke.named() == Some(keyboard::key::Named::Escape) {
                        self.publish(shell, Message::Spell(SpellEvent::Stroke(stroke)));
                    }
                    shell.capture_event();
                }
                // So is what an input method composes or commits in that
                // batch, and whatever the composition that was dropped
                // commits later: none of it is the note's.
                Update::InputMethod(Ime::Preedit { content, .. })
                    if state.dropped.preedit(content.is_empty(), held) => {}
                Update::InputMethod(Ime::Commit(_)) if state.dropped.commit(held) => {}
                // What an input method commits after the host gave back the
                // keyboard, and a preedit that empties, before input of the
                // editor's own: the host's field's, never the note's.
                Update::InputMethod(Ime::Preedit { content, .. })
                    if content.is_empty() && self.held_back() => {}
                Update::InputMethod(Ime::Commit(_)) if self.held_back() => {}
                Update::InputMethod(update) => match update {
                    Ime::Toggle(is_open) => {
                        state.dropped = Dropped::Nothing;
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

        // The host took the keyboard for what this event asked for.
        self.take_handover(state, shell);

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

    /// Once the host has taken the keyboard in this batch, for what an
    /// event asked for or a row chosen: a composition the note has under
    /// way is cancelled, and from here to the end of the batch the input
    /// is dropped.
    fn take_handover(&self, state: &mut State, shell: &mut Shell<'_, Message>) {
        if !self.handed_over.get() && matches!(self.menu_now.live(), Some((Opened::Keyboard, _))) {
            self.handed_over.set(true);
            self.handback_over.set(true);
            self.drop_composition(state, shell);
        }
    }

    /// Cancel the composition the note has under way, if any: what it
    /// commits, whenever it does, is dropped ([`Dropped`]).
    fn drop_composition(&self, state: &mut State, shell: &mut Shell<'_, Message>) {
        if state.is_composing() {
            state.preedit = Some(input_method::Preedit::new());
            state.dropped = Dropped::Composing;
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
        // A row of a provider's menu, or of a popover with a row that runs
        // a command, is chosen from that popover or from none: the choice
        // names it. So are such suggestions' Enter and Tab, which choose
        // their selected row.
        let row = match message {
            Message::Spell(SpellEvent::Accept(row)) => Some(Some(row)),
            Message::Spell(SpellEvent::AcceptSelected) => Some(None),
            Message::Spell(SpellEvent::Key(proof::MenuKey::Enter | proof::MenuKey::Tab))
                if self.menu_now.binds_its_keys(self.popover) =>
            {
                Some(None)
            }
            _ => None,
        };
        // Enter and Tab choose the selected row of open suggestions.
        let suggestion = matches!(
            message,
            Message::Spell(SpellEvent::Key(proof::MenuKey::Enter | proof::MenuKey::Tab))
        ) && self
            .menu_now
            .live()
            .is_some_and(|(opened, _)| opened == Opened::Suggest);
        // Suggestions the batch's own typing opened, which the host has not
        // shown and whose source may answer otherwise when it opens them:
        // Tab on them is no one's. Enter reaches here only on a row that
        // opens a menu ([`Self::target`]): the host chooses that row if it
        // has it selected, or else Enter is the text's; and since it may
        // open the menu, the keys after it are dropped.
        if suggestion && self.menu_now.predicts_suggestions() {
            let enter = matches!(
                message,
                Message::Spell(SpellEvent::Key(proof::MenuKey::Enter))
            );
            if let Some(id) = self.menu_now.predicted_menu_row().filter(|_| enter) {
                let message = Message::Spell(SpellEvent::EnterOn { id });
                self.menu_now
                    .publish(shell, message, &|| self.caret_line(), self.sources);
                self.menu_now.hand_over(true);
            }
            return;
        }
        // A row whose choice opens a menu of the host's, which the widget
        // cannot open the same: the keys after it are dropped.
        let chosen = row.or(suggestion.then_some(None));
        let opens = chosen.is_some_and(|row| self.menu_now.opens_menu(self.popover, row));
        let dialog = chosen.is_some_and(|row| self.menu_now.opens_dialog(self.popover, row));
        let message = match row.zip(self.menu_now.bound(self.popover)) {
            Some((row, menu)) => Message::Spell(SpellEvent::AcceptOf { menu, row }),
            None => message,
        };
        let request = match message {
            Message::Spell(SpellEvent::Request(id)) => Some(id),
            _ => None,
        };
        self.menu_now
            .publish(shell, message, &|| self.caret_line(), self.sources);
        // A dialog's request: the host takes the keyboard for it whatever
        // the widget knows of the caret's line, and the batch's Escape
        // after it is no one's either.
        if let Some((id, source)) = request.zip(self.sources.menus) {
            if source.holds_escape(id) {
                self.menu_now.hand_over(true);
            }
        }
        if opens {
            self.menu_now.hand_over(suggestion || dialog);
        }
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
            // Enter on suggestions opened in this batch, which the host has
            // not shown, is the text's, whatever their rows, but on a row
            // that opens a menu ("Due date…", which writes nothing).
            Opened::Suggest if self.menu_now.predicts_suggestions() => {
                if self.menu_now.predicted_menu_row().is_some() {
                    Target::Suggest
                } else {
                    Target::Tentative
                }
            }
            Opened::Suggest if !self.menu_now.takes_enter(self.popover) => Target::Tentative,
            Opened::Suggest => Target::Suggest,
            Opened::Keyboard => Target::Keyboard,
        })
    }

    /// Whether routing `event` with the pointer at the cursor ends the open
    /// popover or changes its rows, for the overlay, which sees a batch
    /// before the widget routes it ([`popover::InOrder`]). `focused`,
    /// `composing` and `scroll` (the part of a line the wheel has moved)
    /// are the widget's state when the batch starts, and follow the batch's
    /// preedits and wheel as the widget's will; `bounds` are the widget's.
    ///
    /// A press off the popover ends it: a menu closes for it, and for
    /// suggestions a click moves the caret, or the press is outside the
    /// editor; one that leaves suggestions open is taken as ending them
    /// all the same, which only leaves the presses after it to the widget.
    /// So does the wheel over the editor once it adds up to a whole line,
    /// which scrolls.
    fn ending<'b>(
        &'b self,
        focused: bool,
        mut composing: bool,
        mut scroll: f32,
        bounds: Rectangle,
    ) -> impl FnMut(&Event, mouse::Cursor) -> bool + 'b {
        move |event, cursor| match event {
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
                    // The key of a row that cannot be chosen does nothing.
                    Some(Routed::Binding(Binding::Sequence(sequence))) if sequence.is_empty() => {
                        false
                    }
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
            Event::Mouse(mouse::Event::ButtonPressed(_)) => self.target(&mut Vec::new()).is_some(),
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let lines = wheel_lines(*delta) + scroll;
                scroll = lines.fract();
                lines as i32 != 0
            }
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
        let hints = popover::hint_size(style.text_size);
        let extras = popover::Extras::of(menu, |hint| popover::measure(hint, style.font, hints));
        // Compared against the rows as they are: a popover's labels are
        // copied only when they are fitted afresh.
        let fitted = state.menu_fit.as_ref().is_some_and(|(key, _)| {
            key.items == menu.labels()
                && (key.font, key.text_size, key.width) == (style.font, text_size, width)
                && key.extra == extras.width().to_bits()
        });
        if !fitted {
            let fit = popover::fit_with(menu.labels(), extras.width(), bounds.width, |text| {
                popover::measure(text, style.font, style.text_size)
            });
            let key = FitKey {
                items: menu.labels().to_vec(),
                font: style.font,
                text_size,
                width,
                extra: extras.width().to_bits(),
            };
            state.menu_fit = Some((key, fit));
        }
        let state = &*state;
        let (_, fit) = state.menu_fit.as_ref()?;
        let size = Size::new(fit.width, popover::height_of(menu, fit.labels.len()));
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

    /// What `event` does to a press on a task's box, held or let go, before
    /// the event is handled as it is with none ([`crate::BoxClick`]).
    ///
    /// A held press ends as no click on the box with anything else
    /// pressed, a key, a modifier held, or the window losing the keyboard;
    /// and with the pointer moved more than [`crate::BoxClick::DRAG`] from
    /// where it was pressed, inside the editor or out, when it is a click
    /// in the text there, from which the drag selects. Let go within that
    /// reach on the same box, it is a click on the box, which waits until
    /// a second press could no longer make it a double click; let go
    /// anywhere else, it is a click in the text where it was pressed.
    ///
    /// A click that waits is published once its time has passed, at the
    /// redraw asked for then or the first event after it, and at once
    /// before any event but a move, a redraw, a modifier, or a left press
    /// or release, which the press's own handling decides on.
    fn box_events(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) {
        let now = match event {
            Event::Window(window::Event::RedrawRequested(now)) => *now,
            _ => Instant::now(),
        };
        if let Some(cycle) = state.box_cycle {
            let waits = now < cycle.due
                && matches!(
                    event,
                    Event::Mouse(
                        mouse::Event::CursorMoved { .. }
                            | mouse::Event::ButtonPressed(mouse::Button::Left)
                            | mouse::Event::ButtonReleased(mouse::Button::Left)
                    ) | Event::Window(window::Event::RedrawRequested(_))
                        | Event::Keyboard(keyboard::Event::ModifiersChanged(_))
                );
            if !waits {
                state.box_cycle = None;
                if now >= cycle.due {
                    // No press now makes a double click with it.
                    state.last_click = None;
                }
                self.publish(shell, Message::BoxClick(cycle.click));
            }
        }
        let padding = self.padding(bounds.width, state.advance.width);
        let in_text = |at: Point| at - Vector::new(bounds.x + padding.left, bounds.y + padding.top);
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                if !modifiers.is_empty() {
                    state.box_press = None;
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { .. })
            | Event::Mouse(mouse::Event::ButtonPressed(_))
            | Event::Window(window::Event::Unfocused) => state.box_press = None,
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let away = state.box_press.filter(|press| {
                    press.origin.distance(in_text(*position)) > crate::BoxClick::DRAG
                });
                if let Some(press) = away {
                    // A drag from where the box was pressed.
                    state.box_press = None;
                    self.publish(shell, Message::Action(Action::Click(press.origin)));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let Some(press) = state.box_press.take() else {
                    return;
                };
                let on_box = cursor.position().map(in_text).is_some_and(|at| {
                    at.distance(press.origin) <= crate::BoxClick::DRAG
                        && self.box_under(state, at) == Some(press.click.line)
                });
                if on_box {
                    let due = press.at + DOUBLE_CLICK;
                    state.box_cycle = Some(BoxCycle {
                        click: press.click,
                        due,
                    });
                    shell.request_redraw_at(due);
                } else {
                    self.publish(shell, Message::Action(Action::Click(press.origin)));
                }
                shell.capture_event();
            }
            _ => {}
        }
    }

    /// What a left press in the editor is: a single click with no modifier
    /// held on a task's box is a press on it, and any other the click it
    /// is.
    fn click_update(&self, state: &State, click: mouse::Click) -> Update {
        if self.left_to_host.get() {
            return Update::Press(Pressed::Left(click));
        }
        let single =
            popover::click_kind(click.kind(), state.click_kind) == mouse::click::Kind::Single;
        let line = (single && state.modifiers.is_empty())
            .then(|| self.box_under(state, click.position()))
            .flatten();
        match line {
            Some(line) => Update::BoxPress(click, line),
            None => Update::Click(click),
        }
    }

    /// The line of the task whose box, `[` to `]`, lies under `point`,
    /// relative to the text origin, as the last layout placed it: only with
    /// the host's tasks on, on the advanced engine, and while the widget
    /// sees the text as the host will have it.
    fn box_under(&self, state: &State, point: Point) -> Option<usize> {
        if self.content.kind() != BackendKind::Owned
            || self.highlighter_settings.tasks.is_none()
            || !self.menu_now.sees_the_text()
        {
            return None;
        }
        let place = self.content.place_under(point, |_, _| Vec::new())?;
        let column = place.column?;
        // The spans its highlighter read for the line when it drew it, or
        // read afresh when it has not since the text above changed.
        let cached =
            (state.highlighter.borrow().spans(place.line)).map(|spans| hit::on_box(spans, column));
        let on_box = cached.unwrap_or_else(|| {
            self.content.with_lines(|lines| {
                let spans = crate::highlighter::spans_at(
                    &lines,
                    place.line,
                    self.highlighter_settings.front_matter_end,
                    self.highlighter_settings.tasks,
                );
                hit::on_box(&spans, column)
            })
        });
        on_box.then_some(place.line)
    }

    /// The misspelled word under `point`, relative to the text origin; see
    /// [`Self::hit_under`]. With no speller there is none to look for.
    fn misspelling_under(&self, state: &State, point: Point) -> Option<(usize, Range<usize>)> {
        self.speller()?;
        let (line, hit) = self.hit_under(state, point)?;
        Some((line, hit.misspelling()?))
    }

    /// Who takes a right-click at `point`, relative to the text origin, of
    /// the host's providers in order ([`popover::taken`]). The suggestions
    /// menu takes a misspelled word as it is drawn; a provider of the
    /// host's is asked about the place as the editor will read it when the
    /// host hands it the click ([`hit::Reading`]), and only while the
    /// widget sees the text as the host will then
    /// ([`Now::sees_the_text`]).
    fn taken(&self, state: &State, point: Point) -> Option<Taken> {
        // An earlier message of the batch may have changed what is under
        // the pointer by the time the host hears of the click: no provider
        // of the host's is asked then, and the press is what it was before
        // there were any.
        if !self.menu_now.sees_the_text() {
            let spelled = popover::spelling_leads(self.providers)
                && self.misspelling_under(state, point).is_some();
            return spelled.then_some(Taken::Spelling);
        }
        let reading = hit::Reading {
            speller: self.speller(),
            front_matter_end: self.highlighter_settings.front_matter_end,
            tasks: self.highlighter_settings.tasks,
        };
        popover::taken_in(
            self.providers,
            || self.misspelling_under(state, point).is_some(),
            || reading.clicked_in(self.content, point),
        )
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
    /// The composition whose text is not the note's; see [`Dropped`].
    dropped: Dropped,
    /// The last left press inside the editor, as iced counts clicks.
    last_click: Option<mouse::Click>,
    /// What the last left press was taken as, `None` when it only closed
    /// the menu.
    click_kind: Option<mouse::click::Kind>,
    /// The host verdicts already taken into `click_kind`.
    resolved_seen: u64,
    drag_click: Option<mouse::click::Kind>,
    /// A press on a task's box that is a click on it once it is let go,
    /// unless the pointer first moves away; see [`crate::BoxClick`].
    box_press: Option<BoxPress>,
    /// A click on a box, let go, waiting to be sure it is no double click.
    box_cycle: Option<BoxCycle>,
    /// The keyboard modifiers held, as the window last said.
    modifiers: keyboard::Modifiers,
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
    /// The overlay chose a row with the pointer that handed the host the
    /// keyboard ([`popover::InOrder`]), which the widget may hear of only
    /// in a later batch, when every event of this one was the overlay's.
    handed_by_pointer: Rc<Cell<bool>>,
}

/// A left press on a task's box, held: where it was pressed, relative to
/// the text origin, when, and the click it makes once let go on the box.
#[derive(Debug, Clone, Copy)]
struct BoxPress {
    origin: Point,
    at: Instant,
    click: crate::BoxClick,
}

/// A click on a task's box, let go, that waits for the time in which a
/// second press would make it a double click to pass.
#[derive(Debug, Clone, Copy)]
struct BoxCycle {
    click: crate::BoxClick,
    /// When the time ends; at least as late as the window iced counts a
    /// double click in, from the press.
    due: Instant,
}

/// How long after a press a second press makes a double click: iced's
/// window ([`mouse::Click`]), and so how long a click on a box waits.
const DOUBLE_CLICK: Duration = Duration::from_millis(300);

/// A composition of the input method that was under way, or began, in a
/// batch of events in which the host took the keyboard: it was typed
/// before the user could see what the host opened, so it is dropped, and
/// what it commits is not the note's whenever it arrives, the editor
/// having the keyboard still or again (see [`popover`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Dropped {
    /// No such composition.
    #[default]
    Nothing,
    /// It has a preedit.
    Composing,
    /// Its preedit has emptied, as it does just before it commits, and
    /// when it is cancelled.
    Ending,
}

impl Dropped {
    /// Whether a preedit is dropped: one of the dropped composition, or
    /// any while the host holds the keyboard (`held`), which starts one.
    /// A preedit that fills again after the dropped composition ended is
    /// a new composition, and the note's.
    fn preedit(&mut self, empty: bool, held: bool) -> bool {
        *self = match (*self, empty) {
            (Self::Nothing | Self::Ending, false) if !held => Self::Nothing,
            (_, false) => Self::Composing,
            (Self::Nothing, true) => Self::Nothing,
            (Self::Composing | Self::Ending, true) => Self::Ending,
        };
        held || *self != Self::Nothing
    }

    /// Whether committed text is dropped: the dropped composition's, which
    /// ends with it, or any while the host holds the keyboard (`held`).
    fn commit(&mut self, held: bool) -> bool {
        let dropped = held || *self != Self::Nothing;
        *self = Self::Nothing;
        dropped
    }

    /// A key or a button was pressed: a composition that had ended
    /// committed nothing.
    fn pressed(&mut self) {
        if *self == Self::Ending {
            *self = Self::Nothing;
        }
    }
}

/// Whether a key's binding is input of the editor's own that ends a
/// handback ([`crate::EditorView::hand_back`]): one that types, edits or
/// moves the caret, as the messages it publishes end it in the editor.
fn ends_handback(binding: &Binding<Message>) -> bool {
    match binding {
        Binding::Unfocus | Binding::Copy => false,
        Binding::Sequence(bindings) => bindings.iter().any(ends_handback),
        Binding::Custom(message) => matches!(
            message,
            Message::Action(_)
                | Message::Intent(_)
                | Message::Undo
                | Message::Redo
                | Message::Clip(crate::Clip::CutLine(_) | crate::Clip::Paste(_))
        ),
        _ => true,
    }
}

/// What a menu's width and rows were fitted to.
#[derive(Debug, Clone, PartialEq)]
struct FitKey {
    items: Vec<String>,
    font: Font,
    text_size: u32,
    width: u32,
    /// The room the rows take beside their labels ([`popover::Extras`]).
    extra: u32,
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
            dropped: Dropped::Nothing,
            last_click: None,
            click_kind: None,
            resolved_seen: 0,
            drag_click: None,
            box_press: None,
            box_cycle: None,
            modifiers: keyboard::Modifiers::empty(),
            partial_scroll: 0.0,
            highlighter: RefCell::new(MarkdownHighlighter::new(&highlighter_settings)),
            // Layout measures again when the text size differs.
            advance: Advance::measure(highlighter_settings.font, TEXT_SIZE),
            highlighter_settings,
            menu_fit: None,
            was_focused: false,
            menu_look: None,
            zigzags: RefCell::new(Zigzags::default()),
            handed_by_pointer: Rc::default(),
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
        self.box_press = None;
        // Preedit events reach only a focused editor, so one that ends while
        // another field has the keyboard would leave the guard on.
        self.preedit = None;
        self.dropped = Dropped::Nothing;
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
        let bounds = layout.bounds() + translation;
        let handed = Rc::clone(&state.handed_by_pointer);
        let ends = this.ending(
            state.focus.is_some(),
            state.is_composing(),
            state.partial_scroll,
            bounds,
        );
        let menu = this.menu_overlay(state, bounds)?;
        Some(overlay::Element::new(Box::new(popover::InOrder::new(
            menu,
            &this.menu_now,
            handed,
            ends,
        ))))
    }
}

impl<'a> From<TextEditor<'a>> for Element<'a, Message> {
    fn from(text_editor: TextEditor<'a>) -> Self {
        Self::new(text_editor)
    }
}

/// The lines a wheel event scrolls, in quarters of a line for a trackpad's
/// pixels; a wheel's line delta moves at least one line.
fn wheel_lines(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => {
            if y.abs() > 0.0 {
                y.signum() * -(y.abs() * 4.0).max(1.0)
            } else {
                0.0
            }
        }
        mouse::ScrollDelta::Pixels { y, .. } => -y / 4.0,
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

/// What an event is routed against: what is open, as the widget's events
/// have left it.
#[derive(Clone, Copy)]
struct Routes<'a> {
    /// For a press of the pointer.
    presses: Option<Target<'a>>,
    /// For a key: the same, but for the rest of a batch in which the host
    /// took the keyboard, whose keys are dropped whatever has opened or
    /// closed since.
    keys: Option<Target<'a>>,
}

enum Update {
    Click(mouse::Click),
    /// A left press on the box of the task on this line; see
    /// [`crate::BoxClick`].
    BoxPress(mouse::Click, usize),
    /// A second press on a box, a click on which waits, that makes a double
    /// click.
    BoxDouble(mouse::Click),
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
        routes: Routes<'_>,
    ) -> Option<Self> {
        // Only a menu takes the editor's presses, or what may be one;
        // suggestions leave them to the text.
        let menu = routes.presses.and_then(Target::modal);
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
                    Some(Update::Scroll(wheel_lines(*delta)))
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
                    routes.keys,
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
    /// The byte in the line at which the glyph under the point starts;
    /// `None` where the row has no glyph there. Under a misspelled word
    /// the word is the hit ([`crate::hit`]), whatever its glyph is.
    pub column: Option<usize>,
}

/// Where `point`, relative to the text origin, falls: the visible row
/// holding it, and on that row
///
/// - the first of the marks `marks` gives for the row's line whose glyphs
///   on the row span `point`'s x, and
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
    let column = glyphs()
        .find(|(_, x, width)| point.x >= *x && point.x < x + width)
        .map(|(bytes, _, _)| bytes.start);
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
                .resolved_press(ev.resolved_press)
                .handback(ev.handback);
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
                Message::HandbackOver => {
                    ev.handback_over();
                    false
                }
                Message::Command(_) | Message::BoxClick(_) => false,
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

    use crate::fixtures::{status_rows, DIALOG, KEYBOARD, STATUS};
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
                keyboard::Key::Character("d") => crate::KeyRoute::Popover(DIALOG),
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
        /// The row chosen from a provider's menu, with the provider.
        provided: Option<(usize, Choice)>,
        /// The right-click a provider took the keyboard for.
        keyboard: Option<(usize, Clicked)>,
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
                applied.provided = ev.take_provided();
                applied.keyboard = ev.take_keyboard();
                changed
            }
            Message::Clip(clip) => ev.clip(clip),
            Message::Undo => ev.undo(),
            Message::Redo => ev.redo(),
            Message::HandbackOver => {
                ev.handback_over();
                false
            }
            Message::Command(_) | Message::BoxClick(_) => false,
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
            .menu_source(ev.widget_menu_source())
            .providers(ev.providers(), ev.edition())
            .suggest_source(ev.suggest_source(), ev.suggest_suppressed())
            .origin(&ev.origin)
            .resolved_press(ev.resolved_press)
            .handback(ev.handback);
        // The host's tasks, as the editor's view hands them to the widget.
        editor.highlighter_settings.tasks = ev.tasks;
        let state = state.get_or_insert_with(|| {
            let mut state = laid_out_state(&editor);
            state.focus = Some(Focus::now());
            state
        });
        editor.lay_out(state, BOUNDS.size());
        let mut held = vec![false; events.len()];
        let (focused, composing) = (state.focus.is_some(), state.is_composing());
        let scroll = state.partial_scroll;
        let handed = Rc::clone(&state.handed_by_pointer);
        if let Some(overlay) = editor.menu_overlay(state, BOUNDS) {
            let ends = editor.ending(focused, composing, scroll, BOUNDS);
            let mut overlay = popover::InOrder::new(overlay, &editor.menu_now, handed, ends);
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

    /// A clipboard that keeps what is written to it.
    #[derive(Default)]
    struct Kept(Option<String>);

    impl Clipboard for Kept {
        fn read(&self, kind: clipboard::Kind) -> Option<String> {
            self.0.clone().filter(|_| kind == clipboard::Kind::Standard)
        }

        fn write(&mut self, kind: clipboard::Kind, contents: String) {
            if kind == clipboard::Kind::Standard {
                self.0 = Some(contents);
            }
        }
    }

    #[test]
    fn the_host_s_edit_keys_act_through_the_widget_on_both_engines() {
        use crate::{EditKey, KeyRoute};
        // A host whose keymap puts the editing keys on Alt, as Wren's
        // Linux keymap does with `linux-modifier = alt`.
        let policy: crate::KeyPolicy = Arc::new(|key: &keyboard::Key, _, modifiers| {
            let alt = keyboard::Modifiers::ALT;
            let edit = match (key.as_ref(), modifiers) {
                (keyboard::Key::Character("z"), m) if m == alt => EditKey::Undo,
                (keyboard::Key::Character("z"), m) if m == alt | keyboard::Modifiers::SHIFT => {
                    EditKey::Redo
                }
                (keyboard::Key::Character("c"), m) if m == alt => EditKey::Copy,
                (keyboard::Key::Character("x"), m) if m == alt => EditKey::Cut,
                (keyboard::Key::Character("v"), m) if m == alt => EditKey::Paste,
                (keyboard::Key::Character("a"), m) if m == alt => EditKey::SelectAll,
                _ => return KeyRoute::Pass,
            };
            KeyRoute::Edit(edit)
        });
        for kind in BackendKind::ALL {
            let mut ev = editing(kind, "cat teh", 0, 2);
            ev.set_key_policy(Arc::clone(&policy));
            let mut kept = Kept::default();
            // Each press to a focused widget over the editor as it stands,
            // what it published carried out as the host does.
            let press = |ev: &mut crate::EditorView, kept: &mut Kept, letter: &str, modifiers| {
                let mut editor = editor_with(&ev.content, true, Measure::default())
                    .key_policy(ev.key_policy.as_ref());
                let mut state = laid_out_state(&editor);
                state.focus = Some(Focus::now());
                let mut messages = Vec::new();
                let mut shell = Shell::new(&mut messages);
                editor.on_event(
                    &mut state,
                    &key(keyboard::Key::Character(letter.into()), modifiers, None),
                    BOUNDS,
                    mouse::Cursor::Unavailable,
                    kept,
                    &mut shell,
                );
                let captured = shell.event_status() == iced::event::Status::Captured;
                for message in messages.clone() {
                    host_applies(ev, message);
                }
                (messages, captured)
            };
            let alt = keyboard::Modifiers::ALT;
            let (messages, captured) = press(&mut ev, &mut kept, "a", alt);
            assert!(
                matches!(messages.as_slice(), [Message::Action(Action::SelectAll)]),
                "{kind:?}: {messages:?}"
            );
            assert!(captured, "{kind:?}");
            assert_eq!(ev.selected_text().as_deref(), Some("cat teh"), "{kind:?}");
            let (messages, _) = press(&mut ev, &mut kept, "c", alt);
            assert!(
                matches!(messages.as_slice(), [Message::Clip(crate::Clip::Selection)]),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(kept.0.as_deref(), Some("cat teh"), "{kind:?}");
            assert_eq!(ev.text(), "cat teh", "{kind:?}");
            press(&mut ev, &mut kept, "x", alt);
            assert_eq!(
                (ev.text().as_str(), kept.0.as_deref()),
                ("", Some("cat teh"))
            );
            press(&mut ev, &mut kept, "v", alt);
            press(&mut ev, &mut kept, "v", alt);
            assert_eq!(ev.text(), "cat tehcat teh", "{kind:?}");
            let (messages, _) = press(&mut ev, &mut kept, "z", alt);
            assert!(
                matches!(messages.as_slice(), [Message::Undo]),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(ev.text(), "cat teh", "{kind:?}");
            press(&mut ev, &mut kept, "z", alt | keyboard::Modifiers::SHIFT);
            assert_eq!(ev.text(), "cat tehcat teh", "{kind:?}");
            // Off macOS, Ctrl+Y and Ctrl+Z are nothing of the editor's own.
            if !cfg!(target_os = "macos") {
                for letter in ["y", "z"] {
                    let (messages, _) =
                        press(&mut ev, &mut kept, letter, keyboard::Modifiers::CTRL);
                    assert!(messages.is_empty(), "{kind:?} {letter}: {messages:?}");
                }
                assert_eq!(ev.text(), "cat tehcat teh", "{kind:?}");
            }
        }
    }

    #[test]
    fn alt_graph_text_replaces_a_selection_and_is_never_a_clipboard_chord() {
        use crate::{EditKey, KeyRoute};
        // Off macOS AltGr can arrive as Ctrl+Alt with its text, which iced
        // alone would read as Cut, Copy or Paste first.
        if cfg!(target_os = "macos") {
            return;
        }
        // A host that binds Ctrl+Alt+X to Cut and Ctrl+Alt+V to its own.
        let host: crate::KeyPolicy = Arc::new(|key: &keyboard::Key, _, modifiers| {
            let chord = keyboard::Modifiers::CTRL | keyboard::Modifiers::ALT;
            match key.as_ref() {
                keyboard::Key::Character("x") if modifiers == chord => KeyRoute::Edit(EditKey::Cut),
                keyboard::Key::Character("v") if modifiers == chord => KeyRoute::Host,
                _ => KeyRoute::Pass,
            }
        });
        for kind in BackendKind::ALL {
            for policy in [None, Some(&host)] {
                for (letter, typed) in [("x", "»"), ("c", "¢"), ("v", "“"), ("a", "æ")] {
                    let mut ev = editing(kind, "cat teh", 0, 0);
                    ev.perform(Action::SelectAll);
                    let mut editor =
                        editor_with(&ev.content, true, Measure::default()).key_policy(policy);
                    let mut state = laid_out_state(&editor);
                    state.focus = Some(Focus::now());
                    let mut kept = Kept(Some("sentinel".into()));
                    let mut messages = Vec::new();
                    let mut shell = Shell::new(&mut messages);
                    editor.on_event(
                        &mut state,
                        &Event::Keyboard(keyboard::Event::KeyPressed {
                            key: keyboard::Key::Character(letter.into()),
                            modified_key: keyboard::Key::Character(typed.into()),
                            physical_key: keyboard::key::Physical::Unidentified(
                                keyboard::key::NativeCode::Unidentified,
                            ),
                            location: keyboard::Location::Standard,
                            modifiers: keyboard::Modifiers::CTRL | keyboard::Modifiers::ALT,
                            text: Some(typed.into()),
                            repeat: false,
                        }),
                        BOUNDS,
                        mouse::Cursor::Unavailable,
                        &mut kept,
                        &mut shell,
                    );
                    for message in messages {
                        host_applies(&mut ev, message);
                    }
                    assert_eq!(ev.text(), typed, "{kind:?} {letter}");
                    assert_eq!(kept.0.as_deref(), Some("sentinel"), "{kind:?} {letter}");
                }
            }
        }
    }

    #[test]
    fn a_clipboard_key_the_host_takes_does_nothing_in_the_text() {
        use crate::KeyRoute;
        // A host whose user unbound Cmd/Ctrl+V: the key is the host's, so
        // iced's own Paste never runs for it.
        let host: crate::KeyPolicy = Arc::new(|key: &keyboard::Key, _, modifiers| {
            if modifiers == keyboard::Modifiers::COMMAND
                && *key == keyboard::Key::Character("v".into())
            {
                KeyRoute::Host
            } else {
                KeyRoute::Pass
            }
        });
        for kind in BackendKind::ALL {
            for (policy, pastes) in [(Some(&host), false), (None, true)] {
                let mut ev = editing(kind, "cat", 0, 3);
                let mut editor =
                    editor_with(&ev.content, true, Measure::default()).key_policy(policy);
                let mut state = laid_out_state(&editor);
                state.focus = Some(Focus::now());
                let mut kept = Kept(Some("clip".into()));
                let mut messages = Vec::new();
                let mut shell = Shell::new(&mut messages);
                editor.on_event(
                    &mut state,
                    &key(
                        keyboard::Key::Character("v".into()),
                        keyboard::Modifiers::COMMAND,
                        None,
                    ),
                    BOUNDS,
                    mouse::Cursor::Unavailable,
                    &mut kept,
                    &mut shell,
                );
                assert_eq!(
                    matches!(messages.as_slice(), [Message::Clip(crate::Clip::Paste(_))]),
                    pastes,
                    "{kind:?}: {messages:?}"
                );
                for message in messages {
                    host_applies(&mut ev, message);
                }
                let text = if pastes { "catclip" } else { "cat" };
                assert_eq!(ev.text(), text, "{kind:?}");
            }
        }
    }

    #[test]
    fn a_menu_takes_up_down_enter_and_its_accelerators() {
        use keyboard::key::Named;
        for kind in BackendKind::ALL {
            // Keys of one batch see the menu as the earlier ones left it.
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            // A choice names the menu (item 3.4).
            let menu = ev.popover().map(crate::Popover::instance).unwrap();
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
                    SpellEvent::AcceptOf { menu, row: None }
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
                let menu = ev.popover().map(crate::Popover::instance).unwrap();
                let (messages, captured, choice) =
                    keys_batch(&mut ev, vec![named(Named::ArrowDown), event]);
                assert_eq!(
                    spell_events(&messages)[1],
                    SpellEvent::AcceptOf {
                        menu,
                        row: Some(id as usize - 20)
                    },
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
                        Message::Spell(SpellEvent::AcceptOf { row: Some(2), .. }),
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
            let menu = ev.popover().map(crate::Popover::instance).unwrap();
            let row = on_task_row(1);
            let (messages, captured, choice) = popover_batch(
                &mut ev,
                &mut None,
                vec![(moved_to(row), Some(row)), (left(), Some(row))],
            );
            assert_eq!(
                spell_events(&messages),
                [
                    SpellEvent::Hover(1),
                    SpellEvent::AcceptOf { menu, row: Some(1) }
                ],
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

    /// An editor over the task line `text`, the caret at its end, whose
    /// host offers task suggestions ([`fixtures::TaskWords`]).
    fn task_words(text: &str) -> crate::EditorView {
        let mut ev = editing(BackendKind::Owned, text, 0, text.len());
        ev.set_suggest_source(Some(Arc::new(fixtures::TaskWords)));
        ev
    }

    /// The keys that type `text`.
    fn typing(text: &str) -> Vec<Event> {
        text.chars()
            .map(|c| {
                if c == ' ' {
                    space()
                } else {
                    typed(&c.to_string())
                }
            })
            .collect()
    }

    /// The Enter table (item 5.1): Enter at the end of a task line after
    /// a word that is suggestions' and may be ordinary writing continues
    /// the list, and the word stays, whether the suggestions were drawn
    /// before Enter or opened in its batch, as typing at xdotool speed
    /// opens them.
    #[test]
    fn enter_after_an_ordinary_word_continues_the_list() {
        use keyboard::key::Named;
        let line = "- [ ] Meet sam";
        for word in [
            "to", "hi", "lo", "me", "fr", "sa", "due", "today", "@sa", "@sam",
        ] {
            let expected = format!("{line} {word}\n- [ ] ");
            // In one batch with the typing.
            let mut ev = task_words(line);
            let mut events = typing(&format!(" {word}"));
            events.push(named(Named::Enter));
            let (messages, captured, choice) = keys_batch(&mut ev, events);
            assert_eq!(choice, None, "{word}");
            assert!(
                matches!(messages.last(), Some(Message::Intent(Intent::Enter))),
                "{word}: {messages:?}"
            );
            assert!(captured.iter().all(|captured| *captured), "{word}");
            assert_eq!(ev.text(), expected, "{word}");
            assert!(ev.popover().is_none(), "{word}");
            // In a batch of its own, the suggestions open and drawn.
            let mut ev = task_words(line);
            let mut state = None;
            let typed = typing(&format!(" {word}"));
            popover_batch(
                &mut ev,
                &mut state,
                typed.into_iter().map(|e| (e, None)).collect(),
            );
            assert!(ev.popover().is_some(), "{word}: something is offered");
            assert!(!ev.popover().unwrap().takes_enter(), "{word}");
            let (messages, captured, choice) =
                popover_batch(&mut ev, &mut state, vec![(named(Named::Enter), None)]);
            assert_eq!(choice, None, "{word}");
            assert!(
                matches!(messages.as_slice(), [Message::Intent(Intent::Enter)]),
                "{word}: {messages:?}"
            );
            assert_eq!(captured, [true], "{word}");
            assert_eq!(ev.text(), expected, "{word}");
            // The host's own resolution of a popover's Enter agrees.
            let mut ev = task_words(line);
            popover_batch(
                &mut ev,
                &mut None,
                typing(&format!(" {word}"))
                    .into_iter()
                    .map(|e| (e, None))
                    .collect(),
            );
            ev.spell(SpellEvent::Key(MenuKey::Enter));
            assert_eq!(ev.take_choice(), None, "{word}");
            assert_eq!(ev.text(), expected, "{word}");
        }
    }

    /// The Enter table's edge rows: Up on the first row and Down on the
    /// last move nothing, so they pick nothing, and Enter after them still
    /// continues the list.
    #[test]
    fn a_move_that_moves_nothing_picks_nothing() {
        use keyboard::key::Named;
        let line = "- [ ] Meet sam";
        for (word, key) in [
            ("to", Named::ArrowUp),
            ("fr", Named::ArrowDown),
            ("sa", Named::ArrowUp),
        ] {
            let expected = format!("{line} {word}\n- [ ] ");
            let mut ev = task_words(line);
            let mut events = typing(&format!(" {word}"));
            events.extend([named(key), named(Named::Enter)]);
            let (messages, _, choice) = keys_batch(&mut ev, events);
            assert_eq!(choice, None, "{word} {key:?}: {messages:?}");
            assert_eq!(ev.text(), expected, "{word} {key:?}");
            // Across batches.
            let mut ev = task_words(line);
            let mut state = None;
            let typed = typing(&format!(" {word}"))
                .into_iter()
                .map(|e| (e, None))
                .collect();
            popover_batch(&mut ev, &mut state, typed);
            popover_batch(&mut ev, &mut state, vec![(named(key), None)]);
            assert!(!ev.popover().unwrap().takes_enter(), "{word} {key:?}");
            let (_, _, choice) =
                popover_batch(&mut ev, &mut state, vec![(named(Named::Enter), None)]);
            assert_eq!(choice, None, "{word} {key:?}");
            assert_eq!(ev.text(), expected, "{word} {key:?}");
        }
    }

    /// A deletion the widget does not follow, then Enter, in one batch:
    /// whatever the host offers for the word the deletion leaves (`du`'s
    /// "Due date…" here), Enter is the text's, as 3.1 routes every key
    /// after such a deletion, so nothing is chosen and nothing is left
    /// open: the key that acts closes what the host opened.
    #[test]
    fn backspace_then_enter_in_one_batch_continues_the_list() {
        use keyboard::key::Named;
        for (line, keys, expected) in [
            (
                "- [ ] Mail the deck due",
                vec![Named::Backspace, Named::Enter],
                "- [ ] Mail the deck du\n- [ ] ",
            ),
            (
                "- [ ] Mail the deck tod",
                vec![Named::Backspace, Named::Tab],
                "- [ ] Mail the deck to",
            ),
            (
                "- [ ] Mail the deck tod",
                vec![Named::Backspace, Named::ArrowUp, Named::Enter],
                "- [ ] Mail the deck to\n- [ ] ",
            ),
        ] {
            let mut ev = task_words(line);
            let events = keys.iter().map(|key| named(*key)).collect();
            let (messages, _, choice) = keys_batch(&mut ev, events);
            assert_eq!(choice, None, "{keys:?}: {messages:?}");
            assert!(ev.popover().is_none(), "{keys:?}: {messages:?}");
            // Tab is the text's too, which indents no first item.
            assert_eq!(ev.text(), expected, "{keys:?}: {messages:?}");
        }
    }

    #[test]
    fn tab_or_a_move_then_enter_choose_a_suggestion_of_an_ordinary_word() {
        use keyboard::key::Named;
        let line = "- [ ] Meet sam";
        let to = 15..17;
        // In the typing's batch, Tab is dropped (ring review, third round),
        // and chooses in the next.
        let mut ev = task_words(line);
        let mut state = None;
        let mut events: Vec<_> = typing(" to").into_iter().map(|e| (e, None)).collect();
        events.push((named(Named::Tab), None));
        let (_, _, choice) = popover_batch(&mut ev, &mut state, events);
        assert_eq!((choice, ev.text()), (None, format!("{line} to")));
        let (_, _, choice) = popover_batch(&mut ev, &mut state, vec![(named(Named::Tab), None)]);
        assert_eq!(choice, suggested(0, to.clone()));
        // Down then Enter, in one batch and across two.
        let mut ev = task_words(line);
        let mut events = typing(" to");
        events.extend([named(Named::ArrowDown), named(Named::Enter)]);
        let (_, _, choice) = keys_batch(&mut ev, events);
        // In the typing's batch, before the host has shown them, Enter is
        // the text's, whatever the move (ring review, third round).
        assert_eq!((choice, ev.text()), (None, format!("{line} to\n- [ ] ")));
        let mut ev = task_words(line);
        let mut state = None;
        let mut events: Vec<_> = typing(" to").into_iter().map(|e| (e, None)).collect();
        events.push((named(Named::ArrowDown), None));
        popover_batch(&mut ev, &mut state, events);
        assert!(ev.popover().unwrap().takes_enter());
        let (_, _, choice) =
            popover_batch(&mut ev, &mut state, vec![(named(Named::ArrowUp), None)]);
        assert_eq!(choice, None);
        let (_, _, choice) = popover_batch(&mut ev, &mut state, vec![(named(Named::Enter), None)]);
        assert_eq!(choice, suggested(0, to));
        // Typing on after a move offers afresh, and Enter is the text's.
        let mut ev = task_words(line);
        let mut events = typing(" t");
        events.extend([named(Named::ArrowDown)]);
        events.extend(typing("o"));
        events.push(named(Named::Enter));
        let (_, _, choice) = keys_batch(&mut ev, events);
        assert_eq!(choice, None);
        assert_eq!(ev.text(), format!("{line} to\n- [ ] "));
    }

    /// `du` and Enter at xdotool speed, or with `du` shown first: "Due
    /// date…" is chosen, and since it opens a menu of the host's (the
    /// calendar), what follows in the batch is dropped, never typed into
    /// the note. An owner is chosen by Tab once shown; Tab in the typing's
    /// own batch is dropped (ring review, third round).
    #[test]
    fn enter_chooses_the_calendar_for_du_and_tab_an_owner() {
        use keyboard::key::Named;
        let line = "- [ ] Mail the deck";
        let typed = |text: &str| -> Vec<(Event, Option<Point>)> {
            typing(text).into_iter().map(|e| (e, None)).collect()
        };
        let mut ev = task_words(line);
        let mut events = typing(" du");
        events.push(named(Named::Enter));
        events.extend(typing("fri"));
        let (messages, _, choice) = keys_batch(&mut ev, events);
        assert_eq!(choice, suggested(0, 20..22), "{messages:?}");
        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Message::Spell(SpellEvent::EnterOn { id: 0 }))),
            "{messages:?}"
        );
        assert_eq!(ev.text(), format!("{line} du"));
        let mut ev = task_words(line);
        let mut state = None;
        popover_batch(&mut ev, &mut state, typed(" du"));
        let mut events = vec![(named(Named::Enter), None)];
        events.extend(typed("fri"));
        let (messages, _, choice) = popover_batch(&mut ev, &mut state, events);
        assert_eq!(choice, suggested(0, 20..22), "{messages:?}");
        assert_eq!(ev.text(), format!("{line} du"));
        let mut ev = task_words(line);
        let mut state = None;
        popover_batch(&mut ev, &mut state, typed(" @sa"));
        let (_, _, choice) = popover_batch(&mut ev, &mut state, vec![(named(Named::Tab), None)]);
        assert_eq!(choice, suggested(0, 20..23));
        assert_eq!(ev.text(), format!("{line} @sa"));
        // Tab in the typing's batch: dropped.
        for word in [" du", " @sa"] {
            let mut ev = task_words(line);
            let mut events = typing(word);
            events.push(named(Named::Tab));
            let (_, _, choice) = keys_batch(&mut ev, events);
            assert_eq!(
                (choice, ev.text()),
                (None, format!("{line}{word}")),
                "{word}"
            );
        }
    }

    /// `du`, Enter and Escape in one batch: Enter chooses "Due date…",
    /// whose calendar the host opens, and the Escape after it, typed
    /// before the calendar could be seen, is dropped and claimed, so the
    /// host's own key handling drops it too, and the calendar stays.
    #[test]
    fn escape_after_choosing_the_calendar_in_one_batch_is_dropped() {
        use keyboard::key::Named;
        let line = "- [ ] Mail the deck";
        let mut ev = task_words(line);
        let mut events = typing(" du");
        events.extend([named(Named::Enter), named(Named::Escape), typed("x")]);
        let (messages, captured, choice) = keys_batch(&mut ev, events);
        assert_eq!(choice, suggested(0, 20..22), "{messages:?}");
        assert!(
            matches!(
                messages.last(),
                Some(Message::Spell(SpellEvent::Stroke(stroke)))
                    if stroke.named() == Some(Named::Escape)
            ),
            "{messages:?}"
        );
        assert_eq!(captured[captured.len() - 2..], [true, true]);
        assert_eq!(ev.text(), format!("{line} du"));
        // The host hands the editor the stroke, which claims the Escape.
        if let Some(Message::Spell(event)) = messages.last() {
            ev.spell(*event);
        }
        assert!(ev.take_claimed_escape());
        assert!(!ev.take_claimed_escape());
        // After a choice that opens nothing, Escape stays the host's, as it
        // was.
        let mut ev = task_words(line);
        let mut state = None;
        let shown = typing(" to").into_iter().map(|e| (e, None)).collect();
        popover_batch(&mut ev, &mut state, shown);
        let events = [named(Named::Tab), named(Named::Escape)]
            .into_iter()
            .map(|e| (e, None))
            .collect();
        let (messages, _, _) = popover_batch(&mut ev, &mut state, events);
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, Message::Spell(SpellEvent::Stroke(_)))),
            "{messages:?}"
        );
        assert!(!ev.take_claimed_escape());
    }

    /// Composing never opens suggestions: what an input method commits is
    /// not typed key by key, and the source is not asked about it.
    #[test]
    fn an_input_method_composing_a_word_never_opens_suggestions() {
        use keyboard::key::Named;
        let line = "- [ ] Mail the deck ";
        // As winit sends a composition: its preedits, an empty one, and
        // the text committed.
        for events in [
            vec![
                composing("d"),
                composing("du"),
                composing(""),
                committing("du"),
            ],
            vec![
                composing("t"),
                composing("to"),
                composing(""),
                committing("to"),
            ],
            vec![composing("ｔｏ"), composing(""), committing("to")],
        ] {
            let mut ev = task_words(line);
            let mut state = None;
            popover_batch(&mut ev, &mut state, events.clone());
            assert!(ev.popover().is_none(), "{events:?}");
            let (messages, _, choice) =
                popover_batch(&mut ev, &mut state, vec![(named(Named::Enter), None)]);
            assert_eq!(choice, None);
            assert!(
                matches!(messages.as_slice(), [Message::Intent(Intent::Enter)]),
                "{messages:?}"
            );
            assert!(ev.text().ends_with("\n- [ ] "), "{:?}", ev.text());
            // And with the Enter in the composition's batch.
            let mut ev = task_words(line);
            let mut events = events.clone();
            events.push((named(Named::Enter), None));
            let (_, _, choice) = popover_batch(&mut ev, &mut None, events);
            assert_eq!(choice, None);
            assert!(ev.popover().is_none());
        }
    }

    /// Typing that opens suggestions, then their keys, in one batch: the
    /// widget asks the source what the typing opens, as the host will, so
    /// Up and Down are theirs. On suggestions the host has not shown, Tab
    /// is dropped and Enter is the text's, choosing nothing (ring review,
    /// third round); in the next batch both choose.
    #[test]
    fn on_suggestions_the_batch_opened_tab_is_dropped_and_enter_is_the_texts() {
        use keyboard::key::Named;
        let batch = |events: Vec<Event>| -> Vec<(Event, Option<Point>)> {
            events.into_iter().map(|e| (e, None)).collect()
        };
        // Tab: dropped, and theirs in the next batch.
        let mut ev = completing("", 0, 0);
        let mut state = None;
        let events = batch(vec![typed("d"), typed("u"), named(Named::Tab)]);
        let (messages, captured, choice) = popover_batch(&mut ev, &mut state, events);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Action(Action::Edit(Edit::Insert('d'))),
                    Message::Action(Action::Edit(Edit::Insert('u'))),
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(captured, [true; 3]);
        assert_eq!((choice, ev.text().as_str()), (None, "du"));
        let (_, _, choice) = popover_batch(&mut ev, &mut state, batch(vec![named(Named::Tab)]));
        assert_eq!(choice, suggested(0, 0..2));
        assert_eq!((ev.text().as_str(), ev.cursor()), ("du", caret(0, 2)));

        // Enter, after a move too: the text's, as with nothing open.
        for keys in [vec![], vec![named(Named::ArrowDown)]] {
            let mut ev = completing("", 0, 0);
            let mut events = vec![typed("d"), typed("u")];
            events.extend(keys.clone());
            events.push(named(Named::Enter));
            let (messages, _, choice) = keys_batch(&mut ev, events);
            assert!(
                matches!(messages.last(), Some(Message::Intent(Intent::Enter))),
                "{keys:?}: {messages:?}"
            );
            assert_eq!((choice, ev.text().as_str()), (None, "du\n"), "{keys:?}");
        }
        // Shown, Enter chooses the selected row.
        let mut ev = completing("", 0, 0);
        let mut state = None;
        popover_batch(&mut ev, &mut state, batch(vec![typed("d"), typed("u")]));
        let (_, _, choice) = popover_batch(&mut ev, &mut state, batch(vec![named(Named::Enter)]));
        assert_eq!(choice, suggested(0, 0..2));

        // A letter that matches nothing closes them, so Enter after it is
        // the text's.
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

    /// Enter on "Due date…" the widget only predicted names that row by its
    /// id ([`SpellEvent::EnterOn`]): the host chooses it if its suggestions
    /// have it selected, and otherwise Enter is the text's and chooses
    /// nothing else (ring review, third round).
    #[test]
    fn enter_on_a_predicted_menu_row_chooses_only_that_row() {
        let line = "- [ ] Mail the deck du";
        let at_end = |ev: &mut crate::EditorView| {
            let end = ev.line_text(0).unwrap().len();
            ev.move_cursor(caret(0, end));
        };
        // The host's suggestions have it selected: it is chosen.
        let mut ev = editing(BackendKind::Owned, "", 0, 0);
        ev.set_suggest_source(Some(Arc::new(fixtures::TaskWords)));
        ev.replace_text(&line[..line.len() - 1], true);
        at_end(&mut ev);
        ev.perform(Action::Edit(Edit::Insert('u')));
        ev.spell(SpellEvent::EnterOn { id: 0 });
        assert_eq!(ev.text(), line);
        assert_eq!(ev.take_choice().map(|choice| choice.id), Some(0));
        // Another id, or suggestions without that row: Enter is the
        // text's, and nothing is chosen.
        for (id, rows) in [
            (5, vec![Row::new(0, "Due date…").opens_menu()]),
            (0, vec![Row::new(0, "due")]),
        ] {
            let mut ev = editing(BackendKind::Owned, line, 0, line.len());
            ev.set_tasks(Some(wren_markdown::TaskOptions {
                today: wren_markdown::date::Date::new(2026, 11, 19).expect("a date"),
            }));
            assert!(ev.open_popover(crate::Anchor::Caret, Mode::Suggest, rows));
            assert!(ev.spell(SpellEvent::EnterOn { id }), "{id}");
            assert_eq!(ev.take_choice(), None, "{id}");
            assert_eq!(ev.text(), format!("{line}\n- [ ] "), "{id}");
            assert!(ev.popover().is_none(), "{id}");
        }
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
        // The menu the editor will open next, which the choice names
        // (item 3.4).
        assert_eq!(
            spell_events(&messages),
            [
                SpellEvent::Request(STATUS),
                SpellEvent::Move(1),
                SpellEvent::AcceptOf {
                    menu: 1,
                    row: Some(2)
                }
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
            [
                SpellEvent::Request(STATUS),
                SpellEvent::AcceptOf {
                    menu: 1,
                    row: Some(1)
                }
            ]
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
    fn the_keys_after_a_request_the_host_takes_the_keyboard_for_are_dropped() {
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
        // Each key after the request is taken, so that it is no one
        // else's, and nothing is published for it: the host's field has
        // the keyboard from the next batch on, and the editor does
        // nothing with any of them.
        assert_eq!(captured, [true; 6]);
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Spell(SpellEvent::Request(KEYBOARD))]
            ),
            "{messages:?}"
        );
        for message in messages {
            host_applies(&mut ev, message);
        }
        assert_eq!((ev.text().as_str(), ev.cursor()), (TASK, caret(0, 8)));
        assert!(ev.popover().is_none());

        // Nothing of the editor's is open: Escape is the host's alone, and
        // a press in the editor is the click it is. The keys after it are
        // dropped all the same, to the end of the batch.
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, captured, _) =
            keys_batch(&mut ev, vec![request_key("k"), named(Named::Escape)]);
        assert_eq!(spell_events(&messages), [SpellEvent::Request(KEYBOARD)]);
        assert_eq!(captured, [true, false]);
        assert!(!ev.take_claimed_escape());
        let mut ev = requesting(BackendKind::Owned, TASK, 0, 8);
        let (messages, captured, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (request_key("k"), None),
                (press(mouse::Button::Left), Some(over(1.0, 0.0))),
                (typed("x"), None),
            ],
        );
        assert!(
            matches!(&messages[1..], [Message::Action(Action::Click(_))]),
            "{messages:?}"
        );
        assert_eq!(captured, [true; 3]);
        assert_eq!((ev.text().as_str(), ev.cursor()), (TASK, caret(0, 1)));
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
        // which is selected, and the press on the menu as drawn is dropped
        // there, never a click in the text (ring review, third round).
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
                [Message::Spell(SpellEvent::AcceptSelected)]
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

        // Any key before the press, Down too: the press chooses no row and
        // is dropped on the menu, and the key moves the selection of the
        // menu it was pressed on (ring review, third round).
        let mut ev = spelled_menu();
        let (messages, captured, _) = popover_batch(
            &mut ev,
            &mut None,
            vec![
                (named(Named::ArrowDown), None),
                (left(), Some(on_spelled_row(1))),
            ],
        );
        assert!(
            matches!(messages.as_slice(), [Message::Spell(SpellEvent::Move(1))]),
            "{messages:?}"
        );
        assert_eq!(captured, [true, true]);
        assert_eq!(ev.text(), "cat teh");
        assert_eq!(ev.spell_menu().and_then(|menu| menu.selected()), Some(1));

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
        // Any other key before the press, even one the editor ignores, and
        // the press chooses no row: it is dropped on the popover (ring
        // review, third round).
        for kind in BackendKind::ALL {
            let mut ev = on_task(kind, Mode::Suggest, status_rows());
            let (messages, captured, choice) = popover_batch(
                &mut ev,
                &mut None,
                vec![(named(Named::F5), None), (left(), Some(on_task_row(1)))],
            );
            assert!(messages.is_empty(), "{kind:?}: {messages:?}");
            assert_eq!((captured[1], choice), (true, None), "{kind:?}");
            assert!(ev.popover().is_some(), "{kind:?}");
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

        // Open when the batch starts: Tab is theirs, and captured, so no
        // host hears of it as its own. Opened by the letters of the batch,
        // which the host has not shown: Tab is dropped, and captured all
        // the same (ring review, third round).
        let mut ev = hosted();
        let mut state = None;
        keys(&mut ev, &mut state, vec![typed("d"), typed("u")]);
        let (messages, captured, choice) = keys(&mut ev, &mut state, vec![named(Named::Tab)]);
        assert_eq!(spell_events(&messages), [SpellEvent::Key(MenuKey::Tab)]);
        assert_eq!((captured, choice), (vec![true], due));
        let mut ev = hosted();
        let (messages, captured, choice) = keys(
            &mut ev,
            &mut None,
            vec![typed("d"), typed("u"), named(Named::Tab)],
        );
        assert!(spell_events(&messages).is_empty(), "{messages:?}");
        assert_eq!((captured, choice), (vec![true; 3], None));
        assert_eq!(ev.text(), "- a\n- du");

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

    // ── A right-click's providers ──────────────────────────────────────────

    use crate::fixtures::{
        off_task_lines, on_dates, on_task_lines, on_tokens, token_rows, Provides,
    };
    use crate::{Clicked, Provider, Requested};

    /// A task with an owner whose name is misspelled, one whose name is
    /// not, a misspelled word, a tag and a due date; and a line that is no
    /// task, with the same misspelled word.
    const PROVIDED: &str = "- [ ] Mail @sam and @lee teh #home [due:: 2026-11-30]\nplain teh text";

    fn provider(answer: fn(&Clicked) -> Option<Requested>) -> Arc<Provides> {
        Arc::new(Provides::new(answer))
    }

    /// An editor on `kind` over [`PROVIDED`], the caret on its second
    /// line, whose host has tasks on, rejects `sam` and `teh`, and names
    /// `providers`.
    fn providing(kind: BackendKind, providers: Vec<Provider>) -> crate::EditorView {
        let mut ev = editing(kind, PROVIDED, 1, 5);
        ev.set_tasks(Some(wren_markdown::TaskOptions {
            today: wren_markdown::date::Date::new(2026, 11, 19).expect("a date"),
        }));
        let proofreader = fixtures::Rejects::new(&["sam", "teh"]).suggesting(&[("teh", &["the"])]);
        ev.set_proofreader(Some(Arc::new(proofreader)));
        ev.set_menu_providers(providers);
        ev
    }

    /// A right press over cell `cells` of row `row`.
    fn right_click(cells: f32, row: f32) -> (Event, Option<Point>) {
        (press(mouse::Button::Right), Some(over(cells, row)))
    }

    /// Who took a right-click: the suggestions menu, the host's provider
    /// at a place in its list, or no one.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Took {
        Spelling,
        Host(usize),
        Nobody,
    }

    /// Who the widget found to take the right-click that `messages` were
    /// published for, at cell `cells` of row `row`.
    fn took(messages: &[Message], cells: f32, row: f32) -> Took {
        let at = match spell_events(messages).as_slice() {
            [] => {
                assert!(messages.is_empty(), "{messages:?}");
                return Took::Nobody;
            }
            [SpellEvent::OpenAt(at)] => (Took::Spelling, *at),
            [SpellEvent::Context { provider, at, .. }] => (Took::Host(*provider), *at),
            other => panic!("{other:?}"),
        };
        assert_eq!(messages.len(), 1, "{messages:?}");
        // The point is relative to the text origin.
        let (took, at) = at;
        assert!(
            (at.x - cells * ADVANCE).abs() < 0.01 && at.y == row * LINE_HEIGHT + 12.0,
            "{at:?}"
        );
        took
    }

    /// One right-click over cell `cells` of row `row`, alone in its batch:
    /// who the widget finds to take it, and whose menu the host then has
    /// open.
    fn who_takes(ev: &mut crate::EditorView, cells: f32, row: f32) -> Took {
        let (messages, captured) = widget_batch(ev, &mut None, vec![right_click(cells, row)]);
        let took = took(&messages, cells, row);
        assert_eq!(captured, [took != Took::Nobody]);
        for message in messages {
            host_applies(ev, message);
        }
        match took {
            Took::Nobody => assert!(ev.popover().is_none()),
            Took::Spelling => assert!(ev.spell_menu().is_some()),
            Took::Host(index) => {
                assert!(ev.spell_menu().is_none());
                assert_eq!(ev.popover().and_then(Popover::provider), Some(index));
            }
        }
        took
    }

    /// The cells of [`PROVIDED`]'s first row a right-click is tried on.
    mod cell {
        pub const BOX: f32 = 3.5;
        pub const MAIL: f32 = 7.5;
        /// The `@` of the owner whose name is misspelled, and the name.
        pub const AT_SAM: f32 = 11.5;
        pub const SAM: f32 = 13.5;
        pub const LEE: f32 = 21.5;
        pub const TEH: f32 = 26.5;
        pub const HOME: f32 = 30.5;
        pub const DATE: f32 = 44.5;
        pub const PAST_THE_END: f32 = 57.0;
    }

    #[test]
    fn a_right_click_asks_the_providers_in_order_and_the_first_to_answer_takes_it() {
        use Took::{Host, Nobody, Spelling};
        // The providers of a list: the suggestions menu, one that takes an
        // owner or a tag, and one that takes any place on a task line.
        #[derive(Debug, Clone, Copy, PartialEq)]
        enum Asks {
            S,
            Tokens,
            Lines,
        }
        use Asks::{Lines, Tokens, S};
        type Case = (f32, f32, Took);
        let orders: [(&[Asks], &[Case]); 5] = [
            // Spelling first, as a host has it: a misspelled word is the
            // suggestions menu's, an owner's name included, and no other
            // provider hears of it.
            (
                &[S, Tokens, Lines],
                &[
                    (cell::SAM, 0.0, Spelling),
                    (cell::TEH, 0.0, Spelling),
                    (7.5, 1.0, Spelling),
                    // A token that is no misspelling is the first
                    // provider's that takes it.
                    (cell::AT_SAM, 0.0, Host(1)),
                    (cell::LEE, 0.0, Host(1)),
                    (cell::HOME, 0.0, Host(1)),
                    // What it passes on is the next one's: a span it does
                    // not take, a place in no span, and the row past the
                    // end of the line's text.
                    (cell::BOX, 0.0, Host(2)),
                    (cell::DATE, 0.0, Host(2)),
                    (cell::MAIL, 0.0, Host(2)),
                    (cell::PAST_THE_END, 0.0, Host(2)),
                    // No one takes a place on a line that is no task, or
                    // under the text.
                    (1.5, 1.0, Nobody),
                    (1.5, 3.0, Nobody),
                ],
            ),
            // A provider put before spelling takes a misspelled word it
            // answers for, and spelling what it passes on.
            (
                &[Lines, S, Tokens],
                &[
                    (cell::SAM, 0.0, Host(0)),
                    (cell::TEH, 0.0, Host(0)),
                    (cell::LEE, 0.0, Host(0)),
                    (7.5, 1.0, Spelling),
                    (1.5, 1.0, Nobody),
                ],
            ),
            // Where two of the host's take a place, the first does.
            (
                &[Tokens, Lines],
                &[
                    (cell::LEE, 0.0, Host(0)),
                    // A misspelled owner is no token, and a list without
                    // the suggestions menu opens none.
                    (cell::SAM, 0.0, Host(1)),
                    (7.5, 1.0, Nobody),
                ],
            ),
            (&[Lines, Tokens], &[(cell::LEE, 0.0, Host(0))]),
            // The suggestions menu alone, as with no list named.
            (
                &[S],
                &[(cell::SAM, 0.0, Spelling), (cell::LEE, 0.0, Nobody)],
            ),
        ];
        for kind in BackendKind::ALL {
            let owned = kind == BackendKind::Owned;
            for (order, cases) in orders {
                for &(cells, row, expected) in cases {
                    let (tokens, lines) = (provider(on_tokens), provider(on_task_lines));
                    let providers = order.iter().map(|asks| match asks {
                        S => Provider::Spelling,
                        Tokens => Provider::Host(tokens.clone()),
                        Lines => Provider::Host(lines.clone()),
                    });
                    let providers: Vec<Provider> = providers.collect();
                    let mut ev = providing(kind, providers.clone());
                    let case = format!("{kind:?} {order:?} cell {cells} row {row}");
                    // The classic engine opens no menu for a right-click.
                    let expected = if owned { expected } else { Nobody };
                    assert_eq!(who_takes(&mut ev, cells, row), expected, "{case}");
                    // The menu is the provider's, under what it named.
                    if let Host(index) = expected {
                        let open = ev.popover().unwrap();
                        let (rows, range) = match order[index] {
                            Tokens => (
                                token_rows(),
                                ev.hit_at(0, cells as usize).unwrap().range().clone(),
                            ),
                            _ => (status_rows(), 2..5),
                        };
                        let labels: Vec<String> = rows.into_iter().map(|row| row.label).collect();
                        assert_eq!(
                            (open.labels(), open.line(), open.range()),
                            (&labels[..], 0, range),
                            "{case}"
                        );
                        assert_eq!(open.mode(), Mode::Menu, "{case}");
                    }
                    // Each of the host's providers ahead of the one that
                    // took the click was asked and passed; the one that
                    // took it was asked by the widget and again by the
                    // editor, about the same place; and none after it was
                    // asked at all.
                    let taker = match expected {
                        Spelling => order.iter().position(|asks| *asks == S),
                        Host(index) => Some(index),
                        Nobody => None,
                    };
                    for (index, asks) in order.iter().enumerate() {
                        let asked = match asks {
                            S => continue,
                            Tokens => tokens.asked(),
                            Lines => lines.asked(),
                        };
                        let times = match taker {
                            _ if !owned => 0,
                            Some(taker) if index > taker => 0,
                            Some(taker) if index == taker => 2,
                            Some(_) => 1,
                            // Under the text there is no place to ask of.
                            None if row > 1.0 => 0,
                            None => 1,
                        };
                        assert_eq!(asked.len(), times, "{case} {asks:?}");
                        assert!(asked.windows(2).all(|pair| pair[0] == pair[1]), "{case}");
                    }
                    // Without a window, the same provider takes a click
                    // on the character under the cell.
                    let mut ev = providing(kind, providers);
                    widget_batch(&ev, &mut None, Vec::new());
                    let (line, column) = (row as usize, cells as usize);
                    if ev.line_text(line).is_some_and(|text| column < text.len()) {
                        let alone = ev.right_click_message(line, column);
                        assert_eq!(
                            took(alone.as_slice(), cells.floor() + 0.5, row),
                            expected,
                            "{case}"
                        );
                        if let Some(Message::Spell(event)) = alone {
                            ev.spell(event);
                            assert_eq!(ev.popover().is_some(), expected != Nobody, "{case}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_provider_is_asked_about_the_place_under_the_pointer() {
        use wren_markdown::span::SpanKind;
        let lines = provider(on_task_lines);
        let asked = |cells: f32, row: f32| {
            let mut ev = providing(BackendKind::Owned, vec![Provider::Host(lines.clone())]);
            let before = lines.asked().len();
            who_takes(&mut ev, cells, row);
            lines.asked().get(before).cloned()
        };
        let first = PROVIDED.lines().next().unwrap();
        // A token: its span is the hit, and the line's spans come along.
        let lee = asked(cell::LEE, 0.0).unwrap();
        assert_eq!(
            (lee.line, lee.text.as_str(), lee.column),
            (0, first, Some(21))
        );
        assert!(matches!(
            &lee.hit,
            Some(Hit::Span(span))
                if span.range == (20..24) && matches!(span.kind, SpanKind::Owner { .. })
        ));
        assert!(lee
            .spans
            .spans
            .iter()
            .any(|span| matches!(span.kind, SpanKind::Box(_))));
        // A misspelled word is the hit before the span it lies in, with
        // the glyph under the pointer still named.
        let sam = asked(cell::SAM, 0.0).unwrap();
        assert_eq!(
            (sam.column, &sam.hit),
            (Some(13), &Some(Hit::Misspelling(12..15)))
        );
        // A place in no span is no hit, on a line whose spans say what it
        // is; past the end of the text there is no glyph either.
        let mail = asked(cell::MAIL, 0.0).unwrap();
        assert_eq!(
            (mail.column, &mail.hit, &mail.spans),
            (Some(7), &None, &lee.spans)
        );
        let past = asked(cell::PAST_THE_END, 0.0).unwrap();
        assert_eq!((past.line, past.column, &past.hit), (0, None, &None));
        // The second line is no task, and under the text nothing is asked.
        let plain = asked(1.5, 1.0).unwrap();
        assert_eq!(
            (plain.line, plain.text.as_str(), plain.column),
            (1, "plain teh text", Some(1))
        );
        assert!(plain.hit.is_none() && plain.spans.spans.is_empty());
        assert_eq!(asked(1.5, 3.0), None);
    }

    #[test]
    fn a_right_click_a_provider_takes_takes_the_keyboard_and_any_other_changes_nothing() {
        let unfocused = |ev: &crate::EditorView| {
            let editor = editor_with(&ev.content, true, Measure::default());
            let mut state = laid_out_state(&editor);
            state.drag_click = Some(mouse::click::Kind::Single);
            assert!(state.focus.is_none());
            Some(state)
        };
        for kind in BackendKind::ALL {
            let tokens = provider(on_tokens);
            let providers = vec![Provider::Spelling, Provider::Host(tokens.clone())];
            let ev = providing(kind, providers);
            // On a token the provider takes: the editor takes the
            // keyboard, a drag in progress ends, and the caret stays.
            let mut state = unfocused(&ev);
            let (messages, captured) =
                widget_batch(&ev, &mut state, vec![right_click(cell::LEE, 0.0)]);
            let state = state.unwrap();
            if kind == BackendKind::Owned {
                assert_eq!(took(&messages, cell::LEE, 0.0), Took::Host(1));
                assert_eq!(captured, [true]);
                assert!(state.focus.is_some() && state.drag_click.is_none());
            } else {
                assert!(messages.is_empty() && captured == [false], "{messages:?}");
                assert!(state.focus.is_none() && state.drag_click.is_some());
            }
            assert_eq!(ev.cursor(), caret(1, 5));
            // Where no provider answers, the press is no one's.
            for (cells, row) in [(cell::MAIL, 0.0), (cell::BOX, 0.0), (1.5, 1.0), (1.5, 3.0)] {
                let mut state = unfocused(&ev);
                let (messages, captured) =
                    widget_batch(&ev, &mut state, vec![right_click(cells, row)]);
                let state = state.unwrap();
                assert!(messages.is_empty(), "{kind:?} {cells} {row}: {messages:?}");
                assert_eq!(captured, [false], "{kind:?} {cells} {row}");
                assert!(state.focus.is_none() && state.drag_click.is_some());
            }
        }
    }

    /// The text, the caret, the provider and the row chosen, and whether a
    /// popover is open after `batches` of events go, each batch in turn,
    /// to an editor over [`PROVIDED`] whose host asks the suggestions
    /// menu, then the status menu of a task line.
    fn after_right_clicks(
        batches: Vec<Vec<(Event, Option<Point>)>>,
    ) -> (String, Cursor, Option<(usize, u32)>, bool) {
        let providers = vec![Provider::Spelling, Provider::Host(provider(on_task_lines))];
        let mut ev = providing(BackendKind::Owned, providers);
        let mut state = None;
        let mut chosen = None;
        for batch in batches {
            let (published, _) = widget_batch(&ev, &mut state, batch);
            for message in published {
                let provided = host_applies(&mut ev, message).provided;
                chosen = provided
                    .map(|(provider, choice)| (provider, choice.id))
                    .or(chosen);
            }
        }
        (ev.text(), ev.cursor(), chosen, ev.popover().is_some())
    }

    #[test]
    fn keys_after_a_right_click_do_the_same_in_its_batch_as_in_the_next() {
        use keyboard::key::Named;
        let key = |event: Event| (event, None);
        let typing = "- [ ] Mail @sam and @lee teh #home [due:: 2026-11-30]\nplain";
        // What follows the click, the text it leaves, and the row chosen.
        type Case = (
            &'static str,
            Vec<(Event, Option<Point>)>,
            String,
            Option<u32>,
        );
        let cases: [Case; 7] = [
            // An accelerator of the provider's menu chooses its row.
            (
                "an accelerator",
                vec![key(typed("x"))],
                PROVIDED.into(),
                Some(22),
            ),
            (
                "the space bar",
                vec![key(space())],
                PROVIDED.into(),
                Some(20),
            ),
            // The menu's own keys drive it.
            (
                "down and enter",
                vec![key(named(Named::ArrowDown)), key(named(Named::Enter))],
                PROVIDED.into(),
                Some(21),
            ),
            // Escape closes it, and the letter after it is the text's.
            (
                "escape and a letter",
                vec![key(named(Named::Escape)), key(typed("x"))],
                format!("{typing}x teh text"),
                None,
            ),
            // Any other key closes it and is the text's.
            (
                "another letter",
                vec![key(typed("y"))],
                format!("{typing}y teh text"),
                None,
            ),
            // A press in the editor, of either button, only closes it.
            (
                "a left press",
                vec![(press(mouse::Button::Left), Some(over(1.0, 0.0)))],
                PROVIDED.into(),
                None,
            ),
            (
                "a right press",
                vec![right_click(cell::LEE, 0.0)],
                PROVIDED.into(),
                None,
            ),
        ];
        for (name, after, text, chosen) in cases {
            let mut one = vec![right_click(cell::MAIL, 0.0)];
            one.extend(after.iter().cloned());
            let together = after_right_clicks(vec![one]);
            let apart = after_right_clicks(vec![vec![right_click(cell::MAIL, 0.0)], after]);
            assert_eq!(together, apart, "{name}");
            let chosen = chosen.map(|id| (1, id));
            assert_eq!(
                (together.0, together.2, together.3),
                (text, chosen, false),
                "{name}"
            );
            // But for what the text took, the caret stays where it was.
            if chosen.is_some() || name.ends_with("press") {
                assert_eq!(together.1, caret(1, 5), "{name}");
            }
        }
    }

    #[test]
    fn a_right_click_opens_the_providers_menu_for_the_keys_after_it_in_its_batch() {
        use keyboard::key::Named;
        // The widget asks the provider what the click opens, so the keys
        // after it are published as the menu's own events, routed against
        // its rows.
        let lines = provider(on_task_lines);
        let providers = vec![Provider::Spelling, Provider::Host(lines.clone())];
        let mut ev = providing(BackendKind::Owned, providers.clone());
        let events = vec![
            right_click(cell::MAIL, 0.0),
            (named(Named::ArrowDown), None),
            (typed("x"), None),
        ];
        let (messages, captured) = widget_batch(&ev, &mut None, events);
        assert!(
            matches!(
                spell_events(&messages).as_slice(),
                [
                    SpellEvent::Context { provider: 1, .. },
                    SpellEvent::Move(1),
                    // A row of a provider's menu names its menu: the
                    // next popover the editor opens.
                    SpellEvent::AcceptOf {
                        menu: 1,
                        row: Some(2)
                    }
                ]
            ),
            "{messages:?}"
        );
        assert_eq!(captured, [true; 3]);
        let mut provided = None;
        for message in messages {
            provided = host_applies(&mut ev, message).provided.or(provided);
        }
        // The row comes back with its provider, and only so.
        let row = Choice {
            id: 22,
            mode: Mode::Menu,
            line: 0,
            range: 2..5,
        };
        assert_eq!(provided, Some((1, row)));
        assert_eq!((ev.text().as_str(), ev.cursor()), (PROVIDED, caret(1, 5)));
        assert!(ev.popover().is_none());

        // Once the host has drawn the menu, a key or a press on a row
        // names the menu the same way, by the number the editor opened it
        // under.
        for (name, chooses) in [
            ("a key", (typed("/"), None)),
            ("enter", (named(Named::Enter), None)),
            (
                "a press on a row",
                (press(mouse::Button::Left), Some(on_task_row(1))),
            ),
        ] {
            let mut ev = providing(BackendKind::Owned, providers.clone());
            let mut state = None;
            popover_batch(&mut ev, &mut state, vec![right_click(cell::MAIL, 0.0)]);
            let menu = ev.popover().unwrap().instance();
            let row = (name != "enter").then_some(1);
            let (messages, _) = widget_batch(&ev, &mut state, vec![chooses]);
            assert_eq!(
                spell_events(&messages),
                [SpellEvent::AcceptOf { menu, row }],
                "{name}"
            );
            let chosen = host_applies(&mut ev, messages[0].clone()).provided;
            let id = if name == "enter" { 20 } else { 21 };
            assert_eq!(
                chosen.map(|(provider, row)| (provider, row.id)),
                Some((1, id))
            );
        }
        // A menu that is no provider's names itself the same way (item
        // 3.4).
        let mut ev = providing(BackendKind::Owned, providers.clone());
        assert!(ev.open_popover(Anchor::Caret, Mode::Menu, status_rows()));
        let menu = ev.popover().unwrap().instance();
        let (messages, _) = widget_batch(&ev, &mut None, vec![(typed("/"), None)]);
        assert_eq!(
            spell_events(&messages),
            [SpellEvent::AcceptOf { menu, row: Some(1) }]
        );

        // The suggestions menu a right-click asks for is as it was built:
        // its keys go to the host, which resolves them against its menu.
        let mut ev = providing(BackendKind::Owned, providers);
        let events = vec![right_click(cell::TEH, 0.0), (named(Named::Enter), None)];
        let (messages, _, _) = popover_batch(&mut ev, &mut None, events);
        assert!(
            matches!(
                spell_events(&messages).as_slice(),
                [SpellEvent::OpenAt(_), SpellEvent::Key(MenuKey::Enter)]
            ),
            "{messages:?}"
        );
        assert_eq!(
            ev.line_text(0).unwrap(),
            PROVIDED.lines().next().unwrap().replace("teh", "the")
        );
    }

    #[test]
    fn after_a_message_that_may_move_the_text_no_provider_of_the_hosts_answers() {
        use keyboard::key::Named;
        // What comes before the click in its batch: a letter typed, a
        // caret motion, a scroll. The text under the pointer is then not
        // known to be what the host will find there.
        let earlier = || {
            [
                ("a letter", (typed("q"), None)),
                ("a motion", (named(Named::ArrowLeft), None)),
                (
                    "a scroll",
                    (
                        wheel(mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 }),
                        Some(over(1.0, 0.0)),
                    ),
                ),
            ]
        };
        // The click with those providers, and the same click with the
        // suggestions menu alone, as an editor had before there were any.
        let lines = provider(on_task_lines);
        let dates = provider(on_dates);
        let providers = vec![
            Provider::Spelling,
            Provider::Host(dates.clone()),
            Provider::Host(lines.clone()),
        ];
        for (name, before) in earlier() {
            for (cells, row, spelled) in [
                (cell::MAIL, 0.0, false),
                (cell::BOX, 0.0, false),
                (cell::LEE, 0.0, false),
                (cell::DATE, 0.0, false),
                (1.5, 1.0, false),
                // A word drawn as misspelled is asked for as it always
                // was.
                (cell::TEH, 0.0, true),
                (cell::SAM, 0.0, true),
            ] {
                let case = format!("{name} cell {cells} row {row}");
                let events = vec![before.clone(), right_click(cells, row), (typed("x"), None)];
                let mut ev = providing(BackendKind::Owned, providers.clone());
                let (messages, captured) = widget_batch(&ev, &mut None, events.clone());
                // No provider of the host's is asked, and the press is
                // no one's unless it is on such a word.
                assert_eq!((lines.asked().len(), dates.asked().len()), (0, 0), "{case}");
                let click = spell_events(&messages);
                assert_eq!(
                    matches!(click.first(), Some(SpellEvent::OpenAt(_))),
                    spelled,
                    "{case}: {messages:?}"
                );
                assert!(
                    !click.iter().any(|event| matches!(
                        event,
                        SpellEvent::Context { .. } | SpellEvent::Press(_)
                    )),
                    "{case}: {messages:?}"
                );
                assert_eq!(captured[1], spelled, "{case}");
                // It publishes what the same batch does with no provider
                // named at all.
                let alone = providing(BackendKind::Owned, vec![Provider::Spelling]);
                let (expected, _) = widget_batch(&alone, &mut None, events);
                assert_eq!(format!("{messages:?}"), format!("{expected:?}"), "{case}");
                // The host opens no provider's menu, hands no keyboard
                // over, and the letter after the click is typed once.
                let before = ev.text();
                for message in messages {
                    let applied = host_applies(&mut ev, message);
                    assert_eq!((applied.provided, applied.keyboard), (None, None), "{case}");
                    assert_eq!(ev.popover().and_then(Popover::provider), None, "{case}");
                }
                let typed = usize::from(name == "a letter");
                assert_eq!(ev.text().len(), before.len() + 1 + typed, "{case}");
                assert_eq!((lines.asked().len(), dates.asked().len()), (0, 0), "{case}");
            }
        }
        // In a list where a provider of the host's comes before the
        // suggestions menu, that provider would have to be asked first:
        // the press is no one's, a misspelled word included.
        let providers = vec![Provider::Host(lines.clone()), Provider::Spelling];
        let ev = providing(BackendKind::Owned, providers);
        let events = vec![(typed("q"), None), right_click(cell::TEH, 0.0)];
        let (messages, captured) = widget_batch(&ev, &mut None, events);
        assert!(spell_events(&messages).is_empty(), "{messages:?}");
        assert_eq!(captured, [true, false]);
        assert!(lines.asked().is_empty());
        // The click in a batch of its own is the provider's, as ever.
        let mut ev = providing(
            BackendKind::Owned,
            vec![Provider::Spelling, Provider::Host(lines.clone())],
        );
        assert_eq!(who_takes(&mut ev, cell::MAIL, 0.0), Took::Host(1));
    }

    /// The popover of `ev`, as its provider, its anchor and its rows.
    fn provided_menu(ev: &crate::EditorView) -> Option<(Option<usize>, Range<usize>, Vec<String>)> {
        ev.popover().map(|popover| {
            (
                popover.provider(),
                popover.range(),
                popover.labels().to_vec(),
            )
        })
    }

    #[test]
    fn a_right_press_the_host_resolves_asks_for_the_suggestions_menu_alone() {
        // After a request for the suggestions menu in the same batch,
        // which may not have opened it, a right press is the host's to
        // resolve, as it was built: it closes that menu, or with none
        // open opens the suggestions menu on a misspelled word. The
        // widget asks no provider of the host's about such a press, and
        // none answers it.
        let (tokens, lines, dates) = (
            provider(on_tokens),
            provider(on_task_lines),
            provider(on_dates),
        );
        let spelling_first = [
            Provider::Spelling,
            Provider::Host(tokens.clone()),
            Provider::Host(dates.clone()),
            Provider::Host(lines.clone()),
        ];
        let no_spelling = [Provider::Host(tokens.clone())];
        for (providers, cells, row, word) in [
            (&spelling_first[..], cell::MAIL, 0.0, None),
            (&spelling_first[..], cell::LEE, 0.0, None),
            (&spelling_first[..], cell::DATE, 0.0, None),
            (&spelling_first[..], 1.5, 1.0, None),
            (&spelling_first[..], cell::TEH, 0.0, Some("teh")),
            // A host that left the suggestions menu out of its list gets
            // none.
            (&no_spelling[..], cell::LEE, 0.0, None),
            (&no_spelling[..], cell::TEH, 0.0, None),
        ] {
            let case = format!("{} providers, cell {cells} row {row}", providers.len());
            let mut ev = providing(BackendKind::Owned, providers.to_vec());
            ev.set_key_policy(suggest_policy());
            // The caret is after `plain`, which is spelled right.
            let events = vec![(suggest_key(), None), right_click(cells, row)];
            let (messages, _) = widget_batch(&ev, &mut None, events);
            assert!(
                matches!(
                    spell_events(&messages).as_slice(),
                    [
                        SpellEvent::OpenAtCaret,
                        SpellEvent::Press(MenuPress::Right(_))
                    ]
                ),
                "{case}: {messages:?}"
            );
            for message in messages {
                let applied = host_applies(&mut ev, message);
                assert_eq!((applied.provided, applied.keyboard), (None, None), "{case}");
            }
            assert_eq!(ev.spell_menu().map(SpellMenu::word), word, "{case}");
            assert_eq!(ev.popover().and_then(Popover::provider), None, "{case}");
        }
        let asked = [&tokens, &lines, &dates].map(|provider| provider.asked().len());
        assert_eq!(asked, [0; 3]);

        // When the request did open the suggestions menu, the press only
        // closes it.
        let providers = spelling_first.to_vec();
        let mut ev = providing(BackendKind::Owned, providers);
        ev.set_key_policy(suggest_policy());
        ev.move_cursor(caret(1, 7));
        let events = vec![(suggest_key(), None), right_click(cell::MAIL, 0.0)];
        let (messages, _) = widget_batch(&ev, &mut None, events);
        let mut open = Vec::new();
        for message in messages {
            host_applies(&mut ev, message);
            open.push((ev.spell_menu().is_some(), ev.popover().is_some()));
        }
        assert_eq!(open, [(true, true), (false, false)]);
        assert!(lines.asked().is_empty());
    }

    /// An event of an input method.
    fn composing(content: &str) -> (Event, Option<Point>) {
        let event = input_method::Event::Preedit(content.into(), None);
        (Event::InputMethod(event), None)
    }

    fn committing(content: &str) -> (Event, Option<Point>) {
        let event = input_method::Event::Commit(content.into());
        (Event::InputMethod(event), None)
    }

    /// An editor whose provider takes the keyboard for a date, and the
    /// click on the date.
    fn date_clicked() -> (crate::EditorView, (Event, Option<Point>)) {
        let providers = vec![Provider::Spelling, Provider::Host(provider(on_dates))];
        let ev = providing(BackendKind::Owned, providers);
        (ev, right_click(cell::DATE, 0.0))
    }

    /// An editor whose source takes the keyboard for a request, and the
    /// request's key.
    fn keyboard_requested() -> (crate::EditorView, (Event, Option<Point>)) {
        let ev = requesting(BackendKind::Owned, PROVIDED, 1, 5);
        (ev, (request_key("k"), None))
    }

    #[test]
    fn a_provider_that_takes_the_keyboard_has_the_rest_of_its_batch_dropped() {
        use keyboard::key::Named;
        use wren_markdown::span::SpanKind;
        let (mut ev, click) = date_clicked();
        let events = vec![
            click,
            (typed("f"), None),
            (named(Named::Enter), None),
            (named(Named::Backspace), None),
            (space(), None),
            // A press in the editor is the click it is, and the keys
            // after it are dropped all the same.
            (press(mouse::Button::Left), Some(over(1.0, 1.0))),
            (typed("x"), None),
            (named(Named::Delete), None),
        ];
        let (messages, captured) = widget_batch(&ev, &mut None, events);
        // Each key is taken, so that it is no one else's, and nothing is
        // published for it: not to the note, and not to the host.
        assert_eq!(captured, [true; 8]);
        let [click @ Message::Spell(SpellEvent::Context {
            provider: 1, at, ..
        }), Message::Action(Action::Click(_))] = messages.as_slice()
        else {
            panic!("{messages:?}");
        };
        // The editor opens nothing, and hands the host the click with what
        // was clicked, once, to open its own there.
        let at = *at;
        let applied = host_applies(&mut ev, click.clone());
        let (provider, clicked) = applied.keyboard.unwrap();
        assert_eq!((provider, clicked.line), (1, 0));
        assert!(matches!(
            &clicked.hit,
            Some(Hit::Span(span)) if matches!(span.kind, SpanKind::Date { .. })
        ));
        assert_eq!(Some(clicked), ev.clicked(at));
        assert_eq!(ev.take_keyboard(), None);
        assert!(ev.popover().is_none());
        host_applies(&mut ev, messages[1].clone());
        // The note is what it was, byte for byte.
        assert_eq!(ev.text().as_bytes(), PROVIDED.as_bytes());
        assert_eq!(ev.cursor(), caret(1, 1));

        // Escape and a shortcut of the host's stay the host's own.
        let (ev, click) = date_clicked();
        let escape = (named(Named::Escape), None);
        let (messages, captured) = widget_batch(&ev, &mut None, vec![click, escape]);
        assert_eq!((messages.len(), captured), (1, vec![true, false]));
        // The next batch is the editor's again, if it still has the
        // keyboard: the host gives it to its field by then.
        let (mut ev, click) = date_clicked();
        let mut state = None;
        popover_batch(&mut ev, &mut state, vec![click, (typed("f"), None)]);
        popover_batch(&mut ev, &mut state, vec![(typed("g"), None)]);
        assert_eq!(ev.line_text(1).unwrap(), "plaing teh text");
    }

    #[test]
    fn a_dialogs_request_drops_its_batch_escape_included_on_both_engines() {
        for kind in BackendKind::ALL {
            let ev = requesting(kind, TASK, 0, 8);
            let events = [
                request_key("d"),
                typed("x"),
                named(keyboard::key::Named::Escape),
            ]
            .into_iter()
            .map(|event| (event, None))
            .collect();
            let (messages, captured) = widget_batch(&ev, &mut None, events);
            // The Escape is published, so the host's key handling drops its
            // copy, and taken: it closes nothing.
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Spell(SpellEvent::Request(DIALOG)),
                        Message::Spell(SpellEvent::Stroke(stroke)),
                    ] if stroke.named() == Some(keyboard::key::Named::Escape)
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [true; 3], "{kind:?}");
        }
        // After an edit, when the widget no longer knows the caret's line, a
        // dialog's request takes the keyboard all the same.
        for kind in BackendKind::ALL {
            let ev = requesting(kind, TASK, 0, 8);
            let events = [
                named(keyboard::key::Named::Backspace),
                request_key("d"),
                named(keyboard::key::Named::Escape),
            ]
            .into_iter()
            .map(|event| (event, None))
            .collect();
            let (messages, captured) = widget_batch(&ev, &mut None, events);
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        _,
                        Message::Spell(SpellEvent::Request(DIALOG)),
                        Message::Spell(SpellEvent::Stroke(stroke)),
                    ] if stroke.named() == Some(keyboard::key::Named::Escape)
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [true; 3], "{kind:?}");
        }
        // A menu's row that opens a dialog: the Escape after its key in the
        // batch is the dialog's too, not the host's to close it with.
        for kind in BackendKind::ALL {
            let mut ev = editing(kind, TASK, 0, 8);
            let rows = vec![Row::new(14, "Edit task…")
                .accelerator(crate::Accelerator::plain('e'))
                .opens_dialog()];
            hang(&mut ev, Mode::Menu, 0, 2..5, rows);
            let events = [typed("e"), typed("z"), named(keyboard::key::Named::Escape)]
                .into_iter()
                .map(|event| (event, None))
                .collect();
            let (messages, captured) = widget_batch(&ev, &mut None, events);
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Spell(SpellEvent::AcceptOf { .. } | SpellEvent::Accept(_)),
                        Message::Spell(SpellEvent::Stroke(stroke)),
                    ] if stroke.named() == Some(keyboard::key::Named::Escape)
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [true; 3], "{kind:?}");
        }
        // On the engine that opens no popovers, a request for a menu opens
        // nothing, and the keys after it are the note's as before.
        let ev = requesting(BackendKind::Iced, TASK, 0, 8);
        let events = [request_key("s"), typed("x")]
            .into_iter()
            .map(|event| (event, None))
            .collect();
        let (messages, _) = widget_batch(&ev, &mut None, events);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Request(STATUS)),
                    Message::Action(Action::Edit(Edit::Insert('x'))),
                ]
            ),
            "{messages:?}"
        );
    }

    /// A row chosen with the pointer that opens a menu or a dialog of the
    /// host's hands it the keyboard as the row's key does: the keys and the
    /// input-method text after it in its batch are dropped, on both
    /// engines (ring review, finding 2). A row that opens nothing leaves
    /// the batch's keys to the text, as before.
    #[test]
    fn a_row_chosen_with_the_pointer_that_opens_a_menu_drops_the_rest_of_its_batch() {
        use keyboard::key::Named;
        let left = || press(mouse::Button::Left);
        let rows = || {
            let mut rows = status_rows();
            rows.push(Row::new(29, "Priority…").opens_menu());
            rows.push(Row::new(14, "Edit task…").opens_dialog());
            rows
        };
        let accepted = |messages: &[Message], menu: u64, row: usize| {
            matches!(
                messages.first(),
                Some(Message::Spell(SpellEvent::AcceptOf { menu: m, row: Some(r) }))
                    if *m == menu && *r == row
            )
        };
        for kind in BackendKind::ALL {
            for (row, id) in [(3, 29), (4, 14)] {
                let mut ev = on_task(kind, Mode::Menu, rows());
                let menu = ev.popover().map(crate::Popover::instance).unwrap();
                let events = vec![
                    (left(), Some(on_task_row(row))),
                    (typed("h"), None),
                    committing("x"),
                    (named(Named::Enter), None),
                    (space(), None),
                ];
                let (messages, captured, choice) = popover_batch(&mut ev, &mut None, events);
                assert!(
                    accepted(&messages, menu, row),
                    "{kind:?} {row}: {messages:?}"
                );
                assert_eq!(messages.len(), 1, "{kind:?} {row}: {messages:?}");
                assert!(captured[0] && captured[1], "{kind:?} {row}: {captured:?}");
                assert_eq!(choice, chose(id, Mode::Menu), "{kind:?} {row}");
                assert_eq!(ev.text(), TASK, "{kind:?} {row}");
                assert_eq!(ev.cursor(), caret(0, 8), "{kind:?} {row}");
            }

            // The Escape after a dialog's row is the dialog's, dropped;
            // after a menu's row it stays the host's, as after its key.
            for (row, dropped) in [(4, true), (3, false)] {
                let ev = on_task(kind, Mode::Menu, rows());
                let events = vec![
                    (left(), Some(on_task_row(row))),
                    (named(Named::Escape), None),
                ];
                let (messages, _) = widget_batch(&ev, &mut None, events);
                let stroke = matches!(
                    messages.as_slice(),
                    [_, Message::Spell(SpellEvent::Stroke(stroke))]
                        if stroke.named() == Some(Named::Escape)
                );
                assert_eq!(stroke, dropped, "{kind:?} {row}: {messages:?}");
                if !dropped {
                    assert_eq!(messages.len(), 1, "{kind:?}: {messages:?}");
                }
            }

            // A composition under way is cancelled, and what it commits
            // is not the note's.
            let mut ev = on_task(kind, Mode::Menu, rows());
            let mut state = composing_state(&ev);
            let events = vec![(left(), Some(on_task_row(3))), committing("き")];
            let (messages, _, _) = popover_batch(&mut ev, &mut state, events);
            assert_eq!(messages.len(), 1, "{kind:?}: {messages:?}");
            assert!(!state.as_ref().unwrap().is_composing(), "{kind:?}");
            assert_eq!(ev.text(), TASK, "{kind:?}");

            // A row that opens nothing: the key after it is the text's.
            let mut ev = on_task(kind, Mode::Menu, rows());
            let menu = ev.popover().map(crate::Popover::instance).unwrap();
            let events = vec![(left(), Some(on_task_row(2))), (typed("h"), None)];
            let (messages, _, choice) = popover_batch(&mut ev, &mut None, events);
            assert!(accepted(&messages, menu, 2), "{kind:?}: {messages:?}");
            assert!(
                matches!(
                    messages.as_slice(),
                    [_, Message::Action(Action::Edit(Edit::Insert('h')))]
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(choice, chose(22, Mode::Menu), "{kind:?}");
        }
    }

    /// A press off a menu that closes it, or a wheel that scrolls the
    /// editor, ends the menu for the overlay too: a press on one of its
    /// rows later in the batch chooses nothing (ring review, finding 4).
    #[test]
    fn a_press_or_a_scroll_that_dismisses_a_menu_comes_before_a_later_press_on_a_row() {
        let left = || press(mouse::Button::Left);
        let on_text = Some(over(1.0, 0.0));
        let outside = Some(Point::new(700.0, 100.0));
        for (name, first) in [
            ("a left press in the text", (left(), on_text)),
            (
                "a right press in the text",
                (press(mouse::Button::Right), on_text),
            ),
            ("a press outside the editor", (left(), outside)),
            ("a whole line of wheel", (wheel(pixels(-4.0)), on_text)),
            (
                "a wheel's line",
                (
                    wheel(mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 }),
                    on_text,
                ),
            ),
        ] {
            let mut ev = spelled_menu();
            let (messages, _, _) = popover_batch(
                &mut ev,
                &mut None,
                vec![first, (left(), Some(on_spelled_row(1)))],
            );
            assert!(
                !messages.iter().any(|message| matches!(
                    message,
                    Message::Spell(SpellEvent::Accept(_) | SpellEvent::AcceptSelected)
                )),
                "{name}: {messages:?}"
            );
            assert_eq!(ev.text(), "cat teh", "{name}");
            assert!(ev.spell_menu().is_none(), "{name}");
        }
        // Wheel events of no whole line scroll nothing: the press on a row
        // after them still chooses it.
        for delta in no_whole_line() {
            let mut ev = spelled_menu();
            let (messages, _, _) = popover_batch(
                &mut ev,
                &mut None,
                vec![
                    (wheel(delta), Some(over(1.0, 0.0))),
                    (left(), Some(on_spelled_row(1))),
                ],
            );
            assert!(
                matches!(messages.as_slice(), [Message::Spell(SpellEvent::Accept(1))]),
                "{delta:?}: {messages:?}"
            );
            assert_eq!(ev.text(), "cat ten", "{delta:?}");
        }
        // The host's menus alike.
        for kind in BackendKind::ALL {
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let (_, _, choice) = popover_batch(
                &mut ev,
                &mut None,
                vec![(left(), on_text), (left(), Some(on_task_row(2)))],
            );
            assert_eq!(choice, None, "{kind:?}");
            assert!(ev.popover().is_none(), "{kind:?}");
        }
    }

    /// Backspace and then Tab in one batch on a task line whose suggestions
    /// the deletion asks again: the widget cannot know what the host
    /// offers after a deletion, so Tab is the text's, as with nothing open
    /// (item 3.1), and indents. It writes no suggestion, and closes the
    /// ones the host opened (ring review, finding 7).
    #[test]
    fn backspace_then_tab_in_one_batch_indents_and_takes_no_suggestion() {
        use keyboard::key::Named;
        let text = "- a\n- [ ] Call to";
        for kind in BackendKind::ALL {
            let mut ev = editing(kind, text, 1, 13);
            ev.set_suggest_source(Some(Arc::new(fixtures::TaskWords)));
            ev.perform(Action::Edit(Edit::Insert('d')));
            assert_eq!(
                ev.popover().is_some(),
                kind == BackendKind::Owned,
                "{kind:?}: suggestions for `tod`"
            );
            let (messages, _, choice) =
                keys_batch(&mut ev, vec![named(Named::Backspace), named(Named::Tab)]);
            assert!(
                !messages.iter().any(|message| matches!(
                    message,
                    Message::Spell(SpellEvent::Key(_) | SpellEvent::AcceptOf { .. })
                )),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(choice, None, "{kind:?}");
            assert!(ev.popover().is_none(), "{kind:?}");
            let line = ev.line_text(1).unwrap();
            assert!(
                line.starts_with(char::is_whitespace) && line.ends_with("- [ ] Call to"),
                "{kind:?}: {line:?}"
            );
        }
    }

    /// Enter and Tab on suggestions with a bound row (owners) name them, as
    /// on suggestions with a row that runs a command: queued while the
    /// host refreshes them into others, they choose nothing (ring review,
    /// finding 3).
    #[test]
    fn tab_on_bound_suggestions_names_them() {
        use keyboard::key::Named;
        for accept in [Named::Tab, Named::Enter] {
            let source = Arc::new(fixtures::Owners::new(&["sam", "samantha"]));
            let mut ev = editing(BackendKind::Owned, "", 0, 0);
            ev.set_suggest_source(Some(source.clone()));
            for c in ['@', 's', 'a'] {
                ev.perform(Action::Edit(Edit::Insert(c)));
            }
            let a = ev.popover().expect("owners").instance();
            let (messages, captured) = widget_batch(&ev, &mut None, vec![(named(accept), None)]);
            assert_eq!(captured, [true], "{accept:?}");
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Spell(SpellEvent::AcceptOf { menu, row: None })] if *menu == a
                ),
                "{accept:?}: {messages:?}"
            );
            // The ranking changes, and the host refreshes the suggestions.
            source.rank(&["samantha", "sam"]);
            assert!(ev.refresh_suggestions());
            let b = ev.popover().unwrap().instance();
            assert_ne!(a, b);
            for message in messages {
                assert_eq!(host_applies(&mut ev, message).choice, None, "{accept:?}");
            }
            assert_eq!(ev.popover().map(crate::Popover::instance), Some(b));
            assert_eq!(ev.text(), "@sa", "{accept:?}");

            // Opened by the batch's own typing, which the host has not
            // shown and may rank otherwise when it applies the typing: Tab
            // is dropped, choosing nothing and neither indenting nor typing,
            // and Enter is the text's, choosing nothing. In the next batch
            // the key chooses what is shown.
            let source = Arc::new(fixtures::Owners::new(&["sam", "samantha"]));
            let mut ev = editing(BackendKind::Owned, "- [ ] call ", 0, 11);
            ev.set_suggest_source(Some(source.clone()));
            let mut state = None;
            let events = [typed("@"), typed("s"), typed("a"), named(accept)]
                .into_iter()
                .map(|event| (event, None))
                .collect();
            let (messages, captured) = widget_batch(&ev, &mut state, events);
            assert_eq!(captured, [true; 4], "{accept:?}");
            let enter = accept == Named::Enter;
            assert_eq!(
                messages.len(),
                3 + usize::from(enter),
                "{accept:?}: {messages:?}"
            );
            source.rank(&["samantha", "sam"]);
            for message in messages {
                assert_eq!(host_applies(&mut ev, message).choice, None, "{accept:?}");
            }
            if enter {
                assert_eq!(ev.text(), "- [ ] call @sa\n- [ ] ");
                continue;
            }
            assert_eq!(ev.text(), "- [ ] call @sa", "{accept:?}");
            assert_eq!(ev.popover().unwrap().labels()[0], "@samantha");
            let (_, _, choice) = popover_batch(&mut ev, &mut state, vec![(named(accept), None)]);
            assert_eq!(choice.map(|choice| choice.id), Some(0), "{accept:?}");
        }
    }

    /// iced hands the overlay one place of the pointer for a whole batch,
    /// where it ends: only the batch's first press may choose a row (ring
    /// review, second round, finding 1). A later press on the popover as
    /// drawn is dropped there with its release, so it never reaches the
    /// text or a task's box under it (third round, finding 4). A row
    /// pressed after a middle press or a right press that chose nothing is
    /// not chosen either; the user presses it again.
    #[test]
    fn only_a_batchs_first_press_may_choose_a_row_and_later_ones_on_it_are_dropped() {
        let left = || press(mouse::Button::Left);
        // One place for every event of the batch, as iced gives it.
        let at = |events: Vec<Event>, point: Point| -> Vec<(Event, Option<Point>)> {
            events
                .into_iter()
                .map(|event| (event, Some(point)))
                .collect()
        };
        let row = on_spelled_row(1);
        // A known limit of the toolkit's, not a wanted result: a press in
        // the text, a move onto a row and a press there all come with the
        // pointer on the row, so the first press is taken as made there
        // and chooses it. What is pinned is that it is chosen at most once,
        // and the second press is dropped on the menu, no click.
        let mut ev = spelled_menu();
        let events = at(vec![left(), moved_to(row), left()], row);
        let (messages, captured, _) = popover_batch(&mut ev, &mut None, events);
        let accepted = (messages.iter())
            .filter(|message| matches!(message, Message::Spell(SpellEvent::Accept(_))))
            .count();
        assert!(accepted <= 1, "{messages:?}");
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, Message::Action(_))),
            "{messages:?}"
        );
        assert!(captured[0] && captured[2], "{captured:?}");
        // A press that chooses nothing, then a press on a row and its
        // release: the row is not chosen, and the press and the release
        // are dropped on the menu, which stays.
        for first in [mouse::Button::Middle, mouse::Button::Right] {
            let mut ev = spelled_menu();
            let (messages, captured, _) = popover_batch(
                &mut ev,
                &mut None,
                at(vec![press(first), left(), released()], row),
            );
            assert!(messages.is_empty(), "{first:?}: {messages:?}");
            assert_eq!(captured, [true, true, true], "{first:?}");
            assert_eq!(ev.text(), "cat teh", "{first:?}");
            assert!(ev.spell_menu().is_some(), "{first:?}");
        }
        // The host's menus alike, on both engines.
        for kind in BackendKind::ALL {
            let row = on_task_row(2);
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let (messages, captured, choice) = popover_batch(
                &mut ev,
                &mut None,
                at(
                    vec![press(mouse::Button::Middle), left(), released(), left()],
                    row,
                ),
            );
            assert_eq!(choice, None, "{kind:?}");
            assert!(messages.is_empty(), "{kind:?}: {messages:?}");
            assert_eq!(captured, [true; 4], "{kind:?}");
            assert_eq!(ev.text(), TASK, "{kind:?}");
            // Suggestions over the text alike: no click, no box click.
            let mut ev = on_task(kind, Mode::Suggest, status_rows());
            let (messages, captured, choice) = popover_batch(
                &mut ev,
                &mut None,
                at(vec![press(mouse::Button::Middle), left(), released()], row),
            );
            assert_eq!(choice, None, "{kind:?}");
            assert!(messages.is_empty(), "{kind:?}: {messages:?}");
            assert_eq!(captured, [true; 3], "{kind:?}");
            let (messages, _) = widget_batch(&ev, &mut None, vec![(later(), Some(row))]);
            assert_eq!(box_clicks(&messages), 0, "{kind:?}");
            let mut ev = on_task(kind, Mode::Menu, status_rows());
            let (_, _, choice) = popover_batch(&mut ev, &mut None, at(vec![left(), left()], row));
            assert_eq!(choice, chose(22, Mode::Menu), "{kind:?}");
        }
    }

    /// A key that reached the open menu, then a press on a row, in one
    /// batch: the press chooses nothing and is dropped on the menu, and the
    /// key is routed against the menu it was pressed on, so a disabled
    /// row's accelerator stays inert and types nothing (ring review, third
    /// round, finding 3).
    #[test]
    fn a_key_before_a_press_in_one_batch_leaves_the_press_no_row() {
        use keyboard::key::Named;
        for kind in BackendKind::ALL {
            let rows = || {
                let mut rows = status_rows();
                rows.push(
                    Row::new(23, "Cancelled")
                        .accelerator(crate::Accelerator::plain('q'))
                        .enabled(false),
                );
                rows
            };
            let row = on_task_row(2);
            for key in [typed("q"), named(Named::ArrowDown), named(Named::ArrowUp)] {
                let mut ev = on_task(kind, Mode::Menu, rows());
                let (messages, captured, choice) = popover_batch(
                    &mut ev,
                    &mut None,
                    vec![
                        (key.clone(), Some(row)),
                        (press(mouse::Button::Left), Some(row)),
                        (released(), Some(row)),
                    ],
                );
                assert_eq!(choice, None, "{kind:?} {key:?}: {messages:?}");
                assert!(
                    !messages.iter().any(|message| matches!(
                        message,
                        Message::Action(_) | Message::Spell(SpellEvent::AcceptOf { .. })
                    )),
                    "{kind:?} {key:?}: {messages:?}"
                );
                assert_eq!(captured, [true; 3], "{kind:?} {key:?}");
                assert_eq!(ev.text(), TASK, "{kind:?} {key:?}");
                assert!(ev.popover().is_some(), "{kind:?} {key:?}");
            }
        }
    }

    /// A row that opens a menu, clicked alone in its batch while an input
    /// method composes: the widget sees none of that batch, and the
    /// composition's commit in a later one is dropped all the same (ring
    /// review, second round, finding 2).
    #[test]
    fn a_composition_under_a_row_clicked_alone_commits_nothing_later() {
        for kind in BackendKind::ALL {
            let rows = || {
                let mut rows = status_rows();
                rows.push(Row::new(29, "Priority…").opens_menu());
                rows
            };
            let mut ev = on_task(kind, Mode::Menu, rows());
            let mut state = composing_state(&ev);
            let row = on_task_row(3);
            let (messages, captured, choice) = popover_batch(
                &mut ev,
                &mut state,
                vec![(press(mouse::Button::Left), Some(row))],
            );
            assert_eq!((messages.len(), captured), (1, vec![true]), "{kind:?}");
            assert_eq!(choice, chose(29, Mode::Menu), "{kind:?}");
            let (messages, _, _) = popover_batch(&mut ev, &mut state, vec![committing("き")]);
            assert!(messages.is_empty(), "{kind:?}: {messages:?}");
            assert_eq!(ev.text(), TASK, "{kind:?}");
            assert!(!state.as_ref().unwrap().is_composing(), "{kind:?}");
            // A later composition is the note's again.
            let (messages, _, _) =
                popover_batch(&mut ev, &mut state, vec![composing("か"), committing("か")]);
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Action(Action::Edit(Edit::Paste(text)))] if text.as_str() == "か"
                ),
                "{kind:?}: {messages:?}"
            );

            // A row that opens nothing leaves the composition alone.
            let mut ev = on_task(kind, Mode::Menu, rows());
            let mut state = composing_state(&ev);
            popover_batch(
                &mut ev,
                &mut state,
                vec![(press(mouse::Button::Left), Some(on_task_row(2)))],
            );
            assert!(state.as_ref().unwrap().is_composing(), "{kind:?}");
        }
    }

    /// A row of suggestions that opens a menu ("Due date…"), clicked, then
    /// Escape in the batch: the Escape is the menu's, dropped, as after the
    /// row's key (ring review, second round, finding 5). Ctrl+Escape is
    /// any other key, the host's, and claims nothing (finding 7).
    #[test]
    fn escape_after_a_suggestion_clicked_that_opens_a_menu_is_dropped() {
        use keyboard::key::Named;
        let rows = || vec![Row::new(1, "today"), Row::new(2, "Due date…").opens_menu()];
        for kind in BackendKind::ALL {
            let ev = on_task(kind, Mode::Suggest, rows());
            let events = vec![
                (press(mouse::Button::Left), Some(on_task_row(1))),
                (named(Named::Escape), None),
            ];
            let (messages, captured) = widget_batch(&ev, &mut None, events);
            assert!(
                matches!(
                    messages.as_slice(),
                    [
                        Message::Spell(SpellEvent::Accept(1)),
                        Message::Spell(SpellEvent::Stroke(stroke)),
                    ] if stroke.named() == Some(Named::Escape)
                ),
                "{kind:?}: {messages:?}"
            );
            assert_eq!(captured, [true, true], "{kind:?}");

            // Ctrl+Escape after it, or after a dialog's row or request.
            let ctrl_escape = || {
                key(
                    keyboard::Key::Named(Named::Escape),
                    keyboard::Modifiers::CTRL,
                    None,
                )
            };
            let ev = on_task(kind, Mode::Suggest, rows());
            let events = vec![
                (press(mouse::Button::Left), Some(on_task_row(1))),
                (ctrl_escape(), None),
            ];
            let (messages, _) = widget_batch(&ev, &mut None, events);
            assert!(
                matches!(messages.as_slice(), [Message::Spell(SpellEvent::Accept(1))]),
                "{kind:?}: {messages:?}"
            );
            let ev = requesting(kind, TASK, 0, 8);
            let events = vec![(request_key("d"), None), (ctrl_escape(), None)];
            let (messages, _) = widget_batch(&ev, &mut None, events);
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Spell(SpellEvent::Request(DIALOG))]
                ),
                "{kind:?}: {messages:?}"
            );
        }
    }

    /// A preedit of the note's own, under way in the widget's state.
    fn composing_state(ev: &crate::EditorView) -> Option<State> {
        let editor = editor_with(&ev.content, true, Measure::default());
        let mut state = laid_out_state(&editor);
        state.focus = Some(Focus::now());
        state.preedit = Some(input_method::Preedit {
            content: "\u{304d}".into(),
            selection: None,
            text_size: None,
        });
        assert!(state.is_composing());
        Some(state)
    }

    /// After the host gave back the keyboard: each batch of `batches` on a
    /// widget built afresh for it (from `state`, or laid out anew for
    /// `None`), its messages carried out by the host; what was pasted.
    fn after_handback(
        ev: &mut crate::EditorView,
        fresh: bool,
        batches: Vec<Vec<(Event, Option<Point>)>>,
    ) -> Vec<Vec<String>> {
        let mut state = None;
        batches
            .into_iter()
            .map(|batch| {
                if fresh {
                    state = None;
                }
                let (messages, _) = widget_batch(ev, &mut state, batch);
                for message in &messages {
                    host_applies(ev, message.clone());
                }
                messages
                    .iter()
                    .filter_map(|message| match message {
                        Message::Action(Action::Edit(Edit::Paste(text))) => Some(text.to_string()),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn after_a_handback_commits_are_dropped_until_the_editors_own_input() {
        let handed = || {
            let mut ev = providing(BackendKind::Owned, vec![Provider::Spelling]);
            ev.hand_back();
            ev
        };
        let none: Vec<String> = Vec::new();
        // The field's composition goes on after it closed: its preedits
        // that empty and its commit are dropped, however the widget is
        // rebuilt between the close and the commit.
        for fresh in [false, true] {
            let mut ev = handed();
            let pasted = after_handback(
                &mut ev,
                fresh,
                vec![vec![composing("")], vec![committing("\u{91d1}")]],
            );
            assert_eq!(pasted, [none.clone(), none.clone()], "{fresh}");
            assert!(ev.handed_back(), "{fresh}: nothing of the editor's own yet");
            assert_eq!(ev.text(), PROVIDED);
        }
        // The reviewer's sequences: a commit after a close mid-composition;
        // after a preedit that emptied; the same again after a second
        // calendar; and an abandoned composition, then keys.
        let mut ev = handed();
        let pasted = after_handback(&mut ev, true, vec![vec![committing("fri")]]);
        assert_eq!(pasted, std::slice::from_ref(&none));
        let mut ev = handed();
        let pasted = after_handback(&mut ev, false, vec![vec![composing(""), committing("fri")]]);
        assert_eq!(pasted, std::slice::from_ref(&none));
        // Ending (a): the editor's own preedit with text starts its own
        // composition, which it takes to its commit, in the same batch or
        // the next.
        let mut ev = handed();
        let pasted = after_handback(
            &mut ev,
            true,
            vec![
                vec![committing("x")],
                vec![composing("\u{304d}"), committing("\u{91d1}")],
            ],
        );
        assert_eq!(pasted, [none.clone(), vec!["\u{91d1}".to_owned()]]);
        assert!(!ev.handed_back());
        // A second handback, then the note's own composition over two
        // batches: taken.
        ev.hand_back();
        let pasted = after_handback(
            &mut ev,
            false,
            vec![vec![composing("\u{304d}")], vec![committing("\u{91d1}")]],
        );
        assert_eq!(pasted, [none.clone(), vec!["\u{91d1}".to_owned()]]);
        // Ending (b): a key that types, after an abandoned composition sent
        // nothing more: the commit after it is the note's.
        let mut ev = handed();
        let pasted = after_handback(
            &mut ev,
            false,
            vec![
                vec![(typed("y"), None), committing("x")],
                vec![committing("z")],
            ],
        );
        assert_eq!(pasted, [vec!["x".to_owned()], vec!["z".to_owned()]]);
        assert!(!ev.handed_back());
        // A key that only moves the caret ends it as well.
        let mut ev = handed();
        let pasted = after_handback(
            &mut ev,
            false,
            vec![vec![
                (named(keyboard::key::Named::ArrowLeft), None),
                committing("x"),
            ]],
        );
        assert_eq!(pasted, [vec!["x".to_owned()]]);
        assert!(!ev.handed_back());
        // Ending (c): a primary press in the text.
        let mut ev = handed();
        let pasted = after_handback(
            &mut ev,
            false,
            vec![vec![
                (press(mouse::Button::Left), Some(over(1.0, 0.0))),
                committing("x"),
            ]],
        );
        assert_eq!(pasted, [vec!["x".to_owned()]]);
        assert!(!ev.handed_back());
        // Ending (d): a new handover.
        let (mut ev, opener) = keyboard_requested();
        ev.hand_back();
        let _ = after_handback(&mut ev, false, vec![vec![opener]]);
        assert!(!ev.handed_back());
        // The cost: a commit with no preedit as the very first input is
        // dropped, and the input after it is the note's.
        let mut ev = handed();
        let pasted = after_handback(
            &mut ev,
            false,
            vec![vec![committing("x")], vec![(typed("y"), None)]],
        );
        assert_eq!(pasted, [none.clone(), none]);
        assert!(ev.text().starts_with('y') || ev.text().contains('y'));
        assert!(!ev.handed_back());
    }

    #[test]
    fn what_an_input_method_composes_in_the_frame_of_a_handover_is_dropped() {
        use keyboard::key::Named;
        // The messages of each batch in turn, carried out by the host as
        // they come; then the widget's state.
        let run = |ev: &mut crate::EditorView,
                   mut state: Option<State>,
                   batches: Vec<Vec<(Event, Option<Point>)>>| {
            let mut published = Vec::new();
            for batch in batches {
                let (messages, _) = widget_batch(ev, &mut state, batch);
                for message in &messages {
                    host_applies(ev, message.clone());
                }
                published.push(messages);
            }
            (published, state.unwrap())
        };
        let opens = |messages: &[Message]| {
            matches!(
                messages,
                [Message::Spell(
                    SpellEvent::Context { .. } | SpellEvent::Request(KEYBOARD)
                )]
            )
        };
        for (name, opened) in [
            ("a right-click", date_clicked as fn() -> _),
            ("a request", keyboard_requested),
        ] {
            // A composition begun and committed after the handover, in
            // its batch: none of it is the note's, and none of it is
            // handed on. The keys between are dropped with it.
            let (mut ev, opener) = opened();
            let batch = vec![
                opener.clone(),
                composing("\u{304d}"),
                composing("\u{304d}\u{3093}"),
                (named(Named::Backspace), None),
                composing(""),
                committing("\u{91d1}\u{66dc}"),
                (typed("z"), None),
            ];
            let (published, state) = run(&mut ev, None, vec![batch]);
            assert!(opens(&published[0]), "{name}: {published:?}");
            assert!(state.preedit.is_none() && !state.is_composing(), "{name}");
            assert_eq!(state.dropped, Dropped::Nothing, "{name}");
            assert_eq!(ev.text().as_bytes(), PROVIDED.as_bytes(), "{name}");

            // Its commit comes a batch later, with the editor still
            // holding the keyboard, even after a click back into the
            // text: it is discarded, never pasted.
            let (mut ev, opener) = opened();
            let batches = vec![
                vec![
                    opener.clone(),
                    composing("\u{304d}"),
                    (press(mouse::Button::Left), Some(over(1.0, 1.0))),
                ],
                vec![composing("\u{304d}\u{3093}")],
                vec![composing(""), committing("\u{91d1}")],
            ];
            let (published, state) = run(&mut ev, None, batches);
            assert!(
                matches!(
                    published[0].as_slice(),
                    [_, Message::Action(Action::Click(_))]
                ),
                "{name}: {published:?}"
            );
            assert!(
                published[1].is_empty() && published[2].is_empty(),
                "{name}: {published:?}"
            );
            assert!(!state.is_composing(), "{name}");
            assert_eq!(state.dropped, Dropped::Nothing, "{name}");
            assert_eq!(ev.text().as_bytes(), PROVIDED.as_bytes(), "{name}");

            // Once that composition is over, by its commit or with none,
            // the next one is the note's again.
            for ended in [
                vec![composing(""), committing("\u{91d1}")],
                vec![composing(""), (named(Named::ArrowRight), None)],
            ] {
                let (mut ev, opener) = opened();
                let batches = vec![
                    vec![opener, composing("\u{304d}")],
                    ended,
                    vec![composing("\u{306b}"), composing(""), committing("\u{65e5}")],
                ];
                let before = ev.cursor().position.line;
                let (published, state) = run(&mut ev, None, batches);
                assert!(published[1]
                    .iter()
                    .all(|m| matches!(m, Message::Action(Action::Move(_)))));
                assert_eq!(state.dropped, Dropped::Nothing, "{name}");
                let line = ev.line_text(before).unwrap();
                assert_eq!(line.matches('\u{65e5}').count(), 1, "{name}: {line}");
                assert!(!ev.text().contains('\u{91d1}'), "{name}");
            }
        }

        // A composition the note had under way when a provider takes the
        // keyboard is cancelled there. The preedit that follows leaves no
        // key to the note, and what the composition commits is not the
        // note's, in that batch or after.
        let ends = || {
            vec![
                composing("\u{304d}\u{3093}"),
                composing(""),
                committing("\u{91d1}"),
            ]
        };
        let keys = || vec![(typed("x"), None), (named(Named::Enter), None)];
        for (name, batches) in [
            ("in its batch", vec![[ends(), keys()].concat()]),
            (
                "emptied, then keys",
                vec![[vec![composing("")], keys()].concat()],
            ),
            ("in the next batch", vec![keys(), ends()]),
        ] {
            let (mut ev, click) = date_clicked();
            let state = composing_state(&ev);
            let mut batches = batches;
            batches[0].insert(0, click);
            let (published, state) = run(&mut ev, state, batches);
            assert!(opens(&published[0]), "{name}: {published:?}");
            assert!(
                published.iter().skip(1).all(Vec::is_empty),
                "{name}: {published:?}"
            );
            assert!(!state.is_composing(), "{name}");
            assert_eq!(ev.text().as_bytes(), PROVIDED.as_bytes(), "{name}");
        }
        // A click that opens a menu leaves a composition to the note.
        let providers = vec![Provider::Spelling, Provider::Host(provider(on_task_lines))];
        let mut ev = providing(BackendKind::Owned, providers);
        let state = composing_state(&ev);
        let (_, state) = run(&mut ev, state, vec![vec![right_click(cell::MAIL, 0.0)]]);
        assert!(state.is_composing() && state.dropped == Dropped::Nothing);

        // With no one taking the keyboard, a composition is the note's,
        // as it was.
        let (mut ev, _) = date_clicked();
        let batch = vec![composing("\u{304d}"), composing(""), committing("\u{91d1}")];
        run(&mut ev, None, vec![batch]);
        assert_eq!(ev.line_text(1).unwrap(), "plain\u{91d1} teh text");
    }

    #[test]
    fn a_dropped_composition_ends_with_its_commit_or_with_none() {
        // Not held: nothing is dropped, and nothing is remembered.
        let mut dropped = Dropped::Nothing;
        assert!(!dropped.preedit(false, false) && !dropped.preedit(true, false));
        assert!(!dropped.commit(false));
        // Held: every preedit and commit is dropped, and a composition
        // that began is remembered past the batch.
        assert!(dropped.preedit(true, true) && dropped == Dropped::Nothing);
        assert!(dropped.commit(true) && dropped == Dropped::Nothing);
        assert!(dropped.preedit(false, true) && dropped == Dropped::Composing);
        // Its later preedits are dropped, and a key pressed meanwhile
        // changes nothing.
        dropped.pressed();
        assert!(dropped.preedit(false, false) && dropped == Dropped::Composing);
        // It empties, and commits: dropped, and over.
        assert!(dropped.preedit(true, false) && dropped == Dropped::Ending);
        assert!(dropped.commit(false) && dropped == Dropped::Nothing);
        assert!(!dropped.commit(false));
        // It empties, and a press shows it committed nothing: the commit
        // after that is another's.
        dropped = Dropped::Composing;
        assert!(dropped.preedit(true, false));
        dropped.pressed();
        assert!(dropped == Dropped::Nothing && !dropped.commit(false));
        // It empties, and another composition begins: the note's.
        dropped = Dropped::Ending;
        assert!(!dropped.preedit(false, false) && dropped == Dropped::Nothing);
        // But one that begins while the host still holds the keyboard
        // is dropped as the first was.
        dropped = Dropped::Ending;
        assert!(dropped.preedit(false, true) && dropped == Dropped::Composing);
    }

    /// A provider whose answer for a task line changes when it is told
    /// to: the status menu, then the token menu under the same range.
    #[derive(Default)]
    struct Flips(std::sync::atomic::AtomicBool);

    impl crate::MenuProvider for Flips {
        fn menu(&self, clicked: &Clicked) -> Option<Requested> {
            let rows = if self.0.load(std::sync::atomic::Ordering::Relaxed) {
                token_rows()
            } else {
                status_rows()
            };
            (clicked.text.starts_with("- [")).then_some(Requested::Rows { range: 2..5, rows })
        }
    }

    #[test]
    fn a_right_click_made_over_another_text_or_menu_opens_nothing_and_chooses_no_row() {
        // The status menu for a place in no span, the token menu for an
        // owner or a tag.
        fn either(clicked: &Clicked) -> Option<Requested> {
            on_tokens(clicked).or_else(|| on_task_lines(clicked))
        }
        // A right-click on the task and the second row's key, in one
        // batch: the widget vouches for the status menu, and publishes
        // the key as a row of it.
        let published = |ev: &crate::EditorView| {
            let events = vec![right_click(cell::MAIL, 0.0), (typed("/"), None)];
            let (messages, _) = widget_batch(ev, &mut None, events);
            assert!(
                matches!(
                    spell_events(&messages).as_slice(),
                    [
                        SpellEvent::Context { provider: 1, .. },
                        SpellEvent::AcceptOf { row: Some(1), .. }
                    ]
                ),
                "{messages:?}"
            );
            messages
        };
        // What the host has open after each message, and the row chosen.
        let applied = |ev: &mut crate::EditorView, messages: Vec<Message>| {
            let mut open = Vec::new();
            let mut provided = None;
            for message in messages {
                provided = host_applies(ev, message).provided.or(provided);
                open.push(ev.popover().is_some());
            }
            (open, provided.map(|(provider, row)| (provider, row.id)))
        };
        let asks = provider(either);
        let providers = || vec![Provider::Spelling, Provider::Host(asks.clone())];

        // Nothing between: the menu opens and the row is chosen from it.
        let mut ev = providing(BackendKind::Owned, providers());
        let messages = published(&ev);
        assert_eq!(
            applied(&mut ev, messages),
            (vec![true, false], Some((1, 21)))
        );

        // The host opens a menu of its own between the click and the key:
        // the row was chosen from the provider's menu, which is no longer
        // the one open, so it is chosen from none. The host's menu stays,
        // with its second row unchosen.
        let mut ev = providing(BackendKind::Owned, providers());
        let messages = published(&ev);
        let [click, key] = messages.as_slice() else {
            panic!("{messages:?}");
        };
        host_applies(&mut ev, click.clone());
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        let mine = plain_rows(&["mine", "also mine"]);
        assert!(ev.open_popover(Anchor::Caret, Mode::Menu, mine));
        let chosen = host_applies(&mut ev, key.clone());
        assert_eq!((chosen.provided, chosen.choice), (None, None));
        assert_eq!(
            provided_menu(&ev).map(|menu| (menu.0, menu.2.len())),
            Some((None, 2))
        );

        // The host opens another note before it hands the click on, where
        // the same point is on an owner: the provider would answer with
        // another menu, and the key would choose its second row. Nothing
        // opens, no row is chosen, and the provider is not asked again.
        let other = "- [ ] @lee calls\nplain";
        let mut ev = providing(BackendKind::Owned, providers());
        let messages = published(&ev);
        let asked = asks.asked().len();
        ev.replace_text(other, false);
        assert_eq!(applied(&mut ev, messages), (vec![false, false], None));
        assert_eq!((ev.text().as_str(), asks.asked().len()), (other, asked));
        // In a batch of its own, the click there opens the token menu.
        ev.content.lay_out(BOUNDS.size());
        let messages = widget_batch(&ev, &mut None, vec![right_click(cell::MAIL, 0.0)]).0;
        applied(&mut ev, messages);
        assert_eq!(provided_menu(&ev).map(|menu| menu.2.len()), Some(2));

        // The same text loaded again is another text all the same.
        let mut ev = providing(BackendKind::Owned, providers());
        let messages = published(&ev);
        ev.replace_text(PROVIDED, true);
        assert_eq!(applied(&mut ev, messages), (vec![false, false], None));

        // The text as it was, but the provider now answers with another
        // menu for the place: not the menu the key chose from.
        let flips = Arc::new(Flips::default());
        let providers = vec![Provider::Spelling, Provider::Host(flips.clone())];
        let mut ev = providing(BackendKind::Owned, providers);
        let messages = published(&ev);
        flips.0.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(applied(&mut ev, messages), (vec![false, false], None));
        assert_eq!(ev.text(), PROVIDED);

        // The host names other providers before it hands the click on:
        // whoever has the place in the new list is not asked for it.
        let mut ev = providing(
            BackendKind::Owned,
            vec![Provider::Spelling, Provider::Host(asks.clone())],
        );
        let messages = published(&ev);
        let replaced = provider(either);
        ev.set_menu_providers(vec![Provider::Spelling, Provider::Host(replaced.clone())]);
        assert_eq!(applied(&mut ev, messages), (vec![false, false], None));
        assert!(replaced.asked().is_empty());

        // A provider that took the keyboard has it on the same terms.
        let dates = provider(on_dates);
        let mut ev = providing(
            BackendKind::Owned,
            vec![Provider::Spelling, Provider::Host(dates.clone())],
        );
        let (messages, _) = widget_batch(&ev, &mut None, vec![right_click(cell::DATE, 0.0)]);
        ev.replace_text(PROVIDED, true);
        for message in messages {
            assert_eq!(host_applies(&mut ev, message).keyboard, None);
        }
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
        RightClickOnATask,
        RightClickOffEveryProvider,
        RightClickForTheKeyboard,
        // A caret motion first, in the click's batch: the widget cannot
        // vouch for what is under the pointer.
        MotionThenRightClickOnAWord,
        MotionThenRightClickOnATask,
        MotionThenRightClickForTheKeyboard,
        RequestForAMenu,
        RequestForNothing,
        RequestForTheKeyboard,
        TypingThatOffers,
        TypingThatOffersNothing,
        SpellMenuOpen,
        MenuOpen,
        SuggestionsOpen,
        // A menu of the host's drawn, and in the press's batch the key of
        // a row whose choice opens another menu of the host's (item 3.4).
        MenuOpenThenARowThatOpensAMenu,
    }

    const OPENERS: [Opener; 19] = [
        Opener::Nothing,
        Opener::SuggestKeyOnAWord,
        Opener::SuggestKeyOffAWord,
        Opener::RightClickOnAWord,
        Opener::RightClickOnATask,
        Opener::RightClickOffEveryProvider,
        Opener::RightClickForTheKeyboard,
        Opener::MotionThenRightClickOnAWord,
        Opener::MotionThenRightClickOnATask,
        Opener::MotionThenRightClickForTheKeyboard,
        Opener::RequestForAMenu,
        Opener::RequestForNothing,
        Opener::RequestForTheKeyboard,
        Opener::TypingThatOffers,
        Opener::TypingThatOffersNothing,
        Opener::SpellMenuOpen,
        Opener::MenuOpen,
        Opener::SuggestionsOpen,
        Opener::MenuOpenThenARowThatOpensAMenu,
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
    /// key handling, and `D` no one, the press being dropped.
    fn owners(opener: Opener) -> &'static str {
        match opener {
            // After a motion in its batch, a click a provider of the
            // host's would take opens nothing, as one no provider takes.
            Opener::Nothing
            | Opener::SuggestKeyOffAWord
            | Opener::RightClickOffEveryProvider
            | Opener::MotionThenRightClickOnATask
            | Opener::MotionThenRightClickForTheKeyboard
            | Opener::RequestForNothing
            | Opener::TypingThatOffersNothing => "TTTTAATTTT",
            // The suggestions menu: Tab and a letter close it and are the
            // text's, and a press in the editor only closes it. A word
            // drawn as misspelled is asked for after a motion too.
            Opener::SuggestKeyOnAWord
            | Opener::RightClickOnAWord
            | Opener::MotionThenRightClickOnAWord
            | Opener::SpellMenuOpen => "PPPTPATTTP",
            // A menu of the host's, which has an accelerator.
            Opener::RightClickOnATask | Opener::RequestForAMenu | Opener::MenuOpen => "PPPTPATPTP",
            // The host took the keyboard: the keys its field would take
            // are dropped to the end of the batch. Escape and a shortcut
            // stay the host's, and a press in the editor is the click it
            // is.
            //
            // So it is once a row was chosen that opens a menu of the
            // host's the widget cannot know of (drop, don't forward).
            Opener::RightClickForTheKeyboard
            | Opener::RequestForTheKeyboard
            | Opener::MenuOpenThenARowThatOpensAMenu => "DDDDAADDDT",
            // Suggestions leave every other key and the press to the text.
            Opener::SuggestionsOpen => "PPPPPATTTT",
            // Those the batch's own typing opened, which the host has not
            // shown: Enter is the text's and Tab is dropped (ring review,
            // third round).
            Opener::TypingThatOffers => "PPTDPATTTT",
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
        // A right-click asks the suggestions menu first, then a provider
        // with a menu for a task line, then one that takes the keyboard on
        // any other line of text.
        ev.set_menu_providers(vec![
            Provider::Spelling,
            Provider::Host(provider(on_task_lines)),
            Provider::Host(provider(off_task_lines)),
        ]);
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
            Opener::MenuOpenThenARowThatOpensAMenu => {
                let mut rows = status_rows();
                rows.push(
                    Row::new(29, "Priority…")
                        .accelerator(crate::Accelerator::plain('p'))
                        .opens_menu(),
                );
                if owned {
                    let anchor = crate::Anchor::Range {
                        line: 0,
                        range: 2..5,
                    };
                    assert!(ev.open_popover(anchor, Mode::Menu, rows));
                } else {
                    hang(&mut ev, Mode::Menu, 0, 2..5, rows);
                }
            }
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

    /// A caret motion, which leaves the text as it is.
    fn motion() -> (Event, Option<Point>) {
        (named(keyboard::key::Named::ArrowLeft), None)
    }

    /// The events of `opener` that share the press's batch.
    fn opening_events(opener: Opener) -> Vec<(Event, Option<Point>)> {
        match opener {
            Opener::SuggestKeyOnAWord | Opener::SuggestKeyOffAWord => vec![(suggest_key(), None)],
            Opener::RightClickOnAWord => {
                vec![(press(mouse::Button::Right), Some(over(7.0, 0.0)))]
            }
            // The task's box, the second line, and under the text.
            Opener::RightClickOnATask => vec![right_click(3.0, 0.0)],
            Opener::RightClickForTheKeyboard => vec![right_click(1.0, 1.0)],
            Opener::RightClickOffEveryProvider => vec![right_click(1.0, 3.0)],
            Opener::MotionThenRightClickOnAWord => {
                vec![motion(), right_click(7.0, 0.0)]
            }
            Opener::MotionThenRightClickOnATask => vec![motion(), right_click(3.0, 0.0)],
            Opener::MotionThenRightClickForTheKeyboard => {
                vec![motion(), right_click(1.0, 1.0)]
            }
            Opener::RequestForAMenu => vec![(request_key("s"), None)],
            Opener::RequestForNothing => vec![(request_key("n"), None)],
            Opener::RequestForTheKeyboard => vec![(request_key("k"), None)],
            Opener::MenuOpenThenARowThatOpensAMenu => vec![(typed("p"), None)],
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
                    // A row of a provider's menu, which names it.
                    SpellEvent::AcceptOf { menu, .. } => {
                        let named = ev.popover().map(Popover::instance);
                        assert_eq!(named, Some(*menu), "{name}: {event:?}");
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
                let drawn = matches!(
                    opener,
                    Opener::MenuOpen
                        | Opener::SuggestionsOpen
                        | Opener::MenuOpenThenARowThatOpensAMenu
                );
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
                    let (published, captured) = widget_events(&ev, &mut state, events);
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
                        'P' => Some(Owner::Popover),
                        'T' => Some(Owner::Text),
                        'A' => Some(Owner::App),
                        // Dropped: the press is no one's.
                        _ => None,
                    };
                    let case = format!("{kind:?} {opener:?} {name}: {messages:?}");
                    assert_eq!(took, Vec::from_iter(owner), "{case}");
                    // And it was carried out once: a letter the text took
                    // is in the text once more, and what another owner
                    // took left the text alone.
                    let count = |text: &str| text.matches(name).count();
                    let typed = usize::from(owner == Some(Owner::Text) && name.len() == 1);
                    assert_eq!(count(&ev.text()), count(&before.0) + typed, "{case}");
                    if matches!(owner, Some(Owner::App) | None) {
                        assert_eq!((ev.text(), ev.cursor()), before, "{case}");
                    }
                    // A dropped press is taken by the widget, so that the
                    // host's key handling does not act on it either, and
                    // nothing is published for it: the note is what it
                    // was, byte for byte.
                    if owner.is_none() {
                        assert!(messages.is_empty() && captured[opening], "{case}");
                        assert_eq!(ev.text().as_bytes(), before.0.as_bytes(), "{case}");
                    }
                    // No Escape is left claimed for a later press.
                    assert!(!ev.take_claimed_escape(), "{case}");
                }
            }
        }
    }

    /// Enter on suggestions with rows that run commands names them, by the
    /// number the editor gave them; queued while others replace them, it
    /// chooses nothing. On those the batch's own letters open it is
    /// dropped.
    #[test]
    fn enter_on_command_suggestions_names_them() {
        use keyboard::key::Named;
        let commands = || {
            let mut ev = editing(BackendKind::Owned, "", 0, 0);
            ev.set_suggest_source(Some(Arc::new(fixtures::CommandSuggestions)));
            ev
        };
        let named_menu = |messages: &[Message]| match messages.last() {
            Some(Message::Spell(SpellEvent::AcceptOf { menu, row: None })) => Some(*menu),
            _ => None,
        };

        // Opened by the batch's own letters, which the host has not shown:
        // Tab is dropped and Enter is the text's, and no command runs (ring
        // review, third round). In the next batch they name the
        // suggestions shown.
        for accept in [Named::Enter, Named::Tab] {
            let mut ev = commands();
            let mut state = None;
            let (messages, captured) = widget_batch(
                &ev,
                &mut state,
                vec![
                    (typed("d"), None),
                    (typed("u"), None),
                    (named(accept), None),
                ],
            );
            assert_eq!(captured, [true; 3], "{accept:?}");
            assert_eq!(named_menu(&messages), None, "{messages:?}");
            for message in messages {
                host_applies(&mut ev, message);
            }
            assert_eq!(ev.take_command(), None, "{accept:?}");
            if accept == Named::Enter {
                assert_eq!(ev.text(), "du\n");
                let mut ev = commands();
                for c in ['d', 'u'] {
                    ev.perform(Action::Edit(Edit::Insert(c)));
                }
                let shown = ev.popover().expect("suggestions").instance();
                let (messages, _) = widget_batch(&ev, &mut None, vec![(named(accept), None)]);
                assert_eq!(named_menu(&messages), Some(shown), "{messages:?}");
                continue;
            }
            let shown = ev.popover().expect("suggestions").instance();
            let (messages, _) = widget_batch(&ev, &mut state, vec![(named(accept), None)]);
            assert_eq!(named_menu(&messages), Some(shown), "{messages:?}");
            let mut chosen = None;
            for message in messages {
                if let Message::Spell(event) = message {
                    ev.spell(event);
                    chosen = chosen.or(ev.take_command());
                }
            }
            assert_eq!(chosen.map(|choice| choice.id), Some(7), "{accept:?}");
        }

        // Open when the batch starts: named by their own number. Queued,
        // then replaced before the host applies it: nothing is chosen.
        let mut ev = commands();
        for c in ['d', 'u'] {
            ev.perform(Action::Edit(Edit::Insert(c)));
        }
        let a = ev.popover().expect("suggestions").instance();
        let (messages, _) = widget_batch(&ev, &mut None, vec![(named(Named::Enter), None)]);
        assert_eq!(named_menu(&messages), Some(a), "{messages:?}");
        assert!(ev.open_popover(
            crate::Anchor::Caret,
            Mode::Suggest,
            vec![Row::command(9, "nine")]
        ));
        let b = ev.popover().unwrap().instance();
        for message in messages {
            let Message::Spell(event) = message else {
                panic!("{message:?}");
            };
            assert!(!ev.spell(event));
        }
        assert_eq!(ev.take_command(), None);
        assert_eq!(ev.popover().map(crate::Popover::instance), Some(b));
        assert_eq!(ev.text(), "du");
    }

    // ── A click on a task's box ────────────────────────────────────────────

    fn released() -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
    }

    fn held(modifiers: keyboard::Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers))
    }

    /// The redraw asked for once a double click's time has passed.
    fn later() -> Event {
        Event::Window(window::Event::RedrawRequested(
            Instant::now() + DOUBLE_CLICK + Duration::from_millis(50),
        ))
    }

    /// An editor on `kind` over `text`, the caret at the end of its last
    /// line, with `tasks` on or off.
    fn boxed(kind: BackendKind, text: &str, tasks: bool) -> crate::EditorView {
        let last = text.lines().count().saturating_sub(1);
        let end = text.lines().last().map_or(0, str::len);
        let mut ev = editing(kind, text, last, end);
        ev.set_tasks(tasks.then(|| wren_markdown::TaskOptions {
            today: wren_markdown::date::Date::new(2026, 11, 19).expect("a date"),
        }));
        // Laid out as the widget lays it out, for [`middle`].
        widget_batch(&ev, &mut None, Vec::new());
        ev
    }

    /// The window point over the middle of the character at byte `column`
    /// of `line`, as `ev` lays it out in a widget at [`BOUNDS`].
    fn middle(ev: &crate::EditorView, line: usize, column: usize) -> Point {
        let at = ev.middle_of(line, column).expect("laid out");
        Point::new(at.x + 60.0, at.y + 24.0)
    }

    /// A press, a release at `at`, and once the time for a double click
    /// has passed, a redraw, each in a batch of its own on one widget
    /// state: what each publishes and whether the first two were captured.
    fn press_release(
        ev: &crate::EditorView,
        state: &mut Option<State>,
        at: Point,
    ) -> [(Vec<Message>, Vec<bool>); 3] {
        [
            widget_batch(ev, state, vec![(press(mouse::Button::Left), Some(at))]),
            widget_batch(ev, state, vec![(released(), Some(at))]),
            widget_batch(ev, state, vec![(later(), Some(at))]),
        ]
    }

    /// Whether `messages` are one click on the box of `line` of `ev`.
    fn is_box_click(ev: &crate::EditorView, messages: &[Message], line: usize) -> bool {
        matches!(messages, [Message::BoxClick(click)] if ev.box_clicked(*click) == Some(line))
    }

    fn box_clicks(messages: &[Message]) -> usize {
        (messages.iter())
            .filter(|message| matches!(message, Message::BoxClick(_)))
            .count()
    }

    #[test]
    fn a_click_on_a_tasks_box_is_a_box_click_once_no_double_click_can_follow() {
        let text = "intro\n- [ ] call sam";
        let ev = boxed(BackendKind::Owned, text, true);
        let caret_before = ev.cursor();
        // `[`, the symbol and `]`: the whole box, and nothing else.
        for column in [2, 3, 4] {
            let mut state = None;
            let at = middle(&ev, 1, column);
            let [(pressed, captured), (released, let_go), (settled, _)] =
                press_release(&ev, &mut state, at);
            assert!(pressed.is_empty(), "{column}: {pressed:?}");
            assert_eq!(captured, [true], "{column}");
            assert!(released.is_empty(), "{column}: it waits: {released:?}");
            assert_eq!(let_go, [true], "{column}");
            assert!(is_box_click(&ev, &settled, 1), "{column}: {settled:?}");
            assert!(state.unwrap().focus.is_some(), "{column}");
            assert_eq!(ev.cursor(), caret_before, "{column}");
        }
        // Pressed and let go in one batch, with the pointer moved a little
        // in between; a key after it lets the click go first.
        let at = middle(&ev, 1, 3);
        let nudged = Point::new(at.x + 2.0, at.y + 2.0);
        let (messages, _) = widget_batch(
            &ev,
            &mut None,
            vec![
                (press(mouse::Button::Left), Some(at)),
                (moved_to(nudged), Some(nudged)),
                (released(), Some(nudged)),
                (typed("a"), Some(nudged)),
            ],
        );
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::BoxClick(_),
                    Message::Action(Action::Edit(Edit::Insert('a')))
                ]
            ),
            "{messages:?}"
        );
        // The marker, the space after the box and the text are the text's.
        for column in [0, 5, 7] {
            let at = middle(&ev, 1, column);
            let [(pressed, _), (released, _), (settled, _)] = press_release(&ev, &mut None, at);
            assert!(
                matches!(pressed.as_slice(), [Message::Action(Action::Click(_))]),
                "{column}: {pressed:?}"
            );
            assert!(released.is_empty() && settled.is_empty(), "{column}");
        }
    }

    #[test]
    fn a_box_of_a_known_status_is_hit_on_each_of_its_glyphs() {
        // Doing, cancelled, and done with a priority glyph with U+FE0F
        // after it next to the box.
        let text = "- [/] a\n- [-] b\n- [x] ⏫\u{fe0f} c";
        let ev = boxed(BackendKind::Owned, text, true);
        for (line, columns, off) in [
            (0, vec![2, 3, 4], 5),
            (1, vec![2, 3, 4], 5),
            (2, vec![2, 3, 4], 6),
        ] {
            for column in columns {
                let [_, _, (settled, _)] = press_release(&ev, &mut None, middle(&ev, line, column));
                assert!(
                    is_box_click(&ev, &settled, line),
                    "{line} {column}: {settled:?}"
                );
            }
            let [(pressed, _), _, _] = press_release(&ev, &mut None, middle(&ev, line, off));
            assert!(
                matches!(pressed.as_slice(), [Message::Action(Action::Click(_))]),
                "{line} {off}: {pressed:?}"
            );
        }
    }

    /// A box of a symbol of no status of its own, one byte or several, is
    /// not cycled by a click, which would rewrite the user's symbol: the
    /// click is the text's and places the caret.
    #[test]
    fn a_click_on_a_box_of_an_unknown_symbol_is_a_click_in_the_text() {
        let text = "- [?] a\n- [界] b\n- [>] c\n- [1] d";
        for kind in BackendKind::ALL {
            let ev = boxed(kind, text, true);
            for (line, columns) in [
                (0, vec![2, 3, 4]),
                (1, vec![2, 3, 6]),
                (2, vec![2, 3, 4]),
                (3, vec![2, 3, 4]),
            ] {
                for column in columns {
                    let [(pressed, _), (released, _), (settled, _)] =
                        press_release(&ev, &mut None, middle(&ev, line, column));
                    assert!(
                        matches!(pressed.as_slice(), [Message::Action(Action::Click(_))]),
                        "{kind:?} {line} {column}: {pressed:?}"
                    );
                    assert!(
                        released.is_empty() && settled.is_empty(),
                        "{kind:?} {line} {column}: {released:?} {settled:?}"
                    );
                    assert!(
                        matches!(
                            ev.click_message(line, column),
                            Some(Message::Action(Action::Click(_)))
                        ),
                        "{kind:?} {line} {column}"
                    );
                }
            }
        }
        // Carried out, the click moves the caret into the box and leaves
        // the symbol as it is.
        let mut ev = boxed(BackendKind::Owned, text, true);
        let at = middle(&ev, 0, 3);
        let (pressed, _) =
            widget_batch(&ev, &mut None, vec![(press(mouse::Button::Left), Some(at))]);
        for message in pressed {
            host_applies(&mut ev, message);
        }
        assert_eq!(ev.cursor().position.line, 0);
        assert_eq!(ev.text(), text);
    }

    #[test]
    fn a_release_off_the_box_or_out_of_reach_is_a_click_in_the_text() {
        let ev = boxed(BackendKind::Owned, "- [x] task", true);
        // Pressed at the far edge of `]` and let go 2 px on, on the blank
        // after the box.
        let row = ev.content.anchor_row(0, 4..5).expect("laid out");
        let edge = Point::new(row.right - 1.0 + 60.0, (row.top + row.bottom) / 2.0 + 24.0);
        let past = Point::new(edge.x + 2.0, edge.y);
        let mut state = None;
        let (pressed, _) = widget_batch(
            &ev,
            &mut state,
            vec![(press(mouse::Button::Left), Some(edge))],
        );
        assert!(pressed.is_empty(), "the press is on the box: {pressed:?}");
        let (let_go, _) = widget_batch(&ev, &mut state, vec![(released(), Some(past))]);
        let origin = Point::new(edge.x - 60.0, edge.y - 24.0);
        assert!(
            matches!(let_go.as_slice(), [Message::Action(Action::Click(at))] if *at == origin),
            "{let_go:?}"
        );
        let (settled, _) = widget_batch(&ev, &mut state, vec![(later(), None)]);
        assert_eq!(box_clicks(&settled), 0);

        // A drag that leaves the editor: the move out of it is a drag from
        // where the box was pressed, and coming back is no click on it.
        let at = middle(&ev, 0, 3);
        let outside = Point::new(BOUNDS.x + BOUNDS.width + 40.0, at.y);
        let mut state = None;
        widget_batch(
            &ev,
            &mut state,
            vec![(press(mouse::Button::Left), Some(at))],
        );
        let (left, _) = widget_batch(&ev, &mut state, vec![(moved_to(outside), Some(outside))]);
        assert!(
            matches!(left.as_slice(), [Message::Action(Action::Click(_))]),
            "{left:?}"
        );
        let (back, _) = widget_batch(
            &ev,
            &mut state,
            vec![
                (moved_to(at), Some(at)),
                (released(), Some(at)),
                (later(), Some(at)),
            ],
        );
        assert_eq!(box_clicks(&back), 0, "{back:?}");
    }

    #[test]
    fn a_drag_from_the_box_selects_from_where_it_was_pressed() {
        let ev = boxed(BackendKind::Owned, "- [ ] call sam", true);
        let at = middle(&ev, 0, 3);
        let away = middle(&ev, 0, 9);
        let mut state = None;
        let (pressed, _) = widget_batch(
            &ev,
            &mut state,
            vec![(press(mouse::Button::Left), Some(at))],
        );
        assert!(pressed.is_empty(), "{pressed:?}");
        let (moved, _) = widget_batch(&ev, &mut state, vec![(moved_to(away), Some(away))]);
        let origin = Point::new(at.x - 60.0, at.y - 24.0);
        assert!(
            matches!(
                moved.as_slice(),
                [
                    Message::Action(Action::Click(from)),
                    Message::Action(Action::Drag(_))
                ] if *from == origin
            ),
            "{moved:?}"
        );
        let (released, _) = widget_batch(
            &ev,
            &mut state,
            vec![(released(), Some(away)), (later(), Some(away))],
        );
        assert!(released.is_empty(), "{released:?}");
    }

    #[test]
    fn a_double_click_on_the_box_selects_and_two_slow_clicks_cycle_twice() {
        let ev = boxed(BackendKind::Owned, "- [ ] call sam", true);
        let at = middle(&ev, 0, 3);
        // A double click: no click on the box, but the double click's
        // selection, from the place pressed.
        let mut state = None;
        let (messages, _) = widget_batch(
            &ev,
            &mut state,
            vec![
                (press(mouse::Button::Left), Some(at)),
                (released(), Some(at)),
                (press(mouse::Button::Left), Some(at)),
                (released(), Some(at)),
            ],
        );
        let origin = Point::new(at.x - 60.0, at.y - 24.0);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Action(Action::Click(from)),
                    Message::Action(Action::SelectWord)
                ] if *from == origin
            ),
            "{messages:?}"
        );
        let (settled, _) = widget_batch(&ev, &mut state, vec![(later(), Some(at))]);
        assert_eq!(box_clicks(&settled), 0, "{settled:?}");

        // A single click cycles once; two slow clicks, twice.
        let mut state = None;
        let [_, _, (first, _)] = press_release(&ev, &mut state, at);
        assert!(is_box_click(&ev, &first, 0), "{first:?}");
        let [_, _, (second, _)] = press_release(&ev, &mut state, at);
        assert!(is_box_click(&ev, &second, 0), "{second:?}");

        // A click with a modifier held is the text's.
        for modifiers in [
            keyboard::Modifiers::SHIFT,
            keyboard::Modifiers::CTRL,
            keyboard::Modifiers::ALT,
            keyboard::Modifiers::LOGO,
        ] {
            let mut state = None;
            widget_batch(&ev, &mut state, vec![(held(modifiers), None)]);
            let [(pressed, _), (released, _), (settled, _)] = press_release(&ev, &mut state, at);
            assert!(
                matches!(pressed.as_slice(), [Message::Action(Action::Click(_))]),
                "{modifiers:?}: {pressed:?}"
            );
            assert!(released.is_empty() && settled.is_empty(), "{modifiers:?}");
            // Let go of the modifier: the box is a box again.
            widget_batch(
                &ev,
                &mut state,
                vec![(held(keyboard::Modifiers::empty()), None)],
            );
            // A press elsewhere first, so the next is no double click.
            press_release(&ev, &mut state, middle(&ev, 0, 9));
            let [_, _, (settled, _)] = press_release(&ev, &mut state, at);
            assert!(is_box_click(&ev, &settled, 0), "{modifiers:?}: {settled:?}");
        }
    }

    #[test]
    fn what_comes_between_pressing_the_box_and_letting_go_undoes_the_click() {
        let ev = boxed(BackendKind::Owned, "- [ ] call sam", true);
        let at = middle(&ev, 0, 3);
        for between in [
            typed("a"),
            press(mouse::Button::Right),
            Event::Window(window::Event::Unfocused),
            held(keyboard::Modifiers::SHIFT),
        ] {
            let mut state = None;
            widget_batch(
                &ev,
                &mut state,
                vec![(press(mouse::Button::Left), Some(at))],
            );
            widget_batch(&ev, &mut state, vec![(between.clone(), Some(at))]);
            let (released, _) = widget_batch(
                &ev,
                &mut state,
                vec![(released(), Some(at)), (later(), Some(at))],
            );
            assert_eq!(box_clicks(&released), 0, "{between:?}: {released:?}");
        }
        // So does the editor losing the keyboard to a focus operation.
        let mut state = None;
        widget_batch(
            &ev,
            &mut state,
            vec![(press(mouse::Button::Left), Some(at))],
        );
        operation::Focusable::unfocus(state.as_mut().unwrap());
        let (released, _) = widget_batch(
            &ev,
            &mut state,
            vec![(released(), Some(at)), (later(), Some(at))],
        );
        assert_eq!(box_clicks(&released), 0, "{released:?}");
        // A click over a text that changed since is no one's.
        let mut ev = boxed(BackendKind::Owned, "- [ ] call sam", true);
        let [_, _, (settled, _)] = press_release(&ev, &mut None, at);
        let [Message::BoxClick(click)] = settled.as_slice() else {
            panic!("{settled:?}");
        };
        assert!(ev.perform(Action::Edit(Edit::Insert('!'))));
        assert_eq!(ev.box_clicked(*click), None);
    }

    #[test]
    fn the_box_is_the_texts_on_the_classic_engine_with_tasks_off_and_in_code() {
        let text = "- [ ] call\n```\n- [ ] code\n```";
        for (kind, tasks, line) in [
            (BackendKind::Iced, true, 0),
            (BackendKind::Owned, false, 0),
            (BackendKind::Owned, true, 2),
        ] {
            let ev = boxed(kind, text, tasks);
            let [(pressed, _), (released, _), (settled, _)] =
                press_release(&ev, &mut None, middle(&ev, line, 3));
            assert!(
                matches!(pressed.as_slice(), [Message::Action(Action::Click(_))]),
                "{kind:?} {tasks} {line}: {pressed:?}"
            );
            assert!(
                released.is_empty() && settled.is_empty(),
                "{kind:?} {tasks} {line}"
            );
            assert!(
                matches!(
                    ev.click_message(line, 3),
                    Some(Message::Action(Action::Click(_)))
                ),
                "{kind:?} {tasks} {line}"
            );
        }
        // The editor's own account of a click agrees with the widget's.
        let ev = boxed(BackendKind::Owned, text, true);
        assert!(matches!(
            ev.click_message(0, 3),
            Some(Message::BoxClick(click)) if ev.box_clicked(click) == Some(0)
        ));
        assert!(matches!(
            ev.click_message(0, 7),
            Some(Message::Action(Action::Click(_)))
        ));
    }

    // ── Rows that cannot be chosen ─────────────────────────────────────────

    /// The task menu's shape: two status rows, the first checked, a row
    /// that cannot be chosen with its key, one that can, and another that
    /// cannot at the end.
    fn shaped_rows() -> Vec<Row> {
        use crate::Accelerator;
        vec![
            Row::new(10, "Open").accelerator(Accelerator::plain(' ')),
            Row::new(11, "Doing")
                .accelerator(Accelerator::plain('/'))
                .checked(true),
            Row::new(12, "Set due date…")
                .accelerator(Accelerator::plain('d'))
                .enabled(false),
            Row::new(13, "Priority…").accelerator(Accelerator::plain('p')),
            Row::new(14, "Edit task…")
                .accelerator(Accelerator::plain('e'))
                .enabled(false),
        ]
    }

    #[test]
    fn a_menu_opens_on_its_checked_row_and_passes_over_rows_that_cannot_be_chosen() {
        use keyboard::key::Named;
        for kind in BackendKind::ALL {
            let mut ev = on_task(kind, Mode::Menu, shaped_rows());
            let popover = ev.popover().unwrap();
            assert_eq!(popover.selected(), Some(1), "{kind:?}: the checked row");
            assert!(popover.is_checked(1) && !popover.is_checked(0));
            assert!(!popover.is_enabled(2) && popover.is_enabled(3));
            assert_eq!(popover.hint(0).as_deref(), Some("Space"));
            assert_eq!(popover.hint(2).as_deref(), Some("D"));
            let menu = popover.instance();
            // Down passes the row that cannot be chosen, and stops before
            // the last, which cannot be chosen either; Up comes back.
            let (messages, _, choice) = keys_batch(
                &mut ev,
                vec![
                    named(Named::ArrowDown),
                    named(Named::ArrowDown),
                    named(Named::ArrowUp),
                    named(Named::ArrowDown),
                    named(Named::Enter),
                ],
            );
            assert_eq!(
                spell_events(&messages).last(),
                Some(&SpellEvent::AcceptOf { menu, row: None }),
                "{kind:?}"
            );
            assert_eq!(choice, chose(13, Mode::Menu), "{kind:?}");

            // Its key is taken and does nothing: the menu stays open, and
            // nothing is typed; a row's key after it still chooses.
            let mut ev = on_task(kind, Mode::Menu, shaped_rows());
            let (messages, captured, choice) = keys_batch(&mut ev, vec![typed("d"), typed("e")]);
            assert!(messages.is_empty(), "{kind:?}: {messages:?}");
            assert_eq!(captured, [true, true], "{kind:?}");
            assert_eq!(choice, None, "{kind:?}");
            assert!(ev.popover().is_some(), "{kind:?}");
            assert_eq!(ev.text(), TASK, "{kind:?}");
            let (_, _, choice) = keys_batch(&mut ev, vec![typed("p")]);
            assert_eq!(choice, chose(13, Mode::Menu), "{kind:?}");

            // The pointer neither selects nor chooses it.
            let mut ev = on_task(kind, Mode::Menu, shaped_rows());
            let row = on_task_row(2);
            let (messages, captured, choice) = popover_batch(
                &mut ev,
                &mut None,
                vec![
                    (moved_to(row), Some(row)),
                    (press(mouse::Button::Left), Some(row)),
                ],
            );
            assert!(messages.is_empty(), "{kind:?}: {messages:?}");
            assert_eq!(captured, [true, true], "{kind:?}");
            assert_eq!(choice, None, "{kind:?}");
            assert_eq!(ev.popover().and_then(crate::Popover::selected), Some(1));
            // Nor does a choice of it from elsewhere: the menu closes and
            // nothing is chosen.
            assert!(!ev.spell(SpellEvent::AcceptOf {
                menu: ev.popover().unwrap().instance(),
                row: Some(2)
            }));
            assert!(ev.popover().is_none() && ev.take_choice().is_none());
        }
    }

    #[test]
    fn a_menu_with_no_row_that_can_be_chosen_selects_none() {
        let rows = vec![
            Row::new(1, "a").enabled(false),
            Row::new(2, "b").enabled(false),
        ];
        let mut ev = on_task(BackendKind::Owned, Mode::Menu, rows);
        assert_eq!(ev.popover().unwrap().selected(), None);
        let (messages, _, choice) = keys_batch(&mut ev, vec![named(keyboard::key::Named::Enter)]);
        assert_eq!(choice, None, "{messages:?}");
        assert!(ev.popover().is_none());
    }
}
