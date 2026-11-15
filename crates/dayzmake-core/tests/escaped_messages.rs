//! Text taken from untrusted files never reaches an error or warning as a raw control character
//! (issues #15 and #22).

use std::io::Cursor;
#[cfg(unix)]
use std::path::PathBuf;

use dayzmake_core::config::Config;
use dayzmake_core::paa::Paa;
use dayzmake_core::warning::Warnings;

#[cfg(unix)]
use dayzmake_core::preprocess::preprocess;

/// Asserts the message shows `escaped` and holds no raw ESC.
fn assert_escaped(msg: &str, escaped: &str) {
    assert!(msg.contains(escaped), "{msg:?}");
    assert!(!msg.contains('\u{1b}'), "{msg:?}");
}

#[cfg(unix)]
#[test]
fn error_inside_an_include_with_a_control_character_name_is_escaped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.cpp"), "").unwrap();
    std::fs::write(dir.path().join("a\u{1b}b.hpp"), "#endif\n").unwrap();
    let err = preprocess(
        String::from("#include \"a\u{1b}b.hpp\"\n"),
        Some(dir.path().join("main.cpp")),
        &Vec::<PathBuf>::new(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert_escaped(&err.to_string(), "a\\u{1b}b.hpp");
}

#[cfg(unix)]
#[test]
fn parse_error_location_in_an_include_is_escaped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.cpp"), "").unwrap();
    std::fs::write(dir.path().join("a\u{1b}b.hpp"), "class {\n").unwrap();
    let err = Config::from_string(
        String::from("#include \"a\u{1b}b.hpp\"\n"),
        Some(dir.path().join("config.cpp")),
        &[],
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert_escaped(&err.to_string(), "a\\u{1b}b.hpp");
}

#[cfg(unix)]
#[test]
fn include_case_warning_is_escaped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("z")).unwrap();
    std::fs::write(dir.path().join("z/\u{1b}foo.hpp"), "x\n").unwrap();
    let mut warnings = Warnings::new();
    preprocess(
        String::from("#include \"\\z\\\u{1b}Foo.hpp\"\n"),
        None,
        &[dir.path().to_path_buf()],
        &mut warnings,
    )
    .unwrap();
    let w = warnings
        .warnings()
        .iter()
        .find(|w| w.name == Some("include-case"))
        .expect("include-case warning");
    assert_escaped(&w.to_string(), "\\u{1b}Foo.hpp");
}

#[test]
fn rapified_class_name_is_escaped_in_nested_errors() {
    let config = Config::from_string(
        String::from("class AAAA { class BBBB { x=1; }; };"),
        None,
        &[],
        &mut Warnings::new(),
    )
    .unwrap();
    let mut bytes = Vec::new();
    config.write_rapified(&mut bytes).unwrap();
    let pos = bytes.windows(4).position(|w| w == b"AAAA").unwrap();
    bytes[pos] = 0x1b;
    bytes.truncate(bytes.len() - 5);
    let err = Config::read_rapified(&mut Cursor::new(bytes)).unwrap_err();
    assert_escaped(&err.to_string(), "\\u{1b}AAA");
}

#[test]
fn eval_body_is_escaped_in_errors() {
    let err = Config::from_string(
        String::from("x = __EVAL(\u{1b}[31m);\n"),
        None,
        &[],
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert_escaped(&err.to_string(), "\\u{1b}[31m");
}

#[test]
fn exec_statement_is_escaped_in_errors() {
    let err = Config::from_string(
        String::from("__EXEC(\u{1b}[31m);\n"),
        None,
        &[],
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert_escaped(&err.to_string(), "\\u{1b}[31m");
}

#[test]
fn paa_tagg_name_is_escaped_in_errors() {
    let mut bytes = vec![0x01, 0xff];
    bytes.extend_from_slice(b"\x1bGGATSFF");
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    let err = Paa::read(&mut Cursor::new(bytes)).unwrap_err();
    assert_escaped(&err.to_string(), "\\u{1b}GGATSFF");
}

#[test]
fn unquoted_string_warning_is_escaped() {
    let mut warnings = Warnings::new();
    Config::from_string(
        String::from("class A { x = \u{1b}[31mRED; y[] = {\u{1b}[32mGREEN}; };\n"),
        None,
        &[],
        &mut warnings,
    )
    .unwrap();
    let messages: Vec<String> = warnings
        .warnings()
        .iter()
        .filter(|w| w.name == Some("unquoted-string"))
        .map(ToString::to_string)
        .collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert_escaped(&messages[0], "\\u{1b}[31mRED");
    assert_escaped(&messages[1], "\\u{1b}[32mGREEN");
}

#[test]
fn p3d_tagg_name_is_escaped_in_errors() {
    use dayzmake_core::p3d::P3D;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MLOD");
    bytes.extend_from_slice(&257u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes()); // one LOD
    bytes.extend_from_slice(b"P3DM");
    bytes.extend_from_slice(&28u32.to_le_bytes());
    bytes.extend_from_slice(&256u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 16]); // no points, normals, faces; flags
    bytes.extend_from_slice(b"TAGG");
    bytes.push(1);
    bytes.extend_from_slice(b"\x1b[31mT\0");
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    let err = P3D::read(&mut Cursor::new(bytes)).unwrap_err();
    assert_escaped(&err.to_string(), "\\u{1b}[31mT");
}
