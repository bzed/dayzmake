//! Scanner of Enforce Script sources: removes comments and preprocessor lines, keeps doc comments as markers,
//! evaluates `#ifdef` conditions and tags text of inactive branches with their condition.
//!
//! Every newline of the source is kept, so an offset in the cleaned text maps to its source line.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

/// Longest doc text kept, in characters.
pub(super) const DOC_MAX: usize = 240;
/// Start and end of a doc comment in the cleaned text.
pub(super) const DOC_START: u8 = 1;
pub(super) const DOC_END: u8 = 2;

struct Patterns {
    variant: Regex,
    brief: Regex,
    tag: Regex,
    copyright: Regex,
    condition: Regex,
}

#[expect(clippy::unwrap_used, reason = "the static patterns are valid regexes")]
static RE: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    variant: Regex::new(r"^\s*(?:modded\s+)?class\s+(\w+)\s*(?:extends|:)\s*([\w.]+)").unwrap(),
    brief: Regex::new(r"[@\\](?:brief|class)\s*").unwrap(),
    tag: Regex::new(r"[@\\](?:code|param|return|see|details|note)\b").unwrap(),
    copyright: Regex::new(r"©|\(c\)\s*\d{4}|[Cc]opyright|[Ll]icense").unwrap(),
    condition: Regex::new(
        r"^(?:defined\s*\(\s*([A-Za-z_]\w*)\s*\)|defined\s+([A-Za-z_]\w*)|([A-Za-z_]\w*))$",
    )
    .unwrap(),
});

// Byte helpers: no indexing, out of range reads give 0 or an empty slice.

pub(super) fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

pub(super) fn sub(s: &str, from: usize, to: usize) -> &str {
    s.get(from..to).unwrap_or_default()
}

pub(super) fn find_byte(b: &[u8], from: usize, byte: u8) -> Option<usize> {
    b.get(from..)?
        .iter()
        .position(|&x| x == byte)
        .map(|p| p + from)
}

/// End of the string literal that opens at `i`: just past the closing quote, or at the newline or end of
/// input of an unterminated string.
pub(super) fn skip_string(b: &[u8], i: usize) -> usize {
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

/// One level of `#ifdef` nesting.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four flags are the state of one preprocessor branch"
)]
struct Cond {
    /// The enclosing region is active.
    parent: bool,
    /// The condition of this branch alone is true.
    own: bool,
    /// This or an earlier branch was true.
    taken: bool,
    /// The current branch is active.
    active: bool,
    /// The condition of the opening `#ifdef`, e.g. `X` or `!X`.
    open: String,
    /// The condition of the current branch as written.
    text: String,
}

/// Result of a scan.
pub(super) struct Scanned {
    /// Source without comments and preprocessor lines; newlines are kept.
    pub text: String,
    /// `(class, base)` of `class X extends Y` headers that stand alone in an inactive branch.
    pub variants: Vec<(String, String)>,
    /// Offsets where the condition of the text changes: the innermost inactive condition, `None` when active.
    pub changes: Vec<(usize, Option<String>)>,
}

/// The inactive condition that applies at `pos`, if any.
pub(super) fn cond_at(changes: &[(usize, Option<String>)], pos: usize) -> Option<&str> {
    let n = changes.partition_point(|(p, _)| *p <= pos);
    changes.get(n.checked_sub(1)?)?.1.as_deref()
}

fn is_active(conds: &[Cond]) -> bool {
    conds.last().is_none_or(|c| c.active)
}

fn negate(text: &str) -> String {
    text.strip_prefix('!')
        .map_or_else(|| format!("!{text}"), str::to_string)
}

/// Evaluate `X`, `defined(X)` and `defined X`. Anything more complex (`&&`, `||`, `!`, numbers) is not
/// evaluated: the branch then counts as true, so it is active unless an earlier branch was taken.
fn evaluate(expr: &str, is_defined: impl Fn(&str) -> bool) -> bool {
    let name = RE.condition.captures(expr.trim()).and_then(|c| {
        c.iter()
            .skip(1)
            .flatten()
            .next()
            .map(|m| m.as_str().to_string())
    });
    name.is_none_or(|n| is_defined(&n))
}

/// A condition as shown: the name for `X`, `defined(X)` and `defined X`, else the text as written.
fn cond_name(expr: &str) -> String {
    let expr = expr.trim();
    RE.condition
        .captures(expr)
        .and_then(|c| {
            c.iter()
                .skip(1)
                .flatten()
                .next()
                .map(|m| m.as_str().to_string())
        })
        .unwrap_or_else(|| expr.to_string())
}

struct State<'a> {
    out: Vec<u8>,
    variants: Vec<(String, String)>,
    conds: Vec<Cond>,
    seen: BTreeSet<String>,
    defines: &'a BTreeSet<String>,
    changes: Vec<(usize, Option<String>)>,
    current: Option<String>,
    /// Start of the output and length of `changes` when the current inactive region began.
    region: Option<(usize, usize)>,
}

impl State<'_> {
    fn directive(&mut self, line: &str) {
        let rest = line.trim_start();
        let word_end = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let word = sub(rest, 0, word_end);
        let arg = rest.get(word_end..).unwrap_or_default().trim();
        let name = arg
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or_default();
        let defined = |n: &str| self.defines.contains(n) || self.seen.contains(n);
        let parent = is_active(&self.conds);
        match word {
            "ifdef" | "ifndef" => {
                let own = defined(name) == (word == "ifdef");
                let text = if word == "ifdef" {
                    name.to_string()
                } else {
                    negate(name)
                };
                self.conds.push(Cond {
                    parent,
                    own,
                    taken: own,
                    active: parent && own,
                    open: text.clone(),
                    text,
                });
            }
            "if" => {
                let own = evaluate(arg, defined);
                let text = cond_name(arg);
                self.conds.push(Cond {
                    parent,
                    own,
                    taken: own,
                    active: parent && own,
                    open: text.clone(),
                    text,
                });
            }
            "elif" => {
                let own = evaluate(arg, defined);
                if let Some(c) = self.conds.last_mut() {
                    c.active = c.parent && !c.taken && own;
                    c.own = own;
                    c.taken |= own;
                    c.text = cond_name(arg);
                }
            }
            "else" => {
                if let Some(c) = self.conds.last_mut() {
                    c.own = !c.taken;
                    c.active = c.parent && !c.taken;
                    c.taken = true;
                    c.text = negate(&c.open);
                }
            }
            "endif" => {
                self.conds.pop();
            }
            "define" if parent && !name.is_empty() => {
                self.seen.insert(name.to_string());
            }
            _ => {}
        }
        self.sync();
    }

    /// Track the start and end of inactive regions and the condition of the text that follows.
    fn sync(&mut self) {
        let active = is_active(&self.conds);
        if self.region.is_none() && !active {
            self.region = Some((self.out.len(), self.changes.len()));
        } else if self.region.is_some() && active {
            self.close_region();
        }
        let cond = self
            .conds
            .iter()
            .rev()
            .find(|c| !c.own || (!c.active && c.parent))
            .map(|c| c.text.clone());
        if cond != self.current {
            self.changes.push((self.out.len(), cond.clone()));
            self.current = cond;
        }
    }

    /// An inactive region whose braces do not balance (`#ifdef X void f() { #else void f(int a) { #endif`) would
    /// corrupt the statements around it and is dropped; its class headers still count as alternative bases.
    fn close_region(&mut self) {
        let Some((start, kept)) = self.region.take() else {
            return;
        };
        if braces_balanced(self.out.get(start..).unwrap_or_default()) {
            return;
        }
        let dropped = self.out.split_off(start);
        let text = String::from_utf8_lossy(&dropped);
        for line in text.lines() {
            if let Some(c) = RE.variant.captures(line) {
                let get = |k| c.get(k).map_or(String::new(), |m| m.as_str().to_string());
                self.variants.push((get(1), get(2)));
            }
        }
        self.out
            .extend(std::iter::repeat_n(b'\n', text.matches('\n').count()));
        self.changes.truncate(kept);
        self.current = self.changes.last().and_then(|c| c.1.clone());
    }
}

/// Braces never close more than they opened, and end even.
fn braces_balanced(b: &[u8]) -> bool {
    let (mut i, mut depth) = (0, 0usize);
    while i < b.len() {
        match at(b, i) {
            DOC_START => i = find_byte(b, i, DOC_END).unwrap_or(b.len()),
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b'{' => depth += 1,
            b'}' => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            _ => {}
        }
        i += 1;
    }
    depth == 0
}

/// The next line that is not blank or a plain comment starts with `#`.
fn next_line_is_directive(src: &str, from: usize) -> bool {
    for line in src.get(from..).unwrap_or_default().lines().skip(1) {
        let t = line.trim();
        if t.is_empty() || (t.starts_with("//") && !t.starts_with("//!")) {
            continue;
        }
        return t.starts_with('#');
    }
    false
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

/// Scan a source file. Text of inactive branches stays in the output, tagged in `changes`.
pub(super) fn scan(src: &str, defines: &BTreeSet<String>) -> Scanned {
    let b = src.as_bytes();
    let n = b.len();
    let mut st = State {
        out: Vec::with_capacity(n),
        variants: Vec::new(),
        conds: Vec::new(),
        seen: BTreeSet::new(),
        defines,
        changes: Vec::new(),
        current: None,
        region: None,
    };
    let (mut i, mut bol) = (0, true);
    while i < n {
        let c = at(b, i);
        if bol && (c == b' ' || c == b'\t') {
            st.out.push(c);
            i += 1;
            continue;
        }
        if bol && c == b'#' {
            let end = find_byte(b, i, b'\n').unwrap_or(n);
            st.directive(sub(src, i + 1, end));
            i = end;
            continue;
        }
        if bol && !is_active(&st.conds) {
            // A class header that stands alone in an inactive branch is the other half of
            // `#ifdef X class A extends B #else class A extends C #endif`: only its base is of interest.
            let end = find_byte(b, i, b'\n').unwrap_or(n);
            if let Some(v) = RE
                .variant
                .captures(sub(src, i, end))
                .filter(|_| next_line_is_directive(src, end))
            {
                let get = |k| v.get(k).map_or(String::new(), |m| m.as_str().to_string());
                st.variants.push((get(1), get(2)));
                i = end;
                continue;
            }
        }
        let own_line = bol;
        bol = c == b'\n';
        if c == b'"' {
            let end = skip_string(b, i);
            push_clean(&mut st.out, b.get(i..end).unwrap_or_default());
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
                push_doc(&mut st.out, &text);
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
                push_doc(&mut st.out, &text);
            }
            let stop = (end + 2).min(n);
            let lines = sub(src, i, stop).matches('\n').count();
            st.out.extend(std::iter::repeat_n(b'\n', lines));
            i = stop;
        } else {
            push_clean(&mut st.out, &[c]);
            i += 1;
        }
    }
    st.close_region();
    Scanned {
        text: String::from_utf8_lossy(&st.out).into_owned(),
        variants: st.variants,
        changes: st.changes,
    }
}
