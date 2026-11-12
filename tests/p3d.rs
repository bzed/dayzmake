//! MLOD reading/writing and the model-related CLI commands. None of this needs DayZ Tools or Proton.
use std::path::{Path, PathBuf};
use std::process::Command;

use dayzmake::p3d::P3D;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mlod/small.p3d")
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn dayzmake(args: &[&str], cwd: &Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn mlod_round_trips_byte_for_byte() {
    let original = std::fs::read(fixture()).unwrap();
    let p3d = P3D::read(&mut &original[..]).unwrap();

    // repeated tags must all survive, in order
    let names: Vec<&str> = p3d.lods[0].taggs.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names.iter().filter(|n| **n == "#Property#").count(), 2);
    assert_eq!(names.iter().filter(|n| **n == "#UVSet#").count(), 2);
    // the unused 4th vertex of a triangle is kept
    assert_eq!(
        p3d.lods[0].faces[0]
            .spare_vertex
            .as_ref()
            .unwrap()
            .point_index,
        7
    );

    let mut written = Vec::new();
    p3d.write(&mut written).unwrap();
    assert!(written == original, "MLOD round trip changed the file");
}

/// Set `DAYZMAKE_P3D_CORPUS` to a folder of real models to check them all.
#[test]
#[ignore = "set DAYZMAKE_P3D_CORPUS to a folder of real models"]
#[expect(clippy::print_stderr, reason = "skip notice")]
fn real_models_round_trip() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
        {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("p3d")) {
                out.push(p);
            }
        }
    }
    let dir = if let Some(d) = std::env::var_os("DAYZMAKE_P3D_CORPUS") {
        PathBuf::from(d)
    } else {
        eprintln!("skipped: set DAYZMAKE_P3D_CORPUS");
        return;
    };
    let mut files = Vec::new();
    walk(&dir, &mut files);
    let mut checked = 0;
    for f in files {
        let data = std::fs::read(&f).unwrap();
        if !data.starts_with(b"MLOD") {
            continue;
        }
        let p3d = P3D::read(&mut &data[..]).unwrap_or_else(|e| panic!("{}: {}", f.display(), e));
        let mut out = Vec::new();
        p3d.write(&mut out).unwrap();
        assert!(out == data, "{} does not round-trip", f.display());
        checked += 1;
    }
    eprintln!("{checked} models round-trip");
}

#[test]
fn damaged_models_are_errors_not_panics() {
    let original = std::fs::read(fixture()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    // every truncation, and a few corruptions of the counts
    for len in (0..original.len()).step_by(37).chain([original.len() - 1]) {
        std::fs::write(dir.path().join("m.p3d"), &original[..len]).unwrap();
        let (code, _, err) = dayzmake(&["inspect", "m.p3d"], dir.path());
        assert_ne!(code, 101, "panic at length {len}: {err}");
    }
    for at in [8, 12, 16, 20, 24, 28, 32, 36, 40] {
        let mut bad = original.clone();
        bad[at..at + 4].copy_from_slice(&[0xff; 4]);
        std::fs::write(dir.path().join("m.p3d"), &bad).unwrap();
        let (code, _, err) = dayzmake(&["inspect", "m.p3d"], dir.path());
        assert_ne!(code, 101, "panic with 0xff at {at}: {err}");
    }
}

#[test]
fn inspect_mlod() {
    let (code, out, err) = dayzmake(&["inspect", fixture().to_str().unwrap()], Path::new("."));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("MLOD"), "{}", out);
    assert!(out.contains("Version: 257"), "{}", out);
    assert!(out.contains("LODs:    2"), "{}", out);
    assert!(out.contains("t21\\data\\a_co.paa"), "{}", out);
    assert!(out.contains("t21\\data\\m.rvmat"), "{}", out);
    assert!(out.contains("class = house"), "{}", out);
    assert!(out.contains("autocenter = 0"), "{}", out);
    assert!(out.contains("body"), "{}", out);
    assert!(out.contains("Memory"), "{}", out);
}

#[test]
fn inspect_odol_reports_version_for_every_version() {
    let dir = tempfile::tempdir().unwrap();
    for version in [53u32, 54, 55, 56] {
        let mut data = b"ODOL".to_vec();
        data.extend_from_slice(&version.to_le_bytes());
        data.extend_from_slice(b"not a real model");
        std::fs::write(dir.path().join("m.p3d"), &data).unwrap();
        let (code, out, err) = dayzmake(&["inspect", "m.p3d"], dir.path());
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("ODOL"), "{}", out);
        assert!(out.contains(&format!("Version: {version}")), "{}", out);
    }
}

#[cfg(unix)]
#[test]
fn build_warns_about_missing_textures_without_proton() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("data")).unwrap();
    std::fs::write(src.join("$PBOPREFIX$"), "t21\n").unwrap();
    std::fs::write(src.join("data/a_co.paa"), b"x").unwrap();
    std::fs::write(src.join("data/m.rvmat"), "class StageTI {};\n").unwrap();
    std::fs::copy(fixture(), src.join("model.p3d")).unwrap();

    let (code, _, err) = dayzmake(&["build", "src", "out.pbo"], dir.path());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("missing-texture"), "{}", err);
    assert!(err.contains("gone_co.paa"), "{}", err);
    assert!(
        !err.contains("a_co.paa"),
        "an existing texture was reported: {err}"
    );
    assert!(
        !err.contains("argb"),
        "a procedural texture was reported: {err}"
    );
    assert!(!err.contains("missing-material"), "{}", err);

    // the case of the file name doesn't matter, and the warning can be muted
    std::fs::rename(src.join("data/m.rvmat"), src.join("data/M.RVMAT")).unwrap();
    std::fs::remove_file(src.join("data/a_co.paa")).unwrap();
    let (code, _, err) = dayzmake(
        &["build", "-f", "-w", "missing-texture", "src", "out.pbo"],
        dir.path(),
    );
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("missing-texture"), "{}", err);
    assert!(!err.contains("missing-material"), "{}", err);
}
