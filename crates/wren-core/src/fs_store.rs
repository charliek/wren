//! Descriptor-relative storage for a cooperatively locked workspace.
//!
//! No operation follows a workspace symlink. Directory and leaf identities are
//! rechecked before publication. An uncooperative process can still rename an
//! ancestor after its final check, or change an existing file between its final
//! comparison and `renameat`: Unix does not provide a byte-conditional rename.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use rustix::fs::{self, AtFlags, FileType, FlockOperation, Mode, OFlags, RenameFlags, Stat};
use rustix::io::Errno;

const LOCK_NAME: &str = ".wren.lock";
const MAX_NAME_BYTES: usize = 240;
// macOS gives GUI processes a 256-descriptor soft limit; retained directory
// descriptors must leave most of it for the watcher, fonts, and the renderer.
const MAX_PENDING_DIRECTORIES: usize = 32;
const MAX_WARNINGS: usize = 64;
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct RelPath(PathBuf);

impl RelPath {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        let raw = path.as_os_str().as_bytes();
        // Path::components normalizes embedded `.` away, so inspect raw bytes.
        if raw.is_empty()
            || path.is_absolute()
            || raw.contains(&0)
            || raw
                .split(|byte| *byte == b'/')
                .any(|part| part.is_empty() || part == b"." || part == b"..")
        {
            return Err(StoreError::InvalidPath(path.display().to_string()));
        }
        Ok(Self(path.to_path_buf()))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> Result<Self, StoreError> {
        let child = Self::new(name)?;
        Self::new(self.0.join(child.as_path()))
    }

    pub fn parent(&self) -> Option<Self> {
        self.0
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| Self(path.to_path_buf()))
    }

    pub fn file_name(&self) -> &OsStr {
        self.0
            .file_name()
            .expect("validated relative path has a leaf")
    }
}

#[derive(Clone, Debug)]
pub struct DiskFile {
    pub bytes: Vec<u8>,
    pub modified: SystemTime,
    pub mode: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug)]
pub struct DirectoryEntry {
    pub name: OsString,
    pub kind: Result<EntryKind, StoreError>,
}

#[derive(Debug)]
pub struct DirectoryListing {
    pub entries: Vec<DirectoryEntry>,
    /// Errors reading entry names make the listing partial, not empty.
    pub enumeration_errors: Vec<StoreError>,
}

#[derive(Clone, Copy, Debug)]
pub enum Expected<'a> {
    Missing,
    Bytes(&'a [u8]),
}

#[derive(Clone, Debug)]
pub struct WriteReceipt {
    pub path: RelPath,
    pub file: DiskFile,
    pub durability_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MoveReceipt {
    pub path: RelPath,
    pub durability_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    Write,
    Permissions,
    FileSync,
    BeforePublish,
    DirectorySync,
}

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    InvalidPath(String),
    InvalidName(String),
    UnsafePath(String),
    Conflict(String),
    Locked,
    Injected(Fault),
}

impl StoreError {
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict(_))
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Io(error) if error.kind() == io::ErrorKind::NotFound)
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "storage I/O: {error}"),
            Self::InvalidPath(path) => write!(formatter, "invalid relative path: {path:?}"),
            Self::InvalidName(name) => write!(formatter, "invalid name: {name:?}"),
            Self::UnsafePath(path) => {
                write!(formatter, "unsafe or replaced workspace path: {path}")
            }
            Self::Conflict(path) => {
                write!(formatter, "file changed or destination occupied: {path}")
            }
            Self::Locked => formatter.write_str("workspace is already locked by another writer"),
            Self::Injected(fault) => write!(formatter, "injected storage failure: {fault:?}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<Errno> for StoreError {
    fn from(error: Errno) -> Self {
        Self::Io(error.into())
    }
}

#[derive(Debug)]
struct PendingDirectory {
    file: File,
    device: u64,
    inode: u64,
}

/// Read-only, descriptor-relative access to a store's validated root. A
/// duplicate shares the root directory descriptor (never the lock descriptor)
/// so it can scan on another thread; every access still revalidates the root
/// path and the lock name against the owning store's identities.
pub struct StoreReader {
    root: PathBuf,
    directory: File,
    lock: Stat,
}

impl fmt::Debug for StoreReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreReader")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl StoreReader {
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Duplicates the root directory descriptor. Dropping the duplicate can
    /// neither release nor extend the owning store's advisory lock.
    pub fn try_clone(&self) -> Result<Self, StoreError> {
        Ok(Self {
            root: self.root.clone(),
            directory: self.directory.try_clone()?,
            lock: self.lock,
        })
    }

    pub fn read(&self, path: &RelPath) -> Result<DiskFile, StoreError> {
        let parent = path.parent();
        let directory = self.open_directory(parent.as_ref())?;
        let snapshot = snapshot(&directory, path.file_name())?;
        self.check_directory(parent.as_ref(), &directory)?;
        Ok(snapshot.file)
    }

    /// Lists the root (`None`) or a relative directory without following symlinks.
    /// Names are sorted and retain their original bytes. Enumeration failures and
    /// individual metadata failures remain in a successful, possibly partial
    /// listing; failure to open or revalidate the directory rejects the listing.
    pub fn list_dir(&self, path: Option<&RelPath>) -> Result<DirectoryListing, StoreError> {
        self.list_dir_with(path, read_directory)
    }

    fn list_dir_with(
        &self,
        path: Option<&RelPath>,
        enumerate: impl FnOnce(&File) -> Result<DirectoryListing, StoreError>,
    ) -> Result<DirectoryListing, StoreError> {
        let directory = self.open_directory(path)?;
        self.check_directory(path, &directory)?;
        let listing = enumerate(&directory)?;
        self.check_directory(path, &directory)?;
        Ok(listing)
    }

    fn validate_root(&self) -> Result<(), StoreError> {
        let current = open_absolute_directory(&self.root)?;
        if !same_object(&fs::fstat(&current)?, &fs::fstat(&self.directory)?) {
            return Err(StoreError::UnsafePath(self.root.display().to_string()));
        }
        let named_lock = fs::statat(&self.directory, LOCK_NAME, AtFlags::SYMLINK_NOFOLLOW)?;
        if !same_object(&named_lock, &self.lock) {
            return Err(StoreError::UnsafePath(LOCK_NAME.into()));
        }
        Ok(())
    }

    fn open_directory(&self, path: Option<&RelPath>) -> Result<File, StoreError> {
        self.validate_root()?;
        let mut directory = self.directory.try_clone()?;
        if let Some(path) = path {
            for component in path.as_path().components() {
                directory = match fs::openat(
                    &directory,
                    component.as_os_str(),
                    directory_flags(),
                    Mode::empty(),
                ) {
                    Ok(file) => file.into(),
                    Err(error) => {
                        // O_NOFOLLOW | O_DIRECTORY can report ENOTDIR for a
                        // symlink. Keep unsafe replacement distinct from a
                        // genuinely missing directory for scan preservation.
                        if fs::statat(&directory, component.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)
                            .is_ok_and(|stat| {
                                FileType::from_raw_mode(stat.st_mode) == FileType::Symlink
                            })
                        {
                            return Err(StoreError::UnsafePath(
                                path.as_path().display().to_string(),
                            ));
                        }
                        return Err(error.into());
                    }
                };
            }
        }
        Ok(directory)
    }

    fn check_directory(&self, path: Option<&RelPath>, directory: &File) -> Result<(), StoreError> {
        let current = self.open_directory(path)?;
        if !same_object(&fs::fstat(directory)?, &fs::fstat(&current)?) {
            return Err(StoreError::UnsafePath(path.map_or_else(
                || self.root.display().to_string(),
                |path| path.as_path().display().to_string(),
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct FsStore {
    reader: StoreReader,
    // This descriptor, and the persistent name, must survive for the entire lock.
    _lock: File,
    fault: Option<Fault>,
    pending_directories: Vec<PendingDirectory>,
    // Losing an unsynced descriptor means later retries cannot prove durability.
    durability_tracking_error: Option<String>,
    warnings: Vec<String>,
}

impl FsStore {
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        let selected = std::fs::symlink_metadata(root)?;
        if !selected.is_dir() || selected.file_type().is_symlink() {
            return Err(StoreError::UnsafePath(root.display().to_string()));
        }
        // Resolve the user-selected root once (including platform aliases such
        // as macOS /var); every subsequent open walks this physical path.
        let root = std::fs::canonicalize(root)?;
        let directory = open_absolute_directory(&root)?;
        let opened = directory.metadata()?;
        if selected.dev() != opened.dev() || selected.ino() != opened.ino() {
            return Err(StoreError::UnsafePath(root.display().to_string()));
        }
        let lock: File = fs::openat(
            &directory,
            LOCK_NAME,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?
        .into();
        require_type(&fs::fstat(&lock)?, FileType::RegularFile, LOCK_NAME)?;
        if let Err(error) = fs::flock(&lock, FlockOperation::NonBlockingLockExclusive) {
            return if error == Errno::WOULDBLOCK {
                Err(StoreError::Locked)
            } else {
                Err(error.into())
            };
        }
        let reader = StoreReader {
            root,
            directory,
            lock: fs::fstat(&lock)?,
        };
        let store = Self {
            reader,
            _lock: lock,
            fault: None,
            pending_directories: Vec::new(),
            durability_tracking_error: None,
            warnings: Vec::new(),
        };
        store.validate_root()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.reader.root
    }

    pub fn reader(&self) -> &StoreReader {
        &self.reader
    }

    pub fn fail_next(&mut self, fault: Fault) {
        self.fault = Some(fault);
    }

    pub fn pending_durability(&self) -> bool {
        !self.pending_directories.is_empty() || self.durability_tracking_error.is_some()
    }

    pub fn retry_durability(&mut self) -> Result<(), StoreError> {
        self.validate_root()?;
        let mut failure = self
            .durability_tracking_error
            .as_ref()
            .map(|error| StoreError::Io(io::Error::other(error.clone())));
        for directory in std::mem::take(&mut self.pending_directories) {
            if let Err(error) = self.sync_directory(&directory.file) {
                failure.get_or_insert(error);
                self.pending_directories.push(directory);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    pub fn read(&self, path: &RelPath) -> Result<DiskFile, StoreError> {
        self.reader.read(path)
    }

    pub fn list_dir(&self, path: Option<&RelPath>) -> Result<DirectoryListing, StoreError> {
        self.reader.list_dir(path)
    }

    #[cfg(test)]
    fn list_dir_with(
        &self,
        path: Option<&RelPath>,
        enumerate: impl FnOnce(&File) -> Result<DirectoryListing, StoreError>,
    ) -> Result<DirectoryListing, StoreError> {
        self.reader.list_dir_with(path, enumerate)
    }

    pub fn write(
        &mut self,
        path: &RelPath,
        bytes: &[u8],
        expected: Expected<'_>,
    ) -> Result<WriteReceipt, StoreError> {
        reject_lock_target(path)?;
        let parent = path.parent();
        let directory = self.open_directory(parent.as_ref())?;
        let baseline = match expected {
            Expected::Missing => {
                require_missing(&directory, path.file_name(), path)?;
                None
            }
            Expected::Bytes(expected) => {
                let current = snapshot(&directory, path.file_name()).map_err(|error| {
                    if error.is_not_found() {
                        conflict(path)
                    } else {
                        error
                    }
                })?;
                if current.file.bytes != expected {
                    return Err(conflict(path));
                }
                Some(current)
            }
        };
        let mode = baseline.as_ref().map(|value| value.file.mode);
        let (mut temporary, file) = self.prepare_file(&directory, bytes, mode)?;
        self.inject(Fault::BeforePublish)?;
        self.check_directory(parent.as_ref(), &directory)?;
        match &baseline {
            Some(baseline) => {
                let current = snapshot(&directory, path.file_name()).map_err(|error| {
                    if error.is_not_found() {
                        conflict(path)
                    } else {
                        error
                    }
                })?;
                if !same_version(&baseline.stat, &current.stat)
                    || current.file.bytes != baseline.file.bytes
                {
                    return Err(conflict(path));
                }
            }
            None => require_missing(&directory, path.file_name(), path)?,
        }
        temporary.validate()?;
        let publication = if baseline.is_some() {
            // The advisory lock excludes cooperating writers, not an external
            // writer racing this final byte comparison and atomic replacement.
            fs::renameat(&directory, &temporary.name, &directory, path.file_name())
        } else {
            fs::renameat_with(
                &directory,
                &temporary.name,
                &directory,
                path.file_name(),
                RenameFlags::NOREPLACE,
            )
        };
        publication.map_err(|error| {
            if error == Errno::EXIST {
                conflict(path)
            } else {
                error.into()
            }
        })?;
        temporary.published = true;
        let durability_error = self.sync_directories(&directory, None);
        Ok(WriteReceipt {
            path: path.clone(),
            file,
            durability_error,
        })
    }

    pub fn create_file(
        &mut self,
        parent: &RelPath,
        title: &str,
        bytes: &[u8],
    ) -> Result<WriteReceipt, StoreError> {
        let stem = sanitized_name(title, true)?;
        let directory = self.open_directory(Some(parent))?;
        let (mut temporary, file) = self.prepare_file(&directory, bytes, None)?;
        for number in 1..=u64::MAX {
            let name = candidate_name(&stem, number, true);
            let path = parent.join(&name)?;
            self.inject(Fault::BeforePublish)?;
            self.check_directory(Some(parent), &directory)?;
            temporary.validate()?;
            match fs::renameat_with(
                &directory,
                &temporary.name,
                &directory,
                &name,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    temporary.published = true;
                    let durability_error = self.sync_directories(&directory, None);
                    return Ok(WriteReceipt {
                        path,
                        file,
                        durability_error,
                    });
                }
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(StoreError::InvalidName("collision suffix exhausted".into()))
    }

    /// Creates a uniquely named directory. Publication remains successful if
    /// directory fsync fails; the warning is available through `take_warnings`.
    pub fn create_dir(
        &mut self,
        parent: Option<&RelPath>,
        name: &str,
    ) -> Result<RelPath, StoreError> {
        let stem = sanitized_name(name, false)?;
        let directory = self.open_directory(parent)?;
        let mut temporary = Temporary::directory(&directory)?;
        fs::fsync(&temporary.file)?;
        for number in 1..=u64::MAX {
            let name = candidate_name(&stem, number, false);
            let path = child_path(parent, &name)?;
            self.inject(Fault::BeforePublish)?;
            self.check_directory(parent, &directory)?;
            temporary.validate()?;
            match fs::renameat_with(
                &directory,
                &temporary.name,
                &directory,
                &name,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    temporary.published = true;
                    if let Some(error) = self.sync_directories(&directory, None) {
                        self.queue_warning(format!(
                            "Created directory {}, but durability is pending: {error}",
                            path.as_path().display()
                        ));
                    }
                    return Ok(path);
                }
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(StoreError::InvalidName("collision suffix exhausted".into()))
    }

    /// Ensures an exact directory path exists, without suffixing occupied names.
    /// Existing components must all be real directories, never symlinks.
    pub fn ensure_dir(&mut self, path: &RelPath) -> Result<RelPath, StoreError> {
        let mut current = None;
        for component in path.as_path().components() {
            let Component::Normal(name) = component else {
                unreachable!("validated path")
            };
            let directory = self.open_directory(current.as_ref())?;
            self.check_directory(current.as_ref(), &directory)?;
            let created = match fs::mkdirat(&directory, name, Mode::from_raw_mode(0o777)) {
                Ok(()) => true,
                Err(Errno::EXIST) => false,
                Err(error) => return Err(error.into()),
            };
            let next = RelPath::new(match &current {
                Some(parent) => parent.as_path().join(name),
                None => PathBuf::from(name),
            })?;
            self.open_directory(Some(&next))?;
            if created {
                if let Some(error) = self.sync_directories(&directory, None) {
                    self.queue_warning(format!(
                        "Created directory {}, but durability is pending: {error}",
                        next.as_path().display()
                    ));
                }
            }
            current = Some(next);
        }
        Ok(path.clone())
    }

    pub fn relocate(
        &mut self,
        source: &RelPath,
        parent: Option<&RelPath>,
        new_name: &str,
        note: bool,
    ) -> Result<MoveReceipt, StoreError> {
        reject_lock_target(source)?;
        let stem = sanitized_name(new_name, note)?;
        if !note && parent.is_some_and(|parent| parent.as_path().starts_with(source.as_path())) {
            return Err(StoreError::InvalidPath(
                "cannot move a directory into itself".into(),
            ));
        }
        let old_parent = source.parent();
        let old_directory = self.open_directory(old_parent.as_ref())?;
        let new_directory = self.open_directory(parent)?;
        let source_file = open_leaf(&old_directory, source.file_name(), !note)?;
        let source_stat = fs::fstat(&source_file)?;
        require_type(
            &source_stat,
            if note {
                FileType::RegularFile
            } else {
                FileType::Directory
            },
            &source.as_path().display().to_string(),
        )?;
        for number in 1..=u64::MAX {
            let name = candidate_name(&stem, number, note);
            let path = child_path(parent, &name)?;
            self.check_directory(old_parent.as_ref(), &old_directory)?;
            self.check_directory(parent, &new_directory)?;
            let current = fs::statat(
                &old_directory,
                source.file_name(),
                AtFlags::SYMLINK_NOFOLLOW,
            )?;
            if !same_version(&source_stat, &current) {
                return Err(conflict(source));
            }
            if path == *source {
                return Ok(MoveReceipt {
                    path,
                    durability_error: None,
                });
            }
            self.inject(Fault::BeforePublish)?;
            match fs::renameat_with(
                &old_directory,
                source.file_name(),
                &new_directory,
                &name,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    let durability_error =
                        self.sync_directories(&old_directory, Some(&new_directory));
                    return Ok(MoveReceipt {
                        path,
                        durability_error,
                    });
                }
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(StoreError::InvalidName("collision suffix exhausted".into()))
    }

    /// Revalidates a file/directory immediately before a path-based trash API.
    /// This is not a capability: a hostile process can change the returned path
    /// after this call, so the caller must not treat it as race-free authorization.
    pub fn validate_target(&self, path: &RelPath) -> Result<PathBuf, StoreError> {
        reject_lock_target(path)?;
        let parent = path.parent();
        let directory = self.open_directory(parent.as_ref())?;
        let stat = fs::statat(&directory, path.file_name(), AtFlags::SYMLINK_NOFOLLOW)?;
        let kind = FileType::from_raw_mode(stat.st_mode);
        if !matches!(kind, FileType::RegularFile | FileType::Directory)
            || same_object(&stat, &fs::fstat(&self.reader.directory)?)
        {
            return Err(StoreError::UnsafePath(path.as_path().display().to_string()));
        }
        let file = open_leaf(&directory, path.file_name(), kind == FileType::Directory)?;
        if !same_object(&stat, &fs::fstat(&file)?) {
            return Err(StoreError::UnsafePath(path.as_path().display().to_string()));
        }
        self.check_directory(parent.as_ref(), &directory)?;
        let current = fs::statat(&directory, path.file_name(), AtFlags::SYMLINK_NOFOLLOW)?;
        if !same_object(&stat, &current) {
            return Err(StoreError::UnsafePath(path.as_path().display().to_string()));
        }
        Ok(self.reader.root.join(path.as_path()))
    }

    fn validate_root(&self) -> Result<(), StoreError> {
        self.reader.validate_root()
    }

    fn open_directory(&self, path: Option<&RelPath>) -> Result<File, StoreError> {
        self.reader.open_directory(path)
    }

    fn check_directory(&self, path: Option<&RelPath>, directory: &File) -> Result<(), StoreError> {
        self.reader.check_directory(path, directory)
    }

    fn prepare_file(
        &mut self,
        directory: &File,
        bytes: &[u8],
        mode: Option<u32>,
    ) -> Result<(Temporary, DiskFile), StoreError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(oversized_file(OsStr::new("draft")));
        }
        let mut temporary = Temporary::file(directory)?;
        self.inject(Fault::Write)?;
        temporary.file.write_all(bytes)?;
        self.inject(Fault::Permissions)?;
        let mode = mode.unwrap_or(temporary.file.metadata()?.mode() & 0o777);
        temporary
            .file
            .set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
        self.inject(Fault::FileSync)?;
        fs::fsync(&temporary.file)?;
        let metadata = temporary.file.metadata()?;
        let file = DiskFile {
            bytes: bytes.to_vec(),
            modified: metadata.modified()?,
            mode: metadata.mode() & 0o777,
        };
        Ok((temporary, file))
    }

    fn inject(&mut self, fault: Fault) -> Result<(), StoreError> {
        if self.fault == Some(fault) {
            self.fault = None;
            Err(StoreError::Injected(fault))
        } else {
            Ok(())
        }
    }

    fn sync_directory(&mut self, directory: &File) -> Result<(), StoreError> {
        self.inject(Fault::DirectorySync)?;
        fs::fsync(directory)?;
        Ok(())
    }

    fn retain_failed_directory(&mut self, directory: &File) -> Result<(), StoreError> {
        let metadata = directory.metadata()?;
        let device = metadata.dev();
        let inode = metadata.ino();
        if self
            .pending_directories
            .iter()
            .any(|pending| pending.device == device && pending.inode == inode)
        {
            return Ok(());
        }
        if self.pending_directories.len() == MAX_PENDING_DIRECTORIES {
            return Err(StoreError::Io(io::Error::other(format!(
                "pending directory durability descriptor limit ({MAX_PENDING_DIRECTORIES}) reached"
            ))));
        }
        self.pending_directories.push(PendingDirectory {
            file: directory.try_clone()?,
            device,
            inode,
        });
        Ok(())
    }

    fn sync_directories(&mut self, first: &File, second: Option<&File>) -> Option<String> {
        let mut errors = Vec::new();
        // Sync both parents, even if syncing the source parent failed.
        for directory in std::iter::once(first).chain(second) {
            if let Err(error) = self.sync_directory(directory) {
                errors.push(error.to_string());
                if let Err(error) = self.retain_failed_directory(directory) {
                    let error = format!("cannot retain unsynced directory for retry: {error}");
                    self.durability_tracking_error.get_or_insert(error.clone());
                    errors.push(error);
                }
            }
        }
        if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        }
    }

    fn queue_warning(&mut self, warning: String) {
        if self.warnings.contains(&warning) {
            return;
        }
        if self.warnings.len() == MAX_WARNINGS {
            self.warnings.remove(0);
        }
        self.warnings.push(warning);
    }
}

fn read_directory(directory: &File) -> Result<DirectoryListing, StoreError> {
    let entries = fs::Dir::read_from(directory)?;
    Ok(collect_directory_entries(
        directory,
        entries.map(|entry| {
            entry
                .map(|entry| OsStr::from_bytes(entry.file_name().to_bytes()).to_os_string())
                .map_err(StoreError::from)
        }),
    ))
}

fn collect_directory_entries(
    directory: &File,
    names: impl IntoIterator<Item = Result<OsString, StoreError>>,
) -> DirectoryListing {
    let mut listing = DirectoryListing {
        entries: Vec::new(),
        enumeration_errors: Vec::new(),
    };
    for name in names {
        match name {
            Ok(name) if name == "." || name == ".." => (),
            Ok(name) => {
                let kind = fs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
                    .map(|stat| match FileType::from_raw_mode(stat.st_mode) {
                        FileType::RegularFile => EntryKind::File,
                        FileType::Directory => EntryKind::Directory,
                        FileType::Symlink => EntryKind::Symlink,
                        _ => EntryKind::Other,
                    })
                    .map_err(StoreError::from);
                listing.entries.push(DirectoryEntry { name, kind });
            }
            Err(error) => listing.enumeration_errors.push(error),
        }
    }
    listing.entries.sort_by(|a, b| a.name.cmp(&b.name));
    listing
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

fn open_absolute_directory(path: &Path) -> Result<File, StoreError> {
    if !path.is_absolute() {
        return Err(StoreError::InvalidPath(path.display().to_string()));
    }
    let mut directory: File = fs::open("/", directory_flags(), Mode::empty())?.into();
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                directory = fs::openat(&directory, name, directory_flags(), Mode::empty())?.into();
            }
            _ => return Err(StoreError::InvalidPath(path.display().to_string())),
        }
    }
    Ok(directory)
}

fn open_leaf(directory: &File, name: &OsStr, is_directory: bool) -> Result<File, StoreError> {
    let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    if is_directory {
        flags |= OFlags::DIRECTORY;
    }
    Ok(fs::openat(directory, name, flags, Mode::empty())?.into())
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

fn require_type(stat: &Stat, kind: FileType, name: &str) -> Result<(), StoreError> {
    if FileType::from_raw_mode(stat.st_mode) == kind {
        Ok(())
    } else {
        Err(StoreError::UnsafePath(name.to_owned()))
    }
}

struct Snapshot {
    file: DiskFile,
    stat: Stat,
}

fn snapshot(directory: &File, name: &OsStr) -> Result<Snapshot, StoreError> {
    let mut file = open_leaf(directory, name, false)?;
    let before = fs::fstat(&file)?;
    require_type(&before, FileType::RegularFile, &name.to_string_lossy())?;
    if before.st_size > MAX_FILE_BYTES as i64 {
        return Err(oversized_file(name));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(oversized_file(name));
    }
    let metadata = file.metadata()?;
    let after = fs::fstat(&file)?;
    let named = fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if !same_version(&before, &after) || !same_version(&after, &named) {
        return Err(StoreError::Conflict(name.to_string_lossy().into_owned()));
    }
    Ok(Snapshot {
        file: DiskFile {
            bytes,
            modified: metadata.modified()?,
            mode: metadata.mode() & 0o777,
        },
        stat: after,
    })
}

fn oversized_file(name: &OsStr) -> StoreError {
    StoreError::Io(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "file exceeds supported size limit of {MAX_FILE_BYTES} bytes: {}",
            name.to_string_lossy()
        ),
    ))
}

fn conflict(path: &RelPath) -> StoreError {
    StoreError::Conflict(path.as_path().display().to_string())
}

fn reject_lock_target(path: &RelPath) -> Result<(), StoreError> {
    if path.as_path() == Path::new(LOCK_NAME) {
        Err(StoreError::UnsafePath(LOCK_NAME.into()))
    } else {
        Ok(())
    }
}

fn require_missing(directory: &File, name: &OsStr, path: &RelPath) -> Result<(), StoreError> {
    match fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(Errno::NOENT) => Ok(()),
        Ok(_) => Err(conflict(path)),
        Err(error) => Err(error.into()),
    }
}

fn child_path(parent: Option<&RelPath>, name: &str) -> Result<RelPath, StoreError> {
    match parent {
        Some(parent) => parent.join(name),
        None => RelPath::new(name),
    }
}

fn sanitized_name(name: &str, note: bool) -> Result<String, StoreError> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') {
        return Err(StoreError::InvalidName(name.to_owned()));
    }
    let stem: String = name
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                '-'
            } else {
                character
            }
        })
        .collect();
    let mut stem = stem.trim_end_matches([' ', '.']);
    if note {
        while stem
            .get(stem.len().saturating_sub(3)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".md"))
        {
            stem = stem[..stem.len() - 3].trim_end_matches([' ', '.']);
        }
    }
    if stem.is_empty() {
        return Err(StoreError::InvalidName(name.to_owned()));
    }
    Ok(stem.to_owned())
}

fn candidate_name(stem: &str, number: u64, note: bool) -> String {
    let suffix = if number == 1 {
        String::new()
    } else {
        format!(" {number}")
    };
    let extension = if note { ".md" } else { "" };
    let mut end = stem
        .len()
        .min(MAX_NAME_BYTES - suffix.len() - extension.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}{}", &stem[..end], suffix, extension)
}

struct Temporary {
    directory: File,
    name: String,
    file: File,
    is_directory: bool,
    published: bool,
}

impl Temporary {
    fn file(directory: &File) -> Result<Self, StoreError> {
        Self::new(directory, false)
    }

    fn directory(directory: &File) -> Result<Self, StoreError> {
        Self::new(directory, true)
    }

    fn new(directory: &File, is_directory: bool) -> Result<Self, StoreError> {
        let parent = directory.try_clone()?;
        for _ in 0..128 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".wren-tmp-{:x}-{sequence:x}", std::process::id());
            let result = if is_directory {
                match fs::mkdirat(directory, &name, Mode::from_raw_mode(0o777)) {
                    Ok(()) => (),
                    Err(Errno::EXIST) => continue,
                    Err(error) => return Err(error.into()),
                }
                let created = fs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW);
                let opened = fs::openat(directory, &name, directory_flags(), Mode::empty());
                if opened.is_err() {
                    if let (Ok(created), Ok(current)) = (
                        created,
                        fs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW),
                    ) {
                        if same_object(&created, &current) {
                            let _ = fs::unlinkat(directory, &name, AtFlags::REMOVEDIR);
                        }
                    }
                }
                opened
            } else {
                fs::openat(
                    directory,
                    &name,
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::NONBLOCK
                        | OFlags::CLOEXEC,
                    Mode::from_raw_mode(0o600),
                )
            };
            match result {
                Ok(file) => {
                    return Ok(Self {
                        directory: parent,
                        name,
                        file: file.into(),
                        is_directory,
                        published: false,
                    })
                }
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(StoreError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary-name attempts exhausted",
        )))
    }

    fn validate(&self) -> Result<(), StoreError> {
        let named = fs::statat(&self.directory, &self.name, AtFlags::SYMLINK_NOFOLLOW)?;
        if same_object(&named, &fs::fstat(&self.file)?) {
            Ok(())
        } else {
            Err(StoreError::UnsafePath(self.name.clone()))
        }
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.published {
            // Do not remove a name an external writer has replaced.
            if self.validate().is_ok() {
                let flags = if self.is_directory {
                    AtFlags::REMOVEDIR
                } else {
                    AtFlags::empty()
                };
                let _ = fs::unlinkat(&self.directory, &self.name, flags);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{symlink, FileTypeExt, PermissionsExt};
    use tempfile::TempDir;

    fn rel(path: &str) -> RelPath {
        RelPath::new(path).unwrap()
    }

    fn make_fifo(path: &Path) {
        #[cfg(not(target_os = "macos"))]
        rustix::fs::mkfifoat(rustix::fs::CWD, path, Mode::from_raw_mode(0o600)).unwrap();
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

    fn workspace() -> (TempDir, FsStore, RelPath) {
        let temp = TempDir::new().unwrap();
        let mut store = FsStore::open(temp.path()).unwrap();
        let section = store.create_dir(None, "Section").unwrap();
        (temp, store, section)
    }

    fn names(path: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn listed_kinds(listing: DirectoryListing) -> Vec<(OsString, EntryKind)> {
        assert!(listing.enumeration_errors.is_empty());
        listing
            .entries
            .into_iter()
            .map(|entry| (entry.name, entry.kind.unwrap()))
            .collect()
    }

    #[test]
    fn directory_listings_include_root_and_restart_each_enumeration() {
        let (_temp, mut store, section) = workspace();
        store.create_dir(Some(&section), "Group").unwrap();
        store.create_file(&section, "Note", b"note bytes").unwrap();
        for _ in 0..3 {
            assert_eq!(
                listed_kinds(store.list_dir(None).unwrap()),
                [
                    (LOCK_NAME.into(), EntryKind::File),
                    ("Section".into(), EntryKind::Directory),
                ]
            );
            assert_eq!(
                listed_kinds(store.list_dir(Some(&section)).unwrap()),
                [
                    ("Group".into(), EntryKind::Directory),
                    ("Note.md".into(), EntryKind::File),
                ]
            );
        }
        store.create_dir(None, "Another").unwrap();
        assert_eq!(
            listed_kinds(store.list_dir(None).unwrap()),
            [
                (LOCK_NAME.into(), EntryKind::File),
                ("Another".into(), EntryKind::Directory),
                ("Section".into(), EntryKind::Directory),
            ]
        );
    }

    #[test]
    fn directory_listings_do_not_follow_symlinks_and_identify_special_files() {
        let (temp, store, section) = workspace();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.md"), b"foreign").unwrap();
        fs::write(temp.path().join("Section/note.md"), b"note").unwrap();
        symlink(outside.path(), temp.path().join("Section/alias")).unwrap();
        symlink("missing", temp.path().join("Section/dangling")).unwrap();
        make_fifo(&temp.path().join("Section/pipe.md"));
        assert_eq!(
            listed_kinds(store.list_dir(Some(&section)).unwrap()),
            [
                ("alias".into(), EntryKind::Symlink),
                ("dangling".into(), EntryKind::Symlink),
                ("note.md".into(), EntryKind::File),
                ("pipe.md".into(), EntryKind::Other),
            ]
        );
        for path in [rel("Section/alias"), rel("Section/alias/child")] {
            assert!(matches!(
                store.list_dir(Some(&path)),
                Err(StoreError::UnsafePath(_))
            ));
        }
    }

    // APFS rejects invalid UTF-8 names. Keep physical byte-name fixtures on
    // Linux, independent of the portable symlink and special-file assertions.
    #[cfg(target_os = "linux")]
    #[test]
    fn directory_listings_retain_non_utf8_names() {
        let (temp, store, section) = workspace();
        let invalid = OsStr::from_bytes(b"\xff.md");
        fs::write(temp.path().join("Section").join(invalid), b"non-UTF-8").unwrap();
        assert_eq!(
            listed_kinds(store.list_dir(Some(&section)).unwrap()),
            [(invalid.to_os_string(), EntryKind::File)]
        );
    }

    #[test]
    fn directory_listings_retain_partial_enumeration_and_entry_metadata_errors() {
        let (_temp, mut store, section) = workspace();
        store.create_file(&section, "Note", b"note bytes").unwrap();
        let directory = store.open_directory(Some(&section)).unwrap();
        let listing = collect_directory_entries(
            &directory,
            [
                Ok(".".into()),
                Ok("Note.md".into()),
                Err(StoreError::Io(io::Error::other(
                    "injected enumeration error",
                ))),
                Ok("..".into()),
                Ok("missing.md".into()),
                Err(StoreError::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "another enumeration error",
                ))),
            ],
        );
        assert_eq!(listing.enumeration_errors.len(), 2);
        assert!(listing.enumeration_errors[0]
            .to_string()
            .contains("injected enumeration error"));
        assert!(matches!(
            &listing.enumeration_errors[1],
            StoreError::Io(error) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(listing.entries.len(), 2);
        assert_eq!(listing.entries[0].name, "Note.md");
        assert_eq!(*listing.entries[0].kind.as_ref().unwrap(), EntryKind::File);
        assert_eq!(listing.entries[1].name, "missing.md");
        assert!(listing.entries[1].kind.as_ref().unwrap_err().is_not_found());
    }

    #[test]
    fn directory_listings_stay_descriptor_relative_and_reject_replacement_during_enumeration() {
        for relative in [None, Some("Section"), Some("Section/Group")] {
            let temp = TempDir::new().unwrap();
            let root = temp.path().join("workspace");
            fs::create_dir_all(root.join("Section/Group")).unwrap();
            let store = FsStore::open(&root).unwrap();
            let relative = relative.map(rel);
            let target = relative
                .as_ref()
                .map_or_else(|| root.clone(), |path| root.join(path.as_path()));
            fs::write(target.join("trusted.md"), b"trusted").unwrap();
            let outside = TempDir::new().unwrap();
            fs::write(outside.path().join("foreign.md"), b"foreign").unwrap();
            for replace_before_read in [true, false] {
                let parked = temp.path().join("parked");
                let replace = || {
                    fs::rename(&target, &parked).unwrap();
                    symlink(outside.path(), &target).unwrap();
                };
                let result = store.list_dir_with(relative.as_ref(), |directory| {
                    if replace_before_read {
                        replace();
                    }
                    let listing = read_directory(directory)?;
                    assert!(listing.enumeration_errors.is_empty());
                    assert!(listing
                        .entries
                        .iter()
                        .any(|entry| entry.name == "trusted.md"));
                    assert!(!listing
                        .entries
                        .iter()
                        .any(|entry| entry.name == "foreign.md"));
                    if !replace_before_read {
                        replace();
                    }
                    Ok(listing)
                });
                assert!(result.is_err(), "accepted replaced directory: {relative:?}");
                assert!(store.list_dir(relative.as_ref()).is_err());
                fs::remove_file(&target).unwrap();
                fs::rename(&parked, &target).unwrap();
                assert!(store.list_dir(relative.as_ref()).is_ok());
            }
        }
    }

    #[test]
    fn directory_listings_reject_a_real_directory_replaced_after_opening() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("Section/Group")).unwrap();
        let store = FsStore::open(temp.path()).unwrap();
        let path = rel("Section/Group");
        let result = store.list_dir_with(Some(&path), |directory| {
            let listing = read_directory(directory)?;
            fs::rename(
                temp.path().join("Section/Group"),
                temp.path().join("original"),
            )
            .unwrap();
            fs::create_dir(temp.path().join("Section/Group")).unwrap();
            Ok(listing)
        });
        assert!(matches!(result, Err(StoreError::UnsafePath(_))));
    }

    #[test]
    fn relative_paths_reject_raw_traversal_and_dot_components() {
        for path in [
            "", "/", "/note", ".", "..", "./note", "a/./b", "a/../b", "a//b", "a/", "a\0b",
        ] {
            assert!(RelPath::new(path).is_err(), "accepted {path:?}");
        }
        let path = rel("Section/Group/note.md");
        assert_eq!(path.file_name(), "note.md");
        assert_eq!(path.parent(), Some(rel("Section/Group")));
        assert_eq!(rel("Section").parent(), None);
        assert!(rel("Section").join("../escape").is_err());
        assert!(rel("Section").join("/escape").is_err());
        assert!(RelPath::new(".wren/state.json").is_ok());
    }

    #[test]
    fn invalid_user_names_are_not_hidden_or_empty() {
        let (_temp, mut store, section) = workspace();
        for name in ["", "  ", ".", "..", ".hidden", ".md"] {
            assert!(store.create_file(&section, name, b"").is_err());
            assert!(store.create_dir(None, name).is_err());
        }
        let created = store
            .create_file(&section, " a/b\\c:*?\"<>|.md.md ", b"text")
            .unwrap();
        assert_eq!(created.path.file_name(), "a-b-c-------.md");
        assert_eq!(store.read(&created.path).unwrap().bytes, b"text");
    }

    #[test]
    fn note_extension_is_normalized_once_after_trimming() {
        let (_temp, mut store, section) = workspace();
        for (title, expected) in [
            ("Note.MD", "Note.md"),
            ("Note.md.", "Note 2.md"),
            ("Note.md .md", "Note 3.md"),
        ] {
            let created = store.create_file(&section, title, b"text").unwrap();
            assert_eq!(created.path.file_name(), expected);
        }
    }

    #[test]
    fn exact_bytes_not_length_control_conflicts() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"first").unwrap();
        fs::write(temp.path().join(note.path.as_path()), b"other").unwrap();
        let error = store
            .write(&note.path, b"new", Expected::Bytes(b"first"))
            .unwrap_err();
        assert!(error.is_conflict());
        assert_eq!(store.read(&note.path).unwrap().bytes, b"other");
        assert!(store
            .write(&note.path, b"new", Expected::Missing)
            .unwrap_err()
            .is_conflict());
        fs::remove_file(temp.path().join(note.path.as_path())).unwrap();
        assert!(store
            .write(&note.path, b"new", Expected::Bytes(b"other"))
            .unwrap_err()
            .is_conflict());
        assert!(store.read(&note.path).unwrap_err().is_not_found());
    }

    #[test]
    fn temporary_files_are_private_before_plaintext_is_written() {
        let (temp, store, section) = workspace();
        let directory = store.open_directory(Some(&section)).unwrap();
        let mut temporary = Temporary::file(&directory).unwrap();
        assert_eq!(temporary.file.metadata().unwrap().len(), 0);
        assert_eq!(temporary.file.metadata().unwrap().mode() & 0o777, 0o600);
        temporary.file.write_all(b"private plaintext").unwrap();
        assert_eq!(
            fs::metadata(temp.path().join("Section").join(&temporary.name))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
        drop(temporary);
        assert!(names(&temp.path().join("Section")).is_empty());
    }

    #[test]
    fn replacing_a_private_file_keeps_its_published_permissions() {
        let (temp, mut store, section) = workspace();
        let path = section.join("Private.md").unwrap();
        let absolute = temp.path().join(path.as_path());
        fs::write(&absolute, b"private baseline").unwrap();
        fs::set_permissions(&absolute, fs::Permissions::from_mode(0o600)).unwrap();
        let saved = store
            .write(
                &path,
                b"private replacement",
                Expected::Bytes(b"private baseline"),
            )
            .unwrap();
        assert_eq!(saved.file.mode, 0o600);
        assert_eq!(fs::metadata(&absolute).unwrap().mode() & 0o7777, 0o600);
        assert_eq!(store.read(&path).unwrap().bytes, b"private replacement");
    }

    #[test]
    fn writes_cannot_publish_a_document_too_large_to_read_back() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"baseline").unwrap();
        let oversized = vec![0; MAX_FILE_BYTES as usize + 1];
        store.fail_next(Fault::Write);
        let error = store
            .write(&note.path, &oversized, Expected::Bytes(b"baseline"))
            .unwrap_err();
        assert!(error.to_string().contains("size limit"));
        assert!(store
            .create_file(&section, "Large", &oversized)
            .unwrap_err()
            .to_string()
            .contains("size limit"));
        assert_eq!(store.read(&note.path).unwrap().bytes, b"baseline");
        assert!(matches!(
            store.write(&note.path, b"small", Expected::Bytes(b"baseline")),
            Err(StoreError::Injected(Fault::Write))
        ));
        assert_eq!(names(&temp.path().join("Section")), ["Note.md"]);
    }

    #[test]
    fn oversized_sparse_files_are_rejected_without_replacing_them() {
        let (temp, mut store, section) = workspace();
        let path = section.join("Large.md").unwrap();
        let absolute = temp.path().join(path.as_path());
        let mut file = File::create(&absolute).unwrap();
        file.write_all(b"baseline").unwrap();
        file.set_len(MAX_FILE_BYTES * 16).unwrap();
        let original = file.metadata().unwrap();
        let error = store.read(&path).unwrap_err();
        assert!(
            matches!(&error, StoreError::Io(error) if error.kind() == io::ErrorKind::InvalidData)
        );
        assert!(error.to_string().contains(&MAX_FILE_BYTES.to_string()));
        let error = store
            .write(&path, b"replacement", Expected::Bytes(b"baseline"))
            .unwrap_err();
        assert!(
            matches!(error, StoreError::Io(error) if error.kind() == io::ErrorKind::InvalidData)
        );
        assert!(store
            .write(&path, b"replacement", Expected::Missing)
            .unwrap_err()
            .is_conflict());
        let current = fs::metadata(&absolute).unwrap();
        assert_eq!(current.len(), original.len());
        assert_eq!(current.ino(), original.ino());
        let mut prefix = [0; 8];
        File::open(&absolute)
            .unwrap()
            .read_exact(&mut prefix)
            .unwrap();
        assert_eq!(&prefix, b"baseline");
        assert_eq!(names(&temp.path().join("Section")), ["Large.md"]);
    }

    #[test]
    fn writes_preserve_ordinary_permissions_but_not_special_bits() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"old").unwrap();
        fs::set_permissions(
            temp.path().join(note.path.as_path()),
            fs::Permissions::from_mode(0o6750),
        )
        .unwrap();
        let receipt = store
            .write(&note.path, b"new", Expected::Bytes(b"old"))
            .unwrap();
        assert_eq!(receipt.file.mode, 0o750);
        assert_eq!(
            fs::metadata(temp.path().join(note.path.as_path()))
                .unwrap()
                .mode()
                & 0o7777,
            0o750
        );
        assert_eq!(store.read(&note.path).unwrap().bytes, b"new");
        assert!(receipt.durability_error.is_none());
    }

    #[test]
    fn symlink_ancestors_and_leaves_cannot_be_read_or_written() {
        let (temp, mut store, section) = workspace();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.md"), b"secret").unwrap();
        symlink(outside.path(), temp.path().join("alias")).unwrap();
        symlink(
            outside.path().join("secret.md"),
            temp.path().join("Section/link.md"),
        )
        .unwrap();
        for path in [rel("alias/secret.md"), section.join("link.md").unwrap()] {
            assert!(store.read(&path).is_err());
            assert!(store
                .write(&path, b"damage", Expected::Bytes(b"secret"))
                .is_err());
            assert!(store.write(&path, b"damage", Expected::Missing).is_err());
            assert!(store.validate_target(&path).is_err());
        }
        assert!(store.create_file(&rel("alias"), "new", b"damage").is_err());
        assert!(store.create_dir(Some(&rel("alias")), "new").is_err());
        assert!(store.ensure_dir(&rel("alias/inner")).is_err());
        assert_eq!(
            fs::read(outside.path().join("secret.md")).unwrap(),
            b"secret"
        );
        assert!(!outside.path().join("new.md").exists());
    }

    #[test]
    fn fifo_leaves_are_rejected_without_blocking() {
        let (temp, mut store, section) = workspace();
        make_fifo(&temp.path().join("Section/pipe.md"));
        let path = section.join("pipe.md").unwrap();
        assert!(store.read(&path).is_err());
        assert!(store.write(&path, b"x", Expected::Bytes(b"")).is_err());
        assert!(store.validate_target(&path).is_err());
        assert!(fs::symlink_metadata(temp.path().join(path.as_path()))
            .unwrap()
            .file_type()
            .is_fifo());
    }

    #[test]
    fn collisions_skip_files_directories_symlinks_and_fifos() {
        let (temp, mut store, section) = workspace();
        fs::write(temp.path().join("Section/Note.md"), b"keep").unwrap();
        fs::create_dir(temp.path().join("Section/Note 2.md")).unwrap();
        symlink("missing", temp.path().join("Section/Note 3.md")).unwrap();
        make_fifo(&temp.path().join("Section/Note 4.md"));
        let note = store.create_file(&section, "Note.md", b"new").unwrap();
        assert_eq!(note.path, rel("Section/Note 5.md"));
        assert_eq!(
            fs::read(temp.path().join("Section/Note.md")).unwrap(),
            b"keep"
        );
        assert!(fs::symlink_metadata(temp.path().join("Section/Note 3.md"))
            .unwrap()
            .file_type()
            .is_symlink());
        fs::write(temp.path().join("Folder"), b"ignored non-markdown").unwrap();
        fs::create_dir(temp.path().join("Folder 2")).unwrap();
        symlink("missing", temp.path().join("Folder 3")).unwrap();
        assert_eq!(store.create_dir(None, "Folder").unwrap(), rel("Folder 4"));
        assert_eq!(
            fs::read(temp.path().join("Folder")).unwrap(),
            b"ignored non-markdown"
        );
    }

    #[test]
    fn unicode_names_are_bounded_including_collision_suffix() {
        let (_temp, mut store, section) = workspace();
        let title = "界é\u{1d400}".repeat(100);
        let first = store.create_file(&section, &title, b"one").unwrap();
        let second = store.create_file(&section, &title, b"two").unwrap();
        for receipt in [&first, &second] {
            let name = receipt.path.file_name().to_str().unwrap();
            assert!(name.len() <= MAX_NAME_BYTES);
            assert!(name.ends_with(".md"));
        }
        assert!(second.path.file_name().to_str().unwrap().ends_with(" 2.md"));
        let folder = store.create_dir(None, &title).unwrap();
        assert!(folder.file_name().as_bytes().len() <= MAX_NAME_BYTES);
        assert_ne!(first.path, second.path);
    }

    #[test]
    fn moving_is_same_inode_and_never_overwrites() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"move me").unwrap();
        let inode = fs::metadata(temp.path().join(note.path.as_path()))
            .unwrap()
            .ino();
        let destination = store.create_dir(None, "Destination").unwrap();
        fs::write(temp.path().join("Destination/Note.md"), b"keep").unwrap();
        fs::create_dir(temp.path().join("Destination/Note 2.md")).unwrap();
        symlink("missing", temp.path().join("Destination/Note 3.md")).unwrap();
        let moved = store
            .relocate(&note.path, Some(&destination), "Note.md", true)
            .unwrap();
        assert_eq!(moved.path, rel("Destination/Note 4.md"));
        assert_eq!(
            fs::metadata(temp.path().join(moved.path.as_path()))
                .unwrap()
                .ino(),
            inode
        );
        assert!(!temp.path().join(note.path.as_path()).exists());
        assert_eq!(store.read(&moved.path).unwrap().bytes, b"move me");
        assert_eq!(
            fs::read(temp.path().join("Destination/Note.md")).unwrap(),
            b"keep"
        );
        assert!(moved.durability_error.is_none());
    }

    #[test]
    fn directory_moves_use_noreplace_and_reject_descendants() {
        let (temp, mut store, section) = workspace();
        let group = store.create_dir(Some(&section), "Group").unwrap();
        assert!(store
            .relocate(&section, Some(&group), "bad", false)
            .is_err());
        fs::create_dir(temp.path().join("Taken")).unwrap();
        let moved = store.relocate(&section, None, "Taken", false).unwrap();
        assert_eq!(moved.path, rel("Taken 2"));
        assert!(temp.path().join("Taken 2/Group").is_dir());
        assert!(temp.path().join("Taken").is_dir());
    }

    #[test]
    fn same_path_move_is_noop() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"same").unwrap();
        let before = fs::metadata(temp.path().join(note.path.as_path()))
            .unwrap()
            .ino();
        let moved = store
            .relocate(&note.path, Some(&section), "Note", true)
            .unwrap();
        assert_eq!(moved.path, note.path);
        assert_eq!(
            fs::metadata(temp.path().join(moved.path.as_path()))
                .unwrap()
                .ino(),
            before
        );
        assert_eq!(names(&temp.path().join("Section")), ["Note.md"]);
    }

    #[test]
    fn prepublication_faults_leave_original_intact_and_no_temporary() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"old").unwrap();
        for fault in [
            Fault::Write,
            Fault::Permissions,
            Fault::FileSync,
            Fault::BeforePublish,
        ] {
            store.fail_next(fault);
            assert!(
                matches!(store.write(&note.path, b"new", Expected::Bytes(b"old")), Err(StoreError::Injected(value)) if value == fault)
            );
            assert_eq!(store.read(&note.path).unwrap().bytes, b"old");
            assert_eq!(names(&temp.path().join("Section")), ["Note.md"]);
            // Each fault is consumed once, not a permanent disabled-write mode.
            store
                .write(&note.path, b"old", Expected::Bytes(b"old"))
                .unwrap();
            store.fail_next(fault);
            assert!(store.create_file(&section, "Failed", b"new").is_err());
            assert_eq!(names(&temp.path().join("Section")), ["Note.md"]);
        }
    }

    #[test]
    fn postpublication_sync_failure_returns_committed_receipt() {
        let (_temp, mut store, section) = workspace();
        let path = section.join("Note.md").unwrap();
        store.fail_next(Fault::DirectorySync);
        let created = store.write(&path, b"new", Expected::Missing).unwrap();
        assert!(created.durability_error.is_some());
        assert!(store.pending_durability());
        assert_eq!(store.read(&created.path).unwrap().bytes, b"new");
        store.fail_next(Fault::DirectorySync);
        let saved = store
            .write(&path, b"saved", Expected::Bytes(b"new"))
            .unwrap();
        assert!(saved.durability_error.is_some());
        assert!(store.pending_durability());
        assert_eq!(store.read(&path).unwrap().bytes, b"saved");
        store.fail_next(Fault::DirectorySync);
        let another = store
            .create_file(&section, "Another", b"also published")
            .unwrap();
        assert!(another.durability_error.is_some());
        assert_eq!(store.read(&another.path).unwrap().bytes, b"also published");
        assert_eq!(store.pending_directories.len(), 1);
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
        let destination = store.create_dir(None, "Destination").unwrap();
        store.fail_next(Fault::DirectorySync);
        let moved = store
            .relocate(&path, Some(&destination), "Moved", true)
            .unwrap();
        assert!(moved.durability_error.is_some());
        assert!(store.pending_durability());
        assert_eq!(store.read(&moved.path).unwrap().bytes, b"saved");
        assert!(store.read(&path).unwrap_err().is_not_found());
        assert!(store
            .write(&moved.path, b"ok", Expected::Bytes(b"saved"))
            .unwrap()
            .durability_error
            .is_none());
        assert!(store.pending_durability());
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
    }

    #[test]
    fn created_directory_sync_failure_returns_path_and_retains_warning_and_retry() {
        let (temp, mut store, _section) = workspace();
        store.fail_next(Fault::DirectorySync);
        let path = store.create_dir(None, "Published").unwrap();
        assert_eq!(path, rel("Published"));
        assert!(temp.path().join(path.as_path()).is_dir());
        assert!(store.pending_durability());
        let warnings = store.take_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Published"));
        assert!(warnings[0].contains("DirectorySync"));
        assert!(store.take_warnings().is_empty());
        assert!(store.pending_durability());
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
    }

    #[test]
    fn ensured_directory_sync_failure_returns_path_and_retains_warning_and_retry() {
        let (temp, mut store, _section) = workspace();
        let path = rel(".wren/state");
        store.fail_next(Fault::DirectorySync);
        assert_eq!(store.ensure_dir(&path).unwrap(), path);
        assert!(temp.path().join(path.as_path()).is_dir());
        assert!(store.pending_durability());
        let warnings = store.take_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(".wren"));
        assert!(warnings[0].contains("DirectorySync"));
        assert_eq!(store.ensure_dir(&path).unwrap(), path);
        assert!(store.take_warnings().is_empty());
        assert!(store.pending_durability());
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
    }

    #[test]
    fn durability_retry_attempts_every_descriptor_and_retains_failed_ones() {
        let (temp, mut store, section) = workspace();
        store.fail_next(Fault::DirectorySync);
        store.create_file(&section, "Note", b"published").unwrap();
        store.fail_next(Fault::DirectorySync);
        store.create_dir(None, "Published").unwrap();
        assert_eq!(store.pending_directories.len(), 2);
        store.fail_next(Fault::DirectorySync);
        assert!(matches!(
            store.retry_durability(),
            Err(StoreError::Injected(Fault::DirectorySync))
        ));
        assert!(store.pending_durability());
        assert_eq!(store.pending_directories.len(), 1);
        let section_metadata = fs::metadata(temp.path().join(section.as_path())).unwrap();
        assert_eq!(store.pending_directories[0].device, section_metadata.dev());
        assert_eq!(store.pending_directories[0].inode, section_metadata.ino());
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
    }

    #[test]
    fn durability_retry_uses_retained_descriptors_after_directory_rename() {
        let (temp, mut store, section) = workspace();
        let outside = TempDir::new().unwrap();
        store.fail_next(Fault::DirectorySync);
        let created = store.create_file(&section, "Note", b"published").unwrap();
        assert!(created.durability_error.is_some());
        fs::rename(temp.path().join("Section"), temp.path().join("Renamed")).unwrap();
        symlink(outside.path(), temp.path().join("Section")).unwrap();
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
        assert_eq!(
            fs::read(temp.path().join("Renamed/Note.md")).unwrap(),
            b"published"
        );
        assert!(names(outside.path()).is_empty());
    }

    #[test]
    fn reads_and_failed_writes_do_not_clear_pending_durability_or_change_baseline() {
        let (_temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"old").unwrap();
        store.fail_next(Fault::DirectorySync);
        let published = store
            .write(&note.path, b"published", Expected::Bytes(b"old"))
            .unwrap();
        assert!(published.durability_error.is_some());
        assert_eq!(store.read(&note.path).unwrap().bytes, published.file.bytes);
        assert!(store.pending_durability());
        store.fail_next(Fault::Write);
        assert!(matches!(
            store.write(
                &note.path,
                b"unpublished",
                Expected::Bytes(&published.file.bytes)
            ),
            Err(StoreError::Injected(Fault::Write))
        ));
        assert_eq!(store.read(&note.path).unwrap().bytes, published.file.bytes);
        assert!(store.pending_durability());
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
        store
            .write(&note.path, b"saved", Expected::Bytes(&published.file.bytes))
            .unwrap();
        assert_eq!(store.read(&note.path).unwrap().bytes, b"saved");
    }

    #[test]
    fn directory_warning_queue_is_bounded_and_deduplicated() {
        let (_temp, mut store, _section) = workspace();
        for _ in 0..2 {
            store.queue_warning("same warning".into());
        }
        assert_eq!(store.take_warnings(), ["same warning"]);
        for number in 0..MAX_WARNINGS + 3 {
            store.fail_next(Fault::DirectorySync);
            store
                .create_dir(None, &format!("Published {number}"))
                .unwrap();
        }
        let warnings = store.take_warnings();
        assert_eq!(warnings.len(), MAX_WARNINGS);
        assert!(warnings[0].contains("Published 3,"));
        assert!(warnings
            .last()
            .unwrap()
            .contains(&format!("Published {},", MAX_WARNINGS + 2)));
        assert_eq!(store.pending_directories.len(), 1);
        assert!(store.pending_durability());
        assert!(store.take_warnings().is_empty());
        store.retry_durability().unwrap();
        assert!(!store.pending_durability());
    }

    #[test]
    fn descriptor_retention_limit_fails_closed_even_after_successful_retries() {
        let (temp, mut store, _section) = workspace();
        let mut last_error = None;
        for number in 0..=MAX_PENDING_DIRECTORIES {
            let path = rel(&format!("Directory {number}"));
            fs::create_dir(temp.path().join(path.as_path())).unwrap();
            store.fail_next(Fault::DirectorySync);
            let published = store.create_file(&path, "Note", b"published").unwrap();
            assert_eq!(store.read(&published.path).unwrap().bytes, b"published");
            last_error = published.durability_error;
            assert!(last_error.is_some());
        }
        assert!(last_error.unwrap().contains("descriptor limit"));
        assert_eq!(store.pending_directories.len(), MAX_PENDING_DIRECTORIES);
        assert!(store.pending_durability());
        assert!(store
            .retry_durability()
            .unwrap_err()
            .to_string()
            .contains("cannot retain"));
        assert!(store.pending_directories.is_empty());
        assert!(store.pending_durability());
        assert!(store.retry_durability().is_err());
        assert!(store.pending_durability());
    }

    #[test]
    fn exact_metadata_directory_is_not_suffixed_and_rejects_symlink() {
        let (temp, mut store, _section) = workspace();
        assert_eq!(
            store.ensure_dir(&rel(".wren/state")).unwrap(),
            rel(".wren/state")
        );
        assert_eq!(
            store.ensure_dir(&rel(".wren/state")).unwrap(),
            rel(".wren/state")
        );
        fs::write(temp.path().join("occupied"), b"file").unwrap();
        assert!(store.ensure_dir(&rel("occupied")).is_err());
        symlink(".wren", temp.path().join("alias")).unwrap();
        assert!(store.ensure_dir(&rel("alias")).is_err());
        assert!(!temp.path().join("occupied 2").exists());
    }

    #[test]
    fn duplicate_lock_fails_and_lock_name_survives_drop() {
        let temp = TempDir::new().unwrap();
        let store = FsStore::open(temp.path()).unwrap();
        let inode = fs::metadata(temp.path().join(LOCK_NAME)).unwrap().ino();
        assert!(matches!(
            FsStore::open(temp.path()),
            Err(StoreError::Locked)
        ));
        drop(store);
        assert_eq!(
            fs::metadata(temp.path().join(LOCK_NAME)).unwrap().ino(),
            inode
        );
        let reopened = FsStore::open(temp.path()).unwrap();
        assert_eq!(
            fs::metadata(reopened.root().join(LOCK_NAME)).unwrap().ino(),
            inode
        );
    }

    #[test]
    fn duplicated_readers_revalidate_the_root_and_never_hold_the_lock() {
        let (temp, mut store, section) = workspace();
        store.create_file(&section, "Note", b"note bytes").unwrap();
        let reader = store.reader().try_clone().unwrap();
        let note = rel("Section/Note.md");
        let bytes = std::thread::spawn(move || reader.read(&note).unwrap().bytes)
            .join()
            .unwrap();
        assert_eq!(bytes, b"note bytes");

        let reader = store.reader().try_clone().unwrap();
        drop(store);
        let reopened = FsStore::open(temp.path()).unwrap();
        assert_eq!(reader.list_dir(Some(&section)).unwrap().entries.len(), 1);
        drop(reopened);

        let parked = TempDir::new().unwrap();
        let moved = parked.path().join("moved");
        fs::rename(temp.path(), &moved).unwrap();
        fs::create_dir(temp.path()).unwrap();
        assert!(matches!(
            reader.list_dir(None),
            Err(StoreError::UnsafePath(_))
        ));
        fs::remove_dir(temp.path()).unwrap();
        fs::rename(&moved, temp.path()).unwrap();
    }

    #[test]
    fn store_cannot_replace_move_or_authorize_trashing_its_lock() {
        let (temp, mut store, section) = workspace();
        let path = rel(LOCK_NAME);
        let inode = fs::metadata(temp.path().join(LOCK_NAME)).unwrap().ino();
        assert!(store
            .write(&path, b"replacement", Expected::Bytes(b""))
            .is_err());
        assert!(store.relocate(&path, Some(&section), "Lock", true).is_err());
        assert!(store.validate_target(&path).is_err());
        assert_eq!(
            fs::metadata(temp.path().join(LOCK_NAME)).unwrap().ino(),
            inode
        );
        assert!(matches!(
            FsStore::open(temp.path()),
            Err(StoreError::Locked)
        ));
    }

    #[test]
    fn directory_and_move_failures_do_not_publish_or_leave_staging_entries() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"keep").unwrap();
        let root_names = names(temp.path());
        store.fail_next(Fault::BeforePublish);
        assert!(store.create_dir(None, "Failed").is_err());
        assert_eq!(names(temp.path()), root_names);
        store.fail_next(Fault::BeforePublish);
        assert!(store
            .relocate(&note.path, Some(&section), "Changed", true)
            .is_err());
        assert_eq!(names(&temp.path().join("Section")), ["Note.md"]);
        assert_eq!(store.read(&note.path).unwrap().bytes, b"keep");
        assert!(store
            .relocate(&note.path, Some(&section), "Changed", true)
            .is_ok());
        store.fail_next(Fault::DirectorySync);
        let directory = store.create_dir(None, "Published").unwrap();
        assert!(store.validate_target(&directory).unwrap().is_dir());
    }

    #[test]
    fn moves_reject_symlink_sources_and_destination_ancestors() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"keep").unwrap();
        let outside = TempDir::new().unwrap();
        symlink(outside.path(), temp.path().join("alias")).unwrap();
        symlink("Note.md", temp.path().join("Section/link.md")).unwrap();
        assert!(store
            .relocate(&rel("Section/link.md"), Some(&section), "Moved", true)
            .is_err());
        assert!(store
            .relocate(&note.path, Some(&rel("alias")), "Moved", true)
            .is_err());
        assert!(store
            .relocate(&section, Some(&rel("alias")), "Moved", false)
            .is_err());
        assert_eq!(store.read(&note.path).unwrap().bytes, b"keep");
        assert!(names(outside.path()).is_empty());
    }

    #[test]
    fn exact_metadata_paths_support_atomic_writes() {
        let (_temp, mut store, _section) = workspace();
        store.ensure_dir(&rel(".wren")).unwrap();
        let path = rel(".wren/state.json");
        let created = store.write(&path, b"one", Expected::Missing).unwrap();
        assert_eq!(created.path, path);
        let saved = store.write(&path, b"two", Expected::Bytes(b"one")).unwrap();
        assert_eq!(saved.file.bytes, b"two");
        assert_eq!(store.read(&path).unwrap().bytes, b"two");
        assert!(store.root().is_absolute());
        assert_eq!(store.root().canonicalize().unwrap(), store.root());
    }

    #[test]
    fn equal_bytes_in_a_replacement_inode_are_not_the_same_baseline() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"same").unwrap();
        let directory = store.open_directory(Some(&section)).unwrap();
        let original = snapshot(&directory, note.path.file_name()).unwrap();
        fs::write(temp.path().join("replacement"), b"same").unwrap();
        fs::rename(
            temp.path().join("replacement"),
            temp.path().join(note.path.as_path()),
        )
        .unwrap();
        let replacement = snapshot(&directory, note.path.file_name()).unwrap();
        assert_eq!(original.file.bytes, replacement.file.bytes);
        assert!(!same_version(&original.stat, &replacement.stat));
    }

    #[test]
    fn unsafe_lock_files_are_rejected() {
        for directory in [false, true] {
            let temp = TempDir::new().unwrap();
            if directory {
                fs::create_dir(temp.path().join(LOCK_NAME)).unwrap();
            } else {
                symlink("missing", temp.path().join(LOCK_NAME)).unwrap();
            }
            assert!(FsStore::open(temp.path()).is_err());
        }
        let temp = TempDir::new().unwrap();
        make_fifo(&temp.path().join(LOCK_NAME));
        assert!(FsStore::open(temp.path()).is_err());
    }

    #[test]
    fn renamed_root_and_replaced_lock_are_detected() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let mut store = FsStore::open(&root).unwrap();
        store.fail_next(Fault::DirectorySync);
        store.create_dir(None, "Pending").unwrap();
        fs::rename(&root, temp.path().join("old")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(store
            .write(&rel("note.md"), b"x", Expected::Missing)
            .is_err());
        assert!(store.retry_durability().is_err());
        assert!(store.pending_durability());
        drop(store);
        let mut store = FsStore::open(&root).unwrap();
        store.fail_next(Fault::DirectorySync);
        store.create_dir(None, "Pending").unwrap();
        fs::rename(root.join(LOCK_NAME), root.join("old-lock")).unwrap();
        fs::write(root.join(LOCK_NAME), b"").unwrap();
        assert!(store.create_dir(None, "Unsafe").is_err());
        assert!(store.retry_durability().is_err());
        assert!(store.pending_durability());
    }

    #[test]
    fn target_validation_accepts_only_current_real_files_and_directories() {
        let (temp, mut store, section) = workspace();
        let note = store.create_file(&section, "Note", b"ok").unwrap();
        assert_eq!(
            store.validate_target(&section).unwrap(),
            store.root().join("Section")
        );
        assert_eq!(
            store.validate_target(&note.path).unwrap(),
            store.root().join("Section/Note.md")
        );
        symlink(temp.path(), temp.path().join("root-alias")).unwrap();
        assert!(store.validate_target(&rel("root-alias")).is_err());
        assert!(FsStore::open(&temp.path().join("root-alias")).is_err());
        assert!(RelPath::new("").is_err());
    }
}
