//! Building an MLOD model from scratch and reading it back.
use std::io::Cursor;

use dayzmake_core::p3d::{Face, LOD, P3D, Point, Tagg, Vertex};

#[test]
fn build_write_and_read_back_a_model() {
    let mut lod = LOD::new(1.0);
    for coords in [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0)] {
        let mut point = Point::new();
        point.coords = coords;
        lod.points.push(point);
    }
    let mut face = Face::new();
    for i in 0..3 {
        let mut vertex = Vertex::new();
        vertex.point_index = i;
        face.vertices.push(vertex);
    }
    face.texture = String::from("a\\b_co.paa");
    lod.faces = vec![face];
    lod.face_normals.push((0.0, 0.0, 1.0));
    lod.taggs.push(Tagg::new(1, "body", vec![0; 4]));

    let mut model = P3D::new(0x101);
    model.lods.push(lod);

    let mut bytes = Vec::new();
    model.write(&mut bytes).unwrap();
    let read = P3D::read(&mut Cursor::new(bytes)).unwrap();

    assert_eq!(read.version, 0x101);
    assert_eq!(read.lods.len(), 1);
    let lod = read.lods.first().unwrap();
    assert_eq!(lod.points.len(), 3);
    assert_eq!(lod.faces.len(), 1);
    assert_eq!(lod.selections(), ["body"]);
    assert_eq!(read.textures().get("a\\b_co.paa"), Some(&1));
}

fn triangle_lod() -> LOD {
    let mut lod = LOD::new(1.0);
    for _ in 0..3 {
        lod.points.push(Point::new());
    }
    let mut face = Face::new();
    for i in 0..3 {
        let mut vertex = Vertex::new();
        vertex.point_index = i;
        face.vertices.push(vertex);
    }
    lod.faces = vec![face];
    lod.face_normals.push((0.0, 0.0, 1.0));
    lod
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn write_error(lod: LOD) -> String {
    let mut model = P3D::new(0x101);
    model.lods.push(lod);
    model.write(&mut Vec::new()).unwrap_err().to_string()
}

#[test]
fn write_rejects_a_selection_of_the_wrong_length() {
    let mut lod = triangle_lod();
    lod.taggs.push(Tagg::new(1, "bo\u{1b}dy", vec![0; 3]));
    let msg = write_error(lod);
    assert!(msg.contains("Resolution 1"), "{msg}");
    assert!(msg.contains("tagg 0"), "{msg}");
    assert!(
        msg.contains("bo\\u{1b}dy") && !msg.contains('\u{1b}'),
        "{msg}"
    );
    assert!(msg.contains("expected 4"), "{msg}");
}

#[test]
fn write_accepts_marker_taggs_of_any_length() {
    let mut lod = triangle_lod();
    lod.taggs.push(Tagg::new(1, "#Mass#", vec![0; 12]));
    let mut model = P3D::new(0x101);
    model.lods.push(lod);
    model.write(&mut Vec::new()).unwrap();
}

#[test]
fn write_rejects_a_vertex_index_out_of_range() {
    let mut lod = triangle_lod();
    lod.faces[0].vertices[1].point_index = 3;
    let msg = write_error(lod);
    assert!(msg.contains("face 0"), "{msg}");
    assert!(msg.contains("Resolution 1"), "{msg}");
    assert!(msg.contains("point index 3"), "{msg}");

    let mut lod = triangle_lod();
    lod.faces[0].vertices[2].normal_index = 1;
    let msg = write_error(lod);
    assert!(msg.contains("face 0"), "{msg}");
    assert!(msg.contains("normal index 1"), "{msg}");
}

#[test]
fn write_accepts_an_inactive_selection_of_any_length() {
    let mut lod = triangle_lod();
    lod.taggs.push(Tagg::new(0, "old", Vec::new()));
    let mut model = P3D::new(0x101);
    model.lods.push(lod);
    model.write(&mut Vec::new()).unwrap();
}

#[test]
fn a_rejected_model_writes_nothing() {
    let mut bad_selection = LOD::new(2.0);
    bad_selection.taggs.push(Tagg::new(1, "body", vec![0; 3]));
    let mut five = triangle_lod();
    five.faces[0].vertices.push(Vertex::new());
    five.faces[0].vertices.push(Vertex::new());
    for bad in [bad_selection, five] {
        let mut model = P3D::new(0x101);
        model.lods.push(triangle_lod());
        model.lods.push(bad);
        let mut out = Vec::new();
        let msg = model.write(&mut out).unwrap_err().to_string();
        assert!(out.is_empty(), "{msg}");
        assert!(msg.contains("LOD"), "{msg}");
    }
}
