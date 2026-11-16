//! The config index: the classes of every `config.bin` and `config.cpp` of the game and of mods, merged the way
//! the engine merges them.
//!
//! One [`ConfigLayer`](crate::api::config_index::ConfigLayer) is built per source (vanilla, each mod) and cached as JSON; the layers are merged in load
//! order at query time. All input is untrusted: PBO bytes are read through the bounded PBO and config readers,
//! nothing is allocated from a length field, nesting and the number of classes and properties are limited, and no
//! file system path is built from config content.
//!
//! # Merge rules
//!
//! A class defined again extends the earlier definition (its parent is replaced only when the new definition names
//! one); properties are overwritten; `x[] += {..}` appends to the array; `delete X;` removes the class with
//! everything below it; `class X;` (an external declaration) creates nothing. Inside a source the config files are
//! applied in the order of their `CfgPatches.*.requiredAddons` (sorted path order breaks ties); vanilla comes
//! first, then the mods in the order they were selected.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{Cursor, Error};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use super::cache::Cacheable;
use super::model::InputStamp;
use super::source::child_ignore_case;
use dayzmake_core::config::{
    Config, ConfigArray, ConfigArrayElement, ConfigClass, ConfigEntry, is_rapified,
};
use dayzmake_core::error;
use dayzmake_core::error::printable;
use dayzmake_core::pbo::{EntryFilter, PBO};
use dayzmake_core::warning::{Warning, WarningSink, Warnings};

/// Version of the config index layout. Bump it on every change of the model; the cache treats other values as
/// stale.
pub const CONFIG_FORMAT: u32 = 2;

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

impl Cacheable for ConfigLayer {
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
                printable(key),
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
            if let Some(change) = boundary {
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

/// A float as the shortest number that reads back as the same `f32`.
fn float_value(f: f32) -> Value {
    if f.is_finite()
        && let Ok(d) = f.to_string().parse::<f64>()
        && let Some(n) = Number::from_f64(d)
    {
        return Value::Number(n);
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
                    if let Some(change) = change.filter(|_| layer.name != "vanilla") {
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

/// What a config says about load order: the `CfgPatches` classes it defines and the addons they require, in
/// lowercase.
#[derive(Debug, Default)]
struct Patches {
    provides: Vec<String>,
    requires: Vec<String>,
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

/// The configs of a PBO: `config.bin` (rapified) or `config.cpp` (text) of the top level and of every folder
/// below it, `config.bin` winning where a folder has both. The top level comes first, then the folders in path
/// order. A config that cannot be read is an `Err` in its place.
fn read_configs(path: &Path, filter: &EntryFilter) -> Result<Vec<ReadConfig>, Error> {
    let mut file =
        File::open(path).map_err(|e| error!("{}: {e}", printable(&path.display().to_string())))?;
    let selection = PBO::read_entries(&mut file, |n| filter.matches(n))
        .map_err(|e| error!("{}: {e}", printable(&path.display().to_string())))?;
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
    let mut out = Vec::new();
    for (_, (is_bin, i)) in chosen {
        let Some(entry) = selection.entries().get(i) else {
            continue;
        };
        let name = entry.name().to_string();
        let kind = if is_bin { "config.bin" } else { "config.cpp" };
        let parsed = entry
            .data()
            .map_err(|e| error!("cannot read {kind}: {e}"))
            .and_then(|bytes| {
                if is_rapified(bytes) {
                    Config::read_rapified(&mut Cursor::new(bytes))
                } else {
                    // the preprocessor warnings say nothing about the classes, only about how they were written
                    Config::from_string(
                        String::from_utf8_lossy(bytes).into_owned(),
                        None,
                        &[],
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
/// the rest in index order (the PBOs are sorted by path). `None` entries have no config and are left out.
fn load_order(
    infos: &[Option<Patches>],
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
    while let Some(next) = ready.pop_first() {
        order.push(next);
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
            .filter(|(i, info)| info.is_some() && !order.contains(i))
            .map(|(i, _)| i)
            .collect();
        let shown: Vec<String> = stuck
            .iter()
            .take(5)
            .filter_map(|i| names.get(*i))
            .map(|n| printable(n))
            .collect();
        warnings.warn(Warning::new(
            Some("api-config-order"),
            format!(
                "requiredAddons form a cycle between {} configs ({}{}); they load in path order",
                stuck.len(),
                shown.join(", "),
                if stuck.len() > shown.len() {
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

/// Builds the layer of one source from its PBOs, given as (file, name relative to the source, `/`-separated).
/// A PBO that cannot be read is skipped with a warning.
///
/// # Errors
///
/// Never fails today; the `Result` is kept so the cache can call it like the other builders.
pub fn build_layer(
    name: &str,
    build: &str,
    pbos: &[(PathBuf, String)],
    warnings: &mut dyn WarningSink,
) -> Result<ConfigLayer, Error> {
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
    // pass 1: which addons each PBO provides and requires
    let mut infos: Vec<Option<Patches>> = Vec::with_capacity(pbos.len());
    let mut names: Vec<String> = Vec::with_capacity(pbos.len());
    for (path, rel) in pbos {
        names.push(rel.clone());
        infos.push(match read_configs(path, &filter) {
            Ok(configs) if configs.is_empty() => None,
            Ok(configs) => {
                let mut all = Patches::default();
                for (entry, parsed) in configs {
                    match parsed {
                        Ok(config) => {
                            let mut one = patches_of(&config);
                            all.provides.append(&mut one.provides);
                            all.requires.append(&mut one.requires);
                        }
                        Err(e) => bad(warnings, path, &format!("{}: ", printable(&entry)), &e),
                    }
                }
                Some(all)
            }
            Err(e) => {
                bad(warnings, path, "", &e);
                None
            }
        });
    }
    let order = load_order(&infos, &names, warnings);
    // pass 2: apply them in load order, reading each config again so that no parsed config is kept; what could
    // not be read was reported in pass 1
    let mut layer = ConfigLayer::new(name, build);
    'pbos: for i in order {
        let Some((path, rel)) = pbos.get(i) else {
            continue;
        };
        let Ok(configs) = read_configs(path, &filter) else {
            continue;
        };
        let pbo = layer.pbo_index(PboRef {
            source: name.to_string(),
            pbo: rel.clone(),
        });
        let origin = path.display().to_string();
        // `.ok()`: a config that cannot be read was reported in pass 1
        for config in configs.into_iter().filter_map(|(_, c)| c.ok()) {
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
                break 'pbos;
            }
        }
    }
    Ok(layer)
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
