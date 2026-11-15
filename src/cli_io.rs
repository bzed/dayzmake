//! stdin/stdout and file plumbing for the commands: an input that is a file or buffered stdin, an
//! output that is a file or stdout, and the creation of target files.

use std::fs::File;
use std::io;
use std::io::{Cursor, Read, Seek, Stdout, Write};
use std::path::Path;

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
    /// A file on disk.
    File(File),
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

/// Creates the target file of a command; an existing file is an error unless `force`.
///
/// # Errors
///
/// Returns an error if the file exists without `force` or cannot be created.
pub fn create_target(path: &std::path::Path, force: bool) -> io::Result<File> {
    if !force && path.exists() {
        return Err(error!(
            "Target file \"{}\" already exists. Use --force to overwrite.",
            path.display()
        ));
    }
    File::create(path).prepend_error("Failed to open output file:")
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
