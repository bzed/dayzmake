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

//! DayZ passes over the solid heightfield: the mode filter and the multi-height low-height filter
//! (spec 5.4 steps 2 and 4.1, `RecastDayZ.cpp`'s `dzModeFilterWalkableAreaFlat` and
//! `dzFilterLowHeightSpansMulti`).

use crate::navmesh::recast::heightfield::Heightfield;

/// Radius of the mode filter window (21 x 21 cells).
const MODE_RADIUS: i32 = 10;

/// The mode filter (spec 5.4 step 2). Only runs when `border > 10`. Every column in
/// `[border, size - border)` gets the most frequent non-zero area of the lowest spans in its
/// 21 x 21 window (ties go to the lowest id, an all-zero window gives 0); border columns keep their
/// value. The result is written into the lowest span of every non-empty column.
pub fn mode_filter(hf: &mut Heightfield, border: i32) {
    if border <= MODE_RADIUS {
        return;
    }
    let w = hf.width;
    let h = hf.height;
    let src: Vec<u8> = hf
        .columns()
        .iter()
        .map(|c| c.first().map_or(0, |s| s.area))
        .collect();
    let at = |x: i32, z: i32| -> u8 {
        if x < 0 || z < 0 || x >= w || z >= h {
            return 0;
        }
        let i = usize::try_from(x)
            .ok()
            .zip(usize::try_from(z).ok().zip(usize::try_from(w).ok()))
            .and_then(|(x, (z, w))| z.checked_mul(w)?.checked_add(x));
        i.and_then(|i| src.get(i)).copied().unwrap_or(0)
    };
    let mut dst = src.clone();
    for z in border..(h - border) {
        for x in border..(w - border) {
            let mut counts = [0u32; 256];
            for dz in -MODE_RADIUS..=MODE_RADIUS {
                for dx in -MODE_RADIUS..=MODE_RADIUS {
                    if let Some(c) = counts.get_mut(usize::from(at(x + dx, z + dz))) {
                        *c += 1;
                    }
                }
            }
            let mut best = 0u8;
            let mut best_count = 0u32;
            for (value, &c) in (0u8..=255).zip(counts.iter()).skip(1) {
                if c > best_count {
                    best = value;
                    best_count = c;
                }
            }
            let i = usize::try_from(x + z * w).ok();
            if let Some(d) = i.and_then(|i| dst.get_mut(i)) {
                *d = best;
            }
        }
    }
    for (column, &area) in hf.columns_mut().iter_mut().zip(&dst) {
        if let Some(first) = column.first_mut() {
            first.area = area;
        }
    }
}

/// The multi-height low-height filter (spec 5.4 step 4.1) with `heights` (voxels, ascending, the
/// last one the standing height) and `areas`. For every span with a span `next` above it, the gap
/// is `next.smin - s.smax`. Below a normal ceiling a gap of at most `heights[2]` gives area 0.
/// Below a span of `ceiling_area`, the first `i` with `gap <= heights[i]` sets `areas[i]`.
pub fn filter_low_height_multi(
    hf: &mut Heightfield,
    heights: [i32; 3],
    areas: [u8; 3],
    ceiling_area: u8,
) {
    let standing = heights[2];
    for column in hf.columns_mut() {
        for k in 0..column.len() {
            let Some(next) = column.get(k + 1).copied() else {
                break;
            };
            let Some(s) = column.get_mut(k) else {
                break;
            };
            let gap = i32::from(next.smin) - i32::from(s.smax);
            if next.area != ceiling_area {
                if gap <= standing {
                    s.area = 0;
                }
            } else if let Some(&a) = heights
                .iter()
                .position(|&hgt| gap <= hgt)
                .and_then(|i| areas.get(i))
            {
                s.area = a;
            }
        }
    }
}
