//! A signature covers the SHA-1 of the whole PBO (hash 1): a PBO whose bytes were changed while its
//! stored trailer was kept must not verify, and must not be signed (#46).
#![cfg(feature = "sign")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helpers; the allow-in-tests settings only cover #[test] functions"
)]

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use dayzmake_core::pbo::PBO;
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign, BISignVersion};

fn sign_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/sign")
}

fn read_pbo(bytes: &[u8]) -> PBO {
    PBO::read(&mut Cursor::new(bytes)).unwrap()
}

fn public() -> BIPublicKey {
    BIPublicKey::read(&mut fs::File::open(sign_dir().join("armk.bikey")).unwrap()).unwrap()
}

fn private() -> BIPrivateKey {
    BIPrivateKey::read(&mut fs::File::open(sign_dir().join("armk.biprivatekey")).unwrap()).unwrap()
}

fn position(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap()
}

/// The fixture with the byte at `at` changed; the trailer stays as it was.
fn tampered(at: usize) -> Vec<u8> {
    let mut bytes = fs::read(sign_dir().join("testsig.pbo")).unwrap();
    bytes[at] ^= 0x20;
    bytes
}

fn entry_offset(pbo: &[u8], name: &str) -> usize {
    let parsed = read_pbo(pbo);
    let data = parsed
        .entries()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| d.to_vec())
        .unwrap();
    assert!(!data.is_empty(), "{name}");
    position(pbo, &data)
}

fn assert_mismatch(error: &std::io::Error) {
    let text = error.to_string();
    assert!(text.contains("checksum mismatch"), "{text}");
}

#[test]
fn unmodified_fixtures_still_verify() {
    let sig = fs::read(sign_dir().join("testsig.pbo.armk.bisign")).unwrap();
    let pbo = read_pbo(&fs::read(sign_dir().join("testsig.pbo")).unwrap());
    assert_eq!(pbo.checksum(), pbo.computed_checksum());
    public()
        .verify(&pbo, &BISign::read(&mut Cursor::new(sig)).unwrap())
        .unwrap();
}

#[test]
fn tampering_with_any_part_of_the_pbo_fails_verification() {
    let original = fs::read(sign_dir().join("testsig.pbo")).unwrap();
    let sig =
        BISign::read(&mut fs::File::open(sign_dir().join("testsig.pbo.armk.bisign")).unwrap())
            .unwrap();
    let names: Vec<String> = read_pbo(&original)
        .entry_names()
        .map(str::to_string)
        .collect();
    let find = |suffix: &str| names.iter().find(|n| n.ends_with(suffix)).unwrap().clone();

    let cases = [
        ("config.bin", entry_offset(&original, &find("config.bin"))),
        ("txt", entry_offset(&original, &find(".txt"))),
        ("rvmat", entry_offset(&original, &find(".rvmat"))),
        (
            "header extension",
            position(&original, b"product\0dayz") + 8,
        ),
    ];
    for (what, at) in cases {
        let bytes = tampered(at);
        let pbo = read_pbo(&bytes);
        assert_ne!(pbo.checksum(), pbo.computed_checksum(), "{what}");
        let error = public().verify(&pbo, &sig).unwrap_err();
        assert_mismatch(&error);
    }
}

#[test]
fn a_changed_v2_excluded_entry_fails_v2_verification() {
    let original = fs::read(sign_dir().join("testsig.pbo")).unwrap();
    let signature = private()
        .sign(&read_pbo(&original), BISignVersion::V2)
        .unwrap();
    public().verify(&read_pbo(&original), &signature).unwrap();

    let name = "data\\x.rvmat";
    let pbo = read_pbo(&tampered(entry_offset(&original, name)));
    assert_mismatch(&public().verify(&pbo, &signature).unwrap_err());
}

#[test]
fn signing_a_pbo_with_a_wrong_trailer_fails() {
    let original = fs::read(sign_dir().join("testsig.pbo")).unwrap();
    let at = entry_offset(&original, "data\\x.rvmat");
    let pbo = read_pbo(&tampered(at));
    for version in [BISignVersion::V2, BISignVersion::V3] {
        assert_mismatch(&private().sign(&pbo, version).unwrap_err());
    }

    // a wrong trailer alone is as bad as a changed body
    let mut bytes = original;
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let pbo = read_pbo(&bytes);
    assert_mismatch(&private().sign(&pbo, BISignVersion::V3).unwrap_err());
}

#[test]
fn a_short_trailer_is_an_error() {
    let mut bytes = fs::read(sign_dir().join("testsig.pbo")).unwrap();
    bytes.truncate(bytes.len() - 5);
    assert!(PBO::read(&mut Cursor::new(&bytes)).is_err());
    bytes.truncate(bytes.len() - 20);
    assert!(PBO::read(&mut Cursor::new(&bytes)).is_err());
}

#[test]
fn a_modified_pbo_has_no_computed_checksum() {
    let mut pbo = read_pbo(&fs::read(sign_dir().join("testsig.pbo")).unwrap());
    assert!(pbo.computed_checksum().is_some());
    pbo.insert_entry("new.txt", b"x".to_vec()).unwrap();
    assert!(pbo.computed_checksum().is_none());
    assert!(pbo.checksum().is_none());
}
