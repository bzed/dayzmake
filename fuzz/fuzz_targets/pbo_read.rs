#![no_main]

use dayzmake::pbo::PBO;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    if let Ok(pbo) = PBO::read(&mut input) {
        for name in pbo.files.keys() {
            let _ = pbo.file_data(name);
        }
    }
});
