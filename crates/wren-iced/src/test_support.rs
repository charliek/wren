//! Shared helpers for unit tests.

/// A temporary directory whose path is already canonical.
///
/// macOS's temp root lives under the `/var` → `/private/var` symlink, while
/// workspace roots are canonicalized when opened. Creating fixtures under the
/// canonical root keeps path assertions identical on every platform.
pub fn tempdir() -> tempfile::TempDir {
    let root = std::env::temp_dir()
        .canonicalize()
        .expect("the temporary directory must exist");
    tempfile::tempdir_in(root).expect("create a temporary directory")
}
