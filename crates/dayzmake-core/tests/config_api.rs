//! Read access to parsed configs.
#![expect(clippy::unwrap_used, clippy::panic, reason = "tests fail by panicking")]
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;

use dayzmake_core::config::{Config, ConfigArrayElement, ConfigEntry};
use dayzmake_core::warning::Warnings;

fn golden() -> Config {
    let dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/golden/mod"
    ));
    let path = dir.join("config.cpp");
    let input = std::fs::read_to_string(&path).unwrap();
    Config::from_string(input, Some(path), &[dir], &mut Warnings::new()).unwrap()
}

fn strings(elements: &[ConfigArrayElement]) -> Vec<&str> {
    elements
        .iter()
        .map(|e| match e {
            ConfigArrayElement::StringElement(s) => s.as_str(),
            _ => panic!("not a string element"),
        })
        .collect()
}

/// The assertions both reading paths share. Avoids `delete Obsolete;` (issue #3).
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
}

#[test]
fn read_values_from_a_parsed_config() {
    check(&golden());
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
