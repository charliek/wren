//! The owners auto-suggest offers after an `@` (discovery/tasks.md §3.1
//! Q8): every `@name` on a task line of the workspace's Markdown notes, as
//! the task index reads them ([`crate::tasks::TaskIndex::owner_sources`]).
//!
//! - Only task lines count, as the whole note reads them: not a line in a
//!   fenced block, nor one in YAML front matter, so a stray `@` in pasted
//!   mail text adds no one. A note whose tasks are hidden from the task
//!   views still names its owners.
//! - Names compare case folded ([`wren_markdown::task::fold_name`]):
//!   `@Sam` and `@sam` are one owner, and so are `@Σ` and `@ς`. An owner
//!   is offered as the spelling written most often (of equals, the first
//!   in byte order).
//! - An owner ranks by the notes it appears in, each counting more the
//!   more recently it changed: a note changed now counts 1, one changed
//!   [`HALF_LIFE_DAYS`] ago ½, and so on. So the co-workers of this week's
//!   meetings come before someone named often a year ago. Equal scores go
//!   by name.
//!
//! [`Owners`] keeps what each note names and reads again only the notes
//! whose records changed, so it follows the index on the UI thread within
//! a small budget (`perf_owner_*`): typing in one note reads that note's
//! records, which the task index has read from the first changed line.
//! The line being typed on, in the note being typed in, is left out: the
//! name half typed there is no one's yet.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::SystemTime;

use crate::model::NoteId;
use crate::tasks::{OwnerName, OwnerSource};

/// Days after which a note counts half as much.
pub const HALF_LIFE_DAYS: f64 = 30.0;

/// One owner, as offered.
#[derive(Clone, Debug, PartialEq)]
pub struct Owner {
    /// The name as written most often, without its `@`.
    pub name: String,
    /// The key its spellings share ([`wren_markdown::task::fold_name`]).
    pub key: String,
    /// How many notes name it on a task line.
    pub notes: usize,
    /// Its rank's score; see the module's description.
    pub score: f64,
}

/// `names`, but those on line `skip`.
fn counted(names: &[OwnerName], skip: Option<usize>) -> impl Iterator<Item = (&str, &str)> {
    names
        .iter()
        .filter(move |name| Some(name.line) != skip)
        .map(|name| (name.key.as_str(), name.name.as_str()))
}

/// How much a note changed at `modified` counts at `now`.
fn weight(modified: SystemTime, now: SystemTime) -> f64 {
    let age = now.duration_since(modified).unwrap_or_default();
    0.5_f64.powf(age.as_secs_f64() / 86_400.0 / HALF_LIFE_DAYS)
}

/// A line of a note left out: the one being typed on.
pub type Skip = (NoteId, usize);

/// What one note was read as: every name on its task lines, as the task
/// index had them at `stamp`, and the line left out of them, if any.
struct Read {
    stamp: u64,
    modified: SystemTime,
    skip: Option<usize>,
    names: Vec<OwnerName>,
}

impl Read {
    /// The names that count: those off the line left out.
    fn counted(&self) -> impl Iterator<Item = (&str, &str)> {
        counted(&self.names, self.skip)
    }

    /// Whether line `line`, if any, names anyone.
    fn names_on(&self, line: Option<usize>) -> bool {
        line.is_some_and(|line| self.names.iter().any(|name| name.line == line))
    }
}

/// The owners of one workspace's notes, kept up with its index.
#[derive(Default)]
pub struct Owners {
    /// The index read: its identity and revision, and the line left out.
    read: Option<(u64, u64, Option<Skip>)>,
    notes: HashMap<NoteId, Read>,
    ranked: Arc<Vec<Owner>>,
    /// Counts the changes of [`Owners::ranked`].
    generation: u64,
    /// How many times a note was read.
    #[cfg(test)]
    reads: usize,
}

impl Owners {
    /// The owners, best first.
    pub fn ranked(&self) -> &Arc<Vec<Owner>> {
        &self.ranked
    }

    /// The identity of the index last read, if any.
    pub fn identity(&self) -> Option<u64> {
        self.read.map(|(identity, _, _)| identity)
    }

    /// The revision of the index last read, if any.
    pub fn revision(&self) -> Option<u64> {
        self.read.map(|(_, revision, _)| revision)
    }

    /// A number that changes whenever [`Owners::ranked`] does.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether it was last brought up to the index `identity` at
    /// `revision`, leaving out line `skip` of a note.
    pub fn is_current(&self, identity: u64, revision: u64, skip: Option<(NoteId, usize)>) -> bool {
        self.read == Some((identity, revision, skip))
    }

    /// Forget every note: the workspace is gone.
    pub fn clear(&mut self) {
        let had = !self.ranked.is_empty();
        *self = Self {
            generation: self.generation + u64::from(had),
            ..Self::default()
        };
    }

    /// Bring it up to the index `identity` at `revision`, whose notes'
    /// owners are `sources`, leaving out line `skip` of a note, scored at
    /// `now`. Another index than the one before starts afresh. Only a note
    /// whose stamp or change time differ from the last read is read again,
    /// and of it only the names after those it still has alike are taken:
    /// a caret moved to another line reads nothing, and a name typed at
    /// the end of a long note takes that name. Returns whether the owners
    /// changed.
    pub fn update(
        &mut self,
        identity: u64,
        revision: u64,
        sources: &[OwnerSource<'_>],
        skip: Option<(NoteId, usize)>,
        now: SystemTime,
    ) -> bool {
        // Another index: what was read from the one before goes, and the
        // owners are ranked again, even from no notes at all.
        let another = self.read.is_some_and(|(known, _, _)| known != identity);
        if another {
            self.notes.clear();
        }
        self.read = Some((identity, revision, skip));
        // A note counts only by the names it has off the line left out: one
        // that comes, goes or changes with none changes no owner. The note
        // typed in comes among the sources so, with the first name typed.
        let mut changed = another;
        let mut kept: HashMap<NoteId, Read> = HashMap::with_capacity(sources.len());
        for source in sources {
            let (note, modified) = (source.note, source.modified);
            let skipped = skip.and_then(|(at, line)| (at == note).then_some(line));
            let read = match self.notes.remove(&note) {
                Some(mut read) if read.stamp == source.stamp && read.modified == modified => {
                    // Only the line left out moved: what it subtracts.
                    if read.skip != skipped {
                        changed |= read.names_on(read.skip) || read.names_on(skipped);
                        read.skip = skipped;
                    }
                    read
                }
                Some(mut read) => {
                    #[cfg(test)]
                    {
                        self.reads += 1;
                    }
                    // The names both lists start with are kept as they
                    // are. What follows them, and the line left out of
                    // what was kept, are all that may differ.
                    let same = (read.names.iter().zip(source.names))
                        .take_while(|(had, has)| had == has)
                        .count();
                    let (kept, had) = read.names.split_at(same);
                    let has = &source.names[same..];
                    let named = counted(source.names, skipped).next().is_some();
                    changed |= !counted(had, read.skip).eq(counted(has, skipped))
                        || (read.skip != skipped
                            && (kept.iter()).any(|name| {
                                Some(name.line) == read.skip || Some(name.line) == skipped
                            }))
                        || (named && read.modified != modified);
                    read.names.truncate(same);
                    read.names.extend_from_slice(has);
                    Read {
                        stamp: source.stamp,
                        modified,
                        skip: skipped,
                        names: read.names,
                    }
                }
                None => {
                    #[cfg(test)]
                    {
                        self.reads += 1;
                    }
                    changed |= counted(source.names, skipped).next().is_some();
                    Read {
                        stamp: source.stamp,
                        modified,
                        skip: skipped,
                        names: source.names.to_vec(),
                    }
                }
            };
            kept.insert(note, read);
        }
        // What is left was not among the sources: notes gone.
        changed |= (self.notes.values()).any(|read| read.counted().next().is_some());
        self.notes = kept;
        if !changed {
            return false;
        }
        let ranked = rank(&self.notes, now);
        if ranked == *self.ranked {
            return false;
        }
        self.ranked = Arc::new(ranked);
        self.generation += 1;
        true
    }
}

/// What the ranking keeps of one owner.
#[derive(Default)]
struct Tally {
    spellings: HashMap<String, usize>,
    /// Each note naming it, with its weight, in note order.
    notes: BTreeMap<NoteId, f64>,
}

fn rank(notes: &HashMap<NoteId, Read>, now: SystemTime) -> Vec<Owner> {
    let mut tallies: HashMap<&str, Tally> = HashMap::new();
    for (note, read) in notes {
        if read.names.is_empty() {
            continue;
        }
        let weight = weight(read.modified, now);
        for (key, spelling) in read.counted() {
            let tally = tallies.entry(key).or_default();
            tally.notes.insert(*note, weight);
            *tally.spellings.entry(spelling.to_owned()).or_default() += 1;
        }
    }
    let mut owners: Vec<Owner> = tallies
        .into_iter()
        .filter_map(|(key, tally)| {
            let name = tally
                .spellings
                .iter()
                .max_by(|(a, count_a), (b, count_b)| count_a.cmp(count_b).then_with(|| b.cmp(a)))?
                .0
                .clone();
            Some(Owner {
                name,
                key: key.to_owned(),
                notes: tally.notes.len(),
                score: tally.notes.values().sum(),
            })
        })
        .collect();
    owners.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.key.cmp(&b.key))
    });
    owners
}

/// A note's owners read on their own, for a scan or a test: what the
/// task index lends of a note it holds.
#[derive(Clone, Debug)]
pub struct Source {
    pub note: NoteId,
    pub modified: SystemTime,
    /// Another number for another text.
    pub stamp: u64,
    pub names: Vec<OwnerName>,
}

/// The owners on the task lines of `text`, read as a note of its own.
pub fn source(note: NoteId, modified: SystemTime, text: &str) -> Source {
    use std::hash::{Hash, Hasher};
    let path = crate::fs_store::RelPath::new("note.md").expect("a relative path");
    let (records, _) = crate::tasks::read_note(note, &path, text);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    Source {
        note,
        modified,
        stamp: hasher.finish(),
        names: crate::tasks::names_of(&records).collect(),
    }
}

/// `sources`, as [`Owners::update`] takes them.
pub fn lend(sources: &[Source]) -> Vec<OwnerSource<'_>> {
    sources
        .iter()
        .map(|source| OwnerSource {
            note: source.note,
            modified: source.modified,
            stamp: source.stamp,
            names: &source.names,
        })
        .collect()
}

/// The owners named on the task lines of `notes` (each note's last change
/// and its source), best first, scored at `now`: [`Owners`] read once.
pub fn scan<'a>(
    notes: impl IntoIterator<Item = (SystemTime, &'a str)>,
    now: SystemTime,
) -> Vec<Owner> {
    let sources: Vec<Source> = (0..)
        .zip(notes)
        .map(|(id, (modified, text))| source(NoteId(id), modified, text))
        .collect();
    let mut owners = Owners::default();
    owners.update(0, 0, &lend(&sources), None, now);
    owners.ranked.to_vec()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::fs_store::RelPath;
    use crate::index::Index;
    use crate::kind::NoteKind;
    use crate::model::{GroupId, Note, Workspace};
    use crate::tasks::TaskIndex;

    const DAY: Duration = Duration::from_secs(86_400);

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + DAY * 1_000
    }

    fn ago(days: u64) -> SystemTime {
        now() - DAY * u32::try_from(days).unwrap()
    }

    fn names(owners: &[Owner]) -> Vec<&str> {
        owners.iter().map(|owner| owner.name.as_str()).collect()
    }

    #[test]
    fn owners_come_from_task_lines_only() {
        let note = "# Meeting\n\
            \n\
            Mail @prose about it.\n\
            - [ ] Send the deck @sam 📅 2026-12-01\n\
            - [x] Done @priya.k\n\
            - [/] Doing @j-smith.\n\
            - plain item @listed\n\
            ```\n\
            - [ ] fenced @code\n\
            ```\n\
            - [ ] mail a@b.com and @Alice ⏫ @bob\n";
        let owners = scan([(now(), note)], now());
        let mut found = names(&owners);
        found.sort_unstable();
        assert_eq!(found, ["Alice", "bob", "j-smith", "priya.k", "sam"]);
        assert!(owners.iter().all(|owner| owner.notes == 1));
    }

    #[test]
    fn front_matter_adds_no_one() {
        let note = "---\n\
            title: Plan\n\
            description: |\n\
            \x20 - [ ] Draft @ghost\n\
            ---\n\
            - [ ] Real @sam\n";
        assert_eq!(names(&scan([(now(), note)], now())), ["sam"]);
        // Without front matter, the same line is a task.
        let note = "- [ ] Draft @ghost\n";
        assert_eq!(names(&scan([(now(), note)], now())), ["ghost"]);
    }

    #[test]
    fn case_folded_one_owner_written_as_most_often() {
        let notes = [
            (now(), "- [ ] a @Sam\n- [ ] b @Sam\n"),
            (now(), "- [ ] c @sam\n"),
            (now(), "- [ ] d @SAM\n- [ ] e @Bob\n- [ ] f @bob\n"),
        ];
        let owners = scan(notes, now());
        assert_eq!(names(&owners), ["Sam", "Bob"]);
        assert_eq!(owners[0].notes, 3);
        // Of equal counts, the first in byte order: `Bob` before `bob`.
        assert_eq!(owners[1].notes, 1);
        // `Σ` and the final `ς` fold alike.
        let owners = scan(
            [(now(), "- [ ] @Σ\n- [ ] @Σ\n"), (now(), "- [ ] @ς")],
            now(),
        );
        assert_eq!(names(&owners), ["Σ"]);
        assert_eq!(owners[0].notes, 2);
    }

    #[test]
    fn ranked_by_notes_and_how_recently_they_changed() {
        let notes = [
            // Often, long ago.
            (ago(365), "- [ ] @old"),
            (ago(366), "- [ ] @old"),
            (ago(367), "- [ ] @old"),
            // Twice this week.
            (ago(1), "- [ ] @weekly"),
            (ago(2), "- [ ] @weekly"),
            // Once today.
            (now(), "- [ ] @today"),
            // Once yesterday, written three times in one note.
            (ago(1), "- [ ] @once\n- [ ] @once\n- [ ] @once"),
        ];
        let owners = scan(notes, now());
        assert_eq!(names(&owners), ["weekly", "today", "once", "old"]);
        assert_eq!(owners[0].notes, 2);
        assert_eq!(owners[3].notes, 3);
        // A note dated in the future counts as now.
        let owners = scan([(now() + DAY, "- [ ] @soon")], now());
        assert!((owners[0].score - 1.0).abs() < 1e-9);
        // Equal scores go by name.
        let owners = scan([(now(), "- [ ] @b @a")], now());
        assert_eq!(names(&owners), ["a", "b"]);
    }

    fn sources(notes: &[(u64, &str)]) -> Vec<Source> {
        notes
            .iter()
            .map(|(id, text)| source(NoteId(*id), now(), text))
            .collect()
    }

    /// A search index with tasks on, holding note 1, changed at [`now`],
    /// to type in: the task index is fed as the Session feeds it.
    fn index() -> Index {
        let mut workspace = Workspace::empty(std::path::PathBuf::from("/unused"));
        workspace.notes.push(Note {
            id: NoteId(1),
            group: GroupId(1),
            title: "Typed".into(),
            kind: NoteKind::Markdown,
            path: RelPath::new("S/G/Typed.md").unwrap(),
            subtitle: None,
            modified: now(),
            bytes: 0,
        });
        let mut index = Index::with_tasks(true);
        index.sync_metadata(&workspace);
        index
    }

    /// Note 1's draft is now `text`.
    fn draft(index: &mut Index, text: &str) {
        index.set_content(NoteId(1), text, true);
    }

    fn tasks(index: &Index) -> &TaskIndex {
        index.tasks().unwrap()
    }

    #[test]
    fn a_rename_or_a_removal_is_read_again_and_only_changed_notes_are() {
        let mut owners = Owners::default();
        let first = sources(&[(1, "- [ ] @sam\n- [ ] @priya"), (2, "- [ ] @sam")]);
        assert!(owners.update(7, 1, &lend(&first), None, now()));
        assert_eq!(names(owners.ranked()), ["sam", "priya"]);
        let generation = owners.generation();
        assert!(owners.is_current(7, 1, None));
        // The same sources again: nothing read, nothing changed.
        assert!(!owners.update(7, 2, &lend(&first), None, now()));
        assert_eq!(owners.generation(), generation);
        assert!(owners.is_current(7, 2, None) && !owners.is_current(7, 1, None));
        // `@sam` renamed in one note, `@priya` removed.
        let second = vec![
            source(NoteId(1), now(), "- [ ] @samuel\n- [ ] Mail priya"),
            first[1].clone(),
        ];
        assert!(owners.update(7, 3, &lend(&second), None, now()));
        assert_eq!(names(owners.ranked()), ["sam", "samuel"]);
        assert!(owners.generation() > generation);
        // A note gone.
        assert!(owners.update(7, 4, &lend(&sources(&[(1, "- [ ] @samuel")])), None, now()));
        assert_eq!(names(owners.ranked()), ["samuel"]);
        // Another index starts afresh, and `clear` forgets all.
        assert!(owners.update(8, 0, &lend(&sources(&[(1, "- [ ] @zoe")])), None, now()));
        assert_eq!(names(owners.ranked()), ["zoe"]);
        let generation = owners.generation();
        owners.clear();
        assert!(owners.ranked().is_empty());
        assert!(owners.generation() > generation);
        assert!(!owners.is_current(8, 0, None));
    }

    #[test]
    fn the_line_being_typed_on_names_no_one() {
        let mut owners = Owners::default();
        let note = sources(&[(1, "- [ ] Ask @sam\n- [ ] Call @sa")]);
        owners.update(1, 1, &lend(&note), Some((NoteId(1), 1)), now());
        assert_eq!(names(owners.ranked()), ["sam"]);
        // Another note's line of that number counts.
        owners.update(1, 1, &lend(&note), Some((NoteId(2), 1)), now());
        assert_eq!(names(owners.ranked()), ["sa", "sam"]);
        owners.update(1, 1, &lend(&note), Some((NoteId(1), 0)), now());
        assert_eq!(names(owners.ranked()), ["sa"]);
    }

    /// Lines split as the editor splits them: a lone CR ends a line, so
    /// the task after it names its owner, and the line left out is the
    /// editor's line of that number.
    #[test]
    fn lines_split_as_the_editor_splits_them() {
        for note in [
            "Intro\r- [ ] Call @sam",
            "Intro\n\r- [ ] Call @sam",
            "Intro\r\n- [ ] Call @sam",
        ] {
            assert_eq!(names(&scan([(now(), note)], now())), ["sam"], "{note:?}");
            let mut owners = Owners::default();
            owners.update(
                1,
                1,
                &lend(&sources(&[(1, note)])),
                Some((NoteId(1), 1)),
                now(),
            );
            assert!(owners.ranked().is_empty(), "{note:?}");
        }
    }

    /// A caret moved from one task line to another reads no note again:
    /// the line left out is subtracted from what was read.
    #[test]
    fn a_caret_move_reads_nothing_again() {
        let mut owners = Owners::default();
        let note = sources(&[
            (1, "- [ ] Ask @sam\n- [ ] Call @priya\n- [ ] Plain"),
            (2, "- [ ] @zoe"),
        ]);
        owners.update(1, 1, &lend(&note), Some((NoteId(1), 0)), now());
        assert_eq!(names(owners.ranked()), ["priya", "zoe"]);
        let reads = owners.reads;
        let generation = owners.generation();
        assert!(owners.update(1, 1, &lend(&note), Some((NoteId(1), 1)), now()));
        assert_eq!(names(owners.ranked()), ["sam", "zoe"]);
        assert!(owners.generation() > generation);
        // Leaving out a line that names no one leaves out no one.
        assert!(owners.update(1, 1, &lend(&note), Some((NoteId(1), 2)), now()));
        assert_eq!(names(owners.ranked()), ["priya", "sam", "zoe"]);
        let generation = owners.generation();
        assert!(!owners.update(1, 1, &lend(&note), None, now()));
        assert_eq!(owners.generation(), generation);
        assert_eq!(owners.reads, reads, "no note was read again");
    }

    /// Typing in the note whose line is left out reads it again from the
    /// first changed line, in the task index, and finds what a fresh read
    /// finds.
    #[test]
    fn typing_reads_the_note_from_the_first_changed_line() {
        let block =
            "# Week\n- [ ] Ask @sam\n  more\n```\n- [ ] fenced @ghost\n```\n- [x] @Priya done\n\n";
        let mut text = block.repeat(2_500);
        text.push_str("- [ ] Last @");
        let last = text.split('\n').count() - 1;
        assert!(last >= 20_000);
        let mut index = index();
        let mut owners = Owners::default();
        let skip = Some((NoteId(1), last));
        draft(&mut index, &text);
        owners.update(1, 0, &tasks(&index).owner_sources(), skip, now());
        assert_eq!(names(owners.ranked()), ["Priya", "sam"]);
        for (round, typed) in ["z", "zo", "zoe", "zoe more"].iter().enumerate() {
            let before = tasks(&index).lines_read();
            draft(&mut index, &format!("{text}{typed}"));
            owners.update(
                1,
                round as u64 + 1,
                &tasks(&index).owner_sources(),
                skip,
                now(),
            );
            let read = tasks(&index).lines_read() - before;
            assert!(read <= wren_markdown::checkpoints::EVERY as u64, "{read}");
            assert_eq!(names(owners.ranked()), ["Priya", "sam"]);
        }
        // The caret leaves the line: what it typed counts.
        let done = format!("{text}zoe more");
        owners.update(
            1,
            9,
            &tasks(&index).owner_sources(),
            Some((NoteId(1), 0)),
            now(),
        );
        let mut found = names(owners.ranked());
        found.sort_unstable();
        assert_eq!(found, ["Priya", "sam", "zoe"]);
        // A fence opened near the top hides what follows, as a fresh read.
        let fenced = format!("```\n{done}");
        draft(&mut index, &fenced);
        owners.update(
            1,
            10,
            &tasks(&index).owner_sources(),
            Some((NoteId(1), 1)),
            now(),
        );
        assert_eq!(
            *owners.ranked(),
            Arc::new(scan([(now(), fenced.as_str())], now()))
        );
        // The note left: the index gives back what it kept of the draft.
        assert!(tasks(&index).holds_draft());
        index.set_content(NoteId(1), &fenced, false);
        assert!(!tasks(&index).holds_draft());
    }

    /// Read again from the first changed line, a note typed in names the
    /// owners a fresh read names, whatever the edit: line endings changed
    /// or merged, fences and front matter opened, lines added and taken.
    #[test]
    fn typing_finds_what_a_fresh_read_finds() {
        let base = "---\ntitle: x\n---\n- [ ] a @sam\r\n- [ ] b @priya\r- [ ] c @zoe\n```\n- [ ] d @code\n```\n- [ ] e @end";
        let edits: Vec<String> = vec![
            base.replace("\r\n", "\n"),
            base.replace("\r- [ ] c", "\r\n- [ ] c"),
            base.replace("title: x\n---\n", "title: x\n"),
            base.replace("```\n- [ ] d", "- [ ] d"),
            format!("{base} @more\n- [ ] f @new"),
            base[..base.len() / 2].to_owned(),
            format!("```\n{base}"),
            base.replace("@zoe", "@zed"),
            String::new(),
            base.to_owned(),
        ];
        let mut index = index();
        let mut owners = Owners::default();
        draft(&mut index, base);
        owners.update(
            1,
            0,
            &tasks(&index).owner_sources(),
            Some((NoteId(1), 1)),
            now(),
        );
        for (round, edited) in edits.iter().enumerate() {
            for skip in [1, 4] {
                draft(&mut index, edited);
                owners.update(
                    1,
                    round as u64 * 2 + skip as u64,
                    &tasks(&index).owner_sources(),
                    Some((NoteId(1), skip)),
                    now(),
                );
                let fresh = source(NoteId(1), now(), edited);
                if let Some(read) = owners.notes.get(&NoteId(1)) {
                    assert_eq!(read.names, fresh.names, "{round} {edited:?}");
                } else {
                    assert!(fresh.names.is_empty(), "{round} {edited:?}");
                }
                assert_eq!(
                    names(owners.ranked()),
                    names(&{
                        let mut again = Owners::default();
                        again.update(1, 0, &lend(&[fresh]), Some((NoteId(1), skip)), now());
                        again.ranked.to_vec()
                    }),
                    "{round} {skip}"
                );
            }
        }
    }

    /// A note whose only names are on the line left out comes among the
    /// sources and goes again without the owners changing, later on the
    /// clock too: the rows offered while a name is typed stay as they are.
    #[test]
    fn a_note_naming_no_one_that_counts_changes_nothing() {
        let mut owners = Owners::default();
        let other = sources(&[(2, "- [ ] @sam")]);
        assert!(owners.update(1, 1, &lend(&other), Some((NoteId(1), 0)), now()));
        let generation = owners.generation();
        let later = now() + DAY;
        for (revision, typed) in ["- [ ] Ask @z", "- [ ] Ask @ze", "- [ ] Ask @zed"]
            .iter()
            .enumerate()
        {
            let mut with = other.clone();
            with.push(source(NoteId(1), later, typed));
            assert!(!owners.update(
                1,
                revision as u64 + 2,
                &lend(&with),
                Some((NoteId(1), 0)),
                later
            ));
        }
        assert!(!owners.update(1, 9, &lend(&other), Some((NoteId(1), 0)), later));
        assert_eq!(owners.generation(), generation);
        // Off that line, the name counts; and gone again, it goes.
        let mut with = other.clone();
        with.push(source(NoteId(1), later, "- [ ] Ask @zed"));
        assert!(owners.update(1, 10, &lend(&with), None, later));
        assert_eq!(names(owners.ranked()), ["zed", "sam"]);
        assert!(owners.update(1, 11, &lend(&other), None, later));
        assert_eq!(names(owners.ranked()), ["sam"]);
    }

    /// Another index with no notes at all ranks no one, at once.
    #[test]
    fn another_index_with_no_notes_ranks_no_one() {
        let mut owners = Owners::default();
        assert!(owners.update(1, 0, &lend(&sources(&[(1, "- [ ] @alice")])), None, now()));
        assert_eq!(names(owners.ranked()), ["alice"]);
        let generation = owners.generation();
        assert!(owners.update(2, 0, &lend(&Vec::new()), None, now()));
        assert!(owners.ranked().is_empty());
        assert!(owners.generation() > generation);
    }

    /// The perf-search workload: 5,000 notes, 25 MB, with a task line
    /// naming an owner in every note, as the task index holds it: the
    /// owners' first read of every note's records, and then the read
    /// after one note changed, as typing changes it. What reading the
    /// notes themselves costs is the task index's (`make perf-tasks`).
    /// Run by `make perf-search`.
    #[test]
    #[ignore = "release performance benchmark: 5000 notes / 25 MB; run explicitly with --release"]
    fn perf_owner_scan_5000_notes_25mb() {
        let people = ["sam", "priya.k", "alice.chen", "Bob", "j-smith", "Sam"];
        let mut bytes = 0;
        let notes: Vec<Source> = (0..5000_u64)
            .map(|number| {
                let mut text =
                    "Ordinary work notes, migration planning, follow-up items. ".repeat(100);
                text.truncate(4900);
                let person = people[usize::try_from(number).unwrap() % people.len()];
                text.push_str(&format!("\n- [ ] Follow up with @{person} 📅 2026-12-01\n"));
                text.truncate(5000);
                bytes += text.len();
                source(NoteId(number), ago(number % 90), &text)
            })
            .collect();
        let (mut full, mut typed) = (Vec::new(), Vec::new());
        for round in 0..20_u64 {
            let mut owners = Owners::default();
            let started = Instant::now();
            owners.update(1, 0, &lend(&notes), None, now());
            full.push(started.elapsed());
            assert_eq!(owners.ranked().len(), 5);
            // One note typed in.
            let mut edited = notes.clone();
            let line = format!("- [ ] Follow up with @sam @x{round}");
            edited[0] = source(NoteId(0), ago(0), &line);
            let started = Instant::now();
            owners.update(
                1,
                1,
                &lend(&std::hint::black_box(edited)),
                Some((NoteId(0), 9)),
                now(),
            );
            typed.push(started.elapsed());
        }
        full.sort();
        typed.sort();
        eprintln!(
            "owner scan: notes: 5000; bytes: {bytes}; full median: {:?}, max: {:?}; \
             one note changed median: {:?}, max: {:?}",
            full[10], full[19], typed[10], typed[19]
        );
        assert!(full[10] < Duration::from_millis(250));
        assert!(typed[10] < Duration::from_millis(50));
    }
}
