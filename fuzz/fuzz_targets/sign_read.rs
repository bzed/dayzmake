#![no_main]

use dayzmake::sign::{BIPrivateKey, BIPublicKey, BISign};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = BIPrivateKey::read(&mut &data[..]);
    let _ = BIPublicKey::read(&mut &data[..]);
    let _ = BISign::read(&mut &data[..]);
});
