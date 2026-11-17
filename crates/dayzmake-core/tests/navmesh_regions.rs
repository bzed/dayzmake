//! Distance field and region building with DayZ part ids (spec 5.4 step 8, 6.2 step 1, 10.1 items 5, 6).
#![cfg(feature = "navmesh")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use dayzmake_core::navmesh::internals::{
    CompactHeightfield, Heightfield, Span, build_compact, build_distance_field, build_regions,
    build_regions_monotone,
};

const BORDER_REG: u16 = 0x8000;

/// A flat field of `w` x `h` columns; `part(x, z)` gives `(area, sub_area)`.
fn field(w: i32, h: i32, part: impl Fn(i32, i32) -> (u8, u16)) -> CompactHeightfield {
    let mut hf = Heightfield::new(w, h, [0.0, 0.0, 0.0], [100.0, 100.0, 100.0], 0.25, 0.1).unwrap();
    for z in 0..h {
        for x in 0..w {
            let (area, sub) = part(x, z);
            let mut s = Span::new(0, 2, area);
            s.sub_area = sub;
            hf.columns_mut()[usize::try_from(x + z * w).unwrap()].push(s);
        }
    }
    build_compact(4, 2, &hf).unwrap()
}

fn at(c: &CompactHeightfield, x: i32, z: i32) -> usize {
    c.cells[usize::try_from(x + z * c.width).unwrap()].0 as usize
}

fn reg(c: &CompactHeightfield, x: i32, z: i32) -> u16 {
    c.spans[at(c, x, z)].reg
}

/// Default minimum area 16, ids 10, 12, 13, 14 `INT_MAX` (spec 4).
fn mins() -> [i32; 64] {
    let mut m = [16; 64];
    for a in [10, 12, 13, 14] {
        m[a] = i32::MAX;
    }
    m
}

fn distinct(c: &CompactHeightfield) -> Vec<u16> {
    let mut v: Vec<u16> = c.spans.iter().map(|s| s.reg).collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn regions_split_at_sub_area_change() {
    // Watershed: the two halves never become one region, with merging off and on.
    for merge in [0, 10_000] {
        let mut c = field(20, 10, |x, _| (1, u16::from(x >= 10)));
        build_distance_field(&mut c).unwrap();
        build_regions(&mut c, 0, &[0; 64], merge).unwrap();
        assert_eq!(distinct(&c).len(), 2, "merge {merge}");
        // Like upstream, `max_regions` counts the unused id past the last region too: 2 + 1.
        assert_eq!(c.max_regions, 3);
        let left = reg(&c, 2, 5);
        let right = reg(&c, 17, 5);
        assert!(left != 0 && right != 0 && left != right);
        for z in 0..10 {
            for x in 0..20 {
                assert_eq!(reg(&c, x, z), if x < 10 { left } else { right });
            }
        }
    }
}

#[test]
fn regions_split_at_sub_area_change_flood_only() {
    // Merging is off, so only flood and expand keep the halves apart.
    let mut c = field(20, 10, |x, _| (1, u16::from(x >= 10)));
    build_distance_field(&mut c).unwrap();
    build_regions(&mut c, 0, &[0; 64], 0).unwrap();
    assert_ne!(reg(&c, 2, 5), reg(&c, 17, 5));
}

#[test]
fn distance_field_boundary_at_sub_area_change() {
    let mut c = field(9, 9, |x, _| (1, u16::from(x >= 5)));
    build_distance_field(&mut c).unwrap();
    // The seam is a boundary; without the part rule (4,4) would be 8 from any wall.
    assert_eq!(c.dist[at(&c, 4, 4)], 0);
    assert_eq!(c.dist[at(&c, 5, 4)], 0);
    // Away from the seam and the walls the field grows again.
    let mut flat = field(9, 9, |_, _| (1, 0));
    build_distance_field(&mut flat).unwrap();
    assert!(flat.dist[at(&flat, 4, 4)] > 0);
    assert_eq!(flat.max_distance, 8);
}

#[test]
fn monotone_stops_at_sub_area() {
    let mut c = field(8, 3, |x, _| (1, u16::from(x >= 4)));
    build_distance_field(&mut c).unwrap();
    build_regions_monotone(&mut c, 0, &[0; 64], 10_000).unwrap();
    assert_eq!(distinct(&c).len(), 2);
    assert_ne!(reg(&c, 1, 1), reg(&c, 6, 1));
    assert_eq!(reg(&c, 0, 0), reg(&c, 3, 2));
}

#[test]
fn min_area_uses_first_region_of_group() {
    // Two connected regions of 8 spans each (16 in total, the default minimum of area 1 is 16).
    // The lowest-numbered region decides: a door (minimum INT_MAX) drops the whole group, an
    // area 1 region (16 < 16 is false) keeps it, whatever area the other region has.
    let door_first = |x: i32, _| (if x < 4 { 12 } else { 1 }, 0);
    let door_last = |x: i32, _| (if x < 4 { 1 } else { 12 }, 0);

    let mut c = field(8, 2, door_first);
    build_distance_field(&mut c).unwrap();
    build_regions_monotone(&mut c, 0, &mins(), 0).unwrap();
    assert!(
        c.spans.iter().all(|s| s.reg == 0),
        "door first drops the group"
    );

    let mut c = field(8, 2, door_last);
    build_distance_field(&mut c).unwrap();
    build_regions_monotone(&mut c, 0, &mins(), 0).unwrap();
    assert_eq!(distinct(&c).len(), 2, "area 1 first keeps the group");
    assert_eq!(c.max_regions, 3); // two regions plus the unused id, as upstream
}

#[test]
fn min_area_removes_small_group_of_area_one() {
    // 15 spans < 16 are removed, 16 are kept.
    let mut c = field(5, 3, |_, _| (1, 0));
    build_distance_field(&mut c).unwrap();
    build_regions(&mut c, 0, &mins(), 0).unwrap();
    assert!(c.spans.iter().all(|s| s.reg == 0));

    let mut c = field(4, 4, |_, _| (1, 0));
    build_distance_field(&mut c).unwrap();
    build_regions(&mut c, 0, &mins(), 0).unwrap();
    assert!(c.spans.iter().all(|s| s.reg == 1));
    assert_eq!(c.max_regions, 2); // one region plus the unused id, as upstream
}

#[test]
fn border_regions_flagged() {
    for monotone in [false, true] {
        let mut c = field(10, 10, |_, _| (1, 0));
        build_distance_field(&mut c).unwrap();
        if monotone {
            build_regions_monotone(&mut c, 2, &[0; 64], 0).unwrap();
        } else {
            build_regions(&mut c, 2, &[0; 64], 0).unwrap();
        }
        assert_eq!(c.border_size, 2);
        for z in 0..10 {
            for x in 0..10 {
                let border = !(2..8).contains(&x) || !(2..8).contains(&z);
                let r = reg(&c, x, z);
                assert_eq!(r & BORDER_REG != 0, border, "({x},{z}) monotone {monotone}");
                if !border {
                    assert_eq!(r, reg(&c, 5, 5));
                    assert_ne!(r, 0);
                }
            }
        }
        // Four border slots (remapped like upstream) + one region + the unused id.
        assert_eq!(c.max_regions, 6);
    }
}

#[test]
fn watershed_splits_two_rooms_joined_by_a_neck() {
    // Two 9x9 rooms joined by a one-cell neck: the distance field separates them.
    let mut c = field(21, 9, |x, z| {
        let room = !(9..12).contains(&x);
        let neck = (9..12).contains(&x) && z == 4;
        (u8::from(room || neck), 0)
    });
    build_distance_field(&mut c).unwrap();
    build_regions(&mut c, 0, &[0; 64], 0).unwrap();
    assert_ne!(reg(&c, 4, 4), 0);
    assert_ne!(reg(&c, 4, 4), reg(&c, 16, 4));
}

#[test]
fn negative_border_and_missing_distance_field_are_errors() {
    let mut c = field(4, 4, |_, _| (1, 0));
    assert!(
        build_regions(&mut c, 0, &[0; 64], 0).is_err(),
        "no distance field yet"
    );
    build_distance_field(&mut c).unwrap();
    assert!(build_regions(&mut c, -1, &[0; 64], 0).is_err());
    assert!(build_regions_monotone(&mut c, -1, &[0; 64], 0).is_err());
}

type Cell = (i32, i32);

/// Runs the watershed with merging off and returns whether `(x, z)` pairs are in distinct regions.
fn parts_stay_apart(
    w: i32,
    h: i32,
    part: impl Fn(i32, i32) -> (u8, u16),
    pairs: &[(Cell, Cell)],
) -> bool {
    let mut c = field(w, h, part);
    build_distance_field(&mut c).unwrap();
    build_regions(&mut c, 0, &[0; 64], 0).unwrap();
    pairs
        .iter()
        .all(|&((ax, az), (bx, bz))| reg(&c, ax, az) != reg(&c, bx, bz))
}

#[test]
fn watershed_keeps_thin_parts_apart() {
    // Thin field: the distance field is shallow, so flooding reaches every cell at level 0.
    assert!(parts_stay_apart(
        10,
        3,
        |x, _| (1, u16::from(x >= 5)),
        &[((1, 1), (8, 1))]
    ));
    // A narrow strip beside a wide room.
    assert!(parts_stay_apart(
        20,
        10,
        |x, _| (1, u16::from(x >= 18)),
        &[((5, 5), (19, 5))]
    ));
    // A single cell of another part inside a room.
    assert!(parts_stay_apart(
        9,
        9,
        |x, z| (1, u16::from(x == 4 && z == 4)),
        &[((1, 1), (4, 4))]
    ));
    // Same with an area change instead of a part id (stock behaviour, sanity).
    assert!(parts_stay_apart(
        9,
        9,
        |x, z| (if x == 4 && z == 4 { 2 } else { 1 }, 0),
        &[((1, 1), (4, 4))]
    ));
}
