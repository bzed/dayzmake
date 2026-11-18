//! Licence guard: every source file that carries the upstream Recast/Detour copyright must also say
//! that it is a port and not the original software (the zlib licence requires altered versions to be
//! marked as such).

#![expect(
    clippy::unwrap_used,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::fs;
use std::path::{Path, PathBuf};

const UPSTREAM: &str = "Mikko Mononen";
const NOTICE: &str = "Ported to Rust and altered for DayZ (see the dayzmake navmesh spec); this is not the original software.";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn ported_files_carry_the_altered_notice() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut ported = 0;
    let mut missing = Vec::new();
    for f in &files {
        let text = fs::read_to_string(f).unwrap();
        if text.contains(UPSTREAM) {
            ported += 1;
            if !text.lines().any(|l| l.contains(NOTICE)) {
                missing.push(f.display().to_string());
            }
        }
    }
    assert!(
        ported >= 10,
        "found only {ported} ported files: is the walk broken?"
    );
    assert!(
        missing.is_empty(),
        "files without the altered-software notice: {missing:?}"
    );
}
