//! Hostile and odd `.biprivatekey` / `.bikey` files: inconsistent components, zero and one values, wrong
//! lengths and error text. Keys are generated from fixed seeds; the key bytes use BI's layout.
#![cfg(feature = "sign")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helpers; the allow-in-tests settings only cover #[test] functions"
)]

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign};
use rand_chacha::ChaCha20Rng;
use rand_core::{Rng, SeedableRng};
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::{BoxedUint, PrivateKey};

const NAME: &str = "hostile";
/// Header bytes after the name: length field (4), flags and magic (12), bit length (4), exponent (4).
const HEADER: usize = 24;

fn sign_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/sign")
}

fn le(value: &BoxedUint, size: usize) -> Vec<u8> {
    let mut v = value.to_be_bytes_trimmed_vartime().to_vec();
    v.reverse();
    assert!(v.len() <= size);
    v.resize(size, 0);
    v
}

fn seeded_key(seed: u64, bits: u32) -> PrivateKey {
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    PrivateKey::new_with_exp_unchecked(
        &mut rng,
        usize::try_from(bits).unwrap(),
        BoxedUint::from(65537u64),
    )
    .unwrap()
}

/// The `.biprivatekey` bytes of an `rsa` key.
fn key_bytes(key: &PrivateKey, bits: u32) -> Vec<u8> {
    let size = usize::try_from(bits / 8).unwrap();
    let mut out = NAME.as_bytes().to_vec();
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

/// The component names in file order.
const PARTS: [&str; 7] = ["n", "p", "q", "dmp1", "dmq1", "iqmp", "d"];

/// Byte range of component `i` in a key file of `bits` bits.
fn part(bits: u32, i: usize) -> std::ops::Range<usize> {
    let full = usize::try_from(bits / 8).unwrap();
    let half = full / 2;
    let sizes = [full, half, half, half, half, half, full];
    let start = NAME.len() + 1 + HEADER + sizes[..i].iter().sum::<usize>();
    start..start + sizes[i]
}

fn read(bytes: &[u8]) -> Result<BIPrivateKey, std::io::Error> {
    BIPrivateKey::read(&mut Cursor::new(bytes))
}

fn written(key: &BIPrivateKey) -> Vec<u8> {
    let mut out = Vec::new();
    key.write(&mut out).unwrap();
    out
}

fn set(bytes: &mut [u8], range: std::ops::Range<usize>, value: u8) {
    bytes[range.clone()].fill(0);
    bytes[range.start] = value;
}

#[test]
fn valid_keys_round_trip() {
    for (seed, bits) in [(1, 1024), (2, 2048), (3, 512), (4, 288)] {
        let bytes = key_bytes(&seeded_key(seed, bits), bits);
        let key = read(&bytes).unwrap_or_else(|e| panic!("{bits} bit key: {e}"));
        assert_eq!(written(&key), bytes, "{bits} bit key");
        let mut public = Vec::new();
        key.to_public_key().unwrap().write(&mut public).unwrap();
        BIPublicKey::read(&mut Cursor::new(&public)).unwrap();
    }
    let armk = fs::read(sign_dir().join("armk.biprivatekey")).unwrap();
    assert_eq!(written(&read(&armk).unwrap()), armk);
    let bikey = fs::read(sign_dir().join("armk.bikey")).unwrap();
    let public = BIPublicKey::read(&mut Cursor::new(&bikey)).unwrap();
    let mut again = Vec::new();
    public.write(&mut again).unwrap();
    assert_eq!(again, bikey);
}

#[test]
fn private_key_with_wrong_n_is_rejected() {
    let bytes = key_bytes(&seeded_key(1, 1024), 1024);
    // one flipped bit in n
    let mut bad = bytes.clone();
    bad[part(1024, 0).start + 3] ^= 0x10;
    assert!(read(&bad).is_err(), "flipped bit in n");
    // the n of another key
    let other = key_bytes(&seeded_key(2, 1024), 1024);
    let mut bad = bytes.clone();
    bad[part(1024, 0)].copy_from_slice(&other[part(1024, 0)]);
    assert!(read(&bad).is_err(), "n of another key");
    // an even n
    let mut bad = bytes;
    bad[part(1024, 0).start] &= !1;
    assert!(read(&bad).is_err(), "even n");
}

#[test]
fn private_key_with_wrong_crt_values_is_rejected() {
    let bytes = key_bytes(&seeded_key(1, 1024), 1024);
    for i in 3..=5 {
        let mut bad = bytes.clone();
        bad[part(1024, i).start + 1] ^= 1;
        assert!(read(&bad).is_err(), "flipped bit in {}", PARTS[i]);
    }
    // values of another key
    let other = key_bytes(&seeded_key(2, 1024), 1024);
    for i in 1..=6 {
        let mut bad = bytes.clone();
        bad[part(1024, i)].copy_from_slice(&other[part(1024, i)]);
        assert!(read(&bad).is_err(), "{} of another key", PARTS[i]);
    }
    // d that does not match e
    let mut bad = bytes;
    bad[part(1024, 6).start] ^= 2;
    assert!(read(&bad).is_err(), "wrong d");
}

#[test]
fn zero_and_one_components_are_rejected() {
    for bits in [1024, 288] {
        let bytes = key_bytes(&seeded_key(1, bits), bits);
        for (i, name) in PARTS.iter().enumerate() {
            for value in [0u8, 1] {
                let mut bad = bytes.clone();
                set(&mut bad, part(bits, i), value);
                assert!(read(&bad).is_err(), "{name} = {value} ({bits} bit)");
            }
        }
        // everything zero, everything one
        for value in [0u8, 1] {
            let mut bad = bytes.clone();
            for i in 0..PARTS.len() {
                set(&mut bad, part(bits, i), value);
            }
            assert!(read(&bad).is_err(), "all components = {value}");
        }
    }
}

#[test]
fn bad_lengths_are_errors() {
    let bytes = key_bytes(&seeded_key(1, 1024), 1024);
    let name_len = NAME.len() + 1;
    // header length field
    let mut bad = bytes.clone();
    bad[name_len] ^= 1;
    assert!(read(&bad).is_err());
    // bit length that no longer matches the stored sizes
    for length in [0u32, 1, 287, 2048, u32::MAX, 1 << 21] {
        let mut bad = bytes.clone();
        bad[name_len + 16..name_len + 20].copy_from_slice(&length.to_le_bytes());
        assert!(read(&bad).is_err(), "bit length {length}");
    }
    // truncated and empty files
    for cut in [
        0,
        1,
        name_len,
        name_len + 24,
        bytes.len() / 2,
        bytes.len() - 1,
    ] {
        assert!(read(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    // exponent 0, 1, an even one, and one that is not the key's
    for exponent in [0u32, 1, 2, 4, 3, u32::MAX] {
        let mut bad = bytes.clone();
        bad[name_len + 20..name_len + 24].copy_from_slice(&exponent.to_le_bytes());
        assert!(read(&bad).is_err(), "exponent {exponent}");
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        write!(out, "{b:02x}").unwrap();
        out
    })
}

fn decimal(be: &[u8]) -> String {
    let mut digits = Vec::new();
    let mut n = be.to_vec();
    while n.iter().any(|&b| b != 0) {
        let mut rem = 0u32;
        for b in &mut n {
            let cur = rem * 256 + u32::from(*b);
            *b = u8::try_from(cur / 10).unwrap();
            rem = cur % 10;
        }
        digits.push(char::from_digit(rem, 10).unwrap());
    }
    digits.iter().rev().collect()
}

#[test]
fn key_errors_contain_no_key_material() {
    let key = seeded_key(1, 1024);
    let bytes = key_bytes(&key, 1024);
    let secrets: Vec<Vec<u8>> = [
        key.d().to_be_bytes_trimmed_vartime().to_vec(),
        key.primes()[0].to_be_bytes_trimmed_vartime().to_vec(),
        key.primes()[1].to_be_bytes_trimmed_vartime().to_vec(),
    ]
    .into();
    // wrong n, wrong crt, wrong d, wrong p, zero q, wrong exponent
    let mut variants = Vec::new();
    for (i, at) in [(0, 3), (3, 1), (4, 1), (5, 1), (6, 0), (1, 5), (2, 7)] {
        let mut bad = bytes.clone();
        bad[part(1024, i).start + at] ^= 1;
        variants.push(bad);
    }
    let mut bad = bytes.clone();
    set(&mut bad, part(1024, 2), 0);
    variants.push(bad);
    let mut bad = bytes.clone();
    bad[NAME.len() + 1 + 20] = 2;
    variants.push(bad);
    for bad in variants {
        let err = read(&bad).unwrap_err();
        let shown = format!("{err} | {err:?}").to_ascii_lowercase();
        for secret in &secrets {
            let mut le_bytes = secret.clone();
            le_bytes.reverse();
            let be_hex: String = hex(secret);
            let le_hex: String = hex(&le_bytes);
            let dec = decimal(secret);
            for form in [&be_hex, &le_hex, &dec] {
                // any 16-character window of a secret in its text form would be a leak
                assert!(
                    !form
                        .as_bytes()
                        .windows(16)
                        .any(|w| shown.as_bytes().windows(16).any(|s| s == w)),
                    "error text leaks key material: {shown}"
                );
            }
        }
    }
    // the Debug of a good key shows no components either
    let good = read(&bytes).unwrap();
    let shown = format!("{good:?}");
    assert!(!shown.contains(&decimal(&key.d().to_be_bytes_trimmed_vartime())));
}

#[test]
fn mutated_key_files_never_panic_and_accepted_ones_are_canonical() {
    let bytes = key_bytes(&seeded_key(1, 1024), 1024);
    let mut rng = ChaCha20Rng::seed_from_u64(99);
    for round in 0..400 {
        let mut bad = bytes.clone();
        for _ in 0..=(round % 3) {
            // the name and the fixed header words (flags, magic) are not validated and not canonical
            let skip = NAME.len() + 1 + HEADER;
            let at = skip + usize::try_from(rng.next_u32()).unwrap() % (bad.len() - skip);
            bad[at] = rng.next_u32().to_le_bytes()[0];
        }
        if let Ok(key) = read(&bad) {
            assert_eq!(written(&key), bad, "an accepted key file must be canonical");
        }
    }
    // public keys too
    let public = {
        let mut out = Vec::new();
        read(&bytes)
            .unwrap()
            .to_public_key()
            .unwrap()
            .write(&mut out)
            .unwrap();
        out
    };
    for round in 0..400 {
        let mut bad = public.clone();
        for _ in 0..=(round % 3) {
            let at = usize::try_from(rng.next_u32()).unwrap() % bad.len();
            bad[at] = rng.next_u32().to_le_bytes()[0];
        }
        let _ = BIPublicKey::read(&mut Cursor::new(&bad)); // Ok or Err, but no panic
    }
}

#[test]
fn public_key_with_unusable_values_is_rejected() {
    let bytes = key_bytes(&seeded_key(1, 1024), 1024);
    let mut public = Vec::new();
    read(&bytes)
        .unwrap()
        .to_public_key()
        .unwrap()
        .write(&mut public)
        .unwrap();
    let name_len = NAME.len() + 1;
    let n = name_len + 24..public.len();
    for value in [0u8, 1] {
        let mut bad = public.clone();
        set(&mut bad, n.clone(), value);
        assert!(
            BIPublicKey::read(&mut Cursor::new(&bad)).is_err(),
            "n = {value}"
        );
    }
    let mut bad = public.clone();
    bad[n.start] &= !1;
    assert!(BIPublicKey::read(&mut Cursor::new(&bad)).is_err(), "even n");
    for exponent in [0u32, 1, 2] {
        let mut bad = public.clone();
        bad[name_len + 20..name_len + 24].copy_from_slice(&exponent.to_le_bytes());
        assert!(
            BIPublicKey::read(&mut Cursor::new(&bad)).is_err(),
            "e = {exponent}"
        );
    }
}

/// A private key file with the header length `length`, holding the (zero-padded) components of `bytes`
/// (a 1024-bit key file named `NAME`).
fn reframe_private(bytes: &[u8], length: u32) -> Vec<u8> {
    let mut out = NAME.as_bytes().to_vec();
    out.push(0);
    out.extend((length / 16 * 9 + 20).to_le_bytes());
    out.extend(b"\x07\x02\x00\x00\x00\x24\x00\x00RSA2");
    out.extend(length.to_le_bytes());
    out.extend(65537u32.to_le_bytes());
    let full = usize::try_from(length / 8).unwrap();
    let half = usize::try_from(length / 16).unwrap();
    for i in 0..PARTS.len() {
        let mut value = bytes[part(1024, i)].to_vec();
        value.resize(if i == 0 || i == 6 { full } else { half }, 0);
        out.extend(value);
    }
    out
}

fn reframe_public(bytes: &[u8], length: u32) -> Vec<u8> {
    let mut out = NAME.as_bytes().to_vec();
    out.push(0);
    out.extend((length / 8 + 20).to_le_bytes());
    out.extend(b"\x06\x02\x00\x00\x00\x24\x00\x00RSA1");
    out.extend(length.to_le_bytes());
    out.extend(65537u32.to_le_bytes());
    let mut n = bytes[part(1024, 0)].to_vec();
    n.resize(usize::try_from(length / 8).unwrap(), 0);
    out.extend(n);
    out
}

#[test]
fn a_modulus_that_does_not_fill_the_header_length_is_rejected() {
    let bytes = key_bytes(&seeded_key(1, 1024), 1024);
    // sanity: the frame at the true length reads
    read(&reframe_private(&bytes, 1024)).unwrap();
    BIPublicKey::read(&mut Cursor::new(reframe_public(&bytes, 1024))).unwrap();
    // a small key in a big frame, a length that is no multiple of 16, a huge frame
    for length in [2048u32, 1032, 1040, 1 << 20] {
        assert!(
            read(&reframe_private(&bytes, length)).is_err(),
            "private {length}"
        );
    }
    for length in [2048u32, 1032, 1025, 1 << 20] {
        assert!(
            BIPublicKey::read(&mut Cursor::new(reframe_public(&bytes, length))).is_err(),
            "public {length}"
        );
    }
}

#[test]
fn armk_in_a_bigger_frame_is_rejected() {
    let armk = fs::read(sign_dir().join("armk.biprivatekey")).unwrap();
    let name_len = armk.iter().position(|&b| b == 0).unwrap() + 1;
    let bits = 1024;
    let size = bits / 8;
    // zero-pad every component into a 2048-bit frame
    let mut out = armk[..name_len].to_vec();
    out.extend((2048u32 / 16 * 9 + 20).to_le_bytes());
    out.extend(&armk[name_len + 4..name_len + 16]);
    out.extend(2048u32.to_le_bytes());
    out.extend(&armk[name_len + 20..name_len + 24]);
    let mut at = name_len + 24;
    for (len, new_len) in [
        (size, 256),
        (size / 2, 128),
        (size / 2, 128),
        (size / 2, 128),
        (size / 2, 128),
        (size / 2, 128),
        (size, 256),
    ] {
        out.extend(&armk[at..at + len]);
        out.extend(vec![0; new_len - len]);
        at += len;
    }
    assert!(read(&out).is_err());
}

/// A `.bikey` frame of `length` bits holding `n` (an odd number of exactly `length` bits: all ones).
fn public_frame(length: u32) -> Vec<u8> {
    let mut out = NAME.as_bytes().to_vec();
    out.push(0);
    out.extend((length / 8 + 20).to_le_bytes());
    out.extend(b"\x06\x02\x00\x00\x00\x24\x00\x00RSA1");
    out.extend(length.to_le_bytes());
    out.extend(65537u32.to_le_bytes());
    out.extend(vec![0xff; usize::try_from(length / 8).unwrap()]);
    out
}

/// A `.bisign` frame of `length` bits with an all-ones modulus and zero signatures.
fn bisign_frame(length: u32) -> Vec<u8> {
    let size = usize::try_from(length / 8).unwrap();
    let mut out = NAME.as_bytes().to_vec();
    out.push(0);
    out.extend((length / 8 + 20).to_le_bytes());
    out.extend(b"\x06\x02\x00\x00\x00\x24\x00\x00RSA1");
    out.extend(length.to_le_bytes());
    out.extend(65537u32.to_le_bytes());
    out.extend(vec![0xff; size]);
    out.extend(u32::try_from(size).unwrap().to_le_bytes());
    out.extend(vec![0; size]);
    out.extend(2u32.to_le_bytes());
    out.extend(u32::try_from(size).unwrap().to_le_bytes());
    out.extend(vec![0; size]);
    out.extend(u32::try_from(size).unwrap().to_le_bytes());
    out.extend(vec![0; size]);
    out
}

#[test]
fn keys_and_signatures_are_capped_at_16384_bits() {
    // the largest size reads, one step more is an error that names the limit
    BIPublicKey::read(&mut Cursor::new(public_frame(16384))).unwrap();
    BISign::read(&mut Cursor::new(bisign_frame(16384))).unwrap();
    for length in [16400u32, 1 << 17, 1 << 20] {
        let err = BIPublicKey::read(&mut Cursor::new(public_frame(length)))
            .err()
            .unwrap();
        assert!(err.to_string().contains("16384"), "{err}");
        let err = BISign::read(&mut Cursor::new(bisign_frame(length)))
            .err()
            .unwrap();
        assert!(err.to_string().contains("16384"), "{err}");
        let bytes = key_bytes(&seeded_key(1, 1024), 1024);
        let err = read(&reframe_private(&bytes, length)).unwrap_err();
        assert!(err.to_string().contains("16384"), "{err}");
    }
}
