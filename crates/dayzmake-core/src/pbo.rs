use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{File, create_dir_all, read_dir};
use std::io::{BufReader, Cursor, Error, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use linked_hash_map::LinkedHashMap;
use regex::Regex;
use sha1::{Digest, Sha1};
use std::sync::LazyLock;

#[expect(clippy::unwrap_used, reason = "the static patterns are valid regexes")]
static BINARIZABLE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(rtm|p3d)$").unwrap());
#[expect(clippy::unwrap_used, reason = "the static pattern is a valid regex")]
static P3DO_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.p3do$").unwrap());

use crate::binarize::{self, Binarizer};
use crate::config::{Config, is_rapified};
use crate::error::{ErrorExt, printable};
use crate::fileid::FileId;
use crate::io::WriteExt;
use crate::p3d;
use crate::preprocess::IncludeFolders;
use crate::warning::{Warning, WarningSink};

struct PBOHeader {
    filename: String,
    /// Raw name bytes, only set if the name is not valid UTF-8.
    raw_filename: Option<Vec<u8>>,
    packing_method: u32,
    original_size: u32,
    reserved: u32,
    timestamp: u32,
    data_size: u32,
}

/// What a PBO's header says about one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EntryHeader {
    /// The entry name.
    pub name: String,
    /// The packing method (0 is stored, `0x43707273` is compressed).
    pub packing_method: u32,
    /// The size once decompressed.
    pub original_size: u32,
    /// The size as stored in the PBO.
    pub data_size: u32,
    /// The timestamp field of the entry header (Unix time, usually 0).
    pub timestamp: u32,
    /// The bytes the name is stored as in the PBO header (`name` is decoded from them).
    pub name_bytes: Vec<u8>,
}

/// The header extensions as `(key, value)` pairs and the entry names, both in file order.
pub type HeaderSummary = (Vec<(String, String)>, Vec<String>);

/// Longest entry name or header extension string `PBO::read_headers` accepts.
const MAX_HEADER_STRING: usize = 4096;
/// Most entries `PBO::read_headers` accepts.
const MAX_HEADER_ENTRIES: usize = 1 << 20;
/// Most header extensions `PBO::read_headers` accepts.
const MAX_HEADER_EXTENSIONS: usize = 1024;

/// A NUL terminated string of at most `MAX_HEADER_STRING` bytes.
fn read_bounded_cstring<I: Read>(input: &mut I) -> Result<Vec<u8>, Error> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        input.read_exact(&mut buf)?;
        if buf[0] == 0 {
            return Ok(bytes);
        }
        if bytes.len() >= MAX_HEADER_STRING {
            return Err(error!("PBO header string is too long."));
        }
        bytes.push(buf[0]);
    }
}

/// Decodes an entry name. Names that are not valid UTF-8 are decoded as windows-1252, which maps all
/// 256 byte values to distinct characters, so the original bytes are returned as well.
fn decode_name(bytes: Vec<u8>) -> (String, Option<Vec<u8>>) {
    match String::from_utf8(bytes) {
        Ok(s) => (s, None),
        Err(e) => {
            let bytes = e.into_bytes();
            let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
            (decoded.into_owned(), Some(bytes))
        }
    }
}

/// Normalizes a PBO prefix: forward slashes become backslashes, leading and trailing separators are removed.
fn normalize_prefix(prefix: &str, warn: bool, warnings: &mut dyn WarningSink) -> String {
    let trimmed = prefix.trim();
    let normalized = trimmed.replace('/', "\\");
    let stripped = normalized.trim_matches('\\');
    if warn && stripped.len() != normalized.len() {
        warnings.warn(Warning::new(
            Some("prefix-slash"),
            format!("Removed leading/trailing slashes from prefix {trimmed:?}."),
        ));
    }
    stripped.to_string()
}

/// Sets a header extension: a repeated key keeps its first position and takes the last value, so
/// the order of the extensions is the order of the file.
fn set_extension(extensions: &mut LinkedHashMap<String, String>, key: &str, value: String) {
    if let Some(existing) = extensions.get_mut(key) {
        *existing = value;
    } else {
        extensions.insert(key.to_string(), value);
    }
}

/// Parses `$PBOPREFIX$` into header extensions: a bare line is the prefix, `key=value` lines are
/// extensions. Prefix slashes are normalized (`warn` controls the `prefix-slash` warning).
fn parse_prefix_file(
    content: &str,
    header_extensions: &mut LinkedHashMap<String, String>,
    warn: bool,
    warnings: &mut dyn WarningSink,
) {
    for l in content.lines() {
        if l.is_empty() {
            break;
        }

        match l.split_once('=') {
            None => {
                set_extension(
                    header_extensions,
                    "prefix",
                    normalize_prefix(l, warn, warnings),
                );
            }
            Some((key, value)) if key == "prefix" => {
                set_extension(
                    header_extensions,
                    key,
                    normalize_prefix(value, warn, warnings),
                );
            }
            Some((key, value)) => {
                set_extension(header_extensions, key, value.to_string());
            }
        }
    }
}

/// PBO file
///
/// # Examples
///
/// ```
/// # use std::path::Path;
/// # use dayzmake_core::pbo::{PBO, PackOptions};
/// # use dayzmake_core::warning::Warnings;
/// let pbo = PBO::from_directory(Path::new("src"), &PackOptions::pack(), None, &mut Warnings::new()).expect("Failed to create PBO");
///
/// assert!(pbo.entry_names().any(|name| name == "lib.rs"));
///
/// let mut cursor = pbo.to_cursor().unwrap();
/// let reread = PBO::read(&mut cursor).unwrap();
///
/// assert!(reread.checksum().is_some());
/// ```
pub struct PBO {
    /// The PBO's files in header order, keyed by name.
    files: LinkedHashMap<String, Cursor<Box<[u8]>>>,
    /// Header extensions (`key=value`), including the prefix under the key `prefix`.
    header_extensions: LinkedHashMap<String, String>,
    /// Original bytes of entry names that are not valid UTF-8, keyed by their windows-1252 decoding
    /// (the key used in `files`).
    raw_names: HashMap<String, Vec<u8>>,
    /// The stored bytes of every header extension in the order of `header_extensions`; empty for a
    /// PBO that was not read.
    raw_pairs: Vec<(Vec<u8>, Vec<u8>)>,
    headers: Vec<PBOHeader>,
    /// only defined when reading existing PBOs, for created PBOs this is calculated during writing
    /// and included in the output
    checksum: Option<Vec<u8>>,
    /// The SHA-1 of the bytes that were read (header block and entry data, everything before the
    /// trailer); only defined together with `checksum`.
    computed_checksum: Option<Vec<u8>>,
}

/// Hashes everything that is read through it.
struct HashingReader<'a, I> {
    inner: &'a mut I,
    hash: Sha1,
}

impl<I: Read> Read for HashingReader<'_, I> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if let Some(read) = buf.get(..n) {
            self.hash.update(read);
        }
        Ok(n)
    }
}

impl PBOHeader {
    fn read<I: Read>(input: &mut I) -> Result<PBOHeader, Error> {
        let (filename, raw_filename) = decode_name(read_bounded_cstring(input)?);
        Ok(PBOHeader {
            filename,
            raw_filename,
            packing_method: input.read_u32::<LittleEndian>()?,
            original_size: input.read_u32::<LittleEndian>()?,
            reserved: input.read_u32::<LittleEndian>()?,
            timestamp: input.read_u32::<LittleEndian>()?,
            data_size: input.read_u32::<LittleEndian>()?,
        })
    }

    fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        match &self.raw_filename {
            Some(raw) => output.write_cstring(raw)?,
            None => output.write_cstring(&self.filename)?,
        }
        output.write_u32::<LittleEndian>(self.packing_method)?;
        output.write_u32::<LittleEndian>(self.original_size)?;
        output.write_u32::<LittleEndian>(self.reserved)?;
        output.write_u32::<LittleEndian>(self.timestamp)?;
        output.write_u32::<LittleEndian>(self.data_size)?;
        Ok(())
    }
}

/// The parsed header block of a PBO: the header extensions and the entry headers.
struct HeaderBlock {
    extensions: LinkedHashMap<String, String>,
    /// The stored bytes of every extension `(key, value)`, in the order of `extensions`.
    raw_pairs: Vec<(Vec<u8>, Vec<u8>)>,
    headers: Vec<PBOHeader>,
}

/// Parses the header block of a PBO, up to and including the empty terminating header. The data of the
/// first entry starts right after it.
///
/// The input is untrusted: every string is limited to `MAX_HEADER_STRING` bytes, the number of entries
/// to `MAX_HEADER_ENTRIES` and of header extensions to `MAX_HEADER_EXTENSIONS`, and nothing is allocated
/// from a size in the file.
fn read_header_block<I: Read>(input: &mut I) -> Result<HeaderBlock, Error> {
    let mut block = HeaderBlock {
        extensions: LinkedHashMap::new(),
        raw_pairs: Vec::new(),
        headers: Vec::new(),
    };
    let mut first = true;
    loop {
        let header = PBOHeader::read(input)?;
        // todo: garbage filter

        if header.packing_method == 0x5665_7273 {
            if !first {
                return Err(error!("Duplicate version header in PBO."));
            }
            loop {
                let key_bytes = read_bounded_cstring(input)?;
                let (key, _) = decode_name(key_bytes.clone());
                if key.is_empty() {
                    break;
                }
                if block.extensions.len() >= MAX_HEADER_EXTENSIONS {
                    return Err(error!("PBO has too many header extensions."));
                }
                let value_bytes = read_bounded_cstring(input)?;
                let (value, _) = decode_name(value_bytes.clone());
                match block.extensions.keys().position(|k| *k == key) {
                    Some(at) => {
                        if let Some(pair) = block.raw_pairs.get_mut(at) {
                            pair.1 = value_bytes;
                        }
                    }
                    None => block.raw_pairs.push((key_bytes, value_bytes)),
                }
                set_extension(&mut block.extensions, &key, value);
            }
        } else if header.filename.is_empty() {
            break;
        } else {
            if block.headers.len() >= MAX_HEADER_ENTRIES {
                return Err(error!("PBO has too many entries."));
            }
            block.headers.push(header);
        }
        first = false;
    }
    Ok(block)
}

/// An entry name with its decompressed data, or the reason it cannot be read.
#[derive(Debug)]
pub struct SelectedEntry {
    name: String,
    data: Result<Vec<u8>, Error>,
}

impl SelectedEntry {
    /// The entry name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The decompressed data of the entry.
    ///
    /// # Errors
    ///
    /// Returns the error that kept the entry from being decompressed.
    pub fn data(&self) -> Result<&[u8], &Error> {
        self.data.as_ref().map(Vec::as_slice)
    }
}

/// The entries of a PBO that [`PBO::read_entries`] selected.
#[derive(Debug)]
pub struct PboSelection {
    header_extensions: LinkedHashMap<String, String>,
    names: Vec<String>,
    entries: Vec<SelectedEntry>,
}

impl PboSelection {
    /// All entry names in file order.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The selected entries in file order with their decompressed data; an entry that cannot be
    /// decompressed is an error of its own.
    #[must_use]
    pub fn entries(&self) -> &[SelectedEntry] {
        &self.entries
    }

    /// The value of a header extension (the prefix is the key `prefix`).
    #[must_use]
    pub fn header_extension(&self, key: &str) -> Option<&str> {
        self.header_extensions.get(key).map(String::as_str)
    }

    /// The header extensions (`key=value`) in file order.
    pub fn header_extensions(&self) -> impl Iterator<Item = (&str, &str)> {
        self.header_extensions
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Packing method of entries compressed with Bohemia's LZSS variant ("Cprs").
const PACKING_COMPRESSED: u32 = 0x4370_7273;

/// Decompresses a "Cprs" PBO entry: groups of 8 items led by a flag byte (LSB first),
/// bit set = literal byte, bit clear = 2-byte back reference (12 bit offset, 4 bit length - 3).
/// The window starts out filled with spaces; the 4 byte checksum after the data is ignored.
fn lzss_decompress(data: &[u8], original_size: usize) -> Result<Vec<u8>, Error> {
    let truncated = || error!("Truncated compressed PBO entry.");
    // a flag byte and 8 two byte references expand to at most 8 * 18 bytes out of 17 in
    let max_size = data.len().saturating_mul(9);
    if original_size > max_size {
        return Err(error!(
            "Compressed PBO entry claims an implausible size: {original_size} bytes from {} bytes.",
            data.len()
        ));
    }
    let mut out: Vec<u8> = Vec::with_capacity(original_size.min(max_size));
    let mut pos = 0;

    while out.len() < original_size {
        let flags = *data.get(pos).ok_or_else(truncated)?;
        pos += 1;

        for bit in 0..8 {
            if out.len() >= original_size {
                break;
            }

            if flags & (1 << bit) != 0 {
                out.push(*data.get(pos).ok_or_else(truncated)?);
                pos += 1;
            } else {
                let b1 = *data.get(pos).ok_or_else(truncated)? as usize;
                let b2 = *data.get(pos + 1).ok_or_else(truncated)? as usize;
                pos += 2;

                let offset = b1 | ((b2 & 0xF0) << 4);
                let length = (b2 & 0x0F) + 3;
                if offset == 0 {
                    return Err(error!("Invalid back reference in compressed PBO entry."));
                }

                for _ in 0..length {
                    let byte = out
                        .len()
                        .checked_sub(offset)
                        .and_then(|i| out.get(i))
                        .copied()
                        .unwrap_or(b' ');
                    out.push(byte);
                }
            }
        }
    }

    out.truncate(original_size);
    Ok(out)
}

/// Matches `s` against `pattern`, where `*` stands for any run of characters, including none.
fn matches_glob(s: &str, pattern: &str) -> bool {
    let s: Vec<char> = s.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut si, mut pi) = (0, 0);
    // pattern position after the last `*` and the string position it currently covers up to
    let mut backtrack: Option<(usize, usize)> = None;

    while si < s.len() {
        match pattern.get(pi) {
            Some('*') => {
                pi += 1;
                backtrack = Some((pi, si));
            }
            Some(c) if Some(c) == s.get(si) => {
                pi += 1;
                si += 1;
            }
            _ => match backtrack {
                Some((after_star, covered)) => {
                    pi = after_star;
                    si = covered + 1;
                    backtrack = Some((after_star, si));
                }
                None => return false,
            },
        }
    }

    pattern
        .get(pi..)
        .is_some_and(|rest| rest.iter().all(|&c| c == '*'))
}

fn file_allowed(name: &str, exclude_patterns: &[String]) -> bool {
    for pattern in exclude_patterns {
        if matches_glob(name, pattern) {
            return false;
        }
    }

    true
}

/// What [`PBO::from_directory`] does with a folder.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PackOptions {
    /// Binarize models and rapify configs (`build`) instead of packing the files as they are.
    pub build: bool,
    /// Glob patterns of files left out of the PBO.
    pub exclude: Vec<String>,
    /// Folders searched for absolute includes in configs and model dependencies.
    pub include_folders: Vec<PathBuf>,
    /// Folders checked for absolute includes only directly (`<folder>/<include path>`), never walked. The CLI puts
    /// the current working directory here (#66).
    pub direct_include_folders: Vec<PathBuf>,
    /// Header extensions (`key`, `value`) set after `$PBOPREFIX$` was read; they win on a clash.
    pub header_extensions: Vec<(String, String)>,
    /// Paths the folder walk skips (the output file of a build inside the source folder), compared after
    /// canonicalizing. Hidden temp files of such a target (see [`atomic_temp_prefix`]) next to it are skipped too.
    pub exclude_paths: Vec<PathBuf>,
    /// Also skip the file this process's standard output is redirected to, when it is a regular file (`dayzmake
    /// pack m > m/out.pbo` creates the file inside the folder before the walk). A terminal, a pipe, `/dev/null`
    /// or a closed stdout excludes nothing.
    pub exclude_stdout: bool,
}

/// The file-name prefix of the temp files written next to the target `name` before they are renamed into place
/// (`.<name>.dayzmake-tmp-<pid>-<n>`); a build leaves them out of the PBO.
#[must_use]
pub fn atomic_temp_prefix(name: &str) -> String {
    format!(".{name}.dayzmake-tmp-")
}

/// A file the folder walk skips.
struct Excluded {
    /// The canonical folder, name and temp prefix of an excluded path; none for standard output, which has no path.
    by_name: Option<ExcludedName>,
    /// The identity of the excluded file when it exists, for the comparison that is independent of spelling.
    id: Option<FileId>,
}

/// An excluded path split into its canonical folder and file name.
struct ExcludedName {
    folder: PathBuf,
    name: std::ffi::OsString,
    /// Lower-cased temp prefix: the temp file does not exist yet, so it is matched by name.
    temp_prefix: String,
}

fn excluded_paths(options: &PackOptions) -> Vec<Excluded> {
    let mut list: Vec<Excluded> = options
        .exclude_paths
        .iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_owned();
            let parent = p.parent().filter(|d| !d.as_os_str().is_empty());
            // fails open: an exclude whose folder cannot be resolved matches nothing (the folder exists in practice,
            // the temp file was created there)
            let folder = std::fs::canonicalize(parent.unwrap_or(Path::new("."))).ok()?;
            let temp_prefix = atomic_temp_prefix(&name.to_string_lossy()).to_lowercase();
            let id = FileId::of(&folder.join(&name));
            Some(Excluded {
                by_name: Some(ExcludedName {
                    folder,
                    name,
                    temp_prefix,
                }),
                id,
            })
        })
        .collect();
    if options.exclude_stdout
        && let Some(id) = FileId::stdout()
    {
        list.push(Excluded {
            by_name: None,
            id: Some(id),
        });
    }
    list
}

/// Whether `path` is the build's own output or a temp file of it.
///
/// The output is recognised by file identity when it exists (case-insensitive file systems, hard links and symlinks
/// name it differently), by exact name in the same folder otherwise. The temp prefix is matched by name, ignoring
/// case on every platform: the temp file does not exist yet, and a stale hidden `.X.dayzmake-tmp-` file that differs
/// only in case is never wanted in a PBO.
fn is_excluded(path: &Path, excluded: &[Excluded]) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name_lower = name.to_string_lossy().to_lowercase();
    // canonicalize and open only when needed, once per path
    let mut folder: Option<Option<PathBuf>> = None;
    let mut id: Option<Option<FileId>> = None;
    excluded.iter().any(|e| {
        if let Some(n) = &e.by_name
            && (name == n.name || name_lower.starts_with(&n.temp_prefix))
        {
            let folder = folder.get_or_insert_with(|| {
                path.parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map_or_else(
                        // fails open like `excluded_paths`: an unresolvable folder is never equal to an excluded one
                        || std::fs::canonicalize(".").ok(),
                        |d| std::fs::canonicalize(d).ok(),
                    )
            });
            if folder.as_deref() == Some(n.folder.as_path()) {
                return true;
            }
        }
        // an unopenable file (gone meanwhile, no permission) is not the same file; opened lazily: only when
        // there is an identity to compare with
        e.id.as_ref()
            .is_some_and(|own| id.get_or_insert_with(|| FileId::of(path)).as_ref() == Some(own))
    })
}

impl PackOptions {
    /// Options for packing a folder as it is.
    #[must_use]
    pub fn pack() -> PackOptions {
        PackOptions::default()
    }

    /// Options for building a folder: binarize and rapify where needed.
    #[must_use]
    pub fn build() -> PackOptions {
        PackOptions {
            build: true,
            ..PackOptions::default()
        }
    }
}

impl Default for PBO {
    fn default() -> PBO {
        PBO::new()
    }
}

/// Rewrites `Warning.file` to the path relative to the addon folder when it lies inside it.
struct AddonRelative<'a> {
    /// The addon folder as given and canonicalised (includes are reported canonicalised).
    roots: &'a [PathBuf; 2],
    inner: &'a mut dyn WarningSink,
}

/// The addon folder as given and canonicalised, computed once per pack.
fn addon_roots(directory: &Path) -> [PathBuf; 2] {
    // canonicalize fails for a missing or unreadable folder; matching against the path as given still
    // works, so warnings are not lost
    let canonical = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf());
    [directory.to_path_buf(), canonical]
}

impl WarningSink for AddonRelative<'_> {
    fn warn(&mut self, mut warning: Warning) {
        if let Some(file) = &warning.file {
            let file = Path::new(file);
            if let Some(relative) = self
                .roots
                .iter()
                .find_map(|root| file.strip_prefix(root).ok())
            {
                warning.file = Some(relative.to_string_lossy().into_owned());
            }
        }
        self.inner.warn(warning);
    }

    fn is_suppressed(&self, name: Option<&str>) -> bool {
        self.inner.is_suppressed(name)
    }
}

impl PBO {
    /// An empty PBO without entries and header extensions.
    #[must_use]
    pub fn new() -> PBO {
        PBO {
            files: LinkedHashMap::new(),
            header_extensions: LinkedHashMap::new(),
            raw_names: HashMap::new(),
            raw_pairs: Vec::new(),
            headers: Vec::new(),
            checksum: None,
            computed_checksum: None,
        }
    }

    /// The entries in the order the PBO has them: header order for a PBO that was read and not
    /// modified, the order [`PBO::write`] uses (sorted by name) for any other. Data is as stored, so
    /// compressed entries stay compressed; see [`PBO::file_data`].
    pub fn entries(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.ordered_files()
            .into_iter()
            .map(|(name, cursor)| (name, cursor.get_ref().as_ref()))
    }

    /// The entry names, in the order of [`PBO::entries`].
    pub fn entry_names(&self) -> impl Iterator<Item = &str> {
        self.entries().map(|(name, _)| name)
    }

    /// Adds an entry, or replaces the data of the entry with the same name.
    ///
    /// # Errors
    ///
    /// `/` in the name is stored as `\`. Returns an error if the name has an empty component
    /// (leading, trailing or doubled separator, or no name at all) or is one
    /// [`safe_relative_path`] rejects (drive letter, `..` component and so on). Modifying a PBO
    /// that was read drops its stored checksum: it is signed as the PBO that would be written.
    pub fn insert_entry(&mut self, name: impl Into<String>, data: Vec<u8>) -> Result<(), Error> {
        let name = name.into().replace('/', "\\");
        if safe_relative_path(&name).is_none() {
            return Err(error!("Unsafe PBO entry name \"{}\".", printable(&name)));
        }
        self.modified(&name);
        self.files
            .insert(name, Cursor::new(data.into_boxed_slice()));
        Ok(())
    }

    /// Removes an entry and returns its stored data.
    pub fn remove_entry(&mut self, name: &str) -> Option<Vec<u8>> {
        let removed = self.files.remove(name)?;
        self.raw_names.remove(name);
        self.modified(name);
        Some(removed.into_inner().into_vec())
    }

    /// The header extensions (`key=value`) in file order, including the prefix.
    pub fn header_extensions(&self) -> impl Iterator<Item = (&str, &str)> {
        self.header_extensions
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The value of a header extension.
    #[must_use]
    pub fn header_extension(&self, key: &str) -> Option<&str> {
        self.header_extensions.get(key).map(String::as_str)
    }

    /// Sets a header extension; a repeated key keeps its position and takes the new value.
    pub fn set_header_extension(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let (key, value) = (key.into(), value.into());
        // the stored bytes stay in step with the extensions; the new value is stored as UTF-8
        if self.raw_pairs.len() == self.header_extensions.len() {
            match self.header_extensions.keys().position(|k| *k == key) {
                Some(at) => {
                    if let Some(pair) = self.raw_pairs.get_mut(at) {
                        pair.1 = value.as_bytes().to_vec();
                    }
                }
                None => self
                    .raw_pairs
                    .push((key.as_bytes().to_vec(), value.as_bytes().to_vec())),
            }
        } else {
            self.raw_pairs.clear();
        }
        self.checksum = None;
        self.computed_checksum = None;
        set_extension(&mut self.header_extensions, &key, value);
    }

    /// The PBO prefix (the header extension `prefix`).
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        self.header_extension("prefix")
    }

    /// The SHA-1 trailer of a PBO that was read and not modified; `None` otherwise.
    #[must_use]
    pub fn checksum(&self) -> Option<&[u8]> {
        self.checksum.as_deref()
    }

    /// The SHA-1 of the bytes of a PBO that was read and not modified (header block and entry
    /// data, everything before the trailer); `None` otherwise. A PBO whose stored
    /// [`checksum`](PBO::checksum) differs from it is corrupt or was tampered with.
    #[must_use]
    pub fn computed_checksum(&self) -> Option<&[u8]> {
        self.computed_checksum.as_deref()
    }

    /// A stored entry changes: the stored trailer no longer describes the PBO, and the header of
    /// the entry (packing method) no longer describes its data.
    fn modified(&mut self, name: &str) {
        self.checksum = None;
        self.computed_checksum = None;
        self.headers.retain(|h| h.filename != name);
    }

    /// The entries in header order (read, unmodified PBO) or write order (any other).
    fn ordered_files(&self) -> Vec<(&str, &Cursor<Box<[u8]>>)> {
        let mut files: Vec<(&str, &Cursor<Box<[u8]>>)> =
            self.files.iter().map(|(a, b)| (a.as_str(), b)).collect();
        if self.checksum.is_none() {
            files.sort_by_cached_key(|f| self.name_bytes(f.0).to_ascii_lowercase());
        }
        files
    }

    /// The SHA-1 of the PBO: the one computed while reading for a PBO that was read (an error if
    /// the stored trailer differs; `signing` adds the advice to repack), the one [`PBO::write`] would produce otherwise (nothing is
    /// kept in memory or on disk for it).
    #[cfg(feature = "sign")]
    pub(crate) fn trailer_checksum(&self, signing: bool) -> Result<Vec<u8>, Error> {
        match (&self.checksum, &self.computed_checksum) {
            (Some(stored), Some(computed)) => {
                if stored == computed {
                    Ok(computed.clone())
                } else {
                    Err(error!(
                        "PBO checksum mismatch: the stored SHA-1 trailer does not match the SHA-1 of the PBO's contents (the PBO is corrupt or was modified){}.",
                        if signing {
                            "; repack it before signing"
                        } else {
                            ""
                        }
                    ))
                }
            }
            _ => self.write_with_checksum(&mut std::io::sink()),
        }
    }

    /// The entry headers of a PBO that was read, in header order (empty for one made by
    /// [`PBO::from_directory`]).
    #[must_use]
    pub fn entry_headers(&self) -> Vec<EntryHeader> {
        self.headers
            .iter()
            .map(|h| EntryHeader {
                name: h.filename.clone(),
                packing_method: h.packing_method,
                original_size: h.original_size,
                data_size: h.data_size,
                timestamp: h.timestamp,
                name_bytes: h
                    .raw_filename
                    .clone()
                    .unwrap_or_else(|| h.filename.as_bytes().to_vec()),
            })
            .collect()
    }

    /// Returns the bytes an entry name is stored as in the PBO header.
    #[must_use]
    pub fn name_bytes(&self, name: &str) -> Vec<u8> {
        self.raw_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.as_bytes().to_vec())
    }

    /// The header extensions as `(key, value)` bytes, in the order of [`PBO::header_extensions`]:
    /// what the PBO header stores for a PBO that was read (a value set since is its UTF-8 bytes),
    /// the UTF-8 bytes of the strings otherwise. A repeated key has the bytes of its
    /// first key and its last value.
    #[must_use]
    pub fn header_extensions_bytes(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        if self.raw_pairs.len() == self.header_extensions.len() {
            return self.raw_pairs.clone();
        }
        self.header_extensions()
            .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
            .collect()
    }

    /// The bytes the prefix is stored as: the value of the extension `prefix`.
    pub(crate) fn prefix_bytes(&self) -> Option<Vec<u8>> {
        let at = self.header_extensions.keys().position(|k| k == "prefix")?;
        self.header_extensions_bytes()
            .into_iter()
            .nth(at)
            .map(|(_, value)| value)
    }

    /// Reads an existing PBO from input.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not a valid PBO.
    pub fn read<I: Read>(input: &mut I) -> Result<PBO, Error> {
        let mut hashing = HashingReader {
            inner: input,
            hash: Sha1::new(),
        };
        let input = &mut hashing;
        let HeaderBlock {
            extensions: header_extensions,
            raw_pairs,
            headers,
        } = read_header_block(input)?;

        let mut files: LinkedHashMap<String, Cursor<Box<[u8]>>> = LinkedHashMap::new();
        let mut raw_names: HashMap<String, Vec<u8>> = HashMap::new();
        for header in &headers {
            // two names that decode alike are one entry whose data is the last one's: keep that name's bytes
            match &header.raw_filename {
                Some(raw) => {
                    raw_names.insert(header.filename.clone(), raw.clone());
                }
                None => {
                    raw_names.remove(&header.filename);
                }
            }
            // never preallocate the full length from untrusted input
            let mut buffer: Vec<u8> = Vec::new();
            let got = input
                .take(u64::from(header.data_size))
                .read_to_end(&mut buffer)?;
            if got != header.data_size as usize {
                return Err(error!(
                    "PBO entry \"{}\" is truncated ({} of {} bytes).",
                    printable(&header.filename),
                    got,
                    header.data_size
                ));
            }
            files.insert(
                header.filename.clone(),
                Cursor::new(buffer.into_boxed_slice()),
            );
        }

        // the trailer's SHA-1 covers everything before the 0 byte that precedes it
        let computed_checksum = hashing.hash.clone().finalize().to_vec();
        let input = hashing.inner;
        let mut skip = [0u8; 1];
        let _ = input.read(&mut skip)?;
        let mut checksum = vec![0; 20];
        input.read_exact(&mut checksum)?;

        Ok(PBO {
            files,
            header_extensions,
            raw_names,
            raw_pairs,
            headers,
            checksum: Some(checksum),
            computed_checksum: Some(computed_checksum),
        })
    }

    /// Reads only the header block of a PBO: the header extensions and the entry names, in file order.
    /// The entry data is never read, so this is cheap for PBOs of any size. Pass a buffered reader.
    ///
    /// The input is untrusted: every name is limited to `MAX_HEADER_STRING` bytes, the number of entries
    /// and extensions are limited, and nothing is allocated from a size in the file.
    ///
    /// # Errors
    ///
    /// Returns an error if the header block is truncated, malformed or exceeds the limits.
    /// The extensions are `(key, value)` pairs in file order.
    pub fn read_headers<I: Read>(input: &mut I) -> Result<HeaderSummary, Error> {
        let block = read_header_block(input)?;
        let names = block.headers.into_iter().map(|h| h.filename).collect();
        Ok((block.extensions.into_iter().collect(), names))
    }

    /// Reads the header block of a seekable PBO and only the entries `wanted` accepts, by seeking to their
    /// data. Memory use is bounded by the selected entries, not by the size of the PBO, which can be many
    /// hundred MB.
    ///
    /// The input is untrusted: the header block is bounded like `read_headers`, and an entry whose data
    /// selected for reading does not lie inside the file is an error (nothing is allocated from a size before it is checked
    /// against the file length).
    ///
    /// # Errors
    ///
    /// Returns an error if the header block is malformed or exceeds the limits, or the data of a selected
    /// entry is truncated.
    pub fn read_entries<I: Read + Seek>(
        input: &mut I,
        wanted: impl Fn(&str) -> bool,
    ) -> Result<PboSelection, Error> {
        let len = input.seek(SeekFrom::End(0))?;
        input.seek(SeekFrom::Start(0))?;
        let mut reader = BufReader::new(&mut *input);
        let block = read_header_block(&mut reader)?;
        let mut offset = reader.stream_position()?;
        let mut entries = Vec::new();
        for h in &block.headers {
            let size = u64::from(h.data_size);
            let end = offset
                .checked_add(size)
                .ok_or_else(|| error!("PBO entry sizes overflow."))?;
            if wanted(&h.filename) {
                if end > len {
                    return Err(error!(
                        "PBO entry \"{}\" is truncated (its data ends past the end of the file).",
                        printable(&h.filename)
                    ));
                }
                reader.seek(SeekFrom::Start(offset))?;
                let mut data: Vec<u8> = Vec::new();
                (&mut reader).take(size).read_to_end(&mut data)?;
                let data = if h.packing_method == PACKING_COMPRESSED {
                    lzss_decompress(&data, h.original_size as usize).prepend_error(format!(
                        "Failed to decompress \"{}\":",
                        printable(&h.filename)
                    ))
                } else {
                    Ok(data)
                };
                entries.push(SelectedEntry {
                    name: h.filename.clone(),
                    data,
                });
            }
            offset = end;
        }
        Ok(PboSelection {
            header_extensions: block.extensions,
            names: block.headers.into_iter().map(|h| h.filename).collect(),
            entries,
        })
    }

    /// Constructs a PBO from a directory, binarizing and rapifying where `options.build` is set.
    ///
    /// `options.exclude` contains glob patterns to exclude from the PBO, `options.include_folders`
    /// the paths to search recursively for absolute includes and `options.direct_include_folders` the paths
    /// checked only directly (generally the current working directory). `options.header_extensions` are applied after `$PBOPREFIX$`; for the same key
    /// the option wins. A `$NOBIN$` file in the directory turns `build` off. Models are only
    /// binarized when a `binarizer` is given; without one they are copied as-is with a warning.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read or a file cannot be processed.
    #[expect(
        clippy::too_many_lines,
        reason = "one branch per file kind keeps the packing loop readable"
    )]
    pub fn from_directory(
        directory: &Path,
        options: &PackOptions,
        mut binarizer: Option<&mut Binarizer>,
        warnings: &mut dyn WarningSink,
    ) -> Result<PBO, Error> {
        let mut binarize = options.build;
        let exclude_patterns = &options.exclude;
        let includefolders = IncludeFolders::new(options.include_folders.clone())
            .with_direct(options.direct_include_folders.clone());
        let search_roots: Vec<PathBuf> = options
            .include_folders
            .iter()
            .chain(&options.direct_include_folders)
            .cloned()
            .collect();
        let file_list = list_files(directory, &excluded_paths(options))?;
        let mut files: LinkedHashMap<String, Cursor<Box<[u8]>>> = LinkedHashMap::new();
        let mut header_extensions: LinkedHashMap<String, String> = LinkedHashMap::new();

        if directory.join("$NOBIN$").exists() || directory.join("$NOBIN-NOTEST$").exists() {
            binarize = false;
        }

        // the prefix is needed while binarizing, so read it before looking at the other files
        let mut pre_extensions: LinkedHashMap<String, String> = LinkedHashMap::new();
        for prefix_file in &["$PBOPREFIX$", "$PREFIX$"] {
            let prefix_path = directory.join(prefix_file);
            // a FIFO or device named like the prefix file would block the read; the walk below
            // reports it
            if !std::fs::metadata(&prefix_path).is_ok_and(|m| m.file_type().is_file()) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(prefix_path) {
                parse_prefix_file(&content, &mut pre_extensions, false, warnings);
            }
        }
        let prefix: String = pre_extensions
            .get("prefix")
            .cloned()
            .unwrap_or_else(|| default_prefix(directory).unwrap_or_default());

        let addon_files: HashSet<String> = file_list
            .iter()
            .filter_map(|p| p.strip_prefix(directory).ok())
            .filter_map(|p| p.to_str())
            .map(|p| p.replace('/', "\\").to_lowercase())
            .collect();

        let addon_roots = addon_roots(directory);
        let mut seen_names: HashMap<String, String> = HashMap::new();

        // Entries generated from source (the rapified config.cpp, a .p3do packed as .p3d) win over a
        // plain file of the same name, so find them first: the plain file is then skipped before it
        // is binarized or read. The gates are the ones of the loop below.
        // key -> (source file, stored entry name)
        let mut generated_names: HashMap<String, (String, String)> = HashMap::new();
        for path in &file_list {
            let Ok(relative) = path.strip_prefix(directory) else {
                continue;
            };
            let (pre_name, name, generated) = entry_name(relative, binarize);
            if !generated
                || !file_allowed(&pre_name, exclude_patterns)
                || std::fs::metadata(path).is_ok_and(|m| !m.file_type().is_file())
                || relative.to_str().is_none()
                || safe_relative_path(&name).is_none()
            {
                continue;
            }
            generated_names
                .entry(name.to_lowercase())
                .or_insert_with(|| (printable(&relative.to_string_lossy()), printable(&name)));
        }

        for path in file_list {
            let Ok(mut relative) = path.strip_prefix(directory).map(Path::to_path_buf) else {
                continue;
            };
            let shown = printable(&relative.to_string_lossy());
            let (pre_name, name, generated) = entry_name(&relative, binarize);
            if binarize && relative.file_name() == Some(OsStr::new("config.cpp")) {
                relative = relative.with_file_name("config.bin");
            }
            // excluded files are silent, whatever they are
            if !file_allowed(&pre_name, exclude_patterns) {
                continue;
            }
            // only regular files (or links to them): a FIFO would block the build, a device never
            // ends. A path that cannot be inspected falls through to the open error below.
            if std::fs::metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
                warnings.warn(
                    Warning::new(
                        Some("unsupported-file"),
                        "Skipping a file that is not a regular file (FIFO, device or socket).",
                    )
                    .in_file(relative.to_string_lossy()),
                );
                continue;
            }
            if relative.to_str().is_none() || safe_relative_path(&pre_name).is_none() {
                warnings.warn(Warning::new(
                    Some("unsafe-path"),
                    format!("Skipping \"{shown}\": unsafe entry name."),
                ));
                continue;
            }
            // a .p3do is packed as .p3d (it is not binarized itself, so decide that first)
            let is_binarizable = BINARIZABLE_RE.is_match(&pre_name);
            // entry names compare case-insensitively; the first file in walk order wins, except
            // that an entry generated from source replaces a plain file of the same name (found
            // by the pre-pass above, so the plain file is skipped before it is processed)
            let key = name.to_lowercase();
            if !generated && let Some((source, stored)) = generated_names.get(&key) {
                warnings.warn(Warning::new(
                    Some("duplicate-entry"),
                    format!(
                        "Skipping \"{shown}\": the entry \"{stored}\" is generated from \"{source}\"."
                    ),
                ));
                continue;
            }
            if let Some(first) = seen_names.get(&key) {
                warnings.warn(Warning::new(
                    Some("duplicate-entry"),
                    format!(
                        "Skipping \"{shown}\": an entry named \"{}\" was already packed.",
                        printable(first)
                    ),
                ));
                continue;
            }
            seen_names.insert(key, name.clone());

            let mut file =
                File::open(&path).prepend_error(format!("Failed to open \"{shown}\":"))?;

            let prebinarized_as = if binarize && is_binarizable {
                binarize::binarized_description(&path)?
            } else {
                None
            };
            let prebinarized = prebinarized_as.is_some();
            if let Some(description) = prebinarized_as {
                warnings.warn(
                    Warning::new(
                        Some("prebinarized-file"),
                        format!("File is already binarized ({description}); copying it as-is."),
                    )
                    .in_file(relative.to_string_lossy()),
                );
            } else if binarize
                && is_binarizable
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("p3d"))
            {
                p3d::check_dependencies(
                    &path,
                    &relative.to_string_lossy(),
                    &prefix,
                    &addon_files,
                    &search_roots,
                    warnings,
                );
            }

            if name == "$PBOPREFIX$" || name == "$PREFIX$" {
                let mut content = String::new();
                file.read_to_string(&mut content)
                    .prepend_error(format!("Failed to read \"{shown}\":"))?;
                parse_prefix_file(&content, &mut header_extensions, true, warnings);
            } else if binarize
                && ["cpp", "rvmat"]
                    .contains(&path.extension().and_then(|e| e.to_str()).unwrap_or(""))
            {
                let mut buffer: Vec<u8> = Vec::new();
                file.read_to_end(&mut buffer)
                    .prepend_error(format!("Failed to read \"{shown}\":"))?;

                if is_rapified(&buffer) {
                    warnings.warn(
                        Warning::new(
                            Some("prerapified-file"),
                            "File is already rapified; copying it as-is.",
                        )
                        .in_file(relative.to_string_lossy()),
                    );
                    files.insert(name, Cursor::new(buffer.into_boxed_slice()));
                } else {
                    let mut addon_warnings = AddonRelative {
                        roots: &addon_roots,
                        inner: warnings,
                    };
                    let config = Config::read_in(
                        &mut Cursor::new(buffer),
                        Some(path.clone()),
                        &includefolders,
                        &mut addon_warnings,
                    )
                    .prepend_error("Failed to parse config:")?;
                    files.insert(name, config.to_cursor()?);
                }
            } else if let (Some(b), true) = (
                binarizer.as_deref_mut(),
                binarize && is_binarizable && !prebinarized,
            ) {
                let data = b
                    .binarize(&path, directory, &prefix, warnings)
                    .prepend_error(format!(
                        "Failed to binarize {}:",
                        printable(&relative.display().to_string())
                    ))?;

                files.insert(name, Cursor::new(data.into_boxed_slice()));
            } else {
                if binarize
                    && is_binarizable
                    && !prebinarized
                    && binarizer.is_none()
                    && !cfg!(windows)
                {
                    warnings.warn(
                        Warning::new(
                            Some("non-windows-binarization"),
                            "On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is.",
                        )
                        .in_file(relative.to_string_lossy()),
                    );
                }

                let mut buffer: Vec<u8> = Vec::new();
                file.read_to_end(&mut buffer)
                    .prepend_error(format!("Failed to read \"{shown}\":"))?;

                files.insert(name, Cursor::new(buffer.into_boxed_slice()));
            }
        }

        if !header_extensions.contains_key("prefix") {
            let prefix: String = default_prefix(directory).ok_or_else(|| {
                error!(
                    "Invalid addon directory \"{}\".",
                    printable(&directory.display().to_string())
                )
            })?;
            header_extensions.insert("prefix".to_string(), prefix);
        }

        for (key, value) in &options.header_extensions {
            set_extension(&mut header_extensions, key, value.clone());
        }

        Ok(PBO {
            files,
            header_extensions,
            raw_names: HashMap::new(),
            raw_pairs: Vec::new(),
            headers: Vec::new(),
            checksum: None,
            computed_checksum: None,
        })
    }

    /// Writes PBO to output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure, if an entry is too large, or if a header extension key
    /// or value contains a NUL byte.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        self.write_with_checksum(output).map(|_| ())
    }

    /// Writes the PBO and returns the SHA-1 it ends with.
    fn write_with_checksum<O: Write>(&self, output: &mut O) -> Result<Vec<u8>, Error> {
        let mut headers: Cursor<Vec<u8>> = Cursor::new(Vec::new());

        let ext_header = PBOHeader {
            filename: String::new(),
            raw_filename: None,
            packing_method: 0x5665_7273,
            original_size: 0,
            reserved: 0,
            timestamp: 0,
            data_size: 0,
        };
        ext_header.write(&mut headers)?;

        if let Some(prefix) = self.prefix_bytes() {
            headers.write_all(b"prefix\0")?;
            headers
                .write_cstring(prefix)
                .prepend_error("Header extension \"prefix\":")?;
        }

        for ((key, _), (key_bytes, value_bytes)) in
            self.header_extensions().zip(self.header_extensions_bytes())
        {
            if key == "prefix" {
                continue;
            }

            headers
                .write_cstring(key_bytes)
                .and_then(|()| headers.write_cstring(value_bytes))
                .prepend_error(format!("Header extension \"{}\":", printable(key)))?;
        }
        headers.write_cstring(String::new())?;

        let mut files_sorted: Vec<(&str, &Cursor<Box<[u8]>>)> =
            self.files.iter().map(|(a, b)| (a.as_str(), b)).collect();
        files_sorted.sort_by_cached_key(|f| self.name_bytes(f.0).to_ascii_lowercase());

        for (name, cursor) in &files_sorted {
            let size = u32::try_from(cursor.get_ref().len())
                .map_err(|_| error!("PBO entry \"{}\" is too large.", printable(name)))?;
            // an entry that was read and not replaced since keeps its packing method (compressed or
            // one we do not know): `files` holds its stored bytes, so the header must still say
            // how to decode them
            let kept = self
                .headers
                .iter()
                .rfind(|h| h.filename == *name)
                .filter(|h| h.packing_method != 0);
            let header = PBOHeader {
                filename: (*name).to_string(),
                raw_filename: self.raw_names.get(*name).cloned(),
                packing_method: kept.map_or(0, |h| h.packing_method),
                original_size: kept.map_or(size, |h| h.original_size),
                reserved: 0,
                timestamp: 0,
                data_size: size,
            };

            header.write(&mut headers)?;
        }

        let header = PBOHeader {
            packing_method: 0,
            ..ext_header
        };
        header.write(&mut headers)?;

        let mut h = Sha1::new();

        output.write_all(headers.get_ref())?;
        h.update(headers.get_ref());

        for (_, cursor) in &files_sorted {
            output.write_all(cursor.get_ref())?;
            h.update(cursor.get_ref());
        }

        output.write_all(&[0])?;
        let checksum = h.finalize();
        output.write_all(&checksum)?;

        Ok(checksum.to_vec())
    }

    /// Returns the contents of a file, decompressing it if the PBO stores it compressed.
    /// `files` keeps the stored bytes, because signing hashes those.
    ///
    /// # Errors
    ///
    /// Returns an error if a compressed entry cannot be decompressed.
    pub fn file_data(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(cursor) = self.files.get(name) else {
            return Ok(None);
        };

        // duplicate names: `files` kept the last value, so use the last header
        match self.headers.iter().rfind(|h| h.filename == name) {
            Some(h) if h.packing_method == PACKING_COMPRESSED => {
                lzss_decompress(cursor.get_ref(), h.original_size as usize)
                    .prepend_error(format!("Failed to decompress \"{}\":", printable(name)))
                    .map(Some)
            }
            _ => Ok(Some(cursor.get_ref().to_vec())),
        }
    }

    /// Returns the PBO as a `Cursor`.
    ///
    /// # Errors
    ///
    /// Returns an error if the PBO cannot be written.
    pub fn to_cursor(&self) -> Result<Cursor<Vec<u8>>, Error> {
        let mut cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        self.write(&mut cursor)?;

        cursor.seek(SeekFrom::Start(0))?;

        Ok(cursor)
    }
}

fn list_files(directory: &Path, excluded: &[Excluded]) -> Result<Vec<PathBuf>, Error> {
    let mut files: Vec<PathBuf> = Vec::new();

    // read_dir order depends on the file system; sort by file name so warnings come in a fixed order
    let mut paths = read_dir(directory)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<PathBuf>, _>>()?;
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    for path in paths {
        if path.is_dir() {
            for f in list_files(&path, excluded)? {
                files.push(f);
            }
        } else if !is_excluded(&path, excluded) {
            files.push(path);
        }
    }

    Ok(files)
}

/// The entry names a source file packs as in `build`: the name before the `.p3do` rename (what `-x`
/// matches), the final one, and whether the entry is generated from source (the rapified
/// `config.cpp`, a `.p3do` packed as `.p3d`).
fn entry_name(relative: &Path, binarize: bool) -> (String, String, bool) {
    let mut relative = relative.to_path_buf();
    let mut generated = false;
    if binarize && relative.file_name() == Some(OsStr::new("config.cpp")) {
        relative = relative.with_file_name("config.bin");
        generated = true;
    }
    let pre = relative.to_string_lossy().replace('/', "\\");
    let name = if P3DO_RE.is_match(&pre) {
        generated = true;
        P3DO_RE.replace(&pre, ".p3d").into_owned()
    } else {
        pre.clone()
    };
    (pre, name, generated)
}

/// Whether a path component names a Windows device (`CON`, `NUL.txt`, `COM1`, `COM\u{b9}`, `LPT0`, `CONIN$`,
/// ...), which Windows opens instead of a file whatever the extension and case.
#[must_use]
pub fn is_device_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p).is_some_and(|n| {
            let mut chars = n.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
            )
        })
    })
}

/// Maps an untrusted PBO entry name or prefix to a relative path that cannot leave the folder it
/// is joined to.
///
/// The name is split on `\` and `/`. Returns `None` if any component is empty (a leading,
/// trailing or doubled separator, so an absolute name is never accepted), is `.` or `..`, consists
/// only of dots and spaces, is a Windows device name, or contains `:` (drive letters), a NUL or any
/// control character (C0, DEL and C1, `char::is_control`), or if the name is empty.
#[must_use]
pub fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for component in name.split(['\\', '/']) {
        if component.is_empty()
            || is_device_name(component)
            || component.chars().all(|c| c == '.' || c == ' ')
            || component.chars().any(|c| c == ':' || c.is_control())
        {
            return None;
        }
        path.push(component);
    }
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Returns true if `base` joined with `relative` would pass through an existing symlink
/// (checking every component below `base`, including the final one), or if that cannot be
/// determined.
fn crosses_symlink(base: &Path, relative: &Path) -> bool {
    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match current.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            // nothing exists from here on, so nothing below can be a symlink
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            // the path cannot be inspected (permissions, I/O error): do not assume it is safe
            Err(_) => return true,
        }
    }
    false
}

/// The prefix of an addon folder without `$PBOPREFIX$`: its own name (a symlink keeps the name it is
/// reached by), or for `.`, `./` and `..` the name of the real folder.
fn default_prefix(directory: &Path) -> Option<String> {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    name(directory).or_else(|| directory.canonicalize().ok().and_then(|d| name(&d)))
}

/// Replaces control characters (line breaks included) with a space.
fn sanitize_header(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Writes `$PBOPREFIX$` into `base` with control characters removed from keys and values, unless
/// it is a symlink.
fn write_prefix_file(
    extensions: &LinkedHashMap<String, String>,
    base: &Path,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let prefix_path = base.join("$PBOPREFIX$");
    if crosses_symlink(base, Path::new("$PBOPREFIX$")) {
        warnings.warn(Warning::new(
            Some("unsafe-path"),
            "Not writing $PBOPREFIX$: it is a symlink.",
        ));
        return Ok(());
    }
    let mut prefix_file =
        File::create(prefix_path).prepend_error("Failed to create prefix file:")?;

    for (key, value) in extensions {
        let clean_key = sanitize_header(key);
        let clean_value = sanitize_header(value);
        if clean_key != *key || clean_value != *value {
            warnings.warn(Warning::new(Some("unsafe-header"), format!(
                    "Header extension {key:?} contains control characters; replaced them with spaces."
                )));
        }
        prefix_file
            .write_all(format!("{clean_key}={clean_value}\n").as_bytes())
            .prepend_error("Failed to write prefix file:")?;
    }
    Ok(())
}

/// The relative path an unpacked entry is written to, and whether leading separators were
/// stripped from its name (`\scripts\x.c` is written as `scripts/x.c`). Everything else
/// [`safe_relative_path`] rejects stays rejected.
fn unpack_relative(name: &str) -> Option<(PathBuf, bool)> {
    let stripped = name.trim_start_matches(['\\', '/']);
    safe_relative_path(stripped).map(|relative| (relative, stripped.len() != name.len()))
}

/// Warns that an entry name starts with a separator that was stripped.
fn warn_leading_separator(name: &str, relative: &Path, warnings: &mut dyn WarningSink) {
    warnings.warn(Warning::new(
        Some("leading-separator"),
        format!(
            "Entry \"{}\" starts with a path separator; extracting it as \"{}\".",
            printable(name),
            printable(&relative.to_string_lossy())
        ),
    ));
}

/// The folder a PBO is unpacked into: `output`, or with `use_prefix` `<output>/<prefix>`. An empty
/// prefix (or one of only separators) means no prefix.
fn unpack_base(pbo: &PBO, output: &Path, use_prefix: bool) -> Result<PathBuf, Error> {
    if !use_prefix {
        return Ok(output.to_path_buf());
    }
    match pbo.header_extensions.get("prefix") {
        Some(prefix) if prefix.split(['\\', '/']).all(str::is_empty) => Ok(output.to_path_buf()),
        Some(prefix) => {
            // other tools may write a trailing separator; a leading or doubled one is rejected
            let relative = safe_relative_path(prefix.trim_end_matches(['\\', '/']))
                .ok_or_else(|| error!("Unsafe PBO prefix {prefix:?}; refusing to unpack."))?;
            if crosses_symlink(output, &relative) {
                return Err(error!(
                    "PBO prefix {prefix:?} goes through a symlink; refusing to unpack."
                ));
            }
            Ok(output.join(relative))
        }
        None => Ok(output.to_path_buf()),
    }
}

/// How [`unpack`] treats existing files and the PBO prefix.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct UnpackOptions {
    /// Overwrite existing files instead of failing.
    pub force: bool,
    /// Extract into `<target>/<prefix>/` instead of `<target>/`.
    pub use_prefix: bool,
    /// Glob patterns of entries to leave out; see [`EntryFilter`]. An entry that matches one is
    /// never written, even when it matches `only`.
    pub exclude: Vec<String>,
    /// Glob patterns of the entries to write; see [`EntryFilter`]. Empty means every entry.
    pub only: Vec<String>,
}

/// Selects PBO entries by glob pattern: an entry passes when it matches at least one `only` pattern (or
/// `only` is empty) and no `exclude` pattern.
///
/// `*` stands for any run of characters, as in the `-x` patterns of `pack`. The name is matched with
/// `/` separators (a `\` is turned into `/`), without the PBO prefix, and ignoring case, like PBO names.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EntryFilter {
    only: Vec<String>,
    exclude: Vec<String>,
}

impl EntryFilter {
    /// A filter for the `only` and `exclude` patterns. The patterns are normalised once (lowercase,
    /// `\` turned into `/`).
    #[must_use]
    pub fn new(only: Vec<String>, exclude: Vec<String>) -> Self {
        let normalise = |patterns: Vec<String>| {
            patterns
                .into_iter()
                .map(|p| p.replace('\\', "/").to_lowercase())
                .collect()
        };
        Self {
            only: normalise(only),
            exclude: normalise(exclude),
        }
    }

    /// Whether the entry `name` passes the filter.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        let name = name.replace('\\', "/").to_lowercase();
        let hit = |pattern: &String| matches_glob(&name, pattern);
        (self.only.is_empty() || self.only.iter().any(hit)) && !self.exclude.iter().any(hit)
    }
}

/// Fails when a selected file of `pbo` (or `$PBOPREFIX$`) already exists below `base`. Runs before anything
/// is written, so a conflict leaves the target untouched.
fn check_unpack_conflicts(pbo: &PBO, base: &Path, filter: &EntryFilter) -> Result<(), Error> {
    let prefix = (!pbo.header_extensions.is_empty()).then_some("$PBOPREFIX$");
    let first = prefix
        .into_iter()
        .chain(
            pbo.files
                .keys()
                .map(String::as_str)
                .filter(|name| filter.matches(name)),
        )
        .find(|name| {
            unpack_relative(name).is_some_and(|(relative, _)| {
                !crosses_symlink(base, &relative) && base.join(relative).exists()
            })
        });
    match first {
        Some(name) => Err(error!(
            "Target folder \"{}\" already contains files of this PBO (first: \"{}\"). Use --force to overwrite.",
            printable(&base.display().to_string()),
            printable(name)
        )),
        None => Ok(()),
    }
}

/// Unpacks a PBO into the target folder.
///
/// Entry names and the prefix come from the (untrusted) PBO: entries whose name does not map to a
/// safe relative path, that would be written through a symlink, or that collide (case-insensitively)
/// with an entry already written in this run are skipped with a warning. Leading separators are
/// stripped from a name (`leading-separator` warning). With `options.use_prefix`
/// the files are extracted into `<target>/<prefix>/`; an empty prefix means no prefix. `options.only` and
/// `options.exclude` select the entries to write (`$PBOPREFIX$` is always written).
///
/// # Errors
///
/// Returns an error if the PBO cannot be read, the prefix is unsafe with `use_prefix`, or, without
/// `options.force`, a selected file of the PBO (or `$PBOPREFIX$`) already exists in the target. That
/// check covers the selected entries plus `$PBOPREFIX$` and runs before anything is written, so a
/// conflict leaves the target untouched.
pub fn unpack<I: Read>(
    input: &mut I,
    target: &Path,
    options: &UnpackOptions,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let UnpackOptions {
        force,
        use_prefix,
        ref exclude,
        ref only,
    } = *options;
    let filter = EntryFilter::new(only.clone(), exclude.clone());
    let output = target;
    let pbo = PBO::read(input).prepend_error("Failed to read PBO:")?;

    let base = unpack_base(&pbo, output, use_prefix)?;

    if !force {
        check_unpack_conflicts(&pbo, &base, &filter)?;
    }

    create_dir_all(&base).prepend_error("Failed to create output folder:")?;

    if !pbo.header_extensions.is_empty() {
        write_prefix_file(&pbo.header_extensions, &base, warnings)?;
    }

    let mut written: HashSet<String> = HashSet::new();
    for file_name in pbo.files.keys() {
        if !filter.matches(file_name) {
            continue;
        }
        if pbo.raw_names.contains_key(file_name) {
            warnings.warn(Warning::new(
                Some("non-utf8-filename"),
                format!(
                    "Entry name {file_name:?} is not valid UTF-8; extracting it as windows-1252."
                ),
            ));
        }
        let Some((relative, stripped)) = unpack_relative(file_name) else {
            warnings.warn(Warning::new(
                Some("unsafe-path"),
                format!("Skipping \"{}\": unsafe path.", printable(file_name)),
            ));
            continue;
        };
        let key = relative.to_string_lossy().to_lowercase();
        if written.contains(&key) {
            warnings.warn(Warning::new(
                Some("duplicate-entry"),
                format!(
                    "Skipping \"{}\": an entry with the same path was already extracted.",
                    printable(file_name)
                ),
            ));
            continue;
        }
        if crosses_symlink(&base, &relative) {
            warnings.warn(Warning::new(
                Some("unsafe-path"),
                format!(
                    "Skipping \"{}\": the target path goes through a symlink.",
                    printable(file_name)
                ),
            ));
            continue;
        }
        let data = match pbo.file_data(file_name) {
            Ok(Some(data)) => data,
            Ok(None) => continue,
            Err(e) => {
                warnings.warn(Warning::new(
                    None,
                    format!("Skipping \"{}\": {e}", printable(file_name)),
                ));
                continue;
            }
        };
        if stripped {
            warn_leading_separator(file_name, &relative, warnings);
        }
        let path = base.join(&relative);
        written.insert(key);
        let written_ok = create_dir_all(path.parent().unwrap_or(&base))
            .prepend_error("Failed to create output folder:")
            .and_then(|()| File::create(&path).prepend_error("Failed to open output file:"))
            .and_then(|mut file| {
                file.write_all(&data)
                    .prepend_error("Failed to write output file:")
            });
        if let Err(e) = written_ok {
            warnings.warn(Warning::new(
                None,
                format!("Skipping \"{}\": {e}", printable(file_name)),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    /// #62: the full list of Windows device names, in any case, with and without an extension.
    #[test]
    fn device_names_are_complete() {
        for name in [
            "CON",
            "PRN",
            "AUX",
            "NUL",
            "COM0",
            "COM1",
            "COM9",
            "LPT0",
            "LPT1",
            "LPT9",
            "COM\u{b9}",
            "COM\u{b2}",
            "COM\u{b3}",
            "LPT\u{b9}",
            "LPT\u{b2}",
            "LPT\u{b3}",
            "CONIN$",
            "CONOUT$",
        ] {
            for variant in [
                name.to_string(),
                name.to_lowercase(),
                format!("{name}.txt"),
                format!("{}.cfg", name.to_lowercase()),
                format!("{name} .x"),
            ] {
                assert!(super::is_device_name(&variant), "{variant}");
                assert!(
                    super::safe_relative_path(&format!("a\\{variant}")).is_none(),
                    "{variant}"
                );
            }
        }
        for name in [
            "COM",
            "COM10",
            "COM\u{b9}\u{b9}",
            "CONIN",
            "CONOUT",
            "CONIN$$",
            "LPT",
            "COMA",
            "ACON",
            "x.CON",
        ] {
            assert!(!super::is_device_name(name), "{name}");
        }
    }

    use super::*;

    fn p(s: &str) -> Option<PathBuf> {
        safe_relative_path(s)
    }

    #[test]
    fn glob_matches_without_panicking() {
        assert!(matches_glob("", "*"));
        assert!(!matches_glob("ab", "abc*"));
        assert!(!matches_glob("é", "*x"));
        assert!(matches_glob("abc", "a*c"));
        assert!(matches_glob("abc", "*"));
        assert!(matches_glob("abc", "abc"));
        assert!(!matches_glob("abd", "abc"));
        assert!(matches_glob("a.bin", "*.bin"));
        assert!(matches_glob("aXbYc", "a*b*c"));
    }

    #[test]
    fn lzss_size_bomb_is_an_error() {
        let err = lzss_decompress(&[0xFF; 16], 0xFFFF_FFF0).unwrap_err();
        assert!(err.to_string().contains("implausible"), "{err}");
        assert!(lzss_decompress(&[0xFF, b'a'], 9).is_err());
    }

    #[test]
    fn safe_path_accepts_normal_names() {
        assert_eq!(
            p("a\\b/c.txt"),
            Some(PathBuf::from("a").join("b").join("c.txt"))
        );
        assert_eq!(
            p("a.b\\.hidden"),
            Some(PathBuf::from("a.b").join(".hidden"))
        );
    }

    #[test]
    fn safe_path_rejects_empty_components() {
        for bad in [
            "\\abs.txt",
            "/abs",
            "\\abs",
            "/tmp//x",
            "a\\\\b",
            "a\\b\\",
            "a/",
            "\\\\?\\x",
        ] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn safe_path_rejects_dots_and_spaces() {
        for bad in [
            "..",
            "a\\..\\b",
            "a\\.\\b",
            "a\\...\\b",
            "a\\. \\b",
            "a\\ \\b",
            "a\\.. ",
        ] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn safe_path_rejects_colon_nul_and_control() {
        for bad in [
            "C:\\x.txt",
            "a:b",
            "a\0b",
            "a\nb",
            "a\tb",
            "a\x7fb",
            "a\u{80}b",
            "a\u{9b}b",
            "a\u{9f}",
        ] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn crosses_symlink_fails_closed_when_the_path_cannot_be_inspected() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // root ignores permissions, so there is nothing to test
        let permissions_ignored = locked
            .join("x")
            .symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
        let crosses = crosses_symlink(dir.path(), Path::new("locked/x"));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !permissions_ignored {
            assert!(crosses, "an unreadable path must count as unsafe");
        }
    }

    #[test]
    fn safe_path_rejects_windows_device_names() {
        for bad in ["con", "a\\NUL.txt", "Aux.tar.gz", "com1", "LPT9.x", "prn."] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
        for fine in ["console", "com10", "nulls.txt", "a.con"] {
            assert!(p(fine).is_some(), "{fine:?}");
        }
    }

    #[test]
    fn safe_path_rejects_empty() {
        for bad in ["", "\\", "/\\/"] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
    }
}
