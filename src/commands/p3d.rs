//! `dayzmake inspect` for models.

use std::io::{Error, Read, Seek, SeekFrom, Write};

use dayzmake_core::error;
use dayzmake_core::error::{ErrorExt, printable};
use dayzmake_core::p3d::{P3D, model_magic};

/// Writes what is in a model to the output. For ODOL (and BMTR) only the format and version are known.
///
/// # Errors
///
/// Returns an error if the input is not a model or cannot be read, or the output cannot be written.
pub fn cmd_inspect<I: Read + Seek, O: Write>(input: &mut I, output: &mut O) -> Result<(), Error> {
    match model_magic(input)? {
        Some(ref m) if m == b"MLOD" => {}
        Some(m) => {
            input.seek(SeekFrom::Current(4))?;
            let mut bytes = [0u8; 4];
            input
                .read_exact(&mut bytes)
                .prepend_error("Failed to read the version:")?;
            let version = u32::from_le_bytes(bytes);
            let (name, what) = if &m == b"ODOL" {
                ("ODOL", "binarized model")
            } else {
                ("BMTR", "binarized animation")
            };
            writeln!(output, "Format:  {name} ({what})")?;
            writeln!(output, "Version: {version}")?;
            return Ok(());
        }
        None => return Err(error!("Not a P3D model (no MLOD, ODOL or BMTR header).")),
    }

    let p3d = P3D::read(input).prepend_error("Failed to read model:")?;

    writeln!(output, "Format:  MLOD (editable model)")?;
    writeln!(output, "Version: {}", p3d.version)?;
    writeln!(output, "LODs:    {}", p3d.lods.len())?;
    writeln!(output)?;

    writeln!(
        output,
        "  #  {:24} {:>8} {:>8} {:>8} {:>8}",
        "LOD", "Points", "Faces", "Normals", "UV sets"
    )?;
    for (i, lod) in p3d.lods.iter().enumerate() {
        writeln!(
            output,
            "{:3}  {:24} {:8} {:8} {:8} {:8}",
            i,
            printable(&lod.name()),
            lod.points.len(),
            lod.faces.len(),
            lod.face_normals.len(),
            lod.uv_set_count()
        )?;
    }

    writeln!(output)?;
    writeln!(output, "Textures:")?;
    for (name, count) in p3d.textures() {
        writeln!(output, "- {} ({count} faces)", printable(name))?;
    }
    writeln!(output, "Materials:")?;
    for (name, count) in p3d.materials() {
        writeln!(output, "- {} ({count} faces)", printable(name))?;
    }

    writeln!(output)?;
    writeln!(output, "Properties:")?;
    for (i, lod) in p3d.lods.iter().enumerate() {
        for (name, value) in lod.properties() {
            writeln!(
                output,
                "- LOD {i}: {} = {}",
                printable(&name),
                printable(&value)
            )?;
        }
    }

    writeln!(output)?;
    writeln!(output, "Selections:")?;
    for (i, lod) in p3d.lods.iter().enumerate() {
        let selections = lod.selections();
        if !selections.is_empty() {
            writeln!(
                output,
                "- LOD {} ({}): {}",
                i,
                printable(&lod.name()),
                printable(&selections.join(", "))
            )?;
        }
    }

    Ok(())
}
