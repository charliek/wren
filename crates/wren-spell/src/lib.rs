//! Words and dictionaries for Wren's spell check: which words of a line are
//! misspelled and what to offer in their place. The bundled dictionary is
//! SCOWL's Hunspell `en_US` (`dict/`, see `dict/SOURCE.md`).
//!
//! A [`Speller`] asks its [`WordSource`]s in order. Nothing here names a
//! language or a file: a second dictionary or a system checker is another
//! source.

mod hunspell;
mod speller;
mod word_list;
mod words;

use std::fmt;

pub use hunspell::Hunspell;
pub use speller::Speller;
pub use word_list::WordList;

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
