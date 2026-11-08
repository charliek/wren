//! Tab and Shift+Tab (§5.4).

use super::*;

/// Tab (`forward`) or Shift+Tab at `sel`.
pub(super) fn indent<L: Lines + ?Sized>(
    base: &L,
    sel: Selection,
    forward: bool,
    opts: &Options,
) -> EditPlan {
    let sel = clamp_selection(base, sel);
    let an = Analysis::new(base, None);
    let mut d = Draft::new(base);
    let mut marks = sel;
    let mut runs = Vec::new();
    let (first, last) = covered(&sel);
    let width = opts.indent_width.max(1);
    if first < last {
        shift_selection(
            &mut d, &an, first, last, forward, width, &mut marks, &mut runs,
        );
    } else {
        let l = first;
        let kind = &an.recs[l].info.kind;
        if let Some(slot) = an.item_slot(l) {
            if forward {
                tab_item(&mut d, &an, l, slot, &mut marks, &mut runs);
            } else {
                outdent_item(&mut d, &an, l, slot, &mut marks, &mut runs);
            }
        } else if !forward && kind.is_code_like() {
            outdent_code(&mut d, &an, l, width, &mut marks);
        } else if !forward {
            outdent_line(&mut d, &an, l, width, &mut marks);
        } else if !sel.is_empty() {
            indent_line(&mut d, &an, l, width, &mut marks);
        } else if kind.is_code_like() {
            code_tab(&mut d, &an, sel.head, width, &mut marks);
        } else if *kind == Kind::Blank && sel.head.col == d.line(l).len() {
            smart_tab(&mut d, &an, l, width, &mut marks);
        } else {
            plain_tab(&mut d, &an, sel.head, width, &mut marks);
        }
    }
    if opts.renumber {
        renumber_runs(&mut d, &runs, &[], &mut marks);
    }
    finish(base, &d, marks)
}

/// Tab on a list line: the item and its subtree become children of the
/// previous sibling, or nothing happens when there is none.
fn tab_item(
    d: &mut Draft,
    an: &Analysis,
    l: usize,
    slot: usize,
    marks: &mut Selection,
    runs: &mut Vec<Run>,
) {
    let (Some((p, prev)), Some(item)) = (an.prev_sibling(l, slot), an.item_here(l, slot)) else {
        return;
    };
    // An existing nested list under the sibling sets the marker; otherwise
    // the item starts a new list of its own kind.
    let marker = last_child(an, p, l, slot)
        .and_then(|(c, child)| next_marker(an, c, slot + 1, &child))
        .map_or_else(|| first_marker(&item), |(marker, ..)| marker);
    move_item(
        d,
        an,
        l,
        slot,
        &item,
        slot,
        prev.content_column,
        &marker,
        marks,
    );
    runs.push(Run {
        line: l,
        slot: Some(slot + 1),
    });
    runs.push(Run {
        line: p,
        slot: Some(slot),
    });
}

/// The last child item at `slot + 1` of the item opened on line `p`,
/// before line `l`.
fn last_child(an: &Analysis, p: usize, l: usize, slot: usize) -> Option<(usize, ListItem)> {
    (p + 1..l)
        .rev()
        .find_map(|j| an.item_here(j, slot + 1).map(|item| (j, item)))
}

/// Shift+Tab on a list line: the item and its subtree move to the parent's
/// indent with the parent level's marker, and its later siblings become
/// its children. A top-level item moves to its container's column 0.
/// Returns whether anything changed.
pub(super) fn outdent_item(
    d: &mut Draft,
    an: &Analysis,
    l: usize,
    slot: usize,
    marks: &mut Selection,
    runs: &mut Vec<Run>,
) -> bool {
    let Some(item) = an.item_here(l, slot) else {
        return false;
    };
    let text = d.line(l).to_owned();
    let at = an.line_at(l, &text);
    let parent = match slot.checked_sub(1).map(|s| at.stack[s]) {
        Some(Container::Item(parent)) => parent,
        _ => {
            let base = block::column(&text, at.segs[slot].start);
            if item.indent <= base {
                return false;
            }
            let marker = text[item.marker_start..item.marker_end].to_owned();
            move_item(d, an, l, slot, &item, slot, base, &marker, marks);
            return true;
        }
    };
    let (pl, _) = an.opened(l, slot - 1);
    let marker = next_marker(an, pl, slot - 1, &parent).map_or_else(
        || text[item.marker_start..item.marker_end].to_owned(),
        |(marker, ..)| marker,
    );
    let orphan = an.next_sibling(l, slot);
    let delta = move_item(
        d,
        an,
        l,
        slot,
        &item,
        slot - 1,
        parent.indent,
        &marker,
        marks,
    );
    if let Some((o, _)) = orphan {
        // The later siblings stay where they are, which nests them under
        // the moved item unless its marker grew past them.
        let content = item.content_column as isize + delta;
        let short = content - item.indent as isize;
        if short > 0 {
            shift_lines(d, an, o, slot - 1, short, true, marks);
        }
        let ws = an.line_at(o, d.line(o)).ws;
        set_number(d, o, ws, 1, marks);
        runs.push(Run {
            line: o,
            slot: Some(slot),
        });
    }
    runs.push(Run {
        line: l,
        slot: Some(slot - 1),
    });
    true
}

/// Rewrite the first line of `item` (opened on line `l` at `slot`) with
/// `marker` at column `target`, rewriting the indentation that starts at
/// container `region`'s share, and move the item's subtree by the same
/// amount. Returns how far the item's content column moved.
#[allow(clippy::too_many_arguments)]
fn move_item(
    d: &mut Draft,
    an: &Analysis,
    l: usize,
    slot: usize,
    item: &ListItem,
    region: usize,
    target: usize,
    marker: &str,
    marks: &mut Selection,
) -> isize {
    let text = d.line(l).to_owned();
    let at = an.line_at(l, &text);
    let from = at.segs[region].start;
    let tabs = text[at.ws..].starts_with('\t');
    let old = item.marker_end - item.marker_start;
    edit_line(d, l, item.marker_start..item.marker_end, marker, marks);
    let text = d.line(l).to_owned();
    let (_, cur) = ws_run(&text, from);
    let target = tab_target(cur, target, tabs && target > cur);
    let (range, pad) = reindent(&text, from, target, tabs);
    let indent = block::advance_column(block::column(&text, range.start), &pad);
    edit_line(d, l, range, &pad, marks);
    let delta = indent as isize - item.indent as isize + marker.len() as isize - old as isize;
    shift_lines(d, an, l + 1, slot, delta, true, marks);
    delta
}

/// Tab or Shift+Tab over several lines: one delta for every non-blank
/// line, taken from the first list line (what Tab or Shift+Tab would move
/// it by), or the indent width when no line is a list line.
#[allow(clippy::too_many_arguments)]
fn shift_selection(
    d: &mut Draft,
    an: &Analysis,
    first: usize,
    last: usize,
    forward: bool,
    width: usize,
    marks: &mut Selection,
    runs: &mut Vec<Run>,
) {
    let first_item = (first..=last).find_map(|j| an.item_slot(j).map(|slot| (j, slot)));
    let delta = match first_item {
        None => width,
        Some((j, slot)) => {
            let item = an.item_here(j, slot).expect("the line opens an item here");
            if forward {
                an.prev_sibling(j, slot).map_or(0, |(_, prev)| {
                    prev.content_column.saturating_sub(item.indent)
                })
            } else {
                let text = d.line(j);
                let at = an.line_at(j, text);
                let to = match slot.checked_sub(1).map(|s| at.stack[s]) {
                    Some(Container::Item(parent)) => parent.indent,
                    _ => block::column(text, at.segs[slot].start),
                };
                item.indent.saturating_sub(to)
            }
        }
    };
    if delta == 0 {
        return;
    }
    for j in first..=last {
        if an.recs[j].info.kind == Kind::Blank && an.item_slot(j).is_none() {
            continue;
        }
        let text = d.line(j).to_owned();
        let at = an.line_at(j, &text);
        let base = block::column(&text, at.ws);
        let (_, cur) = ws_run(&text, at.ws);
        let tabs = text[at.ws..].starts_with('\t');
        let target = if forward {
            tab_target(cur, cur + delta, tabs)
        } else {
            cur - delta.min(cur - base)
        };
        let (range, pad) = reindent(&text, at.ws, target, tabs);
        edit_line(d, j, range, &pad, marks);
    }
    let Some((j, slot)) = first_item else {
        return;
    };
    if forward {
        let Some((p, _)) = an.prev_sibling(j, slot) else {
            return;
        };
        if last_child(an, p, j, slot).is_none() {
            let ws = an.line_at(j, d.line(j)).ws;
            set_number(d, j, ws, 1, marks);
        }
        runs.push(Run {
            line: p,
            slot: Some(slot),
        });
    } else {
        let parent = slot.checked_sub(1).map(|s| an.opened(j, s));
        if let Some((pl, Container::Item(parent))) = parent {
            let item = an.item_here(j, slot).expect("the line opens an item here");
            if let (Some((_, n, _)), true) = (
                next_marker(an, pl, slot - 1, &parent),
                same_list(&item, &parent),
            ) {
                let ws = an.line_at(j, d.line(j)).ws;
                set_number(d, j, ws, n, marks);
            }
        }
        runs.push(Run {
            line: j,
            slot: None,
        });
        let mut after = j;
        while let Some((k, _)) = an.next_sibling(after, slot) {
            if k > last {
                let ws = an.line_at(k, d.line(k)).ws;
                set_number(d, k, ws, 1, marks);
                runs.push(Run {
                    line: k,
                    slot: None,
                });
                break;
            }
            after = k;
        }
    }
    for k in first..=last {
        runs.push(Run {
            line: k,
            slot: None,
        });
    }
}

/// Tab with a selection on one plain line: the line moves in by one step.
fn indent_line(d: &mut Draft, an: &Analysis, l: usize, width: usize, marks: &mut Selection) {
    let text = d.line(l).to_owned();
    let at = an.line_at(l, &text);
    let (_, cur) = ws_run(&text, at.ws);
    let tabs = text[at.ws..].starts_with('\t');
    let (range, pad) = reindent(&text, at.ws, tab_target(cur, cur + width, tabs), tabs);
    edit_line(d, l, range, &pad, marks);
}

/// Shift+Tab on a plain line: its indentation goes back to the previous
/// stop.
fn outdent_line(d: &mut Draft, an: &Analysis, l: usize, width: usize, marks: &mut Selection) {
    let text = d.line(l).to_owned();
    let at = an.line_at(l, &text);
    let base = block::column(&text, at.ws);
    let (_, cur) = ws_run(&text, at.ws);
    if cur <= base {
        return;
    }
    let target = previous_stop(cur, base, &at.item_stops(), width);
    let (range, pad) = reindent(&text, at.ws, target, false);
    edit_line(d, l, range, &pad, marks);
}

/// Shift+Tab in code: the line moves out one step, measured from the
/// block's own indent.
fn outdent_code(d: &mut Draft, an: &Analysis, l: usize, width: usize, marks: &mut Selection) {
    let text = d.line(l).to_owned();
    let from = an.recs[l].info.prefix_end.max(bom_len(l, &text));
    let base = block::column(&text, from);
    let (_, cur) = ws_run(&text, from);
    if cur <= base {
        return;
    }
    let target = base + (cur - base - 1) / width * width;
    let (range, pad) = reindent(&text, from, target, false);
    edit_line(d, l, range, &pad, marks);
}

/// Tab in code: a tab when the code is indented with tabs, else spaces to
/// the next stop counted from the block's own indent.
fn code_tab(d: &mut Draft, an: &Analysis, pos: Pos, width: usize, marks: &mut Selection) {
    let l = pos.line;
    let text = d.line(l).to_owned();
    let from = an.recs[l].info.prefix_end.max(bom_len(l, &text));
    let c = pos.col.max(from);
    let starts_with_tab = |j: usize| {
        let line = d.line(j);
        let from = an.recs[j].info.prefix_end.min(line.len());
        line[from..].starts_with('\t')
    };
    let body = |j: usize| {
        matches!(an.recs[j].info.kind, Kind::FenceBody | Kind::IndentedCode)
            && !is_ws(&d.line(j)[an.recs[j].info.prefix_end.min(d.line(j).len())..])
    };
    let code = |j: &usize| an.recs[*j].info.kind.is_code_like();
    let nearest = (0..l)
        .rev()
        .take_while(code)
        .find(|&j| body(j))
        .or_else(|| (l + 1..an.recs.len()).take_while(code).find(|&j| body(j)));
    let insert = if starts_with_tab(l) || nearest.is_some_and(starts_with_tab) {
        "\t".to_owned()
    } else {
        let base = block::column(&text, from);
        let col = block::column(&text, c);
        " ".repeat(width - (col - base) % width)
    };
    edit_line(d, l, c..c, &insert, marks);
    *marks = Selection::caret(Pos::new(l, c + insert.len()));
}

/// Tab on a plain line: spaces to the next stop at the cursor, or a tab
/// when the line is indented with tabs.
fn plain_tab(d: &mut Draft, an: &Analysis, pos: Pos, width: usize, marks: &mut Selection) {
    let l = pos.line;
    let text = d.line(l).to_owned();
    let at = an.line_at(l, &text);
    let c = pos.col.max(at.ws);
    let insert = if text[at.ws..].starts_with('\t') {
        "\t".to_owned()
    } else {
        let base = block::column(&text, at.ws);
        let col = block::column(&text, c);
        " ".repeat(width - (col - base) % width)
    };
    edit_line(d, l, c..c, &insert, marks);
    *marks = Selection::caret(Pos::new(l, c + insert.len()));
}

/// Tab on a blank line: the first press pads to the nearest stop deeper
/// than the line's indent, from the previous non-blank line's indent and
/// content column and the enclosing items; later presses add a step.
fn smart_tab(d: &mut Draft, an: &Analysis, l: usize, width: usize, marks: &mut Selection) {
    let text = d.line(l).to_owned();
    let at = an.line_at(l, &text);
    let base = block::column(&text, at.ws);
    let cur = block::column(&text, text.len());
    let target = if l == 0 {
        base + ((cur - base) / width + 1) * width
    } else {
        let mut stops = at.item_stops();
        if let Some(j) = (0..l).rev().find(|&j| an.recs[j].info.kind != Kind::Blank) {
            let prev = d.line(j);
            let prev_at = an.line_at(j, prev);
            stops.push(ws_run(prev, prev_at.ws).1);
            stops.push(an.recs[j].info.content_column);
        }
        stops
            .into_iter()
            .filter(|&s| s > cur)
            .min()
            .unwrap_or(cur + width)
    };
    let tabs = text[at.ws..].starts_with('\t');
    let pad = padding(cur, tab_target(cur, target, tabs), tabs);
    let end = text.len();
    edit_line(d, l, end..end, &pad, marks);
}

#[cfg(test)]
mod tests {
    use crate::edit::tests::{check, check_with};
    use crate::{Intent, Options};

    #[test]
    fn tab_nests_an_item_under_its_previous_sibling() {
        check(
            Intent::Tab,
            &[
                ("- a\n- b|", "- a\n  - b|"),
                ("- a\n- b|c", "- a\n  - b|c"),
                ("- a\n|- b", "- a\n  |- b"),
                ("- a\n- [x] b|", "- a\n  - [x] b|"),
                ("1. a\n2. b|", "1. a\n   1. b|"),
                ("1. a\n   1. x\n2. b|", "1. a\n   1. x\n   2. b|"),
                ("- a\n  * x\n- b|", "- a\n  * x\n  * b|"),
                ("1. a\n   - x\n2. b|", "1. a\n   - x\n   - b|"),
                ("> - a\n> - b|", "> - a\n>   - b|"),
                ("- a\n\n- b|", "- a\n\n  - b|"),
                ("  - a\n  - b|", "  - a\n    - b|"),
                ("9. a\n10. b|", "9. a\n   1. b|"),
            ],
        );
    }

    #[test]
    fn tab_moves_the_subtree_and_renumbers_both_runs() {
        check(
            Intent::Tab,
            &[
                ("- a\n- b|\n  - c", "- a\n  - b|\n    - c"),
                ("- a\n- b|\n  more\n- d", "- a\n  - b|\n    more\n- d"),
                ("1. a\n2. b|\n3. c", "1. a\n   1. b|\n2. c"),
                (
                    "1. a\n   1. x\n2. b|\n3. c\n4. d",
                    "1. a\n   1. x\n   2. b|\n2. c\n3. d",
                ),
                ("1. a\n1. b|\n1. c", "1. a\n   1. b|\n1. c"),
                (
                    "1. a\n2. b|\n   - c\n3. d",
                    "1. a\n   1. b|\n      - c\n2. d",
                ),
            ],
        );
    }

    #[test]
    fn tab_without_a_previous_sibling_does_nothing() {
        check(
            Intent::Tab,
            &[
                ("- a|", "- a|"),
                ("x\n\n- a|", "x\n\n- a|"),
                ("- a\n  - b|", "- a\n  - b|"),
                ("> - a|", "> - a|"),
            ],
        );
    }

    #[test]
    fn tab_keeps_tab_indentation() {
        check(
            Intent::Tab,
            &[
                ("- x\n\t- a\n\t- b|", "- x\n\t- a\n\t\t- b|"),
                ("- a\n- b|\n\t  c", "- a\n  - b|\n\t    c"),
            ],
        );
    }

    #[test]
    fn shift_tab_moves_an_item_out_to_its_parent_level() {
        check(
            Intent::ShiftTab,
            &[
                ("- a\n  - b|", "- a\n- b|"),
                ("- a\n  - b|\n    - c", "- a\n- b|\n  - c"),
                ("1. a\n   - [x] b|", "1. a\n2. [x] b|"),
                ("1. a\n   1. x|\n2. b", "1. a\n2. x|\n3. b"),
                (
                    "1. a\n   1. x\n   2. y|\n   3. z",
                    "1. a\n   1. x\n2. y|\n   1. z",
                ),
                ("- a\n  - b|\n  - c", "- a\n- b|\n  - c"),
                ("> - a\n>   - b|", "> - a\n> - b|"),
                ("- [ ] a\n  - b|", "- [ ] a\n- b|"),
                ("  - a\n\t- b|", "  - a\n  - b|"),
                ("9. a\n   - b|\n   - c", "9. a\n10. b|\n    - c"),
            ],
        );
    }

    #[test]
    fn shift_tab_on_a_top_level_item_moves_it_to_column_0() {
        check(
            Intent::ShiftTab,
            &[
                ("  - a|", "- a|"),
                (" 1. a|\n    b", "1. a|\n   b"),
                ("- a|", "- a|"),
                ("> - a|", "> - a|"),
                (">   - a|", "> - a|"),
            ],
        );
    }

    #[test]
    fn tab_on_a_blank_line_pads_to_the_nearest_deeper_stop() {
        check(
            Intent::Tab,
            &[
                ("- [ ] task\n|", "- [ ] task\n  |"),
                ("- [ ] task\n  |", "- [ ] task\n      |"),
                ("|", "    |"),
                ("  |", "    |"),
                ("- a\n  - b\n|", "- a\n  - b\n  |"),
                ("- a\n  - b\n  |", "- a\n  - b\n    |"),
                ("para\n|", "para\n    |"),
                ("> - a\n> |", "> - a\n>   |"),
                ("1. a\n\n|", "1. a\n\n   |"),
            ],
        );
    }

    #[test]
    fn tab_on_a_plain_line_inserts_to_the_next_stop() {
        check(
            Intent::Tab,
            &[
                ("ab|", "ab  |"),
                ("a|b", "a   |b"),
                ("abcd|", "abcd    |"),
                ("\tx|", "\tx\t|"),
                ("> ab|", "> ab  |"),
                ("|> a", ">     |a"),
                ("# T|", "# T |"),
                ("- a\n  b|", "- a\n  b |"),
            ],
        );
    }

    #[test]
    fn shift_tab_on_a_plain_line_outdents_to_the_previous_stop() {
        check(
            Intent::ShiftTab,
            &[
                ("   x|", "x|"),
                ("- a\n    b|", "- a\n  b|"),
                ("x|", "x|"),
                ("> \tx|", "> x|"),
                ("- a\n  - b\n     c|", "- a\n  - b\n    c|"),
            ],
        );
    }

    #[test]
    fn tab_in_code() {
        check(
            Intent::Tab,
            &[
                ("```\ncode|", "```\ncode    |"),
                ("```\nab|", "```\nab  |"),
                ("```\n\tcode|", "```\n\tcode\t|"),
                ("```\nx|\n\ty", "```\nx\t|\n\ty"),
                ("```\n\ty\n\nx|", "```\n\ty\n\nx\t|"),
                ("- ```\n  ab|", "- ```\n  ab  |"),
                ("    code|", "    code    |"),
                ("```\n- a\n- b|", "```\n- a\n- b |"),
            ],
        );
        check(
            Intent::ShiftTab,
            &[
                ("```\n      x|", "```\n    x|"),
                ("```\n\tx|", "```\nx|"),
                ("```\nx|", "```\nx|"),
                ("- ```\n      x|", "- ```\n  x|"),
            ],
        );
    }

    #[test]
    fn tab_with_a_selection_on_one_line_indents_it() {
        check(
            Intent::Tab,
            &[
                ("«ab»", "    «ab»"),
                ("a«b»", "    a«b»"),
                ("- a\n- «b»", "- a\n  - «b»"),
                ("> «x»", ">     «x»"),
                ("«a\n»b", "    «a\n»b"),
            ],
        );
        check(
            Intent::ShiftTab,
            &[("    «ab»", "«ab»"), ("- a\n  - «b»", "- a\n- «b»")],
        );
    }

    #[test]
    fn tab_over_several_lines_shifts_each_by_one_delta() {
        check(
            Intent::Tab,
            &[
                ("- a\n«- b\n- c»", "- a\n  «- b\n  - c»"),
                ("- a\n- «b\n- c»", "- a\n  - «b\n  - c»"),
                ("«a\nb\n»c", "    «a\n    b\n»c"),
                ("«a\n\nb»", "    «a\n\n    b»"),
                ("«- a\n- b»", "«- a\n- b»"),
                ("1. a\n«2. b\n3. c»", "1. a\n   «1. b\n   2. c»"),
                ("1. a\n«2. b\n3. c»\n4. d", "1. a\n   «1. b\n   2. c»\n2. d"),
                (
                    "- x\n\t- a\n«\t- b\n\t- c»",
                    "- x\n\t- a\n«\t\t- b\n\t\t- c»",
                ),
            ],
        );
    }

    #[test]
    fn shift_tab_over_several_lines_shifts_each_by_one_delta() {
        check(
            Intent::ShiftTab,
            &[
                ("- a\n  «- b\n  - c»", "- a\n«- b\n- c»"),
                ("«  a\n      b»", "«a\n  b»"),
                ("«x\ny»", "«x\ny»"),
                ("«- a\n- b»", "«- a\n- b»"),
                (
                    "1. a\n   «1. b\n   2. c»\n   3. d",
                    "1. a\n«2. b\n3. c»\n   1. d",
                ),
            ],
        );
    }

    #[test]
    fn tab_and_shift_tab_follow_the_indent_width() {
        let opts = Options {
            indent_width: 2,
            ..Options::default()
        };
        check_with(Intent::Tab, &opts, &[("a|", "a |"), ("«a»", "  «a»")]);
        check_with(Intent::ShiftTab, &opts, &[("     a|", "    a|")]);
    }

    #[test]
    fn tab_renumbering_can_be_turned_off() {
        let opts = Options {
            renumber: false,
            ..Options::default()
        };
        check_with(
            Intent::Tab,
            &opts,
            &[("1. a\n2. b|\n3. c", "1. a\n   1. b|\n3. c")],
        );
    }
}
