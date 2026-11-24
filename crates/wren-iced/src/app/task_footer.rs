//! The caret's task in a footer (discovery/tasks.md §2 item 23): the pieces
//! of `wren_markdown::task::summary` that fit in the room a footer's fixed
//! parts leave, each whole with the separator before it.

use std::cell::RefCell;
use std::collections::HashMap;

use iced::advanced::graphics::text::Paragraph;
use iced::advanced::text::{self, Paragraph as _, Text};
use iced::alignment;
use iced::widget::text::{LineHeight, Shaping, Wrapping};
use iced::widget::{container, row};
use iced::{Element, Fill, Font, Pixels, Size};
use wren_markdown::task::{self, Look, Piece, Summary};

use super::{App, Message};
use crate::{chrome, fonts};

/// The width `content` takes as a single-line `text` widget of `size` in
/// `font`, as it lays itself out.
pub(super) fn label_width(content: &str, size: f32, font: Font) -> f32 {
    Paragraph::with_text(Text {
        content,
        bounds: Size::INFINITE,
        size: Pixels(size),
        line_height: LineHeight::default(),
        font,
        align_x: text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::default(),
        wrapping: Wrapping::None,
    })
    .min_width()
}

/// The widths of the labels the footers measure, by text, font and size,
/// so a layout with nothing new to show measures nothing. Fonts and sizes
/// are part of the key; the bound keeps a long session's worth of caret
/// lines from piling up.
#[derive(Debug, Default)]
pub(super) struct Widths {
    known: RefCell<HashMap<(String, Font, u32), f32>>,
    /// How many labels were measured.
    #[cfg(test)]
    measured: std::cell::Cell<usize>,
}

impl Widths {
    /// How many widths are kept before they are all dropped.
    const KEPT: usize = 256;

    /// [`label_width`], measured once.
    pub(super) fn of(&self, content: &str, size: f32, font: Font) -> f32 {
        let key = (content.to_owned(), font, size.to_bits());
        let mut known = self.known.borrow_mut();
        if let Some(width) = known.get(&key) {
            return *width;
        }
        if known.len() >= Self::KEPT {
            known.clear();
        }
        #[cfg(test)]
        self.measured.set(self.measured.get() + 1);
        *known
            .entry(key)
            .or_insert_with(|| label_width(content, size, font))
    }
}

/// How many pieces, from the first, fit in `room` together, given each
/// piece's width with the separator before it: 0 when not even the first
/// does. A width after the first that does not fit is never asked for.
pub(super) fn pieces_that_fit(widths: impl IntoIterator<Item = f32>, room: f32) -> usize {
    let mut used = 0.0;
    widths
        .into_iter()
        .take_while(|width| {
            used += *width;
            used <= room
        })
        .count()
}

/// The font a piece is drawn in: the status leads in semibold.
fn font(piece: &Piece) -> Font {
    match piece.look {
        Look::Status => fonts::SEMIBOLD,
        _ => fonts::UI,
    }
}

impl App {
    /// The pieces of `summary` that fit in `room`, drawn at `size` in a row
    /// that fills what it is given, or `None` when not even the status
    /// fits: a piece is never cut short.
    pub(super) fn task_line(
        &self,
        summary: &Summary,
        room: f32,
        size: f32,
    ) -> Option<Element<'_, Message>> {
        let widths = (summary.pieces.iter().enumerate()).map(|(index, piece)| {
            let gap = if index > 0 {
                self.label_widths.of(task::SEPARATOR, size, fonts::UI)
            } else {
                0.0
            };
            gap + self.label_widths.of(&piece.text, size, font(piece))
        });
        let shown = pieces_that_fit(widths, room);
        if shown == 0 {
            return None;
        }
        let colors = self.definition.colors;
        let size = size as u32;
        let mut pieces = row![].align_y(iced::Center);
        for (index, piece) in summary.pieces.iter().take(shown).enumerate() {
            let color = match piece.look {
                Look::Status => colors.text,
                Look::Plain => colors.body,
                Look::Owners => colors.owner,
                Look::Overdue | Look::Invalid => colors.danger,
            };
            if index > 0 {
                pieces = pieces.push(chrome::label(task::SEPARATOR, size, colors.muted));
            }
            pieces = pieces.push(chrome::label(piece.text.clone(), size, color).font(font(piece)));
        }
        Some(container(pieces).width(Fill).clip(true).into())
    }
}

#[cfg(test)]
mod tests {
    use wren_markdown::block::{Boxes, Scanner};
    use wren_markdown::date::Date;

    use super::*;

    #[test]
    fn pieces_fit_whole_from_the_first() {
        let widths = [40.0, 150.0, 60.0, 80.0];
        for (room, shown) in [
            (0.0, 0),
            (39.9, 0),
            (40.0, 1),
            (189.9, 1),
            (190.0, 2),
            (249.0, 2),
            (250.0, 3),
            (329.0, 3),
            (330.0, 4),
            (1000.0, 4),
            (-16.0, 0),
        ] {
            assert_eq!(pieces_that_fit(widths, room), shown, "{room}");
        }
        assert_eq!(pieces_that_fit([], 100.0), 0);
        // Widths are asked for up to the first that does not fit.
        let asked = std::cell::Cell::new(0);
        let lazy = widths.iter().map(|width| {
            asked.set(asked.get() + 1);
            *width
        });
        assert_eq!(pieces_that_fit(lazy, 200.0), 2);
        assert_eq!(asked.get(), 3);
    }

    #[test]
    fn a_label_is_measured_once_per_text_font_and_size() {
        crate::fonts::load();
        let widths = Widths::default();
        let once = widths.of("Doing", 11.0, fonts::SEMIBOLD);
        assert_eq!(once, label_width("Doing", 11.0, fonts::SEMIBOLD));
        for _ in 0..3 {
            assert_eq!(widths.of("Doing", 11.0, fonts::SEMIBOLD), once);
        }
        assert_eq!(widths.measured.get(), 1);
        widths.of("Doing", 11.0, fonts::UI);
        widths.of("Doing", 12.0, fonts::SEMIBOLD);
        widths.of("Doing ", 11.0, fonts::SEMIBOLD);
        assert_eq!(widths.measured.get(), 4);
        for index in 0..Widths::KEPT * 2 {
            widths.of(&index.to_string(), 11.0, fonts::UI);
        }
        assert!(widths.known.borrow().len() <= Widths::KEPT);
    }

    /// The design's example, measured as the footer draws it: each room
    /// shows exactly the pieces whose measured widths add up within it.
    #[test]
    fn the_measured_pieces_that_fit_are_the_ones_shown() {
        crate::fonts::load();
        let line = "- [/] Draft the cutover runbook @alice.chen ⏫ 📅 2026-11-23";
        let task = Scanner::new(Boxes::Status)
            .next_line(line)
            .item
            .and_then(|item| item.task)
            .unwrap();
        let read = task::parse(line, task).unwrap();
        let summary = task::summary(&read, Date::new(2026, 11, 19).unwrap());
        let separator = label_width(task::SEPARATOR, 11.0, fonts::UI);
        assert!(separator > 0.0);
        let widths: Vec<f32> = (summary.pieces.iter().enumerate())
            .map(|(index, piece)| {
                let gap = if index > 0 { separator } else { 0.0 };
                gap + label_width(&piece.text, 11.0, font(piece))
            })
            .collect();
        assert!(widths.iter().all(|width| *width > 0.0), "{widths:?}");
        // A semibold status is wider than the same word in the regular face.
        assert!(widths[0] > label_width("Doing", 11.0, fonts::UI));
        let mut total = 0.0;
        for (count, width) in widths.iter().enumerate() {
            assert_eq!(pieces_that_fit(widths.clone(), total + width - 0.5), count);
            total += width;
            assert_eq!(pieces_that_fit(widths.clone(), total), count + 1);
        }
        // A longer text is measured wider.
        assert!(
            label_width("due Mon 23 Nov, in 4 days", 11.0, fonts::UI)
                > label_width("due Mon 23 Nov", 11.0, fonts::UI)
        );
    }
}
