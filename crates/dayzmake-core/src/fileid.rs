//! File identity: whether two paths name the same file, whatever their spelling.

use std::path::Path;

/// The identity of an existing file: device and inode on unix, volume serial number and file index on Windows.
///
/// Two ids are equal when the paths they were taken from name the same file: through a hard link, a symlink, a
/// different case on a case-insensitive file system, or a relative and an absolute spelling.
///
/// A `FileId` holds an open handle to the file. On Windows the identity is valid only while that handle is open,
/// so keep the `FileId` alive for as long as it is compared. Creating many ids at once holds many open files.
///
/// `same-file` compares a 32-bit volume serial number and a 64-bit file index, but `ReFS` (Dev Drive) file ids are
/// 128 bits, so two different files could compare equal there. Two ids therefore also have to agree on the file
/// length to be equal; a pair of different files of the same length on `ReFS` is the one case left undetected.
///
/// The length is read when two ids are compared, not when they are created (a growing temp file stays equal to
/// itself). Two ids compare equal only if both lengths can be read and match, so an id is not necessarily equal to
/// itself and the type is `PartialEq` only.
#[derive(Debug)]
pub struct FileId(same_file::Handle);

impl FileId {
    fn len(&self) -> Option<u64> {
        self.0.as_file().metadata().ok().map(|m| m.len())
    }
}

impl PartialEq for FileId {
    fn eq(&self, other: &FileId) -> bool {
        // the length is read now, not at creation: the same file always has one length at one moment
        self.0 == other.0 && self.len().is_some() && self.len() == other.len()
    }
}

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

impl FileId {
    /// The identity of the file standard output is redirected to, or `None` unless that is a regular file: a
    /// terminal, a pipe, a device, a closed stdout or a failed duplicate all give `None`.
    ///
    /// The identity is taken from a duplicate of the stdout descriptor (handle on Windows), owned by the returned
    /// id, so dropping it never closes the process's real stdout. On Windows the "regular file" check relies on
    /// the metadata of a console or pipe handle failing, which gives `None`.
    #[must_use]
    pub fn stdout() -> Option<FileId> {
        #[cfg(unix)]
        let owned = {
            use std::os::fd::AsFd;
            std::io::stdout().as_fd().try_clone_to_owned().ok()?
        };
        #[cfg(windows)]
        let owned = {
            use std::os::windows::io::AsHandle;
            std::io::stdout().as_handle().try_clone_to_owned().ok()?
        };
        let file = std::fs::File::from(owned);
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        same_file::Handle::from_file(file).ok().map(FileId)
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

    #[test]
    fn different_files_of_different_length_never_match() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "xyz").unwrap();
        assert!(!is_same_file(&a, &b));
        // the same file stays the same while it grows
        let before = FileId::of(&a).unwrap();
        std::fs::write(&a, "longer").unwrap();
        assert_eq!(before, FileId::of(&a).unwrap());
    }

    #[test]
    fn stdout_identity_leaves_stdout_open() {
        // under the test harness stdout is a pipe or a file; either way the lookup must not close it
        let _ = FileId::stdout();
        let _ = FileId::stdout();
        #[cfg(unix)]
        {
            use std::os::fd::AsFd;
            // fd 1 is still open: it can be duplicated again
            assert!(std::io::stdout().as_fd().try_clone_to_owned().is_ok());
        }
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
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
