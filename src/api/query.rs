//! Lookups on the script API index and their text and JSON renderers.
//!
//! `indexes[0]` is vanilla; the others are mods. Results own their data (clones of the model), so they outlive the
//! indexes and can be serialized or handed to another thread.
//!
//! Output limits: `render_text` caps the number of lines. `render_json` caps the leaf items of the result: classes
//! and their members for `Classes`, enums and their items for `Enums`, define sets and their names for `Defines`,
//! and the entries of every other variant. A class or enum counts as one item plus one per member or item, so
//! `max` bounds the size of the output.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::io::Error;

use regex::{Regex, RegexBuilder};
use serde::Serialize;

use super::model::{ApiIndex, Class, Enum, EnumItem, FileRef, Member, MemberKind};
use crate::error;

/// Size limit of a compiled user regex, in bytes. The regex comes from an untrusted caller (MCP).
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// A lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Query {
    /// One class, optionally with its base classes; `mods_only` prints only the parts the mods declare.
    Class {
        /// Class name.
        name: String,
        /// Also print every base class, up to the root.
        inherited: bool,
        /// Skip vanilla (`indexes[0]`).
        mods_only: bool,
    },
    /// Case-insensitive regex over member names, signatures and docs, class names and enum items.
    Find {
        /// The regex.
        regex: String,
        /// Restrict the search to the members of this class or enum.
        class: Option<String>,
    },
    /// Subclasses of a class.
    Sub {
        /// Class name.
        name: String,
        /// Transitive subclasses instead of the direct ones.
        all: bool,
    },
    /// One enum.
    Enum {
        /// Enum name.
        name: String,
    },
    /// The base class chain of a class.
    Chain {
        /// Class name.
        name: String,
    },
    /// Where a class or enum is declared.
    Src {
        /// Class or enum name.
        name: String,
    },
    /// Compile-time defines.
    Defines {
        /// Case-insensitive regex over the define names; all when `None`.
        regex: Option<String>,
    },
}

/// A declaration site, with the PBO that holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SrcFile {
    /// Name of the index (`vanilla` or the mod).
    pub index: String,
    /// PBO file relative to its install.
    pub pbo: String,
    /// PBO prefix, `\`-separated.
    pub prefix: String,
    /// Entry name inside the PBO, `\`-separated.
    pub path: String,
    /// 1-based line.
    pub line: u32,
}

/// A class of one index.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClassView {
    /// Name of the index the class is from.
    pub index: String,
    /// The class with all its declarations of that index merged.
    pub class: Class,
}

/// An enum of one index.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnumView {
    /// Name of the index the enum is from.
    pub index: String,
    /// The enum.
    pub en: Enum,
}

/// One hit of a `find`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    /// Name of the index.
    pub index: String,
    /// Class name, enum name, or `fn`, `var`, `typedef` for top-level declarations.
    pub scope: String,
    /// The matching member or declaration.
    pub member: Option<Member>,
    /// The matching enum item.
    pub item: Option<EnumItem>,
    /// The class name itself matched (`member` and `item` are `None`).
    pub class_name_match: bool,
}

/// Compile-time defines of one PBO prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefineSet {
    /// Name of the index.
    pub index: String,
    /// PBO prefix.
    pub prefix: String,
    /// Define names.
    pub names: Vec<String>,
}

/// The answer to a `Query`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum QueryResult {
    /// `Query::Class`, in print order.
    Classes(Vec<ClassView>),
    /// `Query::Find`.
    Matches(Vec<Match>),
    /// `Query::Sub`.
    Names(Vec<String>),
    /// `Query::Chain`, the class first.
    Chain(Vec<String>),
    /// `Query::Enum`.
    Enums(Vec<EnumView>),
    /// `Query::Src`.
    Sources(Vec<SrcFile>),
    /// `Query::Defines`.
    Defines(Vec<DefineSet>),
}

fn compile(pattern: &str) -> Result<Regex, Error> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| error!("invalid regex {pattern:?}: {e}"))
}

fn not_found(indexes: &[ApiIndex], what: &str, name: &str) -> Error {
    let mut msg = format!("no {what} \"{name}\"; try: dayzmake api find {name}");
    if indexes.len() < 2 {
        msg.push_str("\n  (searched vanilla only; add -m <mod> or -m all for mods)");
    }
    error!("{msg}")
}

/// The base class name of `name`, from the first declaration in any index that names one.
fn base_of(indexes: &[ApiIndex], name: &str) -> Option<String> {
    indexes
        .iter()
        .filter_map(|i| i.classes.get(name))
        .find_map(|c| c.base_name())
        .filter(|b| !b.is_empty())
        .map(ToString::to_string)
}

fn has_class(indexes: &[ApiIndex], name: &str) -> bool {
    indexes.iter().any(|i| i.classes.contains_key(name))
}

/// Runs `query`. `indexes[0]` is vanilla.
///
/// # Errors
///
/// An unknown class or enum (with a hint how to search), or an invalid or too large regex.
pub fn run_query(indexes: &[ApiIndex], query: &Query) -> Result<QueryResult, Error> {
    match query {
        Query::Class {
            name,
            inherited,
            mods_only,
        } => class_query(indexes, name, *inherited, *mods_only),
        Query::Find { regex, class } => Ok(find_query(indexes, &compile(regex)?, class.as_deref())),
        Query::Sub { name, all } => sub_query(indexes, name, *all),
        Query::Enum { name } => {
            let views: Vec<EnumView> = indexes
                .iter()
                .filter_map(|i| {
                    i.enums.get(name).map(|e| EnumView {
                        index: i.name.clone(),
                        en: e.clone(),
                    })
                })
                .collect();
            if views.is_empty() {
                return Err(not_found(indexes, "enum", name));
            }
            Ok(QueryResult::Enums(views))
        }
        Query::Chain { name } => {
            if !has_class(indexes, name) {
                return Err(not_found(indexes, "class", name));
            }
            let mut chain: Vec<String> = Vec::new();
            let mut cur = Some(name.clone());
            while let Some(n) = cur.take() {
                if chain.contains(&n) {
                    break;
                }
                cur = base_of(indexes, &n);
                chain.push(n);
            }
            Ok(QueryResult::Chain(chain))
        }
        Query::Src { name } => {
            let files = src_files(indexes, name);
            if files.is_empty() {
                return Err(not_found(indexes, "class or enum", name));
            }
            Ok(QueryResult::Sources(files))
        }
        Query::Defines { regex } => {
            let rx = regex.as_deref().map(compile).transpose()?;
            let mut sets = Vec::new();
            for i in indexes {
                for (prefix, names) in &i.defines {
                    let names: Vec<String> = names
                        .iter()
                        .filter(|n| rx.as_ref().is_none_or(|r| r.is_match(n)))
                        .cloned()
                        .collect();
                    if !names.is_empty() {
                        sets.push(DefineSet {
                            index: i.name.clone(),
                            prefix: prefix.clone(),
                            names,
                        });
                    }
                }
            }
            Ok(QueryResult::Defines(sets))
        }
    }
}

fn class_query(
    indexes: &[ApiIndex],
    name: &str,
    inherited: bool,
    mods_only: bool,
) -> Result<QueryResult, Error> {
    let mut views = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut cur = Some(name.to_string());
    while let Some(n) = cur.take() {
        if !seen.insert(n.clone()) {
            break;
        }
        for (pos, i) in indexes.iter().enumerate() {
            if mods_only && pos == 0 {
                continue;
            }
            if let Some(c) = i.classes.get(&n) {
                views.push(ClassView {
                    index: i.name.clone(),
                    class: c.clone(),
                });
            }
        }
        if inherited {
            cur = base_of(indexes, &n);
        }
    }
    if views.is_empty() {
        if mods_only && has_class(indexes, name) {
            let scope = if inherited {
                " or its base classes"
            } else {
                ""
            };
            return Err(error!(
                "\"{name}\" exists in vanilla, but no loaded mod declares it{scope}"
            ));
        }
        return Err(not_found(indexes, "class", name));
    }
    Ok(QueryResult::Classes(views))
}

fn find_query(indexes: &[ApiIndex], rx: &Regex, class: Option<&str>) -> QueryResult {
    let hit = |m: &Member| {
        rx.is_match(&m.name)
            || rx.is_match(&m.signature)
            || m.doc.as_deref().is_some_and(|d| rx.is_match(d))
    };
    let mut out = Vec::new();
    for i in indexes {
        let mk = |scope: &str,
                  member: Option<&Member>,
                  item: Option<&EnumItem>,
                  name_match: bool| Match {
            index: i.name.clone(),
            scope: scope.to_string(),
            member: member.cloned(),
            item: item.cloned(),
            class_name_match: name_match,
        };
        for (name, c) in &i.classes {
            if class.is_some_and(|f| f != name) {
                continue;
            }
            if rx.is_match(name) {
                out.push(mk(name, None, None, true));
            }
            for m in c.members.iter().filter(|m| hit(m)) {
                out.push(mk(name, Some(m), None, false));
            }
        }
        for (name, e) in &i.enums {
            if class.is_some_and(|f| f != name) {
                continue;
            }
            for it in e.items.iter().filter(|it| {
                rx.is_match(&it.name) || it.doc.as_deref().is_some_and(|d| rx.is_match(d))
            }) {
                out.push(mk(name, None, Some(it), false));
            }
        }
        if class.is_none() {
            for (scope, list) in [
                ("typedef", &i.typedefs),
                ("fn", &i.functions),
                ("var", &i.globals),
            ] {
                for m in list.iter().filter(|m| hit(m)) {
                    out.push(mk(scope, Some(m), None, false));
                }
            }
        }
    }
    QueryResult::Matches(out)
}

fn sub_query(indexes: &[ApiIndex], name: &str, all: bool) -> Result<QueryResult, Error> {
    let mut kids: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for c in indexes.iter().flat_map(|i| i.classes.values()) {
        if let Some(b) = c.base_name().filter(|b| !b.is_empty()) {
            kids.entry(b).or_default().insert(c.name.as_str());
        }
    }
    if !kids.contains_key(name) && !has_class(indexes, name) {
        return Err(not_found(indexes, "class", name));
    }
    let mut found: Vec<String> = Vec::new();
    let mut todo: VecDeque<&str> = VecDeque::from([name]);
    while let Some(n) = todo.pop_front() {
        for k in kids.get(n).into_iter().flatten() {
            if found.iter().any(|f| f == k) {
                continue;
            }
            found.push((*k).to_string());
            if all {
                todo.push_back(k);
            }
        }
    }
    Ok(QueryResult::Names(found))
}

/// Every declaration of the class or enum `name`, in index and load order.
#[must_use]
pub fn src_files(indexes: &[ApiIndex], name: &str) -> Vec<SrcFile> {
    let mut out = Vec::new();
    for i in indexes {
        let decls = i
            .classes
            .get(name)
            .map(|c| c.decls.as_slice())
            .into_iter()
            .chain(i.enums.get(name).map(|e| e.decls.as_slice()));
        for d in decls.flatten() {
            let m = i.modules.get(d.module);
            out.push(SrcFile {
                index: i.name.clone(),
                pbo: m.map(|m| m.pbo.clone()).unwrap_or_default(),
                prefix: m.map(|m| m.prefix.clone()).unwrap_or_default(),
                path: d.path.clone(),
                line: d.line,
            });
        }
    }
    out
}

fn layers(decls: &[FileRef]) -> String {
    let set: BTreeSet<&str> = decls
        .iter()
        .filter_map(|d| d.path.split('\\').next())
        .filter(|l| !l.is_empty())
        .collect();
    set.into_iter().collect::<Vec<_>>().join(",")
}

fn tags(index: &str, decls: &[FileRef], multi: bool, modded: bool, cond: Option<&str>) -> String {
    let mut s = String::new();
    if multi {
        let l = layers(decls);
        let _ = write!(s, "  [{index}{}{l}]", if l.is_empty() { "" } else { " " });
    }
    if modded {
        s.push_str("  [modded]");
    }
    if let Some(c) = cond {
        let _ = write!(s, "  [#if {c}]");
    }
    s
}

fn with_doc_cond(mut s: String, doc: Option<&str>, cond: Option<&str>) -> String {
    if let Some(d) = doc {
        s.push_str("  // ");
        s.push_str(d);
    }
    if let Some(c) = cond {
        let _ = write!(s, "  [#if {c}]");
    }
    s
}

fn member_line(m: &Member) -> String {
    with_doc_cond(m.signature.clone(), m.doc.as_deref(), m.cond.as_deref())
}

fn item_line(it: &EnumItem) -> String {
    let head = match it.value {
        Some(v) => format!("{} = {v}", it.name),
        None => it.name.clone(),
    };
    with_doc_cond(head, it.doc.as_deref(), None)
}

fn by_name(a: &Member, b: &Member) -> std::cmp::Ordering {
    (a.name.to_lowercase(), &a.name, &a.signature).cmp(&(
        b.name.to_lowercase(),
        &b.name,
        &b.signature,
    ))
}

fn render_class(v: &ClassView, multi: bool, out: &mut Vec<String>) {
    let c = &v.class;
    let mut head = format!("class {}", c.name);
    if !c.type_params.is_empty() {
        let _ = write!(head, "<{}>", c.type_params.join(","));
    }
    if let Some(b) = &c.base {
        let _ = write!(head, " : {b}");
    }
    head.push_str(&tags(
        &v.index,
        &c.decls,
        multi,
        c.modded_only,
        c.cond.as_deref(),
    ));
    out.push(head);
    if let Some(d) = &c.doc {
        out.push(format!("  // {d}"));
    }
    if !c.alt_bases.is_empty() {
        out.push(format!(
            "  // base is {} here; {} under the other branch of its #ifdef",
            c.base_name().unwrap_or("?"),
            c.alt_bases.join(" or ")
        ));
    }
    let collapsed = |m: &&Member| m.modifiers.iter().any(|x| x == "override") && m.cond.is_none();
    let mut fields: Vec<&Member> = c
        .members
        .iter()
        .filter(|m| m.kind == MemberKind::Field && !collapsed(m))
        .collect();
    let mut methods: Vec<&Member> = c
        .members
        .iter()
        .filter(|m| m.kind == MemberKind::Method && !collapsed(m))
        .collect();
    let mut overrides: Vec<&Member> = c.members.iter().filter(collapsed).collect();
    fields.sort_by(|a, b| by_name(a, b));
    methods.sort_by(|a, b| by_name(a, b));
    overrides.sort_by(|a, b| by_name(a, b));
    for m in fields.into_iter().chain(methods) {
        out.push(format!("  {}", member_line(m)));
    }
    let mut names: Vec<&str> = overrides.iter().map(|m| m.name.as_str()).collect();
    names.dedup();
    if !names.is_empty() {
        out.push(format!("  override: {}", names.join(" ")));
    }
}

fn render_enum(v: &EnumView, multi: bool, out: &mut Vec<String>) {
    let e = &v.en;
    let mut head = format!("enum {}", e.name);
    if let Some(b) = &e.base {
        let _ = write!(head, " : {b}");
    }
    head.push_str(&tags(&v.index, &e.decls, multi, false, e.cond.as_deref()));
    out.push(head);
    out.extend(e.items.iter().map(|it| format!("  {}", item_line(it))));
}

fn render_match(m: &Match, multi: bool) -> String {
    let tag = if multi {
        format!("[{}] ", m.index)
    } else {
        String::new()
    };
    let body = match (&m.member, &m.item) {
        (Some(mem), _) => format!("{}: {}", m.scope, member_line(mem)),
        (None, Some(it)) => format!("{}: {}", m.scope, item_line(it)),
        (None, None) => format!("class {}", m.scope),
    };
    format!("{tag}{body}")
}

/// Renders `result` as text, at most `max` lines; the cut is announced by a final
/// `... N more lines (raise --max or narrow the query)`. `multi` is true when more than one index is loaded: lines
/// are then tagged with the index name.
#[must_use]
pub fn render_text(result: &QueryResult, multi: bool, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match result {
        QueryResult::Classes(v) => v.iter().for_each(|c| render_class(c, multi, &mut out)),
        QueryResult::Enums(v) => v.iter().for_each(|e| render_enum(e, multi, &mut out)),
        QueryResult::Matches(v) => {
            out.extend(v.iter().map(|m| render_match(m, multi)));
            if out.is_empty() {
                out.push(if multi {
                    "no match".to_string()
                } else {
                    "no match  (searched vanilla only; add -m <mod> or -m all for mods)".to_string()
                });
            }
        }
        QueryResult::Names(v) => out.push(if v.is_empty() {
            "no subclasses".to_string()
        } else {
            v.join(" ")
        }),
        QueryResult::Chain(v) => out.push(v.join(" > ")),
        QueryResult::Sources(v) => out.extend(v.iter().map(|s| {
            format!(
                "[{}] {} {}\\{}:{}",
                s.index, s.pbo, s.prefix, s.path, s.line
            )
        })),
        QueryResult::Defines(v) => {
            out.extend(
                v.iter()
                    .map(|d| format!("[{}] {}: {}", d.index, d.prefix, d.names.join(" "))),
            );
            if out.is_empty() {
                out.push("no matching define".to_string());
            }
        }
    }
    if out.len() > max {
        let more = out.len() - max;
        out.truncate(max);
        out.push(format!(
            "... {more} more lines (raise --max or narrow the query)"
        ));
    }
    out
}

/// Keeps at most `max` leaf items of `items` (each item counts one plus its children, see `count`; `trim` cuts the
/// children down to the budget it is given and returns how many it dropped). Returns the number of dropped items.
fn cap<T>(
    items: &mut Vec<T>,
    max: usize,
    count: impl Fn(&T) -> usize,
    trim: impl Fn(&mut T, usize) -> usize,
) -> usize {
    let mut left = max;
    let mut dropped = 0;
    let mut kept = Vec::new();
    for mut it in std::mem::take(items) {
        let children = count(&it);
        if left == 0 {
            dropped += 1 + children;
            continue;
        }
        left -= 1;
        let d = trim(&mut it, left);
        left -= children.saturating_sub(d);
        dropped += d;
        kept.push(it);
    }
    *items = kept;
    dropped
}

fn cap_flat<T>(items: &mut Vec<T>, max: usize) -> usize {
    let dropped = items.len().saturating_sub(max);
    items.truncate(max);
    dropped
}

fn trim_vec<T>(v: &mut Vec<T>, budget: usize) -> usize {
    cap_flat(v, budget)
}

/// Renders `{"result": <result>, "truncated": N}`. Leaf items beyond `max` are dropped and counted in `truncated`
/// (0 when nothing was dropped); see the module docs for what counts as an item.
#[must_use]
pub fn render_json(result: &QueryResult, max: usize) -> String {
    #[derive(Serialize)]
    struct Out<'a> {
        result: &'a QueryResult,
        truncated: usize,
    }
    let mut r = result.clone();
    let truncated = match &mut r {
        QueryResult::Classes(v) => cap(
            v,
            max,
            |c| c.class.members.len(),
            |c, b| trim_vec(&mut c.class.members, b),
        ),
        QueryResult::Enums(v) => cap(
            v,
            max,
            |e| e.en.items.len(),
            |e, b| trim_vec(&mut e.en.items, b),
        ),
        QueryResult::Defines(v) => cap(v, max, |d| d.names.len(), |d, b| trim_vec(&mut d.names, b)),
        QueryResult::Matches(v) => cap_flat(v, max),
        QueryResult::Names(v) | QueryResult::Chain(v) => cap_flat(v, max),
        QueryResult::Sources(v) => cap_flat(v, max),
    };
    serde_json::to_string(&Out {
        result: &r,
        truncated,
    })
    .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }).to_string())
}
