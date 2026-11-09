//! Session-owned, in-memory search. Queries never access the filesystem.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;
use std::time::SystemTime;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32Str};
use wren_markdown::block::{self, Kind, Scanner};

use crate::fs_store::{DiskFile, FsStore, RelPath};
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
const MAX_QUEUED_WARNINGS: usize = 64;

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
    // Shared with background reconciliation snapshots, which compare exact bytes.
    text: Arc<str>,
    folded: Box<str>,
    preview: Preview,
}

impl Content {
    fn bytes(&self) -> usize {
        self.text.len() + self.folded.len()
    }
}

#[derive(Debug)]
struct Entry {
    title: String,
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
    warnings: Vec<String>,
}

impl Default for Index {
    fn default() -> Self {
        Self::with_budget(MAX_CONTENT_BYTES)
    }
}

impl Index {
    fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
            retained_bytes: 0,
            budget,
            sequence: 0,
            revision: 0,
            warnings: Vec::new(),
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

    /// Retained original and lowercase content bytes, excluding title metadata.
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub fn preview(&self, note: NoteId) -> Option<&str> {
        let content = self.entries.get(&note)?.content.as_ref()?;
        Some(&content.text[content.preview.range.clone()])
    }

    pub fn content(&self, note: NoteId) -> Option<&str> {
        Some(&self.entries.get(&note)?.content.as_ref()?.text)
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
                    .and_then(|content| content.folded.find(&folded_query))
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
                let content_match = start.and_then(|start| {
                    let content = entry.content.as_ref()?;
                    Some(original_range(
                        &content.text,
                        start..start + folded_query.len(),
                    ))
                });
                let snippet = match &entry.content {
                    Some(content) => match &content_match {
                        Some(hit) => snippet(&content.text, hit.clone()),
                        None => content.preview.snippet(&content.text),
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
                Ok(text) => self.set_content(note.id, text, false),
                Err(_) => self.unavailable(note.id, NOT_UTF8),
            },
            Err(reason) => self.unavailable(note.id, reason),
        }
    }

    /// Exact retained text per note, shared rather than copied, so a background
    /// scan can send back only notes whose disk bytes differ.
    pub(crate) fn content_snapshot(&self) -> HashMap<NoteId, Arc<str>> {
        self.entries
            .iter()
            .filter_map(|(id, entry)| Some((*id, Arc::clone(&entry.content.as_ref()?.text))))
            .collect()
    }

    /// Applies one background-scan content result to a note already present
    /// after metadata synchronization; unknown notes are ignored.
    pub(crate) fn ingest_scanned(&mut self, note: NoteId, contents: Result<&str, &str>) {
        if !self.entries.contains_key(&note) {
            return;
        }
        match contents {
            Ok(text) => self.set_content(note, text, false),
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

    pub(crate) fn set_content(&mut self, note: NoteId, text: &str, priority: bool) {
        let Some(entry) = self.entries.get(&note) else {
            return;
        };
        if entry
            .content
            .as_ref()
            .is_some_and(|content| &*content.text == text)
        {
            return;
        }
        self.drop_content(note);
        let folded_bytes: usize = if text.is_ascii() {
            text.len()
        } else {
            text.chars()
                .flat_map(char::to_lowercase)
                .map(char::len_utf8)
                .sum()
        };
        let bytes = text.len().saturating_add(folded_bytes);
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
        let folded = if text.is_ascii() {
            text.to_ascii_lowercase()
        } else {
            // Size before allocating: lowercase expansions must not cause an
            // oversized staging buffer or allocate for content we cannot keep.
            let mut folded = String::with_capacity(folded_bytes);
            folded.extend(text.chars().flat_map(char::to_lowercase));
            folded
        };
        let content = Content {
            text: text.into(),
            folded: folded.into_boxed_str(),
            preview: Preview::new(text),
        };
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
fn body_line(text: &str) -> Range<usize> {
    body_line_in(text, wren_markdown::line_ranges(text).map(|(line, _)| line))
}

/// [`body_line`] over `text`'s lines, read lazily: it stops at the first
/// body line, so a long note costs only the lines before it.
fn body_line_in(text: &str, mut lines: impl Iterator<Item = Range<usize>>) -> Range<usize> {
    let Some(first) = lines.next() else {
        return 0..0;
    };
    // Front matter is read only when line 0 opens it, and then only to the
    // first `---` or `...` line: `front_matter_end` decides by that line.
    let delimiter = |line: &Range<usize>| text[line.clone()].trim_end_matches([' ', '\t']);
    let mut head = vec![first];
    let first = delimiter(&head[0]);
    if first.strip_prefix('\u{feff}').unwrap_or(first) == "---" {
        for line in lines.by_ref() {
            let closes = matches!(delimiter(&line), "---" | "...");
            head.push(line);
            if closes {
                break;
            }
        }
    }
    let head_lines: Vec<&str> = head.iter().map(|line| &text[line.clone()]).collect();
    let mut scanner = Scanner::with_front_matter(block::front_matter_end(&head_lines[..]));
    let mut candidate: Option<(usize, Range<usize>)> = None;
    for (index, range) in head.into_iter().chain(lines).enumerate() {
        let line = &text[range.clone()];
        let info = scanner.next_line(line);
        match info.kind {
            Kind::Prose => {
                if candidate.is_none() {
                    let start = range.start + info.text_start;
                    let words = line[info.text_start..].trim_end();
                    candidate = Some((index, start..start + words.len()));
                }
            }
            // Paragraph lines from `start` on were heading text.
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

/// A note's preview: the first body line, clipped to [`PREVIEW_CHARS`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Preview {
    range: Range<usize>,
    truncated: bool,
}

impl Preview {
    fn new(text: &str) -> Self {
        let line = body_line(text);
        let end = text[line.clone()]
            .char_indices()
            .nth(PREVIEW_CHARS)
            .map_or(line.end, |(byte, _)| line.start + byte);
        Self {
            truncated: end < line.end,
            range: line.start..end,
        }
    }

    fn snippet(&self, text: &str) -> Snippet {
        let mut preview = text[self.range.clone()].to_owned();
        if self.truncated {
            preview.push('…');
        }
        Snippet {
            text: preview,
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

/// A search hit's context: the hit with up to [`CONTEXT_CHARS`] on each side.
fn snippet(text: &str, hit: Range<usize>) -> Snippet {
    let start = text[..hit.start]
        .char_indices()
        .rev()
        .nth(CONTEXT_CHARS - 1)
        .map_or(0, |(byte, _)| byte);
    let end = text[hit.end..]
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(text.len(), |(byte, _)| hit.end + byte);
    let mut result = String::new();
    if start != 0 {
        result.push('…');
    }
    let prefix = result.len();
    result.push_str(&text[start..end]);
    if end != text.len() {
        result.push('…');
    }
    Snippet {
        text: result,
        highlight: Some(prefix + hit.start - start..prefix + hit.end - start),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

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
                title: model::note_title(RelPath::new(path).unwrap().file_name()),
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
    fn snippets_are_utf8_safe_near_both_ends_and_in_long_content() {
        for text in ["é日本", "", "a"] {
            let result = Preview::new(text).snippet(text);
            assert_eq!(result.text, text);
            assert!(result.highlight.is_none());
        }
        let text = format!("{}Needle{}", "界".repeat(200), "é".repeat(200));
        let result = snippet(&text, 600..606);
        assert!(result.text.starts_with('…') && result.text.ends_with('…'));
        assert_eq!(&result.text[result.highlight.unwrap()], "Needle");
        for hit in [0..3, text.len() - 2..text.len()] {
            let result = snippet(&text, hit.clone());
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
                assert_eq!(
                    &hit.snippet.text[hit.snippet.highlight.clone().unwrap()],
                    matched
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
            assert_eq!(&line[body_line(line)], expected, "{line:?}");
        }
        let text = "# Title\n\n- First item\n";
        assert_eq!(&text[body_line(text)], "First item");
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
            assert_eq!(&text[body_line(text)], expected, "{text:?}");
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
        assert_eq!(&text[body_line_in(&text, lines)], "First words");
        assert!(touched.get() <= 4, "read {} lines", touched.get());
    }

    #[test]
    fn lazy_previews_read_front_matter_like_the_whole_document_scan() {
        fn whole(text: &str) -> Range<usize> {
            use wren_markdown::{Document, Lines as _};
            let document = Document::new(text);
            let mut scanner = Scanner::with_front_matter(block::front_matter_end(&document));
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
            assert_eq!(body_line(text), whole(text), "{text:?}");
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
        let snippet = Preview::new(&text).snippet(&text);
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
