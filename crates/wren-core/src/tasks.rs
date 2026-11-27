//! The task index: one small record for each task line of the workspace's
//! Markdown notes, kept current at every point the search index takes a
//! note's source (`crate::index`), and never the note's content. The
//! search index drops content past its budget; the records stay.
//!
//! - A task is a line whose box the whole note reads as one
//!   ([`wren_markdown::checkpoints::opened_task`]) and
//!   [`wren_markdown::task::parse`] reads: not a line in a fenced block, in
//!   front matter or in an HTML block, which is what a task command in the
//!   editor accepts. Boards have no tasks.
//! - A note's front matter hides its tasks with `tasks: off`, or with
//!   Obsidian's `TP_ignore_this_file: true` ([`hides_tasks`]). A folder is
//!   excluded by `.wren/tasks.toml` (`crate::tasks_file`). A hidden or an
//!   excluded note gives no record to a query or a count; its records are
//!   still read, so the owners suggestions offer stay those of every task
//!   line, and showing the note again reads nothing.
//! - Queries take a path: the workspace (`None`), a section, a group or a
//!   note. Records come in the tree's order (sections, their groups, each
//!   group's notes, as the model lists them), then by line.
//! - [`TaskIndex::revision`] moves only when what a query answers changes,
//!   so a view built from one revision is current while it stands.
//!
//! The open note is read from its draft, from the first line an edit
//! changed, so typing at the end of a long note reads a few dozen lines
//! and touches only the records from that line on. What is kept between
//! edits is where the lines and headings lay and the scanner states
//! ([`wren_markdown::checkpoints`]), never the text: the search index,
//! which holds the text before the edit, says how far the two agree. Any
//! other note is read whole, and only when it may hold a box at all.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hasher;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use wren_markdown::block::{self, Boxes, Kind};
use wren_markdown::checkpoints::{self, Checkpoints};
use wren_markdown::date::{Date, Parsed};
use wren_markdown::status::StatusType;
use wren_markdown::task::{self, DateField, Priority};
use wren_markdown::{outline, Lines};

use crate::fs_store::RelPath;
use crate::kind::NoteKind;
use crate::model::{GroupId, Note, NoteId, SectionId, Workspace};

/// One task line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    pub note: NoteId,
    /// The note's path in the workspace.
    pub path: RelPath,
    /// The line's number from 0, as the editor numbers lines.
    pub line: usize,
    /// The line exactly as written, without its ending: what a change to
    /// the task expects to find there.
    pub source: String,
    /// Where the box's `[` is in [`TaskRecord::source`]: the task as the
    /// note reads it, which the line read alone may not say (a task nested
    /// in a quote or a list reads as indented code by itself).
    pub box_at: usize,
    /// The character in the box.
    pub symbol: char,
    pub status: StatusType,
    /// The due date; [`Parsed::Invalid`] for one in a date's shape that is
    /// no real day, which is kept and shown as a problem.
    pub due: Option<Parsed>,
    /// Where the due date's ten bytes are in [`TaskRecord::source`]: what
    /// a date that is no real day is shown as.
    pub due_at: Option<std::ops::Range<usize>>,
    pub priority: Option<Priority>,
    /// The owners' names as written, without the `@`, in line order.
    pub owners: Vec<String>,
    /// The `✅` stamp.
    pub done: Option<Parsed>,
    /// The `❌` stamp.
    pub cancelled: Option<Parsed>,
    /// The description, as [`wren_markdown::task::TaskLine::description_text`].
    pub description: String,
    /// The plain text of the nearest heading above the line; `None` at the
    /// top of the note. One allocation is shared by every task under the
    /// heading, whose title has no length limit.
    pub heading: Option<Arc<str>>,
}

impl TaskRecord {
    /// Work still to do: an open or a doing task.
    pub fn is_unfinished(&self) -> bool {
        self.status.is_unfinished()
    }

    /// The due date, when it is a real day.
    pub fn due_date(&self) -> Option<Date> {
        match self.due {
            Some(Parsed::Date(date)) => Some(date),
            _ => None,
        }
    }

    /// Unfinished and due before `today`.
    pub fn is_overdue(&self, today: Date) -> bool {
        self.is_unfinished() && self.due_date().is_some_and(|due| due < today)
    }
}

/// What a column header shows for a scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Open and doing tasks.
    pub open_and_doing: usize,
    /// Those of them due before today.
    pub overdue: usize,
}

/// Why a note gives no tasks, if it gives none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoteFlags {
    /// Its front matter turns its tasks off.
    pub hidden: bool,
    /// It lies under an excluded folder.
    pub excluded: bool,
    /// Its text could not be read, so its tasks are not known.
    pub unreadable: bool,
}

/// What a scope's queries left out. A note counts under the first reason
/// that holds: excluded, then hidden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// Notes with a task, hidden by their front matter, and their tasks.
    pub hidden_notes: usize,
    pub hidden_tasks: usize,
    /// Notes with a task under an excluded folder, and their tasks.
    pub excluded_notes: usize,
    pub excluded_tasks: usize,
    /// Notes whose text could not be read.
    pub unreadable_notes: usize,
}

/// Whether a note's front matter turns its tasks off: a top-level
/// `tasks: off` (or `false` or `no`), or Obsidian's
/// `TP_ignore_this_file: true` (or `yes` or `on`). Values may be quoted
/// and are read without regard to ASCII case. Each key hides on its own:
/// `tasks: on` does not show a note Obsidian's key hides. Of a key written
/// twice, the last one counts, and a value Wren does not read hides
/// nothing.
///
/// Front matter is what the classifier reads as it
/// ([`wren_markdown::block::front_matter_end`]): a leading `---` that is a
/// thematic break opens none. Two `key: value` lines need no YAML library.
pub fn hides_tasks(source: &str) -> bool {
    let unmarked = source.strip_prefix('\u{feff}').unwrap_or(source);
    if !unmarked.starts_with("---") {
        return false;
    }
    // The lines up to the first that may close the front matter: all
    // `front_matter_end` reads.
    let mut head: Vec<&str> = Vec::new();
    for (range, _) in wren_markdown::line_ranges(source) {
        let line = &source[range];
        let closes =
            !head.is_empty() && matches!(line.trim_end_matches([' ', '\t']), "---" | "...");
        head.push(line);
        if closes {
            break;
        }
    }
    let Some(end) = block::front_matter_end(&head[..]) else {
        return false;
    };
    let (mut own, mut obsidian) = (None, None);
    for line in &head[1..end] {
        // A value that is not read (a quote not closed as expected) still
        // makes its key the last of that name: it hides nothing.
        let value = key_value(line).map(|(_, value)| value);
        match plain_key(line) {
            Some("tasks") => own = value.and_then(switch),
            Some("TP_ignore_this_file") => obsidian = value.and_then(switch),
            _ => {}
        }
    }
    own == Some(false) || obsidian == Some(true)
}

/// A top-level `key: value` line of front matter: the key, and the value
/// without its quotes or a comment after it.
pub(crate) fn key_value(line: &str) -> Option<(&str, &str)> {
    let key = plain_key(line)?.len();
    let value = line[key + 1..].trim();
    for quote in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(quote) {
            let close = inner.find(quote)?;
            let after = inner[close + 1..].trim_start();
            return (after.is_empty() || after.starts_with('#'))
                .then_some((&line[..key], &inner[..close]));
        }
    }
    // A `#` after a space or a tab starts a comment: the first such.
    let comment = (value.match_indices('#'))
        .find(|(at, _)| value[..*at].ends_with([' ', '\t']))
        .map(|(at, _)| at);
    let value = match comment {
        Some(comment) => value[..comment].trim_end(),
        None => value,
    };
    Some((&line[..key], value))
}

/// The key of a top-level `key: value` line of front matter, whatever
/// its value.
fn plain_key(line: &str) -> Option<&str> {
    let key = line
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    let rest = line[key..].strip_prefix(':')?;
    (key > 0 && (rest.is_empty() || rest.starts_with([' ', '\t']))).then_some(&line[..key])
}

/// What a YAML 1.1 boolean says, `None` for any other value.
pub(crate) fn switch(value: &str) -> Option<bool> {
    let is = |word: &str| value.eq_ignore_ascii_case(word);
    if is("on") || is("true") || is("yes") {
        Some(true)
    } else if is("off") || is("false") || is("no") {
        Some(false)
    } else {
        None
    }
}

/// Whether `source` holds `[`, one character and `]` anywhere: without
/// them it has no task box, and is not read line by line at all.
fn may_hold_task(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut from = 0;
    while let Some(found) = source[from..].find('[') {
        let symbol = from + found + 1;
        let Some(&lead) = bytes.get(symbol) else {
            return false;
        };
        let width = match lead {
            0xf0.. => 4,
            0xe0.. => 3,
            0xc0.. => 2,
            _ => 1,
        };
        if bytes.get(symbol + width) == Some(&b']') {
            return true;
        }
        from = symbol;
    }
    false
}

/// A source read whole, told from another by its length and a hash. It is
/// asked only about a note whose text the search index does not hold (it
/// compares the text itself for one it holds): one dropped for its budget,
/// which every scan sends again.
type Fingerprint = (usize, u64);

fn fingerprint(source: &str) -> Fingerprint {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(source.as_bytes());
    (source.len(), hasher.finish())
}

/// One `@name` on a task line: the line, the key the name's spellings
/// share ([`wren_markdown::task::fold_name`]) and the name as written.
/// What `crate::owners` ranks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerName {
    pub line: usize,
    pub key: String,
    pub name: String,
}

/// The `@name`s `tasks` name, one for each time it is written, in line
/// order.
pub(crate) fn names_of(tasks: &[TaskRecord]) -> impl Iterator<Item = OwnerName> + '_ {
    tasks.iter().flat_map(|record| {
        record.owners.iter().map(move |name| OwnerName {
            line: record.line,
            key: task::fold_name(name),
            name: name.clone(),
        })
    })
}

/// A note's owners, as [`TaskIndex::owner_sources`] lends them.
#[derive(Clone, Copy, Debug)]
pub struct OwnerSource<'a> {
    pub note: NoteId,
    /// When the note last changed on disk.
    pub modified: SystemTime,
    /// Another number whenever `names` is another list.
    pub stamp: u64,
    pub names: &'a [OwnerName],
}

/// Where a heading's title is written.
#[derive(Clone, Debug)]
enum Title {
    /// An ATX heading's, as bytes of its line.
    Atx(Range<usize>),
    /// A setext heading's: on the paragraph lines from this one to its
    /// underline.
    Setext { start: usize },
}

/// A heading as the reader keeps it: where it is, not what it says, so a
/// draft's headings hold none of the note's text. What it says is read
/// from its lines when a task under it is ([`said`]).
#[derive(Clone, Debug)]
struct Heading {
    /// The last of its lines: an ATX heading's own, a setext heading's
    /// underline.
    line: usize,
    title: Title,
    /// A heading whose text is nothing once its markup is taken away,
    /// which is then no heading (`outline`'s rule).
    empty: bool,
}

/// What `heading` says: its plain text, read from its lines in `lines`.
fn said<L: Lines + ?Sized>(kept: &mut Checkpoints, lines: &L, heading: &Heading) -> String {
    match &heading.title {
        Title::Atx(title) => {
            let line = lines.line(heading.line);
            outline::plain_text(line.get(title.clone()).unwrap_or_default())
        }
        Title::Setext { start } => {
            let mut parts: Vec<String> = Vec::new();
            kept.read(lines, *start..heading.line, |_, text, info, _| {
                if info.kind == Kind::Prose {
                    parts.push(text[info.text_start..].trim_end().to_owned());
                }
            });
            outline::plain_text(&parts.join(" "))
        }
    }
}

/// A paragraph line's text: the line, and the text's bytes in it.
type Part = (usize, Range<usize>);

/// Read lines `first..` of `lines`, which `kept` has been brought up to:
/// their tasks, as records of `note` at `path`, and their headings, added
/// to `headings`, which holds those of the lines before `first`. Returns
/// the tasks and how many lines were read.
fn read_lines<L: Lines + ?Sized>(
    kept: &mut Checkpoints,
    lines: &L,
    first: usize,
    (note, path): (NoteId, &RelPath),
    headings: &mut Vec<Heading>,
) -> (Vec<TaskRecord>, u64) {
    headings.truncate(headings.partition_point(|heading| heading.line < first));
    let above = headings.len();
    let before = kept.scanned();
    let mut tasks: Vec<TaskRecord> = Vec::new();
    // The heading each task found lies under, and what each heading says,
    // where this read has found out.
    let mut under: Vec<Option<usize>> = Vec::new();
    let mut texts: Vec<Option<Arc<str>>> = vec![None; above];
    // The text of each paragraph line since the last line that was none:
    // a setext heading's, when the read saw them all.
    let mut paragraph: Vec<Part> = Vec::new();
    let mut seen: Vec<(usize, Vec<Part>)> = Vec::new();
    kept.read(
        lines,
        first..lines.line_count(),
        |index, text, info, scanner| {
            let parsed = (text.contains('['))
                .then(|| checkpoints::opened_task(info, scanner))
                .flatten()
                .and_then(|found| task::parse(text, found));
            if let Some(parsed) = parsed {
                let found = parsed.task();
                under.push(headings.len().checked_sub(1));
                tasks.push(TaskRecord {
                    note,
                    path: path.clone(),
                    line: index,
                    source: text.to_owned(),
                    box_at: found.start,
                    symbol: found.symbol,
                    status: found.status,
                    due: parsed.due().map(|token| token.value),
                    due_at: parsed.due().map(|token| token.date.clone()),
                    priority: parsed.priority().map(|token| token.level),
                    owners: parsed.owner_names().map(str::to_owned).collect(),
                    done: parsed.date(DateField::Done).map(|token| token.value),
                    cancelled: (parsed.date(DateField::Cancelled)).map(|token| token.value),
                    description: parsed.description_text().to_owned(),
                    heading: None,
                });
            }
            match &info.kind {
                Kind::Prose => {
                    let end = info.text_start + text[info.text_start..].trim_end().len();
                    paragraph.push((index, info.text_start..end));
                }
                Kind::Heading { text: title, .. } => {
                    let text = outline::plain_text(&text[title.clone()]);
                    if !text.is_empty() {
                        headings.push(Heading {
                            line: index,
                            title: Title::Atx(title.clone()),
                            empty: false,
                        });
                        texts.push(Some(text.into()));
                    }
                }
                Kind::SetextUnderline { start, .. } => {
                    if *start >= first {
                        let parts = (paragraph.iter())
                            .filter(|(line, _)| line >= start)
                            .cloned()
                            .collect();
                        seen.push((headings.len(), parts));
                    }
                    headings.push(Heading {
                        line: index,
                        title: Title::Setext { start: *start },
                        empty: false,
                    });
                    texts.push(None);
                }
                _ => {}
            }
            if info.kind != Kind::Prose {
                paragraph.clear();
            }
        },
    );
    for (at, parts) in seen {
        let parts: Vec<&str> = (parts.into_iter())
            .map(|(line, range)| &lines.line(line)[range])
            .collect();
        texts[at] = Some(outline::plain_text(&parts.join(" ")).into());
    }
    // A setext heading found by this read whose paragraph began above the
    // lines read: its lines are read again, from the nearest state kept.
    for at in above..headings.len() {
        if texts[at].is_none() {
            texts[at] = Some(said(kept, lines, &headings[at]).into());
        }
        headings[at].empty = texts[at].as_deref().is_some_and(str::is_empty);
    }
    for (task, at) in tasks.iter_mut().zip(under) {
        let nearest = headings[..at.map_or(0, |at| at + 1)]
            .iter()
            .rposition(|heading| !heading.empty);
        task.heading = nearest.map(|at| {
            texts[at]
                .get_or_insert_with(|| said(kept, lines, &headings[at]).into())
                .clone()
        });
    }
    (tasks, kept.scanned() - before)
}

/// A note's lines as byte ranges of its source, split as
/// [`wren_markdown::line_ranges`] splits them, which is how the editor
/// numbers lines.
struct Split<'a> {
    source: &'a str,
    ranges: &'a [Range<usize>],
}

impl Lines for Split<'_> {
    fn line_count(&self) -> usize {
        self.ranges.len()
    }

    fn line(&self, i: usize) -> &str {
        &self.source[self.ranges[i].clone()]
    }
}

/// How many bytes `a` and `b` start with alike, found a block at a time.
pub(crate) fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let len = a.len().min(b.len());
    let mut at = 0;
    while at + BLOCK <= len && a[at..at + BLOCK] == b[at..at + BLOCK] {
        at += BLOCK;
    }
    at + a[at..len]
        .iter()
        .zip(&b[at..len])
        .take_while(|(x, y)| x == y)
        .count()
}

/// What the search index says of the text it held for the open note
/// before the one arriving: how long it was, and how many bytes the two
/// start with alike. The index has that text; the task index keeps none.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Before {
    pub len: usize,
    pub same: usize,
}

/// What is kept of the open note between its edits, to read the next one
/// from its first changed line: where each line lay in the text last read
/// (and how long that was), the scanner states its lines left, and where
/// its headings are. Offsets and states only: none of the note's text.
#[derive(Debug)]
struct Draft {
    note: NoteId,
    len: usize,
    ranges: Vec<Range<usize>>,
    kept: Checkpoints,
    headings: Vec<Heading>,
}

impl Draft {
    /// A draft that has read nothing yet.
    fn fresh(note: NoteId) -> Self {
        Self {
            note,
            len: 0,
            ranges: Vec::new(),
            kept: Checkpoints::states(Boxes::Status),
            headings: Vec::new(),
        }
    }

    /// Read `source`, which starts with `same` bytes of the text this last
    /// read: the lines before the first one the edit touched keep their
    /// ranges, states and headings, and only the lines from there on are
    /// split and read. Returns that line, the tasks from it on, and the
    /// lines read. A fresh draft reads every line.
    fn read(&mut self, source: &str, same: usize, path: &RelPath) -> (usize, Vec<TaskRecord>, u64) {
        // A line is as it was while its text, its ending and the byte
        // after it (which may make the ending another) lie in that prefix.
        let first = self.ranges.partition_point(|range| range.end + 2 <= same);
        let start = self.ranges.get(first).map_or(0, |range| range.start);
        self.ranges.truncate(first);
        self.ranges.extend(
            wren_markdown::line_ranges(&source[start..])
                .map(|(range, _)| range.start + start..range.end + start),
        );
        self.len = source.len();
        let lines = Split {
            source,
            ranges: &self.ranges,
        };
        let first = self.kept.update_from(&lines, first);
        let (tasks, read) = read_lines(
            &mut self.kept,
            &lines,
            first,
            (self.note, path),
            &mut self.headings,
        );
        (first, tasks, read)
    }
}

#[derive(Debug)]
struct Entry {
    path: RelPath,
    modified: SystemTime,
    tasks: Vec<TaskRecord>,
    /// The owners `tasks` name, in line order.
    names: Vec<OwnerName>,
    /// The index's revision when `tasks` last changed.
    stamp: u64,
    hidden: bool,
    excluded: bool,
    unreadable: bool,
    /// The source `tasks` were read from, when it was read whole or is
    /// known to be the text on disk: the same source arriving again is not
    /// read again.
    read: Option<Fingerprint>,
    /// Whether `tasks` were last read from an edit of the open note, so
    /// `read` is not known until the note is found on disk as drafted.
    drafted: bool,
    /// Open and doing tasks, and how many of them are due on each real
    /// day.
    unfinished: usize,
    dues: BTreeMap<Date, usize>,
}

impl Entry {
    fn new(path: RelPath, modified: SystemTime, excluded: bool) -> Self {
        Self {
            path,
            modified,
            tasks: Vec::new(),
            names: Vec::new(),
            stamp: 0,
            hidden: false,
            excluded,
            unreadable: false,
            read: None,
            drafted: false,
            unfinished: 0,
            dues: BTreeMap::new(),
        }
    }

    /// Whether queries and counts read this note's tasks.
    fn listed(&self) -> bool {
        !self.tasks.is_empty() && !self.hidden && !self.excluded
    }

    /// Whether a query, a count, a flag or what was left out says
    /// anything of this note.
    fn told(&self) -> bool {
        !self.tasks.is_empty() || self.hidden || self.unreadable
    }

    /// The tasks on lines `first..` are now `tail`: only they, their
    /// tallies and their owners are touched. Returns whether they differ
    /// from the ones there were.
    fn splice(&mut self, first: usize, tail: Vec<TaskRecord>) -> bool {
        let kept = self.tasks.partition_point(|task| task.line < first);
        if self.tasks[kept..] == tail[..] {
            return false;
        }
        for task in self.tasks.drain(kept..) {
            if task.is_unfinished() {
                self.unfinished -= 1;
                if let Some(due) = task.due_date() {
                    if let Some(count) = self.dues.get_mut(&due) {
                        *count -= 1;
                        if *count == 0 {
                            self.dues.remove(&due);
                        }
                    }
                }
            }
        }
        for task in &tail {
            if task.is_unfinished() {
                self.unfinished += 1;
                if let Some(due) = task.due_date() {
                    *self.dues.entry(due).or_default() += 1;
                }
            }
        }
        let named = self.names.partition_point(|name| name.line < first);
        self.names.truncate(named);
        self.names.extend(names_of(&tail));
        self.tasks.extend(tail);
        true
    }
}

/// Counts already added up, for one revision and one day.
#[derive(Debug, Default)]
struct Counted {
    at: Option<(u64, Date)>,
    workspace: Option<Counts>,
    scopes: HashMap<RelPath, Counts>,
}

/// See the module's description.
#[derive(Debug)]
pub struct TaskIndex {
    entries: HashMap<NoteId, Entry>,
    /// The Markdown notes in the tree's order: by section, then group,
    /// then as the model lists a group's notes.
    order: Vec<NoteId>,
    /// What `order` was made from, as the model listed them: the sections,
    /// the groups with their sections, and the Markdown notes with their
    /// groups.
    sections: Vec<SectionId>,
    groups: Vec<(GroupId, SectionId)>,
    listed: Vec<(NoteId, GroupId)>,
    excluded: Vec<RelPath>,
    revision: u64,
    draft: Option<Draft>,
    /// Scanner states for a note read whole, kept only for their room.
    scratch: Checkpoints,
    lines_read: u64,
    counted: Mutex<Counted>,
}

impl Default for TaskIndex {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
            sections: Vec::new(),
            groups: Vec::new(),
            listed: Vec::new(),
            excluded: Vec::new(),
            revision: 0,
            draft: None,
            scratch: Checkpoints::states(Boxes::Status),
            lines_read: 0,
            counted: Mutex::default(),
        }
    }
}

fn under(path: &RelPath, scope: Option<&RelPath>) -> bool {
    scope.is_none_or(|scope| path.as_path().starts_with(scope.as_path()))
}

fn lies_under(path: &RelPath, folders: &[RelPath]) -> bool {
    (folders.iter()).any(|folder| path.as_path().starts_with(folder.as_path()))
}

impl TaskIndex {
    /// Moves whenever what a query, a count or a flag answers changes:
    /// records, paths, the notes' order among those with tasks, hidden
    /// notes and exclusions. A tick that changes none of them leaves it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// How many lines have been classified to keep the index current: what
    /// its upkeep cost, so a test can tell that an update read nothing.
    pub fn lines_read(&self) -> u64 {
        self.lines_read
    }

    /// The tasks of the notes at or under `scope` (the workspace for
    /// `None`; else a section's, a group's or a note's path), in the
    /// tree's order of notes, then by line. Hidden and excluded notes give
    /// none.
    pub fn tasks_under<'a>(
        &'a self,
        scope: Option<&RelPath>,
    ) -> impl Iterator<Item = &'a TaskRecord> + use<'a> {
        // Owned, so the records outlive the path they were asked by.
        let scope = scope.cloned();
        self.order
            .iter()
            .filter_map(|note| self.entries.get(note))
            .filter(move |entry| entry.listed() && under(&entry.path, scope.as_ref()))
            .flat_map(|entry| entry.tasks.iter())
    }

    /// The open and doing tasks [`TaskIndex::tasks_under`] gives for
    /// `scope`, and how many of them are due before `today`. Added up
    /// once for each scope, revision and day, so a header may ask on
    /// every frame.
    pub fn counts_under(&self, scope: Option<&RelPath>, today: Date) -> Counts {
        let mut counted = self.counted.lock().unwrap_or_else(PoisonError::into_inner);
        if counted.at != Some((self.revision, today)) {
            *counted = Counted {
                at: Some((self.revision, today)),
                ..Counted::default()
            };
        }
        let known = match scope {
            None => counted.workspace,
            Some(scope) => counted.scopes.get(scope).copied(),
        };
        if let Some(counts) = known {
            return counts;
        }
        let mut counts = Counts::default();
        for entry in self.entries.values() {
            if entry.unfinished > 0 && entry.listed() && under(&entry.path, scope) {
                counts.open_and_doing += entry.unfinished;
                counts.overdue += entry.dues.range(..today).map(|(_, due)| due).sum::<usize>();
            }
        }
        match scope {
            None => counted.workspace = Some(counts),
            Some(scope) => {
                counted.scopes.insert(scope.clone(), counts);
            }
        }
        counts
    }

    /// Why `note` gives no tasks, if it gives none. `None` for a note the
    /// index does not hold: a board, or no note of the workspace.
    pub fn flags(&self, note: NoteId) -> Option<NoteFlags> {
        self.entries.get(&note).map(|entry| NoteFlags {
            hidden: entry.hidden,
            excluded: entry.excluded,
            unreadable: entry.unreadable,
        })
    }

    /// What [`TaskIndex::tasks_under`] leaves out for `scope`.
    pub fn left_out_under(&self, scope: Option<&RelPath>) -> LeftOut {
        let mut left = LeftOut::default();
        for entry in self.entries.values() {
            if !under(&entry.path, scope) {
                continue;
            }
            if entry.unreadable {
                left.unreadable_notes += 1;
            } else if entry.tasks.is_empty() {
            } else if entry.excluded {
                left.excluded_notes += 1;
                left.excluded_tasks += entry.tasks.len();
            } else if entry.hidden {
                left.hidden_notes += 1;
                left.hidden_tasks += entry.tasks.len();
            }
        }
        left
    }

    /// The excluded folders, as `.wren/tasks.toml` lists them.
    pub fn excluded(&self) -> &[RelPath] {
        &self.excluded
    }

    /// Whether `path` is an excluded folder or lies under one.
    pub fn is_excluded(&self, path: &RelPath) -> bool {
        lies_under(path, &self.excluded)
    }

    /// Each note with an owner on a task line: what `crate::owners` reads.
    /// Hidden and excluded notes are among them, as every task line names
    /// its owners.
    pub fn owner_sources(&self) -> Vec<OwnerSource<'_>> {
        self.entries
            .iter()
            .filter(|(_, entry)| !entry.names.is_empty())
            .map(|(note, entry)| OwnerSource {
                note: *note,
                modified: entry.modified,
                stamp: entry.stamp,
                names: &entry.names,
            })
            .collect()
    }

    /// Whether the open note's line offsets and scanner states are kept,
    /// to read its next edit from the first changed line.
    pub fn holds_draft(&self) -> bool {
        self.draft.is_some()
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Take the excluded folders. Returns whether they changed.
    pub(crate) fn set_excluded(&mut self, folders: &[RelPath]) -> bool {
        if self.excluded == folders {
            return false;
        }
        self.excluded = folders.to_vec();
        for entry in self.entries.values_mut() {
            entry.excluded = lies_under(&entry.path, &self.excluded);
        }
        self.changed();
        true
    }

    /// Hold `note`, found by a scan, before its source arrives.
    pub(crate) fn admit(&mut self, note: &Note) {
        if note.kind != NoteKind::Markdown || self.entries.contains_key(&note.id) {
            return;
        }
        let excluded = self.is_excluded(&note.path);
        self.entries.insert(
            note.id,
            Entry::new(note.path.clone(), note.modified, excluded),
        );
        self.order.push(note.id);
        // A note with no task says nothing, but for its excluded flag.
        if excluded {
            self.changed();
        }
    }

    /// Follow the model: notes gone, added, renamed, moved and reordered.
    /// Returns whether what a query answers changed.
    pub(crate) fn sync(&mut self, workspace: &Workspace) -> bool {
        let before = self.revision;
        let markdown = || {
            workspace
                .notes
                .iter()
                .filter(|note| note.kind == NoteKind::Markdown)
        };
        // One lookup a note while nothing is added or gone, which is
        // nearly every time: this runs after every op.
        let mut met = 0;
        let mut added: Vec<&Note> = Vec::new();
        for note in markdown() {
            let Some(entry) = self.entries.get_mut(&note.id) else {
                added.push(note);
                continue;
            };
            met += 1;
            entry.modified = note.modified;
            if entry.path != note.path {
                let excluded = lies_under(&note.path, &self.excluded);
                let told = entry.told() || entry.excluded != excluded;
                entry.path.clone_from(&note.path);
                for task in &mut entry.tasks {
                    task.path.clone_from(&note.path);
                }
                entry.excluded = excluded;
                if told {
                    self.revision = self.revision.wrapping_add(1);
                }
            }
        }
        if met != self.entries.len() {
            let ids: HashSet<NoteId> = markdown().map(|note| note.id).collect();
            let mut gone = false;
            self.entries.retain(|note, entry| {
                let stays = ids.contains(note);
                gone |= !stays && (entry.told() || entry.excluded);
                stays
            });
            if gone {
                self.changed();
            }
            if (self.draft.as_ref()).is_some_and(|draft| !ids.contains(&draft.note)) {
                self.draft = None;
            }
        }
        for note in added {
            self.admit(note);
        }
        // The tree's order: sections as the model lists them, each one's
        // groups, each group's notes. The model lists notes by group
        // already, so one stable sort by the group's place does it, and
        // only when the folders, the notes or a note's group are listed
        // otherwise: compared in place, with nothing made, after each op.
        let sections = || workspace.sections.iter().map(|section| section.id);
        let groups = || (workspace.groups.iter()).map(|group| (group.id, group.section));
        let listed = || markdown().map(|note| (note.id, note.group));
        if !(self.sections.iter().copied().eq(sections())
            && self.groups.iter().copied().eq(groups())
            && self.listed.iter().copied().eq(listed()))
        {
            self.sections = sections().collect();
            self.groups = groups().collect();
            self.listed = listed().collect();
            let section_places: HashMap<SectionId, usize> = (self.sections.iter().enumerate())
                .map(|(place, section)| (*section, place))
                .collect();
            let mut placed: Vec<(usize, GroupId)> = (self.groups.iter())
                .map(|(group, section)| {
                    let place = section_places.get(section).copied();
                    (place.unwrap_or(usize::MAX), *group)
                })
                .collect();
            placed.sort_by_key(|(section, _)| *section);
            let group_places: HashMap<GroupId, usize> = (placed.iter().enumerate())
                .map(|(place, (_, group))| (*group, place))
                .collect();
            let mut order: Vec<(usize, NoteId)> = (self.listed.iter())
                .map(|(note, group)| {
                    let place = group_places.get(group).copied();
                    (place.unwrap_or(usize::MAX), *note)
                })
                .collect();
            order.sort_by_key(|(place, _)| *place);
            let order: Vec<NoteId> = order.into_iter().map(|(_, note)| note).collect();
            let with_tasks = |order: &[NoteId]| -> Vec<NoteId> {
                (order.iter().copied())
                    .filter(|note| {
                        (self.entries.get(note)).is_some_and(|entry| !entry.tasks.is_empty())
                    })
                    .collect()
            };
            if with_tasks(&self.order) != with_tasks(&order) {
                self.changed();
            }
            self.order = order;
        }
        self.revision != before
    }

    /// Take `note`'s source: the draft of the open note (`open`), or what
    /// was read from disk. `before` is what the search index says of the
    /// text it held for the note until now, if it held one: the open
    /// note's edit is then read from its first changed line. A note the
    /// index does not hold, such as a board, is passed over. Returns
    /// whether what a query answers changed.
    pub(crate) fn take(
        &mut self,
        note: NoteId,
        source: &str,
        open: bool,
        before: Option<Before>,
    ) -> bool {
        // The open note is the only one whose draft is kept; a note read
        // from disk is no longer open.
        let draft = self.draft.take_if(|draft| open || draft.note == note);
        let Some(entry) = self.entries.get_mut(&note) else {
            return false;
        };
        let hidden = hides_tasks(source);
        let holds = may_hold_task(source);
        // The draft goes on from the text the search index held, when
        // that is the text it last read: every source of the note passes
        // here before search keeps it, so the two are the same one.
        let resumed = match (draft, before) {
            (Some(draft), Some(before))
                if open && holds && draft.note == note && draft.len == before.len =>
            {
                Some((draft, before.same))
            }
            _ => None,
        };
        // The first line read and the tasks from it on; `None` for tasks
        // as they were.
        let (read, print, drafted) = if !holds {
            let none = (!entry.tasks.is_empty()).then(|| (0, Vec::new()));
            (none, None, false)
        } else if let Some((mut draft, same)) = resumed {
            let (first, tail, lines) = draft.read(source, same, &entry.path);
            self.draft = Some(draft);
            self.lines_read += lines;
            (Some((first, tail)), None, true)
        } else {
            // Read whole. An edit of text search held is known to be
            // another text; any other source is told by its fingerprint.
            let print = (!open || before.is_none()).then(|| fingerprint(source));
            if print.is_some() && entry.read == print && !entry.unreadable {
                (None, print, false)
            } else if open {
                let mut draft = Draft::fresh(note);
                let (_, tasks, lines) = draft.read(source, 0, &entry.path);
                self.draft = Some(draft);
                self.lines_read += lines;
                (Some((0, tasks)), print, print.is_none())
            } else {
                let ranges: Vec<Range<usize>> = wren_markdown::line_ranges(source)
                    .map(|(range, _)| range)
                    .collect();
                let lines = Split {
                    source,
                    ranges: &ranges,
                };
                self.scratch.update_from(&lines, 0);
                let (tasks, read) = read_lines(
                    &mut self.scratch,
                    &lines,
                    0,
                    (note, &entry.path),
                    &mut Vec::new(),
                );
                // Give back the room a long note took.
                self.scratch.update_from(&[""][..], 0);
                self.lines_read += read;
                (Some((0, tasks)), print, false)
            }
        };
        let spliced = read.is_some_and(|(first, tail)| entry.splice(first, tail));
        let changed = spliced || entry.hidden != hidden || entry.unreadable;
        entry.hidden = hidden;
        entry.unreadable = false;
        entry.read = print;
        entry.drafted = drafted;
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        if spliced {
            entry.stamp = self.revision;
        }
        changed
    }

    /// `note` is no longer the open one, or was read as another's is:
    /// give back what was kept of its draft.
    pub(crate) fn left(&mut self, note: NoteId) {
        self.draft.take_if(|draft| draft.note == note);
    }

    /// `note` was read from disk as `source`, the very text its tasks
    /// were last read from: it is no longer open, and tasks read from its
    /// edits are now known to be those of the text on disk.
    pub(crate) fn closed(&mut self, note: NoteId, source: &str) {
        self.left(note);
        if let Some(entry) = self.entries.get_mut(&note) {
            if entry.drafted {
                entry.drafted = false;
                entry.read = may_hold_task(source).then(|| fingerprint(source));
            }
        }
    }

    /// `note`'s text could not be read: its tasks are not known, so it
    /// gives none until a source arrives. Returns whether what a query
    /// answers changed.
    pub(crate) fn unreadable(&mut self, note: NoteId) -> bool {
        self.left(note);
        let Some(entry) = self.entries.get_mut(&note) else {
            return false;
        };
        if entry.unreadable {
            return false;
        }
        entry.unreadable = true;
        entry.hidden = false;
        entry.read = None;
        entry.drafted = false;
        self.revision = self.revision.wrapping_add(1);
        if entry.splice(0, Vec::new()) {
            entry.stamp = self.revision;
        }
        true
    }
}

/// The records of a note read on its own, as `note` at `path`, and
/// whether its front matter hides them: what the index keeps of it.
pub fn read_note(note: NoteId, path: &RelPath, source: &str) -> (Vec<TaskRecord>, bool) {
    let hidden = hides_tasks(source);
    if !may_hold_task(source) {
        return (Vec::new(), hidden);
    }
    let ranges: Vec<Range<usize>> = wren_markdown::line_ranges(source)
        .map(|(range, _)| range)
        .collect();
    let lines = Split {
        source,
        ranges: &ranges,
    };
    let mut kept = Checkpoints::states(Boxes::Status);
    kept.update_from(&lines, 0);
    let (tasks, _) = read_lines(&mut kept, &lines, 0, (note, path), &mut Vec::new());
    (tasks, hidden)
}

/// The lines of `source` that are tasks, in order: those [`read_note`]
/// gives a record. What a write to a task line checks the line against.
pub fn task_lines(source: &str) -> Vec<usize> {
    if !may_hold_task(source) {
        return Vec::new();
    }
    let ranges: Vec<Range<usize>> = wren_markdown::line_ranges(source)
        .map(|(range, _)| range)
        .collect();
    let lines = Split {
        source,
        ranges: &ranges,
    };
    let mut kept = Checkpoints::states(Boxes::Status);
    kept.update_from(&lines, 0);
    let mut found = Vec::new();
    kept.read(
        &lines,
        0..lines.line_count(),
        |index, text, info, scanner| {
            let task = (text.contains('['))
                .then(|| checkpoints::opened_task(info, scanner))
                .flatten()
                .and_then(|found| task::parse(text, found));
            if task.is_some() {
                found.push(index);
            }
        },
    );
    found
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::index::Index;
    use crate::model::{Group, Section};

    /// Thursday 19 November 2026, the design page's day.
    fn today() -> Date {
        Date::new(2026, 11, 19).unwrap()
    }

    fn date(text: &str) -> Option<Parsed> {
        Some(Parsed::Date(Date::parse_iso(text).unwrap()))
    }

    fn path(text: &str) -> RelPath {
        RelPath::new(text).unwrap()
    }

    /// `text` read as note 1 at `S/G/Note.md`.
    fn read(text: &str) -> Vec<TaskRecord> {
        read_note(NoteId(1), &path("S/G/Note.md"), text).0
    }

    #[test]
    fn task_lines_are_the_lines_a_note_gives_records_for() {
        for text in [
            "",
            "no box here",
            "- [ ] a\n  - [x] nested\n\ttext\n- plain\n- [/] b",
            "---\nkey: v\n- [ ] in front matter\n---\n- [ ] body\n",
            "```\n- [ ] fenced\n```\n> - [ ] quoted\n- [ ] last\r\n- [?] odd\r\n",
            "\u{feff}- [ ] marked\n    - [ ] code or nested\n",
        ] {
            let lines: Vec<usize> = read(text).iter().map(|task| task.line).collect();
            assert_eq!(task_lines(text), lines, "{text:?}");
        }
        assert_eq!(task_lines("- [ ] a\n```\n- [ ] b\n```\n- [ ] c"), [0, 4]);
    }

    fn note(id: u64, at: &str) -> Note {
        Note {
            id: NoteId(id),
            group: GroupId(1),
            title: at.into(),
            kind: if at.ends_with(".md") {
                NoteKind::Markdown
            } else {
                NoteKind::Board
            },
            path: path(at),
            subtitle: None,
            modified: SystemTime::UNIX_EPOCH,
            bytes: 0,
        }
    }

    fn workspace(notes: &[(u64, &str)]) -> Workspace {
        let mut workspace = Workspace::empty(PathBuf::from("/unused"));
        workspace.notes = notes.iter().map(|(id, at)| note(*id, at)).collect();
        workspace
    }

    /// A task index fed as the Session feeds it, through the search index,
    /// which holds the text a draft's edit is told from. Each change says
    /// whether the task index's revision moved.
    struct Fed(Index);

    impl std::ops::Deref for Fed {
        type Target = TaskIndex;

        fn deref(&self) -> &TaskIndex {
            self.0.tasks().expect("tasks are on")
        }
    }

    impl Fed {
        fn new(model: &Workspace) -> Self {
            let mut index = Index::with_tasks(true);
            index.sync_metadata(model);
            Self(index)
        }

        fn revised(&mut self, change: impl FnOnce(&mut Index)) -> bool {
            let before = self.revision();
            change(&mut self.0);
            self.revision() != before
        }

        /// `note`'s text is now `text`: the open note's draft, or read
        /// from disk.
        fn take(&mut self, note: NoteId, text: &str, open: bool) -> bool {
            self.revised(|index| index.set_content(note, text, open))
        }

        fn sync(&mut self, model: &Workspace) -> bool {
            self.revised(|index| drop(index.sync_metadata(model)))
        }

        fn unreadable(&mut self, note: NoteId) -> bool {
            self.revised(|index| index.ingest_scanned(note, Err("not read")))
        }

        fn set_excluded(&mut self, folders: &[RelPath]) -> bool {
            self.revised(|index| index.exclude_tasks(folders))
        }
    }

    /// An index of `notes` (id, path, text), each read from disk.
    fn index(notes: &[(u64, &str, &str)]) -> Fed {
        let listed: Vec<(u64, &str)> = notes.iter().map(|(id, at, _)| (*id, *at)).collect();
        let mut index = Fed::new(&workspace(&listed));
        for (id, _, text) in notes {
            index.take(NoteId(*id), text, false);
        }
        index
    }

    fn lines(index: &TaskIndex, scope: Option<&str>) -> Vec<String> {
        let scope = scope.map(path);
        index
            .tasks_under(scope.as_ref())
            .map(|task| format!("{}:{} {}", task.note.0, task.line, task.description))
            .collect()
    }

    #[test]
    fn a_record_holds_what_the_line_says() {
        let tasks = read(
            "- [ ] Send the deck @sam @Priya.K ⏫ 📅 2026-12-01\n\
             - [/] Draft\n\
             - [x] Shipped 📅 2026-11-10 ✅ 2026-11-12\n\
             - [-] Dropped ❌ 2026-11-13\n\
             - [?] Odd box 🔽\n\
             - [ ] No such day 📅 2026-02-30\n\
             1. [X] Numbered, capital\n\
             - [ ]\n",
        );
        let first = &tasks[0];
        assert_eq!((first.note, first.line), (NoteId(1), 0));
        assert_eq!(first.path, path("S/G/Note.md"));
        assert_eq!(
            first.source,
            "- [ ] Send the deck @sam @Priya.K ⏫ 📅 2026-12-01"
        );
        assert_eq!((first.symbol, first.status), (' ', StatusType::Open));
        assert_eq!(first.description, "Send the deck @sam @Priya.K");
        assert_eq!(first.owners, ["sam", "Priya.K"]);
        assert_eq!(first.priority, Some(Priority::High));
        assert_eq!(first.due, date("2026-12-01"));
        assert_eq!((first.done, first.cancelled), (None, None));
        assert_eq!(first.heading, None);
        let statuses: Vec<_> = tasks
            .iter()
            .map(|task| (task.symbol, task.status))
            .collect();
        assert_eq!(
            statuses,
            [
                (' ', StatusType::Open),
                ('/', StatusType::Doing),
                ('x', StatusType::Done),
                ('-', StatusType::Cancelled),
                ('?', StatusType::Open),
                (' ', StatusType::Open),
                ('X', StatusType::Done),
                (' ', StatusType::Open),
            ]
        );
        assert_eq!(tasks[2].due, date("2026-11-10"));
        assert_eq!(tasks[2].done, date("2026-11-12"));
        assert_eq!(tasks[3].cancelled, date("2026-11-13"));
        assert_eq!(tasks[4].priority, Some(Priority::Low));
        // A date that is no real day is kept, and is no day to be late by.
        assert_eq!(tasks[5].due, Some(Parsed::Invalid));
        assert_eq!(tasks[5].due_date(), None);
        assert!(!tasks[5].is_overdue(today()));
        assert_eq!(tasks[5].source, "- [ ] No such day 📅 2026-02-30");
        let at = tasks[5].due_at.clone().unwrap();
        assert_eq!(&tasks[5].source[at], "2026-02-30");
        assert_eq!(tasks[4].due_at, None);
        // Where the box is, as the note reads the line: a task nested in a
        // quote's list is indented code read alone.
        assert_eq!(tasks[0].box_at, 2);
        let nested = read("> - Parent\n>     - [ ] Child\n");
        assert_eq!(nested.len(), 1);
        assert_eq!(&nested[0].source[nested[0].box_at..][..3], "[ ]");
        assert_eq!(nested[0].box_at, 8);
        // The due date the line's tokens read, not a glyph further on.
        let kept = read("- [ ] Repair 📅 2026-02-30 [meta:: 📅 2026-04-31]");
        assert_eq!(
            &kept[0].source[kept[0].due_at.clone().unwrap()],
            "2026-02-30"
        );
        assert_eq!((tasks[7].line, tasks[7].description.as_str()), (7, ""));
        // Late is unfinished and due before today; due today is not late.
        let late = read("- [ ] a 📅 2026-11-18\n- [/] b 📅 2026-11-19\n- [x] c 📅 2026-11-01");
        let overdue: Vec<bool> = late.iter().map(|task| task.is_overdue(today())).collect();
        assert_eq!(overdue, [true, false, false]);
    }

    #[test]
    fn only_lines_the_note_reads_as_tasks_are_records() {
        let tasks = read(
            "---\n\
             title: Plan\n\
             list: |\n\
             \x20 - [ ] in front matter\n\
             ---\n\
             - [ ] real one\n\
             \x20 - [/] nested\n\
             \x20   its second line [ ] with a box\n\
             > - [ ] quoted\n\
             \n\
             ```\n\
             - [ ] fenced\n\
             ```\n\
             ~~~md\n\
             - [x] fenced too\n\
             ~~~\n\
             \n\
             \x20   - [ ] indented code\n\
             \n\
             <div>\n\
             - [ ] in html\n\
             </div>\n\
             \n\
             - plain item [ ] box later\n\
             - [ ]no space after the box\n\
             [ ] no marker\n\
             * [x] starred\n\
             + [ ] plussed\n",
        );
        let found: Vec<(usize, &str)> = tasks
            .iter()
            .map(|task| (task.line, task.description.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (5, "real one"),
                (6, "nested"),
                (8, "quoted"),
                (26, "starred"),
                (27, "plussed"),
            ]
        );
        // A leading `---` that is a thematic break opens no front matter.
        let tasks = read("---\n- [ ] after a rule\n---\n");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 1);
    }

    #[test]
    fn lines_are_numbered_and_kept_as_the_editor_splits_them() {
        for (text, line) in [
            ("Intro\r\n- [ ] Call @sam 📅 2026-12-01\r\nEnd\r\n", 1),
            ("Intro\r- [ ] Call @sam 📅 2026-12-01\rEnd", 1),
            ("Intro\n\r- [ ] Call @sam 📅 2026-12-01", 1),
            ("\n\n- [ ] Call @sam 📅 2026-12-01", 2),
        ] {
            let tasks = read(text);
            assert_eq!(tasks.len(), 1, "{text:?}");
            assert_eq!(tasks[0].line, line, "{text:?}");
            // No line ending in the source line, nor in what is read.
            assert_eq!(tasks[0].source, "- [ ] Call @sam 📅 2026-12-01", "{text:?}");
            assert_eq!(tasks[0].due, date("2026-12-01"), "{text:?}");
            assert_eq!(tasks[0].owners, ["sam"]);
        }
        // A byte order mark stays in the first line's source, as a change
        // to that line expects it; front matter after one is still read.
        let tasks = read("\u{feff}- [ ] first line\n- [x] second");
        let found: Vec<(usize, &str)> = tasks
            .iter()
            .map(|task| (task.line, task.source.as_str()))
            .collect();
        assert_eq!(
            found,
            [(0, "\u{feff}- [ ] first line"), (1, "- [x] second")]
        );
        assert_eq!(tasks[0].description, "first line");
        let marked = "\u{feff}---\ntasks: off\n---\n- [ ] hidden";
        let (tasks, hidden) = read_note(NoteId(1), &path("S/G/Note.md"), marked);
        assert!(hidden);
        assert_eq!(tasks[0].line, 3);
    }

    #[test]
    fn a_task_names_the_nearest_heading_above_it() {
        let tasks = read(
            "- [ ] before any\n\
             # Week *one*\n\
             - [ ] under week\n\
             ## Notes `raw`\n\
             text\n\
             - [ ] under notes\n\
             \n\
             Setext [title](url)\n\
             continued\n\
             ===\n\
             - [ ] under setext\n\
             ```\n\
             # not a heading\n\
             ```\n\
             - [ ] still under setext\n\
             #\n\
             - [ ] an empty heading is none\n\
             - [ ] # A task's text is no heading\n\
             - ## In an item\n\
             - [ ] under the item's\n\
             > ## Quoted\n\
             - [ ] under quoted\n",
        );
        let found: Vec<(&str, Option<&str>)> = tasks
            .iter()
            .map(|task| (task.description.as_str(), task.heading.as_deref()))
            .collect();
        assert_eq!(
            found,
            [
                ("before any", None),
                ("under week", Some("Week one")),
                ("under notes", Some("Notes raw")),
                ("under setext", Some("Setext title continued")),
                ("still under setext", Some("Setext title continued")),
                ("an empty heading is none", Some("Setext title continued")),
                (
                    "# A task's text is no heading",
                    Some("Setext title continued")
                ),
                ("under the item's", Some("In an item")),
                ("under quoted", Some("Quoted")),
            ]
        );
    }

    /// CodeRabbit on PR #13: a heading's title, of any length, is held
    /// once for all the tasks under it, not once for each.
    #[test]
    fn the_tasks_under_a_heading_share_its_title() {
        let long = "word ".repeat(2_000);
        let tasks = read(&format!(
            "# {long}\n- [ ] one\n- [ ] two\n\nSetext\n===\n- [ ] three\n- [ ] four\n"
        ));
        let titles: Vec<&Arc<str>> = (tasks.iter())
            .map(|task| task.heading.as_ref().unwrap())
            .collect();
        assert_eq!(&**titles[0], long.trim_end());
        assert!(Arc::ptr_eq(titles[0], titles[1]));
        assert_eq!(&**titles[2], "Setext");
        assert!(Arc::ptr_eq(titles[2], titles[3]));
    }

    #[test]
    fn front_matter_turns_a_notes_tasks_off() {
        for hidden in [
            "---\ntasks: off\n---\n",
            "---\ntitle: Groceries\ntasks: off\n---\n- [ ] milk",
            "---\ntasks: false\n---\n",
            "---\ntasks: No\n---\n",
            "---\ntasks:   OFF   \n---\n",
            "---\ntasks: \"off\"\n---\n",
            "---\ntasks: 'off' # a list\n---\n",
            "---\ntasks: off # a list\n---\n",
            "---\nTP_ignore_this_file: true\n---\n",
            "---\nTP_ignore_this_file: \"True\"\n...\n",
            "---\ntitle: x\nTP_ignore_this_file: yes\n---\n",
            "---\ntasks: maybe\nTP_ignore_this_file: true\n---\n",
            // Each key hides on its own: neither one's `on` shows a note
            // the other hides.
            "---\ntasks: on\nTP_ignore_this_file: true\n---\n",
            "---\ntasks: off\nTP_ignore_this_file: false\n---\n",
            // A comment starts at the first `#` after a space or a tab.
            "---\ntasks: off\t# first # second\n---\n",
            "---\ntasks: off # first\t# second\n---\n",
            "---\nTP_ignore_this_file: true\t#x #y\n---\n",
            "---\ntasks: on\ntasks: off\n---\n",
            "---\ntasks: maybe\ntasks: off\n---\n",
            "---\nTP_ignore_this_file: 7\nTP_ignore_this_file: true\n---\n",
            "\u{feff}---\ntasks: off\n---\n",
            "---\r\ntasks: off\r\n---\r\n",
            "---  \n\ntasks: off\n---\n",
        ] {
            assert!(hides_tasks(hidden), "{hidden:?}");
        }
        for shown in [
            "",
            "- [ ] milk",
            "tasks: off\n",
            "---\ntitle: x\n---\n",
            "---\ntasks: on\n---\n",
            "---\ntasks: true\n---\n",
            "---\ntasks:\n---\n",
            "---\ntasks: off-site\n---\n",
            "---\ntasks: off#site\n---\n",
            "---\ntasks: \"off\n---\n",
            "---\ntasks: 'off' and on\n---\n",
            "---\ntasks:off\n---\n",
            "---\nTasks: off\n---\n",
            "---\nmy-tasks: off\n---\n",
            "---\nTP_ignore_this_file: false\n---\n",
            "---\ntasks: on\nTP_ignore_this_file: no\n---\n",
            "---\ntasks: off\ntasks: on\n---\n",
            // The last of a key decides, and a value Wren does not read
            // hides nothing, whatever the one before it said.
            "---\ntasks: off\ntasks: maybe\n---\n",
            "---\ntasks: off\ntitle: x\ntasks:\n---\n",
            "---\nTP_ignore_this_file: true\nTP_ignore_this_file: perhaps\n---\n",
            // So does one whose quotes Wren does not read (CodeRabbit on
            // PR #13).
            "---\ntasks: off\ntasks: 'team''s choice'\n---\n",
            "---\ntasks: off\ntasks: \"off\n---\n",
            "---\nTP_ignore_this_file: true\nTP_ignore_this_file: 'it''s'\n---\n",
            // Indented, it is another key's value.
            "---\nnote: |\n  tasks: off\n---\n",
            "---\nmeta:\n  tasks: off\n---\n",
            // Not front matter: a rule, an unclosed block, text first.
            "---\n\n- tasks: off\n---\n",
            "---\ntasks: off\n",
            "Title\n---\ntasks: off\n---\n",
            // After the front matter has closed.
            "---\ntitle: x\n---\ntasks: off\n---\n",
        ] {
            assert!(!hides_tasks(shown), "{shown:?}");
        }
    }

    #[test]
    fn only_a_source_with_a_box_is_read_line_by_line() {
        for with in [
            "- [ ] a", "[x]", "a [é] b", "[🙂]", "x[[]y", "[a][b]", "[]]",
        ] {
            assert!(may_hold_task(with), "{with:?}");
        }
        for without in [
            "",
            "[",
            "[]",
            "[ab]",
            "[link](url) and [[wiki]]",
            "a ] [",
            "[é",
        ] {
            assert!(!may_hold_task(without), "{without:?}");
        }
        let mut index = index(&[(1, "S/G/A.md", "# Plain\n[link](url)\n")]);
        assert_eq!(index.lines_read(), 0);
        index.take(NoteId(1), "# Plain\n- [ ] now a task\n", false);
        assert_eq!(index.lines_read(), 3);
    }

    /// Two sections, three groups, a board, and tasks of every status.
    fn sample() -> Fed {
        index(&[
            (
                1,
                "Work/Meetings/Monday.md",
                "- [ ] agenda 📅 2026-11-18\n- [/] minutes 📅 2026-11-19\n- [x] room 📅 2026-11-01\n",
            ),
            (2, "Work/Meetings/Board.wren-board", "- [ ] not a note\n"),
            (
                3,
                "Work/Plans/Q1.md",
                "# Goals\n- [ ] hire 📅 2026-10-01 @sam\n- [-] reorg\n- [ ] undated\n",
            ),
            (4, "Home/Lists/Shop.md", "- [ ] milk 📅 2026-11-17\n"),
            (5, "Home/Lists/Empty.md", "Nothing to do.\n"),
        ])
    }

    #[test]
    fn queries_take_a_path_and_answer_note_by_note() {
        let index = sample();
        assert_eq!(
            lines(&index, None),
            [
                "1:0 agenda",
                "1:1 minutes",
                "1:2 room",
                "3:1 hire",
                "3:2 reorg",
                "3:3 undated",
                "4:0 milk"
            ]
        );
        assert_eq!(lines(&index, Some("Work")).len(), 6);
        assert_eq!(
            lines(&index, Some("Work/Plans")),
            ["3:1 hire", "3:2 reorg", "3:3 undated"]
        );
        assert_eq!(lines(&index, Some("Home/Lists/Shop.md")), ["4:0 milk"]);
        // A path is matched part by part: `Wo` is no parent of `Work`.
        assert!(lines(&index, Some("Wo")).is_empty());
        assert!(lines(&index, Some("Home/Lists/Empty.md")).is_empty());
        assert_eq!(
            index.tasks_under(None).nth(3).unwrap().heading.as_deref(),
            Some("Goals")
        );
        // A board is not held at all.
        assert_eq!(index.flags(NoteId(2)), None);
        assert_eq!(index.flags(NoteId(5)), Some(NoteFlags::default()));
    }

    #[test]
    fn records_come_in_the_trees_order_not_the_order_of_ids() {
        let mut model = workspace(&[(1, "A/Y/One.md"), (2, "A/Z/Two.md"), (3, "B/X/Three.md")]);
        let folder = |at: &str| (at.rsplit('/').next().unwrap().to_owned(), path(at));
        // Section B stands before A, and in A the group Z before Y.
        for (id, at) in [(11, "B"), (10, "A")] {
            let (name, path) = folder(at);
            let id = SectionId(id);
            model.sections.push(Section { id, name, path });
        }
        for (id, section, at) in [(5, 10, "A/Z"), (4, 10, "A/Y"), (6, 11, "B/X")] {
            let (name, path) = folder(at);
            model.groups.push(Group {
                id: GroupId(id),
                section: SectionId(section),
                name,
                path,
            });
        }
        // The model lists notes by their group's id.
        for (note, group) in model.notes.iter_mut().zip([4, 5, 6]) {
            note.group = GroupId(group);
        }
        let mut index = Fed::new(&model);
        for note in 1..=3 {
            index.take(NoteId(note), "- [ ] a\n- [ ] b", false);
        }
        let order = |index: &TaskIndex| -> Vec<(u64, usize)> {
            (index.tasks_under(None))
                .map(|task| (task.note.0, task.line))
                .collect()
        };
        assert_eq!(
            order(&index),
            [(3, 0), (3, 1), (2, 0), (2, 1), (1, 0), (1, 1)]
        );
        assert!(!index.sync(&model));
        // Sections reordered, then groups: the answers follow.
        model.sections.swap(0, 1);
        assert!(index.sync(&model));
        assert_eq!(order(&index)[0], (2, 0));
        assert_eq!(order(&index)[4], (3, 0));
        model.groups.swap(0, 1);
        assert!(index.sync(&model));
        assert_eq!(
            order(&index),
            [(1, 0), (1, 1), (2, 0), (2, 1), (3, 0), (3, 1)]
        );
        assert!(!index.sync(&model));
    }

    /// A note moved to another group without the model's flat list of
    /// notes changing takes its place among that group's notes.
    #[test]
    fn a_move_that_keeps_the_flat_order_of_notes_still_reorders() {
        let mut model = workspace(&[(10, "S/A/Alpha.md"), (11, "S/B/Beta.md")]);
        let folder = |at: &str| (at.rsplit('/').next().unwrap().to_owned(), path(at));
        let (name, at) = folder("S");
        let section = SectionId(1);
        model.sections.push(Section {
            id: section,
            name,
            path: at,
        });
        // Group B stands before A; their ids run the other way.
        for (id, at) in [(3, "S/B"), (2, "S/A")] {
            let (name, path) = folder(at);
            model.groups.push(Group {
                id: GroupId(id),
                section,
                name,
                path,
            });
        }
        model.notes[0].group = GroupId(2);
        model.notes[1].group = GroupId(3);
        let mut index = Fed::new(&model);
        index.take(NoteId(10), "- [ ] alpha", false);
        index.take(NoteId(11), "- [ ] beta", false);
        assert_eq!(lines(&index, None), ["11:0 beta", "10:0 alpha"]);
        // Alpha moves into B, whose notes go by title: the model still
        // lists Alpha, then Beta.
        model.notes[0].group = GroupId(3);
        model.notes[0].path = path("S/B/Alpha.md");
        assert!(index.sync(&model));
        assert_eq!(lines(&index, None), ["10:0 alpha", "11:0 beta"]);
        assert_eq!(lines(&index, Some("S/B")), ["10:0 alpha", "11:0 beta"]);
    }

    /// A note with no task moved into an excluded folder, or out of one,
    /// changes what its flags answer, so the revision moves.
    #[test]
    fn a_flag_that_changes_with_a_move_moves_the_revision() {
        let mut notes = vec![(1, "Work/Plans/Empty.md"), (2, "Home/Lists/Shop.md")];
        let mut index = Fed::new(&workspace(&notes));
        index.take(NoteId(1), "No task here.", false);
        index.set_excluded(&[path("Home")]);
        assert!(!index.flags(NoteId(1)).unwrap().excluded);
        notes[0].1 = "Home/Lists/Empty.md";
        assert!(index.sync(&workspace(&notes)), "now excluded");
        assert!(index.flags(NoteId(1)).unwrap().excluded);
        notes[0].1 = "Home/Lists/Renamed.md";
        assert!(!index.sync(&workspace(&notes)), "as excluded as it was");
        notes[0].1 = "Work/Plans/Empty.md";
        assert!(index.sync(&workspace(&notes)), "no longer excluded");
        assert!(!index.flags(NoteId(1)).unwrap().excluded);
        // A note made in the excluded folder, or deleted from it, has a
        // flag to tell that comes and goes with it. One made or deleted
        // elsewhere, with no task, tells nothing.
        notes.push((3, "Home/Lists/New.md"));
        assert!(index.sync(&workspace(&notes)), "an excluded note is new");
        assert!(index.flags(NoteId(3)).unwrap().excluded);
        notes.push((4, "Work/Plans/New.md"));
        assert!(!index.sync(&workspace(&notes)));
        notes.retain(|(id, _)| *id != 4);
        assert!(!index.sync(&workspace(&notes)));
        notes.retain(|(id, _)| *id != 3);
        assert!(index.sync(&workspace(&notes)), "an excluded note is gone");
        assert_eq!(index.flags(NoteId(3)), None);
    }

    /// The index holds task records and nothing else of a note: what it
    /// keeps of the open note between edits is offsets and scanner
    /// states, none of the text, headings included.
    #[test]
    fn nothing_of_a_notes_text_is_kept_but_its_tasks() {
        let prose = "PROSEWORD in a paragraph, with [a link](url) and a [x] box.\n".repeat(400);
        let text = format!(
            "---\ntitle: FRONTWORD\n---\n# HEADWORD one\n{prose}\nSETEXTWORD\n===\n\
             - plain ITEMWORD\n- [ ] the task TASKWORD @sam\n```\nCODEWORD\n```\n## TAILWORD\n"
        );
        let mut index = Fed::new(&workspace(&[(1, "S/G/Typed.md")]));
        index.take(NoteId(1), &text, false);
        let mut typed = text.clone();
        for character in "More PROSEWORD.\n- [ ] typed TASKWORD\n".chars() {
            typed.push(character);
            index.take(NoteId(1), &typed, true);
        }
        assert!(index.holds_draft());
        let held = format!("{:?}", *index);
        for kept_out in ["PROSEWORD", "FRONTWORD", "HEADWORD", "ITEMWORD", "CODEWORD"] {
            assert!(!held.contains(kept_out), "{kept_out}");
        }
        // The records, with the heading each task lies under.
        assert!(held.contains("the task TASKWORD @sam"));
        assert_eq!(
            held.matches("SETEXTWORD").count(),
            1,
            "one record's heading"
        );
        assert_eq!(held.matches("TAILWORD").count(), 1, "the other's");
        assert_eq!(index.tasks_under(None).count(), 2);
        assert_eq!(
            index.tasks_under(None).last().unwrap().heading.as_deref(),
            Some("TAILWORD")
        );
    }

    /// A note edited, saved and left is known to hold the text it was
    /// left with: when search has dropped that text and a scan sends it
    /// again, it is not read again.
    #[test]
    fn a_note_left_as_drafted_is_not_read_again_for_the_same_text() {
        let mut index = Fed::new(&workspace(&[(1, "S/G/Typed.md"), (2, "S/G/Other.md")]));
        index.take(NoteId(1), "- [ ] one\n", false);
        index.take(NoteId(2), "- [ ] other\n", false);
        for typed in [
            "- [ ] one\n-",
            "- [ ] one\n- [ ] two",
            "- [ ] one\n- [ ] two\n",
        ] {
            index.take(NoteId(1), typed, true);
        }
        // Left: read from disk as the draft was.
        index.take(NoteId(1), "- [ ] one\n- [ ] two\n", false);
        assert!(!index.holds_draft());
        let (revision, read) = (index.revision(), index.lines_read());
        index.0.shrink_budget(1, None);
        assert_eq!(index.0.content(NoteId(1)), None);
        for _ in 0..3 {
            assert!(!index.take(NoteId(1), "- [ ] one\n- [ ] two\n", false));
            assert!(!index.take(NoteId(2), "- [ ] other\n", false));
        }
        assert_eq!((index.revision(), index.lines_read()), (revision, read));
        // The open note, too, while search holds no text of it.
        index.take(NoteId(1), "- [ ] one\n- [ ] two\n- [ ] three", true);
        let read = index.lines_read();
        assert!(!index.take(NoteId(1), "- [ ] one\n- [ ] two\n- [ ] three", true));
        assert_eq!(index.lines_read(), read);
        assert_eq!(index.tasks_under(None).count(), 4);
    }

    #[test]
    fn counts_are_open_and_doing_and_those_due_before_today() {
        let index = sample();
        let count = |scope: Option<&str>, day: Date| {
            let scope = scope.map(path);
            let counts = index.counts_under(scope.as_ref(), day);
            (counts.open_and_doing, counts.overdue)
        };
        assert_eq!(count(None, today()), (5, 3));
        assert_eq!(count(Some("Work"), today()), (4, 2));
        assert_eq!(count(Some("Work/Meetings"), today()), (2, 1));
        assert_eq!(count(Some("Work/Plans/Q1.md"), today()), (2, 1));
        assert_eq!(count(Some("Home"), today()), (1, 1));
        assert_eq!(count(Some("Nowhere"), today()), (0, 0));
        // Asked again, and on another day: the day's own answer.
        assert_eq!(count(None, today()), (5, 3));
        let tomorrow = today().add_days(1).unwrap();
        assert_eq!(count(None, tomorrow), (5, 4));
        assert_eq!(count(Some("Work/Meetings"), tomorrow), (2, 2));
        assert_eq!(count(None, Date::new(2026, 10, 1).unwrap()), (5, 0));
        // The counts are those of the tasks a query gives.
        for scope in [None, Some("Work"), Some("Home/Lists")] {
            let at = scope.map(path);
            let listed: Vec<&TaskRecord> = index.tasks_under(at.as_ref()).collect();
            assert_eq!(
                count(scope, today()),
                (
                    listed.iter().filter(|task| task.is_unfinished()).count(),
                    listed
                        .iter()
                        .filter(|task| task.is_overdue(today()))
                        .count()
                )
            );
        }
    }

    #[test]
    fn hidden_and_excluded_notes_give_no_tasks_and_say_so() {
        let mut index = sample();
        let revision = index.revision();
        assert_eq!(index.left_out_under(None), LeftOut::default());
        // A note hidden by its front matter.
        assert!(index.take(
            NoteId(4),
            "---\ntasks: off\n---\n- [ ] milk 📅 2026-11-17\n",
            false
        ));
        assert!(index.revision() > revision);
        assert!(lines(&index, Some("Home")).is_empty());
        assert_eq!(index.counts_under(None, today()).open_and_doing, 4);
        assert_eq!(
            index.flags(NoteId(4)),
            Some(NoteFlags {
                hidden: true,
                ..NoteFlags::default()
            })
        );
        assert_eq!(
            index.left_out_under(Some(&path("Home"))),
            LeftOut {
                hidden_notes: 1,
                hidden_tasks: 1,
                ..LeftOut::default()
            }
        );
        assert_eq!(
            index.left_out_under(Some(&path("Work"))),
            LeftOut::default()
        );
        // Its owners are still named, and showing it again reads nothing.
        let read = index.lines_read();
        assert!(index.take(NoteId(4), "- [ ] milk 📅 2026-11-17\n", false));
        assert_eq!(lines(&index, Some("Home")), ["4:0 milk"]);
        assert!(index.lines_read() > read, "a new source is read");
        // A folder excluded: a section here, with the group under it.
        let revision = index.revision();
        assert!(index.set_excluded(&[path("Work")]));
        assert!(!index.set_excluded(&[path("Work")]));
        assert!(index.revision() > revision);
        assert_eq!(lines(&index, None), ["4:0 milk"]);
        assert!(lines(&index, Some("Work/Plans")).is_empty());
        assert_eq!(
            index.counts_under(None, today()),
            Counts {
                open_and_doing: 1,
                overdue: 1
            }
        );
        assert_eq!(
            index.counts_under(Some(&path("Work")), today()),
            Counts::default()
        );
        assert!(index.is_excluded(&path("Work")) && index.is_excluded(&path("Work/Plans")));
        assert!(!index.is_excluded(&path("Workshop")));
        assert_eq!(index.excluded(), [path("Work")]);
        assert!(index.flags(NoteId(3)).unwrap().excluded);
        assert_eq!(
            index.left_out_under(None),
            LeftOut {
                excluded_notes: 2,
                excluded_tasks: 6,
                ..LeftOut::default()
            }
        );
        // The owner on an excluded note's task is still offered.
        assert_eq!(index.owner_sources().len(), 1);
        // A note found under the folder later is excluded as it arrives.
        let mut notes = vec![
            (1, "Work/Meetings/Monday.md"),
            (3, "Work/Plans/Q1.md"),
            (4, "Home/Lists/Shop.md"),
            (5, "Home/Lists/Empty.md"),
            (6, "Work/Plans/Q2.md"),
        ];
        index.sync(&workspace(&notes));
        index.take(NoteId(6), "- [ ] plan", false);
        assert_eq!(lines(&index, None), ["4:0 milk"]);
        // Moved out of it, a note's tasks come back.
        notes[4].1 = "Home/Lists/Q2.md";
        assert!(index.sync(&workspace(&notes)));
        assert_eq!(lines(&index, None), ["4:0 milk", "6:0 plan"]);
        assert_eq!(
            index.tasks_under(None).last().unwrap().path,
            path("Home/Lists/Q2.md")
        );
        let read = index.lines_read();
        let revision = index.revision();
        assert!(index.set_excluded(&[]));
        assert_eq!(lines(&index, None).len(), 8);
        assert_eq!(
            index.lines_read(),
            read,
            "taking an exclusion away reads nothing"
        );
        assert!(index.revision() > revision);
    }

    #[test]
    fn the_revision_moves_only_when_an_answer_changes() {
        let mut index = sample();
        let revision = index.revision();
        let read = index.lines_read();
        // The same source again, from disk: not read, nothing moves.
        let text = "# Goals\n- [ ] hire 📅 2026-10-01 @sam\n- [-] reorg\n- [ ] undated\n";
        assert!(!index.take(NoteId(3), text, false));
        assert_eq!((index.revision(), index.lines_read()), (revision, read));
        // Prose changed around the tasks: read, and the records stand.
        let prose = "# Goals\n- [ ] hire 📅 2026-10-01 @sam\n- [-] reorg\n- [ ] undated\nMore.";
        assert!(!index.take(NoteId(3), prose, false));
        assert_eq!(index.revision(), revision);
        assert!(index.lines_read() > read);
        // The model as it was, and a time of change alone.
        let mut model = workspace(&[
            (1, "Work/Meetings/Monday.md"),
            (2, "Work/Meetings/Board.wren-board"),
            (3, "Work/Plans/Q1.md"),
            (4, "Home/Lists/Shop.md"),
            (5, "Home/Lists/Empty.md"),
        ]);
        assert!(!index.sync(&model));
        model.notes[2].modified = SystemTime::UNIX_EPOCH + Duration::from_secs(9);
        assert!(!index.sync(&model));
        assert_eq!(index.owner_sources()[0].modified, model.notes[2].modified);
        // A note with no task moved, renamed or gone changes no answer.
        model.notes.swap(3, 4);
        model.notes[3].path = path("Home/Lists/Renamed.md");
        assert!(!index.sync(&model));
        model.notes.remove(3);
        assert!(!index.sync(&model));
        assert_eq!(index.revision(), revision);
        // Notes with tasks in another order: the answers' order.
        model.notes.swap(0, 2);
        assert!(index.sync(&model));
        assert_eq!(lines(&index, None)[0], "3:1 hire");
        // A line changed, a note renamed, a note gone.
        let revision = index.revision();
        assert!(index.take(NoteId(4), "- [x] milk 📅 2026-11-17\n", false));
        model.notes[0].path = path("Work/Plans/Q2.md");
        assert!(index.sync(&model));
        assert_eq!(
            index.tasks_under(None).next().unwrap().path,
            path("Work/Plans/Q2.md")
        );
        model.notes.retain(|note| note.id != NoteId(4));
        assert!(index.sync(&model));
        assert_eq!(index.flags(NoteId(4)), None);
        assert!(index.revision() >= revision + 3);
    }

    #[test]
    fn a_note_that_cannot_be_read_gives_none_until_it_can() {
        let mut index = sample();
        assert!(index.unreadable(NoteId(3)));
        assert!(!index.unreadable(NoteId(3)));
        assert!(!index.unreadable(NoteId(2)), "a board is not held");
        assert!(lines(&index, Some("Work/Plans")).is_empty());
        assert!(index.flags(NoteId(3)).unwrap().unreadable);
        assert_eq!(index.left_out_under(None).unreadable_notes, 1);
        assert_eq!(index.counts_under(None, today()).open_and_doing, 3);
        // The same bytes as before it failed are read again.
        let text = "# Goals\n- [ ] hire 📅 2026-10-01 @sam\n- [-] reorg\n- [ ] undated\n";
        assert!(index.take(NoteId(3), text, false));
        assert_eq!(lines(&index, Some("Work/Plans")).len(), 3);
        assert_eq!(index.left_out_under(None), LeftOut::default());
        // A note with no task that fails and comes back still says so.
        assert!(index.unreadable(NoteId(5)));
        assert!(index.take(NoteId(5), "Nothing to do.\n", false));
        assert_eq!(index.flags(NoteId(5)), Some(NoteFlags::default()));
    }

    /// Every record of the index, as a fresh read of `text` gives note 1's.
    fn assert_fresh(index: &TaskIndex, text: &str, why: &str) {
        let (fresh, hidden) = read_note(NoteId(1), &path("S/G/Typed.md"), text);
        let entry = &index.entries[&NoteId(1)];
        assert_eq!(entry.tasks, fresh, "{why}: {text:?}");
        assert_eq!(index.flags(NoteId(1)).unwrap().hidden, hidden, "{why}");
        // What is kept beside the records is what they add up to.
        let unfinished = || fresh.iter().filter(|task| task.is_unfinished());
        assert_eq!(entry.unfinished, unfinished().count(), "{why}");
        let mut dues: BTreeMap<Date, usize> = BTreeMap::new();
        for due in unfinished().filter_map(TaskRecord::due_date) {
            *dues.entry(due).or_default() += 1;
        }
        assert_eq!(entry.dues, dues, "{why}: {text:?}");
        let names: Vec<OwnerName> = names_of(&fresh).collect();
        assert_eq!(entry.names, names, "{why}: {text:?}");
    }

    #[test]
    fn a_draft_read_from_its_first_changed_line_is_a_fresh_read() {
        let base = "---\ntitle: x\n---\n# One\n- [ ] a @sam\r\n- [ ] b 📅 2026-11-18\r- [ ] c\nTitle\nmore\n===\n- [ ] d\n```\n- [ ] e\n```\n## Two\n- [ ] f ⏫";
        let edits: Vec<String> = vec![
            base.replace("\r\n", "\n"),
            base.replace("\r- [ ] c", "\r\n- [ ] c"),
            base.replace("title: x\n---\n", "title: x\n"),
            base.replace("title: x", "tasks: off"),
            base.replace("```\n- [ ] e", "- [ ] e"),
            base.replace("# One", "# Uno"),
            base.replace("more\n===", "more\n\n==="),
            base.replace("Title\nmore", "Title"),
            base.replace("===", "---"),
            base.replace("===\n", ""),
            base.replace("## Two\n", ""),
            base.replace("- [ ] d", "- [x] d ✅ 2026-11-19"),
            format!("{base} @more\n- [ ] g"),
            format!("{base}\n\n# Three\n- [ ] h"),
            base[..base.len() / 2].to_owned(),
            format!("```\n{base}"),
            format!("\n{base}"),
            "plain text, no box".to_owned(),
            String::new(),
            base.to_owned(),
        ];
        let mut index = Fed::new(&workspace(&[(1, "S/G/Typed.md")]));
        index.take(NoteId(1), base, true);
        assert_fresh(&index, base, "first");
        let mut seed = 0x9e37_79b9_u32;
        for (round, edited) in edits.iter().enumerate() {
            index.take(NoteId(1), edited.as_str(), true);
            assert_fresh(&index, edited, &format!("edit {round}"));
            // And typed a character at a time at some place in it.
            let mut text = edited.clone();
            for typed in "\n- [ ] new @x\n# H\n".chars() {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let mut at = seed as usize % (text.len() + 1);
                while !text.is_char_boundary(at) {
                    at -= 1;
                }
                text.insert(at, typed);
                index.take(NoteId(1), text.as_str(), true);
                assert_fresh(&index, &text, &format!("edit {round} typing"));
            }
            index.take(NoteId(1), edited.as_str(), true);
            assert_fresh(&index, edited, &format!("edit {round} undone"));
        }
    }

    #[test]
    fn typing_reads_a_few_lines_and_moves_the_revision_only_for_a_task() {
        let block = "# Week\n- [ ] Ask @sam\n  more\n```\n- [ ] fenced\n```\n- [x] done\n\n";
        let mut text = block.repeat(2_500);
        text.push_str("Closing words");
        let mut index = Fed::new(&workspace(&[(1, "S/G/Typed.md"), (2, "S/G/Other.md")]));
        index.take(NoteId(1), text.as_str(), false);
        assert!(!index.holds_draft());
        assert_eq!(index.tasks_under(None).count(), 5_000);
        let revision = index.revision();
        // The first edit reads the note once more, to keep its lines.
        text.push('.');
        assert!(!index.take(NoteId(1), text.as_str(), true));
        assert!(index.holds_draft());
        for typed in " and more".chars() {
            let read = index.lines_read();
            text.push(typed);
            assert!(!index.take(NoteId(1), text.as_str(), true));
            let read = index.lines_read() - read;
            assert!(read <= checkpoints::EVERY as u64, "{read}");
        }
        assert_eq!(index.revision(), revision, "prose changes no record");
        // A task typed at the end: its record, and the others as they were.
        let first = index.entries[&NoteId(1)].tasks.clone();
        for typed in "\n- [ ] new".chars() {
            let read = index.lines_read();
            text.push(typed);
            index.take(NoteId(1), text.as_str(), true);
            // The `-` alone underlines the paragraph above it, whose
            // lines are read again for the heading's text.
            assert!(index.lines_read() - read <= 2 * checkpoints::EVERY as u64);
        }
        assert!(index.revision() > revision);
        let tasks = &index.entries[&NoteId(1)].tasks;
        assert_eq!(tasks.len(), 5_001);
        assert_eq!(tasks[..5_000], first[..]);
        assert_eq!(tasks[5_000].description, "new");
        assert_eq!(tasks[5_000].heading.as_deref(), Some("Week"));
        // Another note read from disk leaves the draft; the note itself
        // read from disk, or gone, gives it back.
        index.take(NoteId(2), "- [ ] other", false);
        assert!(index.holds_draft());
        index.take(NoteId(1), text.as_str(), false);
        assert!(!index.holds_draft());
        index.take(NoteId(1), format!("{text}!").as_str(), true);
        assert!(index.holds_draft());
        index.sync(&workspace(&[(2, "S/G/Other.md")]));
        assert!(!index.holds_draft());
    }

    /// The perf-search workload (5,000 notes of 5,000 bytes, 25 MB) with
    /// tasks in it: a heading and five task lines in every note, 25,000
    /// tasks, in 5 sections of 10 groups. Timed in two shapes, the prose
    /// as perf-search writes it (one long line a note) and broken into
    /// lines of a sentence each (some 85 lines a note): the full build, an
    /// update of one note from disk, a keystroke on a task line of the
    /// open note, what a frame asks, and what following the model costs
    /// after an op that changed nothing. Then one note of 5,000 tasks, a
    /// keystroke in its last task. Run by `make perf-tasks`.
    ///
    /// Recorded in release on the development machine, 2026-11-26, as
    /// medians (long lines / sentence lines): full build 37 ms / 44 ms;
    /// one note from disk 7 µs / 10 µs; one keystroke 1.3 µs / 1.6 µs,
    /// finding the edit included; three header counts after a change
    /// 0.4 ms, and asked again under 1 µs; every task of the workspace
    /// 0.1 ms; following an unchanged model after an op 0.1 ms. A
    /// keystroke in the last task of a note of 5,001 tasks (268 KB): 8 µs
    /// in the index, most of it finding the edit, and 16 µs for the owners
    /// to follow. The budgets asserted: 250 ms for the full build, 1 ms
    /// for the one-note update and for each keystroke, 5 ms for the counts
    /// and the query, 2 ms for the model.
    #[test]
    #[ignore = "release performance benchmark: 5000 notes / 25 MB; run explicitly with --release"]
    fn perf_tasks_5000_notes_25mb() {
        for (shape, sentence) in [
            (
                "one long line",
                "Ordinary work notes, migration planning, follow-up items. ",
            ),
            (
                "a sentence a line",
                "Ordinary work notes, migration planning, follow-up items.\n",
            ),
        ] {
            perf_tasks(shape, sentence);
        }
        perf_last_task_of_many();
    }

    fn median(mut samples: Vec<Duration>) -> (Duration, Duration) {
        samples.sort();
        (samples[samples.len() / 2], samples[samples.len() - 1])
    }

    /// What the search index tells of an edit: the text before, against
    /// the text now.
    fn edit(before: &str, now: &str) -> Option<Before> {
        Some(Before {
            len: before.len(),
            same: common_prefix(before.as_bytes(), now.as_bytes()),
        })
    }

    fn perf_tasks(shape: &str, sentence: &str) {
        let people = ["sam", "priya.k", "alice.chen", "Bob", "j-smith"];
        let text = |number: usize, extra: &str| {
            let mut text = sentence.repeat(100);
            text.truncate(4600);
            let person = people[number % people.len()];
            text.push_str(&format!(
                "\n## Actions\n- [ ] Follow up with @{person} 📅 2026-12-01\n\
                 - [/] Draft the plan ⏫ 📅 2026-11-1{}\n- [x] Book a room ✅ 2026-11-12\n\
                 - [-] Old idea ❌ 2026-11-02\n- [ ] Read [the notes](notes.md){extra}\n",
                number % 10
            ));
            assert!(text.len() <= 5000);
            text.push_str(&" ".repeat(5000 - text.len()));
            text
        };
        // 5 sections of 10 groups, each group's notes listed together as
        // the model lists them, and the groups not in the order of ids.
        let model = {
            let mut workspace = Workspace::empty(PathBuf::from("/unused"));
            for section in 0..5_u64 {
                workspace.sections.push(Section {
                    id: SectionId(100_000 + section),
                    name: format!("S{section}"),
                    path: path(&format!("S{section}")),
                });
            }
            for group in (0..50_u64).rev() {
                workspace.groups.push(Group {
                    id: GroupId(200_000 + group),
                    section: SectionId(100_000 + group % 5),
                    name: format!("G{group}"),
                    path: path(&format!("S{}/G{group}", group % 5)),
                });
            }
            workspace.notes = (0..5000_u64)
                .map(|number| {
                    let at = format!("S{}/G{}/Meeting {number}.md", number % 5, number % 50);
                    let mut note = note(number + 10, &at);
                    note.group = GroupId(200_000 + number % 50);
                    note
                })
                .collect();
            workspace.notes.sort_by_key(|note| note.group);
            workspace
        };
        let sources: Vec<String> = (0..5000).map(|number| text(number, "")).collect();
        let bytes: usize = sources.iter().map(|source| source.len()).sum();
        assert_eq!(bytes, 25_000_000);
        let (mut builds, mut updates, mut keys, mut counts, mut queries, mut syncs) = (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let mut index = TaskIndex::default();
        for _ in 0..10 {
            let started = Instant::now();
            index = TaskIndex::default();
            index.sync(&model);
            for (number, source) in sources.iter().enumerate() {
                index.take(NoteId(number as u64 + 10), source, false, None);
            }
            builds.push(started.elapsed());
        }
        let built = index.lines_read();
        assert_eq!(index.tasks_under(None).count(), 25_000);
        for round in 0..20 {
            // One note changed on disk.
            let changed = text(42, &format!(" {round}"));
            let started = Instant::now();
            assert!(index.take(NoteId(52), &changed, false, None));
            updates.push(started.elapsed());
            // A keystroke on a task line of the open note, found as the
            // search index finds it.
            let mut draft = text(7, "");
            index.take(NoteId(17), &draft, true, edit(&sources[7], &draft));
            let was = draft.clone();
            draft.truncate(draft.trim_end().len());
            draft.push_str(&format!(" typed{round}"));
            let started = Instant::now();
            assert!(index.take(NoteId(17), &draft, true, edit(&was, &draft)));
            keys.push(started.elapsed());
            // What a frame asks: three headers' counts, first and again.
            let scopes = [None, Some(path("S2")), Some(path("S2/G7"))];
            let started = Instant::now();
            for scope in &scopes {
                std::hint::black_box(index.counts_under(scope.as_ref(), today()));
            }
            counts.push(started.elapsed());
            let started = Instant::now();
            for _ in 0..1000 {
                for scope in &scopes {
                    std::hint::black_box(index.counts_under(scope.as_ref(), today()));
                }
            }
            let again = started.elapsed() / 1000;
            assert!(again < Duration::from_micros(50), "{again:?}");
            let started = Instant::now();
            let found = (index.tasks_under(None))
                .filter(|task| task.is_unfinished())
                .count();
            queries.push(started.elapsed());
            assert_eq!(found, 15_000);
            // What every op asks: the model, as it was.
            let started = Instant::now();
            assert!(!index.sync(&model));
            syncs.push(started.elapsed());
        }
        let (build, build_max) = median(builds);
        let (update, update_max) = median(updates);
        let (key, key_max) = median(keys);
        let (count, count_max) = median(counts);
        let (query, query_max) = median(queries);
        let (sync, sync_max) = median(syncs);
        eprintln!(
            "task index, {shape}: notes: 5000; bytes: {bytes}; tasks: 25000; \
             lines read by a build: {built}\n\
             \x20 full build median: {build:?}, max: {build_max:?}\n\
             \x20 one note from disk median: {update:?}, max: {update_max:?}\n\
             \x20 one keystroke in the open note median: {key:?}, max: {key_max:?}\n\
             \x20 three header counts after a change median: {count:?}, max: {count_max:?}\n\
             \x20 every task of the workspace median: {query:?}, max: {query_max:?}\n\
             \x20 the model unchanged, after an op, median: {sync:?}, max: {sync_max:?}"
        );
        assert!(build < Duration::from_millis(250), "{build:?}");
        assert!(update < Duration::from_millis(1), "{update:?}");
        assert!(key < Duration::from_millis(1), "{key:?}");
        assert!(count < Duration::from_millis(5), "{count:?}");
        assert!(query < Duration::from_millis(5), "{query:?}");
        assert!(sync < Duration::from_millis(2), "{sync:?}");
    }

    /// One open note of 5,000 tasks, each with an owner and a due date,
    /// and a character typed in the last of them: the records, the
    /// tallies and the owners before that line are not touched, and the
    /// owners suggestions offer take the one name.
    fn perf_last_task_of_many() {
        let mut text: String = (0..5_000)
            .map(|n| {
                format!(
                    "- [ ] Task {n} for the plan @p{} ⏫ 📅 2026-11-{:02}\n",
                    n % 40,
                    n % 28 + 1
                )
            })
            .collect();
        text.push_str("- [ ] The last one @sam");
        let model = workspace(&[(1, "S/G/Dense.md"), (2, "S/G/Other.md")]);
        let mut index = TaskIndex::default();
        index.sync(&model);
        index.take(NoteId(1), &text, false, None);
        index.take(NoteId(2), "- [ ] elsewhere @zoe", false, None);
        let mut was = text.clone();
        text.push('.');
        index.take(NoteId(1), &text, true, edit(&was, &text));
        let mut owners = crate::owners::Owners::default();
        let skip = Some((NoteId(1), 5_000));
        let now = SystemTime::UNIX_EPOCH;
        owners.update(1, 0, &index.owner_sources(), skip, now);
        let (mut keys, mut named) = (Vec::new(), Vec::new());
        for round in 0..200 {
            was.clone_from(&text);
            text.push(char::from(b'a' + round % 26));
            let read = index.lines_read();
            let started = Instant::now();
            assert!(index.take(NoteId(1), &text, true, edit(&was, &text)));
            keys.push(started.elapsed());
            assert!(index.lines_read() - read <= checkpoints::EVERY as u64);
            let started = Instant::now();
            owners.update(1, u64::from(round) + 1, &index.owner_sources(), skip, now);
            named.push(started.elapsed());
        }
        let entry = &index.entries[&NoteId(1)];
        assert_eq!((entry.tasks.len(), entry.unfinished), (5_001, 5_001));
        assert_eq!(entry.names.len(), 5_001);
        assert_eq!(
            index.counts_under(None, today()).overdue,
            5_000 / 28 * 18 + 18 - 2
        );
        let (key, key_max) = median(keys);
        let (name, name_max) = median(named);
        eprintln!(
            "task index, one note of 5001 tasks, {} bytes: a keystroke in the last task\n\
             \x20 the index median: {key:?}, max: {key_max:?}\n\
             \x20 the owners after it median: {name:?}, max: {name_max:?}",
            text.len()
        );
        assert!(key < Duration::from_millis(1), "{key:?}");
        assert!(name < Duration::from_millis(1), "{name:?}");
    }
}
