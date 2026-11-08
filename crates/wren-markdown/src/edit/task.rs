//! `markdown.toggle_task` (§5.7).

use std::collections::HashSet;

use super::*;

/// What toggling does to one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Target {
    /// A task box whose mark is at byte `at` of line `line`.
    Task { line: usize, at: usize, done: bool },
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

/// Toggle the task boxes on the lines the selection covers. With any open
/// task, every task is marked done; otherwise every task is marked open.
/// Items and plain lines get a box only when no line is a task. Code is
/// left alone, and so are blank lines inside a selection.
pub(super) fn toggle<L: Lines + ?Sized>(base: &L, sel: Selection) -> EditPlan {
    let sel = clamp_selection(base, sel);
    let an = Analysis::new(base, None);
    let mut d = Draft::new(base);
    let mut marks = sel;
    let (first, last) = if sel.is_empty() {
        (sel.head.line, sel.head.line)
    } else {
        covered(&sel)
    };
    let setext = setext_text(&an);
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
                        at: task.start + 1,
                        done: task.done,
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
    let any_task = targets.iter().any(|t| matches!(t, Target::Task { .. }));
    let mark_done = targets
        .iter()
        .any(|t| matches!(t, Target::Task { done: false, .. }));
    // Later edits on a line go first, so earlier byte offsets stay valid.
    targets.sort_by_key(|t| std::cmp::Reverse(t.place()));
    for target in targets {
        match target {
            Target::Task { line, at, done } if done != mark_done => {
                edit_line(
                    &mut d,
                    line,
                    at..at + 1,
                    if mark_done { "x" } else { " " },
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
    use crate::edit::tests::{apply, check};
    use crate::{plan, Document, Intent, Lines, Options, Pos, Selection};

    fn items(n: usize) -> String {
        (0..n).map(|i| format!("- item {i}\n  more\n")).collect()
    }

    fn select_all(doc: &Document) -> Selection {
        let last = doc.line_count() - 1;
        Selection::new(Pos::new(0, 0), Pos::new(last, doc.line(last).len()))
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
