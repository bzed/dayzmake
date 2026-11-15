//! The pure-Rust SHA-1 used for the PBO trailer gives the standard SHA-1 bytes.
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
fn sha1_matches_the_fips_180_vectors() {
    let hex = |data: &[u8]| -> String {
        use std::fmt::Write;
        Sha1::digest(data).iter().fold(String::new(), |mut out, b| {
            write!(out, "{b:02x}").unwrap();
            out
        })
    };
    assert_eq!(hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    assert_eq!(
        hex(&vec![b'a'; 1_000_000]),
        "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
    );
}
