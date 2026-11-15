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
use openssl::bn::{BigNum, BigNumContext};
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
    for (seed, bits) in [(11u64, 288u32), (12, 320), (13, 512), (14, 768), (15, 1024)] {
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
            let be = |from: usize| -> BigNum {
                let mut v = written[from..from + size].to_vec();
                v.reverse();
                BigNum::from_slice(&v).unwrap()
            };
            let (n, s1) = (be(n_at), be(n_at + size + 4));
            let mut ctx = BigNumContext::new().unwrap();
            let mut back = BigNum::new().unwrap();
            back.mod_exp(&s1, &BigNum::from_u32(65537).unwrap(), &n, &mut ctx)
                .unwrap();
            // hash 1 is the PBO's SHA-1: it ends the padded block
            let v = back.to_vec();
            assert_eq!(v.len(), size - 1, "{bits} bit: 00 is trimmed");
            // (a 288 bit key has no room for the 01 ff ... prefix: BI's padding is cut to 00 30 21 ...)
            assert_eq!(v[0], if bits == 288 { 0x30 } else { 1 });
            assert_eq!(&v[v.len() - 20..], pbo.checksum().unwrap());
        }
    }
}
