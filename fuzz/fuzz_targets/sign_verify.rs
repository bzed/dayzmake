#![no_main]

use std::io::Cursor;

use dayzmake_core::pbo::PBO;
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign, BISignVersion};
use libfuzzer_sys::fuzz_target;

/// A small valid PBO (a fixture of the repository) that the keys and signatures are tried against.
const PBO_TEMPLATE: &[u8] = include_bytes!("../../tests/fixtures/bi/sign/testsig.pbo");

// Input layout: two big-endian u16 lengths, then a `.bikey` of the first length, a `.bisign` of the second
// length, and the rest is a `.biprivatekey`.
fuzz_target!(|data: &[u8]| {
    let [a, b, c, d, rest @ ..] = data else {
        return;
    };
    let key_len = usize::from(u16::from_be_bytes([*a, *b])).min(rest.len());
    let (key, rest) = rest.split_at(key_len);
    let sig_len = usize::from(u16::from_be_bytes([*c, *d])).min(rest.len());
    let (sig, private) = rest.split_at(sig_len);
    let Ok(pbo) = PBO::read(&mut Cursor::new(PBO_TEMPLATE)) else {
        return;
    };
    if let (Ok(key), Ok(sig)) = (BIPublicKey::read(&mut &key[..]), BISign::read(&mut &sig[..])) {
        let _ = key.verify(&pbo, &sig);
    }
    if let Ok(private) = BIPrivateKey::read(&mut &private[..]) {
        let _ = private.sign(&pbo, BISignVersion::V3);
    }
});
