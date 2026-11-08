//! The editing planner: an [`Intent`] at a selection becomes an
//! [`EditPlan`].
//!
//! Planning works on a draft, a copy-on-write view of the document's
//! lines. The planner first deletes the selection, then rewrites whole
//! lines of the draft, then renumbers ordered lists by rescanning it. The
//! plan is the difference between the draft and the original lines, so
//! every step can reason in plain lines and the result is still a few
//! small replacements.

use std::borrow::Cow;
use std::ops::Range;

use crate::block::{self, Container, Kind, LineInfo, ListItem, Marker, Scanner, Segment};
use crate::{EditPlan, Intent, Lines, Options, Pos, Replace, Selection};

mod delete;
mod motion;
mod tab;

/// Plan `intent` at `sel`.
///
/// Every intent gets a plan, so the editor's own handling of these keys,
/// which knows nothing of line endings or grapheme clusters, never runs. A
/// plan with no edits and the same selection means the key was consumed
/// with nothing to do (Tab with no previous sibling, a line move at the
/// top of the document). Motions (Home and the document start and end)
/// only move the selection.
pub fn plan(
    lines: &(impl Lines + ?Sized),
    sel: Selection,
    intent: Intent,
    opts: &Options,
) -> EditPlan {
    match intent {
        Intent::Enter => enter(lines, sel, false, opts),
        Intent::ShiftEnter => enter(lines, sel, true, opts),
        Intent::Tab => tab::indent(lines, sel, true, opts),
        Intent::ShiftTab => tab::indent(lines, sel, false, opts),
        Intent::Backspace => delete::backspace(lines, sel, opts),
        Intent::Delete => delete::forward(lines, sel),
        Intent::DeleteWordBackward => delete::word_backward(lines, sel),
        Intent::DeleteWordForward => delete::word_forward(lines, sel),
        Intent::DeleteToTextStart => delete::to_text_start(lines, sel),
        Intent::Home { extend } => motion::home(lines, sel, extend),
        Intent::DocumentStart { extend } => motion::document_start(lines, sel, extend),
        Intent::DocumentEnd { extend } => motion::document_end(lines, sel, extend),
        _ => unchanged(clamp_selection(lines, sel)),
    }
}

fn enter<L: Lines + ?Sized>(base: &L, sel: Selection, shift: bool, opts: &Options) -> EditPlan {
    let start = clamp(base, sel.start());
    let end = clamp(base, sel.end());
    let mut draft = Draft::new(base);
    draft.delete(start, end);
    let analysis = Analysis::new(&draft, Some(start.line));
    let (caret, requests) = break_line(&mut draft, &analysis, start, shift);
    let mut marks = Selection::caret(caret);
    if opts.renumber {
        for request in requests {
            let an = Analysis::new(&draft, None);
            renumber(&mut draft, &an, request, &mut marks);
        }
    }
    finish(base, &draft, marks)
}

/// The plan that turns `base` into `draft`, leaving `selection`.
fn finish<L: Lines + ?Sized>(base: &L, draft: &Draft, selection: Selection) -> EditPlan {
    EditPlan {
        edits: diff(base, draft),
        selection,
    }
}

/// A plan that changes nothing.
fn unchanged(selection: Selection) -> EditPlan {
    EditPlan {
        edits: Vec::new(),
        selection,
    }
}

/// `sel` with both ends clamped onto the document.
fn clamp_selection<L: Lines + ?Sized>(lines: &L, sel: Selection) -> Selection {
    Selection::new(clamp(lines, sel.anchor), clamp(lines, sel.head))
}

/// The first and last lines a selection covers. A selection that ends at
/// column 0 of a later line does not cover that line.
fn covered(sel: &Selection) -> (usize, usize) {
    let (start, end) = (sel.start(), sel.end());
    let last = if end.line > start.line && end.col == 0 {
        end.line - 1
    } else {
        end.line
    };
    (start.line, last)
}

/// A request to renumber the ordered items that follow line `after` in
/// the list at stack slot `slot`, starting from `next`.
#[derive(Clone, Copy, Debug)]
struct Renumber {
    after: usize,
    slot: usize,
    delimiter: char,
    next: u32,
}

/// Break line `pos.line` at `pos.col` (§5.2 and §5.3). Returns the caret
/// and the renumbering the break calls for.
fn break_line(d: &mut Draft, an: &Analysis, pos: Pos, shift: bool) -> (Pos, Vec<Renumber>) {
    let l = pos.line;
    let line = d.line(l).to_owned();
    let (info, stack, continued) = as_empty_item(an, l, &line).unwrap_or_else(|| {
        let rec = &an.recs[l];
        (rec.info.clone(), an.stack(l), rec.continued)
    });
    let segs = block::segments(&line, l, &stack, continued);
    let bom = bom_len(l, &line);
    let prefix_end = segs.last().map_or(bom, |s| s.end);
    let mut c = pos.col.max(bom);

    if c < info.text_start {
        let innermost = segs.iter().rev().find_map(|s| s.marker);
        let past_item_marker = segs.iter().zip(&stack).any(|(s, container)| {
            matches!(container, Container::Item(_)) && s.marker.is_some_and(|m| m < c)
        });
        if innermost.is_some_and(|m| c > m) || past_item_marker {
            c = info.text_start;
        } else {
            // In the indentation, before any list marker: a plain break that
            // keeps the quote markers and whitespace left of the cursor.
            let left = trim_ws_end(&line[..c]).to_owned();
            let carry = line[bom..c].to_owned();
            return (split(d, l, &line, c, left, None, carry), Vec::new());
        }
    }

    let left = left_part(&line, c, info.text_start);
    let cont = continuation(&line, &segs, &stack);
    let plain = format!("{cont}{}", carry_ws(&line, prefix_end, c));
    let structural = info.lazy
        || info.kind.is_code_like()
        || matches!(
            info.kind,
            Kind::Heading { .. } | Kind::ThematicBreak | Kind::SetextUnderline { .. }
        );
    let innermost = segs.len().checked_sub(1);
    let Some(last) = innermost.filter(|_| !structural) else {
        return (split(d, l, &line, c, left, None, plain), Vec::new());
    };
    let blank = info.kind == Kind::Blank;
    let ctx = Line {
        l,
        line: &line,
        bom,
        c,
        left,
        segs: &segs,
        stack: &stack,
    };
    match (stack[last], segs[last].marker) {
        (Container::Item(_), None) => (ctx.split(d, None, plain), Vec::new()),
        (Container::Quote { .. }, _) => {
            let has_text = c > info.text_start;
            if shift && has_text {
                let gap = trim_ws_end(&cont).to_owned();
                (ctx.split(d, Some(gap), plain), Vec::new())
            } else if blank && !shift {
                let outer = continuation(&line, &segs[..last], &stack);
                (exit(d, an, &ctx, outer), Vec::new())
            } else {
                (ctx.split(d, None, plain), Vec::new())
            }
        }
        (Container::Item(item), Some(_)) => {
            if shift {
                (ctx.split(d, None, plain), Vec::new())
            } else if blank {
                empty_item(d, an, &ctx, last, item)
            } else if c == info.text_start {
                insert_above(d, an, &ctx, last, item, info.text_start)
            } else {
                continue_item(d, an, &ctx, last, item)
            }
        }
    }
}

/// Line `l` read as an empty list item, when it holds only a list marker
/// right under a paragraph line in the same container (`- a⏎  - `).
/// CommonMark reads such a line as paragraph text or a setext underline,
/// because an empty item cannot interrupt a paragraph, but whoever typed it
/// means an item. It is scanned again as if a blank line came first.
fn as_empty_item(an: &Analysis, l: usize, line: &str) -> Option<(LineInfo, Vec<Container>, usize)> {
    let info = &an.recs[l].info;
    let paragraph_like = matches!(info.kind, Kind::Prose | Kind::SetextUnderline { .. });
    if info.lazy || !paragraph_like || !is_empty_marker(&line[info.text_start..]) {
        return None;
    }
    let mut scanner = an.before.clone()?;
    scanner.next_line(trim_ws_end(&line[..info.prefix_end]));
    let probed = scanner.next_line(line);
    probed
        .is_empty_item()
        .then(|| (probed, scanner.containers().to_vec(), scanner.continued()))
}

/// Whether `s` is a list marker alone, with an optional task box.
fn is_empty_marker(s: &str) -> bool {
    let s = trim_ws_end(s);
    let rest = match s.strip_prefix(['-', '*', '+']) {
        Some(rest) => rest,
        None => {
            let digits = s.bytes().take_while(u8::is_ascii_digit).count();
            match s.as_bytes().get(digits) {
                Some(b'.' | b')') if (1..=MAX_DIGITS).contains(&digits) => &s[digits + 1..],
                _ => return false,
            }
        }
    };
    let task = rest.trim_start_matches([' ', '\t']);
    rest.is_empty() || (task.len() < rest.len() && matches!(task, "[ ]" | "[x]" | "[X]"))
}

/// The line being broken, and where.
struct Line<'s> {
    l: usize,
    line: &'s str,
    /// Length of a byte order mark at the start of line 0.
    bom: usize,
    c: usize,
    /// What stays on the line, already trimmed when it is only markup.
    left: String,
    segs: &'s [Segment],
    stack: &'s [Container],
}

impl Line<'_> {
    fn split(&self, d: &mut Draft, gap: Option<String>, prefix: String) -> Pos {
        split(d, self.l, self.line, self.c, self.left.clone(), gap, prefix)
    }

    fn head(&self) -> &str {
        &self.line[..self.bom]
    }

    /// The prefix a new line gets inside the containers outside `slot`.
    fn outer(&self, slot: usize) -> String {
        continuation(self.line, &self.segs[..slot], self.stack)
    }
}

/// Replace line `l` with `left`, then add an optional `gap` line and a new
/// line holding `prefix` and the text right of `c`. Returns the caret,
/// just after `prefix`.
fn split(
    d: &mut Draft,
    l: usize,
    line: &str,
    c: usize,
    left: String,
    gap: Option<String>,
    prefix: String,
) -> Pos {
    let right = &line[c..];
    d.set(l, left);
    let mut at = l + 1;
    if let Some(gap) = gap {
        d.insert(at, gap);
        at += 1;
    }
    let col = prefix.len();
    d.insert(at, prefix + right);
    Pos::new(at, col)
}

/// Leave the innermost container: the line keeps only `outer`, the prefix
/// of the containers outside it. After a paragraph line a blank line is
/// left behind, so the next text does not continue that paragraph lazily.
fn exit(d: &mut Draft, an: &Analysis, ctx: &Line, outer: String) -> Pos {
    let l = ctx.l;
    let after_paragraph = l > 0 && an.recs[l - 1].info.kind == Kind::Prose;
    if after_paragraph {
        d.set(l, format!("{}{}", ctx.head(), trim_ws_end(&outer)));
        let col = outer.len();
        d.insert(l + 1, outer);
        Pos::new(l + 1, col)
    } else {
        let line = format!("{}{outer}", ctx.head());
        let col = line.len();
        d.set(l, line);
        Pos::new(l, col)
    }
}

/// Enter on an item holding only its marker: a nested item moves out one
/// level, an outermost one loses its marker.
fn empty_item(
    d: &mut Draft,
    an: &Analysis,
    ctx: &Line,
    slot: usize,
    item: ListItem,
) -> (Pos, Vec<Renumber>) {
    if let Some(Container::Item(parent)) = slot.checked_sub(1).map(|s| ctx.stack[s]) {
        if let Some(done) = outdent(d, an, ctx, slot, item, parent) {
            return done;
        }
    }
    let caret = exit(d, an, ctx, ctx.outer(slot));
    let mut requests = Vec::new();
    if let Marker::Ordered {
        number, delimiter, ..
    } = item.marker
    {
        if !all_same(an, ctx.l, slot, &item) {
            let prev = an
                .prev_sibling(ctx.l, slot)
                .filter(|(_, s)| same_list(s, &item));
            let (after, next) = match prev.map(|(p, s)| (p, s.marker)) {
                Some((p, Marker::Ordered { number, .. })) => (p, number + 1),
                _ => (caret.line, number),
            };
            requests.push(Renumber {
                after,
                slot,
                delimiter,
                next,
            });
        }
    }
    (caret, requests)
}

/// Move an empty nested item out to its parent's level, taking the marker
/// Enter at the end of the parent would produce. `None` when that marker
/// would not be a valid list marker.
fn outdent(
    d: &mut Draft,
    an: &Analysis,
    ctx: &Line,
    slot: usize,
    item: ListItem,
    parent: ListItem,
) -> Option<(Pos, Vec<Renumber>)> {
    let (first, _) = an.opened(ctx.l, slot - 1);
    let parent_line = d.line(first).to_owned();
    let (marker, number, same) = next_marker(an, first, slot - 1, &parent)?;
    let indent = indent_to(ctx.line, ctx.segs[slot - 1].start, parent.indent);
    let line = format!(
        "{}{}{indent}{marker}{}{}",
        ctx.head(),
        ctx.outer(slot - 1),
        spacing(&parent_line, &parent),
        task_text(&parent_line, &parent),
    );
    let caret = Pos::new(ctx.l, line.len());
    d.set(ctx.l, line);

    let mut requests = Vec::new();
    if let Marker::Ordered { delimiter, .. } = parent.marker {
        if !same {
            requests.push(Renumber {
                after: ctx.l,
                slot: slot - 1,
                delimiter,
                next: number + 1,
            });
        }
    }
    // The item's later siblings become the moved item's children: a new
    // list, numbered from 1.
    let orphan = an
        .next_sibling(ctx.l, slot)
        .filter(|(_, s)| same_list(s, &item));
    if let Some((k, first)) = orphan {
        if let Marker::Ordered { delimiter, .. } = first.marker {
            if !all_same(an, k, slot, &first) {
                requests.push(Renumber {
                    after: ctx.l,
                    slot,
                    delimiter,
                    next: 1,
                });
            }
        }
    }
    Some((caret, requests))
}

/// Enter with the cursor at the start of an item's text: an empty item goes
/// in above, and the cursor stays with the text. Where an empty item would
/// read as something else (after a paragraph line, `-` is a setext
/// underline), a blank line goes in instead.
fn insert_above(
    d: &mut Draft,
    an: &Analysis,
    ctx: &Line,
    slot: usize,
    item: ListItem,
    text_start: usize,
) -> (Pos, Vec<Renumber>) {
    let mut above = ctx.line[..text_start].to_owned();
    if let Some(task) = item.task {
        above.replace_range(task.start..task.start + 3, "[ ]");
    }
    above.truncate(trim_ws_end(&above).len());
    let mut probe = an.before.clone().unwrap_or_default();
    let probed = probe.next_line(&above);
    let empty_item = probed.is_empty_item() && probed.depth == slot + 1;
    if !empty_item {
        above = format!("{}{}", ctx.head(), trim_ws_end(&ctx.outer(slot)));
    }
    d.set(ctx.l, ctx.line[ctx.bom..].to_owned());
    d.insert(ctx.l, above);
    let caret = Pos::new(ctx.l + 1, ctx.c - ctx.bom);

    let mut requests = Vec::new();
    if let Marker::Ordered {
        number, delimiter, ..
    } = item.marker
    {
        if empty_item && !all_same(an, ctx.l, slot, &item) {
            requests.push(Renumber {
                after: ctx.l,
                slot,
                delimiter,
                next: number + 1,
            });
        }
    }
    (caret, requests)
}

/// Enter inside or at the end of an item's text: a new item at the same
/// level, or a first child when the item already has children.
fn continue_item(
    d: &mut Draft,
    an: &Analysis,
    ctx: &Line,
    slot: usize,
    item: ListItem,
) -> (Pos, Vec<Renumber>) {
    let at_end = is_ws(&ctx.line[ctx.c..]);
    if at_end {
        if let Some((k, child)) = an.first_child(ctx.l, slot) {
            return first_child(d, an, ctx, slot, k, child);
        }
    }
    let outer = ctx.outer(slot);
    let Some((marker, number, same)) = next_marker(an, ctx.l, slot, &item) else {
        // A tenth digit would not be a list marker any more.
        return (ctx.split(d, None, outer), Vec::new());
    };
    let indent = &ctx.line[ctx.segs[slot].start..item.marker_start];
    let marker = format!("{outer}{indent}{marker}{}", spacing(ctx.line, &item));
    // Whatever follows the line inside the item (the moving text's
    // continuation lines and children) now follows the new item, whose
    // marker may be wider.
    let delta = block::advance_column(0, &marker) as isize - item.content_column as isize;
    let mut unused = Selection::default();
    shift_lines(d, an, ctx.l + 1, slot, delta, false, &mut unused);
    let gap = is_loose(an, ctx.l, slot, &item).then(|| trim_ws_end(&outer).to_owned());
    let prefix = marker + &task_text(ctx.line, &item);
    let caret = ctx.split(d, gap, prefix);

    let mut requests = Vec::new();
    if let Marker::Ordered { delimiter, .. } = item.marker {
        if !same {
            requests.push(Renumber {
                after: caret.line,
                slot,
                delimiter,
                next: number + 1,
            });
        }
    }
    (caret, requests)
}

/// Enter at the end of an item whose first child is the item on line `k`:
/// a new first child, with that child's marker.
fn first_child(
    d: &mut Draft,
    an: &Analysis,
    ctx: &Line,
    slot: usize,
    k: usize,
    child: ListItem,
) -> (Pos, Vec<Renumber>) {
    let child_line = d.line(k).to_owned();
    let stack = an.stack(k);
    let segs = block::segments(&child_line, k, &stack, an.recs[k].continued);
    let outer = continuation(&child_line, &segs[..=slot], &stack);
    let marker = match child.marker {
        Marker::Bullet(ch) => ch.to_string(),
        Marker::Ordered {
            number,
            width,
            delimiter,
        } => format!("{number:0width$}{delimiter}"),
    };
    let prefix = format!(
        "{outer}{}{marker}{}{}",
        &child_line[segs[slot + 1].start..child.marker_start],
        spacing(&child_line, &child),
        task_text(&child_line, &child),
    );
    let caret = ctx.split(d, None, prefix);

    let mut requests = Vec::new();
    if let Marker::Ordered {
        number, delimiter, ..
    } = child.marker
    {
        if !all_same(an, k, slot + 1, &child) {
            requests.push(Renumber {
                after: caret.line,
                slot: slot + 1,
                delimiter,
                next: number + 1,
            });
        }
    }
    (caret, requests)
}

/// The marker Enter at the end of `item` (opened on line `l` at `slot`)
/// gives the next item, its number, and whether the list numbers every
/// item the same. `None` when the number would need a tenth digit.
fn next_marker(
    an: &Analysis,
    l: usize,
    slot: usize,
    item: &ListItem,
) -> Option<(String, u32, bool)> {
    match item.marker {
        Marker::Bullet(ch) => Some((ch.to_string(), 0, false)),
        Marker::Ordered {
            number,
            width,
            delimiter,
        } => {
            let same = all_same(an, l, slot, item);
            let next = if same { number } else { number + 1 };
            let digits = digits(next, number, width);
            (digits.len() <= MAX_DIGITS).then(|| (format!("{digits}{delimiter}"), next, same))
        }
    }
}

/// CommonMark allows at most nine digits in an ordered list marker.
const MAX_DIGITS: usize = 9;

/// `n` written in place of `number`, which was written `width` digits
/// wide: zero padding is kept (`09` → `10`, `01` → `02`), and an unpadded
/// number takes the digits it needs (`10` → `9`).
fn digits(n: u32, number: u32, width: usize) -> String {
    if width > number.to_string().len() {
        format!("{n:0width$}")
    } else {
        n.to_string()
    }
}

/// Whether a sibling next to `item` has the same number (`1.`, `1.`, …),
/// in which case new items repeat it and nothing is renumbered.
fn all_same(an: &Analysis, l: usize, slot: usize, item: &ListItem) -> bool {
    let Marker::Ordered { number, .. } = item.marker else {
        return false;
    };
    [an.prev_sibling(l, slot), an.next_sibling(l, slot)]
        .into_iter()
        .flatten()
        .any(|(_, s)| {
            matches!(s.marker, Marker::Ordered { number: n, .. } if n == number)
                && same_list(&s, item)
        })
}

/// A loose list (blank lines between its items) stays loose.
fn is_loose(an: &Analysis, l: usize, slot: usize, item: &ListItem) -> bool {
    let blank_before = |k: usize| k > 0 && an.recs[k - 1].info.kind == Kind::Blank;
    let sibling = |found: Option<(usize, ListItem)>| found.filter(|(_, s)| same_list(s, item));
    (sibling(an.prev_sibling(l, slot)).is_some() && blank_before(l))
        || sibling(an.next_sibling(l, slot)).is_some_and(|(k, _)| blank_before(k))
}

/// Items in one list share a bullet character or an ordered delimiter.
fn same_list(a: &ListItem, b: &ListItem) -> bool {
    match (a.marker, b.marker) {
        (Marker::Bullet(x), Marker::Bullet(y)) => x == y,
        (Marker::Ordered { delimiter: x, .. }, Marker::Ordered { delimiter: y, .. }) => x == y,
        _ => false,
    }
}

/// Renumber the items that follow `req.after` while their numbers are
/// sequential (§5.6), moving each renumbered item's content by the change
/// in its marker's width. `an` describes the draft's structure; its lines
/// must line up with the draft's.
fn renumber(d: &mut Draft, an: &Analysis, req: Renumber, marks: &mut Selection) {
    let mut after = req.after;
    let mut next = req.next;
    let mut previous: Option<u32> = None;
    while let Some((k, item)) = an.next_sibling(after, req.slot) {
        let Marker::Ordered {
            number,
            width,
            delimiter,
        } = item.marker
        else {
            break;
        };
        if delimiter != req.delimiter || previous.is_some_and(|p| p + 1 != number) || number == next
        {
            break;
        }
        let digits = digits(next, number, width);
        if digits.len() > MAX_DIGITS {
            break;
        }
        let start = item.marker_start;
        edit_line(d, k, start..start + width, &digits, marks);
        let delta = digits.len() as isize - width as isize;
        shift_lines(d, an, k + 1, req.slot, delta, false, marks);
        previous = Some(number);
        next += 1;
        after = k;
    }
}

/// An ordered run to renumber from its start after Tab, Shift+Tab, or a
/// line move (§5.6): the run holding the item that line `line` opens at
/// `slot`, or at its innermost item slot.
#[derive(Clone, Copy, Debug)]
struct Run {
    line: usize,
    slot: Option<usize>,
}

/// Renumber each distinct run in `runs` from its first item's number.
///
/// After a line move, `was` holds the original document's ordered number
/// on each line, and a run counts from the number its first line had, so
/// reordered items keep the run's start. Otherwise it is empty.
fn renumber_runs(d: &mut Draft, runs: &[Run], was: &[Option<u32>], marks: &mut Selection) {
    let an = Analysis::new(&*d, None);
    let mut found: Vec<(usize, usize)> = Vec::new();
    for run in runs {
        let Some(slot) = run.slot.or_else(|| an.item_slot(run.line)) else {
            continue;
        };
        let Some(first) = an.run_start(run.line, slot) else {
            continue;
        };
        if !found.contains(&(first, slot)) {
            found.push((first, slot));
        }
    }
    for (first, slot) in found {
        let start = was.get(first).copied().flatten();
        renumber_from(d, first, slot, start, marks);
    }
}

/// Number the run whose first item line `first` opens at `slot`
/// consecutively from `start` (or that item's own number), unless the list
/// numbers every item the same.
fn renumber_from(
    d: &mut Draft,
    first: usize,
    slot: usize,
    start: Option<u32>,
    marks: &mut Selection,
) {
    let an = Analysis::new(&*d, None);
    let Some(Container::Item(head)) = an.opened_here(first, slot) else {
        return;
    };
    let Marker::Ordered { number, .. } = head.marker else {
        return;
    };
    if all_same(&an, first, slot, &head) {
        return;
    }
    let mut next = start.unwrap_or(number);
    let mut at = Some((first, head));
    while let Some((j, item)) = at {
        let Marker::Ordered { number, width, .. } = item.marker else {
            break;
        };
        if !same_list(&item, &head) {
            break;
        }
        let digits = digits(next, number, width);
        if digits.len() > MAX_DIGITS {
            break;
        }
        let range = item.marker_start..item.marker_start + width;
        if d.line(j)[range.clone()] != digits {
            edit_line(d, j, range, &digits, marks);
            let delta = digits.len() as isize - width as isize;
            shift_lines(d, &an, j + 1, slot, delta, false, marks);
        }
        next += 1;
        at = an.next_sibling(j, slot);
    }
}

/// Number the ordered item at the start of draft line `j`'s indentation
/// (which starts at byte `ws`) `n`. A moved item is numbered before its
/// run is renumbered: one that starts a new list under a paragraph line
/// must be 1, or it reads as paragraph text, and one that joins a run must
/// not look like part of an all-same list.
fn set_number(d: &mut Draft, j: usize, ws: usize, n: u32, marks: &mut Selection) {
    let text = d.line(j).to_owned();
    let (start, _) = ws_run(&text, ws);
    let width = text[start..].bytes().take_while(u8::is_ascii_digit).count();
    let delimited = matches!(text.as_bytes().get(start + width), Some(b'.' | b')'));
    if !(1..=MAX_DIGITS).contains(&width) || !delimited {
        return;
    }
    let Ok(number) = text[start..start + width].parse::<u32>() else {
        return;
    };
    edit_line(d, j, start..start + width, &digits(n, number, width), marks);
}

/// Move the lines inside the container at `slot`, from line `from` on,
/// right by `delta` columns, or left when `delta` is negative and the
/// whitespace is there to remove. It stops at the first line outside the
/// container. Blank and lazy lines are left alone. With `keep_tabs`, a
/// line indented only with tabs grows with tabs where they fit exactly.
fn shift_lines(
    d: &mut Draft,
    an: &Analysis,
    from: usize,
    slot: usize,
    delta: isize,
    keep_tabs: bool,
    marks: &mut Selection,
) {
    if delta == 0 {
        return;
    }
    for j in from..an.recs.len() {
        let rec = &an.recs[j];
        if rec.info.lazy {
            continue;
        }
        if rec.continued <= slot {
            break;
        }
        let text = d.line(j).to_owned();
        if is_ws(&text) {
            continue;
        }
        let at = an.line_at(j, &text);
        let start = at.segs[slot].start;
        let (_, cur) = ws_run(&text, start);
        let floor = block::column(&text, start) as isize;
        let target = (cur as isize + delta).max(floor) as usize;
        let (end, _) = ws_run(&text, at.ws);
        let tabs = keep_tabs && end > at.ws && text[at.ws..end].bytes().all(|b| b == b'\t');
        let (range, pad) = reindent(&text, start, target, tabs);
        edit_line(d, j, range, &pad, marks);
    }
}

/// The end of the run of spaces and tabs at byte `from` of `line`, and the
/// column it reaches.
fn ws_run(line: &str, from: usize) -> (usize, usize) {
    let end = from + (line.len() - from - line[from..].trim_start_matches([' ', '\t']).len());
    (end, block::column(line, end))
}

/// The edit that makes the run of spaces and tabs at byte `from` of `line`
/// reach column `target` (never less than the column of `from`). Growing
/// keeps the run and pads it; shrinking keeps the bytes that end at or
/// before `target` and pads with spaces, so a tab that crosses `target`
/// becomes spaces. `tabs` pads with tabs where they fit exactly.
fn reindent(line: &str, from: usize, target: usize, tabs: bool) -> (Range<usize>, String) {
    let b = line.as_bytes();
    let from_col = block::column(line, from);
    let target = target.max(from_col);
    let (end, col) = ws_run(line, from);
    if target >= col {
        return (end..end, padding(col, target, tabs));
    }
    let (mut keep, mut at) = (from, from_col);
    loop {
        let next = match b[keep] {
            b'\t' => block::next_tab_stop(at),
            _ => at + 1,
        };
        if next > target {
            break;
        }
        at = next;
        keep += 1;
    }
    (keep..end, " ".repeat(target - at))
}

/// Whitespace from column `col` to `target`: tabs while a whole tab fits
/// (with `tabs`), then spaces.
fn padding(mut col: usize, target: usize, tabs: bool) -> String {
    let mut out = String::new();
    while tabs && block::next_tab_stop(col) <= target {
        out.push('\t');
        col = block::next_tab_stop(col);
    }
    out.extend(std::iter::repeat_n(' ', target.saturating_sub(col)));
    out
}

/// `target` moved out to the next tab stop, when indentation grows by
/// tabs (§5.4: a tab-indented line gains tabs).
fn tab_target(cur: usize, target: usize, tabs: bool) -> usize {
    if !tabs || target <= cur {
        return target;
    }
    let mut col = cur;
    while col < target {
        col = block::next_tab_stop(col);
    }
    col
}

/// Replace a byte range of one draft line. Marks on the line follow their
/// text: those after the range shift, and those inside it move to the end
/// of the new text.
fn edit_line(d: &mut Draft, j: usize, range: Range<usize>, with: &str, marks: &mut Selection) {
    let mut text = d.line(j).to_owned();
    text.replace_range(range.clone(), with);
    d.set(j, text);
    for p in [&mut marks.anchor, &mut marks.head] {
        if p.line != j || p.col < range.start {
            continue;
        }
        p.col = if p.col >= range.end {
            p.col - range.len() + with.len()
        } else if p.col == range.start {
            range.start
        } else {
            range.start + with.len()
        };
    }
}

/// The prefix a new line gets to stay in the same containers: quote
/// markers and continuation indentation are copied, and a list marker
/// becomes the spaces that reach its content column.
fn continuation(line: &str, segs: &[Segment], stack: &[Container]) -> String {
    let mut out = String::new();
    for (seg, container) in segs.iter().zip(stack) {
        match (container, seg.marker) {
            (Container::Item(item), Some(marker)) => {
                out.push_str(&line[seg.start..marker]);
                let width = item.content_column - item.indent;
                out.extend(std::iter::repeat_n(' ', width));
            }
            _ => out.push_str(&line[seg.start..seg.end]),
        }
    }
    out
}

/// The whitespace right after the container prefix and left of the cursor,
/// which a new line carries over.
fn carry_ws(line: &str, prefix_end: usize, c: usize) -> &str {
    let rest = &line[prefix_end..];
    let ws = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    &line[prefix_end..(prefix_end + ws).min(c).max(prefix_end)]
}

/// What stays on a line broken at `c`. A line left holding only whitespace
/// or markup loses its trailing whitespace; any other keeps it, so a hard
/// line break survives.
fn left_part(line: &str, c: usize, text_start: usize) -> String {
    let content = line.get(text_start..c).unwrap_or("");
    if is_ws(content) {
        trim_ws_end(&line[..c]).to_owned()
    } else {
        line[..c].to_owned()
    }
}

/// The spacing between `item`'s marker and its content, as written on its
/// first line `line`, or one space.
fn spacing(line: &str, item: &ListItem) -> String {
    let marker_end = item.indent + (item.marker_end - item.marker_start);
    if item.content_column <= marker_end + 1 {
        return " ".into();
    }
    line[item.marker_end..block::byte_at_column(line, item.content_column)].to_owned()
}

/// An open task box for a new item when `item` (first line `line`) is a
/// task, with the item's spacing after the box.
fn task_text(line: &str, item: &ListItem) -> String {
    match item.task {
        Some(task) => {
            let gap = &line[task.start + 3..item.text_start.max(task.start + 3)];
            format!("[ ]{}", if gap.is_empty() { " " } else { gap })
        }
        None => String::new(),
    }
}

/// Whitespace from byte `from` that reaches column `target`, copied when
/// the line's own indentation lines up with it, else spaces.
fn indent_to(line: &str, from: usize, target: usize) -> String {
    let start = block::column(line, from);
    let b = line.as_bytes();
    let (mut i, mut col) = (from, start);
    while col < target {
        match b.get(i) {
            Some(b' ') => col += 1,
            Some(b'\t') => col = block::next_tab_stop(col),
            _ => break,
        }
        i += 1;
    }
    if col == target {
        line[from..i].to_owned()
    } else {
        " ".repeat(target.saturating_sub(start))
    }
}

fn bom_len(l: usize, line: &str) -> usize {
    if l == 0 && line.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    }
}

fn is_ws(s: &str) -> bool {
    s.bytes().all(|b| b == b' ' || b == b'\t')
}

fn trim_ws_end(s: &str) -> &str {
    s.trim_end_matches([' ', '\t'])
}

/// A position moved onto the document, onto a character boundary, and past
/// a byte order mark.
fn clamp<L: Lines + ?Sized>(lines: &L, pos: Pos) -> Pos {
    let line = pos.line.min(lines.line_count() - 1);
    let text = lines.line(line);
    let mut col = pos.col.min(text.len());
    while !text.is_char_boundary(col) {
        col -= 1;
    }
    Pos::new(line, col.max(bom_len(line, text)))
}

/// One scanned line and its place in the container stack.
#[derive(Clone, Debug)]
struct Rec {
    info: block::LineInfo,
    /// Containers continued from the line before.
    continued: usize,
    /// Containers open after the line.
    len: usize,
    /// The first stack slot that opened on this line (`len` when none did).
    opened_from: usize,
    /// Where this line's opened containers start in [`Analysis::opened`].
    opened_at: usize,
}

/// A whole-document scan, with enough of the container stack kept to
/// rebuild any line's stack.
struct Analysis {
    recs: Vec<Rec>,
    /// The containers each line opened, line after line.
    opened: Vec<Container>,
    /// The scanner just before the line asked for in [`Analysis::new`].
    before: Option<Scanner>,
}

impl Analysis {
    fn new<L: Lines + ?Sized>(lines: &L, checkpoint: Option<usize>) -> Self {
        let mut scanner = Scanner::with_front_matter(block::front_matter_end(lines));
        let count = lines.line_count();
        let mut recs = Vec::with_capacity(count);
        let mut opened = Vec::new();
        let mut before = None;
        for i in 0..count {
            if checkpoint == Some(i) {
                before = Some(scanner.clone());
            }
            let info = scanner.next_line(lines.line(i));
            let stack = scanner.containers();
            let len = stack.len();
            let continued = scanner.continued();
            let opened_from = if info.lazy { len } else { continued };
            recs.push(Rec {
                info,
                continued,
                len,
                opened_from,
                opened_at: opened.len(),
            });
            opened.extend_from_slice(&stack[opened_from..]);
        }
        Self {
            recs,
            opened,
            before,
        }
    }

    /// The container line `l` opened at `slot`, if it opened one there.
    fn opened_here(&self, l: usize, slot: usize) -> Option<Container> {
        let rec = &self.recs[l];
        (rec.opened_from <= slot && slot < rec.len)
            .then(|| self.opened[rec.opened_at + slot - rec.opened_from])
    }

    /// The container at `slot` on line `l`, and the line that opened it.
    fn opened(&self, l: usize, slot: usize) -> (usize, Container) {
        (0..=l)
            .rev()
            .find_map(|j| self.opened_here(j, slot).map(|c| (j, c)))
            .expect("every open container was opened on some line")
    }

    /// Line `l`'s own containers: on a lazy line, only those it continued.
    fn stack(&self, l: usize) -> Vec<Container> {
        let rec = &self.recs[l];
        let depth = if rec.info.lazy {
            rec.continued
        } else {
            rec.len
        };
        (0..depth).map(|slot| self.opened(l, slot).1).collect()
    }

    /// The next item after line `l` at `slot` inside the same parents,
    /// skipping the current item's own lines and blank lines.
    fn next_sibling(&self, l: usize, slot: usize) -> Option<(usize, ListItem)> {
        for k in l + 1..self.recs.len() {
            let rec = &self.recs[k];
            if rec.info.lazy || rec.continued > slot {
                continue;
            }
            if rec.continued < slot {
                return None;
            }
            if let Some(container) = self.opened_here(k, slot) {
                return match container {
                    Container::Item(item) => Some((k, item)),
                    Container::Quote { .. } => None,
                };
            }
            if rec.info.kind != Kind::Blank {
                return None;
            }
        }
        None
    }

    /// The item before line `l` at `slot` inside the same parents.
    fn prev_sibling(&self, l: usize, slot: usize) -> Option<(usize, ListItem)> {
        let rec = &self.recs[l];
        if !rec.info.lazy && rec.continued < slot {
            return None;
        }
        for j in (0..l).rev() {
            let rec = &self.recs[j];
            if rec.info.lazy {
                continue;
            }
            if let Some(container) = self.opened_here(j, slot) {
                return match container {
                    Container::Item(item) => Some((j, item)),
                    Container::Quote { .. } => None,
                };
            }
            let blank_between = rec.info.kind == Kind::Blank && rec.len == slot;
            if rec.len <= slot && !blank_between {
                return None;
            }
        }
        None
    }

    /// The slot of the innermost list item line `l` opens.
    fn item_slot(&self, l: usize) -> Option<usize> {
        let rec = &self.recs[l];
        if rec.info.lazy {
            return None;
        }
        (rec.opened_from..rec.len)
            .rev()
            .find(|&slot| matches!(self.opened_here(l, slot), Some(Container::Item(_))))
    }

    /// The item line `l` opens at `slot`.
    fn item_here(&self, l: usize, slot: usize) -> Option<ListItem> {
        match self.opened_here(l, slot) {
            Some(Container::Item(item)) => Some(item),
            _ => None,
        }
    }

    /// The first line of the ordered run holding the item line `l` opens
    /// at `slot`.
    fn run_start(&self, l: usize, slot: usize) -> Option<usize> {
        let item = self.item_here(l, slot)?;
        if !matches!(item.marker, Marker::Ordered { .. }) {
            return None;
        }
        let mut first = l;
        while let Some((p, prev)) = self.prev_sibling(first, slot) {
            if !same_list(&prev, &item) {
                break;
            }
            first = p;
        }
        Some(first)
    }

    /// Line `l` (whose text is `text`) split into its containers.
    fn line_at(&self, l: usize, text: &str) -> LineAt {
        let stack = self.stack(l);
        let segs = block::segments(text, l, &stack, self.recs[l].continued);
        let ws = segs
            .iter()
            .zip(&stack)
            .rev()
            .find(|(_, c)| matches!(c, Container::Quote { .. }))
            .map(|(s, _)| s.end)
            .unwrap_or_else(|| bom_len(l, text));
        LineAt { stack, segs, ws }
    }

    /// The first child item of the item opened on line `l` at `slot`: the
    /// first line inside the item after any blank lines, if it opens an
    /// item one level deeper.
    fn first_child(&self, l: usize, slot: usize) -> Option<(usize, ListItem)> {
        for k in l + 1..self.recs.len() {
            let rec = &self.recs[k];
            if rec.info.lazy || rec.continued <= slot {
                return None;
            }
            if rec.info.kind == Kind::Blank && rec.len == rec.continued {
                continue;
            }
            return match self.opened_here(k, slot + 1) {
                Some(Container::Item(child)) => Some((k, child)),
                _ => None,
            };
        }
        None
    }
}

/// A line's own containers and its shares of the container prefix.
struct LineAt {
    stack: Vec<Container>,
    segs: Vec<Segment>,
    /// Where the line's indentation starts: after the innermost quote's
    /// marker, or a byte order mark. Indentation is measured from here
    /// (§5.4), and quote markers before it are left as they are.
    ws: usize,
}

impl LineAt {
    /// The content columns of the list items inside the innermost quote:
    /// the line's indent stops besides its base column (§5.4).
    fn item_stops(&self) -> Vec<usize> {
        let inner = self
            .stack
            .iter()
            .rposition(|c| matches!(c, Container::Quote { .. }))
            .map_or(0, |i| i + 1);
        self.stack[inner..]
            .iter()
            .map(Container::content_column)
            .collect()
    }
}

/// The deepest indent stop below column `cur`: `base`, the item `stops`,
/// and beyond the deepest of those every `width` columns.
fn previous_stop(cur: usize, base: usize, stops: &[usize], width: usize) -> usize {
    let width = width.max(1);
    let deepest = stops.iter().copied().fold(base, usize::max);
    if cur > deepest {
        deepest + (cur - deepest - 1) / width * width
    } else {
        stops
            .iter()
            .copied()
            .filter(|&s| s < cur && s >= base)
            .fold(base, usize::max)
    }
}

/// The marker an item gets as the first of a new list: its own bullet, or
/// 1 with its own delimiter.
fn first_marker(item: &ListItem) -> String {
    match item.marker {
        Marker::Bullet(ch) => ch.to_string(),
        Marker::Ordered {
            number,
            width,
            delimiter,
        } => format!("{}{delimiter}", digits(1, number, width)),
    }
}

/// The document's lines as the planner rewrites them. Untouched lines
/// borrow the original text and remember where they came from, so
/// [`diff`] can find what changed.
struct Draft<'a> {
    lines: Vec<DraftLine<'a>>,
}

struct DraftLine<'a> {
    text: Cow<'a, str>,
    /// The original line this one was made from.
    origin: Option<usize>,
}

impl<'a> Draft<'a> {
    fn new<L: Lines + ?Sized>(base: &'a L) -> Self {
        let lines = (0..base.line_count())
            .map(|i| DraftLine {
                text: Cow::Borrowed(base.line(i)),
                origin: Some(i),
            })
            .collect();
        Self { lines }
    }

    fn set(&mut self, i: usize, text: String) {
        self.lines[i].text = Cow::Owned(text);
    }

    fn insert(&mut self, i: usize, text: String) {
        self.lines.insert(
            i,
            DraftLine {
                text: Cow::Owned(text),
                origin: None,
            },
        );
    }

    /// Delete the text between two positions, joining their lines.
    fn delete(&mut self, start: Pos, end: Pos) {
        if start >= end {
            return;
        }
        let joined = format!(
            "{}{}",
            &self.lines[start.line].text[..start.col],
            &self.lines[end.line].text[end.col..]
        );
        self.set(start.line, joined);
        self.lines.drain(start.line + 1..=end.line);
    }
}

impl Lines for Draft<'_> {
    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, i: usize) -> &str {
        &self.lines[i].text
    }
}

/// The replacements that turn `base` into `draft`: one for each changed
/// line and each run of inserted or deleted lines, trimmed to the
/// characters that differ, with touching replacements merged.
fn diff<L: Lines + ?Sized>(base: &L, draft: &Draft) -> Vec<Replace> {
    let n = base.line_count();
    let m = draft.lines.len();
    let mut edits: Vec<Replace> = Vec::new();
    let (mut i, mut j) = (0, 0);
    loop {
        let next = (j..m).find_map(|jj| draft.lines[jj].origin.map(|ii| (ii, jj)));
        let (ii, jj) = next.unwrap_or((n, m));
        let changed = next.is_some_and(|_| match &draft.lines[jj].text {
            Cow::Borrowed(_) => false,
            Cow::Owned(text) => text != base.line(ii),
        });
        let found = [
            region(base, draft, i..ii, j..jj),
            changed
                .then(|| region(base, draft, ii..ii + 1, jj..jj + 1))
                .flatten(),
        ];
        for edit in found.into_iter().flatten() {
            match edits.last_mut() {
                Some(last) if last.end == edit.start => {
                    last.end = edit.end;
                    last.text.push_str(&edit.text);
                }
                _ => edits.push(edit),
            }
        }
        if next.is_none() {
            return edits;
        }
        (i, j) = (ii + 1, jj + 1);
    }
}

/// The replacement of original lines `old` by draft lines `new`.
fn region<L: Lines + ?Sized>(
    base: &L,
    draft: &Draft,
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
) -> Option<Replace> {
    if old.is_empty() && new.is_empty() {
        return None;
    }
    let n = base.line_count();
    let join = |lines: &mut dyn Iterator<Item = &str>, sep_after: bool| {
        let mut out = String::new();
        for (k, line) in lines.enumerate() {
            if k > 0 && !sep_after {
                out.push('\n');
            }
            out.push_str(line);
            if sep_after {
                out.push('\n');
            }
        }
        out
    };
    let old_lines = &mut old.clone().map(|i| base.line(i));
    let new_lines = &mut new.clone().map(|j| draft.line(j));
    let (start, old_text, new_text) = if old.end < n {
        (
            Pos::new(old.start, 0),
            join(old_lines, true),
            join(new_lines, true),
        )
    } else if old.start == n {
        let last = base.line(n - 1).len();
        let text = format!("\n{}", join(new_lines, false));
        (Pos::new(n - 1, last), String::new(), text)
    } else if new.is_empty() && old.start > 0 {
        let prev = base.line(old.start - 1).len();
        let text = format!("\n{}", join(old_lines, false));
        (Pos::new(old.start - 1, prev), text, String::new())
    } else {
        (
            Pos::new(old.start, 0),
            join(old_lines, false),
            join(new_lines, false),
        )
    };
    let prefix: usize = old_text
        .chars()
        .zip(new_text.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let suffix: usize = old_text[prefix..]
        .chars()
        .rev()
        .zip(new_text[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let old_mid = &old_text[prefix..old_text.len() - suffix];
    let new_mid = &new_text[prefix..new_text.len() - suffix];
    if old_mid.is_empty() && new_mid.is_empty() {
        return None;
    }
    let from = advance(start, &old_text[..prefix]);
    if old_mid.is_empty() && from.col == 0 && from.line > 0 && new_mid.ends_with('\n') {
        // Lines inserted at the start of a line go in at the end of the line
        // before instead, so the new breaks split that line, after its text.
        let at = Pos::new(from.line - 1, base.line(from.line - 1).len());
        let text = format!("\n{}", &new_mid[..new_mid.len() - 1]);
        return Some(Replace {
            start: at,
            end: at,
            text,
        });
    }
    Some(Replace {
        start: from,
        end: advance(from, old_mid),
        text: new_mid.to_owned(),
    })
}

/// The position reached by moving over `text` from `pos`.
fn advance(pos: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        Some(last) => Pos::new(pos.line + text.matches('\n').count(), text.len() - last - 1),
        None => Pos::new(pos.line, pos.col + text.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, LineEnding};

    /// Parse a fixture: `|` is the cursor, `«` a selection's anchor and `»`
    /// its head.
    pub(super) fn parse(fixture: &str) -> (String, Selection) {
        let mut text = String::new();
        let (mut anchor, mut head) = (None, None);
        let mut pos = Pos::default();
        for ch in fixture.chars() {
            match ch {
                '|' => (anchor, head) = (Some(pos), Some(pos)),
                '«' => anchor = Some(pos),
                '»' => head = Some(pos),
                '\n' => {
                    text.push('\n');
                    pos = Pos::new(pos.line + 1, 0);
                }
                _ => {
                    text.push(ch);
                    pos.col += ch.len_utf8();
                }
            }
        }
        let (Some(anchor), Some(head)) = (anchor, head) else {
            panic!("{fixture:?} has no cursor");
        };
        (text, Selection::new(anchor, head))
    }

    /// Mark a selection in `text`.
    pub(super) fn render(text: &str, sel: Selection) -> String {
        let doc = Document::new(text);
        let at = |p: Pos| {
            assert!(
                p.line < doc.line_count(),
                "{p:?} is past the end of {text:?}"
            );
            let line = doc.line(p.line);
            assert!(
                p.col <= line.len() && line.is_char_boundary(p.col),
                "{p:?} in {line:?}"
            );
            doc.offset(p)
        };
        let (anchor, head) = (at(sel.anchor), at(sel.head));
        let mut out = text.to_owned();
        if anchor == head {
            out.insert(anchor, '|');
        } else if anchor < head {
            out.insert(head, '»');
            out.insert(anchor, '«');
        } else {
            out.insert(anchor, '«');
            out.insert(head, '»');
        }
        out
    }

    /// Apply a plan the way an editor does (plan §4): bottom-up, with each
    /// `'\n'` written as the ending of the line being split, or on the last
    /// line the previous line's, else LF.
    pub(super) fn apply(doc: &Document, plan: &EditPlan) -> String {
        for pair in plan.edits.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "edits overlap: {:?}",
                plan.edits
            );
        }
        let mut text = doc.text().to_owned();
        for edit in plan.edits.iter().rev() {
            assert!(edit.start <= edit.end, "{edit:?}");
            for p in [edit.start, edit.end] {
                let line = doc.line(p.line);
                assert!(
                    p.col <= line.len() && line.is_char_boundary(p.col),
                    "{edit:?}"
                );
            }
            let ending = match doc.ending(edit.start.line) {
                LineEnding::None if edit.start.line > 0 => doc.ending(edit.start.line - 1),
                LineEnding::None => LineEnding::Lf,
                ending => ending,
            };
            let range = doc.offset(edit.start)..doc.offset(edit.end);
            text.replace_range(range, &edit.text.replace('\n', ending.as_str()));
        }
        text
    }

    const ENDINGS: [&str; 4] = ["\n", "\r\n", "\r", "\n\r"];
    /// No ending in this cycle runs into the next, even around empty lines.
    const MIXED: [&str; 4] = ["\r\n", "\n", "\n\r", "\r"];

    fn join(text: &str, endings: &[&str]) -> String {
        let mut out = String::new();
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                out.push_str(endings[(i - 1) % endings.len()]);
            }
            out.push_str(line);
        }
        out
    }

    fn normalize(text: &str) -> String {
        let doc = Document::new(text);
        let lines: Vec<&str> = (0..doc.line_count()).map(|i| doc.line(i)).collect();
        lines.join("\n")
    }

    /// Run each `before → after` case under every line ending, mixed
    /// endings, and with and without a final line ending.
    #[track_caller]
    pub(super) fn check_with(intent: Intent, opts: &Options, cases: &[(&str, &str)]) {
        for (before, after) in cases {
            let (text, sel) = parse(before);
            let variants = ENDINGS
                .iter()
                .map(|e| (*e, std::slice::from_ref(e)))
                .chain([("mixed", &MIXED[..])]);
            for (name, endings) in variants {
                for final_ending in [false, true] {
                    let mut input = join(&text, endings);
                    let last = endings[text.matches('\n').count() % endings.len()];
                    if final_ending {
                        input.push_str(last);
                    }
                    let doc = Document::new(&input);
                    let what = format!(
                        "{intent:?} on {before:?} with {name:?} endings{}",
                        if final_ending { " and a final one" } else { "" }
                    );
                    let plan = plan(&doc, sel, intent, opts);
                    let got = render(&apply(&doc, &plan), plan.selection);
                    if name == "mixed" {
                        let tail = if final_ending { "\n" } else { "" };
                        assert_eq!(normalize(&got), format!("{after}{tail}"), "{what}");
                    } else {
                        let used = if input.contains(name) { name } else { "\n" };
                        let tail = if final_ending { name } else { "" };
                        assert_eq!(got, format!("{}{tail}", join(after, &[used])), "{what}");
                    }
                }
            }
        }
    }

    #[track_caller]
    pub(super) fn check(intent: Intent, cases: &[(&str, &str)]) {
        check_with(intent, &Options::default(), cases);
    }

    #[test]
    fn enter_continues_lists_and_quotes() {
        check(
            Intent::Enter,
            &[
                ("- item|", "- item\n- |"),
                ("- it|em", "- it\n- |em"),
                ("* a|", "* a\n* |"),
                ("+ a|", "+ a\n+ |"),
                ("- [x] done|", "- [x] done\n- [ ] |"),
                ("- [ ] a|", "- [ ] a\n- [ ] |"),
                ("- [X]  a|", "- [X]  a\n- [ ]  |"),
                ("1. [x] a|", "1. [x] a\n2. [ ] |"),
                ("> quote|", "> quote\n> |"),
                ("> - a|", "> - a\n> - |"),
                ("> > a|", "> > a\n> > |"),
                (">a|", ">a\n>|"),
                ("  - sub|", "  - sub\n  - |"),
                ("- a\n  - b|", "- a\n  - b\n  - |"),
                ("- - a|", "- - a\n  - |"),
                ("-   a|", "-   a\n-   |"),
                ("-\ta|", "-\ta\n-\t|"),
                ("-     code|", "-     code\n      |"),
                ("- é|ü", "- é\n- |ü"),
                ("- a\n  b|", "- a\n  b\n  |"),
                ("- a\n    b|", "- a\n    b\n    |"),
                ("> - a\n>   b|", "> - a\n>   b\n>   |"),
            ],
        );
    }

    #[test]
    fn enter_gives_an_item_with_children_a_first_child() {
        check(
            Intent::Enter,
            &[
                ("- a|\n  - b", "- a\n  - |\n  - b"),
                ("- a|\n  * b", "- a\n  * |\n  * b"),
                ("- a|  \n  - b", "- a\n  - |  \n  - b"),
                ("1. a|\n   1. b\n   2. c", "1. a\n   1. |\n   2. b\n   3. c"),
                ("- a|\n  - [x] b", "- a\n  - [ ] |\n  - [x] b"),
                ("- a|\n\n  - b", "- a\n  - |\n\n  - b"),
                ("> - a|\n>   - b", "> - a\n>   - |\n>   - b"),
                // Only at the end of the text: a split takes the children
                // along with the text that moves down.
                ("- a|b\n  - c", "- a\n- |b\n  - c"),
                // A continuation paragraph is not a child.
                ("- a|\n  b", "- a\n- |\n  b"),
            ],
        );
    }

    #[test]
    fn enter_with_a_selection_deletes_it_first() {
        check(
            Intent::Enter,
            &[
                ("- a«bc»d", "- a\n- |d"),
                ("- a»bc«d", "- a\n- |d"),
                ("- a«b\n- c»d", "- a\n- |d"),
                ("- a«\n»", "- a\n- |"),
                ("«abc»", "\n|"),
                ("«- a\n- b»", "\n|"),
                ("1. a«b\n2. c»\n3. d", "1. a\n2. |\n3. d"),
            ],
        );
    }

    #[test]
    fn enter_numbers_the_new_item() {
        check(
            Intent::Enter,
            &[
                ("9. nine|", "9. nine\n10. |"),
                ("1) a|", "1) a\n2) |"),
                ("09. a|", "09. a\n10. |"),
                ("01. a|", "01. a\n02. |"),
                ("0. a|", "0. a\n1. |"),
                ("99. a|", "99. a\n100. |"),
                ("999999999. x|", "999999999. x\n|"),
                ("> 999999999. x|", "> 999999999. x\n> |"),
            ],
        );
    }

    #[test]
    fn enter_on_an_empty_item_outdents_or_exits() {
        check(
            Intent::Enter,
            &[
                ("- a\n  - |", "- a\n- |"),
                ("1. a\n   - |", "1. a\n2. |"),
                ("- [ ] a\n  - |", "- [ ] a\n- [ ] |"),
                ("- a\n  - [x] |", "- a\n- |"),
                ("* a\n  - |", "* a\n* |"),
                ("- a\n  - b\n  - |", "- a\n  - b\n- |"),
                ("1. a\n   1. b\n   2. |", "1. a\n   1. b\n2. |"),
                ("> - a\n>   - |", "> - a\n> - |"),
                ("  - a\n    - |", "  - a\n  - |"),
                ("- a\n  -|", "- a\n- |"),
                ("- a\n- |", "- a\n\n|"),
                ("- a\n- [ ] |", "- a\n\n|"),
                ("- [ ] |", "|"),
                ("-|", "|"),
                ("1.|", "|"),
                ("- [ ]|", "|"),
                ("> a\n> |", "> a\n\n|"),
                ("> > a\n> > |", "> > a\n>\n> |"),
                ("> - |", "> |"),
                ("> - a\n> - |", "> - a\n>\n> |"),
                ("- a\n\n- |", "- a\n\n|"),
                ("# T\n- |", "# T\n|"),
                ("> a\n>\n> |", "> a\n>\n|"),
                ("- a\n  > |", "- a\n\n  |"),
            ],
        );
    }

    #[test]
    fn enter_at_the_start_of_item_text_inserts_an_item_above() {
        check(
            Intent::Enter,
            &[
                ("- |item", "-\n- |item"),
                ("- a\n- |b", "- a\n-\n- |b"),
                ("- [x] |done", "- [ ]\n- [x] |done"),
                ("1. |x\n2. y", "1.\n2. |x\n3. y"),
                ("> - |x", "> -\n> - |x"),
                // After a paragraph line an empty item would be a setext
                // underline or a lazy line, so a blank line goes in.
                ("Para\n- |x", "Para\n\n- |x"),
                ("Para\n1. |x", "Para\n\n1. |x"),
                ("- a\n  - |b", "- a\n\n  - |b"),
                ("> Para\n> - |x", "> Para\n>\n> - |x"),
            ],
        );
    }

    #[test]
    fn enter_in_markup_or_indentation() {
        check(
            Intent::Enter,
            &[
                ("-| item", "-\n- |item"),
                ("- [| ] x", "- [ ]\n- [ ] |x"),
                ("1|. x", "1.\n2. |x"),
                (">| a", ">\n> |a"),
                ("  |- a", "\n  |- a"),
                ("|- a", "\n|- a"),
                ("> |- a", ">\n> |- a"),
                ("|> a", "\n|> a"),
                ("- a\n |  b", "- a\n\n |  b"),
            ],
        );
    }

    #[test]
    fn enter_keeps_loose_lists_loose() {
        check(
            Intent::Enter,
            &[
                ("- a\n\n- b|", "- a\n\n- b\n\n- |"),
                ("- a|\n\n- b", "- a\n\n- |\n\n- b"),
                ("1. a|\n\n2. b", "1. a\n\n2. |\n\n3. b"),
                ("> 1. a\n>\n> 2. b|", "> 1. a\n>\n> 2. b\n>\n> 3. |"),
                ("- a\n  - b\n\n  - c|", "- a\n  - b\n\n  - c\n\n  - |"),
                // A blank line before a list's first item is not between items.
                ("x\n\n- a|", "x\n\n- a\n- |"),
            ],
        );
    }

    #[test]
    fn enter_trims_only_lines_left_without_text() {
        check(
            Intent::Enter,
            &[
                ("- foo  |", "- foo  \n- |"),
                ("    text|", "    text\n    |"),
                ("    |", "\n    |"),
                ("\ttext|", "\ttext\n\t|"),
                ("  \t|", "\n  \t|"),
                ("    te|xt", "    te\n    |xt"),
                ("a| b", "a\n| b"),
                ("a|", "a\n|"),
                ("- a\n  |", "- a\n\n  |"),
                ("# |Title", "#\n|Title"),
            ],
        );
    }

    #[test]
    fn enter_on_headings_breaks_and_lazy_lines_continues_only_containers() {
        check(
            Intent::Enter,
            &[
                ("---|", "---\n|"),
                ("***|", "***\n|"),
                ("- - -|", "- - -\n|"),
                ("# Title|", "# Title\n|"),
                ("  # Title|", "  # Title\n  |"),
                ("a\n---|", "a\n---\n|"),
                ("a\n===|", "a\n===\n|"),
                ("> # T|", "> # T\n> |"),
                ("- # T|", "- # T\n  |"),
                ("> a\nb|", "> a\nb\n|"),
                ("> > a\n> b|", "> > a\n> b\n> |"),
                ("1. a\nb c|", "1. a\nb c\n|"),
            ],
        );
    }

    #[test]
    fn enter_in_code_continues_container_prefixes() {
        check(
            Intent::Enter,
            &[
                ("> ```\n> code|", "> ```\n> code\n> |"),
                ("```\n  code|\n```", "```\n  code\n  |\n```"),
                ("```\n- a|\n```", "```\n- a\n|\n```"),
                ("- ```\n  code|", "- ```\n  code\n  |"),
                ("- ```\n    code|", "- ```\n    code\n    |"),
                ("- ```foo|", "- ```foo\n  |"),
                ("> - ```foo|", "> - ```foo\n>   |"),
                ("    code|", "    code\n    |"),
                ("x\n\n    - a|", "x\n\n    - a\n    |"),
                ("<!--\n- a|\n-->", "<!--\n- a\n|\n-->"),
                ("- <div|>", "- <div\n  |>"),
                ("---\ntitle: x|\n---", "---\ntitle: x\n|\n---"),
                ("```\n> a|", "```\n> a\n|"),
            ],
        );
    }

    #[test]
    fn enter_renumbers_the_rest_of_the_run() {
        check(
            Intent::Enter,
            &[
                ("1. a|\n2. b", "1. a\n2. |\n3. b"),
                ("1. a|\n2. b\n3. c\n1. d", "1. a\n2. |\n3. b\n4. c\n1. d"),
                ("1. a|\n2. b\n1) c", "1. a\n2. |\n3. b\n1) c"),
                ("1. a|\n2. b\n- c\n3. d", "1. a\n2. |\n3. b\n- c\n3. d"),
                ("1. a|b\n   - x\n2. c", "1. a\n2. |b\n   - x\n3. c"),
                (
                    "1. a|\n2. b\n   more\n3. c",
                    "1. a\n2. |\n3. b\n   more\n4. c",
                ),
                ("1. a|\n2. b\nlazy\n3. c", "1. a\n2. |\n3. b\nlazy\n4. c"),
                ("1. a|\n2. b\n# h\n3. c", "1. a\n2. |\n3. b\n# h\n3. c"),
                ("1. a|\n2. b\n\n---\n3. c", "1. a\n2. |\n3. b\n\n---\n3. c"),
                (
                    "> 1. a|\n> 2. b\n\n> 3. c",
                    "> 1. a\n> 2. |\n> 3. b\n\n> 3. c",
                ),
                (
                    "- x\n  1. a|\n  2. b\n- y\n  3. c",
                    "- x\n  1. a\n  2. |\n  3. b\n- y\n  3. c",
                ),
                ("1. a|\n3. b\n4. c", "1. a\n2. |\n3. b\n4. c"),
                ("1. a|\n5. b\n9. c", "1. a\n2. |\n3. b\n9. c"),
                ("1. a\n2. |\n3. b", "1. a\n\n|\n2. b"),
                ("1. |\n2. b", "|\n1. b"),
                ("1. a\n   1. b\n   2. |\n2. c", "1. a\n   1. b\n2. |\n3. c"),
                (
                    "1. a\n   1. b\n   2. |\n   3. c",
                    "1. a\n   1. b\n2. |\n   1. c",
                ),
                (
                    "999999998. a|\n999999999. b",
                    "999999998. a\n999999999. |\n999999999. b",
                ),
            ],
        );
    }

    #[test]
    fn enter_keeps_all_same_numbering() {
        check(
            Intent::Enter,
            &[
                ("1. a|\n1. b", "1. a\n1. |\n1. b"),
                ("1. a\n1. b|", "1. a\n1. b\n1. |"),
                ("1. a\n1. |", "1. a\n\n|"),
                ("1. a\n1. |\n1. b", "1. a\n\n|\n1. b"),
                ("1. a\n1. |x", "1. a\n1.\n1. |x"),
                ("1. a\n   - |\n1. b", "1. a\n1. |\n1. b"),
            ],
        );
    }

    #[test]
    fn enter_moves_content_when_a_marker_changes_width() {
        check(
            Intent::Enter,
            &[
                ("8. a|\n9. b\n   - c", "8. a\n9. |\n10. b\n    - c"),
                ("9. ni|ne\n   - c", "9. ni\n10. |ne\n    - c"),
                ("9. |x\n   - c", "9.\n10. |x\n    - c"),
                (
                    "98. a|\n99. b\n    c\n\n    d",
                    "98. a\n99. |\n100. b\n     c\n\n     d",
                ),
                ("8. a\n9. |\n10. b\n    - c", "8. a\n\n|\n9. b\n   - c"),
                (
                    "> 8. a|\n> 9. b\n>    c",
                    "> 8. a\n> 9. |\n> 10. b\n>     c",
                ),
                ("8. a|\n9. b\nlazy", "8. a\n9. |\n10. b\nlazy"),
                ("9. a|\n   more", "9. a\n10. |\n    more"),
            ],
        );
    }

    #[test]
    fn renumbering_can_be_turned_off() {
        let opts = Options {
            renumber: false,
            ..Options::default()
        };
        check_with(
            Intent::Enter,
            &opts,
            &[
                ("1. a|\n2. b", "1. a\n2. |\n2. b"),
                ("1. a\n2. |\n3. b", "1. a\n\n|\n3. b"),
            ],
        );
    }

    #[test]
    fn shift_enter_breaks_inside_the_item() {
        check(
            Intent::ShiftEnter,
            &[
                ("- [ ] a|", "- [ ] a\n  |"),
                ("10. a|", "10. a\n    |"),
                ("- a|", "- a\n  |"),
                ("- a|b", "- a\n  |b"),
                ("- |", "-\n  |"),
                ("- a\n  b|", "- a\n  b\n  |"),
                ("> - a|", "> - a\n>   |"),
                ("> a|", "> a\n>\n> |"),
                ("> |a", ">\n> |a"),
                ("> |", ">\n> |"),
                ("- > a|", "- > a\n  >\n  > |"),
                ("a|", "a\n|"),
                ("    a|", "    a\n    |"),
                ("# T|", "# T\n|"),
                ("```\nx|", "```\nx\n|"),
                ("> ```\n> x|", "> ```\n> x\n> |"),
                ("- a«b»c", "- a\n  |c"),
            ],
        );
    }

    #[test]
    fn enter_on_an_empty_document() {
        check(Intent::Enter, &[("|", "\n|")]);
        check(Intent::ShiftEnter, &[("|", "\n|")]);
    }

    #[test]
    fn a_byte_order_mark_stays_on_the_first_line() {
        check(
            Intent::Enter,
            &[
                ("\u{feff}- a|", "\u{feff}- a\n- |"),
                ("\u{feff}- |x", "\u{feff}-\n- |x"),
                ("\u{feff}> |", "\u{feff}|"),
                ("\u{feff}|a", "\u{feff}\n|a"),
                ("|\u{feff}a", "\u{feff}\n|a"),
                ("\u{feff}  |- a", "\u{feff}\n  |- a"),
            ],
        );
    }

    #[test]
    fn plans_are_small_replacements() {
        let plan = |text: &str, pos: Pos| {
            let doc = Document::new(text);
            super::plan(
                &doc,
                Selection::caret(pos),
                Intent::Enter,
                &Options::default(),
            )
        };
        let insert = |line, col, text: &str| Replace {
            start: Pos::new(line, col),
            end: Pos::new(line, col),
            text: text.into(),
        };
        assert_eq!(plan("- item", Pos::new(0, 6)).edits, [insert(0, 6, "\n- ")]);
        assert_eq!(
            plan("a\r\n- item\r\nb", Pos::new(1, 6)).edits,
            [insert(1, 6, "\n- ")]
        );
        assert_eq!(
            plan("1. a\n2. b\n3. c", Pos::new(0, 4)).edits,
            [
                insert(0, 4, "\n2. "),
                Replace {
                    start: Pos::new(1, 0),
                    end: Pos::new(1, 1),
                    text: "3".into()
                },
                Replace {
                    start: Pos::new(2, 0),
                    end: Pos::new(2, 1),
                    text: "4".into()
                },
            ]
        );
        let exit = plan("- a\n- ", Pos::new(1, 2));
        assert_eq!(
            exit.edits,
            [Replace {
                start: Pos::new(1, 0),
                end: Pos::new(1, 2),
                text: "\n".into()
            }]
        );
        assert_eq!(exit.selection, Selection::caret(Pos::new(2, 0)));
    }

    #[test]
    fn positions_off_the_document_are_clamped() {
        let doc = Document::new("a é");
        let sel = Selection::new(Pos::new(0, 3), Pos::new(9, 9));
        let plan = plan(&doc, sel, Intent::Enter, &Options::default());
        assert_eq!(render(&apply(&doc, &plan), plan.selection), "a \n|");
    }

    #[test]
    fn every_position_in_small_documents_plans_cleanly() {
        const VOCAB: [&str; 20] = [
            "",
            "- a",
            "  - b",
            "1. x",
            "10. y",
            "> q",
            ">",
            "> - w",
            ">   c",
            "-",
            "1.",
            "- [ ] t",
            "```",
            "    code",
            "Para",
            "---",
            "# h",
            "\t- z",
            "\u{feff}- é",
            "<div>",
        ];
        let opts = Options::default();
        let mut docs = 0;
        for a in VOCAB {
            for b in VOCAB {
                for c in ["", "- a", "  1. n", "> q", "x"] {
                    let text = format!("{a}\n{b}\n{c}");
                    let doc = Document::new(&text);
                    docs += 1;
                    for line in 0..doc.line_count() {
                        let len = doc.line(line).len();
                        for col in (0..=len).filter(|&col| doc.line(line).is_char_boundary(col)) {
                            let here = Pos::new(line, col);
                            for sel in
                                [Selection::caret(here), Selection::new(Pos::new(0, 0), here)]
                            {
                                for intent in [Intent::Enter, Intent::ShiftEnter] {
                                    let plan = plan(&doc, sel, intent, &opts);
                                    // A caret always changes the text; a selection may be replaced
                                    // by the same line break.
                                    assert!(
                                        !sel.is_empty() || !plan.edits.is_empty(),
                                        "{intent:?} at {sel:?} in {text:?}"
                                    );
                                    render(&apply(&doc, &plan), plan.selection);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(docs, 2000);
    }

    #[test]
    #[ignore = "micro benchmark; run with --release --ignored --nocapture"]
    fn benchmark_enter_in_5000_lines() {
        let mut text = String::new();
        for i in 0..5_000 {
            let line = match i % 10 {
                0 => "# Section".to_string(),
                1 | 5 => String::new(),
                2..=4 => format!("- item {i}"),
                6 => "  - [ ] nested task".into(),
                _ => "Some paragraph text with `code` in it.".into(),
            };
            text.push_str(&line);
            text.push('\n');
        }
        // A 1,000-item ordered list, renumbered after an Enter on its first item.
        for n in 1..=1_000 {
            text.push_str(&format!("{n}. ordered\n"));
        }
        let doc = Document::new(&text);
        let rounds = 50;
        let time = |pos: Pos| {
            let start = std::time::Instant::now();
            for _ in 0..rounds {
                plan(
                    &doc,
                    Selection::caret(pos),
                    Intent::Enter,
                    &Options::default(),
                );
            }
            start.elapsed() / rounds
        };
        let item = time(Pos::new(2_502, 10));
        let renumber = time(Pos::new(5_000, 10));
        println!(
            "6,000 lines: Enter on an item {item:?}, Enter renumbering 999 items {renumber:?}"
        );
    }

    mod codemirror {
        // The Enter cases of CodeMirror's Markdown command tests, ported to
        // Rust. Where Wren's spec decides differently, the case asserts Wren's
        // behavior and is listed in `discovery/markdown-editor.md` §7.1.
        //
        // Ported from `test/test-commands.ts` (the `insertNewlineContinueMarkup`
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

        fn enter(cases: &[(&str, &str)]) {
            check(Intent::Enter, cases);
        }

        #[test]
        fn doesnt_continue_anything_at_the_top_level() {
            // Divergence: CodeMirror declines; Wren plans the plain break.
            enter(&[("one|", "one\n|")]);
        }

        #[test]
        fn doesnt_do_anything_in_non_markdown_content() {
            // Divergence: an HTML block in an item keeps the item's indent.
            enter(&[("- <div|>", "- <div\n  |>")]);
        }

        #[test]
        fn can_continue_blockquotes() {
            enter(&[("> one|", "> one\n> |")]);
        }

        #[test]
        fn will_end_blockquotes_after_two_empty_lines() {
            // Divergence: one Enter on an empty quote line ends the quote.
            enter(&[("> one\n>\n> |", "> one\n>\n|")]);
        }

        #[test]
        fn will_end_nested_blockquotes_after_two_empty_lines() {
            // Divergence: as above, one level at a time.
            enter(&[("> > one\n> >\n> > |", "> > one\n> >\n> |")]);
        }

        #[test]
        fn can_continue_nested_blockquotes() {
            enter(&[("> > one|", "> > one\n> > |")]);
        }

        #[test]
        fn preserves_the_absence_of_a_blockquote_space() {
            enter(&[(">>one|", ">>one\n>>|")]);
        }

        #[test]
        fn can_continue_bullet_lists_with_dashes() {
            enter(&[(" - one|", " - one\n - |")]);
        }

        #[test]
        fn can_continue_bullet_lists_with_asterisks() {
            enter(&[(" *  one|", " *  one\n *  |")]);
        }

        #[test]
        fn can_continue_bullet_lists_with_plus_signs() {
            enter(&[("+ one|", "+ one\n+ |")]);
        }

        #[test]
        fn can_continue_ordered_lists_with_dots() {
            enter(&[(" 1. one|", " 1. one\n 2. |")]);
        }

        #[test]
        fn can_continue_ordered_lists_with_parens() {
            enter(&[("2)  one|", "2)  one\n3)  |")]);
        }

        #[test]
        fn can_continue_lists_inside_blockquotes() {
            enter(&[("> - one|", "> - one\n> - |")]);
        }

        #[test]
        fn can_continue_nested_lists() {
            enter(&[(" - one\n    1. two|", " - one\n    1. two\n    2. |")]);
        }

        #[test]
        fn will_leave_space_before_nested_blockquotes() {
            enter(&[(" - one\n   > quoted|", " - one\n   > quoted\n   > |")]);
        }

        #[test]
        fn can_drop_trailing_space_when_pressing_enter_in_a_blockquote() {
            // Divergence: an empty quote line is an exit, even on line 0.
            enter(&[(">  |", "|")]);
        }

        #[test]
        fn can_move_list_markup_when_pressing_enter_directly_after_it() {
            // Divergence: Enter on an empty item exits the list, leaving a
            // blank line, instead of making the list loose.
            enter(&[(" - one\n - |", " - one\n\n|")]);
        }

        #[test]
        fn will_preserve_non_tight_list_after_second_item() {
            enter(&[(" - one\n\n - two|", " - one\n\n - two\n\n - |")]);
        }

        #[test]
        fn will_preserve_tight_list_after_second_item() {
            enter(&[(" - one\n - two|", " - one\n - two\n - |")]);
        }

        #[test]
        fn exits_tight_list_after_second_item() {
            // Divergence: the exit leaves a blank line after the paragraph.
            enter(&[(" - one\n - two\n - |", " - one\n - two\n\n|")]);
        }

        #[test]
        fn can_drop_list_markup_after_an_empty_line() {
            enter(&[(" - one\n\n - |", " - one\n\n|")]);
        }

        #[test]
        fn deletes_the_first_list_marker() {
            enter(&[(" - |", "|")]);
        }

        #[test]
        fn will_keep_the_current_ordered_list_number_when_moving_a_marker() {
            // Divergence: an exit, as for bullets.
            enter(&[(" 1. one\n 2. |", " 1. one\n\n|")]);
        }

        #[test]
        fn can_move_list_markup_inside_a_blockquote() {
            // Divergence: an exit that keeps the quote.
            enter(&[("> 1. one\n> 2. |", "> 1. one\n>\n> |")]);
        }

        #[test]
        fn can_preserve_list_tightness_inside_a_blockquote() {
            enter(&[("> 1. one\n>\n> 2. two|", "> 1. one\n>\n> 2. two\n>\n> 3. |")]);
        }

        #[test]
        fn renumbers_following_ordered_list_items() {
            enter(&[("1. one|\n2. two", "1. one\n2. |\n3. two")]);
        }

        #[test]
        fn renumbers_after_removed_markers() {
            enter(&[("1. one\n\n2. |\n\n3. three", "1. one\n\n|\n\n2. three")]);
        }

        #[test]
        fn stops_renumbering_on_discontinuities() {
            enter(&[(
                "1. one|\n2. two\n3. three\n1. four",
                "1. one\n2. |\n3. two\n4. three\n1. four",
            )]);
        }

        #[test]
        fn doesnt_fire_when_the_cursor_is_before_the_markup_depth() {
            // Divergence: CodeMirror declines; Wren plans the plain break.
            enter(&[("- a\n|bc", "- a\n\n|bc")]);
        }

        #[test]
        fn doesnt_fire_in_lazily_continued_paragraphs() {
            // Divergence: CodeMirror declines; Wren plans the plain break.
            enter(&[("1. a\nb c|", "1. a\nb c\n|")]);
        }

        #[test]
        fn continues_list_items() {
            enter(&[("- a\n  b|", "- a\n  b\n  |")]);
        }

        #[test]
        fn continues_dedented_list_items() {
            enter(&[("- hello\nhello|", "- hello\nhello\n|")]);
        }

        #[test]
        fn can_lift_out_of_one_list_level() {
            enter(&[("1. a\n\n   1. b\n\n   2. |", "1. a\n\n   1. b\n\n2. |")]);
        }

        #[test]
        fn can_lift_out_of_one_list_level_and_renumber() {
            enter(&[(
                "1. a\n\n   1. b\n\n   2. |\n\n2. d",
                "1. a\n\n   1. b\n\n2. |\n\n3. d",
            )]);
        }

        #[test]
        fn doesnt_treat_lines_with_content_after_the_cursor_as_empty() {
            enter(&[
                ("1. |x\n2. y", "1.\n2. |x\n3. y"),
                ("1. x\n2. |y", "1. x\n2.\n3. |y"),
            ]);
        }

        #[test]
        fn doesnt_take_effect_in_fenced_code() {
            // Divergence: code keeps its container's prefix.
            enter(&[
                ("- ```foo|", "- ```foo\n  |"),
                ("> - ```foo|", "> - ```foo\n>   |"),
            ]);
        }

        #[test]
        fn continues_nested_task_lists_at_the_right_level() {
            enter(&[(
                "- [ ] item 1\n  - [ ] item 1.1|",
                "- [ ] item 1\n  - [ ] item 1.1\n  - [ ] |",
            )]);
        }

        #[test]
        fn properly_indents_task_lists_nested_2_deep() {
            enter(&[(
                "- [ ] item 1\n  - [ ] item 1.1\n    - [ ] item 1.1.1|",
                "- [ ] item 1\n  - [ ] item 1.1\n    - [ ] item 1.1.1\n    - [ ] |",
            )]);
        }

        #[test]
        fn handles_tab_indentation() {
            enter(&[(" - one\n\t- two|", " - one\n\t- two\n\t- |")]);
        }
    }
}
