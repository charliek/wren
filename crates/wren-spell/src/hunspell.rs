//! A Hunspell `.aff`/`.dic` pair, read by `spellbook`.

use crate::{Error, WordSource};

const EN_US_AFF: &str = include_str!("../dict/en_US.aff");
const EN_US_DIC: &str = include_str!("../dict/en_US.dic");

/// A Hunspell dictionary: one `.aff`/`.dic` pair, read by `spellbook`.
///
/// It follows Hunspell's case rules: a lower-case entry is also accepted
/// capitalised and in upper case, and a mixed-case word is looked up as
/// written.
pub struct Hunspell {
    dictionary: spellbook::Dictionary,
}

impl Hunspell {
    /// The bundled dictionary, SCOWL's `en_US`.
    pub fn en_us() -> Result<Self, Error> {
        Self::from_text(EN_US_AFF, EN_US_DIC)
    }

    /// Any Hunspell pair, from the text of its `.aff` and `.dic` files.
    pub fn from_text(aff: &str, dic: &str) -> Result<Self, Error> {
        spellbook::Dictionary::new(aff, dic)
            .map(|dictionary| Self { dictionary })
            .map_err(|err| Error::Dictionary(err.to_string()))
    }
}

impl WordSource for Hunspell {
    fn accepts(&self, word: &str) -> bool {
        // `spellbook` counts the empty word as correct; no source here does.
        !word.is_empty() && self.dictionary.check(word)
    }

    fn suggest(&self, word: &str, out: &mut Vec<String>) {
        // `spellbook` clears the vector it fills; the trait appends.
        let mut found = Vec::new();
        self.dictionary.suggest(word, &mut found);
        out.append(&mut found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn en_us() -> &'static Hunspell {
        static DICTIONARY: OnceLock<Hunspell> = OnceLock::new();
        DICTIONARY.get_or_init(|| Hunspell::en_us().expect("the bundled dictionary parses"))
    }

    #[test]
    fn the_bundled_dictionary_knows_common_words() {
        let dict = en_us();
        for word in [
            "the",
            "hello",
            "world",
            "spelling",
            "dictionary",
            "suggestions",
            "underline",
            "Wednesday",
            "English",
            "receive",
            "well",
            "known",
            "doesn't",
            "can't",
            "don't",
            "it's",
        ] {
            assert!(dict.accepts(word), "{word:?} should be known");
        }
    }

    #[test]
    fn it_follows_hunspells_case_rules() {
        let dict = en_us();
        assert!(dict.accepts("hello"));
        assert!(dict.accepts("Hello"));
        assert!(dict.accepts("HELLO"));
        assert!(dict.accepts("Wednesday"));
        assert!(
            !dict.accepts("wednesday"),
            "a capitalised entry needs its capital"
        );
        assert!(!dict.accepts("hELLO"));
        assert!(
            !dict.accepts("CommonMark"),
            "a mixed-case word is looked up as written (decision 5)"
        );
    }

    #[test]
    fn it_reads_a_curly_apostrophe_as_a_straight_one() {
        let dict = en_us();
        assert!(dict.accepts("doesn’t"));
        assert!(dict.accepts("can’t"));
    }

    #[test]
    fn it_rejects_the_acceptance_notes_misspellings() {
        let dict = en_us();
        for word in [
            "chek",
            "apear",
            "replacments",
            "bundeled",
            "desing",
            "CommonMark",
            "mispelled",
            "knwon",
            "misteak",
        ] {
            assert!(!dict.accepts(word), "{word:?} should be rejected");
        }
    }

    #[test]
    fn the_empty_word_is_not_accepted() {
        assert!(!en_us().accepts(""));
    }

    #[test]
    fn suggestions_are_appended_best_first() {
        let dict = en_us();
        let mut out = vec!["kept".to_string()];
        dict.suggest("recieve", &mut out);
        assert_eq!(out[0], "kept", "suggest must append, not clear");
        assert_eq!(out.get(1).map(String::as_str), Some("receive"));

        let mut out = Vec::new();
        dict.suggest("chek", &mut out);
        assert!(out.iter().any(|s| s == "check"), "{out:?}");
    }

    #[test]
    fn suggest_on_a_word_with_no_neighbours_adds_nothing() {
        let mut out = Vec::new();
        en_us().suggest("xqzvkpw", &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn any_pair_can_be_loaded_from_text() {
        let dict = Hunspell::from_text("SET UTF-8\n", "2\nwren\nnote\n").expect("a minimal pair");
        assert!(dict.accepts("wren"));
        assert!(dict.accepts("Wren"));
        assert!(!dict.accepts("hello"));
    }

    #[test]
    fn a_pair_that_does_not_parse_is_an_error() {
        let result = Hunspell::from_text("FLAG num\n", "1\nword/abc\n");
        match result {
            Err(Error::Dictionary(reason)) => assert!(!reason.is_empty()),
            Err(other) => panic!("wrong error: {other:?}"),
            Ok(_) => panic!("a numeric flag set that is not numeric should not parse"),
        }
    }

    #[test]
    fn odd_input_neither_panics_nor_is_accepted_wrongly() {
        let dict = en_us();
        let long = "a".repeat(10_000);
        for word in [
            "\u{301}\u{302}\u{303}",
            "😀😀😀",
            "שלום",
            "مرحبا",
            "\u{200f}abc",
            "a\u{0}b",
            "''''",
            long.as_str(),
        ] {
            let _ = dict.accepts(word);
            let mut out = Vec::new();
            dict.suggest(word, &mut out);
        }
        assert!(!dict.accepts(&long));
    }

    #[test]
    fn seeded_random_words_never_panic() {
        let dict = en_us();
        let alphabet: Vec<char> = "abcdeéz'’-. ZQ\u{301}😀שׁ\u{200f}ßİı".chars().collect();
        let mut rng = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            rng.wrapping_mul(0x2545_f491_4f6c_dd1d)
        };
        for _ in 0..5_000 {
            let len = (next() % 24) as usize;
            let word: String = (0..len)
                .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                .collect();
            let _ = dict.accepts(&word);
        }
    }
}
