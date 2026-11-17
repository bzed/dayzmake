//! The index cache: one JSON file per index, rebuilt automatically when an input file changes.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{Error, ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::model::{ApiIndex, INDEX_FORMAT, InputStamp};
use dayzmake_core::error;
use dayzmake_core::warning::{Warning, WarningSink};

/// The cache directory, created (mode 0700 on unix) when missing: `$DAYZMAKE_CACHE`, else
/// `$XDG_CACHE_HOME/dayzmake/api`, else `$HOME/.cache/dayzmake/api` (Windows: `%LOCALAPPDATA%\dayzmake\api`).
///
/// # Errors
///
/// Returns an error when no location can be determined or the directory cannot be created.
pub fn cache_dir() -> Result<PathBuf, Error> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let dir = if let Some(d) = var("DAYZMAKE_CACHE") {
        PathBuf::from(d)
    } else if cfg!(windows) {
        var("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("dayzmake").join("api"))
            .ok_or_else(|| error!("cannot find a cache directory: set DAYZMAKE_CACHE"))?
    } else if let Some(x) = var("XDG_CACHE_HOME") {
        PathBuf::from(x).join("dayzmake/api")
    } else if let Some(h) = var("HOME") {
        PathBuf::from(h).join(".cache/dayzmake/api")
    } else {
        return Err(error!("cannot find a cache directory: set DAYZMAKE_CACHE"));
    };
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&dir)
        .map_err(|e| error!("cannot create the cache directory {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Size and modification time of each input.
///
/// # Errors
///
/// Returns an error when an input cannot be read.
pub fn stamps(inputs: &[PathBuf]) -> Result<Vec<InputStamp>, Error> {
    inputs
        .iter()
        .map(|path| {
            let meta = fs::metadata(path).map_err(|e| error!("{}: {e}", path.display()))?;
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            Ok(InputStamp {
                path: path.clone(),
                size: meta.len(),
                mtime_ns,
            })
        })
        .collect()
}

/// An index the cache can store: it knows its format and inputs.
pub trait Cacheable: Serialize + DeserializeOwned {
    /// Whether this is of the current format and dayzmake version and was made from exactly `want`.
    fn is_current(&self, want: &[InputStamp]) -> bool;
    /// Records the inputs the index was built from.
    fn set_inputs(&mut self, inputs: Vec<InputStamp>);
}

impl Cacheable for ApiIndex {
    fn is_current(&self, want: &[InputStamp]) -> bool {
        self.format == INDEX_FORMAT
            && self.version == env!("CARGO_PKG_VERSION")
            && self.inputs == want
    }

    fn set_inputs(&mut self, inputs: Vec<InputStamp>) {
        self.inputs = inputs;
    }
}

fn load<T: Cacheable>(file: &Path, want: &[InputStamp]) -> Option<T> {
    let bytes = fs::read(file).ok()?;
    let index: T = serde_json::from_slice(&bytes).ok()?;
    index.is_current(want).then_some(index)
}

/// Write `index` through an exclusively created temp file in the same directory and a rename.
fn store<T: Serialize>(dir: &Path, key: &str, index: &T) -> Result<(), Error> {
    let json = serde_json::to_vec(index).map_err(|e| error!("cannot serialize the index: {e}"))?;
    let mut attempts = 0;
    let (tmp, mut file) = loop {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let tmp = dir.join(format!(".{key}.{}.{nanos}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == ErrorKind::AlreadyExists && attempts < 5 => attempts += 1,
            Err(e) => return Err(error!("{}: {e}", tmp.display())),
        }
    };
    // Whatever fails from here on, do not leave the temp file behind. Its removal error adds nothing.
    let result = file
        .write_all(&json)
        .and_then(|()| file.sync_all())
        .map_err(|e| error!("{}: {e}", tmp.display()))
        .and_then(|()| {
            fs::rename(&tmp, dir.join(format!("{key}.json")))
                .map_err(|e| error!("cannot store the index {key}: {e}"))
        });
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn cache_warning(warnings: &mut dyn WarningSink, msg: &str) {
    warnings.warn(Warning::new(Some("api-cache"), msg));
}

/// The cached index `key`, or the result of `build` when there is none, it does not parse, has another
/// format, or was made from other inputs (size or mtime). `refresh` always builds. A corrupt cache file is
/// replaced, never an error.
///
/// # Errors
///
/// Returns an error when `key` has characters outside `[A-Za-z0-9_.-]`, an input cannot be read or `build`
/// fails. A cache that cannot be found, created or written is a warning (`api-cache`): the freshly built index
/// is returned.
pub fn cached<T: Cacheable>(
    key: &str,
    inputs: &[PathBuf],
    refresh: bool,
    warnings: &mut dyn WarningSink,
    build: impl FnOnce(&mut dyn WarningSink) -> Result<T, Error>,
) -> Result<T, Error> {
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(error!("invalid cache key \"{key}\""));
    }
    let dir = cache_dir()
        .inspect_err(|e| {
            cache_warning(
                warnings,
                &format!("{e}; the index is built without a cache"),
            );
        })
        .ok();
    let want = stamps(inputs)?;
    if !refresh
        && let Some(dir) = &dir
        && let Some(index) = load(&dir.join(format!("{key}.json")), &want)
    {
        return Ok(index);
    }
    let mut index = build(warnings)?;
    index.set_inputs(want);
    if let Some(dir) = &dir {
        // The index is right either way; a cache that cannot be written only costs time.
        let _ = store(dir, key, &index)
            .inspect_err(|e| cache_warning(warnings, &format!("cannot write the cache: {e}")));
    }
    Ok(index)
}

/// The first 8 hex digits of the SHA-1 of the canonical `dir`, for cache keys of mods.
#[must_use]
pub fn dir_hash(dir: &Path) -> String {
    use sha1::{Digest, Sha1};
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let text = canonical.to_string_lossy();
    let mut hex = String::new();
    for b in Sha1::digest(text.as_bytes()).iter().take(4) {
        let _ = write!(hex, "{b:02x}"); // writing to a String cannot fail
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache key must not change between releases: `printf %s /nonexistent/dayzmake-pin/mod | sha1sum`.
    #[test]
    fn dir_hash_is_the_first_eight_hex_digits_of_the_sha1_of_the_path() {
        assert_eq!(
            dir_hash(Path::new("/nonexistent/dayzmake-pin/mod")),
            "996712c3"
        );
    }
}
