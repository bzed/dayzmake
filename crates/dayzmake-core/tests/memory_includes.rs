//! `#include` resolved from the entries of one PBO only (#97): the file system is never read.
use std::fs;
use std::path::PathBuf;

use tempfile::tempdir;

use dayzmake_core::preprocess::{IncludeFolders, MemoryIncludes, preprocess_in};
use dayzmake_core::warning::Warnings;

fn files(list: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
    list.iter()
        .map(|(n, c)| ((*n).to_string(), c.as_bytes().to_vec()))
        .collect()
}

fn run(prefix: &str, list: &[(&str, &str)], origin: &str, input: &str) -> Result<String, String> {
    let folders =
        IncludeFolders::new(Vec::new()).with_memory(MemoryIncludes::new(prefix, files(list)));
    preprocess_in(
        input.to_string(),
        Some(PathBuf::from(origin)),
        &folders,
        &mut Warnings::new(),
    )
    .map(|(out, _)| out)
    .map_err(|e| e.to_string())
}

#[test]
fn sibling_include_in_the_pbo_is_resolved() {
    let out = run(
        "x",
        &[("sub\\a.hpp", "a = 1;\n"), ("sub\\config.cpp", "")],
        "sub\\config.cpp",
        "#include \"a.hpp\"\n",
    )
    .expect("resolved");
    assert_eq!(out.trim(), "a = 1;");
}

#[test]
fn prefix_form_resolves_into_the_pbo() {
    let out = run(
        "x\\y",
        &[("data\\A.HPP", "a = 2;\n")],
        "config.cpp",
        "#include \"\\x\\y\\data\\a.hpp\"\n",
    )
    .expect("resolved");
    assert_eq!(out.trim(), "a = 2;");
}

#[test]
fn prefix_form_outside_the_prefix_is_rejected() {
    let err = run(
        "x",
        &[("a.hpp", "a = 3;\n")],
        "config.cpp",
        "#include \"\\other\\a.hpp\"\n",
    )
    .expect_err("not found");
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn absolute_and_drive_includes_are_rejected_without_reading() {
    let dir = tempdir().expect("tempdir");
    let secret = dir.path().join("secret.hpp");
    fs::write(&secret, "token = \"s3cr3t\";\n").expect("write");
    for path in [secret.display().to_string(), "C:\\secret.hpp".to_string()] {
        let input = format!("#include \"{path}\"\n");
        let err = run("x", &[], "config.cpp", &input).expect_err("rejected");
        assert!(!err.contains("s3cr3t"), "{err}");
    }
}

#[test]
fn dotdot_inside_the_pbo_resolves_for_both_separators() {
    let list = [("macros.hpp", "m = 1;\n"), ("a\\b\\config.cpp", "")];
    for path in [
        "..\\..\\macros.hpp",
        "../../macros.hpp",
        "..\\../macros.hpp",
        "..\\c\\..\\..\\macros.hpp",
        "..\\..\\MACROS.HPP",
    ] {
        let input = format!("#include \"{path}\"\n");
        let out =
            run("x", &list, "a\\b\\config.cpp", &input).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(out.trim(), "m = 1;", "{path}");
    }
}

#[test]
fn dotdot_that_leaves_the_pbo_root_is_rejected() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("secret.hpp"), "token = 1;\n").expect("write");
    let list = [("secret.hpp", "inside = 1;\n"), ("a\\config.cpp", "")];
    for (origin, path) in [
        ("config.cpp", "..\\secret.hpp"),
        ("a\\config.cpp", "..\\..\\secret.hpp"),
        ("a\\config.cpp", "sub\\..\\..\\..\\secret.hpp"),
        ("a\\config.cpp", "../..\\secret.hpp"),
        ("a\\config.cpp", "..\\..\\..\\..\\etc\\passwd"),
    ] {
        let input = format!("#include \"{path}\"\n");
        let err = run("x", &list, origin, &input).expect_err(path);
        assert!(err.contains("not allowed"), "{path}: {err}");
    }
}

#[test]
fn an_empty_component_and_a_leading_slash_are_rejected() {
    let list = [
        ("a.hpp", "a = 1;\n"),
        ("sub\\a.hpp", "a = 2;\n"),
        ("sub\\config.cpp", ""),
    ];
    for path in [
        "sub\\\\a.hpp",
        "sub//a.hpp",
        "..\\\\a.hpp",
        "/a.hpp",
        "sub\\",
    ] {
        let input = format!("#include \"{path}\"\n");
        let err = run("x", &list, "sub\\config.cpp", &input).expect_err(path);
        assert!(err.contains("not allowed"), "{path}: {err}");
    }
}

#[test]
fn an_include_of_an_unreadable_entry_names_the_reason() {
    let folders = IncludeFolders::new(Vec::new()).with_memory(
        MemoryIncludes::new("x", files(&[("b.hpp", "b = 1;\n")])).with_unreadable(vec![(
            "Sub\\a.hpp".to_string(),
            "it is truncated".to_string(),
        )]),
    );
    let go = |input: &str| {
        preprocess_in(
            input.to_string(),
            Some(PathBuf::from("sub\\config.cpp")),
            &folders,
            &mut Warnings::new(),
        )
        .map_err(|e| e.to_string())
    };
    let err = go("#include \"a.hpp\"\n").expect_err("unreadable");
    assert!(
        err.contains("cannot be read") && err.contains("it is truncated"),
        "{err}"
    );
    assert!(!err.contains("not found"), "{err}");
    let err = go("#include \"c.hpp\"\n").expect_err("missing");
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn a_file_that_only_exists_in_the_cwd_is_not_read() {
    // the test runs with the crate directory as cwd: Cargo.toml exists there
    let err = run("x", &[], "config.cpp", "#include \"Cargo.toml\"\n").expect_err("not found");
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn include_cycles_and_total_size_are_bounded() {
    let err = run(
        "x",
        &[("a.hpp", "#include \"a.hpp\"\n")],
        "config.cpp",
        "#include \"a.hpp\"\n",
    )
    .expect_err("cycle");
    assert!(err.contains("cycle"), "{err}");

    let big = "x".repeat(1 << 20);
    let input = "#include \"b.hpp\"\n".repeat(80);
    let err = run("x", &[("b.hpp", &big)], "config.cpp", &input).expect_err("too much");
    assert!(err.contains("bytes"), "{err}");
}

#[test]
fn the_prefix_form_needs_a_prefix_and_a_valid_path() {
    // a PBO without a prefix has no `\prefix\...` form
    let err = run(
        "",
        &[("a.hpp", "a = 1;\n")],
        "config.cpp",
        "#include \"\\a.hpp\"\n",
    )
    .expect_err("no prefix");
    assert!(err.contains("not found"), "{err}");
    let err = run(
        "x",
        &[("b.hpp", "b = 1;\n")],
        "config.cpp",
        "#include \"\\x\\a\\..\\b.hpp\"\n",
    )
    .expect_err("invalid path");
    assert!(err.contains("not allowed"), "{err}");
}

#[test]
fn an_include_that_is_not_utf8_is_an_error() {
    let folders = IncludeFolders::new(Vec::new()).with_memory(MemoryIncludes::new(
        "x",
        vec![("bad.hpp".to_string(), vec![0xff, 0xfe])],
    ));
    let err = preprocess_in(
        "#include \"bad.hpp\"\n".to_string(),
        Some(PathBuf::from("config.cpp")),
        &folders,
        &mut Warnings::new(),
    )
    .expect_err("not UTF-8")
    .to_string();
    assert!(err.contains("UTF-8"), "{err}");
}
