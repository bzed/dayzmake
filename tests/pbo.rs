//! PBO packing, unpacking, signing and verification tests.
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use dayzmake::pbo::PBO;
use dayzmake::sign::{BISignVersion, cmd_sign, cmd_verify};

#[expect(clippy::unwrap_used, reason = "test helper")]
fn run_dayzmake(args: &[&str], cwd: &Path) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn sign_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bi/sign")
}

/// Signs `<pbo>` with the fixture key; returns the produced bisign bytes.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn sign_fixture(pbo: &str) -> Vec<u8> {
    let tmp = tempfile::tempdir().unwrap();
    let sig = tmp.path().join("out.bisign");
    cmd_sign(
        sign_dir().join("armk.biprivatekey"),
        sign_dir().join(pbo),
        Some(sig.clone()),
        BISignVersion::V3,
        true,
    )
    .unwrap();
    fs::read(sig).unwrap()
}

#[test]
fn sign_matches_bi_utf8() {
    assert_eq!(
        sign_fixture("testsig.pbo"),
        fs::read(sign_dir().join("testsig.pbo.armk.bisign")).unwrap()
    );
}

#[test]
fn sign_matches_bi_non_utf8() {
    assert_eq!(
        sign_fixture("nonutf8.pbo"),
        fs::read(sign_dir().join("nonutf8.pbo.armk.bisign")).unwrap()
    );
}

#[test]
fn verify_bi_signature_non_utf8() {
    cmd_verify(
        sign_dir().join("armk.bikey"),
        sign_dir().join("nonutf8.pbo"),
        Some(sign_dir().join("nonutf8.pbo.armk.bisign")),
    )
    .unwrap();
}

#[test]
fn read_non_utf8_name() {
    let pbo = PBO::read(&mut File::open(sign_dir().join("nonutf8.pbo")).unwrap()).unwrap();
    assert!(
        pbo.files.contains_key("data\\a\u{2014}b.txt"),
        "{:?}",
        pbo.files.keys().collect::<Vec<_>>()
    );
}

#[test]
fn roundtrip_non_utf8_name() {
    let original = fs::read(sign_dir().join("nonutf8.pbo")).unwrap();
    let pbo = PBO::read(&mut &original[..]).unwrap();
    let mut out = Vec::new();
    pbo.write(&mut out).unwrap();
    assert_eq!(out, original);
}

#[test]
fn cli_unpack_non_utf8_name() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = sign_dir().join("nonutf8.pbo");
    let (code, err) = run_dayzmake(&["unpack", pbo.to_str().unwrap(), "out"], tmp.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("non-utf8-filename"), "{}", err);
    assert!(Path::new(&tmp.path().join("out/data/a\u{2014}b.txt")).exists());
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn prefix_of(content: &str) -> PBO {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("mod");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("$PBOPREFIX$"), content).unwrap();
    PBO::from_directory(root, false, &[], &[]).unwrap()
}

#[test]
fn prefix_trailing_slash_stripped() {
    assert_eq!(
        prefix_of("my\\mod\\").header_extensions["prefix"],
        "my\\mod"
    );
}

#[test]
fn prefix_normalized_crlf_and_leading_slash() {
    assert_eq!(
        prefix_of("\\my\\mod\\\\\r\n").header_extensions["prefix"],
        "my\\mod"
    );
}

#[test]
fn prefix_key_value_form() {
    let pbo = prefix_of("prefix=my\\mod\\\nversion=a=b\n");
    assert_eq!(pbo.header_extensions["prefix"], "my\\mod");
    assert_eq!(pbo.header_extensions["version"], "a=b");
}

#[test]
fn roundtrip_non_utf8_prefix() {
    // same length as "mymod", so no offsets change; 0x97 is not valid UTF-8
    let mut original = fs::read(sign_dir().join("nonutf8.pbo")).unwrap();
    let at = original.windows(5).position(|w| w == b"mymod").unwrap();
    original[at..at + 5].copy_from_slice(b"my\x97od");
    let pbo = PBO::read(&mut &original[..]).unwrap();
    assert_eq!(pbo.header_extensions["prefix"], "my\u{2014}od\\testsig");
    let mut out = Vec::new();
    pbo.write(&mut out).unwrap();
    // everything but the SHA-1 trailer, which the edit made stale
    assert_eq!(out[..out.len() - 20], original[..original.len() - 20]);
}

/// Truncated PBOs and garbage must error out, not panic.
#[test]
fn garbage_is_an_error_not_a_panic() {
    let original = fs::read(sign_dir().join("testsig.pbo")).unwrap();

    // no truncation of a valid PBO may panic
    for len in 0..original.len() {
        let _ = PBO::read(&mut &original[..len]);
    }

    // garbage without headers is an error or at least not a panic; an empty
    // header list reads as an empty PBO, so use data that claims huge sizes
    let mut garbage = Vec::new();
    garbage.extend_from_slice(b"bigfile\0"); // entry name
    garbage.extend_from_slice(&0u32.to_le_bytes()); // packing method
    garbage.extend_from_slice(&0xffff_ffffu32.to_le_bytes()); // original size
    garbage.extend_from_slice(&0u32.to_le_bytes()); // reserved
    garbage.extend_from_slice(&0u32.to_le_bytes()); // timestamp
    garbage.extend_from_slice(&0xffff_ffffu32.to_le_bytes()); // data size
    let result = PBO::read(&mut &garbage[..]);
    assert!(result.is_err(), "huge entry sizes must be rejected");
}

/// Builds a PBO holding exactly `entries`, with an optional prefix header extension.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn crafted_pbo(entries: &[(&str, &[u8])], prefix: Option<&str>) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let mut pbo = PBO::from_directory(dir.path().to_path_buf(), false, &[], &[]).unwrap();
    pbo.files.clear();
    for (name, data) in entries {
        pbo.files.insert(
            (*name).to_string(),
            std::io::Cursor::new(data.to_vec().into_boxed_slice()),
        );
    }
    if let Some(prefix) = prefix {
        pbo.header_extensions
            .insert("prefix".to_string(), prefix.to_string());
    }
    pbo.to_cursor().unwrap().into_inner()
}

/// Unpacks `pbo` into `<tmp>/target`; returns (tmp, exit code, stderr).
#[expect(clippy::unwrap_used, reason = "test helper")]
fn unpack_crafted(pbo: &[u8], extra: &[&str]) -> (tempfile::TempDir, i32, String) {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("outside")).unwrap();
    fs::write(tmp.path().join("in.pbo"), pbo).unwrap();
    let mut args = vec!["unpack"];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["in.pbo", "target"]);
    let (code, err) = run_dayzmake(&args, tmp.path());
    (tmp, code, err)
}

#[test]
fn unpack_rejects_parent_dir_entry() {
    let pbo = crafted_pbo(&[("..\\outside\\evil.txt", b"x"), ("ok.txt", b"y")], None);
    let (tmp, code, err) = unpack_crafted(&pbo, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("unsafe-path"), "{err}");
    assert!(!tmp.path().join("outside/evil.txt").exists());
    assert!(tmp.path().join("target/ok.txt").exists());
}

#[test]
fn unpack_rejects_absolute_entry() {
    let pbo = crafted_pbo(
        &[("\\abs.txt", b"a"), ("/tmp/x", b"b"), ("C:\\x.txt", b"c")],
        None,
    );
    let (tmp, code, err) = unpack_crafted(&pbo, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(tmp.path().join("target/abs.txt").exists());
    assert!(tmp.path().join("target/tmp/x").exists());
    assert!(!tmp.path().join("target/C:").exists());
    assert!(!tmp.path().join("target/x.txt").exists());
    assert!(err.contains("unsafe-path"), "{err}");
}

#[test]
fn unpack_rejects_dot_space_components() {
    let pbo = crafted_pbo(
        &[
            ("a\\...\\b", b"1"),
            ("a\\. \\b", b"2"),
            ("a\\.\\b", b"3"),
            ("ok.txt", b"4"),
        ],
        None,
    );
    let (tmp, code, err) = unpack_crafted(&pbo, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(!tmp.path().join("target/a").exists());
    assert!(tmp.path().join("target/ok.txt").exists());
}

#[test]
fn unpack_use_prefix_rejects_dotdot_prefix() {
    let pbo = crafted_pbo(&[("a.txt", b"x")], Some("..\\..\\outside"));
    let (tmp, code, err) = unpack_crafted(&pbo, &["--use-prefix"]);
    assert_ne!(code, 0, "{err}");
    assert!(err.to_lowercase().contains("prefix"), "{err}");
    assert!(!tmp.path().join("outside/a.txt").exists());
    assert!(!tmp.path().join("target/a.txt").exists());
    assert!(!tmp.path().join("target").exists());
}

#[test]
fn unpack_use_prefix_extracts_below_prefix() {
    let pbo = crafted_pbo(&[("a\\b.c", b"x")], Some("my\\mod"));
    let (tmp, code, err) = unpack_crafted(&pbo, &["--use-prefix"]);
    assert_eq!(code, 0, "{err}");
    assert!(tmp.path().join("target/my/mod/a/b.c").exists());
}

/// An empty prefix means "no prefix": the files go straight into the target folder.
#[test]
fn unpack_use_prefix_with_empty_prefix_extracts_into_target() {
    for prefix in ["", "\\", "/"] {
        let pbo = crafted_pbo(&[("a\\b.c", b"x")], Some(prefix));
        let (tmp, code, err) = unpack_crafted(&pbo, &["--use-prefix"]);
        assert_eq!(code, 0, "{prefix:?}: {err}");
        assert!(tmp.path().join("target/a/b.c").exists(), "{prefix:?}");
    }
}

#[cfg(unix)]
#[test]
fn unpack_does_not_follow_symlinks() {
    let pbo = crafted_pbo(&[("link\\evil.txt", b"x")], None);
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("outside")).unwrap();
    fs::create_dir_all(tmp.path().join("target")).unwrap();
    std::os::unix::fs::symlink("../outside", tmp.path().join("target/link")).unwrap();
    fs::write(tmp.path().join("in.pbo"), &pbo).unwrap();
    let (code, err) = run_dayzmake(&["unpack", "-f", "in.pbo", "target"], tmp.path());
    assert_eq!(code, 0, "{err}");
    assert!(!tmp.path().join("outside/evil.txt").exists());
    assert!(err.contains("unsafe-path"), "{err}");
}

#[test]
fn unpack_prefix_file_strips_control_chars() {
    let pbo = crafted_pbo(&[("a.txt", b"x")], Some("x\nprefix=..\\evil"));
    let (tmp, code, err) = unpack_crafted(&pbo, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("unsafe-header"), "{err}");
    let text = fs::read_to_string(tmp.path().join("target/$PBOPREFIX$")).unwrap();
    assert_eq!(text.lines().count(), 1, "{text:?}");
    assert!(!text.lines().any(|l| l == "prefix=..\\evil"), "{text:?}");
}

#[test]
fn unpack_duplicate_target_warns() {
    let pbo = crafted_pbo(&[("A.txt", b"1"), ("a.txt", b"2")], None);
    let (tmp, code, err) = unpack_crafted(&pbo, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("duplicate-entry"), "{err}");
    assert_eq!(fs::read(tmp.path().join("target/A.txt")).unwrap(), b"1");
}

#[cfg(unix)]
#[test]
fn unpack_use_prefix_does_not_follow_symlinked_prefix() {
    let pbo = crafted_pbo(&[("a.txt", b"x")], Some("my\\mod"));
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("outside")).unwrap();
    fs::create_dir_all(tmp.path().join("target")).unwrap();
    std::os::unix::fs::symlink("../outside", tmp.path().join("target/my")).unwrap();
    fs::write(tmp.path().join("in.pbo"), &pbo).unwrap();
    let (code, err) = run_dayzmake(
        &["unpack", "-f", "--use-prefix", "in.pbo", "target"],
        tmp.path(),
    );
    assert_ne!(code, 0, "{err}");
    assert!(!tmp.path().join("outside/mod").exists());
    assert!(!tmp.path().join("outside/$PBOPREFIX$").exists());
}

#[test]
fn compressed_entry_size_bomb_is_an_error() {
    let mut pbo = crafted_pbo(&[("bomb.txt", &[0u8; 16])], None);
    let needle = b"bomb.txt\0";
    let at = pbo.windows(needle.len()).position(|w| w == needle).unwrap() + needle.len();
    pbo[at..at + 4].copy_from_slice(&0x4370_7273u32.to_le_bytes()); // packing: Cprs
    pbo[at + 4..at + 8].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes()); // original size
    pbo[at + 16..at + 20].copy_from_slice(&16u32.to_le_bytes()); // data size
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("in.pbo"), &pbo).unwrap();
    let (code, err) = run_dayzmake(&["cat", "in.pbo", "bomb.txt"], tmp.path());
    assert_ne!(code, 0, "{err}");
    assert!(code > 0, "the process must not be killed: {code}");
    assert!(err.contains("implausible"), "{err}");
}

/// Found by fuzzing: a compressed entry whose original size is about 2 GiB made `file_data`
/// allocate that much up front. It must fail without that allocation.
#[test]
fn file_data_does_not_trust_the_original_size() {
    let data = include_bytes!("fixtures/fuzz/pbo_huge_original_size.pbo");
    if let Ok(pbo) = PBO::read(&mut &data[..]) {
        for name in pbo.files.keys() {
            let _ = pbo.file_data(name);
        }
    }
}

#[test]
fn cat_missing_entry_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("in.pbo"),
        crafted_pbo(&[("a.txt", b"x")], None),
    )
    .unwrap();
    let (code, err) = run_dayzmake(&["cat", "in.pbo", "nope.txt"], tmp.path());
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("File \"nope.txt\" not found in PBO."), "{err}");
}

#[test]
fn inspect_and_cat_write_to_the_given_writer() {
    let pbo = crafted_pbo(&[("a.txt", b"hello")], Some("pre"));
    let mut out = Vec::new();
    dayzmake::pbo::cmd_inspect(&mut std::io::Cursor::new(&pbo), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("- prefix=pre") && text.contains("a.txt"),
        "{text}"
    );
    let mut out = Vec::new();
    dayzmake::pbo::cmd_cat(&mut std::io::Cursor::new(&pbo), &mut out, "a.txt").unwrap();
    assert_eq!(out, b"hello");
}

#[test]
fn read_headers_lists_names_without_the_data() {
    let pbo = crafted_pbo(&[("a.txt", b"hello"), ("sub\\b.c", b"x")], Some("pre"));
    let (ext, names) = PBO::read_headers(&mut &pbo[..]).unwrap();
    assert_eq!(ext.get("prefix").map(String::as_str), Some("pre"));
    assert_eq!(names, ["a.txt", "sub\\b.c"]);
    // Only the header block is needed: cut off right after it.
    let end = pbo.len() - 6 - 21;
    let (_, names) = PBO::read_headers(&mut &pbo[..end]).unwrap();
    assert_eq!(names.len(), 2);
}

#[test]
fn read_headers_rejects_hostile_input() {
    assert!(PBO::read_headers(&mut &b""[..]).is_err());
    assert!(PBO::read_headers(&mut &b"abc"[..]).is_err());
    // an endless name
    let long = vec![b'a'; 100_000];
    assert!(PBO::read_headers(&mut &long[..]).is_err());
    // too many entries: each header is a one byte name, a NUL and 20 bytes
    let mut many = Vec::new();
    for _ in 0..=(1 << 20) {
        many.extend_from_slice(b"a\0");
        many.extend_from_slice(&[0; 20]);
    }
    assert!(PBO::read_headers(&mut &many[..]).is_err());
}

/// A PBO with a "Cprs" entry (literal-only LZSS) next to a stored one, built by hand.
#[expect(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    reason = "test helper"
)]
fn pbo_with_compressed_entry(plain: &[u8]) -> Vec<u8> {
    let mut packed = Vec::new();
    for chunk in plain.chunks(8) {
        packed.push(((1u16 << chunk.len()) - 1) as u8);
        packed.extend_from_slice(chunk);
    }
    packed.extend_from_slice(&[0; 4]);
    let header = |out: &mut Vec<u8>, name: &[u8], method: u32, orig: u32, size: u32| {
        out.extend_from_slice(name);
        out.push(0);
        for v in [method, orig, 0, 0, size] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    };
    let mut out = Vec::new();
    header(&mut out, b"", 0x5665_7273, 0, 0);
    out.extend_from_slice(b"prefix\0pre\0\0");
    header(&mut out, b"a.txt", 0, 5, 5);
    let (orig, size) = (
        u32::try_from(plain.len()).unwrap(),
        u32::try_from(packed.len()).unwrap(),
    );
    header(&mut out, b"b.c", 0x4370_7273, orig, size);
    header(&mut out, b"", 0, 0, 0);
    out.extend_from_slice(b"hello");
    out.extend_from_slice(&packed);
    out.push(0);
    out.extend_from_slice(&[7; 20]);
    out
}

#[test]
fn read_entries_equals_read_for_stored_and_compressed() {
    let plain = b"class A {};\nclass B : A {};\n";
    let bytes = pbo_with_compressed_entry(plain);
    let full = PBO::read(&mut &bytes[..]).unwrap();
    assert_eq!(full.file_data("b.c").unwrap().unwrap(), plain);
    let sel = PBO::read_entries(&mut std::io::Cursor::new(&bytes), |n| {
        std::path::Path::new(n)
            .extension()
            .is_some_and(|e| e == "c")
            || n == "a.txt"
    })
    .unwrap();
    assert_eq!(sel.names, ["a.txt", "b.c"]);
    assert_eq!(
        sel.header_extensions.get("prefix").map(String::as_str),
        Some("pre")
    );
    assert_eq!(sel.entries.len(), 2);
    for (name, data) in sel.entries {
        assert_eq!(
            data.unwrap(),
            full.file_data(&name).unwrap().unwrap(),
            "{name}"
        );
    }
    // entries that are not wanted are not returned
    let sel = PBO::read_entries(&mut std::io::Cursor::new(&bytes), |n| n == "b.c").unwrap();
    assert_eq!(sel.entries.len(), 1);
}

#[test]
fn read_entries_rejects_truncated_and_lying_headers() {
    let bytes = pbo_with_compressed_entry(b"0123456789");
    for len in 0..bytes.len() - 21 {
        // never a panic; the data of a cut-off entry is an error
        let _ = PBO::read_entries(&mut std::io::Cursor::new(&bytes[..len]), |_| true);
    }
    // an entry whose data lies past the end of the file
    let cut = bytes.len() - 21 - 3;
    assert!(PBO::read_entries(&mut std::io::Cursor::new(&bytes[..cut]), |_| true).is_err());
    // a size field that claims 4 GB
    let mut lying = bytes.clone();
    let pos = lying.windows(6).position(|w| w == b"a.txt\0").unwrap() + 6 + 16;
    lying[pos..pos + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(PBO::read_entries(&mut std::io::Cursor::new(&lying), |_| true).is_err());
}

#[test]
fn read_still_bounds_header_strings() {
    let long = vec![b'a'; 100_000];
    assert!(PBO::read(&mut &long[..]).is_err());
}
