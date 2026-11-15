#![no_main]

use dayzmake_core::pbo::PBO;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut headers = data;
    let _ = PBO::read_headers(&mut headers);
    let mut input = data;
    if let Ok(pbo) = PBO::read(&mut input) {
        for name in pbo.entry_names() {
            let _ = pbo.file_data(name);
        }
    }
});
