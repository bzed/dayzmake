//! Tests that drive the built `dayzmake` binary.
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
    // the empty value it expands to is not reported again as an unquoted string
    assert!(!err.contains("unquoted-string"), "{}", err);
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

    // "." has no folder name to derive a prefix from
    write(dir.path(), "mod/$PBOPREFIX$", "mod\n");
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
