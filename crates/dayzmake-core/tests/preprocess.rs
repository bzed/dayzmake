//! Preprocessor tests.
use std::fs::{File, create_dir};
use std::io::Write;
use std::path::PathBuf;

use tempfile::tempdir;

use dayzmake_core::preprocess::*;
use dayzmake_core::warning::Warnings;

#[test]
fn test_preprocess_macros() {
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

    let (output, _) = preprocess(input, None, &Vec::new(), &mut Warnings::new()).unwrap();

    assert_eq!(
        "\
class CfgPatches {
class ace_frag{
units[] = { };
weapons[] = {};
requiredVersion = 1.56;
requiredAddons[] = {\"ace_common\"};
author[] = {\"Nou\"}   ;
version = \"3.5.0.0\" ;versionStr=\"3.5.0.0\";
versionAr [] = {3,5, 0, 0};
};
};",
        output.trim()
    );
}

#[test]
fn test_preprocess_ifdef() {
    let input = String::from(
        "\
#define foo bar
#define foobar whatever
#undef foobar

#ifdef foo
    #ifdef foobar
        def = 5678;
    #endif
    abc = 1234;
#else
    abc = 4321;
#endif
",
    );

    let (output, _) = preprocess(input, None, &Vec::new(), &mut Warnings::new()).unwrap();

    assert_eq!("abc = 1234;", output.trim());
}

#[test]
fn test_preprocess_include() {
    let input = String::from(
        "\
#include \"\\x\\cba\\addons\\whatever\\include.h\"
DOUBLES(foo,bar)\n",
    );

    let include = String::from(
        "\
#define DOUBLES(x,y) x##_##y
bar_foo\n",
    );

    let prefix = String::from(
        "\
\\x\\cba\\addons\\whatever\n",
    );

    let includedir = tempdir().unwrap();

    let addondir = includedir.path().join("whatever");
    create_dir(&addondir).unwrap();

    File::create(addondir.join("include.h"))
        .unwrap()
        .write_all(include.as_bytes())
        .unwrap();
    File::create(addondir.join("$PBOPREFIX$"))
        .unwrap()
        .write_all(prefix.as_bytes())
        .unwrap();

    let includepath = addondir.join("include.h").canonicalize().unwrap();

    let includefolders = vec![PathBuf::from(includedir.path())];
    let (output, info) = preprocess(
        input,
        Some(PathBuf::from("myfile")),
        &includefolders,
        &mut Warnings::new(),
    )
    .unwrap();

    assert_eq!("bar_foo\n\nfoo_bar", output.trim());
    assert_eq!((2, Some(includepath)), info.line_origins()[0]);
    assert_eq!((2, Some(PathBuf::from("myfile"))), info.line_origins()[2]);
}

#[test]
fn test_proprocess_bom() {
    let input = String::from_utf8(vec![0xef, 0xbb, 0xbf]).unwrap() + "blub";
    let (output, _) = preprocess(input, None, &Vec::new(), &mut Warnings::new()).unwrap();

    assert_eq!("blub", output.trim());
}

#[test]
fn test_preprocess_lineorigins() {
    let input = String::from(
        "\
#define TEST \"test\"/* foo

bar */
class test\\
{
    foo[] = {1,2,3, \\
    4};
    bar[] = {1,2,3, \\
    4}jashdlasd;
};\n",
    );

    let (_, info) = preprocess(input, None, &Vec::new(), &mut Warnings::new()).unwrap();
    assert_eq!(5, info.line_origins().len());
    assert_eq!(8, info.line_origins()[2].0);
}

#[test]
fn test_preprocess_directive_trailing_comments() {
    let dir = tempdir().unwrap();
    File::create(dir.path().join("inc.hpp"))
        .unwrap()
        .write_all(b"inc\n")
        .unwrap();
    File::create(dir.path().join("f.cpp")).unwrap();
    let input = String::from(
        "\
#include \"inc.hpp\" /* c */
#define Y 1   // c
#ifdef Y\t// c
y=Y;
#endif
",
    );
    let (output, _) = preprocess(
        input,
        Some(dir.path().join("f.cpp")),
        &[dir.path().to_path_buf()],
        &mut Warnings::new(),
    )
    .unwrap();
    assert!(output.contains("inc"), "{output:?}");
    assert!(output.contains("y=1"), "{output:?}"); // trailing spaces from the #define value are kept
}

#[test]
fn include_case_insensitive_absolute() {
    let dir = tempdir().unwrap();
    create_dir(dir.path().join("z")).unwrap();
    create_dir(dir.path().join("z/mod")).unwrap();
    File::create(dir.path().join("z/mod/foo.hpp"))
        .unwrap()
        .write_all(b"found\n")
        .unwrap();
    let input = String::from("#include \"\\z\\mod\\Foo.hpp\"\n");
    let (output, _) = preprocess(
        input,
        None,
        &[dir.path().to_path_buf()],
        &mut Warnings::new(),
    )
    .unwrap();
    assert!(output.contains("found"), "{output:?}");
}

#[test]
fn include_exact_case_wins() {
    let dir = tempdir().unwrap();
    File::create(dir.path().join("Inc.hpp"))
        .unwrap()
        .write_all(b"a=1;\n")
        .unwrap();
    File::create(dir.path().join("inc.hpp"))
        .unwrap()
        .write_all(b"a=2;\n")
        .unwrap();
    File::create(dir.path().join("f.cpp")).unwrap();
    let input = String::from("#include \"inc.hpp\"\n");
    let (output, _) = preprocess(
        input,
        Some(dir.path().join("f.cpp")),
        &[dir.path().to_path_buf()],
        &mut Warnings::new(),
    )
    .unwrap();
    assert!(output.contains("a=2;"), "{output:?}");
    assert!(!output.contains("a=1;"), "{output:?}");
}

/// Truncated and malformed input must not panic; preprocessing is lenient, so
/// any outcome without a panic is acceptable.
#[test]
fn truncated_input_does_not_panic() {
    let input = String::from("#define A 1\n#ifdef A\nx = A;\n#endif\ny = \"unterminated");
    for len in 0..=input.len() {
        let Some(prefix) = input.get(..len) else {
            continue;
        };
        let _ = preprocess(prefix.to_string(), None, &Vec::new(), &mut Warnings::new());
    }
}

/// An `#endif` without a matching `#ifdef`/`#ifndef` is an error, not a panic.
#[test]
fn endif_without_ifdef_is_an_error() {
    assert!(
        preprocess(
            String::from("#endif\n"),
            None,
            &Vec::new(),
            &mut Warnings::new()
        )
        .is_err()
    );
}

/// An include path from the file is untrusted: the error must not carry raw control characters.
#[test]
fn missing_include_with_control_characters_is_escaped() {
    let err = preprocess(
        String::from("#include \"a\u{1b}[31mb.hpp\"\n"),
        None,
        &Vec::new(),
        &mut Warnings::new(),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("a\\u{1b}[31mb.hpp"), "{msg:?}");
    assert!(!msg.contains('\u{1b}'), "{msg:?}");
}

/// Preprocessor parse errors report 1-based line numbers, like config errors.
#[test]
fn preprocess_error_line_numbers_are_one_based() {
    let err = |input: &str| {
        preprocess(
            input.to_string(),
            Some(PathBuf::from("in.cpp")),
            &Vec::new(),
            &mut Warnings::new(),
        )
        .unwrap_err()
        .to_string()
    };
    let first = err("#define A(\n");
    assert!(first.contains("in.cpp:1:"), "{first}");
    let third = err("a = 1;\nb = 2;\n#define A(\n");
    assert!(third.contains("in.cpp:3:"), "{third}");
}

/// A parse error inside an included file names that file and its own line.
#[test]
fn parse_error_inside_an_included_file_names_it() {
    let dir = tempdir().unwrap();
    File::create(dir.path().join("inc.hpp"))
        .unwrap()
        .write_all(b"a = 1;\nb = \"open;\n")
        .unwrap();
    File::create(dir.path().join("in.cpp")).unwrap();
    let msg = preprocess(
        "#include \"inc.hpp\"\n".to_string(),
        Some(dir.path().join("in.cpp")),
        &Vec::new(),
        &mut Warnings::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("inc.hpp:2:"), "{msg}");
}

/// #51: a parameter whose value holds a call of a macro that is on the expansion stack was re-expanded
/// without end (argument pre-expansion starts with an empty stack). Found by fuzzing; minimised input.
#[test]
fn macro_expansion_depth_is_limited() {
    let input = String::from("#define QUOTE(x) x\n#define TWO(a,b) \"\"TWO(,x)\nQUOTE(TWO(,))");

    let err = preprocess(input, None, &Vec::new(), &mut Warnings::new()).unwrap_err();

    let message = err.to_string();
    assert!(message.contains("expands too deep"), "{message}");
    assert!(
        message.contains("\"TWO\"") || message.contains("\"QUOTE\""),
        "{message}"
    );
}

fn run_file(path: &std::path::Path) -> std::io::Result<(String, PreprocessInfo)> {
    let input = std::fs::read_to_string(path)?;
    preprocess(
        input,
        Some(path.to_path_buf()),
        &[],
        &mut Warnings::default(),
    )
}

/// A file that includes itself is an error naming the chain, not a stack overflow.
#[test]
fn include_self_cycle_is_an_error() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("self.hpp");
    std::fs::write(&a, "#include \"self.hpp\"\n").unwrap();
    let err = run_file(&a).unwrap_err().to_string();
    assert!(err.contains("cycle"), "{err}");
    assert!(err.contains("self.hpp"), "{err}");
}

/// A -> B -> A, reached through `../` spellings of the same file.
#[test]
fn include_two_file_cycle_is_an_error() {
    let dir = tempdir().unwrap();
    create_dir(dir.path().join("sub")).unwrap();
    let a = dir.path().join("a.hpp");
    std::fs::write(&a, "#include \"sub\\b.hpp\"\n").unwrap();
    std::fs::write(
        dir.path().join("sub").join("b.hpp"),
        "#include \"..\\a.hpp\"\n",
    )
    .unwrap();
    let err = run_file(&a).unwrap_err().to_string();
    assert!(err.contains("cycle"), "{err}");
    assert!(err.contains("b.hpp"), "{err}");
}

fn chain(dir: &std::path::Path, files: usize) -> std::io::Result<PathBuf> {
    for i in 0..files {
        let body = if i + 1 < files {
            format!("#include \"f{}.hpp\"\n", i + 1)
        } else {
            "x = 1;\n".to_string()
        };
        std::fs::write(dir.join(format!("f{i}.hpp")), body)?;
    }
    Ok(dir.join("f0.hpp"))
}

/// 33 nested includes exceed the limit of 32.
#[test]
fn include_chain_over_the_limit_is_an_error() {
    let dir = tempdir().unwrap();
    // f0 is the root; f1..f33 are 33 nested includes.
    let root = chain(dir.path(), 34).unwrap();
    let err = run_file(&root).unwrap_err().to_string();
    assert!(err.contains("too deep"), "{err}");
    assert!(err.contains("32"), "{err}");
}

/// 32 nested includes are still fine.
#[test]
fn include_chain_at_the_limit_works() {
    let dir = tempdir().unwrap();
    let root = chain(dir.path(), 33).unwrap();
    let (out, _) = run_file(&root).unwrap();
    assert!(out.contains("x = 1;"));
}

/// A -> B -> D and A -> C -> D is not a cycle: D is included twice and its content appears twice.
/// Pins that the include stack is restored after each include (#53).
#[test]
fn diamond_include_is_not_a_cycle() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.hpp");
    std::fs::write(&a, "#include \"b.hpp\"\n#include \"c.hpp\"\n").unwrap();
    std::fs::write(dir.path().join("b.hpp"), "#include \"d.hpp\"\n").unwrap();
    std::fs::write(dir.path().join("c.hpp"), "#include \"d.hpp\"\n").unwrap();
    std::fs::write(dir.path().join("d.hpp"), "from_d = 1;\n").unwrap();
    let (out, _) = run_file(&a).unwrap();
    assert_eq!(out.matches("from_d = 1;").count(), 2, "{out}");
}

/// The billion-laughs pattern: every level doubles the output of the one below.
fn laughs(n: usize) -> String {
    let mut s = ["#define E", &n.to_string(), " x\n"].concat();
    for i in (0..n).rev() {
        let next = i + 1;
        s += &[
            "#define E",
            &i.to_string(),
            " E",
            &next.to_string(),
            " E",
            &next.to_string(),
            "\n",
        ]
        .concat();
    }
    s.push_str("a = E0;\n");
    s
}

/// #57: exponential expansion stops early. N=21 expands to about 2M tokens (a few hundred MB) even if the
/// cap regresses; the N=30 repro runs in a memory-limited child process (`tests/cli.rs`). with an error naming the macro.
#[test]
fn exponential_macro_expansion_is_capped() {
    let start = std::time::Instant::now();
    let err = preprocess(laughs(21), None, &[], &mut Warnings::new()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("expands to more than"), "{msg}");
    assert!(msg.contains("\"E"), "{msg}");
    assert!(
        start.elapsed().as_secs_f64() < 1.0,
        "took {:?}",
        start.elapsed()
    );
}

/// The same through macro arguments, which are copied rather than expanded.
#[test]
fn exponential_macro_arguments_are_capped() {
    let mut s = String::from("#define D(x) x x\n");
    s.push_str("a = ");
    s.push_str(&"D(".repeat(24));
    s.push('1');
    s.push_str(&")".repeat(24));
    s.push_str(";\n");
    let err = preprocess(s, None, &[], &mut Warnings::new()).unwrap_err();
    assert!(err.to_string().contains("expands to more than"), "{err}");
}

#[test]
fn moderate_doubling_still_expands() {
    let (out, _) = preprocess(laughs(12), None, &[], &mut Warnings::new()).unwrap();
    assert_eq!(out.matches('x').count(), 1 << 12);
}

/// CBA-style helper macros expand as before.
#[test]
fn cba_style_macros_still_expand() {
    let input = "\
#define PREFIX tag
#define COMPONENT main
#define DOUBLES(var1,var2) var1##_##var2
#define TRIPLES(var1,var2,var3) var1##_##var2##_##var3
#define ADDON DOUBLES(PREFIX,COMPONENT)
#define GVAR(var1) DOUBLES(ADDON,var1)
#define QUOTE(var1) #var1
#define QGVAR(var1) QUOTE(GVAR(var1))
#define QQGVAR(var1) QUOTE(QGVAR(var1))
#define PATHTOF(var1) \\x\\tag\\addons\\main\\var1
#define QPATHTOF(var1) QUOTE(PATHTOF(var1))
#define FUNC(var1) TRIPLES(ADDON,fnc,var1)
#define QFUNC(var1) QUOTE(FUNC(var1))
a = QGVAR(foo);
b = QQGVAR(foo);
c = QPATHTOF(data\\x.paa);
d = QFUNC(bar);
";
    let (out, _) = preprocess(input.to_string(), None, &[], &mut Warnings::new()).unwrap();
    assert!(out.contains("a = \"tag_main_foo\";"), "{out}");
    assert!(out.contains("b = \""), "{out}");
    assert!(out.contains("x\\tag\\addons\\main\\data\\x.paa"), "{out}");
    assert!(out.contains("d = \"tag_main_fnc_bar\";"), "{out}");
}
