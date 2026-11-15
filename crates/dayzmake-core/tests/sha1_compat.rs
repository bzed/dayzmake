//! The pure-Rust SHA-1 used for the PBO trailer gives the same bytes as OpenSSL did.
use std::fs;
use std::io::Cursor;
use std::path::Path;

use dayzmake_core::pbo::PBO;
use sha1::{Digest, Sha1};

#[expect(clippy::unwrap_used, reason = "test helper")]
fn fixture_pbos() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut out = Vec::new();
    for dir in ["bi/sign", "fuzz"] {
        let mut names: Vec<_> = fs::read_dir(root.join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "pbo"))
            .collect();
        names.sort();
        out.extend(names);
    }
    out
}

/// Deterministic noise (xorshift, fixed seed).
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[0]
        })
        .collect()
}

#[test]
fn pbo_trailer_is_unchanged() {
    let mut accepted = 0;
    for path in fixture_pbos() {
        let original = fs::read(&path).unwrap();
        let Ok(pbo) = PBO::read(&mut Cursor::new(&original)) else {
            continue;
        };
        accepted += 1;
        let written = pbo.to_cursor().unwrap().into_inner();
        // Layout: hashed bytes, a 0 byte, the 20-byte SHA-1 of the hashed bytes.
        let (rest, trailer) = written.split_at(written.len() - 20);
        let body = &rest[..rest.len() - 1];
        assert_eq!(trailer, Sha1::digest(body).as_slice(), "{}", path.display());
        // Only the BI fixtures carry a real checksum; the fuzz inputs store garbage.
        if path.to_string_lossy().contains("bi/sign") {
            assert_eq!(Some(trailer), pbo.checksum(), "{}", path.display());
        }
    }
    assert!(accepted >= 2, "the signing fixtures must be read");
}

#[test]
fn sha1_matches_openssl_on_padding_boundaries() {
    let mut inputs: Vec<Vec<u8>> = [0usize, 1, 55, 56, 63, 64, 65, 119, 120, 128]
        .iter()
        .map(|&n| noise(n))
        .collect();
    inputs.push(noise(1 << 20));
    for data in inputs {
        let theirs = openssl::hash::hash(openssl::hash::MessageDigest::sha1(), &data).unwrap();
        assert_eq!(
            Sha1::digest(&data).as_slice(),
            &*theirs,
            "len {}",
            data.len()
        );
    }
}
