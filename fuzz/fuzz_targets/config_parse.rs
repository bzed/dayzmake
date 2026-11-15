#![no_main]

use dayzmake_core::config::Config;
use dayzmake_core::warning::Warnings;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let input = String::from_utf8_lossy(data).into_owned();
    let mut warnings = Warnings::default();
    let _ = Config::from_string(input, None, &[], &mut warnings);
});
