//! The navmesh generator: plans the tiles of a run, fetches them (or reads them from a capture),
//! builds them in parallel and assembles the mesh (spec 2.3, 8.4, 8.5).
//!
//! # Threads
//!
//! Three stages run inside one [`std::thread::scope`], joined by bounded channels:
//!
//! * **fetchers** (one per connection) claim the next plan index from a shared atomic counter, fetch the
//!   tile and send it to the builders (queue bound 64);
//! * **builders** (`threads`, default cores - 1, at least 1) build the tiles and send the results to the
//!   assembler (queue bound `builders * 4`);
//! * the **assembler** is the calling thread. It keeps a reorder buffer keyed by plan index and applies
//!   the results strictly in plan order, so the output is byte-identical for any `threads` and
//!   `connections` (this deliberately differs from the completion order of the original, see the
//!   spec's section 8.4).
//!
//! The graph is acyclic and the assembler only ever waits for the result channel, which it always
//! drains into its (unbounded) reorder buffer. So every blocked `send` is eventually released: the
//! assembler never waits for a send, the builders therefore never wait for it, and the fetchers
//! wait only for builders. Each claimed plan index produces exactly one message (a tile, a skipped
//! tile or an error), so the assembler's next-expected index is always eventually delivered. A
//! fetcher that stops after a final network error hands its claimed index on as an error first; the
//! last fetcher to stop claims every index nobody has claimed and reports it as a fetch error. A
//! failing checkpoint or capture write sets a stop flag: fetchers stop claiming, the channels close
//! as the stages finish and the run ends with the error.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::error::ErrorExt;
use crate::navmesh::build::{BuildOptions, TileOutcome, build_tile};
use crate::navmesh::capture::{Capture, CaptureWriter};
use crate::navmesh::client::{Client, FetchError, Host, RetryPolicy};
use crate::navmesh::detour::{NavMesh, NavMeshParams, NavMeshSet};
use crate::navmesh::islands::{IslandReport, filter_islands};
use crate::navmesh::wire::{TileInput, WorldBlock, WorldParams};
use crate::warning::{Warning, WarningSink};

/// Capacity of the fetch to build queue (spec 8.4).
const FETCH_QUEUE: usize = 64;
/// `maxPolys` of a fresh mesh (spec 8.1).
const MAX_POLYS: i32 = 1 << 20;

/// Where the tile geometry comes from.
#[derive(Debug)]
#[non_exhaustive]
pub enum Source {
    /// One or more data servers. Fetcher `k` uses host `k mod hosts` (spec 3.7); the world params come
    /// from the first host.
    Servers(Vec<Host>),
    /// A capture (`.dzgeo`); tiles it lacks are skipped silently.
    Capture(Capture),
}

/// Options of [`generate`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GenerateOptions {
    /// The tile range `[x0, z0, x1, z1]` (inclusive, clamped, see [`plan_tiles`]); `None`: the whole
    /// world, or for a capture the bounding box of its tiles.
    pub range: Option<[i32; 4]>,
    /// A mesh to continue from: its grid, tile size and origin are kept and new tiles replace the
    /// tiles of their cell.
    pub base: Option<NavMeshSet>,
    /// Leave out the tiles that `base` already has (needs `base`).
    pub skip_existing: bool,
    /// Every `N` finished tiles (built, empty and failed) write the mesh atomically to the path. A
    /// failing checkpoint stops fetching and is returned as an error after the run; `N` below 1 means 1.
    pub checkpoint: Option<(usize, PathBuf)>,
    /// Write a capture of the fetched tiles to this path, in plan order.
    pub record: Option<PathBuf>,
    /// The number of builder threads; `None`: the cores minus one, at least 1.
    pub threads: Option<usize>,
    /// The number of connections (fetcher threads); `None`: one per host (1 for a capture).
    pub connections: Option<usize>,
    /// The island filter seed; `None`: the world's `seedPosition`.
    pub seed: Option<[f32; 3]>,
    /// Run the island filter when the world asks for it (`filterIsolatedIslands`). On by default.
    pub island_filter: bool,
    /// Options of the tile build.
    pub build: BuildOptions,
    /// Connect and retry rules of the data server client.
    pub retry: RetryPolicy,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            range: None,
            base: None,
            skip_existing: false,
            checkpoint: None,
            record: None,
            threads: None,
            connections: None,
            seed: None,
            island_filter: true,
            build: BuildOptions::default(),
            retry: RetryPolicy::default(),
        }
    }
}

/// How far a run has come; passed to the progress callback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Progress {
    /// Tiles finished: built, empty and failed.
    pub done: usize,
    /// Tiles planned.
    pub total: usize,
    /// Tiles built.
    pub built: usize,
    /// Tiles without walkable surface.
    pub empty: usize,
    /// Tiles that failed.
    pub failed: usize,
    /// Time since the run started.
    pub elapsed: Duration,
}

/// The result of [`generate`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct GenerateReport {
    /// The mesh. The caller writes it (see [`NavMeshSet::write_atomic`]).
    pub set: NavMeshSet,
    /// Tiles built.
    pub built: usize,
    /// Tiles without walkable surface (an existing tile of the cell was removed).
    pub empty: usize,
    /// Tiles that failed: `(x, z, message)`, in plan order.
    pub failed: Vec<(i32, i32, String)>,
    /// Tiles that could not be fetched (final network errors): `tile (x,z): message`, in plan order.
    pub fetch_errors: Vec<String>,
    /// Tiles that had out-of-range door part ids (spec 12.3); the ids were ignored.
    pub stray_tiles: usize,
    /// The island filter result; `None` when it did not run or found no polygon near the seed.
    pub islands: Option<IslandReport>,
    /// Wall time of the run in seconds. Reported only; it is not part of the output.
    pub seconds: f64,
}

/// The tiles of a run, in the order of spec 8.4.
///
/// The range is clamped to the grid: lower bounds to 0, an upper bound below 0 or beyond the grid to the
/// last tile. The range is cut into square blocks of `blockSize` tiles starting at the range's
/// own corner; blocks go row by row (z outer), and inside a block the tiles go row by row too.
///
/// # Errors
///
/// "empty tile range" when nothing is left of the range.
pub fn plan_tiles(world: &WorldBlock, range: Option<[i32; 4]>) -> io::Result<Vec<(i32, i32)>> {
    let [rx0, rz0, rx1, rz1] = range.unwrap_or([0, 0, -1, -1]);
    let last = |n: i32| n.saturating_sub(1);
    let (x0, z0) = (rx0.max(0), rz0.max(0));
    let x1 = if rx1 < 0 {
        last(world.tiles_x)
    } else {
        rx1.min(last(world.tiles_x))
    };
    let z1 = if rz1 < 0 {
        last(world.tiles_z)
    } else {
        rz1.min(last(world.tiles_z))
    };
    if world.tiles_x <= 0 || world.tiles_z <= 0 || x0 > x1 || z0 > z1 {
        return Err(error!("empty tile range"));
    }
    let block = world.block_size.max(1);
    let step = usize::try_from(block).unwrap_or(1);
    let mut plan = Vec::new();
    for bz in (z0..=z1).step_by(step) {
        for bx in (x0..=x1).step_by(step) {
            let ez = bz.saturating_add(block - 1).min(z1);
            let ex = bx.saturating_add(block - 1).min(x1);
            for z in bz..=ez {
                for x in bx..=ex {
                    plan.push((x, z));
                }
            }
        }
    }
    Ok(plan)
}

/// Asks the data server at `host` for the world params (one attempt, no retry of the request).
///
/// # Errors
///
/// Fails when the server cannot be reached (the message says how to start the data server) or its
/// answer is malformed.
pub fn probe(host: &Host, policy: &RetryPolicy) -> io::Result<WorldParams> {
    Client::connect(host, policy)?.world_params()
}

/// What a fetcher found for one plan index.
enum Fetch {
    Tile(TileInput),
    /// The capture lacks the tile.
    Skipped,
    /// The payload was malformed.
    Wire(String),
    /// A final network error.
    Net(String),
}

struct Fetched {
    idx: usize,
    what: Fetch,
}

/// What a builder made of one plan index.
enum Outcome {
    Tile(Box<TileOutcome>),
    Skipped,
    Failed(String),
    Net(String),
}

struct Built {
    idx: usize,
    outcome: Outcome,
    /// The fetched input, kept only when recording.
    input: Option<TileInput>,
}

/// Where the fetchers get their tiles.
enum FetchFrom<'a> {
    Servers(&'a [Host]),
    Capture(&'a Capture),
}

/// What the fetchers share.
struct FetchShared<'a> {
    plan: &'a [(i32, i32)],
    next: &'a AtomicUsize,
    stop: &'a AtomicBool,
    alive: &'a AtomicUsize,
    world: &'a WorldParams,
    from: FetchFrom<'a>,
    policy: &'a RetryPolicy,
}

fn net_message(x: i32, z: i32, e: &dyn std::fmt::Display) -> String {
    format!("tile ({x},{z}): {e}")
}

fn fetch_one(
    shared: &FetchShared<'_>,
    slot: usize,
    client: &mut Option<Client>,
    at: (i32, i32),
) -> Fetch {
    let (col, row) = at;
    match &shared.from {
        FetchFrom::Capture(capture) => capture
            .tile(col, row)
            .map_or(Fetch::Skipped, |tile| Fetch::Tile(tile.clone())),
        FetchFrom::Servers(hosts) => {
            if client.is_none() {
                let Some(host) = hosts.get(slot % hosts.len().max(1)) else {
                    return Fetch::Net(net_message(col, row, &"no data server"));
                };
                match Client::connect(host, shared.policy) {
                    Ok(conn) => *client = Some(conn),
                    Err(e) => return Fetch::Net(net_message(col, row, &e)),
                }
            }
            let Some(conn) = client.as_mut() else {
                return Fetch::Net(net_message(col, row, &"not connected"));
            };
            match conn.tile(col, row, shared.world) {
                Ok(tile) => Fetch::Tile(tile),
                Err(FetchError::Wire(e)) => Fetch::Wire(e.to_string()),
                Err(FetchError::Network(e)) => Fetch::Net(net_message(col, row, &e)),
            }
        }
    }
}

fn fetcher(shared: &FetchShared<'_>, slot: usize, tx: &SyncSender<Fetched>) {
    let mut client: Option<Client> = None;
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        let idx = shared.next.fetch_add(1, Ordering::Relaxed);
        let Some(&at) = shared.plan.get(idx) else {
            break;
        };
        let what = fetch_one(shared, slot, &mut client, at);
        let fatal = matches!(what, Fetch::Net(_));
        if tx.send(Fetched { idx, what }).is_err() {
            return;
        }
        if fatal {
            // Spec 3.6: this fetcher stops, the others continue.
            break;
        }
    }
    // The last fetcher to stop reports every index nobody claimed, so the run still ends.
    if shared.alive.fetch_sub(1, Ordering::AcqRel) == 1 && !shared.stop.load(Ordering::Relaxed) {
        loop {
            let idx = shared.next.fetch_add(1, Ordering::Relaxed);
            let Some(&(x, z)) = shared.plan.get(idx) else {
                break;
            };
            let what = Fetch::Net(net_message(x, z, &"not fetched: every connection failed"));
            if tx.send(Fetched { idx, what }).is_err() {
                return;
            }
        }
    }
}

fn builder(
    world: &WorldParams,
    options: &BuildOptions,
    record: bool,
    rx: &Mutex<Receiver<Fetched>>,
    tx: &SyncSender<Built>,
) {
    loop {
        // The lock is held only while waiting for the next tile.
        let msg = rx.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok(Fetched { idx, what }) = msg else {
            return;
        };
        let (outcome, input) = match what {
            Fetch::Tile(input) => {
                let outcome = match build_tile(world, &input, options) {
                    Ok(o) => Outcome::Tile(Box::new(o)),
                    Err(e) => Outcome::Failed(e.to_string()),
                };
                (outcome, record.then_some(input))
            }
            Fetch::Skipped => (Outcome::Skipped, None),
            Fetch::Wire(m) => (Outcome::Failed(m), None),
            Fetch::Net(m) => (Outcome::Net(m), None),
        };
        if tx
            .send(Built {
                idx,
                outcome,
                input,
            })
            .is_err()
        {
            return;
        }
    }
}

/// The assembler's state: the mesh and the report being made.
struct Assembler<'a> {
    mesh: NavMesh,
    plan: &'a [(i32, i32)],
    report: GenerateReport,
    recorder: Option<CaptureWriter<File>>,
    checkpoint: Option<(usize, &'a Path)>,
    grid: (i32, i32),
    stop: &'a AtomicBool,
    /// The first fatal error (checkpoint or capture write).
    fatal: Option<io::Error>,
    start: Instant,
}

impl Assembler<'_> {
    fn finished(&self) -> usize {
        self.report.built + self.report.empty + self.report.failed.len()
    }

    fn fail(&mut self, (x, z): (i32, i32), message: String, warnings: &mut dyn WarningSink) {
        warnings.warn(Warning::new(
            Some("navmesh-tile-failed"),
            format!("tile ({x},{z}): {message}"),
        ));
        self.report.failed.push((x, z, message));
    }

    fn apply(&mut self, msg: Built, warnings: &mut dyn WarningSink) {
        let Some(&at) = self.plan.get(msg.idx) else {
            return;
        };
        let finished_before = self.finished();
        if let (Some(w), Some(input)) = (self.recorder.as_mut(), msg.input.as_ref())
            && let Err(e) = w.add(input)
            && self.fatal.is_none()
        {
            self.fatal = Some(error!("cannot write the capture: {e}"));
            self.stop.store(true, Ordering::Relaxed);
        }
        match msg.outcome {
            Outcome::Skipped => {}
            Outcome::Net(m) => self.report.fetch_errors.push(m),
            Outcome::Failed(m) => self.fail(at, m, warnings),
            Outcome::Tile(outcome) => match *outcome {
                TileOutcome::Empty => {
                    self.mesh.remove_tile_at(at.0, at.1);
                    self.report.empty += 1;
                }
                TileOutcome::Built {
                    data,
                    stray_part_ids,
                    ..
                } => match self.mesh.set_tile(data) {
                    Ok(_) => {
                        self.report.built += 1;
                        if stray_part_ids > 0 {
                            self.report.stray_tiles += 1;
                            warnings.warn(Warning::new(
                                Some("navmesh-stray-part"),
                                format!(
                                    "tile ({},{}): {stray_part_ids} polygons had out-of-range door part ids, which were ignored",
                                    at.0, at.1
                                ),
                            ));
                        }
                    }
                    Err(e) => self.fail(at, e.to_string(), warnings),
                },
            },
        }
        self.checkpoint_if_due(finished_before);
    }

    /// Writes the checkpoint when the tile just applied was the `N`th finished one.
    fn checkpoint_if_due(&mut self, finished_before: usize) {
        let Some((every, path)) = self.checkpoint else {
            return;
        };
        let done = self.finished();
        if self.fatal.is_some() || done == finished_before || !done.is_multiple_of(every) {
            return;
        }
        let set = self.mesh.to_set(self.grid.0, self.grid.1);
        if let Err(e) = set.write_atomic(path) {
            self.fatal = Some(error!("checkpoint {}: {e}", path.display()));
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    fn progress(&self, progress: &mut dyn FnMut(&Progress)) {
        progress(&Progress {
            done: self.finished(),
            total: self.plan.len(),
            built: self.report.built,
            empty: self.report.empty,
            failed: self.report.failed.len(),
            elapsed: self.start.elapsed(),
        });
    }
}

/// The bounding range of the tile coordinates of a capture.
fn capture_range(capture: &Capture) -> Option<[i32; 4]> {
    let tiles = capture.tiles();
    let first = tiles.first()?;
    let mut r = [first.x, first.z, first.x, first.z];
    for t in tiles {
        r = [r[0].min(t.x), r[1].min(t.z), r[2].max(t.x), r[3].max(t.z)];
    }
    Some(r)
}

/// Creates the target mesh (spec 2.3 step 3, 8.1) and its grid size.
fn create_mesh(world: &WorldBlock, base: Option<&NavMeshSet>) -> io::Result<(NavMesh, (i32, i32))> {
    if let Some(set) = base {
        return Ok((
            NavMesh::from_set(set).prepend_error("cannot use the base mesh")?,
            (set.tiles_x, set.tiles_z),
        ));
    }
    let max_tiles = world
        .tiles_x
        .checked_mul(world.tiles_z)
        .filter(|n| *n > 0)
        .ok_or_else(|| error!("invalid tile grid {} x {}", world.tiles_x, world.tiles_z))?;
    let params = NavMeshParams {
        orig: [0.0; 3],
        tile_width: world.tile_width,
        tile_height: world.tile_height,
        max_tiles,
        max_polys: MAX_POLYS,
    };
    Ok((NavMesh::new(params)?, (world.tiles_x, world.tiles_z)))
}

/// Runs the fetchers and builders on scoped threads and applies their results in plan order on the
/// calling thread (see the module docs).
fn run_pipeline(
    asm: &mut Assembler<'_>,
    world: &WorldParams,
    from: FetchFrom<'_>,
    default_connections: usize,
    options: &GenerateOptions,
    progress: &mut dyn FnMut(&Progress),
    warnings: &mut dyn WarningSink,
) {
    let builders = options
        .threads
        .unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1))
        })
        .max(1);
    let connections = options.connections.unwrap_or(default_connections).max(1);
    let (plan, stop) = (asm.plan, asm.stop);
    let next = AtomicUsize::new(0);
    let alive = AtomicUsize::new(connections);
    let shared = FetchShared {
        plan,
        next: &next,
        stop,
        alive: &alive,
        world,
        from,
        policy: &options.retry,
    };
    let (fetch_tx, fetch_rx) = sync_channel::<Fetched>(FETCH_QUEUE);
    let (built_tx, built_rx) = sync_channel::<Built>(builders.saturating_mul(4));
    let fetch_rx = Mutex::new(fetch_rx);
    std::thread::scope(|scope| {
        for slot in 0..connections {
            let tx = fetch_tx.clone();
            let shared = &shared;
            scope.spawn(move || fetcher(shared, slot, &tx));
        }
        drop(fetch_tx);
        for _ in 0..builders {
            let tx = built_tx.clone();
            let (rx, build, record) = (&fetch_rx, &options.build, options.record.is_some());
            scope.spawn(move || builder(world, build, record, rx, &tx));
        }
        drop(built_tx);

        // The assembler: results in plan order through a reorder buffer.
        let mut pending: BTreeMap<usize, Built> = BTreeMap::new();
        let mut expected = 0usize;
        for msg in built_rx {
            pending.insert(msg.idx, msg);
            while let Some(done) = pending.remove(&expected) {
                expected += 1;
                asm.apply(done, warnings);
                asm.progress(progress);
            }
        }
    });
}

/// Generates a navmesh from `source`.
///
/// The steps are those of the spec's section 2.3: world params, the target mesh (from `base` or a
/// fresh one), the tile plan, then fetchers, builders and the assembler (see the module docs), the
/// island filter, and the report. The mesh is in [`GenerateReport::set`]; the caller saves it. A
/// failed tile does not stop the run: it is in [`GenerateReport::failed`] and raises the warning
/// `navmesh-tile-failed`. `progress` is called on the calling thread after each applied tile and once
/// at the end. The result does not depend on `threads` or `connections`.
///
/// # Errors
///
/// "--skip-existing needs --base"; "empty tile range"; an unusable base or world grid; a world
/// params request that fails; a capture file that cannot be created; and, after the run, a failing
/// checkpoint or capture write.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the source (a capture can be large) is owned by the run"
)]
pub fn generate(
    source: Source,
    options: &GenerateOptions,
    progress: &mut dyn FnMut(&Progress),
    warnings: &mut dyn WarningSink,
) -> io::Result<GenerateReport> {
    let start = Instant::now();
    if options.skip_existing && options.base.is_none() {
        return Err(error!("--skip-existing needs --base"));
    }
    let (world_params, range, from, default_connections) = match &source {
        Source::Capture(c) => {
            let range = options.range.or_else(|| capture_range(c));
            (c.params().clone(), range, FetchFrom::Capture(c), 1)
        }
        Source::Servers(hosts) => {
            let first = hosts
                .first()
                .ok_or_else(|| error!("no data server given"))?;
            (
                probe(first, &options.retry)?,
                options.range,
                FetchFrom::Servers(hosts),
                hosts.len(),
            )
        }
    };
    let world = &world_params.world;
    let (mesh, grid) = create_mesh(world, options.base.as_ref())?;
    let mut plan = match range {
        Some(r) => plan_tiles(world, Some(r))?,
        None if matches!(source, Source::Capture(_)) => return Err(error!("empty tile range")),
        None => plan_tiles(world, None)?,
    };
    if options.skip_existing {
        plan.retain(|&(x, z)| mesh.tile_at(x, z).is_none());
    }
    let recorder = match &options.record {
        Some(path) => {
            let file =
                File::create(path).map_err(|e| error!("cannot create {}: {e}", path.display()))?;
            Some(
                CaptureWriter::new(file, &world_params)
                    .prepend_error("cannot write the capture")?,
            )
        }
        None => None,
    };

    let stop = AtomicBool::new(false);
    let mut asm = Assembler {
        mesh,
        plan: &plan,
        report: GenerateReport::default(),
        recorder,
        checkpoint: options
            .checkpoint
            .as_ref()
            .map(|(n, path)| ((*n).max(1), path.as_path())),
        grid,
        stop: &stop,
        fatal: None,
        start,
    };
    run_pipeline(
        &mut asm,
        &world_params,
        from,
        default_connections,
        options,
        progress,
        warnings,
    );

    if let Some(e) = asm.fatal.take() {
        return Err(e);
    }
    let mut report = std::mem::take(&mut asm.report);
    let mut mesh = asm.mesh;
    if options.island_filter && world.filter_isolated_islands {
        let seed = options.seed.unwrap_or(world.seed_position);
        report.islands = filter_islands(&mut mesh, seed);
        if report.islands.is_none() {
            warnings.warn(Warning::new(
                Some("navmesh-seed"),
                format!(
                    "island filter skipped: no polygon near the seed ({:.1} {:.1} {:.1})",
                    seed[0], seed[1], seed[2]
                ),
            ));
        }
    }
    report.set = mesh.to_set(grid.0, grid.1);
    report.seconds = start.elapsed().as_secs_f64();
    progress(&Progress {
        done: report.built + report.empty + report.failed.len(),
        total: plan.len(),
        built: report.built,
        empty: report.empty,
        failed: report.failed.len(),
        elapsed: start.elapsed(),
    });
    Ok(report)
}
