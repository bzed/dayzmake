#![no_main]

use dayzmake::config::Config;
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let _ = Config::read_rapified(&mut Cursor::new(data));
});
