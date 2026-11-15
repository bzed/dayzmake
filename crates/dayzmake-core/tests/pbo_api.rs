//! The packing and unpacking options of the PBO API.
use std::fs;
use std::io::Cursor;
use std::path::Path;

use dayzmake_core::pbo::{PBO, PackOptions, UnpackOptions, unpack};
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
    let got: Vec<(&str, &str)> = pbo
        .header_extensions
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(got, [("prefix", "a"), ("product", "y"), ("version", "1")]);
}

#[test]
fn pack_options_exclude_skips_files() {
    let tmp = addon();
    let mut options = PackOptions::pack();
    options.exclude = vec!["*.txt".to_string()];
    let pbo = read(&pack_to_bytes(&tmp.path().join("addon"), &options));
    assert!(!pbo.files.contains_key("a.txt"));
    assert!(pbo.files.contains_key("b.c"));
}

#[test]
fn build_option_rapifies_config_cpp() {
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join("config.cpp"), "class CfgPatches {};\n").unwrap();
    let pbo = read(&pack_to_bytes(&root, &PackOptions::build()));
    let config = pbo.files.get("config.bin").expect("config.bin entry");
    assert!(config.get_ref().starts_with(b"\0raP"));
    assert!(!pbo.files.contains_key("config.cpp"));

    let packed = read(&pack_to_bytes(&root, &PackOptions::pack()));
    assert!(packed.files.contains_key("config.cpp"));
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
    let mut pbo = PBO::from_directory(
        &tmp.path().join("addon"),
        &PackOptions::pack(),
        None,
        &mut Warnings::new(),
    )
    .unwrap();
    pbo.files.insert(
        "..\\evil.txt".to_string(),
        Cursor::new(b"evil".to_vec().into_boxed_slice()),
    );
    let mut bytes = Vec::new();
    pbo.write(&mut bytes).unwrap();

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
