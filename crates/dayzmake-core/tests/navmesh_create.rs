//! Tests of the Detour tile builder, `create_tile_data` (spec 7.3, 9.3, 10.2, 13.1).

#![cfg(feature = "navmesh")]

use dayzmake_core::navmesh::detour::{
    BvNode, CreateParams, Link, OffMeshInput, TileData, TileExtra, create_tile_data,
};

#[path = "support/navmesh_tiles.rs"]
mod navmesh_tiles;
use navmesh_tiles::tile;

const NIL: u16 = 0xffff;

/// The spec 13.1 triangle `(0,0,0) (50,0,0) (50,0,50)` in cells of 0.25.
fn triangle() -> (Vec<[u16; 3]>, Vec<u16>) {
    let verts = vec![[0, 0, 0], [200, 0, 0], [200, 0, 200]];
    let polys = vec![0, 1, 2, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL];
    (verts, polys)
}

fn params<'a>(
    verts: &'a [[u16; 3]],
    polys: &'a [u16],
    flags: &'a [u16],
    areas: &'a [u8],
    nvp: usize,
    off_mesh: &'a [OffMeshInput],
    extra: &'a TileExtra,
) -> CreateParams<'a> {
    let mut p = CreateParams::new(verts, polys, flags, areas, nvp, off_mesh, extra);
    p.tile_x = 0;
    p.tile_y = 0;
    p.bmin = [0.0, 0.0, 0.0];
    p.bmax = [50.0, 0.0, 50.0];
    p.walkable_height = 1.5;
    p.walkable_radius = 0.3;
    p.walkable_climb = 0.45;
    p.cs = 0.25;
    p.ch = 0.1;
    p
}

fn conn(start: [f32; 3], end: [f32; 3]) -> OffMeshInput {
    let mut c = OffMeshInput::default();
    c.start = start;
    c.end = end;
    c.radius = 0.5;
    c.flags = 0x180;
    c.area = 50;
    c
}

#[test]
fn single_triangle_tile() {
    let (verts, polys) = triangle();
    let extra = tile(0, 0).extra.expect("vector has an extra section");
    let p = params(&verts, &polys, &[1], &[1], 6, &[], &extra);
    let data = create_tile_data(&p).expect("create");

    let h = &data.header;
    assert_eq!(h.poly_count, 1);
    assert_eq!(h.vert_count, 3);
    assert_eq!(h.bv_node_count, 2);
    assert_eq!(h.off_mesh_base, 1);
    assert_eq!(h.off_mesh_con_count, 0);
    assert_eq!(h.detail_mesh_count, 0);
    assert_eq!(h.max_link_count, 3);
    assert!((h.bv_quant_factor - 4.0).abs() < f32::EPSILON);
    assert_eq!(h.extra_data_size, 52);
    assert_eq!(
        data.verts,
        vec![[0.0, 0.0, 0.0], [50.0, 0.0, 0.0], [50.0, 0.0, 50.0]]
    );
    assert_eq!(data.links.len(), 3);
    assert!(data.links.iter().all(|l| *l == Link::default()));

    let bytes = data.to_bytes();
    let sections = 104 + 3 * 12 + 32 + 3 * 16 + 2 * 16 + 52;
    assert_eq!(bytes.len(), sections);
    assert_eq!(TileData::parse(&bytes).expect("parse"), data);

    // The vector's BV nodes are placeholders; every other byte is the spec 13.1 tile.
    let mut expected = tile(0, 0);
    let mut leaf = BvNode::default();
    leaf.bmin = [0, 0, 0];
    leaf.bmax = [200, 0, 200];
    leaf.i = 0;
    expected.bv_tree = vec![leaf, BvNode::default()];
    assert_eq!(data, expected);
    assert_eq!(bytes, expected.to_bytes());
}

#[test]
fn empty_extra_is_a_twelve_byte_section() {
    let (verts, polys) = triangle();
    let extra = TileExtra::default();
    let p = params(&verts, &polys, &[1], &[1], 6, &[], &extra);
    let data = create_tile_data(&p).expect("create");
    assert_eq!(data.extra, Some(TileExtra::default()));
    assert_eq!(data.header.extra_data_size, 12);
}

#[test]
fn vertex_heights_use_ch() {
    let verts = vec![[0, 10, 0], [200, 25, 0], [200, 7, 200]];
    let polys = vec![0, 1, 2, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL];
    let extra = TileExtra::default();
    let mut p = params(&verts, &polys, &[1], &[1], 6, &[], &extra);
    p.bmin = [100.0, -2.0, 200.0];
    let data = create_tile_data(&p).expect("create");
    assert_eq!(data.verts[0], [100.0, -2.0 + 10.0 * 0.1, 200.0]);
    assert_eq!(data.verts[1], [150.0, -2.0 + 25.0 * 0.1, 200.0]);
    // BV y is remapped from ch to cs: floor(10 * 0.1 / 0.25) and ceil(25 * 0.1 / 0.25).
    assert_eq!(data.bv_tree[0].bmin[1], 2);
    assert_eq!(data.bv_tree[0].bmax[1], 10);
}

#[test]
fn off_mesh_start_outside_is_dropped_but_counted() {
    let (verts, polys) = triangle();
    let extra = TileExtra::default();
    let mut inside = conn([10.0, 0.0, 10.0], [60.0, 0.0, 10.0]);
    inside.bidirectional = true;
    inside.center_height = 1.25;
    inside.user_id = 7;
    let outside = conn([60.0, 0.0, 10.0], [20.0, 0.0, 20.0]);
    let off = [inside, outside];
    let p = params(&verts, &polys, &[1], &[1], 6, &off, &extra);
    let data = create_tile_data(&p).expect("create");

    assert_eq!(data.off_mesh.len(), 1);
    assert_eq!(data.header.off_mesh_con_count, 1);
    assert_eq!(data.header.poly_count, 2);
    assert_eq!(data.header.vert_count, 5);
    assert_eq!(data.header.off_mesh_base, 1);
    // edges 3 + portals 0 * 2 + (start of the first, end of the second) * 2
    assert_eq!(data.header.max_link_count, 3 + 2 * 2);
    assert_eq!(data.links.len(), 7);

    let c = &data.off_mesh[0];
    assert_eq!(c.pos, [10.0, 0.0, 10.0, 60.0, 0.0, 10.0]);
    assert!((c.center_height - 1.25).abs() < f32::EPSILON);
    assert!(c.reserved.abs() < f32::EPSILON);
    assert!((c.rad - 0.5).abs() < f32::EPSILON);
    assert_eq!(c.poly, 1);
    assert_eq!(c.flags, 1);
    assert_eq!(c.side, 0);
    assert_eq!(c.user_id, 7);

    let poly = &data.polys[1];
    assert_eq!(poly.vert_count, 2);
    assert_eq!(poly.verts[..2], [3, 4]);
    assert_eq!(poly.flags, 0x180);
    assert_eq!(poly.area(), 50);
    assert_eq!(poly.poly_type(), 1);
    assert_eq!(data.verts[3], [10.0, 0.0, 10.0]);
    assert_eq!(data.verts[4], [60.0, 0.0, 10.0]);
}

#[test]
fn unidirectional_connection_has_flags_zero() {
    let (verts, polys) = triangle();
    let extra = TileExtra::default();
    let off = [conn([10.0, 0.0, 10.0], [20.0, 0.0, 20.0])];
    let p = params(&verts, &polys, &[1], &[1], 6, &off, &extra);
    let data = create_tile_data(&p).expect("create");
    assert_eq!(data.off_mesh[0].flags, 0);
    assert_eq!(data.off_mesh[0].side, 0xff);
    assert_eq!(data.header.max_link_count, 3 + 2 * 2);
}

#[test]
fn off_mesh_y_outside_climb_band_is_outside() {
    let (verts, polys) = triangle();
    let extra = TileExtra::default();
    // The vertex heights are all 0 and the climb is 0.45: the band is [-0.45, 0.45].
    let on_edge = conn([10.0, 0.45, 10.0], [20.0, 0.0, 20.0]);
    let above = conn([10.0, 0.46, 10.0], [20.0, 0.0, 20.0]);
    let below = conn([10.0, -0.46, 10.0], [20.0, 0.0, 20.0]);
    let off = [on_edge, above, below];
    let p = params(&verts, &polys, &[1], &[1], 6, &off, &extra);
    let data = create_tile_data(&p).expect("create");
    assert_eq!(data.off_mesh.len(), 1);
    assert!((data.off_mesh[0].pos[1] - 0.45).abs() < f32::EPSILON);
    // Three inside ends, and the dropped starts are not counted: 1 start + 3 ends.
    assert_eq!(data.header.max_link_count, 3 + 4 * 2);
}

#[test]
fn portal_codes_map_to_detour_dirs() {
    // A square (poly 0) with the four portal codes, and a triangle (poly 1) with a border, an
    // internal neighbour and a polygon with 3 vertices in a stride of 4.
    let verts = vec![[0, 0, 0], [0, 0, 10], [10, 0, 10], [10, 0, 0]];
    let polys = vec![
        0, 1, 2, 3, 0x8000, 0x8001, 0x8002, 0x8003, // poly 0
        0, 2, 3, NIL, 0, 0x800f, 0x800f, NIL, // poly 1: neighbour 0, two borders
    ];
    let extra = TileExtra::default();
    let p = params(&verts, &polys, &[1, 1], &[1, 1], 4, &[], &extra);
    let data = create_tile_data(&p).expect("create");
    assert_eq!(data.polys[0].vert_count, 4);
    assert_eq!(data.polys[0].neis[..4], [0x8004, 0x8002, 0x8000, 0x8006]);
    assert_eq!(data.polys[1].vert_count, 3);
    assert_eq!(data.polys[1].neis[..3], [1, 0, 0]);
    assert_eq!(data.polys[1].verts[..3], [0, 2, 3]);
    // 7 edges and 4 portals counted twice.
    assert_eq!(data.header.max_link_count, 7 + 4 * 2);
}

#[test]
fn empty_input_is_error() {
    let (verts, polys) = triangle();
    let extra = TileExtra::default();
    let p = params(&[], &polys, &[1], &[1], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
    let p = params(&verts, &[], &[], &[], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
}

#[test]
fn nvp_7_is_error() {
    let verts = vec![[0, 0, 0], [200, 0, 0], [200, 0, 200]];
    let polys = vec![NIL; 14];
    let mut polys = polys;
    polys[0] = 0;
    polys[1] = 1;
    polys[2] = 2;
    let extra = TileExtra::default();
    let p = params(&verts, &polys, &[1], &[1], 7, &[], &extra);
    assert!(create_tile_data(&p).is_err());
}

#[test]
fn too_many_verts_is_error() {
    let verts = vec![[0u16, 0, 0]; 0xffff];
    let (_, polys) = triangle();
    let extra = TileExtra::default();
    let p = params(&verts, &polys, &[1], &[1], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
    // One fewer is accepted.
    let verts = vec![[0u16, 0, 0]; 0xfffe];
    let p = params(&verts, &polys, &[1], &[1], 6, &[], &extra);
    assert!(create_tile_data(&p).is_ok());
}

#[test]
fn inconsistent_slices_are_errors() {
    let (verts, polys) = triangle();
    let extra = TileExtra::default();
    // Flags and areas shorter than the polygon count.
    let p = params(&verts, &polys, &[], &[1], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
    let p = params(&verts, &polys, &[1], &[], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
    // A polygon array that is not a multiple of nvp * 2.
    let p = params(&verts, &polys[..11], &[1], &[1], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
    // A vertex index outside the vertex list.
    let bad = vec![0, 1, 3, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL];
    let p = params(&verts, &bad, &[1], &[1], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
    // A polygon without vertices.
    let none = vec![NIL; 12];
    let p = params(&verts, &none, &[1], &[1], 6, &[], &extra);
    assert!(create_tile_data(&p).is_err());
}

#[test]
fn bv_tree_covers_all_polys() {
    let n: u16 = 7;
    let mut verts = Vec::new();
    let mut polys = Vec::new();
    for i in 0..n {
        let x = i * 40;
        let base = u16::try_from(verts.len()).expect("small");
        verts.push([x, 0, 0]);
        verts.push([x + 20, i, 0]);
        verts.push([x, 0, 20 + i * 3]);
        polys.extend_from_slice(&[base, base + 1, base + 2, NIL, NIL, NIL]);
        polys.extend_from_slice(&[NIL; 6]);
    }
    let count = usize::from(n);
    let flags = vec![1u16; count];
    let areas = vec![1u8; count];
    let extra = TileExtra::default();
    let p = params(&verts, &polys, &flags, &areas, 6, &[], &extra);
    let data = create_tile_data(&p).expect("create");

    assert_eq!(data.bv_tree.len(), 2 * count);
    let used = 2 * count - 1;
    let mut seen = vec![0u32; count];
    for (k, node) in data.bv_tree[..used].iter().enumerate() {
        if node.i >= 0 {
            seen[usize::try_from(node.i).expect("leaf index")] += 1;
        } else {
            let escape = usize::try_from(-node.i).expect("escape");
            assert!(k + escape <= used, "escape leaves the tree");
            assert!(escape >= 2);
        }
    }
    assert!(
        seen.iter().all(|&s| s == 1),
        "every polygon in one leaf: {seen:?}"
    );
    assert_eq!(data.bv_tree[used], BvNode::default());
    // The root covers every leaf.
    let root = &data.bv_tree[0];
    for node in data.bv_tree[..used].iter().filter(|n| n.i >= 0) {
        for a in 0..3 {
            assert!(root.bmin[a] <= node.bmin[a] && root.bmax[a] >= node.bmax[a]);
        }
    }
    // Leaves hold the quantised polygon bounds.
    let leaf = data.bv_tree[..used]
        .iter()
        .find(|n| n.i == 3)
        .expect("leaf 3");
    assert_eq!(leaf.bmin, [120, 0, 0]);
    assert_eq!(leaf.bmax, [140, 2, 29]);
}

#[test]
fn bv_tree_is_deterministic_with_ties() {
    // Identical bounds on every polygon: the stable sort keeps the polygon order.
    let verts = vec![[0, 0, 0], [20, 0, 0], [0, 0, 20]];
    let mut polys = Vec::new();
    for _ in 0..5 {
        polys.extend_from_slice(&[0, 1, 2, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL]);
    }
    let extra = TileExtra::default();
    let p = params(&verts, &polys, &[1; 5], &[1; 5], 6, &[], &extra);
    let a = create_tile_data(&p).expect("create");
    let b = create_tile_data(&p).expect("create");
    assert_eq!(a, b);
    let leaves: Vec<i32> = a.bv_tree[..9]
        .iter()
        .filter(|n| n.i >= 0)
        .map(|n| n.i)
        .collect();
    assert_eq!(leaves, vec![0, 1, 2, 3, 4]);
}
