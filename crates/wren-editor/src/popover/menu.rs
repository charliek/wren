//! A popover as drawn: its size and place under its anchor (rule 4.4.3),
//! and the overlay that draws it and takes the pointer (rule 4.4.5).

use std::cell::Cell;
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;
use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::{self, LineHeight, Paragraph as _, Renderer as _, Text, Wrapping};
use iced::advanced::{mouse, overlay, Clipboard, Shell};
use iced::keyboard;
use iced::{
    alignment, Border, Color, Event, Font, Pixels, Point, Rectangle, Renderer, Size, Theme,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{publish, Now, OpenMenu, Popover, PopoverEvent};
#[cfg(any(test, feature = "test-support"))]
use crate::proof::SpellFrame;
#[cfg(test)]
use crate::proof::{SpellEvent, SpellMenu};
use crate::{MenuStyle, Message};

/// The space between the word's row and the menu.
const GAP: f32 = 4.0;
/// How far inside the editor the menu stays.
const MARGIN: f32 = 8.0;
const MIN_WIDTH: f32 = 160.0;
const MAX_WIDTH: f32 = 320.0;
const ROW_HEIGHT: f32 = 30.0;
/// Between the border and the rows.
const PADDING: f32 = 4.0;
const RADIUS: f32 = 8.0;
const BORDER: f32 = 1.0;
/// Between a row's edges and its text.
const TEXT_INSET: f32 = 8.0;
const ROW_RADIUS: f32 = 4.0;
/// How much of the selection's fill a tentative row gets.
const TENTATIVE_FILL: f32 = 0.35;
const ELLIPSIS: &str = "…";
/// The one inactive row of a menu with nothing to offer.
pub const NO_SUGGESTIONS: &str = "No suggestions";

/// The menu's width and the text of its rows, cut to fit it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fit {
    pub width: f32,
    pub labels: Vec<String>,
}

/// Size the menu for `items` (or the one inactive row when there are none)
/// in an editor `editor_width` wide, measuring text with `measure`: wide
/// enough for the longest row, between 160 and 320 px, and never more than
/// the editor's width less 16 px. Text that still does not fit ends in an
/// ellipsis.
#[cfg(test)]
pub(crate) fn fit(items: &[String], editor_width: f32, measure: impl Fn(&str) -> f32) -> Fit {
    fit_with(items, 0.0, editor_width, measure)
}

/// [`fit`], for rows that also hold `extra` pixels beside their labels: a
/// column of glyphs before them and the keys named after them ([`Extras`]).
pub(crate) fn fit_with(
    items: &[String],
    extra: f32,
    editor_width: f32,
    measure: impl Fn(&str) -> f32,
) -> Fit {
    let labels: Vec<&str> = if items.is_empty() {
        vec![NO_SUGGESTIONS]
    } else {
        items.iter().map(String::as_str).collect()
    };
    let inset = 2.0 * (PADDING + TEXT_INSET) + extra;
    let widest = labels
        .iter()
        .map(|label| measure(label))
        .fold(0.0, f32::max);
    let width = (widest + inset)
        .ceil()
        .clamp(MIN_WIDTH, MAX_WIDTH)
        .min((editor_width - 2.0 * MARGIN).floor())
        .max(0.0);
    let room = width - inset;
    Fit {
        width,
        labels: labels
            .into_iter()
            .map(|label| cut(label, room, &measure))
            .collect(),
    }
}

/// `label`, or its longest prefix of whole graphemes that fits `room` with
/// an ellipsis after it.
fn cut(label: &str, room: f32, measure: &impl Fn(&str) -> f32) -> String {
    if measure(label) <= room {
        return label.to_owned();
    }
    let ends: Vec<usize> = label
        .grapheme_indices(true)
        .map(|(at, grapheme)| at + grapheme.len())
        .collect();
    let with_ellipsis = |end: usize| format!("{}{ELLIPSIS}", label[..end].trim_end());
    // The longest prefix that fits, by bisection: widths grow with length.
    let (mut fits, mut over) = (0, ends.len());
    while over - fits > 1 {
        let mid = (fits + over) / 2;
        if measure(&with_ellipsis(ends[mid - 1])) <= room {
            fits = mid;
        } else {
            over = mid;
        }
    }
    with_ellipsis(if fits == 0 { 0 } else { ends[fits - 1] })
}

/// The width of the column of glyphs before a menu's labels, when a row has
/// one.
const GLYPH_COLUMN: f32 = 26.0;
/// The least space between a label and the key named after it.
const HINT_GAP: f32 = 16.0;
/// How much smaller than a label the key named after it is set.
const HINT_SHRINK: f32 = 1.5;

/// What a menu's rows hold beside their labels: a column of glyphs before
/// them, and the keys or notes named after them, right-aligned.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Extras {
    /// The glyph column's width, or nothing when no row has a glyph.
    pub glyphs: f32,
    /// The widest key's or note's width and the gap before it, or nothing
    /// when no row names one.
    pub hints: f32,
}

impl Extras {
    /// What `menu`'s rows hold, measuring a key's name with `measure_hint`.
    pub(crate) fn of(menu: &Popover, measure_hint: impl Fn(&str) -> f32) -> Self {
        let rows = menu.labels().len();
        let glyphs = if (0..rows).any(|row| menu.glyph(row).is_some()) {
            GLYPH_COLUMN
        } else {
            0.0
        };
        let widest = (0..rows)
            .filter_map(|row| menu.note(row))
            .map(|hint| measure_hint(&hint))
            .fold(None, |widest: Option<f32>, width| {
                Some(widest.map_or(width, |w| w.max(width)))
            });
        Self {
            glyphs,
            hints: widest.map_or(0.0, |width| (width + HINT_GAP).ceil()),
        }
    }

    pub(crate) fn width(self) -> f32 {
        self.glyphs + self.hints
    }
}

/// The text size a key named beside a row is set at, for rows at `size`.
pub(crate) fn hint_size(size: f32) -> f32 {
    (size - HINT_SHRINK).max(1.0)
}

/// The menu's height for `rows` rows.
pub(crate) fn height(rows: usize) -> f32 {
    2.0 * PADDING + ROW_HEIGHT * rows as f32
}

/// The height of the hint under the rows ([`Popover::footer`]).
const FOOTER_HEIGHT: f32 = 24.0;

/// The height of `menu` with `rows` rows: with its footer, if it has one.
pub(crate) fn height_of(menu: &Popover, rows: usize) -> f32 {
    height(rows) + menu.footer().map_or(0.0, |_| FOOTER_HEIGHT)
}

/// Where the menu of `size` goes for a word whose glyphs start at
/// `word_left` on the row from `row_top` to `row_bottom`, in an editor at
/// `editor` (rule 4.4.3): 4 px under the row, or 4 px over it when it fits
/// only there, or on the side with more room when it fits neither; then
/// moved to stay 8 px inside the editor. Edges are whole pixels.
pub(crate) fn place(
    editor: Rectangle,
    word_left: f32,
    row_top: f32,
    row_bottom: f32,
    size: Size,
) -> Rectangle {
    let top_limit = editor.y + MARGIN;
    let bottom_limit = editor.y + editor.height - MARGIN;
    let below = row_bottom + GAP;
    let above = row_top - GAP - size.height;
    let y = if below + size.height <= bottom_limit {
        below
    } else if above >= top_limit {
        above
    } else if bottom_limit - below >= row_top - GAP - top_limit {
        below
    } else {
        above
    };
    // The far edge first, so a menu larger than the editor keeps its
    // left and top edges inside it.
    let right_limit = editor.x + editor.width - MARGIN;
    let x = word_left
        .min(right_limit - size.width)
        .max(editor.x + MARGIN);
    let y = y.min(bottom_limit - size.height).max(top_limit);
    Rectangle::new(Point::new(x.round(), y.round()), size)
}

/// The bounds of row `row` of a menu at `bounds`.
fn row_bounds(bounds: Rectangle, row: usize) -> Rectangle {
    Rectangle {
        x: bounds.x + PADDING,
        y: bounds.y + PADDING + ROW_HEIGHT * row as f32,
        width: (bounds.width - 2.0 * PADDING).max(0.0),
        height: ROW_HEIGHT,
    }
}

/// The row of a menu at `bounds` with `rows` rows under `point`, if any.
fn row_at(bounds: Rectangle, rows: usize, point: Point) -> Option<usize> {
    (0..rows).find(|&row| row_bounds(bounds, row).contains(point))
}

/// The width of `content` set in `font` at `size`.
pub(crate) fn measure(content: &str, font: Font, size: f32) -> f32 {
    <Renderer as text::Renderer>::Paragraph::with_text(Text {
        content,
        bounds: Size::INFINITE,
        size: Pixels(size),
        line_height: LineHeight::default(),
        font,
        align_x: text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: Wrapping::None,
    })
    .min_width()
}

/// The open popover, drawn over the editor.
pub(crate) struct MenuOverlay<'a> {
    pub menu: &'a Popover,
    /// The rows' text, from [`fit_with`], which made room for what the
    /// rows hold beside them ([`Extras`]).
    pub labels: &'a [String],
    /// Where the menu is, in window coordinates.
    pub bounds: Rectangle,
    pub style: MenuStyle,
    /// The menu as the editor's events have left it; see [`OpenMenu`].
    pub now: &'a Cell<Option<OpenMenu>>,
    /// Where the menu's bounds are recorded for the host's tests.
    #[cfg(any(test, feature = "test-support"))]
    pub frame: Option<&'a RefCell<SpellFrame>>,
}

impl MenuOverlay<'_> {
    /// The row under `point` that can be selected, if any: not one that
    /// cannot be chosen.
    fn active_row_at(&self, point: Point) -> Option<usize> {
        row_at(self.bounds, self.menu.labels().len(), point)
            .filter(|&row| self.menu.is_enabled(row))
    }

    /// Handle `event` with the pointer at `cursor` (rule 4.4.5), unless an
    /// earlier event has closed the menu. Keys are the editor widget's.
    pub fn on_event(&self, event: &Event, cursor: mouse::Cursor, shell: &mut Shell<'_, Message>) {
        let Some(open) = self.now.get() else {
            return;
        };
        let mut tell = |event| publish(shell, self.now, Message::Spell(event));
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(point) = cursor.position() else {
                    return;
                };
                if let Some(row) = self.active_row_at(point) {
                    if open.selected != Some(row) {
                        tell(PopoverEvent::Hover(row));
                    }
                }
                if self.bounds.contains(point) {
                    shell.capture_event();
                }
            }
            // A press off the menu is the editor widget's, which closes the
            // menu for it in its order with the batch's keys: the overlay
            // sees the whole batch first, so a key before the press that
            // closed the menu would not have closed it yet here.
            Event::Mouse(mouse::Event::ButtonPressed(button)) => match cursor.position() {
                Some(point) if self.bounds.contains(point) => {
                    if *button == mouse::Button::Left {
                        if let Some(row) = self.active_row_at(point) {
                            // A row of any menu but the suggestions menu
                            // is chosen from that menu or from none.
                            tell(if self.menu.named_by_choice() {
                                PopoverEvent::AcceptOf {
                                    menu: self.menu.instance(),
                                    row: Some(row),
                                }
                            } else {
                                PopoverEvent::Accept(row)
                            });
                        } else if self.menu.labels().is_empty()
                            && row_at(self.bounds, 1, point).is_some()
                        {
                            tell(PopoverEvent::Close);
                        }
                    }
                    shell.capture_event();
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// What the pointer over `point` shows: a hand over a suggestion, and
    /// over the rest of the menu anything but nothing, so the editor under
    /// it does not show its own. A menu an earlier event closed shows
    /// nothing.
    fn interaction(&self, point: Option<Point>) -> mouse::Interaction {
        match point.filter(|_| self.now.get().is_some()) {
            Some(point) if self.active_row_at(point).is_some() => mouse::Interaction::Pointer,
            Some(point) if self.bounds.contains(point) => mouse::Interaction::Idle,
            _ => mouse::Interaction::None,
        }
    }
}

impl overlay::Overlay<Message, Theme, Renderer> for MenuOverlay<'_> {
    fn layout(&mut self, _renderer: &Renderer, _bounds: Size) -> layout::Node {
        layout::Node::new(self.bounds.size()).move_to(self.bounds.position())
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
    ) {
        let style = self.style;
        renderer.fill_quad(
            renderer::Quad {
                bounds: self.bounds,
                border: Border {
                    color: style.border,
                    width: BORDER,
                    radius: RADIUS.into(),
                },
                ..renderer::Quad::default()
            },
            style.background,
        );
        if let Some(selected) = self.now.get().and_then(|open| open.selected) {
            // A row Enter does not choose is marked, not filled: a lighter
            // fill inside an outline.
            let tentative = self.menu.is_tentative(selected);
            renderer.fill_quad(
                renderer::Quad {
                    bounds: row_bounds(self.bounds, selected),
                    border: Border {
                        color: style.selected,
                        width: if tentative { BORDER } else { 0.0 },
                        radius: ROW_RADIUS.into(),
                    },
                    ..renderer::Quad::default()
                },
                if tentative {
                    Color {
                        a: style.selected.a * TENTATIVE_FILL,
                        ..style.selected
                    }
                } else {
                    style.selected
                },
            );
        }
        let color = if self.menu.labels().is_empty() {
            style.muted
        } else {
            style.text
        };
        let hints = hint_size(style.text_size);
        let extras = Extras::of(self.menu, |hint| measure(hint, style.font, hints));
        let text = |content: String, size: f32, font: Font, width: f32, align_x| Text {
            content,
            bounds: Size::new(width.max(0.0), ROW_HEIGHT),
            size: Pixels(size),
            line_height: LineHeight::default(),
            font,
            align_x,
            align_y: alignment::Vertical::Center,
            shaping: text::Shaping::Advanced,
            wrapping: Wrapping::None,
        };
        for (row, label) in self.labels.iter().enumerate() {
            let bounds = row_bounds(self.bounds, row);
            // A row that cannot be chosen is muted whole; a checked one
            // has its label in the accent, and heavier.
            let enabled = self.menu.labels().is_empty() || self.menu.is_enabled(row);
            let checked = enabled && self.menu.is_checked(row);
            let (color, font) = match (enabled, checked) {
                (false, _) => (style.muted, style.font),
                (true, true) => (
                    style.accent,
                    Font {
                        weight: iced::font::Weight::Semibold,
                        ..style.font
                    },
                ),
                (true, false) => (color, style.font),
            };
            if let Some(glyph) = self.menu.glyph(row) {
                renderer.fill_text(
                    text(
                        glyph.to_owned(),
                        style.text_size,
                        style.font,
                        extras.glyphs,
                        text::Alignment::Center,
                    ),
                    Point::new(
                        bounds.x + TEXT_INSET + extras.glyphs / 2.0,
                        bounds.center_y(),
                    ),
                    if enabled { style.text } else { style.muted },
                    self.bounds,
                );
            }
            renderer.fill_text(
                text(
                    label.clone(),
                    style.text_size,
                    font,
                    bounds.width - 2.0 * TEXT_INSET - extras.width(),
                    text::Alignment::Default,
                ),
                Point::new(bounds.x + TEXT_INSET + extras.glyphs, bounds.center_y()),
                color,
                self.bounds,
            );
            if let Some(hint) = self.menu.note(row) {
                renderer.fill_text(
                    text(
                        hint,
                        hints,
                        style.font,
                        extras.hints,
                        text::Alignment::Right,
                    ),
                    Point::new(bounds.x + bounds.width - TEXT_INSET, bounds.center_y()),
                    style.muted,
                    self.bounds,
                );
            }
        }
        if let Some(footer) = self.menu.footer() {
            let top = self.bounds.y + self.bounds.height - FOOTER_HEIGHT;
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: self.bounds.x + PADDING,
                        y: top,
                        width: (self.bounds.width - 2.0 * PADDING).max(0.0),
                        height: BORDER,
                    },
                    ..renderer::Quad::default()
                },
                style.border,
            );
            renderer.fill_text(
                Text {
                    bounds: Size::new(
                        (self.bounds.width - 2.0 * (PADDING + TEXT_INSET)).max(0.0),
                        FOOTER_HEIGHT,
                    ),
                    ..text(
                        footer.to_owned(),
                        hints,
                        style.font,
                        0.0,
                        text::Alignment::Default,
                    )
                },
                Point::new(
                    self.bounds.x + PADDING + TEXT_INSET,
                    top + FOOTER_HEIGHT / 2.0,
                ),
                style.muted,
                self.bounds,
            );
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(frame) = self.frame {
            frame.borrow_mut().menu = Some(self.bounds);
        }
    }

    fn update(
        &mut self,
        event: &Event,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.on_event(event, cursor, shell);
    }

    fn mouse_interaction(
        &self,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        self.interaction(cursor.position())
    }
}

/// Whether the widget, routing an event with the pointer where it finds
/// it, will close the popover or change its rows ([`InOrder`]).
type Ends<'a> = dyn FnMut(&Event, mouse::Cursor) -> bool + 'a;

/// The open popover's overlay as a window hands it events: all of a batch,
/// before the editor widget sees any of them.
///
/// A press on a row would so be taken ahead of a key pressed before it,
/// which the widget only routes afterwards: Enter and then a press on
/// another row would choose the row pressed. So once an event of the batch
/// ends or changes what is open, as `ends` says the widget will find, the
/// overlay takes no more of the pointer, and the presses after that event
/// reach the widget, after it. Up and Down leave the rows as they are
/// drawn, and a press on a row after them still chooses that row.
///
/// A row chosen with the pointer comes before every event the widget
/// routes, and the widget routes them against what the choice leaves
/// ([`Now::chosen`]): after a row that opens a menu of the host's, the
/// batch's keys and input-method text are no one's, and so is what a
/// composition under way commits, in this batch or a later one.
///
/// Only a batch's first press may choose a row, and only before any key of
/// the batch: once a press of any button or a key (not a modifier alone)
/// has been seen, the overlay chooses no row, and the keys keep their
/// routing against the popover as it was. A press it does not take on the
/// popover as drawn is dropped there: captured, with its release, so it
/// never reaches the text, a task's box or a selection under it. One off
/// the popover is the widget's, which dismisses it or is the text's. A row
/// pressed after a press that chose nothing (a middle press, or a right
/// press no provider takes) is so not chosen either; the user presses it
/// again.
///
/// Two limits are the toolkit's, as they were for the suggestions menu
/// before there were other popovers. The window hands the overlay one
/// place of the pointer for the whole batch, where it is at the end, so a
/// batch's first press is taken as made there: a press in the text, then
/// a move onto a row before the batch ends, chooses that row. And the
/// overlay sees the whole batch before the widget: a move of the pointer
/// over a row selects it ahead of an Up or Down pressed before it, so
/// Enter after both chooses from where the key left the selection.
pub(crate) struct InOrder<'a> {
    overlay: MenuOverlay<'a>,
    /// What is open as the widget's events leave it, of which the
    /// overlay's `now` is the copy.
    now: &'a Now,
    /// Whether the widget, routing an event with the pointer where it
    /// finds it, will close the popover or change its rows. It is asked
    /// about every event the overlay does not take, in order, since what
    /// an event does depends on those before it.
    ends: Box<Ends<'a>>,
    /// An event of this batch has ended or changed what is open.
    spent: bool,
    /// A press of this batch has been seen.
    pressed: bool,
    /// A key of this batch, other than a modifier alone, has been seen.
    keyed: bool,
    /// A press dropped on the popover, whose release is dropped too.
    dropped: bool,
    /// Set when a row chosen with the pointer hands the host the keyboard,
    /// in the widget's state, which outlasts the batch.
    handed: Rc<Cell<bool>>,
}

impl<'a> InOrder<'a> {
    pub(crate) fn new(
        overlay: MenuOverlay<'a>,
        now: &'a Now,
        handed: Rc<Cell<bool>>,
        ends: impl FnMut(&Event, mouse::Cursor) -> bool + 'a,
    ) -> Self {
        Self {
            overlay,
            now,
            ends: Box::new(ends),
            spent: false,
            pressed: false,
            keyed: false,
            dropped: false,
            handed,
        }
    }

    /// Handle `event` with the pointer at `cursor`, in its order with the
    /// batch's other events.
    pub(crate) fn on_event(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) {
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) if !matches!(key, keyboard::Key::Named(named) if crate::keys::is_modifier(*named)) =>
            {
                self.keyed = true;
            }
            Event::Mouse(mouse::Event::ButtonPressed(_)) => {
                let refused = self.spent || self.pressed || self.keyed;
                self.pressed = true;
                if refused {
                    if (cursor.position()).is_some_and(|point| self.overlay.bounds.contains(point))
                    {
                        shell.capture_event();
                        self.dropped = true;
                    }
                    self.spent = true;
                    return;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(_)) if self.dropped => {
                shell.capture_event();
                return;
            }
            _ => {}
        }
        if self.spent {
            return;
        }
        // A row a press chooses, and what choosing it hands the host,
        // known before the choice closes the popover.
        let chosen = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => cursor
                .position()
                .filter(|_| self.overlay.now.get().is_some())
                .and_then(|point| self.overlay.active_row_at(point)),
            _ => None,
        };
        let handover = chosen.and_then(|row| self.now.handover_for(self.overlay.menu, row));
        self.overlay.on_event(event, cursor, shell);
        if chosen.is_some() {
            self.now.chosen(handover);
            if handover.is_some() {
                self.handed.set(true);
            }
            self.spent = true;
        } else if shell.event_status() != iced::event::Status::Captured {
            // The widget finds no pointer where the overlay holds it.
            let below = match cursor.position() {
                Some(point) if self.overlay.bounds.contains(point) => mouse::Cursor::Unavailable,
                _ => cursor,
            };
            self.spent = (self.ends)(event, below);
        }
    }

    /// What the pointer over `point` shows; nothing once the overlay takes
    /// no more of it, so that the widget below is handed the pointer.
    pub(crate) fn interaction(&self, point: Option<Point>) -> mouse::Interaction {
        if self.spent {
            mouse::Interaction::None
        } else {
            self.overlay.interaction(point)
        }
    }
}

impl overlay::Overlay<Message, Theme, Renderer> for InOrder<'_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.overlay.layout(renderer, bounds)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.overlay.draw(renderer, theme, style, layout, cursor);
    }

    fn update(
        &mut self,
        event: &Event,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.on_event(event, cursor, shell);
    }

    fn mouse_interaction(
        &self,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        self.interaction(cursor.position())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    /// Ten pixels a character.
    fn tens(text: &str) -> f32 {
        10.0 * text.chars().count() as f32
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    #[test]
    fn the_width_fits_the_longest_row_between_160_and_320_px() {
        // Rows are inset 12 px a side.
        let fitted = fit(&strings(&["the", "tech"]), 1000.0, tens);
        assert_eq!(fitted.width, 160.0);
        assert_eq!(fitted.labels, ["the", "tech"]);
        let fitted = fit(&strings(&["a", &"m".repeat(20)]), 1000.0, tens);
        assert_eq!(fitted.width, 224.0);
        let fitted = fit(&strings(&[&"m".repeat(40)]), 1000.0, tens);
        assert_eq!(fitted.width, 320.0);
        // A fractional width rounds up to a whole pixel.
        let fitted = fit(&strings(&["x"]), 1000.0, |_| 200.2);
        assert_eq!(fitted.width, 225.0);
        // With nothing to offer, the one inactive row.
        let fitted = fit(&[], 1000.0, tens);
        assert_eq!(fitted.labels, [NO_SUGGESTIONS]);
        assert_eq!(fitted.width, 164.0);
    }

    #[test]
    fn a_narrow_editor_narrows_the_menu_and_cuts_its_text() {
        let long = "m".repeat(40);
        let fitted = fit(&strings(&["short", &long]), 150.0, tens);
        assert_eq!(fitted.width, 134.0);
        // 110 px of room: ten characters and the ellipsis.
        assert_eq!(
            fitted.labels,
            ["short".to_owned(), format!("{}…", "m".repeat(10))]
        );
        let fitted = fit(&strings(&[&long]), 1000.0, tens);
        // 296 px of room: 28 characters and the ellipsis.
        assert_eq!(fitted.labels, [format!("{}…", "m".repeat(28))]);
        // Never wider than the editor allows, nor narrower than nothing.
        assert_eq!(fit(&strings(&["a"]), 10.0, tens).width, 0.0);
        assert_eq!(fit(&strings(&["ab"]), 10.0, tens).labels, ["…"]);
    }

    #[test]
    fn text_is_cut_between_graphemes_and_before_spaces() {
        let room = 50.0;
        assert_eq!(cut("abcd", room, &tens), "abcd");
        assert_eq!(cut("abcdef", room, &tens), "abcd…");
        // A space before the cut is dropped.
        assert_eq!(cut("abc def", room, &tens), "abc…");
        // `é` written with a combining mark stays whole: it counts two
        // characters here, so only `ab` and it fit.
        assert_eq!(cut("abe\u{301}de", room, &tens), "abe\u{301}…");
        assert_eq!(cut("abcdef", 5.0, &tens), "…");
    }

    #[test]
    fn the_menu_is_padded_rows_of_30_px() {
        assert_eq!(height(1), 38.0);
        assert_eq!(height(5), 158.0);
        let bounds = Rectangle::new(Point::new(100.0, 50.0), Size::new(200.0, height(3)));
        assert_eq!(
            row_bounds(bounds, 1),
            Rectangle::new(Point::new(104.0, 84.0), Size::new(192.0, 30.0))
        );
        assert_eq!(row_at(bounds, 3, Point::new(150.0, 54.0)), Some(0));
        assert_eq!(row_at(bounds, 3, Point::new(150.0, 113.9)), Some(1));
        assert_eq!(row_at(bounds, 3, Point::new(150.0, 115.0)), Some(2));
        assert_eq!(row_at(bounds, 3, Point::new(150.0, 52.0)), None, "padding");
        assert_eq!(row_at(bounds, 3, Point::new(102.0, 60.0)), None, "padding");
        assert_eq!(row_at(bounds, 0, Point::new(150.0, 60.0)), None);
    }

    const EDITOR: Rectangle = Rectangle {
        x: 200.0,
        y: 40.0,
        width: 800.0,
        height: 600.0,
    };

    #[test]
    fn the_menu_goes_4_px_under_the_row_at_the_words_left_edge() {
        let placed = place(EDITOR, 300.4, 100.0, 124.0, Size::new(160.0, 158.0));
        assert_eq!(
            placed,
            Rectangle::new(Point::new(300.0, 128.0), Size::new(160.0, 158.0))
        );
    }

    #[test]
    fn the_menu_goes_above_when_it_fits_only_there() {
        // Under the row it would reach 640 - 8 = 632 px and past.
        let size = Size::new(160.0, 158.0);
        let placed = place(EDITOR, 300.0, 500.0, 524.0, size);
        assert_eq!(placed.y, 500.0 - 4.0 - 158.0);
        // Just fits under: it stays under.
        let placed = place(EDITOR, 300.0, 446.0, 470.0, size);
        assert_eq!(placed.y, 474.0);
        assert_eq!(placed.y + placed.height, 632.0);
    }

    #[test]
    fn fitting_neither_way_it_takes_the_side_with_more_room_then_stays_inside() {
        let short = Rectangle {
            height: 200.0,
            ..EDITOR
        };
        let size = Size::new(160.0, 158.0);
        // More room below (row near the top): under it, then moved up.
        let placed = place(short, 300.0, 60.0, 84.0, size);
        assert_eq!(placed.y, 40.0 + 200.0 - 8.0 - 158.0);
        // More room above: over it, then moved down.
        let placed = place(short, 300.0, 140.0, 164.0, size);
        assert_eq!(placed.y, 48.0);
        // Taller than the editor: its top stays 8 px inside.
        let placed = place(short, 300.0, 140.0, 164.0, Size::new(160.0, 400.0));
        assert_eq!(placed.y, 48.0);
    }

    #[test]
    fn the_menu_stays_8_px_inside_the_editors_sides() {
        let size = Size::new(200.0, 68.0);
        // A word at the right edge.
        let placed = place(EDITOR, 950.0, 100.0, 124.0, size);
        assert_eq!(placed.x, 1000.0 - 8.0 - 200.0);
        // A word left of the editor's inset, scrolled or hanging.
        let placed = place(EDITOR, 203.0, 100.0, 124.0, size);
        assert_eq!(placed.x, 208.0);
        // Wider than the editor: its left edge stays inside.
        let narrow = Rectangle {
            width: 150.0,
            ..EDITOR
        };
        let placed = place(narrow, 260.0, 100.0, 124.0, size);
        assert_eq!(placed.x, 208.0);
    }

    /// The rows' text for `menu`, uncut.
    fn labels_of(menu: &SpellMenu) -> Vec<String> {
        if menu.items().is_empty() {
            vec![NO_SUGGESTIONS.into()]
        } else {
            menu.items().to_vec()
        }
    }

    /// `menu` drawn 200 px wide at (300, 128) in [`EDITOR`], with `now` as
    /// the editor's copy of it.
    fn overlay<'a>(
        menu: &'a SpellMenu,
        labels: &'a [String],
        now: &'a Cell<Option<OpenMenu>>,
    ) -> MenuOverlay<'a> {
        MenuOverlay {
            menu,
            labels,
            bounds: Rectangle::new(
                Point::new(300.0, 128.0),
                Size::new(200.0, height(labels.len())),
            ),
            style: fixtures::STYLE.menu,
            now,
            frame: None,
        }
    }

    /// What the overlay for `menu` publishes for `event` with the pointer at
    /// `point`, and whether it captured it.
    fn send(menu: &SpellMenu, event: Event, point: Point) -> (Vec<Message>, bool) {
        let labels = labels_of(menu);
        let now = Cell::new(Some(OpenMenu::of(menu)));
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        overlay(menu, &labels, &now).on_event(&event, mouse::Cursor::Available(point), &mut shell);
        let captured = shell.event_status() == iced::event::Status::Captured;
        (messages, captured)
    }

    fn menu(items: &[&str], selected: usize) -> SpellMenu {
        let mut menu = SpellMenu::new(0, 0..3, "teh".into(), strings(items));
        menu.hover(selected);
        menu
    }

    fn moved() -> Event {
        Event::Mouse(mouse::Event::CursorMoved {
            position: Point::ORIGIN,
        })
    }

    fn pressed(button: mouse::Button) -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(button))
    }

    /// The middle of row `row` of the test menu.
    fn on_row(row: usize) -> Point {
        Point::new(400.0, 128.0 + PADDING + ROW_HEIGHT * row as f32 + 15.0)
    }

    #[test]
    fn moving_over_a_row_selects_it() {
        let three = menu(&["the", "ten", "tea"], 0);
        let (messages, captured) = send(&three, moved(), on_row(2));
        assert!(matches!(
            messages.as_slice(),
            [Message::Spell(SpellEvent::Hover(2))]
        ));
        assert!(captured);
        // Over the row already selected, or the padding, nothing new.
        assert!(send(&three, moved(), on_row(0)).0.is_empty());
        let (messages, captured) = send(&three, moved(), Point::new(400.0, 129.0));
        assert!(messages.is_empty() && captured);
        // Off the menu, nothing; the editor below sees the move.
        let (messages, captured) = send(&three, moved(), Point::new(250.0, 300.0));
        assert!(messages.is_empty() && !captured);
        // The inactive row never selects.
        let empty = menu(&[], 0);
        assert!(send(&empty, moved(), on_row(0)).0.is_empty());
    }

    #[test]
    fn a_click_on_a_row_accepts_it() {
        let three = menu(&["the", "ten", "tea"], 0);
        let (messages, captured) = send(&three, pressed(mouse::Button::Left), on_row(1));
        assert!(matches!(
            messages.as_slice(),
            [Message::Spell(SpellEvent::Accept(1))]
        ));
        assert!(captured);
        // Other buttons, and the padding, do nothing but keep the press.
        for (button, point) in [
            (mouse::Button::Right, on_row(1)),
            (mouse::Button::Middle, on_row(1)),
            (mouse::Button::Left, Point::new(400.0, 129.0)),
        ] {
            let (messages, captured) = send(&three, pressed(button), point);
            assert!(messages.is_empty() && captured, "{button:?} {point:?}");
        }
        // The inactive row closes the menu, as Enter does.
        let (messages, captured) = send(&menu(&[], 0), pressed(mouse::Button::Left), on_row(0));
        assert!(matches!(
            messages.as_slice(),
            [Message::Spell(SpellEvent::Close)]
        ));
        assert!(captured);
    }

    #[test]
    fn a_press_off_the_menu_is_left_to_the_editor() {
        let three = menu(&["the", "ten", "tea"], 0);
        for button in [mouse::Button::Left, mouse::Button::Right] {
            // Inside the editor and outside it, the editor widget closes the
            // menu in the press's order with the batch's keys.
            for point in [Point::new(250.0, 300.0), Point::new(100.0, 300.0)] {
                let (messages, captured) = send(&three, pressed(button), point);
                assert!(messages.is_empty() && !captured, "{button:?} {point:?}");
            }
        }
    }

    #[test]
    fn the_pointer_shows_a_hand_only_over_a_suggestion() {
        let three = menu(&["the", "ten", "tea"], 0);
        let labels = labels_of(&three);
        let now = Cell::new(Some(OpenMenu::of(&three)));
        let overlay = overlay(&three, &labels, &now);
        assert_eq!(
            overlay.interaction(Some(on_row(1))),
            mouse::Interaction::Pointer
        );
        assert_eq!(
            overlay.interaction(Some(Point::new(400.0, 129.0))),
            mouse::Interaction::Idle
        );
        assert_eq!(
            overlay.interaction(Some(Point::new(250.0, 300.0))),
            mouse::Interaction::None
        );
        assert_eq!(overlay.interaction(None), mouse::Interaction::None);
        now.set(None);
        assert_eq!(
            overlay.interaction(Some(on_row(1))),
            mouse::Interaction::None
        );
    }

    #[test]
    fn measured_text_grows_with_its_length() {
        fixtures::load_font();
        let style = fixtures::STYLE.menu;
        let one = measure("m", style.font, style.text_size);
        let ten = measure(&"m".repeat(10), style.font, style.text_size);
        assert!(one > 5.0 && one < 10.0, "{one}");
        assert!((ten - 10.0 * one).abs() < 0.5, "{one} {ten}");
    }

    // ── In order with the batch's other events ─────────────────────────────

    /// What the overlay for a menu of three publishes for `events`, each
    /// with the pointer on row `row`, as a window hands it a batch in
    /// which the widget would find the events for which `ends` is true to
    /// end the popover; and the pointer it shows over that row afterwards.
    fn in_order(
        events: Vec<Event>,
        mut ends: impl FnMut(&Event) -> bool,
        row: usize,
    ) -> (Vec<Message>, mouse::Interaction) {
        let three = menu(&["the", "ten", "tea"], 0);
        let labels = labels_of(&three);
        let now = Now::new();
        now.start(Some(&three));
        let mut overlay = InOrder::new(
            overlay(&three, &labels, now.cell()),
            &now,
            Rc::default(),
            |event, _| ends(event),
        );
        let mut messages = Vec::new();
        for event in events {
            let mut shell = Shell::new(&mut messages);
            overlay.on_event(&event, mouse::Cursor::Available(on_row(row)), &mut shell);
        }
        (messages, overlay.interaction(Some(on_row(row))))
    }

    /// Stands for any event the overlay takes nothing of.
    fn other() -> Event {
        Event::Mouse(mouse::Event::CursorLeft)
    }

    #[test]
    fn the_overlay_takes_no_press_after_an_event_that_ends_the_popover() {
        let left = || pressed(mouse::Button::Left);
        // Alone, or after an event that leaves the popover as it is, a
        // press on a row accepts it.
        for events in [vec![left()], vec![other(), other(), left()]] {
            let (messages, interaction) = in_order(events, |_| false, 2);
            assert!(matches!(
                messages.as_slice(),
                [Message::Spell(SpellEvent::Accept(2))]
            ));
            // The press closed the popover, so it shows no pointer.
            assert_eq!(interaction, mouse::Interaction::None);
        }
        // After one that ends it, the press and the pointer are the
        // editor's: the widget, which sees that event first, has closed or
        // accepted by then.
        let (messages, interaction) = in_order(vec![other(), moved(), left()], |_| true, 2);
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(interaction, mouse::Interaction::None);
        // The widget is asked about each event the overlay leaves, in
        // order, and about none after the one that ends the popover.
        let mut asked = 0;
        let ends = |_: &Event| {
            asked += 1;
            asked == 2
        };
        let (messages, _) = in_order(vec![other(), other(), other(), left()], ends, 2);
        assert!(messages.is_empty());
        assert_eq!(asked, 2);
        // An event the overlay takes itself ends nothing: the pointer over
        // a row, which only selects it.
        let mut asked = 0;
        let (messages, interaction) = in_order(
            vec![moved(), left()],
            |_| {
                asked += 1;
                true
            },
            2,
        );
        assert!(matches!(
            messages.as_slice(),
            [
                Message::Spell(SpellEvent::Hover(2)),
                Message::Spell(SpellEvent::Accept(2))
            ]
        ));
        assert_eq!((asked, interaction), (0, mouse::Interaction::None));
        // Until then it shows its own pointer.
        assert_eq!(
            in_order(Vec::new(), |_| false, 2).1,
            mouse::Interaction::Pointer
        );
    }
}
