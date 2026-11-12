//! The board canvas: an iced widget that paints a [`wren_board::scene`]
//! and turns pointer and key input into board edits.
//!
//! Like `wren-editor`, it knows no Wren types and no app messages: the host
//! passes in a [`wren_board::Palette`], a style, the label font and a key
//! policy, and owns the view state between frames.
//!
//! Today the crate holds the Phase 1 spike ([`spike`]): a bare pan-and-zoom
//! widget over generated scenes, built to measure how a scene is best
//! painted and to try the label editor's mechanism.

#[cfg(test)]
mod fixtures;
pub mod layers;
mod measure;
mod paint;
pub mod spike;
mod widget;

pub use measure::Measurer;

/// Where a board sits in the widget: the board point at the widget's
/// top-left corner, and how many screen pixels a board unit takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Viewport {
    pub const MIN_ZOOM: f32 = 0.1;
    pub const MAX_ZOOM: f32 = 4.0;

    /// A widget-local point in board units.
    pub fn to_board(self, screen: (f32, f32)) -> (f32, f32) {
        (self.x + screen.0 / self.zoom, self.y + screen.1 / self.zoom)
    }

    /// A board point in widget-local pixels.
    pub fn to_screen(self, board: (f32, f32)) -> (f32, f32) {
        (
            (board.0 - self.x) * self.zoom,
            (board.1 - self.y) * self.zoom,
        )
    }

    /// Moved by a distance in screen pixels: a positive `dx` shows what is
    /// further right.
    pub fn panned(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx / self.zoom,
            y: self.y + dy / self.zoom,
            ..self
        }
    }

    /// Zoomed by `factor`, clamped, keeping the board point under the
    /// widget-local point `about` where it is.
    pub fn zoomed(self, factor: f32, about: (f32, f32)) -> Self {
        let zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let (x, y) = self.to_board(about);
        Self {
            x: x - about.0 / zoom,
            y: y - about.1 / zoom,
            zoom,
        }
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zoom_keeps_the_point_under_the_pointer() {
        let about = (317.0, 211.0);
        let mut viewport = Viewport {
            x: -40.0,
            y: 125.5,
            zoom: 1.0,
        };
        for factor in [1.1, 1.1, 0.5, 3.0, 0.2, 1.05] {
            let before = viewport.to_board(about);
            viewport = viewport.zoomed(factor, about);
            let after = viewport.to_screen(before);
            assert!((after.0 - about.0).abs() < 0.01 && (after.1 - about.1).abs() < 0.01);
        }
    }

    #[test]
    fn zoom_is_clamped_to_ten_and_four_hundred_percent() {
        let viewport = Viewport::default();
        assert_eq!(viewport.zoomed(100.0, (0.0, 0.0)).zoom, 4.0);
        assert_eq!(viewport.zoomed(0.001, (0.0, 0.0)).zoom, 0.1);
    }

    #[test]
    fn a_pan_moves_by_screen_pixels_at_any_zoom() {
        let viewport = Viewport {
            x: 10.0,
            y: 20.0,
            zoom: 2.0,
        };
        let panned = viewport.panned(8.0, -4.0);
        assert_eq!((panned.x, panned.y, panned.zoom), (14.0, 18.0, 2.0));
        assert_eq!(panned.to_screen((14.0, 18.0)), (0.0, 0.0));
    }
}
