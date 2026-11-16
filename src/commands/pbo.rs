//! PBO commands: inspect, cat, pack, build and unpack.

use std::io::{Error, Read, Write};
use std::path::{Path, PathBuf};

use dayzmake_core::binarize::Binarizer;
use dayzmake_core::error;
use dayzmake_core::error::{ErrorExt, printable};
use dayzmake_core::pbo::{PBO, PackOptions, UnpackOptions, unpack};
use dayzmake_core::preprocess::IncludeFolders;
use dayzmake_core::warning::WarningSink;

/// Writes what is in a PBO to the output: header extensions and the entry list.
///
/// # Errors
///
/// Returns an error if the PBO cannot be read or the output cannot be written.
pub fn cmd_inspect<I: Read, O: Write>(input: &mut I, output: &mut O) -> Result<(), Error> {
    let pbo = PBO::read(input).prepend_error("Failed to read PBO:")?;

    if pbo.header_extensions().next().is_some() {
        writeln!(output, "Header extensions:")?;
        for (key, value) in pbo.header_extensions() {
            writeln!(output, "- {}={}", printable(key), printable(value))?;
        }
        writeln!(output)?;
    }

    writeln!(output, "# Files: {}\n", pbo.entry_names().count())?;

    writeln!(
        output,
        "Path                                                  Method  Original    Packed"
    )?;
    writeln!(
        output,
        "                                                                  Size      Size"
    )?;
    writeln!(
        output,
        "================================================================================"
    )?;
    for header in pbo.entry_headers() {
        writeln!(
            output,
            "{:50} {:9} {:9} {:9}",
            printable(&header.name),
            header.packing_method,
            header.original_size,
            header.data_size
        )?;
    }

    Ok(())
}

/// Writes the header of a PBO to the output as one JSON object (see `docs/book/src/reference/pbo.md`):
/// the header extensions, the prefix and the entries, each name with its raw bytes as lowercase hex.
/// Names are decoded lossily as UTF-8 and only JSON-escaped.
///
/// # Errors
///
/// Returns an error if the PBO cannot be read or the output cannot be written.
pub fn cmd_inspect_json<I: Read, O: Write>(input: &mut I, output: &mut O) -> Result<(), Error> {
    let pbo = PBO::read(input).prepend_error("Failed to read PBO:")?;
    let lossy = |raw: &[u8]| (String::from_utf8_lossy(raw).into_owned(), hex(raw));
    let raw_extensions = pbo.header_extensions_bytes();
    let mut prefix = None;
    let mut prefix_hex = None;
    let mut header_extensions = Vec::new();
    for ((key, _), (raw_key, raw_value)) in pbo.header_extensions().zip(&raw_extensions) {
        let (value, value_hex) = lossy(raw_value);
        if key == "prefix" {
            prefix = Some(value.clone());
            prefix_hex = Some(value_hex.clone());
        }
        let (key, key_hex) = lossy(raw_key);
        header_extensions.push(JsonExtension {
            key,
            value,
            key_hex,
            value_hex,
        });
    }
    let entries = pbo
        .entry_headers()
        .into_iter()
        .map(|header| JsonEntry {
            name: String::from_utf8_lossy(&header.name_bytes).into_owned(),
            name_hex: hex(&header.name_bytes),
            packing: header.packing_method,
            original_size: header.original_size,
            data_size: header.data_size,
            timestamp: header.timestamp,
        })
        .collect();
    let json = JsonInspect {
        header_extensions,
        prefix,
        prefix_hex,
        entries,
    };
    serde_json::to_writer(&mut *output, &json).map_err(Error::from)?;
    writeln!(output)
}

/// Lowercase hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        // writing to a String cannot fail
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// `inspect --json`; the fields are in the order of the output.
#[derive(serde::Serialize)]
struct JsonInspect {
    header_extensions: Vec<JsonExtension>,
    prefix: Option<String>,
    prefix_hex: Option<String>,
    entries: Vec<JsonEntry>,
}

#[derive(serde::Serialize)]
struct JsonExtension {
    key: String,
    value: String,
    key_hex: String,
    value_hex: String,
}

#[derive(serde::Serialize)]
struct JsonEntry {
    name: String,
    name_hex: String,
    packing: u32,
    original_size: u32,
    data_size: u32,
    timestamp: u32,
}

/// Writes the named entry of the PBO to the output.
///
/// # Errors
///
/// Returns an error if the PBO cannot be read, the entry does not exist, or the output cannot be
/// written.
pub fn cmd_cat<I: Read, O: Write>(input: &mut I, output: &mut O, name: &str) -> Result<(), Error> {
    let pbo = PBO::read(input).prepend_error("Failed to read PBO:")?;

    match pbo.file_data(name)? {
        Some(data) => output
            .write_all(&data)
            .prepend_error("Failed to write output:"),
        None => Err(error!("File \"{}\" not found in PBO.", printable(name))),
    }
}

/// Parses the `-e key=value` header extensions.
///
/// # Errors
///
/// Returns an error for an entry without `=`.
fn parse_header_extensions(headerext: &[String]) -> Result<Vec<(String, String)>, Error> {
    headerext
        .iter()
        .map(|h| {
            h.split_once('=')
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .ok_or_else(|| error!("Invalid header extension \"{h}\" (expected key=value)."))
        })
        .collect()
}

/// Unpacks a PBO into the output folder; see [`unpack`].
///
/// # Errors
///
/// Returns an error if the PBO cannot be read, the prefix is unsafe with `use_prefix`, or a target
/// file exists without `force`.
pub fn cmd_unpack<I: Read>(
    input: &mut I,
    output: &Path,
    options: &UnpackOptions,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    unpack(input, output, options, warnings)
}

/// Fails with the folder's name when it cannot be read (the library error does not name it).
fn check_source_folder(input: &Path) -> Result<(), Error> {
    std::fs::read_dir(input).map(drop).prepend_error(format!(
        "Failed to read source folder \"{}\":",
        printable(&input.display().to_string())
    ))
}

/// Packs a folder into a PBO without any binarization or rapification.
///
/// # Errors
///
/// Returns an error if the folder cannot be read or the PBO cannot be written.
pub fn cmd_pack<O: Write>(
    input: &Path,
    output: &mut O,
    headerext: &[String],
    excludes: &[String],
    exclude_paths: &[PathBuf],
    exclude_stdout: bool,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let mut options = PackOptions::pack();
    options.exclude_stdout = exclude_stdout;
    options.exclude = excludes.to_vec();
    options.exclude_paths = exclude_paths.to_vec();
    options.header_extensions = parse_header_extensions(headerext)?;
    check_source_folder(input)?;
    let pbo = PBO::from_directory(input, &options, None, warnings)?;

    pbo.write(output).prepend_error("Failed to write PBO:")?;

    Ok(())
}

/// Builds a PBO from a folder, binarizing and rapifying where needed.
///
/// # Errors
///
/// Returns an error if a file cannot be read, binarized or written.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is a separate input of the build; a parameter struct would only move them"
)]
pub fn cmd_build<O: Write>(
    input: &Path,
    output: &mut O,
    headerext: &[String],
    excludes: &[String],
    includefolders: &IncludeFolders,
    exclude_paths: &[PathBuf],
    exclude_stdout: bool,
    binarizer: Option<&mut Binarizer>,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let mut options = PackOptions::build();
    options.exclude_stdout = exclude_stdout;
    options.exclude = excludes.to_vec();
    options.include_folders.clone_from(&includefolders.search);
    options
        .direct_include_folders
        .clone_from(&includefolders.direct);
    options.exclude_paths = exclude_paths.to_vec();
    options.header_extensions = parse_header_extensions(headerext)?;
    check_source_folder(input)?;
    let pbo = PBO::from_directory(input, &options, binarizer, warnings)?;

    pbo.write(output).prepend_error("Failed to write PBO:")?;

    Ok(())
}
