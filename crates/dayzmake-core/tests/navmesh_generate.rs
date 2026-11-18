//! Tests of the navmesh tile scheduler and generator (spec 2.3, 8.4, 8.5; ruling 1).

#![cfg(feature = "navmesh")]
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::path::Path;
use std::time::Duration;

use dayzmake_core::navmesh::capture::{Capture, CaptureWriter};
use dayzmake_core::navmesh::client::{Host, RetryPolicy};
use dayzmake_core::navmesh::detour::NavMeshSet;
use dayzmake_core::navmesh::generate::{
    GenerateOptions, GenerateReport, Progress, Source, generate, plan_tiles, probe,
};
use dayzmake_core::navmesh::wire::{TileInput, WorldBlock, WorldParams};
use dayzmake_core::warning::Warnings;

#[path = "support/navmesh_server.rs"]
mod navmesh_server;
#[path = "support/navmesh_world.rs"]
mod navmesh_world;

use navmesh_server::{FakeServer, Fault, Faults};
use navmesh_world::synthetic_world;

fn capture_of(params: &WorldParams, tiles: &[TileInput]) -> Capture {
    let mut bytes = Vec::new();
    {
        let mut w = CaptureWriter::new(&mut bytes, params).expect("header");
        for t in tiles {
            w.add(t).expect("tile");
        }
    }
    Capture::read(&mut bytes.as_slice()).expect("read capture")
}

fn bytes_of(set: &NavMeshSet) -> Vec<u8> {
    let mut out = Vec::new();
    set.write(&mut out).expect("write");
    out
}

fn quick_retry() -> RetryPolicy {
    let mut p = RetryPolicy::default();
    p.attempts = 2;
    p.delay_unit = Duration::ZERO;
    p.connect_timeout = Duration::from_secs(5);
    p.recv_timeout = Duration::from_secs(10);
    p
}

/// Options with two builders: every test runs in parallel, and under the `prlimit --as` of this plan
/// the address space of cores-1 builders per test (one malloc arena each) would not fit.
fn opts() -> GenerateOptions {
    let mut o = GenerateOptions::default();
    o.threads = Some(2);
    o
}

fn run(source: Source, options: &GenerateOptions) -> (GenerateReport, Warnings) {
    let mut warnings = Warnings::new();
    let report =
        generate(source, options, &mut |_: &Progress| {}, &mut warnings).expect("generate");
    (report, warnings)
}

fn warning_names(w: &Warnings) -> Vec<&'static str> {
    w.warnings().iter().filter_map(|w| w.name).collect()
}

fn grid(n: i32, block: i32) -> WorldBlock {
    let mut w = WorldBlock::default();
    w.tiles_x = n;
    w.tiles_z = n;
    w.block_size = block;
    w
}

#[test]
fn plan_order_blocks() {
    let plan = plan_tiles(&grid(5, 2), None).expect("plan");
    assert_eq!(plan.len(), 25);
    assert_eq!(
        plan[..11],
        [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (3, 0),
            (2, 1),
            (3, 1),
            (4, 0),
            (4, 1),
            (0, 2)
        ]
    );
}

#[test]
fn plan_clamps_range() {
    let all = plan_tiles(&grid(5, 2), None).expect("all");
    assert_eq!(
        plan_tiles(&grid(5, 2), Some([-3, -3, 99, 99])).expect("clamped"),
        all
    );
    // A negative upper bound means the end of the grid.
    assert_eq!(
        plan_tiles(&grid(5, 2), Some([0, 0, -1, -1])).expect("open"),
        all
    );
    let e = plan_tiles(&grid(5, 2), Some([2, 2, 1, 1])).expect_err("inverted");
    assert!(e.to_string().contains("empty tile range"), "{e}");
    // The blocks start at the range's corner, not on a global grid.
    let sub = plan_tiles(&grid(5, 2), Some([1, 1, 3, 1])).expect("sub");
    assert_eq!(sub, [(1, 1), (2, 1), (3, 1)]);
}

#[test]
fn generate_from_capture() {
    let (params, tiles) = synthetic_world();
    let (report, warnings) = run(Source::Capture(capture_of(&params, &tiles)), &opts());
    assert_eq!(report.built, 9, "{:?}", report.failed);
    assert_eq!(report.failed.len(), 0);
    assert_eq!(report.fetch_errors.len(), 0);
    assert_eq!(report.set.tiles.len(), 9);
    assert!(report.islands.expect("islands").reachable > 0);
    assert!(!warning_names(&warnings).contains(&"navmesh-seed"));
    let bytes = bytes_of(&report.set);
    let back = NavMeshSet::read(&mut bytes.as_slice()).expect("read back");
    assert_eq!(bytes_of(&back), bytes);
}

#[test]
fn generate_from_server_equals_capture() {
    let (params, tiles) = synthetic_world();
    let mut options = opts();
    options.range = Some([0, 0, 1, 1]);
    let (from_capture, _) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    let server = FakeServer::start(params, tiles, Faults::none());
    options.retry = quick_retry();
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let (from_server, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(bytes_of(&from_server.set), bytes_of(&from_capture.set));
}

#[test]
fn probe_returns_world_params() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params.clone(), tiles, Faults::none());
    let host = Host::parse(&server.addr().to_string()).expect("host");
    assert_eq!(
        probe(&host, &quick_retry()).expect("probe").world.tiles_x,
        3
    );
}

#[test]
fn threads_do_not_change_output() {
    let (params, tiles) = synthetic_world();
    let a = FakeServer::start(params.clone(), tiles.clone(), Faults::none());
    let b = FakeServer::start(params, tiles, Faults::none());
    let hosts = || {
        vec![
            Host::parse(&a.addr().to_string()).expect("host"),
            Host::parse(&b.addr().to_string()).expect("host"),
        ]
    };
    let mut outputs = Vec::new();
    for threads in [1, 4] {
        let mut options = opts();
        options.retry = quick_retry();
        options.threads = Some(threads);
        options.range = Some([0, 0, 1, 1]);
        options.connections = Some(2);
        let (report, _) = run(Source::Servers(hosts()), &options);
        assert_eq!(report.built, 4);
        outputs.push(bytes_of(&report.set));
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn resume_equals_full_run() {
    let (params, tiles) = synthetic_world();
    let capture = capture_of(&params, &tiles);
    let mut options = opts();
    options.range = Some([0, 0, 2, 1]);
    let (full, _) = run(Source::Capture(capture.clone()), &options);
    let mut options = opts();
    options.range = Some([0, 0, 2, 0]);
    let (a, _) = run(Source::Capture(capture.clone()), &options);
    assert_eq!(a.built, 3);
    let mut options = opts();
    options.range = Some([0, 0, 2, 1]);
    options.base = Some(a.set);
    options.skip_existing = true;
    let (b, _) = run(Source::Capture(capture), &options);
    assert_eq!(b.built, 3);
    assert_eq!(bytes_of(&b.set), bytes_of(&full.set));
}

#[test]
fn skip_existing_needs_base() {
    let (params, tiles) = synthetic_world();
    let mut options = opts();
    options.skip_existing = true;
    let e = generate(
        Source::Capture(capture_of(&params, &tiles)),
        &options,
        &mut |_: &Progress| {},
        &mut Warnings::new(),
    )
    .expect_err("needs base");
    assert!(
        e.to_string().contains("--skip-existing needs --base"),
        "{e}"
    );
}

#[test]
fn failed_tile_reported_and_run_continues() {
    let (params, mut tiles) = synthetic_world();
    let bad = tiles
        .iter_mut()
        .find(|t| (t.x, t.z) == (2, 0))
        .expect("tile");
    bad.cell_class = 99;
    let mut options = opts();
    options.range = Some([1, 0, 2, 0]);
    let (report, warnings) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    assert_eq!(report.built, 1);
    assert_eq!(report.failed.len(), 1);
    assert_eq!((report.failed[0].0, report.failed[0].1), (2, 0));
    assert!(warning_names(&warnings).contains(&"navmesh-tile-failed"));
}

#[test]
fn missing_capture_tiles_are_skipped_silently() {
    let (params, mut tiles) = synthetic_world();
    tiles.retain(|t| (t.x, t.z) != (1, 1));
    let mut options = opts();
    options.range = Some([0, 1, 2, 1]);
    let (report, warnings) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    assert_eq!(report.built, 2);
    assert_eq!(report.failed.len() + report.fetch_errors.len(), 0);
    assert_eq!(warning_names(&warnings).len(), 0);
}

#[test]
fn checkpoint_writes_file() {
    let (params, tiles) = synthetic_world();
    let dir = tempfile::TempDir::new().expect("tmp");
    let path = dir.path().join("out.nm");
    let mut options = opts();
    options.range = Some([0, 0, 2, 0]);
    options.checkpoint = Some((2, path.clone()));
    let (report, _) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    assert_eq!(report.built, 3);
    let set = NavMeshSet::read(&mut std::fs::File::open(&path).expect("file")).expect("readable");
    assert!(!set.tiles.is_empty() && set.tiles.len() <= 3);
}

#[test]
fn failing_checkpoint_is_an_error_after_the_run() {
    let (params, tiles) = synthetic_world();
    let mut options = opts();
    options.checkpoint = Some((
        1,
        Path::new("/nonexistent-dir-for-test/out.nm").to_path_buf(),
    ));
    let e = generate(
        Source::Capture(capture_of(&params, &tiles)),
        &options,
        &mut |_: &Progress| {},
        &mut Warnings::new(),
    )
    .expect_err("checkpoint fails");
    assert!(e.to_string().contains("checkpoint"), "{e}");
}

#[test]
fn record_then_generate_from_record_is_equal() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none());
    let dir = tempfile::TempDir::new().expect("tmp");
    let path = dir.path().join("run.dzgeo");
    let mut options = opts();
    options.retry = quick_retry();
    options.record = Some(path.clone());
    options.range = Some([0, 0, 1, 1]);
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let (live, _) = run(Source::Servers(vec![host]), &options);
    let capture = Capture::read(&mut std::fs::File::open(&path).expect("file")).expect("capture");
    assert_eq!(capture.tiles().len(), 4);
    let (replay, _) = run(Source::Capture(capture), &opts());
    assert_eq!(bytes_of(&replay.set), bytes_of(&live.set));
}

#[test]
fn seed_in_sea_warns() {
    let (params, tiles) = synthetic_world();
    let mut options = opts();
    options.range = Some([0, 0, 1, 0]);
    options.seed = Some([-5000.0, 10.0, -5000.0]);
    let (report, warnings) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    assert!(report.islands.is_none());
    assert!(warning_names(&warnings).contains(&"navmesh-seed"));
}

#[test]
fn dead_connections_end_the_run_with_fetch_errors_in_plan_order() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::always(Fault::Close));
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.connections = Some(2);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.built, 0);
    assert_eq!(report.fetch_errors.len(), 9);
    // The first tile of the plan comes first, whichever fetcher claimed it.
    assert!(
        report.fetch_errors[0].starts_with("tile (0,0):"),
        "{:?}",
        report.fetch_errors
    );
    assert!(
        report.fetch_errors[8].starts_with("tile (2,2):"),
        "{:?}",
        report.fetch_errors
    );
}

#[test]
fn a_stopped_fetcher_hands_on_its_tile_and_the_others_continue() {
    let (params, tiles) = synthetic_world();
    let good = FakeServer::start(params.clone(), tiles.clone(), Faults::none());
    let bad = FakeServer::start(params, tiles, Faults::always(Fault::Close));
    let hosts = vec![
        Host::parse(&good.addr().to_string()).expect("host"),
        Host::parse(&bad.addr().to_string()).expect("host"),
    ];
    let mut options = opts();
    options.retry = quick_retry();
    options.range = Some([0, 0, 2, 0]);
    let (report, _) = run(Source::Servers(hosts), &options);
    assert_eq!(
        report.built + report.fetch_errors.len(),
        3,
        "{:?}",
        report.fetch_errors
    );
    assert_eq!(report.failed.len(), 0);
}

#[test]
fn wire_errors_fail_the_tile_and_the_fetcher_continues() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::script(&[Fault::Malformed]));
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.connections = Some(1);
    options.range = Some([0, 0, 2, 0]);
    let (report, warnings) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.failed.len(), 1);
    assert_eq!((report.failed[0].0, report.failed[0].1), (0, 0));
    assert_eq!(report.built, 2);
    assert!(warning_names(&warnings).contains(&"navmesh-tile-failed"));
}

#[test]
fn progress_reports_every_applied_tile_and_the_end() {
    let (params, tiles) = synthetic_world();
    let mut seen = Vec::new();
    let mut warnings = Warnings::new();
    let mut options = opts();
    options.range = Some([0, 0, 2, 0]);
    generate(
        Source::Capture(capture_of(&params, &tiles)),
        &options,
        &mut |p: &Progress| seen.push((p.done, p.total, p.built)),
        &mut warnings,
    )
    .expect("generate");
    assert_eq!(seen.last().copied(), Some((3, 3, 3)));
    assert!(seen.len() >= 3);
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
}
