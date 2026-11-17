//! Extracts a whole DayZ install, a mod or a single PBO into one tree organised by prefix.
//!
//! The tree is written to a staging folder next to the target and swapped in with a rename when
//! everything succeeded, so a failed or repeated run never leaves a half-written or stale tree. The
//! entry names, prefixes and PBO files are untrusted: names are validated like [`crate::pbo::unpack`]
//! does and the offenders are skipped with a warning.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Error, ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::config::{Config, WriteOptions, is_rapified};
use crate::error::{ErrorExt, printable};
use crate::pbo::{
    EntryFilter, IndexEntry, decompress_entry, prefix_relative, read_index, safe_relative_path,
    unpack_relative, warn_leading_separator,
};
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

/// Whether `folder` is empty or holds the `pbo-map.tsv` of an earlier run, so that replacing it cannot
/// destroy anything but an extract result.
fn looks_like_extract_result(folder: &Path) -> bool {
    let is_empty = fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_none());
    is_empty || fs::symlink_metadata(folder.join(MAP_FILE)).is_ok_and(|m| m.is_file())
}

/// Checks the target before anything is written: an existing one needs `force`, must be a real folder
/// that is empty or an earlier result, and must neither contain nor lie inside the source.
fn check_target(
    sources: &[SourcePbo],
    source: &Path,
    target: &Path,
    force: bool,
) -> Result<(), Error> {
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
            // paths that cannot be compared count as overlapping: better refuse than delete the source
            let outer = target.canonicalize();
            let overlaps = |inner: &Path| match (&outer, inner.canonicalize()) {
                (Ok(outer), Ok(inner)) => inner.starts_with(outer),
                _ => true,
            };
            let inside = match (&outer, source.canonicalize()) {
                (Ok(outer), Ok(source)) => outer.starts_with(source),
                _ => true,
            };
            if inside {
                return Err(error!(
                    "Target folder \"{shown}\" is inside the source; refusing to replace it."
                ));
            }
            if overlaps(source) || sources.iter().any(|s| overlaps(&s.path)) {
                return Err(error!(
                    "Target folder \"{shown}\" contains the source; refusing to replace it."
                ));
            }
            if !looks_like_extract_result(target) {
                return Err(error!(
                    "Target folder \"{shown}\" is not empty and not the result of an earlier extract (it has no {MAP_FILE}); refusing to replace it."
                ));
            }
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).prepend_error(format!("Failed to read target \"{shown}\":")),
    }
}

/// The final folder name of `target`; a target that ends in `..` or is a root has none and is refused.
fn target_name(target: &Path) -> Result<String, Error> {
    target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            error!(
                "Invalid target folder \"{}\".",
                printable(&target.display().to_string())
            )
        })
}

/// Creates an empty staging folder next to `target` (same parent, so the final rename stays on one
/// file system).
fn create_staging(target: &Path) -> Result<Staging, Error> {
    let name = target_name(target)?;
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

/// `bytes` with `\`, TAB, CR and LF escaped as `\\`, `\t`, `\r` and `\n`, so that every `\` of the output starts an
/// escape.
fn escape(bytes: &[u8], out: &mut Vec<u8>) {
    for &b in bytes {
        match b {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\n' => out.extend_from_slice(b"\\n"),
            _ => out.push(b),
        }
    }
}

/// The largest entry that is read into memory (compressed ones and derapify candidates); everything
/// else is copied from the PBO to the file in pieces.
const MAX_IN_MEMORY: u64 = 1 << 29;

/// What an entry's file will hold.
enum Content {
    /// Copy this many bytes from the PBO, starting at the offset.
    Stream {
        offset: u64,
        size: u64,
    },
    Bytes(Vec<u8>),
}

/// The state that spans all PBOs of one run.
struct Run<'a> {
    staging: &'a Path,
    options: &'a ExtractOptions,
    filter: EntryFilter,
    /// The PBO paths, by index.
    names: Vec<String>,
    /// Lowercase relative path of every file written -> (path as written, index of its PBO).
    owners: BTreeMap<String, (PathBuf, usize)>,
    /// Prefix bytes and PBO path of every PBO.
    map: Vec<(Vec<u8>, String)>,
    files: usize,
}

impl Run<'_> {
    /// Extracts one PBO. A PBO that cannot be read or an entry that cannot be decompressed or written is
    /// an error; unsafe names are skipped.
    fn pbo(
        &mut self,
        index: usize,
        source: &SourcePbo,
        warnings: &mut dyn WarningSink,
    ) -> Result<(), Error> {
        let shown = printable(&source.relative);
        let mut file = BufReader::new(
            File::open(&source.path).prepend_error(format!("Failed to open PBO \"{shown}\":"))?,
        );
        let pbo_index =
            read_index(&mut file).prepend_error(format!("Failed to read PBO \"{shown}\":"))?;

        let stem = source
            .path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let prefix = pbo_index.prefix.as_deref();
        let folder = match prefix_relative(prefix) {
            Ok(Some(relative)) => Some(relative),
            Ok(None) => safe_relative_path(&stem),
            Err(_) => None,
        };
        let map_prefix = match (prefix_relative(prefix), pbo_index.prefix_bytes) {
            (Ok(None), _) => stem.as_bytes().to_vec(),
            (_, Some(raw)) => raw,
            (_, None) => prefix.unwrap_or_default().as_bytes().to_vec(),
        };
        // `/` separators like the tree, so that the file needs no `\` of its own
        let map_prefix: Vec<u8> = map_prefix
            .iter()
            .map(|&b| if b == b'\\' { b'/' } else { b })
            .collect();
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

        for entry in &pbo_index.entries {
            if !self.filter.matches(&entry.name) {
                continue;
            }
            let end = entry.offset.checked_add(entry.size);
            if end.is_none_or(|end| end > pbo_index.len) {
                return Err(error!(
                    "PBO \"{shown}\": entry \"{}\" is truncated (its data ends past the end of the file).",
                    printable(&entry.name)
                ));
            }
            self.entry(index, &folder, entry, &mut file, warnings)
                .prepend_error(format!("PBO \"{shown}\":"))?;
        }
        Ok(())
    }

    /// Writes one entry below `folder`, or warns why it is skipped.
    fn entry<F: Read + Seek>(
        &mut self,
        index: usize,
        folder: &Path,
        entry: &IndexEntry,
        file: &mut F,
        warnings: &mut dyn WarningSink,
    ) -> Result<(), Error> {
        let name = entry.name.as_str();
        if entry.non_utf8 {
            warnings.warn(Warning::new(
                Some("non-utf8-filename"),
                format!("Entry name {name:?} is not valid UTF-8; extracting it as windows-1252."),
            ));
        }
        let Some((relative, stripped)) = unpack_relative(name) else {
            warnings.warn(Warning::new(
                Some("unsafe-path"),
                format!("Skipping \"{}\": unsafe path.", printable(name)),
            ));
            return Ok(());
        };
        if stripped {
            warn_leading_separator(name, &relative, warnings);
        }
        let is_bin = relative
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("bin"));
        let fail = |what: &str| format!("{what} \"{}\":", printable(name));
        let mut relative = relative;
        let mut content = Content::Stream {
            offset: entry.offset,
            size: entry.size,
        };
        if entry.compressed {
            if entry.size > MAX_IN_MEMORY || u64::from(entry.original_size) > MAX_IN_MEMORY {
                return Err(error!(
                    "Compressed entry \"{}\" is too large.",
                    printable(name)
                ));
            }
            let stored =
                read_at(file, entry.offset, entry.size).prepend_error(fail("Failed to read"))?;
            let data = decompress_entry(&stored, entry.original_size)
                .prepend_error(fail("Failed to decompress"))?;
            content = Content::Bytes(data);
        }
        if self.options.derapify && is_bin {
            let (derapified, kept) = derapify_entry(entry, content, file, warnings)?;
            if derapified {
                relative = relative.with_extension("cpp");
            }
            content = kept;
        }
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
                return Ok(());
            }
            let pbo_name = |i: usize| self.names.get(i).map_or("?", String::as_str);
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).prepend_error(fail("Failed to create the folder of"))?;
        }
        match content {
            Content::Bytes(data) => fs::write(&path, data),
            Content::Stream { offset, size } => file
                .seek(SeekFrom::Start(offset))
                .and_then(|_| File::create(&path))
                .and_then(|mut out| {
                    let copied = io::copy(&mut file.by_ref().take(size), &mut out)?;
                    if copied == size {
                        Ok(())
                    } else {
                        Err(Error::new(
                            ErrorKind::UnexpectedEof,
                            "the entry is truncated",
                        ))
                    }
                }),
        }
        .prepend_error(fail("Failed to write"))?;
        if self.owners.insert(key, (relative, index)).is_none() {
            self.files += 1;
        }
        Ok(())
    }
}

/// The content of a `.bin` entry as text when it is a rapified config (`true`), else as it was. A config
/// that cannot be derapified is kept with a `derapify-failed` warning.
fn derapify_entry<F: Read + Seek>(
    entry: &IndexEntry,
    content: Content,
    file: &mut F,
    warnings: &mut dyn WarningSink,
) -> Result<(bool, Content), Error> {
    let name = entry.name.as_str();
    let fail = |what: &str| format!("{what} \"{}\":", printable(name));
    let head = match &content {
        Content::Bytes(data) => data.get(..4).map(<[u8]>::to_vec),
        Content::Stream { offset, size } if *size >= 4 => {
            Some(read_at(file, *offset, 4).prepend_error(fail("Failed to read"))?)
        }
        Content::Stream { .. } => None,
    };
    if !head.is_some_and(|h| is_rapified(&h)) {
        return Ok((false, content));
    }
    let data = match content {
        Content::Bytes(data) => data,
        Content::Stream { offset, size } if size <= MAX_IN_MEMORY => {
            read_at(file, offset, size).prepend_error(fail("Failed to read"))?
        }
        Content::Stream { offset, size } => {
            warnings.warn(Warning::new(
                Some("derapify-failed"),
                format!(
                    "Keeping \"{}\" as it is: the entry is too large to derapify.",
                    printable(name)
                ),
            ));
            return Ok((false, Content::Stream { offset, size }));
        }
    };
    match derapify(&data) {
        Ok(text) => Ok((true, Content::Bytes(text))),
        Err(e) => {
            warnings.warn(Warning::new(
                Some("derapify-failed"),
                format!("Keeping \"{}\" as it is: {e}", printable(name)),
            ));
            Ok((false, Content::Bytes(data)))
        }
    }
}

/// `size` bytes of `file` at `offset`; the caller has checked `size` against `MAX_IN_MEMORY`.
fn read_at<F: Read + Seek>(file: &mut F, offset: u64, size: u64) -> Result<Vec<u8>, Error> {
    file.seek(SeekFrom::Start(offset))?;
    let mut data = Vec::new();
    file.by_ref().take(size).read_to_end(&mut data)?;
    if u64::try_from(data.len()).ok() != Some(size) {
        return Err(Error::new(
            ErrorKind::UnexpectedEof,
            "the entry is truncated",
        ));
    }
    Ok(data)
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
/// separators, sorted by prefix. The prefix is written with `/` instead of `\` (like the tree below the target); then
/// `\`, TAB, CR and LF in either column are written as `\\`, `\t`, `\r` and `\n`, so every `\` in the file starts
/// an escape. The bytes of the prefix as stored are in the `--json` output of `inspect`.
///
/// # Errors
///
/// Returns an error if the source has no PBO or cannot be read, a PBO is malformed or an entry is truncated
/// (its data ends past the end of the file), an entry cannot be read or decompressed, the target exists and
/// `options.force` is not set, the target is not a folder, is inside the source or contains it, the target
/// folder is neither empty nor the result of an earlier extract (no `pbo-map.tsv`), or the tree cannot be
/// written. A failure leaves an existing target alone.
pub fn extract(
    source: &Path,
    target: &Path,
    options: &ExtractOptions,
    warnings: &mut dyn WarningSink,
) -> Result<ExtractSummary, Error> {
    let sources = find_pbos(source)?;
    // before anything looks at the target: `a/missing/..` must fail the same way where it resolves to `a`
    target_name(target)?;
    check_target(&sources, source, target, options.force)?;
    let mut staging = create_staging(target)?;
    let mut run = Run {
        staging: &staging.path.clone(),
        options,
        names: sources.iter().map(|s| s.relative.clone()).collect(),
        filter: EntryFilter::new(options.only.clone(), options.exclude.clone()),
        owners: BTreeMap::new(),
        map: Vec::new(),
        files: 0,
    };
    for (index, source) in sources.iter().enumerate() {
        run.pbo(index, source, warnings)?;
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
