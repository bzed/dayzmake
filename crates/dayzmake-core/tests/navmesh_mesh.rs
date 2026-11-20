//! Tests of the tiled Detour navmesh: adding, removing and linking tiles (spec 8, 10.2, 13.4).

#![cfg(feature = "navmesh")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::many_single_char_names,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use dayzmake_core::navmesh::detour::{
    CreateParams, Link, NavMesh, NavMeshParams, NavMeshSet, Obstacle, OffMeshInput, SetTile,
    TileData, TileExtra, create_tile_data, decode_poly_ref, encode_poly_ref,
};

#[path = "support/navmesh_tiles.rs"]
mod navmesh_tiles;
use navmesh_tiles::{empty_set, set_tile, tile, two_tile_set};

const NIL: u16 = 0xffff;
const NULL_LINK: u32 = 0xffff_ffff;

fn mesh_params(max_tiles: i32) -> NavMeshParams {
    let mut p = NavMeshParams::default();
    p.tile_width = 50.0;
    p.tile_height = 50.0;
    p.max_tiles = max_tiles;
    p.max_polys = 1 << 20;
    p
}

/// A rectangle of `w` by `d` cells (0.25 m) in tile `(x, z)` at height `h` cells (0.1 m). The
/// neighbour codes are the Recast ones: `NIL` for a solid edge, `0x8000 | 0..3` for the portals
/// x-, z+, x+ and z-. Edge `i` runs from vertex `i` to `i + 1`: x-, z+, x+, z-.
fn quad(
    x: i32,
    z: i32,
    (w, d, h): (u16, u16, u16),
    neis: [u16; 4],
    off_mesh: &[OffMeshInput],
    extra: &TileExtra,
) -> TileData {
    let verts = [[0, h, 0], [0, h, d], [w, h, d], [w, h, 0]];
    let polys = [
        0, 1, 2, 3, NIL, NIL, neis[0], neis[1], neis[2], neis[3], NIL, NIL,
    ];
    let mut p = CreateParams::new(&verts, &polys, &[1], &[1], 6, off_mesh, extra);
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

fn square(x: i32, z: i32, neis: [u16; 4]) -> TileData {
    quad(x, z, (200, 200, 0), neis, &[], &TileExtra::default())
}

/// The links of polygon `poly`, in list order.
fn chain(t: &TileData, poly: usize) -> Vec<Link> {
    let mut out = Vec::new();
    let mut at = t.polys[poly].first_link;
    while at != NULL_LINK {
        let l = t.links[usize::try_from(at).unwrap()].clone();
        at = l.next;
        out.push(l);
    }
    out
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

#[test]
fn adjacent_tiles_get_portal_links() {
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    let a = mesh
        .set_tile(square(0, 0, [NIL, NIL, 0x8002, NIL]))
        .unwrap();
    let b = mesh
        .set_tile(square(1, 0, [0x8000, NIL, NIL, NIL]))
        .unwrap();
    assert_eq!(a, encode_poly_ref(1, 0, 0));
    assert_eq!(b, encode_poly_ref(1, 1, 0));

    let la = chain(mesh.tile_at(0, 0).unwrap(), 0);
    assert_eq!(la.len(), 1);
    assert_eq!(decode_poly_ref(la[0].reference), (1, 1, 0));
    assert_eq!((la[0].side, la[0].edge), (0, 2));
    assert_eq!((la[0].bmin, la[0].bmax), (0, 255));

    let lb = chain(mesh.tile_at(1, 0).unwrap(), 0);
    assert_eq!(lb.len(), 1);
    assert_eq!(decode_poly_ref(lb[0].reference), (1, 0, 0));
    assert_eq!((lb[0].side, lb[0].edge), (4, 0));
    assert_eq!((lb[0].bmin, lb[0].bmax), (0, 255));
}

#[test]
fn portal_limits_truncate() {
    // B is only 25 m deep, so the overlap with A's 50 m edge is the half of it: t = 0.5 and
    // 0.5 * 255.0 = 127.5, which truncates to 127 (stock Detour would round to 128).
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    mesh.set_tile(square(0, 0, [NIL, NIL, 0x8002, NIL]))
        .unwrap();
    let b = quad(
        1,
        0,
        (200, 100, 0),
        [0x8000, NIL, NIL, NIL],
        &[],
        &TileExtra::default(),
    );
    mesh.set_tile(b).unwrap();
    // A's edge runs from z 50 to z 0 and the overlap is z 0..25: t from 1.0 down to 0.5.
    let la = chain(mesh.tile_at(0, 0).unwrap(), 0);
    assert_eq!(la.len(), 1);
    assert_eq!((la[0].bmin, la[0].bmax), (127, 255));
    // B's edge runs from z 0 to z 25 and the whole of it overlaps.
    let lb = chain(mesh.tile_at(1, 0).unwrap(), 0);
    assert_eq!((lb[0].bmin, lb[0].bmax), (0, 255));
}

#[test]
fn replace_tile_bumps_salt() {
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    mesh.set_tile(square(0, 0, [NIL, NIL, 0x8002, NIL]))
        .unwrap();
    mesh.set_tile(square(1, 0, [0x8000, NIL, NIL, NIL]))
        .unwrap();
    let again = mesh
        .set_tile(square(0, 0, [NIL, NIL, 0x8002, NIL]))
        .unwrap();
    assert_eq!(again, encode_poly_ref(2, 0, 0));
    assert_eq!(mesh.tiles().count(), 2);

    let set = mesh.to_set(2, 1);
    assert_eq!(set.tiles.len(), 2);
    assert_eq!(set.tiles[0].tile_ref, encode_poly_ref(2, 0, 0));
    assert_eq!(set.tiles[1].tile_ref, encode_poly_ref(1, 1, 0));
    // B links to the replacement: the link carries the new salt.
    let lb = chain(mesh.tile_at(1, 0).unwrap(), 0);
    assert_eq!(lb.len(), 1);
    assert_eq!(decode_poly_ref(lb[0].reference), (2, 0, 0));
    let la = chain(mesh.tile_at(0, 0).unwrap(), 0);
    assert_eq!(decode_poly_ref(la[0].reference), (1, 1, 0));
}

#[test]
fn removing_a_tile_unlinks_its_neighbours_and_frees_the_slot() {
    let mut mesh = NavMesh::new(mesh_params(3)).unwrap();
    mesh.set_tile(square(0, 0, [NIL, NIL, 0x8002, NIL]))
        .unwrap();
    mesh.set_tile(square(1, 0, [0x8000, NIL, NIL, NIL]))
        .unwrap();
    assert!(mesh.remove_tile_at(0, 0));
    assert!(!mesh.remove_tile_at(0, 0));
    assert!(mesh.tile_at(0, 0).is_none());
    assert_eq!(chain(mesh.tile_at(1, 0).unwrap(), 0).len(), 0);
    // The most recently freed slot is allocated first, with the bumped salt.
    let c = mesh.set_tile(square(5, 5, [NIL; 4])).unwrap();
    assert_eq!(c, encode_poly_ref(2, 0, 0));
    // Then the untouched slots, in order.
    let d = mesh.set_tile(square(6, 5, [NIL; 4])).unwrap();
    assert_eq!(d, encode_poly_ref(1, 2, 0));
    assert!(mesh.set_tile(square(7, 5, [NIL; 4])).is_err());
}

#[test]
fn off_mesh_link_connects_two_tiles() {
    for bidirectional in [true, false] {
        let cons = [conn([45.0, 0.0, 10.0], [55.0, 0.0, 10.0], bidirectional)];
        let extra = TileExtra::default();
        let a = quad(0, 0, (200, 200, 0), [NIL; 4], &cons, &extra);
        let b = quad(1, 0, (200, 200, 0), [NIL; 4], &cons, &extra);
        assert_eq!(a.off_mesh.len(), 1);
        assert_eq!(b.off_mesh.len(), 0);
        let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
        mesh.set_tile(a).unwrap();
        mesh.set_tile(b).unwrap();

        let ta = mesh.tile_at(0, 0).unwrap();
        // The start was linked in tile A: reserved is set, and the polygons link both ways.
        assert!((ta.off_mesh[0].reserved - 1.0).abs() < f32::EPSILON);
        let from_off_mesh = chain(ta, 1);
        // The landing link in B (added last) heads the list, the start link follows.
        assert_eq!(from_off_mesh.len(), 2);
        assert_eq!(decode_poly_ref(from_off_mesh[0].reference), (1, 1, 0));
        assert_eq!(from_off_mesh[0].edge, 1);
        assert_eq!(from_off_mesh[0].side, 0);
        assert_eq!(decode_poly_ref(from_off_mesh[1].reference), (1, 0, 0));
        assert_eq!(from_off_mesh[1].edge, 0);
        assert_eq!(from_off_mesh[1].side, 0xff);
        // The off-mesh end point was moved onto the mesh of B.
        assert!((ta.verts[usize::from(ta.polys[1].verts[1])][0] - 55.0).abs() < 1e-4);

        let back_a = chain(ta, 0);
        assert_eq!(back_a.len(), 1);
        assert_eq!(decode_poly_ref(back_a[0].reference), (1, 0, 1));
        assert_eq!(back_a[0].edge, 0xff);

        let back_b = chain(mesh.tile_at(1, 0).unwrap(), 0);
        if bidirectional {
            assert_eq!(back_b.len(), 1);
            assert_eq!(decode_poly_ref(back_b[0].reference), (1, 0, 1));
            assert_eq!((back_b[0].edge, back_b[0].side), (0xff, 4));
        } else {
            assert_eq!(back_b.len(), 0);
        }
    }
}

#[test]
fn obstacle_y_is_snapped() {
    let mut extra = TileExtra::default();
    let mut o = Obstacle::default();
    o.pos = [25.0, 5.0, 25.0];
    o.radius = 0.3;
    let mut on_floor = Obstacle::default();
    on_floor.pos = [10.0, 5.0, 10.0];
    extra.obstacles = vec![o, on_floor];

    // A flat polygon at height 0.
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    mesh.set_tile(quad(0, 0, (200, 200, 0), [NIL; 4], &[], &extra))
        .unwrap();
    let obs = &mesh
        .tile_at(0, 0)
        .unwrap()
        .extra
        .as_ref()
        .unwrap()
        .obstacles;
    assert!(obs[0].pos[1].abs() < 1e-6);
    assert!(obs[1].pos[1].abs() < 1e-6);
    assert!((obs[0].pos[0] - 25.0).abs() < f32::EPSILON);
    assert!((obs[0].radius - 0.3).abs() < f32::EPSILON);

    // A polygon at 2 m.
    let mut mesh = NavMesh::new(mesh_params(2)).unwrap();
    mesh.set_tile(quad(0, 0, (200, 200, 20), [NIL; 4], &[], &extra))
        .unwrap();
    let obs = &mesh
        .tile_at(0, 0)
        .unwrap()
        .extra
        .as_ref()
        .unwrap()
        .obstacles;
    assert!((obs[0].pos[1] - 2.0).abs() < 1e-5);
}

/// The shared two-tile vector with real bounding volume nodes.
fn real_two_tile_set() -> NavMeshSet {
    let extra = tile(0, 0).extra.expect("vector has an extra section");
    let verts = [[0, 0, 0], [200, 0, 0], [200, 0, 200]];
    let polys = [0, 1, 2, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL];
    let build = |x: i32| {
        let mut p = CreateParams::new(&verts, &polys, &[1], &[1], 6, &[], &extra);
        p.tile_x = x;
        p.bmax = [50.0, 0.0, 50.0];
        p.walkable_height = 1.5;
        p.walkable_radius = 0.3;
        p.walkable_climb = 0.45;
        create_tile_data(&p).expect("create")
    };
    let mut set = two_tile_set();
    set.tiles = vec![set_tile(0, build(0)), set_tile(1, build(1))];
    set
}

#[test]
fn into_set_equals_to_set() {
    for set in [two_tile_set(), real_two_tile_set()] {
        let mesh = NavMesh::from_set(&set).unwrap();
        let expected = mesh.to_set(set.tiles_x, set.tiles_z);
        assert_eq!(mesh.into_set(set.tiles_x, set.tiles_z), expected);
    }
}

#[test]
fn from_set_roundtrip() {
    for set in [two_tile_set(), real_two_tile_set()] {
        let mesh = NavMesh::from_set(&set).unwrap();
        let out = mesh.to_set(set.tiles_x, set.tiles_z);
        // The links are recomputed: no polygon has any, and the free list chains the unused ones.
        let mut expected = set.clone();
        for t in &mut expected.tiles {
            let n = t.data.links.len();
            for (i, l) in t.data.links.iter_mut().enumerate() {
                l.next = if i + 1 < n {
                    u32::try_from(i + 1).unwrap()
                } else {
                    NULL_LINK
                };
            }
            for p in &mut t.data.polys {
                p.first_link = NULL_LINK;
            }
            for o in t.data.extra.iter_mut().flat_map(|e| e.obstacles.iter_mut()) {
                o.pos[1] = 0.0;
            }
        }
        assert_eq!(out, expected);
        // And the bytes of the file are stable from there on.
        let again = NavMesh::from_set(&out)
            .unwrap()
            .to_set(set.tiles_x, set.tiles_z);
        assert_eq!(again, out);
    }
}

#[test]
fn from_set_keeps_refs_and_slot_order() {
    let mut set = empty_set(2, 2);
    let mut late = set_tile(3, square(1, 1, [NIL; 4]));
    late.padding = 0xdead_beef;
    late.tile_ref = encode_poly_ref(7, 3, 0);
    set.tiles = vec![late, set_tile(1, square(1, 0, [NIL; 4]))];
    let mesh = NavMesh::from_set(&set).unwrap();
    let refs: Vec<u64> = mesh.tiles().map(|(r, _)| r).collect();
    assert_eq!(
        refs,
        vec![encode_poly_ref(1, 1, 0), encode_poly_ref(7, 3, 0)]
    );
    let out = mesh.to_set(2, 2);
    assert_eq!(out.tiles[0].tile_ref, encode_poly_ref(1, 1, 0));
    assert_eq!(out.tiles[1].tile_ref, encode_poly_ref(7, 3, 0));
    // The record padding is preserved; a tile added later gets 0.
    assert_eq!(out.tiles[0].padding, 0);
    assert_eq!(out.tiles[1].padding, 0xdead_beef);
    let mut mesh = mesh;
    mesh.set_tile(square(0, 0, [NIL; 4])).unwrap();
    assert!(
        mesh.to_set(2, 2)
            .tiles
            .iter()
            .filter(|t| t.padding == 0)
            .count()
            >= 2
    );
    assert_eq!(out.params, set.params);
}

#[test]
fn hostile_input_is_rejected() {
    assert!(NavMesh::new(mesh_params(0)).is_err());
    assert!(NavMesh::new(mesh_params(-3)).is_err());
    assert!(NavMesh::new(mesh_params(i32::MAX)).is_err());

    let mut set = empty_set(1, 1);
    let mut bad = set_tile(0, square(0, 0, [NIL; 4]));
    bad.tile_ref = encode_poly_ref(1, 5, 0);
    set.tiles = vec![bad];
    assert!(NavMesh::from_set(&set).is_err(), "slot beyond maxTiles");

    let mut bad = set_tile(0, square(0, 0, [NIL; 4]));
    bad.tile_ref = encode_poly_ref(0, 0, 0) | 1;
    set.tiles = vec![bad];
    assert!(NavMesh::from_set(&set).is_err(), "salt 0");

    let mut broken = square(0, 0, [NIL; 4]);
    broken.header.magic = 0;
    set.tiles = vec![set_tile(0, broken)];
    assert!(NavMesh::from_set(&set).is_err(), "magic");

    let mut broken = square(0, 0, [NIL; 4]);
    broken.polys[0].verts[0] = 99;
    set.tiles = vec![set_tile(0, broken)];
    assert!(NavMesh::from_set(&set).is_err(), "vertex index");

    let mut broken = square(0, 0, [NIL; 4]);
    broken.polys[0].neis[0] = 77;
    set.tiles = vec![set_tile(0, broken)];
    assert!(NavMesh::from_set(&set).is_err(), "neighbour index");

    let mut mesh = NavMesh::new(mesh_params(1)).unwrap();
    mesh.set_tile(square(0, 0, [NIL; 4])).unwrap();
    // A failed replacement leaves the existing tile in place.
    let mut broken = square(0, 0, [NIL; 4]);
    broken.header.version = 3;
    assert!(mesh.set_tile(broken).is_err());
    assert!(mesh.tile_at(0, 0).is_some());
}

/// Names the section of a tile blob that holds `offset`.
fn section_at(t: &TileData, offset: usize) -> &'static str {
    let h = &t.header;
    let n = |v: i32| usize::try_from(v).unwrap_or(0);
    let sizes = [
        ("header", 104),
        ("verts", 12 * n(h.vert_count)),
        ("polys", 32 * n(h.poly_count)),
        ("links", 16 * n(h.max_link_count)),
        ("detail meshes", 12 * n(h.detail_mesh_count)),
        ("detail verts", 12 * n(h.detail_vert_count)),
        ("detail tris", 4 * n(h.detail_tri_count)),
        ("bv tree", 16 * n(h.bv_node_count)),
        ("off-mesh connections", 44 * n(h.off_mesh_con_count)),
    ];
    let mut at = 0;
    for (name, len) in sizes {
        at += len;
        if offset < at {
            return name;
        }
    }
    "extra section"
}

/// Describes how tile `y` differs from tile `x`, field by field (empty: equal).
fn tile_differences(x: &TileData, y: &TileData) -> Vec<String> {
    let mut out = Vec::new();
    if x.header != y.header {
        out.push(format!("header {:?} against {:?}", x.header, y.header));
    }
    for (i, (p, q)) in x.verts.iter().zip(&y.verts).enumerate() {
        if p.map(f32::to_bits) != q.map(f32::to_bits) {
            let owner = x
                .polys
                .iter()
                .enumerate()
                .find(|(_, poly)| {
                    poly.verts
                        .iter()
                        .take(usize::from(poly.vert_count))
                        .any(|&v| usize::from(v) == i)
                })
                .map(|(n, poly)| {
                    let con = x.off_mesh.iter().position(|c| usize::from(c.poly) == n);
                    format!(
                        "poly {n} (type {}, off-mesh connection {con:?}, flags {:#x})",
                        poly.poly_type(),
                        poly.flags
                    )
                });
            out.push(format!(
                "vertex {i} (byte offset {} of the blob) {p:?} against {q:?}, owner {owner:?}",
                104 + 12 * i
            ));
        }
    }
    for (i, (p, q)) in x.polys.iter().zip(&y.polys).enumerate() {
        if p != q {
            out.push(format!("poly {i}: {p:?} against {q:?}"));
        }
    }
    for (i, (p, q)) in x.links.iter().zip(&y.links).enumerate() {
        if p != q {
            out.push(format!("link {i}: {p:?} against {q:?}"));
        }
    }
    for (i, (p, q)) in x.off_mesh.iter().zip(&y.off_mesh).enumerate() {
        if p != q {
            out.push(format!("off-mesh connection {i}: {p:?} against {q:?}"));
        }
    }
    if x.bv_tree != y.bv_tree {
        out.push("bv tree differs".to_owned());
    }
    if x.extra != y.extra {
        out.push("extra section differs".to_owned());
    }
    let (bx, by) = (x.to_bytes(), y.to_bytes());
    if out.is_empty()
        && let Some(i) = bx.iter().zip(&by).position(|(p, q)| p != q)
    {
        out.push(format!(
            "section {} differs at byte offset {i}: {:#04x} against {:#04x}",
            section_at(x, i),
            bx[i],
            by[i]
        ));
    }
    out
}

/// Every difference between two tile sets, with every obstacle y zeroed: one entry per differing
/// tile, naming the tile ref and the differences.
fn differences(a: &NavMeshSet, b: &NavMeshSet) -> Vec<String> {
    if a.tiles.len() != b.tiles.len() {
        return vec![format!("{} tiles against {}", a.tiles.len(), b.tiles.len())];
    }
    let strip = |t: &SetTile| {
        let mut t = t.clone();
        for o in t.data.extra.iter_mut().flat_map(|e| e.obstacles.iter_mut()) {
            o.pos[1] = 0.0;
        }
        t
    };
    let mut out = Vec::new();
    for (x, y) in a.tiles.iter().zip(&b.tiles) {
        let (x, y) = (strip(x), strip(y));
        if x.tile_ref != y.tile_ref {
            out.push(format!(
                "tile ref {:#x} against {:#x}",
                x.tile_ref, y.tile_ref
            ));
            continue;
        }
        let d = tile_differences(&x.data, &y.data);
        if !d.is_empty() {
            out.push(format!("tile {:#x}: {}", x.tile_ref, d.join("; ")));
        }
    }
    out
}

#[test]
#[ignore = "needs a shipped DayZ navmesh (DAYZ_NAVMESH=path)"]
fn shipped_navmesh_relinks_identically() {
    let path = std::env::var("DAYZ_NAVMESH").expect("set DAYZ_NAVMESH to a navmesh.nm");
    let mut file = std::fs::File::open(&path).expect("open the navmesh");
    let original = NavMeshSet::read(&mut file).expect("read the navmesh");

    let mut zeroed = original.clone();
    for t in &mut zeroed.tiles {
        t.data.links.fill(Link::default());
        for p in &mut t.data.polys {
            p.first_link = 0;
        }
    }
    let rebuilt = NavMesh::from_set(&zeroed)
        .expect("add the tiles")
        .to_set(original.tiles_x, original.tiles_z);
    let diffs = differences(&original, &rebuilt);
    // DAYZ_NAVMESH_VERBOSE=1 prints every differing tile, else the first few.
    let shown = if std::env::var_os("DAYZ_NAVMESH_VERBOSE").is_some() {
        diffs.len()
    } else {
        3
    };
    assert!(
        diffs.is_empty(),
        "relinking does not reproduce the shipped navmesh: {} of {} tiles differ\n{}",
        diffs.len(),
        original.tiles.len(),
        diffs[..diffs.len().min(shown)].join("\n")
    );
}
