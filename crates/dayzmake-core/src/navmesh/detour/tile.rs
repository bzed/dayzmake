//! One Detour tile blob (spec 9.3) and its extra section (spec 7.2).

use std::io;

use crate::error::ErrorExt;
use crate::navmesh::bytes::Reader;

/// The tile magic `'DNAV'`; the stored bytes are `VAND`.
pub const NAVMESH_MAGIC: i32 = i32::from_le_bytes(*b"VAND");
/// The Detour tile version DayZ uses.
pub const NAVMESH_VERSION: i32 = 12;

const HEADER_LEN: usize = 104;
const VERT_LEN: usize = 12;
const POLY_LEN: usize = 32;
const LINK_LEN: usize = 16;
const DETAIL_MESH_LEN: usize = 12;
const DETAIL_TRI_LEN: usize = 4;
const BV_NODE_LEN: usize = 16;
const OFF_MESH_LEN: usize = 44;
const PART_LEN: usize = 16;
const PART_POLY_LEN: usize = 8;
const OBSTACLE_LEN: usize = 16;
/// The two leading counts of the extra section, plus the obstacle count.
const EXTRA_FIXED_LEN: usize = 12;

/// The 104-byte `dtMeshHeader` of a tile (spec 9.3).
///
/// When a [`TileData`] is serialised, the counts (`poly_count` ... `off_mesh_con_count`) and
/// `extra_data_size` are recomputed from the section lengths; the other fields are written as stored.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct MeshHeader {
    /// Tile magic, [`NAVMESH_MAGIC`].
    pub magic: i32,
    /// Tile version, [`NAVMESH_VERSION`].
    pub version: i32,
    /// Grid x.
    pub x: i32,
    /// Grid row along world Z.
    pub y: i32,
    /// Layer (0).
    pub layer: i32,
    /// User id (0).
    pub user_id: u32,
    /// Polygons, including off-mesh polygons.
    pub poly_count: i32,
    /// Vertices, including two per off-mesh connection.
    pub vert_count: i32,
    /// Allocated links.
    pub max_link_count: i32,
    /// Detail sub-meshes (0 for DayZ).
    pub detail_mesh_count: i32,
    /// Detail vertices (0 for DayZ).
    pub detail_vert_count: i32,
    /// Detail triangles (0 for DayZ).
    pub detail_tri_count: i32,
    /// Bounding volume nodes.
    pub bv_node_count: i32,
    /// Off-mesh connections.
    pub off_mesh_con_count: i32,
    /// Index of the first off-mesh polygon (the ground polygon count).
    pub off_mesh_base: i32,
    /// Agent height, metres.
    pub walkable_height: f32,
    /// Agent radius.
    pub walkable_radius: f32,
    /// Maximum climb height.
    pub walkable_climb: f32,
    /// Minimum corner of the tile's bounds.
    pub bmin: [f32; 3],
    /// Maximum corner of the tile's bounds.
    pub bmax: [f32; 3],
    /// Bounding volume quantisation factor (1 / cell size).
    pub bv_quant_factor: f32,
    /// Size in bytes of the trailing extra section (0: none).
    pub extra_data_size: i32,
}

/// A `dtPoly` (32 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Poly {
    /// Index of the first link, or the null link.
    pub first_link: u32,
    /// Vertex indices.
    pub verts: [u16; 6],
    /// Neighbour encoding: 0 solid, `1 + poly` internal, `0x8000 | dir` portal.
    pub neis: [u16; 6],
    /// Poly flags (spec 7.1).
    pub flags: u16,
    /// Number of vertices used.
    pub vert_count: u8,
    /// Low 6 bits area, high 2 bits type.
    pub area_and_type: u8,
}

impl Poly {
    /// The area id (low 6 bits).
    #[must_use]
    pub fn area(&self) -> u8 {
        self.area_and_type & 0x3F
    }

    /// The polygon type (high 2 bits): 0 ground, 1 off-mesh.
    #[must_use]
    pub fn poly_type(&self) -> u8 {
        self.area_and_type >> 6
    }

    /// Sets the area id (only the low 6 bits of `area` are used).
    pub fn set_area(&mut self, area: u8) {
        self.area_and_type = (self.area_and_type & 0xC0) | (area & 0x3F);
    }

    /// Sets the polygon type (only the low 2 bits of `poly_type` are used).
    pub fn set_type(&mut self, poly_type: u8) {
        self.area_and_type = (self.area_and_type & 0x3F) | ((poly_type & 0x03) << 6);
    }
}

/// A `dtLink` (16 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Link {
    /// Neighbour polygon reference.
    pub reference: u64,
    /// Index of the next link.
    pub next: u32,
    /// Index of the polygon edge.
    pub edge: u8,
    /// Side of the tile boundary (0xFF: internal).
    pub side: u8,
    /// Minimum sub-edge, 0..=255.
    pub bmin: u8,
    /// Maximum sub-edge, 0..=255.
    pub bmax: u8,
}

/// A `dtBVNode` (16 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct BvNode {
    /// Quantised minimum corner.
    pub bmin: [u16; 3],
    /// Quantised maximum corner.
    pub bmax: [u16; 3],
    /// Leaf polygon index (>= 0) or escape offset (< 0).
    pub i: i32,
}

/// A DayZ `dtOffMeshConnection` (44 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct OffMeshConnection {
    /// Start and end point.
    pub pos: [f32; 6],
    /// Measured obstacle height at the midpoint (not read by Detour).
    pub center_height: f32,
    /// Reserved: 0.0 from the builder, 1.0 after `addTile`.
    pub reserved: f32,
    /// Snap radius.
    pub rad: f32,
    /// Index of the off-mesh polygon in this tile.
    pub poly: u16,
    /// Flags (1 = bidirectional).
    pub flags: u8,
    /// 0xFF when both ends are in this tile, else the neighbour direction.
    pub side: u8,
    /// Index of the link class.
    pub user_id: u32,
}

/// A `TilePart` of the extra section: one object part (16 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct TilePart {
    /// The server's object id.
    pub object_id: u64,
    /// The server's part index (2k + state in practice).
    pub part_index: u32,
    /// Index of the part's first [`TilePartPoly`], or -1.
    pub first_poly: i32,
}

/// A `TilePartPoly` of the extra section (8 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct TilePartPoly {
    /// Polygon index.
    pub poly: u32,
    /// Index of the next entry of the same part, or -1.
    pub next: i32,
}

/// A tree obstacle of the extra section (16 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Obstacle {
    /// Position.
    pub pos: [f32; 3],
    /// Radius.
    pub radius: f32,
}

/// The extra section of a tile blob (spec 7.2).
///
/// A tile either has no extra section at all (`TileData::extra` is `None`, `extraDataSize == 0`) or
/// has one, which is at least the 12 bytes of three zero counts: `Some(TileExtra::default())` is
/// written as those 12 bytes.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct TileExtra {
    /// Object parts.
    pub parts: Vec<TilePart>,
    /// Polygons attached to the parts.
    pub part_polys: Vec<TilePartPoly>,
    /// Tree obstacles.
    pub obstacles: Vec<Obstacle>,
}

impl TileExtra {
    /// `true` when there is nothing to store.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty() && self.part_polys.is_empty() && self.obstacles.is_empty()
    }

    /// The size of the section in bytes:
    /// `12 + 16 * parts + 8 * part_polys + 16 * obstacles` (spec 7.2). An empty value is 12 bytes.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        EXTRA_FIXED_LEN
            .saturating_add(self.parts.len().saturating_mul(PART_LEN))
            .saturating_add(self.part_polys.len().saturating_mul(PART_POLY_LEN))
            .saturating_add(self.obstacles.len().saturating_mul(OBSTACLE_LEN))
    }

    fn parse(bytes: &[u8]) -> io::Result<Self> {
        let mut r = Reader::new(bytes);
        let part_count = u32_to_usize(r.u32().map_err(|_| extra_size_error())?);
        let part_poly_count = u32_to_usize(r.u32().map_err(|_| extra_size_error())?);
        // Both counts are checked against the section before anything is allocated.
        let needed = part_count
            .checked_mul(PART_LEN)
            .zip(part_poly_count.checked_mul(PART_POLY_LEN))
            .and_then(|(a, b)| a.checked_add(b))
            .and_then(|n| n.checked_add(4))
            .ok_or_else(extra_size_error)?;
        if needed > r.remaining() {
            return Err(extra_size_error());
        }
        let mut parts = Vec::with_capacity(part_count);
        for _ in 0..part_count {
            parts.push(TilePart {
                object_id: r.u64()?,
                part_index: r.u32()?,
                first_poly: r.i32()?,
            });
        }
        let mut part_polys = Vec::with_capacity(part_poly_count);
        for _ in 0..part_poly_count {
            part_polys.push(TilePartPoly {
                poly: r.u32()?,
                next: r.i32()?,
            });
        }
        let obstacle_count = u32_to_usize(r.u32()?);
        if obstacle_count.checked_mul(OBSTACLE_LEN) != Some(r.remaining()) {
            return Err(extra_size_error());
        }
        let mut obstacles = Vec::with_capacity(obstacle_count);
        for _ in 0..obstacle_count {
            obstacles.push(Obstacle {
                pos: r.f32x3()?,
                radius: r.f32()?,
            });
        }
        Ok(Self {
            parts,
            part_polys,
            obstacles,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        put_u32(out, len_u32(self.parts.len()));
        put_u32(out, len_u32(self.part_polys.len()));
        for p in &self.parts {
            out.extend_from_slice(&p.object_id.to_le_bytes());
            put_u32(out, p.part_index);
            put_i32(out, p.first_poly);
        }
        for p in &self.part_polys {
            put_u32(out, p.poly);
            put_i32(out, p.next);
        }
        put_u32(out, len_u32(self.obstacles.len()));
        for o in &self.obstacles {
            put_f32s(out, &o.pos);
            put_f32(out, o.radius);
        }
    }
}

fn extra_size_error() -> io::Error {
    error!("extra section size does not match its counts")
}

/// The sections of one Detour tile blob, in file order (spec 9.3).
///
/// The detail sections are kept as raw bytes so that foreign files round-trip; DayZ's own tiles have
/// none. [`TileData::to_bytes`] derives the header counts from the lengths of the vectors.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct TileData {
    /// The tile header.
    pub header: MeshHeader,
    /// Vertices.
    pub verts: Vec<[f32; 3]>,
    /// Polygons.
    pub polys: Vec<Poly>,
    /// Links (`maxLinkCount` entries).
    pub links: Vec<Link>,
    /// Raw `dtPolyDetail` records (12 bytes each).
    pub detail_meshes: Vec<[u8; 12]>,
    /// Detail vertices.
    pub detail_verts: Vec<[f32; 3]>,
    /// Raw detail triangles (4 bytes each).
    pub detail_tris: Vec<[u8; 4]>,
    /// Bounding volume tree.
    pub bv_tree: Vec<BvNode>,
    /// Off-mesh connections.
    pub off_mesh: Vec<OffMeshConnection>,
    /// The extra section: `None` when the blob has none (`extraDataSize == 0`), `Some` otherwise,
    /// even when all three counts are zero (12 bytes).
    pub extra: Option<TileExtra>,
}

fn u32_to_usize(n: u32) -> usize {
    // usize is at least 32 bits on every supported target; saturate on the others so that the
    // size checks that follow reject the value.
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// A header count as `usize`; negative counts are rejected.
fn count(what: &str, n: i32) -> io::Result<usize> {
    usize::try_from(n).map_err(|_| error!("negative {what} {n}"))
}

fn len_u32(n: usize) -> u32 {
    // A section longer than u32::MAX entries cannot be held in memory; saturating keeps the
    // function total.
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn len_i32(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f32s(out: &mut Vec<u8>, v: &[f32]) {
    for f in v {
        put_f32(out, *f);
    }
}

fn put_u16s(out: &mut Vec<u8>, v: &[u16]) {
    for n in v {
        out.extend_from_slice(&n.to_le_bytes());
    }
}

impl TileData {
    /// Parses one tile blob. Rejects a bad magic, a tile version other than [`NAVMESH_VERSION`],
    /// negative counts, section sizes that do not match the blob length (truncated, or trailing
    /// bytes) and an extra section that disagrees with `extraDataSize` (spec 9.4). Every size is
    /// computed with checked arithmetic and compared with the blob length before anything is
    /// allocated.
    ///
    /// # Errors
    ///
    /// Returns an error for any of the rejections above.
    pub fn parse(blob: &[u8]) -> io::Result<Self> {
        let mut r = Reader::new(blob);
        let header = read_header(&mut r)?;
        let [
            polys,
            verts,
            links,
            d_meshes,
            d_verts,
            d_tris,
            bv,
            off_mesh,
            extra_size,
        ] = check_sizes(&header, blob.len())?;

        let mut vs = Vec::with_capacity(verts);
        for _ in 0..verts {
            vs.push(r.f32x3()?);
        }
        let mut ps = Vec::with_capacity(polys);
        for _ in 0..polys {
            ps.push(read_poly(&mut r)?);
        }
        let mut ls = Vec::with_capacity(links);
        for _ in 0..links {
            ls.push(Link {
                reference: r.u64()?,
                next: r.u32()?,
                edge: r.u8()?,
                side: r.u8()?,
                bmin: r.u8()?,
                bmax: r.u8()?,
            });
        }
        let mut detail_meshes = Vec::with_capacity(d_meshes);
        for _ in 0..d_meshes {
            detail_meshes.push(r.array::<12>()?);
        }
        let mut detail_verts = Vec::with_capacity(d_verts);
        for _ in 0..d_verts {
            detail_verts.push(r.f32x3()?);
        }
        let mut detail_tris = Vec::with_capacity(d_tris);
        for _ in 0..d_tris {
            detail_tris.push(r.array::<4>()?);
        }
        let mut bv_tree = Vec::with_capacity(bv);
        for _ in 0..bv {
            bv_tree.push(BvNode {
                bmin: [r.u16()?, r.u16()?, r.u16()?],
                bmax: [r.u16()?, r.u16()?, r.u16()?],
                i: r.i32()?,
            });
        }
        let mut off = Vec::with_capacity(off_mesh);
        for _ in 0..off_mesh {
            off.push(OffMeshConnection {
                pos: [r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?],
                center_height: r.f32()?,
                reserved: r.f32()?,
                rad: r.f32()?,
                poly: r.u16()?,
                flags: r.u8()?,
                side: r.u8()?,
                user_id: r.u32()?,
            });
        }
        let extra = if extra_size == 0 {
            None
        } else {
            Some(TileExtra::parse(r.rest()).prepend_error("Invalid extra section:")?)
        };
        Ok(Self {
            header,
            verts: vs,
            polys: ps,
            links: ls,
            detail_meshes,
            detail_verts,
            detail_tris,
            bv_tree,
            off_mesh: off,
            extra,
        })
    }

    /// The size of the serialised blob in bytes, or `None` if it would overflow `usize`.
    pub(crate) fn blob_len(&self) -> Option<usize> {
        let extra = self.extra.as_ref().map_or(0, TileExtra::byte_size);
        [
            self.verts.len().checked_mul(VERT_LEN)?,
            self.polys.len().checked_mul(POLY_LEN)?,
            self.links.len().checked_mul(LINK_LEN)?,
            self.detail_meshes.len().checked_mul(DETAIL_MESH_LEN)?,
            self.detail_verts.len().checked_mul(VERT_LEN)?,
            self.detail_tris.len().checked_mul(DETAIL_TRI_LEN)?,
            self.bv_tree.len().checked_mul(BV_NODE_LEN)?,
            self.off_mesh.len().checked_mul(OFF_MESH_LEN)?,
            extra,
        ]
        .iter()
        .try_fold(HEADER_LEN, |acc, &x| acc.checked_add(x))
    }

    /// Serialises the tile. The header counts and `extraDataSize` are derived from the sections
    /// (`extra` is `None`: no section, `extraDataSize` 0; `Some`: the section, even when empty); all other header fields
    /// are written as stored.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.blob_len().unwrap_or(HEADER_LEN));
        let h = &self.header;
        let extra_size = len_i32(self.extra.as_ref().map_or(0, TileExtra::byte_size));
        for v in [h.magic, h.version, h.x, h.y, h.layer] {
            put_i32(&mut out, v);
        }
        put_u32(&mut out, h.user_id);
        for n in [
            self.polys.len(),
            self.verts.len(),
            self.links.len(),
            self.detail_meshes.len(),
            self.detail_verts.len(),
            self.detail_tris.len(),
            self.bv_tree.len(),
            self.off_mesh.len(),
        ] {
            put_i32(&mut out, len_i32(n));
        }
        put_i32(&mut out, h.off_mesh_base);
        put_f32s(
            &mut out,
            &[h.walkable_height, h.walkable_radius, h.walkable_climb],
        );
        put_f32s(&mut out, &h.bmin);
        put_f32s(&mut out, &h.bmax);
        put_f32(&mut out, h.bv_quant_factor);
        put_i32(&mut out, extra_size);

        for v in &self.verts {
            put_f32s(&mut out, v);
        }
        for p in &self.polys {
            put_u32(&mut out, p.first_link);
            put_u16s(&mut out, &p.verts);
            put_u16s(&mut out, &p.neis);
            put_u16s(&mut out, &[p.flags]);
            out.push(p.vert_count);
            out.push(p.area_and_type);
        }
        for l in &self.links {
            out.extend_from_slice(&l.reference.to_le_bytes());
            put_u32(&mut out, l.next);
            out.extend_from_slice(&[l.edge, l.side, l.bmin, l.bmax]);
        }
        for d in &self.detail_meshes {
            out.extend_from_slice(d);
        }
        for v in &self.detail_verts {
            put_f32s(&mut out, v);
        }
        for t in &self.detail_tris {
            out.extend_from_slice(t);
        }
        for n in &self.bv_tree {
            put_u16s(&mut out, &n.bmin);
            put_u16s(&mut out, &n.bmax);
            put_i32(&mut out, n.i);
        }
        for c in &self.off_mesh {
            put_f32s(&mut out, &c.pos);
            put_f32s(&mut out, &[c.center_height, c.reserved, c.rad]);
            put_u16s(&mut out, &[c.poly]);
            out.extend_from_slice(&[c.flags, c.side]);
            put_u32(&mut out, c.user_id);
        }
        if let Some(extra) = &self.extra {
            extra.write(&mut out);
        }
        out
    }
}

fn check_sizes(header: &MeshHeader, blob_len: usize) -> io::Result<[usize; 9]> {
    let counts = [
        ("polyCount", header.poly_count),
        ("vertCount", header.vert_count),
        ("maxLinkCount", header.max_link_count),
        ("detailMeshCount", header.detail_mesh_count),
        ("detailVertCount", header.detail_vert_count),
        ("detailTriCount", header.detail_tri_count),
        ("bvNodeCount", header.bv_node_count),
        ("offMeshConCount", header.off_mesh_con_count),
        ("extraDataSize", header.extra_data_size),
    ];
    let mut n = [0usize; 9];
    for (slot, (what, value)) in n.iter_mut().zip(counts) {
        *slot = count(what, value)?;
    }
    let [
        polys,
        verts,
        links,
        d_meshes,
        d_verts,
        d_tris,
        bv,
        off_mesh,
        extra_size,
    ] = n;
    let sizes = [
        (verts, VERT_LEN),
        (polys, POLY_LEN),
        (links, LINK_LEN),
        (d_meshes, DETAIL_MESH_LEN),
        (d_verts, VERT_LEN),
        (d_tris, DETAIL_TRI_LEN),
        (bv, BV_NODE_LEN),
        (off_mesh, OFF_MESH_LEN),
    ];
    let total = sizes
        .iter()
        .try_fold(HEADER_LEN, |acc, &(n, len)| {
            acc.checked_add(n.checked_mul(len)?)
        })
        .and_then(|t| t.checked_add(extra_size))
        .ok_or_else(|| error!("tile blob truncated"))?;
    if total > blob_len {
        return Err(error!("tile blob truncated"));
    }
    if total < blob_len {
        return Err(error!("trailing bytes after the tile blob"));
    }
    Ok(n)
}

fn read_header(r: &mut Reader<'_>) -> io::Result<MeshHeader> {
    let magic = r.i32().map_err(|_| error!("tile blob truncated"))?;
    if magic != NAVMESH_MAGIC {
        return Err(error!("Invalid tile header (bad tile magic)"));
    }
    let version = r.i32().map_err(|_| error!("tile blob truncated"))?;
    if version != NAVMESH_VERSION {
        return Err(error!("Invalid tile version {version}"));
    }
    let truncated = |_| error!("tile blob truncated");
    Ok(MeshHeader {
        magic,
        version,
        x: r.i32().map_err(truncated)?,
        y: r.i32().map_err(truncated)?,
        layer: r.i32().map_err(truncated)?,
        user_id: r.u32().map_err(truncated)?,
        poly_count: r.i32().map_err(truncated)?,
        vert_count: r.i32().map_err(truncated)?,
        max_link_count: r.i32().map_err(truncated)?,
        detail_mesh_count: r.i32().map_err(truncated)?,
        detail_vert_count: r.i32().map_err(truncated)?,
        detail_tri_count: r.i32().map_err(truncated)?,
        bv_node_count: r.i32().map_err(truncated)?,
        off_mesh_con_count: r.i32().map_err(truncated)?,
        off_mesh_base: r.i32().map_err(truncated)?,
        walkable_height: r.f32().map_err(truncated)?,
        walkable_radius: r.f32().map_err(truncated)?,
        walkable_climb: r.f32().map_err(truncated)?,
        bmin: r.f32x3().map_err(truncated)?,
        bmax: r.f32x3().map_err(truncated)?,
        bv_quant_factor: r.f32().map_err(truncated)?,
        extra_data_size: r.i32().map_err(truncated)?,
    })
}

fn read_poly(r: &mut Reader<'_>) -> io::Result<Poly> {
    let first_link = r.u32()?;
    let mut verts = [0u16; 6];
    for v in &mut verts {
        *v = r.u16()?;
    }
    let mut neis = [0u16; 6];
    for v in &mut neis {
        *v = r.u16()?;
    }
    Ok(Poly {
        first_link,
        verts,
        neis,
        flags: r.u16()?,
        vert_count: r.u8()?,
        area_and_type: r.u8()?,
    })
}
