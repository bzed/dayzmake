//! PAA conversion commands.

use std::fs::File;
use std::io::Error;
use std::path::Path;

use dayzmake_core::error;
use dayzmake_core::error::ErrorExt;
use dayzmake_core::paa::{Paa, PaaFormat};

/// Converts a PAA file to a standard image format.
///
/// # Errors
///
/// Returns an error if the input cannot be read or converted, or the target exists.
pub fn cmd_paa2img(input_path: &Path, output_path: &Path, force: bool) -> Result<(), Error> {
    if !force && output_path.exists() {
        return Err(error!(
            "Target file \"{}\" already exists. Use --force to overwrite.",
            output_path.display()
        ));
    }

    let mut file = File::open(input_path).prepend_error("Failed to open input PAA file:")?;
    let paa = Paa::read(&mut file).prepend_error("Failed to read PAA file:")?;
    let img = paa
        .to_dynamic_image()
        .prepend_error("Failed to convert PAA to image:")?;

    img.save(output_path)
        .map_err(|e| error!("{}", e))
        .prepend_error("Failed to save output image:")?;

    Ok(())
}

/// Converts a standard image file to PAA.
///
/// # Errors
///
/// Returns an error if the input cannot be read or converted, or the target exists.
pub fn cmd_img2paa(
    input_path: &Path,
    output_path: &Path,
    paa_type: &str,
    // `-c` has no effect; the flag stays so existing scripts keep working
    _compress: bool,
    force: bool,
) -> Result<(), Error> {
    if !force && output_path.exists() {
        return Err(error!(
            "Target file \"{}\" already exists. Use --force to overwrite.",
            output_path.display()
        ));
    }

    let img = image::open(input_path)
        .map_err(|e| error!("{}", e))
        .prepend_error("Failed to open input image file:")?;
    let paa = paa_type
        .parse::<PaaFormat>()
        .and_then(|format| Paa::from_dynamic_image(&img, format))
        .prepend_error("Failed to convert image to PAA:")?;

    let mut file = File::create(output_path).prepend_error("Failed to create output PAA file:")?;
    paa.write(&mut file)
        .prepend_error("Failed to write PAA file:")?;

    Ok(())
}
