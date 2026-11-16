//! Tests that drive the built `dayzmake` binary.
#[cfg(unix)]
#[path = "../crates/dayzmake-core/tests/support/mod.rs"]
mod support;

use std::path::Path;
use std::process::Command;

/// Points `HOME` and the XDG directories of `cmd` at `home`, so a test never reads or writes the
/// user's configuration or caches.
fn isolate(cmd: &mut Command, home: &Path) {
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env_remove("STEAM_ROOT")
        .env_remove("DAYZ_TOOLS")
        .env_remove("PROTON")
        .env_remove("BIOUTPUT");
}

/// Runs dayzmake in `cwd`; returns (exit code, stdout, stderr). A panic shows up as 101.
///
/// # Panics
///
/// Panics if the binary cannot be spawned.
#[must_use]
pub fn dayzmake(args: &[&str], cwd: &Path) -> (i32, String, String) {
    let (code, out, err) = dayzmake_raw(args, cwd);
    (code, String::from_utf8_lossy(&out).into_owned(), err)
}

/// Like `dayzmake`, but stdout as raw bytes.
///
/// # Panics
///
/// Panics if the binary cannot be spawned.
#[must_use]
#[expect(clippy::expect_used, reason = "test helper")]
pub fn dayzmake_raw(args: &[&str], cwd: &Path) -> (i32, Vec<u8>, String) {
    let home = tempfile::tempdir().expect("failed to create a temp HOME");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(args).current_dir(cwd);
    isolate(&mut cmd, home.path());
    let out = cmd.output().expect("failed to run dayzmake");
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
    assert!(out.starts_with('v'), "{}", out);
}

#[expect(clippy::unwrap_used, reason = "test helper")]
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
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = dayzmake(&["derapify", "c.bin"], dir.path());
    assert_eq!(code, 0);
    assert!(out.contains("x = \"foo\";"), "{}", out);
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn rapify_str(src: &str) -> (i32, String) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "config.cpp", src);
    let (code, _, err) = dayzmake(&["rapify", "config.cpp", "c.bin"], dir.path());
    (code, err)
}

#[test]
fn warning_on_line_one_no_panic() {
    let (code, err) = rapify_str("class A { x = foo; };\n");
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains(":1:") && err.contains("unquoted-string"),
        "{}",
        err
    );
}

#[test]
fn warning_on_line_two_location() {
    let (code, err) = rapify_str("\nclass A { x = foo; };\n");
    assert_eq!(code, 0, "{err}");
    assert!(err.contains(":2:"), "{}", err);
}

#[test]
fn warning_printed_once() {
    let (_, err) = rapify_str("class A { x = foo; };\n");
    assert_eq!(err.matches("unquoted-string").count(), 1, "{err}");
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn prerapified_mod() -> (tempfile::TempDir, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src.cpp", "class A { x = 1; };\n");
    let (code, _, err) = dayzmake(&["rapify", "src.cpp", "src.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
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
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = dayzmake_raw(&["cat", "out.pbo", "data\\x.rvmat"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, bin);
}

#[test]
fn build_keeps_prerapified_config_cpp() {
    let (dir, bin) = prerapified_mod();
    std::fs::create_dir_all(dir.path().join("m")).unwrap();
    std::fs::write(dir.path().join("m/config.cpp"), &bin).unwrap();
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = dayzmake_raw(&["cat", "out.pbo", "config.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, bin);
}

#[test]
fn macro_arity_warns() {
    let (code, err) = rapify_str("#define QUOTE(x) #x\nclass A { t = QUOTE(a,b); };\n");
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("macro-arg-count") && err.contains("QUOTE"),
        "{}",
        err
    );
}

#[test]
fn macro_arity_warning_has_location() {
    let (code, err) = rapify_str("#define QUOTE(x) #x\n\nclass A { t = QUOTE(a,b); };\n");
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("config.cpp:3") && err.contains("macro-arg-count"),
        "{}",
        err
    );
    // the empty value it expands to is not reported again as an unquoted string, but as an empty value
    assert!(!err.contains("unquoted-string"), "{}", err);
    assert!(err.contains("empty-value"), "{}", err);
}

#[test]
fn macro_arity_in_inactive_block_silent() {
    let (code, err) =
        rapify_str("#define QUOTE(x) #x\n#ifdef NOPE\nt = QUOTE(a,b);\n#endif\nx = 1;\n");
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("macro-arg-count"), "{}", err);
}

#[test]
fn eval_error_without_unquoted_warning() {
    let (code, err) = rapify_str("a = __EVAL(1 > 0);\n");
    assert!(code != 0 && code != 101, "code {code} {err}");
    assert!(err.contains("not supported"), "{}", err);
    assert!(!err.contains("unquoted-string"), "{}", err);
}

#[test]
fn object_like_macro_no_arity_warning() {
    let (code, err) = rapify_str("#define FOO bar\nclass A { t = FOO(1); };\n");
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("macro-arg-count"), "{}", err);
}

#[test]
fn cli_include_case_warns() {
    let dir = tempdir_with(&[
        ("config.cpp", "#include \"INC.hpp\"\n"),
        ("inc.hpp", "x = 1;\n"),
    ]);
    let (code, _, err) = dayzmake(&["rapify", "config.cpp", "c.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("include-case"), "{}", err);
}

#[test]
fn cli_missing_include_folder() {
    let dir = tempdir_with(&[("config.cpp", "x = 1;\n")]);
    let (code, _, err) = dayzmake(
        &["rapify", "-i", "nonexist", "config.cpp", "c.bin"],
        dir.path(),
    );
    assert!(code != 0 && code != 101, "code {code} {err}");
    assert!(err.contains("nonexist"), "{}", err);
}

#[test]
fn include_not_found_has_line() {
    let dir = tempdir_with(&[("config.cpp", "x = 1;\n#include \"missing.hpp\"\n")]);
    let (code, _, err) = dayzmake(&["rapify", "config.cpp", "c.bin"], dir.path());
    assert!(code != 0, "{}", err);
    assert!(err.contains("config.cpp:2"), "{}", err);
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn tempdir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (n, c) in files {
        write(dir.path(), n, c);
    }
    dir
}

/// Runs dayzmake with DayZ Tools unreachable, so any attempt to start binarize.exe is an error.
///
/// # Panics
///
/// Panics if the binary cannot be spawned.
#[must_use]
#[expect(clippy::expect_used, reason = "test helper")]
fn dayzmake_no_tools(args: &[&str], cwd: &Path) -> (i32, String) {
    let home = tempfile::tempdir().expect("failed to create a temp HOME");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    isolate(&mut cmd, home.path());
    cmd.args(args)
        .current_dir(cwd)
        .env("DAYZ_TOOLS", "/nonexistent/dayz-tools");
    let out = cmd.output().expect("failed to run dayzmake");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn prebinarized_build(file: &str, magic: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::create_dir_all(&src).unwrap();
    let body = [magic, b"\x38\0\0\0 not a real model"].concat();
    std::fs::write(src.join(file), &body).unwrap();
    let (code, err) = dayzmake_no_tools(
        &["build", "--proton-binarize", "src", "out.pbo"],
        dir.path(),
    );
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("prebinarized-file"), "{}", err);
    assert!(
        err.contains(&format!("{} v56", String::from_utf8_lossy(magic))),
        "{}",
        err
    );
    let (code, out, err) = dayzmake_raw(&["cat", "out.pbo", file], dir.path());
    assert_eq!(code, 0, "{err}");
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
    std::fs::create_dir(&src).unwrap();
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("m.p3d"), b"MLOD\x01\x01\0\0").unwrap();
    let (code, err) = dayzmake_no_tools(
        &["build", "--proton-binarize", "src", "out.pbo"],
        dir.path(),
    );
    assert_ne!(code, 0, "an MLOD must reach binarize.exe: {err}");
}

#[test]
fn binarize_copies_prebinarized_p3d() {
    let dir = tempfile::tempdir().unwrap();
    let body = b"ODOL\x38\0\0\0 not a real model";
    std::fs::write(dir.path().join("m.p3d"), body).unwrap();
    let (code, err) = dayzmake_no_tools(&["binarize", "m.p3d", "out.p3d"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("prebinarized-file"), "{}", err);
    assert_eq!(std::fs::read(dir.path().join("out.p3d")).unwrap(), body);
}

#[test]
fn binarize_reports_older_odol_versions_instead_of_failing() {
    let dir = tempfile::tempdir().unwrap();
    for version in [53u8, 54, 55] {
        let body = [
            b"ODOL".as_slice(),
            &[version, 0, 0, 0],
            b" not a real model",
        ]
        .concat();
        std::fs::write(dir.path().join("m.p3d"), &body).unwrap();
        let (code, err) = dayzmake_no_tools(&["binarize", "-f", "m.p3d", "out.p3d"], dir.path());
        assert_eq!(code, 0, "{err}");
        assert!(err.contains(&format!("ODOL v{version}")), "{}", err);
        assert_eq!(std::fs::read(dir.path().join("out.p3d")).unwrap(), body);
    }
}

#[test]
fn cli_usage_lists_api() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = dayzmake(&["--help"], dir.path());
    assert_eq!(code, 0, "{err}");
    for line in [
        "dayzmake api class",
        "dayzmake api find",
        "dayzmake api sub",
        "dayzmake api enum",
        "dayzmake api chain",
        "dayzmake api src",
        "dayzmake api defines",
        "dayzmake api update",
        "--refresh",
        "--json",
    ] {
        assert!(out.contains(line), "{line} missing from {out}");
    }
}

/// The usage block of the README is the `USAGE` text of the binary.
#[test]
fn readme_usage_block_equals_usage() {
    // A Windows checkout (`text=auto`) has CRLF line endings.
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
        .expect("README")
        .replace("\r\n", "\n");
    let block = readme
        .split("## Usage\n")
        .nth(1)
        .and_then(|rest| rest.split("```\n").nth(1))
        .expect("the usage block");
    assert_eq!(block.trim(), dayzmake::run::USAGE.trim());
}

#[test]
fn build_warnings_come_in_sorted_file_order() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/config.cpp", "class B { y = 2; };\n");
    // created out of order; each file warns once
    for name in ["b", "a", "c"] {
        write(
            dir.path(),
            &format!("m/{name}.rvmat"),
            "class A { x = foo; };\n",
        );
    }
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let order: Vec<&str> = err
        .lines()
        .filter(|l| l.contains("unquoted-string"))
        .map(|l| l.split(':').next().unwrap())
        .collect();
    assert_eq!(order.len(), 3, "{err}");
    assert!(
        order[0].contains("a.rvmat")
            && order[1].contains("b.rvmat")
            && order[2].contains("c.rvmat"),
        "{err}"
    );
}

#[test]
fn suppressed_warnings_are_summarised() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    for i in 0..12 {
        std::fs::write(src.join(format!("c{i:02}.cpp")), b"\0raP\0\0\0\0").unwrap();
    }
    let (code, _, err) = dayzmake(&["build", "src", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(err.matches("[prerapified-file]").count(), 10, "{err}");
    assert!(
        err.contains("2 warnings of type \"prerapified-file\" were suppressed to prevent spam. Use \"-w prerapified-file\" to disable these warnings entirely."),
        "{err}"
    );
}

/// A fake Steam tree whose `proton` creates the Wine prefix but fails every other run.
#[cfg(unix)]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn fake_steam(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let tools = root.join("steamapps/common/DayZ Tools/Bin/Binarize");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(tools.join("binarize.exe"), b"").unwrap();
    let proton_dir = root.join("steamapps/common/Proton Hotfix");
    std::fs::create_dir_all(&proton_dir).unwrap();
    let script = proton_dir.join("proton");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$*\" in\n*cmd.exe*) mkdir -p \"$STEAM_COMPAT_DATA_PATH/pfx/dosdevices\"; exit 0;;\nesac\necho binarize exploded\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn build_failure_with_proton_removes_the_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let steam = dir.path().join("steam");
    let tmp = dir.path().join("tmp");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(&src).unwrap();
    fake_steam(&steam);
    std::fs::write(src.join("m.p3d"), b"MLOD\x01\x01\0\0").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .args(["build", "--proton-binarize", "src", "out.pbo"])
        .current_dir(dir.path())
        .env("STEAM_ROOT", &steam)
        .env_remove("DAYZ_TOOLS")
        .env_remove("PROTON")
        .env("TMPDIR", &tmp)
        .env("HOME", dir.path().join("home"))
        .env("XDG_CONFIG_HOME", dir.path().join("xdg-config"))
        .env("XDG_CACHE_HOME", dir.path().join("xdg-cache"))
        .env("XDG_DATA_HOME", dir.path().join("xdg-data"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("binarize exploded"), "{err}");
    let left: Vec<_> = std::fs::read_dir(&tmp)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "sandbox left behind: {left:?}");
}

/// A fake Steam tree whose `proton` prints two lines, creates the prefix, and "binarizes" by
/// writing a file into the output folder it is given.
#[cfg(unix)]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn chatty_steam(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let tools = root.join("steamapps/common/DayZ Tools/Bin/Binarize");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(tools.join("binarize.exe"), b"").unwrap();
    let proton_dir = root.join("steamapps/common/Proton Hotfix");
    std::fs::create_dir_all(&proton_dir).unwrap();
    let script = proton_dir.join("proton");
    std::fs::write(
        &script,
        "#!/bin/sh\necho first line\necho second line\ncase \"$*\" in\n*cmd.exe*) mkdir -p \"$STEAM_COMPAT_DATA_PATH/pfx/dosdevices\"; exit 0;;\nesac\nroot=$(dirname \"$STEAM_COMPAT_DATA_PATH\")\nout=$(printf '%s' \"$8\" | sed 's/^Z://; s#\\\\#/#g')\necho ODOL > \"$root$out/$9\"\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Proton's progress message and `BIOUTPUT=1` output are plain stderr lines: no `warning:`
/// label, no `[name]`, one line per output line, and no per-name quota.
#[cfg(unix)]
#[test]
fn proton_messages_stay_plain_stderr_lines() {
    let dir = tempfile::tempdir().unwrap();
    let steam = dir.path().join("steam");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    chatty_steam(&steam);
    for i in 0..12 {
        std::fs::write(src.join(format!("m{i:02}.p3d")), b"MLOD\x01\x01\0\0").unwrap();
    }
    let home = dir.path().join("home");
    let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .args(["build", "--proton-binarize", "src", "out.pbo"])
        .current_dir(dir.path())
        .env("STEAM_ROOT", &steam)
        .env_remove("DAYZ_TOOLS")
        .env_remove("PROTON")
        .env("BIOUTPUT", "1")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DATA_HOME", home.join("data"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    let lines: Vec<&str> = err.lines().collect();
    // one prefix run plus twelve models, two lines each
    assert_eq!(
        lines.iter().filter(|l| **l == "first line").count(),
        13,
        "{err}"
    );
    assert_eq!(
        lines.iter().filter(|l| **l == "second line").count(),
        13,
        "{err}"
    );
    assert!(
        lines.contains(&"Creating a Wine prefix for binarize.exe (the first run takes a while)..."),
        "{err}"
    );
    assert!(!err.contains("warning: Creating"), "{err}");
    assert!(!err.contains("[proton"), "{err}");
    assert!(!err.contains("[binarize-output]"), "{err}");
}

#[test]
fn pack_rejects_a_header_extension_without_equals() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), "a").unwrap();
    let (code, _, err) = dayzmake(&["pack", "-e", "bad", ".", "../out.pbo"], &src);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("Invalid header extension \"bad\" (expected key=value)."),
        "{err}"
    );
}

#[test]
fn build_warning_path_is_relative_to_the_addon() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/config.cpp", "class A {\n x = foo;\n};\n");
    let (code, _, err) = dayzmake(&["build", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("In file config.cpp:2: "), "{err}");
    assert!(
        !err.contains("./config.cpp")
            && !err.contains(&Path::new("mod").join("config.cpp").display().to_string()),
        "{err}"
    );

    // "." builds without $PBOPREFIX$ too
    let (code, _, err) = dayzmake(&["build", ".", "../out2.pbo"], &dir.path().join("mod"));
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("In file config.cpp:2: "), "{err}");
    assert!(!err.contains("./config.cpp"), "{err}");
}

#[test]
fn build_warning_in_an_included_file_is_relative_to_the_addon() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/config.cpp", "#include \"inc\\a.hpp\"\n");
    write(dir.path(), "mod/inc/a.hpp", "class A { x = foo; };\n");
    let (code, _, err) = dayzmake(&["build", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let expected = format!("In file {}:1: ", Path::new("inc").join("a.hpp").display());
    assert!(err.contains(&expected), "{err}");
    let root = dir.path().to_string_lossy().into_owned();
    assert!(!err.contains(&root), "{err}");
}

#[test]
fn build_warning_in_an_include_outside_the_addon_keeps_its_path() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/config.cpp", "#include \"\\a.hpp\"\n");
    write(dir.path(), "shared/a.hpp", "class A { x = foo; };\n");
    let (code, _, err) = dayzmake(&["build", "-i", "shared", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    // recorded on the unfixed code: the canonical absolute path
    let shared = dir
        .path()
        .join("shared")
        .canonicalize()
        .unwrap()
        .join("a.hpp");
    let expected = format!("In file {}:1: ", shared.display());
    assert!(err.contains(&expected), "{err}");
}

#[test]
fn missing_input_errors_name_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "x.pbo", "");
    let cases: [(&[&str], &str); 8] = [
        (&["binarize", "nope.p3d", "o.p3d"], "nope.p3d"),
        (&["rapify", "nope.cpp", "x"], "nope.cpp"),
        (&["inspect", "nope.pbo"], "nope.pbo"),
        (&["unpack", "nope.pbo", "d"], "nope.pbo"),
        (
            &["sign", "nokey.biprivatekey", "x.pbo"],
            "nokey.biprivatekey",
        ),
        (&["verify", "nokey.bikey", "x.pbo"], "nokey.bikey"),
        (&["paa2img", "nope.paa", "o.png"], "nope.paa"),
        (&["img2paa", "nope.png", "o.paa"], "nope.png"),
    ];
    for (args, name) in cases {
        let (code, _, err) = dayzmake(args, dir.path());
        assert_eq!(code, 1, "{args:?}: {err}");
        assert!(err.contains(&format!("\"{name}\":")), "{args:?}: {err}");
    }
}

#[test]
fn control_characters_in_a_missing_path_are_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let (code, _, err) = dayzmake(&["rapify", "a\u{1b}[31mb.cpp", "x"], dir.path());
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("a\\u{1b}[31mb.cpp"), "{err}");
    assert!(!err.contains('\u{1b}'), "{err}");
}

#[test]
fn missing_source_folder_is_named() {
    let dir = tempfile::tempdir().unwrap();
    for cmd in ["build", "pack"] {
        let (code, _, err) = dayzmake(&[cmd, "nodir", &format!("{cmd}.pbo")], dir.path());
        assert_eq!(code, 1, "{cmd}: {err}");
        assert!(
            err.contains("Failed to read source folder \"nodir\":"),
            "{cmd}: {err}"
        );
    }
}

#[test]
fn missing_pbo_and_signature_are_named() {
    let dir = tempfile::tempdir().unwrap();
    let (code, _, err) = dayzmake(&["keygen", "k"], dir.path());
    assert_eq!(code, 0, "{err}");
    for args in [
        ["sign", "k.biprivatekey", "gone.pbo"],
        ["verify", "k.bikey", "gone.pbo"],
    ] {
        let (code, _, err) = dayzmake(&args, dir.path());
        assert_eq!(code, 1, "{args:?}: {err}");
        assert!(
            err.contains("Failed to open PBO \"gone.pbo\":"),
            "{args:?}: {err}"
        );
    }
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write(&src, "a.txt", "a");
    let (code, _, err) = dayzmake(&["pack", "src", "x.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = dayzmake(&["verify", "k.bikey", "x.pbo"], dir.path());
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("Failed to open signature \""), "{err}");
}

#[test]
fn unpack_conflict_message_uses_the_target_folder() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/$PBOPREFIX$", "x\\y");
    write(dir.path(), "m/data/a.txt", "a");
    let (code, _, err) = dayzmake(&["pack", "m", "x.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = dayzmake(&["unpack", "x.pbo", "out"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = dayzmake(&["unpack", "x.pbo", "out"], dir.path());
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: Target folder \"out\" already contains files of this PBO (first: \"$PBOPREFIX$\"). Use --force to overwrite.\n"
    );
}

#[test]
fn rapify_nbsp_indent_reports_a_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("nbsp.cpp"), b"\xc2\xa0foo = 1;\n").unwrap();
    let (code, _, err) = dayzmake(&["rapify", "nbsp.cpp", "out.bin"], dir.path());
    assert_eq!(code, 1, "{err}");
    assert!(err.starts_with("error:"), "{err}");
}

/// File names in `dir`, sorted.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn failed_build_keeps_the_existing_target() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/config.cpp", "class A {\n");
    std::fs::write(dir.path().join("out.pbo"), b"good pbo bytes").unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        std::fs::read(dir.path().join("out.pbo")).unwrap(),
        b"good pbo bytes"
    );
    assert_eq!(names(dir.path()), ["mod", "out.pbo"]);
}

#[test]
fn build_of_a_missing_folder_creates_no_target() {
    let dir = tempfile::tempdir().unwrap();
    let (code, _, err) = dayzmake(&["build", "nodir", "out.pbo"], dir.path());
    assert_eq!(code, 1, "{err}");
    assert!(names(dir.path()).is_empty(), "{:?}", names(dir.path()));
}

#[test]
fn successful_build_replaces_the_target() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    std::fs::write(dir.path().join("out.pbo"), b"old").unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let pbo = std::fs::read(dir.path().join("out.pbo")).unwrap();
    assert_ne!(pbo, b"old");
    assert!(pbo.windows(5).any(|w| w == b"hello"));
    assert_eq!(names(dir.path()), ["mod", "out.pbo"]);
}

#[test]
fn failed_rapify_keeps_the_existing_target() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "bad.cpp", "class A {\n");
    std::fs::write(dir.path().join("c.bin"), b"good bin").unwrap();
    let (code, _, err) = dayzmake(&["rapify", "-f", "bad.cpp", "c.bin"], dir.path());
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        std::fs::read(dir.path().join("c.bin")).unwrap(),
        b"good bin"
    );
    assert_eq!(names(dir.path()), ["bad.cpp", "c.bin"]);
}

#[cfg(unix)]
#[test]
fn build_keeps_the_permissions_of_the_existing_target() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let out = dir.path().join("out.pbo");
    std::fs::write(&out, b"old").unwrap();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(windows)]
#[test]
fn build_writes_to_a_windows_device_name_directly() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/a.txt", "hello");
    let (code, _, err) = dayzmake(&["build", "m", "NUL"], dir.path());
    assert_eq!(code, 0, "{err}");
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["m"], "no temp file or NUL file is left");
}

/// #62: a target named like one of the newer device names is written as a device, not as a file.
#[cfg(windows)]
#[test]
fn build_writes_to_a_conout_device_name_directly() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/a.txt", "hello");
    let (code, _, err) = dayzmake(&["build", "m", "CONOUT$"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(!dir.path().join("CONOUT$").exists());
}

/// A regular file reached under a device name through a verbatim path (`\\?\C:\tmp\NUL`).
#[cfg(windows)]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn verbatim_nul_file(dir: &std::path::Path) -> String {
    let verbatim = format!(r"\\?\{}\NUL", dir.display());
    std::fs::write(&verbatim, vec![b'x'; 100_000]).unwrap();
    verbatim
}

/// Whether `bytes` start like a PBO (an empty first name, then the `sreV` product entry).
#[cfg(windows)]
fn looks_like_a_pbo(bytes: &[u8]) -> bool {
    bytes.windows(4).take(8).any(|w| w == b"sreV")
}

#[cfg(windows)]
#[test]
fn build_refuses_a_regular_file_with_a_device_name_without_force() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/a.txt", "hello");
    let target = verbatim_nul_file(dir.path());
    let (code, _, err) = dayzmake(&["build", "m", &target], dir.path());
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(std::fs::read(&target).unwrap(), vec![b'x'; 100_000]);
}

#[cfg(windows)]
#[test]
fn build_force_replaces_and_truncates_a_regular_file_with_a_device_name() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/a.txt", "hello");
    let target = verbatim_nul_file(dir.path());
    let (code, _, err) = dayzmake(&["build", "--force", "m", &target], dir.path());
    assert_eq!(code, 0, "{err}");
    let written = std::fs::read(&target).unwrap();
    assert!(looks_like_a_pbo(&written), "not a PBO");
    assert!(written.len() < 100_000, "the old tail is gone");
}

#[cfg(windows)]
#[test]
fn build_creates_a_new_target_named_like_a_device() {
    // does this Windows treat `nul.probe` as a file? (older ones treat reserved names with any extension as devices)
    let scratch = tempfile::tempdir().unwrap();
    let names_with_an_extension_are_files = std::fs::File::create(scratch.path().join("nul.probe"))
        .is_ok()
        && std::fs::read_dir(scratch.path()).unwrap().count() == 1;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/a.txt", "hello");
    let (code, _, err) = dayzmake(&["build", "m", "aux.pbo"], dir.path());
    if names_with_an_extension_are_files {
        assert_eq!(code, 0, "{err}");
        let bytes = std::fs::read(dir.path().join("aux.pbo")).unwrap();
        assert!(looks_like_a_pbo(&bytes), "not a PBO");
    } else if code != 0 {
        // the device took the name, but the target is not reported as missing
        let lower = err.to_lowercase();
        assert!(
            !lower.contains("not found") && !lower.contains("cannot find"),
            "{err}"
        );
    }
}

#[cfg(windows)]
#[test]
fn build_writes_to_con_in_place() {
    // without a console (a CI runner with redirected handles) CON cannot be opened: nothing to test
    if std::fs::OpenOptions::new()
        .write(true)
        .open("CONOUT$")
        .is_err()
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/a.txt", "hello");
    let (code, _, err) = dayzmake(&["build", "m", "CON"], dir.path());
    assert_eq!(code, 0, "{err}");
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["m"], "no temp file or CON file is left");
}

#[cfg(windows)]
#[test]
fn build_reports_a_read_only_regular_file_with_a_device_name_as_existing() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/a.txt", "hello");
    let target = verbatim_nul_file(dir.path());
    let mut permissions = std::fs::metadata(&target).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&target, permissions).unwrap();
    let (code, _, err) = dayzmake(&["build", "m", &target], dir.path());
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("already exists"), "{err}");
    let mut permissions = std::fs::metadata(&target).unwrap().permissions();
    #[expect(clippy::permissions_set_readonly_false, reason = "test cleanup")]
    permissions.set_readonly(false);
    std::fs::set_permissions(&target, permissions).unwrap();
}

#[cfg(unix)]
#[test]
fn build_force_replaces_a_read_only_target_and_keeps_it_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let out = dir.path().join("out.pbo");
    std::fs::write(&out, b"old").unwrap();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (code, _, err) = dayzmake(&["build", "--force", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_ne!(std::fs::read(&out).unwrap(), b"old");
    assert_eq!(
        std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
        0o444
    );
}

#[cfg(windows)]
#[test]
fn build_force_refuses_a_read_only_target_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let out = dir.path().join("out.pbo");
    std::fs::write(&out, b"old").unwrap();
    let mut permissions = std::fs::metadata(&out).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&out, permissions).unwrap();
    let (code, _, err) = dayzmake(&["build", "--force", "mod", "out.pbo"], dir.path());
    assert_ne!(code, 0, "{err}");
    assert_eq!(std::fs::read(&out).unwrap(), b"old");
    let mut names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["mod", "out.pbo"], "no temp file is left");
    let mut permissions = std::fs::metadata(&out).unwrap().permissions();
    #[expect(clippy::permissions_set_readonly_false, reason = "test cleanup")]
    permissions.set_readonly(false);
    std::fs::set_permissions(&out, permissions).unwrap();
}

#[cfg(unix)]
#[test]
fn build_into_a_read_only_directory_names_the_directory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let out_dir = dir.path().join("ro");
    std::fs::create_dir(&out_dir).unwrap();
    std::fs::set_permissions(&out_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "ro/out.pbo"], dir.path());
    std::fs::set_permissions(&out_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    // root ignores directory permissions
    if code == 0 {
        return;
    }
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("Failed to open output file: cannot create a temporary file in \""),
        "{err}"
    );
    assert!(err.contains("ro\":"), "{err}");
}

#[cfg(unix)]
#[test]
fn build_of_a_bare_name_in_a_read_only_cwd_names_the_current_directory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "out.pbo"], dir.path());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    // root ignores directory permissions
    if code == 0 {
        return;
    }
    assert!(
        err.contains("cannot create a temporary file in \".\":"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn build_writes_through_a_symlinked_target() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    std::fs::create_dir(dir.path().join("real")).unwrap();
    std::fs::write(dir.path().join("real/out.pbo"), b"old").unwrap();
    std::os::unix::fs::symlink("real/out.pbo", dir.path().join("out.pbo")).unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(
        std::fs::symlink_metadata(dir.path().join("out.pbo"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let real = std::fs::read(dir.path().join("real/out.pbo")).unwrap();
    assert!(real.windows(5).any(|w| w == b"hello"));
    assert_eq!(names(&dir.path().join("real")), ["out.pbo"]);
}

#[cfg(unix)]
#[test]
fn build_writes_through_a_dangling_symlink() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    std::fs::create_dir(dir.path().join("real")).unwrap();
    std::os::unix::fs::symlink("real/out.pbo", dir.path().join("out.pbo")).unwrap();
    let (code, _, err) = dayzmake(&["build", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(dir.path().join("real/out.pbo").is_file());
}

#[test]
fn a_directory_target_is_rejected_up_front() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    std::fs::create_dir(dir.path().join("out.pbo")).unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 1);
    assert!(err.contains("is a directory"), "{err}");
    // nothing was built or written: no output, no temp file, no warnings
    assert_eq!(names(dir.path()), ["mod", "out.pbo"]);
    assert_eq!(names(&dir.path().join("out.pbo")), Vec::<String>::new());
    assert!(!err.contains("arning"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_target_is_rejected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    symlink("self.pbo", dir.path().join("self.pbo")).unwrap();
    symlink("two-b.pbo", dir.path().join("two-a.pbo")).unwrap();
    symlink("two-a.pbo", dir.path().join("two-b.pbo")).unwrap();
    for target in ["self.pbo", "two-a.pbo"] {
        let (code, _, err) = dayzmake(&["build", "-f", "mod", target], dir.path());
        assert_eq!(code, 1, "{target}: {err}");
        assert!(
            err.contains("is a symlink loop or chain that cannot be resolved"),
            "{target}: {err}"
        );
    }
    assert_eq!(
        names(dir.path()),
        ["mod", "self.pbo", "two-a.pbo", "two-b.pbo"]
    );
}

#[test]
fn atomic_target_commit_failure_removes_the_temp_file() {
    use dayzmake::cli_io::AtomicTarget;
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.pbo");
    let mut t = AtomicTarget::create(&target, false).unwrap();
    t.write_all(b"data").unwrap();
    // a directory appears at the target before the rename
    std::fs::create_dir(&target).unwrap();
    let err = t.commit().unwrap_err().to_string();
    assert!(err.contains("out.pbo"), "{err}");
    assert_eq!(names(dir.path()), ["out.pbo"]);
}

#[test]
fn atomic_target_skips_an_existing_temp_name() {
    use dayzmake::cli_io::AtomicTarget;
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.pbo");
    let taken = dir
        .path()
        .join(format!(".out.pbo.dayzmake-tmp-{}-0", std::process::id()));
    std::fs::write(&taken, b"foreign").unwrap();
    let mut t = AtomicTarget::create(&target, false).unwrap();
    t.write_all(b"data").unwrap();
    t.commit().unwrap();
    assert_eq!(std::fs::read(&taken).unwrap(), b"foreign");
    assert_eq!(std::fs::read(&target).unwrap(), b"data");
}

#[cfg(unix)]
#[test]
fn keygen_creates_the_private_key_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let mode = |n: &str| {
        std::fs::metadata(dir.path().join(n))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    let (code, _, err) = dayzmake(&["keygen", "k"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(mode("k.biprivatekey"), 0o600);
    // the public key keeps the default mode: the one a file created here gets (umask applied)
    std::fs::write(dir.path().join("probe"), b"").unwrap();
    assert_eq!(
        mode("k.bikey"),
        mode("probe"),
        "the public key keeps the default mode"
    );
    // over a wider existing key, and over a 0600 one
    std::fs::set_permissions(
        dir.path().join("k.biprivatekey"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let (code, _, err) = dayzmake(&["keygen", "-f", "k"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(mode("k.biprivatekey"), 0o600);
    let (code, _, err) = dayzmake(&["keygen", "-f", "k"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(mode("k.biprivatekey"), 0o600);
}

/// The `prefix` header extension of the PBO at `path`.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn pbo_prefix(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).unwrap();
    let pbo = dayzmake_core::pbo::PBO::read(&mut f).unwrap();
    pbo.header_extension("prefix").map(str::to_string)
}

#[test]
fn build_dot_uses_the_folder_name_as_prefix() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mymod/config.cpp", "class A {};\n");
    let inside = dir.path().join("mymod");
    let (code, _, err) = dayzmake(&["build", ".", "../a.pbo"], &inside);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = dayzmake(&["build", "./", "../b.pbo"], &inside);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = dayzmake(&["build", "mymod", "c.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let a = std::fs::read(dir.path().join("a.pbo")).unwrap();
    assert_eq!(a, std::fs::read(dir.path().join("b.pbo")).unwrap());
    assert_eq!(a, std::fs::read(dir.path().join("c.pbo")).unwrap());
    assert_eq!(
        pbo_prefix(&dir.path().join("a.pbo")).as_deref(),
        Some("mymod")
    );
}

#[cfg(unix)]
#[test]
fn build_through_a_symlink_keeps_the_link_name_as_prefix() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "real/config.cpp", "class A {};\n");
    std::os::unix::fs::symlink("real", dir.path().join("mylink")).unwrap();
    let (code, _, err) = dayzmake(&["build", "mylink", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        pbo_prefix(&dir.path().join("out.pbo")).as_deref(),
        Some("mylink")
    );
}

#[test]
fn empty_value_warns_and_can_be_muted() {
    let (code, err) = rapify_str("class A { x = ; };\n");
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("config.cpp:1") && err.contains("empty-value"),
        "{err}"
    );
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "config.cpp", "class A { x = ; };\n");
    let (code, _, err) = dayzmake(
        &["-w", "empty-value", "rapify", "config.cpp", "c.bin"],
        dir.path(),
    );
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("empty-value"), "{err}");
}

#[test]
fn rapify_rejects_a_nul_in_a_string_and_names_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "config.cpp", "class A { a = \"x\0y\"; };\n");
    let (code, _, err) = dayzmake(&["rapify", "config.cpp", "c.bin"], dir.path());
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("Entry \"a\""), "{err}");
    assert!(!dir.path().join("c.bin").exists());
}

#[cfg(unix)]
#[test]
fn build_writes_to_dev_null() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "/dev/null"], dir.path());
    assert_eq!(code, 0, "{err}");
}

#[cfg(unix)]
#[test]
fn build_writes_to_dev_stdout_through_a_pipe() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let (code, out, err) = dayzmake_raw(&["build", "-f", "mod", "/dev/stdout"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(out.windows(5).any(|w| w == b"hello"), "{out:?}");
    assert_eq!(names(dir.path()), ["mod"]);
}

#[cfg(unix)]
#[test]
fn build_writes_into_a_fifo_and_keeps_it() {
    use std::os::unix::fs::FileTypeExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let fifo = dir.path().join("f.pbo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let reader = {
        let fifo = fifo.clone();
        std::thread::spawn(move || std::fs::read(fifo).unwrap())
    };
    // our own write end, held until dayzmake is done: the reader sees EOF even if dayzmake never opens the FIFO
    let keep_open = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "f.pbo"], dir.path());
    drop(keep_open);
    let bytes = reader.join().unwrap();
    assert_eq!(code, 0, "{err}");
    assert!(bytes.windows(5).any(|w| w == b"hello"));
    assert!(
        std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo()
    );
    assert_eq!(names(dir.path()), ["f.pbo", "mod"]);
}

#[cfg(unix)]
#[test]
fn build_writes_through_a_symlink_to_a_fifo() {
    use std::os::unix::fs::FileTypeExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod/a.txt", "hello");
    let fifo = dir.path().join("f.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    std::os::unix::fs::symlink("f.fifo", dir.path().join("link.pbo")).unwrap();
    let reader = {
        let fifo = fifo.clone();
        std::thread::spawn(move || std::fs::read(fifo).unwrap())
    };
    let keep_open = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    let (code, _, err) = dayzmake(&["build", "-f", "mod", "link.pbo"], dir.path());
    drop(keep_open);
    let bytes = reader.join().unwrap();
    assert_eq!(code, 0, "{err}");
    assert!(bytes.windows(5).any(|w| w == b"hello"));
    assert!(
        std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo()
    );
    assert!(
        std::fs::symlink_metadata(dir.path().join("link.pbo"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(names(dir.path()), ["f.fifo", "link.pbo", "mod"]);
}

/// Runs `args` twice in `cwd` (the second with `--force`), checks both runs gave the same bytes and that `inspect`
/// lists neither the target nor a temp entry.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn build_twice_is_identical(args: &[&str], cwd: &Path, pbo: &Path) {
    let mut forced = args.to_vec();
    forced.push("--force");
    let (code, _, err) = dayzmake(args, cwd);
    assert_eq!(code, 0, "{err}");
    let first = std::fs::read(pbo).unwrap();
    let (code, _, err) = dayzmake(&forced, cwd);
    assert_eq!(code, 0, "{err}");
    assert_eq!(first, std::fs::read(pbo).unwrap());
    let (code, out, err) = dayzmake(&["inspect", pbo.to_str().unwrap()], cwd);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("config.bin") || out.contains("config.cpp"),
        "{out}"
    );
    assert!(!out.contains("out.pbo"), "{out}");
    assert!(!out.contains("dayzmake-tmp"), "{out}");
}

#[test]
fn build_inside_source_folder_does_not_pack_output() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    build_twice_is_identical(&["build", ".", "out.pbo"], &modf, &modf.join("out.pbo"));
}

#[test]
fn build_target_inside_source_from_parent_does_not_pack_output() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    build_twice_is_identical(
        &["build", "mod", "mod/out.pbo"],
        dir.path(),
        &dir.path().join("mod/out.pbo"),
    );
}

#[test]
fn pack_inside_source_folder_does_not_pack_output() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    build_twice_is_identical(&["pack", ".", "out.pbo"], &modf, &modf.join("out.pbo"));
}

#[cfg(unix)]
#[test]
fn pack_through_symlinked_target_does_not_pack_it() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};"), ("dist/.keep", "")]);
    let modf = dir.path().join("mod");
    std::os::unix::fs::symlink("../dist/out.pbo", modf.join("out.pbo")).unwrap();
    build_twice_is_identical(
        &["pack", ".", "out.pbo"],
        &modf,
        &dir.path().join("dist/out.pbo"),
    );
}

/// Runs dayzmake like `dayzmake`, but as a child with a deadline: a build that hangs fails the
/// test (the child is killed) instead of blocking it. Returns (exit code, stderr).
///
/// # Panics
///
/// Panics if the binary cannot be spawned or does not exit within 20 seconds (it is killed first).
#[cfg(unix)]
#[must_use]
#[expect(clippy::expect_used, clippy::panic, reason = "test helper")]
fn dayzmake_with_deadline(args: &[&str], cwd: &Path) -> (i32, String) {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let home = tempfile::tempdir().expect("failed to create a temp HOME");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    isolate(&mut cmd, home.path());
    let mut child = cmd.spawn().expect("failed to run dayzmake");
    let mut stderr = child.stderr.take().expect("stderr");
    // drain stderr on a thread so a full pipe cannot stall the child
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text); // a read error only shortens the diagnostics
        String::from_utf8_lossy(&text).into_owned()
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill(); // already gone is fine: the test fails below either way
            let _ = child.wait();
            panic!("build hung: killed {args:?} after 20 s");
        }
        // a watchdog poll for the deadline, not a wait for state: the child's exit ends the loop
        std::thread::sleep(Duration::from_millis(10));
    };
    (
        status.code().unwrap_or(-1),
        reader.join().expect("stderr reader"),
    )
}

#[cfg(unix)]
#[test]
fn pack_into_fifo_inside_source_completes() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    let fifo = modf.join("f.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let reader = {
        let fifo = fifo.clone();
        std::thread::spawn(move || std::fs::read(fifo).unwrap())
    };
    // held open so the reader sees EOF and the test cannot hang even if dayzmake never opens the FIFO
    let keep_open = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    let (code, err) = dayzmake_with_deadline(&["pack", ".", "f.fifo", "--force"], &modf);
    drop(keep_open);
    assert_eq!(code, 0, "{err}");
    let data = reader.join().unwrap();
    assert!(data.windows(10).any(|w| w == b"config.cpp"), "no PBO read");
    assert!(!data.windows(6).any(|w| w == b"f.fifo"));
}

#[test]
fn stale_temp_is_skipped_but_same_name_in_subfolder_is_packed() {
    let dir = tempdir_with(&[
        ("mod/config.cpp", "class A{x=1;};"),
        ("mod/.out.pbo.dayzmake-tmp-999-0", "stale"),
        ("mod/d/out.pbo", "data"),
    ]);
    let modf = dir.path().join("mod");
    let (code, _, err) = dayzmake(&["pack", ".", "out.pbo"], &modf);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = dayzmake(&["inspect", "out.pbo"], &modf);
    assert!(!out.contains("dayzmake-tmp"), "{out}");
    assert!(out.contains("out.pbo"), "{out}");
}

#[cfg(unix)]
#[test]
fn pack_skips_file_names_that_are_unsafe_as_entry_names() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};"), ("mod/ok.txt", "x")]);
    let modf = dir.path().join("mod");
    write(&modf, "a\x1bb.txt", "bad");
    let (code, _, err) = dayzmake(&["pack", ".", "out.pbo"], &modf);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("unsafe-path"), "{err}");
    assert!(err.contains("a\\u{1b}b.txt"), "{err}");
    assert!(!err.contains('\x1b'), "{err:?}");
    let (_, out, _) = dayzmake(&["inspect", "out.pbo"], &modf);
    assert!(out.contains("ok.txt"), "{out}");
    assert!(!out.contains("b.txt"), "{out}");
}

#[cfg(unix)]
#[test]
fn pack_skips_fifos_and_device_links_in_the_source() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    assert!(
        Command::new("mkfifo")
            .arg(modf.join("pipe"))
            .status()
            .unwrap()
            .success()
    );
    std::os::unix::fs::symlink("/dev/zero", modf.join("zero")).unwrap();
    let (code, err) = dayzmake_with_deadline(&["pack", ".", "../out.pbo"], &modf);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("unsupported-file"), "{err}");
    assert!(err.contains("pipe") && err.contains("zero"), "{err}");
    let (_, out, _) = dayzmake(&["inspect", "out.pbo"], dir.path());
    assert!(out.contains("config.cpp"), "{out}");
    assert!(!out.contains("pipe") && !out.contains("zero"), "{out}");
}

#[cfg(unix)]
#[test]
fn pack_skips_a_fifo_named_like_the_prefix_file() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    assert!(
        Command::new("mkfifo")
            .arg(modf.join("$PBOPREFIX$"))
            .status()
            .unwrap()
            .success()
    );
    let (code, err) = dayzmake_with_deadline(&["pack", ".", "../out.pbo"], &modf);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("unsupported-file"), "{err}");
}

#[cfg(unix)]
#[test]
fn pack_warns_about_names_that_are_not_utf8_and_packs_neither() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    for n in [&b"x\xff.txt"[..], &b"x\xfe.txt"[..]] {
        std::fs::write(modf.join(std::ffi::OsStr::from_bytes(n)), "d").unwrap();
    }
    let (code, _, err) = dayzmake(&["pack", ".", "../out.pbo"], &modf);
    assert_eq!(code, 0, "{err}");
    assert_eq!(err.matches("unsafe-path").count(), 2, "{err}");
    let (_, out, _) = dayzmake(&["inspect", "out.pbo"], dir.path());
    assert!(
        out.contains("config.cpp") && out.contains("# Files: 1"),
        "{out}"
    );
}

#[cfg(unix)]
#[test]
fn pack_warns_about_names_that_differ_only_in_case() {
    let dir = tempdir_with(&[
        ("mod/config.cpp", "class A{x=1;};"),
        ("mod/A.txt", "upper"),
        ("mod/a.txt", "lower"),
    ]);
    let modf = dir.path().join("mod");
    let (code, _, err) = dayzmake(&["pack", ".", "../out.pbo"], &modf);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("duplicate-entry"), "{err}");
    assert!(err.contains("a.txt") && err.contains("A.txt"), "{err}");
    let (_, out, _) = dayzmake(&["inspect", "out.pbo"], dir.path());
    assert_eq!(out.to_lowercase().matches("a.txt").count(), 1, "{out}");
}

#[test]
fn build_warns_when_config_cpp_and_config_bin_collide() {
    let dir = tempdir_with(&[
        ("mod/config.cpp", "class A{x=1;};"),
        ("mod/config.bin", "bin"),
    ]);
    let (code, _, err) = dayzmake(&["build", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("duplicate-entry"), "{err}");
    assert!(
        err.contains("config.cpp") && err.contains("config.bin"),
        "{err}"
    );
}

#[test]
fn build_prefers_the_compiled_config_cpp_over_a_stale_config_bin() {
    let dir = tempdir_with(&[
        ("mod/config.cpp", "class CfgPatches{class A{units[]={};};};"),
        ("mod/config.bin", "stale"),
    ]);
    let (code, _, err) = dayzmake(&["build", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("duplicate-entry"), "{err}");
    assert!(err.contains("Skipping \"config.bin\""), "{err}");
    let (code, out, err) = dayzmake(&["cat", "out.pbo", "config.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains("stale"), "{out}");
    assert!(out.contains("CfgPatches"), "{out}");
}

#[cfg(unix)]
#[test]
fn build_skips_a_plain_file_replaced_by_a_generated_entry_without_opening_it() {
    let dir = tempdir_with(&[("mod/config.cpp", "class CfgPatches{class A{units[]={};};};")]);
    // opening this would fail the build, so it must be skipped before it is processed
    std::os::unix::fs::symlink("nowhere", dir.path().join("mod/config.bin")).unwrap();
    let (code, _, err) = dayzmake(&["build", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("duplicate-entry"), "{err}");
    assert!(err.contains("Skipping \"config.bin\""), "{err}");
    let (code, out, err) = dayzmake(&["cat", "out.pbo", "config.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("CfgPatches"), "{out}");
}

#[test]
fn build_prefers_the_p3do_over_a_plain_p3d() {
    let dir = tempdir_with(&[("m/config.cpp", "class A{x=1;};")]);
    std::fs::write(dir.path().join("m/m.p3d"), b"plain-p3d").unwrap();
    std::fs::write(dir.path().join("m/m.p3do"), b"from-p3do").unwrap();
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("Skipping \"m.p3d\""), "{err}");
    let (code, out, err) = dayzmake(&["cat", "out.pbo", "m.p3d"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "from-p3do");
}

#[cfg(unix)]
#[test]
fn build_replace_warning_names_the_generated_entry_not_the_plain_spelling() {
    // #41: the skipped plain file keeps its spelling, the entry named is the one that is stored
    let dir = tempdir_with(&[
        ("mod/config.cpp", "class CfgPatches{class A{units[]={};};};"),
        ("mod/Config.bin", "stale"),
        ("mod/M.p3do", "from-p3do"),
        ("mod/m.P3D", "plain"),
    ]);
    let (code, _, err) = dayzmake(&["build", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains(
            "Skipping \"Config.bin\": the entry \"config.bin\" is generated from \"config.cpp\""
        ),
        "{err}"
    );
    assert!(
        err.contains("Skipping \"m.P3D\": the entry \"M.p3d\" is generated from \"M.p3do\""),
        "{err}"
    );
}

#[test]
fn build_skips_a_prebinarized_plain_p3d_next_to_a_p3do_without_inspecting_it() {
    // #41: the plain file is replaced, so it is neither inspected nor reported as pre-binarized
    let dir = tempdir_with(&[("m/config.cpp", "class A{x=1;};")]);
    std::fs::write(dir.path().join("m/m.p3d"), b"ODOL\x31\0\0\0 model").unwrap();
    std::fs::write(dir.path().join("m/m.p3do"), b"from-p3do").unwrap();
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("duplicate-entry"), "{err}");
    assert!(!err.contains("prebinarized-file"), "{err}");
    assert!(!err.contains("In file m.p3d"), "{err}");
    let (code, out, err) = dayzmake(&["cat", "out.pbo", "m.p3d"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "from-p3do");
}

#[cfg(unix)]
#[test]
fn pack_names_a_dangling_symlink_in_the_error() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    std::os::unix::fs::symlink("nowhere", modf.join("dangling")).unwrap();
    let (code, _, err) = dayzmake(&["pack", ".", "../out.pbo"], &modf);
    assert_ne!(code, 0);
    assert!(err.contains("dangling"), "{err}");
}

#[cfg(unix)]
#[test]
fn build_excluded_special_and_unsafe_files_are_silent() {
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    assert!(
        Command::new("mkfifo")
            .arg(modf.join("pipe"))
            .status()
            .unwrap()
            .success()
    );
    write(&modf, "bad\x1bname.txt", "x");
    let (code, _, err) = dayzmake(
        &["build", "-x", "pipe", "-x", "bad*", "mod", "out.pbo"],
        dir.path(),
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        !err.contains("unsupported-file") && !err.contains("unsafe-path"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn build_matches_model_extensions_case_insensitively() {
    let dir = tempdir_with(&[("m/config.cpp", "class A{x=1;};")]);
    std::fs::write(dir.path().join("m/N.P3D"), b"MLOD\x01\x01\0\0").unwrap();
    std::fs::write(dir.path().join("m/xp3d"), b"plain").unwrap();
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("non-windows-binarization"), "{err}");
    assert!(err.contains("N.P3D"), "{err}");
    assert_eq!(err.matches("non-windows-binarization").count(), 1, "{err}");
}

#[cfg(unix)]
#[test]
fn build_warns_when_p3d_and_p3do_collide_and_renames_p3do() {
    let dir = tempdir_with(&[("m/config.cpp", "class A{x=1;};")]);
    std::fs::write(dir.path().join("m/a.p3d"), b"MLOD\x01\x01\0\0").unwrap();
    std::fs::write(dir.path().join("m/a.p3do"), b"other").unwrap();
    std::fs::write(dir.path().join("m/b.P3DO"), b"other").unwrap();
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("duplicate-entry"), "{err}");
    let (_, out, _) = dayzmake(&["inspect", "out.pbo"], dir.path());
    assert_eq!(out.matches("a.p3d").count(), 1, "{out}");
    assert!(out.contains("b.p3d") && !out.contains("p3do"), "{out}");
}

#[test]
fn pack_force_over_the_output_spelled_in_another_case_does_not_pack_the_old_output() {
    // #37: on a case-insensitive file system (Windows, macOS default) `OUT.PBO` is `out.pbo`; elsewhere the
    // probe fails and there is nothing to test
    let dir = tempdir_with(&[("mod/config.cpp", "class A{x=1;};")]);
    let modf = dir.path().join("mod");
    let (code, _, err) = dayzmake(&["pack", ".", "out.pbo"], &modf);
    assert_eq!(code, 0, "{err}");
    if !modf.join("OUT.PBO").exists() {
        return;
    }
    let (code, _, err) = dayzmake(&["pack", ".", "OUT.PBO", "--force"], &modf);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = dayzmake(&["inspect", "OUT.PBO"], &modf);
    assert!(out.contains("config.cpp"), "{out}");
    assert!(!out.to_lowercase().contains("out.pbo"), "{out}");
}

/// Runs `dayzmake <args>` in `cwd` with stdout redirected to the file `out`, like `dayzmake ... > out`.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn dayzmake_to_file(args: &[&str], cwd: &Path, out: &Path) -> i32 {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(args)
        .current_dir(cwd)
        .stdout(std::fs::File::create(out).unwrap());
    isolate(&mut cmd, home.path());
    cmd.status().unwrap().code().unwrap_or(-1)
}

#[test]
fn pack_and_build_leave_a_redirected_stdout_out_of_the_pbo() {
    for cmd in ["pack", "build"] {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("m");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.txt"), "hi").unwrap();
        let out = src.join("out.pbo");
        assert_eq!(dayzmake_to_file(&[cmd, "m"], dir.path(), &out), 0, "{cmd}");
        let (code, listing, err) = dayzmake(&["inspect", "m/out.pbo"], dir.path());
        assert_eq!(code, 0, "{cmd}: {err}");
        assert!(listing.contains("a.txt"), "{cmd}: {listing}");
        assert!(!listing.contains("out.pbo"), "{cmd}: {listing}");
    }
}

#[test]
fn pack_with_stdout_to_a_pipe_or_null_packs_every_file() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("m");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("a.txt"), "hi").unwrap();
    std::fs::write(src.join("b.txt"), "yo").unwrap();
    // a pipe: dayzmake_raw captures stdout through one
    let (code, out, err) = dayzmake_raw(&["pack", "m"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(out.windows(5).any(|w| w == b"a.txt") && out.windows(5).any(|w| w == b"b.txt"));
    // /dev/null
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(["pack", "m"])
        .current_dir(dir.path())
        .stdout(std::process::Stdio::null());
    isolate(&mut cmd, home.path());
    assert_eq!(cmd.status().unwrap().code(), Some(0));
}

/// Packs a mod with one 1 MiB file (more than a pipe buffer holds) into `pbo`.
#[cfg(unix)]
fn big_pbo(dir: &Path) {
    let big = "x".repeat(1 << 20);
    write(dir, "mod/big.txt", &big);
    write(dir, "mod/config.cpp", "class A{x=1;};");
    let (code, _, err) = dayzmake(&["pack", "mod", "out.pbo"], dir);
    assert_eq!(code, 0, "{err}");
}

/// Runs `args` with stdout a pipe: reads one chunk, drops the read end while the child still has output to
/// write, and returns (exit code, stderr). The child blocks on the full pipe until the read end goes away,
/// so no timing is involved.
#[cfg(unix)]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn run_with_closed_stdout(args: &[&str], cwd: &Path) -> (i32, String) {
    use std::io::Read;
    use std::process::Stdio;

    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate(&mut cmd, home.path());
    let mut child = cmd.spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0u8; 16];
    stdout.read_exact(&mut first).unwrap();
    drop(stdout);
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[cfg(unix)]
#[test]
fn closed_stdout_pipe_ends_cat_quietly() {
    let dir = tempfile::tempdir().unwrap();
    big_pbo(dir.path());
    let (code, err) = run_with_closed_stdout(&["cat", "out.pbo", "big.txt"], dir.path());
    assert!(!err.contains("error"), "{err}");
    assert_eq!(code, 0, "{err}");
}

#[cfg(unix)]
#[test]
fn closed_stdout_pipe_ends_inspect_quietly() {
    let dir = tempfile::tempdir().unwrap();
    // a listing longer than a pipe buffer
    for i in 0..2000 {
        write(dir.path(), &format!("mod/{}{i}.txt", "n".repeat(120)), "x");
    }
    let (code, _, err) = dayzmake(&["pack", "mod", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, err) = run_with_closed_stdout(&["inspect", "out.pbo"], dir.path());
    assert!(!err.contains("error"), "{err}");
    assert_eq!(code, 0, "{err}");
}

/// Runs `args` in `cwd` with stdout discarded and stderr a pipe whose read end is already closed.
#[cfg(unix)]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn run_with_closed_stderr(args: &[&str], cwd: &Path) -> Option<i32> {
    use std::process::Stdio;

    let home = tempfile::tempdir().unwrap();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(writer);
    isolate(&mut cmd, home.path());
    cmd.status().unwrap().code()
}

#[cfg(unix)]
#[test]
fn closed_stderr_pipe_still_fails() {
    // the error line cannot be written; the exit status stays 1 and nothing panics
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        run_with_closed_stderr(&["inspect", "nope.pbo"], dir.path()),
        Some(1)
    );
}

#[cfg(unix)]
#[test]
fn closed_stderr_pipe_with_a_usage_error_still_exits_1() {
    // docopt's own error exit would panic on the closed pipe
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(run_with_closed_stderr(&["--bogus"], dir.path()), Some(1));
    assert_eq!(run_with_closed_stderr(&["build"], dir.path()), Some(1));
}

#[cfg(unix)]
#[test]
fn closed_stderr_pipe_does_not_fail_a_command_with_warnings() {
    // an unpacked entry with a leading separator makes unpack warn (leading-separator)
    let dir = tempfile::tempdir().unwrap();
    let pbo = support::raw_pbo(&[("\\scripts\\a.c", b"a")], &[]);
    std::fs::write(dir.path().join("in.pbo"), pbo).unwrap();
    assert_eq!(
        run_with_closed_stderr(&["unpack", "in.pbo", "target"], dir.path()),
        Some(0)
    );
    assert!(dir.path().join("target/scripts/a.c").exists());
}

#[cfg(unix)]
#[test]
fn a_broken_pipe_on_a_fifo_file_target_still_fails() {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = tempfile::tempdir().unwrap();
    big_pbo(dir.path());
    let fifo = dir.path().join("sink");
    let made = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(made.success());
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(["pack", "-f", "mod", "sink"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    isolate(&mut cmd, home.path());
    let mut child = cmd.spawn().unwrap();
    // open the read end without blocking, retrying until the child has opened its end; a child that
    // failed first ends the wait instead of blocking forever
    // O_NONBLOCK: 0o4000 on Linux, 4 on the BSDs and macOS
    let o_nonblock = if cfg!(target_os = "linux") { 0o4000 } else { 4 };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut reader = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(o_nonblock)
        .open(&fifo)
        .unwrap();
    let mut first = [0u8; 16];
    loop {
        match reader.read(&mut first) {
            Ok(n) if n > 0 => break,
            // no writer yet (0) or no data yet (WouldBlock): retry unless the child is gone or time is up
            Ok(_) | Err(_) => {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "the child ended before writing"
                );
                assert!(std::time::Instant::now() < deadline, "timed out");
                std::thread::yield_now();
            }
        }
    }
    drop(reader);
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs `dayzmake preprocess e.cpp` on the files in a child with a 2 GiB address-space limit and a 30 s
/// deadline (`timeout` kills it, exit 124), so a regressed cap fails the test instead of exhausting the
/// machine. Returns (exit code, stderr); `None` when the platform has no `ulimit -v` or `timeout` (the
/// child exits 99 before running dayzmake): the test is skipped, never run unlimited.
#[cfg(unix)]
#[expect(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test helper: failures panic, the skip is reported on stderr"
)]
fn preprocess_limited(files: &[(&str, &str)]) -> Option<(Option<i32>, String)> {
    let dir = tempdir_with(files);
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(
            "ulimit -v 2097152 || exit 99; command -v timeout >/dev/null || exit 99; \
             exec timeout 60 \"$0\" preprocess e.cpp",
        )
        .arg(env!("CARGO_BIN_EXE_dayzmake"))
        .current_dir(dir.path());
    isolate(&mut cmd, home.path());
    let out = cmd.output().unwrap();
    if out.status.code() == Some(99) {
        eprintln!("skipped: `ulimit -v` or `timeout` is not available");
        return None;
    }
    Some((
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// #57: the billion-laughs repro with N=30 (2^30 tokens uncapped) fails fast with an error naming the
/// macro.
#[cfg(unix)]
#[test]
fn exponential_macro_expansion_fails_fast() {
    let mut src = String::from("#define E30 x\n");
    for i in (0..30).rev() {
        let next = i + 1;
        src += &[
            "#define E",
            &i.to_string(),
            " E",
            &next.to_string(),
            " E",
            &next.to_string(),
            "\n",
        ]
        .concat();
    }
    src.push_str("a = E0;\n");
    let Some((code, err)) = preprocess_limited(&[("e.cpp", &src)]) else {
        return;
    };
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("expands to more than"), "{err}");
    assert!(err.contains("\"E"), "{err}");
}

/// Stringification collapses an expanded argument into one token, so the bytes double per level while
/// the token count stays small; the byte cap stops it (L5 needs about 20 GB uncapped).
#[cfg(unix)]
#[test]
fn exponential_stringification_fails_fast() {
    let mut src = String::from(
        "#define Q(x) #x\n#define D(x) x x\n#define S1(x) Q(D(x))\n#define L0(x) S1(x)\n",
    );
    for i in 1..=5 {
        let prev = i - 1;
        src += &[
            "#define L",
            &i.to_string(),
            "(x) L",
            &prev.to_string(),
            "(L",
            &prev.to_string(),
            "(x))\n",
        ]
        .concat();
    }
    src.push_str("a = L5(ab);\n");
    let Some((code, err)) = preprocess_limited(&[("e.cpp", &src)]) else {
        return;
    };
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("expands to more than"), "{err}");
}

/// #59: exponential expansion that produces no output (the macro drops its argument) is bounded too.
#[cfg(unix)]
#[test]
fn exponential_empty_expansion_fails_fast() {
    let mut src = String::from("#define E(x)\n#define A0\n");
    for i in 1..=30 {
        let prev = i - 1;
        src += &[
            "#define A",
            &i.to_string(),
            " E(A",
            &prev.to_string(),
            ")E(A",
            &prev.to_string(),
            ")\n",
        ]
        .concat();
    }
    src.push_str("A30\n");
    let Some((code, err)) = preprocess_limited(&[("e.cpp", &src)]) else {
        return;
    };
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("expands to more than"), "{err}");
}

/// #60: exponential `#include` fan-out (four includes per file, 30 levels) fails fast.
#[cfg(unix)]
#[test]
fn include_fan_out_fails_fast() {
    let mut files: Vec<(String, String)> = Vec::new();
    for i in 0..30 {
        let line = format!("#include \"f{}.h\"\n", i + 1);
        files.push((format!("f{i}.h"), line.repeat(4)));
    }
    files.push(("f30.h".to_string(), String::new()));
    files.push(("e.cpp".to_string(), "#include \"f0.h\"\n".to_string()));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_str()))
        .collect();
    let Some((code, err)) = preprocess_limited(&refs) else {
        return;
    };
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("the limit is"), "{err}");
}

/// #64: 3000 lines of `A13` (about 16,000 macro calls each, 37 s uncapped) fail fast on the per-file cap.
#[cfg(unix)]
#[test]
fn per_file_macro_work_fails_fast() {
    let mut src = String::from("#define E(x)\n#define A0\n");
    for i in 1..=13 {
        let prev = i - 1;
        src += &[
            "#define A",
            &i.to_string(),
            " E(A",
            &prev.to_string(),
            ")E(A",
            &prev.to_string(),
            ")\n",
        ]
        .concat();
    }
    src.push_str(&"A13\n".repeat(3000));
    let Some((code, err)) = preprocess_limited(&[("e.cpp", &src)]) else {
        return;
    };
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("macro calls (at macro"), "{err}");
}

/// The working directory is only checked directly for an absolute include; a file buried in it is not found
/// without `-i` (#66).
#[test]
fn working_directory_include_is_checked_directly_not_walked() {
    let dir = tempfile::tempdir().unwrap();
    let write = |relative: &str, text: &str| {
        let path = dir.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("e.cpp", "#include \"\\DZ\\data\\x.hpp\"\n");
    write("deep/er/DZ/data/x.hpp", "a = 1;\n");

    let (code, _, err) = dayzmake(&["preprocess", "e.cpp"], dir.path());
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("not found"), "{err}");

    // a folder given with -i is searched recursively
    let (code, out, err) = dayzmake(&["preprocess", "-i", "deep", "e.cpp"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "a = 1;");

    // the file directly below the working directory is found without -i
    write("DZ/data/x.hpp", "a = 2;\n");
    let (code, out, err) = dayzmake(&["preprocess", "e.cpp"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "a = 2;");
}

#[test]
fn derapify_indent_option() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "c.cpp", "class A { x = 1; };\n");
    let (code, _, err) = dayzmake(&["rapify", "c.cpp", "c.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = dayzmake(&["derapify", "-d", "  ", "c.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\n  x = 1;"), "{out}");
    assert!(!out.contains("\n   x = 1;"), "{out}");
}

#[test]
fn derapify_rejects_bad_indent() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "c.cpp", "class A { x = 1; };\n");
    let (code, _, err) = dayzmake(&["rapify", "c.cpp", "c.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = dayzmake(&["derapify", "-d", "x", "c.bin", "out.cpp"], dir.path());
    assert_ne!(code, 0);
    assert!(err.contains("indent"), "{err}");
    assert!(!dir.path().join("out.cpp").exists());
}

/// A PBO `out.pbo` holding `config.cpp` next to a pre-existing `t.txt`.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn cat_setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "m/config.cpp", "class B { y = 2; };\n");
    let (code, _, err) = dayzmake(&["build", "m", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    write(dir.path(), "t.txt", "old");
    dir
}

#[test]
fn cat_refuses_existing_target_without_force() {
    let dir = cat_setup();
    let (code, _, err) = dayzmake(&["cat", "out.pbo", "config.bin", "t.txt"], dir.path());
    assert_ne!(code, 0);
    assert!(err.contains("--force"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("t.txt")).unwrap(),
        "old"
    );
}

#[test]
fn cat_force_overwrites_target() {
    let dir = cat_setup();
    let (code, _, err) = dayzmake(&["cat", "-f", "out.pbo", "config.bin", "t.txt"], dir.path());
    assert_eq!(code, 0, "{err}");
    let written = std::fs::read(dir.path().join("t.txt")).unwrap();
    assert!(is_rapified_bytes(&written), "{written:?}");
}

#[test]
fn cat_force_to_stdout_is_accepted() {
    let dir = cat_setup();
    let (code, out, err) = dayzmake_raw(&["cat", "-f", "out.pbo", "config.bin"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(is_rapified_bytes(&out));
}

/// A rapified config starts with a NUL byte followed by `raP`.
fn is_rapified_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\0raP")
}

#[test]
fn extract_unpacks_an_install_by_prefix_with_a_map() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a/$PBOPREFIX$", "a");
    write(dir.path(), "a/x.txt", "x");
    write(
        dir.path(),
        "a/config.cpp",
        "class CfgPatches { class A {}; };",
    );
    write(dir.path(), "b/$PBOPREFIX$", "b\\sub");
    write(dir.path(), "b/y.txt", "y");
    std::fs::create_dir_all(dir.path().join("install/dta")).unwrap();
    std::fs::create_dir_all(dir.path().join("install/Addons")).unwrap();
    let (code, _, err) = dayzmake(&["pack", "a", "install/dta/a.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = dayzmake(&["build", "b", "install/Addons/b.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");

    let (code, out, err) = dayzmake(&["extract", "-v", "install", "out"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "extracted 3 files from 2 PBOs to out\n");
    assert_eq!(
        std::fs::read(dir.path().join("out/pbo-map.tsv")).unwrap(),
        b"a\tdta/a.pbo\nb\\sub\tAddons/b.pbo\n"
    );
    assert!(dir.path().join("out/a/x.txt").is_file());
    assert!(dir.path().join("out/b/sub/y.txt").is_file());

    // an existing target needs -f
    let (code, _, err) = dayzmake(&["extract", "install", "out"], dir.path());
    assert_eq!(code, 1);
    assert!(err.contains("--force"), "{err}");

    // --derapify, --only
    let (code, _, err) = dayzmake(
        &[
            "extract",
            "-f",
            "--derapify",
            "--only",
            "*.cpp",
            "--only",
            "*.bin",
            "install",
            "out",
        ],
        dir.path(),
    );
    assert_eq!(code, 0, "{err}");
    assert!(dir.path().join("out/a/config.cpp").is_file());
    assert!(!dir.path().join("out/a/x.txt").exists());
}
