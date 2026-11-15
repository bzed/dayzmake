//! File identity: whether two paths name the same file, whatever their spelling.

use std::path::Path;

/// The identity of an existing file: device and inode on unix, volume serial number and file index on Windows.
///
/// Two ids are equal when the paths they were taken from name the same file: through a hard link, a symlink, a
/// different case on a case-insensitive file system, or a relative and an absolute spelling.
#[derive(Debug, PartialEq, Eq)]
pub struct FileId(same_file::Handle);

impl FileId {
    /// The identity of the file at `path` (symlinks are followed), or `None` when it cannot be opened (missing,
    /// removed meanwhile, no permission) or is neither a regular file nor a folder (a FIFO or device is never opened). A file without an identity is never the same as another one.
    #[must_use]
    pub fn of(path: &Path) -> Option<FileId> {
        // opening a FIFO or a device can block, so only regular files and folders are opened
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() && !meta.is_dir() {
            return None;
        }
        // an unopenable file is "not the same file" by contract
        same_file::Handle::from_path(path).ok().map(FileId)
    }
}

/// Whether `a` and `b` name the same existing file. `false` when either cannot be opened.
#[must_use]
pub fn is_same_file(a: &Path, b: &Path) -> bool {
    match (FileId::of(a), FileId::of(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_and_different_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "x").unwrap();
        assert!(is_same_file(&a, &a));
        assert!(is_same_file(&a, &dir.path().join(".").join("a")));
        assert!(!is_same_file(&a, &b));
    }

    #[test]
    fn a_missing_file_is_never_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let gone = dir.path().join("gone");
        std::fs::write(&a, "x").unwrap();
        assert!(FileId::of(&gone).is_none());
        assert!(!is_same_file(&a, &gone));
        assert!(!is_same_file(&gone, &gone));
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_and_symlinks_are_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::write(&a, "x").unwrap();
        std::fs::hard_link(&a, dir.path().join("h")).unwrap();
        std::os::unix::fs::symlink("a", dir.path().join("s")).unwrap();
        assert!(is_same_file(&a, &dir.path().join("h")));
        assert!(is_same_file(&a, &dir.path().join("s")));
    }
}
