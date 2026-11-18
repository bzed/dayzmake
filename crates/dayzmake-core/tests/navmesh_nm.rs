//! Tests of the `.nm` set file and the Detour tile layout (spec 7.2, 8.1, 9, 13.1).

#![cfg(feature = "navmesh")]
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::fs;
use std::io;

use dayzmake_core::navmesh::detour::{
    NavMeshSet, SET_MAGIC, SET_VERSION, SET_VERSION_OLD, TileData, TileExtra, decode_poly_ref,
    encode_poly_ref,
};

#[path = "support/navmesh_tiles.rs"]
mod navmesh_tiles;
use navmesh_tiles::{empty_set, set_tile, tile, two_tile_set};

fn to_bytes(set: &NavMeshSet) -> Vec<u8> {
    let mut out = Vec::new();
    set.write(&mut out).expect("write to a Vec");
    out
}

fn read(bytes: &[u8]) -> io::Result<NavMeshSet> {
    NavMeshSet::read(&mut &bytes[..])
}

fn err_of<T>(r: io::Result<T>) -> String {
    match r {
        Ok(_) => String::from("<ok>"),
        Err(e) => e.to_string(),
    }
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_i32(b: &mut [u8], at: usize, v: i32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn poly_ref_vectors() {
    assert_eq!(encode_poly_ref(1, 1, 0), 0x0001_0000_0010_0000);
    assert_eq!(decode_poly_ref(encode_poly_ref(1, 5, 7)), (1, 5, 7));
    assert_eq!(
        decode_poly_ref(encode_poly_ref(0xFFFF, 0x0FFF_FFFF, 0xF_FFFF)),
        (0xFFFF, 0x0FFF_FFFF, 0xF_FFFF)
    );
}

#[test]
fn constants() {
    assert_eq!(SET_MAGIC, i32::from_le_bytes(*b"TESM"));
    assert_eq!(SET_VERSION, 9);
    assert_eq!(SET_VERSION_OLD, 7);
}

#[test]
fn two_tile_file() {
    let set = two_tile_set();
    let bytes = to_bytes(&set);
    assert_eq!(&bytes[..4], b"TESM");
    assert_eq!(&bytes[48 + 8 + 16..][..4], b"VAND");
    assert_eq!(
        set.tiles[0].data.extra.as_ref().map(TileExtra::byte_size),
        Some(52)
    );
    assert_eq!(set.tiles[0].data.header.extra_data_size, 52);
    // The header of the first blob carries extraDataSize at offset 100.
    let blob = 48 + 8 + 16;
    assert_eq!(&bytes[blob + 100..][..4], &52i32.to_le_bytes());
    let back = read(&bytes).expect("reads");
    assert_eq!(back, set);
    assert_eq!(to_bytes(&back), bytes);
    // 104 + 3*12 + 32 + 3*16 + 2*16 + 52 = 304 bytes per blob.
    assert_eq!(bytes.len(), 48 + 8 + 2 * (16 + 304));
}

#[test]
fn tile_blob_round_trips() {
    let t = tile(3, 4);
    let blob = t.to_bytes();
    assert_eq!(blob.len(), 304);
    assert_eq!(TileData::parse(&blob).expect("parses"), t);
}

#[test]
fn tile_without_extra_has_no_extra_section_and_an_empty_one_has_twelve_bytes() {
    let mut t = tile(0, 0);
    t.extra = None;
    t.header.extra_data_size = 0;
    let blob = t.to_bytes();
    assert_eq!(blob.len(), 304 - 52);
    let back = TileData::parse(&blob).expect("parses");
    assert_eq!(back.header.extra_data_size, 0);
    assert_eq!(back.extra, None);
    assert_eq!(back.to_bytes(), blob);

    // An empty section is 12 bytes of zero counts and survives a round trip.
    let mut e = tile(0, 0);
    e.extra = Some(TileExtra::default());
    let blob = e.to_bytes();
    assert_eq!(blob.len(), 304 - 52 + 12);
    assert_eq!(&blob[100..104], &12i32.to_le_bytes());
    let back = TileData::parse(&blob).expect("parses");
    assert_eq!(back.extra, Some(TileExtra::default()));
    assert_eq!(back.header.extra_data_size, 12);
    assert_eq!(back.to_bytes(), blob);

    // A foreign blob built by hand: extraDataSize 12, zero counts.
    let mut hand = tile(0, 0).to_bytes();
    hand.truncate(304 - 52);
    hand[100..104].copy_from_slice(&12i32.to_le_bytes());
    hand.extend_from_slice(&[0; 12]);
    assert_eq!(TileData::parse(&hand).expect("parses").to_bytes(), hand);
}

#[test]
fn poly_area_and_type() {
    let mut p = dayzmake_core::navmesh::detour::Poly::default();
    p.set_area(0x3F);
    p.set_type(1);
    assert_eq!((p.area(), p.poly_type()), (0x3F, 1));
    assert_eq!(p.area_and_type, 0x7F);
    p.set_area(5);
    assert_eq!((p.area(), p.poly_type()), (5, 1));
    p.set_type(0);
    assert_eq!(p.area_and_type, 5);
}

#[test]
fn offset_table_vector() {
    let mut set = empty_set(2, 2);
    set.tiles = vec![set_tile(3, tile(1, 1)), set_tile(1, tile(1, 0))];
    let bytes = to_bytes(&set);
    let size0 = 16 + 304u32;
    let table: Vec<u32> = (0..4)
        .map(|i| u32::from_le_bytes(bytes[48 + 4 * i..][..4].try_into().expect("4 bytes")))
        .collect();
    assert_eq!(table, [0xFFFF_FFFF, size0, 0xFFFF_FFFF, 0]);
    assert_eq!(read(&bytes).expect("reads"), set);
}

#[test]
fn version_7_is_read_and_written_as_9() {
    let set = two_tile_set();
    let mut bytes = to_bytes(&set);
    put_i32(&mut bytes, 4, 7);
    let back = read(&bytes).expect("version 7 reads");
    assert_eq!(back.version, 9, "normalised on read");
    let out = to_bytes(&back);
    assert_eq!(&out[4..8], &9i32.to_le_bytes());
    put_i32(&mut bytes, 4, 9);
    assert_eq!(out, bytes);
}

#[test]
fn rejects() {
    let good = to_bytes(&two_tile_set());

    let mut b = good.clone();
    b[0] = b'X';
    assert!(err_of(read(&b)).contains("Invalid header format (bad magic)"));

    let mut b = good.clone();
    put_i32(&mut b, 4, 8);
    assert!(err_of(read(&b)).contains("Invalid version 8"));

    // numTiles > maxTiles
    let mut b = good.clone();
    put_i32(&mut b, 8, 3);
    assert!(err_of(read(&b)).contains("numTiles"));

    // negative numTiles / maxTiles
    let mut b = good.clone();
    put_i32(&mut b, 8, -1);
    assert!(err_of(read(&b)).contains("numTiles"));
    let mut b = good.clone();
    put_i32(&mut b, 40, -1);
    assert!(err_of(read(&b)).contains("maxTiles"));

    // tileRef 0 and dataSize 0 in the first record (at 48 + 8)
    let mut b = good.clone();
    b[56..64].copy_from_slice(&0u64.to_le_bytes());
    assert!(err_of(read(&b)).contains("tileRef"));
    let mut b = good.clone();
    put_i32(&mut b, 64, 0);
    assert!(err_of(read(&b)).contains("dataSize"));
    let mut b = good.clone();
    put_i32(&mut b, 64, -5);
    assert!(err_of(read(&b)).contains("dataSize"));

    // a record running past the end of the file
    let b = &good[..good.len() - 1];
    assert!(err_of(read(b)).contains("past the end"));
    // a table running past the end
    assert!(err_of(read(&good[..50])).contains("truncated"));
    assert!(err_of(read(&good[..10])).contains("truncated"));
    // huge maxTiles with a tiny file: no allocation, a truncation error
    let mut b = good[..48].to_vec();
    put_i32(&mut b, 40, i32::MAX);
    assert!(err_of(read(&b)).contains("truncated"));
}

fn corrupt_blob(f: impl FnOnce(&mut Vec<u8>)) -> io::Result<NavMeshSet> {
    let mut set = two_tile_set();
    let mut blob = set.tiles[0].data.to_bytes();
    f(&mut blob);
    // Re-wrap by hand: a one-tile file with an arbitrary blob.
    set.tiles.truncate(1);
    let mut bytes = to_bytes(&set);
    let head = 48 + 8 + 16;
    bytes.truncate(head);
    let size = i32::try_from(blob.len()).expect("small");
    put_i32(&mut bytes, 48 + 8 + 8, size);
    bytes.extend_from_slice(&blob);
    read(&bytes)
}

#[test]
fn rejects_bad_blobs() {
    // tile version 7
    let e = err_of(corrupt_blob(|b| put_i32(b, 4, 7)));
    assert!(e.contains("tile version 7"), "{e}");
    // bad blob magic
    let e = err_of(corrupt_blob(|b| b[0] = 0));
    assert!(e.contains("bad tile magic"), "{e}");
    // a negative count
    let e = err_of(corrupt_blob(|b| put_i32(b, 24, -1)));
    assert!(e.contains("negative"), "{e}");
    // extra-size mismatch (blob has 52 bytes of extra; header claims 51 + one trailing byte)
    let e = err_of(corrupt_blob(|b| put_i32(b, 100, 51)));
    assert!(e.contains("trailing") || e.contains("extra"), "{e}");
    // the extra counts disagree with extraDataSize
    let e = err_of(corrupt_blob(|b| {
        let at = b.len() - 52;
        put_u32(b, at, 2); // partCount 2 does not fit
    }));
    assert!(e.contains("extra"), "{e}");
    let e = err_of(corrupt_blob(|b| {
        let at = b.len() - 52 + 4;
        put_u32(b, at, u32::MAX);
    }));
    assert!(e.contains("extra"), "{e}");
    let e = err_of(corrupt_blob(|b| {
        let at = b.len() - 4 - 16;
        put_u32(b, at, 9); // obstacleCount 9
    }));
    assert!(e.contains("extra"), "{e}");
    // extraDataSize 5 < 12
    let e = err_of(corrupt_blob(|b| {
        put_i32(b, 100, 5);
        b.truncate(b.len() - 47);
    }));
    assert!(e.contains("extra"), "{e}");
    // negative extraDataSize
    let e = err_of(corrupt_blob(|b| put_i32(b, 100, -1)));
    assert!(e.contains("negative"), "{e}");
    // trailing blob bytes
    let e = err_of(corrupt_blob(|b| b.push(0)));
    assert!(e.contains("trailing"), "{e}");
    // truncated blob
    let e = err_of(corrupt_blob(|b| b.truncate(b.len() - 1)));
    assert!(e.contains("truncated") || e.contains("extra"), "{e}");
    let e = err_of(corrupt_blob(|b| b.truncate(50)));
    assert!(e.contains("truncated"), "{e}");
}

#[test]
fn write_rejects_slot_beyond_max_tiles() {
    let mut set = empty_set(2, 1);
    set.tiles = vec![set_tile(2, tile(0, 0))];
    let mut out = Vec::new();
    let e = set.write(&mut out).expect_err("slot 2 of 2");
    assert!(e.to_string().contains("slot"), "{e}");
    // more tiles than maxTiles
    let mut set = empty_set(1, 1);
    set.tiles = vec![set_tile(0, tile(0, 0)), set_tile(0, tile(0, 0))];
    assert!(set.write(&mut Vec::new()).is_err());
    // two tiles in one slot
    let mut set = empty_set(2, 1);
    set.tiles = vec![set_tile(1, tile(0, 0)), set_tile(1, tile(1, 0))];
    assert!(set.write(&mut Vec::new()).is_err());
    // negative maxTiles
    let mut set = empty_set(2, 1);
    set.params.max_tiles = -1;
    assert!(set.write(&mut Vec::new()).is_err());
}

#[test]
fn huge_counts_are_cheap() {
    let mut blob = tile(0, 0).to_bytes();
    blob.truncate(200);
    put_i32(&mut blob, 24, i32::MAX); // polyCount
    let e = err_of(TileData::parse(&blob));
    assert!(e.contains("truncated"), "{e}");
    // every count at i32::MAX: the sizes overflow or exceed the blob, never allocate
    let mut blob = tile(0, 0).to_bytes();
    for at in (24..=52).step_by(4) {
        put_i32(&mut blob, at, i32::MAX);
    }
    assert!(TileData::parse(&blob).is_err());
    // partCount and partPolyCount near u32::MAX in a small extra section
    let e = err_of(corrupt_blob(|b| {
        let at = b.len() - 52;
        put_u32(b, at, u32::MAX);
        put_u32(b, at + 4, u32::MAX);
    }));
    assert!(e.contains("extra"), "{e}");
}

#[test]
fn write_atomic_replaces_the_target() -> io::Result<()> {
    let dir = tempfile::TempDir::new()?;
    let path = dir.path().join("map.nm");
    fs::write(&path, b"previous save")?;
    let set = two_tile_set();
    set.write_atomic(&path)?;
    assert_eq!(fs::read(&path)?, to_bytes(&set));
    let names: Vec<_> = fs::read_dir(dir.path())?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<io::Result<_>>()?;
    assert_eq!(names.len(), 1, "no temp file left: {names:?}");

    // A failing write leaves the previous file and no temp file.
    let mut bad = empty_set(1, 1);
    bad.tiles = vec![set_tile(5, tile(0, 0))];
    assert!(bad.write_atomic(&path).is_err());
    assert_eq!(fs::read(&path)?, to_bytes(&set));
    assert_eq!(fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

#[test]
fn nan_and_foreign_fields_round_trip_by_bytes() {
    let mut set = two_tile_set();
    set.tiles[0].padding = 0xDEAD_BEEF;
    set.tiles[0].data.verts[0] = [f32::NAN, f32::INFINITY, -0.0];
    set.tiles[0].data.detail_meshes = vec![[7; 12]];
    set.tiles[0].data.detail_verts = vec![[1.0, 2.0, 3.0]];
    set.tiles[0].data.detail_tris = vec![[1, 2, 3, 4]];
    let bytes = to_bytes(&set);
    let back = read(&bytes).expect("reads");
    assert_eq!(to_bytes(&back), bytes);
    assert_eq!(back.tiles[0].padding, 0xDEAD_BEEF);
}

/// Seeds of the `navmesh_nm` fuzz target. Run `cargo test -p dayzmake-core --test navmesh_nm
/// write_fuzz_seeds -- --ignored` to regenerate `fuzz/corpus/navmesh_nm/`.
#[test]
#[ignore = "regenerates fuzz/corpus/navmesh_nm"]
fn write_fuzz_seeds() -> io::Result<()> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/navmesh_nm");
    fs::create_dir_all(&dir)?;
    let good = to_bytes(&two_tile_set());
    let mut v7 = good.clone();
    put_i32(&mut v7, 4, 7);
    let mut sparse = empty_set(2, 2);
    sparse.tiles = vec![set_tile(3, tile(1, 1))];
    // The first byte selects the reader: 0 = a whole file, 1 = a single tile blob.
    let mut no_extra = tile(0, 0);
    no_extra.extra = None;
    let mut empty_extra = tile(0, 0);
    empty_extra.extra = Some(TileExtra::default());
    let seeds: [(&str, Vec<u8>); 8] = [
        ("two_tiles", good.clone()),
        ("version_7", v7),
        ("truncated", good[..good.len() - 40].to_vec()),
        ("sparse", to_bytes(&sparse)),
        ("header_only", good[..48].to_vec()),
        ("tile_blob", tile(2, 3).to_bytes()),
        ("tile_no_extra", no_extra.to_bytes()),
        ("tile_empty_extra", empty_extra.to_bytes()),
    ];
    for (name, data) in seeds {
        let mut seed = vec![u8::from(name.starts_with("tile_"))];
        seed.extend_from_slice(&data);
        fs::write(dir.join(name), seed)?;
    }
    Ok(())
}

/// Found by `navmesh_nm` fuzzing: a tile whose bounds span the whole `i32` range made the cell count
/// of `tiles_in_range` overflow `i64` during the island flood.
#[test]
fn huge_query_box_does_not_overflow() {
    use dayzmake_core::navmesh::detour::NavMesh;
    use dayzmake_core::navmesh::islands::filter_islands;
    let data = include_bytes!("../../../fuzz/corpus/navmesh_nm/regress_tiles_in_range_overflow");
    let set = NavMeshSet::read(&mut &data[1..]).expect("the input is a set file");
    if let Ok(mut mesh) = NavMesh::from_set(&set) {
        for seed in [[0.0; 3], [f32::NAN; 3], [1.0e30; 3]] {
            let first = filter_islands(&mut mesh, seed);
            assert_eq!(filter_islands(&mut mesh, seed), first);
        }
    }
}
