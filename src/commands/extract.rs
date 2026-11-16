//! `dayzmake extract`: a whole install or mod into one tree.

use std::io::Error;
use std::path::Path;

use dayzmake_core::extract::{ExtractOptions, ExtractSummary, extract};
use dayzmake_core::warning::WarningSink;

/// Extracts the PBOs of `source` into `target`; see [`extract`].
///
/// # Errors
///
/// Returns an error if the source has no PBO or a PBO is malformed, the target exists without
/// `options.force`, or the tree cannot be written.
pub fn cmd_extract(
    source: &Path,
    target: &Path,
    options: &ExtractOptions,
    warnings: &mut dyn WarningSink,
) -> Result<ExtractSummary, Error> {
    extract(source, target, options, warnings)
}
