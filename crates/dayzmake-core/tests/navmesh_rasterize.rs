//! The Recast heightfield and the DayZ five-mode rasteriser (spec 5.2, 5.3, 10.1).
//!
//! Heightfields are tiny: cell size 1 and cell height 1 (unless noted), so spans are integers that
//! can be computed by hand.
#![cfg(feature = "navmesh")]
#![expect(
    clippy::unwrap_used,
    clippy::assert_is_empty,
    clippy::field_reassign_with_default,
    clippy::many_single_char_names,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use dayzmake_core::navmesh::internals::{
    Heightfield, RasterMode, RasterOutcome, RasterParams, Span, rasterize_part, rasterize_triangle,
    walkable,
};

fn hf4() -> Heightfield {
    Heightfield::new(4, 4, [0.0, 0.0, 0.0], [4.0, 100.0, 4.0], 1.0, 1.0).unwrap()
}

fn params(mode: RasterMode, area: u8) -> RasterParams {
    let mut p = RasterParams::default();
    p.mode = mode;
    p.area = area;
    p
}

fn span(smin: u16, smax: u16, area: u8) -> Span {
    Span::new(smin, smax, area)
}

/// `s` as a span of hide part `part` (ruling R21).
fn in_part(mut s: Span, part: u16) -> Span {
    s.hide_part = part;
    s
}

/// A vertical sliver inside cell `(x, 0)` covering heights `y0..y1`.
fn wall(x: f32, y0: f32, y1: f32) -> [[f32; 3]; 3] {
    [[x + 0.2, y0, 0.5], [x + 0.8, y0, 0.5], [x + 0.5, y1, 0.5]]
}

/// A flat triangle at height `y` inside cell `(x, 0)`.
fn flat(x: f32, y: f32) -> [[f32; 3]; 3] {
    [[x + 0.2, y, 0.2], [x + 0.8, y, 0.2], [x + 0.2, y, 0.8]]
}

fn raster(hf: &mut Heightfield, t: &[[f32; 3]; 3], p: &RasterParams) -> RasterOutcome {
    rasterize_triangle(hf, t[0], t[1], t[2], p)
}

#[test]
fn add_merges_overlapping_spans() {
    let mut hf = hf4();
    hf.add_span(0, 0, 20, 22, 7, 1);
    hf.add_span(0, 0, 0, 5, 5, 1);
    // Sorted ascending even though inserted out of order.
    assert_eq!(hf.column(0, 0), &[span(0, 5, 5), span(20, 22, 7)]);

    // New smax 6 is within the threshold of the old smax 5: the area is max(2, 5).
    hf.add_span(0, 0, 3, 6, 2, 1);
    assert_eq!(hf.column(0, 0), &[span(0, 6, 5), span(20, 22, 7)]);

    // Far above the old smax: the new area wins, even though it is lower.
    let mut hf = hf4();
    hf.add_span(1, 1, 0, 5, 5, 1);
    hf.add_span(1, 1, 3, 9, 2, 1);
    assert_eq!(hf.column(1, 1), &[span(0, 9, 2)]);

    // A span that bridges two spans merges both.
    let mut hf = hf4();
    hf.add_span(2, 2, 0, 3, 1, 1);
    hf.add_span(2, 2, 10, 13, 1, 1);
    hf.add_span(2, 2, 2, 11, 4, 1);
    assert_eq!(hf.column(2, 2), &[span(0, 13, 4)]);
}

#[test]
fn merged_span_does_not_inherit_sub_area() {
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    hf.columns_mut()[0][0].sub_area = 7;
    hf.add_span(0, 0, 2, 5, 1, 1);
    assert_eq!(hf.column(0, 0).len(), 1);
    assert_eq!(hf.column(0, 0)[0].sub_area, 0);
}

#[test]
fn flat_quad_one_span_per_cell() {
    let mut hf = hf4();
    let p = params(RasterMode::Add, 63);
    // A quad over x, z in [0.5, 1.5] at height 2: cells 0 and 1 in both axes.
    let (a, b, c, d) = (
        [0.5, 2.0, 0.5],
        [1.5, 2.0, 0.5],
        [1.5, 2.0, 1.5],
        [0.5, 2.0, 1.5],
    );
    assert_eq!(
        rasterize_triangle(&mut hf, a, c, b, &p),
        RasterOutcome::Done
    );
    assert_eq!(
        rasterize_triangle(&mut hf, a, d, c, &p),
        RasterOutcome::Done
    );
    for z in 0..4 {
        for x in 0..4 {
            let col = hf.column(x, z);
            if x < 2 && z < 2 {
                // floor(2.0) = 2; ceil(2.0) = 2 is raised to smin + 1.
                assert_eq!(col, &[span(2, 3, 63)], "cell {x},{z}");
            } else {
                assert!(col.is_empty(), "cell {x},{z}");
            }
        }
    }
}

#[test]
fn steep_triangle_rasterised_as_null_area() {
    let thr = 0.707_106_77_f32;
    let mut hf = hf4();
    // A vertical wall in cell (0,0); a ramp-free floor triangle in cell (2,0).
    let steep = [[0.2, 0.0, 0.5], [0.8, 0.0, 0.5], [0.5, 5.0, 0.5]];
    let floor = [[2.2, 1.0, 0.2], [2.8, 1.0, 0.2], [2.2, 1.0, 0.8]];
    let mut put = |t: &[[f32; 3]; 3]| {
        // The server's winding: walkability on (v0, v1, v2), rasterise (v0, v2, v1).
        let area = if walkable(thr, t[0], t[1], t[2]) {
            63
        } else {
            0
        };
        rasterize_triangle(&mut hf, t[0], t[2], t[1], &params(RasterMode::Add, area));
        area
    };
    assert_eq!(put(&steep), 0);
    assert_eq!(put(&floor), 63);
    assert_eq!(hf.column(0, 0), &[span(0, 5, 0)]);
    assert_eq!(hf.column(2, 0), &[span(1, 2, 63)]);
}

#[test]
fn slope_threshold_45_deg() {
    let thr = ((45.0f32 / 180.0) * std::f32::consts::PI).cos();
    assert_eq!(thr.to_bits(), 0.707_106_77_f32.to_bits());
    let tilted = |deg: f32| {
        let r = (deg / 180.0) * std::f32::consts::PI;
        ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, r.sin(), r.cos()])
    };
    let (a, b, c) = tilted(44.0);
    assert!(walkable(thr, a, b, c));
    let (a, b, c) = tilted(46.0);
    assert!(!walkable(thr, a, b, c));
    // Flat, the server winding: normal up. The opposite winding faces down.
    assert!(walkable(thr, [0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]));
    assert!(!walkable(thr, [0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]));
    // A degenerate triangle: d is not above FLT_MIN, n.y stays 0, never walkable.
    assert!(!walkable(thr, [1.0; 3], [1.0; 3], [1.0; 3]));
}

#[test]
fn paint_below_repeats() {
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    let mut p = params(RasterMode::PaintBelow, 20);
    p.sub_area = 3;
    let door = wall(0.0, 0.0, 8.0);
    assert_eq!(raster(&mut hf, &door, &p), RasterOutcome::Done);
    assert_eq!(
        (hf.column(0, 0)[0].area, hf.column(0, 0)[0].sub_area),
        (20, 3)
    );
    // The same (area, sub) again: repeat.
    assert_eq!(raster(&mut hf, &door, &p), RasterOutcome::Repeat);

    // A repeat in the first cell aborts the remaining cells of the triangle.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 20, 1);
    hf.columns_mut()[0][0].sub_area = 3;
    hf.add_span(1, 0, 0, 5, 1, 1);
    let two = [[0.2, 0.0, 0.5], [1.8, 0.0, 0.5], [1.0, 8.0, 0.5]];
    assert_eq!(raster(&mut hf, &two, &p), RasterOutcome::Repeat);
    assert_eq!(hf.column(1, 0)[0].area, 1);

    // Unwalkable ground (area 0) is not painted, and a piece below the top is a stop.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 0, 1);
    assert_eq!(raster(&mut hf, &door, &p), RasterOutcome::Done);
    assert_eq!(hf.column(0, 0)[0].area, 0);
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 20, 1, 1);
    assert_eq!(raster(&mut hf, &door, &p), RasterOutcome::Done);
    assert_eq!(hf.column(0, 0)[0].area, 1);
}

#[test]
fn tree_paints_ground_not_adds() {
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    let mut p = params(RasterMode::Tree, 10);
    p.sub_area = 7;
    p.walkable_climb = 5;
    p.walkable_height = 15;
    let trunk = wall(0.0, 5.0, 30.0);
    assert_eq!(raster(&mut hf, &trunk, &p), RasterOutcome::Done);
    assert_eq!(hf.column(0, 0).len(), 1);
    assert_eq!(hf.column(0, 0)[0].smax, 5);
    assert_eq!(
        (hf.column(0, 0)[0].area, hf.column(0, 0)[0].sub_area),
        (10, 7)
    );

    // Nothing to paint: nothing is added.
    let mut hf = hf4();
    raster(&mut hf, &trunk, &p);
    assert!(hf.column(0, 0).is_empty());

    // A piece reaching only to ground + climb is a stop: no paint.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 5.0, 10.0), &p);
    assert_eq!(hf.column(0, 0)[0].area, 1);

    // Floating well above the ground (smin >= s.smax + walkableHeight): not a hit.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 25.0, 40.0), &p);
    assert_eq!(hf.column(0, 0)[0].area, 1);
}

#[test]
fn water_upgrades_area_only_when_deep_enough() {
    let mut hf = hf4();
    let mut p = params(RasterMode::Water, 9);
    p.walkable_height = 15;
    p.water_depth = 5;
    // Water surface at height 10: piece [10, 11].
    hf.add_span(0, 0, 0, 2, 1, 1); // 0 + 5 <= 10: upgraded to max(1, 9)
    hf.add_span(1, 0, 8, 9, 1, 1); // 8 + 5 > 10: left alone
    hf.add_span(2, 0, 0, 2, 0, 1); // unwalkable ground also becomes water
    hf.add_span(3, 0, 12, 14, 1, 1); // piece smax 11 <= s.smin 12: stop
    for x in 0..4 {
        raster(
            &mut hf,
            &flat(f32::from(u8::try_from(x).unwrap()), 10.0),
            &p,
        );
    }
    assert_eq!(hf.column(0, 0)[0].area, 9);
    assert_eq!(hf.column(1, 0)[0].area, 1);
    assert_eq!(hf.column(2, 0)[0].area, 9);
    assert_eq!(hf.column(3, 0)[0].area, 1);
    // Water never adds spans.
    assert_eq!(hf.column(0, 0).len(), 1);

    // A higher area is kept (max).
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 2, 12, 1);
    raster(&mut hf, &flat(0.0, 10.0), &p);
    assert_eq!(hf.column(0, 0)[0].area, 12);
}

#[test]
fn paint_or_add_adds_when_nothing_touched() {
    let mut p = params(RasterMode::PaintOrAdd, 14);
    p.sub_area = 4;
    p.flag_merge_threshold = 1;

    // Empty column: the piece is added, with sub_area 0 (addSpan) and the hide part (R21).
    let mut hf = hf4();
    raster(&mut hf, &wall(0.0, 20.0, 25.0), &p);
    assert_eq!(hf.column(0, 0), &[in_part(span(20, 25, 14), 4)]);

    // Ground touched by the piece: painted, nothing added.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 4.0, 10.0), &p);
    assert_eq!(hf.column(0, 0).len(), 1);
    assert_eq!(
        (hf.column(0, 0)[0].area, hf.column(0, 0)[0].sub_area),
        (14, 4)
    );

    // Floating above the ground, not touching: added next to it.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 20.0, 25.0), &p);
    assert_eq!(
        hf.column(0, 0),
        &[span(0, 5, 1), in_part(span(20, 25, 14), 4)]
    );
}

#[test]
fn part_at_min_edge_never_paints_row_0() {
    let p = params(RasterMode::Add, 5);
    // Crosses hf.bmin.x = 0 in row 1.
    let x_cross = [[-1.5, 2.0, 1.5], [1.5, 2.0, 1.5], [1.5, 2.0, 1.2]];
    let mut hf = hf4();
    rasterize_part(&mut hf, &x_cross, &p);
    assert!(hf.column(0, 1).is_empty(), "column 0 untouched");
    assert_eq!(hf.column(1, 1), &[span(2, 3, 5)]);

    // The stock rasteriser (clamp -1) does paint column 0 for the same triangle.
    let mut hf = hf4();
    rasterize_triangle(&mut hf, x_cross[0], x_cross[1], x_cross[2], &p);
    assert_eq!(hf.column(0, 1), &[span(2, 3, 5)]);

    // Crosses hf.bmin.z = 0 in column 1: row 0 untouched, row 1 painted.
    let z_cross = [[1.2, 2.0, -1.5], [1.5, 2.0, 1.5], [1.5, 2.0, -1.5]];
    let mut hf = hf4();
    rasterize_part(&mut hf, &z_cross, &p);
    assert!(hf.column(1, 0).is_empty(), "row 0 untouched");
    assert!(!hf.column(1, 1).is_empty());
}

#[test]
fn part_applies_the_union_of_its_triangles_once() {
    // Two triangles in one cell: the union [3, 9] is added once.
    let p = params(RasterMode::Add, 14);
    let mut hf = hf4();
    let tris = [
        [0.2, 3.0, 0.5],
        [0.8, 3.0, 0.5],
        [0.5, 4.0, 0.5],
        [0.2, 8.0, 0.5],
        [0.8, 8.0, 0.5],
        [0.5, 9.0, 0.5],
    ];
    rasterize_part(&mut hf, &tris, &p);
    assert_eq!(hf.column(0, 0), &[span(3, 9, 14)]);
}

#[test]
fn heights_clamp_to_8191() {
    // A flat piece at exactly 8191: lo = ceil = 8191, so clamp(8191, 8192, 8191) is 8192 as in C
    // (the minimum wins); the 13-bit store wraps it to 0.
    let mut hf = Heightfield::new(2, 2, [0.0; 3], [2.0, 20000.0, 2.0], 1.0, 1.0).unwrap();
    let t = [[0.2, 8191.0, 0.2], [0.8, 8191.0, 0.2], [0.2, 8191.0, 0.8]];
    raster(&mut hf, &t, &params(RasterMode::Add, 1));
    assert_eq!(hf.column(0, 0), &[span(8191, 0, 1)]);

    // The ordinary clamp: lo below 8191, ceil above it gives hi == 8191.
    let mut hf = Heightfield::new(2, 2, [0.0; 3], [2.0, 2000.0, 2.0], 1.0, 0.1).unwrap();
    raster(
        &mut hf,
        &wall(0.0, 800.5, 1500.0),
        &params(RasterMode::Add, 1),
    );
    let col = hf.column(0, 0);
    assert_eq!(col.len(), 1);
    assert_eq!(col[0].smax, 8191);
    assert!((8004..=8006).contains(&col[0].smin));

    // A piece wholly above the heightfield is skipped.
    let mut hf = Heightfield::new(2, 2, [0.0; 3], [2.0, 10.0, 2.0], 1.0, 0.1).unwrap();
    let above = [[0.2, 50.0, 0.2], [0.8, 50.0, 0.2], [0.2, 50.0, 0.8]];
    raster(&mut hf, &above, &params(RasterMode::Add, 1));
    assert!(hf.column(0, 0).is_empty());
}

#[test]
fn heightfield_new_rejects_bad_sizes() {
    assert!(Heightfield::new(-1, 4, [0.0; 3], [1.0; 3], 1.0, 1.0).is_err());
    assert!(Heightfield::new(4, 4, [0.0; 3], [1.0; 3], 0.0, 1.0).is_err());
    assert!(Heightfield::new(100_000, 100_000, [0.0; 3], [1.0; 3], 1.0, 1.0).is_err());
}

#[test]
fn hostile_vertices_do_not_panic() {
    let mut hf = hf4();
    let p = params(RasterMode::Add, 1);
    let nan = f32::NAN;
    rasterize_triangle(&mut hf, [nan; 3], [0.0; 3], [1.0; 3], &p);
    rasterize_triangle(&mut hf, [f32::INFINITY; 3], [0.0; 3], [-1.0e30; 3], &p);
    rasterize_part(&mut hf, &[[nan; 3], [0.0; 3], [f32::INFINITY; 3]], &p);
}

#[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
#[test]
fn rsqrt_is_the_hardware_approximation() {
    use dayzmake_core::navmesh::internals::rsqrt_approx;
    let exact = 1.0f32 / 2.0f32.sqrt();
    let approx = rsqrt_approx(2.0);
    assert_ne!(
        approx.to_bits(),
        exact.to_bits(),
        "RSQRTSS, not an exact sqrt"
    );
    assert!(((approx - exact) / exact).abs() < 1.5 * 2.0f32.powi(-12) * 1.01);
}

#[test]
fn paint_below_starts_at_the_last_span_not_above_the_piece() {
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    hf.add_span(0, 0, 10, 15, 2, 1);
    hf.add_span(0, 0, 20, 25, 3, 1);
    raster(
        &mut hf,
        &wall(0.0, 12.0, 30.0),
        &params(RasterMode::PaintBelow, 20),
    );
    let areas: Vec<u8> = hf.column(0, 0).iter().map(|s| s.area).collect();
    assert_eq!(areas, [1, 20, 20]);
}

#[test]
fn tree_hit_when_trunk_starts_within_walkable_height_above_ground() {
    let mut p = params(RasterMode::Tree, 10);
    p.walkable_climb = 5;
    p.walkable_height = 15;
    // Ground top 5, trunk from 12: 12 < 5 + 15, hit.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 12.0, 30.0), &p);
    assert_eq!(hf.column(0, 0)[0].area, 10);
    // With a walkable height of 5: 12 < 10 fails, no hit.
    p.walkable_height = 5;
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 12.0, 30.0), &p);
    assert_eq!(hf.column(0, 0)[0].area, 1);
}

#[test]
fn paint_or_add_stops_at_smax_plus_one() {
    let mut p = params(RasterMode::PaintOrAdd, 14);
    p.sub_area = 4;
    // Piece [3, 6]: 6 <= 5 + 1 is a stop, so it is added (merged), never painted.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 3.0, 6.0), &p);
    assert_eq!(hf.column(0, 0), &[in_part(span(0, 6, 14), 4)]);
    // Piece [3, 7]: painted with the sub area, nothing added.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 3.0, 7.0), &p);
    assert_eq!(hf.column(0, 0).len(), 1);
    assert_eq!(hf.column(0, 0)[0].sub_area, 4);
}

#[test]
fn water_depth_boundary_is_inclusive() {
    let mut p = params(RasterMode::Water, 9);
    p.water_depth = 5;
    // Span smin 0: piece smin 5 gives 0 + 5 <= 5, upgraded; piece smin 4 does not.
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 2, 1, 1);
    hf.add_span(1, 0, 0, 2, 1, 1);
    raster(&mut hf, &flat(0.0, 5.0), &p);
    raster(&mut hf, &flat(1.0, 4.0), &p);
    assert_eq!(hf.column(0, 0)[0].area, 9);
    assert_eq!(hf.column(1, 0)[0].area, 1);
}

#[test]
fn parts_far_apart_in_a_large_grid() {
    let mut hf = Heightfield::new(1024, 1024, [0.0; 3], [1024.0, 100.0, 1024.0], 1.0, 1.0).unwrap();
    let p = params(RasterMode::Add, 12);
    let part_a = [[900.2, 3.0, 5.5], [900.8, 3.0, 5.5], [900.5, 4.0, 5.5]];
    let part_b = [[2.2, 6.0, 800.5], [2.8, 6.0, 800.5], [2.5, 7.0, 800.5]];
    rasterize_part(&mut hf, &part_b, &p);
    rasterize_part(&mut hf, &part_a, &p);
    assert_eq!(hf.column(900, 5), &[span(3, 4, 12)]);
    assert_eq!(hf.column(2, 800), &[span(6, 7, 12)]);
    let total: usize = hf.columns().iter().map(Vec::len).sum();
    assert_eq!(total, 2);
}

/// Ruling R21: the hide part of a span is a layer of its own. A painted span gets it with the sub
/// area; an added span, which has sub area 0, gets it too; the other modes do not write it.
#[test]
fn hide_part_layer_follows_the_hide_rasterisation_only() {
    let mut p = params(RasterMode::PaintOrAdd, 14);
    p.sub_area = 4;
    let mut hf = hf4();
    hf.add_span(0, 0, 0, 5, 1, 1);
    raster(&mut hf, &wall(0.0, 3.0, 7.0), &p);
    assert_eq!(
        (hf.column(0, 0)[0].sub_area, hf.column(0, 0)[0].hide_part),
        (4, 4)
    );
    // The doors (PaintBelow) and the trees (Tree) paint the sub area, not the hide part.
    for mode in [RasterMode::PaintBelow, RasterMode::Tree] {
        let mut q = params(mode, 12);
        q.sub_area = 6;
        q.walkable_height = 2;
        q.walkable_climb = 1;
        let mut hf = hf4();
        hf.add_span(0, 0, 0, 5, 1, 1);
        raster(&mut hf, &wall(0.0, 3.0, 12.0), &q);
        assert_eq!(hf.column(0, 0)[0].hide_part, 0, "{mode:?}");
    }
    // A plain addSpan leaves it 0.
    let mut hf = hf4();
    hf.add_span(1, 1, 0, 5, 14, 1);
    assert_eq!(hf.column(1, 1)[0].hide_part, 0);
}

/// Ruling R21: a hide part painted or added over spans that already belong to another hide part (two
/// copies of one object) leaves them in the first part; the lowest span that has one decides.
#[test]
fn an_added_hide_span_keeps_the_hide_part_it_merges_with() {
    let mut first = params(RasterMode::PaintOrAdd, 14);
    first.sub_area = 4;
    first.flag_merge_threshold = 1;
    let mut second = first;
    second.sub_area = 9;
    let mut hf = hf4();
    raster(&mut hf, &wall(0.0, 20.0, 25.0), &first);
    raster(&mut hf, &wall(0.0, 24.0, 25.0), &second);
    assert_eq!(hf.column(0, 0), &[in_part(span(20, 25, 14), 4)]);
    // Painting (the piece reaches past the span's top) keeps the first part too, and takes the area
    // and the sub area as before.
    raster(&mut hf, &wall(0.0, 22.0, 30.0), &second);
    assert_eq!(hf.column(0, 0).len(), 1);
    assert_eq!(
        (hf.column(0, 0)[0].sub_area, hf.column(0, 0)[0].hide_part),
        (9, 4)
    );
    let mut hf = hf4();
    raster(&mut hf, &wall(0.0, 20.0, 25.0), &first);
    raster(&mut hf, &wall(0.0, 24.0, 25.0), &second);
    // Separate spans stay separate and keep their own part.
    raster(&mut hf, &wall(0.0, 40.0, 45.0), &second);
    assert_eq!(
        hf.column(0, 0),
        &[in_part(span(20, 25, 14), 4), in_part(span(40, 45, 14), 9)]
    );
    // A span bridging both merges with both: the lower one's part.
    hf.add_span_in_part(0, 0, 24, 41, 14, 1, Some(9));
    assert_eq!(hf.column(0, 0), &[in_part(span(20, 45, 14), 4)]);
}
