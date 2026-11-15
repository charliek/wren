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
use crate::keys::{KeyPolicy, Routed};
use crate::proof::{
    self, publish, MenuOverlay, MenuPress, OpenMenu, Proofreader, ResolvedPress, SpellEvent,
    SpellMenu,
};
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

    /// The mark whose glyphs lie under `point`, relative to the text origin,
    /// among the marks `marks` gives for a line and its text; see
    /// [`mark_under`].
    pub fn mark_under(
        &self,
        point: Point,
        marks: impl FnMut(usize, &str) -> Vec<Range<usize>>,
    ) -> Option<(usize, Range<usize>)> {
        mark_under(&self.0.borrow(), point, marks)
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
    /// The open suggestions menu.
    spell_menu: Option<&'a SpellMenu>,
    /// The open menu as the events this widget has handled leave it; see
    /// [`OpenMenu`]. The host builds a new widget once it has applied them.
    menu_now: Cell<Option<OpenMenu>>,
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
            spell_menu: None,
            menu_now: Cell::new(None),
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
    pub fn spell_menu(mut self, menu: Option<&'a SpellMenu>) -> Self {
        self.spell_menu = menu;
        self.menu_now.set(menu.map(OpenMenu::of));
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

        // What the open menu is first laid out with, so that only a change
        // made while it is open closes it (rule 4.4.8).
        if self.spell_menu.is_none() {
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
            && self.menu_now.get().is_some()
        {
            publish(shell, &self.menu_now, Message::Spell(SpellEvent::Close));
        }
        let menu = self.menu_now.get();
        // Whether the open menu closes (rule 4.4.8).
        let mut closing = false;

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => {
                // The host closes the menu on Escape, so it is not told.
                self.menu_now.set(None);
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
            menu,
        ) {
            let update = match update {
                Update::Click(click) if self.left_to_host.get() => {
                    Update::Press(Pressed::Left(click))
                }
                update => update,
            };
            match update {
                Update::Click(click) => {
                    let kind = proof::click_kind(click.kind(), state.click_kind);
                    let action = match kind {
                        mouse::click::Kind::Single => Action::Click(click.position()),
                        mouse::click::Kind::Double => Action::SelectWord,
                        mouse::click::Kind::Triple => Action::SelectLine,
                    };
                    state.focus = Some(Focus::now());
                    state.last_click = Some(click);
                    state.click_kind = Some(kind);
                    state.drag_click = Some(kind);
                    publish(shell, &self.menu_now, Message::Action(action));
                    shell.capture_event();
                }
                Update::RightClick(position) => {
                    // Only a misspelled word answers; anywhere else the
                    // press leaves focus and the caret alone.
                    if self.misspelling_under(state, position).is_some() {
                        state.focus = Some(Focus::now());
                        state.drag_click = None;
                        publish(
                            shell,
                            &self.menu_now,
                            Message::Spell(SpellEvent::OpenAt(position)),
                        );
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
                    publish(shell, &self.menu_now, Message::Spell(SpellEvent::Close));
                    shell.capture_event();
                }
                Update::Press(press) => {
                    // Only the host knows whether a menu opened. The press
                    // is taken as it would be with none, and the host only
                    // closes the menu if one did.
                    let event = match press {
                        Pressed::Left(click) => {
                            let kind = proof::click_kind(click.kind(), state.click_kind);
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
                            MenuPress::Right(position)
                        }
                    };
                    publish(
                        shell,
                        &self.menu_now,
                        Message::Spell(SpellEvent::Press(event)),
                    );
                    shell.capture_event();
                }
                Update::Drag(position) => {
                    publish(
                        shell,
                        &self.menu_now,
                        Message::Action(Action::Drag(position)),
                    );
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
                    publish(
                        shell,
                        &self.menu_now,
                        Message::Action(Action::Scroll {
                            lines: lines as i32,
                        }),
                    );
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
                        publish(
                            shell,
                            &self.menu_now,
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
                            publish(shell, &self.menu_now, Message::Spell(SpellEvent::Close));
                            binding
                        }
                    };
                    if let Some(binding) = binding {
                        if !matches!(binding, Binding::Unfocus) {
                            shell.capture_event();
                        }
                        apply_binding(
                            binding,
                            self.content,
                            state,
                            clipboard,
                            shell,
                            &self.menu_now,
                        );
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
        if closing && self.menu_now.get().is_some() {
            publish(shell, &self.menu_now, Message::Spell(SpellEvent::Close));
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

    /// The open menu as an overlay of a widget at `bounds`: placed under
    /// its word's first row as the last layout left it (rule 4.4.3), or
    /// `None` when that row is out of view.
    fn menu_overlay<'b>(
        &'b self,
        state: &'b mut State,
        bounds: Rectangle,
    ) -> Option<MenuOverlay<'b>> {
        let menu = self.spell_menu.filter(|_| self.menu_now.get().is_some())?;
        let text_bounds = bounds.shrink(self.padding(bounds.width, state.advance.width));
        let (left, top, bottom) = word_row(&self.content.0.borrow(), menu)?;
        let style = self.style.menu;
        let key = FitKey {
            items: menu.items().to_vec(),
            font: style.font,
            text_size: style.text_size.to_bits(),
            width: bounds.width.to_bits(),
        };
        if state
            .menu_fit
            .as_ref()
            .is_none_or(|(fitted, _)| *fitted != key)
        {
            let fit = proof::fit(menu.items(), bounds.width, |text| {
                proof::measure(text, style.font, style.text_size)
            });
            state.menu_fit = Some((key, fit));
        }
        let state = &*state;
        let (_, fit) = state.menu_fit.as_ref()?;
        let size = Size::new(fit.width, proof::height(fit.labels.len()));
        Some(MenuOverlay {
            menu,
            labels: &fit.labels,
            bounds: proof::place(
                bounds,
                text_bounds.x + left,
                text_bounds.y + top,
                text_bounds.y + bottom,
                size,
            ),
            style,
            now: &self.menu_now,
            #[cfg(any(test, feature = "test-support"))]
            frame: self.spell_frame,
        })
    }

    /// The misspelled word under `point`, relative to the text origin, as
    /// the last layout placed it and the widget's highlighter found its
    /// prose. Only the owned engine has any (§3.3).
    fn misspelling_under(&self, state: &State, point: Point) -> Option<(usize, Range<usize>)> {
        let proofreader = self
            .proofreader
            .filter(|_| self.content.kind() == BackendKind::Owned)?;
        let highlighter = state.highlighter.borrow();
        self.content.mark_under(point, |line, text| {
            highlighter
                .decor(line)
                .map(|decor| proof::line_marks(proofreader, text, &decor.prose))
                .unwrap_or_default()
        })
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
    menu_fit: Option<(FitKey, proof::Fit)>,
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
            self.spell_menu,
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
        let state = tree.state.downcast_mut::<State>();
        let menu = self.menu_overlay(state, layout.bounds() + translation)?;
        Some(overlay::Element::new(Box::new(menu)))
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
    menu: &Cell<Option<OpenMenu>>,
) {
    let mut act = |action| publish(shell, menu, Message::Action(action));

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
                act(Action::Edit(Edit::Delete));
            }
        }
        Binding::Paste => {
            if let Some(contents) = clipboard.read(clipboard::Kind::Standard) {
                act(Action::Edit(Edit::Paste(Arc::new(contents))));
            }
        }
        Binding::Move(motion) => act(Action::Move(motion)),
        Binding::Select(motion) => act(Action::Select(motion)),
        Binding::SelectWord => act(Action::SelectWord),
        Binding::SelectLine => act(Action::SelectLine),
        Binding::SelectAll => act(Action::SelectAll),
        Binding::Insert(c) => act(Action::Edit(Edit::Insert(c))),
        Binding::Enter => act(Action::Edit(Edit::Enter)),
        Binding::Backspace => act(Action::Edit(Edit::Backspace)),
        Binding::Delete => act(Action::Edit(Edit::Delete)),
        Binding::Sequence(sequence) => {
            for binding in sequence {
                apply_binding(binding, content, state, clipboard, shell, menu);
            }
        }
        Binding::Custom(message) => publish(shell, menu, message),
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
        menu: Option<OpenMenu>,
    ) -> Option<Self> {
        match event {
            Event::Mouse(event) => match event {
                mouse::Event::ButtonPressed(button) if menu.is_some() && cursor.is_over(bounds) => {
                    let position = cursor
                        .position_in(bounds)
                        .map(|position| position - Vector::new(padding.left, padding.top));
                    Some(
                        match (menu.is_some_and(|menu| menu.opening), button, position) {
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
                        },
                    )
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
                    menu,
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
/// hidden, and the open menu's word, `menu`, tinted. Only the owned engine
/// asks the proofreader (§3.3).
fn visual_rows(
    editor: &Backend,
    highlighter: &MarkdownHighlighter,
    proofreader: Option<&dyn Proofreader>,
    caret: Option<Position>,
    menu: Option<&SpellMenu>,
) -> Vec<VisualRow> {
    let buffer = editor.buffer();
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
                tint: menu
                    .filter(|menu| menu.line() == run.line_i)
                    .and_then(|menu| glyph_span(run.text, glyphs(), menu.mark())),
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

/// Where `menu`'s word starts: the left edge of its glyphs on the first
/// visible row that holds any, and that row's top and bottom, relative to
/// the text origin.
fn word_row(editor: &Backend, menu: &SpellMenu) -> Option<(f32, f32, f32)> {
    editor
        .buffer()
        .layout_runs()
        .filter(|run| run.line_i == menu.line())
        .find_map(|run| {
            let glyphs = run
                .glyphs
                .iter()
                .map(|glyph| (glyph.start..glyph.end, glyph.x, glyph.w));
            let (left, _) = glyph_span(run.text, glyphs, menu.mark())?;
            Some((left, run.line_top, run.line_top + run.line_height))
        })
}

/// The misspelled word whose glyphs lie under `point`, relative to the text
/// origin, as its line and byte range: the visible row holding `point`, and
/// the first of the marks `marks` gives for that row's line whose glyphs on
/// that row span `point`'s x.
fn mark_under(
    editor: &Backend,
    point: Point,
    mut marks: impl FnMut(usize, &str) -> Vec<Range<usize>>,
) -> Option<(usize, Range<usize>)> {
    let run = editor
        .buffer()
        .layout_runs()
        .find(|run| point.y >= run.line_top && point.y < run.line_top + run.line_height)?;
    let glyphs = || {
        run.glyphs
            .iter()
            .map(|glyph| (glyph.start..glyph.end, glyph.x, glyph.w))
    };
    marks(run.line_i, run.text)
        .into_iter()
        .find(|mark| {
            glyph_span(run.text, glyphs(), mark.clone())
                .is_some_and(|(left, right)| point.x >= left && point.x < right)
        })
        .map(|mark| (run.line_i, mark))
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
}
