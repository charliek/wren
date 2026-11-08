//! Unicode-lowercase literal find over UTF-8 byte offsets.

use std::collections::VecDeque;
use std::ops::Range;

use iced::widget::text_editor::{LineEnding, Position};

use crate::backend::Lines;
use crate::clamp_position;

struct FirstLiteral {
    needle: Vec<char>,
    prefixes: Vec<usize>,
    matched: usize,
    provenance: VecDeque<(usize, usize)>,
}

impl FirstLiteral {
    fn new(query: &str) -> Option<Self> {
        let needle: Vec<_> = query.chars().flat_map(char::to_lowercase).collect();
        if needle.is_empty() {
            return None;
        }
        let mut prefixes = vec![0; needle.len()];
        let mut matched = 0;
        for index in 1..needle.len() {
            while matched > 0 && needle[index] != needle[matched] {
                matched = prefixes[matched - 1];
            }
            if needle[index] == needle[matched] {
                matched += 1;
            }
            prefixes[index] = matched;
        }
        let provenance = VecDeque::with_capacity(needle.len());
        Some(Self {
            needle,
            prefixes,
            matched: 0,
            provenance,
        })
    }

    fn push(&mut self, byte: usize, character: char) -> Option<Range<usize>> {
        for lower in character.to_lowercase() {
            if self.provenance.len() == self.needle.len() {
                self.provenance.pop_front();
            }
            self.provenance
                .push_back((byte, byte + character.len_utf8()));
            while self.matched > 0 && self.needle[self.matched] != lower {
                self.matched = self.prefixes[self.matched - 1];
            }
            if self.needle[self.matched] == lower {
                self.matched += 1;
            }
            if self.matched == self.needle.len() {
                return Some(self.provenance.front()?.0..self.provenance.back()?.1);
            }
        }
        None
    }
}

pub(crate) fn first_content_match(lines: Lines<'_>, query: &str) -> Option<Range<usize>> {
    let mut matcher = FirstLiteral::new(query)?;
    let count = lines.len();
    let mut base = 0;
    // Stream the borrowed lines rather than join the whole text; keep only
    // query-sized provenance and stop at the first hit.
    for (line, text) in lines.iter().enumerate() {
        for (byte, character) in text.text.char_indices() {
            if let Some(range) = matcher.push(base + byte, character) {
                return Some(range);
            }
        }
        base += text.text.len();
        if line + 1 < count {
            let ending = match text.ending {
                LineEnding::None => LineEnding::default(),
                ending => ending,
            };
            for (byte, character) in ending.as_str().char_indices() {
                if let Some(range) = matcher.push(base + byte, character) {
                    return Some(range);
                }
            }
            base += ending.as_str().len();
        }
    }
    None
}

/// Return `(byte_start, byte_end)` pairs for all case-insensitive occurrences
/// of `query` in `text`.
///
/// Uses full `char::to_lowercase` expansions, not Unicode case folding or
/// normalization. Each expanded character maps back to its original UTF-8
/// character range, so even a partial expansion match selects valid text.
/// Overlapping matches are included, with duplicate source ranges removed.
pub(crate) fn find_all_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_chars: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let text_chars: Vec<(char, usize, usize)> = text
        .char_indices()
        .flat_map(|(start, ch)| {
            ch.to_lowercase()
                .map(move |lower| (lower, start, start + ch.len_utf8()))
        })
        .collect();
    let mut matches = Vec::new();
    for window in text_chars.windows(query_chars.len()) {
        if window
            .iter()
            .zip(&query_chars)
            .all(|((ch, _, _), query)| ch == query)
        {
            let range = (window[0].1, window[window.len() - 1].2);
            if matches.last() != Some(&range) {
                matches.push(range);
            }
        }
    }
    matches
}

pub(crate) fn exact_byte_offset_to_position(
    lines: Lines<'_>,
    mut offset: usize,
) -> Option<Position> {
    let count = lines.len();
    for (line, text) in lines.iter().enumerate() {
        if offset <= text.text.len() {
            return text.text.is_char_boundary(offset).then_some(Position {
                line,
                column: offset,
            });
        }
        if line + 1 == count {
            return None;
        }
        let ending = match text.ending {
            LineEnding::None => LineEnding::default(),
            ending => ending,
        };
        let width = text.text.len() + ending.as_str().len();
        // A cursor cannot represent the gap between CR and LF (or LF and CR).
        if offset < width {
            return None;
        }
        offset -= width;
    }
    None
}

/// Convert an offset in `Content::text()` to a valid UTF-8 cursor position.
/// Offsets inside a two-byte line ending snap to the next line's start.
pub(crate) fn byte_offset_to_position(lines: Lines<'_>, mut offset: usize) -> Position {
    let line_count = lines.len().max(1);
    for (line, text) in lines.iter().enumerate() {
        if offset <= text.text.len() || line + 1 == line_count {
            return clamp_position(
                Position {
                    line,
                    column: offset,
                },
                lines,
            );
        }
        // Match Content::text's fallback for an unterminated interior line.
        let ending = match text.ending {
            LineEnding::None => LineEnding::default(),
            ending => ending,
        };
        offset = offset.saturating_sub(text.text.len() + ending.as_str().len());
    }
    Position { line: 0, column: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::Content;

    #[test]
    fn first_match_stream_agrees_with_literal_find_on_small_unicode_inputs() {
        for (text, query) in [
            ("aaaaab", "aaab"),
            ("abababac", "ababac"),
            ("İx", "\u{0307}x"),
            ("prefix 🐦 e\u{0301} CAFÉ", "café"),
            ("ΟΣ", "σ"),
            ("ΟΣ", "ς"),
            ("i\u{0307}", "İ"),
            ("first\r\nsecond\n\rthird\rlast", "second\n\rthird"),
            ("no match", "missing"),
            ("anything", ""),
        ] {
            let content = Content::with_text(text);
            let expected = find_all_matches(text, query)
                .first()
                .map(|&(start, end)| start..end);
            assert_eq!(
                content.with_lines(|lines| first_content_match(lines, query)),
                expected,
                "{text:?}, {query:?}"
            );
        }
    }

    #[test]
    fn first_match_stream_stops_immediately_and_retains_only_query_sized_provenance() {
        let mut matcher = FirstLiteral::new("a").unwrap();
        let mut visited = 0;
        let range = (0..64 * 1024 * 1024).find_map(|byte| {
            visited += 1;
            matcher.push(byte, 'a')
        });
        assert_eq!(range, Some(0..1));
        assert_eq!(visited, 1);
        assert_eq!(matcher.provenance.len(), 1);

        let mut matcher = FirstLiteral::new("aaaaab").unwrap();
        for byte in 0..100_000 {
            assert_eq!(matcher.push(byte, 'a'), None);
            assert!(matcher.provenance.len() <= matcher.needle.len());
        }
        assert_eq!(matcher.push(100_000, 'b'), Some(99_995..100_001));
        assert_eq!(matcher.provenance.len(), 6);
    }

    #[test]
    fn find_all_matches_empty_query() {
        assert!(find_all_matches("hello", "").is_empty());
    }

    #[test]
    fn find_all_matches_no_overlap() {
        let matches = find_all_matches("aaa", "aa");
        // "aaa" with query "aa": matches at [0,2) only (no overlapping)
        // Our implementation advances by 1 so there would be two: [0,2) and [1,3)
        // This is deliberate — overlapping matches are reported.
        assert!(!matches.is_empty());
        // All byte boundaries must be valid UTF-8
        let text = "aaa";
        for (s, e) in &matches {
            assert!(text.is_char_boundary(*s));
            assert!(text.is_char_boundary(*e));
        }
    }

    #[test]
    fn find_all_matches_emoji() {
        let text = "👋 hello 👋";
        let matches = find_all_matches(text, "👋");
        assert_eq!(matches.len(), 2);
        // First match: byte 0 to byte 4 ("👋" = 4 bytes)
        assert_eq!(matches[0], (0, 4));
        // "👋 hello 👋": 👋(4) + ' '(1) + "hello"(5) + ' '(1) + 👋(4)
        // Second 👋 starts at byte 11 (4+1+5+1 = 11), ends at 15.
        assert_eq!(matches[1].0, 11);
        assert_eq!(matches[1].1, 15);
    }
}
