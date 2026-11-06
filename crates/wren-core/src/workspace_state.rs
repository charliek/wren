//! Optional workspace metadata. `schema_version = 1` is the current format;
//! an absent version is accepted for files containing only optional fields.
//! Unknown TOML values survive saves, but comments and formatting are not retained.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::fs_store::{Expected, FsStore, RelPath};

const STATE_PATH: &str = ".wren/workspace.toml";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug)]
pub struct WorkspaceState {
    root: PathBuf,
    table: Table,
    baseline: Option<Vec<u8>>,
    writable: bool,
    warning: Option<String>,
}

impl WorkspaceState {
    pub fn load(store: &FsStore) -> Self {
        let mut state = Self {
            root: store.root().to_path_buf(),
            table: defaults(),
            baseline: None,
            writable: true,
            warning: None,
        };
        let path = metadata_path();
        let file = match store.read(&path) {
            Ok(file) => file,
            Err(error) if error.is_not_found() => return state,
            Err(error) => {
                state.disable(format!("could not read {STATE_PATH}: {error}"));
                return state;
            }
        };
        state.baseline = Some(file.bytes);
        let bytes = state.baseline.as_deref().unwrap_or_default();
        let parsed = std::str::from_utf8(bytes)
            .map_err(|_| format!("{STATE_PATH} is not UTF-8"))
            .and_then(|text| {
                text.parse::<Table>()
                    .map_err(|error| format!("invalid TOML in {STATE_PATH}: {}", error.message()))
            })
            .and_then(|table| {
                validate(&table)?;
                Ok(table)
            });
        match parsed {
            Ok(table) => state.table = table,
            Err(reason) => state.disable(reason),
        }
        state
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn writable(&self) -> bool {
        self.writable
    }

    /// Save only over the exact bytes loaded (or last published) by this state.
    /// A post-publication durability failure is returned without forgetting the
    /// newly published revision, so a retry does not conflict with our own write.
    pub fn save(&mut self, store: &mut FsStore) -> Result<(), String> {
        if !self.writable {
            return Err(self
                .warning
                .clone()
                .unwrap_or_else(|| "Workspace metadata is read-only".to_owned()));
        }
        if store.root() != self.root {
            return Err("Workspace metadata belongs to a different workspace".to_owned());
        }
        validate(&self.table)?;
        self.table
            .entry("schema_version")
            .or_insert(Value::Integer(SCHEMA_VERSION));
        let bytes = toml::to_string_pretty(&self.table)
            .map_err(|error| format!("Could not serialize {STATE_PATH}: {error}"))?
            .into_bytes();
        let directory = RelPath::new(".wren").expect("constant metadata directory is relative");
        store
            .ensure_dir(&directory)
            .map_err(|error| format!("Could not create .wren: {error}"))?;
        let expected = match self.baseline.as_deref() {
            Some(bytes) => Expected::Bytes(bytes),
            None => Expected::Missing,
        };
        let receipt = store
            .write(&metadata_path(), &bytes, expected)
            .map_err(|error| format!("Could not save {STATE_PATH}: {error}"))?;
        self.record_publication(receipt.file.bytes, receipt.durability_error)
    }

    pub fn selection(&self) -> Option<RelPath> {
        optional_path(&self.table, "selection")
    }

    pub fn set_selection(&mut self, selection: Option<&RelPath>) {
        self.set_optional_path("selection", selection, Reference::Any);
    }

    pub fn last_open_note(&self) -> Option<RelPath> {
        optional_path(&self.table, "last_open_note")
    }

    pub fn set_last_open_note(&mut self, note: Option<&RelPath>) {
        self.set_optional_path("last_open_note", note, Reference::Note);
    }

    pub fn history(&self) -> Vec<RelPath> {
        paths(&self.table, "history")
    }

    pub fn set_history(&mut self, history: &[RelPath]) {
        let encoded = encode_paths(history, Reference::Note);
        match encoded {
            Ok(value) => {
                self.table.insert("history".to_owned(), value);
            }
            Err(reason) => self.disable(reason),
        }
    }

    /// Workspace-relative section/group paths, not note paths. Each sibling
    /// list is obtained by filtering this global list for the relevant parent.
    pub fn order(&self) -> Vec<RelPath> {
        paths(&self.table, "order")
    }

    pub fn reorder(
        &mut self,
        target: &RelPath,
        before: Option<&RelPath>,
        siblings: &[RelPath],
    ) -> Result<(), String> {
        encode_path(target, Reference::Container)?;
        let parent = target.as_path().parent();
        let mut members = BTreeSet::new();
        for sibling in siblings {
            encode_path(sibling, Reference::Container)?;
            if sibling.as_path().parent() != parent {
                return Err("Only sections or groups with the same parent can be reordered".into());
            }
            if !members.insert(sibling.clone()) {
                return Err("The sibling list contains duplicate paths".into());
            }
        }
        if !members.contains(target) {
            return Err("The reorder target is not in the sibling list".into());
        }
        if let Some(before) = before {
            if !members.contains(before) {
                return Err("The insertion target is not in the sibling list".into());
            }
        }

        let existing = self.order();
        let mut ordered: Vec<_> = existing
            .iter()
            .filter(|path| members.contains(*path))
            .cloned()
            .collect();
        let mut missing = members;
        for path in &ordered {
            missing.remove(path);
        }
        // The scan's enumeration order is not stable across filesystems.
        ordered.extend(missing);
        if before != Some(target) {
            ordered.retain(|path| path != target);
            let position = before
                .and_then(|before| ordered.iter().position(|path| path == before))
                .unwrap_or(ordered.len());
            ordered.insert(position, target.clone());
        }
        let sibling_set: BTreeSet<_> = siblings.iter().collect();
        let mut combined: Vec<_> = existing
            .into_iter()
            .filter(|path| !sibling_set.contains(path))
            .collect();
        combined.extend(ordered);
        self.table.insert(
            "order".into(),
            encode_paths(&combined, Reference::Container)?,
        );
        Ok(())
    }

    pub fn layout(&self) -> &str {
        self.table
            .get("layout")
            .and_then(Value::as_str)
            .unwrap_or("columns")
    }

    pub fn set_layout(&mut self, layout: &str) -> Result<(), String> {
        check_layout(layout)?;
        self.table
            .insert("layout".into(), Value::String(layout.into()));
        Ok(())
    }

    /// Returns the stored width, or the specification's default. Unknown panes
    /// return `None`; their TOML values are nevertheless preserved.
    pub fn width(&self, layout: &str, pane: &str) -> Option<u16> {
        let (default, _, _) = width_limits(layout, pane)?;
        Some(
            self.table
                .get("widths")
                .and_then(Value::as_table)
                .and_then(|widths| widths.get(layout))
                .and_then(Value::as_table)
                .and_then(|panes| panes.get(pane))
                .and_then(Value::as_integer)
                .and_then(|width| u16::try_from(width).ok())
                .unwrap_or(default),
        )
    }

    pub fn set_width(&mut self, layout: &str, pane: &str, width: u16) -> Result<(), String> {
        let (_directory, min, max) = width_limits(layout, pane)
            .ok_or_else(|| "Unknown layout or pane for a persisted width".to_owned())?;
        let widths = self
            .table
            .entry("widths")
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| "widths must be a table".to_owned())?;
        let panes = widths
            .entry(layout)
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| "The layout widths must be a table".to_owned())?;
        panes.insert(
            pane.into(),
            Value::Integer(i64::from(width.clamp(min, max))),
        );
        Ok(())
    }

    /// Per-section/group note sort mode. The default is most recently modified.
    pub fn sort(&self, parent: &RelPath) -> &str {
        parent
            .as_path()
            .to_str()
            .and_then(|path| {
                self.table
                    .get("sort")
                    .and_then(Value::as_table)
                    .and_then(|sort| sort.get(path))
                    .and_then(Value::as_str)
            })
            .unwrap_or("modified")
    }

    pub fn set_sort(&mut self, parent: &RelPath, mode: &str) -> Result<(), String> {
        let path = encode_path(parent, Reference::Container)?;
        check_sort(mode)?;
        let sort = self
            .table
            .entry("sort")
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| "sort must be a table".to_owned())?;
        sort.insert(path, Value::String(mode.into()));
        Ok(())
    }

    pub fn remap_paths(&mut self, from: &RelPath, to: &RelPath) {
        self.transform_paths(|path| {
            let path = path.as_path();
            match path.strip_prefix(from.as_path()) {
                Ok(suffix) if suffix.as_os_str().is_empty() => Ok(Some(to.clone())),
                Ok(suffix) => RelPath::new(to.as_path().join(suffix))
                    .map(Some)
                    .map_err(|_| "The remapped metadata path is not workspace-relative".into()),
                Err(_) => RelPath::new(path)
                    .map(Some)
                    .map_err(|_| "The metadata path is not workspace-relative".into()),
            }
        });
    }

    pub fn remove_paths(&mut self, prefix: &RelPath) {
        self.transform_paths(|path| {
            Ok((!path.as_path().starts_with(prefix.as_path())).then(|| path.clone()))
        });
    }

    fn disable(&mut self, reason: String) {
        self.table = defaults();
        self.writable = false;
        self.warning = Some(format!(
            "Workspace metadata is read-only: {reason}. Using defaults; {STATE_PATH} will not be overwritten."
        ));
    }

    fn record_publication(
        &mut self,
        bytes: Vec<u8>,
        durability_error: Option<String>,
    ) -> Result<(), String> {
        self.baseline = Some(bytes);
        self.warning = durability_error.map(|error| {
            format!(
                "{STATE_PATH} was published, but its durability could not be confirmed: {error}"
            )
        });
        match &self.warning {
            Some(warning) => Err(warning.clone()),
            None => Ok(()),
        }
    }

    fn set_optional_path(&mut self, field: &str, path: Option<&RelPath>, kind: Reference) {
        match path.map(|path| encode_path(path, kind)).transpose() {
            Ok(Some(path)) => {
                self.table.insert(field.into(), Value::String(path));
            }
            Ok(None) => {
                self.table.remove(field);
            }
            Err(reason) => self.disable(reason),
        }
    }

    fn transform_paths(&mut self, transform: impl Fn(&RelPath) -> Result<Option<RelPath>, String>) {
        let result = (|| {
            let mut table = self.table.clone();
            for (field, kind) in [
                ("selection", Reference::Any),
                ("last_open_note", Reference::Note),
            ] {
                if let Some(path) = optional_path(&table, field) {
                    match transform(&path)? {
                        Some(path) => {
                            table.insert(field.into(), Value::String(encode_path(&path, kind)?));
                        }
                        None => {
                            table.remove(field);
                        }
                    }
                }
            }
            for (field, kind) in [
                ("history", Reference::Note),
                ("order", Reference::Container),
            ] {
                if table.contains_key(field) {
                    let mut mapped = Vec::new();
                    for path in paths(&table, field) {
                        if let Some(path) = transform(&path)? {
                            if field != "order" || !mapped.contains(&path) {
                                mapped.push(path);
                            }
                        }
                    }
                    table.insert(field.into(), encode_paths(&mapped, kind)?);
                }
            }
            if let Some(sort) = table.get_mut("sort").and_then(Value::as_table_mut) {
                let mut mapped = Table::new();
                let mut changed = Vec::new();
                for (key, mode) in sort.iter() {
                    let path = parse_path(key, Reference::Container)?;
                    if let Some(path) = transform(&path)? {
                        let new_key = encode_path(&path, Reference::Container)?;
                        if new_key == *key {
                            mapped.insert(new_key, mode.clone());
                        } else {
                            changed.push((new_key, mode.clone()));
                        }
                    }
                }
                // A stale destination reference must not override the moved
                // container's sort mode.
                mapped.extend(changed);
                *sort = mapped;
            }
            validate(&table)?;
            Ok(table)
        })();
        match result {
            Ok(table) => self.table = table,
            Err(reason) => self.disable(reason),
        }
    }
}

#[derive(Clone, Copy)]
enum Reference {
    Any,
    Note,
    Container,
}

fn metadata_path() -> RelPath {
    RelPath::new(STATE_PATH).expect("constant metadata path is relative")
}

fn defaults() -> Table {
    let mut table = Table::new();
    table.insert("schema_version".into(), Value::Integer(SCHEMA_VERSION));
    table.insert("layout".into(), Value::String("columns".into()));
    table
}

fn optional_path(table: &Table, field: &str) -> Option<RelPath> {
    table
        .get(field)
        .and_then(Value::as_str)
        .and_then(|path| parse_path(path, Reference::Any).ok())
}

fn paths(table: &Table, field: &str) -> Vec<RelPath> {
    table
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|path| parse_path(path, Reference::Any).ok())
        .collect()
}

fn encode_path(path: &RelPath, kind: Reference) -> Result<String, String> {
    let path = path
        .as_path()
        .to_str()
        .ok_or_else(|| "Metadata references must be UTF-8".to_owned())?;
    parse_path(path, kind)?;
    Ok(path.to_owned())
}

fn encode_paths(paths: &[RelPath], kind: Reference) -> Result<Value, String> {
    paths
        .iter()
        .map(|path| encode_path(path, kind).map(Value::String))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn parse_path(path: &str, kind: Reference) -> Result<RelPath, String> {
    // Validate the portable spelling before Path normalizes away components or
    // treats a Windows absolute path as an ordinary Unix filename.
    if path.is_empty()
        || path.contains('\\')
        || path.contains('\0')
        || path.starts_with('/')
        || path.as_bytes().get(1) == Some(&b':')
        || path
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return Err("Metadata contains an invalid workspace-relative reference".into());
    }
    let depth = path.split('/').count();
    let markdown = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    match kind {
        Reference::Note if depth < 2 || !markdown => {
            return Err("History and last-open-note references must name Markdown notes".into());
        }
        // Directories may themselves end in .md. Session validates item kinds
        // against the scanned model; a filename extension is not a type check.
        Reference::Container if depth > 2 => {
            return Err(
                "Ordering and sort references must name sections or groups, not notes".into(),
            );
        }
        _ => {}
    }
    RelPath::new(path)
        .map_err(|_| "Metadata contains an invalid workspace-relative reference".into())
}

fn check_layout(layout: &str) -> Result<(), String> {
    match layout {
        "columns" | "outline" => Ok(()),
        _ => Err("The workspace layout must be columns or outline".into()),
    }
}

fn check_sort(mode: &str) -> Result<(), String> {
    match mode {
        "modified" | "title" => Ok(()),
        _ => Err("The note sort mode must be modified or title".into()),
    }
}

fn width_limits(layout: &str, pane: &str) -> Option<(u16, u16, u16)> {
    match (layout, pane) {
        ("columns", "sections") => Some((200, 140, 360)),
        ("columns", "groups") => Some((220, 150, 400)),
        ("columns" | "outline", "notes") => Some((280, 200, 480)),
        ("outline", "sections") => Some((260, 180, 420)),
        _ => None,
    }
}

fn validate(table: &Table) -> Result<(), String> {
    if let Some(version) = table.get("schema_version") {
        if version.as_integer() != Some(SCHEMA_VERSION) {
            return Err("Unsupported workspace metadata schema_version (expected 1)".into());
        }
    }
    if let Some(layout) = table.get("layout") {
        check_layout(
            layout
                .as_str()
                .ok_or_else(|| "layout must be a string".to_owned())?,
        )?;
    }
    for (field, kind) in [
        ("selection", Reference::Any),
        ("last_open_note", Reference::Note),
    ] {
        if let Some(value) = table.get(field) {
            parse_path(
                value
                    .as_str()
                    .ok_or_else(|| format!("{field} must be a workspace-relative path string"))?,
                kind,
            )?;
        }
    }
    for (field, kind) in [
        ("history", Reference::Note),
        ("order", Reference::Container),
    ] {
        if let Some(value) = table.get(field) {
            let entries = value
                .as_array()
                .ok_or_else(|| format!("{field} must be an array of path strings"))?;
            let mut seen = BTreeSet::new();
            for entry in entries {
                let path = parse_path(
                    entry
                        .as_str()
                        .ok_or_else(|| format!("{field} must contain only path strings"))?,
                    kind,
                )?;
                if field == "order" && !seen.insert(path) {
                    return Err("order must not contain duplicate paths".into());
                }
            }
        }
    }
    if let Some(value) = table.get("widths") {
        let widths = value
            .as_table()
            .ok_or_else(|| "widths must be a table".to_owned())?;
        for layout in ["columns", "outline"] {
            if let Some(value) = widths.get(layout) {
                let panes = value
                    .as_table()
                    .ok_or_else(|| "The layout widths must be tables".to_owned())?;
                for (pane, value) in panes {
                    if let Some((_, min, max)) = width_limits(layout, pane) {
                        let width = value
                            .as_integer()
                            .ok_or_else(|| "Pane widths must be integers".to_owned())?;
                        if !(i64::from(min)..=i64::from(max)).contains(&width) {
                            return Err(
                                "A persisted pane width is outside its supported range".into()
                            );
                        }
                    }
                }
            }
        }
    }
    if let Some(value) = table.get("sort") {
        let sort = value
            .as_table()
            .ok_or_else(|| "sort must be a table keyed by section/group paths".to_owned())?;
        for (path, mode) in sort {
            parse_path(path, Reference::Container)?;
            check_sort(
                mode.as_str()
                    .ok_or_else(|| "Sort modes must be strings".to_owned())?,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::{tempdir, TempDir};

    use super::*;

    fn rel(path: &str) -> RelPath {
        RelPath::new(path).unwrap()
    }

    fn fixture(bytes: Option<&[u8]>) -> (TempDir, FsStore) {
        let directory = tempdir().unwrap();
        if let Some(bytes) = bytes {
            fs::create_dir(directory.path().join(".wren")).unwrap();
            fs::write(directory.path().join(STATE_PATH), bytes).unwrap();
        }
        let store = FsStore::open(directory.path()).unwrap();
        (directory, store)
    }

    fn disk_table(directory: &TempDir) -> Table {
        fs::read_to_string(directory.path().join(STATE_PATH))
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn missing_optional_metadata_loads_defaults_and_round_trips() {
        let (directory, mut store) = fixture(None);
        let mut state = WorkspaceState::load(&store);
        assert!(state.writable());
        assert_eq!(state.warning(), None);
        assert_eq!(state.layout(), "columns");
        assert_eq!(state.selection(), None);
        assert!(state.history().is_empty());
        assert!(state.order().is_empty());
        assert!(!directory.path().join(".wren").exists());

        let note = rel("Projects/Group/note.md");
        state.set_layout("outline").unwrap();
        state.set_selection(Some(&note));
        state.set_last_open_note(Some(&note));
        state.set_history(&[note.clone(), rel("Inbox/other.md"), note.clone()]);
        state.set_width("columns", "groups", 268).unwrap();
        state.set_width("outline", "sections", 310).unwrap();
        state.set_sort(&rel("Projects/Group"), "title").unwrap();
        state.save(&mut store).unwrap();

        let loaded = WorkspaceState::load(&store);
        assert!(loaded.writable());
        assert_eq!(loaded.warning(), None);
        assert_eq!(loaded.layout(), "outline");
        assert_eq!(loaded.selection(), Some(note.clone()));
        assert_eq!(loaded.last_open_note(), Some(note.clone()));
        assert_eq!(
            loaded.history(),
            [note.clone(), rel("Inbox/other.md"), note]
        );
        assert_eq!(loaded.width("columns", "groups"), Some(268));
        assert_eq!(loaded.width("outline", "sections"), Some(310));
        assert_eq!(loaded.sort(&rel("Projects/Group")), "title");
        assert_eq!(loaded.sort(&rel("Inbox")), "modified");
        assert_eq!(
            disk_table(&directory)["schema_version"].as_integer(),
            Some(1)
        );
    }

    #[test]
    fn empty_metadata_is_valid() {
        let (_directory, mut store) = fixture(Some(b""));
        let mut state = WorkspaceState::load(&store);
        assert!(state.writable());
        assert_eq!(state.layout(), "columns");
        state.save(&mut store).unwrap();
        assert!(WorkspaceState::load(&store).writable());
    }

    #[test]
    fn unknown_valid_toml_values_survive_known_field_updates() {
        let source = br#"
            schema_version = 1
            layout = 'columns'
            future_flag = true
            future_array = [1, 'two', { three = 3 }]
            future_date = 2026-11-04T12:30:00Z
            future_float = 1.25
            "dotted.literal.key" = 'unchanged'

            [widths.columns]
            sections = 200
            future_pane = { unusual = [false, 2] }

            [widths.future_layout]
            pane = 'arbitrary'

            [plugin]
            selection = '../plugin-owned-value'
            path = 'A'
            [[plugin.entries]]
            id = 'one'
            [[plugin.entries]]
            id = 'two'
        "#;
        let original: Table = std::str::from_utf8(source).unwrap().parse().unwrap();
        let (directory, mut store) = fixture(Some(source));
        let mut state = WorkspaceState::load(&store);
        assert!(state.writable(), "{:?}", state.warning());
        state.set_layout("outline").unwrap();
        state.set_width("columns", "sections", 300).unwrap();
        state.remap_paths(&rel("A"), &rel("B"));
        state.save(&mut store).unwrap();
        let saved = disk_table(&directory);
        for key in [
            "future_flag",
            "future_array",
            "future_date",
            "future_float",
            "dotted.literal.key",
            "plugin",
        ] {
            assert_eq!(saved[key], original[key], "{key}");
        }
        assert_eq!(
            saved["widths"]["columns"]["future_pane"],
            original["widths"]["columns"]["future_pane"]
        );
        assert_eq!(
            saved["widths"]["future_layout"],
            original["widths"]["future_layout"]
        );
        assert_eq!(saved["layout"].as_str(), Some("outline"));
    }

    #[test]
    fn malformed_utf8_and_unsupported_metadata_are_never_overwritten() {
        let cases: &[&[u8]] = &[
            b"layout = [",
            b"\xff\xfe",
            b"schema_version = 2\nlayout = 'outline'",
            b"schema_version = '1'",
            b"schema_version = true",
            b"layout = 'future'",
            b"layout = 1",
            b"selection = ['A']",
            b"history = 'A/note.md'",
            b"history = [1]",
            b"last_open_note = 'A'",
            b"order = ['A', 'A']",
            b"order = ['A/B/C']",
            b"widths = 4",
            b"[widths]\ncolumns = []",
            b"[widths.columns]\nsections = -1",
            b"[widths.columns]\nsections = 361",
            b"[widths.columns]\ngroups = 149",
            b"[widths.outline]\nnotes = 481",
            b"[widths.outline]\nsections = 179",
            b"[widths.columns]\nnotes = 280.0",
            b"[widths.columns]\nnotes = true",
            b"sort = 'title'",
            b"[sort]\n'A' = 'future'",
        ];
        for bytes in cases {
            let (directory, mut store) = fixture(Some(bytes));
            let mut state = WorkspaceState::load(&store);
            assert!(!state.writable(), "{}", String::from_utf8_lossy(bytes));
            assert!(state.warning().is_some());
            assert_eq!(state.layout(), "columns");
            assert_eq!(state.selection(), None);
            assert_eq!(state.last_open_note(), None);
            assert!(state.history().is_empty());
            assert!(state.order().is_empty());
            assert_eq!(state.width("columns", "sections"), Some(200));
            state.set_layout("outline").unwrap();
            assert!(state.save(&mut store).is_err());
            assert_eq!(fs::read(directory.path().join(STATE_PATH)).unwrap(), *bytes);
        }
    }

    #[test]
    fn unsafe_references_make_the_entire_metadata_read_only() {
        let unsafe_paths = [
            "",
            ".",
            "..",
            "../A/note.md",
            "/A/note.md",
            "A/../B/note.md",
            "A/./note.md",
            "A//note.md",
            "A/note.md/",
            "A/.hidden/note.md",
            ".wren/workspace.toml",
            "C:/A/note.md",
            "C:\\A\\note.md",
            "\\\\server\\share\\note.md",
        ];
        for path in unsafe_paths {
            for field in ["selection", "last_open_note", "history", "order", "sort"] {
                let value = Value::String(path.into());
                let mut source = Table::new();
                source.insert("layout".into(), Value::String("outline".into()));
                source.insert(
                    field.into(),
                    match field {
                        "history" | "order" => Value::Array(vec![value]),
                        "sort" => {
                            let mut sort = Table::new();
                            sort.insert(path.into(), Value::String("title".into()));
                            Value::Table(sort)
                        }
                        _ => value,
                    },
                );
                let bytes = toml::to_string(&source).unwrap().into_bytes();
                let (directory, mut store) = fixture(Some(&bytes));
                let mut state = WorkspaceState::load(&store);
                assert!(!state.writable(), "{field}: {path:?}");
                assert!(state.warning().is_some());
                assert_eq!(state.layout(), "columns");
                assert!(state.save(&mut store).is_err());
                assert_eq!(fs::read(directory.path().join(STATE_PATH)).unwrap(), bytes);
            }
        }
    }

    #[test]
    fn non_missing_read_failure_is_not_treated_as_missing_metadata() {
        let (directory, mut store) = fixture(None);
        fs::create_dir_all(directory.path().join(STATE_PATH)).unwrap();
        let mut state = WorkspaceState::load(&store);
        assert!(!state.writable());
        assert!(state.warning().is_some());
        assert!(state.save(&mut store).is_err());
        assert!(directory.path().join(STATE_PATH).is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_metadata_is_read_only_and_its_bytes_are_preserved() {
        use std::os::unix::fs::PermissionsExt;

        let original = b"layout = 'outline'\n";
        let (directory, mut store) = fixture(Some(original));
        let path = directory.path().join(STATE_PATH);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
        let read = store.read(&metadata_path());
        let mut state = WorkspaceState::load(&store);
        let save = state.save(&mut store);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            read.is_err(),
            "This permission test requires an unprivileged test process"
        );
        assert!(!read.unwrap_err().is_not_found());
        assert!(!state.writable());
        assert!(state.warning().is_some());
        assert_eq!(state.layout(), "columns");
        assert!(save.is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_dangling_symlink_metadata_are_not_overwritten() {
        use std::os::unix::fs::symlink;

        for exists in [false, true] {
            let (directory, mut store) = fixture(None);
            let outside = tempdir().unwrap();
            let target = outside.path().join("external.toml");
            if exists {
                fs::write(&target, b"layout = 'outline'").unwrap();
            }
            fs::create_dir(directory.path().join(".wren")).unwrap();
            symlink(&target, directory.path().join(STATE_PATH)).unwrap();
            let mut state = WorkspaceState::load(&store);
            assert!(!state.writable());
            assert!(state.warning().is_some());
            assert!(state.save(&mut store).is_err());
            assert!(fs::symlink_metadata(directory.path().join(STATE_PATH))
                .unwrap()
                .file_type()
                .is_symlink());
            if exists {
                assert_eq!(fs::read(target).unwrap(), b"layout = 'outline'");
            } else {
                assert!(!target.exists());
            }
        }
    }

    #[test]
    fn same_length_external_metadata_edit_is_not_overwritten() {
        let original = b"layout = 'columns'\n";
        let external = b"layout = 'outline'\n";
        assert_eq!(original.len(), external.len());
        let (directory, mut store) = fixture(Some(original));
        let mut state = WorkspaceState::load(&store);
        state.set_selection(Some(&rel("A/note.md")));
        fs::write(directory.path().join(STATE_PATH), external).unwrap();
        assert!(state.save(&mut store).is_err());
        assert_eq!(
            fs::read(directory.path().join(STATE_PATH)).unwrap(),
            external
        );
    }

    #[test]
    fn externally_created_or_removed_metadata_is_a_conflict() {
        let (directory, mut store) = fixture(None);
        let mut state = WorkspaceState::load(&store);
        fs::create_dir(directory.path().join(".wren")).unwrap();
        let external = b"layout = 'outline'\n";
        fs::write(directory.path().join(STATE_PATH), external).unwrap();
        assert!(state.save(&mut store).is_err());
        assert_eq!(
            fs::read(directory.path().join(STATE_PATH)).unwrap(),
            external
        );

        let mut state = WorkspaceState::load(&store);
        fs::remove_file(directory.path().join(STATE_PATH)).unwrap();
        assert!(state.save(&mut store).is_err());
        assert!(!directory.path().join(STATE_PATH).exists());
    }

    #[test]
    fn successful_save_advances_the_baseline_for_the_next_save() {
        let (_directory, mut store) = fixture(None);
        let mut state = WorkspaceState::load(&store);
        state.save(&mut store).unwrap();
        state.set_layout("outline").unwrap();
        state.save(&mut store).unwrap();
        state.set_selection(Some(&rel("Inbox/note.md")));
        state.save(&mut store).unwrap();
        let loaded = WorkspaceState::load(&store);
        assert_eq!(loaded.layout(), "outline");
        assert_eq!(loaded.selection(), Some(rel("Inbox/note.md")));
    }

    #[test]
    fn prepublication_failures_preserve_bytes_and_allow_a_retry() {
        use crate::fs_store::Fault;

        for fault in [
            Fault::Write,
            Fault::Permissions,
            Fault::FileSync,
            Fault::BeforePublish,
        ] {
            let original = b"layout = 'columns'\n";
            let (directory, mut store) = fixture(Some(original));
            let mut state = WorkspaceState::load(&store);
            state.set_layout("outline").unwrap();
            store.fail_next(fault);
            assert!(state.save(&mut store).is_err(), "{fault:?}");
            assert_eq!(
                fs::read(directory.path().join(STATE_PATH)).unwrap(),
                original
            );
            assert_eq!(state.baseline.as_deref(), Some(original.as_slice()));
            state.save(&mut store).unwrap();
            assert_eq!(WorkspaceState::load(&store).layout(), "outline");
        }
    }

    #[test]
    fn metadata_failure_after_a_document_mutation_does_not_roll_it_back() {
        let original = b"layout = [";
        let (directory, mut store) = fixture(Some(original));
        let mut state = WorkspaceState::load(&store);
        store.ensure_dir(&rel("A")).unwrap();
        store
            .write(&rel("A/note.md"), b"new note", Expected::Missing)
            .unwrap();
        state.set_selection(Some(&rel("A/note.md")));
        assert!(state.save(&mut store).is_err());
        assert_eq!(
            fs::read(directory.path().join("A/note.md")).unwrap(),
            b"new note"
        );
        assert_eq!(
            fs::read(directory.path().join(STATE_PATH)).unwrap(),
            original
        );
    }

    #[test]
    fn post_publication_durability_error_still_advances_the_baseline() {
        let (directory, mut store) = fixture(Some(b"layout = 'columns'\n"));
        let mut state = WorkspaceState::load(&store);
        state.set_layout("outline").unwrap();
        store.fail_next(crate::fs_store::Fault::DirectorySync);
        assert!(state
            .save(&mut store)
            .unwrap_err()
            .contains("was published"));
        let published = fs::read(directory.path().join(STATE_PATH)).unwrap();
        assert_eq!(state.baseline.as_deref(), Some(published.as_slice()));
        assert_eq!(WorkspaceState::load(&store).layout(), "outline");
        assert!(state.writable());
        assert!(state.warning().unwrap().contains("durability"));
        state.set_selection(Some(&rel("A/note.md")));
        state.save(&mut store).unwrap();
        assert_eq!(state.warning(), None);
        assert_eq!(
            WorkspaceState::load(&store).selection(),
            Some(rel("A/note.md"))
        );
    }

    #[test]
    fn state_cannot_be_saved_to_another_workspace() {
        let (_directory, store) = fixture(None);
        let (other_directory, mut other_store) = fixture(None);
        let mut state = WorkspaceState::load(&store);
        assert!(state.save(&mut other_store).is_err());
        assert!(!other_directory.path().join(".wren").exists());
    }

    #[test]
    fn reorder_appends_missing_siblings_in_deterministic_order() {
        let (_directory, mut store) = fixture(Some(b"order = ['Z']\n"));
        let mut state = WorkspaceState::load(&store);
        let siblings = [rel("Z"), rel("M"), rel("A")];
        state
            .reorder(&rel("M"), Some(&rel("Z")), &siblings)
            .unwrap();
        assert_eq!(state.order(), [rel("M"), rel("Z"), rel("A")]);
        state.save(&mut store).unwrap();
        assert_eq!(WorkspaceState::load(&store).order(), state.order());

        state
            .reorder(&rel("M"), None, &[rel("B"), rel("A"), rel("M"), rel("Z")])
            .unwrap();
        assert_eq!(state.order(), [rel("Z"), rel("A"), rel("B"), rel("M")]);
    }

    #[test]
    fn group_reorder_preserves_other_sibling_lists_and_self_drop_is_a_noop() {
        let (_directory, store) = fixture(Some(b"order = ['B', 'A', 'A/Y', 'A/X', 'B/G']\n"));
        let mut state = WorkspaceState::load(&store);
        let siblings = [rel("A/X"), rel("A/Y"), rel("A/Z")];
        state
            .reorder(&rel("A/Y"), Some(&rel("A/Y")), &siblings)
            .unwrap();
        let group_order: Vec<_> = state
            .order()
            .into_iter()
            .filter(|path| path.as_path().parent() == Some(Path::new("A")))
            .collect();
        assert_eq!(group_order, [rel("A/Y"), rel("A/X"), rel("A/Z")]);
        state
            .reorder(&rel("A/Z"), Some(&rel("A/X")), &siblings)
            .unwrap();
        assert_eq!(
            state.order(),
            [
                rel("B"),
                rel("A"),
                rel("B/G"),
                rel("A/Y"),
                rel("A/Z"),
                rel("A/X")
            ]
        );
    }

    #[test]
    fn container_names_with_markdown_extensions_round_trip_as_directories() {
        let (_directory, mut store) = fixture(None);
        let section = rel("Archive.md");
        let group = rel("Archive.md/Group.md");
        store.ensure_dir(&group).unwrap();
        let mut state = WorkspaceState::load(&store);
        state
            .reorder(&section, None, std::slice::from_ref(&section))
            .unwrap();
        state
            .reorder(&group, None, std::slice::from_ref(&group))
            .unwrap();
        state.set_sort(&group, "title").unwrap();
        state.save(&mut store).unwrap();
        let loaded = WorkspaceState::load(&store);
        assert!(loaded.writable());
        assert_eq!(loaded.order(), [section, group.clone()]);
        assert_eq!(loaded.sort(&group), "title");
    }

    #[test]
    fn invalid_reorders_do_not_change_metadata() {
        let (_directory, store) = fixture(Some(b"order = ['A', 'B']\n"));
        let mut state = WorkspaceState::load(&store);
        let original = state.table.clone();
        assert!(state
            .reorder(&rel("A/B/deep"), None, &[rel("A/B/deep")])
            .is_err());
        assert!(state
            .reorder(&rel("A/X"), None, &[rel("A/X"), rel("B/Y")])
            .is_err());
        assert!(state.reorder(&rel("A"), None, &[rel("B")]).is_err());
        assert!(state
            .reorder(&rel("A"), Some(&rel("C")), &[rel("A"), rel("B")])
            .is_err());
        assert!(state
            .reorder(&rel("A"), None, &[rel("A"), rel("A")])
            .is_err());
        assert_eq!(state.table, original);
    }

    fn referenced_source() -> &'static [u8] {
        br#"
            selection = 'A/G/note.md'
            last_open_note = 'A/G/deep/other.md'
            history = ['A/G/note.md', 'AB/G/note.md', 'A/loose.md', 'A/G/note.md']
            order = ['A', 'A/G', 'AB', 'AB/G']
            [sort]
            'A/G' = 'title'
            'AB/G' = 'modified'
            [plugin]
            selection = 'A/G/note.md'
        "#
    }

    #[test]
    fn rename_remaps_all_known_references_component_wise_and_persists_them() {
        let (directory, mut store) = fixture(Some(referenced_source()));
        let mut state = WorkspaceState::load(&store);
        assert!(state.writable());
        state.remap_paths(&rel("A"), &rel("Renamed"));
        assert_eq!(state.selection(), Some(rel("Renamed/G/note.md")));
        assert_eq!(state.last_open_note(), Some(rel("Renamed/G/deep/other.md")));
        assert_eq!(
            state.history(),
            [
                rel("Renamed/G/note.md"),
                rel("AB/G/note.md"),
                rel("Renamed/loose.md"),
                rel("Renamed/G/note.md")
            ]
        );
        assert_eq!(
            state.order(),
            [rel("Renamed"), rel("Renamed/G"), rel("AB"), rel("AB/G")]
        );
        assert_eq!(state.sort(&rel("Renamed/G")), "title");
        assert_eq!(state.sort(&rel("A/G")), "modified");
        state.save(&mut store).unwrap();
        let loaded = WorkspaceState::load(&store);
        assert_eq!(loaded.history(), state.history());
        assert_eq!(loaded.order(), state.order());
        assert_eq!(
            disk_table(&directory)["plugin"]["selection"].as_str(),
            Some("A/G/note.md")
        );
    }

    #[test]
    fn group_and_note_moves_remap_only_matching_descendants() {
        let (_directory, store) = fixture(Some(referenced_source()));
        let mut state = WorkspaceState::load(&store);
        state.remap_paths(&rel("A/G"), &rel("B/H"));
        assert_eq!(state.selection(), Some(rel("B/H/note.md")));
        assert_eq!(state.last_open_note(), Some(rel("B/H/deep/other.md")));
        assert_eq!(state.sort(&rel("B/H")), "title");
        assert_eq!(
            state.order(),
            [rel("A"), rel("B/H"), rel("AB"), rel("AB/G")]
        );
        assert!(state.history().contains(&rel("A/loose.md")));
        assert!(state.history().contains(&rel("AB/G/note.md")));
        state.remap_paths(&rel("B/H/note.md"), &rel("B/H/renamed.md"));
        assert_eq!(state.selection(), Some(rel("B/H/renamed.md")));
        assert_eq!(state.history()[0], rel("B/H/renamed.md"));
        assert_eq!(state.history()[3], rel("B/H/renamed.md"));
    }

    #[test]
    fn delete_removes_all_known_references_but_not_similar_names_or_unknown_keys() {
        let (directory, mut store) = fixture(Some(referenced_source()));
        let mut state = WorkspaceState::load(&store);
        state.remove_paths(&rel("A"));
        assert_eq!(state.selection(), None);
        assert_eq!(state.last_open_note(), None);
        assert_eq!(state.history(), [rel("AB/G/note.md")]);
        assert_eq!(state.order(), [rel("AB"), rel("AB/G")]);
        assert!(!state.table["sort"].as_table().unwrap().contains_key("A/G"));
        state.save(&mut store).unwrap();
        let loaded = WorkspaceState::load(&store);
        assert_eq!(loaded.selection(), None);
        assert_eq!(loaded.history(), [rel("AB/G/note.md")]);
        assert_eq!(
            disk_table(&directory)["plugin"]["selection"].as_str(),
            Some("A/G/note.md")
        );
    }

    #[test]
    fn remapping_deduplicates_stale_order_and_keeps_the_sources_sort_mode() {
        let (_directory, store) = fixture(Some(
            b"order = ['A', 'B']\n[sort]\n'A' = 'title'\n'B' = 'modified'\n",
        ));
        let mut state = WorkspaceState::load(&store);
        state.remap_paths(&rel("A"), &rel("B"));
        assert!(state.writable());
        assert_eq!(state.order(), [rel("B")]);
        assert_eq!(state.sort(&rel("B")), "title");
    }

    #[test]
    fn pane_widths_use_defaults_and_setters_clamp_to_supported_ranges() {
        let (_directory, store) = fixture(None);
        let mut state = WorkspaceState::load(&store);
        for (layout, pane, default, min, max) in [
            ("columns", "sections", 200, 140, 360),
            ("columns", "groups", 220, 150, 400),
            ("columns", "notes", 280, 200, 480),
            ("outline", "sections", 260, 180, 420),
            ("outline", "notes", 280, 200, 480),
        ] {
            assert_eq!(state.width(layout, pane), Some(default));
            state.set_width(layout, pane, 0).unwrap();
            assert_eq!(state.width(layout, pane), Some(min));
            state.set_width(layout, pane, u16::MAX).unwrap();
            assert_eq!(state.width(layout, pane), Some(max));
        }
        assert_eq!(state.width("future", "notes"), None);
        assert!(state.set_width("columns", "future", 200).is_err());
        assert!(state.set_layout("future").is_err());
        assert!(state.set_sort(&rel("A"), "future").is_err());
    }

    #[test]
    fn clearing_optional_references_does_not_leave_empty_invalid_paths() {
        let (_directory, mut store) = fixture(Some(referenced_source()));
        let mut state = WorkspaceState::load(&store);
        state.set_selection(None);
        state.set_last_open_note(None);
        state.set_history(&[]);
        state.save(&mut store).unwrap();
        let loaded = WorkspaceState::load(&store);
        assert!(loaded.writable());
        assert_eq!(loaded.selection(), None);
        assert_eq!(loaded.last_open_note(), None);
        assert!(loaded.history().is_empty());
    }
}
