//! `dayzmake inspect` for models.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, Read, Seek, SeekFrom, Write};

use dayzmake_core::error;
use dayzmake_core::error::{ErrorExt, printable};
use dayzmake_core::odol::Odol;
use dayzmake_core::p3d::{P3D, model_magic};

/// Writes what is in a model to the output. MLOD and ODOL version 56 are listed;
/// for other ODOL versions and for BMTR only the format and version are known.
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
            if &m == b"ODOL" {
                if version == 56 {
                    input.seek(SeekFrom::Start(0))?;
                    let odol = Odol::read(input).prepend_error("Failed to read model:")?;
                    return write_odol(&odol, output);
                }
                writeln!(
                    output,
                    "Note: only version 56 is decoded; the contents of this model are not listed."
                )?;
            }
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

/// The listing of an ODOL v56 model, laid out like the MLOD one.
fn write_odol<O: Write>(odol: &Odol, output: &mut O) -> Result<(), Error> {
    writeln!(output, "LODs:    {}", odol.lods.len())?;
    if odol.skeleton_name.is_empty() {
        writeln!(output, "Skeleton: none")?;
    } else {
        writeln!(
            output,
            "Skeleton: {} ({} bones)",
            printable(&odol.skeleton_name),
            odol.bone_count
        )?;
    }
    writeln!(output, "Class:   {}", printable(&odol.property_class))?;
    writeln!(output, "Damage:  {}", printable(&odol.property_damage))?;
    writeln!(output, "Mass:    {}", odol.mass)?;
    writeln!(
        output,
        "Bounding box: ({}, {}, {}) to ({}, {}, {})",
        odol.bbox_min[0],
        odol.bbox_min[1],
        odol.bbox_min[2],
        odol.bbox_max[0],
        odol.bbox_max[1],
        odol.bbox_max[2]
    )?;
    writeln!(output)?;

    writeln!(output, "  #  {:24} {:>8} {:>8}", "LOD", "Vertices", "Faces")?;
    for (i, lod) in odol.lods.iter().enumerate() {
        writeln!(
            output,
            "{:3}  {:24} {:8} {:8}",
            i,
            printable(&lod.name()),
            lod.vertex_count,
            lod.face_count
        )?;
    }

    let (textures, materials) = count_names(odol);
    writeln!(output)?;
    writeln!(output, "Textures:")?;
    for (name, count) in &textures {
        writeln!(output, "- {} ({count} LODs)", printable(name))?;
    }
    writeln!(output, "Materials:")?;
    for (name, count) in &materials {
        writeln!(output, "- {} ({count} LODs)", printable(name))?;
    }

    writeln!(output)?;
    writeln!(output, "Proxies:")?;
    for (i, lod) in odol.lods.iter().enumerate() {
        for proxy in &lod.proxies {
            writeln!(output, "- LOD {i}: {}", printable(proxy))?;
        }
    }

    writeln!(output)?;
    writeln!(output, "Properties:")?;
    for (i, lod) in odol.lods.iter().enumerate() {
        for (name, value) in &lod.properties {
            writeln!(
                output,
                "- LOD {i}: {} = {}",
                printable(name),
                printable(value)
            )?;
        }
    }

    writeln!(output)?;
    writeln!(output, "Selections:")?;
    for (i, lod) in odol.lods.iter().enumerate() {
        if !lod.selections.is_empty() {
            writeln!(
                output,
                "- LOD {} ({}): {}",
                i,
                printable(&lod.name()),
                printable(&lod.selections.join(", "))
            )?;
        }
    }

    writeln!(output)?;
    writeln!(output, "Animations:")?;
    for a in &odol.animations {
        writeln!(
            output,
            "- {} (source {}, type {})",
            printable(&a.name),
            printable(&a.source),
            a.kind
        )?;
    }
    Ok(())
}

/// Texture and material names in sorted order, with the number of LODs that use each.
fn count_names(odol: &Odol) -> (BTreeMap<&str, usize>, BTreeMap<&str, usize>) {
    let mut textures: BTreeMap<&str, usize> = BTreeMap::new();
    let mut materials: BTreeMap<&str, usize> = BTreeMap::new();
    for lod in &odol.lods {
        // Each name counts once per LOD, however often the LOD mentions it.
        let mut lod_textures: BTreeSet<&str> = lod
            .textures
            .iter()
            .filter(|t| !t.is_empty())
            .map(String::as_str)
            .collect();
        let mut lod_materials: BTreeSet<&str> = BTreeSet::new();
        for m in &lod.materials {
            lod_materials.insert(&m.name);
            lod_textures.extend(
                m.textures
                    .iter()
                    .filter(|t| !t.is_empty())
                    .map(String::as_str),
            );
        }
        for t in lod_textures {
            *textures.entry(t).or_default() += 1;
        }
        for m in lod_materials {
            *materials.entry(m).or_default() += 1;
        }
    }
    (textures, materials)
}
