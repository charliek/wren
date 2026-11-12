//! The display list of a board: what the canvas widget paints and the SVG
//! writer serialises, so the two cannot disagree.
//!
//! Coordinates are board units as `f32`: one logical pixel at zoom 1, x
//! growing right and y growing down. Labels arrive wrapped into lines, each
//! placed and measured, so a painter never lays text out itself.

use unicode_segmentation::UnicodeSegmentation;

use crate::{Id, Rgba};

/// Text measurement, supplied by the host: the canvas measures with the
/// font it draws with, tests with [`FixedMeasure`].
pub trait Measure {
    /// The advance width of `text` on one line.
    fn width(&self, text: &str, face: Face, size: f32) -> f32;
    fn line_height(&self, face: Face, size: f32) -> f32;
}

/// A label typeface. Ring 0 draws every label in Inter Regular.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Regular,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A board's display list.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// The union of the items' bounds.
    pub bounds: Rect,
    /// Back to front.
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The element the item belongs to.
    pub source: Source,
    /// Everything the item paints, strokes and text included.
    pub bounds: Rect,
    pub draw: Draw,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Node(Id),
    Edge(Id),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Path {
        segments: Vec<Segment>,
        fill: Option<Rgba>,
        stroke: Option<Stroke>,
    },
    Text {
        lines: Vec<Line>,
        face: Face,
        size: f32,
        color: Rgba,
    },
}

/// One step of a path, in the order SVG and iced's path builder take them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    MoveTo(Point),
    LineTo(Point),
    QuadTo {
        control: Point,
        to: Point,
    },
    CubicTo {
        first: Point,
        second: Point,
        to: Point,
    },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f32,
    /// On and off lengths; `None` is a solid line.
    pub dash: Option<[f32; 2]>,
}

/// One wrapped line of a label.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// The top-left corner of the line's box.
    pub origin: Point,
    /// The measured advance width of `text`.
    pub width: f32,
}

/// A measurer with no font, for tests and golden scene dumps: every
/// grapheme is half the font size wide, and a line is 1.25 times the font
/// size tall. Both factors are exact in binary, so dumps do not drift.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedMeasure;

impl Measure for FixedMeasure {
    fn width(&self, text: &str, _face: Face, size: f32) -> f32 {
        text.graphemes(true).count() as f32 * size * 0.5
    }

    fn line_height(&self, _face: Face, size: f32) -> f32 {
        size * 1.25
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_measure_counts_graphemes() {
        let m = FixedMeasure;
        assert_eq!(m.width("", Face::Regular, 14.0), 0.0);
        assert_eq!(m.width("Start", Face::Regular, 14.0), 35.0);
        // A decomposed é, a flag and a family emoji are one grapheme each.
        assert_eq!(m.width("e\u{301}🇳🇿👨‍👩‍👧", Face::Regular, 12.0), 18.0);
        assert_eq!(m.line_height(Face::Regular, 14.0), 17.5);
    }
}
