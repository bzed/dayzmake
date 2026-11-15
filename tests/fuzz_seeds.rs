//! Runs every fuzz corpus seed (and every committed crash regression input) through the
//! same readers the fuzz targets call, so that `cargo test` catches panics without nightly.

use std::fs;
use std::io;
use std::path::Path;

use dayzmake::api::model::ApiIndex;
use dayzmake::api::parse::{ScriptFile, active_defines, collect_defines, index_scripts};
use dayzmake::api::steam::{Branch, vdf_values, vdf_values_unescaped};

fn seeds(target: &str) -> io::Result<Vec<Vec<u8>>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus")
        .join(target);
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir)? {
        paths.push(entry?.path());
    }
    paths.sort();
    assert!(!paths.is_empty(), "no seeds in {}", dir.display());
    paths.iter().map(fs::read).collect()
}

#[test]
fn api_parse_seeds() -> io::Result<()> {
    for data in seeds("api_parse")? {
        let text = String::from_utf8_lossy(&data);
        let files = [
            ScriptFile {
                module: 0,
                path: "3_Game\\a.c",
                text: &text,
            },
            ScriptFile {
                module: 0,
                path: "4_World\\b.c",
                text: &text,
            },
        ];
        let _ = collect_defines(&text, &files);
        let mut index = ApiIndex::new("fuzz", "fuzz", "fuzz");
        index_scripts(&files, &active_defines(Branch::Stable), &mut index);
    }
    Ok(())
}

#[test]
fn steam_vdf_seeds() -> io::Result<()> {
    for data in seeds("steam_vdf")? {
        let text = String::from_utf8_lossy(&data);
        for key in ["path", "installdir", "buildid"] {
            let _ = vdf_values(&text, key);
            let _ = vdf_values_unescaped(&text, key);
        }
    }
    Ok(())
}
