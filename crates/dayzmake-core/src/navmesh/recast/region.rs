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

//! `rcBuildDistanceField`, `rcBuildRegions` (watershed) and `rcBuildRegionsMonotone` with the DayZ
//! part ids (spec 5.4 step 8, 6.2 step 1, 10.1 items 5 and 6).
//!
//! Wherever stock Recast compares the area of two spans this port compares the pair
//! `(area, sub_area)`. A region carries the part id of its first span, two regions merge only when
//! their part ids are equal, and the "too small" test of a connected group of regions takes the
//! minimum from the area of the group's first (lowest numbered) region.

use std::io;

use super::compact::CompactHeightfield;

/// `RC_BORDER_REG`: a region id with this bit set belongs to the tile border.
const BORDER_REG: u16 = 0x8000;
/// `RC_NULL_AREA`.
const NULL_AREA: u8 = 0;
/// `RC_NULL_NEI`.
const NULL_NEI: u16 = 0xffff;
/// Stock `LOG_NB_STACKS` and the matching stack count.
const NB_STACKS: usize = 8;
/// Stock `expandIters`.
const EXPAND_ITERS: i32 = 8;

fn rd<T: Copy + Default>(s: &[T], i: usize) -> T {
    s.get(i).copied().unwrap_or_default()
}

fn wr<T>(s: &mut [T], i: usize, v: T) {
    if let Some(slot) = s.get_mut(i) {
        *slot = v;
    }
}

/// Spans `a` and `b` have the same area and the same part id (the DayZ replacement of stock's
/// `areas[a] == areas[b]`).
fn same_part(chf: &CompactHeightfield, a: usize, b: usize) -> bool {
    rd(&chf.areas, a) == rd(&chf.areas, b) && rd(&chf.sub_areas, a) == rd(&chf.sub_areas, b)
}

/// The arrays of a compact field must agree before any stage indexes them.
fn check(chf: &CompactHeightfield) -> io::Result<()> {
    let n = chf.spans.len();
    if chf.areas.len() != n || chf.sub_areas.len() != n {
        return Err(error!("compact heightfield areas do not match its spans"));
    }
    Ok(())
}

/// `calculateDistanceField` (DayZ: a span is a boundary unless all four neighbours are connected
/// and share its area and part id). Returns the maximum distance.
fn calculate_distance_field(chf: &CompactHeightfield, src: &mut [u16]) -> u16 {
    src.fill(0xffff);
    for y in 0..chf.height {
        for x in 0..chf.width {
            for i in chf.column_range(x, y) {
                let mut nc = 0;
                for dir in 0..4 {
                    if let Some((_, _, ai)) = chf.neighbor(x, y, i, dir)
                        && same_part(chf, i, ai)
                    {
                        nc += 1;
                    }
                }
                if nc != 4 {
                    wr(src, i, 0);
                }
            }
        }
    }

    let improve = |src: &mut [u16], i: usize, m: usize, k: i32| {
        let cand = i32::from(rd(src, m)) + k;
        if cand < i32::from(rd(src, i)) {
            wr(src, i, u16::try_from(cand).unwrap_or(u16::MAX));
        }
    };

    // Pass 1: (-1,0) then (-1,-1), (0,-1) then (1,-1).
    for y in 0..chf.height {
        for x in 0..chf.width {
            for i in chf.column_range(x, y) {
                for (d1, d2) in [(0, 3), (3, 2)] {
                    if let Some((ax, ay, ai)) = chf.neighbor(x, y, i, d1) {
                        improve(src, i, ai, 2);
                        if let Some((_, _, aai)) = chf.neighbor(ax, ay, ai, d2) {
                            improve(src, i, aai, 3);
                        }
                    }
                }
            }
        }
    }
    // Pass 2: (1,0) then (1,1), (0,1) then (-1,1).
    for y in (0..chf.height).rev() {
        for x in (0..chf.width).rev() {
            for i in chf.column_range(x, y) {
                for (d1, d2) in [(2, 1), (1, 0)] {
                    if let Some((ax, ay, ai)) = chf.neighbor(x, y, i, d1) {
                        improve(src, i, ai, 2);
                        if let Some((_, _, aai)) = chf.neighbor(ax, ay, ai, d2) {
                            improve(src, i, aai, 3);
                        }
                    }
                }
            }
        }
    }
    src.iter().copied().max().unwrap_or(0)
}

/// `boxBlur`.
fn box_blur(chf: &CompactHeightfield, thr: i32, src: &[u16], dst: &mut [u16]) {
    let thr = thr * 2;
    for y in 0..chf.height {
        for x in 0..chf.width {
            for i in chf.column_range(x, y) {
                let cd = rd(src, i);
                if i32::from(cd) <= thr {
                    wr(dst, i, cd);
                    continue;
                }
                let cdi = i32::from(cd);
                let mut d = cdi;
                for dir in 0..4 {
                    if let Some((ax, ay, ai)) = chf.neighbor(x, y, i, dir) {
                        d += i32::from(rd(src, ai));
                        let dir2 = (dir + 1) & 3;
                        if let Some((_, _, ai2)) = chf.neighbor(ax, ay, ai, dir2) {
                            d += i32::from(rd(src, ai2));
                        } else {
                            d += cdi;
                        }
                    } else {
                        d += cdi * 2;
                    }
                }
                wr(dst, i, u16::try_from((d + 5) / 9).unwrap_or(u16::MAX));
            }
        }
    }
}

/// `rcBuildDistanceField`: fills `chf.dist` and `chf.max_distance` (the maximum before blurring,
/// as upstream).
///
/// # Errors
/// Returns an error when the compact field is inconsistent.
pub fn build_distance_field(chf: &mut CompactHeightfield) -> io::Result<()> {
    check(chf)?;
    let n = chf.spans.len();
    let mut src = vec![0u16; n];
    let mut dst = vec![0u16; n];
    chf.max_distance = calculate_distance_field(chf, &mut src);
    box_blur(chf, 1, &src, &mut dst);
    // The blur result is the distance field (upstream swaps `src` and `dst`).
    chf.dist = dst;
    Ok(())
}

/// `LevelStackEntry`; `index` is `None` once the cell has been used (upstream stores -1).
#[derive(Clone, Copy)]
struct LevelEntry {
    x: i32,
    y: i32,
    index: Option<usize>,
}

/// `floodRegion`.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors upstream floodRegion's parameter list"
)]
fn flood_region(
    x: i32,
    y: i32,
    i: usize,
    level: u16,
    r: u16,
    chf: &CompactHeightfield,
    src_reg: &mut [u16],
    src_dist: &mut [u16],
    stack: &mut Vec<LevelEntry>,
) -> bool {
    stack.clear();
    stack.push(LevelEntry {
        x,
        y,
        index: Some(i),
    });
    wr(src_reg, i, r);
    wr(src_dist, i, 0);

    let lev = level.saturating_sub(2);
    let mut count = 0;

    while let Some(back) = stack.pop() {
        let (cx, cy) = (back.x, back.y);
        let Some(ci) = back.index else {
            continue;
        };

        // Check if any of the neighbours already have a valid region set.
        let mut ar = 0u16;
        for dir in 0..4 {
            let Some((ax, ay, ai)) = chf.neighbor(cx, cy, ci, dir) else {
                continue;
            };
            if !same_part(chf, ai, i) {
                continue;
            }
            let nr = rd(src_reg, ai);
            if nr & BORDER_REG != 0 {
                continue; // Do not take borders into account.
            }
            if nr != 0 && nr != r {
                ar = nr;
                break;
            }
            let dir2 = (dir + 1) & 3;
            if let Some((_, _, ai2)) = chf.neighbor(ax, ay, ai, dir2) {
                if !same_part(chf, ai2, i) {
                    continue;
                }
                let nr2 = rd(src_reg, ai2);
                if nr2 != 0 && nr2 != r {
                    ar = nr2;
                    break;
                }
            }
        }
        if ar != 0 {
            wr(src_reg, ci, 0);
            continue;
        }

        count += 1;

        // Expand neighbours.
        for dir in 0..4 {
            if let Some((ax, ay, ai)) = chf.neighbor(cx, cy, ci, dir) {
                if !same_part(chf, ai, i) {
                    continue;
                }
                if rd(&chf.dist, ai) >= lev && rd(src_reg, ai) == 0 {
                    wr(src_reg, ai, r);
                    wr(src_dist, ai, 0);
                    stack.push(LevelEntry {
                        x: ax,
                        y: ay,
                        index: Some(ai),
                    });
                }
            }
        }
    }
    count > 0
}

/// `expandRegions`.
fn expand_regions(
    max_iter: i32,
    level: u16,
    chf: &CompactHeightfield,
    src_reg: &mut [u16],
    src_dist: &mut [u16],
    stack: &mut Vec<LevelEntry>,
    fill_stack: bool,
) {
    if fill_stack {
        // Find cells revealed by the raised level.
        stack.clear();
        for y in 0..chf.height {
            for x in 0..chf.width {
                for i in chf.column_range(x, y) {
                    if rd(&chf.dist, i) >= level
                        && rd(src_reg, i) == 0
                        && rd(&chf.areas, i) != NULL_AREA
                    {
                        stack.push(LevelEntry {
                            x,
                            y,
                            index: Some(i),
                        });
                    }
                }
            }
        }
    } else {
        // Mark all cells which already have a region.
        for e in stack.iter_mut() {
            if e.index.is_some_and(|i| rd(src_reg, i) != 0) {
                e.index = None;
            }
        }
    }

    let mut dirty: Vec<(usize, u16, u16)> = Vec::new();
    let mut iter = 0;
    while !stack.is_empty() {
        let mut failed = 0usize;
        dirty.clear();
        for e in stack.iter_mut() {
            let Some(i) = e.index else {
                failed += 1;
                continue;
            };
            let mut r = rd(src_reg, i);
            let mut d2 = 0xffffu16;
            for dir in 0..4 {
                let Some((_, _, ai)) = chf.neighbor(e.x, e.y, i, dir) else {
                    continue;
                };
                if !same_part(chf, ai, i) {
                    continue;
                }
                let nr = rd(src_reg, ai);
                if nr > 0 && nr & BORDER_REG == 0 && i32::from(rd(src_dist, ai)) + 2 < i32::from(d2)
                {
                    r = nr;
                    d2 = rd(src_dist, ai).wrapping_add(2);
                }
            }
            if r != 0 {
                e.index = None; // mark as used
                dirty.push((i, r, d2));
            } else {
                failed += 1;
            }
        }

        // Copy entries that differ between src and dst to keep them in sync.
        for &(idx, region, distance2) in &dirty {
            wr(src_reg, idx, region);
            wr(src_dist, idx, distance2);
        }

        if failed == stack.len() {
            break;
        }
        if level > 0 {
            iter += 1;
            if iter >= max_iter {
                break;
            }
        }
    }
}

/// `sortCellsByLevel` with `loglevelsPerStack = 1`.
fn sort_cells_by_level(
    start_level: u16,
    chf: &CompactHeightfield,
    src_reg: &[u16],
    stacks: &mut [Vec<LevelEntry>],
) {
    let start_level = i32::from(start_level >> 1);
    for s in stacks.iter_mut() {
        s.clear();
    }
    for y in 0..chf.height {
        for x in 0..chf.width {
            for i in chf.column_range(x, y) {
                if rd(&chf.areas, i) == NULL_AREA || rd(src_reg, i) != 0 {
                    continue;
                }
                let level = i32::from(rd(&chf.dist, i) >> 1);
                let s_id = (start_level - level).max(0);
                let Ok(s_id) = usize::try_from(s_id) else {
                    continue;
                };
                if let Some(s) = stacks.get_mut(s_id) {
                    s.push(LevelEntry {
                        x,
                        y,
                        index: Some(i),
                    });
                }
            }
        }
    }
}

/// `appendStacks`.
fn append_stacks(src_stack: &[LevelEntry], dst_stack: &mut Vec<LevelEntry>, src_reg: &[u16]) {
    for e in src_stack {
        if let Some(i) = e.index
            && rd(src_reg, i) == 0
        {
            dst_stack.push(*e);
        }
    }
}

/// Paints the non-null spans of a rectangle of columns (`paintRectRegion`).
fn paint_rect_region(
    chf: &CompactHeightfield,
    xs: (i32, i32),
    ys: (i32, i32),
    reg_id: u16,
    src_reg: &mut [u16],
) {
    for y in ys.0..ys.1 {
        for x in xs.0..xs.1 {
            for i in chf.column_range(x, y) {
                if rd(&chf.areas, i) != NULL_AREA {
                    wr(src_reg, i, reg_id);
                }
            }
        }
    }
}

/// Paints the four border strips with ids `1..=4 | RC_BORDER_REG`; returns the next region id.
fn paint_borders(chf: &CompactHeightfield, border_size: i32, src_reg: &mut [u16]) -> u16 {
    let mut id = 1u16;
    if border_size > 0 {
        let (w, h) = (chf.width, chf.height);
        // Make sure border will not overflow.
        let bw = w.min(border_size);
        let bh = h.min(border_size);
        paint_rect_region(chf, (0, bw), (0, h), id | BORDER_REG, src_reg);
        id += 1;
        paint_rect_region(chf, (w - bw, w), (0, h), id | BORDER_REG, src_reg);
        id += 1;
        paint_rect_region(chf, (0, w), (0, bh), id | BORDER_REG, src_reg);
        id += 1;
        paint_rect_region(chf, (0, w), (h - bh, h), id | BORDER_REG, src_reg);
        id += 1;
    }
    id
}

/// `rcRegion` (DayZ: with `sub_area`).
struct Region {
    span_count: i32,
    id: u16,
    area_type: u8,
    sub_area: u16,
    remap: bool,
    visited: bool,
    overlap: bool,
    connections: Vec<u16>,
    floors: Vec<u16>,
}

impl Region {
    fn new(id: u16) -> Self {
        Self {
            span_count: 0,
            id,
            area_type: 0,
            sub_area: 0,
            remap: false,
            visited: false,
            overlap: false,
            connections: Vec::new(),
            floors: Vec::new(),
        }
    }
}

/// `removeAdjacentNeighbours`.
fn remove_adjacent_neighbours(reg: &mut Region) {
    let mut i = 0;
    while i < reg.connections.len() && reg.connections.len() > 1 {
        let ni = (i + 1) % reg.connections.len();
        if reg.connections.get(i) == reg.connections.get(ni) {
            reg.connections.remove(i);
        } else {
            i += 1;
        }
    }
}

/// `replaceNeighbour`.
fn replace_neighbour(reg: &mut Region, old_id: u16, new_id: u16) {
    let mut changed = false;
    for c in &mut reg.connections {
        if *c == old_id {
            *c = new_id;
            changed = true;
        }
    }
    for f in &mut reg.floors {
        if *f == old_id {
            *f = new_id;
        }
    }
    if changed {
        remove_adjacent_neighbours(reg);
    }
}

/// `canMergeWithRegion` (DayZ: the part ids must be equal as well).
fn can_merge_with_region(a: &Region, b: &Region) -> bool {
    if a.area_type != b.area_type || a.sub_area != b.sub_area {
        return false;
    }
    if a.connections.iter().filter(|&&c| c == b.id).count() > 1 {
        return false;
    }
    !a.floors.contains(&b.id)
}

/// `addUniqueFloorRegion`.
fn add_unique_floor_region(reg: &mut Region, n: u16) {
    if !reg.floors.contains(&n) {
        reg.floors.push(n);
    }
}

/// `mergeRegions(regions[a], regions[b])`.
fn merge_regions(regions: &mut [Region], a: usize, b: usize) -> bool {
    let (Some(first), Some(second)) = (regions.get(a), regions.get(b)) else {
        return false;
    };
    let (aid, bid) = (first.id, second.id);
    let acon = first.connections.clone();
    let bcon = second.connections.clone();
    let bfloors = second.floors.clone();
    let bcount = second.span_count;

    let Some(pos_a) = acon.iter().position(|&c| c == bid) else {
        return false;
    };
    let Some(pos_b) = bcon.iter().position(|&c| c == aid) else {
        return false;
    };

    let mut merged = Vec::new();
    let na = acon.len();
    for i in 0..na - 1 {
        merged.push(rd(&acon, (pos_a + 1 + i) % na));
    }
    let nb = bcon.len();
    for i in 0..nb - 1 {
        merged.push(rd(&bcon, (pos_b + 1 + i) % nb));
    }
    if let Some(rega) = regions.get_mut(a) {
        rega.connections = merged;
        remove_adjacent_neighbours(rega);
        for f in bfloors {
            add_unique_floor_region(rega, f);
        }
        rega.span_count += bcount;
    }
    if let Some(regb) = regions.get_mut(b) {
        regb.span_count = 0;
        regb.connections.clear();
    }
    true
}

/// `isRegionConnectedToBorder`: one of the neighbours is the null region.
fn is_region_connected_to_border(reg: &Region) -> bool {
    reg.connections.contains(&0)
}

/// `isSolidEdge`.
fn is_solid_edge(
    chf: &CompactHeightfield,
    src_reg: &[u16],
    x: i32,
    y: i32,
    i: usize,
    dir: i32,
) -> bool {
    let r = chf
        .neighbor(x, y, i, dir)
        .map_or(0, |(_, _, ai)| rd(src_reg, ai));
    r != rd(src_reg, i)
}

/// `walkContour`: the regions met walking round the region of span `i`.
fn walk_contour(
    (mut x, mut y, mut i): (i32, i32, usize),
    mut dir: i32,
    chf: &CompactHeightfield,
    src_reg: &[u16],
) -> Vec<u16> {
    let mut cont = Vec::new();
    let start_dir = dir;
    let starti = i;

    let mut cur_reg = chf
        .neighbor(x, y, i, dir)
        .map_or(0, |(_, _, ai)| rd(src_reg, ai));
    cont.push(cur_reg);

    let mut iter = 0;
    loop {
        iter += 1;
        if iter >= 40000 {
            break;
        }
        if is_solid_edge(chf, src_reg, x, y, i, dir) {
            // Choose the edge corner.
            let r = chf
                .neighbor(x, y, i, dir)
                .map_or(0, |(_, _, ai)| rd(src_reg, ai));
            if r != cur_reg {
                cur_reg = r;
                cont.push(cur_reg);
            }
            dir = (dir + 1) & 3; // Rotate CW
        } else {
            let Some((nx, ny, ni)) = chf.neighbor(x, y, i, dir) else {
                // Should not happen.
                return cont;
            };
            x = nx;
            y = ny;
            i = ni;
            dir = (dir + 3) & 3; // Rotate CCW
        }
        if starti == i && start_dir == dir {
            break;
        }
    }

    // Remove adjacent duplicates.
    if cont.len() > 1 {
        let mut j = 0;
        while j < cont.len() {
            let nj = (j + 1) % cont.len();
            if cont.get(j) == cont.get(nj) {
                cont.remove(j);
            } else {
                j += 1;
            }
        }
    }
    cont
}

/// The minimum region area of `area`, from the per-area table (ids are 6 bits).
fn min_area_of(min_area_by_area: &[i32; 64], area: u8) -> i32 {
    rd(min_area_by_area, usize::from(area & 63))
}

/// `mergeAndFilterRegions` (DayZ: per-area minimum taken from the first region of a connected
/// group, merge only within equal part ids).
#[expect(
    clippy::too_many_lines,
    reason = "a direct port of upstream mergeAndFilterRegions"
)]
fn merge_and_filter_regions(
    chf: &CompactHeightfield,
    min_area_by_area: &[i32; 64],
    merge_region_size: i32,
    max_region_id: &mut u16,
    src_reg: &mut [u16],
) {
    let nreg = usize::from(*max_region_id) + 1;
    let mut regions: Vec<Region> = (0..=*max_region_id).map(Region::new).collect();

    // Find edge of a region and find connections around the contour.
    for y in 0..chf.height {
        for x in 0..chf.width {
            let col = chf.column_range(x, y);
            for i in col.clone() {
                let r = usize::from(rd(src_reg, i));
                if r == 0 || r >= nreg {
                    continue;
                }
                let Some(reg) = regions.get_mut(r) else {
                    continue;
                };
                reg.span_count += 1;
                if reg.span_count == 1 {
                    // DayZ: the region's part id is the one of its first span.
                    reg.sub_area = rd(&chf.sub_areas, i);
                }

                // Update floors.
                for j in col.clone() {
                    if i == j {
                        continue;
                    }
                    let floor_id = rd(src_reg, j);
                    if floor_id == 0 || usize::from(floor_id) >= nreg {
                        continue;
                    }
                    if usize::from(floor_id) == r {
                        reg.overlap = true;
                    }
                    add_unique_floor_region(reg, floor_id);
                }

                // Have found contour.
                if !reg.connections.is_empty() {
                    continue;
                }
                reg.area_type = rd(&chf.areas, i);

                // Check if this cell is next to a border.
                let ndir = (0..4).find(|&dir| is_solid_edge(chf, src_reg, x, y, i, dir));
                if let Some(ndir) = ndir {
                    // The cell is at border: walk around the contour to find all the neighbours.
                    let cont = walk_contour((x, y, i), ndir, chf, src_reg);
                    if let Some(reg) = regions.get_mut(r) {
                        reg.connections = cont;
                    }
                }
            }
        }
    }

    // Remove too small regions.
    for i in 0..nreg {
        let Some(reg) = regions.get_mut(i) else {
            continue;
        };
        if reg.id == 0 || reg.id & BORDER_REG != 0 || reg.span_count == 0 || reg.visited {
            continue;
        }
        // DayZ: the minimum comes from the first region of the group (this one).
        let min_region_area = min_area_of(min_area_by_area, reg.area_type);

        // Count the total size of all the connected regions and note whether the group touches a
        // tile border.
        let mut connects_to_border = false;
        let mut span_count = 0i32;
        let mut stack = vec![i];
        let mut trace = Vec::new();
        reg.visited = true;

        while let Some(ri) = stack.pop() {
            let Some(creg) = regions.get(ri) else {
                continue;
            };
            span_count += creg.span_count;
            trace.push(ri);
            let conns = creg.connections.clone();
            for c in conns {
                if c & BORDER_REG != 0 {
                    connects_to_border = true;
                    continue;
                }
                let Some(neireg) = regions.get_mut(usize::from(c)) else {
                    continue;
                };
                if neireg.visited || neireg.id == 0 || neireg.id & BORDER_REG != 0 {
                    continue;
                }
                neireg.visited = true;
                stack.push(usize::from(neireg.id));
            }
        }

        // Do not remove areas which connect to tile borders, as their size cannot be estimated
        // correctly.
        if span_count < min_region_area && !connects_to_border {
            for t in trace {
                if let Some(r) = regions.get_mut(t) {
                    r.span_count = 0;
                    r.id = 0;
                }
            }
        }
    }

    // Merge too small regions to neighbour regions.
    loop {
        let mut merge_count = 0;
        for i in 0..nreg {
            let Some(reg) = regions.get(i) else {
                continue;
            };
            if reg.id == 0 || reg.id & BORDER_REG != 0 || reg.overlap || reg.span_count == 0 {
                continue;
            }
            // Check to see if the region should be merged.
            if reg.span_count > merge_region_size && is_region_connected_to_border(reg) {
                continue;
            }
            // Find the smallest neighbour region that connects to this one.
            let mut smallest = 0x0fff_ffff;
            let mut merge_id = reg.id;
            for &c in &reg.connections {
                if c & BORDER_REG != 0 {
                    continue;
                }
                let Some(mreg) = regions.get(usize::from(c)) else {
                    continue;
                };
                if mreg.id == 0 || mreg.id & BORDER_REG != 0 || mreg.overlap {
                    continue;
                }
                if mreg.span_count < smallest
                    && can_merge_with_region(reg, mreg)
                    && can_merge_with_region(mreg, reg)
                {
                    smallest = mreg.span_count;
                    merge_id = mreg.id;
                }
            }
            // Found new id.
            if merge_id != reg.id {
                let old_id = reg.id;
                if merge_regions(&mut regions, usize::from(merge_id), i) {
                    // Fix up regions pointing to the current region.
                    for r in &mut regions {
                        if r.id == 0 || r.id & BORDER_REG != 0 {
                            continue;
                        }
                        // If another region was already merged into the current region, change
                        // the id of the previous region too.
                        if r.id == old_id {
                            r.id = merge_id;
                        }
                        // Replace the current region with the new one if it is a neighbour.
                        replace_neighbour(r, old_id, merge_id);
                    }
                    merge_count += 1;
                }
            }
        }
        if merge_count == 0 {
            break;
        }
    }

    // Compress region ids.
    for r in &mut regions {
        r.remap = r.id != 0 && r.id & BORDER_REG == 0;
    }
    let mut reg_id_gen = 0u16;
    for i in 0..nreg {
        if !regions.get(i).is_some_and(|r| r.remap) {
            continue;
        }
        let old_id = regions.get(i).map_or(0, |r| r.id);
        reg_id_gen += 1;
        for r in regions.iter_mut().skip(i) {
            if r.id == old_id {
                r.id = reg_id_gen;
                r.remap = false;
            }
        }
    }
    *max_region_id = reg_id_gen;

    // Remap regions.
    for s in src_reg.iter_mut() {
        if *s & BORDER_REG == 0 {
            *s = regions.get(usize::from(*s)).map_or(0, |r| r.id);
        }
    }
}

fn check_border_size(border_size: i32) -> io::Result<()> {
    if border_size < 0 {
        return Err(error!("negative border size {border_size}"));
    }
    Ok(())
}

/// `rcBuildRegions`: watershed partitioning of the compact field. Needs
/// [`build_distance_field`] first. `min_area_by_area` is the DayZ per-area minimum region area; a
/// group of regions smaller than the entry of its first region's area is dropped unless it
/// reaches a border region. Fills `reg` of the spans, `max_regions` and `border_size`.
///
/// # Errors
/// Returns an error for an inconsistent field, a missing distance field, a negative
/// `border_size` or when the region ids overflow ("Region ID overflow").
pub fn build_regions(
    chf: &mut CompactHeightfield,
    border_size: i32,
    min_area_by_area: &[i32; 64],
    merge_region_area: i32,
) -> io::Result<()> {
    check(chf)?;
    check_border_size(border_size)?;
    let n = chf.spans.len();
    if chf.dist.len() != n {
        return Err(error!(
            "the distance field must be built before the regions"
        ));
    }
    let mut src_reg = vec![0u16; n];
    let mut src_dist = vec![0u16; n];
    let mut lvl_stacks: Vec<Vec<LevelEntry>> = vec![Vec::new(); NB_STACKS];
    let mut stack: Vec<LevelEntry> = Vec::new();

    let mut region_id = paint_borders(chf, border_size, &mut src_reg);
    let mut level = u16::try_from(((i32::from(chf.max_distance) + 1) & !1) & 0xffff).unwrap_or(0);
    chf.border_size = border_size;

    let mut s_id = NB_STACKS - 1;
    while level > 0 {
        level = level.saturating_sub(2);
        s_id = (s_id + 1) & (NB_STACKS - 1);

        if s_id == 0 {
            sort_cells_by_level(level, chf, &src_reg, &mut lvl_stacks);
        } else {
            // Copy left overs from the last level.
            let (left, right) = lvl_stacks.split_at_mut(s_id);
            if let (Some(prev), Some(cur)) = (left.last(), right.first_mut()) {
                append_stacks(prev, cur, &src_reg);
            }
        }

        let mut cur = lvl_stacks
            .get_mut(s_id)
            .map(std::mem::take)
            .unwrap_or_default();
        // Expand current regions until no empty connected cells found.
        expand_regions(
            EXPAND_ITERS,
            level,
            chf,
            &mut src_reg,
            &mut src_dist,
            &mut cur,
            false,
        );
        // Mark new regions with ids.
        for e in &cur {
            if let Some(i) = e.index
                && rd(&src_reg, i) == 0
                && flood_region(
                    e.x,
                    e.y,
                    i,
                    level,
                    region_id,
                    chf,
                    &mut src_reg,
                    &mut src_dist,
                    &mut stack,
                )
            {
                if region_id == 0xffff {
                    return Err(error!("Region ID overflow"));
                }
                region_id += 1;
            }
        }
        if let Some(slot) = lvl_stacks.get_mut(s_id) {
            *slot = cur;
        }
    }

    // Expand current regions until no empty connected cells found.
    expand_regions(
        EXPAND_ITERS * 8,
        0,
        chf,
        &mut src_reg,
        &mut src_dist,
        &mut stack,
        true,
    );

    // Merge regions and filter out small regions.
    let mut max_regions = region_id;
    merge_and_filter_regions(
        chf,
        min_area_by_area,
        merge_region_area,
        &mut max_regions,
        &mut src_reg,
    );
    chf.max_regions = max_regions;
    for (s, &r) in chf.spans.iter_mut().zip(&src_reg) {
        s.reg = r;
    }
    Ok(())
}

/// `rcSweepSpan`.
#[derive(Clone, Copy, Default)]
struct SweepSpan {
    id: u16,
    ns: u16,
    nei: u16,
}

/// `rcBuildRegionsMonotone` (DayZ: sweeps also stop at changes of the part id). Same contract as
/// [`build_regions`]; the distance field is not used.
///
/// # Errors
/// Returns an error for an inconsistent field, a negative `border_size` or when the region ids
/// overflow ("Region ID overflow").
#[expect(
    clippy::too_many_lines,
    reason = "a direct port of upstream rcBuildRegionsMonotone"
)]
pub fn build_regions_monotone(
    chf: &mut CompactHeightfield,
    border_size: i32,
    min_area_by_area: &[i32; 64],
    merge_region_area: i32,
) -> io::Result<()> {
    check(chf)?;
    check_border_size(border_size)?;
    let (w, h) = (chf.width, chf.height);
    let mut src_reg = vec![0u16; chf.spans.len()];
    let mut id = paint_borders(chf, border_size, &mut src_reg);
    chf.border_size = border_size;

    let mut prev: Vec<i32> = Vec::new();
    // Index 0 is unused: sweep ids start at 1. A growable table replaces stock's fixed `max(w, h)`
    // one, which can overrun on multi-layer rows; indices are identical whenever stock stays in bounds.
    let mut sweeps: Vec<SweepSpan> = Vec::new();

    // Sweep one line at a time.
    for y in border_size..h - border_size {
        // Collect spans from this row.
        prev.clear();
        prev.resize(usize::from(id) + 1, 0);
        sweeps.clear();
        sweeps.push(SweepSpan::default());
        let mut rid = 1u16;

        for x in border_size..w - border_size {
            for i in chf.column_range(x, y) {
                if rd(&chf.areas, i) == NULL_AREA {
                    continue;
                }
                // -x
                let mut previd = 0u16;
                if let Some((_, _, ai)) = chf.neighbor(x, y, i, 0)
                    && rd(&src_reg, ai) & BORDER_REG == 0
                    && same_part(chf, i, ai)
                {
                    previd = rd(&src_reg, ai);
                }
                if previd == 0 {
                    previd = rid;
                    if rid == 0xffff {
                        return Err(error!("Region ID overflow"));
                    }
                    rid += 1;
                    sweeps.push(SweepSpan::default());
                }

                // -y
                if let Some((_, _, ai)) = chf.neighbor(x, y, i, 3) {
                    let nr = rd(&src_reg, ai);
                    if nr != 0 && nr & BORDER_REG == 0 && same_part(chf, i, ai) {
                        let sw = sweeps.get_mut(usize::from(previd));
                        if let Some(sw) = sw {
                            if sw.nei == 0 || sw.nei == nr {
                                sw.nei = nr;
                                sw.ns += 1;
                                if let Some(p) = prev.get_mut(usize::from(nr)) {
                                    *p += 1;
                                }
                            } else {
                                sw.nei = NULL_NEI;
                            }
                        }
                    }
                }
                wr(&mut src_reg, i, previd);
            }
        }

        // Create unique ids.
        for i in 1..usize::from(rid) {
            let Some(sw) = sweeps.get(i).copied() else {
                continue;
            };
            let new_id = if sw.nei != NULL_NEI
                && sw.nei != 0
                && prev.get(usize::from(sw.nei)).copied() == Some(i32::from(sw.ns))
            {
                sw.nei
            } else {
                if id == 0xffff {
                    return Err(error!("Region ID overflow"));
                }
                let v = id;
                id += 1;
                v
            };
            if let Some(sw) = sweeps.get_mut(i) {
                sw.id = new_id;
            }
        }

        // Remap ids.
        for x in border_size..w - border_size {
            for i in chf.column_range(x, y) {
                let r = rd(&src_reg, i);
                if r > 0 && r < rid {
                    wr(
                        &mut src_reg,
                        i,
                        sweeps.get(usize::from(r)).map_or(0, |s| s.id),
                    );
                }
            }
        }
    }

    // Merge regions and filter out small regions. Monotone partitioning does not generate
    // overlapping regions.
    let mut max_regions = id;
    merge_and_filter_regions(
        chf,
        min_area_by_area,
        merge_region_area,
        &mut max_regions,
        &mut src_reg,
    );
    chf.max_regions = max_regions;
    for (s, &r) in chf.spans.iter_mut().zip(&src_reg) {
        s.reg = r;
    }
    Ok(())
}
