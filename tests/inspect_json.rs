//! `dayzmake inspect --json` and the MCP argument `json`: the PBO header as exact bytes.
#![expect(clippy::unwrap_used, reason = "a failing unwrap is a failing test")]
#![expect(clippy::expect_used, reason = "a failing expect is a failing test")]
#![expect(clippy::indexing_slicing, reason = "a failing index is a failing test")]
#[path = "../crates/dayzmake-core/tests/support/mod.rs"]
#[expect(
    dead_code,
    reason = "shared by several test crates; this one needs one helper"
)]
mod support;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn isolate(cmd: &mut Command, home: &Path) {
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DATA_HOME", home.join("data"));
}

fn dayzmake(args: &[&str], cwd: &Path) -> (i32, String, String) {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.args(args).current_dir(cwd);
    isolate(&mut cmd, home.path());
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A PBO whose prefix is `scripts` + LF + 0xFF, with a second extension and two entries.
fn tricky_pbo() -> Vec<u8> {
    support::raw_pbo_bytes(
        &[
            (b"config.bin", b"abc", 0),
            (b"a\xffb\n.txt", b"hello", 1_700_000_000),
        ],
        &[(b"prefix", b"scripts\n\xff"), (b"product", b"x\"y")],
    )
}

fn write_pbo(dir: &Path) -> PathBuf {
    let path = dir.join("t.pbo");
    std::fs::write(&path, tricky_pbo()).unwrap();
    path
}

#[test]
fn json_has_the_exact_hex_and_a_stable_field_order() {
    let dir = tempfile::tempdir().unwrap();
    write_pbo(dir.path());
    let (code, out, err) = dayzmake(&["inspect", "--json", "t.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let v: Value = serde_json::from_str(&out).expect("one JSON object");
    assert_eq!(v["prefix"], "scripts\n\u{fffd}");
    assert_eq!(v["prefix_hex"], "736372697074730aff");
    let ext = v["header_extensions"].as_array().unwrap();
    assert_eq!(ext.len(), 2);
    assert_eq!(
        ext[0],
        json!({"key":"prefix","value":"scripts\n\u{fffd}","key_hex":"707265666978","value_hex":"736372697074730aff"})
    );
    assert_eq!(ext[1]["key"], "product");
    assert_eq!(ext[1]["value"], "x\"y");
    assert_eq!(ext[1]["value_hex"], "782279");
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0],
        json!({"name":"config.bin","name_hex":"636f6e6669672e62696e","packing":0,"original_size":3,"data_size":3,"timestamp":0})
    );
    assert_eq!(entries[1]["name"], "a\u{fffd}b\n.txt");
    assert_eq!(entries[1]["name_hex"], "61ff620a2e747874");
    assert_eq!(entries[1]["original_size"], 5);
    assert_eq!(entries[1]["timestamp"], 1_700_000_000);
    // the order of the fields is part of the contract
    let order = |keys: &[&str], text: &str| {
        let mut last = 0;
        for k in keys {
            let at = text[last..].find(&format!("\"{k}\":")).expect(k) + last;
            last = at + 1;
        }
    };
    order(
        &["header_extensions", "prefix", "prefix_hex", "entries"],
        &out,
    );
    order(&["key", "value", "key_hex", "value_hex"], &out);
    order(
        &[
            "name",
            "name_hex",
            "packing",
            "original_size",
            "data_size",
            "timestamp",
        ],
        &out,
    );
    // JSON escaping only: the LF is `\n`, no other escaping of the text
    assert!(out.contains("\"prefix\":\"scripts\\n\u{fffd}\""), "{out}");
}

#[test]
fn json_without_a_prefix_is_null() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("t.pbo"),
        support::raw_pbo_bytes(&[(b"a.txt", b"x", 0)], &[]),
    )
    .unwrap();
    let (code, out, err) = dayzmake(&["inspect", "--json", "t.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["header_extensions"], json!([]));
    assert_eq!(v["prefix"], Value::Null);
    assert_eq!(v["prefix_hex"], Value::Null);
}

#[test]
fn text_output_is_unchanged_and_escaped() {
    let dir = tempfile::tempdir().unwrap();
    write_pbo(dir.path());
    let (code, out, err) = dayzmake(&["inspect", "t.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("Header extensions:\n- prefix="), "{out}");
    assert!(!out.contains('{'), "{out}");
}

#[test]
fn json_for_a_model_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mlod/small.p3d");
    let (code, out, err) = dayzmake(&["inspect", "--json", model.to_str().unwrap()], dir.path());
    assert_ne!(code, 0);
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("--json is only supported for PBOs"), "{err}");
}

/// Calls one MCP tool and returns (`is_error`, text).
fn mcp_call(name: &str, args: &Value, cwd: &Path) -> (bool, String) {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.arg("mcp")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate(&mut cmd, home.path());
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}});
    let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}});
    writeln!(stdin, "{init}\n{req}").unwrap();
    drop(stdin);
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    child.wait().unwrap();
    let reply: Value = serde_json::from_str(out.lines().nth(1).expect("reply")).unwrap();
    (
        reply["result"]["isError"].as_bool().unwrap(),
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string(),
    )
}

#[test]
fn mcp_pbo_inspect_json_returns_the_same_object() {
    let dir = tempfile::tempdir().unwrap();
    write_pbo(dir.path());
    let (_, cli, _) = dayzmake(&["inspect", "--json", "t.pbo"], dir.path());
    let (is_error, text) = mcp_call(
        "pbo_inspect",
        &json!({"path":"t.pbo","json":true}),
        dir.path(),
    );
    assert!(!is_error, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        serde_json::from_str::<Value>(&cli).unwrap()
    );
    let (is_error, text) = mcp_call("pbo_inspect", &json!({"path":"t.pbo"}), dir.path());
    assert!(
        !is_error && text.starts_with("Header extensions:"),
        "{text}"
    );
    let (is_error, text) = mcp_call(
        "pbo_inspect",
        &json!({"path":"t.pbo","json":"yes"}),
        dir.path(),
    );
    assert!(is_error, "{text}");
}
