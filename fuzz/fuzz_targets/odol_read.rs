#![no_main]

use std::io::Cursor;

use dayzmake_core::odol::Odol;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = Odol::read(&mut Cursor::new(data));
});
