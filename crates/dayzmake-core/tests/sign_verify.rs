//! Edge cases of signing and verification: hostile signature values and keys outside `rsa`'s default sizes.
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
use num_bigint::BigUint;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::{BoxedUint, PrivateKey};

fn sign_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/sign")
}

fn testsig() -> (PBO, Vec<u8>, BIPublicKey) {
    let pbo = PBO::read(&mut Cursor::new(
        fs::read(sign_dir().join("testsig.pbo")).unwrap(),
    ))
    .unwrap();
    let sig = fs::read(sign_dir().join("testsig.pbo.armk.bisign")).unwrap();
    let public =
        BIPublicKey::read(&mut fs::File::open(sign_dir().join("armk.bikey")).unwrap()).unwrap();
    (pbo, sig, public)
}

#[test]
fn signature_value_not_below_n_is_an_error() {
    let (pbo, sig, public) = testsig();
    let name_len = sig.iter().position(|&b| b == 0).unwrap() + 1;
    let n_at = name_len + 24;
    let sig1_at = n_at + 128 + 4;
    let sig2_at = sig1_at + 128 + 8;
    let n = sig[n_at..n_at + 128].to_vec();
    for (what, value) in [("n", n), ("all ones", vec![0xff; 128])] {
        for at in [sig1_at, sig2_at] {
            let mut bad = sig.clone();
            bad[at..at + 128].copy_from_slice(&value);
            let bad = BISign::read(&mut Cursor::new(&bad)).unwrap();
            let err = public.verify(&pbo, &bad).unwrap_err();
            assert!(
                err.to_string().contains("not smaller than the modulus"),
                "{what}: {err}"
            );
        }
    }
}

/// A seeded key of `bits` bits as `.biprivatekey` bytes.
fn key_bytes(seed: u64, bits: u32) -> Vec<u8> {
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let key = PrivateKey::new_with_exp_unchecked(
        &mut rng,
        usize::try_from(bits).unwrap(),
        BoxedUint::from(65537u64),
    )
    .unwrap();
    let size = usize::try_from(bits / 8).unwrap();
    let le = |v: &BoxedUint, size: usize| {
        let mut v = v.to_be_bytes_trimmed_vartime().to_vec();
        v.reverse();
        v.resize(size, 0);
        v
    };
    let mut out = b"small\0".to_vec();
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
fn small_key_signatures_verify() {
    let pbo = PBO::read(&mut Cursor::new(
        fs::read(sign_dir().join("testsig.pbo")).unwrap(),
    ))
    .unwrap();
    for (seed, bits) in [(12u64, 384u32), (13, 512), (14, 768), (15, 1024)] {
        let bytes = key_bytes(seed, bits);
        let private = BIPrivateKey::read(&mut Cursor::new(&bytes)).unwrap();
        let public = private.to_public_key().unwrap();
        for version in [BISignVersion::V2, BISignVersion::V3] {
            let signed = private.sign(&pbo, version).unwrap();
            let mut written = Vec::new();
            signed.write(&mut written).unwrap();
            let read = BISign::read(&mut Cursor::new(&written)).unwrap();
            public.verify(&pbo, &read).unwrap_or_else(|e| {
                panic!("{bits} bit {version:?}: {e}");
            });
            // independent check of the first signature value: sig^e mod n is the padded hash, which starts
            // with 00 01 ff ...
            let name_len = written.iter().position(|&b| b == 0).unwrap() + 1;
            let size = usize::try_from(bits / 8).unwrap();
            let n_at = name_len + 24;
            let be = |from: usize| BigUint::from_bytes_le(&written[from..from + size]);
            let (n, s1) = (be(n_at), be(n_at + size + 4));
            let back = s1.modpow(&BigUint::from(65537u32), &n);
            // hash 1 is the PBO's SHA-1: it ends the padded block
            let v = back.to_bytes_be();
            assert_eq!(v.len(), size - 1, "{bits} bit: 00 is trimmed");
            assert_eq!(v[0], 1);
            assert_eq!(&v[v.len() - 20..], pbo.checksum().unwrap());
        }
    }
}

#[test]
fn sign_refuses_keys_below_384_bits() {
    let pbo = PBO::read(&mut Cursor::new(
        fs::read(sign_dir().join("testsig.pbo")).unwrap(),
    ))
    .unwrap();
    // 288 bits is the smallest key that can be read; 368 is the largest below the limit whose length is a
    // multiple of 16 (376 cannot be read at all)
    for (seed, bits) in [(11u64, 288u32), (16, 368)] {
        let private = BIPrivateKey::read(&mut Cursor::new(key_bytes(seed, bits))).unwrap();
        for version in [BISignVersion::V2, BISignVersion::V3] {
            let err = private.sign(&pbo, version).unwrap_err();
            let text = err.to_string();
            assert!(
                text.contains("384-bit") && text.contains(&bits.to_string()),
                "{bits}: {text}"
            );
        }
    }
}

#[test]
fn a_signature_from_another_key_says_so() {
    let (pbo, sig, _) = testsig();
    let signature = BISign::read(&mut Cursor::new(&sig)).unwrap();
    // a shorter key, and another key of the same length
    for (seed, bits) in [(21u64, 512u32), (22, 1024)] {
        let other = BIPrivateKey::read(&mut Cursor::new(key_bytes(seed, bits)))
            .unwrap()
            .to_public_key()
            .unwrap();
        let err = other.verify(&pbo, &signature).unwrap_err().to_string();
        assert!(err.contains("made with a different key"), "{bits}: {err}");
        assert!(err.contains("armk") && err.contains("small"), "{err}");
        assert!(!err.contains("modulus"), "{err}");
    }
}

#[test]
fn a_bad_bisign_header_names_the_signature_file() {
    let (_, mut sig, _) = testsig();
    let name_len = sig.iter().position(|&b| b == 0).unwrap() + 1;
    sig[name_len + 12] = b'X';
    let err = BISign::read(&mut Cursor::new(&sig))
        .unwrap_err()
        .to_string();
    assert!(err.contains("Invalid signature file"), "{err}");
}

/// Little-endian bytes of `value`, zero-padded to `size`.
fn le_bn(value: &BigUint, size: usize) -> Vec<u8> {
    let mut v = value.to_bytes_le();
    v.resize(size, 0);
    v
}

fn bn_le(bytes: &[u8]) -> BigUint {
    BigUint::from_bytes_le(bytes)
}

#[test]
fn signatures_of_keys_below_384_bits_still_verify() {
    let pbo = PBO::read(&mut Cursor::new(
        fs::read(sign_dir().join("testsig.pbo")).unwrap(),
    ))
    .unwrap();
    // The three SHA-1 hashes of the PBO: the last 20 bytes of the padded blocks that a 1024-bit key signs.
    let big = BIPrivateKey::read(&mut Cursor::new(key_bytes(15, 1024))).unwrap();
    let signed = big.sign(&pbo, BISignVersion::V2).unwrap();
    let mut bytes = Vec::new();
    signed.write(&mut bytes).unwrap();
    let values = {
        let name_len = bytes.iter().position(|&b| b == 0).unwrap() + 1;
        let n_at = name_len + 24;
        [
            n_at + 128 + 4,
            n_at + 128 + 4 + 128 + 8,
            n_at + 128 + 4 + 2 * 128 + 12,
        ]
    };
    let n_big = bn_le(&bytes[values[0] - 132..values[0] - 4]);
    let hashes: Vec<Vec<u8>> = values
        .iter()
        .map(|&at| {
            let block = bn_le(&bytes[at..at + 128])
                .modpow(&BigUint::from(65537u32), &n_big)
                .to_bytes_be();
            block[block.len() - 20..].to_vec()
        })
        .collect();

    for (seed, bits) in [(31u64, 288u32), (32, 304), (33, 368)] {
        let key = key_bytes(seed, bits);
        let size = usize::try_from(bits / 8).unwrap();
        let name_len = key.iter().position(|&b| b == 0).unwrap() + 1;
        let n_at = name_len + 24;
        let n_le = &key[n_at..n_at + size];
        let d_le = &key[n_at + size + 5 * size / 2..n_at + 2 * size + 5 * size / 2];
        let (n, d) = (bn_le(n_le), bn_le(d_le));

        // BI's padding as `pad_hash` writes it: 00 01, then ff up to `size - 36` bytes (the resize cuts the
        // block when that is below 2), then 00 30 21 ... and the hash. 288 bits: 00 30 21 ...; 304: 00 01 00 30 ...
        let pad = |hash: &[u8]| {
            let mut v = vec![0u8, 1];
            v.resize(size - 36, 255);
            v.extend(b"\x00\x30\x21\x30\x09\x06\x05\x2b\x0e\x03\x02\x1a\x05\x00\x04\x14");
            v.extend(hash);
            v
        };
        let mut sigs = Vec::new();
        for hash in &hashes {
            let block = pad(hash);
            assert_eq!(block.len(), size);
            let sig = BigUint::from_bytes_be(&block).modpow(&d, &n);
            sigs.push(le_bn(&sig, size));
        }

        let mut bisign = b"small\0".to_vec();
        bisign.extend((bits / 8 + 20).to_le_bytes());
        bisign.extend(b"\x06\x02\x00\x00\x00\x24\x00\x00RSA1");
        bisign.extend(bits.to_le_bytes());
        bisign.extend(65537u32.to_le_bytes());
        bisign.extend(n_le);
        let size32 = u32::try_from(size).unwrap().to_le_bytes();
        bisign.extend(size32);
        bisign.extend(&sigs[0]);
        bisign.extend(2u32.to_le_bytes());
        bisign.extend(size32);
        bisign.extend(&sigs[1]);
        bisign.extend(size32);
        bisign.extend(&sigs[2]);

        let public = BIPrivateKey::read(&mut Cursor::new(&key))
            .unwrap()
            .to_public_key()
            .unwrap();
        let read = BISign::read(&mut Cursor::new(&bisign)).unwrap();
        public
            .verify(&pbo, &read)
            .unwrap_or_else(|e| panic!("{bits} bits: {e}"));

        let mut bad = bisign.clone();
        let sig1_at = name_len + 24 + size + 4;
        bad[sig1_at] ^= 1;
        let bad = BISign::read(&mut Cursor::new(&bad)).unwrap();
        assert!(
            public.verify(&pbo, &bad).is_err(),
            "{bits} bits: flipped bit"
        );
    }
}
