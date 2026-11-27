//! Single-line text that ends in “…” when it does not fit.
//!
//! iced 0.14 text clips at its bounds. This widget shapes the full text once
//! per (text, width) pair and, when it overflows, binary-searches the longest
//! prefix that fits alongside the ellipsis. The text may be drawn in runs
//! of colour ([`ellipsis_runs`]), the ellipsis in the colour of the run it
//! cuts, and struck through.

use std::borrow::Cow;

use iced::advanced::layout::{self, Layout};
use iced::advanced::text::{self, Paragraph as _, Text};
use iced::advanced::widget::{tree, Tree};
use iced::advanced::{mouse, renderer, Widget};
use iced::alignment;
use iced::widget::text::{LineHeight, Shaping, Wrapping};
use iced::{Color, Element, Font, Length, Pixels, Point, Rectangle, Size};

pub struct Ellipsis<'a> {
    content: Cow<'a, str>,
    size: f32,
    font: Font,
    color: Color,
    width: Length,
    /// Where each run of colour ends in `content`, and its colour; none
    /// draws it all in `color`.
    runs: Vec<(usize, Color)>,
    strike: bool,
}

/// `content` at `size` px, filling the available width by default.
pub fn ellipsis<'a>(content: impl Into<Cow<'a, str>>, size: f32, color: Color) -> Ellipsis<'a> {
    Ellipsis {
        content: content.into(),
        size,
        font: crate::fonts::UI,
        color,
        width: Length::Fill,
        runs: Vec::new(),
        strike: false,
    }
}

/// `runs` of text, each in its colour, as one line at `size` px that ends
/// in "…" when it does not fit; `color` is the colour of what no run
/// colours, and of the strike.
pub fn ellipsis_runs<'a>(runs: Vec<(String, Color)>, size: f32, color: Color) -> Ellipsis<'a> {
    let mut content = String::new();
    let mut ends = Vec::with_capacity(runs.len());
    for (text, run) in runs {
        content.push_str(&text);
        ends.push((content.len(), run));
    }
    Ellipsis {
        runs: ends,
        ..ellipsis(content, size, color)
    }
}

impl Ellipsis<'_> {
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    /// A line through the text, in its colour.
    pub fn strikethrough(mut self, strike: bool) -> Self {
        self.strike = strike;
        self
    }

    /// `Shrink` takes only the text's width (up to the available space).
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    fn line_height(&self) -> f32 {
        (self.size * 1.3).round()
    }
}

struct State<P> {
    key: Option<Key>,
    paragraph: P,
    width: f32,
}

#[derive(PartialEq)]
struct Key {
    content: String,
    width: u32,
    size: u32,
    font: Font,
    runs: Vec<(usize, [u32; 4])>,
}

fn bits(color: Color) -> [u32; 4] {
    [color.r, color.g, color.b, color.a].map(f32::to_bits)
}

impl Key {
    fn matches(
        &self,
        content: &str,
        width: f32,
        size: f32,
        font: Font,
        runs: &[(usize, Color)],
    ) -> bool {
        self.content == content
            && self.width == width.to_bits()
            && self.size == size.to_bits()
            && self.font == font
            && self.runs.len() == runs.len()
            && (self.runs.iter().zip(runs))
                .all(|(known, (end, color))| *known == (*end, bits(*color)))
    }
}

/// `shown` (all of `content`, or a prefix of it and "…") in the colours of
/// `runs`: the ellipsis takes the colour of the run it cuts.
fn coloured<P: text::Paragraph<Font = Font>>(
    content: &str,
    shown: &str,
    runs: &[(usize, Color)],
    size: f32,
    font: Font,
) -> P {
    let cut = if shown == content {
        content.len()
    } else {
        shown.len() - '…'.len_utf8()
    };
    let mut spans: Vec<text::Span<'_, (), Font>> = Vec::new();
    let mut start = 0;
    let mut last = None;
    for &(end, color) in runs {
        if start < cut {
            let piece = &shown[start..end.min(cut)];
            if !piece.is_empty() {
                spans.push(text::Span::new(piece).color(color));
            }
            last = Some(color);
        }
        start = end;
    }
    if cut < shown.len() {
        let mut dots = text::Span::new("…");
        if let Some(color) = last {
            dots = dots.color(color);
        }
        spans.push(dots);
    }
    P::with_spans(Text {
        content: &spans[..],
        bounds: Size::INFINITE,
        size: Pixels(size),
        line_height: LineHeight::Relative(1.3),
        font,
        align_x: text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
    })
}

fn shaped<P: text::Paragraph<Font = Font>>(content: &str, size: f32, font: Font) -> P {
    P::with_text(Text {
        content,
        bounds: Size::INFINITE,
        size: Pixels(size),
        line_height: LineHeight::Relative(1.3),
        font,
        align_x: text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
    })
}

/// The longest prefix of `content` (on a char boundary) for which `fits`
/// holds with an ellipsis appended, or just "…" when nothing fits.
pub fn truncate(content: &str, mut fits: impl FnMut(&str) -> bool) -> String {
    if fits(content) {
        return content.to_string();
    }
    // Candidate prefixes end before each char start; the full text (which
    // did not fit) is never a candidate.
    let boundaries: Vec<usize> = content.char_indices().map(|(index, _)| index).collect();
    let (mut low, mut high) = (0, boundaries.len().saturating_sub(1));
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate = format!("{}…", content[..boundaries[middle]].trim_end());
        if fits(&candidate) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    format!(
        "{}…",
        content[..boundaries.get(low).copied().unwrap_or(0)].trim_end()
    )
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Ellipsis<'_>
where
    Renderer: text::Renderer<Font = Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Renderer::Paragraph> {
            key: None,
            paragraph: Renderer::Paragraph::default(),
            width: 0.0,
        })
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let available = limits.max().width;
        let fresh = state.key.as_ref().is_some_and(|key| {
            key.matches(&self.content, available, self.size, self.font, &self.runs)
        });
        if !fresh {
            let (size, font) = (self.size, self.font);
            let measure = |candidate: &str| {
                shaped::<Renderer::Paragraph>(candidate, size, font)
                    .min_bounds()
                    .width
            };
            let shown = truncate(&self.content, |candidate| measure(candidate) <= available);
            state.paragraph = if self.runs.is_empty() {
                shaped(&shown, size, font)
            } else {
                coloured(&self.content, &shown, &self.runs, size, font)
            };
            state.width = state.paragraph.min_bounds().width.min(available);
            state.key = Some(Key {
                content: self.content.to_string(),
                width: available.to_bits(),
                size: self.size.to_bits(),
                font: self.font,
                runs: (self.runs.iter())
                    .map(|(end, color)| (*end, bits(*color)))
                    .collect(),
            });
        }
        let width = match self.width {
            Length::Shrink => state.width,
            _ => available,
        };
        layout::Node::new(limits.resolve(
            self.width,
            Length::Shrink,
            Size::new(width, self.line_height()),
        ))
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let bounds = layout.bounds();
        let clip = bounds.intersection(viewport).unwrap_or(bounds);
        renderer.fill_paragraph(
            &state.paragraph,
            Point::new(bounds.x, bounds.y),
            self.color,
            clip,
        );
        if self.strike && state.width > 0.0 {
            let line = Rectangle::new(
                Point::new(bounds.x, bounds.y + (self.line_height() / 2.0).round()),
                Size::new(state.width, 1.0),
            );
            if let Some(line) = line.intersection(&clip) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: line,
                        ..renderer::Quad::default()
                    },
                    self.color,
                );
            }
        }
    }
}

impl<'a, Message, Theme, Renderer> From<Ellipsis<'a>> for Element<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn from(widget: Ellipsis<'a>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::truncate;

    fn fits_chars(limit: usize) -> impl FnMut(&str) -> bool {
        move |candidate| candidate.chars().count() <= limit
    }

    #[test]
    fn short_text_is_unchanged_and_long_text_ends_in_an_ellipsis() {
        assert_eq!(truncate("Alice Chen", fits_chars(20)), "Alice Chen");
        assert_eq!(
            truncate("API migration: cutover plan", fits_chars(10)),
            "API migra…"
        );
        // Trailing spaces before the ellipsis are dropped.
        assert_eq!(truncate("API migration", fits_chars(5)), "API…");
        assert_eq!(truncate("anything", fits_chars(0)), "…");
    }

    #[test]
    fn a_last_character_wider_than_the_ellipsis_never_indexes_past_the_end() {
        // Everything but the full text fits: the longest candidate is the
        // first n-1 characters plus the ellipsis.
        assert_eq!(truncate("ab", |candidate| candidate != "ab"), "a…");
        assert_eq!(truncate("W", |candidate| candidate != "W"), "…");
        assert_eq!(truncate("", |_| false), "…");
    }

    #[test]
    fn truncation_respects_multibyte_boundaries() {
        assert_eq!(truncate("café 🐦 notes", fits_chars(7)), "café 🐦…");
        assert_eq!(truncate("日本語のノート", fits_chars(4)), "日本語…");
    }
}
