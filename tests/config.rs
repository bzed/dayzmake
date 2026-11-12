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
