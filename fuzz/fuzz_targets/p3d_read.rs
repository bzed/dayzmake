#![no_main]

use dayzmake::p3d::P3D;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    let _ = P3D::read(&mut input);
});
