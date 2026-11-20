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

//! `rcBuildContours` with the DayZ part ids and the vertical simplification error (spec 5.4 step 9,
//! 6.2 step 1, 10.1 item 7).
//!
//! The port follows `RecastContour.cpp` at `6d1f971`. The DayZ differences are:
//!
//! * `RC_AREA_BORDER` is set on an edge when the area **or** the part id (`sub_area`) of the two
//!   spans differs ([`walk_contour`]);
//! * the "two same interior cells" test of `getCornerHeight` also requires equal part ids;
//! * each contour stores the part id of its first span;
//! * [`simplify_contour`] takes `max_vert_error` (voxels, `0` = off). After the horizontal pass has
//!   found its worst raw point, the vertical deviation of the raw points from the segment's linearly
//!   interpolated height is tracked as well. When the horizontal test does not insert a vertex and the
//!   worst vertical deviation exceeds `max_vert_error`, that raw point is inserted, and it is tagged
//!   `RC_BORDER_VERTEX | RC_AREA_BORDER` in the raw contour;
//! * the "tessellate only outer edges or edges between areas" gate of that loop accepts raw points
//!   that carry `RC_AREA_BORDER` or `RC_BORDER_VERTEX` (ruling R10). The widened gate covers both the
//!   horizontal and the vertical tracking of the loop; the gate of the `max_edge_len` split is stock.
//!
//! Hole merging is iterative, as upstream. The two `qsort` calls (holes, diagonal candidates) use the
//! order of the MSVC `qsort` for equal keys (ruling R18).

#![expect(
    clippy::similar_names,
    clippy::many_single_char_names,
    reason = "geometry code keeps Recast's variable names"
)]

use std::io;

use super::compact::CompactHeightfield;
use super::region::BORDER_REG;
use crate::navmesh::num::i32_to_f32;
use crate::navmesh::qsort::msvc_qsort;

/// `RC_BORDER_VERTEX`: the vertex lies on a tile border.
pub const RC_BORDER_VERTEX: i32 = 0x10000;
/// `RC_AREA_BORDER`: the edge lies between two areas (or two part ids).
pub const RC_AREA_BORDER: i32 = 0x20000;
/// `RC_CONTOUR_REG_MASK`: the region bits of a contour vertex flag.
pub const RC_CONTOUR_REG_MASK: i32 = 0xffff;
/// `RC_CONTOUR_TESS_WALL_EDGES`: split long solid edges.
pub const RC_CONTOUR_TESS_WALL_EDGES: i32 = 0x01;
/// `RC_CONTOUR_TESS_AREA_EDGES`: split long edges between areas.
pub const RC_CONTOUR_TESS_AREA_EDGES: i32 = 0x02;

/// `walkContour` gives up after this many steps.
const MAX_WALK_ITERATIONS: u32 = 40_000;

/// `rcContour`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contour {
    /// Simplified vertices `[x, y, z, flags]` (flags: neighbour region, `RC_BORDER_VERTEX`,
    /// `RC_AREA_BORDER`); x and z are relative to the set's `bmin`.
    pub verts: Vec<[i32; 4]>,
    /// Raw vertices, same layout.
    pub rverts: Vec<[i32; 4]>,
    /// Region id.
    pub reg: u16,
    /// Area id.
    pub area: u8,
    /// DayZ part id of the contour's first span.
    pub sub_area: u16,
}

/// `rcContourSet`.
#[derive(Clone, Debug, Default)]
pub struct ContourSet {
    /// The contours; holes that were merged into an outline are gone.
    pub contours: Vec<Contour>,
    /// Minimum bounds, shifted by `border_size * cs` on x and z.
    pub bmin: [f32; 3],
    /// Maximum bounds, shifted by `border_size * cs` on x and z.
    pub bmax: [f32; 3],
    /// Cell size.
    pub cs: f32,
    /// Cell height.
    pub ch: f32,
    /// Width of the set (the field's width without the border).
    pub width: i32,
    /// Height of the set (the field's height without the border).
    pub height: i32,
    /// Border size of the compact field, in cells.
    pub border_size: i32,
    /// The `max_error` the contours were built with.
    pub max_error: f32,
}

fn rd<T: Copy + Default>(s: &[T], i: usize) -> T {
    s.get(i).copied().unwrap_or_default()
}

/// Region code and part id of span `i`: stock packs `reg | area << 16`, the part id travels next to it.
fn code(chf: &CompactHeightfield, i: usize) -> (u32, u16) {
    (
        u32::from(rd(&chf.spans, i).reg) | (u32::from(rd(&chf.areas, i)) << 16),
        rd(&chf.sub_areas, i),
    )
}

/// `getCornerHeight`: the corner height and whether the corner is a tile-border vertex.
fn corner_height(chf: &CompactHeightfield, x: i32, y: i32, i: usize, dir: i32) -> (i32, bool) {
    let mut is_border_vertex = false;
    let dirp = (dir + 1) & 3;
    let mut ch = i32::from(rd(&chf.spans, i).y);
    let mut regs = [0u32; 4];
    let mut subs = [0u16; 4];
    (regs[0], subs[0]) = code(chf, i);

    if let Some((ax, ay, ai)) = chf.neighbor(x, y, i, dir) {
        ch = ch.max(i32::from(rd(&chf.spans, ai).y));
        (regs[1], subs[1]) = code(chf, ai);
        if let Some((_, _, ai2)) = chf.neighbor(ax, ay, ai, dirp) {
            ch = ch.max(i32::from(rd(&chf.spans, ai2).y));
            (regs[2], subs[2]) = code(chf, ai2);
        }
    }
    if let Some((ax, ay, ai)) = chf.neighbor(x, y, i, dirp) {
        ch = ch.max(i32::from(rd(&chf.spans, ai).y));
        (regs[3], subs[3]) = code(chf, ai);
        if let Some((_, _, ai2)) = chf.neighbor(ax, ay, ai, dir) {
            ch = ch.max(i32::from(rd(&chf.spans, ai2).y));
            // Like upstream, this overwrites slot 2.
            (regs[2], subs[2]) = code(chf, ai2);
        }
    }

    // Check if the vertex is a special edge vertex; these vertices are removed later.
    let border = u32::from(BORDER_REG);
    for j in 0..4usize {
        let a = j;
        let b = (j + 1) & 3;
        let c = (j + 2) & 3;
        let d = (j + 3) & 3;
        let (ra, rb, rc, rdd) = (rd(&regs, a), rd(&regs, b), rd(&regs, c), rd(&regs, d));
        // Two same exterior cells in a row, followed by two interior cells and none of the regions
        // out of bounds.
        let two_same_exts = (ra & rb & border) != 0 && ra == rb;
        let two_ints = ((rc | rdd) & border) == 0;
        // DayZ: the interior cells must also have the same part id.
        let ints_same_area = (rc >> 16) == (rdd >> 16) && rd(&subs, c) == rd(&subs, d);
        let no_zeros = ra != 0 && rb != 0 && rc != 0 && rdd != 0;
        if two_same_exts && two_ints && ints_same_area && no_zeros {
            is_border_vertex = true;
            break;
        }
    }
    (ch, is_border_vertex)
}

/// `walkContour`: traces one contour starting at span `i` of column `(x, y)`, clearing the visited
/// edge flags.
fn walk_contour(
    mut x: i32,
    mut y: i32,
    mut i: usize,
    chf: &CompactHeightfield,
    flags: &mut [u8],
    points: &mut Vec<[i32; 4]>,
) {
    let flag = |flags: &[u8], i: usize, dir: i32| rd(flags, i) & (1u8 << dir) != 0;
    // Choose the first non-connected edge.
    let mut dir = 0i32;
    while !flag(flags, i, dir) {
        dir += 1;
        if dir >= 4 {
            return;
        }
    }
    let start_dir = dir;
    let start_i = i;
    let area = rd(&chf.areas, i);
    let sub_area = rd(&chf.sub_areas, i);

    let mut iter = 0;
    loop {
        iter += 1;
        if iter >= MAX_WALK_ITERATIONS {
            break;
        }
        if flag(flags, i, dir) {
            // Choose the edge corner.
            let (py, is_border_vertex) = corner_height(chf, x, y, i, dir);
            let mut px = x;
            let mut pz = y;
            match dir {
                0 => pz += 1,
                1 => {
                    px += 1;
                    pz += 1;
                }
                2 => px += 1,
                _ => {}
            }
            let mut r = 0i32;
            let mut is_area_border = false;
            if let Some((_, _, ai)) = chf.neighbor(x, y, i, dir) {
                r = i32::from(rd(&chf.spans, ai).reg);
                // DayZ: an edge between two part ids is an area border too.
                if area != rd(&chf.areas, ai) || sub_area != rd(&chf.sub_areas, ai) {
                    is_area_border = true;
                }
            }
            if is_border_vertex {
                r |= RC_BORDER_VERTEX;
            }
            if is_area_border {
                r |= RC_AREA_BORDER;
            }
            points.push([px, py, pz, r]);
            if let Some(f) = flags.get_mut(i) {
                *f &= !(1u8 << dir); // Remove visited edges.
            }
            dir = (dir + 1) & 3; // Rotate CW.
        } else {
            let Some((nx, ny, ni)) = chf.neighbor(x, y, i, dir) else {
                // Should not happen.
                return;
            };
            x = nx;
            y = ny;
            i = ni;
            dir = (dir + 3) & 3; // Rotate CCW.
        }
        if start_i == i && start_dir == dir {
            break;
        }
    }
}

/// `distancePtSeg`: the squared xz distance of `(x, z)` to the segment `p`-`q` and the clamped
/// projection parameter `t` it was measured at.
#[expect(
    clippy::manual_clamp,
    reason = "upstream's comparisons, so that a NaN stays a NaN as in C"
)]
fn distance_pt_seg(x: i32, z: i32, p: (i32, i32), q: (i32, i32)) -> (f32, f32) {
    let pqx = i32_to_f32(q.0 - p.0);
    let pqz = i32_to_f32(q.1 - p.1);
    let mut dx = i32_to_f32(x - p.0);
    let mut dz = i32_to_f32(z - p.1);
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    if t < 0.0 {
        t = 0.0;
    } else if t > 1.0 {
        t = 1.0;
    }
    dx = i32_to_f32(p.0) + t * pqx - i32_to_f32(x);
    dz = i32_to_f32(p.1) + t * pqz - i32_to_f32(z);
    (dx * dx + dz * dz, t)
}

/// A simplified vertex under construction: `x`, `y`, `z` and the index of its raw point.
#[derive(Clone, Copy)]
struct SVert {
    x: i32,
    y: i32,
    z: i32,
    i: usize,
}

fn sv(points: &[[i32; 4]], i: usize) -> SVert {
    let p = rd(points, i);
    SVert {
        x: p[0],
        y: p[1],
        z: p[2],
        i,
    }
}

/// `simplifyContour` with the DayZ `max_vert_error`. `points` is the raw contour; the vertical pass
/// tags the raw point it inserts, so it is mutable. Returns the simplified `[x, y, z, flags]`.
#[expect(
    clippy::too_many_lines,
    reason = "a direct port of upstream simplifyContour"
)]
fn simplify_contour(
    points: &mut [[i32; 4]],
    max_error: f32,
    max_vert_error: f32,
    max_edge_len: i32,
    build_flags: i32,
) -> Vec<[i32; 4]> {
    let pn = points.len();
    let mut simplified: Vec<SVert> = Vec::new();
    if pn == 0 {
        return Vec::new();
    }

    // Add initial points.
    let has_connections = points.iter().any(|p| p[3] & RC_CONTOUR_REG_MASK != 0);
    if has_connections {
        // The contour has some portals to other regions: add a new point to every location where
        // the region changes.
        for i in 0..pn {
            let ii = (i + 1) % pn;
            let (a, b) = (rd(points, i)[3], rd(points, ii)[3]);
            let different_regs = (a & RC_CONTOUR_REG_MASK) != (b & RC_CONTOUR_REG_MASK);
            let area_borders = (a & RC_AREA_BORDER) != (b & RC_AREA_BORDER);
            if different_regs || area_borders {
                simplified.push(sv(points, i));
            }
        }
    }
    if simplified.is_empty() {
        // No connections at all: create the initial points from the lower-left and upper-right
        // vertices of the contour.
        let first = sv(points, 0);
        let mut ll = first;
        let mut ur = first;
        for (i, p) in points.iter().enumerate() {
            let (x, z) = (p[0], p[2]);
            if x < ll.x || (x == ll.x && z < ll.z) {
                ll = sv(points, i);
            }
            if x > ur.x || (x == ur.x && z > ur.z) {
                ur = sv(points, i);
            }
        }
        simplified.push(ll);
        simplified.push(ur);
    }

    // Add points until all raw points are within the error tolerance of the simplified shape.
    let mut i = 0;
    while i < simplified.len() {
        let ii = (i + 1) % simplified.len();
        let (mut a, mut b) = (rd_sv(&simplified, i), rd_sv(&simplified, ii));
        let (ai, bi) = (a.i, b.i);

        let mut maxd = 0.0f32;
        let mut maxi: Option<usize> = None;
        // DayZ: the worst vertical deviation, in squared voxels.
        let mut maxdy = 0.0f32;
        let mut maxdyi: Option<usize> = None;
        let (mut ci, cinc, endi);

        // Traverse the segment in lexicographic order so that the maximum deviation is calculated
        // the same way when traversing opposite segments.
        if b.x > a.x || (b.x == a.x && b.z > a.z) {
            cinc = 1;
            ci = (ai + cinc) % pn;
            endi = bi;
        } else {
            cinc = pn - 1;
            ci = (bi + cinc) % pn;
            endi = ai;
            std::mem::swap(&mut a, &mut b);
        }

        // Tessellate only outer edges or edges between areas. DayZ: border vertices count too.
        let gate = rd(points, ci)[3];
        if (gate & RC_CONTOUR_REG_MASK) == 0 || (gate & (RC_AREA_BORDER | RC_BORDER_VERTEX)) != 0 {
            let dy_total = i32_to_f32(b.y - a.y);
            while ci != endi {
                let p = rd(points, ci);
                let (d, t) = distance_pt_seg(p[0], p[2], (a.x, a.z), (b.x, b.z));
                if d > maxd {
                    maxd = d;
                    maxi = Some(ci);
                }
                // DayZ: the height of the segment at the same clamped parameter `t` that
                // `distance_pt_seg` projected the point to (the parameter is the fraction of the way
                // from `a` to `b` in the lexicographic order used for the traversal).
                let dy = i32_to_f32(p[1]) - (i32_to_f32(a.y) + t * dy_total);
                let dy2 = dy * dy;
                if dy2 > maxdy {
                    maxdy = dy2;
                    maxdyi = Some(ci);
                }
                ci = (ci + cinc) % pn;
            }
        }

        // If the maximum deviation is larger than accepted, add a new point, else continue to the
        // next segment.
        let insert = if let Some(m) = maxi.filter(|_| maxd > max_error * max_error) {
            Some(m)
        } else if let Some(m) =
            maxdyi.filter(|_| max_vert_error > 0.0 && maxdy > max_vert_error * max_vert_error)
        {
            // DayZ: the vertically worst point, tagged in the raw contour.
            if let Some(p) = points.get_mut(m) {
                p[3] |= RC_BORDER_VERTEX | RC_AREA_BORDER;
            }
            Some(m)
        } else {
            None
        };
        if let Some(m) = insert {
            simplified.insert(i + 1, sv(points, m));
        } else {
            i += 1;
        }
    }

    // Split too long edges.
    if max_edge_len > 0
        && (build_flags & (RC_CONTOUR_TESS_WALL_EDGES | RC_CONTOUR_TESS_AREA_EDGES)) != 0
    {
        let max_len2 = i64::from(max_edge_len) * i64::from(max_edge_len);
        let mut i = 0;
        while i < simplified.len() {
            let ii = (i + 1) % simplified.len();
            let (a, b) = (rd_sv(&simplified, i), rd_sv(&simplified, ii));
            let mut maxi: Option<usize> = None;
            let ci = (a.i + 1) % pn;
            let flag = rd(points, ci)[3];

            // Tessellate only outer edges or edges between areas.
            let mut tess = false;
            // Wall edges.
            if (build_flags & RC_CONTOUR_TESS_WALL_EDGES) != 0 && (flag & RC_CONTOUR_REG_MASK) == 0
            {
                tess = true;
            }
            // Edges between areas.
            if (build_flags & RC_CONTOUR_TESS_AREA_EDGES) != 0 && (flag & RC_AREA_BORDER) != 0 {
                tess = true;
            }
            if tess {
                let dx = i64::from(b.x) - i64::from(a.x);
                let dz = i64::from(b.z) - i64::from(a.z);
                if dx * dx + dz * dz > max_len2 {
                    // Round based on the segments in lexicographic order so that the maximum
                    // tessellation is consistent whichever direction a segment is traversed.
                    let n = if b.i < a.i { b.i + pn - a.i } else { b.i - a.i };
                    if n > 1 {
                        maxi = if b.x > a.x || (b.x == a.x && b.z > a.z) {
                            Some((a.i + n / 2) % pn)
                        } else {
                            Some((a.i + n.div_ceil(2)) % pn)
                        };
                    }
                }
            }
            if let Some(m) = maxi {
                simplified.insert(i + 1, sv(points, m));
            } else {
                i += 1;
            }
        }
    }

    simplified
        .iter()
        .map(|v| {
            // The edge vertex flag is taken from the current raw point, and the neighbour region
            // from the next raw point.
            let ai = (v.i + 1) % pn;
            let flags = (rd(points, ai)[3] & (RC_CONTOUR_REG_MASK | RC_AREA_BORDER))
                | (rd(points, v.i)[3] & RC_BORDER_VERTEX);
            [v.x, v.y, v.z, flags]
        })
        .collect()
}

fn rd_sv(s: &[SVert], i: usize) -> SVert {
    s.get(i).copied().unwrap_or(SVert {
        x: 0,
        y: 0,
        z: 0,
        i: 0,
    })
}

/// `calcAreaOfPolygon2D`.
fn area_of_polygon_2d(verts: &[[i32; 4]]) -> i64 {
    let mut area = 0i64;
    let mut j = verts.len().saturating_sub(1);
    for (i, vi) in verts.iter().enumerate() {
        let vj = rd(verts, j);
        area += i64::from(vi[0]) * i64::from(vj[2]) - i64::from(vj[0]) * i64::from(vi[2]);
        j = i;
    }
    (area + 1) / 2
}

fn prev(i: usize, n: usize) -> usize {
    if i > 0 { i - 1 } else { n.saturating_sub(1) }
}

fn next(i: usize, n: usize) -> usize {
    if i + 1 < n { i + 1 } else { 0 }
}

fn area2(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> i64 {
    (i64::from(b[0]) - i64::from(a[0])) * (i64::from(c[2]) - i64::from(a[2]))
        - (i64::from(c[0]) - i64::from(a[0])) * (i64::from(b[2]) - i64::from(a[2]))
}

/// True iff `c` is strictly to the left of the directed line through `a` to `b`.
fn left(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    area2(a, b, c) < 0
}

fn left_on(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    area2(a, b, c) <= 0
}

fn collinear(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    area2(a, b, c) == 0
}

/// True iff `ab` properly intersects `cd`.
fn intersect_prop(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4], d: &[i32; 4]) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    (left(a, b, c) != left(a, b, d)) && (left(c, d, a) != left(c, d, b))
}

/// True iff `a`, `b`, `c` are collinear and `c` lies on the closed segment `ab`.
fn between(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    // If ab is not vertical, check betweenness on x; else on z.
    if a[0] == b[0] {
        (a[2] <= c[2] && c[2] <= b[2]) || (a[2] >= c[2] && c[2] >= b[2])
    } else {
        (a[0] <= c[0] && c[0] <= b[0]) || (a[0] >= c[0] && c[0] >= b[0])
    }
}

/// True iff the segments `ab` and `cd` intersect, properly or improperly.
fn intersect(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4], d: &[i32; 4]) -> bool {
    intersect_prop(a, b, c, d)
        || between(a, b, c)
        || between(a, b, d)
        || between(c, d, a)
        || between(c, d, b)
}

fn vequal(a: &[i32; 4], b: &[i32; 4]) -> bool {
    a[0] == b[0] && a[2] == b[2]
}

fn intersect_seg_contour(
    d0: &[i32; 4],
    d1: &[i32; 4],
    skip: Option<usize>,
    verts: &[[i32; 4]],
) -> bool {
    let n = verts.len();
    // For each edge (k, k+1) of P.
    for (k, p0) in verts.iter().enumerate() {
        let k1 = next(k, n);
        // Skip edges incident to the skipped vertex.
        if skip == Some(k) || skip == Some(k1) {
            continue;
        }
        let p1 = rd(verts, k1);
        if vequal(d0, p0) || vequal(d1, p0) || vequal(d0, &p1) || vequal(d1, &p1) {
            continue;
        }
        if intersect(d0, d1, p0, &p1) {
            return true;
        }
    }
    false
}

fn in_cone(i: usize, verts: &[[i32; 4]], pj: &[i32; 4]) -> bool {
    let n = verts.len();
    let pi = rd(verts, i);
    let pi1 = rd(verts, next(i, n));
    let pin1 = rd(verts, prev(i, n));
    // If P[i] is a convex vertex [ i+1 left or on (i-1,i) ].
    if left_on(&pin1, &pi, &pi1) {
        return left(&pi, pj, &pin1) && left(pj, &pi, &pi1);
    }
    // Assume (i-1,i,i+1) not collinear; else P[i] is reflex.
    !(left_on(&pi, pj, &pi1) && left_on(pj, &pi, &pin1))
}

/// `removeDegenerateSegments`: removes adjacent vertices that are equal on the xz-plane.
fn remove_degenerate_segments(simplified: &mut Vec<[i32; 4]>) {
    let mut npts = simplified.len();
    let mut i = 0;
    while i < npts {
        let ni = next(i, npts);
        if vequal(&rd(simplified, i), &rd(simplified, ni)) {
            // Degenerate segment, remove. Upstream's loop moves on to the next index as well.
            simplified.remove(i);
            npts -= 1;
        }
        i += 1;
    }
}

/// `mergeContours`: the outline `ca` and the hole `cb`, joined at `ia` and `ib`.
fn merge_contours(ca: &[[i32; 4]], cb: &[[i32; 4]], ia: usize, ib: usize) -> Vec<[i32; 4]> {
    let mut verts = Vec::with_capacity(ca.len() + cb.len() + 2);
    for i in 0..=ca.len() {
        verts.push(rd(ca, (ia + i) % ca.len().max(1)));
    }
    for i in 0..=cb.len() {
        verts.push(rd(cb, (ib + i) % cb.len().max(1)));
    }
    verts
}

/// `rcContourHole`, with the hole's vertices taken out of its contour.
struct Hole {
    contour: usize,
    verts: Vec<[i32; 4]>,
    minx: i32,
    minz: i32,
    leftmost: usize,
    merged: bool,
}

/// `findLeftMostVertex`: the lowest leftmost vertex of a contour.
fn find_left_most_vertex(verts: &[[i32; 4]]) -> (i32, i32, usize) {
    let first = rd(verts, 0);
    let (mut minx, mut minz, mut leftmost) = (first[0], first[2], 0);
    for (i, v) in verts.iter().enumerate().skip(1) {
        if v[0] < minx || (v[0] == minx && v[2] < minz) {
            minx = v[0];
            minz = v[2];
            leftmost = i;
        }
    }
    (minx, minz, leftmost)
}

/// `mergeRegionHoles`: merges the holes of one region into its outline, one by one. A hole for
/// which no merge point is found stays a separate contour (upstream logs and continues).
fn merge_region_holes(outline: &mut Vec<[i32; 4]>, holes: &mut [Hole]) {
    // Sort holes from left to right (`compareHoles`) with the order of the MSVC `qsort` for equal keys.
    for h in holes.iter_mut() {
        (h.minx, h.minz, h.leftmost) = find_left_most_vertex(&h.verts);
    }
    msvc_qsort(holes, |a, b| (a.minx, a.minz).cmp(&(b.minx, b.minz)));

    // Each merge adds the hole's vertices plus two (the bridge), so this bounds the final outline.
    let max_verts =
        outline.len() + holes.iter().map(|h| h.verts.len()).sum::<usize>() + 2 * holes.len();
    // Upstream keeps one scratch array of `(vert, dist)` for the whole region and its diagonal test
    // reads entry `i` (the hole number, not the candidate number) of it, which can be a leftover of
    // an earlier hole. The buffer is zero filled here, where upstream's is uninitialised.
    let mut diags: Vec<(usize, i64)> = vec![(0, 0); max_verts];

    // Merge holes into the outline one by one.
    for i in 0..holes.len() {
        let hole_len = rd_hole_len(holes, i);
        let mut index: Option<usize> = None;
        let mut best_vertex = holes.get(i).map_or(0, |h| h.leftmost);
        for _ in 0..hole_len {
            // Find potential diagonals. The 'best' vertex must be in the cone described by 3
            // consecutive vertices of the outline.
            let corner = holes.get(i).map_or([0; 4], |h| rd(&h.verts, best_vertex));
            let mut ndiags = 0;
            for j in 0..outline.len() {
                if in_cone(j, outline, &corner) {
                    let dx = i64::from(rd(outline, j)[0]) - i64::from(corner[0]);
                    let dz = i64::from(rd(outline, j)[2]) - i64::from(corner[2]);
                    if let Some(slot) = diags.get_mut(ndiags) {
                        *slot = (j, dx * dx + dz * dz);
                    }
                    ndiags += 1;
                }
            }
            // Sort the potential diagonals by distance: the connection should be as short as
            // possible (`compareDiagDist`, the order of the MSVC `qsort` for equal distances).
            if let Some(found) = diags.get_mut(..ndiags) {
                msvc_qsort(found, |a, b| a.1.cmp(&b.1));
            }

            // Find a diagonal that intersects neither the outline nor the remaining holes.
            index = None;
            for j in 0..ndiags {
                let pt = rd(outline, rd(&diags, j).0);
                let mut hit = intersect_seg_contour(&pt, &corner, Some(rd(&diags, i).0), outline);
                for h in holes.iter().skip(i) {
                    if hit {
                        break;
                    }
                    hit |= intersect_seg_contour(&pt, &corner, None, &h.verts);
                }
                if !hit {
                    index = Some(rd(&diags, j).0);
                    break;
                }
            }
            // A non-intersecting diagonal ends the search.
            if index.is_some() {
                break;
            }
            // All the potential diagonals for this vertex were intersecting: try the next vertex.
            best_vertex = (best_vertex + 1) % hole_len.max(1);
        }

        let Some(index) = index else {
            // Failed to find merge points; upstream logs a warning and continues. The library does
            // not print: the hole stays a contour of its own.
            continue;
        };
        if let Some(h) = holes.get_mut(i) {
            *outline = merge_contours(outline, &h.verts, index, best_vertex);
            h.verts.clear();
            h.merged = true;
        }
    }
}

fn rd_hole_len(holes: &[Hole], i: usize) -> usize {
    holes.get(i).map_or(0, |h| h.verts.len())
}

/// `rcBuildContours`: builds the contours of all regions of `chf`.
///
/// `max_error` is the horizontal simplification error in cells, `max_vert_error` the vertical one
/// in voxels (`0` turns it off), `max_edge_len` the longest edge in cells (`0` = off) and `flags` the
/// `RC_CONTOUR_TESS_*` flags. When the field has a border, the set's `bmin`/`bmax` and the vertices
/// are shifted by it and its width and height shrink by it.
///
/// # Errors
/// Returns an error when the arrays of the field disagree, the border is negative or larger than the
/// field, or a contour names a region the field does not have.
pub fn build_contours(
    chf: &CompactHeightfield,
    max_error: f32,
    max_vert_error: f32,
    max_edge_len: i32,
    flags: i32,
) -> io::Result<ContourSet> {
    let n = chf.spans.len();
    if chf.areas.len() != n || chf.sub_areas.len() != n {
        return Err(error!("compact heightfield areas do not match its spans"));
    }
    let (w, h) = (chf.width, chf.height);
    let border = chf.border_size;
    let border2 = border
        .checked_mul(2)
        .filter(|_| border >= 0)
        .ok_or_else(|| error!("invalid border size {border}"))?;
    if w < border2 || h < border2 {
        return Err(error!("border size {border} is larger than the field"));
    }

    let mut cset = ContourSet {
        contours: Vec::new(),
        bmin: chf.bmin,
        bmax: chf.bmax,
        cs: chf.cs,
        ch: chf.ch,
        width: w - border2,
        height: h - border2,
        border_size: border,
        max_error,
    };
    if border > 0 {
        // If the heightfield was built with a border size, remove the offset.
        let pad = i32_to_f32(border) * chf.cs;
        cset.bmin[0] += pad;
        cset.bmin[2] += pad;
        cset.bmax[0] -= pad;
        cset.bmax[2] -= pad;
    }

    let mut edge_flags = vec![0u8; n];

    // Mark boundaries.
    for y in 0..h {
        for x in 0..w {
            for i in chf.column_range(x, y) {
                let reg = rd(&chf.spans, i).reg;
                if reg == 0 || reg & BORDER_REG != 0 {
                    continue; // The flag stays 0.
                }
                let mut res = 0u8;
                for dir in 0..4 {
                    let r = chf
                        .neighbor(x, y, i, dir)
                        .map_or(0, |(_, _, ai)| rd(&chf.spans, ai).reg);
                    if r == reg {
                        res |= 1 << dir;
                    }
                }
                if let Some(f) = edge_flags.get_mut(i) {
                    *f = res ^ 0xf; // Inverse: mark non-connected edges.
                }
            }
        }
    }

    let mut verts: Vec<[i32; 4]> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            for i in chf.column_range(x, y) {
                let f = rd(&edge_flags, i);
                if f == 0 || f == 0xf {
                    if let Some(slot) = edge_flags.get_mut(i) {
                        *slot = 0;
                    }
                    continue;
                }
                let reg = rd(&chf.spans, i).reg;
                if reg == 0 || reg & BORDER_REG != 0 {
                    continue;
                }
                verts.clear();
                walk_contour(x, y, i, chf, &mut edge_flags, &mut verts);
                let mut simplified =
                    simplify_contour(&mut verts, max_error, max_vert_error, max_edge_len, flags);
                remove_degenerate_segments(&mut simplified);

                if simplified.len() >= 3 {
                    let mut contour = Contour {
                        verts: simplified,
                        rverts: verts.clone(),
                        reg,
                        area: rd(&chf.areas, i),
                        sub_area: rd(&chf.sub_areas, i),
                    };
                    if border > 0 {
                        // If the heightfield was built with a border size, remove the offset.
                        for v in contour.verts.iter_mut().chain(contour.rverts.iter_mut()) {
                            v[0] -= border;
                            v[2] -= border;
                        }
                    }
                    cset.contours.push(contour);
                }
            }
        }
    }

    merge_holes(&mut cset, usize::from(chf.max_regions) + 1)?;
    Ok(cset)
}

/// The hole-merging part of `rcBuildContours`: outlines wind positively, holes negatively.
fn merge_holes(cset: &mut ContourSet, nregions: usize) -> io::Result<()> {
    let winding: Vec<bool> = cset
        .contours
        .iter()
        .map(|c| area_of_polygon_2d(&c.verts) < 0)
        .collect();
    if !winding.iter().any(|&hole| hole) {
        return Ok(());
    }

    // Collect the outline and the holes per region. We assume one outline and multiple holes.
    let mut outlines: Vec<Option<usize>> = vec![None; nregions];
    let mut region_holes: Vec<Vec<usize>> = vec![Vec::new(); nregions];
    for (ci, (c, &hole)) in cset.contours.iter().zip(&winding).enumerate() {
        let r = usize::from(c.reg);
        let (Some(outline), Some(holes)) = (outlines.get_mut(r), region_holes.get_mut(r)) else {
            return Err(error!(
                "contour of region {r} but the field has {nregions} regions"
            ));
        };
        if hole {
            holes.push(ci);
        } else {
            // Upstream logs "Multiple outlines for region"; the last outline wins.
            *outline = Some(ci);
        }
    }

    // Finally merge each region's holes into its outline.
    for (r, hole_ids) in region_holes.iter().enumerate() {
        if hole_ids.is_empty() {
            continue;
        }
        // A region without an outline (self-overlapping contour after too aggressive
        // simplification): upstream logs an error and leaves the holes alone.
        let Some(Some(oi)) = outlines.get(r) else {
            continue;
        };
        let mut holes: Vec<Hole> = hole_ids
            .iter()
            .map(|&ci| Hole {
                contour: ci,
                verts: cset
                    .contours
                    .get_mut(ci)
                    .map(|c| std::mem::take(&mut c.verts))
                    .unwrap_or_default(),
                minx: 0,
                minz: 0,
                leftmost: 0,
                merged: false,
            })
            .collect();
        let mut outline = cset
            .contours
            .get_mut(*oi)
            .map(|c| std::mem::take(&mut c.verts))
            .unwrap_or_default();
        merge_region_holes(&mut outline, &mut holes);
        if let Some(c) = cset.contours.get_mut(*oi) {
            c.verts = outline;
        }
        for h in holes {
            if let (false, Some(c)) = (h.merged, cset.contours.get_mut(h.contour)) {
                c.verts = h.verts; // Not merged: the hole stays as it was.
            }
        }
    }
    // The merged holes are empty now; upstream leaves them in the set with zero vertices.
    cset.contours.retain(|c| !c.verts.is_empty());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Hole, merge_region_holes};

    fn pt(x: i32, z: i32) -> [i32; 4] {
        [x, 0, z, 0]
    }

    /// A hole whose leftmost vertex is the origin, as the merge expects.
    fn hole(verts: Vec<[i32; 4]>) -> Hole {
        Hole {
            contour: 0,
            verts,
            minx: 0,
            minz: 0,
            leftmost: 0,
            merged: false,
        }
    }

    #[test]
    fn equal_diagonals_take_the_msvc_qsort_order() {
        // Eight outline vertices on a circle of radius 5, all at the squared distance 25 from the hole's
        // leftmost vertex at the origin, and all of them in its cone. The sort of eight equal
        // candidates is the short sort, which moves the first one to the end (R18): the bridge goes to
        // the second vertex, not to the first, which the stable sort took. Both diagonals are free.
        let mut outline = vec![
            pt(3, -4),
            pt(0, -5),
            pt(-3, -4),
            pt(-5, 0),
            pt(-3, 4),
            pt(0, 5),
            pt(3, 4),
            pt(5, 0),
        ];
        let mut holes = vec![hole(vec![pt(0, 0), pt(1, -1), pt(1, 1)])];
        merge_region_holes(&mut outline, &mut holes);
        assert!(holes[0].merged);
        assert_eq!(outline.len(), 8 + 3 + 2);
        assert_eq!(outline[0], pt(0, -5));
    }

    #[test]
    fn many_equal_diagonals_take_the_partition_order() {
        // Fourteen outline vertices at the squared distance 2500 or 2501 from the hole's leftmost vertex:
        // more than eight candidates, so the sort partitions, and it moves equal candidates (R18). The
        // bridge goes to (30, -40); the stable sort took (-48, 14).
        let ring = [
            (-50, 1),
            (-48, 14),
            (-40, 30),
            (0, 50),
            (48, 14),
            (50, -1),
            (48, -14),
            (30, -40),
            (14, -48),
            (1, -50),
            (-30, -40),
            (-40, -30),
            (-48, -14),
            (-50, -1),
        ];
        let mut outline: Vec<[i32; 4]> = ring.iter().map(|&(x, z)| pt(x, z)).collect();
        let mut holes = vec![hole(vec![pt(0, 0), pt(10, -10), pt(10, 10)])];
        merge_region_holes(&mut outline, &mut holes);
        assert!(holes[0].merged);
        assert_eq!(outline[0], pt(30, -40));
    }

    #[test]
    fn holes_with_equal_keys_are_merged_in_the_msvc_qsort_order() {
        // Three holes with the same leftmost vertex: the short sort of three equal keys gives the
        // order 1, 2, 0 (R18), so the hole with the 3s is merged first and the one with the 2s last;
        // the stable sort merged them in the order 0, 1, 2.
        let mut outline = vec![
            pt(30, -40),
            pt(0, -50),
            pt(-30, -40),
            pt(-50, 0),
            pt(-30, 40),
            pt(0, 50),
            pt(30, 40),
            pt(50, 0),
        ];
        let mut holes = vec![
            hole(vec![pt(0, 0), pt(2, -1), pt(2, 1)]),
            hole(vec![pt(0, 0), pt(3, -1), pt(3, 1)]),
            hole(vec![pt(0, 0), pt(4, -1), pt(4, 1)]),
        ];
        merge_region_holes(&mut outline, &mut holes);
        assert!(holes.iter().all(|h| h.merged));
        let tails: Vec<i32> = outline
            .iter()
            .filter(|v| (2..10).contains(&v[0]))
            .map(|v| v[0])
            .collect();
        assert_eq!(tails, vec![3, 3, 4, 4, 2, 2]);
    }
}
