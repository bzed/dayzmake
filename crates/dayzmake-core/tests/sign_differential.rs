//! Differential tests of BI signing: they pin the RSA results of dayzmake against an independent
//! implementation, so a change of the RSA backend can be measured.
//!
//! dayzmake signs with the `rsa` crate; the test computes the signatures independently with `OpenSSL`'s
//! `BigNum::mod_exp` (the roles were the other way round before the switch to `rsa`). Keys are generated here
//! from fixed seeds; no key material is committed except the throwaway `armk` fixture key.
#![cfg(feature = "sign")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test helpers; the allow-in-tests settings only cover #[test] functions"
)]

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use dayzmake_core::pbo::PBO;
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign, BISignVersion};
use openssl::bn::{BigNum, BigNumContext};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::{BoxedUint, PrivateKey};
use sha1::{Digest, Sha1};

fn sign_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/sign")
}

fn read_pbo(bytes: &[u8]) -> PBO {
    PBO::read(&mut Cursor::new(bytes)).unwrap()
}

fn armk() -> BIPrivateKey {
    BIPrivateKey::read(&mut fs::File::open(sign_dir().join("armk.biprivatekey")).unwrap()).unwrap()
}

fn sign_bytes(key: &BIPrivateKey, pbo: &PBO, version: BISignVersion) -> Vec<u8> {
    let mut out = Vec::new();
    key.sign(pbo, version).unwrap().write(&mut out).unwrap();
    out
}

/// The signature version stored in a `.bisign` file (it follows the first signature value).
fn stored_version(bisign: &[u8]) -> BISignVersion {
    let name_len = bisign.iter().position(|&b| b == 0).unwrap() + 1;
    let length = usize::try_from(u32::from_le_bytes(
        bisign[name_len + 16..name_len + 20].try_into().unwrap(),
    ))
    .unwrap();
    // header (24) + n + length field (4) + sig1
    let at = name_len + 24 + length / 8 + 4 + length / 8;
    match u32::from_le_bytes(bisign[at..at + 4].try_into().unwrap()) {
        2 => BISignVersion::V2,
        3 => BISignVersion::V3,
        v => panic!("unknown signature version {v}"),
    }
}

/// The three signature values (little-endian, `length / 8` bytes each) of a `.bisign` file.
fn signature_values(bisign: &[u8]) -> [Vec<u8>; 3] {
    let name_len = bisign.iter().position(|&b| b == 0).unwrap() + 1;
    let length = usize::try_from(u32::from_le_bytes(
        bisign[name_len + 16..name_len + 20].try_into().unwrap(),
    ))
    .unwrap();
    let size = length / 8;
    let first = name_len + 24 + size + 4;
    // sig1, then (version, length) before sig2, then a length before sig3
    let starts = [first, first + size + 8, first + 2 * size + 12];
    starts.map(|s| bisign[s..s + size].to_vec())
}

/// The three padded hashes BI signs, computed from the PBO with SHA-1 and BI's padding (independently of
/// `sign.rs`). Test PBOs have ASCII prefixes, so the prefix string is hashed as is.
fn padded_hashes(pbo: &PBO, version: BISignVersion, size: usize) -> [Vec<u8>; 3] {
    let mut names: Vec<(Vec<u8>, &[u8])> = pbo
        .entries()
        .map(|(n, d)| (pbo.name_bytes(n).to_ascii_lowercase(), d))
        .collect();
    names.sort();
    let mut namehash = Sha1::new();
    for (name, data) in &names {
        if !data.is_empty() {
            namehash.update(name);
        }
    }
    let namehash = namehash.finalize();

    let mut filehash = Sha1::new();
    let mut nothing = true;
    for (name, data) in pbo.entries() {
        let ext = name.rsplit('.').next().unwrap_or("");
        let hashed = match version {
            BISignVersion::V2 => !matches!(
                ext,
                "paa"
                    | "jpg"
                    | "p3d"
                    | "tga"
                    | "rvmat"
                    | "lip"
                    | "ogg"
                    | "wss"
                    | "png"
                    | "rtm"
                    | "pac"
                    | "fxy"
                    | "wrp"
            ),
            _ => matches!(
                ext,
                "sqf" | "c" | "inc" | "bikb" | "ext" | "fsm" | "sqm" | "hpp" | "cfg" | "sqs" | "h"
            ),
        };
        if hashed {
            filehash.update(data);
            nothing = false;
        }
    }
    if nothing {
        filehash.update(if version == BISignVersion::V2 {
            &b"nothing"[..]
        } else {
            &b"gnihton"[..]
        });
    }
    let filehash = filehash.finalize();

    let mut suffix = Vec::new();
    if let Some(prefix) = pbo.prefix() {
        suffix.extend(prefix.as_bytes());
        if !prefix.ends_with('\\') {
            suffix.push(b'\\');
        }
    }
    let hash1 = pbo.checksum().unwrap().to_vec();
    let mut h2 = Sha1::new();
    h2.update(&hash1);
    h2.update(namehash);
    h2.update(&suffix);
    let mut h3 = Sha1::new();
    h3.update(filehash);
    h3.update(namehash);
    h3.update(&suffix);

    [hash1, h2.finalize().to_vec(), h3.finalize().to_vec()].map(|hash| {
        let mut v = vec![0, 1];
        v.resize(size - 36, 255);
        v.extend(b"\x00\x30\x21\x30\x09\x06\x05\x2b\x0e\x03\x02\x1a\x05\x00\x04\x14");
        v.extend(hash);
        v
    })
}

/// Little-endian bytes of `value`, zero-padded to `size`.
fn le(value: &BoxedUint, size: usize) -> Vec<u8> {
    let mut v = value.to_be_bytes_trimmed_vartime().to_vec();
    v.reverse();
    assert!(v.len() <= size);
    v.resize(size, 0);
    v
}

/// Little-endian bytes of `value`, zero-padded to `size`.
fn le_bn(value: &BigNum, size: usize) -> Vec<u8> {
    let mut v = value.to_vec();
    v.reverse();
    assert!(v.len() <= size);
    v.resize(size, 0);
    v
}

/// The `.biprivatekey` bytes of an `rsa` key (BI layout, same as `BIPrivateKey::write`).
fn biprivatekey_bytes(key: &PrivateKey, bits: u32, name: &str) -> Vec<u8> {
    let size = usize::try_from(bits / 8).unwrap();
    let mut out = name.as_bytes().to_vec();
    out.push(0);
    out.extend((bits / 16 * 9 + 20).to_le_bytes());
    out.extend(b"\x07\x02\x00\x00\x00\x24\x00\x00RSA2");
    out.extend(bits.to_le_bytes());
    out.extend(65537u32.to_le_bytes());
    let primes = key.primes();
    out.extend(le(key.n().as_ref(), size));
    out.extend(le(primes[0].as_ref(), size / 2));
    out.extend(le(primes[1].as_ref(), size / 2));
    out.extend(le(key.dp().unwrap(), size / 2));
    out.extend(le(key.dq().unwrap(), size / 2));
    out.extend(le(&key.crt_coefficient().unwrap(), size / 2));
    out.extend(le(key.d(), size));
    out
}

#[test]
fn fixture_signatures_are_reproduced() {
    let key = armk();
    let mut checked = 0;
    for entry in fs::read_dir(sign_dir()).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap().to_string();
        let Some(pbo_name) = file.strip_suffix(".armk.bisign") else {
            continue;
        };
        let pbo = read_pbo(&fs::read(sign_dir().join(pbo_name)).unwrap());
        let expected = fs::read(&path).unwrap();
        let version = stored_version(&expected);
        assert_eq!(sign_bytes(&key, &pbo, version), expected, "{file}");
        checked += 1;
    }
    assert!(checked >= 2, "expected the BI fixtures, found {checked}");
}

#[test]
fn seeded_keys_sign_like_an_independent_implementation() {
    let pbo = read_pbo(&fs::read(sign_dir().join("testsig.pbo")).unwrap());
    let bits_of = [1024u32, 1024, 1024, 1024, 1024, 1024, 2048, 4096];
    for (seed, bits) in (1u64..).zip(bits_of) {
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let key = PrivateKey::new_with_exp(
            &mut rng,
            usize::try_from(bits).unwrap(),
            BoxedUint::from(65537u64),
        )
        .unwrap();
        let bytes = biprivatekey_bytes(&key, bits, &format!("seed{seed}"));

        // the helper writes exactly what dayzmake reads and writes
        let bi = BIPrivateKey::read(&mut Cursor::new(&bytes)).unwrap();
        let mut again = Vec::new();
        bi.write(&mut again).unwrap();
        assert_eq!(again, bytes, "seed {seed}: key bytes round trip");

        let size = usize::try_from(bits / 8).unwrap();
        for version in [BISignVersion::V2, BISignVersion::V3] {
            let signed = sign_bytes(&bi, &pbo, version);
            assert_eq!(stored_version(&signed), version);
            let padded = padded_hashes(&pbo, version, size);
            let values = signature_values(&signed);
            for (i, (value, hash)) in values.iter().zip(&padded).enumerate() {
                let mut ctx = BigNumContext::new().unwrap();
                let n =
                    BigNum::from_slice(&key.n().as_ref().to_be_bytes_trimmed_vartime()).unwrap();
                let d = BigNum::from_slice(&key.d().to_be_bytes_trimmed_vartime()).unwrap();
                let m = BigNum::from_slice(hash).unwrap();
                let mut expected = BigNum::new().unwrap();
                expected.mod_exp(&m, &d, &n, &mut ctx).unwrap();
                assert_eq!(
                    *value,
                    le_bn(&expected, size),
                    "seed {seed} {version:?} signature {}",
                    i + 1
                );
                // the public operation gives the padded hash back
                let sig =
                    BigNum::from_slice(&value.iter().rev().copied().collect::<Vec<_>>()).unwrap();
                let mut back = BigNum::new().unwrap();
                back.mod_exp(&sig, &BigNum::from_u32(65537).unwrap(), &n, &mut ctx)
                    .unwrap();
                assert_eq!(back.to_vec(), hash[1..].to_vec(), "seed {seed} {version:?}");
            }
            // and dayzmake verifies its own signature
            let public = bi.to_public_key().unwrap();
            public
                .verify(&pbo, &BISign::read(&mut Cursor::new(&signed)).unwrap())
                .unwrap();
        }
    }
}

fn flip(bytes: &mut [u8], at: usize) {
    bytes[at] ^= 1;
}

fn key_verify(pbo: &PBO, sig: &[u8], key: &BIPublicKey) -> Result<(), std::io::Error> {
    key.verify(pbo, &BISign::read(&mut Cursor::new(sig)).unwrap())
}

#[test]
fn verify_accepts_and_rejects() {
    let public_bytes = fs::read(sign_dir().join("armk.bikey")).unwrap();
    let public = BIPublicKey::read(&mut Cursor::new(&public_bytes)).unwrap();
    for (pbo_name, sig_name) in [
        ("testsig.pbo", "testsig.pbo.armk.bisign"),
        ("nonutf8.pbo", "nonutf8.pbo.armk.bisign"),
    ] {
        let pbo_bytes = fs::read(sign_dir().join(pbo_name)).unwrap();
        let sig_bytes = fs::read(sign_dir().join(sig_name)).unwrap();
        let verify = |pbo: &[u8], sig: &[u8], key: &BIPublicKey| {
            key.verify(
                &read_pbo(pbo),
                &BISign::read(&mut Cursor::new(sig)).unwrap(),
            )
        };
        verify(&pbo_bytes, &sig_bytes, &public).unwrap();

        // one flipped bit in the signature (first byte of sig1: after name, 24 header bytes, n, length)
        let name_len = sig_bytes.iter().position(|&b| b == 0).unwrap() + 1;
        let mut bad_sig = sig_bytes.clone();
        flip(&mut bad_sig, name_len + 24 + 128 + 4);
        assert!(
            verify(&pbo_bytes, &bad_sig, &public).is_err(),
            "{pbo_name} sig"
        );

        // one flipped bit in the data of a hashed script entry
        let pbo = read_pbo(&pbo_bytes);
        let (_, data) = pbo
            .entries()
            .find(|(n, d)| {
                std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("c"))
                    && !d.is_empty()
            })
            .unwrap();
        let at = pbo_bytes
            .windows(data.len())
            .position(|w| w == data)
            .unwrap();
        let mut bad_pbo = pbo_bytes.clone();
        flip(&mut bad_pbo, at);
        assert!(
            verify(&bad_pbo, &sig_bytes, &public).is_err(),
            "{pbo_name} data"
        );

        // one flipped bit in config.bin or in the header: only hash 1 (the SHA-1 of the whole PBO)
        // covers them, and the stored trailer is kept
        let rejected = |bytes: &[u8]| match PBO::read(&mut Cursor::new(bytes)) {
            Ok(pbo) => key_verify(&pbo, &sig_bytes, &public).is_err(),
            Err(_) => true,
        };
        let config = pbo
            .entries()
            .find(|(n, d)| n.ends_with("config.bin") && !d.is_empty());
        if let Some((_, config)) = config {
            let at = pbo_bytes
                .windows(config.len())
                .position(|w| w == config)
                .unwrap();
            let mut bad = pbo_bytes.clone();
            flip(&mut bad, at + config.len() / 2);
            assert!(rejected(&bad), "{pbo_name} config.bin");
        }
        let mut bad_header = pbo_bytes.clone();
        flip(&mut bad_header, 30);
        assert!(rejected(&bad_header), "{pbo_name} header");

        // one flipped bit in the key's n: a key `BIPublicKey::read` refuses counts as rejected, one it reads
        // must not verify
        let key_name_len = public_bytes.iter().position(|&b| b == 0).unwrap() + 1;
        let mut bad_key = public_bytes.clone();
        flip(&mut bad_key, key_name_len + 24 + 5);
        if let Ok(bad_public) = BIPublicKey::read(&mut Cursor::new(&bad_key)) {
            assert!(
                verify(&pbo_bytes, &sig_bytes, &bad_public).is_err(),
                "{pbo_name} n"
            );
        }
    }
}
