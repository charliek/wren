//! The owners auto-suggest offers after an `@` (discovery/tasks.md §3.1
//! Q8, option B): every `@name` on a task line of the Markdown notes the
//! search index holds ([`crate::index::Index::markdown_sources`]).
//!
//! - Only task lines count, as the whole note reads them: not a line in a
//!   fenced block, nor one in YAML front matter, so a stray `@` in pasted
//!   mail text adds no one.
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
//! whose source changed, so it follows the index on the UI thread within
//! a small budget (`perf_owner_*`): typing in one note reads that note.
//! The line being typed on, in the note being typed in, is left out: the
//! name half typed there is no one's yet.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::SystemTime;

use wren_markdown::block::{front_matter_end, Boxes, Scanner};
use wren_markdown::task;

use crate::model::NoteId;

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

/// The `@name`s on the task lines of `source`, as `(key, spelling)`, one
/// for each time it is written, skipping line `skip`.
fn names_in(source: &str, skip: Option<usize>) -> Vec<(String, String)> {
    if !source.contains('@') {
        return Vec::new();
    }
    let lines: Vec<&str> = source.lines().collect();
    let mut scanner = Scanner::with_front_matter(front_matter_end(&lines[..]), Boxes::Status);
    let mut names = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let info = scanner.next_line(line);
        if Some(index) == skip || !line.contains('@') {
            continue;
        }
        let parsed = info
            .item
            .and_then(|item| item.task)
            .and_then(|found| task::parse(line, found));
        if let Some(parsed) = parsed {
            names.extend(
                parsed
                    .owner_names()
                    .map(|name| (task::fold_name(name), name.to_owned())),
            );
        }
    }
    names
}

/// How much a note changed at `modified` counts at `now`.
fn weight(modified: SystemTime, now: SystemTime) -> f64 {
    let age = now.duration_since(modified).unwrap_or_default();
    0.5_f64.powf(age.as_secs_f64() / 86_400.0 / HALF_LIFE_DAYS)
}

/// A line of a note left out: the one being typed on.
pub type Skip = (NoteId, usize);

/// What one note was read as.
struct Read {
    source: Arc<str>,
    modified: SystemTime,
    skip: Option<usize>,
    names: Vec<(String, String)>,
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

    /// Bring it up to the index `identity` at `revision`, whose Markdown
    /// notes are `sources`, leaving out line `skip` of a note, scored at
    /// `now`. Another index than the one before starts afresh. Only a
    /// note whose source, change time or left-out line differs from the
    /// last read is read again. Returns whether the owners changed.
    pub fn update(
        &mut self,
        identity: u64,
        revision: u64,
        sources: Vec<(NoteId, SystemTime, Arc<str>)>,
        skip: Option<(NoteId, usize)>,
        now: SystemTime,
    ) -> bool {
        if self.read.is_some_and(|(known, _, _)| known != identity) {
            self.notes.clear();
        }
        self.read = Some((identity, revision, skip));
        let mut changed = self.notes.len() != sources.len();
        let mut kept: HashMap<NoteId, Read> = HashMap::with_capacity(sources.len());
        for (note, modified, source) in sources {
            let skipped = skip.and_then(|(at, line)| (at == note).then_some(line));
            let read = match self.notes.remove(&note) {
                Some(read)
                    if Arc::ptr_eq(&read.source, &source)
                        && read.modified == modified
                        && read.skip == skipped =>
                {
                    read
                }
                old => {
                    let names = names_in(&source, skipped);
                    changed |= old.is_none_or(|old| old.names != names || old.modified != modified);
                    Read {
                        source,
                        modified,
                        skip: skipped,
                        names,
                    }
                }
            };
            kept.insert(note, read);
        }
        changed |= !self.notes.is_empty();
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
        for (key, spelling) in &read.names {
            let tally = tallies.entry(key).or_default();
            tally.notes.insert(*note, weight);
            *tally.spellings.entry(spelling.clone()).or_default() += 1;
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

/// The owners named on the task lines of `notes` (each note's last change
/// and its source), best first, scored at `now`: [`Owners`] read once.
pub fn scan<'a>(
    notes: impl IntoIterator<Item = (SystemTime, &'a str)>,
    now: SystemTime,
) -> Vec<Owner> {
    let sources = (0..)
        .zip(notes)
        .map(|(id, (modified, text))| (NoteId(id), modified, Arc::from(text)))
        .collect();
    let mut owners = Owners::default();
    owners.update(0, 0, sources, None, now);
    owners.ranked.to_vec()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

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

    fn sources(notes: &[(u64, &str)]) -> Vec<(NoteId, SystemTime, Arc<str>)> {
        notes
            .iter()
            .map(|(id, text)| (NoteId(*id), now(), Arc::from(*text)))
            .collect()
    }

    #[test]
    fn a_rename_or_a_removal_is_read_again_and_only_changed_notes_are() {
        let mut owners = Owners::default();
        let first = sources(&[(1, "- [ ] @sam\n- [ ] @priya"), (2, "- [ ] @sam")]);
        assert!(owners.update(7, 1, first.clone(), None, now()));
        assert_eq!(names(owners.ranked()), ["sam", "priya"]);
        let generation = owners.generation();
        assert!(owners.is_current(7, 1, None));
        // The same sources again: nothing read, nothing changed.
        assert!(!owners.update(7, 2, first.clone(), None, now()));
        assert_eq!(owners.generation(), generation);
        assert!(owners.is_current(7, 2, None) && !owners.is_current(7, 1, None));
        // `@sam` renamed in one note, `@priya` removed.
        let second = vec![
            (
                NoteId(1),
                now(),
                Arc::from("- [ ] @samuel\n- [ ] Mail priya"),
            ),
            first[1].clone(),
        ];
        assert!(owners.update(7, 3, second, None, now()));
        assert_eq!(names(owners.ranked()), ["sam", "samuel"]);
        assert!(owners.generation() > generation);
        // A note gone.
        assert!(owners.update(7, 4, sources(&[(1, "- [ ] @samuel")]), None, now()));
        assert_eq!(names(owners.ranked()), ["samuel"]);
        // Another index starts afresh, and `clear` forgets all.
        assert!(owners.update(8, 0, sources(&[(1, "- [ ] @zoe")]), None, now()));
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
        owners.update(1, 1, note.clone(), Some((NoteId(1), 1)), now());
        assert_eq!(names(owners.ranked()), ["sam"]);
        // Another note's line of that number counts.
        owners.update(1, 1, note.clone(), Some((NoteId(2), 1)), now());
        assert_eq!(names(owners.ranked()), ["sa", "sam"]);
        owners.update(1, 1, note, Some((NoteId(1), 0)), now());
        assert_eq!(names(owners.ranked()), ["sa"]);
    }

    /// The perf-search workload: 5,000 notes, 25 MB, with a task line naming an owner in every note, so no note is
    /// skipped for having no `@`: the first read of all of it, and then
    /// the read after one note changed, as typing changes it. Run by
    /// `make perf-search`.
    #[test]
    #[ignore = "release performance benchmark: 5000 notes / 25 MB; run explicitly with --release"]
    fn perf_owner_scan_5000_notes_25mb() {
        let people = ["sam", "priya.k", "alice.chen", "Bob", "j-smith", "Sam"];
        let notes: Vec<(NoteId, SystemTime, Arc<str>)> = (0..5000_u64)
            .map(|number| {
                let mut text =
                    "Ordinary work notes, migration planning, follow-up items. ".repeat(100);
                text.truncate(4900);
                let person = people[usize::try_from(number).unwrap() % people.len()];
                text.push_str(&format!("\n- [ ] Follow up with @{person} 📅 2026-12-01\n"));
                text.truncate(5000);
                (NoteId(number), ago(number % 90), Arc::from(text))
            })
            .collect();
        let bytes: usize = notes.iter().map(|(_, _, text)| text.len()).sum();
        let (mut full, mut typed) = (Vec::new(), Vec::new());
        for round in 0..20_u64 {
            let mut owners = Owners::default();
            let started = Instant::now();
            owners.update(1, 0, notes.clone(), None, now());
            full.push(started.elapsed());
            assert_eq!(owners.ranked().len(), 5);
            // One note typed in.
            let mut edited = notes.clone();
            let text = format!("{} @x{round}", edited[0].2);
            edited[0].2 = Arc::from(text);
            let started = Instant::now();
            owners.update(
                1,
                1,
                std::hint::black_box(edited),
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
