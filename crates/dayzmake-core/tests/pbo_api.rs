//! The packing and unpacking options of the PBO API.
mod support;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use dayzmake_core::pbo::{PBO, PackOptions, TolerantRead, UnpackOptions, unpack};
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
    fs::write(tmp.path().join("addon").join("$PBOPREFIX$"), "prefix=a\n").unwrap();
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
    assert_eq!(
        err.to_string(),
        format!(
            "Target folder \"{}\" already contains files of this PBO (first: \"$PBOPREFIX$\"). Use --force to overwrite.",
            target.display()
        )
    );

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

/// #72: `pack` never binarizes, so it must not claim that binarization was skipped; `build` without a binarizer
/// warns outside Windows (there is a native binarize.exe on Windows).
#[test]
fn only_build_warns_about_models_that_are_not_binarized() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("addon");
    fs::create_dir_all(&root).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mlod/small.p3d"),
        root.join("small.p3d"),
    )
    .unwrap();
    let warned = |options: &PackOptions| {
        let mut warnings = Warnings::new();
        PBO::from_directory(&root, options, None, &mut warnings).unwrap();
        warnings
            .warnings()
            .iter()
            .any(|w| w.name == Some("non-windows-binarization"))
    };
    assert!(!warned(&PackOptions::pack()), "pack warned");
    assert_eq!(warned(&PackOptions::build()), !cfg!(windows));
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

#[test]
fn tolerant_selection_keeps_the_good_entries_next_to_a_truncated_one() {
    let mut bytes = support::raw_pbo(
        &[
            ("a.txt", b"aaaa"),
            ("b.txt", b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ],
        &[],
    );
    bytes.truncate(bytes.len() - 30);
    assert!(PBO::read_entries(&mut Cursor::new(&bytes), |_| true).is_err());
    let sel = PBO::read_entries_tolerant(
        &mut Cursor::new(&bytes),
        |_| true,
        &TolerantRead::new(1 << 20),
    )
    .unwrap();
    let entries = sel.into_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].data().unwrap(), b"aaaa");
    let err = entries[1].data().unwrap_err().to_string();
    assert!(err.contains("truncated"), "{err}");
    assert!(!entries[1].is_over_limit());
    assert_eq!(
        entries.into_iter().next().unwrap().into_data().unwrap(),
        b"aaaa"
    );
}

#[test]
fn tolerant_selection_stops_reading_at_the_byte_limit() {
    let bytes = support::raw_pbo(
        &[
            ("a.txt", &[b'a'; 10]),
            ("b.txt", &[b'b'; 10]),
            ("c.txt", &[b'c'; 10]),
        ],
        &[],
    );
    let sel =
        PBO::read_entries_tolerant(&mut Cursor::new(&bytes), |_| true, &TolerantRead::new(15))
            .unwrap();
    let entries = sel.entries();
    assert_eq!(entries[0].data().unwrap(), &[b'a'; 10]);
    assert!(entries[1].is_over_limit() && entries[1].data().is_err());
    assert!(entries[2].is_over_limit());
    // a later entry that would fit is not read either once the limit was reached
    let bytes = support::raw_pbo(
        &[
            ("a.txt", &[b'a'; 10]),
            ("b.txt", &[b'b'; 10]),
            ("c.txt", b"c"),
        ],
        &[],
    );
    let sel =
        PBO::read_entries_tolerant(&mut Cursor::new(&bytes), |_| true, &TolerantRead::new(12))
            .unwrap();
    assert!(sel.entries()[0].data().is_ok());
    assert!(sel.entries()[1].is_over_limit());
    assert!(sel.entries()[2].is_over_limit() && sel.entries()[2].data().is_err());
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

#[test]
fn unpack_conflict_writes_nothing() {
    let tmp = addon();
    fs::write(tmp.path().join("addon").join("$PBOPREFIX$"), "prefix=a\n").unwrap();
    let bytes = pack_to_bytes(&tmp.path().join("addon"), &PackOptions::pack());
    let target = tmp.path().join("out");
    fs::create_dir_all(&target).unwrap();
    // b.c is the last entry written, so a write-as-you-go check would already have written the
    // prefix file and a.txt
    fs::write(target.join("b.c"), "old").unwrap();
    let err = unpack(
        &mut Cursor::new(&bytes),
        &target,
        &UnpackOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "Target folder \"{}\" already contains files of this PBO (first: \"b.c\"). Use --force to overwrite.",
            target.display()
        )
    );
    assert_eq!(fs::read_to_string(target.join("b.c")).unwrap(), "old");
    assert_eq!(walk(&target), 1, "files or folders were created");
}

/// Counts the files and folders below `dir`.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn walk(dir: &Path) -> usize {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            1 + if p.is_dir() { walk(&p) } else { 0 }
        })
        .sum()
}

#[test]
fn unpack_force_overwrites() {
    let tmp = addon();
    fs::write(tmp.path().join("addon").join("$PBOPREFIX$"), "prefix=a\n").unwrap();
    let bytes = pack_to_bytes(&tmp.path().join("addon"), &PackOptions::pack());
    let target = tmp.path().join("out");
    let mut warnings = Warnings::new();
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &UnpackOptions::default(),
        &mut warnings,
    )
    .unwrap();
    fs::write(target.join("a.txt"), "old").unwrap();
    fs::write(target.join("$PBOPREFIX$"), "prefix=other\n").unwrap();
    let mut force = UnpackOptions::default();
    force.force = true;
    unpack(&mut Cursor::new(&bytes), &target, &force, &mut warnings).unwrap();
    assert_eq!(fs::read_to_string(target.join("a.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(target.join("b.c")).unwrap(), "b");
    assert_eq!(
        fs::read_to_string(target.join("$PBOPREFIX$")).unwrap(),
        "prefix=a\n"
    );
}

/// Unpacks twice into `target` and returns the conflict message of the second run.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn conflict_message_for_folder(name: &str) -> String {
    let tmp = addon();
    let bytes = pack_to_bytes(&tmp.path().join("addon"), &PackOptions::pack());
    let target = tmp.path().join(name);
    let options = UnpackOptions::default();
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap();
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap_err()
    .to_string()
}

#[test]
fn unpack_conflict_message_escapes_c1_control_characters() {
    // U+009B is a control character but a valid file name character on every platform.
    let err = conflict_message_for_folder("o\u{9b}ut");
    assert!(err.contains("o\\u{9b}ut"), "{err}");
    assert!(!err.contains('\u{9b}'), "{err}");
}

#[cfg(unix)]
#[test]
fn unpack_conflict_message_escapes_control_characters() {
    // NTFS rejects characters 1 to 31 in names, so this one is Unix only.
    let err = conflict_message_for_folder("o\x1but");
    assert!(err.contains("o\\u{1b}ut"), "{err}");
    assert!(!err.contains('\x1b'), "{err}");
}

#[test]
fn unpack_skips_entry_names_with_c1_control_characters() {
    let tmp = tempfile::tempdir().unwrap();
    let bytes = support::raw_pbo(&[("a\u{9b}b.txt", b"x"), ("ok.txt", b"y")], &[]);
    let target = tmp.path().join("out");
    let mut warnings = Warnings::new();
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &UnpackOptions::default(),
        &mut warnings,
    )
    .unwrap();
    assert!(target.join("ok.txt").exists());
    let on_disk: Vec<_> = fs::read_dir(&target)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(on_disk.len(), 1, "{on_disk:?}");
    let skipped: Vec<_> = warnings
        .warnings()
        .iter()
        .filter(|w| w.message.contains("a\\u{9b}b.txt"))
        .collect();
    assert_eq!(skipped.len(), 1, "{:?}", warnings.warnings());
    assert!(
        !skipped[0].message.contains('\u{9b}'),
        "{}",
        skipped[0].message
    );
}

#[test]
fn unpack_conflict_with_use_prefix_names_the_prefix_folder() {
    let tmp = addon();
    fs::write(
        tmp.path().join("addon").join("$PBOPREFIX$"),
        "prefix=x\\y\n",
    )
    .unwrap();
    let bytes = pack_to_bytes(&tmp.path().join("addon"), &PackOptions::pack());
    let target = tmp.path().join("out");
    let mut options = UnpackOptions::default();
    options.use_prefix = true;
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
    assert_eq!(
        err.to_string(),
        format!(
            "Target folder \"{}\" already contains files of this PBO (first: \"$PBOPREFIX$\"). Use --force to overwrite.",
            target.join("x").join("y").display()
        )
    );
}

#[test]
fn unpack_into_a_non_empty_folder_without_conflicts_works() {
    let tmp = addon();
    let bytes = pack_to_bytes(&tmp.path().join("addon"), &PackOptions::pack());
    let target = tmp.path().join("out");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("unrelated.txt"), "x").unwrap();
    unpack(
        &mut Cursor::new(&bytes),
        &target,
        &UnpackOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    assert!(target.join("unrelated.txt").exists());
    assert!(target.join("a.txt").exists());
}

const SECOND: &[u8] = b"compressed content, long enough for two groups";

#[expect(clippy::unwrap_used, reason = "test helper")]
fn with_compressed_entry() -> Vec<u8> {
    let stored = support::lzss_literals(SECOND);
    support::raw_pbo_with_headers(
        &[
            ("a.txt", b"plain", 0, 5),
            (
                "b.txt",
                &stored,
                support::CPRS,
                u32::try_from(SECOND.len()).unwrap(),
            ),
        ],
        &[],
    )
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn rewritten(pbo: &PBO) -> Vec<u8> {
    let mut out = Vec::new();
    pbo.write(&mut out).unwrap();
    out
}

#[test]
fn writing_a_read_pbo_keeps_compressed_entries() {
    let pbo = read(&with_compressed_entry());
    assert_eq!(pbo.file_data("b.txt").unwrap().unwrap(), SECOND);
    let back = read(&rewritten(&pbo));
    assert_eq!(back.file_data("b.txt").unwrap().unwrap(), SECOND);
    assert_eq!(back.file_data("a.txt").unwrap().unwrap(), b"plain");
    let header = back
        .entry_headers()
        .into_iter()
        .find(|h| h.name == "b.txt")
        .unwrap();
    assert_eq!(header.packing_method, support::CPRS);
    assert_eq!(header.original_size as usize, SECOND.len());
}

#[test]
fn unknown_packing_method_survives_a_rewrite() {
    const ENCR: u32 = 0x456e_6372;
    let pbo = read(&support::raw_pbo_with_headers(
        &[
            ("a.txt", b"plain", 0, 5),
            ("b.bin", b"0123456789", ENCR, 10),
        ],
        &[],
    ));
    let back = read(&rewritten(&pbo));
    let header = back
        .entry_headers()
        .into_iter()
        .find(|h| h.name == "b.bin")
        .unwrap();
    assert_eq!(header.packing_method, ENCR);
    assert_eq!(header.original_size, 10);
    assert_eq!(header.data_size, 10);
    assert_eq!(back.file_data("b.bin").unwrap().unwrap(), b"0123456789");
    assert_eq!(back.file_data("a.txt").unwrap().unwrap(), b"plain");
}

#[test]
fn replacing_an_unknown_method_entry_stores_it_with_method_0() {
    const ENCR: u32 = 0x456e_6372;
    let mut pbo = read(&support::raw_pbo_with_headers(
        &[("b.bin", b"0123456789", ENCR, 10)],
        &[],
    ));
    pbo.insert_entry("b.bin", b"new".to_vec()).unwrap();
    let back = read(&rewritten(&pbo));
    let header = back
        .entry_headers()
        .into_iter()
        .find(|h| h.name == "b.bin")
        .unwrap();
    assert_eq!(header.packing_method, 0);
    assert_eq!(back.file_data("b.bin").unwrap().unwrap(), b"new");
}

#[test]
fn modifying_a_read_pbo_keeps_untouched_compressed_entries() {
    let mut pbo = read(&with_compressed_entry());
    pbo.insert_entry("c.txt", b"new".to_vec()).unwrap();
    let back = read(&rewritten(&pbo));
    assert_eq!(back.file_data("b.txt").unwrap().unwrap(), SECOND);
    assert_eq!(back.file_data("c.txt").unwrap().unwrap(), b"new");
}

#[test]
fn replacing_a_compressed_entry_stores_it_uncompressed() {
    let mut pbo = read(&with_compressed_entry());
    pbo.insert_entry("b.txt", b"replaced".to_vec()).unwrap();
    let back = read(&rewritten(&pbo));
    assert_eq!(back.file_data("b.txt").unwrap().unwrap(), b"replaced");
    let header = back
        .entry_headers()
        .into_iter()
        .find(|h| h.name == "b.txt")
        .unwrap();
    assert_eq!(header.packing_method, 0);
}

#[cfg(feature = "sign")]
#[test]
fn signature_of_a_rewritten_read_pbo_verifies() {
    let key = armk();
    let written = read(&rewritten(&read(&with_compressed_entry())));
    // The old write kept the compressed bytes under a stored header, so the data read back
    // was the compressed form, not the original.
    assert_eq!(written.file_data("b.txt").unwrap().unwrap(), SECOND);
    let signature = key.sign(&written, BISignVersion::V3).unwrap();
    key.to_public_key()
        .unwrap()
        .verify(&read(&rewritten(&written)), &signature)
        .unwrap();
}

#[test]
fn duplicate_entry_names_use_the_header_of_the_kept_value() {
    let second = b"second!!";
    let stored = support::lzss_literals(second);
    let bytes = support::raw_pbo_with_headers(
        &[
            ("a.txt", b"first", 0, 5),
            ("a.txt", &stored, support::CPRS, 8),
        ],
        &[],
    );
    let pbo = read(&bytes);
    assert_eq!(pbo.file_data("a.txt").unwrap().unwrap(), second);
    let back = read(&rewritten(&pbo));
    assert_eq!(back.file_data("a.txt").unwrap().unwrap(), second);
}

#[test]
fn duplicate_names_compressed_first_stored_last_survive_a_rewrite() {
    let stored = support::lzss_literals(b"first!!!");
    let bytes = support::raw_pbo_with_headers(
        &[
            ("a.txt", &stored, support::CPRS, 8),
            ("a.txt", b"second", 0, 6),
        ],
        &[],
    );
    let pbo = read(&bytes);
    assert_eq!(pbo.file_data("a.txt").unwrap().unwrap(), b"second");
    let back = read(&rewritten(&pbo));
    assert_eq!(back.file_data("a.txt").unwrap().unwrap(), b"second");
}

#[test]
fn a_header_extension_with_nul_is_rejected_and_nothing_written() {
    let mut pbo = PBO::new();
    pbo.set_header_extension("k", "a\0b");
    let mut out = Vec::new();
    let msg = pbo.write(&mut out).unwrap_err().to_string();
    assert!(out.is_empty(), "{msg}");
    assert!(msg.contains("NUL") && msg.contains("\"k\""), "{msg}");
}

#[test]
fn a_prefix_with_nul_is_rejected_and_nothing_written() {
    let mut pbo = PBO::new();
    pbo.set_header_extension("prefix", "a\0b");
    let mut out = Vec::new();
    let msg = pbo.write(&mut out).unwrap_err().to_string();
    assert!(out.is_empty(), "{msg}");
    assert!(msg.contains("Header extension \"prefix\""), "{msg}");
}

/// Packs `root` with `exclude` as the own output and returns the entry names.
fn packed_names_excluding(root: &Path, exclude: &Path) -> Vec<String> {
    let mut options = PackOptions::pack();
    options.exclude_paths = vec![exclude.to_path_buf()];
    let bytes = pack_to_bytes(root, &options);
    read(&bytes).entry_names().map(str::to_owned).collect()
}

#[cfg(unix)]
#[test]
fn own_output_is_excluded_by_identity_through_a_hard_link() {
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join("out.pbo"), "old").unwrap();
    fs::hard_link(root.join("out.pbo"), root.join("alias.pbo")).unwrap();
    let names = packed_names_excluding(&root, &root.join("out.pbo"));
    assert!(!names.iter().any(|n| n.contains("alias")), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("out.pbo")), "{names:?}");
    assert!(names.iter().any(|n| n == "a.txt"), "{names:?}");
}

#[cfg(unix)]
#[test]
fn own_output_is_excluded_through_a_symlink_with_a_different_spelling() {
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join("out.pbo"), "old").unwrap();
    std::os::unix::fs::symlink("out.pbo", root.join("OUT.PBO")).unwrap();
    let names = packed_names_excluding(&root, &root.join("out.pbo"));
    assert!(
        !names.iter().any(|n| n.eq_ignore_ascii_case("out.pbo")),
        "{names:?}"
    );
}

#[test]
fn own_output_is_excluded_whatever_the_case_of_the_given_path() {
    // the repro of #37: `build m M\OUT.PBO` with an existing `m\out.pbo`. Only meaningful on a case-insensitive
    // file system (Windows, macOS default); on others the probe below fails and the test returns.
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join("out.pbo"), "old").unwrap();
    let upper = root.join("OUT.PBO");
    if !upper.exists() {
        return;
    }
    let names = packed_names_excluding(&root, &upper);
    assert!(
        !names.iter().any(|n| n.eq_ignore_ascii_case("out.pbo")),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n == "a.txt"), "{names:?}");
}

#[test]
fn stale_temp_file_of_the_target_is_skipped_whatever_its_case() {
    let tmp = addon();
    let root = tmp.path().join("addon");
    fs::write(root.join(".OUT.PBO.dayzmake-tmp-1-0"), "stale").unwrap();
    let names = packed_names_excluding(&root, &root.join("out.pbo"));
    assert!(
        !names.iter().any(|n| n.contains("dayzmake-tmp")),
        "{names:?}"
    );
}

/// The files below `dir`, as sorted `/`-separated relative paths.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn files_below(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn unpack_filtered(exclude: &[&str], only: &[&str]) -> Vec<String> {
    let bytes = support::raw_pbo(
        &[
            ("scripts\\Game.C", b"c"),
            ("scripts\\x.c", b"c"),
            ("model.p3d", b"p"),
            ("config.bin", b"b"),
            ("data\\x.paa", b"t"),
        ],
        &[],
    );
    let tmp = tempfile::tempdir().unwrap();
    let mut options = UnpackOptions::default();
    options.exclude = exclude.iter().map(ToString::to_string).collect();
    options.only = only.iter().map(ToString::to_string).collect();
    unpack(
        &mut Cursor::new(&bytes),
        tmp.path(),
        &options,
        &mut Warnings::new(),
    )
    .unwrap();
    files_below(tmp.path())
}

#[test]
fn unpack_only_writes_the_matching_entries() {
    assert_eq!(
        unpack_filtered(&[], &["*.c"]),
        ["scripts/Game.C", "scripts/x.c"]
    );
    assert_eq!(
        unpack_filtered(&[], &["*.c", "*config.bin"]),
        ["config.bin", "scripts/Game.C", "scripts/x.c"]
    );
}

#[test]
fn unpack_exclude_leaves_out_the_matching_entries() {
    assert_eq!(
        unpack_filtered(&["*.p3d"], &[]),
        ["config.bin", "data/x.paa", "scripts/Game.C", "scripts/x.c"]
    );
}

#[test]
fn unpack_exclude_wins_over_only() {
    assert_eq!(
        unpack_filtered(&["scripts/*"], &["*.c", "*.p3d"]),
        ["model.p3d"]
    );
}

#[test]
fn unpack_conflict_check_only_covers_the_selected_entries() {
    let bytes = support::raw_pbo(&[("a.c", b"c"), ("b.p3d", b"p")], &[]);
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("b.p3d"), "mine").unwrap();
    let mut options = UnpackOptions::default();
    options.only = vec!["*.c".to_string()];
    unpack(
        &mut Cursor::new(&bytes),
        tmp.path(),
        &options,
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(tmp.path().join("b.p3d")).unwrap(),
        "mine"
    );
}

#[test]
fn raw_bytes_and_timestamps_of_a_read_pbo_are_available() {
    let bytes = support::raw_pbo_bytes(
        &[(b"a\xffb.txt", b"x", 1_700_000_000)],
        &[(b"prefix", b"scripts\n\xff")],
    );
    let pbo = read(&bytes);
    assert_eq!(
        pbo.header_extensions_bytes(),
        vec![(b"prefix".to_vec(), b"scripts\n\xff".to_vec())]
    );
    let header = &pbo.entry_headers()[0];
    assert_eq!(header.name_bytes, b"a\xffb.txt");
    assert_eq!(header.timestamp, 1_700_000_000);
}

#[test]
fn raw_bytes_keep_utf8_and_windows_1252_apart() {
    // UTF-8 "é" (c3 a9) and the byte e9 (windows-1252 "é") decode to the same string
    let bytes = support::raw_pbo_bytes(
        &[(b"\xc3\xa9.txt", b"x", 0), (b"\xe9.txt", b"y", 0)],
        &[(b"prefix", b"\xc3\xa9"), (b"other", b"\xe9")],
    );
    let pbo = read(&bytes);
    assert_eq!(
        pbo.header_extensions_bytes(),
        vec![
            (b"prefix".to_vec(), b"\xc3\xa9".to_vec()),
            (b"other".to_vec(), b"\xe9".to_vec()),
        ]
    );
    let names: Vec<Vec<u8>> = pbo
        .entry_headers()
        .into_iter()
        .map(|h| h.name_bytes)
        .collect();
    assert_eq!(names, vec![b"\xc3\xa9.txt".to_vec(), b"\xe9.txt".to_vec()]);
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn rewrite(bytes: &[u8]) -> PBO {
    let mut out = Vec::new();
    read(bytes).write(&mut out).unwrap();
    read(&out)
}

#[test]
fn write_keeps_utf8_and_windows_1252_extensions_apart() {
    let bytes = support::raw_pbo_bytes(
        &[(b"a.txt", b"x", 0)],
        &[(b"prefix", b"\xc3\xa9"), (b"other", b"\xe9")],
    );
    let again = rewrite(&bytes);
    assert_eq!(
        again.header_extensions_bytes(),
        vec![
            (b"prefix".to_vec(), b"\xc3\xa9".to_vec()),
            (b"other".to_vec(), b"\xe9".to_vec()),
        ]
    );
    // and the other way round
    let bytes = support::raw_pbo_bytes(
        &[(b"a.txt", b"x", 0)],
        &[(b"prefix", b"\xe9"), (b"other", b"\xc3\xa9")],
    );
    assert_eq!(
        rewrite(&bytes).header_extensions_bytes(),
        vec![
            (b"prefix".to_vec(), b"\xe9".to_vec()),
            (b"other".to_vec(), b"\xc3\xa9".to_vec()),
        ]
    );
}

#[test]
fn write_keeps_the_stored_name_bytes_of_the_entry_that_survives() {
    // both names decode to the same string, so they are one entry; the last one wins, with its data
    for (first, second) in [
        (&b"\xe9.txt"[..], &b"\xc3\xa9.txt"[..]),
        (&b"\xc3\xa9.txt"[..], &b"\xe9.txt"[..]),
    ] {
        let bytes = support::raw_pbo_bytes(&[(first, b"x", 0), (second, b"y", 0)], &[]);
        let again = rewrite(&bytes);
        let headers = again.entry_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].name_bytes, second);
        assert_eq!(again.entries().next().unwrap().1, b"y");
    }
}

#[cfg(feature = "sign")]
#[test]
fn colliding_names_sign_and_verify_after_a_rewrite() {
    let bytes = support::raw_pbo_bytes(
        &[(b"\xe9.txt", b"x", 0), (b"\xc3\xa9.txt", b"y", 0)],
        &[(b"prefix", b"\xc3\xa9"), (b"other", b"\xe9")],
    );
    let pbo = rewrite(&bytes);
    let key = armk();
    let signature = key.sign(&pbo, BISignVersion::V3).unwrap();
    let public = key.to_public_key().unwrap();
    public.verify(&pbo, &signature).unwrap();
    let again = PBO::read(&mut pbo.to_cursor().unwrap()).unwrap();
    assert_eq!(sign_bytes(&key, &again), sign_bytes(&key, &pbo));
    public.verify(&again, &signature).unwrap();
}

#[test]
fn set_header_extension_keeps_the_stored_bytes_of_the_others() {
    let bytes = support::raw_pbo_bytes(
        &[(b"a.txt", b"x", 0)],
        &[(b"prefix", b"\xe9"), (b"other", b"\xc3\xa9")],
    );
    let mut pbo = read(&bytes);
    pbo.set_header_extension("other", "x");
    pbo.set_header_extension("added", "y");
    let mut out = Vec::new();
    pbo.write(&mut out).unwrap();
    assert_eq!(
        read(&out).header_extensions_bytes(),
        vec![
            (b"prefix".to_vec(), b"\xe9".to_vec()),
            (b"other".to_vec(), b"x".to_vec()),
            (b"added".to_vec(), b"y".to_vec()),
        ]
    );
}

#[test]
fn entry_filter_normalises_patterns_and_names() {
    use dayzmake_core::pbo::EntryFilter;
    let filter = EntryFilter::new(vec!["Scripts\\*.C".into()], vec!["*\\skip.c".into()]);
    assert!(filter.matches("scripts/a.c"));
    assert!(filter.matches("SCRIPTS\\A.C"));
    assert!(!filter.matches("scripts/skip.c"));
    assert!(!filter.matches("data/a.c"));
    assert!(EntryFilter::default().matches("anything"));
}

/// #92: the prefix hash input is the prefix as stored, by position. A UTF-8 `é` prefix next to another
/// extension holding the windows-1252 byte `e9` (both decode to `é`) must hash `c3 a9`, not `e9`. Signing
/// and verifying with the same code cannot show that, so the expected hash is computed here.
#[cfg(feature = "sign")]
#[test]
fn signature_hashes_the_stored_prefix_bytes_of_colliding_extensions() {
    use num_bigint::BigUint;
    use sha1::{Digest, Sha1};

    let bytes = support::raw_pbo_bytes(
        &[(b"a.txt", b"x", 0)],
        &[(b"prefix", b"\xc3\xa9"), (b"other", b"\xe9")],
    );
    // a rewrite gives a PBO with a valid SHA-1 trailer
    let pbo = rewrite(&bytes);
    let written = pack_bytes(&pbo);
    assert_eq!(pbo.header_extensions_bytes()[0].1, b"\xc3\xa9");

    let signed = sign_bytes(&armk(), &pbo);
    // armk is 1024 bits: after the key name come a 24 byte header and n, a length, sig1, a version and a
    // length, then sig2
    let name_len = signed.iter().position(|&b| b == 0).unwrap() + 1;
    let n_at = name_len + 24;
    let sig2_at = n_at + 128 + 4 + 128 + 8;
    let n = BigUint::from_bytes_le(&signed[n_at..n_at + 128]);
    let sig2 = BigUint::from_bytes_le(&signed[sig2_at..sig2_at + 128]);
    let block = sig2.modpow(&BigUint::from(65537u32), &n).to_bytes_be();

    let mut namehash = Sha1::new();
    namehash.update(b"a.txt");
    let mut hash2 = Sha1::new();
    hash2.update(&written[written.len() - 20..]);
    hash2.update(namehash.finalize());
    hash2.update(b"\xc3\xa9\\");
    // the PKCS#1 block ends with the SHA-1 hash
    assert_eq!(block[block.len() - 20..], hash2.finalize()[..]);
}

#[test]
fn insert_entry_over_a_windows_1252_name_stores_the_new_names_bytes() {
    let mut pbo = PBO::read(&mut fs::File::open(sign_dir().join("nonutf8.pbo")).unwrap()).unwrap();
    let (name, _) = pbo
        .entries()
        .find(|(n, _)| pbo.name_bytes(n).contains(&0x97))
        .unwrap();
    let name = name.to_string();
    // the lossily decoded name is valid UTF-8 now; the entry takes its bytes, not the old 0x97
    pbo.insert_entry(name.clone(), b"new".to_vec()).unwrap();
    assert_eq!(pbo.name_bytes(&name), name.as_bytes());
    let again = rewrite(&pack_bytes(&pbo));
    let headers = again.entry_headers();
    let header = headers.iter().find(|h| h.name_bytes == name.as_bytes());
    assert!(header.is_some(), "{headers:?}");
    assert!(!headers.iter().any(|h| h.name_bytes.contains(&0x97)));
}
