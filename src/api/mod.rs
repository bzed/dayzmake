//! `dayzmake api`: build and query an index of the DayZ script API.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Error;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;

use cache::dir_hash;
use dayzmake_core::error;
use dayzmake_core::error::printable;
use dayzmake_core::pbo::{PBO, PboSelection, safe_relative_path};
use dayzmake_core::warning::{Warning, WarningSink, Warnings};
use model::ApiIndex;
use query::{Query, compile_with, render_json, render_text, run_query, src_files};
use source::{
    InstalledMod, ModSource, ModSpec, build_mod, build_vanilla, index_entry_name, installed_mods,
    installed_source, match_installed, parse_mods, resolve_mod, script_texts,
};
use steam::{Branch, find_game, libraries, steam_roots};

/// What to load: the branch, where DayZ is, which mods.
#[derive(Debug, Clone)]
pub struct ApiContext {
    /// Stable or experimental.
    pub branch: Branch,
    /// A DayZ client or server directory (`<dir>/dta/scripts.pbo`); Steam discovery is skipped when set.
    pub dayz: Option<PathBuf>,
    /// Comma separated mod list (see `source::parse_mods`); empty means vanilla only. `all` indexes every
    /// installed mod with scripts, which can take a while the first time.
    pub mods: String,
    /// Rebuild the indexes even when the cache is current.
    pub refresh: bool,
}

/// Everything `ApiContext` points at.
struct Located {
    scripts_pbo: PathBuf,
    build: String,
    mods: Vec<ModSource>,
}

/// Where the game is installed: `--dayz` or the Steam install.
fn game_dir(ctx: &ApiContext, libs: &[PathBuf]) -> Option<PathBuf> {
    ctx.dayz
        .clone()
        .or_else(|| find_game(ctx.branch, libs).map(|g| g.dir))
}

/// Every installed mod of the context, for `dayzmake api mods`.
#[must_use]
pub fn list_mods(ctx: &ApiContext, warnings: &mut dyn WarningSink) -> Vec<InstalledMod> {
    let libs = libraries(&steam_roots());
    installed_mods(ctx.branch, &libs, game_dir(ctx, &libs).as_deref(), warnings)
}

/// `dayzmake api mods`: one line per installed mod (`<id or ->  <name>  <dir>  [scripts]`), or JSON
/// (`{"mods": [...]}`).
///
/// # Errors
///
/// Returns an error when the JSON cannot be produced.
pub fn mods_text(
    ctx: &ApiContext,
    json: bool,
    warnings: &mut dyn WarningSink,
) -> Result<String, Error> {
    let mods = list_mods(ctx, warnings);
    if json {
        return serde_json::to_string(&serde_json::json!({ "mods": mods }))
            .map_err(|e| error!("cannot serialize: {e}"));
    }
    Ok(mods
        .iter()
        .map(|m| {
            format!(
                "{}  {}  {}{}",
                m.id.map_or_else(|| "-".to_string(), |i| i.to_string()),
                printable(&m.name),
                printable(&m.dir.display().to_string()),
                if m.has_scripts { "  scripts" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn locate(ctx: &ApiContext, warnings: &mut dyn WarningSink) -> Result<Located, Error> {
    let libs = libraries(&steam_roots());
    let (scripts_pbo, build) = if let Some(dir) = &ctx.dayz {
        let pbo = dir.join("dta/scripts.pbo");
        if !pbo.is_file() {
            return Err(error!(
                "{} not found (--dayz)",
                printable(&pbo.display().to_string())
            ));
        }
        (pbo, format!("{} build unknown", ctx.branch.name()))
    } else {
        let game = find_game(ctx.branch, &libs).ok_or_else(|| {
            error!(
                "no DayZ {} install found in the Steam libraries; pass the install directory with --dayz <dir>",
                ctx.branch.name()
            )
        })?;
        let id = game.buildid.as_deref().unwrap_or("unknown");
        (
            game.dir.join("dta/scripts.pbo"),
            format!("{} build {id}", ctx.branch.name()),
        )
    };
    let mut mods: Vec<ModSource> = Vec::new();
    if !ctx.mods.trim().is_empty() {
        let explicit: Vec<String> = ctx
            .mods
            .split(',')
            .map(|t| t.trim().to_ascii_lowercase())
            .collect();
        let specs = parse_mods(&ctx.mods)?;
        let installed = if specs
            .iter()
            .any(|s| matches!(s, ModSpec::All | ModSpec::Name(_)))
        {
            installed_mods(ctx.branch, &libs, game_dir(ctx, &libs).as_deref(), warnings)
        } else {
            Vec::new()
        };
        let mut add = |m: ModSource| {
            let canon = m.dir.canonicalize().unwrap_or_else(|_| m.dir.clone());
            if !mods
                .iter()
                .any(|o| o.dir.canonicalize().unwrap_or_else(|_| o.dir.clone()) == canon)
            {
                mods.push(m);
            }
        };
        for spec in specs {
            match &spec {
                ModSpec::All => {
                    for m in installed.iter().filter(|m| m.has_scripts) {
                        add(installed_source(m)?);
                    }
                }
                ModSpec::Name(token) => add(installed_source(match_installed(token, &installed)?)?),
                _ => match resolve_mod(&spec, ctx.branch, &libs) {
                    Ok(m) => add(m),
                    // `aliases` means the installed aliases; an alias that is named explicitly must be there.
                    Err(_) if matches!(&spec, ModSpec::Alias(a, _) if !explicit.iter().any(|e| e == a)) =>
                        {}
                    Err(e) => return Err(e),
                },
            }
        }
    }
    unique_names(&mut mods);
    Ok(Located {
        scripts_pbo,
        build,
        mods,
    })
}

/// Make the index names unique and different from `vanilla`: a clash gets `-<workshop id>`, else `-2`, `-3`, ...
fn unique_names(mods: &mut [ModSource]) {
    let mut used: Vec<String> = vec!["vanilla".to_string()];
    for m in mods {
        if used.contains(&m.name) {
            let base = m.name.clone();
            let mut candidate = m.workshop.map(|id| format!("{base}-{id}"));
            let mut n = 2;
            while candidate.as_ref().is_none_or(|c| used.contains(c)) {
                candidate = Some(format!("{base}-{n}"));
                n += 1;
            }
            m.name = candidate.unwrap_or(base);
        }
        used.push(m.name.clone());
    }
}

fn mod_key(m: &ModSource, branch: Branch) -> String {
    format!("mod-{}-{}-{}", m.name, branch.name(), dir_hash(&m.dir))
}

/// Most mod indexes built at once. Each worker holds the scripts of one mod in memory.
const MAX_WORKERS: usize = 4;

/// How many mod indexes are built at once.
fn workers(jobs: usize) -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(MAX_WORKERS)
        .min(jobs)
        .max(1)
}

/// The cached or freshly built index of every mod, in the order of `mods`. A mod that fails is skipped with a
/// warning; so is one whose worker thread panicked. The workers collect their warnings per mod; they are
/// passed on in the order of `mods`, so the output does not depend on thread timing.
fn load_mods(
    found: &Located,
    branch: Branch,
    refresh: bool,
    warnings: &mut dyn WarningSink,
) -> Vec<Option<(String, ApiIndex)>> {
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<(String, ApiIndex)>> = found.mods.iter().map(|_| None).collect();
    let mut raised: Vec<Vec<Warning>> = found.mods.iter().map(|_| Vec::new()).collect();
    let mut panicked = false;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers(found.mods.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(m) = found.mods.get(i) else {
                            break;
                        };
                        let key = mod_key(m, branch);
                        let mut local = Warnings::new();
                        local.unlimited();
                        let index = match cache::cached(&key, &m.pbos, refresh, &mut local, |w| {
                            build_mod(m, branch, w)
                        }) {
                            Ok(index) => Some(index),
                            Err(e) => {
                                local.warn(
                                    Warning::new(
                                        Some("api-bad-mod"),
                                        format!("skipping mod {}: {e}", printable(&m.name)),
                                    )
                                    .in_file(m.dir.display().to_string()),
                                );
                                None
                            }
                        };
                        done.push((i, key, index, local.into_warnings()));
                    }
                    done
                })
            })
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(done) => {
                    for (i, key, index, w) in done {
                        if let Some(slot) = out.get_mut(i) {
                            *slot = index.map(|index| (key, index));
                        }
                        if let Some(slot) = raised.get_mut(i) {
                            *slot = w;
                        }
                    }
                }
                Err(_) => panicked = true,
            }
        }
    });
    for warning in raised.into_iter().flatten() {
        warnings.warn(warning);
    }
    if panicked {
        warnings.warn(Warning::new(
            Some("api-bad-mod"),
            "an indexing thread panicked; its mods are skipped",
        ));
    }
    out
}

/// The indexes of vanilla and of the located mods, with their cache keys.
fn load_located(
    ctx: &ApiContext,
    found: &Located,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<(String, ApiIndex)>, Error> {
    let key = format!("vanilla-{}", ctx.branch.name());
    let index = cache::cached(
        &key,
        std::slice::from_ref(&found.scripts_pbo),
        ctx.refresh,
        warnings,
        |w| build_vanilla(&found.scripts_pbo, ctx.branch, &found.build, w),
    )?;
    let mut out = vec![(key, index)];
    out.extend(
        load_mods(found, ctx.branch, ctx.refresh, warnings)
            .into_iter()
            .flatten(),
    );
    Ok(out)
}

/// The indexes with their cache keys: vanilla first, then the selected mods. The mods that are not cached
/// are indexed in parallel.
///
/// # Errors
///
/// Returns an error when the install or a selected mod is missing or the vanilla PBO or the cache cannot be
/// read. A mod that cannot be indexed is skipped with a warning.
pub fn load_keyed(
    ctx: &ApiContext,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<(String, ApiIndex)>, Error> {
    let found = locate(ctx, warnings)?;
    load_located(ctx, &found, warnings)
}

/// The indexes to query: vanilla at position 0, then the selected mods. Stale caches are rebuilt.
///
/// # Errors
///
/// See [`load_keyed`].
pub fn load_indexes(
    ctx: &ApiContext,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<ApiIndex>, Error> {
    Ok(load_keyed(ctx, warnings)?
        .into_iter()
        .map(|(_, i)| i)
        .collect())
}

/// One source file that declares a class or enum, with its full text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SrcContent {
    /// Name of the index (`vanilla` or the mod).
    pub index: String,
    /// PBO file relative to its install.
    pub pbo: String,
    /// PBO prefix, `\`-separated.
    pub prefix: String,
    /// Entry name inside the PBO, `\`-separated.
    pub path: String,
    /// 1-based line of the declaration.
    pub line: u32,
    /// The whole file, lossily decoded.
    pub text: String,
}

/// Every file that declares the class or enum `name`, read from its PBO again.
///
/// # Errors
///
/// Returns an error when the name is unknown or a PBO cannot be read.
pub fn src_contents(
    ctx: &ApiContext,
    name: &str,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<SrcContent>, Error> {
    let found = locate(ctx, warnings)?;
    let indexes: Vec<ApiIndex> = load_located(ctx, &found, warnings)?
        .into_iter()
        .map(|(_, i)| i)
        .collect();
    let files = src_files(&indexes, name);
    if files.is_empty() {
        return Err(error!("no class or enum named \"{name}\""));
    }
    // Resolve every file to its PBO, then read each PBO once, seeking to just the needed entries.
    let mut paths: Vec<PathBuf> = Vec::new();
    for f in &files {
        // Index names are unique and a mod is never called `vanilla` (see `unique_names`).
        paths.push(if f.index == "vanilla" {
            found.scripts_pbo.clone()
        } else {
            let m = found
                .mods
                .iter()
                .find(|m| m.name == f.index)
                .ok_or_else(|| error!("mod {} is not selected", printable(&f.index)))?;
            let rel = safe_relative_path(&f.pbo)
                .ok_or_else(|| error!("unsafe PBO path \"{}\" in the index", printable(&f.pbo)))?;
            m.dir.join(rel)
        });
    }
    let mut read: BTreeMap<PathBuf, PboSelection> = BTreeMap::new();
    for path in &paths {
        if read.contains_key(path) {
            continue;
        }
        let wanted: BTreeSet<&str> = files
            .iter()
            .zip(&paths)
            .filter(|(_, p)| *p == path)
            .map(|(f, _)| f.path.as_str())
            .collect();
        let mut file = std::fs::File::open(path)
            .map_err(|e| error!("{}: {e}", printable(&path.display().to_string())))?;
        let sel = PBO::read_entries(&mut file, |n| {
            index_entry_name(n).is_some_and(|k| wanted.contains(k.as_str()))
        })
        .map_err(|e| error!("{}: {e}", printable(&path.display().to_string())))?;
        read.insert(path.clone(), sel);
    }
    let mut out = Vec::new();
    for (f, path) in files.into_iter().zip(&paths) {
        let data = match read
            .get(path)
            // two raw names that normalise to one index name resolve to the first match
            .and_then(|sel| {
                sel.entries()
                    .iter()
                    .find(|e| index_entry_name(e.name()).as_deref() == Some(f.path.as_str()))
            }) {
            Some(entry) => match entry.data() {
                Ok(data) => data.to_vec(),
                Err(e) => return Err(error!("{}: {e}", printable(&f.path))),
            },
            None => {
                return Err(error!(
                    "{} not found in {}",
                    printable(&f.path),
                    printable(&path.display().to_string())
                ));
            }
        };
        out.push(SrcContent {
            index: f.index,
            pbo: f.pbo,
            prefix: f.prefix,
            path: f.path,
            line: f.line,
            text: String::from_utf8_lossy(&data).into_owned(),
        });
    }
    Ok(out)
}

/// The text form of [`src_contents`]: `// ==== [<index>] <pbo> <prefix>\<path>:<line>` and the file, a blank
/// line between files.
///
/// # Errors
///
/// See [`src_contents`].
pub fn cat_sources(
    ctx: &ApiContext,
    name: &str,
    warnings: &mut dyn WarningSink,
) -> Result<String, Error> {
    let mut out = String::new();
    for f in src_contents(ctx, name, warnings)? {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "// ==== [{}] {} {}\\{}:{}",
            f.index, f.pbo, f.prefix, f.path, f.line
        ); // writing to a String cannot fail
        out.push_str(&f.text);
        if !f.text.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

/// One line of a script that `api grep` matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct GrepHit {
    /// Name of the index (`vanilla` or the mod).
    pub source: String,
    /// `<PBO prefix>\<entry name>`: the path part of what `api src` prints (without the PBO file).
    pub path: String,
    /// 1-based line.
    pub line: u32,
    /// The raw line, lossily decoded.
    pub text: String,
}

/// What `api grep` found: the first `max` hits in order and how many more there were.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GrepResult {
    /// The hits, sorted by source (vanilla first, then the mods in `-m` order), path and line.
    pub hits: Vec<GrepHit>,
    /// Matching lines beyond `max`, all counted.
    pub more: usize,
    /// Whether more than one source was searched (the text output then tags each line with its source).
    pub multi: bool,
}

/// Searches every `.c` file of vanilla and of the selected mods, straight from the PBOs, for lines that
/// match `regex`. Nothing is written and no index is needed. The whole input is scanned even after `max`
/// hits, so that `more` is exact.
///
/// # Errors
///
/// Returns an error for an invalid regex, a missing install or mod, or an unreadable vanilla PBO. A mod PBO
/// that cannot be read is skipped with a warning.
pub fn grep(
    ctx: &ApiContext,
    regex: &str,
    ignore_case: bool,
    max: usize,
    warnings: &mut dyn WarningSink,
) -> Result<GrepResult, Error> {
    let rx = compile_with(regex, ignore_case)?;
    let found = locate(ctx, warnings)?;
    let mut sources: Vec<(&str, bool, &[PathBuf])> =
        vec![("vanilla", true, std::slice::from_ref(&found.scripts_pbo))];
    sources.extend(
        found
            .mods
            .iter()
            .map(|m| (m.name.as_str(), false, m.pbos.as_slice())),
    );
    let multi = sources.len() > 1;
    let mut result = GrepResult {
        hits: Vec::new(),
        more: 0,
        multi,
    };
    for (name, vanilla, pbos) in sources {
        // the hits of this source; cut to what is left of `max` after every PBO, the rest is only counted
        let mut kept: Vec<GrepHit> = Vec::new();
        let left = max.saturating_sub(result.hits.len());
        for path in pbos {
            let texts = if vanilla {
                script_texts(path, true, warnings)?
            } else {
                match script_texts(path, false, warnings) {
                    Ok(t) => t,
                    Err(e) => {
                        warnings.warn(
                            Warning::new(
                                Some("api-bad-pbo"),
                                format!("skipping unreadable PBO: {e}"),
                            )
                            .in_file(path.display().to_string()),
                        );
                        continue;
                    }
                }
            };
            let mut mine = 0usize;
            let Some((prefix, texts)) = texts else {
                continue;
            };
            // Entries in path order and lines in order give this PBO's hits in output order, so only the
            // first `left` need to be built; the others are counted. Memory follows `left`, not the matches.
            let mut texts = texts;
            texts.sort_by(|a, b| a.0.cmp(&b.0));
            for (entry, text) in &texts {
                let mut shown: Option<String> = None;
                for (i, line) in text.lines().enumerate() {
                    if !rx.is_match(line) {
                        continue;
                    }
                    if mine >= left {
                        result.more += 1;
                        continue;
                    }
                    mine += 1;
                    let path = shown
                        .get_or_insert_with(|| format!("{prefix}\\{entry}"))
                        .clone();
                    kept.push(GrepHit {
                        source: name.to_string(),
                        path,
                        line: u32::try_from(i + 1).unwrap_or(u32::MAX),
                        text: line.to_string(),
                    });
                }
            }
            // merge with the PBOs before: keep the first `left` overall, count the others
            kept.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
            if kept.len() > left {
                result.more += kept.len() - left;
                kept.truncate(left);
            }
        }
        result.hits.append(&mut kept);
    }
    Ok(result)
}

/// The output of `api grep`: JSON (an array of `{source, path, line, text}`, cut to `max`) or text lines
/// (`[<source>] <path>:<line>: <text>`, the tag only when mods are selected), then the `... N more lines`
/// note.
///
/// # Errors
///
/// See [`grep`].
pub fn grep_lines(
    ctx: &ApiContext,
    regex: &str,
    ignore_case: bool,
    max: usize,
    json: bool,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<String>, Error> {
    let r = grep(ctx, regex, ignore_case, max, warnings)?;
    if json {
        return Ok(vec![
            serde_json::to_string(&r.hits).map_err(|e| error!("cannot serialize: {e}"))?,
        ]);
    }
    let mut out: Vec<String> = r
        .hits
        .iter()
        .map(|h| {
            let tag = if r.multi {
                format!("[{}] ", h.source)
            } else {
                String::new()
            };
            // paths and text come from files the user did not write
            printable(&format!("{tag}{}:{}: {}", h.path, h.line, h.text))
        })
        .collect();
    if out.is_empty() && r.more == 0 {
        out.push(if r.multi {
            "no match".to_string()
        } else {
            "no match  (searched vanilla only; add -m <mod> or -m all for mods)".to_string()
        });
    }
    if r.more > 0 {
        out.push(format!(
            "... {} more lines (raise --max or narrow the query)",
            r.more
        ));
    }
    Ok(out)
}

/// Steam and DayZ install discovery.
pub mod steam;

/// The JSON cache of the indexes.
pub mod cache;
/// Script sources: the PBOs of the game and of mods.
pub mod source;

/// Index model.
pub mod model;
/// Enforce Script declaration parser.
pub mod parse;
/// Lookups on the index and their text and JSON renderers.
pub mod query;
/// Scanner: comments, docs and conditional compilation.
mod scan;

/// One line per index with its key, class count and build, as `dayzmake api update` prints them.
/// The caller sets `refresh` to rebuild the caches.
///
/// # Errors
///
/// See [`load_keyed`].
pub fn update_lines(
    ctx: &ApiContext,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<String>, Error> {
    Ok(load_keyed(ctx, warnings)?
        .into_iter()
        .map(|(key, index)| format!("{key}: {} classes, {}", index.classes.len(), index.build))
        .collect())
}

/// Runs `query` and renders the answer: JSON (one line) or text (at most `max` lines, with a
/// `# <index>: <build>` header per index when `verbose`).
///
/// # Errors
///
/// Returns an error when the indexes cannot be loaded or the query fails.
pub fn answer(
    ctx: &ApiContext,
    query: &Query,
    max: usize,
    json: bool,
    verbose: bool,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<String>, Error> {
    let indexes = load_indexes(ctx, warnings)?;
    let result = run_query(&indexes, query)?;
    if json {
        return Ok(vec![render_json(&result, max)]);
    }
    let mut lines = Vec::new();
    if verbose {
        lines.extend(
            indexes
                .iter()
                .map(|i| printable(&format!("# {}: {}", i.name, i.build))),
        );
    }
    lines.extend(render_text(&result, indexes.len() > 1, max));
    Ok(lines)
}

/// A part of each source file, as `api src --context` and `--lines` select it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SrcRange {
    /// `n` lines before and after the declaration.
    Context(usize),
    /// The lines `from` to `to`, both 1-based and inclusive.
    Lines(usize, usize),
}

impl SrcRange {
    /// The range the options ask for; `None` when neither is given. `--context` and `--lines` exclude each
    /// other and `--cat`.
    ///
    /// # Errors
    ///
    /// Returns an error for a combination of the options, a value that is not a number (`--context`) or
    /// `<from>-<to>` (`--lines`), or `from` greater than `to`.
    pub fn parse(
        cat: bool,
        context: Option<&str>,
        lines: Option<&str>,
    ) -> Result<Option<SrcRange>, Error> {
        if context.is_some() && lines.is_some() {
            return Err(error!("--context and --lines exclude each other"));
        }
        if cat && (context.is_some() || lines.is_some()) {
            return Err(error!("--cat excludes --context and --lines"));
        }
        if let Some(c) = context {
            let n = c
                .trim()
                .parse::<usize>()
                .map_err(|_| error!("--context must be a number, not \"{}\"", printable(c)))?;
            return Ok(Some(SrcRange::Context(n)));
        }
        let Some(l) = lines else { return Ok(None) };
        let bad = || {
            error!(
                "--lines must be <from>-<to>, for example 10-20, not \"{}\"",
                printable(l)
            )
        };
        let (a, b) = l.split_once('-').ok_or_else(bad)?;
        let a = a.trim().parse::<usize>().map_err(|_| bad())?;
        let b = b.trim().parse::<usize>().map_err(|_| bad())?;
        if a > b {
            return Err(error!("--lines {a}-{b}: the start is after the end"));
        }
        Ok(Some(SrcRange::Lines(a, b)))
    }

    /// The 1-based inclusive bounds in a file of `total` lines, clamped to the file; `from > to` means
    /// nothing is selected.
    fn bounds(self, decl: u32, total: usize) -> (usize, usize) {
        let (from, to) = match self {
            SrcRange::Context(n) => {
                let d = usize::try_from(decl).unwrap_or(usize::MAX);
                (d.saturating_sub(n), d.saturating_add(n))
            }
            SrcRange::Lines(a, b) => (a, b),
        };
        (from.max(1), to.min(total))
    }
}

/// `api src --context` or `--lines`: the selected lines of each file that declares `name`, as
/// `// ==== ...` headers followed by `<line>| <text>` (the line number right-aligned to the widest in the range)
/// or as JSON (`{"files": [{..., "lines": [{"line": n, "text": "..."}]}]}`).
///
/// # Errors
///
/// See [`src_contents`].
pub fn src_range(
    ctx: &ApiContext,
    name: &str,
    range: SrcRange,
    json: bool,
    warnings: &mut dyn WarningSink,
) -> Result<String, Error> {
    let files = src_contents(ctx, name, warnings)?;
    let mut out = String::new();
    let mut jfiles = Vec::new();
    for f in &files {
        let total = f.text.lines().count();
        let (from, to) = range.bounds(f.line, total);
        let picked: Vec<(usize, &str)> = if from > to {
            Vec::new()
        } else {
            f.text
                .lines()
                .enumerate()
                .skip(from - 1)
                .take(to - from + 1)
                .map(|(i, t)| (i + 1, t))
                .collect()
        };
        if json {
            let lines: Vec<serde_json::Value> = picked
                .iter()
                .map(|(n, t)| serde_json::json!({"line": n, "text": t}))
                .collect();
            jfiles.push(serde_json::json!({
                "index": f.index, "pbo": f.pbo, "prefix": f.prefix,
                "path": f.path, "line": f.line, "lines": lines,
            }));
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "// ==== [{}] {} {}\\{}:{}",
            f.index, f.pbo, f.prefix, f.path, f.line
        ); // writing to a String cannot fail
        let width = picked.last().map_or(1, |(n, _)| n.to_string().len());
        for (n, t) in picked {
            let _ = writeln!(out, "{n:>width$}| {t}"); // writing to a String cannot fail
        }
    }
    if json {
        return serde_json::to_string(&serde_json::json!({ "files": jfiles }))
            .map_err(|e| error!("cannot serialize: {e}"));
    }
    Ok(out)
}

/// `api src --cat`: the full source files as JSON (`{"files": [...]}`) or as text.
///
/// # Errors
///
/// Returns an error when the name is unknown or a PBO cannot be read.
pub fn src_cat(
    ctx: &ApiContext,
    name: &str,
    json: bool,
    warnings: &mut dyn WarningSink,
) -> Result<String, Error> {
    if json {
        let files = src_contents(ctx, name, warnings)?;
        serde_json::to_string(&serde_json::json!({ "files": files }))
            .map_err(|e| error!("cannot serialize: {e}"))
    } else {
        cat_sources(ctx, name, warnings)
    }
}
