//! Session-owned, in-memory search. Queries never access the filesystem.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;
use std::time::SystemTime;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32Str};
use wren_markdown::block::{self, Boxes, Kind, LineInfo, Scanner};

use crate::fs_store::{DiskFile, FsStore, RelPath};
use crate::kind::NoteKind;
use crate::model::{GroupId, Note, NoteId, SectionId, Workspace};

pub const MAX_CONTENT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_QUERY_BYTES: usize = 4096;
pub const MAX_QUERY_WORDS: usize = 128;
pub(crate) const NOT_UTF8: &str = "not valid UTF-8; content search unavailable";

/// Check before allocating a fuzzy pattern or lowercase copy. UI callers should
/// display this error, rather than presenting an oversized query as no matches.
pub fn validate_query(query: &str) -> Result<(), &'static str> {
    if query.len() > MAX_QUERY_BYTES {
        return Err("Search queries are limited to 4,096 UTF-8 bytes");
    }
    if query.split_whitespace().take(MAX_QUERY_WORDS + 1).count() > MAX_QUERY_WORDS {
        return Err("Search queries are limited to 128 words");
    }
    Ok(())
}
const PREVIEW_CHARS: usize = 160;
const CONTEXT_CHARS: usize = 48;
/// Joins blocks (list items, tasks, quotes, headings, paragraphs) that a
/// preview or snippet puts on one line.
const BLOCK_SEPARATOR: &str = " · ";
const MAX_QUEUED_WARNINGS: usize = 64;

/// The next [`Index::identity`].
static NEXT_IDENTITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Workspace,
    /// Every group in the section.
    Section(SectionId),
    Group(GroupId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryKind {
    Jump,
    Search,
    Filter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub note: NoteId,
    pub title: String,
    pub path: RelPath,
    pub snippet: Snippet,
    /// Byte range in the original indexed content, not its lowercase copy.
    pub content_match: Option<Range<usize>>,
    pub score: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// UTF-8 byte range within `text`.
    pub highlight: Option<Range<usize>>,
}

#[derive(Debug)]
struct Content {
    /// The note's text as read or drafted. Shared with background
    /// reconciliation snapshots, which compare exact bytes, so a change is
    /// detected without extracting anything.
    source: Arc<str>,
    /// `None` while a board read from disk waits for its labels to be taken
    /// off the owner's thread: parsing every board as a workspace opens
    /// would hold up the open (the board plan's §3.6).
    searched: Option<Searched>,
}

#[derive(Debug)]
struct Searched {
    /// What search reads: the same allocation as the source when nothing is
    /// extracted from it.
    text: Arc<str>,
    folded: Box<str>,
    preview: Preview,
}

impl Content {
    fn bytes(&self) -> usize {
        self.source.len()
            + self.searched.as_ref().map_or(0, |searched| {
                let extracted = if Arc::ptr_eq(&self.source, &searched.text) {
                    0
                } else {
                    searched.text.len()
                };
                extracted + searched.folded.len()
            })
    }
}

#[derive(Debug)]
struct Entry {
    title: String,
    kind: NoteKind,
    path: RelPath,
    group: GroupId,
    section: Option<SectionId>,
    modified: SystemTime,
    opened: u64,
    content: Option<Content>,
    limitation: Option<String>,
}

#[derive(Debug)]
pub struct Index {
    entries: HashMap<NoteId, Entry>,
    order: Vec<NoteId>,
    retained_bytes: usize,
    budget: usize,
    sequence: u64,
    revision: u64,
    /// A number no other index of this run has: with [`Index::revision`],
    /// what names one state of one workspace's notes.
    identity: u64,
    warnings: Vec<String>,
    /// The newest source of a note whose extraction waits for
    /// [`Index::settle`]: the open board while its labels are typed, which
    /// would otherwise be parsed whole on every keystroke (the board plan's
    /// §3.4 rule 7). Until then search reads the source before it.
    pending: Option<(NoteId, Arc<str>)>,
    /// The task boxes previews and snippets read, as the editor of the same
    /// run reads them: fixed by the Session's `tasks` option.
    boxes: Boxes,
    /// Board sources handed to extraction, so tests can prove that unchanged
    /// boards are never extracted again.
    #[cfg(test)]
    pub(crate) extractions: usize,
}

impl Default for Index {
    fn default() -> Self {
        Self::with_boxes(Boxes::Plain)
    }
}

impl Index {
    /// An empty index whose previews and snippets read the task boxes
    /// `boxes` names.
    pub(crate) fn with_boxes(boxes: Boxes) -> Self {
        Self {
            boxes,
            ..Self::with_budget(MAX_CONTENT_BYTES)
        }
    }

    fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
            retained_bytes: 0,
            budget,
            sequence: 0,
            revision: 0,
            identity: NEXT_IDENTITY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            warnings: Vec::new(),
            pending: None,
            boxes: Boxes::Plain,
            #[cfg(test)]
            extractions: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Changes for content, paths, ordering, limitations, and opened recency.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// A number no other index of this run has, so a workspace opened
    /// again, whose revisions count from the start, is never taken for the
    /// one before it.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// Each Markdown note's source as the index holds it, with the time the
    /// note was last changed on disk: what the owners of tasks are read
    /// from (`crate::owners`). Boards, and notes whose content the index
    /// dropped, are left out. The sources are shared, not copied.
    pub fn markdown_sources(&self) -> Vec<(NoteId, SystemTime, Arc<str>)> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.kind == NoteKind::Markdown)
            .filter_map(|(note, entry)| {
                let content = entry.content.as_ref()?;
                Some((*note, entry.modified, Arc::clone(&content.source)))
            })
            .collect()
    }

    /// Retained source, extracted and lowercase content bytes, excluding title
    /// metadata.
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub fn preview(&self, note: NoteId) -> Option<&str> {
        Some(&self.searched(note)?.preview.text)
    }

    /// The text search reads: a Markdown note's source, or what was
    /// extracted from a board's.
    pub fn content(&self, note: NoteId) -> Option<&str> {
        Some(&self.searched(note)?.text)
    }

    /// How many boards wait for their labels to be extracted.
    pub fn unextracted(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| {
                entry
                    .content
                    .as_ref()
                    .is_some_and(|content| content.searched.is_none())
            })
            .count()
    }

    fn searched(&self, note: NoteId) -> Option<&Searched> {
        self.entries.get(&note)?.content.as_ref()?.searched.as_ref()
    }

    pub fn limitation(&self, note: NoteId) -> Option<&str> {
        self.entries.get(&note)?.limitation.as_deref()
    }

    /// Queries are trimmed. Blank Jump lists notes by opened recency; blank
    /// Filter preserves the model's per-parent sort; blank Search returns none.
    /// Nonblank title matches rank ahead of content-only matches. Content is a
    /// literal substring of char-wise Unicode-lowercased text (including
    /// expansions), without contextual casing, normalization, or full casefolding.
    pub fn query(&self, query: &str, scope: Scope, kind: QueryKind, limit: usize) -> Vec<Hit> {
        if validate_query(query).is_err() || limit == 0 {
            return Vec::new();
        }
        let query = query.trim();
        let in_scope = |entry: &Entry| match scope {
            Scope::Workspace => true,
            Scope::Section(section) => entry.section == Some(section),
            Scope::Group(group) => entry.group == group,
        };
        if (query.is_empty() && kind == QueryKind::Search) || !self.entries.values().any(in_scope) {
            return Vec::new();
        }
        let pattern = fuzzy_pattern(query);
        let folded_query = lowercase(query);
        let mut matcher = Matcher::default();
        let mut buffer = Vec::new();
        let mut candidates = Vec::new();
        for (position, note) in self.order.iter().copied().enumerate() {
            let entry = &self.entries[&note];
            if !in_scope(entry) {
                continue;
            }
            let title_score = if query.is_empty() {
                Some(0)
            } else {
                pattern.score(Utf32Str::new(&entry.title, &mut buffer), &mut matcher)
            };
            let content_start = if kind != QueryKind::Jump && !query.is_empty() {
                entry
                    .content
                    .as_ref()
                    .and_then(|content| content.searched.as_ref())
                    .and_then(|searched| searched.folded.find(&folded_query))
            } else {
                None
            };
            if title_score.is_none() && content_start.is_none() {
                continue;
            }
            let score = title_score.unwrap_or(0);
            let recency = if kind == QueryKind::Jump {
                entry.opened
            } else {
                0
            };
            // For a cleared filter the caller expects the same order as the
            // notes column, not search relevance or an arbitrary hash order.
            let rank = (title_score.is_some(), score, recency, Reverse(position));
            candidates.push((rank, note, content_start));
        }
        let count = candidates.len().min(limit);
        if count < candidates.len() {
            candidates.select_nth_unstable_by(count, |left, right| right.0.cmp(&left.0));
            candidates.truncate(count);
        }
        candidates.sort_unstable_by_key(|candidate| Reverse(candidate.0));
        candidates
            .into_iter()
            .map(|(rank, note, start)| {
                let entry = &self.entries[&note];
                let searched = entry
                    .content
                    .as_ref()
                    .and_then(|content| content.searched.as_ref());
                let content_match = start.and_then(|start| {
                    Some(original_range(
                        &searched?.text,
                        start..start + folded_query.len(),
                    ))
                });
                let snippet = match searched {
                    Some(searched) => match &content_match {
                        Some(hit) => snippet(&searched.text, hit.clone(), entry.kind, self.boxes),
                        None => searched.preview.snippet(),
                    },
                    None => Snippet {
                        text: String::new(),
                        highlight: None,
                    },
                };
                Hit {
                    note,
                    title: entry.title.clone(),
                    path: entry.path.clone(),
                    snippet,
                    content_match,
                    score: rank.1,
                }
            })
            .collect()
    }

    pub(crate) fn sync_metadata(&mut self, workspace: &Workspace) -> Vec<NoteId> {
        let sections: HashMap<_, _> = workspace
            .groups
            .iter()
            .map(|group| (group.id, group.section))
            .collect();
        let ids: HashSet<_> = workspace.notes.iter().map(|note| note.id).collect();
        let old_len = self.entries.len();
        if self
            .pending
            .as_ref()
            .is_some_and(|(note, _)| !ids.contains(note))
        {
            self.pending = None;
        }
        self.entries.retain(|id, entry| {
            if ids.contains(id) {
                true
            } else {
                self.retained_bytes -= entry.content.as_ref().map_or(0, Content::bytes);
                false
            }
        });
        if self.entries.len() != old_len {
            self.changed();
        }
        let mut added = Vec::new();
        for note in &workspace.notes {
            let section = sections.get(&note.group).copied();
            if let Some(entry) = self.entries.get_mut(&note.id) {
                if entry.kind != note.kind {
                    // The searched text was derived for the old kind.
                    entry.kind = note.kind;
                    self.drop_content(note.id);
                }
                let entry = self.entries.get_mut(&note.id).expect("known index entry");
                if entry.title != note.title
                    || entry.path != note.path
                    || entry.group != note.group
                    || entry.section != section
                    || entry.modified != note.modified
                {
                    entry.title.clone_from(&note.title);
                    entry.path.clone_from(&note.path);
                    entry.group = note.group;
                    entry.section = section;
                    entry.modified = note.modified;
                    self.changed();
                }
            } else {
                self.entries.insert(
                    note.id,
                    Entry {
                        title: note.title.clone(),
                        kind: note.kind,
                        path: note.path.clone(),
                        group: note.group,
                        section,
                        modified: note.modified,
                        opened: 0,
                        content: None,
                        limitation: None,
                    },
                );
                added.push(note.id);
                self.changed();
            }
        }
        if self
            .order
            .iter()
            .copied()
            .ne(workspace.notes.iter().map(|note| note.id))
        {
            self.order = workspace.notes.iter().map(|note| note.id).collect();
            self.changed();
        }
        added
    }

    /// The scanner supplies one descriptor-validated file at a time. Only notes
    /// accepted by that scan may be ingested; final metadata synchronization
    /// supplies group scopes/order and removes vanished entries.
    pub(crate) fn ingest(&mut self, note: &Note, contents: Result<&DiskFile, &str>) {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.entries.entry(note.id) {
            entry.insert(Entry {
                title: note.title.clone(),
                kind: note.kind,
                path: note.path.clone(),
                group: note.group,
                section: None,
                modified: note.modified,
                opened: 0,
                content: None,
                limitation: None,
            });
            self.changed();
        }
        match contents {
            Ok(file) => match std::str::from_utf8(&file.bytes) {
                Ok(text) => self.hold_content(note.id, text),
                Err(_) => self.unavailable(note.id, NOT_UTF8),
            },
            Err(reason) => self.unavailable(note.id, reason),
        }
    }

    /// Exact retained source per note, shared rather than copied, so a
    /// background scan can send back only notes whose disk bytes differ.
    pub(crate) fn content_snapshot(&self) -> HashMap<NoteId, Arc<str>> {
        let mut snapshot: HashMap<NoteId, Arc<str>> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| Some((*id, Arc::clone(&entry.content.as_ref()?.source))))
            .collect();
        if let Some((note, source)) = &self.pending {
            snapshot.insert(*note, Arc::clone(source));
        }
        snapshot
    }

    /// Applies one background-scan content result to a note already present
    /// after metadata synchronization; unknown notes are ignored.
    pub(crate) fn ingest_scanned(&mut self, note: NoteId, contents: Result<&str, &str>) {
        if !self.entries.contains_key(&note) {
            return;
        }
        match contents {
            Ok(text) => self.hold_content(note, text),
            Err(reason) => self.unavailable(note, reason),
        }
    }

    pub(crate) fn scan_failed(&mut self, active: Option<NoteId>) {
        for note in self.order.clone() {
            if Some(note) != active {
                self.unavailable(
                    note,
                    "scan failed; content search unavailable until reconciliation succeeds",
                );
            }
        }
    }

    pub(crate) fn read_note(&mut self, store: &FsStore, note: NoteId) {
        let Some(entry) = self.entries.get(&note) else {
            return;
        };
        match store.read(&entry.path) {
            Ok(file) => match std::str::from_utf8(&file.bytes) {
                Ok(text) => self.set_content(note, text, false),
                Err(_) => self.unavailable(note, NOT_UTF8),
            },
            Err(error) => self.unavailable(note, &format!("content search unavailable: {error}")),
        }
    }

    /// Takes a note's newest source without extracting it yet: search goes
    /// on reading the one before until [`Index::settle`] or another source
    /// for any note arrives. Only a source that differs from the one kept
    /// waits.
    pub(crate) fn defer_content(&mut self, note: NoteId, source: &str) {
        let Some(entry) = self.entries.get(&note) else {
            return;
        };
        let kept = entry
            .content
            .as_ref()
            .is_some_and(|content| &*content.source == source);
        if self.pending.as_ref().is_some_and(|(of, _)| *of != note) {
            self.settle();
        }
        self.pending = (!kept).then(|| (note, source.into()));
    }

    /// Takes the open document's source as [`Index::set_content`] does,
    /// unless a source of the note already waits for [`Index::settle`]:
    /// then this one waits in its place.
    pub(crate) fn keep_content(&mut self, note: NoteId, source: &str) {
        if self.pending.as_ref().is_some_and(|(of, _)| *of == note) {
            self.defer_content(note, source);
        } else {
            self.set_content(note, source, true);
        }
    }

    /// Whether a source waits for [`Index::settle`].
    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Extract the source that waits, as the open document's.
    pub(crate) fn settle(&mut self) {
        if let Some((note, source)) = self.pending.take() {
            self.set_content(note, &source, true);
        }
    }

    /// Takes a note's source. Only a source that differs from the one kept
    /// is extracted and folded again; a source of the note that waited is
    /// replaced by this one.
    pub(crate) fn set_content(&mut self, note: NoteId, source: &str, priority: bool) {
        self.put(note, source, priority, false);
    }

    /// Takes a source read from disk as [`Index::set_content`] does, except
    /// that a board's labels wait for [`Index::extracted`] or
    /// [`Index::extract_waiting`]; until then search finds the board by
    /// its title only.
    fn hold_content(&mut self, note: NoteId, source: &str) {
        self.put(note, source, false, true);
    }

    /// The boards whose labels wait, with the source each waits on.
    pub(crate) fn waiting(&self) -> Vec<(NoteId, Arc<str>)> {
        self.entries
            .iter()
            .filter_map(|(note, entry)| {
                let content = entry.content.as_ref()?;
                content
                    .searched
                    .is_none()
                    .then(|| (*note, Arc::clone(&content.source)))
            })
            .collect()
    }

    /// Takes the labels extracted elsewhere from `source`, if the board's
    /// labels still wait on that source.
    pub(crate) fn extracted(&mut self, note: NoteId, source: &str, text: Option<String>) {
        let waits = self.entries.get(&note).is_some_and(|entry| {
            entry.kind == NoteKind::Board
                && entry
                    .content
                    .as_ref()
                    .is_some_and(|content| content.searched.is_none() && &*content.source == source)
        });
        if waits {
            let content = self
                .entries
                .get_mut(&note)
                .and_then(|entry| entry.content.take());
            if let Some(content) = content {
                self.retained_bytes -= content.bytes();
                self.install(note, content.source, Some(text), false);
            }
        }
    }

    /// Extracts every board's labels that wait, on the caller's thread.
    pub(crate) fn extract_waiting(&mut self) {
        for (note, source) in self.waiting() {
            #[cfg(test)]
            {
                self.extractions += 1;
            }
            let text = searchable(NoteKind::Board, &source);
            self.extracted(note, &source, text);
        }
    }

    fn put(&mut self, note: NoteId, source: &str, priority: bool, hold: bool) {
        if self.pending.as_ref().is_some_and(|(of, _)| *of == note) {
            self.pending = None;
        }
        let Some(entry) = self.entries.get(&note) else {
            return;
        };
        if entry
            .content
            .as_ref()
            .is_some_and(|content| &*content.source == source)
        {
            return;
        }
        let kind = entry.kind;
        self.drop_content(note);
        let source: Arc<str> = source.into();
        let searched = if hold && kind == NoteKind::Board {
            None
        } else {
            #[cfg(test)]
            if kind == NoteKind::Board {
                self.extractions += 1;
            }
            Some(searchable(kind, &source))
        };
        self.install(note, source, searched, priority);
    }

    /// Keeps `source` with what search reads from it: `None` waits for a
    /// board's labels, `Some(None)` searches the source itself.
    fn install(
        &mut self,
        note: NoteId,
        source: Arc<str>,
        searched: Option<Option<String>>,
        priority: bool,
    ) {
        let text = searched.map(|text| text.map_or_else(|| Arc::clone(&source), Arc::from));
        let extracted = match &text {
            Some(text) if !Arc::ptr_eq(&source, text) => text.len(),
            _ => 0,
        };
        let folded_bytes: usize = match &text {
            None => 0,
            Some(text) if text.is_ascii() => text.len(),
            Some(text) => text
                .chars()
                .flat_map(char::to_lowercase)
                .map(char::len_utf8)
                .sum(),
        };
        let bytes = source
            .len()
            .saturating_add(extracted)
            .saturating_add(folded_bytes);
        if bytes > self.budget {
            self.set_limitation(note, Some(self.budget_message()));
            return;
        }
        if priority && self.retained_bytes + bytes > self.budget {
            // The live draft takes precedence over cached inactive content.
            for candidate in self.order.clone().into_iter().rev() {
                if candidate != note && self.entries[&candidate].content.is_some() {
                    self.drop_content(candidate);
                    self.set_limitation(candidate, Some(self.budget_message()));
                    if self.retained_bytes + bytes <= self.budget {
                        break;
                    }
                }
            }
        }
        if self.retained_bytes + bytes > self.budget {
            self.set_limitation(note, Some(self.budget_message()));
            return;
        }
        let kind = self.entries[&note].kind;
        let boxes = self.boxes;
        let searched = text.map(|text| {
            let folded = if text.is_ascii() {
                text.to_ascii_lowercase()
            } else {
                // Size before allocating: lowercase expansions must not cause an
                // oversized staging buffer or allocate for content we cannot keep.
                let mut folded = String::with_capacity(folded_bytes);
                folded.extend(text.chars().flat_map(char::to_lowercase));
                folded
            };
            Searched {
                preview: Preview::new(&text, kind, boxes),
                text,
                folded: folded.into_boxed_str(),
            }
        });
        let content = Content { source, searched };
        self.retained_bytes += content.bytes();
        self.entries
            .get_mut(&note)
            .expect("known index entry")
            .content = Some(content);
        self.set_limitation(note, None);
        self.changed();
    }

    pub(crate) fn opened(&mut self, note: NoteId) {
        if let Some(entry) = self.entries.get_mut(&note) {
            self.sequence = self.sequence.saturating_add(1);
            entry.opened = self.sequence;
            self.changed();
        }
    }

    pub(crate) fn forget_opened(&mut self, note: NoteId) {
        if let Some(entry) = self.entries.get_mut(&note) {
            if entry.opened != 0 {
                entry.opened = 0;
                self.changed();
            }
        }
    }

    pub(crate) fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    pub(crate) fn limitations(&self) -> impl Iterator<Item = (&RelPath, &str)> {
        self.entries.values().filter_map(|entry| {
            entry
                .limitation
                .as_deref()
                .map(|reason| (&entry.path, reason))
        })
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn drop_content(&mut self, note: NoteId) {
        if let Some(content) = self
            .entries
            .get_mut(&note)
            .and_then(|entry| entry.content.take())
        {
            self.retained_bytes -= content.bytes();
            self.changed();
        }
    }

    fn unavailable(&mut self, note: NoteId, reason: &str) {
        self.drop_content(note);
        self.set_limitation(note, Some(reason.into()));
    }

    fn budget_message(&self) -> String {
        format!(
            "Content search unavailable: the in-memory index limit of {} bytes (original plus lowercase text) was reached; title search remains available",
            self.budget
        )
    }

    fn set_limitation(&mut self, note: NoteId, limitation: Option<String>) {
        let entry = self.entries.get_mut(&note).expect("known index entry");
        if entry.limitation == limitation {
            return;
        }
        if let Some(reason) = &limitation {
            if self.warnings.len() < MAX_QUEUED_WARNINGS {
                self.warnings
                    .push(format!("{}: {reason}", entry.path.as_path().display()));
            }
        }
        entry.limitation = limitation;
        self.changed();
    }
}

/// Case-insensitive nucleo title matching with literal, whitespace-separated
/// fuzzy words (no special query operators). Blank queries match with score 0.
pub fn fuzzy_score(query: &str, text: &str) -> Option<u32> {
    validate_query(query).ok()?;
    let mut matcher = Matcher::default();
    let mut buffer = Vec::new();
    fuzzy_pattern(query.trim()).score(Utf32Str::new(text, &mut buffer), &mut matcher)
}

fn fuzzy_pattern(query: &str) -> Pattern {
    Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    )
}

/// The text search reads for a note of `kind`, or `None` to read the source
/// itself, which keeps a Markdown note's text one shared allocation, byte for
/// byte what was read. A board is searched by its labels, so its TOML keys
/// and ids never match; one that does not parse is searched as it is written.
pub(crate) fn searchable(kind: NoteKind, source: &str) -> Option<String> {
    match kind {
        NoteKind::Markdown => None,
        NoteKind::Board => wren_board::parse(source)
            .ok()
            .map(|board| wren_board::search_text(&board)),
    }
}

fn lowercase(text: &str) -> String {
    if text.is_ascii() {
        text.to_ascii_lowercase()
    } else {
        text.chars().flat_map(char::to_lowercase).collect()
    }
}

/// The first body line's words: the first paragraph line that is not
/// heading text, with its block markers (`>`, list markers, task boxes) and
/// surrounding whitespace dropped. Classified by `wren_markdown::block`, so
/// front matter, setext headings, and fences read as the editor reads them.
#[cfg(test)]
fn body_line(text: &str, boxes: Boxes) -> Range<usize> {
    let lines = wren_markdown::line_ranges(text).map(|(line, _)| line);
    body_line_in(text, lines, boxes).0
}

/// `text`'s lines with their classification, read one at a time. Front
/// matter is read only when line 0 opens it, and then only to the first
/// `---` or `...` line: `front_matter_end` decides by that line.
fn classified<'a>(
    text: &'a str,
    mut lines: impl Iterator<Item = Range<usize>> + 'a,
    boxes: Boxes,
) -> impl Iterator<Item = (Range<usize>, LineInfo)> + 'a {
    let delimiter = |line: &Range<usize>| text[line.clone()].trim_end_matches([' ', '\t']);
    let mut head: Vec<Range<usize>> = lines.next().into_iter().collect();
    let opens = head.first().is_some_and(|first| {
        let first = delimiter(first);
        first.strip_prefix('\u{feff}').unwrap_or(first) == "---"
    });
    if opens {
        for line in lines.by_ref() {
            let closes = matches!(delimiter(&line), "---" | "...");
            head.push(line);
            if closes {
                break;
            }
        }
    }
    let head_lines: Vec<&str> = head.iter().map(|line| &text[line.clone()]).collect();
    let front_matter_end = (!head_lines.is_empty())
        .then(|| block::front_matter_end(&head_lines[..]))
        .flatten();
    let mut scanner = Scanner::with_front_matter(front_matter_end, boxes);
    head.into_iter().chain(lines).map(move |range| {
        let info = scanner.next_line(&text[range.clone()]);
        (range, info)
    })
}

/// [`body_line`] over `text`'s lines, read lazily: it stops at the first
/// body line, so a long note costs only the lines before it. With the line
/// comes whether it is a task's description, the text after its box.
fn body_line_in<'a>(
    text: &'a str,
    lines: impl Iterator<Item = Range<usize>> + 'a,
    boxes: Boxes,
) -> (Range<usize>, bool) {
    let mut candidate: Option<(usize, (Range<usize>, bool))> = None;
    for (index, (range, info)) in classified(text, lines, boxes).enumerate() {
        match info.kind {
            Kind::Prose => {
                if candidate.is_none() {
                    let start = range.start + info.text_start;
                    let words = text[range][info.text_start..].trim_end();
                    let task = info.item.is_some_and(|item| item.task.is_some());
                    candidate = Some((index, (start..start + words.len(), task)));
                }
            }
            // Paragraph lines from `start` on were heading text.
            Kind::SetextUnderline { start, .. } => match candidate {
                Some((line, _)) if line >= start => candidate = None,
                Some((_, body)) => return body,
                None => {}
            },
            _ => {
                if let Some((_, body)) = candidate {
                    return body;
                }
            }
        }
    }
    candidate.map_or((0..0, false), |(_, body)| body)
}

/// A note's preview: the first body line, clipped to [`PREVIEW_CHARS`], as
/// plain words. Built once per content change, since the notes column asks
/// for every visible note's preview on each frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Preview {
    text: Box<str>,
    truncated: bool,
}

impl Preview {
    /// A board's searched text is plain, one label per line, so its preview
    /// is the first line with words rather than a Markdown body line.
    fn new(text: &str, kind: NoteKind, boxes: Boxes) -> Self {
        let (line, task) = match kind {
            NoteKind::Markdown => {
                let lines = wren_markdown::line_ranges(text).map(|(line, _)| line);
                body_line_in(text, lines, boxes)
            }
            NoteKind::Board => (first_line(text), false),
        };
        let end = text[line.clone()]
            .char_indices()
            .nth(PREVIEW_CHARS)
            .map_or(line.end, |(byte, _)| line.start + byte);
        let text = match kind {
            // With tasks on, a task's description is past every block marker
            // already: what looks like one in it (`- [/] - [x] words`) is its
            // text. Any other line is read as it was before tasks.
            NoteKind::Markdown => {
                let at_line_start = !(task && boxes == Boxes::Status);
                plain_markdown(&text[line.start..end], at_line_start).0
            }
            NoteKind::Board => text[line.start..end].to_owned(),
        };
        Self {
            text: text.into_boxed_str(),
            truncated: end < line.end,
        }
    }

    fn snippet(&self) -> Snippet {
        let mut text = String::from(&*self.text);
        if self.truncated {
            text.push('…');
        }
        Snippet {
            text,
            highlight: None,
        }
    }
}

fn original_range(text: &str, folded: Range<usize>) -> Range<usize> {
    if text.is_ascii() {
        return folded;
    }
    let mut offset = 0;
    let mut start = None;
    for (byte, character) in text.char_indices() {
        offset += character.to_lowercase().map(char::len_utf8).sum::<usize>();
        if start.is_none() && offset > folded.start {
            start = Some(byte);
        }
        if offset >= folded.end {
            return start.unwrap_or(byte)..byte + character.len_utf8();
        }
    }
    start.unwrap_or(text.len())..text.len()
}

/// The first line of `text` with words, trimmed.
fn first_line(text: &str) -> Range<usize> {
    wren_markdown::line_ranges(text)
        .map(|(line, _)| {
            let words = &text[line.clone()];
            let start = line.start + (words.len() - words.trim_start().len());
            start..start + words.trim().len()
        })
        .find(|line| !line.is_empty())
        .unwrap_or(0..0)
}

/// A search hit's context: the hit with up to [`CONTEXT_CHARS`] on each side,
/// started and ended on word boundaries and reduced to plain words.
fn snippet(text: &str, hit: Range<usize>, kind: NoteKind, boxes: Boxes) -> Snippet {
    let start = text[..hit.start]
        .char_indices()
        .rev()
        .nth(CONTEXT_CHARS - 1)
        .map_or(0, |(byte, _)| byte);
    let end = text[hit.end..]
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(text.len(), |(byte, _)| hit.end + byte);
    // Start and end on word boundaries so the context never opens or closes
    // on half a word or half a link.
    let start = text[start..hit.start]
        .char_indices()
        .find(|(_, character)| character.is_whitespace())
        .filter(|_| start != 0)
        .map_or(start, |(relative, space)| {
            start + relative + space.len_utf8()
        });
    let end = text[hit.end..end]
        .rfind(char::is_whitespace)
        .filter(|_| end != text.len())
        .map_or(end, |relative| hit.end + relative);
    let mut result = String::new();
    if start != 0 {
        result.push('…');
    }
    let prefix = result.len();
    let (plain, map) = match kind {
        NoteKind::Markdown => {
            let at_line_start = start == 0 || text[..start].ends_with(['\n', '\r']);
            let tasks = task_prefixes(text, start..end, boxes);
            plain_blocks(&text[start..end], at_line_start, &tasks)
        }
        NoteKind::Board => plain_lines(&text[start..end]),
    };
    result.push_str(&plain);
    if end != text.len() {
        result.push('…');
    }
    let highlight = Some(prefix + map[hit.start - start]..prefix + map[hit.end - start])
        .filter(|range| !range.is_empty());
    Snippet {
        text: result,
        highlight,
    }
}

/// Drop leading Markdown block markers (`>`, `-`/`*`/`+`, `1.`, `[ ]`/`[x]`)
/// so previews show the line's words.
fn strip_block_markers(mut line: &str) -> &str {
    loop {
        let trimmed = line.trim_start();
        let rest = if let Some(rest) = trimmed.strip_prefix('>') {
            rest
        } else if let Some(rest) = ["- ", "* ", "+ "]
            .iter()
            .find_map(|marker| trimmed.strip_prefix(marker))
        {
            rest
        } else if let Some(rest) = ["[ ] ", "[x] ", "[X] "]
            .iter()
            .find_map(|marker| trimmed.strip_prefix(marker))
        {
            rest
        } else {
            let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
            match trimmed[digits..]
                .strip_prefix(". ")
                .or_else(|| trimmed[digits..].strip_prefix(") "))
            {
                Some(rest) if digits > 0 => rest,
                _ => return trimmed,
            }
        };
        if rest.trim().is_empty() {
            return trimmed;
        }
        line = rest;
    }
}

/// Markdown source reduced to its words for previews and snippets: block
/// markers (headings, lists, tasks, quotes), emphasis and code delimiters,
/// and link destinations are dropped. A line break inside a paragraph
/// becomes a single space; a break between blocks (before a list item,
/// task, quote, or heading, after a heading, or across a blank or
/// marker-only line) becomes [`BLOCK_SEPARATOR`]. `at_line_start` says
/// whether `text` begins a line, so its first line's block markers are
/// markers rather than mid-line text.
///
/// Returns the text and, for each source byte offset `0..=text.len()`, the
/// matching offset in the result.
fn plain_markdown(text: &str, at_line_start: bool) -> (String, Vec<usize>) {
    plain_blocks(text, at_line_start, &[])
}

/// With [`Boxes::Status`], the block markup of each task line that shows
/// in `window` of `text`, as byte ranges of the window: from the line's
/// start to its text's, clipped to the window, so quote and list markers,
/// the box and the spacing after it all go, however they are spaced. The
/// classifier decides what a task is, on the whole line and with the lines
/// before it, so a window that opens inside the markup, a tab after the
/// marker, and a line that only looks like a task are read as the editor
/// reads them. Empty with [`Boxes::Plain`], when snippets are what they
/// were before tasks.
///
/// The note is classified from its first line down to the window, and only
/// when the window's lines hold a `[`.
fn task_prefixes(text: &str, window: Range<usize>, boxes: Boxes) -> Vec<Range<usize>> {
    if boxes != Boxes::Status {
        return Vec::new();
    }
    let first = text[..window.start]
        .rfind(['\n', '\r'])
        .map_or(0, |ending| ending + 1);
    if !text[first..window.end].contains('[') {
        return Vec::new();
    }
    let lines = wren_markdown::line_ranges(text).map(|(line, _)| line);
    classified(text, lines, boxes)
        .take_while(|(line, _)| line.start < window.end)
        .filter(|(_, info)| info.item.is_some_and(|item| item.task.is_some()))
        .map(|(line, info)| {
            let markup = line.start..line.start + info.text_start;
            markup.start.max(window.start)..markup.end.min(window.end)
        })
        .filter(|markup| markup.start < markup.end)
        .map(|markup| markup.start - window.start..markup.end - window.start)
        .collect()
}

/// [`plain_markdown`] for text whose task lines the classifier has found:
/// `tasks` are their block markup ([`task_prefixes`]). Such a line's markup
/// is dropped as given and nothing after it is read as a block marker; any
/// other line is read as [`plain_markdown`] reads it.
fn plain_blocks(text: &str, at_line_start: bool, tasks: &[Range<usize>]) -> (String, Vec<usize>) {
    let mut dropped = vec![false; text.len()];
    let mut block_break = vec![false; text.len()];
    let mut line_start = 0;
    let mut first = true;
    let mut previous_break = None;
    let mut after_heading = false;
    // A `>` line that follows quoted words continues their paragraph.
    let mut after_quote = false;
    while line_start <= text.len() {
        let line_end = text[line_start..]
            .find(['\n', '\r'])
            .map_or(text.len(), |relative| line_start + relative);
        let line = &text[line_start..line_end];
        let mut body = line_start;
        let mut heading = false;
        let mut quote = false;
        if let Some(task) = tasks.iter().find(|task| task.start == line_start) {
            // A task with no description has no words to show.
            let markup = if text[task.end..line_end].trim().is_empty() {
                line_end
            } else {
                task.end
            };
            dropped[line_start..markup].fill(true);
            body = markup;
            if let Some(index) = previous_break {
                block_break[index] = true;
            }
        } else if !first || at_line_start {
            let block = if only_block_markers(line) {
                dropped[line_start..line_end].fill(true);
                body = line_end;
                true
            } else {
                let indent = line.len() - line.trim_start().len();
                let hashes = line[indent..]
                    .bytes()
                    .take_while(|byte| *byte == b'#')
                    .count();
                heading = (1..=6).contains(&hashes) && line[indent + hashes..].starts_with(' ');
                let prefix = if heading {
                    indent + hashes + 1
                } else {
                    line.len() - strip_block_markers(line).len()
                };
                dropped[line_start..line_start + prefix].fill(true);
                body = line_start + prefix;
                quote = prefix > indent
                    && line[indent..prefix]
                        .chars()
                        .all(|character| character == '>' || character.is_whitespace());
                heading || (prefix > indent && !(quote && after_quote))
            };
            if let Some(index) = previous_break.filter(|_| block || after_heading) {
                block_break[index] = true;
            }
        }
        drop_inline_markup(&text[body..line_end], &mut dropped[body..line_end]);
        first = false;
        after_heading = heading;
        after_quote = quote;
        previous_break = Some(line_end);
        line_start = line_end
            + if text[line_end..].starts_with("\r\n") {
                2
            } else {
                1
            };
    }
    join_words(text, &dropped, &block_break)
}

/// Plain lines, such as a board's labels, with each line break shown as
/// [`BLOCK_SEPARATOR`] and nothing dropped. Returns the same offset map as
/// [`plain_markdown`].
fn plain_lines(text: &str) -> (String, Vec<usize>) {
    let block_break: Vec<bool> = text
        .bytes()
        .map(|byte| matches!(byte, b'\n' | b'\r'))
        .collect();
    join_words(text, &vec![false; text.len()], &block_break)
}

/// `text` without its `dropped` bytes, each whitespace run between words
/// shown as one space, or as [`BLOCK_SEPARATOR`] when it holds a
/// `block_break`. Returns the text and, for each source byte offset
/// `0..=text.len()`, the matching offset in the result.
fn join_words(text: &str, dropped: &[bool], block_break: &[bool]) -> (String, Vec<usize>) {
    let mut plain = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len() + 1);
    // Bytes of separator at the end of `plain`, dropped if no word follows.
    let mut gap = 0;
    for (index, character) in text.char_indices() {
        map.resize(index, plain.len());
        map.push(plain.len());
        if dropped[index] {
            continue;
        }
        if !character.is_whitespace() {
            plain.push(character);
            gap = 0;
        } else if plain.is_empty() {
            continue;
        } else if gap == 0 {
            let separator = if block_break[index] {
                BLOCK_SEPARATOR
            } else {
                " "
            };
            plain.push_str(separator);
            gap = separator.len();
        } else if gap == 1 && block_break[index] {
            // Widen the space already pushed rather than replacing it, so
            // offsets mapped to just after it stay on a character boundary.
            plain.push_str(&BLOCK_SEPARATOR[1..]);
            gap = BLOCK_SEPARATOR.len();
        }
    }
    map.resize(text.len(), plain.len());
    plain.truncate(plain.len() - gap);
    map.push(plain.len());
    for offset in &mut map {
        *offset = (*offset).min(plain.len());
    }
    (plain, map)
}

/// Whether `line` is blank or holds only block markers (an empty heading,
/// list item, task, or quote), so it has no words to show.
fn only_block_markers(line: &str) -> bool {
    // Trailing blanks stay, so `3. ` is an empty item but `3.` is a word.
    let mut rest = line.trim_start();
    if rest.trim_end().is_empty() {
        return true;
    }
    let mut listed = false;
    let hashes = rest.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) && hashes == rest.trim_end().len() {
        return true;
    }
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let ordered = digits > 0 && rest[digits..].starts_with(['.', ')']);
        let marker = if rest.starts_with(['>', '-', '*', '+']) {
            1
        } else if listed
            && ["[ ]", "[x]", "[X]"]
                .iter()
                .any(|task| rest.starts_with(task))
        {
            3
        } else if ordered {
            digits + 1
        } else {
            return false;
        };
        let after = &rest[marker..];
        let quote = rest.starts_with('>');
        // A bare number needs the space that makes it a list item.
        if ordered && after.is_empty() {
            return false;
        }
        if !quote && !after.is_empty() && !after.starts_with(char::is_whitespace) {
            return false;
        }
        listed = !quote;
        rest = after.trim_start();
    }
    true
}

/// Mark the delimiters of code spans, emphasis, strikethrough, and links in
/// one line. A `*` or `_` run counts as emphasis only when it opens or closes
/// a word, so `2*3` and `snake_case` keep theirs.
fn drop_inline_markup(line: &str, dropped: &mut [bool]) {
    let bytes = line.as_bytes();
    let blank = |character: Option<char>| {
        character
            .is_none_or(|character| character.is_whitespace() || character.is_ascii_punctuation())
    };
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'`' => {
                dropped[index] = true;
                index += 1;
            }
            b'~' if bytes.get(index + 1) == Some(&b'~') => {
                dropped[index..index + 2].fill(true);
                index += 2;
            }
            marker @ (b'*' | b'_') => {
                let run = bytes[index..]
                    .iter()
                    .take_while(|byte| **byte == marker)
                    .count();
                let before = line[..index].chars().next_back();
                let after = line[index + run..].chars().next();
                let opens = !blank(after) && blank(before);
                let closes = !blank(before) && blank(after);
                if run <= 3 && (opens || closes) {
                    dropped[index..index + run].fill(true);
                }
                index += run;
            }
            b'[' => {
                let Some(close) = line[index + 1..]
                    .find(']')
                    .map(|relative| index + 1 + relative)
                else {
                    index += 1;
                    continue;
                };
                let destination = match bytes.get(close + 1) {
                    Some(b'(') => line[close + 2..]
                        .find(')')
                        .map(|relative| close + 2 + relative),
                    Some(b'[') => line[close + 2..]
                        .find(']')
                        .map(|relative| close + 2 + relative),
                    _ => None,
                };
                let Some(end) = destination else {
                    index += 1;
                    continue;
                };
                if index > 0 && bytes[index - 1] == b'!' {
                    dropped[index - 1] = true;
                }
                dropped[index] = true;
                dropped[close..=end].fill(true);
                drop_inline_markup(&line[index + 1..close], &mut dropped[index + 1..close]);
                index = end + 1;
            }
            _ => index += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use crate::kind::NoteKind;
    use crate::model::{self, Group, Note, Section};

    use super::*;

    fn workspace() -> Workspace {
        let mut workspace = Workspace::empty(PathBuf::from("/unused"));
        for (id, path) in [(SectionId(1), "A"), (SectionId(2), "B")] {
            workspace.sections.push(Section {
                id,
                name: path.into(),
                path: RelPath::new(path).unwrap(),
            });
        }
        for (id, section, path) in [
            (GroupId(3), SectionId(1), "A/G"),
            (GroupId(7), SectionId(1), "A/H"),
            (GroupId(8), SectionId(2), "B/C"),
        ] {
            workspace.groups.push(Group {
                id,
                section,
                name: model::dir_name(&RelPath::new(path).unwrap()),
                path: RelPath::new(path).unwrap(),
            });
        }
        for (id, group, path) in [
            (NoteId(4), GroupId(7), "A/H/Note 2.md"),
            (NoteId(5), GroupId(3), "A/G/Note 10.md"),
            (NoteId(6), GroupId(8), "B/C/Other.md"),
        ] {
            workspace.notes.push(Note {
                id,
                group,
                title: NoteKind::Markdown.title(RelPath::new(path).unwrap().file_name()),
                kind: NoteKind::Markdown,
                path: RelPath::new(path).unwrap(),
                subtitle: None,
                modified: SystemTime::UNIX_EPOCH,
                bytes: 0,
            });
        }
        workspace
    }

    fn index() -> Index {
        let mut index = Index::default();
        index.sync_metadata(&workspace());
        for note in [NoteId(4), NoteId(5), NoteId(6)] {
            index.set_content(note, "shared content", false);
        }
        index
    }

    fn ids(index: &Index, query: &str, scope: Scope, kind: QueryKind, limit: usize) -> Vec<NoteId> {
        index
            .query(query, scope, kind, limit)
            .iter()
            .map(|hit| hit.note)
            .collect()
    }

    #[test]
    fn scopes_distinguish_section_descendants_from_one_group() {
        let index = index();
        assert_eq!(
            ids(&index, "shared", Scope::Workspace, QueryKind::Search, 20),
            [NoteId(4), NoteId(5), NoteId(6)]
        );
        assert_eq!(
            ids(
                &index,
                "shared",
                Scope::Section(SectionId(1)),
                QueryKind::Search,
                20
            ),
            [NoteId(4), NoteId(5)]
        );
        assert_eq!(
            ids(
                &index,
                "shared",
                Scope::Group(GroupId(7)),
                QueryKind::Filter,
                20
            ),
            [NoteId(4)]
        );
        assert_eq!(
            ids(
                &index,
                "shared",
                Scope::Group(GroupId(3)),
                QueryKind::Filter,
                20
            ),
            [NoteId(5)]
        );
        assert!(index
            .query(
                "shared",
                Scope::Section(SectionId(999)),
                QueryKind::Search,
                20
            )
            .is_empty());
    }

    #[test]
    fn oversized_fuzzy_queries_are_rejected_before_pattern_allocation() {
        let query = "a ".repeat(129);
        assert_eq!(fuzzy_score(&query, &query), None);
        assert_eq!(
            validate_query(&query),
            Err("Search queries are limited to 128 words")
        );
        assert!(validate_query(&"a ".repeat(MAX_QUERY_WORDS)).is_ok());
        assert!(validate_query(&"é".repeat(MAX_QUERY_BYTES / 2)).is_ok());
        let bytes = "é".repeat(MAX_QUERY_BYTES / 2 + 1);
        assert_eq!(
            validate_query(&bytes),
            Err("Search queries are limited to 4,096 UTF-8 bytes")
        );
        assert_eq!(fuzzy_score(&bytes, &bytes), None);
        let index = index();
        for kind in [QueryKind::Jump, QueryKind::Search, QueryKind::Filter] {
            assert!(index
                .query(&bytes, Scope::Workspace, kind, usize::MAX)
                .is_empty());
            assert!(index.query(&query, Scope::Workspace, kind, 1).is_empty());
            assert!(Index::default()
                .query(&bytes, Scope::Workspace, kind, 1)
                .is_empty());
        }
    }

    #[test]
    fn blank_queries_and_limits_are_deliberate() {
        let index = index();
        assert!(index
            .query(" \t\n", Scope::Workspace, QueryKind::Search, 20)
            .is_empty());
        assert_eq!(
            ids(&index, " \t\n", Scope::Workspace, QueryKind::Filter, 20),
            [NoteId(4), NoteId(5), NoteId(6)]
        );
        assert_eq!(
            index.query("", Scope::Workspace, QueryKind::Jump, 2).len(),
            2
        );
        assert!(index
            .query("note", Scope::Workspace, QueryKind::Jump, 0)
            .is_empty());
        assert!(index
            .query(
                "no matches",
                Scope::Workspace,
                QueryKind::Search,
                usize::MAX
            )
            .is_empty());
    }

    #[test]
    fn jump_is_title_only_and_recency_breaks_equal_scores() {
        let mut index = index();
        let mut workspace = workspace();
        workspace.notes[1].title = workspace.notes[0].title.clone();
        index.sync_metadata(&workspace);
        assert!(index
            .query("shared", Scope::Workspace, QueryKind::Jump, 20)
            .is_empty());
        index.opened(NoteId(4));
        index.opened(NoteId(5));
        assert_eq!(
            ids(&index, "note", Scope::Workspace, QueryKind::Jump, 20),
            [NoteId(5), NoteId(4)]
        );
        index.opened(NoteId(4));
        assert_eq!(
            ids(&index, "note", Scope::Workspace, QueryKind::Jump, 1),
            [NoteId(4)]
        );
        assert_eq!(
            ids(&index, "", Scope::Workspace, QueryKind::Jump, 1),
            [NoteId(4)]
        );
    }

    #[test]
    fn jump_match_quality_ranks_before_opened_recency() {
        let mut index = index();
        let mut workspace = workspace();
        workspace.notes[0].title = "Note".into();
        workspace.notes[1].title = "N o t e".into();
        index.sync_metadata(&workspace);
        index.opened(NoteId(5));
        let hits = index.query("note", Scope::Workspace, QueryKind::Jump, 20);
        assert_eq!(
            hits.iter().map(|hit| hit.note).collect::<Vec<_>>(),
            [NoteId(4), NoteId(5)]
        );
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn fuzzy_score_uses_literal_words_without_special_operators() {
        assert!(fuzzy_score("dsg nt", "Design Notes").is_some());
        assert!(fuzzy_score("cafe", "Café").is_some());
        assert_eq!(fuzzy_score("  ", "Anything"), Some(0));
        assert_eq!(fuzzy_score("!note", "note"), None);
        assert!(fuzzy_score("!note", "!note").is_some());
        assert!(fuzzy_score("Note", "Note").unwrap() > fuzzy_score("Note", "N o t e").unwrap());
    }

    #[test]
    fn title_match_ranks_before_content_only_without_changing_match_offsets() {
        let mut index = index();
        index.set_content(NoteId(6), "a note in the body", false);
        let hits = index.query("note", Scope::Workspace, QueryKind::Search, 20);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[2].note, NoteId(6));
        assert_eq!(hits[2].content_match, Some(2..6));
        assert_eq!(hits[2].snippet.highlight, Some(2..6));
    }

    #[test]
    fn unicode_lowercase_expansions_map_to_whole_original_characters() {
        let mut index = index();
        index.set_content(NoteId(6), "前İSTANBUL café ΟΣ 後", false);
        for (query, expected) in [
            ("i", "İ"),
            ("i\u{307}s", "İS"),
            ("\u{307}", "İ"),
            ("CAFÉ", ""),
            ("CAFÉ", "café"),
            ("οσ", "ΟΣ"),
            ("ος", ""),
        ] {
            let hits = index.query(query, Scope::Group(GroupId(8)), QueryKind::Search, 10);
            if expected.is_empty() {
                assert!(hits.is_empty());
                continue;
            }
            let hit = &hits[0];
            assert_eq!(
                &index.content(hit.note).unwrap()[hit.content_match.clone().unwrap()],
                expected
            );
            assert_eq!(
                &hit.snippet.text[hit.snippet.highlight.clone().unwrap()],
                expected
            );
        }
        assert_eq!(
            index.retained_bytes,
            index
                .entries
                .values()
                .filter_map(|entry| entry.content.as_ref())
                .map(Content::bytes)
                .sum()
        );
    }

    #[test]
    fn plain_markdown_keeps_words_and_drops_markup() {
        for (source, plain) in [
            (
                "Owner: **Alice Chen** · *Bob*, _Priya_",
                "Owner: Alice Chen · Bob, Priya",
            ),
            (
                "Status: `in review` and ~~old~~ new",
                "Status: in review and old new",
            ),
            (
                "See [the docs](https://x.io) and ![logo](a.png)",
                "See the docs and logo",
            ),
            ("Read [the guide][ref] now", "Read the guide now"),
            (
                "## Agenda\n- [ ] Review **promo**\n> quoted",
                "Agenda · Review promo · quoted",
            ),
            (
                "2*3 = 6 and snake_case_name stay",
                "2*3 = 6 and snake_case_name stay",
            ),
            ("a * b and [unclosed link", "a * b and [unclosed link"),
            ("***both*** café", "both café"),
        ] {
            assert_eq!(plain_markdown(source, true).0, plain, "{source:?}");
        }
        // Mid-line text keeps a leading `#` or `-`; it is not a block marker.
        assert_eq!(plain_markdown("- item # one", false).0, "- item # one");
    }

    #[test]
    fn plain_markdown_separates_blocks_and_joins_wrapped_prose() {
        for (source, plain) in [
            (
                "- [ ] Review promo\n- [x] Share Q4",
                "Review promo · Share Q4",
            ),
            ("## Agenda\n- API migration", "Agenda · API migration"),
            ("## Agenda\nPlain paragraph", "Agenda · Plain paragraph"),
            (
                "first line of prose\ncontinues here",
                "first line of prose continues here",
            ),
            ("Intro\n1. One\n2) Two", "Intro · One · Two"),
            ("Said:\n> quoted\n+ plus", "Said: · quoted · plus"),
            ("- item\n  wrapped item", "item wrapped item"),
            ("One para\n\n\nTwo para", "One para · Two para"),
            ("One para  \r\n\r\nTwo para", "One para · Two para"),
            ("prose\r\nwraps", "prose wraps"),
            ("\n\n- Lead\n\n", "Lead"),
            ("- [x]\nNext", "Next"),
            ("Owner: Alice · Bob", "Owner: Alice · Bob"),
            // A quote wrapped over `>` lines is one paragraph; an empty `>`
            // line between them splits it.
            ("> quoted line\n> wraps on", "quoted line wraps on"),
            ("> one\n>\n> two", "one · two"),
            ("Said:\n> quoted\n> more", "Said: · quoted more"),
            // A bare number or `[x]` with no list marker is just text.
            ("The answer is\n42.", "The answer is 42."),
            ("draft\n[x]", "draft [x]"),
        ] {
            assert_eq!(plain_markdown(source, true).0, plain, "{source:?}");
        }
        // Lines with no words after their markers add nothing, not even a
        // separator, at either end or between blocks.
        for (source, plain) in [
            (
                "Review promo packet draft\n- [x]",
                "Review promo packet draft",
            ),
            ("draft\n- [ ]\n- [x] Share", "draft · Share"),
            ("draft\n> \n-\n## \n#\n3. ", "draft"),
        ] {
            assert_eq!(plain_markdown(source, false).0, plain, "{source:?}");
        }
        assert!(only_block_markers("> - [ ]"));
        assert!(!only_block_markers("---"));
        assert!(!only_block_markers("-x"));
        assert!(!only_block_markers("[ ]x"));
        assert!(!only_block_markers("2026-11-04"));
        assert!(!only_block_markers("42."));
        assert!(!only_block_markers("[x]"));
        assert!(only_block_markers("1. [x] "));
    }

    #[test]
    fn snippet_highlights_map_through_block_separators() {
        let text = "Intro words here\n- [ ] Review promo\n- [x] Share Q4 deck\n- [x]";
        for needle in ["promo\n- [x] Share", "Share", "Review"] {
            let hit = text.find(needle).unwrap();
            let result = snippet(
                text,
                hit..hit + needle.len(),
                NoteKind::Markdown,
                Boxes::Plain,
            );
            assert_eq!(
                result.text,
                "Intro words here · Review promo · Share Q4 deck"
            );
            let shown = needle.replace("\n- [x] ", " · ");
            assert_eq!(&result.text[result.highlight.unwrap()], shown);
        }
        // A window that closes just after a task marker drops the marker.
        let text = format!(
            "Review promo packet draft\n- [x] {}",
            "z".repeat(CONTEXT_CHARS)
        );
        let hit = text.find("packet").unwrap();
        let result = snippet(&text, hit..hit + 6, NoteKind::Markdown, Boxes::Plain);
        assert_eq!(result.text, "Review promo packet draft…");
        assert_eq!(&result.text[result.highlight.unwrap()], "packet");
    }

    #[test]
    fn snippet_highlights_follow_the_match_through_dropped_markup() {
        let text = "## Agenda\n- API migration: cutover and **rollback** window\n";
        let hit = text.find("rollback").unwrap();
        let result = snippet(
            text,
            hit..hit + "rollback".len(),
            NoteKind::Markdown,
            Boxes::Plain,
        );
        assert_eq!(
            result.text,
            "Agenda · API migration: cutover and rollback window"
        );
        assert_eq!(&result.text[result.highlight.unwrap()], "rollback");
        let marker = text.find("**").unwrap();
        assert_eq!(
            snippet(text, marker..marker + 2, NoteKind::Markdown, Boxes::Plain).highlight,
            None
        );

        // Context trimmed mid-word or mid-link snaps to the next word.
        let text = format!(
            "{} [diff dashboard](https://grafana.internal/d/api-diff)\n- [ ] Ramp with a rollback gate and more words after the match {}",
            "lead ".repeat(20),
            "tail ".repeat(20)
        );
        let hit = text.find("rollback").unwrap();
        let result = snippet(&text, hit..hit + 8, NoteKind::Markdown, Boxes::Plain);
        assert!(
            result.text.starts_with("…Ramp") || result.text.starts_with("…diff"),
            "{}",
            result.text
        );
        assert!(!result.text.contains("f)"), "{}", result.text);
        assert!(
            result.text.ends_with(" …") || result.text.ends_with("tail…"),
            "{}",
            result.text
        );
        assert_eq!(&result.text[result.highlight.unwrap()], "rollback");
    }

    #[test]
    fn markup_and_word_boundaries_after_multi_byte_characters_stay_on_char_boundaries() {
        for text in [
            "café*bold*",
            "naïve_name",
            "日本語_x_ and **é**",
            "é_",
            "»*«",
        ] {
            plain_markdown(text, true);
            Preview::new(text, NoteKind::Markdown, Boxes::Plain);
        }
        assert_eq!(
            plain_markdown("café *latte* naïve_name", true).0,
            "café latte naïve_name"
        );
        // The leading context's first whitespace is a three-byte ideographic space.
        let text = format!("{}\u{3000}abc needle", "語".repeat(100));
        let hit = text.find("needle").unwrap();
        let result = snippet(&text, hit..hit + 6, NoteKind::Markdown, Boxes::Plain);
        assert_eq!(result.text, "…abc needle");
        assert_eq!(&result.text[result.highlight.unwrap()], "needle");
    }

    #[test]
    fn snippets_are_utf8_safe_near_both_ends_and_in_long_content() {
        for text in ["é日本", "", "a"] {
            let result = Preview::new(text, NoteKind::Markdown, Boxes::Plain).snippet();
            assert_eq!(result.text, text);
            assert!(result.highlight.is_none());
        }
        let text = format!("{}Needle{}", "界".repeat(200), "é".repeat(200));
        let result = snippet(&text, 600..606, NoteKind::Markdown, Boxes::Plain);
        assert!(result.text.starts_with('…') && result.text.ends_with('…'));
        assert_eq!(&result.text[result.highlight.unwrap()], "Needle");
        for hit in [0..3, text.len() - 2..text.len()] {
            let result = snippet(&text, hit.clone(), NoteKind::Markdown, Boxes::Plain);
            assert_eq!(&result.text[result.highlight.unwrap()], &text[hit]);
        }
    }

    #[test]
    fn bounded_content_keeps_titles_and_prioritizes_live_drafts_without_warning_loops() {
        let mut index = Index::with_budget(30);
        index.sync_metadata(&workspace());
        index.set_content(NoteId(4), "1234567890", false);
        index.set_content(NoteId(5), "1234567890", false);
        assert_eq!(index.retained_bytes(), 20);
        assert!(index.limitation(NoteId(5)).is_some());
        assert_eq!(index.take_warnings().len(), 1);
        index.set_content(NoteId(5), "1234567890", false);
        assert!(index.take_warnings().is_empty());
        index.set_content(NoteId(5), "draft123456", true);
        assert_eq!(index.content(NoteId(5)), Some("draft123456"));
        assert_eq!(index.content(NoteId(4)), None);
        assert!(index.retained_bytes() <= 30);
        index.set_content(NoteId(6), &"İ".repeat(7), false);
        assert!(index.content(NoteId(6)).is_none());
        assert_eq!(index.len(), 3);
        assert_eq!(
            index
                .query("Note", Scope::Workspace, QueryKind::Jump, 20)
                .len(),
            2
        );
        index.sync_metadata(&Workspace::empty(PathBuf::from("/unused")));
        assert!(index.is_empty());
        assert_eq!(index.retained_bytes(), 0);
    }

    #[test]
    fn unchanged_content_is_revision_stable_and_unavailable_content_is_not_stale() {
        let mut index = index();
        let revision = index.revision();
        index.set_content(NoteId(4), "shared content", false);
        assert_eq!(index.revision(), revision);
        index.unavailable(NoteId(4), "read failed");
        assert!(index.revision() > revision);
        assert!(index.content(NoteId(4)).is_none());
        assert_eq!(index.take_warnings().len(), 1);
        index.unavailable(NoteId(4), "read failed");
        assert!(index.take_warnings().is_empty());
        index.set_content(NoteId(4), "recovered", false);
        assert_eq!(index.limitation(NoteId(4)), None);

        // A board's source is the change test: the same source is never
        // extracted again and changes nothing.
        let mut index = board_index();
        index.set_content(BOARD, BOARD_SOURCE, false);
        assert_eq!(index.extractions, 1);
        let revision = index.revision();
        for _ in 0..40 {
            index.set_content(BOARD, BOARD_SOURCE, false);
        }
        assert_eq!(index.extractions, 1);
        assert_eq!(index.revision(), revision);
        index.set_content(BOARD, "schema = 1\n", false);
        assert_eq!(index.extractions, 2);
        assert!(index.revision() > revision);
        index.set_content(NoteId(4), "markdown", false);
        assert_eq!(index.extractions, 2);
    }

    const BOARD: NoteId = NoteId(9);
    const BOARD_SOURCE: &str = "schema = 1\n\n[node.a]\ntext = \"Receive *order*\"\n";

    /// The test workspace plus a board beside the Markdown notes of `A/G`.
    fn board_index() -> Index {
        let mut index = Index::default();
        index.sync_metadata(&board_index_workspace());
        index
    }

    fn board_index_workspace() -> Workspace {
        let mut workspace = workspace();
        let path = RelPath::new("A/G/Flow.wren-board").unwrap();
        workspace.notes.push(Note {
            id: BOARD,
            group: GroupId(3),
            title: NoteKind::Board.title(path.file_name()),
            kind: NoteKind::Board,
            path,
            subtitle: None,
            modified: SystemTime::UNIX_EPOCH,
            bytes: 0,
        });
        workspace
    }

    #[test]
    fn a_deferred_source_is_searched_once_it_is_settled() {
        let mut index = board_index();
        let source = |label: &str| {
            format!(
                "schema = 1\n\n[node.a]\ntext = \"{label}\"\nkind = \"shape\"\n\
                 at = [0, 0]\nsize = [120, 60]\n"
            )
        };
        let (first, typed) = (source("Receive order"), source("Receive orders"));
        let (first, typed) = (first.as_str(), typed.as_str());
        index.set_content(BOARD, first, true);
        for _ in 0..5 {
            index.defer_content(BOARD, typed);
        }
        assert_eq!(index.extractions, 1);
        assert_eq!(index.content(BOARD), Some("Receive order"));
        // A background scan compares the disk against the newest draft.
        assert_eq!(&*index.content_snapshot()[&BOARD], typed);
        index.settle();
        assert_eq!(index.extractions, 2);
        assert_eq!(index.content(BOARD), Some("Receive orders"));
        assert!(!index.has_pending());

        // The source already kept waits for nothing, and a set source of
        // the same note replaces the one that waited.
        index.defer_content(BOARD, typed);
        assert!(!index.has_pending());
        index.defer_content(BOARD, first);
        index.set_content(BOARD, typed, true);
        assert!(!index.has_pending());
        assert_eq!(index.extractions, 2);
        // Another note's source settles the one that waited.
        index.defer_content(BOARD, first);
        index.defer_content(NoteId(4), "other");
        assert_eq!(index.content(BOARD), Some("Receive order"));
        // A note that left the workspace leaves nothing waiting.
        index.defer_content(BOARD, typed);
        assert!(index.has_pending());
        index.sync_metadata(&workspace());
        assert!(!index.has_pending());
    }

    #[test]
    fn markdown_searches_its_source_in_one_shared_allocation() {
        let mut index = board_index();
        let text = "# Title\n\nBody *words*\n";
        index.set_content(NoteId(5), text, false);
        let content = index.entries[&NoteId(5)].content.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &content.source,
            &content.searched.as_ref().unwrap().text
        ));
        assert_eq!(&*content.source, text);
        assert_eq!(index.content(NoteId(5)), Some(text));
        assert_eq!(index.retained_bytes(), 2 * text.len());
        assert_eq!(index.preview(NoteId(5)), Some("Body words"));
        assert_eq!(index.extractions, 0);

        index.set_content(BOARD, BOARD_SOURCE, false);
        let snapshot = index.content_snapshot();
        assert_eq!(&*snapshot[&NoteId(5)], text);
        assert_eq!(&*snapshot[&BOARD], BOARD_SOURCE);
        assert!(Arc::ptr_eq(
            &snapshot[&BOARD],
            &index.entries[&BOARD].content.as_ref().unwrap().source
        ));
    }

    #[test]
    fn a_board_preview_and_snippet_read_plain_lines() {
        let mut index = board_index();
        index.set_content(
            BOARD,
            "\n   # Receive *order*  \n- Check stock\nShip",
            false,
        );
        assert_eq!(index.preview(BOARD), Some("# Receive *order*"));
        let hits = index.query("check", Scope::Group(GroupId(3)), QueryKind::Search, 10);
        assert_eq!(hits.len(), 1);
        let snippet = &hits[0].snippet;
        assert_eq!(snippet.text, "# Receive *order* · - Check stock · Ship");
        assert_eq!(&snippet.text[snippet.highlight.clone().unwrap()], "Check");

        let long = "word ".repeat(100);
        index.set_content(BOARD, &format!("{long}\nsecond"), false);
        let preview = index.entries[&BOARD]
            .content
            .as_ref()
            .unwrap()
            .searched
            .as_ref()
            .unwrap()
            .preview
            .snippet();
        assert_eq!(preview.text.chars().count(), PREVIEW_CHARS + 1);
        assert!(preview.text.ends_with('…'));
        index.set_content(BOARD, " \n\t\n", false);
        assert_eq!(index.preview(BOARD), Some(""));
    }

    /// A valid board: two labelled shapes, the lower one first in the file,
    /// and a labelled connector.
    const LABELLED_BOARD: &str = r#"schema = 1

[node.pack]
text = "Pack the parcel"
kind = "shape"
at = [0, 200]
size = [120, 60]

[node.receive]
text = "Receive order"
kind = "shape"
at = [0, 0]
size = [120, 60]

[edge.then]
kind = "edge"
from = { node = "receive" }
to = { node = "pack" }
labels = [{ text = "when paid" }]
"#;

    fn search(index: &Index, query: &str) -> Vec<Hit> {
        index.query(query, Scope::Workspace, QueryKind::Search, 10)
    }

    #[test]
    fn a_board_is_searched_by_its_labels_and_never_by_its_toml() {
        let mut index = board_index();
        index.set_content(BOARD, LABELLED_BOARD, false);
        assert_eq!(index.extractions, 1);
        assert_eq!(
            index.content(BOARD),
            Some("Receive order\nPack the parcel\nwhen paid")
        );
        // The preview is the first label in reading order, not in file order.
        assert_eq!(index.preview(BOARD), Some("Receive order"));
        for word in ["parcel", "RECEIVE", "paid"] {
            let hits = search(&index, word);
            assert_eq!(hits.len(), 1, "{word}");
            assert_eq!(hits[0].note, BOARD, "{word}");
            let range = hits[0].content_match.clone().unwrap();
            assert!(
                index.content(BOARD).unwrap()[range].eq_ignore_ascii_case(word),
                "{word}"
            );
        }
        let snippet = &search(&index, "parcel")[0].snippet;
        assert_eq!(snippet.text, "Receive order · Pack the parcel · when paid");
        assert_eq!(&snippet.text[snippet.highlight.clone().unwrap()], "parcel");
        for syntax in ["schema", "kind", "shape", "node", "size", "labels"] {
            assert!(search(&index, syntax).is_empty(), "{syntax}");
        }
        // Change detection still reads the source, and the extracted text is
        // counted beside it.
        assert_eq!(&*index.content_snapshot()[&BOARD], LABELLED_BOARD);
        let extracted = index.content(BOARD).unwrap().len();
        assert_eq!(index.retained_bytes(), LABELLED_BOARD.len() + 2 * extracted);
        index.set_content(BOARD, LABELLED_BOARD, false);
        assert_eq!(index.extractions, 1);

        // A board with no label has nothing to find and an empty preview.
        index.set_content(BOARD, wren_board::EMPTY, false);
        assert_eq!(index.content(BOARD), Some(""));
        assert_eq!(index.preview(BOARD), Some(""));
        assert!(search(&index, "schema").is_empty());
    }

    #[test]
    fn a_board_that_does_not_parse_is_searched_as_written() {
        let mut index = board_index();
        let conflicted = format!("<<<<<<< ours\n{LABELLED_BOARD}=======\n>>>>>>> theirs\n");
        assert!(wren_board::parse(&conflicted).is_err());
        index.set_content(BOARD, &conflicted, false);
        let content = index.entries[&BOARD].content.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &content.source,
            &content.searched.as_ref().unwrap().text
        ));
        for word in ["schema", "parcel", "theirs"] {
            assert_eq!(search(&index, word).len(), 1, "{word}");
        }
        assert_eq!(index.preview(BOARD), Some("<<<<<<< ours"));

        // Once the text parses again its labels take over.
        index.set_content(BOARD, LABELLED_BOARD, false);
        assert!(search(&index, "schema").is_empty());
        assert_eq!(search(&index, "parcel").len(), 1);

        // A board over the size a board may have is not read as one: it is
        // searched as written, as the app opens it as text.
        let pad = "x".repeat(wren_board::MAX_BOARD_BYTES);
        let large =
            LABELLED_BOARD.replacen("schema = 1\n", &format!("schema = 1\npad = \"{pad}\"\n"), 1);
        assert!(wren_board::parse(&large).is_err());
        index.set_content(BOARD, &large, false);
        let content = index.entries[&BOARD].content.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &content.source,
            &content.searched.as_ref().unwrap().text
        ));
        assert_eq!(search(&index, "schema").len(), 1);
    }

    #[test]
    fn a_board_read_from_disk_waits_for_its_labels_and_takes_only_its_own() {
        let mut index = board_index();
        index.hold_content(BOARD, LABELLED_BOARD);
        // Held, not parsed: found by its title, never by its labels or TOML.
        assert_eq!(index.extractions, 0);
        assert_eq!(index.unextracted(), 1);
        assert_eq!(index.content(BOARD), None);
        assert_eq!(index.preview(BOARD), None);
        assert!(search(&index, "parcel").is_empty());
        assert!(search(&index, "schema").is_empty());
        let hits = search(&index, "flow");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet.text, "");
        assert_eq!(index.retained_bytes(), LABELLED_BOARD.len());
        assert_eq!(&*index.content_snapshot()[&BOARD], LABELLED_BOARD);
        // Markdown is never held.
        index.hold_content(NoteId(5), "# Title\n\nBody\n");
        assert_eq!(index.content(NoteId(5)), Some("# Title\n\nBody\n"));
        assert_eq!(index.unextracted(), 1);

        // Labels taken from another source are not kept.
        let [(note, source)] = index.waiting().try_into().unwrap();
        assert_eq!((note, &*source), (BOARD, LABELLED_BOARD));
        index.extracted(BOARD, wren_board::EMPTY, Some(String::new()));
        assert_eq!(index.unextracted(), 1);
        index.extracted(NoteId(5), "# Title\n\nBody\n", Some("labels".into()));
        assert_eq!(index.content(NoteId(5)), Some("# Title\n\nBody\n"));

        let revision = index.revision();
        let text = searchable(NoteKind::Board, &source);
        index.extracted(BOARD, &source, text);
        assert!(index.revision() > revision);
        assert_eq!(index.unextracted(), 0);
        assert_eq!(index.extractions, 0);
        assert_eq!(index.preview(BOARD), Some("Receive order"));
        assert_eq!(search(&index, "parcel").len(), 1);
        let extracted = index.content(BOARD).unwrap().len();
        let markdown = 2 * "# Title\n\nBody\n".len();
        assert_eq!(
            index.retained_bytes(),
            LABELLED_BOARD.len() + 2 * extracted + markdown
        );
        // Labels kept are not replaced by a second delivery, and the same
        // source read again keeps them.
        index.extracted(BOARD, &source, Some("other".into()));
        index.hold_content(BOARD, LABELLED_BOARD);
        assert_eq!(index.preview(BOARD), Some("Receive order"));

        // A changed source waits again; extracting what waits finds it.
        index.hold_content(BOARD, wren_board::EMPTY);
        assert_eq!(index.unextracted(), 1);
        assert_eq!(index.retained_bytes(), wren_board::EMPTY.len() + markdown);
        index.extract_waiting();
        assert_eq!(index.extractions, 1);
        assert_eq!(index.content(BOARD), Some(""));
    }

    #[test]
    fn labels_that_do_not_fit_the_budget_leave_the_title_searchable() {
        let mut index = Index::with_budget(LABELLED_BOARD.len() + 10);
        index.sync_metadata(&board_index_workspace());
        index.hold_content(BOARD, LABELLED_BOARD);
        assert_eq!(index.unextracted(), 1);
        index.extract_waiting();
        assert_eq!(index.unextracted(), 0);
        assert_eq!(index.retained_bytes(), 0);
        assert!(index.limitation(BOARD).is_some());
        assert_eq!(search(&index, "flow").len(), 1);
    }

    #[test]
    fn plain_lines_join_lines_with_the_block_separator() {
        for (text, plain) in [
            ("one", "one"),
            ("one\ntwo", "one · two"),
            ("one \r\n\n  two  three\n", "one · two three"),
            ("\n\nlead", "lead"),
            ("", ""),
        ] {
            let (result, map) = plain_lines(text);
            assert_eq!(result, plain, "{text:?}");
            assert_eq!(map.len(), text.len() + 1);
            assert!(map.iter().all(|offset| result.is_char_boundary(*offset)));
        }
    }

    #[test]
    fn charwise_casing_agrees_for_sigma_expansions_combining_emoji_and_original_line_endings() {
        let mut index = index();
        let scope = Scope::Group(GroupId(8));
        for (text, query, matched) in [
            ("ΟΣ", "οσ", Some("ΟΣ")),
            ("ΟΣ", "ΟΣ", Some("ΟΣ")),
            ("ΟΣ", "ος", None),
            ("ος", "ΟΣ", None),
            ("ος", "ος", Some("ος")),
            ("Σ σ ς", "σ", Some("Σ")),
            ("Σ σ ς", "ς", Some("ς")),
            ("前İSTANBUL後", "I\u{307}s", Some("İS")),
            ("前İ後", "\u{307}", Some("İ")),
            ("👩🏽‍💻 e\u{301} café", "👩🏽‍💻 E\u{301}", Some("👩🏽‍💻 e\u{301}")),
            ("e\u{301}", "é", None),
            ("Straße", "strasse", None),
            ("前\r\nİS\r\nNext\rTail\n終", "is\r\nnext", None),
            (
                "前\r\nİS\r\nNext\rTail\n終",
                "i\u{307}s\r\nnext",
                Some("İS\r\nNext"),
            ),
            (
                "前\r\nİS\r\nNext\rTail\n終",
                "next\rtail\n終",
                Some("Next\rTail\n終"),
            ),
            (
                "前\r\nİS\r\n- Next\n終",
                "i\u{307}s\r\n- next",
                Some("İS\r\n- Next"),
            ),
        ] {
            index.set_content(NoteId(6), text, false);
            let hits = index.query(query, scope, QueryKind::Search, 10);
            assert_eq!(
                !hits.is_empty(),
                lowercase(text).contains(&lowercase(query)),
                "{text:?}, {query:?}"
            );
            if let Some(matched) = matched {
                assert_eq!(hits.len(), 1, "{query:?}");
                let hit = &hits[0];
                assert_eq!(&text[hit.content_match.clone().unwrap()], matched);
                // Snippets join wrapped prose with a space and blocks with
                // the block separator.
                let shown = matched
                    .replace("\r\n- ", BLOCK_SEPARATOR)
                    .replace("\r\n", " ")
                    .replace(['\r', '\n'], " ");
                assert_eq!(
                    &hit.snippet.text[hit.snippet.highlight.clone().unwrap()],
                    shown
                );
                assert!(text.is_char_boundary(hit.content_match.as_ref().unwrap().start));
                assert!(text.is_char_boundary(hit.content_match.as_ref().unwrap().end));
            } else {
                assert!(hits.is_empty(), "{text:?}, {query:?}");
            }
        }
    }

    #[test]
    fn previews_drop_list_task_and_quote_markers() {
        for (line, expected) in [
            ("- API migration", "API migration"),
            ("  - [ ] Review promo", "Review promo"),
            ("- [x] Shipped", "Shipped"),
            ("> - quoted item", "quoted item"),
            ("12. Numbered", "Numbered"),
            ("3) Paren", "Paren"),
            ("-", ""),
            ("- [ ]", ""),
            ("Plain *emphasis*", "Plain *emphasis*"),
            ("2026-11-04 notes", "2026-11-04 notes"),
        ] {
            assert_eq!(&line[body_line(line, Boxes::Plain)], expected, "{line:?}");
        }
        let text = "# Title\n\n- First item\n";
        assert_eq!(&text[body_line(text, Boxes::Plain)], "First item");
        let text = "# Title\n- [ ]\n>\n- [x] Shipped\n";
        assert_eq!(&text[body_line(text, Boxes::Plain)], "Shipped");
    }

    /// With tasks on, a box of any status is a marker the preview drops,
    /// whatever its symbol's width; off, only the three plain boxes are.
    #[test]
    fn previews_drop_status_boxes_only_with_tasks_on() {
        for (line, plain, status) in [
            ("- [/] Doing", "[/] Doing", "Doing"),
            ("- [-] Dropped", "[-] Dropped", "Dropped"),
            ("- [界] x", "[界] x", "x"),
            ("  1. [é]\tAccent", "[é]\tAccent", "Accent"),
            ("> - [✓] Quoted", "[✓] Quoted", "Quoted"),
            ("- [😀]  Wide", "[😀]  Wide", "Wide"),
            ("- [1] x", "[1] x", "x"),
            ("- [[] bracket", "[[] bracket", "bracket"),
            ("- [ ] Open", "Open", "Open"),
            ("- [X] Shipped", "Shipped", "Shipped"),
            // Not boxes with tasks on either.
            ("- [✔\u{fe0f}] two", "[✔\u{fe0f}] two", "[✔\u{fe0f}] two"),
            ("- []] no", "[]] no", "[]] no"),
            ("- [a]: http://u", "[a]: http://u", "[a]: http://u"),
            ("[/] bare", "[/] bare", "[/] bare"),
            ("- [/]", "[/]", ""),
        ] {
            assert_eq!(&line[body_line(line, Boxes::Plain)], plain, "{line:?}");
            assert_eq!(&line[body_line(line, Boxes::Status)], status, "{line:?}");
        }
        // An empty task of any status has no words, so the next line has.
        let text = "# Title\n- [/]\n- [界] \n- [-] Dropped\n";
        assert_eq!(&text[body_line(text, Boxes::Status)], "Dropped");
        assert_eq!(&text[body_line(text, Boxes::Plain)], "[/]");
        for (line, plain, status) in [
            ("- [/] Doing", "[/] Doing", "Doing"),
            ("- [界] x", "[界] x", "x"),
            ("- [-] **Dropped** `now`", "[-] Dropped now", "Dropped now"),
            ("- [x] Shipped", "Shipped", "Shipped"),
            // A task's description is its text, whatever it looks like.
            (
                "- [/] - [界] literal",
                "[/] - [界] literal",
                "- [界] literal",
            ),
            ("- [/] [界]", "[/] [界]", "[界]"),
            ("- [ ] - [x] words", "words", "- [x] words"),
            ("- [/] 1. words", "[/] 1. words", "1. words"),
            ("-\t[界]\tneedle", "[界] needle", "needle"),
            ("1.\t[é]\tneedle", "[é] needle", "needle"),
            // A line that is no task is read as it was.
            ("- - nested", "nested", "nested"),
            ("- 1. nested", "nested", "nested"),
        ] {
            let text = |boxes| Preview::new(line, NoteKind::Markdown, boxes).text;
            assert_eq!(&*text(Boxes::Plain), plain, "{line:?}");
            assert_eq!(&*text(Boxes::Status), status, "{line:?}");
        }
    }

    /// The snippet for the first `needle` in `text`, and what it highlights.
    fn shown(text: &str, needle: &str, boxes: Boxes) -> (String, Option<String>) {
        let hit = text.find(needle).expect("the needle is in the text");
        let result = snippet(text, hit..hit + needle.len(), NoteKind::Markdown, boxes);
        let highlight = result.highlight.map(|range| result.text[range].to_owned());
        (result.text, highlight)
    }

    /// With tasks on, a search snippet drops the block markup of a line the
    /// classifier reads as a task, and nothing else as a box; with tasks
    /// off it is what it was. `(text, needle, off, on)`; the needle is
    /// highlighted in both.
    #[test]
    fn snippets_drop_a_task_lines_markup_where_the_classifier_reads_a_task() {
        for (text, needle, off, on) in [
            (
                "## Agenda\n- [/] Draft the rollback plan\n- [界]\n1. [-] Old **idea**\n",
                "rollback",
                "Agenda · [/] Draft the rollback plan · [界] · [-] Old idea",
                "Agenda · Draft the rollback plan · Old idea",
            ),
            (
                "- [1] x-ray\n- [/] yes",
                "yes",
                "[1] x-ray · [/] yes",
                "x-ray · yes",
            ),
            // A task's description may itself look like a box, or like
            // another task: it is the task's text.
            ("- [/] [界]", "界", "[/] [界]", "[界]"),
            ("- [/] [2] second", "second", "[/] [2] second", "[2] second"),
            (
                "- [/] - [界] literal",
                "literal",
                "[/] - [界] literal",
                "- [界] literal",
            ),
            ("- [ ] - [x] words", "words", "words", "- [x] words"),
            // A tab after the marker or the box, as the editor reads it.
            ("-\t[界]\tneedle", "needle", "- [界] needle", "needle"),
            ("1.\t[é]\tneedle", "needle", "1. [é] needle", "needle"),
            (
                "- [界]\tx-ray\n1. [ ]\tyes",
                "yes",
                "[界] x-ray · [ ] yes",
                "x-ray · yes",
            ),
            (
                "> - [✓] quoted needle",
                "needle",
                "[✓] quoted needle",
                "quoted needle",
            ),
            // Lines that only look like tasks: `2.` cannot interrupt a
            // paragraph, a fence holds code, and a box needs a list marker.
            (
                "Intro\n2. [界] needle",
                "needle",
                "Intro · [界] needle",
                "Intro · [界] needle",
            ),
            (
                "```\n- [/] needle\n```",
                "needle",
                "[/] needle",
                "[/] needle",
            ),
            (
                "[1] Smith needle",
                "needle",
                "[1] Smith needle",
                "[1] Smith needle",
            ),
            (
                "> [a] quoted needle",
                "needle",
                "[a] quoted needle",
                "[a] quoted needle",
            ),
            (
                "- [a]: http://needle",
                "needle",
                "[a]: http://needle",
                "[a]: http://needle",
            ),
            (
                "- [✔\u{fe0f}] two needle",
                "needle",
                "[✔\u{fe0f}] two needle",
                "[✔\u{fe0f}] two needle",
            ),
            // The plain boxes, on a line that is no task, as before tasks.
            ("[x] bare needle", "needle", "bare needle", "bare needle"),
        ] {
            let highlighted = Some(needle.to_owned());
            assert_eq!(
                shown(text, needle, Boxes::Plain),
                (off.to_owned(), highlighted.clone()),
                "{text:?} off"
            );
            assert_eq!(
                shown(text, needle, Boxes::Status),
                (on.to_owned(), highlighted),
                "{text:?} on"
            );
        }
    }

    /// The context window may open or close inside a task line's markup:
    /// the box is found on the whole line, so what the window shows of it
    /// is dropped, and the hit keeps its place.
    #[test]
    fn a_snippet_window_that_cuts_a_task_lines_markup_shows_none_of_it() {
        let long = "a".repeat(42);
        let text = format!("- [界] {long} needle");
        assert_eq!(
            shown(&text, "needle", Boxes::Status),
            (format!("…{long} needle"), Some("needle".to_owned()))
        );
        assert_eq!(
            shown(&text, "needle", Boxes::Plain),
            (format!("…[界] {long} needle"), Some("needle".to_owned()))
        );
        // The window ends inside the next task's markup.
        let text = format!("- [/] needle {long}\n- [界] next");
        let (on, highlight) = shown(&text, "needle", Boxes::Status);
        assert!(
            on.starts_with("needle aaaa") && !on.contains(['[', '-']),
            "{on:?}"
        );
        assert_eq!(highlight.as_deref(), Some("needle"));
        // A hit in the box itself has nothing to highlight.
        assert_eq!(
            shown("- [界] words", "界", Boxes::Status),
            ("words".into(), None)
        );
        // Behind front matter and a byte order mark, with every line
        // ending; the front matter's own lines are shown as they were.
        for ending in ["\n", "\r\n", "\r", "\n\r"] {
            let lines = [
                "\u{feff}---",
                "title: x",
                "---",
                "- [/] one",
                "-\t[界]\ttwo needle",
            ];
            let (on, highlight) = shown(&lines.join(ending), "needle", Boxes::Status);
            assert!(on.ends_with("--- · one · two needle"), "{ending:?}: {on:?}");
            assert_eq!(highlight.as_deref(), Some("needle"), "{ending:?}");
        }
        // A box in front matter is no task's.
        let text = "---\nkey: v\n- [/] needle\n---\nbody";
        let on = shown(text, "needle", Boxes::Status);
        assert_eq!(on, shown(text, "needle", Boxes::Plain));
        assert!(on.0.contains("[/] needle"), "{on:?}");
        // No `[` in the window: nothing is classified, nothing changes.
        let text = "- plain item\n- another needle";
        assert_eq!(
            shown(text, "needle", Boxes::Status),
            shown(text, "needle", Boxes::Plain)
        );
        assert!(task_prefixes(text, 0..text.len(), Boxes::Status).is_empty());
        assert!(task_prefixes("- [/] x", 0..7, Boxes::Plain).is_empty());
        assert_eq!(
            task_prefixes("- [/] x\n- [界]", 0..15, Boxes::Status),
            [0..6, 8..15]
        );
    }

    /// The marker-only heuristic and the marker stripper know the three
    /// plain boxes only, with tasks on or off.
    #[test]
    fn the_block_marker_heuristics_read_only_the_plain_boxes() {
        for (line, markers) in [
            ("- [ ]", true),
            ("1. [x] ", true),
            ("- [/]", false),
            ("> - [界]", false),
        ] {
            assert_eq!(only_block_markers(line), markers, "{line:?}");
        }
        assert_eq!(strip_block_markers("- [x] words"), "words");
        assert_eq!(strip_block_markers("- [/] words"), "[/] words");
        assert_eq!(plain_markdown("- [ ]\tx\n- [x] y", true).0, "[ ] x · y");
    }

    /// The index reads the boxes it was built with, for the preview and
    /// for a search hit's snippet.
    #[test]
    fn an_index_built_with_status_boxes_previews_and_snippets_with_them() {
        let text = "# Tasks\n\n- [/] Doing the cutover\n- [界] Wide box\n";
        for (boxes, preview, snippet) in [
            (Boxes::Plain, "[/] Doing the cutover", "[界] Wide box"),
            (Boxes::Status, "Doing the cutover", "Wide box"),
        ] {
            let mut index = Index::with_boxes(boxes);
            index.sync_metadata(&workspace());
            index.set_content(NoteId(4), text, false);
            assert_eq!(index.preview(NoteId(4)), Some(preview), "{boxes:?}");
            let hit = &index.query("wide", Scope::Workspace, QueryKind::Search, 10)[0];
            assert!(
                hit.snippet.text.ends_with(snippet),
                "{:?}",
                hit.snippet.text
            );
            assert_eq!(
                &hit.snippet.text[hit.snippet.highlight.clone().unwrap()],
                "Wide"
            );
        }
        assert_eq!(Index::default().boxes, Boxes::Plain);
    }

    #[test]
    fn previews_skip_front_matter_code_fences_and_multi_line_setext_headings() {
        for (text, expected) in [
            ("---\ntitle: Notes\n---\nBody", "Body"),
            ("Two line\nheading\n---\nBody", "Body"),
            ("```\n\n```\nAfter", "After"),
            ("~~~\ncode\n~~~\nAfter", "After"),
            ("Para\n\n---\n", "Para"),
        ] {
            assert_eq!(&text[body_line(text, Boxes::Plain)], expected, "{text:?}");
        }
    }

    #[test]
    fn previews_read_only_the_lines_before_the_first_body_line() {
        // A paragraph is read to its end, since a setext underline below it
        // would make the whole paragraph a heading; nothing past it is read.
        let text = format!("# Title\nFirst words\n\n{}", "more prose\n".repeat(100_000));
        let touched = std::cell::Cell::new(0);
        let lines = wren_markdown::line_ranges(&text).map(|(line, _)| {
            touched.set(touched.get() + 1);
            line
        });
        assert_eq!(
            &text[body_line_in(&text, lines, Boxes::Plain).0],
            "First words"
        );
        assert!(touched.get() <= 4, "read {} lines", touched.get());
    }

    #[test]
    fn lazy_previews_read_front_matter_like_the_whole_document_scan() {
        fn whole(text: &str) -> Range<usize> {
            use wren_markdown::{Document, Lines as _};
            let document = Document::new(text);
            let mut scanner =
                Scanner::with_front_matter(block::front_matter_end(&document), Boxes::Plain);
            let mut candidate: Option<(usize, Range<usize>)> = None;
            for index in 0..document.line_count() {
                let line = document.line(index);
                let info = scanner.next_line(line);
                match info.kind {
                    Kind::Prose => {
                        if candidate.is_none() {
                            let start = document.line_range(index).start + info.text_start;
                            let words = line[info.text_start..].trim_end();
                            candidate = Some((index, start..start + words.len()));
                        }
                    }
                    Kind::SetextUnderline { start, .. } => match candidate {
                        Some((line, _)) if line >= start => candidate = None,
                        Some((_, range)) => return range,
                        None => {}
                    },
                    _ => {
                        if let Some((_, range)) = candidate {
                            return range;
                        }
                    }
                }
            }
            candidate.map_or(0..0, |(_, range)| range)
        }
        for text in [
            "",
            "---",
            "---\n",
            "---\n---\nBody",
            "---\n\n---\nBody",
            "---\nkey: v\n---\nBody",
            "---\r\nkey: v\r\n...\r\nBody",
            "\u{feff}---  \nkey: v\n---\t\nBody",
            "---\n\n\ntitle: x\n\n---\nBody",
            "---\nnot yaml\n---\nBody",
            "---\nkey: v\nBody never closed",
            "---\n# comment\n...\n\nBody",
            "---\nkey: v\n--- \u{a0}\nstill front\n---\nBody",
            "Para\n---\nBody",
        ] {
            assert_eq!(body_line(text, Boxes::Plain), whole(text), "{text:?}");
        }
    }

    #[test]
    fn normal_previews_use_the_first_body_line_without_changing_literal_hit_snippets() {
        let mut index = index();
        for separator in ["\n", "\r\n", "\r"] {
            let text = [
                "",
                "# Title",
                "",
                "Section title",
                "===",
                "## Subheading",
                "",
                "  First café body  ",
                "Later body",
            ]
            .join(separator);
            index.set_content(NoteId(4), &text, false);
            assert_eq!(index.preview(NoteId(4)), Some("First café body"));
            let hit = &index.query("Note 2", Scope::Workspace, QueryKind::Jump, 10)[0];
            assert_eq!(hit.snippet.text, "First café body");
            assert_eq!(hit.snippet.highlight, None);
            let literal = &index.query("Title", Scope::Workspace, QueryKind::Search, 10)[0];
            assert_eq!(
                &literal.snippet.text[literal.snippet.highlight.clone().unwrap()],
                "Title"
            );
        }
        for (text, preview) in [
            ("# Only heading\n\n## Another", ""),
            ("#hashtag is body\nnext", "#hashtag is body"),
            ("Body\n\n---\nnext", "Body"),
            ("   \r\n", ""),
        ] {
            index.set_content(NoteId(4), text, false);
            assert_eq!(index.preview(NoteId(4)), Some(preview));
        }
        let text = format!("# Heading\n\n{}\nSecond", "界".repeat(200));
        index.set_content(NoteId(4), &text, false);
        assert_eq!(
            index.preview(NoteId(4)).unwrap().chars().count(),
            PREVIEW_CHARS
        );
        let snippet = Preview::new(&text, NoteKind::Markdown, Boxes::Plain).snippet();
        assert!(snippet.text.ends_with('…'));
        assert_eq!(snippet.text.chars().count(), PREVIEW_CHARS + 1);
    }

    #[test]
    #[ignore = "release performance benchmark: 5000 notes / 25 MB; run explicitly with --release"]
    fn benchmark_search_5000_notes_25mb_under_30ms() {
        let mut workspace = workspace();
        workspace.notes.clear();
        let mut index = Index::default();
        for number in 0..5000 {
            let title = format!("Meeting {number}");
            workspace.notes.push(Note {
                id: NoteId(number + 10),
                group: GroupId(3),
                title: title.clone(),
                kind: NoteKind::Markdown,
                path: RelPath::new(format!("A/G/{title}.md")).unwrap(),
                subtitle: None,
                modified: SystemTime::UNIX_EPOCH,
                bytes: 5000,
            });
        }
        let started = Instant::now();
        index.sync_metadata(&workspace);
        for number in 0..5000 {
            let mut text = "Ordinary work notes, migration planning, follow-up items. ".repeat(100);
            text.truncate(5000);
            if number % 100 == 0 {
                text.replace_range(4200..4212, "rare needle!");
            }
            index.set_content(NoteId(number + 10), &text, false);
        }
        eprintln!(
            "in-memory build: {:?}; notes: {}; original bytes: {}; retained bytes: {}",
            started.elapsed(),
            index.len(),
            workspace.notes.iter().map(|note| note.bytes).sum::<usize>(),
            index.retained_bytes()
        );
        for (query, kind, expected) in [
            ("mtg42", QueryKind::Jump, 40),
            ("rare needle!", QueryKind::Search, 40),
            ("follow-up", QueryKind::Filter, 40),
            ("no such phrase", QueryKind::Search, 0),
        ] {
            let mut timings = Vec::new();
            for _ in 0..20 {
                let started = Instant::now();
                let hits = std::hint::black_box(&index).query(
                    std::hint::black_box(query),
                    Scope::Workspace,
                    kind,
                    40,
                );
                timings.push(started.elapsed());
                assert_eq!(hits.len(), expected);
                if kind == QueryKind::Jump {
                    assert!(hits.iter().any(|hit| hit.title == "Meeting 42"));
                    assert!(hits
                        .iter()
                        .all(|hit| hit.score > 0 && hit.content_match.is_none()));
                    assert!(hits.windows(2).all(|pair| pair[0].score >= pair[1].score));
                } else {
                    for hit in &hits {
                        assert_eq!(
                            &hit.snippet.text[hit.snippet.highlight.clone().unwrap()],
                            query
                        );
                        assert_eq!(
                            &index.content(hit.note).unwrap()[hit.content_match.clone().unwrap()],
                            query
                        );
                    }
                }
                std::hint::black_box(hits);
            }
            timings.sort();
            eprintln!(
                "{kind:?} {query:?}: median {:?}, max {:?}",
                timings[10], timings[19]
            );
            assert!(
                timings[19].as_millis() < 30,
                "{kind:?} {query:?}: {:?}",
                timings[19]
            );
        }
    }
}
