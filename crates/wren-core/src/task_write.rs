//! Changes to a note's tasks made from outside an editor: one task line
//! rewritten, and the note's task switch (`tasks: off` in its front
//! matter) set or taken away. Each is planned here on the note's text as
//! it reads now, whoever holds that text: the Session plans on the bytes
//! of a note that is not open and writes them back
//! ([`crate::ops::Op::EditTaskLine`], [`crate::ops::Op::EditTaskSwitch`]),
//! and a frontend plans on its editor's text for the open note and applies
//! the plan there, so the editor stays that note's one writer.
//!
//! A plan carries what it read. A task line is found where it was, or at
//! the one place a task line of the same text is now ([`locate`]); a
//! change to the switch gives the front matter before and after, which is
//! what undoing it expects and writes back ([`restore_switch`]).

use std::ops::Range;

use wren_markdown::block;
use wren_markdown::date::Date;
use wren_markdown::task::{self, Change};
use wren_markdown::{Document, LineEnding, Lines, Options, Pos, Replace, Selection, TaskOptions};

use crate::tasks;

/// What a write is refused with once the task it was made for reads
/// differently: the line has changed, or has moved to where it cannot be
/// told from another.
pub const CHANGED: &str = "This task changed; the list was refreshed";

/// What a change to the task switch is refused with once the front matter
/// it was made for reads differently.
pub const SWITCH_CHANGED: &str = "This note's front matter changed; nothing was written";

const BOM: char = '\u{feff}';
const SWITCH_OFF: &str = "tasks: off";

/// What to write on a task line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineChange {
    /// A change planned on the line as it is found, with `today` for a
    /// done or cancelled stamp.
    Change { change: Change, today: Date },
    /// The line itself, without a line ending: what undoing a change
    /// writes back.
    Line(String),
}

/// What to do to a note's task switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchChange {
    /// Write `tasks: off` into the front matter ([`set_hidden`]).
    Hide,
    /// Take it out again.
    Show,
    /// Put the front matter back as it was before a change: what undoing
    /// one writes ([`restore_switch`]).
    Restore { expect: String, replace: String },
}

/// Why a change was not planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// What the change expects is not there any more: [`CHANGED`] for a
    /// task line, [`SWITCH_CHANGED`] for the switch.
    Changed,
    /// The change cannot be made, whatever is there.
    Invalid(String),
}

/// A task line found and rewritten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineWrite {
    /// The line the task is on now.
    pub line: usize,
    /// That line as it reads, without its ending.
    pub before: String,
    /// The line as the change writes it: `before` when nothing changes.
    pub after: String,
    /// The bytes of `before` in the text.
    pub range: Range<usize>,
}

impl LineWrite {
    /// Whether writing it changes anything.
    pub fn changes(&self) -> bool {
        self.before != self.after
    }

    /// `source`, the text this was planned on, with the line rewritten.
    pub fn apply(&self, source: &str) -> String {
        splice(source, &self.range, &self.after)
    }

    /// The line's change as an editor's replacements, each on the line and
    /// as small as the two texts allow, so a caret outside what changed
    /// keeps its place ([`LineWrite::moved`]).
    pub fn replaces(&self) -> Vec<Replace> {
        (hunks(&self.before, &self.after).into_iter())
            .map(|(range, text)| Replace {
                start: Pos::new(self.line, range.start),
                end: Pos::new(self.line, range.end),
                text,
            })
            .collect()
    }

    /// Where `pos` of the text before lands after
    /// [`replaces`](Self::replaces): on another line, where it was; on the
    /// line, moved with the text around it, to the end of the new text
    /// when what it was in was replaced, and left before text that goes in
    /// at it.
    pub fn moved(&self, pos: Pos) -> Pos {
        if pos.line != self.line {
            return pos;
        }
        let (mut added, mut removed) = (0, 0);
        for (range, text) in hunks(&self.before, &self.after) {
            if pos.col <= range.start {
                break;
            }
            if pos.col < range.end {
                return Pos::new(pos.line, range.start + added + text.len() - removed);
            }
            added += text.len();
            removed += range.len();
        }
        Pos::new(pos.line, pos.col + added - removed)
    }
}

/// `text` without a byte order mark before it.
fn unmarked(text: &str) -> &str {
    text.strip_prefix(BOM).unwrap_or(text)
}

fn splice(source: &str, range: &Range<usize>, text: &str) -> String {
    let mut out = String::with_capacity(source.len() + text.len());
    out.push_str(&source[..range.start]);
    out.push_str(text);
    out.push_str(&source[range.end..]);
    out
}

/// Where the task line that read `expect` on line `line` is in `source`
/// now: that line, while it still reads so and is still a task; else the
/// one task line of the same text. `None` when there is none, or more
/// than one, which cannot be told apart.
///
/// A task line is one the task index reads as a task
/// ([`tasks::task_lines`]): not a line of the same text in a fenced block
/// or in front matter. Lines are compared without their endings, and
/// without the byte order mark the first line of a file may begin with,
/// so a line that moved to or from the top is still found.
pub fn locate(source: &str, line: usize, expect: &str) -> Option<usize> {
    let document = Document::new(source);
    let expect = unmarked(expect);
    let tasks = tasks::task_lines(source);
    let reads = |at: usize| {
        let text = document.line(at);
        (if at == 0 { unmarked(text) } else { text }) == expect
    };
    if tasks.binary_search(&line).is_ok() && reads(line) {
        return Some(line);
    }
    let mut same = tasks.iter().copied().filter(|at| reads(*at));
    match (same.next(), same.next()) {
        (Some(at), None) => Some(at),
        _ => None,
    }
}

/// Plan `change` to the task line of `source` that read `expect` on line
/// `line`, found as [`locate`] finds it. A change that changes nothing is
/// a plan whose line stays as it is.
pub fn plan_line(
    source: &str,
    line: usize,
    expect: &str,
    change: &LineChange,
) -> Result<LineWrite, Refused> {
    let at = locate(source, line, expect).ok_or(Refused::Changed)?;
    let document = Document::new(source);
    let before = document.line(at);
    let after = match change {
        LineChange::Change { change, today } => {
            let options = Options {
                tasks: Some(TaskOptions { today: *today }),
                ..Options::default()
            };
            let caret = Selection::caret(Pos::new(at, 0));
            task::edit(&document, at, caret, change, &options)
                .filter(|edit| edit.line == at)
                .ok_or_else(|| Refused::Invalid("That change cannot be made to this task".into()))?
                .text
        }
        LineChange::Line(text) => {
            if text.contains(['\n', '\r']) {
                return Err(Refused::Invalid("A task is one line".into()));
            }
            let text = unmarked(text);
            if at == 0 && before.starts_with(BOM) {
                format!("{BOM}{text}")
            } else {
                text.to_owned()
            }
        }
    };
    let write = LineWrite {
        line: at,
        before: before.to_owned(),
        after,
        range: document.line_range(at),
    };
    // A line written whole is still a task where it is: what undoes the
    // write looks for it among the note's tasks.
    if matches!(change, LineChange::Line(_))
        && write.changes()
        && tasks::task_lines(&write.apply(source))
            .binary_search(&at)
            .is_err()
    {
        return Err(Refused::Invalid("That line is no task".into()));
    }
    Ok(write)
}

/// The most single changes two texts may differ by to be compared part
/// by part ([`changes`]).
const MAX_CHANGES: usize = 256;

/// The changes that turn `a` into `b`, in order and apart: each a range
/// of `a` and the range of `b` that replaces it, with what the two have
/// in common between them. It is a shortest such script (Myers' greedy
/// search), which costs by how much the two differ and not by how long
/// they are, so a long line with a box and a stamp changed keeps
/// everything between them. Two that differ by more than
/// [`MAX_CHANGES`] items are one change, between what they start and end
/// with in common.
fn changes<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(Range<usize>, Range<usize>)> {
    let prefix = a.iter().zip(b).take_while(|(a, b)| a == b).count();
    let (a, b) = (&a[prefix..], &b[prefix..]);
    let suffix = (a.iter().rev().zip(b.iter().rev()))
        .take_while(|(a, b)| a == b)
        .count();
    let (a, b) = (&a[..a.len() - suffix], &b[..b.len() - suffix]);
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    let found = shortest(a, b).unwrap_or_else(|| vec![(0..a.len(), 0..b.len())]);
    (found.into_iter())
        .map(|(old, new)| {
            (
                old.start + prefix..old.end + prefix,
                new.start + prefix..new.end + prefix,
            )
        })
        .collect()
}

/// [`changes`] of two sequences that differ at both ends, or `None` when
/// they differ by more than [`MAX_CHANGES`] items.
fn shortest<T: PartialEq>(a: &[T], b: &[T]) -> Option<Vec<(Range<usize>, Range<usize>)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = MAX_CHANGES.min(a.len() + b.len()) as isize;
    // `reach[at(k)]` is how far along `a` the search has got on diagonal
    // `k` (where `x - y == k`), and `rounds[d]` is `reach` before round `d`.
    let at = |k: isize| (k + max + 1) as usize;
    let down = |reach: &[isize], k: isize, d: isize| {
        k == -d || (k != d && reach[at(k - 1)] < reach[at(k + 1)])
    };
    let mut reach = vec![0isize; (2 * max + 3) as usize];
    let mut rounds: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    'search: for d in 0..=max {
        rounds.push(reach.clone());
        for k in (-d..=d).step_by(2) {
            let mut x = if down(&reach, k, d) {
                reach[at(k + 1)]
            } else {
                reach[at(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            reach[at(k)] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'search;
            }
        }
    }
    // Walk back from the end: each round added one item of `b` (a step
    // down) or took one of `a` away (a step right).
    let (mut x, mut y) = (n, m);
    let mut steps: Vec<(usize, usize, bool)> = Vec::new();
    for d in (1..=found?).rev() {
        let reach = &rounds[d as usize];
        let k = x - y;
        let added = down(reach, k, d);
        let from = if added { k + 1 } else { k - 1 };
        x = reach[at(from)];
        y = x - from;
        steps.push((x as usize, y as usize, added));
    }
    let mut found: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for (x, y, added) in steps.into_iter().rev() {
        let (taken, given) = (usize::from(!added), usize::from(added));
        match found.last_mut() {
            Some((old, new)) if old.end == x && new.end == y => {
                old.end += taken;
                new.end += given;
            }
            _ => found.push((x..x + taken, y..y + given)),
        }
    }
    Some(found)
}

/// The edits that turn the line `old` into `new`, in order and apart: the
/// text between what the two have in common, character by character.
fn hunks(old: &str, new: &str) -> Vec<(Range<usize>, String)> {
    let a: Vec<(usize, char)> = old.char_indices().collect();
    let b: Vec<(usize, char)> = new.char_indices().collect();
    let letters = |chars: &[(usize, char)]| -> Vec<char> { chars.iter().map(|c| c.1).collect() };
    let byte = |chars: &[(usize, char)], text: &str, at: usize| {
        chars.get(at).map_or(text.len(), |(byte, _)| *byte)
    };
    (changes(&letters(&a), &letters(&b)).into_iter())
        .map(|(gone, come)| {
            (
                byte(&a, old, gone.start)..byte(&a, old, gone.end),
                new[byte(&b, new, come.start)..byte(&b, new, come.end)].to_owned(),
            )
        })
        .collect()
}

/// A note's task switch found and changed: its front matter before and
/// after, and the changes to the text between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchWrite {
    /// The front matter as it reads, from its opening `---` to the end of
    /// its closing line, ending included; empty when the note has none. A
    /// byte order mark is before it, and no part of it.
    pub before: String,
    /// The front matter as the change leaves it; empty when the block is
    /// taken away.
    pub after: String,
    /// The changes, in order and apart: each the bytes of the text that
    /// go, whole lines, and what they become, with its line endings as
    /// they are written.
    pub edits: Vec<(Range<usize>, String)>,
}

impl SwitchWrite {
    /// `source`, the text this was planned on, with the change made.
    pub fn apply(&self, source: &str) -> String {
        let mut text = source.to_owned();
        for (range, with) in self.edits.iter().rev() {
            text.replace_range(range.clone(), with);
        }
        text
    }

    /// The change as an editor's replacements on `source`, the text this
    /// was planned on, made together as one change. Each one's text is to
    /// go in as it is, line endings and all: an editor that gave a line
    /// break the ending of the line it splits would not put a CRLF block
    /// back above an LF body as it was.
    pub fn replaces(&self, source: &str) -> Vec<Replace> {
        let document = Document::new(source);
        (self.edits.iter())
            .map(|(range, text)| Replace {
                start: position(&document, range.start),
                end: position(&document, range.end),
                text: text.clone(),
            })
            .collect()
    }

    /// Where `pos` of `source` lands after [`replaces`](Self::replaces):
    /// before a change, where it was; after one, on the same text, so a
    /// caret where lines go in moves down with its own; inside lines that
    /// are replaced, at the end of what replaces them. A position on a
    /// line between two changes keeps its place on that line.
    pub fn moved(&self, source: &str, pos: Pos) -> Pos {
        let document = Document::new(source);
        let line = pos.line.min(document.line_count() - 1);
        let within = document.line_range(line);
        let mut at = within.start + pos.col.min(within.len());
        let mut shift = 0isize;
        for (range, text) in &self.edits {
            if at >= range.end {
                shift += text.len() as isize - range.len() as isize;
            } else {
                if at > range.start {
                    at = range.start + text.len();
                }
                break;
            }
        }
        let written = self.apply(source);
        position(&Document::new(&written), at.saturating_add_signed(shift))
    }
}

/// The line and column of byte `at` of a document.
fn position(document: &Document<'_>, at: usize) -> Pos {
    let count = document.line_count();
    let line = (0..count)
        .rev()
        .find(|line| document.line_range(*line).start <= at)
        .unwrap_or(0);
    let within = document.line_range(line);
    Pos::new(line, at.min(within.end) - within.start)
}

/// The ending a line that goes in before line `line` takes: that line's
/// own, or, for a last line that has none, the one above's, else LF. It
/// is what an editor gives a line break put there.
fn ending_at(document: &Document<'_>, line: usize) -> &'static str {
    let own = |line: usize| {
        (line < document.line_count())
            .then(|| document.ending(line))
            .filter(|ending| *ending != LineEnding::None)
    };
    own(line)
        .or_else(|| line.checked_sub(1).and_then(own))
        .unwrap_or(LineEnding::Lf)
        .as_str()
}

/// Where the front matter of a document is: its closing line, and its
/// bytes after a byte order mark, through the closing line's ending. With
/// no front matter, no line and the empty place it would go.
fn front_matter(document: &Document<'_>) -> (Option<usize>, Range<usize>) {
    let source = document.text();
    let start = source.len() - unmarked(source).len();
    match block::front_matter_end(document) {
        Some(close) => {
            let end = if close + 1 < document.line_count() {
                document.line_range(close + 1).start
            } else {
                source.len()
            };
            (Some(close), start..end)
        }
        None => (None, start..start),
    }
}

/// The front matter of `source` as [`SwitchWrite::before`] gives it: from
/// its opening `---` through its closing line's ending, or nothing.
pub fn front_matter_of(source: &str) -> &str {
    let (_, place) = front_matter(&Document::new(source));
    &source[place]
}

/// Line `line` of a document with its ending, and without the byte
/// order mark before the first.
fn whole_line<'a>(document: &Document<'a>, line: usize) -> &'a str {
    let range = document.line_range(line);
    let end = range.end + document.ending(line).as_str().len();
    let text = &document.text()[range.start..end];
    if line == 0 {
        unmarked(text)
    } else {
        text
    }
}

/// The lines of `text`, each with its ending, as bytes of it.
fn whole_lines(text: &str) -> Vec<Range<usize>> {
    let document = Document::new(text);
    (0..document.line_count())
        .map(|line| {
            let range = document.line_range(line);
            range.start..range.end + document.ending(line).as_str().len()
        })
        .filter(|range| !range.is_empty())
        .collect()
}

/// The change from the front matter at `place` of `source` to `after`,
/// kept to the lines that differ: lines taken from two places are two
/// changes, and what is between them is left alone.
fn switch_write(source: &str, place: Range<usize>, after: String) -> SwitchWrite {
    let before = &source[place.clone()];
    let (old, new) = (whole_lines(before), whole_lines(&after));
    let texts = |text: &'_ str, lines: &[Range<usize>]| -> Vec<String> {
        lines
            .iter()
            .map(|line| text[line.clone()].to_owned())
            .collect()
    };
    let start = |lines: &[Range<usize>], text: &str, at: usize| {
        lines.get(at).map_or(text.len(), |line| line.start)
    };
    let edits = (changes(&texts(before, &old), &texts(&after, &new)).into_iter())
        .map(|(gone, come)| {
            let gone = start(&old, before, gone.start)..start(&old, before, gone.end);
            let come = start(&new, &after, come.start)..start(&new, &after, come.end);
            (
                place.start + gone.start..place.start + gone.end,
                after[come].to_owned(),
            )
        })
        .collect();
    SwitchWrite {
        edits,
        before: before.to_owned(),
        after,
    }
}

/// Whether a line of front matter names a top-level key `tasks`, however
/// it is written: bare, quoted, or with space before its colon.
fn names_tasks(line: &str) -> bool {
    let quoted = ['"', '\''].into_iter().find_map(|quote| {
        line.strip_prefix(quote)?
            .strip_prefix("tasks")?
            .strip_prefix(quote)
    });
    let Some(rest) = quoted.or_else(|| line.strip_prefix("tasks")) else {
        return false;
    };
    let rest = rest.trim_start_matches([' ', '\t']);
    rest.strip_prefix(':')
        .is_some_and(|value| value.is_empty() || value.starts_with([' ', '\t']))
}

/// Plan the note's task switch set: `hidden` writes `tasks: off` into the
/// front matter, and otherwise takes it out. `None` when the note already
/// reads that way ([`tasks::hides_tasks`]).
///
/// - **Hiding.** A note with no front matter gets one of that single
///   line, at the top, after a byte order mark; a leading `---` that is a
///   thematic break (no front matter follows it) stays below the new
///   block. Front matter that is there gets the line before its closing
///   line, or, when its `tasks` key says on, has that line rewritten.
///   Refused when a `tasks` key is there that is no switch Wren reads (a
///   list of another tool's, a quoted key): it is not Wren's to rewrite,
///   and a second key beside it would be one too many. New lines end as
///   the line they go in before does.
/// - **Showing.** Every `tasks` line that hides goes. A block left with
///   nothing but blank lines goes with it, whoever wrote it; one that
///   holds anything else stays. Refused when the note would still be
///   hidden, by Obsidian's `TP_ignore_this_file`, which is not Wren's to
///   remove.
///
/// Either way the front matter must end where the change leaves it:
/// refused when what is left would no longer read as front matter, and
/// when taking the block away would make front matter of the lines under
/// it, which are the note's text.
pub fn set_hidden(source: &str, hidden: bool) -> Result<Option<SwitchWrite>, Refused> {
    if tasks::hides_tasks(source) == hidden {
        return Ok(None);
    }
    let document = Document::new(source);
    let (close, place) = front_matter(&document);
    // The closing line once the change is made.
    let left;
    let after = match (hidden, close) {
        (true, None) => {
            let ending = ending_at(&document, 0);
            left = Some(2);
            format!("---{ending}{SWITCH_OFF}{ending}---{ending}")
        }
        (true, Some(close)) => {
            let mut own = None;
            for line in (1..close).filter(|line| names_tasks(document.line(*line))) {
                let reads = match tasks::key_value(document.line(line)) {
                    Some(("tasks", value)) => tasks::switch(value).is_some(),
                    _ => false,
                };
                // A list, a number, a quoted key, some other tool's
                // `tasks`: rewriting it would lose what it holds, and a
                // second key below it would make the front matter one a
                // YAML reader refuses.
                if !reads {
                    return Err(Refused::Invalid(
                        "This note's front matter has a `tasks` key Wren does not read; \
                         add `tasks: off` in the note"
                            .into(),
                    ));
                }
                own = Some(line);
            }
            left = Some(if own.is_some() { close } else { close + 1 });
            let mut after = String::new();
            for line in 0..=close {
                match own {
                    Some(own) if own == line => {
                        after.push_str(SWITCH_OFF);
                        after.push_str(document.ending(line).as_str());
                        continue;
                    }
                    None if line == close => {
                        after.push_str(SWITCH_OFF);
                        after.push_str(ending_at(&document, close));
                    }
                    _ => {}
                }
                after.push_str(whole_line(&document, line));
            }
            after
        }
        (false, Some(close)) => {
            let hides = |line: usize| {
                matches!(
                    tasks::key_value(document.line(line)),
                    Some(("tasks", value)) if tasks::switch(value) == Some(false)
                )
            };
            let kept: Vec<usize> = (1..close).filter(|line| !hides(*line)).collect();
            let blank = |line: &usize| document.line(*line).trim().is_empty();
            if kept.iter().all(blank) {
                left = None;
                String::new()
            } else {
                left = Some(kept.len() + 1);
                (std::iter::once(0).chain(kept).chain([close]))
                    .map(|line| whole_line(&document, line))
                    .collect()
            }
        }
        // A note with no front matter hides nothing.
        (false, None) => return Ok(None),
    };
    let write = switch_write(source, place, after);
    let written = write.apply(source);
    if tasks::hides_tasks(&written) != hidden {
        return Err(Refused::Invalid(if hidden {
            "This note's front matter could not take `tasks: off`; add it in the note".into()
        } else {
            "`TP_ignore_this_file` hides this note's tasks; remove it in the note to show them"
                .into()
        }));
    }
    if block::front_matter_end(&Document::new(&written)) != left {
        return Err(Refused::Invalid(
            if left.is_some() {
                "With that change the note's front matter would not read as it does; \
                 edit the note"
            } else {
                "Without its front matter the note's next lines would read as front matter; \
                 edit the note"
            }
            .into(),
        ));
    }
    Ok(Some(write))
}

/// Plan the note's front matter put back: `replace` written where the
/// front matter reads `expect` (each as [`SwitchWrite::before`] and
/// [`SwitchWrite::after`] give them), which undoes the change that left
/// it so. [`Refused::Changed`] once it reads otherwise, and when the
/// note's text has changed so that what is put back would not be its
/// front matter: a block with no ending after its closing line, above
/// text that would run on from it, or a block taken away from above
/// lines that would then read as one.
pub fn restore_switch(source: &str, expect: &str, replace: &str) -> Result<SwitchWrite, Refused> {
    let (_, place) = front_matter(&Document::new(source));
    if &source[place.clone()] != expect {
        return Err(Refused::Changed);
    }
    let write = switch_write(source, place, replace.to_owned());
    let close = whole_lines(replace).len().checked_sub(1);
    if block::front_matter_end(&Document::new(&write.apply(source))) != close {
        return Err(Refused::Changed);
    }
    Ok(write)
}

/// Plan `change` to the task switch of `source`. `None` when it changes
/// nothing.
pub fn plan_switch(source: &str, change: &SwitchChange) -> Result<Option<SwitchWrite>, Refused> {
    let write = match change {
        SwitchChange::Hide => set_hidden(source, true)?,
        SwitchChange::Show => set_hidden(source, false)?,
        SwitchChange::Restore { expect, replace } => Some(restore_switch(source, expect, replace)?),
    };
    Ok(write.filter(|write| write.before != write.after))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday 19 November 2026, the design page's day.
    fn today() -> Date {
        Date::new(2026, 11, 19).unwrap()
    }

    fn change(change: Change) -> LineChange {
        LineChange::Change {
            change,
            today: today(),
        }
    }

    /// `source` after `change` to the task that read `expect` on `line`.
    fn written(source: &str, line: usize, expect: &str, with: &LineChange) -> String {
        plan_line(source, line, expect, with).unwrap().apply(source)
    }

    #[test]
    fn a_line_is_found_where_it_was_or_at_its_one_new_place() {
        let source = "# A\n- [ ] one\n- [ ] two\n";
        assert_eq!(locate(source, 1, "- [ ] one"), Some(1));
        // Moved: lines went in above it.
        assert_eq!(locate(source, 0, "- [ ] two"), Some(2));
        assert_eq!(locate(source, 9, "- [ ] two"), Some(2));
        // Changed, or gone.
        assert_eq!(locate(source, 1, "- [ ] won"), None);
        assert_eq!(locate("", 0, "- [ ] one"), None);
    }

    #[test]
    fn duplicate_lines_are_told_apart_only_by_their_place() {
        let source = "- [ ] same\ntext\n- [ ] same\n";
        assert_eq!(locate(source, 0, "- [ ] same"), Some(0));
        assert_eq!(locate(source, 2, "- [ ] same"), Some(2));
        // Neither is where the row said: it cannot be told which was meant.
        assert_eq!(locate(source, 1, "- [ ] same"), None);
        assert_eq!(
            plan_line(source, 5, "- [ ] same", &change(Change::Cycle)),
            Err(Refused::Changed)
        );
    }

    #[test]
    fn a_twin_in_a_fence_or_in_front_matter_is_no_task() {
        // The line the row named is in a fence now; its twin below is the
        // one task of that text.
        let fenced = "```\n- [ ] same\n```\n- [ ] same\n";
        assert_eq!(locate(fenced, 1, "- [ ] same"), Some(3));
        assert_eq!(
            written(fenced, 1, "- [ ] same", &change(Change::Cycle)),
            "```\n- [ ] same\n```\n- [/] same\n"
        );
        let front = "---\nkey: v\n- [ ] same\n---\n- [ ] same\n";
        assert_eq!(locate(front, 2, "- [ ] same"), Some(4));
        // With no twin outside, the line is no task at all.
        assert_eq!(locate("```\n- [ ] same\n```\n", 1, "- [ ] same"), None);
    }

    #[test]
    fn a_change_is_planned_on_the_line_as_found_and_keeps_every_ending() {
        let source = "# A\r\n- [ ] one 📅 2026-12-01\r\n  - [ ] nested\r\nlast";
        assert_eq!(
            written(
                source,
                1,
                "- [ ] one 📅 2026-12-01",
                &change(Change::ToggleDone)
            ),
            "# A\r\n- [x] one 📅 2026-12-01 ✅ 2026-11-19\r\n  - [ ] nested\r\nlast"
        );
        // A nested task, which alone would read as indented code.
        assert_eq!(
            written(source, 2, "  - [ ] nested", &change(Change::Cycle)),
            "# A\r\n- [ ] one 📅 2026-12-01\r\n  - [/] nested\r\nlast"
        );
        // A change that changes nothing plans the line as it is.
        let kept = plan_line(source, 2, "  - [ ] nested", &change(Change::LowerPriority)).unwrap();
        assert!(kept.changes());
        let kept = plan_line(source, 2, "  - [ ] nested", &change(Change::SetDue(None))).unwrap();
        assert!(!kept.changes());
        assert_eq!(kept.apply(source), source);
        // One the task refuses.
        assert!(matches!(
            plan_line(source, 2, "  - [ ] nested", &change(Change::SetStatus(']'))),
            Err(Refused::Invalid(_))
        ));
    }

    #[test]
    fn the_byte_order_mark_stays_at_the_top_whichever_line_is_written() {
        let marked = "\u{feff}- [ ] top\n- [ ] below\n";
        // The index gives the first line with its mark.
        let plan = plan_line(marked, 0, "\u{feff}- [ ] top", &change(Change::Cycle)).unwrap();
        assert_eq!(plan.before, "\u{feff}- [ ] top");
        assert_eq!(plan.after, "\u{feff}- [/] top");
        assert_eq!(plan.apply(marked), "\u{feff}- [/] top\n- [ ] below\n");
        // Undo writes the line back, with or without the mark it had.
        for old in ["\u{feff}- [ ] top", "- [ ] top"] {
            let cycled = plan.apply(marked);
            let undo = LineChange::Line(old.into());
            assert_eq!(written(&cycled, 0, &plan.after, &undo), marked);
        }
        // A line that was at the top and has moved down carries no mark.
        let moved = "\u{feff}# New\n- [ ] top\n";
        assert_eq!(locate(moved, 0, "\u{feff}- [ ] top"), Some(1));
        assert_eq!(
            written(
                moved,
                0,
                "\u{feff}- [ ] top",
                &LineChange::Line("\u{feff}- [x] top".into())
            ),
            "\u{feff}# New\n- [x] top\n"
        );
        // One that moved up to the top gains it.
        assert_eq!(
            written(
                marked,
                1,
                "- [ ] top",
                &LineChange::Line("- [x] top".into())
            ),
            "\u{feff}- [x] top\n- [ ] below\n"
        );
    }

    #[test]
    fn a_line_written_whole_is_one_line_and_still_a_task() {
        let source = "- [ ] a\n";
        for text in [
            "- [x] a\n",
            "- [x] a\r- [ ] b",
            // No task: what undoes the write could not find it again.
            "plain text",
            "- a",
            "    - [ ] a",
            "```",
        ] {
            assert!(
                matches!(
                    plan_line(source, 0, "- [ ] a", &LineChange::Line(text.into())),
                    Err(Refused::Invalid(_))
                ),
                "{text:?}"
            );
        }
        // The line as it is is nothing to write, whatever it is.
        let same = LineChange::Line("- [ ] a".into());
        assert!(!plan_line(source, 0, "- [ ] a", &same).unwrap().changes());
        let other = LineChange::Line("* [/] b @sam".into());
        assert_eq!(written(source, 0, "- [ ] a", &other), "* [/] b @sam\n");
    }

    /// `old` with `edits` made, last first.
    fn patched(old: &str, edits: &[(Range<usize>, String)]) -> String {
        let mut text = old.to_owned();
        for (range, with) in edits.iter().rev() {
            text.replace_range(range.clone(), with);
        }
        text
    }

    #[test]
    fn hunks_turn_one_line_into_the_other_in_small_edits() {
        let cases = [
            ("- [ ] Mail the deck", "- [x] Mail the deck ✅ 2026-11-19"),
            ("- [x] Mail the deck ✅ 2026-11-19", "- [ ] Mail the deck"),
            ("- [ ] a 📅 2026-12-01", "- [ ] a ⏫ 📅 2026-12-04"),
            ("- [ ] é", "- [ ] è @sam"),
            ("", "- [ ] new"),
            ("- [ ] gone", ""),
            ("same", "same"),
            ("aXbYc", "abc"),
        ];
        for (old, new) in cases {
            let edits = hunks(old, new);
            assert_eq!(patched(old, &edits), new, "{old:?} to {new:?}");
            for pair in edits.windows(2) {
                assert!(pair[0].0.end < pair[1].0.start, "{edits:?}");
            }
            for (range, _) in &edits {
                assert!(old.is_char_boundary(range.start) && old.is_char_boundary(range.end));
            }
        }
        // The box and the stamp are two edits: the description between
        // them is untouched.
        let edits = hunks("- [ ] Mail the deck", "- [x] Mail the deck ✅ 2026-11-19");
        assert_eq!(
            edits,
            [
                (3..4, "x".to_owned()),
                (19..19, " ✅ 2026-11-19".to_owned())
            ]
        );
        // However long the line: the cost is in what differs.
        let long = "word ".repeat(400);
        let (old, new) = (format!("- [ ] {long}a"), format!("- [x] {long}b"));
        let edits = hunks(&old, &new);
        assert_eq!(
            edits,
            [
                (3..4, "x".to_owned()),
                (old.len() - 1..old.len(), "b".to_owned())
            ]
        );
        // Two lines with next to nothing in common are one edit.
        let (old, new) = ("ab".repeat(300), "cd".repeat(300));
        let edits = hunks(&old, &new);
        assert_eq!(edits, [(0..old.len(), new.clone())]);
    }

    #[test]
    fn a_caret_in_a_long_description_keeps_its_place_when_the_change_is_undone() {
        let words = "word ".repeat(120);
        let done = format!("- [x] {words}✅ 2026-11-19");
        let open = format!("- [ ] {words}");
        let undo = LineChange::Line(open.trim_end().to_owned());
        let plan = plan_line(&done, 0, &done, &undo).unwrap();
        assert_eq!(plan.replaces().len(), 2, "the box, and the stamp");
        for col in [6, 300, 600] {
            assert_eq!(plan.moved(Pos::new(0, col)), Pos::new(0, col));
        }
        assert_eq!(plan.after, open.trim_end());
    }

    #[test]
    fn changes_are_a_shortest_script_and_one_change_past_the_limit() {
        let run = |a: &str, b: &str| {
            let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
            changes(&a, &b)
        };
        assert_eq!(run("", ""), []);
        assert_eq!(run("abc", "abc"), []);
        assert_eq!(run("abc", ""), [(0..3, 0..0)]);
        assert_eq!(run("", "abc"), [(0..0, 0..3)]);
        assert_eq!(run("abcd", "aXcY"), [(1..2, 1..2), (3..4, 3..4)]);
        assert_eq!(run("a-b-c", "abc"), [(1..2, 1..1), (3..4, 2..2)]);
        assert_eq!(run("abc", "a-b-c"), [(1..1, 1..2), (2..2, 3..4)]);
        // Each result rebuilds the second from the first.
        let texts = [
            "",
            "a",
            "abcabba",
            "cbabac",
            "xaxbxcx",
            "the quick brown fox",
        ];
        for a in texts {
            for b in texts {
                let (old, new): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
                let mut built: Vec<char> = Vec::new();
                let mut from = 0;
                for (gone, come) in changes(&old, &new) {
                    assert!(from <= gone.start, "{a:?} to {b:?}");
                    built.extend(&old[from..gone.start]);
                    built.extend(&new[come]);
                    from = gone.end;
                }
                built.extend(&old[from..]);
                assert_eq!(built, new, "{a:?} to {b:?}");
            }
        }
    }

    #[test]
    fn a_position_on_the_line_moves_with_its_text() {
        let source = "# A\n- [x] Mail the deck ✅ 2026-11-19\n";
        let undo = LineChange::Line("- [ ] Mail the deck".into());
        let plan = plan_line(source, 1, "- [x] Mail the deck ✅ 2026-11-19", &undo).unwrap();
        assert_eq!(plan.replaces().len(), 2);
        // Another line: where it was.
        assert_eq!(plan.moved(Pos::new(0, 2)), Pos::new(0, 2));
        // In the description, after the box whose symbol changed.
        assert_eq!(plan.moved(Pos::new(1, 10)), Pos::new(1, 10));
        // In the stamp that went: the end of the line.
        assert_eq!(plan.moved(Pos::new(1, 25)), Pos::new(1, 19));
        assert_eq!(plan.moved(Pos::new(1, plan.before.len())), Pos::new(1, 19));
        // The other way, text goes in at the end: a caret there stays
        // before it, one in the description keeps its place.
        let redo = LineChange::Line("- [x] Mail the deck ✅ 2026-11-19".into());
        let plan = plan_line("- [ ] Mail the deck", 0, "- [ ] Mail the deck", &redo).unwrap();
        assert_eq!(plan.moved(Pos::new(0, 19)), Pos::new(0, 19));
        assert_eq!(plan.moved(Pos::new(0, 10)), Pos::new(0, 10));
    }

    /// `source` hidden or shown, with the plan.
    fn switched(source: &str, hidden: bool) -> (String, SwitchWrite) {
        let write = set_hidden(source, hidden)
            .unwrap_or_else(|refused| panic!("{refused:?} for {source:?}"))
            .unwrap_or_else(|| panic!("no change to {source:?}"));
        (write.apply(source), write)
    }

    #[test]
    fn hiding_writes_the_switch_and_showing_takes_it_out_again() {
        let cases = [
            // No front matter: a block of the one line.
            ("# A\n- [ ] a\n", "---\ntasks: off\n---\n# A\n- [ ] a\n"),
            ("", "---\ntasks: off\n---\n"),
            ("one line", "---\ntasks: off\n---\none line"),
            // CRLF, and after a byte order mark.
            (
                "# A\r\n- [ ] a\r\n",
                "---\r\ntasks: off\r\n---\r\n# A\r\n- [ ] a\r\n",
            ),
            ("\u{feff}# A\n", "\u{feff}---\ntasks: off\n---\n# A\n"),
            // A leading `---` that is a thematic break.
            ("---\n# A\n", "---\ntasks: off\n---\n---\n# A\n"),
            (
                "---\nkey: unclosed\n",
                "---\ntasks: off\n---\n---\nkey: unclosed\n",
            ),
            // Front matter that is there keeps its other lines.
            (
                "---\ntitle: A\n---\nbody\n",
                "---\ntitle: A\ntasks: off\n---\nbody\n",
            ),
            (
                "---\r\ntitle: A\r\n...\r\n",
                "---\r\ntitle: A\r\ntasks: off\r\n...\r\n",
            ),
            ("---\ntitle: A\n---", "---\ntitle: A\ntasks: off\n---"),
            (
                "\u{feff}---\ntitle: |\n  two\n  lines\n---\n",
                "\u{feff}---\ntitle: |\n  two\n  lines\ntasks: off\n---\n",
            ),
        ];
        for (shown, hidden) in cases {
            assert!(!tasks::hides_tasks(shown), "{shown:?}");
            let (text, write) = switched(shown, true);
            assert_eq!(text, hidden, "hiding {shown:?}");
            assert!(tasks::hides_tasks(&text));
            assert_eq!(set_hidden(&text, true), Ok(None));
            // Showing it again gives the note back byte for byte, and so
            // does putting the front matter back.
            assert_eq!(switched(&text, false).0, shown, "showing {hidden:?}");
            let back = restore_switch(&text, &write.after, &write.before).unwrap();
            assert_eq!(back.apply(&text), shown);
            assert_eq!(set_hidden(shown, false), Ok(None));
        }
    }

    #[test]
    fn a_tasks_key_that_does_not_hide_is_rewritten_and_undo_puts_it_back() {
        let shown = "---\ntasks: on # for now\ntitle: A\n---\nbody\n";
        let (hidden, write) = switched(shown, true);
        assert_eq!(hidden, "---\ntasks: off\ntitle: A\n---\nbody\n");
        assert_eq!(write.edits, [(4..24, "tasks: off\n".to_owned())]);
        let back = restore_switch(&hidden, &write.after, &write.before).unwrap();
        assert_eq!(back.apply(&hidden), shown);
        // The last of several decides, so the last is the one rewritten.
        let twice = "---\ntasks: off\ntasks: yes\n---\n";
        assert_eq!(
            switched(twice, true).0,
            "---\ntasks: off\ntasks: off\n---\n"
        );
        // A `tasks` key that is no switch is some other tool's: nothing
        // of it is rewritten, and no second key is put beside it.
        for other in [
            "---\ntasks:\n  - a\n  - b\n---\n",
            "---\ntasks: [a, b]\n---\n",
            "---\ntasks: 3\ntitle: A\n---\n",
            "---\ntasks: off\ntasks: maybe\n---\n",
        ] {
            let refused = set_hidden(other, true).unwrap_err();
            assert!(
                matches!(&refused, Refused::Invalid(why) if why.contains("does not read")),
                "{other:?}: {refused:?}"
            );
        }
    }

    #[test]
    fn showing_takes_every_line_that_hides_and_an_empty_block_with_them() {
        // The block holds something else: it stays.
        let (shown, write) = switched("---\ntasks: off\ntitle: A\ntasks: no\n---\nbody\n", false);
        assert_eq!(shown, "---\ntitle: A\n---\nbody\n");
        // Undo puts each line back where it was.
        let back = restore_switch(&shown, &write.after, &write.before).unwrap();
        assert_eq!(
            back.apply(&shown),
            "---\ntasks: off\ntitle: A\ntasks: no\n---\nbody\n"
        );
        // A key that does not hide is not Wren's line to take.
        assert_eq!(
            switched("---\ntasks: on\ntasks: off\n---\n", false).0,
            "---\ntasks: on\n---\n"
        );
        // Nothing but blank lines is left: the block goes, whoever wrote it.
        assert_eq!(
            switched("---\n\ntasks: off\n  \n---\nbody\n", false).0,
            "body\n"
        );
        assert_eq!(
            switched("\u{feff}---\r\ntasks: false\r\n---", false).0,
            "\u{feff}"
        );
        // The mark stays where it is, before a block that stays too.
        assert_eq!(
            switched("\u{feff}---\ntasks: off\ntitle: A\n---\n", false).0,
            "\u{feff}---\ntitle: A\n---\n"
        );
    }

    #[test]
    fn showing_is_refused_where_wrens_line_is_not_what_hides() {
        for source in [
            "---\nTP_ignore_this_file: true\n---\n- [ ] a\n",
            "---\ntasks: off\nTP_ignore_this_file: true\n---\n",
        ] {
            let refused = set_hidden(source, false).unwrap_err();
            assert!(
                matches!(&refused, Refused::Invalid(why) if why.contains("TP_ignore_this_file")),
                "{refused:?}"
            );
            // Hidden already, by whichever key.
            assert_eq!(set_hidden(source, true), Ok(None));
        }
        // What is left would stop being front matter, and become a rule
        // and a heading.
        let odd = "---\ntasks: off\n- item\n---\n";
        assert!(matches!(set_hidden(odd, false), Err(Refused::Invalid(_))));
    }

    #[test]
    fn putting_front_matter_back_expects_what_the_change_left() {
        let shown = "# A\n";
        let (hidden, write) = switched(shown, true);
        // Edited since: another key went in.
        let edited = "---\ntasks: off\ntitle: A\n---\n# A\n";
        assert_eq!(
            restore_switch(edited, &write.after, &write.before),
            Err(Refused::Changed)
        );
        // The body may change freely.
        let longer = format!("{hidden}more\n");
        let back = restore_switch(&longer, &write.after, &write.before).unwrap();
        assert_eq!(back.apply(&longer), "# A\nmore\n");
        // Undoing a removal expects no front matter where it goes.
        let (removed, write) = switched(&hidden, false);
        assert_eq!(removed, shown);
        let again = restore_switch(&removed, &write.after, &write.before).unwrap();
        assert_eq!(again.apply(&removed), hidden);
        assert_eq!(
            restore_switch(edited, &write.after, &write.before),
            Err(Refused::Changed)
        );
    }

    #[test]
    fn a_switch_change_is_an_editors_replacements_of_whole_lines() {
        let source = "\u{feff}# A\r\nbody\r\n";
        let (_, write) = switched(source, true);
        let [replace] = &write.replaces(source)[..] else {
            panic!("one replacement");
        };
        // After the mark, which is the first three bytes of line 0.
        assert_eq!(
            (replace.start, replace.end),
            (Pos::new(0, 3), Pos::new(0, 3))
        );
        // With its line endings as they are written.
        assert_eq!(replace.text, "---\r\ntasks: off\r\n---\r\n");
        // A caret where the block goes in moves down with its line, and
        // one before it (in the mark) stays.
        assert_eq!(write.moved(source, Pos::new(0, 3)), Pos::new(3, 0));
        assert_eq!(write.moved(source, Pos::new(0, 5)), Pos::new(3, 2));
        assert_eq!(write.moved(source, Pos::new(1, 4)), Pos::new(4, 4));
        assert_eq!(write.moved(source, Pos::new(0, 0)), Pos::new(0, 0));

        // A line taken out of the middle: positions below move up, one in
        // the line goes to where it was.
        let source = "---\ntitle: A\ntasks: off\n---\nbody";
        let (_, write) = switched(source, false);
        let [replace] = &write.replaces(source)[..] else {
            panic!("one replacement");
        };
        assert_eq!(
            (replace.start, replace.end),
            (Pos::new(2, 0), Pos::new(3, 0))
        );
        assert_eq!(replace.text, "");
        assert_eq!(write.moved(source, Pos::new(1, 3)), Pos::new(1, 3));
        assert_eq!(write.moved(source, Pos::new(2, 5)), Pos::new(2, 0));
        assert_eq!(write.moved(source, Pos::new(4, 2)), Pos::new(3, 2));

        // A block that is the whole text, with no ending after it.
        let source = "---\ntasks: off\n---";
        let (shown, write) = switched(source, false);
        assert_eq!(shown, "");
        let [replace] = &write.replaces(source)[..] else {
            panic!("one replacement");
        };
        assert_eq!(
            (replace.start, replace.end),
            (Pos::new(0, 0), Pos::new(2, 3))
        );
        assert_eq!(write.moved(source, Pos::new(2, 3)), Pos::new(0, 0));
    }

    #[test]
    fn lines_taken_from_two_places_are_two_replacements() {
        let source = "---\ntasks: off\ntitle: keep\ntasks: off\n---\nbody\n";
        let (shown, write) = switched(source, false);
        assert_eq!(shown, "---\ntitle: keep\n---\nbody\n");
        let replaces = write.replaces(source);
        let places: Vec<(Pos, Pos)> = (replaces.iter())
            .map(|replace| (replace.start, replace.end))
            .collect();
        assert_eq!(
            places,
            [
                (Pos::new(1, 0), Pos::new(2, 0)),
                (Pos::new(3, 0), Pos::new(4, 0))
            ]
        );
        // The line between them is not touched: a selection in it keeps
        // both its ends.
        assert_eq!(write.moved(source, Pos::new(2, 7)), Pos::new(1, 7));
        assert_eq!(write.moved(source, Pos::new(2, 11)), Pos::new(1, 11));
        assert_eq!(write.moved(source, Pos::new(5, 2)), Pos::new(3, 2));
        // Undo puts both back, each where it was.
        let back = restore_switch(&shown, &write.after, &write.before).unwrap();
        assert_eq!(back.edits.len(), 2);
        assert_eq!(back.apply(&shown), source);
        assert_eq!(back.moved(&shown, Pos::new(1, 7)), Pos::new(2, 7));
    }

    #[test]
    fn a_block_put_back_keeps_its_own_line_endings() {
        // A CRLF block over an LF body.
        let source = "---\r\ntasks: off\r\n---\r\nbody\n";
        let (shown, write) = switched(source, false);
        assert_eq!(shown, "body\n");
        let back = restore_switch(&shown, &write.after, &write.before).unwrap();
        assert_eq!(back.apply(&shown), source);
        let [replace] = &back.replaces(&shown)[..] else {
            panic!("one replacement");
        };
        assert_eq!(replace.text, "---\r\ntasks: off\r\n---\r\n");
    }

    #[test]
    fn showing_is_refused_when_the_lines_under_the_block_would_become_front_matter() {
        let source = "---\ntasks: off\n---\n---\ntitle: keep\n---\nbody\n";
        let refused = set_hidden(source, false).unwrap_err();
        assert!(
            matches!(&refused, Refused::Invalid(why) if why.contains("next lines")),
            "{refused:?}"
        );
        // A blank line between them keeps the second a rule and a heading.
        let apart = "---\ntasks: off\n---\n\n---\ntitle: keep\n---\nbody\n";
        assert_eq!(switched(apart, false).0, "\n---\ntitle: keep\n---\nbody\n");
    }

    #[test]
    fn front_matter_is_put_back_only_where_it_would_read_as_front_matter() {
        // The block was the whole note, with no ending after its closing
        // line. Text written since would run on from that line.
        let source = "---\ntasks: off\n---";
        let (shown, write) = switched(source, false);
        assert_eq!(shown, "");
        let back = |text: &str| restore_switch(text, &write.after, &write.before);
        assert_eq!(back("").unwrap().apply(""), source);
        assert_eq!(back("body\n"), Err(Refused::Changed));
        // Below a line break of its own the body is the body.
        assert_eq!(
            back("\nbody\n").unwrap().apply("\nbody\n"),
            "---\ntasks: off\n---\nbody\n"
        );

        // A block taken away again from above lines that have since
        // become what front matter looks like.
        let (hidden, write) = switched("# A\n", true);
        let edited = hidden.replace("# A\n", "---\ntitle: A\n---\n");
        assert_eq!(
            restore_switch(&edited, &write.after, &write.before),
            Err(Refused::Changed)
        );
    }

    #[test]
    fn a_tasks_key_written_another_way_is_not_wrens_to_double() {
        for other in [
            "---\ntitle: A\n\"tasks\": [a, b]\n---\n",
            "---\ntitle: A\n'tasks': on\n---\n",
            "---\ntitle: A\ntasks : on\n---\n",
            "---\ntitle: A\n\"tasks\":\n  - a\n---\n",
            // Read, but with one that is not beside it.
            "---\ntasks: on\n\"tasks\": 1\n---\n",
        ] {
            assert!(!tasks::hides_tasks(other), "{other:?}");
            let refused = set_hidden(other, true).unwrap_err();
            assert!(
                matches!(&refused, Refused::Invalid(why) if why.contains("does not read")),
                "{other:?}: {refused:?}"
            );
        }
        // A key that only begins like it, or is nested, is another key.
        for fine in [
            "---\ntasks_done: 3\n---\n",
            "---\nmeta:\n  tasks: [a]\n---\n",
            "---\ntitle: \"tasks\": x\n---\n",
        ] {
            assert!(
                switched(fine, true).0.contains("\ntasks: off\n---\n"),
                "{fine:?}"
            );
        }
    }
}
