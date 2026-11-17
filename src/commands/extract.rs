//! `dayzmake extract`: a whole install or mod into one tree.

use std::io::Error;
use std::path::Path;

use dayzmake_core::extract::{ExtractOptions, ExtractSummary, extract};
use dayzmake_core::warning::WarningSink;

/// Extracts the PBOs of `source` into `target`; see [`extract`].
///
/// # Errors
///
/// Returns an error if the source has no PBO or cannot be read, a PBO is malformed or an entry is truncated or does
/// not decompress, the target exists without `options.force`, is not a folder, is inside the source or contains it,
/// is a folder that is neither empty nor an earlier extract result (no `pbo-map.tsv`), or the tree cannot be written.
pub fn cmd_extract(
    source: &Path,
    target: &Path,
    options: &ExtractOptions,
    warnings: &mut dyn WarningSink,
) -> Result<ExtractSummary, Error> {
    extract(source, target, options, warnings)
}
