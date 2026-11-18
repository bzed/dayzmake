//! Live comparison of a mesh built from a capture with the mesh that ships with DayZ (ignored: it
//! needs game data, which never goes into the repository).
//!
//! `DAYZ_NAVMESH` is the shipped `.nm`, `DAYZ_NAVMESH_CAPTURE` a `.dzgeo` capture of the same world
//! and `DAYZ_NAVMESH_INNER` the tile range `x0,z0,x1,z1` to compare. Only inner tiles can match: a
//! link whose far end is in a tile outside the capture is not made. The island filter is off, so the
//! polygon flags are not compared.

#![cfg(feature = "navmesh")]
#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::fs::File;

use dayzmake_core::navmesh::capture::Capture;
use dayzmake_core::navmesh::detour::{NavMeshSet, TileData};
use dayzmake_core::navmesh::generate::{GenerateOptions, Progress, Source, generate};
use dayzmake_core::warning::Warnings;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} (see the header of this file)"))
}

/// Builds the capture without the island filter on `threads` builders; returns the mesh bytes and set.
fn build(path: &str, threads: usize) -> (Vec<u8>, NavMeshSet) {
    let capture = Capture::read(&mut File::open(path).expect("open the capture")).expect("capture");
    let mut options = GenerateOptions::default();
    options.island_filter = false;
    options.threads = Some(threads);
    let mut warnings = Warnings::new();
    let report = generate(
        Source::Capture(capture),
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
    let mut bytes = Vec::new();
    report.set.write(&mut bytes).expect("write");
    (bytes, report.set)
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
    sections(
        &mut out,
        "off-mesh connection",
        &ours.off_mesh,
        &shipped.off_mesh,
    );
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
    let inner: Vec<i32> = env("DAYZ_NAVMESH_INNER")
        .split(',')
        .map(|n| n.trim().parse().expect("DAYZ_NAVMESH_INNER is x0,z0,x1,z1"))
        .collect();
    let [x0, z0, x1, z1] = inner[..] else {
        panic!("DAYZ_NAVMESH_INNER is x0,z0,x1,z1");
    };
    let shipped = NavMeshSet::read(&mut File::open(&shipped_path).expect("open the navmesh"))
        .expect("read the navmesh");

    let (bytes, ours) = build(&capture_path, 1);
    let (bytes8, _) = build(&capture_path, 8);
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
                        bad.push(format!("tile {x},{z}: {}", d[..d.len().min(4)].join("; ")));
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
    println!("compared {compared} tiles, {} differ", bad.len());
    assert!(
        bad.is_empty(),
        "{} of {compared} tiles differ\n{}",
        bad.len(),
        bad.join("\n")
    );
}
