//! Mutation test for the readers of untrusted files: byte-flipped and truncated copies of the
//! fixtures must make the readers return an error or a value, never panic, hang or abort.
#![expect(clippy::expect_used, reason = "a failing expect is a failing test")]
mod support;

use std::fmt::Write as _;
use std::fs;
use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use dayzmake_core::config::Config;
use dayzmake_core::p3d::{self, P3D};
use dayzmake_core::paa::Paa;
use dayzmake_core::pbo::PBO;
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign};

const SEED: u64 = 0x9E37_79B9_7F4A_7C15;
const MUTANTS_PER_FILE: usize = 300;
const MUTANTS_PER_FILE_LONG: usize = 20_000;

/// Deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A value in `0..bound` (`bound` must not be 0).
    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % (bound.max(1) as u64)).unwrap_or(0)
    }
}

/// Returns a mutated copy: 1 to 8 random byte flips, and in 1 of 4 cases a random truncation.
fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut data = seed.to_vec();
    if data.is_empty() {
        return data;
    }
    for _ in 0..=rng.below(8) {
        let at = rng.below(data.len());
        let flip = u8::try_from(rng.next() & 0xff).unwrap_or(0xff).max(1);
        if let Some(b) = data.get_mut(at) {
            *b ^= flip;
        }
    }
    if rng.below(4) == 0 {
        let len = rng.below(data.len());
        data.truncate(len.max(1));
    }
    data
}

/// Feeds `data` to the readers for the extension `ext`; results are dropped.
fn feed(ext: &str, data: &[u8]) {
    match ext {
        "pbo" => {
            if let Ok(pbo) = PBO::read(&mut Cursor::new(data)) {
                let names: Vec<String> = pbo.entry_names().map(str::to_string).collect();
                for name in names {
                    if let Ok(Some(content)) = pbo.file_data(&name)
                        && name.to_lowercase().ends_with(".bin")
                    {
                        let _ = Config::read_rapified(&mut Cursor::new(content));
                    }
                }
            }
        }
        "bin" => {
            let _ = Config::read_rapified(&mut Cursor::new(data));
        }
        "paa" => {
            if let Ok(paa) = Paa::read(&mut Cursor::new(data)) {
                let _ = paa.to_dynamic_image();
            }
        }
        "p3d" => {
            let _ = P3D::read(&mut Cursor::new(data));
            let _ = p3d::model_magic(&mut Cursor::new(data));
        }
        "bikey" => {
            let _ = BIPublicKey::read(&mut Cursor::new(data));
        }
        "biprivatekey" => {
            let _ = BIPrivateKey::read(&mut Cursor::new(data));
        }
        "bisign" => {
            let _ = BISign::read(&mut Cursor::new(data));
        }
        _ => {}
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// A PBO with one `Cprs` (LZSS) entry `c.txt`, and the bytes it unpacks to. The data is three
/// literals, a back reference (offset 3, length 9) and three more literals.
fn lzss_pbo() -> (Vec<u8>, Vec<u8>) {
    let expected = b"abcabcabcabcXYZ".to_vec();
    let packed = [
        0x77, b'a', b'b', b'c', 0x03, 0x06, b'X', b'Y', b'Z', 0, 0, 0, 0,
    ];
    let mut pbo = b"c.txt\0".to_vec();
    for field in [0x4370_7273_u32, 15, 0, 0, 13] {
        pbo.extend_from_slice(&field.to_le_bytes());
    }
    pbo.extend_from_slice(&[0; 21]); // the empty header that ends the list
    pbo.extend_from_slice(&packed);
    pbo.extend_from_slice(&[0; 21]); // separator and checksum
    (pbo, expected)
}

/// The compressed entry of the synthetic seed unpacks to the expected bytes.
#[test]
fn lzss_seed_decompresses() {
    let (bytes, expected) = lzss_pbo();
    let pbo = PBO::read(&mut Cursor::new(bytes)).expect("read");
    assert_eq!(pbo.file_data("c.txt").expect("data"), Some(expected));
}

/// Every seed with a reader as (name, extension, bytes), in a stable order: the fixtures, plus a
/// synthetic PBO with a compressed entry (no fixture has one).
fn seeds() -> Vec<(String, String, Vec<u8>)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = Vec::new();
    collect(&root.join("bi"), &mut files);
    files.push(root.join("mlod/small.p3d"));
    files.sort();
    let mut seeds: Vec<(String, String, Vec<u8>)> = files
        .into_iter()
        .filter_map(|p| {
            let ext = p.extension()?.to_str()?.to_lowercase();
            [
                "pbo",
                "bin",
                "paa",
                "p3d",
                "bikey",
                "bisign",
                "biprivatekey",
            ]
            .contains(&ext.as_str())
            .then(|| {
                (
                    p.display().to_string(),
                    ext,
                    fs::read(&p).unwrap_or_default(),
                )
            })
        })
        .collect();
    seeds.push((
        "synthetic lzss.pbo".to_string(),
        "pbo".to_string(),
        lzss_pbo().0,
    ));
    seeds
}

fn run_mutants(count: usize) {
    let seeds = seeds();
    assert!(!seeds.is_empty(), "no seed files found");
    let mut failures: Vec<String> = Vec::new();
    for (path, ext, seed) in &seeds {
        let mut rng = Rng(SEED);
        for i in 0..count {
            let mutant = mutate(seed, &mut rng);
            if catch_unwind(AssertUnwindSafe(|| feed(ext, &mutant))).is_err() {
                let hex = mutant.iter().take(64).fold(String::new(), |mut acc, b| {
                    let _ = write!(acc, "{b:02x}");
                    acc
                });
                failures.push(format!(
                    "{} mutant {i} ({} bytes) panicked: {hex}",
                    path,
                    mutant.len()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn mutants_do_not_panic() {
    run_mutants(MUTANTS_PER_FILE);
}

#[test]
#[ignore = "slow: 20 000 mutants per fixture"]
fn mutants_do_not_panic_long() {
    run_mutants(MUTANTS_PER_FILE_LONG);
}

// ---- regression cases: the exact bytes that used to fail ----

/// A PAA with a single uncompressed mipmap of `data`.
fn paa_with_mip(magic: u16, w: u16, h: u16, data: &[u8]) -> Vec<u8> {
    let mut b = magic.to_le_bytes().to_vec();
    b.extend_from_slice(&[0, 0]); // no taggs, no palette
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_le_bytes()[..3]);
    b.extend_from_slice(data);
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    b
}

/// A DXT1 mipmap that holds less data than its dimensions need made texpresso index out of range.
#[test]
fn paa_short_dxt_mipmap_is_an_error_not_a_panic() {
    let bytes = paa_with_mip(0xff01, 8, 8, &[0; 4]);
    let paa = Paa::read(&mut Cursor::new(bytes)).expect("the container is well formed");
    assert!(paa.to_dynamic_image().is_err());
}

/// A zero width made `chunks_mut(0)` panic.
#[test]
fn paa_zero_width_decode_is_an_error_not_a_panic() {
    // [0x11, 0, 0] is the LZO end-of-stream marker: it decompresses to 0 bytes
    let bytes = paa_with_mip(0xff01, 0x8000, 8, &[0x11, 0, 0]);
    let paa = Paa::read(&mut Cursor::new(bytes)).expect("the container is well formed");
    assert_eq!(paa.width, 0);
    assert!(paa.to_dynamic_image().is_err());
}

/// Appends a mipmap record to a PAA that ends with a 6 byte terminator.
fn push_mip(paa: &mut Vec<u8>, w: u16, h: u16, data: &[u8]) {
    paa.truncate(paa.len() - 6);
    paa.extend_from_slice(&w.to_le_bytes());
    paa.extend_from_slice(&h.to_le_bytes());
    paa.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_le_bytes()[..3]);
    paa.extend_from_slice(data);
    paa.extend_from_slice(&[0; 6]);
}

/// Real PAAs have at most 16 mipmaps.
#[test]
fn paa_too_many_mipmaps_is_an_error() {
    let mut bytes = paa_with_mip(0xff01, 4, 4, &[0; 8]);
    for _ in 0..16 {
        push_mip(&mut bytes, 4, 4, &[0; 8]);
    }
    assert!(Paa::read(&mut Cursor::new(bytes)).is_err());
}

/// Three 16384 x 16384 DXT1 mipmaps (128 MiB each) out of a file of about 1.5 MiB.
#[test]
fn paa_lzo_expansion_is_bounded_in_total() {
    let zeros = vec![0u8; 128 << 20];
    let packed = lzo1x::compress(&zeros, lzo1x::CompressLevel::default());
    drop(zeros);
    let mut bytes = paa_with_mip(0xff01, 0x8000 | 16384, 16384, &packed);
    for _ in 0..2 {
        push_mip(&mut bytes, 0x8000 | 16384, 16384, &packed);
    }
    assert!(Paa::read(&mut Cursor::new(bytes)).is_err());
}

/// The key name comes from the key file and names the default signature file; with a path
/// separator in it the signature used to be written below a directory the key file picked.
#[test]
fn sign_rejects_a_key_name_with_a_path_separator() {
    use dayzmake::commands::sign::cmd_sign;
    use dayzmake_core::sign::BISignVersion;

    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bi/sign");
    let key = fs::read(fixtures.join("armk.biprivatekey")).expect("fixture");
    let rest = key.get(5..).expect("key longer than its name");
    assert_eq!(key.get(..5), Some(&b"armk\0"[..]));
    let mut evil = b"a/bc\0".to_vec();
    evil.extend_from_slice(rest);

    let tmp = tempfile::tempdir().expect("tempdir");
    let key_path = tmp.path().join("evil.biprivatekey");
    fs::write(&key_path, evil).expect("write key");
    fs::copy(fixtures.join("testsig.pbo"), tmp.path().join("testsig.pbo")).expect("copy pbo");
    // the directory the separator would send the signature into
    fs::create_dir(tmp.path().join("testsig.pbo.a")).expect("mkdir");

    let result = cmd_sign(
        key_path,
        tmp.path().join("testsig.pbo"),
        None,
        BISignVersion::V3,
        true,
    );
    assert!(result.is_err(), "signed with a key name containing '/'");
    assert!(!tmp.path().join("testsig.pbo.a/bc.bisign").exists());
}

/// A PBO with the given entry names and header extensions, built through the library.
fn crafted_pbo(names: &[&str], extensions: &[(&str, &str)]) -> Vec<u8> {
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (*n, b"x".as_slice())).collect();
    support::raw_pbo(&entries, extensions)
}

/// Runs the binary in `cwd`; returns (stdout, stderr).
fn run_cli(args: &[&str], cwd: &Path) -> (String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .output()
        .expect("run dayzmake");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Names and header extensions from a PBO end up on the terminal; an escape sequence in them
/// must not reach it as such.
#[test]
fn inspect_and_unpack_do_not_print_terminal_escapes() {
    let pbo = crafted_pbo(
        &["a\x1b[2Jb.txt", "..\\\x1b]0;pwned\x07"],
        &[("prefix", "x"), ("k\x1b[31m", "v\x1b[0m")],
    );
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("in.pbo"), pbo).expect("write");

    let (stdout, stderr) = run_cli(&["inspect", "in.pbo"], tmp.path());
    assert!(stdout.contains("a\\u{1b}[2Jb.txt"), "{stdout:?}");
    assert!(!stdout.contains('\x1b'), "{stdout:?}");
    assert!(!stderr.contains('\x1b'), "{stderr:?}");

    let (stdout, stderr) = run_cli(&["unpack", "in.pbo", "target"], tmp.path());
    assert!(!stdout.contains('\x1b'), "{stdout:?}");
    assert!(
        !stderr.contains('\x1b') && !stderr.contains('\x07'),
        "{stderr:?}"
    );
    assert!(stderr.contains("unsafe-path"), "{stderr:?}");
}

/// One 16384 x 16384 DXT1 LZO mipmap (128 MiB, from about 600 KB) passes `read`, but decoding it
/// would allocate 1 GiB of RGBA.
#[test]
fn paa_decoded_size_is_bounded() {
    let zeros = vec![0u8; 128 << 20];
    let packed = lzo1x::compress(&zeros, lzo1x::CompressLevel::default());
    drop(zeros);
    let bytes = paa_with_mip(0xff01, 0x8000 | 16384, 16384, &packed);
    let paa = Paa::read(&mut Cursor::new(bytes)).expect("the container is well formed");
    assert!(paa.to_dynamic_image().is_err());
}

#[test]
fn duplicate_header_extension_keys_read_and_write_deterministically() {
    let mut bytes = Vec::new();
    let header = |out: &mut Vec<u8>, name: &[u8], method: u32, size: u32| {
        out.extend_from_slice(name);
        out.push(0);
        for v in [method, size, 0, 0, size] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    };
    header(&mut bytes, b"", 0x5665_7273, 0);
    bytes.extend_from_slice(b"prefix\0p\0k\x001\0j\0x\0k\x002\0\0");
    header(&mut bytes, b"a.txt", 0, 1);
    header(&mut bytes, b"", 0, 0);
    bytes.push(b'x');
    bytes.push(0);
    bytes.extend_from_slice(&[0; 20]);

    let pbo = PBO::read(&mut &bytes[..]).expect("read");
    let pairs: Vec<(&str, &str)> = pbo.header_extensions().collect();
    assert_eq!(pairs, [("prefix", "p"), ("k", "2"), ("j", "x")]);
    let first = pbo.to_cursor().expect("write").into_inner();
    let second = pbo.to_cursor().expect("write").into_inner();
    assert_eq!(first, second);
}
