//! Colours and a font for the crate's own tests.

use std::borrow::Cow;
use std::sync::Once;

use iced::advanced::graphics::text::font_system;
use iced::{Color, Font};

use crate::highlighter::Settings;
use crate::{Palette, Style};

pub const FONT: Font = Font::with_name("JetBrains Mono");

pub const STYLE: Style = Style {
    background: Color::from_rgb8(0x14, 0x16, 0x1b),
    value: Color::from_rgb8(0xd5, 0xd9, 0xe0),
    text_selection: Color::from_rgb8(0x2a, 0x31, 0x40),
    gutter: Color::from_rgb8(0x8a, 0x91, 0x9c),
    gutter_current: Color::from_rgb8(0xd5, 0xd9, 0xe0),
    current_line: Color::from_rgb8(0x19, 0x1d, 0x25),
    code_block: Color::from_rgb8(0x18, 0x1b, 0x21),
    caret: Color::from_rgb8(0xf0, 0xb4, 0x5a),
    strike: Color::from_rgb8(0x8a, 0x91, 0x9c),
};

pub const DARK: Palette = Palette {
    text: Color::from_rgb8(0xe6, 0xe8, 0xec),
    muted: Color::from_rgb8(0xa3, 0xaa, 0xb5),
    faint: Color::from_rgb8(0x8a, 0x91, 0x9c),
    accent: Color::from_rgb8(0xf0, 0xb4, 0x5a),
    heading: Color::from_rgb8(0xf0, 0xb4, 0x5b),
    heading2: Color::from_rgb8(0xe8, 0xa8, 0x50),
    link: Color::from_rgb8(0x7f, 0xb0, 0xe6),
    code: Color::from_rgb8(0x7f, 0xd1, 0xa0),
    quote: Color::from_rgb8(0xa3, 0xaa, 0xb6),
    completed: Color::from_rgb8(0x9a, 0xa1, 0xac),
};

pub const SETTINGS: Settings = Settings {
    palette: DARK,
    font: FONT,
    front_matter_end: None,
};

pub const LIGHT: Palette = Palette {
    text: Color::from_rgb8(0x20, 0x24, 0x2d),
    muted: Color::from_rgb8(0x58, 0x61, 0x70),
    faint: Color::from_rgb8(0x60, 0x6a, 0x78),
    accent: Color::from_rgb8(0x9a, 0x58, 0x00),
    heading: Color::from_rgb8(0xa1, 0x48, 0x08),
    heading2: Color::from_rgb8(0x8a, 0x40, 0x08),
    link: Color::from_rgb8(0x1f, 0x5f, 0xbf),
    code: Color::from_rgb8(0x19, 0x73, 0x42),
    quote: Color::from_rgb8(0x58, 0x61, 0x71),
    completed: Color::from_rgb8(0x50, 0x5a, 0x68),
};

/// Registers every face of [`FONT`] so layout tests measure real glyphs
/// rather than a system fallback; headings are bold.
pub fn load_font() {
    static LOADED: Once = Once::new();
    LOADED.call_once(|| {
        let mut fonts = font_system().write().expect("font system");
        for bytes in [
            include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Regular.ttf")
                .as_slice(),
            include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Bold.ttf"),
            include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Italic.ttf"),
            include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-BoldItalic.ttf"),
        ] {
            fonts.load_font(Cow::Borrowed(bytes));
        }
    });
}
