//! The navmesh generator: plans the tiles of a run, fetches them (or reads them from a capture),
//! builds them in parallel and assembles the mesh (spec 2.3, 8.4, 8.5).
//!
//! # Threads
//!
//! Three stages run inside one [`std::thread::scope`], joined by bounded channels:
//!
//! * **fetchers** (one per connection) claim the next plan index (a hand-back queue first, then a shared
//!   counter), fetch the tile and send it to the builders (queue bound 64);
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
//! assembler's next-expected index is always eventually delivered. A fetcher whose connection died
//! (a final network error) puts its claimed index back into the hand-back queue and stops; the other
//! fetchers take it from there, and a fetcher with nothing left to claim waits while another one still has
//! an index in flight. Only the last fetcher to stop reports what is left (handed-back and unclaimed
//! indices) as fetch errors (small messages, so it ignores the window below).
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

use std::collections::{BTreeMap, VecDeque};
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
use crate::navmesh::capture::{Capture, CaptureFile, CaptureWriter};
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
    /// A capture file that is indexed and read tile by tile (see [`CaptureFile`]); tiles it lacks, or that
    /// the file was not asked to index (its range), are skipped silently. Memory is proportional to the
    /// tiles of the range, not to the file. `record` must not name the file being read.
    CaptureFile(CaptureFile),
}

/// The most builder threads or connections [`generate`] accepts; more is an error (each is an operating
/// system thread, and the channels are sized by them).
pub const MAX_WORKERS: usize = 256;

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
    /// failing checkpoint stops fetching; the run still returns `Ok` with [`GenerateReport::stopped`]
    /// set. `N` below 1 means 1.
    pub checkpoint: Option<(usize, PathBuf)>,
    /// Write a capture of the fetched tiles to this path. Tiles are recorded by the assembler in plan
    /// order, so the capture is deterministic, and a crash loses the tiles that were fetched but not
    /// yet applied. Queued tile inputs can be large (a frame may be up to 1 GiB) and there is no
    /// aggregate byte budget: the memory of a run grows with the largest tiles in flight.
    pub record: Option<PathBuf>,
    /// The number of builder threads, at most [`MAX_WORKERS`]; `None`: the cores minus one, at least 1.
    pub threads: Option<usize>,
    /// The number of connections (fetcher threads), at most [`MAX_WORKERS`]; `None`: one per host (1 for a capture).
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
    /// Tiles that could not be fetched (final network errors): `(x, z, message)`, in plan order. The
    /// message does not name the tile.
    pub fetch_errors: Vec<(i32, i32, String)>,
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
    /// The seed of the island filter: `GenerateOptions::seed`, else the world's seed position. Set also when
    /// the filter did not run (a stopped run, or `island_filter` off).
    pub seed: Option<[f32; 3]>,
    /// Diagnostic: the largest number of finished tiles the reorder buffer held while waiting for an
    /// earlier tile. It never exceeds [`GenerateReport::window`] (the final drain of unfetched tiles
    /// after every connection failed is exempt).
    pub peak_pending: usize,
    /// Diagnostic: the claim window of the run, the number of plan indices a fetcher may be ahead of
    /// the first tile not yet applied.
    pub window: usize,
}

/// The tiles of the grid that `range` selects, as inclusive bounds `[x0, z0, x1, z1]`; `None` when
/// nothing is left. Lower bounds below 0 become 0, an upper bound below 0 or beyond the grid becomes
/// the last tile, and no range is the whole grid. [`plan_tiles`] and the index of a
/// [`CaptureFile`](crate::navmesh::capture::CaptureFile) both use it, so a run and its capture cannot
/// disagree about the range.
pub(crate) fn clip_range(range: Option<[i32; 4]>, tiles_x: i32, tiles_z: i32) -> Option<[i32; 4]> {
    let [rx0, rz0, rx1, rz1] = range.unwrap_or([0, 0, -1, -1]);
    let last = |n: i32| n.saturating_sub(1);
    let x1 = if rx1 < 0 {
        last(tiles_x)
    } else {
        rx1.min(last(tiles_x))
    };
    let z1 = if rz1 < 0 {
        last(tiles_z)
    } else {
        rz1.min(last(tiles_z))
    };
    let (x0, z0) = (rx0.max(0), rz0.max(0));
    (x0 <= x1 && z0 <= z1).then_some([x0, z0, x1, z1])
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
    let Some([x0, z0, x1, z1]) = clip_range(range, world.tiles_x, world.tiles_z) else {
        return Err(error!("empty tile range"));
    };
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
    CaptureFile(&'a CaptureFile),
}

/// What the fetchers share.
struct FetchShared<'a> {
    plan: &'a [(i32, i32)],
    next: &'a AtomicUsize,
    stop: &'a AtomicBool,
    alive: &'a AtomicUsize,
    /// The number of fetchers.
    connections: usize,
    gate: &'a Gate,
    window: usize,
    world: &'a WorldParams,
    from: FetchFrom<'a>,
    policy: &'a RetryPolicy,
}

/// A tile handed back after its connection died.
struct Handback {
    /// The error of the first failed try (reported when the tile ends up failed).
    why: String,
    /// On how many connections that had been served before (so they were working) the tile has failed.
    proven: u32,
    /// On how many connections that had not been served yet (a refused or reset start looks the same as a
    /// tile the server cannot answer) the tile has failed.
    unproven: u32,
}

impl Handback {
    /// The tile is reported as failed when the tile is the likely cause, not the connections: it failed
    /// on two working connections, or on a working one and another one. Failures on connections that
    /// were never served count only together with a working one, so a refused start loses no tile.
    fn is_poison(&self) -> bool {
        self.proven >= 2 || (self.proven >= 1 && self.unproven >= 1)
    }
}

/// What the fetchers coordinate on, under one lock: how many plan indices the assembler has applied (the
/// claim window), the hand-back queue and the number of indices in flight.
#[derive(Default)]
struct Work {
    applied: usize,
    /// Indices to fetch again on another connection.
    retry: VecDeque<(usize, Handback)>,
    /// Indices claimed and not yet delivered or handed back.
    inflight: usize,
}

/// The claim window and the hand-back queue, with one condition variable for both: a waiting fetcher
/// wakes when the assembler advances, when a tile is handed back, when an index is delivered and on stop.
struct Gate {
    work: Mutex<Work>,
    changed: Condvar,
}

impl Gate {
    fn new() -> Self {
        Self {
            work: Mutex::new(Work::default()),
            changed: Condvar::new(),
        }
    }

    fn set_applied(&self, n: usize) {
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .applied = n;
        self.changed.notify_all();
    }

    /// Wakes the waiting fetchers (after `stop` was set).
    fn wake(&self) {
        // Taking the lock orders this after any fetcher's check of `stop`, so no wake-up is lost.
        drop(self.work.lock().unwrap_or_else(PoisonError::into_inner));
        self.changed.notify_all();
    }

    /// Claims the next index: a handed-back one first, then the next unclaimed one, which has to be inside
    /// the claim window unless `windowed` is false (the last fetcher reporting what is left). Waits while
    /// there is none: the next index is outside the window, or another fetcher still has one in flight
    /// (its connection may die and hand it back). `None` when everything is delivered or the run was
    /// stopped. The second value is set for a handed-back index.
    ///
    /// `prefer_fresh` takes a fresh index before a handed-back one when both are available: a connection
    /// that has not delivered a tile yet should prove itself on a tile nobody failed before, or a late
    /// starter would fail the tile of a connection that had worked.
    ///
    /// `before_wait` runs once, before the first wait: the caller closes its connection there, because
    /// a server that serves one client at a time would otherwise keep serving the idle connection while
    /// the connections that hold the tiles in flight wait for it.
    fn claim(
        &self,
        shared: &FetchShared<'_>,
        windowed: bool,
        prefer_fresh: bool,
        before_wait: &mut dyn FnMut(),
    ) -> Option<(usize, Option<Handback>)> {
        let mut work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        let mut waited = Some(());
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return None;
            }
            let next = shared.next.load(Ordering::Relaxed);
            let fresh = next < shared.plan.len()
                && (!windowed || next < work.applied.saturating_add(shared.window));
            if !(prefer_fresh && fresh)
                && let Some((idx, back)) = work.retry.pop_front()
            {
                work.inflight += 1;
                return Some((idx, Some(back)));
            }
            if fresh {
                shared.next.store(next + 1, Ordering::Relaxed);
                work.inflight += 1;
                return Some((next, None));
            }
            if next >= shared.plan.len() && work.inflight == 0 {
                return None;
            }
            if waited.take().is_some() {
                before_wait();
            }
            work = self
                .changed
                .wait(work)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// An index was delivered (`back` is `None`) or its connection died (`Some`: it goes back to the
    /// queue).
    fn done(&self, idx: usize, back: Option<Handback>) {
        let mut work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        work.inflight = work.inflight.saturating_sub(1);
        if let Some(back) = back {
            work.retry.push_back((idx, back));
        }
        drop(work);
        self.changed.notify_all();
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

/// What `fetch_one` found.
enum Got {
    /// A result for the plan index (a tile, a skipped tile, or an error of the tile or the capture that
    /// leaves the connection usable).
    Done(Fetch),
    /// The connection is dead (its retries are used up); the message is the last error.
    Dead(String),
}

fn fetch_one(
    shared: &FetchShared<'_>,
    slot: usize,
    client: &mut Option<Client>,
    at: (i32, i32),
) -> Got {
    let (col, row) = at;
    match &shared.from {
        FetchFrom::Capture(capture) => Got::Done(
            capture
                .tile(col, row)
                .map_or(Fetch::Skipped, |tile| Fetch::Tile(tile.clone())),
        ),
        FetchFrom::CaptureFile(file) => Got::Done(match file.tile(col, row) {
            Ok(Some(tile)) => Fetch::Tile(tile),
            Ok(None) => Fetch::Skipped,
            // A malformed record fails the tile (it would fail again); anything else is a read error
            // of the moment, so the tile is reported as not fetched and offered for a resume.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Fetch::Wire(e.to_string()),
            Err(e) => Fetch::Net(e.to_string()),
        }),
        FetchFrom::Servers(hosts) => {
            if client.is_none() {
                let Some(host) = hosts.get(slot % hosts.len().max(1)) else {
                    return Got::Dead("no data server".to_owned());
                };
                // Not connected yet, so the retry loop of `tile` covers the first connect too.
                *client = Some(Client::lazy(host, shared.policy));
            }
            let Some(conn) = client.as_mut() else {
                return Got::Dead("not connected".to_owned());
            };
            match conn.tile(col, row, shared.world) {
                Ok(tile) => Got::Done(Fetch::Tile(tile)),
                Err(FetchError::Wire(e)) => Got::Done(Fetch::Wire(e.to_string())),
                // A complete but wrong answer: the tile is at fault. The failed try closed the
                // connection, the next request opens a fresh one, so nothing is out of sync.
                Err(FetchError::Network(e)) if conn.last_failure_was_answer() => {
                    Got::Done(Fetch::Net(e.to_string()))
                }
                Err(FetchError::Network(e)) => Got::Dead(e.to_string()),
            }
        }
    }
}

/// Armed for the life of a fetcher. When the fetcher leaves any way but its normal end (a panic, or the
/// assembler being gone), the run is stopped, the waiting fetchers are woken and the fetcher is no longer
/// counted as alive, so nothing waits for a tile that will never come.
struct FetcherExit<'a> {
    shared: &'a FetchShared<'a>,
    armed: bool,
}

impl Drop for FetcherExit<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.shared.stop.store(true, Ordering::Relaxed);
            self.shared.gate.wake();
            self.shared.alive.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// Whether another fetcher uses the server of fetcher `slot` (fetcher `k` uses host `k mod hosts`).
fn shares_host(shared: &FetchShared<'_>, slot: usize) -> bool {
    match &shared.from {
        FetchFrom::Servers(hosts) if !hosts.is_empty() => {
            (0..shared.connections)
                .filter(|k| k % hosts.len() == slot % hosts.len())
                .count()
                > 1
        }
        _ => false,
    }
}

fn fetcher(shared: &FetchShared<'_>, slot: usize, tx: &SyncSender<Fetched>) {
    let mut exit = FetcherExit {
        shared,
        armed: true,
    };
    let mut client: Option<Client> = None;
    // This fetcher has been served by its server at least once: a failure of a connection of its then says
    // something about the tile. It is kept when the connection is closed to wait or replaced, so a poison
    // tile cannot take every connection down uncounted. Known limit: with three or more connections to a
    // server that serves one client at a time, a reconnect that is reset can still count against an innocent
    // tile; the result is a spurious fetch error for that tile (resumable), never a lost run.
    let mut delivered = false;
    let others_on_host = shares_host(shared, slot);
    while let Some((idx, back)) = shared.gate.claim(shared, true, !delivered, &mut || {
        // Only a fetcher that shares its server with others closes the connection before it waits:
        // nobody else can be waiting for a server that only this fetcher uses.
        if others_on_host {
            client = None;
        }
    }) {
        let Some(&at) = shared.plan.get(idx) else {
            shared.gate.done(idx, None);
            break;
        };
        let what = match fetch_one(shared, slot, &mut client, at) {
            Got::Done(what) => what,
            Got::Dead(message) => {
                let (mut proven, mut unproven) =
                    back.as_ref().map_or((0, 0), |b| (b.proven, b.unproven));
                if delivered {
                    proven += 1;
                } else {
                    unproven += 1;
                }
                let why = back.map_or(message, |b| b.why);
                let hand = Handback {
                    why,
                    proven,
                    unproven,
                };
                if !hand.is_poison() {
                    // This connection is dead. The tile goes back for the live connections and
                    // fails only when none is left (the last fetcher reports it below).
                    shared.gate.done(idx, Some(hand));
                    break;
                }
                let why = hand.why;
                // The tile failed on another connection too: report it and carry on with a fresh
                // connection of our own.
                client = None;
                Fetch::Net(why)
            }
        };
        delivered |= matches!(what, Fetch::Tile(_) | Fetch::Skipped);
        if tx.send(Fetched { idx, what }).is_err() {
            return;
        }
        shared.gate.done(idx, None);
    }
    exit.armed = false;
    // The last fetcher to stop reports every index that is left (handed back, or nobody claimed), so the
    // run still ends. No other fetcher is alive, so nothing is in flight and `claim` does not wait.
    if shared.alive.fetch_sub(1, Ordering::AcqRel) == 1 && !shared.stop.load(Ordering::Relaxed) {
        while let Some((idx, back)) = shared.gate.claim(shared, false, false, &mut || {}) {
            let message = back.map_or_else(
                || "not fetched: every connection failed".to_owned(),
                |b| b.why,
            );
            let sent = tx.send(Fetched {
                idx,
                what: Fetch::Net(message),
            });
            shared.gate.done(idx, None);
            if sent.is_err() {
                return;
            }
        }
    }
}

/// Runs a tile build; a panic of the build (a bug on hostile geometry) fails the tile and nothing else.
fn build_guarded(build: impl FnOnce() -> io::Result<TileOutcome>) -> Outcome {
    match catch_unwind(AssertUnwindSafe(build)) {
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
                let outcome = build_guarded(|| build_tile(world, &input, &options.build));
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
        {
            self.halt(format!("cannot write the capture: {e}"));
        }
        match msg.outcome {
            Outcome::Skipped => self.skipped += 1,
            Outcome::Net(m) => self.report.fetch_errors.push((at.0, at.1, m)),
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
        let p = &set.params;
        let mismatch = p.tile_width.to_bits() != world.tile_width.to_bits()
            || p.tile_height.to_bits() != world.tile_height.to_bits()
            || p.orig.iter().any(|c| c.to_bits() & 0x7fff_ffff != 0)
            || set.tiles_x != world.tiles_x
            || set.tiles_z != world.tiles_z;
        if mismatch {
            return Err(error!(
                "base navmesh does not match the world (base: {} x {} tiles of {} x {} m at {:?}; world: {} x {} tiles of {} x {} m at [0.0, 0.0, 0.0])",
                set.tiles_x,
                set.tiles_z,
                p.tile_width,
                p.tile_height,
                p.orig,
                world.tiles_x,
                world.tiles_z,
                world.tile_width,
                world.tile_height
            ));
        }
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
        connections,
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

/// Refuses more than [`MAX_WORKERS`] builder threads or connections, before anything is allocated or spawned.
fn check_worker_counts(options: &GenerateOptions, source: &Source) -> io::Result<()> {
    if let Source::Servers(hosts) = source
        && options.connections.is_none()
        && hosts.len() > MAX_WORKERS
    {
        return Err(error!(
            "hosts: at most {MAX_WORKERS} data servers (one connection each)"
        ));
    }
    if options.threads.is_some_and(|n| n > MAX_WORKERS) {
        return Err(error!("threads: at most {MAX_WORKERS} builder threads"));
    }
    if options.connections.is_some_and(|n| n > MAX_WORKERS) {
        return Err(error!("connections: at most {MAX_WORKERS} connections"));
    }
    Ok(())
}

/// Whether two paths name the same existing file (by device and inode on Unix, by canonical path
/// everywhere).
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b))
            && x.dev() == y.dev()
            && x.ino() == y.ino()
        {
            return true;
        }
    }
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// The world params, the range, the fetch source and the default connection count of a source.
fn open_source<'a>(
    source: &'a Source,
    options: &GenerateOptions,
) -> io::Result<(WorldParams, Option<[i32; 4]>, FetchFrom<'a>, usize)> {
    Ok(match source {
        Source::Capture(c) => {
            let range = options.range.or_else(|| capture_range(c));
            (c.params().clone(), range, FetchFrom::Capture(c), 1)
        }
        Source::CaptureFile(c) => {
            if let (Some(record), Some(read)) = (&options.record, c.path())
                && same_file(record, read)
            {
                // Creating the recording would truncate the capture that is being read.
                return Err(error!(
                    "the recording {} is the capture being read; give another file",
                    record.display()
                ));
            }
            let range = options.range.or_else(|| c.bounds());
            if !c.covers(range) {
                // The tiles outside the indexed range would read as absent: a mesh without them and
                // no error. The caller opened the capture for another range.
                let fmt = |r: Option<[i32; 4]>| {
                    r.map_or_else(
                        || "no tiles".to_owned(),
                        |[x0, z0, x1, z1]| format!("{x0},{z0},{x1},{z1}"),
                    )
                };
                return Err(error!(
                    "the capture was indexed for the range {} but the run asks for {}; open the capture with the range of the run",
                    fmt(c.indexed_range()),
                    fmt(range)
                ));
            }
            (c.params().clone(), range, FetchFrom::CaptureFile(c), 1)
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
    })
}

/// Generates a navmesh from `source`.
///
/// The steps are those of the spec's section 2.3: world params, the target mesh (from `base` or a
/// fresh one), the tile plan, then fetchers, builders and the assembler (see the module docs), the
/// island filter, and the report. The mesh is in [`GenerateReport::set`]; the caller saves it. A
/// failing checkpoint or capture write stops the run but is not an `Err`: the report has
/// [`GenerateReport::stopped`], and the caller should save `set` and then fail. A failed tile does not stop the run: it is in [`GenerateReport::failed`] and raises the warning
/// `navmesh-tile-failed`. `progress` is called on the calling thread after each applied tile and once
/// at the end. The result does not depend on `threads` or `connections`. Memory: the queues between
/// fetchers, builders and the assembler are bounded in tiles, not in bytes; a tile input can be large (frames up
/// to 1 GiB) and there is no aggregate byte budget.
///
/// # Errors
///
/// "--skip-existing needs --base"; `threads` or `connections` above [`MAX_WORKERS`], or more than
/// [`MAX_WORKERS`] hosts without an explicit `connections` (the default is one connection per host); both
/// are checked before anything is allocated, spawned or connected; "empty tile range"; an unusable base or world grid; a world
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
    check_worker_counts(options, &source)?;
    if options.skip_existing && options.base.is_none() {
        return Err(error!("--skip-existing needs --base"));
    }
    let (world_params, range, from, default_connections) = open_source(&source, options)?;
    let world = &world_params.world;
    let (mesh, grid) = create_mesh(world, options.base.as_ref())?;
    let mut plan = match range {
        Some(r) => plan_tiles(world, Some(r))?,
        None if matches!(source, Source::Capture(_) | Source::CaptureFile(_)) => {
            return Err(error!("empty tile range"));
        }
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
    let gate = Gate::new();
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
    report.seed = Some(options.seed.unwrap_or(world.seed_position));
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
    report.set = mesh.into_set(grid.0, grid.1);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A fetcher that panics must stop the run, wake the fetchers waiting for the window and stop
    /// counting as alive, or the run would hang (the assembler waits for the tile it claimed).
    #[test]
    fn a_panicking_fetcher_stops_the_run_and_wakes_the_waiters() {
        let world = WorldParams::default();
        let policy = RetryPolicy::default();
        let plan = [(0, 0), (1, 0)];
        let (next, stop, alive) = (
            AtomicUsize::new(0),
            AtomicBool::new(false),
            AtomicUsize::new(2),
        );
        let gate = Gate::new();
        let shared = FetchShared {
            plan: &plan,
            next: &next,
            stop: &stop,
            alive: &alive,
            connections: 1,
            gate: &gate,
            window: 1,
            world: &world,
            from: FetchFrom::Servers(&[]),
            policy: &policy,
        };
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                // Index 0 is inside the window of 1; index 1 is outside it, so the second claim waits
                // until the run is stopped.
                assert_eq!(
                    gate.claim(&shared, true, false, &mut || {}).map(|c| c.0),
                    Some(0)
                );
                ready_tx.send(()).ok();
                gate.claim(&shared, true, false, &mut || {}).is_none()
            });
            ready_rx.recv().expect("waiter started");
            let panicked = catch_unwind(AssertUnwindSafe(|| {
                let _exit = FetcherExit {
                    shared: &shared,
                    armed: true,
                };
                std::panic::resume_unwind(Box::new("boom"))
            }));
            assert!(panicked.is_err());
            assert!(waiter.join().expect("waiter"), "the waiter was released");
        });
        assert!(stop.load(Ordering::Relaxed));
        assert_eq!(alive.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_fetcher_that_ends_normally_does_not_stop_the_run() {
        let world = WorldParams::default();
        let policy = RetryPolicy::default();
        let (next, stop, alive) = (
            AtomicUsize::new(0),
            AtomicBool::new(false),
            AtomicUsize::new(1),
        );
        let gate = Gate::new();
        let shared = FetchShared {
            plan: &[],
            next: &next,
            stop: &stop,
            alive: &alive,
            connections: 1,
            gate: &gate,
            window: 1,
            world: &world,
            from: FetchFrom::Servers(&[]),
            policy: &policy,
        };
        drop(FetcherExit {
            shared: &shared,
            armed: false,
        });
        assert!(!stop.load(Ordering::Relaxed));
        assert_eq!(alive.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_panicking_build_becomes_a_failed_tile() {
        // `resume_unwind` raises a panic without calling the panic hook: no noise, no global hook change.
        let outcome = build_guarded(|| std::panic::resume_unwind(Box::new("boom")));
        assert!(
            matches!(&outcome, Outcome::Failed(m) if m == "internal error: tile build panicked")
        );
        let outcome = build_guarded(|| Err(error!("bad tile")));
        assert!(matches!(&outcome, Outcome::Failed(m) if m == "bad tile"));
        let outcome = build_guarded(|| Ok(TileOutcome::Empty));
        assert!(matches!(&outcome, Outcome::Tile(t) if **t == TileOutcome::Empty));
    }
}
