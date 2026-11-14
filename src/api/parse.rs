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
use super::steam::Branch;

/// Longest doc text kept, in characters.
const DOC_MAX: usize = 240;
/// Start and end of a doc comment in the cleaned text.
const DOC_START: u8 = 1;
const DOC_END: u8 = 2;
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
    variant: Regex,
    comma: Regex,
    equals: Regex,
    lt: Regex,
    gt: Regex,
    noise: Regex,
    override_static: Regex,
    brief: Regex,
    tag: Regex,
    copyright: Regex,
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
    variant: Regex::new(r"^\s*(?:modded\s+)?class\s+(\w+)\s*(?:extends|:)\s*([\w.]+)").unwrap(),
    comma: Regex::new(r"\s*,\s*").unwrap(),
    equals: Regex::new(r"\s*=\s*").unwrap(),
    lt: Regex::new(r"\s*<\s*").unwrap(),
    gt: Regex::new(r"\s*>").unwrap(),
    noise: Regex::new(r"\b(?:autoptr|ref|owned|notnull|reference)\s+").unwrap(),
    override_static: Regex::new(r"\b(?:override|static)\s+").unwrap(),
    brief: Regex::new(r"[@\\](?:brief|class)\s*").unwrap(),
    tag: Regex::new(r"[@\\](?:code|param|return|see|details|note)\b").unwrap(),
    copyright: Regex::new(r"©|\(c\)\s*\d{4}|[Cc]opyright|[Ll]icense").unwrap(),
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
        typedef_keys: signatures(&into.typedefs),
        function_keys: signatures(&into.functions),
        global_keys: signatures(&into.globals),
        into,
        defines,
    };
    for file in order {
        indexer.file(file);
    }
}

fn signatures(members: &[Member]) -> HashSet<String> {
    members.iter().map(|m| m.signature.clone()).collect()
}

/// Position of the layer directory in the path; paths without one sort last.
fn layer_rank(path: &str) -> usize {
    path.split(['\\', '/'])
        .find_map(|part| LAYERS.iter().position(|l| *l == part))
        .unwrap_or(LAYERS.len())
}

// ---------------------------------------------------------------------------------------------------------
// Byte helpers: no indexing, out of range reads give 0 or an empty slice.

fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

fn sub(s: &str, from: usize, to: usize) -> &str {
    s.get(from..to).unwrap_or_default()
}

fn find_byte(b: &[u8], from: usize, byte: u8) -> Option<usize> {
    b.get(from..)?
        .iter()
        .position(|&x| x == byte)
        .map(|p| p + from)
}

/// End of the string literal that opens at `i`: just past the closing quote, or at the newline or end of
/// input of an unterminated string.
fn skip_string(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < b.len() {
        match at(b, j) {
            b'"' => return j + 1,
            b'\n' => return j,
            b'\\' => j += 2,
            _ => j += 1,
        }
    }
    b.len()
}

// ---------------------------------------------------------------------------------------------------------
// Scanner

/// One level of `#ifdef` nesting.
struct Cond {
    /// The enclosing region is active.
    parent: bool,
    /// The `#if` condition (or an earlier branch) was true.
    taken: bool,
    /// The current branch is active.
    active: bool,
}

struct Scanned {
    /// Source without comments, preprocessor lines and inactive branches; newlines are kept.
    text: String,
    /// `(class, base)` of `class X extends Y` headers in inactive branches.
    variants: Vec<(String, String)>,
}

fn is_active(conds: &[Cond]) -> bool {
    conds.last().is_none_or(|c| c.active)
}

fn directive(
    line: &str,
    conds: &mut Vec<Cond>,
    seen: &mut BTreeSet<String>,
    defines: &BTreeSet<String>,
) {
    let rest = line.trim_start();
    let word_end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let word = sub(rest, 0, word_end);
    let arg = rest.get(word_end..).unwrap_or_default().trim_start();
    let name = arg
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or_default();
    match word {
        "ifdef" | "ifndef" => {
            let parent = is_active(conds);
            let taken = (defines.contains(name) || seen.contains(name)) == (word == "ifdef");
            conds.push(Cond {
                parent,
                taken,
                active: parent && taken,
            });
        }
        // Expressions are not evaluated: the first branch counts as taken.
        "if" => {
            let parent = is_active(conds);
            conds.push(Cond {
                parent,
                taken: true,
                active: parent,
            });
        }
        "elif" => {
            if let Some(c) = conds.last_mut() {
                c.active = false;
                c.taken = true;
            }
        }
        "else" => {
            if let Some(c) = conds.last_mut() {
                c.active = c.parent && !c.taken;
                c.taken = true;
            }
        }
        "endif" => {
            conds.pop();
        }
        "define" if is_active(conds) && !name.is_empty() => {
            seen.insert(name.to_string());
        }
        _ => {}
    }
}

/// Push a text slice, replacing the doc marker bytes so source text can never forge a marker.
fn push_clean(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend(bytes.iter().map(|&c| {
        if c == DOC_START || c == DOC_END {
            b' '
        } else {
            c
        }
    }));
}

fn push_doc(out: &mut Vec<u8>, text: &str) {
    out.push(DOC_START);
    push_clean(out, text.as_bytes());
    out.push(DOC_END);
}

/// First sentence of a block comment doc; empty when it is a file header (copyright, license, file name).
fn block_doc(content: &str) -> String {
    let mut joined = String::with_capacity(content.len());
    for (n, line) in content.split('\n').enumerate() {
        joined.push(' ');
        joined.push_str(if n == 0 {
            line
        } else {
            line.trim_start().trim_start_matches('*')
        });
    }
    let text = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = RE.brief.replace_all(&text, "");
    let mut cut = text.len();
    let mut chars = text.char_indices().peekable();
    while let Some((idx, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?') && chars.peek().is_some_and(|(_, n)| n.is_whitespace()) {
            cut = idx + 1;
            break;
        }
    }
    if let Some(m) = RE.tag.find(&text) {
        cut = cut.min(m.start());
    }
    let first = sub(&text, 0, cut).trim();
    if RE.copyright.is_match(first)
        || std::path::Path::new(first)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("c"))
    {
        String::new()
    } else {
        first.to_string()
    }
}

fn scan(src: &str, defines: &BTreeSet<String>) -> Scanned {
    let b = src.as_bytes();
    let n = b.len();
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut variants = Vec::new();
    let mut conds: Vec<Cond> = Vec::new();
    let mut seen = BTreeSet::new();
    let (mut i, mut bol) = (0, true);
    while i < n {
        if !is_active(&conds) {
            // Inactive text is skipped line by line; only directives and class headers matter.
            let end = find_byte(b, i, b'\n').unwrap_or(n);
            let line = sub(src, i, end);
            if let Some(d) = line.trim_start().strip_prefix('#') {
                directive(d, &mut conds, &mut seen, defines);
            } else if let Some(c) = RE.variant.captures(line) {
                let get = |k| c.get(k).map_or(String::new(), |m| m.as_str().to_string());
                variants.push((get(1), get(2)));
            }
            if end < n {
                out.push(b'\n');
            }
            i = end + 1;
            bol = true;
            continue;
        }
        let c = at(b, i);
        if bol && (c == b' ' || c == b'\t') {
            out.push(c);
            i += 1;
            continue;
        }
        if bol && c == b'#' {
            let end = find_byte(b, i, b'\n').unwrap_or(n);
            directive(sub(src, i + 1, end), &mut conds, &mut seen, defines);
            i = end;
            continue;
        }
        let own_line = bol;
        bol = c == b'\n';
        if c == b'"' {
            let end = skip_string(b, i);
            push_clean(&mut out, b.get(i..end).unwrap_or_default());
            i = end;
        } else if c == b'/' && at(b, i + 1) == b'/' {
            let end = find_byte(b, i, b'\n').unwrap_or(n);
            // A doc comment after code on the same line documents nothing that follows.
            if own_line
                && src
                    .get(i..)
                    .is_some_and(|s| s.starts_with("//!") && !s.starts_with("//!<"))
            {
                let text: String = sub(src, i + 3, end).trim().chars().take(DOC_MAX).collect();
                push_doc(&mut out, &text);
            }
            i = end;
        } else if c == b'/' && at(b, i + 1) == b'*' {
            let close = b
                .get(i + 2..)
                .and_then(|r| r.windows(2).position(|w| w == b"*/"))
                .map(|p| p + i + 2);
            let end = close.unwrap_or(n);
            let is_doc = src
                .get(i..)
                .is_some_and(|s| s.starts_with("/*!") || s.starts_with("/**"));
            if own_line && is_doc && end >= i + 3 {
                let text: String = block_doc(sub(src, i + 3, end))
                    .chars()
                    .take(DOC_MAX)
                    .collect();
                push_doc(&mut out, &text);
            }
            let stop = (end + 2).min(n);
            let lines = sub(src, i, stop).matches('\n').count();
            out.extend(std::iter::repeat_n(b'\n', lines));
            i = stop;
        } else {
            push_clean(&mut out, &[c]);
            i += 1;
        }
    }
    Scanned {
        text: String::from_utf8_lossy(&out).into_owned(),
        variants,
    }
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
    /// Member keys per class, filled when a class is first touched.
    member_keys: HashMap<String, HashSet<String>>,
    typedef_keys: HashSet<String>,
    function_keys: HashSet<String>,
    global_keys: HashSet<String>,
}

/// Where a statement is: file, source text and line table.
struct Place<'a> {
    file: &'a ScriptFile<'a>,
    text: &'a str,
    /// Offsets of the newlines of `text`.
    newlines: Vec<usize>,
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

    /// The header of a statement as a cleaned text and its location.
    fn head(&self, st: &Stmt) -> (Cleaned, FileRef) {
        let pos = first_token(self.text.as_bytes(), st.start, st.head_end);
        (
            clean(sub(self.text, st.start, st.head_end)),
            self.file_ref(pos),
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
        };
        let mut declared = BTreeSet::new();
        for st in split(&scanned.text, 0, scanned.text.len()) {
            self.statement(&place, &st, &mut declared);
        }
        for (name, other) in scanned.variants {
            if !declared.contains(&name) {
                continue;
            }
            if let Some(class) = self.into.classes.get_mut(&name)
                && class.base_name() != Some(other.as_str())
                && !class.alt_bases.contains(&other)
            {
                class.alt_bases.push(other);
            }
        }
    }

    fn statement(&mut self, place: &Place<'_>, st: &Stmt, declared: &mut BTreeSet<String>) {
        let (c, decl) = place.head(st);
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
                    self.enum_decl(place, &name, modded, base, decl, (from, to));
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
                    self.class_decl(
                        place,
                        ClassHeader {
                            name,
                            modded,
                            type_params,
                            base,
                            doc,
                            decl,
                        },
                        body,
                    );
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
                signature,
                modifiers: Vec::new(),
                doc: doc_value(c.doc.as_deref()),
                obsolete: false,
                decl,
            };
            if self.typedef_keys.insert(member.signature.clone()) {
                self.into.typedefs.push(member);
            }
            return;
        }
        if let Some(member) = member_of(&c, decl) {
            let (keys, list) = match member.kind {
                MemberKind::Method => (&mut self.function_keys, &mut self.into.functions),
                MemberKind::Field => (&mut self.global_keys, &mut self.into.globals),
            };
            if keys.insert(member.signature.clone()) {
                list.push(member);
            }
        }
    }

    fn class_decl(&mut self, place: &Place<'_>, h: ClassHeader, body: Option<(usize, usize)>) {
        let mut members = Vec::new();
        if let Some((from, to)) = body {
            for st in split(place.text, from, to) {
                let (c, decl) = place.head(&st);
                // Private members are unreachable from mods.
                members.extend(
                    member_of(&c, decl).filter(|m| !m.modifiers.iter().any(|x| x == "private")),
                );
            }
        }
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
            });
        if !h.modded {
            class.modded_only = false;
        }
        if class.base.is_none() {
            class.base = h.base;
        }
        if class.type_params.is_empty() {
            class.type_params = h.type_params;
        }
        if class.doc.is_none() {
            class.doc = h.doc;
        }
        if !class
            .decls
            .iter()
            .any(|d| d.module == h.decl.module && d.path == h.decl.path)
        {
            class.decls.push(h.decl);
        }
        let keys = self
            .member_keys
            .entry(h.name)
            .or_insert_with(|| class.members.iter().map(member_key).collect());
        for m in members {
            if keys.insert(member_key(&m)) {
                class.members.push(m);
            }
        }
    }

    fn enum_decl(
        &mut self,
        place: &Place<'_>,
        name: &str,
        modded: bool,
        base: Option<String>,
        decl: FileRef,
        body: (usize, usize),
    ) {
        let e = self
            .into
            .enums
            .entry(name.to_string())
            .or_insert_with(|| Enum {
                name: name.to_string(),
                base: None,
                decls: Vec::new(),
                items: Vec::new(),
            });
        if e.base.is_none() {
            e.base = base;
        }
        e.decls.push(decl);
        // A modded enum continues counting after the last item of the enum it extends.
        let mut value = if modded {
            e.items.last().map_or(Some(-1), |i| i.value)
        } else {
            Some(-1)
        };
        for (from, to) in enum_items(place.text, body.0, body.1) {
            let c = clean(sub(place.text, from, to));
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
}
