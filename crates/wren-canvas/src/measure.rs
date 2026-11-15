//! Label widths from the text stack the renderer draws with, so a line
//! the scene wraps is as wide on screen as the scene believes.
//!
//! cosmic-text is reached through iced, never as a dependency of its own:
//! the font system is the one iced shapes with, so a font the host loads
//! is the font measured. A host therefore loads its faces before it opens
//! a view; a label measured in a fallback face keeps that width until the
//! scene is next built.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::PoisonError;

use iced::advanced::graphics::text::{self as graphics, cosmic_text, Version};
use iced::Font;
use wren_board::scene::{self, Face, Measure};

/// Measures label text in the host's label font.
///
/// A scene asks for the same strings each time it is built, and shaping
/// one costs microseconds, so widths are kept by face, size and text.
/// [`Measurer::sweep`] and [`Measurer::trim`] bound what is kept to what
/// recent builds used.
pub struct Measurer {
    font: Font,
    cache: RefCell<Cache>,
}

#[derive(Default)]
struct Cache {
    widths: HashMap<(Face, u32), HashMap<Box<str>, Width>>,
    /// Counts the sweeps; a width remembers the last one it was asked in.
    sweep: u64,
    /// The fonts the widths were shaped with.
    fonts: Option<Version>,
    /// Reused across strings: a new buffer allocates on every measurement.
    buffer: Option<cosmic_text::Buffer>,
    #[cfg(test)]
    shaped: u64,
}

struct Width {
    width: f32,
    asked: u64,
}

impl Measurer {
    /// A measurer for labels drawn in `font`.
    pub fn new(font: Font) -> Self {
        Self {
            font,
            cache: RefCell::default(),
        }
    }

    /// The font [`Face::Regular`] is drawn in.
    pub fn font(&self, face: Face) -> Font {
        match face {
            Face::Regular => self.font,
        }
    }

    /// Call before each scene build. It forgets the widths that the build
    /// before the last did not ask for, and every width when fonts have
    /// been loaded since, so the cache follows the board instead of
    /// growing with each label ever typed.
    pub fn sweep(&self) {
        let fonts = graphics::font_system()
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .version();
        let mut cache = self.cache.borrow_mut();
        if cache.fonts != Some(fonts) {
            cache.fonts = Some(fonts);
            cache.widths.clear();
        }
        let sweep = cache.sweep;
        for widths in cache.widths.values_mut() {
            widths.retain(|_, width| width.asked == sweep);
        }
        cache.sweep = sweep + 1;
    }

    /// Call after a build that measured every label, following a
    /// [`Measurer::sweep`]: forgets every width that build did not ask
    /// for, so what is kept is what the board needs.
    pub fn trim(&self) {
        let mut cache = self.cache.borrow_mut();
        let sweep = cache.sweep;
        for widths in cache.widths.values_mut() {
            widths.retain(|_, width| width.asked == sweep);
        }
    }

    /// Whether fonts have been loaded since the widths kept were shaped,
    /// or none have been yet: a width measured now may differ from one
    /// measured before, until the next [`Measurer::sweep`].
    pub fn stale(&self) -> bool {
        let fonts = graphics::font_system()
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .version();
        self.cache.borrow().fonts != Some(fonts)
    }

    /// How many widths are kept.
    pub fn kept(&self) -> usize {
        self.cache.borrow().widths.values().map(HashMap::len).sum()
    }

    #[cfg(test)]
    fn shaped(&self) -> u64 {
        self.cache.borrow().shaped
    }
}

impl Cache {
    /// Shape `text` on one unbounded line, as a paragraph of the same
    /// text is shaped when it is drawn.
    fn shape(&mut self, text: &str, font: Font, size: f32) -> f32 {
        let mut fonts = graphics::font_system()
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let metrics = cosmic_text::Metrics::new(size, scene::line_height(size));
        let buffer = match &mut self.buffer {
            Some(buffer) => {
                buffer.set_metrics(fonts.raw(), metrics);
                buffer
            }
            empty => {
                let mut buffer = cosmic_text::Buffer::new(fonts.raw(), metrics);
                buffer.set_size(fonts.raw(), None, None);
                buffer.set_wrap(fonts.raw(), cosmic_text::Wrap::None);
                empty.insert(buffer)
            }
        };
        buffer.set_text(
            fonts.raw(),
            text,
            &graphics::to_attributes(font),
            cosmic_text::Shaping::Advanced,
            None,
        );
        #[cfg(test)]
        {
            self.shaped += 1;
        }
        graphics::measure(buffer).0.width
    }
}

impl Measure for Measurer {
    fn width(&self, text: &str, face: Face, size: f32) -> f32 {
        // A buffer cannot be made with a line height of zero.
        if text.is_empty() || !size.is_finite() || size <= 0.0 {
            return 0.0;
        }
        let mut cache = self.cache.borrow_mut();
        let sweep = cache.sweep;
        let key = (face, size.to_bits());
        if let Some(known) = cache
            .widths
            .get_mut(&key)
            .and_then(|widths| widths.get_mut(text))
        {
            known.asked = sweep;
            return known.width;
        }
        let width = cache.shape(text, self.font(face), size);
        cache.widths.entry(key).or_default().insert(
            text.into(),
            Width {
                width,
                asked: sweep,
            },
        );
        width
    }

    /// The scene's own line height: the scene places lines by it, and the
    /// SVG writer, which has no measurer, must place them identically.
    fn line_height(&self, _face: Face, size: f32) -> f32 {
        scene::line_height(size)
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::text::{self, Paragraph as _};
    use iced::{alignment, Pixels, Size};
    use wren_board::scene::{Draw, LABEL_SIZE, SMALL_SIZE};
    use wren_board::{shape, Shape};

    use super::*;
    use crate::fixtures;

    type Paragraph = <iced::Renderer as text::Renderer>::Paragraph;

    /// The width iced gives the paragraph the widget draws for one line.
    fn drawn(line: &str, size: f32) -> f32 {
        Paragraph::with_text(text::Text {
            content: line,
            bounds: Size::INFINITE,
            size: Pixels(size),
            line_height: text::LineHeight::Absolute(Pixels(scene::line_height(size))),
            font: fixtures::FONT,
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        })
        .min_bounds()
        .width
    }

    fn lines() -> impl Iterator<Item = &'static str> {
        fixtures::LABELS.iter().flat_map(|label| label.split('\n'))
    }

    #[test]
    fn the_corpus_is_the_twenty_four_labels_in_twenty_five_lines() {
        assert_eq!(fixtures::LABELS.len(), 24);
        assert_eq!(lines().count(), 25);
    }

    #[test]
    fn a_width_is_the_width_of_the_paragraph_that_is_drawn() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        for size in [LABEL_SIZE, SMALL_SIZE] {
            for line in lines() {
                let measured = measurer.width(line, Face::Regular, size);
                let drawn = drawn(line, size);
                assert!(
                    (measured - drawn).abs() <= 0.5,
                    "{line:?} at {size}: measured {measured}, drawn {drawn}"
                );
                assert!(measured > 0.0, "{line:?}");
            }
        }
    }

    #[test]
    fn labels_are_measured_in_inter() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        // Inter's advances at 14 px. A fallback face would not give them,
        // and CI has no other font to fall back to.
        for (text, width) in [("Start", 31.56), ("Notify customer", 106.10), ("0", 8.83)] {
            let measured = measurer.width(text, Face::Regular, LABEL_SIZE);
            assert!((measured - width).abs() < 0.01, "{text:?}: {measured}");
        }
        // Twice the size is twice as wide: nothing is hinted to the grid.
        let wide = measurer.width("Notify customer", Face::Regular, LABEL_SIZE * 2.0);
        assert!((wide - 212.20).abs() < 0.01, "{wide}");
    }

    #[test]
    fn line_heights_are_the_scenes() {
        let measurer = Measurer::new(fixtures::FONT);
        assert_eq!(measurer.line_height(Face::Regular, LABEL_SIZE), 20.0);
        assert_eq!(measurer.line_height(Face::Regular, SMALL_SIZE), 16.0);
        assert_eq!(measurer.line_height(Face::Regular, 28.0), 40.0);
    }

    #[test]
    fn nothing_to_measure_is_no_width_and_no_shaping() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        assert_eq!(measurer.width("", Face::Regular, LABEL_SIZE), 0.0);
        for size in [0.0, -14.0, f32::NAN, f32::INFINITY] {
            assert_eq!(measurer.width("text", Face::Regular, size), 0.0);
        }
        assert_eq!(measurer.shaped(), 0);
    }

    #[test]
    fn a_string_is_shaped_once_per_size() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        measurer.sweep();
        let first = measurer.width("Receive order", Face::Regular, LABEL_SIZE);
        for _ in 0..10 {
            assert_eq!(
                measurer.width("Receive order", Face::Regular, LABEL_SIZE),
                first
            );
        }
        assert_eq!(measurer.shaped(), 1);
        let small = measurer.width("Receive order", Face::Regular, SMALL_SIZE);
        assert!(small < first);
        assert_eq!(measurer.shaped(), 2);
        // The reused buffer does not carry one string's width to the next.
        assert!(measurer.width("Ship", Face::Regular, LABEL_SIZE) < small);
        assert_eq!(
            measurer.width("Receive order", Face::Regular, LABEL_SIZE),
            first
        );
        assert_eq!(measurer.shaped(), 3);
    }

    #[test]
    fn a_sweep_keeps_what_the_last_build_asked_for() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        let ask = |texts: &[&str]| {
            measurer.sweep();
            for text in texts {
                measurer.width(text, Face::Regular, LABEL_SIZE);
            }
        };
        ask(&["one", "two", "three"]);
        assert_eq!((measurer.kept(), measurer.shaped()), (3, 3));
        // A build that asks again shapes nothing.
        ask(&["one", "two", "three"]);
        assert_eq!((measurer.kept(), measurer.shaped()), (3, 3));
        // A label changes: its old width goes one build later.
        ask(&["one", "two", "four"]);
        assert_eq!((measurer.kept(), measurer.shaped()), (4, 4));
        ask(&["one", "two", "four"]);
        assert_eq!((measurer.kept(), measurer.shaped()), (3, 4));
        // Undo within one build of the change costs nothing.
        ask(&["one", "two", "three"]);
        assert_eq!(measurer.shaped(), 5);
    }

    /// A build that measured every label trims what it did not ask for,
    /// what builds since the sweep before asked included; a font loaded
    /// makes the widths stale until the next sweep.
    #[test]
    fn a_trim_keeps_what_the_build_after_the_sweep_asked_for() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        assert!(measurer.stale());
        measurer.sweep();
        assert!(!measurer.stale());
        let ask = |texts: &[&str]| {
            for text in texts {
                measurer.width(text, Face::Regular, LABEL_SIZE);
            }
        };
        ask(&["one", "two"]);
        // Builds of one changed label at a time, with no sweep between.
        ask(&["t"]);
        ask(&["th"]);
        measurer.sweep();
        ask(&["one", "two", "three"]);
        measurer.trim();
        assert_eq!(measurer.kept(), 3);
        assert_eq!(measurer.shaped(), 5);
    }

    /// One node of the default size with `label`, laid out by the scene.
    fn fits(label: &str, shape: Shape, measurer: &Measurer) -> Result<(), String> {
        let name = <Shape as wren_board::Vocabulary>::name(shape);
        let board = wren_board::parse(&format!(
            "schema = 1\n\n[node.n]\ntext = \"{label}\"\nkind = \"shape\"\nat = [0, 0]\n\
             shape = \"{name}\"\nsize = [120, 60]\nz = \"V\"\n"
        ))
        .map_err(|error| error.message)?;
        let scene = scene::build(&board, &fixtures::palette(), measurer);
        let bounds = scene::Rect::new(0.0, 0.0, 120.0, 60.0);
        let inside = shape::label_box(shape, bounds);
        let lines = scene
            .items
            .iter()
            .find_map(|item| match &item.draw {
                Draw::Text { lines, .. } => Some(lines),
                Draw::Path { .. } => None,
            })
            .ok_or("no label was laid out")?;
        // The slack the scene's wrap allows a line measured at the width.
        let slack = 0.001;
        let height = scene::line_height(LABEL_SIZE);
        for line in lines {
            let fits = line.origin.x >= inside.x - slack
                && line.origin.x + line.width <= inside.right() + slack
                && line.origin.y >= inside.y - slack
                && line.origin.y + height <= inside.bottom() + slack;
            if !fits {
                return Err(format!(
                    "{:?} at {:?}, {} wide, leaves {inside:?} ({} lines)",
                    line.text,
                    line.origin,
                    line.width,
                    lines.len()
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn the_acceptance_scripts_labels_fit_their_default_boxes() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        for (label, shape) in fixtures::SCRIPT {
            if let Err(error) = fits(label, shape, &measurer) {
                panic!("{label:?} in a default {shape:?}: {error}");
            }
        }
        assert_eq!(fixtures::SCRIPT.len(), 15);
    }

    /// The script's longest label is 106.1 units wide, two more than a
    /// default rectangle's label box, so it takes two lines in every
    /// shape. Only the cylinder's box, 36 units under its cap, is too low
    /// for them; the diamond's, 80 by 40, holds them exactly.
    #[test]
    fn notify_customer_fits_every_shape_but_the_cylinder() {
        fixtures::load_fonts();
        let measurer = Measurer::new(fixtures::FONT);
        for shape in <Shape as wren_board::Vocabulary>::ALL {
            let fitted = fits("Notify customer", *shape, &measurer);
            if *shape == Shape::Cylinder {
                assert!(fitted.is_err(), "it fits the cylinder after all");
            } else if let Err(error) = fitted {
                panic!("{shape:?}: {error}");
            }
        }
    }
}
