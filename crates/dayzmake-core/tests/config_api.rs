//! Read access to parsed configs.
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;

use dayzmake_core::config::{Config, ConfigArrayElement, ConfigEntry};
use dayzmake_core::warning::Warnings;

#[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
fn golden() -> Config {
    let dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/golden/mod"
    ));
    let path = dir.join("config.cpp");
    let input = std::fs::read_to_string(&path).unwrap();
    Config::from_string(input, Some(path), &[dir], &mut Warnings::new()).unwrap()
}

#[expect(clippy::panic, reason = "tests fail by panicking")]
fn strings(elements: &[ConfigArrayElement]) -> Vec<&str> {
    elements
        .iter()
        .map(|e| match e {
            ConfigArrayElement::StringElement(s) => s.as_str(),
            _ => panic!("not a string element"),
        })
        .collect()
}

/// The assertions both reading paths share.
#[expect(clippy::unwrap_used, clippy::panic, reason = "tests fail by panicking")]
fn check(config: &Config) {
    let root = config.root();
    assert!(!root.is_external());
    assert!(!root.is_deletion());
    assert_eq!(root.parent(), None);

    // case-insensitive lookup, file order
    let Some(ConfigEntry::ClassEntry(patches)) = root.get("cfgpatches") else {
        panic!("CfgPatches is not a class");
    };
    let names: Vec<&str> = root.entries().map(|(n, _)| n).collect();
    assert_eq!(names.first(), Some(&"CfgPatches"));
    assert!(root.get("nothing").is_none());

    let (first_name, first) = patches.entries().next().unwrap();
    assert_eq!(first_name, "golden_addon");
    let ConfigEntry::ClassEntry(modname) = first else {
        panic!("golden_addon is not a class");
    };
    let Some(ConfigEntry::ArrayEntry(units)) = modname.get("UNITS") else {
        panic!("units is not an array");
    };
    assert!(units.elements().is_empty());
    assert!(!units.is_expansion());
    let Some(ConfigEntry::ArrayEntry(weapons)) = modname.get("weapons") else {
        panic!("weapons is not an array");
    };
    assert_eq!(strings(weapons.elements()), ["golden_a", "golden_b"]);

    let Some(ConfigEntry::ClassEntry(golden)) = root.get("CfgGolden") else {
        panic!("CfgGolden is not a class");
    };
    let Some(ConfigEntry::ClassEntry(child)) = golden.get("Child") else {
        panic!("Child is not a class");
    };
    assert_eq!(child.parent(), Some("Parent"));
    let Some(ConfigEntry::ClassEntry(parent)) = golden.get("Parent") else {
        panic!("Parent is not a class");
    };
    assert_eq!(parent.parent(), None);
    let Some(ConfigEntry::ClassEntry(obsolete)) = child.get("Obsolete") else {
        panic!("Obsolete is not a class entry");
    };
    assert!(obsolete.is_deletion());
}

#[test]
fn read_values_from_a_parsed_config() {
    let config = golden();
    check(&config);
}

#[test]
fn get_returns_the_first_of_duplicated_names() {
    let config = Config::from_string(
        String::from("a = 1;\nA = 2;\n"),
        None,
        &[],
        &mut Warnings::new(),
    )
    .unwrap();
    assert_eq!(config.root().entries().count(), 2);
    assert!(matches!(
        config.root().get("a"),
        Some(ConfigEntry::IntEntry(1))
    ));
}

#[test]
fn read_values_from_a_rapified_config() {
    let config = golden();
    let mut cursor = config.to_cursor().unwrap();
    cursor.seek(SeekFrom::Start(0)).unwrap();
    let rapified = Config::read_rapified(&mut cursor).unwrap();
    check(&rapified);
}

#[test]
fn external_classes_have_no_entries() {
    let config = golden();
    let Some(ConfigEntry::ClassEntry(base)) = config.root().get("Base") else {
        panic!("Base is not a class");
    };
    assert!(base.is_external());
    assert_eq!(base.entries().count(), 0);
}

#[test]
fn a_nul_in_a_string_entry_names_the_entry() {
    let src = "class A { a = \"x\0y\"; b[] = {\"ok\", \"p\0q\"}; };".to_string();
    let config = Config::from_string(src, None, &[], &mut Warnings::new()).unwrap();
    let msg = config.to_cursor().unwrap_err().to_string();
    assert!(msg.contains("Entry \"a\""), "{msg}");
    let src = "class A { b[] = {\"ok\", \"p\0q\"}; };".to_string();
    let config = Config::from_string(src, None, &[], &mut Warnings::new()).unwrap();
    let msg = config.to_cursor().unwrap_err().to_string();
    assert!(
        msg.contains("Entry \"b\"") && msg.contains("element 1"),
        "{msg}"
    );
    let src = "class A { b[] = {\"ok\", \"ok\", {\"p\0q\"}}; };".to_string();
    let config = Config::from_string(src, None, &[], &mut Warnings::new()).unwrap();
    let msg = config.to_cursor().unwrap_err().to_string();
    let outer = msg.find("Array element 2").unwrap();
    let inner = msg.find("Array element 0").unwrap();
    assert!(outer < inner, "{msg}");
}
