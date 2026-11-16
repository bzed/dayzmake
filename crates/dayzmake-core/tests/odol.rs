//! Reading a synthetic ODOL v56 model, and hostile variants of it.

#[path = "support/odol_builder.rs"]
mod odol_builder;

use std::io::Cursor;

use dayzmake_core::odol::Odol;
use odol_builder::{Knobs, build, build_parts};

fn read(bytes: &[u8]) -> Result<Odol, std::io::Error> {
    Odol::read(&mut Cursor::new(bytes))
}

#[test]
fn reads_every_field_of_a_synthetic_model() {
    let model = read(&build(Knobs::default())).unwrap();
    assert_eq!(model.version, 56);
    assert_eq!(model.property_class, "ClassName");
    assert_eq!(model.property_damage, "DamageName");
    assert_eq!(model.skeleton_name, "Skel");
    let bone_count: u32 = model.bone_count;
    assert_eq!(bone_count, 2);
    assert!((model.mass - 12.5).abs() < 1e-6);
    assert_eq!(model.bbox_min, [-1.0, -2.0, -3.0]);
    assert_eq!(model.bbox_max, [1.0, 2.0, 3.0]);

    let animations: Vec<_> = model
        .animations
        .iter()
        .map(|a| (a.kind, a.name.as_str(), a.source.as_str()))
        .collect();
    assert_eq!(animations, [(0, "turn", "door"), (9, "vanish", "hide")]);

    assert_eq!(model.lods.len(), 2);
    let a = &model.lods[0];
    assert!((a.resolution - 1.0).abs() < 1e-6);
    assert_eq!(a.name(), "Resolution 1");
    assert_eq!(a.vertex_count, 3);
    assert_eq!(a.face_count, 1);
    assert_eq!(a.textures, ["\\dz\\tex_a_co.paa", "\\dz\\tex_b_co.paa"]);
    assert_eq!(a.materials.len(), 1);
    assert_eq!(a.materials[0].name, "\\dz\\mat.rvmat");
    assert_eq!(a.materials[0].textures, ["\\dz\\stage_co.paa"]);
    assert_eq!(a.proxies, ["\\dz\\proxy\\thing.p3d"]);
    assert_eq!(a.selections, ["body"]);
    assert_eq!(a.properties, [("class".to_string(), "house".to_string())]);

    let b = &model.lods[1];
    assert_eq!(b.name(), "Geometry");
    assert_eq!(b.vertex_count, 4);
    assert_eq!(b.face_count, 2);
    assert_eq!(b.selections, ["big"]);
    assert!(b.textures.is_empty() && b.proxies.is_empty() && b.properties.is_empty());
}

#[test]
fn other_versions_are_refused() {
    let mut bytes = build(Knobs::default());
    bytes[4] = 58;
    let message = read(&bytes).unwrap_err().to_string();
    assert!(
        message.contains("ODOL version 58 is not supported, only 56"),
        "{message}"
    );
}

#[test]
fn a_file_that_is_not_odol_is_refused() {
    let message = read(b"MLOD\x38\0\0\0").unwrap_err().to_string();
    assert!(message.contains("not an ODOL model"), "{message}");
    assert!(read(b"").is_err());
}

#[test]
fn every_truncated_header_is_an_error() {
    let (bytes, header) = build_parts(Knobs::default());
    for len in 0..=header {
        assert!(read(&bytes[..len]).is_err(), "prefix of {len} bytes");
    }
}

#[test]
fn every_truncation_is_an_error_or_ok_never_a_panic() {
    let bytes = build(Knobs::default());
    for len in 0..bytes.len() {
        let _ = read(&bytes[..len]);
    }
}

#[test]
fn a_huge_lod_count_is_an_error() {
    let bytes = build(Knobs {
        lod_count: Some(0xFFFF_FFFF),
        ..Knobs::default()
    });
    let message = read(&bytes).unwrap_err().to_string();
    assert!(message.contains("too many LODs"), "{message}");
}

#[test]
fn a_lod_start_past_the_end_is_an_error() {
    let bytes = build(Knobs {
        lod1_start: Some(0x00FF_FFFF),
        ..Knobs::default()
    });
    let message = read(&bytes).unwrap_err().to_string();
    assert!(
        message.contains("LOD 1") && message.contains("past the end"),
        "{message}"
    );
}

#[test]
fn an_array_count_beyond_the_limits_is_an_error_without_allocating() {
    for count in [0x7FFF_FFFF, 0xFFFF_FFFF, 0x0200_0000] {
        let bytes = build(Knobs {
            huge_array: Some(count),
            ..Knobs::default()
        });
        let message = read(&bytes).unwrap_err().to_string();
        assert!(message.contains("LOD 1"), "{count:#x}: {message}");
    }
}

#[test]
fn an_unknown_material_version_is_named() {
    let mut bytes = build(Knobs::default());
    let name = b"\\dz\\mat.rvmat\0";
    let at = bytes.windows(name.len()).position(|w| w == name).unwrap() + name.len();
    bytes[at] = 11;
    let message = read(&bytes).unwrap_err().to_string();
    assert!(message.contains("version 11"), "{message}");
}

#[test]
fn a_string_without_nul_is_an_error() {
    let bytes = build(Knobs {
        unterminated_property_class: true,
        ..Knobs::default()
    });
    let message = read(&bytes).unwrap_err().to_string();
    assert!(message.contains("string without end"), "{message}");
}

#[test]
fn an_lzo_stream_that_overruns_its_declared_size_is_an_error() {
    let bytes = build(Knobs {
        lzo_overrun: true,
        ..Knobs::default()
    });
    let message = read(&bytes).unwrap_err().to_string();
    assert!(
        message.contains("LOD 1") && message.contains("LZO"),
        "{message}"
    );
}

#[test]
fn an_lzo_stream_that_ends_before_the_declared_size_is_an_error() {
    let bytes = build(Knobs {
        lzo_short: true,
        ..Knobs::default()
    });
    let message = read(&bytes).unwrap_err().to_string();
    assert!(message.contains("LOD 1"), "{message}");
}

#[test]
fn lods_that_share_data_are_refused() {
    // Both LODs start at the same offset: reading that data twice is what lets a small file cost a lot of work.
    let (_, first) = build_parts(Knobs::default());
    let bytes = build(Knobs {
        lod1_start: Some(u32::try_from(first).unwrap()),
        ..Knobs::default()
    });
    let message = read(&bytes).unwrap_err().to_string();
    assert!(
        message.contains("inside the data of the LOD before it"),
        "{message}"
    );
}
