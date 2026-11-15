//! Colours: a file names a colour token and a fill level, and the theme's
//! [`Palette`] turns them into the colours drawn. The label colour is
//! derived for contrast here and never stored.

use crate::{Color, Fill, Palette, Rgba, Token};

/// How much of the hue a `tint` fill mixes into the canvas colour, in a
/// dark theme and in a light one.
pub const TINT_DARK: f32 = 0.16;
pub const TINT_LIGHT: f32 = 0.12;

/// The colours one element is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Resolved {
    /// The outline, or a connector's line and arrowheads.
    pub stroke: Rgba,
    /// `None` for `fill = "none"`.
    pub fill: Option<Rgba>,
    /// A label drawn on the fill, or on the canvas when there is none.
    pub label: Rgba,
}

/// The colours of `color` at `fill` in `palette`. `neutral`, and a colour
/// this build does not know, stroke in `ink`; a literal `#rrggbb` is the hue
/// unchanged in every theme.
pub fn resolve(color: &Color, fill: Fill, palette: &Palette) -> Resolved {
    let hue = hue(color, palette);
    let stroke = hue.unwrap_or(palette.ink);
    match fill {
        Fill::None => Resolved {
            stroke,
            fill: None,
            label: palette.ink,
        },
        Fill::Tint => {
            let amount = if palette.dark { TINT_DARK } else { TINT_LIGHT };
            Resolved {
                stroke,
                fill: Some(mix(palette.canvas, hue.unwrap_or(palette.ink), amount)),
                label: palette.ink,
            }
        }
        Fill::Solid => {
            let fill = hue.unwrap_or(palette.raised);
            Resolved {
                stroke,
                fill: Some(fill),
                label: on(fill, palette),
            }
        }
    }
}

/// The colour a token or literal names, or `None` for `neutral` and for a
/// colour this build does not know.
fn hue(color: &Color, palette: &Palette) -> Option<Rgba> {
    match color {
        Color::Token(Token::Neutral) | Color::Unknown(_) => None,
        Color::Token(token) => {
            let index = (*token as usize).checked_sub(1)?;
            palette.hues.get(index).copied()
        }
        Color::Hex([r, g, b]) => Some(Rgba::rgb(*r, *g, *b)),
    }
}

/// `amount` of `over` mixed into `base`, per channel, rounded; opaque.
fn mix(base: Rgba, over: Rgba, amount: f32) -> Rgba {
    let channel =
        |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    Rgba::rgb(
        channel(base.r, over.r),
        channel(base.g, over.g),
        channel(base.b, over.b),
    )
}

/// Whichever of `ink` and `canvas` contrasts more with `fill`; `ink` on a
/// tie.
fn on(fill: Rgba, palette: &Palette) -> Rgba {
    if contrast(palette.canvas, fill) > contrast(palette.ink, fill) {
        palette.canvas
    } else {
        palette.ink
    }
}

/// The WCAG 2 contrast ratio of two colours, 1 to 21.
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The WCAG 2 relative luminance of an sRGB colour.
fn luminance(color: Rgba) -> f32 {
    let linear = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Vocabulary;

    /// A light palette for tests and the golden scene dumps. Not a theme:
    /// the themes' palettes live in `wren-core`.
    pub(crate) const LIGHT: Palette = Palette {
        canvas: Rgba::rgb(0xff, 0xff, 0xff),
        ink: Rgba::rgb(0x20, 0x24, 0x28),
        muted: Rgba::rgb(0x80, 0x88, 0x90),
        raised: Rgba::rgb(0xe0, 0xe4, 0xe8),
        accent: Rgba::rgb(0x20, 0x60, 0xc0),
        issue: Rgba::rgb(0xd0, 0x20, 0x20),
        hues: [
            Rgba::rgb(0xc0, 0x30, 0x30),
            Rgba::rgb(0xd0, 0x70, 0x10),
            Rgba::rgb(0xf0, 0xd0, 0x40),
            Rgba::rgb(0x30, 0x90, 0x40),
            Rgba::rgb(0x20, 0x90, 0xa0),
            Rgba::rgb(0x80, 0x40, 0xb0),
            Rgba::rgb(0x30, 0x60, 0xd0),
        ],
        dark: false,
    };

    pub(crate) const DARK: Palette = Palette {
        canvas: Rgba::rgb(0x18, 0x1a, 0x1e),
        ink: Rgba::rgb(0xe8, 0xea, 0xee),
        muted: Rgba::rgb(0x80, 0x88, 0x90),
        raised: Rgba::rgb(0x30, 0x34, 0x3a),
        dark: true,
        ..LIGHT
    };

    fn hex(text: &str) -> Rgba {
        let Color::Hex([r, g, b]) = Color::read(text.into()) else {
            panic!("{text} is not #rrggbb");
        };
        Rgba::rgb(r, g, b)
    }

    /// Every token, a literal, and an unknown string at each fill, in both
    /// palettes: stroke, fill, label.
    #[test]
    fn the_resolve_table() {
        type Row = (
            &'static str,
            Fill,
            &'static str,
            Option<&'static str>,
            &'static str,
        );
        #[rustfmt::skip]
        let light: &[Row] = &[
            ("neutral", Fill::None,  "#202428", None,            "#202428"),
            ("neutral", Fill::Tint,  "#202428", Some("#e4e5e5"), "#202428"),
            ("neutral", Fill::Solid, "#202428", Some("#e0e4e8"), "#202428"),
            ("red",     Fill::None,  "#c03030", None,            "#202428"),
            ("red",     Fill::Tint,  "#c03030", Some("#f7e6e6"), "#202428"),
            ("red",     Fill::Solid, "#c03030", Some("#c03030"), "#ffffff"),
            ("orange",  Fill::Tint,  "#d07010", Some("#f9eee2"), "#202428"),
            ("orange",  Fill::Solid, "#d07010", Some("#d07010"), "#202428"),
            ("yellow",  Fill::Tint,  "#f0d040", Some("#fdf9e8"), "#202428"),
            ("yellow",  Fill::Solid, "#f0d040", Some("#f0d040"), "#202428"),
            ("green",   Fill::Tint,  "#309040", Some("#e6f2e8"), "#202428"),
            ("green",   Fill::Solid, "#309040", Some("#309040"), "#ffffff"),
            ("cyan",    Fill::Tint,  "#2090a0", Some("#e4f2f4"), "#202428"),
            ("cyan",    Fill::Solid, "#2090a0", Some("#2090a0"), "#202428"),
            ("purple",  Fill::Tint,  "#8040b0", Some("#f0e8f6"), "#202428"),
            ("purple",  Fill::Solid, "#8040b0", Some("#8040b0"), "#ffffff"),
            ("blue",    Fill::Tint,  "#3060d0", Some("#e6ecf9"), "#202428"),
            ("blue",    Fill::Solid, "#3060d0", Some("#3060d0"), "#ffffff"),
            ("#123456", Fill::None,  "#123456", None,            "#202428"),
            ("#ABCDEF", Fill::Tint,  "#abcdef", Some("#f5f9fd"), "#202428"),
            ("#000000", Fill::Solid, "#000000", Some("#000000"), "#ffffff"),
            ("#ffffff", Fill::Solid, "#ffffff", Some("#ffffff"), "#202428"),
            ("mauve",   Fill::None,  "#202428", None,            "#202428"),
            ("mauve",   Fill::Tint,  "#202428", Some("#e4e5e5"), "#202428"),
            ("mauve",   Fill::Solid, "#202428", Some("#e0e4e8"), "#202428"),
            ("Red",     Fill::Solid, "#202428", Some("#e0e4e8"), "#202428"),
            ("#12345",  Fill::Solid, "#202428", Some("#e0e4e8"), "#202428"),
        ];
        #[rustfmt::skip]
        let dark: &[Row] = &[
            ("neutral", Fill::None,  "#e8eaee", None,            "#e8eaee"),
            ("neutral", Fill::Tint,  "#e8eaee", Some("#393b3f"), "#e8eaee"),
            ("neutral", Fill::Solid, "#e8eaee", Some("#30343a"), "#e8eaee"),
            ("red",     Fill::Tint,  "#c03030", Some("#331e21"), "#e8eaee"),
            ("red",     Fill::Solid, "#c03030", Some("#c03030"), "#e8eaee"),
            ("yellow",  Fill::Solid, "#f0d040", Some("#f0d040"), "#181a1e"),
            ("green",   Fill::Solid, "#309040", Some("#309040"), "#181a1e"),
            ("#ffffff", Fill::Tint,  "#ffffff", Some("#3d3f42"), "#e8eaee"),
            ("#000000", Fill::Solid, "#000000", Some("#000000"), "#e8eaee"),
            ("mauve",   Fill::Tint,  "#e8eaee", Some("#393b3f"), "#e8eaee"),
        ];
        for (palette, rows) in [(&LIGHT, light), (&DARK, dark)] {
            for &(color, fill, stroke, filled, label) in rows {
                let got = resolve(&Color::read(color.into()), fill, palette);
                let want = Resolved {
                    stroke: hex(stroke),
                    fill: filled.map(hex),
                    label: hex(label),
                };
                assert_eq!(got, want, "{color} {fill:?} dark={}", palette.dark);
            }
        }
    }

    #[test]
    fn every_token_has_its_own_hue() {
        for (index, &token) in Token::ALL.iter().enumerate().skip(1) {
            let got = resolve(&Color::Token(token), Fill::None, &LIGHT);
            assert_eq!(got.stroke, LIGHT.hues[index - 1], "{}", token.name());
        }
    }

    #[test]
    fn a_solid_label_takes_the_higher_contrast() {
        for palette in [LIGHT, DARK] {
            for &token in Token::ALL {
                let got = resolve(&Color::Token(token), Fill::Solid, &palette);
                let fill = got.fill.unwrap();
                let other = if got.label == palette.ink {
                    palette.canvas
                } else {
                    palette.ink
                };
                assert!(contrast(got.label, fill) >= contrast(other, fill));
            }
        }
    }

    #[test]
    fn contrast_is_symmetric_and_spans_one_to_twenty_one() {
        let (black, white) = (Rgba::rgb(0, 0, 0), Rgba::rgb(255, 255, 255));
        assert!((contrast(black, white) - 21.0).abs() < 1e-3);
        assert_eq!(contrast(white, black), contrast(black, white));
        assert_eq!(contrast(white, white), 1.0);
    }
}
