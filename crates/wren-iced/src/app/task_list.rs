//! The task view's list as data (discovery/tasks.md §9; item 10.3): which
//! of a scope's tasks a view's filters keep, how they are grouped, and in
//! what order. Pure functions of the task index's records, the filters,
//! the `tasks-me` names and today, so the view and its tests read the same
//! answer.
//!
//! - **Filters.** Status chips narrow (none pressed shows every status);
//!   the owner is anyone, Me (no owner, or a `tasks-me` name) or one
//!   person; a due bucket; a priority; and finished work whose `✅`/`❌`
//!   stamp is more than [`OLD_DAYS`] days old is hidden while `hide_old`
//!   is on. Finished work with no stamp counts as recent.
//! - **Groups.** By note (the tree's order), due bucket ([`bucket`]),
//!   priority (rank order) or owner (Me first, then each person by name).
//!   A task with several owners is listed under each.
//! - **Order within a group.** Open and doing before done and cancelled,
//!   overdue first, then priority rank (none between Medium and Low, §5.7),
//!   then due date (none last), then the tree's order.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use wren_core::fs_store::RelPath;
use wren_core::model::NoteId;
use wren_core::tasks::TaskRecord;
use wren_core::tasks_file::{DueBucket, GroupBy, OwnerFilter, PriorityFilter, TaskFilters};
use wren_markdown::date::{Date, Parsed};
use wren_markdown::status::StatusType;
use wren_markdown::task::{fold_name, Priority};

/// Finished work stamped more than this many days before today is hidden
/// while a view hides old work.
pub(super) const OLD_DAYS: i32 = 14;

/// The status chips, in the order they stand.
pub(super) const CHIPS: [StatusType; 4] = [
    StatusType::Open,
    StatusType::Doing,
    StatusType::Done,
    StatusType::Cancelled,
];

/// A group header's height in the list.
pub(super) const HEADER_HEIGHT: f32 = 30.0;
/// A row's height in the list: the description over its source.
pub(super) const ROW_HEIGHT: f32 = 44.0;

/// The bucket `record`'s due date falls in on `today`. Weeks run Monday to
/// Sunday; a date that is no real day has no date.
pub(super) fn bucket(record: &TaskRecord, today: Date) -> DueBucket {
    let Some(due) = record.due_date() else {
        return DueBucket::NoDate;
    };
    let days = today.days_until(due);
    let to_sunday = 6 - i32::from(today.weekday().days_from_monday());
    match days {
        ..0 if record.is_unfinished() => DueBucket::Overdue,
        ..0 => DueBucket::Earlier,
        0 => DueBucket::Today,
        days if days <= to_sunday => DueBucket::ThisWeek,
        days if days <= to_sunday + 7 => DueBucket::NextWeek,
        _ => DueBucket::Later,
    }
}

/// A priority's sort rank (§5.7): Highest 1, High 2, Medium 3, none 4,
/// Low 5, Lowest 6.
pub(super) const fn rank(priority: Option<Priority>) -> u8 {
    match priority {
        Some(Priority::Highest) => 1,
        Some(Priority::High) => 2,
        Some(Priority::Medium) => 3,
        None => 4,
        Some(Priority::Low) => 5,
        Some(Priority::Lowest) => 6,
    }
}

/// The finished work's stamp: the `✅` date of a done task, the `❌` date
/// of a cancelled one, either when only the other is written.
fn stamp(record: &TaskRecord) -> Option<Date> {
    let date = |parsed: Option<Parsed>| match parsed {
        Some(Parsed::Date(date)) => Some(date),
        _ => None,
    };
    let (first, second) = match record.status {
        StatusType::Cancelled => (record.cancelled, record.done),
        _ => (record.done, record.cancelled),
    };
    date(first).or_else(|| date(second))
}

/// Whether `record` is finished work stamped more than [`OLD_DAYS`] days
/// before `today`. Unstamped work is never old.
pub(super) fn is_old(record: &TaskRecord, today: Date) -> bool {
    record.status.is_finished()
        && stamp(record).is_some_and(|stamped| stamped.days_until(today) > OLD_DAYS)
}

/// Whether `record` is the user's: it has no owner, or one of its owners
/// is a `tasks-me` name (`me`, case folded).
pub(super) fn is_mine(record: &TaskRecord, me: &[String]) -> bool {
    record.owners.is_empty()
        || record
            .owners
            .iter()
            .any(|owner| me.contains(&fold_name(owner)))
}

/// Whether `filters` keep `record`, leaving the status chips out when
/// `any_status`: what a chip's count counts.
pub(super) fn keeps(
    record: &TaskRecord,
    filters: &TaskFilters,
    me: &[String],
    today: Date,
    any_status: bool,
) -> bool {
    if !any_status && !filters.statuses.is_empty() && !filters.statuses.contains(&record.status) {
        return false;
    }
    let owner = match &filters.owner {
        OwnerFilter::Anyone => true,
        OwnerFilter::Me => is_mine(record, me),
        OwnerFilter::Name(name) => {
            let name = fold_name(name);
            record.owners.iter().any(|owner| fold_name(owner) == name)
        }
    };
    let due = filters.due.is_none_or(|due| bucket(record, today) == due);
    let priority = match filters.priority {
        None => true,
        Some(PriorityFilter::Level(level)) => record.priority == Some(level),
        Some(PriorityFilter::Unset) => record.priority.is_none(),
    };
    owner && due && priority && !(filters.hide_old && is_old(record, today))
}

/// Whether any of `filters` narrows beyond the defaults' every task: what
/// the empty list's hint names.
pub(super) fn narrowed(filters: &TaskFilters) -> bool {
    !filters.statuses.is_empty()
        || filters.owner != OwnerFilter::Anyone
        || filters.due.is_some()
        || filters.priority.is_some()
}

/// Order within a group; `a` and `b` are indices into the records, which
/// are in the tree's order.
fn order(records: &[TaskRecord], today: Date, a: usize, b: usize) -> Ordering {
    let (x, y) = (&records[a], &records[b]);
    let due = |record: &TaskRecord| record.due_date().map_or((1, None), |due| (0, Some(due)));
    (y.is_unfinished().cmp(&x.is_unfinished()))
        .then_with(|| y.is_overdue(today).cmp(&x.is_overdue(today)))
        .then_with(|| rank(x.priority).cmp(&rank(y.priority)))
        .then_with(|| due(x).cmp(&due(y)))
        .then_with(|| a.cmp(&b))
}

/// What a group gathers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum GroupKey {
    Note(NoteId),
    Due(DueBucket),
    Priority(Option<Priority>),
    /// No owner, or a `tasks-me` name.
    Me,
    /// One person, by their case-folded name.
    Owner(String),
}

/// One group of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Group {
    pub key: GroupKey,
    pub label: String,
    /// A quieter note after the label: where a note lies, what Me means.
    pub detail: String,
    /// Its rows, as indices into [`Listing::records`], in order.
    pub rows: Vec<usize>,
}

/// One line of the list as it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Entry {
    /// The header of the group at this index.
    Header(usize),
    /// A task row: its group and its record.
    Row { group: usize, record: usize },
}

/// Names a task row across refreshes: its note, its line and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowKey {
    pub note: NoteId,
    pub line: usize,
    pub source: String,
}

impl RowKey {
    pub fn of(record: &TaskRecord) -> Self {
        Self {
            note: record.note,
            line: record.line,
            source: record.source.clone(),
        }
    }
}

/// A task the view wrote, held where its rows stood when it was first
/// written until the list is refreshed (R): in each group that showed it,
/// at its place there, and shown whatever the filters now say of it
/// (discovery/tasks-build.md item 10.4, the prototype's sticky rows).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Hold {
    pub note: NoteId,
    /// The line it is on, and the line as the view's last write left it:
    /// a task that no longer reads so is no longer held
    /// ([`relocate_holds`]).
    pub line: usize,
    pub source: String,
    /// Where each of its rows stood.
    pub places: Vec<Place>,
}

/// Where one row of a held task stood.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Place {
    pub group: GroupKey,
    /// The group's label and detail, for a group the row alone now keeps.
    pub label: (String, String),
    /// Its place among the group's rows.
    pub index: usize,
}

impl Hold {
    /// Whether `record` is the task held.
    #[cfg(feature = "smoke-test")]
    pub fn holds(&self, record: &TaskRecord) -> bool {
        record.note == self.note && record.line == self.line && record.source == self.source
    }
}

/// Find each task `held` among `current` (the scope's tasks now), with
/// `previous` those the list was built from: on its line, or else the one
/// line of its note that reads as it does, when no other task read so
/// before (that one is another task, which the held one cannot be told
/// from). A task that cannot be traced is let go. One pass over each list,
/// whatever the number held.
pub(super) fn relocate_holds<'a>(
    held: &mut Vec<Hold>,
    previous: &'a [TaskRecord],
    current: impl Iterator<Item = &'a TaskRecord>,
) {
    if held.is_empty() {
        return;
    }
    type Lines<'a> = HashMap<(NoteId, &'a str), Vec<usize>>;
    let (now, before) = {
        let wanted: HashSet<(NoteId, &str)> = (held.iter())
            .map(|hold| (hold.note, hold.source.as_str()))
            .collect();
        let gather = |records: &mut dyn Iterator<Item = &'a TaskRecord>| {
            let mut lines: Lines<'a> = HashMap::new();
            for record in records {
                let key = (record.note, record.source.as_str());
                if wanted.contains(&key) {
                    lines.entry(key).or_default().push(record.line);
                }
            }
            lines
        };
        let mut current = current;
        (gather(&mut current), gather(&mut previous.iter()))
    };
    held.retain_mut(|hold| {
        let key = (hold.note, hold.source.as_str());
        let found = now.get(&key).map_or(&[][..], Vec::as_slice);
        if found.contains(&hold.line) {
            return true;
        }
        let others = (before.get(&key).into_iter().flatten())
            .filter(|&&line| line != hold.line)
            .count();
        match found {
            [line] if others == 0 => {
                hold.line = *line;
                true
            }
            _ => false,
        }
    });
}

/// A scope's tasks as a view lists them.
#[derive(Debug, Clone, Default)]
pub(super) struct Listing {
    /// Every task of the scope, filtered or not, in the tree's order.
    pub records: Vec<TaskRecord>,
    pub groups: Vec<Group>,
    /// The groups' headers and rows in the order they are drawn.
    pub entries: Vec<Entry>,
    /// Where each entry's top is in the list, and after the last, its
    /// height.
    pub tops: Vec<f32>,
    /// What each status chip counts: the tasks every filter but the
    /// chips keeps, in [`CHIPS`] order.
    pub chip_counts: [usize; 4],
    /// The scope's owners, each once as first written, by folded name.
    pub owners: Vec<String>,
    /// How many tasks the filters keep, each once.
    #[cfg_attr(not(any(test, feature = "smoke-test")), allow(dead_code))]
    pub shown: usize,
    /// How many tasks every filter keeps but "Hide done after 14 days":
    /// what the empty list's hint names.
    pub old_hidden: usize,
}

impl Listing {
    /// The list of `records` (a scope's tasks in the tree's order) under
    /// `filters`, with `me` the `tasks-me` names and `today` the day.
    /// `place` names a note's group header: its title, and where it is.
    #[cfg(test)]
    pub fn build(
        records: Vec<TaskRecord>,
        filters: &TaskFilters,
        me: &[String],
        today: Date,
        place: &dyn Fn(&TaskRecord) -> (String, String),
    ) -> Self {
        Self::build_holding(records, filters, me, today, place, &[])
    }

    /// [`Listing::build`], with the rows `held` kept where they stood. The
    /// chips count as if nothing were held.
    pub fn build_holding(
        records: Vec<TaskRecord>,
        filters: &TaskFilters,
        me: &[String],
        today: Date,
        place: &dyn Fn(&TaskRecord) -> (String, String),
        held: &[Hold],
    ) -> Self {
        let mut holds: HashMap<(NoteId, usize, &str), &Hold> = HashMap::new();
        for hold in held {
            holds
                .entry((hold.note, hold.line, hold.source.as_str()))
                .or_insert(hold);
        }
        let hold_of = |record: &TaskRecord| {
            if holds.is_empty() {
                return None;
            }
            holds
                .get(&(record.note, record.line, record.source.as_str()))
                .copied()
        };
        let mut chip_counts = [0; 4];
        let mut owners: Vec<(String, String)> = Vec::new();
        let mut kept = Vec::new();
        let mut old_hidden = 0;
        let with_old = filters.hide_old.then(|| TaskFilters {
            hide_old: false,
            ..filters.clone()
        });
        for (index, record) in records.iter().enumerate() {
            for owner in &record.owners {
                let folded = fold_name(owner);
                if !owners.iter().any(|(key, _)| *key == folded) {
                    owners.push((folded, owner.clone()));
                }
            }
            let wide = keeps(record, filters, me, today, true);
            if wide {
                if let Some(chip) = CHIPS.iter().position(|status| *status == record.status) {
                    chip_counts[chip] += 1;
                }
            }
            if hold_of(record).is_some() || (wide && keeps(record, filters, me, today, false)) {
                kept.push(index);
            } else if with_old.as_ref().is_some_and(|with_old| {
                is_old(record, today) && keeps(record, with_old, me, today, false)
            }) {
                old_hidden += 1;
            }
        }
        owners.sort();
        let mut groups: Vec<Group> = Vec::new();
        let mut found: HashMap<GroupKey, usize> = HashMap::new();
        let mut add =
            |key: GroupKey, record: usize, label: &dyn Fn() -> (String, String)| match found
                .get(&key)
            {
                Some(&group) => groups[group].rows.push(record),
                None => {
                    let (label, detail) = label();
                    found.insert(key.clone(), groups.len());
                    groups.push(Group {
                        key,
                        label,
                        detail,
                        rows: vec![record],
                    });
                }
            };
        for &index in &kept {
            let record = &records[index];
            if let Some(hold) = hold_of(record) {
                for held in &hold.places {
                    // A note's header is the note's as it is now (renamed
                    // or moved inside Wren); the others say what they
                    // gathered when the row was held.
                    match &held.group {
                        GroupKey::Note(_) => add(held.group.clone(), index, &|| place(record)),
                        _ => add(held.group.clone(), index, &|| held.label.clone()),
                    }
                }
                continue;
            }
            match filters.group_by {
                GroupBy::Note => add(GroupKey::Note(record.note), index, &|| place(record)),
                GroupBy::Due => {
                    let due = bucket(record, today);
                    add(GroupKey::Due(due), index, &|| {
                        (due.label().to_owned(), String::new())
                    });
                }
                GroupBy::Priority => {
                    let level = record.priority;
                    add(GroupKey::Priority(level), index, &|| {
                        let label = level.map_or_else(
                            || "No priority".to_owned(),
                            |level| format!("{} {}", level.glyph(), level.name()),
                        );
                        (label, String::new())
                    });
                }
                GroupBy::Owner => {
                    let mut keys: Vec<(GroupKey, &str)> = Vec::new();
                    if record.owners.is_empty() {
                        keys.push((GroupKey::Me, ""));
                    }
                    for owner in &record.owners {
                        let folded = fold_name(owner);
                        let key = if me.contains(&folded) {
                            GroupKey::Me
                        } else {
                            GroupKey::Owner(folded)
                        };
                        if !keys.iter().any(|(known, _)| *known == key) {
                            keys.push((key, owner));
                        }
                    }
                    for (key, owner) in keys {
                        let label = match &key {
                            GroupKey::Me => ("Me".to_owned(), "no owner, or tasks-me".to_owned()),
                            _ => (format!("@{owner}"), String::new()),
                        };
                        add(key, index, &|| label.clone());
                    }
                }
            }
        }
        // By note the groups stand in the tree's order, which is the order
        // they were met in; the others in their own order.
        let place_of = |key: &GroupKey| match key {
            GroupKey::Note(_) => (0, 0, String::new()),
            GroupKey::Due(due) => (0, *due as u8, String::new()),
            GroupKey::Priority(level) => (0, rank(*level), String::new()),
            GroupKey::Me => (0, 0, String::new()),
            GroupKey::Owner(name) => (1, 0, name.clone()),
        };
        groups.sort_by_key(|group| place_of(&group.key));
        let mut entries = Vec::new();
        let mut tops = Vec::new();
        let mut top = 0.0;
        for (index, group) in groups.iter_mut().enumerate() {
            group.rows.sort_by(|&a, &b| order(&records, today, a, b));
            // A held row goes back to its place, the others round it.
            let mut pinned: Vec<(usize, usize)> = (group.rows.iter())
                .filter_map(|&row| {
                    let hold = hold_of(&records[row])?;
                    let place = hold.places.iter().find(|place| place.group == group.key)?;
                    Some((place.index, row))
                })
                .collect();
            if !pinned.is_empty() {
                group
                    .rows
                    .retain(|row| !pinned.iter().any(|(_, held)| held == row));
                pinned.sort_unstable();
                for (at, row) in pinned {
                    let at = at.min(group.rows.len());
                    group.rows.insert(at, row);
                }
            }
            entries.push(Entry::Header(index));
            tops.push(top);
            top += HEADER_HEIGHT;
            for &record in &group.rows {
                entries.push(Entry::Row {
                    group: index,
                    record,
                });
                tops.push(top);
                top += ROW_HEIGHT;
            }
        }
        tops.push(top);
        Self {
            records,
            groups,
            entries,
            tops,
            chip_counts,
            owners: owners.into_iter().map(|(_, name)| name).collect(),
            shown: kept.len(),
            old_hidden,
        }
    }

    /// The whole list's height.
    pub fn height(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.0)
    }

    /// The record of the entry at `index`, when it is a row.
    pub fn record(&self, index: usize) -> Option<&TaskRecord> {
        match self.entries.get(index)? {
            Entry::Row { record, .. } => self.records.get(*record),
            Entry::Header(_) => None,
        }
    }

    /// The entries whose rows meet the band `top..top + height`.
    pub fn visible(&self, top: f32, height: f32) -> std::ops::Range<usize> {
        let count = self.entries.len();
        let first = self.tops[..count]
            .partition_point(|&start| start <= top)
            .saturating_sub(1);
        let last = self.tops[..count].partition_point(|&start| start < top + height);
        first.min(last)..last.max(first)
    }

    /// The row entries in the order the list draws them.
    pub fn rows(&self) -> impl Iterator<Item = usize> + '_ {
        (self.entries.iter().enumerate())
            .filter(|(_, entry)| matches!(entry, Entry::Row { .. }))
            .map(|(index, _)| index)
    }

    /// Where the row `key` names is now: the entry with the same note,
    /// line and text; else the same note and text, nearest the line; else
    /// none. Within the group `near` when that group holds it.
    pub fn find(&self, key: &RowKey, near: Option<&GroupKey>) -> Option<usize> {
        let matches = |exact: bool| {
            self.rows().filter(move |&index| {
                let record = self.record(index).expect("a row");
                record.note == key.note
                    && record.source == key.source
                    && (!exact || record.line == key.line)
            })
        };
        let in_group = |index: &usize| match (near, self.entries[*index]) {
            (Some(near), Entry::Row { group, .. }) => self.groups[group].key == *near,
            _ => false,
        };
        let best = |found: Vec<usize>| {
            found
                .iter()
                .copied()
                .find(in_group)
                .or_else(|| found.first().copied())
        };
        let exact: Vec<usize> = matches(true).collect();
        if !exact.is_empty() {
            return best(exact);
        }
        let moved: Vec<usize> = matches(false).collect();
        let nearest = moved.iter().map(|&index| {
            let line = self.record(index).expect("a row").line;
            line.abs_diff(key.line)
        });
        let distance = nearest.clone().min()?;
        best(
            moved
                .iter()
                .zip(nearest)
                .filter(|(_, gap)| *gap == distance)
                .map(|(&index, _)| index)
                .collect(),
        )
    }
}

/// A due date as written, when it is in a date's shape but no real day:
/// what a row shows, marked, in place of a date. The date the line's
/// tokens read as its due date, whatever else the line holds.
pub(super) fn invalid_due(record: &TaskRecord) -> Option<&str> {
    match record.due {
        Some(Parsed::Invalid) => record.source.get(record.due_at.clone()?),
        _ => None,
    }
}

/// A piece of a row's description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Piece<'a> {
    Text(&'a str),
    /// An `@owner`, with its `@`.
    Owner(&'a str),
}

/// `record`'s description in pieces, its `@owner`s apart, followed by any
/// owner written among the tokens after it; `(no description)` when it has
/// none and no owner.
pub(super) fn pieces(record: &TaskRecord) -> Vec<Piece<'_>> {
    let text = record.description.as_str();
    let mut pieces = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut plain = 0;
    let mut at = 0;
    while let Some(found) = text[at..].find('@') {
        let start = at + found;
        let after = &text[start + 1..];
        let length = after.find(char::is_whitespace).unwrap_or(after.len());
        let word = &after[..length];
        let owner = (record.owners.iter())
            .filter(|owner| word.starts_with(owner.as_str()))
            .max_by_key(|owner| owner.len());
        let opens = text[..start]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        match owner.filter(|_| opens) {
            Some(owner) => {
                if plain < start {
                    pieces.push(Piece::Text(&text[plain..start]));
                }
                let end = start + 1 + owner.len();
                pieces.push(Piece::Owner(&text[start..end]));
                seen.push(owner);
                plain = end;
                at = end;
            }
            None => at = start + 1,
        }
    }
    if plain < text.len() {
        pieces.push(Piece::Text(&text[plain..]));
    }
    for owner in &record.owners {
        if !seen.contains(&owner.as_str()) {
            seen.push(owner);
            pieces.push(Piece::Owner(owner));
        }
    }
    pieces
}

/// The pieces' text as one string, each token owner with its `@`.
#[cfg(any(test, feature = "smoke-test"))]
pub(super) fn plain(record: &TaskRecord) -> String {
    let mut out = String::new();
    for piece in pieces(record) {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Owner(owner) if owner.starts_with('@') => out.push_str(owner),
            Piece::Owner(owner) => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push('@');
                out.push_str(owner);
            }
        }
    }
    out
}

/// The group header's text for the note `record` is in: its title, and
/// "Section › Group".
pub(super) fn note_place(path: &RelPath, title: &str) -> (String, String) {
    let parts: Vec<String> = path
        .as_path()
        .parent()
        .into_iter()
        .flat_map(|parent| parent.iter())
        .map(|part| part.to_string_lossy().into_owned())
        .collect();
    (title.to_owned(), parts.join(" \u{203a} "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday 19 November 2026, the design page's day.
    fn today() -> Date {
        Date::parse_iso("2026-11-19").unwrap()
    }

    // The M11 fixture (`tools/wrentest/fixtures/tasks/list/`): two
    // groups, owners that `tasks-me` names (`sam`, `Sam.Lee`), a task with
    // two owners, finished work with and without a stamp, stamps 14 and 15
    // days old, and a date that is no real day.
    const ROADMAP: &str =
        include_str!("../../../../tools/wrentest/fixtures/tasks/list/Work/Planning/Roadmap.md");
    const STANDUP: &str =
        include_str!("../../../../tools/wrentest/fixtures/tasks/list/Work/Meetings/Standup.md");

    fn records() -> Vec<TaskRecord> {
        let mut all = Vec::new();
        for (id, path, text) in [
            (1, "Work/Planning/Roadmap.md", ROADMAP),
            (2, "Work/Meetings/Standup.md", STANDUP),
        ] {
            let (records, hidden) =
                wren_core::tasks::read_note(NoteId(id), &RelPath::new(path).unwrap(), text);
            assert!(!hidden);
            all.extend(records);
        }
        all
    }

    fn me() -> Vec<String> {
        vec!["sam".into(), "sam.lee".into()]
    }

    fn place(record: &TaskRecord) -> (String, String) {
        let title = record
            .path
            .as_path()
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        note_place(&record.path, &title)
    }

    fn list(filters: &TaskFilters) -> Listing {
        Listing::build(records(), filters, &me(), today(), &place)
    }

    /// Each group's label and its rows' descriptions, in order.
    fn shape(listing: &Listing) -> Vec<(String, Vec<String>)> {
        listing
            .groups
            .iter()
            .map(|group| {
                let rows = group
                    .rows
                    .iter()
                    .map(|&row| listing.records[row].description.clone())
                    .collect();
                (group.label.clone(), rows)
            })
            .collect()
    }

    fn by(group_by: GroupBy) -> TaskFilters {
        TaskFilters {
            group_by,
            ..TaskFilters::default()
        }
    }

    #[test]
    fn the_buckets_follow_the_week_from_today() {
        let records = records();
        let of = |description: &str| {
            let record = records
                .iter()
                .find(|record| record.description == description)
                .unwrap();
            bucket(record, today())
        };
        assert_eq!(of("Draft the Q1 plan"), DueBucket::Overdue);
        assert_eq!(of("Close the sprint"), DueBucket::Earlier);
        assert_eq!(of("Review the budget @sam"), DueBucket::Today);
        // Friday to Sunday is this week; Monday 23 to Sunday 29 the next.
        assert_eq!(of("Follow up with legal @priya"), DueBucket::ThisWeek);
        assert_eq!(of("Book the offsite @priya @Sam.Lee"), DueBucket::NextWeek);
        assert_eq!(of("Write the retro"), DueBucket::Later);
        assert_eq!(of("Order badges"), DueBucket::NoDate);
        // A date that is no real day has none.
        assert_eq!(of("Fix the date"), DueBucket::NoDate);
        let sunday = Date::parse_iso("2026-11-22").unwrap();
        let record = records
            .iter()
            .find(|record| record.description == "Book the offsite @priya @Sam.Lee")
            .unwrap();
        // From a Sunday, this week is today alone.
        assert_eq!(bucket(record, sunday), DueBucket::NextWeek);
    }

    #[test]
    fn hide_old_keeps_fourteen_days_and_unstamped_work() {
        let records = records();
        let old = |description: &str| {
            is_old(
                records
                    .iter()
                    .find(|record| record.description == description)
                    .unwrap(),
                today(),
            )
        };
        assert!(!old("Send the minutes"), "14 days");
        assert!(old("Archive old docs"), "15 days");
        assert!(!old("Tidy the wiki"), "no stamp");
        assert!(!old("Cancel the vendor call"));
        // Open work is never old.
        assert!(!old("Draft the Q1 plan"));

        let all = list(&TaskFilters::default());
        assert_eq!(all.records.len(), 13);
        assert_eq!(all.shown, 12);
        let shown = list(&TaskFilters {
            hide_old: false,
            ..TaskFilters::default()
        });
        assert_eq!(shown.shown, 13);
        assert_eq!((all.old_hidden, shown.old_hidden), (1, 0));
        // Only what the other filters keep counts: done work alone here.
        let done = list(&TaskFilters {
            statuses: vec![StatusType::Done],
            ..TaskFilters::default()
        });
        let open = list(&TaskFilters {
            statuses: vec![StatusType::Open],
            ..TaskFilters::default()
        });
        assert_eq!((done.old_hidden, open.old_hidden), (1, 0));
    }

    #[test]
    fn by_note_open_work_comes_first_overdue_then_priority_then_due() {
        let listing = list(&TaskFilters::default());
        assert_eq!(
            shape(&listing),
            [
                (
                    "Roadmap".to_owned(),
                    vec![
                        "Draft the Q1 plan".to_owned(),
                        "Book the offsite @priya @Sam.Lee".to_owned(),
                        "Review the budget @sam".to_owned(),
                        "Fix the date".to_owned(),
                        "Write the retro".to_owned(),
                        "Send the minutes".to_owned(),
                        "Tidy the wiki".to_owned(),
                    ]
                ),
                (
                    "Standup".to_owned(),
                    vec![
                        "Order badges".to_owned(),
                        "Ping finance @SAM".to_owned(),
                        "Follow up with legal @priya".to_owned(),
                        "Close the sprint".to_owned(),
                        "Cancel the vendor call".to_owned(),
                    ]
                ),
            ]
        );
        assert_eq!(listing.groups[0].detail, "Work \u{203a} Planning");
        assert_eq!(listing.entries.len(), 2 + 12);
        assert_eq!(listing.entries[0], Entry::Header(0));
        assert_eq!(listing.height(), 2.0 * HEADER_HEIGHT + 12.0 * ROW_HEIGHT);
    }

    #[test]
    fn no_priority_sorts_between_medium_and_low() {
        let mut ranks: Vec<u8> = Priority::ALL
            .iter()
            .map(|level| rank(Some(*level)))
            .collect();
        ranks.push(rank(None));
        assert_eq!(ranks, [1, 2, 3, 5, 6, 4]);
        let listing = list(&by(GroupBy::Priority));
        let labels: Vec<&str> = listing.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "🔺 Highest",
                "⏫ High",
                "🔼 Medium",
                "No priority",
                "🔽 Low",
                "⏬ Lowest"
            ]
        );
    }

    #[test]
    fn by_due_the_buckets_stand_in_order() {
        let listing = list(&by(GroupBy::Due));
        let labels: Vec<&str> = listing.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Overdue",
                "Earlier",
                "Today",
                "This week",
                "Next week",
                "Later",
                "No date"
            ]
        );
        let no_date = &shape(&listing)[6].1;
        // Open before finished, then the tree's order.
        assert_eq!(
            no_date,
            &[
                "Order badges",
                "Fix the date",
                "Ping finance @SAM",
                "Send the minutes",
                "Tidy the wiki",
                "Cancel the vendor call",
            ]
        );
    }

    #[test]
    fn by_owner_me_comes_first_and_a_task_stands_under_each_owner() {
        let listing = list(&by(GroupBy::Owner));
        let shape = shape(&listing);
        let labels: Vec<&str> = shape.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["Me", "@priya"]);
        let offsite = "Book the offsite @priya @Sam.Lee".to_owned();
        assert!(shape[0].1.contains(&offsite));
        assert!(shape[1].1.contains(&offsite));
        // `@sam`, `@SAM` and `@Sam.Lee` are all Me, and no group of their
        // own; the unowned tasks are Me too.
        assert!(shape[0].1.contains(&"Ping finance @SAM".to_owned()));
        assert!(shape[0].1.contains(&"Order badges".to_owned()));
        assert_eq!(shape[1].1.len(), 2);
        // Each task is counted once.
        assert_eq!(listing.shown, 12);
        assert_eq!(listing.rows().count(), 12 + 1);

        // With no `tasks-me`, Sam is a person of their own, after Me.
        let listing = Listing::build(records(), &by(GroupBy::Owner), &[], today(), &place);
        let labels: Vec<String> = listing.groups.iter().map(|g| g.label.clone()).collect();
        assert_eq!(labels, ["Me", "@priya", "@sam", "@Sam.Lee"]);
    }

    #[test]
    fn the_owner_filter_reads_me_and_a_person_case_folded() {
        let names = |filters: &TaskFilters| -> Vec<String> {
            let listing = list(filters);
            listing
                .rows()
                .map(|row| listing.record(row).unwrap().description.clone())
                .collect()
        };
        let mine = names(&TaskFilters {
            owner: OwnerFilter::Me,
            ..TaskFilters::default()
        });
        assert!(mine.contains(&"Review the budget @sam".to_owned()));
        assert!(mine.contains(&"Ping finance @SAM".to_owned()));
        assert!(mine.contains(&"Book the offsite @priya @Sam.Lee".to_owned()));
        assert!(mine.contains(&"Order badges".to_owned()));
        assert!(!mine.contains(&"Follow up with legal @priya".to_owned()));
        let priya = names(&TaskFilters {
            owner: OwnerFilter::Name("PRIYA".into()),
            ..TaskFilters::default()
        });
        assert_eq!(
            priya,
            [
                "Book the offsite @priya @Sam.Lee",
                "Follow up with legal @priya"
            ]
        );
        // With no `tasks-me`, Me is the tasks with no owner.
        let listing = Listing::build(
            records(),
            &TaskFilters {
                owner: OwnerFilter::Me,
                ..TaskFilters::default()
            },
            &[],
            today(),
            &place,
        );
        assert!(listing
            .rows()
            .all(|row| listing.record(row).unwrap().owners.is_empty()));
        assert_eq!(
            list(&TaskFilters::default()).owners,
            ["priya", "sam", "Sam.Lee"]
        );
    }

    #[test]
    fn chips_narrow_and_count_what_the_other_filters_keep() {
        let all = list(&TaskFilters::default());
        // Open, doing, done (one 15 days old hidden), cancelled.
        assert_eq!(all.chip_counts, [7, 1, 3, 1]);
        let done = list(&TaskFilters {
            statuses: vec![StatusType::Done],
            ..TaskFilters::default()
        });
        assert_eq!(done.shown, 3);
        assert_eq!(done.chip_counts, all.chip_counts);
        let two = list(&TaskFilters {
            statuses: vec![StatusType::Doing, StatusType::Cancelled],
            ..TaskFilters::default()
        });
        assert_eq!(two.shown, 2);
        let mine = list(&TaskFilters {
            owner: OwnerFilter::Me,
            ..TaskFilters::default()
        });
        assert_eq!(mine.chip_counts, [6, 1, 3, 1]);
        assert!(narrowed(&TaskFilters {
            statuses: vec![StatusType::Doing],
            ..TaskFilters::default()
        }));
        assert!(!narrowed(&TaskFilters {
            hide_old: false,
            group_by: GroupBy::Owner,
            ..TaskFilters::default()
        }));
    }

    #[test]
    fn due_and_priority_filters_narrow() {
        let due = list(&TaskFilters {
            due: Some(DueBucket::Overdue),
            ..TaskFilters::default()
        });
        assert_eq!(due.shown, 1);
        let none = list(&TaskFilters {
            due: Some(DueBucket::NoDate),
            ..TaskFilters::default()
        });
        assert_eq!(none.shown, 6);
        let high = list(&TaskFilters {
            priority: Some(PriorityFilter::Level(Priority::High)),
            ..TaskFilters::default()
        });
        assert_eq!(high.shown, 1);
        let unset = list(&TaskFilters {
            priority: Some(PriorityFilter::Unset),
            ..TaskFilters::default()
        });
        assert_eq!(unset.shown, 7);
    }

    #[test]
    fn an_invalid_date_is_kept_as_written() {
        let read = |line: &str| {
            let (records, _) =
                wren_core::tasks::read_note(NoteId(9), &RelPath::new("a.md").unwrap(), line);
            records.into_iter().next().unwrap()
        };
        let due = |line: &str| invalid_due(&read(line)).map(str::to_owned);
        assert_eq!(
            due("- [ ] Fix the date 📅 2026-02-30"),
            Some("2026-02-30".into())
        );
        assert_eq!(
            due("- [ ] Fix 📅\u{fe0f} 2026-13-01 ⏫"),
            Some("2026-13-01".into())
        );
        assert_eq!(
            due("- [ ] Fix [due:: 2026-02-31]"),
            Some("2026-02-31".into())
        );
        assert_eq!(due("- [ ] Fine 📅 2026-02-28"), None);
        assert_eq!(due("- [ ] None"), None);
        // The due date the tokens read, not a glyph in a field after it.
        assert_eq!(
            due("- [ ] Repair 📅 2026-02-30 [meta:: 📅 2026-04-31]"),
            Some("2026-02-30".into())
        );
        let records = records();
        let record = records
            .iter()
            .find(|record| record.description == "Fix the date")
            .unwrap();
        assert_eq!(record.due, Some(Parsed::Invalid));
        assert!(!record.is_overdue(today()));
    }

    #[test]
    fn a_description_shows_its_owners_apart() {
        let records = records();
        let find = |description: &str| {
            records
                .iter()
                .find(|record| record.description == description)
                .unwrap()
        };
        assert_eq!(
            pieces(find("Book the offsite @priya @Sam.Lee")),
            [
                Piece::Text("Book the offsite "),
                Piece::Owner("@priya"),
                Piece::Text(" "),
                Piece::Owner("@Sam.Lee"),
            ]
        );
        assert_eq!(pieces(find("Order badges")), [Piece::Text("Order badges")]);
        let (records, _) = wren_core::tasks::read_note(
            NoteId(9),
            &RelPath::new("a.md").unwrap(),
            "- [ ] Mail sam@example.com the deck 📅 2026-11-20 @priya\n",
        );
        let pieces = pieces(&records[0]);
        assert_eq!(pieces[0], Piece::Text("Mail sam@example.com the deck"));
        assert_eq!(pieces.last(), Some(&Piece::Owner("priya")));
        assert_eq!(plain(&records[0]), "Mail sam@example.com the deck @priya");
    }

    #[test]
    fn a_row_is_found_again_by_note_line_and_text() {
        let listing = list(&by(GroupBy::Owner));
        let offsite = listing
            .rows()
            .find(|&row| listing.record(row).unwrap().description.starts_with("Book"))
            .unwrap();
        let key = RowKey::of(listing.record(offsite).unwrap());
        // Under Me first, or under the group asked for.
        assert_eq!(listing.find(&key, None), Some(offsite));
        let priya = GroupKey::Owner("priya".into());
        let found = listing.find(&key, Some(&priya)).unwrap();
        assert_ne!(found, offsite);
        assert_eq!(listing.record(found).unwrap().source, key.source);
        // Moved two lines down, it is found by its text.
        let moved = RowKey {
            line: key.line + 2,
            ..key.clone()
        };
        assert_eq!(listing.find(&moved, None), Some(offsite));
        let gone = RowKey {
            source: "- [ ] gone".into(),
            ..key
        };
        assert_eq!(listing.find(&gone, None), None);
    }

    #[test]
    fn only_the_rows_in_view_are_visible() {
        let listing = list(&TaskFilters::default());
        assert_eq!(listing.visible(0.0, 1.0), 0..1);
        assert_eq!(listing.visible(0.0, HEADER_HEIGHT + 1.0), 0..2);
        let end = listing.entries.len();
        assert_eq!(listing.visible(listing.height() - 1.0, 100.0), end - 1..end);
        assert_eq!(listing.visible(0.0, 10_000.0), 0..end);
        let empty = Listing::build(Vec::new(), &TaskFilters::default(), &[], today(), &place);
        assert_eq!(empty.visible(0.0, 100.0), 0..0);
    }
}
