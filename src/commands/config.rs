//! Config commands: rapify, derapify and preprocess.

use std::io::{Cursor, Error, Read, Seek, Write};
use std::path::PathBuf;

use dayzmake_core::config::{Config, WriteOptions, is_rapified};
use dayzmake_core::error::ErrorExt;
use dayzmake_core::preprocess::{IncludeFolders, preprocess_in};
use dayzmake_core::warning::{Warning, WarningSink};

/// Reads input, preprocesses and rapifies it and writes to output.
///
/// `path` is the path to the input if it is known and is used for relative includes and error
/// messages. `includefolders` are the folders searched recursively for absolute includes.
///
/// # Errors
///
/// Returns an error on preprocessing, parsing or I/O failure. Already-rapified
/// input is passed through unchanged with a warning.
pub fn cmd_rapify<I: Read, O: Write>(
    input: &mut I,
    output: &mut O,
    path: Option<PathBuf>,
    includefolders: &IncludeFolders,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let mut bytes = Vec::new();
    input
        .read_to_end(&mut bytes)
        .prepend_error("Failed to read input file:")?;

    if is_rapified(&bytes) {
        let mut warning = Warning::new(
            Some("prerapified-file"),
            "Input is already rapified; passing it through unchanged.",
        );
        if let Some(file) = path.as_ref().and_then(|p| p.to_str()) {
            warning = warning.in_file(file);
        }
        warnings.warn(warning);
        output.write_all(&bytes)?;
        return Ok(());
    }

    let config = Config::read_in(&mut Cursor::new(bytes), path, includefolders, warnings)?;

    config
        .write_rapified(output)
        .prepend_error("Failed to write rapified config:")?;

    Ok(())
}

/// Reads input, derapifies it and writes to output, indenting with `indent`
/// (spaces and tabs only).
///
/// # Errors
///
/// Returns an error if the input is not a rapified config or cannot be written.
pub fn cmd_derapify<I: Read + Seek, O: Write>(
    input: &mut I,
    output: &mut O,
    indent: &str,
) -> Result<(), Error> {
    let options = WriteOptions::new().indent(indent)?;
    let config = Config::read_rapified(input).prepend_error("Failed to read rapified config:")?;

    config
        .write_with(output, &options)
        .prepend_error("Failed to derapify config:")?;

    Ok(())
}

/// Reads input, preprocesses it and writes to output.
///
/// `path` is the `path` to the input if it is known and is used for relative includes and error
/// messages. `includefolders` are the folders searched recursively for absolute includes.
///
/// # Errors
///
/// Returns an error on preprocessing or I/O failure.
pub fn cmd_preprocess<I: Read, O: Write>(
    input: &mut I,
    output: &mut O,
    path: Option<PathBuf>,
    includefolders: &IncludeFolders,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let mut buffer = String::new();
    input
        .read_to_string(&mut buffer)
        .prepend_error("Failed to read input file")?;

    let (result, _) = preprocess_in(buffer, path, includefolders, warnings)?;

    output
        .write_all(result.as_bytes())
        .prepend_error("Failed to write output")?;

    Ok(())
}
