//! `dayzmake navmesh probe`, `generate` and `info`.

use std::cell::Cell;
use std::fmt::Write as _;
use std::io::{self, BufReader, Error, IsTerminal, Write};
use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::cli_io::open_input;
use dayzmake_core::error;
use dayzmake_core::error::ErrorExt;
use dayzmake_core::navmesh::build::DoorPolicy;
use dayzmake_core::navmesh::capture::CaptureFile;
use dayzmake_core::navmesh::client::{Client, Host, RetryPolicy};
use dayzmake_core::navmesh::detour::NavMeshSet;
use dayzmake_core::navmesh::generate::{
    GenerateOptions, GenerateReport, Progress, Source, generate,
};
use dayzmake_core::navmesh::wire::{TileInput, WorldParams};
use dayzmake_core::warning::{Warning, WarningSink};

/// The host used when no `--server` is given.
const DEFAULT_HOST: &str = "127.0.0.1";
/// How many failed tiles `generate` lists before it summarises the rest.
const LISTED_ERRORS: usize = 20;

fn parse_ints(text: &str, what: &str) -> Result<Vec<i32>, Error> {
    text.split(',')
        .map(|part| {
            part.trim()
                .parse::<i32>()
                .map_err(|_| error!("{what}: \"{}\" is not a whole number", part.trim()))
        })
        .collect()
}

/// Parses `--range x0,z0,x1,z1` (tile coordinates, inclusive).
///
/// # Errors
///
/// "expected x0,z0,x1,z1" unless the text is four whole numbers separated by commas.
pub fn parse_range(text: &str) -> Result<[i32; 4], Error> {
    let numbers = parse_ints(text, "--range")?;
    <[i32; 4]>::try_from(numbers)
        .map_err(|_| error!("--range: expected x0,z0,x1,z1, got \"{text}\""))
}

/// Parses `--tile x,z`.
///
/// # Errors
///
/// "expected x,z" unless the text is two whole numbers separated by a comma.
pub fn parse_tile(text: &str) -> Result<(i32, i32), Error> {
    let numbers = parse_ints(text, "--tile")?;
    match numbers.as_slice() {
        &[x, z] => Ok((x, z)),
        _ => Err(error!("--tile: expected x,z, got \"{text}\"")),
    }
}

/// Parses `--seed x,y,z` (world coordinates in metres).
///
/// # Errors
///
/// "expected x,y,z" unless the text is three finite numbers separated by commas.
pub fn parse_seed(text: &str) -> Result<[f32; 3], Error> {
    let numbers = text
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<f32>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| error!("--seed: \"{}\" is not a finite number", part.trim()))
        })
        .collect::<Result<Vec<f32>, Error>>()?;
    <[f32; 3]>::try_from(numbers).map_err(|_| error!("--seed: expected x,y,z, got \"{text}\""))
}

/// Parses `--doors closed|open|enabled`.
///
/// # Errors
///
/// Anything but the three names.
pub fn parse_doors(text: &str) -> Result<DoorPolicy, Error> {
    match text {
        "closed" => Ok(DoorPolicy::Closed),
        "open" => Ok(DoorPolicy::Open),
        "enabled" => Ok(DoorPolicy::Enabled),
        _ => Err(error!(
            "--doors: expected closed, open or enabled, got \"{text}\""
        )),
    }
}

/// A finite `f32` as the JSON number with its shortest decimal form (`0.3`, not `0.30000001192092896`).
fn num(x: f32) -> Value {
    x.to_string()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn nums<const N: usize>(a: [f32; N]) -> Value {
    Value::Array(a.into_iter().map(num).collect())
}

fn params_json(p: &WorldParams) -> Value {
    let w = &p.world;
    json!({
        "worldSizeX": num(w.world_size_x),
        "worldSizeZ": num(w.world_size_z),
        "tilesX": w.tiles_x,
        "tilesZ": w.tiles_z,
        "tileWidth": num(w.tile_width),
        "tileHeight": num(w.tile_height),
        "seaLevel": num(w.deep_sea_height),
        "deepWaterDepth": num(w.deep_water_depth),
        "contourMaxError": num(w.contour_max_error),
        "obstacles": w.obstacles,
        "blockSize": w.block_size,
        "filterIsolatedIslands": w.filter_isolated_islands,
        "seed": nums(w.seed_position),
        "agent": {
            "standHeight": num(w.stand_height),
            "crouchHeight": num(w.crouch_height),
            "proneHeight": num(w.prone_height),
            "diameter": num(w.diameter),
            "maxStepHeight": num(w.max_step_height),
            "maxSlope": num(w.max_slope),
        },
        "cellClasses": p.cell_classes.iter().map(|c| json!({
            "cells": c.cells,
            "width": num(c.width),
            "cellSize": num(c.cell_size()),
        })).collect::<Vec<_>>(),
        "linkClasses": p.link_classes.iter().map(|l| json!({
            "index": l.index,
            "typeId": l.type_id,
            "jumpLength": num(l.jump_length),
            "jumpHeight": num(l.jump_height),
            "minCenterHeight": num(l.min_center_height),
            "dropdownMin": num(l.jump_dropdown_min),
            "dropdownMax": num(l.jump_dropdown_max),
            "agentHeight": num(l.agent_height),
            "maxStepHeight": num(l.max_step_height),
            "agentRadius": num(l.agent_radius),
            "flags": l.flags,
            "area": l.area,
        })).collect::<Vec<_>>(),
    })
}

fn tile_json(t: &TileInput) -> Value {
    json!({
        "x": t.x,
        "z": t.z,
        "cellClass": t.cell_class,
        "bmin": nums(t.bmin),
        "bmax": nums(t.bmax),
        "groups": t.groups.iter().map(|g| json!({
            "area": g.area,
            "parts": g.parts.len(),
            "verts": g.verts.len(),
        })).collect::<Vec<_>>(),
    })
}

fn write_params(p: &WorldParams, out: &mut dyn Write) -> Result<(), Error> {
    let w = &p.world;
    writeln!(
        out,
        "world {:.1} x {:.1} m, tiles {} x {} of {:.1} x {:.1} m",
        w.world_size_x, w.world_size_z, w.tiles_x, w.tiles_z, w.tile_width, w.tile_height
    )?;
    writeln!(
        out,
        "deep sea height {:.2}, deep water depth {:.2}, contour error {:.3}, obstacles {}, block {}",
        w.deep_sea_height,
        w.deep_water_depth,
        w.contour_max_error,
        u8::from(w.obstacles),
        w.block_size
    )?;
    writeln!(
        out,
        "filter islands {}, seed ({:.1}, {:.1}, {:.1})",
        u8::from(w.filter_isolated_islands),
        w.seed_position[0],
        w.seed_position[1],
        w.seed_position[2]
    )?;
    writeln!(
        out,
        "agent: stand {:.2} crouch {:.2} prone {:.2} diameter {:.2} step {:.2} slope {:.1}",
        w.stand_height, w.crouch_height, w.prone_height, w.diameter, w.max_step_height, w.max_slope
    )?;
    for (i, c) in p.cell_classes.iter().enumerate() {
        writeln!(
            out,
            "cell class {i}: {} cells over {:.2} m (cs {:.3}) [unk {} {}]",
            c.cells,
            c.width,
            c.cell_size(),
            c.cells2,
            c.unused
        )?;
    }
    for l in &p.link_classes {
        writeln!(
            out,
            "link {}: typeId {} len {:.2} height {:.2} centre {:.2} dropdown {:.2}..{:.2} agent h {:.2} step {:.2} r {:.2} flags {:#x} area {}",
            l.index,
            l.type_id,
            l.jump_length,
            l.jump_height,
            l.min_center_height,
            l.jump_dropdown_min,
            l.jump_dropdown_max,
            l.agent_height,
            l.max_step_height,
            l.agent_radius,
            l.flags,
            l.area
        )?;
    }
    Ok(())
}

fn write_tile(t: &TileInput, out: &mut dyn Write) -> Result<(), Error> {
    writeln!(
        out,
        "tile ({},{}) cell class {} bounds ({:.2} {:.2} {:.2})-({:.2} {:.2} {:.2}), {} groups",
        t.x,
        t.z,
        t.cell_class,
        t.bmin[0],
        t.bmin[1],
        t.bmin[2],
        t.bmax[0],
        t.bmax[1],
        t.bmax[2],
        t.groups.len()
    )?;
    for g in &t.groups {
        writeln!(
            out,
            "  area {:2}: {} parts, {} verts",
            g.area,
            g.parts.len(),
            g.verts.len()
        )?;
    }
    Ok(())
}

/// `dayzmake navmesh probe`: asks the data server for the world parameters (and one tile) and prints them.
///
/// # Errors
///
/// Returns an error if the server cannot be reached (the message names `host:port` and how to start the
/// data server), answers badly, or an argument is invalid.
pub fn cmd_navmesh_probe(
    server: Option<&str>,
    tile: Option<&str>,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let host = Host::parse(server.unwrap_or(DEFAULT_HOST))?;
    let tile = tile.map(parse_tile).transpose()?;
    let policy = RetryPolicy::default();
    let mut client = Client::connect(&host, &policy)?;
    let params = client.world_params()?;
    let tile = match tile {
        Some((x, z)) => Some(
            client
                .tile(x, z, &params)
                .map_err(|e| error!("tile ({x},{z}): {e}"))?,
        ),
        None => None,
    };
    if json_output {
        let mut object = Map::new();
        object.insert("params".into(), params_json(&params));
        if let Some(t) = &tile {
            object.insert("tile".into(), tile_json(t));
        }
        writeln!(out, "{}", Value::Object(object))?;
    } else {
        write_params(&params, out)?;
        if let Some(t) = &tile {
            write_tile(t, out)?;
        }
    }
    Ok(())
}

/// The options of `dayzmake navmesh generate`, as the command line gives them (not yet parsed).
#[derive(Debug, Default, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one bool per command-line flag, as in the docopt Args"
)]
pub struct GenerateArgs {
    /// `--force`.
    pub force: bool,
    /// `--server`, repeatable; `127.0.0.1` when there is neither a server nor a capture.
    pub servers: Vec<String>,
    /// `--capture`.
    pub capture: Option<String>,
    /// `--record`.
    pub record: Option<String>,
    /// `--range`.
    pub range: Option<String>,
    /// `--base`.
    pub base: Option<String>,
    /// `--skip-existing`.
    pub skip_existing: bool,
    /// `--checkpoint`.
    pub checkpoint: Option<String>,
    /// `--threads`.
    pub threads: Option<String>,
    /// `--connections`.
    pub connections: Option<String>,
    /// `--sea-level`.
    pub sea_level: Option<String>,
    /// `--doors`.
    pub doors: Option<String>,
    /// `--no-links`.
    pub no_links: bool,
    /// `--no-island-filter`.
    pub no_island_filter: bool,
    /// `--seed`.
    pub seed: Option<String>,
}

/// A count option: a whole number that is not negative; 0 is `None` (the default, or off for `--checkpoint`).
fn parse_count(text: &str, what: &str) -> Result<Option<usize>, Error> {
    let n: i64 = text
        .trim()
        .parse()
        .map_err(|_| error!("{what}: \"{text}\" is not a whole number"))?;
    let n = usize::try_from(n).map_err(|_| error!("{what}: \"{text}\" must not be negative"))?;
    Ok(Some(n).filter(|n| *n > 0))
}

fn load_base(path: &str) -> Result<NavMeshSet, Error> {
    let file = open_input(Path::new(path), "base navmesh")?;
    NavMeshSet::read(&mut BufReader::new(file)).prepend_error(format!("Failed to read {path}:"))
}

/// Indexes the capture; the tiles are read from the file when they are built, so a range of a large
/// capture needs memory in proportion to the range.
fn load_capture(path: &str) -> Result<CaptureFile, Error> {
    // Opened once to name a missing or unreadable file the way every other input is named.
    open_input(Path::new(path), "capture")?;
    CaptureFile::open(Path::new(path)).prepend_error(format!("Failed to read {path}:"))
}

/// Seconds as `h:mm:ss`.
fn clock(seconds: u128) -> String {
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The progress line of the spec: `done/total tiles (failed failed), rate tiles/s, h:mm:ss left`.
fn progress_line(p: &Progress) -> String {
    let secs = p.elapsed.as_secs_f64().max(0.001);
    let rate = count_f64(p.done) / secs;
    let done = u128::try_from(p.done).unwrap_or(u128::MAX).max(1);
    let remaining = u128::try_from(p.total.saturating_sub(p.done)).unwrap_or(0);
    let left = remaining.saturating_mul(p.elapsed.as_millis()) / done / 1000;
    format!(
        "{}/{} tiles ({} failed), {rate:.1} tiles/s, {} left",
        p.done,
        p.total,
        p.failed,
        clock(left)
    )
}

/// Receives the progress of a run.
pub trait ProgressSink {
    /// A tile finished (or the run ended).
    fn update(&mut self, progress: &Progress);
    /// The run is over; ends a progress line that was left open.
    fn finish(&mut self) {}
}

/// Ignores the progress; for callers whose stdout and stderr are not for humans (the MCP server).
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn update(&mut self, _progress: &Progress) {}
}

/// Prints progress to stderr: rewritten with `\r` on a terminal, otherwise one line per 5 % of the run.
struct ProgressPrinter<'a> {
    terminal: bool,
    /// Something was printed without a final newline; the warning sink ends the line before it prints.
    pending_line: &'a Cell<bool>,
    last_percent: usize,
    last_print: Duration,
    last_done: Option<usize>,
}

impl<'a> ProgressPrinter<'a> {
    fn new(pending_line: &'a Cell<bool>) -> Self {
        Self {
            terminal: io::stderr().is_terminal(),
            pending_line,
            last_percent: 0,
            last_print: Duration::ZERO,
            last_done: None,
        }
    }

    fn update_line(&mut self, p: &Progress) {
        let finished = p.done >= p.total;
        let percent = p.done.saturating_mul(100) / p.total.max(1);
        let due = if self.terminal {
            finished || p.elapsed.saturating_sub(self.last_print) >= Duration::from_millis(250)
        } else {
            finished || percent >= self.last_percent.saturating_add(5)
        };
        // the generator reports the end of the run once more after the last tile
        if !due || self.last_done == Some(p.done) {
            return;
        }
        self.last_done = Some(p.done);
        self.last_percent = percent;
        self.last_print = p.elapsed;
        let line = progress_line(p);
        // a failed write to stderr cannot be reported anywhere
        let _ = if self.terminal {
            self.pending_line.set(!finished);
            write!(
                io::stderr(),
                "\r{line}   {}",
                if finished { "\n" } else { "" }
            )
        } else {
            writeln!(io::stderr(), "{line}")
        };
    }
}

impl ProgressSink for ProgressPrinter<'_> {
    fn update(&mut self, p: &Progress) {
        self.update_line(p);
    }

    /// Ends a rewritten line that was left open (a stopped run).
    fn finish(&mut self) {
        if self.pending_line.replace(false) {
            let _ = writeln!(io::stderr());
        }
    }
}

/// Ends the open progress line before a warning is printed, so the warning does not start in the middle of it.
struct LineEnding<'a> {
    inner: &'a mut dyn WarningSink,
    pending_line: &'a Cell<bool>,
}

impl WarningSink for LineEnding<'_> {
    fn warn(&mut self, warning: Warning) {
        if !self.inner.is_suppressed(warning.name) && self.pending_line.replace(false) {
            // a failed write to stderr cannot be reported anywhere
            let _ = writeln!(io::stderr());
        }
        self.inner.warn(warning);
    }

    fn is_suppressed(&self, name: Option<&str>) -> bool {
        self.inner.is_suppressed(name)
    }
}

fn existing_target_error(path: &Path) -> Error {
    error!(
        "Target file \"{}\" already exists. Use --force to overwrite.",
        path.display()
    )
}

/// `text` as one shell word: unchanged when it only has safe characters, else in single quotes.
fn shell_word(text: &str) -> String {
    let safe = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// The command that continues a partial run: every option of the run except `--record` (with `-f` it would
/// overwrite the first recording) and `--base`, which becomes the target.
fn resume_hint(args: &GenerateArgs, target: &Path) -> String {
    let t = shell_word(&target.display().to_string());
    let mut hint = format!("dayzmake navmesh generate --base {t} --skip-existing -f");
    let mut option = |name: &str, value: Option<&String>| {
        if let Some(value) = value {
            let _ = write!(hint, " {name} {}", shell_word(value));
        }
    };
    option("--capture", args.capture.as_ref());
    for server in &args.servers {
        option("--server", Some(server));
    }
    option("--range", args.range.as_ref());
    option("--checkpoint", args.checkpoint.as_ref());
    option("--threads", args.threads.as_ref());
    option("--connections", args.connections.as_ref());
    option("--sea-level", args.sea_level.as_ref());
    option("--doors", args.doors.as_ref());
    option("--seed", args.seed.as_ref());
    for (given, name) in [
        (args.no_links, "--no-links"),
        (args.no_island_filter, "--no-island-filter"),
    ] {
        if given {
            let _ = write!(hint, " {name}");
        }
    }
    let _ = write!(hint, " {t}");
    hint
}

/// The MCP form of `resume_hint`: the arguments of the `navmesh_generate` call that continues a partial run.
#[must_use]
pub fn resume_args(args: &GenerateArgs, target: &Path) -> String {
    let t = target.display();
    let mut parts = vec![
        format!("target={t}"),
        format!("base={t}"),
        "skip_existing=true".to_owned(),
        "force=true".to_owned(),
    ];
    let texts = [
        ("capture", &args.capture),
        ("range", &args.range),
        ("checkpoint", &args.checkpoint),
        ("threads", &args.threads),
        ("connections", &args.connections),
        ("sea_level", &args.sea_level),
        ("doors", &args.doors),
        ("seed", &args.seed),
    ];
    for (name, value) in texts {
        if name == "range" && !args.servers.is_empty() {
            parts.push(format!("server=[{}]", args.servers.join(", ")));
        }
        if let Some(value) = value {
            parts.push(format!("{name}={value}"));
        }
    }
    for (given, name) in [
        (args.no_links, "no_links"),
        (args.no_island_filter, "no_island_filter"),
    ] {
        if given {
            parts.push(format!("{name}=true"));
        }
    }
    format!("to continue with navmesh_generate: {}", parts.join(", "))
}

/// A CLI message in the words of the MCP tools: `--sea-level` is `sea_level`, "Use --force" is "Pass force".
#[must_use]
pub fn mcp_wording(message: &str) -> String {
    let mut text = replace_word(message, "Use --force", "Pass force");
    for name in [
        "--skip-existing",
        "--no-island-filter",
        "--no-links",
        "--sea-level",
        "--connections",
        "--checkpoint",
        "--threads",
        "--capture",
        "--server",
        "--record",
        "--range",
        "--doors",
        "--force",
        "--base",
        "--seed",
        "--tile",
    ] {
        text = replace_word(&text, name, &name.trim_start_matches('-').replace('-', "_"));
    }
    text
}

/// Replaces `from` by `to` where it starts a word (at the start, or after whitespace or a quote) and
/// ends one (not followed by a letter, digit, `-` or `_`), so that a path like `map--base.nm` stays.
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let starts = prev.is_none_or(|p| p.is_whitespace() || p == '"' || p == '\'');
        let tail = rest.strip_prefix(from);
        let ends = tail.is_some_and(|t| {
            t.chars()
                .next()
                .is_none_or(|n| !(n.is_alphanumeric() || n == '-' || n == '_'))
        });
        if starts && ends {
            out.push_str(to);
            rest = tail.unwrap_or(rest);
            prev = to.chars().next_back();
        } else {
            out.push(c);
            prev = Some(c);
            rest = rest.get(c.len_utf8()..).unwrap_or("");
        }
    }
    out
}

fn write_summary(report: &GenerateReport, out: &mut dyn Write) -> Result<(), Error> {
    let prefix = if report.stopped.is_some() {
        "stopped: "
    } else {
        ""
    };
    writeln!(
        out,
        "{prefix}built {} tiles, {} empty, {} failed in {:.1} s",
        report.built,
        report.empty,
        report.failed.len(),
        report.seconds
    )?;
    let errors: Vec<String> = report
        .failed
        .iter()
        .map(|(x, z, message)| format!("tile ({x},{z}): {message}"))
        .chain(
            report
                .fetch_errors
                .iter()
                .map(|(x, z, message)| format!("tile ({x},{z}): {message}")),
        )
        .chain(report.stopped.iter().cloned())
        .collect();
    for line in errors.iter().take(LISTED_ERRORS) {
        writeln!(out, "  {line}")?;
    }
    if errors.len() > LISTED_ERRORS {
        writeln!(out, "  ... and {} more", errors.len() - LISTED_ERRORS)?;
    }
    if report.stray_tiles > 0 {
        writeln!(
            out,
            "{} tiles had out-of-range door part ids, which were ignored",
            report.stray_tiles
        )?;
    }
    if let Some(islands) = &report.islands {
        let at = report.seed.map_or_else(String::new, |s| {
            format!("seed ({:.1} {:.1} {:.1}) -> ", s[0], s[1], s[2])
        });
        writeln!(
            out,
            "island filter: {at}{} reachable, {} isolated polys",
            islands.reachable, islands.isolated
        )?;
    }
    Ok(())
}

/// `dayzmake navmesh generate`: builds the navmesh from a data server or a capture and saves it to `target`.
///
/// The mesh is saved even when the run was partial (failed tiles, fetch errors, a failed checkpoint); the
/// command then fails after saving.
///
/// # Errors
///
/// Returns an error for an existing target without `--force`, invalid arguments, unreadable inputs, a
/// failed run, or a run that did not complete.
pub fn cmd_navmesh_generate(
    args: &GenerateArgs,
    target: &Path,
    out: &mut dyn Write,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    let pending_line = Cell::new(false);
    let mut printer = ProgressPrinter::new(&pending_line);
    let mut sink = LineEnding {
        inner: warnings,
        pending_line: &pending_line,
    };
    run_generate(args, target, out, &mut sink, &mut printer)
}

/// The run behind `cmd_navmesh_generate` with the progress going to `progress`, so that the MCP tool can
/// keep it off its channel. Writes the summary to `out` before it fails for a partial run.
///
/// # Errors
///
/// The errors of `cmd_navmesh_generate`.
pub fn run_generate(
    args: &GenerateArgs,
    target: &Path,
    out: &mut dyn Write,
    warnings: &mut dyn WarningSink,
    progress: &mut dyn ProgressSink,
) -> Result<(), Error> {
    if !args.force && target.exists() {
        return Err(existing_target_error(target));
    }
    if args.capture.is_some() && !args.servers.is_empty() {
        return Err(error!("Use either --capture or --server, not both."));
    }
    let mut options = GenerateOptions::default();
    options.range = args.range.as_deref().map(parse_range).transpose()?;
    options.seed = args.seed.as_deref().map(parse_seed).transpose()?;
    options.skip_existing = args.skip_existing;
    options.island_filter = !args.no_island_filter;
    if let Some(text) = &args.threads {
        options.threads = parse_count(text, "--threads")?;
    }
    if let Some(text) = &args.connections {
        options.connections = parse_count(text, "--connections")?;
    }
    if let Some(text) = &args.checkpoint {
        options.checkpoint = parse_count(text, "--checkpoint")?.map(|n| (n, target.to_path_buf()));
    }
    options.build.links = !args.no_links;
    if let Some(text) = &args.doors {
        options.build.doors = parse_doors(text)?;
    }
    if let Some(text) = &args.sea_level {
        let level = text
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| error!("--sea-level: \"{text}\" is not a finite number"))?;
        options.build.sea_level = Some(level);
    }
    if let Some(record) = &args.record {
        let record = Path::new(record);
        if !args.force && record.exists() {
            return Err(existing_target_error(record));
        }
        options.record = Some(record.to_path_buf());
    }
    if let Some(base) = &args.base {
        options.base = Some(load_base(base)?);
    }
    let source = if let Some(path) = &args.capture {
        Source::CaptureFile(load_capture(path)?)
    } else {
        let names: Vec<&str> = if args.servers.is_empty() {
            vec![DEFAULT_HOST]
        } else {
            args.servers.iter().map(String::as_str).collect()
        };
        Source::Servers(
            names
                .into_iter()
                .map(Host::parse)
                .collect::<Result<Vec<_>, _>>()?,
        )
    };

    let result = generate(source, &options, &mut |p| progress.update(p), warnings);
    progress.finish();
    let report = result?;

    report
        .set
        .write_atomic(target)
        .prepend_error(format!("Failed to save {}:", target.display()))?;
    write_summary(&report, out)?;
    writeln!(out, "wrote {}", target.display())?;
    let partial =
        report.stopped.is_some() || (report.failed.is_empty() && !report.fetch_errors.is_empty());
    if partial {
        writeln!(out, "to continue: {}", resume_hint(args, target))?;
    }
    if let Some(message) = &report.stopped {
        return Err(error!("the run was stopped: {message}"));
    }
    if !report.failed.is_empty() || !report.fetch_errors.is_empty() {
        return Err(error!(
            "{} tiles could not be built and {} could not be fetched; the mesh was saved without them",
            report.failed.len(),
            report.fetch_errors.len()
        ));
    }
    Ok(())
}

/// One tile of a `.nm` for `info`.
struct TileRow {
    x: i32,
    z: i32,
    tile_ref: u64,
    polys: usize,
    verts: usize,
    off_mesh: usize,
    parts: usize,
    obstacles: usize,
}

fn tile_rows(set: &NavMeshSet) -> Vec<TileRow> {
    set.tiles
        .iter()
        .map(|t| TileRow {
            x: t.data.header.x,
            z: t.data.header.y,
            tile_ref: t.tile_ref,
            polys: t.data.polys.len(),
            verts: t.data.verts.len(),
            off_mesh: t.data.off_mesh.len(),
            parts: t.data.extra.as_ref().map_or(0, |e| e.parts.len()),
            obstacles: t.data.extra.as_ref().map_or(0, |e| e.obstacles.len()),
        })
        .collect()
}

/// `dayzmake navmesh info`: the grid and the totals of a `.nm` file, with `tiles` one line per tile.
///
/// # Errors
///
/// Returns an error if the file cannot be read or is not a navmesh set.
pub fn cmd_navmesh_info(
    source: &Path,
    tiles: bool,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let file = open_input(source, "navmesh")?;
    let set = NavMeshSet::read(&mut BufReader::new(file))
        .prepend_error(format!("Failed to read {}:", source.display()))?;
    let rows = tile_rows(&set);
    let polys: usize = rows.iter().map(|r| r.polys).sum();
    let verts: usize = rows.iter().map(|r| r.verts).sum();
    let off_mesh: usize = rows.iter().map(|r| r.off_mesh).sum();
    let p = &set.params;
    if json_output {
        let mut object = json!({
            "version": set.version,
            "tilesX": set.tiles_x,
            "tilesZ": set.tiles_z,
            "tileWidth": num(p.tile_width),
            "tileHeight": num(p.tile_height),
            "maxTiles": p.max_tiles,
            "maxPolys": p.max_polys,
            "tileCount": rows.len(),
            "polys": polys,
            "verts": verts,
            "offMesh": off_mesh,
        });
        if tiles && let Some(map) = object.as_object_mut() {
            map.insert(
                "tiles".into(),
                rows.iter()
                    .map(|r| {
                        json!({
                            "x": r.x, "z": r.z, "ref": r.tile_ref, "polys": r.polys,
                            "verts": r.verts, "offMesh": r.off_mesh, "parts": r.parts,
                            "obstacles": r.obstacles,
                        })
                    })
                    .collect(),
            );
        }
        writeln!(out, "{object}")?;
        return Ok(());
    }
    writeln!(
        out,
        "navmesh version {}, grid {} x {} tiles of {:.1} x {:.1} m, max tiles {}",
        set.version, set.tiles_x, set.tiles_z, p.tile_width, p.tile_height, p.max_tiles
    )?;
    writeln!(
        out,
        "tiles {}, polys {polys}, verts {verts}, off-mesh connections {off_mesh}",
        rows.len()
    )?;
    if tiles {
        writeln!(out, "x z ref polys verts offmesh parts obstacles")?;
        for r in &rows {
            writeln!(
                out,
                "{} {} {:#x} {} {} {} {} {}",
                r.x, r.z, r.tile_ref, r.polys, r.verts, r.off_mesh, r.parts, r.obstacles
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_words_are_quoted_when_needed() {
        assert_eq!(shell_word("a/b-c_1.nm"), "a/b-c_1.nm");
        assert_eq!(shell_word("0,0,2,0"), "0,0,2,0");
        assert_eq!(shell_word("my map.nm"), "'my map.nm'");
        assert_eq!(shell_word("it's"), "'it'\\''s'");
        assert_eq!(shell_word("a;b$c"), "'a;b$c'");
        assert_eq!(shell_word(""), "''");
    }

    #[test]
    fn resume_args_repeat_every_option_but_record_and_base() {
        let args = GenerateArgs {
            servers: vec!["10.0.0.1".into(), "10.0.0.2:40000".into()],
            record: Some("first.dzgeo".into()),
            range: Some("0,0,9,9".into()),
            base: Some("old.nm".into()),
            threads: Some("4".into()),
            no_links: true,
            ..GenerateArgs::default()
        };
        assert_eq!(
            resume_args(&args, Path::new("o.nm")),
            "to continue with navmesh_generate: target=o.nm, base=o.nm, skip_existing=true, force=true, \
             server=[10.0.0.1, 10.0.0.2:40000], range=0,0,9,9, threads=4, no_links=true"
        );
    }

    #[test]
    fn mcp_wording_names_arguments() {
        assert_eq!(
            mcp_wording("--sea-level: \"x\" is not a finite number; --skip-existing needs --base"),
            "sea_level: \"x\" is not a finite number; skip_existing needs base"
        );
        assert_eq!(
            mcp_wording("Target file \"a\" already exists. Use --force to overwrite."),
            "Target file \"a\" already exists. Pass force to overwrite."
        );
    }

    #[test]
    fn mcp_wording_leaves_paths_alone() {
        // An option name inside a word or a path is not an option.
        assert_eq!(
            mcp_wording(
                "Target file \"out/map--base.nm\" already exists. Use --force to overwrite."
            ),
            "Target file \"out/map--base.nm\" already exists. Pass force to overwrite."
        );
        assert_eq!(
            mcp_wording("cannot read 'my--seed' and x--force--range; --base 'a --seed b'"),
            "cannot read 'my--seed' and x--force--range; base 'a seed b'"
        );
        // At a boundary: start, after a space or a quote; before punctuation.
        assert_eq!(mcp_wording("--seed"), "seed");
        assert_eq!(mcp_wording("\"--seed\", '--range'"), "\"seed\", 'range'");
        assert_eq!(mcp_wording("--seed: bad"), "seed: bad");
        // A longer word is not the option.
        assert_eq!(mcp_wording("--seeds"), "--seeds");
    }

    #[test]
    fn resume_hint_repeats_every_option_but_record_and_base() {
        let args = GenerateArgs {
            force: false,
            servers: vec!["10.0.0.1".into(), "10.0.0.2:40000".into()],
            capture: None,
            record: Some("first.dzgeo".into()),
            range: Some("0,0,9,9".into()),
            base: Some("old.nm".into()),
            skip_existing: false,
            checkpoint: Some("50".into()),
            threads: Some("4".into()),
            connections: Some("2".into()),
            sea_level: Some("1.5".into()),
            doors: Some("open".into()),
            no_links: true,
            no_island_filter: true,
            seed: Some("1,2,3".into()),
        };
        let hint = resume_hint(&args, Path::new("out dir/my map.nm"));
        assert_eq!(
            hint,
            "dayzmake navmesh generate --base 'out dir/my map.nm' --skip-existing -f \
             --server 10.0.0.1 --server 10.0.0.2:40000 --range 0,0,9,9 --checkpoint 50 --threads 4 \
             --connections 2 --sea-level 1.5 --doors open --seed 1,2,3 --no-links --no-island-filter \
             'out dir/my map.nm'"
        );
        assert!(!hint.contains("--record") && !hint.contains("old.nm"));
        let capture = GenerateArgs {
            capture: Some("w.dzgeo".into()),
            ..GenerateArgs::default()
        };
        assert_eq!(
            resume_hint(&capture, Path::new("o.nm")),
            "dayzmake navmesh generate --base o.nm --skip-existing -f --capture w.dzgeo o.nm"
        );
    }
}
