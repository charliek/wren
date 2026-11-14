//! What a word is: the chunks of a prose range, the candidate words in them,
//! and which of those are checked at all.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// The longest word, in characters, that is checked.
pub(crate) const MAX_CHARS: usize = 40;

/// Calls `found` with the byte range, in `line`, of each word of the prose
/// range `prose` that is checked. `prose` must lie on character boundaries.
pub(crate) fn checked_words(line: &str, prose: Range<usize>, mut found: impl FnMut(Range<usize>)) {
    let text = &line[prose.clone()];
    for (chunk_start, chunk) in chunks(text) {
        if skipped(chunk) {
            continue;
        }
        for (offset, word) in chunk.unicode_word_indices() {
            let start = prose.start + chunk_start + offset;
            let end = start + word.len();
            // A prose range that cuts a word of the line leaves only a piece
            // of it here, which is not a word to judge.
            let cut_before = start == prose.start && continues(line[..start].chars().next_back());
            let cut_after = end == prose.end && continues(line[end..].chars().next());
            if !cut_before && !cut_after && checkable(word) {
                found(start..end);
            }
        }
    }
}

/// The runs of `text` between Unicode white space, with their byte offsets.
fn chunks(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut rest = 0;
    std::iter::from_fn(move || {
        let tail = &text[rest..];
        let skip = tail.len() - tail.trim_start().len();
        let start = rest + skip;
        if start == text.len() {
            return None;
        }
        let end = text[start..]
            .find(char::is_whitespace)
            .map_or(text.len(), |at| start + at);
        rest = end;
        Some((start, &text[start..end]))
    })
}

/// Whether a chunk is left alone whole: an address, a path, an inline tag, a
/// footnote mark, a `#tag`, or words joined by `_`, `*` or `~`.
fn skipped(chunk: &str) -> bool {
    if chunk.contains(['@', '/', '\\', '<', '>']) || chunk.contains("[^") {
        return true;
    }
    let mut chars = chunk.chars();
    if chars.next() == Some('#') && chars.next().is_some_and(char::is_alphabetic) {
        return true;
    }
    joined_inside(chunk)
}

/// Whether a run of `_`, `*` or `~` has a letter or digit directly on both
/// sides. A run, not a single character, so `foo**bar` counts as joined.
fn joined_inside(chunk: &str) -> bool {
    let mut previous: Option<char> = None;
    let mut before_run: Option<char> = None;
    let mut in_run = false;
    for c in chunk.chars() {
        if matches!(c, '_' | '*' | '~') {
            if !in_run {
                before_run = previous;
                in_run = true;
            }
        } else {
            if in_run && continues(before_run) && c.is_alphanumeric() {
                return true;
            }
            in_run = false;
        }
        previous = Some(c);
    }
    false
}

fn continues(neighbour: Option<char>) -> bool {
    neighbour.is_some_and(char::is_alphanumeric)
}

pub(crate) fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '’'
}

/// `text` with each `’` read as `'`, as words are looked up.
pub(crate) fn straight(text: &str) -> Cow<'_, str> {
    if text.contains('’') {
        Cow::Owned(text.replace('’', "'"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Whether a candidate word is checked: Latin letters and apostrophes only,
/// 2 to 40 characters, and not an all-capitals word.
pub(crate) fn checkable(word: &str) -> bool {
    let mut count = 0;
    for c in word.chars() {
        if !(is_apostrophe(c) || (c.is_alphabetic() && c <= '\u{24F}')) {
            return false;
        }
        count += 1;
    }
    (2..=MAX_CHARS).contains(&count) && !all_capitals(word)
}

/// `README`, `README's`, `PRs`: two or more letters, every cased one upper
/// case, optionally followed by `s` or `'s`.
fn all_capitals(word: &str) -> bool {
    let stem = word
        .strip_suffix("'s")
        .or_else(|| word.strip_suffix("’s"))
        .or_else(|| word.strip_suffix('s'))
        .unwrap_or(word);
    // A character that upper-cases to itself is upper case or has no case;
    // a lower-case or title-case letter does not.
    stem.chars().filter(|c| c.is_alphabetic()).count() >= 2
        && stem.chars().all(|c| c.to_uppercase().eq([c]))
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // an expected list of marks often holds one
mod tests {
    use super::*;

    fn words_of(line: &str) -> Vec<&str> {
        let mut out = Vec::new();
        checked_words(line, 0..line.len(), |r| out.push(&line[r]));
        out
    }

    #[test]
    fn chunks_split_at_unicode_white_space() {
        let text = " a\tb\u{a0}c\u{3000}d  ";
        let found: Vec<_> = chunks(text).collect();
        assert_eq!(
            found,
            vec![(1, "a"), (3, "b"), (6, "c"), (10, "d")],
            "tab, no-break and ideographic spaces all separate chunks"
        );
        assert_eq!(chunks("").count(), 0);
        assert_eq!(chunks(" \n ").count(), 0);
    }

    #[test]
    fn rule_1_skips_whole_chunks() {
        assert_eq!(words_of("mail me@exmaple.com now"), ["mail", "now"]);
        assert_eq!(words_of("and/or adn/or"), Vec::<&str>::new());
        assert_eq!(words_of(r"C:\Windwos\Sytem path"), ["path"]);
        assert_eq!(words_of("a <br> and <kbd>Ctrll</kbd>"), ["and"]);
        assert_eq!(words_of("x->y ok"), ["ok"]);
        assert_eq!(words_of("note[^1] and [^fotnote]"), ["and"]);
        assert_eq!(words_of("#tagg #2024 ##heding"), ["heding"]);
        assert_eq!(
            words_of("snake_case_nmae un*believ*able wavy~tilda strong**wrod**ish"),
            Vec::<&str>::new()
        );
        assert_eq!(words_of("v1_2 kept"), ["kept"], "a digit counts as a side");
    }

    #[test]
    fn rule_1_keeps_delimiters_that_do_not_join_letters() {
        assert_eq!(words_of("*bold* ~~gone~~ a*"), ["bold", "gone"]);
        let line = "_word_";
        let mut out = Vec::new();
        checked_words(line, 1..5, |r| out.push(r));
        assert_eq!(out, [1..5], "emphasis whose delimiters are not prose");
        assert_eq!(
            words_of("_word_ trailing_"),
            Vec::<&str>::new(),
            "`_` is part of a UAX #29 word, so these are never judged"
        );
        assert_eq!(words_of("a [link] here"), ["link", "here"]);
        assert_eq!(words_of("#"), Vec::<&str>::new());
        assert_eq!(words_of("# heading"), ["heading"]);
    }

    #[test]
    fn rule_2_candidates_are_uax29_words() {
        assert_eq!(
            words_of("well-known well-knwon"),
            ["well", "known", "well", "knwon"]
        );
        assert_eq!(words_of("(quoted), \"words\"!"), ["quoted", "words"]);
        assert_eq!(words_of("doesn't doesn’t"), ["doesn't", "doesn’t"]);
        assert_eq!(words_of("end."), ["end"]);
    }

    #[test]
    fn rule_4_letters_and_apostrophes_only() {
        assert_eq!(words_of("v0.1.1 h2o 3rd e.g"), Vec::<&str>::new());
        assert_eq!(words_of("café naïve façade"), ["café", "naïve", "façade"]);
        assert_eq!(words_of("Łódź Știință"), ["Łódź", "Știință"]);
        assert_eq!(words_of("слово λόγος שלום 日本語"), Vec::<&str>::new());
        assert_eq!(
            words_of("cafe\u{301}"),
            Vec::<&str>::new(),
            "a combining mark is not a letter"
        );
    }

    #[test]
    fn rule_4_two_to_forty_characters() {
        let forty = "a".repeat(40);
        let forty_one = "a".repeat(41);
        let line = format!("a b c {forty} {forty_one} ab");
        assert_eq!(words_of(&line), [forty.as_str(), "ab"]);
        assert!(checkable("it"));
        assert!(!checkable("I"));
        assert!(checkable(&"é".repeat(40)), "characters, not bytes");
    }

    #[test]
    fn rule_4_all_capitals_words_are_skipped() {
        for word in [
            "README",
            "README's",
            "README’s",
            "PRs",
            "NASA",
            "DON'T",
            "ÉCOLE",
        ] {
            assert!(!checkable(word), "{word:?}");
        }
        for word in [
            "As",
            "Is",
            "Ok",
            "CommonMark",
            "PRsx",
            "READMEss",
            "ǅA",
            "Readme",
        ] {
            assert!(checkable(word), "{word:?}");
        }
    }

    #[test]
    fn a_cut_word_is_not_checked() {
        let line = "helloworld foo";
        let mut out = Vec::new();
        checked_words(line, 0..3, |r| out.push(r));
        assert!(out.is_empty(), "{out:?}: `hel` is a piece of `helloworld`");
        checked_words(line, 3..14, |r| out.push(r));
        assert_eq!(out, [11..14], "`loworld` is a piece; `foo` is whole");
        out.clear();
        checked_words("foo`code`bar", 0..3, |r| out.push(r));
        checked_words("foo`code`bar", 9..12, |r| out.push(r));
        assert_eq!(out, [0..3, 9..12], "a code span does not cut a word");
    }
}
