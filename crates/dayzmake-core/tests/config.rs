//! Config parsing, rapification and derapification tests.
use std::io::{Cursor, Seek, SeekFrom};

use dayzmake_core::config::*;
use dayzmake_core::warning::Warnings;

#[test]
fn config_read() {
    let input = String::from(
        "\
#define VERSIONAR {3,5, 0, 0}
#define FOO(x  , y ) #x z x_y x##_##y
#define QUOTE(x) #x
#define DOUBLES(x,y) x##_##y
    #define ADDON DOUBLES(ace, frag)

class CfgPatches {
    class ADDON{
        units[] = { };
        weapons[] = {};
        requiredVersion = 1.56;
        requiredAddons[] = {\"ace_common\"};
        author[] = {\"Nou\"}   ;
        version = QUOTE(3.5.0.0) ;versionStr=\"3.5.0.0\";
        versionAr [] = VERSIONAR;
    };
};",
    );
    let mut cursor = Cursor::new(input);

    let mut config = Config::read(&mut cursor, None, &Vec::new(), &mut Warnings::new()).unwrap();

    let mut rapified = config.to_cursor().unwrap();
    rapified.seek(SeekFrom::Start(0)).unwrap();

    config = Config::read_rapified(&mut rapified).unwrap();

    let output = config.to_string().unwrap();

    assert_eq!(
        "class CfgPatches {
    class ace_frag {
        units[] = {};
        weapons[] = {};
        requiredVersion = 1.56;
        requiredAddons[] = {\"ace_common\"};
        author[] = {\"Nou\"};
        version = \"3.5.0.0\";
        versionStr = \"3.5.0.0\";
        versionAr[] = {3, 5, 0, 0};
    };
};",
        output.trim()
    );
}

#[test]
fn eval_unsupported_errors() {
    let err = Config::from_string(
        String::from("x = __EVAL(floor 2.7);\n"),
        None,
        &Vec::new(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("not supported"), "{}", err);
}

/// Truncated rapified configs and garbage input must error out, not panic.
#[test]
fn garbage_is_an_error_not_a_panic() {
    let config = Config::from_string(
        String::from("class A { x = 1; arr[] = {1, 2.5, \"s\"}; };"),
        None,
        &Vec::new(),
        &mut Warnings::new(),
    )
    .unwrap();
    let rapified = config.to_cursor().unwrap().into_inner();

    // no truncation of a rapified config may panic
    for len in 0..rapified.len() {
        let mut cursor = Cursor::new(rapified[..len].to_vec());
        let _ = Config::read_rapified(&mut cursor);
    }

    // garbage without the rapified signature is an error
    let mut garbage = Cursor::new(vec![0xff, 0x00, 0x12, 0x7f, 0x80, 0x81]);
    assert!(Config::read_rapified(&mut garbage).is_err());
}

/// Integer literals that do not fit an i32 are an error, not a panic.
#[test]
fn integer_overflow_is_an_error() {
    let result = Config::from_string(
        String::from("x = 0x1FFFFFFFF;\n"),
        None,
        &Vec::new(),
        &mut Warnings::new(),
    );
    assert!(result.is_err());
}

/// The 16 byte header of a rapified config; the root body follows.
fn rapified_header() -> Vec<u8> {
    let mut bytes = b"\0raP".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&8u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // enum offset, unused by the reader
    bytes
}

fn read_bytes(bytes: Vec<u8>) -> Result<Config, std::io::Error> {
    Config::read_rapified(&mut Cursor::new(bytes))
}

#[test]
fn rapified_self_reference_is_an_error() {
    let mut bytes = rapified_header();
    bytes.push(0); // parent ""
    bytes.push(1); // one entry
    bytes.push(0); // class entry
    bytes.extend_from_slice(b"a\0");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // the root body itself
    let err = read_bytes(bytes).unwrap_err().to_string();
    assert!(err.contains("cycle") || err.contains("nesting"), "{err}");
}

#[test]
fn rapified_deep_array_nesting_is_an_error() {
    let mut bytes = rapified_header();
    bytes.push(0); // parent ""
    bytes.push(1); // one entry
    bytes.push(2); // array entry
    bytes.extend_from_slice(b"a\0");
    for _ in 0..10_000 {
        bytes.push(1); // one element
        bytes.push(3); // an array
    }
    bytes.push(0); // innermost array is empty
    let err = read_bytes(bytes).unwrap_err().to_string();
    assert!(err.contains("nesting"), "{err}");
}

#[test]
fn rapified_exponential_fanout_is_an_error() {
    let mut bytes = rapified_header();
    // every body has two class entries that both point at the next body
    for level in 0u32..30 {
        let next = 16 + (level + 1) * 16;
        bytes.push(0);
        bytes.push(2);
        for name in [b"a\0", b"b\0"] {
            bytes.push(0);
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&next.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&[0, 0]); // last body: no entries
    let err = read_bytes(bytes).unwrap_err().to_string();
    assert!(err.contains("shared"), "{err}");
}

/// Found by fuzzing: a class body offset that points back into itself recursed until the
/// stack overflowed. Reading must fail with an error instead.
#[test]
fn read_rapified_rejects_cyclic_class_bodies() {
    let data = include_bytes!("../../../tests/fixtures/fuzz/config_class_cycle.bin");
    assert!(Config::read_rapified(&mut Cursor::new(&data[..])).is_err());
}

/// Deeply nested arrays must be rejected, not recursed into until the stack overflows.
#[test]
fn read_rapified_rejects_deeply_nested_arrays() {
    // header, empty parent name, one entry: an array entry named "a" whose array nests 1M deep
    let mut data = b"\0raP\0\0\0\0\x08\0\0\0\0\0\0\0".to_vec();
    data.extend_from_slice(&[0, 1, 2, b'a', 0, 1]);
    data.extend(std::iter::repeat_n([3u8, 1], 1_000_000).flatten());
    assert!(Config::read_rapified(&mut Cursor::new(&data[..])).is_err());
}

/// 20 class bodies, each with two class entries that point at the next body, and a last body with two
/// 1 KB strings: 1.4 KB that derapify to over 1 GB when shared bodies are read once per reference.
fn shared_body_fan_out() -> Vec<u8> {
    const LEVELS: u32 = 20;
    let mut bytes = rapified_header();
    for level in 0..LEVELS - 1 {
        bytes.push(0); // parent ""
        bytes.push(2); // two entries
        let next = 16 + 16 * (level + 1);
        for name in [b"a\0", b"b\0"] {
            bytes.push(0); // class entry
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&next.to_le_bytes());
        }
    }
    bytes.push(0); // parent ""
    bytes.push(2); // two entries
    for name in [b"a\0", b"b\0"] {
        bytes.push(1); // variable
        bytes.push(0); // string
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&[b'x'; 1024]);
        bytes.push(0);
    }
    bytes
}

#[test]
fn rapified_shared_class_body_fan_out_is_an_error() {
    let bytes = shared_body_fan_out();
    assert!(bytes.len() < 3000, "{}", bytes.len());
    let started = std::time::Instant::now();
    let Err(err) = read_bytes(bytes) else {
        panic!("shared bodies are rejected");
    };
    let err = err.to_string();
    assert!(err.contains("shared"), "{err}");
    assert!(started.elapsed().as_secs() < 2);
}

#[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
fn rapify_src(src: &str) -> Vec<u8> {
    let config = Config::from_string(String::from(src), None, &[], &mut Warnings::new()).unwrap();
    config.to_cursor().unwrap().into_inner().into_vec()
}

#[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
fn derapify(bytes: Vec<u8>) -> String {
    read_bytes(bytes).unwrap().to_string().unwrap()
}

#[test]
fn derapify_keeps_nested_delete() {
    let text = derapify(rapify_src("class A { x = 1; delete B; };"));
    assert!(text.contains("delete B;"), "{text}");
    assert!(!text.contains("class B"), "{text}");
}

#[test]
fn derapify_keeps_top_level_delete() {
    let text = derapify(rapify_src("delete X;\nclass Y {};"));
    assert!(text.contains("delete X;"), "{text}");
}

#[test]
fn rapified_delete_roundtrips_byte_for_byte() {
    let bytes1 = rapify_src("class A { x = 1; delete B; };\ndelete X;\nclass Y {};");
    let bytes2 = read_bytes(bytes1.clone())
        .unwrap()
        .to_cursor()
        .unwrap()
        .into_inner()
        .into_vec();
    assert_eq!(bytes1, bytes2);
}

fn parse_err(input: &str) -> String {
    let mut w = Warnings::default();
    // An unexpected success yields an empty message, which the callers' assertions reject.
    Config::from_string(input.to_string(), None, &[], &mut w)
        .err()
        .map_or_else(String::new, |e| e.to_string())
}

#[test]
fn parse_error_with_nbsp_indent_is_an_error() {
    let msg = parse_err("\u{a0}foo = 1;\n");
    assert!(msg.contains("Unexpected token"), "{msg}");
    assert!(msg.contains("\n  ^\n"), "caret at the line start: {msg}");
    assert!(msg.contains("\"\\u{a0}\""), "visible token: {msg}");
}

#[test]
fn parse_error_with_ideographic_space_indent_is_an_error() {
    let msg = parse_err("\u{3000}\u{3000}class A { = 1; };");
    assert!(msg.contains("Unexpected token"), "{msg}");
    assert!(msg.contains("\n  ^\n"), "caret at the line start: {msg}");
    assert!(msg.contains("\"\\u{3000}\""), "visible token: {msg}");
}

#[test]
fn parse_error_with_multibyte_text_before_the_error_has_the_caret_under_it() {
    let msg = parse_err("  class A { x = \"\u{e9}\u{3000}\"; ? };\n");
    let lines: Vec<&str> = msg.lines().collect();
    let text = lines
        .iter()
        .find(|l| l.contains("class A"))
        .expect("text line");
    let caret = lines
        .iter()
        .find(|l| l.trim_start() == "^")
        .expect("caret line");
    let bad = text.chars().position(|c| c == '?').expect("? in text");
    assert_eq!(caret.chars().position(|c| c == '^'), Some(bad), "{msg}");
}

#[test]
fn preprocess_errors_with_multibyte_indent_have_the_caret_in_the_right_place() {
    for input in [
        "\u{a0}\"abc\n",
        "  \u{a0}\"abc\n",
        "\u{3000}\u{3000}\"abc\n",
    ] {
        let mut w = Warnings::default();
        let err = dayzmake_core::preprocess::preprocess(input.to_string(), None, &[], &mut w).err();
        let msg = err.map_or_else(String::new, |e| e.to_string());
        assert!(msg.contains("Unexpected token"), "{input:?}: {msg}");
        let lines: Vec<&str> = msg.lines().collect();
        let text = lines.iter().find(|l| l.contains("abc")).expect("text line");
        let caret = lines
            .iter()
            .find(|l| l.trim_start() == "^")
            .expect("caret line");
        let caret_at = caret.chars().position(|c| c == '^').expect("caret");
        let shown = text.chars().position(|c| c == '"').expect("quote");
        assert!(
            caret_at == shown + 4,
            "{input:?}: caret {caret_at}, quote {shown}\n{msg}"
        );
    }
}

#[test]
fn parse_error_with_nbsp_indent_and_multibyte_letter_has_the_caret_in_column_one() {
    // The NBSP is two bytes but one character: the old byte arithmetic subtracted 2 from
    // column 1 and underflowed.
    let msg = parse_err("\u{a0}\u{e9}x = 1;\n");
    let lines: Vec<&str> = msg.lines().collect();
    let text = lines
        .iter()
        .position(|l| l.contains('\u{e9}'))
        .expect("text line");
    assert_eq!(lines[text], "  \u{e9}x = 1;", "{msg}");
    assert_eq!(lines[text + 1], "  ^", "{msg}");
    assert!(msg.contains("\"\\u{a0}\""), "visible token: {msg}");
}

#[test]
fn parse_error_on_a_line_of_only_invisible_spaces_does_not_panic() {
    let msg = parse_err("class A {\n\u{a0}\n= 1; };");
    assert!(msg.contains("In line 2:"), "{msg}");
    assert!(
        msg.contains("\n  \n  ^\n"),
        "empty text line, caret at start: {msg}"
    );
    assert!(msg.contains("\"\\u{a0}\""), "visible token: {msg}");
    let msg = parse_err("class A {\n\u{3000}\n= 1; };");
    assert!(msg.contains("\"\\u{3000}\""), "visible token: {msg}");
}

#[test]
fn parse_error_shows_tabs_in_the_line_unescaped() {
    let msg = parse_err("class A {\n\tx\t= 1; ?\n};");
    assert!(msg.contains("x\t= 1; ?"), "real tab in the line: {msg}");
    assert!(!msg.contains("\\t"), "no escaped tab: {msg}");
}

#[test]
fn empty_value_raises_a_warning() {
    let mut w = Warnings::default();
    let config = Config::from_string("class A { x = ; };".to_string(), None, &[], &mut w).unwrap();
    let warnings = w.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let shown = warnings[0].to_string();
    assert!(shown.contains("empty-value"), "{shown}");
    assert!(shown.contains("\"x\""), "{shown}");
    assert!(shown.contains("empty string"), "{shown}");
    assert_eq!(warnings[0].line, Some(1));
    assert_eq!(warnings[0].name, Some("empty-value"));
    assert!(matches!(
        config.root().get("A"),
        Some(ConfigEntry::ClassEntry(c)) if matches!(c.get("x"), Some(ConfigEntry::StringEntry(v)) if v.is_empty())
    ));
}

#[test]
fn empty_value_warning_points_at_the_entry_line() {
    let mut w = Warnings::default();
    Config::from_string("class A {\n  n =\n;\n};".to_string(), None, &[], &mut w).unwrap();
    assert_eq!(w.warnings()[0].line, Some(2));
}
