//! Config parsing, rapification and derapification tests.
use std::io::{Cursor, Seek, SeekFrom};

use dayzmake::config::*;

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

    let mut config = Config::read(&mut cursor, None, &Vec::new()).unwrap();

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
    let err = Config::from_string(String::from("x = __EVAL(floor 2.7);\n"), None, &Vec::new())
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
    let result = Config::from_string(String::from("x = 0x1FFFFFFFF;\n"), None, &Vec::new());
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
    assert!(err.contains("too many entries"), "{err}");
}

/// Found by fuzzing: a class body offset that points back into itself recursed until the
/// stack overflowed. Reading must fail with an error instead.
#[test]
fn read_rapified_rejects_cyclic_class_bodies() {
    let data = include_bytes!("fixtures/fuzz/config_class_cycle.bin");
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
