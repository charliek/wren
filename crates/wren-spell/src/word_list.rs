//! The user's own words, from `words.txt`.

use std::collections::HashSet;

use crate::words::{self, MAX_CHARS};
use crate::WordSource;

/// The words the user lists in `words.txt`, one per line.
///
/// An all-lower-case entry is accepted in lower case, capitalised and in
/// upper case; an entry with any capital is accepted exactly as written.
#[derive(Debug, Default)]
pub struct WordList {
    /// Every accepted spelling, with straight apostrophes.
    accepted: HashSet<String>,
}

impl WordList {
    /// The list in `text`, and a warning for each line that is not one word.
    ///
    /// A leading byte-order mark, blank lines and lines starting with `#` are
    /// ignored, each line is trimmed, and `’` is read as `'`. A warning names
    /// its line, counted from 1.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut list = Self::default();
        let mut warnings = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            let entry = line.trim();
            if entry.is_empty() || entry.starts_with('#') {
                continue;
            }
            if entry.contains(char::is_whitespace) {
                warnings.push(format!("line {number}: not one word"));
                continue;
            }
            if entry.chars().count() > MAX_CHARS {
                warnings.push(format!("line {number}: longer than {MAX_CHARS} characters"));
                continue;
            }
            list.insert(&words::straight(entry));
        }
        (list, warnings)
    }

    fn insert(&mut self, entry: &str) {
        if entry.chars().all(|c| c.to_lowercase().eq([c])) {
            self.accepted.insert(capitalised(entry));
            self.accepted.insert(entry.to_uppercase());
        }
        self.accepted.insert(entry.to_owned());
    }
}

/// `entry` with its first character in upper case.
fn capitalised(entry: &str) -> String {
    let mut chars = entry.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

impl WordSource for WordList {
    fn accepts(&self, word: &str) -> bool {
        self.accepted.contains(words::straight(word).as_ref())
    }

    /// A word list offers no corrections of its own.
    fn suggest(&self, _word: &str, _out: &mut Vec<String>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> WordList {
        let (list, warnings) = WordList::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        list
    }

    #[test]
    fn rule_1_one_word_per_line() {
        let list = parsed("catppuccin\nzensical\n");
        assert!(list.accepts("catppuccin"));
        assert!(list.accepts("zensical"));
        assert!(!list.accepts("catppuccin zensical"));
        assert!(!list.accepts("wren"));
        assert!(!list.accepts(""));
    }

    #[test]
    fn rule_1_a_leading_bom_blank_lines_and_comments_are_ignored() {
        let (list, warnings) =
            WordList::parse("\u{feff}# my words\n\n   \n  # indented comment\ncatppuccin\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(list.accepts("catppuccin"));
        assert!(!list.accepts("#"));
        assert!(!list.accepts("my"));
        assert!(!list.accepts("\u{feff}"));

        let list = parsed("\u{feff}catppuccin");
        assert!(
            list.accepts("catppuccin"),
            "the mark is not part of the first word"
        );
        assert!(!list.accepts("\u{feff}catppuccin"));
    }

    #[test]
    fn rule_1_lines_are_trimmed_and_crlf_is_one_break() {
        let list = parsed("  catppuccin\t\r\nzensical\r\n\r\nwren\r");
        assert!(list.accepts("catppuccin"));
        assert!(list.accepts("zensical"));
        assert!(list.accepts("wren"));
        assert!(!list.accepts("wren\r"));
    }

    #[test]
    fn rule_1_a_curly_apostrophe_is_read_as_straight() {
        let list = parsed("y’all\n");
        assert!(list.accepts("y'all"));
        assert!(
            list.accepts("y’all"),
            "a word is read with straight ones too"
        );
        assert!(list.accepts("Y'all"));
        let list = parsed("o'neill\n");
        assert!(list.accepts("o’neill"));
    }

    #[test]
    fn rule_2_a_lower_case_entry_is_also_capitalised_and_upper_case() {
        let list = parsed("catppuccin\nłódź\n");
        for word in [
            "catppuccin",
            "Catppuccin",
            "CATPPUCCIN",
            "łódź",
            "Łódź",
            "ŁÓDŹ",
        ] {
            assert!(list.accepts(word), "{word:?}");
        }
        for word in ["CatPpuccin", "catppuccinS", "cATPPUCCIN", "CAtppuccin"] {
            assert!(!list.accepts(word), "{word:?}");
        }
    }

    #[test]
    fn rule_2_an_entry_with_a_capital_is_exact() {
        let list = parsed("CommonMark\nWren\nǅemal\n");
        assert!(list.accepts("CommonMark"));
        assert!(list.accepts("Wren"));
        assert!(list.accepts("ǅemal"));
        for word in [
            "commonmark",
            "Commonmark",
            "COMMONMARK",
            "wren",
            "WREN",
            "ǆemal",
            "ǄEMAL",
        ] {
            assert!(!list.accepts(word), "{word:?}");
        }
    }

    #[test]
    fn rule_2_entries_that_are_not_letters_keep_their_rule() {
        let list = parsed("don't\nx86\n'tis\n");
        assert!(list.accepts("DON'T"));
        assert!(list.accepts("Don’t"));
        assert!(list.accepts("X86"));
        assert!(list.accepts("'tis"));
        assert!(
            !list.accepts("'Tis"),
            "the first character, not the first letter"
        );
    }

    #[test]
    fn rule_3_a_line_that_is_not_one_word_is_skipped_with_a_warning() {
        let forty = "a".repeat(40);
        let forty_one = "b".repeat(41);
        let text = format!(
            "# header\nok\ntwo words\n{forty}\n{forty_one}\nnon\u{a0}break\ttabbed in\n\r\nlast\n"
        );
        let (list, warnings) = WordList::parse(&text);
        assert_eq!(
            warnings,
            [
                "line 3: not one word",
                "line 5: longer than 40 characters",
                "line 6: not one word",
            ]
        );
        assert!(list.accepts("ok"));
        assert!(list.accepts(&forty));
        assert!(list.accepts("last"));
        for word in [
            "two",
            "words",
            "two words",
            forty_one.as_str(),
            "non",
            "break",
        ] {
            assert!(!list.accepts(word), "{word:?}");
        }
    }

    #[test]
    fn rule_3_characters_are_counted_not_bytes() {
        let forty = "é".repeat(40);
        let list = parsed(&forty);
        assert!(list.accepts(&forty));
        let (_, warnings) = WordList::parse(&"é".repeat(41));
        assert_eq!(warnings, ["line 1: longer than 40 characters"]);
        let (_, warnings) = WordList::parse(&format!("\u{feff}{}", "’".repeat(41)));
        assert_eq!(
            warnings,
            ["line 1: longer than 40 characters"],
            "the mark does not count; the line is still line 1"
        );
    }

    #[test]
    fn rule_5_it_offers_no_suggestions() {
        let list = parsed("catppuccin\n");
        let mut out = vec!["kept".to_string()];
        list.suggest("catppucin", &mut out);
        assert_eq!(out, ["kept"]);
    }

    #[test]
    fn an_empty_or_odd_file_is_an_empty_list() {
        for text in ["", "\u{feff}", "\n\n\n", "\r\n", "#", "\u{feff}#\r\n"] {
            let (list, warnings) = WordList::parse(text);
            assert!(warnings.is_empty(), "{text:?}: {warnings:?}");
            assert!(list.accepted.is_empty(), "{text:?}");
        }
        let (list, warnings) = WordList::parse("😀\n\0\nשלום\n\u{301}\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(list.accepts("😀"));
        assert!(list.accepts("שלום"));
    }
}
