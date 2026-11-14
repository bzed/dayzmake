//! Tests of the script API index model and the Enforce Script declaration parser.
#![expect(clippy::expect_used, reason = "a failing expect is a failing test")]
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dayzmake::api::model::{ApiIndex, Class, Enum, InputStamp, Member, MemberKind, Module};
use dayzmake::api::parse::{ScriptFile, active_defines, collect_defines, index_scripts};
use dayzmake::api::query::{
    Query, QueryResult, SrcFile, render_json, render_text, run_query, src_files,
};
use dayzmake::api::steam::Branch;

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/api")
        .join(rel)
}

/// All `.c` files below `root` as (path below root with `\`, text), in directory order.
fn read_scripts(root: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry"))
            .collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}\\{name}")
            };
            if e.path().is_dir() {
                walk(&e.path(), &rel, out);
            } else if Path::new(&name).extension().is_some_and(|x| x == "c") {
                out.push((rel, fs::read_to_string(e.path()).expect("read")));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out
}

fn script_files(files: &[(String, String)], module: usize) -> Vec<ScriptFile<'_>> {
    files
        .iter()
        .map(|(p, t)| ScriptFile {
            module,
            path: p,
            text: t,
        })
        .collect()
}

fn stable() -> BTreeSet<String> {
    active_defines(Branch::Stable)
}

fn vanilla_with(defines: &BTreeSet<String>) -> ApiIndex {
    let files = read_scripts(&fixture("vanilla/scripts"));
    let mut idx = ApiIndex::new("vanilla", "Fixture", "test");
    index_scripts(&script_files(&files, 0), defines, &mut idx);
    idx
}

fn vanilla() -> ApiIndex {
    vanilla_with(&stable())
}

fn cls<'a>(idx: &'a ApiIndex, name: &str) -> &'a Class {
    assert!(idx.classes.contains_key(name), "class {name}");
    idx.classes.get(name).expect("checked")
}

fn mem<'a>(c: &'a Class, name: &str) -> &'a Member {
    assert!(has(c, name), "member {name} of {}", c.name);
    c.members.iter().find(|m| m.name == name).expect("checked")
}

fn has(c: &Class, name: &str) -> bool {
    c.members.iter().any(|m| m.name == name)
}

fn en<'a>(idx: &'a ApiIndex, name: &str) -> &'a Enum {
    {
        assert!(idx.enums.contains_key(name), "enum {name}");
        idx.enums.get(name).expect("checked")
    }
}

fn mods(m: &Member) -> Vec<&str> {
    m.modifiers.iter().map(String::as_str).collect()
}

#[test]
fn active_defines_for_both_branches() {
    for b in [Branch::Stable, Branch::Experimental] {
        let d = active_defines(b);
        for want in [
            "DAYZ_1_30",
            "SERVER",
            "PLATFORM_LINUX",
            "RELEASE",
            "FEATURE_NETWORK_RECONCILIATION",
        ] {
            assert!(d.contains(want), "{b:?} lacks {want}");
        }
    }
}

#[test]
fn parses_class_with_base_and_members() {
    let idx = vanilla();
    let base = cls(&idx, "Base");
    assert_eq!(base.base, None);
    assert_eq!(base.doc.as_deref(), Some("The base of everything."));
    assert_eq!(base.decls[0].path, "3_Game\\Base.c");
    assert_eq!(base.decls[0].module, 0);
    assert_eq!(base.decls[0].line, 3);

    let count = mem(base, "m_Count");
    assert_eq!(count.kind, MemberKind::Field);
    assert_eq!(count.signature, "int m_Count");
    assert_eq!(count.doc.as_deref(), Some("Counter, with a doc."));
    assert_eq!(count.decl.line, 6);

    let run = mem(base, "Run");
    assert_eq!(run.kind, MemberKind::Method);
    assert_eq!(
        run.signature,
        "void Run(int x=5,string s=\"x,y\",array<int> list=null)"
    );
    assert_eq!(run.doc.as_deref(), Some("Run it."));
    assert_eq!(run.decl.line, 21);

    assert_eq!(
        mem(base, "GetInstance").signature,
        "static Base GetInstance()"
    );
    assert_eq!(mods(mem(base, "GetInstance")), ["static"]);
    assert_eq!(
        mem(base, "GetNames").signature,
        "array<string> GetNames(int count=1)"
    );
    assert_eq!(mem(base, "~Base").signature, "void ~Base()");
    assert_eq!(mem(base, "m_Children").signature, "array<Base> m_Children");
    assert_eq!(mem(base, "m_Param").signature, "Param<int> m_Param");
    assert_eq!(mem(base, "m_Arr").signature, "int m_Arr[4]");
    assert_eq!(mem(base, "m_Doc").doc, None, "//!< is not a doc comment");
    assert_eq!(
        mem(base, "Protected").signature,
        "protected void Protected()"
    );
    assert_eq!(mods(mem(base, "Protected")), ["protected"]);
    assert_eq!(mem(base, "LIMIT").signature, "static const int LIMIT=10");
    assert_eq!(mods(mem(base, "LIMIT")), ["static", "const"]);
    assert_eq!(mem(base, "s_Value").signature, "static int s_Value");

    let ctr = cls(&idx, "Legacy");
    assert_eq!(
        ctr.base.as_deref(),
        Some("Param"),
        "`:` is accepted like `extends`"
    );
    let z = cls(&idx, "Zeta");
    assert_eq!(z.base.as_deref(), Some("Base"));
    assert_eq!(z.doc.as_deref(), Some("Zeta documented."));
    assert_eq!(
        cls(&idx, "AtDoc").doc,
        None,
        "`//! @{{` is a group marker, not a doc"
    );
    assert_eq!(
        cls(&idx, "Vec2").doc.as_deref(),
        Some("Struct docs."),
        "structs are classes"
    );
    let fwd = cls(&idx, "Fwd");
    assert!(fwd.members.is_empty() && fwd.decls.len() == 1 && !fwd.modded_only);
    assert_eq!(cls(&idx, "Empty").members.len(), 0);
}

#[test]
fn modded_parts_are_merged() {
    let idx = vanilla();
    let base = cls(&idx, "Base");
    assert_eq!(base.decls.len(), 2);
    assert_eq!(base.decls[1].path, "4_World\\Modded.c");
    assert_eq!(base.decls[1].line, 1);
    assert!(!base.modded_only);
    for added in ["m_Added", "AddedMethod", "StaticAdded"] {
        assert!(has(base, added), "{added}");
    }
    assert_eq!(
        mem(base, "StaticAdded").signature,
        "static void StaticAdded()"
    );
    assert_eq!(mem(base, "m_Added").decl.path, "4_World\\Modded.c");
    assert_eq!(mem(base, "m_Added").decl.line, 3);
    // The override of an existing method does not duplicate it, and the first declaration wins.
    for name in ["Run", "Protected"] {
        assert_eq!(
            base.members.iter().filter(|m| m.name == name).count(),
            1,
            "{name}"
        );
        assert!(!mods(mem(base, name)).contains(&"override"), "{name}");
    }
    let missions = cls(&idx, "MissionFixture");
    assert_eq!(missions.base.as_deref(), Some("Base"));
    assert_eq!(mods(mem(missions, "Run2")), ["override"]);
}

#[test]
fn modded_only_class() {
    let idx = vanilla();
    let c = cls(&idx, "OnlyModded");
    assert!(c.modded_only);
    assert_eq!(c.base, None);
    assert_eq!(c.decls.len(), 1);
    assert_eq!(mods(mem(c, "Anything")), ["override"]);
    assert!(!cls(&idx, "Sealed").modded_only);
    assert_eq!(cls(&idx, "Sealed").base.as_deref(), Some("Base"));
}

#[test]
fn ifdef_variant_picks_active_base_and_records_alt() {
    let idx = vanilla();
    let v = cls(&idx, "Variant");
    assert_eq!(v.base.as_deref(), Some("Base"));
    assert_eq!(v.alt_bases, ["Other"]);
    assert_eq!(v.decls.len(), 2, "plus `modded class Variant`");
    assert!(has(v, "VariantMethod"));
    let v2 = cls(&idx, "Variant2");
    assert_eq!(
        (v2.base.as_deref(), v2.alt_bases.as_slice()),
        (Some("Other"), ["Base".to_string()].as_slice())
    );
    let v3 = cls(&idx, "Variant3");
    assert_eq!(
        (v3.base.as_deref(), v3.alt_bases.as_slice()),
        (Some("Other"), ["Base".to_string()].as_slice())
    );
    assert!(has(v3, "x"));
    assert_eq!(v3.decls[0].line, 63, "line of `class`, not of the #ifdef");

    // With no defines the other branch is active.
    let idx = vanilla_with(&BTreeSet::new());
    let v = cls(&idx, "Variant");
    assert_eq!(
        (v.base.as_deref(), v.alt_bases.as_slice()),
        (Some("Other"), ["Base".to_string()].as_slice())
    );
    assert_eq!(cls(&idx, "Variant2").base.as_deref(), Some("Base"));
}

#[test]
fn private_members_dropped() {
    let idx = vanilla();
    let base = cls(&idx, "Base");
    assert!(!has(base, "m_Secret"));
    assert!(
        !has(base, "Secret"),
        "also not when modded as `private override`"
    );
    assert!(has(base, "m_Prot") && has(base, "Protected"));
}

#[test]
fn override_and_proto_native_modifiers() {
    let idx = vanilla();
    let base = cls(&idx, "Base");
    let len = mem(base, "Length");
    assert_eq!(
        (len.signature.as_str(), mods(len)),
        ("native int Length()", vec!["native"])
    );
    let text = mem(base, "Text");
    assert_eq!(
        text.signature,
        "native string Text(Base other,out string o,inout int io)"
    );
    assert_eq!(mods(text), ["native"]);
    let ext = mem(base, "External");
    assert_eq!(
        ext.signature, "native void External()",
        "external is dropped"
    );
    assert_eq!(mods(ext), ["native"]);
    let run2 = mem(base, "Run2");
    assert_eq!(
        (run2.signature.as_str(), mods(run2)),
        ("override void Run2()", vec!["override"])
    );
    let ev = mem(base, "OnEvent");
    assert_eq!(
        (ev.signature.as_str(), mods(ev)),
        ("void OnEvent()", Vec::<&str>::new()),
        "event is dropped"
    );
    assert_eq!(mods(mem(cls(&idx, "LibThing"), "Native")), ["native"]);
}

#[test]
fn obsolete_attribute() {
    let idx = vanilla();
    let base = cls(&idx, "Base");
    let old = mem(base, "OldMethod");
    assert!(old.obsolete);
    assert_eq!(old.signature, "int OldMethod()");
    assert_eq!(old.decl.line, 28, "the attribute is the first token");
    let names = mem(base, "GetNames");
    assert!(!names.obsolete);
    assert_eq!(names.decl.line, 30);
    let f = idx
        .functions
        .iter()
        .find(|m| m.name == "OldAdd")
        .expect("OldAdd");
    assert!(f.obsolete);
    assert_eq!(f.signature, "void OldAdd(int a=1,int b=2)");
}

#[test]
fn template_params() {
    let idx = vanilla();
    assert_eq!(cls(&idx, "Param").type_params, ["T"]);
    let pair = cls(&idx, "Pair");
    assert_eq!(pair.type_params, ["K", "V"]);
    assert_eq!(pair.base.as_deref(), Some("Param<K>"));
    assert_eq!(pair.base_name(), Some("Param"));
    assert_eq!(mem(pair, "m_Map").signature, "map<K,V> m_Map");
    assert_eq!(cls(&idx, "Base").type_params.len(), 0);
}

#[test]
fn enum_values_implicit_explicit_hex() {
    let idx = vanilla();
    let e = en(&idx, "AIStatus");
    let got: Vec<(&str, Option<i64>)> =
        e.items.iter().map(|i| (i.name.as_str(), i.value)).collect();
    assert_eq!(
        got,
        [
            ("IDLE", Some(0)),
            ("RUNNING", Some(1)),
            ("DONE", Some(10)),
            ("AFTER", Some(11)),
            ("HEX", Some(16)),
            ("NEXT", Some(17)),
            ("NEG", Some(-3)),
            ("NEGNEXT", Some(-2)),
            ("PLUS", Some(5)),
            ("BIN", Some(3)),
            ("LEADING", None),
            ("LOST", None),
            ("EXPR", None),
            ("ALSOLOST", None),
            ("UNDER", None),
            ("LAST", None),
            ("EXTRA", None),
        ]
    );
    assert_eq!(e.decls.len(), 2);
    assert_eq!(
        (e.decls[0].path.as_str(), e.decls[0].line),
        ("3_Game\\AI\\AIAgent.c", 2)
    );
    let plain = en(&idx, "Plain");
    assert_eq!(plain.base.as_deref(), Some("int"));
    assert_eq!(
        plain.items.iter().map(|i| i.value).collect::<Vec<_>>(),
        [Some(0), Some(4), Some(5)]
    );
    assert_eq!(en(&idx, "Empty2").items.len(), 0);
    // A modded enum continues counting after the last item of the enum it extends.
    let counted = en(&idx, "Counted");
    assert_eq!(
        counted.items.iter().map(|i| i.value).collect::<Vec<_>>(),
        [Some(0), Some(4), Some(5), Some(6)]
    );
    assert_eq!(counted.items[3].name, "D");
    // A plain redeclaration is a new body that counts from zero.
    let plain2 = en(&idx, "Plain2");
    assert_eq!(
        plain2.items.iter().map(|i| i.value).collect::<Vec<_>>(),
        [Some(0x7F), Some(0x80), Some(0)]
    );
}

#[test]
fn enum_item_docs() {
    let idx = vanilla();
    let e = en(&idx, "AIStatus");
    assert_eq!(e.items[0].doc.as_deref(), Some("Not started"));
    assert_eq!(e.items[1].doc, None);
    assert_eq!(e.items[2].doc.as_deref(), Some("Explicit value"));
    assert!(
        !idx.classes.contains_key("AIStatus"),
        "enums are not classes"
    );
    assert_eq!(cls(&idx, "AIAgent").base.as_deref(), Some("Base"));
}

#[test]
fn typedefs_functions_globals() {
    let idx = vanilla();
    let td: Vec<_> = idx
        .typedefs
        .iter()
        .map(|m| (m.name.as_str(), m.signature.as_str(), m.doc.as_deref()))
        .collect();
    assert_eq!(
        td,
        [
            (
                "TStringArray",
                "array<string> TStringArray",
                Some("Plain typedefs and helpers of the fixture core.")
            ),
            ("TStringIntMap", "map<string,int> TStringIntMap", None),
            (
                "TParamArray",
                "array<Param> TParamArray",
                Some("Documented alias.")
            ),
        ]
    );
    assert_eq!(idx.typedefs[0].decl.line, 7);

    let fns: Vec<_> = idx.functions.iter().map(|m| m.signature.as_str()).collect();
    assert_eq!(
        fns,
        [
            "int Add(int a,int b)",
            "native void FixtureNative(string text,out int result)",
            "native string FixtureString(Param p)",
            "private void HiddenFn()",
            "void OldAdd(int a=1,int b=2)",
            "void Balanced()",
            "void ParamSplit(int a)",
            "void MissionHelper(int x)",
        ]
    );
    assert_eq!(idx.functions[0].doc.as_deref(), Some("Adds two numbers."));
    assert_eq!(idx.functions[0].decl.line, 20);
    assert!(idx.functions.iter().all(|m| m.kind == MemberKind::Method));

    let globals: Vec<_> = idx
        .globals
        .iter()
        .map(|m| (m.name.as_str(), m.signature.as_str()))
        .collect();
    assert_eq!(
        globals,
        [
            ("FIXTURE_FLAG", "const int FIXTURE_FLAG=3"),
            ("g_Counter", "int g_Counter"),
            ("g_List", "static array<int> g_List"),
            ("g_Table", "int g_Table[]"),
            ("g_Url", "string g_Url"),
            ("g_Hidden", "protected Param g_Hidden"),
        ],
        "the duplicate g_Counter of 5_Mission is listed once"
    );
}

#[test]
fn initializer_list_is_a_field() {
    let idx = vanilla();
    let base = cls(&idx, "Base");
    let names = mem(base, "m_Names");
    assert_eq!(names.kind, MemberKind::Field);
    assert_eq!(names.signature, "string m_Names[]");
    // The members after it are not swallowed.
    let order: Vec<_> = base
        .members
        .iter()
        .map(|m| m.name.as_str())
        .skip_while(|n| *n != "m_Names")
        .take(3)
        .collect();
    assert_eq!(order, ["m_Names", "apple", "Banana"]);
}

#[test]
fn strings_with_braces_and_comments() {
    let idx = vanilla();
    let base = cls(&idx, "Base");
    assert_eq!(mem(base, "Quote").signature, "string Quote()");
    assert_eq!(mem(base, "Inline").signature, "int Inline()");
    assert!(
        has(base, "OnEvent"),
        "member after a body with a string full of braces"
    );
    assert_eq!(
        mem(cls(&idx, "Pair"), "SEP").signature,
        "static const string SEP=\"a;b{c//d\""
    );
    let m = cls(&idx, "MissionFixture");
    assert_eq!(m.members.len(), 2);
    assert_eq!(mem(m, "OnInit").signature, "void OnInit()");
    assert!(idx.classes.contains_key("Zeta"), "class after all of that");
    assert_eq!(
        idx.globals
            .iter()
            .find(|g| g.name == "g_Url")
            .map(|g| g.signature.as_str()),
        Some("string g_Url")
    );
}

#[test]
fn copyright_header_is_not_a_doc() {
    let idx = vanilla();
    assert_eq!(cls(&idx, "LibThing").doc, None);
    assert_eq!(cls(&idx, "Other").doc, None, "`Foo.c` is a file name");
    let all_docs = idx
        .typedefs
        .iter()
        .chain(&idx.functions)
        .chain(&idx.globals)
        .filter_map(|m| m.doc.as_deref())
        .chain(idx.classes.values().filter_map(|c| c.doc.as_deref()));
    for d in all_docs {
        assert!(!d.contains("Copyright"), "{d}");
    }
}

#[test]
fn crlf_line_numbers() {
    let idx = vanilla();
    let c = cls(&idx, "Crlf");
    assert_eq!(c.base.as_deref(), Some("Base"));
    assert_eq!(c.doc.as_deref(), Some("Crlf class."));
    assert_eq!(c.decls[0].line, 2);
    let go = mem(c, "Go");
    assert_eq!((go.decl.line, go.doc.as_deref()), (5, Some("Crlf member.")));
    assert_eq!(mem(c, "m_X").decl.line, 6);
    assert_eq!(c.members.len(), 2, "#define is not a member");
}

#[test]
fn deterministic_order() {
    let files = read_scripts(&fixture("vanilla/scripts"));
    let mut idx = ApiIndex::new("vanilla", "Fixture", "test");
    index_scripts(&script_files(&files, 0), &stable(), &mut idx);
    let order = cls(&idx, "Order");
    let paths: Vec<_> = order.decls.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "4_World\\B.c",
            "4_World\\a.c",
            "4_World\\a\\b\\y.c",
            "4_World\\a\\x.c",
            "4_World\\a\\z.c",
            "4_World\\a_b.c"
        ]
    );
    assert_eq!(
        order.base.as_deref(),
        Some("BaseB"),
        "the first declaration in load order names the base"
    );
    let names: Vec<_> = order.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "FromUpperB",
            "FromA",
            "FromABY",
            "FromAX",
            "FromAZ",
            "FromAB"
        ]
    );

    let mut reversed = files.clone();
    reversed.reverse();
    let mut idx2 = ApiIndex::new("vanilla", "Fixture", "test");
    index_scripts(&script_files(&reversed, 0), &stable(), &mut idx2);
    assert_eq!(idx, idx2, "input order does not matter");
    // Layers sort before paths: 4_World/Modded.c comes after 3_Game files, whatever their names.
    assert_eq!(cls(&idx, "Variant").decls[1].path, "4_World\\Modded.c");
}

#[test]
fn modules_sort_after_layers() {
    let a = "class X { void FromLate(); }";
    let b = "class X { void FromEarly(); }";
    let files = [
        ScriptFile {
            module: 1,
            path: "3_Game\\a.c",
            text: a,
        },
        ScriptFile {
            module: 0,
            path: "4_World\\a.c",
            text: b,
        },
        ScriptFile {
            module: 0,
            path: "3_Game\\z.c",
            text: b,
        },
    ];
    let mut idx = ApiIndex::new("m", "m", "1");
    index_scripts(&files, &BTreeSet::new(), &mut idx);
    let x = cls(&idx, "X");
    let got: Vec<_> = x
        .decls
        .iter()
        .map(|d| (d.module, d.path.as_str()))
        .collect();
    assert_eq!(
        got,
        [(0, "3_Game\\z.c"), (1, "3_Game\\a.c"), (0, "4_World\\a.c")]
    );
}

#[test]
fn hostile_input_does_not_panic() {
    let deep = "{".repeat(10_000);
    let deep_closed = format!("class D {{ void f() {}{} }}", deep, "}".repeat(10_000));
    let cases: Vec<String> = vec![
        "class A { void f(); } /* unterminated".into(),
        "class B { string s = \"abc; void g();\n void h(); }".into(),
        "class C { string s = \"abc".into(),
        deep.clone(),
        format!("class E {deep}"),
        deep_closed,
        "#endif\n#endif\n#else\nclass F { }\n#endif".into(),
        "#ifdef X\n#ifdef Y\nclass G extends H\n".into(),
        "class N\0 { void\0 f(); }\0".into(),
        "enum Z { A = , , = 3, ,, B = 99999999999999999999999 }".into(),
        "class { } class X extends { } enum { } typedef ; ( ) ] [ \u{1}text\u{2} \u{1}".into(),
        "class U { void f(int a = \u{e9}\u{4e2d}); } /*! \u{e9} */ //! \u{4e2d}".into(),
        "[".repeat(5000),
        "(".repeat(100_000),
        "//! x\n".repeat(50_000),
    ];
    for (n, text) in cases.iter().enumerate() {
        let files = [ScriptFile {
            module: 0,
            path: "3_Game\\h.c",
            text,
        }];
        let mut idx = ApiIndex::new("h", "h", "1");
        let start = Instant::now();
        index_scripts(&files, &stable(), &mut idx);
        assert!(start.elapsed() < Duration::from_secs(5), "case {n} is slow");
    }
    // Best-effort partial results.
    let files = [ScriptFile {
        module: 0,
        path: "3_Game\\h.c",
        text: &cases[0],
    }];
    let mut idx = ApiIndex::new("h", "h", "1");
    index_scripts(&files, &stable(), &mut idx);
    assert!(has(cls(&idx, "A"), "f"));
}

#[test]
fn collect_defines_from_config_and_scripts() {
    let config = fs::read_to_string(fixture("mod/JM/CF/config.cpp")).expect("config");
    let files = read_scripts(&fixture("mod/JM/CF/Scripts"));
    let defs = collect_defines(&config, &script_files(&files, 0));
    assert_eq!(
        defs,
        [
            "CF_FIXTURE",
            "CF_MODULE",
            "CF_FIXTURE_LOCAL",
            "CF_INDENTED",
            "CF_LATER"
        ]
    );
    assert_eq!(collect_defines("", &[]).len(), 0);
    assert_eq!(
        collect_defines(
            "class X { defines[] = {\"A\"}; defines[]={\"B\",\"A\"}; };",
            &[]
        ),
        ["A", "B"]
    );
}

#[test]
fn mod_scripts_use_defines_and_local_defines() {
    let files = read_scripts(&fixture("mod/JM/CF/Scripts"));
    let build = |defines: &[&str]| {
        let set: BTreeSet<String> = defines.iter().map(ToString::to_string).collect();
        let mut idx = ApiIndex::new("cf", "CF", "1");
        index_scripts(&script_files(&files, 3), &set, &mut idx);
        idx
    };
    let idx = build(&["CF_FIXTURE"]);
    let sw = cls(&idx, "CFSwitch");
    assert_eq!(
        (sw.base.as_deref(), sw.alt_bases.as_slice()),
        (Some("CFBase"), ["Base".to_string()].as_slice())
    );
    assert_eq!(sw.decls[0].module, 3);
    assert_eq!(sw.decls[0].line, 10);
    assert!(
        cls(&idx, "Base").modded_only,
        "the mod only has `modded class Base`"
    );
    assert!(has(cls(&idx, "Base"), "CFExtra"));
    assert_eq!(cls(&idx, "CFWorld").base.as_deref(), Some("CFBase"));
    assert_eq!(
        cls(&idx, "LocalDefined").cond,
        None,
        "#define earlier in the same file"
    );
    assert_eq!(
        cls(&idx, "TooEarly").cond.as_deref(),
        Some("CF_LATER"),
        "#define after the #ifdef does not count"
    );

    let idx = build(&[]);
    let sw = cls(&idx, "CFSwitch");
    assert_eq!(
        (sw.base.as_deref(), sw.alt_bases.as_slice()),
        (Some("Base"), ["CFBase".to_string()].as_slice())
    );
}

#[test]
fn indexing_twice_extends_the_same_index() {
    let files = read_scripts(&fixture("vanilla/scripts"));
    let mut idx = ApiIndex::new("vanilla", "Fixture", "test");
    index_scripts(&script_files(&files, 0), &stable(), &mut idx);
    let once = idx.clone();
    index_scripts(&script_files(&files, 0), &stable(), &mut idx);
    assert_eq!(idx.typedefs, once.typedefs, "typedefs are deduplicated");
    assert_eq!(idx.functions, once.functions);
    assert_eq!(cls(&idx, "Base").members, cls(&once, "Base").members);
}

#[test]
fn index_roundtrips_through_json() {
    let mut idx = vanilla();
    idx.modules.push(Module {
        prefix: "DZ\\scripts".into(),
        pbo: "dta/scripts.pbo".into(),
    });
    idx.inputs.push(InputStamp {
        path: "/x/scripts.pbo".into(),
        size: 7,
        mtime_ns: 1_700_000_000_123_456_789,
    });
    idx.defines.insert("CF".into(), vec!["CF_FIXTURE".into()]);
    let json = serde_json::to_string(&idx).expect("serialize");
    let back: ApiIndex = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(idx, back);
    assert_eq!(back.format, dayzmake::api::model::INDEX_FORMAT);
}

#[test]
fn real_style_header_regression() {
    // `class X extends Y` with attributes and a doc before it.
    let text = "//! Doc.\n[Attr]\nclass X extends Y<int>\n{\n}\n";
    let files = [ScriptFile {
        module: 0,
        path: "3_Game\\x.c",
        text,
    }];
    let mut idx = ApiIndex::new("x", "x", "1");
    index_scripts(&files, &BTreeSet::new(), &mut idx);
    let x = cls(&idx, "X");
    assert_eq!(
        (x.base.as_deref(), x.base_name(), x.doc.as_deref()),
        (Some("Y<int>"), Some("Y"), Some("Doc."))
    );
    assert_eq!(x.decls[0].line, 2);
}

fn cond(c: &Class) -> Option<&str> {
    c.cond.as_deref()
}

#[test]
fn inactive_branch_class_is_kept_with_cond() {
    let idx = vanilla();
    let diag = cls(&idx, "DiagOnly");
    assert_eq!(cond(diag), Some("DIAG_DEVELOPER"));
    assert_eq!(diag.base.as_deref(), Some("Base"));
    assert_eq!(diag.doc.as_deref(), Some("Diag only."));
    assert_eq!(diag.decls[0].line, 3);
    let dm = mem(diag, "DiagMethod");
    assert_eq!(
        (dm.cond.as_deref(), dm.decl.line),
        (Some("DIAG_DEVELOPER"), 5)
    );
    // The class under `#ifdef GAME_TEMPLATE` of the fixture core is inactive too.
    assert_eq!(cond(cls(&idx, "Base")), None);
    let diag_enum = en(&idx, "DiagEnum");
    assert_eq!(diag_enum.cond.as_deref(), Some("DIAG_DEVELOPER"));
    assert_eq!(
        diag_enum.items.iter().map(|i| i.value).collect::<Vec<_>>(),
        [Some(0), Some(1)]
    );
    // The active defines decide.
    let mut with = stable();
    with.insert("DIAG_DEVELOPER".to_string());
    let idx = vanilla_with(&with);
    assert_eq!(cond(cls(&idx, "DiagOnly")), None);
    assert_eq!(en(&idx, "DiagEnum").cond, None);
}

#[test]
fn inactive_branch_member_is_tagged() {
    let idx = vanilla();
    let mixed = cls(&idx, "Mixed");
    assert_eq!(cond(mixed), None);
    let tags: Vec<_> = mixed
        .members
        .iter()
        .map(|m| (m.name.as_str(), m.cond.as_deref()))
        .collect();
    assert_eq!(
        tags,
        [
            ("Always", None),
            ("DiagHelper", Some("DIAG_DEVELOPER")),
            ("ClientHelper", Some("!SERVER")),
            ("ServerHelper", None),
            ("Twice", None),
        ]
    );
    // Inactive items of an enum do not shift the values of the others.
    let items: Vec<_> = en(&idx, "CondEnum")
        .items
        .iter()
        .map(|i| (i.name.as_str(), i.value))
        .collect();
    assert_eq!(items, [("A", Some(0)), ("C", Some(1))]);
    // Parts of a header in another branch are not part of its signature.
    let f = idx
        .functions
        .iter()
        .find(|m| m.name == "ParamSplit")
        .expect("ParamSplit");
    assert_eq!(f.signature, "void ParamSplit(int a)");
    // A branch whose braces do not balance is dropped instead of swallowing what follows.
    assert!(
        idx.functions
            .iter()
            .any(|m| m.name == "Balanced" && m.cond.is_none())
    );
    assert!(!idx.functions.iter().any(|m| m.name == "Unbalanced"));
}

#[test]
fn active_wins_over_inactive_duplicate() {
    let idx = vanilla();
    // The same method in an inactive and an active branch is listed once, as active.
    let dup = cls(&idx, "Dup");
    assert_eq!(dup.members.len(), 1);
    assert_eq!(dup.members[0].cond, None);
    // Inactive class first, active class later: the active data wins, the other base is an alternative.
    let a = cls(&idx, "DupClass");
    assert_eq!(
        (cond(a), a.base.as_deref(), a.alt_bases.as_slice()),
        (None, Some("Base"), ["Other".to_string()].as_slice())
    );
    assert_eq!(a.decls.len(), 1, "same file");
    let names: Vec<_> = a
        .members
        .iter()
        .map(|m| (m.name.as_str(), m.cond.as_deref()))
        .collect();
    assert_eq!(
        names,
        [
            ("InactiveOnly", Some("DIAG_DEVELOPER")),
            ("ActiveOnly", None)
        ]
    );
    // Active class first, inactive later: the inactive base is only an alternative.
    let b = cls(&idx, "DupClass2");
    assert_eq!(
        (cond(b), b.base.as_deref(), b.alt_bases.as_slice()),
        (None, Some("Base"), ["Other".to_string()].as_slice())
    );
    assert!(has(b, "ActiveFirst") && has(b, "InactiveSecond"));
    assert_eq!(
        mem(b, "InactiveSecond").cond.as_deref(),
        Some("DIAG_DEVELOPER")
    );
}

#[test]
fn ifndef_else_cond_text() {
    let idx = vanilla();
    assert_eq!(
        cond(cls(&idx, "ClientOnly")),
        Some("!SERVER"),
        "#ifndef SERVER"
    );
    assert_eq!(cond(cls(&idx, "ServerOnly")), None);
    // With the define off, the `#else` branch is the inactive one: `!SERVER` negated again.
    let idx = vanilla_with(&BTreeSet::new());
    assert_eq!(cond(cls(&idx, "ClientOnly")), None);
    assert_eq!(cond(cls(&idx, "ServerOnly")), Some("SERVER"));
}

#[test]
fn elif_branch_is_evaluated() {
    let idx = vanilla();
    assert_eq!(cond(cls(&idx, "ElifA")), Some("DIAG_DEVELOPER"));
    assert_eq!(cond(cls(&idx, "ElifB")), None);
    assert_eq!(
        cond(cls(&idx, "ElifC")),
        Some("PLATFORM_LINUX"),
        "an earlier branch was taken"
    );
    assert_eq!(cond(cls(&idx, "ElifD")), Some("!DIAG_DEVELOPER"));
    assert_eq!(
        cond(cls(&idx, "Complex")),
        None,
        "complex expressions count as true"
    );
    let idx = vanilla_with(&BTreeSet::new());
    assert_eq!(cond(cls(&idx, "ElifC")), Some("PLATFORM_LINUX"));
    assert_eq!(cond(cls(&idx, "ElifD")), None);
    let mut linux = BTreeSet::new();
    linux.insert("PLATFORM_LINUX".to_string());
    let idx = vanilla_with(&linux);
    assert_eq!(cond(cls(&idx, "ElifC")), None);
    assert_eq!(cond(cls(&idx, "ElifD")), Some("!DIAG_DEVELOPER"));
}

// ---- query engine ----

/// Vanilla plus the fixture mod (`cf`, module 3, defines `CF_FIXTURE`) as a second index.
fn both() -> Vec<ApiIndex> {
    let mut van = vanilla();
    van.modules = vec![Module {
        prefix: "scripts".into(),
        pbo: "dta/scripts.pbo".into(),
    }];
    let files = read_scripts(&fixture("mod/JM/CF/Scripts"));
    let set: BTreeSet<String> = ["CF_FIXTURE".to_string()].into();
    let mut cf = ApiIndex::new("cf", "CF", "1");
    let m = |p: &str, b: &str| Module {
        prefix: p.into(),
        pbo: b.into(),
    };
    cf.modules = vec![
        m("a", "addons/a.pbo"),
        m("b", "addons/b.pbo"),
        m("c", "addons/c.pbo"),
        m("jm\\cf\\scripts", "addons/jm_cf.pbo"),
    ];
    index_scripts(&script_files(&files, 3), &set, &mut cf);
    cf.defines.insert(
        "jm\\cf".into(),
        vec!["CF_FIXTURE".into(), "CF_MODULE".into()],
    );
    vec![van, cf]
}

fn text(idx: &[ApiIndex], q: &Query, max: usize) -> Vec<String> {
    let r = run_query(idx, q).expect("query");
    render_text(&r, idx.len() > 1, max)
}

fn class_q(name: &str, inherited: bool, mods_only: bool) -> Query {
    Query::Class {
        name: name.into(),
        inherited,
        mods_only,
    }
}

fn err(idx: &[ApiIndex], q: &Query) -> String {
    run_query(idx, q).expect_err("must fail").to_string()
}

/// A tiny index of `(name, base)` classes without members.
fn tiny(classes: &[(&str, Option<&str>)]) -> Vec<ApiIndex> {
    let mut idx = ApiIndex::new("vanilla", "T", "1");
    for (n, b) in classes {
        idx.classes.insert(
            (*n).to_string(),
            Class {
                name: (*n).to_string(),
                type_params: Vec::new(),
                base: b.map(ToString::to_string),
                alt_bases: Vec::new(),
                modded_only: false,
                doc: None,
                decls: Vec::new(),
                members: Vec::new(),
                cond: None,
            },
        );
    }
    vec![idx]
}

#[test]
fn class_text_single_index() {
    let idx = [vanilla()];
    assert_eq!(
        text(&idx, &class_q("Mixed", false, false), 100),
        [
            "class Mixed : Base",
            "  void Always()",
            "  void ClientHelper()  [#if !SERVER]",
            "  void DiagHelper()  [#if DIAG_DEVELOPER]",
            "  void ServerHelper()",
            "  void Twice(int a)",
        ]
    );
}

#[test]
fn class_header_doc_template_and_cond() {
    let idx = [vanilla()];
    let l = text(&idx, &class_q("Zeta", false, false), 100);
    assert_eq!(l, ["class Zeta : Base", "  // Zeta documented."]);
    let l = text(&idx, &class_q("Pair", false, false), 100);
    assert_eq!(
        l.first().map(String::as_str),
        Some("class Pair<K,V> : Param<K>")
    );
    assert!(
        l.contains(&"  void Put(K key,V value)".to_string()),
        "{l:?}"
    );
    let l = text(&idx, &class_q("DiagOnly", false, false), 100);
    assert_eq!(
        l,
        [
            "class DiagOnly : Base  [#if DIAG_DEVELOPER]",
            "  // Diag only.",
            "  void DiagMethod()  [#if DIAG_DEVELOPER]",
        ]
    );
    let l = text(&idx, &class_q("Variant3", false, false), 100);
    assert_eq!(
        l.get(..2),
        Some(
            [
                "class Variant3 : Other".to_string(),
                "  // base is Other here; Base under the other branch of its #ifdef".into()
            ]
            .as_slice()
        ),
        "{l:?}"
    );
}

#[test]
fn class_orders_fields_before_methods_and_collapses_overrides() {
    let idx = [vanilla()];
    let l = text(&idx, &class_q("Base", false, false), 200);
    assert_eq!(
        l.get(..2).map(<[String]>::to_vec),
        Some(vec![
            "class Base".to_string(),
            "  // The base of everything.".to_string()
        ])
    );
    let pos = |s: &str| {
        l.iter()
            .position(|x| x.trim_start().starts_with(s))
            .expect(s)
    };
    assert!(
        pos("int apple") < pos("int Banana"),
        "case-insensitive: {l:?}"
    );
    assert!(
        pos("static const int LIMIT") < pos("void Base"),
        "fields before methods"
    );
    assert!(
        l.contains(
            &"  void Run(int x=5,string s=\"x,y\",array<int> list=null)  // Run it.".to_string()
        ),
        "{l:?}"
    );
    assert!(l.contains(&"  override: Run2".to_string()), "{l:?}");
    assert!(!l.iter().any(|x| x.contains("void Run2")), "{l:?}");
}

#[test]
fn class_inherited_walks_template_bases() {
    let idx = [vanilla()];
    let l = text(&idx, &class_q("Pair", true, false), 200);
    let heads: Vec<&str> = l
        .iter()
        .filter(|x| x.starts_with("class "))
        .map(String::as_str)
        .collect();
    assert_eq!(heads, ["class Pair<K,V> : Param<K>", "class Param<T>"]);
    let l = text(&idx, &class_q("Zeta", true, false), 200);
    let heads: Vec<&str> = l
        .iter()
        .filter(|x| x.starts_with("class "))
        .map(String::as_str)
        .collect();
    assert_eq!(heads, ["class Zeta : Base", "class Base"]);
}

#[test]
fn class_with_two_indexes_is_tagged() {
    let idx = both();
    let l = text(&idx, &class_q("Base", false, false), 300);
    let heads: Vec<&str> = l
        .iter()
        .filter(|x| x.starts_with("class "))
        .map(String::as_str)
        .collect();
    assert_eq!(heads.len(), 2, "{heads:?}");
    assert!(
        heads[0].starts_with("class Base  [vanilla 3_Game"),
        "{heads:?}"
    );
    assert_eq!(heads[1], "class Base  [cf 4_World]  [modded]");
    assert!(l.contains(&"  void CFExtra()".to_string()));
}

#[test]
fn class_mods_only_skips_vanilla() {
    let idx = both();
    assert_eq!(
        text(&idx, &class_q("Base", false, true), 100),
        ["class Base  [cf 4_World]  [modded]", "  void CFExtra()",]
    );
    // inherited + mods_only still walks the base chain through vanilla, but prints only mod parts
    let l = text(&idx, &class_q("CFSwitch", true, true), 100);
    assert_eq!(
        l.first().map(String::as_str),
        Some("class CFSwitch : CFBase  [cf 3_Game]")
    );
    assert!(
        l.contains(&"class CFBase  [cf 3_Game]".to_string()),
        "{l:?}"
    );
    assert!(!l.iter().any(|x| x.contains("[vanilla")), "{l:?}");
}

#[test]
fn find_matches_members_by_signature_and_doc() {
    let idx = [vanilla()];
    let q = Query::Find {
        regex: "getinstance".into(),
        class: None,
    };
    assert_eq!(text(&idx, &q, 100), ["Base: static Base GetInstance()"]);
    let q = Query::Find {
        regex: "^it\\.?$|run it".into(),
        class: None,
    };
    assert_eq!(
        text(&idx, &q, 100),
        ["Base: void Run(int x=5,string s=\"x,y\",array<int> list=null)  // Run it."]
    );
    let q = Query::Find {
        regex: "Twice".into(),
        class: Some("Dup".into()),
    };
    assert_eq!(text(&idx, &q, 100), ["Dup: void Twice(int a)"]);
}

#[test]
fn find_anchored_regex_matches_member_names() {
    let idx = [vanilla()];
    let q = Query::Find {
        regex: "^GetInstance$".into(),
        class: None,
    };
    assert_eq!(text(&idx, &q, 100), ["Base: static Base GetInstance()"]);
    let q = Query::Find {
        regex: "^m_Count".into(),
        class: Some("Base".into()),
    };
    let l = text(&idx, &q, 100);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].contains("m_Count"), "{l:?}");
}

#[test]
fn text_output_escapes_untrusted_control_characters() {
    let src = "//! Evil \u{1b}[31mdoc\nclass Esc\n{\n\t//! member \u{1b}[2Jdoc\n\tvoid M();\n}\n";
    let files = [ScriptFile {
        module: 0,
        path: "3_Game\\\u{1b}[1mx.c",
        text: src,
    }];
    let mut idx = ApiIndex::new("vanilla", "T", "1");
    index_scripts(&files, &stable(), &mut idx);
    for q in [
        class_q("Esc", false, false),
        Query::Find {
            regex: "Esc|doc".into(),
            class: None,
        },
        Query::Src { name: "Esc".into() },
    ] {
        let lines = text(std::slice::from_ref(&idx), &q, 100);
        assert_ne!(lines.len(), 0);
        for l in &lines {
            assert!(!l.contains('\u{1b}'), "{l:?}");
        }
    }
    let lines = text(
        std::slice::from_ref(&idx),
        &class_q("Esc", false, false),
        100,
    );
    assert!(
        lines.iter().any(|l| l.contains("\\u{1b}[2Jdoc")),
        "{lines:?}"
    );
}

#[test]
fn not_found_suggests_the_other_case() {
    let idx = [vanilla()];
    let e = err(&idx, &class_q("base", false, false));
    assert!(
        e.starts_with("no class \"base\"; did you mean Base?"),
        "{e}"
    );
    let e = err(
        &idx,
        &Query::Enum {
            name: "aistatus".into(),
        },
    );
    assert!(e.contains("did you mean AIStatus?"), "{e}");
    let e = err(&idx, &class_q("Nope", false, false));
    assert!(e.contains("try: dayzmake api find Nope"), "{e}");
}

#[test]
fn find_class_filter_and_cond() {
    let idx = [vanilla()];
    let q = Query::Find {
        regex: "helper".into(),
        class: Some("Mixed".into()),
    };
    assert_eq!(
        text(&idx, &q, 100),
        [
            "Mixed: void DiagHelper()  [#if DIAG_DEVELOPER]",
            "Mixed: void ClientHelper()  [#if !SERVER]",
            "Mixed: void ServerHelper()",
        ]
    );
}

#[test]
fn find_class_names_enum_items_and_top_level() {
    let idx = both();
    let q = Query::Find {
        regex: "^CFBase$".into(),
        class: None,
    };
    assert_eq!(text(&idx, &q, 100), ["[cf] class CFBase"]);
    let q = Query::Find {
        regex: "^EXTRA$".into(),
        class: None,
    };
    assert_eq!(text(&idx, &q, 100), ["[vanilla] AIStatus: EXTRA"]);
    let q = Query::Find {
        regex: "^HEX$".into(),
        class: None,
    };
    assert_eq!(text(&idx, &q, 100), ["[vanilla] AIStatus: HEX = 16"]);
    let q = Query::Find {
        regex: "FixtureNative|TParamArray|g_Counter".into(),
        class: None,
    };
    let l = text(&idx, &q, 100);
    assert_eq!(l.len(), 3, "{l:?}");
    assert!(
        l.iter()
            .any(|x| x.starts_with("[vanilla] fn: ") && x.contains("FixtureNative")),
        "{l:?}"
    );
    assert!(
        l.iter()
            .any(|x| x.starts_with("[vanilla] typedef: ") && x.contains("TParamArray")),
        "{l:?}"
    );
    assert!(
        l.iter()
            .any(|x| x.starts_with("[vanilla] var: ") && x.contains("g_Counter")),
        "{l:?}"
    );
    // a class filter excludes top-level declarations
    let q = Query::Find {
        regex: "FixtureNative".into(),
        class: Some("Base".into()),
    };
    assert!(matches!(run_query(&idx, &q), Ok(QueryResult::Matches(m)) if m.is_empty()));
    assert_eq!(text(&idx, &q, 100), ["no match"]);
    assert_eq!(
        text(&[vanilla()], &q, 100),
        ["no match  (searched vanilla only; add -m <mod> or -m all for mods)"]
    );
}

#[test]
fn sub_direct_and_all() {
    let idx = [vanilla()];
    let q = Query::Sub {
        name: "Param".into(),
        all: false,
    };
    assert_eq!(text(&idx, &q, 100), ["Legacy Pair"]);
    let t = tiny(&[
        ("A", None),
        ("B", Some("A")),
        ("C", Some("B")),
        ("D", Some("A<int>")),
    ]);
    let q = Query::Sub {
        name: "A".into(),
        all: false,
    };
    assert_eq!(text(&t, &q, 100), ["B D"]);
    let q = Query::Sub {
        name: "A".into(),
        all: true,
    };
    assert_eq!(text(&t, &q, 100), ["B D C"]);
    let q = Query::Sub {
        name: "C".into(),
        all: true,
    };
    assert_eq!(text(&t, &q, 100), ["no subclasses"]);
    let cyc = tiny(&[("A", Some("B")), ("B", Some("A"))]);
    let q = Query::Sub {
        name: "A".into(),
        all: true,
    };
    assert_eq!(text(&cyc, &q, 100), ["B A"]);
}

#[test]
fn sub_spans_indexes() {
    let idx = both();
    let q = Query::Sub {
        name: "CFBase".into(),
        all: false,
    };
    assert_eq!(text(&idx, &q, 100), ["CFSwitch CFWorld"]);
    let q = Query::Sub {
        name: "Base".into(),
        all: false,
    };
    let l = text(&idx, &q, 100);
    assert!(l[0].split(' ').any(|n| n == "AIAgent"), "{l:?}");
    assert!(
        !l[0].split(' ').any(|n| n == "CFSwitch"),
        "its base is CFBase when CF_FIXTURE is on: {l:?}"
    );
}

#[test]
fn enum_text() {
    let idx = [vanilla()];
    let q = Query::Enum {
        name: "Counted".into(),
    };
    assert_eq!(
        text(&idx, &q, 100),
        ["enum Counted", "  A = 0", "  B = 4", "  C = 5", "  D = 6"]
    );
    let q = Query::Enum {
        name: "Plain".into(),
    };
    assert_eq!(text(&idx, &q, 100)[0], "enum Plain : int");
    let q = Query::Enum {
        name: "AIStatus".into(),
    };
    let l = text(&idx, &q, 100);
    assert_eq!(l[0], "enum AIStatus");
    assert_eq!(l[1], "  IDLE = 0  // Not started");
    let q = Query::Enum {
        name: "CondEnum".into(),
    };
    assert_eq!(text(&idx, &q, 100)[0], "enum CondEnum");
    let q = Query::Enum {
        name: "DiagEnum".into(),
    };
    assert_eq!(
        text(&idx, &q, 100)[0],
        "enum DiagEnum  [#if DIAG_DEVELOPER]"
    );
}

#[test]
fn enum_with_two_indexes() {
    let mut idx = both();
    let mut counted = en(&idx[0], "Counted").clone();
    counted.decls.clear();
    idx[1].enums.insert("Counted".into(), counted);
    let q = Query::Enum {
        name: "Counted".into(),
    };
    let l = text(&idx, &q, 100);
    let heads: Vec<&str> = l
        .iter()
        .filter(|x| x.starts_with("enum "))
        .map(String::as_str)
        .collect();
    assert_eq!(heads.len(), 2, "{heads:?}");
    assert!(
        heads[0].starts_with("enum Counted  [vanilla 3_Game"),
        "{heads:?}"
    );
    assert_eq!(heads[1], "enum Counted  [cf]");
}

#[test]
fn chain_walks_and_names_unknown_base() {
    let idx = [vanilla()];
    let q = Query::Chain {
        name: "Zeta".into(),
    };
    assert_eq!(text(&idx, &q, 100), ["Zeta > Base"]);
    let q = Query::Chain {
        name: "Pair".into(),
    };
    assert_eq!(text(&idx, &q, 100), ["Pair > Param"]);
    let q = Query::Chain {
        name: "Variant3".into(),
    };
    assert_eq!(text(&idx, &q, 100), ["Variant3 > Other"]);
    let q = Query::Chain {
        name: "Variant".into(),
    };
    // Variant's base (Base) is reached
    assert_eq!(text(&idx, &q, 100), ["Variant > Base"]);
    let t = tiny(&[("A", Some("Missing"))]);
    let q = Query::Chain { name: "A".into() };
    assert_eq!(text(&t, &q, 100), ["A > Missing"]);
}

#[test]
fn chain_stops_on_cycle() {
    let t = tiny(&[("A", Some("B")), ("B", Some("A"))]);
    let q = Query::Chain { name: "A".into() };
    assert_eq!(text(&t, &q, 100), ["A > B"]);
    let q = class_q("A", true, false);
    assert_eq!(text(&t, &q, 100), ["class A : B", "class B : A"]);
}

#[test]
fn src_lists_every_declaration() {
    let idx = both();
    let q = Query::Src {
        name: "Base".into(),
    };
    let l = text(&idx, &q, 100);
    assert_eq!(l[0], "[vanilla] dta/scripts.pbo scripts\\3_Game\\Base.c:3");
    assert!(
        l.iter()
            .any(|x| x.starts_with("[vanilla] dta/scripts.pbo scripts\\4_World\\Modded.c:")),
        "{l:?}"
    );
    assert!(
        l.iter()
            .any(|x| x.starts_with("[cf] addons/jm_cf.pbo jm\\cf\\scripts\\4_World\\CFWorld.c:")),
        "{l:?}"
    );
    let files: Vec<SrcFile> = src_files(&idx, "Base");
    assert_eq!(files[0].index, "vanilla");
    assert_eq!(files[0].pbo, "dta/scripts.pbo");
    assert_eq!(files[0].prefix, "scripts");
    assert_eq!(files[0].path, "3_Game\\Base.c");
    assert_eq!(files[0].line, 3);
    // enums too
    let q = Query::Src {
        name: "Counted".into(),
    };
    assert_eq!(text(&idx, &q, 100).len(), 2, "one line per declaration");
    assert_eq!(src_files(&idx, "Nope").len(), 0);
}

#[test]
fn defines_filtered_by_regex() {
    let idx = both();
    let q = Query::Defines { regex: None };
    assert_eq!(text(&idx, &q, 100), ["[cf] jm\\cf: CF_FIXTURE CF_MODULE"]);
    let q = Query::Defines {
        regex: Some("module".into()),
    };
    assert_eq!(text(&idx, &q, 100), ["[cf] jm\\cf: CF_MODULE"]);
    let q = Query::Defines {
        regex: Some("zzz".into()),
    };
    assert_eq!(text(&idx, &q, 100), ["no matching define"]);
}

#[test]
fn max_cap_line() {
    let idx = [vanilla()];
    let l = text(&idx, &class_q("Mixed", false, false), 2);
    assert_eq!(
        l,
        [
            "class Mixed : Base",
            "  void Always()",
            "... 4 more lines (raise --max or narrow the query)",
        ]
    );
    assert_eq!(text(&idx, &class_q("Mixed", false, false), 6).len(), 6);
}

#[test]
fn json_output_parses_and_has_members() {
    let idx = [vanilla()];
    let r = run_query(&idx, &class_q("Mixed", false, false)).expect("q");
    let v: serde_json::Value = serde_json::from_str(&render_json(&r, 100)).expect("json");
    assert_eq!(v["truncated"], 0);
    let c = &v["result"]["Classes"][0];
    assert_eq!(c["index"], "vanilla");
    assert_eq!(c["class"]["name"], "Mixed");
    assert_eq!(c["class"]["members"].as_array().map(Vec::len), Some(5));
    assert_eq!(c["class"]["members"][2]["cond"], "!SERVER");
}

#[test]
fn json_truncation_count() {
    let idx = [vanilla()];
    // Mixed: 1 class + 5 members; max 3 keeps the class and 2 members, drops 3.
    let r = run_query(&idx, &class_q("Mixed", false, false)).expect("q");
    let v: serde_json::Value = serde_json::from_str(&render_json(&r, 3)).expect("json");
    assert_eq!(v["truncated"], 3);
    assert_eq!(
        v["result"]["Classes"][0]["class"]["members"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let v: serde_json::Value = serde_json::from_str(&render_json(&r, 0)).expect("json");
    assert_eq!(v["truncated"], 6);
    assert_eq!(v["result"]["Classes"].as_array().map(Vec::len), Some(0));
    // flat results
    let t = tiny(&[
        ("A", None),
        ("B", Some("A")),
        ("C", Some("A")),
        ("D", Some("A")),
    ]);
    let r = run_query(
        &t,
        &Query::Sub {
            name: "A".into(),
            all: false,
        },
    )
    .expect("q");
    let v: serde_json::Value = serde_json::from_str(&render_json(&r, 2)).expect("json");
    assert_eq!(v["truncated"], 1);
    assert_eq!(v["result"]["Names"], serde_json::json!(["B", "C"]));
    let r = run_query(
        &idx,
        &Query::Find {
            regex: "helper".into(),
            class: Some("Mixed".into()),
        },
    )
    .expect("q");
    let v: serde_json::Value = serde_json::from_str(&render_json(&r, 1)).expect("json");
    assert_eq!(
        (
            v["truncated"].as_u64(),
            v["result"]["Matches"].as_array().map(Vec::len)
        ),
        (Some(2), Some(1))
    );
}

#[test]
fn query_not_found_hint() {
    let one = [vanilla()];
    assert_eq!(
        err(&one, &class_q("Nope", false, false)),
        "no class \"Nope\"; try: dayzmake api find Nope\n  (searched vanilla only; add -m <mod> or -m all for mods)"
    );
    let two = both();
    assert_eq!(
        err(&two, &class_q("Nope", false, false)),
        "no class \"Nope\"; try: dayzmake api find Nope"
    );
    assert!(
        err(
            &one,
            &Query::Enum {
                name: "Nope".into()
            }
        )
        .starts_with("no enum \"Nope\"; try: dayzmake api find Nope")
    );
    assert!(
        err(
            &one,
            &Query::Src {
                name: "Nope".into()
            }
        )
        .starts_with("no class or enum \"Nope\"")
    );
    assert!(
        err(
            &one,
            &Query::Chain {
                name: "Nope".into()
            }
        )
        .starts_with("no class \"Nope\"")
    );
    assert!(
        err(
            &one,
            &Query::Sub {
                name: "Nope".into(),
                all: false
            }
        )
        .starts_with("no class \"Nope\"")
    );
    // mods_only on a class that only vanilla has
    assert_eq!(
        err(&two, &class_q("Zeta", false, true)),
        "\"Zeta\" exists in vanilla, but no loaded mod declares it"
    );
    assert_eq!(
        err(&two, &class_q("Other", true, true)),
        "\"Other\" exists in vanilla, but no loaded mod declares it or its base classes"
    );
    assert!(err(&two, &class_q("Nope", true, true)).starts_with("no class \"Nope\""));
}

#[test]
fn find_invalid_regex_is_an_error() {
    let idx = [vanilla()];
    let q = Query::Find {
        regex: "(".into(),
        class: None,
    };
    assert!(run_query(&idx, &q).is_err());
    let q = Query::Defines {
        regex: Some("[".into()),
    };
    assert!(run_query(&idx, &q).is_err());
    // a huge pattern is rejected, not compiled without limit
    let q = Query::Find {
        regex: "(a{1000}){1000}{1000}".into(),
        class: None,
    };
    assert!(run_query(&idx, &q).is_err());
}

#[test]
fn query_serializes() {
    let q = Query::Class {
        name: "A".into(),
        inherited: true,
        mods_only: false,
    };
    let v = serde_json::to_value(&q).expect("json");
    assert_eq!(v["Class"]["name"], "A");
}

#[test]
fn inherited_mods_only_walks_vanilla_bases() {
    let idx = both();
    // Zeta is vanilla only; its base Base has a mod part.
    assert_eq!(
        text(&idx, &class_q("Zeta", true, true), 100),
        ["class Base  [cf 4_World]  [modded]", "  void CFExtra()"]
    );
}

#[test]
fn inherited_walks_across_indexes() {
    let mut idx = both();
    idx[1].classes.get_mut("CFWorld").expect("CFWorld").base = Some("Zeta".into());
    let l = text(&idx, &class_q("CFWorld", true, false), 300);
    let heads: Vec<&str> = l
        .iter()
        .filter(|x| x.starts_with("class "))
        .map(String::as_str)
        .collect();
    assert_eq!(heads.len(), 4, "{heads:?}");
    assert_eq!(heads[0], "class CFWorld : Zeta  [cf 4_World]");
    assert!(
        heads[1].starts_with("class Zeta : Base  [vanilla"),
        "{heads:?}"
    );
    assert!(heads[2].starts_with("class Base  [vanilla"), "{heads:?}");
    assert_eq!(heads[3], "class Base  [cf 4_World]  [modded]");
}

// ---- sources from PBOs, cache and CLI ----

mod from_pbo {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use dayzmake::api::cache::{cached, stamps};
    use dayzmake::api::model::{ApiIndex, INDEX_FORMAT};
    use dayzmake::api::source::{ModSpec, parse_mods, resolve_mod};
    use dayzmake::api::steam::Branch;
    use dayzmake_core::pbo::PBO;

    use super::fixture;

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("mkdir");
        for e in fs::read_dir(from).expect("read_dir") {
            let e = e.expect("entry");
            let target = to.join(e.file_name());
            if e.path().is_dir() {
                copy_dir(&e.path(), &target);
            } else {
                fs::copy(e.path(), target).expect("copy");
            }
        }
    }

    /// Packs `src` (a copy of a fixture tree plus `extra` files) into `out`, with an optional prefix.
    fn make_pbo(src: &Path, extra: &[(&str, &str)], prefix: Option<&str>, out: &Path) {
        let work = tempfile::tempdir().expect("tmp");
        copy_dir(src, work.path());
        for (name, text) in extra {
            let p = work.path().join(name);
            fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            fs::write(p, text).expect("write");
        }
        if let Some(p) = prefix {
            fs::write(work.path().join("$PBOPREFIX$"), p).expect("prefix");
        }
        let pbo = PBO::from_directory(work.path().to_path_buf(), false, &[], &[]).expect("pbo");
        fs::create_dir_all(out.parent().expect("parent")).expect("mkdir");
        let mut f = fs::File::create(out).expect("create");
        pbo.write(&mut f).expect("write pbo");
    }

    fn manifest(root: &Path, app: u32, dir: &str, buildid: u32) {
        fs::create_dir_all(root.join("steamapps")).expect("mkdir");
        fs::write(
            root.join(format!("steamapps/appmanifest_{app}.acf")),
            format!(
                "\"AppState\"\n{{\n\t\"installdir\"\t\t\"{dir}\"\n\t\"buildid\"\t\t\"{buildid}\"\n}}\n"
            ),
        )
        .expect("manifest");
    }

    struct World {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        cache: PathBuf,
        home: PathBuf,
    }

    impl World {
        fn new() -> World {
            let tmp = tempfile::tempdir().expect("tmp");
            let root = tmp.path().join("steam");
            let cache = tmp.path().join("cache");
            let home = tmp.path().join("home");
            fs::create_dir_all(&home).expect("home");
            let van = fixture("vanilla/scripts");
            manifest(&root, 221_100, "DayZ", 111);
            make_pbo(
                &van,
                &[],
                None,
                &root.join("steamapps/common/DayZ/dta/scripts.pbo"),
            );
            manifest(&root, 1_024_020, "DayZ Exp", 222);
            make_pbo(
                &van,
                &[("3_Game/Exp.c", "class OnlyExperimental\n{\n}\n")],
                None,
                &root.join("steamapps/common/DayZ Exp/dta/scripts.pbo"),
            );
            World {
                _tmp: tmp,
                root,
                cache,
                home,
            }
        }

        fn stable_pbo(&self) -> PathBuf {
            self.root.join("steamapps/common/DayZ/dta/scripts.pbo")
        }

        fn cache_file(&self, name: &str) -> PathBuf {
            self.cache.join(name)
        }

        /// Runs the CLI with the environment on the child only.
        fn run(&self, args: &[&str]) -> (i32, String, String) {
            let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
                .args(args)
                .env("STEAM_ROOT", &self.root)
                .env("DAYZMAKE_CACHE", &self.cache)
                .env("HOME", &self.home)
                .env_remove("XDG_CACHE_HOME")
                .current_dir(&self.home)
                .output()
                .expect("run");
            (
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        }

        /// A mod directory with the fixture CF scripts in `addons/jm_cf.pbo`.
        fn cf_mod(&self) -> PathBuf {
            let dir = self.home.join("@CF");
            make_pbo(
                &fixture("mod/JM/CF"),
                &[],
                Some("jm\\cf"),
                &dir.join("addons/jm_cf.pbo"),
            );
            fs::write(dir.join("meta.cpp"), "name = \"Community Framework\";\n").expect("meta");
            dir
        }
    }

    #[test]
    fn api_class_from_pbo_stable_default() {
        let w = World::new();
        let (code, out, err) = w.run(&["api", "class", "Base"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("class Base"), "{out}");
        assert!(!out.contains("OnlyExperimental"));
        let (code, _, err) = w.run(&["api", "class", "OnlyExperimental"]);
        assert_eq!(code, 1);
        assert!(err.contains("OnlyExperimental"), "{err}");
    }

    #[test]
    fn api_experimental_uses_exp_install() {
        let w = World::new();
        let (code, out, err) = w.run(&["api", "class", "OnlyExperimental", "--experimental"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("OnlyExperimental"), "{out}");
        let (code, out, _) = w.run(&["api", "update", "--experimental"]);
        assert_eq!(code, 0);
        assert!(out.contains("vanilla-experimental:") && out.contains("experimental build 222"));
    }

    #[test]
    fn branches_have_separate_caches() {
        let w = World::new();
        assert_eq!(w.run(&["api", "class", "Base"]).0, 0);
        assert_eq!(w.run(&["api", "class", "Base", "--experimental"]).0, 0);
        assert!(w.cache_file("vanilla-stable.json").is_file());
        assert!(w.cache_file("vanilla-experimental.json").is_file());
        let (_, out, _) = w.run(&["api", "update"]);
        assert!(out.starts_with("vanilla-stable: "), "{out}");
        assert!(out.contains("stable build 111"), "{out}");
    }

    #[test]
    fn cache_reused_when_unchanged() {
        let w = World::new();
        assert_eq!(w.run(&["api", "class", "Base"]).0, 0);
        let file = w.cache_file("vanilla-stable.json");
        let first = fs::metadata(&file)
            .expect("meta")
            .modified()
            .expect("mtime");
        assert_eq!(w.run(&["api", "class", "Base"]).0, 0);
        let second = fs::metadata(&file)
            .expect("meta")
            .modified()
            .expect("mtime");
        assert_eq!(first, second);
        // --refresh rebuilds.
        let before = fs::read(&file).expect("read");
        assert_eq!(w.run(&["api", "class", "Base", "--refresh"]).0, 0);
        assert_eq!(before, fs::read(&file).expect("read"));
    }

    #[test]
    fn cache_rebuilds_on_pbo_change() {
        let w = World::new();
        assert_eq!(w.run(&["api", "class", "Base"]).0, 0);
        assert_eq!(w.run(&["api", "class", "Fresh"]).0, 1);
        make_pbo(
            &fixture("vanilla/scripts"),
            &[("3_Game/Fresh.c", "class Fresh\n{\n}\n")],
            None,
            &w.stable_pbo(),
        );
        let (code, out, err) = w.run(&["api", "class", "Fresh"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("Fresh"));
    }

    #[test]
    fn cache_write_is_atomic() {
        let w = World::new();
        fs::create_dir_all(&w.cache).expect("mkdir");
        fs::write(w.cache_file("vanilla-stable.json"), b"{ truncated").expect("corrupt");
        let (code, _, err) = w.run(&["api", "class", "Base"]);
        assert_eq!(code, 0, "{err}");
        let text = fs::read_to_string(w.cache_file("vanilla-stable.json")).expect("read");
        let idx: ApiIndex = serde_json::from_str(&text).expect("valid json again");
        assert!(idx.classes.contains_key("Base"));
        let names: Vec<String> = fs::read_dir(&w.cache)
            .expect("dir")
            .map(|e| e.expect("e").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["vanilla-stable.json".to_string()]);
    }

    #[test]
    fn cache_format_bump_rebuilds() {
        let w = World::new();
        assert_eq!(w.run(&["api", "class", "Base"]).0, 0);
        let file = w.cache_file("vanilla-stable.json");
        let mut idx: ApiIndex =
            serde_json::from_slice(&fs::read(&file).expect("read")).expect("json");
        idx.format = INDEX_FORMAT + 1;
        idx.classes.clear();
        fs::write(&file, serde_json::to_vec(&idx).expect("ser")).expect("write");
        let (code, out, err) = w.run(&["api", "class", "Base"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("class Base"));
        let idx: ApiIndex = serde_json::from_slice(&fs::read(&file).expect("read")).expect("json");
        assert_eq!(idx.format, INDEX_FORMAT);
    }

    #[test]
    fn cache_dir_is_private_and_keys_are_checked() {
        let w = World::new();
        assert_eq!(w.run(&["api", "class", "Base"]).0, 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&w.cache).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let idx = ApiIndex::new("x", "x", "x");
        let r = cached("../evil", &[], false, || Ok(idx));
        assert!(r.is_err());
        assert!(stamps(&[w.cache.join("missing")]).is_err());
    }

    #[test]
    fn api_src_cat_prints_full_file() {
        let w = World::new();
        let (code, out, err) = w.run(&["api", "src", "Base", "--cat"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.starts_with("// ==== [vanilla] dta/scripts.pbo scripts\\3_Game\\Base.c:"));
        assert!(out.contains("private void Secret(int a) { m_Secret = a; }"));
    }

    #[test]
    fn api_src_shows_line_numbers() {
        let w = World::new();
        let (code, out, err) = w.run(&["api", "src", "Base"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("Base.c:"), "{out}");
    }

    #[test]
    fn api_class_json() {
        let w = World::new();
        let (code, out, err) = w.run(&["api", "class", "Base", "--json"]);
        assert_eq!(code, 0, "{err}");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert!(v.to_string().contains("m_Count"));
        let (code, out, err) = w.run(&["api", "class", "Nope", "--json"]);
        assert_eq!(code, 1);
        assert!(out.is_empty() && err.contains("Nope"), "{out} {err}");
    }

    #[test]
    fn api_other_commands_and_verbose() {
        let w = World::new();
        for args in [
            vec!["api", "find", "Secret"],
            vec!["api", "sub", "Base", "--all"],
            vec!["api", "enum", "AIStatus"],
            vec!["api", "chain", "Variant"],
            vec!["api", "defines"],
            vec!["api", "defines", "DAYZ", "--max", "3"],
        ] {
            let (code, _, err) = w.run(&args);
            assert_eq!(code, 0, "{args:?}: {err}");
        }
        let (_, out, _) = w.run(&["api", "class", "Base", "-v"]);
        assert!(out.starts_with("# vanilla: stable build 111\n"), "{out}");
        let (code, _, err) = w.run(&["api", "class", "Base", "--max", "x"]);
        assert_eq!(code, 1);
        assert!(err.contains("--max"), "{err}");
    }

    #[test]
    fn api_mod_by_dir() {
        let w = World::new();
        let dir = w.cf_mod();
        let m = dir.to_string_lossy().into_owned();
        // Without -m only vanilla is queried.
        let (code, _, _) = w.run(&["api", "class", "CFBase"]);
        assert_eq!(code, 1);
        let (code, out, err) = w.run(&["api", "class", "CFBase", "-m", &m]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("CFBase") && out.contains("[_CF"), "{out}");
        let (code, out, err) = w.run(&["api", "src", "CFBase", "-m", &m, "--cat"]);
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains("// ==== [_CF] addons/jm_cf.pbo jm\\cf\\"),
            "{out}"
        );
        assert!(out.contains("void Init();"));
        let (_, out, _) = w.run(&["api", "defines", "-m", &m]);
        assert!(
            out.contains("CF_FIXTURE") && out.contains("jm\\cf"),
            "{out}"
        );
        let (_, out, _) = w.run(&["api", "update", "-m", &m]);
        assert!(out.contains("mod-_CF-stable-"), "{out}");
        // The mod's own defines select the branch of the #ifdef.
        let (_, out, _) = w.run(&["api", "class", "CFSwitch", "-m", &m]);
        assert!(out.contains("CFBase"), "{out}");
    }

    #[test]
    fn api_mod_unsafe_prefix_skipped() {
        let w = World::new();
        let dir = w.cf_mod();
        let evil = tempfile::tempdir().expect("tmp");
        fs::write(evil.path().join("E.c"), "class EvilClass {}\n").expect("w");
        make_pbo(
            evil.path(),
            &[],
            Some("..\\evil"),
            &dir.join("addons/evil.pbo"),
        );
        let m = dir.to_string_lossy().into_owned();
        let (code, out, err) = w.run(&["api", "class", "CFBase", "-m", &m]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("CFBase"));
        assert!(err.contains("unsafe prefix"), "{err}");
        let (code, _, _) = w.run(&["api", "class", "EvilClass", "-m", &m]);
        assert_eq!(code, 1);
        // Nothing was written outside the cache dir and the mod dir.
        assert!(!w.home.join("evil").exists() && !dir.join("evil").exists());
        assert!(!w.home.join(".cache").exists());
    }

    #[cfg(unix)]
    #[test]
    fn query_works_when_the_cache_is_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let w = World::new();
        fs::create_dir_all(&w.cache).expect("cache");
        fs::set_permissions(&w.cache, fs::Permissions::from_mode(0o500)).expect("chmod");
        // root (e.g. in a CI container) ignores the mode; the test needs a cache it cannot write.
        if fs::write(w.cache.join("probe"), "").is_ok() {
            fs::set_permissions(&w.cache, fs::Permissions::from_mode(0o700)).expect("chmod");
            return;
        }
        let (code, out, err) = w.run(&["api", "class", "Base"]);
        fs::set_permissions(&w.cache, fs::Permissions::from_mode(0o700)).expect("chmod");
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("class Base"), "{out}");
        assert!(err.contains("api-cache"), "{err}");
    }

    #[test]
    fn query_works_without_a_cache_location() {
        let w = World::new();
        let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
            .args(["api", "class", "Base"])
            .env("STEAM_ROOT", &w.root)
            .env_remove("DAYZMAKE_CACHE")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("HOME")
            .env_remove("LOCALAPPDATA")
            .current_dir(&w.home)
            .output()
            .expect("run");
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(0), "{err}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("class Base"),
            "{err}"
        );
        assert!(err.contains("api-cache"), "{err}");
    }

    #[test]
    fn api_missing_install_message() {
        let w = World::new();
        let empty = w.home.join("emptysteam");
        fs::create_dir_all(empty.join("steamapps")).expect("mkdir");
        let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
            .args(["api", "class", "Base", "--experimental"])
            .env("STEAM_ROOT", &empty)
            .env("DAYZMAKE_CACHE", &w.cache)
            .env("HOME", &w.home)
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(1));
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("experimental") && err.contains("--dayz"),
            "{err}"
        );
        let (code, _, err) = w.run(&["api", "class", "Base", "--dayz", "/nonexistent"]);
        assert_eq!(code, 1);
        assert!(err.contains("--dayz"), "{err}");
    }

    #[test]
    fn mod_list_parsing_and_resolution() {
        assert_eq!(
            parse_mods("cf").expect("cf"),
            vec![ModSpec::Alias("cf", 1_559_212_036)]
        );
        let all = parse_mods("aliases,CF").expect("aliases");
        assert_eq!(all.len(), 4);
        assert_eq!(
            parse_mods("all,CF").expect("all"),
            vec![ModSpec::All, ModSpec::Alias("cf", 1_559_212_036)]
        );
        assert_eq!(
            parse_mods("123,some/dir").expect("list"),
            vec![ModSpec::Workshop(123), ModSpec::Dir("some/dir".into())]
        );
        assert!(parse_mods("").is_err() && parse_mods("cf,").is_err());
        assert!(parse_mods("99999999999999999999999").is_err());
        let w = World::new();
        assert!(resolve_mod(&ModSpec::Dir(w.home.join("nope")), Branch::Stable, &[]).is_err());
        let err = resolve_mod(&ModSpec::Alias("cf", 1), Branch::Stable, &[]).expect_err("missing");
        assert!(err.to_string().contains("not installed"), "{err}");
        assert!(resolve_mod(&ModSpec::Workshop(7), Branch::Stable, &[]).is_err());
        let file = w.home.join("file");
        fs::write(&file, "x").expect("w");
        assert!(resolve_mod(&ModSpec::Dir(file), Branch::Stable, &[]).is_err());
        let dir = w.cf_mod();
        let m = resolve_mod(&ModSpec::Dir(dir), Branch::Stable, &[]).expect("dir");
        assert_eq!(m.name, "_CF");
        assert_eq!(m.pbos.len(), 1);
    }

    #[test]
    fn workshop_mods_and_all() {
        let w = World::new();
        let ws = w.root.join("steamapps/workshop/content/221100");
        make_pbo(
            &fixture("mod/JM/CF"),
            &[],
            Some("jm\\cf"),
            &ws.join("1559212036/Addons/jm_cf.pbo"),
        );
        let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
            .args(["api", "class", "CFBase", "-m", "aliases", "-v"])
            .env("STEAM_ROOT", &w.root)
            .env("DAYZMAKE_CACHE", &w.cache)
            .env("HOME", &w.home)
            .output()
            .expect("run");
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(text.contains("# cf:") && !text.contains("# cot:"), "{text}");
        // Naming an alias that is not installed is an error.
        let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
            .args(["api", "class", "Base", "-m", "cot"])
            .env("STEAM_ROOT", &w.root)
            .env("DAYZMAKE_CACHE", &w.cache)
            .env("HOME", &w.home)
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("not installed"));
    }

    #[test]
    fn api_src_cat_json() {
        let w = World::new();
        let (code, out, err) = w.run(&["api", "src", "Base", "--cat", "--json"]);
        assert_eq!(code, 0, "{err}");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        let files = v["files"].as_array().expect("files");
        assert_eq!(files[0]["index"], "vanilla");
        assert!(files[0]["line"].is_number());
        assert!(
            files[0]["text"]
                .as_str()
                .expect("text")
                .contains("private void Secret(int a) { m_Secret = a; }")
        );
    }

    #[test]
    fn cache_version_change_rebuilds() {
        let w = World::new();
        assert_eq!(w.run(&["api", "class", "Base"]).0, 0);
        let file = w.cache_file("vanilla-stable.json");
        let mut v: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).expect("read")).expect("json");
        v["version"] = serde_json::json!("0.0.0-old");
        v["classes"] = serde_json::json!({});
        fs::write(&file, serde_json::to_vec(&v).expect("ser")).expect("write");
        let (code, out, err) = w.run(&["api", "class", "Base"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("class Base"));
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).expect("read")).expect("json");
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    }

    fn one_class_mod(dir: &Path, class: &str) {
        let src = tempfile::tempdir().expect("tmp");
        fs::write(
            src.path().join("X.c"),
            format!("class {class}\n{{\n\tvoid Only{class}();\n}}\n"),
        )
        .expect("w");
        make_pbo(src.path(), &[], Some("m\\x"), &dir.join("addons/x.pbo"));
    }

    #[test]
    fn mod_named_vanilla_is_renamed() {
        let w = World::new();
        let dir = w.home.join("vanilla");
        one_class_mod(&dir, "FromMod");
        let m = dir.to_string_lossy().into_owned();
        let (code, out, err) = w.run(&["api", "src", "FromMod", "-m", &m, "--cat", "-v"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("// ==== [vanilla-2] addons/x.pbo"), "{out}");
        assert!(out.contains("OnlyFromMod"), "{out}");
        let (_, out, _) = w.run(&["api", "class", "FromMod", "-m", &m, "-v"]);
        assert!(
            out.contains("# vanilla: ") && out.contains("# vanilla-2: "),
            "{out}"
        );
    }

    #[test]
    fn duplicate_mod_names_get_suffix() {
        let w = World::new();
        let a = w.home.join("a/@M");
        let b = w.home.join("b/@M");
        one_class_mod(&a, "ClassA");
        one_class_mod(&b, "ClassB");
        let list = format!("{},{}", a.display(), b.display());
        let (code, out, err) = w.run(&["api", "class", "ClassB", "-m", &list, "-v"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("# _M: ") && out.contains("# _M-2: "), "{out}");
        let (code, out, err) = w.run(&["api", "src", "ClassA", "-m", &list, "--cat"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("[_M] ") && out.contains("OnlyClassA"), "{out}");
        let (code, out, err) = w.run(&["api", "src", "ClassB", "-m", &list, "--cat"]);
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains("[_M-2] ") && out.contains("OnlyClassB"),
            "{out}"
        );
        assert!(!out.contains("OnlyClassA"));
    }

    // ---- installed mods ----

    const WS: &str = "steamapps/workshop/content/221100";

    /// A workshop mod `id` with one script class `class` and a `meta.cpp` name.
    fn ws_scripted(w: &World, id: u64, class: &str, name: &str) -> PathBuf {
        let dir = w.root.join(WS).join(id.to_string());
        one_class_mod(&dir, class);
        fs::write(dir.join("meta.cpp"), format!("name = \"{name}\";\n")).expect("meta");
        dir
    }

    /// A workshop mod `id` with a texture-only PBO.
    fn ws_textures(w: &World, id: u64, name: &str) -> PathBuf {
        let dir = w.root.join(WS).join(id.to_string());
        let src = tempfile::tempdir().expect("tmp");
        fs::write(src.path().join("a.paa"), "x").expect("w");
        make_pbo(src.path(), &[], Some("t\\x"), &dir.join("addons/t.pbo"));
        fs::write(dir.join("meta.cpp"), format!("name = \"{name}\";\n")).expect("meta");
        dir
    }

    fn installed(w: &World) -> Vec<dayzmake::api::source::InstalledMod> {
        let game = w.root.join("steamapps/common/DayZ");
        dayzmake::api::source::installed_mods(
            Branch::Stable,
            std::slice::from_ref(&w.root),
            Some(&game),
        )
    }

    /// Three workshop mods (100 and 200 with scripts, 300 textures only) and an `@Link` to 200.
    fn three_mods(w: &World) {
        ws_scripted(w, 100, "ClassOne", "Alpha Mod");
        ws_scripted(w, 200, "ClassTwo", "Beta Mod");
        ws_textures(w, 300, "Gamma Textures");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            w.root.join(WS).join("200"),
            w.root.join("steamapps/common/DayZ/@Link"),
        )
        .expect("symlink");
    }

    #[test]
    fn installed_mods_dedups_links() {
        let w = World::new();
        three_mods(&w);
        let mods = installed(&w);
        assert_eq!(mods.len(), 3, "{mods:?}");
        assert_eq!(
            mods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["Alpha Mod", "Beta Mod", "Gamma Textures"]
        );
        assert_eq!(mods[1].id, Some(200));
    }

    #[test]
    fn installed_mods_flags_scripts() {
        let w = World::new();
        three_mods(&w);
        let flags: Vec<bool> = installed(&w).iter().map(|m| m.has_scripts).collect();
        assert_eq!(flags, [true, true, false]);
    }

    #[test]
    fn installed_mods_in_game_dir_without_workshop_id() {
        let w = World::new();
        let dir = w.root.join("steamapps/common/DayZ/@Local");
        one_class_mod(&dir, "LocalClass");
        let mods = installed(&w);
        assert_eq!(mods.len(), 1);
        assert_eq!((mods[0].id, mods[0].name.as_str()), (None, "@Local"));
        assert!(mods[0].has_scripts);
    }

    #[test]
    fn mods_all_queries_every_scripted_mod() {
        let w = World::new();
        three_mods(&w);
        let (code, out, err) = w.run(&["api", "find", "^Class(One|Two)$", "-m", "all", "-v"]);
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains("class ClassOne") && out.contains("class ClassTwo"),
            "{out}"
        );
        assert!(!out.contains("Gamma"), "{out}");
        // Without -m only vanilla is queried.
        let (code, out, err) = w.run(&["api", "find", "^Class(One|Two)$"]);
        assert_eq!(code, 0, "{err}");
        assert!(!out.contains("class ClassOne"), "{out}");
    }

    #[test]
    fn mods_by_name_case_insensitive() {
        let w = World::new();
        three_mods(&w);
        let (code, out, err) = w.run(&["api", "class", "ClassOne", "-m", "ALPHA mod"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("class ClassOne"), "{out}");
        // three_mods creates the @Link symlink on Unix only.
        if cfg!(unix) {
            let (code, out, err) = w.run(&["api", "class", "ClassTwo", "-m", "@link"]);
            assert_eq!(code, 0, "{err}");
            assert!(out.contains("class ClassTwo"), "{out}");
        }
    }

    #[test]
    fn mods_ambiguous_name_is_an_error() {
        let w = World::new();
        three_mods(&w);
        let (code, _, err) = w.run(&["api", "class", "ClassOne", "-m", "mod"]);
        assert_eq!(code, 1);
        assert!(
            err.contains("more than one") && err.contains("Alpha Mod") && err.contains("Beta Mod"),
            "{err}"
        );
        let (code, _, err) = w.run(&["api", "class", "ClassOne", "-m", "nonexistent-thing"]);
        assert_eq!(code, 1);
        assert!(err.contains("no installed mod"), "{err}");
    }

    #[test]
    fn mods_aliases_keeps_old_all() {
        let w = World::new();
        let ws = w.root.join(WS);
        make_pbo(
            &fixture("mod/JM/CF"),
            &[],
            Some("jm\\cf"),
            &ws.join("1559212036/Addons/jm_cf.pbo"),
        );
        ws_scripted(&w, 100, "ClassOne", "Alpha Mod");
        let (code, out, err) = w.run(&["api", "class", "CFBase", "-m", "aliases", "-v"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("# cf:") && !out.contains("# alpha"), "{out}");
        // `all` names the alias mod `cf` too.
        let (code, out, err) = w.run(&["api", "class", "CFBase", "-m", "all", "-v"]);
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains("# cf:") && out.matches("# ").count() == 3,
            "{out}"
        );
    }

    #[test]
    fn broken_mod_is_skipped_with_warning() {
        let w = World::new();
        three_mods(&w);
        let bad = w.root.join(WS).join("400");
        fs::create_dir_all(bad.join("addons")).expect("mkdir");
        fs::write(bad.join("addons/garbage.pbo"), b"not a pbo at all").expect("w");
        let (code, out, err) = w.run(&["api", "find", "^Class(One|Two)$", "-m", "all"]);
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains("class ClassOne") && out.contains("class ClassTwo"),
            "{out}"
        );
        assert!(err.contains("garbage.pbo"), "{err}");
        // An explicit selection of the broken mod still answers, without it.
        let (code, _, err) = w.run(&["api", "class", "ClassOne", "-m", "400,100"]);
        assert_eq!(code, 0, "{err}");
    }

    #[test]
    fn texture_pbos_are_not_read_completely() {
        let w = World::new();
        // A PBO whose header says the entry has 4 GiB; a full read would fail with a truncation error.
        let dir = w.root.join(WS).join("500/addons");
        fs::create_dir_all(&dir).expect("mkdir");
        let mut bytes: Vec<u8> = Vec::new();
        // version header: empty name, "Vers", four u32, then an empty extension list
        bytes.push(0);
        bytes.extend_from_slice(&0x5665_7273u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 16]);
        bytes.push(0);
        // the entry: name, method, original size, reserved, timestamp, data size
        bytes.extend_from_slice(b"tex.paa\0");
        bytes.extend_from_slice(&[0; 16]);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        // the terminating header, and no data at all
        bytes.push(0);
        bytes.extend_from_slice(&[0; 20]);
        fs::write(dir.join("tex.pbo"), bytes).expect("w");
        ws_scripted(&w, 100, "ClassOne", "Alpha Mod");
        let (code, out, err) = w.run(&["api", "find", "^Class(One|Two)$", "-m", "500,100"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("class ClassOne"), "{out}");
        assert!(!err.contains("tex.pbo"), "{err}");
    }

    #[test]
    fn index_name_clash_gets_id_suffix() {
        let w = World::new();
        ws_scripted(&w, 100, "ClassOne", "Same Name");
        ws_scripted(&w, 200, "ClassTwo", "Same Name");
        let (code, out, err) = w.run(&["api", "find", "^Class(One|Two)$", "-m", "all", "-v"]);
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains("# Same_Name: ") && out.contains("# Same_Name-200: "),
            "{out}"
        );
    }

    #[test]
    fn cli_api_mods_lists() {
        let w = World::new();
        three_mods(&w);
        let (code, out, err) = w.run(&["api", "mods"]);
        assert_eq!(code, 0, "{err}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "{out}");
        assert!(
            lines[0].starts_with("100  Alpha Mod  ") && lines[0].ends_with("  scripts"),
            "{out}"
        );
        assert!(
            lines[2].starts_with("300  Gamma Textures  ") && !lines[2].ends_with("scripts"),
            "{out}"
        );
    }

    #[test]
    fn cli_api_mods_text_escapes_the_mod_name() {
        let w = World::new();
        ws_scripted(&w, 100, "ClassOne", "Evil \u{1b}[31mMod");
        let (code, out, err) = w.run(&["api", "mods"]);
        assert_eq!(code, 0, "{err}");
        assert!(!out.contains('\u{1b}'), "{out:?}");
        assert!(out.contains("Evil \\u{1b}[31mMod"), "{out:?}");
        let (_, json, _) = w.run(&["api", "mods", "--json"]);
        assert!(json.contains("\\u001b[31m"), "{json}");
    }

    #[test]
    fn cli_api_mods_json() {
        let w = World::new();
        three_mods(&w);
        let (code, out, err) = w.run(&["api", "mods", "--json"]);
        assert_eq!(code, 0, "{err}");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        let mods = v["mods"].as_array().expect("mods");
        assert_eq!(mods.len(), 3);
        assert_eq!(mods[0]["id"], 100);
        assert_eq!(mods[0]["has_scripts"], true);
        assert_eq!(mods[2]["has_scripts"], false);
    }
}
