//! The hand-made two-tile `.nm` set of spec 13.1, built in code.
//!
//! Self-contained (std and `dayzmake_core` only), so other test crates can include it with
//! `#[path = ".../support/navmesh_tiles.rs"] mod navmesh_tiles;`.

#![allow(
    dead_code,
    reason = "shared by several test crates; not all of them use every helper"
)]

use dayzmake_core::navmesh::detour::{
    BvNode, Link, MeshHeader, NAVMESH_MAGIC, NAVMESH_VERSION, NavMeshParams, NavMeshSet, Obstacle,
    Poly, SetTile, TileData, TileExtra, TilePart, TilePartPoly, encode_poly_ref,
};

/// One tile at grid cell `(x, y)`: three vertices, one triangle, three zeroed links, two BV nodes,
/// one part, one part-poly and one obstacle.
pub fn tile(x: i32, y: i32) -> TileData {
    let mut header = MeshHeader::default();
    header.magic = NAVMESH_MAGIC;
    header.version = NAVMESH_VERSION;
    header.x = x;
    header.y = y;
    header.poly_count = 1;
    header.vert_count = 3;
    header.max_link_count = 3;
    header.bv_node_count = 2;
    header.off_mesh_base = 1;
    header.walkable_height = 1.5;
    header.walkable_radius = 0.3;
    header.walkable_climb = 0.45;
    header.bmin = [0.0, 0.0, 0.0];
    header.bmax = [50.0, 0.0, 50.0];
    header.bv_quant_factor = 4.0;
    header.extra_data_size = 52;

    let mut poly = Poly::default();
    poly.verts[0] = 0;
    poly.verts[1] = 1;
    poly.verts[2] = 2;
    poly.flags = 1;
    poly.vert_count = 3;
    poly.set_area(1);

    let mut part = TilePart::default();
    part.object_id = 0x8000_f000_0501_f61f;
    part.part_index = 0;
    part.first_poly = 0;
    let mut part_poly = TilePartPoly::default();
    part_poly.poly = 0;
    part_poly.next = -1;
    let mut obstacle = Obstacle::default();
    obstacle.pos = [10.0, 0.0, 10.0];
    obstacle.radius = 0.3;
    let mut extra = TileExtra::default();
    extra.parts = vec![part];
    extra.part_polys = vec![part_poly];
    extra.obstacles = vec![obstacle];

    let mut data = TileData::default();
    data.header = header;
    data.verts = vec![[0.0, 0.0, 0.0], [50.0, 0.0, 0.0], [50.0, 0.0, 50.0]];
    data.polys = vec![poly];
    data.links = vec![Link::default(); 3];
    data.bv_tree = vec![BvNode::default(); 2];
    data.extra = extra;
    data
}

/// A set with `tiles_x * tiles_z` slots (`maxTiles`) and no tiles.
pub fn empty_set(tiles_x: i32, tiles_z: i32) -> NavMeshSet {
    let mut params = NavMeshParams::default();
    params.tile_width = 50.0;
    params.tile_height = 50.0;
    params.max_tiles = tiles_x * tiles_z;
    params.max_polys = 1 << 20;
    let mut set = NavMeshSet::default();
    set.version = 9;
    set.tiles_x = tiles_x;
    set.tiles_z = tiles_z;
    set.params = params;
    set
}

/// A tile record for grid cell `(x, y)` in slot `slot`.
pub fn set_tile(slot: u32, data: TileData) -> SetTile {
    let mut t = SetTile::default();
    t.tile_ref = encode_poly_ref(1, slot, 0);
    t.data = data;
    t
}

/// The two-tile set of spec 13.1: `tilesX 2, tilesZ 1`, refs `encodePolyRef(1, y*2 + x, 0)`.
pub fn two_tile_set() -> NavMeshSet {
    let mut set = empty_set(2, 1);
    set.tiles = vec![set_tile(0, tile(0, 0)), set_tile(1, tile(1, 0))];
    set
}
