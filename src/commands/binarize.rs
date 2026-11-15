//! `dayzmake binarize`: binarize one model through BI's `binarize.exe`.

use std::fs::File;
use std::io::{Error, Write};
use std::path::{Path, PathBuf};

use dayzmake_core::binarize::{BinarizeBackend, Binarizer, binarized_description};
use dayzmake_core::error;
use dayzmake_core::error::ErrorExt;
use dayzmake_core::warning::{Warning, WarningSink};

/// Binarizes the given path using BI's binarize.exe and writes it to the output.
///
/// # Errors
///
/// Returns an error if the input cannot be binarized or the output exists without `force`.
pub fn cmd_binarize(
    input: PathBuf,
    output: &Path,
    force: bool,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    if !force && output.exists() {
        return Err(error!(
            "Target file \"{}\" already exists. Use --force to overwrite.",
            output.display()
        ));
    }

    if let Some(description) = binarized_description(&input)? {
        warnings.warn(
            Warning::new(
                Some("prebinarized-file"),
                format!("File is already binarized ({description}); copying it as-is."),
            )
            .in_file(input.to_string_lossy()),
        );
        std::fs::copy(&input, output).prepend_error("Failed to write result to file:")?;
        return Ok(());
    }

    // `dayzmake binarize` always goes through Proton on unix; the sandbox is removed when `binarizer` drops
    let mut binarizer = Binarizer::new(if cfg!(windows) {
        BinarizeBackend::Native
    } else {
        BinarizeBackend::Proton
    })?;
    let (input, addon_root) = cli_addon(input);
    let prefix = addon_root
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let data = binarizer.binarize(&input, &addon_root, &prefix, warnings)?;
    let mut file = File::create(output).prepend_error("Failed to open output:")?;
    file.write_all(&data)
        .prepend_error("Failed to write result to file:")?;

    Ok(())
}

/// For `dayzmake binarize` there is no PBO: the model's folder is the addon (and its name the
/// prefix). Returns the input and that folder, with the input inside it.
fn cli_addon(input: PathBuf) -> (PathBuf, PathBuf) {
    // a bare file name ("model.p3d") has the empty parent; make it "./model.p3d" so the addon is "."
    let input = match input.parent() {
        Some(p) if p.as_os_str().is_empty() => Path::new(".").join(&input),
        _ => input,
    };
    let addon_root = input
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    (input, addon_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_addon_bare_file_name() {
        for input in &[
            "model.p3d",
            "./model.p3d",
            "t21/model.p3d",
            "/abs/t21/model.p3d",
        ] {
            let (input, root) = cli_addon(PathBuf::from(input));
            assert!(
                input.parent().unwrap().strip_prefix(&root).is_ok(),
                "{input:?} not in {root:?}"
            );
            assert_eq!(input.file_name().unwrap(), "model.p3d");
        }
        assert_eq!(cli_addon(PathBuf::from("model.p3d")).1, PathBuf::from("."));
    }
}
