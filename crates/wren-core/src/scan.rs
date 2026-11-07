use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::fs_store::{DirectoryListing, DiskFile, EntryKind, FsStore, RelPath, StoreError};
use crate::model::{
    compute_subtitle, natural_cmp, note_title, Group, GroupId, Note, NoteId, Parent, ScanWarning,
    Section, SectionId, Workspace,
};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ScanError {
    RootUnreadable(std::io::Error),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::RootUnreadable(e) => write!(f, "cannot read workspace root: {e}"),
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ScanError::RootUnreadable(e) => Some(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Walk the workspace directory through `store` and produce a [`Workspace`].
///
/// When `previous` is provided, IDs are preserved for entries whose paths
/// have not changed, and entries under unreadable subtrees are carried
/// forward rather than silently dropped.
pub fn scan(store: &FsStore, previous: Option<&Workspace>) -> Result<Workspace, ScanError> {
    scan_with_contents(store, previous, |_, _| {})
}

/// Streams each accepted note's exact descriptor-relative read to `visit`.
/// The borrowed file is released before the next read; no vault-sized content
/// staging map is retained. Preserved unreadable/unseen notes yield `Err` rather
/// than stale contents. A root error happens before any visitor calls.
///
/// Callers retaining an active draft must ignore its disk content and finish by
/// synchronizing against the returned model (which owns the accepted IDs).
#[allow(clippy::type_complexity)]
pub fn scan_with_contents(
    store: &FsStore,
    previous: Option<&Workspace>,
    mut visit: impl FnMut(&Note, Result<&DiskFile, &str>),
) -> Result<Workspace, ScanError> {
    scan_with_fs_contents(store, previous, &RealScanFs, &mut visit)
}

type ContentsVisitor<'a> = dyn FnMut(&Note, Result<&DiskFile, &str>) + 'a;

#[cfg(test)]
fn scan_with_fs(
    store: &FsStore,
    previous: Option<&Workspace>,
    scan_fs: &impl ScanFs,
) -> Result<Workspace, ScanError> {
    scan_with_fs_contents(store, previous, scan_fs, &mut |_, _| {})
}

fn scan_with_fs_contents(
    store: &FsStore,
    previous: Option<&Workspace>,
    scan_fs: &impl ScanFs,
    visit: &mut ContentsVisitor<'_>,
) -> Result<Workspace, ScanError> {
    let root = store.root();
    let next_id = previous.map_or(1, |p| p.next_id());
    let mut ws = Workspace::with_next_id(root.to_path_buf(), next_id);

    let prev_section_ids: HashMap<&Path, SectionId> = previous.map_or_else(HashMap::new, |p| {
        p.sections
            .iter()
            .map(|s| (s.path.as_path(), s.id))
            .collect()
    });
    let prev_group_ids: HashMap<&Path, GroupId> = previous.map_or_else(HashMap::new, |p| {
        p.groups.iter().map(|g| (g.path.as_path(), g.id)).collect()
    });
    let prev_notes: HashMap<&Path, &Note> = previous.map_or_else(HashMap::new, |p| {
        p.notes.iter().map(|n| (n.path.as_path(), n)).collect()
    });

    let root_listing = scan_fs
        .collect_entries(store, None)
        .map_err(ScanError::RootUnreadable)?;
    let mut progress = ScanProgress::default();
    progress.observe_listing(root, &root_listing);
    ws.warnings.extend(root_listing.warnings);

    for entry in &root_listing.entries {
        progress.observed_entries.insert(entry.path());
        let name = entry.file_name();
        if is_dot_prefixed(&name) {
            continue;
        }

        let kind = match &entry.kind {
            Ok(kind) => *kind,
            Err(e) => {
                progress.read_error(&mut ws, entry.path(), "entry metadata unreadable", e);
                continue;
            }
        };

        if kind == EntryKind::Symlink {
            ws.warnings.push(warn(entry.path(), "symlink ignored"));
            continue;
        }

        if kind == EntryKind::File {
            if is_markdown_name(&name) {
                ws.warnings.push(warn(
                    entry.path(),
                    "Markdown file at workspace root ignored",
                ));
            }
            continue;
        }

        if kind != EntryKind::Directory {
            continue;
        }

        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                ws.warnings
                    .push(warn(entry.path(), "non-UTF-8 directory name"));
                continue;
            }
        };

        let section_rel = match RelPath::new(name_str) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let section_id = prev_section_ids
            .get(section_rel.as_path())
            .copied()
            .unwrap_or_else(|| SectionId(ws.allocate_id()));

        ws.sections.push(Section {
            id: section_id,
            name: name_str.to_owned(),
            path: section_rel.clone(),
        });

        let section_abs = entry.path();
        match scan_fs.collect_entries(store, Some(&section_rel)) {
            Ok(listing) => {
                progress.observe_listing(&section_abs, &listing);
                ws.warnings.extend(listing.warnings);
                scan_section_entries(
                    store,
                    &mut ws,
                    section_id,
                    &section_rel,
                    &listing.entries,
                    &prev_group_ids,
                    &prev_notes,
                    &mut progress,
                    scan_fs,
                    visit,
                );
            }
            Err(e) => {
                progress.read_error(&mut ws, section_abs, "section directory unreadable", &e);
            }
        }
    }

    if let Some(previous) = previous {
        preserve_incomplete(previous, &mut ws, &progress, visit);
    }
    sort_workspace(&mut ws);
    Ok(ws)
}

// ---------------------------------------------------------------------------
// Section-level scan
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn scan_section_entries(
    store: &FsStore,
    ws: &mut Workspace,
    section_id: SectionId,
    section_rel: &RelPath,
    entries: &[ScanEntry],
    prev_group_ids: &HashMap<&Path, GroupId>,
    prev_notes: &HashMap<&Path, &Note>,
    progress: &mut ScanProgress,
    scan_fs: &impl ScanFs,
    visit: &mut ContentsVisitor<'_>,
) {
    for entry in entries {
        progress.observed_entries.insert(entry.path());
        let name = entry.file_name();
        if is_dot_prefixed(&name) {
            continue;
        }

        let kind = match &entry.kind {
            Ok(kind) => *kind,
            Err(e) => {
                progress.read_error(ws, entry.path(), "entry metadata unreadable", e);
                continue;
            }
        };

        if kind == EntryKind::Symlink {
            ws.warnings.push(warn(entry.path(), "symlink ignored"));
            continue;
        }

        if kind == EntryKind::File {
            if !is_markdown_name(&name) {
                continue;
            }
            let name_str = match name.to_str() {
                Some(s) => s,
                None => {
                    ws.warnings.push(warn(entry.path(), "non-UTF-8 filename"));
                    continue;
                }
            };
            let note_rel = match section_rel.join(name_str) {
                Ok(r) => r,
                Err(_) => continue,
            };
            add_note(
                store,
                ws,
                Parent::Section(section_id),
                note_rel,
                &entry.path(),
                prev_notes,
                None,
                visit,
            );
            continue;
        }

        if kind != EntryKind::Directory {
            continue;
        }

        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                ws.warnings
                    .push(warn(entry.path(), "non-UTF-8 directory name"));
                continue;
            }
        };

        let group_rel = match section_rel.join(name_str) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let group_id = prev_group_ids
            .get(group_rel.as_path())
            .copied()
            .unwrap_or_else(|| GroupId(ws.allocate_id()));

        ws.groups.push(Group {
            id: group_id,
            section: section_id,
            name: name_str.to_owned(),
            path: group_rel.clone(),
        });

        scan_group_entries(
            store, ws, group_id, &group_rel, prev_notes, progress, scan_fs, visit,
        );
    }
}

// ---------------------------------------------------------------------------
// Group-level iterative scan (handles deeper-than-group notes)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn scan_group_entries(
    store: &FsStore,
    ws: &mut Workspace,
    group_id: GroupId,
    group_rel: &RelPath,
    prev_notes: &HashMap<&Path, &Note>,
    progress: &mut ScanProgress,
    scan_fs: &impl ScanFs,
    visit: &mut ContentsVisitor<'_>,
) {
    let mut pending = vec![group_rel.clone()];
    while let Some(current_rel) = pending.pop() {
        let current_abs = store.root().join(current_rel.as_path());
        let listing = match scan_fs.collect_entries(store, Some(&current_rel)) {
            Ok(listing) => listing,
            Err(e) => {
                let description = if current_rel == *group_rel {
                    "group directory unreadable"
                } else {
                    "subdirectory unreadable"
                };
                progress.read_error(ws, current_abs, description, &e);
                continue;
            }
        };
        progress.observe_listing(&current_abs, &listing);
        ws.warnings.extend(listing.warnings);
        let mut subdirectories = Vec::new();

        for entry in listing.entries {
            progress.observed_entries.insert(entry.path());
            let name = entry.file_name();
            if is_dot_prefixed(&name) {
                continue;
            }

            let kind = match &entry.kind {
                Ok(kind) => *kind,
                Err(e) => {
                    progress.read_error(ws, entry.path(), "entry metadata unreadable", e);
                    continue;
                }
            };

            if kind == EntryKind::Symlink {
                ws.warnings.push(warn(entry.path(), "symlink ignored"));
                continue;
            }

            if kind == EntryKind::File {
                if !is_markdown_name(&name) {
                    continue;
                }
                let name_str = match name.to_str() {
                    Some(s) => s,
                    None => {
                        ws.warnings.push(warn(entry.path(), "non-UTF-8 filename"));
                        continue;
                    }
                };
                let note_rel = match current_rel.join(name_str) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                let subtitle = compute_subtitle(note_rel.as_path(), group_rel.as_path());
                if subtitle.is_some() {
                    ws.warnings.push(warn(
                        entry.path(),
                        "note deeper than group level; shown under enclosing group",
                    ));
                }

                add_note(
                    store,
                    ws,
                    Parent::Group(group_id),
                    note_rel,
                    &entry.path(),
                    prev_notes,
                    subtitle,
                    visit,
                );
                continue;
            }

            if kind != EntryKind::Directory {
                continue;
            }

            let name_str = match name.to_str() {
                Some(s) => s,
                None => {
                    ws.warnings
                        .push(warn(entry.path(), "non-UTF-8 directory name"));
                    continue;
                }
            };
            if let Ok(sub_rel) = current_rel.join(name_str) {
                subdirectories.push(sub_rel);
            }
        }
        pending.extend(subdirectories.into_iter().rev());
    }
}

// ---------------------------------------------------------------------------
// Note creation with store.read and prior-entry preservation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn add_note(
    store: &FsStore,
    ws: &mut Workspace,
    parent: Parent,
    note_rel: RelPath,
    abs_path: &Path,
    prev_notes: &HashMap<&Path, &Note>,
    subtitle: Option<String>,
    visit: &mut ContentsVisitor<'_>,
) {
    match store.read(&note_rel) {
        Ok(disk_file) => {
            let note_id = prev_notes
                .get(note_rel.as_path())
                .map(|prev| prev.id)
                .unwrap_or_else(|| NoteId(ws.allocate_id()));

            let note = Note {
                id: note_id,
                parent,
                title: note_title(note_rel.file_name()),
                path: note_rel,
                subtitle,
                modified: disk_file.modified,
                bytes: disk_file.bytes.len(),
            };
            visit(&note, Ok(&disk_file));
            ws.notes.push(note);
        }
        Err(e) => {
            if e.is_not_found() {
                // Genuinely gone — do not preserve.
                return;
            }
            if let Some(prev) = prev_notes.get(note_rel.as_path()) {
                visit(prev, Err(&format!("content search unavailable: {e}")));
                ws.notes.push((*prev).clone());
                ws.warnings.push(warn(
                    abs_path.to_path_buf(),
                    format!("unreadable note, preserving previous entry: {e}"),
                ));
            } else {
                ws.warnings.push(warn(
                    abs_path.to_path_buf(),
                    format!("unreadable note: {e}"),
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Preservation helpers
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ScanProgress {
    incomplete_prefixes: HashSet<PathBuf>,
    observed_entries: HashSet<PathBuf>,
}

impl ScanProgress {
    fn observe_listing(&mut self, path: &Path, listing: &DirectoryEntries) {
        self.observed_entries.insert(path.to_path_buf());
        if listing.incomplete {
            self.incomplete_prefixes.insert(path.to_path_buf());
        }
    }

    fn read_error(
        &mut self,
        ws: &mut Workspace,
        path: PathBuf,
        description: &str,
        error: &std::io::Error,
    ) {
        self.observed_entries.insert(path.clone());
        let message = if matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        ) {
            format!("{description}: {error}; entry disappeared during scan")
        } else {
            self.incomplete_prefixes.insert(path.clone());
            format!("{description}: {error}; preserving previous state")
        };
        ws.warnings.push(warn(path, message));
    }

    fn should_preserve(&self, path: &Path) -> bool {
        // A successfully observed child overrides an incomplete ancestor. Its
        // own read failure, if any, takes precedence at the same prefix.
        for prefix in path.ancestors() {
            if self.incomplete_prefixes.contains(prefix) {
                return true;
            }
            if self.observed_entries.contains(prefix) {
                return false;
            }
        }
        false
    }
}

fn preserve_incomplete(
    previous: &Workspace,
    ws: &mut Workspace,
    progress: &ScanProgress,
    visit: &mut ContentsVisitor<'_>,
) {
    let mut paths: HashSet<RelPath> = ws
        .sections
        .iter()
        .map(|s| s.path.clone())
        .chain(ws.groups.iter().map(|g| g.path.clone()))
        .chain(ws.notes.iter().map(|n| n.path.clone()))
        .collect();
    let mut ids: HashSet<u64> = ws
        .sections
        .iter()
        .map(|s| s.id.0)
        .chain(ws.groups.iter().map(|g| g.id.0))
        .chain(ws.notes.iter().map(|n| n.id.0))
        .collect();
    let root = &ws.root;
    let mut preserve = |path: &RelPath, id: u64| {
        if progress.should_preserve(&root.join(path.as_path()))
            && !paths.contains(path)
            && !ids.contains(&id)
        {
            paths.insert(path.clone());
            ids.insert(id);
            true
        } else {
            false
        }
    };

    let mut section_ids: HashSet<SectionId> = ws.sections.iter().map(|s| s.id).collect();
    for section in &previous.sections {
        if preserve(&section.path, section.id.0) {
            section_ids.insert(section.id);
            ws.sections.push(section.clone());
        }
    }
    let mut group_ids: HashSet<GroupId> = ws.groups.iter().map(|g| g.id).collect();
    for group in &previous.groups {
        if section_ids.contains(&group.section) && preserve(&group.path, group.id.0) {
            group_ids.insert(group.id);
            ws.groups.push(group.clone());
        }
    }
    for note in &previous.notes {
        let parent_exists = match note.parent {
            Parent::Section(id) => section_ids.contains(&id),
            Parent::Group(id) => group_ids.contains(&id),
        };
        if parent_exists && preserve(&note.path, note.id.0) {
            visit(
                note,
                Err("scan incomplete; content search unavailable until this note is readable"),
            );
            ws.notes.push(note.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Directory listing
// ---------------------------------------------------------------------------

struct ScanEntry {
    path: PathBuf,
    name: OsString,
    kind: Result<EntryKind, std::io::Error>,
}

impl ScanEntry {
    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    fn file_name(&self) -> OsString {
        self.name.clone()
    }
}

struct DirectoryEntries {
    entries: Vec<ScanEntry>,
    warnings: Vec<ScanWarning>,
    incomplete: bool,
}

trait ScanFs {
    fn collect_entries(
        &self,
        store: &FsStore,
        path: Option<&RelPath>,
    ) -> Result<DirectoryEntries, std::io::Error>;
}

struct RealScanFs;

impl ScanFs for RealScanFs {
    fn collect_entries(
        &self,
        store: &FsStore,
        path: Option<&RelPath>,
    ) -> Result<DirectoryEntries, std::io::Error> {
        let listing = store.list_dir(path).map_err(scan_io_error)?;
        Ok(scan_listing(&listing_path(store, path), listing))
    }
}

fn listing_path(store: &FsStore, path: Option<&RelPath>) -> PathBuf {
    path.map_or_else(
        || store.root().to_path_buf(),
        |path| store.root().join(path.as_path()),
    )
}

fn scan_io_error(error: StoreError) -> std::io::Error {
    match error {
        StoreError::Io(error) => error,
        error => std::io::Error::other(error),
    }
}

fn scan_listing(dir: &Path, listing: DirectoryListing) -> DirectoryEntries {
    DirectoryEntries {
        incomplete: !listing.enumeration_errors.is_empty(),
        warnings: listing
            .enumeration_errors
            .into_iter()
            .map(|error| {
                warn(
                    dir.to_path_buf(),
                    format!(
                        "error reading directory entry: {error}; directory listing incomplete; preserving previous state for unseen entries"
                    ),
                )
            })
            .collect(),
        entries: listing
            .entries
            .into_iter()
            .map(|entry| ScanEntry {
                path: dir.join(&entry.name),
                name: entry.name,
                kind: entry.kind.map_err(scan_io_error),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

fn sort_workspace(ws: &mut Workspace) {
    ws.sections.sort_by(|a, b| {
        natural_cmp(&a.name, &b.name).then_with(|| a.path.as_path().cmp(b.path.as_path()))
    });

    ws.groups.sort_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then_with(|| natural_cmp(&a.name, &b.name))
            .then_with(|| a.path.as_path().cmp(b.path.as_path()))
    });

    ws.notes.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.as_path().cmp(b.path.as_path()))
    });
}

// ---------------------------------------------------------------------------
// Predicates
// ---------------------------------------------------------------------------

fn is_dot_prefixed(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

fn is_markdown_name(name: &OsStr) -> bool {
    let s = name.to_string_lossy();
    s.len() > 3 && s.as_bytes()[s.len() - 3..].eq_ignore_ascii_case(b".md")
}

fn warn(path: impl Into<PathBuf>, message: impl Into<String>) -> ScanWarning {
    ScanWarning {
        path: path.into(),
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn write_file(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
    }

    fn make_dir(root: &Path, rel: &str) {
        fs::create_dir_all(root.join(rel)).unwrap();
    }

    fn section_names(ws: &Workspace) -> Vec<&str> {
        ws.sections.iter().map(|s| s.name.as_str()).collect()
    }

    fn group_names(ws: &Workspace) -> Vec<&str> {
        ws.groups.iter().map(|g| g.name.as_str()).collect()
    }

    fn note_titles(ws: &Workspace) -> Vec<&str> {
        ws.notes.iter().map(|n| n.title.as_str()).collect()
    }

    fn has_warning_containing(ws: &Workspace, substr: &str) -> bool {
        ws.warnings.iter().any(|w| w.message.contains(substr))
    }

    fn entry_ids(ws: &Workspace) -> HashMap<RelPath, u64> {
        ws.sections
            .iter()
            .map(|s| (s.path.clone(), s.id.0))
            .chain(ws.groups.iter().map(|g| (g.path.clone(), g.id.0)))
            .chain(ws.notes.iter().map(|n| (n.path.clone(), n.id.0)))
            .collect()
    }

    fn assert_unique_entries(ws: &Workspace) {
        let entries = entry_ids(ws);
        let count = ws.sections.len() + ws.groups.len() + ws.notes.len();
        assert_eq!(entries.len(), count, "duplicate model paths");
        assert_eq!(
            entries.values().copied().collect::<HashSet<_>>().len(),
            count,
            "duplicate model IDs"
        );
        for group in &ws.groups {
            assert!(ws.section(group.section).is_some());
        }
        for note in &ws.notes {
            match note.parent {
                Parent::Section(id) => assert!(ws.section(id).is_some()),
                Parent::Group(id) => assert!(ws.group(id).is_some()),
            }
        }
    }

    #[derive(Default)]
    struct FaultyScanFs {
        metadata_errors: HashMap<PathBuf, std::io::ErrorKind>,
        listing_errors: HashMap<PathBuf, std::io::ErrorKind>,
        partial_listings: HashMap<PathBuf, HashSet<std::ffi::OsString>>,
    }

    impl ScanFs for FaultyScanFs {
        fn collect_entries(
            &self,
            store: &FsStore,
            path: Option<&RelPath>,
        ) -> Result<DirectoryEntries, std::io::Error> {
            let dir = listing_path(store, path);
            if let Some(kind) = self.listing_errors.get(&dir) {
                return Err(std::io::Error::new(*kind, "injected directory read error"));
            }
            let mut listing = store.list_dir(path).map_err(scan_io_error)?;
            if let Some(omitted) = self.partial_listings.get(&dir) {
                listing
                    .entries
                    .retain(|entry| !omitted.contains(&entry.name));
                listing
                    .enumeration_errors
                    .push(StoreError::Io(std::io::Error::other(
                        "injected directory entry error",
                    )));
            }
            for entry in &mut listing.entries {
                if let Some(kind) = self.metadata_errors.get(&dir.join(&entry.name)) {
                    entry.kind = Err(StoreError::Io(std::io::Error::new(
                        *kind,
                        "injected metadata read error",
                    )));
                }
            }
            Ok(scan_listing(&dir, listing))
        }
    }

    #[cfg(unix)]
    struct ReplacingScanFs {
        before_listing: RelPath,
        replaced: RelPath,
        outside: PathBuf,
        parked: PathBuf,
        triggered: std::cell::Cell<bool>,
    }

    #[cfg(unix)]
    impl ScanFs for ReplacingScanFs {
        fn collect_entries(
            &self,
            store: &FsStore,
            path: Option<&RelPath>,
        ) -> Result<DirectoryEntries, std::io::Error> {
            let replacing = path == Some(&self.before_listing) && !self.triggered.replace(true);
            if replacing {
                let target = store.root().join(self.replaced.as_path());
                fs::rename(&target, &self.parked).unwrap();
                std::os::unix::fs::symlink(&self.outside, &target).unwrap();
            }
            let result = RealScanFs.collect_entries(store, path);
            if replacing {
                assert!(result.is_err(), "a replaced subtree must not be enumerated");
            }
            result
        }
    }

    #[cfg(unix)]
    fn make_fifo(path: &Path) {
        #[cfg(not(target_os = "macos"))]
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            path,
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .unwrap();
        // rustix does not expose mkfifo/mkfifoat on Apple targets.
        #[cfg(target_os = "macos")]
        assert!(std::process::Command::new("mkfifo")
            .arg("-m")
            .arg("600")
            .arg(path)
            .status()
            .unwrap()
            .success());
    }

    #[cfg(unix)]
    struct RestorePermissions {
        path: PathBuf,
        permissions: fs::Permissions,
    }

    #[cfg(unix)]
    impl RestorePermissions {
        fn restrict(path: &Path, mode: u32) -> Self {
            use std::os::unix::fs::PermissionsExt;

            let permissions = fs::metadata(path).unwrap().permissions();
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
            Self {
                path: path.to_path_buf(),
                permissions,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            fs::set_permissions(&self.path, self.permissions.clone()).unwrap();
        }
    }

    #[test]
    fn content_visitor_streams_exact_read_bytes_once_per_accepted_id_without_rereading() {
        let directory = tempfile::tempdir().unwrap();
        write_file(directory.path(), "S/G/a.md", "initial bytes");
        write_file(directory.path(), "S/G/deep/b.md", "deep bytes");
        let store = FsStore::open(directory.path()).unwrap();
        let mut index = crate::index::Index::default();
        let mut seen = HashSet::new();
        let workspace = scan_with_contents(&store, None, |note, contents| {
            let file = contents.unwrap();
            assert!(seen.insert(note.id));
            assert_eq!(note.bytes, file.bytes.len());
            index.ingest(note, Ok(file));
            // The scanner has already obtained these exact bytes. Changing the
            // pathname now must not cause an index-side second read.
            fs::write(store.root().join(note.path.as_path()), "later bytes").unwrap();
        })
        .unwrap();
        index.sync_metadata(&workspace);
        assert_eq!(seen.len(), workspace.notes.len());
        for note in &workspace.notes {
            assert_eq!(
                index.content(note.id),
                Some(if note.title == "a" {
                    "initial bytes"
                } else {
                    "deep bytes"
                })
            );
            assert_eq!(store.read(&note.path).unwrap().bytes, b"later bytes");
        }
    }

    #[test]
    fn content_visitor_invalidates_preserved_unseen_ids_then_recovers_from_a_complete_scan() {
        let directory = tempfile::tempdir().unwrap();
        for path in ["S/G/a.md", "S/G/b.md", "S/Unreadable/c.md"] {
            write_file(directory.path(), path, "old bytes");
        }
        let store = FsStore::open(directory.path()).unwrap();
        let mut index = crate::index::Index::default();
        let previous =
            scan_with_contents(&store, None, |note, contents| index.ingest(note, contents))
                .unwrap();
        index.sync_metadata(&previous);
        write_file(directory.path(), "S/G/a.md", "changed bytes");
        let mut scan_fs = FaultyScanFs::default();
        scan_fs.partial_listings.insert(
            store.root().join("S/G"),
            [OsString::from("b.md")].into_iter().collect(),
        );
        scan_fs.metadata_errors.insert(
            store.root().join("S/Unreadable"),
            std::io::ErrorKind::PermissionDenied,
        );
        let mut seen = HashSet::new();
        let current =
            scan_with_fs_contents(&store, Some(&previous), &scan_fs, &mut |note, contents| {
                assert!(seen.insert(note.id), "duplicate content callback");
                index.ingest(note, contents);
            })
            .unwrap();
        index.sync_metadata(&current);
        assert_eq!(entry_ids(&current), entry_ids(&previous));
        assert_eq!(seen.len(), current.notes.len());
        for note in &current.notes {
            if note.title == "a" {
                assert_eq!(index.content(note.id), Some("changed bytes"));
                assert_eq!(index.limitation(note.id), None);
            } else {
                assert_eq!(index.content(note.id), None);
                assert!(index
                    .limitation(note.id)
                    .unwrap()
                    .contains("scan incomplete"));
            }
        }
        let complete = scan_with_contents(&store, Some(&current), |note, contents| {
            index.ingest(note, contents)
        })
        .unwrap();
        index.sync_metadata(&complete);
        for note in &complete.notes {
            assert!(index.content(note.id).is_some());
            assert_eq!(index.limitation(note.id), None);
        }
        let mut failed = FaultyScanFs::default();
        failed.listing_errors.insert(
            store.root().to_path_buf(),
            std::io::ErrorKind::PermissionDenied,
        );
        assert!(
            scan_with_fs_contents(&store, Some(&complete), &failed, &mut |_, _| panic!(
                "a root error must not publish callbacks"
            ))
            .is_err()
        );
    }

    // -- basic scan --

    #[test]
    fn scan_basic_structure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Projects/WebApp/design.md", "# Design");
        write_file(root, "Projects/WebApp/notes.md", "# Notes");
        write_file(root, "Projects/plan.md", "# Plan");
        write_file(root, "Inbox/scratch.md", "# Scratch");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.sections.len(), 2);
        assert_eq!(section_names(&ws), vec!["Inbox", "Projects"]);
        assert_eq!(ws.groups.len(), 1);
        assert_eq!(group_names(&ws), vec!["WebApp"]);
        assert_eq!(ws.notes.len(), 4);
    }

    #[test]
    fn scan_integrates_with_locked_store_and_published_updates() {
        use crate::fs_store::Expected;

        let dir = tempfile::tempdir().unwrap();
        let mut store = FsStore::open(dir.path()).unwrap();
        let section = store.create_dir(None, "Section").unwrap();
        let group = store.create_dir(Some(&section), "Group").unwrap();
        let created = store.create_file(&group, "café.MD", b"first").unwrap();
        let previous = scan(&store, None).unwrap();
        assert_eq!(previous.root, store.root());
        assert_eq!(previous.sections.len(), 1);
        assert_eq!(previous.groups.len(), 1);
        assert_eq!(previous.notes.len(), 1);
        assert!(previous.warnings.is_empty());
        assert_eq!(previous.notes[0].path, created.path);
        assert_eq!(previous.notes[0].title, "café");
        assert_eq!(previous.notes[0].bytes, created.file.bytes.len());
        assert_eq!(previous.notes[0].modified, created.file.modified);

        let saved = store
            .write(
                &created.path,
                b"published update",
                Expected::Bytes(b"first"),
            )
            .unwrap();
        let current = scan(&store, Some(&previous)).unwrap();
        assert_eq!(entry_ids(&current), entry_ids(&previous));
        assert_eq!(current.notes[0].bytes, saved.file.bytes.len());
        assert_eq!(current.notes[0].modified, saved.file.modified);
        assert!(current.warnings.is_empty());
        assert_unique_entries(&current);
    }

    #[test]
    fn scan_ungrouped_notes_in_section() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Inbox/scratch.md", "hello");
        write_file(root, "Inbox/todo.md", "world");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.sections.len(), 1);
        assert_eq!(ws.groups.len(), 0);
        assert_eq!(ws.notes.len(), 2);
        for note in &ws.notes {
            assert!(matches!(note.parent, Parent::Section(_)));
        }
    }

    // -- depth rules --

    #[test]
    fn deep_notes_get_subtitle_and_warning() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Projects/WebApp/design.md", "direct");
        write_file(root, "Projects/WebApp/src/arch.md", "deep");
        write_file(root, "Projects/WebApp/src/impl/detail.md", "deeper");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.groups.len(), 1);
        assert_eq!(ws.notes.len(), 3);

        let direct = ws.notes.iter().find(|n| n.title == "design").unwrap();
        assert_eq!(direct.subtitle, None);

        let deep = ws.notes.iter().find(|n| n.title == "arch").unwrap();
        assert_eq!(deep.subtitle.as_deref(), Some("src/arch.md"));
        assert_eq!(deep.parent, direct.parent);

        let deeper = ws.notes.iter().find(|n| n.title == "detail").unwrap();
        assert_eq!(deeper.subtitle.as_deref(), Some("src/impl/detail.md"));
        assert_eq!(deeper.parent, direct.parent);

        assert!(has_warning_containing(&ws, "deeper than group level"));
    }

    #[test]
    fn deeply_nested_notes_scan_on_a_small_thread_stack() {
        use rustix::fs::{self as descriptor_fs, AtFlags, Mode, OFlags};

        let dir = tempfile::tempdir().unwrap();
        let mut relative = PathBuf::from("Section/Group");
        for _ in 0..512 {
            relative.push("d");
        }
        // Build through directory descriptors: this depth exceeds Darwin's
        // absolute PATH_MAX before the scanner is exercised.
        let mut store = FsStore::open(dir.path()).unwrap();
        let parent = RelPath::new(&relative).unwrap();
        store.ensure_dir(&parent).unwrap();
        let created = store.create_file(&parent, "note", b"deep note").unwrap();
        assert!(created.durability_error.is_none());
        assert!(store.take_warnings().is_empty());
        assert!(!store.pending_durability());
        relative.push("note.md");

        let ws = std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || scan(&store, None).unwrap())
            .unwrap()
            .join()
            .unwrap();

        assert_eq!(ws.sections.len(), 1);
        assert_eq!(ws.groups.len(), 1);
        assert_eq!(note_titles(&ws), vec!["note"]);
        assert_eq!(ws.notes[0].path.as_path(), relative);
        assert_eq!(ws.notes[0].bytes, b"deep note".len());
        assert_eq!(ws.notes[0].parent, Parent::Group(ws.groups[0].id));
        let subtitle = relative.strip_prefix("Section/Group").unwrap();
        assert_eq!(ws.notes[0].subtitle.as_deref(), subtitle.to_str());
        assert!(has_warning_containing(&ws, "deeper than group level"));
        assert_unique_entries(&ws);

        // std's recursive cleanup retains a descriptor per level. Prune this
        // single branch bottom-up without exhausting a low descriptor limit.
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory = descriptor_fs::open(dir.path(), flags, Mode::empty()).unwrap();
        for component in parent.as_path().components() {
            directory =
                descriptor_fs::openat(&directory, component.as_os_str(), flags, Mode::empty())
                    .unwrap();
        }
        descriptor_fs::unlinkat(&directory, created.path.file_name(), AtFlags::empty()).unwrap();
        for component in parent.as_path().components().rev() {
            let parent_directory =
                descriptor_fs::openat(&directory, "..", flags, Mode::empty()).unwrap();
            descriptor_fs::unlinkat(&parent_directory, component.as_os_str(), AtFlags::REMOVEDIR)
                .unwrap();
            directory = parent_directory;
        }
        drop(directory);
        dir.close().unwrap();
    }

    // -- dotfiles and dotdirs --

    #[test]
    fn dotfiles_and_dotdirs_ignored_at_every_level() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        make_dir(root, ".git");
        write_file(root, ".gitignore", "");
        make_dir(root, "Projects/.obsidian");
        write_file(root, "Projects/.DS_Store", "");
        make_dir(root, "Projects/WebApp/.hidden");
        write_file(root, "Projects/WebApp/.secret.md", "");
        write_file(root, "Projects/WebApp/visible.md", "ok");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.sections.len(), 1);
        assert_eq!(ws.groups.len(), 1);
        assert_eq!(ws.notes.len(), 1);
        assert_eq!(ws.notes[0].title, "visible");
    }

    // -- non-MD files --

    #[test]
    fn non_md_files_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Projects/WebApp/design.md", "md");
        write_file(root, "Projects/WebApp/photo.jpg", "jpg");
        write_file(root, "Projects/WebApp/data.json", "json");
        write_file(root, "Projects/WebApp/readme", "no ext");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes.len(), 1);
        assert_eq!(ws.notes[0].title, "design");
    }

    // -- root-level MD warning --

    #[test]
    fn root_level_md_warned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "README.md", "root markdown");
        write_file(root, "Inbox/note.md", "ok");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes.len(), 1);
        assert!(has_warning_containing(&ws, "workspace root"));
    }

    // -- unicode names --

    #[test]
    fn unicode_section_group_note_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "日記/旅行/東京.md", "Tokyo notes");
        write_file(root, "Ñotas/café.md", "coffee");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.sections.len(), 2);
        assert!(ws.sections.iter().any(|s| s.name == "日記"));
        assert!(ws.sections.iter().any(|s| s.name == "Ñotas"));

        assert_eq!(ws.groups.len(), 1);
        assert_eq!(ws.groups[0].name, "旅行");

        assert_eq!(ws.notes.len(), 2);
        assert!(ws.notes.iter().any(|n| n.title == "東京"));
        assert!(ws.notes.iter().any(|n| n.title == "café"));
    }

    // -- case-insensitive .md extension --

    #[test]
    fn case_insensitive_md_extension() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/lower.md", "a");
        write_file(root, "Sec/upper.MD", "b");
        write_file(root, "Sec/mixed.Md", "c");
        write_file(root, "Sec/mixed2.mD", "d");
        write_file(root, "Sec/not.txt", "e");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes.len(), 4);
        let titles: Vec<&str> = ws.notes.iter().map(|n| n.title.as_str()).collect();
        assert!(titles.contains(&"lower"));
        assert!(titles.contains(&"upper"));
        assert!(titles.contains(&"mixed"));
        assert!(titles.contains(&"mixed2"));
    }

    // -- symlinks --

    #[cfg(unix)]
    #[test]
    fn symlinks_warned_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/real.md", "content");

        // Symlink at root level
        std::os::unix::fs::symlink("/tmp", root.join("linked_section")).unwrap();
        // Symlink file in section
        std::os::unix::fs::symlink(root.join("Sec/real.md"), root.join("Sec/linked.md")).unwrap();
        // Symlink dir in section
        std::os::unix::fs::symlink("/tmp", root.join("Sec/linked_group")).unwrap();

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes.len(), 1);
        assert_eq!(ws.notes[0].title, "real");
        assert_eq!(ws.groups.len(), 0);

        let symlink_warnings = ws
            .warnings
            .iter()
            .filter(|w| w.message.contains("symlink"))
            .count();
        assert_eq!(symlink_warnings, 3);
    }

    #[cfg(unix)]
    #[test]
    fn root_replaced_with_foreign_symlink_returns_error_instead_of_foreign_containers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        write_file(&root, "Section/Group/note.md", "trusted");
        let store = FsStore::open(&root).unwrap();
        let previous = scan(&store, None).unwrap();
        let previous_ids = entry_ids(&previous);
        let outside = tempfile::tempdir().unwrap();
        write_file(
            outside.path(),
            "OutsideOnly/ForeignGroup/foreign.md",
            "foreign",
        );
        let parked = dir.path().join("parked");
        fs::rename(&root, &parked).unwrap();
        std::os::unix::fs::symlink(outside.path(), &root).unwrap();

        for previous in [None, Some(&previous)] {
            assert!(matches!(
                scan(&store, previous),
                Err(ScanError::RootUnreadable(_))
            ));
        }
        assert!(store
            .read(&RelPath::new("OutsideOnly/ForeignGroup/foreign.md").unwrap())
            .is_err());
        assert_eq!(entry_ids(&previous), previous_ids);
        fs::remove_file(&root).unwrap();
        fs::rename(&parked, &root).unwrap();
        let restored = scan(&store, Some(&previous)).unwrap();
        assert_eq!(entry_ids(&restored), previous_ids);
        assert!(restored.warnings.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn subtree_symlink_replacement_before_listing_warns_and_preserves_previous_entries() {
        for (before_listing, replaced) in [
            ("Section", "Section"),
            ("Section/Group", "Section/Group"),
            ("Section/Group/deep", "Section/Group/deep"),
            ("Section/Group", "Section"),
            ("Section/Group/deep", "Section/Group"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_file(dir.path(), "Section/direct.md", "trusted direct");
            write_file(dir.path(), "Section/Group/note.md", "trusted group");
            write_file(dir.path(), "Section/Group/deep/note.md", "trusted deep");
            write_file(dir.path(), "Other/note.md", "unaffected");
            let store = FsStore::open(dir.path()).unwrap();
            let previous = scan(&store, None).unwrap();
            let outside = tempfile::tempdir().unwrap();
            for name in [
                "OutsideOnly/ForeignGroup/foreign.md",
                "ForeignGroup/foreign.md",
                "foreign.md",
                "direct.md",
                "note.md",
            ] {
                write_file(
                    outside.path(),
                    name,
                    "foreign bytes must not enter the model",
                );
            }
            let scan_fs = ReplacingScanFs {
                before_listing: RelPath::new(before_listing).unwrap(),
                replaced: RelPath::new(replaced).unwrap(),
                outside: outside.path().to_path_buf(),
                parked: dir.path().join(".parked"),
                triggered: std::cell::Cell::new(false),
            };

            let current = scan_with_fs(&store, Some(&previous), &scan_fs).unwrap();
            assert!(scan_fs.triggered.get());
            assert_eq!(
                entry_ids(&current),
                entry_ids(&previous),
                "{before_listing}"
            );
            for note in &current.notes {
                let previous_note = previous.note(note.id).unwrap();
                assert_eq!(note.bytes, previous_note.bytes);
                assert_eq!(note.modified, previous_note.modified);
            }
            assert!(current.warnings.iter().any(|warning| {
                warning.path == store.root().join(before_listing)
                    && warning.message.contains("preserving previous state")
            }));
            assert!(!entry_ids(&current).keys().any(|path| {
                path.as_path().components().any(|component| {
                    matches!(
                        component.as_os_str().to_str(),
                        Some("OutsideOnly" | "ForeignGroup" | "foreign.md")
                    )
                })
            }));
            assert!(store
                .read(&RelPath::new(Path::new(replaced).join("foreign.md")).unwrap())
                .is_err());
            assert_unique_entries(&current);
        }
    }

    // -- invalid UTF-8 --

    // Keep physical byte-name fixtures on Linux: APFS rejects invalid UTF-8
    // names during fixture creation, before the scanner can inspect them.
    #[cfg(target_os = "linux")]
    #[test]
    fn invalid_utf8_directory_warned() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Good/note.md", "ok");

        // Create a directory with an invalid UTF-8 name at the root level
        let bad_name = OsStr::from_bytes(&[0xff, 0xfe]);
        fs::create_dir(root.join(bad_name)).unwrap();

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.sections.len(), 1);
        assert_eq!(ws.sections[0].name, "Good");
        assert!(has_warning_containing(&ws, "non-UTF-8"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn invalid_utf8_filename_warned() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/good.md", "ok");

        // Create a file with invalid UTF-8 name ending in .md
        let bad_name = OsStr::from_bytes(&[0xff, 0xfe, b'.', b'm', b'd']);
        fs::write(root.join("Sec").join(bad_name), b"bad").unwrap();

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes.len(), 1);
        assert_eq!(ws.notes[0].title, "good");
        assert!(has_warning_containing(&ws, "non-UTF-8"));
    }

    // -- stable IDs across rescans --

    #[test]
    fn ids_stable_on_unchanged_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Projects/WebApp/design.md", "v1");
        write_file(root, "Inbox/scratch.md", "v1");

        let store = FsStore::open(root).unwrap();
        let ws1 = scan(&store, None).unwrap();
        let ws2 = scan(&store, Some(&ws1)).unwrap();

        for s1 in &ws1.sections {
            let s2 = ws2.sections.iter().find(|s| s.name == s1.name).unwrap();
            assert_eq!(s1.id, s2.id, "section '{}' ID changed", s1.name);
        }
        for g1 in &ws1.groups {
            let g2 = ws2.groups.iter().find(|g| g.name == g1.name).unwrap();
            assert_eq!(g1.id, g2.id, "group '{}' ID changed", g1.name);
        }
        for n1 in &ws1.notes {
            let n2 = ws2.notes.iter().find(|n| n.title == n1.title).unwrap();
            assert_eq!(n1.id, n2.id, "note '{}' ID changed", n1.title);
        }
    }

    #[test]
    fn new_entries_get_higher_ids() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Projects/WebApp/design.md", "v1");

        let store = FsStore::open(root).unwrap();
        let ws1 = scan(&store, None).unwrap();
        let max_id_1 = ws1.next_id();

        write_file(root, "Projects/WebApp/notes.md", "new");
        write_file(root, "Archive/old.md", "archived");

        let ws2 = scan(&store, Some(&ws1)).unwrap();

        let new_note = ws2.notes.iter().find(|n| n.title == "notes").unwrap();
        assert!(new_note.id.0 >= max_id_1, "new note should have higher ID");

        let new_section = ws2.sections.iter().find(|s| s.name == "Archive").unwrap();
        assert!(
            new_section.id.0 >= max_id_1,
            "new section should have higher ID"
        );
    }

    // -- remap + rescan --

    #[test]
    fn remap_then_rescan_preserves_ids() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Projects/WebApp/design.md", "v1");
        write_file(root, "Projects/plan.md", "v1");

        let store = FsStore::open(root).unwrap();
        let mut ws1 = scan(&store, None).unwrap();
        let sec_id = ws1
            .sections
            .iter()
            .find(|s| s.name == "Projects")
            .unwrap()
            .id;
        let grp_id = ws1.groups.iter().find(|g| g.name == "WebApp").unwrap().id;
        let note_id = ws1.notes.iter().find(|n| n.title == "design").unwrap().id;

        // Remap paths in the model before renaming on disk
        ws1.remap_paths(
            &RelPath::new("Projects").unwrap(),
            &RelPath::new("Work").unwrap(),
        );

        // Rename on disk
        fs::rename(root.join("Projects"), root.join("Work")).unwrap();

        let ws2 = scan(&store, Some(&ws1)).unwrap();

        assert_eq!(
            ws2.sections.iter().find(|s| s.name == "Work").unwrap().id,
            sec_id,
        );
        assert_eq!(
            ws2.groups.iter().find(|g| g.name == "WebApp").unwrap().id,
            grp_id,
        );
        assert_eq!(
            ws2.notes.iter().find(|n| n.title == "design").unwrap().id,
            note_id,
        );
    }

    // -- deletions --

    #[test]
    fn deleted_file_removed_from_model() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/a.md", "a");
        write_file(root, "Sec/b.md", "b");

        let store = FsStore::open(root).unwrap();
        let ws1 = scan(&store, None).unwrap();
        assert_eq!(ws1.notes.len(), 2);

        fs::remove_file(root.join("Sec/b.md")).unwrap();
        let ws2 = scan(&store, Some(&ws1)).unwrap();

        assert_eq!(ws2.notes.len(), 1);
        assert_eq!(ws2.notes[0].title, "a");
    }

    #[test]
    fn deleted_group_removed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/Grp/note.md", "content");
        write_file(root, "Sec/other.md", "other");

        let store = FsStore::open(root).unwrap();
        let ws1 = scan(&store, None).unwrap();
        assert_eq!(ws1.groups.len(), 1);
        assert_eq!(ws1.notes.len(), 2);

        fs::remove_dir_all(root.join("Sec/Grp")).unwrap();
        let ws2 = scan(&store, Some(&ws1)).unwrap();

        assert_eq!(ws2.groups.len(), 0);
        assert_eq!(ws2.notes.len(), 1);
        assert_eq!(ws2.notes[0].title, "other");
    }

    #[test]
    fn deleted_section_removed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Alpha/note.md", "a");
        write_file(root, "Beta/note.md", "b");

        let store = FsStore::open(root).unwrap();
        let ws1 = scan(&store, None).unwrap();
        assert_eq!(ws1.sections.len(), 2);

        fs::remove_dir_all(root.join("Beta")).unwrap();
        let ws2 = scan(&store, Some(&ws1)).unwrap();

        assert_eq!(ws2.sections.len(), 1);
        assert_eq!(ws2.sections[0].name, "Alpha");
    }

    // -- incomplete / unreadable preservation --

    #[test]
    fn metadata_read_errors_preserve_entries_at_every_level() {
        for unreadable in [
            "Section",
            "Section/direct.md",
            "Section/Group",
            "Section/Group/note.md",
            "Section/Group/deep",
            "Section/Group/deep/note.md",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_file(dir.path(), "Section/direct.md", "direct");
            write_file(dir.path(), "Section/Group/note.md", "grouped");
            write_file(dir.path(), "Section/Group/deep/note.md", "deep");
            let store = FsStore::open(dir.path()).unwrap();
            let previous = scan(&store, None).unwrap();
            write_file(dir.path(), "New/new.md", "new");
            let unreadable_path = store.root().join(unreadable);
            let mut scan_fs = FaultyScanFs::default();
            scan_fs.metadata_errors.insert(
                unreadable_path.clone(),
                std::io::ErrorKind::PermissionDenied,
            );

            let current = scan_with_fs(&store, Some(&previous), &scan_fs).unwrap();
            let current_ids = entry_ids(&current);
            for (path, id) in entry_ids(&previous) {
                assert_eq!(current_ids.get(&path), Some(&id), "{unreadable}: {path:?}");
            }
            assert_eq!(current.notes.len(), 4, "{unreadable}");
            assert!(current.warnings.iter().any(|warning| {
                warning.path == unreadable_path
                    && warning.message.contains("preserving previous state")
            }));
            assert_unique_entries(&current);
        }
    }

    #[test]
    fn metadata_missing_errors_do_not_preserve_entries() {
        for missing in [
            "Section",
            "Section/direct.md",
            "Section/Group",
            "Section/Group/deep",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_file(dir.path(), "Section/direct.md", "direct");
            write_file(dir.path(), "Section/Group/deep/note.md", "deep");
            write_file(dir.path(), "Other/note.md", "other");
            let store = FsStore::open(dir.path()).unwrap();
            let previous = scan(&store, None).unwrap();
            let mut scan_fs = FaultyScanFs::default();
            scan_fs
                .metadata_errors
                .insert(store.root().join(missing), std::io::ErrorKind::NotFound);

            let current = scan_with_fs(&store, Some(&previous), &scan_fs).unwrap();
            assert!(!entry_ids(&current)
                .keys()
                .any(|path| path.as_path().starts_with(missing)));
            assert!(current
                .notes
                .iter()
                .any(|note| note.title == "note" && note.path.as_path().starts_with("Other")));
            assert!(!has_warning_containing(
                &current,
                "preserving previous state"
            ));
            assert_unique_entries(&current);
        }
    }

    #[test]
    fn directory_read_errors_preserve_only_the_unreadable_subtree() {
        for unreadable in ["Section", "Section/Group", "Section/Group/deep"] {
            let dir = tempfile::tempdir().unwrap();
            write_file(dir.path(), "Section/direct.md", "direct");
            write_file(dir.path(), "Section/Group/note.md", "grouped");
            write_file(dir.path(), "Section/Group/deep/note.md", "deep");
            write_file(dir.path(), "Other/gone.md", "gone");
            let store = FsStore::open(dir.path()).unwrap();
            let previous = scan(&store, None).unwrap();
            fs::remove_file(dir.path().join("Other/gone.md")).unwrap();
            let mut scan_fs = FaultyScanFs::default();
            scan_fs.listing_errors.insert(
                store.root().join(unreadable),
                std::io::ErrorKind::PermissionDenied,
            );

            let current = scan_with_fs(&store, Some(&previous), &scan_fs).unwrap();
            let current_ids = entry_ids(&current);
            for (path, id) in entry_ids(&previous) {
                if path.as_path().starts_with(unreadable) {
                    assert_eq!(current_ids.get(&path), Some(&id));
                }
            }
            assert_eq!(current.notes.len(), 3);
            assert!(!current.notes.iter().any(|note| note.title == "gone"));
            assert!(has_warning_containing(
                &current,
                "preserving previous state"
            ));
            assert_unique_entries(&current);
        }
    }

    #[cfg(unix)]
    #[test]
    fn partial_listings_preserve_unseen_entries_without_undoing_observed_changes() {
        for container in ["", "Section", "Section/Group", "Section/Group/deep"] {
            let dir = tempfile::tempdir().unwrap();
            let relative = |suffix: &str| {
                Path::new(container)
                    .join(suffix)
                    .to_str()
                    .unwrap()
                    .to_owned()
            };
            for path in [
                "Unseen/nested/note.md",
                "Missing/nested/note.md",
                "Unreadable/nested/note.md",
                "Seen/changed.md",
                "Seen/gone.md",
                "Ignored/nested/note.md",
                "Linked/nested/note.md",
            ] {
                write_file(dir.path(), &relative(path), "previous");
            }
            let store = FsStore::open(dir.path()).unwrap();
            let previous = scan(&store, None).unwrap();
            fs::remove_dir_all(dir.path().join(relative("Missing"))).unwrap();
            fs::remove_file(dir.path().join(relative("Seen/gone.md"))).unwrap();
            write_file(dir.path(), &relative("Seen/changed.md"), "changed on disk");
            write_file(dir.path(), &relative("Seen/new.md"), "new");
            fs::remove_dir_all(dir.path().join(relative("Ignored"))).unwrap();
            write_file(dir.path(), &relative("Ignored"), "not a directory");
            fs::remove_dir_all(dir.path().join(relative("Linked"))).unwrap();
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join(relative("Linked")))
                .unwrap();
            let mut scan_fs = FaultyScanFs::default();
            scan_fs.partial_listings.insert(
                store.root().join(container),
                [std::ffi::OsString::from("Unseen")].into_iter().collect(),
            );
            scan_fs.metadata_errors.insert(
                store.root().join(relative("Unreadable")),
                std::io::ErrorKind::PermissionDenied,
            );

            let current = scan_with_fs(&store, Some(&previous), &scan_fs).unwrap();
            assert_eq!(current.notes.len(), 5, "{container}");
            let current_ids = entry_ids(&current);
            let previous_ids = entry_ids(&previous);
            for suffix in [
                "Unseen/nested/note.md",
                "Missing/nested/note.md",
                "Unreadable/nested/note.md",
            ] {
                let path = RelPath::new(relative(suffix)).unwrap();
                assert_eq!(current_ids.get(&path), previous_ids.get(&path));
            }
            assert!(!current.notes.iter().any(|note| {
                note.path.as_path().starts_with(relative("Ignored"))
                    || note.path.as_path().starts_with(relative("Linked"))
                    || note.title == "gone"
            }));
            let changed = current
                .notes
                .iter()
                .find(|note| note.title == "changed")
                .unwrap();
            assert_eq!(changed.bytes, "changed on disk".len());
            assert_eq!(
                current_ids.get(&changed.path),
                previous_ids.get(&changed.path)
            );
            assert!(has_warning_containing(
                &current,
                "directory listing incomplete"
            ));
            assert!(has_warning_containing(&current, "symlink ignored"));
            assert_unique_entries(&current);

            let complete = scan(&store, Some(&current)).unwrap();
            assert_eq!(complete.notes.len(), 4, "{container}");
            assert!(!entry_ids(&complete)
                .keys()
                .any(|path| path.as_path().starts_with(relative("Missing"))));
            assert_unique_entries(&complete);
        }
    }

    #[test]
    fn unreadable_replacement_directory_does_not_restore_a_former_note() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "Section/note.md", "previous");
        let store = FsStore::open(dir.path()).unwrap();
        let previous = scan(&store, None).unwrap();
        let former_note_id = previous.notes[0].id;
        fs::remove_file(dir.path().join("Section/note.md")).unwrap();
        fs::create_dir(dir.path().join("Section/note.md")).unwrap();
        let mut scan_fs = FaultyScanFs::default();
        scan_fs.listing_errors.insert(
            store.root().join("Section/note.md"),
            std::io::ErrorKind::PermissionDenied,
        );

        let current = scan_with_fs(&store, Some(&previous), &scan_fs).unwrap();
        assert!(current.notes.is_empty());
        assert_eq!(current.groups.len(), 1);
        assert_eq!(
            current.groups[0].path.as_path(),
            Path::new("Section/note.md")
        );
        assert_ne!(current.groups[0].id.0, former_note_id.0);
        assert_unique_entries(&current);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_section_preserves_previous_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Projects/WebApp/design.md", "content");
        write_file(root, "Projects/plan.md", "plan");
        write_file(root, "Inbox/scratch.md", "scratch");

        let store = FsStore::open(root).unwrap();
        let ws1 = scan(&store, None).unwrap();
        assert_eq!(ws1.notes.len(), 3);

        let permissions = RestorePermissions::restrict(&root.join("Projects"), 0o000);
        let result = scan(&store, Some(&ws1));
        drop(permissions);
        let ws2 = result.unwrap();

        // The section should still appear (directory exists in root listing)
        assert!(ws2.sections.iter().any(|s| s.name == "Projects"));
        // Previous groups and notes under Projects should be preserved
        assert!(ws2.groups.iter().any(|g| g.name == "WebApp"));
        let preserved_notes: Vec<&str> = ws2
            .notes
            .iter()
            .filter(|n| n.path.as_path().starts_with("Projects"))
            .map(|n| n.title.as_str())
            .collect();
        assert!(preserved_notes.contains(&"design"));
        assert!(preserved_notes.contains(&"plan"));
        assert!(has_warning_containing(&ws2, "preserving previous state"));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_group_preserves_previous_notes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/Grp/note.md", "content");
        write_file(root, "Sec/other.md", "other");

        let store = FsStore::open(root).unwrap();
        let ws1 = scan(&store, None).unwrap();

        let permissions = RestorePermissions::restrict(&root.join("Sec/Grp"), 0o000);
        let result = scan(&store, Some(&ws1));
        drop(permissions);
        let ws2 = result.unwrap();

        assert!(ws2.groups.iter().any(|g| g.name == "Grp"));
        assert!(ws2.notes.iter().any(|n| n.title == "note"));
        assert!(ws2.notes.iter().any(|n| n.title == "other"));
        assert!(has_warning_containing(&ws2, "preserving previous state"));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_note_preserves_previous_metadata_from_real_store() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "Section/note.md", "previous bytes");
        let store = FsStore::open(dir.path()).unwrap();
        let previous = scan(&store, None).unwrap();
        write_file(dir.path(), "Section/note.md", "changed");
        let permissions = RestorePermissions::restrict(&dir.path().join("Section/note.md"), 0);
        let read_error = store.read(&previous.notes[0].path).unwrap_err();
        let result = scan(&store, Some(&previous));
        drop(permissions);
        let current = result.unwrap();

        assert!(
            matches!(read_error, crate::fs_store::StoreError::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
        assert_eq!(entry_ids(&current), entry_ids(&previous));
        assert_eq!(current.notes[0].bytes, previous.notes[0].bytes);
        assert_eq!(current.notes[0].modified, previous.notes[0].modified);
        assert!(has_warning_containing(
            &current,
            "unreadable note, preserving previous entry"
        ));
        assert_unique_entries(&current);
    }

    #[cfg(unix)]
    #[test]
    fn unsearchable_root_returns_error_when_store_cannot_revalidate_its_lock() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "Section/Group/deep/note.md", "previous");
        let store = FsStore::open(dir.path()).unwrap();
        let previous = scan(&store, None).unwrap();
        let previous_ids = entry_ids(&previous);
        let permissions = RestorePermissions::restrict(dir.path(), 0o400);
        let result = scan(&store, Some(&previous));
        drop(permissions);

        assert!(matches!(
            result,
            Err(ScanError::RootUnreadable(error))
                if error.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert_eq!(entry_ids(&previous), previous_ids);
        let current = scan(&store, Some(&previous)).unwrap();
        assert_eq!(entry_ids(&current), previous_ids);
        assert!(current
            .warnings
            .iter()
            .all(|warning| { warning.message.contains("deeper than group level") }));
        assert_unique_entries(&current);
    }

    // -- root listing failure --

    #[cfg(unix)]
    #[test]
    fn unreadable_root_returns_permission_denied_after_opening_store() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "Section/note.md", "previous");
        let store = FsStore::open(dir.path()).unwrap();
        let previous = scan(&store, None).unwrap();
        let permissions = RestorePermissions::restrict(dir.path(), 0);
        let result = scan(&store, Some(&previous));
        drop(permissions);

        let error = result.unwrap_err();
        assert!(
            matches!(error, ScanError::RootUnreadable(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
        assert_eq!(previous.notes.len(), 1);
        assert!(scan(&store, Some(&previous)).is_ok());
    }

    #[test]
    fn root_listing_failure_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let store = FsStore::open(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();

        let err = scan(&store, None).unwrap_err();
        assert!(
            matches!(&err, ScanError::RootUnreadable(error) if error.kind() == std::io::ErrorKind::NotFound)
        );
        assert!(err.to_string().contains("cannot read workspace root"));
    }

    // -- empty workspace --

    #[test]
    fn scan_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::open(dir.path()).unwrap();
        let ws = scan(&store, None).unwrap();

        assert!(ws.sections.is_empty());
        assert!(ws.groups.is_empty());
        assert!(ws.notes.is_empty());
        assert!(ws.warnings.is_empty());
    }

    // -- sorting --

    #[test]
    fn sections_sorted_natural_alphabetical() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        make_dir(root, "Zebra");
        make_dir(root, "alpha");
        make_dir(root, "Beta");
        make_dir(root, "Section 2");
        make_dir(root, "Section 10");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(
            section_names(&ws),
            vec!["alpha", "Beta", "Section 2", "Section 10", "Zebra"],
        );
    }

    #[test]
    fn notes_sorted_modified_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/old.md", "old");

        let old_time = SystemTime::now() - Duration::from_secs(10);
        fs::File::open(root.join("Sec/old.md"))
            .unwrap()
            .set_modified(old_time)
            .unwrap();

        write_file(root, "Sec/new.md", "new");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes.len(), 2);
        assert_eq!(ws.notes[0].title, "new");
        assert_eq!(ws.notes[1].title, "old");
    }

    // -- note bytes and modified --

    #[test]
    fn note_records_bytes_and_modified() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let content = "hello world";
        write_file(root, "Sec/note.md", content);

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes[0].bytes, content.len());
        let age = SystemTime::now()
            .duration_since(ws.notes[0].modified)
            .unwrap();
        assert!(age.as_secs() < 5);
    }

    // -- note parent assignment --

    #[test]
    fn notes_assigned_correct_parents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(root, "Sec/ungrouped.md", "a");
        write_file(root, "Sec/Grp/grouped.md", "b");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        let sec_id = ws.sections[0].id;
        let grp_id = ws.groups[0].id;

        let ungrouped = ws.notes.iter().find(|n| n.title == "ungrouped").unwrap();
        assert_eq!(ungrouped.parent, Parent::Section(sec_id));

        let grouped = ws.notes.iter().find(|n| n.title == "grouped").unwrap();
        assert_eq!(grouped.parent, Parent::Group(grp_id));
    }

    // -- groups sorted within their section --

    #[test]
    fn groups_sorted_within_section() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        make_dir(root, "Sec/Zebra");
        make_dir(root, "Sec/Alpha");
        make_dir(root, "Sec/beta");

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(group_names(&ws), vec!["Alpha", "beta", "Zebra"]);
    }

    // -- predicates --

    #[test]
    fn test_is_dot_prefixed() {
        assert!(is_dot_prefixed(std::ffi::OsStr::new(".git")));
        assert!(is_dot_prefixed(std::ffi::OsStr::new(".hidden")));
        assert!(is_dot_prefixed(std::ffi::OsStr::new(".")));
        assert!(!is_dot_prefixed(std::ffi::OsStr::new("visible")));
        assert!(!is_dot_prefixed(std::ffi::OsStr::new("a.md")));
    }

    #[test]
    fn test_is_markdown_name() {
        assert!(is_markdown_name(std::ffi::OsStr::new("note.md")));
        assert!(is_markdown_name(std::ffi::OsStr::new("NOTE.MD")));
        assert!(is_markdown_name(std::ffi::OsStr::new("x.Md")));
        assert!(is_markdown_name(std::ffi::OsStr::new("a.md")));
        assert!(!is_markdown_name(std::ffi::OsStr::new(".md")));
        assert!(!is_markdown_name(std::ffi::OsStr::new("file.txt")));
        assert!(!is_markdown_name(std::ffi::OsStr::new("readme")));
        assert!(!is_markdown_name(std::ffi::OsStr::new("md")));
    }

    // -- special files silently skipped --

    #[cfg(unix)]
    #[test]
    fn special_files_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        make_dir(root, "Sec");
        write_file(root, "Sec/note.md", "ok");

        make_fifo(&root.join("Sec/pipe.md"));

        let store = FsStore::open(root).unwrap();
        let ws = scan(&store, None).unwrap();

        assert_eq!(ws.notes.len(), 1);
        assert_eq!(ws.notes[0].title, "note");
    }
}
