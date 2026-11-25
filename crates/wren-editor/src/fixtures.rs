//! Colours and a font for the crate's own tests.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once};

use iced::advanced::graphics::text::font_system;
use iced::{Color, Font};

use unicode_segmentation::UnicodeSegmentation;

use wren_markdown::span::SpanKind;

use crate::highlighter::Settings;
use crate::{
    Accelerator, Clicked, Hit, MenuProvider, MenuSource, MenuStyle, Palette, Proofreader,
    Requested, Row, Style, SuggestSource, Suggestion,
};

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
    date: Color::from_rgb8(0x72, 0xd6, 0xe3),
    overdue: Color::from_rgb8(0xf0, 0x88, 0x88),
    owner: Color::from_rgb8(0x7e, 0xb1, 0xe7),
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
    date: Color::from_rgb8(0x04, 0x77, 0x84),
    overdue: Color::from_rgb8(0xb3, 0x2d, 0x36),
    owner: Color::from_rgb8(0x04, 0x4e, 0xa4),
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

/// A source of suggestions that completes the word before the caret from a
/// fixed list, once two of its characters are typed, and keeps what it was
/// asked.
pub struct Completes {
    words: &'static [&'static str],
    /// The `(line, caret)` of each request.
    asked: Mutex<Vec<(String, usize)>>,
}

impl Completes {
    pub fn new(words: &'static [&'static str]) -> Self {
        Self {
            words,
            asked: Mutex::new(Vec::new()),
        }
    }

    /// The `(line, caret)` of each request so far.
    pub fn asked(&self) -> Vec<(String, usize)> {
        self.asked.lock().expect("asked").clone()
    }
}

impl SuggestSource for Completes {
    /// The words that start with the letters before the caret, each named
    /// by its place in the list, under those letters.
    fn suggest(&self, line: &str, caret: usize) -> Option<Suggestion> {
        self.asked
            .lock()
            .expect("asked")
            .push((line.to_owned(), caret));
        let before = line.get(..caret)?;
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric())
            .last()
            .map_or(caret, |(at, _)| at);
        let typed = &before[start..];
        let rows: Vec<Row> = (0..)
            .zip(self.words)
            .filter(|(_, word)| typed.chars().count() >= 2 && word.starts_with(typed))
            .map(|(id, word)| Row::new(id, *word))
            .collect();
        (!rows.is_empty()).then_some(Suggestion {
            range: start..caret,
            rows,
        })
    }
}

/// Suggestions of rows that run commands: after two letters at a word's
/// start, a row running command 7 and one running command 8, under the
/// letters typed.
pub struct CommandSuggestions;

impl SuggestSource for CommandSuggestions {
    fn suggest(&self, line: &str, caret: usize) -> Option<Suggestion> {
        let before = line.get(..caret)?;
        let start = before.rfind(' ').map_or(0, |at| at + 1);
        (caret - start >= 2).then(|| Suggestion {
            range: start..caret,
            rows: vec![Row::command(7, "seven"), Row::command(8, "eight")],
        })
    }
}

/// The request [`Menus`] answers with the status menu.
pub const STATUS: u32 = 1;
/// The request [`Menus`] answers by taking the keyboard.
pub const KEYBOARD: u32 = 3;

/// The status menu: three rows, numbered from 20, each with its key.
pub fn status_rows() -> Vec<Row> {
    vec![
        Row::new(20, "Open").accelerator(Accelerator::plain(' ')),
        Row::new(21, "Doing").accelerator(Accelerator::plain('/')),
        Row::new(22, "Done").accelerator(Accelerator::plain('x')),
    ]
}

/// A source of menus: [`STATUS`] opens the status menu on the box of a
/// line that starts a task, [`KEYBOARD`] has the host take the keyboard,
/// and any other request opens nothing. It keeps what it was asked.
#[derive(Default)]
pub struct Menus {
    /// The `(request, line, caret)` of each request.
    asked: Mutex<Vec<(u32, String, usize)>>,
}

impl Menus {
    /// The `(request, line, caret)` of each request so far.
    pub fn asked(&self) -> Vec<(u32, String, usize)> {
        self.asked.lock().expect("asked").clone()
    }
}

impl MenuSource for Menus {
    fn menu(&self, id: u32, line: &str, caret: usize) -> Option<Requested> {
        self.asked
            .lock()
            .expect("asked")
            .push((id, line.to_owned(), caret));
        match id {
            STATUS if line.starts_with("- [") => Some(Requested::Rows {
                range: 2..5,
                rows: status_rows(),
            }),
            KEYBOARD => Some(Requested::Keyboard),
            _ => None,
        }
    }
}

/// The menu [`on_tokens`] opens: two rows, numbered from 30, the first
/// with a key.
pub fn token_rows() -> Vec<Row> {
    vec![
        Row::new(30, "Rename").accelerator(Accelerator::plain('r')),
        Row::new(31, "Remove"),
    ]
}

/// A provider's answer that takes a task's owner or tag, when that is the
/// hit: [`token_rows`] under it. A misspelled owner is a misspelling, which
/// it passes on.
pub fn on_tokens(clicked: &Clicked) -> Option<Requested> {
    let Some(Hit::Span(span)) = &clicked.hit else {
        return None;
    };
    matches!(span.kind, SpanKind::Owner { .. } | SpanKind::Tag).then(|| Requested::Rows {
        range: span.range.clone(),
        rows: token_rows(),
    })
}

/// A provider's answer that takes any place on a line that starts a task,
/// whatever is there: a token, a misspelled word, a place in no span, or
/// the row past the end of its text. The status menu under the box.
pub fn on_task_lines(clicked: &Clicked) -> Option<Requested> {
    clicked.text.starts_with("- [").then(|| Requested::Rows {
        range: 2..5,
        rows: status_rows(),
    })
}

/// A provider's answer that takes a task's date by having the host take
/// the keyboard, as it would for a calendar.
pub fn on_dates(clicked: &Clicked) -> Option<Requested> {
    let Some(Hit::Span(span)) = &clicked.hit else {
        return None;
    };
    matches!(span.kind, SpanKind::Date { .. }).then_some(Requested::Keyboard)
}

/// A provider's answer that takes any place on a line of text that starts
/// no task, by having the host take the keyboard.
pub fn off_task_lines(clicked: &Clicked) -> Option<Requested> {
    (!clicked.text.is_empty() && !clicked.text.starts_with("- [")).then_some(Requested::Keyboard)
}

/// A provider's answer on a line that starts a task: a menu under the
/// box's dash of a row of its own (11) and a row that runs the host's
/// command 7, as a task menu's rows do.
pub fn commands_on_task_lines(clicked: &Clicked) -> Option<Requested> {
    clicked.text.starts_with("- [").then(|| Requested::Rows {
        range: 0..1,
        rows: vec![Row::new(11, "Own"), Row::command(7, "Command")],
    })
}

/// A provider of menus for a right-click that answers as `answer` does,
/// and keeps what it was asked about.
pub struct Provides {
    answer: fn(&Clicked) -> Option<Requested>,
    pub asked: Mutex<Vec<Clicked>>,
}

impl Provides {
    pub fn new(answer: fn(&Clicked) -> Option<Requested>) -> Self {
        Self {
            answer,
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Each place it was asked about so far.
    pub fn asked(&self) -> Vec<Clicked> {
        self.asked.lock().expect("asked").clone()
    }
}

impl MenuProvider for Provides {
    fn menu(&self, clicked: &Clicked) -> Option<Requested> {
        self.asked.lock().expect("asked").push(clicked.clone());
        (self.answer)(clicked)
    }
}
