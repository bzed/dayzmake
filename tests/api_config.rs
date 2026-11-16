//! `dayzmake api config`: lookup of config classes with their inheritance chain and resolved properties.
#![expect(clippy::expect_used, reason = "a failing expect is a failing test")]
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
    // the first run builds the index, and warns
    let (code, out, err) = w.run(&["api", "config", "CfgVehicles/Hatch"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("api-config-base-changed"), "{err}");
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
    let v = json(&w, "CfgVehicles/Hatch", &["-m", &m]);
    assert_eq!(v["chain_end"], "root");
    assert_eq!(v["chain"].as_array().expect("chain").len(), 1);
    assert_eq!(v["base_changes"][0]["from"], "Base");
    assert_ne!(v["base_changes"][0]["source"], "vanilla");
}
