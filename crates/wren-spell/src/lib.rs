//! Words and dictionaries for Wren's spell check: which words of a line are
//! misspelled and what to offer in their place. The bundled dictionary is
//! SCOWL's Hunspell `en_US` (`dict/`, see `dict/SOURCE.md`).
//!
//! A [`Speller`] asks its [`WordSource`]s in order. Nothing here names a
//! language or a file: a second dictionary or a system checker is another
//! source.

use std::fmt;
use std::ops::Range;

/// Why a source could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A Hunspell `.aff`/`.dic` pair that does not parse, with the reason.
    Dictionary(String),
    /// A word list that cannot be read, with the reason.
    WordList(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dictionary(reason) => write!(f, "the dictionary does not parse: {reason}"),
            Self::WordList(reason) => write!(f, "the word list cannot be read: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

/// One place words come from: a dictionary, the user's list, later a system checker.
pub trait WordSource: Send + Sync {
    /// Whether this source knows `word` as correctly spelled.
    fn accepts(&self, word: &str) -> bool;
    /// Corrections for `word`, best first, appended to `out`.
    fn suggest(&self, word: &str, out: &mut Vec<String>);
}

/// A Hunspell dictionary: one `.aff`/`.dic` pair, read by `spellbook`.
#[non_exhaustive]
pub struct Hunspell {}

impl Hunspell {
    /// The bundled dictionary, SCOWL's `en_US`.
    pub fn en_us() -> Result<Self, Error> {
        Ok(Self {})
    }

    /// Any Hunspell pair, from the text of its `.aff` and `.dic` files.
    pub fn from_text(_aff: &str, _dic: &str) -> Result<Self, Error> {
        Ok(Self {})
    }
}

impl WordSource for Hunspell {
    fn accepts(&self, _word: &str) -> bool {
        false
    }

    fn suggest(&self, _word: &str, _out: &mut Vec<String>) {}
}

/// The words the user lists in `words.txt`, one per line.
#[non_exhaustive]
pub struct WordList {}

impl WordList {
    /// The list in `text`, and a warning for each line that is not one word.
    pub fn parse(_text: &str) -> (Self, Vec<String>) {
        (Self {}, Vec::new())
    }
}

impl WordSource for WordList {
    fn accepts(&self, _word: &str) -> bool {
        false
    }

    /// A word list offers no corrections of its own.
    fn suggest(&self, _word: &str, _out: &mut Vec<String>) {}
}

/// Judges the words of a line against its sources, asked in order.
#[non_exhaustive]
pub struct Speller {}

impl Speller {
    pub fn new(_sources: Vec<Box<dyn WordSource>>) -> Self {
        Self {}
    }

    /// The misspelled words of `line` that start and end inside `prose`, as byte
    /// ranges of `line`: sorted, apart, on character boundaries. Appends to `out`.
    pub fn marks(&self, _line: &str, _prose: &[Range<usize>], _out: &mut Vec<Range<usize>>) {}

    /// Up to `limit` corrections for `word`, best first, without duplicates.
    pub fn suggestions(&self, _word: &str, _limit: usize) -> Vec<String> {
        Vec::new()
    }
}
