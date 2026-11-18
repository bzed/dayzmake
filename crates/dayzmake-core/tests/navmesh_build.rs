//! Tests of the per-tile build pipeline (spec 4, 5, 7 and 13.2).

#![cfg(feature = "navmesh")]

use dayzmake_core::navmesh::build::{
    BuildOptions, DoorPolicy, TileOutcome, build_tile, tile_config,
};
use dayzmake_core::navmesh::detour::TileData;
use dayzmake_core::navmesh::wire::{TileGroup, TileInput, TilePart};

#[path = "support/navmesh_world.rs"]
mod navmesh_world;
use navmesh_world::{synthetic_world, terrain, world_params};

fn flat_tile(y: f32) -> TileInput {
    let mut input = TileInput::default();
    input.bmin = [0.0, 0.0, 0.0];
    input.bmax = [50.0, 30.0, 50.0];
    let mut g = TileGroup::default();
    g.area = 1;
    g.verts = terrain([0.0, 0.0], [50.0, 50.0])
        .into_iter()
        .map(|v| [v[0], y, v[2]])
        .collect();
    input.groups.push(g);
    input
}

#[expect(clippy::panic, reason = "test helper")]
fn built(outcome: TileOutcome) -> (TileData, usize, usize) {
    match outcome {
        TileOutcome::Built {
            data,
            stray_part_ids,
            obstacles,
        } => (data, stray_part_ids, obstacles),
        other => panic!("expected a built tile, got {other:?}"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn config_class_0() {
    let p = world_params();
    let c = tile_config(&p, &p.cell_classes[0], &BuildOptions::default());
    assert_eq!(c.cells, 200);
    assert!(close(c.cs, 0.25));
    assert!(close(c.ch, 0.1));
    assert_eq!(c.walkable_radius, 2);
    assert_eq!(c.border_size, 14);
    assert_eq!(c.width, 228);
    assert_eq!(c.min_region_area, 16);
    assert!(close(c.max_simplification_error, 1.2));
    assert_eq!(c.door_radius, 1);
    common(&c);
}

#[test]
fn config_class_1() {
    let p = world_params();
    let c = tile_config(&p, &p.cell_classes[1], &BuildOptions::default());
    assert_eq!(c.cells, 500);
    assert!(close(c.cs, 0.1));
    assert_eq!(c.walkable_radius, 3);
    assert_eq!(c.border_size, 33);
    assert_eq!(c.width, 566);
    assert_eq!(c.min_region_area, 100);
    assert!(close(c.max_simplification_error, 3.0));
    assert_eq!(c.door_radius, 2);
    common(&c);
}

fn common(c: &dayzmake_core::navmesh::build::TileConfig) {
    assert_eq!(c.walkable_height, 15);
    assert_eq!(c.crouch_height, 8);
    assert_eq!(c.prone_height, 3);
    assert_eq!(c.walkable_climb, 5);
    assert_eq!(c.deep_water, 15);
    assert_eq!(c.sea_level, 8);
    assert_eq!(c.slope_threshold.to_bits(), 0.707_106_77_f32.to_bits());
}

#[test]
fn sea_level_override() {
    let p = world_params();
    let mut o = BuildOptions::default();
    o.sea_level = Some(2.0);
    assert_eq!(tile_config(&p, &p.cell_classes[0], &o).sea_level, 20);
}

#[test]
fn flat_tile_builds() {
    let p = world_params();
    let mut input = flat_tile(10.0);
    input.x = 1;
    input.z = 2;
    let (data, stray, obstacles) = built(build_tile(&p, &input, &BuildOptions::default()).unwrap());
    assert_eq!((stray, obstacles), (0, 0));
    assert_eq!((data.header.x, data.header.y), (1, 2));
    assert_ne!(data.polys.len(), 0);
    for v in &data.verts {
        assert!(v[0] >= 0.0 - 1e-3 && v[0] <= 50.0 + 1e-3, "x {v:?}");
        assert!(v[2] >= 0.0 - 1e-3 && v[2] <= 50.0 + 1e-3, "z {v:?}");
    }
    for poly in &data.polys {
        assert_eq!(poly.flags, 0x01);
        assert_eq!(poly.area(), 1);
    }
    assert_eq!(
        usize::try_from(data.header.bv_node_count).unwrap(),
        2 * data.polys.len()
    );
    assert!(data.extra.is_some());
}

#[test]
fn unknown_cell_class() {
    let p = world_params();
    let mut input = flat_tile(10.0);
    input.x = 3;
    input.z = 4;
    input.cell_class = 7;
    let e = build_tile(&p, &input, &BuildOptions::default()).unwrap_err();
    assert!(
        e.to_string().contains("tile 3 4 has unknown cell class 7"),
        "{e}"
    );
}

#[test]
fn empty_tile() {
    let p = world_params();
    // Terrain at 0.2 m is under the sea box (0.8 m): deep sea, flags 0x20.
    let (data, _, _) = built(build_tile(&p, &flat_tile(0.2), &BuildOptions::default()).unwrap());
    assert_ne!(data.polys.len(), 0);
    for poly in &data.polys {
        assert_eq!(poly.area(), 5);
        assert_eq!(poly.flags, 0x20);
    }
    // No geometry at all.
    let mut none = TileInput::default();
    none.bmax = [50.0, 30.0, 50.0];
    assert!(matches!(
        build_tile(&p, &none, &BuildOptions::default()).unwrap(),
        TileOutcome::Empty
    ));
}

#[test]
fn synthetic_world_every_tile_builds() {
    let (p, tiles) = synthetic_world();
    let mut obstacles_total = 0;
    let mut door: Option<(TileData, TileData)> = None;
    for input in &tiles {
        let closed = build_tile(&p, input, &BuildOptions::default()).unwrap();
        let (data, _, obstacles) = built(closed);
        obstacles_total += obstacles;
        if (input.x, input.z) == (1, 1) {
            let mut o = BuildOptions::default();
            o.doors = DoorPolicy::Enabled;
            let (enabled, _, _) = built(build_tile(&p, input, &o).unwrap());
            door = Some((data, enabled));
        }
    }
    assert!(obstacles_total >= 1, "no tree obstacles");
    let (closed, enabled) = door.unwrap();
    let open = {
        let mut o = BuildOptions::default();
        o.doors = DoorPolicy::Open;
        let input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap();
        built(build_tile(&p, input, &o).unwrap()).0
    };
    let mut disabled_seen = false;
    for (part, polys) in part_polys(&closed) {
        // The only door part with polygons is the closed door (area 13, part index 1).
        assert_eq!(part.part_index, 1);
        for poly in polys {
            assert_eq!(
                closed.polys[poly].flags & 2,
                0,
                "Closed leaves area 13 enabled"
            );
            assert_eq!(enabled.polys[poly].flags & 2, 0, "Enabled disables nothing");
            let off = open.polys[poly].flags & 2 != 0;
            assert!(off, "Open disables area 13");
            disabled_seen |= off;
        }
    }
    assert!(disabled_seen);
}

/// The parts with polygons and the polygons of each.
#[expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper on a tile it just built"
)]
fn part_polys(data: &TileData) -> Vec<(dayzmake_core::navmesh::detour::TilePart, Vec<usize>)> {
    let extra = data.extra.as_ref().unwrap();
    let mut out = Vec::new();
    for part in extra.parts.iter().filter(|p| p.first_poly >= 0) {
        let mut polys = Vec::new();
        let mut next = part.first_poly;
        while next >= 0 {
            let pp = &extra.part_polys[usize::try_from(next).unwrap()];
            polys.push(usize::try_from(pp.poly).unwrap());
            next = pp.next;
        }
        out.push((part.clone(), polys));
    }
    assert!(!out.is_empty(), "no door part has polygons");
    out
}

/// A flat tile with one area-12 door part (a box on the ground).
fn door_tile() -> TileInput {
    let mut input = flat_tile(10.0);
    let mut door = TileGroup::default();
    door.area = 12;
    let mut dp = TilePart::default();
    dp.object_id = 3;
    door.parts = vec![dp];
    door.verts = navmesh_world::door_box(24.0, 10.0);
    input.groups.push(door);
    input
}

#[test]
fn door_disabled_flag_follows_the_policy() {
    let p = world_params();
    let input = door_tile();
    let flags_of = |policy: DoorPolicy| {
        let mut o = BuildOptions::default();
        o.doors = policy;
        let data = built(build_tile(&p, &input, &o).unwrap()).0;
        let parts = part_polys(&data);
        let (part, polys) = &parts[0];
        assert_eq!(part.object_id, 3);
        polys
            .iter()
            .map(|i| data.polys[*i].flags)
            .collect::<Vec<_>>()
    };
    let closed = flags_of(DoorPolicy::Closed);
    assert!(
        closed.iter().all(|f| f & 2 != 0),
        "Closed disables area 12: {closed:?}"
    );
    for policy in [DoorPolicy::Open, DoorPolicy::Enabled] {
        let flags = flags_of(policy);
        assert!(flags.iter().all(|f| f & 2 == 0), "{policy:?}: {flags:?}");
    }
}

#[test]
fn stray_part_id_counted_not_crashing() {
    let p = world_params();
    let mut input = flat_tile(0.2);
    // Two trees on ground below the sea box, then a door part: the second tree's id (1) is beyond
    // the one-entry extras list once the sea box has relabelled its spans.
    let mut trees = TileGroup::default();
    trees.area = 10;
    let mut a = TilePart::default();
    a.object_id = 1;
    let mut b = TilePart::default();
    b.object_id = 2;
    trees.verts = navmesh_world::tree_prism(15.0, 25.0, 0.2);
    b.first_vertex = u32::try_from(trees.verts.len()).unwrap();
    trees
        .verts
        .extend(navmesh_world::tree_prism(35.0, 25.0, 0.2));
    trees.parts = vec![a, b];
    input.groups.push(trees);
    let mut door = TileGroup::default();
    door.area = 12;
    let mut dp = TilePart::default();
    dp.object_id = 3;
    door.parts = vec![dp];
    door.verts = navmesh_world::door_box(24.0, 0.2);
    input.groups.push(door);
    let (data, stray, _) = built(build_tile(&p, &input, &BuildOptions::default()).unwrap());
    assert!(stray >= 1, "stray {stray}");
    assert_ne!(data.polys.len(), 0);
}

#[test]
fn build_is_deterministic() {
    let (p, tiles) = synthetic_world();
    let input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap();
    let a = built(build_tile(&p, input, &BuildOptions::default()).unwrap()).0;
    let b = built(build_tile(&p, input, &BuildOptions::default()).unwrap()).0;
    assert_eq!(a.to_bytes(), b.to_bytes());
}

/// A tile whose terrain group is `layers` full-tile horizontal surfaces, 0.2 m apart.
fn stacked_tile(layers: usize) -> TileInput {
    let mut input = flat_tile(0.0);
    let one = terrain([0.0, 0.0], [50.0, 50.0]);
    let mut verts = Vec::new();
    for k in 0..layers {
        #[expect(clippy::cast_precision_loss, reason = "small test index")]
        let y = k as f32 * 0.2;
        verts.extend(one.iter().map(|v| [v[0], y, v[2]]));
    }
    if let Some(g) = input.groups.first_mut() {
        g.verts = verts;
    }
    input
}

/// Hostile input: stacked surfaces must hit the span budget and fail the tile, not allocate without
/// bound. The real budget is 2^26 spans (512 MB), too big for a test, so a small one proves the
/// refusal; the heightfield counts spans exactly like it does for the real limit.
#[test]
fn stacked_surfaces_exceed_the_span_budget() {
    use dayzmake_core::navmesh::internals::build_tile_with_span_limit;
    let p = world_params();
    let input = stacked_tile(100);
    let e = build_tile_with_span_limit(&p, &input, &BuildOptions::default(), 50_000).unwrap_err();
    let msg = e.to_string();
    assert!(msg.contains("too many spans ("), "{msg}");
    assert!(msg.contains("limit 50000)"), "{msg}");
    // The same input within a generous budget still builds (or is empty), proving the limit is the cause.
    assert!(build_tile_with_span_limit(&p, &input, &BuildOptions::default(), 50_000_000).is_ok());
}
