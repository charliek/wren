//! Label text broken into lines that fit a width, with the host's
//! measurer.

use unicode_segmentation::UnicodeSegmentation;

use super::{Face, Measure};

/// A measured width at most this much over the box still fits, so a label
/// measured at exactly the box's width is not broken by rounding.
const SLACK: f32 = 0.001;

/// `text` broken at its line feeds and wrapped at `width`: a line breaks
/// after a run of spaces, and inside a word only when the word alone is
/// wider than `width`, then between graphemes. Every line comes back with
/// trailing white space dropped and with its measured width; a blank line
/// is kept, empty, because it still takes its height. `None` for `width`
/// breaks at line feeds only.
pub(super) fn wrap(
    text: &str,
    width: Option<f32>,
    face: Face,
    size: f32,
    measure: &dyn Measure,
) -> Vec<(String, f32)> {
    let mut lines = Vec::new();
    let mut emit = |line: &str| {
        let line = line.trim_end();
        let measured = if line.is_empty() {
            0.0
        } else {
            measure.width(line, face, size)
        };
        lines.push((line.to_owned(), measured));
    };
    for hard in text.split('\n') {
        let Some(width) = width else {
            emit(hard);
            continue;
        };
        let fits = |candidate: &str| {
            let candidate = candidate.trim_end();
            candidate.is_empty() || measure.width(candidate, face, size) <= width + SLACK
        };
        if fits(hard) {
            emit(hard);
            continue;
        }
        // Each line is the longest run of whole words from `start` that
        // fits, found as `split_word` finds a run of graphemes, so a long
        // line costs a few measurements rather than one per word.
        let ends: Vec<usize> = words(hard)
            .scan(0, |end, word| {
                *end += word.len();
                Some(*end)
            })
            .collect();
        let (mut start, mut first) = (0, 0);
        while let Some(&word_end) = ends.get(first) {
            let mut word = hard.get(start..word_end).unwrap_or_default();
            if !fits(word) {
                // A word wider than the box alone is broken between
                // graphemes, each line holding at least one; its last
                // part starts the next line.
                let mut parts = split_word(word, &fits);
                let last = parts.pop().unwrap_or(word);
                for part in parts {
                    emit(part);
                }
                word = last;
                start = word_end - last.len();
            }
            if word.trim_end().is_empty() {
                // White space left at the start of a wrapped line starts
                // no line.
                start = word_end;
                first += 1;
                continue;
            }
            let count = longest(ends.len() - first, |count| {
                hard.get(start..ends[first + count - 1]).is_some_and(&fits)
            });
            let end = ends[first + count - 1];
            emit(hard.get(start..end).unwrap_or_default());
            start = end;
            first += count;
        }
    }
    lines
}

/// The words of one line, each with the spaces that follow it; spaces at
/// the start belong to the first word. A no-break space joins.
fn words(line: &str) -> impl Iterator<Item = &str> {
    let mut rest = line;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut seen_word = false;
        let mut spacing = false;
        let mut end = rest.len();
        for (index, c) in rest.char_indices() {
            let space = breaks_after(c);
            if !space && spacing && seen_word {
                end = index;
                break;
            }
            seen_word |= !space;
            spacing = space;
        }
        let (word, tail) = rest.split_at(end);
        rest = tail;
        Some(word)
    })
}

fn breaks_after(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}')
}

/// A word too wide for one line, cut between graphemes into lines that
/// fit, each holding at least one grapheme; the last part may be short.
/// Each line is found by [`longest`], so a line of `k` graphemes costs
/// about `2 log k` measurements of at most `2k` graphemes, and an enormous
/// word costs no more than its length times a logarithm.
fn split_word<'a>(word: &'a str, fits: &dyn Fn(&str) -> bool) -> Vec<&'a str> {
    let ends: Vec<usize> = word
        .grapheme_indices(true)
        .map(|(index, grapheme)| index + grapheme.len())
        .collect();
    let mut parts = Vec::new();
    let (mut start, mut first) = (0, 0);
    while first < ends.len() {
        let count = longest(ends.len() - first, |count| {
            fits(&word[start..ends[first + count - 1]])
        });
        parts.push(&word[start..ends[first + count - 1]]);
        start = ends[first + count - 1];
        first += count;
    }
    parts
}

/// The largest `count` from 1 to `max` for which `fits` holds, or 1 when
/// none does, for a `fits` that holds for every count below one that holds.
/// Doubling until it fails and then bisecting takes about `2 log count`
/// calls.
fn longest(max: usize, fits: impl Fn(usize) -> bool) -> usize {
    // `good` is known to fit (0: none known), `bad` not.
    let (mut good, mut bad) = (0, None);
    let mut probe = 1;
    while good < max {
        if fits(probe) {
            good = probe;
            probe = (probe * 2).min(max);
        } else {
            bad = Some(probe);
            break;
        }
    }
    if let Some(mut hi) = bad {
        while hi - good > 1 {
            let mid = good + (hi - good) / 2;
            if fits(mid) {
                good = mid;
            } else {
                hi = mid;
            }
        }
    }
    good.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::FixedMeasure;

    /// Lines at 14 px under the fixed measurer, where a grapheme is 7 wide.
    fn lines(text: &str, width: f32) -> Vec<String> {
        wrap(text, Some(width), Face::Regular, 14.0, &FixedMeasure)
            .into_iter()
            .map(|(line, measured)| {
                assert_eq!(measured, FixedMeasure.width(&line, Face::Regular, 14.0));
                line
            })
            .collect()
    }

    #[test]
    fn words_keep_their_following_spaces() {
        let split = |s| words(s).collect::<Vec<_>>();
        assert_eq!(split(""), Vec::<&str>::new());
        assert_eq!(split("a"), ["a"]);
        assert_eq!(split("ab  cd e"), ["ab  ", "cd ", "e"]);
        assert_eq!(split("  lead"), ["  lead"]);
        assert_eq!(split("   "), ["   "]);
        assert_eq!(split("no\u{a0}break here"), ["no\u{a0}break ", "here"]);
        assert_eq!(split("tab\there"), ["tab\t", "here"]);
    }

    /// Text, box width, the lines drawn.
    #[test]
    fn the_wrap_table() {
        #[rustfmt::skip]
        let rows: &[(&str, f32, &[&str])] = &[
            ("", 100.0, &[""]),
            ("Start", 100.0, &["Start"]),
            ("Start", 35.0, &["Start"]),
            ("Start", 34.9, &["Star", "t"]),
            ("Send reset link", 105.0, &["Send reset link"]),
            ("Send reset link", 104.0, &["Send reset", "link"]),
            ("Send reset link", 70.0, &["Send reset", "link"]),
            ("Send reset link", 69.0, &["Send", "reset", "link"]),
            ("a\nb", 100.0, &["a", "b"]),
            ("a\n\nb", 100.0, &["a", "", "b"]),
            ("trailing   ", 100.0, &["trailing"]),
            ("wrap   here", 28.0, &["wrap", "here"]),
            ("  indent", 100.0, &["  indent"]),
            ("unbreakable", 21.0, &["unb", "rea", "kab", "le"]),
            ("x unbreakable y", 21.0, &["x", "unb", "rea", "kab", "le", "y"]),
            ("ab", 0.0, &["a", "b"]),
            ("ab cd", 0.0, &["a", "b", "c", "d"]),
            ("注文を受け取る", 28.0, &["注文を受", "け取る"]),
            ("e\u{301}e\u{301}e\u{301}", 14.0, &["e\u{301}e\u{301}", "e\u{301}"]),
            ("👨‍👩‍👧👨‍👩‍👧", 7.0, &["👨‍👩‍👧", "👨‍👩‍👧"]),
            ("no\u{a0}break", 28.0, &["no\u{a0}b", "reak"]),
        ];
        for &(text, width, want) in rows {
            assert_eq!(lines(text, width), want, "{text:?} at {width}");
        }
    }

    #[test]
    fn no_width_breaks_at_line_feeds_only() {
        let got = wrap("one two\nthree  ", None, Face::Regular, 12.0, &FixedMeasure);
        assert_eq!(
            got,
            [("one two".to_owned(), 42.0), ("three".to_owned(), 30.0)]
        );
    }

    #[test]
    fn every_line_fits_unless_it_is_one_grapheme() {
        let text = "The quick brown fox jumps over the lazy dog, \
                    then naps beneath an extraordinarily-long-hyphenated-word.";
        for width in [0.0, 5.0, 7.0, 20.0, 49.0, 50.0, 100.0, 333.0, 1000.0] {
            let got = lines(text, width);
            for line in &got {
                let fits = FixedMeasure.width(line, Face::Regular, 14.0) <= width;
                assert!(
                    fits || line.graphemes(true).count() == 1,
                    "{line:?} at {width}"
                );
            }
            let rejoined: String = got.concat();
            let squeezed: String = text.split_whitespace().collect();
            assert_eq!(
                rejoined.replace(' ', ""),
                squeezed,
                "nothing lost at {width}"
            );
        }
    }

    use std::cell::Cell;

    /// The fixed measurer, counting its measurements.
    struct Counting(Cell<usize>);

    impl Measure for Counting {
        fn width(&self, text: &str, face: Face, size: f32) -> f32 {
            self.0.set(self.0.get() + 1);
            FixedMeasure.width(text, face, size)
        }
        fn line_height(&self, face: Face, size: f32) -> f32 {
            FixedMeasure.line_height(face, size)
        }
    }

    #[test]
    fn an_enormous_word_breaks_in_few_measurements() {
        let word = "x".repeat(16_384);
        let counting = Counting(Cell::new(0));
        let got = wrap(&word, Some(7.0 * 1000.0), Face::Regular, 14.0, &counting);
        assert_eq!(got.len(), 17);
        assert!(
            counting.0.get() < 17 * 25,
            "{} measurements",
            counting.0.get()
        );
        // A box too narrow for any grapheme: one per line, and still
        // linear in the word's length rather than quadratic.
        let counting = Counting(Cell::new(0));
        let got = wrap(&word, Some(0.0), Face::Regular, 14.0, &counting);
        assert_eq!(got.len(), word.len());
        assert!(
            counting.0.get() < 3 * word.len(),
            "{} measurements",
            counting.0.get()
        );
    }

    /// Joining words one at a time re-measured the whole line for each
    /// word, so a line of `k` words cost `k` measurements of up to `k`
    /// words: seconds for a large label in a wide box.
    #[test]
    fn a_line_of_many_words_takes_few_measurements() {
        let label = "a ".repeat(8_000);
        let counting = Counting(Cell::new(0));
        // 4,000 words to a line, so four lines.
        let got = wrap(&label, Some(28_000.0), Face::Regular, 14.0, &counting);
        assert_eq!(got.len(), 4);
        assert!(
            counting.0.get() < 4 * 30,
            "{} measurements",
            counting.0.get()
        );
    }

    /// The lines are the ones the plain greedy rule gives: take words while
    /// the line fits, and break a word only when it is alone and too wide.
    #[test]
    fn wrapping_matches_word_by_word_greedy_wrapping() {
        fn greedy(text: &str, width: f32) -> Vec<String> {
            let fits = |candidate: &str| {
                let candidate = candidate.trim_end();
                candidate.is_empty()
                    || FixedMeasure.width(candidate, Face::Regular, 14.0) <= width + SLACK
            };
            let mut lines = Vec::new();
            for hard in text.split('\n') {
                if fits(hard) {
                    lines.push(hard.trim_end().to_owned());
                    continue;
                }
                let mut line = String::new();
                for word in words(hard) {
                    if !line.trim_end().is_empty() {
                        if fits(&format!("{line}{word}")) {
                            line.push_str(word);
                            continue;
                        }
                        lines.push(line.trim_end().to_owned());
                    }
                    line.clear();
                    let mut parts = split_word(word, &fits);
                    let last = parts.pop().unwrap_or(word);
                    lines.extend(parts.iter().map(|part| part.trim_end().to_owned()));
                    line.push_str(last);
                }
                if !line.trim_end().is_empty() {
                    lines.push(line.trim_end().to_owned());
                }
            }
            lines
        }
        let pieces = [
            "a",
            "bb",
            "ccccc",
            "dddddddddddd",
            " ",
            "  ",
            "\n",
            "\u{a0}",
            "é",
            "漢字",
        ];
        let mut rng = crate::fixtures::Rng(7);
        for _ in 0..2_000 {
            let text: String = (0..rng.below(24)).map(|_| *rng.pick(&pieces)).collect();
            let width = *rng.pick(&[0.0, 7.0, 13.0, 21.0, 35.0, 70.0, 100.0, 1_000.0]);
            assert_eq!(
                lines(&text, width),
                greedy(&text, width),
                "{text:?} at {width}"
            );
        }
    }
}
