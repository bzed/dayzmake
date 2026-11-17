//! The wire format of the navmesh data server (messages 3, 4 and 5).
//!
//! All integers and floats are little-endian. This module parses and encodes message *payloads*; the
//! frame (`u32 length`, `u32 messageId`) belongs to the transport. [`WorldParams::parse_message`]
//! takes a payload that starts with the protocol version, [`WorldParams::parse`] one without it.
//!
//! The readers check every count against the bytes left before they allocate, so a hostile payload
//! cannot make them allocate more than a small multiple of its own size.

use std::io;
use std::ops::Range;

use crate::navmesh::num::i32_to_f32;

/// The protocol version of the world params message.
pub const PROTOCOL_VERSION: u32 = 10;
/// The default TCP port of the data server.
pub const DEFAULT_PORT: u16 = 36533;

const WORLD_BLOCK_LEN: usize = 84;
const CELL_CLASS_LEN: usize = 16;
const LINK_CLASS_LEN: usize = 52;
const PART_LEN: usize = 16;
const VERTEX_LEN: usize = 12;
/// Smallest group: area, partCount, vertexCount.
const MIN_GROUP_LEN: usize = 12;
const MAX_CELL_CLASSES: usize = 64;
const MAX_LINK_CLASSES: usize = 256;
const MAX_TILES: i64 = 1 << 28;
const MAX_CELLS: i32 = 2048;
const MAX_GROUPS: usize = 65536;
const MAX_PARTS: usize = 1_048_576;
const MAX_VERTICES: i32 = 1 << 26;

/// A bounds-checked little-endian cursor over a byte slice.
struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    fn remaining(&self) -> usize {
        self.buf.len()
    }

    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let (head, tail) = self
            .buf
            .split_first_chunk::<N>()
            .ok_or_else(|| error!("message truncated"))?;
        self.buf = tail;
        Ok(*head)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn f32x3(&mut self) -> io::Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// A count read from the payload as `usize`, checked against `limit` and against the bytes left
    /// (`count * element_len`) before anything is allocated for it.
    fn count(&mut self, what: &str, limit: usize, element_len: usize) -> io::Result<usize> {
        let n =
            usize::try_from(self.u32()?).map_err(|_| error!("implausible {what} {}", u32::MAX))?;
        if n > limit {
            return Err(error!("implausible {what} {n}"));
        }
        self.check_available(n, element_len)?;
        Ok(n)
    }

    fn check_available(&self, n: usize, element_len: usize) -> io::Result<()> {
        let bytes = n
            .checked_mul(element_len)
            .ok_or_else(|| error!("message truncated"))?;
        if bytes > self.remaining() {
            return Err(error!("message truncated"));
        }
        Ok(())
    }
}

/// The 84-byte world block of the world params message (spec 3.4).
///
/// `obstacles` and `filter_isolated_islands` are one byte plus three pad bytes on the wire. The raw
/// four bytes are kept in `obstacles_raw` and `filter_raw` so that parsing and encoding round-trips.
/// Parsing sets the `bool` to `raw[0] != 0`. Encoding writes the raw bytes, but when the `bool`
/// disagrees with `raw[0]` (a value built from [`Default`] with only the `bool` set), byte 0 is
/// replaced by `u8::from(bool)`.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct WorldBlock {
    /// World size in x, metres.
    pub world_size_x: f32,
    /// World size in z, metres.
    pub world_size_z: f32,
    /// Tile width, metres.
    pub tile_width: f32,
    /// Tile height (extent in z), metres.
    pub tile_height: f32,
    /// Number of tiles in x.
    pub tiles_x: i32,
    /// Number of tiles in z.
    pub tiles_z: i32,
    /// Top of the sea box (a live tide value), metres.
    pub deep_sea_height: f32,
    /// Water deeper than this becomes deep water, metres.
    pub deep_water_depth: f32,
    /// Contour simplification error, metres.
    pub contour_max_error: f32,
    /// Extract tree parts as cylinder obstacles.
    pub obstacles: bool,
    /// Scheduler block edge length in tiles.
    pub block_size: i32,
    /// Run the island filter.
    pub filter_isolated_islands: bool,
    /// Seed position of the island filter.
    pub seed_position: [f32; 3],
    /// Agent stand height, metres.
    pub stand_height: f32,
    /// Agent crouch height, metres.
    pub crouch_height: f32,
    /// Agent prone height, metres.
    pub prone_height: f32,
    /// Agent diameter, metres.
    pub diameter: f32,
    /// Agent maximum step height, metres.
    pub max_step_height: f32,
    /// Agent maximum slope, degrees.
    pub max_slope: f32,
    /// The four wire bytes of `obstacles` (flag plus pad), kept for round-tripping.
    pub obstacles_raw: [u8; 4],
    /// The four wire bytes of `filter_isolated_islands` (flag plus pad), kept for round-tripping.
    pub filter_raw: [u8; 4],
}

impl WorldBlock {
    fn read(r: &mut Reader<'_>) -> io::Result<Self> {
        let world_size_x = r.f32()?;
        let world_size_z = r.f32()?;
        let tile_width = r.f32()?;
        let tile_height = r.f32()?;
        let tiles_x = r.i32()?;
        let tiles_z = r.i32()?;
        let deep_sea_height = r.f32()?;
        let deep_water_depth = r.f32()?;
        let contour_max_error = r.f32()?;
        let obstacles_raw = r.array::<4>()?;
        let block_size = r.i32()?;
        let filter_raw = r.array::<4>()?;
        Ok(Self {
            world_size_x,
            world_size_z,
            tile_width,
            tile_height,
            tiles_x,
            tiles_z,
            deep_sea_height,
            deep_water_depth,
            contour_max_error,
            obstacles: obstacles_raw[0] != 0,
            block_size,
            filter_isolated_islands: filter_raw[0] != 0,
            seed_position: r.f32x3()?,
            stand_height: r.f32()?,
            crouch_height: r.f32()?,
            prone_height: r.f32()?,
            diameter: r.f32()?,
            max_step_height: r.f32()?,
            max_slope: r.f32()?,
            obstacles_raw,
            filter_raw,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        for v in [
            self.world_size_x,
            self.world_size_z,
            self.tile_width,
            self.tile_height,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.tiles_x.to_le_bytes());
        out.extend_from_slice(&self.tiles_z.to_le_bytes());
        for v in [
            self.deep_sea_height,
            self.deep_water_depth,
            self.contour_max_error,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&flag_bytes(self.obstacles, self.obstacles_raw));
        out.extend_from_slice(&self.block_size.to_le_bytes());
        out.extend_from_slice(&flag_bytes(self.filter_isolated_islands, self.filter_raw));
        for v in self.seed_position {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [
            self.stand_height,
            self.crouch_height,
            self.prone_height,
            self.diameter,
            self.max_step_height,
            self.max_slope,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// The raw flag bytes, with byte 0 forced to agree with `flag`.
fn flag_bytes(flag: bool, mut raw: [u8; 4]) -> [u8; 4] {
    if (raw[0] != 0) != flag {
        raw[0] = u8::from(flag);
    }
    raw
}

/// One cell class: the voxel grid resolution of a tile (spec 3.4).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct CellClass {
    /// Number of cells across the tile width.
    pub cells: i32,
    /// Second cell count (observed equal to `cells`; unused).
    pub cells2: u32,
    /// Width in metres covered by `cells`.
    pub width: f32,
    /// Unused field (0 in test fixtures).
    pub unused: u32,
}

impl CellClass {
    /// The cell size in metres: `width / float(cells)`.
    #[must_use]
    pub fn cell_size(&self) -> f32 {
        self.width / i32_to_f32(self.cells)
    }
}

/// One link class: an off-mesh jump or climb link type (spec 3.4).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct LinkClass {
    /// Becomes the off-mesh connection `userId`.
    pub index: u32,
    /// Name hash (unused).
    pub name_hash: u32,
    /// Type id, -1 if unset (unused).
    pub type_id: i32,
    /// Jump height, metres.
    pub jump_height: f32,
    /// Jump length, metres.
    pub jump_length: f32,
    /// Minimum jump drop, metres.
    pub jump_dropdown_min: f32,
    /// Maximum jump drop, metres.
    pub jump_dropdown_max: f32,
    /// Minimum centre height (its sign selects the obstacle-height test).
    pub min_center_height: f32,
    /// Agent height, metres.
    pub agent_height: f32,
    /// Maximum step height, metres.
    pub max_step_height: f32,
    /// Agent radius, metres.
    pub agent_radius: f32,
    /// Poly flags assigned to the generated off-mesh polygons.
    pub flags: u16,
    /// Area id assigned to them (must fit 6 bits).
    pub area: u8,
    /// Padding byte.
    pub pad: u8,
    /// Debug colour (unused).
    pub color: i32,
}

/// The world params message (id 4): world block, cell classes and link classes.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct WorldParams {
    /// The world block.
    pub world: WorldBlock,
    /// The cell classes, at most 64.
    pub cell_classes: Vec<CellClass>,
    /// The link classes, at most 256.
    pub link_classes: Vec<LinkClass>,
}

impl WorldParams {
    /// Parses a world params payload that starts with the protocol version.
    ///
    /// # Errors
    ///
    /// Fails when the payload is shorter than 4 bytes ("world params truncated"), when the version is
    /// not [`PROTOCOL_VERSION`] ("server protocol version V, expected 10"), and for every error of
    /// [`WorldParams::parse`].
    pub fn parse_message(payload: &[u8]) -> io::Result<Self> {
        let mut r = Reader::new(payload);
        let version = r.u32().map_err(|_| error!("world params truncated"))?;
        if version != PROTOCOL_VERSION {
            return Err(error!(
                "server protocol version {version}, expected {PROTOCOL_VERSION}"
            ));
        }
        Self::parse(r.buf)
    }

    /// Parses a world params payload without the protocol version.
    ///
    /// # Errors
    ///
    /// Fails with the texts of spec 12.1: "message truncated", "unexpected bytes after world params",
    /// "implausible cell class count N", "implausible link count N", "invalid tile grid", "invalid
    /// tile size", "invalid cell class" and "implausible cell count" (more than 2048 cells).
    pub fn parse(payload: &[u8]) -> io::Result<Self> {
        let mut r = Reader::new(payload);
        let world = WorldBlock::read(&mut r)?;
        let n = r.count("cell class count", MAX_CELL_CLASSES, CELL_CLASS_LEN)?;
        let mut cell_classes = Vec::with_capacity(n);
        for _ in 0..n {
            cell_classes.push(CellClass {
                cells: r.i32()?,
                cells2: r.u32()?,
                width: r.f32()?,
                unused: r.u32()?,
            });
        }
        let m = r.count("link count", MAX_LINK_CLASSES, LINK_CLASS_LEN)?;
        let mut link_classes = Vec::with_capacity(m);
        for _ in 0..m {
            link_classes.push(LinkClass {
                index: r.u32()?,
                name_hash: r.u32()?,
                type_id: r.i32()?,
                jump_height: r.f32()?,
                jump_length: r.f32()?,
                jump_dropdown_min: r.f32()?,
                jump_dropdown_max: r.f32()?,
                min_center_height: r.f32()?,
                agent_height: r.f32()?,
                max_step_height: r.f32()?,
                agent_radius: r.f32()?,
                flags: r.u16()?,
                area: r.u8()?,
                pad: r.u8()?,
                color: r.i32()?,
            });
        }
        if r.remaining() != 0 {
            return Err(error!("unexpected bytes after world params"));
        }
        let tiles = i64::from(world.tiles_x) * i64::from(world.tiles_z);
        if world.tiles_x <= 0 || world.tiles_z <= 0 || tiles > MAX_TILES {
            return Err(error!(
                "invalid tile grid {} x {}",
                world.tiles_x, world.tiles_z
            ));
        }
        // `!(x > 0.0)` is deliberate: NaN fails.
        #[expect(
            clippy::neg_cmp_op_on_partial_ord,
            reason = "NaN must fail the check, as in the spec"
        )]
        if !(world.tile_width > 0.0) || !(world.tile_height > 0.0) {
            return Err(error!(
                "invalid tile size {} x {}",
                world.tile_width, world.tile_height
            ));
        }
        for (i, class) in cell_classes.iter().enumerate() {
            #[expect(
                clippy::neg_cmp_op_on_partial_ord,
                reason = "NaN must fail the check, as in the spec"
            )]
            if class.cells <= 0 || !(class.width > 0.0) {
                return Err(error!("invalid cell class {i}"));
            }
            if class.cells > MAX_CELLS {
                return Err(error!(
                    "implausible cell count {} in cell class {i}",
                    class.cells
                ));
            }
        }
        Ok(Self {
            world,
            cell_classes,
            link_classes,
        })
    }

    /// Encodes the payload without the protocol version.
    ///
    /// Class counts above `u32::MAX` are not representable and are clamped; no real value comes
    /// near that.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            WORLD_BLOCK_LEN
                + 8
                + self.cell_classes.len() * CELL_CLASS_LEN
                + self.link_classes.len() * LINK_CLASS_LEN,
        );
        self.world.write(&mut out);
        out.extend_from_slice(&len_u32(self.cell_classes.len()).to_le_bytes());
        for c in &self.cell_classes {
            out.extend_from_slice(&c.cells.to_le_bytes());
            out.extend_from_slice(&c.cells2.to_le_bytes());
            out.extend_from_slice(&c.width.to_le_bytes());
            out.extend_from_slice(&c.unused.to_le_bytes());
        }
        out.extend_from_slice(&len_u32(self.link_classes.len()).to_le_bytes());
        for l in &self.link_classes {
            out.extend_from_slice(&l.index.to_le_bytes());
            out.extend_from_slice(&l.name_hash.to_le_bytes());
            out.extend_from_slice(&l.type_id.to_le_bytes());
            for v in [
                l.jump_height,
                l.jump_length,
                l.jump_dropdown_min,
                l.jump_dropdown_max,
                l.min_center_height,
                l.agent_height,
                l.max_step_height,
                l.agent_radius,
            ] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&l.flags.to_le_bytes());
            out.push(l.area);
            out.push(l.pad);
            out.extend_from_slice(&l.color.to_le_bytes());
        }
        out
    }
}

fn len_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// One part of a tile group: an object (or a door state) and where its vertices start.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TilePart {
    /// Part index, copied into the output's extra section.
    pub part_index: u32,
    /// Object id.
    pub object_id: u64,
    /// Index of the part's first vertex in the group's vertex list.
    pub first_vertex: u32,
}

/// One geometry group of a tile: an area id, its parts and its vertices.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TileGroup {
    /// Input area id (1 terrain, 2 water, 8 building, 10 tree, 12/13 door, 15 ladder, ...).
    pub area: u32,
    /// The parts, in vertex order.
    pub parts: Vec<TilePart>,
    /// The vertices (x, y up, z): a triangle soup, or vertex pairs for ladders.
    pub verts: Vec<[f32; 3]>,
}

/// A tile input message (id 5): one tile's geometry.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TileInput {
    /// Tile x.
    pub x: i32,
    /// Tile z.
    pub z: i32,
    /// Index into the cell-class list.
    pub cell_class: u32,
    /// Tile bounds minimum.
    pub bmin: [f32; 3],
    /// Tile bounds maximum.
    pub bmax: [f32; 3],
    /// The geometry groups.
    pub groups: Vec<TileGroup>,
}

impl TileInput {
    /// Parses a tile input payload.
    ///
    /// # Errors
    ///
    /// Fails with the texts of spec 12.1: "message truncated", "implausible group count N",
    /// "implausible part count N", "implausible vertex count N", "part vertex range out of bounds",
    /// "non-finite vertex" and "unexpected bytes after tile record".
    pub fn parse(payload: &[u8]) -> io::Result<Self> {
        let mut r = Reader::new(payload);
        let x = r.i32()?;
        let z = r.i32()?;
        let cell_class = r.u32()?;
        let group_count = r.count("group count", MAX_GROUPS, MIN_GROUP_LEN)?;
        let bmin = r.f32x3()?;
        let bmax = r.f32x3()?;
        let mut groups = Vec::with_capacity(group_count);
        for _ in 0..group_count {
            groups.push(read_group(&mut r)?);
        }
        if r.remaining() != 0 {
            return Err(error!("unexpected bytes after tile record"));
        }
        Ok(Self {
            x,
            z,
            cell_class,
            bmin,
            bmax,
            groups,
        })
    }

    /// Encodes the payload.
    ///
    /// Counts above the wire's range are clamped; no real value comes near that.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.z.to_le_bytes());
        out.extend_from_slice(&self.cell_class.to_le_bytes());
        out.extend_from_slice(&len_u32(self.groups.len()).to_le_bytes());
        for v in self.bmin.iter().chain(&self.bmax) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for g in &self.groups {
            out.extend_from_slice(&g.area.to_le_bytes());
            out.extend_from_slice(&len_u32(g.parts.len()).to_le_bytes());
            for p in &g.parts {
                out.extend_from_slice(&p.part_index.to_le_bytes());
                out.extend_from_slice(&p.object_id.to_le_bytes());
                out.extend_from_slice(&p.first_vertex.to_le_bytes());
            }
            let count = i32::try_from(g.verts.len()).unwrap_or(i32::MAX);
            out.extend_from_slice(&count.to_le_bytes());
            for v in g.verts.iter().flatten() {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    /// The vertex range of part `part` of group `group`: from its `first_vertex` to the next part's
    /// (or the end of the group's vertices for the last part). `None` when the group or part does not
    /// exist or the range is out of bounds.
    #[must_use]
    pub fn part_vertex_range(&self, group: usize, part: usize) -> Option<Range<usize>> {
        let g = self.groups.get(group)?;
        let start = usize::try_from(g.parts.get(part)?.first_vertex).ok()?;
        let end = match g.parts.get(part.checked_add(1)?) {
            Some(next) => usize::try_from(next.first_vertex).ok()?,
            None => g.verts.len(),
        };
        (start <= end && end <= g.verts.len()).then_some(start..end)
    }
}

fn read_group(r: &mut Reader<'_>) -> io::Result<TileGroup> {
    let area = r.u32()?;
    let part_count = r.count("part count", MAX_PARTS, PART_LEN)?;
    let mut parts = Vec::with_capacity(part_count);
    for _ in 0..part_count {
        parts.push(TilePart {
            part_index: r.u32()?,
            object_id: r.u64()?,
            first_vertex: r.u32()?,
        });
    }
    let vertex_count = r.i32()?;
    if !(0..=MAX_VERTICES).contains(&vertex_count) {
        return Err(error!("implausible vertex count {vertex_count}"));
    }
    let vertex_count = usize::try_from(vertex_count)
        .map_err(|_| error!("implausible vertex count {vertex_count}"))?;
    r.check_available(vertex_count, VERTEX_LEN)?;
    let mut previous = 0usize;
    for part in &parts {
        let first = usize::try_from(part.first_vertex)
            .map_err(|_| error!("part vertex range out of bounds"))?;
        if first < previous || first > vertex_count {
            return Err(error!("part vertex range out of bounds"));
        }
        previous = first;
    }
    let mut verts = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        let v = r.f32x3()?;
        if !v.iter().all(|c| c.is_finite()) {
            return Err(error!("non-finite vertex"));
        }
        verts.push(v);
    }
    Ok(TileGroup { area, parts, verts })
}

/// Encodes a tile request (message 3, spec 3.3): tile x and z, then the tile's min and max x and z
/// as `float(x) * tileWidth`, `float(z) * tileHeight`, `float(x + 1) * tileWidth` and
/// `float(z + 1) * tileHeight`.
#[must_use]
pub fn encode_tile_request(x: i32, z: i32, world: &WorldBlock) -> [u8; 24] {
    let mut out = [0u8; 24];
    let fields = [
        x.to_le_bytes(),
        z.to_le_bytes(),
        (i32_to_f32(x) * world.tile_width).to_le_bytes(),
        (i32_to_f32(z) * world.tile_height).to_le_bytes(),
        (i32_to_f32(x.saturating_add(1)) * world.tile_width).to_le_bytes(),
        (i32_to_f32(z.saturating_add(1)) * world.tile_height).to_le_bytes(),
    ];
    for (chunk, field) in out.as_chunks_mut::<4>().0.iter_mut().zip(fields) {
        *chunk = field;
    }
    out
}
