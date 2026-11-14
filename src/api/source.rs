//! Script sources of the installed game and mods: the PBOs, mod selection and index building.
//!
//! PBO bytes, prefixes and entry names are untrusted. Text is read lossily, entries and prefixes that fail
//! [`safe_relative_path`] are skipped with a warning, and no file system path is ever built from PBO content.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Cursor, Error, Read};
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
}

/// Parse a comma separated mod list: an alias (`cf`, `cot`, `dabs`, `expansion`), `all` (the four aliases),
/// a workshop id (digits) or a directory. Duplicates are dropped.
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
        } else {
            add(ModSpec::Dir(PathBuf::from(token)));
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
