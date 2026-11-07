//! Optional global workspace registrations, not workspace contents.
//!
//! `schema_version = 1`, `[[workspaces]]` (`name`, `path`), and an optional
//! `active` path are the known fields. An absent version is accepted. Unknown
//! TOML values survive updates; comments and formatting do not. Loading never
//! creates a directory or acquires a workspace writer. Mutations briefly lock
//! the config directory through `FsStore`, independently of note editing.

use std::fs::File;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use directories::ProjectDirs;
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RawMode, Stat};
use rustix::io::Errno;
use toml::{Table, Value};

use crate::fs_store::{Expected, FsStore, RelPath, StoreError};

const CONFIG_FILE: &str = "config.toml";
const SCHEMA_VERSION: i64 = 1;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_WORKSPACES: usize = 4096;
const MAX_NAME_BYTES: usize = 256;
const MAX_DIRECTORY_DEPTH: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRegistration {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct Config {
    directory: Option<PathBuf>,
    anchor: Option<DirectoryAnchor>,
    registry: Registry,
    baseline: Option<Vec<u8>>,
    writable: bool,
    warning: Option<String>,
    // FsStore retains failed directory-sync descriptors. Keep its lock only
    // while durability is pending, so another writer cannot outrun a retry.
    pending_store: Option<FsStore>,
    #[cfg(test)]
    fault: Option<crate::fs_store::Fault>,
}

impl Config {
    /// Discover the platform location without requiring global config to be
    /// available for opening or editing a workspace.
    pub fn load() -> Self {
        match ProjectDirs::from("", "", "Wren") {
            Some(project) => Self::load_from(project.config_dir()),
            None => {
                let mut config = Self::defaults(None);
                config.disable("the platform config directory is unavailable");
                config
            }
        }
    }

    /// Load from an explicit directory, without modifying it. Tests and private
    /// frontend harnesses can use this instead of the actual user config.
    pub fn load_from(config_directory: &Path) -> Self {
        let mut config = Self::defaults(Some(config_directory.to_path_buf()));
        let (directory, anchor) = match DirectoryAnchor::locate(config_directory) {
            Ok(location) => location,
            Err(reason) => {
                config.disable(&reason);
                return config;
            }
        };
        config.directory = Some(directory.clone());
        config.anchor = Some(anchor);
        let directory_file = match open_absolute_directory(&directory) {
            Ok(file) => file,
            Err(Errno::NOENT) => {
                if config
                    .anchor
                    .as_ref()
                    .is_some_and(|anchor| !has_write_bits(&anchor.file))
                {
                    config.read_only("the config parent directory is not writable");
                }
                return config;
            }
            Err(_) => {
                config.disable("the config directory is unreadable or contains a symlink");
                return config;
            }
        };
        let file = match read_config(&directory_file) {
            Ok(file) => file,
            Err(ReadError::Missing) => {
                if !has_write_bits(&directory_file) {
                    config.read_only("the config directory is not writable");
                }
                return config;
            }
            Err(ReadError::Invalid(reason)) => {
                config.disable(reason);
                return config;
            }
        };
        config.baseline = Some(file.bytes);
        let parsed = std::str::from_utf8(config.baseline.as_deref().unwrap_or_default())
            .map_err(|_| "config.toml is not UTF-8".to_owned())
            .and_then(|text| {
                text.parse::<Table>()
                    .map_err(|_| "config.toml contains invalid TOML".to_owned())
            })
            .and_then(Registry::parse);
        match parsed {
            Ok(registry) => {
                config.registry = registry;
                config.refresh_warning(None);
                if file.mode & 0o222 == 0 || !has_write_bits(&directory_file) {
                    config.read_only("config.toml or its directory is not writable");
                }
            }
            Err(reason) => config.disable(&reason),
        }
        config
    }

    pub fn workspaces(&self) -> &[WorkspaceRegistration] {
        &self.registry.workspaces
    }

    /// May name an unavailable registration. Loading does not create or open a
    /// missing workspace; the frontend must handle a failed session open.
    pub fn active(&self) -> Option<&Path> {
        self.registry.active.as_deref()
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// False for invalid config and known permission restrictions. Actual I/O
    /// and exact-baseline checks still happen on every attempted mutation.
    pub fn writable(&self) -> bool {
        self.writable
    }

    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    /// Register an existing directory without changing last-active. An already
    /// registered canonical path is idempotent and keeps its original name.
    /// Call only after the frontend's session switch has succeeded.
    pub fn add(&mut self, path: &Path, name: Option<&str>) -> Result<(), String> {
        self.require_writable()?;
        let path = workspace_directory(path)?;
        let mut table = self.registry.table.clone();
        if !self
            .registry
            .workspaces
            .iter()
            .any(|entry| entry.path == path)
        {
            let name = registration_name(&path, name)?;
            let mut entry = Table::new();
            entry.insert("name".into(), Value::String(name));
            entry.insert("path".into(), path_value(&path)?);
            table
                .entry("workspaces")
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| "workspaces must be an array".to_owned())?
                .push(Value::Table(entry));
        }
        self.persist(Registry::parse(table)?)
    }

    /// Remove only registration metadata, including every duplicate spelling.
    /// Unavailable, replaced, and symlinked stale registrations remain removable.
    /// The frontend may retain its open session until a safe subsequent switch.
    pub fn remove(&mut self, path: &Path) -> Result<(), String> {
        self.require_writable()?;
        let path = self.registered_path(path)?;
        let mut table = self.registry.table.clone();
        if let Some(entries) = table.get_mut("workspaces").and_then(Value::as_array_mut) {
            entries.retain(|entry| {
                entry
                    .as_table()
                    .and_then(|entry| entry.get("path"))
                    .and_then(Value::as_str)
                    .is_none_or(|candidate| Path::new(candidate) != path)
            });
        }
        if self.registry.active.as_deref() == Some(path.as_path()) {
            table.remove("active");
        }
        self.persist(Registry::parse(table)?)
    }

    /// Validate a stored root before opening it, without writing or acquiring a
    /// config lock. Unlike a new folder choice, its ancestors must not redirect
    /// through symlinks. Optional config writability does not block note access.
    pub fn validate_registration(&self, path: &Path) -> Result<PathBuf, String> {
        let path = self.registered_path(path)?;
        if !registered_directory_available(&path) {
            return Err(
                "The registered workspace is unavailable or contains a replacement symlink".into(),
            );
        }
        Ok(path)
    }

    /// Persist last-active only for a registered, available real directory.
    /// This is not a session switch or a save barrier: the frontend must invoke
    /// it only after its actual session switch successfully commits.
    pub fn set_active(&mut self, path: &Path) -> Result<(), String> {
        self.require_writable()?;
        let path = self.validate_registration(path)?;
        let mut table = self.registry.table.clone();
        table.insert("active".into(), path_value(&path)?);
        self.persist(Registry::parse(table)?)
    }

    /// Retry a previously published update's directory fsync without repeating
    /// the registration mutation. This does not rewrite config.toml.
    pub fn retry_durability(&mut self) -> Result<(), String> {
        let Some(store) = self.pending_store.as_mut() else {
            return Ok(());
        };
        let result = store.retry_durability().map_err(|error| {
            format!(
                "config.toml remains published, but directory durability is still unconfirmed: {}",
                config_store_error(error)
            )
        });
        if result.is_ok() {
            self.pending_store = None;
        }
        self.refresh_warning(result.as_ref().err().cloned());
        result
    }

    fn defaults(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            anchor: None,
            registry: Registry::default(),
            baseline: None,
            writable: true,
            warning: None,
            pending_store: None,
            #[cfg(test)]
            fault: None,
        }
    }

    fn require_writable(&self) -> Result<(), String> {
        if self.writable {
            Ok(())
        } else {
            Err(self
                .warning
                .clone()
                .unwrap_or_else(|| "Workspace configuration is read-only".into()))
        }
    }

    fn disable(&mut self, reason: &str) {
        self.registry = Registry::default();
        self.writable = false;
        self.warning = Some(format!(
            "Workspace configuration is read-only: {reason}. Using defaults; config.toml will not be overwritten."
        ));
    }

    fn read_only(&mut self, reason: &str) {
        self.writable = false;
        self.refresh_warning(Some(format!(
            "Workspace configuration is read-only: {reason}. config.toml will not be overwritten."
        )));
    }

    fn refresh_warning(&mut self, failure: Option<String>) {
        let unavailable = self.registry.unavailable.then_some(
            "One or more registered workspaces are unavailable; their registrations have been kept and can be removed.",
        );
        self.warning = match (failure, unavailable) {
            (Some(failure), Some(unavailable)) => Some(format!("{failure} {unavailable}")),
            (Some(failure), None) => Some(failure),
            (None, Some(unavailable)) => Some(unavailable.into()),
            (None, None) => None,
        };
    }

    fn registered_path(&self, path: &Path) -> Result<PathBuf, String> {
        if let Ok(direct) = stored_path(path) {
            if self
                .registry
                .workspaces
                .iter()
                .any(|entry| entry.path == direct)
            {
                return Ok(direct);
            }
        }
        let canonical = workspace_directory(path)?;
        self.registry
            .workspaces
            .iter()
            .any(|entry| entry.path == canonical)
            .then_some(canonical)
            .ok_or_else(|| "The workspace is not registered".into())
    }

    fn open_store(&mut self) -> Result<FsStore, String> {
        let directory = self
            .directory
            .as_ref()
            .ok_or_else(|| "The platform config directory is unavailable".to_owned())?;
        let anchor = self
            .anchor
            .as_ref()
            .ok_or_else(|| "The config directory could not be validated".to_owned())?;
        let file = anchor.ensure(directory)?;
        self.anchor = Some(DirectoryAnchor {
            path: directory.clone(),
            file,
        });
        let store = open_registry_store(directory).map_err(config_store_error)?;
        if store.root() != directory {
            return Err(
                "The config directory was replaced; reload configuration before updating it".into(),
            );
        }
        self.anchor.as_ref().expect("set above").validate()?;
        Ok(store)
    }

    fn check_metadata_writable(&self) -> Result<(), String> {
        let anchor = self
            .anchor
            .as_ref()
            .expect("opened store has a directory anchor");
        anchor.validate()?;
        if !has_write_bits(&anchor.file) {
            return Err("The config directory is no longer writable; the registration update was not published".into());
        }
        match read_config(&anchor.file) {
            Ok(file) if file.mode & 0o222 == 0 => {
                Err("config.toml is read-only; the registration update was not published".into())
            }
            Ok(_) | Err(ReadError::Missing) => Ok(()),
            Err(ReadError::Invalid(reason)) => Err(format!(
                "Could not update config.toml: {reason}. The registration update was not published."
            )),
        }
    }

    fn persist(&mut self, mut registry: Registry) -> Result<(), String> {
        registry
            .table
            .insert("schema_version".into(), Value::Integer(SCHEMA_VERSION));
        let bytes = toml::to_string_pretty(&registry.table)
            .map_err(|_| "Could not serialize config.toml".to_owned())?
            .into_bytes();
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err("config.toml would exceed the supported 1 MiB size limit".into());
        }
        let result = (|| {
            let mut store = match self.pending_store.take() {
                Some(store) => store,
                None => self.open_store()?,
            };
            if let Err(error) = store.retry_durability() {
                self.pending_store = Some(store);
                return Err(format!(
                    "The previous config.toml update remains published, but its durability is unconfirmed: {}. The new registration update was not published.",
                    config_store_error(error)
                ));
            }
            self.check_metadata_writable()?;
            #[cfg(test)]
            if let Some(fault) = self.fault.take() {
                store.fail_next(fault);
            }
            let expected = match self.baseline.as_deref() {
                Some(bytes) => Expected::Bytes(bytes),
                None => Expected::Missing,
            };
            let path = RelPath::new(CONFIG_FILE).expect("constant config path is relative");
            let receipt = store
                .write(&path, &bytes, expected)
                .map_err(config_store_error)?;
            // Only a receipt proves publication. An error before this point
            // must not change what the UI believes was registered or selected.
            self.baseline = Some(receipt.file.bytes);
            self.registry = registry;
            if receipt.durability_error.is_some() {
                self.pending_store = Some(store);
                Err("config.toml was published, but directory durability could not be confirmed. The registration changes are committed; retry to confirm durability.".into())
            } else {
                Ok(())
            }
        })();
        self.refresh_warning(result.as_ref().err().cloned());
        result
    }
}

#[derive(Debug, Default)]
struct Registry {
    table: Table,
    workspaces: Vec<WorkspaceRegistration>,
    active: Option<PathBuf>,
    unavailable: bool,
}

impl Registry {
    fn parse(mut table: Table) -> Result<Self, String> {
        if table
            .get("schema_version")
            .is_some_and(|version| version.as_integer() != Some(SCHEMA_VERSION))
        {
            return Err("Unsupported config.toml schema_version (expected 1)".into());
        }
        let mut workspaces = Vec::<WorkspaceRegistration>::new();
        let mut unavailable = false;
        if let Some(value) = table.get_mut("workspaces") {
            let entries = value
                .as_array_mut()
                .ok_or_else(|| "workspaces must be an array of registration tables".to_owned())?;
            if entries.len() > MAX_WORKSPACES {
                return Err(
                    "config.toml exceeds the supported workspace registration limit".into(),
                );
            }
            for value in entries {
                let entry = value
                    .as_table_mut()
                    .ok_or_else(|| "Every workspace registration must be a table".to_owned())?;
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Workspace names must be strings".to_owned())?
                    .trim()
                    .to_owned();
                validate_name(&name)?;
                let path = entry
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Workspace paths must be absolute path strings".to_owned())?;
                let path = stored_path(Path::new(path))?;
                if !registered_directory_available(&path) {
                    unavailable = true;
                }
                entry.insert("name".into(), Value::String(name.clone()));
                entry.insert("path".into(), path_value(&path)?);
                // Retain duplicate source tables so unknown per-registration
                // fields cannot be lost, but expose only one canonical entry.
                if !workspaces.iter().any(|entry| entry.path == path) {
                    workspaces.push(WorkspaceRegistration { name, path });
                }
            }
        }
        let active = table
            .get("active")
            .map(|value| -> Result<PathBuf, String> {
                let path = value
                    .as_str()
                    .ok_or_else(|| "active must be an absolute workspace path string".to_owned())?;
                let path = stored_path(Path::new(path))?;
                if !workspaces.iter().any(|entry| entry.path == path) {
                    return Err("active must refer to a registered workspace".into());
                }
                Ok(path)
            })
            .transpose()?;
        if let Some(active) = &active {
            table.insert("active".into(), path_value(active)?);
        }
        Ok(Self {
            table,
            workspaces,
            active,
            unavailable,
        })
    }
}

fn path_value(path: &Path) -> Result<Value, String> {
    path.to_str()
        .map(|path| Value::String(path.into()))
        .ok_or_else(|| "Workspace paths must be UTF-8 to persist in config.toml".into())
}

fn stored_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path.as_os_str().as_bytes().contains(&0)
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(
            "Workspace registration paths must be absolute and contain no parent traversal".into(),
        );
    }
    Ok(path.components().collect())
}

fn registered_directory_available(path: &Path) -> bool {
    // A saved registration already names a physical directory. Do not resolve
    // a newly introduced ancestor symlink into a different workspace at load.
    let Ok(file) = open_absolute_directory(path) else {
        return false;
    };
    let (Ok(opened), Ok(named)) = (file.metadata(), std::fs::symlink_metadata(path)) else {
        return false;
    };
    named.is_dir() && opened.dev() == named.dev() && opened.ino() == named.ino()
}

fn workspace_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute workspace directory path".into());
    }
    // Strip trailing separators/dots before lstat, which otherwise follows a
    // symlink spelled `alias/`. Ancestor platform aliases are resolved once,
    // matching FsStore's selected-root convention; only the physical path is saved.
    let selected_path: PathBuf = path.components().collect();
    let selected = std::fs::symlink_metadata(&selected_path)
        .map_err(|_| "The selected workspace directory is unavailable".to_owned())?;
    if !selected.is_dir() || selected.file_type().is_symlink() {
        return Err("A workspace must be an existing directory, not a symlink or file".into());
    }
    let canonical = std::fs::canonicalize(&selected_path)
        .map_err(|_| "The selected workspace directory could not be resolved".to_owned())?;
    let file = open_absolute_directory(&canonical)
        .map_err(|_| "The selected workspace directory could not be safely opened".to_owned())?;
    let opened = file
        .metadata()
        .map_err(|_| "The selected workspace directory could not be validated".to_owned())?;
    if selected.dev() != opened.dev() || selected.ino() != opened.ino() {
        return Err("The selected workspace directory changed while it was being opened".into());
    }
    path_value(&canonical)?;
    Ok(canonical)
}

fn registration_name(path: &Path, name: Option<&str>) -> Result<String, String> {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        validate_name(name)?;
        return Ok(name.into());
    }
    let basename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Workspace");
    let name: String = basename
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = if name.is_empty() { "Workspace" } else { &name };
    validate_name(name)?;
    Ok(name.into())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        Err("Workspace names must be nonempty, at most 256 UTF-8 bytes, and contain no control characters".into())
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct DirectoryAnchor {
    path: PathBuf,
    file: File,
}

impl DirectoryAnchor {
    fn locate(path: &Path) -> Result<(PathBuf, Self), String> {
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(
                "The config directory must be an absolute path without parent traversal".into(),
            );
        }
        let mut existing: PathBuf = path.components().collect();
        let mut missing = Vec::new();
        loop {
            match std::fs::symlink_metadata(&existing) {
                Ok(metadata) => {
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err("The config directory or its nearest existing parent is not a real directory".into());
                    }
                    let canonical = std::fs::canonicalize(&existing)
                        .map_err(|_| "The config directory could not be resolved".to_owned())?;
                    let file = open_absolute_directory(&canonical).map_err(|_| {
                        "The config directory could not be safely opened".to_owned()
                    })?;
                    let opened = file
                        .metadata()
                        .map_err(|_| "The config directory could not be validated".to_owned())?;
                    if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
                        return Err("The config directory changed while it was being opened".into());
                    }
                    let mut directory = canonical.clone();
                    directory.extend(missing.into_iter().rev());
                    return Ok((
                        directory,
                        Self {
                            path: canonical,
                            file,
                        },
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if missing.len() == MAX_DIRECTORY_DEPTH {
                        return Err("The missing config directory hierarchy is too deep".into());
                    }
                    let name = existing
                        .file_name()
                        .ok_or_else(|| "The config directory has no existing parent".to_owned())?
                        .to_os_string();
                    missing.push(name);
                    existing.pop();
                }
                Err(_) => return Err("The config directory is unreadable".into()),
            }
        }
    }

    fn validate(&self) -> Result<(), String> {
        let current = open_absolute_directory(&self.path).map_err(|_| {
            "The config directory is missing, unreadable, or was replaced".to_owned()
        })?;
        let current = fs::fstat(&current)
            .map_err(|_| "Could not validate the config directory".to_owned())?;
        let original = fs::fstat(&self.file)
            .map_err(|_| "Could not validate the config directory".to_owned())?;
        if !same_object(&original, &current) {
            return Err(
                "The config directory was replaced; reload configuration before updating it".into(),
            );
        }
        Ok(())
    }

    fn ensure(&self, path: &Path) -> Result<File, String> {
        self.validate()?;
        let relative = path.strip_prefix(&self.path).map_err(|_| {
            "The config directory does not belong to its validated parent".to_owned()
        })?;
        let mut parent = self
            .file
            .try_clone()
            .map_err(|_| "Could not open the config parent directory".to_owned())?;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err("The config directory contains an unsupported component".into());
            };
            match fs::mkdirat(&parent, name, Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(Errno::EXIST) => (),
                Err(_) => return Err("Could not create a private config directory".into()),
            }
            let child: File = fs::openat(&parent, name, directory_flags(), Mode::empty())
                .map_err(|_| "The config directory is unreadable or contains a symlink".to_owned())?
                .into();
            // Sync even an existing child: a previous prepublication failure
            // may have left a newly created, not-yet-durable directory here.
            fs::fsync(&child)
                .and_then(|()| fs::fsync(&parent))
                .map_err(|_| "Could not confirm config directory creation durability".to_owned())?;
            parent = child;
        }
        self.validate()?;
        let named = open_absolute_directory(path)
            .map_err(|_| "The config directory was replaced during creation".to_owned())?;
        let named_stat =
            fs::fstat(&named).map_err(|_| "Could not validate the config directory".to_owned())?;
        let opened_stat =
            fs::fstat(&parent).map_err(|_| "Could not validate the config directory".to_owned())?;
        if !same_object(&named_stat, &opened_stat) {
            return Err("The config directory was replaced during creation".into());
        }
        Ok(parent)
    }
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

fn open_absolute_directory(path: &Path) -> Result<File, Errno> {
    if !path.is_absolute() {
        return Err(Errno::INVAL);
    }
    let mut directory: File = fs::open("/", directory_flags(), Mode::empty())?.into();
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                directory = fs::openat(&directory, name, directory_flags(), Mode::empty())?.into();
            }
            _ => return Err(Errno::INVAL),
        }
    }
    Ok(directory)
}

fn has_write_bits(file: &File) -> bool {
    file.metadata()
        .is_ok_and(|metadata| metadata.permissions().mode() & 0o222 != 0)
}

struct ConfigFile {
    bytes: Vec<u8>,
    mode: RawMode,
}

enum ReadError {
    Missing,
    Invalid(&'static str),
}

fn read_config(directory: &File) -> Result<ConfigFile, ReadError> {
    let mut file: File = fs::openat(
        directory,
        CONFIG_FILE,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| {
        if error == Errno::NOENT {
            ReadError::Missing
        } else {
            ReadError::Invalid("config.toml is unreadable or is a symlink")
        }
    })?
    .into();
    let before =
        fs::fstat(&file).map_err(|_| ReadError::Invalid("Could not inspect config.toml"))?;
    if FileType::from_raw_mode(before.st_mode) != FileType::RegularFile {
        return Err(ReadError::Invalid("config.toml is not a regular file"));
    }
    if before.st_size > MAX_CONFIG_BYTES as i64 {
        return Err(ReadError::Invalid(
            "config.toml exceeds the supported 1 MiB size limit",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadError::Invalid("config.toml could not be read"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ReadError::Invalid(
            "config.toml exceeds the supported 1 MiB size limit",
        ));
    }
    let after =
        fs::fstat(&file).map_err(|_| ReadError::Invalid("Could not recheck config.toml"))?;
    let named = fs::statat(directory, CONFIG_FILE, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| ReadError::Invalid("config.toml changed while it was being read"))?;
    if !same_version(&before, &after) || !same_version(&after, &named) {
        return Err(ReadError::Invalid(
            "config.toml changed while it was being read",
        ));
    }
    Ok(ConfigFile {
        bytes,
        mode: after.st_mode & 0o777,
    })
}

fn same_object(left: &Stat, right: &Stat) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && FileType::from_raw_mode(left.st_mode) == FileType::from_raw_mode(right.st_mode)
}

fn same_version(left: &Stat, right: &Stat) -> bool {
    same_object(left, right)
        && left.st_mode == right.st_mode
        && left.st_size == right.st_size
        && left.st_mtime == right.st_mtime
        && left.st_mtime_nsec == right.st_mtime_nsec
        && left.st_ctime == right.st_ctime
        && left.st_ctime_nsec == right.st_ctime_nsec
}

/// Open the registry's store, retrying briefly while its lock is held.
///
/// Registry updates are short, so a competing writer releases quickly. A
/// concurrent `fork` (Reveal, opening settings, or another thread's child
/// process) also briefly duplicates the lock descriptor until `exec`.
fn open_registry_store(directory: &Path) -> Result<FsStore, StoreError> {
    let mut attempts = 0;
    loop {
        match FsStore::open(directory) {
            Err(StoreError::Locked) if attempts < 20 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

fn config_store_error(error: StoreError) -> String {
    // Store errors may contain an absolute root or user-supplied path. Config
    // warnings identify the operation, never include paths or parsed values.
    match error {
        StoreError::Locked => "Workspace configuration is busy in another process; retry the registration update".into(),
        StoreError::Conflict(_) => "config.toml changed since it was loaded; reload configuration before updating registrations".into(),
        StoreError::UnsafePath(_) | StoreError::InvalidPath(_) => "The config directory or file was replaced or contains a symlink; reload configuration before updating it".into(),
        StoreError::Io(error) => format!("Could not update config.toml: {}", error.kind()),
        StoreError::Injected(fault) => format!("Could not update config.toml: injected storage failure ({fault:?})"),
        StoreError::InvalidName(_) => "Could not update config.toml: invalid metadata filename".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs::{self, DirBuilder, FileTimes};
    use std::os::unix::fs::{symlink, DirBuilderExt};
    use std::process::Command;

    use tempfile::TempDir;

    use crate::fs_store::Fault;

    use super::*;

    struct Fixture {
        _temporary: TempDir,
        root: PathBuf,
        directory: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = TempDir::new().unwrap();
            let root = fs::canonicalize(temporary.path()).unwrap();
            let directory = root.join("private/config/wren");
            Self {
                _temporary: temporary,
                root,
                directory,
            }
        }

        fn workspace(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            fs::create_dir(&path).unwrap();
            path
        }

        fn source(&self, bytes: &[u8]) {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.directory)
                .unwrap();
            fs::write(self.file(), bytes).unwrap();
            fs::set_permissions(self.file(), fs::Permissions::from_mode(0o600)).unwrap();
        }

        fn file(&self) -> PathBuf {
            self.directory.join(CONFIG_FILE)
        }

        fn load(&self) -> Config {
            Config::load_from(&self.directory)
        }

        fn bytes(&self) -> Vec<u8> {
            fs::read(self.file()).unwrap()
        }

        fn table(&self) -> Table {
            fs::read_to_string(self.file()).unwrap().parse().unwrap()
        }
    }

    fn source_with_workspace(path: &Path) -> Table {
        let mut entry = Table::new();
        entry.insert("name".into(), Value::String("Work".into()));
        entry.insert("path".into(), path_value(path).unwrap());
        let mut table = Table::new();
        table.insert("schema_version".into(), Value::Integer(1));
        table.insert("workspaces".into(), Value::Array(vec![Value::Table(entry)]));
        table.insert("active".into(), path_value(path).unwrap());
        table
    }

    fn save_source(fixture: &Fixture, table: &Table) {
        fixture.source(toml::to_string(table).unwrap().as_bytes());
    }

    fn assert_defaults(config: &Config) {
        assert!(!config.writable());
        assert!(config.workspaces().is_empty());
        assert_eq!(config.active(), None);
        assert!(config.warning().unwrap().contains("read-only"));
        assert!(config
            .warning()
            .unwrap()
            .contains("will not be overwritten"));
    }

    #[test]
    fn missing_config_is_lazy_and_uses_empty_writable_defaults() {
        let fixture = Fixture::new();
        let config = fixture.load();
        assert!(config.writable());
        assert_eq!(config.warning(), None);
        assert_eq!(config.directory(), Some(fixture.directory.as_path()));
        assert!(config.workspaces().is_empty());
        assert_eq!(config.active(), None);
        assert!(!fixture.directory.exists());
        assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);

        fixture.source(b"");
        let config = fixture.load();
        assert!(config.writable());
        assert_eq!(config.warning(), None);
        assert!(!fixture.directory.join(".wren.lock").exists());
        assert_eq!(fixture.bytes(), b"");
    }

    #[test]
    fn platform_layout_uses_only_private_environment() {
        const CHILD_ROOT: &str = "WREN_CONFIG_LAYOUT_TEST_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            #[cfg(target_os = "macos")]
            let expected = root.join("home/Library/Application Support/Wren");
            #[cfg(not(target_os = "macos"))]
            let expected = root.join("xdg/wren");
            // Assert the discovered directory before any load/write could touch
            // a real user config if the platform discovery changed.
            let project = ProjectDirs::from("", "", "Wren").unwrap();
            assert_eq!(project.config_dir(), expected);
            let mut config = Config::load();
            assert_eq!(config.directory(), Some(expected.as_path()));
            assert!(config.writable(), "{:?}", config.warning());
            assert!(!expected.exists());
            let workspace = root.join("workspace");
            fs::create_dir(&workspace).unwrap();
            config.add(&workspace, None).unwrap();
            config.set_active(&workspace).unwrap();
            let reopened = Config::load();
            assert_eq!(reopened.active(), Some(workspace.as_path()));
            assert_eq!(reopened.workspaces().len(), 1);
            assert!(expected.join(CONFIG_FILE).is_file());
            return;
        }
        let temporary = TempDir::new().unwrap();
        let root = fs::canonicalize(temporary.path()).unwrap();
        fs::create_dir(root.join("home")).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "config::tests::platform_layout_uses_only_private_environment",
                "--nocapture",
            ])
            .env(CHILD_ROOT, &root)
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("xdg"))
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn two_workspaces_keep_registration_order_names_and_last_active_on_reopen() {
        let fixture = Fixture::new();
        let work = fixture.workspace("work");
        let personal = fixture.workspace("personal");
        let mut config = fixture.load();
        config.add(&work, None).unwrap();
        assert_eq!(config.active(), None, "add is not a session switch");
        config.set_active(&work).unwrap();
        config.add(&personal, Some("  Personal 日本語  ")).unwrap();
        assert_eq!(config.active(), Some(work.as_path()));
        config.set_active(&personal).unwrap();
        config.add(&work, Some("not a rename")).unwrap();
        let reopened = fixture.load();
        assert_eq!(
            reopened.workspaces(),
            [
                WorkspaceRegistration {
                    name: "work".into(),
                    path: work.clone()
                },
                WorkspaceRegistration {
                    name: "Personal 日本語".into(),
                    path: personal.clone()
                },
            ]
        );
        assert_eq!(reopened.active(), Some(personal.as_path()));
        assert_eq!(reopened.warning(), None);
        assert_eq!(fixture.table()["schema_version"].as_integer(), Some(1));
        config.remove(&work).unwrap();
        assert_eq!(fixture.load().active(), Some(personal.as_path()));
        config.remove(&personal).unwrap();
        assert_eq!(config.active(), None);
        assert!(fixture.load().workspaces().is_empty());
        assert!(!fixture.table().contains_key("active"));
    }

    #[test]
    fn new_config_directories_files_and_lock_are_private() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        fixture.load().add(&workspace, None).unwrap();
        for directory in ["private", "private/config", "private/config/wren"] {
            assert_eq!(
                fs::metadata(fixture.root.join(directory)).unwrap().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(fs::metadata(fixture.file()).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(fixture.directory.join(".wren.lock"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
        let names: Vec<_> = fs::read_dir(&fixture.directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            names.len(),
            2,
            "no temporary files survive a successful update"
        );
    }

    #[test]
    fn config_reads_preserve_native_permission_bits_and_writability() {
        let fixture = Fixture::new();
        let bytes = b"schema_version = 1\n";
        fixture.source(bytes);
        let directory = open_absolute_directory(&fixture.directory).unwrap();
        for (mode, expected) in [
            (0o400, 0o400),
            (0o640, 0o640),
            (0o777, 0o777),
            (0o4750, 0o750),
        ] {
            fs::set_permissions(fixture.file(), fs::Permissions::from_mode(mode)).unwrap();
            let Ok(file) = read_config(&directory) else {
                panic!("a readable regular config must retain its permissions");
            };
            assert_eq!(file.bytes, bytes);
            assert_eq!(file.mode, expected);
            assert_eq!(fixture.load().writable(), expected & 0o222 != 0);
        }
    }

    #[test]
    fn duplicates_are_canonicalized_and_selected_symlinks_are_rejected() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        fs::create_dir(workspace.join("child")).unwrap();
        let mut config = fixture.load();
        config.add(&workspace.join("."), None).unwrap();
        config.add(&workspace.join("child/.."), None).unwrap();
        assert_eq!(config.workspaces().len(), 1);
        assert_eq!(
            config.workspaces()[0].path,
            fs::canonicalize(&workspace).unwrap()
        );
        assert_eq!(fixture.table()["workspaces"].as_array().unwrap().len(), 1);
        let alias = fixture.root.join("alias");
        symlink(&workspace, &alias).unwrap();
        let before = fixture.bytes();
        for path in [alias.clone(), alias.join(""), alias.join(".")] {
            assert!(config.add(&path, None).is_err());
            assert!(config.set_active(&path).is_err());
        }
        assert_eq!(fixture.bytes(), before);
        assert!(fs::symlink_metadata(alias)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_dir(&workspace).unwrap().count(), 1);
    }

    #[test]
    fn invalid_adds_and_unregistered_activation_do_not_create_workspace_roots() {
        let fixture = Fixture::new();
        let existing = fixture.workspace("existing");
        let missing = fixture.root.join("missing");
        let file = fixture.root.join("file");
        fs::write(&file, b"unchanged").unwrap();
        let non_utf8 = fixture.root.join(OsStr::from_bytes(b"invalid-\xff"));
        assert!(path_value(&non_utf8).is_err());
        match fs::create_dir(&non_utf8) {
            Ok(()) => (),
            // APFS rejects the fixture name before a workspace can exist.
            Err(error) if error.raw_os_error() == Some(Errno::ILSEQ.raw_os_error()) => (),
            Err(error) => panic!("could not create non-UTF-8 workspace fixture: {error}"),
        }
        let mut config = fixture.load();
        for path in [&missing, &file, &PathBuf::from("relative"), &non_utf8] {
            assert!(config.add(path, None).is_err());
        }
        for name in ["bad\nname".to_owned(), "x".repeat(MAX_NAME_BYTES + 1)] {
            assert!(config.add(&existing, Some(&name)).is_err());
        }
        assert!(config.set_active(&existing).is_err());
        assert!(config.remove(&existing).is_err());
        assert!(config.workspaces().is_empty());
        assert!(!fixture.file().exists());
        assert!(!missing.exists());
        assert_eq!(fs::read(file).unwrap(), b"unchanged");
    }

    #[test]
    fn default_names_are_sensible_and_blank_overrides_use_the_basename() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("  日本語\nnotes  ");
        let mut config = fixture.load();
        config.add(&workspace, Some("  \t ")).unwrap();
        assert_eq!(config.workspaces()[0].name, "日本語 notes");
        assert_eq!(
            registration_name(Path::new("/"), None).unwrap(),
            "Workspace"
        );
    }

    #[test]
    fn unavailable_registrations_and_stale_active_remain_visible_and_removable() {
        let fixture = Fixture::new();
        let missing = fixture.root.join("no-longer-mounted");
        save_source(&fixture, &source_with_workspace(&missing));
        let original = fixture.bytes();
        let mut config = fixture.load();
        assert!(config.writable());
        assert_eq!(config.workspaces()[0].path, missing);
        assert_eq!(config.active(), Some(missing.as_path()));
        assert!(config.warning().unwrap().contains("unavailable"));
        assert!(config.set_active(&missing).is_err());
        assert_eq!(fixture.bytes(), original);
        assert!(!missing.exists());
        config.remove(&missing).unwrap();
        assert_eq!(config.active(), None);
        assert_eq!(config.warning(), None);
        assert!(fixture.load().workspaces().is_empty());
        assert!(!missing.exists());
    }

    #[test]
    fn replaced_workspace_registrations_can_be_removed_without_following_them() {
        let fixture = Fixture::new();
        let outside = fixture.workspace("outside");
        fs::write(outside.join("secret.md"), b"private").unwrap();
        for symlinked in [false, true] {
            let root = fixture
                .root
                .join(if symlinked { "link" } else { "file-root" });
            if symlinked {
                symlink(&outside, &root).unwrap();
            } else {
                fs::write(&root, b"not a directory").unwrap();
            }
            save_source(&fixture, &source_with_workspace(&root));
            let mut config = fixture.load();
            assert_eq!(config.workspaces().len(), 1);
            assert!(config.warning().unwrap().contains("unavailable"));
            config.remove(&root).unwrap();
            if symlinked {
                assert!(fs::symlink_metadata(&root)
                    .unwrap()
                    .file_type()
                    .is_symlink());
            } else {
                assert_eq!(fs::read(&root).unwrap(), b"not a directory");
            }
            assert_eq!(fs::read(outside.join("secret.md")).unwrap(), b"private");
        }
    }

    type Tree = BTreeMap<PathBuf, (u32, Vec<u8>)>;

    fn snapshot_tree(root: &Path) -> Tree {
        fn visit(root: &Path, parent: &Path, tree: &mut Tree) {
            for entry in fs::read_dir(parent).unwrap() {
                let path = entry.unwrap().path();
                let metadata = fs::symlink_metadata(&path).unwrap();
                let bytes = if metadata.file_type().is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_bytes()
                        .to_vec()
                } else if metadata.is_file() {
                    fs::read(&path).unwrap()
                } else {
                    Vec::new()
                };
                tree.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    (metadata.mode(), bytes),
                );
                if metadata.is_dir() {
                    visit(root, &path, tree);
                }
            }
        }
        let mut tree = Tree::new();
        visit(root, root, &mut tree);
        tree
    }

    #[test]
    fn removing_registration_preserves_every_markdown_metadata_and_ignored_file() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("notes");
        for directory in ["Section/Group", ".wren", ".git", ".hidden"] {
            fs::create_dir_all(workspace.join(directory)).unwrap();
        }
        for (path, contents) in [
            ("Section/Group/note.md", b"# authoritative\n".as_slice()),
            ("Section/ungrouped.md", b"plain text"),
            ("root.md", b"unsupported root note"),
            (".wren/workspace.toml", b"unknown = true\n"),
            (".wren.lock", b"existing lock contents"),
            (".git/HEAD", b"ref: refs/heads/main\n"),
            (".hidden/private.md", b"ignored"),
            ("image.bin", b"\xff\x00\x01"),
        ] {
            fs::write(workspace.join(path), contents).unwrap();
        }
        symlink("Section/Group/note.md", workspace.join("alias.md")).unwrap();
        let before = snapshot_tree(&workspace);
        let mut config = fixture.load();
        config.add(&workspace, None).unwrap();
        config.set_active(&workspace).unwrap();
        config.remove(&workspace).unwrap();
        assert_eq!(snapshot_tree(&workspace), before);
        assert!(workspace.is_dir());
        assert!(fixture.load().workspaces().is_empty());
    }

    #[test]
    fn unknown_toml_values_and_duplicate_entry_metadata_survive_updates() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        let personal = fixture.workspace("personal");
        let mut source = source_with_workspace(&workspace);
        let extra: Table = "future_flag = true\nfuture_float = 1.25\nfuture_date = 2026-11-04T12:30:00Z\nfuture_array = [1, 'two', { three = 3 }]\n[plugin]\nsecret = 'preserve-dont-log'\n".parse().unwrap();
        source.extend(extra.clone());
        let entries = source["workspaces"].as_array_mut().unwrap();
        entries[0]
            .as_table_mut()
            .unwrap()
            .insert("future".into(), Value::String("first".into()));
        let mut duplicate = entries[0].clone();
        duplicate
            .as_table_mut()
            .unwrap()
            .insert("path".into(), path_value(&workspace.join(".")).unwrap());
        duplicate
            .as_table_mut()
            .unwrap()
            .insert("future".into(), Value::String("second".into()));
        entries.push(duplicate);
        save_source(&fixture, &source);
        let original = fixture.bytes();
        let mut config = fixture.load();
        assert!(config.writable());
        assert_eq!(config.workspaces().len(), 1);
        assert_eq!(fixture.bytes(), original, "load must not normalize on disk");
        config.add(&personal, None).unwrap();
        config.set_active(&workspace).unwrap();
        config.remove(&personal).unwrap();
        let saved = fixture.table();
        for (key, value) in extra {
            assert_eq!(saved[&key], value, "{key}");
        }
        let entries = saved["workspaces"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["future"].as_str(), Some("first"));
        assert_eq!(entries[1]["future"].as_str(), Some("second"));
        for entry in entries {
            assert_eq!(entry["path"], path_value(&workspace).unwrap());
        }
        config.remove(&workspace).unwrap();
        assert!(fixture.table()["workspaces"].as_array().unwrap().is_empty());
    }

    #[test]
    fn malformed_utf8_unsupported_and_invalid_known_fields_are_read_only_defaults() {
        let cases: &[&[u8]] = &[
            b"workspaces = [",
            b"\xff\xfe",
            b"schema_version = 2",
            b"schema_version = '1'",
            b"schema_version = true",
            b"workspaces = 3",
            b"workspaces = ['path']",
            b"workspaces = [{ name = 3, path = '/absolute' }]",
            b"workspaces = [{ name = '', path = '/absolute' }]",
            b"workspaces = [{ name = 'Work', path = 'relative' }]",
            b"workspaces = [{ name = 'Work', path = '/../traversal' }]",
            b"workspaces = [{ name = 'Work', path = 3 }]",
            b"active = 1",
            b"active = '/unregistered'",
            b"secret = 'do-not-display'\nbroken = [",
        ];
        for bytes in cases {
            let fixture = Fixture::new();
            let workspace = fixture.workspace("work");
            fixture.source(bytes);
            let mut config = fixture.load();
            assert_defaults(&config);
            assert!(config.add(&workspace, None).is_err());
            assert!(config.set_active(&workspace).is_err());
            assert!(config.remove(&workspace).is_err());
            assert_eq!(fixture.bytes(), *bytes);
            assert!(!config.warning().unwrap().contains("do-not-display"));
            assert!(!config
                .warning()
                .unwrap()
                .contains(fixture.root.to_str().unwrap()));
            assert!(!fixture.directory.join(".wren.lock").exists());
        }
    }

    #[test]
    fn readable_read_only_config_retains_registrations_but_never_overwrites() {
        for readonly_directory in [false, true] {
            let fixture = Fixture::new();
            let workspace = fixture.workspace("work");
            save_source(&fixture, &source_with_workspace(&workspace));
            let before = fixture.bytes();
            let restricted = if readonly_directory {
                &fixture.directory
            } else {
                &fixture.file()
            };
            fs::set_permissions(
                restricted,
                fs::Permissions::from_mode(if readonly_directory { 0o500 } else { 0o400 }),
            )
            .unwrap();
            let mut config = fixture.load();
            let result = config.remove(&workspace);
            fs::set_permissions(
                restricted,
                fs::Permissions::from_mode(if readonly_directory { 0o700 } else { 0o600 }),
            )
            .unwrap();
            assert!(!config.writable());
            assert_eq!(config.workspaces().len(), 1);
            assert_eq!(config.active(), Some(workspace.as_path()));
            assert!(result.is_err());
            assert_eq!(fixture.bytes(), before);
            // Optional global configuration never owns the note writer.
            let mut store = FsStore::open(&workspace).unwrap();
            store.ensure_dir(&RelPath::new("Section").unwrap()).unwrap();
            store
                .write(
                    &RelPath::new("Section/note.md").unwrap(),
                    b"saved normally",
                    Expected::Missing,
                )
                .unwrap();
            assert_eq!(
                fs::read(workspace.join("Section/note.md")).unwrap(),
                b"saved normally"
            );
        }
    }

    #[test]
    fn unreadable_config_is_left_untouched_and_uses_read_only_defaults() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        save_source(&fixture, &source_with_workspace(&workspace));
        let before = fixture.bytes();
        fs::set_permissions(fixture.file(), fs::Permissions::from_mode(0o0)).unwrap();
        let config = fixture.load();
        fs::set_permissions(fixture.file(), fs::Permissions::from_mode(0o600)).unwrap();
        assert_defaults(&config);
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn symlink_and_dangling_config_files_and_directories_are_never_followed() {
        for target_exists in [false, true] {
            let fixture = Fixture::new();
            let workspace = fixture.workspace("work");
            let target = fixture.root.join("outside.toml");
            if target_exists {
                fs::write(&target, b"future = 'private'\n").unwrap();
            }
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&fixture.directory)
                .unwrap();
            symlink(&target, fixture.file()).unwrap();
            let mut config = fixture.load();
            assert_defaults(&config);
            assert!(config.add(&workspace, None).is_err());
            assert!(fs::symlink_metadata(fixture.file())
                .unwrap()
                .file_type()
                .is_symlink());
            if target_exists {
                assert_eq!(fs::read(target).unwrap(), b"future = 'private'\n");
            } else {
                assert!(!target.exists());
            }
        }
        let fixture = Fixture::new();
        let outside = fixture.workspace("outside");
        let link = fixture.root.join("config-link");
        symlink(&outside, &link).unwrap();
        assert_defaults(&Config::load_from(&link));
        assert_defaults(&Config::load_from(&link.join("missing")));
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[test]
    fn nonregular_oversized_and_unreadable_directory_configs_are_read_only() {
        let fixture = Fixture::new();
        fixture.source(b"");
        fs::remove_file(fixture.file()).unwrap();
        fs::create_dir(fixture.file()).unwrap();
        assert_defaults(&fixture.load());
        fs::remove_dir(fixture.file()).unwrap();
        let oversized = File::create(fixture.file()).unwrap();
        oversized.set_len(MAX_CONFIG_BYTES + 1).unwrap();
        let config = fixture.load();
        assert_defaults(&config);
        assert!(config.warning().unwrap().contains("1 MiB"));
        assert_eq!(oversized.metadata().unwrap().len(), MAX_CONFIG_BYTES + 1);
        fs::set_permissions(&fixture.directory, fs::Permissions::from_mode(0o0)).unwrap();
        let unreadable = fixture.load();
        fs::set_permissions(&fixture.directory, fs::Permissions::from_mode(0o700)).unwrap();
        assert_defaults(&unreadable);
        #[cfg(not(target_os = "macos"))]
        {
            fs::remove_file(fixture.file()).unwrap();
            rustix::fs::mkfifoat(rustix::fs::CWD, fixture.file(), Mode::from_raw_mode(0o600))
                .unwrap();
            assert_defaults(&fixture.load());
        }
    }

    #[test]
    fn exact_baseline_protects_same_length_same_mtime_external_changes() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        let original = b"future = 'one'\n";
        let external = b"future = 'two'\n";
        assert_eq!(original.len(), external.len());
        fixture.source(original);
        let times = FileTimes::new()
            .set_modified(fs::metadata(fixture.file()).unwrap().modified().unwrap());
        let mut config = fixture.load();
        fs::write(fixture.file(), external).unwrap();
        File::options()
            .write(true)
            .open(fixture.file())
            .unwrap()
            .set_times(times)
            .unwrap();
        let error = config.add(&workspace, None).unwrap_err();
        assert!(error.contains("changed since it was loaded"));
        assert_eq!(fixture.bytes(), external);
        assert_eq!(config.baseline.as_deref(), Some(original.as_slice()));
        assert!(config.workspaces().is_empty());
        assert_eq!(config.active(), None);
    }

    #[test]
    fn externally_created_removed_or_symlinked_config_is_not_overwritten() {
        for replacement in ["created", "removed", "symlinked"] {
            let fixture = Fixture::new();
            let workspace = fixture.workspace("work");
            if replacement != "created" {
                fixture.source(b"future = 1\n");
            }
            let mut config = fixture.load();
            let outside = fixture.root.join("outside.toml");
            fs::write(&outside, b"future = 9\n").unwrap();
            match replacement {
                "created" => fixture.source(b"future = 2\n"),
                "removed" => fs::remove_file(fixture.file()).unwrap(),
                "symlinked" => {
                    fs::remove_file(fixture.file()).unwrap();
                    symlink(&outside, fixture.file()).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(config.add(&workspace, None).is_err());
            assert!(config.workspaces().is_empty());
            assert_eq!(fs::read(outside).unwrap(), b"future = 9\n");
            match replacement {
                "created" => assert_eq!(fixture.bytes(), b"future = 2\n"),
                "removed" => assert!(!fixture.file().exists()),
                "symlinked" => assert!(fs::symlink_metadata(fixture.file())
                    .unwrap()
                    .file_type()
                    .is_symlink()),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn replaced_config_directory_is_rejected_even_when_baseline_bytes_match() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        fixture.source(b"future = 1\n");
        let mut config = fixture.load();
        let old = fixture.root.join("old-config");
        fs::rename(&fixture.directory, &old).unwrap();
        fixture.source(b"future = 1\n");
        assert!(config
            .add(&workspace, None)
            .unwrap_err()
            .contains("replaced"));
        assert!(config.workspaces().is_empty());
        assert_eq!(fixture.bytes(), b"future = 1\n");
        assert_eq!(fs::read(old.join(CONFIG_FILE)).unwrap(), b"future = 1\n");
        assert!(!fixture.directory.join(".wren.lock").exists());
    }

    #[test]
    fn independently_loaded_writers_conflict_without_lifetime_global_locking() {
        let fixture = Fixture::new();
        let work = fixture.workspace("work");
        let personal = fixture.workspace("personal");
        let mut first = fixture.load();
        let mut second = fixture.load();
        first.add(&work, None).unwrap();
        assert!(second
            .add(&personal, None)
            .unwrap_err()
            .contains("changed since it was loaded"));
        assert!(second.workspaces().is_empty());
        assert_eq!(fixture.load().workspaces(), first.workspaces());
        let lock = FsStore::open(&fixture.directory).unwrap();
        let while_locked = fixture.load();
        assert_eq!(while_locked.workspaces(), first.workspaces());
        drop(lock);
        let mut second = fixture.load();
        second.add(&personal, None).unwrap();
        assert_eq!(second.workspaces().len(), 2);
    }

    #[test]
    fn cross_process_lock_contention_does_not_block_reading_or_note_editing() {
        const CHILD_ROOT: &str = "WREN_CONFIG_LOCK_TEST_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            let directory = root.join("private/config/wren");
            let workspace = root.join("work");
            let mut config = Config::load_from(&directory);
            assert_eq!(config.workspaces().len(), 1);
            assert!(config.set_active(&workspace).unwrap_err().contains("busy"));
            assert_eq!(config.active(), None);
            let mut store = FsStore::open(&workspace).unwrap();
            store.ensure_dir(&RelPath::new("Section").unwrap()).unwrap();
            store
                .write(
                    &RelPath::new("Section/note.md").unwrap(),
                    b"saved under config contention",
                    Expected::Missing,
                )
                .unwrap();
            return;
        }
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        let mut config = fixture.load();
        config.add(&workspace, None).unwrap();
        let before = fixture.bytes();
        let lock = FsStore::open(&fixture.directory).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "config::tests::cross_process_lock_contention_does_not_block_reading_or_note_editing", "--nocapture"])
            .env(CHILD_ROOT, &fixture.root).status().unwrap();
        assert!(status.success());
        assert_eq!(fixture.bytes(), before);
        assert_eq!(
            fs::read(workspace.join("Section/note.md")).unwrap(),
            b"saved under config contention"
        );
        drop(lock);
        config.set_active(&workspace).unwrap();
        assert_eq!(fixture.load().active(), Some(workspace.as_path()));
    }

    #[derive(Clone, Copy, Debug)]
    enum Mutation {
        Add,
        Remove,
        Activate,
    }

    fn mutate(
        config: &mut Config,
        kind: Mutation,
        work: &Path,
        personal: &Path,
    ) -> Result<(), String> {
        match kind {
            Mutation::Add => config.add(personal, None),
            Mutation::Remove => config.remove(work),
            Mutation::Activate => config.set_active(personal),
        }
    }

    #[test]
    fn prepublication_failures_roll_back_all_in_memory_mutations_and_allow_retry() {
        for kind in [Mutation::Add, Mutation::Remove, Mutation::Activate] {
            for fault in [
                Fault::Write,
                Fault::Permissions,
                Fault::FileSync,
                Fault::BeforePublish,
            ] {
                let fixture = Fixture::new();
                let work = fixture.workspace("work");
                let personal = fixture.workspace("personal");
                let mut config = fixture.load();
                config.add(&work, None).unwrap();
                if matches!(kind, Mutation::Activate) {
                    config.add(&personal, None).unwrap();
                }
                config.set_active(&work).unwrap();
                let bytes = fixture.bytes();
                let table = config.registry.table.clone();
                let workspaces = config.workspaces().to_vec();
                config.fault = Some(fault);
                assert!(
                    mutate(&mut config, kind, &work, &personal).is_err(),
                    "{kind:?} {fault:?}"
                );
                assert_eq!(fixture.bytes(), bytes, "{kind:?} {fault:?}");
                assert_eq!(config.baseline.as_deref(), Some(bytes.as_slice()));
                assert_eq!(config.registry.table, table);
                assert_eq!(config.workspaces(), workspaces);
                assert_eq!(config.active(), Some(work.as_path()));
                assert_eq!(
                    fs::read_dir(&fixture.directory).unwrap().count(),
                    2,
                    "failed temporaries must be cleaned up"
                );
                mutate(&mut config, kind, &work, &personal).unwrap();
                assert_eq!(fixture.load().workspaces(), config.workspaces());
                assert_eq!(fixture.load().active(), config.active());
                assert_eq!(config.warning(), None);
            }
        }
    }

    #[test]
    fn failed_initial_publication_does_not_invent_registration_or_baseline() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        let mut config = fixture.load();
        config.fault = Some(Fault::BeforePublish);
        assert!(config.add(&workspace, None).is_err());
        assert!(config.workspaces().is_empty());
        assert_eq!(config.baseline, None);
        assert!(!fixture.file().exists());
        config.add(&workspace, None).unwrap();
        assert_eq!(fixture.load().workspaces().len(), 1);
    }

    #[test]
    fn postpublication_errors_retain_committed_registry_baseline_and_retryable_sync() {
        for kind in [Mutation::Add, Mutation::Remove, Mutation::Activate] {
            let fixture = Fixture::new();
            let work = fixture.workspace("work");
            let personal = fixture.workspace("personal");
            let mut config = fixture.load();
            config.add(&work, None).unwrap();
            if matches!(kind, Mutation::Activate) {
                config.add(&personal, None).unwrap();
            }
            config.set_active(&work).unwrap();
            config.fault = Some(Fault::DirectorySync);
            let error = mutate(&mut config, kind, &work, &personal).unwrap_err();
            assert!(error.contains("was published"), "{kind:?}: {error}");
            let published = fixture.bytes();
            assert_eq!(config.baseline.as_deref(), Some(published.as_slice()));
            let reopened = fixture.load();
            assert_eq!(reopened.workspaces(), config.workspaces());
            assert_eq!(reopened.active(), config.active());
            match kind {
                Mutation::Add => assert_eq!(config.workspaces().len(), 2),
                Mutation::Remove => {
                    assert!(config.workspaces().is_empty());
                    assert_eq!(config.active(), None);
                }
                Mutation::Activate => assert_eq!(config.active(), Some(personal.as_path())),
            }
            assert!(config.writable());
            assert!(config.pending_store.as_ref().unwrap().pending_durability());
            config
                .pending_store
                .as_mut()
                .unwrap()
                .fail_next(Fault::DirectorySync);
            assert!(config.retry_durability().is_err());
            assert_eq!(fixture.bytes(), published);
            assert_eq!(config.baseline.as_deref(), Some(published.as_slice()));
            config.retry_durability().unwrap();
            assert_eq!(
                fixture.bytes(),
                published,
                "a durability retry must not rewrite the file"
            );
            assert!(config.pending_store.is_none());
            assert_eq!(config.warning(), None);
            // The next update uses the new baseline, not the prepublication one.
            config.add(&work, None).unwrap();
            config.set_active(&work).unwrap();
            assert_eq!(fixture.load().active(), Some(work.as_path()));
        }
    }

    #[test]
    fn saved_paths_with_replaced_symlink_ancestors_stay_stale_not_redirected() {
        let fixture = Fixture::new();
        let parent = fixture.workspace("original-parent");
        let workspace = parent.join("work");
        fs::create_dir(&workspace).unwrap();
        let mut config = fixture.load();
        config.add(&workspace, None).unwrap();
        config.set_active(&workspace).unwrap();
        let renamed = fixture.root.join("renamed-parent");
        fs::rename(&parent, &renamed).unwrap();
        let outside = fixture.workspace("outside");
        fs::create_dir(outside.join("work")).unwrap();
        fs::write(outside.join("work/private.md"), b"do not follow").unwrap();
        symlink(&outside, &parent).unwrap();
        let mut reopened = fixture.load();
        assert_eq!(reopened.workspaces()[0].path, workspace);
        assert_eq!(reopened.active(), Some(workspace.as_path()));
        assert!(reopened.warning().unwrap().contains("unavailable"));
        assert!(reopened.set_active(&workspace).is_err());
        reopened.remove(&workspace).unwrap();
        assert!(renamed.join("work").is_dir());
        assert!(fs::symlink_metadata(parent)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read(outside.join("work/private.md")).unwrap(),
            b"do not follow"
        );
    }

    #[test]
    fn activation_cannot_redirect_a_stored_root_even_to_another_registered_workspace() {
        let fixture = Fixture::new();
        let original_parent = fixture.workspace("original-parent");
        let original = original_parent.join("work");
        fs::create_dir(&original).unwrap();
        let destination_parent = fixture.workspace("destination-parent");
        let destination = destination_parent.join("work");
        fs::create_dir(&destination).unwrap();
        let mut config = fixture.load();
        config.add(&original, None).unwrap();
        config.add(&destination, None).unwrap();
        config.set_active(&original).unwrap();
        let before = fixture.bytes();
        let moved = fixture.root.join("moved-parent");
        fs::rename(&original_parent, &moved).unwrap();
        symlink(&destination_parent, &original_parent).unwrap();

        assert!(config.validate_registration(&original).is_err());
        assert!(config.set_active(&original).is_err());
        assert_eq!(config.active(), Some(original.as_path()));
        assert_eq!(fixture.bytes(), before);
        let mut reopened = fixture.load();
        assert!(reopened.validate_registration(&original).is_err());
        assert_eq!(
            reopened.validate_registration(&destination).unwrap(),
            destination
        );
        assert!(reopened.set_active(&original).is_err());
        assert_eq!(fixture.bytes(), before);
        assert!(moved.join("work").is_dir());
        assert!(destination.is_dir());
        reopened.set_active(&destination).unwrap();
        assert_eq!(reopened.active(), Some(destination.as_path()));
    }

    #[test]
    fn registration_validation_is_nonmutating_and_independent_of_config_writes() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        let mut config = fixture.load();
        config.add(&workspace, None).unwrap();
        let before = fixture.bytes();
        fs::set_permissions(fixture.file(), fs::Permissions::from_mode(0o400)).unwrap();
        let config = fixture.load();
        assert!(!config.writable());
        let lock = FsStore::open(&fixture.directory).unwrap();
        assert_eq!(config.validate_registration(&workspace).unwrap(), workspace);
        assert!(config
            .validate_registration(&fixture.workspace("unregistered"))
            .is_err());
        assert_eq!(config.active(), None);
        assert_eq!(fixture.bytes(), before);
        drop(lock);
    }

    #[test]
    fn permission_changes_after_loading_prevent_publication_without_losing_registry() {
        for restrict_directory in [false, true] {
            let fixture = Fixture::new();
            let workspace = fixture.workspace("work");
            let mut config = fixture.load();
            config.add(&workspace, None).unwrap();
            let before = fixture.bytes();
            let restricted = if restrict_directory {
                fixture.directory.clone()
            } else {
                fixture.file()
            };
            let mode = if restrict_directory { 0o500 } else { 0o400 };
            fs::set_permissions(&restricted, fs::Permissions::from_mode(mode)).unwrap();
            let result = config.remove(&workspace);
            fs::set_permissions(
                &restricted,
                fs::Permissions::from_mode(if restrict_directory { 0o700 } else { 0o600 }),
            )
            .unwrap();
            assert!(result.is_err());
            assert_eq!(fixture.bytes(), before);
            assert_eq!(config.workspaces().len(), 1);
            assert_eq!(config.baseline.as_deref(), Some(before.as_slice()));
            config.remove(&workspace).unwrap();
            assert!(fixture.load().workspaces().is_empty());
        }
    }

    #[test]
    fn bounded_config_limits_fail_without_touching_source_or_claiming_publication() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        let mut source = source_with_workspace(&workspace);
        let entry = source["workspaces"].as_array().unwrap()[0].clone();
        source.insert(
            "workspaces".into(),
            Value::Array(vec![entry; MAX_WORKSPACES + 1]),
        );
        save_source(&fixture, &source);
        let original = fixture.bytes();
        assert!(original.len() as u64 <= MAX_CONFIG_BYTES);
        assert_defaults(&fixture.load());
        assert_eq!(fixture.bytes(), original);
        assert!(!fixture.directory.join(".wren.lock").exists());

        // A valid file can fit the read bound but leave no space for a new
        // registration once TOML is serialized. Reject before even locking.
        let mut source = Table::new();
        source.insert(
            "future".into(),
            Value::String("x".repeat(MAX_CONFIG_BYTES as usize - 32)),
        );
        save_source(&fixture, &source);
        let original = fixture.bytes();
        assert!(original.len() as u64 <= MAX_CONFIG_BYTES);
        let mut config = fixture.load();
        assert!(config.writable(), "{:?}", config.warning());
        assert!(config.add(&workspace, None).unwrap_err().contains("1 MiB"));
        assert!(config.workspaces().is_empty());
        assert_eq!(config.baseline.as_deref(), Some(original.as_slice()));
        assert_eq!(fixture.bytes(), original);
        assert!(!fixture.directory.join(".wren.lock").exists());
    }

    #[test]
    fn existing_config_permissions_survive_atomic_replacement() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        fixture.source(b"future = true\n");
        fs::set_permissions(fixture.file(), fs::Permissions::from_mode(0o640)).unwrap();
        let mut config = fixture.load();
        config.add(&workspace, None).unwrap();
        assert_eq!(fs::metadata(fixture.file()).unwrap().mode() & 0o777, 0o640);
        assert_eq!(fixture.table()["future"], Value::Boolean(true));
    }

    #[test]
    fn initial_postpublication_error_keeps_registration_and_can_retry_via_next_update() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace("work");
        let mut config = fixture.load();
        config.fault = Some(Fault::DirectorySync);
        assert!(config
            .add(&workspace, None)
            .unwrap_err()
            .contains("was published"));
        assert_eq!(config.workspaces().len(), 1);
        assert_eq!(fixture.load().workspaces(), config.workspaces());
        config.set_active(&workspace).unwrap();
        assert_eq!(config.warning(), None);
        assert!(config.pending_store.is_none());
        assert_eq!(fixture.load().active(), Some(workspace.as_path()));
    }
}
