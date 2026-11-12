//! Reading, writing and inspecting P3D models.
//!
//! [`P3D`] handles the editable MLOD format and keeps everything the file holds, so that
//! `write(read(x))` reproduces `x` byte for byte. Binarized ODOL models are only recognised
//! (format and version); they are produced by BI's binarize.exe, not by this crate.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::error::{ErrorExt, warning};
use crate::io::{ReadExt, WriteExt};

fn invalid<S: Into<String>>(msg: S) -> Error {
    Error::new(ErrorKind::InvalidData, msg.into())
}

/// Counts in the file are not trusted for preallocation.
fn capacity(n: u32) -> usize {
    (n as usize).min(1 << 16)
}

/// A vertex position with its flags.
#[derive(Debug, Default)]
pub struct Point {
    /// Position in model space.
    pub coords: (f32, f32, f32),
    /// Point flags.
    pub flags: u32,
}

/// A face vertex: indices into the point and normal arrays plus a UV pair.
#[derive(Debug, Default)]
pub struct Vertex {
    /// Index into the LOD's points.
    pub point_index: u32,
    /// Index into the LOD's face normals.
    pub normal_index: u32,
    /// Texture coordinates.
    pub uv: (f32, f32),
}

/// A face with its vertices, texture and material.
#[derive(Debug, Default)]
pub struct Face {
    /// The face's vertices (3 for a triangle, 4 for a quad).
    pub vertices: Vec<Vertex>,
    /// Face flags.
    pub flags: u32,
    /// Texture path.
    pub texture: String,
    /// Material (rvmat) path.
    pub material: String,
    /// Triangles are stored with a fourth, unused vertex; kept so the file round-trips.
    pub spare_vertex: Option<Vertex>,
}

/// One entry of a LOD's TAGG section. The same name can occur several times
/// (`#Property#`, `#UVSet#`), so these are kept in file order in a `Vec`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tagg {
    /// The tagg's active byte.
    pub active: u8,
    /// The tagg's name (e.g. `#Property#` or a selection name).
    pub name: String,
    /// The tagg's payload.
    pub data: Vec<u8>,
}

/// One level of detail of the model.
#[derive(Debug)]
pub struct LOD {
    /// LOD format version, major part.
    pub version_major: u32,
    /// LOD format version, minor part.
    pub version_minor: u32,
    /// The 4 bytes after the counts in the LOD header.
    pub flags: u32,
    /// The LOD's resolution; special values name named LODs (geometry etc.).
    pub resolution: f32,
    /// Positions.
    pub points: Vec<Point>,
    /// Face normals.
    pub face_normals: Vec<(f32, f32, f32)>,
    /// Faces.
    pub faces: Vec<Face>,
    /// TAGG entries in file order.
    pub taggs: Vec<Tagg>,
}

/// An MLOD model.
#[derive(Debug)]
pub struct P3D {
    /// MLOD format version.
    pub version: u32,
    /// The model's LODs, in file order.
    pub lods: Vec<LOD>,
}

impl Point {
    /// Creates an all-zero point.
    #[must_use]
    pub fn new() -> Point {
        Point {
            coords: (0.0, 0.0, 0.0),
            flags: 0,
        }
    }

    fn read<I: Read>(input: &mut I) -> Result<Point, Error> {
        Ok(Point {
            coords: (
                input.read_f32::<LittleEndian>()?,
                input.read_f32::<LittleEndian>()?,
                input.read_f32::<LittleEndian>()?,
            ),
            flags: input.read_u32::<LittleEndian>()?,
        })
    }

    fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        output.write_f32::<LittleEndian>(self.coords.0)?;
        output.write_f32::<LittleEndian>(self.coords.1)?;
        output.write_f32::<LittleEndian>(self.coords.2)?;
        output.write_u32::<LittleEndian>(self.flags)?;
        Ok(())
    }
}

impl Vertex {
    /// Creates an all-zero vertex.
    #[must_use]
    pub fn new() -> Vertex {
        Vertex {
            point_index: 0,
            normal_index: 0,
            uv: (0.0, 0.0),
        }
    }

    fn read<I: Read>(input: &mut I) -> Result<Vertex, Error> {
        Ok(Vertex {
            point_index: input.read_u32::<LittleEndian>()?,
            normal_index: input.read_u32::<LittleEndian>()?,
            uv: (
                input.read_f32::<LittleEndian>()?,
                input.read_f32::<LittleEndian>()?,
            ),
        })
    }

    fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        output.write_u32::<LittleEndian>(self.point_index)?;
        output.write_u32::<LittleEndian>(self.normal_index)?;
        output.write_f32::<LittleEndian>(self.uv.0)?;
        output.write_f32::<LittleEndian>(self.uv.1)?;
        Ok(())
    }
}

impl Face {
    /// Creates an empty face.
    #[must_use]
    pub fn new() -> Face {
        Face {
            vertices: Vec::with_capacity(4),
            flags: 0,
            texture: String::new(),
            material: String::new(),
            spare_vertex: None,
        }
    }

    fn read<I: Read>(input: &mut I) -> Result<Face, Error> {
        let num_verts = input.read_u32::<LittleEndian>()?;
        if num_verts != 3 && num_verts != 4 {
            return Err(invalid(format!(
                "face with {num_verts} vertices (expected 3 or 4)"
            )));
        }

        let mut vertices: Vec<Vertex> = Vec::with_capacity(num_verts as usize);
        for _i in 0..num_verts {
            vertices.push(Vertex::read(input)?);
        }

        let spare_vertex = if num_verts == 3 {
            Some(Vertex::read(input)?)
        } else {
            None
        };

        let flags = input.read_u32::<LittleEndian>()?;
        let texture = input.read_cstring()?;
        let material = input.read_cstring()?;

        Ok(Face {
            vertices,
            flags,
            texture,
            material,
            spare_vertex,
        })
    }

    fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        if self.vertices.len() != 3 && self.vertices.len() != 4 {
            return Err(invalid(format!(
                "face with {} vertices (expected 3 or 4)",
                self.vertices.len()
            )));
        }
        let num_verts =
            u32::try_from(self.vertices.len()).map_err(|_| invalid("too many vertices in face"))?;
        output.write_u32::<LittleEndian>(num_verts)?;

        for vert in &self.vertices {
            vert.write(output)?;
        }
        if self.vertices.len() == 3 {
            match self.spare_vertex {
                Some(ref vert) => vert.write(output)?,
                None => Vertex::new().write(output)?,
            }
        }

        output.write_u32::<LittleEndian>(self.flags)?;
        output.write_cstring(&self.texture)?;
        output.write_cstring(&self.material)?;
        Ok(())
    }
}

impl Tagg {
    /// For `#Property#`: the (name, value) pair, two NUL-padded 64 byte strings.
    #[must_use]
    pub fn property(&self) -> Option<(String, String)> {
        if self.name != "#Property#" || self.data.len() != 128 {
            return None;
        }
        let field = |b: &[u8]| {
            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            String::from_utf8_lossy(b.get(..end).unwrap_or(b)).into_owned()
        };
        let (Some(low), Some(high)) = (self.data.get(..64), self.data.get(64..)) else {
            return None;
        };
        Some((field(low), field(high)))
    }

    /// Selections are the tags whose name isn't one of the `#...#` markers.
    #[must_use]
    pub fn is_selection(&self) -> bool {
        !self.name.starts_with('#')
    }
}

/// Human readable name of a LOD resolution, for the values that are well known.
#[must_use]
pub fn lod_name(resolution: f32) -> Option<&'static str> {
    const NAMES: &[(f32, &str)] = &[
        (1.0e3, "View - Gunner"),
        (1.1e3, "View - Pilot"),
        (1.2e3, "View - Cargo"),
        (1.0e4, "Shadow Volume"),
        (1.0e13, "Geometry"),
        (1.0e15, "Memory"),
        (2.0e15, "Land Contact"),
        (3.0e15, "Roadway"),
        (4.0e15, "Paths"),
        (5.0e15, "Hit-points"),
        (6.0e15, "View Geometry"),
        (7.0e15, "Fire Geometry"),
    ];
    NAMES
        .iter()
        .find(|(r, _)| (resolution - r).abs() <= r * 1e-4)
        .map(|(_, n)| *n)
}

impl LOD {
    /// Human readable name of this LOD (e.g. "Geometry", or its resolution).
    #[must_use]
    pub fn name(&self) -> String {
        match lod_name(self.resolution) {
            Some(n) => n.to_string(),
            None if self.resolution < 1.0e3 => format!("Resolution {}", self.resolution),
            None => format!("{:e}", self.resolution),
        }
    }

    /// The LOD's `#Property#` taggs as (name, value) pairs.
    #[must_use]
    pub fn properties(&self) -> Vec<(String, String)> {
        self.taggs.iter().filter_map(Tagg::property).collect()
    }

    /// Selections are the tags whose name isn't one of the `#...#` markers.
    #[must_use]
    pub fn selections(&self) -> Vec<&str> {
        self.taggs
            .iter()
            .filter(|t| t.is_selection())
            .map(|t| t.name.as_str())
            .collect()
    }

    /// Number of `#UVSet#` taggs.
    #[must_use]
    pub fn uv_set_count(&self) -> usize {
        self.taggs.iter().filter(|t| t.name == "#UVSet#").count()
    }

    fn read<I: Read>(input: &mut I) -> Result<LOD, Error> {
        let mut buffer = [0; 4];
        input.read_exact(&mut buffer)?;
        if &buffer != b"P3DM" {
            return Err(invalid("LOD doesn't start with P3DM"));
        }

        let version_major = input.read_u32::<LittleEndian>()?;
        let version_minor = input.read_u32::<LittleEndian>()?;

        let num_points = input.read_u32::<LittleEndian>()?;
        let num_face_normals = input.read_u32::<LittleEndian>()?;
        let num_faces = input.read_u32::<LittleEndian>()?;
        let flags = input.read_u32::<LittleEndian>()?;

        let mut points: Vec<Point> = Vec::with_capacity(capacity(num_points));
        let mut face_normals: Vec<(f32, f32, f32)> = Vec::with_capacity(capacity(num_face_normals));
        let mut faces: Vec<Face> = Vec::with_capacity(capacity(num_faces));

        for _i in 0..num_points {
            points.push(Point::read(input)?);
        }

        for _i in 0..num_face_normals {
            face_normals.push((
                input.read_f32::<LittleEndian>()?,
                input.read_f32::<LittleEndian>()?,
                input.read_f32::<LittleEndian>()?,
            ));
        }

        for _i in 0..num_faces {
            faces.push(Face::read(input)?);
        }

        input.read_exact(&mut buffer)?;
        if &buffer != b"TAGG" {
            return Err(invalid("LOD has no TAGG section"));
        }

        let mut taggs: Vec<Tagg> = Vec::new();

        loop {
            let active = input.read_u8()?;
            let name = input.read_cstring()?;
            let size = input.read_u32::<LittleEndian>()?;
            let mut data = Vec::with_capacity(capacity(size));
            let got = input
                .by_ref()
                .take(u64::from(size))
                .read_to_end(&mut data)?;
            if got != size as usize {
                return Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    format!("tag \"{name}\" is truncated"),
                ));
            }

            if name == "#EndOfFile#" {
                break;
            }

            taggs.push(Tagg { active, name, data });
        }

        let resolution = input.read_f32::<LittleEndian>()?;

        Ok(LOD {
            version_major,
            version_minor,
            flags,
            resolution,
            points,
            face_normals,
            faces,
            taggs,
        })
    }

    fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        let len32 =
            |len: usize| u32::try_from(len).map_err(|_| invalid("LOD section count out of range"));
        output.write_all(b"P3DM")?;
        output.write_u32::<LittleEndian>(self.version_major)?;
        output.write_u32::<LittleEndian>(self.version_minor)?;
        output.write_u32::<LittleEndian>(len32(self.points.len())?)?;
        output.write_u32::<LittleEndian>(len32(self.face_normals.len())?)?;
        output.write_u32::<LittleEndian>(len32(self.faces.len())?)?;
        output.write_u32::<LittleEndian>(self.flags)?;

        for point in &self.points {
            point.write(output)?;
        }

        for normal in &self.face_normals {
            output.write_f32::<LittleEndian>(normal.0)?;
            output.write_f32::<LittleEndian>(normal.1)?;
            output.write_f32::<LittleEndian>(normal.2)?;
        }

        for face in &self.faces {
            face.write(output)?;
        }

        output.write_all(b"TAGG")?;

        for tagg in &self.taggs {
            output.write_all(&[tagg.active])?;
            output.write_cstring(&tagg.name)?;
            let size = u32::try_from(tagg.data.len()).map_err(|_| invalid("tag data too large"))?;
            output.write_u32::<LittleEndian>(size)?;
            output.write_all(&tagg.data)?;
        }

        output.write_all(&[1])?;
        output.write_cstring("#EndOfFile#")?;
        output.write_u32::<LittleEndian>(0)?;

        output.write_f32::<LittleEndian>(self.resolution)?;

        Ok(())
    }
}

impl P3D {
    /// Reads an MLOD model.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not an MLOD model or is malformed.
    pub fn read<I: Read>(input: &mut I) -> Result<P3D, Error> {
        let mut reader = BufReader::new(input);

        let mut buffer = [0; 4];
        reader.read_exact(&mut buffer)?;
        match &buffer {
            b"MLOD" => {}
            b"ODOL" => {
                return Err(invalid(
                    "this is a binarized ODOL model; only MLOD models can be read",
                ));
            }
            _ => return Err(invalid("not an MLOD model")),
        }

        let version = reader.read_u32::<LittleEndian>()?;
        let num_lods = reader.read_u32::<LittleEndian>()?;
        let mut lods: Vec<LOD> = Vec::with_capacity(capacity(num_lods));

        for i in 0..num_lods {
            lods.push(LOD::read(&mut reader).prepend_error(format!("Failed to read LOD {i}:"))?);
        }

        Ok(P3D { version, lods })
    }

    /// Writes the model as MLOD.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        let mut writer = BufWriter::new(output);

        writer.write_all(b"MLOD")?;
        writer.write_u32::<LittleEndian>(self.version)?;
        let num_lods = u32::try_from(self.lods.len()).map_err(|_| invalid("too many LODs"))?;
        writer.write_u32::<LittleEndian>(num_lods)?;

        for lod in &self.lods {
            lod.write(&mut writer)?;
        }

        writer.flush()
    }

    /// Texture and material paths used by the faces of all LODs, with the number of faces using them.
    /// Empty names are left out.
    #[must_use]
    pub fn textures(&self) -> BTreeMap<&str, usize> {
        self.count_faces(|f| &f.texture)
    }

    /// Material (rvmat) paths used by the faces of all LODs, with the number of faces using them.
    /// Empty names are left out.
    #[must_use]
    pub fn materials(&self) -> BTreeMap<&str, usize> {
        self.count_faces(|f| &f.material)
    }

    fn count_faces<'a>(
        &'a self,
        field: impl Fn(&'a Face) -> &'a String,
    ) -> BTreeMap<&'a str, usize> {
        let mut map = BTreeMap::new();
        for face in self.lods.iter().flat_map(|l| l.faces.iter()) {
            let name = field(face);
            if !name.is_empty() {
                *map.entry(name.as_str()).or_insert(0) += 1;
            }
        }
        map
    }
}

/// Whether `rel` (relative, `\\` or `/` separated) exists below `root`, ignoring case like the game does.
fn exists_ignore_case(root: &Path, rel: &str) -> bool {
    let mut dir = root.to_path_buf();
    for part in rel.split('\\').filter(|p| !p.is_empty()) {
        let found = std::fs::read_dir(&dir).ok().and_then(|rd| {
            rd.filter_map(std::result::Result::ok)
                .find(|e| e.file_name().to_string_lossy().to_lowercase() == part)
                .map(|e| e.path())
        });
        match found {
            Some(p) => dir = p,
            None => return false,
        }
    }
    dir != root
}

/// Warns about textures and materials of the MLOD model at `path` that don't exist, before it
/// is handed to binarize.exe (which silently leaves the model's alpha flags unset instead).
///
/// References that start with the addon's `prefix` are looked up in the addon, whose files are
/// `addon_files` (lowercase, relative to the addon, separated by `\`). Anything else
/// (vanilla `dz\...`, other mods) can only be checked if it is below one of `search_roots`;
/// if it isn't, there is no way to know whether the reference is wrong, so it is not reported.
/// Procedural textures (`#(argb,...)`) are skipped. A model that can't be read only gets a warning.
pub fn check_dependencies<S: std::hash::BuildHasher>(
    path: &Path,
    display: &str,
    prefix: &str,
    addon_files: &HashSet<String, S>,
    search_roots: &[PathBuf],
) {
    let p3d = match File::open(path).and_then(|f| P3D::read(&mut BufReader::new(f))) {
        Ok(p) => p,
        Err(e) => {
            warning(
                format!(
                    "Cannot check textures and materials: {}",
                    e.to_string().replace('\n', " ")
                ),
                Some("model-unreadable"),
                (Some(display.to_string()), None),
            );
            return;
        }
    };

    let prefix = prefix.replace('/', "\\").trim_matches('\\').to_lowercase();
    let own = format!("{prefix}\\");

    let kinds: [(&str, &'static str, BTreeMap<&str, usize>); 2] = [
        ("texture", "missing-texture", p3d.textures()),
        ("material", "missing-material", p3d.materials()),
    ];
    for (kind, warning_name, refs) in &kinds {
        for name in refs.keys().filter(|n| !n.starts_with('#')) {
            let normalized = name
                .replace('/', "\\")
                .trim_start_matches('\\')
                .to_lowercase();
            let exists = if !prefix.is_empty() && normalized.starts_with(&own) {
                Some(addon_files.contains(&normalized[own.len()..]))
            } else if search_roots
                .iter()
                .any(|r| exists_ignore_case(r, &normalized))
            {
                Some(true)
            } else {
                None
            };
            if exists == Some(false) {
                warning(
                    format!("Model references {kind} \"{name}\", which is not in the addon."),
                    Some(*warning_name),
                    (Some(display.to_string()), None),
                );
            }
        }
    }
}

/// The 4 byte magic of a model file ("MLOD", "ODOL" or "BMTR"), if it is one. The reader is
/// put back to where it was.
///
/// # Errors
///
/// Returns an error if seeking or reading fails (a short read is not an error).
pub fn model_magic<I: Read + Seek>(input: &mut I) -> Result<Option<[u8; 4]>, Error> {
    let start = input.stream_position()?;
    let mut magic = [0; 4];
    let complete = input.read_exact(&mut magic).is_ok();
    input.seek(SeekFrom::Start(start))?;
    Ok(
        if complete && (&magic == b"MLOD" || &magic == b"ODOL" || &magic == b"BMTR") {
            Some(magic)
        } else {
            None
        },
    )
}

/// Prints what is in a model. For ODOL (and BMTR) only the format and version are known.
///
/// # Errors
///
/// Returns an error if the input is not a model or cannot be read.
#[expect(clippy::print_stdout, reason = "CLI command output")]
pub fn cmd_inspect<I: Read + Seek>(input: &mut I) -> Result<(), Error> {
    match model_magic(input)? {
        Some(ref m) if m == b"MLOD" => {}
        Some(m) => {
            input.seek(SeekFrom::Current(4))?;
            let version = input
                .read_u32::<LittleEndian>()
                .prepend_error("Failed to read the version:")?;
            let (name, what) = if &m == b"ODOL" {
                ("ODOL", "binarized model")
            } else {
                ("BMTR", "binarized animation")
            };
            println!("Format:  {name} ({what})");
            println!("Version: {version}");
            return Ok(());
        }
        None => return Err(error!("Not a P3D model (no MLOD, ODOL or BMTR header).")),
    }

    let p3d = P3D::read(input).prepend_error("Failed to read model:")?;

    println!("Format:  MLOD (editable model)");
    println!("Version: {}", p3d.version);
    println!("LODs:    {}", p3d.lods.len());
    println!();

    println!(
        "  #  {:24} {:>8} {:>8} {:>8} {:>8}",
        "LOD", "Points", "Faces", "Normals", "UV sets"
    );
    for (i, lod) in p3d.lods.iter().enumerate() {
        println!(
            "{:3}  {:24} {:8} {:8} {:8} {:8}",
            i,
            lod.name(),
            lod.points.len(),
            lod.faces.len(),
            lod.face_normals.len(),
            lod.uv_set_count()
        );
    }

    println!();
    println!("Textures:");
    for (name, count) in p3d.textures() {
        println!("- {name} ({count} faces)");
    }
    println!("Materials:");
    for (name, count) in p3d.materials() {
        println!("- {name} ({count} faces)");
    }

    println!();
    println!("Properties:");
    for (i, lod) in p3d.lods.iter().enumerate() {
        for (name, value) in lod.properties() {
            println!("- LOD {i}: {name} = {value}");
        }
    }

    println!();
    println!("Selections:");
    for (i, lod) in p3d.lods.iter().enumerate() {
        let selections = lod.selections();
        if !selections.is_empty() {
            println!("- LOD {} ({}): {}", i, lod.name(), selections.join(", "));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn mlod(lod_body: &[u8]) -> Vec<u8> {
        let mut b = b"MLOD".to_vec();
        b.extend_from_slice(&[0x01, 0x01, 0, 0, 1, 0, 0, 0]);
        b.extend_from_slice(lod_body);
        b
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(P3D::read(&mut Cursor::new(b"".to_vec())).is_err());
        assert!(P3D::read(&mut Cursor::new(b"XXXXXXXX".to_vec())).is_err());
        assert!(P3D::read(&mut Cursor::new(b"ODOL\x38\0\0\0".to_vec())).is_err());
        assert!(P3D::read(&mut Cursor::new(mlod(b"NOPE"))).is_err());
        // claims 4 billion points, has none
        let mut lod = b"P3DM".to_vec();
        lod.extend_from_slice(&[
            0x1c, 0, 0, 0, 0, 1, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        assert!(P3D::read(&mut Cursor::new(mlod(&lod))).is_err());
    }

    #[test]
    fn bad_face_vertex_count_is_an_error() {
        let mut lod = b"P3DM".to_vec();
        lod.extend_from_slice(&[
            0x1c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        ]);
        lod.extend_from_slice(&[7, 0, 0, 0]);
        let err = P3D::read(&mut Cursor::new(mlod(&lod))).unwrap_err();
        assert!(err.to_string().contains("7 vertices"), "{}", err);
    }
}
