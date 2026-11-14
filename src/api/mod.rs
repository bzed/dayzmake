//! `dayzmake api`: build and query an index of the DayZ script API.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Error;
use std::path::PathBuf;

use crate::error;
use crate::pbo::{PBO, safe_relative_path};
use cache::dir_hash;
use model::ApiIndex;
use query::src_files;
use source::{ModSource, ModSpec, build_mod, build_vanilla, parse_mods, resolve_mod};
use steam::{Branch, find_game, libraries, steam_roots};

/// What to load: the branch, where DayZ is, which mods.
#[derive(Debug, Clone)]
pub struct ApiContext {
    /// Stable or experimental.
    pub branch: Branch,
    /// A DayZ client or server directory (`<dir>/dta/scripts.pbo`); Steam discovery is skipped when set.
    pub dayz: Option<PathBuf>,
    /// Comma separated mod list (see `source::parse_mods`); empty means vanilla only.
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
        for spec in parse_mods(&ctx.mods)? {
            match resolve_mod(&spec, ctx.branch, &libs) {
                Ok(m) => mods.push(m),
                // `all` means the installed aliases; an alias that is named explicitly must be there.
                Err(_) if matches!(&spec, ModSpec::Alias(a, _) if !explicit.iter().any(|e| e == a)) =>
                    {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(Located {
        scripts_pbo,
        build,
        mods,
    })
}

fn mod_key(m: &ModSource, branch: Branch) -> String {
    format!("mod-{}-{}-{}", m.name, branch.name(), dir_hash(&m.dir))
}

/// The indexes with their cache keys: vanilla first, then the selected mods.
///
/// # Errors
///
/// Returns an error when the install or a selected mod is missing or a PBO or the cache cannot be read.
pub fn load_keyed(ctx: &ApiContext) -> Result<Vec<(String, ApiIndex)>, Error> {
    let found = locate(ctx)?;
    let mut out = Vec::new();
    let key = format!("vanilla-{}", ctx.branch.name());
    let index = cache::cached(
        &key,
        std::slice::from_ref(&found.scripts_pbo),
        ctx.refresh,
        || build_vanilla(&found.scripts_pbo, ctx.branch, &found.build),
    )?;
    out.push((key, index));
    for m in &found.mods {
        let key = mod_key(m, ctx.branch);
        let index = cache::cached(&key, &m.pbos, ctx.refresh, || build_mod(m, ctx.branch))?;
        out.push((key, index));
    }
    Ok(out)
}

/// The indexes to query: vanilla at position 0, then the selected mods. Stale caches are rebuilt.
///
/// # Errors
///
/// See [`load_keyed`].
pub fn load_indexes(ctx: &ApiContext) -> Result<Vec<ApiIndex>, Error> {
    Ok(load_keyed(ctx)?.into_iter().map(|(_, i)| i).collect())
}

/// The full text of every file that declares the class or enum `name`, read from its PBO again.
///
/// # Errors
///
/// Returns an error when the name is unknown or a PBO cannot be read.
pub fn cat_sources(ctx: &ApiContext, name: &str) -> Result<String, Error> {
    let found = locate(ctx)?;
    let indexes = load_indexes(ctx)?;
    let files = src_files(&indexes, name);
    if files.is_empty() {
        return Err(error!("no class or enum named \"{name}\""));
    }
    let mut pbos: BTreeMap<PathBuf, PBO> = BTreeMap::new();
    let mut out = String::new();
    for f in files {
        let path = if f.index == "vanilla" {
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
        };
        if !pbos.contains_key(&path) {
            let mut file =
                std::fs::File::open(&path).map_err(|e| error!("{}: {e}", path.display()))?;
            let pbo = PBO::read(&mut file).map_err(|e| error!("{}: {e}", path.display()))?;
            pbos.insert(path.clone(), pbo);
        }
        let data = pbos
            .get(&path)
            .map(|p| p.file_data(&f.path))
            .transpose()?
            .flatten()
            .ok_or_else(|| error!("{} not found in {}", f.path, path.display()))?;
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "// ==== [{}] {} {}\\{}:{}",
            f.index, f.pbo, f.prefix, f.path, f.line
        ); // writing to a String cannot fail
        let text = String::from_utf8_lossy(&data);
        out.push_str(&text);
        if !text.ends_with('\n') {
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
