//! Functions for creating and working with BI keys and signatures

use std::io::{Error, Read, Write};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use openssl::bn::{BigNum, BigNumContext, BigNumRef};
use openssl::hash::{DigestBytes, Hasher, MessageDigest};
use openssl::rsa::Rsa;

use crate::io::{ReadExt, WriteExt};
use crate::pbo::PBO;

/// BI private key (.biprivatekey)
pub struct BIPrivateKey {
    name: String,
    length: u32,
    exponent: u32,
    n: BigNum,
    p: BigNum,
    q: BigNum,
    dmp1: BigNum,
    dmq1: BigNum,
    iqmp: BigNum,
    d: BigNum,
}

/// BI public key (.bikey)
pub struct BIPublicKey {
    name: String,
    length: u32,
    exponent: u32,
    n: BigNum,
}

/// BI signature version
#[derive(Copy, Clone)]
pub enum BISignVersion {
    /// Version 2
    V2,
    /// Version 3
    V3,
}

/// BI signature (.bisign)
pub struct BISign {
    version: BISignVersion,
    name: String,
    length: u32,
    exponent: u32,
    n: BigNum,
    sig1: BigNum,
    sig2: BigNum,
    sig3: BigNum,
}

fn write_bignum<O: Write>(output: &mut O, bn: &BigNum, size: usize) -> Result<(), Error> {
    let mut vec: Vec<u8> = bn.to_vec();
    vec = vec.iter().rev().copied().collect();
    vec.resize(size, 0);

    output.write_all(&vec)
}

/// A PBO entry's lowercased raw name bytes paired with its contents.
type NameFilePair<'a> = (Vec<u8>, &'a [u8]);

/// Creates a SHA-1 hasher.
///
/// # Errors
///
/// Returns an error if OpenSSL cannot provide the hasher.
fn sha1() -> Result<Hasher, Error> {
    Hasher::new(MessageDigest::sha1()).map_err(sha1_err)
}

/// Maps an OpenSSL error into an `std::io::Error`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "used as a map_err closure; the error is formatted, not stored"
)]
fn sha1_err(err: openssl::error::ErrorStack) -> Error {
    error!("OpenSSL operation failed: {err}")
}

/// Validates a key length read from a key or signature file.
///
/// # Errors
///
/// Returns an error if the length is not a sane RSA key length.
fn check_key_length(length: u32) -> Result<(), Error> {
    if !(288..=(1 << 20)).contains(&length) {
        return Err(error!("Invalid key length: {length}."));
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

/// Converts bytes into a `BigNum`.
///
/// # Errors
///
/// Returns an error if the bytes are not a valid big number.
fn bignum(bytes: &[u8]) -> Result<BigNum, Error> {
    BigNum::from_slice(bytes).map_err(|e| error!("Invalid key data: {e}"))
}

fn namehash(pbo: &PBO) -> Result<DigestBytes, Error> {
    let mut files_sorted: Vec<NameFilePair<'_>> = pbo
        .entries()
        .map(|(a, b)| (pbo.name_bytes(a).to_ascii_lowercase(), b))
        .collect();
    files_sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut h = sha1()?;

    for (name, data) in &files_sorted {
        if data.is_empty() {
            continue;
        }

        h.update(name).map_err(sha1_err)?;
    }

    h.finish().map_err(sha1_err)
}

fn filehash(pbo: &PBO, version: BISignVersion) -> Result<DigestBytes, Error> {
    let mut h = sha1()?;
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

        h.update(data).map_err(sha1_err)?;
        nothing = false;
    }

    match version {
        BISignVersion::V2 => {
            if nothing {
                h.update(b"nothing").map_err(sha1_err)?;
            }
        }
        BISignVersion::V3 => {
            if nothing {
                h.update(b"gnihton").map_err(sha1_err)?;
            }
        }
    }

    h.finish().map_err(sha1_err)
}

fn generate_hashes(
    pbo: &PBO,
    version: BISignVersion,
    length: u32,
) -> Result<(BigNum, BigNum, BigNum), Error> {
    // a PBO that was read keeps its stored trailer (a foreign one may differ from what we would
    // write); one that was not read gets the trailer `PBO::write` would produce
    let checksum = pbo.trailer_checksum()?;
    let hash1 = checksum.as_slice();

    let mut h = sha1()?;
    h.update(hash1).map_err(sha1_err)?;
    h.update(&namehash(pbo)?).map_err(sha1_err)?;
    if let Some(prefix) = pbo.prefix() {
        h.update(&pbo.extension_bytes(prefix)).map_err(sha1_err)?;
        if !prefix.ends_with('\\') {
            h.update(b"\\").map_err(sha1_err)?;
        }
    }
    let hash2 = h.finish().map_err(sha1_err)?;

    let mut h = sha1()?;
    h.update(&filehash(pbo, version)?).map_err(sha1_err)?;
    h.update(&namehash(pbo)?).map_err(sha1_err)?;
    if let Some(prefix) = pbo.prefix() {
        h.update(&pbo.extension_bytes(prefix)).map_err(sha1_err)?;
        if !prefix.ends_with('\\') {
            h.update(b"\\").map_err(sha1_err)?;
        }
    }
    let hash3 = h.finish().map_err(sha1_err)?;

    let size = usize::try_from(length / 8).map_err(|_| error!("Invalid key length: {length}."))?;
    Ok((
        pad_hash(hash1, size)?,
        pad_hash(&hash2, size)?,
        pad_hash(&hash3, size)?,
    ))
}

fn pad_hash(hash: &[u8], size: usize) -> Result<BigNum, Error> {
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

    bignum(&vec)
}

fn display_hashes(signed: &BigNum, real: &BigNum) -> Result<(String, String), Error> {
    let hex_signed = signed.to_hex_str().map_err(sha1_err)?.to_lowercase();
    let hex_real = real.to_hex_str().map_err(sha1_err)?.to_lowercase();

    if hex_signed.len() != hex_real.len() || hex_signed.len() <= 40 {
        return Ok((hex_signed, hex_real));
    }

    let (signed_padding, signed_hash) = hex_signed.split_at(hex_signed.len() - 40);
    let (real_padding, real_hash) = hex_real.split_at(hex_real.len() - 40);

    if signed_padding == real_padding {
        Ok((signed_hash.to_string(), real_hash.to_string()))
    } else {
        Ok((hex_signed, hex_real))
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
        input.read_u32::<LittleEndian>()?;
        input.read_u32::<LittleEndian>()?;
        input.read_u32::<LittleEndian>()?;
        let length = input.read_u32::<LittleEndian>()?;
        let exponent = input.read_u32::<LittleEndian>()?;

        check_key_length(length)?;
        if temp != length / 16 * 9 + 20 {
            return Err(error!("Invalid private key file (bad length field)."));
        }

        let mut buffer = vec![0; (length / 8) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let n = bignum(&buffer)?;

        buffer = vec![0; (length / 16) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let p = bignum(&buffer)?;

        buffer = vec![0; (length / 16) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let q = bignum(&buffer)?;

        buffer = vec![0; (length / 16) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let exponent1 = bignum(&buffer)?;

        buffer = vec![0; (length / 16) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let exponent2 = bignum(&buffer)?;

        buffer = vec![0; (length / 16) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let q_inv = bignum(&buffer)?;

        buffer = vec![0; (length / 8) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let d = bignum(&buffer)?;

        Ok(BIPrivateKey {
            name,
            length,
            exponent,
            n,
            p,
            q,
            dmp1: exponent1,
            dmq1: exponent2,
            iqmp: q_inv,
            d,
        })
    }

    /// Generate a new private key with the given name and bitlength.
    ///
    /// Arma 3 uses 1024 bit keys.
    ///
    /// # Errors
    ///
    /// Returns an error if the keypair cannot be generated.
    pub fn generate(length: u32, name: String) -> Result<BIPrivateKey, Error> {
        let rsa = Rsa::generate(length).map_err(|e| error!("Failed to generate keypair: {e}"))?;
        let factor = |v: Option<&BigNumRef>| {
            let v = v.ok_or_else(|| error!("Generated keypair is missing a factor."))?;
            bignum(&v.to_vec())
        };

        Ok(BIPrivateKey {
            name,
            length,
            exponent: 65537,
            n: bignum(&rsa.n().to_vec())?,
            p: factor(rsa.p())?,
            q: factor(rsa.q())?,
            dmp1: factor(rsa.dmp1())?,
            dmq1: factor(rsa.dmq1())?,
            iqmp: factor(rsa.iqmp())?,
            d: bignum(&rsa.d().to_vec())?,
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
            n: bignum(&self.n.to_vec())?,
        })
    }

    /// Signs the given PBO with this private key.
    ///
    /// # Errors
    ///
    /// Returns an error if hashing or the RSA operation fails.
    pub fn sign(&self, pbo: &PBO, version: BISignVersion) -> Result<BISign, Error> {
        let (hash1, hash2, hash3) = generate_hashes(pbo, version, self.length)?;

        let mut ctx = BigNumContext::new().map_err(sha1_err)?;

        let mut sig1 = BigNum::new().map_err(sha1_err)?;
        sig1.mod_exp(&hash1, &self.d, &self.n, &mut ctx)
            .map_err(sha1_err)?;
        let mut sig2 = BigNum::new().map_err(sha1_err)?;
        sig2.mod_exp(&hash2, &self.d, &self.n, &mut ctx)
            .map_err(sha1_err)?;
        let mut sig3 = BigNum::new().map_err(sha1_err)?;
        sig3.mod_exp(&hash3, &self.d, &self.n, &mut ctx)
            .map_err(sha1_err)?;

        Ok(BISign {
            version,
            name: self.name.clone(),
            length: self.length,
            exponent: self.exponent,
            n: bignum(&self.n.to_vec())?,
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
        output.write_cstring(&self.name)?;
        output.write_u32::<LittleEndian>(self.length / 16 * 9 + 20)?;
        output.write_all(b"\x07\x02\x00\x00\x00\x24\x00\x00")?;
        output.write_all(b"RSA2")?;
        output.write_u32::<LittleEndian>(self.length)?;
        output.write_u32::<LittleEndian>(self.exponent)?;
        write_bignum(output, &self.n, (self.length / 8) as usize)?;
        write_bignum(output, &self.p, (self.length / 16) as usize)?;
        write_bignum(output, &self.q, (self.length / 16) as usize)?;
        write_bignum(output, &self.dmp1, (self.length / 16) as usize)?;
        write_bignum(output, &self.dmq1, (self.length / 16) as usize)?;
        write_bignum(output, &self.iqmp, (self.length / 16) as usize)?;
        write_bignum(output, &self.d, (self.length / 8) as usize)?;
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
        input.read_u32::<LittleEndian>()?;
        input.read_u32::<LittleEndian>()?;
        input.read_u32::<LittleEndian>()?;
        let length = input.read_u32::<LittleEndian>()?;
        let exponent = input.read_u32::<LittleEndian>()?;

        check_key_length(length)?;
        if temp != length / 8 + 20 {
            return Err(error!("Invalid public key file (bad length field)."));
        }

        let mut buffer = vec![0; (length / 8) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let n = bignum(&buffer)?;

        Ok(BIPublicKey {
            name,
            length,
            exponent,
            n,
        })
    }

    /// Verifies a signature against this public key.
    ///
    /// # Errors
    ///
    /// Returns an error if hashing fails or one of the signature hashes doesn't match.
    pub fn verify(&self, pbo: &PBO, signature: &BISign) -> Result<(), Error> {
        let (real_hash1, real_hash2, real_hash3) =
            generate_hashes(pbo, signature.version, self.length)?;

        let mut ctx = BigNumContext::new().map_err(sha1_err)?;

        let exponent = BigNum::from_u32(self.exponent).map_err(sha1_err)?;

        let mut signed_hash1 = BigNum::new().map_err(sha1_err)?;
        signed_hash1
            .mod_exp(&signature.sig1, &exponent, &self.n, &mut ctx)
            .map_err(sha1_err)?;
        let mut signed_hash2 = BigNum::new().map_err(sha1_err)?;
        signed_hash2
            .mod_exp(&signature.sig2, &exponent, &self.n, &mut ctx)
            .map_err(sha1_err)?;
        let mut signed_hash3 = BigNum::new().map_err(sha1_err)?;
        signed_hash3
            .mod_exp(&signature.sig3, &exponent, &self.n, &mut ctx)
            .map_err(sha1_err)?;

        if real_hash1 != signed_hash1 {
            let (s, r) = display_hashes(&signed_hash1, &real_hash1)?;
            return Err(error!(
                "Hash 1 doesn't match\nSigned hash: {}\nReal hash:   {}",
                s, r
            ));
        }

        if real_hash2 != signed_hash2 {
            let (s, r) = display_hashes(&signed_hash2, &real_hash2)?;
            return Err(error!(
                "Hash 2 doesn't match\nSigned hash: {}\nReal hash:   {}",
                s, r
            ));
        }

        if real_hash3 != signed_hash3 {
            let (s, r) = display_hashes(&signed_hash3, &real_hash3)?;
            return Err(error!(
                "Hash 3 doesn't match\nSigned hash: {}\nReal hash:   {}",
                s, r
            ));
        }

        Ok(())
    }

    /// Write public key to output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        output.write_cstring(&self.name)?;
        output.write_u32::<LittleEndian>(self.length / 8 + 20)?;
        output.write_all(b"\x06\x02\x00\x00\x00\x24\x00\x00")?;
        output.write_all(b"RSA1")?;
        output.write_u32::<LittleEndian>(self.length)?;
        output.write_u32::<LittleEndian>(self.exponent)?;
        write_bignum(output, &self.n, (self.length / 8) as usize)?;
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
        input.read_u32::<LittleEndian>()?;
        input.read_u32::<LittleEndian>()?;
        input.read_u32::<LittleEndian>()?;
        let length = input.read_u32::<LittleEndian>()?;
        let exponent = input.read_u32::<LittleEndian>()?;

        check_key_length(length)?;
        if temp != length / 8 + 20 {
            return Err(error!("Invalid signature file (bad length field)."));
        }

        let mut buffer = vec![0; (length / 8) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let n = bignum(&buffer)?;

        input.read_u32::<LittleEndian>()?;

        let mut buffer = vec![0; (length / 8) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let sig1 = bignum(&buffer)?;

        let version = match input.read_u32::<LittleEndian>()? {
            2 => BISignVersion::V2,
            3 => BISignVersion::V3,
            _ => {
                return Err(error!("Unknown BISign version."));
            }
        };

        input.read_u32::<LittleEndian>()?;

        let mut buffer = vec![0; (length / 8) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let sig2 = bignum(&buffer)?;

        input.read_u32::<LittleEndian>()?;

        let mut buffer = vec![0; (length / 8) as usize];
        input.read_exact(&mut buffer)?;
        buffer = buffer.iter().rev().copied().collect();
        let sig3 = bignum(&buffer)?;

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
        output.write_cstring(&self.name)?;
        output.write_u32::<LittleEndian>(self.length / 8 + 20)?;
        output.write_all(b"\x06\x02\x00\x00\x00\x24\x00\x00")?;
        output.write_all(b"RSA1")?;
        output.write_u32::<LittleEndian>(self.length)?;
        output.write_u32::<LittleEndian>(self.exponent)?;
        write_bignum(output, &self.n, (self.length / 8) as usize)?;
        output.write_u32::<LittleEndian>(self.length / 8)?;
        write_bignum(output, &self.sig1, (self.length / 8) as usize)?;
        output.write_u32::<LittleEndian>(self.version.into())?;
        output.write_u32::<LittleEndian>(self.length / 8)?;
        write_bignum(output, &self.sig2, (self.length / 8) as usize)?;
        output.write_u32::<LittleEndian>(self.length / 8)?;
        write_bignum(output, &self.sig3, (self.length / 8) as usize)?;
        Ok(())
    }
}
