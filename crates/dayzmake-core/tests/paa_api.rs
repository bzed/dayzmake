//! The PAA conversion API: formats, accessors and the bytes the CLI writes.
#![expect(clippy::expect_used, reason = "tests fail by panicking")]
use std::io::Cursor;

use dayzmake_core::image::{DynamicImage, Rgba, RgbaImage};
use dayzmake_core::paa::{Paa, PaaFormat};

/// The gradient `golden_cli.rs` writes with alpha.
fn gradient() -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_fn(64, 64, |x, y| {
        Rgba([
            u8::try_from(x * 4).unwrap_or(0),
            u8::try_from(y * 4).unwrap_or(0),
            128,
            u8::try_from(255 - x * 2).unwrap_or(0),
        ])
    }))
}

fn to_bytes(paa: &Paa) -> Vec<u8> {
    let mut bytes = Vec::new();
    paa.write(&mut bytes).expect("writing to a Vec works");
    bytes
}

#[test]
fn format_parses_case_insensitively() {
    assert_eq!(
        "DxT5".parse::<PaaFormat>().expect("parses"),
        PaaFormat::Dxt5
    );
    assert_eq!(
        "rgba".parse::<PaaFormat>().expect("parses"),
        PaaFormat::Rgba
    );
}

#[test]
fn unknown_format_has_the_cli_error_text() {
    let err = "bogus".parse::<PaaFormat>().expect_err("unknown format");
    assert!(
        err.to_string()
            .contains("Unsupported PAA compression type for creation: bogus")
    );
}

#[test]
fn dxt1_and_dxt5_roundtrip_dimensions() {
    for (format, name) in [(PaaFormat::Dxt1, "DXT1"), (PaaFormat::Dxt5, "DXT5")] {
        let paa = Paa::from_dynamic_image(&gradient(), format).expect("converts");
        let read = Paa::read(&mut Cursor::new(to_bytes(&paa))).expect("reads back");
        assert_eq!(read.width(), 64);
        assert_eq!(read.height(), 64);
        assert_eq!(read.compression(), name);
        assert!(read.mipmap_count() >= 1);
        assert!(read.tagg("GGATSFFO").is_some());
        assert!(read.tagg("NOSUCH").is_none());
    }
}

#[test]
fn conversion_matches_the_cli_bytes() {
    let snapshot = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/snapshots/golden_cli__img2paa_dxt5.snap.paa"
    ))
    .expect("snapshot binary");
    let paa = Paa::from_dynamic_image(&gradient(), PaaFormat::Dxt5).expect("converts");
    assert_eq!(to_bytes(&paa), snapshot);
}
