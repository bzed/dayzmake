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

//! `rasterizeTri` and `dividePoly` from Recast `6d1f971`, plus the DayZ five-mode cell actions and
//! the part rasteriser (spec 5.3).

#![expect(
    clippy::similar_names,
    reason = "geometry code keeps Recast's variable names"
)]

use std::collections::BTreeMap;

use super::heightfield::{Heightfield, SPAN_MAX_HEIGHT, Span, SpanPiece};
use crate::navmesh::num::{ceil_i32, floor_i32, i32_to_f32, trunc_i32};

/// What a rasterisation does to the spans of a cell (spec 5.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterMode {
    /// Mode 0: stock `addSpan`.
    Add,
    /// Mode 1, doors: paint from the start span (the last span with `smin <=` the piece's, or the
    /// first) upward until `smax <= s.smax`.
    PaintBelow,
    /// Mode 2, trees: paint the ground around the trunk; never adds.
    Tree,
    /// Mode 3: raise the area of spans under deep enough water.
    Water,
    /// Mode 4, hide parts: paint touched spans, add the piece when nothing was touched.
    PaintOrAdd,
}

/// Whether a triangle was rasterised completely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterOutcome {
    /// All cells were handled.
    Done,
    /// [`RasterMode::PaintBelow`] met a span already painted with this `(area, sub_area)`; the
    /// remaining cells of the triangle were skipped (not an error).
    Repeat,
}

/// Parameters of one rasterisation.
#[derive(Clone, Copy, Debug)]
pub struct RasterParams {
    /// The cell action.
    pub mode: RasterMode,
    /// Area id to write.
    pub area: u8,
    /// Part id to write.
    pub sub_area: u16,
    /// `flagMergeThreshold` of `addSpan` (1 for all triangles).
    pub flag_merge_threshold: i32,
    /// `walkableClimb` in voxels ([`RasterMode::Tree`]).
    pub walkable_climb: i32,
    /// `walkableHeight` in voxels ([`RasterMode::Tree`]).
    pub walkable_height: i32,
    /// Deep water depth in voxels ([`RasterMode::Water`]).
    pub water_depth: i32,
}

impl Default for RasterParams {
    fn default() -> Self {
        Self {
            mode: RasterMode::Add,
            area: 0,
            sub_area: 0,
            flag_merge_threshold: 1,
            walkable_climb: 0,
            walkable_height: 0,
            water_depth: 0,
        }
    }
}

/// `rcClamp` with C semantics: no panic when `lo > hi` (the lower bound wins first).
fn clamp_c(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Room for a polygon during clipping: convex pieces of a triangle have at most 7 vertices; the
/// buffer is larger so that hostile (NaN) input can only drop vertices, never overflow.
const MAX_VERTS: usize = 16;

#[derive(Clone, Copy)]
struct Poly {
    v: [[f32; 3]; MAX_VERTS],
    n: usize,
}

impl Poly {
    const EMPTY: Self = Self {
        v: [[0.0; 3]; MAX_VERTS],
        n: 0,
    };

    fn push(&mut self, p: [f32; 3]) {
        if let Some(slot) = self.v.get_mut(self.n) {
            *slot = p;
            self.n += 1;
        }
    }

    fn verts(&self) -> &[[f32; 3]] {
        self.v.get(..self.n).unwrap_or(&[])
    }
}

/// `dividePoly`: split a convex polygon at `axis_offset` along `axis` into the part before (`out1`)
/// and after (`out2`).
fn divide_poly(input: &Poly, out1: &mut Poly, out2: &mut Poly, axis_offset: f32, axis: usize) {
    out1.n = 0;
    out2.n = 0;
    let n = input.n;
    let mut delta = [0.0f32; MAX_VERTS];
    for (d, v) in delta.iter_mut().zip(input.verts()) {
        *d = axis_offset - v.get(axis).copied().unwrap_or(0.0);
    }
    let mut b = n.wrapping_sub(1);
    for a in 0..n {
        let (da, db) = (
            delta.get(a).copied().unwrap_or(0.0),
            delta.get(b).copied().unwrap_or(0.0),
        );
        let (va, vb) = (
            input.v.get(a).copied().unwrap_or([0.0; 3]),
            input.v.get(b).copied().unwrap_or([0.0; 3]),
        );
        let same_side = (da >= 0.0) == (db >= 0.0);
        if same_side {
            // Addition is done even for points on the dividing line.
            if da >= 0.0 {
                out1.push(va);
                if da != 0.0 {
                    b = a;
                    continue;
                }
            }
            out2.push(va);
        } else {
            let s = db / (db - da);
            let p = [
                vb[0] + (va[0] - vb[0]) * s,
                vb[1] + (va[1] - vb[1]) * s,
                vb[2] + (va[2] - vb[2]) * s,
            ];
            out1.push(p);
            out2.push(p);
            // Points on the dividing line were already added above.
            if da > 0.0 {
                out1.push(va);
            } else if da < 0.0 {
                out2.push(va);
            }
        }
        b = a;
    }
}

/// The grid data the clipper needs, copied so the cell callback can mutate the heightfield.
#[derive(Clone, Copy)]
struct Grid {
    w: i32,
    h: i32,
    bmin: [f32; 3],
    bmax: [f32; 3],
    cs: f32,
    ch: f32,
}

impl Grid {
    fn of(hf: &Heightfield) -> Self {
        Self {
            w: hf.width,
            h: hf.height,
            bmin: hf.bmin,
            bmax: hf.bmax,
            cs: hf.cs,
            ch: hf.ch,
        }
    }
}

/// `overlapBounds`.
fn overlap_bounds(a_min: [f32; 3], a_max: [f32; 3], b_min: [f32; 3], b_max: [f32; 3]) -> bool {
    a_min[0] <= b_max[0]
        && a_max[0] >= b_min[0]
        && a_min[1] <= b_max[1]
        && a_max[1] >= b_min[1]
        && a_min[2] <= b_max[2]
        && a_max[2] >= b_min[2]
}

/// `rasterizeTri`: clip the triangle into grid cells and call `cell(x, z, smin, smax)` for each
/// piece; a `false` result stops the clipping. `older_way` is the part clipping of spec 5.3: the
/// first row and column are clamped to 0 instead of -1, and pieces with a vertex before the grid's
/// min x or z are skipped. Returns `false` when `cell` stopped it.
fn clip_triangle(
    g: &Grid,
    tri: [[f32; 3]; 3],
    older_way: bool,
    cell: &mut dyn FnMut(i32, i32, u16, u16) -> bool,
) -> bool {
    let [v0, v1, v2] = tri;
    let mut tri_min = v0;
    let mut tri_max = v0;
    for v in [v1, v2] {
        for ((lo, hi), c) in tri_min.iter_mut().zip(tri_max.iter_mut()).zip(v) {
            *lo = lo.min(c);
            *hi = hi.max(c);
        }
    }
    if !overlap_bounds(tri_min, tri_max, g.bmin, g.bmax) {
        return true;
    }
    let inv_cs = 1.0f32 / g.cs;
    let inv_ch = 1.0f32 / g.ch;
    let by = g.bmax[1] - g.bmin[1];
    let first = if older_way { 0 } else { -1 };

    let z0 = trunc_i32((tri_min[2] - g.bmin[2]) * inv_cs);
    let z1 = trunc_i32((tri_max[2] - g.bmin[2]) * inv_cs);
    let z0 = clamp_c(z0, first, g.h.saturating_sub(1));
    let z1 = clamp_c(z1, 0, g.h.saturating_sub(1));

    let mut input = Poly::EMPTY;
    input.push(v0);
    input.push(v1);
    input.push(v2);
    let mut in_row = Poly::EMPTY;
    let mut p1 = Poly::EMPTY;
    let mut p2 = Poly::EMPTY;

    for z in z0..=z1 {
        let cell_z = g.bmin[2] + i32_to_f32(z) * g.cs;
        divide_poly(&input, &mut in_row, &mut p1, cell_z + g.cs, 2);
        std::mem::swap(&mut input, &mut p1);
        if in_row.n < 3 || z < 0 {
            continue;
        }
        let mut min_x = in_row.v[0][0];
        let mut max_x = min_x;
        for v in in_row.verts().iter().skip(1) {
            min_x = min_x.min(v[0]);
            max_x = max_x.max(v[0]);
        }
        let x0 = trunc_i32((min_x - g.bmin[0]) * inv_cs);
        let x1 = trunc_i32((max_x - g.bmin[0]) * inv_cs);
        if x1 < 0 || x0 >= g.w {
            continue;
        }
        let x0 = clamp_c(x0, first, g.w.saturating_sub(1));
        let x1 = clamp_c(x1, 0, g.w.saturating_sub(1));

        for x in x0..=x1 {
            let cx = g.bmin[0] + i32_to_f32(x) * g.cs;
            divide_poly(&in_row, &mut p1, &mut p2, cx + g.cs, 0);
            std::mem::swap(&mut in_row, &mut p2);
            if p1.n < 3 || x < 0 {
                continue;
            }
            if older_way
                && p1
                    .verts()
                    .iter()
                    .any(|v| v[2] < g.bmin[2] || v[0] < g.bmin[0])
            {
                continue;
            }
            let mut span_min = p1.v[0][1];
            let mut span_max = span_min;
            for v in p1.verts().iter().skip(1) {
                span_min = span_min.min(v[1]);
                span_max = span_max.max(v[1]);
            }
            span_min -= g.bmin[1];
            span_max -= g.bmin[1];
            if span_max < 0.0 || span_min > by {
                continue;
            }
            if span_min < 0.0 {
                span_min = 0.0;
            }
            if span_max > by {
                span_max = by;
            }
            let max_h = i32::from(SPAN_MAX_HEIGHT);
            let lo = clamp_c(floor_i32(span_min * inv_ch), 0, max_h);
            let hi = clamp_c(ceil_i32(span_max * inv_ch), lo.saturating_add(1), max_h);
            let (Ok(lo), Ok(hi)) = (u16::try_from(lo), u16::try_from(hi)) else {
                continue;
            };
            if !cell(x, z, lo, hi) {
                return false;
            }
        }
    }
    true
}

fn paint(s: &mut Span, p: &RasterParams) {
    s.area = p.area;
    s.sub_area = p.sub_area;
    // The first hide part keeps the span (R21): a second copy of an object does not take it over.
    if p.mode == RasterMode::PaintOrAdd && s.hide_part == 0 {
        s.hide_part = p.sub_area;
    }
}

/// The mode's action on one cell with the new piece `(smin, smax)`. Returns `false` for a repeat
/// ([`RasterMode::PaintBelow`] meeting an already painted span).
fn apply_cell(
    hf: &mut Heightfield,
    x: i32,
    z: i32,
    smin: u16,
    smax: u16,
    p: &RasterParams,
) -> bool {
    let (smin_i, smax_i) = (i32::from(smin), i32::from(smax));
    match p.mode {
        RasterMode::Add => {
            hf.add_span(x, z, smin, smax, p.area, p.flag_merge_threshold);
        }
        RasterMode::PaintBelow => {
            let Some(col) = hf.column_mut(x, z) else {
                return true;
            };
            // The last span whose smin <= piece smin, or the first span.
            let start = col.iter().rposition(|s| s.smin <= smin).unwrap_or(0);
            for s in col.iter_mut().skip(start) {
                if smax <= s.smax {
                    break;
                }
                if s.area != 0 {
                    if s.area == p.area && s.sub_area == p.sub_area {
                        return false;
                    }
                    paint(s, p);
                }
            }
        }
        RasterMode::Tree => {
            let Some(col) = hf.column_mut(x, z) else {
                return true;
            };
            for s in col.iter_mut() {
                if smax < s.smin {
                    break;
                }
                let s_max = i32::from(s.smax);
                if smax_i <= s_max.saturating_add(p.walkable_climb) {
                    break;
                }
                let hit = if s_max < smin_i {
                    smin_i < s_max.saturating_add(p.walkable_height)
                } else {
                    s_max < smax_i
                };
                if hit {
                    paint(s, p);
                }
            }
        }
        RasterMode::Water => {
            let Some(col) = hf.column_mut(x, z) else {
                return true;
            };
            for s in col.iter_mut() {
                if smax <= s.smin {
                    break;
                }
                if i32::from(s.smin).saturating_add(p.water_depth) <= smin_i {
                    s.area = s.area.max(p.area);
                }
            }
        }
        RasterMode::PaintOrAdd => {
            let Some(col) = hf.column_mut(x, z) else {
                return true;
            };
            let mut hit = false;
            for s in col.iter_mut() {
                if smax < s.smin {
                    break;
                }
                if smax_i <= i32::from(s.smax) + 1 {
                    break;
                }
                if s.smax >= smin {
                    paint(s, p);
                    hit = true;
                }
            }
            if !hit {
                // The added span has sub area 0 (addSpan), but it belongs to the hide part (R21).
                hf.add_piece(
                    &SpanPiece::new(x, z, smin, smax, p.area, p.flag_merge_threshold)
                        .in_hide_part(p.sub_area),
                );
            }
        }
    }
    true
}

/// `dzRasterizeTriangle`: rasterise one triangle in the mode of `p`. The vertices are used as
/// given; the caller passes the server's `(v0, v2, v1)` order. [`RasterOutcome::Repeat`] means a
/// [`RasterMode::PaintBelow`] cell was already painted and the rest of the triangle was skipped.
pub fn rasterize_triangle(
    hf: &mut Heightfield,
    v0: [f32; 3],
    v1: [f32; 3],
    v2: [f32; 3],
    p: &RasterParams,
) -> RasterOutcome {
    let g = Grid::of(hf);
    let done = clip_triangle(&g, [v0, v1, v2], false, &mut |x, z, lo, hi| {
        apply_cell(hf, x, z, lo, hi, p)
    });
    if done {
        RasterOutcome::Done
    } else {
        RasterOutcome::Repeat
    }
}

/// `dzRasterizePart`: rasterise the triangles `tris` (three vertices each, rasteriser order) as one
/// footprint. The union of vertical extents per cell is collected first ("the older way" clipping:
/// pieces with a vertex before the min x or z edge are skipped), then the mode's cell action runs
/// once per cell with that extent, row by row. A repeat only ends its cell. Trailing vertices
/// (when the count is not a multiple of 3) are ignored.
pub fn rasterize_part(hf: &mut Heightfield, tris: &[[f32; 3]], p: &RasterParams) {
    let g = Grid::of(hf);
    // Only the cells the part touches are recorded, ordered z outer, x inner.
    let mut extents: BTreeMap<(i32, i32), (u16, u16)> = BTreeMap::new();
    for &tri in tris.as_chunks::<3>().0 {
        // The result only says whether the callback stopped early; this callback never does.
        clip_triangle(&g, tri, true, &mut |x, z, lo, hi| {
            extents
                .entry((z, x))
                .and_modify(|e| *e = (e.0.min(lo), e.1.max(hi)))
                .or_insert((lo, hi));
            true
        });
    }
    for ((z, x), (lo, hi)) in extents {
        // A repeat only ends this cell; nothing else to do for it.
        apply_cell(hf, x, z, lo, hi, p);
    }
}
