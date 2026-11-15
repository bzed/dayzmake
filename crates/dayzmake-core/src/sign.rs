//! Functions for creating and working with BI keys and signatures

use std::fmt::Write as _;
use std::io::{Error, Read, Write};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use getrandom::SysRng;
use openssl::bn::BigNumRef;
use openssl::rsa::Rsa;
use rsa::hazmat::{rsa_decrypt_and_check, rsa_encrypt};
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::{BoxedUint, PrivateKey, PublicKey};
use sha1::{Digest, Sha1};
use zeroize::Zeroizing;

use crate::error::ErrorExt;
use crate::io::{ReadExt, WriteExt};
use crate::pbo::PBO;

/// BI private key (.biprivatekey)
pub struct BIPrivateKey {
    name: String,
    length: u32,
    exponent: u32,
    /// The validated key: `n`, `e`, `d`, `p`, `q` and the CRT values, checked against each other on creation.
    key: PrivateKey,
}

/// BI public key (.bikey)
#[derive(Debug)]
pub struct BIPublicKey {
    name: String,
    length: u32,
    exponent: u32,
    key: PublicKey,
}

/// BI signature version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BISignVersion {
    /// Version 2
    V2,
    /// Version 3
    V3,
}

/// BI signature (.bisign)
#[derive(Debug)]
pub struct BISign {
    version: BISignVersion,
    name: String,
    length: u32,
    exponent: u32,
    n: BoxedUint,
    sig1: BoxedUint,
    sig2: BoxedUint,
    sig3: BoxedUint,
}

impl std::fmt::Debug for BIPrivateKey {
    /// Prints the name and length only, never any key material.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BIPrivateKey")
            .field("name", &self.name)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

/// Writes `value` little-endian, zero-padded to `size` bytes.
///
/// # Errors
///
/// Returns an error if the value does not fit into `size` bytes, or on I/O failure.
fn write_uint<O: Write>(output: &mut O, value: &BoxedUint, size: usize) -> Result<(), Error> {
    // Both buffers can hold secret key bytes: they are wiped on drop, and the output buffer is allocated at its
    // final size so that no reallocation leaves a copy behind.
    let trimmed = Zeroizing::new(value.to_be_bytes_trimmed_vartime().into_vec());
    if trimmed.len() > size {
        return Err(error!("Key component does not fit into {size} bytes."));
    }
    let mut vec = Zeroizing::new(vec![0u8; size]);
    for (dst, src) in vec.iter_mut().zip(trimmed.iter().rev()) {
        *dst = *src;
    }

    output.write_all(&vec)
}

/// Reads a key component of `size` little-endian bytes.
fn read_uint<I: Read>(input: &mut I, size: usize) -> Result<BoxedUint, Error> {
    // The buffer can hold a secret key component: wiped on drop, also on the error paths.
    let mut buffer = Zeroizing::new(vec![0; size]);
    input.read_exact(&mut buffer)?;
    uint_from_le(&buffer)
}

/// Converts little-endian bytes into an integer wide enough for them.
fn uint_from_le(bytes: &[u8]) -> Result<BoxedUint, Error> {
    let bits = u32::try_from(bytes.len())
        .ok()
        .and_then(|len| len.checked_mul(8))
        .ok_or_else(|| error!("Key component is too large."))?;
    BoxedUint::from_le_slice(bytes, bits).map_err(|_| error!("Invalid key data."))
}

/// Whether two integers have the same value, whatever their storage widths.
///
/// Variable time: used only to check a key file against itself, where the file's owner knows every value.
fn same_value(a: &BoxedUint, b: &BoxedUint) -> bool {
    let a = Zeroizing::new(a.to_be_bytes_trimmed_vartime().into_vec());
    let b = Zeroizing::new(b.to_be_bytes_trimmed_vartime().into_vec());
    *a == *b
}

/// Whether `value` is zero or one: no key component can be.
fn is_trivial(value: &BoxedUint) -> bool {
    value.bit_width_vartime() <= 1
}

/// Maps an `rsa` error onto a message that carries no key material.
///
/// `rsa` errors name the kind of defect only; they never contain a value.
#[expect(
    clippy::needless_pass_by_value,
    reason = "used as a map_err closure; the error is formatted, not stored"
)]
fn key_err(err: rsa::Error) -> Error {
    error!("{err}")
}

/// A PBO entry's lowercased raw name bytes paired with its contents.
type NameFilePair<'a> = (Vec<u8>, &'a [u8]);

/// Smallest key length accepted, in bits.
const MIN_KEY_BITS: u32 = 288;
/// Smallest key length that `sign` and `generate` accept, in bits: the minimum of the Windows `CryptoAPI` RSA
/// signature key (BI's tools and the game cannot use smaller ones). PKCS#1 v1.5 padding of a `SHA-1` hash needs
/// 368 bits at least. Smaller keys can still be read, and signatures made with them verified.
const MIN_SIGN_BITS: u32 = 384;
/// Largest key length accepted, in bits: the largest RSA key the Windows `CryptoAPI` (which BI's key format comes
/// from) handles. Bigger keys would only make hostile files expensive to read.
const MAX_KEY_BITS: u32 = 16384;

/// bType, bVersion and reserved of a private key blob (`PRIVATEKEYBLOB`, version 2), as a little-endian word.
const PRIVATE_BLOB: u32 = 0x0207;
/// The same for a public key blob (`PUBLICKEYBLOB`).
const PUBLIC_BLOB: u32 = 0x0206;
/// `aiKeyAlg` of every key: `CALG_RSA_SIGN`.
const KEY_ALG: u32 = 0x2400;
/// Magic of a private key blob.
const PRIVATE_MAGIC: [u8; 4] = *b"RSA2";
/// Magic of a public key blob.
const PUBLIC_MAGIC: [u8; 4] = *b"RSA1";

/// Reads and checks the 12 bytes of a `CryptoAPI` key blob header that follow the blob length: type and
/// version, key algorithm and magic.
///
/// # Errors
///
/// Returns an error on I/O failure or if the header is not the one of a `kind` key.
fn check_blob_header<I: Read>(
    input: &mut I,
    blob: u32,
    magic: [u8; 4],
    kind: &str,
) -> Result<(), Error> {
    let found_blob = input.read_u32::<LittleEndian>()?;
    let found_alg = input.read_u32::<LittleEndian>()?;
    let mut found_magic = [0; 4];
    input.read_exact(&mut found_magic)?;
    if found_blob != blob || found_alg != KEY_ALG || found_magic != magic {
        return Err(error!(
            "Invalid {kind} key file (bad header: not a {kind} key blob)."
        ));
    }
    Ok(())
}

/// Validates a key length read from a key or signature file.
///
/// # Errors
///
/// Returns an error if the length is not a sane RSA key length.
fn check_key_length(length: u32) -> Result<(), Error> {
    if !(MIN_KEY_BITS..=MAX_KEY_BITS).contains(&length) {
        return Err(error!(
            "Invalid key length: {length} (supported: {MIN_KEY_BITS} to {MAX_KEY_BITS} bits)."
        ));
    }
    Ok(())
}

/// Refuses key lengths that cannot sign.
///
/// # Errors
///
/// Returns an error if `length` is below the 384-bit `CryptoAPI` minimum.
fn check_sign_length(length: u32) -> Result<(), Error> {
    if length < MIN_SIGN_BITS {
        return Err(error!(
            "A {length}-bit key is smaller than the {MIN_SIGN_BITS}-bit CryptoAPI minimum and cannot sign."
        ));
    }
    Ok(())
}

/// The default signature file of `pbo_path`: `<pbo>.<key name>.bisign`.
///
/// The key name is read from the (untrusted) key file, so it must be a plain file name part:
/// `Path::set_extension` panics on a path separator, and a name must not steer the file elsewhere.
///
/// # Errors
///
/// Returns an error if the key name is empty or contains a path separator, `:` or a control
/// character.
pub fn signature_path(pbo_path: &Path, key_name: &str) -> Result<PathBuf, Error> {
    if key_name.is_empty()
        || key_name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
    {
        return Err(error!(
            "The key name {key_name:?} cannot be used in a file name; pass the signature path explicitly."
        ));
    }
    let mut path = pbo_path.to_path_buf();
    path.set_extension(format!("pbo.{key_name}.bisign"));
    Ok(path)
}

/// SHA-1 of the lowercased names of the non-empty entries, in byte order.
fn namehash(pbo: &PBO) -> [u8; 20] {
    let mut files_sorted: Vec<NameFilePair<'_>> = pbo
        .entries()
        .map(|(a, b)| (pbo.name_bytes(a).to_ascii_lowercase(), b))
        .collect();
    files_sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut h = Sha1::new();

    for (name, data) in &files_sorted {
        if data.is_empty() {
            continue;
        }

        h.update(name);
    }

    h.finalize().into()
}

/// SHA-1 of the entries the signature `version` covers.
fn filehash(pbo: &PBO, version: BISignVersion) -> [u8; 20] {
    let mut h = Sha1::new();
    let mut nothing = true;

    for (name, data) in pbo.entries() {
        // file names are never empty, so there is always at least one (possibly empty) part
        let ext = name.split('.').next_back().unwrap_or("");

        match version {
            BISignVersion::V2 => {
                if ext == "paa"
                    || ext == "jpg"
                    || ext == "p3d"
                    || ext == "tga"
                    || ext == "rvmat"
                    || ext == "lip"
                    || ext == "ogg"
                    || ext == "wss"
                    || ext == "png"
                    || ext == "rtm"
                    || ext == "pac"
                    || ext == "fxy"
                    || ext == "wrp"
                {
                    continue;
                }
            }
            BISignVersion::V3 => {
                // "c": DayZ's DSSignFile also hashes Enforce Script files (verified against DSSignFile.exe
                // from DayZ Tools); without it a DayZ server rejects the signature of a script mod
                if ext != "sqf"
                    && ext != "c"
                    && ext != "inc"
                    && ext != "bikb"
                    && ext != "ext"
                    && ext != "fsm"
                    && ext != "sqm"
                    && ext != "hpp"
                    && ext != "cfg"
                    && ext != "sqs"
                    && ext != "h"
                {
                    continue;
                }
            }
        }

        h.update(data);
        nothing = false;
    }

    match version {
        BISignVersion::V2 => {
            if nothing {
                h.update(b"nothing");
            }
        }
        BISignVersion::V3 => {
            if nothing {
                h.update(b"gnihton");
            }
        }
    }

    h.finalize().into()
}

/// The three padded hashes that BI signs, big-endian, `length / 8` bytes each.
fn generate_hashes(pbo: &PBO, version: BISignVersion, length: u32) -> Result<[Vec<u8>; 3], Error> {
    // a PBO that was read keeps its stored trailer (a foreign one may differ from what we would
    // write); one that was not read gets the trailer `PBO::write` would produce
    let checksum = pbo.trailer_checksum()?;
    let hash1 = &checksum[..];

    let namehash = namehash(pbo);
    // the prefix part both of the following hashes end with
    let mut suffix = Vec::new();
    if let Some(prefix) = pbo.prefix() {
        suffix.extend(pbo.extension_bytes(prefix));
        if !prefix.ends_with('\\') {
            suffix.push(b'\\');
        }
    }

    let mut h = Sha1::new();
    h.update(hash1);
    h.update(namehash);
    h.update(&suffix);
    let hash2 = h.finalize();

    let mut h = Sha1::new();
    h.update(filehash(pbo, version));
    h.update(namehash);
    h.update(&suffix);
    let hash3 = h.finalize();

    let size = usize::try_from(length / 8).map_err(|_| error!("Invalid key length: {length}."))?;
    Ok([
        pad_hash(hash1, size)?,
        pad_hash(&hash2, size)?,
        pad_hash(&hash3, size)?,
    ])
}

/// The PKCS#1 v1.5 (`SHA-1`) padded block of `hash`, `size` bytes big-endian.
fn pad_hash(hash: &[u8], size: usize) -> Result<Vec<u8>, Error> {
    if size < 36 {
        return Err(error!("Key length too small: {size} bytes."));
    }
    let mut vec: Vec<u8> = Vec::new();

    vec.push(0);
    vec.push(1);
    vec.resize(size - 36, 255);
    vec.extend(b"\x00\x30\x21\x30\x09\x06\x05\x2b");
    vec.extend(b"\x0e\x03\x02\x1a\x05\x00\x04\x14");
    vec.extend(hash);

    Ok(vec)
}

/// `bytes` without its leading zero bytes.
fn trim_zeros(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    bytes.get(start..).unwrap_or_default()
}

/// Lowercase hex of a number (`0` for zero).
fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0".to_string();
    }
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // writing into a String cannot fail
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The two numbers as hex, reduced to the hashes if their padding is the same.
fn display_hashes(signed: &[u8], real: &[u8]) -> (String, String) {
    let hex_signed = hex(signed);
    let hex_real = hex(real);

    if hex_signed.len() != hex_real.len() || hex_signed.len() <= 40 {
        return (hex_signed, hex_real);
    }

    let (signed_padding, signed_hash) = hex_signed.split_at(hex_signed.len() - 40);
    let (real_padding, real_hash) = hex_real.split_at(hex_real.len() - 40);

    if signed_padding == real_padding {
        (signed_hash.to_string(), real_hash.to_string())
    } else {
        (hex_signed, hex_real)
    }
}

impl BIPrivateKey {
    /// The key name, as stored in the key file.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Reads a private key from the given input.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not a valid private key.
    pub fn read<I: Read>(input: &mut I) -> Result<BIPrivateKey, Error> {
        let name = input.read_cstring()?;
        let temp = input.read_u32::<LittleEndian>()?;
        check_blob_header(input, PRIVATE_BLOB, PRIVATE_MAGIC, "private")?;
        let length = input.read_u32::<LittleEndian>()?;
        let exponent = input.read_u32::<LittleEndian>()?;

        check_key_length(length)?;
        if temp != length / 16 * 9 + 20 {
            return Err(error!("Invalid private key file (bad length field)."));
        }
        if length % 16 != 0 {
            return Err(error!(
                "Invalid private key file (the length {length} is not a multiple of 16)."
            ));
        }

        let full = (length / 8) as usize;
        let half = (length / 16) as usize;
        let n = read_uint(input, full)?;
        let p = read_uint(input, half)?;
        let q = read_uint(input, half)?;
        let exp_p = read_uint(input, half)?;
        let exp_q = read_uint(input, half)?;
        let iqmp = read_uint(input, half)?;
        let d = read_uint(input, full)?;

        // `rsa` would reject most of these, but some values would reach its arithmetic first (a modulus of
        // one, a prime of zero), so every component is checked here.
        for value in [&n, &p, &q, &exp_p, &exp_q, &iqmp, &d] {
            if is_trivial(value) {
                return Err(error!("Invalid private key (a component is zero or one)."));
            }
        }
        if n.bit_width_vartime() != length {
            return Err(error!(
                "Invalid private key file (the modulus is not {length} bits wide)."
            ));
        }
        let key =
            PrivateKey::from_components(n, BoxedUint::from(u64::from(exponent)), d, vec![p, q])
                .map_err(key_err)
                .prepend_error("Invalid private key:")?;

        // `rsa` recomputes the CRT values from `d`, `p` and `q`; the file's copies must agree.
        let consistent = match (key.dp(), key.dq(), key.crt_coefficient()) {
            (Some(dp), Some(dq), Some(qinv)) => {
                same_value(dp, &exp_p) && same_value(dq, &exp_q) && same_value(&qinv, &iqmp)
            }
            _ => false,
        };
        if !consistent {
            return Err(error!(
                "Invalid private key (CRT values do not match the primes)."
            ));
        }

        Ok(BIPrivateKey {
            name,
            length,
            exponent,
            key,
        })
    }

    /// Generate a new private key with the given name and bitlength.
    ///
    /// Arma 3 uses 1024 bit keys.
    ///
    /// # Errors
    ///
    /// Returns an error if `length` is below 384 bits (the `CryptoAPI` minimum) or the keypair cannot be generated.
    pub fn generate(length: u32, name: String) -> Result<BIPrivateKey, Error> {
        check_sign_length(length)?;
        let rsa = Rsa::generate(length).map_err(|e| error!("Failed to generate keypair: {e}"))?;
        // `OpenSSL` hands out its numbers as plain vectors; ours are wiped on drop.
        let secret = |v: &BigNumRef| BoxedUint::from_be_slice_vartime(&Zeroizing::new(v.to_vec()));
        let factor = |v: Option<&BigNumRef>| {
            let v = v.ok_or_else(|| error!("Generated keypair is missing a factor."))?;
            Ok::<_, Error>(secret(v))
        };
        let exponent = 65537;
        let key = PrivateKey::from_components(
            secret(rsa.n()),
            BoxedUint::from(u64::from(exponent)),
            secret(rsa.d()),
            vec![factor(rsa.p())?, factor(rsa.q())?],
        )
        .map_err(key_err)
        .prepend_error("Generated keypair:")?;

        Ok(BIPrivateKey {
            name,
            length,
            exponent,
            key,
        })
    }

    /// Returns the public key for this private key.
    ///
    /// # Errors
    ///
    /// Returns an error if the key material is invalid.
    pub fn to_public_key(&self) -> Result<BIPublicKey, Error> {
        Ok(BIPublicKey {
            name: self.name.clone(),
            length: self.length,
            exponent: self.exponent,
            key: self.key.to_public_key(),
        })
    }

    /// Signs the given PBO with this private key.
    ///
    /// # Errors
    ///
    /// Returns an error if the key is below 384 bits (the `CryptoAPI` minimum), hashing fails or the RSA operation
    /// fails.
    pub fn sign(&self, pbo: &PBO, version: BISignVersion) -> Result<BISign, Error> {
        check_sign_length(self.length)?;
        let hashes = generate_hashes(pbo, version, self.length)?;
        let n = self.key.n().as_ref();
        let precision = n.bits_precision();

        // Blinding (base, exponent and CRT modulus) uses the OS RNG; it changes the computation, never the
        // result, so signatures stay deterministic. The result is checked against the public key, which
        // catches a faulty CRT computation before a bad signature is written.
        let mut rng = SysRng;
        let mut sign_one = |padded: &[u8]| -> Result<BoxedUint, Error> {
            let m = BoxedUint::from_be_slice(padded, precision)
                .map_err(|_| error!("The hash does not fit the key."))?;
            rsa_decrypt_and_check(&self.key, Some(&mut rng), &m).map_err(key_err)
        };
        let [h1, h2, h3] = &hashes;
        let (sig1, sig2, sig3) = (sign_one(h1)?, sign_one(h2)?, sign_one(h3)?);

        Ok(BISign {
            version,
            name: self.name.clone(),
            length: self.length,
            exponent: self.exponent,
            n: n.clone(),
            sig1,
            sig2,
            sig3,
        })
    }

    /// Write private key to output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        output
            .write_cstring(&self.name)
            .prepend_error("Key name:")?;
        output.write_u32::<LittleEndian>(self.length / 16 * 9 + 20)?;
        output.write_all(b"\x07\x02\x00\x00\x00\x24\x00\x00")?;
        output.write_all(b"RSA2")?;
        output.write_u32::<LittleEndian>(self.length)?;
        output.write_u32::<LittleEndian>(self.exponent)?;
        let full = (self.length / 8) as usize;
        let half = (self.length / 16) as usize;
        let primes = self.key.primes();
        let (Some(p), Some(q)) = (primes.first(), primes.get(1)) else {
            return Err(error!("Invalid private key (missing primes)."));
        };
        // The CRT values are recomputed from `d`, `p` and `q`; `read` checks that the file holds the same.
        let (Some(exp_p), Some(exp_q), Some(iqmp)) =
            (self.key.dp(), self.key.dq(), self.key.crt_coefficient())
        else {
            return Err(error!("Invalid private key (missing CRT values)."));
        };
        write_uint(output, self.key.n().as_ref(), full)?;
        write_uint(output, p.as_ref(), half)?;
        write_uint(output, q.as_ref(), half)?;
        write_uint(output, exp_p, half)?;
        write_uint(output, exp_q, half)?;
        write_uint(output, &iqmp, half)?;
        write_uint(output, self.key.d(), full)?;
        Ok(())
    }
}

impl BIPublicKey {
    /// The key name, as stored in the key file.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Reads a public key from the given input.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not a valid public key.
    pub fn read<I: Read>(input: &mut I) -> Result<BIPublicKey, Error> {
        let name = input.read_cstring()?;
        let temp = input.read_u32::<LittleEndian>()?;
        check_blob_header(input, PUBLIC_BLOB, PUBLIC_MAGIC, "public")?;
        let length = input.read_u32::<LittleEndian>()?;
        let exponent = input.read_u32::<LittleEndian>()?;

        check_key_length(length)?;
        if temp != length / 8 + 20 {
            return Err(error!("Invalid public key file (bad length field)."));
        }

        let n = read_uint(input, (length / 8) as usize)?;
        // The modulus must be exactly `length` bits wide (a key frame bigger than its number, or a length that is
        // no multiple of 8, is rejected). BI keys down to 288 bits are accepted, `rsa`'s default limits (1024 to
        // 8192 bits) are widened to exactly this size.
        let length_bits = length as usize;
        let key = PublicKey::new_with_size_limits(
            n,
            BoxedUint::from(u64::from(exponent)),
            length_bits..=length_bits,
        )
        .map_err(key_err)
        .prepend_error("Invalid public key:")?;

        Ok(BIPublicKey {
            name,
            length,
            exponent,
            key,
        })
    }

    /// Verifies a signature against this public key.
    ///
    /// # Errors
    ///
    /// Returns an error if hashing fails or one of the signature hashes doesn't match.
    pub fn verify(&self, pbo: &PBO, signature: &BISign) -> Result<(), Error> {
        let hashes = generate_hashes(pbo, signature.version, self.length)?;
        let n = self.key.n().as_ref();
        let precision = n.bits_precision();

        let values = [&signature.sig1, &signature.sig2, &signature.sig3];
        let mut signed = Vec::with_capacity(values.len());
        for (i, value) in values.into_iter().enumerate() {
            let number = i + 1;
            // a signature value is below n; a bigger one (or one too wide for the key) is no signature
            let value = BoxedUint::from_be_slice(&value.to_be_bytes_trimmed_vartime(), precision)
                .ok()
                .filter(|value| value < n)
                .ok_or_else(|| error!("Signature {number} is not smaller than the modulus."))?;
            signed.push(rsa_encrypt(&self.key, &value).map_err(key_err)?);
        }

        // Plain `==` on the trimmed bytes: both sides are public (the recovered block and the hashes of the
        // PBO), so no constant-time comparison is needed.
        for (i, (signed, real)) in signed.iter().zip(&hashes).enumerate() {
            let signed = signed.to_be_bytes_trimmed_vartime();
            let real = trim_zeros(real);
            if *signed != *real {
                let (s, r) = display_hashes(&signed, real);
                return Err(error!(
                    "Hash {} doesn't match\nSigned hash: {}\nReal hash:   {}",
                    i + 1,
                    s,
                    r
                ));
            }
        }

        Ok(())
    }

    /// Write public key to output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        output
            .write_cstring(&self.name)
            .prepend_error("Key name:")?;
        output.write_u32::<LittleEndian>(self.length / 8 + 20)?;
        output.write_all(b"\x06\x02\x00\x00\x00\x24\x00\x00")?;
        output.write_all(b"RSA1")?;
        output.write_u32::<LittleEndian>(self.length)?;
        output.write_u32::<LittleEndian>(self.exponent)?;
        write_uint(output, self.key.n().as_ref(), (self.length / 8) as usize)?;
        Ok(())
    }
}

impl From<BISignVersion> for u32 {
    fn from(val: BISignVersion) -> Self {
        match val {
            BISignVersion::V2 => 2,
            BISignVersion::V3 => 3,
        }
    }
}

/// BI signature (.bisign)
impl BISign {
    /// Reads a signature from the given input.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not a valid signature.
    pub fn read<I: Read>(input: &mut I) -> Result<BISign, Error> {
        let name = input.read_cstring()?;
        let temp = input.read_u32::<LittleEndian>()?;
        check_blob_header(input, PUBLIC_BLOB, PUBLIC_MAGIC, "public")?;
        let length = input.read_u32::<LittleEndian>()?;
        let exponent = input.read_u32::<LittleEndian>()?;

        check_key_length(length)?;
        if temp != length / 8 + 20 {
            return Err(error!("Invalid signature file (bad length field)."));
        }

        let size = (length / 8) as usize;
        let n = read_uint(input, size)?;
        if n.bit_width_vartime() != length {
            return Err(error!(
                "Invalid signature file (the modulus is not {length} bits wide)."
            ));
        }

        input.read_u32::<LittleEndian>()?;
        let sig1 = read_uint(input, size)?;

        let version = match input.read_u32::<LittleEndian>()? {
            2 => BISignVersion::V2,
            3 => BISignVersion::V3,
            _ => {
                return Err(error!("Unknown BISign version."));
            }
        };

        input.read_u32::<LittleEndian>()?;
        let sig2 = read_uint(input, size)?;

        input.read_u32::<LittleEndian>()?;
        let sig3 = read_uint(input, size)?;

        Ok(BISign {
            version,
            name,
            length,
            exponent,
            n,
            sig1,
            sig2,
            sig3,
        })
    }

    /// Writes the signature to the given output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        output
            .write_cstring(&self.name)
            .prepend_error("Key name:")?;
        output.write_u32::<LittleEndian>(self.length / 8 + 20)?;
        output.write_all(b"\x06\x02\x00\x00\x00\x24\x00\x00")?;
        output.write_all(b"RSA1")?;
        output.write_u32::<LittleEndian>(self.length)?;
        output.write_u32::<LittleEndian>(self.exponent)?;
        write_uint(output, &self.n, (self.length / 8) as usize)?;
        output.write_u32::<LittleEndian>(self.length / 8)?;
        write_uint(output, &self.sig1, (self.length / 8) as usize)?;
        output.write_u32::<LittleEndian>(self.version.into())?;
        output.write_u32::<LittleEndian>(self.length / 8)?;
        write_uint(output, &self.sig2, (self.length / 8) as usize)?;
        output.write_u32::<LittleEndian>(self.length / 8)?;
        write_uint(output, &self.sig3, (self.length / 8) as usize)?;
        Ok(())
    }
}
