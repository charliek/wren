//! What the crate's tests share: the label font, loaded from its bundled
//! bytes; the label corpus; the acceptance script's labels; the sample
//! boards; and a palette.

use std::borrow::Cow;
use std::sync::Once;

use iced::advanced::graphics::text::font_system;
use iced::Font;
use wren_board::{Palette, Rgba, Shape};

/// The label font, as the app names it.
pub const FONT: Font = Font::with_name("Inter");

/// Registers Inter Regular, the face every ring-0 label is drawn in, so
/// tests measure its glyphs and not a system fallback.
pub fn load_fonts() {
    static LOADED: Once = Once::new();
    LOADED.call_once(|| {
        font_system()
            .write()
            .expect("font system")
            .load_font(Cow::Borrowed(include_bytes!(
                "../../../third_party/inter/Inter-Regular.ttf"
            )));
    });
}

/// Twenty-four plausible flowchart labels, chosen in the plan's research
/// because each would read differently as Markdown: punctuation, digits,
/// leading white space, a line break and a literal backslash.
pub const LABELS: [&str; 24] = [
    "Validate request",
    "count > 0?",
    "Vec<String> cache",
    "List<Item> -> Map<K, V>",
    "<T> generic handler",
    "*args, **kwargs",
    "2 * rate * hours",
    "snake_case_name",
    "__init__ runs",
    "# of retries",
    "1. Parse",
    "- none -",
    "+ add item",
    "C# service",
    "price: $5 * qty",
    "a_b and c_d",
    "use `cfg` flag",
    "[draft] review",
    "see [1]",
    "http://example.com/x_y_z",
    "100% done\\n",
    "~~old~~ path",
    "line one\nline two",
    "    indented step",
];

/// The fifteen labels of the plan's acceptance script (§5.20), each with
/// the shape the script gives it.
pub const SCRIPT: [(&str, Shape); 15] = [
    ("Start", Shape::Pill),
    ("Receive order", Shape::Rect),
    ("Valid?", Shape::Diamond),
    ("Reject order", Shape::Rect),
    ("Check stock", Shape::Rect),
    ("In stock?", Shape::Diamond),
    ("Backorder", Shape::Rect),
    ("Notify customer", Shape::Rect),
    ("Charge card", Shape::Rect),
    ("Payment ok?", Shape::Diamond),
    ("Retry payment", Shape::Rect),
    ("Pack", Shape::Rect),
    ("Ship", Shape::Rect),
    ("Send receipt", Shape::Rect),
    ("End", Shape::Pill),
];

macro_rules! sample {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/wrentest/fixtures/boards/workspace/Boards/Samples/",
            $name,
            ".wren-board"
        ))
    };
}

/// Boards of the sample workspace the harness and the scenes open.
pub const CHECKOUT_FLOW: &str = sample!("Checkout flow");
pub const SHAPES: &str = sample!("Shapes");
pub const OVERLAP: &str = sample!("Overlap");
pub const FUTURE: &str = sample!("Future");

pub fn palette() -> Palette {
    Palette {
        canvas: Rgba::rgb(20, 20, 24),
        ink: Rgba::rgb(230, 230, 230),
        muted: Rgba::rgb(120, 120, 130),
        raised: Rgba::rgb(40, 40, 48),
        accent: Rgba::rgb(90, 160, 250),
        issue: Rgba::rgb(240, 80, 80),
        hues: [
            Rgba::rgb(230, 90, 90),
            Rgba::rgb(230, 150, 80),
            Rgba::rgb(220, 200, 90),
            Rgba::rgb(110, 200, 120),
            Rgba::rgb(90, 200, 210),
            Rgba::rgb(170, 130, 230),
            Rgba::rgb(100, 150, 240),
        ],
        dark: true,
    }
}
