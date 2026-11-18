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

//! The tiled Detour navmesh: `dtNavMesh::init`, `addTile` and `removeTile` with the tile linking
//! (spec 8.1 to 8.3 and 10.2).
//!
//! Safe Rust: every tile owns its [`TileData`], whose `links` are the link pool of the tile (with a
//! free list through the `next` fields, as upstream). Linking two tiles first reads both to
//! compute the links and then applies them one tile at a time.
//!
//! DayZ differences from stock Detour (spec 10.2) are marked `DAYZ:`.

use std::collections::BTreeMap;
use std::io;

use crate::error::ErrorExt;
use crate::navmesh::detour::common::{dt_clamp, dt_max, dt_min, opposite_tile};
use crate::navmesh::detour::query::find_nearest_poly_in_tile;
use crate::navmesh::detour::set::{
    NavMeshParams, NavMeshSet, SET_VERSION, SetTile, decode_poly_ref, encode_poly_ref,
};
use crate::navmesh::detour::tile::{NAVMESH_MAGIC, NAVMESH_VERSION, Poly, TileData};
use crate::navmesh::num::trunc_u8;

/// `DT_NULL_LINK`.
const NULL_LINK: u32 = 0xffff_ffff;
/// `DT_EXT_LINK`.
const EXT_LINK: u16 = 0x8000;
/// `DT_POLYTYPE_OFFMESH_CONNECTION`.
const POLYTYPE_OFFMESH: u8 = 1;
/// `DT_OFFMESH_CON_BIDIR`.
const OFFMESH_CON_BIDIR: u8 = 1;
/// The most polygons the 20-bit polygon field of a reference can address.
const MAX_POLYS_PER_TILE: usize = 1 << 20;
/// The most tile slots the 28-bit tile field of a reference can address.
const MAX_TILE_SLOTS: u32 = 1 << 28;
/// The salt is 16 bits wide.
const SALT_MASK: u32 = 0xffff;
/// The most polygons `findConnectingPolys` returns.
const MAX_CONNECTING: usize = 4;
/// The vertical half extent of the obstacle snap (spec 8.3).
const OBSTACLE_SNAP_HALF_HEIGHT: f32 = 30.0;

/// A loaded tile: its data and the head of its free link list.
#[derive(Debug, Clone)]
struct MeshTile {
    data: TileData,
    links_free_list: u32,
    /// The padding word of the set record, kept on rewrite.
    padding: u32,
}

/// One tile slot. A slot that was never touched has salt 1 and no tile.
#[derive(Debug, Clone)]
struct Slot {
    salt: u32,
    tile: Option<MeshTile>,
}

/// A tiled Detour navigation mesh (`dtNavMesh`) with 64-bit polygon references: salt 16 bits, tile
/// 28 bits, polygon 20 bits (spec 8.1).
///
/// Tiles live in slots. All slots start with salt 1 and slot 0 is allocated first; removing a tile
/// bumps the salt of its slot and puts the slot at the head of the free list. There is one layer
/// (0) per grid cell.
#[derive(Debug, Clone)]
pub struct NavMesh {
    params: NavMeshParams,
    max_tiles: u32,
    /// The slots that were ever used, in slot order.
    slots: BTreeMap<u32, Slot>,
    /// Slots that were never used are allocated in ascending order from here.
    next_fresh: u32,
    /// Freed slots; the head of the free list is the last element.
    free: Vec<u32>,
    /// Tile grid `(x, y)` to slot.
    grid: BTreeMap<(i32, i32), u32>,
}

fn salt_after_removal(salt: u32) -> u32 {
    let s = salt.wrapping_add(1) & SALT_MASK;
    if s == 0 { 1 } else { s }
}

fn alloc_link(t: &mut MeshTile) -> u32 {
    if t.links_free_list == NULL_LINK {
        return NULL_LINK;
    }
    let link = t.links_free_list;
    let next = usize::try_from(link)
        .ok()
        .and_then(|i| t.data.links.get(i))
        .map_or(NULL_LINK, |l| l.next);
    t.links_free_list = next;
    link
}

fn free_link(t: &mut MeshTile, link: u32) {
    let head = t.links_free_list;
    if let Some(l) = usize::try_from(link)
        .ok()
        .and_then(|i| t.data.links.get_mut(i))
    {
        l.next = head;
        t.links_free_list = link;
    }
}

/// Allocates a link and puts it at the head of the list of polygon `poly`. `bounds` is the portal
/// range; `None` leaves the link's old `bmin`/`bmax` alone (upstream does not write them for the
/// sides it does not know). Returns whether a link was available.
fn push_link(
    t: &mut MeshTile,
    poly: usize,
    reference: u64,
    edge: u8,
    side: u8,
    bounds: Option<(u8, u8)>,
) -> bool {
    let idx = alloc_link(t);
    if idx == NULL_LINK {
        return false;
    }
    let Some(first) = t.data.polys.get(poly).map(|p| p.first_link) else {
        free_link(t, idx);
        return false;
    };
    let linked = usize::try_from(idx)
        .ok()
        .and_then(|i| t.data.links.get_mut(i));
    if let Some(l) = linked {
        l.reference = reference;
        l.edge = edge;
        l.side = side;
        if let Some((lo, hi)) = bounds {
            l.bmin = lo;
            l.bmax = hi;
        }
        l.next = first;
    }
    if let Some(p) = t.data.polys.get_mut(poly) {
        p.first_link = idx;
    }
    true
}

fn vert(t: &TileData, index: u16) -> Option<[f32; 3]> {
    t.verts.get(usize::from(index)).copied()
}

fn sqr(x: f32) -> f32 {
    x * x
}

/// `getSlabCoord`.
fn get_slab_coord(va: &[f32; 3], side: i32) -> f32 {
    if side == 0 || side == 4 {
        va[0]
    } else if side == 2 || side == 6 {
        va[2]
    } else {
        0.0
    }
}

/// `calcSlabEndPoints`: the `(min, max)` ends of the edge as (position along the slab, height).
fn calc_slab_end_points(va: &[f32; 3], vb: &[f32; 3], side: i32) -> ([f32; 2], [f32; 2]) {
    let axis = if side == 0 || side == 4 {
        2
    } else if side == 2 || side == 6 {
        0
    } else {
        return ([0.0; 2], [0.0; 2]);
    };
    let (Some(&a), Some(&b)) = (va.get(axis), vb.get(axis)) else {
        return ([0.0; 2], [0.0; 2]);
    };
    if a < b {
        ([a, va[1]], [b, vb[1]])
    } else {
        ([b, vb[1]], [a, va[1]])
    }
}

/// `overlapSlabs`.
fn overlap_slabs(
    amin: [f32; 2],
    amax: [f32; 2],
    bmin: [f32; 2],
    bmax: [f32; 2],
    px: f32,
    py: f32,
) -> bool {
    // The segments are shrunk a little so that slabs which touch at end points are not connected.
    let lo_x = dt_max(amin[0] + px, bmin[0] + px);
    let hi_x = dt_min(amax[0] - px, bmax[0] - px);
    if lo_x > hi_x {
        return false;
    }
    let ad = (amax[1] - amin[1]) / (amax[0] - amin[0]);
    let ak = amin[1] - ad * amin[0];
    let bd = (bmax[1] - bmin[1]) / (bmax[0] - bmin[0]);
    let bk = bmin[1] - bd * bmin[0];
    let aminy = ad * lo_x + ak;
    let amaxy = ad * hi_x + ak;
    let bminy = bd * lo_x + bk;
    let bmaxy = bd * hi_x + bk;
    let dmin = bminy - aminy;
    let dmax = bmaxy - amaxy;
    // Crossing segments always overlap.
    if dmin * dmax < 0.0 {
        return true;
    }
    // Check for overlap at the end points.
    let thr = sqr(py * 2.0);
    dmin * dmin <= thr || dmax * dmax <= thr
}

/// A polygon of the target tile that an edge connects to, and the overlap along the slab.
struct Connection {
    reference: u64,
    lo: f32,
    hi: f32,
}

/// `findConnectingPolys`: the polygons of `tile` (whose polygon references start at `base`) with
/// an edge on `side` that overlaps the edge `va`-`vb`, at most `max` of them.
fn find_connecting_polys(
    va: &[f32; 3],
    vb: &[f32; 3],
    tile: &TileData,
    base: u64,
    side: i32,
    max: usize,
) -> Vec<Connection> {
    let mut out = Vec::new();
    let (amin, amax) = calc_slab_end_points(va, vb, side);
    let apos = get_slab_coord(va, side);
    let Ok(side_bits) = u16::try_from(side) else {
        return out;
    };
    let m = EXT_LINK | side_bits;
    for (i, poly) in tile.polys.iter().enumerate() {
        let nv = usize::from(poly.vert_count);
        for j in 0..nv {
            // Skip edges which do not point to the right side.
            if poly.neis.get(j).copied() != Some(m) {
                continue;
            }
            let k = (j + 1) % nv;
            let (Some(vc), Some(vd)) = (
                poly.verts.get(j).and_then(|&v| vert(tile, v)),
                poly.verts.get(k).and_then(|&v| vert(tile, v)),
            ) else {
                continue;
            };
            let bpos = get_slab_coord(&vc, side);
            // Segments are not close enough.
            if (apos - bpos).abs() > 0.01 {
                continue;
            }
            // Check if the segments touch.
            let (bmin, bmax) = calc_slab_end_points(&vc, &vd, side);
            if !overlap_slabs(amin, amax, bmin, bmax, 0.01, tile.header.walkable_climb) {
                continue;
            }
            if out.len() < max {
                out.push(Connection {
                    reference: base | u64::try_from(i).unwrap_or(0),
                    lo: dt_max(amin[0], bmin[0]),
                    hi: dt_min(amax[0], bmax[0]),
                });
            }
            break;
        }
    }
    out
}

/// DAYZ: the portal limits are `(unsigned char)(clamp(t, 0, 1) * 255.0f)`, truncated; stock Detour
/// rounds with `roundf`.
fn portal_limits(lo: f32, hi: f32, start: f32, end: f32) -> (u8, u8) {
    let mut tmin = (lo - start) / (end - start);
    let mut tmax = (hi - start) / (end - start);
    if tmin > tmax {
        std::mem::swap(&mut tmin, &mut tmax);
    }
    (
        trunc_u8(dt_clamp(tmin, 0.0, 1.0) * 255.0),
        trunc_u8(dt_clamp(tmax, 0.0, 1.0) * 255.0),
    )
}

/// A link `connectExtLinks` is going to add to polygon `poly`.
struct PortalLink {
    poly: usize,
    edge: u8,
    side: u8,
    reference: u64,
    bounds: Option<(u8, u8)>,
}

/// The links `connectExtLinks(tile, target, side)` adds, in order.
fn plan_ext_links(
    tile: &TileData,
    target: &TileData,
    target_base: u64,
    side: i32,
) -> Vec<PortalLink> {
    let mut plan = Vec::new();
    for (i, poly) in tile.polys.iter().enumerate() {
        let nv = usize::from(poly.vert_count);
        for j in 0..nv {
            let Some(&nei) = poly.neis.get(j) else {
                continue;
            };
            // Skip non-portal edges.
            if nei & EXT_LINK == 0 {
                continue;
            }
            let dir = i32::from(nei & 0xff);
            if side != -1 && dir != side {
                continue;
            }
            let (Some(va), Some(vb)) = (
                poly.verts.get(j).and_then(|&v| vert(tile, v)),
                poly.verts.get((j + 1) % nv).and_then(|&v| vert(tile, v)),
            ) else {
                continue;
            };
            let found = find_connecting_polys(
                &va,
                &vb,
                target,
                target_base,
                opposite_tile(dir),
                MAX_CONNECTING,
            );
            for c in found {
                // Compress the portal limits to a byte value.
                let bounds = if dir == 0 || dir == 4 {
                    Some(portal_limits(c.lo, c.hi, va[2], vb[2]))
                } else if dir == 2 || dir == 6 {
                    Some(portal_limits(c.lo, c.hi, va[0], vb[0]))
                } else {
                    None
                };
                plan.push(PortalLink {
                    poly: i,
                    edge: u8::try_from(j).unwrap_or(0),
                    side: u8::try_from(nei & 0xff).unwrap_or(0),
                    reference: c.reference,
                    bounds,
                });
            }
        }
    }
    plan
}

/// An off-mesh connection of the target tile that lands in the tile.
struct OffMeshLanding {
    /// The off-mesh polygon in the target tile.
    con_poly: usize,
    /// The vertex of that polygon that is the landing point.
    vert: usize,
    /// The point on the mesh of the tile.
    nearest: [f32; 3],
    /// The polygon of the tile it lands on.
    reference: u64,
    bidirectional: bool,
}

/// The landings `connectExtOffMeshLinks(tile, target, side)` makes, in order.
fn plan_ext_off_mesh_links(
    tile: &TileData,
    tile_base: u64,
    target: &TileData,
    opposite_side: u8,
) -> Vec<OffMeshLanding> {
    let mut plan = Vec::new();
    // We are interested in the links which land from the target tile in this tile.
    for con in &target.off_mesh {
        if con.side != opposite_side {
            continue;
        }
        let con_poly = usize::from(con.poly);
        let Some(target_poly) = target.polys.get(con_poly) else {
            continue;
        };
        // Skip off-mesh connections whose start location could not be connected at all.
        if target_poly.first_link == NULL_LINK {
            continue;
        }
        let Some(&vert_index) = target_poly.verts.get(1) else {
            continue;
        };
        let half_extents = [con.rad, target.header.walkable_climb, con.rad];
        // Find the polygon to connect to.
        let p = [con.pos[3], con.pos[4], con.pos[5]];
        let Some((reference, nearest)) =
            find_nearest_poly_in_tile(tile, tile_base, &p, &half_extents)
        else {
            continue;
        };
        // findNearestPoly may return too optimistic results, check again.
        if sqr(nearest[0] - p[0]) + sqr(nearest[2] - p[2]) > sqr(con.rad) {
            continue;
        }
        plan.push(OffMeshLanding {
            con_poly,
            vert: usize::from(vert_index),
            nearest,
            reference,
            bidirectional: con.flags & OFFMESH_CON_BIDIR != 0,
        });
    }
    plan
}

/// `connectIntLinks`.
fn connect_int_links(t: &mut MeshTile, base: u64) {
    for i in 0..t.data.polys.len() {
        let Some(poly) = t.data.polys.get_mut(i) else {
            break;
        };
        poly.first_link = NULL_LINK;
        if poly.poly_type() == POLYTYPE_OFFMESH {
            continue;
        }
        let neis = poly.neis;
        let nv = usize::from(poly.vert_count);
        // Build the edge links backwards so that the links will be in the linked list from the
        // lowest index to the highest.
        for j in (0..nv).rev() {
            let nei = neis.get(j).copied().unwrap_or(0);
            // Skip hard and non-internal edges.
            if nei == 0 || nei & EXT_LINK != 0 {
                continue;
            }
            let target = u64::from(nei - 1);
            push_link(
                t,
                i,
                base | target,
                u8::try_from(j).unwrap_or(0),
                0xff,
                Some((0, 0)),
            );
        }
    }
}

/// `baseOffMeshLinks` (with `connectExtOffMeshLinks(tile, tile, -1)` run by the caller).
///
/// DAYZ: every connection gets `reserved = 1.0` before the search, and the back-link from the
/// landing polygon is only allocated when the forward link was.
fn base_off_mesh_links(t: &mut MeshTile, base: u64) {
    // Base the off-mesh connection start points.
    for i in 0..t.data.off_mesh.len() {
        let Some(con) = t.data.off_mesh.get_mut(i) else {
            break;
        };
        con.reserved = 1.0;
        let (pos, rad, con_poly) = (con.pos, con.rad, usize::from(con.poly));
        let half_extents = [rad, t.data.header.walkable_climb, rad];
        // Find the polygon to connect to: the first vertex.
        let p = [pos[0], pos[1], pos[2]];
        let Some((reference, nearest)) =
            find_nearest_poly_in_tile(&t.data, base, &p, &half_extents)
        else {
            continue;
        };
        // findNearestPoly may return too optimistic results, check again.
        if sqr(nearest[0] - p[0]) + sqr(nearest[2] - p[2]) > sqr(rad) {
            continue;
        }
        // Make sure the location is on the current mesh.
        let first = t
            .data
            .polys
            .get(con_poly)
            .and_then(|poly| poly.verts.first().copied());
        let Some(v) = first.and_then(|v| t.data.verts.get_mut(usize::from(v))) else {
            continue;
        };
        *v = nearest;

        // Link the off-mesh connection to the target polygon.
        if push_link(t, con_poly, reference, 0, 0xff, Some((0, 0))) {
            // The start end point is always connected back to the off-mesh connection.
            let land = usize::try_from(decode_poly_ref(reference).2).unwrap_or(usize::MAX);
            push_link(
                t,
                land,
                base | u64::try_from(con_poly).unwrap_or(0),
                0xff,
                0xff,
                Some((0, 0)),
            );
        }
    }
}

impl NavMesh {
    /// Creates an empty mesh (`dtNavMesh::init`): every slot has salt 1 and slot 0 is allocated
    /// first.
    ///
    /// # Errors
    ///
    /// Returns an error when `max_tiles` is not positive or exceeds what the 28-bit tile field of a
    /// polygon reference can address.
    pub fn new(params: NavMeshParams) -> io::Result<Self> {
        let max_tiles = u32::try_from(params.max_tiles)
            .ok()
            .filter(|&n| n > 0 && n <= MAX_TILE_SLOTS)
            .ok_or_else(|| error!("invalid maxTiles {}", params.max_tiles))?;
        Ok(Self {
            params,
            max_tiles,
            slots: BTreeMap::new(),
            next_fresh: 0,
            free: Vec::new(),
            grid: BTreeMap::new(),
        })
    }

    /// Builds a mesh from a set file: the tiles are added in file order, each into the slot and
    /// with the salt of its stored reference. Stored links are ignored and recomputed (spec 2.3).
    ///
    /// # Errors
    ///
    /// Returns an error for invalid parameters or a tile that [`NavMesh::set_tile`]'s checks
    /// reject: wrong magic or version, an inconsistent header, indices out of range, an occupied
    /// grid cell, or a slot that is out of range or already used.
    pub fn from_set(set: &NavMeshSet) -> io::Result<Self> {
        let mut mesh = Self::new(set.params.clone())?;
        for (i, t) in set.tiles.iter().enumerate() {
            mesh.add_tile(t.data.clone(), t.tile_ref, t.padding)
                .prepend_error(format!("tile record {i}:"))?;
        }
        Ok(mesh)
    }

    /// The mesh parameters.
    #[must_use]
    pub fn params(&self) -> &NavMeshParams {
        &self.params
    }

    /// Adds a tile, replacing the one in the same grid cell (spec 8.2). A replacement keeps its
    /// slot and gets the salt plus one. Returns the reference of the tile.
    ///
    /// # Errors
    ///
    /// Returns an error, leaving the mesh unchanged, when the tile is rejected (see
    /// [`NavMesh::from_set`]) or no slot is free.
    pub fn set_tile(&mut self, data: TileData) -> io::Result<u64> {
        Self::validate_tile(&data)?;
        let key = (data.header.x, data.header.y);
        let last_ref = match self.grid.get(&key).copied() {
            Some(slot) => {
                self.remove_slot(slot);
                encode_poly_ref(self.salt_of(slot), slot, 0)
            }
            None => 0,
        };
        self.add_tile(data, last_ref, 0)
    }

    /// Removes the tile in grid cell `(x, y)` and unlinks its neighbours. Returns whether there
    /// was one.
    pub fn remove_tile_at(&mut self, x: i32, y: i32) -> bool {
        match self.grid.get(&(x, y)).copied() {
            Some(slot) => {
                self.remove_slot(slot);
                true
            }
            None => false,
        }
    }

    /// The tile in grid cell `(x, y)`.
    #[must_use]
    pub fn tile_at(&self, x: i32, y: i32) -> Option<&TileData> {
        let slot = self.grid.get(&(x, y))?;
        self.tile(*slot).map(|t| &t.data)
    }

    /// The tiles with their references, in slot order.
    pub fn tiles(&self) -> impl Iterator<Item = (u64, &TileData)> {
        self.slots.iter().filter_map(|(&slot, s)| {
            s.tile
                .as_ref()
                .map(|t| (encode_poly_ref(s.salt, slot, 0), &t.data))
        })
    }

    /// The mesh as a set file for a `tiles_x` by `tiles_z` grid: the tiles in slot order with
    /// their references and record padding (kept from `from_set`, 0 for tiles added with `set_tile`).
    #[must_use]
    pub fn to_set(&self, tiles_x: i32, tiles_z: i32) -> NavMeshSet {
        NavMeshSet {
            version: SET_VERSION,
            tiles_x,
            tiles_z,
            params: self.params.clone(),
            tiles: self
                .slots
                .iter()
                .filter_map(|(&slot, s)| {
                    s.tile.as_ref().map(|t| SetTile {
                        tile_ref: encode_poly_ref(s.salt, slot, 0),
                        padding: t.padding,
                        data: t.data.clone(),
                    })
                })
                .collect(),
        }
    }

    /// The tiles of the grid cells `min..=max` (`x`, `y`), with their reference bases, row by row
    /// (`y` outer, `x` inner) as `dtNavMeshQuery::queryPolygons` visits them.
    pub(crate) fn tiles_in_range(
        &self,
        (lo_x, lo_y): (i32, i32),
        (hi_x, hi_y): (i32, i32),
    ) -> Vec<(u64, &TileData)> {
        if lo_x > hi_x || lo_y > hi_y {
            return Vec::new();
        }
        // Each side is at most 2^32, so the product can exceed `i64`: saturate (a huge box scans the grid).
        let cells = (i64::from(hi_x) - i64::from(lo_x) + 1)
            .saturating_mul(i64::from(hi_y) - i64::from(lo_y) + 1);
        let mut keys: Vec<(i32, i32)> =
            if usize::try_from(cells).is_ok_and(|c| c <= self.grid.len()) {
                (lo_y..=hi_y)
                    .flat_map(|y| (lo_x..=hi_x).map(move |x| (x, y)))
                    .filter(|k| self.grid.contains_key(k))
                    .collect()
            } else {
                // A box far larger than the grid: scan the grid instead of the cells.
                self.grid
                    .keys()
                    .copied()
                    .filter(|&(x, y)| (lo_x..=hi_x).contains(&x) && (lo_y..=hi_y).contains(&y))
                    .collect()
            };
        keys.sort_by_key(|&(x, y)| (y, x));
        keys.iter()
            .filter_map(|k| {
                let slot = *self.grid.get(k)?;
                Some((self.base_of(slot), &self.tile(slot)?.data))
            })
            .collect()
    }

    /// The polygons of every tile, tile by tile in slot order, for the island filter.
    pub(crate) fn polys_mut(&mut self) -> impl Iterator<Item = (u32, &mut [Poly])> {
        self.slots
            .iter_mut()
            .filter_map(|(&slot, s)| s.tile.as_mut().map(|t| (slot, t.data.polys.as_mut_slice())))
    }

    /// The tile a polygon reference points into, with its slot, if the salt and the polygon index
    /// are valid (`getTileAndPolyByRef`).
    pub(crate) fn tile_by_ref(&self, reference: u64) -> Option<(u32, &TileData)> {
        let (salt, slot, poly) = decode_poly_ref(reference);
        let s = self.slots.get(&slot)?;
        let t = s.tile.as_ref()?;
        let valid = s.salt == salt && usize::try_from(poly).is_ok_and(|p| p < t.data.polys.len());
        valid.then_some((slot, &t.data))
    }

    fn tile(&self, slot: u32) -> Option<&MeshTile> {
        self.slots.get(&slot)?.tile.as_ref()
    }

    fn tile_mut(&mut self, slot: u32) -> Option<&mut MeshTile> {
        self.slots.get_mut(&slot)?.tile.as_mut()
    }

    fn salt_of(&self, slot: u32) -> u32 {
        self.slots.get(&slot).map_or(1, |s| s.salt)
    }

    fn base_of(&self, slot: u32) -> u64 {
        encode_poly_ref(self.salt_of(slot), slot, 0)
    }

    /// Checks everything the linker relies on, so that it never sees an index out of range.
    fn validate_tile(d: &TileData) -> io::Result<()> {
        let h = &d.header;
        if h.magic != NAVMESH_MAGIC {
            return Err(error!("tile has the wrong magic"));
        }
        if h.version != NAVMESH_VERSION {
            return Err(error!(
                "tile has version {}, not {NAVMESH_VERSION}",
                h.version
            ));
        }
        if h.layer != 0 {
            return Err(error!("tile layer {} (only layer 0 is supported)", h.layer));
        }
        let counts = [
            ("polygon", h.poly_count, d.polys.len()),
            ("vertex", h.vert_count, d.verts.len()),
            ("link", h.max_link_count, d.links.len()),
            ("BV node", h.bv_node_count, d.bv_tree.len()),
            (
                "off-mesh connection",
                h.off_mesh_con_count,
                d.off_mesh.len(),
            ),
        ];
        for (what, header, actual) in counts {
            if usize::try_from(header).ok() != Some(actual) {
                return Err(error!(
                    "header says {header} {what}s, the tile has {actual}"
                ));
            }
        }
        if d.polys.len() > MAX_POLYS_PER_TILE {
            return Err(error!(
                "{} polygons do not fit a polygon reference",
                d.polys.len()
            ));
        }
        for (i, p) in d.polys.iter().enumerate() {
            let nv = usize::from(p.vert_count);
            if nv > p.verts.len() {
                return Err(error!("polygon {i} has {nv} vertices"));
            }
            for (&v, &nei) in p.verts.iter().zip(&p.neis).take(nv) {
                if usize::from(v) >= d.verts.len() {
                    return Err(error!("polygon {i} uses vertex {v} outside the tile"));
                }
                if nei != 0 && nei & EXT_LINK == 0 && usize::from(nei) > d.polys.len() {
                    return Err(error!("polygon {i} has neighbour {nei} outside the tile"));
                }
            }
        }
        for (i, c) in d.off_mesh.iter().enumerate() {
            let ok = d
                .polys
                .get(usize::from(c.poly))
                .is_some_and(|p| p.vert_count >= 2);
            if !ok {
                return Err(error!(
                    "off-mesh connection {i} has no polygon with two vertices"
                ));
            }
        }
        Ok(())
    }

    /// `dtNavMesh::addTile` with `lastRef` (0: none); returns the tile reference.
    fn add_tile(&mut self, mut data: TileData, last_ref: u64, padding: u32) -> io::Result<u64> {
        Self::validate_tile(&data)?;
        let key = (data.header.x, data.header.y);
        if self.grid.contains_key(&key) {
            return Err(error!(
                "tile cell ({}, {}) is already occupied",
                key.0, key.1
            ));
        }
        let (slot, salt) = if last_ref == 0 {
            self.alloc_next()?
        } else {
            self.alloc_specific(last_ref)?
        };

        // Build the link free list.
        let n = data.links.len();
        for (i, l) in data.links.iter_mut().enumerate() {
            l.next = if i + 1 < n {
                u32::try_from(i + 1).unwrap_or(NULL_LINK)
            } else {
                NULL_LINK
            };
        }
        let links_free_list = if n == 0 { NULL_LINK } else { 0 };
        self.slots.insert(
            slot,
            Slot {
                salt,
                tile: Some(MeshTile {
                    data,
                    links_free_list,
                    padding,
                }),
            },
        );
        self.grid.insert(key, slot);
        self.link_tile(slot, key);
        Ok(encode_poly_ref(salt, slot, 0))
    }

    /// Takes the head of the free list, else the next never-used slot.
    fn alloc_next(&mut self) -> io::Result<(u32, u32)> {
        if let Some(slot) = self.free.pop() {
            return Ok((slot, self.salt_of(slot)));
        }
        while self.slots.contains_key(&self.next_fresh) {
            self.next_fresh = self.next_fresh.saturating_add(1);
        }
        if self.next_fresh >= self.max_tiles {
            return Err(error!("no free tile slot (maxTiles {})", self.max_tiles));
        }
        let slot = self.next_fresh;
        self.next_fresh = self.next_fresh.saturating_add(1);
        Ok((slot, 1))
    }

    /// Takes the slot of `last_ref` out of the free list and restores its salt.
    fn alloc_specific(&mut self, last_ref: u64) -> io::Result<(u32, u32)> {
        let (salt, slot, _) = decode_poly_ref(last_ref);
        if slot >= self.max_tiles {
            return Err(error!(
                "tile slot {slot} is not below maxTiles {}",
                self.max_tiles
            ));
        }
        if salt == 0 {
            return Err(error!("tile reference {last_ref:#x} has salt 0"));
        }
        if self.tile(slot).is_some() {
            return Err(error!("tile slot {slot} is already in use"));
        }
        self.free.retain(|&s| s != slot);
        Ok((slot, salt))
    }

    /// The linking half of `addTile`.
    fn link_tile(&mut self, slot: u32, (x, y): (i32, i32)) {
        let base = self.base_of(slot);
        if let Some(t) = self.tile_mut(slot) {
            connect_int_links(t, base);
            // Base the off-mesh connections to their starting polygons and connect the
            // connections inside the tile.
            base_off_mesh_links(t, base);
        }
        self.connect_ext_off_mesh_links(slot, slot, -1);

        // Create connections with the neighbour tiles.
        for side in 0..8 {
            let Some(nei) = self.neighbour_slot(x, y, side) else {
                continue;
            };
            self.connect_ext_links(slot, nei, side);
            self.connect_ext_links(nei, slot, opposite_tile(side));
            self.connect_ext_off_mesh_links(slot, nei, side);
            self.connect_ext_off_mesh_links(nei, slot, opposite_tile(side));
        }

        self.snap_obstacles(slot);
    }

    /// `getNeighbourTilesAt`.
    fn neighbour_slot(&self, x: i32, y: i32, side: i32) -> Option<u32> {
        let (dx, dy) = match side {
            0 => (1, 0),
            1 => (1, 1),
            2 => (0, 1),
            3 => (-1, 1),
            4 => (-1, 0),
            5 => (-1, -1),
            6 => (0, -1),
            7 => (1, -1),
            _ => return None,
        };
        let key = (x.checked_add(dx)?, y.checked_add(dy)?);
        self.grid.get(&key).copied()
    }

    /// `connectExtLinks(tile, target, side)`.
    fn connect_ext_links(&mut self, tile: u32, target: u32, side: i32) {
        let plan = match (self.tile(tile), self.tile(target)) {
            (Some(t), Some(tg)) => plan_ext_links(&t.data, &tg.data, self.base_of(target), side),
            _ => return,
        };
        let Some(t) = self.tile_mut(tile) else {
            return;
        };
        for l in plan {
            push_link(t, l.poly, l.reference, l.edge, l.side, l.bounds);
        }
    }

    /// `connectExtOffMeshLinks(tile, target, side)`; `tile == target` for `side == -1`.
    fn connect_ext_off_mesh_links(&mut self, tile: u32, target: u32, side: i32) {
        let opposite_side = if side == -1 {
            0xff
        } else {
            u8::try_from(opposite_tile(side)).unwrap_or(0xff)
        };
        let tile_base = self.base_of(tile);
        let target_base = self.base_of(target);
        let plan = match (self.tile(tile), self.tile(target)) {
            (Some(t), Some(tg)) => {
                plan_ext_off_mesh_links(&t.data, tile_base, &tg.data, opposite_side)
            }
            _ => return,
        };
        for l in plan {
            // Link the off-mesh connection to the target polygon.
            if let Some(tg) = self.tile_mut(target) {
                // Make sure the location is on the current mesh.
                if let Some(v) = tg.data.verts.get_mut(l.vert) {
                    *v = l.nearest;
                }
                push_link(tg, l.con_poly, l.reference, 1, opposite_side, Some((0, 0)));
            }
            // Link the target polygon to the off-mesh connection.
            if l.bidirectional
                && let Some(t) = self.tile_mut(tile)
            {
                let land = usize::try_from(decode_poly_ref(l.reference).2).unwrap_or(usize::MAX);
                let back_side = if side == -1 {
                    0xff
                } else {
                    u8::try_from(side).unwrap_or(0xff)
                };
                push_link(
                    t,
                    land,
                    target_base | u64::try_from(l.con_poly).unwrap_or(0),
                    0xff,
                    back_side,
                    Some((0, 0)),
                );
            }
        }
    }

    /// DAYZ (spec 8.3): snaps the y of every obstacle of the extra section to the nearest
    /// polygon of its tile, within a vertical half extent of 30. Not idempotent for obstacles
    /// that are not over a polygon.
    fn snap_obstacles(&mut self, slot: u32) {
        let base = self.base_of(slot);
        let Some(t) = self.tile_mut(slot) else {
            return;
        };
        let positions: Vec<[f32; 3]> = t
            .data
            .extra
            .iter()
            .flat_map(|e| e.obstacles.iter().map(|o| o.pos))
            .collect();
        let half_extents = [0.0, OBSTACLE_SNAP_HALF_HEIGHT, 0.0];
        let snapped: Vec<Option<f32>> = positions
            .iter()
            .map(|p| {
                find_nearest_poly_in_tile(&t.data, base, p, &half_extents).map(|(_, pt)| pt[1])
            })
            .collect();
        let obstacles = t.data.extra.iter_mut().flat_map(|e| e.obstacles.iter_mut());
        for (o, y) in obstacles.zip(snapped) {
            if let Some(y) = y {
                o.pos[1] = y;
            }
        }
    }

    /// `removeTile`: unlinks the neighbours, bumps the salt and puts the slot at the head of the
    /// free list.
    fn remove_slot(&mut self, slot: u32) {
        let Some((x, y)) = self.tile(slot).map(|t| (t.data.header.x, t.data.header.y)) else {
            return;
        };
        self.grid.remove(&(x, y));
        for side in 0..8 {
            if let Some(nei) = self.neighbour_slot(x, y, side) {
                self.unconnect_links(nei, slot);
            }
        }
        if let Some(s) = self.slots.get_mut(&slot) {
            s.tile = None;
            s.salt = salt_after_removal(s.salt);
        }
        self.free.push(slot);
    }

    /// `unconnectLinks`: removes the links of `tile` that point into the tile in slot `target`.
    fn unconnect_links(&mut self, tile: u32, target: u32) {
        let Some(t) = self.tile_mut(tile) else {
            return;
        };
        let polys = t.data.polys.len();
        for i in 0..polys {
            let mut j = t.data.polys.get(i).map_or(NULL_LINK, |p| p.first_link);
            let mut pj = NULL_LINK;
            // A tile owns its links, so the chain ends; the bound only guards a corrupted pool.
            let mut steps = t.data.links.len();
            while j != NULL_LINK && steps > 0 {
                steps -= 1;
                let Some(link) = usize::try_from(j).ok().and_then(|k| t.data.links.get(k)) else {
                    break;
                };
                let nj = link.next;
                if decode_poly_ref(link.reference).1 == target {
                    // Remove the link.
                    if pj == NULL_LINK {
                        if let Some(p) = t.data.polys.get_mut(i) {
                            p.first_link = nj;
                        }
                    } else if let Some(prev) = usize::try_from(pj)
                        .ok()
                        .and_then(|k| t.data.links.get_mut(k))
                    {
                        prev.next = nj;
                    }
                    free_link(t, j);
                    j = nj;
                } else {
                    // Advance.
                    pj = j;
                    j = nj;
                }
            }
        }
    }
}
