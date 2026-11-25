//! `markdown.toggle_task` (§5.7).

use std::collections::HashSet;

use super::*;
use crate::status::{self, StatusType};

/// What toggling does to one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Target {
    /// A task box on line `line` whose symbol is the bytes `at..end`.
    Task {
        line: usize,
        at: usize,
        end: usize,
        status: StatusType,
    },
    /// A list item without a box; one goes in at byte `at`.
    Item { line: usize, at: usize, space: bool },
    /// A plain line; `- [ ] ` goes in at byte `at`.
    Plain { line: usize, at: usize },
}

impl Target {
    fn place(&self) -> (usize, usize) {
        match *self {
            Self::Task { line, at, .. }
            | Self::Item { line, at, .. }
            | Self::Plain { line, at } => (line, at),
        }
    }
}

/// Toggle the task boxes on the lines the selection covers. With any
/// unfinished task (open or doing; a symbol without a status is open),
/// every task is marked done; otherwise every task, the cancelled ones
/// included, is marked open. Items and plain lines get a box only when no
/// line is a task. Code is left alone, and so are blank lines inside a
/// selection.
pub(super) fn toggle<L: Lines + ?Sized>(base: &L, sel: Selection, opts: &Options) -> EditPlan {
    let sel = clamp_selection(base, sel);
    let an = Analysis::new(base, None, opts.boxes());
    let mut d = Draft::new(base);
    let mut marks = sel;
    let (first, last) = if sel.is_empty() {
        (sel.head.line, sel.head.line)
    } else {
        covered(&sel)
    };
    let mut targets = targets(base, &an, sel, first, last);
    let any_task = targets.iter().any(|t| matches!(t, Target::Task { .. }));
    let mark_done = targets
        .iter()
        .any(|t| matches!(t, Target::Task { status, .. } if status.is_unfinished()));
    // Later edits on a line go first, so earlier byte offsets stay valid.
    targets.sort_by_key(|t| std::cmp::Reverse(t.place()));
    for target in targets {
        match target {
            // A done task keeps its own `x` or `X` when the rest join it.
            Target::Task {
                line,
                at,
                end,
                status,
            } if !(mark_done && status == StatusType::Done) => {
                let symbol = if mark_done {
                    status::DONE
                } else {
                    status::OPEN
                };
                edit_line(
                    &mut d,
                    line,
                    at..end,
                    symbol.encode_utf8(&mut [0; 4]),
                    &mut marks,
                );
            }
            Target::Item { line, at, space } if !any_task => {
                let with = if space { " [ ] " } else { "[ ] " };
                edit_line(&mut d, line, at..at, with, &mut marks);
            }
            Target::Plain { line, at } if !any_task => {
                edit_line(&mut d, line, at..at, "- [ ] ", &mut marks);
            }
            _ => {}
        }
    }
    finish(base, &d, marks)
}

/// What toggling does to each of lines `first..=last` of `base`, in the
/// order of the lines, each target once: `an` is the whole document's
/// analysis, and `sel` the selection. The lines' innermost containers are
/// read in one pass forwards, so a selection over a long item's lines
/// costs one pass, not one walk back to the item's first line per line.
pub(super) fn targets<L: Lines + ?Sized>(
    base: &L,
    an: &Analysis,
    sel: Selection,
    first: usize,
    last: usize,
) -> Vec<Target> {
    let setext = setext_text(an);
    let innermost = innermost(an, last);
    let mut targets: Vec<Target> = Vec::new();
    let mut seen = HashSet::new();
    for (j, innermost) in innermost.iter().enumerate().skip(first) {
        let info = &an.recs[j].info;
        if info.kind.is_code_like() {
            continue;
        }
        let blank = info.kind == Kind::Blank && info.item.is_none();
        if blank && !sel.is_empty() {
            continue;
        }
        let target = match *innermost {
            Some((k, Container::Item(item))) if !blank => match item.task {
                Some(task) => Target::Task {
                    line: k,
                    at: task.symbol_range().start,
                    end: task.symbol_range().end,
                    status: task.status,
                },
                None => Target::Item {
                    line: k,
                    at: item.text_start,
                    space: item.text_start == item.marker_end,
                },
            },
            _ if setext.get(j) == Some(&true)
                || matches!(
                    info.kind,
                    Kind::Heading { .. } | Kind::ThematicBreak | Kind::SetextUnderline { .. }
                ) =>
            {
                continue;
            }
            _ => Target::Plain {
                line: j,
                at: info.text_start.max(bom_len(j, base.line(j))),
            },
        };
        if seen.insert(target) {
            targets.push(target);
        }
    }
    targets
}

/// Each of lines `0..=last`'s innermost open container and the line that
/// opened it, as [`Analysis::opened`] finds it walking back, read in one
/// pass forwards: the latest line to open a container at each slot.
fn innermost(an: &Analysis, last: usize) -> Vec<Option<(usize, Container)>> {
    let mut latest: Vec<Option<(usize, Container)>> = Vec::new();
    let mut found = Vec::with_capacity(last + 1);
    for (j, rec) in an.recs.iter().enumerate().take(last + 1) {
        if latest.len() < rec.len {
            latest.resize(rec.len, None);
        }
        let opened = &an.opened[rec.opened_at..][..rec.len - rec.opened_from];
        for (slot, container) in latest[rec.opened_from..rec.len].iter_mut().zip(opened) {
            *slot = Some((j, *container));
        }
        found.push(rec.len.checked_sub(1).and_then(|slot| latest[slot]));
    }
    found
}

/// The targets as the toggle found them before [`targets`] read forwards:
/// walking back from each line to the line that opened its innermost
/// container. Kept to prove the two agree.
#[cfg(test)]
pub(super) fn targets_walking_back<L: Lines + ?Sized>(
    base: &L,
    an: &Analysis,
    sel: Selection,
    first: usize,
    last: usize,
) -> Vec<Target> {
    let setext = setext_text(an);
    let mut targets: Vec<Target> = Vec::new();
    let mut seen = HashSet::new();
    for j in first..=last {
        let info = &an.recs[j].info;
        if info.kind.is_code_like() {
            continue;
        }
        let blank = info.kind == Kind::Blank && info.item.is_none();
        if blank && !sel.is_empty() {
            continue;
        }
        let stack = an.open_stack(j);
        let target = match stack.last() {
            Some(Container::Item(_)) if !blank => {
                let (k, container) = an.opened(j, stack.len() - 1);
                let Container::Item(item) = container else {
                    continue;
                };
                match item.task {
                    Some(task) => Target::Task {
                        line: k,
                        at: task.symbol_range().start,
                        end: task.symbol_range().end,
                        status: task.status,
                    },
                    None => Target::Item {
                        line: k,
                        at: item.text_start,
                        space: item.text_start == item.marker_end,
                    },
                }
            }
            _ if setext.get(j) == Some(&true)
                || matches!(
                    info.kind,
                    Kind::Heading { .. } | Kind::ThematicBreak | Kind::SetextUnderline { .. }
                ) =>
            {
                continue;
            }
            _ => Target::Plain {
                line: j,
                at: info.text_start.max(bom_len(j, base.line(j))),
            },
        };
        if seen.insert(target) {
            targets.push(target);
        }
    }
    targets
}

/// The task line `l` belongs to: the line its box is on, and the box. A
/// line of a task item's own text (its first line, a continuation, a lazy
/// line) belongs to that task, as the toggle reads it. A blank line, code,
/// a line past the end and a line in no task item belong to none.
pub(crate) fn task_item<L: Lines + ?Sized>(
    base: &L,
    l: usize,
    boxes: Boxes,
) -> Option<(usize, block::Task)> {
    if l >= base.line_count() {
        return None;
    }
    task_in(&Analysis::through(base, l, boxes), l)
}

/// [`task_item`] for each of `wanted`, in order, from one scan of the
/// lines up to the last of them: what a change to many tasks reads, so it
/// costs one pass over the document however many lines it asks about.
pub(crate) fn task_items<L: Lines + ?Sized>(
    base: &L,
    wanted: &[usize],
    boxes: Boxes,
) -> Vec<Option<(usize, block::Task)>> {
    let count = base.line_count();
    let Some(last) = wanted.iter().copied().filter(|&l| l < count).max() else {
        return vec![None; wanted.len()];
    };
    let an = Analysis::through(base, last, boxes);
    let innermost = innermost(&an, last);
    let found: Vec<Option<(usize, block::Task)>> = (an.recs.iter().zip(&innermost))
        .map(|(rec, innermost)| {
            let info = &rec.info;
            let skipped =
                info.kind.is_code_like() || (info.kind == Kind::Blank && info.item.is_none());
            match innermost {
                Some((k, Container::Item(item))) if !skipped => item.task.map(|task| (*k, task)),
                _ => None,
            }
        })
        .collect();
    wanted
        .iter()
        .map(|&l| found.get(l).copied().flatten())
        .collect()
}

/// The task line `l` belongs to in `an`, which reaches at least line `l`.
fn task_in(an: &Analysis, l: usize) -> Option<(usize, block::Task)> {
    let info = &an.recs[l].info;
    if info.kind.is_code_like() || (info.kind == Kind::Blank && info.item.is_none()) {
        return None;
    }
    let slot = an.recs[l].len.checked_sub(1)?;
    match an.opened(l, slot) {
        (k, Container::Item(item)) => item.task.map(|task| (k, task)),
        (_, Container::Quote { .. }) => None,
    }
}

/// Which lines are the text of a setext heading: the lines from each
/// underline's `start` up to the underline.
fn setext_text(an: &Analysis) -> Vec<bool> {
    let mut text = vec![false; an.recs.len()];
    for (u, rec) in an.recs.iter().enumerate() {
        if let Kind::SetextUnderline { start, .. } = rec.info.kind {
            text[start..u].fill(true);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use crate::edit::tests::{apply, check, check_with, tasks_on};
    use crate::{plan, Document, Intent, Lines, Options, Pos, Selection};

    fn items(n: usize) -> String {
        (0..n).map(|i| format!("- item {i}\n  more\n")).collect()
    }

    fn select_all(doc: &Document) -> Selection {
        let last = doc.line_count() - 1;
        Selection::new(Pos::new(0, 0), Pos::new(last, doc.line(last).len()))
    }

    /// Select all of one list item 50,000 lines long and toggle, with
    /// tasks on and off: its lines are read in one pass forwards, so the
    /// plan takes well under a second even in a debug build.
    #[test]
    fn toggle_task_over_a_long_item_reads_it_once() {
        let text: String = std::iter::once("- item\n".to_owned())
            .chain((1..50_000).map(|i| format!("  more {i}\n")))
            .collect();
        let doc = Document::new(&text);
        for opts in [Options::default(), tasks_on()] {
            let started = std::time::Instant::now();
            let boxed = plan(&doc, select_all(&doc), Intent::ToggleTask, &opts);
            let elapsed = started.elapsed();
            assert!(
                elapsed < std::time::Duration::from_secs(3),
                "{elapsed:?} with {opts:?}"
            );
            assert_eq!(
                apply(&doc, &boxed),
                text.replacen("- item", "- [ ] item", 1)
            );
        }
    }

    #[test]
    fn toggle_task_over_many_items_boxes_each_once() {
        let text = items(2_000);
        let doc = Document::new(&text);
        let boxed = plan(
            &doc,
            select_all(&doc),
            Intent::ToggleTask,
            &Options::default(),
        );
        let after = apply(&doc, &boxed);
        assert_eq!(after, text.replace("- item", "- [ ] item"));
        let again = Document::new(&after);
        let done = plan(
            &again,
            select_all(&again),
            Intent::ToggleTask,
            &Options::default(),
        );
        assert_eq!(apply(&again, &done), text.replace("- item", "- [x] item"));
    }

    #[test]
    #[ignore = "micro benchmark; run with --release --ignored --nocapture"]
    fn benchmark_toggle_task_over_a_whole_document() {
        for n in [2_500, 5_000, 10_000, 20_000] {
            let text = items(n / 2);
            let doc = Document::new(&text);
            let start = std::time::Instant::now();
            plan(
                &doc,
                select_all(&doc),
                Intent::ToggleTask,
                &Options::default(),
            );
            println!("{n} lines: {:?}", start.elapsed());
        }
    }

    #[test]
    fn toggle_task_flips_a_box_or_adds_one() {
        check(
            Intent::ToggleTask,
            &[
                ("- [ ] a|", "- [x] a|"),
                ("- [x] a|", "- [ ] a|"),
                ("- [X] a|", "- [ ] a|"),
                ("- [|x] a", "- [| ] a"),
                ("- a|", "- [ ] a|"),
                ("- |a", "- [ ] |a"),
                ("1. a|", "1. [ ] a|"),
                ("> a|", "> - [ ] a|"),
                ("a|", "- [ ] a|"),
                ("|a", "- [ ] |a"),
                ("|", "- [ ] |"),
                ("-|", "- [ ] |"),
                ("- |", "- [ ] |"),
                ("> |", "> - [ ] |"),
                ("  text|", "  - [ ] text|"),
                ("- a\n  b|", "- [ ] a\n  b|"),
                ("- [ ] a\n  b|", "- [x] a\n  b|"),
                ("- a\nlazy|", "- [ ] a\nlazy|"),
                ("> - [ ] a|", "> - [x] a|"),
                ("- > a|", "- > - [ ] a|"),
                ("\u{feff}a|", "\u{feff}- [ ] a|"),
            ],
        );
    }

    /// The done toggle by status: an unfinished task (open, doing, or a
    /// symbol with no status) becomes done, a finished one (done or
    /// cancelled) open. The whole symbol goes, whatever its width.
    #[test]
    fn toggle_task_reads_the_status_and_replaces_the_whole_symbol() {
        check_with(
            Intent::ToggleTask,
            &tasks_on(),
            &[
                ("- [ ] a|", "- [x] a|"),
                ("- [/] a|", "- [x] a|"),
                ("- [x] a|", "- [ ] a|"),
                ("- [X] a|", "- [ ] a|"),
                ("- [-] a|", "- [ ] a|"),
                ("- [a] b|", "- [x] b|"),
                ("- [1] x|", "- [x] x|"),
                ("- [[] a|", "- [x] a|"),
                ("- [é] a|", "- [x] a|"),
                ("- [界] a|", "- [x] a|"),
                ("- [😀] a|", "- [x] a|"),
                ("|- [界] a", "|- [x] a"),
                ("- [界]| a", "- [x]| a"),
                ("- [界]\ta|", "- [x]\ta|"),
                ("- [界]|", "- [x]|"),
                ("1. [✓] a|", "1. [x] a|"),
                ("> - [/] a|", "> - [x] a|"),
                ("- [界] a\n  b|", "- [x] a\n  b|"),
                // Not boxes: the item gets one.
                ("- [✔\u{fe0f}] a|", "- [ ] [✔\u{fe0f}] a|"),
                ("- []] a|", "- [ ] []] a|"),
                ("- [a]: http://u|", "- [ ] [a]: http://u|"),
            ],
        );
        // Each status toggles to the symbol its row of the table names.
        for row in crate::status::STATUSES {
            let before = format!("- [{}] a|", row.symbol);
            let after = format!("- [{}] a|", row.next);
            check_with(Intent::ToggleTask, &tasks_on(), &[(&before, &after)]);
        }
        // Off, a box that is not one of the three is the item's text.
        check(
            Intent::ToggleTask,
            &[
                ("- [/] a|", "- [ ] [/] a|"),
                ("- [-] a|", "- [ ] [-] a|"),
                ("- [界] a|", "- [ ] [界] a|"),
                ("- [1] x|", "- [ ] [1] x|"),
            ],
        );
    }

    /// With any unfinished task in the selection every task becomes done,
    /// cancelled ones included; otherwise every task becomes open.
    #[test]
    fn toggle_task_over_a_selection_of_statuses() {
        check_with(
            Intent::ToggleTask,
            &tasks_on(),
            &[
                ("«- [ ] a\n- [-] b»", "«- [x] a\n- [x] b»"),
                ("«- [/] a\n- [-] b»", "«- [x] a\n- [x] b»"),
                ("«- [界] a\n- [-] b»", "«- [x] a\n- [x] b»"),
                ("«- [/] a\n- [x] b\n- [X] c»", "«- [x] a\n- [x] b\n- [X] c»"),
                ("«- [x] a\n- [-] b»", "«- [ ] a\n- [ ] b»"),
                ("«- [-] a\n- [-] b»", "«- [ ] a\n- [ ] b»"),
                ("«- [X] a\n- [-] b\n- [x] c»", "«- [ ] a\n- [ ] b\n- [ ] c»"),
                (
                    "«- [ ] a\n- [/] b\n- [x] c\n- [-] d\n- [😀] e»",
                    "«- [x] a\n- [x] b\n- [x] c\n- [x] d\n- [x] e»",
                ),
                // The selection's ends follow a symbol that changes width.
                ("- [界] a«b\n- [é] c»d", "- [x] a«b\n- [x] c»d"),
                ("- [«界] a\n- [é»] c", "- [«x] a\n- [x»] c"),
                // A plain item among tasks gets no box.
                ("«- [-] a\n- c»", "«- [ ] a\n- c»"),
                ("«- [/] a\nc»", "«- [x] a\nc»"),
            ],
        );
    }

    #[test]
    fn toggle_task_leaves_code_and_headings_alone() {
        check(
            Intent::ToggleTask,
            &[
                ("```\na|", "```\na|"),
                ("    code|", "    code|"),
                ("# T|", "# T|"),
                ("---|", "---|"),
                ("Foo|\n===", "Foo|\n==="),
                ("Foo\nbar|\n---", "Foo\nbar|\n---"),
                ("> Foo|\n> ===", "> Foo|\n> ==="),
            ],
        );
    }

    #[test]
    fn toggle_task_over_a_selection() {
        check(
            Intent::ToggleTask,
            &[
                ("«- [ ] a\n- [x] b»", "«- [x] a\n- [x] b»"),
                ("«- [x] a\n- [x] b»", "«- [ ] a\n- [ ] b»"),
                ("«- [ ] a\nc»", "«- [x] a\nc»"),
                ("«- [x] a\n- c»", "«- [ ] a\n- c»"),
                ("«a\n\nb»", "- [ ] «a\n\n- [ ] b»"),
                ("«- a\n\nb»", "«- [ ] a\n\n- [ ] b»"),
                ("«- a\nlazy»", "«- [ ] a\nlazy»"),
                ("«- a\n  b»", "«- [ ] a\n  b»"),
                ("«a\n»b", "- [ ] «a\n»b"),
                ("«```\nx»", "«```\nx»"),
                ("«Foo\n===\nbar»", "«Foo\n===\n- [ ] bar»"),
            ],
        );
    }
}
