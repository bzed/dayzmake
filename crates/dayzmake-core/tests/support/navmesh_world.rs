//! The synthetic navmesh world of spec 13.2: 3 x 3 tiles of 50 x 50 m with rolling ground, a pond, a
//! crate and a low wall, a room with a doorway and two door parts, five trees and a ladder.
//!
//! Self-contained (std and `dayzmake_core` only), so other test crates can include it with
//! `#[path = ".../support/navmesh_world.rs"] mod navmesh_world;`.
//!
//! The triangles are in server winding: `walkable(...)` (normal `cross(e2, e1)`) faces up for the
//! terrain, and the build passes `(v0, v2, v1)` to the rasteriser.

#![allow(
    dead_code,
    reason = "shared by several test crates; not all of them use every helper"
)]

use dayzmake_core::navmesh::wire::{
    CellClass, LinkClass, TileGroup, TileInput, TilePart, WorldBlock, WorldParams,
};

/// World parameters of spec 13.2: 3 x 3 tiles, two cell classes, one link class.
pub fn world_params() -> WorldParams {
    let mut world = WorldBlock::default();
    world.world_size_x = 150.0;
    world.world_size_z = 150.0;
    world.tile_width = 50.0;
    world.tile_height = 50.0;
    world.tiles_x = 3;
    world.tiles_z = 3;
    world.deep_sea_height = 0.75;
    world.deep_water_depth = 1.5;
    world.contour_max_error = 0.3;
    world.obstacles = true;
    world.block_size = 4;
    world.filter_isolated_islands = true;
    world.seed_position = [60.0, 10.0, 60.0];
    world.stand_height = 1.5;
    world.crouch_height = 1.0;
    world.prone_height = 0.5;
    world.diameter = 0.6;
    world.max_step_height = 0.45;
    world.max_slope = 45.0;
    let mut c0 = CellClass::default();
    c0.cells = 200;
    c0.cells2 = 200;
    c0.width = 50.0;
    let mut c1 = CellClass::default();
    c1.cells = 500;
    c1.cells2 = 500;
    c1.width = 50.0;
    let mut link = LinkClass::default();
    link.index = 0;
    link.name_hash = 0x1234_5678;
    link.type_id = -1;
    link.jump_height = 1.2;
    link.jump_length = 1.8;
    link.min_center_height = 0.4;
    link.agent_height = 1.5;
    link.max_step_height = 0.45;
    link.agent_radius = 0.3;
    link.flags = 0x100;
    link.area = 48;
    link.color = -1;
    let mut params = WorldParams::default();
    params.world = world;
    params.cell_classes = vec![c0, c1];
    params.link_classes = vec![link];
    params
}

/// The rolling ground of the world: about 10 m, minus a 3 m pond hollow within 12 m of (30, 105).
pub fn ground(x: f32, z: f32) -> f32 {
    let mut y = 10.0 + 1.5 * (0.05 * x).sin() * (0.07 * z).cos();
    let d = ((x - 30.0) * (x - 30.0) + (z - 105.0) * (z - 105.0)).sqrt();
    if d < 12.0 {
        y -= 3.0 * (1.0 - d / 12.0);
    }
    y
}

type V = [f32; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// One triangle in server winding: its normal `cross(e2, e1)` faces `outward`.
fn tri(out: &mut Vec<V>, a: V, b: V, c: V, outward: V) {
    let n = cross(sub(c, a), sub(b, a));
    if dot(n, outward) >= 0.0 {
        out.extend([a, b, c]);
    } else {
        out.extend([a, c, b]);
    }
}

/// A quad `a b c d` (in loop order) as two triangles facing `outward`.
fn quad(out: &mut Vec<V>, a: V, b: V, c: V, d: V, outward: V) {
    tri(out, a, b, c, outward);
    tri(out, a, c, d, outward);
}

/// An axis-aligned box, outward facing.
fn cuboid(min: V, max: V) -> Vec<V> {
    let mut out = Vec::new();
    let p = |x: usize, y: usize, z: usize| -> V {
        [
            if x == 0 { min[0] } else { max[0] },
            if y == 0 { min[1] } else { max[1] },
            if z == 0 { min[2] } else { max[2] },
        ]
    };
    quad(
        &mut out,
        p(0, 1, 0),
        p(1, 1, 0),
        p(1, 1, 1),
        p(0, 1, 1),
        [0.0, 1.0, 0.0],
    );
    quad(
        &mut out,
        p(0, 0, 0),
        p(1, 0, 0),
        p(1, 0, 1),
        p(0, 0, 1),
        [0.0, -1.0, 0.0],
    );
    quad(
        &mut out,
        p(0, 0, 0),
        p(0, 1, 0),
        p(0, 1, 1),
        p(0, 0, 1),
        [-1.0, 0.0, 0.0],
    );
    quad(
        &mut out,
        p(1, 0, 0),
        p(1, 1, 0),
        p(1, 1, 1),
        p(1, 0, 1),
        [1.0, 0.0, 0.0],
    );
    quad(
        &mut out,
        p(0, 0, 0),
        p(1, 0, 0),
        p(1, 1, 0),
        p(0, 1, 0),
        [0.0, 0.0, -1.0],
    );
    quad(
        &mut out,
        p(0, 0, 1),
        p(1, 0, 1),
        p(1, 1, 1),
        p(0, 1, 1),
        [0.0, 0.0, 1.0],
    );
    out
}

/// A hexagonal prism of `radius` around `(cx, cz)` from `y0` to `y1`.
fn prism(cx: f32, cz: f32, y0: f32, y1: f32, radius: f32) -> Vec<V> {
    let mut out = Vec::new();
    let ring: Vec<(f32, f32)> = (0..6_u8)
        .map(|i| {
            let a = f32::from(i) * std::f32::consts::FRAC_PI_3;
            (cx + radius * a.cos(), cz + radius * a.sin())
        })
        .collect();
    for (&(x0, z0), &(x1, z1)) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let mid = [f32::midpoint(x0, x1) - cx, 0.0, f32::midpoint(z0, z1) - cz];
        quad(
            &mut out,
            [x0, y0, z0],
            [x1, y0, z1],
            [x1, y1, z1],
            [x0, y1, z0],
            mid,
        );
        tri(
            &mut out,
            [cx, y1, cz],
            [x0, y1, z0],
            [x1, y1, z1],
            [0.0, 1.0, 0.0],
        );
        tri(
            &mut out,
            [cx, y0, cz],
            [x0, y0, z0],
            [x1, y0, z1],
            [0.0, -1.0, 0.0],
        );
    }
    out
}

/// Whether the box `[min, max]` (xz) touches the tile `[tmin, tmax]` grown by 10 m.
fn near(tmin: [f32; 2], tmax: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    min[0] <= tmax[0] + 10.0
        && max[0] >= tmin[0] - 10.0
        && min[1] <= tmax[1] + 10.0
        && max[1] >= tmin[1] - 10.0
}

fn group(area: u32, verts: Vec<V>) -> TileGroup {
    let mut g = TileGroup::default();
    g.area = area;
    g.verts = verts;
    g
}

fn part(part_index: u32, object_id: u64, first_vertex: usize) -> TilePart {
    let mut p = TilePart::default();
    p.part_index = part_index;
    p.object_id = object_id;
    p.first_vertex = u32::try_from(first_vertex).unwrap_or(u32::MAX);
    p
}

/// Terrain covering the tile and 10 m beyond it, on a 2 m grid, in server winding (faces up).
pub fn terrain(tmin: [f32; 2], tmax: [f32; 2]) -> Vec<V> {
    let mut out = Vec::new();
    // Tiles are 50 m wide: 10 m on each side makes 70 m, 35 cells of 2 m.
    for (lo, hi) in tmin.iter().zip(&tmax) {
        assert!((f32::midpoint(hi - lo, 20.0) - 35.0).abs() < f32::EPSILON * 64.0);
    }
    for iz in 0..35_u8 {
        for ix in 0..35_u8 {
            let x0 = tmin[0] - 10.0 + 2.0 * f32::from(ix);
            let z0 = tmin[1] - 10.0 + 2.0 * f32::from(iz);
            let (x1, z1) = (x0 + 2.0, z0 + 2.0);
            let c = |x: f32, z: f32| -> V { [x, ground(x, z), z] };
            quad(
                &mut out,
                c(x0, z0),
                c(x1, z0),
                c(x1, z1),
                c(x0, z1),
                [0.0, 1.0, 0.0],
            );
        }
    }
    out
}

/// The input of tile `(x, z)`.
pub fn tile_input(x: i32, z: i32) -> TileInput {
    let coord = |c: i32| f32::from(i16::try_from(c).unwrap_or(0)) * 50.0;
    let tmin = [coord(x), coord(z)];
    let tmax = [tmin[0] + 50.0, tmin[1] + 50.0];
    let mut groups = vec![group(1, terrain(tmin, tmax))];

    // The pond water (area 2): a quad at y 9.6, half-width 9.5.
    if near(tmin, tmax, [20.5, 95.5], [39.5, 114.5]) {
        let mut v = Vec::new();
        let y = 9.6;
        quad(
            &mut v,
            [20.5, y, 95.5],
            [39.5, y, 95.5],
            [39.5, y, 114.5],
            [20.5, y, 114.5],
            [0.0, 1.0, 0.0],
        );
        groups.push(group(2, v));
    }

    // Area 9: the crate and the low wall.
    let mut boxes = Vec::new();
    if near(tmin, tmax, [64.0, 64.0], [70.0, 70.0]) {
        boxes.extend(cuboid([64.0, 8.0, 64.0], [70.0, 12.5, 70.0]));
    }
    if near(tmin, tmax, [55.0, 80.0], [65.0, 80.3]) {
        boxes.extend(cuboid([55.0, 8.0, 80.0], [65.0, 10.9, 80.3]));
    }
    if !boxes.is_empty() {
        groups.push(group(9, boxes));
    }

    // Area 8: the room (80..90, 60..68) with a doorway 84.5..85.5 in the south wall, and its floor.
    let in_room = near(tmin, tmax, [80.0, 60.0], [90.0, 68.0]);
    if in_room {
        let mut v = Vec::new();
        for (min, max) in [
            ([80.0, 10.0, 60.0], [84.5, 13.0, 60.3]),
            ([85.5, 10.0, 60.0], [90.0, 13.0, 60.3]),
            ([80.0, 10.0, 67.7], [90.0, 13.0, 68.0]),
            ([80.0, 10.0, 60.3], [80.3, 13.0, 67.7]),
            ([89.7, 10.0, 60.3], [90.0, 13.0, 67.7]),
        ] {
            v.extend(cuboid(min, max));
        }
        quad(
            &mut v,
            [80.0, 10.6, 60.0],
            [90.0, 10.6, 60.0],
            [90.0, 10.6, 68.0],
            [80.0, 10.6, 68.0],
            [0.0, 1.0, 0.0],
        );
        groups.push(group(8, v));

        // The door parts: open (area 12, a thin box at the hinge side), closed (area 13, filling the
        // doorway).
        let id = 0x8000_f000_0501_f61f_u64;
        let mut open = group(12, cuboid([84.5, 10.0, 59.9], [84.7, 12.5, 60.4]));
        open.parts = vec![part(0, id, 0)];
        let mut closed = group(13, cuboid([84.5, 10.0, 60.0], [85.5, 12.5, 60.3]));
        closed.parts = vec![part(1, id, 0)];
        groups.push(open);
        groups.push(closed);
    }

    // Area 10: five trees, hexagonal prisms of radius 0.35 and 6 m height.
    let mut trees = group(10, Vec::new());
    for (i, (tx, tz)) in [
        (20.0, 20.0),
        (40.0, 70.0),
        (110.0, 30.0),
        (120.0, 120.0),
        (75.0, 130.0),
    ]
    .into_iter()
    .enumerate()
    {
        if near(tmin, tmax, [tx, tz], [tx, tz]) {
            let y = ground(tx, tz);
            trees.parts.push(part(
                0,
                0x100 + u64::try_from(i).unwrap_or(0),
                trees.verts.len(),
            ));
            trees.verts.extend(prism(tx, tz, y, y + 6.0, 0.35));
        }
    }
    if !trees.parts.is_empty() {
        groups.push(trees);
    }

    // Area 15: the ladder along the crate's west face, one vertex pair from the ground to y 12.5.
    if near(tmin, tmax, [63.8, 67.0], [63.8, 67.0]) {
        groups.push(group(
            15,
            vec![[63.8, ground(63.8, 67.0), 67.0], [63.8, 12.5, 67.0]],
        ));
    }

    let mut input = TileInput::default();
    input.x = x;
    input.z = z;
    input.cell_class = u32::from(x == 1 && z == 1);
    input.bmin = [tmin[0], 0.0, tmin[1]];
    input.bmax = [tmax[0], 30.0, tmax[1]];
    input.groups = groups;
    input
}

/// The world of spec 13.2 and the inputs of its 9 tiles, in x-then-z order.
pub fn synthetic_world() -> (WorldParams, Vec<TileInput>) {
    let tiles = (0..3)
        .flat_map(|x| (0..3).map(move |z| tile_input(x, z)))
        .collect();
    (world_params(), tiles)
}

/// A hexagonal tree prism of radius 0.35 and 6 m height standing on `y` at `(x, z)`.
pub fn tree_prism(x: f32, z: f32, y: f32) -> Vec<[f32; 3]> {
    prism(x, z, y, y + 6.0, 0.35)
}

/// A door box 1 m wide, 0.3 m deep and 2.5 m high standing on `y`, from `x` to `x + 1` at z 25.
pub fn door_box(x: f32, y: f32) -> Vec<[f32; 3]> {
    cuboid([x, y, 25.0], [x + 1.0, y + 2.5, 25.3])
}
