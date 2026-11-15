//! The packing and unpacking options of the PBO API.
mod support;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use dayzmake_core::pbo::{PBO, PackOptions, UnpackOptions, unpack};
#[cfg(feature = "sign")]
use dayzmake_core::sign::{BIPrivateKey, BISignVersion};
use dayzmake_core::warning::Warnings;

#[expect(clippy::unwrap_used, reason = "test helper")]
fn pack_to_bytes(dir: &Path, options: &PackOptions) -> Vec<u8> {
    let pbo = PBO::from_directory(dir, options, None, &mut Warnings::new()).unwrap();
    let mut bytes = Vec::new();
    pbo.write(&mut bytes).unwrap();
    bytes
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn read(bytes: &[u8]) -> PBO {
    PBO::read(&mut Cursor::new(bytes)).unwrap()
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn addon() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("addon");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.c"), "b").unwrap();
    tmp
}

#[test]
fn pack_options_header_extensions_override_the_prefix_file() {
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join("$PBOPREFIX$"), "prefix=a\nproduct=x\n").unwrap();
    let mut options = PackOptions::pack();
    options.header_extensions = vec![
        ("product".to_string(), "y".to_string()),
        ("version".to_string(), "1".to_string()),
    ];
    let pbo = read(&pack_to_bytes(&root, &options));
    let got: Vec<(&str, &str)> = pbo.header_extensions().collect();
    assert_eq!(got, [("prefix", "a"), ("product", "y"), ("version", "1")]);
}

#[test]
fn pack_options_exclude_skips_files() {
    let tmp = addon();
    let mut options = PackOptions::pack();
    options.exclude = vec!["*.txt".to_string()];
    let pbo = read(&pack_to_bytes(&tmp.path().join("addon"), &options));
    assert!(!pbo.entry_names().any(|n| n == "a.txt"));
    assert!(pbo.entry_names().any(|n| n == "b.c"));
}

#[test]
fn build_option_rapifies_config_cpp() {
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join("config.cpp"), "class CfgPatches {};\n").unwrap();
    let pbo = read(&pack_to_bytes(&root, &PackOptions::build()));
    let (_, config) = pbo
        .entries()
        .find(|(n, _)| *n == "config.bin")
        .expect("config.bin entry");
    assert!(config.starts_with(b"\0raP"));
    assert!(!pbo.entry_names().any(|n| n == "config.cpp"));

    let packed = read(&pack_to_bytes(&root, &PackOptions::pack()));
    assert!(packed.entry_names().any(|n| n == "config.cpp"));
}

#[test]
fn unpack_refuses_existing_files_without_force() {
    let tmp = addon();
    let bytes = pack_to_bytes(&tmp.path().join("addon"), &PackOptions::pack());
    let target = tmp.path().join("out");
    let options = UnpackOptions::default();
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap();
    let err = unpack(
        &mut Cursor::new(&bytes),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    let mut force = UnpackOptions::default();
    force.force = true;
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &force,
        &mut Warnings::new(),
    )
    .unwrap();
}

#[test]
fn unpack_rejects_unsafe_entry_names() {
    let tmp = addon();
    let bytes = support::raw_pbo(&[("a.txt", b"a"), ("..\\evil.txt", b"evil")], &[]);
    // the hand-built PBO really holds the hostile entry
    assert!(read(&bytes).entry_names().any(|n| n == "..\\evil.txt"));

    let target = tmp.path().join("out").join("inner");
    let mut warnings = Warnings::new();
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &UnpackOptions::default(),
        &mut warnings,
    )
    .unwrap();
    assert!(!tmp.path().join("out").join("evil.txt").exists());
    assert!(!target.join("evil.txt").exists());
    assert!(target.join("a.txt").exists());
    let named: Vec<_> = warnings
        .warnings()
        .iter()
        .filter(|w| w.message.contains("evil.txt"))
        .collect();
    assert_eq!(named.len(), 1, "{:?}", warnings.warnings());
}

#[test]
fn packing_a_model_without_binarizer_warns_only_outside_windows() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("addon");
    fs::create_dir_all(&root).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mlod/small.p3d"),
        root.join("small.p3d"),
    )
    .unwrap();
    let mut warnings = Warnings::new();
    PBO::from_directory(&root, &PackOptions::pack(), None, &mut warnings).unwrap();
    let warned = warnings
        .warnings()
        .iter()
        .any(|w| w.name == Some("non-windows-binarization"));
    // Windows has a native binarize.exe, so "non-Windows systems" must not be mentioned there.
    assert_eq!(warned, !cfg!(windows));
}

fn sign_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/sign")
}

#[cfg(feature = "sign")]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn armk() -> BIPrivateKey {
    BIPrivateKey::read(&mut fs::File::open(sign_dir().join("armk.biprivatekey")).unwrap()).unwrap()
}

#[cfg(feature = "sign")]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn sign_bytes(key: &BIPrivateKey, pbo: &PBO) -> Vec<u8> {
    let mut out = Vec::new();
    key.sign(pbo, BISignVersion::V3)
        .unwrap()
        .write(&mut out)
        .unwrap();
    out
}

#[cfg(feature = "sign")]
#[test]
fn signing_a_fresh_pbo_matches_signing_it_after_a_roundtrip() {
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join("$PBOPREFIX$"), "my\\mod\n").unwrap();
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("sub").join("Z.c"), "class Z {};\n").unwrap();
    fs::write(root.join("B.hpp"), "x").unwrap();
    let pbo = PBO::from_directory(&root, &PackOptions::pack(), None, &mut Warnings::new()).unwrap();
    assert!(pbo.checksum().is_none());
    let key = armk();
    let reread = PBO::read(&mut pbo.to_cursor().unwrap()).unwrap();
    let fresh = sign_bytes(&key, &pbo);
    assert_eq!(fresh, sign_bytes(&key, &reread));

    let signature = key.sign(&pbo, BISignVersion::V3).unwrap();
    let public = key.to_public_key().unwrap();
    public.verify(&reread, &signature).unwrap();
    public.verify(&pbo, &signature).unwrap();
}

#[cfg(feature = "sign")]
#[test]
fn signing_a_read_pbo_uses_its_stored_checksum() {
    let dir = sign_dir();
    let pbo = PBO::read(&mut fs::File::open(dir.join("testsig.pbo")).unwrap()).unwrap();
    assert!(pbo.checksum().is_some());
    assert_eq!(
        sign_bytes(&armk(), &pbo),
        fs::read(dir.join("testsig.pbo.armk.bisign")).unwrap()
    );
}

#[test]
fn accessors_expose_entries_and_extensions_in_order() {
    let mut pbo = PBO::new();
    pbo.insert_entry("b.txt", b"bee".to_vec()).unwrap();
    pbo.insert_entry("a\\c.txt", b"sea".to_vec()).unwrap();
    pbo.set_header_extension("prefix", "p");
    pbo.set_header_extension("x", "1");
    pbo.set_header_extension("prefix", "q");
    assert!(pbo.checksum().is_none());
    // an unwritten PBO lists its entries in the order write() stores them
    assert_eq!(pbo.entry_names().collect::<Vec<_>>(), ["a\\c.txt", "b.txt"]);

    let read = read(&pack_bytes(&pbo));
    assert_eq!(
        read.entry_names().collect::<Vec<_>>(),
        ["a\\c.txt", "b.txt"]
    );
    assert_eq!(
        read.entries().collect::<Vec<_>>(),
        [
            ("a\\c.txt", b"sea".as_slice()),
            ("b.txt", b"bee".as_slice())
        ]
    );
    assert_eq!(
        read.header_extensions().collect::<Vec<_>>(),
        [("prefix", "q"), ("x", "1")]
    );
    assert_eq!(read.header_extension("x"), Some("1"));
    assert_eq!(read.prefix(), Some("q"));
    assert!(read.checksum().is_some());
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn pack_bytes(pbo: &PBO) -> Vec<u8> {
    let mut bytes = Vec::new();
    pbo.write(&mut bytes).unwrap();
    bytes
}

#[test]
fn insert_entry_rejects_unsafe_names() {
    for bad in ["", "..\\x", "/abs", "C:\\x", "a\\..\\..\\x"] {
        assert!(PBO::new().insert_entry(bad, vec![]).is_err(), "{bad:?}");
    }
}

#[test]
fn remove_entry_returns_the_data() {
    let mut pbo = PBO::new();
    pbo.insert_entry("a.txt", b"a".to_vec()).unwrap();
    assert_eq!(pbo.remove_entry("a.txt"), Some(b"a".to_vec()));
    assert_eq!(pbo.remove_entry("a.txt"), None);
    assert_eq!(pbo.entry_names().count(), 0);
}

#[test]
fn selection_gives_requested_entries() {
    let tmp = addon();
    let bytes = pack_to_bytes(&tmp.path().join("addon"), &PackOptions::pack());
    let sel = PBO::read_entries(&mut Cursor::new(&bytes), |n| n == "a.txt").unwrap();
    assert_eq!(sel.names(), ["a.txt", "b.c"]);
    assert_eq!(sel.entries().len(), 1);
    assert_eq!(sel.entries()[0].name(), "a.txt");
    assert_eq!(sel.entries()[0].data().unwrap(), b"a");
    assert_eq!(sel.header_extension("prefix"), Some("addon"));
    assert_eq!(
        sel.header_extensions().collect::<Vec<_>>(),
        [("prefix", "addon")]
    );
}

#[cfg(feature = "sign")]
#[test]
fn a_modified_read_pbo_signs_as_what_is_written() {
    let mut pbo = PBO::read(&mut fs::File::open(sign_dir().join("testsig.pbo")).unwrap()).unwrap();
    pbo.insert_entry("new.txt", b"new".to_vec()).unwrap();
    assert!(pbo.checksum().is_none());
    let key = armk();
    let signature = key.sign(&pbo, BISignVersion::V3).unwrap();
    let written = PBO::read(&mut pbo.to_cursor().unwrap()).unwrap();
    key.to_public_key()
        .unwrap()
        .verify(&written, &signature)
        .unwrap();
}

#[test]
fn mutators_drop_the_stored_checksum() {
    let mut pbo = PBO::read(&mut fs::File::open(sign_dir().join("testsig.pbo")).unwrap()).unwrap();
    pbo.set_header_extension("version", "2");
    assert!(pbo.checksum().is_none());
    let mut pbo = PBO::read(&mut fs::File::open(sign_dir().join("testsig.pbo")).unwrap()).unwrap();
    let first = pbo.entry_names().next().unwrap().to_string();
    pbo.remove_entry(&first).unwrap();
    assert!(pbo.checksum().is_none());
}

#[test]
fn insert_entry_normalises_separators_and_rejects_empty_components() {
    let mut pbo = PBO::new();
    pbo.insert_entry("a/b.txt", b"x".to_vec()).unwrap();
    assert_eq!(pbo.entry_names().collect::<Vec<_>>(), ["a\\b.txt"]);
    for bad in ["a\\", "a\\\\b", "a//b", "a/"] {
        assert!(PBO::new().insert_entry(bad, vec![]).is_err(), "{bad:?}");
    }
}

#[test]
fn removing_a_missing_entry_changes_nothing() {
    let mut pbo = PBO::read(&mut fs::File::open(sign_dir().join("testsig.pbo")).unwrap()).unwrap();
    let before: Vec<String> = pbo.entry_names().map(str::to_owned).collect();
    assert!(pbo.remove_entry("nope").is_none());
    assert!(pbo.checksum().is_some());
    assert_eq!(
        pbo.entry_names().map(str::to_owned).collect::<Vec<_>>(),
        before
    );
}
#[cfg(feature = "sign")]
#[test]
fn private_key_debug_has_no_key_material() {
    let text = format!("{:?}", armk());
    assert!(text.contains("armk"), "{text}");
    let d = {
        let bytes = fs::read(sign_dir().join("armk.biprivatekey")).unwrap();
        // The private key file holds d, p, q, ... as raw bytes; none of their hex may show up.
        bytes[bytes.len() - 32..]
            .iter()
            .fold(String::new(), |mut acc, b| {
                use std::fmt::Write as _;
                let _ = write!(acc, "{b:02x}"); // writing to a String cannot fail
                acc
            })
    };
    assert!(!text.to_lowercase().contains(&d), "{text}");
    for field in ["dmp1", "dmq1", "iqmp", "d:", "p:", "q:"] {
        assert!(!text.contains(field), "{text}");
    }
}
