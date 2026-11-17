//! Golden tests against output produced by BI's `CfgConvert` (see tests/fixtures/bi/README.md).
use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;

use dayzmake_core::config::Config;
use dayzmake_core::warning::Warnings;

#[expect(clippy::unwrap_used, reason = "test helper")]
fn golden(name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/cfgconvert");
    let src = dir.join(format!("{name}.cpp"));
    let expected = std::fs::read_to_string(dir.join(format!("{name}.expected.cpp"))).unwrap();

    let mut input = File::open(&src).unwrap();
    let config = Config::read(
        &mut input,
        Some(src.clone()),
        std::slice::from_ref(&dir),
        &mut Warnings::new(),
    )
    .unwrap();
    let rapified = config.to_cursor().unwrap();
    let mut cursor = Cursor::new(rapified.into_inner());
    let decoded = Config::read_rapified(&mut cursor).unwrap();
    assert_eq!(decoded.to_string().unwrap(), expected, "golden {name}");

    // CfgConvert's own bytes, read by our reader.
    let bi = std::fs::read(dir.join(format!("{name}.bin"))).unwrap();
    let decoded = Config::read_rapified(&mut Cursor::new(bi)).unwrap();
    assert_eq!(
        decoded.to_string().unwrap(),
        expected,
        "golden {name} (CfgConvert .bin)"
    );
}

#[test]
fn golden_g() {
    golden("g");
}
#[test]
fn golden_w1() {
    golden("w1");
}
#[test]
fn golden_us() {
    golden("us");
}
#[test]
fn golden_c51() {
    golden("c51");
}
#[test]
fn golden_rap() {
    golden("rap");
}
#[test]
fn golden_ma() {
    golden("ma");
}
#[test]
fn golden_m28() {
    golden("m28");
}
#[test]
fn golden_case() {
    golden("case");
}
#[test]
fn golden_ev() {
    golden("ev");
}
#[test]
fn golden_ex() {
    golden("ex");
}
#[test]
fn golden_e23() {
    golden("e23");
}
#[test]
fn golden_mac() {
    golden("mac");
}

// A last entry with a line end but no `;`: CfgConvert keeps the value type (2026-11-17).
#[test]
fn golden_last_float() {
    golden("last_float");
}
#[test]
fn golden_last_exp() {
    golden("last_exp");
}
#[test]
fn golden_last_str() {
    golden("last_str");
}
#[test]
fn golden_last_mid() {
    golden("last_mid");
}
#[test]
fn golden_last_endif() {
    golden("last_endif");
}
#[test]
fn golden_last_define() {
    golden("last_define");
}
#[test]
fn golden_last_eval() {
    golden("last_eval");
}

/// `enum { A, B = 5, A };`: `CfgConvert` accepts it silently and its table holds `a` once, at the
/// first position, with the last value (`a = 6`, `b = 5`). dayzmake keeps every name, so
/// the last entry of a name is the one that has to agree.
#[test]
fn enum_duplicate_resolves_like_cfgconvert() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/cfgconvert");
    for (name, table) in [
        ("enum_dup", vec![("a", 6), ("b", 5)]),
        ("enum_dup2", vec![("a", 2)]),
    ] {
        let bi = std::fs::read(dir.join(format!("{name}.bin"))).unwrap();
        let bi = Config::read_rapified(&mut Cursor::new(bi)).unwrap();
        let table: Vec<(String, i32)> =
            table.into_iter().map(|(n, v)| (n.to_string(), v)).collect();
        assert_eq!(bi.enum_values(), table.as_slice(), "{name}");

        let src = std::fs::read_to_string(dir.join(format!("{name}.cpp"))).unwrap();
        let mut warnings = Warnings::new();
        let ours = Config::from_string(src, None, &Vec::new(), &mut warnings).unwrap();
        // only the unquoted `y = A;` is warned about; the duplicate names are not
        assert!(
            warnings
                .warnings()
                .iter()
                .all(|w| w.name == Some("unquoted-string")),
            "{name}: silent about the enum, like CfgConvert"
        );
        for (n, value) in &table {
            let last = ours.enum_values().iter().rev().find(|(x, _)| x == n);
            assert_eq!(last.map(|(_, v)| *v), Some(*value), "{name}: enum {n}");
        }
    }
}
