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

//! `rcFilterLowHangingWalkableObstacles` (copying the DayZ `subArea` too) and `rcFilterLedgeSpans`
//! with the DayZ `skipAreas` parameter (spec 5.4 step 4, 10.1 items 3 and 4).

use super::compact::{dir_offset_x, dir_offset_y};
use super::heightfield::Heightfield;

/// `RC_NULL_AREA`.
const NULL_AREA: u8 = 0;

/// `MAX_HEIGHT` of the ledge filter.
const MAX_HEIGHT: i32 = 0xffff;

/// `rcFilterLowHangingWalkableObstacles`: a non-walkable span directly above a walkable one, within
/// `climb` of its top, becomes walkable with the area and (DayZ) the `subArea` of the span below.
pub fn filter_low_hanging(hf: &mut Heightfield, climb: i32) {
    for column in hf.columns_mut() {
        let mut previous_smax = 0u16;
        let mut previous_was_walkable = false;
        let mut previous_area = NULL_AREA;
        let mut previous_sub = 0u16;
        for span in column.iter_mut() {
            let walkable = span.area != NULL_AREA;
            // If the current span is not walkable, but there is a walkable span just below it,
            // mark the span above it walkable too.
            if !walkable
                && previous_was_walkable
                && (i32::from(span.smax) - i32::from(previous_smax)).abs() <= climb
            {
                span.area = previous_area;
                span.sub_area = previous_sub;
            }
            // Copy the walkable flag so that it cannot propagate past multiple non-walkable objects.
            previous_was_walkable = walkable;
            previous_area = span.area;
            previous_sub = span.sub_area;
            previous_smax = span.smax;
        }
    }
}

/// `rcFilterLedgeSpans`: marks spans next to a ledge or on a too steep slope as unwalkable.
/// DayZ: a span whose area has `skip_areas[area]` set is skipped (areas 0 and 1 in the pipeline).
pub fn filter_ledges(
    hf: &mut Heightfield,
    walkable_height: i32,
    climb: i32,
    skip_areas: &[bool; 64],
) {
    let x_size = hf.width;
    let z_size = hf.height;
    let mut to_clear: Vec<(i32, i32, usize)> = Vec::new();
    for z in 0..z_size {
        for x in 0..x_size {
            let column = hf.column(x, z);
            for (si, span) in column.iter().enumerate() {
                // Skip non walkable spans (and, DayZ, the areas of skip_areas).
                if span.area == NULL_AREA
                    || skip_areas
                        .get(usize::from(span.area))
                        .copied()
                        .unwrap_or(false)
                {
                    continue;
                }
                let bot = i32::from(span.smax);
                let top = column.get(si + 1).map_or(MAX_HEIGHT, |n| i32::from(n.smin));

                // Find the neighbours' minimum height.
                let mut min_neighbor_height = MAX_HEIGHT;
                // Min and max height of accessible neighbours.
                let mut accessible_min = i32::from(span.smax);
                let mut accessible_max = i32::from(span.smax);

                for direction in 0..4 {
                    let dx = x + dir_offset_x(direction);
                    let dy = z + dir_offset_y(direction);
                    // Skip neighbours which are out of bounds.
                    if dx < 0 || dy < 0 || dx >= x_size || dy >= z_size {
                        min_neighbor_height = min_neighbor_height.min(-climb - bot);
                        continue;
                    }
                    let neighbor = hf.column(dx, dy);

                    // From minus infinity to the first span.
                    let neighbor_bot = -climb;
                    let neighbor_top = neighbor.first().map_or(MAX_HEIGHT, |n| i32::from(n.smin));
                    // Skip the neighbour if the gap between the spans is too small.
                    if top.min(neighbor_top) - bot.max(neighbor_bot) > walkable_height {
                        min_neighbor_height = min_neighbor_height.min(neighbor_bot - bot);
                    }

                    // Rest of the spans.
                    for (ni, n) in neighbor.iter().enumerate() {
                        let neighbor_bot = i32::from(n.smax);
                        let neighbor_top = neighbor
                            .get(ni + 1)
                            .map_or(MAX_HEIGHT, |m| i32::from(m.smin));
                        // Skip the neighbour if the gap between the spans is too small.
                        if top.min(neighbor_top) - bot.max(neighbor_bot) > walkable_height {
                            min_neighbor_height = min_neighbor_height.min(neighbor_bot - bot);
                            // Find the min/max accessible neighbour height.
                            if (neighbor_bot - bot).abs() <= climb {
                                if neighbor_bot < accessible_min {
                                    accessible_min = neighbor_bot;
                                }
                                if neighbor_bot > accessible_max {
                                    accessible_max = neighbor_bot;
                                }
                            }
                        }
                    }
                }

                // The current span is close to a ledge if the drop to any neighbour span is less
                // than the walkable climb; if the difference between all neighbours is too large,
                // we are at a steep slope.
                if min_neighbor_height < -climb || (accessible_max - accessible_min) > climb {
                    to_clear.push((x, z, si));
                }
            }
        }
    }
    // Upstream writes in place, but the filter only reads `smin`/`smax` of neighbours and the
    // area of the current span, so deferring the writes gives the same result.
    for (x, z, si) in to_clear {
        if let Some(span) = hf.column_mut(x, z).and_then(|c| c.get_mut(si)) {
            span.area = NULL_AREA;
        }
    }
}
