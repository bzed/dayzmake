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
fn dotdot_include_is_rejected() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("secret.hpp"), "token = 1;\n").expect("write");
    let err = run(
        "x",
        &[("a\\config.cpp", "")],
        "a\\config.cpp",
        "#include \"..\\secret.hpp\"\n",
    )
    .expect_err("rejected");
    assert!(err.contains("not allowed"), "{err}");
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
