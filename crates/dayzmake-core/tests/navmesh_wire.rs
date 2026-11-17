//! Tests of the navmesh data-server wire format (spec 3 and 12.1).

#![cfg(feature = "navmesh")]

use std::io;
use std::path::Path;

use dayzmake_core::navmesh::wire::{
    CellClass, DEFAULT_PORT, LinkClass, PROTOCOL_VERSION, TileGroup, TileInput, TilePart,
    WorldBlock, WorldParams, encode_tile_request,
};

/// The synthetic world of spec 13.2.
fn synthetic_world() -> WorldParams {
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
    world.obstacles_raw = [1, 0, 0, 0];
    world.block_size = 4;
    world.filter_isolated_islands = true;
    world.filter_raw = [1, 0, 0, 0];
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

fn err_of<T>(r: io::Result<T>) -> String {
    match r {
        Ok(_) => String::from("<ok>"),
        Err(e) => e.to_string(),
    }
}

fn assert_err<T>(r: io::Result<T>, text: &str) {
    let msg = err_of(r);
    assert!(msg.contains(text), "expected {text:?}, got {msg:?}");
}

fn put(buf: &mut [u8], at: usize, bytes: [u8; 4]) {
    for (dst, src) in buf.iter_mut().skip(at).zip(bytes) {
        *dst = src;
    }
}

fn put_i32(buf: &mut [u8], at: usize, v: i32) {
    put(buf, at, v.to_le_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    put(buf, at, v.to_le_bytes());
}

fn put_f32(buf: &mut [u8], at: usize, v: f32) {
    put(buf, at, v.to_le_bytes());
}

#[test]
fn constants() {
    assert_eq!(PROTOCOL_VERSION, 10);
    assert_eq!(DEFAULT_PORT, 36533);
}

#[test]
fn world_params_roundtrip() {
    let world = synthetic_world();
    let bytes = world.encode();
    assert_eq!(bytes.len(), 176);
    assert_eq!(WorldParams::parse(&bytes).unwrap(), world);
    let mut message = PROTOCOL_VERSION.to_le_bytes().to_vec();
    message.extend_from_slice(&bytes);
    assert_eq!(WorldParams::parse_message(&message).unwrap(), world);
    assert!((world.cell_classes[1].cell_size() - 0.1).abs() <= f32::EPSILON);
    assert!((world.cell_classes[0].cell_size() - 0.25).abs() <= f32::EPSILON);
}

#[test]
fn world_params_keeps_raw_flag_bytes() {
    let mut bytes = synthetic_world().encode();
    // obstacles at 0x24, filterIsolatedIslands at 0x2C: flag 5, pad bytes 1, 2, 3.
    bytes[0x24..0x28].copy_from_slice(&[5, 1, 2, 3]);
    bytes[0x2C..0x30].copy_from_slice(&[0, 9, 8, 7]);
    let parsed = WorldParams::parse(&bytes).unwrap();
    assert!(parsed.world.obstacles);
    assert!(!parsed.world.filter_isolated_islands);
    assert_eq!(parsed.encode(), bytes);
    // A bool changed after parsing wins over the raw byte.
    let mut changed = parsed.clone();
    changed.world.obstacles = false;
    changed.world.filter_isolated_islands = true;
    let out = changed.encode();
    assert_eq!(out[0x24..0x28], [0, 1, 2, 3]);
    assert_eq!(out[0x2C..0x30], [1, 9, 8, 7]);
}

#[test]
fn world_params_default_bools_encode() {
    let bytes = synthetic_world().encode();
    assert_eq!(bytes[0x24..0x28], [1, 0, 0, 0]);
    assert_eq!(bytes[0x2C..0x30], [1, 0, 0, 0]);
}

#[test]
fn world_params_rejects() {
    let good = synthetic_world().encode();
    let mut message = PROTOCOL_VERSION.to_le_bytes().to_vec();
    message.extend_from_slice(&good);

    let mut v9 = message.clone();
    put_u32(&mut v9, 0, 9);
    assert_err(
        WorldParams::parse_message(&v9),
        "server protocol version 9, expected 10",
    );
    assert_err(
        WorldParams::parse_message(&[1, 2]),
        "world params truncated",
    );

    assert_err(
        WorldParams::parse(&good[..good.len() - 1]),
        "message truncated",
    );
    let mut trailing = good.clone();
    trailing.push(0);
    assert_err(
        WorldParams::parse(&trailing),
        "unexpected bytes after world params",
    );

    // Cell class count at 84, link count after the two cell classes.
    let mut p = good.clone();
    put_u32(&mut p, 84, 65);
    assert_err(WorldParams::parse(&p), "implausible cell class count 65");
    let mut p = good.clone();
    put_u32(&mut p, 84 + 4 + 32, 257);
    assert_err(WorldParams::parse(&p), "implausible link count 257");

    let mut p = good.clone();
    put_i32(&mut p, 0x10, 0);
    assert_err(WorldParams::parse(&p), "invalid tile grid");
    let mut p = good.clone();
    put_i32(&mut p, 0x10, 1 << 15);
    put_i32(&mut p, 0x14, 1 << 15);
    assert_err(WorldParams::parse(&p), "invalid tile grid");
    let mut p = good.clone();
    put_f32(&mut p, 0x08, f32::NAN);
    assert_err(WorldParams::parse(&p), "invalid tile size");

    // First cell class at 88: cells, cells2, width, unused.
    let mut p = good.clone();
    put_i32(&mut p, 88, 0);
    assert_err(WorldParams::parse(&p), "invalid cell class");
    let mut p = good.clone();
    put_f32(&mut p, 88 + 8, f32::NAN);
    assert_err(WorldParams::parse(&p), "invalid cell class");
    let mut p = good;
    put_i32(&mut p, 88, 4096);
    assert_err(WorldParams::parse(&p), "implausible cell count");
}

#[test]
fn world_params_huge_class_counts_are_cheap() {
    let mut p = synthetic_world().encode();
    // 64 cell classes claimed but only two present.
    put_u32(&mut p, 84, 64);
    assert_err(WorldParams::parse(&p), "message truncated");
}

fn synthetic_tile() -> TileInput {
    let mut terrain = TileGroup::default();
    terrain.area = 1;
    let mut part = TilePart::default();
    part.object_id = 0;
    terrain.parts = vec![part];
    terrain.verts = vec![
        [0.0, 10.0, 0.0],
        [50.0, 10.0, 0.0],
        [50.0, 10.5, 50.0],
        [0.0, 10.0, 0.0],
        [50.0, 10.5, 50.0],
        [0.0, 10.0, 50.0],
    ];

    let mut door = TileGroup::default();
    door.area = 12;
    let mut p0 = TilePart::default();
    p0.part_index = 0;
    p0.object_id = 0x8000_f000_0501_f61f;
    p0.first_vertex = 0;
    let mut p1 = TilePart::default();
    p1.part_index = 1;
    p1.object_id = 0x8000_f000_0501_f61f;
    p1.first_vertex = 3;
    door.parts = vec![p0, p1];
    door.verts = vec![
        [1.0, 10.0, 1.0],
        [2.0, 10.0, 1.0],
        [2.0, 12.0, 1.0],
        [3.0, 10.0, 1.0],
        [4.0, 10.0, 1.0],
        [4.0, 12.0, 1.0],
    ];

    let mut ladder = TileGroup::default();
    ladder.area = 15;
    ladder.verts = vec![[63.8, 8.0, 67.0], [63.8, 12.5, 67.0]];

    let mut tile = TileInput::default();
    tile.x = 1;
    tile.z = 1;
    tile.cell_class = 1;
    tile.bmin = [50.0, 8.0, 50.0];
    tile.bmax = [100.0, 12.5, 100.0];
    tile.groups = vec![terrain, door, ladder];
    tile
}

#[test]
fn tile_input_roundtrip_and_ranges() {
    let tile = synthetic_tile();
    let bytes = tile.encode();
    assert_eq!(TileInput::parse(&bytes).unwrap(), tile);
    assert_eq!(tile.part_vertex_range(0, 0), Some(0..6));
    assert_eq!(tile.part_vertex_range(1, 0), Some(0..3));
    assert_eq!(tile.part_vertex_range(1, 1), Some(3..6));
    assert_eq!(tile.part_vertex_range(1, 2), None);
    assert_eq!(tile.part_vertex_range(7, 0), None);
    // A group without parts has no part ranges.
    assert_eq!(tile.part_vertex_range(2, 0), None);
}

/// Byte offset of group 0's `partCount` in an encoded tile with one group: 12 header + 4 group
/// count + 24 bounds + 4 area.
const G0_PART_COUNT: usize = 4 + 4 + 4 + 4 + 24 + 4;

fn one_group_tile(parts: &[(u32, u64, u32)], verts: usize) -> Vec<u8> {
    let mut group = TileGroup::default();
    group.area = 1;
    group.parts = parts
        .iter()
        .map(|&(i, o, f)| {
            let mut p = TilePart::default();
            p.part_index = i;
            p.object_id = o;
            p.first_vertex = f;
            p
        })
        .collect();
    group.verts = vec![[1.0, 2.0, 3.0]; verts];
    let mut tile = TileInput::default();
    tile.groups = vec![group];
    tile.encode()
}

#[test]
fn tile_input_rejects() {
    let good = synthetic_tile().encode();

    let mut p = good.clone();
    put_u32(&mut p, 12, 65537);
    assert_err(TileInput::parse(&p), "implausible group count 65537");

    let mut p = one_group_tile(&[], 0);
    put_u32(&mut p, G0_PART_COUNT, 1_048_577);
    assert_err(TileInput::parse(&p), "implausible part count 1048577");

    // vertexCount follows the (empty) part list.
    let at = G0_PART_COUNT + 4;
    let mut p = one_group_tile(&[], 0);
    put_i32(&mut p, at, -1);
    assert_err(TileInput::parse(&p), "implausible vertex count");
    let mut p = one_group_tile(&[], 0);
    put_i32(&mut p, at, (1 << 26) + 1);
    assert_err(TileInput::parse(&p), "implausible vertex count");

    // firstVertex beyond the next part's.
    let p = one_group_tile(&[(0, 0, 4), (1, 0, 2)], 6);
    assert_err(TileInput::parse(&p), "part vertex range out of bounds");
    // firstVertex beyond the vertex count.
    let p = one_group_tile(&[(0, 0, 7)], 6);
    assert_err(TileInput::parse(&p), "part vertex range out of bounds");

    let mut p = one_group_tile(&[], 3);
    let first_vertex = G0_PART_COUNT + 4 + 4;
    put_f32(&mut p, first_vertex + 4, f32::NAN);
    assert_err(TileInput::parse(&p), "non-finite vertex");
    let mut p = one_group_tile(&[], 3);
    put_f32(&mut p, first_vertex, f32::INFINITY);
    assert_err(TileInput::parse(&p), "non-finite vertex");

    assert_err(
        TileInput::parse(&good[..good.len() - 1]),
        "message truncated",
    );
    assert_err(TileInput::parse(&[]), "message truncated");
    let mut p = good;
    p.push(0);
    assert_err(TileInput::parse(&p), "unexpected bytes after tile record");
}

#[test]
fn tile_input_huge_count_is_cheap() {
    let mut p = one_group_tile(&[], 0);
    put_i32(&mut p, G0_PART_COUNT + 4, 1 << 26);
    p.extend_from_slice(&[0u8; 10]);
    assert_err(TileInput::parse(&p), "message truncated");

    // A group count of 65536 with no bytes behind it.
    let mut p = one_group_tile(&[], 0);
    put_u32(&mut p, 12, 65536);
    assert_err(TileInput::parse(&p), "message truncated");
}

#[test]
fn tile_request_bytes() {
    let mut world = WorldBlock::default();
    world.tile_width = 50.0;
    world.tile_height = 50.0;
    let request = encode_tile_request(2, 3, &world);
    let mut expected = Vec::new();
    expected.extend_from_slice(&2i32.to_le_bytes());
    expected.extend_from_slice(&3i32.to_le_bytes());
    for v in [100f32, 150.0, 150.0, 200.0] {
        expected.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(request.to_vec(), expected);
}

/// Seeds of the `navmesh_wire` fuzz target: the encoded test values (the target feeds every input to
/// both parsers). Run `cargo test -p dayzmake-core --test navmesh_wire
/// write_fuzz_seeds -- --ignored` to regenerate `fuzz/corpus/navmesh_wire/`.
#[test]
#[ignore = "regenerates fuzz/corpus/navmesh_wire"]
fn write_fuzz_seeds() -> io::Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/navmesh_wire");
    std::fs::create_dir_all(&dir)?;
    let mut message = PROTOCOL_VERSION.to_le_bytes().to_vec();
    message.extend_from_slice(&synthetic_world().encode());
    let seeds: [(&str, Vec<u8>); 5] = [
        ("world_params", message),
        ("world_params_body", synthetic_world().encode()),
        ("tile_input", synthetic_tile().encode()),
        ("tile_one_group", one_group_tile(&[(0, 7, 0), (1, 7, 3)], 6)),
        ("tile_header_only", one_group_tile(&[], 0)[..28].to_vec()),
    ];
    for (name, data) in seeds {
        std::fs::write(dir.join(name), data)?;
    }
    Ok(())
}
