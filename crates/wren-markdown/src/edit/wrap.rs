//! `markdown.bold`, `markdown.italic`, and `markdown.code` (§5.7).

use unicode_segmentation::UnicodeSegmentation;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    Bold,
    Italic,
    Code,
}

/// Wrap the selection (or the word at the cursor) in `style`'s markers, or
/// unwrap it when it is already wrapped.
pub(super) fn toggle<L: Lines + ?Sized>(base: &L, sel: Selection, style: Style) -> EditPlan {
    let sel = clamp_selection(base, sel);
    let an = Analysis::new(base, None);
    let mut d = Draft::new(base);
    let (start, end) = (sel.start(), sel.end());
    let forward = sel.anchor <= sel.head;
    let floor = |j: usize| {
        let info = &an.recs[j].info;
        (!info.kind.is_code_like()).then(|| info.text_start.max(bom_len(j, base.line(j))))
    };
    if start.line == end.line {
        let l = start.line;
        let Some(floor) = floor(l) else {
            return unchanged(sel);
        };
        let text = base.line(l);
        let s = start.col.max(floor);
        let e = end.col.max(s);
        if s == e {
            let span = word_at(text, s, floor).unwrap_or(s..s);
            return caret(base, &mut d, Pos::new(l, s), span, style);
        }
        let (s, e) = trim(text, s, e);
        if s == e {
            return caret(base, &mut d, Pos::new(l, s), s..s, style);
        }
        let (line, range) = apply(text, s..e, find(text, s..e, style), style);
        d.set(l, line);
        let selection = oriented(Pos::new(l, range.start), Pos::new(l, range.end), forward);
        return finish(base, &d, selection);
    }
    let (_, last) = covered(&sel);
    let mut spans = Vec::new();
    for j in start.line..=last {
        let Some(floor) = floor(j) else {
            continue;
        };
        let text = base.line(j);
        let s = if j == start.line { start.col } else { 0 }.max(floor);
        let e = if j == end.line { end.col } else { text.len() }.max(s);
        let (s, e) = trim(text, s, e);
        if s < e {
            spans.push((j, s..e));
        }
    }
    let (Some(first), Some(last)) = (spans.first().cloned(), spans.last().cloned()) else {
        return unchanged(sel);
    };
    let unwrap_all = spans
        .iter()
        .all(|(j, r)| find(base.line(*j), r.clone(), style).is_some());
    let mut ranges = Vec::new();
    for (j, range) in spans {
        let text = base.line(j);
        let found = find(text, range.clone(), style);
        let (line, range) = match (unwrap_all, found) {
            (true, found @ Some(_)) => apply(text, range, found, style),
            (false, None) => apply(text, range, None, style),
            _ => (text.to_owned(), range),
        };
        d.set(j, line);
        ranges.push(range);
    }
    // The selection keeps to the wrapped text; an end on a line left alone
    // stays where it was.
    let new_start = if first.0 == start.line {
        Pos::new(first.0, ranges[0].start)
    } else {
        start
    };
    let new_end = if last.0 == end.line {
        Pos::new(last.0, ranges[ranges.len() - 1].end)
    } else {
        end
    };
    finish(base, &d, oriented(new_start, new_end, forward))
}

/// No selection: `span`, the word at the cursor, is toggled, or when it is
/// empty a pair goes in around the cursor (or comes out, when the cursor
/// is inside an empty one). The cursor keeps its place in the text.
fn caret<L: Lines + ?Sized>(
    base: &L,
    d: &mut Draft,
    pos: Pos,
    span: Range<usize>,
    style: Style,
) -> EditPlan {
    let text = base.line(pos.line);
    let found = find(text, span.clone(), style);
    let (line, range) = apply(text, span.clone(), found, style);
    let col = match found {
        // The cursor stays on its character, or at the edge of the text
        // when it was among the markers.
        Some(Found {
            outer,
            inside: true,
        }) => {
            range.start
                + (pos.col - span.start)
                    .saturating_sub(outer)
                    .min(range.len())
        }
        _ => range.start + (pos.col - span.start),
    };
    d.set(pos.line, line);
    finish(base, d, Selection::caret(Pos::new(pos.line, col)))
}

/// The word around byte `c` (or touching it, preferring the one before),
/// by Unicode word boundaries.
fn word_at(text: &str, c: usize, floor: usize) -> Option<Range<usize>> {
    let mut found = None;
    for (i, segment) in text.split_word_bound_indices() {
        let end = i + segment.len();
        if i > c {
            break;
        }
        if i < floor || end < c || !segment.chars().any(char::is_alphanumeric) {
            continue;
        }
        if i < c && c < end {
            return Some(i..end);
        }
        found.get_or_insert(i..end);
    }
    found
}

/// `s..e` without spaces and tabs at either end.
fn trim(text: &str, mut s: usize, mut e: usize) -> (usize, usize) {
    let b = text.as_bytes();
    while s < e && matches!(b[s], b' ' | b'\t') {
        s += 1;
    }
    while e > s && matches!(b[e - 1], b' ' | b'\t') {
        e -= 1;
    }
    (s, e)
}

fn oriented(start: Pos, end: Pos, forward: bool) -> Selection {
    if forward {
        Selection::new(start, end)
    } else {
        Selection::new(end, start)
    }
}

/// Markers already around a span: `outer` bytes to remove before it and
/// after it, and whether they sit just inside the span rather than just
/// outside it.
#[derive(Clone, Copy, Debug)]
struct Found {
    outer: usize,
    inside: bool,
}

/// How many `ch` bytes run backwards from `at`, not past `limit`.
fn run_back(b: &[u8], at: usize, ch: u8, limit: usize) -> usize {
    b[limit..at].iter().rev().take_while(|&&x| x == ch).count()
}

/// How many `ch` bytes run forwards from `at`, not past `limit`.
fn run_on(b: &[u8], at: usize, ch: u8, limit: usize) -> usize {
    b[at..limit].iter().take_while(|&&x| x == ch).count()
}

/// Whether `style` already wraps `span`, with its markers just outside or
/// just inside it. `*` and `_` count alike; a run of three is both bold
/// and italic.
fn find(text: &str, span: Range<usize>, style: Style) -> Option<Found> {
    let b = text.as_bytes();
    let (s, e) = (span.start, span.end);
    match style {
        Style::Bold | Style::Italic => {
            let (len, has): (usize, fn(usize) -> bool) = match style {
                Style::Bold => (2, |n| n >= 2),
                _ => (1, |n| n % 2 == 1),
            };
            for ch in *b"*_" {
                let n = run_back(b, s, ch, 0).min(run_on(b, e, ch, b.len()));
                if has(n) {
                    return Some(Found {
                        outer: len,
                        inside: false,
                    });
                }
                let n = run_on(b, s, ch, e)
                    .min(run_back(b, e, ch, s))
                    .min((e - s) / 2);
                if has(n) && e - s > 2 * len {
                    return Some(Found {
                        outer: len,
                        inside: true,
                    });
                }
            }
            None
        }
        Style::Code => {
            let (before, after) = (run_back(b, s, b'`', 0), run_on(b, e, b'`', b.len()));
            if before > 0 && before == after {
                return Some(Found {
                    outer: before,
                    inside: false,
                });
            }
            let padded = s > 0 && e < b.len() && b[s - 1] == b' ' && b[e] == b' ';
            if padded {
                let (before, after) =
                    (run_back(b, s - 1, b'`', 0), run_on(b, e + 1, b'`', b.len()));
                if before > 0 && before == after {
                    return Some(Found {
                        outer: before + 1,
                        inside: false,
                    });
                }
            }
            let (open, close) = (run_on(b, s, b'`', e), run_back(b, e, b'`', s));
            if open > 0 && open == close && 2 * open < e - s {
                let inner = &text[s + open..e - open];
                let pad = usize::from(
                    inner.len() > 2
                        && inner.starts_with(' ')
                        && inner.ends_with(' ')
                        && !inner.trim().is_empty(),
                );
                return Some(Found {
                    outer: open + pad,
                    inside: true,
                });
            }
            None
        }
    }
}

/// The line with `span` unwrapped (when `found`) or wrapped, and the new
/// range of the span's text.
fn apply(
    text: &str,
    span: Range<usize>,
    found: Option<Found>,
    style: Style,
) -> (String, Range<usize>) {
    let (s, e) = (span.start, span.end);
    match found {
        Some(Found {
            outer,
            inside: false,
        }) => (
            format!(
                "{}{}{}",
                &text[..s - outer],
                &text[s..e],
                &text[e + outer..]
            ),
            s - outer..e - outer,
        ),
        Some(Found {
            outer,
            inside: true,
        }) => (
            format!(
                "{}{}{}",
                &text[..s],
                &text[s + outer..e - outer],
                &text[e..]
            ),
            s..e - 2 * outer,
        ),
        None => {
            let content = &text[s..e];
            let (open, close) = match style {
                Style::Bold => ("**".to_owned(), "**".to_owned()),
                Style::Italic => ("*".to_owned(), "*".to_owned()),
                Style::Code => {
                    let longest = content.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                    let fence = "`".repeat(longest + 1);
                    if longest > 0 {
                        (format!("{fence} "), format!(" {fence}"))
                    } else {
                        (fence.clone(), fence)
                    }
                }
            };
            (
                format!("{}{open}{content}{close}{}", &text[..s], &text[e..]),
                s + open.len()..e + open.len(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::edit::tests::check;
    use crate::Intent;

    #[test]
    fn bold_wraps_and_unwraps() {
        check(
            Intent::Bold,
            &[
                ("«foo»", "**«foo»**"),
                ("»foo«", "**»foo«**"),
                ("a «foo» b", "a **«foo»** b"),
                ("fo|o", "**fo|o**"),
                ("|foo bar", "**|foo** bar"),
                ("foo| bar", "**foo|** bar"),
                ("a | b", "a **|** b"),
                ("|", "**|**"),
                ("**|**", "|"),
                ("**fo|o**", "fo|o"),
                ("__fo|o__", "fo|o"),
                ("|__foo__", "|foo"),
                ("__foo|__", "foo|"),
                ("«**foo**»", "«foo»"),
                ("**«foo»**", "«foo»"),
                ("« foo »", " **«foo»** "),
                ("«- foo»", "- **«foo»**"),
                ("- [ ] «a b»", "- [ ] **«a b»**"),
                ("*«foo»*", "***«foo»***"),
                ("***«foo»***", "*«foo»*"),
                ("« »", " **|**"),
                ("snake_ca|se", "**snake_ca|se**"),
                ("日|本", "**日|**本"),
            ],
        );
    }

    #[test]
    fn italic_wraps_and_unwraps() {
        check(
            Intent::Italic,
            &[
                ("«foo»", "*«foo»*"),
                ("*«foo»*", "«foo»"),
                ("_«foo»_", "«foo»"),
                ("«*foo*»", "«foo»"),
                ("**«foo»**", "***«foo»***"),
                ("***«foo»***", "**«foo»**"),
                ("*fo|o*", "fo|o"),
                ("|", "*|*"),
                ("*|*", "|"),
            ],
        );
    }

    #[test]
    fn code_wraps_with_a_fence_longer_than_any_backtick_run() {
        check(
            Intent::Code,
            &[
                ("«foo»", "`«foo»`"),
                ("`«foo»`", "«foo»"),
                ("«`foo`»", "«foo»"),
                ("`fo|o`", "fo|o"),
                ("«a`b»", "`` «a`b» ``"),
                ("`` «a`b» ``", "«a`b»"),
                ("«`` a`b ``»", "«a`b»"),
                ("«a``b`c»", "``` «a``b`c» ```"),
                ("|", "`|`"),
            ],
        );
    }

    #[test]
    fn wraps_each_line_of_a_multi_line_selection() {
        check(
            Intent::Bold,
            &[
                ("«foo\nbar»", "**«foo**\n**bar»**"),
                ("f«oo\nba»r", "f**«oo**\n**ba»**r"),
                ("«- a\n\n- b»", "- **«a**\n\n- **b»**"),
                ("«**foo**\n**bar**»", "«foo\nbar»"),
                ("«**foo**\nbar»", "«**foo**\n**bar»**"),
                ("«foo\n»bar", "**«foo**\n»bar"),
                ("«a\n```\nb»", "**«a**\n```\nb»"),
            ],
        );
    }

    #[test]
    fn wraps_leave_code_alone() {
        check(
            Intent::Bold,
            &[("```\n«x»", "```\n«x»"), ("    co|de", "    co|de")],
        );
    }
}
