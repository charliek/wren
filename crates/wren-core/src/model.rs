use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::fs_store::RelPath;

// ---------------------------------------------------------------------------
// ID types — shared monotonic counter, never reused within a session
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionId(pub u64);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(pub u64);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId(pub u64);

// ---------------------------------------------------------------------------
// Parent / ItemRef
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Parent {
    Section(SectionId),
    Group(GroupId),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemRef {
    Section(SectionId),
    Group(GroupId),
    Note(NoteId),
}

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Section {
    pub id: SectionId,
    pub name: String,
    pub path: RelPath,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub id: GroupId,
    pub section: SectionId,
    pub name: String,
    pub path: RelPath,
}

#[derive(Clone, Debug)]
pub struct Note {
    pub id: NoteId,
    pub group: GroupId,
    pub title: String,
    pub path: RelPath,
    pub subtitle: Option<String>,
    pub modified: SystemTime,
    pub bytes: usize,
}

#[derive(Clone, Debug)]
pub struct ScanWarning {
    pub path: PathBuf,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

pub struct Workspace {
    pub root: PathBuf,
    pub sections: Vec<Section>,
    pub groups: Vec<Group>,
    pub notes: Vec<Note>,
    pub warnings: Vec<ScanWarning>,
    next_id: u64,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("root", &self.root)
            .field("sections", &self.sections.len())
            .field("groups", &self.groups.len())
            .field("notes", &self.notes.len())
            .field("warnings", &self.warnings.len())
            .finish()
    }
}

impl Workspace {
    pub fn empty(root: PathBuf) -> Self {
        Self {
            root,
            sections: Vec::new(),
            groups: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
            next_id: 1,
        }
    }

    pub(crate) fn with_next_id(root: PathBuf, next_id: u64) -> Self {
        Self {
            root,
            sections: Vec::new(),
            groups: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
            next_id,
        }
    }

    pub fn section(&self, id: SectionId) -> Option<&Section> {
        self.sections.iter().find(|s| s.id == id)
    }

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn note(&self, id: NoteId) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn path(&self, item: ItemRef) -> Option<&RelPath> {
        match item {
            ItemRef::Section(id) => self.section(id).map(|s| &s.path),
            ItemRef::Group(id) => self.group(id).map(|g| &g.path),
            ItemRef::Note(id) => self.note(id).map(|n| &n.path),
        }
    }

    pub fn parent_path(&self, parent: Parent) -> Option<&RelPath> {
        match parent {
            Parent::Section(id) => self.section(id).map(|s| &s.path),
            Parent::Group(id) => self.group(id).map(|g| &g.path),
        }
    }

    pub(crate) fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) fn next_id(&self) -> u64 {
        self.next_id
    }

    /// The entries and ID allocator a background scan needs to preserve IDs
    /// and carry unreadable entries forward. Warnings are not copied.
    pub(crate) fn scan_baseline(&self) -> Self {
        Self {
            root: self.root.clone(),
            sections: self.sections.clone(),
            groups: self.groups.clone(),
            notes: self.notes.clone(),
            warnings: Vec::new(),
            next_id: self.next_id,
        }
    }

    /// Update every path that has `from` as a prefix, replacing that prefix
    /// with `to`. Refreshes derived fields (section/group name, note title,
    /// note subtitle) from the updated paths. IDs are preserved.
    pub fn remap_paths(&mut self, from: &RelPath, to: &RelPath) {
        let from_path = from.as_path();
        let to_path = to.as_path();

        for section in &mut self.sections {
            if let Some(new) = remap_one(section.path.as_path(), from_path, to_path) {
                if let Ok(rp) = RelPath::new(&new) {
                    section.name = dir_name(&rp);
                    section.path = rp;
                }
            }
        }

        for group in &mut self.groups {
            if let Some(new) = remap_one(group.path.as_path(), from_path, to_path) {
                if let Ok(rp) = RelPath::new(&new) {
                    group.name = dir_name(&rp);
                    group.path = rp;
                }
            }
        }

        let group_paths: HashMap<GroupId, PathBuf> = self
            .groups
            .iter()
            .map(|g| (g.id, g.path.as_path().to_path_buf()))
            .collect();

        for note in &mut self.notes {
            if let Some(new) = remap_one(note.path.as_path(), from_path, to_path) {
                if let Ok(rp) = RelPath::new(&new) {
                    note.title = note_title(rp.file_name());
                    note.path = rp;
                    note.subtitle = group_paths
                        .get(&note.group)
                        .and_then(|gp| compute_subtitle(note.path.as_path(), gp));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers (pub(crate) for use by scan)
// ---------------------------------------------------------------------------

/// Extract a note title from its filename, stripping a trailing `.md`
/// (case-insensitive).
pub(crate) fn note_title(file_name: &OsStr) -> String {
    let name = file_name.to_string_lossy();
    if name.len() > 3
        && name
            .get(name.len() - 3..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".md"))
    {
        name[..name.len() - 3].to_string()
    } else {
        name.into_owned()
    }
}

/// Directory display name: the last path component.
pub(crate) fn dir_name(rel: &RelPath) -> String {
    rel.file_name().to_string_lossy().into_owned()
}

/// Compute the subtitle for a note that lives deeper than its parent group.
/// Returns `None` when the note sits directly inside the group directory.
pub(crate) fn compute_subtitle(note_path: &Path, group_path: &Path) -> Option<String> {
    let rel = note_path.strip_prefix(group_path).ok()?;
    let parent = rel.parent()?;
    if parent.as_os_str().is_empty() {
        None
    } else {
        Some(rel.to_string_lossy().into_owned())
    }
}

/// Natural alphabetical comparison with ASCII case-folding.
///
/// Numeric runs are compared by value so `"Note 2"` sorts before `"Note 10"`.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.as_bytes().iter().peekable();
    let mut bi = b.as_bytes().iter().peekable();
    loop {
        match (ai.peek(), bi.peek()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&&ab), Some(&&bb)) => {
                if ab.is_ascii_digit() && bb.is_ascii_digit() {
                    let an = consume_number(&mut ai);
                    let bn = consume_number(&mut bi);
                    match an.cmp(&bn) {
                        Ordering::Equal => {}
                        ord => return ord,
                    }
                } else {
                    let af = ab.to_ascii_lowercase();
                    let bf = bb.to_ascii_lowercase();
                    match af.cmp(&bf) {
                        Ordering::Equal => {
                            ai.next();
                            bi.next();
                        }
                        ord => return ord,
                    }
                }
            }
        }
    }
}

fn consume_number<'a, I: Iterator<Item = &'a u8>>(iter: &mut std::iter::Peekable<I>) -> u64 {
    let mut n: u64 = 0;
    while let Some(&&b) = iter.peek() {
        if b.is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add(u64::from(b - b'0'));
            iter.next();
        } else {
            break;
        }
    }
    n
}

fn remap_one(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    if path == from {
        Some(to.to_path_buf())
    } else {
        path.strip_prefix(from).ok().map(|suffix| to.join(suffix))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    // -- natural_cmp --

    #[test]
    fn natural_cmp_basic_alphabetical() {
        assert_eq!(natural_cmp("abc", "def"), Ordering::Less);
        assert_eq!(natural_cmp("def", "abc"), Ordering::Greater);
        assert_eq!(natural_cmp("abc", "abc"), Ordering::Equal);
    }

    #[test]
    fn natural_cmp_case_insensitive() {
        assert_eq!(natural_cmp("ABC", "abc"), Ordering::Equal);
        assert_eq!(natural_cmp("Alice", "bob"), Ordering::Less);
        assert_eq!(natural_cmp("INBOX", "inbox"), Ordering::Equal);
    }

    #[test]
    fn natural_cmp_numeric_runs() {
        assert_eq!(natural_cmp("Note 2", "Note 10"), Ordering::Less);
        assert_eq!(natural_cmp("File 100", "File 20"), Ordering::Greater);
        assert_eq!(natural_cmp("v1", "v1"), Ordering::Equal);
        assert_eq!(natural_cmp("ch1", "ch02"), Ordering::Less);
    }

    #[test]
    fn natural_cmp_mixed_digit_nondigit() {
        assert!(natural_cmp("1a", "a1") == Ordering::Less);
        assert!(natural_cmp("a", "1") == Ordering::Greater);
    }

    #[test]
    fn natural_cmp_prefix() {
        assert_eq!(natural_cmp("ab", "abc"), Ordering::Less);
        assert_eq!(natural_cmp("abc", "ab"), Ordering::Greater);
    }

    #[test]
    fn natural_cmp_empty() {
        assert_eq!(natural_cmp("", ""), Ordering::Equal);
        assert_eq!(natural_cmp("", "a"), Ordering::Less);
        assert_eq!(natural_cmp("a", ""), Ordering::Greater);
    }

    // -- note_title --

    #[test]
    fn note_title_strips_md_extension() {
        assert_eq!(note_title(OsStr::new("design.md")), "design");
        assert_eq!(note_title(OsStr::new("Design.MD")), "Design");
        assert_eq!(note_title(OsStr::new("notes.Md")), "notes");
        assert_eq!(note_title(OsStr::new("UPPER.mD")), "UPPER");
    }

    #[test]
    fn note_title_preserves_non_md() {
        assert_eq!(note_title(OsStr::new("file.txt")), "file.txt");
        assert_eq!(note_title(OsStr::new("readme")), "readme");
        assert_eq!(note_title(OsStr::new("data.json")), "data.json");
    }

    #[test]
    fn note_title_edge_cases() {
        assert_eq!(note_title(OsStr::new("a.md")), "a");
        assert_eq!(note_title(OsStr::new(".md")), ".md");
        assert_eq!(note_title(OsStr::new("x.md.md")), "x.md");
    }

    #[test]
    fn note_title_unicode_stem() {
        assert_eq!(note_title(OsStr::new("café.md")), "café");
        assert_eq!(note_title(OsStr::new("日本語.md")), "日本語");
        assert_eq!(note_title(OsStr::new("🎵.md")), "🎵");
    }

    #[test]
    fn note_title_preserves_unicode_without_md_extension() {
        for name in ["\u{1f426}", "café", "日本語", "éé", "é.M"] {
            assert_eq!(note_title(OsStr::new(name)), name);
        }
    }

    // -- compute_subtitle --

    #[test]
    fn subtitle_direct_child_is_none() {
        assert_eq!(
            compute_subtitle(Path::new("sec/grp/note.md"), Path::new("sec/grp")),
            None,
        );
    }

    #[test]
    fn subtitle_one_level_deep() {
        assert_eq!(
            compute_subtitle(Path::new("sec/grp/sub/note.md"), Path::new("sec/grp")),
            Some("sub/note.md".into()),
        );
    }

    #[test]
    fn subtitle_multi_level_deep() {
        assert_eq!(
            compute_subtitle(Path::new("sec/grp/a/b/c.md"), Path::new("sec/grp")),
            Some("a/b/c.md".into()),
        );
    }

    #[test]
    fn subtitle_unrelated_paths() {
        assert_eq!(
            compute_subtitle(Path::new("other/note.md"), Path::new("sec/grp")),
            None,
        );
    }

    // -- remap_one --

    #[test]
    fn remap_exact() {
        assert_eq!(
            remap_one(
                Path::new("Projects"),
                Path::new("Projects"),
                Path::new("Work")
            ),
            Some(PathBuf::from("Work")),
        );
    }

    #[test]
    fn remap_descendant() {
        assert_eq!(
            remap_one(
                Path::new("Projects/WebApp/design.md"),
                Path::new("Projects"),
                Path::new("Work"),
            ),
            Some(PathBuf::from("Work/WebApp/design.md")),
        );
    }

    #[test]
    fn remap_no_match() {
        assert_eq!(
            remap_one(
                Path::new("Inbox/note.md"),
                Path::new("Projects"),
                Path::new("Work"),
            ),
            None,
        );
    }

    // -- Workspace --

    #[test]
    fn workspace_empty_is_clean() {
        let ws = Workspace::empty(PathBuf::from("/tmp/notes"));
        assert_eq!(ws.root, PathBuf::from("/tmp/notes"));
        assert!(ws.sections.is_empty());
        assert!(ws.groups.is_empty());
        assert!(ws.notes.is_empty());
        assert!(ws.warnings.is_empty());
    }

    #[test]
    fn workspace_lookup_returns_none_for_missing() {
        let ws = Workspace::empty(PathBuf::from("/tmp"));
        assert!(ws.section(SectionId(99)).is_none());
        assert!(ws.group(GroupId(99)).is_none());
        assert!(ws.note(NoteId(99)).is_none());
        assert!(ws.path(ItemRef::Section(SectionId(99))).is_none());
        assert!(ws.parent_path(Parent::Section(SectionId(99))).is_none());
    }

    #[test]
    fn allocate_id_monotonic_no_reuse() {
        let mut ws = Workspace::empty(PathBuf::from("/tmp"));
        let a = ws.allocate_id();
        let b = ws.allocate_id();
        let c = ws.allocate_id();
        assert!(a < b && b < c);
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(c, 3);
    }

    #[test]
    fn workspace_lookups_return_correct_entries() {
        let mut ws = Workspace::empty(PathBuf::from("/tmp"));
        let sid = SectionId(ws.allocate_id());
        let gid = GroupId(ws.allocate_id());
        let nid = NoteId(ws.allocate_id());

        ws.sections.push(Section {
            id: sid,
            name: "Projects".into(),
            path: rel("Projects"),
        });
        ws.groups.push(Group {
            id: gid,
            section: sid,
            name: "WebApp".into(),
            path: rel("Projects/WebApp"),
        });
        ws.notes.push(Note {
            id: nid,
            group: gid,
            title: "design".into(),
            path: rel("Projects/WebApp/design.md"),
            subtitle: None,
            modified: SystemTime::UNIX_EPOCH,
            bytes: 42,
        });

        assert_eq!(ws.section(sid).unwrap().name, "Projects");
        assert_eq!(ws.group(gid).unwrap().name, "WebApp");
        assert_eq!(ws.note(nid).unwrap().title, "design");

        assert_eq!(
            ws.path(ItemRef::Section(sid)).unwrap().as_path(),
            Path::new("Projects"),
        );
        assert_eq!(
            ws.path(ItemRef::Group(gid)).unwrap().as_path(),
            Path::new("Projects/WebApp"),
        );
        assert_eq!(
            ws.path(ItemRef::Note(nid)).unwrap().as_path(),
            Path::new("Projects/WebApp/design.md"),
        );
        assert_eq!(
            ws.parent_path(Parent::Section(sid)).unwrap().as_path(),
            Path::new("Projects"),
        );
        assert_eq!(
            ws.parent_path(Parent::Group(gid)).unwrap().as_path(),
            Path::new("Projects/WebApp"),
        );
    }

    // -- remap_paths on Workspace --

    fn populated_workspace() -> Workspace {
        let mut ws = Workspace::empty(PathBuf::from("/tmp"));
        let sid = SectionId(ws.allocate_id());
        let gid = GroupId(ws.allocate_id());
        let nid1 = NoteId(ws.allocate_id());
        let nid2 = NoteId(ws.allocate_id());

        ws.sections.push(Section {
            id: sid,
            name: "Projects".into(),
            path: rel("Projects"),
        });
        ws.groups.push(Group {
            id: gid,
            section: sid,
            name: "WebApp".into(),
            path: rel("Projects/WebApp"),
        });
        ws.notes.push(Note {
            id: nid1,
            group: gid,
            title: "design".into(),
            path: rel("Projects/WebApp/design.md"),
            subtitle: None,
            modified: SystemTime::UNIX_EPOCH,
            bytes: 42,
        });
        ws.notes.push(Note {
            id: nid2,
            group: gid,
            title: "arch".into(),
            path: rel("Projects/WebApp/docs/arch.md"),
            subtitle: Some("docs/arch.md".into()),
            modified: SystemTime::UNIX_EPOCH,
            bytes: 100,
        });
        ws
    }

    #[test]
    fn remap_section_rename_updates_all_descendants() {
        let mut ws = populated_workspace();
        let sid = ws.sections[0].id;
        let gid = ws.groups[0].id;
        let nid1 = ws.notes[0].id;
        let nid2 = ws.notes[1].id;

        ws.remap_paths(&rel("Projects"), &rel("Work"));

        assert_eq!(ws.sections[0].id, sid);
        assert_eq!(ws.sections[0].name, "Work");
        assert_eq!(ws.sections[0].path.as_path(), Path::new("Work"));

        assert_eq!(ws.groups[0].id, gid);
        assert_eq!(ws.groups[0].name, "WebApp");
        assert_eq!(ws.groups[0].path.as_path(), Path::new("Work/WebApp"));

        assert_eq!(ws.notes[0].id, nid1);
        assert_eq!(ws.notes[0].title, "design");
        assert_eq!(
            ws.notes[0].path.as_path(),
            Path::new("Work/WebApp/design.md"),
        );
        assert_eq!(ws.notes[0].subtitle, None);

        assert_eq!(ws.notes[1].id, nid2);
        assert_eq!(
            ws.notes[1].path.as_path(),
            Path::new("Work/WebApp/docs/arch.md"),
        );
        assert_eq!(ws.notes[1].subtitle.as_deref(), Some("docs/arch.md"));
    }

    #[test]
    fn remap_group_rename() {
        let mut ws = populated_workspace();
        let gid = ws.groups[0].id;

        ws.remap_paths(&rel("Projects/WebApp"), &rel("Projects/Frontend"));

        assert_eq!(ws.groups[0].id, gid);
        assert_eq!(ws.groups[0].name, "Frontend");
        assert_eq!(ws.groups[0].path.as_path(), Path::new("Projects/Frontend"),);

        assert_eq!(
            ws.notes[0].path.as_path(),
            Path::new("Projects/Frontend/design.md"),
        );
        assert_eq!(
            ws.notes[1].path.as_path(),
            Path::new("Projects/Frontend/docs/arch.md"),
        );
    }

    #[test]
    fn remap_unrelated_prefix_leaves_entries_unchanged() {
        let mut ws = populated_workspace();
        ws.remap_paths(&rel("Inbox"), &rel("Archive"));

        assert_eq!(ws.sections[0].name, "Projects");
        assert_eq!(ws.sections[0].path.as_path(), Path::new("Projects"));
        assert_eq!(
            ws.notes[0].path.as_path(),
            Path::new("Projects/WebApp/design.md")
        );
    }

    #[test]
    fn next_id_preserved_via_with_next_id() {
        let ws1 = Workspace::with_next_id(PathBuf::from("/tmp"), 42);
        assert_eq!(ws1.next_id(), 42);

        let mut ws2 = Workspace::with_next_id(PathBuf::from("/tmp"), ws1.next_id());
        let id = ws2.allocate_id();
        assert_eq!(id, 42);
        assert_eq!(ws2.next_id(), 43);
    }
}
