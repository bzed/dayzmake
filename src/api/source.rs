//! Script sources of the installed game and mods: the PBOs, mod selection and index building.
//!
//! PBO bytes, prefixes and entry names are untrusted. Text is read lossily, entries and prefixes that fail
//! [`safe_relative_path`] are skipped with a warning, and no file system path is ever built from PBO content.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufReader, Cursor, Error, Read};
use std::path::{Path, PathBuf};

use regex::Regex;

use super::model::{ApiIndex, Module};
use super::parse::{ScriptFile, active_defines, collect_defines, index_scripts};
use super::steam::{Branch, workshop_dir};
use crate::config::Config;
use crate::error;
use crate::error::{printable, warning};
use crate::pbo::{PBO, safe_relative_path};

/// Mod aliases: name and Steam workshop id.
const ALIASES: [(&str, u64); 4] = [
    ("cf", 1_559_212_036),
    ("cot", 1_564_026_768),
    ("dabs", 2_545_327_648),
    ("expansion", 2_572_331_007),
];

/// Largest `meta.cpp` that is read.
const MAX_META_BYTES: u64 = 1024 * 1024;

/// A mod as named on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModSpec {
    /// A well-known mod: alias and workshop id.
    Alias(&'static str, u64),
    /// A workshop id.
    Workshop(u64),
    /// A mod directory.
    Dir(PathBuf),
    /// Every installed mod that has scripts.
    All,
    /// The name of an installed mod (its `meta.cpp` name or its `@dir` name), matched case-insensitively.
    Name(String),
}

/// Parse a comma separated mod list: an alias (`cf`, `cot`, `dabs`, `expansion`), `aliases` (all four),
/// `all` (every installed mod with scripts), a workshop id (digits), an existing path or anything with a
/// path separator (a directory), or else the name of an installed mod. Duplicates are dropped.
///
/// # Errors
///
/// Returns an error for an empty list or an empty entry.
pub fn parse_mods(spec: &str) -> Result<Vec<ModSpec>, Error> {
    let mut out: Vec<ModSpec> = Vec::new();
    let mut add = |m: ModSpec| {
        if !out.contains(&m) {
            out.push(m);
        }
    };
    for token in spec.split(',').map(str::trim) {
        if token.is_empty() {
            return Err(error!("empty entry in the mod list \"{spec}\""));
        }
        let lower = token.to_ascii_lowercase();
        if lower == "all" {
            add(ModSpec::All);
        } else if lower == "aliases" {
            for (name, id) in ALIASES {
                add(ModSpec::Alias(name, id));
            }
        } else if let Some((name, id)) = ALIASES.iter().find(|(n, _)| *n == lower) {
            add(ModSpec::Alias(name, *id));
        } else if token.bytes().all(|b| b.is_ascii_digit()) {
            let id = token
                .parse::<u64>()
                .map_err(|_| error!("workshop id \"{token}\" is out of range"))?;
            add(ModSpec::Workshop(id));
        } else if Path::new(token).exists() || token.contains(['/', '\\']) {
            add(ModSpec::Dir(PathBuf::from(token)));
        } else {
            add(ModSpec::Name(token.to_string()));
        }
    }
    Ok(out)
}

/// A resolved mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModSource {
    /// Alias, or the directory name with everything but `[A-Za-z0-9_.-]` replaced by `_`.
    pub name: String,
    /// The mod directory.
    pub dir: PathBuf,
    /// `<dir>/addons/*.pbo` (the `addons` directory is found case-insensitively), sorted.
    pub pbos: Vec<PathBuf>,
    /// Workshop id, when the mod was named by alias or id.
    pub workshop: Option<u64>,
}

/// Replace everything but `[A-Za-z0-9_.-]` by `_`.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A child of `dir` whose name equals `name` ignoring ASCII case.
fn child_ignore_case(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Find the directory of a mod and its PBOs.
///
/// # Errors
///
/// Returns an error when the mod is not installed or the directory does not exist.
pub fn resolve_mod(
    spec: &ModSpec,
    branch: Branch,
    libraries: &[PathBuf],
) -> Result<ModSource, Error> {
    let (name, dir, workshop) = match spec {
        ModSpec::Alias(alias, id) => (
            (*alias).to_string(),
            workshop_mod_dir(alias, *id, branch, libraries)?,
            Some(*id),
        ),
        ModSpec::Workshop(id) => (
            id.to_string(),
            workshop_mod_dir(&id.to_string(), *id, branch, libraries)?,
            Some(*id),
        ),
        ModSpec::All | ModSpec::Name(_) => {
            return Err(error!(
                "mod {spec:?} must be resolved against the installed mods"
            ));
        }
        ModSpec::Dir(path) => {
            let dir = path
                .canonicalize()
                .map_err(|e| error!("mod directory \"{}\": {e}", path.display()))?;
            if !dir.is_dir() {
                return Err(error!("\"{}\" is not a directory", path.display()));
            }
            let name = dir
                .file_name()
                .map(|n| sanitize(&n.to_string_lossy()))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "mod".to_string());
            (name, dir, None)
        }
    };
    let mut pbos: Vec<PathBuf> = child_ignore_case(&dir, "addons")
        .filter(|a| a.is_dir())
        .and_then(|a| fs::read_dir(a).ok())
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pbo"))
                })
                .collect()
        })
        .unwrap_or_default();
    pbos.sort();
    Ok(ModSource {
        name,
        dir,
        pbos,
        workshop,
    })
}

/// `<workshop content>/<id>` of the branch.
fn workshop_mod_dir(
    what: &str,
    id: u64,
    branch: Branch,
    libraries: &[PathBuf],
) -> Result<PathBuf, Error> {
    let dir = workshop_dir(branch, libraries)
        .map(|w| w.join(id.to_string()))
        .filter(|d| d.is_dir());
    dir.ok_or_else(|| {
        error!(
            "mod {what} (workshop {id}) is not installed for the {} branch; subscribe to it on the Steam workshop or pass its directory",
            branch.name()
        )
    })
}

/// Entry names of the `.c` scripts of a PBO, safe ones only.
fn script_entries(pbo: &PBO, origin: &str) -> Vec<String> {
    let mut out = Vec::new();
    for name in pbo.files.keys() {
        if !Path::new(&name.replace('\\', "/"))
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("c"))
        {
            continue;
        }
        if safe_relative_path(name).is_none() {
            warning(
                format!("skipping unsafe entry name {:?}", printable(name)),
                Some("api-unsafe-entry"),
                (Some(origin.to_string()), None),
            );
            continue;
        }
        out.push(name.clone());
    }
    out
}

/// The text of every named entry, lossily decoded; entries that cannot be read are skipped with a warning.
fn entry_texts(pbo: &PBO, names: Vec<String>, origin: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in names {
        match pbo.file_data(&name) {
            Ok(Some(bytes)) => out.push((name, String::from_utf8_lossy(&bytes).into_owned())),
            Ok(None) => {}
            Err(e) => warning(
                format!("cannot read {:?}: {e}", printable(&name)),
                Some("api-bad-entry"),
                (Some(origin.to_string()), None),
            ),
        }
    }
    out
}

fn read_pbo(path: &Path) -> Result<PBO, Error> {
    let mut file = File::open(path).map_err(|e| error!("{}: {e}", path.display()))?;
    PBO::read(&mut file).map_err(|e| error!("{}: {e}", path.display()))
}

/// Index `dta/scripts.pbo`: one module, prefix `scripts`.
///
/// # Errors
///
/// Returns an error when the PBO cannot be read.
pub fn build_vanilla(scripts_pbo: &Path, branch: Branch, build: &str) -> Result<ApiIndex, Error> {
    let origin = scripts_pbo.display().to_string();
    let pbo = read_pbo(scripts_pbo)?;
    let texts = entry_texts(&pbo, script_entries(&pbo, &origin), &origin);
    let files: Vec<ScriptFile<'_>> = texts
        .iter()
        .map(|(path, text)| ScriptFile {
            module: 0,
            path,
            text,
        })
        .collect();
    let mut index = ApiIndex::new("vanilla", "DayZ vanilla scripts", build);
    index.modules.push(Module {
        prefix: "scripts".to_string(),
        pbo: "dta/scripts.pbo".to_string(),
    });
    index_scripts(&files, &active_defines(branch), &mut index);
    Ok(index)
}

/// `name = "..."` of a `meta.cpp`.
fn meta_name(dir: &Path) -> Option<String> {
    let mut buf = Vec::new();
    File::open(dir.join("meta.cpp"))
        .ok()?
        .take(MAX_META_BYTES)
        .read_to_end(&mut buf)
        .ok()?;
    let text = String::from_utf8_lossy(&buf);
    let re = Regex::new(r#"(?m)^\s*name\s*=\s*"([^"]*)""#).ok()?;
    re.captures(&text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|n| !n.is_empty())
}

/// The `config.bin` (derapified) or `config.cpp` text of a PBO, empty when there is none.
fn config_text(pbo: &PBO, origin: &str) -> String {
    let find = |wanted: &str| pbo.files.keys().find(|n| n.eq_ignore_ascii_case(wanted));
    if let Some(name) = find("config.bin")
        && let Ok(Some(bytes)) = pbo.file_data(name)
    {
        return match Config::read_rapified(&mut Cursor::new(bytes)).and_then(|c| c.to_string()) {
            Ok(text) => text,
            Err(e) => {
                warning(
                    format!("cannot read config.bin: {e}"),
                    Some("api-bad-config"),
                    (Some(origin.to_string()), None),
                );
                String::new()
            }
        };
    }
    if let Some(name) = find("config.cpp")
        && let Ok(Some(bytes)) = pbo.file_data(name)
    {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    String::new()
}

/// The extension and entry names of a PBO, read from its header block only.
fn pbo_entry_names(path: &Path) -> Result<Vec<String>, Error> {
    let file = File::open(path).map_err(|e| error!("{}: {e}", path.display()))?;
    PBO::read_headers(&mut BufReader::new(file))
        .map(|(_, names)| names)
        .map_err(|e| error!("{}: {e}", path.display()))
}

/// The extension of an entry name (either path separator).
fn entry_extension(name: &str) -> Option<&str> {
    name.rsplit(['/', '\\'])
        .next()?
        .rsplit_once('.')
        .map(|(_, e)| e)
}

/// Whether the PBO has a `.c` script or a `config.*` entry (header block only; nothing else is read).
fn has_sources(path: &Path) -> Result<bool, Error> {
    Ok(pbo_entry_names(path)?.iter().any(|n| {
        entry_extension(n).is_some_and(|e| e.eq_ignore_ascii_case("c"))
            || n.rsplit(['/', '\\'])
                .next()
                .and_then(|f| f.split_once('.'))
                .is_some_and(|(stem, _)| stem.eq_ignore_ascii_case("config"))
    }))
}

/// One readable PBO of a mod.
struct ModPbo {
    prefix: String,
    rel: String,
    config: String,
    texts: Vec<(String, String)>,
}

/// The PBO prefix in `\` form; the file stem when there is none. `None` when it is unsafe.
fn pbo_prefix(pbo: &PBO, path: &Path, origin: &str) -> Option<String> {
    let prefix = match pbo.header_extensions.get("prefix") {
        Some(p) if !p.trim().is_empty() => {
            p.trim().replace('/', "\\").trim_matches('\\').to_string()
        }
        _ => path.file_stem()?.to_string_lossy().into_owned(),
    };
    if safe_relative_path(&prefix).is_none() {
        warning(
            format!("skipping PBO with unsafe prefix {:?}", printable(&prefix)),
            Some("api-unsafe-prefix"),
            (Some(origin.to_string()), None),
        );
        return None;
    }
    Some(prefix)
}

/// Index the script PBOs of a mod: one module per PBO.
///
/// The defines of the index are those of a dedicated server plus everything the mod's `config.cpp` or
/// `config.bin` files and scripts define.
///
/// # Errors
///
/// Never fails for a bad PBO (it is skipped with a warning); the `Result` is kept for the callers' sake.
pub fn build_mod(m: &ModSource, branch: Branch) -> Result<ApiIndex, Error> {
    let mut pbos: Vec<ModPbo> = Vec::new();
    for path in &m.pbos {
        let origin = path.display().to_string();
        match has_sources(path) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                warning(
                    format!("skipping unreadable PBO: {e}"),
                    Some("api-bad-pbo"),
                    (Some(origin), None),
                );
                continue;
            }
        }
        let pbo = match read_pbo(path) {
            Ok(p) => p,
            Err(e) => {
                warning(
                    format!("skipping unreadable PBO: {e}"),
                    Some("api-bad-pbo"),
                    (Some(origin), None),
                );
                continue;
            }
        };
        let Some(prefix) = pbo_prefix(&pbo, path, &origin) else {
            continue;
        };
        let rel = path
            .strip_prefix(&m.dir)
            .unwrap_or(path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let config = config_text(&pbo, &origin);
        let texts = entry_texts(&pbo, script_entries(&pbo, &origin), &origin);
        pbos.push(ModPbo {
            prefix,
            rel,
            config,
            texts,
        });
    }

    let title = meta_name(&m.dir).unwrap_or_else(|| {
        m.dir
            .file_name()
            .map_or_else(|| m.name.clone(), |n| n.to_string_lossy().into_owned())
    });
    let title = match m.workshop {
        Some(id) => format!("{title} (workshop {id})"),
        None => title,
    };
    let mut index = ApiIndex::new(&m.name, &title, branch.name());
    let mut defines: BTreeSet<String> = active_defines(branch);
    let mut files: Vec<ScriptFile<'_>> = Vec::new();
    for (module, p) in pbos.iter().enumerate() {
        index.modules.push(Module {
            prefix: p.prefix.clone(),
            pbo: p.rel.clone(),
        });
        let scripts: Vec<ScriptFile<'_>> = p
            .texts
            .iter()
            .map(|(path, text)| ScriptFile { module, path, text })
            .collect();
        let found = collect_defines(&p.config, &scripts);
        defines.extend(found.iter().cloned());
        if !found.is_empty() {
            index.defines.insert(p.prefix.clone(), found);
        }
        files.extend(scripts);
    }
    index_scripts(&files, &defines, &mut index);
    Ok(index)
}

/// A mod found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstalledMod {
    /// Workshop id, when the mod lives in the workshop content directory.
    pub id: Option<u64>,
    /// The `name` of its `meta.cpp`, else the directory name.
    pub name: String,
    /// The mod directory (symlinks resolved).
    pub dir: PathBuf,
    /// Whether one of its PBOs has a `.c` entry.
    pub has_scripts: bool,
    /// The name of the `@` directory in the game install that links to it, when there is one.
    pub link: Option<String>,
}

/// Whether any `addons/*.pbo` of `dir` lists a `.c` entry. Only the PBO headers are read; an unreadable PBO
/// is a warning.
fn dir_has_scripts(dir: &Path) -> bool {
    let Some(addons) = child_ignore_case(dir, "addons").filter(|a| a.is_dir()) else {
        return false;
    };
    let Ok(rd) = fs::read_dir(addons) else {
        return false;
    };
    let mut pbos: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pbo")))
        .collect();
    pbos.sort();
    for path in pbos {
        match pbo_entry_names(&path) {
            Ok(names) => {
                if names
                    .iter()
                    .any(|n| entry_extension(n).is_some_and(|e| e.eq_ignore_ascii_case("c")))
                {
                    return true;
                }
            }
            Err(e) => warning(
                format!("cannot read the PBO header: {e}"),
                Some("api-bad-pbo"),
                (Some(path.display().to_string()), None),
            ),
        }
    }
    false
}

/// Every mod in the branch's workshop content directory plus every `@*` directory of the game install (the
/// DayZ launcher links mods there), deduplicated by canonical path and sorted by name. Only directory
/// listings, `meta.cpp` and PBO header blocks are read.
#[must_use]
pub fn installed_mods(
    branch: Branch,
    libraries: &[PathBuf],
    game: Option<&Path>,
) -> Vec<InstalledMod> {
    let subdirs = |dir: &Path, only_at: bool| -> Vec<(String, PathBuf)> {
        let Ok(rd) = fs::read_dir(dir) else {
            return Vec::new();
        };
        rd.filter_map(Result::ok)
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .filter(|(n, p)| (!only_at || n.starts_with('@')) && p.is_dir())
            .collect()
    };
    let workshop = workshop_dir(branch, libraries);
    let workshop_canon = workshop.as_deref().and_then(|w| w.canonicalize().ok());
    let mut found: Vec<InstalledMod> = Vec::new();
    let candidates = workshop
        .iter()
        .flat_map(|w| subdirs(w, false))
        .map(|(n, p)| (n, p, false))
        .chain(
            game.iter()
                .flat_map(|g| subdirs(g, true))
                .map(|(n, p)| (n, p, true)),
        );
    for (entry, path, is_link) in candidates {
        let Ok(dir) = path.canonicalize() else {
            continue;
        };
        if let Some(have) = found.iter_mut().find(|m| m.dir == dir) {
            if is_link {
                have.link.get_or_insert(entry);
            }
            continue;
        }
        let id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse::<u64>().ok())
            .filter(|_| dir.parent() == workshop_canon.as_deref());
        let dir_name = dir
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        found.push(InstalledMod {
            id,
            name: meta_name(&dir).unwrap_or_else(|| if is_link { entry.clone() } else { dir_name }),
            has_scripts: dir_has_scripts(&dir),
            link: is_link.then_some(entry),
            dir,
        });
    }
    found.sort_by_cached_key(|m| (m.name.to_lowercase(), m.dir.clone()));
    found
}

/// The installed mod named `token`: an exact (case-insensitive) match of its name or `@dir` name, else a
/// unique substring match.
///
/// # Errors
///
/// Returns an error when nothing matches or the match is not unique; it lists up to 10 candidates.
pub fn match_installed<'a>(
    token: &str,
    installed: &'a [InstalledMod],
) -> Result<&'a InstalledMod, Error> {
    let want = token.to_lowercase();
    let pick = |exact: bool| -> Vec<&'a InstalledMod> {
        installed
            .iter()
            .filter(|m| {
                [Some(m.name.as_str()), m.link.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|n| {
                        let n = n.to_lowercase();
                        if exact { n == want } else { n.contains(&want) }
                    })
            })
            .collect()
    };
    let mut hits = pick(true);
    if hits.is_empty() {
        hits = pick(false);
    }
    let list = |mods: &[&InstalledMod]| {
        let mut names: Vec<String> = mods.iter().take(10).map(|m| m.name.clone()).collect();
        if mods.len() > 10 {
            names.push("...".to_string());
        }
        names.join(", ")
    };
    match hits.as_slice() {
        [one] => Ok(one),
        [] => {
            let all: Vec<&InstalledMod> = installed.iter().collect();
            Err(error!(
                "no installed mod is named \"{token}\" (installed: {}); see `dayzmake api mods`",
                if all.is_empty() {
                    "none".to_string()
                } else {
                    list(&all)
                }
            ))
        }
        many => Err(error!(
            "\"{token}\" matches more than one installed mod: {}",
            list(many)
        )),
    }
}

/// The alias of a workshop id, if it has one.
fn alias_of(id: u64) -> Option<&'static str> {
    ALIASES.iter().find(|(_, i)| *i == id).map(|(n, _)| *n)
}

/// An installed mod as a source: the index name is the alias for the well-known ids, else its sanitized name.
///
/// # Errors
///
/// Returns an error when the directory vanished.
pub fn installed_source(m: &InstalledMod) -> Result<ModSource, Error> {
    let mut source = resolve_mod(&ModSpec::Dir(m.dir.clone()), Branch::Stable, &[])?;
    let name =
        m.id.and_then(alias_of)
            .map_or_else(|| sanitize(&m.name), str::to_string);
    if !name.is_empty() {
        source.name = name;
    }
    source.workshop = m.id;
    Ok(source)
}
