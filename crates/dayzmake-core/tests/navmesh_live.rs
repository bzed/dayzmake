//! Live comparison of a mesh built from a capture with the mesh that ships with DayZ (ignored: it
//! needs game data, which never goes into the repository).
//!
//! `DAYZ_NAVMESH` is the shipped `.nm`, `DAYZ_NAVMESH_CAPTURE` a `.dzgeo` capture of the same world
//! and `DAYZ_NAVMESH_INNER` the tile range `x0,z0,x1,z1` to compare. Only inner tiles can match: the
//! far end of an off-mesh connection into a tile outside the capture is not snapped to that tile's mesh
//! (and not linked), so the tiles at the edge of a block differ. The island filter is off, so the
//! polygon flags are not compared. The optional `DAYZ_NAVMESH_RANGE` (`x0,z0,x1,z1`) limits the build to
//! that tile range, which makes a block of a capture that is too big to build whole usable; build one
//! tile more than `DAYZ_NAVMESH_INNER` on each side.

#![cfg(feature = "navmesh")]
#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::fs::File;

use dayzmake_core::navmesh::capture::{CaptureFile, CaptureFileOptions};
use dayzmake_core::navmesh::detour::{NavMeshSet, TileData};
use dayzmake_core::navmesh::generate::{GenerateOptions, Progress, Source, generate};
use dayzmake_core::warning::Warnings;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} (see the header of this file)"))
}

/// Builds the capture without the island filter on `threads` builders; returns the mesh bytes and set.
fn build(path: &str, threads: usize, range: Option<[i32; 4]>) -> (Vec<u8>, NavMeshSet) {
    let mut warnings = Warnings::new();
    let capture = CaptureFile::open(
        std::path::Path::new(path),
        &CaptureFileOptions::new(),
        &mut warnings,
    )
    .unwrap_or_else(|e| panic!("open {path} (DAYZ_NAVMESH_CAPTURE): {e}"));
    let mut options = GenerateOptions::default();
    options.island_filter = false;
    options.threads = Some(threads);
    options.range = range;
    let report = generate(
        Source::CaptureFile(capture),
        &options,
        &mut |_: &Progress| {},
        &mut warnings,
    )
    .expect("generate");
    assert!(
        report.failed.is_empty(),
        "failed tiles: {:?}",
        report.failed
    );
    assert!(
        report.stopped.is_none(),
        "the run stopped early: {:?}",
        report.stopped
    );
    let mut bytes = Vec::new();
    report.set.write(&mut bytes).expect("write");
    (bytes, report.set)
}

/// Parses `x0,z0,x1,z1` from the environment variable `name`.
fn rect(name: &str, text: &str) -> [i32; 4] {
    let n: Vec<i32> = text
        .split(',')
        .map(|n| {
            n.trim()
                .parse()
                .unwrap_or_else(|_| panic!("{name} is x0,z0,x1,z1"))
        })
        .collect();
    let [x0, z0, x1, z1] = n[..] else {
        panic!("{name} is x0,z0,x1,z1");
    };
    [x0, z0, x1, z1]
}

/// Describes how our tile differs from the shipped one, section by section; empty when equal. The
/// links, `max_link_count`, `first_link` and the polygon flags are not compared.
fn differences(ours: &TileData, shipped: &TileData) -> Vec<String> {
    let mut out = Vec::new();
    let (mut a, mut b) = (ours.header.clone(), shipped.header.clone());
    a.max_link_count = 0;
    b.max_link_count = 0;
    if a != b {
        out.push(format!("header: ours {a:?}, shipped {b:?}"));
    }
    if ours.verts.len() != shipped.verts.len() {
        out.push(format!(
            "vertex count: ours {}, shipped {}",
            ours.verts.len(),
            shipped.verts.len()
        ));
    }
    for (i, (p, q)) in ours.verts.iter().zip(&shipped.verts).enumerate() {
        if p.map(f32::to_bits) != q.map(f32::to_bits) {
            out.push(format!("vertex {i}: ours {p:?}, shipped {q:?}"));
        }
    }
    let strip = |p: &dayzmake_core::navmesh::detour::Poly| {
        let mut p = p.clone();
        p.first_link = 0;
        p.flags = 0;
        p
    };
    if ours.polys.len() != shipped.polys.len() {
        out.push(format!(
            "polygon count: ours {}, shipped {}",
            ours.polys.len(),
            shipped.polys.len()
        ));
    }
    for (i, (p, q)) in ours.polys.iter().zip(&shipped.polys).enumerate() {
        if strip(p) != strip(q) {
            out.push(format!("poly {i}: ours {p:?}, shipped {q:?}"));
        }
    }
    sections(
        &mut out,
        "detail mesh",
        &ours.detail_meshes,
        &shipped.detail_meshes,
    );
    sections(
        &mut out,
        "detail vertex",
        &ours
            .detail_verts
            .iter()
            .map(|v| v.map(f32::to_bits))
            .collect::<Vec<_>>(),
        &shipped
            .detail_verts
            .iter()
            .map(|v| v.map(f32::to_bits))
            .collect::<Vec<_>>(),
    );
    sections(
        &mut out,
        "detail triangle",
        &ours.detail_tris,
        &shipped.detail_tris,
    );
    sections(&mut out, "bv node", &ours.bv_tree, &shipped.bv_tree);
    // By f32 bits: (pos, centre height, reserved, radius, poly, flags, side, user id).
    let bits = |t: &TileData| {
        t.off_mesh
            .iter()
            .map(|c| {
                (
                    c.pos.map(f32::to_bits),
                    [c.center_height, c.reserved, c.rad].map(f32::to_bits),
                    (c.poly, c.flags, c.side, c.user_id),
                )
            })
            .collect::<Vec<_>>()
    };
    sections(&mut out, "off-mesh connection", &bits(ours), &bits(shipped));
    if ours.extra != shipped.extra {
        out.push(format!(
            "extra section: ours {:?}, shipped {:?}",
            ours.extra, shipped.extra
        ));
    }
    out
}

/// Pushes the first differing entry of a section, and a length mismatch.
fn sections<T: PartialEq + std::fmt::Debug>(out: &mut Vec<String>, name: &str, a: &[T], b: &[T]) {
    if a.len() != b.len() {
        out.push(format!(
            "{name} count: ours {}, shipped {}",
            a.len(),
            b.len()
        ));
    }
    for (i, (p, q)) in a.iter().zip(b).enumerate() {
        if p != q {
            out.push(format!("{name} {i}: ours {p:?}, shipped {q:?}"));
        }
    }
}

#[test]
#[ignore = "needs DayZ navmesh data"]
fn capture_matches_shipped_inner_tiles() {
    let shipped_path = env("DAYZ_NAVMESH");
    let capture_path = env("DAYZ_NAVMESH_CAPTURE");
    let [x0, z0, x1, z1] = rect("DAYZ_NAVMESH_INNER", &env("DAYZ_NAVMESH_INNER"));
    let range = std::env::var("DAYZ_NAVMESH_RANGE")
        .ok()
        .map(|r| rect("DAYZ_NAVMESH_RANGE", &r));
    let shipped = NavMeshSet::read(
        &mut File::open(&shipped_path)
            .unwrap_or_else(|e| panic!("open {shipped_path} (DAYZ_NAVMESH): {e}")),
    )
    .expect("read the navmesh");

    let (bytes, ours) = build(&capture_path, 1, range);
    let (bytes8, _) = build(&capture_path, 8, range);
    assert!(
        bytes == bytes8,
        "the build is not deterministic: 1 and 8 threads give different bytes"
    );

    let (mut compared, mut bad) = (0, Vec::new());
    for z in z0..=z1 {
        for x in x0..=x1 {
            let find = |set: &'_ NavMeshSet| {
                set.tiles
                    .iter()
                    .find(|t| t.data.header.x == x && t.data.header.y == z)
                    .map(|t| t.data.clone())
            };
            let (a, b) = (find(&ours), find(&shipped));
            match (a, b) {
                (None, None) => {}
                (Some(a), Some(b)) => {
                    compared += 1;
                    let d = differences(&a, &b);
                    if !d.is_empty() {
                        let more = d.len().saturating_sub(4);
                        let note = if more > 0 {
                            format!(" (+{more} more differences left out)")
                        } else {
                            String::new()
                        };
                        bad.push(format!(
                            "tile {x},{z}: {}{note}",
                            d[..d.len().min(4)].join("; ")
                        ));
                    }
                }
                (a, b) => bad.push(format!(
                    "tile {x},{z}: ours {}, shipped {}",
                    if a.is_some() { "present" } else { "missing" },
                    if b.is_some() { "present" } else { "missing" }
                )),
            }
        }
    }
    assert!(
        compared > 0,
        "no tile of the inner range exists in both meshes"
    );
    println!("compared {compared} tiles, {} differ", bad.len());
    assert!(
        bad.is_empty(),
        "{} of {compared} tiles differ\n{}",
        bad.len(),
        bad.join("\n")
    );
}
