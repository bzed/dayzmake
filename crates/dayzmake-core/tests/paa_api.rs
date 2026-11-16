//! The PAA conversion API: formats, accessors and the bytes the CLI writes.
#[cfg(feature = "image")]
use std::io::Cursor;

#[cfg(feature = "image")]
use dayzmake_core::image::{DynamicImage, Rgba, RgbaImage};
#[cfg(feature = "image")]
use dayzmake_core::paa::Paa;
use dayzmake_core::paa::PaaFormat;

#[cfg(feature = "image")]
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

#[cfg(feature = "image")]
#[expect(clippy::expect_used, reason = "tests fail by panicking")]
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

#[cfg(feature = "image")]
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

#[cfg(feature = "image")]
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

/// A smooth `size` x `size` picture that LZO can shrink.
#[cfg(feature = "image")]
fn smooth(size: u32) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_fn(size, size, |x, y| {
        Rgba([
            u8::try_from(x / 4).unwrap_or(255),
            u8::try_from(y / 4).unwrap_or(255),
            90,
            255,
        ])
    }))
}

/// The file written by `from_dynamic_image`, read back: its mipmaps are held uncompressed.
#[cfg(feature = "image")]
#[expect(clippy::expect_used, reason = "tests fail by panicking")]
fn raw_paa(size: u32, format: PaaFormat) -> (Paa, Vec<u8>) {
    let paa = Paa::from_dynamic_image(&smooth(size), format).expect("converts");
    let written = to_bytes(&paa);
    let raw = Paa::read(&mut Cursor::new(written.clone())).expect("reads back");
    (raw, written)
}

/// The width field (with the LZO bit) of the first mipmap of a PAA file.
#[cfg(feature = "image")]
#[expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test helper parsing output the test itself created"
)]
fn first_mipmap_width_field(paa: &[u8]) -> u16 {
    let at = paa
        .windows(8)
        // a read PAA keeps its offset tagg and the writer adds a fresh one; the last one is right
        .rposition(|w| w == b"GGATSFFO")
        .expect("offset tagg");
    let offset = u32::from_le_bytes(paa[at + 12..at + 16].try_into().expect("4 bytes")) as usize;
    u16::from_le_bytes([paa[offset], paa[offset + 1]])
}

#[cfg(feature = "image")]
#[test]
fn lzo_compress_marks_large_dxt_mipmaps() {
    let (mut raw, _) = raw_paa(256, PaaFormat::Dxt5);
    let raw_file = to_bytes(&raw);
    assert_eq!(first_mipmap_width_field(&raw_file) & 0x8000, 0);
    raw.lzo_compress().expect("compresses");
    let out = to_bytes(&raw);
    assert!(out.len() < raw_file.len());
    assert_ne!(first_mipmap_width_field(&out) & 0x8000, 0);
    let back = Paa::read(&mut Cursor::new(out)).expect("reads back");
    let expected = Paa::read(&mut Cursor::new(raw_file)).expect("reads back");
    assert_eq!(
        back.to_dynamic_image().expect("decodes").to_rgba8(),
        expected.to_dynamic_image().expect("decodes").to_rgba8()
    );
}

#[cfg(feature = "image")]
#[test]
fn lzo_compress_leaves_small_and_non_dxt_unchanged() {
    for (size, format) in [
        (64, PaaFormat::Dxt5),
        (128, PaaFormat::Dxt1),
        (256, PaaFormat::Rgba),
    ] {
        let (mut paa, _) = raw_paa(size, format);
        let before = to_bytes(&paa);
        paa.lzo_compress().expect("no error");
        assert_eq!(to_bytes(&paa), before, "{size} {format:?}");
    }
}

#[cfg(feature = "image")]
#[test]
fn lzo_compress_twice_is_idempotent() {
    let (mut paa, _) = raw_paa(256, PaaFormat::Dxt1);
    paa.lzo_compress().expect("compresses");
    let once = to_bytes(&paa);
    paa.lzo_compress().expect("compresses");
    assert_eq!(to_bytes(&paa), once);
}

/// A DXT1 PAA of 256 x 256 whose first mipmap is stored raw but longer than the raw size.
#[cfg(feature = "image")]
#[expect(clippy::expect_used, reason = "tests fail by panicking")]
fn oversized_first_mipmap_file() -> Vec<u8> {
    let first: Vec<u8> = (0..40000u32)
        .map(|i| u8::try_from(i / 160).unwrap_or(255))
        .collect();
    let second = vec![7u8; 8192];
    let offset0 = 2u32 + 8 + 4 + 64 + 2;
    let offset1 = offset0 + 7 + u32::try_from(first.len()).expect("fits");
    let mut out = vec![0x01, 0xff];
    out.extend_from_slice(b"GGATSFFO");
    out.extend_from_slice(&64u32.to_le_bytes());
    let mut table = vec![offset0, offset1];
    table.resize(16, 0);
    for o in table {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    for (w, h, data) in [(256u16, 256u16, &first), (128, 128, &second)] {
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&u32::try_from(data.len()).expect("fits").to_le_bytes()[..3]);
        out.extend_from_slice(data);
    }
    out.extend_from_slice(&[0; 6]);
    out
}

#[cfg(feature = "image")]
#[test]
fn lzo_compress_leaves_an_oversized_raw_mipmap_readable() {
    let mut paa = Paa::read(&mut Cursor::new(oversized_first_mipmap_file())).expect("reads");
    let before = paa.to_dynamic_image().expect("decodes").to_rgba8();
    paa.lzo_compress().expect("no error");
    let back = Paa::read(&mut Cursor::new(to_bytes(&paa))).expect("the written file reads");
    assert_eq!(back.to_dynamic_image().expect("decodes").to_rgba8(), before);
}

#[test]
fn indent_too_long_reports_the_length() {
    let msg = dayzmake_core::config::WriteOptions::new()
        .indent(" ".repeat(65))
        .expect_err("too long")
        .to_string();
    assert!(msg.contains("65 characters"), "{msg}");
    assert!(msg.len() < 120, "{msg}");
}
