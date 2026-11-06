//! Test-only quarantine selected by the already gated --test-socket option.

use std::fs::DirBuilder;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use rustix::fs::{renameat_with, RenameFlags, CWD};
use rustix::io::Errno;
use wren_core::ops::Trash;

pub struct Quarantine {
    path: PathBuf,
    next: u64,
}

impl Quarantine {
    pub fn new(sandbox: &Path) -> Result<Self, String> {
        private_directory(sandbox)?;
        let path = sandbox.join("trash");
        match DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("Create test trash quarantine: {error}")),
        }
        private_directory(&path)?;
        Ok(Self { path, next: 0 })
    }
}

impl Trash for Quarantine {
    fn trash(&mut self, path: &Path) -> Result<(), String> {
        if !path.is_absolute() {
            return Err("Test trash requires an absolute source path".into());
        }
        private_directory(&self.path)?;
        for _ in 0..1024 {
            let destination = self.path.join(format!("item-{:08}", self.next));
            self.next = self.next.wrapping_add(1);
            match renameat_with(CWD, path, CWD, &destination, RenameFlags::NOREPLACE) {
                Ok(()) => return Ok(()),
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(format!("Move to test trash quarantine: {error}")),
            }
        }
        Err("Test trash quarantine has too many destination collisions".into())
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Test trash sandbox must be absolute".into());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.mode() & 0o777 != 0o700
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err("Test trash must be an owned, non-symlink 0700 directory".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{symlink, PermissionsExt};

    use super::*;

    #[test]
    fn quarantine_moves_files_and_directories_without_overwriting_collisions() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut trash = Quarantine::new(sandbox.path()).unwrap();
        assert_eq!(
            std::fs::metadata(&trash.path).unwrap().mode() & 0o777,
            0o700
        );
        std::fs::write(trash.path.join("item-00000000"), "sentinel").unwrap();
        let note = sandbox.path().join("note.md");
        std::fs::write(&note, "draft").unwrap();
        trash.trash(&note).unwrap();
        assert!(!note.exists());
        assert_eq!(
            std::fs::read_to_string(trash.path.join("item-00000000")).unwrap(),
            "sentinel"
        );
        assert_eq!(
            std::fs::read_to_string(trash.path.join("item-00000001")).unwrap(),
            "draft"
        );
        let group = sandbox.path().join("group");
        std::fs::create_dir(&group).unwrap();
        std::fs::write(group.join("nested.md"), "nested").unwrap();
        trash.trash(&group).unwrap();
        assert_eq!(
            std::fs::read_to_string(trash.path.join("item-00000002/nested.md")).unwrap(),
            "nested"
        );
    }

    #[test]
    fn quarantine_rejects_symlinks_and_public_directories() {
        let sandbox = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        symlink(target.path(), sandbox.path().join("trash")).unwrap();
        assert!(Quarantine::new(sandbox.path()).is_err());
        std::fs::remove_file(sandbox.path().join("trash")).unwrap();
        std::fs::set_permissions(sandbox.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Quarantine::new(sandbox.path()).is_err());
    }

    #[test]
    fn quarantine_failure_never_removes_the_source() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut trash = Quarantine::new(sandbox.path()).unwrap();
        let note = sandbox.path().join("note.md");
        std::fs::write(&note, "draft").unwrap();
        std::fs::set_permissions(&trash.path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(trash.trash(&note).is_err());
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "draft");
        assert!(trash.trash(Path::new("relative")).is_err());
    }
}
