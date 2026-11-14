#![no_main]

use dayzmake::paa::Paa;
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    if let Ok(paa) = Paa::read(&mut Cursor::new(data)) {
        let _ = paa.to_dynamic_image();
    }
});
