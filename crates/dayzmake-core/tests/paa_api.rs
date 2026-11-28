//! The PAA conversion API: formats, accessors and the bytes the CLI writes.
use std::io::Cursor;

#[cfg(feature = "image")]
use dayzmake_core::image::{DynamicImage, Rgba, RgbaImage};
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
        assert!(
            read.tagg("GGATSFFO").is_none(),
            "the offset tagg is not exposed"
        );
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

/// The file written by `from_dynamic_image`, read back. DXT mipmaps wider than 128 px that LZO shrinks
/// stay compressed after `read`; the others are held raw.
#[cfg(feature = "image")]
#[expect(clippy::expect_used, reason = "tests fail by panicking")]
fn raw_paa(size: u32, format: PaaFormat) -> (Paa, Vec<u8>) {
    let paa = Paa::from_dynamic_image(&smooth(size), format).expect("converts");
    let written = to_bytes(&paa);
    let raw = Paa::read(&mut Cursor::new(written.clone())).expect("reads back");
    (raw, written)
}

/// The width field (with the LZO bit) of the first mipmap of a PAA file.
#[expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test helper parsing output the test itself created"
)]
fn first_mipmap_width_field(paa: &[u8]) -> u16 {
    let at = paa
        .windows(8)
        // a re-written PAA has exactly one offset tagg
        .position(|w| w == b"GGATSFFO")
        .expect("offset tagg");
    let offset = u32::from_le_bytes(paa[at + 12..at + 16].try_into().expect("4 bytes")) as usize;
    u16::from_le_bytes([paa[offset], paa[offset + 1]])
}

#[cfg(feature = "image")]
#[test]
fn lzo_compress_marks_large_dxt_mipmaps() {
    // a raw (unflagged) 256 x 256 DXT1 mipmap
    let blocks: Vec<u8> = (0..32768u32)
        .map(|i| u8::try_from(i / 64 % 251).unwrap_or(0))
        .collect();
    let mut raw =
        Paa::read(&mut Cursor::new(hand_made_paa(&[], &[(256, 256, blocks)]))).expect("reads");
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
    // a raw (unflagged) 256 x 256 DXT1 mipmap, so the first call really compresses
    let blocks: Vec<u8> = (0..32768u32)
        .map(|i| u8::try_from(i / 64 % 251).unwrap_or(0))
        .collect();
    let mut paa =
        Paa::read(&mut Cursor::new(hand_made_paa(&[], &[(256, 256, blocks)]))).expect("reads");
    let raw = to_bytes(&paa);
    paa.lzo_compress().expect("compresses");
    let once = to_bytes(&paa);
    assert_ne!(once, raw, "the first call compresses");
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

/// A hand-built DXT1 file: `taggs`, no palette, then the given `(width field, height, data)` mipmaps.
#[expect(clippy::expect_used, reason = "tests fail by panicking")]
fn hand_made_paa(taggs: &[(&[u8; 8], Vec<u8>)], mipmaps: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![0x01, 0xff];
    for (name, data) in taggs {
        bytes.extend_from_slice(*name);
        bytes.extend_from_slice(&u32::try_from(data.len()).expect("small").to_le_bytes());
        bytes.extend_from_slice(data);
    }
    bytes.extend_from_slice(&[0, 0]); // palette size, also the tagg terminator
    for (w, h, data) in mipmaps {
        bytes.extend_from_slice(&w.to_le_bytes());
        bytes.extend_from_slice(&h.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(data.len()).expect("small").to_le_bytes()[..3]);
        bytes.extend_from_slice(data);
    }
    bytes.extend_from_slice(&[0; 6]);
    bytes
}

#[test]
fn short_raw_mipmap_is_written_unflagged() {
    let input = hand_made_paa(&[], &[(256, 256, vec![0u8; 1000])]);
    let paa = Paa::read(&mut Cursor::new(input)).expect("a short raw mipmap is accepted");
    let width = first_mipmap_width_field(&to_bytes(&paa));
    assert_eq!(width, 256, "the raw mipmap must not carry the LZO flag");
    #[cfg(feature = "image")]
    assert!(
        paa.to_dynamic_image().is_err(),
        "a short mipmap does not fit its dimensions"
    );
}

#[cfg(feature = "image")]
#[test]
fn compressed_paa_decodes_without_a_round_trip() {
    let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(256, 256, |x, y| {
        Rgba([
            u8::try_from(x).unwrap_or(0),
            u8::try_from(y).unwrap_or(0),
            128,
            255,
        ])
    }));
    let paa = Paa::from_dynamic_image(&img, PaaFormat::Dxt5).expect("converts");
    let direct = paa.to_dynamic_image().expect("decodes in memory");
    let read = Paa::read(&mut Cursor::new(to_bytes(&paa))).expect("reads back");
    assert_eq!(direct, read.to_dynamic_image().expect("decodes"));
}

#[cfg(feature = "image")]
#[test]
fn rewritten_paa_has_one_offset_tagg() {
    let blocks = vec![0u8; 8 * 16 * 16];
    let input = hand_made_paa(
        &[(b"GGATCXAM", vec![1, 2, 3, 4]), (b"GGATSFFO", vec![0; 64])],
        &[(64, 64, blocks)],
    );
    let first = Paa::read(&mut Cursor::new(input)).expect("reads");
    let count = |bytes: &[u8]| bytes.windows(8).filter(|w| *w == b"GGATSFFO").count();
    let again = to_bytes(&first);
    assert_eq!(count(&again), 1);
    let second = Paa::read(&mut Cursor::new(again)).expect("reads again");
    assert_eq!(second.tagg("GGATCXAM"), Some(&[1u8, 2, 3, 4][..]));
    assert_eq!(
        first.to_dynamic_image().expect("decodes"),
        second.to_dynamic_image().expect("decodes")
    );
}

/// Deterministic noise that LZO cannot shrink.
#[cfg(feature = "image")]
fn noise_image(size: u32) -> DynamicImage {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    DynamicImage::ImageRgba8(RgbaImage::from_fn(size, size, |_, _| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let b = state.to_le_bytes();
        Rgba([b[0], b[1], b[2], b[3]])
    }))
}

#[cfg(feature = "image")]
#[test]
fn write_rejects_a_mipmap_of_16_mib() {
    let paa = Paa::from_dynamic_image(&noise_image(4096), PaaFormat::Dxt5).expect("converts");
    let mut out = Vec::new();
    let err = paa
        .write(&mut out)
        .expect_err("the length field has 3 bytes");
    let text = err.to_string();
    assert!(
        text.contains("16777216") && text.contains("16777215"),
        "{text}"
    );
}

#[cfg(feature = "image")]
#[test]
fn read_then_write_keeps_lzo_mipmaps_compressed() {
    let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(4096, 4096, |x, y| {
        Rgba([
            u8::try_from(x / 16).unwrap_or(255),
            u8::try_from(y / 16).unwrap_or(255),
            90,
            255,
        ])
    }));
    let paa = Paa::from_dynamic_image(&img, PaaFormat::Dxt5).expect("converts");
    let first = to_bytes(&paa);
    let read = Paa::read(&mut Cursor::new(first.clone())).expect("reads");
    let second = to_bytes(&read);
    assert_eq!(first, second);
    let again = Paa::read(&mut Cursor::new(second)).expect("reads again");
    assert_eq!(
        read.to_dynamic_image().expect("decodes"),
        again.to_dynamic_image().expect("decodes")
    );
}

/// An LZSS stream of literals only: a flag byte of `0xFF` per 8 bytes.
fn lzss_literals(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for group in raw.chunks(8) {
        out.push(0xFF);
        out.extend_from_slice(group);
    }
    out
}

/// The signed byte sum that follows the stream.
fn lzss_checksum(raw: &[u8]) -> [u8; 4] {
    let sum: i32 = raw.iter().map(|&b| i32::from(i8::from_le_bytes([b]))).sum();
    sum.to_le_bytes()
}

/// A PAA of one `size` x `size` mipmap holding `stream` and `checksum`.
#[expect(clippy::expect_used, reason = "tests fail by panicking")]
fn packed_paa(magic: u16, size: u16, stream: &[u8], checksum: [u8; 4]) -> Vec<u8> {
    let mut bytes = magic.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0, 0]); // no taggs, no palette
    let len = u32::try_from(stream.len() + 4).expect("small");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&len.to_le_bytes()[..3]);
    bytes.extend_from_slice(stream);
    bytes.extend_from_slice(&checksum);
    bytes.extend_from_slice(&[0; 6]);
    bytes
}

/// A 4 x 4 PAA of literal bytes with a correct checksum; the pixel data is `pixels`, 2 bytes per pixel.
fn packed_4x4(magic: u16, pixels: &[[u8; 2]; 16]) -> Vec<u8> {
    let raw: Vec<u8> = pixels.iter().flatten().copied().collect();
    packed_paa(magic, 4, &lzss_literals(&raw), lzss_checksum(&raw))
}

#[cfg(feature = "image")]
#[test]
fn rgba4444_decodes_to_rgba() {
    // nibbles from high to low: A, R, G, B (as ImageToPAA writes them, see tests/bi_paa_golden.rs)
    let mut pixels = [[0x21, 0x43]; 16];
    pixels[1] = [0xFF, 0xFF];
    pixels[2] = [0x00, 0x00];
    let paa = Paa::read(&mut Cursor::new(packed_4x4(0x4444, &pixels))).expect("reads");
    assert_eq!(paa.compression(), "ARGB4444");
    let img = paa.to_dynamic_image().expect("decodes").to_rgba8();
    assert_eq!((img.width(), img.height()), (4, 4));
    assert_eq!(img.get_pixel(0, 0).0, [51, 34, 17, 68]);
    assert_eq!(img.get_pixel(1, 0).0, [255; 4]);
    assert_eq!(img.get_pixel(2, 0).0, [0; 4]);
}

#[cfg(feature = "image")]
#[test]
fn argb1555_decodes_to_rgba() {
    // bit 15 alpha, then 5 bits each of R, G, B
    let mut pixels = [[0u8; 2]; 16];
    pixels[0] = 0xFFFFu16.to_le_bytes(); // white, opaque
    pixels[1] = 0x7C00u16.to_le_bytes(); // red, transparent
    pixels[2] = 0x83E0u16.to_le_bytes(); // green, opaque
    pixels[3] = (0x8000u16 | 16).to_le_bytes(); // blue 16/31, opaque
    let paa = Paa::read(&mut Cursor::new(packed_4x4(0x1555, &pixels))).expect("reads");
    assert_eq!(paa.compression(), "ARGB1555");
    let img = paa.to_dynamic_image().expect("decodes").to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
    assert_eq!(img.get_pixel(1, 0).0, [255, 0, 0, 0]);
    assert_eq!(img.get_pixel(2, 0).0, [0, 255, 0, 255]);
    assert_eq!(img.get_pixel(3, 0).0, [0, 0, 131, 255]);
}

#[cfg(feature = "image")]
#[test]
fn ai88_decodes_to_grey_with_alpha() {
    let mut pixels = [[7, 9]; 16];
    pixels[5] = [200, 255];
    let paa = Paa::read(&mut Cursor::new(packed_4x4(0x8080, &pixels))).expect("reads");
    assert_eq!(paa.compression(), "AI88");
    let img = paa.to_dynamic_image().expect("decodes").to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [7, 7, 7, 9]);
    assert_eq!(img.get_pixel(1, 1).0, [200, 200, 200, 255]);
}

#[cfg(feature = "image")]
#[test]
fn lzss_back_references_are_followed() {
    // literals grey 5, alpha 6, then two references to 2 bytes back: 18 and 12 bytes
    let stream = [0b0000_0011, 5, 6, 2, 0x0F, 2, 0x09];
    let raw = [5u8, 6].repeat(16);
    let bytes = packed_paa(0x8080, 4, &stream, lzss_checksum(&raw));
    let img = Paa::read(&mut Cursor::new(bytes))
        .expect("reads")
        .to_dynamic_image()
        .expect("decodes")
        .to_rgba8();
    assert!(img.pixels().all(|p| p.0 == [5, 5, 5, 6]));
}

#[test]
fn packed_formats_are_written_back_unchanged() {
    let bytes = packed_4x4(0x4444, &[[1, 2]; 16]);
    let paa = Paa::read(&mut Cursor::new(&bytes)).expect("reads");
    let written = to_bytes(&paa);
    let again = Paa::read(&mut Cursor::new(&written)).expect("reads back");
    assert_eq!(again.compression(), "ARGB4444");
    assert_eq!(to_bytes(&again), written);
    // the mipmap's stored bytes are in the output
    let raw = [1u8, 2].repeat(16);
    let stored = [lzss_literals(&raw), lzss_checksum(&raw).to_vec()].concat();
    assert!(written.windows(stored.len()).any(|w| w == stored));
}

#[test]
fn a_wrong_checksum_is_an_error() {
    let raw = [3u8; 32];
    let bytes = packed_paa(0x8080, 4, &lzss_literals(&raw), [0, 0, 0, 0]);
    let err = Paa::read(&mut Cursor::new(bytes)).expect_err("checksum mismatch");
    assert!(err.to_string().contains("checksum"), "{err}");
}

#[test]
fn a_truncated_lzss_stream_is_an_error() {
    let raw = [3u8; 32];
    let mut stream = lzss_literals(&raw);
    stream.truncate(stream.len() - 12);
    let bytes = packed_paa(0x4444, 4, &stream, lzss_checksum(&raw));
    assert!(Paa::read(&mut Cursor::new(bytes)).is_err());
    // no room for the checksum
    let bytes = packed_paa(0x4444, 4, &lzss_literals(&raw)[..20], [0; 4]);
    assert!(Paa::read(&mut Cursor::new(bytes)).is_err());
}

#[test]
fn a_png_named_paa_is_called_a_png() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0; 32]);
    let err = Paa::read(&mut Cursor::new(png)).expect_err("a PNG is not a PAA");
    assert!(err.to_string().contains("PNG"), "{err}");
}

#[test]
fn an_unknown_magic_is_still_invalid() {
    let err = Paa::read(&mut Cursor::new(vec![0x34, 0x12, 0, 0, 0, 0])).expect_err("unknown magic");
    assert!(err.to_string().contains("Invalid PAA magic number: 0x1234"));
    // 0x5089 without the rest of the PNG signature
    let err = Paa::read(&mut Cursor::new(vec![0x89, 0x50, 0, 0, 0, 0, 0, 0])).expect_err("unknown");
    assert!(
        err.to_string().contains("Invalid PAA magic number"),
        "{err}"
    );
}

/// A PNG decoded with the re-exported `image` (0.25: `ImageReader`) converts without a second decode (#132).
#[cfg(feature = "image")]
#[test]
fn from_dynamic_image_takes_an_image_0_25_decoded_png() {
    let mut png = Vec::new();
    DynamicImage::ImageRgba8(RgbaImage::new(8, 8))
        .write_to(
            &mut std::io::Cursor::new(&mut png),
            dayzmake_core::image::ImageFormat::Png,
        )
        .expect("encode");
    let img = dayzmake_core::image::ImageReader::new(std::io::Cursor::new(png))
        .with_guessed_format()
        .expect("guess")
        .decode()
        .expect("decode");
    let paa = Paa::from_dynamic_image(&img, PaaFormat::Dxt1).expect("paa");
    assert_eq!((paa.width(), paa.height()), (8, 8));
}
