//! PAA conversion commands.

use std::io::{Cursor, Error, Write};
use std::path::Path;

use crate::cli_io::{AtomicTarget, open_failure, open_input};
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

    let mut file = open_input(input_path, "input PAA file")?;
    let paa = Paa::read(&mut file).prepend_error("Failed to read PAA file:")?;
    let img = paa
        .to_dynamic_image()
        .prepend_error("Failed to convert PAA to image:")?;

    // the image format comes from the target's extension, which a temp file name would hide: encode first
    let format = image::ImageFormat::from_path(output_path)
        .map_err(|e| error!("{}", e))
        .prepend_error("Failed to save output image:")?;
    let mut encoded = Cursor::new(Vec::new());
    img.write_to(&mut encoded, format)
        .map_err(|e| error!("{}", e))
        .prepend_error("Failed to save output image:")?;
    let mut file =
        AtomicTarget::create_with(output_path, true, false, "Failed to save output image:")?;
    file.write_all(encoded.get_ref())
        .prepend_error("Failed to save output image:")?;
    file.commit()?;

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
        .prepend_error(open_failure(input_path, "input image file"))?;
    let paa = paa_type
        .parse::<PaaFormat>()
        .and_then(|format| Paa::from_dynamic_image(&img, format))
        .prepend_error("Failed to convert image to PAA:")?;

    let mut file = AtomicTarget::create_with(
        output_path,
        true,
        false,
        "Failed to create output PAA file:",
    )?;
    paa.write(&mut file)
        .prepend_error("Failed to write PAA file:")?;
    file.commit()?;

    Ok(())
}
