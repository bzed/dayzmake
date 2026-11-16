//! The stdio MCP server: spawns `dayzmake mcp` and talks JSON-RPC over its pipes.
#![expect(clippy::expect_used, reason = "a failing expect is a failing test")]
#![expect(clippy::indexing_slicing, reason = "a failing index is a failing test")]
#[path = "../crates/dayzmake-core/tests/support/mod.rs"]
mod support;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use dayzmake_core::pbo::{PBO, PackOptions};
use dayzmake_core::warning::Warnings;
use serde_json::{Value, json};

struct Run {
    stdout: String,
    stderr: String,
    replies: Vec<Value>,
}

/// Runs the server, feeds it `input`, closes stdin and collects everything it printed.
fn run_raw(input: Vec<u8>, env: &[(&str, &Path)], cwd: &Path) -> Run {
    run_with(&[], input, env, cwd)
}

/// Like `run_raw`, with extra arguments after `mcp`.
fn run_with(extra: &[&str], input: Vec<u8>, env: &[(&str, &Path)], cwd: &Path) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayzmake"));
    cmd.arg("mcp")
        .args(extra)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn");
    let mut stdin = child.stdin.take().expect("stdin");
    let writer = std::thread::spawn(move || {
        // the server may exit early; a broken pipe is fine
        let _ = stdin.write_all(&input);
    });
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut stdout)
        .expect("read");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read");
    writer.join().expect("writer");
    assert!(child.wait().expect("wait").success(), "{stderr}");
    let replies = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    Run {
        stdout,
        stderr,
        replies,
    }
}

fn lines(reqs: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in reqs {
        out.extend_from_slice(r.to_string().as_bytes());
        out.push(b'\n');
    }
    out
}

fn run(reqs: &[Value], cwd: &Path) -> Run {
    run_raw(lines(reqs), &[], cwd)
}

fn call(id: u32, name: &str, args: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
}

fn text(reply: &Value) -> &str {
    reply["result"]["content"][0]["text"]
        .as_str()
        .expect("text content")
}

fn is_error(reply: &Value) -> bool {
    reply["result"]["isError"].as_bool().expect("isError")
}

/// A PBO with the given entries (name, bytes), built through the library.
fn make_pbo(entries: &[(&str, &[u8])]) -> Vec<u8> {
    support::raw_pbo(entries, &[])
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

#[test]
fn mcp_initialize_and_list_tools() {
    let tmp = tempfile::tempdir().expect("tmp");
    let r = run(
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
        ],
        tmp.path(),
    );
    assert_eq!(r.replies[0]["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(r.replies[0]["result"]["serverInfo"]["name"], "dayzmake");
    assert_eq!(r.replies[0]["result"]["capabilities"], json!({"tools":{}}));
    assert_eq!(r.replies[1]["result"]["protocolVersion"], "2025-06-18");
    let tools = r.replies[2]["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    for want in [
        "api_class",
        "api_find",
        "api_subclasses",
        "api_enum",
        "api_chain",
        "api_source",
        "api_defines",
        "api_update",
        "api_mods",
        "pbo_inspect",
        "pbo_cat",
        "pbo_unpack",
        "pbo_pack",
        "pbo_build",
        "config_rapify",
        "config_derapify",
        "config_preprocess",
        "paa_to_image",
        "image_to_paa",
        "key_generate",
        "pbo_sign",
        "pbo_verify",
        "binarize",
    ] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    assert_eq!(names.len(), 24);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(t["description"].is_string());
    }
    let class = tools
        .iter()
        .find(|t| t["name"] == "api_class")
        .expect("api_class");
    assert_eq!(class["inputSchema"]["required"], json!(["name"]));
}

#[test]
fn mcp_stdout_is_pure_jsonrpc() {
    let tmp = tempfile::tempdir().expect("tmp");
    fs::write(tmp.path().join("a.pbo"), make_pbo(&[("x.txt", b"hi")])).expect("write");
    fs::write(
        tmp.path().join("bad.pbo"),
        make_pbo(&[("..\\evil.txt", b"x"), ("ok.txt", b"y")]),
    )
    .expect("write");
    let r = run(
        &[
            call(1, "pbo_inspect", &json!({"path":"a.pbo"})),
            call(2, "pbo_unpack", &json!({"pbo":"bad.pbo","target":"out"})),
        ],
        tmp.path(),
    );
    assert_eq!(r.stdout.lines().count(), 2, "{}", r.stdout);
    for reply in &r.replies {
        assert_eq!(reply["jsonrpc"], "2.0");
    }
    assert!(!is_error(&r.replies[0]));
    assert!(text(&r.replies[0]).contains("x.txt"));
    assert!(!is_error(&r.replies[1]), "{}", text(&r.replies[1]));
    assert!(
        text(&r.replies[1]).contains("warnings:"),
        "{}",
        text(&r.replies[1])
    );
    assert!(
        text(&r.replies[1]).contains("unsafe-path"),
        "{}",
        text(&r.replies[1])
    );
    assert!(
        !r.stderr.contains("unsafe-path"),
        "warning leaked: {}",
        r.stderr
    );
}

#[test]
fn mcp_unknown_method_is_32601() {
    let tmp = tempfile::tempdir().expect("tmp");
    let r = run(
        &[json!({"jsonrpc":"2.0","id":"a","method":"nope/nothing"})],
        tmp.path(),
    );
    assert_eq!(r.replies[0]["error"]["code"], -32601);
    assert_eq!(r.replies[0]["id"], "a");
}

#[test]
fn mcp_parse_error_is_32700() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut input = b"{not json\n".to_vec();
    input.extend(lines(&[json!({"jsonrpc":"2.0","id":5,"method":"ping"})]));
    let r = run_raw(input, &[], tmp.path());
    assert_eq!(r.replies[0]["error"]["code"], -32700);
    assert!(r.replies[0]["id"].is_null());
    assert_eq!(r.replies[1]["id"], 5);
    assert_eq!(r.replies[1]["result"], json!({}));
}

#[test]
fn mcp_invalid_request_is_32600() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut input = b"[1,2]\n42\n{\"jsonrpc\":\"2.0\",\"id\":1}\n".to_vec();
    input.extend(lines(&[json!({"jsonrpc":"2.0","id":5,"method":"ping"})]));
    let r = run_raw(input, &[], tmp.path());
    assert_eq!(r.replies.len(), 4, "{}", r.stdout);
    for reply in &r.replies[..3] {
        assert_eq!(reply["error"]["code"], -32600, "{reply}");
    }
    assert_eq!(r.replies[3]["id"], 5);
}

#[test]
fn mcp_overlong_line_is_rejected_and_skipped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut input = vec![b'x'; 16 * 1024 * 1024 + 10];
    input.push(b'\n');
    input.extend(lines(&[json!({"jsonrpc":"2.0","id":9,"method":"ping"})]));
    let r = run_raw(input, &[], tmp.path());
    assert_eq!(r.replies.len(), 2, "{}", r.stdout.len());
    let code = r.replies[0]["error"]["code"].as_i64().expect("code");
    assert!(code == -32700 || code == -32600, "{code}");
    assert!(r.replies[0]["id"].is_null());
    assert_eq!(r.replies[1]["id"], 9);
}

#[test]
fn mcp_notification_gets_no_reply() {
    let tmp = tempfile::tempdir().expect("tmp");
    let r = run(
        &[
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","method":"nope"}),
            json!({"jsonrpc":"2.0","id":7,"method":"ping"}),
        ],
        tmp.path(),
    );
    assert_eq!(r.replies.len(), 1, "{}", r.stdout);
    assert_eq!(r.replies[0]["id"], 7);
}

#[test]
fn mcp_tool_error_is_result() {
    let tmp = tempfile::tempdir().expect("tmp");
    let r = run(
        &[
            call(1, "pbo_inspect", &json!({"path":"missing.pbo"})),
            call(2, "pbo_inspect", &json!({})),
            call(3, "pbo_inspect", &json!({"path": 5})),
            call(4, "no_such_tool", &json!({})),
            json!({"jsonrpc":"2.0","id":5,"method":"ping"}),
        ],
        tmp.path(),
    );
    assert!(is_error(&r.replies[0]));
    assert!(
        text(&r.replies[0]).contains("missing.pbo"),
        "{}",
        text(&r.replies[0])
    );
    assert!(is_error(&r.replies[1]));
    assert!(text(&r.replies[1]).contains("path"));
    assert!(is_error(&r.replies[2]));
    assert_eq!(r.replies[3]["error"]["code"], -32602);
    assert_eq!(r.replies[4]["result"], json!({}));
}

struct FakeSteam {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    cache: PathBuf,
    home: PathBuf,
}

fn fake_steam() -> FakeSteam {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("steam");
    let cache = tmp.path().join("cache");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(root.join("steamapps/common/DayZ/dta")).expect("dirs");
    fs::write(
        root.join("steamapps/appmanifest_221100.acf"),
        "\"AppState\"\n{\n\t\"installdir\"\t\t\"DayZ\"\n\t\"buildid\"\t\t\"111\"\n}\n",
    )
    .expect("manifest");
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api/vanilla/scripts");
    let pbo = PBO::from_directory(&scripts, &PackOptions::pack(), None, &mut Warnings::new())
        .expect("pbo");
    pbo.write(
        &mut fs::File::create(root.join("steamapps/common/DayZ/dta/scripts.pbo")).expect("create"),
    )
    .expect("write");
    FakeSteam {
        _tmp: tmp,
        root,
        cache,
        home,
    }
}

#[test]
fn mcp_api_source_context_and_lines() {
    let w = fake_steam();
    let r = run_raw(
        lines(&[
            call(1, "api_source", &json!({"name":"Base","context":1})),
            call(2, "api_source", &json!({"name":"Base","lines":"3-4"})),
            call(
                3,
                "api_source",
                &json!({"name":"Base","lines":"3-4","json":true}),
            ),
            call(
                4,
                "api_source",
                &json!({"name":"Base","context":1,"lines":"3-4"}),
            ),
        ]),
        &[
            ("STEAM_ROOT", &w.root),
            ("DAYZMAKE_CACHE", &w.cache),
            ("HOME", &w.home),
        ],
        &w.home,
    );
    assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    assert!(
        text(&r.replies[0]).contains("\n3| class Base\n"),
        "{}",
        text(&r.replies[0])
    );
    assert!(text(&r.replies[1]).contains("\n3| class Base\n4| {\n"));
    let j: Value = serde_json::from_str(text(&r.replies[2])).expect("json text");
    assert_eq!(j["files"][0]["lines"][1]["text"], "{", "{j}");
    assert!(is_error(&r.replies[3]));
}

#[test]
fn mcp_api_grep() {
    let w = fake_steam();
    let r = run_raw(
        lines(&[
            call(
                1,
                "api_grep",
                &json!({"regex":"class base","ignore_case":true}),
            ),
            call(2, "api_grep", &json!({"regex":"class base"})),
            call(
                3,
                "api_grep",
                &json!({"regex":"modded","json":true,"max":1}),
            ),
            call(4, "api_grep", &json!({"regex":"(bad"})),
        ]),
        &[
            ("STEAM_ROOT", &w.root),
            ("DAYZMAKE_CACHE", &w.cache),
            ("HOME", &w.home),
        ],
        &w.home,
    );
    assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    assert!(
        text(&r.replies[0]).starts_with("scripts\\3_Game\\Base.c:3: class Base"),
        "{}",
        text(&r.replies[0])
    );
    assert!(text(&r.replies[1]).starts_with("no match"));
    let j: Value = serde_json::from_str(text(&r.replies[2])).expect("json text");
    assert_eq!(j.as_array().map(Vec::len), Some(1), "{j}");
    assert_eq!(j[0]["source"], "vanilla");
    assert!(is_error(&r.replies[3]));
}

#[test]
fn mcp_api_class_with_fake_steam() {
    let w = fake_steam();
    let r = run_raw(
        lines(&[
            call(1, "api_class", &json!({"name":"Base","max":5})),
            call(2, "api_class", &json!({"name":"Base","json":true})),
            call(
                3,
                "api_source",
                &json!({"name":"Base","cat":true,"json":true}),
            ),
            call(4, "api_source", &json!({"name":"Base","cat":true})),
            call(5, "api_update", &json!({})),
            call(6, "api_class", &json!({"name":"Base","experimental":true})),
            call(
                7,
                "api_class",
                &json!({"name":"Base","mods":"@nonexistent-mod"}),
            ),
        ]),
        &[
            ("STEAM_ROOT", &w.root),
            ("DAYZMAKE_CACHE", &w.cache),
            ("HOME", &w.home),
        ],
        &w.home,
    );
    assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    assert!(
        text(&r.replies[0]).contains("class Base"),
        "{}",
        text(&r.replies[0])
    );
    assert!(text(&r.replies[0]).lines().count() <= 6);
    let j: Value = serde_json::from_str(text(&r.replies[1])).expect("json text");
    assert!(j.is_object() || j.is_array());
    let files: Value = serde_json::from_str(text(&r.replies[2])).expect("json text");
    assert!(files["files"][0]["text"].is_string(), "{files}");
    assert!(text(&r.replies[3]).contains("class Base"));
    assert!(
        text(&r.replies[4]).contains("vanilla-stable:"),
        "{}",
        text(&r.replies[4])
    );
    assert!(is_error(&r.replies[5]));
    assert!(is_error(&r.replies[6]));
}

#[test]
fn mcp_pbo_cat_binary_requires_target() {
    let tmp = tempfile::tempdir().expect("tmp");
    fs::write(
        tmp.path().join("a.pbo"),
        make_pbo(&[("bin.dat", &[0xff, 0xfe, 0x00, 0x80]), ("t.txt", b"hello")]),
    )
    .expect("write");
    let r = run(
        &[
            call(1, "pbo_cat", &json!({"pbo":"a.pbo","file":"t.txt"})),
            call(2, "pbo_cat", &json!({"pbo":"a.pbo","file":"bin.dat"})),
            call(
                3,
                "pbo_cat",
                &json!({"pbo":"a.pbo","file":"bin.dat","target":"b.dat"}),
            ),
            call(
                4,
                "pbo_cat",
                &json!({"pbo":"a.pbo","file":"bin.dat","target":"b.dat"}),
            ),
            call(
                5,
                "pbo_cat",
                &json!({"pbo":"a.pbo","file":"bin.dat","target":"b.dat","force":true}),
            ),
            call(6, "pbo_cat", &json!({"pbo":"a.pbo","file":"nope.txt"})),
        ],
        tmp.path(),
    );
    assert_eq!(text(&r.replies[0]), "hello");
    assert!(is_error(&r.replies[1]));
    assert!(
        text(&r.replies[1]).contains("target"),
        "{}",
        text(&r.replies[1])
    );
    assert!(!is_error(&r.replies[2]), "{}", text(&r.replies[2]));
    assert_eq!(
        fs::read(tmp.path().join("b.dat")).expect("read"),
        vec![0xff, 0xfe, 0x00, 0x80]
    );
    assert!(is_error(&r.replies[3]), "no overwrite without force");
    assert!(!is_error(&r.replies[4]));
    assert!(is_error(&r.replies[5]));
}

#[test]
fn mcp_unpack_dotdot_entry_stays_inside() {
    let tmp = tempfile::tempdir().expect("tmp");
    fs::write(
        tmp.path().join("evil.pbo"),
        make_pbo(&[
            ("..\\..\\escape.txt", b"x"),
            ("a\\..\\b.txt", b"x"),
            ("good\\ok.txt", b"y"),
        ]),
    )
    .expect("write");
    let r = run(
        &[call(
            1,
            "pbo_unpack",
            &json!({"pbo":"evil.pbo","target":"work/out"}),
        )],
        tmp.path(),
    );
    assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    assert!(tmp.path().join("work/out/good/ok.txt").is_file());
    assert!(!tmp.path().join("escape.txt").exists());
    assert!(!tmp.path().join("work/escape.txt").exists());
    assert!(!tmp.path().join("work/out/escape.txt").exists());
}

#[test]
fn mcp_file_tools_roundtrip() {
    let tmp = tempfile::tempdir().expect("tmp");
    let p = tmp.path();
    fs::create_dir_all(p.join("src")).expect("mkdir");
    fs::write(
        p.join("src/config.cpp"),
        "class CfgPatches { class X { units[] = {}; }; };\n",
    )
    .expect("write");
    fs::write(p.join("src/a.txt"), "hi").expect("write");
    let key = p.join("k");
    let r = run(
        &[
            call(1, "key_generate", &json!({"name": s(&key)})),
            call(
                2,
                "config_rapify",
                &json!({"source":"src/config.cpp","target":"config.bin"}),
            ),
            call(3, "config_derapify", &json!({"source":"config.bin"})),
            call(4, "config_preprocess", &json!({"source":"src/config.cpp"})),
            call(
                5,
                "pbo_pack",
                &json!({"source":"src","target":"a.pbo","key":s(&p.join("k.biprivatekey")),"headerext":["prefix=x"]}),
            ),
            call(6, "pbo_pack", &json!({"source":"src","target":"c.pbo"})),
            call(
                7,
                "pbo_verify",
                &json!({"public_key":s(&p.join("k.bikey")),"pbo":"a.pbo","signature":"a.pbo.k.bisign"}),
            ),
            call(
                8,
                "pbo_build",
                &json!({"source":"src","target":"b.pbo","exclude":["*.txt"],"include":["."]}),
            ),
            call(9, "pbo_inspect", &json!({"path":"b.pbo"})),
            call(
                10,
                "pbo_sign",
                &json!({"private_key":s(&p.join("k.biprivatekey")),"pbo":"b.pbo"}),
            ),
            call(
                11,
                "binarize",
                &json!({"source":"missing.p3d","target":"o.p3d"}),
            ),
            call(
                12,
                "paa_to_image",
                &json!({"source":"missing.paa","target":"o.png"}),
            ),
            call(
                13,
                "image_to_paa",
                &json!({"source":"missing.png","target":"o.paa"}),
            ),
        ],
        p,
    );
    for (i, reply) in r.replies.iter().enumerate().take(10) {
        assert!(!is_error(reply), "call {}: {}", i + 1, text(reply));
    }
    assert!(
        text(&r.replies[2]).contains("CfgPatches"),
        "{}",
        text(&r.replies[2])
    );
    assert!(text(&r.replies[3]).contains("CfgPatches"));
    assert!(p.join("config.bin").is_file());
    assert!(p.join("a.pbo.k.bisign").is_file());
    assert!(text(&r.replies[8]).contains("config.bin") && !text(&r.replies[8]).contains("a.txt"));
    assert!(p.join("b.pbo.k.bisign").is_file());
    for reply in &r.replies[10..13] {
        assert!(is_error(reply));
    }
}

#[test]
fn mcp_truncates_huge_output() {
    let tmp = tempfile::tempdir().expect("tmp");
    fs::write(tmp.path().join("big.txt"), "a\n".repeat(150_000)).expect("write");
    let r = run(
        &[call(1, "config_preprocess", &json!({"source":"big.txt"}))],
        tmp.path(),
    );
    let t = text(&r.replies[0]);
    assert!(t.len() < 200_100, "{}", t.len());
    assert!(
        t.ends_with("call again with offset=200000]")
            && t.contains("[truncated: showing bytes 0-200000 of 3000"),
        "{}",
        &t[t.len() - 60..]
    );
}

#[test]
fn mcp_api_mods_tool() {
    let w = fake_steam();
    let dir = w.root.join("steamapps/workshop/content/221100/100");
    fs::create_dir_all(dir.join("addons")).expect("mkdir");
    fs::write(
        dir.join("addons/x.pbo"),
        make_pbo(&[("X.c", b"class McpMod\n{\n}\n")]),
    )
    .expect("pbo");
    fs::write(dir.join("meta.cpp"), "name = \"Mcp Mod\";\n").expect("meta");
    let r = run_raw(
        lines(&[
            call(1, "api_mods", &json!({})),
            call(2, "api_mods", &json!({"json":true})),
            call(3, "api_class", &json!({"name":"McpMod","mods":"all"})),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}),
        ]),
        &[
            ("STEAM_ROOT", &w.root),
            ("DAYZMAKE_CACHE", &w.cache),
            ("HOME", &w.home),
        ],
        &w.home,
    );
    assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    assert!(
        text(&r.replies[0]).contains("Mcp Mod"),
        "{}",
        text(&r.replies[0])
    );
    let j: Value = serde_json::from_str(text(&r.replies[1])).expect("json text");
    assert_eq!(j["mods"][0]["id"], 100);
    assert!(
        text(&r.replies[2]).contains("class McpMod"),
        "{}",
        text(&r.replies[2])
    );
    let tools = r.replies[3]["result"]["tools"].as_array().expect("tools");
    for t in tools
        .iter()
        .filter(|t| t["name"] == "api_mods" || t["name"] == "api_class")
    {
        let desc = t["description"].as_str().unwrap_or_default().to_string()
            + t["inputSchema"]["properties"]["mods"]["description"]
                .as_str()
                .unwrap_or_default();
        assert!(desc.contains("while"), "{t}");
    }
}

/// Fetches `file` of `a.pbo` in chunks, following the `offset=` hint of each truncation note.
fn fetch_in_chunks(tool: &str, base: &Value, tmp: &Path) -> String {
    let mut out = String::new();
    let mut offset = 0u64;
    for round in 0..10 {
        let mut args = base.clone();
        args["offset"] = json!(offset);
        let r = run(&[call(1, tool, &args)], tmp);
        assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
        let t = text(&r.replies[0]);
        let Some((body, note)) = t.rsplit_once("\n[truncated: ") else {
            let body = t.rsplit_once("\n[showing bytes ").map_or(t, |(b, _)| b);
            out.push_str(body);
            return out;
        };
        out.push_str(body);
        assert!(note.contains("call again with offset="), "{note}");
        offset = note
            .split("offset=")
            .nth(1)
            .and_then(|n| n.trim_end_matches(']').parse().ok())
            .expect("an offset");
        assert!(round < 9, "too many chunks");
    }
    out
}

#[test]
fn mcp_pbo_cat_chunks_reassemble_the_file() {
    let tmp = tempfile::tempdir().expect("tmp");
    let original = "class A { // é\n};\n".repeat(20_000);
    assert!(original.len() > 300_000);
    fs::write(
        tmp.path().join("a.pbo"),
        make_pbo(&[("big.c", original.as_bytes())]),
    )
    .expect("write");
    let got = fetch_in_chunks(
        "pbo_cat",
        &json!({"pbo":"a.pbo","file":"big.c"}),
        tmp.path(),
    );
    assert_eq!(got, original);
    // an explicit length
    let r = run(
        &[call(
            1,
            "pbo_cat",
            &json!({"pbo":"a.pbo","file":"big.c","offset":6,"length":4}),
        )],
        tmp.path(),
    );
    assert!(
        text(&r.replies[0]).starts_with("A { "),
        "{}",
        text(&r.replies[0])
    );
    assert!(
        text(&r.replies[0]).contains("offset=10]"),
        "{}",
        text(&r.replies[0])
    );
}

#[test]
fn mcp_api_source_cat_chunks_and_target() {
    let w = fake_steam();
    let env = [
        ("STEAM_ROOT", w.root.as_path()),
        ("DAYZMAKE_CACHE", w.cache.as_path()),
        ("HOME", w.home.as_path()),
    ];
    let whole = run_raw(
        lines(&[call(1, "api_source", &json!({"name":"Base","cat":true}))]),
        &env,
        &w.home,
    );
    let full = text(&whole.replies[0]).to_string();
    // two chunks of 100 bytes each, reassembled
    let a = run_raw(
        lines(&[call(
            1,
            "api_source",
            &json!({"name":"Base","cat":true,"length":100}),
        )]),
        &env,
        &w.home,
    );
    let first = text(&a.replies[0]);
    let (body1, note) = first.rsplit_once("\n[truncated: ").expect("truncated");
    assert!(note.contains("showing bytes 0-100 of"), "{note}");
    assert!(note.contains("offset=100]"), "{note}");
    let b = run_raw(
        lines(&[call(
            1,
            "api_source",
            &json!({"name":"Base","cat":true,"offset":100}),
        )]),
        &env,
        &w.home,
    );
    let (body2, last) = text(&b.replies[0])
        .rsplit_once("\n[showing bytes ")
        .expect("a closing note");
    assert!(last.starts_with("100-"), "{last}");
    assert_eq!(format!("{body1}{body2}"), full);
    // target writes the file instead
    let target = w.home.join("Base.c");
    let t = run_raw(
        lines(&[call(
            1,
            "api_source",
            &json!({"name":"Base","cat":true,"target":s(&target)}),
        )]),
        &env,
        &w.home,
    );
    assert!(!is_error(&t.replies[0]), "{}", text(&t.replies[0]));
    assert_eq!(fs::read_to_string(&target).expect("read"), full);
}

#[test]
fn mcp_mods_arguments_mention_the_slow_first_run() {
    let tmp = tempfile::tempdir().expect("tmp");
    let r = run(
        &[json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})],
        tmp.path(),
    );
    let tools = r.replies[0]["result"]["tools"].as_array().expect("tools");
    let find = |name: &str| tools.iter().find(|t| t["name"] == name).expect("tool");
    let mods = find("api_class")["inputSchema"]["properties"]["mods"]["description"]
        .as_str()
        .expect("mods");
    assert!(
        mods.contains("first run") && mods.contains("api_update"),
        "{mods}"
    );
    let api_mods = find("api_mods")["description"].as_str().expect("desc");
    assert!(
        api_mods.contains("first run") && api_mods.contains("api_update"),
        "{api_mods}"
    );
}

#[test]
fn mcp_every_tool_that_writes_a_target_offers_force() {
    let tmp = tempfile::tempdir().expect("tmp");
    let r = run(
        &[json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})],
        tmp.path(),
    );
    for t in r.replies[0]["result"]["tools"].as_array().expect("tools") {
        let props = &t["inputSchema"]["properties"];
        if props.get("target").is_some() {
            assert!(props.get("force").is_some(), "{} has no force", t["name"]);
        }
    }
}

/// 12 pre-rapified configs: one `prerapified-file` warning each when built.
fn quota_dir() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("tempdir");
    let work = root.path().canonicalize().expect("canonical").join("q");
    fs::create_dir_all(&work).expect("mkdir");
    for i in 0..12 {
        fs::write(work.join(format!("c{i:02}.cpp")), b"\0raP\0\0\0\0").expect("write");
    }
    (root, work)
}

fn prerapified_warnings(extra: &[&str]) -> usize {
    let (_root, work) = quota_dir();
    let req = call(
        1,
        "pbo_build",
        &json!({"source": ".", "target": "../a.pbo"}),
    );
    let r = run_with(extra, lines(&[req]), &[], &work);
    text(&r.replies[0]).matches("[prerapified-file]").count()
}

#[test]
fn mcp_verbose_keeps_all_warnings() {
    assert_eq!(prerapified_warnings(&["-v"]), 12);
    assert_eq!(prerapified_warnings(&[]), 10);
}

#[test]
fn mcp_failed_build_keeps_the_existing_target() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir(p.join("src")).unwrap();
    fs::write(p.join("src/config.cpp"), "class A {\n").unwrap();
    fs::write(p.join("out.pbo"), b"good pbo bytes").unwrap();
    let r = run(
        &[
            call(
                1,
                "pbo_build",
                &json!({"source":"src","target":"out.pbo","force":true}),
            ),
            call(
                2,
                "pbo_build",
                &json!({"source":"nodir","target":"new.pbo"}),
            ),
        ],
        p,
    );
    assert!(is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    assert!(is_error(&r.replies[1]), "{}", text(&r.replies[1]));
    assert_eq!(fs::read(p.join("out.pbo")).unwrap(), b"good pbo bytes");
    assert!(!p.join("new.pbo").exists());
    let mut names: Vec<_> = fs::read_dir(p)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["out.pbo", "src"]);
}

#[cfg(unix)]
#[test]
fn mcp_key_generate_creates_the_private_key_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let r = run(
        &[call(1, "key_generate", &json!({"name": s(&p.join("k"))}))],
        p,
    );
    assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    let mode = fs::metadata(p.join("k.biprivatekey"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn mcp_verify_and_sign_reject_a_pbo_with_a_stale_trailer() {
    let tmp = tempfile::tempdir().expect("tmp");
    let sign = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bi/sign");
    let mut bytes = fs::read(sign.join("testsig.pbo")).expect("read");
    *bytes.get_mut(261).unwrap() ^= 0x20;
    fs::write(tmp.path().join("t.pbo"), bytes).expect("write");
    let r = run(
        &[
            call(
                1,
                "pbo_verify",
                &json!({"public_key":s(&sign.join("armk.bikey")),"pbo":"t.pbo","signature":s(&sign.join("testsig.pbo.armk.bisign"))}),
            ),
            call(
                2,
                "pbo_sign",
                &json!({"private_key":s(&sign.join("armk.biprivatekey")),"pbo":"t.pbo"}),
            ),
        ],
        tmp.path(),
    );
    for reply in &r.replies {
        assert!(is_error(reply), "{}", text(reply));
        assert!(text(reply).contains("checksum mismatch"), "{}", text(reply));
    }
}

#[test]
fn mcp_config_derapify_indent() {
    let tmp = tempfile::tempdir().expect("tmp");
    let p = tmp.path();
    fs::write(p.join("c.cpp"), "class A { x = 1; };\n").expect("write");
    let r = run(
        &[
            call(
                1,
                "config_rapify",
                &json!({"source":"c.cpp","target":"c.bin"}),
            ),
            call(
                2,
                "config_derapify",
                &json!({"source":"c.bin","indent":"\t"}),
            ),
            call(
                3,
                "config_derapify",
                &json!({"source":"c.bin","indent":"x"}),
            ),
        ],
        p,
    );
    assert!(
        text(&r.replies[1]).contains("\n\tx = 1;"),
        "{}",
        text(&r.replies[1])
    );
    assert!(is_error(&r.replies[2]));
    assert!(
        text(&r.replies[2]).contains("indent"),
        "{}",
        text(&r.replies[2])
    );
}

#[test]
fn mcp_image_to_paa_compress() {
    let tmp = tempfile::tempdir().expect("tmp");
    let p = tmp.path();
    image::RgbImage::from_fn(256, 256, |x, y| {
        image::Rgb([
            u8::try_from(x / 4).unwrap_or(255),
            u8::try_from(y / 4).unwrap_or(255),
            9,
        ])
    })
    .save(p.join("in.png"))
    .expect("png");
    let r = run(
        &[call(
            1,
            "image_to_paa",
            &json!({"source":"in.png","target":"o.paa","type":"dxt1","compress":true}),
        )],
        p,
    );
    assert!(!is_error(&r.replies[0]), "{}", text(&r.replies[0]));
    assert!(p.join("o.paa").is_file());
}

#[test]
fn mcp_unpack_filters_with_exclude_and_only() {
    let tmp = tempfile::tempdir().expect("tmp");
    fs::write(
        tmp.path().join("a.pbo"),
        make_pbo(&[
            ("scripts\\a.c", b"c"),
            ("model.p3d", b"p"),
            ("config.bin", b"b"),
            ("data\\x.paa", b"t"),
        ]),
    )
    .expect("write");
    let r = run(
        &[
            call(
                1,
                "pbo_unpack",
                &json!({"pbo":"a.pbo","target":"only","only":["*.c","*config.bin"]}),
            ),
            call(
                2,
                "pbo_unpack",
                &json!({"pbo":"a.pbo","target":"excl","exclude":["*.p3d"]}),
            ),
            call(
                3,
                "pbo_unpack",
                &json!({"pbo":"a.pbo","target":"both","only":["*.c","*.p3d"],"exclude":["scripts/*"]}),
            ),
        ],
        tmp.path(),
    );
    for reply in &r.replies {
        assert!(!is_error(reply), "{}", text(reply));
    }
    let p = tmp.path();
    assert!(p.join("only/scripts/a.c").is_file());
    assert!(p.join("only/config.bin").is_file());
    assert!(!p.join("only/model.p3d").exists());
    assert!(!p.join("only/data/x.paa").exists());
    assert!(!p.join("excl/model.p3d").exists());
    assert!(p.join("excl/data/x.paa").is_file());
    assert!(!p.join("both/scripts/a.c").exists());
    assert!(p.join("both/model.p3d").is_file());
    assert!(!p.join("both/config.bin").exists());
}
