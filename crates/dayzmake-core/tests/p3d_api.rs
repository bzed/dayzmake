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
