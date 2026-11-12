use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{File, create_dir_all, read_dir};
use std::io::{Cursor, Error, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use linked_hash_map::LinkedHashMap;
use openssl::hash::{Hasher, MessageDigest};
use regex::Regex;
use std::sync::LazyLock;

#[expect(clippy::unwrap_used, reason = "the static patterns are valid regexes")]
static BINARIZABLE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(".(rtm|p3d)$").unwrap());
#[expect(clippy::unwrap_used, reason = "the static pattern is a valid regex")]
static P3DO_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(".p3do$").unwrap());

use crate::binarize;
use crate::config::{Config, is_rapified};
use crate::error::{ErrorExt, warning};
use crate::io::{ReadExt, WriteExt};
use crate::p3d;
use crate::preprocess::pathsep;

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
fn normalize_prefix(prefix: &str, warn: bool) -> String {
    let trimmed = prefix.trim();
    let normalized = trimmed.replace('/', "\\");
    let stripped = normalized.trim_matches('\\');
    if warn && stripped.len() != normalized.len() {
        warning(
            format!("Removed leading/trailing slashes from prefix {trimmed:?}."),
            Some("prefix-slash"),
            (None, None),
        );
    }
    stripped.to_string()
}

/// Parses `$PBOPREFIX$` into header extensions: a bare line is the prefix, `key=value` lines are
/// extensions. Prefix slashes are normalized (`warn` controls the `prefix-slash` warning).
fn parse_prefix_file(content: &str, header_extensions: &mut HashMap<String, String>, warn: bool) {
    for l in content.lines() {
        if l.is_empty() {
            break;
        }

        match l.split_once('=') {
            None => {
                header_extensions.insert("prefix".to_string(), normalize_prefix(l, warn));
            }
            Some((key, value)) if key == "prefix" => {
                header_extensions.insert(key.to_string(), normalize_prefix(value, warn));
            }
            Some((key, value)) => {
                header_extensions.insert(key.to_string(), value.to_string());
            }
        }
    }
}

/// PBO file
///
/// # Examples
///
/// ```
/// # use std::path::PathBuf;
/// # use dayzmake::pbo::PBO;
/// let pbo = PBO::from_directory(PathBuf::from("src"), false, &Vec::new(), &Vec::new()).expect("Failed to create PBO");
///
/// assert!(pbo.files.iter().any(|(name, _data)| name == "main.rs"));
///
/// let mut cursor = pbo.to_cursor().unwrap();
/// let reread = PBO::read(&mut cursor).unwrap();
///
/// assert!(reread.checksum.is_some());
/// ```
pub struct PBO {
    /// The PBO's files in header order, keyed by name.
    pub files: LinkedHashMap<String, Cursor<Box<[u8]>>>,
    /// Header extensions (`key=value`), including the prefix under the key `prefix`.
    pub header_extensions: HashMap<String, String>,
    /// Original bytes of entry names that are not valid UTF-8, keyed by their windows-1252 decoding
    /// (the key used in `files`).
    pub raw_names: HashMap<String, Vec<u8>>,
    /// Same as `raw_names`, for header extension keys and values.
    raw_extensions: HashMap<String, Vec<u8>>,
    headers: Vec<PBOHeader>,
    /// only defined when reading existing PBOs, for created PBOs this is calculated during writing
    /// and included in the output
    pub checksum: Option<Vec<u8>>,
}

impl PBOHeader {
    fn read<I: Read>(input: &mut I) -> Result<PBOHeader, Error> {
        let (filename, raw_filename) = decode_name(input.read_cstring_bytes()?);
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

/// Packing method of entries compressed with Bohemia's LZSS variant ("Cprs").
const PACKING_COMPRESSED: u32 = 0x4370_7273;

/// Decompresses a "Cprs" PBO entry: groups of 8 items led by a flag byte (LSB first),
/// bit set = literal byte, bit clear = 2-byte back reference (12 bit offset, 4 bit length - 3).
/// The window starts out filled with spaces; the 4 byte checksum after the data is ignored.
fn lzss_decompress(data: &[u8], original_size: usize) -> Result<Vec<u8>, Error> {
    let truncated = || error!("Truncated compressed PBO entry.");
    let mut out: Vec<u8> = Vec::with_capacity(original_size);
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

fn matches_glob(s: &str, pattern: &str) -> bool {
    if let Some(index) = pattern.find('*') {
        if s[..index] != pattern[..index] {
            return false;
        }

        for i in (index + 1)..(s.len() - 1) {
            if matches_glob(&s[i..], &pattern[(index + 1)..]) {
                return true;
            }
        }

        false
    } else {
        s == pattern
    }
}

fn file_allowed(name: &str, exclude_patterns: &[String]) -> bool {
    for pattern in exclude_patterns {
        if matches_glob(name, pattern) {
            return false;
        }
    }

    true
}

impl PBO {
    /// Returns the bytes an entry name is stored as in the PBO header.
    #[must_use]
    pub fn name_bytes(&self, name: &str) -> Vec<u8> {
        self.raw_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.as_bytes().to_vec())
    }

    /// Returns the bytes a header extension key or value is stored as.
    pub(crate) fn extension_bytes(&self, s: &str) -> Vec<u8> {
        self.raw_extensions
            .get(s)
            .cloned()
            .unwrap_or_else(|| s.as_bytes().to_vec())
    }

    /// Reads an existing PBO from input.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not a valid PBO.
    pub fn read<I: Read>(input: &mut I) -> Result<PBO, Error> {
        let mut headers: Vec<PBOHeader> = Vec::new();
        let mut first = true;
        let mut header_extensions: HashMap<String, String> = HashMap::new();
        let mut raw_extensions: HashMap<String, Vec<u8>> = HashMap::new();
        let mut decode_ext = |bytes: Vec<u8>| {
            let (s, raw) = decode_name(bytes);
            if let Some(raw) = raw {
                raw_extensions.insert(s.clone(), raw);
            }
            s
        };

        loop {
            let header = PBOHeader::read(input)?;
            // todo: garbage filter

            if header.packing_method == 0x5665_7273 {
                if !first {
                    return Err(error!("Duplicate version header in PBO."));
                }

                loop {
                    let s = decode_ext(input.read_cstring_bytes()?);
                    if s.is_empty() {
                        break;
                    }

                    let value = decode_ext(input.read_cstring_bytes()?);
                    header_extensions.insert(s, value);
                }
            } else if header.filename.is_empty() {
                break;
            } else {
                headers.push(header);
            }

            first = false;
        }

        let mut files: LinkedHashMap<String, Cursor<Box<[u8]>>> = LinkedHashMap::new();
        let mut raw_names: HashMap<String, Vec<u8>> = HashMap::new();
        for header in &headers {
            if let Some(raw) = &header.raw_filename {
                raw_names.insert(header.filename.clone(), raw.clone());
            }
            // never preallocate the full length from untrusted input
            let mut buffer: Vec<u8> = Vec::new();
            let got = input
                .take(u64::from(header.data_size))
                .read_to_end(&mut buffer)?;
            if got != header.data_size as usize {
                return Err(error!(
                    "PBO entry \"{}\" is truncated ({} of {} bytes).",
                    header.filename, got, header.data_size
                ));
            }
            files.insert(
                header.filename.clone(),
                Cursor::new(buffer.into_boxed_slice()),
            );
        }

        let mut skip = [0u8; 1];
        let _ = input.read(&mut skip)?;
        let mut checksum = vec![0; 20];
        input.read_exact(&mut checksum)?;

        Ok(PBO {
            files,
            header_extensions,
            raw_names,
            raw_extensions,
            headers,
            checksum: Some(checksum),
        })
    }

    /// Constructs a PBO from a directory with optional binarization.
    ///
    /// `exclude_patterns` contains glob patterns to exclude from the PBO, `includefolders` contain
    /// paths to search for absolute includes and should generally include the current working
    /// directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read or a file cannot be processed.
    #[expect(
        clippy::too_many_lines,
        reason = "one branch per file kind keeps the packing loop readable"
    )]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "public API: callers pass an owned directory path"
    )]
    pub fn from_directory(
        directory: PathBuf,
        mut binarize: bool,
        exclude_patterns: &[String],
        includefolders: &[PathBuf],
    ) -> Result<PBO, Error> {
        let file_list = list_files(&directory)?;
        let mut files: LinkedHashMap<String, Cursor<Box<[u8]>>> = LinkedHashMap::new();
        let mut header_extensions: HashMap<String, String> = HashMap::new();

        if directory.join("$NOBIN$").exists() || directory.join("$NOBIN-NOTEST$").exists() {
            binarize = false;
        }

        // the prefix is needed while binarizing, so read it before looking at the other files
        let mut pre_extensions: HashMap<String, String> = HashMap::new();
        for prefix_file in &["$PBOPREFIX$", "$PREFIX$"] {
            if let Ok(content) = std::fs::read_to_string(directory.join(prefix_file)) {
                parse_prefix_file(&content, &mut pre_extensions, false);
            }
        }
        let prefix: String = pre_extensions.get("prefix").cloned().unwrap_or_else(|| {
            directory
                .canonicalize()
                .ok()
                .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default()
        });

        let addon_files: HashSet<String> = file_list
            .iter()
            .filter_map(|p| p.strip_prefix(&directory).ok())
            .filter_map(|p| p.to_str())
            .map(|p| p.replace('/', "\\").to_lowercase())
            .collect();

        for path in file_list {
            let Ok(mut relative) = path.strip_prefix(&directory).map(Path::to_path_buf) else {
                continue;
            };
            if binarize && relative.file_name() == Some(OsStr::new("config.cpp")) {
                relative = relative.with_file_name("config.bin");
            }

            let mut name: String = relative.to_string_lossy().replace('/', "\\");
            let is_binarizable = BINARIZABLE_RE.is_match(&name);

            if !file_allowed(&name, exclude_patterns) {
                continue;
            }

            let mut file = File::open(&path)?;

            let prebinarized_as = if binarize && is_binarizable {
                binarize::binarized_description(&path)?
            } else {
                None
            };
            let prebinarized = prebinarized_as.is_some();
            if let Some(description) = prebinarized_as {
                warning(
                    format!("File is already binarized ({description}); copying it as-is."),
                    Some("prebinarized-file"),
                    (Some(relative.to_string_lossy().into_owned()), None),
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
                    includefolders,
                );
            }

            if name == "$PBOPREFIX$" || name == "$PREFIX$" {
                let mut content = String::new();
                file.read_to_string(&mut content)?;
                parse_prefix_file(&content, &mut header_extensions, true);
            } else if binarize
                && ["cpp", "rvmat"]
                    .contains(&path.extension().and_then(|e| e.to_str()).unwrap_or(""))
            {
                let mut buffer: Vec<u8> = Vec::new();
                file.read_to_end(&mut buffer)?;

                if is_rapified(&buffer) {
                    warning(
                        "File is already rapified; copying it as-is.",
                        Some("prerapified-file"),
                        (Some(relative.to_string_lossy().as_ref()), None),
                    );
                    files.insert(name, Cursor::new(buffer.into_boxed_slice()));
                } else {
                    let config =
                        Config::read(&mut Cursor::new(buffer), Some(path.clone()), includefolders)
                            .prepend_error("Failed to parse config:")?;
                    files.insert(name, config.to_cursor()?);
                }
            } else if binarize::available() && binarize && is_binarizable && !prebinarized {
                let cursor = binarize::binarize(&path, &directory, &prefix)
                    .prepend_error(format!("Failed to binarize {}:", relative.display()))?;

                files.insert(name, cursor);
            } else {
                if is_binarizable && !prebinarized && !binarize::available() {
                    warning(
                        "On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is.",
                        Some("non-windows-binarization"),
                        (Some(relative.to_string_lossy().as_ref()), None),
                    );
                }

                let mut buffer: Vec<u8> = Vec::new();
                file.read_to_end(&mut buffer)?;

                name = P3DO_RE.replace_all(&name, ".p3d").to_string();

                files.insert(name, Cursor::new(buffer.into_boxed_slice()));
            }
        }

        if !header_extensions.contains_key("prefix") {
            let prefix: String = directory
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .ok_or_else(|| error!("Invalid addon directory \"{}\".", directory.display()))?;
            header_extensions.insert("prefix".to_string(), prefix);
        }

        Ok(PBO {
            files,
            header_extensions,
            raw_names: HashMap::new(),
            raw_extensions: HashMap::new(),
            headers: Vec::new(),
            checksum: None,
        })
    }

    /// Writes PBO to output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or if an entry is too large.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
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

        if let Some(prefix) = self.header_extensions.get("prefix") {
            headers.write_all(b"prefix\0")?;
            headers.write_cstring(self.extension_bytes(prefix))?;
        }

        for (key, value) in &self.header_extensions {
            if key == "prefix" {
                continue;
            }

            headers.write_cstring(self.extension_bytes(key))?;
            headers.write_cstring(self.extension_bytes(value))?;
        }
        headers.write_cstring(String::new())?;

        let mut files_sorted: Vec<(String, &Cursor<Box<[u8]>>)> =
            self.files.iter().map(|(a, b)| (a.clone(), b)).collect();
        files_sorted.sort_by_cached_key(|f| self.name_bytes(&f.0).to_ascii_lowercase());

        for (name, cursor) in &files_sorted {
            let size = u32::try_from(cursor.get_ref().len())
                .map_err(|_| error!("PBO entry \"{name}\" is too large."))?;
            let header = PBOHeader {
                filename: name.clone(),
                raw_filename: self.raw_names.get(name).cloned(),
                packing_method: 0,
                original_size: size,
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

        let mut h = Hasher::new(MessageDigest::sha1()).map_err(|e| error!("SHA-1: {e}"))?;

        output.write_all(headers.get_ref())?;
        h.update(headers.get_ref())
            .map_err(|e| error!("SHA-1: {e}"))?;

        for (_, cursor) in &files_sorted {
            output.write_all(cursor.get_ref())?;
            h.update(cursor.get_ref())
                .map_err(|e| error!("SHA-1: {e}"))?;
        }

        output.write_all(&[0])?;
        let checksum = h.finish().map_err(|e| error!("SHA-1: {e}"))?;
        output.write_all(&checksum)?;

        Ok(())
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

        match self.headers.iter().find(|h| h.filename == name) {
            Some(h) if h.packing_method == PACKING_COMPRESSED => {
                lzss_decompress(cursor.get_ref(), h.original_size as usize)
                    .prepend_error(format!("Failed to decompress \"{name}\":"))
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

fn list_files(directory: &PathBuf) -> Result<Vec<PathBuf>, Error> {
    let mut files: Vec<PathBuf> = Vec::new();

    for entry in read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            for f in list_files(&path)? {
                files.push(f);
            }
        } else {
            files.push(path);
        }
    }

    Ok(files)
}

/// Prints what is in a PBO: header extensions and the entry list.
///
/// # Errors
///
/// Returns an error if the PBO cannot be read.
#[expect(clippy::print_stdout, reason = "CLI command output")]
pub fn cmd_inspect<I: Read>(input: &mut I) -> Result<(), Error> {
    let pbo = PBO::read(input).prepend_error("Failed to read PBO:")?;

    if !pbo.header_extensions.is_empty() {
        println!("Header extensions:");
        for (key, value) in &pbo.header_extensions {
            println!("- {key}={value}");
        }
        println!();
    }

    println!("# Files: {}\n", pbo.files.len());

    println!("Path                                                  Method  Original    Packed");
    println!("                                                                  Size      Size");
    println!("================================================================================");
    for header in pbo.headers {
        println!(
            "{:50} {:9} {:9} {:9}",
            header.filename, header.packing_method, header.original_size, header.data_size
        );
    }

    Ok(())
}

/// Writes the named entry of the PBO to the output.
///
/// # Errors
///
/// Returns an error if the PBO cannot be read or the output cannot be written.
pub fn cmd_cat<I: Read, O: Write>(input: &mut I, output: &mut O, name: &str) -> Result<(), Error> {
    let pbo = PBO::read(input).prepend_error("Failed to read PBO:")?;

    match pbo.file_data(name)? {
        Some(data) => {
            output
                .write_all(&data)
                .prepend_error("Failed to write output:")?;
        }
        None => {
            #[expect(clippy::print_stderr, reason = "CLI diagnostics")]
            {
                eprintln!("not found"); // @todo
            }
        }
    }

    Ok(())
}

/// Unpacks a PBO into the output folder.
///
/// # Errors
///
/// Returns an error if the PBO cannot be read or a target file exists without `force`.
pub fn cmd_unpack<I: Read>(input: &mut I, output: &Path, force: bool) -> Result<(), Error> {
    let pbo = PBO::read(input).prepend_error("Failed to read PBO:")?;

    create_dir_all(output).prepend_error("Failed to create output folder:")?;

    if !pbo.header_extensions.is_empty() {
        let prefix_path = output.join(PathBuf::from("$PBOPREFIX$"));
        if !force && prefix_path.exists() {
            return Err(error!(
                "Target file \"{}\" already exists. Use --force to overwrite.",
                prefix_path.display()
            ));
        }
        let mut prefix_file =
            File::create(prefix_path).prepend_error("Failed to create prefix file:")?;

        for (key, value) in &pbo.header_extensions {
            prefix_file
                .write_all(format!("{key}={value}\n").as_bytes())
                .prepend_error("Failed to write prefix file:")?;
        }
    }

    for file_name in pbo.files.keys() {
        if pbo.raw_names.contains_key(file_name) {
            warning(
                format!(
                    "Entry name {file_name:?} is not valid UTF-8; extracting it as windows-1252."
                ),
                Some("non-utf8-filename"),
                (None, None),
            );
        }
        let data = match pbo.file_data(file_name) {
            Ok(Some(data)) => data,
            Ok(None) => continue,
            Err(e) => {
                warning(format!("Skipping \"{file_name}\": {e}"), None, (None, None));
                continue;
            }
        };
        // @todo: windows
        let path = output.join(PathBuf::from(file_name.replace('\\', pathsep())));
        if !force && path.exists() {
            return Err(error!(
                "Target file \"{}\" already exists. Use --force to overwrite.",
                path.display()
            ));
        }
        let written = create_dir_all(path.parent().unwrap_or(output))
            .prepend_error("Failed to create output folder:")
            .and_then(|()| File::create(&path).prepend_error("Failed to open output file:"))
            .and_then(|mut file| {
                file.write_all(&data)
                    .prepend_error("Failed to write output file:")
            });
        if let Err(e) = written {
            warning(format!("Skipping \"{file_name}\": {e}"), None, (None, None));
        }
    }

    Ok(())
}

/// Packs a folder into a PBO without any binarization or rapification.
///
/// # Errors
///
/// Returns an error if the folder cannot be read or the PBO cannot be written.
pub fn cmd_pack<O: Write>(
    input: PathBuf,
    output: &mut O,
    headerext: &[String],
    excludes: &[String],
) -> Result<(), Error> {
    let mut pbo = PBO::from_directory(input, false, excludes, &Vec::new())?;

    for h in headerext {
        let Some((key, value)) = h.split_once('=') else {
            return Err(error!(
                "Invalid header extension \"{h}\" (expected key=value)."
            ));
        };
        pbo.header_extensions
            .insert(key.to_string(), value.to_string());
    }

    pbo.write(output).prepend_error("Failed to write PBO:")?;

    Ok(())
}

/// Builds a PBO from a folder, binarizing and rapifying where needed.
///
/// # Errors
///
/// Returns an error if a file cannot be read, binarized or written.
pub fn cmd_build<O: Write>(
    input: PathBuf,
    output: &mut O,
    headerext: &[String],
    excludes: &[String],
    includefolders: &[PathBuf],
) -> Result<(), Error> {
    let mut pbo = PBO::from_directory(input, true, excludes, includefolders)?;

    for h in headerext {
        let Some((key, value)) = h.split_once('=') else {
            return Err(error!(
                "Invalid header extension \"{h}\" (expected key=value)."
            ));
        };
        pbo.header_extensions
            .insert(key.to_string(), value.to_string());
    }

    pbo.write(output).prepend_error("Failed to write PBO:")?;

    Ok(())
}
