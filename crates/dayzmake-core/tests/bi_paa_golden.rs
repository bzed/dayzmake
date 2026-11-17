//! Golden tests against PAA files made by BI's `ImageToPAA.exe` (see tests/fixtures/bi/README.md).
//!
//! The input is `rgbq.png`, 8 x 8: red top left, green top right, blue bottom left, white with alpha 136
//! bottom right.
#![cfg(feature = "image")]

use std::io::Cursor;
use std::path::PathBuf;

use dayzmake_core::paa::Paa;

#[expect(clippy::unwrap_used, reason = "test helper")]
fn corners(name: &str, compression: &str) -> [[u8; 4]; 4] {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bi/paa")
        .join(name);
    let paa = Paa::read(&mut Cursor::new(std::fs::read(path).unwrap())).unwrap();
    assert_eq!(paa.compression(), compression);
    let img = paa.to_dynamic_image().unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (8, 8));
    [(0, 0), (7, 0), (0, 7), (7, 7)].map(|(x, y)| img.get_pixel(x, y).0)
}

#[test]
fn argb4444_from_imagetopaa() {
    assert_eq!(
        corners("rgbq_4444.paa", "ARGB4444"),
        [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 136]
        ]
    );
}

#[test]
fn argb1555_from_imagetopaa() {
    assert_eq!(
        corners("rgbq_1555.paa", "ARGB1555"),
        [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255]
        ]
    );
}

#[test]
fn ai88_from_imagetopaa() {
    // grey from the luma of the input (0.299 R + 0.587 G + 0.114 B), alpha unchanged
    assert_eq!(
        corners("rgbq_88.paa", "AI88"),
        [
            [76, 76, 76, 255],
            [150, 150, 150, 255],
            [29, 29, 29, 255],
            [255, 255, 255, 136]
        ]
    );
}

// Error paths of the 16-bit formats, from the same BI files with their first (8 x 8) mipmap damaged.

const FORMATS_16: [&str; 3] = ["rgbq_4444.paa", "rgbq_1555.paa", "rgbq_88.paa"];

/// A BI file and the offset of the header of its first mipmap (width, height, 3 byte size).
#[expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "test helper")]
fn bi_file(name: &str) -> (Vec<u8>, usize) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bi/paa")
        .join(name);
    let bytes = std::fs::read(path).unwrap();
    // magic, then taggs (8 byte name, u32 size, data) up to a zero byte, then the u16 palette size
    let mut pos = 2;
    while bytes[pos] != 0 {
        let size = u32::from_le_bytes(bytes[pos + 8..pos + 12].try_into().unwrap());
        pos += 12 + usize::try_from(size).unwrap();
    }
    pos += 2;
    assert_eq!(bytes[pos..pos + 4], [8, 0, 8, 0], "first mipmap is 8 x 8");
    (bytes, pos)
}

/// The mipmap size: 3 bytes, little-endian, after the width and height.
#[expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "test helper")]
fn mipmap_size(bytes: &[u8], hdr: usize) -> usize {
    let [a, b, c] = bytes[hdr + 4..hdr + 7].try_into().unwrap();
    let size = usize::from(a) | usize::from(b) << 8 | usize::from(c) << 16;
    assert!(4 < size && size < 0x100_0000, "mipmap size {size}");
    assert!(
        hdr + 7 + size <= bytes.len(),
        "mipmap size {size} past the file"
    );
    size
}

/// A size as 3 bytes, little-endian.
fn u24_le(size: usize) -> [u8; 3] {
    assert!(size < 0x100_0000);
    let b = size.to_le_bytes();
    [b[0], b[1], b[2]]
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn read_error(bytes: Vec<u8>) -> String {
    Paa::read(&mut Cursor::new(bytes)).unwrap_err().to_string()
}

#[test]
fn sixteen_bit_mipmap_without_checksum_is_an_error() {
    for name in FORMATS_16 {
        let (bytes, hdr) = bi_file(name);
        let size = mipmap_size(&bytes, hdr);
        let end = hdr + 7 + size;
        let mut cut = bytes[..hdr + 4].to_vec();
        cut.extend_from_slice(&u24_le(size - 4));
        cut.extend_from_slice(&bytes[hdr + 7..end - 4]);
        cut.extend_from_slice(&bytes[end..]);
        let message = read_error(cut);
        assert!(
            message.contains("the checksum is missing"),
            "{name}: {message}"
        );
    }
}

#[test]
fn sixteen_bit_mipmap_with_lzo_bit_is_an_error() {
    for name in FORMATS_16 {
        let (mut bytes, hdr) = bi_file(name);
        bytes[hdr + 1] |= 0x80;
        let message = read_error(bytes);
        assert!(
            message.contains("LZO compression is only supported for DXT formats"),
            "{name}: {message}"
        );
    }
}

#[test]
fn sixteen_bit_mipmap_cut_in_the_data_is_an_error() {
    for name in FORMATS_16 {
        let (bytes, hdr) = bi_file(name);
        let size = mipmap_size(&bytes, hdr);
        let message = read_error(bytes[..hdr + 7 + size / 2].to_vec());
        assert!(
            message.contains("Truncated PAA mipmap."),
            "{name}: {message}"
        );
    }
}
