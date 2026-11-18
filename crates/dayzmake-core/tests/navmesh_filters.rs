//! Span filters, the compact heightfield and the DayZ area passes (spec 5.4 steps 2, 4, 5, 6).
#![cfg(feature = "navmesh")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::assert_is_empty,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use dayzmake_core::navmesh::internals::{
    CompactHeightfield, Heightfield, Span, build_compact, dilate_area, erode,
    extract_tree_obstacles, filter_ledges, filter_low_hanging, filter_low_height_multi,
    mark_box_area, mode_filter, remap_and_roof as remap,
};

fn hf(w: i32, h: i32) -> Heightfield {
    Heightfield::new(w, h, [0.0, 0.0, 0.0], [100.0, 100.0, 100.0], 0.25, 0.1).unwrap()
}

fn put(hf: &mut Heightfield, x: i32, z: i32, smin: u16, smax: u16, area: u8, sub: u16) {
    let mut s = Span::new(smin, smax, area);
    s.sub_area = sub;
    let w = hf.width;
    hf.columns_mut()[usize::try_from(x + z * w).unwrap()].push(s);
}

fn flat(w: i32, h: i32, area: u8) -> Heightfield {
    let mut f = hf(w, h);
    for z in 0..h {
        for x in 0..w {
            put(&mut f, x, z, 0, 2, area, 0);
        }
    }
    f
}

fn idx(c: &CompactHeightfield, x: i32, z: i32) -> usize {
    c.cells[usize::try_from(x + z * c.width).unwrap()].0 as usize
}

fn all_but_doors() -> [bool; 64] {
    let mut m = [true; 64];
    m[12] = false;
    m[13] = false;
    m[14] = false;
    m
}

#[test]
fn mode_filter_majority_and_tie_to_lowest() {
    let mut f = hf(23, 23);
    for z in 0..23 {
        for x in 0..23 {
            let inside = (1..=21).contains(&x) && (1..=21).contains(&z);
            let k = (z - 1) * 21 + (x - 1);
            let area = if !inside {
                8
            } else if k < 220 {
                1
            } else {
                8
            };
            put(&mut f, x, z, 0, 2, area, 0);
        }
    }
    mode_filter(&mut f, 11);
    // 220 x area 1 against 221 x area 8 (the centre itself is the 221st area-8 cell or an area-1 cell).
    assert_eq!(f.column(11, 11)[0].area, 8);
    // Border columns keep their value.
    assert_eq!(f.column(0, 0)[0].area, 8);
    assert_eq!(f.column(22, 5)[0].area, 8);

    // Exact tie 220 : 220 plus one zero gives the lowest id.
    let mut g = hf(23, 23);
    for z in 0..23 {
        for x in 0..23 {
            let inside = (1..=21).contains(&x) && (1..=21).contains(&z);
            let k = (z - 1) * 21 + (x - 1);
            let area = if !inside {
                0
            } else if k < 220 {
                8
            } else {
                u8::from(k < 440)
            };
            put(&mut g, x, z, 0, 2, area, 0);
        }
    }
    mode_filter(&mut g, 11);
    assert_eq!(g.column(11, 11)[0].area, 1);

    // An all-zero window gives 0, and the value is written into the lowest span only.
    let mut z0 = flat(23, 23, 0);
    z0.columns_mut()[usize::try_from(11 + 11 * 23).unwrap()].push(Span::new(10, 12, 5));
    mode_filter(&mut z0, 11);
    assert_eq!(z0.column(11, 11)[0].area, 0);
    assert_eq!(z0.column(11, 11)[1].area, 5);

    // No window at borderSize 10.
    let mut small = flat(23, 23, 0);
    small.columns_mut()[usize::try_from(11 + 11 * 23).unwrap()][0].area = 3;
    mode_filter(&mut small, 10);
    assert_eq!(small.column(11, 11)[0].area, 3);
    // A neighbour would take the 3 from its window if the filter ran.
    assert_eq!(small.column(12, 11)[0].area, 0);
}

#[test]
fn low_height_multi() {
    let mut f = hf(8, 1);
    for (i, gap) in [3u16, 8, 15, 4, 16].into_iter().enumerate() {
        let x = i32::try_from(i).unwrap();
        put(&mut f, x, 0, 0, 10, 1, 0);
        put(&mut f, x, 0, 10 + gap, 40, 18, 0);
    }
    // Normal ceiling (area 1): gap 15 gives 0, gap 16 keeps the area.
    put(&mut f, 5, 0, 0, 10, 1, 0);
    put(&mut f, 5, 0, 25, 40, 1, 0);
    put(&mut f, 6, 0, 0, 10, 1, 0);
    put(&mut f, 6, 0, 26, 40, 1, 0);
    // A span that is already area 0 is overwritten under a ceiling.
    put(&mut f, 7, 0, 0, 10, 0, 0);
    put(&mut f, 7, 0, 18, 40, 18, 0);
    filter_low_height_multi(&mut f, [3, 8, 15], [0, 16, 17], 18);
    let a = |x: i32| f.column(x, 0)[0].area;
    assert_eq!([a(0), a(1), a(2)], [0, 16, 17]);
    assert_eq!(a(3), 16); // gap 4
    assert_eq!(a(4), 1); // gap 16 under a ceiling: no entry matches
    assert_eq!(a(5), 0);
    assert_eq!(a(6), 1);
    assert_eq!(a(7), 16);
    // The top span has nothing above it.
    assert_eq!(f.column(0, 0)[1].area, 18);
}

#[test]
fn low_hanging_copies_sub_area() {
    let mut f = hf(2, 1);
    put(&mut f, 0, 0, 0, 5, 1, 7);
    put(&mut f, 0, 0, 6, 9, 0, 0);
    put(&mut f, 0, 0, 10, 12, 0, 0); // does not propagate past the first promoted span
    put(&mut f, 1, 0, 0, 5, 1, 7);
    put(&mut f, 1, 0, 6, 20, 0, 0); // climb too high
    filter_low_hanging(&mut f, 5);
    let c = f.column(0, 0);
    assert_eq!((c[1].area, c[1].sub_area), (1, 7));
    assert_eq!((c[2].area, c[2].sub_area), (0, 0));
    let d = f.column(1, 0);
    assert_eq!((d[1].area, d[1].sub_area), (0, 0));
}

/// The hide part (R21) is not copied along with area and sub area: a promoted span keeps its own.
#[test]
fn low_hanging_does_not_copy_the_hide_part() {
    let mut f = hf(1, 1);
    put(&mut f, 0, 0, 0, 5, 1, 7);
    f.columns_mut()[0][0].hide_part = 3;
    put(&mut f, 0, 0, 6, 9, 0, 0);
    f.columns_mut()[0][1].hide_part = 5;
    put(&mut f, 0, 0, 20, 22, 0, 0);
    filter_low_hanging(&mut f, 5);
    let c = f.column(0, 0);
    assert_eq!((c[1].area, c[1].sub_area, c[1].hide_part), (1, 7, 5));
    assert_eq!((c[0].hide_part, c[2].hide_part), (3, 0));
}

#[test]
fn ledge_skips_terrain() {
    let mut f = hf(2, 1);
    put(&mut f, 0, 0, 0, 10, 1, 0);
    put(&mut f, 1, 0, 0, 10, 8, 0);
    let mut skip = [false; 64];
    skip[0] = true;
    skip[1] = true;
    filter_ledges(&mut f, 15, 5, &skip);
    assert_eq!(f.column(0, 0)[0].area, 1);
    assert_eq!(f.column(1, 0)[0].area, 0);

    let mut g = hf(2, 1);
    put(&mut g, 0, 0, 0, 10, 1, 0);
    put(&mut g, 1, 0, 0, 10, 8, 0);
    let mut only0 = [false; 64];
    only0[0] = true;
    filter_ledges(&mut g, 15, 5, &only0);
    assert_eq!(g.column(0, 0)[0].area, 0);
}

#[test]
fn compact_bmax_adds_prone_height() {
    let mut f = flat(3, 3, 1);
    f.bmax = [1.0, 10.0, 1.0];
    put(&mut f, 1, 1, 20, 22, 4, 9);
    put(&mut f, 0, 0, 5, 6, 0, 0); // unwalkable: not in the compact field
    let c = build_compact(3, 5, &f).unwrap();
    assert_eq!(c.bmax[1], 10.0f32 + 3.0f32 * 0.1f32);
    assert_eq!(c.bmin, f.bmin);
    assert_eq!(
        (c.walkable_height, c.walkable_climb, c.border_size),
        (3, 5, 0)
    );
    assert_eq!(c.spans.len(), 10);
    assert_eq!(c.areas.len(), 10);
    assert_eq!(c.sub_areas.len(), 10);
    assert_eq!(c.cells[0], (0, 1));
    let i = idx(&c, 1, 1);
    assert_eq!(c.cells[usize::try_from(1 + 3).unwrap()].1, 2);
    assert_eq!((c.areas[i + 1], c.sub_areas[i + 1]), (4, 9));
    assert_eq!((c.spans[i].y, c.spans[i].h), (2, 18));
    assert_eq!((c.spans[i + 1].y, c.spans[i + 1].h), (22, 255));
    // The two spans at (1,1) are 18 apart (> climb 5): ground to the +x neighbour only.
    let ground = idx(&c, 1, 1);
    let con = (c.spans[ground].con >> 12) & 0x3f; // dir 2: +x
    assert_eq!(con, 0);
    let high = (c.spans[ground + 1].con >> 12) & 0x3f;
    assert_eq!(high, 0x3f);
    assert_eq!(c.max_regions, 0);
}

#[test]
fn compact_too_many_layers_is_not_connected() {
    let mut f = hf(2, 1);
    for k in 0..70u16 {
        put(&mut f, 0, 0, k * 4, k * 4 + 1, 1, 0);
    }
    // Column 1 has one span level with layer 62 (the highest connectable) and one level with layer 64 (6-bit packing would wrap it to 0).
    put(&mut f, 1, 0, 248, 249, 1, 0);
    put(&mut f, 1, 0, 256, 257, 1, 0);
    let c = build_compact(1, 1, &f).unwrap();
    assert_eq!(c.cells[0].1, 70);
    let first = idx(&c, 1, 0);
    assert_eq!(c.spans[first].con & 0x3f, 62);
    assert_eq!(c.spans[first + 1].con & 0x3f, 0x3f);
}

#[test]
fn sea_box_overwrites_tree_area_keeps_part() {
    let mut f = hf(4, 4);
    f.bmin = [0.0, 0.0, 0.0];
    put(&mut f, 0, 0, 0, 2, 10, 3);
    put(&mut f, 1, 0, 0, 2, 1, 0);
    put(&mut f, 2, 0, 0, 30, 1, 0);
    put(&mut f, 3, 0, 0, 2, 12, 4);
    let mut c = build_compact(3, 5, &f).unwrap();
    mark_box_area(&mut c, [0.0, -500.0, 0.0], [1.0, 1.0, 1.0], 5);
    assert_eq!((c.areas[0], c.sub_areas[0]), (5, 3));
    assert_eq!(c.areas[idx(&c, 1, 0)], 5);
    assert_eq!(c.areas[idx(&c, 2, 0)], 1); // y 30 is above 1.0 / 0.1 = 10
    assert_eq!(c.areas[idx(&c, 3, 0)], 5); // any prior area is overwritten; x 0..=4 is in the box
}

#[test]
fn erode_marks_the_border_ring() {
    let f = flat(7, 7, 1);
    let mut c = build_compact(3, 5, &f).unwrap();
    erode(&mut c, 1).unwrap();
    for z in 0..7 {
        for x in 0..7 {
            let edge = x == 0 || z == 0 || x == 6 || z == 6;
            assert_eq!(c.areas[idx(&c, x, z)], u8::from(!edge), "{x},{z}");
        }
    }
}

#[test]
fn dilate_copies_sub_area_from_nearest() {
    let mut f = flat(9, 3, 1);
    let at = |x: usize, z: usize| x + z * 9;
    for (x, sub) in [(4, 5), (7, 6)] {
        f.columns_mut()[at(x, 1)][0].area = 13;
        f.columns_mut()[at(x, 1)][0].sub_area = sub;
    }
    // A door right next to a seed is not in the mask and keeps its area.
    f.columns_mut()[at(4, 2)][0].area = 12;
    let mut c = build_compact(3, 5, &f).unwrap();
    dilate_area(&mut c, 2, 13, &all_but_doors());
    for (x, z, area, sub) in [
        (4, 1, 13, 5),
        (3, 1, 13, 5),
        (2, 1, 13, 5),
        (5, 1, 13, 5),
        (4, 0, 13, 5),
        (3, 2, 13, 5),
        (4, 2, 12, 0),
        (6, 1, 13, 6), // 2 from the seed with part 6, 4 from the one with part 5
        (8, 1, 13, 6),
        (7, 1, 13, 6),
        (1, 1, 1, 0),
        (0, 1, 1, 0),
    ] {
        let i = idx(&c, x, z);
        assert_eq!((c.areas[i], c.sub_areas[i]), (area, sub), "{x},{z}");
    }
}

fn tree_world(wall: Option<(i32, u8)>) -> CompactHeightfield {
    let mut f = flat(40, 40, 1);
    f.bmin = [100.0, 3.0, 200.0];
    for z in 20..22 {
        for x in 20..22 {
            let i = usize::try_from(x + z * 40).unwrap();
            f.columns_mut()[i][0].area = 10;
        }
    }
    // A wall of door spans (area 12, a labelled seed), or area 0: such spans never enter the
    // compact field, so the wall is a line of columns without a span and shows as a missing connection.
    if let Some((wx, area)) = wall {
        for z in 0..40 {
            let i = usize::try_from(wx + z * 40).unwrap();
            if area == 0 {
                f.columns_mut()[i].clear();
            } else {
                f.columns_mut()[i][0].area = area;
            }
        }
    }
    build_compact(3, 5, &f).unwrap()
}

fn seeds() -> [bool; 64] {
    let mut s = [false; 64];
    for a in [0, 10, 12, 13, 14] {
        s[a] = true;
    }
    s
}

#[test]
fn tree_obstacle_isolated_becomes_walkable_with_record() {
    let mut c = tree_world(None);
    let obs = extract_tree_obstacles(&mut c, 10, 1, 1, &seeds(), 2.0, 3.0);
    assert_eq!(obs.len(), 1);
    let cs = 0.25f32;
    let ch = 0.1f32;
    let (count, sum_x, sum_y, sum_z) = (4.0f32, 82.0f32, 8.0f32, 82.0f32);
    let b = 0.0f32;
    let footprint = 2.0f32;
    assert_eq!(obs[0].pos[0], (sum_x / count + b + 0.5f32) * cs + 100.0f32);
    assert_eq!(obs[0].pos[1], (sum_y / count + b + 0.5f32) * ch + 3.0f32);
    assert_eq!(obs[0].pos[2], (sum_z / count + b + 0.5f32) * cs + 200.0f32);
    assert_eq!(obs[0].radius, footprint * cs * 0.5f32);
    for z in 20..22 {
        for x in 20..22 {
            let i = idx(&c, x, z);
            assert_eq!((c.areas[i], c.sub_areas[i]), (1, 0));
        }
    }
}

#[test]
fn tree_near_wall_becomes_hole() {
    let mut c = tree_world(Some((25, 12)));
    let obs = extract_tree_obstacles(&mut c, 10, 1, 1, &seeds(), 2.0, 3.0);
    assert!(obs.is_empty());
    for z in 20..22 {
        for x in 20..22 {
            assert_eq!(c.areas[idx(&c, x, z)], 0);
        }
    }
}

#[test]
fn tree_near_missing_wall_becomes_hole() {
    // The "no connection: candidate is the own distance" branch.
    let mut c = tree_world(Some((25, 0)));
    let obs = extract_tree_obstacles(&mut c, 10, 1, 1, &seeds(), 2.0, 3.0);
    assert!(obs.is_empty());
    for z in 20..22 {
        for x in 20..22 {
            assert_eq!(c.areas[idx(&c, x, z)], 0);
        }
    }
}

#[test]
fn tree_too_big_footprint_becomes_hole() {
    let mut f = flat(60, 60, 1);
    for z in 20..40 {
        for x in 20..40 {
            let i = usize::try_from(x + z * 60).unwrap();
            f.columns_mut()[i][0].area = 10;
        }
    }
    let mut c = build_compact(3, 5, &f).unwrap();
    let obs = extract_tree_obstacles(&mut c, 10, 1, 1, &seeds(), 2.0, 3.0);
    assert!(obs.is_empty());
    assert_eq!(c.areas[idx(&c, 25, 25)], 0);
}

#[test]
fn remap_and_roof() {
    let mut f = hf(5, 1);
    put(&mut f, 0, 0, 0, 2, 7, 0);
    put(&mut f, 1, 0, 0, 2, 9, 0);
    put(&mut f, 2, 0, 0, 2, 11, 0);
    put(&mut f, 2, 0, 40, 42, 1, 0);
    put(&mut f, 3, 0, 0, 2, 8, 0);
    put(&mut f, 4, 0, 0, 2, 8, 0);
    let mut c = build_compact(3, 5, &f).unwrap();
    let k = idx(&c, 4, 0);
    c.spans[k].h = 100;
    remap(&mut c);
    assert_eq!(c.areas[idx(&c, 0, 0)], 1);
    assert_eq!(c.areas[idx(&c, 1, 0)], 1);
    assert_eq!(c.areas[idx(&c, 2, 0)], 8); // 11 becomes 8, not topmost
    assert_eq!(c.areas[idx(&c, 2, 0) + 1], 1);
    assert_eq!(c.areas[idx(&c, 3, 0)], 1); // topmost area 8, h 255
    assert_eq!(c.areas[k], 8); // h != 255 keeps 8
}
