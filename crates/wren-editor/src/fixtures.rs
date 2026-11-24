//! Colours and a font for the crate's own tests.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once};

use iced::advanced::graphics::text::font_system;
use iced::{Color, Font};

use unicode_segmentation::UnicodeSegmentation;

use crate::highlighter::Settings;
use crate::{MenuStyle, Palette, Proofreader, Style};

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
    misspelling: Color::from_rgb8(0xe0, 0x6c, 0x6c),
    menu: MenuStyle {
        background: Color::from_rgb8(0x1d, 0x21, 0x28),
        border: Color::from_rgb8(0x33, 0x3a, 0x46),
        selected: Color::from_rgb8(0x2a, 0x31, 0x40),
        text: Color::from_rgb8(0xe6, 0xe8, 0xec),
        muted: Color::from_rgb8(0xa3, 0xaa, 0xb5),
        font: FONT,
        text_size: 13.0,
    },
};

/// The plan's acceptance note (§5.5 of `discovery/spell-check.md`).
pub const SPELL_NOTE: &str = "---\ntitle: Spel chek\n---\n# Spell chek plan\n\nMarks apear under prose as you type, and a\nright-click on one offers replacments.\n\n- Engine: `spelbook` with a bundeled dictionary\n- See [the desing note](https://example.com/spel) or <https://example.com/chek>\n- README, README's, PRs and v0.1.1 stay unmarked; CommonMark does not\n- Catppuccin and Catppuccin’s are in the word list; mail me@exmaple.com\n- and/or, #tagg, snake_case_nmae, un*believ*able and <kbd>Ctrll</kbd> are skipped\n- [x] A done task with a mispelled word\n- It doesn’t mark don't, well-known or a b c; it marks well-knwon\n\n> A quote with a misteak.\n\n```rust\nlet chek = speller.check(wrd);\n```\n\n[ref-labl]: https://example.com \"Titel\"\n";

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
    tasks: None,
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

/// A proofreader that rejects a fixed set of words, offers fixed
/// corrections, and counts the times it is asked for marks.
pub struct Rejects {
    words: &'static [&'static str],
    corrections: &'static [(&'static str, &'static [&'static str])],
    calls: AtomicUsize,
    /// The `(line, mark, limit)` of each request for suggestions.
    asked: Mutex<Vec<(String, Range<usize>, usize)>>,
}

impl Rejects {
    pub fn new(words: &'static [&'static str]) -> Self {
        Self {
            words,
            corrections: &[],
            calls: AtomicUsize::new(0),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Offer these corrections for each word, best first.
    pub fn suggesting(
        mut self,
        corrections: &'static [(&'static str, &'static [&'static str])],
    ) -> Self {
        self.corrections = corrections;
        self
    }

    /// How many times [`Proofreader::marks`] has been called.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    /// The `(line, mark, limit)` of each request for suggestions so far.
    pub fn asked(&self) -> Vec<(String, Range<usize>, usize)> {
        self.asked.lock().expect("asked").clone()
    }
}

impl Proofreader for Rejects {
    fn marks(&self, line: &str, prose: &[Range<usize>], out: &mut Vec<Range<usize>>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        for range in prose {
            for (at, word) in line[range.clone()].unicode_word_indices() {
                if self.words.contains(&word) {
                    let start = range.start + at;
                    out.push(start..start + word.len());
                }
            }
        }
    }

    fn suggestions(&self, line: &str, mark: Range<usize>, limit: usize) -> Vec<String> {
        self.asked
            .lock()
            .expect("asked")
            .push((line.to_owned(), mark.clone(), limit));
        let word = &line[mark];
        self.corrections
            .iter()
            .find(|(wrong, _)| *wrong == word)
            .map(|(_, right)| right.iter().map(|&right| right.to_owned()).collect())
            .unwrap_or_default()
    }
}
