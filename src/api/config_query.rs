//! Lookup of one config class in the merged config index: inheritance chain, resolved properties, subclasses and
//! the PBOs that define it, with their text and JSON renderings.
//!
//! # Inheritance lookup
//!
//! For `class B: A` the base `A` is searched in the scope that holds `B` first: among the classes of that scope
//! (never `B` itself), then among the classes the scope inherits from. When that fails the search moves out one
//! scope at a time, up to the top level. So `class Inner: Inner` inside a derived class finds the `Inner` of its
//! base class. A base that is only found as the class itself, or that leads back to a class already on the
//! chain, is an inheritance cycle. The lookup is bounded in depth and in chain length.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::Error;

use serde::Serialize;
use serde_json::Value;

use super::config_index::{ConfigLayer, ConfigNode, NONFINITE, PboRef};
use dayzmake_core::error;
use dayzmake_core::error::printable;

/// Deepest nesting of base class lookups.
const MAX_RESOLVE_DEPTH: u32 = 64;

/// Longest inheritance chain that is followed.
const MAX_CHAIN: usize = 256;

/// Longest value shown in the text output.
const MAX_TEXT_VALUE: usize = 300;

/// Most suggestions of a failed lookup.
const MAX_SUGGESTIONS: usize = 5;

/// Where the base class of a class is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Base {
    /// The class has no base.
    Root,
    /// The base class, by key.
    Found(String),
    /// The base class name is defined nowhere.
    Unknown(String),
    /// The base is the class itself or leads back to a class being resolved.
    Cycle,
    /// The lookups are nested too deeply.
    TooDeep,
}

struct Resolver<'a> {
    layer: &'a ConfigLayer,
    /// Resolved bases; `None` while a class is being resolved.
    memo: HashMap<String, Option<Base>>,
}

/// The key of the scope that holds `key` (`""` for the top level).
fn scope_of(key: &str) -> &str {
    key.rsplit_once('/').map_or("", |(scope, _)| scope)
}

fn join(scope: &str, lower: &str) -> String {
    if scope.is_empty() {
        lower.to_string()
    } else {
        format!("{scope}/{lower}")
    }
}

impl<'a> Resolver<'a> {
    fn new(layer: &'a ConfigLayer) -> Self {
        Resolver {
            layer,
            memo: HashMap::new(),
        }
    }

    fn base_of(&mut self, key: &str, depth: u32) -> Base {
        if let Some(slot) = self.memo.get(key) {
            return slot.clone().unwrap_or(Base::Cycle);
        }
        if depth > MAX_RESOLVE_DEPTH {
            return Base::TooDeep;
        }
        let Some(parent) = self.layer.live(key).and_then(|n| n.parent.clone()) else {
            return Base::Root;
        };
        self.memo.insert(key.to_string(), None);
        let found = self.find_base(key, &parent, depth);
        if found == Base::TooDeep {
            // depends on where the lookup started, so it must not stick
            self.memo.remove(key);
        } else {
            self.memo.insert(key.to_string(), Some(found.clone()));
        }
        found
    }

    fn find_base(&mut self, key: &str, name: &str, depth: u32) -> Base {
        let lower = name.to_ascii_lowercase();
        let mut itself = false;
        let mut scope = scope_of(key).to_string();
        loop {
            let direct = join(&scope, &lower);
            if direct == key {
                itself = true;
            } else if self.layer.live(&direct).is_some() {
                return Base::Found(direct);
            }
            if !scope.is_empty() {
                // the classes the scope inherits
                let mut seen: BTreeSet<String> = BTreeSet::new();
                let mut cur = scope.clone();
                while seen.len() < MAX_CHAIN {
                    match self.base_of(&cur, depth + 1) {
                        Base::Found(next) => {
                            if !seen.insert(next.clone()) {
                                break;
                            }
                            let inherited = join(&next, &lower);
                            if inherited == key {
                                itself = true;
                            } else if self.layer.live(&inherited).is_some() {
                                return Base::Found(inherited);
                            }
                            cur = next;
                        }
                        Base::TooDeep => return Base::TooDeep,
                        _ => break,
                    }
                }
            }
            if scope.is_empty() {
                break;
            }
            scope = scope_of(&scope).to_string();
        }
        if itself {
            Base::Cycle
        } else {
            Base::Unknown(name.to_string())
        }
    }

    /// The class and its bases, most derived first.
    fn chain(&mut self, key: &str) -> (Vec<String>, ChainEnd) {
        let mut keys = vec![key.to_string()];
        loop {
            let Some(cur) = keys.last().cloned() else {
                return (keys, ChainEnd::Root);
            };
            match self.base_of(&cur, 0) {
                Base::Root => return (keys, ChainEnd::Root),
                Base::Unknown(name) => return (keys, ChainEnd::Unknown(name)),
                Base::TooDeep => return (keys, ChainEnd::TooDeep),
                Base::Cycle => return (keys, ChainEnd::Cycle(None)),
                Base::Found(next) => {
                    if keys.contains(&next) {
                        return (keys, ChainEnd::Cycle(Some(next)));
                    }
                    if keys.len() >= MAX_CHAIN {
                        return (keys, ChainEnd::TooDeep);
                    }
                    keys.push(next);
                }
            }
        }
    }
}

/// How an inheritance chain ends.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ChainEnd {
    Root,
    Unknown(String),
    Cycle(Option<String>),
    TooDeep,
}

/// A class in a chain or a list of subclasses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassRef {
    /// The class name.
    pub name: String,
    /// The class path with the names as written, `/`-separated.
    pub path: String,
}

/// A property with its resolved value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedProp {
    /// The property name.
    pub name: String,
    /// The value. An infinite float is the string `"1e999"` / `"-1e999"` in JSON, as JSON has no infinity.
    #[serde(serialize_with = "plain_value")]
    pub value: Value,
    /// The name of the class the value comes from (the last one that set or extended it).
    pub origin: String,
    /// The path of that class.
    pub origin_path: String,
}

/// A redefinition that changed the base class of the class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaseChangeRef {
    /// The base before; `None` for no base.
    pub from: Option<String>,
    /// The base after; `None` for no base.
    pub to: Option<String>,
    /// The name of the source of the redefinition: `vanilla` or the mod.
    pub source: String,
    /// The PBO, relative to its install or mod directory.
    pub pbo: String,
}

/// The answer of a config lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigAnswer {
    /// The class path with the names as written.
    pub path: String,
    /// The class name.
    pub name: String,
    /// The class and its base classes, most derived first.
    pub chain: Vec<ClassRef>,
    /// `root`, or `unknown base <name>` when the last base is defined nowhere.
    pub chain_end: String,
    /// The own and inherited properties, sorted by name.
    pub properties: Vec<ResolvedProp>,
    /// The direct subclasses, sorted by path.
    pub subclasses: Vec<ClassRef>,
    /// The classes declared inside this class, sorted by path.
    pub classes: Vec<ClassRef>,
    /// The PBOs that define or modify the class, in load order.
    pub pbos: Vec<PboRef>,
    /// Redefinitions that replaced the base class (including with none), in load order.
    pub base_changes: Vec<BaseChangeRef>,
}

/// The path of `key` with the names as written.
pub(super) fn display_path(layer: &ConfigLayer, key: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut end = 0usize;
    for part in key.split('/') {
        end += part.len();
        let prefix = key.get(..end).unwrap_or(key);
        out.push(layer.nodes.get(prefix).map_or(part, |n| n.name.as_str()));
        end += 1;
    }
    out.join("/")
}

fn class_ref(layer: &ConfigLayer, key: &str) -> ClassRef {
    ClassRef {
        name: layer
            .nodes
            .get(key)
            .map_or_else(|| key.to_string(), |n| n.name.clone()),
        path: display_path(layer, key),
    }
}

/// The keys of the classes directly below `key` (`""`: the top level), in key order.
fn children<'a>(layer: &'a ConfigLayer, key: &str) -> Vec<&'a str> {
    let prefix = if key.is_empty() {
        String::new()
    } else {
        format!("{key}/")
    };
    let mut out = Vec::new();
    let mut from = prefix.clone();
    while let Some((k, node)) = layer
        .nodes
        .range::<str, _>((
            std::ops::Bound::Included(from.as_str()),
            std::ops::Bound::Unbounded,
        ))
        .next()
    {
        let Some(rest) = k.strip_prefix(prefix.as_str()) else {
            break;
        };
        if let Some((child, _)) = rest.split_once('/') {
            // below a child that sorted after its descendants' siblings: skip its whole subtree
            from = format!("{prefix}{child}0");
            continue;
        }
        if !node.deleted {
            out.push(k.as_str());
        }
        from = format!("{k}\0");
    }
    out
}

/// What is wrong with the path `wanted` that has no class: the closest existing scope and similar names.
fn not_found(layer: &ConfigLayer, wanted: &str, parts: &[&str]) -> Error {
    let mut scope = String::new();
    let mut deepest = String::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        let next = join(&scope, &part.to_ascii_lowercase());
        if layer.live(&next).is_none() {
            break;
        }
        scope = next;
        deepest = display_path(layer, &scope);
    }
    let last = parts
        .last()
        .map(|p| p.to_ascii_lowercase())
        .unwrap_or_default();
    let scope_is_parent =
        parts.len().saturating_sub(1) == scope.split('/').filter(|s| !s.is_empty()).count();
    let similar: Vec<String> = if scope_is_parent && !last.is_empty() {
        children(layer, &scope)
            .into_iter()
            .filter(|k| k.rsplit('/').next().is_some_and(|n| n.contains(&last)))
            .take(MAX_SUGGESTIONS)
            .map(|k| class_ref(layer, k).name)
            .collect()
    } else {
        Vec::new()
    };
    let mut msg = format!("no config class \"{}\"", printable(wanted));
    if !similar.is_empty() {
        let _ = write!(
            msg,
            "; similar in {}: {}",
            if deepest.is_empty() {
                "the top level"
            } else {
                deepest.as_str()
            },
            similar
                .iter()
                .map(|s| printable(s))
                .collect::<Vec<_>>()
                .join(", ")
        ); // writing to a String cannot fail
    } else if !deepest.is_empty() {
        let _ = write!(
            msg,
            "; the deepest class found is \"{}\"",
            printable(&deepest)
        ); // writing to a String cannot fail
    }
    error!("{msg}")
}

/// The properties of the chain, from the root class down: the nearer class wins and `+=` extends what the bases
/// have.
fn resolve_props(layer: &ConfigLayer, keys: &[String]) -> Vec<ResolvedProp> {
    let mut props: BTreeMap<&str, ResolvedProp> = BTreeMap::new();
    for k in keys.iter().rev() {
        let Some(class) = layer.nodes.get(k) else {
            continue;
        };
        let origin = class_ref(layer, k);
        for (lower, prop) in &class.props {
            let value = match (props.get(lower.as_str()), &prop.value) {
                (Some(have), Value::Array(more)) if prop.append => match &have.value {
                    Value::Array(base) => {
                        let mut all = base.clone();
                        all.extend(more.iter().cloned());
                        Value::Array(all)
                    }
                    _ => prop.value.clone(),
                },
                _ => prop.value.clone(),
            };
            props.insert(
                lower.as_str(),
                ResolvedProp {
                    name: prop.name.clone(),
                    value,
                    origin: origin.name.clone(),
                    origin_path: origin.path.clone(),
                },
            );
        }
    }
    props.into_values().collect()
}

/// The classes whose base resolves to `key`.
fn subclasses_of(
    layer: &ConfigLayer,
    resolver: &mut Resolver<'_>,
    key: &str,
    node: &ConfigNode,
) -> Vec<ClassRef> {
    let mut subclasses = Vec::new();
    for (k, candidate) in &layer.nodes {
        if candidate.deleted
            || k == key
            || !candidate
                .parent
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&node.name))
        {
            continue;
        }
        if resolver.base_of(k, 0) == Base::Found(key.to_string()) {
            subclasses.push(class_ref(layer, k));
        }
    }
    subclasses
}

/// Looks up the class at `path` (`/`-separated, case-insensitive) in `layer`.
///
/// # Errors
///
/// Returns an error for an empty path, a class that does not exist (or was deleted), and an inheritance cycle or
/// a chain that is too long on the way to the root.
pub fn lookup(layer: &ConfigLayer, path: &str) -> Result<ConfigAnswer, Error> {
    let parts: Vec<&str> = path
        .split('/')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err(error!(
            "the config class path is empty; give a path like CfgVehicles/Car"
        ));
    }
    let key = parts
        .iter()
        .map(|p| p.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/");
    let Some(node) = layer.live(&key) else {
        return Err(not_found(layer, &parts.join("/"), &parts));
    };
    let mut resolver = Resolver::new(layer);
    let (keys, end) = resolver.chain(&key);
    let chain: Vec<ClassRef> = keys.iter().map(|k| class_ref(layer, k)).collect();
    let chain_end = match end {
        ChainEnd::Root => "root".to_string(),
        ChainEnd::Unknown(name) => format!("unknown base {name}"),
        ChainEnd::Cycle(back) => {
            let names: Vec<&str> = chain.iter().map(|c| c.name.as_str()).collect();
            let back_name = back.as_deref().map_or_else(
                || names.last().copied().unwrap_or_default().to_string(),
                |k| class_ref(layer, k).name,
            );
            return Err(error!(
                "inheritance cycle at \"{}\": {} : {}",
                printable(&display_path(layer, &key)),
                printable(&names.join(" : ")),
                printable(&back_name)
            ));
        }
        ChainEnd::TooDeep => {
            return Err(error!(
                "the inheritance chain of \"{}\" is longer than {MAX_CHAIN} classes or nested too deeply",
                printable(&display_path(layer, &key))
            ));
        }
    };
    let properties = resolve_props(layer, &keys);
    let subclasses = subclasses_of(layer, &mut resolver, &key, node);
    let base_changes = node
        .base_changes
        .iter()
        .filter_map(|c| {
            let p = layer.pbos.get(usize::try_from(c.pbo).ok()?)?;
            Some(BaseChangeRef {
                from: c.from.clone(),
                to: c.to.clone(),
                source: p.source.clone(),
                pbo: p.pbo.clone(),
            })
        })
        .collect();
    let pbos = node
        .pbos
        .iter()
        .filter_map(|i| layer.pbos.get(usize::try_from(*i).ok()?))
        .cloned()
        .collect();
    Ok(ConfigAnswer {
        path: display_path(layer, &key),
        name: node.name.clone(),
        chain,
        chain_end,
        properties,
        subclasses,
        classes: children(layer, &key)
            .into_iter()
            .map(|k| class_ref(layer, k))
            .collect(),
        pbos,
        base_changes,
    })
}

/// The text of an infinite float (see [`NONFINITE`]) held by `v`.
fn nonfinite(v: &Value) -> Option<&str> {
    v.as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get(NONFINITE))
        .and_then(Value::as_str)
}

/// `value` for JSON: an infinite float becomes a string.
fn plain(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(plain).collect()),
        other => nonfinite(other).map_or_else(|| other.clone(), |t| Value::String(t.to_string())),
    }
}

fn plain_value<S: serde::Serializer>(value: &Value, serializer: S) -> Result<S::Ok, S::Error> {
    plain(value).serialize(serializer)
}

/// A value as config text: strings quoted, arrays in braces; long values are cut.
fn value_text(value: &Value) -> String {
    fn go(v: &Value, out: &mut String) {
        match v {
            Value::String(s) => {
                out.push('"');
                out.push_str(&s.replace('"', "\"\""));
                out.push('"');
            }
            Value::Number(n) => out.push_str(&n.to_string()),
            inf if nonfinite(inf).is_some() => out.push_str(nonfinite(inf).unwrap_or_default()),
            Value::Array(items) => {
                out.push('{');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    go(item, out);
                    if out.len() > MAX_TEXT_VALUE * 4 {
                        return;
                    }
                }
                out.push('}');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut text = String::new();
    go(value, &mut text);
    printable(&cut(&text))
}

fn cut(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT_VALUE {
        return text.to_string();
    }
    let mut short: String = text.chars().take(MAX_TEXT_VALUE).collect();
    short.push_str(" ...");
    short
}

/// The text output: the path, the chain, the PBOs, the properties and the subclasses, at most `max` lines then
/// `... N more lines`.
#[must_use]
pub fn render_text(answer: &ConfigAnswer, max: usize) -> Vec<String> {
    let mut lines = vec![printable(&answer.path)];
    let mut chain: Vec<String> = answer.chain.iter().map(|c| printable(&c.name)).collect();
    if let Some(name) = answer.chain_end.strip_prefix("unknown base ") {
        chain.push(format!("{} (not found)", printable(name)));
    }
    lines.push(format!("chain: {}", chain.join(" : ")));
    let shown = |b: &Option<String>| b.as_deref().map_or_else(|| "(none)".to_string(), printable);
    lines.extend(answer.base_changes.iter().map(|c| {
        format!(
            "Base changed: {} -> {} by [{}] {}",
            shown(&c.from),
            shown(&c.to),
            printable(&c.source),
            printable(&c.pbo)
        )
    }));
    lines.push(format!("defined in ({}):", answer.pbos.len()));
    lines.extend(
        answer
            .pbos
            .iter()
            .map(|p| format!("  [{}] {}", printable(&p.source), printable(&p.pbo))),
    );
    lines.push(format!("properties ({}):", answer.properties.len()));
    lines.extend(answer.properties.iter().map(|p| {
        format!(
            "  {} = {}  (from {})",
            printable(&p.name),
            value_text(&p.value),
            printable(&p.origin)
        )
    }));
    lines.push(format!("classes ({}):", answer.classes.len()));
    lines.extend(
        answer
            .classes
            .iter()
            .map(|c| format!("  {}", printable(&c.name))),
    );
    lines.push(format!("subclasses ({}):", answer.subclasses.len()));
    lines.extend(
        answer
            .subclasses
            .iter()
            .map(|c| format!("  {}", printable(&c.name))),
    );
    if lines.len() > max {
        let more = lines.len() - max;
        lines.truncate(max);
        lines.push(format!(
            "... {more} more lines (raise --max or narrow the query)"
        ));
    }
    lines
}

/// The JSON output: one object with `path`, `name`, `chain`, `chain_end`, `properties`, `subclasses`, `classes`,
/// `pbos` and `truncated`. At most `max` properties, subclasses and nested classes are given; `truncated` counts the ones left out.
///
/// # Errors
///
/// Returns an error when the answer cannot be serialized.
pub fn render_json(answer: &ConfigAnswer, max: usize) -> Result<String, Error> {
    let props: Vec<&ResolvedProp> = answer.properties.iter().take(max).collect();
    let subs: Vec<&ClassRef> = answer.subclasses.iter().take(max).collect();
    let nested: Vec<&ClassRef> = answer.classes.iter().take(max).collect();
    let truncated = (answer.properties.len() - props.len())
        + (answer.subclasses.len() - subs.len())
        + (answer.classes.len() - nested.len());
    serde_json::to_string(&serde_json::json!({
        "path": answer.path,
        "name": answer.name,
        "chain": answer.chain,
        "chain_end": answer.chain_end,
        "properties": props,
        "subclasses": subs,
        "classes": nested,
        "pbos": answer.pbos,
        "base_changes": answer.base_changes,
        "truncated": truncated,
    }))
    .map_err(|e| error!("cannot serialize: {e}"))
}
