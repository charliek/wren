//! Line edits to every task a selection covers: the done toggle with its
//! stamps, and the joining of several [`LineEdit`]s into one plan, so an
//! editor makes them one undo step. Each reads the document once, however
//! many lines it covers: finding the tasks, planning their edits and
//! checking them before they apply all share one scan.

use std::collections::BTreeMap;
use std::fmt;

use super::change::{edit_task, Change, LineEdit, Stale};
use crate::block::Boxes;
use crate::status::{self, StatusType};
use crate::{EditPlan, Lines, Options, Selection};

/// Why a batch of line edits was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A line has changed since its edit was planned ([`Stale`]).
    Stale,
    /// The edits do not fit together: two on one line, or a replacement
    /// that is no place in the text.
    Invalid,
}

impl From<Stale> for Refused {
    fn from(_: Stale) -> Self {
        Self::Stale
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => Stale.fmt(f),
            Self::Invalid => f.write_str("the task edits do not fit the text"),
        }
    }
}

impl std::error::Error for Refused {}

/// The done toggle over the tasks `sel` covers, with the stamps each
/// status change asks for, as [`super::edit`] writes them: one
/// [`LineEdit`] per task, in the order of their boxes' lines.
///
/// The rule is the one the plain toggle (`Intent::ToggleTask`) has for a
/// selection: with any unfinished task among them (open or doing, a symbol
/// with no status being open), every task that is not done becomes done
/// (`x`), and a done one keeps its own `x` or `X`; otherwise every task,
/// the cancelled ones included, becomes open. On one task it is
/// [`Change::ToggleDone`].
///
/// A task belongs to the selection when a line of its own text does (its
/// box's line, a continuation, a lazy line), and it is toggled once however
/// many of its lines the selection covers, other tasks' lines among them
/// (a parent's text after its child's); a selection that ends at column 0
/// of a line does not cover that line, and a blank line in no item is
/// skipped. `None` with tasks off in `opts` and when the selection covers
/// no task, where the plain toggle gives the lines boxes instead;
/// [`Refused::Invalid`] when a task's edit cannot be planned.
pub fn toggle<L: Lines + ?Sized>(
    lines: &L,
    sel: Selection,
    opts: &Options,
) -> Option<Result<Vec<LineEdit>, Refused>> {
    opts.tasks?;
    let count = lines.line_count();
    if count == 0 {
        return None;
    }
    let (start, end) = (sel.start(), sel.end());
    let first = start.line.min(count - 1);
    let mut last = end.line.min(count - 1);
    if end.line > start.line && end.col == 0 {
        last = last.saturating_sub(1).max(first);
    }
    let covered: Vec<usize> = (first..=last).collect();
    let tasks: BTreeMap<usize, crate::block::Task> =
        crate::edit::task_items(lines, &covered, opts.boxes())
            .into_iter()
            .flatten()
            .collect();
    if tasks.is_empty() {
        return None;
    }
    let mark_done = tasks.values().any(|task| task.status.is_unfinished());
    let edits = tasks.into_iter().map(|(k, task)| {
        let symbol = match (mark_done, task.status) {
            (true, StatusType::Done) => task.symbol,
            (true, _) => status::DONE,
            (false, _) => status::OPEN,
        };
        edit_task(lines, k, task, sel, &Change::SetStatus(symbol), opts).ok_or(Refused::Invalid)
    });
    Some(edits.collect())
}

/// `edits`, each planned on `lines` at one selection and each on a line of
/// its own (as [`toggle`] gives them), as one plan: every edit's
/// replacement, and the selection with each end moved by the edit of the
/// line it is on. [`Refused::Stale`] when any edit's line has changed since
/// ([`LineEdit::plan_for`], checked for them all in one scan), and
/// [`Refused::Invalid`] for two edits on one line; an empty list is a plan
/// with no edit at an empty selection.
pub fn join<L: Lines + ?Sized>(edits: &[LineEdit], lines: &L) -> Result<EditPlan, Refused> {
    let Some(first) = edits.first() else {
        return Ok(EditPlan {
            edits: Vec::new(),
            selection: Selection::default(),
        });
    };
    let mut at: Vec<usize> = edits.iter().map(|edit| edit.line).collect();
    if !edits.iter().all(|edit| edit.reads(lines)) {
        return Err(Refused::Stale);
    }
    let found = crate::edit::task_items(lines, &at, Boxes::Status);
    if (edits.iter().zip(&found)).any(|(edit, found)| *found != Some((edit.line, edit.task))) {
        return Err(Refused::Stale);
    }
    at.sort_unstable();
    if at.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Refused::Invalid);
    }
    let base = first.plan.selection;
    let by_line: BTreeMap<usize, &LineEdit> = edits.iter().map(|edit| (edit.line, edit)).collect();
    let mapped = |end: crate::Pos, pick: fn(&Selection) -> crate::Pos| {
        by_line
            .get(&end.line)
            .map_or(end, |edit| pick(&edit.plan.selection))
    };
    // Each edit moved only the ends on its own line, so each end is read
    // from the edit of its line, or is where every plan left it.
    let selection = Selection::new(
        mapped(base.anchor, |sel| sel.anchor),
        mapped(base.head, |sel| sel.head),
    );
    let mut replaces: Vec<_> = edits
        .iter()
        .flat_map(|edit| edit.plan.edits.iter().cloned())
        .collect();
    replaces.sort_by_key(|replace| replace.start);
    Ok(EditPlan {
        edits: replaces,
        selection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::tests::{apply, parse, render, tasks_on};
    use crate::task::edit;
    use crate::Document;

    /// The toggle over the fixture's selection, joined and applied, in the
    /// `|`, `«` and `»` notation; `None` when it covers no task.
    fn toggled(before: &str) -> Option<String> {
        let (text, sel) = parse(before);
        let doc = Document::new(&text);
        let edits = toggle(&doc, sel, &tasks_on())?.expect("every task's edit plans");
        let plan = join(&edits, &doc).expect("the lines it was planned on");
        Some(render(&apply(&doc, &plan), plan.selection))
    }

    #[test]
    fn one_task_toggles_as_toggle_done_does() {
        for before in [
            "- [ ] a|",
            "- [/] a|",
            "- [界] a|",
            "- [x] a ✅ 2026-11-18|",
            "- [X] a|",
            "- [-] a ❌ 2026-11-18|",
            "- [ ] a 📅 2026-12-01 ^abc|",
            "- [ ] a|b",
            "|- [ ] a",
            "- [x] a ✅ 2026-1|1-18",
        ] {
            let (text, sel) = parse(before);
            let doc = Document::new(&text);
            let single = edit(&doc, sel.head.line, sel, &Change::ToggleDone, &tasks_on()).unwrap();
            let expected = render(&apply(&doc, &single.plan), single.plan.selection);
            assert_eq!(toggled(before).as_deref(), Some(&*expected), "{before:?}");
        }
    }

    #[test]
    fn a_selection_follows_the_plain_toggles_rule_with_stamps() {
        for (before, after) in [
            // An unfinished task makes every task done; a done one keeps
            // its own symbol and stamp, and a cancelled one becomes done.
            (
                "«- [ ] a\n- [-] b ❌ 2026-11-18\n- [X] c ✅ 2026-11-01\n- [/] d»",
                "«- [x] a ✅ 2026-11-19\n- [x] b ✅ 2026-11-19\n- [X] c ✅ 2026-11-01\n- [x] d» ✅ 2026-11-19",
            ),
            // With none, every task opens and loses its stamps.
            (
                "«- [x] a ✅ 2026-11-18\n- [-] b ❌ 2026-11-18»",
                "«- [ ] a\n- [ ] b»",
            ),
            // A backwards selection keeps its direction.
            (
                "»- [ ] a\n- [ ] b«",
                "»- [x] a ✅ 2026-11-19\n- [x] b« ✅ 2026-11-19",
            ),
            // Plain lines and items in between are left alone, and so is a
            // line the selection only reaches at column 0.
            (
                "«- [ ] a\nprose\n- item\n»- [ ] b",
                "«- [x] a ✅ 2026-11-19\nprose\n- item\n»- [ ] b",
            ),
            // A continuation line selects its task once.
            (
                "- [ ] a\n  mo«re\n  m»ore\n- [ ] b",
                "- [x] a ✅ 2026-11-19\n  mo«re\n  m»ore\n- [ ] b",
            ),
            // A parent's text after its child's is the parent's still: the
            // parent toggles once, and so does the child.
            (
                "«- [ ] parent\n  - [ ] child\n\n  parent continuation»",
                "«- [x] parent ✅ 2026-11-19\n  - [x] child ✅ 2026-11-19\n\n  parent continuation»",
            ),
            (
                "«- [x] parent\n  - [ ] child\n  more child\n\n  parent again\n- [-] next»",
                "«- [x] parent\n  - [x] child ✅ 2026-11-19\n  more child\n\n  parent again\n- [x] next» ✅ 2026-11-19",
            ),
            // Code is no task.
            (
                "«```\n- [ ] in code\n```\n- [ ] a»",
                "«```\n- [ ] in code\n```\n- [x] a» ✅ 2026-11-19",
            ),
        ] {
            assert_eq!(toggled(before).as_deref(), Some(after), "{before:?}");
        }
    }

    #[test]
    fn no_task_and_tasks_off_give_none() {
        for before in [
            "prose|",
            "- item|",
            "«prose\n- item»",
            "|",
            "```\n- [ ] a|\n```",
        ] {
            assert_eq!(toggled(before), None, "{before:?}");
        }
        let (text, sel) = parse("- [ ] a|");
        let doc = Document::new(&text);
        assert_eq!(toggle(&doc, sel, &Options::default()), None);
    }

    #[test]
    fn a_join_is_refused_once_any_line_has_changed() {
        let (text, sel) = parse("«- [ ] a\n- [ ] b»");
        let doc = Document::new(&text);
        let edits = toggle(&doc, sel, &tasks_on()).unwrap().unwrap();
        assert_eq!(edits.len(), 2);
        let changed = Document::new("- [ ] a\n- [ ] b2");
        assert_eq!(join(&edits, &changed), Err(Refused::Stale));
        // A line that reads the same but is no longer that task's.
        let fenced = Document::new("```\n- [ ] b");
        let second = [edits[1].clone()];
        assert_eq!(join(&second, &fenced), Err(Refused::Stale));
        let plan = join(&[], &doc).unwrap();
        assert!(plan.edits.is_empty());
    }

    #[test]
    fn two_edits_of_one_line_are_refused_not_dropped() {
        let (text, sel) = parse("- [ ] a|");
        let doc = Document::new(&text);
        let cycle = edit(&doc, 0, sel, &Change::Cycle, &tasks_on()).unwrap();
        let nothing = edit(&doc, 0, sel, &Change::SetPriority(None), &tasks_on()).unwrap();
        let toggle = edit(&doc, 0, sel, &Change::ToggleDone, &tasks_on()).unwrap();
        assert_eq!(join(&[cycle, toggle.clone()], &doc), Err(Refused::Invalid));
        // An edit that changes nothing still names its line.
        assert!(nothing.is_noop());
        assert_eq!(join(&[toggle, nothing], &doc), Err(Refused::Invalid));
    }

    /// Select all of a long note and toggle: finding the tasks, planning
    /// their edits and checking them read the note once, so a note of
    /// 50,000 lines (25,000 tasks, each followed by a line of its own text)
    /// takes well under a second even in a debug build.
    #[test]
    fn a_long_selection_is_read_once() {
        let text: String = (0..25_000)
            .map(|i| format!("- [ ] task {i} 📅 2026-12-01\n  its next line\n"))
            .collect();
        let doc = Document::new(&text);
        let count = doc.line_count();
        let sel = Selection::new(crate::Pos::new(0, 0), crate::Pos::new(count - 1, 0));
        let started = std::time::Instant::now();
        let edits = toggle(&doc, sel, &tasks_on()).unwrap().unwrap();
        let plan = join(&edits, &doc).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(edits.len(), 25_000);
        assert_eq!(plan.edits.len(), 25_000);
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "{elapsed:?} for {count} lines"
        );
    }
}
