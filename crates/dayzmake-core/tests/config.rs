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

/// An enum value that does not fit an i32 is an error, not a panic (entries and array elements take
/// 64-bit and wide hex values like `CfgConvert`).
#[test]
fn integer_overflow_is_an_error() {
    let result = Config::from_string(
        String::from("enum { X = 0x1FFFFFFFF };\n"),
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
    let Err(err) = read_bytes(bytes) else {
        panic!("shared bodies are rejected");
    };
    let err = err.to_string();
    assert!(err.contains("shared"), "{err}");
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

fn empty_value_line(src: &str) -> Option<u32> {
    let mut w = Warnings::default();
    Config::from_string(src.to_string(), None, &[], &mut w).ok()?;
    w.warnings()
        .iter()
        .find(|warning| warning.name == Some("empty-value"))
        .and_then(|warning| warning.line)
}

#[test]
fn line_numbers_survive_a_disabled_block_of_plain_lines() {
    // Warning is on line 4 of the source (#63).
    assert_eq!(
        empty_value_line("#ifdef NOPE\nz=1;\n#endif\nclass X { y = ; };"),
        Some(4)
    );
}

#[test]
fn line_numbers_survive_disabled_directives() {
    let src =
        "#ifdef NOPE\n#define A 1\n#undef B\n#include \"missing.hpp\"\n#endif\nclass X { y = ; };";
    assert_eq!(empty_value_line(src), Some(6));
}

#[test]
fn line_numbers_survive_nested_disabled_branches_with_else() {
    let src =
        "#ifdef NOPE\n#ifdef ALSO\na;\n#else\nb;\n#endif\nc;\n#else\nclass Z { w = ; };\n#endif\n";
    // enabled #else branch: the warning sits on line 9
    assert_eq!(empty_value_line(src), Some(9));
    let src = "#ifdef NOPE\n#ifdef ALSO\na;\n#else\nb;\n#endif\n#endif\nclass Z { w = ; };";
    assert_eq!(empty_value_line(src), Some(8));
}

#[test]
fn line_numbers_survive_multiline_comments_and_strings_in_disabled_branches() {
    let src = "#ifdef NOPE\n/* a\nb\nc */\nx = \"p\\\nq\";\n#define M 1 /* x\ny */\n#endif\nclass X { y = ; };";
    assert_eq!(empty_value_line(src), Some(10));
}

#[test]
fn error_location_survives_nested_disabled_branches() {
    let src = "#ifdef NOPE\n#ifndef OTHER\na;\n#else\nb;\n#endif\n#endif\nclass X { ;;; @@@ };";
    let err = Config::from_string(src.to_string(), None, &[], &mut Warnings::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("In line 8:"), "{err}");
}

#[test]
fn line_numbers_count_backslash_continuations_in_disabled_lines() {
    // The warning is on the last line of each source (#63).
    assert_eq!(
        empty_value_line("#ifdef NOPE\nx = 1 \\\n;\n#endif\nclass X { y = ; };\n"),
        Some(5)
    );
    assert_eq!(
        empty_value_line("#ifdef NOPE\nM(1,\\\n2);\n#endif\nclass X { y = ; };\n"),
        Some(5)
    );
    assert_eq!(
        empty_value_line("#ifdef NOPE\n#define M(a) \\\n  a \\\n  a\n#endif\nclass X { y = ; };\n"),
        Some(6)
    );
}

#[test]
fn line_numbers_count_backslash_continuations_in_an_enabled_define() {
    // #65
    assert_eq!(
        empty_value_line("#define M(a) \\\n  a \\\n  a\nclass X { y = ; };\n"),
        Some(4)
    );
}

#[test]
fn macro_uses_do_not_shift_line_numbers() {
    // #65: the continuation lines of a #define are counted once, at the definition.
    let one = "#define M(a) \\\n  a \\\n  a\nx = M(1);\nclass X { y = ; };\n";
    assert_eq!(empty_value_line(one), Some(5));
    let two = "#define M(a) \\\n  a \\\n  a\nx = M(1);\nx = M(1);\nclass X { y = ; };\n";
    assert_eq!(empty_value_line(two), Some(6));
    let nested = "#define N \\\n 1\n#define M(a) N a N\nx = M(1);\nclass X { y = ; };\n";
    assert_eq!(empty_value_line(nested), Some(5));
    let comment = "#define M 1 /* x\ny */\nx = M;\nclass X { y = ; };\n";
    assert_eq!(empty_value_line(comment), Some(4));
    let string = "#define M \"a\\\nb\"\nx = M;\nclass X { y = ; };\n";
    assert_eq!(empty_value_line(string), Some(4));
}

#[test]
fn from_string_in_matches_from_string() {
    let folders = dayzmake_core::preprocess::IncludeFolders::new(Vec::new());
    let mut w = Warnings::default();
    let config =
        Config::from_string_in("class A { x = 1; };".to_string(), None, &folders, &mut w).unwrap();
    assert!(config.root().get("A").is_some());
}

#[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
fn write_with(bytes: Vec<u8>, indent: &str) -> String {
    let config = read_bytes(bytes).unwrap();
    let mut out = Vec::new();
    config
        .write_with(&mut out, &WriteOptions::new().indent(indent).unwrap())
        .unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn write_with_uses_the_given_indent() {
    let bytes = rapify_src("class A { class B { x = 1; }; y = 2; };");
    let tab = write_with(bytes.clone(), "\t");
    assert!(tab.contains("\n\ty = 2;"), "{tab}");
    assert!(tab.contains("\n\t\tx = 1;"), "{tab}");
    let two = write_with(bytes, "  ");
    assert!(two.contains("\n  y = 2;"), "{two}");
    assert!(two.contains("\n    x = 1;"), "{two}");
    assert!(two.contains("\n  };"), "{two}");
}

#[test]
fn write_with_empty_indent_round_trips() {
    let bytes = rapify_src("class A { class B { x = 1; }; y[] = {1,2}; s = \"t\"; };");
    let text = write_with(bytes.clone(), "");
    assert!(text.contains("\nx = 1;"), "{text}");
    let again = Config::from_string(text, None, &[], &mut Warnings::new())
        .unwrap()
        .to_cursor()
        .unwrap()
        .into_inner()
        .into_vec();
    assert_eq!(bytes, again);
}

#[test]
fn indent_rejects_other_characters() {
    for bad in ["x", "\n", " \n", "  //", &" ".repeat(65)] {
        let err = WriteOptions::new().indent(bad).err();
        assert!(err.is_some(), "{bad:?} must be rejected");
    }
    let msg = WriteOptions::new().indent("a\nb").unwrap_err().to_string();
    assert!(msg.contains("a\\nb"), "{msg}");
    assert!(WriteOptions::new().indent("\t ").is_ok());
}

#[test]
fn indent_too_long_reports_the_length() {
    let msg = dayzmake_core::config::WriteOptions::new()
        .indent(" ".repeat(65))
        .expect_err("too long")
        .to_string();
    assert!(msg.contains("65 characters"), "{msg}");
    assert!(msg.len() < 120, "{msg}");
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn parse(src: &str) -> Config {
    Config::from_string(src.to_string(), None, &Vec::new(), &mut Warnings::new()).unwrap()
}

/// Returns the bytes of the rapified config and the enum table (from the header's `enum_offset`).
#[expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "test helper")]
fn table_bytes(config: &Config) -> Vec<u8> {
    let bytes = config.to_cursor().unwrap().into_inner().into_vec();
    let off = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    bytes[off..].to_vec()
}

#[test]
fn rapify_enum_block() {
    let with = parse("enum { A, B = 5, C };\nclass X { y = 1; };");
    let without = parse("class X { y = 1; };");
    assert_eq!(
        table_bytes(&with),
        b"\x03\0\0\0a\0\0\0\0\0b\0\x05\0\0\0c\0\x06\0\0\0"
    );
    assert_eq!(table_bytes(&without), b"\0\0\0\0");
    let a = with.to_cursor().unwrap().into_inner();
    let b = without.to_cursor().unwrap().into_inner();
    // the class tree (bytes 16..enum_offset) is identical
    let ea = u32::from_le_bytes(a[12..16].try_into().unwrap()) as usize;
    assert_eq!(&a[16..ea], &b[16..b.len() - 4]);
    assert_eq!(
        with.enum_values(),
        &[
            ("a".to_string(), 0),
            ("b".to_string(), 5),
            ("c".to_string(), 6)
        ]
    );
}

#[test]
fn enum_in_class_and_trailing_comma() {
    let c = parse("class X { enum { Foo = -2, Bar, }; v = 1; };\nenum { Z = 0x10 };");
    assert_eq!(
        c.enum_values(),
        &[
            ("foo".to_string(), -2),
            ("bar".to_string(), -1),
            ("z".to_string(), 16)
        ]
    );
    assert_eq!(c.to_string().unwrap().lines().next(), Some("enum {"));
    assert!(c.root().get("X").is_some());
}

fn bin_with_table(table: &[u8]) -> Vec<u8> {
    let mut v = b"\0raP\0\0\0\0\x08\0\0\0".to_vec();
    v.extend_from_slice(&21u32.to_le_bytes());
    v.extend_from_slice(b"\0\0"); // root: parent "", 0 entries
    v.extend_from_slice(&[0; 3]); // padding to offset 21
    v.extend_from_slice(table);
    v
}

#[test]
fn derapify_enum_table() {
    let data = bin_with_table(b"\x02\0\0\0destructno\0\0\0\0\0destructwreck\0\x07\0\0\0");
    let c = Config::read_rapified(&mut Cursor::new(data)).unwrap();
    assert_eq!(
        c.to_string().unwrap(),
        "enum {\n    destructno = 0,\n    destructwreck = 7\n};\n"
    );
}

#[test]
fn enum_round_trip() {
    let c = parse("enum { A, B = 5, C };\nclass X { y = 1; };");
    let text = c.to_string().unwrap();
    let again = parse(&text);
    assert_eq!(again.to_string().unwrap(), text);
    let mut cur = c.to_cursor().unwrap();
    cur.seek(SeekFrom::Start(0)).unwrap();
    let back = Config::read_rapified(&mut cur).unwrap();
    assert_eq!(back.to_string().unwrap(), text);
}

#[test]
fn damaged_enum_table_is_no_error() {
    // offset past the end
    let mut data = bin_with_table(b"");
    data[12..16].copy_from_slice(&9999u32.to_le_bytes());
    let c = Config::read_rapified(&mut Cursor::new(data)).unwrap();
    assert_eq!(c.enum_values(), []);
    // table cut short: second entry has no value
    let data = bin_with_table(b"\x02\0\0\0a\0\x01\0\0\0b\0\x02");
    let c = Config::read_rapified(&mut Cursor::new(data)).unwrap();
    assert_eq!(c.enum_values(), &[("a".to_string(), 1)]);
    // hostile count, no data
    let data = bin_with_table(b"\xff\xff\xff\xff");
    let c = Config::read_rapified(&mut Cursor::new(data)).unwrap();
    assert_eq!(c.enum_values(), []);
}

#[test]
fn enum_table_ends_at_the_first_name_that_is_no_identifier() {
    for bad in [&b"\x01"[..], b"a b", b"a-b", b"a\"b", b"", b"\xff\xfe"] {
        let mut table = b"\x03\0\0\0ok\0\x05\0\0\0".to_vec();
        table.extend_from_slice(bad);
        table.extend_from_slice(b"\0\x01\0\0\0fine\0\x02\0\0\0");
        let c = Config::read_rapified(&mut Cursor::new(bin_with_table(&table))).unwrap();
        assert_eq!(c.enum_values(), &[("ok".to_string(), 5)], "{bad:?}");
        assert_eq!(c.to_string().unwrap(), "enum {\n    ok = 5\n};\n");
    }
    // a bad first name leaves no block at all
    let data = bin_with_table(b"\x01\0\0\0\x01\0\x01\0\0\0");
    let c = Config::read_rapified(&mut Cursor::new(data)).unwrap();
    assert_eq!(c.to_string().unwrap(), "");
}

#[test]
fn enum_max_value_is_accepted_but_an_implicit_overflow_is_not() {
    let c = parse("enum { A = 2147483647 };");
    assert_eq!(c.enum_values(), &[("a".to_string(), i32::MAX)]);
    let c = parse("enum { A = 0x7FFFFFFF };");
    assert_eq!(c.enum_values(), &[("a".to_string(), i32::MAX)]);
    assert!(
        Config::from_string(
            "enum { A = 2147483647, B };".to_string(),
            None,
            &Vec::new(),
            &mut Warnings::new()
        )
        .is_err()
    );
}

#[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
fn parse_a(src: &str) -> (Config, Warnings) {
    let mut w = Warnings::default();
    let config = Config::from_string(src.to_string(), None, &[], &mut w).unwrap();
    (config, w)
}

#[expect(clippy::unwrap_used, clippy::panic, reason = "tests fail by panicking")]
fn entry_a<'a>(config: &'a Config, name: &str) -> &'a ConfigEntry {
    let Some(ConfigEntry::ClassEntry(c)) = config.root().get("A") else {
        panic!("no class A");
    };
    c.get(name).unwrap()
}

#[test]
fn exponent_floats_are_floats() {
    let src = "class A { a = 1e-5; b = 1.5E+3; c = -2.5e-7; d = 3.162277e-5; e = 7; f = 1.0; g = 0x10; };";
    let (config, w) = parse_a(src);
    assert!(w.warnings().is_empty(), "{:?}", w.warnings());
    for (n, v) in [
        ("a", 1e-5_f32),
        ("b", 1500.0),
        ("c", -2.5e-7),
        ("d", 3.162_277e-5),
    ] {
        assert!(
            matches!(entry_a(&config, n), ConfigEntry::FloatEntry(f) if f.to_bits() == v.to_bits()),
            "{n}"
        );
    }
    assert!(matches!(entry_a(&config, "e"), ConfigEntry::IntEntry(7)));
    assert!(matches!(entry_a(&config, "f"), ConfigEntry::FloatEntry(_)));
    assert!(matches!(entry_a(&config, "g"), ConfigEntry::IntEntry(16)));
}

#[test]
fn exponent_floats_in_arrays_are_floats() {
    let (config, w) = parse_a("class A { x[] = {1e-5, 1.5E+3, -2.5e-7, 3.162277e-5, 4}; };");
    assert!(w.warnings().is_empty(), "{:?}", w.warnings());
    let ConfigEntry::ArrayEntry(a) = entry_a(&config, "x") else {
        panic!("not an array");
    };
    let kinds: Vec<bool> = a
        .elements()
        .iter()
        .map(|e| matches!(e, ConfigArrayElement::FloatElement(_)))
        .collect();
    assert_eq!(kinds, [true, true, true, true, false]);
}

#[test]
fn malformed_exponents_stay_strings() {
    let (config, w) = parse_a("class A { a = 1e5x; b = e5; c = 1e; d[] = {1e5x, e5, 1e}; };");
    assert!(
        w.warnings()
            .iter()
            .any(|x| x.name == Some("unquoted-string")),
        "{:?}",
        w.warnings()
    );
    for n in ["a", "b", "c"] {
        assert!(
            matches!(entry_a(&config, n), ConfigEntry::StringEntry(_)),
            "{n}"
        );
    }
    let ConfigEntry::ArrayEntry(a) = entry_a(&config, "d") else {
        panic!("not an array");
    };
    assert!(
        a.elements()
            .iter()
            .all(|e| matches!(e, ConfigArrayElement::StringElement(_)))
    );
}

#[test]
fn derapified_exponent_float_roundtrips() {
    let first = derapify(rapify_src("class A { x = 0.00003162277; };"));
    let second = derapify(rapify_src(&first));
    assert_eq!(first, second);
}

/// `CfgConvert` writes an f32 infinity for a literal that overflows (`float_ovf` fixtures).
#[test]
fn overflowing_floats_become_infinity() {
    let long = format!("1{}.5", "0".repeat(45));
    let cases = [
        ("1e999", f32::INFINITY),
        ("-1e999", f32::NEG_INFINITY),
        ("1e39", f32::INFINITY),
        ("-1e39", f32::NEG_INFINITY),
        ("3.5e38", f32::INFINITY),
        (long.as_str(), f32::INFINITY),
    ];
    for (v, want) in cases {
        let (config, w) = parse_a(&format!("class A {{ x = {v}; y[] = {{1, {v}}}; }};"));
        assert!(w.warnings().is_empty(), "{v}: {:?}", w.warnings());
        assert!(
            matches!(entry_a(&config, "x"), ConfigEntry::FloatEntry(f) if f.to_bits() == want.to_bits()),
            "{v}"
        );
        let ConfigEntry::ArrayEntry(a) = entry_a(&config, "y") else {
            panic!("not an array");
        };
        assert!(
            matches!(a.elements(), [_, ConfigArrayElement::FloatElement(f)] if f.to_bits() == want.to_bits()),
            "{v}"
        );
    }
}

/// Derapify writes infinity as `1e999` / `-1e999`, which parses back to the same f32.
#[test]
fn infinite_floats_roundtrip_through_derapify() {
    let bytes =
        rapify_src("class A { a = 1e999; b = -1e39; c[] = {1e999, -1e999, 1.5}; d = -1e-999; };");
    let text = derapify(bytes.clone());
    assert!(text.contains("a = 1e999;"), "{text}");
    assert!(text.contains("b = -1e999;"), "{text}");
    assert!(text.contains("c[] = {1e999, -1e999, 1.5};"), "{text}");
    assert_eq!(rapify_src(&text), bytes);
}

#[test]
fn float_underflow_is_zero_without_warning() {
    let (config, w) = parse_a("class A { a = 1e-50; b[] = {1e-50}; };");
    assert!(w.warnings().is_empty(), "{:?}", w.warnings());
    assert!(matches!(entry_a(&config, "a"), ConfigEntry::FloatEntry(f) if f.to_bits() == 0));
    let ConfigEntry::ArrayEntry(a) = entry_a(&config, "b") else {
        panic!("not an array");
    };
    assert!(matches!(a.elements(), [ConfigArrayElement::FloatElement(f)] if f.to_bits() == 0));
}

#[test]
fn hex_and_leading_dot_exponent() {
    let (config, w) = parse_a("class A { a = 0x1e5; b = .5e3; c[] = {0x1e5, .5e3}; };");
    assert!(w.warnings().is_empty(), "{:?}", w.warnings());
    assert!(matches!(entry_a(&config, "a"), ConfigEntry::IntEntry(485)));
    assert!(
        matches!(entry_a(&config, "b"), ConfigEntry::FloatEntry(f) if f.to_bits() == 500.0_f32.to_bits())
    );
    let ConfigEntry::ArrayEntry(a) = entry_a(&config, "c") else {
        panic!("not an array");
    };
    assert!(matches!(
        a.elements(),
        [ConfigArrayElement::IntElement(485), ConfigArrayElement::FloatElement(f)] if f.to_bits() == 500.0_f32.to_bits()
    ));
}

#[test]
fn overflowing_float_followed_by_text_stays_a_string() {
    let (config, w) = parse_a("class A { a = 1e999x; b[] = {1e999x}; };");
    assert!(
        w.warnings()
            .iter()
            .any(|x| x.name == Some("unquoted-string")),
        "{:?}",
        w.warnings()
    );
    assert!(matches!(entry_a(&config, "a"), ConfigEntry::StringEntry(_)));
    let ConfigEntry::ArrayEntry(a) = entry_a(&config, "b") else {
        panic!("not an array");
    };
    assert!(matches!(
        a.elements(),
        [ConfigArrayElement::StringElement(_)]
    ));
}

#[test]
fn overflowing_integer_followed_by_text_stays_a_string() {
    let (config, w) = parse_a("class A { a = 99999999999x; b[] = {99999999999x, 0xfffffffffy}; };");
    assert_eq!(
        w.warnings()
            .iter()
            .filter(|x| x.name == Some("unquoted-string"))
            .count(),
        3,
        "{:?}",
        w.warnings()
    );
    assert!(matches!(entry_a(&config, "a"), ConfigEntry::StringEntry(s) if s == "99999999999x"));
    let ConfigEntry::ArrayEntry(a) = entry_a(&config, "b") else {
        panic!("not an array");
    };
    assert!(matches!(
        a.elements(),
        [
            ConfigArrayElement::StringElement(_),
            ConfigArrayElement::StringElement(_)
        ]
    ));
}

#[test]
fn an_integer_beyond_32_bits_alone_is_a_64_bit_entry_or_an_array_float() {
    assert_eq!(
        text_of("class A { a = 99999999999; };"),
        "class A {\n    a = 99999999999;\n};\n"
    );
    // in an array `CfgConvert` makes it a float (`overflowing_integer_literal_is_a_float_like_cfgconvert`)
    assert_eq!(
        text_of("class A { a[] = {99999999999}; };"),
        "class A {\n    a[] = {100000000000.0};\n};\n"
    );
}

#[test]
fn inf_and_nan_are_unquoted_strings_with_a_warning() {
    let (config, w) = parse_a("class A { a = inf; b = nan; };");
    assert_eq!(
        w.warnings()
            .iter()
            .filter(|x| x.name == Some("unquoted-string"))
            .count(),
        2,
        "{:?}",
        w.warnings()
    );
    assert!(matches!(entry_a(&config, "a"), ConfigEntry::StringEntry(s) if s == "inf"));
    assert!(matches!(entry_a(&config, "b"), ConfigEntry::StringEntry(s) if s == "nan"));
}

#[test]
fn floats_ending_at_a_closing_brace_stay_floats() {
    let (config, w) = parse_a("class A { x = 1.5 };");
    assert!(w.warnings().is_empty(), "{:?}", w.warnings());
    assert!(
        matches!(entry_a(&config, "x"), ConfigEntry::FloatEntry(f) if f.to_bits() == 1.5_f32.to_bits())
    );
    let (config, w) = parse_a("class A {\n x = 1e5\n};");
    assert!(w.warnings().is_empty(), "{:?}", w.warnings());
    assert!(
        matches!(entry_a(&config, "x"), ConfigEntry::FloatEntry(f) if f.to_bits() == 1e5_f32.to_bits())
    );
    let (config, _) = parse_a("class A { x = 1e999 };");
    assert!(
        matches!(entry_a(&config, "x"), ConfigEntry::FloatEntry(f) if f.to_bits() == f32::INFINITY.to_bits())
    );
}

#[test]
fn array_floats_accept_whitespace_before_terminators() {
    let (config, w) = parse_a("class A { x[] = {1.5 , 2e1 , 1e-5 }; };");
    assert!(w.warnings().is_empty(), "{:?}", w.warnings());
    let ConfigEntry::ArrayEntry(a) = entry_a(&config, "x") else {
        panic!("not an array");
    };
    assert!(
        a.elements()
            .iter()
            .all(|e| matches!(e, ConfigArrayElement::FloatElement(_)))
    );
    assert_eq!(a.elements().len(), 3);
}

fn read_err(src: &str, path: &str) -> String {
    Config::from_string(
        src.to_string(),
        Some(std::path::PathBuf::from(path)),
        &[],
        &mut Warnings::new(),
    )
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default()
}

#[test]
fn grammar_action_errors_carry_file_and_line() {
    let cases = [
        (
            "class A {\n  y = 1;\n  enum { X = 2147483648 };\n};",
            "3",
            "Invalid integer",
        ),
        (
            "class A {\n  enum { X = 2147483648 };\n};",
            "2",
            "Invalid integer",
        ),
        (
            "class A {\n  y = 1;\n  z = 2;\n  x = __EVAL(1/0);\n};",
            "4",
            "division by zero",
        ),
        // CRLF line ends count the same
        (
            "class A {\r\n  y = 1;\r\n  enum { X = 2147483648 };\r\n};",
            "3",
            "Invalid integer",
        ),
    ];
    for (src, line, what) in cases {
        let err = read_err(src, "dir/cfg.cpp");
        assert!(
            err.contains(&format!("In line dir/cfg.cpp:{line}:")),
            "{src:?}: {err:?}"
        );
        assert!(err.contains(what), "{src:?}: {err:?}");
    }
    let deep = format!(
        "class A {{\n  x[] = {}1{};\n}};",
        "{".repeat(200),
        "}".repeat(200)
    );
    let more = [
        (deep.as_str(), "2", "Nesting is deeper"),
        ("class A {\n  y = 1;\n  __EXEC(foo);\n};", "3", "__EXEC"),
    ];
    for (src, line, what) in more {
        let err = read_err(src, "dir/cfg.cpp");
        assert!(
            err.contains(&format!("In line dir/cfg.cpp:{line}:\n\n  ")),
            "{err:?}"
        );
        assert!(err.contains(what), "{err:?}");
    }
    // the same prefix as a PEG parse error in the same file
    let peg = read_err("class A {\n  y = 1;\n  x = ;;\n};", "dir/cfg.cpp");
    assert!(peg.starts_with("In line dir/cfg.cpp:3:\n\n  "), "{peg:?}");
    for (src, line) in [(cases[0].0, 3), (more[1].0, 3)] {
        let err = read_err(src, "dir/cfg.cpp");
        assert!(
            err.starts_with(&format!("In line dir/cfg.cpp:{line}:\n\n  ")),
            "{err:?}"
        );
    }
}

/// The config as text.
#[expect(clippy::unwrap_used, reason = "test helper")]
fn text_of(src: &str) -> String {
    parse(src).to_string().unwrap()
}

/// A last entry that ends at a line end instead of `;` keeps its value type, as in `CfgConvert`
/// (`tests/fixtures/bi/cfgconvert/last_*.cpp`). The same with CRLF, and inside a class.
#[test]
fn last_entry_without_semicolon_keeps_its_type() {
    assert_eq!(text_of("x = 1.5\n"), "x = 1.5;\n");
    assert_eq!(text_of("x = 1.5\r\n"), "x = 1.5;\n");
    assert_eq!(text_of("x = 1e5\n"), "x = 100000.0;\n");
    assert_eq!(text_of("x = 7\n"), "x = 7;\n");
    assert_eq!(text_of("x = \"s\"\n"), "x = \"s\";\n");
    assert_eq!(text_of("x = 1.5  \t\n"), "x = 1.5;\n");
    assert_eq!(text_of("x = abc\n"), "x = \"abc\";\n");
    assert_eq!(text_of("x = abc def\n"), "x = \"abc def\";\n");
    assert_eq!(text_of("x = 1 abc\n"), "x = \"1 abc\";\n");
    assert_eq!(
        text_of("class A { x = 1.5\n};"),
        "class A {\n    x = 1.5;\n};\n"
    );
}

/// A final `//` comment, or an unterminated `/*`, without a line end after it (#103); and a file
/// whose last line is a directive, with any line-end style (a lone CR ends a line for the preprocessor).
#[test]
fn comment_or_directive_on_the_last_line_without_line_end() {
    assert_eq!(text_of("x = 1\n// c"), "x = 1;\n");
    assert_eq!(text_of("x = 1;\n// c"), "x = 1;\n");
    assert_eq!(text_of("x = 1;\n/* c */"), "x = 1;\n");
    assert_eq!(text_of("x = 1;\n/* c"), "x = 1;\n");
    assert_eq!(text_of("x = 1\r\n// c"), "x = 1;\n");
    assert_eq!(text_of("x = 1\r#define Y 1"), "x = 1;\n");
    assert_eq!(text_of("x = 1\r// c"), "x = 1;\n");
}

/// A last entry without `;` keeps its integer type at the line end, LF and CRLF: 2147483648 is a 64-bit integer.
#[test]
fn last_entry_without_semicolon_is_a_64_bit_integer() {
    for src in ["y = 1;\nx = 2147483648\n", "y = 1;\r\nx = 2147483648\r\n"] {
        assert_eq!(text_of(src), "y = 1;\nx = 2147483648;\n", "{src:?}");
    }
}

/// `CfgConvert` rejects a last entry at the very end of the file with neither `;` nor line end
/// (`x = 1.5`, `x = abc`, `x = 1e5`); so does dayzmake, with the line.
#[test]
fn last_entry_at_end_of_file_without_terminator_is_an_error() {
    for src in [
        "y = 1;\nx = 1.5",
        "y = 1;\nx = abc",
        "y = 1;\nx = 1e5",
        "y = 1;\nx = 1e999",
    ] {
        let err = read_err(src, "cfg.cpp");
        assert!(
            err.starts_with("In line cfg.cpp:2:\n\n  "),
            "{src:?}: {err:?}"
        );
    }
}

/// A directive as the last line without a line end does not make the entry before it
/// unterminated: its own line end is in the source (`CfgConvert` accepts all of these).
#[test]
fn last_entry_before_a_final_directive_or_comment_keeps_its_type() {
    for src in [
        "#ifndef X\nx = 1\n#endif",
        "x = 1\n#define Y 1",
        "x = 1.5\n#define Y 1",
    ] {
        let text = text_of(src);
        assert!(text.starts_with("x = 1"), "{src:?}: {text:?}");
        assert!(!text.contains('"'), "{src:?}: {text:?}");
    }
}

/// Any entry, not only the last one, may end at the line end (`CfgConvert`, `last_mid.cpp`).
#[test]
fn entries_in_the_middle_may_end_at_the_line_end() {
    assert_eq!(
        text_of("x = 1.5\ny = \"s\"\nz = 2\nw = 3;\n"),
        "x = 1.5;\ny = \"s\";\nz = 2;\nw = 3;\n"
    );
    assert_eq!(text_of("x = __EVAL(1+1)\n"), "x = 2;\n");
}

/// The last source line decides whether the entry before the preprocessor's own line end is terminated,
/// not the raw text: a continued `#define` or an unterminated `/*` on the last lines do not make the
/// entry before them unterminated.
#[test]
fn last_entry_before_a_continued_define_or_open_comment_keeps_its_type() {
    for src in [
        "x = 1\n#define A \\\n 2",
        "x = 1\n#define A \\\r\n 2",
        "z = a /* c\ny = 2;",
        "x = 1\n/* c\ny = 2;",
        "x = 1.5\n#define A \\\n 2",
    ] {
        let text = text_of(src);
        assert!(!text.is_empty(), "{src:?}: {text:?}");
    }
    assert_eq!(text_of("x = 1\n#define A \\\n 2"), "x = 1;\n");
    assert_eq!(text_of("x = 1\n/* c\ny = 2;"), "x = 1;\n");
}

/// An integer literal that does not fit `i64` is a float in `CfgConvert` (`golden_int_ovf`), at a line end
/// and before `;`; in an array so is one that does not fit `i32`. One followed by text stays a string.
#[test]
fn overflowing_integer_literal_is_a_float_like_cfgconvert() {
    assert_eq!(
        text_of("x = 99999999999999999999\ny = 1;\n"),
        "x = 1e20;\ny = 1;\n"
    );
    assert_eq!(text_of("x = 99999999999999999999;"), "x = 1e20;\n");
    assert_eq!(text_of("x = -99999999999999999999;"), "x = -1e20;\n");
    assert_eq!(
        text_of("x[] = {99999999999999999999, 3000000000, 2147483647};"),
        "x[] = {1e20, 3000000000.0, 2147483647};\n"
    );
    assert_eq!(
        text_of("x = 99999999999999999999x;"),
        "x = \"99999999999999999999x\";\n"
    );
}

/// `x = 1 // c` and `x = 2 \` plus a comment line, both without a final newline: `CfgConvert` rejects them
/// (2026-11-17), so does dayzmake, naming the line.
#[test]
fn entry_followed_by_a_comment_at_the_end_of_the_file_is_an_error() {
    let err = read_err("x = 1 // c", "cfg.cpp");
    assert!(err.starts_with("In line cfg.cpp:1:\n\n  "), "{err:?}");
    let err = read_err("y = 1;\nx = 2 \\\n// c", "cfg.cpp");
    assert!(err.starts_with("In line cfg.cpp:2:\n\n  "), "{err:?}");
}

/// The last line of an included file without a line end is not an unterminated entry: `CfgConvert` accepts
/// `a = 1;\nb = 2` from an include followed by `z = 3;` (it glues its own `#line` marker to the value, so
/// its `b` is the string `2#line ...`, which dayzmake does not copy). This pins the lenient result.
#[test]
fn include_without_final_newline_is_accepted() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("inc.hpp"), "a = 1;\nb = 2").unwrap();
    let main = dir.path().join("m.cpp");
    std::fs::write(&main, "#include \"inc.hpp\"\nz = 3;\n").unwrap();
    let config = Config::read(
        &mut std::fs::File::open(&main).unwrap(),
        Some(main),
        &[],
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(config.to_string().unwrap(), "a = 1;\nb = 2;\nz = 3;\n");
}

fn unquoted_lines(src: &str) -> (bool, Vec<u32>) {
    let mut warnings = Warnings::new();
    let ok = Config::from_string(
        src.to_string(),
        Some(std::path::PathBuf::from("cfg.cpp")),
        &[],
        &mut warnings,
    )
    .is_ok();
    let lines = warnings
        .warnings()
        .iter()
        .filter(|w| w.name == Some("unquoted-string"))
        .filter_map(|w| w.line)
        .collect();
    (ok, lines)
}

/// A parse error does not bring the `unquoted-string` warning of the failed attempt on its own line
/// (`x = 1 // c` was tried as an unquoted string first), but keeps the real ones from earlier lines.
#[test]
fn parse_error_drops_stray_warnings_of_its_own_line_only() {
    assert_eq!(unquoted_lines("x = 1 // c"), (false, vec![]));
    assert_eq!(unquoted_lines("y = 1;\nx = 2 \\\n// c"), (false, vec![]));
    assert_eq!(
        unquoted_lines("a = abc;\nb = 2;\nc = 1 // c"),
        (false, vec![1])
    );
    assert_eq!(
        unquoted_lines("a = abc;\nb = 2;\nc = ;;\n"),
        (false, vec![1])
    );
    // without an error every warning stays
    assert_eq!(unquoted_lines("a = abc;\nb = def;\n"), (true, vec![1, 2]));
}

/// The warning filter of a failed parse is linear: many warning lines before one error finish at once
/// (it was quadratic, 20,000 lines took 47 s).
#[test]
fn parse_error_after_many_warnings_is_fast() {
    let mut src = "a = b;\n".repeat(20_000);
    src.push_str("c = ;;\n");
    let start = std::time::Instant::now();
    let (ok, lines) = unquoted_lines(&src);
    assert!(!ok);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    assert!(lines.len() <= 20_000);
}

/// An error in an included file drops the warning of its own line and keeps the real one of the parent's
/// earlier line; and the reverse: a warning in the include stays when the error is in the parent.
#[test]
fn parse_error_and_warnings_across_an_include() {
    let dir = tempfile::TempDir::new().unwrap();
    let run = |parent: &str, inc: &str| {
        std::fs::write(dir.path().join("inc.hpp"), inc).unwrap();
        let main = dir.path().join("m.cpp");
        std::fs::write(&main, parent).unwrap();
        let mut warnings = Warnings::new();
        let ok = Config::read(
            &mut std::fs::File::open(&main).unwrap(),
            Some(main),
            &[],
            &mut warnings,
        )
        .is_ok();
        let msgs: Vec<String> = warnings
            .warnings()
            .iter()
            .filter(|w| w.name == Some("unquoted-string"))
            .map(|w| w.message.clone())
            .collect();
        (ok, msgs)
    };
    // error in the include, real warning (abc) in the parent before it
    let (ok, msgs) = run("a = abc;\n#include \"inc.hpp\"\n", "x = 1 // c\ny = ;;\n");
    assert!(!ok);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs[0].contains("abc"), "{msgs:?}");
    // real warning in the include, error in the parent after it
    let (ok, msgs) = run("#include \"inc.hpp\"\nc = ;;\n", "a = def;\n");
    assert!(!ok);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs[0].contains("def"), "{msgs:?}");
}

/// The entry subtype byte of the first entry of a rapified config with a single top-level entry named
/// `x`: header (16 bytes), parent `\0`, count, entry type, subtype.
#[expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "test helper")]
fn first_subtype(config: &Config) -> u8 {
    config.to_cursor().unwrap().into_inner()[16 + 1 + 1 + 1]
}

/// A decimal literal that fits `i64` but not `i32` is a 64-bit integer entry (subtype 6), as in `CfgConvert`
/// (`golden_int64`); one that fits `i32` stays subtype 2, one beyond `i64` is a float (subtype 1).
#[test]
fn decimal_entries_pick_the_subtype_like_cfgconvert() {
    for (lit, subtype) in [
        ("2147483647", 2),
        ("-2147483648", 2),
        ("2147483648", 6),
        ("-2147483649", 6),
        ("+2147483648", 6),
        ("9223372036854775807", 6),
        ("-9223372036854775808", 6),
        ("9223372036854775808", 1),
        ("-9223372036854775809", 1),
    ] {
        let config = parse(&format!("x = {lit};"));
        assert_eq!(first_subtype(&config), subtype, "{lit}");
        // the same at a line end
        let config = parse(&format!("x = {lit}\n"));
        assert_eq!(first_subtype(&config), subtype, "{lit} at a line end");
    }
    let config = parse("x = 2147483648; y = -9223372036854775808;");
    let ConfigEntry::Int64Entry(x) = entry_top(&config, "x") else {
        panic!("not an Int64Entry");
    };
    assert_eq!(*x, 2_147_483_648);
    assert!(matches!(
        entry_top(&config, "y"),
        ConfigEntry::Int64Entry(i64::MIN)
    ));
    assert_eq!(
        config.to_string().unwrap(),
        "x = 2147483648;\ny = -9223372036854775808;\n"
    );
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn entry_top<'a>(config: &'a Config, name: &str) -> &'a ConfigEntry {
    config.root().get(name).unwrap()
}

/// Derapify writes a literal that reads back to the same subtype: text, rapify, derapify, rapify is
/// bit for bit the same.
#[test]
fn int64_entries_round_trip_bit_for_bit() {
    let first = parse("a = 2147483648; b = -2147483649; c = 9223372036854775807; d = 1;\n");
    let bytes = first.to_cursor().unwrap().into_inner().into_vec();
    let decoded = Config::read_rapified(&mut Cursor::new(bytes.clone())).unwrap();
    assert!(matches!(
        entry_top(&decoded, "a"),
        ConfigEntry::Int64Entry(2_147_483_648)
    ));
    let again = parse(&decoded.to_string().unwrap());
    assert_eq!(again.to_cursor().unwrap().into_inner().into_vec(), bytes);
}

/// A hex literal without a sign keeps its low 32 bits (`0x1FFFFFFFFF` is `-1`, `0x100000000` is `0`), at any
/// length and with `0X`; with a sign it is a float. All as `CfgConvert` writes it (`golden_hex_*`).
#[test]
fn hex_literals_follow_cfgconvert() {
    assert_eq!(
        text_of(
            "a = 0xFFFFFFFF; b = 0x100000000; c = 0x1FFFFFFFFF; d = 0x1FFFFFFFFFFFFFFFF; e = 0X10; f = 0x0000000000000000000001;"
        ),
        "a = -1;\nb = 0;\nc = -1;\nd = -1;\ne = 16;\nf = 1;\n"
    );
    assert_eq!(
        text_of("a = -0x10; b = +0x10; c = -0X10; d = -0x0;"),
        "a = -16.0;\nb = 16.0;\nc = -16.0;\nd = -0.0;\n"
    );
    assert_eq!(
        text_of("x[] = {0x1FFFFFFFFF, 0X10, -0x10, +0x10, 0x100000000};"),
        "x[] = {-1, 16, -16.0, 16.0, 0};\n"
    );
    assert_eq!(text_of("a = 0x;"), "a = \"0x\";\n");
}

/// Reading entry subtype 6, and rejecting a cut-short one with an error.
#[test]
fn rapified_int64_entry_is_read_and_a_short_one_is_an_error() {
    let bytes = parse("x = 4294967296;")
        .to_cursor()
        .unwrap()
        .into_inner()
        .into_vec();
    let decoded = Config::read_rapified(&mut Cursor::new(bytes.clone())).unwrap();
    assert!(matches!(
        entry_top(&decoded, "x"),
        ConfigEntry::Int64Entry(4_294_967_296)
    ));
    // name `x\0` then 8 value bytes: cut after 7 of them
    let end = 16 + 1 + 1 + 1 + 1 + 2 + 7;
    let err = Config::read_rapified(&mut Cursor::new(&bytes[..end])).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof, "{err}");
}

/// The committed fuzz seed with subtype 6 reads, and its cut-short copy is an error, not a panic.
#[test]
fn int64_fuzz_seeds() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/config_rapified");
    let whole = std::fs::read(dir.join("int64-entries.bin")).unwrap();
    let config = Config::read_rapified(&mut Cursor::new(whole)).unwrap();
    assert!(matches!(
        entry_top(&config, "d"),
        ConfigEntry::Int64Entry(i64::MAX)
    ));
    let cut = std::fs::read(dir.join("int64-truncated.bin")).unwrap();
    let err = Config::read_rapified(&mut Cursor::new(cut)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof, "{err}");
}
