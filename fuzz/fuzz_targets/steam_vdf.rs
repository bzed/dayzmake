#![no_main]

use dayzmake::api::steam::{vdf_values, vdf_values_unescaped};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // `.vdf` and `.acf` files are read lossily, like `libraries` and `find_app` do.
    let text = String::from_utf8_lossy(data);
    for key in ["path", "installdir", "buildid"] {
        let _ = vdf_values(&text, key);
        let _ = vdf_values_unescaped(&text, key);
    }
});
