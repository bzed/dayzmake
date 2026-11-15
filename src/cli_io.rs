//! stdin/stdout and file plumbing for the commands: an input that is a file or buffered stdin, an
//! output that is a file or stdout, and the creation of target files.

use std::fs::File;
use std::io;
use std::io::{Cursor, Read, Seek, Stdout, Write};
use std::path::{Path, PathBuf};

use dayzmake_core::error;
use dayzmake_core::error::ErrorExt;

/// Input source: a file, or an in-memory buffer (stdin).
pub enum Input {
    /// A file on disk.
    File(File),
    /// An in-memory buffer.
    Cursor(Cursor<Box<[u8]>>),
}

/// Output sink: a file, or stdout.
pub enum Output {
    /// A target file, written atomically (see [`AtomicTarget`]).
    File(AtomicTarget),
    /// Standard output.
    Standard(Stdout),
}

impl Read for Input {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            Input::File(ref mut f) => f.read(buf),
            Input::Cursor(ref mut c) => c.read(buf),
        }
    }
}

impl Seek for Input {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match *self {
            Input::File(ref mut f) => f.seek(pos),
            Input::Cursor(ref mut c) => c.seek(pos),
        }
    }
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Output::File(ref mut f) => f.write(buf),
            Output::Standard(ref mut s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            Output::File(ref mut f) => f.flush(),
            Output::Standard(ref mut s) => s.flush(),
        }
    }
}

impl Output {
    /// Makes a file output the real target file; stdout needs nothing. Call it only after the command succeeded:
    /// an output dropped without it leaves the target as it was.
    ///
    /// # Errors
    ///
    /// Returns an error if the target cannot be flushed or renamed into place.
    pub fn commit(self) -> io::Result<()> {
        match self {
            Output::File(f) => f.commit(),
            Output::Standard(_) => Ok(()),
        }
    }
}

/// A target file written atomically: data goes to a temp file next to `path`, which
/// [`commit`](AtomicTarget::commit) renames over `path`; dropped without `commit`, the temp file is removed.
///
/// `std::fs::rename` replaces an existing file on every platform (on Windows it uses `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING`).
pub struct AtomicTarget {
    path: PathBuf,
    temp: PathBuf,
    file: Option<File>,
    done: bool,
}

impl AtomicTarget {
    /// Prepares the target: an existing file is an error unless `force`; nothing at `path` is touched.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists without `force` or the temp file cannot be created.
    pub fn create(path: &Path, force: bool) -> io::Result<AtomicTarget> {
        if !force && path.exists() {
            return Err(error!(
                "Target file \"{}\" already exists. Use --force to overwrite.",
                path.display()
            ));
        }
        let name = path
            .file_name()
            .ok_or_else(|| error!("Invalid output file \"{}\"", path.display()))?
            .to_string_lossy();
        let pid = std::process::id();
        for n in 0_u32..1000 {
            let temp = path.with_file_name(format!(".{name}.dayzmake-tmp-{pid}-{n}"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
            {
                Ok(file) => {
                    return Ok(AtomicTarget {
                        path: path.to_path_buf(),
                        temp,
                        file: Some(file),
                        done: false,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e).prepend_error("Failed to open output file:"),
            }
        }
        Err(error!(
            "Failed to open output file: no free temp file name next to \"{}\"",
            path.display()
        ))
    }

    /// Flushes the data to disk and renames the temp file over the target.
    ///
    /// # Errors
    ///
    /// Returns an error if syncing or renaming fails; the temp file is removed then.
    pub fn commit(mut self) -> io::Result<()> {
        if let Some(file) = self.file.take() {
            file.sync_all()
                .prepend_error("Failed to write output file:")?;
            // closed before the rename, which Windows needs
            drop(file);
            std::fs::rename(&self.temp, &self.path)
                .prepend_error("Failed to replace output file:")?;
            self.done = true;
        }
        Ok(())
    }
}

impl Write for AtomicTarget {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.as_mut().map_or(Ok(0), |f| f.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), Write::flush)
    }
}

impl Drop for AtomicTarget {
    fn drop(&mut self) {
        // without a successful commit the temp file goes
        if !self.done {
            // a failed cleanup leaves a hidden temp file; nothing more can be done in a destructor
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

/// The first line of an open error: `Failed to open <what> "<path>":`, the path made printable.
#[must_use]
pub fn open_failure(path: &Path, what: &str) -> String {
    format!(
        "Failed to open {what} \"{}\":",
        dayzmake_core::error::printable(&path.display().to_string())
    )
}

/// Opens a user-given input file; the error names the file: `Failed to open <what> "<path>":\n<os error>`.
///
/// # Errors
///
/// Returns an error if the file cannot be opened.
pub fn open_input(path: &Path, what: &str) -> io::Result<File> {
    File::open(path).prepend_error(open_failure(path, what))
}
