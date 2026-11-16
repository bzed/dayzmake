//! Reading binarized ODOL models (version 56), the format of every `.p3d` in the DayZ game files.
//!
//! [`Odol::read`](crate::odol::Odol::read) reads what a listing needs: the model-level properties, the skeleton, the animation
//! classes and, per LOD, the vertex and face counts, textures, materials, proxies, named selections and
//! properties. It doesn't decode geometry. Every count and size in the file is checked against the bytes
//! left and against a limit before it is used, and a LOD that fails to parse is reported with its index and
//! file offset.
//!
//! The reader seeks a lot and reads small values; wrap a [`std::fs::File`] in a [`std::io::BufReader`].

use std::io::{Error, ErrorKind, Read, Seek, SeekFrom};

use byteorder::{LittleEndian, ReadBytesExt};

use crate::error::ErrorExt;

/// The only ODOL version this reader understands.
const SUPPORTED_VERSION: u32 = 56;
/// Longest string accepted.
const MAX_STRING: usize = 64 * 1024;
/// Most LODs, proxies, textures, materials, sections, selections, properties and similar lists.
const MAX_ITEMS: u32 = 1 << 16;
/// Largest decompressed array.
const MAX_ARRAY_BYTES: usize = 64 * 1024 * 1024;
/// Arrays of at least this many bytes are LZO compressed.
const COMPRESSION_THRESHOLD: usize = 1024;

fn invalid<S: Into<String>>(msg: S) -> Error {
    Error::new(ErrorKind::InvalidData, msg.into())
}

/// A binarized model: the model-level facts and the LODs.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Odol {
    /// Format version (always 56).
    pub version: u32,
    /// The model's `class` property (for example `house`); empty if none.
    pub property_class: String,
    /// The model's `damage` property (for example `tent`); empty if none.
    pub property_damage: String,
    /// Skeleton name, empty if the model has none.
    pub skeleton_name: String,
    /// Number of bones in the skeleton.
    pub bone_count: usize,
    /// Mass of the model.
    pub mass: f32,
    /// Bounding box minimum, model space.
    pub bbox_min: [f32; 3],
    /// Bounding box maximum, model space.
    pub bbox_max: [f32; 3],
    /// Animation classes, in file order.
    pub animations: Vec<OdolAnimation>,
    /// The LODs, in file order.
    pub lods: Vec<OdolLod>,
}

/// An animation class of the model (a `model.cfg` animation, baked into the model).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OdolAnimation {
    /// Animation type: 0 rotation, 1 to 3 rotationX/Y/Z, 4 translation, 5 to 7 translationX/Y/Z, 8 direct,
    /// 9 hide.
    pub kind: u32,
    /// Class name (for example `Door1`).
    pub name: String,
    /// Animation source (for example `door1`).
    pub source: String,
}

/// A LOD of a binarized model.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OdolLod {
    /// LOD resolution; special LODs (geometry, memory, ...) have large well known values.
    pub resolution: f32,
    /// Number of vertices.
    pub vertex_count: u32,
    /// Number of faces.
    pub face_count: u32,
    /// Texture paths used by faces.
    pub textures: Vec<String>,
    /// Materials with the texture paths of their stages.
    pub materials: Vec<OdolMaterial>,
    /// Proxy model paths.
    pub proxies: Vec<String>,
    /// Named selection names.
    pub selections: Vec<String>,
    /// LOD properties as (name, value) pairs.
    pub properties: Vec<(String, String)>,
}

/// A material of a LOD.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OdolMaterial {
    /// Material path (`.rvmat`), empty for built-in materials.
    pub name: String,
    /// Texture paths of the stages, empty entries left out.
    pub textures: Vec<String>,
}

impl OdolLod {
    /// Human readable name of this LOD (for example "Geometry", or its resolution), as for MLOD LODs.
    #[must_use]
    pub fn name(&self) -> String {
        crate::p3d::resolution_name(self.resolution)
    }
}

/// A reader that knows how many bytes are left.
struct Rd<'a, R: Read + Seek> {
    r: &'a mut R,
    len: u64,
}

impl<R: Read + Seek> Rd<'_, R> {
    fn pos(&mut self) -> Result<u64, Error> {
        self.r.stream_position()
    }

    fn left(&mut self) -> Result<u64, Error> {
        let pos = self.pos()?;
        Ok(self.len.saturating_sub(pos))
    }

    fn need(&mut self, n: u64) -> Result<(), Error> {
        if n > self.left()? {
            return Err(invalid(format!("{n} bytes wanted, the file ends sooner")));
        }
        Ok(())
    }

    fn skip(&mut self, n: u64) -> Result<(), Error> {
        self.need(n)?;
        let n = i64::try_from(n).map_err(|_| invalid("skip too long"))?;
        self.r.seek(SeekFrom::Current(n))?;
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, Error> {
        self.r.read_u8()
    }

    fn u32(&mut self) -> Result<u32, Error> {
        self.r.read_u32::<LittleEndian>()
    }

    fn f32(&mut self) -> Result<f32, Error> {
        self.r.read_f32::<LittleEndian>()
    }

    fn vec3(&mut self) -> Result<[f32; 3], Error> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// A NUL terminated string of at most [`MAX_STRING`] bytes.
    fn cstr(&mut self) -> Result<String, Error> {
        let mut bytes = Vec::new();
        loop {
            let b = self.u8()?;
            if b == 0 {
                break;
            }
            if bytes.len() >= MAX_STRING {
                return Err(invalid("string without end"));
            }
            bytes.push(b);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A count of elements that take at least `min_size` bytes each, checked against the bytes left and `max`.
    fn count(&mut self, what: &str, max: u32, min_size: u64) -> Result<usize, Error> {
        let n = self.u32()?;
        if n > max {
            return Err(invalid(format!("too many {what}: {n}")));
        }
        let bytes = u64::from(n)
            .checked_mul(min_size)
            .ok_or_else(|| invalid(format!("too many {what}: {n}")))?;
        if bytes > self.left()? {
            return Err(invalid(format!(
                "{n} {what} don't fit into the rest of the file"
            )));
        }
        usize::try_from(n).map_err(|_| invalid(format!("too many {what}: {n}")))
    }

    /// The bytes of an array of `count` elements of `size` bytes: raw below the compression threshold,
    /// an LZO stream (without a stored length; the reader stops after its end marker) from it.
    fn array(&mut self, count: usize, size: usize) -> Result<Vec<u8>, Error> {
        let total = count
            .checked_mul(size)
            .filter(|t| *t <= MAX_ARRAY_BYTES)
            .ok_or_else(|| invalid(format!("array of {count} elements is too large")))?;
        if total < COMPRESSION_THRESHOLD {
            self.need(total as u64)?;
            let mut buf = vec![0; total];
            self.r.read_exact(&mut buf)?;
            return Ok(buf);
        }
        let out = lzokay_native::decompress_bounded(&mut *self.r, total)
            .map_err(|e| invalid(format!("LZO stream of {total} bytes: {e}")))?;
        if out.len() != total {
            return Err(invalid(format!(
                "LZO stream gave {} bytes, {total} declared",
                out.len()
            )));
        }
        Ok(out)
    }

    /// A counted array whose contents are not needed.
    fn skip_array(&mut self, size: usize) -> Result<(), Error> {
        let count = usize::try_from(self.u32()?).map_err(|_| invalid("count too large"))?;
        self.array(count, size).map(drop)
    }
}

impl Odol {
    /// Reads a model.
    ///
    /// # Errors
    /// Fails for a file that is not ODOL, for a version other than 56, for truncated or corrupt data
    /// and for sizes above the reader's limits. An error in a LOD names the LOD and its file offset.
    pub fn read<R: Read + Seek>(input: &mut R) -> Result<Odol, Error> {
        let len = input.seek(SeekFrom::End(0))?;
        input.seek(SeekFrom::Start(0))?;
        let mut rd = Rd { r: input, len };

        let mut magic = [0; 4];
        rd.r.read_exact(&mut magic)?;
        if &magic != b"ODOL" {
            return Err(invalid("not an ODOL model"));
        }
        let version = rd.u32()?;
        if version != SUPPORTED_VERSION {
            return Err(invalid(format!(
                "ODOL version {version} is not supported, only {SUPPORTED_VERSION}"
            )));
        }
        let lod_count = rd.count("LODs", MAX_ITEMS, 12)?;
        let mut resolutions = Vec::with_capacity(lod_count);
        for _ in 0..lod_count {
            resolutions.push(rd.f32()?);
        }

        let mut model = Odol {
            version,
            property_class: String::new(),
            property_damage: String::new(),
            skeleton_name: String::new(),
            bone_count: 0,
            mass: 0.0,
            bbox_min: [0.0; 3],
            bbox_max: [0.0; 3],
            animations: Vec::new(),
            lods: Vec::new(),
        };
        model_info(&mut rd, &mut model).prepend_error("ODOL model info:")?;
        if rd.u8()? != 0 {
            animations(&mut rd, &mut model, lod_count).prepend_error("ODOL animations:")?;
        }

        let mut starts = Vec::with_capacity(lod_count);
        for _ in 0..lod_count {
            starts.push(rd.u32()?);
        }
        let mut ends = Vec::with_capacity(lod_count);
        for _ in 0..lod_count {
            ends.push(rd.u32()?);
        }

        for (index, resolution) in resolutions.iter().enumerate() {
            let start = u64::from(starts.get(index).copied().unwrap_or(0));
            let end = u64::from(ends.get(index).copied().unwrap_or(0));
            let context = format!("ODOL LOD {index} at offset {start}:");
            if start >= len {
                return Err(invalid(format!(
                    "{context}\nstarts past the end of the file"
                )));
            }
            rd.r.seek(SeekFrom::Start(start))?;
            let lod = read_lod(&mut rd, *resolution, end).prepend_error(&context)?;
            model.lods.push(lod);
        }
        Ok(model)
    }
}

fn model_info<R: Read + Seek>(rd: &mut Rd<'_, R>, model: &mut Odol) -> Result<(), Error> {
    // index, two sphere radii, remarks, hints, aiming center, colors, view density
    rd.skip(48)?;
    model.bbox_min = rd.vec3()?;
    model.bbox_max = rd.vec3()?;
    // visual box, three centers, inverse inertia, 5 flags, 8 floats, then 3 flags with a shadow mode and offset
    rd.skip(24 + 36 + 36 + 5 + 32 + 1 + 4 + 1 + 4 + 1)?;

    model.skeleton_name = rd.cstr()?;
    if !model.skeleton_name.is_empty() {
        rd.u8()?; // is_discrete
        let bones = rd.count("bones", MAX_ITEMS, 2)?;
        for _ in 0..bones {
            rd.cstr()?;
            rd.cstr()?;
        }
        model.bone_count = bones;
        rd.cstr()?; // pivots name
    }
    rd.u8()?; // map type
    rd.skip_array(4)?; // mass array
    model.mass = rd.f32()?;
    // inverse mass, alt masses, the special LOD indices, min shadow, can blend
    rd.skip(4 + 4 + 4 + 13 + 4 + 1)?;
    model.property_class = rd.cstr()?;
    model.property_damage = rd.cstr()?;
    rd.u8()?; // property_frequent
    let extra = rd.count("model strings", MAX_ITEMS, 1)?;
    for _ in 0..extra {
        rd.cstr()?;
    }
    rd.skip(1)?; // unknown flag
    Ok(())
}

fn animations<R: Read + Seek>(
    rd: &mut Rd<'_, R>,
    model: &mut Odol,
    lod_count: usize,
) -> Result<(), Error> {
    let classes = rd.count("animation classes", MAX_ITEMS, 40)?;
    let mut kinds = Vec::with_capacity(classes);
    for _ in 0..classes {
        let kind = rd.u32()?;
        let name = rd.cstr()?;
        let source = rd.cstr()?;
        rd.skip(20)?; // min and max phase, min and max value, source address
        match kind {
            0..=7 => rd.skip(8)?,
            8 => rd.skip(32)?,
            9 => rd.skip(4)?,
            _ => return Err(invalid(format!("unknown animation type {kind}"))),
        }
        kinds.push(kind);
        model.animations.push(OdolAnimation { kind, name, source });
    }
    // Without animation classes the block count is 0 and no blocks follow.
    let resolutions = rd.u32()?;
    let blocks = usize::try_from(resolutions).map_err(|_| invalid("too many animation blocks"))?;
    if blocks != lod_count && !(classes == 0 && blocks == 0) {
        return Err(invalid(format!(
            "{resolutions} animation LOD blocks for {lod_count} LODs"
        )));
    }
    for _ in 0..blocks {
        let bones = rd.count("animated bones", MAX_ITEMS, 4)?;
        for _ in 0..bones {
            let n = rd.count("bone animations", MAX_ITEMS, 4)?;
            rd.skip(u64::try_from(n).map_err(|_| invalid("too many"))? * 4)?;
        }
    }
    for _ in 0..blocks {
        for kind in &kinds {
            let bone = rd.u32()?;
            if bone != u32::MAX && *kind != 8 && *kind != 9 {
                rd.skip(24)?;
            }
        }
    }
    Ok(())
}

fn read_lod<R: Read + Seek>(
    rd: &mut Rd<'_, R>,
    resolution: f32,
    end: u64,
) -> Result<OdolLod, Error> {
    let proxy_count = rd.count("proxies", MAX_ITEMS, 1 + 48 + 16)?;
    let mut proxies = Vec::with_capacity(proxy_count);
    for _ in 0..proxy_count {
        proxies.push(rd.cstr()?);
        rd.skip(48 + 16)?;
    }
    let items = rd.count("items", u32::MAX, 4)?;
    rd.skip(u64::try_from(items).map_err(|_| invalid("too many"))? * 4)?;
    let links = rd.count("bone links", MAX_ITEMS * 16, 4)?;
    for _ in 0..links {
        let n = rd.count("bone link entries", u32::MAX, 4)?;
        rd.skip(u64::try_from(n).map_err(|_| invalid("too many"))? * 4)?;
    }
    let vertex_count = rd.u32()?;
    // face area, hints, bounding box, center, radius
    rd.skip(4 + 8 + 36 + 4)?;
    let texture_count = rd.count("textures", MAX_ITEMS, 1)?;
    let mut textures = Vec::with_capacity(texture_count);
    for _ in 0..texture_count {
        textures.push(rd.cstr()?);
    }
    let material_count = rd.count("materials", MAX_ITEMS, 8)?;
    let mut materials = Vec::with_capacity(material_count);
    for _ in 0..material_count {
        materials.push(read_material(rd)?);
    }
    rd.skip_array(2)?; // edges
    rd.skip_array(2)?;
    let face_count = rd.u32()?;
    let face_bytes = rd.u32()?;
    rd.skip(2)?;
    if u64::from(face_count) > rd.left()? / 7 {
        return Err(invalid(format!(
            "{face_count} faces don't fit into the rest of the file"
        )));
    }
    // Each face is a u8 vertex count (3 or 4) and that many u16 indices. The declared size counts every
    // face as the vertex count plus one u16 each.
    let mut declared = 0u64;
    for _ in 0..face_count {
        let n = rd.u8()?;
        if n != 3 && n != 4 {
            return Err(invalid(format!("a face with {n} vertices")));
        }
        rd.skip(u64::from(n) * 2)?;
        declared += u64::from(n) * 2 + 2;
    }
    if declared != u64::from(face_bytes) {
        return Err(invalid(format!(
            "the faces take {declared} bytes, {face_bytes} declared"
        )));
    }

    let sections = rd.count("sections", MAX_ITEMS, 34)?;
    for _ in 0..sections {
        rd.skip(26)?;
        let material_index = rd.u32()?;
        if material_index == u32::MAX {
            rd.cstr()?;
        }
        let stages = rd.count("section stages", MAX_ITEMS, 4)?;
        rd.skip(u64::try_from(stages).map_err(|_| invalid("too many"))? * 4)?;
    }
    let selection_count = rd.count("selections", MAX_ITEMS, 22)?;
    let mut selections = Vec::with_capacity(selection_count);
    for _ in 0..selection_count {
        selections.push(rd.cstr()?);
        rd.skip_array(2)?; // faces
        rd.skip(4 + 1)?; // zero, is_sectional
        rd.skip_array(4)?; // sections
        rd.skip_array(2)?; // vertices
        rd.skip_array(1)?; // weights
    }
    let property_count = rd.count("properties", MAX_ITEMS, 2)?;
    let mut properties = Vec::with_capacity(property_count);
    for _ in 0..property_count {
        let name = rd.cstr()?;
        let value = rd.cstr()?;
        properties.push((name, value));
    }
    if end < rd.len && rd.pos()? > end {
        return Err(invalid("the LOD reads past its end offset"));
    }
    Ok(OdolLod {
        resolution,
        vertex_count,
        face_count,
        textures,
        materials,
        proxies,
        selections,
        properties,
    })
}

/// The only material version the game's models use.
const MATERIAL_VERSION: u32 = 20;

/// A stage texture: render flags, path, stage id, "use world environment" flag.
fn read_stage<R: Read + Seek>(rd: &mut Rd<'_, R>) -> Result<String, Error> {
    rd.skip(4)?;
    let texture = rd.cstr()?;
    rd.skip(4 + 1)?;
    Ok(texture)
}

fn read_material<R: Read + Seek>(rd: &mut Rd<'_, R>) -> Result<OdolMaterial, Error> {
    let name = rd.cstr()?;
    let version = rd.u32()?;
    if version != MATERIAL_VERSION {
        return Err(invalid(format!(
            "material {name:?} has version {version}, only {MATERIAL_VERSION} is supported"
        )));
    }
    // colors and lighting values, specular power, pixel and vertex shader, main light, fog mode
    rd.skip(200 + 4 + 16)?;
    rd.cstr()?; // surface
    rd.skip(8)?; // render flag count and flags
    let stages = rd.count("material stages", MAX_ITEMS, 10)?;
    let transforms = rd.count("material transforms", MAX_ITEMS, 52)?;
    let mut textures = Vec::new();
    for _ in 0..stages {
        let texture = read_stage(rd)?;
        if !texture.is_empty() {
            textures.push(texture);
        }
    }
    rd.skip(u64::try_from(transforms).map_err(|_| invalid("too many"))? * 52)?;
    read_stage(rd)?; // the dummy stage after the transforms
    Ok(OdolMaterial { name, textures })
}
