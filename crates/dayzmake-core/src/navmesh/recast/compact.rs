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

//! `rcCompactHeightfield`, `rcBuildCompactHeightfield` (with the DayZ `subAreas`), `rcMarkBoxArea`
//! and `rcErodeWalkableArea` (spec 5.4 steps 5 and 6, 10.1 item 1), plus the chamfer distance
//! transform they share with the DayZ dilation.

use std::io;

use super::heightfield::Heightfield;
use crate::navmesh::num::{i32_to_f32, trunc_i32};

/// `RC_NULL_AREA`.
const NULL_AREA: u8 = 0;

/// `RC_NOT_CONNECTED`: the 6-bit "no neighbour" value of a direction.
pub const NOT_CONNECTED: u32 = 0x3f;

/// The largest span index of a column start (24 bits, `rcCompactCell::index`).
const MAX_SPAN_INDEX: usize = 0x00ff_ffff;

/// `rcGetDirOffsetX`.
#[must_use]
pub fn dir_offset_x(direction: i32) -> i32 {
    match direction & 3 {
        0 => -1,
        2 => 1,
        _ => 0,
    }
}

/// `rcGetDirOffsetY` (the second grid coordinate, z).
#[must_use]
pub fn dir_offset_y(direction: i32) -> i32 {
    match direction & 3 {
        1 => 1,
        3 => -1,
        _ => 0,
    }
}

/// `rcCompactSpan`: `con` packs six bits per direction, `h` is the free height above `y`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompactSpan {
    /// Lower extent of the span.
    pub y: u16,
    /// Region id (0 = none).
    pub reg: u16,
    /// Packed neighbour connections (24 bits).
    pub con: u32,
    /// Height of the span above `y`.
    pub h: u8,
}

impl CompactSpan {
    /// `rcGetCon`: the neighbour layer of `direction`, or [`NOT_CONNECTED`].
    #[must_use]
    pub fn get_con(&self, direction: i32) -> u32 {
        let shift = u32::try_from(direction & 3).unwrap_or(0) * 6;
        (self.con >> shift) & 0x3f
    }

    /// `rcSetCon`.
    pub fn set_con(&mut self, direction: i32, neighbor_index: u32) {
        let shift = u32::try_from(direction & 3).unwrap_or(0) * 6;
        self.con = (self.con & !(0x3f << shift)) | ((neighbor_index & 0x3f) << shift);
    }
}

/// `rcCompactHeightfield` with the DayZ `subAreas`. Fields past `areas` are filled by later stages
/// (distance field, regions, contours).
#[derive(Clone, Debug, Default)]
pub struct CompactHeightfield {
    /// Width of the field (x).
    pub width: i32,
    /// Height of the field (z).
    pub height: i32,
    /// Border size in cells.
    pub border_size: i32,
    /// Agent height in voxels.
    pub walkable_height: i32,
    /// Agent climb in voxels.
    pub walkable_climb: i32,
    /// Minimum bounds.
    pub bmin: [f32; 3],
    /// Maximum bounds.
    pub bmax: [f32; 3],
    /// Cell size.
    pub cs: f32,
    /// Cell height.
    pub ch: f32,
    /// Per column `(index of the first span, span count)`, `x + z * width` order.
    pub cells: Vec<(u32, u8)>,
    /// The spans of all columns.
    pub spans: Vec<CompactSpan>,
    /// Area id per span.
    pub areas: Vec<u8>,
    /// DayZ part id per span.
    pub sub_areas: Vec<u16>,
    /// Distance field per span (filled by the region builder).
    pub dist: Vec<u16>,
    /// Maximum of `dist`.
    pub max_distance: u16,
    /// Number of regions.
    pub max_regions: u16,
}

impl CompactHeightfield {
    /// The span index range of column `(x, z)`; empty when out of range.
    #[must_use]
    pub fn column_range(&self, x: i32, z: i32) -> std::ops::Range<usize> {
        if x < 0 || z < 0 || x >= self.width || z >= self.height {
            return 0..0;
        }
        let Some(i) = usize::try_from(x)
            .ok()
            .zip(
                usize::try_from(z)
                    .ok()
                    .zip(usize::try_from(self.width).ok()),
            )
            .and_then(|(x, (z, w))| z.checked_mul(w)?.checked_add(x))
        else {
            return 0..0;
        };
        let Some(&(index, count)) = self.cells.get(i) else {
            return 0..0;
        };
        let start = usize::try_from(index).unwrap_or(0);
        start..start.saturating_add(usize::from(count))
    }

    /// The neighbour of span `i` in column `(x, z)` through `direction`: `(nx, nz, index)`, or
    /// `None` when not connected.
    #[must_use]
    pub fn neighbor(&self, x: i32, z: i32, i: usize, direction: i32) -> Option<(i32, i32, usize)> {
        let con = self.spans.get(i)?.get_con(direction);
        if con == NOT_CONNECTED {
            return None;
        }
        let nx = x + dir_offset_x(direction);
        let nz = z + dir_offset_y(direction);
        let start = self.column_range(nx, nz).start;
        Some((nx, nz, start.checked_add(usize::try_from(con).ok()?)?))
    }
}

/// `rcGetHeightFieldSpanCount`: the number of walkable spans.
fn span_count(hf: &Heightfield) -> usize {
    hf.columns()
        .iter()
        .flatten()
        .filter(|s| s.area != NULL_AREA)
        .count()
}

/// `rcBuildCompactHeightfield`. The compact field takes `walkable_height * ch` on top of `bmax.y`
/// (the pipeline passes `proneHeight`), copies each solid span's `subArea` and packs the neighbour
/// connections, six bits per direction; a neighbour layer above 62 stays `RC_NOT_CONNECTED`.
///
/// # Errors
/// Returns an error when the walkable spans do not fit the 24-bit span index, or a column has more
/// than 255 walkable spans.
pub fn build_compact(
    walkable_height: i32,
    walkable_climb: i32,
    hf: &Heightfield,
) -> io::Result<CompactHeightfield> {
    const MAX_HEIGHT: i32 = 0xffff;
    const MAX_LAYERS: i32 = 0x3f - 1;
    let x_size = hf.width;
    let z_size = hf.height;
    let count = span_count(hf);
    if count > MAX_SPAN_INDEX {
        return Err(error!(
            "compact heightfield has too many spans ({count}, max {MAX_SPAN_INDEX})"
        ));
    }
    let mut bmax = hf.bmax;
    bmax[1] += i32_to_f32(walkable_height) * hf.ch;
    let mut chf = CompactHeightfield {
        width: x_size,
        height: z_size,
        border_size: 0,
        walkable_height,
        walkable_climb,
        bmin: hf.bmin,
        bmax,
        cs: hf.cs,
        ch: hf.ch,
        cells: vec![(0, 0); hf.columns().len()],
        spans: Vec::with_capacity(count),
        areas: Vec::with_capacity(count),
        sub_areas: Vec::with_capacity(count),
        dist: Vec::new(),
        max_distance: 0,
        max_regions: 0,
    };

    // Fill in cells and spans.
    for (column, cell) in hf.columns().iter().zip(chf.cells.iter_mut()) {
        // A column without spans keeps index 0, count 0.
        if column.is_empty() {
            continue;
        }
        let first = chf.spans.len();
        cell.0 = u32::try_from(first).map_err(|_| error!("compact span index overflow"))?;
        for (k, span) in column.iter().enumerate() {
            if span.area == NULL_AREA {
                continue;
            }
            let bot = i32::from(span.smax);
            let top = column.get(k + 1).map_or(MAX_HEIGHT, |n| i32::from(n.smin));
            chf.spans.push(CompactSpan {
                y: u16::try_from(bot.clamp(0, 0xffff)).unwrap_or(0),
                reg: 0,
                con: 0,
                h: u8::try_from((top - bot).clamp(0, 0xff)).unwrap_or(0),
            });
            chf.areas.push(span.area);
            chf.sub_areas.push(span.sub_area);
        }
        cell.1 = u8::try_from(chf.spans.len() - first)
            .map_err(|_| error!("a column has more than 255 walkable spans"))?;
    }

    // Find the neighbour connections.
    for z in 0..z_size {
        for x in 0..x_size {
            for i in chf.column_range(x, z) {
                let Some(&span) = chf.spans.get(i) else {
                    continue;
                };
                let mut new = span;
                for dir in 0..4 {
                    new.set_con(dir, NOT_CONNECTED);
                    let nx = x + dir_offset_x(dir);
                    let nz = z + dir_offset_y(dir);
                    // First check that the neighbour cell is in bounds.
                    if nx < 0 || nz < 0 || nx >= x_size || nz >= z_size {
                        continue;
                    }
                    // Iterate over all neighbour spans and check if any of them is accessible.
                    let range = chf.column_range(nx, nz);
                    let start = range.start;
                    for k in range {
                        let Some(n) = chf.spans.get(k) else {
                            continue;
                        };
                        let bot = i32::from(span.y).max(i32::from(n.y));
                        let top = (i32::from(span.y) + i32::from(span.h))
                            .min(i32::from(n.y) + i32::from(n.h));
                        // The gap must be walkable and the climb not too high.
                        if (top - bot) >= walkable_height
                            && (i32::from(n.y) - i32::from(span.y)).abs() <= walkable_climb
                        {
                            let layer = i32::try_from(k - start).unwrap_or(i32::MAX);
                            if layer > MAX_LAYERS {
                                continue;
                            }
                            new.set_con(dir, u32::try_from(layer).unwrap_or(NOT_CONNECTED));
                            break;
                        }
                    }
                }
                if let Some(s) = chf.spans.get_mut(i) {
                    *s = new;
                }
            }
        }
    }
    Ok(chf)
}

/// `rcMarkBoxArea`: every walkable span whose `y` lies within the box (world units, truncated to
/// voxels like upstream) gets `area`. A span's part id is untouched.
pub fn mark_box_area(chf: &mut CompactHeightfield, bmin: [f32; 3], bmax: [f32; 3], area: u8) {
    let x_lo = trunc_i32((bmin[0] - chf.bmin[0]) / chf.cs);
    let y_lo = trunc_i32((bmin[1] - chf.bmin[1]) / chf.ch);
    let z_lo = trunc_i32((bmin[2] - chf.bmin[2]) / chf.cs);
    let x_hi = trunc_i32((bmax[0] - chf.bmin[0]) / chf.cs);
    let y_hi = trunc_i32((bmax[1] - chf.bmin[1]) / chf.ch);
    let z_hi = trunc_i32((bmax[2] - chf.bmin[2]) / chf.cs);

    if x_hi < 0 || x_lo >= chf.width || z_hi < 0 || z_lo >= chf.height {
        return;
    }
    let x_lo = x_lo.max(0);
    let z_lo = z_lo.max(0);
    let x_hi = x_hi.min(chf.width - 1);
    let z_hi = z_hi.min(chf.height - 1);
    for z in z_lo..=z_hi {
        for x in x_lo..=x_hi {
            for i in chf.column_range(x, z) {
                let y = chf.spans.get(i).map_or(i32::MIN, |s| i32::from(s.y));
                if y >= y_lo
                    && y <= y_hi
                    && let Some(a) = chf.areas.get_mut(i)
                    && *a != NULL_AREA
                {
                    *a = area;
                }
            }
        }
    }
}

/// The two chamfer passes of `rcErodeWalkableArea` over the 4-connected spans, with a `label`
/// copied along from the neighbour whenever a span's distance strictly improves (DayZ: the area
/// and part id of the nearest seed travel with the distance). `dist` is a byte per span.
pub(crate) fn chamfer(chf: &CompactHeightfield, dist: &mut [u8], label: &mut [(u8, u16)]) {
    let w = chf.width;
    let h = chf.height;

    // Improve span `i` from span `m` with the step cost `k`.
    let relax = |dist: &mut [u8], label: &mut [(u8, u16)], i: usize, m: usize, k: i32| {
        let Some(&dm) = dist.get(m) else {
            return;
        };
        let nd = (i32::from(dm) + k).min(255);
        if dist.get(i).is_some_and(|&di| nd < i32::from(di))
            && let (Some(d), Some(&l)) = (dist.get_mut(i), label.get(m))
        {
            *d = u8::try_from(nd).unwrap_or(255);
            if let Some(slot) = label.get_mut(i) {
                *slot = l;
            }
        }
    };

    // Pass 1
    for y in 0..h {
        for x in 0..w {
            for i in chf.column_range(x, y) {
                // (-1,0) then (-1,-1) and (0,-1) then (1,-1)
                for (d1, d2) in [(0, 3), (3, 2)] {
                    if let Some((ax, ay, ai)) = chf.neighbor(x, y, i, d1) {
                        relax(dist, label, i, ai, 2);
                        if let Some((_, _, aai)) = chf.neighbor(ax, ay, ai, d2) {
                            relax(dist, label, i, aai, 3);
                        }
                    }
                }
            }
        }
    }

    // Pass 2
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            for i in chf.column_range(x, y) {
                for (d1, d2) in [(2, 1), (1, 0)] {
                    if let Some((ax, ay, ai)) = chf.neighbor(x, y, i, d1) {
                        relax(dist, label, i, ai, 2);
                        if let Some((_, _, aai)) = chf.neighbor(ax, ay, ai, d2) {
                            relax(dist, label, i, aai, 3);
                        }
                    }
                }
            }
        }
    }
}

/// `rcErodeWalkableArea`: spans with a chamfer distance to the nearest unwalkable span or missing
/// neighbour below `2 * radius` become `RC_NULL_AREA`.
///
/// # Errors
/// Returns an error when the field is inconsistent (arrays shorter than the span list).
pub fn erode(chf: &mut CompactHeightfield, radius: i32) -> io::Result<()> {
    let n = chf.spans.len();
    if chf.areas.len() != n {
        return Err(error!("compact heightfield areas do not match its spans"));
    }
    let mut dist = vec![0xffu8; n];

    // Mark boundary cells.
    for y in 0..chf.height {
        for x in 0..chf.width {
            for i in chf.column_range(x, y) {
                let area = chf.areas.get(i).copied().unwrap_or(NULL_AREA);
                let boundary = if area == NULL_AREA {
                    true
                } else {
                    let mut nc = 0;
                    for dir in 0..4 {
                        if let Some((_, _, ni)) = chf.neighbor(x, y, i, dir)
                            && chf.areas.get(ni).is_some_and(|&a| a != NULL_AREA)
                        {
                            nc += 1;
                        }
                    }
                    // At least one missing neighbour.
                    nc != 4
                };
                if boundary && let Some(d) = dist.get_mut(i) {
                    *d = 0;
                }
            }
        }
    }

    // The labels are not needed here (erosion only looks at distances).
    let mut label = vec![(0u8, 0u16); n];
    chamfer(chf, &mut dist, &mut label);

    // `(unsigned char)(radius*2)`
    let thr = u8::try_from((radius.wrapping_mul(2)) & 0xff).unwrap_or(0);
    for (a, &d) in chf.areas.iter_mut().zip(&dist) {
        if d < thr {
            *a = NULL_AREA;
        }
    }
    Ok(())
}
