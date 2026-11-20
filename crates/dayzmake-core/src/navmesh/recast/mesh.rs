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

//! `rcBuildPolyMesh` with the DayZ changes (spec 5.4 step 10, 10.1 item 8): ear-clipping
//! triangulation of the contours, merging of the triangles into convex polygons of at most `nvp`
//! vertices, mesh adjacency and the portal edges of a tile with a border.
//!
//! DayZ never removes border vertices: stock Recast flags the vertices that lie on the tile border
//! (`vflags`) and removes them again, DayZ does not set the flags, so the "remove edge vertices"
//! loop does nothing. The removal code is ported anyway, so that
//! [`build_poly_mesh_with_border_removal`] can show what stock would do.

use std::io;

use super::contour::{Contour, ContourSet, RC_BORDER_VERTEX};

/// `RC_MESH_NULL_IDX`.
const NULL_IDX: u16 = 0xffff;
/// `RC_MULTIPLE_REGS`.
const MULTIPLE_REGS: u16 = 0;
/// `VERTEX_BUCKET_COUNT`.
const VERTEX_BUCKET_COUNT: usize = 1 << 12;
/// The most vertices per polygon that are accepted (stock's `MAX_VERTS_PER_POLY` is 6 for Detour).
const MAX_NVP: usize = 12;

/// `rcPolyMesh`: the polygons of one tile in cell coordinates.
///
/// `polys` holds `nvp * 2` values per polygon: the vertex indices (padded with `0xffff`) followed
/// by the neighbour of each edge (`0xffff` for none, `0x8000 | dir` for a portal to the tile in
/// direction `dir`: 0 x-, 1 z+, 2 x+, 3 z-).
#[derive(Clone, Debug, Default)]
pub struct PolyMesh {
    /// Vertices `[x, y, z]` in cells, relative to `bmin`.
    pub verts: Vec<[u16; 3]>,
    /// The polygons, `nvp * 2` values each.
    pub polys: Vec<u16>,
    /// Region id of each polygon.
    pub regs: Vec<u16>,
    /// Flags of each polygon, all zero (the caller sets them).
    pub flags: Vec<u16>,
    /// Area id of each polygon.
    pub areas: Vec<u8>,
    /// DayZ part id of each polygon.
    pub sub_areas: Vec<u16>,
    /// Maximum vertices per polygon.
    pub nvp: usize,
    /// Minimum bounds.
    pub bmin: [f32; 3],
    /// Maximum bounds.
    pub bmax: [f32; 3],
    /// Cell size.
    pub cs: f32,
    /// Cell height.
    pub ch: f32,
    /// Border size in cells.
    pub border_size: i32,
    /// `maxEdgeError`: the contour set's `max_error`.
    pub max_edge_error: f32,
}

fn rd<T: Copy + Default>(s: &[T], i: usize) -> T {
    s.get(i).copied().unwrap_or_default()
}

/// `s[dst] = s[src]`.
fn mv<T: Copy + Default>(s: &mut [T], dst: usize, src: usize) {
    let v = rd(s, src);
    wr(s, dst, v);
}

fn wr<T>(s: &mut [T], i: usize, v: T) {
    if let Some(slot) = s.get_mut(i) {
        *slot = v;
    }
}

/// `s[from..from + len]`, or an empty slice when out of range.
fn sub<T>(s: &[T], from: usize, len: usize) -> &[T] {
    from.checked_add(len)
        .and_then(|end| s.get(from..end))
        .unwrap_or(&[])
}

/// C `(unsigned short)v`: the low 16 bits.
fn low_u16(v: i32) -> u16 {
    u16::try_from(v & 0xffff).unwrap_or_default()
}

/// A vertex index as `u16`; indices are below `0xffff` by the vertex limit.
fn idx_u16(i: usize) -> u16 {
    u16::try_from(i).unwrap_or(NULL_IDX)
}

fn md(a: usize, n: usize) -> usize {
    a.checked_rem(n).unwrap_or(0)
}

fn next(i: usize, n: usize) -> usize {
    if i + 1 < n { i + 1 } else { 0 }
}

fn prev(i: usize, n: usize) -> usize {
    if i >= 1 { i - 1 } else { n.saturating_sub(1) }
}

/// The x and z of a contour vertex.
type P = (i64, i64);

fn area2(a: P, b: P, c: P) -> i64 {
    (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)
}

fn xorb(x: bool, y: bool) -> bool {
    x != y
}

fn left(a: P, b: P, c: P) -> bool {
    area2(a, b, c) < 0
}

fn left_on(a: P, b: P, c: P) -> bool {
    area2(a, b, c) <= 0
}

fn collinear(a: P, b: P, c: P) -> bool {
    area2(a, b, c) == 0
}

fn intersect_prop(a: P, b: P, c: P, d: P) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    xorb(left(a, b, c), left(a, b, d)) && xorb(left(c, d, a), left(c, d, b))
}

fn between(a: P, b: P, c: P) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a.0 == b.0 {
        (a.1 <= c.1 && c.1 <= b.1) || (a.1 >= c.1 && c.1 >= b.1)
    } else {
        (a.0 <= c.0 && c.0 <= b.0) || (a.0 >= c.0 && c.0 >= b.0)
    }
}

fn intersect(a: P, b: P, c: P, d: P) -> bool {
    intersect_prop(a, b, c, d)
        || between(a, b, c)
        || between(a, b, d)
        || between(c, d, a)
        || between(c, d, b)
}

/// The ring being triangulated: the points and the current order of their indices.
struct Ring<'a> {
    pts: &'a [P],
    ix: &'a [usize],
}

impl Ring<'_> {
    fn n(&self) -> usize {
        self.ix.len()
    }

    fn p(&self, k: usize) -> P {
        rd(self.pts, rd(self.ix, k))
    }

    /// `diagonalie` (`strict`) and `diagonalieLoose`.
    fn diagonalie(&self, i: usize, j: usize, strict: bool) -> bool {
        let n = self.n();
        let (d0, d1) = (self.p(i), self.p(j));
        for k in 0..n {
            let k1 = next(k, n);
            if !(k == i || k1 == i || k == j || k1 == j) {
                let (p0, p1) = (self.p(k), self.p(k1));
                if d0 == p0 || d1 == p0 || d0 == p1 || d1 == p1 {
                    continue;
                }
                let hit = if strict {
                    intersect(d0, d1, p0, p1)
                } else {
                    intersect_prop(d0, d1, p0, p1)
                };
                if hit {
                    return false;
                }
            }
        }
        true
    }

    /// `inCone` (`strict`) and `inConeLoose`.
    fn in_cone(&self, i: usize, j: usize, strict: bool) -> bool {
        let n = self.n();
        let (pi, pj) = (self.p(i), self.p(j));
        let pi1 = self.p(next(i, n));
        let pprev = self.p(prev(i, n));
        if left_on(pprev, pi, pi1) {
            return if strict {
                left(pi, pj, pprev) && left(pj, pi, pi1)
            } else {
                left_on(pi, pj, pprev) && left_on(pj, pi, pi1)
            };
        }
        !(left_on(pi, pj, pi1) && left_on(pj, pi, pprev))
    }

    fn diagonal(&self, i: usize, j: usize) -> bool {
        self.in_cone(i, j, true) && self.diagonalie(i, j, true)
    }

    fn diagonal_loose(&self, i: usize, j: usize) -> bool {
        self.in_cone(i, j, false) && self.diagonalie(i, j, false)
    }
}

/// `triangulate`: ear clipping. Returns the triangles (indices into `pts`) and whether the
/// contour could be triangulated completely. On failure the triangles found so far are still
/// returned: stock logs "Bad triangulation" and carries on with them.
fn triangulate(pts: &[P]) -> (Vec<[usize; 3]>, bool) {
    let mut ix: Vec<usize> = (0..pts.len()).collect();
    let mut can: Vec<bool> = vec![false; pts.len()];
    let mut tris: Vec<[usize; 3]> = Vec::new();
    if pts.len() < 3 {
        return (tris, false);
    }
    let n0 = ix.len();
    for i in 0..n0 {
        let i1 = next(i, n0);
        let i2 = next(i1, n0);
        if (Ring { pts, ix: &ix }).diagonal(i, i2) {
            wr(&mut can, i1, true);
        }
    }

    while ix.len() > 3 {
        let n = ix.len();
        let mut min_len = -1_i64;
        let mut mini: Option<usize> = None;
        {
            let ring = Ring { pts, ix: &ix };
            for i in 0..n {
                let i1 = next(i, n);
                if rd(&can, i1) {
                    let (p0, p2) = (ring.p(i), ring.p(next(i1, n)));
                    let (dx, dy) = (p2.0 - p0.0, p2.1 - p0.1);
                    let len = dx * dx + dy * dy;
                    if min_len < 0 || len < min_len {
                        min_len = len;
                        mini = Some(i);
                    }
                }
            }
            if mini.is_none() {
                // Recover from overlapping segments by loosening the cone test.
                min_len = -1;
                for i in 0..n {
                    let i1 = next(i, n);
                    let i2 = next(i1, n);
                    if ring.diagonal_loose(i, i2) {
                        let (p0, p2) = (ring.p(i), ring.p(next(i2, n)));
                        let (dx, dy) = (p2.0 - p0.0, p2.1 - p0.1);
                        let len = dx * dx + dy * dy;
                        if min_len < 0 || len < min_len {
                            min_len = len;
                            mini = Some(i);
                        }
                    }
                }
            }
        }
        let Some(i) = mini else {
            return (tris, false);
        };
        let mut i1 = next(i, n);
        let i2 = next(i1, n);
        tris.push([rd(&ix, i), rd(&ix, i1), rd(&ix, i2)]);

        // Remove P[i1].
        ix.remove(i1);
        can.remove(i1);
        let n = ix.len();
        if i1 >= n {
            i1 = 0;
        }
        let i = prev(i1, n);
        let ring = Ring { pts, ix: &ix };
        let d = ring.diagonal(prev(i, n), i1);
        wr(&mut can, i, d);
        let d = ring.diagonal(i, next(i1, n));
        wr(&mut can, i1, d);
    }
    tris.push([rd(&ix, 0), rd(&ix, 1), rd(&ix, 2)]);
    (tris, true)
}

/// `countPolyVerts`.
fn count_poly_verts(p: &[u16], nvp: usize) -> usize {
    for (i, &v) in p.iter().take(nvp).enumerate() {
        if v == NULL_IDX {
            return i;
        }
    }
    nvp.min(p.len())
}

fn vpt(verts: &[[u16; 3]], i: u16) -> [i64; 3] {
    let v = rd(verts, usize::from(i));
    [i64::from(v[0]), i64::from(v[1]), i64::from(v[2])]
}

/// `uleft`.
fn uleft(a: [i64; 3], b: [i64; 3], c: [i64; 3]) -> bool {
    (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]) < 0
}

/// `getPolyMergeValue`: the squared length of the shared edge, or -1 when the polygons cannot be
/// merged; also the shared edge of each polygon.
fn poly_merge_value(pa: &[u16], pb: &[u16], verts: &[[u16; 3]], nvp: usize) -> (i64, usize, usize) {
    let na = count_poly_verts(pa, nvp);
    let nb = count_poly_verts(pb, nvp);
    if na + nb < 2 || na + nb - 2 > nvp {
        return (-1, 0, 0);
    }
    let mut ea: Option<usize> = None;
    let mut eb: Option<usize> = None;
    for i in 0..na {
        let mut va0 = rd(pa, i);
        let mut va1 = rd(pa, md(i + 1, na));
        if va0 > va1 {
            std::mem::swap(&mut va0, &mut va1);
        }
        for j in 0..nb {
            let mut vb0 = rd(pb, j);
            let mut vb1 = rd(pb, md(j + 1, nb));
            if vb0 > vb1 {
                std::mem::swap(&mut vb0, &mut vb1);
            }
            if va0 == vb0 && va1 == vb1 {
                ea = Some(i);
                eb = Some(j);
                break;
            }
        }
    }
    let (Some(ea), Some(eb)) = (ea, eb) else {
        return (-1, 0, 0);
    };

    // The merged polygon must be convex.
    let va = rd(pa, md(ea + na - 1, na));
    let vb = rd(pa, ea);
    let vc = rd(pb, md(eb + 2, nb));
    if !uleft(vpt(verts, va), vpt(verts, vb), vpt(verts, vc)) {
        return (-1, 0, 0);
    }
    let va = rd(pb, md(eb + nb - 1, nb));
    let vb = rd(pb, eb);
    let vc = rd(pa, md(ea + 2, na));
    if !uleft(vpt(verts, va), vpt(verts, vb), vpt(verts, vc)) {
        return (-1, 0, 0);
    }

    let a = vpt(verts, rd(pa, ea));
    let b = vpt(verts, rd(pa, md(ea + 1, na)));
    let dx = a[0] - b[0];
    let dy = a[2] - b[2];
    (dx * dx + dy * dy, ea, eb)
}

/// `mergePolyVerts`: the merged polygon, `nvp` long.
fn merge_poly_verts(pa: &[u16], pb: &[u16], ea: usize, eb: usize, nvp: usize) -> Vec<u16> {
    let na = count_poly_verts(pa, nvp);
    let nb = count_poly_verts(pb, nvp);
    let mut tmp = vec![NULL_IDX; nvp];
    let mut n = 0;
    for i in 0..na.saturating_sub(1) {
        wr(&mut tmp, n, rd(pa, md(ea + 1 + i, na)));
        n += 1;
    }
    for i in 0..nb.saturating_sub(1) {
        wr(&mut tmp, n, rd(pb, md(eb + 1 + i, nb)));
        n += 1;
    }
    tmp
}

/// Merges the triangles in `polys` (`nvp` values each, `npolys` of them) greedily, the longest
/// shared edge first. `on_merge(a, b, last)` is called for every merge (`a` and `b` are the merged
/// polygons, `last` the index of the last polygon, which then replaces `b`), before that happens.
/// Returns the new polygon count.
fn merge_polys(
    polys: &mut [u16],
    mut npolys: usize,
    verts: &[[u16; 3]],
    nvp: usize,
    on_merge: &mut dyn FnMut(usize, usize, usize),
) -> usize {
    loop {
        let mut best_val = 0_i64;
        let (mut best_a, mut best_b, mut edge_a, mut edge_b) = (0, 0, 0, 0);
        for j in 0..npolys.saturating_sub(1) {
            for k in j + 1..npolys {
                let (v, ea, eb) = poly_merge_value(
                    sub(polys, j * nvp, nvp),
                    sub(polys, k * nvp, nvp),
                    verts,
                    nvp,
                );
                if v > best_val {
                    best_val = v;
                    best_a = j;
                    best_b = k;
                    edge_a = ea;
                    edge_b = eb;
                }
            }
        }
        if best_val <= 0 {
            return npolys;
        }
        let merged = merge_poly_verts(
            sub(polys, best_a * nvp, nvp),
            sub(polys, best_b * nvp, nvp),
            edge_a,
            edge_b,
            nvp,
        );
        for (k, v) in merged.into_iter().enumerate() {
            wr(polys, best_a * nvp + k, v);
        }
        let last = npolys.saturating_sub(1);
        on_merge(best_a, best_b, last);
        if best_b != last {
            for k in 0..nvp {
                mv(polys, best_b * nvp + k, last * nvp + k);
            }
        }
        npolys -= 1;
    }
}

/// The mesh while it is built (`maxTris` polygons are allocated up front, as in stock).
struct Work {
    nvp: usize,
    verts: Vec<[u16; 3]>,
    polys: Vec<u16>,
    regs: Vec<u16>,
    areas: Vec<u8>,
    sub_areas: Vec<u16>,
    npolys: usize,
}

impl Work {
    fn poly(&self, i: usize) -> &[u16] {
        sub(&self.polys, i * self.nvp * 2, self.nvp * 2)
    }

    fn poly_len(&self, i: usize) -> usize {
        count_poly_verts(self.poly(i), self.nvp)
    }

    /// `canRemoveVertex`.
    fn can_remove_vertex(&self, rem: u16) -> bool {
        let mut touched = 0_usize;
        let mut remaining_edges = 0_i64;
        for i in 0..self.npolys {
            let p = self.poly(i);
            let nv = self.poly_len(i);
            let mut removed = 0_i64;
            for &v in p.iter().take(nv) {
                if v == rem {
                    touched += 1;
                    removed += 1;
                }
            }
            if removed != 0 {
                remaining_edges += i64::try_from(nv).unwrap_or(0) - (removed + 1);
            }
        }
        if remaining_edges <= 2 {
            return false;
        }
        // Edges that share the removed vertex: (a, b, share count).
        let mut edges: Vec<(u16, u16, u32)> = Vec::with_capacity(touched * 2);
        for i in 0..self.npolys {
            let p = self.poly(i);
            let nv = self.poly_len(i);
            let mut k = nv.saturating_sub(1);
            for j in 0..nv {
                let (pj, pk) = (rd(p, j), rd(p, k));
                if pj == rem || pk == rem {
                    let (a, b) = if pk == rem { (pk, pj) } else { (pj, pk) };
                    let mut exists = false;
                    for e in &mut edges {
                        if e.1 == b {
                            e.2 += 1;
                            exists = true;
                        }
                    }
                    if !exists {
                        edges.push((a, b, 1));
                    }
                }
                k = j;
            }
        }
        edges.iter().filter(|e| e.2 < 2).count() <= 2
    }

    /// `removeVertex`.
    #[expect(
        clippy::too_many_lines,
        reason = "a literal port of stock's removeVertex"
    )]
    fn remove_vertex(&mut self, rem: u16, max_tris: usize) {
        let nvp = self.nvp;
        // Edges (a, b, reg, area, sub_area) that do not touch the removed vertex.
        let mut edges: Vec<[i32; 5]> = Vec::new();
        let mut i = 0;
        while i < self.npolys {
            let nv = self.poly_len(i);
            let p: Vec<u16> = sub(self.poly(i), 0, nv).to_vec();
            if p.contains(&rem) {
                let mut k = nv.saturating_sub(1);
                for j in 0..nv {
                    let (pj, pk) = (rd(&p, j), rd(&p, k));
                    if pj != rem && pk != rem {
                        edges.push([
                            i32::from(pk),
                            i32::from(pj),
                            i32::from(rd(&self.regs, i)),
                            i32::from(rd(&self.areas, i)),
                            i32::from(rd(&self.sub_areas, i)),
                        ]);
                    }
                    k = j;
                }
                // Remove the polygon: the last polygon takes its place.
                let last = self.npolys.saturating_sub(1);
                if i != last {
                    for k in 0..nvp {
                        mv(&mut self.polys, i * nvp * 2 + k, last * nvp * 2 + k);
                    }
                }
                for k in 0..nvp {
                    wr(&mut self.polys, i * nvp * 2 + nvp + k, NULL_IDX);
                }
                mv(&mut self.regs, i, last);
                mv(&mut self.areas, i, last);
                mv(&mut self.sub_areas, i, last);
                self.npolys -= 1;
            } else {
                i += 1;
            }
        }

        // Remove the vertex.
        let ri = usize::from(rem);
        if ri < self.verts.len() {
            self.verts.remove(ri);
        }
        // Adjust the indices.
        for i in 0..self.npolys {
            let nv = self.poly_len(i);
            for j in 0..nv {
                let at = i * nvp * 2 + j;
                let v = rd(&self.polys, at);
                if v > rem {
                    wr(&mut self.polys, at, v - 1);
                }
            }
        }
        let rem_i = i32::from(rem);
        for e in &mut edges {
            if e[0] > rem_i {
                e[0] -= 1;
            }
            if e[1] > rem_i {
                e[1] -= 1;
            }
        }
        let Some(first) = edges.first().copied() else {
            return;
        };

        // Chain the edges into the hole outline.
        let mut hole = vec![first[0]];
        let mut hreg = vec![first[2]];
        let mut harea = vec![first[3]];
        let mut hsub = vec![first[4]];
        while !edges.is_empty() {
            let mut matched = false;
            let mut i = 0;
            while i < edges.len() {
                let e = rd(&edges, i);
                let mut add = false;
                if rd(&hole, 0) == e[1] {
                    hole.insert(0, e[0]);
                    hreg.insert(0, e[2]);
                    harea.insert(0, e[3]);
                    hsub.insert(0, e[4]);
                    add = true;
                } else if hole.last().copied() == Some(e[0]) {
                    hole.push(e[1]);
                    hreg.push(e[2]);
                    harea.push(e[3]);
                    hsub.push(e[4]);
                    add = true;
                }
                if add {
                    edges.swap_remove(i);
                    matched = true;
                } else {
                    i += 1;
                }
            }
            if !matched {
                break;
            }
        }

        // Triangulate the hole.
        let hole_u: Vec<usize> = hole
            .iter()
            .map(|&h| usize::try_from(h).unwrap_or(0))
            .collect();
        let pts: Vec<P> = hole_u
            .iter()
            .map(|&h| {
                let v = vpt(&self.verts, idx_u16(h));
                (v[0], v[2])
            })
            .collect();
        // A bad triangulation keeps the triangles found so far, as in stock.
        let (tris, _ok) = triangulate(&pts);
        let ntris = tris.len();

        let mut polys = vec![NULL_IDX; (ntris + 1) * nvp];
        let mut pregs = vec![0_u16; ntris];
        let mut pareas = vec![0_u8; ntris];
        let mut psubs = vec![0_u16; ntris];
        let mut npolys = 0;
        for t in &tris {
            if t[0] != t[1] && t[0] != t[2] && t[1] != t[2] {
                for (k, &tv) in t.iter().enumerate() {
                    wr(&mut polys, npolys * nvp + k, idx_u16(rd(&hole_u, tv)));
                }
                let (r0, r1, r2) = (rd(&hreg, t[0]), rd(&hreg, t[1]), rd(&hreg, t[2]));
                let reg = if r0 != r1 || r1 != r2 {
                    i32::from(MULTIPLE_REGS)
                } else {
                    r0
                };
                wr(
                    &mut pregs,
                    npolys,
                    u16::try_from(reg).unwrap_or(MULTIPLE_REGS),
                );
                wr(
                    &mut pareas,
                    npolys,
                    u8::try_from(rd(&harea, t[0])).unwrap_or(0),
                );
                wr(
                    &mut psubs,
                    npolys,
                    u16::try_from(rd(&hsub, t[0])).unwrap_or(0),
                );
                npolys += 1;
            }
        }
        if npolys == 0 {
            return;
        }

        if nvp > 3 {
            let verts = self.verts.clone();
            npolys = {
                let (pregs, pareas, psubs) = (&mut pregs, &mut pareas, &mut psubs);
                let mut on_merge = |a: usize, b: usize, last: usize| {
                    if rd(pregs, a) != rd(pregs, b) {
                        wr(pregs, a, MULTIPLE_REGS);
                    }
                    mv(pregs, b, last);
                    mv(pareas, b, last);
                    mv(psubs, b, last);
                };
                merge_polys(&mut polys, npolys, &verts, nvp, &mut on_merge)
            };
        }

        // Store the polygons.
        for i in 0..npolys {
            if self.npolys >= max_tris {
                break;
            }
            let base = self.npolys * nvp * 2;
            for k in 0..nvp * 2 {
                wr(&mut self.polys, base + k, NULL_IDX);
            }
            for j in 0..nvp {
                wr(&mut self.polys, base + j, rd(&polys, i * nvp + j));
            }
            wr(&mut self.regs, self.npolys, rd(&pregs, i));
            wr(&mut self.areas, self.npolys, rd(&pareas, i));
            wr(&mut self.sub_areas, self.npolys, rd(&psubs, i));
            self.npolys += 1;
        }
    }
}

/// `computeVertexHash` of `(x, 0, z)`.
fn vertex_hash(x: u16, z: u16) -> usize {
    let n = 0x8da6_b343_u32
        .wrapping_mul(u32::from(x))
        .wrapping_add(0xcb1a_b31f_u32.wrapping_mul(u32::from(z)));
    usize::try_from(n & u32::try_from(VERTEX_BUCKET_COUNT - 1).unwrap_or(0)).unwrap_or(0)
}

/// `addVertex`: the vertex with the same x and z whose y is at most 2 away, else a new one.
fn add_vertex(
    vertex: [u16; 3],
    verts: &mut Vec<[u16; 3]>,
    first_vert: &mut [Option<usize>],
    next_vert: &mut Vec<Option<usize>>,
) -> usize {
    let [vx, vy, vz] = vertex;
    let bucket = vertex_hash(vx, vz);
    let mut cur = first_vert.get(bucket).copied().flatten();
    while let Some(k) = cur {
        let v = rd(verts, k);
        if v[0] == vx && (i32::from(v[1]) - i32::from(vy)).abs() <= 2 && v[2] == vz {
            return k;
        }
        cur = next_vert.get(k).copied().flatten();
    }
    let k = verts.len();
    verts.push(vertex);
    next_vert.push(first_vert.get(bucket).copied().flatten());
    if let Some(slot) = first_vert.get_mut(bucket) {
        *slot = Some(k);
    }
    k
}

/// `buildMeshAdjacency`: fills the neighbour half of every polygon.
fn build_mesh_adjacency(polys: &mut [u16], npolys: usize, nverts: usize, nvp: usize) {
    struct Edge {
        vert: [u16; 2],
        slot_in_poly: [usize; 2],
        poly: [usize; 2],
    }
    let mut first_edge: Vec<Option<usize>> = vec![None; nverts];
    let mut next_edge: Vec<Option<usize>> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    let ends = |polys: &[u16], i: usize, j: usize| -> Option<(u16, u16)> {
        let t = sub(polys, i * nvp * 2, nvp);
        let v0 = rd(t, j);
        if v0 == NULL_IDX {
            return None;
        }
        let v1 = if j + 1 >= nvp || rd(t, j + 1) == NULL_IDX {
            rd(t, 0)
        } else {
            rd(t, j + 1)
        };
        Some((v0, v1))
    };

    for i in 0..npolys {
        for j in 0..nvp {
            let Some((v0, v1)) = ends(polys, i, j) else {
                break;
            };
            if v0 < v1 {
                let e = edges.len();
                edges.push(Edge {
                    vert: [v0, v1],
                    slot_in_poly: [j, 0],
                    poly: [i, i],
                });
                next_edge.push(first_edge.get(usize::from(v0)).copied().flatten());
                if let Some(slot) = first_edge.get_mut(usize::from(v0)) {
                    *slot = Some(e);
                }
            }
        }
    }
    for i in 0..npolys {
        for j in 0..nvp {
            let Some((v0, v1)) = ends(polys, i, j) else {
                break;
            };
            if v0 > v1 {
                let mut e = first_edge.get(usize::from(v1)).copied().flatten();
                while let Some(k) = e {
                    if let Some(edge) = edges.get_mut(k)
                        && edge.vert[1] == v0
                        && edge.poly[0] == edge.poly[1]
                    {
                        edge.poly[1] = i;
                        edge.slot_in_poly[1] = j;
                        break;
                    }
                    e = next_edge.get(k).copied().flatten();
                }
            }
        }
    }
    for e in &edges {
        if e.poly[0] != e.poly[1] {
            wr(
                polys,
                e.poly[0] * nvp * 2 + nvp + e.slot_in_poly[0],
                idx_u16(e.poly[1]),
            );
            wr(
                polys,
                e.poly[1] * nvp * 2 + nvp + e.slot_in_poly[1],
                idx_u16(e.poly[0]),
            );
        }
    }
}

fn too_many_vertices(n: usize) -> io::Error {
    crate::error!("Too many vertices per tile {n} (max: 65535)")
}

/// `rcBuildPolyMesh` with nvp vertices per polygon. DayZ never removes border vertices.
///
/// Contours with fewer than 3 vertices are skipped. A contour that cannot be triangulated
/// completely keeps the triangles found so far (stock logs "Bad triangulation" and does the
/// same). A set without usable contours gives a mesh without polygons.
///
/// # Errors
/// `nvp` outside `3..=12`, 65 534 or more vertices in the contours ("Too many vertices per tile
/// N (max: 65535)") and more polygons than a tile can number.
pub fn build_poly_mesh(cset: &ContourSet, nvp: usize) -> io::Result<PolyMesh> {
    build(cset, nvp, false)
}

/// Stock `rcBuildPolyMesh`, which marks the vertices along the tile border and removes them
/// again where it can. DayZ does not do that (see [`build_poly_mesh`]); this entry point only
/// exists to show the difference in tests.
///
/// # Errors
/// As [`build_poly_mesh`].
pub fn build_poly_mesh_with_border_removal(cset: &ContourSet, nvp: usize) -> io::Result<PolyMesh> {
    build(cset, nvp, true)
}

fn contour_points(c: &Contour) -> Vec<P> {
    c.verts
        .iter()
        .map(|v| (i64::from(v[0]), i64::from(v[2])))
        .collect()
}

#[expect(
    clippy::too_many_lines,
    reason = "a literal port of stock's rcBuildPolyMesh"
)]
fn build(cset: &ContourSet, nvp: usize, mark_border_vertices: bool) -> io::Result<PolyMesh> {
    if !(3..=MAX_NVP).contains(&nvp) {
        return Err(crate::error!(
            "Vertices per polygon {nvp} out of range (3..={MAX_NVP})"
        ));
    }
    let mut max_vertices = 0_usize;
    let mut max_tris = 0_usize;
    for c in &cset.contours {
        if c.verts.len() < 3 {
            continue;
        }
        max_vertices += c.verts.len();
        max_tris += c.verts.len() - 2;
    }
    for c in &cset.contours {
        for v in &c.verts {
            if v.iter().take(3).any(|&k| !(0..=0xffff).contains(&k)) {
                return Err(crate::error!("Contour vertex {v:?} is outside 0..=65535"));
            }
        }
    }
    if max_vertices >= 0xfffe {
        return Err(too_many_vertices(max_vertices));
    }

    let mut vflags = vec![false; max_vertices];
    let mut w = Work {
        nvp,
        verts: Vec::new(),
        polys: vec![NULL_IDX; max_tris * nvp * 2],
        regs: vec![0; max_tris],
        areas: vec![0; max_tris],
        sub_areas: vec![0; max_tris],
        npolys: 0,
    };
    let mut next_vert: Vec<Option<usize>> = Vec::new();
    let mut first_vert: Vec<Option<usize>> = vec![None; VERTEX_BUCKET_COUNT];

    for cont in &cset.contours {
        if cont.verts.len() < 3 {
            continue;
        }
        // Triangulate the contour. A failure keeps the partial result, as stock does.
        let (tris, _complete) = triangulate(&contour_points(cont));

        // Add and merge the vertices.
        let mut indices: Vec<usize> = Vec::with_capacity(cont.verts.len());
        for v in &cont.verts {
            let k = add_vertex(
                [low_u16(v[0]), low_u16(v[1]), low_u16(v[2])],
                &mut w.verts,
                &mut first_vert,
                &mut next_vert,
            );
            indices.push(k);
            if mark_border_vertices && v[3] & RC_BORDER_VERTEX != 0 {
                wr(&mut vflags, k, true);
            }
        }

        // Build the initial polygons.
        let mut polys = vec![NULL_IDX; cont.verts.len() * nvp];
        let mut npolys = 0;
        for t in &tris {
            if t[0] != t[1] && t[0] != t[2] && t[1] != t[2] {
                for (k, &tv) in t.iter().enumerate() {
                    wr(&mut polys, npolys * nvp + k, idx_u16(rd(&indices, tv)));
                }
                npolys += 1;
            }
        }
        if npolys == 0 {
            continue;
        }
        if nvp > 3 {
            npolys = merge_polys(&mut polys, npolys, &w.verts, nvp, &mut |_, _, _| {});
        }

        // Store the polygons.
        for j in 0..npolys {
            if w.npolys >= max_tris {
                return Err(crate::error!(
                    "Too many polygons {} (max:{max_tris})",
                    w.npolys + 1
                ));
            }
            let base = w.npolys * nvp * 2;
            for k in 0..nvp {
                wr(&mut w.polys, base + k, rd(&polys, j * nvp + k));
            }
            wr(&mut w.regs, w.npolys, cont.reg);
            wr(&mut w.areas, w.npolys, cont.area);
            wr(&mut w.sub_areas, w.npolys, cont.sub_area);
            w.npolys += 1;
        }
    }

    // Remove the edge vertices (DayZ: no vertex is flagged, so nothing happens).
    let mut i = 0;
    while i < w.verts.len() {
        if rd(&vflags, i) {
            if !w.can_remove_vertex(idx_u16(i)) {
                i += 1;
                continue;
            }
            w.remove_vertex(idx_u16(i), max_tris);
            if i < vflags.len() {
                vflags.remove(i);
            }
        } else {
            i += 1;
        }
    }

    if w.verts.len() >= 0xffff {
        return Err(too_many_vertices(w.verts.len()));
    }
    if w.npolys > 0xffff {
        return Err(crate::error!(
            "Too many polygons per tile {} (max: 65535)",
            w.npolys
        ));
    }
    let npolys = w.npolys;
    build_mesh_adjacency(&mut w.polys, npolys, w.verts.len(), nvp);

    // Portal edges.
    if cset.border_size > 0 {
        for i in 0..npolys {
            let base = i * nvp * 2;
            for j in 0..nvp {
                let pj = rd(&w.polys, base + j);
                if pj == NULL_IDX {
                    break;
                }
                if rd(&w.polys, base + nvp + j) != NULL_IDX {
                    continue;
                }
                let mut nj = j + 1;
                if nj >= nvp || rd(&w.polys, base + nj) == NULL_IDX {
                    nj = 0;
                }
                let va = rd(&w.verts, usize::from(pj));
                let vb = rd(&w.verts, usize::from(rd(&w.polys, base + nj)));
                let (ax, az) = (i32::from(va[0]), i32::from(va[2]));
                let (bx, bz) = (i32::from(vb[0]), i32::from(vb[2]));
                let code = if ax == 0 && bx == 0 {
                    Some(0x8000)
                } else if az == cset.height && bz == cset.height {
                    Some(0x8001)
                } else if ax == cset.width && bx == cset.width {
                    Some(0x8002)
                } else if az == 0 && bz == 0 {
                    Some(0x8003)
                } else {
                    None
                };
                if let Some(code) = code {
                    wr(&mut w.polys, base + nvp + j, code);
                }
            }
        }
    }

    w.polys.truncate(npolys * nvp * 2);
    w.regs.truncate(npolys);
    w.areas.truncate(npolys);
    w.sub_areas.truncate(npolys);
    Ok(PolyMesh {
        verts: w.verts,
        polys: w.polys,
        regs: w.regs,
        flags: vec![0; npolys],
        areas: w.areas,
        sub_areas: w.sub_areas,
        nvp,
        bmin: cset.bmin,
        bmax: cset.bmax,
        cs: cset.cs,
        ch: cset.ch,
        border_size: cset.border_size,
        max_edge_error: cset.max_error,
    })
}
