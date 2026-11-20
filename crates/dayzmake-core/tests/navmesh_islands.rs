//! Tests of the nearest-polygon query and the island filter (spec 8.5, 10.2, 13.3).

#![cfg(feature = "navmesh")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use dayzmake_core::navmesh::detour::{
    CreateParams, NavMesh, NavMeshParams, OffMeshInput, TileData, TileExtra, create_tile_data,
    decode_poly_ref, encode_poly_ref, find_nearest_poly,
};
use dayzmake_core::navmesh::islands::filter_islands;

const NIL: u16 = 0xffff;

fn mesh_params(max_tiles: i32) -> NavMeshParams {
    let mut p = NavMeshParams::default();
    p.tile_width = 50.0;
    p.tile_height = 50.0;
    p.max_tiles = max_tiles;
    p.max_polys = 1 << 20;
    p
}

/// A 50 m square in tile `(x, z)` at height `h` cells (0.1 m), with polygon flags `flags`. `neis`
/// are the portal codes of the edges x-, z+, x+, z-.
fn square(
    (x, z): (i32, i32),
    h: u16,
    neis: [u16; 4],
    flags: u16,
    off_mesh: &[OffMeshInput],
) -> TileData {
    let verts = [[0, h, 0], [0, h, 200], [200, h, 200], [200, h, 0]];
    let polys = [
        0, 1, 2, 3, NIL, NIL, neis[0], neis[1], neis[2], neis[3], NIL, NIL,
    ];
    let extra = TileExtra::default();
    let flag_arr = [flags];
    let mut p = CreateParams::new(&verts, &polys, &flag_arr, &[1], 6, off_mesh, &extra);
    p.tile_x = x;
    p.tile_y = z;
    p.bmin = [
        50.0 * f32::from(i16::try_from(x).unwrap()),
        0.0,
        50.0 * f32::from(i16::try_from(z).unwrap()),
    ];
    p.bmax = [p.bmin[0] + 50.0, 5.0, p.bmin[2] + 50.0];
    p.walkable_height = 1.5;
    p.walkable_radius = 0.3;
    p.walkable_climb = 0.45;
    create_tile_data(&p).expect("create tile")
}

fn conn(start: [f32; 3], end: [f32; 3], bidirectional: bool) -> OffMeshInput {
    let mut c = OffMeshInput::default();
    c.start = start;
    c.end = end;
    c.radius = 1.0;
    c.flags = 0x180;
    c.area = 50;
    c.bidirectional = bidirectional;
    c
}

fn flags_of(mesh: &NavMesh, x: i32, z: i32) -> Vec<u16> {
    mesh.tile_at(x, z)
        .unwrap()
        .polys
        .iter()
        .map(|p| p.flags)
        .collect()
}

/// Tiles (0,0) and (1,0) linked, tile (2,0) alone.
fn three_tiles(flags_a: u16) -> NavMesh {
    let mut mesh = NavMesh::new(mesh_params(3)).unwrap();
    mesh.set_tile(square((0, 0), 0, [NIL, NIL, 0x8002, NIL], flags_a, &[]))
        .unwrap();
    mesh.set_tile(square((1, 0), 0, [0x8000, NIL, NIL, NIL], 1, &[]))
        .unwrap();
    mesh.set_tile(square((2, 0), 0, [NIL; 4], 0x8001, &[]))
        .unwrap();
    mesh
}

const SEED_A: [f32; 3] = [25.0, 0.0, 25.0];

#[test]
fn nearest_poly_prefers_poly_directly_below_within_climb() {
    // Tile 0's polygon is at y 0 and tile 1's at y 0.3, side by side. A centre 0.3 m above tile
    // 0's polygon is within the climb height 0.45, so it counts as distance 0.
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    mesh.set_tile(square((0, 0), 0, [NIL; 4], 1, &[])).unwrap();
    mesh.set_tile(square((1, 0), 3, [NIL; 4], 1, &[])).unwrap();
    let (r, p) = find_nearest_poly(&mesh, [49.9, 0.3, 25.0], [30.0, 30.0, 3.0], 0xffff, 0).unwrap();
    assert_eq!(r, encode_poly_ref(1, 0, 0));
    assert!((p[1] - 0.0).abs() < 1e-4 && (p[0] - 49.9).abs() < 1e-4);
    // Over the second tile's polygon, which is 0.3 m up: found there, in the other tile.
    let (r, p) = find_nearest_poly(&mesh, [50.1, 0.3, 25.0], [30.0, 30.0, 3.0], 0xffff, 0).unwrap();
    assert_eq!(decode_poly_ref(r).1, 1);
    assert!((p[1] - 0.3).abs() < 1e-3);
}

#[test]
fn nearest_poly_honours_the_flags() {
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    mesh.set_tile(square((0, 0), 0, [NIL; 4], 0x2, &[]))
        .unwrap();
    mesh.set_tile(square((1, 0), 0, [NIL; 4], 0x1, &[]))
        .unwrap();
    let c = [25.0, 0.0, 25.0];
    let e = [30.0, 30.0, 3.0];
    // Excluded flag 0x2: the polygon the centre is over is skipped, the one in the next tile wins.
    let (r, _) = find_nearest_poly(&mesh, c, e, 0xffff, 0x8002).unwrap();
    assert_eq!(decode_poly_ref(r).1, 1);
    // No include flag matches.
    assert!(find_nearest_poly(&mesh, c, e, 0x4, 0).is_none());
    // Nothing in the box.
    assert!(find_nearest_poly(&mesh, [500.0, 0.0, 25.0], e, 0xffff, 0).is_none());
}

#[test]
fn nearest_poly_rejects_non_finite_input() {
    let mesh = three_tiles(1);
    let e = [30.0, 30.0, 3.0];
    assert!(find_nearest_poly(&mesh, SEED_A, e, 0xffff, 0).is_some());
    assert!(find_nearest_poly(&mesh, [f32::NAN, 0.0, 25.0], e, 0xffff, 0).is_none());
    assert!(find_nearest_poly(&mesh, SEED_A, [f32::INFINITY, 30.0, 3.0], 0xffff, 0).is_none());
}

#[test]
fn seed_extents_are_30_30_3() {
    let mut mesh = NavMesh::new(mesh_params(1)).unwrap();
    mesh.set_tile(square((0, 0), 0, [NIL; 4], 1, &[])).unwrap();
    // The polygon is z 0..50: 10 m beyond it in z is outside the z half extent of 3 ...
    assert!(filter_islands(&mut mesh, [25.0, 0.0, 60.0]).is_none());
    // ... and the same offset in x is inside the x half extent of 30.
    assert!(filter_islands(&mut mesh, [60.0, 0.0, 25.0]).is_some());
}

#[test]
fn islands_flag_unreachable() {
    let mut mesh = three_tiles(0x8001);
    let report = filter_islands(&mut mesh, SEED_A).expect("seed found");
    assert_eq!(report.reachable, 2);
    assert_eq!(report.isolated, 1);
    // The old 0x8000 on a reachable polygon is cleared, the third tile's polygon gets it.
    assert_eq!(flags_of(&mesh, 0, 0), vec![0x0001]);
    assert_eq!(flags_of(&mesh, 1, 0), vec![0x0001]);
    assert_eq!(flags_of(&mesh, 2, 0), vec![0x8001]);
    // Running it again gives the same result.
    let again = filter_islands(&mut mesh, SEED_A).unwrap();
    assert_eq!((again.reachable, again.isolated), (2, 1));
}

#[test]
fn islands_follow_one_way_off_mesh_in_direction() {
    let build = || {
        let cons = [conn([45.0, 0.0, 10.0], [55.0, 0.0, 10.0], false)];
        let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
        mesh.set_tile(square((0, 0), 0, [NIL; 4], 1, &cons))
            .unwrap();
        mesh.set_tile(square((1, 0), 0, [NIL; 4], 1, &[])).unwrap();
        mesh
    };
    // From the start side: the quad, the off-mesh polygon and the landing quad.
    let mut mesh = build();
    let r = filter_islands(&mut mesh, SEED_A).unwrap();
    assert_eq!((r.reachable, r.isolated), (3, 0));
    // From the landing side the one-way link cannot be followed back: the start quad and the
    // off-mesh polygon (flags 0x180) are isolated.
    let mut mesh = build();
    let r = filter_islands(&mut mesh, [75.0, 0.0, 25.0]).unwrap();
    assert_eq!((r.reachable, r.isolated), (1, 2));
    assert_eq!(flags_of(&mesh, 0, 0), vec![0x8001, 0x8180]);
    assert_eq!(flags_of(&mesh, 1, 0), vec![0x0001]);
}

#[test]
fn islands_traverse_disabled_doors() {
    // Tile 1's polygon is a closed door (flag 0x2): the seed in tile 0 floods through it.
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    mesh.set_tile(square((0, 0), 0, [NIL, NIL, 0x8002, NIL], 1, &[]))
        .unwrap();
    mesh.set_tile(square((1, 0), 0, [0x8000, NIL, NIL, NIL], 0x3, &[]))
        .unwrap();
    let r = filter_islands(&mut mesh, SEED_A).unwrap();
    assert_eq!((r.reachable, r.isolated), (2, 0));
    assert_eq!(flags_of(&mesh, 1, 0), vec![0x0003]);
}

#[test]
fn seed_not_found_is_none() {
    let mut mesh = three_tiles(1);
    assert!(filter_islands(&mut mesh, [900.0, 0.0, 900.0]).is_none());
    // Step 1 of 8.5 still ran: the stray 0x8000 of tile 2 is cleared and nothing is flagged.
    assert_eq!(flags_of(&mesh, 2, 0), vec![0x0001]);
    // Too high above the polygons for the vertical extent of 30 m.
    assert!(filter_islands(&mut mesh, [25.0, 100.0, 25.0]).is_none());
}

#[test]
#[ignore = "needs a shipped DayZ navmesh (DAYZ_NAVMESH=path, DAYZ_NAVMESH_SEED=x,y,z)"]
#[expect(clippy::print_stderr, reason = "manual test that reports its counts")]
fn shipped_islands_are_reproduced() {
    use dayzmake_core::navmesh::detour::NavMeshSet;
    let path = std::env::var("DAYZ_NAVMESH").expect("set DAYZ_NAVMESH to a navmesh.nm");
    let seed: Vec<f32> = std::env::var("DAYZ_NAVMESH_SEED")
        .expect("set DAYZ_NAVMESH_SEED to x,y,z")
        .split(',')
        .map(|s| s.trim().parse().expect("a number"))
        .collect();
    let seed = [seed[0], seed[1], seed[2]];
    let mut file = std::fs::File::open(&path).expect("open the navmesh");
    let set = NavMeshSet::read(&mut file).expect("read the navmesh");
    let mut mesh = NavMesh::from_set(&set).expect("add the tiles");
    let before: Vec<(u64, Vec<u16>)> = mesh
        .tiles()
        .map(|(r, t)| (r, t.polys.iter().map(|p| p.flags).collect()))
        .collect();
    let report = filter_islands(&mut mesh, seed).expect("the seed finds a polygon");
    let after: Vec<(u64, Vec<u16>)> = mesh
        .tiles()
        .map(|(r, t)| (r, t.polys.iter().map(|p| p.flags).collect()))
        .collect();
    let mut diffs = Vec::new();
    let mut total = 0_usize;
    for ((r, b), (_, a)) in before.iter().zip(&after) {
        total += b.len();
        for (i, (fb, fa)) in b.iter().zip(a).enumerate() {
            if (fb ^ fa) & 0x8000 != 0 {
                diffs.push(format!("tile ref {r:#x} poly {i}: {fb:#06x} -> {fa:#06x}"));
            }
        }
    }
    eprintln!(
        "{total} polygons, {} reachable, {} isolated, {} differ",
        report.reachable,
        report.isolated,
        diffs.len()
    );
    assert!(
        diffs.is_empty(),
        "{} of {total} polygons differ in the isolated bit\n{}",
        diffs.len(),
        diffs[..diffs.len().min(10)].join("\n")
    );
}
