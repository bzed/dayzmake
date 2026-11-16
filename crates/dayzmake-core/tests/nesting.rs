//! #54: deeply nested input is a parse error, never a stack overflow. Every case runs on a 1 MiB
//! stack (the Windows main-thread size) in a debug build.

#![expect(
    clippy::unwrap_used,
    reason = "tests fail by panicking; the helpers are not #[test] functions"
)]

use dayzmake_core::config::Config;
use dayzmake_core::preprocess::preprocess;
use dayzmake_core::warning::Warnings;

/// Runs `f` on a thread with a 1 MiB stack; an overflow aborts the whole test process.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn config_error(input: String) -> String {
    on_small_stack(move || {
        Config::from_string(input, None, &[], &mut Warnings::new())
            .map(|_| ())
            .unwrap_err()
            .to_string()
    })
}

fn config_ok(input: String) {
    on_small_stack(move || {
        let config = Config::from_string(input, None, &[], &mut Warnings::new()).unwrap();
        // writing and dropping recurse over the same depth
        config.to_cursor().unwrap();
        config.to_string().unwrap();
    });
}

fn nested_classes(depth: usize) -> String {
    let mut s = String::new();
    for _ in 0..depth {
        s.push_str("class A {\n");
    }
    for _ in 0..depth {
        s.push_str("};\n");
    }
    s
}

fn macro_call(depth: usize) -> String {
    format!(
        "#define F(x) x\na = F({}{});\n",
        "(".repeat(depth),
        ")".repeat(depth)
    )
}

#[test]
fn macro_argument_parentheses_are_limited() {
    let input = macro_call(200_000);
    let err = on_small_stack(move || {
        preprocess(input, None, &[], &mut Warnings::new())
            .unwrap_err()
            .to_string()
    });
    assert!(err.contains("deeper than 128"), "{err}");
}

#[test]
fn macro_argument_parentheses_below_the_limit_work() {
    let input = macro_call(127);
    let out = on_small_stack(move || {
        preprocess(input, None, &[], &mut Warnings::new())
            .unwrap()
            .0
    });
    assert!(out.contains(&"(".repeat(127)), "lost the parentheses");
}

/// Parentheses in a string are plain text for the macro argument rule's input, but a balanced
/// pair never nests: many sequential groups are fine.
#[test]
fn many_sequential_parenthesis_groups_work() {
    let input = format!("#define F(x) x\na = F({});\n", "()".repeat(5_000));
    let out = on_small_stack(move || {
        preprocess(input, None, &[], &mut Warnings::new())
            .unwrap()
            .0
    });
    assert!(out.contains("()()()"));
}

#[test]
fn class_nesting_is_limited() {
    let err = config_error(nested_classes(20_000));
    assert!(err.contains("deeper than 128"), "{err}");
}

#[test]
fn class_nesting_below_the_limit_works() {
    config_ok(nested_classes(128));
}

/// One level over the limit is an error, and what is allowed also reads back from its rapified form.
#[test]
fn class_nesting_limit_is_exact_and_rapified_roundtrip_works() {
    let err = config_error(nested_classes(129));
    assert!(err.contains("deeper than 128"), "{err}");
    on_small_stack(|| {
        let config =
            Config::from_string(nested_classes(128), None, &[], &mut Warnings::new()).unwrap();
        let bytes = config.to_cursor().unwrap().into_inner();
        Config::read_rapified(&mut std::io::Cursor::new(bytes)).unwrap();
    });
}

#[test]
fn array_nesting_limit_is_exact() {
    let err = config_error(format!("x[] = {}{};\n", "{".repeat(129), "}".repeat(129)));
    assert!(err.contains("deeper than 128"), "{err}");
}

#[test]
fn array_nesting_is_limited() {
    let input = format!("x[] = {}{};\n", "{".repeat(50_000), "}".repeat(50_000));
    let err = config_error(input);
    assert!(err.contains("deeper than 128"), "{err}");
}

#[test]
fn array_nesting_below_the_limit_works() {
    config_ok(format!("x[] = {}{};\n", "{".repeat(128), "}".repeat(128)));
}

#[test]
fn eval_parentheses_are_limited() {
    let input = format!(
        "x = __EVAL({}1{});\n",
        "(".repeat(20_000),
        ")".repeat(20_000)
    );
    let err = config_error(input);
    assert!(err.contains("deeper than 128"), "{err}");
}

#[test]
fn eval_parentheses_below_the_limit_work() {
    config_ok(format!(
        "x = __EVAL({}1{});\n",
        "(".repeat(127),
        ")".repeat(127)
    ));
}

/// `-` and `^` recurse in the evaluator without any parenthesis.
#[test]
fn eval_unary_and_power_chains_are_limited() {
    for body in [
        format!("{}1", "-".repeat(100_000)),
        vec!["1"; 100_000].join("^"),
    ] {
        let err = config_error(format!("x = __EVAL({body});\n"));
        assert!(err.contains("deeper than 128"), "{err}");
    }
}

/// Brackets inside strings and comments do not count as nesting.
#[test]
fn brackets_in_strings_and_comments_do_not_count() {
    let many = "{".repeat(2_000);
    config_ok(format!(
        "x = \"{many}{}\";\n// {many}\n/* {many} */\nclass A {{ z = 1; }};\n",
        "(".repeat(2_000)
    ));
}

#[test]
fn a_realistic_config_still_parses() {
    config_ok(
        "\
class CfgPatches {
    class Mod {
        units[] = {};
        requiredAddons[] = {\"DZ_Data\"};
        w[] = {{1, 2}, {3, {4, 5}}};
        v = __EVAL((1 + 2) * (3 - (4 / 2)));
    };
};
class CfgVehicles {
    class A { class B { class C { class D { x = 1; }; }; }; };
};
"
        .to_string(),
    );
}

/// The deepest class nesting holding the deepest `__EVAL` unary chain and a power chain still
/// fits a 1 MiB stack: the depths of the separate limits do not add up to an overflow.
#[test]
fn combined_class_and_eval_nesting_fits_a_small_stack() {
    let mut input = "class A {\n".repeat(127);
    input += &["x = __EVAL(", &"-".repeat(127), "1);\n"].concat();
    input += &["y = __EVAL(", &vec!["1"; 127].join("^"), ");\n"].concat();
    input.push_str(&"};\n".repeat(127));
    config_ok(input);
}
