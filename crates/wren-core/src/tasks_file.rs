//! `.wren/tasks.toml`: what the task views remember about a workspace,
//! beside `.wren/workspace.toml` and separate from it so that file stays
//! as released Wren reads it. App-managed, and written only while the
//! Session's tasks are on and something has been set: opening a workspace
//! never creates it.
//!
//! ```toml
//! schema_version = 1
//! exclude = ["Home/Shopping", "Archive"]
//!
//! [views."Work/Meetings"]
//! statuses = ["open", "doing"]
//! owner = "@sam"
//! group = "due"
//! hide_old = false
//! layout = "board"
//! ```
//!
//! - `exclude` lists sections and groups whose tasks every rollup leaves
//!   out.
//! - `[views."<path>"]` holds the filters remembered for the view of one
//!   scope: a section, a group or a note by its path, and the workspace as
//!   `"."`. A key is written only while it differs from its default, and a
//!   table only while it has a key. `layout` is the view's layout, kept
//!   beside its filters: written only as `"board"`, and read as the list
//!   whatever else it says.
//!
//! The file is read leniently: an entry Wren cannot read (a path outside
//! the workspace, an unknown word) is passed over, not an error. Unknown
//! keys survive a save; comments and formatting do not. A file that is not
//! TOML, or of another `schema_version`, is left alone: defaults apply and
//! nothing is written over it.

use std::path::PathBuf;

use toml::{Table, Value};
use wren_markdown::status::StatusType;
use wren_markdown::task::Priority;

use crate::fs_store::{Expected, FsStore, RelPath};

const TASKS_PATH: &str = ".wren/tasks.toml";
const SCHEMA_VERSION: i64 = 1;
const EXCLUDE: &str = "exclude";
const VIEWS: &str = "views";
/// The key of the workspace's own view.
const WORKSPACE: &str = ".";

const STATUSES: [StatusType; 4] = [
    StatusType::Open,
    StatusType::Doing,
    StatusType::Done,
    StatusType::Cancelled,
];

/// Whose tasks a view shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum OwnerFilter {
    #[default]
    Anyone,
    /// Tasks with no owner, or one of the `tasks-me` names.
    Me,
    /// One owner, by name without the `@`.
    Name(String),
}

/// What a view's rows are grouped by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GroupBy {
    #[default]
    Note,
    Due,
    Priority,
    Owner,
}

/// The due buckets a view groups and filters by, in the order a view
/// lists them. Weeks run Monday to Sunday.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DueBucket {
    /// Unfinished and due before today.
    Overdue,
    /// Finished, and due before today.
    Earlier,
    Today,
    /// Due after today, by Sunday.
    ThisWeek,
    /// Due in the week after this one.
    NextWeek,
    /// Due after next week.
    Later,
    /// No due date, or one that is no real day.
    NoDate,
}

impl DueBucket {
    /// Every bucket, in the order a view lists them.
    pub const ALL: [Self; 7] = [
        Self::Overdue,
        Self::Earlier,
        Self::Today,
        Self::ThisWeek,
        Self::NextWeek,
        Self::Later,
        Self::NoDate,
    ];

    /// The bucket's name, as a view shows it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overdue => "Overdue",
            Self::Earlier => "Earlier",
            Self::Today => "Today",
            Self::ThisWeek => "This week",
            Self::NextWeek => "Next week",
            Self::Later => "Later",
            Self::NoDate => "No date",
        }
    }
}

/// How a view lays its tasks out: rows grouped in a list, or cards in a
/// column for each status (discovery/tasks.md §11).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    List,
    Board,
}

impl Layout {
    /// The other layout: what the List | Board switch flips to.
    pub const fn other(self) -> Self {
        match self {
            Self::List => Self::Board,
            Self::Board => Self::List,
        }
    }
}

/// The priority a view narrows to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityFilter {
    Level(Priority),
    /// Tasks with no priority.
    Unset,
}

/// The filters a view remembers for one scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFilters {
    /// The status chips pressed; none pressed shows every status.
    pub statuses: Vec<StatusType>,
    pub owner: OwnerFilter,
    pub group_by: GroupBy,
    /// Hide done and cancelled tasks whose stamp is older than 14 days.
    pub hide_old: bool,
    pub due: Option<DueBucket>,
    pub priority: Option<PriorityFilter>,
    /// Not a filter, but remembered with them for the scope: it narrows
    /// nothing.
    pub layout: Layout,
}

impl Default for TaskFilters {
    fn default() -> Self {
        Self {
            statuses: Vec::new(),
            owner: OwnerFilter::Anyone,
            group_by: GroupBy::Note,
            hide_old: true,
            due: None,
            priority: None,
            layout: Layout::List,
        }
    }
}

const GROUPS: [(GroupBy, &str); 4] = [
    (GroupBy::Note, "note"),
    (GroupBy::Due, "due"),
    (GroupBy::Priority, "priority"),
    (GroupBy::Owner, "owner"),
];

const BUCKETS: [(DueBucket, &str); 7] = [
    (DueBucket::Overdue, "overdue"),
    (DueBucket::Earlier, "earlier"),
    (DueBucket::Today, "today"),
    (DueBucket::ThisWeek, "week"),
    (DueBucket::NextWeek, "next"),
    (DueBucket::Later, "later"),
    (DueBucket::NoDate, "none"),
];

/// The word a priority filter is stored as.
fn priority_word(filter: PriorityFilter) -> String {
    match filter {
        PriorityFilter::Level(level) => level.name().to_ascii_lowercase(),
        PriorityFilter::Unset => "none".into(),
    }
}

fn priority_from(word: &str) -> Option<PriorityFilter> {
    if word == "none" {
        return Some(PriorityFilter::Unset);
    }
    Priority::ALL
        .into_iter()
        .find(|level| level.name().eq_ignore_ascii_case(word))
        .map(PriorityFilter::Level)
}

impl TaskFilters {
    /// The filters `table` holds; a key that is missing or unreadable is
    /// its default.
    fn read(table: &Table) -> Self {
        let word = |key: &str| table.get(key).and_then(Value::as_str);
        let mut statuses = Vec::new();
        for name in (table.get("statuses").and_then(Value::as_array))
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let status = STATUSES.into_iter().find(|status| status.name() == name);
            if let Some(status) = status.filter(|status| !statuses.contains(status)) {
                statuses.push(status);
            }
        }
        let owner = match word("owner") {
            Some("me") => OwnerFilter::Me,
            Some(name) => match name.strip_prefix('@') {
                Some(name) if !name.is_empty() => OwnerFilter::Name(name.to_owned()),
                _ => OwnerFilter::Anyone,
            },
            None => OwnerFilter::Anyone,
        };
        Self {
            statuses,
            owner,
            group_by: word("group")
                .and_then(|word| GROUPS.iter().find(|(_, name)| *name == word))
                .map_or(GroupBy::Note, |(group, _)| *group),
            hide_old: (table.get("hide_old").and_then(Value::as_bool)).unwrap_or(true),
            due: word("due")
                .and_then(|word| BUCKETS.iter().find(|(_, name)| *name == word))
                .map(|(bucket, _)| *bucket),
            priority: word("priority").and_then(priority_from),
            layout: match word("layout") {
                Some("board") => Layout::Board,
                _ => Layout::List,
            },
        }
    }

    /// Write the filters into `table`: each key that differs from its
    /// default, and no other of the keys Wren knows.
    fn write(&self, table: &mut Table) {
        let defaults = Self::default();
        let mut set = |key: &str, value: Option<Value>| {
            match value {
                Some(value) => table.insert(key.into(), value),
                None => table.remove(key),
            };
        };
        set(
            "statuses",
            (!self.statuses.is_empty()).then(|| {
                Value::Array(
                    // In the order the chips stand, whatever order they
                    // were pressed in.
                    (STATUSES.into_iter())
                        .filter(|status| self.statuses.contains(status))
                        .map(|status| Value::String(status.name().into()))
                        .collect(),
                )
            }),
        );
        set(
            "owner",
            match &self.owner {
                OwnerFilter::Anyone => None,
                OwnerFilter::Me => Some(Value::String("me".into())),
                OwnerFilter::Name(name) => Some(Value::String(format!("@{name}"))),
            },
        );
        set(
            "group",
            (self.group_by != defaults.group_by).then(|| {
                let (_, name) = GROUPS
                    .iter()
                    .find(|(group, _)| *group == self.group_by)
                    .expect("every grouping has a word");
                Value::String((*name).into())
            }),
        );
        set(
            "hide_old",
            (self.hide_old != defaults.hide_old).then_some(Value::Boolean(self.hide_old)),
        );
        set(
            "due",
            self.due.map(|due| {
                let (_, name) = BUCKETS
                    .iter()
                    .find(|(bucket, _)| *bucket == due)
                    .expect("every bucket has a word");
                Value::String((*name).into())
            }),
        );
        set(
            "priority",
            self.priority
                .map(|priority| Value::String(priority_word(priority))),
        );
        set(
            "layout",
            (self.layout == Layout::Board).then(|| Value::String("board".into())),
        );
    }
}

/// A path as a key or an entry of the file: UTF-8, with `/` between its
/// parts, none of them hidden.
fn encode(path: &RelPath) -> Result<String, String> {
    let text = path
        .as_path()
        .to_str()
        .ok_or("Task settings can only name UTF-8 paths")?;
    decode(text)
        .map(|_| text.to_owned())
        .ok_or_else(|| "Task settings cannot name this path".into())
}

fn decode(path: &str) -> Option<RelPath> {
    let portable = !path.is_empty()
        && !path.contains(['\\', '\0'])
        && !path.starts_with('/')
        && path.as_bytes().get(1) != Some(&b':')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'));
    portable.then(|| RelPath::new(path).ok()).flatten()
}

/// A section's or a group's path: one or two parts.
fn decode_folder(path: &str) -> Option<RelPath> {
    decode(path).filter(|_| path.split('/').count() <= 2)
}

fn scope_key(scope: Option<&RelPath>) -> Result<String, String> {
    scope.map_or_else(|| Ok(WORKSPACE.to_owned()), encode)
}

/// `path` with a leading `from` replaced by `to`.
fn remapped(path: &RelPath, from: &RelPath, to: &RelPath) -> RelPath {
    match path.as_path().strip_prefix(from.as_path()) {
        Ok(rest) if rest.as_os_str().is_empty() => to.clone(),
        Ok(rest) => RelPath::new(to.as_path().join(rest)).unwrap_or_else(|_| path.clone()),
        Err(_) => path.clone(),
    }
}

#[derive(Clone, Debug)]
pub struct TasksFile {
    root: PathBuf,
    table: Table,
    baseline: Option<Vec<u8>>,
    writable: bool,
    warning: Option<String>,
    /// Whether the table differs from what the file holds.
    dirty: bool,
}

impl TasksFile {
    /// What the workspace's file holds, or defaults with no file. Reading
    /// never writes.
    pub fn load(store: &FsStore) -> Self {
        let mut file = Self {
            root: store.root().to_path_buf(),
            table: Table::new(),
            baseline: None,
            writable: true,
            warning: None,
            dirty: false,
        };
        let read = match store.read(&file_path()) {
            Ok(read) => read,
            Err(error) if error.is_not_found() => return file,
            Err(error) => {
                file.disable(format!("could not read {TASKS_PATH}: {error}"));
                return file;
            }
        };
        let parsed = std::str::from_utf8(&read.bytes)
            .map_err(|_| format!("{TASKS_PATH} is not UTF-8"))
            .and_then(|text| {
                text.parse::<Table>()
                    .map_err(|error| format!("invalid TOML in {TASKS_PATH}: {}", error.message()))
            })
            .and_then(|table| match table.get("schema_version") {
                Some(version) if version.as_integer() != Some(SCHEMA_VERSION) => Err(format!(
                    "unsupported {TASKS_PATH} schema_version (expected 1)"
                )),
                _ => Ok(table),
            });
        file.baseline = Some(read.bytes);
        match parsed {
            Ok(table) => file.table = table,
            Err(reason) => file.disable(reason),
        }
        file
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn writable(&self) -> bool {
        self.writable
    }

    /// Whether a change waits to be saved.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    fn disable(&mut self, reason: String) {
        self.table = Table::new();
        self.writable = false;
        self.dirty = false;
        self.warning = Some(format!(
            "Task settings are read-only: {reason}. Using defaults; {TASKS_PATH} will not be overwritten."
        ));
    }

    fn check_writable(&self) -> Result<(), String> {
        if self.writable {
            Ok(())
        } else {
            Err(self
                .warning
                .clone()
                .unwrap_or_else(|| "Task settings are read-only".to_owned()))
        }
    }

    /// The sections and groups left out of every rollup, in the file's
    /// order, each once.
    pub fn excluded(&self) -> Vec<RelPath> {
        let mut folders: Vec<RelPath> = Vec::new();
        for folder in (self.table.get(EXCLUDE).and_then(Value::as_array))
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(decode_folder)
        {
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        folders
    }

    /// Whether `folder` itself is listed, not a folder above it.
    pub fn is_excluded(&self, folder: &RelPath) -> bool {
        self.excluded().contains(folder)
    }

    fn write_excluded(&mut self, folders: &[RelPath]) -> Result<(), String> {
        let entries = folders
            .iter()
            .map(|folder| encode(folder).map(Value::String))
            .collect::<Result<Vec<_>, _>>()?;
        if entries.is_empty() {
            self.table.remove(EXCLUDE);
        } else {
            self.table.insert(EXCLUDE.into(), Value::Array(entries));
        }
        self.dirty = true;
        Ok(())
    }

    /// List `folder`, a section or a group, or take it off the list.
    /// Returns whether the list changed.
    pub fn set_excluded(&mut self, folder: &RelPath, excluded: bool) -> Result<bool, String> {
        self.check_writable()?;
        if encode(folder)
            .ok()
            .and_then(|path| decode_folder(&path))
            .is_none()
        {
            return Err("Only a section or a group can be left out of task views".into());
        }
        let mut folders = self.excluded();
        let listed = folders.contains(folder);
        if listed == excluded {
            return Ok(false);
        }
        if excluded {
            folders.push(folder.clone());
        } else {
            folders.retain(|listed| listed != folder);
        }
        self.write_excluded(&folders)?;
        Ok(true)
    }

    fn views(&self) -> Option<&Table> {
        self.table.get(VIEWS).and_then(Value::as_table)
    }

    /// The filters remembered for the view of `scope` (the workspace for
    /// `None`), or the defaults.
    pub fn filters(&self, scope: Option<&RelPath>) -> TaskFilters {
        scope_key(scope)
            .ok()
            .and_then(|key| self.views()?.get(&key)?.as_table())
            .map_or_else(TaskFilters::default, TaskFilters::read)
    }

    /// Remember `filters` for the view of `scope`. Returns whether what
    /// the file holds changed.
    pub fn set_filters(
        &mut self,
        scope: Option<&RelPath>,
        filters: &TaskFilters,
    ) -> Result<bool, String> {
        self.check_writable()?;
        let key = scope_key(scope)?;
        let before = self.table.clone();
        let views = self
            .table
            .entry(VIEWS)
            .or_insert_with(|| Value::Table(Table::new()));
        if !views.is_table() {
            *views = Value::Table(Table::new());
        }
        let views = views.as_table_mut().expect("views was made a table");
        let view = views
            .entry(key.clone())
            .or_insert_with(|| Value::Table(Table::new()));
        if !view.is_table() {
            *view = Value::Table(Table::new());
        }
        let view = view.as_table_mut().expect("the view was made a table");
        filters.write(view);
        if view.is_empty() {
            views.remove(&key);
        }
        if views.is_empty() {
            self.table.remove(VIEWS);
        }
        let changed = self.table != before;
        self.dirty |= changed;
        Ok(changed)
    }

    /// Rewrite every path the file names through `transform`; `None`
    /// drops the entry.
    fn transform_paths(&mut self, transform: impl Fn(&RelPath) -> Option<RelPath>) {
        if !self.writable {
            return;
        }
        let before = self.table.clone();
        if self.table.contains_key(EXCLUDE) {
            let listed = self.excluded();
            let mut folders: Vec<RelPath> = Vec::new();
            for folder in listed.iter().filter_map(&transform) {
                // What no longer names a section or a group is dropped.
                let fits = encode(&folder).is_ok_and(|path| decode_folder(&path).is_some());
                if fits && !folders.contains(&folder) {
                    folders.push(folder);
                }
            }
            if folders != listed {
                let entries = folders
                    .iter()
                    .filter_map(|folder| encode(folder).ok())
                    .map(Value::String)
                    .collect::<Vec<_>>();
                if entries.is_empty() {
                    self.table.remove(EXCLUDE);
                } else {
                    self.table.insert(EXCLUDE.into(), Value::Array(entries));
                }
            }
        }
        if let Some(views) = self.table.get_mut(VIEWS).and_then(Value::as_table_mut) {
            let mut mapped = Table::new();
            let mut moved = Vec::new();
            for (key, view) in views.iter() {
                let Some(path) = decode(key) else {
                    // The workspace's view, and keys Wren does not read.
                    mapped.insert(key.clone(), view.clone());
                    continue;
                };
                match transform(&path).map(|path| encode(&path)) {
                    Some(Ok(new_key)) if new_key == *key => {
                        mapped.insert(new_key, view.clone());
                    }
                    Some(Ok(new_key)) => moved.push((new_key, view.clone())),
                    Some(Err(_)) | None => {}
                }
            }
            // A view left at the destination must not override the one
            // that moved there.
            mapped.extend(moved);
            *views = mapped;
            if views.is_empty() {
                self.table.remove(VIEWS);
            }
        }
        self.dirty |= self.table != before;
    }

    /// A section, a group or a note moved from `from` to `to`.
    pub fn remap_paths(&mut self, from: &RelPath, to: &RelPath) {
        self.transform_paths(|path| Some(remapped(path, from, to)));
    }

    /// Everything at or under `prefix` was deleted.
    pub fn remove_paths(&mut self, prefix: &RelPath) {
        self.transform_paths(|path| {
            (!path.as_path().starts_with(prefix.as_path())).then(|| path.clone())
        });
    }

    /// Whether the table holds anything worth a file.
    fn holds_settings(&self) -> bool {
        self.table.keys().any(|key| key != "schema_version")
    }

    /// Write what changed since the file was read or last saved, over
    /// exactly those bytes. With nothing changed, nothing is written, and
    /// a file that does not exist is created only once it has something to
    /// hold.
    pub fn save(&mut self, store: &mut FsStore) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        self.check_writable()?;
        if store.root() != self.root {
            return Err("Task settings belong to a different workspace".to_owned());
        }
        if self.baseline.is_none() && !self.holds_settings() {
            self.dirty = false;
            return Ok(());
        }
        self.table
            .entry("schema_version")
            .or_insert(Value::Integer(SCHEMA_VERSION));
        let bytes = toml::to_string_pretty(&self.table)
            .map_err(|error| format!("Could not serialize {TASKS_PATH}: {error}"))?
            .into_bytes();
        let directory = RelPath::new(".wren").expect("constant metadata directory is relative");
        let expected = match self.baseline.as_deref() {
            Some(bytes) => Expected::Bytes(bytes),
            None => Expected::Missing,
        };
        // Task settings are no draft: a sync that fails is retried, but
        // never holds back a flush or a workspace switch.
        let receipt = store.best_effort(|store| {
            store
                .ensure_dir(&directory)
                .map_err(|error| format!("Could not create .wren: {error}"))?;
            store
                .write(&file_path(), &bytes, expected)
                .map_err(|error| format!("Could not save {TASKS_PATH}: {error}"))
        })?;
        // Published: a durability failure must not make a retry conflict
        // with our own write.
        self.baseline = Some(receipt.file.bytes);
        self.dirty = false;
        match receipt.durability_error {
            Some(error) => Err(format!(
                "{TASKS_PATH} was published, but its durability could not be confirmed: {error}"
            )),
            None => Ok(()),
        }
    }
}

fn file_path() -> RelPath {
    RelPath::new(TASKS_PATH).expect("constant metadata path is relative")
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn path(text: &str) -> RelPath {
        RelPath::new(text).unwrap()
    }

    fn store() -> (TempDir, FsStore) {
        let directory = TempDir::new().unwrap();
        let store = FsStore::open(directory.path()).unwrap();
        (directory, store)
    }

    fn written(directory: &TempDir) -> Option<String> {
        std::fs::read_to_string(directory.path().join(TASKS_PATH)).ok()
    }

    fn write(directory: &TempDir, text: &str) {
        std::fs::create_dir_all(directory.path().join(".wren")).unwrap();
        std::fs::write(directory.path().join(TASKS_PATH), text).unwrap();
    }

    #[test]
    fn loading_and_saving_with_nothing_set_creates_no_file() {
        let (directory, mut store) = store();
        let mut file = TasksFile::load(&store);
        assert_eq!(file.warning(), None);
        assert!(file.excluded().is_empty());
        assert_eq!(file.filters(None), TaskFilters::default());
        file.save(&mut store).unwrap();
        // Defaults set, and an exclusion that was never there removed:
        // still nothing to hold.
        assert!(!file.set_filters(None, &TaskFilters::default()).unwrap());
        assert!(!file.set_excluded(&path("Work"), false).unwrap());
        file.remap_paths(&path("Work"), &path("Job"));
        file.remove_paths(&path("Work"));
        file.save(&mut store).unwrap();
        assert_eq!(written(&directory), None);
        assert!(!directory.path().join(".wren").exists());
    }

    #[test]
    fn exclusions_are_listed_once_and_written_in_order() {
        let (directory, mut store) = store();
        let mut file = TasksFile::load(&store);
        assert!(file.set_excluded(&path("Home/Shopping"), true).unwrap());
        assert!(file.set_excluded(&path("Archive"), true).unwrap());
        assert!(!file.set_excluded(&path("Archive"), true).unwrap());
        assert!(file.dirty());
        file.save(&mut store).unwrap();
        assert!(!file.dirty());
        assert_eq!(
            written(&directory).unwrap(),
            "exclude = [\n    \"Home/Shopping\",\n    \"Archive\",\n]\nschema_version = 1\n"
        );
        let mut again = TasksFile::load(&store);
        assert_eq!(again.excluded(), [path("Home/Shopping"), path("Archive")]);
        assert!(again.is_excluded(&path("Archive")));
        assert!(!again.is_excluded(&path("Home")));
        assert!(again.set_excluded(&path("Home/Shopping"), false).unwrap());
        assert!(again.set_excluded(&path("Archive"), false).unwrap());
        again.save(&mut store).unwrap();
        // The file stays, now holding nothing.
        assert_eq!(written(&directory).unwrap(), "schema_version = 1\n");
        // A note, and a path outside the workspace, name no folder.
        for refused in ["Work/Notes/Plan.md", ".wren", "Work/.hidden"] {
            assert!(
                again.set_excluded(&path(refused), true).is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn every_due_bucket_is_written_and_read_back() {
        let mut words = Vec::new();
        for bucket in DueBucket::ALL {
            let filters = TaskFilters {
                due: Some(bucket),
                ..TaskFilters::default()
            };
            let mut table = Table::new();
            filters.write(&mut table);
            words.push(table["due"].as_str().unwrap().to_owned());
            assert_eq!(TaskFilters::read(&table), filters, "{bucket:?}");
        }
        assert_eq!(
            words,
            ["overdue", "earlier", "today", "week", "next", "later", "none"]
        );
    }

    #[test]
    fn filters_keep_only_what_differs_from_the_defaults() {
        let (directory, mut store) = store();
        let mut file = TasksFile::load(&store);
        let filters = TaskFilters {
            statuses: vec![StatusType::Doing, StatusType::Open],
            owner: OwnerFilter::Name("sam".into()),
            group_by: GroupBy::Due,
            hide_old: false,
            due: Some(DueBucket::ThisWeek),
            priority: Some(PriorityFilter::Level(Priority::High)),
            layout: Layout::List,
        };
        let scope = path("Work/Meetings");
        assert!(file.set_filters(Some(&scope), &filters).unwrap());
        assert!(file
            .set_filters(
                None,
                &TaskFilters {
                    owner: OwnerFilter::Me,
                    priority: Some(PriorityFilter::Unset),
                    ..TaskFilters::default()
                }
            )
            .unwrap());
        file.save(&mut store).unwrap();
        assert_eq!(
            written(&directory).unwrap(),
            "schema_version = 1\n\n[views.\".\"]\nowner = \"me\"\npriority = \"none\"\n\n\
             [views.\"Work/Meetings\"]\ndue = \"week\"\ngroup = \"due\"\nhide_old = false\n\
             owner = \"@sam\"\npriority = \"high\"\nstatuses = [\n    \"open\",\n    \"doing\",\n]\n"
        );
        let mut again = TasksFile::load(&store);
        // The chips come back in the order they stand.
        assert_eq!(
            again.filters(Some(&scope)),
            TaskFilters {
                statuses: vec![StatusType::Open, StatusType::Doing],
                ..filters
            }
        );
        assert_eq!(again.filters(None).owner, OwnerFilter::Me);
        assert_eq!(again.filters(Some(&path("Other"))), TaskFilters::default());
        // Back to the defaults: the tables go.
        assert!(again
            .set_filters(Some(&scope), &TaskFilters::default())
            .unwrap());
        assert!(again.set_filters(None, &TaskFilters::default()).unwrap());
        again.save(&mut store).unwrap();
        assert_eq!(written(&directory).unwrap(), "schema_version = 1\n");
    }

    #[test]
    fn a_file_is_read_leniently_and_unknown_keys_survive() {
        let (directory, mut store) = store();
        write(
            &directory,
            "exclude = [\"Home\", 7, \"../out\", \"Home\", \"A/B/C.md\", \"Work/Old\"]\n\
             later = { kanban = [1, 2] }\n\n\
             [views.\"Work\"]\nstatuses = [\"done\", \"paused\", \"done\"]\nowner = \"sam\"\n\
             group = \"colour\"\nhide_old = \"no\"\ndue = \"someday\"\npriority = \"HIGHEST\"\n\
             columns = 3\n",
        );
        let mut file = TasksFile::load(&store);
        assert_eq!(file.warning(), None);
        assert_eq!(file.excluded(), [path("Home"), path("Work/Old")]);
        assert_eq!(
            file.filters(Some(&path("Work"))),
            TaskFilters {
                statuses: vec![StatusType::Done],
                priority: Some(PriorityFilter::Level(Priority::Highest)),
                ..TaskFilters::default()
            }
        );
        file.set_filters(
            Some(&path("Work")),
            &TaskFilters {
                group_by: GroupBy::Owner,
                ..TaskFilters::default()
            },
        )
        .unwrap();
        file.save(&mut store).unwrap();
        let saved = written(&directory).unwrap();
        assert!(saved.contains("[later]\nkanban = ["), "{saved}");
        assert!(
            saved.contains("columns = 3\ngroup = \"owner\"\n"),
            "{saved}"
        );
        assert!(!saved.contains("statuses"), "{saved}");
    }

    /// The layout is written only as the board, beside the scope's
    /// filters; the list removes it, and an all-list file names none.
    #[test]
    fn the_layout_is_written_only_for_the_board() {
        let (directory, mut store) = store();
        let mut file = TasksFile::load(&store);
        let scope = path("Work");
        let board = TaskFilters {
            layout: Layout::Board,
            ..TaskFilters::default()
        };
        // The list is the default: nothing to hold.
        assert!(!file
            .set_filters(Some(&scope), &TaskFilters::default())
            .unwrap());
        file.save(&mut store).unwrap();
        assert_eq!(written(&directory), None);
        assert!(file.set_filters(Some(&scope), &board).unwrap());
        file.save(&mut store).unwrap();
        assert_eq!(
            written(&directory).unwrap(),
            "schema_version = 1\n\n[views.Work]\nlayout = \"board\"\n"
        );
        let mut again = TasksFile::load(&store);
        assert_eq!(again.filters(Some(&scope)), board);
        // Each scope has its own.
        assert_eq!(again.filters(None).layout, Layout::List);
        assert_eq!(again.filters(Some(&path("Home"))).layout, Layout::List);
        // Beside a filter, and back to the list: only the filter stays.
        let narrowed = TaskFilters {
            owner: OwnerFilter::Me,
            ..board.clone()
        };
        assert!(again.set_filters(Some(&scope), &narrowed).unwrap());
        again.save(&mut store).unwrap();
        assert_eq!(
            written(&directory).unwrap(),
            "schema_version = 1\n\n[views.Work]\nlayout = \"board\"\nowner = \"me\"\n"
        );
        assert!(again
            .set_filters(
                Some(&scope),
                &TaskFilters {
                    layout: Layout::List,
                    ..narrowed
                }
            )
            .unwrap());
        again.save(&mut store).unwrap();
        assert_eq!(
            written(&directory).unwrap(),
            "schema_version = 1\n\n[views.Work]\nowner = \"me\"\n"
        );
        assert_eq!(Layout::List.other(), Layout::Board);
        assert_eq!(Layout::Board.other(), Layout::List);
    }

    /// A layout Wren does not know is the list, as an unknown value of a
    /// filter is its default; the scope's next save drops it as it drops
    /// those.
    #[test]
    fn an_unknown_layout_is_the_list() {
        for unknown in ["\"grid\"", "\"Board\"", "3", "[\"board\"]"] {
            let (directory, mut store) = store();
            write(
                &directory,
                &format!("[views.Work]\nlayout = {unknown}\ngroup = \"due\"\n"),
            );
            let mut file = TasksFile::load(&store);
            assert_eq!(file.warning(), None);
            let filters = file.filters(Some(&path("Work")));
            assert_eq!(filters.layout, Layout::List, "{unknown}");
            assert_eq!(filters.group_by, GroupBy::Due, "{unknown}");
            file.set_filters(
                Some(&path("Work")),
                &TaskFilters {
                    group_by: GroupBy::Owner,
                    ..filters
                },
            )
            .unwrap();
            file.save(&mut store).unwrap();
            assert_eq!(
                written(&directory).unwrap(),
                "schema_version = 1\n\n[views.Work]\ngroup = \"owner\"\n",
                "{unknown}"
            );
        }
    }

    #[test]
    fn a_bad_file_is_left_alone_with_a_warning() {
        for bad in ["exclude = [", "schema_version = 2\nexclude = [\"Home\"]\n"] {
            let (directory, mut store) = store();
            write(&directory, bad);
            let mut file = TasksFile::load(&store);
            assert!(file.warning().unwrap().contains("read-only"), "{bad}");
            assert!(!file.writable());
            assert!(file.excluded().is_empty());
            assert!(file.set_excluded(&path("Home"), true).is_err());
            assert!(file.set_filters(None, &TaskFilters::default()).is_err());
            file.remap_paths(&path("Home"), &path("House"));
            file.save(&mut store).unwrap();
            assert_eq!(written(&directory).unwrap(), bad);
        }
    }

    #[test]
    fn a_rename_remaps_and_a_delete_drops_what_names_the_path() {
        let (directory, mut store) = store();
        let mut file = TasksFile::load(&store);
        for folder in ["Work", "Work/Old", "Home/Lists"] {
            file.set_excluded(&path(folder), true).unwrap();
        }
        let by_owner = TaskFilters {
            group_by: GroupBy::Owner,
            ..TaskFilters::default()
        };
        let by_due = TaskFilters {
            group_by: GroupBy::Due,
            ..TaskFilters::default()
        };
        for scope in ["Work", "Work/Old/Plan.md", "Home"] {
            file.set_filters(Some(&path(scope)), &by_owner).unwrap();
        }
        file.set_filters(Some(&path("Job")), &by_due).unwrap();
        file.set_filters(None, &by_due).unwrap();
        file.save(&mut store).unwrap();
        // A section renamed: itself, the group under it and the note
        // under that follow, and the view that moved replaces the one
        // left at its new name.
        file.remap_paths(&path("Work"), &path("Job"));
        assert_eq!(
            file.excluded(),
            [path("Job"), path("Job/Old"), path("Home/Lists")]
        );
        assert_eq!(file.filters(Some(&path("Job"))), by_owner);
        assert_eq!(file.filters(Some(&path("Job/Old/Plan.md"))), by_owner);
        assert_eq!(file.filters(Some(&path("Work"))), TaskFilters::default());
        assert_eq!(file.filters(None), by_due);
        assert!(file.dirty());
        // A group deleted: what names it or a note in it goes.
        file.remove_paths(&path("Job/Old"));
        assert_eq!(file.excluded(), [path("Job"), path("Home/Lists")]);
        assert_eq!(
            file.filters(Some(&path("Job/Old/Plan.md"))),
            TaskFilters::default()
        );
        assert_eq!(file.filters(Some(&path("Job"))), by_owner);
        file.remove_paths(&path("Home"));
        assert_eq!(file.excluded(), [path("Job")]);
        assert_eq!(file.filters(Some(&path("Home"))), TaskFilters::default());
        file.save(&mut store).unwrap();
        let again = TasksFile::load(&store);
        assert_eq!(again.excluded(), [path("Job")]);
        assert_eq!(again.filters(None), by_due);
        drop(directory);
    }

    #[test]
    fn a_save_goes_over_the_bytes_read_only() {
        let (directory, mut store) = store();
        write(&directory, "exclude = [\"Home\"]\n");
        let mut file = TasksFile::load(&store);
        write(&directory, "exclude = [\"Elsewhere\"]\n");
        file.set_excluded(&path("Work"), true).unwrap();
        assert!(file
            .save(&mut store)
            .unwrap_err()
            .contains("Could not save"));
        assert_eq!(written(&directory).unwrap(), "exclude = [\"Elsewhere\"]\n");
        assert!(file.dirty());
    }
}
