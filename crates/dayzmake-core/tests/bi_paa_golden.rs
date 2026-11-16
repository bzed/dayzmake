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
