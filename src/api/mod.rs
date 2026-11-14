//! `dayzmake api`: build and query an index of the DayZ script API.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Error;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;

use cache::dir_hash;
use dayzmake_core::error;
use dayzmake_core::error::{printable, warning};
use dayzmake_core::pbo::{PBO, SelectedEntry, safe_relative_path};
use model::ApiIndex;
use query::{Query, render_json, render_text, run_query, src_files};
use source::{
    InstalledMod, ModSource, ModSpec, build_mod, build_vanilla, installed_mods, installed_source,
    match_installed, parse_mods, resolve_mod,
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
pub fn list_mods(ctx: &ApiContext) -> Vec<InstalledMod> {
    let libs = libraries(&steam_roots());
    installed_mods(ctx.branch, &libs, game_dir(ctx, &libs).as_deref())
}

/// `dayzmake api mods`: one line per installed mod (`<id or ->  <name>  <dir>  [scripts]`), or JSON
/// (`{"mods": [...]}`).
///
/// # Errors
///
/// Returns an error when the JSON cannot be produced.
pub fn mods_text(ctx: &ApiContext, json: bool) -> Result<String, Error> {
    let mods = list_mods(ctx);
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

fn locate(ctx: &ApiContext) -> Result<Located, Error> {
    let libs = libraries(&steam_roots());
    let (scripts_pbo, build) = if let Some(dir) = &ctx.dayz {
        let pbo = dir.join("dta/scripts.pbo");
        if !pbo.is_file() {
            return Err(error!("{} not found (--dayz)", pbo.display()));
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
            installed_mods(ctx.branch, &libs, game_dir(ctx, &libs).as_deref())
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
/// warning; so is one whose worker thread panicked.
fn load_mods(found: &Located, branch: Branch, refresh: bool) -> Vec<Option<(String, ApiIndex)>> {
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<(String, ApiIndex)>> = found.mods.iter().map(|_| None).collect();
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
                        match cache::cached(&key, &m.pbos, refresh, || build_mod(m, branch)) {
                            Ok(index) => done.push((i, key, index)),
                            Err(e) => warning(
                                format!("skipping mod {}: {e}", m.name),
                                Some("api-bad-mod"),
                                (Some(m.dir.display().to_string()), None),
                            ),
                        }
                    }
                    done
                })
            })
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(done) => {
                    for (i, key, index) in done {
                        if let Some(slot) = out.get_mut(i) {
                            *slot = Some((key, index));
                        }
                    }
                }
                Err(_) => warning(
                    "an indexing thread panicked; its mods are skipped",
                    Some("api-bad-mod"),
                    (None, None),
                ),
            }
        }
    });
    out
}

/// The indexes of vanilla and of the located mods, with their cache keys.
fn load_located(ctx: &ApiContext, found: &Located) -> Result<Vec<(String, ApiIndex)>, Error> {
    let key = format!("vanilla-{}", ctx.branch.name());
    let index = cache::cached(
        &key,
        std::slice::from_ref(&found.scripts_pbo),
        ctx.refresh,
        || build_vanilla(&found.scripts_pbo, ctx.branch, &found.build),
    )?;
    let mut out = vec![(key, index)];
    out.extend(
        load_mods(found, ctx.branch, ctx.refresh)
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
pub fn load_keyed(ctx: &ApiContext) -> Result<Vec<(String, ApiIndex)>, Error> {
    load_located(ctx, &locate(ctx)?)
}

/// The indexes to query: vanilla at position 0, then the selected mods. Stale caches are rebuilt.
///
/// # Errors
///
/// See [`load_keyed`].
pub fn load_indexes(ctx: &ApiContext) -> Result<Vec<ApiIndex>, Error> {
    Ok(load_keyed(ctx)?.into_iter().map(|(_, i)| i).collect())
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
pub fn src_contents(ctx: &ApiContext, name: &str) -> Result<Vec<SrcContent>, Error> {
    let found = locate(ctx)?;
    let indexes: Vec<ApiIndex> = load_located(ctx, &found)?
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
                .ok_or_else(|| error!("mod {} is not selected", f.index))?;
            let rel = safe_relative_path(&f.pbo)
                .ok_or_else(|| error!("unsafe PBO path \"{}\" in the index", f.pbo))?;
            m.dir.join(rel)
        });
    }
    let mut read: BTreeMap<PathBuf, Vec<SelectedEntry>> = BTreeMap::new();
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
        let mut file = std::fs::File::open(path).map_err(|e| error!("{}: {e}", path.display()))?;
        let sel = PBO::read_entries(&mut file, |n| wanted.contains(n))
            .map_err(|e| error!("{}: {e}", path.display()))?;
        read.insert(path.clone(), sel.entries);
    }
    let mut out = Vec::new();
    for (f, path) in files.into_iter().zip(&paths) {
        let data = match read
            .get_mut(path)
            .and_then(|entries| entries.iter_mut().find(|(n, _)| *n == f.path))
        {
            Some((_, Ok(data))) => data.clone(),
            Some((_, Err(e))) => return Err(error!("{}: {e}", f.path)),
            None => return Err(error!("{} not found in {}", f.path, path.display())),
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
pub fn cat_sources(ctx: &ApiContext, name: &str) -> Result<String, Error> {
    let mut out = String::new();
    for f in src_contents(ctx, name)? {
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
pub fn update_lines(ctx: &ApiContext) -> Result<Vec<String>, Error> {
    Ok(load_keyed(ctx)?
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
) -> Result<Vec<String>, Error> {
    let indexes = load_indexes(ctx)?;
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

/// `api src --cat`: the full source files as JSON (`{"files": [...]}`) or as text.
///
/// # Errors
///
/// Returns an error when the name is unknown or a PBO cannot be read.
pub fn src_cat(ctx: &ApiContext, name: &str, json: bool) -> Result<String, Error> {
    if json {
        let files = src_contents(ctx, name)?;
        serde_json::to_string(&serde_json::json!({ "files": files }))
            .map_err(|e| error!("cannot serialize: {e}"))
    } else {
        cat_sources(ctx, name)
    }
}
