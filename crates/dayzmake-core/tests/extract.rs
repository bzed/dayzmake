//! `extract`: a whole install or mod into one tree by prefix.
mod support;

use std::fs;
use std::path::Path;

use dayzmake_core::config::Config;
use dayzmake_core::extract::{ExtractOptions, extract};
use dayzmake_core::warning::{Warning, Warnings};

#[expect(clippy::unwrap_used, reason = "test helper")]
fn write_pbo(path: &Path, prefix: &str, entries: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let extensions: Vec<(&str, &str)> = if prefix.is_empty() {
        Vec::new()
    } else {
        vec![("prefix", prefix)]
    };
    fs::write(path, support::raw_pbo(entries, &extensions)).unwrap();
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn rapified(text: &str) -> Vec<u8> {
    let config =
        Config::from_string(text.to_string(), None, &Vec::new(), &mut Warnings::new()).unwrap();
    config.to_cursor().unwrap().into_inner().into_vec()
}

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

/// An install with `dta/a.pbo` (prefix `a`, a rapified config) and `Addons/b.pbo` (prefix `b\sub`).
#[expect(clippy::unwrap_used, reason = "test helper")]
fn install() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("install");
    let config = rapified("class CfgPatches { class A { units[] = {}; }; };");
    write_pbo(
        &src.join("dta/a.pbo"),
        "a",
        &[("config.bin", &config), ("x.txt", b"x")],
    );
    write_pbo(
        &src.join("Addons/b.pbo"),
        "b\\sub",
        &[("data\\y.txt", b"y")],
    );
    tmp
}

fn names(warnings: &Warnings) -> Vec<&'static str> {
    warnings
        .warnings()
        .iter()
        .filter_map(|w: &Warning| w.name)
        .collect()
}

#[test]
fn extracts_by_prefix_and_writes_the_map() {
    let tmp = install();
    let target = tmp.path().join("out");
    let mut warnings = Warnings::new();
    let summary = extract(
        &tmp.path().join("install"),
        &target,
        &ExtractOptions::default(),
        &mut warnings,
    )
    .unwrap();
    assert_eq!(
        files_below(&target),
        ["a/config.bin", "a/x.txt", "b/sub/data/y.txt", "pbo-map.tsv"]
    );
    assert_eq!(
        fs::read(target.join("pbo-map.tsv")).unwrap(),
        b"a\tdta/a.pbo\nb/sub\tAddons/b.pbo\n"
    );
    assert_eq!(summary.pbos(), 2);
    assert_eq!(summary.files(), 3);
    assert!(warnings.warnings().is_empty(), "{:?}", warnings.warnings());
}

#[test]
fn derapify_writes_config_cpp() {
    let tmp = install();
    let target = tmp.path().join("out");
    let mut options = ExtractOptions::default();
    options.derapify = true;
    extract(
        &tmp.path().join("install"),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(
        files_below(&target),
        ["a/config.cpp", "a/x.txt", "b/sub/data/y.txt", "pbo-map.tsv"]
    );
    let text = fs::read_to_string(target.join("a/config.cpp")).unwrap();
    assert!(text.contains("class CfgPatches"), "{text}");
}

#[test]
fn a_bin_that_fails_to_derapify_stays_a_bin() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("m");
    write_pbo(
        &src.join("addons/a.pbo"),
        "a",
        &[("bad.bin", b"\0raP\x01\x02"), ("plain.bin", b"nope")],
    );
    let mut options = ExtractOptions::default();
    options.derapify = true;
    let mut warnings = Warnings::new();
    extract(&src, &tmp.path().join("out"), &options, &mut warnings).unwrap();
    assert_eq!(
        files_below(&tmp.path().join("out")),
        ["a/bad.bin", "a/plain.bin", "pbo-map.tsv"]
    );
    assert_eq!(names(&warnings), ["derapify-failed"]);
}

#[test]
fn the_later_pbo_wins_a_duplicate_path_case_insensitively() {
    let tmp = install();
    let src = tmp.path().join("install");
    write_pbo(&src.join("Addons/c.pbo"), "A", &[("X.TXT", b"from c")]);
    let target = tmp.path().join("out");
    let mut warnings = Warnings::new();
    extract(&src, &target, &ExtractOptions::default(), &mut warnings).unwrap();
    // Addons/c.pbo sorts before dta/a.pbo, so a.pbo wins
    let dup: Vec<&Warning> = warnings
        .warnings()
        .iter()
        .filter(|w| w.name == Some("duplicate-path"))
        .collect();
    assert_eq!(dup.len(), 1, "{:?}", warnings.warnings());
    assert!(
        dup[0].message.contains("Addons/c.pbo"),
        "{}",
        dup[0].message
    );
    assert!(dup[0].message.contains("dta/a.pbo"), "{}", dup[0].message);
    let all = files_below(&target);
    assert_eq!(
        all.iter().filter(|f| f.to_lowercase() == "a/x.txt").count(),
        1
    );
    let kept = all.iter().find(|f| f.to_lowercase() == "a/x.txt").unwrap();
    assert_eq!(fs::read(target.join(kept)).unwrap(), b"x");
}

#[test]
fn a_rerun_with_force_drops_files_that_no_longer_exist() {
    let tmp = install();
    let src = tmp.path().join("install");
    let target = tmp.path().join("out");
    extract(
        &src,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    fs::remove_file(src.join("Addons/b.pbo")).unwrap();
    let err = extract(
        &src,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("--force"), "{err}");
    assert!(target.join("b/sub/data/y.txt").exists());
    let mut options = ExtractOptions::default();
    options.force = true;
    extract(&src, &target, &options, &mut Warnings::new()).unwrap();
    assert_eq!(
        files_below(&target),
        ["a/config.bin", "a/x.txt", "pbo-map.tsv"]
    );
    let leftovers: Vec<_> = fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 2, "{leftovers:?}"); // install, out
}

#[test]
fn a_corrupt_pbo_fails_and_leaves_the_previous_tree_alone() {
    let tmp = install();
    let src = tmp.path().join("install");
    let target = tmp.path().join("out");
    extract(
        &src,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    let before = files_below(&target);
    fs::write(src.join("dta/z.pbo"), b"this is not a pbo").unwrap();
    let mut options = ExtractOptions::default();
    options.force = true;
    let err = extract(&src, &target, &options, &mut Warnings::new()).unwrap_err();
    assert!(err.to_string().contains("z.pbo"), "{err}");
    assert_eq!(files_below(&target), before);
    assert_eq!(
        fs::read_dir(tmp.path()).unwrap().count(),
        2,
        "staging dir left behind"
    );
}

#[test]
fn a_single_pbo_with_an_empty_prefix_uses_its_file_stem() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("Thing.pbo");
    write_pbo(&pbo, "", &[("a.txt", b"a")]);
    let target = tmp.path().join("out");
    extract(
        &pbo,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(files_below(&target), ["Thing/a.txt", "pbo-map.tsv"]);
    assert_eq!(
        fs::read(target.join("pbo-map.tsv")).unwrap(),
        b"Thing\tThing.pbo\n"
    );
}

#[test]
fn only_and_exclude_select_entries() {
    let tmp = install();
    let target = tmp.path().join("out");
    let mut options = ExtractOptions::default();
    options.only = vec!["*.txt".to_string()];
    options.exclude = vec!["data/*".to_string()];
    extract(
        &tmp.path().join("install"),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(files_below(&target), ["a/x.txt", "pbo-map.tsv"]);
    // the map still lists every PBO
    assert_eq!(
        fs::read_to_string(target.join("pbo-map.tsv"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn unsafe_names_are_skipped_with_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a.pbo");
    write_pbo(&pbo, "p", &[("..\\evil.txt", b"e"), ("ok.txt", b"o")]);
    let mut warnings = Warnings::new();
    let target = tmp.path().join("out");
    extract(&pbo, &target, &ExtractOptions::default(), &mut warnings).unwrap();
    assert_eq!(files_below(&target), ["p/ok.txt", "pbo-map.tsv"]);
    assert_eq!(names(&warnings), ["unsafe-path"]);
}

#[test]
fn the_map_escapes_tab_cr_and_lf_in_the_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a.pbo");
    write_pbo(&pbo, "x\ty\rz", &[("ok.txt", b"o")]);
    let mut warnings = Warnings::new();
    let target = tmp.path().join("out");
    extract(&pbo, &target, &ExtractOptions::default(), &mut warnings).unwrap();
    // a control character makes the prefix unsafe as a folder name: the PBO is skipped with a warning
    assert_eq!(names(&warnings), ["unsafe-path"]);
    assert_eq!(
        fs::read(target.join("pbo-map.tsv")).unwrap(),
        b"x\\ty\\rz\ta.pbo\n"
    );
}

#[test]
fn a_prefix_named_like_the_map_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a.pbo");
    write_pbo(&pbo, "PBO-MAP.tsv", &[("ok.txt", b"o")]);
    let mut warnings = Warnings::new();
    let target = tmp.path().join("out");
    extract(&pbo, &target, &ExtractOptions::default(), &mut warnings).unwrap();
    assert_eq!(names(&warnings), ["unsafe-path"]);
    assert_eq!(files_below(&target), ["pbo-map.tsv"]);
    assert_eq!(
        fs::read(target.join("pbo-map.tsv")).unwrap(),
        b"PBO-MAP.tsv\ta.pbo\n"
    );
}

#[test]
fn only_the_first_level_of_dta_and_addons_is_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("m");
    write_pbo(&src.join("ADDONS/a.PBO"), "a", &[("a.txt", b"a")]);
    write_pbo(&src.join("addons/sub/b.pbo"), "b", &[("b.txt", b"b")]);
    write_pbo(&src.join("other/c.pbo"), "c", &[("c.txt", b"c")]);
    write_pbo(&src.join("top.pbo"), "t", &[("t.txt", b"t")]);
    let target = tmp.path().join("out");
    extract(
        &src,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(files_below(&target), ["a/a.txt", "pbo-map.tsv"]);
}

#[test]
fn a_source_without_pbos_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("m/addons")).unwrap();
    let err = extract(
        &tmp.path().join("m"),
        &tmp.path().join("out"),
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("no PBOs"), "{err}");
    assert!(!tmp.path().join("out").exists());
}

#[test]
fn a_target_that_contains_the_source_is_refused() {
    let tmp = install();
    let mut options = ExtractOptions::default();
    options.force = true;
    let err = extract(
        &tmp.path().join("install"),
        tmp.path(),
        &options,
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("contains the source"), "{err}");
    assert!(tmp.path().join("install/dta/a.pbo").exists());
}

#[test]
fn a_target_that_is_a_file_is_refused_even_with_force() {
    let tmp = install();
    let target = tmp.path().join("out");
    fs::write(&target, "file").unwrap();
    let mut options = ExtractOptions::default();
    options.force = true;
    let err = extract(
        &tmp.path().join("install"),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("not a folder"), "{err}");
    assert_eq!(fs::read(&target).unwrap(), b"file");
}

/// `m/addons/a.pbo` and a target below it that an earlier extract made.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn mod_with_target_inside(
    target_rel: &str,
) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("m");
    write_pbo(&src.join("addons/a.pbo"), "p", &[("x.txt", b"x")]);
    let target = src.join(target_rel);
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("pbo-map.tsv"), "").unwrap();
    (tmp, src, target)
}

#[test]
fn force_never_deletes_the_source_when_the_target_is_inside_it() {
    for rel in ["addons", "addons/out", "."] {
        let (_tmp, src, target) = mod_with_target_inside(rel);
        let mut options = ExtractOptions::default();
        options.force = true;
        let result = extract(&src, &target, &options, &mut Warnings::new());
        assert!(result.is_err(), "{rel}");
        assert!(
            src.join("addons/a.pbo").exists(),
            "{rel}: the source was deleted"
        );
    }
}

#[test]
fn force_refuses_a_folder_that_is_not_an_extract_result() {
    let tmp = install();
    let target = tmp.path().join("home");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("precious.txt"), "mine").unwrap();
    let mut options = ExtractOptions::default();
    options.force = true;
    let err = extract(
        &tmp.path().join("install"),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("not the result of"), "{err}");
    assert!(err.to_string().contains("home"), "{err}");
    assert_eq!(fs::read(target.join("precious.txt")).unwrap(), b"mine");
}

#[test]
fn force_replaces_an_empty_folder() {
    let tmp = install();
    let target = tmp.path().join("out");
    fs::create_dir_all(&target).unwrap();
    let mut options = ExtractOptions::default();
    options.force = true;
    extract(
        &tmp.path().join("install"),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap();
    assert!(target.join("pbo-map.tsv").is_file());
}

/// A tree from an earlier run, to see that a failed run leaves it alone.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn previous_tree(tmp: &Path) -> (std::path::PathBuf, Vec<String>) {
    let target = tmp.join("out");
    let src = tmp.join("install");
    extract(
        &src,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    let before = files_below(&target);
    (target, before)
}

#[test]
fn a_corrupt_compressed_entry_fails_and_keeps_the_old_tree() {
    let tmp = install();
    let (target, before) = previous_tree(tmp.path());
    let pbo = support::raw_pbo_with_headers(
        &[("bad.txt", &[0, 1], support::CPRS, 100)],
        &[("prefix", "z")],
    );
    fs::write(tmp.path().join("install/dta/z.pbo"), pbo).unwrap();
    let mut options = ExtractOptions::default();
    options.force = true;
    let err = extract(
        &tmp.path().join("install"),
        &target,
        &options,
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("bad.txt"), "{err}");
    assert_eq!(files_below(&target), before);
    assert_eq!(
        fs::read_dir(tmp.path()).unwrap().count(),
        2,
        "staging dir left behind"
    );
}

#[test]
fn a_write_failure_fails_and_keeps_the_old_tree() {
    let tmp = install();
    let (target, before) = previous_tree(tmp.path());
    // "a" is a file, so "a/b" cannot be written
    write_pbo(
        &tmp.path().join("install/dta/z.pbo"),
        "z",
        &[("a", b"1"), ("a\\b", b"2")],
    );
    let mut options = ExtractOptions::default();
    options.force = true;
    assert!(
        extract(
            &tmp.path().join("install"),
            &target,
            &options,
            &mut Warnings::new()
        )
        .is_err()
    );
    assert_eq!(files_below(&target), before);
    assert_eq!(
        fs::read_dir(tmp.path()).unwrap().count(),
        2,
        "staging dir left behind"
    );
}

#[test]
fn a_truncated_entry_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("t.pbo");
    let mut bytes = support::raw_pbo(&[("a.txt", b"0123456789")], &[("prefix", "p")]);
    // cut into the data
    bytes.truncate(bytes.len() - 20 - 1 - 4);
    fs::write(&pbo, bytes).unwrap();
    let err = extract(
        &pbo,
        &tmp.path().join("out"),
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("truncated"), "{err}");
    assert!(!tmp.path().join("out").exists());
}

#[test]
fn compressed_and_large_entries_are_extracted_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("c.pbo");
    let big = vec![b'z'; 3 << 20];
    let packed = support::lzss_literals(b"hello world");
    let bytes = support::raw_pbo_with_headers(
        &[
            ("c.txt", &packed, support::CPRS, 11),
            ("big.bin", &big, 0, u32::try_from(big.len()).unwrap()),
        ],
        &[("prefix", "p")],
    );
    fs::write(&pbo, bytes).unwrap();
    let mut options = ExtractOptions::default();
    options.derapify = true;
    let out = tmp.path().join("out");
    extract(&pbo, &out, &options, &mut Warnings::new()).unwrap();
    assert_eq!(fs::read(out.join("p/c.txt")).unwrap(), b"hello world");
    assert_eq!(fs::read(out.join("p/big.bin")).unwrap(), big);
}

#[test]
fn a_non_utf8_entry_name_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("n.pbo");
    fs::write(
        &pbo,
        support::raw_pbo_bytes(&[(b"a\xffb.txt", b"x", 0)], &[(b"prefix", b"p")]),
    )
    .unwrap();
    let mut warnings = Warnings::new();
    extract(
        &pbo,
        &tmp.path().join("out"),
        &ExtractOptions::default(),
        &mut warnings,
    )
    .unwrap();
    assert_eq!(names(&warnings), ["non-utf8-filename"]);
}

/// Every `\` of the map starts an escape: the prefix is written with `/`, a literal `\n` of a name is `\\n`.
#[test]
fn the_map_writes_slashes_and_escapes_the_backslash() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a.pbo");
    // `dz\gear\navigation` must not read back as an LF; the TAB makes the prefix unsafe as a folder
    write_pbo(&pbo, "dz\\gear\\navigation\tz", &[("ok.txt", b"o")]);
    let target = tmp.path().join("out");
    extract(
        &pbo,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(
        fs::read(target.join("pbo-map.tsv")).unwrap(),
        b"dz/gear/navigation\\tz\ta.pbo\n"
    );
}

#[cfg(unix)]
#[test]
fn a_backslash_in_the_pbo_path_is_escaped_in_the_map() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("we\\nird.pbo");
    write_pbo(&pbo, "p", &[("ok.txt", b"o")]);
    let target = tmp.path().join("out");
    extract(
        &tmp.path().join("we\\nird.pbo"),
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(
        fs::read(target.join("pbo-map.tsv")).unwrap(),
        b"p\twe\\\\nird.pbo\n"
    );
}

#[test]
fn a_compressed_entry_with_a_huge_original_size_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("h.pbo");
    let packed = support::lzss_literals(b"x");
    fs::write(
        &pbo,
        support::raw_pbo_with_headers(
            &[("huge.txt", &packed, support::CPRS, 0x3000_0000)],
            &[("prefix", "p")],
        ),
    )
    .unwrap();
    let out = tmp.path().join("out");
    let err = extract(&pbo, &out, &ExtractOptions::default(), &mut Warnings::new()).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    assert!(!out.exists());
}

#[test]
fn a_compressed_rapified_bin_is_derapified_and_tiny_bins_stay() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("d.pbo");
    let config = rapified("class CfgPatches { class A { units[] = {}; }; };");
    let packed = support::lzss_literals(&config);
    let original = u32::try_from(config.len()).unwrap();
    fs::write(
        &pbo,
        support::raw_pbo_with_headers(
            &[
                ("config.bin", &packed, support::CPRS, original),
                ("tiny.bin", b"ab", 0, 2),
            ],
            &[("prefix", "p")],
        ),
    )
    .unwrap();
    let mut options = ExtractOptions::default();
    options.derapify = true;
    let out = tmp.path().join("out");
    let mut warnings = Warnings::new();
    extract(&pbo, &out, &options, &mut warnings).unwrap();
    assert!(warnings.warnings().is_empty(), "{:?}", warnings.warnings());
    assert_eq!(
        files_below(&out),
        ["p/config.cpp", "p/tiny.bin", "pbo-map.tsv"]
    );
    let text = fs::read_to_string(out.join("p/config.cpp")).unwrap();
    assert!(text.contains("class CfgPatches"), "{text}");
    assert_eq!(fs::read(out.join("p/tiny.bin")).unwrap(), b"ab");
}

#[test]
fn a_repeated_entry_of_one_pbo_is_skipped_and_a_leading_separator_is_stripped() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("r.pbo");
    write_pbo(
        &pbo,
        "p",
        &[
            ("x.txt", b"first"),
            ("X.TXT", b"second"),
            ("\\lead.txt", b"l"),
        ],
    );
    let out = tmp.path().join("out");
    let mut warnings = Warnings::new();
    let summary = extract(&pbo, &out, &ExtractOptions::default(), &mut warnings).unwrap();
    assert_eq!(summary.files(), 2);
    assert_eq!(fs::read(out.join("p/x.txt")).unwrap(), b"first");
    assert_eq!(fs::read(out.join("p/lead.txt")).unwrap(), b"l");
    let got = names(&warnings);
    assert!(got.contains(&"duplicate-entry"), "{got:?}");
    assert!(got.len() >= 2, "a leading separator warns too: {got:?}");
}

#[test]
fn a_target_without_a_folder_name_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a.pbo");
    write_pbo(&pbo, "p", &[("x.txt", b"x")]);
    let target = tmp.path().join("missing").join("..");
    let err = extract(
        &pbo,
        &target,
        &ExtractOptions::default(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("Invalid target"), "{err}");
}

#[test]
fn a_target_below_a_file_is_an_error_and_a_taken_staging_name_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a.pbo");
    write_pbo(&pbo, "p", &[("x.txt", b"x")]);
    let file = tmp.path().join("file");
    fs::write(&file, b"f").unwrap();
    assert!(
        extract(
            &pbo,
            &file.join("out"),
            &ExtractOptions::default(),
            &mut Warnings::new()
        )
        .is_err()
    );
    // the first staging name is taken by a folder: the next one is used
    let taken = tmp
        .path()
        .join(format!(".out.extract-{}-0", std::process::id()));
    fs::create_dir(&taken).unwrap();
    let out = tmp.path().join("out");
    extract(&pbo, &out, &ExtractOptions::default(), &mut Warnings::new()).unwrap();
    assert!(out.join("p/x.txt").is_file());
    assert!(taken.is_dir(), "the foreign folder is left alone");
}

#[cfg(unix)]
#[test]
fn the_map_escapes_a_newline_of_the_pbo_path() {
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a\nb.pbo");
    write_pbo(&pbo, "p", &[("ok.txt", b"o")]);
    let out = tmp.path().join("out");
    extract(&pbo, &out, &ExtractOptions::default(), &mut Warnings::new()).unwrap();
    assert_eq!(
        fs::read(out.join("pbo-map.tsv")).unwrap(),
        b"p\ta\\nb.pbo\n"
    );
}

#[cfg(unix)]
#[test]
fn an_old_tree_that_cannot_be_removed_is_a_warning() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let pbo = tmp.path().join("a.pbo");
    write_pbo(&pbo, "p", &[("x.txt", b"x")]);
    let out = tmp.path().join("out");
    extract(&pbo, &out, &ExtractOptions::default(), &mut Warnings::new()).unwrap();
    // a read-only folder with a file in it cannot be removed
    let locked = out.join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("f"), b"f").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let blocked = fs::remove_file(locked.join("f")).is_err();
    let mut options = ExtractOptions::default();
    options.force = true;
    let mut warnings = Warnings::new();
    let result = extract(&pbo, &out, &options, &mut warnings);
    // let the temp dir clean up
    for entry in fs::read_dir(tmp.path()).unwrap() {
        let path = entry.unwrap().path().join("locked");
        if path.is_dir() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    result.unwrap();
    if blocked {
        assert_eq!(names(&warnings), ["old-tree-left"]);
        assert!(out.join("p/x.txt").is_file());
    }
}
