//! PBO commands: inspect, cat, pack, build and unpack.

use std::io::{Error, Read, Write};
use std::path::{Path, PathBuf};

use dayzmake_core::binarize::Binarizer;
use dayzmake_core::error;
use dayzmake_core::error::{ErrorExt, printable};
use dayzmake_core::pbo::{PBO, PackOptions, UnpackOptions, unpack};
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
    force: bool,
    use_prefix: bool,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let mut options = UnpackOptions::default();
    options.force = force;
    options.use_prefix = use_prefix;
    unpack(input, output, &options, warnings)
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
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let mut options = PackOptions::pack();
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
    includefolders: &[PathBuf],
    exclude_paths: &[PathBuf],
    binarizer: Option<&mut Binarizer>,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let mut options = PackOptions::build();
    options.exclude = excludes.to_vec();
    options.include_folders = includefolders.to_vec();
    options.exclude_paths = exclude_paths.to_vec();
    options.header_extensions = parse_header_extensions(headerext)?;
    check_source_folder(input)?;
    let pbo = PBO::from_directory(input, &options, binarizer, warnings)?;

    pbo.write(output).prepend_error("Failed to write PBO:")?;

    Ok(())
}
