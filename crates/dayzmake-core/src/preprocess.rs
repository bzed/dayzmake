//! Functions for preprocessing Arma configs and scripts

use std::cell::Cell;
use std::clone::Clone;
use std::collections::HashMap;
use std::env::current_dir;
use std::ffi::OsStr;
use std::fs::{File, read_dir};
use std::io::{Error, Read};
use std::path::{Component, Path, PathBuf};

use crate::error::{ErrorExt, PreprocessParseErrorExt, printable};
use crate::warning::{Warning, WarningSink};

peg::parser! {
    /// PEG grammar for the preprocessor: directives, macros, comments and tokens. `deep` is set when
    /// macro argument parentheses nest deeper than [`MAX_NESTING_DEPTH`]; the caller turns that into an error.
    pub(crate) grammar preprocess_grammar(deep: &Cell<bool>) for str {
        rule newline() = "\r\n" / "\n"

        rule name() -> String = n:$(['a'..='z' | 'A'..='Z' | '0'..='9' | '_']+) {
            n.to_string()
        }

        rule include_path() -> String =
            "\"" path:$((!['"'] [_])*) "\"" { path.to_string() } /
            "<" path:$((!['>'] [_])*) ">"   { path.to_string() }

        rule parameters() -> Vec<String> = "(" [' ' | '\t']* p:(name() ** ([' ' | '\t']* "," [' ' | '\t']*)) [' ' | '\t']* ")" {
            p
        }

        rule definition_value() -> Vec<Token> = ([' ' | '\t']+ / &("\\" newline())) v:(token()*) {
            v
        }

        rule definition() -> Definition = n:name() p:parameters()? v:definition_value()? {
            Definition {
                name: n,
                parameters: p,
                value: v.unwrap_or_default(),
                local: false
            }
        }

        rule directive() -> Directive =
            "#" [' ' | '\t']* "include" [' ' | '\t']+ path:include_path() { Directive::IncludeDirective(path) } /
            "#" [' ' | '\t']* "define" [' ' | '\t']+ d:definition() { Directive::DefineDirective(d) } /
            "#" [' ' | '\t']* "undef" [' ' | '\t']+ n:name() { Directive::UndefDirective(n) } /
            "#" [' ' | '\t']* "ifdef" [' ' | '\t']+ n:name() { Directive::IfDefDirective(n) } /
            "#" [' ' | '\t']* "ifndef" [' ' | '\t']+ n:name() { Directive::IfNDefDirective(n) } /
            "#" [' ' | '\t']* "else" { Directive::ElseDirective } /
            "#" [' ' | '\t']* "endif" { Directive::EndIfDirective }

        rule nesting_ok(d: usize) = {?
            if deep.get() || d >= MAX_NESTING_DEPTH {
                deep.set(true);
                Err("parentheses nested within the limit")
            } else {
                Ok(())
            }
        }

        rule arg_rec(d: usize) = "(" nesting_ok(d) (arg_rec(d + 1) / "\\\\" / ("\\" newline()) / !['\r' | '\n' | '(' | ')'] [_])* ")"

        rule argument() -> String = a:$((arg_rec(0) / "\\\\" / ("\\" newline()) / !['\r' | '\n' | ',' | ')'] [_])*) {
            a.to_string()
        }

        /// Parses a macro argument list: `(a, b, c)`.
        pub rule arguments() -> Vec<String> = [' ' | '\t']* "(" [' ' | '\t']* a:(argument() ** ([' ' | '\t']* "," [' ' | '\t']*)) [' ' | '\t']* ")" {
            a
        }

        rule nonmacro_token() -> String = s:$((!macro_proper() !comment_token() !concat_token() ("\\\\" / ("\\" newline()) / !['"' | '\r' | '\n'] [_]))+) {
            s.to_string()
        }

        rule string_token() -> (String, u32) =
                s:$("\"" ("\\\\" / ("\\" newline()) / "\"\"" / !['\r' | '\n' | '"'] [_])* "\"") {
            let newlines =
                u32::try_from(s.chars().filter(|c| c == &'\n').count()).unwrap_or(u32::MAX);
            (String::from(s).replace("\r\n", "\n").replace("\\\n", ""), newlines)
        }

        /// Parses a macro invocation: an optional `#`, a name, optional arguments.
        pub rule macro_proper() -> Macro = quoted:"#"? n:name() args:arguments()? {
            Macro {
                name: n,
                arguments: args,
                original: String::new(),
                quoted: quoted.is_some()
            }
        }

        rule macro_token() -> Macro = original:$(macro_proper()) {?
            parse_macro(original).map_err(|_| "a parseable macro invocation")
        }

        rule concat_token() = "##"

        rule sl_comment() -> u32 = "//" (!newline() [_])* &newline() {
            0
        }

        rule ml_comment() -> u32 = [' ' | '\t']* "/*" content:$((!"*/" (newline() / [_]))*) "*/" {
            u32::try_from(content.chars().filter(|c| c == &'\n').count()).unwrap_or(u32::MAX)
        }

        rule comment_token() -> u32 = sl_comment() / ml_comment()

        rule token() -> Token =
            c:comment_token() { Token::CommentToken(c) } /
            sn:string_token() { Token::NewlineToken(sn.0, sn.1) } /
            concat_token() { Token::ConcatToken } /
            m:macro_token() { Token::MacroToken(m) } /
            nm:nonmacro_token() { Token::RegularToken(nm) }

        /// Parses a sequence of tokens.
        pub rule tokens() -> Vec<Token> = t:(token()*) {
            t
        }

        rule line() -> Line =
            [' ' | '\t']* d:directive() cmts:(([' ' | '\t']* c:comment_token() { c })*) [' ' | '\t']* {
                Line::DirectiveLine(d, cmts.iter().sum())
            } /
            [' ' | '\t']* t:tokens() { Line::TokenLine(t) }

        /// Parses a whole file into lines.
        pub rule file() -> Vec<Line> = lines:(line() ** (newline())) ![_] {
            lines
        }
    }
}

/// Macro definition
#[derive(Clone, Debug)]
pub(crate) struct Definition {
    name: String,
    parameters: Option<Vec<String>>,
    value: Vec<Token>,
    local: bool,
}

/// Preprocessor directive
#[derive(Debug)]
#[expect(
    clippy::enum_variant_names,
    reason = "the variants are named after the directives they stand for"
)]
pub(crate) enum Directive {
    /// `#include` directive containing the given path
    IncludeDirective(String),
    /// `#define` directive containing the definition
    DefineDirective(Definition),
    /// `#undef` directive containing the name of the macro
    UndefDirective(String),
    /// `#ifdef` directive containing the name of the macro
    IfDefDirective(String),
    /// `#ifndef` directive containing the name of the macro
    IfNDefDirective(String),
    /// `#else` directive
    ElseDirective,
    /// `#endif` directive
    EndIfDirective,
}

/// Potential macro invocation
#[derive(Debug)]
pub(crate) struct Macro {
    name: String,
    arguments: Option<Vec<String>>,
    original: String,
    quoted: bool,
}

/// Preprocessor token
#[derive(Debug)]
#[expect(
    clippy::enum_variant_names,
    reason = "the variants are named after the tokens they stand for"
)]
pub(crate) enum Token {
    /// Non-macro token
    RegularToken(String),
    /// Non-macro token that contains a number of newlines
    NewlineToken(String, u32),
    /// Potential macro token
    MacroToken(Macro),
    /// Comment token containing a number of newlines
    CommentToken(u32),
    /// Token for the concatenation operator (`##`)
    ConcatToken,
}

/// Preprocessor line
#[derive(Debug)]
pub(crate) enum Line {
    /// Directive line
    DirectiveLine(Directive, u32),
    /// Non-directive line of tokens
    TokenLine(Vec<Token>),
}

/// Struct for additional information about preprocessor output. Contains import stack used for
/// loop detection and the origins of all the lines in the output.
#[derive(Debug)]
pub struct PreprocessInfo {
    /// For every line in the output, `line_origins` contains a line number (starting at 1) and a
    /// `PathBuf` to the file where the line was found. The path may be `None` if the line was in the
    /// original input to `preprocess` and `origin` was not given.
    pub(crate) line_origins: Vec<(u32, Option<PathBuf>)>,
    import_stack: Vec<PathBuf>,
    /// Number of `#include`s currently being processed (the root file does not count).
    include_depth: usize,
    /// Tokens produced by macro expansion so far, over the file and its includes.
    expanded: usize,
    /// Bytes of token text copied by macro expansion so far.
    expanded_bytes: usize,
    /// `#include`s processed so far, over the file and its includes.
    includes: usize,
}

impl PreprocessInfo {
    /// For every line in the output, a line number (starting at 1) and the file where the line was
    /// found. The path is `None` if the line was in the original input to `preprocess` and no
    /// path was given.
    #[must_use]
    pub fn line_origins(&self) -> &[(u32, Option<PathBuf>)] {
        &self.line_origins
    }
}

/// Maximum nesting of brackets in the grammars: macro argument parentheses, config classes, config
/// arrays and `__EVAL` parentheses and operator chains. Deeper input is a parse error, not a stack
/// overflow. The generated parsers use up to about 5 KiB of stack per level in a debug build, so this
/// fits the 1 MiB main-thread stack of Windows (`tests/nesting.rs` runs on such a stack); it equals the
/// limit of the rapified reader (`MAX_RAPIFIED_DEPTH`), and real configs nest fewer than 20 levels.
pub(crate) const MAX_NESTING_DEPTH: usize = 128;

/// The error for input nested deeper than [`MAX_NESTING_DEPTH`].
pub(crate) fn nesting_error() -> Error {
    error!("Nesting is deeper than {MAX_NESTING_DEPTH} levels")
}

/// Runs a grammar entry point with a fresh nesting flag; a parse that hit the limit is the nesting
/// error, whatever the parser reported.
fn parse_checked<T, E>(
    parse: impl FnOnce(&Cell<bool>) -> Result<T, E>,
    on_error: impl FnOnce(E) -> Error,
) -> Result<T, Error> {
    let deep = Cell::new(false);
    let result = parse(&deep);
    if deep.get() {
        return Err(nesting_error());
    }
    result.map_err(on_error)
}

/// Maximum nesting of macro expansions; deeper input is an error, not a stack overflow.
const MAX_EXPANSION_DEPTH: usize = 64;

/// Maximum tokens one source line may produce while its macros expand (every token copied at any level
/// counts). Real configs stay far below it (see the #57 measurements in the README); an exponential
/// definition chain hits it after a few milliseconds instead of exhausting memory.
const MAX_LINE_EXPANSION: usize = 500_000;

/// Maximum tokens all lines of one `preprocess` call (the includes count too) may produce.
const MAX_FILE_EXPANSION: usize = 16_000_000;

/// Maximum bytes of token text one source line may copy while its macros expand. Stringification
/// collapses many tokens into one long string, so the token count alone does not bound memory.
const MAX_LINE_BYTES: usize = 64 << 20;

/// Maximum bytes copied by the lines of one `preprocess` call (the includes count too).
const MAX_FILE_BYTES: usize = 512 << 20;

/// Maximum macro resolution steps (every call of a defined macro, argument pre-expansion included) of one source line. A step
/// that produces nothing still costs, so expansion that drops its output cannot run exponentially (#59).
const MAX_LINE_STEPS: usize = 100_000;

/// Counts the tokens and bytes macro expansion produces, so that exponential expansion stops early.
struct Budget {
    tokens: usize,
    bytes: usize,
    steps: usize,
}

impl Budget {
    fn new() -> Budget {
        Budget {
            tokens: 0,
            bytes: 0,
            steps: 0,
        }
    }

    /// Charges one resolution step of the macro `name`.
    fn step(&mut self, name: &str) -> Result<(), Error> {
        self.steps = self.steps.saturating_add(1);
        if self.steps > MAX_LINE_STEPS {
            return Err(error!(
                "Macro \"{}\" expands to more than {MAX_LINE_STEPS} macro calls in one line",
                printable(name)
            ));
        }
        self.spend(1, name.len().max(1), name)
    }

    /// Charges `tokens` tokens and `bytes` bytes of text, produced by the expansion of `name`.
    fn spend(&mut self, tokens: usize, bytes: usize, name: &str) -> Result<(), Error> {
        self.tokens = self.tokens.saturating_add(tokens);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.tokens > MAX_LINE_EXPANSION || self.bytes > MAX_LINE_BYTES {
            return Err(error!(
                "Macro \"{}\" expands to more than {MAX_LINE_EXPANSION} tokens or {MAX_LINE_BYTES} bytes in one line",
                printable(name)
            ));
        }
        Ok(())
    }
}

/// The bytes a token costs: its text, at least 1.
fn token_bytes(token: &Token) -> usize {
    match token {
        Token::RegularToken(s) | Token::NewlineToken(s, _) => s.len().max(1),
        Token::MacroToken(m) => m.original.len().max(1),
        Token::CommentToken(_) | Token::ConcatToken => 1,
    }
}

/// Maximum nesting of `#include`s; deeper input is an error, not a stack overflow.
const MAX_INCLUDE_DEPTH: usize = 32;

/// Maximum `#include`s processed by one `preprocess` call, nested includes counted (every inclusion of
/// the same file counts). Bounds exponential include fan-out (#60), which no token budget sees.
const MAX_INCLUDES: usize = 10_000;

/// The file as the cycle check compares it: canonical when the file system can say, else as given.
fn identity(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn parse_macro(input: &str) -> Result<Macro, Error> {
    let without_original = parse_checked(
        |deep| preprocess_grammar::macro_proper(input, deep),
        |e| error!("Failed to parse macro invocation \"{input}\": {e}"),
    )?;

    Ok(Macro {
        original: String::from(input),
        ..without_original
    })
}

impl Clone for Macro {
    fn clone(&self) -> Macro {
        Macro {
            name: self.name.clone(),
            arguments: self.arguments.clone(),
            original: self.original.clone(),
            quoted: self.quoted,
        }
    }
}

impl Clone for Token {
    fn clone(&self) -> Token {
        match self {
            Token::RegularToken(s) => Token::RegularToken(s.clone()),
            Token::NewlineToken(s, n) => Token::NewlineToken(s.clone(), *n),
            Token::MacroToken(m) => Token::MacroToken(m.clone()),
            Token::CommentToken(n) => Token::CommentToken(*n),
            Token::ConcatToken => Token::ConcatToken,
        }
    }
}

impl Definition {
    fn value(
        &self,
        arguments: Option<&[String]>,
        def_map: &HashMap<String, Definition>,
        stack: &[Definition],
        depth: usize,
        mismatches: &mut Vec<String>,
        budget: &mut Budget,
    ) -> Result<Option<Vec<Token>>, Error> {
        let params = self.parameters.clone().unwrap_or_default();
        let args = arguments.unwrap_or(&[]);

        let empty_call = params.is_empty()
            && args.len() == 1
            && args.first().is_some_and(|a| a.trim().is_empty());
        if params.len() != args.len() && !empty_call {
            return Ok(None);
        }

        let mut tokens = self.value.clone();

        if stack.iter().any(|d| d.name == self.name) {
            return Ok(Some(tokens));
        }

        let mut stack_new: Vec<Definition> = stack.to_vec();
        stack_new.push(self.clone());

        if params.is_empty() {
            tokens = Macro::resolve_all(&tokens, def_map, &stack_new, depth, mismatches, budget)?;
        } else {
            let mut local_map: HashMap<String, Definition> = HashMap::new();

            for (key, value) in def_map {
                local_map.insert(key.clone(), value.clone());
            }

            // @todo: handle these errors properly
            for (param, arg) in params.iter().zip(args.iter()) {
                let mut tokens = parse_checked(
                    |deep| preprocess_grammar::tokens(arg, deep),
                    |e| error!("Failed to parse macro argument: {e}"),
                )?;
                let stack: Vec<Definition> = Vec::new();
                tokens = Macro::resolve_all(&tokens, def_map, &stack, depth, mismatches, budget)?;

                local_map.insert(
                    param.clone(),
                    Definition {
                        name: param.clone(),
                        parameters: None,
                        value: tokens,
                        local: true,
                    },
                );
            }

            tokens =
                Macro::resolve_all(&tokens, &local_map, &stack_new, depth, mismatches, budget)?;
        }

        Ok(Some(tokens))
    }
}

impl Macro {
    fn resolve_pseudoargs(
        &self,
        def_map: &HashMap<String, Definition>,
        stack: &[Definition],
        depth: usize,
        mismatches: &mut Vec<String>,
        budget: &mut Budget,
    ) -> Result<Vec<Token>, Error> {
        let mut tokens: Vec<Token> = Vec::new();
        tokens.push(Token::RegularToken(self.name.clone()));

        if self.arguments.is_none() {
            return Ok(tokens);
        }

        let (_, without_name) = self.original.split_at(self.name.len());
        let mut arg_tokens = parse_checked(
            |deep| preprocess_grammar::tokens(without_name, deep),
            |e| error!("Failed to parse macro arguments: {e}"),
        )?;

        arg_tokens = Macro::resolve_all(&arg_tokens, def_map, stack, depth, mismatches, budget)?;
        for t in arg_tokens {
            tokens.push(t);
        }

        Ok(tokens)
    }

    fn resolve(
        &self,
        def_map: &HashMap<String, Definition>,
        stack: &[Definition],
        depth: usize,
        mismatches: &mut Vec<String>,
        budget: &mut Budget,
    ) -> Result<Vec<Token>, Error> {
        // Every recursive path (body, argument pre-expansion, pseudo-arguments, local parameters) goes through
        // here, so this one counter bounds them all; the `stack` check alone is reset by argument pre-expansion.
        if depth >= MAX_EXPANSION_DEPTH {
            return Err(error!(
                "Macro \"{}\" expands too deep (limit {MAX_EXPANSION_DEPTH})",
                printable(&self.name)
            ));
        }
        let depth = depth + 1;
        match def_map.get(&self.name) {
            Some(def) => {
                budget.step(&self.name)?;
                if let Some(args) = &self.arguments {
                    if def.parameters.is_none() {
                        // Object-like macro followed by parentheses: expand it, keep the parentheses.
                        let mut tokens = def
                            .value(None, def_map, stack, depth, mismatches, budget)?
                            .unwrap_or_default();
                        let (_, rest) = self.original.split_at(self.name.len());
                        let mut rest_tokens = parse_checked(
                            |deep| preprocess_grammar::tokens(rest, deep),
                            |e| error!("Failed to parse macro arguments: {e}"),
                        )?;
                        rest_tokens = Macro::resolve_all(
                            &rest_tokens,
                            def_map,
                            stack,
                            depth,
                            mismatches,
                            budget,
                        )?;
                        tokens.extend(rest_tokens);
                        return Ok(tokens);
                    }

                    let expected = def.parameters.as_ref().map_or(0, std::vec::Vec::len);
                    let got = if expected == 0
                        && args.len() == 1
                        && args.first().is_some_and(|a| a.trim().is_empty())
                    {
                        0
                    } else {
                        args.len()
                    };
                    if got != expected {
                        let msg = format!(
                            "Macro \"{}\" expects {} argument(s), got {}",
                            printable(&self.name),
                            expected,
                            got
                        );
                        mismatches.push(msg);
                        return Ok(if self.quoted {
                            vec![Token::NewlineToken("\"\"".to_string(), 0)]
                        } else {
                            Vec::new()
                        });
                    }
                }

                let value = def.value(
                    self.arguments.as_deref(),
                    def_map,
                    stack,
                    depth,
                    mismatches,
                    budget,
                )?;

                if !def.local && self.quoted {
                    // @todo: complain
                }

                if let Some(tokens) = value {
                    if self.quoted {
                        // charge before `concat` allocates the collapsed string
                        budget.spend(
                            tokens.len(),
                            tokens.iter().map(token_bytes).sum(),
                            &self.name,
                        )?;
                        let (concatted, newlines) = Token::concat(&tokens);
                        let mut tokens: Vec<Token> = Vec::new();
                        tokens.push(Token::NewlineToken(
                            format!("\"{}\"", concatted.trim()),
                            newlines,
                        ));
                        Ok(tokens)
                    } else {
                        Ok(tokens)
                    }
                } else {
                    self.resolve_pseudoargs(def_map, stack, depth, mismatches, budget)
                }
            }
            None => self.resolve_pseudoargs(def_map, stack, depth, mismatches, budget),
        }
    }

    fn resolve_all(
        tokens: &[Token],
        def_map: &HashMap<String, Definition>,
        stack: &[Definition],
        depth: usize,
        mismatches: &mut Vec<String>,
        budget: &mut Budget,
    ) -> Result<Vec<Token>, Error> {
        let mut result: Vec<Token> = Vec::new();

        for token in tokens {
            if let Token::MacroToken(m) = token {
                let resolved = m.resolve(def_map, stack, depth, mismatches, budget)?;
                budget.spend(
                    resolved.len(),
                    resolved.iter().map(token_bytes).sum(),
                    &m.name,
                )?;
                result.extend(resolved);
            } else {
                budget.spend(
                    1,
                    token_bytes(token),
                    stack.last().map_or("", |d| d.name.as_str()),
                )?;
                result.push(token.clone());
            }
        }

        Ok(result)
    }
}

impl Token {
    fn concat(tokens: &[Token]) -> (String, u32) {
        let mut output = String::new();
        let mut newlines = 0;

        for token in tokens {
            match token {
                Token::RegularToken(s) => {
                    output += s;
                }
                Token::NewlineToken(s, n) => {
                    output += s;
                    newlines += n;
                }
                Token::MacroToken(m) => {
                    output += &m.original;
                }
                Token::CommentToken(n) => {
                    newlines += n;
                }
                Token::ConcatToken => {}
            }
        }

        (output, newlines)
    }
}

/// The source newlines of tokens that are not expanded: those of strings and comments (a string inside a
/// macro call is counted once) and every `\`-newline continuation, wherever it sits.
fn skipped_newlines(tokens: &[Token]) -> u32 {
    let mut total = 0u32;
    for token in tokens {
        let n = match token {
            Token::NewlineToken(_, n) | Token::CommentToken(n) => *n,
            Token::MacroToken(m) => {
                let mut in_string = false;
                let mut prev = ' ';
                let mut count = 0u32;
                for c in m.original.chars() {
                    match c {
                        '"' => in_string = !in_string,
                        '\n' if in_string || prev == '\\' => count = count.saturating_add(1),
                        _ => {}
                    }
                    prev = c;
                }
                count
            }
            Token::RegularToken(s) => u32::try_from(s.matches("\\\n").count()).unwrap_or(u32::MAX),
            Token::ConcatToken => 0,
        };
        total = total.saturating_add(n);
    }
    total
}

/// Reads the first line of a `$PBOPREFIX$` file.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
fn read_prefix(prefix_path: &Path) -> Result<String, Error> {
    let mut content = String::new();
    File::open(prefix_path)?.read_to_string(&mut content)?;

    Ok(content.lines().next().unwrap_or_default().to_string())
}

/// Returns the path seperator used on the current operating system
#[must_use]
pub(crate) fn pathsep() -> &'static str {
    if cfg!(windows) { "\\" } else { "/" }
}

fn names_equal(a: &OsStr, b: &OsStr, ci: bool) -> bool {
    if ci {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// Whether two paths to the same file differ in the case of any component.
/// A case-insensitive filesystem (Windows) can open a path whose case does not
/// match the file on disk; comparing the components byte-wise detects that.
fn differs_in_case(a: &Path, b: &Path) -> bool {
    let (mut a, mut b) = (a.components(), b.components());
    loop {
        match (a.next(), b.next()) {
            (None, None) => return false,
            (Some(x), Some(y)) if x.as_os_str() == y.as_os_str() => {}
            _ => return true,
        }
    }
}

/// Resolves `relative` below `base`, comparing each component ASCII-case-insensitively.
/// An exact match is preferred over a case-insensitive one at every level.
///
/// The names always come from the directory entries, never from `exists()`
/// probes: on a case-insensitive filesystem (Windows) those succeed for any
/// casing and would hide the name the file has on disk.
fn resolve_case_insensitive(base: &Path, relative: &Path) -> Option<PathBuf> {
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let mut exact = None;
        let mut ignoring_case = None;
        for entry in read_dir(&current).ok()?.filter_map(std::result::Result::ok) {
            if exact.is_none() && names_equal(&entry.file_name(), name, false) {
                exact = Some(entry);
            } else if ignoring_case.is_none() && names_equal(&entry.file_name(), name, true) {
                ignoring_case = Some(entry);
            }
        }
        current = exact.or(ignoring_case)?.path();
    }
    Some(current)
}

fn matches_include_path(path: &Path, include_path: &str, ci: bool) -> bool {
    let include_pathbuf = PathBuf::from(&include_path.replace('\\', pathsep()));

    match (path.file_name(), include_pathbuf.file_name()) {
        (Some(a), Some(b)) if names_equal(a, b, ci) => {}
        _ => return false,
    }

    for parent in path.ancestors() {
        if parent.is_file() {
            continue;
        }

        let prefixpath = parent.join("$PBOPREFIX$");
        if !prefixpath.is_file() {
            continue;
        }

        // an unreadable prefix file cannot match the include; skip it instead of failing
        let Ok(mut prefix) = read_prefix(&prefixpath) else {
            continue;
        };

        prefix = if !prefix.is_empty() && !prefix.starts_with('\\') {
            format!("\\{prefix}")
        } else {
            prefix
        };

        let prefix_pathbuf = PathBuf::from(prefix.replace('\\', pathsep()));

        let Ok(relative) = path.strip_prefix(parent) else {
            continue;
        };
        let test_path = prefix_pathbuf.join(relative);

        let (mut a, mut b) = (test_path.components(), include_pathbuf.components());
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some(x), Some(y)) if names_equal(x.as_os_str(), y.as_os_str(), ci) => {}
                _ => break,
            }
        }
    }

    false
}

fn search_directory(include_path: &str, directory: &Path, ci: bool) -> Option<PathBuf> {
    // unreadable directories are skipped
    if let Ok(entries) = read_dir(directory) {
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name() == Some(OsStr::new(".git")) {
                    continue;
                }

                if let Some(path) = search_directory(include_path, &path, ci) {
                    return Some(path);
                }
            } else if matches_include_path(&path, include_path, ci) {
                return Some(path);
            }
        }
    }

    let relative = PathBuf::from(
        include_path
            .trim_start_matches('\\')
            .replace('\\', pathsep()),
    );
    let direct_pathbuf = if ci {
        resolve_case_insensitive(directory, &relative)?
    } else {
        // A case-insensitive filesystem (Windows) opens the joined path even
        // when the case does not match; leave such matches to the ci pass,
        // which reports them, instead of returning them silently.
        let joined = directory.join(&relative);
        if !joined.is_file() {
            return None;
        }
        match resolve_case_insensitive(directory, &relative) {
            Some(found) if differs_in_case(&joined, &found) => return None,
            // the directories cannot be read, so the case cannot be checked
            _ => return Some(joined),
        }
    };

    if direct_pathbuf.is_file() {
        return Some(direct_pathbuf);
    }

    None
}

fn canonicalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            _ => {
                result.push(component);
            }
        }
    }
    result
}

fn include_not_found(include_path: &str, origin: Option<&Path>, line: u32) -> Error {
    match origin {
        Some(origin_path) => error!(
            "File \"{}\" included from \"{}:{}\" not found.",
            printable(include_path),
            printable(&origin_path.display().to_string()),
            line
        ),
        None => error!(
            "Included file \"{}\" (line {}) not found.",
            printable(include_path),
            line
        ),
    }
}

fn warn_include_case(include_path: &str, found: &Path, warnings: &mut dyn WarningSink) {
    warnings.warn(Warning::new(
        Some("include-case"),
        format!(
            "Include \"{}\" only matched \"{}\" ignoring case.",
            printable(include_path),
            printable(&found.display().to_string())
        ),
    ));
}

fn find_include_file(
    include_path: &str,
    origin: Option<&Path>,
    search_paths: &[PathBuf],
    line: u32,
    warnings: &mut dyn WarningSink,
) -> Result<PathBuf, Error> {
    if include_path.starts_with('\\') {
        let mut roots: Vec<PathBuf> = Vec::new();
        for search_path in search_paths {
            roots.push(search_path.canonicalize().map_err(|e| {
                error!(
                    "Include folder \"{}\" is not usable: {}",
                    search_path.display(),
                    e
                )
            })?);
        }

        for ci in [false, true] {
            for root in &roots {
                if let Some(file_path) = search_directory(include_path, root, ci) {
                    if ci {
                        warn_include_case(include_path, &file_path, warnings);
                    }
                    return Ok(file_path);
                }
            }
        }

        Err(include_not_found(include_path, origin, line))
    } else {
        let mut path = PathBuf::from(include_path.replace('\\', pathsep()));

        if let Some(origin_path) = origin {
            let absolute = PathBuf::from(origin_path).canonicalize()?;
            let Some(origin_dir) = absolute.parent() else {
                return Err(error!(
                    "Include origin \"{}\" has no parent directory.",
                    printable(&origin_path.display().to_string())
                ));
            };
            path = origin_dir.join(path);
        } else {
            path = current_dir()?.join(path);
        }

        let absolute = canonicalize(&path);
        let root: PathBuf = absolute
            .components()
            .take_while(|c| !matches!(c, Component::Normal(_)))
            .collect();
        let relative: PathBuf = absolute
            .components()
            .skip_while(|c| !matches!(c, Component::Normal(_)))
            .collect();

        if absolute.is_file() {
            // A case-insensitive filesystem (Windows) opens the probe even when
            // the case does not match the file on disk; check the components
            // against the directory entries so the warning fires there too.
            if let Some(found) = resolve_case_insensitive(&root, &relative)
                && differs_in_case(&absolute, &found)
            {
                warn_include_case(include_path, &found, warnings);
            }
            return Ok(absolute);
        }

        // case-insensitive fallback for case-sensitive filesystems
        match resolve_case_insensitive(&root, &relative) {
            Some(found) if found.is_file() => {
                warn_include_case(include_path, &found, warnings);
                Ok(found)
            }
            _ => Err(include_not_found(include_path, origin, line)),
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one branch per directive keeps the preprocessor loop readable"
)]
fn preprocess_rec(
    input: &str,
    origin: Option<&Path>,
    definition_map: &mut HashMap<String, Definition>,
    info: &mut PreprocessInfo,
    includefolders: &[PathBuf],
    warnings: &mut dyn WarningSink,
) -> Result<String, Error> {
    // CRLF and classic Mac (lone CR) newlines
    let input = input.replace("\r\n", "\n").replace('\r', "\n");
    let deep = Cell::new(false);
    let parsed = preprocess_grammar::file(&input, &deep);
    if deep.get() {
        return Err(nesting_error());
    }
    let lines = PreprocessParseErrorExt::format_error(parsed, origin, &input)?;
    let mut output = String::new();
    let mut original_lineno = 1;
    let mut level = 0;
    let mut level_true = 0;

    for line in lines {
        // Every path leaves this block (never `continue`), so the line counter below always runs (#63).
        'line: {
            match line {
                Line::DirectiveLine(dir, newlines) => {
                    original_lineno += newlines;

                    match dir {
                        Directive::IncludeDirective(path) => {
                            if level > level_true {
                                break 'line;
                            }

                            let file_path = find_include_file(
                                &path,
                                origin,
                                includefolders,
                                original_lineno,
                                warnings,
                            )?;

                            let wanted = identity(&file_path);
                            if info.import_stack.iter().any(|p| identity(p) == wanted) {
                                let chain: Vec<String> = info
                                    .import_stack
                                    .iter()
                                    .chain(std::iter::once(&file_path))
                                    .map(|p| printable(&p.display().to_string()))
                                    .collect();
                                return Err(error!("Include cycle: {}", chain.join(" -> ")));
                            }
                            if info.include_depth >= MAX_INCLUDE_DEPTH {
                                return Err(error!(
                                    "Include \"{}\" nests too deep (limit {MAX_INCLUDE_DEPTH})",
                                    printable(&path)
                                ));
                            }

                            info.includes += 1;
                            if info.includes > MAX_INCLUDES {
                                return Err(error!(
                                    "Include \"{}\" is the {}th include; the limit is {MAX_INCLUDES} per file",
                                    printable(&path),
                                    info.includes
                                ));
                            }

                            info.import_stack.push(file_path.clone());
                            info.include_depth += 1;

                            let mut content = String::new();
                            File::open(&file_path)?.read_to_string(&mut content)?;
                            let result = preprocess_rec(
                                &content,
                                Some(&file_path),
                                definition_map,
                                info,
                                includefolders,
                                warnings,
                            )
                            .prepend_error(format!(
                                "Failed to preprocess include \"{}\":",
                                printable(&path)
                            ))?;

                            info.import_stack.pop();
                            info.include_depth -= 1;

                            output += &result;
                        }
                        Directive::DefineDirective(def) => {
                            original_lineno += skipped_newlines(&def.value);

                            if level > level_true {
                                break 'line;
                            }

                            if definition_map.remove(&def.name).is_some() {
                                // @todo: warn about redefine
                            }

                            definition_map.insert(def.name.clone(), def);
                        }
                        Directive::UndefDirective(name) => {
                            if level > level_true {
                                break 'line;
                            }

                            definition_map.remove(&name);
                        }
                        Directive::IfDefDirective(name) => {
                            level_true += i32::from(
                                level_true == level && definition_map.contains_key(&name),
                            );
                            level += 1;
                        }
                        Directive::IfNDefDirective(name) => {
                            level_true += i32::from(
                                level_true == level && !definition_map.contains_key(&name),
                            );
                            level += 1;
                        }
                        Directive::ElseDirective => {
                            if level_true + 1 == level {
                                level_true = level;
                            } else if level_true == level {
                                level_true -= 1;
                            }
                        }
                        Directive::EndIfDirective => {
                            if level == 0 {
                                return Err(error!("#endif without matching #ifdef/#ifndef"));
                            }
                            level -= 1;
                            if level_true > level {
                                level_true -= 1;
                            }
                        }
                    }
                }
                Line::TokenLine(tokens) => {
                    if level > level_true {
                        // A disabled line is never expanded (#61): it can neither error nor cost time.
                        original_lineno += skipped_newlines(&tokens);
                        break 'line;
                    }
                    let stack: Vec<Definition> = Vec::new();
                    let mut mismatches: Vec<String> = Vec::new();
                    let mut budget = Budget::new();
                    let resolved = Macro::resolve_all(
                        &tokens,
                        definition_map,
                        &stack,
                        0,
                        &mut mismatches,
                        &mut budget,
                    )
                    .prepend_error("Failed to resolve macros:");
                    let resolved = resolved?;
                    info.expanded = info.expanded.saturating_add(budget.tokens);
                    info.expanded_bytes = info.expanded_bytes.saturating_add(budget.bytes);
                    if info.expanded > MAX_FILE_EXPANSION || info.expanded_bytes > MAX_FILE_BYTES {
                        return Err(error!(
                            "Macro expansion produced more than {MAX_FILE_EXPANSION} tokens or {MAX_FILE_BYTES} bytes in one file"
                        ));
                    }

                    let (mut result, newlines) = Token::concat(&resolved);
                    result = result.replace("\r\n", "\n");
                    original_lineno += newlines;

                    let before = result.len();
                    result = result.replace("\\\n", "");

                    let file = origin.map(|p| p.display().to_string());
                    for msg in mismatches {
                        let mut warning =
                            Warning::new(Some("macro-arg-count"), msg).at_line(original_lineno);
                        if let Some(file) = &file {
                            warning = warning.in_file(file.clone());
                        }
                        warnings.warn(warning);
                    }

                    output += &result;
                    output += "\n";

                    info.line_origins
                        .push((original_lineno, origin.map(Path::to_path_buf)));
                    let removed = u32::try_from((before - result.len()) / 2)
                        .map_err(|_| error!("line number out of range"))?;
                    original_lineno += removed;
                }
            }
        }
        original_lineno += 1;

        if level > 0 {
            // @todo: complain
        }
    }

    Ok(output)
}

/// Reads input string and returns preprocessed string with an info struct containing the origins
/// of the lines in the output.
///
/// `path` is the path to the input if it is known and is used for relative includes and error
/// messages. `includefolders` are the folders searched for absolute includes and should usually at
/// least include the current working directory.
///
/// # Errors
///
/// Returns an error on parse failures, unreadable includes or invalid include folders.
///
/// # Examples
///
/// ```
/// # use dayzmake_core::preprocess::preprocess;
/// # use dayzmake_core::warning::Warnings;
/// let input = String::from("
/// #define QUOTE(x) #x
/// #define DOUBLES(x,y) x##_##y
///
/// foo = QUOTE(DOUBLES(abc, xyz));
/// ");
///
/// let (output, _) = preprocess(input, None, &Vec::new(), &mut Warnings::new()).expect("Failed to preprocess");
///
/// assert_eq!("foo = \"abc_xyz\";", output.trim());
/// ```
#[expect(
    clippy::needless_pass_by_value,
    reason = "public API takes ownership of the input; callers pass owned strings"
)]
pub fn preprocess(
    mut input: String,
    origin: Option<PathBuf>,
    includefolders: &[PathBuf],
    warnings: &mut dyn WarningSink,
) -> Result<(String, PreprocessInfo), Error> {
    if input.as_bytes().starts_with(&[0xef, 0xbb, 0xbf]) {
        input = input[3..].to_string();
    }

    for folder in includefolders {
        if !folder.is_dir() {
            return Err(error!(
                "Include folder \"{}\" does not exist or is not a directory.",
                folder.display()
            ));
        }
    }

    let mut info = PreprocessInfo {
        line_origins: Vec::new(),
        import_stack: Vec::new(),
        include_depth: 0,
        expanded: 0,
        expanded_bytes: 0,
        includes: 0,
    };

    if let Some(ref path) = origin {
        info.import_stack.push(path.clone());
    }

    let mut def_map: HashMap<String, Definition> = HashMap::new();

    match preprocess_rec(
        &input,
        origin.as_deref(),
        &mut def_map,
        &mut info,
        includefolders,
        warnings,
    ) {
        Ok(result) => Ok((result, info)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    /// Argument-count mismatches go into the buffer of the call, not into state shared by the
    /// thread: a second resolve with a fresh buffer sees nothing of the first.
    #[test]
    fn arg_count_mismatches_belong_to_their_call() {
        let mut defs: HashMap<String, Definition> = HashMap::new();
        let def = Definition {
            name: "F".to_string(),
            parameters: Some(vec!["a".to_string()]),
            value: Vec::new(),
            local: false,
        };
        defs.insert("F".to_string(), def);
        let tokens = vec![Token::MacroToken(parse_macro("F(1,2)").unwrap())];

        let mut first = Vec::new();
        Macro::resolve_all(&tokens, &defs, &[], 0, &mut first, &mut Budget::new()).unwrap();
        assert_eq!(first.len(), 1);

        let plain = vec![Token::RegularToken("x".to_string())];
        let mut second = Vec::new();
        Macro::resolve_all(&plain, &defs, &[], 0, &mut second, &mut Budget::new()).unwrap();
        assert_eq!(second.len(), 0);
    }

    /// The resolver reports the name the file has on disk, never the requested
    /// one: on Windows an `exists()` probe succeeds for any casing, so only the
    /// directory entries carry the truth.
    #[test]
    fn resolve_reports_the_on_disk_casing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("inc.hpp"), b"x").unwrap();
        let found = resolve_case_insensitive(dir.path(), Path::new("INC.hpp")).unwrap();
        assert_eq!(found.file_name(), Some(OsStr::new("inc.hpp")));
    }

    /// An exact match is preferred over a case-insensitive one. The fixture
    /// needs two files that differ only in case, which a case-insensitive
    /// filesystem (Windows) cannot hold.
    #[test]
    #[cfg(not(windows))]
    fn resolve_prefers_the_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Inc.hpp"), b"a").unwrap();
        std::fs::write(dir.path().join("inc.hpp"), b"b").unwrap();
        let found = resolve_case_insensitive(dir.path(), Path::new("inc.hpp")).unwrap();
        assert_eq!(found.file_name(), Some(OsStr::new("inc.hpp")));
    }

    #[test]
    fn case_difference_is_detected_component_wise() {
        assert!(differs_in_case(
            Path::new("/a/B.hpp"),
            Path::new("/a/b.hpp")
        ));
        assert!(!differs_in_case(
            Path::new("/a/b.hpp"),
            Path::new("/a/b.hpp")
        ));
    }
}
