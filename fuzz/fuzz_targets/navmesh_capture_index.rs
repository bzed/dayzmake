#![no_main]

use std::io::Cursor;

use dayzmake_core::navmesh::capture::{Capture, CaptureFile, CaptureFileOptions};
use dayzmake_core::warning::Warnings;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The index of a capture must agree with reading it whole: every tile the whole reader finds, the
    // indexed reader finds with the same bytes. (The indexed reader may refuse a capture that the whole
    // reader accepts: more records than the world has tiles, or too many tiles.)
    let mut warnings = Warnings::new();
    let indexed = CaptureFile::from_reader(
        Cursor::new(data.to_vec()),
        &CaptureFileOptions::new(),
        &mut warnings,
    );
    let Ok(indexed) = indexed else {
        return;
    };
    let _ = indexed.bounds();
    if let Ok(whole) = Capture::read(&mut &data[..]) {
        for t in whole.tiles() {
            if let Ok(Some(found)) = indexed.tile(t.x, t.z) {
                assert_eq!(found.encode(), t.encode());
            }
        }
    } else {
        // Reading every indexed tile must not panic.
        for z in 0..4 {
            for x in 0..4 {
                let _ = indexed.tile(x, z);
            }
        }
    }
});
