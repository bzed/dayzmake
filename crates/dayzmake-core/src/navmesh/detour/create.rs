//
// Copyright (c) 2009-2010 Mikko Mononen memon@inside.org
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Ported to Rust and altered for DayZ (see the dayzmake navmesh spec); this is not the original software.
//

//! `dtCreateNavMeshData`, the DayZ variant (spec 7.3, 9.3 and 10.2): one Detour tile blob from a
//! Recast poly mesh.
//!
//! Stock Detour, except for: tile version 12, the extra header field, 44-byte off-mesh connections,
//! no detail mesh, a trailing extra section, and the off-mesh link accounting of spec 7.3.

use std::io;

use crate::navmesh::detour::tile::{
    BvNode, Link, MeshHeader, NAVMESH_MAGIC, NAVMESH_VERSION, OffMeshConnection, Poly, TileData,
    TileExtra,
};
use crate::navmesh::num::{ceil_i32, floor_i32};

/// The polygon vertex list terminator of a Recast poly mesh.
const MESH_NULL_IDX: u16 = 0xffff;
/// `DT_EXT_LINK`: the neighbour is in another tile.
const EXT_LINK: u16 = 0x8000;
/// The largest number of vertices of one polygon.
const VERTS_PER_POLYGON: usize = 6;

/// One off-mesh connection of the input (a jump, climb or ladder link).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct OffMeshInput {
    /// Start point, in world coordinates.
    pub start: [f32; 3],
    /// End point, in world coordinates.
    pub end: [f32; 3],
    /// The measured obstacle height at the midpoint, relative to the start.
    pub center_height: f32,
    /// Snap radius used when the tile is linked.
    pub radius: f32,
    /// Polygon flags of the connection.
    pub flags: u16,
    /// Area id of the connection.
    pub area: u8,
    /// `true` when the connection can be used in both directions.
    pub bidirectional: bool,
    /// User id (the index of the link class).
    pub user_id: u32,
}

/// The inputs of [`create_tile_data`], mirroring `dtNavMeshCreateParams`.
///
/// Build it with [`CreateParams::new`] and set the scalars by field assignment.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CreateParams<'a> {
    /// Poly mesh vertices in cell coordinates (relative to `bmin`).
    pub verts: &'a [[u16; 3]],
    /// Polygons: `nvp * 2` values each, the vertex indices (terminated by `0xffff`) followed by
    /// the neighbour codes.
    pub polys: &'a [u16],
    /// Polygon flags, one per polygon.
    pub flags: &'a [u16],
    /// Polygon area ids, one per polygon.
    pub areas: &'a [u8],
    /// Vertices per polygon of the poly mesh (at most 6).
    pub nvp: usize,
    /// The off-mesh connections that touch the tile.
    pub off_mesh: &'a [OffMeshInput],
    /// The extra section stored after the off-mesh connections.
    pub extra: &'a TileExtra,
    /// Tile grid x.
    pub tile_x: i32,
    /// Tile grid y (the z grid index).
    pub tile_y: i32,
    /// Tile bounds minimum.
    pub bmin: [f32; 3],
    /// Tile bounds maximum.
    pub bmax: [f32; 3],
    /// Agent height in metres.
    pub walkable_height: f32,
    /// Agent radius in metres.
    pub walkable_radius: f32,
    /// Agent climb height in metres.
    pub walkable_climb: f32,
    /// Cell size.
    pub cs: f32,
    /// Cell height.
    pub ch: f32,
}

impl<'a> CreateParams<'a> {
    /// Parameters for the given poly mesh. The scalars start as zero, except for `cs = 0.25` and
    /// `ch = 0.1`.
    #[must_use]
    pub fn new(
        verts: &'a [[u16; 3]],
        polys: &'a [u16],
        flags: &'a [u16],
        areas: &'a [u8],
        nvp: usize,
        off_mesh: &'a [OffMeshInput],
        extra: &'a TileExtra,
    ) -> Self {
        Self {
            verts,
            polys,
            flags,
            areas,
            nvp,
            off_mesh,
            extra,
            tile_x: 0,
            tile_y: 0,
            bmin: [0.0; 3],
            bmax: [0.0; 3],
            walkable_height: 0.0,
            walkable_radius: 0.0,
            walkable_climb: 0.0,
            cs: 0.25,
            ch: 0.1,
        }
    }
}

/// `classifyOffMeshPoint`: the side of the tile `bmin`-`bmax` (xz) the point is on, as a
/// `dtNavMesh` side 0 to 7, or 0xff when it is inside.
pub(crate) fn classify_off_mesh_point(pt: &[f32; 3], bmin: &[f32; 3], bmax: &[f32; 3]) -> u8 {
    const XP: u8 = 1;
    const ZP: u8 = 2;
    const XM: u8 = 4;
    const ZM: u8 = 8;
    let mut outcode = 0_u8;
    if pt[0] >= bmax[0] {
        outcode |= XP;
    }
    if pt[2] >= bmax[2] {
        outcode |= ZP;
    }
    if pt[0] < bmin[0] {
        outcode |= XM;
    }
    if pt[2] < bmin[2] {
        outcode |= ZM;
    }
    match outcode {
        XP => 0,
        v if v == XP | ZP => 1,
        ZP => 2,
        v if v == XM | ZP => 3,
        XM => 4,
        v if v == XM | ZM => 5,
        ZM => 6,
        v if v == XP | ZM => 7,
        _ => 0xff,
    }
}

/// An item of the bounding volume tree build (`BVItem`).
#[derive(Clone, Copy)]
struct BvItem {
    bmin: [u16; 3],
    bmax: [u16; 3],
    i: i32,
}

/// Saturating conversion of a computed, integral `f32` to `u16` (C's cast on in-range values).
fn clamp_u16(v: i32) -> u16 {
    u16::try_from(v.clamp(0, i32::from(u16::MAX))).unwrap_or(u16::MAX)
}

fn longest_axis(x: u16, y: u16, z: u16) -> usize {
    let mut axis = 0;
    let mut max_val = x;
    if y > max_val {
        axis = 1;
        max_val = y;
    }
    if z > max_val {
        axis = 2;
    }
    axis
}

/// Grows `bmin`/`bmax` to contain `vmin`/`vmax`.
fn grow(bmin: &mut [u16; 3], bmax: &mut [u16; 3], vmin: [u16; 3], vmax: [u16; 3]) {
    for (lo, v) in bmin.iter_mut().zip(vmin) {
        if v < *lo {
            *lo = v;
        }
    }
    for (hi, v) in bmax.iter_mut().zip(vmax) {
        if v > *hi {
            *hi = v;
        }
    }
}

fn calc_extends(items: &[BvItem]) -> ([u16; 3], [u16; 3]) {
    let Some(first) = items.first() else {
        return ([0; 3], [0; 3]);
    };
    let mut bmin = first.bmin;
    let mut bmax = first.bmax;
    for it in items.iter().skip(1) {
        grow(&mut bmin, &mut bmax, it.bmin, it.bmax);
    }
    (bmin, bmax)
}

/// `subdivide`. The sort is stable and ties go by polygon index (plan ruling 7); upstream's
/// `qsort` has no tie-break.
fn subdivide(items: &mut [BvItem], nodes: &mut Vec<BvNode>) {
    let icur = nodes.len();
    nodes.push(BvNode::default());
    let inum = items.len();
    if let [only] = items {
        if let Some(node) = nodes.get_mut(icur) {
            node.bmin = only.bmin;
            node.bmax = only.bmax;
            node.i = only.i;
        }
        return;
    }
    let (bmin, bmax) = calc_extends(items);
    let axis = longest_axis(
        bmax[0].saturating_sub(bmin[0]),
        bmax[1].saturating_sub(bmin[1]),
        bmax[2].saturating_sub(bmin[2]),
    );
    items.sort_by(|a, b| {
        a.bmin
            .get(axis)
            .cmp(&b.bmin.get(axis))
            .then_with(|| a.i.cmp(&b.i))
    });
    let isplit = inum / 2;
    let (left, right) = items.split_at_mut(isplit);
    subdivide(left, nodes);
    subdivide(right, nodes);
    let escape = nodes.len().saturating_sub(icur);
    let escape = i32::try_from(escape).unwrap_or(i32::MAX);
    if let Some(node) = nodes.get_mut(icur) {
        node.bmin = bmin;
        node.bmax = bmax;
        node.i = -escape;
    }
}

/// `createBVTree` for a poly mesh without a detail mesh.
fn create_bv_tree(
    params: &CreateParams<'_>,
    nvp: usize,
    poly_count: usize,
) -> io::Result<Vec<BvNode>> {
    let mut items = Vec::with_capacity(poly_count);
    for (i, p) in params.polys.chunks_exact(nvp * 2).enumerate() {
        let (idx, _) = p.split_at(nvp);
        let vertex = |k: u16| -> io::Result<[u16; 3]> {
            params
                .verts
                .get(usize::from(k))
                .copied()
                .ok_or_else(|| error!("polygon {i} uses vertex {k} outside the vertex list"))
        };
        let first = vertex(idx.first().copied().unwrap_or(MESH_NULL_IDX))?;
        let mut bmin = first;
        let mut bmax = first;
        for &k in idx.iter().skip(1) {
            if k == MESH_NULL_IDX {
                break;
            }
            let v = vertex(k)?;
            grow(&mut bmin, &mut bmax, v, v);
        }
        // Remap y from ch to cs units.
        bmin[1] = clamp_u16(floor_i32(f32::from(bmin[1]) * params.ch / params.cs));
        bmax[1] = clamp_u16(ceil_i32(f32::from(bmax[1]) * params.ch / params.cs));
        items.push(BvItem {
            bmin,
            bmax,
            i: i32::try_from(i).map_err(|_| error!("too many polygons"))?,
        });
    }
    let mut nodes = Vec::with_capacity(poly_count.saturating_mul(2));
    subdivide(&mut items, &mut nodes);
    Ok(nodes)
}

fn count_i32(n: usize, what: &str) -> io::Result<i32> {
    i32::try_from(n).map_err(|_| error!("too many {what}"))
}

/// The `(start, end)` classes of the off-mesh connections, the number that is stored and the
/// number of endpoints inside the tile.
struct OffMeshClasses {
    classes: Vec<(u8, u8)>,
    stored: usize,
    link_count: usize,
}

/// Classifies the off-mesh connection points. Only connections whose start is inside the tile
/// (and inside its height band) are stored; the link count includes every inside endpoint.
fn classify_off_mesh(params: &CreateParams<'_>) -> OffMeshClasses {
    let mut out = OffMeshClasses {
        classes: Vec::with_capacity(params.off_mesh.len()),
        stored: 0,
        link_count: 0,
    };
    if params.off_mesh.is_empty() {
        return out;
    }
    // Tight height bounds, used for culling out off-mesh start locations.
    let mut hmin = f32::MAX;
    let mut hmax = -f32::MAX;
    for iv in params.verts {
        let h = params.bmin[1] + f32::from(iv[1]) * params.ch;
        hmin = if hmin < h { hmin } else { h };
        hmax = if hmax > h { hmax } else { h };
    }
    hmin -= params.walkable_climb;
    hmax += params.walkable_climb;
    let mut bmin = params.bmin;
    let mut bmax = params.bmax;
    bmin[1] = hmin;
    bmax[1] = hmax;
    for c in params.off_mesh {
        let mut start = classify_off_mesh_point(&c.start, &bmin, &bmax);
        let end = classify_off_mesh_point(&c.end, &bmin, &bmax);
        // Zero out off-mesh start positions which are not even potentially touching the mesh.
        if start == 0xff && (c.start[1] < bmin[1] || c.start[1] > bmax[1]) {
            start = 0;
        }
        // Count how many links should be allocated for off-mesh connections.
        if start == 0xff {
            out.link_count += 1;
            out.stored += 1;
        }
        if end == 0xff {
            out.link_count += 1;
        }
        out.classes.push((start, end));
    }
    out
}

/// Checks the slice lengths and limits; returns the polygon count.
fn validate(params: &CreateParams<'_>) -> io::Result<usize> {
    let nvp = params.nvp;
    if nvp > VERTS_PER_POLYGON {
        return Err(error!(
            "{nvp} vertices per polygon (at most {VERTS_PER_POLYGON})"
        ));
    }
    if nvp == 0 {
        return Err(error!("no vertices per polygon"));
    }
    if params.verts.len() >= usize::from(MESH_NULL_IDX) {
        return Err(error!("too many vertices ({})", params.verts.len()));
    }
    if params.verts.is_empty() {
        return Err(error!("no vertices"));
    }
    if params.polys.is_empty() {
        return Err(error!("no polygons"));
    }
    let stride = nvp * 2;
    if !params.polys.len().is_multiple_of(stride) {
        return Err(error!(
            "polygon array of {} values is not a multiple of {stride}",
            params.polys.len()
        ));
    }
    let poly_count = params.polys.len() / stride;
    if params.flags.len() != poly_count || params.areas.len() != poly_count {
        return Err(error!(
            "{poly_count} polygons but {} flags and {} areas",
            params.flags.len(),
            params.areas.len()
        ));
    }
    Ok(poly_count)
}

/// The number of polygon edges and of portal edges (`dir != 0xf`).
fn count_edges(params: &CreateParams<'_>) -> (usize, usize) {
    let nvp = params.nvp;
    let mut edge_count = 0_usize;
    let mut portal_count = 0_usize;
    for p in params.polys.chunks_exact(nvp * 2) {
        let (idx, nei) = p.split_at(nvp);
        for (&k, &n) in idx.iter().zip(nei) {
            if k == MESH_NULL_IDX {
                break;
            }
            edge_count += 1;
            if n & EXT_LINK != 0 && n & 0xf != 0xf {
                portal_count += 1;
            }
        }
    }
    (edge_count, portal_count)
}

/// The Detour neighbour code of a Recast one (spec 9.3): portals x-, z+, x+, z- become 4, 2, 0, 6.
fn neighbour_code(n: u16) -> u16 {
    if n & EXT_LINK == 0 {
        // Normal connection (`n` is at most 0x7fff here).
        return n + 1;
    }
    match n & 0xf {
        0 => EXT_LINK | 4, // portal x-
        1 => EXT_LINK | 2, // portal z+
        2 => EXT_LINK,     // portal x+
        3 => EXT_LINK | 6, // portal z-
        _ => 0,            // border (0xf), and the codes stock Detour leaves unset
    }
}

/// The ground polygons.
fn ground_polys(params: &CreateParams<'_>) -> io::Result<Vec<Poly>> {
    let nvp = params.nvp;
    let mut polys = Vec::new();
    for (i, p) in params.polys.chunks_exact(nvp * 2).enumerate() {
        let (idx, nei) = p.split_at(nvp);
        let mut poly = Poly {
            flags: params.flags.get(i).copied().unwrap_or(0),
            ..Poly::default()
        };
        poly.set_area(params.areas.get(i).copied().unwrap_or(0));
        poly.set_type(0);
        for (j, (&k, &n)) in idx.iter().zip(nei).enumerate() {
            if k == MESH_NULL_IDX {
                break;
            }
            if usize::from(k) >= params.verts.len() {
                return Err(error!(
                    "polygon {i} uses vertex {k} outside the vertex list"
                ));
            }
            if let Some(slot) = poly.verts.get_mut(j) {
                *slot = k;
            }
            if let Some(slot) = poly.neis.get_mut(j) {
                *slot = neighbour_code(n);
            }
            poly.vert_count += 1;
        }
        if poly.vert_count == 0 {
            return Err(error!("polygon {i} has no vertices"));
        }
        polys.push(poly);
    }
    Ok(polys)
}

/// The stored off-mesh polygons and connections; their vertices are appended to `verts`.
fn off_mesh_parts(
    params: &CreateParams<'_>,
    classes: &[(u8, u8)],
    verts: &mut Vec<[f32; 3]>,
    polys: &mut Vec<Poly>,
) -> io::Result<Vec<OffMeshConnection>> {
    let poly_base = polys.len();
    let mut cons = Vec::new();
    for (c, &(start, end)) in params.off_mesh.iter().zip(classes) {
        if start != 0xff {
            continue;
        }
        let n = cons.len();
        let first = u16::try_from(verts.len())
            .map_err(|_| error!("too many vertices with the off-mesh connections"))?;
        let second = first
            .checked_add(1)
            .ok_or_else(|| error!("too many vertices with the off-mesh connections"))?;
        let poly = u16::try_from(poly_base + n)
            .map_err(|_| error!("too many polygons with the off-mesh connections"))?;
        verts.push(c.start);
        verts.push(c.end);
        let mut off = Poly {
            vert_count: 2,
            flags: c.flags,
            ..Poly::default()
        };
        off.verts = [first, second, 0, 0, 0, 0];
        off.set_area(c.area);
        off.set_type(1);
        polys.push(off);
        cons.push(OffMeshConnection {
            pos: [
                c.start[0], c.start[1], c.start[2], c.end[0], c.end[1], c.end[2],
            ],
            center_height: c.center_height,
            reserved: 0.0,
            rad: c.radius,
            poly,
            flags: u8::from(c.bidirectional),
            side: end,
            user_id: c.user_id,
        });
    }
    Ok(cons)
}

/// Builds one Detour tile (version 12) from a poly mesh, the DayZ way (spec 7.3).
///
/// No detail mesh is produced. Off-mesh connections are stored only when their start point is
/// inside the tile; `max_link_count` also reserves slots for every endpoint inside the tile of
/// every passed connection. The BV tree is built over the ground polygons. The links are zeroed
/// (linking happens when the tile is added to a mesh) and the extra section is a copy of
/// `params.extra`.
///
/// # Errors
///
/// Returns an error for an empty vertex or polygon list, `nvp` above 6 or zero, 65535 or more
/// vertices, slices of inconsistent lengths, vertex indices outside the vertex list, a polygon
/// without vertices, or more off-mesh polygons or vertices than 16 bits can index.
pub fn create_tile_data(params: &CreateParams<'_>) -> io::Result<TileData> {
    let poly_count = validate(params)?;
    let off = classify_off_mesh(params);
    let (edge_count, portal_count) = count_edges(params);
    let max_link_count = edge_count + portal_count * 2 + off.link_count * 2;

    // Vertices of the mesh, then the off-mesh ones.
    let mut verts: Vec<[f32; 3]> = params
        .verts
        .iter()
        .map(|iv| {
            [
                params.bmin[0] + f32::from(iv[0]) * params.cs,
                params.bmin[1] + f32::from(iv[1]) * params.ch,
                params.bmin[2] + f32::from(iv[2]) * params.cs,
            ]
        })
        .collect();
    let mut polys = ground_polys(params)?;
    let off_mesh = off_mesh_parts(params, &off.classes, &mut verts, &mut polys)?;

    // BV tree over the ground polygons, in a 2n-node array.
    let mut bv_tree = create_bv_tree(params, params.nvp, poly_count)?;
    bv_tree.resize(poly_count * 2, BvNode::default());

    let header = MeshHeader {
        magic: NAVMESH_MAGIC,
        version: NAVMESH_VERSION,
        x: params.tile_x,
        y: params.tile_y,
        layer: 0,
        user_id: 0,
        poly_count: count_i32(polys.len(), "polygons")?,
        vert_count: count_i32(verts.len(), "vertices")?,
        max_link_count: count_i32(max_link_count, "links")?,
        detail_mesh_count: 0,
        detail_vert_count: 0,
        detail_tri_count: 0,
        bv_node_count: count_i32(poly_count * 2, "polygons")?,
        off_mesh_con_count: count_i32(off_mesh.len(), "off-mesh connections")?,
        off_mesh_base: count_i32(poly_count, "polygons")?,
        walkable_height: params.walkable_height,
        walkable_radius: params.walkable_radius,
        walkable_climb: params.walkable_climb,
        bmin: params.bmin,
        bmax: params.bmax,
        bv_quant_factor: 1.0 / params.cs,
        extra_data_size: count_i32(params.extra.byte_size(), "extra section bytes")?,
    };
    Ok(TileData {
        header,
        verts,
        polys,
        links: vec![Link::default(); max_link_count],
        detail_meshes: Vec::new(),
        detail_verts: Vec::new(),
        detail_tris: Vec::new(),
        bv_tree,
        off_mesh,
        extra: Some(params.extra.clone()),
    })
}
