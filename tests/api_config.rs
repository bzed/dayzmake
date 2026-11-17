//! `dayzmake api config`: lookup of config classes with their inheritance chain and resolved properties.
#![cfg_attr(
    unix,
    expect(clippy::expect_used, reason = "a failing expect is a failing test")
)]
// the CLI is run with unix-style environment variables and paths only
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dayzmake_core::config::Config;
use dayzmake_core::warning::Warnings;
use serde_json::Value;

#[path = "../crates/dayzmake-core/tests/support/mod.rs"]
mod support;

/// A PBO whose only entry is the rapified `text` as `config.bin` (or `config.cpp` verbatim when `bin` is off).
fn config_pbo(text: &str, bin: bool) -> Vec<u8> {
    if bin {
        let config = Config::from_string(text.to_string(), None, &[], &mut Warnings::new())
            .expect("config parses");
        let bytes = config.to_cursor().expect("rapify").into_inner();
        support::raw_pbo(&[("config.bin", &bytes)], &[("prefix", "x")])
    } else {
        support::raw_pbo(&[("config.cpp", text.as_bytes())], &[("prefix", "x")])
    }
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, bytes).expect("write");
}

/// Addon A, stored under a name that sorts after B's.
const ADDON_A: &str = r#"
class CfgPatches { class A_Patch { requiredAddons[] = {}; }; };
class CfgVehicles {
    class Base {
        displayName = "Base";
        scope = 0;
        tags[] = {"a", "b"};
        class Inner { x = 1; };
    };
    class Hatch: Base {
        displayName = "Hatch";
        scope = 2;
        tags[] += {"hatch"};
        class Inner: Inner { y = 2; };
    };
    class Other: Base { };
    class Doomed { v = 1; };
};
"#;

/// Addon B requires A, sorts before it, and modifies it.
const ADDON_B: &str = r#"
class CfgPatches { class B_Patch { requiredAddons[] = {"A_Patch"}; }; };
class CfgVehicles {
    class Base;
    class Hatch: Base {
        displayName = "Hatch B";
        tags[] += {"b_tag"};
        extra = 5;
    };
    class Newcar: Hatch { scope = 2; };
    delete Doomed;
};
"#;

const ADDON_MOD: &str = r#"
class CfgPatches { class M_Patch { requiredAddons[] = {"B_Patch"}; }; };
class CfgVehicles {
    class Hatch;
    class Modcar: Hatch { displayName = "Modcar"; };
    class Hatch2: Hatch { displayName = "Hatch2"; extra = 9; };
};
"#;

struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    cache: PathBuf,
    home: PathBuf,
    game: PathBuf,
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("steam");
        let cache = tmp.path().join("cache");
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(root.join("steamapps")).expect("mkdir");
        fs::write(
            root.join("steamapps/appmanifest_221100.acf"),
            "\"AppState\"\n{\n\t\"installdir\"\t\t\"DayZ\"\n\t\"buildid\"\t\t\"111\"\n}\n",
        )
        .expect("manifest");
        let game = root.join("steamapps/common/DayZ");
        write(
            &game.join("dta/scripts.pbo"),
            &support::raw_pbo(
                &[("3_Game/a.c", b"class Base\n{\n}\n")],
                &[("prefix", "scripts")],
            ),
        );
        write(&game.join("Addons/zz_a.pbo"), &config_pbo(ADDON_A, true));
        write(&game.join("Addons/aa_b.pbo"), &config_pbo(ADDON_B, true));
        World {
            _tmp: tmp,
            root,
            cache,
            home,
            game,
        }
    }

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

    fn mod_dir(&self) -> String {
        let dir = self.home.join("@Mod");
        write(&dir.join("addons/m.pbo"), &config_pbo(ADDON_MOD, false));
        dir.to_string_lossy().into_owned()
    }
}

fn json(w: &World, path: &str, extra: &[&str]) -> Value {
    let mut args = vec!["api", "config", "--json"];
    args.extend_from_slice(extra);
    args.push(path);
    let (code, out, err) = w.run(&args);
    assert_eq!(code, 0, "{err}");
    serde_json::from_str(&out).expect("json")
}

fn prop<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["properties"]
        .as_array()
        .expect("properties")
        .iter()
        .find(|p| p["name"] == name)
        .expect("property present")
}

#[test]
fn chain_properties_and_origins_after_the_load_order_merge() {
    let w = World::new();
    let (code, out, err) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Hatch : Base"), "{out}");
    // B (aa_b.pbo) sorts first but requires A: its displayName wins
    assert!(out.contains("displayName = \"Hatch B\""), "{out}");
    assert!(out.contains("scope = 2"), "{out}");
    assert!(out.contains("extra = 5"), "{out}");
    // the array grows through inheritance and both += operations
    assert!(
        out.contains("tags = {\"a\", \"b\", \"hatch\", \"b_tag\"}"),
        "{out}"
    );
    assert!(out.contains("Newcar"), "subclass listed: {out}");
}

#[test]
fn json_has_chain_origins_subclasses_and_pbos() {
    let w = World::new();
    let v = json(&w, "cfgvehicles/HATCH", &[]);
    assert_eq!(v["path"], "CfgVehicles/Hatch");
    assert_eq!(v["name"], "Hatch");
    let chain: Vec<&str> = v["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    assert_eq!(chain, ["Hatch", "Base"]);
    assert_eq!(v["chain"][1]["path"], "CfgVehicles/Base");
    assert_eq!(prop(&v, "displayName")["value"], "Hatch B");
    assert_eq!(prop(&v, "displayName")["origin"], "Hatch");
    assert_eq!(prop(&v, "scope")["value"], 2);
    assert_eq!(
        prop(&v, "tags")["value"],
        serde_json::json!(["a", "b", "hatch", "b_tag"])
    );
    assert_eq!(prop(&v, "tags")["origin"], "Hatch");
    // an inherited property is tagged with the class it comes from
    let base = json(&w, "CfgVehicles/Other", &[]);
    assert_eq!(prop(&base, "displayName")["value"], "Base");
    assert_eq!(prop(&base, "displayName")["origin"], "Base");
    assert_eq!(
        prop(&base, "displayName")["origin_path"],
        "CfgVehicles/Base"
    );
    let subs: Vec<&str> = v["subclasses"]
        .as_array()
        .expect("subclasses")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    assert_eq!(subs, ["Newcar"]);
    let pbos: Vec<String> = v["pbos"]
        .as_array()
        .expect("pbos")
        .iter()
        .map(|p| format!("{}:{}", p["source"], p["pbo"]))
        .collect();
    // load order: A first, then B
    assert_eq!(
        pbos,
        [
            "\"vanilla\":\"Addons/zz_a.pbo\"",
            "\"vanilla\":\"Addons/aa_b.pbo\""
        ]
    );
}

#[test]
fn nested_classes_are_listed_and_configs_in_folders_are_read() {
    let w = World::new();
    let v = json(&w, "CfgVehicles/Hatch", &[]);
    assert_eq!(v["classes"][0]["name"], "Inner");
    let top = json(&w, "CfgVehicles", &["--max", "100"]);
    let names: Vec<&str> = top["classes"]
        .as_array()
        .expect("classes")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["Base", "Hatch", "Newcar", "Other"]);
    // a config in a folder of the PBO, as the weapon PBOs of the game have them
    let text = "class CfgVehicles { class InFolder { v = 3; }; };";
    let bytes = Config::from_string(text.to_string(), None, &[], &mut Warnings::new())
        .expect("config")
        .to_cursor()
        .expect("rapify")
        .into_inner();
    write(
        &w.game.join("Addons/folders.pbo"),
        &support::raw_pbo(&[("model\\config.bin", &bytes)], &[]),
    );
    let v = json(&w, "CfgVehicles/InFolder", &[]);
    assert_eq!(prop(&v, "v")["value"], 3);
}

#[test]
fn nested_class_inherits_from_the_inherited_scope() {
    let w = World::new();
    let v = json(&w, "CfgVehicles/Hatch/Inner", &[]);
    let chain: Vec<&str> = v["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .map(|c| c["path"].as_str().expect("path"))
        .collect();
    assert_eq!(chain, ["CfgVehicles/Hatch/Inner", "CfgVehicles/Base/Inner"]);
    assert_eq!(prop(&v, "x")["origin"], "Inner");
    assert_eq!(prop(&v, "y")["value"], 2);
}

#[test]
fn delete_removes_the_class_and_a_missing_path_is_a_clear_error() {
    let w = World::new();
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/Doomed"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("no config class \"CfgVehicles/Doomed\""),
        "{err}"
    );
    let (code, _, err) = w.run(&["api", "config", "CfgNope/X"]);
    assert_eq!(code, 1);
    assert!(err.contains("CfgNope/X"), "{err}");
    let (code, _, err) = w.run(&["api", "config", "/"]);
    assert_eq!(code, 1, "{err}");
}

#[test]
fn an_inheritance_cycle_is_an_error() {
    let w = World::new();
    write(
        &w.game.join("Addons/cyc.pbo"),
        &config_pbo(
            "class CfgVehicles { class X: Y {}; class Y: X {}; class Z: Z {}; };",
            true,
        ),
    );
    for name in ["X", "Y", "Z"] {
        let (code, _, err) = w.run(&["api", "config", &format!("CfgVehicles/{name}")]);
        assert_eq!(code, 1, "{name}: {err}");
        assert!(err.contains("inheritance cycle"), "{name}: {err}");
    }
    // the other classes are unaffected
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn an_unknown_base_is_reported_not_an_error() {
    let w = World::new();
    write(
        &w.game.join("Addons/lost.pbo"),
        &config_pbo(
            "class CfgVehicles { class Lost: Missing { a = 1; }; };",
            true,
        ),
    );
    let v = json(&w, "CfgVehicles/Lost", &[]);
    assert_eq!(v["chain_end"], "unknown base Missing");
    assert_eq!(prop(&v, "a")["value"], 1);
    let (_, out, _) = w.run(&["api", "config", "CfgVehicles/Lost"]);
    assert!(out.contains("Lost : Missing (not found)"), "{out}");
}

#[test]
fn mods_merge_after_vanilla_in_order() {
    let w = World::new();
    let m = w.mod_dir();
    let v = json(&w, "CfgVehicles/Modcar", &["-m", &m]);
    assert_eq!(prop(&v, "displayName")["value"], "Modcar");
    // vanilla alone has no such class
    let (code, _, _) = w.run(&["api", "config", "CfgVehicles/Modcar"]);
    assert_eq!(code, 1);
    // the mod's `class Hatch;` forward declaration changes nothing, so Hatch keeps its own content
    let hatch = json(&w, "CfgVehicles/Hatch", &["-m", &m]);
    assert_eq!(prop(&hatch, "displayName")["value"], "Hatch B");
    let subs: Vec<&str> = hatch["subclasses"]
        .as_array()
        .expect("subclasses")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    assert_eq!(subs, ["Hatch2", "Modcar", "Newcar"]);
    let hatch2 = json(&w, "CfgVehicles/Hatch2", &["-m", &m]);
    assert_eq!(prop(&hatch2, "extra")["value"], 9);
    assert_eq!(prop(&hatch2, "extra")["origin"], "Hatch2");
    assert_eq!(prop(&hatch2, "scope")["origin"], "Hatch");
    assert!(
        hatch2["pbos"]
            .as_array()
            .expect("pbos")
            .iter()
            .any(|p| p["source"] != "vanilla")
    );
}

#[test]
fn the_index_is_cached_and_refresh_rebuilds_it() {
    let w = World::new();
    let (code, first, err) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    let cache_file = w.cache.join("config-vanilla-stable.json");
    assert!(
        cache_file.is_file(),
        "{:?}",
        fs::read_dir(&w.cache).map(Iterator::count)
    );
    // a hand-edited cache file proves the second run reads it instead of the PBOs
    let text = fs::read_to_string(&cache_file).expect("read");
    assert!(text.contains("Hatch B"), "{text}");
    fs::write(&cache_file, text.replace("Hatch B", "Hatch X")).expect("write");
    let (_, second, _) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert!(
        second.contains("Hatch X") && !first.contains("Hatch X"),
        "{second}"
    );
    let (code, third, err) = w.run(&["api", "config", "--refresh", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(first, third);
    // a changed PBO invalidates the cache without --refresh
    write(
        &w.game.join("Addons/extra.pbo"),
        &config_pbo("class CfgVehicles { class Fresh {}; };", true),
    );
    let (code, out, err) = w.run(&["api", "config", "CfgVehicles/Fresh"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Fresh"), "{out}");
}

#[test]
fn max_limits_the_lines_and_the_json_items() {
    let w = World::new();
    let (code, out, err) = w.run(&["api", "config", "--max", "3", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.lines().count(), 4, "{out}");
    assert!(
        out.lines()
            .last()
            .expect("line")
            .contains("more lines (raise --max"),
        "{out}"
    );
    let v = json(&w, "CfgVehicles/Hatch", &["--max", "1"]);
    assert_eq!(v["properties"].as_array().expect("properties").len(), 1);
    assert!(v["truncated"].as_u64().expect("truncated") > 0);
}

#[test]
fn config_cpp_is_indexed_and_a_broken_config_is_a_warning() {
    let w = World::new();
    write(
        &w.game.join("Addons/text.pbo"),
        &config_pbo("class CfgVehicles { class FromText { v = 1; }; };", false),
    );
    write(
        &w.game.join("Addons/broken.pbo"),
        &support::raw_pbo(&[("config.bin", b"\0raPgarbage")], &[]),
    );
    let (code, out, err) = w.run(&["api", "config", "CfgVehicles/FromText"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("v = 1"), "{out}");
    assert!(err.contains("api-bad-config"), "{err}");
}

#[test]
fn a_required_addons_cycle_still_loads_everything() {
    let w = World::new();
    write(
        &w.game.join("Addons/c1.pbo"),
        &config_pbo(
            "class CfgPatches { class C1 { requiredAddons[] = {\"C2\"}; }; }; class CfgVehicles { class C1Car { a = 1; }; };",
            true,
        ),
    );
    write(
        &w.game.join("Addons/c2.pbo"),
        &config_pbo(
            "class CfgPatches { class C2 { requiredAddons[] = {\"C1\"}; }; }; class CfgVehicles { class C2Car { a = 2; }; };",
            true,
        ),
    );
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/C2Car"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("api-config-order"), "{err}");
}

fn base_world(extra_b: &str) -> World {
    let w = World::new();
    write(
        &w.game.join("Addons/rebase.pbo"),
        &config_pbo(
            &format!(
                "class CfgPatches {{ class R_Patch {{ requiredAddons[] = {{\"B_Patch\"}}; }}; }};\n\
                 class CfgVehicles {{ {extra_b} }};"
            ),
            true,
        ),
    );
    w
}

#[test]
fn a_redefinition_without_a_base_drops_it_and_records_the_change() {
    let w = base_world("class Hatch { displayName = \"Cut\"; };");
    // the first run builds the index; a change made by vanilla is shown but is no warning
    let (code, out, err) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("api-config-base-changed"), "{err}");
    let v = json(&w, "CfgVehicles/Hatch", &[]);
    assert_eq!(v["chain"].as_array().expect("chain").len(), 1);
    assert_eq!(v["chain_end"], "root");
    assert_eq!(v["base_changes"][0]["from"], "Base");
    assert!(v["base_changes"][0]["to"].is_null());
    assert_eq!(v["base_changes"][0]["pbo"], "Addons/rebase.pbo");
    // nothing inherited is left
    assert!(
        v["properties"]
            .as_array()
            .expect("p")
            .iter()
            .all(|p| p["name"] != "tags" || p["origin"] == "Hatch")
    );
    assert!(
        out.contains("Base changed: Base -> (none) by [vanilla] Addons/rebase.pbo"),
        "{out}"
    );
}

#[test]
fn a_redefinition_with_another_base_is_recorded_and_with_the_same_base_is_not() {
    let w = base_world("class Hatch: Other { };");
    let v = json(&w, "CfgVehicles/Hatch", &[]);
    assert_eq!(v["base_changes"][0]["from"], "Base");
    assert_eq!(v["base_changes"][0]["to"], "Other");
    let w = base_world("class Hatch: BASE { x = 1; };");
    let v = json(&w, "CfgVehicles/Hatch", &[]);
    assert_eq!(v["base_changes"].as_array().expect("changes").len(), 0);
    let (_, out, err) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert!(
        !out.contains("Base changed") && !err.contains("base-changed"),
        "{out}{err}"
    );
}

#[test]
fn an_extern_declaration_changes_nothing() {
    let w = base_world("class Hatch;");
    let v = json(&w, "CfgVehicles/Hatch", &[]);
    assert_eq!(v["chain"].as_array().expect("chain").len(), 2);
    assert_eq!(v["base_changes"].as_array().expect("changes").len(), 0);
}

#[test]
fn a_mod_that_drops_the_base_is_reported_across_sources() {
    let w = World::new();
    let dir = w.home.join("@Cut");
    write(
        &dir.join("addons/c.pbo"),
        &config_pbo(
            "class CfgPatches { class C_Patch { requiredAddons[] = {\"B_Patch\"}; }; }; class CfgVehicles { class Hatch { y = 1; }; };",
            false,
        ),
    );
    let m = dir.to_string_lossy().into_owned();
    // a mod's change warns (the sources are merged on every call)
    let (code, _, err) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("api-config-base-changed"), "{err}");
    let v = json(&w, "CfgVehicles/Hatch", &["-m", &m]);
    assert_eq!(v["chain_end"], "root");
    assert_eq!(v["chain"].as_array().expect("chain").len(), 1);
    assert_eq!(v["base_changes"][0]["from"], "Base");
    assert_ne!(v["base_changes"][0]["source"], "vanilla");
}

#[test]
fn a_base_change_inside_one_mod_warns_when_its_index_is_built() {
    let w = World::new();
    let dir = w.home.join("@Own");
    write(
        &dir.join("addons/o.pbo"),
        &config_pbo(
            "class CfgVehicles { class P {}; class Q: P {}; class Q {}; };",
            true,
        ),
    );
    let m = dir.to_string_lossy().into_owned();
    let (code, out, err) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Q"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("api-config-base-changed"), "{err}");
    assert!(out.contains("Base changed: P -> (none)"), "{out}");
}

/// A mod with two PBOs that redefine `Hatch` one after the other; p2 requires p1.
fn two_pbo_mod(w: &World, first: &str, second: &str) -> String {
    let dir = w.home.join("@Two");
    let patch = |name: &str, req: &str, body: &str| {
        format!(
            "class CfgPatches {{ class {name} {{ requiredAddons[] = {{\"{req}\"}}; }}; }};\n\
             class CfgVehicles {{ class X {{}}; {body} }};"
        )
    };
    write(
        &dir.join("addons/p1.pbo"),
        &config_pbo(&patch("T1", "B_Patch", first), true),
    );
    write(
        &dir.join("addons/p2.pbo"),
        &config_pbo(&patch("T2", "T1", second), true),
    );
    dir.to_string_lossy().into_owned()
}

fn change_list(v: &Value) -> Vec<String> {
    v["base_changes"]
        .as_array()
        .expect("changes")
        .iter()
        .map(|c| {
            let show = |k: &str| c[k].as_str().unwrap_or("(none)").to_string();
            format!("{}->{}@{}", show("from"), show("to"), show("pbo"))
        })
        .collect()
}

#[test]
fn a_mod_changing_the_base_twice_records_each_step_from_the_right_base() {
    let w = World::new();
    let m = two_pbo_mod(&w, "class Hatch: X {};", "class Hatch {};");
    let (code, _, err) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    // the boundary change and the mod's own change are both warned about, each once
    assert_eq!(err.matches("api-config-base-changed").count(), 2, "{err}");
    let v = json(&w, "CfgVehicles/Hatch", &["-m", &m]);
    assert_eq!(
        change_list(&v),
        ["Base->X@addons/p1.pbo", "X->(none)@addons/p2.pbo"]
    );
}

#[test]
fn a_mod_changing_the_base_and_back_is_not_hidden_by_the_boundary() {
    let w = World::new();
    let m = two_pbo_mod(&w, "class Hatch: X {};", "class Hatch: Base {};");
    let (code, _, err) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(err.matches("api-config-base-changed").count(), 2, "{err}");
    let v = json(&w, "CfgVehicles/Hatch", &["-m", &m]);
    assert_eq!(
        change_list(&v),
        ["Base->X@addons/p1.pbo", "X->Base@addons/p2.pbo"]
    );
}

#[test]
fn base_names_compare_ignoring_case_across_sources() {
    let w = World::new();
    let m = two_pbo_mod(&w, "class Hatch: BASE {};", "class Hatch: base {};");
    let v = json(&w, "CfgVehicles/Hatch", &["-m", &m]);
    assert!(change_list(&v).is_empty(), "{v}");
}

/// A mod PBO whose `config.cpp` includes `include`; `extra` are more entries of the PBO.
fn include_mod(w: &World, include: &str, extra: &[(&str, &[u8])]) -> String {
    let dir = w.home.join("@Inc");
    let text = format!(
        "class CfgPatches {{ class I_Patch {{ requiredAddons[] = {{}}; }}; }};\nclass Leaked {{\n#include \"{include}\"\n}};\n"
    );
    let mut entries: Vec<(&str, &[u8])> = vec![("config.cpp", text.as_bytes())];
    entries.extend_from_slice(extra);
    write(
        &dir.join("addons/i.pbo"),
        &support::raw_pbo(&entries, &[("prefix", "inc")]),
    );
    dir.to_string_lossy().into_owned()
}

/// `#include` in a mod config never reads the file system (#97): an absolute path, `..` and a file of the
/// working directory are refused with a warning, a sibling entry of the PBO is resolved.
#[test]
fn includes_come_from_the_pbo_only() {
    let w = World::new();
    let secret = w.home.join("secret.hpp");
    fs::write(&secret, "token = \"s3cr3t\";\n").expect("secret");
    fs::write(w.home.join("cwdonly.hpp"), "token = \"s3cr3t\";\n").expect("cwd file");
    for include in [
        secret.to_string_lossy().into_owned(),
        "..\\secret.hpp".to_string(),
        "cwdonly.hpp".to_string(),
        "\\other\\cwdonly.hpp".to_string(),
    ] {
        let m = include_mod(&w, &include, &[]);
        let (code, out, err) = w.run(&["api", "config", "-m", &m, "Leaked"]);
        assert!(
            !out.contains("s3cr3t") && !err.contains("s3cr3t"),
            "{include}: {out}{err}"
        );
        assert!(
            err.contains("api-bad-config") || code != 0,
            "{include}: code {code} out {out} err {err}"
        );
    }
    let m = include_mod(&w, "sub\\a.hpp", &[("sub\\a.hpp", b"found = 7;\n")]);
    let (_, out, err) = w.run(&["api", "config", "-m", &m, "Leaked"]);
    assert!(out.contains("found = 7"), "{out}{err}");
}

/// A mod PBO with `config_name` (text), then `extra` entries, the last of which loses its final bytes.
fn damaged_mod(w: &World, config_name: &str, text: &str, extra: &[(&str, &[u8])]) -> String {
    let dir = w.home.join("@Dmg");
    let mut entries: Vec<(&str, &[u8])> = vec![(config_name, text.as_bytes())];
    entries.extend_from_slice(extra);
    let mut bytes = support::raw_pbo(&entries, &[("prefix", "dmg")]);
    bytes.truncate(bytes.len() - 26);
    write(&dir.join("addons/d.pbo"), &bytes);
    dir.to_string_lossy().into_owned()
}

const PATCHES: &str = "class CfgPatches { class D_Patch { requiredAddons[] = {}; }; };\n";

/// A damaged includable does not drop the PBO's configs (#99): it is skipped with a warning.
#[test]
fn a_damaged_includable_is_skipped_with_a_warning() {
    let w = World::new();
    let text = format!("{PATCHES}class Fine {{ v = 1; }};\n");
    let m = damaged_mod(
        &w,
        "config.cpp",
        &text,
        &[(
            "zz_readme.txt",
            b"some long readme text, longer than the cut",
        )],
    );
    let (code, out, err) = w.run(&["api", "config", "-m", &m, "Fine"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("v = 1"), "{out}{err}");
    assert!(err.contains("api-config-include-unreadable"), "{err}");
    assert!(
        err.contains("zz_readme.txt") && err.contains("truncated"),
        "{err}"
    );
}

/// An include of a damaged entry reports the read error, not "not found".
#[test]
fn an_include_of_a_damaged_entry_names_the_read_error() {
    let w = World::new();
    let text = format!(
        "{PATCHES}class Fine {{ v = 1; }};\nclass Leaked {{\n#include \"zz_readme.txt\"\n}};\n"
    );
    let m = damaged_mod(
        &w,
        "config.cpp",
        &text,
        &[(
            "zz_readme.txt",
            b"v = 2;\n and more text, longer than the cut",
        )],
    );
    let (_, out, err) = w.run(&["api", "config", "-m", &m, "Fine"]);
    assert!(
        err.contains("cannot be read") && err.contains("truncated"),
        "{out}{err}"
    );
    assert!(!err.contains("not found"), "{err}");
}

/// A PBO with only a `config.bin` reads no includables: a truncated one does not matter, and raises no warning.
#[test]
fn a_config_bin_pbo_reads_no_includables() {
    let w = World::new();
    let config = Config::from_string(
        format!("{PATCHES}class Fine {{ v = 1; }};\n"),
        None,
        &[],
        &mut Warnings::new(),
    )
    .expect("config parses");
    let bin = config.to_cursor().expect("rapify").into_inner();
    let m = w.home.join("@Dmg").to_string_lossy().into_owned();
    let mut bytes = support::raw_pbo(
        &[
            ("config.bin", &bin),
            ("readme.txt", b"long text here, longer than the cut"),
        ],
        &[("prefix", "dmg")],
    );
    bytes.truncate(bytes.len() - 26);
    write(&PathBuf::from(&m).join("addons/d.pbo"), &bytes);
    let (code, out, err) = w.run(&["api", "config", "-m", &m, "Fine"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("v = 1"), "{out}{err}");
    assert!(!err.contains("api-config-include-unreadable"), "{err}");
}

fn dotdot_mod(w: &World, config_name: &str, include: &str, extra: &[(&str, &[u8])]) -> String {
    let dir = w.home.join("@Dots");
    let text = format!("{PATCHES}class Leaked {{\n#include \"{include}\"\n}};\n");
    let mut entries: Vec<(&str, &[u8])> = vec![(config_name, text.as_bytes())];
    entries.extend_from_slice(extra);
    write(
        &dir.join("addons/d.pbo"),
        &support::raw_pbo(&entries, &[("prefix", "dots")]),
    );
    dir.to_string_lossy().into_owned()
}

/// `..` that stays inside the PBO resolves (#99); one that leaves the root is refused.
#[test]
fn dotdot_inside_the_pbo_resolves_and_outside_is_refused() {
    let w = World::new();
    let macros: &[(&str, &[u8])] = &[("macros.hpp", b"found = 7;\n")];
    for include in [
        "..\\macros.hpp",
        "../macros.hpp",
        "..\\sub\\..\\macros.hpp",
        "inner\\..\\../macros.hpp",
    ] {
        let m = dotdot_mod(&w, "sub\\config.cpp", include, macros);
        let (_, out, err) = w.run(&["api", "config", "-m", &m, "Leaked"]);
        assert!(out.contains("found = 7"), "{include}: {out}{err}");
    }
    for include in [
        "..\\..\\macros.hpp",
        "sub\\..\\..\\..\\macros.hpp",
        "../..\\macros.hpp",
        "a\\\\b.hpp",
    ] {
        let m = dotdot_mod(&w, "sub\\config.cpp", include, macros);
        let (_, out, err) = w.run(&["api", "config", "-m", &m, "Leaked"]);
        assert!(!out.contains("found = 7"), "{include}: {out}{err}");
        assert!(err.contains("api-bad-config"), "{include}: {out}{err}");
    }
}

/// A game folder (a `World`'s) gets one more config PBO that requires `B_Patch`.
fn add_addon(w: &World, file: &str, patch: &str, body: &str) {
    write(
        &w.game.join(format!("Addons/{file}.pbo")),
        &config_pbo(
            &format!(
                "class CfgPatches {{ class {patch} {{ requiredAddons[] = {{\"B_Patch\"}}; }}; }};\n{body}"
            ),
            true,
        ),
    );
}

#[test]
fn a_failed_lookup_suggests_similar_classes_or_names_the_deepest_one() {
    let w = World::new();
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/atch"]);
    assert_eq!(code, 1);
    assert!(err.contains("similar in CfgVehicles: Hatch"), "{err}");
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/Hatch/Nope/Deeper"]);
    assert_eq!(code, 1);
    assert!(err.contains("the deepest class found is"), "{err}");
    let (code, _, err) = w.run(&["api", "config", "Nothing"]);
    assert_eq!(code, 1);
    assert!(err.contains("no config class"), "{err}");
}

#[test]
fn an_inheritance_chain_that_is_too_long_is_an_error() {
    let w = World::new();
    let mut body = String::from("class CfgVehicles {\nclass Link0 { v = 0; };\n");
    body.extend((1..300).map(|i| format!("class Link{i}: Link{} {{ }};\n", i - 1)));
    body.push_str("};\n");
    add_addon(&w, "chain", "Chain_Patch", &body);
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/Link299"]);
    assert_eq!(code, 1);
    assert!(err.contains("longer than"), "{err}");
}

#[test]
fn values_of_every_kind_are_shown_and_long_ones_are_cut() {
    let w = World::new();
    let long_string = "s".repeat(500);
    let words: Vec<String> = (0..400).map(|i| format!("\"word{i}\"")).collect();
    let mut nested = String::new();
    for _ in 0..40 {
        nested.push('{');
    }
    for _ in 0..40 {
        nested.push('}');
    }
    let body = format!(
        "class CfgVehicles {{\n\
         class Scalar {{ tags = \"plain\"; ratio = 0.5; mixed[] = {{1, 2.5, \"x\", {{3}}}}; }};\n\
         class Derived: Scalar {{ tags[] += {{\"added\"}}; }};\n\
         class Long {{ text = \"{long_string}\"; words[] = {{{}}}; deep[] = {nested}; }};\n\
         }};",
        words.join(",")
    );
    write(&w.game.join("Addons/values.pbo"), &config_pbo(&body, false));
    // a `+=` onto something that is no array replaces it
    let v = json(&w, "CfgVehicles/Derived", &[]);
    assert_eq!(prop(&v, "tags")["value"], serde_json::json!(["added"]));
    let v = json(&w, "CfgVehicles/Scalar", &[]);
    assert_eq!(prop(&v, "ratio")["value"], 0.5);
    assert_eq!(
        prop(&v, "mixed")["value"],
        serde_json::json!([1, 2.5, "x", [3]])
    );
    let (code, out, err) = w.run(&["api", "config", "--max", "1000", "CfgVehicles/Long"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains(" ..."), "cut values end in an ellipsis: {out}");
    assert!(!out.contains(&long_string), "{out}");
    assert!(out.contains("word0"), "{out}");
    assert!(!out.contains("word399"), "a late word is cut: {out}");
    let deep = out
        .lines()
        .find(|l| l.trim_start().starts_with("deep"))
        .expect("deep line");
    assert!(deep.contains("{{{{"), "{deep}");
    let (code, out, err) = w.run(&["api", "config", "-v", "CfgVehicles/Scalar"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("# config vanilla: "), "{out}");
}

#[test]
fn delete_removes_nested_classes_and_a_redefinition_starts_fresh() {
    let w = World::new();
    add_addon(
        &w,
        "holder",
        "Holder_Patch",
        "class CfgVehicles { class Holder { keep = 1; class Kid { k = 1; }; }; class Gone { g = 1; }; class Phoenix { old = 1; }; };",
    );
    write(
        &w.game.join("Addons/holder2.pbo"),
        &config_pbo(
            "class CfgPatches { class H2_Patch { requiredAddons[] = {\"Holder_Patch\"}; }; };\n\
             class CfgVehicles { delete Holder; delete Phoenix; class Phoenix { born = 2; }; };",
            true,
        ),
    );
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/Holder"]);
    assert_eq!(code, 1);
    assert!(err.contains("no config class"), "{err}");
    let (code, _, _) = w.run(&["api", "config", "CfgVehicles/Holder/Kid"]);
    assert_eq!(code, 1);
    let v = json(&w, "CfgVehicles/Phoenix", &[]);
    assert_eq!(prop(&v, "born")["value"], 2);
    assert!(
        v["properties"]
            .as_array()
            .expect("properties")
            .iter()
            .all(|p| p["name"] != "old")
    );
    // a mod deletes and revives classes of the game
    let dir = w.home.join("@Del");
    write(
        &dir.join("addons/del.pbo"),
        &config_pbo(
            "class CfgPatches { class D_Patch { requiredAddons[] = {\"B_Patch\"}; }; };\n\
             class CfgVehicles { delete Other; delete Newcar; class Newcar { reborn = 1; }; };",
            true,
        ),
    );
    let m = dir.to_string_lossy().into_owned();
    let (code, _, _) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Other"]);
    assert_eq!(code, 1);
    let v = json(&w, "CfgVehicles/Newcar", &["-m", &m]);
    assert_eq!(prop(&v, "reborn")["value"], 1);
    assert!(
        v["properties"]
            .as_array()
            .expect("properties")
            .iter()
            .all(|p| p["name"] != "scope")
    );
    assert_eq!(v["chain"].as_array().expect("chain").len(), 1);
}

#[test]
fn a_config_bin_wins_over_a_config_cpp_of_the_same_folder() {
    let w = World::new();
    let cpp = "class CfgVehicles { class Dup { from = \"cpp\"; }; };";
    let bin = Config::from_string(
        "class CfgVehicles { class Dup { from = \"bin\"; }; };".to_string(),
        None,
        &[],
        &mut Warnings::new(),
    )
    .expect("config")
    .to_cursor()
    .expect("rapify")
    .into_inner();
    write(
        &w.game.join("Addons/both.pbo"),
        &support::raw_pbo(
            &[("config.cpp", cpp.as_bytes()), ("config.bin", &bin)],
            &[("prefix", "both")],
        ),
    );
    let v = json(&w, "CfgVehicles/Dup", &[]);
    assert_eq!(prop(&v, "from")["value"], "bin");
}

#[test]
fn a_requiredaddons_ring_of_many_configs_is_named_with_an_ellipsis() {
    let w = World::new();
    for i in 0..7 {
        let next = (i + 1) % 7;
        write(
            &w.game.join(format!("Addons/ring{i}.pbo")),
            &config_pbo(
                &format!(
                    "class CfgPatches {{ class Ring{i} {{ requiredAddons[] = {{\"Ring{next}\"}}; }}; }};\n\
                     class CfgVehicles {{ class RingCar{i} {{ n = {i}; }}; }};"
                ),
                true,
            ),
        );
    }
    let (code, _, err) = w.run(&["api", "config", "CfgVehicles/RingCar6"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("api-config-order") && err.contains(", ..."),
        "{err}"
    );
}

#[test]
fn a_file_that_is_no_pbo_is_a_warning_and_a_game_with_only_dta_works() {
    let w = World::new();
    write(&w.game.join("Addons/garbage.pbo"), b"this is not a pbo");
    let (code, out, err) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Hatch"), "{out}");
    assert!(
        err.contains("api-bad-config") && err.contains("garbage.pbo"),
        "{err}"
    );
    // a game folder with a dta folder and no addons folder
    let only = w.home.join("onlydta");
    write(
        &only.join("dta/scripts.pbo"),
        &support::raw_pbo(
            &[("3_Game/a.c", b"class Base\n{\n}\n")],
            &[("prefix", "scripts")],
        ),
    );
    write(
        &only.join("dta/cfg.pbo"),
        &config_pbo("class CfgVehicles { class OnlyDta { v = 7; }; };", true),
    );
    let (code, out, err) = w.run(&[
        "api",
        "config",
        "--dayz",
        &only.to_string_lossy(),
        "CfgVehicles/OnlyDta",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("v = 7"), "{out}");
}

#[test]
fn a_game_folder_without_pbos_has_no_config_index() {
    let tmp = tempfile::tempdir().expect("tmp");
    fs::create_dir_all(tmp.path().join("dta")).expect("mkdir");
    let err = dayzmake::api::config_index::vanilla_config_pbos(tmp.path()).expect_err("no PBOs");
    assert!(err.to_string().contains("no PBOs found"), "{err}");
}

/// A config PBO that provides `patch`, requires `requires` and sets `v` of the class `Z` to `v`.
fn z_pbo(patch: &str, requires: &str, v: i32) -> Vec<u8> {
    config_pbo(
        &format!(
            "class CfgPatches {{ class {patch} {{ requiredAddons[] = {{{requires}}}; }}; }};\n\
             class CfgVehicles {{ class Z {{ v = {v}; {patch}_set = 1; }}; }};"
        ),
        true,
    )
}

fn z_value(w: &World, extra: &[&str]) -> Value {
    let v = json(w, "CfgVehicles/Z", extra);
    prop(&v, "v")["value"].clone()
}

#[test]
fn mods_all_includes_a_mod_with_only_a_config_pbo() {
    let w = World::new();
    let dir = w.root.join("steamapps/workshop/content/221100/300");
    write(
        &dir.join("addons/retex.pbo"),
        &config_pbo(
            "class CfgVehicles { class Retex: Hatch { displayName = \"Retexed\"; }; };",
            true,
        ),
    );
    let v = json(&w, "CfgVehicles/Retex", &["-m", "all"]);
    assert_eq!(prop(&v, "displayName")["value"], "Retexed");
}

#[test]
fn a_vanilla_addon_that_requires_a_mod_addon_loads_after_it() {
    let w = World::new();
    write(
        &w.game.join("Addons/aa_req.pbo"),
        &z_pbo("V_Req", "\"M_Z\"", 2),
    );
    let dir = w.home.join("@ZMod");
    write(&dir.join("addons/z.pbo"), &z_pbo("M_Z", "", 1));
    let m = dir.to_string_lossy().into_owned();
    // without the mod the game's own value stands
    assert_eq!(z_value(&w, &[]), 2);
    // the vanilla addon needs the mod addon: it loads after it and wins, although vanilla comes first
    assert_eq!(z_value(&w, &["-m", &m]), 2);
    let v = json(&w, "CfgVehicles/Z", &["-m", &m]);
    let order: Vec<&str> = v["pbos"]
        .as_array()
        .expect("pbos")
        .iter()
        .map(|p| p["source"].as_str().expect("source"))
        .collect();
    assert_eq!(order, ["_ZMod", "vanilla"], "{v}");
}

#[test]
fn a_mod_addon_required_by_a_later_mod_loads_first() {
    let w = World::new();
    let a = w.home.join("@A");
    let b = w.home.join("@B");
    // a is named first but requires b's addon
    write(&a.join("addons/a.pbo"), &z_pbo("A_Z", "\"B_Z\"", 1));
    write(&b.join("addons/b.pbo"), &z_pbo("B_Z", "", 2));
    let (a, b) = (
        a.to_string_lossy().into_owned(),
        b.to_string_lossy().into_owned(),
    );
    assert_eq!(z_value(&w, &["-m", &format!("{a},{b}")]), 1);
}

#[test]
fn without_dependencies_the_order_is_vanilla_then_the_mods_as_selected() {
    let w = World::new();
    let a = w.home.join("@A");
    let b = w.home.join("@B");
    write(&a.join("addons/a.pbo"), &z_pbo("A_Z", "", 1));
    write(&b.join("addons/b.pbo"), &z_pbo("B_Z", "", 2));
    let (a, b) = (
        a.to_string_lossy().into_owned(),
        b.to_string_lossy().into_owned(),
    );
    assert_eq!(z_value(&w, &["-m", &format!("{a},{b}")]), 2);
    assert_eq!(z_value(&w, &["-m", &format!("{b},{a}")]), 1);
}

#[test]
fn a_mod_folder_named_vanilla_still_warns_about_a_base_change() {
    let w = World::new();
    let dir = w.home.join("vanilla");
    write(
        &dir.join("addons/o.pbo"),
        &config_pbo(
            "class CfgVehicles { class P {}; class Car_Base: P {}; class Car_Base {}; };",
            true,
        ),
    );
    let m = dir.to_string_lossy().into_owned();
    let (code, _, err) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Car_Base"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("api-config-base-changed"), "{err}");
}

#[test]
fn the_base_changed_warning_shows_the_class_path_as_written() {
    let w = World::new();
    let dir = w.home.join("@Own");
    write(
        &dir.join("addons/o.pbo"),
        &config_pbo(
            "class CfgVehicles { class P {}; class Car_Base: P {}; class Car_Base {}; };",
            true,
        ),
    );
    let m = dir.to_string_lossy().into_owned();
    let (code, _, err) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Car_Base"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("class CfgVehicles/Car_Base changes its base class: P -> (none)"),
        "{err}"
    );
    // and across sources
    write(
        &w.game.join("Addons/zz_cars.pbo"),
        &config_pbo(
            "class CfgPatches { class Cars { requiredAddons[] = {\"B_Patch\"}; }; };\n\
             class CfgVehicles { class Car_Base2: Base {}; };",
            true,
        ),
    );
    let dir = w.home.join("@Own2");
    write(
        &dir.join("addons/o.pbo"),
        &config_pbo("class CfgVehicles { class Car_Base2 {}; };", true),
    );
    let m = dir.to_string_lossy().into_owned();
    let (code, _, err) = w.run(&["api", "config", "-m", &m, "CfgVehicles/Car_Base2"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("class CfgVehicles/Car_Base2 changes its base class: Base -> (none)"),
        "{err}"
    );
}

#[test]
fn non_finite_floats_are_shown_as_derapify_writes_them() {
    let w = World::new();
    write(
        &w.game.join("Addons/inf.pbo"),
        &config_pbo(
            "class CfgVehicles { class Wild { up = 1e999; down = -1e999; ok = 0.5; }; };",
            false,
        ),
    );
    let (code, out, err) = w.run(&["api", "config", "CfgVehicles/Wild"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("up = 1e999"), "{out}");
    assert!(out.contains("down = -1e999"), "{out}");
    let v = json(&w, "CfgVehicles/Wild", &[]);
    assert_eq!(prop(&v, "up")["value"], "1e999");
    assert_eq!(prop(&v, "down")["value"], "-1e999");
    assert_eq!(prop(&v, "ok")["value"], 0.5);
}
