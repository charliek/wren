//! Backspace, Delete, and the word and line deletes (§5.5).

use unicode_segmentation::UnicodeSegmentation;

use super::*;

/// Backspace: Markdown markup first, then indentation, then one grapheme
/// cluster (§5.5).
pub(super) fn backspace<L: Lines + ?Sized>(base: &L, sel: Selection, opts: &Options) -> EditPlan {
    let sel = clamp_selection(base, sel);
    if !sel.is_empty() {
        return delete_between(sel.start(), sel.end());
    }
    let p = sel.head;
    if may_end_markup(&base.line(p.line)[..p.col]) {
        if let Some(plan) = markup_or_indentation(base, p, sel, opts) {
            return plan;
        }
    }
    let text = base.line(p.line);
    let floor = bom_len(p.line, text);
    let from = if p.col > floor {
        let prev = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|&i| i < p.col)
            .last()
            .unwrap_or(0);
        Pos::new(p.line, prev.max(floor))
    } else if p.line > 0 {
        Pos::new(p.line - 1, base.line(p.line - 1).len())
    } else {
        p
    };
    delete_between(from, p)
}

/// Whether `before`, a line's text before the cursor, could all be
/// indentation and markup: only then can rules 1 and 2 apply, and only then
/// is the document scanned.
pub(super) fn may_end_markup(before: &str) -> bool {
    before.chars().all(|c| {
        c.is_whitespace()
            || c.is_ascii_digit()
            || matches!(
                c,
                '>' | '-' | '*' | '+' | '.' | ')' | '[' | ']' | 'x' | 'X' | '#' | '\u{feff}'
            )
    })
}

/// Rules 1 and 2 at the caret `p`, when one applies.
pub(super) fn markup_or_indentation<L: Lines + ?Sized>(
    base: &L,
    p: Pos,
    sel: Selection,
    opts: &Options,
) -> Option<EditPlan> {
    let an = Analysis::new(base, Some(p.line));
    let mut d = Draft::new(base);
    let mut marks = sel;
    let mut runs = Vec::new();
    let mut requests = Vec::new();
    if markup(&mut d, &an, p, &mut marks, &mut runs, &mut requests)
        || indentation(&mut d, &an, p, opts.indent_width, &mut marks)
    {
        if opts.renumber {
            for request in requests {
                renumber(&mut d, &an, request, &mut marks);
            }
            renumber_runs(&mut d, &runs, &[], &mut marks);
        }
        return Some(finish(base, &d, marks));
    }
    None
}

/// Rule 1: the cursor sits exactly at the end of the line's markup.
fn markup(
    d: &mut Draft,
    an: &Analysis,
    p: Pos,
    marks: &mut Selection,
    runs: &mut Vec<Run>,
    requests: &mut Vec<Renumber>,
) -> bool {
    let l = p.line;
    let text = d.line(l).to_owned();
    let probed = as_empty_item(an, l, &text);
    let (info, stack, continued) = probed.clone().unwrap_or_else(|| {
        let rec = &an.recs[l];
        (rec.info.clone(), an.stack(l), rec.continued)
    });
    if info.lazy || info.kind.is_code_like() || p.col != info.text_start {
        return false;
    }
    let segs = block::segments(&text, l, &stack, continued);
    let (Some(&seg), Some(&container)) = (segs.last(), stack.last()) else {
        return false;
    };
    let Some(marker) = seg.marker else {
        return false;
    };
    let slot = segs.len() - 1;
    let canonical = match container {
        Container::Quote { .. } => after_space(&text, marker + 1),
        Container::Item(item) => after_space(
            &text,
            item.task.map_or(item.marker_end, |task| task.start + 3),
        ),
    };
    if canonical < p.col {
        edit_line(d, l, canonical..p.col, "", marks);
        return true;
    }
    let item = match container {
        Container::Quote { .. } => {
            edit_line(d, l, seg.start..p.col, "", marks);
            return true;
        }
        Container::Item(item) => item,
    };
    if let Some(task) = item.task {
        edit_line(d, l, task.start..p.col, "", marks);
        return true;
    }
    if let Some(Container::Item(parent)) = slot.checked_sub(1).map(|s| stack[s]) {
        if probed.is_none() {
            return tab::outdent_item(d, an, l, slot, marks, runs);
        }
        // An empty marker under a paragraph line, which only reads as an
        // item to the writer: it moves out as Enter would move it.
        let ctx = Line {
            l,
            line: &text,
            bom: bom_len(l, &text),
            c: p.col,
            left: String::new(),
            segs: &segs,
            stack: &stack,
        };
        if let Some((caret, found)) = outdent(d, an, &ctx, slot, item, parent) {
            *marks = Selection::caret(caret);
            requests.extend(found);
            return true;
        }
        return false;
    }
    match an.prev_sibling(l, slot) {
        // The marker becomes the indentation of a continuation paragraph
        // of the previous item.
        Some((_, prev)) => {
            let (range, pad) = reindent(
                &text[..item.marker_start],
                seg.start,
                prev.content_column,
                false,
            );
            edit_line(d, l, range.start..p.col, &pad, marks);
        }
        None => edit_line(d, l, seg.start..p.col, "", marks),
    }
    if let Marker::Ordered {
        number, delimiter, ..
    } = item.marker
    {
        if !all_same(an, l, slot, &item) {
            requests.push(Renumber {
                after: l,
                slot,
                delimiter,
                next: number,
                expect: Some(number + 1),
            });
        }
    }
    true
}

/// Byte `i`, or one past it when a space or tab is there.
fn after_space(text: &str, i: usize) -> usize {
    match text.as_bytes().get(i) {
        Some(b' ' | b'\t') => i + 1,
        _ => i,
    }
}

/// Rule 2: only whitespace before the cursor and nothing after it. The
/// indentation goes back to the previous indent stop.
fn indentation(d: &mut Draft, an: &Analysis, p: Pos, width: usize, marks: &mut Selection) -> bool {
    let l = p.line;
    let text = d.line(l).to_owned();
    if p.col != text.len() || an.recs[l].info.kind.is_code_like() {
        return false;
    }
    let at = an.line_at(l, &text);
    if p.col < at.ws || !is_ws(&text[at.ws..]) {
        return false;
    }
    let base = block::column(&text, at.ws);
    let cur = block::column(&text, p.col);
    if cur <= base {
        return false;
    }
    let target = previous_stop(cur, base, &at.item_stops(), width);
    let (range, pad) = reindent(&text, at.ws, target, false);
    edit_line(d, l, range, &pad, marks);
    true
}

/// Delete one grapheme cluster forward.
pub(super) fn forward<L: Lines + ?Sized>(base: &L, sel: Selection) -> EditPlan {
    let sel = clamp_selection(base, sel);
    if !sel.is_empty() {
        return delete_between(sel.start(), sel.end());
    }
    let p = sel.head;
    let text = base.line(p.line);
    let to = if p.col < text.len() {
        let next = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|&i| i > p.col)
            .unwrap_or(text.len());
        Pos::new(p.line, next)
    } else if p.line + 1 < base.line_count() {
        Pos::new(p.line + 1, 0)
    } else {
        p
    };
    delete_between(p, to)
}

/// Delete back one group (§5.5): the whitespace before the cursor, then
/// one word or one run of punctuation; at the start of a line, the line
/// break.
pub(super) fn word_backward<L: Lines + ?Sized>(base: &L, sel: Selection) -> EditPlan {
    let sel = clamp_selection(base, sel);
    if !sel.is_empty() {
        return delete_between(sel.start(), sel.end());
    }
    let p = sel.head;
    let text = base.line(p.line);
    let floor = bom_len(p.line, text);
    if p.col <= floor {
        return join_back(base, p);
    }
    let segments: Vec<(usize, &str)> = text[floor..p.col].split_word_bound_indices().collect();
    let classes: Vec<Class> = segments.iter().map(|s| Class::of(s.1)).collect();
    let mut i = classes.len();
    while i > 0 && classes[i - 1] == Class::Space {
        i -= 1;
    }
    match i.checked_sub(1).map(|last| classes[last]) {
        Some(Class::Word) => i -= 1,
        Some(Class::Punctuation) => {
            while i > 0 && classes[i - 1] == Class::Punctuation {
                i -= 1;
            }
        }
        Some(Class::Space) | None => {}
    }
    let from = floor + segments.get(i).map_or(p.col - floor, |s| s.0);
    delete_between(Pos::new(p.line, from), p)
}

/// Delete forward one group: the whitespace after the cursor, then one
/// word or one run of punctuation; at the end of a line, the line break.
pub(super) fn word_forward<L: Lines + ?Sized>(base: &L, sel: Selection) -> EditPlan {
    let sel = clamp_selection(base, sel);
    if !sel.is_empty() {
        return delete_between(sel.start(), sel.end());
    }
    let p = sel.head;
    let text = base.line(p.line);
    if p.col == text.len() {
        return forward(base, sel);
    }
    let rest = &text[p.col..];
    let mut segments = rest
        .split_word_bound_indices()
        .map(|(i, s)| (i + s.len(), Class::of(s)))
        .skip_while(|&(_, class)| class == Class::Space)
        .peekable();
    let to = match segments.next() {
        Some((end, Class::Word)) => end,
        Some((mut end, Class::Punctuation)) => {
            while let Some(&(next, Class::Punctuation)) = segments.peek() {
                end = next;
                segments.next();
            }
            end
        }
        Some((_, Class::Space)) | None => rest.len(),
    };
    delete_between(p, Pos::new(p.line, p.col + to))
}

/// The character class of a word-bound segment, for the group deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    /// A UAX #29 word: any segment with a letter or digit in it.
    Word,
    /// Everything else, emoji included; consecutive segments form one run.
    Punctuation,
}

impl Class {
    fn of(segment: &str) -> Class {
        if segment.chars().all(char::is_whitespace) {
            Class::Space
        } else if segment.chars().any(char::is_alphanumeric) {
            Class::Word
        } else {
            Class::Punctuation
        }
    }
}

/// Delete back to the line's text start, or from there (or from inside the
/// markup) to column 0; at column 0, the line break.
pub(super) fn to_text_start<L: Lines + ?Sized>(base: &L, sel: Selection) -> EditPlan {
    let sel = clamp_selection(base, sel);
    if !sel.is_empty() {
        return delete_between(sel.start(), sel.end());
    }
    let p = sel.head;
    let text = base.line(p.line);
    let floor = bom_len(p.line, text);
    if p.col <= floor {
        return join_back(base, p);
    }
    let text_start = super::motion::text_start(base, p.line, floor);
    let from = if p.col > text_start {
        text_start
    } else {
        floor
    };
    delete_between(Pos::new(p.line, from), p)
}

/// Delete the line break before line `p.line`, if there is one.
fn join_back<L: Lines + ?Sized>(base: &L, p: Pos) -> EditPlan {
    if p.line == 0 {
        return unchanged(Selection::caret(p));
    }
    let from = Pos::new(p.line - 1, base.line(p.line - 1).len());
    delete_between(from, p)
}

/// Delete the text between two positions, leaving the cursor there. The
/// edit is made directly, without a draft of the whole document.
pub(super) fn delete_between(from: Pos, to: Pos) -> EditPlan {
    EditPlan {
        edits: if from < to {
            vec![Replace {
                start: from,
                end: to,
                text: String::new(),
            }]
        } else {
            Vec::new()
        },
        selection: Selection::caret(from),
    }
}

#[cfg(test)]
mod tests {
    use crate::edit::tests::{check, check_with};
    use crate::{plan, Document, Intent, Lines, Options, Pos, Selection};

    #[test]
    fn backspace_at_the_end_of_markup_takes_extra_spaces_first() {
        check(
            Intent::Backspace,
            &[
                (">   |", "> |"),
                ("> -    |", "> - |"),
                ("-   |a", "- |a"),
                ("- [ ]   |a", "- [ ] |a"),
                (">\t |a", ">\t|a"),
            ],
        );
    }

    #[test]
    fn backspace_removes_a_task_box() {
        check(
            Intent::Backspace,
            &[
                ("- [ ] |", "- |"),
                ("- [x] |foo", "- |foo"),
                ("- [ ]|", "- |"),
                ("1. [X] |a", "1. |a"),
                ("> - [ ] |", "> - |"),
            ],
        );
    }

    #[test]
    fn backspace_outdents_a_nested_item() {
        check(
            Intent::Backspace,
            &[
                ("- a\n  - |b", "- a\n- |b"),
                ("- a\n  - b\n    - |c", "- a\n  - b\n  - |c"),
                ("- a\n  - b\n    - |", "- a\n  - b\n  - |"),
                ("1. a\n   1. |x\n2. b", "1. a\n2. |x\n3. b"),
                ("> - a\n>   - |b", "> - a\n> - |b"),
            ],
        );
    }

    #[test]
    fn backspace_turns_a_later_item_into_a_continuation() {
        check(
            Intent::Backspace,
            &[
                ("- a\n- |b", "- a\n  |b"),
                ("- a\n- |", "- a\n  |"),
                ("1. a\n2. |b\n3. c", "1. a\n   |b\n2. c"),
                ("1. a\n1. |b\n1. c", "1. a\n   |b\n1. c"),
                ("1. a\n2. |b\n5. c", "1. a\n   |b\n5. c"),
                ("1. a\n2. |b\n3. c\n7. d", "1. a\n   |b\n2. c\n7. d"),
                ("- a\n\n- |b", "- a\n\n  |b"),
                ("10. a\n11. |b", "10. a\n    |b"),
            ],
        );
    }

    #[test]
    fn backspace_removes_a_first_item_marker_or_a_quote_level() {
        check(
            Intent::Backspace,
            &[
                ("- |", "|"),
                ("- |a", "|a"),
                ("  - |a", "|a"),
                ("1. |a\n2. b", "|a\n1. b"),
                ("> - |", "> |"),
                ("> |", "|"),
                ("> |a", "|a"),
                ("> > |", "> |"),
                (">|", "|"),
                ("> a\n> |b", "> a\n|b"),
            ],
        );
    }

    #[test]
    fn backspace_in_blank_indentation_goes_back_one_stop() {
        check(
            Intent::Backspace,
            &[
                ("- a\n  |", "- a\n|"),
                ("- a\n    |", "- a\n  |"),
                ("- a\n  - b\n      |", "- a\n  - b\n    |"),
                ("    |", "|"),
                ("      |", "    |"),
                ("\t|", "|"),
                ("> - a\n>     |", "> - a\n>   |"),
                ("x\n   |", "x\n|"),
            ],
        );
    }

    #[test]
    fn backspace_otherwise_deletes_one_grapheme() {
        check(
            Intent::Backspace,
            &[
                ("ab|", "a|"),
                ("é|", "|"),
                ("e\u{301}|x", "|x"),
                ("👨\u{200d}👩\u{200d}👧|", "|"),
                ("🇫🇷|", "|"),
                ("a\n|b", "a|b"),
                ("|", "|"),
                ("|a", "|a"),
                ("- a\n  |b", "- a\n |b"),
                ("```\n    |", "```\n   |"),
                ("> q\nrs|t", "> q\nr|t"),
                ("\u{feff}|a", "\u{feff}|a"),
                ("\u{feff}a|", "\u{feff}|"),
            ],
        );
    }

    #[test]
    fn backspace_and_delete_remove_a_selection() {
        for intent in [
            Intent::Backspace,
            Intent::Delete,
            Intent::DeleteWordBackward,
            Intent::DeleteWordForward,
            Intent::DeleteToTextStart,
        ] {
            check(
                intent,
                &[
                    ("a«bc»d", "a|d"),
                    ("a»bc«d", "a|d"),
                    ("- a«\n- »b", "- a|b"),
                    ("«\u{feff}ab»", "\u{feff}|"),
                ],
            );
        }
    }

    #[test]
    fn backspace_renumbering_can_be_turned_off() {
        let opts = Options {
            renumber: false,
            ..Options::default()
        };
        check_with(
            Intent::Backspace,
            &opts,
            &[("1. a\n2. |b\n3. c", "1. a\n   |b\n3. c")],
        );
    }

    #[test]
    fn delete_removes_one_grapheme_forward() {
        check(
            Intent::Delete,
            &[
                ("|ab", "|b"),
                ("|e\u{301}x", "|x"),
                ("|👨\u{200d}👩\u{200d}👧!", "|!"),
                ("a|\nb", "a|b"),
                ("|\u{feff}a", "\u{feff}|"),
            ],
        );
    }

    #[test]
    fn word_deletes_take_whitespace_then_one_group() {
        check(
            Intent::DeleteWordBackward,
            &[
                ("foo bar|", "foo |"),
                ("foo bar  |", "foo |"),
                ("foo |", "|"),
                ("foo, bar|", "foo, |"),
                ("foo,|", "foo|"),
                ("foo!!|", "foo|"),
                ("foo ?!|", "foo |"),
                ("**bold**|", "**bold|"),
                ("**bold|", "**|"),
                ("- [ ] |", "- [ |"),
                ("- [ |", "- |"),
                ("- |", "|"),
                ("snake_case|", "|"),
                ("can't|", "|"),
                ("a.b|", "|"),
                ("3.14|", "|"),
                ("foo ba|r", "foo |r"),
                ("  |foo", "|foo"),
                ("a\n|b", "a|b"),
                ("a\n  |b", "a\n|b"),
                ("|", "|"),
                ("\u{feff}ab|", "\u{feff}|"),
                ("\u{feff}|ab", "\u{feff}|ab"),
            ],
        );
        check(
            Intent::DeleteWordForward,
            &[
                ("|foo bar", "| bar"),
                ("|  bar baz", "| baz"),
                ("f|oo bar", "f| bar"),
                ("|,foo", "|foo"),
                ("|!!foo", "|foo"),
                ("|** bold", "| bold"),
                ("|**bold**", "|bold**"),
                ("| [ ] a", "| ] a"),
                ("|a.b c", "| c"),
                ("|3.14!", "|!"),
                ("|can't stop", "| stop"),
                ("a|\nb", "a|b"),
                ("a|  \nb", "a|\nb"),
            ],
        );
    }

    #[test]
    fn word_deletes_group_cjk_and_emoji_by_segment() {
        // Without a dictionary, UAX #29 makes each ideograph a word of its
        // own; an emoji is a symbol, so a run of them goes together, and a
        // joined sequence is never split.
        check(
            Intent::DeleteWordBackward,
            &[
                ("日本語|", "日本|"),
                ("foo 日本|", "foo 日|"),
                ("カタカナ|", "|"),
                ("ok👍👍|", "ok|"),
                ("ok 👨\u{200d}👩\u{200d}👧|", "ok |"),
                ("🇫🇷!|", "|"),
                ("e\u{301}te\u{301}|", "|"),
            ],
        );
        check(
            Intent::DeleteWordForward,
            &[
                ("|日本語", "|本語"),
                ("|👍👍ok", "|ok"),
                ("|👨\u{200d}👩\u{200d}👧 a", "| a"),
                ("|e\u{301}te\u{301}!", "|!"),
            ],
        );
    }

    #[test]
    fn delete_to_text_start_then_to_column_0() {
        check(
            Intent::DeleteToTextStart,
            &[
                ("- [ ] foo|", "- [ ] |"),
                ("- [ ] |", "|"),
                ("- [ ] fo|o", "- [ ] |o"),
                ("  |- a", "|- a"),
                ("> quote|", "> |"),
                ("# Title|", "# |"),
                ("plain text|", "|"),
                ("a\n|b", "a|b"),
                ("|", "|"),
                ("\u{feff}- a|", "\u{feff}- |"),
            ],
        );
    }

    #[test]
    fn forward_deletes_at_the_document_end_change_nothing() {
        for intent in [Intent::Delete, Intent::DeleteWordForward] {
            for text in ["", "a", "a\nb"] {
                let doc = Document::new(text);
                let end = Pos::new(doc.line_count() - 1, doc.line(doc.line_count() - 1).len());
                let plan = plan(&doc, Selection::caret(end), intent, &Options::default());
                assert!(plan.edits.is_empty(), "{intent:?} at the end of {text:?}");
                assert_eq!(plan.selection, Selection::caret(end));
            }
        }
    }

    mod codemirror {
        // The cases of CodeMirror's Markdown Backspace command tests, ported
        // to Rust. Where Wren's spec decides differently, the case asserts
        // Wren's behavior and is listed in `discovery/markdown-editor.md`
        // §7.1.
        //
        // Ported from `test/test-commands.ts` (the `deleteMarkupBackward`
        // suite) of `@codemirror/lang-markdown`,
        // <https://github.com/codemirror/lang-markdown>, at commit
        // f90daf6e56a72f1cf8bbe10b07867f535d0ad2e4, under the MIT license.
        // Modified by Wren: converted to Rust table tests, with expectations
        // changed where noted. The multiple-cursor case is dropped, because
        // Wren's editor has one cursor.
        //
        // MIT License
        //
        // Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin>
        // and others
        //
        // Permission is hereby granted, free of charge, to any person obtaining
        // a copy of this software and associated documentation files (the
        // "Software"), to deal in the Software without restriction, including
        // without limitation the rights to use, copy, modify, merge, publish,
        // distribute, sublicense, and/or sell copies of the Software, and to
        // permit persons to whom the Software is furnished to do so, subject to
        // the following conditions:
        //
        // The above copyright notice and this permission notice shall be
        // included in all copies or substantial portions of the Software.
        //
        // THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
        // EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
        // MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
        // NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
        // LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
        // OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
        // WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

        use super::*;

        fn backspace(cases: &[(&str, &str)]) {
            check(Intent::Backspace, cases);
        }

        #[test]
        fn does_nothing_in_regular_text() {
            // Divergence: CodeMirror declines; Wren deletes a grapheme.
            backspace(&[("one|", "on|")]);
        }

        #[test]
        fn does_nothing_at_the_top_level() {
            // Divergence: CodeMirror declines; Wren joins the lines.
            backspace(&[("one\n|", "one|")]);
        }

        #[test]
        fn can_delete_blockquote_markers() {
            backspace(&[("> |", "|")]);
        }

        #[test]
        fn only_deletes_one_marker_at_a_time() {
            backspace(&[("> > |", "> |")]);
        }

        #[test]
        fn deletes_trailing_whitespace() {
            backspace(&[(">   |", "> |")]);
        }

        #[test]
        fn clears_list_markers() {
            backspace(&[(" - one\n - |", " - one\n   |")]);
        }

        #[test]
        fn removes_extra_indentation() {
            backspace(&[("> - one\n> -    |", "> - one\n> - |")]);
        }

        #[test]
        fn clears_triple_space_indentation() {
            backspace(&[(" - one\n   |", " - one\n|")]);
        }

        #[test]
        fn clears_one_level_of_indentation() {
            backspace(&[("- one\n    - two\n      |", "- one\n    - two\n  |")]);
        }

        #[test]
        fn deletes_the_first_list_marker_immediately() {
            backspace(&[(" - |", "|")]);
        }

        #[test]
        fn clears_number_markers_in_one_go() {
            backspace(&[("1. one\n2. |", "1. one\n   |")]);
        }

        #[test]
        fn deletes_nested_list_markers() {
            backspace(&[(" > - |", " > |")]);
        }

        #[test]
        fn does_nothing_in_a_continued_list_item() {
            // Divergence: CodeMirror declines; Wren deletes a grapheme.
            backspace(&[("- Foo\n-\n  |Welcome", "- Foo\n-\n |Welcome")]);
        }

        #[test]
        fn doesnt_delete_normal_text_in_continued_list_items() {
            // Divergence: CodeMirror declines; Wren deletes a grapheme.
            backspace(&[("- \na |b", "- \na|b")]);
        }

        #[test]
        fn normalizes_whitespace_on_deleting() {
            // Divergence: CodeMirror indents with its tab indent unit; Wren
            // keeps spaces, since the line's indentation has no tab.
            backspace(&[("  - one\n  - |", "  - one\n    |")]);
        }

        #[test]
        fn doesnt_kick_in_in_a_continued_paragraph_under_a_quote() {
            // Divergence: CodeMirror declines; Wren deletes a grapheme.
            backspace(&[("> q\nrs|t", "> q\nr|t")]);
        }
    }
}
