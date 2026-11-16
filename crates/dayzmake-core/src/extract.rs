//! Extracts a whole DayZ install, a mod or a single PBO into one tree organised by prefix.
//!
//! The tree is written to a staging folder next to the target and swapped in with a rename when
//! everything succeeded, so a failed or repeated run never leaves a half-written or stale tree. The
//! entry names, prefixes and PBO files are untrusted: names are validated like [`crate::pbo::unpack`]
//! does and the offenders are skipped with a warning.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, Cursor, Error, ErrorKind};
use std::path::{Path, PathBuf};

use crate::config::{Config, WriteOptions, is_rapified};
use crate::error::{ErrorExt, printable};
use crate::pbo::warn_leading_separator;
use crate::pbo::{EntryFilter, PBO, prefix_relative, safe_relative_path, unpack_relative};
use crate::warning::{Warning, WarningSink};

/// The file in the target that maps every prefix to its PBO.
const MAP_FILE: &str = "pbo-map.tsv";

/// What [`extract`] does.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ExtractOptions {
    /// Replace an existing target folder instead of failing.
    pub force: bool,
    /// Write every rapified `*.bin` as text, with the extension `.cpp`.
    pub derapify: bool,
    /// Glob patterns of entries to leave out; see [`EntryFilter`].
    pub exclude: Vec<String>,
    /// Glob patterns of the entries to write; see [`EntryFilter`]. Empty means every entry.
    pub only: Vec<String>,
}

/// What [`extract`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExtractSummary {
    pbos: usize,
    files: usize,
}

impl ExtractSummary {
    /// The number of PBOs that were read.
    #[must_use]
    pub fn pbos(&self) -> usize {
        self.pbos
    }

    /// The number of files in the tree, without the PBO map.
    #[must_use]
    pub fn files(&self) -> usize {
        self.files
    }
}

/// A PBO of the source: its path and its path relative to the source with `/` separators.
struct SourcePbo {
    path: PathBuf,
    relative: String,
}

/// Removes the staging folder unless it was handed over.
struct Staging {
    path: PathBuf,
    armed: bool,
}

impl Drop for Staging {
    fn drop(&mut self) {
        if self.armed {
            // best effort on the way out: the error that got us here matters more
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn is_dir(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_dir())
}

/// The PBOs of `source`, in sorted path order: the file itself, or the `*.pbo` files directly in the
/// `dta` and `addons` folders of a folder (any case).
fn find_pbos(source: &Path) -> Result<Vec<SourcePbo>, Error> {
    let shown = printable(&source.display().to_string());
    let meta = fs::metadata(source).prepend_error(format!("Failed to read source \"{shown}\":"))?;
    let mut found = Vec::new();
    if meta.is_file() {
        let name = source
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        found.push(SourcePbo {
            path: source.to_path_buf(),
            relative: name,
        });
        return Ok(found);
    }
    let read = |dir: &Path| {
        fs::read_dir(dir).prepend_error(format!(
            "Failed to read folder \"{}\":",
            printable(&dir.display().to_string())
        ))
    };
    for folder in read(source)? {
        let folder = folder.prepend_error(format!("Failed to read folder \"{shown}\":"))?;
        let folder_name = folder.file_name().to_string_lossy().into_owned();
        let lower = folder_name.to_lowercase();
        if (lower != "dta" && lower != "addons") || !is_dir(&folder.path()) {
            continue;
        }
        for file in read(&folder.path())? {
            let file = file.prepend_error("Failed to read folder entry:")?;
            let path = file.path();
            let is_pbo = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("pbo"));
            if is_pbo && fs::metadata(&path).is_ok_and(|m| m.is_file()) {
                found.push(SourcePbo {
                    relative: format!("{folder_name}/{}", file.file_name().to_string_lossy()),
                    path,
                });
            }
        }
    }
    found.sort_by(|a, b| a.relative.cmp(&b.relative));
    if found.is_empty() {
        return Err(error!(
            "Found no PBOs in the dta or addons folder of \"{shown}\"."
        ));
    }
    Ok(found)
}

/// Checks the target before anything is written: an existing one needs `force`, must be a real folder
/// and must not contain the source.
fn check_target(source: &Path, target: &Path, force: bool) -> Result<(), Error> {
    let shown = printable(&target.display().to_string());
    match fs::symlink_metadata(target) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(error!("Target \"{shown}\" exists and is not a folder."));
            }
            if !force {
                return Err(error!(
                    "Target folder \"{shown}\" already exists. Use --force to replace it."
                ));
            }
            let contains = |inner: &Path| {
                match (target.canonicalize(), inner.canonicalize()) {
                    (Ok(outer), Ok(inner)) => inner.starts_with(outer),
                    // not comparable: do not take the chance of deleting the source
                    _ => true,
                }
            };
            if contains(source) {
                return Err(error!(
                    "Target folder \"{shown}\" contains the source; refusing to replace it."
                ));
            }
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).prepend_error(format!("Failed to read target \"{shown}\":")),
    }
}

/// Creates an empty staging folder next to `target` (same parent, so the final rename stays on one
/// file system).
fn create_staging(target: &Path) -> Result<Staging, Error> {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            error!(
                "Invalid target folder \"{}\".",
                printable(&target.display().to_string())
            )
        })?;
    let parent = parent_of(target);
    fs::create_dir_all(&parent).prepend_error("Failed to create the folder of the target:")?;
    for attempt in 0..1000_u32 {
        let path = parent.join(format!(".{name}.extract-{}-{attempt}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(Staging { path, armed: true }),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).prepend_error("Failed to create the staging folder:"),
        }
    }
    Err(error!("Failed to find a free name for the staging folder."))
}

fn parent_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// `data` as text if it is a rapified config.
fn derapify(data: &[u8]) -> Result<Vec<u8>, Error> {
    let config = Config::read_rapified(&mut Cursor::new(data))?;
    let mut text = Vec::new();
    config.write_with(&mut text, &WriteOptions::new())?;
    Ok(text)
}

/// `bytes` with TAB, CR and LF escaped as `\t`, `\r` and `\n`.
fn escape(bytes: &[u8], out: &mut Vec<u8>) {
    for &b in bytes {
        match b {
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\n' => out.extend_from_slice(b"\\n"),
            _ => out.push(b),
        }
    }
}

/// The state that spans all PBOs of one run.
struct Run<'a> {
    staging: &'a Path,
    options: &'a ExtractOptions,
    filter: EntryFilter,
    /// Lowercase relative path of every file written -> (path as written, index of its PBO).
    owners: BTreeMap<String, (PathBuf, usize)>,
    /// Prefix bytes and PBO path of every PBO.
    map: Vec<(Vec<u8>, String)>,
    files: usize,
}

impl Run<'_> {
    /// Extracts one PBO. A PBO that cannot be read is an error; unsafe names are skipped.
    fn pbo(
        &mut self,
        index: usize,
        sources: &[SourcePbo],
        warnings: &mut dyn WarningSink,
    ) -> Result<(), Error> {
        let Some(source) = sources.get(index) else {
            return Ok(());
        };
        let shown = printable(&source.relative);
        let file =
            File::open(&source.path).prepend_error(format!("Failed to open PBO \"{shown}\":"))?;
        let selection =
            PBO::read_entries(&mut BufReader::new(file), |name| self.filter.matches(name))
                .prepend_error(format!("Failed to read PBO \"{shown}\":"))?;

        let stem = source
            .path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let prefix = selection.header_extension("prefix");
        let (map_prefix, folder) = match prefix_relative(prefix) {
            Ok(Some(relative)) => (
                selection.prefix_bytes().map_or_else(
                    || prefix.unwrap_or_default().as_bytes().to_vec(),
                    <[u8]>::to_vec,
                ),
                Some(relative),
            ),
            Ok(None) => (stem.as_bytes().to_vec(), safe_relative_path(&stem)),
            Err(_) => (
                selection.prefix_bytes().map_or_else(
                    || prefix.unwrap_or_default().as_bytes().to_vec(),
                    <[u8]>::to_vec,
                ),
                None,
            ),
        };
        self.map.push((map_prefix, source.relative.clone()));
        let folder = folder.filter(|f| {
            !f.components().next().is_some_and(|c| {
                c.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(MAP_FILE)
            })
        });
        let Some(folder) = folder else {
            warnings.warn(Warning::new(
                Some("unsafe-path"),
                format!("Skipping PBO \"{shown}\": its prefix is not a safe folder name."),
            ));
            return Ok(());
        };

        for entry in selection.entries() {
            self.entry(
                index,
                sources,
                &folder,
                entry.name(),
                entry.data(),
                warnings,
            );
        }
        Ok(())
    }

    /// Writes one entry below `folder`, or warns why not.
    fn entry(
        &mut self,
        index: usize,
        sources: &[SourcePbo],
        folder: &Path,
        name: &str,
        data: Result<&[u8], &Error>,
        warnings: &mut dyn WarningSink,
    ) {
        let Some((relative, stripped)) = unpack_relative(name) else {
            warnings.warn(Warning::new(
                Some("unsafe-path"),
                format!("Skipping \"{}\": unsafe path.", printable(name)),
            ));
            return;
        };
        let data = match data {
            Ok(data) => data,
            Err(e) => {
                warnings.warn(Warning::new(
                    None,
                    format!("Skipping \"{}\": {e}", printable(name)),
                ));
                return;
            }
        };
        if stripped {
            warn_leading_separator(name, &relative, warnings);
        }
        let is_bin = relative
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("bin"));
        let (relative, data) = if self.options.derapify && is_bin && is_rapified(data) {
            match derapify(data) {
                Ok(text) => (relative.with_extension("cpp"), text),
                Err(e) => {
                    warnings.warn(Warning::new(
                        Some("derapify-failed"),
                        format!("Keeping \"{}\" as it is: {e}", printable(name)),
                    ));
                    (relative, data.to_vec())
                }
            }
        } else {
            (relative, data.to_vec())
        };
        let relative = folder.join(relative);
        let key = relative.to_string_lossy().to_lowercase();
        if let Some((old, owner)) = self.owners.get(&key).cloned() {
            if owner == index {
                warnings.warn(Warning::new(
                    Some("duplicate-entry"),
                    format!(
                        "Skipping \"{}\": an entry with the same path was already extracted.",
                        printable(name)
                    ),
                ));
                return;
            }
            let pbo_name = |i: usize| sources.get(i).map_or("?", |s| s.relative.as_str());
            warnings.warn(Warning::new(
                Some("duplicate-path"),
                format!(
                    "\"{}\" is in {} and in {}; keeping the one of {}.",
                    printable(&relative.to_string_lossy()),
                    printable(pbo_name(owner)),
                    printable(pbo_name(index)),
                    printable(pbo_name(index)),
                ),
            ));
            // the file is replaced below; one that is already gone does not matter
            let _ = fs::remove_file(self.staging.join(old));
        }
        let path = self.staging.join(&relative);
        // the staging folder is new and holds nothing but what this run wrote, so no path in it is a symlink
        let written = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&path, &data));
        match written {
            Ok(()) => {
                if self.owners.insert(key, (relative, index)).is_none() {
                    self.files += 1;
                }
            }
            Err(e) => warnings.warn(Warning::new(
                None,
                format!("Skipping \"{}\": {e}", printable(name)),
            )),
        }
    }
}

/// Replaces `target` with the finished `staging` folder. The old tree is renamed away first, so a failure
/// puts it back.
fn swap(staging: &mut Staging, target: &Path, warnings: &mut dyn WarningSink) -> Result<(), Error> {
    let shown = printable(&target.display().to_string());
    let old = match fs::symlink_metadata(target) {
        Ok(_) => {
            let mut old = staging.path.clone().into_os_string();
            old.push(".old");
            let old = PathBuf::from(old);
            fs::rename(target, &old)
                .prepend_error(format!("Failed to move the old target \"{shown}\" away:"))?;
            Some(old)
        }
        Err(_) => None,
    };
    if let Err(e) = fs::rename(&staging.path, target) {
        if let Some(old) = &old {
            // put the old tree back; if that fails too the error below names where it is
            if let Err(back) = fs::rename(old, target) {
                return Err(error!(
                    "Failed to move the new tree to \"{shown}\": {e}; the old tree is in \"{}\" ({back}).",
                    printable(&old.display().to_string())
                ));
            }
        }
        return Err(e).prepend_error(format!("Failed to move the new tree to \"{shown}\":"));
    }
    staging.armed = false;
    if let Some(old) = old
        && let Err(e) = fs::remove_dir_all(&old)
    {
        warnings.warn(Warning::new(
            Some("old-tree-left"),
            format!(
                "Could not remove the old tree \"{}\": {e}",
                printable(&old.display().to_string())
            ),
        ));
    }
    Ok(())
}

/// Extracts `source` into `target`.
///
/// `source` is a DayZ client or server folder, a mod folder or a single PBO. For a folder, every `*.pbo`
/// directly in its `dta` and `addons` subfolders (any case) is read, in sorted path order. The entries of
/// each PBO go to `<target>/<prefix>/<entry>`; an empty prefix means the file stem of the PBO. Names are
/// validated like [`crate::pbo::unpack`] does and unsafe ones are skipped with a warning. When two PBOs
/// write the same path (ignoring case), the later one wins and a `duplicate-path` warning names both.
/// With `options.derapify`, a rapified `*.bin` becomes a `.cpp` of the same name, or stays a `.bin` with a
/// `derapify-failed` warning. `options.only` and `options.exclude` select entries like they do for
/// [`crate::pbo::unpack`].
///
/// The tree is written to a staging folder next to `target` and then replaces it with one rename, so a
/// re-run drops files that no longer exist and a failure leaves the old tree alone. The file `pbo-map.tsv`
/// in the target has one line per PBO, `prefix<TAB>path of the PBO relative to source` with `/`
/// separators, sorted by prefix; TAB, CR and LF in the prefix are written as `\t`, `\r` and `\n`.
///
/// # Errors
///
/// Returns an error if the source has no PBO or cannot be read, a PBO is malformed, the target exists
/// and `options.force` is not set, is not a folder or contains the source, or the tree cannot be written.
pub fn extract(
    source: &Path,
    target: &Path,
    options: &ExtractOptions,
    warnings: &mut dyn WarningSink,
) -> Result<ExtractSummary, Error> {
    let sources = find_pbos(source)?;
    check_target(source, target, options.force)?;
    let mut staging = create_staging(target)?;
    let mut run = Run {
        staging: &staging.path.clone(),
        options,
        filter: EntryFilter::new(options.only.clone(), options.exclude.clone()),
        owners: BTreeMap::new(),
        map: Vec::new(),
        files: 0,
    };
    for index in 0..sources.len() {
        run.pbo(index, &sources, warnings)?;
    }
    run.map.sort();
    let mut text = Vec::new();
    for (prefix, path) in &run.map {
        escape(prefix, &mut text);
        text.push(b'\t');
        escape(path.as_bytes(), &mut text);
        text.push(b'\n');
    }
    fs::write(staging.path.join(MAP_FILE), &text).prepend_error("Failed to write the PBO map:")?;
    let summary = ExtractSummary {
        pbos: sources.len(),
        files: run.files,
    };
    swap(&mut staging, target, warnings)?;
    Ok(summary)
}
