//! Characterization snapshots: what `dayzmake mcp` replies today. A snapshot changes only on purpose.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "test code")]
#![expect(clippy::expect_used, reason = "test code")]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

/// The replies of one MCP session.
struct Run {
    replies: Vec<Value>,
    /// Paths to replace by `[TMP]`.
    tmp: Vec<String>,
}

impl Run {
    /// The `result` of the reply with this id, temp paths replaced by `[TMP]`.
    fn result(&self, id: u64) -> Value {
        let reply = self
            .replies
            .iter()
            .find(|r| r["id"] == id)
            .expect("a reply with that id");
        self.norm(&reply["result"])
    }
    fn norm(&self, v: &Value) -> Value {
        let mut s = v.to_string();
        for t in &self.tmp {
            s = s.replace(t.as_str(), "[TMP]");
        }
        serde_json::from_str(&s).unwrap()
    }
}

/// Runs `dayzmake mcp` in `cwd`, feeds it the requests, closes stdin and collects the replies.
fn session(reqs: &[Value], cwd: &Path) -> Run {
    let home = tempfile::tempdir().unwrap();
    let h = home.path().canonicalize().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.arg("mcp")
        .current_dir(cwd)
        .env("HOME", &h)
        .env("XDG_CONFIG_HOME", h.join("config"))
        .env("XDG_CACHE_HOME", h.join("cache"))
        .env("XDG_DATA_HOME", h.join("data"))
        .env("NO_COLOR", "1")
        .env_remove("STEAM_ROOT")
        .env_remove("DAYZ_TOOLS")
        .env_remove("PROTON")
        .env_remove("BIOUTPUT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, _) in std::env::vars() {
        if k.starts_with("DAYZMAKE_") {
            cmd.env_remove(k);
        }
    }
    let mut child = cmd.spawn().unwrap();
    let mut input = Vec::new();
    for r in reqs {
        input.extend_from_slice(r.to_string().as_bytes());
        input.push(b'\n');
    }
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        // the server may exit early; a broken pipe is fine
        let _ = stdin.write_all(&input);
    });
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    writer.join().unwrap();
    assert!(child.wait().unwrap().success(), "{stderr}");
    let replies = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    let mut tmp = vec![h.to_string_lossy().into_owned()];
    if let Some(parent) = cwd.parent() {
        tmp.push(parent.to_string_lossy().into_owned());
    }
    tmp.push(cwd.to_string_lossy().into_owned());
    // Longest first, so the most specific path wins.
    tmp.sort_by_key(|p| std::cmp::Reverse(p.len()));
    Run { replies, tmp }
}

fn init() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}})
}

fn call(id: u64, name: &str, args: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
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

/// A temp root holding the mod copy (`<root>/mod`, the cwd of the session) and the fixture key.
fn setup() -> (TempDir, PathBuf) {
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
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([
            u8::try_from(x * 4).unwrap(),
            u8::try_from(y * 4).unwrap(),
            128,
            255,
        ])
    });
    img.save(work.join("grad.png")).unwrap();
    (root, work)
}

/// Warning order follows the file system's directory order, so the quota tests hide the file names.
fn hide_quota_names(v: &Value) -> Value {
    let s = v
        .to_string()
        .split("In file c")
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_string()
            } else {
                format!("In file cNN{}", part.get(2..).unwrap_or(""))
            }
        })
        .collect::<String>();
    serde_json::from_str(&s).unwrap()
}

#[test]
fn tools_list_schema() {
    let (_root, work) = setup();
    let r = session(
        &[
            init(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ],
        &work,
    );
    insta::assert_json_snapshot!(r.result(2)["tools"]);
}

#[test]
fn file_tools_replies() {
    let (_root, work) = setup();
    let key = "../armk.biprivatekey";
    let calls: Vec<(&str, Value)> = vec![
        ("pbo_build", json!({"source":".","target":"../build.pbo"})),
        ("pbo_pack", json!({"source":".","target":"../pack.pbo"})),
        ("pbo_inspect", json!({"path":"../build.pbo"})),
        (
            "pbo_cat",
            json!({"pbo":"../build.pbo","file":"data\\readme.txt"}),
        ),
        ("pbo_unpack", json!({"pbo":"../build.pbo","target":"../x"})),
        ("pbo_sign", json!({"private_key":key,"pbo":"../build.pbo"})),
        (
            "pbo_verify",
            json!({"public_key":"../armk.bikey","pbo":"../build.pbo"}),
        ),
        ("config_preprocess", json!({"source":"config.cpp"})),
        (
            "config_rapify",
            json!({"source":"config.cpp","target":"config.bin"}),
        ),
        ("config_derapify", json!({"source":"config.bin"})),
        (
            "image_to_paa",
            json!({"source":"grad.png","target":"grad.paa","type":"dxt1"}),
        ),
        (
            "paa_to_image",
            json!({"source":"grad.paa","target":"back.png"}),
        ),
        ("key_generate", json!({"name":"k"})),
        (
            "binarize",
            json!({"source":"data/model.p3d","target":"out.p3d"}),
        ),
    ];
    let mut reqs = vec![init()];
    for (i, (name, args)) in calls.iter().enumerate() {
        reqs.push(call(10 + i as u64, name, args));
    }
    let r = session(&reqs, &work);
    for (i, (name, _)) in calls.iter().enumerate() {
        insta::assert_json_snapshot!(*name, r.result(10 + i as u64));
    }
}

#[test]
fn warnings_reset_per_call() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().canonicalize().unwrap().join("q");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("$PBOPREFIX$"), "golden\\quota\n").unwrap();
    for i in 0..12 {
        fs::write(work.join(format!("c{i:02}.cpp")), b"\0raP\0\0\0\0").unwrap();
    }
    let r = session(
        &[
            init(),
            call(2, "pbo_build", &json!({"source":".","target":"../a.pbo"})),
            call(3, "pbo_build", &json!({"source":".","target":"../b.pbo"})),
        ],
        &work,
    );
    let (a, b) = (
        hide_quota_names(&r.result(2)),
        hide_quota_names(&r.result(3)),
    );
    assert_eq!(
        a["content"][0]["text"],
        b["content"][0]["text"]
            .as_str()
            .unwrap()
            .replace("b.pbo", "a.pbo")
    );
    insta::assert_json_snapshot!("warnings_first_call", a);
}

#[test]
fn error_reply_for_missing_file() {
    let (_root, work) = setup();
    let r = session(
        &[init(), call(2, "pbo_inspect", &json!({"path":"nope.pbo"}))],
        &work,
    );
    insta::assert_json_snapshot!(r.result(2));
}

#[test]
fn pbo_pack_missing_source() {
    let (_root, work) = setup();
    let r = session(
        &[
            init(),
            call(2, "pbo_pack", &json!({"source":"nope","target":"../p.pbo"})),
        ],
        &work,
    );
    insta::assert_json_snapshot!(r.result(2));
}
