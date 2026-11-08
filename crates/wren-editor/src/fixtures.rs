//! Colours and a font for the crate's own tests.

use std::borrow::Cow;

use iced::advanced::graphics::text::font_system;
use iced::{Color, Font};

use crate::{Palette, Style};

pub const FONT: Font = Font::with_name("JetBrains Mono");

pub const STYLE: Style = Style {
    background: Color::from_rgb8(0x14, 0x16, 0x1b),
    value: Color::from_rgb8(0xd5, 0xd9, 0xe0),
    selection: Color::from_rgb8(0x2a, 0x31, 0x40),
    gutter: Color::from_rgb8(0x8a, 0x91, 0x9c),
    current_line: Color::from_rgb8(0x19, 0x1d, 0x25),
    caret: Color::from_rgb8(0xf0, 0xb4, 0x5a),
    strike: Color::from_rgb8(0x8a, 0x91, 0x9c),
};

pub const DARK: Palette = Palette {
    text: Color::from_rgb8(0xe6, 0xe8, 0xec),
    muted: Color::from_rgb8(0xa3, 0xaa, 0xb5),
    faint: Color::from_rgb8(0x8a, 0x91, 0x9c),
    accent: Color::from_rgb8(0xf0, 0xb4, 0x5a),
    danger: Color::from_rgb8(0xf0, 0x88, 0x88),
    success: Color::from_rgb8(0x7f, 0xd1, 0xa0),
};

pub const LIGHT: Palette = Palette {
    text: Color::from_rgb8(0x20, 0x24, 0x2d),
    muted: Color::from_rgb8(0x58, 0x61, 0x70),
    faint: Color::from_rgb8(0x60, 0x6a, 0x78),
    accent: Color::from_rgb8(0x9a, 0x58, 0x00),
    danger: Color::from_rgb8(0xb3, 0x2d, 0x36),
    success: Color::from_rgb8(0x19, 0x73, 0x42),
};

/// Registers [`FONT`] so layout tests measure real glyphs rather than a
/// system fallback.
pub fn load_font() {
    font_system()
        .write()
        .expect("font system")
        .load_font(Cow::Borrowed(include_bytes!(
            "../../../third_party/jetbrains-mono/JetBrainsMono-Regular.ttf"
        )));
}
