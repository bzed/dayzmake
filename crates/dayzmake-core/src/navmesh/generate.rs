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
//! drains into its reorder buffer. So every blocked `send` is eventually released: the assembler never
//! waits for a send, the builders therefore never wait for it, and the fetchers wait only for builders.
//! Each claimed plan index produces exactly one message (a tile, a skipped tile or an error), so the
//! assembler's next-expected index is always eventually delivered. A fetcher that stops after a final
//! network error hands its claimed index on as an error first; the last fetcher to stop claims every
//! index nobody has claimed and reports it as a fetch error (small messages, so it ignores the window
//! below).
//!
//! The reorder buffer is bounded by a claim window: a fetcher that has claimed index `i` waits (on a
//! condition variable) while `i >= applied + window` before it fetches. The index the assembler waits
//! for is always below that limit, so its holder never waits and the wait is released as soon as that
//! tile is applied; a hung connection therefore stalls the run (until the receive timeout) but cannot
//! make the buffer grow past the window. A failing checkpoint or capture write, and a panic of the
//! assembler, set the stop flag and wake the waiting fetchers: they stop claiming, the channels close
//! as the stages finish and the run ends with [`GenerateReport::stopped`] set. A panic while building a
//! tile is caught and makes that tile fail; builders own the receiver of the fetch queue, so if every
//! builder is gone the fetchers' sends fail and they return.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
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
/// The largest plan: more tiles than any real map has.
const MAX_PLAN: i64 = 1 << 22;
/// The smallest claim window.
const MIN_WINDOW: usize = 256;

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
    /// Write a capture of the fetched tiles to this path. Tiles are recorded by the assembler in plan
    /// order, so the capture is deterministic, and a crash loses the tiles that were fetched but not
    /// yet applied.
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
    /// Test hook: the size of the claim window instead of the computed one.
    #[doc(hidden)]
    pub window: Option<usize>,
    /// Test hook: the build of this tile panics.
    #[doc(hidden)]
    pub panic_tile: Option<(i32, i32)>,
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
            window: None,
            panic_tile: None,
        }
    }
}

/// How far a run has come; passed to the progress callback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Progress {
    /// Tiles dealt with: built, empty, failed, skipped (not in the capture) and not fetched (fetch
    /// errors). It reaches `total` when the run is complete.
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
    /// Set when the run was stopped by a failing checkpoint or capture write: the message. The set
    /// holds what was assembled until then and the island filter did not run. Callers should save
    /// `set` and then report this as an error.
    pub stopped: Option<String>,
    /// Test hook: the largest number of results the reorder buffer held.
    #[doc(hidden)]
    pub peak_pending: usize,
    /// Test hook: the claim window of the run.
    #[doc(hidden)]
    pub window: usize,
}

/// The tiles of a run, in the order of spec 8.4.
///
/// The range is clamped to the grid: lower bounds to 0, an upper bound below 0 or beyond the grid to the
/// last tile. The range is cut into square blocks of `blockSize` tiles starting at the range's
/// own corner; blocks go row by row (z outer), and inside a block the tiles go row by row too.
///
/// # Errors
///
/// "empty tile range" when nothing is left of the range; "tile range too large (N tiles, limit
/// 4194304)" above 2^22 tiles.
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
    let count = (i64::from(x1) - i64::from(x0) + 1) * (i64::from(z1) - i64::from(z0) + 1);
    if count > MAX_PLAN {
        return Err(error!(
            "tile range too large ({count} tiles, limit {MAX_PLAN})"
        ));
    }
    let count = usize::try_from(count).map_err(|_| error!("empty tile range"))?;
    let block = world.block_size.max(1);
    let step = usize::try_from(block).unwrap_or(1);
    let mut plan = Vec::new();
    plan.try_reserve_exact(count)
        .map_err(|e| error!("cannot allocate the tile plan ({count} tiles): {e}"))?;
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
    gate: &'a Gate,
    window: usize,
    world: &'a WorldParams,
    from: FetchFrom<'a>,
    policy: &'a RetryPolicy,
}

/// `tile (x,z): message`, unless the message (a client error) already names the tile.
fn net_message(x: i32, z: i32, e: &dyn std::fmt::Display) -> String {
    let text = e.to_string();
    let prefix = format!("tile ({x},{z})");
    if text.starts_with(&prefix) || text.starts_with(&format!("tile {x} {z} ")) {
        text
    } else {
        format!("{prefix}: {text}")
    }
}

/// The claim window state: how many plan indices the assembler has applied.
struct Gate {
    applied: Mutex<usize>,
    changed: Condvar,
}

impl Gate {
    fn set_applied(&self, n: usize) {
        *self.applied.lock().unwrap_or_else(PoisonError::into_inner) = n;
        self.changed.notify_all();
    }

    /// Wakes the waiting fetchers (after `stop` was set).
    fn wake(&self) {
        // Taking the lock orders this after any fetcher's check of `stop`, so no wake-up is lost.
        drop(self.applied.lock().unwrap_or_else(PoisonError::into_inner));
        self.changed.notify_all();
    }

    /// Waits until index `idx` is inside the window. Returns false when the run was stopped.
    fn enter(&self, idx: usize, window: usize, stop: &AtomicBool) -> bool {
        let guard = self.applied.lock().unwrap_or_else(PoisonError::into_inner);
        drop(
            self.changed
                .wait_while(guard, |applied| {
                    idx >= applied.saturating_add(window) && !stop.load(Ordering::Relaxed)
                })
                .unwrap_or_else(PoisonError::into_inner),
        );
        !stop.load(Ordering::Relaxed)
    }
}

/// Sets the stop flag and wakes the fetchers when the assembler's scope ends, also by a panic.
struct StopOnDrop<'a> {
    stop: &'a AtomicBool,
    gate: &'a Gate,
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.gate.wake();
    }
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
                // Not connected yet, so the retry loop of `tile` covers the first connect too.
                *client = Some(Client::lazy(host, shared.policy));
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
        if !shared.gate.enter(idx, shared.window, shared.stop) {
            break;
        }
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

/// Builds one tile; a panic of the build (a bug on hostile geometry) fails the tile and nothing else.
fn build_guarded(world: &WorldParams, options: &GenerateOptions, input: &TileInput) -> Outcome {
    let built = catch_unwind(AssertUnwindSafe(|| {
        // Test hook (`GenerateOptions::panic_tile`).
        assert!(
            options.panic_tile != Some((input.x, input.z)),
            "build panic requested for tile ({},{})",
            input.x,
            input.z
        );
        build_tile(world, input, &options.build)
    }));
    match built {
        Ok(Ok(outcome)) => Outcome::Tile(Box::new(outcome)),
        Ok(Err(e)) => Outcome::Failed(e.to_string()),
        Err(_) => Outcome::Failed("internal error: tile build panicked".to_owned()),
    }
}

/// A builder. It owns a handle of the fetch queue's receiver: when the last builder is gone the
/// receiver is dropped and the fetchers' sends fail instead of blocking.
fn builder(
    world: &WorldParams,
    options: &GenerateOptions,
    rx: &Mutex<Receiver<Fetched>>,
    tx: &SyncSender<Built>,
) {
    let record = options.record.is_some();
    loop {
        // The lock is held only while waiting for the next tile.
        let msg = rx.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok(Fetched { idx, what }) = msg else {
            return;
        };
        let (outcome, input) = match what {
            Fetch::Tile(input) => {
                let outcome = build_guarded(world, options, &input);
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
    gate: &'a Gate,
    /// Tiles the capture lacks.
    skipped: usize,
    start: Instant,
}

impl Assembler<'_> {
    fn finished(&self) -> usize {
        self.report.built + self.report.empty + self.report.failed.len()
    }

    /// Stops the run: no further fetching, checkpoints or island filter. The first reason is kept.
    fn halt(&mut self, reason: String) {
        if self.report.stopped.is_none() {
            self.report.stopped = Some(reason);
        }
        self.stop.store(true, Ordering::Relaxed);
        self.gate.wake();
    }

    fn fail(&mut self, (x, z): (i32, i32), message: String, warnings: &mut dyn WarningSink) {
        warnings.warn(Warning::new(
            Some("navmesh-tile-failed"),
            net_message(x, z, &message),
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
        {
            self.halt(format!("cannot write the capture: {e}"));
        }
        match msg.outcome {
            Outcome::Skipped => self.skipped += 1,
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
        if self.report.stopped.is_some() || done == finished_before || !done.is_multiple_of(every) {
            return;
        }
        let set = self.mesh.to_set(self.grid.0, self.grid.1);
        if let Err(e) = set.write_atomic(path) {
            self.halt(format!("checkpoint {}: {e}", path.display()));
        }
    }

    fn progress(&self, progress: &mut dyn FnMut(&Progress)) {
        progress(&Progress {
            done: self.finished() + self.skipped + self.report.fetch_errors.len(),
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
    let window = options
        .window
        .unwrap_or_else(|| {
            MIN_WINDOW.max(
                FETCH_QUEUE
                    .saturating_add(builders.saturating_mul(5))
                    .saturating_add(connections),
            )
        })
        .max(1);
    let (plan, stop, gate) = (asm.plan, asm.stop, asm.gate);
    let next = AtomicUsize::new(0);
    let alive = AtomicUsize::new(connections);
    let shared = FetchShared {
        plan,
        next: &next,
        stop,
        alive: &alive,
        gate,
        window,
        world,
        from,
        policy: &options.retry,
    };
    let (fetch_tx, fetch_rx) = sync_channel::<Fetched>(FETCH_QUEUE);
    let (built_tx, built_rx) = sync_channel::<Built>(builders.saturating_mul(4));
    let fetch_rx = Arc::new(Mutex::new(fetch_rx));
    let mut peak = 0usize;
    std::thread::scope(|scope| {
        let _wake_on_exit = StopOnDrop { stop, gate };
        for slot in 0..connections {
            let tx = fetch_tx.clone();
            let shared = &shared;
            scope.spawn(move || fetcher(shared, slot, &tx));
        }
        drop(fetch_tx);
        for _ in 0..builders {
            let tx = built_tx.clone();
            let rx = Arc::clone(&fetch_rx);
            scope.spawn(move || builder(world, options, &rx, &tx));
        }
        drop(built_tx);
        drop(fetch_rx);

        // The assembler: results in plan order through a reorder buffer.
        let mut pending: BTreeMap<usize, Built> = BTreeMap::new();
        let mut expected = 0usize;
        for msg in built_rx {
            pending.insert(msg.idx, msg);
            peak = peak.max(pending.len());
            while let Some(done) = pending.remove(&expected) {
                expected += 1;
                asm.apply(done, warnings);
                gate.set_applied(expected);
                asm.progress(progress);
            }
        }
    });
    asm.report.peak_pending = peak;
    asm.report.window = window;
}

/// Creates the capture file of `record` and writes its header.
fn open_recorder(path: &Path, params: &WorldParams) -> io::Result<CaptureWriter<File>> {
    let file = File::create(path).map_err(|e| error!("cannot create {}: {e}", path.display()))?;
    CaptureWriter::new(file, params).prepend_error("cannot write the capture")
}

/// Generates a navmesh from `source`.
///
/// The steps are those of the spec's section 2.3: world params, the target mesh (from `base` or a
/// fresh one), the tile plan, then fetchers, builders and the assembler (see the module docs), the
/// island filter, and the report. The mesh is in [`GenerateReport::set`]; the caller saves it. A
/// failing checkpoint or capture write stops the run but is not an `Err`: the report has
/// [`GenerateReport::stopped`], and the caller should save `set` and then fail. A failed tile does not stop the run: it is in [`GenerateReport::failed`] and raises the warning
/// `navmesh-tile-failed`. `progress` is called on the calling thread after each applied tile and once
/// at the end. The result does not depend on `threads` or `connections`.
///
/// # Errors
///
/// "--skip-existing needs --base"; "empty tile range"; an unusable base or world grid; a world
/// params request that fails; "tile range too large"; a capture file that cannot be created.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the API hands the source (a capture can be large) over to the run"
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
        Some(path) => Some(open_recorder(path, &world_params)?),
        None => None,
    };

    let stop = AtomicBool::new(false);
    let gate = Gate {
        applied: Mutex::new(0),
        changed: Condvar::new(),
    };
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
        gate: &gate,
        skipped: 0,
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

    let skipped = asm.skipped;
    let mut report = std::mem::take(&mut asm.report);
    let mut mesh = asm.mesh;
    if report.stopped.is_none() && options.island_filter && world.filter_isolated_islands {
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
        done: report.built
            + report.empty
            + report.failed.len()
            + skipped
            + report.fetch_errors.len(),
        total: plan.len(),
        built: report.built,
        empty: report.empty,
        failed: report.failed.len(),
        elapsed: start.elapsed(),
    });
    Ok(report)
}
