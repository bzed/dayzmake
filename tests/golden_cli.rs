//! Characterization snapshots: how every CLI command behaves today. A snapshot changes only on purpose.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "test code")]
#![expect(clippy::expect_used, reason = "test code")]

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::TempDir;

/// Result of one CLI run.
struct Run {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
    /// Paths to replace by `[TMP]` in text output.
    tmp: Vec<String>,
}

impl Run {
    fn norm(&self, s: &str) -> String {
        let mut s = s.replace("\r\n", "\n");
        for t in &self.tmp {
            s = s.replace(t.as_str(), "[TMP]");
        }
        s
    }
    fn out(&self) -> String {
        self.norm(&String::from_utf8_lossy(&self.stdout))
    }
    fn err(&self) -> String {
        self.norm(&self.stderr)
    }
}

fn dayzmake(args: &[&str], cwd: &Path, stdin: Option<&[u8]>) -> Run {
    dayzmake_env(args, cwd, stdin, &[])
}

fn dayzmake_env(args: &[&str], cwd: &Path, stdin: Option<&[u8]>, env: &[(&str, &Path)]) -> Run {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(args)
        .current_dir(cwd)
        .env("HOME", h)
        .env("XDG_CONFIG_HOME", h.join("config"))
        .env("XDG_CACHE_HOME", h.join("cache"))
        .env("XDG_DATA_HOME", h.join("data"))
        .env("NO_COLOR", "1")
        .env_remove("STEAM_ROOT")
        .env_remove("DAYZ_TOOLS")
        .env_remove("PROTON")
        .env_remove("BIOUTPUT")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, _) in std::env::vars() {
        if k.starts_with("DAYZMAKE_") {
            cmd.env_remove(k);
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("failed to run dayzmake");
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let mut tmp = vec![h.to_string_lossy().into_owned()];
    if let Some(parent) = cwd.parent() {
        tmp.push(parent.to_string_lossy().into_owned());
    }
    tmp.push(cwd.to_string_lossy().into_owned());
    // Longest first, so the most specific path wins.
    tmp.sort_by_key(|p| std::cmp::Reverse(p.len()));
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        tmp,
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A temp root holding the mod copy (`<root>/mod`, the cwd of the tests) and the fixture key.
struct Env {
    root: TempDir,
    work: PathBuf,
}

fn setup() -> Env {
    let root = tempfile::tempdir().unwrap();
    // Canonical, so the `[TMP]` replacement matches what the binary prints.
    let base = root.path().canonicalize().unwrap();
    let work = base.join("mod");
    copy_tree(&manifest().join("tests/fixtures/golden/mod"), &work);
    fs::copy(
        manifest().join("tests/fixtures/mlod/small.p3d"),
        work.join("data/model.p3d"),
    )
    .unwrap();
    for f in ["armk.biprivatekey", "armk.bikey"] {
        fs::copy(
            manifest().join("tests/fixtures/bi/sign").join(f),
            base.join(f),
        )
        .unwrap();
    }
    Env { root, work }
}

impl Env {
    fn base(&self) -> PathBuf {
        self.root.path().canonicalize().unwrap()
    }
    fn run(&self, args: &[&str]) -> Run {
        dayzmake(args, &self.work, None)
    }
    fn read(&self, rel: &str) -> Vec<u8> {
        fs::read(self.work.join(rel)).unwrap()
    }
    /// Builds `../out.pbo` and asserts success.
    fn build(&self) {
        let r = self.run(&["build", ".", "../out.pbo"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
    }
    fn rapify(&self) {
        let r = self.run(&["rapify", "config.cpp", "config.bin"]);
        assert_eq!(r.code, 0, "{}", r.stderr);
    }
}

/// Writes the gradient PNG (`alpha` false: opaque) to `<dir>/grad.png`.
fn write_png(dir: &Path, alpha: bool) {
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        let a = if alpha { 255 - x * 2 } else { 255 };
        image::Rgba([
            u8::try_from(x * 4).unwrap(),
            u8::try_from(y * 4).unwrap(),
            128,
            u8::try_from(a).unwrap(),
        ])
    });
    img.save(dir.join("grad.png")).unwrap();
}

/// `path<TAB>len<TAB>first 16 bytes hex` of every file under `dir`, sorted.
fn listing(dir: &Path) -> String {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                let data = fs::read(&p).unwrap();
                let mut hex = String::new();
                for b in data.iter().take(16) {
                    write!(hex, "{b:02x}").unwrap();
                }
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                out.push(format!("{rel}\t{}\t{hex}", data.len()));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v.join("\n") + "\n"
}

/// Warning order follows the file system's directory order, so the quota tests hide the file names.
fn hide_quota_names(stderr: &str) -> String {
    stderr
        .lines()
        .map(|l| match l.strip_prefix("In file c") {
            Some(rest) => format!("In file cNN{}\n", rest.get(2..).unwrap_or("")),
            None => format!("{l}\n"),
        })
        .collect()
}

fn snap_run(name: &str, r: &Run) {
    insta::assert_snapshot!(format!("{name}_stderr"), r.err());
}

#[test]
fn help_text() {
    let e = setup();
    let r = e.run(&["--help"]);
    assert_eq!(r.code, 0);
    insta::assert_snapshot!(r.out());
}

#[test]
fn version_text() {
    let e = setup();
    let r = e.run(&["--version"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.out(), format!("v{}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn preprocess_file() {
    let e = setup();
    let r = e.run(&["preprocess", "config.cpp", "out.cpp"]);
    assert_eq!(r.code, 0);
    insta::assert_snapshot!(
        "preprocess_file_out",
        String::from_utf8_lossy(&e.read("out.cpp"))
    );
    snap_run("preprocess_file", &r);
}

#[test]
fn preprocess_stdin_stdout() {
    let e = setup();
    let input = e.read("config.cpp");
    let r = dayzmake(&["preprocess", "-i", "."], &e.work, Some(&input));
    assert_eq!(r.code, 0);
    insta::assert_snapshot!("preprocess_stdin_stdout_out", r.out());
    snap_run("preprocess_stdin_stdout", &r);
}

#[test]
fn rapify_file() {
    let e = setup();
    let r = e.run(&["rapify", "config.cpp", "config.bin"]);
    assert_eq!(r.code, 0);
    insta::assert_binary_snapshot!("rapify_file.bin", e.read("config.bin"));
    snap_run("rapify_file", &r);
}

#[test]
fn rapify_stdin_stdout() {
    let e = setup();
    let input = e.read("config.cpp");
    let r = dayzmake(&["rapify", "-i", "."], &e.work, Some(&input));
    assert_eq!(r.code, 0);
    insta::assert_binary_snapshot!("rapify_stdin_stdout.bin", r.stdout.clone());
}

#[test]
fn derapify_default() {
    let e = setup();
    e.rapify();
    let r = e.run(&["derapify", "config.bin", "out.cpp"]);
    assert_eq!(r.code, 0);
    insta::assert_snapshot!(String::from_utf8_lossy(&e.read("out.cpp")));
    snap_run("derapify_default", &r);
}

#[test]
fn derapify_indent() {
    let e = setup();
    e.rapify();
    let r = e.run(&["derapify", "-d", "\t", "config.bin", "out.cpp"]);
    assert_eq!(r.code, 0);
    insta::assert_snapshot!(String::from_utf8_lossy(&e.read("out.cpp")));
}

#[test]
fn derapify_stdin_stdout() {
    let e = setup();
    e.rapify();
    let input = e.read("config.bin");
    let r = dayzmake(&["derapify"], &e.work, Some(&input));
    assert_eq!(r.code, 0);
    insta::assert_snapshot!(r.out());
}

#[test]
fn pack_bytes() {
    let e = setup();
    let r = e.run(&["pack", ".", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(".pbo", fs::read(e.base().join("out.pbo")).unwrap());
}

#[test]
fn pack_headerext() {
    let e = setup();
    let r = e.run(&["pack", "-e", "product=golden", ".", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(".pbo", fs::read(e.base().join("out.pbo")).unwrap());
}

#[test]
fn pack_exclude() {
    let e = setup();
    let r = e.run(&["pack", "-x", "*.txt", ".", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(".pbo", fs::read(e.base().join("out.pbo")).unwrap());
}

#[test]
fn build_bytes_and_warnings() {
    let e = setup();
    let r = e.run(&["build", ".", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(".pbo", fs::read(e.base().join("out.pbo")).unwrap());
    snap_run("build_bytes_and_warnings", &r);
}

#[test]
fn build_signed() {
    let e = setup();
    let r = e.run(&["build", "-k", "../armk.biprivatekey", ".", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(
        "build_signed_pbo.pbo",
        fs::read(e.base().join("out.pbo")).unwrap()
    );
    insta::assert_binary_snapshot!(
        "build_signed_sig.bisign",
        fs::read(e.base().join("out.pbo.armk.bisign")).unwrap()
    );
}

#[test]
fn build_signed_custom_signature_path() {
    let e = setup();
    let r = e.run(&["build", "-k", "../armk.biprivatekey", ".", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let default_sig = fs::read(e.base().join("out.pbo.armk.bisign")).unwrap();
    let r = e.run(&[
        "build",
        "-k",
        "../armk.biprivatekey",
        "-s",
        "../sig.bisign",
        "-f",
        ".",
        "../out2.pbo",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(fs::read(e.base().join("sig.bisign")).unwrap(), default_sig);
}

#[test]
fn inspect_pbo_file() {
    let e = setup();
    e.build();
    let r = e.run(&["inspect", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!(r.out());
}

#[test]
fn inspect_pbo_stdin() {
    let e = setup();
    e.build();
    let input = fs::read(e.base().join("out.pbo")).unwrap();
    let r = dayzmake(&["inspect"], &e.work, Some(&input));
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!(r.out());
}

#[test]
fn inspect_p3d() {
    let e = setup();
    let r = e.run(&["inspect", "data/model.p3d"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!(r.out());
}

#[test]
fn unpack_tree() {
    let e = setup();
    e.build();
    let r = e.run(&["unpack", "../out.pbo", "../x"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!(listing(&e.base().join("x")));
}

#[test]
fn unpack_use_prefix() {
    let e = setup();
    e.build();
    let r = e.run(&["unpack", "--use-prefix", "../out.pbo", "../x"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!(listing(&e.base().join("x")));
}

#[test]
fn unpack_existing_target() {
    let e = setup();
    e.build();
    assert_eq!(e.run(&["unpack", "../out.pbo", "../x"]).code, 0);
    let r = e.run(&["unpack", "../out.pbo", "../x"]);
    assert_eq!(r.code, 1);
    snap_run("unpack_existing_target", &r);
    assert_eq!(e.run(&["unpack", "-f", "../out.pbo", "../x"]).code, 0);
}

#[test]
fn cat_stdout() {
    let e = setup();
    e.build();
    let r = e.run(&["cat", "../out.pbo", "data\\readme.txt"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!(r.out());
}

#[test]
fn cat_to_file() {
    let e = setup();
    e.build();
    let r = e.run(&["cat", "../out.pbo", "data\\readme.txt", "t"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!(String::from_utf8_lossy(&e.read("t")));
}

#[test]
fn cat_missing_entry() {
    let e = setup();
    e.build();
    let r = e.run(&["cat", "../out.pbo", "data\\nope.txt"]);
    assert_eq!(r.code, 1);
    snap_run("cat_missing_entry", &r);
}

#[test]
fn keygen_sign_verify() {
    let e = setup();
    e.build();
    let r = e.run(&["keygen", "k"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(e.work.join("k.biprivatekey").exists());
    assert!(e.work.join("k.bikey").exists());
    insta::assert_snapshot!("keygen_bikey_len", e.read("k.bikey").len().to_string());
    let r = e.run(&["sign", "k.biprivatekey", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let r = e.run(&["verify", "k.bikey", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.err(), "");
}

#[test]
fn sign_v3_bytes() {
    let e = setup();
    e.build();
    let r = e.run(&["sign", "../armk.biprivatekey", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(
        ".bisign",
        fs::read(e.base().join("out.pbo.armk.bisign")).unwrap()
    );
}

#[test]
fn sign_v2_bytes() {
    let e = setup();
    e.build();
    let r = e.run(&["sign", "--v2", "../armk.biprivatekey", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(
        ".bisign",
        fs::read(e.base().join("out.pbo.armk.bisign")).unwrap()
    );
}

#[test]
fn verify_ok() {
    let e = setup();
    e.build();
    assert_eq!(
        e.run(&["sign", "../armk.biprivatekey", "../out.pbo"]).code,
        0
    );
    let r = e.run(&["verify", "../armk.bikey", "../out.pbo"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
}

#[test]
fn verify_tampered() {
    let e = setup();
    e.build();
    assert_eq!(
        e.run(&["sign", "../armk.biprivatekey", "../out.pbo"]).code,
        0
    );
    let path = e.base().join("out.pbo");
    let mut data = fs::read(&path).unwrap();
    // Version 3 signatures only hash some file types (.hpp yes, .txt and .bin no).
    let needle = b"#define SCALE";
    let at = data
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap();
    data[at + 1] ^= 0x01; // flips one bit of the hashed .hpp
    fs::write(&path, data).unwrap();
    let r = e.run(&["verify", "../armk.bikey", "../out.pbo"]);
    assert_eq!(r.code, 1);
    snap_run("verify_tampered", &r);
}

fn img2paa(e: &Env, ty: Option<&str>, alpha: bool) -> (Run, Vec<u8>) {
    write_png(&e.work, alpha);
    let mut args = vec!["img2paa"];
    if let Some(t) = ty {
        args.extend(["-t", t]);
    }
    args.extend(["grad.png", "out.paa"]);
    let r = e.run(&args);
    let bytes = fs::read(e.work.join("out.paa")).unwrap_or_default();
    (r, bytes)
}

#[test]
fn img2paa_dxt1() {
    let e = setup();
    let (r, b) = img2paa(&e, Some("dxt1"), false);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(".paa", b);
}

#[test]
fn img2paa_dxt5() {
    let e = setup();
    let (r, b) = img2paa(&e, Some("dxt5"), true);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(".paa", b);
}

#[test]
fn img2paa_rgba() {
    let e = setup();
    let (r, b) = img2paa(&e, Some("rgba"), true);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_binary_snapshot!(".paa", b);
}

#[test]
fn img2paa_default_type() {
    let e = setup();
    let (r, def) = img2paa(&e, None, true);
    assert_eq!(r.code, 0, "{}", r.stderr);
    fs::remove_file(e.work.join("out.paa")).unwrap();
    let (r, dxt5) = img2paa(&e, Some("dxt5"), true);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(def, dxt5);
}

#[test]
fn img2paa_bad_type() {
    let e = setup();
    let (r, _) = img2paa(&e, Some("bogus"), true);
    assert_eq!(r.code, 1);
    snap_run("img2paa_bad_type", &r);
}

#[test]
fn paa2img_pixels() {
    let e = setup();
    let (r, _) = img2paa(&e, Some("rgba"), true);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let r = e.run(&["paa2img", "out.paa", "back.png"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let img = image::open(e.work.join("back.png")).unwrap().to_rgba8();
    insta::assert_snapshot!("paa2img_size", format!("{}x{}", img.width(), img.height()));
    insta::assert_binary_snapshot!("paa2img_pixels.rgba", img.into_raw());
}

#[test]
fn binarize_without_proton() {
    let e = setup();
    let steam = tempfile::tempdir().unwrap();
    let r = dayzmake_env(
        &["binarize", "data/model.p3d", "out.p3d"],
        &e.work,
        None,
        &[("STEAM_ROOT", steam.path())],
    );
    assert_eq!(r.code, 1);
    let r = Run {
        tmp: {
            let mut t = r.tmp.clone();
            t.push(steam.path().to_string_lossy().into_owned());
            t
        },
        ..r
    };
    snap_run("binarize_without_proton", &r);
}

#[test]
fn existing_targets_need_force() {
    let e = setup();
    e.rapify();
    write_png(&e.work, true);
    assert_eq!(e.run(&["img2paa", "grad.png", "g.paa"]).code, 0);
    e.build();
    let cases: [(&str, Vec<&str>); 9] = [
        ("rapify", vec!["rapify", "config.cpp", "t.bin"]),
        ("derapify", vec!["derapify", "config.bin", "t.cpp"]),
        ("preprocess", vec!["preprocess", "config.cpp", "t.txt"]),
        ("build", vec!["build", ".", "../b.pbo"]),
        ("pack", vec!["pack", ".", "../p.pbo"]),
        ("keygen", vec!["keygen", "kk"]),
        (
            "sign",
            vec!["sign", "../armk.biprivatekey", "../out.pbo", "../s.bisign"],
        ),
        ("img2paa", vec!["img2paa", "grad.png", "t.paa"]),
        ("paa2img", vec!["paa2img", "g.paa", "t.png"]),
    ];
    for (name, args) in cases {
        let first = e.run(&args);
        assert_eq!(first.code, 0, "{name}: {}", first.stderr);
        let second = e.run(&args);
        assert_eq!(second.code, 1, "{name}");
        insta::assert_snapshot!(format!("exists_{name}"), second.err());
    }
}

#[test]
fn missing_inputs() {
    let e = setup();
    for (name, args) in [
        ("rapify", vec!["rapify", "nope.cpp", "x"]),
        ("inspect", vec!["inspect", "nope.pbo"]),
        ("unpack", vec!["unpack", "nope.pbo", "d"]),
    ] {
        let r = e.run(&args);
        assert_eq!(r.code, 1, "{name}");
        insta::assert_snapshot!(format!("missing_{name}"), r.err());
    }
}

/// A directory with 12 pre-rapified `cNN.cpp` files (one warning each).
fn quota_dir() -> (TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().canonicalize().unwrap().join("q");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("$PBOPREFIX$"), "golden\\quota\n").unwrap();
    for i in 0..12 {
        fs::write(work.join(format!("c{i:02}.cpp")), b"\0raP\0\0\0\0").unwrap();
    }
    (root, work)
}

#[test]
fn warning_quota() {
    let (_root, work) = quota_dir();
    let r = dayzmake(&["build", ".", "../out.pbo"], &work, None);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!("warning_quota_stderr", hide_quota_names(&r.err()));
}

#[test]
fn warning_mute() {
    let (_root, work) = quota_dir();
    let r = dayzmake(
        &["build", "-w", "prerapified-file", ".", "../out.pbo"],
        &work,
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(!r.stderr.contains("prerapified-file"), "{}", r.stderr);
    snap_run("warning_mute", &r);
}

#[test]
fn warning_verbose() {
    let (_root, work) = quota_dir();
    let r = dayzmake(&["build", "-v", ".", "../out.pbo"], &work, None);
    assert_eq!(r.code, 0, "{}", r.stderr);
    insta::assert_snapshot!("warning_verbose_stderr", hide_quota_names(&r.err()));
}

#[test]
fn parse_error_output() {
    // The brief's `class A { x = ; };` is accepted today (empty string), see `empty_value_is_accepted`.
    let e = setup();
    fs::write(e.work.join("bad.cpp"), "class A { = 1; };\n").unwrap();
    let r = e.run(&["rapify", "bad.cpp", "bad.bin"]);
    assert_eq!(r.code, 1);
    snap_run("parse_error_output", &r);
}

#[test]
fn empty_value_is_accepted() {
    let e = setup();
    fs::write(e.work.join("bad.cpp"), "class A { x = ; };\n").unwrap();
    let r = e.run(&["rapify", "bad.cpp", "bad.bin"]);
    assert_eq!(r.code, 0);
    snap_run("empty_value_is_accepted", &r);
    let r = e.run(&["derapify", "bad.bin"]);
    insta::assert_snapshot!("empty_value_is_accepted_derapified", r.out());
}
