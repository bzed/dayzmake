//! The polygon mesh with DayZ's changes (spec 5.4 step 10, 7.3, 9.3, 10.1 item 8).

#![cfg(feature = "navmesh")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use dayzmake_core::navmesh::detour::{CreateParams, TileData, TileExtra, create_tile_data};
use dayzmake_core::navmesh::internals::{
    CompactHeightfield, Contour, ContourSet, Heightfield, PolyMesh, RC_BORDER_VERTEX,
    RC_CONTOUR_TESS_WALL_EDGES, Span, build_compact, build_contours, build_distance_field,
    build_poly_mesh, build_poly_mesh_with_border_removal, build_regions,
};

const NIL: u16 = 0xffff;

/// A `w` x `h` field; `part(x, z)` gives `(area, sub_area)`, the span covers `0..2`.
fn field(w: i32, h: i32, part: impl Fn(i32, i32) -> (u8, u16)) -> CompactHeightfield {
    let mut hf = Heightfield::new(w, h, [0.0, 0.0, 0.0], [100.0, 100.0, 100.0], 0.25, 0.1).unwrap();
    for z in 0..h {
        for x in 0..w {
            let (area, sub) = part(x, z);
            if area == 0 {
                continue;
            }
            let mut s = Span::new(0, 2, area);
            s.sub_area = sub;
            hf.columns_mut()[usize::try_from(x + z * w).unwrap()].push(s);
        }
    }
    build_compact(4, 100, &hf).unwrap()
}

fn contour_set(c: &mut CompactHeightfield, border: i32) -> ContourSet {
    build_distance_field(c).unwrap();
    build_regions(c, border, &[0; 64], 10_000).unwrap();
    build_contours(c, 1.3, 8.0, 0, RC_CONTOUR_TESS_WALL_EDGES).unwrap()
}

fn mesh_of(w: i32, h: i32, border: i32, part: impl Fn(i32, i32) -> (u8, u16)) -> PolyMesh {
    let mut c = field(w, h, part);
    let set = contour_set(&mut c, border);
    build_poly_mesh(&set, 6).unwrap()
}

/// The vertex indices of polygon `i`.
fn poly_verts(m: &PolyMesh, i: usize) -> Vec<u16> {
    m.polys[i * m.nvp * 2..i * m.nvp * 2 + m.nvp]
        .iter()
        .copied()
        .take_while(|&v| v != NIL)
        .collect()
}

/// The neighbour codes of polygon `i` (one per edge).
fn poly_neis(m: &PolyMesh, i: usize) -> Vec<u16> {
    let n = poly_verts(m, i).len();
    m.polys[i * m.nvp * 2 + m.nvp..i * m.nvp * 2 + m.nvp + n].to_vec()
}

fn xz(m: &PolyMesh, v: u16) -> (u16, u16) {
    let p = m.verts[usize::from(v)];
    (p[0], p[2])
}

/// Twice the area of polygon `i` in cells (shoelace on x and z).
fn area2(m: &PolyMesh, i: usize) -> i64 {
    let vs = poly_verts(m, i);
    let mut sum = 0_i64;
    for (k, &a) in vs.iter().enumerate() {
        let b = vs[(k + 1) % vs.len()];
        let (ax, az) = xz(m, a);
        let (bx, bz) = xz(m, b);
        sum += i64::from(ax) * i64::from(bz) - i64::from(bx) * i64::from(az);
    }
    sum.abs()
}

fn cont(pts: &[[i32; 4]], reg: u16, area: u8, sub_area: u16) -> Contour {
    Contour {
        verts: pts.to_vec(),
        reg,
        area,
        sub_area,
        ..Contour::default()
    }
}

fn set_of(contours: Vec<Contour>, border: i32) -> ContourSet {
    ContourSet {
        contours,
        bmin: [0.0; 3],
        bmax: [25.0, 10.0, 25.0],
        cs: 0.25,
        ch: 0.1,
        width: 20,
        height: 20,
        border_size: border,
        max_error: 1.3,
    }
}

#[test]
fn square_contour_gives_one_quad() {
    let mut c = field(20, 20, |_, _| (1, 0));
    let set = contour_set(&mut c, 0);
    let m = build_poly_mesh(&set, 6).unwrap();
    assert_eq!(m.nvp, 6);
    assert_eq!(m.verts.len(), 4);
    assert_eq!(m.areas, [1]);
    assert_eq!(m.sub_areas, [0]);
    assert_eq!(m.flags, [0]);
    assert_eq!(m.regs.len(), 1);
    assert_eq!(m.polys.len(), 12);
    assert_eq!(poly_verts(&m, 0).len(), 4);
    assert_eq!(&m.polys[4..6], [NIL, NIL]);
    assert_eq!(&m.polys[6..], [NIL; 6]);
    assert_eq!(area2(&m, 0), 2 * 400);
    let mut corners: Vec<(u16, u16)> = (0..4).map(|v| xz(&m, v)).collect();
    corners.sort_unstable();
    assert_eq!(corners, [(0, 0), (0, 20), (20, 0), (20, 20)]);
    assert_eq!(
        (m.cs.to_bits(), m.ch.to_bits()),
        (0.25_f32.to_bits(), 0.1_f32.to_bits())
    );
    assert_eq!((m.bmin, m.bmax), (set.bmin, set.bmax));
    assert!((m.max_edge_error - 1.3).abs() < 1e-6);
}

#[test]
fn poly_mesh_nvp6_merges_and_keeps_border_verts() {
    // An L shape: triangles merge up to convex polygons of at most 6 vertices.
    let m = mesh_of(30, 30, 0, |x, z| (u8::from(x < 10 || z < 10), 0));
    let total: i64 = (0..m.areas.len()).map(|i| area2(&m, i)).sum();
    assert_eq!(total, 2 * (30 * 30 - 20 * 20));
    assert_eq!(m.areas.len(), 2, "an L is two convex polygons");
    for i in 0..m.areas.len() {
        assert!(poly_verts(&m, i).len() <= 6);
    }

    // A tile-sized region inside a border keeps its 4 corner vertices.
    let m = mesh_of(24, 24, 2, |_, _| (1, 0));
    assert_eq!(m.verts.len(), 4);
    assert_eq!(m.border_size, 2);

    // A flagged vertex in the middle of a tile edge: stock removes it, DayZ keeps it.
    let flagged = [
        [0, 0, 0, 0],
        [0, 0, 20, 0],
        [20, 0, 20, 0],
        [20, 0, 0, 0],
        [10, 0, 0, RC_BORDER_VERTEX],
    ];
    let set = set_of(vec![cont(&flagged, 1, 1, 0)], 2);
    let dayz = build_poly_mesh(&set, 6).unwrap();
    assert_eq!(dayz.verts.len(), 5);
    assert!(dayz.verts.contains(&[10, 0, 0]));
    let total: i64 = (0..dayz.areas.len()).map(|i| area2(&dayz, i)).sum();
    assert_eq!(total, 2 * 400);
    let stock = build_poly_mesh_with_border_removal(&set, 6).unwrap();
    assert_eq!(stock.verts.len(), 4, "stock removes the flagged vertex");
    assert!(!stock.verts.contains(&[10, 0, 0]));
}

#[test]
fn poly_mesh_sub_areas_from_contours() {
    let m = mesh_of(20, 10, 0, |x, _| (1, if x < 10 { 3 } else { 7 }));
    assert_eq!(m.sub_areas.len(), 2);
    let mut subs = m.sub_areas.clone();
    subs.sort_unstable();
    assert_eq!(subs, [3, 7]);
    // The part id belongs to the polygon of its side.
    for (i, &s) in m.sub_areas.iter().enumerate() {
        let x = poly_verts(&m, i)
            .iter()
            .map(|&v| xz(&m, v).0)
            .min()
            .unwrap();
        assert_eq!(s, if x < 10 { 3 } else { 7 });
    }
    // Hand-built contours: every polygon of a contour carries that contour's part id.
    let sq = [[0, 0, 0, 0], [0, 0, 20, 0], [20, 0, 20, 0], [20, 0, 0, 0]];
    let set = set_of(vec![cont(&sq, 1, 1, 42)], 0);
    let m = build_poly_mesh(&set, 3).unwrap();
    assert_eq!(m.sub_areas, [42, 42]);
}

#[test]
fn adjacency_internal_neighbours() {
    // Two touching regions (different areas) over a 20 x 10 field.
    let m = mesh_of(20, 10, 0, |x, _| (if x < 10 { 1 } else { 2 }, 0));
    assert_eq!(m.areas.len(), 2);
    let left = usize::from(m.areas[0] != 1);
    let right = 1 - left;
    let ln = poly_neis(&m, left);
    let rn = poly_neis(&m, right);
    // Exactly one edge each is shared and refers to the other polygon.
    assert_eq!(
        ln.iter()
            .filter(|&&n| n == u16::try_from(right).unwrap())
            .count(),
        1
    );
    assert_eq!(
        rn.iter()
            .filter(|&&n| n == u16::try_from(left).unwrap())
            .count(),
        1
    );
    assert_eq!(ln.iter().filter(|&&n| n == NIL).count(), 3);
    // The shared edge lies on x = 10.
    let lv = poly_verts(&m, left);
    let j = ln.iter().position(|&n| n != NIL).unwrap();
    let (a, b) = (xz(&m, lv[j]), xz(&m, lv[(j + 1) % lv.len()]));
    assert_eq!((a.0, b.0), (10, 10));
    // No portals without a border.
    assert!(ln.iter().chain(&rn).all(|&n| n & 0x8000 == 0 || n == NIL));
}

#[test]
fn portal_edges_marked_with_border_size() {
    let m = mesh_of(24, 24, 2, |_, _| (1, 0));
    assert_eq!(m.areas.len(), 1);
    let vs = poly_verts(&m, 0);
    let ns = poly_neis(&m, 0);
    let mut codes = Vec::new();
    for (j, &n) in ns.iter().enumerate() {
        let (a, b) = (xz(&m, vs[j]), xz(&m, vs[(j + 1) % vs.len()]));
        let want = if a.0 == 0 && b.0 == 0 {
            0x8000
        } else if a.1 == 20 && b.1 == 20 {
            0x8001
        } else if a.0 == 20 && b.0 == 20 {
            0x8002
        } else {
            0x8003
        };
        assert_eq!(n, want, "edge {a:?} -> {b:?}");
        codes.push(n);
    }
    codes.sort_unstable();
    assert_eq!(codes, [0x8000, 0x8001, 0x8002, 0x8003]);
    // Without a border the same region has no portal codes.
    let m = mesh_of(20, 20, 0, |_, _| (1, 0));
    assert!(poly_neis(&m, 0).iter().all(|&n| n == NIL));
    // The edge of an internal neighbour is not overwritten by a portal code.
    let sq_l = [[0, 0, 0, 0], [0, 0, 20, 0], [10, 0, 20, 0], [10, 0, 0, 0]];
    let sq_r = [[10, 0, 0, 0], [10, 0, 20, 0], [20, 0, 20, 0], [20, 0, 0, 0]];
    let set = set_of(vec![cont(&sq_l, 1, 1, 0), cont(&sq_r, 2, 1, 0)], 2);
    let m = build_poly_mesh(&set, 6).unwrap();
    let all: Vec<u16> = (0..2).flat_map(|i| poly_neis(&m, i)).collect();
    assert_eq!(all.iter().filter(|&&n| n < 2).count(), 2, "{all:x?}");
    assert_eq!(all.iter().filter(|&&n| n & 0x8000 != 0).count(), 6);
}

#[test]
fn degenerate_contour_skipped_not_panicking() {
    let sq = [[0, 0, 0, 0], [0, 0, 20, 0], [20, 0, 20, 0], [20, 0, 0, 0]];
    let two = [[50, 0, 50, 0], [60, 0, 60, 0]];
    let set = set_of(
        vec![cont(&two, 9, 1, 0), cont(&sq, 1, 1, 0), cont(&[], 8, 1, 0)],
        0,
    );
    let m = build_poly_mesh(&set, 6).unwrap();
    assert_eq!(m.areas.len(), 1);
    assert_eq!(m.regs, [1]);
    // The skipped contour adds no vertices.
    assert_eq!(m.verts.len(), 4);
    // Nothing but degenerate contours gives an empty mesh.
    let set = set_of(vec![cont(&two, 9, 1, 0)], 0);
    let m = build_poly_mesh(&set, 6).unwrap();
    assert!(m.areas.is_empty() && m.verts.is_empty() && m.polys.is_empty());
    // Contours that cannot be triangulated completely still add their vertices (stock logs
    // "Bad triangulation" and carries on). This one yields no triangle at all.
    let none = [[1, 0, 0, 0], [2, 0, 3, 0], [1, 0, 2, 0], [0, 0, 1, 0]];
    let set = set_of(vec![cont(&none, 1, 1, 0)], 0);
    let m = build_poly_mesh(&set, 3).unwrap();
    assert_eq!((m.verts.len(), m.areas.len()), (4, 0));
    // This one yields a partial triangulation (1 triangle instead of 3), which is kept.
    let partial = [
        [2, 0, 0, 0],
        [3, 0, 3, 0],
        [1, 0, 2, 0],
        [2, 0, 3, 0],
        [0, 0, 0, 0],
    ];
    let set = set_of(vec![cont(&partial, 1, 1, 0)], 0);
    let m = build_poly_mesh(&set, 3).unwrap();
    assert_eq!((m.verts.len(), m.areas.len()), (5, 1));
    // A bow tie: its 4 vertices are in the mesh and it gives 2 triangles.
    let bow = [[0, 0, 0, 0], [20, 0, 20, 0], [20, 0, 0, 0], [0, 0, 20, 0]];
    let set = set_of(vec![cont(&bow, 1, 1, 0)], 0);
    let m = build_poly_mesh(&set, 6).unwrap();
    assert_eq!((m.verts.len(), m.areas.len()), (4, 2));
}

#[test]
fn vertices_within_two_cells_of_height_are_merged() {
    // The shared corner (10,0) differs by 2 in y (merged, the first one wins); the shared corner
    // (10,20) differs by 3 (two vertices).
    let a = [[0, 5, 0, 0], [0, 5, 20, 0], [10, 5, 20, 0], [10, 5, 0, 0]];
    let b = [[10, 7, 0, 0], [10, 8, 20, 0], [20, 5, 20, 0], [20, 5, 0, 0]];
    let set = set_of(vec![cont(&a, 1, 1, 0), cont(&b, 2, 1, 0)], 0);
    let m = build_poly_mesh(&set, 6).unwrap();
    assert_eq!(m.verts.len(), 7);
    assert_eq!(m.verts[3], [10, 5, 0]);
    assert!(m.verts.contains(&[10, 8, 20]));
    // The (10,20) corners are different vertices, so the edge x = 10 is not shared.
    assert!(poly_neis(&m, 0).iter().all(|&n| n == NIL));
    assert_eq!(m.polys[0..4], [0, 1, 2, 3]);
}

/// `n` triangles with distinct coordinates, then `last`.
fn triangle_soup(n: usize, last: &[[i32; 4]]) -> ContourSet {
    let mut contours = Vec::with_capacity(n + 1);
    for k in 0..n {
        let (x, z) = (
            i32::try_from(k % 200).unwrap() * 3,
            i32::try_from(k / 200).unwrap() * 3,
        );
        let tri = [[x, 0, z, 0], [x, 0, z + 1, 0], [x + 1, 0, z, 0]];
        contours.push(cont(&tri, 1, 1, 0));
    }
    contours.push(cont(last, 2, 1, 0));
    set_of(contours, 0)
}

#[test]
fn too_many_vertices_is_an_error() {
    // 21843 triangles are 65529 vertices; a pentagon makes 65534 = 0xfffe (error), a square 65533
    // (fine). Many small contours keep a regression of the limit fast.
    let pentagon = [
        [700, 0, 0, 0],
        [700, 0, 10, 0],
        [710, 0, 20, 0],
        [720, 0, 10, 0],
        [720, 0, 0, 0],
    ];
    let e = build_poly_mesh(&triangle_soup(21_843, &pentagon), 6).unwrap_err();
    assert!(
        e.to_string()
            .contains("Too many vertices per tile 65534 (max: 65535)"),
        "{e}"
    );
    let square = [
        [700, 0, 0, 0],
        [700, 0, 10, 0],
        [720, 0, 10, 0],
        [720, 0, 0, 0],
    ];
    let m = build_poly_mesh(&triangle_soup(21_843, &square), 6).unwrap();
    assert_eq!(m.verts.len(), 65_533);
    assert_eq!(m.areas.len(), 21_844);
}

#[test]
fn vertex_outside_u16_is_an_error() {
    for bad in [[-1, 0, 0, 0], [0, 0x1_0000, 0, 0], [0, 0, i32::MAX, 0]] {
        let tri = [[0, 0, 0, 0], [0, 0, 5, 0], [5, 0, 0, 0]];
        let mut pts = tri.to_vec();
        pts.push(bad);
        let set = set_of(vec![cont(&pts, 1, 1, 0)], 0);
        assert!(build_poly_mesh(&set, 6).is_err(), "{bad:?}");
    }
}

#[test]
fn non_square_tile_portals_use_width_and_height() {
    let mut set = set_of(
        vec![cont(
            &[[0, 0, 0, 0], [0, 0, 20, 0], [30, 0, 20, 0], [30, 0, 0, 0]],
            1,
            1,
            0,
        )],
        2,
    );
    set.width = 30;
    set.height = 20;
    let m = build_poly_mesh(&set, 6).unwrap();
    let vs = poly_verts(&m, 0);
    for (j, &n) in poly_neis(&m, 0).iter().enumerate() {
        let (a, b) = (xz(&m, vs[j]), xz(&m, vs[(j + 1) % vs.len()]));
        let want = match (a, b) {
            ((0, _), (0, _)) => 0x8000,
            ((_, 20), (_, 20)) => 0x8001,
            ((30, _), (30, _)) => 0x8002,
            _ => 0x8003,
        };
        assert_eq!(n, want, "{a:?} -> {b:?}");
    }
}

#[test]
fn poly_mesh_feeds_create_tile_data() {
    let m = mesh_of(24, 24, 2, |x, z| (1, u16::from(x > 12 && z > 12)));
    assert!(m.areas.len() >= 2);
    let extra = TileExtra::default();
    let mut p = CreateParams::new(&m.verts, &m.polys, &m.flags, &m.areas, m.nvp, &[], &extra);
    p.bmin = m.bmin;
    p.bmax = m.bmax;
    p.cs = m.cs;
    p.ch = m.ch;
    p.walkable_height = 1.5;
    p.walkable_radius = 0.3;
    p.walkable_climb = 0.45;
    let data = create_tile_data(&p).unwrap();
    let parsed = TileData::parse(&data.to_bytes()).unwrap();
    assert_eq!(parsed.polys.len(), m.areas.len());
    // The portal codes of the border edges became external links.
    assert!(
        parsed
            .polys
            .iter()
            .any(|p| p.neis.iter().any(|&n| n & 0x8000 != 0))
    );
}
