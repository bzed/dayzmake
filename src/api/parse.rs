//! Enforce Script declaration parser.
//!
//! The parser reads script sources without a full grammar. A scanner first removes comments and preprocessor
//! lines (evaluating `#ifdef`/`#ifndef`/`#else`/`#endif` against a set of defines), keeps doc comments as
//! marker-delimited text and keeps every newline, so a byte offset in the cleaned text still maps to its source
//! line. A splitter then cuts the cleaned text into statements at brace depth 0, which is enough to find classes,
//! enums, members and top-level declarations. Nothing recurses and nothing indexes out of bounds, so hostile
//! input produces a partial result instead of a panic.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use super::model::{ApiIndex, Class, Enum, EnumItem, FileRef, Member, MemberKind};
use super::scan::{DOC_END, DOC_MAX, DOC_START, at, cond_at, find_byte, scan, skip_string, sub};
use super::steam::Branch;

/// Script layer directories, in load order.
const LAYERS: [&str; 5] = ["1_Core", "2_GameLib", "3_Game", "4_World", "5_Mission"];
/// Keywords that are modifiers when they precede a member name.
const MODS: [&str; 21] = [
    "static",
    "override",
    "proto",
    "native",
    "private",
    "protected",
    "event",
    "const",
    "external",
    "volatile",
    "sealed",
    "owned",
    "autoptr",
    "ref",
    "notnull",
    "local",
    "reference",
    "abstract",
    "final",
    "out",
    "inout",
];
/// Modifiers that are not kept.
const DROP: [&str; 8] = [
    "autoptr",
    "ref",
    "owned",
    "external",
    "volatile",
    "local",
    "reference",
    "event",
];
/// Defines of a dedicated server; picks the active base of `#ifdef X class A extends B #else ... #endif`.
const ACTIVE_DEFINES: [&str; 7] = [
    "DAYZ_1_30",
    "SERVER",
    "PLATFORM_LINUX",
    "RELEASE",
    "FEATURE_CURSOR",
    "FEATURE_NETWORK_RECONCILIATION",
    "FEATURE_NETWORK_RECONCILATION",
];

/// The static patterns of the parser.
struct Patterns {
    header: Regex,
    token: Regex,
    comma: Regex,
    equals: Regex,
    lt: Regex,
    gt: Regex,
    noise: Regex,
    override_static: Regex,
    defines_array: Regex,
    quoted: Regex,
    define_line: Regex,
}

#[expect(clippy::unwrap_used, reason = "the static patterns are valid regexes")]
static RE: LazyLock<Patterns> = LazyLock::new(|| {
    Patterns {
    header: Regex::new(
        r"^((?:(?:modded|sealed|abstract)\s+)*)(class|enum|struct)\s+(\w+)\s*(<[^>]*>)?\s*(?:(?:extends|:)\s*([\w.]+(?:<[^>]*>)?))?\s*$",
    )
    .unwrap(),
    token: Regex::new(r"~?[\w.]+(?:<[^>]*>)?(?:\[\])?|\S").unwrap(),
    comma: Regex::new(r"\s*,\s*").unwrap(),
    equals: Regex::new(r"\s*=\s*").unwrap(),
    lt: Regex::new(r"\s*<\s*").unwrap(),
    gt: Regex::new(r"\s*>").unwrap(),
    noise: Regex::new(r"\b(?:autoptr|ref|owned|notnull|reference)\s+").unwrap(),
    override_static: Regex::new(r"\b(?:override|static)\s+").unwrap(),
    defines_array: Regex::new(r"\bdefines\[\]\s*=\s*\{([^}]*)\}").unwrap(),
    quoted: Regex::new(r#""([^"]+)""#).unwrap(),
    define_line: Regex::new(r"(?m)^[ \t]*#define[ \t]+(\w+)").unwrap(),
}
});

/// One script file to index.
pub struct ScriptFile<'a> {
    /// Index into `ApiIndex::modules`.
    pub module: usize,
    /// Entry name inside the PBO, `\`-separated, including the layer directory (`4_World\Entities\X.c`).
    pub path: &'a str,
    /// File content.
    pub text: &'a str,
}

/// Defines that are set on a dedicated server, used to pick the active branch of `#ifdef`s.
///
/// Both branches use the same set; the version define is the one of the 1.30 scripts the set was taken from.
#[must_use]
pub fn active_defines(_branch: Branch) -> BTreeSet<String> {
    ACTIVE_DEFINES.iter().map(ToString::to_string).collect()
}

/// The defines a mod sets: the `defines[]` arrays of its `config.cpp` and the `#define NAME` lines of its
/// scripts, without duplicates, in the order they are first seen.
#[must_use]
pub fn collect_defines(config_text: &str, scripts: &[ScriptFile<'_>]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |name: &str| {
        if seen.insert(name.to_string()) {
            out.push(name.to_string());
        }
    };
    for array in RE.defines_array.captures_iter(config_text) {
        let list = array.get(1).map_or("", |m| m.as_str());
        for name in RE.quoted.captures_iter(list).filter_map(|c| c.get(1)) {
            add(name.as_str());
        }
    }
    for script in scripts {
        for name in RE
            .define_line
            .captures_iter(script.text)
            .filter_map(|c| c.get(1))
        {
            add(name.as_str());
        }
    }
    out
}

/// Parse the files and merge their classes, enums and top-level declarations into `into`.
///
/// Files are processed in a deterministic order, sorted by script layer, module and path, whatever the order
/// of `files`. The caller sets name, title, build and modules of the index.
///
/// A class member is identified by its signature without `override`/`static`; the first declaration of a
/// member wins, so a `modded class` that overrides a method does not list it twice and the `override` flag of
/// the later declaration is lost.
pub fn index_scripts(files: &[ScriptFile<'_>], defines: &BTreeSet<String>, into: &mut ApiIndex) {
    let mut order: Vec<&ScriptFile<'_>> = files.iter().collect();
    order.sort_by(|a, b| {
        (layer_rank(a.path), a.module, a.path).cmp(&(layer_rank(b.path), b.module, b.path))
    });
    let mut indexer = Indexer {
        member_keys: HashMap::new(),
        typedef_keys: index_keys(&into.typedefs, |m| m.signature.clone()),
        function_keys: index_keys(&into.functions, |m| m.signature.clone()),
        global_keys: index_keys(&into.globals, |m| m.signature.clone()),
        into,
        defines,
    };
    for file in order {
        indexer.file(file);
    }
}

/// Position of the layer directory in the path; paths without one sort last.
fn layer_rank(path: &str) -> usize {
    path.split(['\\', '/'])
        .find_map(|part| LAYERS.iter().position(|l| *l == part))
        .unwrap_or(LAYERS.len())
}

// ---------------------------------------------------------------------------------------------------------
// Statement splitter

/// A statement at brace depth 0: its header is `text[start..head_end]`.
struct Stmt {
    start: usize,
    head_end: usize,
    /// Range of the text between the braces.
    body: Option<(usize, usize)>,
}

/// `(end of body, position after the closing brace)` of the brace at `open`; unbalanced braces end at `to`.
fn match_brace(b: &[u8], open: usize, to: usize) -> (usize, usize) {
    let (mut j, mut depth) = (open + 1, 1usize);
    while j < to {
        match at(b, j) {
            DOC_START => j = find_byte(b, j, DOC_END).unwrap_or(to),
            b'"' => {
                j = skip_string(b, j);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (j, j + 1);
                }
            }
            _ => {}
        }
        j += 1;
    }
    (to, to)
}

/// The header declares a field with an initializer list (`int a[] = {1, 2};`), not a body.
fn is_initializer(header: &str) -> bool {
    let plain = clean(header).text;
    plain
        .find('=')
        .is_some_and(|p| !sub(&plain, 0, p).contains('('))
}

fn split(text: &str, from: usize, to: usize) -> Vec<Stmt> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut start) = (from, from);
    while i < to {
        match at(b, i) {
            DOC_START => i = find_byte(b, i, DOC_END).unwrap_or(to),
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b';' => {
                let blank = b
                    .get(start..i)
                    .unwrap_or_default()
                    .iter()
                    .all(u8::is_ascii_whitespace);
                if !blank {
                    out.push(Stmt {
                        start,
                        head_end: i,
                        body: None,
                    });
                }
                start = i + 1;
            }
            b'{' => {
                let (body_end, next) = match_brace(b, i, to);
                if is_initializer(sub(text, start, i)) {
                    let k = find_byte(b, next, b';').filter(|&k| k < to).unwrap_or(to);
                    out.push(Stmt {
                        start,
                        head_end: k,
                        body: None,
                    });
                    i = k;
                    start = k + 1;
                } else {
                    out.push(Stmt {
                        start,
                        head_end: i,
                        body: Some((i + 1, body_end)),
                    });
                    i = next;
                    if i < to && at(b, i) == b';' {
                        i += 1;
                    }
                    start = i;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Offset of the first token of a statement: not whitespace, not a doc comment.
fn first_token(b: &[u8], start: usize, end: usize) -> usize {
    let mut i = start;
    while i < end {
        match at(b, i) {
            DOC_START => i = find_byte(b, i, DOC_END).unwrap_or(end),
            c if c.is_ascii_whitespace() => {}
            _ => return i,
        }
        i += 1;
    }
    end
}

// ---------------------------------------------------------------------------------------------------------
// Declarations

struct Cleaned {
    /// Header without doc comments and leading attributes, whitespace collapsed.
    text: String,
    /// Raw text of the last doc comment.
    doc: Option<String>,
    obsolete: bool,
}

fn clean(header: &str) -> Cleaned {
    let mut text = String::with_capacity(header.len());
    let mut doc = None;
    let mut rest = header;
    while let Some(p) = rest.find(char::from(DOC_START)) {
        text.push_str(sub(rest, 0, p));
        text.push(' ');
        let after = rest.get(p + 1..).unwrap_or_default();
        let end = after.find(char::from(DOC_END)).unwrap_or(after.len());
        doc = Some(sub(after, 0, end).to_string());
        rest = after.get(end + 1..).unwrap_or_default();
    }
    text.push_str(rest);
    let mut t = text.trim_start();
    let mut obsolete = false;
    while t.starts_with('[') {
        let Some(close) = t.find(']') else { break };
        obsolete |= sub(t, 1, close)
            .split(',')
            .any(|a| a.trim_start().starts_with("Obsolete"));
        t = t.get(close + 1..).unwrap_or_default().trim_start();
    }
    Cleaned {
        text: t.split_whitespace().collect::<Vec<_>>().join(" "),
        doc,
        obsolete,
    }
}

/// A doc as stored: group markers (`@{`, `}`) and empty docs are none.
fn doc_value(doc: Option<&str>) -> Option<String> {
    let d = doc?.trim();
    if d.is_empty() || d.starts_with(['@', '}', '{']) {
        return None;
    }
    Some(d.chars().take(DOC_MAX).collect())
}

/// Collapse the spaces of a declaration and drop ownership keywords. String literals are left alone.
fn short(s: &str) -> String {
    fn plain(seg: &str) -> String {
        let s = RE.comma.replace_all(seg, ",");
        let s = RE.equals.replace_all(&s, "=");
        let s = RE.lt.replace_all(&s, "<");
        let s = RE.gt.replace_all(&s, ">");
        RE.noise.replace_all(&s, "").into_owned()
    }
    let mut out = String::with_capacity(s.len());
    let mut seg = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            out.push_str(&plain(&seg));
            seg.clear();
            out.push(c);
            let mut escaped = false;
            for d in chars.by_ref() {
                out.push(d);
                if escaped {
                    escaped = false;
                } else if d == '\\' {
                    escaped = true;
                } else if d == '"' {
                    break;
                }
            }
        } else {
            seg.push(c);
        }
    }
    out.push_str(&plain(&seg));
    out.trim().to_string()
}

/// A modifier as shown: `proto` is `native`.
fn shown(keyword: &str) -> &str {
    if keyword == "proto" {
        "native"
    } else {
        keyword
    }
}

fn add_modifier(mods: &mut Vec<String>, keyword: &str) {
    let m = shown(keyword);
    if !DROP.contains(&keyword) && !mods.iter().any(|x| x == m) {
        mods.push(m.to_string());
    }
}

/// Parse a method or field header (as cleaned), without its location.
fn member_of(c: &Cleaned, decl: FileRef) -> Option<Member> {
    let h = c.text.as_str();
    if h.is_empty() {
        return None;
    }
    let base = h.split('=').next().unwrap_or_default();
    let (kind, name, signature, modifiers);
    if base.contains('(') {
        let (pre, args) = h.split_once('(')?;
        let params = args.rsplit_once(')').map_or(args, |(p, _)| p);
        let tokens: Vec<&str> = RE.token.find_iter(pre).map(|m| m.as_str()).collect();
        let (last, before) = tokens.split_last()?;
        let (mut mods, mut ret) = (Vec::new(), Vec::new());
        for t in before {
            if MODS.contains(t) {
                add_modifier(&mut mods, t);
            } else {
                ret.push(*t);
            }
        }
        if let Some(p) = mods.iter().position(|m| m == "native") {
            let native = mods.remove(p);
            mods.push(native);
        }
        let mut sig = String::new();
        for m in &mods {
            sig.push_str(m);
            sig.push(' ');
        }
        if !ret.is_empty() {
            sig.push_str(&short(&ret.join(" ")));
            sig.push(' ');
        }
        sig.push_str(last);
        sig.push('(');
        sig.push_str(&short(params));
        sig.push(')');
        (kind, name, signature, modifiers) = (MemberKind::Method, (*last).to_string(), sig, mods);
    } else {
        let mut mods = Vec::new();
        for t in h.split_whitespace().take_while(|t| MODS.contains(t)) {
            add_modifier(&mut mods, t);
        }
        // Constants keep their value, everything else loses its initializer.
        let sig = if mods.iter().any(|m| m == "const") {
            short(h)
        } else {
            short(base)
        };
        let cut = base.find('[').map_or(base, |p| sub(base, 0, p)).trim_end();
        let field = cut.split_whitespace().next_back().unwrap_or(cut);
        (kind, name, signature, modifiers) = (MemberKind::Field, field.to_string(), sig, mods);
    }
    if !signature.chars().any(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(Member {
        kind,
        name,
        signature,
        modifiers,
        doc: doc_value(c.doc.as_deref()),
        obsolete: c.obsolete,
        decl,
        cond: None,
    })
}

/// What identifies a member of a class: methods ignore `override` and `static`.
fn member_key(m: &Member) -> String {
    match m.kind {
        MemberKind::Method => RE
            .override_static
            .replace_all(&m.signature, "")
            .into_owned(),
        MemberKind::Field => m.signature.clone(),
    }
}

/// Integer literal of an enum item; `None` for anything else (expressions, octal-looking, with underscores).
fn parse_int(v: &str) -> Option<i64> {
    let v = v.trim();
    let (neg, rest) = match v.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let rest = rest.trim_start();
    let (radix, digits) =
        if let Some(d) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            (16, d)
        } else if let Some(d) = rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")) {
            (2, d)
        } else if let Some(d) = rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")) {
            (8, d)
        } else {
            (10, rest)
        };
    if digits.is_empty()
        || !digits.chars().all(|c| c.is_digit(radix))
        || (radix == 10 && digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    let n = i64::from_str_radix(digits, radix).ok()?;
    if neg { n.checked_neg() } else { Some(n) }
}

/// Ranges of the comma or semicolon separated items of an enum body.
fn enum_items(text: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut start) = (from, from);
    while i < to {
        match at(b, i) {
            DOC_START => i = find_byte(b, i, DOC_END).unwrap_or(to),
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b',' | b';' => {
                out.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push((start, to));
    out
}

struct Indexer<'a> {
    into: &'a mut ApiIndex,
    defines: &'a BTreeSet<String>,
    /// Member key to index in `Class::members`, per class, filled when a class is first touched.
    member_keys: HashMap<String, HashMap<String, usize>>,
    typedef_keys: HashMap<String, usize>,
    function_keys: HashMap<String, usize>,
    global_keys: HashMap<String, usize>,
}

/// Where a statement is: file, source text, line table and conditions.
struct Place<'a> {
    file: &'a ScriptFile<'a>,
    text: &'a str,
    /// Offsets of the newlines of `text`.
    newlines: Vec<usize>,
    /// Condition changes of `text`, see `Scanned::changes`.
    changes: &'a [(usize, Option<String>)],
}

/// Add a member unless its key is taken. An active declaration replaces an inactive one.
fn add_member(list: &mut Vec<Member>, keys: &mut HashMap<String, usize>, key: String, m: Member) {
    if let Some(&at) = keys.get(&key) {
        if let Some(old) = list.get_mut(at)
            && old.cond.is_some()
            && m.cond.is_none()
        {
            *old = m;
        }
    } else {
        keys.insert(key, list.len());
        list.push(m);
    }
}

fn index_keys(members: &[Member], key: fn(&Member) -> String) -> HashMap<String, usize> {
    let mut keys = HashMap::new();
    for (i, m) in members.iter().enumerate() {
        keys.entry(key(m)).or_insert(i);
    }
    keys
}

impl Place<'_> {
    fn file_ref(&self, pos: usize) -> FileRef {
        let line = self.newlines.partition_point(|&p| p < pos) + 1;
        FileRef {
            module: self.file.module,
            path: self.file.path.to_string(),
            line: u32::try_from(line).unwrap_or(u32::MAX),
        }
    }

    fn cond_at(&self, pos: usize) -> Option<String> {
        cond_at(self.changes, pos).map(str::to_string)
    }

    /// Text of the header of a statement, without the parts that belong to another condition than its
    /// first token (`void F(int a #ifdef X , int b #endif )` keeps one signature).
    fn header(&self, st: &Stmt, first: usize) -> String {
        let whole = sub(self.text, st.start, st.head_end);
        let from = self.changes.partition_point(|(p, _)| *p <= st.start);
        let inner: Vec<&(usize, Option<String>)> = self
            .changes
            .iter()
            .skip(from)
            .take_while(|(p, _)| *p < st.head_end)
            .collect();
        if inner.is_empty() {
            return whole.to_string();
        }
        let want = self.cond_at(first);
        let mut out = String::with_capacity(whole.len());
        let mut cursor = st.start;
        let mut cond = self.cond_at(st.start);
        for (pos, next) in inner
            .into_iter()
            .map(|c| (c.0, &c.1))
            .chain([(st.head_end, &None)])
        {
            if cursor < first || cond == want {
                out.push_str(sub(self.text, cursor, pos));
            }
            cursor = pos;
            cond.clone_from(next);
        }
        out
    }

    /// The header of a statement as a cleaned text, its location and its condition.
    fn head(&self, st: &Stmt) -> (Cleaned, FileRef, Option<String>) {
        let pos = first_token(self.text.as_bytes(), st.start, st.head_end);
        (
            clean(&self.header(st, pos)),
            self.file_ref(pos),
            self.cond_at(pos),
        )
    }
}

impl Indexer<'_> {
    fn file(&mut self, file: &ScriptFile<'_>) {
        let scanned = scan(file.text, self.defines);
        let newlines = scanned
            .text
            .bytes()
            .enumerate()
            .filter(|&(_, c)| c == b'\n')
            .map(|(p, _)| p)
            .collect();
        let place = Place {
            file,
            text: &scanned.text,
            newlines,
            changes: &scanned.changes,
        };
        let mut declared = BTreeSet::new();
        for st in split(&scanned.text, 0, scanned.text.len()) {
            self.statement(&place, &st, &mut declared);
        }
        for (name, other) in scanned.variants {
            if declared.contains(&name) {
                self.add_alt_base(&name, other);
            }
        }
    }

    fn add_alt_base(&mut self, class: &str, other: String) {
        if let Some(class) = self.into.classes.get_mut(class)
            && class.base_name() != Some(other.split('<').next().unwrap_or(&other))
            && !class.alt_bases.contains(&other)
        {
            class.alt_bases.push(other);
        }
    }

    fn statement(&mut self, place: &Place<'_>, st: &Stmt, declared: &mut BTreeSet<String>) {
        let (c, decl, cond) = place.head(st);
        if c.text.is_empty() {
            return;
        }
        if let Some(caps) = RE.header.captures(&c.text) {
            let get = |k| caps.get(k).map(|m| m.as_str());
            let modded = get(1).is_some_and(|p| p.split_whitespace().any(|w| w == "modded"));
            let name = get(3).unwrap_or_default().to_string();
            let base = get(5).map(short);
            let doc = doc_value(c.doc.as_deref());
            match (get(2), st.body) {
                (Some("enum"), Some((from, to))) => {
                    let h = EnumHeader {
                        name,
                        modded,
                        base,
                        decl,
                        cond,
                    };
                    self.enum_decl(place, h, (from, to));
                }
                (Some("class" | "struct"), body) => {
                    let tparams = get(4)
                        .map(|t| t.trim_matches(['<', '>']).to_string())
                        .unwrap_or_default();
                    let type_params = tparams
                        .split(',')
                        .filter_map(|p| p.split_whitespace().next_back())
                        .map(String::from)
                        .collect();
                    declared.insert(name.clone());
                    let h = ClassHeader {
                        name,
                        modded,
                        type_params,
                        base,
                        doc,
                        decl,
                        cond,
                    };
                    self.class_decl(place, h, body);
                }
                _ => {}
            }
            return;
        }
        if let Some(rest) = c
            .text
            .strip_prefix("typedef")
            .filter(|r| r.starts_with(' '))
        {
            let signature = short(rest);
            let name = signature
                .split_whitespace()
                .next_back()
                .unwrap_or_default()
                .to_string();
            let member = Member {
                kind: MemberKind::Field,
                name,
                signature: signature.clone(),
                modifiers: Vec::new(),
                doc: doc_value(c.doc.as_deref()),
                obsolete: false,
                decl,
                cond,
            };
            add_member(
                &mut self.into.typedefs,
                &mut self.typedef_keys,
                signature,
                member,
            );
            return;
        }
        if let Some(mut member) = member_of(&c, decl) {
            member.cond = cond;
            let (keys, list) = match member.kind {
                MemberKind::Method => (&mut self.function_keys, &mut self.into.functions),
                MemberKind::Field => (&mut self.global_keys, &mut self.into.globals),
            };
            add_member(list, keys, member.signature.clone(), member);
        }
    }

    fn class_decl(&mut self, place: &Place<'_>, h: ClassHeader, body: Option<(usize, usize)>) {
        let mut members = Vec::new();
        if let Some((from, to)) = body {
            for st in split(place.text, from, to) {
                let (c, decl, cond) = place.head(&st);
                // Private members are unreachable from mods.
                let member =
                    member_of(&c, decl).filter(|m| !m.modifiers.iter().any(|x| x == "private"));
                members.extend(member.map(|m| Member { cond, ..m }));
            }
        }
        let active = h.cond.is_none();
        let class = self
            .into
            .classes
            .entry(h.name.clone())
            .or_insert_with(|| Class {
                name: h.name.clone(),
                type_params: Vec::new(),
                base: None,
                alt_bases: Vec::new(),
                modded_only: true,
                doc: None,
                decls: Vec::new(),
                members: Vec::new(),
                cond: h.cond.clone(),
            });
        if !h.modded {
            class.modded_only = false;
        }
        let known = class
            .decls
            .iter()
            .any(|d| d.module == h.decl.module && d.path == h.decl.path);
        if active && class.cond.is_some() {
            // The first active declaration takes over from inactive ones; their base stays as an alternative.
            if let Some(old) = class.base.take()
                && Some(&old) != h.base.as_ref()
                && !class.alt_bases.contains(&old)
            {
                class.alt_bases.push(old);
            }
            class.base = h.base;
            if !h.type_params.is_empty() {
                class.type_params = h.type_params;
            }
            if h.doc.is_some() {
                class.doc = h.doc;
            }
            class.cond = None;
            if !known {
                class.decls.insert(0, h.decl);
            }
        } else {
            if class.base.is_none() {
                class.base = h.base;
            } else if let Some(other) = h.base.filter(|b| !active && Some(b) != class.base.as_ref())
                && !class.alt_bases.contains(&other)
            {
                class.alt_bases.push(other);
            }
            if class.type_params.is_empty() {
                class.type_params = h.type_params;
            }
            if class.doc.is_none() {
                class.doc = h.doc;
            }
            if !known {
                class.decls.push(h.decl);
            }
        }
        let keys = self
            .member_keys
            .entry(h.name)
            .or_insert_with(|| index_keys(&class.members, member_key));
        for m in members {
            add_member(&mut class.members, keys, member_key(&m), m);
        }
    }

    fn enum_decl(&mut self, place: &Place<'_>, h: EnumHeader, body: (usize, usize)) {
        let active = h.cond.is_none();
        let e = self
            .into
            .enums
            .entry(h.name.clone())
            .or_insert_with(|| Enum {
                name: h.name.clone(),
                base: None,
                decls: Vec::new(),
                items: Vec::new(),
                cond: h.cond.clone(),
            });
        let mut skip_items = false;
        if active && e.cond.is_some() {
            // An active declaration replaces what inactive ones contributed.
            e.cond = None;
            e.items.clear();
            e.base = h.base;
            e.decls.insert(0, h.decl);
        } else {
            if e.base.is_none() {
                e.base = h.base;
            }
            e.decls.push(h.decl);
            skip_items = !active && e.cond.is_none();
        }
        if skip_items {
            return;
        }
        // A modded enum continues counting after the last item of the enum it extends.
        let mut value = if h.modded {
            e.items.last().map_or(Some(-1), |i| i.value)
        } else {
            Some(-1)
        };
        for (from, to) in enum_items(place.text, body.0, body.1) {
            // Items of another `#ifdef` branch than the enum itself would shift the values of the others.
            let pos = first_token(place.text.as_bytes(), from, to);
            if place.cond_at(pos) != h.cond {
                continue;
            }
            let c = clean(&place.header(
                &Stmt {
                    start: from,
                    head_end: to,
                    body: None,
                },
                pos,
            ));
            let (item, expr) = c.text.split_once('=').unwrap_or((c.text.as_str(), ""));
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            value = if expr.trim().is_empty() {
                value.and_then(|v| v.checked_add(1))
            } else {
                parse_int(expr)
            };
            e.items.push(EnumItem {
                name: item.to_string(),
                value,
                doc: doc_value(c.doc.as_deref()),
            });
        }
    }
}

struct ClassHeader {
    name: String,
    modded: bool,
    type_params: Vec<String>,
    base: Option<String>,
    doc: Option<String>,
    decl: FileRef,
    cond: Option<String>,
}

struct EnumHeader {
    name: String,
    modded: bool,
    base: Option<String>,
    decl: FileRef,
    cond: Option<String>,
}
