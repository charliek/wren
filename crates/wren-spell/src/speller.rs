//! Judging the words of a line against the sources.

use std::ops::Range;

use crate::words;
use crate::WordSource;

/// Judges the words of a line against its sources, asked in order.
pub struct Speller {
    sources: Vec<Box<dyn WordSource>>,
}

impl Speller {
    pub fn new(sources: Vec<Box<dyn WordSource>>) -> Self {
        Self { sources }
    }

    /// The misspelled words of `line` that start and end inside `prose`, as byte
    /// ranges of `line`: sorted, apart, on character boundaries. Appends to `out`.
    ///
    /// `prose` should be sorted and apart. A range is clamped to the line and
    /// shrunk to character boundaries, and a range that starts before the end
    /// of an earlier one keeps only its part after it.
    pub fn marks(&self, line: &str, prose: &[Range<usize>], out: &mut Vec<Range<usize>>) {
        let mut floor = 0;
        for range in prose {
            let start = boundary_at_or_after(line, range.start.max(floor).min(line.len()));
            let end = boundary_at_or_before(line, range.end.min(line.len()));
            if start >= end {
                continue;
            }
            floor = end;
            words::checked_words(line, start..end, |word| {
                if !self.correct(&line[word.clone()]) {
                    out.push(word);
                }
            });
        }
    }

    /// Up to `limit` corrections for `word`, best first, without duplicates.
    ///
    /// The sources are asked in order and their corrections kept in that
    /// order. `word` and its corrections are read with straight apostrophes.
    /// A correction that is empty, holds a control character such as a line
    /// break, or is `word` itself is left out. A word that is never checked
    /// (§4.2 rule 4) gets none, which also bounds what a source is asked.
    pub fn suggestions(&self, word: &str, limit: usize) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        if limit == 0 || !words::checkable(word) {
            return found;
        }
        let word = words::straight(word);
        let mut offered = Vec::new();
        for source in &self.sources {
            source.suggest(&word, &mut offered);
            for correction in offered.drain(..) {
                let correction = if correction.contains('’') {
                    correction.replace('’', "'")
                } else {
                    correction
                };
                if correction.is_empty()
                    || correction.contains(char::is_control)
                    || correction == *word
                    || found.contains(&correction)
                {
                    continue;
                }
                found.push(correction);
                if found.len() == limit {
                    return found;
                }
            }
        }
        found
    }

    /// Whether some source accepts `word`, read with straight apostrophes, or
    /// accepts the part before a final `'s`.
    fn correct(&self, word: &str) -> bool {
        let word = words::straight(word);
        self.accepted(&word)
            || word
                .strip_suffix("'s")
                .is_some_and(|base| !base.is_empty() && self.accepted(base))
    }

    fn accepted(&self, word: &str) -> bool {
        self.sources.iter().any(|source| source.accepts(word))
    }
}

fn boundary_at_or_after(line: &str, mut at: usize) -> usize {
    while !line.is_char_boundary(at) {
        at += 1;
    }
    at
}

fn boundary_at_or_before(line: &str, mut at: usize) -> usize {
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // an expected list of marks often holds one
mod tests {
    use super::*;
    use crate::{Hunspell, WordList};
    use std::sync::OnceLock;

    fn list(text: &str) -> Box<WordList> {
        let (list, warnings) = WordList::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        Box::new(list)
    }

    struct Shared(&'static Hunspell);

    impl WordSource for Shared {
        fn accepts(&self, word: &str) -> bool {
            self.0.accepts(word)
        }

        fn suggest(&self, word: &str, out: &mut Vec<String>) {
            self.0.suggest(word, out);
        }
    }

    fn en_us() -> &'static Hunspell {
        static DICTIONARY: OnceLock<Hunspell> = OnceLock::new();
        DICTIONARY.get_or_init(|| Hunspell::en_us().expect("the bundled dictionary parses"))
    }

    /// The bundled dictionary and a word list holding `catppuccin`, as the
    /// acceptance note is checked.
    fn speller() -> Speller {
        Speller::new(vec![Box::new(Shared(en_us())), list("catppuccin\n")])
    }

    fn marked<'a>(speller: &Speller, line: &'a str, prose: &[Range<usize>]) -> Vec<&'a str> {
        let mut out = Vec::new();
        speller.marks(line, prose, &mut out);
        out.into_iter().map(|r| &line[r]).collect()
    }

    fn marked_line<'a>(speller: &Speller, line: &'a str) -> Vec<&'a str> {
        marked(speller, line, &[0..line.len()])
    }

    /// The prose ranges of `line` named by their text, found left to right.
    fn ranges(line: &str, parts: &[&str]) -> Vec<Range<usize>> {
        let mut from = 0;
        parts
            .iter()
            .map(|part| {
                let start = from + line[from..].find(part).expect("part in line");
                from = start + part.len();
                start..from
            })
            .collect()
    }

    /// The acceptance note (plan §5.5), each line with the prose ranges the
    /// editor gives it (§4.1), named by their text.
    const ACCEPTANCE: &[(&str, &[&str])] = &[
        ("---", &[]),
        ("title: Spel chek", &[]),
        ("---", &[]),
        ("# Spell chek plan", &["Spell chek plan"]),
        ("", &[]),
        (
            "Marks apear under prose as you type, and a",
            &["Marks apear under prose as you type, and a"],
        ),
        (
            "right-click on one offers replacments.",
            &["right-click on one offers replacments."],
        ),
        ("", &[]),
        (
            "- Engine: `spelbook` with a bundeled dictionary",
            &["Engine: ", " with a bundeled dictionary"],
        ),
        (
            "- See [the desing note](https://example.com/spel) or <https://example.com/chek>",
            &["See ", "the desing note", " or "],
        ),
        (
            "- README, README's, PRs and v0.1.1 stay unmarked; CommonMark does not",
            &["README, README's, PRs and v0.1.1 stay unmarked; CommonMark does not"],
        ),
        (
            "- Catppuccin and Catppuccin’s are in the word list; mail me@exmaple.com",
            &["Catppuccin and Catppuccin’s are in the word list; mail me@exmaple.com"],
        ),
        (
            "- and/or, #tagg, snake_case_nmae, un*believ*able and <kbd>Ctrll</kbd> are skipped",
            &["and/or, #tagg, snake_case_nmae, un*believ*able and <kbd>Ctrll</kbd> are skipped"],
        ),
        (
            "- [x] A done task with a mispelled word",
            &["A done task with a mispelled word"],
        ),
        (
            "- It doesn’t mark don't, well-known or a b c; it marks well-knwon",
            &["It doesn’t mark don't, well-known or a b c; it marks well-knwon"],
        ),
        ("", &[]),
        ("> A quote with a misteak.", &["A quote with a misteak."]),
        ("", &[]),
        ("```rust", &[]),
        ("let chek = speller.check(wrd);", &[]),
        ("```", &[]),
        ("", &[]),
        ("[ref-labl]: https://example.com \"Titel\"", &[]),
    ];

    #[test]
    fn the_acceptance_note_has_exactly_its_nine_marks() {
        let speller = speller();
        let mut found = Vec::new();
        for (line, parts) in ACCEPTANCE {
            found.extend(marked(&speller, line, &ranges(line, parts)));
        }
        assert_eq!(
            found,
            [
                "chek",
                "apear",
                "replacments",
                "bundeled",
                "desing",
                "CommonMark",
                "mispelled",
                "knwon",
                "misteak"
            ]
        );
    }

    #[test]
    fn the_acceptance_notes_skipped_lines_hold_misspellings() {
        let speller = speller();
        for line in [
            "title: Spel chek",
            "let chek = speller.check(wrd);",
            "[ref-labl]: https://example.com \"Titel\"",
        ] {
            assert!(
                !marked_line(&speller, line).is_empty(),
                "{line:?}: the note relies on §4.1 to leave it out"
            );
        }
    }

    #[test]
    fn rule_1_skipped_chunks_are_never_marked() {
        let speller = speller();
        for line in [
            "me@exmaple.com",
            "adn/or",
            r"C:\Usres\wrne",
            "<kbd>Ctrll</kbd>",
            "<br>",
            "[^fotnote]",
            "#tagg",
            "snake_case_nmae",
            "un*believ*able",
            "strkie~~thru~~ogh",
        ] {
            assert_eq!(marked_line(&speller, line), Vec::<&str>::new(), "{line:?}");
        }
        assert_eq!(marked_line(&speller, "##heding"), ["heding"]);
    }

    #[test]
    fn rule_2_a_hyphenated_compound_is_checked_part_by_part() {
        let speller = speller();
        assert_eq!(marked_line(&speller, "well-known"), Vec::<&str>::new());
        assert_eq!(marked_line(&speller, "well-knwon"), ["knwon"]);
        assert_eq!(marked_line(&speller, "wel-known"), ["wel"]);
    }

    #[test]
    fn rule_3_a_curly_apostrophe_is_looked_up_as_straight() {
        let speller = Speller::new(vec![list("isn't")]);
        assert_eq!(marked_line(&speller, "isn’t isn't"), Vec::<&str>::new());
        assert_eq!(marked_line(&speller, "wasn’t"), ["wasn’t"]);
    }

    #[test]
    fn rule_4_words_outside_the_checked_set_are_never_marked() {
        let speller = Speller::new(Vec::new());
        assert_eq!(
            marked_line(
                &speller,
                "a I v0.1.1 x86 README PRs README's слово 日本 😀😀"
            ),
            Vec::<&str>::new(),
            "with no source every checked word would be marked"
        );
        let long = "q".repeat(41);
        assert_eq!(marked_line(&speller, &long), Vec::<&str>::new());
        assert_eq!(marked_line(&speller, "qq"), ["qq"]);
    }

    #[test]
    fn rule_5_any_source_accepts_and_case_follows_hunspell() {
        let speller = speller();
        assert_eq!(marked_line(&speller, "Hello hello"), Vec::<&str>::new());
        assert_eq!(
            marked_line(&speller, "Catppuccin catppuccin"),
            Vec::<&str>::new()
        );
        assert_eq!(
            marked_line(&speller, "CommonMark hEllo wednesday"),
            ["CommonMark", "hEllo", "wednesday"]
        );

        let either = Speller::new(vec![list("alpha"), list("beta")]);
        assert_eq!(marked_line(&either, "alpha beta gamma"), ["gamma"]);
    }

    #[test]
    fn rule_6_a_possessive_is_correct_when_its_base_is() {
        let speller = speller();
        assert_eq!(
            marked_line(&speller, "Catppuccin's Catppuccin’s catppuccin's"),
            Vec::<&str>::new()
        );
        assert_eq!(marked_line(&speller, "Wren's Wrne's"), ["Wrne's"]);
    }

    #[test]
    fn a_word_list_read_from_a_bom_crlf_file_vouches_for_its_words() {
        let (words, warnings) =
            WordList::parse("\u{feff}# my words\r\nzensical\r\no’kelly\r\nCommonMark\r\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        let speller = Speller::new(vec![Box::new(Shared(en_us())), Box::new(words)]);
        assert_eq!(
            marked_line(
                &speller,
                "Zensical zensical’s O'Kelly o'kelly O’kelly CommonMark’s Commonmark"
            ),
            ["O'Kelly", "Commonmark"],
            "a lower-case entry is not accepted with an inner capital; one with a capital is exact"
        );
    }

    /// A source that knows no word and offers the same corrections for any.
    struct Offers(&'static [&'static str]);

    impl WordSource for Offers {
        fn accepts(&self, _word: &str) -> bool {
            false
        }

        fn suggest(&self, _word: &str, out: &mut Vec<String>) {
            out.extend(self.0.iter().map(|s| s.to_string()));
        }
    }

    /// A source that must not be asked for corrections.
    struct Unasked;

    impl WordSource for Unasked {
        fn accepts(&self, _word: &str) -> bool {
            false
        }

        fn suggest(&self, word: &str, _out: &mut Vec<String>) {
            panic!("asked for corrections to {word:?}");
        }
    }

    #[test]
    fn suggestions_stop_at_the_limit() {
        let speller = Speller::new(vec![Box::new(Offers(&["one", "two", "three"]))]);
        assert_eq!(speller.suggestions("chek", 2), ["one", "two"]);
        assert_eq!(speller.suggestions("chek", 3), ["one", "two", "three"]);
        assert_eq!(speller.suggestions("chek", 9), ["one", "two", "three"]);

        let unasked = Speller::new(vec![Box::new(Unasked)]);
        assert!(unasked.suggestions("chek", 0).is_empty());
        let enough = Speller::new(vec![Box::new(Offers(&["one", "two"])), Box::new(Unasked)]);
        assert_eq!(
            enough.suggestions("chek", 2),
            ["one", "two"],
            "a later source is not asked once the limit is reached"
        );
    }

    #[test]
    fn suggestions_follow_the_sources_in_order() {
        let speller = Speller::new(vec![
            Box::new(Offers(&["b1", "b2"])),
            list("chek"),
            Box::new(Offers(&["a1", "a2"])),
        ]);
        assert_eq!(speller.suggestions("chke", 5), ["b1", "b2", "a1", "a2"]);
        assert_eq!(speller.suggestions("chke", 3), ["b1", "b2", "a1"]);
    }

    #[test]
    fn suggestions_hold_no_duplicates() {
        let speller = Speller::new(vec![
            Box::new(Offers(&["check", "cheek", "check", "don’t"])),
            Box::new(Offers(&["cheek", "Check", "don't", "chef"])),
        ]);
        assert_eq!(
            speller.suggestions("chek", 5),
            ["check", "cheek", "don't", "Check", "chef"],
            "the first of each is kept; `’` and `'` are one spelling"
        );
    }

    #[test]
    fn suggestions_leave_out_empty_multi_line_and_identical_corrections() {
        let speller = Speller::new(vec![Box::new(Offers(&[
            "",
            "two\nlines",
            "carriage\r",
            "nul\0",
            "tab\tbed",
            "chek",
            "chek’s",
            "a lot",
            "check",
        ]))]);
        assert_eq!(speller.suggestions("chek", 5), ["chek's", "a lot", "check"]);
        assert_eq!(
            speller.suggestions("chek’s", 5),
            ["chek", "a lot", "check"],
            "the word is compared with straight apostrophes"
        );
    }

    #[test]
    fn a_word_that_is_never_checked_gets_no_suggestions() {
        let speller = Speller::new(vec![Box::new(Unasked)]);
        let long = "q".repeat(41);
        for word in [
            "",
            "a",
            "README",
            "PRs",
            "v0.1",
            "слово",
            "😀😀",
            long.as_str(),
        ] {
            assert!(speller.suggestions(word, 5).is_empty(), "{word:?}");
        }
    }

    #[test]
    fn the_acceptance_notes_marks_have_these_first_suggestions() {
        let speller = speller();
        for (word, first) in [
            ("chek", "cheek"),
            ("apear", "pear"),
            ("replacments", "replacements"),
            ("bundeled", "bundled"),
            ("desing", "design"),
            ("CommonMark", "Common Mark"),
            ("mispelled", "misspelled"),
            ("knwon", "known"),
            ("misteak", "mistake"),
        ] {
            let found = speller.suggestions(word, 5);
            assert!(found.len() <= 5, "{word:?}: {found:?}");
            assert_eq!(
                found.first().map(String::as_str),
                Some(first),
                "{word:?}: {found:?}"
            );
        }
        assert!(speller.suggestions("chek", 5).iter().any(|s| s == "check"));
    }

    #[test]
    fn suggestions_for_a_curly_word_use_straight_apostrophes() {
        let speller = speller();
        let found = speller.suggestions("dosen’t", 5);
        assert_eq!(
            found.first().map(String::as_str),
            Some("doesn't"),
            "{found:?}"
        );
        assert!(found.iter().all(|s| !s.contains('’')), "{found:?}");
        assert_eq!(speller.suggestions("Wrne’s", 1), ["Wren's"]);
    }

    #[test]
    fn rule_7_a_mark_is_the_byte_range_of_the_word() {
        let speller = speller();
        let line = "Ünïcode chek’s, then misteak.";
        let mut out = Vec::new();
        speller.marks(line, &[0..line.len()], &mut out);
        let words: Vec<_> = out.iter().map(|r| &line[r.clone()]).collect();
        assert_eq!(words, ["Ünïcode", "chek’s", "misteak"]);
        assert_eq!(out[0], 0..9);
    }

    #[test]
    fn marks_are_appended() {
        let speller = speller();
        let mut out = vec![100..101];
        speller.marks("chek", &[0..4], &mut out);
        assert_eq!(out, [100..101, 0..4]);
    }

    #[test]
    fn only_words_inside_prose_are_marked() {
        let speller = speller();
        let line = "chek `chek` chek";
        assert_eq!(marked(&speller, line, &[0..5, 11..16]), ["chek", "chek"]);
        let mut out = Vec::new();
        speller.marks(line, &[0..5, 11..16], &mut out);
        assert_eq!(out, [0..4, 12..16]);
        assert_eq!(marked(&speller, line, &[]), Vec::<&str>::new());
    }

    #[test]
    fn a_prose_range_that_cuts_a_word_marks_no_piece_of_it() {
        let speller = speller();
        let line = "understanding";
        assert_eq!(marked(&speller, line, &[0..5]), Vec::<&str>::new());
        assert_eq!(marked(&speller, line, &[5..13]), Vec::<&str>::new());
        assert_eq!(marked(&speller, line, &[3..9]), Vec::<&str>::new());
    }

    #[test]
    fn bad_prose_ranges_are_clamped_and_ordered() {
        let speller = speller();
        let line = "é chek misteak";
        let mut out = Vec::new();
        speller.marks(line, &[1..400], &mut out);
        assert_eq!(
            out,
            [3..7, 8..15],
            "start inside `é` moves to its end; end clamps"
        );
        out.clear();
        speller.marks(line, &[8..15, 0..7], &mut out);
        assert_eq!(out, [8..15], "a range before an earlier one is dropped");
        out.clear();
        speller.marks(line, &[0..10, 5..15], &mut out);
        assert_eq!(
            out,
            [3..7],
            "an overlap keeps only the part after the earlier range"
        );
        out.clear();
        let backwards = Range { start: 7, end: 3 };
        speller.marks(line, &[backwards, 15..15, 20..30], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn odd_lines_are_handled() {
        let speller = speller();
        for line in [
            "\u{301}\u{301}\u{301}\u{301}",
            "😀😀😀 🧑‍🧑‍🧒 🇺🇸🇺🇸",
            "שלום עולם chek مرحبا",
            "\u{202e}chek\u{202c} \u{200f}misteak",
            "\0chek\0",
            "",
        ] {
            let mut out = Vec::new();
            speller.marks(line, &[0..line.len()], &mut out);
            assert_valid(line, &[0..line.len()], &out);
        }
        assert_eq!(
            marked_line(&speller, "שלום עולם chek مرحبا"),
            ["chek"],
            "RTL words are skipped; Latin ones among them are checked"
        );
    }

    #[test]
    fn a_ten_thousand_character_line() {
        let speller = speller();
        let line = "the chek misteak ".repeat(600);
        assert!(line.chars().count() >= 10_000);
        let mut out = Vec::new();
        speller.marks(&line, &[0..line.len()], &mut out);
        assert_eq!(out.len(), 1_200);
        assert_valid(&line, &[0..line.len()], &out);

        let word = "a".repeat(10_000);
        out.clear();
        speller.marks(&word, &[0..word.len()], &mut out);
        assert!(out.is_empty(), "longer than 40 characters");
    }

    /// The contract of `marks`: in bounds, on character boundaries, sorted,
    /// apart, and each inside one of the (clamped) prose ranges.
    fn assert_valid(line: &str, prose: &[Range<usize>], marks: &[Range<usize>]) {
        for (i, mark) in marks.iter().enumerate() {
            assert!(
                mark.start < mark.end && mark.end <= line.len(),
                "{line:?}: {mark:?} out of bounds"
            );
            assert!(
                line.is_char_boundary(mark.start) && line.is_char_boundary(mark.end),
                "{line:?}: {mark:?} splits a character"
            );
            assert!(
                prose
                    .iter()
                    .any(|p| p.start <= mark.start && mark.end <= p.end.min(line.len())),
                "{line:?}: {mark:?} is outside {prose:?}"
            );
            if i > 0 {
                assert!(
                    marks[i - 1].end < mark.start,
                    "{line:?}: {:?} and {mark:?} are not sorted and apart",
                    marks[i - 1]
                );
            }
        }
    }

    /// A small deterministic generator (xorshift64*), so failures replay.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
            &items[self.below(items.len())]
        }
    }

    const PIECES: &[&str] = &[
        "the",
        "chek",
        "misteak",
        "Hello",
        "CommonMark",
        "README",
        "PRs",
        "doesn’t",
        "don't",
        "Catppuccin’s",
        "it's",
        "'s",
        "’",
        "'",
        " ",
        "  ",
        "\t",
        "\u{a0}",
        "\u{3000}",
        "-",
        ".",
        ",",
        "_",
        "*",
        "**",
        "~",
        "~~",
        "@",
        "/",
        "\\",
        "<",
        ">",
        "[^",
        "[",
        "]",
        "(",
        ")",
        "#",
        "`",
        "0",
        "v1.2",
        "é",
        "e\u{301}",
        "\u{301}",
        "😀",
        "🧑‍🧑‍🧒",
        "🇺🇸",
        "שלום",
        "مرحبا",
        "\u{200f}",
        "\u{202e}",
        "日本",
        "ß",
        "İ",
        "ı",
        "ǅ",
        "Știință",
        "\r",
        "\0",
    ];

    #[test]
    fn seeded_random_lines_keep_the_contract() {
        let speller = speller();
        let mut rng = Rng(0x5eed_5eed_1234_abcd);
        let mut suggested = 0;
        for case in 0..3_000 {
            let mut line = String::new();
            for _ in 0..rng.below(30) {
                line.push_str(rng.pick(PIECES));
            }
            let mut prose: Vec<Range<usize>> = (0..rng.below(4))
                .map(|_| {
                    let a = rng.below(line.len() + 3);
                    let b = rng.below(line.len() + 3);
                    a..b
                })
                .collect();
            if rng.below(2) == 0 {
                prose.sort_by_key(|r| r.start);
            }
            if rng.below(4) == 0 {
                prose = vec![0..line.len()];
            }
            let mut out = Vec::new();
            speller.marks(&line, &prose, &mut out);
            assert_valid(&line, &prose, &out);
            for mark in &out {
                assert!(
                    words::checkable(&line[mark.clone()]),
                    "case {case}: {line:?} marked {mark:?}, which is not checked"
                );
            }
            if let (0, Some(mark)) = (case % 100, out.first()) {
                let word = &line[mark.clone()];
                let found = speller.suggestions(word, 5);
                assert!(found.len() <= 5, "{word:?}: {found:?}");
                let straight = words::straight(word);
                for (i, correction) in found.iter().enumerate() {
                    assert!(
                        !correction.is_empty()
                            && !correction.contains(char::is_control)
                            && !correction.contains('’')
                            && *correction != *straight
                            && !found[..i].contains(correction),
                        "case {case}: {word:?}: {found:?}"
                    );
                }
                suggested += 1;
            }
        }
        assert!(suggested > 0);
    }
}
