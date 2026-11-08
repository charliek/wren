//! The heading outline of a document.
//!
//! Headings come from [`block::scan`](crate::block::scan), so nothing in
//! code-like lines (fences, indented code, HTML blocks) or front matter is
//! a heading. Headings inside block quotes and list items are included:
//! they render as headings, and a note that nests its sections under list
//! items still wants them in its outline. ATX and setext headings both
//! count; headings whose plain text is empty are skipped.

use std::ops::Range;

use crate::block::{self, Kind};
use crate::Lines;

/// One heading of the outline.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Heading {
    /// 1–6. Setext `===` headings are level 1 and `---` headings level 2.
    pub level: u8,
    /// The title with inline markup removed: emphasis, code spans, links
    /// and images (their text), autolinks, HTML tags, and escapes.
    pub text: String,
    /// The first line of the heading.
    pub line: usize,
    /// The lines the heading covers: one for ATX, and the text lines plus
    /// the underline for setext.
    pub lines: Range<usize>,
    /// The byte range of the raw title on `line`, before markup is removed.
    pub range: Range<usize>,
    /// The index of the enclosing heading: the nearest earlier heading of a
    /// lower level.
    pub parent: Option<usize>,
}

/// A document's headings, in document order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Outline {
    headings: Vec<Heading>,
}

impl Outline {
    pub fn new<L: Lines + ?Sized>(lines: &L) -> Self {
        let infos = block::scan(lines);
        let mut headings: Vec<Heading> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        // The text range of each paragraph line since the last non-paragraph
        // line, for setext headings.
        let mut paragraph: Vec<(usize, Range<usize>)> = Vec::new();
        for (i, info) in infos.iter().enumerate() {
            let line = lines.line(i);
            let found = match &info.kind {
                Kind::Prose => {
                    let end = info.text_start + line[info.text_start..].trim_end().len();
                    paragraph.push((i, info.text_start..end));
                    None
                }
                Kind::Heading { level, text } => Some((
                    *level,
                    i,
                    i..i + 1,
                    text.clone(),
                    plain_text(&line[text.clone()]),
                )),
                Kind::SetextUnderline { level, start } => {
                    let parts: Vec<&(usize, Range<usize>)> =
                        paragraph.iter().filter(|(l, _)| l >= start).collect();
                    let raw: Vec<&str> = parts
                        .iter()
                        .map(|(l, r)| &lines.line(*l)[r.clone()])
                        .collect();
                    let range = parts.first().map_or(0..0, |(_, r)| r.clone());
                    Some((
                        *level,
                        *start,
                        *start..i + 1,
                        range,
                        plain_text(&raw.join(" ")),
                    ))
                }
                _ => None,
            };
            if info.kind != Kind::Prose {
                paragraph.clear();
            }
            let Some((level, first, covered, range, text)) = found else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            while open.last().is_some_and(|&h| headings[h].level >= level) {
                open.pop();
            }
            headings.push(Heading {
                level,
                text,
                line: first,
                lines: covered,
                range,
                parent: open.last().copied(),
            });
            open.push(headings.len() - 1);
        }
        Self { headings }
    }

    pub fn headings(&self) -> &[Heading] {
        &self.headings
    }

    /// The index of the deepest heading whose section contains `line`: the
    /// last heading that starts at or before it.
    pub fn section_at(&self, line: usize) -> Option<usize> {
        self.headings
            .partition_point(|h| h.line <= line)
            .checked_sub(1)
    }

    /// The headings enclosing heading `index`, nearest first.
    pub fn ancestors(&self, index: usize) -> impl Iterator<Item = &Heading> + '_ {
        std::iter::successors(self.headings[index].parent, |&i| self.headings[i].parent)
            .map(|i| &self.headings[i])
    }
}

/// Inline Markdown reduced to its visible text, with whitespace collapsed.
pub fn plain_text(s: &str) -> String {
    let mut pieces = Vec::new();
    inline_pieces(s, &mut pieces);
    match_delimiters(&mut pieces);
    let mut out = String::new();
    for piece in &pieces {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Delim { ch, len, .. } => out.extend(std::iter::repeat_n(*ch, *len)),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug)]
enum Piece {
    Text(String),
    /// A run of `*`, `_`, or `~` that may open or close emphasis.
    Delim {
        ch: char,
        len: usize,
        open: bool,
        close: bool,
    },
}

fn push_text(pieces: &mut Vec<Piece>, s: &str) {
    if let Some(Piece::Text(t)) = pieces.last_mut() {
        t.push_str(s);
    } else {
        pieces.push(Piece::Text(s.into()));
    }
}

fn inline_pieces(s: &str, pieces: &mut Vec<Piece>) {
    let mut i = 0;
    while let Some(ch) = s[i..].chars().next() {
        let rest = &s[i..];
        match ch {
            '\\' => {
                let next = rest[1..].chars().next();
                match next {
                    Some(p) if p.is_ascii_punctuation() => {
                        push_text(pieces, &rest[1..2]);
                        i += 2;
                    }
                    _ => {
                        push_text(pieces, "\\");
                        i += 1;
                    }
                }
            }
            '`' => {
                let run = rest.bytes().take_while(|&b| b == b'`').count();
                match code_span(rest, run) {
                    Some((content, len)) => {
                        push_text(pieces, content);
                        i += len;
                    }
                    None => {
                        push_text(pieces, &rest[..run]);
                        i += run;
                    }
                }
            }
            '!' if rest[1..].starts_with('[') => match link(&rest[1..]) {
                Some((label, len)) => {
                    inline_pieces(label, pieces);
                    i += 1 + len;
                }
                None => {
                    push_text(pieces, "!");
                    i += 1;
                }
            },
            '[' => match link(rest) {
                Some((label, len)) => {
                    inline_pieces(label, pieces);
                    i += len;
                }
                None => {
                    push_text(pieces, "[");
                    i += 1;
                }
            },
            '<' => match angle(rest) {
                Some((visible, len)) => {
                    push_text(pieces, visible);
                    i += len;
                }
                None => {
                    push_text(pieces, "<");
                    i += 1;
                }
            },
            '*' | '_' | '~' => {
                let len = rest.chars().take_while(|&c| c == ch).count();
                let before = s[..i].chars().next_back();
                let after = rest[len..].chars().next();
                let (open, close) = flanking(ch, before, after);
                let usable = ch != '~' || len <= 2;
                pieces.push(Piece::Delim {
                    ch,
                    len,
                    open: open && usable,
                    close: close && usable,
                });
                i += len;
            }
            _ => {
                push_text(pieces, &rest[..ch.len_utf8()]);
                i += ch.len_utf8();
            }
        }
    }
}

/// A code span opening with `run` backticks: its content and total length.
fn code_span(rest: &str, run: usize) -> Option<(&str, usize)> {
    let b = rest.as_bytes();
    let mut j = run;
    while j < b.len() {
        if b[j] == b'`' {
            let close = b[j..].iter().take_while(|&&x| x == b'`').count();
            if close == run {
                let content = &rest[run..j];
                let stripped = match content.strip_prefix(' ').and_then(|c| c.strip_suffix(' ')) {
                    Some(inner) if !content.trim().is_empty() => inner,
                    _ => content,
                };
                return Some((stripped, j + close));
            }
            j += close;
        } else {
            j += 1;
        }
    }
    None
}

/// A link or reference starting at `[`: its label and total length. A bare
/// `[text]` is left alone, since without its definition it is literal text.
fn link(rest: &str) -> Option<(&str, usize)> {
    let close = matching(rest, b'[', b']')?;
    let label = &rest[1..close];
    let after = &rest[close + 1..];
    let tail = match after.as_bytes().first() {
        Some(b'(') => matching(after, b'(', b')')?,
        Some(b'[') => matching(after, b'[', b']')?,
        _ => return None,
    };
    Some((label, close + 1 + tail + 1))
}

/// The byte index of the bracket closing the one at the start of `s`.
fn matching(s: &str, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut escaped = false;
    for (i, &b) in s.as_bytes().iter().enumerate() {
        if escaped {
            escaped = false;
        } else if b == b'\\' {
            escaped = true;
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// An autolink (its URL) or an HTML tag (nothing) starting at `<`.
fn angle(rest: &str) -> Option<(&str, usize)> {
    let end = rest.find('>')?;
    let inner = &rest[1..end];
    if inner.is_empty() || (inner.contains(['<', ' ', '\t']) && !is_tag(inner)) {
        return None;
    }
    let scheme = inner.split_once(':').map(|(s, _)| s);
    if scheme.is_some_and(|s| {
        s.len() >= 2
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
    }) || (inner.contains('@') && !inner.contains(' '))
    {
        return Some((inner, end + 1));
    }
    is_tag(inner).then_some(("", end + 1))
}

fn is_tag(inner: &str) -> bool {
    let name = inner.strip_prefix('/').unwrap_or(inner);
    name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .bytes()
            .take_while(|b| !matches!(b, b' ' | b'\t' | b'/'))
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Whether a delimiter run between `before` and `after` can open and close
/// emphasis (CommonMark's flanking rules).
fn flanking(ch: char, before: Option<char>, after: Option<char>) -> (bool, bool) {
    let ws = |c: Option<char>| c.is_none_or(char::is_whitespace);
    let punct = |c: Option<char>| c.is_some_and(|c| c.is_ascii_punctuation());
    let left = !ws(after) && (!punct(after) || ws(before) || punct(before));
    let right = !ws(before) && (!punct(before) || ws(after) || punct(after));
    if ch == '_' {
        (
            left && (!right || punct(before)),
            right && (!left || punct(after)),
        )
    } else {
        (left, right)
    }
}

/// Pair emphasis delimiters, removing the characters that pair up.
fn match_delimiters(pieces: &mut [Piece]) {
    let mut openers: Vec<usize> = Vec::new();
    for i in 0..pieces.len() {
        let Piece::Delim {
            ch, open, close, ..
        } = pieces[i]
        else {
            continue;
        };
        if close {
            while let Some(pos) = openers.iter().rposition(|&o| delim_char(&pieces[o]) == ch) {
                let o = openers[pos];
                let used = delim_len(&pieces[o]).min(delim_len(&pieces[i]));
                if ch == '~' && delim_len(&pieces[o]) != delim_len(&pieces[i]) {
                    break;
                }
                shrink(&mut pieces[o], used);
                shrink(&mut pieces[i], used);
                openers.truncate(pos + usize::from(delim_len(&pieces[o]) > 0));
                if delim_len(&pieces[i]) == 0 {
                    break;
                }
            }
        }
        if open && delim_len(&pieces[i]) > 0 {
            openers.push(i);
        }
    }
}

fn delim_char(piece: &Piece) -> char {
    match piece {
        Piece::Delim { ch, .. } => *ch,
        Piece::Text(_) => '\0',
    }
}

fn delim_len(piece: &Piece) -> usize {
    match piece {
        Piece::Delim { len, .. } => *len,
        Piece::Text(_) => 0,
    }
}

fn shrink(piece: &mut Piece, by: usize) {
    if let Piece::Delim { len, .. } = piece {
        *len -= by;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    fn outline(text: &str) -> Outline {
        Outline::new(&Document::new(text))
    }

    /// `(level, text, line)` for each heading.
    fn summary(text: &str) -> Vec<(u8, String, usize)> {
        outline(text)
            .headings()
            .iter()
            .map(|h| (h.level, h.text.clone(), h.line))
            .collect()
    }

    fn h(level: u8, text: &str, line: usize) -> (u8, String, usize) {
        (level, text.into(), line)
    }

    #[test]
    fn atx_and_setext_headings_in_order() {
        let text = "# One\ntext\n\nTwo\n===\n## Three ##\nFour\nfive\n---\n###### Six";
        assert_eq!(
            summary(text),
            [
                h(1, "One", 0),
                h(1, "Two", 3),
                h(2, "Three", 5),
                h(2, "Four five", 6),
                h(6, "Six", 9),
            ]
        );
    }

    #[test]
    fn headings_in_code_front_matter_and_html_are_skipped() {
        let text = "---\ntitle: x\n# yaml comment\n---\n```\n# fenced\n```\n~~~\n# tilde\n~~~\n\n    # indented\n\n<div>\n# html\n\n# real";
        assert_eq!(summary(text), [h(1, "real", 16)]);
    }

    #[test]
    fn unclosed_fence_hides_the_rest() {
        assert_eq!(summary("# a\n```\n# b\n# c"), [h(1, "a", 0)]);
    }

    #[test]
    fn empty_and_escaped_headings_are_skipped() {
        assert_eq!(
            summary("#\n# \n# #\n\\# no\n#no\n## `` ``\n# ok"),
            [h(1, "ok", 6)]
        );
    }

    #[test]
    fn headings_in_quotes_and_list_items_are_included() {
        let text = "> # Quoted\n- ## Item\n  Under\n  ---\n> > ### Deep";
        assert_eq!(
            summary(text),
            [
                h(1, "Quoted", 0),
                h(2, "Item", 1),
                h(2, "Under", 2),
                h(3, "Deep", 4)
            ]
        );
    }

    #[test]
    fn list_then_dashes_is_not_a_setext_heading() {
        assert_eq!(summary("- foo\n---\n# a"), [h(1, "a", 2)]);
    }

    #[test]
    fn heading_lines_and_ranges() {
        let text = "  ## *Hi* ##\nA\nB\n===";
        let o = outline(text);
        let [atx, setext] = o.headings() else {
            panic!("expected two headings");
        };
        assert_eq!(
            (atx.line, atx.lines.clone(), atx.range.clone()),
            (0, 0..1, 5..9)
        );
        assert_eq!(&text[atx.range.clone()], "*Hi*");
        assert_eq!(atx.text, "Hi");
        assert_eq!((setext.line, setext.lines.clone()), (1, 1..4));
        assert_eq!(setext.range, 0..1);
        assert_eq!(setext.text, "A B");
    }

    #[test]
    fn crlf_and_bom_documents() {
        assert_eq!(
            summary("\u{feff}# A\r\ntext\r\n\r\nB\r\n---\r\n## C\r\n"),
            [h(1, "A", 0), h(2, "B", 3), h(2, "C", 5)]
        );
        assert_eq!(summary("# A\rb\n\r## C"), [h(1, "A", 0), h(2, "C", 2)]);
        assert!(summary("").is_empty());
    }

    #[test]
    fn parents_and_ancestors() {
        let text = "# A\n## B\n### C\n## D\n#### E\n# F\n### G";
        let o = outline(text);
        let parents: Vec<Option<usize>> = o.headings().iter().map(|h| h.parent).collect();
        assert_eq!(
            parents,
            [None, Some(0), Some(1), Some(0), Some(3), None, Some(5)]
        );
        let path: Vec<&str> = o.ancestors(4).map(|h| h.text.as_str()).collect();
        assert_eq!(path, ["D", "A"]);
        assert_eq!(o.ancestors(0).count(), 0);
    }

    #[test]
    fn section_at_finds_the_deepest_containing_heading() {
        let text = "intro\n# A\na\n## B\nb\n\nC\n---\nc";
        let o = outline(text);
        let sections: Vec<Option<&str>> = (0..9)
            .map(|line| o.section_at(line).map(|i| o.headings()[i].text.as_str()))
            .collect();
        assert_eq!(
            sections,
            [
                None,
                Some("A"),
                Some("A"),
                Some("B"),
                Some("B"),
                Some("B"),
                Some("C"),
                Some("C"),
                Some("C")
            ]
        );
        assert_eq!(o.section_at(100), Some(2));
        assert_eq!(Outline::default().section_at(0), None);
    }

    #[test]
    fn plain_text_strips_inline_markup() {
        let cases: &[(&str, &str)] = &[
            ("plain", "plain"),
            ("*em* and **strong**", "em and strong"),
            ("_em_ and __strong__", "em and strong"),
            ("***both***", "both"),
            ("~~gone~~", "gone"),
            ("snake_case_name", "snake_case_name"),
            ("2 * 3 * 4", "2 * 3 * 4"),
            ("a*b*c", "abc"),
            ("**unclosed", "**unclosed"),
            ("**a*", "*a"),
            ("`code` span", "code span"),
            ("`` a`b ``", "a`b"),
            ("a ` ` b", "a b"),
            ("`unclosed", "`unclosed"),
            ("`*not em*`", "*not em*"),
            ("[link](http://x.y) text", "link text"),
            ("[*em* link](u \"t\")", "em link"),
            ("[ref][r]", "ref"),
            ("![alt](img.png)", "alt"),
            ("[WIP] plan", "[WIP] plan"),
            ("[a [b] c](u)", "a [b] c"),
            ("<https://example.com>", "https://example.com"),
            ("<me@example.com>", "me@example.com"),
            ("a <b>bold</b>", "a bold"),
            ("a < b > c", "a < b > c"),
            ("\\*not em\\*", "*not em*"),
            ("\\# hash", "# hash"),
            ("back\\slash", "back\\slash"),
            ("  spaced   out  ", "spaced out"),
            ("café *ñ*", "café ñ"),
        ];
        for (input, expected) in cases {
            assert_eq!(plain_text(input), *expected, "{input:?}");
        }
    }

    /// A generated note with headings, prose, lists, quotes, and fences.
    fn generated(lines: usize) -> String {
        let mut text = String::new();
        let mut i = 0;
        while i < lines {
            let line = match i % 33 {
                0 => "# Section heading".to_string(),
                1 | 12 | 24 => String::new(),
                5 => "## Subsection with *emphasis*".into(),
                6..=9 => format!("- item {i} with [a link](http://example.com)"),
                10 => "  - [ ] nested task".into(),
                11 => "  - [x] done task".into(),
                13 => "```rust".into(),
                14..=17 => "    let x = 1; // # not a heading".into(),
                18 => "```".into(),
                19..=21 => "> quoted text that goes on for a while".into(),
                22 => "1. ordered".into(),
                23 => "2. second".into(),
                _ => "Some paragraph text with `code` and **bold** words in it.".into(),
            };
            text.push_str(&line);
            text.push('\n');
            i += 1;
        }
        text
    }

    #[test]
    #[ignore = "micro benchmark; run with --release --ignored --nocapture"]
    fn benchmark_scan_and_outline_5000_lines() {
        let text = generated(5_000);
        let doc = Document::new(&text);
        let rounds = 200;
        let start = std::time::Instant::now();
        let mut kinds = 0;
        for _ in 0..rounds {
            kinds += block::scan(&doc).len();
        }
        let scan = start.elapsed() / rounds;
        let start = std::time::Instant::now();
        let mut headings = 0;
        for _ in 0..rounds {
            headings += Outline::new(&doc).headings().len();
        }
        let outline = start.elapsed() / rounds;
        println!(
            "5,000 lines: scan {scan:?}, outline {outline:?} ({} lines, {} headings)",
            kinds / rounds as usize,
            headings / rounds as usize
        );
        assert_eq!(kinds / rounds as usize, doc.line_count());
    }
}
