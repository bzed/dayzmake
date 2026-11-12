//! Tests that drive the built `dayzmake` binary.
#![allow(dead_code)]
use std::path::Path;
use std::process::Command;

/// Runs dayzmake in `cwd`; returns (exit code, stdout, stderr). A panic shows up as 101.
pub fn dayzmake(args: &[&str], cwd: &Path) -> (i32, String, String) {
    let (code, out, err) = dayzmake_raw(args, cwd);
    (code, String::from_utf8_lossy(&out).into_owned(), err)
}

/// Like `dayzmake`, but stdout as raw bytes.
pub fn dayzmake_raw(args: &[&str], cwd: &Path) -> (i32, Vec<u8>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run dayzmake");
    (
        out.status.code().unwrap_or(-1),
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn cli_version_runs() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, _) = dayzmake(&["--version"], dir.path());
    assert_eq!(code, 0);
    assert!(out.starts_with("v"), "{}", out);
}

fn write(dir: &Path, name: &str, content: &str) {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn unquoted_before_brace() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "c.cpp", "\nclass A { x = foo };\n"); // leading newline: line-1 warnings are fixed in a later task
    let (code, _, err) = dayzmake(&["rapify", "c.cpp", "c.bin"], dir.path());
    assert_eq!(code, 0, "{}", err);
    let (code, out, _) = dayzmake(&["derapify", "c.bin"], dir.path());
    assert_eq!(code, 0);
    assert!(out.contains("x = \"foo\";"), "{}", out);
}

fn rapify_str(src: &str) -> (i32, String) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "config.cpp", src);
    let (code, _, err) = dayzmake(&["rapify", "config.cpp", "c.bin"], dir.path());
    (code, err)
}

#[test]
fn warning_on_line_one_no_panic() {
    let (code, err) = rapify_str("class A { x = foo; };\n");
    assert_eq!(code, 0, "{}", err);
    assert!(err.contains(":1:") && err.contains("unquoted-string"), "{}", err);
}

#[test]
fn warning_on_line_two_location() {
    let (code, err) = rapify_str("\nclass A { x = foo; };\n");
    assert_eq!(code, 0, "{}", err);
    assert!(err.contains(":2:"), "{}", err);
}

#[test]
fn warning_printed_once() {
    let (_, err) = rapify_str("class A { x = foo; };\n");
    assert_eq!(err.matches("unquoted-string").count(), 1, "{}", err);
}

fn prerapified_mod() -> (tempfile::TempDir, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src.cpp", "class A { x = 1; };\n");
    let (code, _, err) = dayzmake(&["rapify", "src.cpp", "src.bin"], dir.path());
    assert_eq!(code, 0, "{}", err);
    let bin = std::fs::read(dir.path().join("src.bin")).unwrap();
    (dir, bin)
}

#[test]
fn build_keeps_prerapified_rvmat() {
    let (dir, bin) = prerapified_mod();
    write(dir.path(), "m/config.cpp", "class B { y = 2; };\n");
    std::fs::create_dir_all(dir.path().join("m/data")).unwrap();
    std::fs::write(dir.path().join("m/data/x.rvmat"), &bin).unwrap();
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{}", err);
    let (code, out, err) = dayzmake_raw(&["cat", "out.pbo", "data\\x.rvmat"], dir.path());
    assert_eq!(code, 0, "{}", err);
    assert_eq!(out, bin);
}

#[test]
fn build_keeps_prerapified_config_cpp() {
    let (dir, bin) = prerapified_mod();
    std::fs::create_dir_all(dir.path().join("m")).unwrap();
    std::fs::write(dir.path().join("m/config.cpp"), &bin).unwrap();
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{}", err);
    let (code, out, err) = dayzmake_raw(&["cat", "out.pbo", "config.bin"], dir.path());
    assert_eq!(code, 0, "{}", err);
    assert_eq!(out, bin);
}

#[test]
fn macro_arity_warns() {
    let (code, err) = rapify_str("#define QUOTE(x) #x\nclass A { t = QUOTE(a,b); };\n");
    assert_eq!(code, 0, "{}", err);
    assert!(err.contains("macro-arg-count") && err.contains("QUOTE"), "{}", err);
}

#[test]
fn macro_arity_warning_has_location() {
    let (code, err) = rapify_str("#define QUOTE(x) #x\n\nclass A { t = QUOTE(a,b); };\n");
    assert_eq!(code, 0, "{}", err);
    assert!(err.contains("config.cpp:3") && err.contains("macro-arg-count"), "{}", err);
    // the empty value it expands to is not reported again as an unquoted string
    assert!(!err.contains("unquoted-string"), "{}", err);
}

#[test]
fn macro_arity_in_inactive_block_silent() {
    let (code, err) = rapify_str("#define QUOTE(x) #x\n#ifdef NOPE\nt = QUOTE(a,b);\n#endif\nx = 1;\n");
    assert_eq!(code, 0, "{}", err);
    assert!(!err.contains("macro-arg-count"), "{}", err);
}

#[test]
fn eval_error_without_unquoted_warning() {
    let (code, err) = rapify_str("a = __EVAL(1 > 0);\n");
    assert!(code != 0 && code != 101, "code {} {}", code, err);
    assert!(err.contains("not supported"), "{}", err);
    assert!(!err.contains("unquoted-string"), "{}", err);
}

#[test]
fn object_like_macro_no_arity_warning() {
    let (code, err) = rapify_str("#define FOO bar\nclass A { t = FOO(1); };\n");
    assert_eq!(code, 0, "{}", err);
    assert!(!err.contains("macro-arg-count"), "{}", err);
}

#[test]
fn cli_include_case_warns() {
    let dir = tempdir_with(&[("config.cpp", "#include \"INC.hpp\"\n"), ("inc.hpp", "x = 1;\n")]);
    let (code, _, err) = dayzmake(&["rapify", "config.cpp", "c.bin"], dir.path());
    assert_eq!(code, 0, "{}", err);
    assert!(err.contains("include-case"), "{}", err);
}

#[test]
fn cli_missing_include_folder() {
    let dir = tempdir_with(&[("config.cpp", "x = 1;\n")]);
    let (code, _, err) = dayzmake(&["rapify", "-i", "nonexist", "config.cpp", "c.bin"], dir.path());
    assert!(code != 0 && code != 101, "code {} {}", code, err);
    assert!(err.contains("nonexist"), "{}", err);
}

#[test]
fn include_not_found_has_line() {
    let dir = tempdir_with(&[("config.cpp", "x = 1;\n#include \"missing.hpp\"\n")]);
    let (code, _, err) = dayzmake(&["rapify", "config.cpp", "c.bin"], dir.path());
    assert!(code != 0, "{}", err);
    assert!(err.contains("config.cpp:2"), "{}", err);
}

fn tempdir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (n, c) in files { write(dir.path(), n, c); }
    dir
}

/// Runs dayzmake with DayZ Tools unreachable, so any attempt to start binarize.exe is an error.
fn dayzmake_no_tools(args: &[&str], cwd: &Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .args(args)
        .current_dir(cwd)
        .env("DAYZ_TOOLS", "/nonexistent/dayz-tools")
        .output()
        .expect("failed to run dayzmake");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn prebinarized_build(file: &str, magic: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let body = [magic, b"\x38\0\0\0 not a real model"].concat();
    std::fs::write(src.join(file), &body).unwrap();
    let (code, err) = dayzmake_no_tools(&["build", "--proton-binarize", "src", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{}", err);
    assert!(err.contains("prebinarized-file"), "{}", err);
    assert!(err.contains(&format!("{} v56", String::from_utf8_lossy(magic))), "{}", err);
    let (code, out, err) = dayzmake_raw(&["cat", "out.pbo", file], dir.path());
    assert_eq!(code, 0, "{}", err);
    assert_eq!(out, body);
}

#[test]
fn build_keeps_prebinarized_p3d() {
    prebinarized_build("m.p3d", b"ODOL");
}

#[test]
fn build_keeps_prebinarized_rtm() {
    prebinarized_build("a.rtm", b"BMTR");
}

#[test]
fn build_still_binarizes_mlod() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("m.p3d"), b"MLOD\x01\x01\0\0").unwrap();
    let (code, err) = dayzmake_no_tools(&["build", "--proton-binarize", "src", "out.pbo"], dir.path());
    assert_ne!(code, 0, "an MLOD must reach binarize.exe: {}", err);
}

#[test]
fn binarize_copies_prebinarized_p3d() {
    let dir = tempfile::tempdir().unwrap();
    let body = b"ODOL\x38\0\0\0 not a real model";
    std::fs::write(dir.path().join("m.p3d"), body).unwrap();
    let (code, err) = dayzmake_no_tools(&["binarize", "m.p3d", "out.p3d"], dir.path());
    assert_eq!(code, 0, "{}", err);
    assert!(err.contains("prebinarized-file"), "{}", err);
    assert_eq!(std::fs::read(dir.path().join("out.p3d")).unwrap(), body);
}

#[test]
fn binarize_reports_older_odol_versions_instead_of_failing() {
    let dir = tempfile::tempdir().unwrap();
    for version in [53u8, 54, 55] {
        let body = [b"ODOL".as_slice(), &[version, 0, 0, 0], b" not a real model"].concat();
        std::fs::write(dir.path().join("m.p3d"), &body).unwrap();
        let (code, err) = dayzmake_no_tools(&["binarize", "-f", "m.p3d", "out.p3d"], dir.path());
        assert_eq!(code, 0, "{}", err);
        assert!(err.contains(&format!("ODOL v{}", version)), "{}", err);
        assert_eq!(std::fs::read(dir.path().join("out.p3d")).unwrap(), body);
    }
}
