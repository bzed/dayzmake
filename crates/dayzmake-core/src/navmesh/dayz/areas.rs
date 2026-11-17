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

//! DayZ passes over the compact heightfield: `dzDilateArea`, `dzExtractTreeObstacles`, and the area
//! remap with the roof rule (spec 5.4 step 6). The dilation adapts the distance transform of
//! `rcErodeWalkableArea`.

use crate::navmesh::detour::Obstacle;
use crate::navmesh::num::{i32_to_f32, trunc_i32};
use crate::navmesh::recast::compact::{CompactHeightfield, chamfer};

fn flag(set: &[bool; 64], area: u8) -> bool {
    set.get(usize::from(area)).copied().unwrap_or(false)
}

/// `dzDilateArea`: spans of `target` area spread over the 4-connected neighbours with the chamfer
/// distance; every span with `mask[area]` set and `dist <= radius * 2` becomes `target` and takes
/// the part id of the nearest `target` span.
pub fn dilate_area(chf: &mut CompactHeightfield, radius: i32, target: u8, mask: &[bool; 64]) {
    let n = chf.spans.len();
    let mut dist: Vec<u8> = chf
        .areas
        .iter()
        .map(|&a| if a == target { 0 } else { 255 })
        .collect();
    let mut label: Vec<(u8, u16)> = chf
        .areas
        .iter()
        .zip(&chf.sub_areas)
        .map(|(&a, &s)| (a, s))
        .collect();
    dist.resize(n, 255);
    label.resize(n, (0, 0));
    chamfer(chf, &mut dist, &mut label);
    let limit = radius.wrapping_mul(2);
    for i in 0..n {
        let d = i32::from(dist.get(i).copied().unwrap_or(255));
        let Some(&area) = chf.areas.get(i) else {
            continue;
        };
        if flag(mask, area) && d <= limit {
            if let Some(a) = chf.areas.get_mut(i) {
                *a = target;
            }
            if let (Some(s), Some(&(_, sub))) = (chf.sub_areas.get_mut(i), label.get(i)) {
                *s = sub;
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct TreeAcc {
    count: i64,
    sum_x: i64,
    sum_y: i64,
    sum_z: i64,
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
    footprint: i32,
}

/// `(float)i` for a sum of cell indices.
#[expect(
    clippy::cast_precision_loss,
    reason = "C int to float conversion of a cell index sum; rounds to nearest as in C"
)]
fn i64_to_f32(i: i64) -> f32 {
    i as f32
}

/// Step 2: for each part the smallest chamfer distance to a span with a different label.
fn part_min_dist(
    chf: &CompactHeightfield,
    dist: &[u8],
    label: &[(u8, u16)],
    tree_area: u8,
    part_count: usize,
) -> Vec<u16> {
    let (w, h) = (chf.width, chf.height);
    let mut min_dist = vec![65535u16; part_count];
    for y in 0..h {
        for x in 0..w {
            for i in chf.column_range(x, y) {
                let Some(&(l_area, l_part)) = label.get(i) else {
                    continue;
                };
                if l_area != tree_area {
                    continue;
                }
                let own = u16::from(dist.get(i).copied().unwrap_or(255));
                for dir in 0..4 {
                    let d = match chf.neighbor(x, y, i, dir) {
                        None => own,
                        Some((_, _, ni)) => {
                            if label.get(ni) == label.get(i) {
                                continue;
                            }
                            u16::from(dist.get(ni).copied().unwrap_or(255)).saturating_add(own)
                        }
                    };
                    if let Some(m) = min_dist.get_mut(usize::from(l_part)) {
                        *m = (*m).min(d);
                    }
                }
            }
        }
    }
    min_dist
}

/// Step 6: one obstacle per surviving part, in the spec's operation order.
fn tree_records(chf: &CompactHeightfield, acc: &[TreeAcc]) -> Vec<Obstacle> {
    let border = i32_to_f32(chf.border_size);
    let cs = chf.cs;
    let ch = chf.ch;
    let mut out = Vec::new();
    for a in acc {
        if a.count == 0 {
            continue;
        }
        let count = i64_to_f32(a.count);
        let mut o = Obstacle::default();
        o.pos[0] = (i64_to_f32(a.sum_x) / count + border + 0.5f32) * cs + chf.bmin[0];
        o.pos[1] = (i64_to_f32(a.sum_y) / count + border + 0.5f32) * ch + chf.bmin[1];
        o.pos[2] = (i64_to_f32(a.sum_z) / count + border + 0.5f32) * cs + chf.bmin[2];
        o.radius = i32_to_f32(a.footprint) * cs * 0.5f32;
        out.push(o);
    }
    out
}

/// `dzExtractTreeObstacles` (spec 5.4, definition after the steps). Trees (`tree_area` spans with a
/// part id below `part_count`) whose nearest other seed is farther than `clearance + clearance`
/// and whose footprint is at most `max_footprint` become `walkable_area` and give one obstacle
/// record; all other tree spans become area 0. Tree spans lose their part id.
///
/// The obstacle position uses `chf.border_size` (0 at this point of the pipeline) and `chf.bmin`.
pub fn extract_tree_obstacles(
    chf: &mut CompactHeightfield,
    tree_area: u8,
    walkable_area: u8,
    part_count: usize,
    seed_areas: &[bool; 64],
    clearance: f32,
    max_footprint: f32,
) -> Vec<Obstacle> {
    let n = chf.spans.len();
    let w = chf.width;
    let h = chf.height;

    // 1. Seeds and the chamfer; each span carries the label of its nearest seed.
    let mut dist: Vec<u8> = chf
        .areas
        .iter()
        .map(|&a| if flag(seed_areas, a) { 0 } else { 255 })
        .collect();
    let mut label: Vec<(u8, u16)> = chf
        .areas
        .iter()
        .zip(&chf.sub_areas)
        .map(|(&a, &s)| (a, s))
        .collect();
    dist.resize(n, 255);
    label.resize(n, (0, 0));
    chamfer(chf, &mut dist, &mut label);

    // 2. The distance of each part to the nearest other label.
    let mut min_dist = part_min_dist(chf, &dist, &label, tree_area, part_count);

    // 3. Collect the trees that are far enough from everything else.
    // `(u16)(int)((2.0f + 2.0f) / cs)`
    let thr = u16::try_from(trunc_i32((clearance + clearance) / chf.cs) & 0xffff).unwrap_or(0);
    let mut acc = vec![TreeAcc::default(); part_count];
    for y in 0..h {
        for x in 0..w {
            for i in chf.column_range(x, y) {
                if chf.areas.get(i) != Some(&tree_area) {
                    continue;
                }
                let Some(&sub) = chf.sub_areas.get(i) else {
                    continue;
                };
                let p = usize::from(sub);
                if p >= part_count || min_dist.get(p).is_none_or(|&m| thr >= m) {
                    continue;
                }
                let sy = chf.spans.get(i).map_or(0, |s| i64::from(s.y));
                if let Some(a) = acc.get_mut(p) {
                    if a.count == 0 {
                        a.min_x = x;
                        a.max_x = x;
                        a.min_z = y;
                        a.max_z = y;
                    }
                    a.count += 1;
                    a.sum_x += i64::from(x);
                    a.sum_y += sy;
                    a.sum_z += i64::from(y);
                    a.min_x = a.min_x.min(x);
                    a.max_x = a.max_x.max(x);
                    a.min_z = a.min_z.min(y);
                    a.max_z = a.max_z.max(y);
                }
            }
        }
    }

    // 4. Reject parts with a footprint above the maximum.
    let max_cells = trunc_i32(max_footprint / chf.cs);
    for (p, a) in acc.iter_mut().enumerate() {
        if a.count == 0 {
            continue;
        }
        let mut size = a.max_z - a.min_z;
        let wx = a.max_x - a.min_x;
        if size < wx {
            size = wx;
        }
        if max_cells < size + 1 {
            if let Some(m) = min_dist.get_mut(p) {
                *m = 0;
            }
            a.count = 0;
        } else {
            a.footprint = size + 1;
        }
    }

    // 5. Rewrite every tree span.
    for i in 0..n {
        if chf.areas.get(i) != Some(&tree_area) {
            continue;
        }
        let p = label.get(i).map_or(usize::MAX, |l| usize::from(l.1));
        let ok = p < part_count && min_dist.get(p).is_some_and(|&m| thr < m);
        if let Some(a) = chf.areas.get_mut(i) {
            *a = if ok { walkable_area } else { 0 };
        }
        if let Some(s) = chf.sub_areas.get_mut(i) {
            *s = 0;
        }
    }

    // 6. One obstacle per surviving part.
    tree_records(chf, &acc)
}

/// The remap and the roof rule (spec 5.4 steps 6.5 and 6.6): areas 7 and 9 become 1, 11 becomes 8,
/// then the topmost span of a column with area 8 and `h == 255` becomes 1.
pub fn remap_and_roof(chf: &mut CompactHeightfield) {
    for a in &mut chf.areas {
        match *a {
            7 | 9 => *a = 1,
            11 => *a = 8,
            _ => {}
        }
    }
    for z in 0..chf.height {
        for x in 0..chf.width {
            let range = chf.column_range(x, z);
            if range.is_empty() {
                continue;
            }
            let top = range.end - 1;
            let h = chf.spans.get(top).map_or(0, |s| s.h);
            if let Some(a) = chf.areas.get_mut(top)
                && *a == 8
                && h == 255
            {
                *a = 1;
            }
        }
    }
}
