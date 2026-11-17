//! The config index: the classes of every `config.bin` and `config.cpp` of the game and of mods, merged the way
//! the engine merges them.
//!
//! One [`ConfigSource`](crate::api::config_index::ConfigSource) is built per source (vanilla, each mod) and cached
//! as JSON; it holds one [`ConfigLayer`](crate::api::config_index::ConfigLayer) per config PBO. The layers of all
//! selected sources are merged in load order at query time. All input is untrusted: PBO bytes are read through the bounded PBO and config readers,
//! nothing is allocated from a length field, nesting and the number of classes and properties are limited, and no
//! file system path is built from config content.
//!
//! # Merge rules
//!
//! A class defined again extends the earlier definition (its parent is replaced only when the new definition names
//! one); properties are overwritten; `x[] += {..}` appends to the array; `delete X;` removes the class with
//! everything below it; `class X;` (an external declaration) creates nothing. The config PBOs of all selected
//! sources are sorted together by their `CfgPatches.*.requiredAddons`, as the engine does, so a game addon that
//! requires a mod addon loads after it. Ties are broken by the source (vanilla first, then the mods in the order
//! they were selected) and then by the sorted path.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{Cursor, Error};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use super::cache::Cacheable;
use super::config_query::display_path;
use super::model::InputStamp;
use super::source::child_ignore_case;
use dayzmake_core::config::{
    Config, ConfigArray, ConfigArrayElement, ConfigClass, ConfigEntry, is_rapified,
};
use dayzmake_core::error;
use dayzmake_core::error::printable;
use dayzmake_core::pbo::{EntryFilter, PBO, SelectedEntry};
use dayzmake_core::preprocess::{IncludeFolders, MemoryIncludes};
use dayzmake_core::warning::{Warning, WarningSink, Warnings};

/// Version of the config index layout. Bump it on every change of the model; the cache treats other values as
/// stale.
pub const CONFIG_FORMAT: u32 = 3;

/// Most classes one layer holds; the rest of a hostile config is skipped with a warning.
const MAX_CLASSES: usize = 2_000_000;

/// Most properties one layer holds.
const MAX_PROPS: usize = 10_000_000;

/// Deepest class nesting that is walked (the readers of dayzmake-core accept 129 levels).
const MAX_DEPTH: usize = 130;

/// Array nesting kept in a property value; deeper levels are cut so the cache file stays readable.
const MAX_VALUE_DEPTH: u32 = 32;

/// A PBO that defines or modifies classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PboRef {
    /// The index the PBO belongs to: `vanilla` or the mod name.
    pub source: String,
    /// The PBO file relative to its install or mod directory, `/`-separated.
    pub pbo: String,
}

/// A property of a class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prop {
    /// The name as first written.
    pub name: String,
    /// The value: a string, a number or an array of those.
    pub value: Value,
    /// An array that was only extended (`+=`) in this layer, with nothing to extend yet: it is appended to the
    /// array of the earlier layer or of the base class.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub append: bool,
}

/// A class of the merged config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigNode {
    /// The name as first written.
    pub name: String,
    /// The base class name as written, if the class names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The properties by lowercase name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, Prop>,
    /// The PBOs that define or modify the class, as indices into [`ConfigLayer::pbos`], in load order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pbos: Vec<u32>,
    /// `delete X;`: the class is gone, and so is its content in the layers before this one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// The class was deleted and defined again: the content of the layers before this one is dropped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
    /// Redefinitions that replaced the base class, in load order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_changes: Vec<BaseChange>,
}

/// A redefinition that gave a class another base class (or none): the engine takes the base of the last
/// definition, so a mod that writes `class X {..}` without `: Base` cuts X off from its base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseChange {
    /// The base before, `None` for no base.
    pub from: Option<String>,
    /// The base after, `None` for no base.
    pub to: Option<String>,
    /// The PBO of the redefinition, an index into [`ConfigLayer::pbos`].
    pub pbo: u32,
}

/// The classes of one source, or of all selected sources merged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigLayer {
    /// Layout version, [`CONFIG_FORMAT`].
    pub format: u32,
    /// The dayzmake version that built the layer; the cache treats other versions as stale.
    #[serde(default)]
    pub version: String,
    /// `vanilla` or the mod name.
    pub name: String,
    /// The layer is made of the game's own configs: its base class changes are not warned about.
    #[serde(default)]
    pub is_vanilla: bool,
    /// Build or version the layer was made from.
    pub build: String,
    /// Files the layer was built from.
    pub inputs: Vec<InputStamp>,
    /// PBOs, referenced by [`ConfigNode::pbos`], in load order.
    pub pbos: Vec<PboRef>,
    /// The classes by lowercase path (`cfgvehicles/car`). A parent always sorts before its children.
    pub nodes: BTreeMap<String, ConfigNode>,
    /// How many properties the nodes hold, for the size limit.
    #[serde(default)]
    pub prop_count: usize,
}

/// The addons a config PBO provides (`CfgPatches` classes) and requires (`requiredAddons`), in lowercase.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patches {
    /// The `CfgPatches` class names.
    pub provides: Vec<String>,
    /// The `requiredAddons` entries.
    pub requires: Vec<String>,
}

/// The classes of one config PBO.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigPart {
    /// The PBO file relative to its install or mod directory, `/`-separated.
    pub pbo: String,
    /// The addons the PBO provides and requires.
    pub patches: Patches,
    /// The classes of the PBO, with its path as the only entry of the PBO table.
    pub layer: ConfigLayer,
}

/// The config PBOs of one source (the game or a mod), each as a layer of its own so that the PBOs of all sources
/// can be merged in one global order. This is what the cache stores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigSource {
    /// Layout version, [`CONFIG_FORMAT`].
    pub format: u32,
    /// The dayzmake version that built the source; the cache treats other versions as stale.
    #[serde(default)]
    pub version: String,
    /// `vanilla` or the mod name.
    pub name: String,
    /// The source is the game itself.
    #[serde(default)]
    pub is_vanilla: bool,
    /// Build or version the source was made from.
    pub build: String,
    /// Files the source was built from.
    pub inputs: Vec<InputStamp>,
    /// The PBOs that have a config, in sorted path order.
    pub parts: Vec<ConfigPart>,
}

impl ConfigSource {
    /// How many distinct classes the source defines (deletions are not counted).
    #[must_use]
    pub fn class_count(&self) -> usize {
        self.parts
            .iter()
            .flat_map(|p| p.layer.nodes.iter())
            .filter(|(_, n)| !n.deleted)
            .map(|(k, _)| k.as_str())
            .collect::<BTreeSet<&str>>()
            .len()
    }
}

impl Cacheable for ConfigSource {
    fn is_current(&self, want: &[InputStamp]) -> bool {
        self.format == CONFIG_FORMAT
            && self.version == env!("CARGO_PKG_VERSION")
            && self.inputs == want
    }

    fn set_inputs(&mut self, inputs: Vec<InputStamp>) {
        self.inputs = inputs;
    }
}

/// A size limit was hit: no more classes or properties are taken.
#[derive(Debug, Clone, Copy)]
struct LimitHit;

impl ConfigLayer {
    /// An empty layer of the current format.
    #[must_use]
    pub fn new(name: &str, build: &str) -> Self {
        ConfigLayer {
            format: CONFIG_FORMAT,
            version: env!("CARGO_PKG_VERSION").to_string(),
            name: name.to_string(),
            is_vanilla: false,
            build: build.to_string(),
            inputs: Vec::new(),
            pbos: Vec::new(),
            nodes: BTreeMap::new(),
            prop_count: 0,
        }
    }

    /// How many classes the layer holds (deletions are not counted).
    #[must_use]
    pub fn class_count(&self) -> usize {
        self.nodes.values().filter(|n| !n.deleted).count()
    }

    /// The class at `key` (a lowercase path), unless it was deleted.
    #[must_use]
    pub fn live(&self, key: &str) -> Option<&ConfigNode> {
        self.nodes.get(key).filter(|n| !n.deleted)
    }

    /// The index of `pbo` in the PBO table, added when new.
    fn pbo_index(&mut self, pbo: PboRef) -> u32 {
        let at = self.pbos.iter().position(|p| *p == pbo).unwrap_or_else(|| {
            self.pbos.push(pbo);
            self.pbos.len() - 1
        });
        u32::try_from(at).unwrap_or(u32::MAX)
    }

    /// Removes everything below `key`.
    fn remove_below(&mut self, key: &str) {
        // `/` + 1 is `0`: every key below `key` lies in `key/` .. `key0`
        let below: Vec<String> = self
            .nodes
            .range(format!("{key}/")..format!("{key}0"))
            .map(|(k, _)| k.clone())
            .collect();
        for k in below {
            self.drop_node(&k);
        }
    }

    fn drop_node(&mut self, key: &str) {
        if let Some(old) = self.nodes.remove(key) {
            self.prop_count = self.prop_count.saturating_sub(old.props.len());
        }
    }

    /// `delete X;`: the class and everything below it are removed, a tombstone stays.
    fn delete_class(&mut self, key: &str, name: &str) {
        self.remove_below(key);
        self.drop_node(key);
        self.nodes.insert(
            key.to_string(),
            ConfigNode {
                name: name.to_string(),
                parent: None,
                props: BTreeMap::new(),
                pbos: Vec::new(),
                deleted: true,
                reset: false,
                base_changes: Vec::new(),
            },
        );
    }

    /// A definition of the class `key`: creates it, extends it, or revives a deleted one. A definition with a
    /// body sets the base class to what it names, none included (the engine does the same); the returned change
    /// says the base changed, and the change is recorded with `pbo`. An external declaration never gets here.
    fn define_class(
        &mut self,
        key: &str,
        name: &str,
        parent: Option<&str>,
        pbo: u32,
    ) -> Result<(&mut ConfigNode, Option<BaseChange>), LimitHit> {
        let fresh = |reset: bool| ConfigNode {
            name: name.to_string(),
            parent: parent.map(str::to_string),
            props: BTreeMap::new(),
            pbos: Vec::new(),
            deleted: false,
            reset,
            base_changes: Vec::new(),
        };
        let existed = self.nodes.contains_key(key);
        if !existed && self.nodes.len() >= MAX_CLASSES {
            return Err(LimitHit);
        }
        let node = self
            .nodes
            .entry(key.to_string())
            .or_insert_with(|| fresh(false));
        let mut changed = None;
        if node.deleted {
            *node = fresh(true);
        } else if existed {
            let same = match (&node.parent, parent) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            };
            if !same {
                let change = BaseChange {
                    from: node.parent.take(),
                    to: parent.map(str::to_string),
                    pbo,
                };
                node.base_changes.push(change.clone());
                node.parent = parent.map(str::to_string);
                changed = Some(change);
            }
        }
        Ok((node, changed))
    }

    /// The `api-config-base-changed` warning for `change` of the class `key`.
    fn base_changed_warning(&self, key: &str, change: &BaseChange) -> Warning {
        let shown = |b: Option<&String>| b.map_or_else(|| "(none)".to_string(), |b| printable(b));
        let file = self
            .pbos
            .get(usize::try_from(change.pbo).unwrap_or(usize::MAX))
            .map(|p| format!("{}: {}", p.source, p.pbo));
        let warning = Warning::new(
            Some("api-config-base-changed"),
            format!(
                "class {} changes its base class: {} -> {}",
                printable(&display_path(self, key)),
                shown(change.from.as_ref()),
                shown(change.to.as_ref())
            ),
        );
        match file {
            Some(f) => warning.in_file(f),
            None => warning,
        }
    }

    /// Sets the property `name` of the class `key`; `append` extends an existing array (`+=`).
    fn set_prop(
        &mut self,
        key: &str,
        name: &str,
        value: Value,
        append: bool,
    ) -> Result<(), LimitHit> {
        let lower = name.to_ascii_lowercase();
        let mut added = false;
        let Some(node) = self.nodes.get_mut(key) else {
            return Ok(());
        };
        if node.deleted {
            return Ok(());
        }
        if let Some(prop) = node.props.get_mut(&lower) {
            if append && let (Value::Array(have), Value::Array(more)) = (&mut prop.value, &value) {
                have.extend(more.iter().cloned());
            } else {
                prop.value = value;
                prop.append = false;
            }
        } else {
            if self.prop_count >= MAX_PROPS {
                return Err(LimitHit);
            }
            node.props.insert(
                lower,
                Prop {
                    name: name.to_string(),
                    value,
                    append,
                },
            );
            added = true;
        }
        if added {
            self.prop_count += 1;
        }
        Ok(())
    }

    /// Merges `over`, loaded after this layer, into it: the engine's rules, see the module documentation.
    ///
    /// The PBOs of `over` are added to the PBO table. A layer that reaches a size limit stops taking classes
    /// and the function returns `false`.
    pub fn merge(&mut self, over: ConfigLayer, warnings: &mut dyn WarningSink) -> bool {
        let over_is_vanilla = over.is_vanilla;
        let map: Vec<u32> = over
            .pbos
            .iter()
            .map(|p| self.pbo_index(p.clone()))
            .collect();
        let mapped = |p: u32| map.get(usize::try_from(p).unwrap_or(usize::MAX)).copied();
        for (key, node) in over.nodes {
            if node.deleted {
                self.delete_class(&key, &node.name);
                continue;
            }
            if node.reset {
                self.delete_class(&key, &node.name);
            }
            // The layer boundary: the base the mod's first definition names (the base before its own first
            // change, else its final base) against the base so far, at the PBO of that first definition.
            let first_base = node
                .base_changes
                .first()
                .map_or(node.parent.as_deref(), |c| c.from.as_deref());
            let first_pbo = node.pbos.first().and_then(|p| mapped(*p)).unwrap_or(0);
            let Ok((mine, boundary)) = self.define_class(&key, &node.name, first_base, first_pbo)
            else {
                return false;
            };
            mine.reset |= node.reset;
            for p in node.pbos {
                if let Some(at) = mapped(p)
                    && !mine.pbos.contains(&at)
                {
                    mine.pbos.push(at);
                }
            }
            // the changes inside `over` follow unchanged, and the class ends with the base `over` ends with
            for c in node.base_changes {
                if let Some(at) = mapped(c.pbo) {
                    mine.base_changes.push(BaseChange { pbo: at, ..c });
                }
            }
            mine.parent = node.parent;
            // the game's own layout changes bases on purpose; only a mod's change is worth a warning
            if let Some(change) = boundary.filter(|_| !over_is_vanilla) {
                let warning = self.base_changed_warning(&key, &change);
                warnings.warn(warning);
            }
            for (_, prop) in node.props {
                if self
                    .set_prop(&key, &prop.name, prop.value, prop.append)
                    .is_err()
                {
                    return false;
                }
            }
        }
        true
    }
}

/// The key of the child `name` of the class at `scope` (`""` for the top level), or `None` for a name that
/// cannot be a path segment.
fn child_key(scope: &str, name: &str) -> Option<String> {
    if name.is_empty() || name.contains('/') || name.chars().any(char::is_control) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    Some(if scope.is_empty() {
        lower
    } else {
        format!("{scope}/{lower}")
    })
}

/// The key of the object that holds an infinite float in [`Prop::value`].
pub const NONFINITE: &str = "nonfinite";

/// A float as the shortest number that reads back as the same `f32`.
fn float_value(f: f32) -> Value {
    if f.is_finite()
        && let Ok(d) = f.to_string().parse::<f64>()
        && let Some(n) = Number::from_f64(d)
    {
        return Value::Number(n);
    }
    // An infinity is kept apart from a string that reads the same: `{"nonfinite": "1e999"}` (derapify's text).
    // The query shows it as `1e999` in the text output and as the string `"1e999"` in JSON. NaN has no literal and
    // stays the string `NaN`.
    if f.is_infinite() {
        let text = if f.is_sign_positive() {
            "1e999"
        } else {
            "-1e999"
        };
        return serde_json::json!({ NONFINITE: text });
    }
    Value::String(f.to_string())
}

fn array_value(array: &ConfigArray, depth: u32) -> Value {
    if depth >= MAX_VALUE_DEPTH {
        return Value::String("{...}".to_string());
    }
    Value::Array(
        array
            .elements()
            .iter()
            .map(|e| match e {
                ConfigArrayElement::StringElement(s) => Value::String(s.clone()),
                ConfigArrayElement::FloatElement(f) => float_value(*f),
                ConfigArrayElement::IntElement(i) => Value::from(*i),
                ConfigArrayElement::ArrayElement(a) => array_value(a, depth + 1),
                _ => Value::Null,
            })
            .collect(),
    )
}

/// Applies the entries of `class` (the class at `scope`) to `layer`, in file order.
fn apply_class(
    layer: &mut ConfigLayer,
    scope: &str,
    class: &ConfigClass,
    pbo: u32,
    depth: usize,
    warnings: &mut dyn WarningSink,
    origin: &str,
) -> Result<(), LimitHit> {
    for (name, entry) in class.entries() {
        match entry {
            ConfigEntry::ClassEntry(c) => {
                let Some(key) = child_key(scope, name) else {
                    warnings.warn(
                        Warning::new(
                            Some("api-bad-config"),
                            format!(
                                "skipping class with the unusable name \"{}\"",
                                printable(name)
                            ),
                        )
                        .in_file(origin.to_string()),
                    );
                    continue;
                };
                if c.is_deletion() {
                    layer.delete_class(&key, name);
                } else if c.is_external() {
                    // a forward declaration: it only tells the parser where the base class is
                } else if depth < MAX_DEPTH {
                    let (node, change) = layer.define_class(&key, name, c.parent(), pbo)?;
                    if !node.pbos.contains(&pbo) {
                        node.pbos.push(pbo);
                    }
                    // the game's own layout changes bases on purpose; only a mod's change is worth a warning
                    if let Some(change) = change.filter(|_| !layer.is_vanilla) {
                        warnings.warn(
                            layer
                                .base_changed_warning(&key, &change)
                                .in_file(origin.to_string()),
                        );
                    }
                    apply_class(layer, &key, c, pbo, depth + 1, warnings, origin)?;
                }
            }
            // properties of the top level belong to no class
            _ if scope.is_empty() => {}
            ConfigEntry::ArrayEntry(a) => {
                layer.set_prop(scope, name, array_value(a, 0), a.is_expansion())?;
            }
            ConfigEntry::StringEntry(s) => {
                layer.set_prop(scope, name, Value::String(s.clone()), false)?;
            }
            ConfigEntry::FloatEntry(f) => {
                layer.set_prop(scope, name, float_value(*f), false)?;
            }
            ConfigEntry::IntEntry(i) => {
                layer.set_prop(scope, name, Value::from(*i), false)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn patches_of(config: &Config) -> Patches {
    let mut out = Patches::default();
    let Some(ConfigEntry::ClassEntry(patches)) = config.root().get("CfgPatches") else {
        return out;
    };
    for (name, entry) in patches.entries() {
        let ConfigEntry::ClassEntry(patch) = entry else {
            continue;
        };
        out.provides.push(name.to_ascii_lowercase());
        if let Some(ConfigEntry::ArrayEntry(required)) = patch.get("requiredAddons") {
            for element in required.elements() {
                if let ConfigArrayElement::StringElement(s) = element {
                    out.requires.push(s.to_ascii_lowercase());
                }
            }
        }
    }
    out
}

/// A config entry of a PBO and what reading it gave.
type ReadConfig = (String, Result<Config, Error>);

/// A chosen config: entry name, whether it is a `config.bin`, and its bytes (none for a text config, whose bytes
/// live in the include map).
type ChosenConfig = (String, bool, Result<Option<Vec<u8>>, Error>);

/// Whether a PBO entry can be the target of an `#include` of a config (a header or text file).
fn is_includable(name: &str) -> bool {
    let lower = name.to_lowercase();
    ["hpp", "h", "inc", "hh", "hxx", "cpp", "ext", "txt", "cfg"]
        .iter()
        .any(|e| lower.rsplit_once('.').is_some_and(|(_, x)| x == *e))
}

/// The include source of a PBO with a text config: the text configs in `files` and the other includable
/// entries, read tolerantly (a damaged one and the ones past `cap` bytes in all are skipped with a warning).
fn read_includes(
    path: &Path,
    file: &mut File,
    filter: &EntryFilter,
    prefix: &str,
    mut files: Vec<(String, Vec<u8>)>,
    cap: u64,
    warnings: &mut dyn WarningSink,
) -> Result<IncludeFolders, Error> {
    let used: u64 = files.iter().map(|(_, d)| d.len() as u64).sum();
    let rest = PBO::read_entries_tolerant(
        file,
        |n| is_includable(n) && !filter.matches(n),
        cap.saturating_sub(used),
    )
    .map_err(|e| error!("{}: {e}", printable(&path.display().to_string())))?;
    let mut unreadable: Vec<(String, String)> = Vec::new();
    let mut capped = false;
    for entry in rest.into_entries() {
        let name = entry.name().to_string();
        let over = entry.is_over_limit();
        match entry.into_data() {
            Ok(data) => files.push((name, data)),
            Err(e) if over => {
                capped = true;
                unreadable.push((name, e.to_string()));
            }
            Err(e) => {
                warnings.warn(
                    Warning::new(
                        Some("api-config-include-unreadable"),
                        format!(
                            "cannot read the includable {}: {e}; it is skipped",
                            printable(&name)
                        ),
                    )
                    .in_file(path.display().to_string()),
                );
                unreadable.push((name, e.to_string()));
            }
        }
    }
    if capped {
        warnings.warn(
            Warning::new(
                Some("api-config-includes-capped"),
                format!("the includable files of this PBO are more than {cap} bytes; the rest is skipped"),
            )
            .in_file(path.display().to_string()),
        );
    }
    Ok(IncludeFolders::new(Vec::new())
        .with_memory(MemoryIncludes::new(prefix, files).with_unreadable(unreadable)))
}

/// Most bytes of includable entries read from one PBO; the rest are skipped with a warning.
const MAX_INCLUDE_BYTES: u64 = 64 << 20;

/// The configs of a PBO: `config.bin` (rapified) or `config.cpp` (text) of the top level and of every folder
/// below it, `config.bin` winning where a folder has both. The top level comes first, then the folders in path
/// order. A config that cannot be read is an `Err` in its place. The text files the configs may include are
/// read only when a text config is chosen, and a damaged one is skipped with a warning.
fn read_configs(
    path: &Path,
    filter: &EntryFilter,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<ReadConfig>, Error> {
    read_configs_capped(path, filter, MAX_INCLUDE_BYTES, warnings)
}

/// [`read_configs`] with the byte cap of the includable entries as a parameter.
fn read_configs_capped(
    path: &Path,
    filter: &EntryFilter,
    include_cap: u64,
    warnings: &mut dyn WarningSink,
) -> Result<Vec<ReadConfig>, Error> {
    let shown = printable(&path.display().to_string());
    let mut file = File::open(path).map_err(|e| error!("{shown}: {e}"))?;
    let selection =
        PBO::read_entries(&mut file, |n| filter.matches(n)).map_err(|e| error!("{shown}: {e}"))?;
    // folder (lowercase, `/`-separated, with a trailing `/`) -> (is config.bin, entry index)
    let mut chosen: BTreeMap<String, (bool, usize)> = BTreeMap::new();
    for (i, entry) in selection.entries().iter().enumerate() {
        let name = entry.name().replace('\\', "/").to_lowercase();
        let (folder, file_name) = name
            .rsplit_once('/')
            .map_or(("", name.as_str()), |(f, n)| (f, n));
        let is_bin = file_name == "config.bin";
        let slot = chosen.entry(folder.to_string()).or_insert((is_bin, i));
        if is_bin && !slot.0 {
            *slot = (true, i);
        }
    }
    let prefix = selection
        .header_extension("prefix")
        .unwrap_or("")
        .to_string();
    let mut slots: Vec<Option<SelectedEntry>> =
        selection.into_entries().into_iter().map(Some).collect();
    let has_text = chosen.values().any(|(is_bin, _)| !is_bin);
    // what each chosen config gives: its bytes (a `config.bin`), or nothing for a text config, whose bytes were
    // moved into the include map together with the other text entries
    let mut todo: Vec<ChosenConfig> = Vec::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (_, (is_bin, i)) in chosen {
        let Some(entry) = slots.get_mut(i).and_then(Option::take) else {
            continue;
        };
        let name = entry.name().to_string();
        let data = if is_bin || !has_text {
            entry.into_data().map(Some)
        } else {
            entry.into_data().map(|bytes| {
                files.push((name.clone(), bytes));
                None
            })
        };
        todo.push((name, is_bin, data));
    }
    if has_text {
        // a config.cpp that lost to a config.bin of its folder can still be included
        for entry in slots.into_iter().flatten() {
            let name = entry.name().to_string();
            if is_includable(&name)
                && let Ok(bytes) = entry.into_data()
            {
                files.push((name, bytes));
            }
        }
    }
    let folders = if has_text {
        read_includes(
            path,
            &mut file,
            filter,
            &prefix,
            files,
            include_cap,
            warnings,
        )?
    } else {
        IncludeFolders::new(Vec::new())
    };
    let mut out = Vec::new();
    for (name, is_bin, data) in todo {
        let kind = if is_bin { "config.bin" } else { "config.cpp" };
        let parsed = data
            .map_err(|e| error!("cannot read {kind}: {e}"))
            .and_then(|own| {
                let bytes = own
                    .as_deref()
                    .or_else(|| folders.memory.as_ref().and_then(|m| m.get(&name)))
                    .ok_or_else(|| error!("cannot read {kind}: the entry is missing"))?;
                if is_rapified(bytes) {
                    Config::read_rapified(&mut Cursor::new(bytes))
                } else {
                    // the preprocessor warnings say nothing about the classes, only about how they were written
                    Config::from_string_in(
                        String::from_utf8_lossy(bytes).into_owned(),
                        Some(PathBuf::from(&name)),
                        &folders,
                        &mut Warnings::new(),
                    )
                }
                .map_err(|e| error!("cannot read {kind}: {e}"))
            });
        out.push((name, parsed));
    }
    Ok(out)
}

/// The order to load `infos` in: every config after the configs that provide the addons it requires, ties and
/// the rest in index order (the sources in selection order, each with its PBOs sorted by path). `None` entries
/// have no config and are left out. Kahn's algorithm with the lowest ready index first, so the result does not
/// depend on anything but the input; a cycle is reported and its members are appended in index order.
fn load_order(
    infos: &[Option<&Patches>],
    names: &[String],
    warnings: &mut dyn WarningSink,
) -> Vec<usize> {
    let mut provider: HashMap<&str, usize> = HashMap::new();
    for (i, info) in infos.iter().enumerate() {
        for p in info.iter().flat_map(|i| &i.provides) {
            provider.entry(p.as_str()).or_insert(i);
        }
    }
    let mut waiting = vec![0usize; infos.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); infos.len()];
    for (i, info) in infos.iter().enumerate() {
        let Some(info) = info else { continue };
        let needs: BTreeSet<usize> = info
            .requires
            .iter()
            .filter_map(|r| provider.get(r.as_str()).copied())
            .filter(|p| *p != i)
            .collect();
        for p in needs {
            if let Some(slot) = waiting.get_mut(i) {
                *slot += 1;
            }
            if let Some(d) = dependents.get_mut(p) {
                d.push(i);
            }
        }
    }
    let mut ready: BTreeSet<usize> = infos
        .iter()
        .enumerate()
        .filter(|(i, info)| info.is_some() && waiting.get(*i) == Some(&0))
        .map(|(i, _)| i)
        .collect();
    let total = infos.iter().flatten().count();
    let mut order = Vec::with_capacity(total);
    let mut done = vec![false; infos.len()];
    while let Some(next) = ready.pop_first() {
        order.push(next);
        if let Some(d) = done.get_mut(next) {
            *d = true;
        }
        for d in dependents.get(next).into_iter().flatten() {
            if let Some(slot) = waiting.get_mut(*d) {
                *slot -= 1;
                if *slot == 0 {
                    ready.insert(*d);
                }
            }
        }
    }
    if order.len() < total {
        let stuck: Vec<usize> = infos
            .iter()
            .enumerate()
            .filter(|(i, info)| info.is_some() && done.get(*i) == Some(&false))
            .map(|(i, _)| i)
            .collect();
        // Only the configs that really are in a cycle are named; the ones that merely depend on it are not.
        let in_cycle = cycle_members(&stuck, &dependents);
        let shown: Vec<String> = in_cycle
            .iter()
            .take(5)
            .filter_map(|i| names.get(*i))
            .map(|n| printable(n))
            .collect();
        warnings.warn(Warning::new(
            Some("api-config-order"),
            format!(
                "requiredAddons form a cycle between {} configs ({}{}); they and the configs that depend on them load last, in source and path order",
                in_cycle.len(),
                shown.join(", "),
                if in_cycle.len() > shown.len() {
                    ", ..."
                } else {
                    ""
                }
            ),
        ));
        order.extend(stuck);
    }
    order
}

/// The members of `nodes` that lie on a cycle of `edges` (`edges[a]` lists the nodes that follow `a`): the
/// nodes of strongly connected components of more than one node (a node alone is no cycle: a config does not
/// wait for itself). Tarjan's algorithm without recursion, in index order.
fn cycle_members(nodes: &[usize], edges: &[Vec<usize>]) -> Vec<usize> {
    let in_set: BTreeSet<usize> = nodes.iter().copied().collect();
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut low: HashMap<usize, usize> = HashMap::new();
    let mut on_stack: BTreeSet<usize> = BTreeSet::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut out: BTreeSet<usize> = BTreeSet::new();
    for &root in nodes {
        if index.contains_key(&root) {
            continue;
        }
        // (node, position of the next edge to look at)
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index.insert(root, next);
        low.insert(root, next);
        next += 1;
        stack.push(root);
        on_stack.insert(root);
        while let Some(&(v, at)) = work.last() {
            let succ = edges
                .get(v)
                .and_then(|e| e.get(at))
                .copied()
                .filter(|w| in_set.contains(w));
            let has_edge = edges.get(v).is_some_and(|e| at < e.len());
            if let Some(top) = work.last_mut() {
                top.1 += 1;
            }
            if !has_edge {
                work.pop();
                let lv = low.get(&v).copied().unwrap_or(0);
                if Some(lv) == index.get(&v).copied() {
                    let mut component = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack.remove(&w);
                        component.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if component.len() > 1 {
                        out.extend(component);
                    }
                }
                if let Some(&(parent, _)) = work.last() {
                    let lp = low.get(&parent).copied().unwrap_or(0);
                    low.insert(parent, lp.min(lv));
                }
                continue;
            }
            let Some(w) = succ else { continue };
            if !index.contains_key(&w) {
                index.insert(w, next);
                low.insert(w, next);
                next += 1;
                stack.push(w);
                on_stack.insert(w);
                work.push((w, 0));
            } else if on_stack.contains(&w) {
                let lv = low.get(&v).copied().unwrap_or(0);
                let iw = index.get(&w).copied().unwrap_or(0);
                low.insert(v, lv.min(iw));
            }
        }
    }
    out.into_iter().collect()
}

/// Builds the config source from its PBOs, given as (file, name relative to the source, `/`-separated), sorted
/// by path. A PBO that cannot be read is skipped with a warning, and so is one without a config. Each PBO becomes
/// a layer of its own; the load order is decided across all sources by [`merge_sources`].
///
/// # Errors
///
/// Never fails today; the `Result` is kept so the cache can call it like the other builders.
pub fn build_source(
    name: &str,
    build: &str,
    is_vanilla: bool,
    pbos: &[(PathBuf, String)],
    warnings: &mut dyn WarningSink,
) -> Result<ConfigSource, Error> {
    let filter = EntryFilter::new(
        ["config.bin", "config.cpp", "*/config.bin", "*/config.cpp"]
            .map(str::to_string)
            .to_vec(),
        Vec::new(),
    );
    let bad = |warnings: &mut dyn WarningSink, path: &Path, what: &str, e: &Error| {
        warnings.warn(
            Warning::new(Some("api-bad-config"), format!("{what}{e}"))
                .in_file(path.display().to_string()),
        );
    };
    let mut parts: Vec<ConfigPart> = Vec::new();
    'pbos: for (path, rel) in pbos {
        let configs = match read_configs(path, &filter, warnings) {
            Ok(configs) if configs.is_empty() => continue,
            Ok(configs) => configs,
            Err(e) => {
                bad(warnings, path, "", &e);
                continue;
            }
        };
        let mut layer = ConfigLayer::new(name, build);
        layer.is_vanilla = is_vanilla;
        let pbo = layer.pbo_index(PboRef {
            source: name.to_string(),
            pbo: rel.clone(),
        });
        let origin = path.display().to_string();
        let mut patches = Patches::default();
        for (entry, parsed) in configs {
            let config = match parsed {
                Ok(config) => config,
                Err(e) => {
                    bad(warnings, path, &format!("{}: ", printable(&entry)), &e);
                    continue;
                }
            };
            let mut one = patches_of(&config);
            patches.provides.append(&mut one.provides);
            patches.requires.append(&mut one.requires);
            if apply_class(&mut layer, "", config.root(), pbo, 0, warnings, &origin).is_err() {
                warnings.warn(
                    Warning::new(
                        Some("api-config-limit"),
                        format!(
                            "the config of {name} is too large (more than {MAX_CLASSES} classes or {MAX_PROPS} properties); the rest is skipped"
                        ),
                    )
                    .in_file(origin),
                );
                parts.push(ConfigPart {
                    pbo: rel.clone(),
                    patches,
                    layer,
                });
                break 'pbos;
            }
        }
        parts.push(ConfigPart {
            pbo: rel.clone(),
            patches,
            layer,
        });
    }
    Ok(ConfigSource {
        format: CONFIG_FORMAT,
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: name.to_string(),
        is_vanilla,
        build: build.to_string(),
        inputs: Vec::new(),
        parts,
    })
}

/// Merges the config PBOs of all `sources` into one layer. The PBOs of all sources are sorted together by
/// `requiredAddons` (Kahn's algorithm, lowest index first); the order of `sources` and the order of the PBOs inside a source break
/// ties. A cycle is a warning (`api-config-order`). A merge that reaches a size limit stops taking classes
/// (`api-config-limit`).
#[must_use]
pub fn merge_sources(
    sources: Vec<ConfigSource>,
    build: &str,
    warnings: &mut dyn WarningSink,
) -> ConfigLayer {
    let mut flat: Vec<(String, Patches, Option<ConfigLayer>)> = Vec::new();
    for source in sources {
        for part in source.parts {
            flat.push((source.name.clone(), part.patches, Some(part.layer)));
        }
    }
    merge_flat(flat, build, warnings, ConfigLayer::merge)
}

/// The body of [`merge_sources`] with the layer merge as a parameter, so that a test can make it hit the limit
/// without a layer of millions of classes. `flat` is (source name, patches, layer) per PBO.
fn merge_flat(
    mut flat: Vec<(String, Patches, Option<ConfigLayer>)>,
    build: &str,
    warnings: &mut dyn WarningSink,
    mut merge: impl FnMut(&mut ConfigLayer, ConfigLayer, &mut dyn WarningSink) -> bool,
) -> ConfigLayer {
    let infos: Vec<Option<&Patches>> = flat.iter().map(|(_, p, _)| Some(p)).collect();
    let names: Vec<String> = flat
        .iter()
        .map(|(source, _, l)| {
            let pbo = l
                .as_ref()
                .and_then(|l| l.pbos.first())
                .map_or("", |p| p.pbo.as_str());
            format!("[{source}] {pbo}")
        })
        .collect();
    let order = load_order(&infos, &names, warnings);
    let mut merged = ConfigLayer::new("merged", build);
    for i in order {
        let Some((_, _, layer)) = flat.get_mut(i) else {
            continue;
        };
        let Some(layer) = layer.take() else { continue };
        if !merge(&mut merged, layer, warnings) {
            // every later layer would hit the same limit: one warning, naming where the merge stopped
            warnings.warn(Warning::new(
                Some("api-config-limit"),
                format!(
                    "the merged config is too large; it stops at {} and the PBOs after it are skipped",
                    printable(names.get(i).map_or("", String::as_str))
                ),
            ));
            break;
        }
    }
    merged
}

/// The PBOs of the game that can hold a config: every `*.pbo` of `dta` and `addons` (any case), as (file, name
/// relative to the game directory), sorted by name.
///
/// # Errors
///
/// Returns an error when there is neither a `dta` nor an `addons` folder with PBOs.
pub fn vanilla_config_pbos(game: &Path) -> Result<Vec<(PathBuf, String)>, Error> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for folder in ["dta", "addons"] {
        let Some(dir) = child_ignore_case(game, folder).filter(|d| d.is_dir()) else {
            continue;
        };
        let dir_name = dir
            .file_name()
            .map_or_else(|| folder.to_string(), |n| n.to_string_lossy().into_owned());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("pbo"))
            {
                let rel = format!("{dir_name}/{}", entry.file_name().to_string_lossy());
                out.push((path, rel));
            }
        }
    }
    if out.is_empty() {
        return Err(error!(
            "no PBOs found in the dta or addons folder of \"{}\"",
            printable(&game.display().to_string())
        ));
    }
    out.sort_by(|a, b| {
        a.1.to_lowercase()
            .cmp(&b.1.to_lowercase())
            .then(a.1.cmp(&b.1))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PBO file with `config.cpp` including `a.txt` and `b.txt` (60 bytes each).
    fn capped_pbo(dir: &Path) -> (PathBuf, u64) {
        let config = "class CfgPatches { class P { requiredAddons[] = {}; }; };\nclass X {\n#include \"a.txt\"\n#include \"b.txt\"\n};\n";
        let mut pbo = PBO::new();
        pbo.insert_entry("config.cpp", config.as_bytes().to_vec())
            .unwrap();
        pbo.insert_entry("a.txt", format!("a = 1; {}\n", " ".repeat(50)).into_bytes())
            .unwrap();
        pbo.insert_entry("b.txt", format!("b = 2; {}\n", " ".repeat(50)).into_bytes())
            .unwrap();
        let path = dir.join("c.pbo");
        let mut bytes = Vec::new();
        pbo.write(&mut bytes).unwrap();
        std::fs::write(&path, bytes).unwrap();
        (path, config.len() as u64)
    }

    fn filter() -> EntryFilter {
        EntryFilter::new(
            ["config.bin", "config.cpp"].map(str::to_string).to_vec(),
            Vec::new(),
        )
    }

    #[test]
    fn includables_past_the_cap_are_skipped_with_one_warning() {
        let dir = tempfile::tempdir().unwrap();
        let (path, config_len) = capped_pbo(dir.path());
        let mut warnings = Warnings::new();
        let read = read_configs_capped(&path, &filter(), config_len + 70, &mut warnings).unwrap();
        let names: Vec<_> = warnings
            .warnings()
            .iter()
            .map(|w| w.message.clone())
            .collect();
        assert_eq!(read.len(), 1);
        // b.txt does not fit: its include fails with the cap as the reason, and a.txt was read
        let err = read[0].1.as_ref().unwrap_err().to_string();
        assert!(
            err.contains("b.txt") && err.contains("cannot be read"),
            "{err}"
        );
        assert!(!err.contains("a.txt"), "{err}");
        let capped = warnings
            .warnings()
            .iter()
            .filter(|w| w.name == Some("api-config-includes-capped"))
            .count();
        assert_eq!(capped, 1, "{names:?}");
    }

    #[test]
    fn includables_within_the_cap_are_all_read() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = capped_pbo(dir.path());
        let mut warnings = Warnings::new();
        let read = read_configs(&path, &filter(), &mut warnings).unwrap();
        assert!(read[0].1.is_ok());
        assert_eq!(warnings.warnings(), []);
    }

    /// A PBO file with only the given `config.cpp`.
    fn config_pbo(dir: &Path, file: &str, config: &str) -> (PathBuf, String) {
        let mut pbo = PBO::new();
        pbo.insert_entry("config.cpp", config.as_bytes().to_vec())
            .unwrap();
        let path = dir.join(file);
        let mut bytes = Vec::new();
        pbo.write(&mut bytes).unwrap();
        std::fs::write(&path, bytes).unwrap();
        (path, file.to_string())
    }

    fn patches(provides: &str, requires: &[&str]) -> Patches {
        Patches {
            provides: vec![provides.to_string()],
            requires: requires.iter().map(|r| (*r).to_string()).collect(),
        }
    }

    fn part(source: &str, pbo: &str, patches: Patches) -> (String, Patches, Option<ConfigLayer>) {
        let mut layer = ConfigLayer::new(source, "b");
        layer.pbo_index(PboRef {
            source: source.to_string(),
            pbo: pbo.to_string(),
        });
        (source.to_string(), patches, Some(layer))
    }

    /// The warning names the PBOs of the cycle with their source, and not the PBOs that merely require one.
    #[test]
    fn cycle_warning_counts_only_the_cycle() {
        let flat = vec![
            part("m", "a.pbo", patches("a", &["b"])),
            part("m", "b.pbo", patches("b", &["a"])),
            part("m", "c.pbo", patches("c", &["a"])),
        ];
        let mut warnings = Warnings::new();
        let _ = merge_flat(flat, "b", &mut warnings, ConfigLayer::merge);
        let found: Vec<_> = warnings
            .warnings()
            .iter()
            .filter(|w| w.name == Some("api-config-order"))
            .collect();
        assert_eq!(found.len(), 1);
        let message = &found[0].message;
        assert!(message.contains("between 2 configs"), "{message}");
        assert!(
            message.contains("[m] a.pbo") && message.contains("[m] b.pbo"),
            "{message}"
        );
        assert!(!message.contains("c.pbo"), "{message}");
    }

    /// Once the merge hits its limit the rest is skipped with one warning that names where it stopped.
    #[test]
    fn merge_limit_warns_once_and_names_the_pbo() {
        let flat = vec![
            part("one", "a.pbo", patches("a", &[])),
            part("two", "b.pbo", patches("b", &[])),
            part("two", "c.pbo", patches("c", &[])),
            part("two", "d.pbo", patches("d", &[])),
        ];
        let mut warnings = Warnings::new();
        let mut calls = 0;
        let _ = merge_flat(flat, "b", &mut warnings, |_, _, _| {
            calls += 1;
            calls < 2
        });
        assert_eq!(calls, 2, "the merge stops at the first failure");
        let found: Vec<_> = warnings
            .warnings()
            .iter()
            .filter(|w| w.name == Some("api-config-limit"))
            .collect();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("[two] b.pbo"), "{found:?}");
    }

    /// A mod that happens to be called `vanilla` is not the game: its base class change is warned about.
    #[test]
    fn a_source_named_vanilla_is_not_filtered_like_the_game() {
        let dir = tempfile::tempdir().unwrap();
        let base = config_pbo(
            dir.path(),
            "base.pbo",
            "class CfgPatches { class P1 { requiredAddons[] = {}; }; };\nclass CfgVehicles { class Base {}; class X: Base {}; };",
        );
        let named = config_pbo(
            dir.path(),
            "named.pbo",
            "class CfgPatches { class P2 { requiredAddons[] = {\"P1\"}; }; };\nclass CfgVehicles { class X {}; };",
        );
        let mut warnings = Warnings::new();
        let sources = vec![
            build_source("base", "b", false, &[base], &mut warnings).unwrap(),
            build_source("vanilla", "b", false, &[named], &mut warnings).unwrap(),
        ];
        assert!(!sources[1].is_vanilla);
        let _ = merge_sources(sources, "b", &mut warnings);
        assert!(
            warnings
                .warnings()
                .iter()
                .any(|w| w.name == Some("api-config-base-changed")),
            "{:?}",
            warnings.warnings()
        );
    }
}
