//! The index cache: one JSON file per index, rebuilt automatically when an input file changes.

use std::fmt::Write as _;
use std::fs;
use std::io::Error;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::model::{ApiIndex, INDEX_FORMAT, InputStamp};
use crate::error;

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

fn load(file: &Path, want: &[InputStamp]) -> Option<ApiIndex> {
    let bytes = fs::read(file).ok()?;
    let index: ApiIndex = serde_json::from_slice(&bytes).ok()?;
    (index.format == INDEX_FORMAT && index.inputs == want).then_some(index)
}

/// Write `index` to `file` through a temp file in the same directory and a rename.
fn store(dir: &Path, key: &str, index: &ApiIndex) -> Result<(), Error> {
    let json = serde_json::to_vec(index).map_err(|e| error!("cannot serialize the index: {e}"))?;
    let tmp = dir.join(format!(".{key}.{}.tmp", std::process::id()));
    fs::write(&tmp, json).map_err(|e| error!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, dir.join(format!("{key}.json"))).map_err(|e| {
        // The rename failed; do not leave the temp file behind. Its removal error adds nothing.
        let _ = fs::remove_file(&tmp);
        error!("cannot store the index {key}: {e}")
    })
}

/// The cached index `key`, or the result of `build` when there is none, it does not parse, has another
/// format, or was made from other inputs (size or mtime). `refresh` always builds. A corrupt cache file is
/// replaced, never an error.
///
/// # Errors
///
/// Returns an error when `key` has characters outside `[A-Za-z0-9_.-]`, an input cannot be read, `build`
/// fails or the cache cannot be written.
pub fn cached(
    key: &str,
    inputs: &[PathBuf],
    refresh: bool,
    build: impl FnOnce() -> Result<ApiIndex, Error>,
) -> Result<ApiIndex, Error> {
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(error!("invalid cache key \"{key}\""));
    }
    let dir = cache_dir()?;
    let want = stamps(inputs)?;
    if !refresh && let Some(index) = load(&dir.join(format!("{key}.json")), &want) {
        return Ok(index);
    }
    let mut index = build()?;
    index.inputs = want;
    store(&dir, key, &index)?;
    Ok(index)
}

/// The first 8 hex digits of the SHA-1 of the canonical `dir`, for cache keys of mods.
#[must_use]
pub fn dir_hash(dir: &Path) -> String {
    use openssl::hash::{MessageDigest, hash};
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let text = canonical.to_string_lossy();
    // A failing SHA-1 is not expected; the key then loses its hash and only mods in same-named dirs share it.
    let digest = hash(MessageDigest::sha1(), text.as_bytes()).ok();
    let mut hex = String::new();
    for b in digest.iter().flat_map(|d| d.iter()).take(4) {
        let _ = write!(hex, "{b:02x}"); // writing to a String cannot fail
    }
    if hex.is_empty() {
        "00000000".to_string()
    } else {
        hex
    }
}
