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

//! The queries of `dtNavMesh` and `dtNavMeshQuery` the tile linker and the island filter need:
//! `queryPolygonsInTile`, `findNearestPolyInTile`, `closestPointOnPoly`, `getPolyHeight` and the
//! multi-tile [`find_nearest_poly`] with a query filter.
//!
//! The last two are the pre-2016 behaviour DayZ uses (spec 10.2): there is no detail mesh, so the
//! height comes from the first triangle of the polygon's fan that contains the position, with the
//! old barycentric test.

use crate::navmesh::detour::common::{
    distance_pt_poly_edges_sqr, dt_clamp, dt_max, dt_min, overlap_bounds, overlap_quant_bounds,
    point_in_polygon, vadd, vdist, vlerp, vsub,
};
use crate::navmesh::detour::mesh::NavMesh;
use crate::navmesh::detour::set::decode_poly_ref;
use crate::navmesh::detour::tile::{Poly, TileData};
use crate::navmesh::num::{floor_i32, trunc_u16};

/// `DT_POLYTYPE_OFFMESH_CONNECTION`.
const POLYTYPE_OFFMESH: u8 = 1;
/// The largest number of vertices of a polygon.
const VERTS_PER_POLYGON: usize = 6;
/// The size of the polygon buffer of `findNearestPolyInTile`.
const NEAREST_MAX_POLYS: usize = 128;

/// The vertex positions of polygon `poly`, or `None` when the polygon or one of its vertices does
/// not exist.
fn poly_verts(tile: &TileData, poly: usize) -> Option<([[f32; 3]; VERTS_PER_POLYGON], usize)> {
    let p = tile.polys.get(poly)?;
    let nv = usize::from(p.vert_count);
    if nv > VERTS_PER_POLYGON {
        return None;
    }
    let mut out = [[0.0_f32; 3]; VERTS_PER_POLYGON];
    for (slot, &vi) in out.iter_mut().zip(&p.verts).take(nv) {
        *slot = *tile.verts.get(usize::from(vi))?;
    }
    Some((out, nv))
}

/// The pre-2016 `closestHeightPointTriangle`: the height of the triangle `a`, `b`, `c` at the xz
/// position of `p`, with the old, sloppier barycentric tolerances.
#[expect(
    clippy::many_single_char_names,
    reason = "the names of the upstream function are kept for comparison"
)]
fn closest_height_point_triangle(
    p: &[f32; 3],
    a: &[f32; 3],
    b: &[f32; 3],
    c: &[f32; 3],
) -> Option<f32> {
    let v0 = vsub(c, a);
    let v1 = vsub(b, a);
    let v2 = vsub(p, a);
    let dot00 = v0[0] * v0[0] + v0[2] * v0[2];
    let dot01 = v0[0] * v1[0] + v0[2] * v1[2];
    let dot02 = v0[0] * v2[0] + v0[2] * v2[2];
    let dot11 = v1[0] * v1[0] + v1[2] * v1[2];
    let dot12 = v1[0] * v2[0] + v1[2] * v2[2];
    let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01);
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;
    if u >= -1e-3 && v >= -1e-3 && u + v <= 1.001 {
        Some((v0[1] * u + a[1]) + v1[1] * v)
    } else {
        None
    }
}

/// The height at `pos` from the first triangle `(v0, v[j+1], v[j+2])` of the polygon's fan that
/// contains it.
fn fan_height(verts: &[[f32; 3]], pos: &[f32; 3]) -> Option<f32> {
    let a = verts.first()?;
    verts
        .windows(2)
        .skip(1)
        .find_map(|w| closest_height_point_triangle(pos, a, w.first()?, w.get(1)?))
}

/// `getPolyHeight`: the height of polygon `poly` at the xz position of `pos`, or `None` when the
/// position is not over the polygon (or the polygon is an off-mesh connection).
///
/// A position over the polygon that no fan triangle contains (a degenerate triangle) gets the
/// height of the closest point on the polygon's edges.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "port of getPolyHeight; the nearest-polygon query and the island filter do not need it"
    )
)]
pub(crate) fn get_poly_height(tile: &TileData, poly: usize, pos: &[f32; 3]) -> Option<f32> {
    if tile.polys.get(poly)?.poly_type() == POLYTYPE_OFFMESH {
        return None;
    }
    let (arr, nv) = poly_verts(tile, poly)?;
    let verts = arr.get(..nv)?;
    if !point_in_polygon(pos, verts) {
        return None;
    }
    if let Some(h) = fan_height(verts, pos) {
        return Some(h);
    }
    let (_, closest) = clamp_to_edges(pos, verts)?;
    Some(closest[1])
}

/// The nearest edge of the polygon to `pos` and the point of it that is closest.
fn clamp_to_edges(pos: &[f32; 3], verts: &[[f32; 3]]) -> Option<(bool, [f32; 3])> {
    let mut ed = [0.0_f32; VERTS_PER_POLYGON];
    let mut et = [0.0_f32; VERTS_PER_POLYGON];
    let inside = distance_pt_poly_edges_sqr(pos, verts, &mut ed, &mut et)?;
    // As upstream: start from the first edge and scan the rest.
    let mut dmin = *ed.first()?;
    let mut imin = 0;
    for (i, &d) in ed.iter().enumerate().take(verts.len()).skip(1) {
        if d < dmin {
            dmin = d;
            imin = i;
        }
    }
    let va = verts.get(imin)?;
    let vb = verts.get((imin + 1) % verts.len())?;
    Some((inside, vlerp(va, vb, *et.get(imin)?)))
}

/// `closestPointOnPoly`: the point of polygon `poly` closest to `pos` and whether `pos` is over
/// the polygon (`posOverPoly`). `None` when the polygon does not exist.
///
/// Over the polygon, the point is `pos` at the height of the fan triangle containing it (its own
/// `y` when no triangle does). An off-mesh polygon interpolates
/// between its two end points by the ratio of the distances. Otherwise the point is clamped to the
/// nearest edge, and its height is that of the fan triangle containing the *original* position, if
/// one does, else the clamped edge point's own height (spec 10.2).
pub(crate) fn closest_point_on_poly(
    tile: &TileData,
    poly: usize,
    pos: &[f32; 3],
) -> Option<([f32; 3], bool)> {
    let p = tile.polys.get(poly)?;
    let mut closest = *pos;
    if p.poly_type() != POLYTYPE_OFFMESH {
        let (arr, nv) = poly_verts(tile, poly)?;
        let verts = arr.get(..nv)?;
        if point_in_polygon(pos, verts) {
            // Over the polygon: the point keeps its own y when no fan triangle contains it.
            closest[1] = fan_height(verts, pos).unwrap_or(pos[1]);
            return Some((closest, true));
        }
    }
    if p.poly_type() == POLYTYPE_OFFMESH {
        let v0 = tile.verts.get(usize::from(*p.verts.first()?))?;
        let v1 = tile.verts.get(usize::from(*p.verts.get(1)?))?;
        let d0 = vdist(pos, v0);
        let d1 = vdist(pos, v1);
        let sum = d0 + d1;
        let u = if sum > 0.0 { d0 / sum } else { 0.0 };
        return Some((vlerp(v0, v1, u), false));
    }
    let (arr, nv) = poly_verts(tile, poly)?;
    let verts = arr.get(..nv)?;
    let (_, mut closest) = clamp_to_edges(pos, verts)?;
    if let Some(h) = fan_height(verts, pos) {
        closest[1] = h;
    }
    Some((closest, false))
}

/// `queryPolygonsInTile`: the references (`base | poly`) of the polygons whose bounds overlap the
/// box `qmin`-`qmax`, through the BV tree when the tile has one, else by brute force (which skips
/// off-mesh polygons), at most `max_polys` of them.
pub(crate) fn query_polygons_in_tile(
    tile: &TileData,
    base: u64,
    qmin: &[f32; 3],
    qmax: &[f32; 3],
    max_polys: usize,
) -> Vec<u64> {
    query_polygons_filtered(tile, base, qmin, qmax, max_polys, &|_| true)
}

/// `dtNavMeshQuery::queryPolygonsInTile`: [`query_polygons_in_tile`] that only reports polygons
/// that pass `filter` (the filter runs before the `max_polys` limit counts).
pub(crate) fn query_polygons_filtered(
    tile: &TileData,
    base: u64,
    qmin: &[f32; 3],
    qmax: &[f32; 3],
    max_polys: usize,
    filter: &dyn Fn(&Poly) -> bool,
) -> Vec<u64> {
    let mut out = Vec::new();
    if tile.bv_tree.is_empty() {
        for (i, p) in tile.polys.iter().enumerate() {
            if p.poly_type() == POLYTYPE_OFFMESH || !filter(p) {
                continue;
            }
            let Some((arr, nv)) = poly_verts(tile, i) else {
                continue;
            };
            let Some((first, rest)) = arr.get(..nv).and_then(<[_]>::split_first) else {
                continue;
            };
            let mut bmin = *first;
            let mut bmax = *first;
            for v in rest {
                for ((lo, hi), &c) in bmin.iter_mut().zip(bmax.iter_mut()).zip(v) {
                    *lo = dt_min(*lo, c);
                    *hi = dt_max(*hi, c);
                }
            }
            if overlap_bounds(qmin, qmax, &bmin, &bmax) && out.len() < max_polys {
                out.push(base | u64::try_from(i).unwrap_or(0));
            }
        }
        return out;
    }

    let tbmin = &tile.header.bmin;
    let tbmax = &tile.header.bmax;
    let qfac = tile.header.bv_quant_factor;
    // Clamp the query box to the tile box, then quantize.
    let quant = |v: f32, lo: f32, hi: f32| qfac * (dt_clamp(v, lo, hi) - lo);
    let min_q = |v: f32, lo: f32, hi: f32| trunc_u16(quant(v, lo, hi)) & 0xfffe;
    let max_q = |v: f32, lo: f32, hi: f32| trunc_u16(quant(v, lo, hi) + 1.0) | 1;
    let bmin = [
        min_q(qmin[0], tbmin[0], tbmax[0]),
        min_q(qmin[1], tbmin[1], tbmax[1]),
        min_q(qmin[2], tbmin[2], tbmax[2]),
    ];
    let bmax = [
        max_q(qmax[0], tbmin[0], tbmax[0]),
        max_q(qmax[1], tbmin[1], tbmax[1]),
        max_q(qmax[2], tbmin[2], tbmax[2]),
    ];

    let mut at = 0_usize;
    while let Some(node) = tile.bv_tree.get(at) {
        let overlap = overlap_quant_bounds(bmin, bmax, node.bmin, node.bmax);
        let leaf = node.i >= 0;
        if leaf
            && overlap
            && out.len() < max_polys
            && let Ok(i) = usize::try_from(node.i)
            && tile
                .polys
                .get(i)
                .is_some_and(|p| p.poly_type() != POLYTYPE_OFFMESH && filter(p))
        {
            out.push(base | u64::try_from(i).unwrap_or(0));
        }
        if overlap || leaf {
            at += 1;
        } else {
            // The escape index skips the whole subtree.
            let Some(escape) = node.i.checked_neg().and_then(|e| usize::try_from(e).ok()) else {
                break;
            };
            let Some(next) = at.checked_add(escape) else {
                break;
            };
            at = next;
        }
    }
    out
}

/// The running result of `dtFindNearestPolyQuery`.
struct Nearest {
    distance_sqr: f32,
    found: Option<(u64, [f32; 3])>,
}

impl Nearest {
    fn new() -> Self {
        Self {
            distance_sqr: f32::MAX,
            found: None,
        }
    }

    /// `dtFindNearestPolyQuery::process` for the polygons `refs` of `tile`.
    ///
    /// A position directly over a polygon counts by its vertical distance beyond the tile's
    /// `walkableClimb` only, which favours it over a nearer polygon the position is not over.
    fn process(&mut self, tile: &TileData, refs: &[u64], center: &[f32; 3]) {
        for &r in refs {
            let idx = usize::try_from(decode_poly_ref(r).2).unwrap_or(usize::MAX);
            let Some((closest, over)) = closest_point_on_poly(tile, idx, center) else {
                continue;
            };
            let diff = vsub(center, &closest);
            let d = if over {
                let d = diff[1].abs() - tile.header.walkable_climb;
                if d > 0.0 { d * d } else { 0.0 }
            } else {
                diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]
            };
            if d < self.distance_sqr {
                self.found = Some((r, closest));
                self.distance_sqr = d;
            }
        }
    }
}

/// `findNearestPolyInTile`: the polygon of the tile nearest to `center` within the box of
/// `half_extents`, and the nearest point on it.
pub(crate) fn find_nearest_poly_in_tile(
    tile: &TileData,
    base: u64,
    center: &[f32; 3],
    half_extents: &[f32; 3],
) -> Option<(u64, [f32; 3])> {
    let bmin = vsub(center, half_extents);
    let bmax = vadd(center, half_extents);
    let polys = query_polygons_in_tile(tile, base, &bmin, &bmax, NEAREST_MAX_POLYS);
    let mut nearest = Nearest::new();
    nearest.process(tile, &polys, center);
    nearest.found
}

/// `dtNavMeshQuery::findNearestPoly`: the polygon of the mesh nearest to `center` within the box
/// `center` plus or minus `half_extents`, and the nearest point on it.
///
/// A polygon is a candidate when its flags have a bit of `include` and none of `exclude` (the
/// stock `dtQueryFilter`). Off-mesh connections are never candidates. The candidates come from
/// the BV trees of the tiles the box touches (`calcTileLoc` of its corners, rows first), and the
/// nearest is chosen as in Detour: a position directly over a polygon counts by its vertical
/// distance beyond the tile's `walkableClimb` only. The point uses the DayZ
/// `closestPointOnPoly` (spec 10.2).
///
/// Returns `None` when no candidate exists.
#[must_use]
pub fn find_nearest_poly(
    mesh: &NavMesh,
    center: [f32; 3],
    half_extents: [f32; 3],
    include: u16,
    exclude: u16,
) -> Option<(u64, [f32; 3])> {
    // As upstream (`dtVisfinite`): a non-finite centre or extent is a failure.
    if !center.iter().chain(&half_extents).all(|v| v.is_finite()) {
        return None;
    }
    let bmin = vsub(&center, &half_extents);
    let bmax = vadd(&center, &half_extents);
    let params = mesh.params();
    let loc = |pos: &[f32; 3]| {
        (
            floor_i32((pos[0] - params.orig[0]) / params.tile_width),
            floor_i32((pos[2] - params.orig[2]) / params.tile_height),
        )
    };
    let (lo_x, lo_y) = loc(&bmin);
    let (hi_x, hi_y) = loc(&bmax);
    let pass = |p: &Poly| p.flags & include != 0 && p.flags & exclude == 0;
    let mut nearest = Nearest::new();
    for (base, tile) in mesh.tiles_in_range((lo_x, lo_y), (hi_x, hi_y)) {
        let polys = query_polygons_filtered(tile, base, &bmin, &bmax, usize::MAX, &pass);
        nearest.process(tile, &polys, &center);
    }
    nearest.found
}

#[cfg(test)]
#[expect(
    clippy::field_reassign_with_default,
    reason = "the tile types are non_exhaustive; tests fill them field by field"
)]
mod tests {
    use super::*;
    use crate::navmesh::detour::tile::{BvNode, MeshHeader, Poly};

    /// One quad `(0,0)-(10,10)` sloping from y 0 at x 0 to y 10 at x 10, and an off-mesh polygon.
    fn slope() -> TileData {
        let mut quad = Poly::default();
        quad.verts = [0, 1, 2, 3, 0, 0];
        quad.vert_count = 4;
        let mut off = Poly::default();
        off.verts = [4, 5, 0, 0, 0, 0];
        off.vert_count = 2;
        off.set_type(1);
        let mut t = TileData::default();
        t.header = MeshHeader::default();
        t.header.bmin = [0.0; 3];
        t.header.bmax = [10.0, 10.0, 10.0];
        t.header.bv_quant_factor = 1.0;
        t.header.walkable_climb = 0.5;
        t.verts = vec![
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 10.0],
            [10.0, 10.0, 10.0],
            [10.0, 10.0, 0.0],
            [20.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
        ];
        t.polys = vec![quad, off];
        t
    }

    #[test]
    fn height_over_a_sloped_polygon() {
        let t = slope();
        let h = get_poly_height(&t, 0, &[5.0, 99.0, 5.0]).expect("over the polygon");
        assert!((h - 5.0).abs() < 1e-4);
        assert!(get_poly_height(&t, 0, &[15.0, 0.0, 5.0]).is_none());
        assert!(get_poly_height(&t, 1, &[25.0, 0.0, 0.0]).is_none());
        let (c, over) = closest_point_on_poly(&t, 0, &[2.0, 50.0, 8.0]).expect("poly");
        assert!(over && (c[1] - 2.0).abs() < 1e-4 && (c[0] - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn inside_point_no_fan_triangle_contains_keeps_its_y() {
        // With huge coordinates the barycentric denominators overflow to inf - inf = NaN, so no
        // fan triangle contains the point, which is still over the polygon (the numerically
        // degenerate case of upstream's "directly over, but no triangle" fall-through).
        let mut t = slope();
        t.verts = vec![
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1e10],
            [1e10, 0.0, 1e10],
            [1e10, 0.0, 0.0],
            [0.0; 3],
            [0.0; 3],
        ];
        let pos = [5e9, 77.0, 5e9];
        assert!(fan_height(&t.verts[..4], &pos).is_none());
        let (c, over) = closest_point_on_poly(&t, 0, &pos).expect("poly");
        assert!(over);
        assert!((c[1] - 77.0).abs() < f32::EPSILON);
        // `get_poly_height` falls back to the height of the closest edge point.
        let h = get_poly_height(&t, 0, &pos).expect("over the polygon");
        assert!(h.abs() < f32::EPSILON);
    }

    #[test]
    fn outside_points_clamp_to_the_nearest_edge() {
        let t = slope();
        // 5 m beyond the x = 10 edge at z 5: the clamped point is (10, 10, 5).
        let (c, over) = closest_point_on_poly(&t, 0, &[15.0, 0.0, 5.0]).expect("poly");
        assert!(!over);
        assert!((c[0] - 10.0).abs() < 1e-4 && (c[2] - 5.0).abs() < 1e-4);
        assert!((c[1] - 10.0).abs() < 1e-4);
    }

    /// A quad whose second fan triangle is not coplanar with the first: `v2` is 20 m up, so the
    /// first triangle `(v0, v1, v2)` is `y = 2x` and the second `(v0, v2, v3)` is `y = 2z`.
    fn warped() -> TileData {
        let mut t = slope();
        t.verts[2] = [10.0, 20.0, 10.0];
        t.verts[3] = [10.0, 0.0, 0.0];
        t
    }

    #[test]
    fn closest_point_inside_uses_fan_height() {
        let t = warped();
        // 0.009 m below the diagonal v0-v2 (z < x): inside the second triangle, and inside the
        // first only by the 1e-3 barycentric tolerance. The first triangle containing it wins.
        let pos = [5.0, 77.0, 4.991];
        let (c, over) = closest_point_on_poly(&t, 0, &pos).expect("poly");
        assert!(over);
        assert!((c[0] - 5.0).abs() < 1e-6 && (c[2] - 4.991).abs() < 1e-6);
        // First triangle: 2 * x = 10.0. The second would give 2 * z = 9.982.
        assert!((c[1] - 10.0).abs() < 0.005, "height {}", c[1]);
        let second = closest_height_point_triangle(&pos, &t.verts[0], &t.verts[2], &t.verts[3])
            .expect("second triangle contains it");
        assert!((second - 9.982).abs() < 0.005 && (second - c[1]).abs() > 0.01);
    }

    #[test]
    fn closest_point_outside_clamps_to_edge() {
        let t = slope();
        // 5 mm beyond the x = 10 edge: clamped to x = 10, but a fan triangle contains the
        // original position within tolerance, so the height is the fan height there (10.005),
        // not the height of the clamped point (10.0).
        let (c, over) = closest_point_on_poly(&t, 0, &[10.005, 0.0, 5.0]).expect("poly");
        assert!(!over);
        assert!((c[0] - 10.0).abs() < 1e-4 && (c[2] - 5.0).abs() < 1e-4);
        assert!((c[1] - 10.005).abs() < 1e-3, "height {}", c[1]);
    }

    #[test]
    fn off_mesh_polygons_interpolate_by_distance() {
        let t = slope();
        let (c, over) = closest_point_on_poly(&t, 1, &[22.0, 0.0, 0.0]).expect("poly");
        assert!(!over);
        // 2 m from the start and 8 m from the end: u = 0.2.
        assert!((c[0] - 22.0).abs() < 1e-4);
        assert!(closest_point_on_poly(&t, 9, &[0.0; 3]).is_none());
    }

    #[test]
    fn brute_force_query_skips_off_mesh_polygons() {
        let t = slope();
        let all = query_polygons_in_tile(&t, 7 << 20, &[-1.0; 3], &[40.0; 3], 8);
        assert_eq!(all, vec![7 << 20]);
        assert_eq!(
            query_polygons_in_tile(&t, 0, &[50.0; 3], &[60.0; 3], 8).len(),
            0
        );
        assert_eq!(
            query_polygons_in_tile(&t, 0, &[-1.0; 3], &[40.0; 3], 0).len(),
            0
        );
    }

    #[test]
    fn bv_tree_query_follows_escape_indices() {
        let mut t = slope();
        // Root over two leaves (poly 0 twice is enough to exercise the walk) and a bad leaf.
        let mut root = BvNode::default();
        root.bmin = [0; 3];
        root.bmax = [10; 3];
        root.i = -3;
        let mut leaf = BvNode::default();
        leaf.bmin = [0; 3];
        leaf.bmax = [10; 3];
        let mut stray = leaf.clone();
        stray.i = 40; // not a polygon of the tile
        t.bv_tree = vec![root, leaf, stray];
        let hits = query_polygons_in_tile(&t, 0, &[1.0; 3], &[2.0; 3], 8);
        assert_eq!(hits, vec![0]);
        // A query outside the root's box takes the escape index and finds nothing.
        t.header.bmax = [100.0; 3];
        assert_eq!(
            query_polygons_in_tile(&t, 0, &[50.0; 3], &[60.0; 3], 8).len(),
            0
        );
    }

    #[test]
    fn nearest_poly_prefers_the_one_below_within_climb() {
        let t = slope();
        // Directly over the polygon, 0.4 m above the surface, within the climb height: distance 0.
        let (r, p) = find_nearest_poly_in_tile(&t, 3 << 20, &[5.0, 5.4, 5.0], &[0.0, 5.0, 0.0])
            .expect("found");
        assert_eq!(r, 3 << 20);
        assert!((p[1] - 5.0).abs() < 1e-4);
        assert!(find_nearest_poly_in_tile(&t, 0, &[50.0, 0.0, 5.0], &[1.0; 3]).is_none());
    }
}
