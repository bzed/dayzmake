//! Tests of the jump, climb and ladder links of a tile build (spec 6, 13.2).

#![cfg(feature = "navmesh")]

use dayzmake_core::navmesh::build::{BuildOptions, TileOutcome, build_tile};
use dayzmake_core::navmesh::detour::{OffMeshConnection, TileData};
use dayzmake_core::navmesh::wire::{TileGroup, TileInput, WorldParams};

#[path = "support/navmesh_world.rs"]
mod navmesh_world;
use navmesh_world::{synthetic_world, terrain, world_params};

/// The extras index of the hide part the tests add: the two door parts of the room come first.
const HIDE_PART: usize = 2;

#[expect(clippy::panic, reason = "test helper")]
fn built(outcome: TileOutcome) -> TileData {
    match outcome {
        TileOutcome::Built { data, .. } => data,
        other => panic!("expected a built tile, got {other:?}"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// A flat tile at 10 m with a ladder from the ground to 12 m at (20, 20).
fn ladder_tile() -> TileInput {
    let mut input = TileInput::default();
    input.bmin = [0.0, 0.0, 0.0];
    input.bmax = [50.0, 30.0, 50.0];
    let mut ground = TileGroup::default();
    ground.area = 1;
    ground.verts = terrain([0.0, 0.0], [50.0, 50.0])
        .into_iter()
        .map(|v| [v[0], 10.0, v[2]])
        .collect();
    input.groups.push(ground);
    let mut ladder = TileGroup::default();
    ladder.area = 15;
    ladder.verts = vec![[20.0, 10.0, 20.0], [20.0, 12.0, 20.0]];
    input.groups.push(ladder);
    input
}

/// The off-mesh connections of a tile with the flags and area of their polygons.
#[expect(
    clippy::indexing_slicing,
    reason = "test helper on a tile it just built"
)]
fn connections(data: &TileData) -> Vec<(OffMeshConnection, u16, u8)> {
    data.off_mesh
        .iter()
        .map(|c| {
            let poly = &data.polys[usize::from(c.poly)];
            (c.clone(), poly.flags, poly.area())
        })
        .collect()
}

#[expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper on the synthetic world"
)]
fn wall_tile(p: &WorldParams) -> (TileData, f32) {
    let (_, tiles) = synthetic_world();
    let input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap();
    let data = built(build_tile(p, input, &BuildOptions::default()).unwrap());
    // The tile uses cell class 1 (cs 0.1).
    (data, p.cell_classes[1].cell_size())
}

#[test]
fn ladder_link() {
    let mut p = world_params();
    p.link_classes.clear();
    let data = built(build_tile(&p, &ladder_tile(), &BuildOptions::default()).unwrap());
    let all = connections(&data);
    assert_eq!(all.len(), 1);
    let (c, flags, area) = &all[0];
    assert_eq!((*flags, *area), (0x40, 15));
    assert_eq!(c.flags, 1, "bidirectional");
    assert!(close(c.rad, 1.0) && close(c.center_height, 0.0));
    assert!(close(c.pos[0], 20.0) && close(c.pos[1], 10.0) && close(c.pos[2], 20.0));
    assert!(close(c.pos[3], 20.0) && close(c.pos[4], 12.0) && close(c.pos[5], 20.0));
    assert_eq!(c.user_id, 0);
}

#[test]
fn wall_gives_jump_links() {
    let p = world_params();
    let (data, cs) = wall_tile(&p);
    let jump_length = p.link_classes[0].jump_length;
    let all = connections(&data);
    let jumps: Vec<_> = all.iter().filter(|(_, _, area)| *area == 48).collect();
    assert!(!jumps.is_empty(), "no jump links: {all:?}");
    for (c, flags, _) in &jumps {
        assert_eq!(*flags, 0x100);
        assert_eq!(c.user_id, 0);
        let dx = c.pos[3] - c.pos[0];
        let dz = c.pos[5] - c.pos[2];
        let horizontal = dx.hypot(dz);
        assert!(
            (horizontal - jump_length).abs() <= cs,
            "horizontal length {horizontal} vs {jump_length}"
        );
    }
    // The ladder of the tile comes first.
    assert_eq!(all[0].2, 15);
}

#[test]
fn no_links_option() {
    let p = world_params();
    let (_, tiles) = synthetic_world();
    let input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap();
    let mut o = BuildOptions::default();
    o.links = false;
    let data = built(build_tile(&p, input, &o).unwrap());
    assert_eq!(data.off_mesh.len(), 0);
    assert_eq!(data.header.off_mesh_con_count, 0);
}

#[test]
fn no_link_classes_only_ladders() {
    let mut p = world_params();
    p.link_classes.clear();
    let (_, tiles) = synthetic_world();
    let input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap();
    let data = built(build_tile(&p, input, &BuildOptions::default()).unwrap());
    let all = connections(&data);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!((all[0].1, all[0].2), (0x40, 15));
}

#[test]
fn links_are_deterministic() {
    let p = world_params();
    let (a, _) = wall_tile(&p);
    let (b, _) = wall_tile(&p);
    assert_eq!(a, b);
}

#[test]
fn hostile_link_class_is_an_error_not_an_allocation() {
    let mut p = world_params();
    p.link_classes[0].jump_length = 1.0e9;
    let (_, tiles) = synthetic_world();
    let input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap();
    let e = build_tile(&p, input, &BuildOptions::default())
        .map(|_| ())
        .unwrap_err();
    assert!(e.to_string().contains("samples"), "{e}");
}

/// The wall tile with a hide part (area 14) laid over the low wall: a box a little wider than the wall
/// that paints the wall's spans with the part id.
#[expect(clippy::unwrap_used, reason = "test helper on the synthetic world")]
fn hide_wall_input() -> TileInput {
    let (_, tiles) = synthetic_world();
    let mut input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap().clone();
    let mut hide = TileGroup::default();
    hide.area = 14;
    hide.verts = navmesh_world::cuboid([54.8, 8.0, 79.9], [65.2, 11.0, 80.4]);
    let mut part = dayzmake_core::navmesh::wire::TilePart::default();
    part.part_index = 0;
    part.object_id = 0x8000_f000_0001_0000;
    hide.parts = vec![part];
    input.groups.push(hide);
    input
}

/// The polygons the extra section lists for part `index`.
#[expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper on a tile it just built"
)]
fn part_polys(data: &TileData, index: usize) -> Vec<u32> {
    let extra = data.extra.as_ref().unwrap();
    let mut out = Vec::new();
    let mut at = extra.parts[index].first_poly;
    while let Ok(i) = usize::try_from(at) {
        out.push(extra.part_polys[i].poly);
        at = extra.part_polys[i].next;
    }
    out
}

/// Spec 7.2, ruling R17: a jump over a hide part joins that part's polygon list with its off-mesh
/// polygon, which lies after the ground polygons.
#[test]
#[expect(clippy::cast_sign_loss, reason = "test on the synthetic world")]
fn links_over_a_hide_part_join_its_polygons() {
    let p = world_params();
    let data = built(build_tile(&p, &hide_wall_input(), &BuildOptions::default()).unwrap());
    let base = data.header.off_mesh_base as u32;
    let polys = part_polys(&data, HIDE_PART);
    let jumps: Vec<u32> = data
        .off_mesh
        .iter()
        .filter(|c| data.polys[usize::from(c.poly)].area() == 48)
        .map(|c| u32::from(c.poly))
        .collect();
    assert!(!jumps.is_empty(), "no jump links");
    assert!(
        polys.iter().any(|poly| *poly >= base),
        "no off-mesh polygon in the hide part: {polys:?}, base {base}"
    );
    for poly in polys.iter().filter(|poly| **poly >= base) {
        assert!(jumps.contains(poly), "{poly} is not a jump link: {jumps:?}");
    }
    // The list stays ascending, as the section needs it.
    assert!(polys.windows(2).all(|w| w[0] < w[1]), "{polys:?}");
}

/// Without a hide part no off-mesh polygon is listed in any part.
#[test]
#[expect(clippy::cast_sign_loss, reason = "test on the synthetic world")]
fn links_without_a_hide_part_join_nothing() {
    let p = world_params();
    let (_, tiles) = synthetic_world();
    let input = tiles.iter().find(|t| (t.x, t.z) == (1, 1)).unwrap();
    let data = built(build_tile(&p, input, &BuildOptions::default()).unwrap());
    let base = data.header.off_mesh_base as u32;
    let extra = data.extra.as_ref().unwrap();
    assert!(extra.part_polys.iter().all(|e| e.poly < base));
}
