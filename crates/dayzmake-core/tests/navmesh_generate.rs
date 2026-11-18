//! Tests of the navmesh tile scheduler and generator (spec 2.3, 8.4, 8.5; ruling 1).

#![cfg(feature = "navmesh")]
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dayzmake_core::navmesh::capture::{Capture, CaptureFile, CaptureFileOptions, CaptureWriter};
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

/// Writes a capture file with the `real` tiles and then `fake` records of `fake_size` bytes each whose
/// payload is a hole (a sparse file): their `(x, z)` are valid, the rest is zeros that nobody may read.
fn sparse_capture(
    path: &std::path::Path,
    params: &WorldParams,
    real: &[TileInput],
    fake: &[(i32, i32)],
    fake_size: u32,
) {
    use std::io::{Seek, SeekFrom, Write};
    let mut bytes = Vec::new();
    {
        let mut w = CaptureWriter::new(&mut bytes, params).expect("header");
        for t in real {
            w.add(t).expect("tile");
        }
    }
    let mut f = std::fs::File::create(path).expect("create");
    f.write_all(&bytes).expect("write");
    for &(x, z) in fake {
        f.write_all(&fake_size.to_le_bytes()).expect("size");
        f.write_all(&x.to_le_bytes()).expect("x");
        f.write_all(&z.to_le_bytes()).expect("z");
        f.seek(SeekFrom::Current(i64::from(fake_size) - 8))
            .expect("seek");
    }
    let end = f.stream_position().expect("pos");
    f.set_len(end).expect("len");
}

fn open_file(path: &std::path::Path) -> io::Result<CaptureFile> {
    CaptureFile::open(path, &CaptureFileOptions::new(), &mut Warnings::new())
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

type Watched = (io::Result<GenerateReport>, Warnings, Vec<Progress>);

/// Runs `generate` on its own thread and fails the test when it does not return within 20 s (600 s under coverage), so a
/// deadlock in the scheduler fails a test instead of hanging CI.
fn generate_watched(source: Source, options: &GenerateOptions) -> Watched {
    let options = options.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut warnings = Warnings::new();
        let mut seen = Vec::new();
        let result = generate(
            source,
            &options,
            &mut |p: &Progress| seen.push(*p),
            &mut warnings,
        );
        // The receiver is gone only when the test already failed.
        let _ = tx.send((result, warnings, seen));
    });
    // Under `cargo llvm-cov` (it sets LLVM_PROFILE_FILE) a tile build is about 100 times slower, and the
    // tests of this file run side by side: there the limit only has to catch a real hang.
    let limit = if std::env::var_os("LLVM_PROFILE_FILE").is_some() {
        600
    } else {
        20
    };
    rx.recv_timeout(Duration::from_secs(limit))
        .expect("generate hung")
}

fn run(source: Source, options: &GenerateOptions) -> (GenerateReport, Warnings) {
    let (result, warnings, _) = generate_watched(source, options);
    (result.expect("generate"), warnings)
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
    let (result, _, _) = generate_watched(Source::Capture(capture_of(&params, &tiles)), &options);
    let e = result.expect_err("needs base");
    assert!(
        e.to_string().contains("--skip-existing needs --base"),
        "{e}"
    );
}

#[test]
fn absurd_thread_and_connection_counts_are_refused_quickly() {
    let (params, tiles) = synthetic_world();
    for (threads, connections) in [(Some(1_000_000), None), (None, Some(1_000_000))] {
        let mut options = opts();
        options.threads = threads;
        options.connections = connections;
        let (result, _, _) =
            generate_watched(Source::Capture(capture_of(&params, &tiles)), &options);
        let e = result.expect_err("too many workers");
        let text = e.to_string();
        assert!(
            text.contains("at most 256")
                && (text.contains("threads") || text.contains("connections")),
            "{text}"
        );
    }
}

#[test]
fn absurd_host_count_is_refused_before_any_connection() {
    // The default connection count is one per host: 1000 hosts would spawn 1000 fetchers. The check
    // comes before the world params request, so nothing connects (the hosts are not reachable).
    let hosts: Vec<_> = (0..1000)
        .map(|_| Host::parse("127.0.0.1:1").expect("host"))
        .collect();
    let (result, _, _) = generate_watched(Source::Servers(hosts), &opts());
    let e = result.expect_err("too many hosts");
    let text = e.to_string();
    assert!(
        text.contains("hosts") && text.contains("at most 256"),
        "{text}"
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
    // One wording: the report keeps the bare reason and the warning names the tile once.
    assert_eq!(report.failed[0].2, "unknown cell class 99");
    let text = &warnings.warnings()[0].message;
    assert_eq!(text, "tile (2,0): unknown cell class 99");
}

#[test]
fn missing_capture_tiles_are_skipped_silently() {
    let (params, mut tiles) = synthetic_world();
    tiles.retain(|t| (t.x, t.z) != (1, 1));
    let mut options = opts();
    options.range = Some([0, 1, 2, 1]);
    let (result, warnings, seen) =
        generate_watched(Source::Capture(capture_of(&params, &tiles)), &options);
    let report = result.expect("generate");
    assert_eq!(report.built, 2);
    assert_eq!(report.failed.len() + report.fetch_errors.len(), 0);
    assert_eq!(warning_names(&warnings).len(), 0);
    assert!(report.stopped.is_none());
    // The skipped tile counts as dealt with: the progress reaches 100 %.
    let last = seen.last().expect("progress");
    assert_eq!((last.done, last.total), (3, 3));
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
    assert_eq!(set.tiles.len(), 2);
}

#[test]
fn failing_checkpoint_stops_the_run_but_returns_the_report() {
    let (params, tiles) = synthetic_world();
    let dir = tempfile::TempDir::new().expect("tmp");
    let mut options = opts();
    // The range contains tile (1,1), where the seed is: a running island filter would find it.
    options.range = Some([0, 0, 1, 1]);
    options.checkpoint = Some((1, dir.path().join("missing/out.nm")));
    let (report, warnings) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    let stopped = report.stopped.expect("stopped");
    assert!(stopped.contains("checkpoint"), "{stopped}");
    // The island filter did not run; what was assembled is in the set.
    assert!(report.islands.is_none());
    assert!(!warning_names(&warnings).contains(&"navmesh-seed"));
    assert_ne!(report.set.tiles.len(), 0);
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
    assert_eq!(report.seed, Some([-5000.0, 10.0, -5000.0]));
}

#[test]
fn report_seed_defaults_to_the_world_seed() {
    let (params, tiles) = synthetic_world();
    let mut options = opts();
    options.range = Some([0, 0, 0, 0]);
    let (report, _) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    assert_eq!(report.seed, Some(params.world.seed_position));
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
    assert_eq!(
        (report.fetch_errors[0].0, report.fetch_errors[0].1),
        (0, 0),
        "{:?}",
        report.fetch_errors
    );
    assert_eq!(
        (report.fetch_errors[8].0, report.fetch_errors[8].1),
        (2, 2),
        "{:?}",
        report.fetch_errors
    );
    // The message does not repeat the tile; the callers add it.
    assert!(
        !report.fetch_errors[0].2.starts_with("tile"),
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
    let mut options = opts();
    options.range = Some([0, 0, 2, 0]);
    let (result, _, seen) =
        generate_watched(Source::Capture(capture_of(&params, &tiles)), &options);
    result.expect("generate");
    let tuples: Vec<_> = seen.iter().map(|p| (p.done, p.total, p.built)).collect();
    assert_eq!(tuples.last().copied(), Some((3, 3, 3)));
    assert!(tuples.len() >= 3);
    assert!(tuples.windows(2).all(|w| w[0].0 <= w[1].0));
}

#[test]
fn a_huge_range_is_refused_before_anything_is_allocated() {
    let e = plan_tiles(&grid(1 << 14, 1 << 9), None).expect_err("too large");
    assert!(e.to_string().contains("tile range too large"), "{e}");
    assert!(e.to_string().contains("limit 4194304"), "{e}");
    // The range, not the world, counts.
    assert!(plan_tiles(&grid(1 << 14, 1 << 9), Some([0, 0, 9, 9])).is_ok());
}

#[test]
fn the_claim_window_bounds_the_reorder_buffer_and_the_output_is_unchanged() {
    let (params, tiles) = synthetic_world();
    let delay = Duration::from_millis(300);
    // Both servers are slow for the first tile of the plan, whichever fetcher claims it, so the
    // other fetcher runs ahead until the window stops it.
    let slow = || Faults::none().slow_tile((0, 0), delay);
    let a = FakeServer::start(params.clone(), tiles.clone(), slow());
    let b = FakeServer::start(params.clone(), tiles.clone(), slow());
    let hosts = vec![
        Host::parse(&a.addr().to_string()).expect("host"),
        Host::parse(&b.addr().to_string()).expect("host"),
    ];
    let mut options = opts();
    options.range = Some([0, 0, 1, 1]);
    let (reference, _) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    options.retry = quick_retry();
    options.connections = Some(2);
    options.window = Some(2);
    let (report, _) = run(Source::Servers(hosts), &options);
    assert_eq!(report.window, 2);
    assert!(report.peak_pending <= 2, "peak {}", report.peak_pending);
    assert_eq!(report.built, 4);
    assert_eq!(bytes_of(&report.set), bytes_of(&reference.set));
}

/// A base mesh made for another grid or tile size is refused, not silently mixed with new tiles.
#[test]
fn a_base_that_does_not_match_the_world_is_refused() {
    let (params, tiles) = synthetic_world();
    let mut options = opts();
    options.range = Some([0, 0, 0, 0]);
    let (first, _) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    let mismatches: [fn(&mut NavMeshSet); 5] = [
        |s| s.params.tile_width += 1.0,
        |s| s.params.tile_height += 1.0,
        |s| s.params.orig[0] = 5.0,
        |s| s.tiles_x += 1,
        |s| s.tiles_z += 1,
    ];
    for change in mismatches {
        let mut base = first.set.clone();
        change(&mut base);
        let mut options = opts();
        options.base = Some(base);
        let (result, _, _) =
            generate_watched(Source::Capture(capture_of(&params, &tiles)), &options);
        let text = result.expect_err("mismatch").to_string();
        assert!(
            text.contains("base navmesh does not match the world"),
            "{text}"
        );
    }
    // The matching base is still fine.
    let mut options = opts();
    options.base = Some(first.set.clone());
    options.range = Some([1, 0, 1, 0]);
    let (report, _) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    assert_eq!(report.set.tiles.len(), 2);
}

#[test]
fn a_range_of_a_huge_capture_reads_only_its_tiles() {
    // Five records of 300 MiB (1.5 GiB in all, sparse) outside the range: reading or parsing them would
    // exceed the address-space limit this test is run under (`prlimit --as=2000000000`) and take
    // minutes. Only the tiles of the range may be read.
    let (params, tiles) = synthetic_world();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("big.dzgeo");
    // On the grid, outside the range (0,0 to 1,1): indexed neither by position nor by size.
    let real: Vec<_> = tiles
        .iter()
        .filter(|t| t.x < 2 && t.z < 2)
        .cloned()
        .collect();
    let fake: Vec<(i32, i32)> = vec![(2, 0), (2, 1), (2, 2), (0, 2), (1, 2)];
    sparse_capture(&path, &params, &real, &fake, 300 << 20);
    let mut options = opts();
    options.range = Some([0, 0, 1, 1]);
    let mut o = CaptureFileOptions::new();
    o.range = Some([0, 0, 1, 1]);
    let file = CaptureFile::open(&path, &o, &mut Warnings::new()).expect("open");
    assert_eq!(file.len(), 4);
    let (from_file, _) = run(Source::CaptureFile(file), &options);
    let (from_memory, _) = run(Source::Capture(capture_of(&params, &tiles)), &options);
    assert_eq!(from_file.built, 4);
    assert_eq!(from_file.built, from_memory.built);
    assert_eq!(bytes_of(&from_file.set), bytes_of(&from_memory.set));
}

#[test]
fn a_capture_file_without_a_range_builds_its_bounding_box() {
    let (params, tiles) = synthetic_world();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("w.dzgeo");
    sparse_capture(&path, &params, &tiles, &[], 8);
    let file = open_file(&path).expect("open");
    let (report, _) = run(Source::CaptureFile(file), &opts());
    assert_eq!(report.built + report.empty, 9);
}

#[test]
fn refused_connections_do_not_lose_their_tiles() {
    // Connections 1 to 3 (0 is the world params probe) are closed at once; with one attempt each the
    // three fetchers that got them die, and their tiles must go to the fourth.
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none().refuse_connections(1, 3));
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.retry.attempts = 1;
    options.connections = Some(4);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.fetch_errors.len(), 0, "{:?}", report.fetch_errors);
    assert_eq!(report.built + report.empty, 9);
}

#[test]
fn a_refused_connect_is_retried_with_backoff() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none().refuse_connections(1, 2));
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.retry.attempts = 5;
    options.connections = Some(1);
    options.range = Some([0, 0, 1, 0]);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.fetch_errors.len(), 0, "{:?}", report.fetch_errors);
    assert_eq!(report.built + report.empty, 2);
}

#[test]
fn every_connection_dead_still_fails_the_tiles_once_each() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none().refuse_connections(1, 100));
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.retry.attempts = 1;
    options.connections = Some(3);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.built, 0);
    assert_eq!(report.fetch_errors.len(), 9, "{:?}", report.fetch_errors);
    let mut seen: Vec<_> = report.fetch_errors.iter().map(|e| (e.0, e.1)).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 9);
}

#[test]
fn a_hand_back_reaches_fetchers_that_wait_in_the_claim_window() {
    // Window 1: only the holder of index 0 may fetch, and it gets the closed connection 1 (0 was the
    // probe). The holder of index 1 waits for the window; the hand-back of index 0 must wake it.
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none().refuse_connections(1, 1));
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.retry.attempts = 1;
    options.connections = Some(2);
    options.window = Some(1);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.fetch_errors.len(), 0, "{:?}", report.fetch_errors);
    assert_eq!(report.built + report.empty, 9);
}

#[test]
fn a_tile_that_gets_a_wrong_answer_fails_alone_and_the_connections_live() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(
        params,
        tiles,
        Faults::none().fault_tile((1, 1), Fault::WrongMessage),
    );
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.connections = Some(3);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.fetch_errors.len(), 1, "{:?}", report.fetch_errors);
    assert_eq!((report.fetch_errors[0].0, report.fetch_errors[0].1), (1, 1));
    assert_eq!(report.built + report.empty, 8);
}

#[test]
fn a_tile_that_kills_two_connections_fails_alone() {
    // The server closes the connection on tile (1,1) every time: the first connection hands it back and
    // stops, the second fails it for good and goes on, the third is untouched.
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(
        params,
        tiles,
        Faults::none().fault_tile((1, 1), Fault::Close),
    );
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.retry.attempts = 1;
    options.connections = Some(3);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.fetch_errors.len(), 1, "{:?}", report.fetch_errors);
    assert_eq!((report.fetch_errors[0].0, report.fetch_errors[0].1), (1, 1));
    assert_eq!(report.built + report.empty, 8);
}

#[test]
fn recording_over_the_capture_being_read_is_refused() {
    let (params, tiles) = synthetic_world();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("w.dzgeo");
    sparse_capture(&path, &params, &tiles, &[], 8);
    let before = std::fs::read(&path).expect("read");
    let mut options = opts();
    options.record = Some(path.clone());
    let source = Source::CaptureFile(open_file(&path).expect("open"));
    let (result, _, _) = generate_watched(source, &options);
    let e = result.expect_err("same file");
    assert!(e.to_string().contains("capture being read"), "{e}");
    assert_eq!(std::fs::read(&path).expect("read"), before);
    // Also through a second name for the same file.
    #[cfg(unix)]
    {
        let link = dir.path().join("link.dzgeo");
        std::fs::hard_link(&path, &link).expect("link");
        options.record = Some(link);
        let source = Source::CaptureFile(open_file(&path).expect("open"));
        let (result, _, _) = generate_watched(source, &options);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).expect("read"), before);
    }
}

#[test]
fn a_malformed_record_fails_its_tile_and_the_run_goes_on() {
    let (params, tiles) = synthetic_world();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("w.dzgeo");
    let mut bytes = Vec::new();
    {
        let mut w = CaptureWriter::new(&mut bytes, &params).expect("header");
        for t in tiles.iter().filter(|t| (t.x, t.z) != (0, 0)) {
            w.add(t).expect("tile");
        }
    }
    // Tile (0,0) with a trailing byte: it indexes, and fails to parse when it is read.
    let mut bad = tiles
        .iter()
        .find(|t| (t.x, t.z) == (0, 0))
        .expect("tile")
        .encode();
    bad.push(0);
    bytes.extend_from_slice(&u32::try_from(bad.len()).expect("small").to_le_bytes());
    bytes.extend_from_slice(&bad);
    std::fs::write(&path, bytes).expect("write");
    let (report, warnings) = run(
        Source::CaptureFile(open_file(&path).expect("open")),
        &opts(),
    );
    assert_eq!(report.failed.len(), 1);
    assert_eq!((report.failed[0].0, report.failed[0].1), (0, 0));
    assert!(
        report.failed[0].2.contains("capture tile (0,0)"),
        "{:?}",
        report.failed
    );
    assert_eq!(report.fetch_errors.len(), 0);
    assert!(warning_names(&warnings).contains(&"navmesh-tile-failed"));
}

#[test]
fn a_range_only_indexes_its_tiles() {
    let (params, tiles) = synthetic_world();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("w.dzgeo");
    sparse_capture(&path, &params, &tiles, &[], 8);
    let mut o = CaptureFileOptions::new();
    o.range = Some([0, 0, 1, 1]);
    let file = CaptureFile::open(&path, &o, &mut Warnings::new()).expect("open");
    assert_eq!(file.len(), 4);
    let mut options = opts();
    options.range = Some([0, 0, 1, 1]);
    let (report, _) = run(Source::CaptureFile(file), &options);
    assert_eq!(report.built + report.empty, 4);
}

#[test]
fn a_run_wider_than_the_indexed_range_is_refused() {
    let (params, tiles) = synthetic_world();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("w.dzgeo");
    sparse_capture(&path, &params, &tiles, &[], 8);
    let mut o = CaptureFileOptions::new();
    o.range = Some([0, 0, 1, 1]);
    let open = || CaptureFile::open(&path, &o, &mut Warnings::new()).expect("open");
    assert_eq!(open().indexed_range(), Some([0, 0, 1, 1]));
    for range in [Some([0, 0, 2, 2]), Some([1, 1, 2, 1]), Some([0, 0, -1, -1])] {
        let mut options = opts();
        options.range = range;
        let (result, _, _) = generate_watched(Source::CaptureFile(open()), &options);
        let e = result.expect_err("wider than the index");
        let text = e.to_string();
        assert!(text.contains("indexed for the range 0,0,1,1"), "{text}");
        assert!(text.contains("the run asks for"), "{text}");
    }
    // Inside the indexed range, and the default range (the bounds of what was indexed), are fine.
    for range in [Some([0, 0, 1, 1]), Some([1, 0, 1, 1]), None] {
        let mut options = opts();
        options.range = range;
        let (report, _) = run(Source::CaptureFile(open()), &options);
        assert!(report.built + report.empty > 0);
    }
}

#[test]
fn a_single_connection_is_not_closed_and_reopened_while_it_waits() {
    // Window 1: the fetcher waits for the assembler after every tile. With one connection nobody else
    // can use the server, so the connection stays: the probe and the fetcher, two connections in all.
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none());
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = opts();
    options.retry = quick_retry();
    options.connections = Some(1);
    options.window = Some(1);
    let (report, _) = run(Source::Servers(vec![host]), &options);
    assert_eq!(report.built + report.empty, 9);
    assert_eq!(server.connections(), 2);
}

#[test]
fn a_poison_tile_with_waiting_fetchers_still_fails_alone() {
    // Window 2 makes the fetchers wait (and close their connections) while one retries the poison tile;
    // a fetcher that forgot it had delivered would then fail the handed-back tile without counting it,
    // and every connection would die on it. Looped, because which fetcher meets the tile is a race.
    for round in 0..10 {
        let (params, tiles) = synthetic_world();
        let server = FakeServer::start(
            params,
            tiles,
            Faults::none().fault_tile((1, 0), Fault::Close),
        );
        let host = Host::parse(&server.addr().to_string()).expect("host");
        let mut options = opts();
        options.retry = quick_retry();
        options.retry.attempts = 1;
        options.connections = Some(3);
        options.window = Some(2);
        let (report, _) = run(Source::Servers(vec![host]), &options);
        assert_eq!(
            report.fetch_errors.len(),
            1,
            "round {round}: {:?}",
            report.fetch_errors
        );
        assert_eq!((report.fetch_errors[0].0, report.fetch_errors[0].1), (1, 0));
        assert_eq!(report.built + report.empty, 8, "round {round}");
    }
}
