//! Contours with DayZ part ids and the vertical error (spec 5.4 step 9, 6.2 step 1, 10.1 item 7).
#![cfg(feature = "navmesh")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use dayzmake_core::navmesh::internals::{
    CompactHeightfield, ContourSet, Heightfield, RC_AREA_BORDER, RC_BORDER_VERTEX,
    RC_CONTOUR_TESS_WALL_EDGES, Span, build_compact, build_contours, build_distance_field,
    build_regions,
};

/// A `w` x `h` field; `part(x, z)` gives `(area, sub_area, top)`, the span covers `0..top`.
fn field(w: i32, h: i32, part: impl Fn(i32, i32) -> (u8, u16, u16)) -> CompactHeightfield {
    let mut hf = Heightfield::new(w, h, [0.0, 0.0, 0.0], [100.0, 100.0, 100.0], 0.25, 0.1).unwrap();
    for z in 0..h {
        for x in 0..w {
            let (area, sub, top) = part(x, z);
            if area == 0 {
                continue;
            }
            let mut s = Span::new(0, top, area);
            s.sub_area = sub;
            hf.columns_mut()[usize::try_from(x + z * w).unwrap()].push(s);
        }
    }
    build_compact(4, 100, &hf).unwrap()
}

fn flat(w: i32, h: i32) -> CompactHeightfield {
    field(w, h, |_, _| (1, 0, 2))
}

fn regions(c: &mut CompactHeightfield, border: i32) {
    build_distance_field(c).unwrap();
    build_regions(c, border, &[0; 64], 10_000).unwrap();
}

fn contours(c: &CompactHeightfield, vert_err: f32, edge_len: i32, flags: i32) -> ContourSet {
    build_contours(c, 1.3, vert_err, edge_len, flags).unwrap()
}

#[test]
fn square_region_gives_4_vertex_contour() {
    let mut c = flat(20, 20);
    regions(&mut c, 0);
    let set = contours(&c, 8.0, 0, 0);
    assert_eq!(set.contours.len(), 1);
    let k = &set.contours[0];
    assert_eq!(k.verts.len(), 4);
    assert_eq!(k.area, 1);
    assert_eq!(k.sub_area, 0);
    assert_eq!(k.reg, c.spans[0].reg);
    // 4 * 20 raw edge points.
    assert_eq!(k.rverts.len(), 80);
    let mut xz: Vec<(i32, i32)> = k.verts.iter().map(|v| (v[0], v[2])).collect();
    xz.sort_unstable();
    assert_eq!(xz, [(0, 0), (0, 20), (20, 0), (20, 20)]);
    assert_eq!((set.width, set.height, set.border_size), (20, 20, 0));
    assert!((set.max_error - 1.3).abs() < 1e-6);
}

#[test]
fn no_regions_give_no_contours() {
    let mut c = field(10, 10, |_, _| (0, 0, 2));
    regions(&mut c, 0);
    assert_eq!(contours(&c, 8.0, 0, 0).contours.len(), 0);
}

#[test]
fn vertical_error_inserts_vertex() {
    // One edge rises 1.5 m (15 voxels) in the middle of the field.
    let mut c = field(20, 20, |x, z| {
        (
            1,
            0,
            if z == 0 && (8..12).contains(&x) {
                17
            } else {
                2
            },
        )
    });
    regions(&mut c, 0);
    let off = contours(&c, 0.0, 0, 0);
    assert_eq!(off.contours[0].verts.len(), 4);
    assert!(
        off.contours[0]
            .rverts
            .iter()
            .all(|v| v[3] & (RC_BORDER_VERTEX | RC_AREA_BORDER) == 0)
    );

    let on = contours(&c, 8.0, 0, 0);
    let k = &on.contours[0];
    assert!(k.verts.len() > 4, "{:?}", k.verts);
    // The raw point of the inserted vertex carries both tags.
    let tagged: Vec<_> = k
        .rverts
        .iter()
        .filter(|v| {
            v[3] & (RC_BORDER_VERTEX | RC_AREA_BORDER) == (RC_BORDER_VERTEX | RC_AREA_BORDER)
        })
        .collect();
    // The four corners of the bump, none of them within the vertical error of its neighbours.
    assert_eq!(tagged.len(), 4, "{:?}", k.rverts);
    assert_eq!(k.verts.len(), 8, "{:?}", k.verts);
    // ... and it reaches the simplified vertices through the final flag copy.
    assert!(
        k.verts
            .iter()
            .any(|v| v[1] == 17 && v[3] & RC_BORDER_VERTEX != 0)
    );
    // A bump below the limit does not insert anything.
    let mut low = field(20, 20, |x, z| {
        (1, 0, if z == 0 && (8..12).contains(&x) { 8 } else { 2 })
    });
    regions(&mut low, 0);
    assert_eq!(contours(&low, 8.0, 0, 0).contours[0].verts.len(), 4);
}

#[test]
fn area_border_respects_sub_area() {
    let mut c = field(20, 10, |x, _| (1, u16::from(x >= 10), 2));
    regions(&mut c, 0);
    let set = contours(&c, 8.0, 0, 0);
    assert_eq!(set.contours.len(), 2);
    for k in &set.contours {
        // The shared edge x = 10 is a portal: raw points with RC_AREA_BORDER and a neighbour region.
        let flagged: Vec<_> = k
            .rverts
            .iter()
            .filter(|v| v[3] & RC_AREA_BORDER != 0)
            .collect();
        assert_eq!(flagged.len(), 10, "{flagged:?}");
        assert!(flagged.iter().all(|v| v[0] == 10 && v[3] & 0xffff != 0));
        let right_half = k.rverts.iter().any(|v| v[0] > 10);
        assert_eq!(k.sub_area, u16::from(right_half));
    }
}

#[test]
fn corner_between_parts_is_not_a_border_vertex() {
    // A 14 x 14 field with a 2 cell tile border; the interior is split by part id at x = 7. The
    // vertex (7, 2) has two equal border cells below and two interior cells of the same area but
    // different parts above, so it is not a border vertex (spec 10.1 item 7).
    let mut c = field(14, 14, |x, _| (1, u16::from(x >= 7), 2));
    regions(&mut c, 2);
    let set = contours(&c, 8.0, 0, 0);
    assert_eq!(set.contours.len(), 2);
    let all: Vec<_> = set.contours.iter().flat_map(|k| k.rverts.iter()).collect();
    // The corner (7, 2) is (5, 0) after the border shift.
    assert!(all.iter().any(|v| v[0] == 5 && v[2] == 0));
    assert!(
        all.iter()
            .filter(|v| v[0] == 5 && v[2] == 0)
            .all(|v| v[3] & RC_BORDER_VERTEX == 0),
        "{all:?}"
    );
    // Control: the corner of the tile border itself, with equal parts, is a border vertex.
    assert!(all.iter().any(|v| v[3] & RC_BORDER_VERTEX != 0));
}

#[test]
fn border_shifts_bounds_and_vertices() {
    let mut c = flat(14, 14);
    regions(&mut c, 2);
    let set = contours(&c, 8.0, 0, 0);
    assert_eq!((set.width, set.height, set.border_size), (10, 10, 2));
    assert!((set.bmin[0] - 0.5).abs() < 1e-6 && (set.bmin[2] - 0.5).abs() < 1e-6);
    assert!((set.bmax[0] - 99.5).abs() < 1e-4);
    assert_eq!(set.contours.len(), 1);
    let mut xz: Vec<(i32, i32)> = set.contours[0].verts.iter().map(|v| (v[0], v[2])).collect();
    xz.sort_unstable();
    assert_eq!(xz, [(0, 0), (0, 10), (10, 0), (10, 10)]);
}

#[test]
fn max_edge_len_splits_long_wall_edges() {
    let mut c = flat(60, 10);
    regions(&mut c, 0);
    assert_eq!(
        contours(&c, 8.0, 0, RC_CONTOUR_TESS_WALL_EDGES).contours[0]
            .verts
            .len(),
        4
    );
    // The tessellation flag is needed too.
    assert_eq!(contours(&c, 8.0, 20, 0).contours[0].verts.len(), 4);
    let k = &contours(&c, 8.0, 20, RC_CONTOUR_TESS_WALL_EDGES).contours[0];
    assert!(k.verts.len() > 4);
    for (j, v) in k.verts.iter().enumerate() {
        let n = &k.verts[(j + 1) % k.verts.len()];
        let (dx, dz) = (n[0] - v[0], n[2] - v[2]);
        assert!(dx * dx + dz * dz <= 400, "edge {v:?} -> {n:?}");
    }
}

#[test]
fn region_with_hole_is_merged() {
    // A ring: a 30 x 30 field with the middle 10 x 10 cut out.
    let mut c = field(30, 30, |x, z| {
        let hole = (10..20).contains(&x) && (10..20).contains(&z);
        (u8::from(!hole), 0, 2)
    });
    // Watershed would split the ring in two (they could only merge across two borders); give it
    // one region by hand.
    for s in &mut c.spans {
        s.reg = 1;
    }
    c.max_regions = 2;
    let set = contours(&c, 8.0, 0, 0);
    assert_eq!(set.contours.len(), 1);
    // 4 outer corners, 4 hole corners and the two ends of the bridge.
    let k = &set.contours[0];
    assert_eq!(k.verts.len(), 10, "{:?}", k.verts);
    // The raw contour is the outline only.
    assert_eq!(k.rverts.len(), 120);
    // The hole corners are part of the merged contour.
    for corner in [(10, 10), (10, 20), (20, 10), (20, 20)] {
        assert!(k.verts.iter().any(|v| (v[0], v[2]) == corner), "{corner:?}");
    }
}

#[test]
fn vertical_error_applies_along_tile_borders() {
    // The interior edge facing a tile border has a bump; its raw points are border vertices of a
    // contour whose neighbour is a border region, and the gate of the pass accepts them.
    let mut c = field(24, 24, |x, z| {
        (
            1,
            0,
            if z == 2 && (10..14).contains(&x) {
                17
            } else {
                2
            },
        )
    });
    regions(&mut c, 2);
    let on = contours(&c, 8.0, 0, 0);
    assert_eq!(on.contours.len(), 1);
    assert!(
        on.contours[0]
            .rverts
            .iter()
            .any(|v| v[3] & RC_BORDER_VERTEX != 0)
    );
    let off = contours(&c, 0.0, 0, 0);
    assert!(on.contours[0].verts.len() > off.contours[0].verts.len());
}

#[test]
fn area_change_along_an_edge_keeps_a_vertex() {
    // Region 1 (area 1) borders region 2 along x = 10; region 2 changes from area 1 to area 2 half
    // way. Real regions never do that, so the regions are set by hand. The initial points of the
    // simplification include every place where RC_AREA_BORDER changes, so the straight edge keeps a
    // vertex where the neighbour's area changes.
    let mut c = field(20, 6, |x, z| (if x >= 10 && z >= 3 { 2 } else { 1 }, 0, 2));
    for z in 0..6 {
        for x in 0..20 {
            let i = usize::try_from(c.cells[usize::try_from(x + z * 20).unwrap()].0).unwrap();
            c.spans[i].reg = if x < 10 { 1 } else { 2 };
        }
    }
    c.max_regions = 3;
    let set = contours(&c, 8.0, 0, 0);
    let k = set.contours.iter().find(|k| k.reg == 1).unwrap();
    assert!(
        k.verts.iter().any(|v| v[0] == 10 && v[2] == 3),
        "{:?}",
        k.verts
    );
}

#[test]
fn region_with_several_holes_is_merged() {
    // Eight small holes in one region: every merge grows the outline by the hole plus two bridge
    // vertices, so the scratch buffer of candidate diagonals must have room for the grown outline.
    let holes: Vec<(i32, i32)> = (0..8)
        .map(|k| (4 + 8 * (k % 4), 6 + 14 * (k / 4)))
        .collect();
    let mut c = field(40, 30, |x, z| {
        let hole = holes
            .iter()
            .any(|&(hx, hz)| (hx..hx + 2).contains(&x) && (hz..hz + 2).contains(&z));
        (u8::from(!hole), 0, 2)
    });
    for s in &mut c.spans {
        s.reg = 1;
    }
    c.max_regions = 2;
    let set = contours(&c, 8.0, 0, 0);
    assert_eq!(set.contours.len(), 1);
    let k = &set.contours[0];
    // 4 outer corners, 4 corners and 2 bridge vertices per hole.
    assert_eq!(k.verts.len(), 4 + 8 * (4 + 2), "{:?}", k.verts);
    for &(hx, hz) in &holes {
        for corner in [(hx, hz), (hx, hz + 2), (hx + 2, hz), (hx + 2, hz + 2)] {
            assert!(k.verts.iter().any(|v| (v[0], v[2]) == corner), "{corner:?}");
        }
    }
}
